// brotli::enc -- Brotli compression (RFC 7932): compress(), and the parts brotli::Encoder
// (senc.rs) shares with it
//
// Levels 0 to 9 match greedily with lazy evaluation: hash chains over the window (deeper by
// level), after the last distances and their neighbours (the short distance codes, a few
// bits each); a candidate scores as in libbrotli (135 per byte, 30 per bit of distance); the
// search skips ahead in data without matches.  Levels 10 and 11 parse near-optimally: every
// position's matches once, then the cheapest path through the meta-block under a cost model,
// at level 11 a second time with the costs of the first path's commands (as libbrotli's
// Zopfli parse).  A position without a match looks for a word of the static dictionary, also
// with a space before it, its first letter in upper case, one of 44 suffixes after it or up
// to 9 of its last letters cut (the transforms of RFC 7932 appendix B).  Each meta-block
// (64 KiB to 1 MiB of input, by level) gets its own prefix codes: the literals 64 contexts
// from the two bytes before them (the UTF-8 context mode for text, the signed one for the
// rest, from level 9 whichever costs less), merged greedily into as few trees as pays, or
// one tree when that takes fewer bits; the distances 4 contexts by copy length, merged the
// same way.  A meta-block that would come out larger than its input is stored.

use alloc::vec;
use alloc::vec::Vec;

use super::tab as T;
use super::{COPY, DICT, INS};

// ---------------------------------------------------------------- levels

pub(super) struct Params {
    /// candidates per position: without chains a bucket of the last `depth` positions of a
    /// hash (a power of two, 1: one position), with chains as many along the chain
    depth: u32,
    hash_log: u32,
    /// the reach of the chains (0: buckets)
    chain_log: u32,
    lazy: bool,
    /// distances of the cache tried first: 1, 4, 10 or 16
    cache: usize,
    /// literals in a row before the search skips ahead
    spree: usize,
    /// literal context modeling (else one literal tree)
    ctx: bool,
    /// try both context modes (else the UTF-8 one for text, the signed one for the rest)
    both: bool,
    /// words of the static dictionary
    dict: bool,
    /// the input of a meta-block (log2)
    pub(super) block_log: u32,
    /// the window (log2) without a size given
    pub(super) lgwin: u32,
    /// rounds of the optimal parse (0: lazy matching)
    opt: u32,
    /// matches longer than this are taken as they are (the optimal parse)
    nice: usize,
}

pub(super) fn params(level: i32) -> Params {
    let l = level.clamp(0, 11);
    let p = |depth, hash_log, chain_log, lazy, cache, spree, ctx, dict, block_log, lgwin| Params {
        opt: 0,
        nice: 0,
        both: l >= 9,
        depth,
        hash_log,
        chain_log,
        lazy,
        cache,
        spree,
        ctx,
        dict,
        block_log,
        lgwin,
    };
    match l {
        0 => p(1, 14, 0, false, 1, 64, false, false, 16, 22),
        1 => p(1, 16, 0, false, 4, 64, false, false, 16, 22),
        2 => p(4, 14, 0, false, 4, 64, false, true, 16, 22),
        3 => p(8, 14, 0, true, 4, 64, false, true, 16, 22),
        4 => p(16, 14, 0, true, 4, 64, true, true, 17, 22),
        5 => p(16, 15, 0, true, 4, 64, true, true, 18, 22),
        6 => p(32, 15, 0, true, 4, 64, true, true, 18, 22),
        7 => p(64, 15, 0, true, 10, 64, true, true, 18, 22),
        8 => p(128, 15, 0, true, 10, 64, true, true, 18, 22),
        9 => p(256, 15, 0, true, 16, 512, true, true, 20, 22),
        10 => Params { opt: 1, nice: 150, ..p(128, 20, 22, true, 16, 512, true, true, 20, 24) },
        _ => Params { opt: 2, nice: 325, ..p(512, 20, 24, true, 16, 512, true, true, 20, 24) },
    }
}

// ---------------------------------------------------------------- bits

/// LSB-first bit writer
pub(super) struct Bw {
    pub(super) out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bw {
    pub(super) fn new() -> Bw {
        Bw { out: Vec::new(), acc: 0, n: 0 }
    }

    /// the low n bits of v (n <= 32)
    #[inline(always)]
    pub(super) fn put(&mut self, v: u32, n: u32) {
        self.acc |= (v as u64) << self.n;
        self.n += n;
        if self.n >= 32 {
            self.out.extend_from_slice(&(self.acc as u32).to_le_bytes());
            self.acc >>= 32;
            self.n -= 32;
        }
    }

    /// to the byte boundary (zeros)
    pub(super) fn align(&mut self) {
        while self.n > 0 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n = self.n.saturating_sub(8);
        }
        self.acc = 0;
    }

    pub(super) fn bits(&self) -> usize {
        self.out.len() * 8 + self.n as usize
    }

    /// bits that do not fill a byte of out yet
    pub(super) fn pending(&self) -> bool {
        self.n > 0
    }

    fn mark(&self) -> (usize, u64, u32) {
        (self.out.len(), self.acc, self.n)
    }

    fn reset(&mut self, m: (usize, u64, u32)) {
        self.out.truncate(m.0);
        self.acc = m.1;
        self.n = m.2;
    }
}

fn varlen8(bw: &mut Bw, v: u32) {
    if v == 0 {
        bw.put(0, 1);
    } else {
        let n = 31 - v.leading_zeros();
        bw.put(1, 1);
        bw.put(n, 3);
        bw.put(v - (1 << n), n);
    }
}

/// the stream header: the window bits
pub(super) fn wbits(bw: &mut Bw, lgwin: u32) {
    match lgwin {
        16 => bw.put(0, 1),
        17 => bw.put(1, 7),
        18..=24 => bw.put(((lgwin - 17) << 1) | 1, 4),
        _ => bw.put(((lgwin - 8) << 4) | 1, 7),
    }
}

/// the end of the stream: an empty last meta-block (ISLAST, ISLASTEMPTY), then the byte boundary
pub(super) fn end(bw: &mut Bw) {
    bw.put(3, 2);
    bw.align();
}

/// an empty metadata meta-block: what the stream has so far fills whole bytes (Flush::Sync)
pub(super) fn sync(bw: &mut Bw) {
    bw.put(6, 6); // ISLAST 0, MNIBBLES 0 (11), reserved 0, MSKIPBYTES 0
    bw.align();
}

/// ISLAST (with ISLASTEMPTY 0), MNIBBLES and MLEN - 1
fn mlen_header(bw: &mut Bw, last: bool, mlen: usize) -> u32 {
    let m = (mlen - 1) as u32;
    let nib = if m < 1 << 16 {
        4
    } else if m < 1 << 20 {
        5
    } else {
        6
    };
    bw.put(last as u32, 1);
    if last {
        bw.put(0, 1);
    }
    bw.put(nib - 4, 2);
    bw.put(m, 4 * nib);
    nib
}

/// a meta-block of these bytes as they are (never the last one: ISLAST has no uncompressed form)
fn stored(bw: &mut Bw, data: &[u8]) {
    mlen_header(bw, false, data.len());
    bw.put(1, 1); // ISUNCOMPRESSED
    bw.align();
    bw.out.extend_from_slice(data);
}

// ---------------------------------------------------------------- costs

/// log2(1 + i / 256) in 1/4096 bits, by repeated squaring
const fn log2_table() -> [u16; 256] {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut x: u64 = (256 + i as u64) << 22; // 1 + i/256 with 30 fractional bits
        let mut r = 0u32;
        let mut k = 0;
        while k < 12 {
            x = (x * x) >> 30;
            r <<= 1;
            if x >= 2 << 30 {
                x >>= 1;
                r |= 1;
            }
            k += 1;
        }
        t[i] = r as u16;
        i += 1;
    }
    t
}

static LOG2: [u16; 256] = log2_table();

/// the cost unit: 1/4096 of a bit
const Q: u64 = 4096;

/// log2(v) in 1/4096 bits (0 for 0)
#[inline]
fn lg(v: u32) -> u64 {
    if v == 0 {
        return 0;
    }
    let hb = 31 - v.leading_zeros();
    let m = if hb >= 8 { (v >> (hb - 8)) & 0xFF } else { (v << (8 - hb)) & 0xFF };
    ((hb as u64) << 12) + LOG2[m as usize] as u64
}

