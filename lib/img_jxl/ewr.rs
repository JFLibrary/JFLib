// ewr -- JPEG XL writer: bit writer, hybrid uint tokens, length-limited Huffman codes, prefix
// codes and histogram sets (model: tools/jxl_enc_ref.py BW, hybrid_token, huffman_lengths,
// canonical_codes, write_prefix, Histos; byte-exact)                     [src/img/jxl_ewr.s]
//
//   Jw                          bit writer: bits(v, n) (n <= 56, LSB first), u32(v, dist),
//                               u64(v), enum_(v), vu8(v), vu16(v), pad(), data(src),
//                               code(code, len) (prefix code word, MSB first)
//   je_token(v) -> (token, bits, count)   HybridUintConfig( 4, 2, 0 )
//   jh_lengths(counts, limit, out)        Huffman code lengths (length limited)
//   jh_codes(lengths, out)                canonical codes of the lengths
//   jp_write(jw, lengths, single)         prefix code of an alphabet of >= 2 symbols
//   Eh                          histogram set: new(nctx, fixmap) (clusters for the used
//                               contexts, nctx <= 16, or a fixed u8 context map), add(ctx, v),
//                               finish() -> 0 / ERR_UNSUPPORTED, write(jw) -> 0 / ERR_*,
//                               put(jw, ctx, v) (prefix mode)
//   Es                          token stream: put(eh, jw, ctx, v), flush(eh, jw) (ANS: rANS
//                               backwards at the flush)
//
// The encoders only use ANS histogram sets (the assembly never clears EH_ANS); the prefix
// code writer is kept as in the assembly.

use alloc::vec;
use alloc::vec::Vec;

use img_core::ERR_UNSUPPORTED;

use crate::ent::{ej_alias, Ae};
use crate::tab;

/// tokens per context (HybridUintConfig( 4, 2, 0 ): values < 65536)
pub const EH_NTOK: usize = 64;

