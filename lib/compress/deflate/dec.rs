// deflate::dec -- the DEFLATE (RFC 1951) and zlib (RFC 1950) decoder behind compress::deflate,
// compress::zlib and compress::gzip (until 0.2 img_core::inflate)          [src/img/inflate.s]
//
//   zlib_inflate_ex(st, src, dst) -> Ok((bytes, consumed)) / Err(Error)   consumed = header,
//                                     deflate data and Adler-32
//   inflate_raw_st(st, src, dst) -> Ok((bytes, consumed)) / Err(Error)   consumed = input
//                                     bytes of the deflate data (up to the end of the final
//                                     block)
//   InflateState::new() -> Box      reusable decoder state (one thread at a time; an error
//                                     never affects later calls); st None = a temporary one
//   rev16(x)                        16-bit bit reversal (shared with deflate)
//   LBASE / LEXT / DBASE / DEXT / CLORDER   the code tables (shared with deflate)
//
// The output buffer has a fixed capacity: every write is bounds-checked (Error::Overflow),
// every back reference is checked against the bytes produced so far (Error::Distance), reading
// past the end of the input gives Error::Truncated.  zlib header errors: Error::Header; a wrong
// Adler-32: Error::Checksum.
//
// The bit buffer behaves exactly as the assembly's (the error codes of corrupt input depend
// on it): refilled 8 bytes at a time while 8 input bytes remain (the count becomes
// 56 + count % 8), byte by byte near the end with zero bytes counted as padding (more than
// PAD_MAX of them = Error::Truncated); Huffman tables as stb_image (10-bit fast table, canonical
// slow path); a code table's sorted-index arrays keep entries of an earlier build (the
// slow path checks the length, as the assembly).
//
// Speed (perf-inflate): the symbol loop runs in codes_fast while 8 input bytes remain at
// every refill and a whole match fits the output: no slow refill, no overflow checks, no
// call on the literal path (the state stays in registers), u32 fast tables (lx / dx) with
// the length / distance base and extra-bit count folded in (one load and two shifts per
// code), wide match moves.  It hands over to the careful loop before the symbol where a
// condition fails; both follow the assembly's loop step by step.

use alloc::boxed::Box;

use crate::checksum::adler32;
use super::Error;

const FAST_BITS: u32 = 10;
pub(super) const FAST_MASK: u64 = 1023;
const PAD_MAX: u32 = 8;

