// deflate -- DEFLATE (RFC 1951) / zlib (RFC 1950) compression  [src/img/deflate.s]
//
//   zlib_deflate(src, level) -> Vec          zlib stream (level 0..9, < 0 = 6, > 9 = 9)
//   deflate_raw(src, level) -> Vec           raw RFC 1951 data
//   deflate_raw_ex(st, src, level, head, tail) -> Vec   head free bytes before the data
//                                            (gzip writes its header there), tail reserved
//                                            after it (not part of the Vec)
//   DeflateState::new() -> Box               a reusable compressor state (~330 KB; one
//                                            thread at a time); the same bytes as fresh calls
//   zlib_deflate_st(st, src, level) / deflate_raw_st(st, src, level) -> Vec
//   zlib_deflate_to(st, src, level, dst) / deflate_raw_to(...) -> Ok(bytes) /
//                                            Err(ERR_OVERFLOW): into dst (no allocation with
//                                            a state); overflow when the stream + 8 bytes do
//                                            not fit (dst contents undefined then)
//   deflate_bound(len) -> len + len / 512 + 128   a cap with which *_to never overflow
//
// The algorithm and every table of the assembly: 3-byte multiplicative hash, head / prev
// chains over "global" positions (input offset + base; every input starts GEN_GAP after
// the previous one, so a reused state matches a fresh one), zlib's levels (greedy 1..3, lazy
// 4..9, chain depth / nice / lazy limits), TOO_FAR, 16K-symbol blocks written as the
// cheapest of stored / fixed / dynamic by exact bit count, two-queue Huffman lengths with
// miniz's Kraft repair, RLE-coded code lengths.  Inputs of 4 GiB - 64 KiB and more are
// stored.  The output limit: a block that would end past cap - 8 stops the compressor
// (ERR_OVERFLOW).  Speed (perf-inflate): the LZ77 loops keep their state in locals and
// load the next position's hash head before the current chain walk (the walk is a chain of
// dependent loads; the early load overlaps it) -- the same candidates in the same order.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use crate::checksum::adler32;
use crate::inflate::{CLORDER, DBASE, DEXT, LBASE, LEXT, rev16};
use crate::ERR_OVERFLOW;

const BLOCK_SYMS: usize = 16384;
const HASH_SHIFT: u32 = 32 - 15;
const HASH_MUL: u32 = 0x9E37_79B1;
const WMASK: u32 = 32767;
const MAX_DIST: u32 = 32768;
const TOO_FAR: u32 = 4096;
const GEN_GAP: u64 = 65536;
const POS_LIMIT: u64 = 0xFFFF_0000;

// per level: good length, max lazy, nice length, chain depth, mode (1 lazy)
static LEVELS: [[u32; 5]; 10] = [
    [0, 0, 0, 0, 0],
    [4, 4, 8, 4, 0],
    [4, 5, 16, 8, 0],
    [4, 6, 32, 32, 0],
    [4, 4, 16, 16, 1],
    [8, 16, 32, 32, 1],
    [8, 16, 128, 128, 1],
    [8, 32, 128, 256, 1],
    [32, 128, 258, 1024, 1],
    [32, 258, 258, 4096, 1],
];

// length - 3 -> length code 0..28
static LCOF: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut c = 0;
    while c < 28 {
        let first = LBASE[c] as usize - 3;
        let n = 1usize << LEXT[c];
        let mut i = 0;
        while i < n {
            t[first + i] = c as u8;
            i += 1;
        }
        c += 1;
    }
    t[255] = 28;
    t
};

// distance - 1 -> distance code (zlib layout: 0..255 direct, 256 + (d - 1) >> 7 above)
static DCOF: [u8; 512] = {
    let mut t = [0u8; 512];
    let mut c = 0;
    while c < 30 {
        let mut first = DBASE[c] as usize - 1;
        let mut ext = DEXT[c] as usize;
        if c >= 16 {
            first = (first >> 7) + 256;
            ext -= 7;
        }
        let n = 1usize << ext;
        let mut i = 0;
        while i < n {
            t[first + i] = c as u8;
            i += 1;
        }
        c += 1;
    }
    t
};

static FIXLLEN: [u8; 288] = {
    let mut t = [8u8; 288];
    let mut i = 144;
    while i < 256 {
        t[i] = 9;
        i += 1;
    }
    while i < 280 {
        t[i] = 7;
        i += 1;
    }
    t
};
static FIXDLEN: [u8; 32] = [5; 32];
static FIXLCODE: [u16; 288] = huff_codes_c(&FIXLLEN);
static FIXDCODE: [u16; 32] = huff_codes_c(&FIXDLEN);

