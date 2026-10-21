// enc -- the zstd encoder: one frame with content size and checksum, blocks of 128 KiB.
//
// Matches come from a hash-chain finder over a window of up to 8 MiB (levels 1..=19 set the
// search depth and how many positions ahead a lazy match may be preferred); the three repeat
// offsets are tried first.  Literals are Huffman coded when that is smaller; each of the three
// sequence codes takes the cheapest of RLE, the predefined distribution and its own table.
// A block that does not get smaller is stored raw; a run of one byte value becomes an RLE
// block.  The decoder's repeat offsets do not move on raw blocks, so the encoder rolls them back.

use alloc::vec;
use alloc::vec::Vec;

use super::bits::Writer;
use super::dec::{BLOCK_MAX, MAGIC};
use super::fse::{self, CTable};
use super::huf;
use super::seq::*;
use crate::xxh::xxh64;

/// a frame stays single-segment (window = content) up to this size
const SINGLE_MAX: usize = 8 << 20;
const WINDOW_LOG: u32 = 23;

struct Params {
    depth: u32,
    lazy: u32,
    hash_log: u32,
    chain_log: u32,
    reps: usize,
    /// the shortest match the finder reports (repeat offsets: 4); longer minimums keep short
    /// matches at far offsets out, which cost more bits than their literals
    min: usize,
    /// enter only a few positions inside a match into the tables; misses before the step
    /// grows (log2) on data without matches
    sparse: bool,
    skip_log: u32,
}

fn params(level: i32) -> Params {
    let l = level.clamp(1, 19) as u32;
    let p = |depth, lazy, hash_log, chain_log, reps, min, sparse, skip_log| Params { depth, lazy, hash_log, chain_log, reps, min, sparse, skip_log };
    match l {
        1 => p(1, 0, 16, 0, 1, 4, true, 4),
        2 => p(1, 0, 17, 0, 1, 5, true, 5),
        3 => p(2, 0, 17, 16, 1, 5, true, 6),
        4 => p(4, 0, 17, 17, 1, 5, true, 6),
        5 => p(4, 1, 18, 17, 3, 5, false, 6),
        6 => p(8, 1, 18, 18, 3, 5, false, 6),
        7 => p(16, 1, 19, 18, 3, 5, false, 7),
        8 => p(24, 2, 19, 19, 3, 5, false, 7),
        9 => p(32, 2, 20, 20, 3, 5, false, 8),
        _ => p(32 + (l - 9) * 32, 2, 20, 21, 3, 5, false, 8),
    }
}

#[inline(always)]
fn rd32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

#[inline(always)]
fn rd64(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3], b[i + 4], b[i + 5], b[i + 6], b[i + 7]])
}

/// equal bytes at a and b (a < b), at most max
#[inline(always)]
fn common(s: &[u8], a: usize, b: usize, max: usize) -> usize {
    let mut n = 0;
    while n + 8 <= max {
        let x = rd64(s, a + n) ^ rd64(s, b + n);
        if x != 0 {
            return n + (x.trailing_zeros() / 8) as usize;
        }
        n += 8;
    }
    while n < max && s[a + n] == s[b + n] {
        n += 1;
    }
    n
}

struct Finder<'a> {
    s: &'a [u8],
    hash: Vec<u32>,
    chain: Vec<u32>,
    hash_log: u32,
    cmask: usize,
    depth: u32,
    window: usize,
    min: usize,
    sparse: bool,
    /// the next position to enter into the tables
    next: usize,
}