/// Shannon bits of a histogram, at least one per symbol (libbrotli's BitsEntropy)
fn bits_entropy(h: &[u32]) -> u64 {
    let (mut sum, mut acc) = (0u64, 0u64);
    for &c in h {
        if c > 0 {
            sum += c as u64;
            acc += c as u64 * lg(c);
        }
    }
    if sum == 0 {
        return 0;
    }
    (sum * lg(sum as u32)).saturating_sub(acc).max(sum * Q)
}

/// the used symbols of a histogram and their counts
fn sparse(h: &[u32]) -> Vec<(u16, u32)> {
    h.iter().enumerate().filter(|&(_, &c)| c > 0).map(|(i, &c)| (i as u16, c)).collect()
}

/// estimated bits of coding a histogram (its used symbols in order) with a prefix code of
/// its own, the code included (as libbrotli's BrotliPopulationCost)
fn sparse_cost(v: &[(u16, u32)]) -> u64 {
    let total: u64 = v.iter().map(|&(_, c)| c as u64).sum();
    match v.len() {
        0 | 1 => return 12 * Q,
        2 => return (20 + total) * Q,
        3 => {
            let (a, b, c) = (v[0].1 as u64, v[1].1 as u64, v[2].1 as u64);
            return (28 + 2 * (a + b + c) - a.max(b).max(c)) * Q;
        }
        4 => {
            let mut h = [v[0].1 as u64, v[1].1 as u64, v[2].1 as u64, v[3].1 as u64];
            h.sort_unstable_by(|a, b| b.cmp(a));
            let h23 = h[2] + h[3];
            return (37 + 3 * h23 + 2 * (h[0] + h[1]) - h23.max(h[0])) * Q;
        }
        _ => {}
    }
    let lt = lg(total as u32);
    let mut bits = 0u64;
    let mut max_depth = 1u64;
    let mut depths = [0u32; 18];
    let mut next = 0usize;
    for &(sym, c) in v {
        // the zeros before the symbol (those after the last one cost nothing)
        let reps = sym as usize - next;
        if reps < 3 {
            depths[0] += reps as u32;
        } else {
            let mut r = reps - 2;
            while r > 0 {
                depths[17] += 1;
                bits += 3 * Q;
                r >>= 3;
            }
        }
        let lp = lt.saturating_sub(lg(c));
        let d = ((lp + Q / 2) / Q).min(15);
        bits += c as u64 * lp;
        max_depth = max_depth.max(d);
        depths[d as usize] += 1;
        next = sym as usize + 1;
    }
    bits + (18 + 2 * max_depth) * Q + bits_entropy(&depths)
}

/// the sum of two sparse histograms
fn sparse_sum(a: &[(u16, u32)], b: &[(u16, u32)], out: &mut Vec<(u16, u32)>) {
    out.clear();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].0 < b[j].0 {
            out.push(a[i]);
            i += 1;
        } else if b[j].0 < a[i].0 {
            out.push(b[j]);
            j += 1;
        } else {
            out.push((a[i].0, a[i].1 + b[j].1));
            i += 1;
            j += 1;
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
}

/// histograms of width `w` merged greedily while that saves bits (and down to `max`): the
/// cluster of each histogram (empty ones take the one before), the clusters, their cost.  A
/// merge also saves half the entropy of telling the two apart in the context map (as
/// libbrotli's HistogramCombine)
fn cluster(h: &[u32], w: usize, max: usize) -> (Vec<u8>, Vec<Vec<u32>>, u64) {
    let n = h.len() / w;
    let mut cl: Vec<Vec<u32>> = Vec::new();
    let mut sp: Vec<Vec<(u16, u32)>> = Vec::new();
    let mut of: Vec<usize> = vec![usize::MAX; n];
    let mut cost: Vec<u64> = Vec::new();
    for i in 0..n {
        let hi = &h[i * w..(i + 1) * w];
        let v = sparse(hi);
        if !v.is_empty() {
            of[i] = cl.len();
            cost.push(sparse_cost(&v));
            cl.push(hi.to_vec());
            sp.push(v);
        }
    }
    if cl.is_empty() {
        return (vec![0; n], vec![vec![0; w]], 12 * Q);
    }
    let k = cl.len();
    let mut tmp = Vec::new();
    let mut diff = vec![i64::MAX; k * k];
    let nlg = |n: u32| n as i64 * lg(n) as i64;
    let map_diff = |a: u32, b: u32| (nlg(a) + nlg(b) - nlg(a + b)) / 2;
    let mut size = vec![1u32; k];
    for a in 0..k {
        for b in a + 1..k {
            sparse_sum(&sp[a], &sp[b], &mut tmp);
            diff[a * k + b] = sparse_cost(&tmp) as i64 - cost[a] as i64 - cost[b] as i64 + map_diff(1, 1);
        }
    }
    let mut alive = vec![true; k];
    let mut into: Vec<usize> = (0..k).collect();
    let mut live = k;
    loop {
        let mut best = (i64::MAX, 0, 0);
        for a in 0..k {
            if !alive[a] {
                continue;
            }
            for b in a + 1..k {
                if alive[b] && diff[a * k + b] < best.0 {
                    best = (diff[a * k + b], a, b);
                }
            }
        }
        if best.0 == i64::MAX || (best.0 >= 0 && live <= max) {
            break;
        }
        let (_, a, b) = best;
        for j in 0..w {
            let v = cl[b][j];
            cl[a][j] += v;
        }
        sparse_sum(&sp[a], &sp[b], &mut tmp);
        core::mem::swap(&mut sp[a], &mut tmp);
        cost[a] = sparse_cost(&sp[a]);
        size[a] += size[b];
        alive[b] = false;
        into[b] = a;
        live -= 1;
        for x in 0..k {
            if alive[x] && x != a {
                let (lo, hi) = if x < a { (x, a) } else { (a, x) };
                sparse_sum(&sp[a], &sp[x], &mut tmp);
                diff[lo * k + hi] = sparse_cost(&tmp) as i64 - cost[a] as i64 - cost[x] as i64 + map_diff(size[a], size[x]);
            }
        }
    }
    // number the clusters in the order they first appear in the map
    let root = |mut c: usize| {
        while into[c] != c {
            c = into[c];
        }
        c
    };
    let mut id = vec![usize::MAX; k];
    let (mut hs, mut total) = (Vec::new(), 0u64);
    let mut map = vec![0u8; n];
    let mut prev = 0u8;
    for i in 0..n {
        if of[i] != usize::MAX {
            let r = root(of[i]);
            if id[r] == usize::MAX {
                id[r] = hs.len();
                total += cost[r];
                hs.push(core::mem::take(&mut cl[r]));
            }
            prev = id[r] as u8;
        }
        map[i] = prev;
    }
    (map, hs, total)
}

// ---------------------------------------------------------------- prefix codes

/// Huffman code lengths of at most `limit` bits; with fewer than two symbols used all stay 0
fn lengths(h: &[u32], limit: usize, lens: &mut [u8]) {
    lens.fill(0);
    let mut syms: Vec<u16> = (0..h.len()).filter(|&i| h[i] > 0).map(|i| i as u16).collect();
    let m = syms.len();
    if m < 2 {
        return;
    }
    syms.sort_unstable_by_key(|&s| (h[s as usize], s));
    let mut wgt = vec![0u32; 2 * m];
    let mut par = vec![0u32; 2 * m];
    for i in 0..m {
        wgt[i] = h[syms[i] as usize];
    }
    // two queues: the leaves by weight, the inner nodes as they are made
    let (mut c, mut d, mut e) = (0usize, m, m);
    while e < 2 * m - 1 {
        let mut pick = || {
            if c < m && (d >= e || wgt[c] <= wgt[d]) {
                c += 1;
                c - 1
            } else {
                d += 1;
                d - 1
            }
        };
        let a = pick();
        let b = pick();
        wgt[e] = wgt[a].saturating_add(wgt[b]);
        par[a] = e as u32;
        par[b] = e as u32;
        e += 1;
    }
    let root = 2 * m - 2;
    let mut dep = vec![0u32; 2 * m];
    for i in (0..root).rev() {
        dep[i] = dep[par[i] as usize] + 1;
    }
    let mut bl = [0u32; 16];
    for &x in dep.iter().take(m) {
        bl[(x as usize).min(limit)] += 1;
    }
    // over the limit: repair the Kraft sum
    let mut total = 0u32;
    for (l, &b) in bl.iter().enumerate().take(limit + 1).skip(1) {
        total += b << (limit - l);
    }
    while total > 1 << limit {
        bl[limit] -= 1;
        let mut l = limit - 1;
        while l != 0 {
            if bl[l] != 0 {
                bl[l] -= 1;
                bl[l + 1] += 2;
                break;
            }
            l -= 1;
        }
        total -= 1;
    }
    let mut idx = 0;
    for l in (1..=limit).rev() {
        for _ in 0..bl[l] {
            lens[syms[idx] as usize] = l as u8;
            idx += 1;
        }
    }
}