// huff_codes(lens, n, codes): canonical codes, bit-reversed for LSB-first output
const fn huff_codes_c<const N: usize>(lens: &[u8; N]) -> [u16; N] {
    let mut out = [0u16; N];
    let mut cnt = [0u32; 16];
    let mut i = 0;
    while i < N {
        cnt[(lens[i] & 15) as usize] += 1;
        i += 1;
    }
    cnt[0] = 0;
    let mut next = [0u32; 16];
    let mut code = 0u32;
    let mut b = 1;
    while b < 16 {
        code = (code + cnt[b - 1]) * 2;
        next[b] = code;
        b += 1;
    }
    let mut s = 0;
    while s < N {
        let l = (lens[s] & 15) as usize;
        if l != 0 {
            let c = next[l];
            next[l] += 1;
            out[s] = (rev16(c) >> (16 - l)) as u16;
        }
        s += 1;
    }
    out
}

fn huff_codes(lens: &[u8], codes: &mut [u16]) {
    let mut cnt = [0u32; 16];
    for &l in lens {
        cnt[(l & 15) as usize] += 1;
    }
    cnt[0] = 0;
    let mut next = [0u32; 16];
    let mut code = 0u32;
    for b in 1..16 {
        code = (code + cnt[b - 1]) * 2;
        next[b] = code;
    }
    for (s, &l) in lens.iter().enumerate() {
        let l = (l & 15) as usize;
        if l != 0 {
            let c = next[l];
            next[l] += 1;
            codes[s] = (rev16(c) >> (16 - l)) as u16;
        }
    }
}

// huff_lengths scratch (HS_*)
struct Hs {
    syms: [u16; 288],
    w: [u32; 576],
    p: [u16; 576],
    d: [u16; 576],
    bl: [u32; 17],
}

// huff_lengths(freq, n, maxbits, lens): length-limited Huffman code lengths; fewer than two
// used symbols still give a complete two-code tree
fn huff_lengths(freq: &[u32], maxbits: usize, lens: &mut [u8], hs: &mut Hs) {
    let n = freq.len();
    lens[..n].fill(0);
    let mut m = 0usize;
    for (i, &f) in freq.iter().enumerate() {
        if f != 0 {
            hs.syms[m] = i as u16;
            m += 1;
        }
    }
    if m < 2 {
        lens[0] = 1;
        lens[1] = 1;
        if m == 0 {
            return;
        }
        let s = hs.syms[0] as usize;
        lens[s] = 1;
        if s > 1 {
            lens[1] = 0; // keep symbol 0 and the used one
        }
        return;
    }
    // insertion sort by frequency (stable: ties stay in symbol order)
    for i in 1..m {
        let key = hs.syms[i];
        let f = freq[key as usize];
        let mut j = i;
        while j > 0 && freq[hs.syms[j - 1] as usize] > f {
            hs.syms[j] = hs.syms[j - 1];
            j -= 1;
        }
        hs.syms[j] = key;
    }
    for i in 0..m {
        hs.w[i] = freq[hs.syms[i] as usize];
    }
    // two-queue construction: leaves 0..m-1, internal nodes m..2m-2
    let (mut c, mut d, mut e) = (0usize, m, m);
    while e < 2 * m - 1 {
        let a = if c < m && (d >= e || hs.w[c] <= hs.w[d]) {
            c += 1;
            c - 1
        } else {
            d += 1;
            d - 1
        };
        let b = if c < m && (d >= e || hs.w[c] <= hs.w[d]) {
            c += 1;
            c - 1
        } else {
            d += 1;
            d - 1
        };
        hs.w[e] = hs.w[a].wrapping_add(hs.w[b]);
        hs.p[a] = e as u16;
        hs.p[b] = e as u16;
        e += 1;
    }
    let root = 2 * m - 2;
    hs.d[root] = 0;
    for i in (0..root).rev() {
        hs.d[i] = hs.d[hs.p[i] as usize] + 1;
    }
    hs.bl = [0; 17];
    for i in 0..m {
        let l = (hs.d[i] as usize).min(maxbits);
        hs.bl[l] += 1;
    }
    // Kraft sum; repair until it is exactly 2^maxbits
    let mut total = 0u32;
    for l in 1..=maxbits {
        total = total.wrapping_add(hs.bl[l] << (maxbits - l));
    }
    let target = 1u32 << maxbits;
    while total > target {
        hs.bl[maxbits] = hs.bl[maxbits].wrapping_sub(1);
        let mut l = maxbits - 1;
        while l != 0 {
            if hs.bl[l] != 0 {
                hs.bl[l] -= 1;
                hs.bl[l + 1] += 2;
                break;
            }
            l -= 1;
        }
        total -= 1;
    }
    // longest codes to the least frequent symbols
    let mut idx = 0usize;
    for l in (1..=maxbits).rev() {
        for _ in 0..hs.bl[l] {
            lens[hs.syms[idx] as usize] = l as u8;
            idx += 1;
        }
    }
}

// a zeroed array on the heap (not through the stack)
fn zeroed_box<const N: usize>() -> Box<[u32; N]> {
    vec![0u32; N].into_boxed_slice().try_into().unwrap()
}