pub static LBASE: [u16; 32] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258, 0,
    0, 0,
];
pub static DBASE: [u16; 32] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577, 0, 0,
];
pub static LEXT: [u8; 32] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0, 0, 0, 0];
pub static DEXT: [u8; 32] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 0, 0];
pub static CLORDER: [u8; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// 16-bit bit reversal  [zl_rev16]
#[inline]
pub const fn rev16(x: u32) -> u32 {
    ((x & 0xFFFF) as u16).reverse_bits() as u32
}

// Huffman decode table (HT_*)
#[derive(Clone)]
pub(super) struct Huff {
    pub(super) fast: [u16; 1024], // (symbol << 6) | len, 0 = slow path (the length: a shift count as is)
    fcode: [u16; 16],  // first canonical code per length
    fsym: [u16; 16],   // first sorted index per length
    maxc: [u32; 17],   // (last code + 1) << (16 - len)
    len: [u8; 320],    // code length per sorted index
    val: [u16; 320],   // symbol per sorted index
}

impl Huff {
    const ZERO: Huff = Huff { fast: [0; 1024], fcode: [0; 16], fsym: [0; 16], maxc: [0; 17], len: [0; 320], val: [0; 320] };

    // huff_build(ht, lens, n) -> 0 / Error::Code (over-subscribed); incomplete codes are
    // accepted (unused bit patterns decode as Error::Code)
    pub(super) fn build(&mut self, lens: &[u8]) -> Result<(), Error> {
        self.build_into(lens, None)
    }

    // build with the fast-loop table xt instead of the u16 one (its entry of each code
    // computed once by xf; the literal / length and distance codes)
    pub(super) fn build_x(&mut self, lens: &[u8], xt: &mut [u32; 1024], xf: fn(u32) -> u32) -> Result<(), Error> {
        self.build_into(lens, Some((xt, xf)))
    }

    fn build_into(&mut self, lens: &[u8], mut x: Option<(&mut [u32; 1024], fn(u32) -> u32)>) -> Result<(), Error> {
        match &mut x {
            Some((xt, _)) => xt.fill(0), // (the u16 fast table stays unused)
            None => self.fast = [0; 1024],
        }
        let mut sizes = [0u32; 18];
        for &l in lens {
            sizes[(l & 15) as usize] += 1;
        }
        sizes[0] = 0;
        let mut next = [0u32; 16];
        let mut code = 0u32;
        let mut k = 0u32;
        for i in 1..16 {
            next[i] = code;
            self.fcode[i] = code as u16;
            self.fsym[i] = k as u16;
            code += sizes[i];
            k += sizes[i];
            if code > 1 << i {
                return Err(Error::Code);
            }
            self.maxc[i] = code << (16 - i);
            code += code;
        }
        self.maxc[16] = 0x10000;
        for (sym, &l) in lens.iter().enumerate() {
            let s = (l & 15) as usize;
            if s == 0 {
                continue;
            }
            let c = next[s];
            let idx = c.wrapping_sub(self.fcode[s] as u32).wrapping_add(self.fsym[s] as u32) as usize;
            if idx >= 320 {
                return Err(Error::Code);
            }
            self.len[idx] = s as u8;
            self.val[idx] = sym as u16;
            if s as u32 <= FAST_BITS {
                let e = (sym << 6 | s) as u16;
                let mut j = (rev16(c) >> (16 - s)) as usize;
                match &mut x {
                    Some((xt, xf)) => {
                        let xe = xf(e as u32);
                        while j < 1024 {
                            xt[j] = xe;
                            j += 1 << s;
                        }
                    }
                    None => {
                        while j < 1024 {
                            self.fast[j] = e;
                            j += 1 << s;
                        }
                    }
                }
            }
            next[s] += 1;
        }
        Ok(())
    }

    // .ic_slow as a fast-table entry: (symbol << 6) | length of a code longer than
    // FAST_BITS, 0 = a bad code
    #[cold]
    #[inline(never)]
    pub(super) fn slow_entry(&self, bb: u64) -> u32 {
        let v = rev16((bb & 0xFFFF) as u32);
        let mut l = FAST_BITS as usize + 1;
        while l < 16 && v >= self.maxc[l] {
            l += 1;
        }
        if l >= 16 {
            return 0;
        }
        let idx = (v >> (16 - l)).wrapping_sub(self.fcode[l] as u32).wrapping_add(self.fsym[l] as u32) as usize;
        if idx >= 320 || self.len[idx] as usize != l {
            return 0;
        }
        (self.val[idx] as u32) << 6 | l as u32
    }

    // .ic_slow: canonical decode of a code longer than FAST_BITS -> (symbol, length)
    #[inline(never)]
    pub(super) fn slow(&self, bb: u64) -> Option<(u32, u32)> {
        let v = rev16((bb & 0xFFFF) as u32);
        let mut l = FAST_BITS as usize + 1;
        loop {
            if l >= 16 {
                return None;
            }
            if v < self.maxc[l] {
                break;
            }
            l += 1;
        }
        let idx = (v >> (16 - l)).wrapping_sub(self.fcode[l] as u32).wrapping_add(self.fsym[l] as u32) as usize;
        if idx >= 320 || self.len[idx] as usize != l {
            return None;
        }
        Some((self.val[idx] as u32, l as u32))
    }
}

// the two code tables of a block (one pointer in the symbol loop) and their extended fast
// tables for codes_fast
pub(super) struct Tabs {
    pub(super) lit: Huff,
    pub(super) dist: Huff,
    pub(super) lx: [u32; 1024], // the fast table of lit as fast-loop entries (X_*; lit.fast unused)
    pub(super) dx: [u32; 1024], // the fast table of dist
}

// fast-loop entries: bits 0..5 the code length (a shift count as is), 10..13 the extra bits,
// 14..15 the kind, 16..31 the literal byte / the length or distance base; 0 = slow path
pub(super) const X_LEN: u32 = 1 << 14; // a length symbol
pub(super) const X_EOB: u32 = 2 << 14; // the end-of-block code
pub(super) const X_BAD: u32 = 3 << 14; // literal / length symbols 286, 287, distance symbols 30, 31
pub(super) const X_KIND: u32 = 3 << 14;

// the fast-loop entry of the literal / length code's fast entry e (0 stays 0)
#[inline(always)]
pub(super) fn lit_x(e: u32) -> u32 {
    let (sym, l) = (e >> 6, e & 63);
    if e == 0 {
        0
    } else if sym < 256 {
        sym << 16 | l
    } else if sym == 256 {
        X_EOB | l
    } else if sym < 257 + 29 {
        let k = (sym - 257) as usize;
        let x = LEXT[k] as u32;
        (LBASE[k] as u32) << 16 | X_LEN | x << 10 | l
    } else {
        X_BAD | l
    }
}

// the fast-loop entry of the distance code's fast entry e (0 stays 0)
#[inline(always)]
pub(super) fn dist_x(e: u32) -> u32 {
    let (sym, l) = (e >> 6, e & 63);
    if e == 0 {
        0
    } else if sym < 30 {
        let x = DEXT[sym as usize] as u32;
        (DBASE[sym as usize] as u32) << 16 | x << 10 | l
    } else {
        X_BAD | l
    }
}

/// a reusable decoder state (IS_*); only `pad` is reset per call, the fixed tables are built
/// once per state, every other table is rebuilt before it is read
pub struct InflateState {
    pub(super) tabs: Tabs, // literal / length and distance codes
    pub(super) cl: Huff,
    pub(super) lens: [u8; 320], // code lengths being built
    pub(super) cll: [u8; 24],   // code-length code lengths
    pub(super) fixed: bool,     // lit / dist hold the fixed code
    pad: u32,        // zero bytes appended after the input end
}

impl InflateState {
    /// a zeroed state  [inflate_state_new]
    pub fn new() -> Box<InflateState> {
        Box::new(InflateState {
            tabs: Tabs { lit: Huff::ZERO, dist: Huff::ZERO, lx: [0; 1024], dx: [0; 1024] },
            cl: Huff::ZERO,
            lens: [0; 320],
            cll: [0; 24],
            fixed: false,
            pad: 0,
        })
    }
}

// the bit reader of the decoder (rbx, r12d, rsi; r13 = input end); Copy, so that the hot
// loop works on a local copy that stays in registers
#[derive(Clone, Copy)]
struct Bits<'a> {
    src: &'a [u8],
    bb: u64,
    cnt: u32,
    pos: usize,
}

