// brotli -- Brotli (RFC 7932) both ways, the Content-Encoding "br" of the web and the
// compression of WOFF2 fonts and of JPEG XL's JPEG reconstruction data (until 0.2 in
// img_jxl)                                                               [jxl-plus: no asm]
//
//   compress(data, level) -> stream              levels 0..=11 (LEVEL_DEFAULT 5), the window
//                                                as small as the data allows (enc.rs)
//   decompress(data, max_out) -> Option<Vec<u8>>   the stream that fills data completely;
//                                                None for a damaged stream, data after it
//                                                or more than max_out bytes
//   decompress_prefix(data, max_out) -> Ok((bytes, consumed))   the stream at the start of
//                                                data; Error::Corrupt for a damaged stream,
//                                                Error::Limit past max_out bytes
//   Decoder / Encoder                            in pieces (stream::Decode / Encode; sdec.rs,
//                                                senc.rs)
//
// The decoder makes one pass over the whole input: bit reader, prefix codes (simple and
// complex), block switches, context maps (RLE + move to front), insert-and-copy commands, the
// distance ring buffer and the static dictionary with its 121 transforms (tab.rs, dict.bin;
// tools/gen_brotli_dict.py).  The output is bounded by max_out before it grows, every
// distance is checked against the output so far.  Large windows (an extension outside RFC
// 7932) are refused.
//
// The bit reader holds up to 63 bits in a register, refilled eight bytes at a time (byte by
// byte near the end).  A prefix code is a table of its first 8 bits (fewer for short codes)
// and second-level tables for the longer ones, so a symbol takes one or two lookups; reading
// past the end fails exactly where reading bit by bit would.

use alloc::vec;
use alloc::vec::Vec;

mod enc;
mod sdec;
mod senc;
mod tab;
use tab as T;

pub use enc::{bound, compress};
pub use sdec::Decoder;
pub use senc::Encoder;

/// the level of compress() for most uses, as for the responses of a web server
pub const LEVEL_DEFAULT: i32 = 5;
/// the strongest level, for files compressed once and sent often (as `brotli -q 11`)
pub const LEVEL_MAX: i32 = 11;

/// why a Brotli stream could not be read
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// the stream is damaged, truncated or uses something RFC 7932 does not allow
    Corrupt,
    /// the output would be larger than the caller allows
    Limit,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Error::Corrupt => "the Brotli stream is damaged",
            Error::Limit => "the output would be too large",
        })
    }
}

static DICT: &[u8] = include_bytes!("dict.bin");