/// a compressor state (DS_*); reusable, one thread at a time
pub struct DeflateState {
    head: Box<[u32; 32768]>, // global positions
    prev: Box<[u32; 32768]>,
    syms: Box<[u32; BLOCK_SYMS]>, // lit | dist << 8 (match: len - 3 | dist << 8)
    lfreq: [u32; 288],
    dfreq: [u32; 32],
    clfreq: [u32; 20],
    llen: [u8; 288],
    dlen: [u8; 32],
    cllen: [u8; 24],
    lcode: [u16; 288],
    dcode: [u16; 32],
    clcode: [u16; 24],
    alll: [u8; 320],
    rle: [u16; 320],
    hs: Hs,
    out: usize,    // output cursor (index into the output slice)
    bitbuf: u64,
    bitcnt: u32,
    nsym: usize,
    bstart: usize, // input position of the block
    bbytes: usize, // input bytes of the block
    nrle: usize,
    hlit: usize,
    hdist: usize,
    hclen: usize,
    good: u32,
    lazy: u32,
    nice: u32,
    chain: u32,
    mode: u32,
    base: u64,   // global position of input byte 0
    limit: usize, // no block may end after this index
    ovf: bool,
}

impl DeflateState {
    /// a zeroed state  [deflate_state_new]
    pub fn new() -> Box<DeflateState> {
        Box::new(DeflateState {
            head: zeroed_box(),
            prev: zeroed_box(),
            syms: zeroed_box(),
            lfreq: [0; 288],
            dfreq: [0; 32],
            clfreq: [0; 20],
            llen: [0; 288],
            dlen: [0; 32],
            cllen: [0; 24],
            lcode: [0; 288],
            dcode: [0; 32],
            clcode: [0; 24],
            alll: [0; 320],
            rle: [0; 320],
            hs: Hs { syms: [0; 288], w: [0; 576], p: [0; 576], d: [0; 576], bl: [0; 17] },
            out: 0,
            bitbuf: 0,
            bitcnt: 0,
            nsym: 0,
            bstart: 0,
            bbytes: 0,
            nrle: 0,
            hlit: 0,
            hdist: 0,
            hclen: 0,
            good: 0,
            lazy: 0,
            nice: 0,
            chain: 0,
            mode: 0,
            base: 0,
            limit: 0,
            ovf: false,
        })
    }

    // bw_put: append bits (LSB first); 8-byte store at the cursor
    #[inline(always)]
    fn put(&mut self, o: &mut [u8], v: u64, n: u32) {
        let x = v << self.bitcnt | self.bitbuf;
        let c = self.bitcnt + n;
        o[self.out..self.out + 8].copy_from_slice(&x.to_le_bytes());
        let k = c >> 3;
        self.out += k as usize;
        self.bitcnt = c & 7;
        self.bitbuf = x >> (k * 8);
    }

    // bw_align: pad the pending bits with zeroes to a byte boundary
    fn align(&mut self, o: &mut [u8]) {
        if self.bitcnt != 0 {
            o[self.out..self.out + 8].copy_from_slice(&self.bitbuf.to_le_bytes());
            self.out += ((self.bitcnt + 7) >> 3) as usize;
            self.bitbuf = 0;
            self.bitcnt = 0;
        }
    }

    // defl_stored: len bytes as stored blocks of <= 65535 bytes
    fn stored(&mut self, o: &mut [u8], data: &[u8], fin: bool) {
        let mut rest = data;
        loop {
            let n = rest.len().min(65535);
            let last = n == rest.len();
            self.put(o, (last && fin) as u64, 3);
            self.align(o);
            let p = self.out;
            o[p..p + 2].copy_from_slice(&(n as u16).to_le_bytes());
            o[p + 2..p + 4].copy_from_slice(&(!(n as u16)).to_le_bytes());
            o[p + 4..p + 4 + n].copy_from_slice(&rest[..n]);
            self.out = p + 4 + n;
            rest = &rest[n..];
            if last {
                break;
            }
        }
    }