/// the canonical codes of these lengths, bit-reversed for the LSB-first writer
fn codes(lens: &[u8], codes: &mut [u16]) {
    let mut cnt = [0u16; 16];
    for &l in lens {
        cnt[l as usize] += 1;
    }
    cnt[0] = 0;
    let mut next = [0u16; 16];
    let mut c = 0u16;
    for l in 1..16 {
        c = (c + cnt[l - 1]) << 1;
        next[l] = c;
    }
    for (s, &l) in lens.iter().enumerate() {
        if l != 0 {
            let v = next[l as usize];
            next[l as usize] += 1;
            codes[s] = v.reverse_bits() >> (16 - l);
        } else {
            codes[s] = 0;
        }
    }
}

/// a run of a length after `prev` as code length symbols (16: repeat the last length)
fn reps_value(prev: u8, v: u8, mut reps: usize, toks: &mut Vec<(u8, u8)>) {
    if prev != v {
        toks.push((v, 0));
        reps -= 1;
    }
    if reps == 7 {
        toks.push((v, 0));
        reps -= 1;
    }
    if reps < 3 {
        for _ in 0..reps {
            toks.push((v, 0));
        }
    } else {
        let start = toks.len();
        reps -= 3;
        loop {
            toks.push((16, (reps & 3) as u8));
            reps >>= 2;
            if reps == 0 {
                break;
            }
            reps -= 1;
        }
        toks[start..].reverse();
    }
}

/// a run of zero lengths (17: repeat zero)
fn reps_zeros(mut reps: usize, toks: &mut Vec<(u8, u8)>) {
    if reps == 11 {
        toks.push((0, 0));
        reps -= 1;
    }
    if reps < 3 {
        for _ in 0..reps {
            toks.push((0, 0));
        }
    } else {
        let start = toks.len();
        reps -= 3;
        loop {
            toks.push((17, (reps & 7) as u8));
            reps >>= 3;
            if reps == 0 {
                break;
            }
            reps -= 1;
        }
        toks[start..].reverse();
    }
}

/// the code lengths as code length symbols with their extra bits (libbrotli's
/// BrotliWriteHuffmanTree: runs only for alphabets over 50 symbols and when they pay)
fn tree_tokens(lens: &[u8], toks: &mut Vec<(u8, u8)>) {
    let mut n = lens.len();
    while n > 0 && lens[n - 1] == 0 {
        n -= 1;
    }
    let (mut rle_nz, mut rle_z) = (false, false);
    if lens.len() > 50 {
        let (mut tz, mut tnz, mut cz, mut cnz) = (0usize, 0usize, 1usize, 1usize);
        let mut i = 0;
        while i < n {
            let v = lens[i];
            let mut reps = 1;
            while i + reps < n && lens[i + reps] == v {
                reps += 1;
            }
            if reps >= 3 && v == 0 {
                tz += reps;
                cz += 1;
            }
            if reps >= 4 && v != 0 {
                tnz += reps;
                cnz += 1;
            }
            i += reps;
        }
        rle_nz = tnz > cnz * 2;
        rle_z = tz > cz * 2;
    }
    let mut prev = 8u8;
    let mut i = 0;
    while i < n {
        let v = lens[i];
        let mut reps = 1;
        if (v != 0 && rle_nz) || (v == 0 && rle_z) {
            while i + reps < n && lens[i + reps] == v {
                reps += 1;
            }
        }
        if v == 0 {
            reps_zeros(reps, toks);
        } else {
            reps_value(prev, v, reps, toks);
            prev = v;
        }
        i += reps;
    }
}