impl Bits<'_> {
    // REFILL: at least 48 bits (Err = Error::Truncated: the padding ran out)
    #[inline(always)]
    fn refill(&mut self, pad: &mut u32) -> Result<(), Error> {
        if self.cnt >= 48 {
            return Ok(());
        }
        if self.pos + 8 <= self.src.len() {
            let v = u64::from_le_bytes(self.src[self.pos..self.pos + 8].try_into().unwrap());
            self.bb |= v << self.cnt;
            self.pos += ((63 - self.cnt) >> 3) as usize;
            self.cnt |= 56;
            Ok(())
        } else {
            *self = refill_slow(*self, pad)?;
            Ok(())
        }
    }

    #[inline(always)]
    fn get(&mut self, n: u32) -> u32 {
        let v = (self.bb & ((1u64 << n) - 1)) as u32;
        self.bb >>= n;
        self.cnt -= n;
        v
    }

    // HDECODE: one symbol (needs >= 15 bits in the buffer); Err(Error::Code) for a bad code
    #[inline(always)]
    fn decode(&mut self, h: &Huff) -> Result<u32, Error> {
        let e = h.fast[(self.bb & FAST_MASK) as usize] as u32;
        if e != 0 {
            let l = e & 63;
            self.bb >>= l;
            self.cnt -= l;
            return Ok(e >> 6);
        }
        match h.slow(self.bb) {
            Some((s, l)) => {
                self.bb >>= l;
                self.cnt -= l;
                Ok(s)
            }
            None => Err(Error::Code),
        }
    }

    // whole bytes still buffered after dropping the bits of a partial byte; the input
    // position of the first unread byte (Err = padding consumed)
    fn byte_pos(&mut self, pad: u32) -> Result<usize, Error> {
        let k = self.cnt & 7;
        self.bb >>= k;
        self.cnt -= k;
        let whole = (self.cnt >> 3) as usize;
        if pad as usize > whole {
            return Err(Error::Truncated);
        }
        let p = self.pos as isize - whole as isize + pad as isize;
        if p < 0 || p as usize > self.src.len() {
            return Err(Error::Truncated);
        }
        Ok(p as usize)
    }
}