impl Finder<'_> {
    #[inline(always)]
    fn h(&self, p: usize) -> usize {
        if self.min == 4 {
            (rd32(self.s, p).wrapping_mul(2_654_435_761) >> (32 - self.hash_log)) as usize
        } else {
            let v = rd32(self.s, p) as u64 | (self.s[p + 4] as u64) << 32;
            (v.wrapping_mul(0xCF1B_BCDC_BB00_0000) >> (64 - self.hash_log)) as usize
        }
    }

    #[inline(always)]
    fn insert(&mut self, q: usize) {
        if q + 8 > self.s.len() {
            return;
        }
        let h = self.h(q);
        if !self.chain.is_empty() {
            self.chain[q & self.cmask] = self.hash[h];
        }
        self.hash[h] = q as u32 + 1;
    }

    /// the longest earlier match at p that ends by `end` -> (length, offset); p is entered
    /// (and the positions before it, unless the finder is sparse: then skipped ones stay out)
    fn find(&mut self, p: usize, end: usize) -> (usize, usize) {
        if self.sparse {
            self.next = self.next.max(p);
        }
        while self.next < p {
            self.insert(self.next);
            self.next += 1;
        }
        let limit = end - p;
        let mut best = (0usize, 0usize);
        let mut cand = self.hash[self.h(p)];
        let mut d = 0;
        while cand != 0 && d < self.depth {
            let c = cand as usize - 1;
            if p - c > self.window {
                break;
            }
            if self.s[c + best.0] == self.s[p + best.0] && rd32(self.s, c) == rd32(self.s, p) {
                let len = 4 + common(self.s, c + 4, p + 4, limit - 4);
                // farther candidates must be longer by enough to pay for their offset bits
                if len >= self.min && (best.0 == 0 || gain(len, p - c, false) > gain(best.0, best.1, false)) {
                    best = (len, p - c);
                    if len == limit {
                        break;
                    }
                }
            }
            d += 1;
            if self.chain.is_empty() || p - c > self.cmask {
                break;
            }
            cand = self.chain[c & self.cmask];
        }
        if self.next == p {
            self.insert(p);
            self.next = p + 1;
        }
        best
    }
}

#[derive(Clone, Copy)]
struct Seq {
    ll: u32,
    ml: u32,
    /// the offset value: offset + 3, or 1..=3 for a repeat offset
    ov: u32,
}

/// the offset value of a match and the repeat offsets the decoder will have afterwards
fn offset_value(rep: &mut [u32; 3], off: u32, ll: u32) -> u32 {
    let r = *rep;
    if ll > 0 {
        if off == r[0] {
            return 1;
        }
        if off == r[1] {
            *rep = [r[1], r[0], r[2]];
            return 2;
        }
        if off == r[2] {
            *rep = [r[2], r[0], r[1]];
            return 3;
        }
    } else {
        if off == r[1] {
            *rep = [r[1], r[0], r[2]];
            return 1;
        }
        if off == r[2] {
            *rep = [r[2], r[0], r[1]];
            return 2;
        }
        if r[0] > 1 && off == r[0] - 1 {
            *rep = [r[0] - 1, r[0], r[1]];
            return 3;
        }
    }
    *rep = [off, r[0], r[1]];
    off + 3
}

#[inline(always)]
fn gain(len: usize, off: usize, rep: bool) -> i64 {
    len as i64 * 4 - if rep { 1 } else { (31 - (off as u32 + 3).leading_zeros()) as i64 }
}

struct Cand {
    len: usize,
    off: usize,
    rep: bool,
}

/// the best match at p: repeat offsets first, then the finder
fn best_at(f: &mut Finder, rep: &[u32; 3], nreps: usize, p: usize, end: usize) -> Cand {
    let s = f.s;
    let mut best = Cand { len: 0, off: 0, rep: false };
    for &r in rep.iter().take(nreps) {
        let r = r as usize;
        if r == 0 || r > p || r > f.window || rd32(s, p - r) != rd32(s, p) {
            continue;
        }
        let len = 4 + common(s, p - r + 4, p + 4, end - p - 4);
        if len > best.len {
            best = Cand { len, off: r, rep: true };
        }
    }
    let (len, off) = f.find(p, end);
    if len >= 4 && gain(len, off, false) > gain(best.len, best.off, best.rep) {
        best = Cand { len, off, rep: false };
    }
    best
}