/// the order of the code length code lengths
const CL_ORDER: [usize; 18] = [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// the fixed code of a code length code length: (bits, length)
const CL_FIXED: [(u32, u32); 6] = [(0, 2), (7, 4), (3, 3), (2, 2), (1, 2), (15, 4)];

/// build the prefix code of `h` (an alphabet of h.len() symbols) and write it: its lengths
/// and codes for the data
fn store_code(bw: &mut Bw, h: &[u32], lens: &mut [u8], cs: &mut [u16]) {
    let size = h.len();
    let abits = 32 - ((size - 1) as u32).leading_zeros();
    let mut s4 = [0usize; 4];
    let mut m = 0;
    for (i, &c) in h.iter().enumerate() {
        if c > 0 {
            if m < 4 {
                s4[m] = i;
            }
            m += 1;
        }
    }
    if m <= 1 {
        // one symbol (or none: any symbol), no bits per symbol
        lens.fill(0);
        cs.fill(0);
        bw.put(1, 2);
        bw.put(0, 2);
        bw.put(s4[0] as u32, abits);
        return;
    }
    lengths(h, 15, lens);
    codes(lens, cs);
    if m <= 4 {
        let ss = &mut s4[..m];
        ss.sort_unstable_by_key(|&s| (lens[s], s));
        bw.put(1, 2);
        bw.put(m as u32 - 1, 2);
        for &s in ss.iter() {
            bw.put(s as u32, abits);
        }
        if m == 4 {
            bw.put((lens[ss[0]] == 1) as u32, 1);
        }
        return;
    }
    let mut toks = Vec::new();
    tree_tokens(lens, &mut toks);
    let mut clh = [0u32; 18];
    for &(t, _) in &toks {
        clh[t as usize] += 1;
    }
    let mut cll = [0u8; 18];
    let mut clc = [0u16; 18];
    lengths(&clh, 5, &mut cll);
    let nc = clh.iter().filter(|&&c| c > 0).count();
    if nc == 1 {
        // one code length symbol: its code has no bits
        cll[clh.iter().position(|&c| c > 0).unwrap_or(0)] = 1;
    } else {
        codes(&cll, &mut clc);
    }
    let mut stored = 18;
    if nc > 1 {
        while stored > 0 && cll[CL_ORDER[stored - 1]] == 0 {
            stored -= 1;
        }
    }
    let skip = if cll[CL_ORDER[0]] == 0 && cll[CL_ORDER[1]] == 0 {
        if cll[CL_ORDER[2]] == 0 { 3 } else { 2 }
    } else {
        0
    };
    bw.put(skip as u32, 2);
    for &o in &CL_ORDER[skip..stored] {
        let (v, n) = CL_FIXED[cll[o] as usize];
        bw.put(v, n);
    }
    for &(t, x) in &toks {
        if nc > 1 {
            bw.put(clc[t as usize] as u32, cll[t as usize] as u32);
        }
        match t {
            16 => bw.put(x as u32, 2),
            17 => bw.put(x as u32, 3),
            _ => {}
        }
    }
}

/// a context map over `ntrees` trees: NTREES, then (with two or more) the map with runs of
/// zeros after move-to-front
fn store_cmap(bw: &mut Bw, map: &[u8], ntrees: usize) {
    varlen8(bw, ntrees as u32 - 1);
    if ntrees < 2 {
        return;
    }
    let mut mtf: Vec<u8> = (0..=255u8).collect();
    let v: Vec<u32> = map
        .iter()
        .map(|&x| {
            let i = mtf.iter().position(|&y| y == x).unwrap_or(0);
            mtf.copy_within(0..i, 1);
            mtf[0] = x;
            i as u32
        })
        .collect();
    let mut longest = 0usize;
    let mut i = 0;
    while i < v.len() {
        let mut reps = 0;
        while i < v.len() && v[i] == 0 {
            reps += 1;
            i += 1;
        }
        longest = longest.max(reps);
        i += 1;
    }
    let rmax = if longest > 0 { (31 - (longest as u32).leading_zeros()).min(6) } else { 0 };
    let mut syms: Vec<(u32, u32)> = Vec::new();
    let mut i = 0;
    while i < v.len() {
        if v[i] != 0 {
            syms.push((v[i] + rmax, 0));
            i += 1;
            continue;
        }
        let mut reps = 1u32;
        while i + (reps as usize) < v.len() && v[i + reps as usize] == 0 {
            reps += 1;
        }
        i += reps as usize;
        while reps != 0 {
            if reps < 2 << rmax {
                let p = 31 - reps.leading_zeros();
                syms.push((p, reps - (1 << p)));
                break;
            }
            syms.push((rmax, (1 << rmax) - 1));
            reps -= (2 << rmax) - 1;
        }
    }
    let mut h = vec![0u32; ntrees + rmax as usize];
    for &(s, _) in &syms {
        h[s as usize] += 1;
    }
    bw.put((rmax > 0) as u32, 1);
    if rmax > 0 {
        bw.put(rmax - 1, 4);
    }
    let mut lens = vec![0u8; h.len()];
    let mut cs = vec![0u16; h.len()];
    store_code(bw, &h, &mut lens, &mut cs);
    for &(s, x) in &syms {
        bw.put(cs[s as usize] as u32, lens[s as usize] as u32);
        if s > 0 && s <= rmax {
            bw.put(x, s);
        }
    }
    bw.put(1, 1); // inverse move-to-front
}

// ---------------------------------------------------------------- commands

/// an insert-and-copy command
#[derive(Clone, Copy)]
pub(super) struct Cmd {
    ins: u32,
    /// the copy length of the command (a dictionary word: its length); 0: none (the literals
    /// at the end of a meta-block)
    copy: u32,
    /// the bytes the copy makes (a transformed dictionary word: other than copy)
    out: u32,
    /// the distance code: 0..=15 the short codes, else the distance + 15
    dcode: u32,
}

#[inline]
fn floor_log2(v: u32) -> u32 {
    31 - v.leading_zeros()
}

fn ins_code(n: u32) -> u32 {
    if n < 6 {
        n
    } else if n < 130 {
        let nb = floor_log2(n - 2) - 1;
        (nb << 1) + ((n - 2) >> nb) + 2
    } else if n < 2114 {
        floor_log2(n - 66) + 10
    } else if n < 6210 {
        21
    } else if n < 22594 {
        22
    } else {
        23
    }
}

fn copy_code(n: u32) -> u32 {
    if n < 10 {
        n - 2
    } else if n < 134 {
        let nb = floor_log2(n - 6) - 1;
        (nb << 1) + ((n - 6) >> nb) + 4
    } else if n < 2118 {
        floor_log2(n - 70) + 12
    } else {
        23
    }
}

/// the command symbol of an insert code, a copy code and whether the distance is the last
/// one (cells 0 and 1 then leave the distance out)
fn cmd_symbol(ic: u32, cc: u32, last: bool) -> u32 {
    let low = (cc & 7) | ((ic & 7) << 3);
    if last && ic < 8 && cc < 16 {
        if cc < 8 { low } else { low | 64 }
    } else {
        const CELL: [[u32; 3]; 3] = [[2, 3, 6], [4, 5, 8], [7, 9, 10]];
        (CELL[(ic >> 3) as usize][(cc >> 3) as usize] << 6) | low
    }
}

/// the symbol, extra bits and their number of a distance code (NPOSTFIX 0, NDIRECT 0)
fn dist_symbol(dcode: u32) -> (u32, u32, u32) {
    if dcode < 16 {
        return (dcode, 0, 0);
    }
    let dist = 4 + (dcode - 16);
    let nb = floor_log2(dist) - 1;
    let prefix = (dist >> nb) & 1;
    (16 + 2 * (nb - 1) + prefix, dist - ((2 + prefix) << nb), nb)
}

/// the distance code of a distance (libbrotli's ComputeDistanceCode): the short codes for the
/// last two distances and their neighbours and the two before them
fn dist_code(d: u32, cache: &[u32; 4]) -> u32 {
    if d == cache[0] {
        return 0;
    }
    if d == cache[1] {
        return 1;
    }
    let off0 = (d + 3).wrapping_sub(cache[0]);
    if off0 < 7 {
        return (0x0975_0468 >> (4 * off0)) & 0xF;
    }
    let off1 = (d + 3).wrapping_sub(cache[1]);
    if off1 < 7 {
        return (0x0FDB_1ACE >> (4 * off1)) & 0xF;
    }
    if d == cache[2] {
        return 2;
    }
    if d == cache[3] {
        return 3;
    }
    d + 15
}

// ---------------------------------------------------------------- meta-block statistics

/// the distance alphabet (NPOSTFIX 0, NDIRECT 0)
const DSIZE: usize = 64;

/// the distance context of a copy length
#[inline]
fn dctx(copy: u32) -> usize {
    if copy > 4 { 3 } else { copy as usize - 2 }
}

/// the literal context of position i in a context mode's table
#[inline]
fn context(s: &[u8], lut: &[u8], i: usize) -> usize {
    let p1 = if i >= 1 { s[i - 1] } else { 0 } as usize;
    let p2 = if i >= 2 { s[i - 2] } else { 0 } as usize;
    (lut[p1] | lut[256 + p2]) as usize
}

/// distance j of the cache: the four last ones, then the last and the second last +-1..3
#[inline]
fn cached(c: &[u32; 4], j: usize) -> u32 {
    const BASE: [u8; 16] = [0, 1, 2, 3, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1];
    const OFF: [i8; 16] = [0, 0, 0, 0, -1, 1, -2, 2, -3, 3, -1, 1, -2, 2, -3, 3];
    (c[BASE[j] as usize] as i64 + OFF[j] as i64).max(0) as u32
}

/// the symbols of a meta-block's commands and their histograms
struct Hists {
    /// command and distance symbol of each command (u16::MAX: no distance)
    syms: Vec<(u16, u16)>,
    cmd: Vec<u32>,
    /// 4 contexts of DSIZE
    dist: Vec<u32>,
    /// the literals per context mode: 0 (one context), 2 (UTF-8) or 3 (signed: 64 contexts)
    lits: Vec<(u32, Vec<u32>)>,
}

impl Hists {
    fn new(cmds: usize, modes: &[u32]) -> Hists {
        Hists {
            syms: Vec::with_capacity(cmds),
            cmd: vec![0; 704],
            dist: vec![0; 4 * DSIZE],
            lits: modes.iter().map(|&m| (m, vec![0; if m == 0 { 256 } else { 64 * 256 }])).collect(),
        }
    }

    /// the literals s[from..to]
    fn literals(&mut self, s: &[u8], from: usize, to: usize) {
        for (m, h) in self.lits.iter_mut() {
            if *m == 0 {
                for &b in &s[from..to] {
                    h[b as usize] += 1;
                }
            } else {
                let lut = &T::CONTEXT_LUT[(*m as usize) << 9..(*m as usize + 1) << 9];
                for i in from..to {
                    h[context(s, lut, i) * 256 + s[i] as usize] += 1;
                }
            }
        }
    }
}

/// mostly text: three quarters of the bytes or more in UTF-8 characters other than controls
fn texty(d: &[u8]) -> bool {
    let (mut ok, mut i) = (0, 0);
    while i < d.len() {
        let b = d[i];
        let n = if b < 0x80 {
            (b >= 0x20 || b == b'\t' || b == b'\n' || b == b'\r') as usize
        } else if b & 0xE0 == 0xC0 {
            2
        } else if b & 0xF0 == 0xE0 {
            3
        } else if b & 0xF8 == 0xF0 {
            4
        } else {
            0
        };
        if n > 0 && i + n <= d.len() && d[i + 1..i + n].iter().all(|&c| c & 0xC0 == 0x80) {
            ok += n;
            i += n;
        } else {
            i += 1;
        }
    }
    ok * 4 >= d.len() * 3
}

/// the bits of the prefix code of `h` and of the symbols it codes
fn code_bits(h: &[u32]) -> u64 {
    let mut bw = Bw::new();
    let mut lens = vec![0u8; h.len()];
    let mut cs = vec![0u16; h.len()];
    store_code(&mut bw, h, &mut lens, &mut cs);
    bw.bits() as u64 + h.iter().zip(&lens).map(|(&c, &l)| c as u64 * l as u64).sum::<u64>()
}

/// the literal trees: one, or the contexts of a mode merged into clusters, whichever takes
/// the fewest bits -> (mode, context map, the trees' histograms)
fn literal_trees(lits: &[(u32, Vec<u32>)]) -> (u32, Vec<u8>, Vec<Vec<u32>>) {
    let mut one = vec![0u32; 256];
    for (i, &c) in lits[0].1.iter().enumerate() {
        one[i & 255] += c;
    }
    if lits.len() == 1 && lits[0].0 == 0 {
        return (0, vec![0; 64], vec![one]);
    }
    let mut best = (code_bits(&one), 0u32, vec![0u8; 64], vec![one]);
    for (mode, h) in lits {
        let (map, hs, _) = cluster(h, 256, 256);
        if hs.len() < 2 {
            continue;
        }
        let mut bw = Bw::new();
        store_cmap(&mut bw, &map, hs.len());
        let bits = bw.bits() as u64 + hs.iter().map(|h| code_bits(h)).sum::<u64>();
        if bits < best.0 {
            best = (bits, *mode, map, hs);
        }
    }
    (best.1, best.2, best.3)
}

// ---------------------------------------------------------------- the optimal parse's parts

/// a static dictionary word in Mt::dist and Node::dist
const DICT_FLAG: u32 = 1 << 31;

/// a match of a position: `out` bytes, the command's copy length (a dictionary word's
/// length) and the distance
#[derive(Clone, Copy)]
struct Mt {
    out: u32,
    copy: u32,
    dist: u32,
}

/// the cheapest way to a position: its cost, the last step (out 0: a literal), the literals
/// at the end of the way and the distance cache after it
#[derive(Clone, Copy)]
struct Node {
    cost: u64,
    out: u32,
    copy: u32,
    dist: u32,
    lits: u32,
    cache: [u32; 4],
}

/// the costs (1/4096 bits) the optimal parse works with: each position's literal, each
/// command symbol, each distance symbol in its context
struct Model {
    lit: Vec<u32>,
    cmd: Vec<u32>,
    dist: Vec<u32>,
}

/// -log2 of c in total, a symbol not seen 2 bits more than one seen once
fn cost_of(c: u32, total: u32, size: u32) -> u32 {
    if total == 0 {
        lg(size) as u32
    } else if c == 0 {
        (lg(total) + 2 * Q) as u32
    } else {
        (lg(total) - lg(c)) as u32
    }
}

impl Model {
    /// the literals' costs from these histograms
    fn literal_costs(lits: &[(u32, Vec<u32>)], s: &[u8], bs: usize, be: usize) -> Vec<u32> {
        let (mode, map, hs) = literal_trees(lits);
        let totals: Vec<u32> = hs.iter().map(|h| h.iter().sum()).collect();
        let lut = &T::CONTEXT_LUT[(mode as usize) << 9..(mode as usize + 1) << 9];
        (bs..be)
            .map(|i| {
                let t = if hs.len() > 1 { map[context(s, lut, i)] as usize } else { 0 };
                cost_of(hs[t][s[i] as usize], totals[t], 256)
            })
            .collect()
    }

    /// before any commands: the literals as all bytes of the block, the commands and
    /// distances cheaper the smaller their symbol (as libbrotli's first Zopfli round)
    fn estimated(s: &[u8], bs: usize, be: usize) -> Model {
        let mut h = Hists::new(0, &[0]);
        h.literals(s, bs, be);
        Model {
            lit: Model::literal_costs(&h.lits, s, bs, be),
            cmd: (0..704).map(|i| lg(11 + i) as u32).collect(),
            dist: (0..4 * DSIZE as u32).map(|i| lg(20 + i % DSIZE as u32) as u32).collect(),
        }
    }

    /// the costs of these commands' symbols
    fn of(h: &Hists, s: &[u8], bs: usize, be: usize) -> Model {
        let ct: u32 = h.cmd.iter().sum();
        let all: Vec<u32> = (0..DSIZE).map(|i| (0..4).map(|c| h.dist[c * DSIZE + i]).sum()).collect();
        let at: u32 = all.iter().sum();
        let mut dist = vec![0u32; 4 * DSIZE];
        for c in 0..4 {
            let t: u32 = h.dist[c * DSIZE..(c + 1) * DSIZE].iter().sum();
            for i in 0..DSIZE {
                dist[c * DSIZE + i] = if t > 0 { cost_of(h.dist[c * DSIZE + i], t, 64) } else { cost_of(all[i], at, 64) };
            }
        }
        Model {
            lit: Model::literal_costs(&h.lits, s, bs, be),
            cmd: h.cmd.iter().map(|&c| cost_of(c, ct, 704)).collect(),
            dist,
        }
    }
}

// ---------------------------------------------------------------- the static dictionary

const NWORDS: usize = 13504;
const DH_LOG: u32 = 15;

/// the words of the dictionary by their first four bytes
struct DictIndex {
    head: [u16; 1 << DH_LOG],
    next: [u16; NWORDS],
    len: [u8; NWORDS],
    base: [u16; 25],
}

const fn dict_key(b: &[u8], i: usize) -> usize {
    let v = u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    (v.wrapping_mul(0x1E35_A7BD) >> (32 - DH_LOG)) as usize
}

const fn dict_index() -> DictIndex {
    let mut x = DictIndex { head: [0; 1 << DH_LOG], next: [0; NWORDS], len: [0; NWORDS], base: [0; 25] };
    let mut w = 0usize;
    let mut l = 4;
    while l <= 24 {
        x.base[l] = w as u16;
        let n = 1usize << T::SIZE_BITS[l];
        let mut i = n;
        // the last words first: the chain then lists the first ones first
        while i > 0 {
            i -= 1;
            let k = dict_key(DICT, T::OFFSETS[l] as usize + i * l);
            let id = w + i;
            x.next[id] = x.head[k];
            x.head[k] = id as u16 + 1;
            x.len[id] = l as u8;
        }
        w += n;
        l += 1;
    }
    assert!(w == NWORDS);
    x
}

static DIDX: DictIndex = dict_index();

/// the transforms looked for, by (a space before the word, its first letter in upper case):
/// (transform, letters cut from the end, suffix id)
struct Fams {
    t: [[(u8, u8, u8); 64]; 4],
    n: [usize; 4],
}

const fn fams() -> Fams {
    let mut f = Fams { t: [[(0, 0, 0); 64]; 4], n: [0; 4] };
    let mut t = 0;
    while t < 121 {
        let [pre, ty, suf] = T::TRANSFORMS[t];
        if (pre == 49 || pre == 0) && ty <= 10 {
            let k = (pre == 0) as usize * 2 + (ty == 10) as usize;
            let cut = if ty <= 9 { ty } else { 0 };
            f.t[k][f.n[k]] = (t as u8, cut, suf);
            f.n[k] += 1;
        }
        t += 1;
    }
    f
}

static FAMS: Fams = fams();

fn affix(id: u8) -> &'static [u8] {
    let s = T::PREFIX_SUFFIX_MAP[id as usize] as usize;
    let n = T::PREFIX_SUFFIX[s] as usize;
    &T::PREFIX_SUFFIX[s + 1..s + 1 + n]
}