/// U32 distributions: 4 x (offset or value, bits)
pub static WD_ENUM: [u32; 8] = [0, 0, 1, 0, 2, 4, 18, 6];
pub static WD_U32EC: [u32; 8] = [0, 0, 1, 0, 2, 4, 1, 12];
static CLORDER: [u8; 18] = [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// static code of code length code lengths 0..5: (LSB-first pattern, bits)
static CLSTATIC: [u8; 12] = [0, 2, 7, 4, 3, 3, 2, 2, 1, 2, 15, 4];

/// the bit writer (JW)
#[derive(Default)]
pub struct Jw {
    pub buf: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Jw {
    pub fn new() -> Jw {
        Jw { buf: Vec::with_capacity(4096), acc: 0, n: 0 }
    }

    /// whole bytes written
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// jw_bits: n <= 56 bits of v, LSB first
    #[inline]
    pub fn bits(&mut self, v: u64, n: u32) {
        let v = v & (1u64 << n).wrapping_sub(1);
        self.acc |= v << self.n;
        self.n += n;
        while self.n >= 8 {
            self.buf.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// jw_u32: the first distribution that holds v (none: nothing written)
    pub fn u32(&mut self, v: u32, dist: &[u32; 8]) {
        for s in 0..4 {
            let off = dist[s * 2];
            let b = dist[s * 2 + 1];
            if b == 0 {
                if v == off {
                    self.bits(s as u64, 2);
                    return;
                }
            } else if v >= off && ((v - off) as u64) < (1u64 << b) {
                self.bits(s as u64, 2);
                self.bits((v - off) as u64, b);
                return;
            }
        }
    }

    /// jw_u64 (v <= 272)
    pub fn u64(&mut self, v: u32) {
        if v == 0 {
            self.bits(0, 2);
        } else if v <= 16 {
            self.bits(1, 2);
            self.bits((v - 1) as u64, 4);
        } else {
            self.bits(2, 2);
            self.bits((v - 17) as u64, 8);
        }
    }

    /// jw_enum
    pub fn enum_(&mut self, v: u32) {
        self.u32(v, &WD_ENUM);
    }

    /// jw_vu16: varlen u16
    pub fn vu16(&mut self, v: u32) {
        if v == 0 {
            self.bits(0, 1);
            return;
        }
        self.bits(1, 1);
        let n = 31 - v.leading_zeros();
        self.bits(n as u64, 4);
        if n != 0 {
            self.bits((v - (1 << n)) as u64, n);
        }
    }

    /// jw_vu8: varlen u8
    pub fn vu8(&mut self, v: u32) {
        if v == 0 {
            self.bits(0, 1);
            return;
        }
        let n = 31 - v.leading_zeros();
        self.bits((n * 2 + 1) as u64, 4);
        if n != 0 {
            self.bits((v - (1 << n)) as u64, n);
        }
    }

    /// jw_pad: zero bits to the byte boundary
    pub fn pad(&mut self) {
        let r = self.n & 7;
        if r != 0 {
            self.bits(0, 8 - r);
        }
    }

    /// jw_data: whole bytes into a byte-aligned writer
    pub fn data(&mut self, src: &[u8]) {
        self.buf.extend_from_slice(src);
    }

    /// bits written so far (jxl-plus)
    pub fn bits_written(&self) -> usize {
        self.buf.len() * 8 + self.n as usize
    }

    /// append the bits of another writer at the current (unaligned) position (jxl-plus)
    pub fn append(&mut self, o: &Jw) {
        for &b in &o.buf {
            self.bits(b as u64, 8);
        }
        if o.n != 0 {
            self.bits(o.acc, o.n);
        }
    }

    /// F16 field of v (finite, |v| <= 65504) (jxl-plus)
    pub fn f16(&mut self, v: f32) {
        let b = v.to_bits();
        let sign = (b >> 31) as u64;
        let a = f32::from_bits(b & 0x7FFF_FFFF);
        let bits: u64 = if a == 0.0 {
            0
        } else {
            let e = ((b >> 23) & 0xFF) as i32 - 127;
            if e < -14 {
                // subnormal half: mantissa = a * 2^24
                let m = (a * 16_777_216.0) as u64;
                m & 0x3FF
            } else {
                let mant = ((b >> 13) & 0x3FF) as u64;
                ((e + 15) as u64) << 10 | mant
            }
        };
        self.bits(sign << 15 | bits, 16);
    }

    /// jw_code: prefix code word, most significant bit first
    pub fn code(&mut self, code: u32, len: u32) {
        let mut r = 0u32;
        let mut c = code;
        for _ in 0..len {
            r = r << 1 | (c & 1);
            c >>= 1;
        }
        self.bits(r as u64, len);
    }
}

/// je_token: (token, extra bits, their count) of HybridUintConfig( 4, 2, 0 )
#[inline]
pub fn je_token(v: u32) -> (u32, u32, u32) {
    if v < 16 {
        return (v, 0, 0);
    }
    let top = 31 - v.leading_zeros();
    let n = top - 2;
    let m = (v >> n) & 3;
    (16 + ((top - 4) << 2) + m, v & ((1 << n) - 1), n)
}

/// jh_lengths: lengths of a Huffman code of the used symbols, at most `limit` bits: every
/// count raised to at least 1, 2, 4, ... until the depth fits; leaves sorted by (count,
/// symbol), two queues, a leaf wins ties
pub fn jh_lengths(counts: &[u32], limit: u32, out: &mut [u8]) {
    let n = counts.len();
    for o in out.iter_mut().take(n) {
        *o = 0;
    }
    let k = counts.iter().filter(|&&c| c != 0).count();
    if !(2..=64).contains(&k) {
        return;
    }
    let mut floor: u64 = 1;
    let mut w = [0u64; 128];
    let mut sym = [0u8; 64];
    let mut a = [0u8; 128];
    let mut b = [0u8; 128];
    let mut d = [0u8; 128];
    loop {
        let mut placed = 0usize;
        for (s, &c) in counts.iter().enumerate() {
            if c == 0 {
                continue;
            }
            let v = (c as u64).max(floor);
            let mut i = placed;
            while i > 0 && w[i - 1] > v {
                w[i] = w[i - 1];
                sym[i] = sym[i - 1];
                i -= 1;
            }
            w[i] = v;
            sym[i] = s as u8;
            placed += 1;
        }
        let (mut li, mut ii, mut made) = (0usize, 0usize, 0usize);
        while made < k - 1 {
            let mut pick = [0usize; 2];
            for p in pick.iter_mut() {
                let leaf = li < k && (ii >= made || w[li] <= w[k + ii]);
                if leaf {
                    *p = li;
                    li += 1;
                } else {
                    *p = k + ii;
                    ii += 1;
                }
            }
            w[k + made] = w[pick[0]] + w[pick[1]];
            a[made] = pick[0] as u8;
            b[made] = pick[1] as u8;
            made += 1;
        }
        d[k + made - 1] = 0;
        let mut j = made;
        while j != 0 {
            j -= 1;
            let dd = d[k + j] + 1;
            d[a[j] as usize] = dd;
            d[b[j] as usize] = dd;
        }
        let deepest = d[..k].iter().copied().max().unwrap_or(0) as u32;
        if deepest <= limit {
            for i in 0..k {
                out[sym[i] as usize] = d[i];
            }
            return;
        }
        floor <<= 1;
    }
}

/// jh_codes: canonical codes: by length, then symbol; first code 0
pub fn jh_codes(lengths: &[u8], out: &mut [u16]) {
    let mut code = 0u32;
    for l in 1..=15u8 {
        for (s, &len) in lengths.iter().enumerate() {
            if len == l {
                out[s] = code as u16;
                code += 1;
            }
        }
        code <<= 1;
    }
}

/// jp_write: ReadPrefixCode inverse (n >= 2)
pub fn jp_write(jw: &mut Jw, lengths: &[u8], single: u32) {
    let n = lengths.len();
    let mut syms = [0u8; 4];
    let mut nu = 0usize;
    let mut last = 0usize;
    for (s, &l) in lengths.iter().enumerate() {
        if l != 0 {
            if nu < 4 {
                syms[nu] = s as u8;
            }
            nu += 1;
            last = s;
        }
    }
    if nu <= 4 {
        // simple code: (1, 2), n - 1, symbols sorted by (length, symbol), tree select
        if nu == 0 {
            syms[0] = single as u8;
            nu = 1;
        }
        for i in 1..nu {
            let mut j = i;
            while j > 0 && lengths[syms[j - 1] as usize] > lengths[syms[j] as usize] {
                syms.swap(j - 1, j);
                j -= 1;
            }
        }
        jw.bits(1, 2);
        jw.bits((nu - 1) as u64, 2);
        let maxbits = 32 - ((n - 1) as u32).leading_zeros();
        for &s in syms.iter().take(nu) {
            jw.bits(s as u64, maxbits);
        }
        if nu == 4 {
            jw.bits((lengths[syms[0] as usize] == 1) as u64, 1);
        }
        return;
    }
    // complex code
    let mut clc = [0u32; 18];
    for &l in lengths.iter().take(last + 1) {
        clc[l as usize] += 1;
    }
    let mut cl = [0u8; 18];
    jh_lengths(&clc, 5, &mut cl);
    let mut nzc = 0;
    let mut lastc = 0;
    for (i, &c) in clc.iter().enumerate() {
        if c != 0 {
            nzc += 1;
            lastc = i;
        }
    }
    if nzc == 1 {
        cl[lastc] = 1; // (one symbol: any non-zero length)
    }
    let hskip = if cl[1] != 0 || cl[2] != 0 {
        0
    } else if cl[3] != 0 {
        2
    } else {
        3
    };
    jw.bits(hskip, 2);
    let mut space: i32 = 32;
    for &o in CLORDER.iter().skip(hskip as usize) {
        let l = cl[o as usize] as usize;
        jw.bits(CLSTATIC[l * 2] as u64, CLSTATIC[l * 2 + 1] as u32);
        if l != 0 {
            space -= 32 >> l;
            if space == 0 {
                break;
            }
        }
    }
    if nzc == 1 {
        return;
    }
    let mut codes = [0u16; 18];
    jh_codes(&cl, &mut codes);
    for &l in lengths.iter().take(last + 1) {
        jw.code(codes[l as usize] as u32, cl[l as usize] as u32);
    }
}

/// a histogram set (EH)
pub struct Eh {
    pub nctx: u32,
    pub ncl: u32,
    pub err: u32,
    pub map: [u8; 16],
    pub src: [u8; 8],
    pub ans: u32,
    pub asz: [u32; 8],
    pub single: [u32; 8],
    pub has: [u32; 8],
    pub loga: u32,
    pub fix: Option<Vec<u8>>,
    pub cnt: [[u32; EH_NTOK]; 16],
    pub len: [[u8; EH_NTOK]; 8],
    pub code: [[u16; EH_NTOK]; 8],
    pub freq: [[u32; EH_NTOK]; 8],
    pub cs: [[u16; EH_NTOK]; 8],
    pub slot: Vec<[u16; 4096]>, // 8
}

impl Eh {
    /// eh_init: a histogram set (ANS) of nctx contexts: fix None = clusters for the used
    /// contexts in order (nctx <= 16), else u8[nctx] context -> cluster (< 8, every one used)
    pub fn new(nctx: u32, fix: Option<Vec<u8>>) -> Eh {
        Eh {
            nctx,
            ncl: 0,
            err: 0,
            map: [0; 16],
            src: [0; 8],
            ans: 1,
            asz: [0; 8],
            single: [0; 8],
            has: [0; 8],
            loga: 0,
            fix,
            cnt: [[0; EH_NTOK]; 16],
            len: [[0; EH_NTOK]; 8],
            code: [[0; EH_NTOK]; 8],
            freq: [[0; EH_NTOK]; 8],
            cs: [[0; EH_NTOK]; 8],
            slot: vec![[0u16; 4096]; 8],
        }
    }

    /// eh_cluster: the counting slot / cluster of a context
    #[inline]
    fn cluster(&self, ctx: u32) -> usize {
        match &self.fix {
            Some(f) => (f.get(ctx as usize).copied().unwrap_or(0) & 7) as usize,
            None => (ctx & 15) as usize,
        }
    }

    /// the cluster of a context for the token stream
    #[inline]
    fn put_cluster(&self, ctx: u32) -> usize {
        let c = self.cluster(ctx);
        if self.fix.is_some() { c } else { (self.map[c] & 7) as usize }
    }

    /// eh_add: count the token of v
    #[inline]
    pub fn add(&mut self, ctx: u32, v: u32) {
        let s = self.cluster(ctx);
        let (t, _, _) = je_token(v);
        if t as usize >= EH_NTOK {
            self.err = 1;
            return;
        }
        self.cnt[s][t as usize] += 1;
    }

    /// eh_finish: clusters (<= 8), frequencies, slots (prefix mode: code lengths, codes)
    pub fn finish(&mut self) -> i32 {
        if self.err != 0 {
            return ERR_UNSUPPORTED;
        }
        let ncl: usize;
        if let Some(f) = &self.fix {
            let mut mx = 0u8;
            for &v in f.iter().take(self.nctx as usize) {
                mx = mx.max(v & 7);
            }
            ncl = mx as usize + 1;
            for i in 0..ncl {
                self.src[i] = i as u8;
            }
        } else {
            let mut n = 0usize;
            for c in 0..self.nctx as usize {
                self.map[c] = 0;
                if self.cnt[c].iter().any(|&v| v != 0) {
                    if n >= 8 {
                        return ERR_UNSUPPORTED;
                    }
                    self.map[c] = n as u8;
                    self.src[n] = c as u8;
                    n += 1;
                }
            }
            if n == 0 {
                n = 1; // no tokens: one cluster with the symbol 0
                self.src[0] = 0;
            }
            ncl = n;
        }
        self.ncl = ncl as u32;
        self.loga = 5;
        for cl in 0..ncl {
            let s = self.src[cl] as usize;
            if self.cnt[s].iter().all(|&v| v == 0) {
                self.cnt[s][0] = 1;
            }
            let counts = self.cnt[s];
            let mut asz = EH_NTOK;
            while asz > 0 && counts[asz - 1] == 0 {
                asz -= 1;
            }
            if asz < 1 {
                asz = 1;
            }
            self.asz[cl] = asz as u32;
            let (mut mx, mut mi) = (0u32, 0u32);
            for (i, &c) in counts.iter().enumerate().take(asz) {
                if c > mx {
                    mx = c;
                    mi = i as u32;
                }
            }
            self.single[cl] = mi;
            if asz > 1 {
                let l = 32 - ((asz - 1) as u32).leading_zeros();
                if l > self.loga {
                    self.loga = l;
                }
            }
            if self.ans != 0 {
                let mut fr = [0u32; EH_NTOK];
                eh_norm(&counts[..asz], &mut fr[..asz]);
                self.freq[cl] = fr;
            } else {
                let mut l = [0u8; EH_NTOK];
                jh_lengths(&counts[..asz], 15, &mut l[..asz]);
                self.len[cl] = l;
                let mut cd = [0u16; EH_NTOK];
                jh_codes(&l[..asz], &mut cd);
                self.code[cl] = cd;
                let nz = l[..asz].iter().filter(|&&x| x != 0).count();
                self.has[cl] = (nz > 1) as u32;
            }
        }
        if self.ans != 0 {
            // per cluster: alias table -> slot of (symbol, offset)
            let le = 12 - self.loga;
            let mask = (1u32 << le) - 1;
            let mut at = [Ae::default(); 256];
            for cl in 0..ncl {
                let mut scr = [0u32; 260];
                scr[..EH_NTOK].copy_from_slice(&self.freq[cl]);
                ej_alias(&mut scr, self.asz[cl], self.loga, &mut at);
                let mut run = 0u32;
                for c in 0..EH_NTOK {
                    self.cs[cl][c] = run as u16;
                    run = run.wrapping_add(self.freq[cl][c]);
                }
                for s in 0..4096u32 {
                    let i = s >> le;
                    let pos = s & mask;
                    let e = at[i as usize];
                    let (sym, off) = if pos < e.cutoff as u32 {
                        if i as usize >= EH_NTOK {
                            continue;
                        }
                        (i, pos)
                    } else {
                        (e.right as u32, e.offset as u32 + pos)
                    };
                    let k = (off + self.cs[cl][(sym as usize) & (EH_NTOK - 1)] as u32) & 4095;
                    self.slot[cl][k as usize] = s as u16;
                }
            }
        }
        0
    }

    /// eh_write: DecodeHistograms layout (no LZ77)
    pub fn write(&self, jw: &mut Jw) -> i32 {
        jw.bits(0, 1); // no LZ77
        if self.nctx > 1 {
            let r = match &self.fix {
                Some(f) => em_ctxmap(jw, &f[..self.nctx as usize]),
                None => em_ctxmap(jw, &self.map[..self.nctx as usize]),
            };
            if r != 0 {
                return r;
            }
        }
        let ncl = self.ncl as usize;
        if self.ans != 0 {
            jw.bits(((self.loga - 5) * 2) as u64, 3);
            let se = 32 - self.loga.leading_zeros();
            for _ in 0..ncl {
                jw.bits((2u64 << se) | 4, se + 5);
            }
            for cl in 0..ncl {
                eh_ansh(jw, &self.freq[cl][..self.asz[cl] as usize]);
            }
        } else {
            jw.bits(1, 1);
            for _ in 0..ncl {
                jw.bits(4 | 2 << 4, 9);
            }
            for cl in 0..ncl {
                jw.vu16(self.asz[cl] - 1);
            }
            for cl in 0..ncl {
                let n = self.asz[cl] as usize;
                if n > 1 {
                    jp_write(jw, &self.len[cl][..n], self.single[cl]);
                }
            }
        }
        0
    }

    /// eh_put: prefix code mode: v in context ctx
    pub fn put(&self, jw: &mut Jw, ctx: u32, v: u32) {
        let cl = self.put_cluster(ctx);
        let (t, bits, n) = je_token(v);
        if t as usize >= EH_NTOK {
            return;
        }
        if self.has[cl] != 0 {
            jw.code(self.code[cl][t as usize] as u32, self.len[cl][t as usize] as u32);
        }
        if n != 0 {
            jw.bits(bits as u64, n);
        }
    }
}

/// eh_norm: ans_normalize: floor( c * 4096 / total ) (used symbols >= 1), the rest to the
/// first most frequent symbol; while that is < 1 or the sum > 4096, the first largest
/// frequency loses 1
fn eh_norm(counts: &[u32], freq: &mut [u32]) {
    let n = counts.len();
    let mut total: u64 = 0;
    let mut top = 0usize;
    for i in 0..n {
        total += counts[i] as u64;
        if counts[i] > counts[top] {
            top = i;
        }
    }
    let mut sum = 0u32;
    for i in 0..n {
        let c = counts[i];
        let mut f = 0u32;
        if c != 0 {
            f = (((c as u64) << 12) / total) as u32;
            if f < 1 {
                f = 1;
            }
        }
        freq[i] = f;
        sum = sum.wrapping_add(f);
    }
    freq[top] = freq[top].wrapping_add(4096u32.wrapping_sub(sum));
    loop {
        if (freq[top] as i32) < 1 {
            freq[top] = 1;
        } else {
            let s: i32 = freq.iter().fold(0i32, |a, &v| a.wrapping_add(v as i32));
            if s <= 4096 {
                return;
            }
        }
        let mut b = 0usize;
        for i in 0..n {
            if (freq[i] as i32) > (freq[b] as i32) {
                b = i;
            }
        }
        freq[b] = freq[b].wrapping_sub(1);
    }
}

/// eh_ansh: ReadHistogram inverse: <= 2 used symbols simple, else shift 13
fn eh_ansh(jw: &mut Jw, freq: &[u32]) {
    let n = freq.len();
    let mut used = [0u32; 2];
    let mut nu = 0usize;
    for (i, &f) in freq.iter().enumerate() {
        if f != 0 {
            if nu < 2 {
                used[nu] = i as u32;
            }
            nu += 1;
        }
    }
    if nu <= 2 {
        // simple: 1, n - 1, symbols (varlen u8), count0
        jw.bits((nu * 2).wrapping_sub(1) as u64, 2);
        jw.vu8(used[0]);
        if nu == 2 {
            jw.vu8(used[1]);
            jw.bits(freq[used[0] as usize] as u64, 12);
        }
        return;
    }
    // 0 (not simple), 0 (not flat), log 1 1 1 (3 bits), 6 (3 bits): shift 13
    jw.bits(0b110111 << 2, 8);
    let na = n.max(3);
    jw.vu8((na - 3) as u32);
    let mut lc = [0u8; 64];
    let mut mx = 0u32;
    let mut omit = 0usize;
    for i in 0..na {
        let mut l = 0u32;
        if i < n && freq[i] != 0 {
            l = 32 - freq[i].leading_zeros();
        }
        lc[i] = l as u8;
        if l > mx {
            mx = l;
            omit = i;
        }
    }
    for &l in lc.iter().take(na) {
        let k = l as usize;
        jw.bits(tab::HISTCODE[k * 2] as u64, tab::HISTCODE[k * 2 + 1] as u32);
    }
    for i in 0..n {
        if i == omit {
            continue;
        }
        let l = lc[i] as u32;
        if l <= 1 {
            continue;
        }
        let c = l - 1;
        jw.bits((freq[i] - (1 << c)) as u64, c);
    }
}

/// em_ctxmap: DecodeContextMap inverse: n <= 16 simple (bits per entry), else move-to-front
/// and one ANS context
fn em_ctxmap(jw: &mut Jw, map: &[u8]) -> i32 {
    let n = map.len();
    if n <= 16 {
        let mx = map.iter().copied().max().unwrap_or(0) as u32;
        let bpe = if mx == 0 { 0 } else { 32 - mx.leading_zeros() };
        jw.bits((bpe * 2 + 1) as u64, 3);
        if bpe != 0 {
            for &v in map {
                jw.bits(v as u64, bpe);
            }
        }
        return 0;
    }
    jw.bits(2, 2); // 0 (not simple), 1 (move-to-front)
    let mut mtf = [0u8; 256];
    for (i, m) in mtf.iter_mut().enumerate() {
        *m = i as u8;
    }
    let mut val = vec![0u8; n];
    for (i, &v) in map.iter().enumerate() {
        let mut k = 0usize;
        while k < 255 && mtf[k] != v {
            k += 1;
        }
        val[i] = k as u8;
        mtf.copy_within(0..k, 1);
        mtf[0] = v;
    }
    let mut eh = Eh::new(1, None);
    for &v in &val {
        eh.add(0, v as u32);
    }
    let r = eh.finish();
    if r != 0 {
        return r;
    }
    let r = eh.write(jw);
    if r != 0 {
        return r;
    }
    let mut es = Es::new();
    for &v in &val {
        es.put(&eh, jw, 0, v as u32);
    }
    es.flush(&eh, jw);
    0
}

/// a token record of the ANS stream
#[derive(Clone, Copy, Default)]
struct Tok {
    bits: u32,
    cnt: u8,
    cl: u8,
    tok: u8,
    flag: u8,
    chunk: u16,
}

/// a token stream (ES)
#[derive(Default)]
pub struct Es {
    buf: Vec<Tok>,
}

impl Es {
    pub fn new() -> Es {
        Es { buf: Vec::new() }
    }

    /// es_put: prefix mode: written at once; ANS: kept (cluster, token, extra bits)
    #[inline]
    pub fn put(&mut self, eh: &Eh, jw: &mut Jw, ctx: u32, v: u32) {
        if eh.ans == 0 {
            eh.put(jw, ctx, v);
            return;
        }
        let cl = eh.put_cluster(ctx);
        let (t, bits, n) = je_token(v);
        if t as usize >= EH_NTOK {
            return;
        }
        self.buf.push(Tok { bits, cnt: n as u8, cl: cl as u8, tok: t as u8, flag: 0, chunk: 0 });
    }

    /// es_flush: ANS: rANS backwards from state 0x130000 (renormalization chunks of 16 bits),
    /// then the 32-bit state and per token its chunk and extra bits
    pub fn flush(&mut self, eh: &Eh, jw: &mut Jw) {
        if eh.ans == 0 {
            return;
        }
        let mut st: u32 = 0x13_0000;
        for r in self.buf.iter_mut().rev() {
            let cl = r.cl as usize;
            let t = r.tok as usize;
            let f = eh.freq[cl][t];
            if f == 0 {
                continue;
            }
            if st >> 20 >= f {
                r.chunk = st as u16;
                r.flag = 1;
                st >>= 16;
            }
            let q = st / f;
            let m = st % f;
            let k = (m + eh.cs[cl][t] as u32) & 4095;
            st = (q << 12).wrapping_add(eh.slot[cl][k as usize] as u32);
        }
        jw.bits(st as u64, 32);
        for r in &self.buf {
            if r.flag != 0 {
                jw.bits(r.chunk as u64, 16);
            }
            if r.cnt != 0 {
                jw.bits(r.bits as u64, r.cnt as u32);
            }
        }
        self.buf.clear();
    }
}
