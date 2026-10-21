// eans -- entropy coding of the new encoder: tokens, hybrid uint configs, histogram clustering,
// ANS histograms with a chosen precision, prefix codes, context maps, LZ77 (libjxl 0.11.1
// enc_ans.cc, enc_cluster.cc, enc_context_map.cc, enc_huffman.cc, enc_huffman_tree.cc)
//                                                                          [jxl-plus: no asm]
//
//   Huc                        HybridUintConfig: encode(v) -> (token, extra bits count, bits)
//   Token                      (context, value); bit 31 of the context marks an LZ77 length
//   HParams                    HistogramParams (clustering, uint method, LZ77, ANS strategy;
//                              jxl-speed: par = the per-stream passes on the worker pool)
//   build(params, nctx, streams, jw) -> Code   BuildAndEncodeHistograms: LZ77 (may rewrite
//                              the streams), clustering, configs, histograms written to jw
//                              (None: cost estimate only, Code.cost)
//   write_tokens(tokens, code, jw)            WriteTokens (ANS backwards, or prefix codes)
//   encode_ctxmap(map, n, jw)                 EncodeContextMap
//
// Everything is deterministic: no hash maps with random state, no float reductions whose
// order depends on threads.

use alloc::collections::BinaryHeap;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::emath::{ceilf, fast_log2f};
use crate::ent::{ej_alias, Ae};
use crate::ewr::Jw;
use crate::tab;

pub const ANS_LOG_TAB: u32 = 12;
pub const ANS_TAB: i32 = 1 << ANS_LOG_TAB;
pub const ANS_MAX_ALPHA: usize = 256;
pub const CLUSTERS_LIMIT: usize = 128;
const PREFIX_MAX_BITS: u32 = 15;
const PREFIX_MAX_ALPHA: usize = 4096;
const MAX_SMALL: usize = 4;
const WINDOW: usize = 1 << 20;
pub const TOK_LZ: u32 = 1 << 31;

// ------------------------------------------------------------------------- tokens

/// HybridUintConfig
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Huc {
    pub se: u32,
    pub msb: u32,
    pub lsb: u32,
}

impl Default for Huc {
    fn default() -> Self {
        Huc::new(4, 2, 0)
    }
}

impl Huc {
    pub const fn new(se: u32, msb: u32, lsb: u32) -> Huc {
        Huc { se, msb, lsb }
    }

    /// (token, nbits, bits)
    #[inline(always)]
    pub fn encode(&self, v: u32) -> (u32, u32, u32) {
        let split = 1u32 << self.se;
        if v < split {
            return (v, 0, 0);
        }
        let n = 31 - v.leading_zeros();
        let m = v - (1 << n);
        let tok = split
            + ((n - self.se) << (self.msb + self.lsb))
            + ((m >> (n - self.msb)) << self.lsb)
            + (m & ((1 << self.lsb) - 1));
        let nbits = n - self.msb - self.lsb;
        let bits = (v >> self.lsb) & ((1u32 << nbits).wrapping_sub(1));
        (tok, nbits, bits)
    }

    /// EncodeUintConfig
    fn write<S: Sink>(&self, w: &mut S, loga: u32) {
        w.w(ceil_log2(loga + 1), self.se as u64);
        if self.se == loga {
            return;
        }
        w.w(ceil_log2(self.se + 1), self.msb as u64);
        w.w(ceil_log2(self.se - self.msb + 1), self.lsb as u64);
    }
}

/// a token: context (bit 31: LZ77 length) and value
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Token {
    pub ctx: u32,
    pub val: u32,
}

impl Token {
    #[inline(always)]
    pub fn new(ctx: u32, val: u32) -> Token {
        Token { ctx, val }
    }
    #[inline(always)]
    pub fn lz(&self) -> bool {
        self.ctx & TOK_LZ != 0
    }
    #[inline(always)]
    pub fn c(&self) -> usize {
        (self.ctx & !TOK_LZ) as usize
    }
}

/// PackSigned
#[inline(always)]
pub fn pack_signed(v: i32) -> u32 {
    ((v as u32) << 1) ^ ((v >> 31) as u32)
}

#[inline(always)]
pub fn ceil_log2(x: u32) -> u32 {
    if x <= 1 { 0 } else { 32 - (x - 1).leading_zeros() }
}

#[inline(always)]
fn floor_log2(x: u32) -> u32 {
    31 - x.leading_zeros()
}

/// a bit sink: the writer or a size counter
pub trait Sink {
    fn w(&mut self, n: u32, v: u64);
}

impl Sink for Jw {
    #[inline(always)]
    fn w(&mut self, n: u32, v: u64) {
        if n != 0 {
            self.bits(v, n);
        }
    }
}

/// counts bits (SizeWriter)
#[derive(Default)]
pub struct SizeW(pub usize);

impl Sink for SizeW {
    #[inline(always)]
    fn w(&mut self, n: u32, _v: u64) {
        self.0 += n as usize;
    }
}

fn varlen8<S: Sink>(w: &mut S, n: u32) {
    if n == 0 {
        w.w(1, 0);
    } else {
        w.w(1, 1);
        let nb = floor_log2(n);
        w.w(3, nb as u64);
        w.w(nb, (n - (1 << nb)) as u64);
    }
}

fn varlen16<S: Sink>(w: &mut S, n: u32) {
    if n == 0 {
        w.w(1, 0);
    } else {
        w.w(1, 1);
        let nb = floor_log2(n);
        w.w(4, nb as u64);
        w.w(nb, (n - (1 << nb)) as u64);
    }
}

// ------------------------------------------------------------------------- histograms

/// a histogram (Histogram)
#[derive(Clone, Default)]
pub struct Histo {
    pub d: Vec<i32>,
    pub total: usize,
    pub entropy: f32,
}

impl Histo {
    #[inline]
    pub fn add(&mut self, s: usize) {
        if self.d.len() <= s {
            self.d.resize((s + 8) & !7, 0);
        }
        self.d[s] += 1;
        self.total += 1;
    }
    pub fn add_h(&mut self, o: &Histo) {
        if o.d.len() > self.d.len() {
            self.d.resize(o.d.len(), 0);
        }
        for (a, &b) in self.d.iter_mut().zip(o.d.iter()) {
            *a += b;
        }
        self.total += o.total;
    }
    pub fn clear(&mut self) {
        self.d.clear();
        self.total = 0;
    }
    pub fn alphabet(&self) -> usize {
        let mut i = self.d.len();
        while i > 0 {
            if self.d[i - 1] > 0 {
                return i;
            }
            i -= 1;
        }
        1
    }
    /// HistogramEntropy (stores and returns the Shannon entropy in bits)
    pub fn shannon(&mut self) -> f32 {
        self.entropy = entropy_of(&self.d, self.total);
        self.entropy
    }
}

fn entropy_of(d: &[i32], total: usize) -> f32 {
    if total == 0 {
        return 0.0;
    }
    let inv = 1.0f32 / total as f32;
    let mut e = 0.0f32;
    for &c in d {
        if c != 0 && c as usize != total {
            let cf = c as f32;
            e = e - cf * fast_log2f(inv * cf);
        }
    }
    e
}

/// HistogramDistance: cost of merging a and b
fn hdist(a: &Histo, b: &Histo) -> f32 {
    if a.total == 0 || b.total == 0 {
        return 0.0;
    }
    let total = a.total + b.total;
    let inv = 1.0f32 / total as f32;
    let n = a.d.len().max(b.d.len());
    let mut e = 0.0f32;
    for i in 0..n {
        let c = a.d.get(i).copied().unwrap_or(0) + b.d.get(i).copied().unwrap_or(0);
        if c != 0 && c as usize != total {
            let cf = c as f32;
            e = e - cf * fast_log2f(inv * cf);
        }
    }
    e - a.entropy - b.entropy
}

// ------------------------------------------------------------------------- ANS histograms

#[inline]
fn pop_precision(logcount: i32, shift: u32) -> i32 {
    let r = logcount.min(shift as i32 - ((ANS_LOG_TAB as i32 - logcount) >> 1));
    r.max(0)
}

#[inline]
fn smallest_inc(count: i32, shift: u32) -> i32 {
    if count == 0 {
        return 1;
    }
    let bits = floor_log2(count as u32) as i32;
    let drop = bits - pop_precision(bits, shift);
    if drop < 0 { 1 } else { 1 << drop }
}