    // defl_emit: the buffered symbols and the end-of-block code with the given tables
    fn emit(&mut self, o: &mut [u8], lcode: &[u16], llen: &[u8], dcode: &[u16], dlen: &[u8]) {
        let mut out = self.out;
        let mut bb = self.bitbuf;
        let mut bc = self.bitcnt;
        for i in 0..self.nsym {
            let s = self.syms[i];
            if s < 256 {
                bb |= (lcode[s as usize] as u64) << bc;
                bc += llen[s as usize] as u32;
            } else {
                let dist = s >> 8;
                let l3 = (s & 255) as usize; // length - 3
                let lc = LCOF[l3] as usize;
                let ex = (l3 + 3 - LBASE[lc] as usize) as u64;
                let ll = llen[257 + lc] as u32;
                let v = ex << ll | lcode[257 + lc] as u64;
                bb |= v << bc;
                bc += LEXT[lc] as u32 + ll;
                let dm1 = dist - 1;
                let di = if dm1 < 256 { dm1 } else { (dm1 >> 7) + 256 } as usize;
                let dc = DCOF[di] as usize;
                let dex = (dist - DBASE[dc] as u32) as u64;
                let dl = dlen[dc] as u32;
                let v = dex << dl | dcode[dc] as u64;
                bb |= v << bc;
                bc += DEXT[dc] as u32 + dl;
            }
            o[out..out + 8].copy_from_slice(&bb.to_le_bytes());
            let k = bc >> 3;
            out += k as usize;
            bb >>= k * 8;
            bc &= 7;
        }
        bb |= (lcode[256] as u64) << bc;
        bc += llen[256] as u32;
        o[out..out + 8].copy_from_slice(&bb.to_le_bytes());
        let k = bc >> 3;
        out += k as usize;
        bb >>= k * 8;
        bc &= 7;
        self.out = out;
        self.bitbuf = bb;
        self.bitcnt = bc;
    }

    // defl_flush: the buffered symbols as the cheapest of stored / fixed / dynamic, then a
    // new block; a block that does not fit sets ovf (nothing written)
    fn flush(&mut self, o: &mut [u8], src: &[u8], fin: bool) {
        self.lfreq[256] = 1;
        huff_lengths(&self.lfreq[..286], 15, &mut self.llen, &mut self.hs);
        huff_lengths(&self.dfreq[..30], 15, &mut self.dlen, &mut self.hs);
        let mut hlit = 286;
        while hlit > 257 && self.llen[hlit - 1] == 0 {
            hlit -= 1;
        }
        self.hlit = hlit;
        let mut hdist = 30;
        while hdist > 1 && self.dlen[hdist - 1] == 0 {
            hdist -= 1;
        }
        self.hdist = hdist;
        self.alll[..hlit].copy_from_slice(&self.llen[..hlit]);
        self.alll[hlit..hlit + hdist].copy_from_slice(&self.dlen[..hdist]);
        self.clfreq = [0; 20];
        // run-length code the lengths (16 = repeat previous 3-6, 17 = 3-10 zeroes,
        // 18 = 11-138 zeroes)
        let total = hlit + hdist;
        let mut i = 0;
        let mut ops = 0;
        while i < total {
            let v = self.alll[i];
            let mut j = i + 1;
            while j < total && self.alll[j] == v {
                j += 1;
            }
            let mut run = j - i;
            i = j;
            if v == 0 {
                while run >= 11 {
                    let k = run.min(138);
                    run -= k;
                    self.rle[ops] = (((k - 11) << 8) | 18) as u16;
                    ops += 1;
                    self.clfreq[18] += 1;
                }
                if run >= 3 {
                    self.rle[ops] = (((run - 3) << 8) | 17) as u16;
                    ops += 1;
                    self.clfreq[17] += 1;
                    continue;
                }
                while run > 0 {
                    self.rle[ops] = 0;
                    ops += 1;
                    self.clfreq[0] += 1;
                    run -= 1;
                }
            } else {
                self.rle[ops] = v as u16;
                ops += 1;
                self.clfreq[v as usize] += 1;
                run -= 1;
                while run >= 3 {
                    let k = run.min(6);
                    run -= k;
                    self.rle[ops] = (((k - 3) << 8) | 16) as u16;
                    ops += 1;
                    self.clfreq[16] += 1;
                }
                while run > 0 {
                    self.rle[ops] = v as u16;
                    ops += 1;
                    self.clfreq[v as usize] += 1;
                    run -= 1;
                }
            }
        }
        self.nrle = ops;
        huff_lengths(&self.clfreq[..19], 7, &mut self.cllen, &mut self.hs);
        let mut hclen = 19;
        while hclen > 4 && self.cllen[CLORDER[hclen - 1] as usize] == 0 {
            hclen -= 1;
        }
        self.hclen = hclen;
        // ---- bit costs
        let mut extra = 0u64;
        let mut dynb = 0u64;
        let mut fixb = 0u64;
        for c in 0..286 {
            let f = self.lfreq[c] as u64;
            dynb += self.llen[c] as u64 * f;
            fixb += FIXLLEN[c] as u64 * f;
        }
        for c in 0..29 {
            extra += self.lfreq[257 + c] as u64 * LEXT[c] as u64;
        }
        for c in 0..30 {
            let f = self.dfreq[c] as u64;
            dynb += self.dlen[c] as u64 * f;
            fixb += 5 * f;
            extra += DEXT[c] as u64 * f;
        }
        let mut hdr = 3 * hclen as u64 + 17;
        for k in 0..ops {
            let s = (self.rle[k] & 255) as usize;
            hdr += self.cllen[s] as u64;
            hdr += match s {
                16 => 2,
                17 => 3,
                18 => 7,
                _ => 0,
            };
        }
        let dynamic = dynb + extra + hdr;
        let fixed = fixb + extra + 3;
        let mut stored = (((self.bitcnt + 3).wrapping_neg() & 7) + 35) as u64 + self.bbytes as u64 * 8;
        if self.bbytes != 0 {
            stored = stored - 40 + (self.bbytes as u64).div_ceil(65535) * 40;
        }
        // ---- output limit: the block costs min(dynamic, fixed, stored) bits
        let cost = dynamic.min(fixed).min(stored);
        let end = self.out as u64 + (cost + self.bitcnt as u64 + 7) / 8;
        if end > self.limit as u64 {
            self.ovf = true;
            return;
        }
        // ---- choose
        let best = if dynamic < fixed { dynamic } else { fixed };
        if stored < best {
            let data = &src[self.bstart..self.bstart + self.bbytes];
            self.stored(o, data, fin);
        } else if fixed <= dynamic {
            self.put(o, fin as u64 | 2, 3);
            self.emit(o, &FIXLCODE, &FIXLLEN, &FIXDCODE, &FIXDLEN);
        } else {
            let (mut lc, mut dc, mut cc) = ([0u16; 288], [0u16; 32], [0u16; 24]);
            huff_codes(&self.llen[..286], &mut lc[..286]);
            huff_codes(&self.dlen[..30], &mut dc[..30]);
            huff_codes(&self.cllen[..19], &mut cc[..19]);
            self.lcode = lc;
            self.dcode = dc;
            self.clcode = cc;
            self.put(o, fin as u64 | 4, 3);
            self.put(o, (hlit - 257) as u64, 5);
            self.put(o, (hdist - 1) as u64, 5);
            self.put(o, (hclen - 4) as u64, 4);
            for &c in &CLORDER[..hclen] {
                let v = self.cllen[c as usize] as u64;
                self.put(o, v, 3);
            }
            for k in 0..ops {
                let s = (self.rle[k] & 255) as usize;
                let x = (self.rle[k] >> 8) as u64;
                let (code, len) = (self.clcode[s] as u64, self.cllen[s] as u32);
                self.put(o, code, len);
                match s {
                    16 => self.put(o, x, 2),
                    17 => self.put(o, x, 3),
                    18 => self.put(o, x, 7),
                    _ => {}
                }
            }
            let (llen, dlen) = (self.llen, self.dlen);
            self.emit(o, &lc, &llen, &dc, &dlen);
        }
        self.bstart += self.bbytes;
        self.bbytes = 0;
        self.nsym = 0;
        self.lfreq = [0; 288];
        self.dfreq = [0; 32];
    }