/// zlib stream src -> dst with a reusable state (None = a temporary one): (bytes written,
/// input bytes of the stream: the header, the deflate data and the Adler-32)
/// [zlib_inflate_st]
pub fn zlib_inflate_ex(st: Option<&mut InflateState>, src: &[u8], dst: &mut [u8]) -> Result<(usize, usize), Error> {
    if src.len() < 2 {
        return Err(Error::Truncated);
    }
    let (b0, b1) = (src[0] as u32, src[1] as u32);
    if b0 & 15 != 8 || b0 >> 4 > 7 || b1 & 0x20 != 0 || (b0 << 8 | b1) % 31 != 0 {
        return Err(Error::Header);
    }
    // core checked that the 4 bytes of the Adler-32 follow the deflate data
    core(st, &src[2..], dst, true).map(|(n, used)| (n, 2 + used + 4))
}

/// raw deflate data src -> dst with a reusable state (None = a temporary one): (bytes
/// written, input bytes the deflate data occupied)  [inflate_raw_st]
pub fn inflate_raw_st(st: Option<&mut InflateState>, src: &[u8], dst: &mut [u8]) -> Result<(usize, usize), Error> {
    core(st, src, dst, false)
}

// inflate_core: a temporary state when st is None
fn core(st: Option<&mut InflateState>, src: &[u8], dst: &mut [u8], zlib: bool) -> Result<(usize, usize), Error> {
    match st {
        Some(s) => body(s, src, dst, zlib),
        None => body(&mut InflateState::new(), src, dst, zlib),
    }
}

// the fixed literal / distance code lengths
pub(super) fn fixed_lens(l: &mut [u8; 320]) {
    l[..144].fill(8);
    l[144..256].fill(9);
    l[256..280].fill(7);
    l[280..288].fill(8);
    l[288..320].fill(5);
}

// .ic_refill: byte-wise near the end; zero bytes past the end are counted in pad
#[inline(never)]
fn refill_slow<'a>(mut b: Bits<'a>, pad: &mut u32) -> Result<Bits<'a>, Error> {
    b.bb &= (1u64 << b.cnt) - 1; // drop look-ahead bits above the count
    while b.cnt <= 56 {
        if b.pos < b.src.len() {
            b.bb |= (b.src[b.pos] as u64) << b.cnt;
            b.pos += 1;
        } else {
            *pad += 1;
            if *pad > PAD_MAX {
                return Err(Error::Truncated);
            }
        }
        b.cnt += 8;
    }
    Ok(b)
}