// ---------------------------------------------------------------- matching

#[inline(always)]
fn rd32(b: &[u8], i: usize) -> u32 {
    let mut a = [0u8; 4];
    a.copy_from_slice(&b[i..i + 4]);
    u32::from_le_bytes(a)
}

#[inline(always)]
fn rd64(b: &[u8], i: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[i..i + 8]);
    u64::from_le_bytes(a)
}

/// equal bytes at a and b, at most max
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

const SCORE_BASE: i64 = 1920;
const MIN_SCORE: i64 = SCORE_BASE + 100;

#[inline]
fn score(len: usize, dist: usize) -> i64 {
    SCORE_BASE + 135 * len as i64 - 30 * floor_log2(dist as u32) as i64
}

#[derive(Clone, Copy)]
struct Cand {
    copy: usize,
    out: usize,
    dist: usize,
    score: i64,
}

const NONE: Cand = Cand { copy: 0, out: 0, dist: 0, score: MIN_SCORE };

/// the state of a stream's compression from one meta-block to the next
pub(super) struct Core {
    pub(super) prm: Params,
    pub(super) lgwin: u32,
    pub(super) window: usize,
    /// per hash the last position (+ 1), or a bucket of the last ones, or the head of a chain
    hash: Vec<u32>,
    chain: Vec<u32>,
    hash_log: u32,
    /// the bucket size (log2; 0: one position or chains) and how many entered each bucket
    blog: u32,
    num: Vec<u16>,
    pub(super) cmask: usize,
    /// the next position to enter into the tables
    next: usize,
    /// the last four distances, the last first
    cache: [u32; 4],
    lookups: u32,
    hits: u32,
    cmds: Vec<Cmd>,
}