    // defl_compress: LZ77 over the whole input, blocks flushed as the symbol buffer fills,
    // the final block at the end; stops when a block does not fit (ovf).  The loops run in
    // greedy_run / lazy_run with their state in locals (perf-inflate: no store-forwarding
    // chains through the state, no reloads); they return at every point where the
    // assembly's CHECK_FLUSH finds a full buffer, the flush happens here.
    fn compress(&mut self, src: &[u8], o: &mut [u8]) {
        let end = self.base + src.len() as u64;
        let mut z = Lz { p: self.base, mlen: 2, mdist: 0, avail: false };
        while !if self.mode != 0 { self.lazy_run(src, end, &mut z) } else { self.greedy_run(src, end, &mut z) } {
            self.flush(o, src, false);
            if self.ovf {
                return;
            }
        }
        if self.mode != 0 && z.avail {
            let c = src[(z.p - 1 - self.base) as usize];
            self.syms[self.nsym] = c as u32;
            self.nsym += 1;
            self.lfreq[c as usize] += 1;
            self.bbytes += 1;
        }
        self.flush(o, src, true);
    }

    // the greedy loop (levels 1..3) until the input ends (true) or the symbol buffer is full
    // after a symbol (false).  The hash head of the next position is loaded before this
    // position's chain walk (a literal is the common case; the load then overlaps the walk
    // and its branch misses) and corrected when this position's insert hit the same bucket
    // -- the same candidates as inserting position by position.
    #[inline(never)]
    fn greedy_run(&mut self, src: &[u8], end: u64, z: &mut Lz) -> bool {
        let base = self.base;
        let (chain, lazy, nice) = (self.chain, self.lazy, self.nice);
        let mut nsym = self.nsym;
        let mut bbytes = self.bbytes;
        let head = &mut *self.head;
        let prev = &mut *self.prev;
        let syms = &mut *self.syms;
        let lfreq = &mut self.lfreq;
        let dfreq = &mut self.dfreq;
        let mut p = z.p;
        let mut pre: Option<(usize, u32)> = None; // (bucket, head) of position p
        let done = loop {
            if p >= end {
                break true;
            }
            let i = (p - base) as usize;
            if p + 3 > end {
                lit(syms, lfreq, &mut nsym, &mut bbytes, src[i]);
                p += 1;
                if nsym >= BLOCK_SYMS {
                    break false;
                }
                continue;
            }
            let (h, cand) = match pre.take() {
                Some(x) => x,
                None => {
                    let h = bucket(src, i);
                    (h, head[h])
                }
            };
            // the next position's bucket and head, read before this insert
            let next = if p + 4 <= end {
                let h1 = bucket(src, i + 1);
                Some((h1, head[h1]))
            } else {
                None
            };
            head[h] = p as u32;
            prev[(p as u32 & WMASK) as usize] = cand;
            let maxl = (end - p).min(258) as usize;
            let (l, d) = longest(prev, src, i, maxl, p as u32, cand, 2, chain, nice);
            if d == 0 {
                lit(syms, lfreq, &mut nsym, &mut bbytes, src[i]);
                pre = next.map(|(h1, c1)| (h1, if h1 == h { p as u32 } else { c1 }));
                p += 1;
                if nsym >= BLOCK_SYMS {
                    break false;
                }
                continue;
            }
            mat(syms, lfreq, dfreq, &mut nsym, &mut bbytes, l, d);
            if l > lazy {
                p += l as u64;
            } else {
                let e = p + l as u64;
                p += 1;
                while p < e {
                    if p + 3 <= end {
                        ins(head, prev, src, (p - base) as usize, p as u32);
                    }
                    p += 1;
                }
            }
            if nsym >= BLOCK_SYMS {
                break false;
            }
        };
        z.p = p;
        self.nsym = nsym;
        self.bbytes = bbytes;
        done
    }