// ic_body -> (bytes written, input bytes consumed)
fn body(st: &mut InflateState, src: &[u8], dst: &mut [u8], zlib: bool) -> Result<(usize, usize), Error> {
    st.pad = 0;
    let mut br = Bits { src, bb: 0, cnt: 0, pos: 0 };
    let mut o = 0usize; // output position
    let cap = dst.len();
    loop {
        // ---- block header
        br.refill(&mut st.pad)?;
        let fin = br.bb & 1 != 0;
        let typ = (br.bb >> 1) & 3;
        br.bb >>= 3;
        br.cnt -= 3;
        match typ {
            0 => {
                // stored block
                let p = br.byte_pos(st.pad)?;
                br.bb = 0;
                br.cnt = 0;
                st.pad = 0;
                if src.len() - p < 4 {
                    return Err(Error::Truncated);
                }
                let n = u16::from_le_bytes([src[p], src[p + 1]]) as usize;
                let nn = u16::from_le_bytes([src[p + 2], src[p + 3]]) as usize;
                if n != nn ^ 0xFFFF {
                    return Err(Error::Block);
                }
                let p = p + 4;
                if n > src.len() - p {
                    return Err(Error::Truncated);
                }
                if n > cap - o {
                    return Err(Error::Overflow);
                }
                dst[o..o + n].copy_from_slice(&src[p..p + n]);
                o += n;
                br.pos = p + n;
                if fin {
                    break;
                }
                continue;
            }
            1 => {
                if !st.fixed {
                    fixed_lens(&mut st.lens);
                    st.tabs.lit.build_x(&st.lens[..288], &mut st.tabs.lx, lit_x)?;
                    st.tabs.dist.build_x(&st.lens[288..320], &mut st.tabs.dx, dist_x)?;
                    st.fixed = true;
                }
            }
            2 => {
                st.fixed = false;
                br.refill(&mut st.pad)?;
                let hlit = br.get(5) as usize + 257;
                let hdist = br.get(5) as usize + 1;
                let total = hlit + hdist;
                let hclen = br.get(4) as usize + 4;
                if hlit > 286 || hdist > 30 {
                    return Err(Error::Block);
                }
                st.cll = [0; 24];
                for &c in &CLORDER[..hclen] {
                    br.refill(&mut st.pad)?;
                    st.cll[c as usize] = br.get(3) as u8;
                }
                st.cl.build(&st.cll[..19])?;
                let mut i = 0usize;
                while i < total {
                    br.refill(&mut st.pad)?;
                    let sym = br.decode(&st.cl)?;
                    if sym < 16 {
                        st.lens[i] = sym as u8;
                        i += 1;
                        continue;
                    }
                    let (rep, val) = match sym {
                        16 => {
                            if i == 0 {
                                return Err(Error::Block);
                            }
                            (br.get(2) as usize + 3, st.lens[i - 1])
                        }
                        17 => (br.get(3) as usize + 3, 0),
                        _ => (br.get(7) as usize + 11, 0),
                    };
                    if i + rep > total {
                        return Err(Error::Block);
                    }
                    st.lens[i..i + rep].fill(val);
                    i += rep;
                }
                if st.lens[256] == 0 {
                    return Err(Error::Code); // no end-of-block code
                }
                st.tabs.lit.build_x(&st.lens[..hlit], &mut st.tabs.lx, lit_x)?;
                st.tabs.dist.build_x(&st.lens[hlit..hlit + hdist], &mut st.tabs.dx, dist_x)?;
            }
            _ => return Err(Error::Block),
        }
        // ---- symbols
        let (no, bb, cnt, pos) = codes(src, br.bb, br.cnt, br.pos, &st.tabs, &mut st.pad, dst, o)?;
        o = no;
        br.bb = bb;
        br.cnt = cnt;
        br.pos = pos;
        if fin {
            break;
        }
    }
    // ---- end of stream
    let p = br.byte_pos(st.pad)?;
    if zlib {
        if src.len() - p < 4 {
            return Err(Error::Truncated);
        }
        let want = u32::from_be_bytes([src[p], src[p + 1], src[p + 2], src[p + 3]]);
        if adler32(1, &dst[..o]) != want {
            return Err(Error::Checksum);
        }
    }
    Ok((o, p))
}

// free output bytes the fast loop needs before a symbol: the longest match plus the slack of
// its wide moves
pub(super) const FAST_ROOM: usize = 258 + 16;

// .ic_codes: literal / length / distance symbols until the end-of-block code; the bit
// state (bb, cnt, pos) in plain locals -> (output position, bb, cnt, pos).  codes_fast runs
// while every refill finds 8 input bytes and FAST_ROOM output bytes are free; it stops
// before a symbol where either fails and this loop (refill near the end of the input,
// overflow checks) takes over.  Both follow the assembly's loop step by step (refill below
// 48 bits before every symbol, the same checks in the same order), so results and error
// codes do not depend on where the switch happens.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codes(
    src: &[u8],
    bb: u64,
    cnt: u32,
    pos: usize,
    t: &Tabs,
    pad: &mut u32,
    dst: &mut [u8],
    o: usize,
) -> Result<(usize, u64, u32, usize), Error> {
    let mut f = Fast { rest: &src[pos.min(src.len())..], bb, cnt, o };
    let eob = codes_fast(&mut f, t, dst)?;
    let Fast { mut rest, mut bb, mut cnt, mut o } = f; // rest: the unread input
    if eob {
        return Ok((o, bb, cnt, src.len() - rest.len()));
    }
    let cap = dst.len();
    macro_rules! refill {
        () => {
            if cnt < 48 {
                if rest.len() >= 8 {
                    let v = u64::from_le_bytes(rest[..8].try_into().unwrap());
                    bb |= v << cnt;
                    rest = &rest[((63 - cnt) >> 3) as usize..];
                    cnt |= 56;
                } else {
                    let b = refill_slow(Bits { src, bb, cnt, pos: src.len() - rest.len() }, pad)?;
                    bb = b.bb;
                    cnt = b.cnt;
                    rest = &src[b.pos..];
                }
            }
        };
    }
    // one code through a fast-loop table (the slow path for longer codes): its entry, the
    // code bits dropped; Err(Error::Code) for a bad code
    macro_rules! decode {
        ($xt:expr, $h:expr, $xf:expr) => {{
            let mut e = $xt[(bb & FAST_MASK) as usize];
            if e == 0 {
                e = $xf($h.slow_entry(bb));
                if e == 0 {
                    return Err(Error::Code);
                }
            }
            bb >>= e & 15;
            cnt -= e & 15;
            e
        }};
    }
    // the base of entry e plus its extra bits
    macro_rules! value {
        ($e:expr) => {{
            let x = ($e >> 10) & 15;
            let v = ($e >> 16) as usize + (bb & ((1u64 << x) - 1)) as usize;
            bb >>= x;
            cnt -= x;
            v
        }};
    }
    loop {
        refill!();
        let e = decode!(t.lx, t.lit, lit_x);
        if e & X_KIND == 0 {
            if o >= cap {
                return Err(Error::Overflow);
            }
            dst[o] = (e >> 16) as u8;
            o += 1;
            continue;
        }
        if e & X_KIND == X_EOB {
            return Ok((o, bb, cnt, src.len() - rest.len()));
        }
        if e & X_KIND == X_BAD {
            return Err(Error::Code);
        }
        let len = value!(e);
        let e = decode!(t.dx, t.dist, dist_x);
        if e & X_KIND != 0 {
            return Err(Error::Distance);
        }
        let dis = value!(e);
        if dis > o {
            return Err(Error::Distance);
        }
        if len > cap - o {
            return Err(Error::Overflow);
        }
        copy_match(dst, o, dis, len);
        o += len;
    }
}