impl Core {
    /// the compression at `level` of `size` bytes if that is known; `lgwin` a window
    /// other than the level's
    pub(super) fn new(level: i32, size: Option<u64>, lgwin: Option<u32>) -> Core {
        let prm = params(level);
        let mut lg = lgwin.unwrap_or(prm.lgwin).clamp(10, 24);
        if let Some(n) = size {
            // no larger than the data needs, but 16 rather than less: its header is 1 bit
            let mut need = 10;
            while need < 24 && (1u64 << need) - 16 < n {
                need += 1;
            }
            lg = lg.min(need.max(16));
        }
        let need = size.map_or(u64::MAX, |n| n.max(1));
        let fit = |log: u32| {
            let mut l = log;
            while l > 8 && 1u64 << (l - 1) >= need {
                l -= 1;
            }
            l
        };
        let hash_log = fit(prm.hash_log);
        let chain_log = if prm.chain_log == 0 { 0 } else { fit(prm.chain_log.min(lg)) };
        let blog = if prm.chain_log == 0 { prm.depth.trailing_zeros() } else { 0 };
        Core {
            hash: vec![0; 1 << (hash_log + blog)],
            chain: if chain_log == 0 { Vec::new() } else { vec![0; 1 << chain_log] },
            num: if blog == 0 { Vec::new() } else { vec![0; 1 << hash_log] },
            blog,
            hash_log,
            cmask: (1usize << chain_log) - 1,
            next: 0,
            cache: [4, 11, 15, 16],
            lookups: 0,
            hits: 0,
            cmds: Vec::new(),
            window: (1usize << lg) - 16,
            lgwin: lg,
            prm,
        }
    }

    /// the buffer lost its first `k` bytes
    pub(super) fn shift(&mut self, k: usize) {
        self.next = self.next.saturating_sub(k);
        let k = k as u32;
        for e in self.hash.iter_mut().chain(self.chain.iter_mut()) {
            *e = e.saturating_sub(k);
        }
    }

    #[inline(always)]
    fn h(&self, s: &[u8], p: usize) -> usize {
        (rd32(s, p).wrapping_mul(0x1E35_A7BD) >> (32 - self.hash_log)) as usize
    }

    #[inline(always)]
    fn insert(&mut self, s: &[u8], q: usize) {
        if q + 8 > s.len() {
            return;
        }
        let h = self.h(s, q);
        if !self.chain.is_empty() {
            self.chain[q & self.cmask] = self.hash[h];
            self.hash[h] = q as u32 + 1;
        } else if self.blog == 0 {
            self.hash[h] = q as u32 + 1;
        } else {
            let i = self.num[h] as usize & ((1 << self.blog) - 1);
            self.hash[(h << self.blog) + i] = q as u32 + 1;
            self.num[h] = self.num[h].wrapping_add(1);
        }
    }

    /// the best match at p that ends by `end` into `best`; p enters the tables
    fn search(&mut self, s: &[u8], p: usize, end: usize, maxd: usize, best: &mut Cand) {
        let limit = end - p;
        let c = self.cache;
        for i in 0..self.prm.cache {
            let d = if i < 4 { c[i] } else { cached(&c, i) } as usize;
            if d == 0 || d > maxd || d > p {
                continue;
            }
            let q = p - d;
            // two bytes at least, and longer than the best so far
            if limit < 2 || s[q] != s[p] || s[q + 1] != s[p + 1] || (best.copy < limit && s[q + best.copy] != s[p + best.copy]) {
                continue;
            }
            let len = 2 + common(s, q + 2, p + 2, limit - 2);
            if len >= 3 || (len == 2 && i < 2) {
                let mut sc = SCORE_BASE + 15 + 135 * len as i64;
                if best.score < sc {
                    if i != 0 {
                        sc -= 39 + ((0x1CA10 >> (i & 0xE)) & 0xE);
                    }
                    if best.score < sc {
                        *best = Cand { copy: len, out: len, dist: d, score: sc };
                    }
                }
            }
        }
        if limit >= 4 {
            // the bucket of the hash, the last position first
            let h = self.h(s, p);
            let nb = 1usize << self.blog;
            let top = if self.blog == 0 { 0 } else { self.num[h] as usize };
            for k in 0..nb {
                let cand = self.hash[(h << self.blog) + (top.wrapping_sub(k + 1) & (nb - 1))];
                if cand == 0 {
                    break;
                }
                let q = cand as usize - 1;
                let d = p - q;
                if d > maxd || best.copy >= limit {
                    break; // older ones are farther; a farther one can only be as long
                }
                if s[q + best.copy] == s[p + best.copy] && rd32(s, q) == rd32(s, p) {
                    let len = 4 + common(s, q + 4, p + 4, limit - 4);
                    let sc = score(len, d);
                    if sc > best.score {
                        *best = Cand { copy: len, out: len, dist: d, score: sc };
                    }
                }
            }
        }
        if self.next <= p {
            self.insert(s, p);
            self.next = p + 1;
        }
        if best.score == MIN_SCORE && self.prm.dict && self.hits >= self.lookups >> 7 {
            self.dict_search(s, p, end, maxd, best, None);
        }
    }

    /// a word of the static dictionary at p, transformed: the best one into `best`, or
    /// (for the optimal parse) every one into `all`
    fn dict_search(&mut self, s: &[u8], p: usize, end: usize, maxd: usize, best: &mut Cand, mut all: Option<&mut Vec<Mt>>) {
        self.lookups += 1;
        let mut found = false;
        for pre in 0..2 {
            if pre == 1 && s[p] != b' ' {
                break;
            }
            let q = p + pre;
            if q + 4 > end {
                break;
            }
            for up in 0..2 {
                let mut first = s[q];
                if up == 1 {
                    if !first.is_ascii_uppercase() {
                        break;
                    }
                    first |= 0x20;
                }
                let key = dict_key(&[first, s[q + 1], s[q + 2], s[q + 3]], 0);
                let fam = pre * 2 + up;
                let fam = &FAMS.t[fam][..FAMS.n[fam]];
                let mut w = DIDX.head[key];
                let mut steps = 0;
                while w != 0 && steps < 64 {
                    let id = w as usize - 1;
                    w = DIDX.next[id];
                    steps += 1;
                    let l = DIDX.len[id] as usize;
                    let idx = id - DIDX.base[l] as usize;
                    let o = T::OFFSETS[l] as usize + idx * l;
                    let word = &DICT[o..o + l];
                    if word[0] != first || word[1..4] != s[q + 1..q + 4] {
                        continue;
                    }
                    let mut c = 4;
                    let lim = l.min(end - q);
                    while c < lim && word[c] == s[q + c] {
                        c += 1;
                    }
                    let nb = T::SIZE_BITS[l];
                    for &(t, cut, suf) in fam {
                        // a transform that cuts more than the word gives no match (a release
                        // build wrapped it into a length no match reaches; a debug build panicked)
                        let Some(wl) = l.checked_sub(cut as usize) else { continue };
                        if c < wl {
                            continue;
                        }
                        let sfx = affix(suf);
                        let at = q + wl;
                        if at + sfx.len() > end || s[at..at + sfx.len()] != *sfx {
                            continue;
                        }
                        let out = pre + wl + sfx.len();
                        let dist = maxd + 1 + (((t as usize) << nb) | idx);
                        if let Some(v) = all.as_mut() {
                            if out >= 4 {
                                v.push(Mt { out: out as u32, copy: l as u32, dist: dist as u32 | DICT_FLAG });
                                found = true;
                            }
                            continue;
                        }
                        let sc = score(out, dist);
                        if sc > best.score {
                            *best = Cand { copy: l, out, dist, score: sc };
                            found = true;
                        }
                    }
                }
            }
        }
        self.hits += found as u32;
    }