    // the lazy loop (levels 4..9) until the input ends (true) or the symbol buffer is full
    // after a symbol (false); a pending literal stays in z.avail.  The next position's hash
    // head is loaded early as in greedy_run (used when no match is emitted here).
    #[inline(never)]
    fn lazy_run(&mut self, src: &[u8], end: u64, z: &mut Lz) -> bool {
        let base = self.base;
        let (chain, lazy, nice, good) = (self.chain, self.lazy, self.nice, self.good);
        let mut nsym = self.nsym;
        let mut bbytes = self.bbytes;
        let head = &mut *self.head;
        let prev = &mut *self.prev;
        let syms = &mut *self.syms;
        let lfreq = &mut self.lfreq;
        let dfreq = &mut self.dfreq;
        let Lz { mut p, mut mlen, mut mdist, mut avail } = *z;
        let mut pre: Option<(usize, u32)> = None; // (bucket, head) of position p
        let done = loop {
            if p >= end {
                break true;
            }
            let i = (p - base) as usize;
            let mut next = None; // (bucket, head) of position p + 1
            let cand = if p + 3 <= end {
                let (h, c) = match pre.take() {
                    Some(x) => x,
                    None => {
                        let h = bucket(src, i);
                        (h, head[h])
                    }
                };
                if p + 4 <= end {
                    let h1 = bucket(src, i + 1);
                    next = Some((h1, head[h1]));
                }
                head[h] = p as u32;
                prev[(p as u32 & WMASK) as usize] = c;
                next = next.map(|(h1, c1)| (h1, if h1 == h { p as u32 } else { c1 }));
                c
            } else {
                pre = None;
                p as u32
            };
            let (plen, pdist) = (mlen, mdist);
            mlen = 2;
            if plen < lazy {
                let mut budget = chain;
                if plen >= good {
                    budget >>= 2;
                }
                let maxl = (end - p).min(258) as usize;
                let (l, d) = longest(prev, src, i, maxl, p as u32, cand, plen, budget, nice);
                if d != 0 && !(l == 3 && d > TOO_FAR) {
                    mlen = l;
                    mdist = d;
                }
            }
            if plen >= 3 && mlen <= plen {
                mat(syms, lfreq, dfreq, &mut nsym, &mut bbytes, plen, pdist);
                let e = p + plen as u64 - 1;
                p += 1;
                while p < e {
                    if p + 3 <= end {
                        ins(head, prev, src, (p - base) as usize, p as u32);
                    }
                    p += 1;
                }
                avail = false;
                mlen = 2;
                if nsym >= BLOCK_SYMS {
                    break false;
                }
                continue;
            }
            pre = next;
            if avail {
                lit(syms, lfreq, &mut nsym, &mut bbytes, src[i - 1]);
                p += 1;
                if nsym >= BLOCK_SYMS {
                    break false;
                }
                continue;
            }
            avail = true;
            p += 1;
        };
        *z = Lz { p, mlen, mdist, avail };
        self.nsym = nsym;
        self.bbytes = bbytes;
        done
    }