// the state codes_fast continues and leaves: the unread input, the bit buffer, the output
// position (a struct behind one pointer: every argument in a register)
pub(super) struct Fast<'a> {
    pub(super) rest: &'a [u8],
    pub(super) bb: u64,
    pub(super) cnt: u32,
    pub(super) o: usize,
}

// the symbol loop of codes while 8 input bytes are there at every refill and FAST_ROOM output
// bytes are free: no slow refill, no overflow checks, every match with wide moves -> Ok(true)
// at the end-of-block code, Ok(false) before the symbol where a condition fails.  Literals
// run in an inner loop of their own (the register allocator favours its values).
#[inline(never)]
pub(super) fn codes_fast(f: &mut Fast, t: &Tabs, dst: &mut [u8]) -> Result<bool, Error> {
    let mut rest = f.rest;
    let mut bb = f.bb;
    let mut cnt = f.cnt;
    let mut o = f.o;
    if dst.len() < FAST_ROOM {
        return Ok(false);
    }
    let lim = dst.len() - FAST_ROOM; // the last output position with FAST_ROOM bytes free
    let eob = 'run: loop {
        let sym = loop {
            if cnt < 48 {
                if rest.len() < 8 {
                    break 'run false;
                }
                bb |= u64::from_le_bytes(rest[..8].try_into().unwrap()) << cnt;
                rest = &rest[((63 - cnt) >> 3) as usize..];
                cnt |= 56;
            }
            if o > lim {
                break 'run false;
            }
            let mut e = t.lx[(bb & FAST_MASK) as usize];
            if e == 0 {
                e = lit_x(t.lit.slow_entry(bb));
                if e == 0 {
                    return Err(Error::Code);
                }
            }
            bb = bb.wrapping_shr(e); // shr masks the count: the low bits = the code length
            cnt -= e & 15; // (& 15, not & 63: no shared mask before the shift)
            if e & X_KIND != 0 {
                break e;
            }
            dst[o] = (e >> 16) as u8;
            o += 1;
        };
        let e = sym;
        if e & X_KIND == X_EOB {
            break true;
        }
        if e & X_KIND == X_BAD {
            return Err(Error::Code);
        }
        let x = (e >> 10) & 15;
        let len = (e >> 16) as usize + (bb & ((1u64 << x) - 1)) as usize;
        bb >>= x;
        cnt -= x;
        let mut e = t.dx[(bb & FAST_MASK) as usize];
        if e == 0 {
            e = dist_x(t.dist.slow_entry(bb));
            if e == 0 {
                return Err(Error::Code);
            }
        }
        if e & X_KIND != 0 {
            return Err(Error::Distance);
        }
        bb = bb.wrapping_shr(e);
        cnt -= e & 15;
        let x = (e >> 10) & 15;
        let dis = (e >> 16) as usize + (bb & ((1u64 << x) - 1)) as usize;
        bb >>= x;
        cnt -= x;
        if dis > o {
            return Err(Error::Distance);
        }
        if dis >= 16 && len <= 16 {
            // the common short match: one move (the source ends dis >= 16 bytes before o)
            let (head, tail) = dst.split_at_mut(o);
            let v: [u8; 16] = head[o - dis..][..16].try_into().unwrap();
            tail[..16].copy_from_slice(&v);
        } else {
            copy_fast(dst, o, dis, o + len);
        }
        o += len;
    };
    f.rest = rest;
    f.bb = bb;
    f.cnt = cnt;
    f.o = o;
    Ok(eob)
}