/// insert length codes: (offset, extra bits)
static INS: [(u32, u32); 24] = [
    (0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (5, 0), (6, 1), (8, 1), (10, 2), (14, 2), (18, 3), (26, 3),
    (34, 4), (50, 4), (66, 5), (98, 5), (130, 6), (194, 7), (322, 8), (578, 9), (1090, 10), (2114, 12),
    (6210, 14), (22594, 24),
];
/// copy length codes: (offset, extra bits)
static COPY: [(u32, u32); 24] = [
    (2, 0), (3, 0), (4, 0), (5, 0), (6, 0), (7, 0), (8, 0), (9, 0), (10, 1), (12, 1), (14, 2), (18, 2),
    (22, 3), (30, 3), (38, 4), (54, 4), (70, 5), (102, 5), (134, 6), (198, 7), (326, 8), (582, 9),
    (1094, 10), (2118, 24),
];
/// the insert and copy code bases of the 11 command cells (symbol >> 6)
static CELLS: [(u32, u32, bool); 11] = [
    (0, 0, true),
    (0, 8, true),
    (0, 0, false),
    (0, 8, false),
    (8, 0, false),
    (8, 8, false),
    (0, 16, false),
    (16, 0, false),
    (8, 16, false),
    (16, 8, false),
    (16, 16, false),
];
/// the order of the code length code lengths
static CL_ORDER: [usize; 18] = [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// the fixed code of the code length code lengths (4 bits peeked): length, value
static CL_LEN: [u32; 16] = [2, 2, 2, 3, 2, 2, 2, 4, 2, 2, 2, 3, 2, 2, 2, 4];
static CL_VAL: [u8; 16] = [0, 4, 3, 2, 0, 4, 3, 1, 0, 4, 3, 2, 0, 4, 3, 5];

/// LSB-first bit reader: a 64-bit buffer refilled bytewise from the input.  The bits of buf
/// above nb are zeros or the stream's next bits, past the end of d only zeros.
struct Br<'a> {
    d: &'a [u8],
    next: usize, // the next byte of d to go into buf
    buf: u64,    // the bits from the position on
    nb: u32,     // bits in buf (at most 63)
    short: bool, // a read went past the end (the streaming decoder waits for more input)
}

impl<'a> Br<'a> {
    /// at bit `pos` of d (at most its end)
    fn new(d: &'a [u8], pos: usize) -> Br<'a> {
        let mut br = Br { d, next: 0, buf: 0, nb: 0, short: false };
        br.seek(pos);
        br
    }
    /// the position in bits
    #[inline]
    fn pos(&self) -> usize {
        self.next * 8 - self.nb as usize
    }
    /// to bit `pos` (the buffer starts again there)
    fn seek(&mut self, pos: usize) {
        self.next = pos >> 3;
        self.buf = 0;
        self.nb = 0;
        let r = (pos & 7) as u32;
        if r != 0 {
            if let Some(&b) = self.d.get(self.next) {
                self.buf = (b >> r) as u64;
                self.nb = 8 - r;
                self.next += 1;
            }
        }
    }
    /// at least 56 bits in buf, or all that are left
    #[inline(always)]
    fn refill(&mut self) {
        if let Some(w) = self.d.get(self.next..self.next + 8) {
            let w = u64::from_le_bytes([w[0], w[1], w[2], w[3], w[4], w[5], w[6], w[7]]);
            self.buf |= w << self.nb;
            let k = (63 - self.nb) >> 3;
            self.next += k as usize;
            self.nb += 8 * k;
        } else {
            self.refill_end();
        }
    }
    #[cold]
    #[inline(never)]
    fn refill_end(&mut self) {
        while self.nb <= 55 && self.next < self.d.len() {
            self.buf |= (self.d[self.next] as u64) << self.nb;
            self.next += 1;
            self.nb += 8;
        }
    }
    /// bits of input from the position on
    #[inline]
    fn left(&self) -> usize {
        (self.d.len() - self.next) * 8 + self.nb as usize
    }
    /// n bits (at most 32)
    #[inline(always)]
    fn bits(&mut self, n: u32) -> Result<u32, Error> {
        if self.nb < n {
            self.refill();
            if self.nb < n {
                self.short = true;
                return Err(Error::Corrupt);
            }
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.nb -= n;
        Ok(v)
    }
    /// up to n bits (at most 32) without consuming (zeros past the end)
    #[inline]
    fn peek(&mut self, n: u32) -> u32 {
        if self.nb < n {
            self.refill();
        }
        (self.buf & ((1u64 << n) - 1)) as u32
    }
    fn skip(&mut self, n: u32) -> Result<(), Error> {
        if n <= self.nb {
            self.buf >>= n;
            self.nb -= n;
            return Ok(());
        }
        let p = self.pos() + n as usize;
        if p > self.d.len() * 8 {
            self.short = true;
            return Err(Error::Corrupt);
        }
        self.seek(p);
        Ok(())
    }
    /// to the byte boundary; the padding bits must be zero
    fn align(&mut self) -> Result<(), Error> {
        if self.bits(self.nb & 7)? != 0 {
            return Err(Error::Corrupt);
        }
        Ok(())
    }
    /// VarLenUint8
    fn varlen8(&mut self) -> Result<u32, Error> {
        if self.bits(1)? == 0 {
            return Ok(0);
        }
        let n = self.bits(3)?;
        if n == 0 { Ok(1) } else { Ok(self.bits(n)? + (1 << n)) }
    }
}

/// the largest alphabet (insert-and-copy codes)
const ALPHA_MAX: usize = 704;
/// bits of a first-level table
const ROOT: u32 = 8;
/// a second-level table's entry in the first-level table: LINK | offset << 3 | (bits - 1)
const LINK: u16 = 0x8000;

/// a canonical prefix code (codes read most significant bit first, as in deflate) as lookup
/// tables: the first min(8, longest) bits index the first-level table, longer codes go on in
/// a second-level table of that prefix (as libbrotli and zlib build them).  An entry is
/// symbol << 4 | length, or a LINK; a code of one symbol is one entry of length 0.
struct Huff {
    t: Vec<u16>,
    mask: u32, // of the first-level index
}

impl Huff {
    fn single(s: u16) -> Huff {
        Huff { t: vec![s << 4], mask: 0 }
    }

    fn new(lens: &[u8]) -> Result<Huff, Error> {
        if lens.len() > ALPHA_MAX {
            return Err(Error::Corrupt);
        }
        let mut count = [0u16; 16];
        for &l in lens {
            count[(l & 15) as usize] += 1;
        }
        let used = lens.len() - count[0] as usize;
        if used == 0 {
            return Err(Error::Corrupt);
        }
        if used == 1 {
            let s = lens.iter().position(|&l| l != 0).unwrap_or(0) as u16;
            return Ok(Huff::single(s));
        }
        count[0] = 0;
        // complete codes only (read_code makes no others): 2^15 = the sum of 2^(15 - length)
        let mut space = 0u32;
        let mut longest = 0u32;
        for (l, &c) in count.iter().enumerate().skip(1) {
            space += (c as u32) << (15 - l);
            if c != 0 {
                longest = l as u32;
            }
        }
        if space != 1 << 15 {
            return Err(Error::Corrupt);
        }
        // the symbols by (length, symbol)
        let mut offs = [0u16; 16];
        for l in 1..15 {
            offs[l + 1] = offs[l] + count[l];
        }
        let mut sorted = [0u16; ALPHA_MAX];
        for (s, &l) in lens.iter().enumerate() {
            if l != 0 {
                sorted[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        let root = longest.min(ROOT);
        let mut t = vec![0u16; 1 << root];
        let mut rem = count; // codes of each length not placed yet
        let mut code = 0u32; // the canonical code of the next symbol
        let mut len = 1u32;
        let mut prefix = u32::MAX; // the first-level index of the current second-level table
        let (mut sub, mut sub_bits) = (0usize, 0u32);
        for &s in &sorted[..used] {
            while rem[len as usize] == 0 {
                len += 1;
                code <<= 1;
            }
            let rev = ((code as u16).reverse_bits() >> (16 - len)) as u32;
            let e = s << 4 | len as u16;
            if len <= root {
                let mut j = rev as usize;
                while j < t.len() {
                    t[j] = e;
                    j += 1 << len;
                }
            } else {
                let p = rev & ((1 << ROOT) - 1);
                if p != prefix {
                    // a new second-level table: as many bits as its codes need
                    prefix = p;
                    let mut l = len;
                    let mut room = 1i32 << (l - ROOT);
                    loop {
                        room -= rem[l as usize] as i32;
                        if room <= 0 || l == 15 {
                            break;
                        }
                        l += 1;
                        room <<= 1;
                    }
                    sub_bits = l - ROOT;
                    sub = t.len();
                    if sub >= 1 << 12 {
                        return Err(Error::Corrupt); // (no code of 704 symbols needs this: 1080 at most)
                    }
                    t.resize(sub + (1 << sub_bits), 0);
                    t[p as usize] = LINK | (sub as u16) << 3 | (sub_bits - 1) as u16;
                }
                let step = 1usize << (len - ROOT);
                let mut j = (rev >> ROOT) as usize;
                while j < 1 << sub_bits {
                    t[sub + j] = e;
                    j += step;
                }
            }
            rem[len as usize] -= 1;
            code += 1;
        }
        Ok(Huff { t, mask: (1 << root) - 1 })
    }

    /// the entry of the code at the start of bits
    #[inline(always)]
    fn entry(&self, bits: u64) -> u16 {
        let e = self.t[(bits as u32 & self.mask) as usize];
        if e & LINK == 0 {
            return e;
        }
        let i = ((e >> 3) & 0xFFF) as usize + ((bits >> ROOT) as u32 & ((2 << (e & 7)) - 1)) as usize;
        self.t[i]
    }

    #[inline(always)]
    fn decode(&self, br: &mut Br) -> Result<u32, Error> {
        if br.nb < 15 {
            br.refill();
            if br.nb < 15 {
                return self.decode_end(br);
            }
        }
        let e = self.entry(br.buf);
        let n = (e & 15) as u32;
        br.buf >>= n;
        br.nb -= n;
        Ok((e >> 4) as u32)
    }

    /// near the end of the input: the code may need more bits than are left
    #[cold]
    #[inline(never)]
    fn decode_end(&self, br: &mut Br) -> Result<u32, Error> {
        let e = self.entry(br.buf);
        let n = (e & 15) as u32;
        if n > br.nb {
            br.short = true;
            return Err(Error::Corrupt);
        }
        br.buf >>= n;
        br.nb -= n;
        Ok((e >> 4) as u32)
    }
}

/// a prefix code of an alphabet of `size` symbols
fn read_code(br: &mut Br, size: usize) -> Result<Huff, Error> {
    if size > ALPHA_MAX {
        return Err(Error::Corrupt);
    }
    let mut lens = [0u8; ALPHA_MAX];
    let lens = &mut lens[..size];
    let hskip = br.bits(2)?;
    if hskip == 1 {
        // simple code
        let mut bits = 0u32;
        let mut i = size - 1;
        while i != 0 {
            i >>= 1;
            bits += 1;
        }
        let nsym = br.bits(2)? as usize + 1;
        let mut syms = [0usize; 4];
        for s in syms.iter_mut().take(nsym) {
            *s = br.bits(bits)? as usize;
            if *s >= size {
                return Err(Error::Corrupt);
            }
        }
        for a in 0..nsym {
            for b in a + 1..nsym {
                if syms[a] == syms[b] {
                    return Err(Error::Corrupt);
                }
            }
        }
        let ls: &[u8] = match nsym {
            1 => &[1],
            2 => &[1, 1],
            3 => &[1, 2, 2],
            _ => {
                if br.bits(1)? == 0 {
                    &[2, 2, 2, 2]
                } else {
                    &[1, 2, 3, 3]
                }
            }
        };
        for k in 0..nsym {
            lens[syms[k]] = ls[k];
        }
        if nsym == 1 {
            return Ok(Huff::single(syms[0] as u16));
        }
        return Huff::new(lens);
    }
    // complex code: the code length code, then the code lengths
    let mut cl = [0u8; 18];
    let mut space = 32i32;
    let mut num = 0;
    for &o in CL_ORDER.iter().skip(hskip as usize) {
        let p = br.peek(4) as usize;
        br.skip(CL_LEN[p])?;
        let v = CL_VAL[p];
        cl[o] = v;
        if v != 0 {
            space -= 32 >> v;
            num += 1;
            if space <= 0 {
                break;
            }
        }
    }
    if !(num == 1 || space == 0) {
        return Err(Error::Corrupt);
    }
    let clh = Huff::new(&cl)?;
    let mut sym = 0usize;
    let mut prev = 8u8;
    let mut repeat = 0usize;
    let mut repeat_len = 0u8;
    let mut space = 32768i32;
    while sym < size && space > 0 {
        let c = clh.decode(br)? as u8;
        if c < 16 {
            repeat = 0;
            lens[sym] = c;
            sym += 1;
            if c != 0 {
                prev = c;
                space -= 32768 >> c;
            }
        } else {
            let (eb, nl) = if c == 16 { (2, prev) } else { (3, 0) };
            if repeat_len != nl {
                repeat = 0;
                repeat_len = nl;
            }
            let old = repeat;
            if repeat > 0 {
                repeat -= 2;
                repeat <<= eb;
            }
            repeat += br.bits(eb)? as usize + 3;
            let delta = repeat - old;
            if sym + delta > size {
                return Err(Error::Corrupt);
            }
            for l in lens.iter_mut().skip(sym).take(delta) {
                *l = repeat_len;
            }
            sym += delta;
            if repeat_len != 0 {
                space -= (delta as i32) * (32768 >> repeat_len);
            }
        }
    }
    if space != 0 {
        return Err(Error::Corrupt);
    }
    Huff::new(lens)
}

/// a context map of `size` entries over `ntrees` trees
fn read_cmap(br: &mut Br, size: usize, ntrees: usize) -> Result<Vec<u8>, Error> {
    let mut map = vec![0u8; size];
    if ntrees < 2 {
        return Ok(map);
    }
    let rlemax = if br.bits(1)? != 0 { br.bits(4)? as usize + 1 } else { 0 };
    let h = read_code(br, ntrees + rlemax)?;
    let mut i = 0;
    while i < size {
        let c = h.decode(br)? as usize;
        if c == 0 {
            map[i] = 0;
            i += 1;
        } else if c <= rlemax {
            let reps = (1usize << c) + br.bits(c as u32)? as usize;
            if i + reps > size {
                return Err(Error::Corrupt);
            }
            i += reps; // (zeros)
        } else {
            map[i] = (c - rlemax) as u8;
            i += 1;
        }
    }
    if br.bits(1)? != 0 {
        // inverse move to front
        let mut mtf: Vec<u8> = (0..=255u8).collect();
        for v in map.iter_mut() {
            let idx = *v as usize;
            let val = mtf[idx];
            *v = val;
            mtf.copy_within(0..idx, 1);
            mtf[0] = val;
        }
    }
    Ok(map)
}

/// block switching of one category (literals, commands, distances)
struct Blocks {
    n: u32,
    types: Option<Huff>,
    lens: Option<Huff>,
    cur: u32,
    last: u32,
    second: u32,
    left: u32,
}

impl Blocks {
    fn read(br: &mut Br) -> Result<Blocks, Error> {
        let n = br.varlen8()? + 1;
        let mut b = Blocks { n, types: None, lens: None, cur: 0, last: 0, second: 1, left: u32::MAX };
        if n >= 2 {
            b.types = Some(read_code(br, n as usize + 2)?);
            let l = read_code(br, 26)?;
            b.left = block_len(br, &l)?;
            b.lens = Some(l);
        }
        Ok(b)
    }
    /// one symbol of the current block; the next block's type when it is used up
    #[inline(always)]
    fn next(&mut self, br: &mut Br) -> Result<(), Error> {
        if self.left == 0 {
            self.switch(br)?;
        }
        self.left -= 1;
        Ok(())
    }
    /// the next block's type and length
    #[inline(never)]
    fn switch(&mut self, br: &mut Br) -> Result<(), Error> {
        let (Some(t), Some(l)) = (self.types.as_ref(), self.lens.as_ref()) else { return Err(Error::Corrupt) };
        let c = t.decode(br)?;
        let mut ty = match c {
            0 => self.second,
            1 => self.last + 1,
            _ => c - 2,
        };
        if ty >= self.n {
            ty -= self.n;
        }
        self.second = self.last;
        self.last = ty;
        self.cur = ty;
        self.left = block_len(br, l)?;
        Ok(())
    }
}

fn block_len(br: &mut Br, h: &Huff) -> Result<u32, Error> {
    let c = h.decode(br)? as usize;
    let (o, e) = T::BLOCK_LEN[c];
    Ok(o as u32 + br.bits(e as u32)?)
}

/// to upper case of the UTF-8 character at p (RFC 7932 ToUpperCase): its length
fn upper(p: &mut [u8]) -> usize {
    if p[0] < 0xC0 {
        if p[0].is_ascii_lowercase() {
            p[0] ^= 32;
        }
        return 1;
    }
    if p[0] < 0xE0 {
        if p.len() > 1 {
            p[1] ^= 32;
        }
        return 2;
    }
    if p.len() > 2 {
        p[2] ^= 5;
    }
    3
}

/// a copy up to this long goes as one copy of this size, cut back after it (so out needs this
/// much room past the copy)
const COPY_SHORT: usize = 32;

/// an insert-and-copy code with its extra bits -> (insert length, copy length, whether the
/// distance is the last one again)
#[inline(always)]
fn command(cmd: u32, br: &mut Br) -> Result<(usize, usize, bool), Error> {
    let (ib, cb, d0) = CELLS[(cmd >> 6) as usize];
    let ic = (ib + ((cmd >> 3) & 7)) as usize;
    let cc = (cb + (cmd & 7)) as usize;
    let ilen = INS[ic].0 as usize + br.bits(INS[ic].1)? as usize;
    let clen = COPY[cc].0 as usize + br.bits(COPY[cc].1)? as usize;
    Ok((ilen, clen, d0))
}

/// the distance of distance code dc (with its extra bits); rb: the last four distances, the
/// last at rb_idx
#[inline(always)]
fn distance(dc: usize, rb: &[usize; 4], rb_idx: usize, npostfix: usize, ndirect: usize, br: &mut Br) -> Result<usize, Error> {
    if dc < 16 {
        static IDX: [usize; 16] = [0, 1, 2, 3, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1];
        static OFF: [i64; 16] = [0, 0, 0, 0, -1, 1, -2, 2, -3, 3, -1, 1, -2, 2, -3, 3];
        let v = rb[(rb_idx + 4 - IDX[dc]) & 3] as i64 + OFF[dc];
        if v <= 0 {
            return Err(Error::Corrupt);
        }
        Ok(v as usize)
    } else if dc < 16 + ndirect {
        Ok(dc - 15)
    } else {
        let x = dc - ndirect - 16;
        let nb = 1 + (x >> (npostfix + 1));
        let hcode = x >> npostfix;
        let lcode = x & ((1usize << npostfix) - 1);
        let off = ((2 + (hcode & 1)) << nb) - 4;
        Ok(((off + br.bits(nb as u32)? as usize) << npostfix) + lcode + ndirect + 1)
    }
}

/// a static dictionary word (len 4 to 24) with transform t appended to out
fn dict_word(out: &mut Vec<u8>, len: usize, idx: usize, t: usize) {
    let o = T::OFFSETS[len] as usize + idx * len;
    let mut buf = [0u8; 24];
    buf[..len].copy_from_slice(&DICT[o..o + len]);
    let [pre, ty, suf] = T::TRANSFORMS[t];
    let ty = ty as usize;
    let ps = |id: u8| -> &'static [u8] {
        let s = T::PREFIX_SUFFIX_MAP[id as usize] as usize;
        let n = T::PREFIX_SUFFIX[s] as usize;
        &T::PREFIX_SUFFIX[s + 1..s + 1 + n]
    };
    out.extend_from_slice(ps(pre));
    let w: &mut [u8] = if (1..=9).contains(&ty) {
        &mut buf[..len.saturating_sub(ty)]
    } else if (12..=20).contains(&ty) {
        &mut buf[(ty - 11).min(len)..len]
    } else {
        &mut buf[..len]
    };
    if ty == 10 && !w.is_empty() {
        upper(w);
    } else if ty == 11 {
        let mut i = 0;
        while i < w.len() {
            i += upper(&mut w[i..]);
        }
    }
    out.extend_from_slice(w);
    out.extend_from_slice(ps(suf));
}

/// a static dictionary reference of copy length clen, id = the distance beyond the output
/// minus 1: the word appended to out -> its length
fn dict_ref(out: &mut Vec<u8>, clen: usize, id: usize) -> Result<usize, Error> {
    if !(4..=24).contains(&clen) {
        return Err(Error::Corrupt);
    }
    let nb = T::SIZE_BITS[clen] as usize;
    let idx = id & ((1usize << nb) - 1);
    let t = id >> nb;
    if t >= T::TRANSFORMS.len() || nb == 0 {
        return Err(Error::Corrupt);
    }
    let before = out.len();
    dict_word(out, clen, idx, t);
    Ok(out.len() - before)
}

/// clen bytes from dist back (1 <= dist <= out.len()) appended to out
#[inline(always)]
fn copy_back(out: &mut Vec<u8>, dist: usize, clen: usize) {
    let n = out.len();
    let s = n - dist;
    if clen <= COPY_SHORT && dist >= COPY_SHORT {
        // most copies: one copy of a fixed size, then the length cut back
        let mut w = [0u8; COPY_SHORT];
        w.copy_from_slice(&out[s..s + COPY_SHORT]);
        out.extend_from_slice(&w);
        out.truncate(n + clen);
        return;
    }
    if dist >= clen {
        out.extend_from_within(s..s + clen);
        return;
    }
    // overlapping: the bytes from s on repeat with period dist, so each round may copy all
    // of them that are there
    let mut left = clen;
    while left > 0 {
        let k = left.min(out.len() - s);
        out.extend_from_within(s..s + k);
        left -= k;
    }
}

/// the data of the Brotli stream that fills `data` completely, at most `max_out` bytes
pub fn decompress(data: &[u8], max_out: usize) -> Option<Vec<u8>> {
    match decompress_prefix(data, max_out) {
        Ok((out, used)) if used == data.len() => Some(out),
        _ => None,
    }
}

/// the Brotli stream at the start of `data`: (output, bytes of input used)
pub fn decompress_prefix(data: &[u8], max: usize) -> Result<(Vec<u8>, usize), Error> {
    let mut br = Br::new(data, 0);
    // window size
    let wbits = if br.bits(1)? == 0 {
        16
    } else {
        let n = br.bits(3)?;
        if n != 0 {
            17 + n
        } else {
            let m = br.bits(3)?;
            if m == 1 {
                return Err(Error::Corrupt); // large window: not in RFC 7932
            }
            if m != 0 { 8 + m } else { 17 }
        }
    };
    let window = (1usize << wbits) - 16;
    let mut out: Vec<u8> = Vec::new();
    let mut dist_rb = [16usize, 15, 11, 4]; // the last distance last
    let mut rb_idx = 3usize; // index of the last distance
    loop {
        let last = br.bits(1)? != 0;
        if last && br.bits(1)? != 0 {
            break; // ISLASTEMPTY
        }
        let nib = br.bits(2)?;
        if nib == 3 {
            // metadata block
            if br.bits(1)? != 0 {
                return Err(Error::Corrupt);
            }
            let nb = br.bits(2)?;
            let mut skip = 0usize;
            if nb > 0 {
                skip = br.bits(8 * nb)? as usize + 1;
            }
            br.align()?;
            br.skip(8 * skip as u32)?;
            if last {
                break;
            }
            continue;
        }
        let mlen = br.bits(4 * (nib + 4))? as usize + 1;
        if out.len() + mlen > max {
            return Err(Error::Limit);
        }
        if !last && br.bits(1)? != 0 {
            // uncompressed
            br.align()?;
            let p = br.pos() >> 3;
            if p + mlen > data.len() {
                return Err(Error::Corrupt);
            }
            out.extend_from_slice(&data[p..p + mlen]);
            br.seek(8 * (p + mlen));
            continue;
        }
        let mut bl = Blocks::read(&mut br)?;
        let mut bi = Blocks::read(&mut br)?;
        let mut bd = Blocks::read(&mut br)?;
        let npostfix = br.bits(2)?;
        let ndirect = br.bits(4)? << npostfix;
        let mut cmode = vec![0u8; bl.n as usize];
        for m in cmode.iter_mut() {
            *m = br.bits(2)? as u8;
        }
        let ntl = br.varlen8()? as usize + 1;
        let cmap_l = read_cmap(&mut br, 64 * bl.n as usize, ntl)?;
        let ones = lit_ones(&cmap_l);
        let ntd = br.varlen8()? as usize + 1;
        let cmap_d = read_cmap(&mut br, 4 * bd.n as usize, ntd)?;
        let mut lit_h = Vec::with_capacity(ntl);
        for _ in 0..ntl {
            lit_h.push(read_code(&mut br, 256)?);
        }
        let mut cmd_h = Vec::with_capacity(bi.n as usize);
        for _ in 0..bi.n {
            cmd_h.push(read_code(&mut br, 704)?);
        }
        let dsize = 16 + ndirect as usize + (48usize << npostfix);
        let mut dist_h = Vec::with_capacity(ntd);
        for _ in 0..ntd {
            dist_h.push(read_code(&mut br, dsize)?);
        }
        let mut left = mlen;
        out.reserve(mlen + COPY_SHORT);
        let mut lc = LitCtx::new(&cmode, &cmap_l, &ones, &lit_h, bl.cur)?;
        while left > 0 {
            bi.next(&mut br)?;
            let cmd = cmd_h[bi.cur as usize].decode(&mut br)?;
            let (ilen, clen, d0) = command(cmd, &mut br)?;
            let k = ilen.min(left);
            if k > 0 {
                let n = out.len();
                let mut p = (if n > 0 { out[n - 1] } else { 0 }, if n > 1 { out[n - 2] } else { 0 });
                let mut i = 0;
                while i < k {
                    if bl.left == 0 {
                        bl.switch(&mut br)?;
                        lc = LitCtx::new(&cmode, &cmap_l, &ones, &lit_h, bl.cur)?;
                    }
                    // the literals up to the end of the block
                    let m = (k - i).min(bl.left as usize);
                    bl.left -= m as u32;
                    lc.run(m, &mut out, &mut br, &mut p)?;
                    i += m;
                }
                left -= k;
                if left == 0 {
                    break;
                }
            }
            let (dist, dcode) = if d0 {
                (dist_rb[rb_idx], 0usize)
            } else {
                bd.next(&mut br)?;
                let dctx = if clen > 4 { 3 } else { clen - 2 };
                let tree = cmap_d[bd.cur as usize * 4 + dctx] as usize;
                let dc = dist_h.get(tree).ok_or(Error::Corrupt)?.decode(&mut br)? as usize;
                (distance(dc, &dist_rb, rb_idx, npostfix as usize, ndirect as usize, &mut br)?, dc)
            };
            let maxd = out.len().min(window);
            if dist > maxd {
                // a static dictionary word
                let n = dict_ref(&mut out, clen, dist - maxd - 1)?;
                if n > left {
                    return Err(Error::Corrupt);
                }
                left -= n;
            } else {
                if clen > left {
                    return Err(Error::Corrupt);
                }
                if dcode != 0 {
                    rb_idx = (rb_idx + 1) & 3;
                    dist_rb[rb_idx] = dist;
                }
                copy_back(&mut out, dist, clen);
                left -= clen;
            }
        }
        if last {
            break;
        }
    }
    Ok((out, br.pos().div_ceil(8)))
}

/// per literal block type of a literal context map: the tree all its 64 contexts name, or
/// u16::MAX when they name several
fn lit_ones(cmap: &[u8]) -> Vec<u16> {
    cmap.chunks(64).map(|m| if m.iter().all(|&t| t == m[0]) { m[0] as u16 } else { u16::MAX }).collect()
}

/// the literal context of a literal block type: its context mode's part of CONTEXT_LUT (p1,
/// then p2), its 64 entries of the context map and the trees, or the one tree they all name
/// if they do (then the context needs no computing)
#[derive(Clone, Copy)]
struct LitCtx<'a> {
    lut: &'static [u8],
    cm: &'a [u8],
    trees: &'a [Huff],
    one: Option<&'a Huff>,
}

impl<'a> LitCtx<'a> {
    /// ones: lit_ones(cmap)
    #[inline]
    fn new(cmode: &[u8], cmap: &'a [u8], ones: &[u16], trees: &'a [Huff], cur: u32) -> Result<LitCtx<'a>, Error> {
        let mode = *cmode.get(cur as usize).ok_or(Error::Corrupt)? as usize & 3;
        let cm = cmap.get(cur as usize * 64..cur as usize * 64 + 64).ok_or(Error::Corrupt)?;
        let one = match ones.get(cur as usize) {
            Some(&t) if t != u16::MAX => Some(trees.get(t as usize).ok_or(Error::Corrupt)?),
            _ => None,
        };
        Ok(LitCtx { lut: &T::CONTEXT_LUT[mode << 9..][..512], cm, trees, one })
    }

    /// m literals appended to out (none of them starts a new block); p: the last two bytes
    /// (0 before the stream's start)
    #[inline(always)]
    fn run(&self, m: usize, out: &mut Vec<u8>, br: &mut Br, p: &mut (u8, u8)) -> Result<(), Error> {
        if let Some(h) = self.one {
            for _ in 0..m {
                let lit = h.decode(br)? as u8;
                out.push(lit);
                *p = (lit, p.0);
            }
            return Ok(());
        }
        for _ in 0..m {
            let ctx = (self.lut[p.0 as usize] | self.lut[256 + p.1 as usize]) as usize;
            let lit = self.trees.get(self.cm[ctx & 63] as usize).ok_or(Error::Corrupt)?.decode(br)? as u8;
            out.push(lit);
            *p = (lit, p.0);
        }
        Ok(())
    }
}