    // defl_run: raw deflate data of src at o[out..] (level 0..9); no block may end after
    // lim -> Err(ERR_OVERFLOW); afterwards self.out = the end of the data
    fn run(&mut self, src: &[u8], level: u32, o: &mut [u8], out: usize, lim: usize) -> Result<(), i32> {
        self.out = out;
        self.limit = lim;
        self.bitbuf = 0;
        self.bstart = 0;
        self.bbytes = 0;
        self.bitcnt = 0;
        self.nsym = 0;
        self.ovf = false;
        self.lfreq = [0; 288];
        self.dfreq = [0; 32];
        if self.out > self.limit {
            return Err(ERR_OVERFLOW);
        }
        if level == 0 {
            // 5 bytes per stored piece of <= 65535 bytes (at least one) + the data
            let pieces = src.len().div_ceil(65535).max(1);
            if (self.out + pieces * 5 + src.len()) as u64 > self.limit as u64 {
                return Err(ERR_OVERFLOW);
            }
            self.stored(o, src, true);
        } else {
            let lv = &LEVELS[level as usize];
            self.good = lv[0];
            self.lazy = lv[1];
            self.nice = lv[2];
            self.chain = lv[3];
            self.mode = lv[4];
            if self.base + src.len() as u64 >= POS_LIMIT {
                self.head.fill(0);
                self.base = 0;
            }
            self.compress(src, o);
            self.base += src.len() as u64 + GEN_GAP;
            if self.ovf {
                return Err(ERR_OVERFLOW);
            }
        }
        self.align(o);
        Ok(())
    }
}

// the LZ77 loop state that lives across flushes (position, the lazy loop's match and pending
// literal)
#[derive(Clone, Copy)]
struct Lz {
    p: u64,
    mlen: u32,
    mdist: u32,
    avail: bool,
}

// the hash bucket of the 3 bytes at input byte i (i + 3 <= len)
#[inline(always)]
fn bucket(src: &[u8], i: usize) -> usize {
    let v = if i + 4 <= src.len() {
        u32::from_le_bytes(src[i..i + 4].try_into().unwrap()) & 0xFF_FFFF
    } else {
        let t = &src[i..i + 3];
        t[0] as u32 | (t[1] as u32) << 8 | (t[2] as u32) << 16
    };
    (v.wrapping_mul(HASH_MUL) >> HASH_SHIFT) as usize
}

// .dc_ins: insert the input byte i (global position p, i + 3 <= len) into the hash chains
// -> the previous head
#[inline(always)]
fn ins(head: &mut [u32; 32768], prev: &mut [u32; 32768], src: &[u8], i: usize, p: u32) -> u32 {
    let h = bucket(src, i);
    let old = head[h];
    head[h] = p;
    prev[(p & WMASK) as usize] = old;
    old
}

// .dc_lm: longest match at input byte cur (global position pp, maxl = min(bytes left, 258))
// starting with candidate cand; best = length so far (>= 2), budget = chain depth -> (best
// length, its distance or 0)
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn longest(prev: &[u32; 32768], src: &[u8], cur: usize, maxl: usize, pp: u32, mut cand: u32, mut best: u32, mut budget: u32, nice: u32) -> (u32, u32) {
    let mut dist = 0u32;
    if best as usize >= maxl {
        return (best, 0);
    }
    let curw = &src[cur..cur + maxl];
    // the two bytes at best - 1 that a longer match must have (the assembly's word compare)
    let mut wc = rd16(curw, best as usize - 1);
    let mut cb = cur + best as usize - 1; // a longer match has wc at cb - d too
    loop {
        let d = pp.wrapping_sub(cand);
        if d.wrapping_sub(1) > MAX_DIST - 1 {
            break;
        }
        // the candidate lies in this input (an older one fails the window check)
        if rd16(src, cb - d as usize) == wc {
            let c = cur - d as usize;
            let l = match_len(&src[c..c + maxl], curw) as u32;
            if l > best {
                best = l;
                dist = d;
                if l >= nice || l as usize >= maxl {
                    break;
                }
                wc = rd16(curw, l as usize - 1);
                cb = cur + l as usize - 1;
            }
        }
        budget -= 1;
        if budget == 0 {
            break;
        }
        let nx = prev[(cand & WMASK) as usize];
        if nx >= cand {
            break;
        }
        cand = nx;
    }
    (best, dist)
}

// .dc_emit_lit
#[inline(always)]
fn lit(syms: &mut [u32; BLOCK_SYMS], lfreq: &mut [u32; 288], nsym: &mut usize, bbytes: &mut usize, c: u8) {
    syms[*nsym & (BLOCK_SYMS - 1)] = c as u32; // (nsym < BLOCK_SYMS: the mask only drops the check)
    *nsym += 1;
    lfreq[c as usize] += 1;
    *bbytes += 1;
}

// .dc_emit_match
#[inline(always)]
fn mat(syms: &mut [u32; BLOCK_SYMS], lfreq: &mut [u32; 288], dfreq: &mut [u32; 32], nsym: &mut usize, bbytes: &mut usize, len: u32, dist: u32) {
    let l3 = len - 3;
    syms[*nsym & (BLOCK_SYMS - 1)] = dist << 8 | l3;
    *nsym += 1;
    *bbytes += len as usize;
    lfreq[257 + LCOF[(l3 & 255) as usize] as usize] += 1; // (len 3..258)
    let dm1 = dist - 1;
    let di = if dm1 < 256 { dm1 } else { (dm1 >> 7) + 256 } as usize;
    dfreq[DCOF[di & 511] as usize] += 1; // (dist 1..32768)
}