// the match [o, end) (3..258 bytes) from dis (1..o) back, at least 16 free bytes after it:
// 16- / 8-byte moves whose source lies before their destination (final bytes), slack after
// the match (later writes overwrite it)
#[inline(never)]
fn copy_fast(dst: &mut [u8], o: usize, dis: usize, end: usize) {
    let mut d = o;
    if dis >= 16 {
        loop {
            let v: [u8; 16] = dst[d - dis..d - dis + 16].try_into().unwrap();
            dst[d..d + 16].copy_from_slice(&v);
            d += 16;
            if d >= end {
                return;
            }
        }
    }
    if dis >= 8 {
        loop {
            let v: [u8; 8] = dst[d - dis..d - dis + 8].try_into().unwrap();
            dst[d..d + 8].copy_from_slice(&v);
            d += 8;
            if d >= end {
                return;
            }
        }
    }
    if dis == 1 {
        let v = [dst[d - 1]; 16];
        loop {
            dst[d..d + 16].copy_from_slice(&v);
            d += 16;
            if d >= end {
                return;
            }
        }
    }
    // period 2..7: 8-byte moves dis bytes apart until p bytes (the smallest multiple of dis
    // >= 8) are final, then 8-byte moves from p bytes back (the same bytes: the data
    // repeats every dis bytes)
    let p = PERIOD8[dis] as usize;
    let pe = o + p;
    while d < pe {
        let v: [u8; 8] = dst[d - dis..d - dis + 8].try_into().unwrap();
        dst[d..d + 8].copy_from_slice(&v);
        d += dis;
    }
    while d < end {
        let v: [u8; 8] = dst[d - p..d - p + 8].try_into().unwrap();
        dst[d..d + 8].copy_from_slice(&v);
        d += 8;
    }
}

// the bytes of an LZ77 match (overlapping copies repeat the pattern)
#[inline(always)]
pub(super) fn copy_match(dst: &mut [u8], o: usize, dis: usize, len: usize) {
    let s = o - dis;
    if dis >= len && len >= 32 {
        dst.copy_within(s..s + len, o); // no overlap: one move
        return;
    }
    if o + len + 16 <= dst.len() {
        // wide moves with slack after the match (later writes overwrite it); each chunk's
        // source lies before its destination, so it holds final bytes
        let w = &mut dst[s..o + len + 16]; // the source, the match and the slack
        if dis >= 16 {
            let mut k = 0;
            while k < len {
                let t: [u8; 16] = w[k..k + 16].try_into().unwrap();
                w[dis + k..dis + k + 16].copy_from_slice(&t);
                k += 16;
            }
            return;
        }
        if dis >= 8 {
            let mut k = 0;
            while k < len {
                let t: [u8; 8] = w[k..k + 8].try_into().unwrap();
                w[dis + k..dis + k + 8].copy_from_slice(&t);
                k += 8;
            }
            return;
        }
        if dis == 1 {
            let b = w[0];
            w[1..1 + len].fill(b);
            return;
        }
        // period 2..7: the first p bytes one by one (p = a multiple of dis >= 8), then 8-byte
        // moves from p bytes back (the same bytes: the data repeats every dis bytes)
        let p = PERIOD8[dis] as usize;
        let first = p.min(len);
        for k in 0..first {
            w[dis + k] = w[k];
        }
        let mut k = first;
        while k < len {
            let t: [u8; 8] = w[dis + k - p..dis + k - p + 8].try_into().unwrap();
            w[dis + k..dis + k + 8].copy_from_slice(&t);
            k += 8;
        }
        return;
    }
    for k in 0..len {
        dst[o + k] = dst[s + k];
    }
}

// the smallest multiple of d that is >= 8 (d = 2..7)
static PERIOD8: [u8; 8] = [8, 8, 8, 9, 8, 10, 12, 14];