/// RebalanceHistogram
fn rebalance(targets: &[f32], max_symbol: usize, table: i32, shift: u32, minsum: bool, omit: &mut usize, counts: &mut [i32]) -> bool {
    let mut sum: i32 = 0;
    let mut sum_nr: f32 = 0.0;
    let mut rpos = 0usize;
    let mut rlog: i32 = -1;
    for n in 0..max_symbol {
        if targets[n] > 0.0 && targets[n] < 1.0 {
            counts[n] = 1;
            sum_nr += targets[n];
            sum += counts[n];
        }
    }
    let discount = (table - sum) as f32 / (table as f32 - sum_nr);
    if !(discount > 0.0 && discount <= 1.0) {
        return false;
    }
    for n in 0..max_symbol {
        if targets[n] >= 1.0 {
            sum_nr += targets[n];
            counts[n] = (targets[n] * discount) as i32;
            if counts[n] == 0 {
                counts[n] = 1;
            }
            if counts[n] == table {
                counts[n] = table - 1;
            }
            let inc = smallest_inc(counts[n], shift);
            counts[n] -= counts[n] & (inc - 1);
            let target = if minsum { sum_nr as i32 - sum } else { targets[n] as i32 };
            if counts[n] == 0 || (target >= counts[n] + inc / 2 && counts[n] + inc < table) {
                counts[n] += inc;
            }
            sum += counts[n];
            let cl = floor_log2(counts[n] as u32) as i32;
            if cl > rlog {
                rpos = n;
                rlog = cl;
            }
        }
    }
    counts[rpos] -= sum - table;
    *omit = rpos;
    counts[rpos] > 0
}

/// NormalizeCounts -> (num symbols, first symbols); false = failure
fn normalize(counts: &mut [i32], omit: &mut usize, shift: u32, nsym: &mut usize, syms: &mut [usize; MAX_SMALL]) -> bool {
    let table = ANS_TAB;
    let mut total: u64 = 0;
    let mut max_symbol = 0usize;
    let mut sc = 0usize;
    for (n, &c) in counts.iter().enumerate() {
        total += c as u64;
        if c > 0 {
            if sc < MAX_SMALL {
                syms[sc] = n;
            }
            sc += 1;
            max_symbol = n + 1;
        }
    }
    *nsym = sc;
    if sc == 0 {
        return true;
    }
    if sc == 1 {
        counts[syms[0]] = table;
        return true;
    }
    if sc > table as usize {
        return false;
    }
    let norm = table as f32 / total as f32;
    let targets: Vec<f32> = counts[..max_symbol].iter().map(|&c| norm * c as f32).collect();
    let orig: Vec<i32> = counts.to_vec();
    if !rebalance(&targets, max_symbol, table, shift, false, omit, counts) {
        counts.copy_from_slice(&orig);
        if !rebalance(&targets, max_symbol, table, shift, true, omit, counts) {
            return false;
        }
    }
    true
}

static LOGCOUNT_LEN: [u8; 14] = [5, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 6, 7, 7];
static LOGCOUNT_SYM: [u8; 14] = [17, 11, 15, 3, 9, 7, 4, 2, 5, 6, 0, 33, 1, 65];

/// EncodeCounts
fn encode_counts<S: Sink>(counts: &[i32], omit: usize, nsym: usize, shift: u32, syms: &[usize; MAX_SMALL], w: &mut S) -> bool {
    let alpha = counts.len();
    let mut ok = true;
    if nsym <= 2 {
        w.w(1, 1);
        if nsym == 0 {
            w.w(1, 0);
            varlen8(w, 0);
        } else {
            w.w(1, (nsym - 1) as u64);
            for &s in syms.iter().take(nsym) {
                varlen8(w, s as u32);
            }
        }
        if nsym == 2 {
            w.w(ANS_LOG_TAB, counts[syms[0]] as u64);
        }
        return true;
    }
    w.w(1, 0);
    w.w(1, 0);
    let mut same = vec![0u32; alpha];
    let mut last = 0usize;
    for i in 1..alpha {
        if counts[i] != counts[last] || i + 1 == alpha || i - last >= 255 || i == omit || i == omit + 1 {
            same[last] = (i - last) as u32;
            last = i + 1;
        }
    }
    let mut length = 0usize;
    let mut logcounts = vec![0i32; alpha];
    let mut omit_log = 0i32;
    for i in 0..alpha {
        if i == omit {
            length = i + 1;
        } else if counts[i] > 0 {
            logcounts[i] = floor_log2(counts[i] as u32) as i32 + 1;
            length = i + 1;
            if i < omit {
                omit_log = omit_log.max(logcounts[i] + 1);
            } else {
                omit_log = omit_log.max(logcounts[i]);
            }
        }
    }
    logcounts[omit] = omit_log;
    let ub = floor_log2(ANS_LOG_TAB + 1);
    let lg = floor_log2(shift + 1);
    w.w(lg, ((1u32 << lg) - 1) as u64);
    if lg != ub {
        w.w(1, 0);
    }
    w.w(lg, (((1u32 << lg) - 1) & (shift + 1)) as u64);
    if length < 3 || length - 3 > 255 {
        varlen8(w, 255);
        ok = false;
    } else {
        varlen8(w, (length - 3) as u32);
    }
    const MIN_REPS: u32 = 4;
    let rep = (ANS_LOG_TAB + 1) as usize;
    let mut i = 0usize;
    while i < length {
        if i > 0 && same[i - 1] > MIN_REPS {
            w.w(LOGCOUNT_LEN[rep] as u32, LOGCOUNT_SYM[rep] as u64);
            varlen8(w, same[i - 1] - MIN_REPS - 1);
            i += same[i - 1] as usize - 2;
            i += 1;
            continue;
        }
        let lc = logcounts[i] as usize;
        w.w(LOGCOUNT_LEN[lc] as u32, LOGCOUNT_SYM[lc] as u64);
        i += 1;
    }
    let mut i = 0usize;
    while i < length {
        if i > 0 && same[i - 1] > MIN_REPS {
            i += same[i - 1] as usize - 2;
            i += 1;
            continue;
        }
        if logcounts[i] > 1 && i != omit {
            let bc = pop_precision(logcounts[i] - 1, shift);
            let drop = logcounts[i] - 1 - bc;
            w.w(bc as u32, ((counts[i] >> drop) - (1 << bc)) as u64);
        }
        i += 1;
    }
    ok
}

fn data_bits(hist: &[i32], counts: &[i32]) -> f32 {
    let mut sum = 0.0f32;
    for i in 0..hist.len() {
        if hist[i] > 0 {
            let v = ANS_LOG_TAB as f32 - fast_log2f(counts[i] as f32);
            sum += hist[i] as f32 * if v > 0.0 { v } else { 0.0 };
        }
    }
    sum
}

fn data_bits_flat(hist: &[i32]) -> f32 {
    let fb = fast_log2f(hist.len() as f32).max(0.0);
    let t: f32 = hist.iter().map(|&c| c as f32).sum();
    t * fb
}

/// ComputeHistoAndDataCost (method 0 = flat, else shift = method - 1)
fn histo_cost(hist: &[i32], method: u32) -> f32 {
    if method == 0 {
        return ANS_LOG_TAB as f32 + 2.0 + data_bits_flat(hist);
    }
    let shift = method - 1;
    let mut counts = hist.to_vec();
    let mut omit = 0usize;
    let mut nsym = 0usize;
    let mut syms = [0usize; MAX_SMALL];
    if !normalize(&mut counts, &mut omit, shift, &mut nsym, &mut syms) {
        return f32::MAX;
    }
    let mut sw = SizeW(0);
    let _ = encode_counts(&counts, omit, nsym, shift, &syms, &mut sw);
    sw.0 as f32 + data_bits(hist, &counts)
}

/// ANS histogram strategy
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Strat {
    Fast,
    Approx,
    Precise,
}

/// ComputeBestMethod -> (method, cost)
fn best_method(hist: &[i32], strat: Strat) -> (u32, f32) {
    let mut method = 0u32;
    let mut cost = histo_cost(hist, 0);
    let tryit = |shift: u32, method: &mut u32, cost: &mut f32| {
        let c = histo_cost(hist, shift + 1);
        if c < *cost {
            *method = shift + 1;
            *cost = c;
        }
    };
    match strat {
        Strat::Precise => {
            for s in 0..=ANS_LOG_TAB {
                tryit(s, &mut method, &mut cost);
            }
        }
        Strat::Approx => {
            let mut s = 0;
            while s <= ANS_LOG_TAB {
                tryit(s, &mut method, &mut cost);
                s += 2;
            }
        }
        Strat::Fast => {
            tryit(0, &mut method, &mut cost);
            tryit(ANS_LOG_TAB / 2, &mut method, &mut cost);
            tryit(ANS_LOG_TAB, &mut method, &mut cost);
        }
    }
    (method, cost)
}

/// ANSPopulationCost
pub fn ans_pop_cost(hist: &[i32]) -> f32 {
    if hist.len() > ANS_MAX_ALPHA {
        return f32::MAX;
    }
    best_method(hist, Strat::Fast).1
}

fn flat_histo(len: usize, total: i32) -> Vec<i32> {
    let c = total / len as i32;
    let mut r = vec![c; len];
    let rem = (total % len as i32) as usize;
    for v in r.iter_mut().take(rem) {
        *v += 1;
    }
    r
}

/// the encoding of one histogram (ANSEncSymbolInfo of all symbols)
#[derive(Clone, Default)]
pub struct SymInfo {
    pub freq: Vec<u16>,  // ANS frequency per symbol
    pub cum: Vec<u16>,   // start of the symbol's reverse map entries
    pub rev: Vec<u16>,   // 4096 reverse map entries
    pub depth: Vec<u8>,  // prefix code
    pub bits: Vec<u16>,
}