// two bytes at i (little endian)
#[inline(always)]
fn rd16(s: &[u8], i: usize) -> u16 {
    u16::from_le_bytes(s[i..i + 2].try_into().unwrap())
}

// the common prefix of a and b (equally long slices)
#[inline(always)]
fn match_len(a: &[u8], b: &[u8]) -> usize {
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    let mut k = 0;
    while k + 8 <= n {
        let x = u64::from_le_bytes(a[k..k + 8].try_into().unwrap());
        let y = u64::from_le_bytes(b[k..k + 8].try_into().unwrap());
        let d = x ^ y;
        if d != 0 {
            return k + (d.trailing_zeros() >> 3) as usize;
        }
        k += 8;
    }
    while k < n && a[k] == b[k] {
        k += 1;
    }
    k
}

/// output capacity with which the *_to functions never report ERR_OVERFLOW  [deflate_bound]
pub fn deflate_bound(len: usize) -> usize {
    len + len / 512 + 128
}

// defl_to: level normalised (< 0 -> 6, > 9 -> 9, inputs of POS_LIMIT bytes and more stored),
// a temporary state when st is None, zlib header / Adler-32 trailer when zlib
fn defl_to(st: Option<&mut DeflateState>, src: &[u8], level: i32, dst: &mut [u8], zlib: bool) -> Result<usize, i32> {
    let mut lv = if level < 0 { 6 } else { level.min(9) as u32 };
    if src.len() as u64 >= POS_LIMIT {
        lv = 0;
    }
    let hdr = if zlib { 2 } else { 0 };
    if dst.len() < hdr + 8 {
        return Err(ERR_OVERFLOW);
    }
    if zlib {
        let h: u16 = if lv < 2 {
            0x0178
        } else if lv < 6 {
            0x5E78
        } else if lv == 6 {
            0x9C78
        } else {
            0xDA78
        };
        dst[..2].copy_from_slice(&h.to_le_bytes());
    }
    let lim = dst.len() - 8;
    let mut tmp;
    let s = match st {
        Some(s) => s,
        None => {
            tmp = DeflateState::new();
            &mut *tmp
        }
    };
    s.run(src, lv, dst, hdr, lim)?;
    let mut n = s.out;
    if zlib {
        dst[n..n + 4].copy_from_slice(&adler32(1, src).to_be_bytes());
        n += 4;
    }
    Ok(n)
}

/// zlib stream into dst  [zlib_deflate_to]
pub fn zlib_deflate_to(st: Option<&mut DeflateState>, src: &[u8], level: i32, dst: &mut [u8]) -> Result<usize, i32> {
    defl_to(st, src, level, dst, true)
}

/// raw deflate data into dst  [deflate_raw_to]
pub fn deflate_raw_to(st: Option<&mut DeflateState>, src: &[u8], level: i32, dst: &mut [u8]) -> Result<usize, i32> {
    defl_to(st, src, level, dst, false)
}

/// zlib stream  [zlib_deflate]
pub fn zlib_deflate(src: &[u8], level: i32) -> Vec<u8> {
    zlib_deflate_st(None, src, level)
}

/// zlib stream with a reusable state (None = a temporary one)  [zlib_deflate_st]
pub fn zlib_deflate_st(st: Option<&mut DeflateState>, src: &[u8], level: i32) -> Vec<u8> {
    let mut buf = vec![0u8; deflate_bound(src.len())];
    let n = defl_to(st, src, level, &mut buf, true).unwrap_or(0);
    buf.truncate(n);
    buf
}

/// raw deflate data  [deflate_raw]
pub fn deflate_raw(src: &[u8], level: i32) -> Vec<u8> {
    deflate_raw_ex(None, src, level, 0, 0)
}

/// raw deflate data with a reusable state  [deflate_raw_st without head / tail]
pub fn deflate_raw_st(st: Option<&mut DeflateState>, src: &[u8], level: i32) -> Vec<u8> {
    deflate_raw_ex(st, src, level, 0, 0)
}

/// raw deflate data after `head` zero bytes (head, tail: 0..65536, larger values clamped);
/// the Vec holds head + the compressed bytes, with room for `tail` more  [deflate_raw_st]
pub fn deflate_raw_ex(st: Option<&mut DeflateState>, src: &[u8], level: i32, head: usize, tail: usize) -> Vec<u8> {
    let head = head.min(65536);
    let tail = tail.min(65536);
    let cap = deflate_bound(src.len()) + tail;
    let mut buf = vec![0u8; head + cap];
    let n = defl_to(st, src, level, &mut buf[head..], false).unwrap_or(0);
    buf.truncate(head + n);
    buf
}