/// the sequences and literals of one block
fn parse(f: &mut Finder, prm: &Params, bs: usize, be: usize, rep: &mut [u32; 3], seqs: &mut Vec<Seq>, lits: &mut Vec<u8>) {
    let s = f.s;
    let mut p = bs;
    let mut anchor = bs;
    let mut misses = 0usize;
    while p + 8 <= be {
        let mut m = best_at(f, rep, prm.reps, p, be);
        if m.len < 4 {
            misses += 1;
            p += 1 + (misses >> prm.skip_log);
            continue;
        }
        for _ in 0..prm.lazy {
            if p + 9 > be {
                break;
            }
            let m2 = best_at(f, rep, prm.reps, p + 1, be);
            if m2.len >= 4 && gain(m2.len, m2.off, m2.rep) > gain(m.len, m.off, m.rep) + 4 {
                p += 1;
                m = m2;
            } else {
                break;
            }
        }
        // grow the match backwards over equal literals
        let mut start = p;
        while start > anchor && start > m.off && s[start - 1] == s[start - 1 - m.off] {
            start -= 1;
            m.len += 1;
        }
        let ll = (start - anchor) as u32;
        let ov = offset_value(rep, m.off as u32, ll);
        lits.extend_from_slice(&s[anchor..start]);
        seqs.push(Seq { ll, ml: m.len as u32, ov });
        p = start + m.len;
        anchor = p;
        misses = 0;
        if prm.sparse && f.next < p {
            // the start and the end of the match are enough to find it again
            let a = f.next;
            f.insert(a);
            f.insert(a + 1);
            for q in p.saturating_sub(2).max(a + 2)..p {
                f.insert(q);
            }
            f.next = p;
        }
    }
    lits.extend_from_slice(&s[anchor..be]);
}

fn raw_lit_header(out: &mut Vec<u8>, ty: u8, n: usize) {
    if n < 32 {
        out.push(ty | (n as u8) << 3);
    } else if n < 4096 {
        out.extend_from_slice(&[ty | 1 << 2 | ((n & 15) as u8) << 4, (n >> 4) as u8]);
    } else {
        out.extend_from_slice(&[ty | 3 << 2 | ((n & 15) as u8) << 4, (n >> 4) as u8, (n >> 12) as u8]);
    }
}

fn literals(lits: &[u8], out: &mut Vec<u8>) {
    let n = lits.len();
    if n > 1 && lits.iter().all(|&b| b == lits[0]) {
        raw_lit_header(out, 1, n);
        out.push(lits[0]);
        return;
    }
    let raw_size = n + if n < 32 { 1 } else if n < 4096 { 2 } else { 3 };
    if n >= 64 {
        let mut hist = [0u32; 256];
        for &b in lits {
            hist[b as usize] += 1;
        }
        if let Some(code) = huf::Code::build(&hist) {
            if (code.cost(&hist) / 8) as usize + 16 < n {
                let mut body = Vec::new();
                if code.write_table(&mut body) {
                    let streams = if n > 1023 { 4 } else { 1 };
                    if streams == 1 {
                        body.extend_from_slice(&code.encode(lits));
                    } else {
                        let seg = n.div_ceil(4);
                        let parts: Vec<Vec<u8>> = (0..4).map(|k| code.encode(&lits[(k * seg).min(n)..((k + 1) * seg).min(n)])).collect();
                        for p in &parts[..3] {
                            body.extend_from_slice(&(p.len() as u16).to_le_bytes());
                        }
                        for p in &parts {
                            body.extend_from_slice(p);
                        }
                    }
                    let c = body.len();
                    let big = n.max(c);
                    let (sf, hs, bits) = if streams == 1 { (0, 3, 10) } else if big <= 1023 { (1, 3, 10) } else if big <= 16383 { (2, 4, 14) } else { (3, 5, 18) };
                    if (big >> bits) == 0 && hs + c < raw_size {
                        let h = 2u64 | (sf as u64) << 2 | (n as u64) << 4 | (c as u64) << (4 + bits);
                        out.extend_from_slice(&h.to_le_bytes()[..hs]);
                        out.extend_from_slice(&body);
                        return;
                    }
                }
            }
        }
    }
    raw_lit_header(out, 0, n);
    out.extend_from_slice(lits);
}