/// BuildAndStoreANSEncodingData (ANS) -> (info, cost)
fn build_ans<S: Sink>(strat: Strat, hist: &[i32], loga: u32, w: Option<&mut S>) -> (SymInfo, usize) {
    let alpha = hist.len();
    let (method, fcost) = best_method(hist, strat);
    let mut counts = hist.to_vec();
    if !counts.is_empty() && counts.iter().all(|&c| c == 0) {
        counts[0] = ANS_TAB;
    }
    let mut omit = 0usize;
    let mut nsym = 0usize;
    let mut syms = [0usize; MAX_SMALL];
    if method == 0 {
        counts = flat_histo(alpha.max(1), ANS_TAB);
    } else {
        let ok = normalize(&mut counts, &mut omit, method - 1, &mut nsym, &mut syms);
        debug_assert!(ok);
    }
    // alias table -> reverse map
    let tsize = 1usize << loga;
    let mut at = vec![Ae::default(); tsize.max(256)];
    let mut scr = vec![0u32; tsize.max(260)];
    for (i, &c) in counts.iter().enumerate() {
        scr[i] = c as u32;
    }
    let _ = ej_alias(&mut scr, counts.len() as u32, loga, &mut at);
    let n = alpha.max(1);
    let mut info = SymInfo { freq: vec![0; n], cum: vec![0; n], rev: vec![0; ANS_TAB as usize], ..Default::default() };
    let mut run = 0u32;
    for s in 0..n {
        let f = if s < counts.len() { counts[s] as u32 } else { 0 };
        info.freq[s] = f.min(65535) as u16;
        info.cum[s] = run as u16;
        run += f;
    }
    let le = ANS_LOG_TAB - loga;
    let mask = (1u32 << le) - 1;
    for st in 0..ANS_TAB as u32 {
        let i = st >> le;
        let pos = st & mask;
        let e = at[i as usize];
        let (sym, off) = if pos < e.cutoff as u32 { (i, pos) } else { (e.right as u32, e.offset as u32 + pos) };
        if (sym as usize) < n {
            let k = (info.cum[sym as usize] as u32 + off) & 4095;
            info.rev[k as usize] = st as u16;
        }
    }
    if let Some(w) = w {
        if method == 0 {
            w.w(1, 0);
            w.w(1, 1);
            varlen8(w, (alpha.max(1) - 1) as u32);
        } else {
            let _ = encode_counts(&counts, omit, nsym, method - 1, &syms, w);
        }
    }
    (info, fcost as usize)
}

// ------------------------------------------------------------------------- prefix codes

/// CreateHuffmanTree: depths of the used symbols, at most `limit` bits
fn huff_depths(data: &[u32], limit: u32, depth: &mut [u8]) {
    #[derive(Clone, Copy)]
    struct Node {
        total: u32,
        left: i32,
        right: i32,
    }
    let len = data.len();
    let mut count_limit: u32 = 1;
    loop {
        let mut tree: Vec<Node> = Vec::with_capacity(2 * len + 1);
        let mut i = len;
        while i != 0 {
            i -= 1;
            if data[i] != 0 {
                let c = data[i].max(count_limit - 1);
                tree.push(Node { total: c, left: -1, right: i as i32 });
            }
        }
        let n = tree.len();
        if n == 1 {
            depth[tree[0].right as usize] = 1;
            return;
        }
        // stable sort by count
        tree.sort_by(|a, b| a.total.cmp(&b.total));
        let sentinel = Node { total: u32::MAX, left: -1, right: -1 };
        tree.push(sentinel);
        tree.push(sentinel);
        let mut i = 0usize;
        let mut j = n + 1;
        let mut k = n - 1;
        while k != 0 {
            let left;
            let right;
            if tree[i].total <= tree[j].total {
                left = i;
                i += 1;
            } else {
                left = j;
                j += 1;
            }
            if tree[i].total <= tree[j].total {
                right = i;
                i += 1;
            } else {
                right = j;
                j += 1;
            }
            let je = tree.len() - 1;
            tree[je].total = tree[left].total.wrapping_add(tree[right].total);
            tree[je].left = left as i32;
            tree[je].right = right as i32;
            tree.push(sentinel);
            k -= 1;
        }
        for d in depth.iter_mut().take(len) {
            *d = 0;
        }
        // SetDepth (iterative)
        let mut stack: Vec<(usize, u8)> = vec![(2 * n - 1, 0)];
        while let Some((p, lvl)) = stack.pop() {
            let nd = tree[p];
            if nd.left >= 0 {
                stack.push((nd.left as usize, lvl + 1));
                stack.push((nd.right as usize, lvl + 1));
            } else {
                depth[nd.right as usize] = lvl;
            }
        }
        let mx = depth[..len].iter().copied().max().unwrap_or(0);
        if mx as u32 <= limit {
            return;
        }
        count_limit *= 2;
    }
}

fn reverse_bits(n: u32, bits: u16) -> u16 {
    let mut r = 0u16;
    let mut b = bits;
    for _ in 0..n {
        r = r << 1 | (b & 1);
        b >>= 1;
    }
    r
}

/// ConvertBitDepthsToSymbols
fn depths_to_bits(depth: &[u8], bits: &mut [u16]) {
    let mut bl = [0u16; 16];
    for &d in depth {
        bl[d as usize] += 1;
    }
    bl[0] = 0;
    let mut next = [0u16; 16];
    let mut code = 0i32;
    for i in 1..16 {
        code = (code + bl[i - 1] as i32) << 1;
        next[i] = code as u16;
    }
    for (i, &d) in depth.iter().enumerate() {
        if d != 0 {
            bits[i] = reverse_bits(d as u32, next[d as usize]);
            next[d as usize] = next[d as usize].wrapping_add(1);
        }
    }
}

fn tree_reps(prev: u8, value: u8, mut reps: usize, tree: &mut Vec<u8>, extra: &mut Vec<u8>) {
    if prev != value {
        tree.push(value);
        extra.push(0);
        reps -= 1;
    }
    if reps == 7 {
        tree.push(value);
        extra.push(0);
        reps -= 1;
    }
    if reps < 3 {
        for _ in 0..reps {
            tree.push(value);
            extra.push(0);
        }
    } else {
        reps -= 3;
        let start = tree.len();
        loop {
            tree.push(16);
            extra.push((reps & 3) as u8);
            reps >>= 2;
            if reps == 0 {
                break;
            }
            reps -= 1;
        }
        tree[start..].reverse();
        extra[start..].reverse();
    }
}

fn tree_zeros(mut reps: usize, tree: &mut Vec<u8>, extra: &mut Vec<u8>) {
    if reps == 11 {
        tree.push(0);
        extra.push(0);
        reps -= 1;
    }
    if reps < 3 {
        for _ in 0..reps {
            tree.push(0);
            extra.push(0);
        }
    } else {
        reps -= 3;
        let start = tree.len();
        loop {
            tree.push(17);
            extra.push((reps & 7) as u8);
            reps >>= 3;
            if reps == 0 {
                break;
            }
            reps -= 1;
        }
        tree[start..].reverse();
        extra[start..].reverse();
    }
}

/// WriteHuffmanTree: the RLE representation of the depths
fn write_tree(depth: &[u8], tree: &mut Vec<u8>, extra: &mut Vec<u8>) {
    let length = depth.len();
    let mut nl = length;
    while nl > 0 && depth[nl - 1] == 0 {
        nl -= 1;
    }
    let (mut rle_nz, mut rle_z) = (false, false);
    if length > 50 {
        let (mut trz, mut trn, mut crz, mut crn) = (0usize, 0usize, 1usize, 1usize);
        let mut i = 0;
        while i < nl {
            let v = depth[i];
            let mut reps = 1;
            while i + reps < nl && depth[i + reps] == v {
                reps += 1;
            }
            if reps >= 3 && v == 0 {
                trz += reps;
                crz += 1;
            }
            if reps >= 4 && v != 0 {
                trn += reps;
                crn += 1;
            }
            i += reps;
        }
        rle_nz = trn > crn * 2;
        rle_z = trz > crz * 2;
    }
    let mut prev = 8u8;
    let mut i = 0;
    while i < nl {
        let v = depth[i];
        let mut reps = 1;
        if (v != 0 && rle_nz) || (v == 0 && rle_z) {
            while i + reps < nl && depth[i + reps] == v {
                reps += 1;
            }
        }
        if v == 0 {
            tree_zeros(reps, tree, extra);
        } else {
            tree_reps(prev, v, reps, tree, extra);
            prev = v;
        }
        i += reps;
    }
}

static STORAGE_ORDER: [usize; 18] = [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15];
static CL_SYM: [u8; 6] = [0, 7, 3, 2, 1, 15];
static CL_LEN: [u8; 6] = [2, 4, 3, 2, 2, 4];