    /// the commands of s[bs..be] (base: the stream position of s[0])
    fn parse(&mut self, s: &[u8], bs: usize, be: usize, base: u64) {
        self.cmds.clear();
        let spree = self.prm.spree;
        let window = self.window as u64;
        let maxd = |p: usize| (base + p as u64).min(window) as usize;
        let (mut p, mut anchor) = (bs, bs);
        let mut skip_after = bs + spree;
        while p + 8 <= be {
            let mut m = NONE;
            self.search(s, p, be, maxd(p), &mut m);
            if m.score <= MIN_SCORE {
                p += 1;
                if p > skip_after && p + 8 <= be {
                    // data without matches: enter fewer positions, look less often
                    let (jump, step) = if p > skip_after + 4 * spree { (16, 4) } else { (8, 2) };
                    let to = (p + jump).min(be - 8);
                    while p < to {
                        self.insert(s, p);
                        p += step;
                    }
                    self.next = self.next.max(p);
                }
                continue;
            }
            if self.prm.lazy {
                let mut delayed = 0;
                while p + 9 <= be {
                    let mut m2 = NONE;
                    self.search(s, p + 1, be, maxd(p + 1), &mut m2);
                    if m2.score >= m.score + 175 {
                        p += 1;
                        m = m2;
                        delayed += 1;
                        if delayed < 4 {
                            continue;
                        }
                    }
                    break;
                }
            }
            let dcode = if m.dist <= maxd(p) {
                let dc = dist_code(m.dist as u32, &self.cache);
                if dc != 0 {
                    self.cache = [m.dist as u32, self.cache[0], self.cache[1], self.cache[2]];
                }
                dc
            } else {
                m.dist as u32 + 15
            };
            self.cmds.push(Cmd { ins: (p - anchor) as u32, copy: m.copy as u32, out: m.out as u32, dcode });
            let e = p + m.out;
            let mut from = self.next.max(p + 1);
            if m.dist < m.out >> 2 {
                // a run (a long match at a short distance): its end is enough, the tables keep
                // where it began, which a later copy of it finds (as libbrotli)
                from = from.max(e - 4 * m.dist);
            }
            if self.prm.depth > 1 {
                for q in from..e {
                    self.insert(s, q);
                }
            } else {
                // the fast levels: the end of the match is enough to find it again
                for q in from.max(e.saturating_sub(2))..e {
                    self.insert(s, q);
                }
            }
            self.next = self.next.max(e);
            p = e;
            anchor = p;
            skip_after = p + spree;
        }
        if anchor < be {
            self.cmds.push(Cmd { ins: (be - anchor) as u32, copy: 0, out: 0, dcode: 0 });
        }
    }

    /// compress s[bs..be] as one meta-block (base: the stream position of s[0]); `last` ends
    /// the stream with it
    pub(super) fn block(&mut self, s: &[u8], bs: usize, be: usize, base: u64, last: bool, bw: &mut Bw) {
        let cache = self.cache;
        if self.prm.opt > 0 {
            self.parse_opt(s, bs, be, base);
        } else {
            self.parse(s, bs, be, base);
        }
        let mark = bw.mark();
        let at = bw.bits();
        self.write(s, bs, be, last, bw);
        let used = bw.bits() - at;
        let mlen = be - bs;
        let raw = 8 * mlen + 4 + 4 * if mlen <= 1 << 16 { 4 } else if mlen <= 1 << 20 { 5 } else { 6 } + 8;
        if used > raw {
            bw.reset(mark);
            self.cache = cache; // the decoder does not see these distances
            stored(bw, &s[bs..be]);
            if last {
                end(bw);
            }
        } else if last {
            bw.align();
        }
    }

    /// the symbols of the commands and their histograms
    fn hists(&self, s: &[u8], bs: usize, be: usize) -> Hists {
        let modes: &[u32] = if !self.prm.ctx {
            &[0]
        } else if self.prm.both {
            &[2, 3]
        } else if texty(&s[bs..be]) {
            &[2]
        } else {
            &[3]
        };
        let mut h = Hists::new(self.cmds.len(), modes);
        let mut pos = bs;
        for c in &self.cmds {
            let ic = ins_code(c.ins);
            let cc = if c.copy == 0 { 0 } else { copy_code(c.copy) };
            let sym = cmd_symbol(ic, cc, c.copy == 0 || c.dcode == 0);
            h.cmd[sym as usize] += 1;
            let mut ds = u16::MAX;
            if c.copy != 0 && sym >= 128 {
                let (d, _, _) = dist_symbol(c.dcode);
                h.dist[dctx(c.copy) * DSIZE + d as usize] += 1;
                ds = d as u16;
            }
            h.syms.push((sym as u16, ds));
            h.literals(s, pos, pos + c.ins as usize);
            pos += c.ins as usize + c.out as usize;
        }
        debug_assert_eq!(pos, be);
        h
    }

    /// the commands as a compressed meta-block
    fn write(&self, s: &[u8], bs: usize, be: usize, last: bool, bw: &mut Bw) {
        let Hists { syms, cmd: cmd_h, dist: dist_h, lits } = self.hists(s, bs, be);
        let ctx = |lut: &[u8], i: usize| context(s, lut, i);
        let (mode, lmap, lhs) = literal_trees(&lits);
        let (dmap, dhs, _) = cluster(&dist_h, DSIZE, 4);
        // the header
        mlen_header(bw, last, be - bs);
        if !last {
            bw.put(0, 1); // ISUNCOMPRESSED
        }
        bw.put(0, 3); // one block type of literals, commands and distances each
        bw.put(0, 6); // NPOSTFIX, NDIRECT
        bw.put(mode, 2);
        store_cmap(bw, &lmap, lhs.len());
        store_cmap(bw, &dmap, dhs.len());
        let mut llens = vec![0u8; 256 * lhs.len()];
        let mut lcodes = vec![0u16; 256 * lhs.len()];
        for (k, h) in lhs.iter().enumerate() {
            store_code(bw, h, &mut llens[k * 256..(k + 1) * 256], &mut lcodes[k * 256..(k + 1) * 256]);
        }
        let mut clens = vec![0u8; 704];
        let mut ccodes = vec![0u16; 704];
        store_code(bw, &cmd_h, &mut clens, &mut ccodes);
        let mut dlens = vec![0u8; DSIZE * dhs.len()];
        let mut dcodes = vec![0u16; DSIZE * dhs.len()];
        for (k, h) in dhs.iter().enumerate() {
            store_code(bw, h, &mut dlens[k * DSIZE..(k + 1) * DSIZE], &mut dcodes[k * DSIZE..(k + 1) * DSIZE]);
        }
        // the data
        let lut = &T::CONTEXT_LUT[(mode as usize) << 9..((mode as usize) + 1) << 9];
        let mut pos = bs;
        for (c, &(sym, ds)) in self.cmds.iter().zip(&syms) {
            let sym = sym as usize;
            bw.put(ccodes[sym] as u32, clens[sym] as u32);
            let ic = ins_code(c.ins) as usize;
            bw.put(c.ins - INS[ic].0, INS[ic].1);
            if c.copy != 0 {
                let cc = copy_code(c.copy) as usize;
                bw.put(c.copy - COPY[cc].0, COPY[cc].1);
            }
            for i in pos..pos + c.ins as usize {
                let t = if lhs.len() > 1 { lmap[ctx(lut, i)] as usize * 256 } else { 0 };
                let b = t + s[i] as usize;
                bw.put(lcodes[b] as u32, llens[b] as u32);
            }
            pos += c.ins as usize;
            if ds != u16::MAX {
                let t = dmap[dctx(c.copy)] as usize * DSIZE + ds as usize;
                bw.put(dcodes[t] as u32, dlens[t] as u32);
                let (_, x, nb) = dist_symbol(c.dcode);
                bw.put(x, nb);
            }
            pos += c.out as usize;
        }
    }

    // ------------------------------------------------------------ the optimal parse