/// the cheapest way to code one of the three sequence codes -> (mode, table, description)
fn choose(hist: &[u32], n: usize, def: &[i16], def_log: u32, max_log: u32) -> Option<(u8, CTable, Vec<u8>)> {
    let used = hist.iter().filter(|&&c| c > 0).count();
    let top = hist.iter().rposition(|&c| c > 0)?;
    if used == 1 {
        return Some((1, CTable::rle(), vec![top as u8]));
    }
    let mut best: Option<(u64, u8, CTable, Vec<u8>)> = None;
    if let Some(c) = fse::cost(hist, def, def_log) {
        best = Some((c, 0, CTable::build(def, def_log)?, Vec::new()));
    }
    let log = fse::choose_log(n, top, max_log);
    let norm = fse::normalize(&hist[..=top], log);
    if let Some(ct) = CTable::build(&norm, log) {
        let mut d = Vec::new();
        fse::write_ncount(&mut d, &norm, log);
        if let Some(c) = fse::cost(hist, &norm, log) {
            let c = c + d.len() as u64 * 8 * 256;
            if best.as_ref().is_none_or(|b| c < b.0) {
                best = Some((c, 2, ct, d));
            }
        }
    }
    best.map(|(_, m, t, d)| (m, t, d))
}

fn sequences(seqs: &[Seq], out: &mut Vec<u8>) -> Option<()> {
    let n = seqs.len();
    if n < 128 {
        out.push(n as u8);
    } else if n < 0x7F00 {
        out.extend_from_slice(&[(n >> 8) as u8 + 128, n as u8]);
    } else {
        out.push(255);
        out.extend_from_slice(&((n - 0x7F00) as u16).to_le_bytes());
    }
    if n == 0 {
        return Some(());
    }
    let mut llc = Vec::with_capacity(n);
    let mut mlc = Vec::with_capacity(n);
    let mut ofc = Vec::with_capacity(n);
    let (mut hl, mut hm, mut ho) = ([0u32; 36], [0u32; 53], [0u32; 32]);
    for q in seqs {
        let (a, b, c) = (ll_code(q.ll), ml_code(q.ml), of_code(q.ov));
        hl[a as usize] += 1;
        hm[b as usize] += 1;
        ho[c as usize] += 1;
        llc.push(a);
        mlc.push(b);
        ofc.push(c);
    }
    let (mode_l, tl, dl) = choose(&hl, n, &LL_DEFAULT, LL_DEFAULT_LOG, LL_LOG_MAX)?;
    let (mode_o, to, d_o) = choose(&ho, n, &OF_DEFAULT, OF_DEFAULT_LOG, OF_LOG_MAX)?;
    let (mode_m, tm, dm) = choose(&hm, n, &ML_DEFAULT, ML_DEFAULT_LOG, ML_LOG_MAX)?;
    out.push(mode_l << 6 | mode_o << 4 | mode_m << 2);
    out.extend_from_slice(&dl);
    out.extend_from_slice(&d_o);
    out.extend_from_slice(&dm);
    let mut w = Writer::new();
    let last = n - 1;
    let (mut sl, mut so, mut sm) = (tl.init(llc[last]), to.init(ofc[last]), tm.init(mlc[last]));
    let extras = |w: &mut Writer, i: usize| {
        let q = seqs[i];
        let (a, b, c) = (llc[i] as usize, mlc[i] as usize, ofc[i] as u32);
        w.add((q.ll - LL_BASE[a]) as u64, LL_BITS[a] as u32);
        w.add((q.ml - ML_BASE[b]) as u64, ML_BITS[b] as u32);
        w.add((q.ov - (1 << c)) as u64, c);
    };
    extras(&mut w, last);
    for i in (0..last).rev() {
        so = to.encode(&mut w, so, ofc[i]);
        sm = tm.encode(&mut w, sm, mlc[i]);
        sl = tl.encode(&mut w, sl, llc[i]);
        extras(&mut w, i);
    }
    tm.flush(&mut w, sm);
    to.flush(&mut w, so);
    tl.flush(&mut w, sl);
    out.extend_from_slice(&w.close());
    Some(())
}