/// StoreHuffmanTree
fn store_tree<S: Sink>(depth: &[u8], w: &mut S) {
    let mut tree = Vec::new();
    let mut extra = Vec::new();
    write_tree(depth, &mut tree, &mut extra);
    let mut hist = [0u32; 18];
    for &t in &tree {
        hist[t as usize] += 1;
    }
    let mut ncodes = 0;
    let mut code = 0usize;
    for (i, &h) in hist.iter().enumerate() {
        if h != 0 {
            if ncodes == 0 {
                code = i;
                ncodes = 1;
            } else if ncodes == 1 {
                ncodes = 2;
                break;
            }
        }
    }
    let mut cld = [0u8; 18];
    let mut clb = [0u16; 18];
    huff_depths(&hist, 5, &mut cld);
    depths_to_bits(&cld, &mut clb);
    // StoreHuffmanTreeOfHuffmanTreeToBitMask
    let mut to_store = 18usize;
    if ncodes > 1 {
        while to_store > 0 && cld[STORAGE_ORDER[to_store - 1]] == 0 {
            to_store -= 1;
        }
    }
    let mut skip = 0usize;
    if cld[STORAGE_ORDER[0]] == 0 && cld[STORAGE_ORDER[1]] == 0 {
        skip = 2;
        if cld[STORAGE_ORDER[2]] == 0 {
            skip = 3;
        }
    }
    w.w(2, skip as u64);
    for &o in STORAGE_ORDER.iter().take(to_store).skip(skip) {
        let l = cld[o] as usize;
        w.w(CL_LEN[l] as u32, CL_SYM[l] as u64);
    }
    if ncodes == 1 {
        cld[code] = 0;
    }
    for (k, &ix) in tree.iter().enumerate() {
        let ix = ix as usize;
        w.w(cld[ix] as u32, clb[ix] as u64);
        match ix {
            16 => w.w(2, extra[k] as u64),
            17 => w.w(3, extra[k] as u64),
            _ => {}
        }
    }
}

/// BuildAndStoreHuffmanTree -> SymInfo (depth / bits), cost (bits of the tree)
fn build_prefix<S: Sink>(hist: &[i32], w: &mut S) -> SymInfo {
    let len = hist.len();
    let h: Vec<u32> = hist.iter().map(|&c| c.max(0) as u32).collect();
    let mut info = SymInfo { depth: vec![0; len], bits: vec![0; len], ..Default::default() };
    if len <= 1 {
        return info;
    }
    let mut count = 0usize;
    let mut s4 = [0usize; 4];
    for (i, &c) in h.iter().enumerate() {
        if c != 0 {
            if count < 4 {
                s4[count] = i;
            } else if count > 4 {
                break;
            }
            count += 1;
        }
    }
    let mut mbc = len - 1;
    let mut max_bits = 0u32;
    while mbc != 0 {
        mbc >>= 1;
        max_bits += 1;
    }
    if count <= 1 {
        w.w(4, 1);
        w.w(max_bits, s4[0] as u64);
        return info;
    }
    huff_depths(&h, PREFIX_MAX_BITS, &mut info.depth);
    depths_to_bits(&info.depth, &mut info.bits);
    if count <= 4 {
        // StoreSimpleHuffmanTree
        w.w(2, 1);
        w.w(2, (count - 1) as u64);
        for i in 0..count {
            for j in i + 1..count {
                if info.depth[s4[j]] < info.depth[s4[i]] {
                    s4.swap(i, j);
                }
            }
        }
        for &s in s4.iter().take(count) {
            w.w(max_bits, s as u64);
        }
        if count == 4 {
            w.w(1, (info.depth[s4[0]] == 1) as u64);
        }
    } else {
        store_tree(&info.depth, w);
    }
    info
}