    /// every match at p longer than the nearer ones, and the dictionary's words, into ms; p
    /// enters the tables -> the longest length and its distance
    fn find_all(&mut self, s: &[u8], p: usize, end: usize, maxd: usize, ms: &mut Vec<Mt>) -> (usize, usize) {
        let limit = end - p;
        let mut best = 3usize;
        let mut far = 0;
        let mut cand = self.hash[self.h(s, p)];
        let mut k = 0;
        while cand != 0 && k < self.prm.depth {
            let q = cand as usize - 1;
            let d = p - q;
            if d > maxd {
                break;
            }
            if s[q + best] == s[p + best] {
                let len = common(s, q, p, limit);
                if len > best {
                    ms.push(Mt { out: len as u32, copy: len as u32, dist: d as u32 });
                    best = len;
                    far = d;
                    if len == limit {
                        break;
                    }
                }
            }
            k += 1;
            if self.chain.is_empty() || d > self.cmask {
                break;
            }
            cand = self.chain[q & self.cmask];
        }
        if self.next <= p {
            self.insert(s, p);
            self.next = p + 1;
        }
        if self.prm.dict && self.hits >= self.lookups >> 7 {
            let mut none = NONE;
            self.dict_search(s, p, end, maxd, &mut none, Some(ms));
        }
        if best > 3 { (best, far) } else { (0, 0) }
    }

    /// near-optimal commands for s[bs..be] (levels 10 and 11): the cheapest path over the
    /// positions under a cost model, as libbrotli's Zopfli parse; the first round with
    /// estimated costs, a second one with the costs of the first one's commands
    fn parse_opt(&mut self, s: &[u8], bs: usize, be: usize, base: u64) {
        let n = be - bs;
        let window = self.window as u64;
        // the matches of every position, found once: the tables then hold the whole block
        let mut ms: Vec<Mt> = Vec::new();
        let mut at = vec![0u32; n + 1];
        let (mut skip, mut tail) = (0, 0);
        for k in 0..n {
            at[k] = ms.len() as u32;
            let p = bs + k;
            if p + 8 > be {
                continue;
            }
            if k < skip {
                // inside a long match: only enter the position (of a run only its end)
                if k >= tail {
                    self.insert(s, p);
                }
                self.next = p + 1;
                continue;
            }
            let maxd = (base + p as u64).min(window) as usize;
            let (longest, d) = self.find_all(s, p, be, maxd, &mut ms);
            if longest > self.prm.nice {
                skip = k + longest;
                tail = if d < longest >> 2 { skip - 4 * d } else { 0 };
            }
        }
        at[n] = ms.len() as u32;
        let cache = self.cache;
        let mut model = Model::estimated(s, bs, be);
        for round in 0..self.prm.opt {
            if round > 0 {
                model = Model::of(&self.hists(s, bs, be), s, bs, be);
                self.cache = cache;
            }
            self.shortest(s, bs, be, base, &model, &ms, &at);
        }
    }

    /// the cheapest path through s[bs..be] -> self.cmds (and the distance cache after them)
    fn shortest(&mut self, s: &[u8], bs: usize, be: usize, base: u64, m: &Model, ms: &[Mt], at: &[u32]) {
        let n = be - bs;
        let window = self.window as u64;
        let nice = self.prm.nice;
        let inf = Node { cost: u64::MAX, out: 0, copy: 0, dist: 0, lits: 0, cache: [0; 4] };
        let mut node = vec![inf; n + 1];
        node[0] = Node { cost: 0, cache: self.cache, ..inf };
        let ins_x = |l: u32| INS[ins_code(l) as usize].1 as u64 * Q;
        let mut skip = 0; // inside a match longer than nice: the positions are not expanded
        for k in 0..n {
            if k < skip {
                continue;
            }
            let nd = node[k];
            let p = bs + k;
            // a literal (with the extra bits its insert length grows by)
            let c = nd.cost + m.lit[k] as u64 + ins_x(nd.lits + 1) - ins_x(nd.lits);
            if c < node[k + 1].cost {
                node[k + 1] = Node { cost: c, out: 0, copy: 0, dist: 0, lits: nd.lits + 1, cache: nd.cache };
            }
            let limit = be - p;
            if limit < 2 {
                continue;
            }
            let maxd = (base + p as u64).min(window) as usize;
            let ic = ins_code(nd.lits);
            let cc0 = nd.cache;
            let relax = |l: usize, copy: u32, dist: u32, dcode: u32, node: &mut [Node]| {
                let cc = copy_code(copy);
                let sym = cmd_symbol(ic, cc, dcode == 0);
                let mut c = nd.cost + m.cmd[sym as usize] as u64 + COPY[cc as usize].1 as u64 * Q;
                if sym >= 128 {
                    let (ds, _, nb) = dist_symbol(dcode);
                    c += m.dist[dctx(copy) * DSIZE + ds as usize] as u64 + nb as u64 * Q;
                }
                if c < node[k + l].cost {
                    let cache = if dcode == 0 || dist & DICT_FLAG != 0 { cc0 } else { [dist, cc0[0], cc0[1], cc0[2]] };
                    node[k + l] = Node { cost: c, out: l as u32, copy, dist, lits: 0, cache };
                }
            };
            // the distances of the cache, each by its own code
            for j in 0..self.prm.cache {
                let d = cached(&cc0, j);
                if d == 0 || d as usize > maxd || d as usize > p || dist_code(d, &cc0) != j as u32 {
                    continue;
                }
                let len = common(s, p - d as usize, p, limit);
                if len > nice {
                    relax(len, len as u32, d, j as u32, &mut node);
                    skip = skip.max(k + len);
                    continue;
                }
                for l in (if j < 2 { 2 } else { 3 })..=len {
                    relax(l, l as u32, d, j as u32, &mut node);
                }
            }
            // the matches of the position
            let mut prev = 3;
            for mt in &ms[at[k] as usize..at[k + 1] as usize] {
                if mt.dist & DICT_FLAG != 0 {
                    relax(mt.out as usize, mt.copy, mt.dist, (mt.dist & !DICT_FLAG) + 15, &mut node);
                    continue;
                }
                let dc = dist_code(mt.dist, &cc0);
                let top = mt.out as usize;
                let from = if top > nice { top } else { prev + 1 };
                for l in from..=top {
                    relax(l, l as u32, mt.dist, dc, &mut node);
                }
                if top > nice {
                    skip = skip.max(k + top);
                }
                prev = top;
            }
        }
        // the path back from the end
        let mut steps: Vec<(usize, Node)> = Vec::new();
        let mut k = n;
        while k > 0 {
            let nd = node[k];
            if nd.out == 0 {
                k -= 1;
            } else {
                k -= nd.out as usize;
                steps.push((k, nd));
            }
        }
        self.cmds.clear();
        let mut anchor = 0;
        for &(k, nd) in steps.iter().rev() {
            let dcode = if nd.dist & DICT_FLAG != 0 {
                (nd.dist & !DICT_FLAG) + 15
            } else {
                let dc = dist_code(nd.dist, &self.cache);
                if dc != 0 {
                    self.cache = [nd.dist, self.cache[0], self.cache[1], self.cache[2]];
                }
                dc
            };
            self.cmds.push(Cmd { ins: (k - anchor) as u32, copy: nd.copy, out: nd.out, dcode });
            anchor = k + nd.out as usize;
        }
        if anchor < n {
            self.cmds.push(Cmd { ins: (n - anchor) as u32, copy: 0, out: 0, dcode: 0 });
        }
    }
}

/// the largest stream compress() makes from n bytes
pub const fn bound(n: usize) -> usize {
    n + (n >> 10) + 16
}

/// a Brotli stream of `data` at `level` (0..=11; brotli::LEVEL_DEFAULT is 5), with the
/// smallest window that holds the data (at most 4 MiB, 16 MiB at levels 10 and 11)
pub fn compress(data: &[u8], level: i32) -> Vec<u8> {
    let mut core = Core::new(level, Some(data.len() as u64), None);
    let mut bw = Bw::new();
    bw.out.reserve(data.len() / 3 + 16);
    wbits(&mut bw, core.lgwin);
    let blk = 1usize << core.prm.block_log;
    let mut bs = 0;
    while bs < data.len() {
        let be = (bs + blk).min(data.len());
        core.block(data, bs, be, 0, be == data.len(), &mut bw);
        bs = be;
    }
    if data.is_empty() {
        end(&mut bw);
    }
    bw.out
}