fn block_header(out: &mut Vec<u8>, last: bool, ty: u32, size: usize) {
    let h = u32::from(last) | ty << 1 | (size as u32) << 3;
    out.extend_from_slice(&h.to_le_bytes()[..3]);
}

/// the largest frame compress() makes from n bytes
pub const fn bound(n: usize) -> usize {
    n + (n / BLOCK_MAX + 1) * 3 + 18 + 4
}

/// one zstd frame
pub fn compress(src: &[u8], level: i32) -> Vec<u8> {
    let prm = params(level);
    let n = src.len();
    let mut out = Vec::with_capacity(bound(n).min(n / 2 + 4096));
    out.extend_from_slice(&MAGIC.to_le_bytes());
    let single = n <= SINGLE_MAX;
    let (fcs_flag, fcs_bytes) = if single && n < 256 {
        (0u8, 1usize)
    } else if (256..65536 + 256).contains(&n) {
        (1, 2)
    } else if n as u64 <= u32::MAX as u64 {
        (2, 4)
    } else {
        (3, 8)
    };
    out.push(fcs_flag << 6 | u8::from(single) << 5 | 0x04);
    if !single {
        out.push(((WINDOW_LOG - 10) << 3) as u8);
    }
    let fcs = if fcs_bytes == 2 { n as u64 - 256 } else { n as u64 };
    out.extend_from_slice(&fcs.to_le_bytes()[..fcs_bytes]);
    if n == 0 {
        block_header(&mut out, true, 0, 0);
    } else {
        // tables no larger than the input needs
        let bits = 64 - (n as u64 | 1 << 10).leading_zeros();
        let hash_log = prm.hash_log.min(bits + 1);
        let chain_log = prm.chain_log.min(bits);
        let mut f = Finder {
            s: src,
            hash: vec![0; 1 << hash_log],
            chain: if prm.chain_log == 0 { Vec::new() } else { vec![0; 1 << chain_log] },
            hash_log,
            cmask: (1usize << chain_log) - 1,
            depth: prm.depth,
            window: if single { n } else { (1 << WINDOW_LOG) - 1 },
            min: prm.min,
            sparse: prm.sparse,
            next: 0,
        };
        let mut rep = [1u32, 4, 8];
        let mut seqs = Vec::new();
        let mut lits = Vec::new();
        let mut body = Vec::new();
        let mut bs = 0;
        while bs < n {
            let be = (bs + BLOCK_MAX).min(n);
            let last = be == n;
            let b = &src[bs..be];
            if b.len() > 16 && b.iter().all(|&x| x == b[0]) {
                block_header(&mut out, last, 1, b.len());
                out.push(b[0]);
                bs = be;
                continue;
            }
            let saved = rep;
            seqs.clear();
            lits.clear();
            body.clear();
            parse(&mut f, &prm, bs, be, &mut rep, &mut seqs, &mut lits);
            literals(&lits, &mut body);
            if sequences(&seqs, &mut body).is_some() && body.len() < b.len() {
                block_header(&mut out, last, 2, body.len());
                out.extend_from_slice(&body);
            } else {
                rep = saved;
                block_header(&mut out, last, 0, b.len());
                out.extend_from_slice(b);
            }
            bs = be;
        }
    }
    out.extend_from_slice(&(xxh64(src, 0) as u32).to_le_bytes());
    out
}