// ------------------------------------------------------------------------- clustering

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Clu {
    Fastest,
    Fast,
    Best,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UintM {
    None,
    K000,
    Fast,
    CtxMap,
    Best,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lz {
    None,
    Rle,
    Lz77,
    Optimal,
}

/// HistogramParams
#[derive(Clone)]
pub struct HParams {
    pub clu: Clu,
    pub uint: UintM,
    pub lz: Lz,
    pub strat: Strat,
    pub widths: Vec<usize>,
    pub max_hist: usize,
    pub force_huffman: bool,
    /// jxl-speed: the histogram passes over the streams on the worker pool (the same counts)
    pub par: bool,
}

impl Default for HParams {
    fn default() -> Self {
        HParams {
            clu: Clu::Best,
            uint: UintM::Best,
            lz: Lz::Rle,
            strat: Strat::Precise,
            widths: Vec::new(),
            max_hist: usize::MAX,
            force_huffman: false,
            par: false,
        }
    }
}

impl HParams {
    /// HistogramParams( speed_tier, num_ctx ) (VarDCT AC)
    pub fn for_tier(tier: i32) -> HParams {
        let mut p = HParams::default();
        if tier > 7 {
            p.clu = Clu::Fastest;
            p.lz = Lz::None;
        } else if tier > 1 {
            p.clu = Clu::Fast;
        } else {
            p.clu = Clu::Best;
        }
        if tier > 1 {
            p.uint = UintM::None;
        }
        if tier >= 3 {
            p.strat = Strat::Approx;
        }
        p
    }

    /// HistogramParams::ForModular (lossless / modular streams; decoding speed tier 0)
    pub fn for_modular(tier: i32, modular: bool, extra_dc: bool) -> HParams {
        let mut p = HParams::default();
        if tier > 2 {
            p.clu = Clu::Fast;
            p.strat = if tier > 8 { Strat::Fast } else { Strat::Approx };
            p.lz = Lz::None;
            p.uint = if extra_dc || (modular && tier < 6) { UintM::Fast } else { UintM::None };
        } else if tier <= 1 {
            p.lz = Lz::Optimal;
        } else {
            p.lz = Lz::Lz77;
        }
        p
    }
}

/// FastClusterHistograms -> (clusters, symbol of every input)
fn fast_cluster(inp: &mut [Histo], max_h: usize) -> (Vec<Histo>, Vec<u32>) {
    let mut out: Vec<Histo> = Vec::with_capacity(max_h);
    let mut sym = vec![max_h as u32; inp.len()];
    let mut dists = vec![f32::MAX; inp.len()];
    let mut largest = 0usize;
    for i in 0..inp.len() {
        if inp[i].total == 0 {
            sym[i] = 0;
            dists[i] = 0.0;
            continue;
        }
        inp[i].shannon();
        if inp[i].total > inp[largest].total {
            largest = i;
        }
    }
    const MIN_DIST: f32 = 48.0;
    while out.len() < max_h {
        sym[largest] = out.len() as u32;
        out.push(inp[largest].clone());
        dists[largest] = 0.0;
        largest = 0;
        let last = out.len() - 1;
        for i in 0..inp.len() {
            if dists[i] == 0.0 {
                continue;
            }
            dists[i] = hdist(&inp[i], &out[last]).min(dists[i]);
            if dists[i] > dists[largest] {
                largest = i;
            }
        }
        if dists[largest] < MIN_DIST {
            break;
        }
    }
    for i in 0..inp.len() {
        if sym[i] != max_h as u32 {
            continue;
        }
        let mut best = 0usize;
        let mut bd = f32::MAX;
        for (j, o) in out.iter().enumerate() {
            let d = hdist(&inp[i], o);
            if d < bd {
                best = j;
                bd = d;
            }
        }
        out[best].add_h(&inp[i]);
        out[best].shannon();
        sym[i] = best as u32;
    }
    (out, sym)
}

#[derive(PartialEq)]
struct Pair {
    cost: f32,
    first: u32,
    second: u32,
    version: u32,
}

impl Eq for Pair {}

impl Ord for Pair {
    fn cmp(&self, o: &Self) -> Ordering {
        // BinaryHeap pops the greatest: make the smallest (cost, first, second, version) greatest
        o.cost
            .total_cmp(&self.cost)
            .then(o.first.cmp(&self.first))
            .then(o.second.cmp(&self.second))
            .then(o.version.cmp(&self.version))
    }
}

impl PartialOrd for Pair {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

fn merged_cost(a: &Histo, b: &Histo) -> f32 {
    let mut h = Histo::default();
    h.add_h(a);
    h.add_h(b);
    ans_pop_cost(&h.d)
}

/// ClusterHistograms -> (clusters, context map)
fn cluster(p: &HParams, inp: &mut [Histo], max_h: usize) -> (Vec<Histo>, Vec<u32>) {
    let mut max_h = max_h.min(p.max_hist).min(inp.len());
    if p.clu == Clu::Fastest {
        max_h = max_h.min(4);
    }
    let (mut out, mut syms) = fast_cluster(inp, max_h);
    if p.clu == Clu::Best {
        for h in out.iter_mut() {
            h.entropy = ans_pop_cost(&h.d);
        }
        let mut next_version = 2u32;
        let mut version = vec![1u32; out.len()];
        let mut renum: Vec<u32> = (0..out.len() as u32).collect();
        let mut heap = BinaryHeap::new();
        for i in 0..out.len() {
            for j in i + 1..out.len() {
                let c = merged_cost(&out[i], &out[j]) - out[i].entropy - out[j].entropy;
                if c >= 0.0 {
                    continue;
                }
                heap.push(Pair { cost: c, first: i as u32, second: j as u32, version: version[i].max(version[j]) });
            }
        }
        while let Some(t) = heap.pop() {
            let (f, s) = (t.first as usize, t.second as usize);
            if t.version != version[f].max(version[s]) || version[f] == 0 || version[s] == 0 {
                continue;
            }
            let sh = out[s].clone();
            out[f].add_h(&sh);
            out[f].entropy = ans_pop_cost(&out[f].d);
            for r in renum.iter_mut() {
                if *r == s as u32 {
                    *r = f as u32;
                }
            }
            version[s] = 0;
            version[f] = next_version;
            next_version += 1;
            for j in 0..out.len() {
                if j == f || version[j] == 0 {
                    continue;
                }
                let c = merged_cost(&out[f], &out[j]) - out[f].entropy - out[j].entropy;
                if c >= 0.0 {
                    continue;
                }
                heap.push(Pair { cost: c, first: f.min(j) as u32, second: f.max(j) as u32, version: version[f].max(version[j]) });
            }
        }
        let mut rev = vec![u32::MAX; out.len()];
        let mut alive = 0usize;
        for i in 0..out.len() {
            if version[i] == 0 {
                continue;
            }
            out[alive] = out[i].clone();
            rev[i] = alive as u32;
            alive += 1;
        }
        out.truncate(alive);
        for s in syms.iter_mut() {
            *s = rev[renum[*s as usize] as usize];
        }
    }
    // HistogramReindex: clusters numbered by first use
    let tmp = out.clone();
    let mut map = vec![u32::MAX; tmp.len()];
    let mut next = 0usize;
    for s in syms.iter() {
        if map[*s as usize] == u32::MAX {
            map[*s as usize] = next as u32;
            out[next] = tmp[*s as usize].clone();
            next += 1;
        }
    }
    out.truncate(next);
    for s in syms.iter_mut() {
        *s = map[*s as usize];
    }
    (out, syms)
}

// ------------------------------------------------------------------------- LZ77

/// LZ77Params
#[derive(Clone, Copy)]
pub struct Lz77 {
    pub enabled: bool,
    pub min_symbol: u32,
    pub min_length: u32,
    pub len_cfg: Huc,
    pub dist_ctx: u32,
}

impl Default for Lz77 {
    fn default() -> Self {
        Lz77 { enabled: false, min_symbol: 224, min_length: 3, len_cfg: Huc::new(0, 0, 0), dist_ctx: 0 }
    }
}

/// SymbolCostEstimator
struct Sce {
    maxa: usize,
    bits: Vec<f32>,
    add: Vec<f32>,
}

impl Sce {
    fn new(nctx: usize, force_huffman: bool, streams: &[Vec<Token>], lz: &Lz77) -> Sce {
        let mut hs = vec![Histo::default(); nctx];
        let uc = Huc::default();
        for s in streams {
            for t in s {
                let (mut tok, _, _) = if t.lz() { lz.len_cfg.encode(t.val) } else { uc.encode(t.val) };
                if t.lz() {
                    tok += lz.min_symbol;
                }
                hs[t.c()].add(tok as usize);
            }
        }
        let maxa = hs.iter().map(|h| h.d.len()).max().unwrap_or(0);
        let mut bits = vec![0.0f32; nctx * maxa];
        let mut add = vec![0.0f32; nctx];
        for i in 0..nctx {
            let h = &hs[i];
            let inv = 1.0f32 / (h.total as f32 + 1e-8);
            let mut tc = 0.0f32;
            for j in 0..h.d.len() {
                let cnt = h.d[j] as usize;
                let mut cost = 0.0f32;
                if cnt != 0 && cnt != h.total {
                    cost = -fast_log2f(cnt as f32 * inv);
                    if force_huffman {
                        cost = ceilf(cost);
                    }
                } else if cnt == 0 {
                    cost = ANS_LOG_TAB as f32;
                }
                bits[i * maxa + j] = cost;
                tc += cost * h.d[j] as f32;
            }
            add[i] = (6.0 - tc * inv).max(0.0);
        }
        Sce { maxa, bits, add }
    }
    #[inline]
    fn bits(&self, ctx: usize, sym: usize) -> f32 {
        if sym >= self.maxa {
            return ANS_LOG_TAB as f32;
        }
        self.bits[ctx * self.maxa + sym]
    }
}

fn special_distance(i: usize, mult: i64) -> i64 {
    let dx = tab::SPECIAL[i * 2] as i64;
    let dy = tab::SPECIAL[i * 2 + 1] as i64;
    let d = dy * mult + dx;
    if d < 1 { 1 } else { d }
}

/// ApplyLZ77_RLE
fn lz77_rle(p: &HParams, nctx: usize, streams: &[Vec<Token>], lz: &mut Lz77) -> Vec<Vec<Token>> {
    let sce = Sce::new(nctx, p.force_huffman, streams, lz);
    let mut decrease = 0.0f32;
    let mut total = 0usize;
    let uc = Huc::default();
    let mut out_all = Vec::with_capacity(streams.len());
    let mut cost = Vec::new();
    for (si, inp) in streams.iter().enumerate() {
        let dm = p.widths.get(si).copied().unwrap_or(0);
        total += inp.len();
        cost.clear();
        cost.resize(inp.len() + 1, 0.0f32);
        for i in 0..inp.len() {
            let (tok, nb, _) = uc.encode(inp[i].val);
            cost[i + 1] = sce.bits(inp[i].c(), tok as usize) + nb as f32 + cost[i];
        }
        let mut out = Vec::with_capacity(inp.len());
        let mut i = 0usize;
        while i < inp.len() {
            let dsym = if dm != 0 { 1 } else { 0 };
            let mut ncopy = 0usize;
            if i > 0 {
                while i + ncopy < inp.len() && inp[i + ncopy].val == inp[i - 1].val {
                    ncopy += 1;
                }
            }
            if ncopy == 0 {
                out.push(inp[i]);
                i += 1;
                continue;
            }
            let c = cost[i + ncopy] - cost[i];
            let lzlen = ncopy.wrapping_sub(lz.min_length as usize);
            let lzc = if ncopy >= lz.min_length as usize { (ceil_log2(lzlen as u32 + 1) + 1) as f32 } else { 0.0 };
            if ncopy < lz.min_length as usize || c <= lzc {
                out.extend_from_slice(&inp[i..i + ncopy]);
                i += ncopy;
                continue;
            }
            out.push(Token::new(inp[i].ctx | TOK_LZ, lzlen as u32));
            i += ncopy;
            decrease += c - lzc;
            out.push(Token::new(lz.dist_ctx, dsym));
        }
        out_all.push(out);
    }
    if decrease > total as f32 * 0.2 + 16.0 {
        lz.enabled = true;
    }
    out_all
}

/// HashChain of ApplyLZ77_LZ77
struct Chain<'a> {
    data: &'a [Token],
    size: usize,
    head: Vec<i32>,
    chain: Vec<u32>,
    val: Vec<i32>,
    headz: Vec<i32>,
    chainz: Vec<u32>,
    zeros: Vec<u32>,
    numzeros: u32,
    wmask: usize,
    wsize: usize,
    minl: usize,
    maxl: usize,
    special: Vec<(i64, u32)>, // (distance, code), sorted by distance
    nspecial: usize,
}

const HASH_N: usize = 32768;
const HASH_SHIFT: u32 = 5;
const MAX_CHAIN: u32 = 256;

impl<'a> Chain<'a> {
    fn new(data: &'a [Token], wsize: usize, minl: usize, maxl: usize, dm: usize) -> Chain<'a> {
        let mut special = Vec::new();
        let mut nspecial = 0;
        if dm != 0 {
            // count down: the smallest code wins for equal distances
            let mut m: Vec<(i64, u32)> = Vec::new();
            for i in (0..120).rev() {
                let d = special_distance(i, dm as i64);
                if let Some(e) = m.iter_mut().find(|e| e.0 == d) {
                    e.1 = i as u32;
                } else {
                    m.push((d, i as u32));
                }
            }
            m.sort_by(|a, b| a.0.cmp(&b.0));
            special = m;
            nspecial = 120;
        }
        Chain {
            data,
            size: data.len(),
            head: vec![-1; HASH_N],
            chain: (0..wsize as u32).collect(),
            val: vec![-1; wsize],
            headz: vec![-1; wsize + 1],
            chainz: (0..wsize as u32).collect(),
            zeros: vec![0; wsize],
            numzeros: 0,
            wmask: wsize - 1,
            wsize,
            minl,
            maxl,
            special,
            nspecial,
        }
    }
    #[inline]
    fn v(&self, i: usize) -> u32 {
        self.data[i].val
    }
    fn hash(&self, pos: usize) -> u32 {
        if pos + 2 < self.size {
            let r = self.v(pos) ^ (self.v(pos + 1) << HASH_SHIFT) ^ (self.v(pos + 2) << (HASH_SHIFT * 2));
            r & (HASH_N as u32 - 1)
        } else {
            0
        }
    }
    fn count_zeros(&self, pos: usize, prev: u32) -> u32 {
        let mut end = pos + self.wsize;
        if end > self.size {
            end = self.size;
        }
        if prev > 0 {
            if prev as usize >= self.wmask && self.v(end - 1) == 0 && end == pos + self.wsize {
                return prev;
            } else {
                return prev - 1;
            }
        }
        let mut n = 0u32;
        while pos + (n as usize) < end && self.v(pos + n as usize) == 0 {
            n += 1;
        }
        n
    }
    fn update(&mut self, pos: usize) {
        let hv = self.hash(pos);
        let wpos = pos & self.wmask;
        self.val[wpos] = hv as i32;
        if self.head[hv as usize] != -1 {
            self.chain[wpos] = self.head[hv as usize] as u32;
        }
        self.head[hv as usize] = wpos as i32;
        if pos > 0 && self.v(pos) != self.v(pos - 1) {
            self.numzeros = 0;
        }
        self.numzeros = self.count_zeros(pos, self.numzeros);
        self.zeros[wpos] = self.numzeros;
        let nz = self.numzeros as usize;
        if self.headz[nz] != -1 {
            self.chainz[wpos] = self.headz[nz] as u32;
        }
        self.headz[nz] = wpos as i32;
    }
    fn update_n(&mut self, pos: usize, len: usize) {
        for i in 0..len {
            self.update(pos + i);
        }
    }
    fn dist_symbol(&self, dist: i64) -> u32 {
        if let Ok(k) = self.special.binary_search_by(|e| e.0.cmp(&dist)) {
            return self.special[k].1;
        }
        (self.nspecial as i64 + dist - 1) as u32
    }
    /// FindMatch -> (dist symbol, len)
    fn find(&self, pos: usize) -> (u32, usize) {
        let mut rd = 0u32;
        let mut rl = 1usize;
        let wpos = pos & self.wmask;
        let hv = self.hash(pos);
        let mut hp = self.chain[wpos] as usize;
        let mut prev_dist: i64 = 0;
        let end = (pos + self.maxl).min(self.size);
        let mut cl = 0u32;
        let mut best_len = 0usize;
        loop {
            let dist = if hp <= wpos { (wpos - hp) as i64 } else { (wpos + self.wmask + 1 - hp) as i64 };
            if dist < prev_dist {
                break;
            }
            prev_dist = dist;
            let mut len = 0usize;
            if dist > 0 {
                let mut i = pos;
                let mut j = pos - dist as usize;
                if self.numzeros > 3 {
                    let mut r = ((self.numzeros - 1) as usize).min(self.zeros[hp] as usize);
                    if i + r >= end {
                        r = end - i - 1;
                    }
                    i += r;
                    j += r;
                }
                while i < end && self.v(i) == self.v(j) {
                    i += 1;
                    j += 1;
                }
                len = i - pos;
                if len >= self.minl && len + 2 >= best_len {
                    let ds = self.dist_symbol(dist);
                    if len > rl || (len == rl && rd > ds) {
                        rl = len;
                        rd = ds;
                    }
                    if len > best_len {
                        best_len = len;
                    }
                }
            }
            cl += 1;
            if cl >= MAX_CHAIN {
                break;
            }
            if self.numzeros >= 3 && len > self.numzeros as usize {
                if hp == self.chainz[hp] as usize {
                    break;
                }
                hp = self.chainz[hp] as usize;
                if self.zeros[hp] != self.numzeros {
                    break;
                }
            } else {
                if hp == self.chain[hp] as usize {
                    break;
                }
                hp = self.chain[hp] as usize;
                if self.val[hp] != hv as i32 {
                    break;
                }
            }
        }
        (rd, rl)
    }
}

fn lz_len_cost(len: u32) -> f32 {
    static T: [f32; 17] = [
        2.797_667_3, 3.213_177_7, 2.570_601, 2.408_392_5, 2.829_649_2, 3.392_308_7, 4.029_267_5, 4.415_577, 4.509_357_5,
        9.214_815, 10.020_59, 11.858_672, 12.458_533, 11.713_106, 12.561_996, 13.775_478, 13.174_027,
    ];
    let (tok, nb, _) = Huc::new(1, 0, 0).encode(len);
    T[(tok as usize).min(16)] + nb as f32
}

fn lz_dist_cost(d: u32) -> f32 {
    static T: [f32; 139] = [
        6.368282626312716, 5.680793277090298, 8.347404197105247, 7.641619201599141, 6.914328374119438, 7.959808291537444,
        8.70023120759855, 8.71378518934703, 9.379132523982769, 9.110472749092708, 9.159029569270908, 9.430936766731973,
        7.278284055315169, 7.8278514904267755, 10.026641158289236, 9.976049229827066, 9.64351607048908, 9.563403863480442,
        10.171474111762747, 10.45950155077234, 9.994813912104219, 10.322524683741156, 8.465808729388186, 8.756254166066853,
        10.160930174662234, 10.247329273413435, 10.04090403724809, 10.129398517544082, 9.342311691539546, 9.07608009102374,
        10.104799540677513, 10.378079384990906, 10.165828974075072, 10.337595322341553, 7.940557464567944, 10.575665823319431,
        11.023344321751955, 10.736144698831827, 11.118277044595054, 7.468468230648442, 10.738305230932939, 10.906980780216568,
        10.163468216353817, 10.17805759656433, 11.167283670483565, 11.147050200274544, 10.517921919244333, 10.651764778156886,
        10.17074446448919, 11.217636876224745, 11.261630721139484, 11.403140815247259, 10.892472096873417, 11.1859607804481,
        8.017346947551262, 7.895143720278828, 11.036577113822025, 11.170562110315794, 10.326988722591086, 10.40872184751056,
        11.213498225466386, 11.30580635516863, 10.672272515665442, 10.768069466228063, 11.145257364153565, 11.64668307145549,
        10.593156194627339, 11.207499484844943, 10.767517766396908, 10.826629811407042, 10.737764794499988, 10.6200448518045,
        10.191315385198092, 8.468384171390085, 11.731295299170432, 11.824619886654398, 10.41518844301179, 10.16310536548649,
        10.539423685097576, 10.495136599328031, 10.469112847728267, 11.72057686174922, 10.910326337834674, 11.378921834673758,
        11.847759036098536, 11.92071647623854, 10.810628276345282, 11.008601085273893, 11.910326337834674, 11.949212023423133,
        11.298614839104337, 11.611603659010392, 10.472930394619985, 11.835564720850282, 11.523267392285337, 12.01055816679611,
        8.413029688994023, 11.895784139536406, 11.984679534970505, 11.220654278717394, 11.716311684833672, 10.61036646226114,
        10.89849965960364, 10.203762898863669, 10.997560826267238, 11.484217379438984, 11.792836176993665, 12.24310468755171,
        11.464858097919262, 12.212747017409377, 11.425595666074955, 11.572048533398757, 12.742093965163013, 11.381874288645637,
        12.191870445817015, 11.683156920035426, 11.152442115262197, 11.90303691580457, 11.653292787169159, 11.938615382266098,
        16.970641701570223, 16.853602280380002, 17.26240782594733, 16.644655390108507, 17.14310889757499, 16.910935455445955,
        17.505678976959697, 17.213498225466388, 2.4162310293553024, 3.494587244462329, 3.5258600986408344, 3.4959806589517095,
        3.098390886949687, 3.343454654302911, 3.588847442290287, 4.14614790111827, 5.152948641990529, 7.433696808092598,
        9.716311684833672,
    ];
    let (tok, nb, _) = Huc::new(7, 0, 0).encode(d);
    T[(tok as usize).min(T.len() - 1)] + nb as f32
}

/// ApplyLZ77_LZ77 (greedy with one step of lazy matching)
fn lz77_lz77(p: &HParams, nctx: usize, streams: &[Vec<Token>], lz: &mut Lz77) -> Vec<Vec<Token>> {
    let sce = Sce::new(nctx, p.force_huffman, streams, lz);
    let (min_length, dist_ctx) = (lz.min_length, lz.dist_ctx);
    let total: usize = streams.iter().map(|s| s.len()).sum();
    // one stream: its tokens and the cost decreases of its matches (in order)
    let one = |si: usize| -> (Vec<Token>, Vec<f32>) {
        let inp = &streams[si];
        let dm = p.widths.get(si).copied().unwrap_or(0);
        let uc = Huc::default();
        let mut dec: Vec<f32> = Vec::new();
        let mut cost = vec![0.0f32; inp.len() + 1];
        for i in 0..inp.len() {
            let (tok, nb, _) = uc.encode(inp[i].val);
            cost[i + 1] = sce.bits(inp[i].c(), tok as usize) + nb as f32 + cost[i];
        }
        let mut out: Vec<Token> = Vec::with_capacity(inp.len());
        if inp.is_empty() {
            return (out, dec);
        }
        let minl = min_length as usize;
        let mut ws = 1usize;
        while ws < inp.len() && ws < WINDOW {
            ws <<= 1;
        }
        let mut ch = Chain::new(inp, ws, minl, inp.len(), dm);
        let mut updated = false;
        let mut i = 0usize;
        while i < inp.len() {
            out.push(inp[i]);
            if !updated {
                ch.update(i);
            }
            updated = false;
            let (mut ds, mut len) = ch.find(i);
            if len >= minl {
                if len < 256 && i + 1 < inp.len() {
                    ch.update(i + 1);
                    updated = true;
                    let (ds2, len2) = ch.find(i + 1);
                    if len2 > len {
                        i += 1;
                        updated = false;
                        len = len2;
                        ds = ds2;
                        out.push(inp[i]);
                    }
                }
                let c = cost[i + len] - cost[i];
                let lzlen = (len - minl) as u32;
                let lzc = lz_len_cost(lzlen) + lz_dist_cost(ds) + sce.add[out.last().map(|t| t.c()).unwrap_or(0)];
                if lzc <= c {
                    let last = out.len() - 1;
                    out[last].val = lzlen;
                    out[last].ctx |= TOK_LZ;
                    out.push(Token::new(dist_ctx, ds));
                    dec.push(c - lzc);
                } else {
                    out.extend_from_slice(&inp[i + 1..i + len]);
                }
                if updated {
                    ch.update_n(i + 2, len - 2);
                    updated = false;
                } else {
                    ch.update_n(i + 1, len - 1);
                }
                i += len;
            } else {
                i += 1;
            }
        }
        (out, dec)
    };
    // jxl-speed: the streams at the same time (each parses on its own)
    let res: Vec<(Vec<Token>, Vec<f32>)> =
        if p.par && streams.len() > 1 { crate::emod::par_map(streams.len(), true, one) } else { (0..streams.len()).map(one).collect() };
    let mut decrease = 0.0f32;
    let mut out_all = Vec::with_capacity(streams.len());
    for (out, dec) in res {
        for d in dec {
            decrease += d;
        }
        out_all.push(out);
    }
    if decrease > total as f32 * 0.2 + 16.0 {
        lz.enabled = true;
    }
    out_all
}

// ------------------------------------------------------------------------- the code

/// EntropyEncodingData + context map
#[derive(Clone, Default)]
pub struct Code {
    pub prefix: bool,
    pub lz: Lz77,
    pub cfg: Vec<Huc>,
    pub info: Vec<SymInfo>,
    pub ctxmap: Vec<u8>,
    pub loga: u32,
    pub cost: usize,
}

/// ChooseUintConfigs (clustered histograms rebuilt with the chosen configs); log alpha size
fn choose_configs(p: &HParams, streams: &[Vec<Token>], cmap: &[u8], hs: &mut [Histo], code: &mut Code, loga: &mut u32) {
    code.cfg = vec![Huc::default(); hs.len()];
    match p.uint {
        UintM::None => return,
        UintM::K000 => {
            code.cfg = vec![Huc::new(0, 0, 0); hs.len()];
            return;
        }
        UintM::CtxMap => {
            code.cfg = vec![Huc::new(2, 0, 1); hs.len()];
            return;
        }
        _ => {}
    }
    let configs: &[Huc] = if p.uint == UintM::Best {
        &[
            Huc::new(4, 2, 0),
            Huc::new(4, 1, 0),
            Huc::new(4, 2, 1),
            Huc::new(4, 2, 2),
            Huc::new(4, 1, 2),
            Huc::new(5, 2, 0),
            Huc::new(5, 1, 0),
            Huc::new(5, 2, 1),
            Huc::new(5, 2, 2),
            Huc::new(5, 1, 2),
            Huc::new(3, 2, 0),
            Huc::new(3, 1, 0),
            Huc::new(3, 2, 1),
            Huc::new(3, 1, 2),
            Huc::new(4, 1, 3),
            Huc::new(5, 1, 4),
            Huc::new(5, 2, 3),
            Huc::new(6, 1, 5),
            Huc::new(6, 2, 4),
            Huc::new(6, 0, 0),
            Huc::new(0, 0, 0),
            Huc::new(2, 0, 1),
            Huc::new(7, 0, 0),
            Huc::new(8, 0, 0),
            Huc::new(9, 0, 0),
            Huc::new(10, 0, 0),
            Huc::new(11, 0, 0),
            Huc::new(12, 0, 0),
        ]
    } else {
        &[Huc::new(4, 2, 0), Huc::new(4, 1, 2), Huc::new(0, 0, 0), Huc::new(2, 0, 1)]
    };
    let n = hs.len();
    let mut costs = vec![f32::MAX; n];
    let max_alpha = if code.prefix { PREFIX_MAX_ALPHA } else { ANS_MAX_ALPHA };
    let (lz_enabled, min_symbol, len_cfg) = (code.lz.enabled, code.lz.min_symbol, code.lz.len_cfg);
    // the cost of every cluster under configuration cfg (None: not usable)
    let cost_of = |cfg: Huc| -> Vec<Option<f32>> {
        let mut valid = vec![true; n];
        let mut extra = vec![0u32; n];
        let mut h: Vec<Histo> = vec![Histo::default(); n];
        for s in streams {
            for t in s {
                if t.lz() {
                    continue;
                }
                let hi = cmap[t.c()] as usize;
                let (tok, nb, _) = cfg.encode(t.val);
                if tok as usize >= max_alpha || (lz_enabled && tok >= min_symbol) {
                    valid[hi] = false;
                    continue;
                }
                extra[hi] += nb;
                h[hi].add(tok as usize);
            }
        }
        (0..n)
            .map(|i| {
                if !valid[i] {
                    return None;
                }
                let mut c = ans_pop_cost(&h[i].d) + extra[i] as f32;
                c += ceil_log2(cfg.se + 1) as f32;
                c += ceil_log2(cfg.se - cfg.msb + 1) as f32;
                Some(c)
            })
            .collect()
    };
    // jxl-speed: the configurations at the same time, chosen in their order
    let all: Vec<Vec<Option<f32>>> = if p.par && configs.len() > 1 {
        crate::emod::par_map(configs.len(), true, |k| cost_of(configs[k]))
    } else {
        configs.iter().map(|&cfg| cost_of(cfg)).collect()
    };
    for (k, &cfg) in configs.iter().enumerate() {
        for i in 0..n {
            if let Some(c) = all[k][i]
                && c < costs[i]
            {
                code.cfg[i] = cfg;
                costs[i] = c;
            }
        }
    }
    for h in hs.iter_mut() {
        h.clear();
    }
    let cfgs = &code.cfg;
    let final_of = |s: &[Token], hs: &mut [Histo]| -> u32 {
        let mut maxtok = 0u32;
        for t in s {
            let hi = cmap[t.c()] as usize;
            let (mut tok, _, _) = if t.lz() { len_cfg.encode(t.val) } else { cfgs[hi].encode(t.val) };
            if t.lz() {
                tok += min_symbol;
            }
            hs[hi].add(tok as usize);
            maxtok = maxtok.max(tok);
        }
        maxtok
    };
    let mut maxtok = 0u32;
    if p.par && streams.len() > 1 {
        let parts: Vec<(Vec<Histo>, u32)> = crate::emod::par_map(streams.len(), true, |k| {
            let mut h: Vec<Histo> = Vec::new();
            let mut m = 0;
            if !streams[k].is_empty() {
                h = vec![Histo::default(); n];
                m = final_of(&streams[k], &mut h);
            }
            (h, m)
        });
        for (part, m) in &parts {
            maxtok = maxtok.max(*m);
            for (a, b) in hs.iter_mut().zip(part.iter()) {
                if b.total != 0 || b.d.len() > a.d.len() {
                    a.add_h(b);
                }
            }
        }
    } else {
        for s in streams {
            maxtok = maxtok.max(final_of(s, hs));
        }
    }
    *loga = 4;
    while maxtok >= (1u32 << *loga) {
        *loga += 1;
    }
}

/// BuildAndEncodeHistograms: nctx contexts, streams (rewritten when LZ77 is chosen); with jw
/// the LZ77 params, context map and histograms are written
pub fn build(p: &HParams, nctx: usize, streams: &mut Vec<Vec<Token>>, mut jw: Option<&mut Jw>) -> Code {
    let mut code = Code::default();
    let mut nctx = nctx;
    code.lz.dist_ctx = nctx as u32;
    code.lz.enabled = false;
    code.lz.min_symbol = if p.force_huffman { 512 } else { 224 };
    let lzs = match p.lz {
        Lz::None => None,
        Lz::Rle => Some(lz77_rle(p, nctx, streams, &mut code.lz)),
        Lz::Lz77 | Lz::Optimal => Some(lz77_lz77(p, nctx, streams, &mut code.lz)),
    };
    let mut cost = 0usize;
    // LZ77Params bundle
    {
        let mut sw = SizeW(0);
        let s: &mut dyn FnMut(u32, u64) = &mut |n, v| match jw.as_deref_mut() {
            Some(w) => w.w(n, v),
            None => sw.w(n, v),
        };
        s(1, code.lz.enabled as u64);
        if code.lz.enabled {
            // min_symbol 224 / 512 (selector 0 / 1), min_length 3 (selector 0)
            s(2, if code.lz.min_symbol == 512 { 1 } else { 0 });
            s(2, 0);
            // length config, log alpha 8
            s(ceil_log2(9), code.lz.len_cfg.se as u64);
            if code.lz.len_cfg.se != 8 {
                s(ceil_log2(code.lz.len_cfg.se + 1), code.lz.len_cfg.msb as u64);
                s(ceil_log2(code.lz.len_cfg.se - code.lz.len_cfg.msb + 1), code.lz.len_cfg.lsb as u64);
            }
        }
        cost += sw.0;
    }
    if code.lz.enabled {
        nctx += 1;
        if let Some(l) = lzs {
            *streams = l;
        }
    }
    // histograms of the default config
    let mut uc = Huc::default();
    if p.uint == UintM::CtxMap {
        uc = Huc::new(2, 0, 1);
    }
    if p.uint == UintM::K000 {
        uc = Huc::new(0, 0, 0);
    }
    let mut hs = vec![Histo::default(); nctx];
    let mut total_tokens = 0usize;
    let len_cfg = code.lz.len_cfg;
    let min_symbol = code.lz.min_symbol;
    let histo_of = |s: &[Token], hs: &mut Vec<Histo>| {
        for t in s {
            let (mut tok, _, _) = if t.lz() { len_cfg.encode(t.val) } else { uc.encode(t.val) };
            if t.lz() {
                tok += min_symbol;
            }
            let c = if nctx == 1 { 0 } else { t.c() };
            hs[c].add(tok as usize);
        }
    };
    if p.par && streams.len() > 1 && nctx <= 4096 {
        // per stream, then the sums in stream order (jxl-speed)
        let parts: Vec<Vec<Histo>> = crate::emod::par_map(streams.len(), true, |k| {
            let mut h: Vec<Histo> = Vec::new();
            if !streams[k].is_empty() {
                h = vec![Histo::default(); nctx];
                histo_of(&streams[k], &mut h);
            }
            h
        });
        for part in &parts {
            for (a, b) in hs.iter_mut().zip(part.iter()) {
                if b.total != 0 || b.d.len() > a.d.len() {
                    a.add_h(b);
                }
            }
        }
        total_tokens = streams.iter().map(|s| s.len()).sum();
    } else {
        for s in streams.iter() {
            total_tokens += s.len();
            histo_of(s, &mut hs);
        }
    }
    let mut prefix = p.force_huffman || total_tokens < 100 || p.clu == Clu::Fastest;
    if !prefix && hs.iter_mut().all(|h| h.shannon() < 1e-5) {
        prefix = true;
    }
    code.prefix = prefix;
    // BuildAndStoreEntropyCodes
    let mut clustered: Vec<Histo>;
    code.ctxmap = vec![0u8; nctx];
    if nctx > 1 {
        let (cl, syms) = cluster(p, &mut hs, CLUSTERS_LIMIT);
        clustered = cl;
        for c in 0..nctx {
            code.ctxmap[c] = syms[c] as u8;
        }
        match jw.as_deref_mut() {
            Some(w) => encode_ctxmap(&code.ctxmap, clustered.len(), w),
            None => {
                let mut tw = Jw::new();
                encode_ctxmap(&code.ctxmap, clustered.len(), &mut tw);
                cost += tw.len() * 8;
            }
        }
    } else {
        clustered = vec![hs[0].clone()];
    }
    let mut loga = if code.lz.enabled { 8 } else { 7 };
    let cmap = code.ctxmap.clone();
    choose_configs(p, streams, &cmap, &mut clustered, &mut code, &mut loga);
    if loga < 5 {
        loga = 5;
    }
    cost += 1;
    match jw.as_deref_mut() {
        Some(w) => w.w(1, code.prefix as u64),
        None => {}
    }
    if code.prefix {
        loga = PREFIX_MAX_BITS;
    } else {
        cost += 2;
        if let Some(w) = jw.as_deref_mut() {
            w.w(2, (loga - 5) as u64);
        }
    }
    {
        let mut sw = SizeW(0);
        for c in &code.cfg {
            match jw.as_deref_mut() {
                Some(w) => c.write(w, loga),
                None => c.write(&mut sw, loga),
            }
        }
        if code.prefix {
            for h in &clustered {
                match jw.as_deref_mut() {
                    Some(w) => varlen16(w, (h.alphabet() - 1) as u32),
                    None => varlen16(&mut sw, (h.alphabet() - 1) as u32),
                }
            }
        }
        cost += sw.0;
    }
    code.loga = loga;
    for h in &clustered {
        let alpha = h.alphabet();
        let mut d = h.d.clone();
        d.resize(alpha, 0);
        if code.prefix {
            let mut sw = SizeW(0);
            let info = match jw.as_deref_mut() {
                Some(w) => {
                    let st = w.bits_written();
                    let i = build_prefix(&d, w);
                    sw.0 = w.bits_written() - st;
                    i
                }
                None => build_prefix(&d, &mut sw),
            };
            cost += sw.0;
            for (i, &c) in d.iter().enumerate() {
                cost += c.max(0) as usize * info.depth.get(i).copied().unwrap_or(0) as usize;
            }
            code.info.push(info);
        } else {
            let (info, c) = match jw.as_deref_mut() {
                Some(w) => build_ans(p.strat, &d, loga, Some(w)),
                None => build_ans::<SizeW>(p.strat, &d, loga, None),
            };
            cost += c;
            code.info.push(info);
        }
    }
    code.cost = cost;
    code
}

/// WriteTokens
pub fn write_tokens(tokens: &[Token], code: &Code, jw: &mut Jw) {
    if code.prefix {
        for t in tokens {
            let h = code.ctxmap.get(t.c()).copied().unwrap_or(0) as usize;
            let (mut tok, nb, bits) = if t.lz() { code.lz.len_cfg.encode(t.val) } else { code.cfg[h].encode(t.val) };
            if t.lz() {
                tok += code.lz.min_symbol;
            }
            let info = &code.info[h];
            let d = info.depth.get(tok as usize).copied().unwrap_or(0) as u32;
            let b = info.bits.get(tok as usize).copied().unwrap_or(0) as u64;
            jw.w(d, b);
            jw.w(nb, bits as u64);
        }
        return;
    }
    // ANS, backwards: per token (renormalization chunk, extra bits) in reverse
    let mut rec: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(tokens.len()); // (chunk flag | nb << 1, chunk, bits)
    let mut state: u32 = 0x13 << 16;
    for t in tokens.iter().rev() {
        let h = code.ctxmap.get(t.c()).copied().unwrap_or(0) as usize;
        let (mut tok, nb, bits) = if t.lz() { code.lz.len_cfg.encode(t.val) } else { code.cfg[h].encode(t.val) };
        if t.lz() {
            tok += code.lz.min_symbol;
        }
        let info = &code.info[h];
        let f = info.freq[tok as usize] as u32;
        let mut flag = 0u32;
        let mut chunk = 0u32;
        if (state >> (32 - ANS_LOG_TAB)) >= f {
            chunk = state & 0xFFFF;
            state >>= 16;
            flag = 1;
        }
        let q = state / f;
        let r = state % f;
        state = (q << ANS_LOG_TAB) + info.rev[(info.cum[tok as usize] as u32 + r) as usize & 4095] as u32;
        rec.push((flag, chunk, nb, bits));
    }
    jw.w(32, state as u64);
    for &(flag, chunk, nb, bits) in rec.iter().rev() {
        if flag != 0 {
            jw.w(16, chunk as u64);
        }
        jw.w(nb, bits as u64);
    }
}

/// MoveToFrontTransform
fn mtf(v: &[u8]) -> Vec<u8> {
    if v.is_empty() {
        return Vec::new();
    }
    let mx = *v.iter().max().unwrap_or(&0) as usize;
    let mut m: Vec<u8> = (0..=mx as u32).map(|i| i as u8).collect();
    let mut out = Vec::with_capacity(v.len());
    for &x in v {
        let idx = m.iter().position(|&e| e == x).unwrap_or(0);
        out.push(idx as u8);
        let val = m[idx];
        m.copy_within(0..idx, 1);
        m[0] = val;
    }
    out
}

/// EncodeContextMap
pub fn encode_ctxmap(map: &[u8], nhist: usize, jw: &mut Jw) {
    if nhist == 1 {
        jw.w(1, 1);
        jw.w(2, 0);
        return;
    }
    let tr = mtf(map);
    let p = HParams { uint: UintM::CtxMap, ..HParams::default() };
    let mut t0 = vec![map.iter().map(|&c| Token::new(0, c as u32)).collect::<Vec<_>>()];
    let mut t1 = vec![tr.iter().map(|&c| Token::new(0, c as u32)).collect::<Vec<_>>()];
    let c0 = build(&p, 1, &mut t0, None).cost;
    let c1 = build(&p, 1, &mut t1, None).cost;
    let use_mtf = c1 < c0;
    let bits = ceil_log2(nhist as u32);
    let simple = bits as usize * map.len();
    if bits < 4 && simple < c0 && simple < c1 {
        jw.w(1, 1);
        jw.w(2, bits as u64);
        for &e in map {
            jw.w(bits, e as u64);
        }
        return;
    }
    jw.w(1, 0);
    jw.w(1, use_mtf as u64);
    let mut toks = vec![(if use_mtf { &tr[..] } else { map }).iter().map(|&c| Token::new(0, c as u32)).collect::<Vec<_>>()];
    let code = build(&p, 1, &mut toks, Some(jw));
    write_tokens(&toks[0], &code, jw);
}
