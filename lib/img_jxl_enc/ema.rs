// ema -- MA tree learning of the new encoder (libjxl 0.11.1 modular/encoding/enc_ma.cc,
// enc_encoding.cc PredefinedTree / MakeFixedTree, enc_modular.cc MergeTrees)
//                                                                          [jxl-plus: no asm]
//
//   Node / Tree                PropertyDecisionNode: prop -1 = leaf; lchild is taken when the
//                              property > split
//   Samples                    TreeSamples: residual tokens per predictor, quantized
//                              properties, deduplicated samples with counts
//   Samples::prequantize(..)   PreQuantizeProperties (thresholds per property)
//   best_tree(samples, threshold, mul_info, range, fast_mult, threads) -> Tree   ComputeBestTree
//   Samples::merge_all(parts, threads)          the samples of all streams without duplicates
//
// jxl-speed: the tree depends only on which samples there are and how often (integer counts;
// a split is a partition), not on their order or grouping.  So the streams' samples are
// merged in parallel hash buckets, large nodes are counted and partitioned on the pool and
// their two parts searched at the same time, and the tree is numbered afterwards in the order
// of the serial search: the same tree for any number of threads.
//   tokenize(tree) -> (tokens, decoder tree)   TokenizeTree (BFS order, leaf ids)
//   fixed_tree(kind, pixels, bitdepth, prevprop) -> Tree   PredefinedTree
//   merge(trees, splits) -> Tree                MergeTrees (splits on the stream id)
//   Rng                        libjxl's Xorshift128+ (base/random.h)

use alloc::vec;
use alloc::vec::Vec;

use crate::eans::{pack_signed, Huc, Token};
use crate::emath::{fast_log2f, ln};

pub const NUM_STATIC: usize = 2;
pub const NUM_NONREF: usize = 16; // static + 13 + WP
pub const WP_PROP: usize = 15;
pub const GRAD_PROP: usize = 9;
pub const PRED_ZERO: u32 = 0;
pub const PRED_LEFT: u32 = 1;
pub const PRED_GRADIENT: u32 = 5;
pub const PRED_WEIGHTED: u32 = 6;
pub const NUM_PREDICTORS: usize = 14;
const PROP_RANGE: i32 = 511;
const DEDUP_UNUSED: u32 = u32::MAX;

/// a tree node (PropertyDecisionNode)
#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub prop: i32,
    pub split: i32,
    pub l: u32,
    pub r: u32,
    pub pred: u32,
    pub off: i64,
    pub mul: u32,
}

impl Node {
    pub fn leaf(pred: u32) -> Node {
        Node { prop: -1, split: 0, l: 0, r: 0, pred, off: 0, mul: 1 }
    }
    pub fn split(prop: i32, split: i32, l: u32) -> Node {
        Node { prop, split, l, r: l + 1, pred: 0, off: 0, mul: 1 }
    }
}

pub type Tree = Vec<Node>;

/// Xorshift128+ (Rng)
pub struct Rng {
    s: [u64; 2],
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng { s: [0x94D0_49BB_1331_11EB, 0xBF58_476D_1CE4_E5B9u64.wrapping_add(seed)] }
    }
    #[inline]
    pub fn next(&mut self) -> u64 {
        let mut s1 = self.s[0];
        let s0 = self.s[1];
        let bits = s1.wrapping_add(s0);
        self.s[0] = s0;
        s1 ^= s1 << 23;
        s1 ^= s0 ^ (s1 >> 18) ^ (s0 >> 5);
        self.s[1] = s1;
        bits
    }
    pub fn uniform_u(&mut self, begin: u64, end: u64) -> u64 {
        self.next() % (end - begin) + begin
    }
    pub fn uniform_f(&mut self, begin: f32, end: f32) -> f32 {
        let u = ((self.next() >> (64 - 23)) as u32) | 0x3F80_0000;
        let f = f32::from_bits(u);
        (end - begin) * (f - 1.0) + begin
    }
    /// Geometric(dist) with dist = 1 / ln(1 - p)
    pub fn geometric(&mut self, dist: f32) -> u32 {
        let f = self.uniform_f(0.0, 1.0);
        let l = ln(1.0 - f) * dist;
        if l > 0.0 { l as u32 } else { 0 }
    }
}

/// a multiplier range (ModularMultiplierInfo): [channel range, stream range], multiplier
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MulInfo {
    pub range: [[u32; 2]; 2],
    pub mul: u32,
}

/// TreeSamples
pub struct Samples {
    pub preds: Vec<u32>,
    pub props_used: Vec<u32>,
    res: Vec<Vec<(u8, u8)>>, // per predictor: (token, nbits)
    counts: Vec<u16>,
    props: Vec<Vec<u8>>,
    compact: Vec<Vec<i32>>,
    mapping: Vec<Vec<u8>>,
    pub num_samples: usize,
    dedup: Vec<u32>,
}

impl Samples {
    /// SetPredictor + SetProperties: predictor 0..13 = that one, 14 = Best (WP + gradient),
    /// 15 = Variable (all); wp_mode: 0 default, 1 WP only, 2 no WP, 3 gradient only
    pub fn new(predictor: u32, props: &[u32], wp_mode: u32) -> Samples {
        let mut preds: Vec<u32> = if wp_mode == 1 {
            vec![PRED_WEIGHTED]
        } else if predictor == 15 {
            let mut v: Vec<u32> = (0..NUM_PREDICTORS as u32).collect();
            v.swap(0, PRED_WEIGHTED as usize);
            v.swap(1, PRED_GRADIENT as usize);
            v
        } else if predictor == 14 {
            vec![PRED_WEIGHTED, PRED_GRADIENT]
        } else {
            vec![predictor]
        };
        if wp_mode == 2 {
            preds.retain(|&p| p != PRED_WEIGHTED);
        }
        let mut pu: Vec<u32> = props.to_vec();
        if wp_mode == 1 {
            pu = vec![WP_PROP as u32];
        }
        if wp_mode == 3 {
            pu = vec![GRAD_PROP as u32];
        }
        if wp_mode == 2 {
            pu.retain(|&p| p != WP_PROP as u32);
        }
        let np = preds.len();
        let nq = pu.len();
        Samples {
            preds,
            props_used: pu,
            res: vec![Vec::new(); np],
            counts: Vec::new(),
            props: vec![Vec::new(); nq],
            compact: Vec::new(),
            mapping: Vec::new(),
            num_samples: 0,
            dedup: Vec::new(),
        }
    }

    pub fn has_samples(&self) -> bool {
        !self.res.is_empty() && !self.res[0].is_empty()
    }
    pub fn distinct(&self) -> usize {
        self.counts.len()
    }
    pub fn uses_wp(&self) -> bool {
        self.preds.contains(&PRED_WEIGHTED) || self.props_used.contains(&(WP_PROP as u32))
    }
    /// the largest property index used (for the reference properties)
    pub fn max_prop(&self) -> u32 {
        self.props_used.iter().copied().max().unwrap_or(0)
    }

    fn init_table(&mut self, log: u32) {
        let size = 1usize << log;
        if self.dedup.len() == size {
            return;
        }
        self.dedup = vec![DEDUP_UNUSED; size];
        for i in 0..self.distinct() {
            if self.counts[i] != u16::MAX {
                self.add_to_table(i);
            }
        }
    }

    /// PrepareForSamples
    pub fn prepare(&mut self, n: usize) {
        for r in self.res.iter_mut() {
            r.reserve(n);
        }
        for p in self.props.iter_mut() {
            p.reserve(n);
        }
        let total = n + self.counts.len();
        let log = crate::eans::ceil_log2((total * 3 / 2).max(1) as u32);
        self.init_table(log);
    }

    fn hash1(&self, a: usize) -> usize {
        const C: u64 = 0x1e35a7bd;
        let mut h = C;
        for r in &self.res {
            h = h.wrapping_mul(C).wrapping_add(r[a].0 as u64);
            h = h.wrapping_mul(C).wrapping_add(r[a].1 as u64);
        }
        for p in &self.props {
            h = h.wrapping_mul(C).wrapping_add(p[a] as u64);
        }
        ((h >> 16) as usize) & (self.dedup.len() - 1)
    }
    fn hash2(&self, a: usize) -> usize {
        const C: u64 = 0x1e35a7bd1e35a7bd;
        let mut h = C;
        for p in &self.props {
            h = h.wrapping_mul(C) ^ p[a] as u64;
        }
        for r in &self.res {
            h = h.wrapping_mul(C) ^ r[a].0 as u64;
            h = h.wrapping_mul(C) ^ r[a].1 as u64;
        }
        ((h >> 16) as usize) & (self.dedup.len() - 1)
    }
    fn same(&self, a: usize, b: usize) -> bool {
        for r in &self.res {
            if r[a] != r[b] {
                return false;
            }
        }
        for p in &self.props {
            if p[a] != p[b] {
                return false;
            }
        }
        true
    }
    fn add_to_table(&mut self, a: usize) {
        let p1 = self.hash1(a);
        let p2 = self.hash2(a);
        if self.dedup[p1] == DEDUP_UNUSED {
            self.dedup[p1] = a as u32;
        } else if self.dedup[p2] == DEDUP_UNUSED {
            self.dedup[p2] = a as u32;
        }
    }
    fn add_merge(&mut self, a: usize) -> bool {
        let p1 = self.hash1(a);
        let p2 = self.hash2(a);
        for p in [p1, p2] {
            let e = self.dedup[p];
            if e != DEDUP_UNUSED && self.same(a, e as usize) {
                self.counts[e as usize] += 1;
                if self.counts[e as usize] == u16::MAX {
                    self.dedup[p] = DEDUP_UNUSED;
                }
                return true;
            }
        }
        self.add_to_table(a);
        false
    }

    #[inline]
    pub fn quantize(&self, i: usize, v: i32) -> u8 {
        let v = v.clamp(-PROP_RANGE, PROP_RANGE) + PROP_RANGE;
        self.mapping[i][v as usize]
    }

    /// AddSample: pixel, all properties, the predictions of every predictor (index =
    /// predictor id)
    pub fn add(&mut self, pixel: i64, props: &[i32], predictions: &[i64; NUM_PREDICTORS]) {
        let cfg = Huc::new(4, 1, 2);
        for i in 0..self.preds.len() {
            let v = pixel.wrapping_sub(predictions[self.preds[i] as usize]) as i32;
            let (tok, nb, _) = cfg.encode(pack_signed(v));
            self.res[i].push((tok as u8, nb as u8));
        }
        for i in 0..self.props_used.len() {
            let q = self.quantize(i, props[self.props_used[i] as usize]);
            self.props[i].push(q);
        }
        self.counts.push(1);
        self.num_samples += 1;
        let last = self.counts.len() - 1;
        if self.add_merge(last) {
            for r in self.res.iter_mut() {
                r.pop();
            }
            for p in self.props.iter_mut() {
                p.pop();
            }
            self.counts.pop();
        }
    }

    /// an empty sample set with the same predictors, properties and quantization
    pub fn empty_like(&self) -> Samples {
        Samples {
            preds: self.preds.clone(),
            props_used: self.props_used.clone(),
            res: vec![Vec::new(); self.preds.len()],
            counts: Vec::new(),
            props: vec![Vec::new(); self.props_used.len()],
            compact: self.compact.clone(),
            mapping: self.mapping.clone(),
            num_samples: 0,
            dedup: Vec::new(),
        }
    }

    /// a 64-bit hash of sample i (its tokens and quantized properties)
    fn hash64(&self, i: usize) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for r in &self.res {
            h = (h ^ r[i].0 as u64).wrapping_mul(0x100_0000_01b3);
            h = (h ^ r[i].1 as u64).wrapping_mul(0x100_0000_01b3);
        }
        for p in &self.props {
            h = (h ^ p[i] as u64).wrapping_mul(0x100_0000_01b3);
        }
        h ^ (h >> 29)
    }

    /// sample i of o equals sample e of self
    fn same_as(&self, e: usize, o: &Samples, i: usize) -> bool {
        for (r, orr) in self.res.iter().zip(o.res.iter()) {
            if r[e] != orr[i] {
                return false;
            }
        }
        for (p, op) in self.props.iter().zip(o.props.iter()) {
            if p[e] != op[i] {
                return false;
            }
        }
        true
    }

    /// sample i of o appended with count c
    fn push_from(&mut self, o: &Samples, i: usize, c: u16) {
        for (r, orr) in self.res.iter_mut().zip(o.res.iter()) {
            r.push(orr[i]);
        }
        for (p, op) in self.props.iter_mut().zip(o.props.iter()) {
            p.push(op[i]);
        }
        self.counts.push(c);
        self.num_samples += c as usize;
    }

    /// the samples of every part (same quantization) without duplicates: equal samples of all
    /// parts become one with the sum of their counts (split at the u16 limit), grouped by
    /// hash buckets that are merged at the same time.  The tree search depends only on which
    /// samples there are and how often (its sums are integer counts, its splits partitions
    /// of the samples), not on their order or on how equal samples are grouped, so this
    /// gives the tree the serial table merge of merge_from gives  [jxl-speed]
    pub fn merge_all(&mut self, parts: &[Samples], threads: bool) {
        const LOG_NB: u32 = 6;
        const NB: usize = 1 << LOG_NB;
        // per part: the hash of every sample and the samples of every bucket
        let idx: Vec<(Vec<u64>, Vec<Vec<u32>>)> = crate::emod::par_map(parts.len(), threads, |k| {
            let pt = &parts[k];
            let hs: Vec<u64> = (0..pt.distinct()).map(|i| pt.hash64(i)).collect();
            let mut lists: Vec<Vec<u32>> = vec![Vec::new(); NB];
            for (i, &h) in hs.iter().enumerate() {
                lists[(h >> (64 - LOG_NB)) as usize].push(i as u32);
            }
            (hs, lists)
        });
        let proto = &*self;
        let buckets: Vec<Samples> = crate::emod::par_map(NB, threads, |b| {
            let n: usize = idx.iter().map(|x| x.1[b].len()).sum();
            let mut out = proto.empty_like();
            for r in out.res.iter_mut() {
                r.reserve(n);
            }
            for p in out.props.iter_mut() {
                p.reserve(n);
            }
            out.counts.reserve(n);
            // open addressing on the hash: (hash, the newest entry of the sample)
            let size = (2 * n).next_power_of_two().max(16);
            let mask = size - 1;
            let mut table: Vec<(u64, u32)> = vec![(0, u32::MAX); size];
            for (k, (hs, lists)) in idx.iter().enumerate() {
                let pt = &parts[k];
                for &i in &lists[b] {
                    let i = i as usize;
                    let h = hs[i];
                    let c = pt.counts[i];
                    let mut slot = (h as usize) & mask;
                    loop {
                        let (th, e) = table[slot];
                        if e == u32::MAX {
                            table[slot] = (h, out.counts.len() as u32);
                            out.push_from(pt, i, c);
                            break;
                        }
                        if th == h && out.same_as(e as usize, pt, i) {
                            let sum = out.counts[e as usize] as u32 + c as u32;
                            if sum < u16::MAX as u32 {
                                out.counts[e as usize] = sum as u16;
                                out.num_samples += c as usize;
                            } else {
                                table[slot] = (h, out.counts.len() as u32);
                                out.push_from(pt, i, c);
                            }
                            break;
                        }
                        slot = (slot + 1) & mask;
                    }
                }
            }
            out
        });
        let total: usize = buckets.iter().map(|b| b.distinct()).sum();
        for bk in &buckets {
            self.num_samples += bk.num_samples;
        }
        if !threads || self.distinct() != 0 {
            for r in self.res.iter_mut() {
                r.reserve(total);
            }
            for p in self.props.iter_mut() {
                p.reserve(total);
            }
            self.counts.reserve(total);
            for bk in &buckets {
                for (r, br) in self.res.iter_mut().zip(bk.res.iter()) {
                    r.extend_from_slice(br);
                }
                for (p, bp) in self.props.iter_mut().zip(bk.props.iter()) {
                    p.extend_from_slice(bp);
                }
                self.counts.extend_from_slice(&bk.counts);
            }
            return;
        }
        // the buckets copied into their ranges of the (zeroed, fresh) columns on the pool
        for r in self.res.iter_mut() {
            *r = vec![(0u8, 0u8); total];
        }
        for p in self.props.iter_mut() {
            *p = vec![0u8; total];
        }
        self.counts = vec![0u16; total];
        enum Dst<'b> {
            R(&'b mut [(u8, u8)], usize),
            P(&'b mut [u8], usize),
            C(&'b mut [u16]),
        }
        let mut per: Vec<Vec<Dst>> = (0..NB).map(|_| Vec::new()).collect();
        for (ri, r) in self.res.iter_mut().enumerate() {
            let mut rest = &mut r[..];
            for (b, bk) in buckets.iter().enumerate() {
                let (a, z) = rest.split_at_mut(bk.distinct());
                per[b].push(Dst::R(a, ri));
                rest = z;
            }
        }
        for (pi, pc) in self.props.iter_mut().enumerate() {
            let mut rest = &mut pc[..];
            for (b, bk) in buckets.iter().enumerate() {
                let (a, z) = rest.split_at_mut(bk.distinct());
                per[b].push(Dst::P(a, pi));
                rest = z;
            }
        }
        {
            let mut rest = &mut self.counts[..];
            for (b, bk) in buckets.iter().enumerate() {
                let (a, z) = rest.split_at_mut(bk.distinct());
                per[b].push(Dst::C(a));
                rest = z;
            }
        }
        let cells: Vec<rt::sync::SpinLock<Vec<Dst>>> = per.into_iter().map(rt::sync::SpinLock::new).collect();
        let bks = &buckets;
        crate::emod::par_map(NB, true, |b| {
            let mut g = cells[b].lock();
            let bk = &bks[b];
            for d in g.iter_mut() {
                match d {
                    Dst::R(a, ri) => a.copy_from_slice(&bk.res[*ri]),
                    Dst::P(a, pi) => a.copy_from_slice(&bk.props[*pi]),
                    Dst::C(a) => a.copy_from_slice(&bk.counts),
                }
            }
        });
    }

    pub fn done(&mut self) {
        self.dedup = Vec::new();
    }

    /// PreQuantizeProperties
    #[allow(clippy::too_many_arguments)]
    pub fn prequantize(
        &mut self,
        range: &[[u32; 2]; 2],
        mul_info: &[MulInfo],
        group_px: &[u32],
        chan_px: &[u32],
        pixel_samples: &mut Vec<i32>,
        diff_samples: &mut Vec<i32>,
        max_vals: usize,
    ) {
        let mut gthr: Vec<i32> = Vec::new();
        let mut cthr: Vec<i32> = Vec::new();
        for v in mul_info {
            if v.range[0][0] != range[0][0] {
                cthr.push(v.range[0][0] as i32 - 1);
            }
            if v.range[0][1] != range[0][1] {
                cthr.push(v.range[0][1] as i32 - 1);
            }
            if v.range[1][0] != range[1][0] {
                gthr.push(v.range[1][0] as i32 - 1);
            }
            if v.range[1][1] != range[1][1] {
                gthr.push(v.range[1][1] as i32 - 1);
            }
        }
        cthr.sort_unstable();
        cthr.dedup();
        gthr.sort_unstable();
        gthr.dedup();
        let mut pix_thr: Option<Vec<i32>> = None;
        let mut abs_pix_thr: Option<Vec<i32>> = None;
        let mut diff_thr: Option<Vec<i32>> = None;
        let mut abs_diff_thr: Option<Vec<i32>> = None;
        self.compact = vec![Vec::new(); self.props_used.len()];
        self.mapping = vec![Vec::new(); self.props_used.len()];
        for i in 0..self.props_used.len() {
            let p = self.props_used[i] as usize;
            let c: Vec<i32> = if p == 0 {
                if !cthr.is_empty() { cthr.clone() } else { quantize_histogram(chan_px, max_vals) }
            } else if p == 1 {
                if !gthr.is_empty() { gthr.clone() } else { quantize_histogram(group_px, max_vals) }
            } else if p == 2 || p == 3 {
                (0..max_vals.saturating_sub(1)).map(|k| ((k + 1) * 256 / max_vals) as i32 - 1).collect()
            } else if p == 6 || p == 7 || p == 8 || (p >= NUM_NONREF && (p - NUM_NONREF) % 4 == 1) {
                if pix_thr.is_none() {
                    pix_thr = Some(quantize_samples(pixel_samples, max_vals));
                }
                pix_thr.clone().unwrap_or_default()
            } else if p == 4 || p == 5 || (p >= NUM_NONREF && (p - NUM_NONREF) % 4 == 0) {
                if abs_pix_thr.is_none() {
                    if pix_thr.is_none() {
                        pix_thr = Some(quantize_samples(pixel_samples, max_vals));
                    }
                    for v in pixel_samples.iter_mut() {
                        *v = v.wrapping_abs();
                    }
                    abs_pix_thr = Some(quantize_samples(pixel_samples, max_vals));
                }
                abs_pix_thr.clone().unwrap_or_default()
            } else if p >= NUM_NONREF && (p - NUM_NONREF) % 4 == 2 {
                if abs_diff_thr.is_none() {
                    if diff_thr.is_none() {
                        diff_thr = Some(quantize_samples(diff_samples, max_vals));
                    }
                    for v in diff_samples.iter_mut() {
                        *v = v.wrapping_abs();
                    }
                    abs_diff_thr = Some(quantize_samples(diff_samples, max_vals));
                }
                abs_diff_thr.clone().unwrap_or_default()
            } else if p == WP_PROP {
                wp_thresholds(max_vals)
            } else {
                if diff_thr.is_none() {
                    diff_thr = Some(quantize_samples(diff_samples, max_vals));
                }
                diff_thr.clone().unwrap_or_default()
            };
            let mut map = vec![0u8; (PROP_RANGE * 2 + 1) as usize];
            let mut mapped = 0usize;
            for (j, m) in map.iter_mut().enumerate() {
                while mapped < c.len() && j as i32 - PROP_RANGE > c[mapped] {
                    mapped += 1;
                }
                *m = mapped as u8;
            }
            self.compact[i] = c;
            self.mapping[i] = map;
        }
    }

}

fn wp_thresholds(max_vals: usize) -> Vec<i32> {
    if max_vals < 32 {
        return vec![-127, -63, -31, -15, -7, -3, -1, 0, 1, 3, 7, 15, 31, 63, 127];
    }
    if max_vals < 64 {
        return vec![
            -255, -191, -127, -95, -63, -47, -31, -23, -15, -11, -7, -5, -3, -1, 0, 1, 3, 5, 7, 11, 15, 23, 31, 47, 63, 95,
            127, 191, 255,
        ];
    }
    vec![
        -255, -223, -191, -159, -127, -111, -95, -79, -63, -55, -47, -39, -31, -27, -23, -19, -15, -13, -11, -9, -7, -6,
        -5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 9, 11, 13, 15, 19, 23, 27, 31, 39, 47, 55, 63, 79, 95, 111, 127, 159,
        191, 223, 255,
    ]
}

/// QuantizeHistogram
fn quantize_histogram(h: &[u32], chunks: usize) -> Vec<i32> {
    if h.is_empty() {
        return Vec::new();
    }
    let mut thr = Vec::new();
    let sum: u64 = h.iter().map(|&v| v as u64).sum();
    let mut cum = 0u64;
    let mut t = 1u64;
    for i in 0..h.len() - 1 {
        cum += h[i] as u64;
        if cum >= t * sum / chunks as u64 {
            thr.push(i as i32);
            while cum > t * sum / chunks as u64 {
                t += 1;
            }
        }
    }
    thr
}

/// QuantizeSamples
fn quantize_samples(s: &[i32], chunks: usize) -> Vec<i32> {
    if s.is_empty() {
        return Vec::new();
    }
    const R: i32 = 512;
    let mut mn = *s.iter().min().unwrap_or(&0);
    mn = mn.clamp(-R, R);
    let mut counts = vec![0u32; (2 * R + 1) as usize];
    for &v in s {
        let off = v.clamp(-R, R) - mn;
        if off >= 0 && (off as usize) < counts.len() {
            counts[off as usize] += 1;
        }
    }
    let mut t = quantize_histogram(&counts, chunks);
    for v in t.iter_mut() {
        *v += mn;
    }
    t
}

/// EstimateBits: entropy with the minimum ANS probability.  jxl-speed: the terms
/// c log2(max(c / total, 1 / 4096)) of 8 symbols at a time (the scalar formula per lane, as
/// emath::fast_log2f), then subtracted one by one in symbol order as before
fn estimate_bits(counts: &[i32]) -> f32 {
    let total: i32 = counts.iter().sum();
    if total == 0 {
        return 0.0;
    }
    let mut terms = [0f32; 64];
    let n8 = (counts.len() & !7).min(64);
    if n8 > 0 {
        simd::dispatch(TermsK { counts: &counts[..n8], inv: 1.0f32 / total as f32, out: &mut terms[..n8] });
    }
    let minp = 1.0f32 / 4096.0;
    let inv = 1.0f32 / total as f32;
    let mut b = 0.0f32;
    for (k, &c) in counts.iter().enumerate() {
        if c == 0 || c == total {
            continue;
        }
        let t = if k < n8 {
            terms[k]
        } else {
            let p = (c as f32 * inv).max(minp);
            c as f32 * fast_log2f(p)
        };
        b -= t;
    }
    b
}

/// the terms of estimate_bits for whole groups of 8 symbols
struct TermsK<'a> {
    counts: &'a [i32],
    inv: f32,
    out: &'a mut [f32],
}

impl simd::Kernel for TermsK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: simd::Simd>(self, s: S) {
        let l = S::BYTES / 4;
        let fp = &crate::tab::FASTPOW;
        let (p0, p1, p2, p3, p4, p5) = (s.splatf(fp[0]), s.splatf(fp[1]), s.splatf(fp[2]), s.splatf(fp[3]), s.splatf(fp[4]), s.splatf(fp[5]));
        let inv = s.splatf(self.inv);
        let minp = s.splatf(1.0f32 / 4096.0);
        let one = s.splatf(1.0);
        let off = s.splat32(0x3f2a_aaab);
        let mut k = 0;
        while k + l <= self.counts.len() {
            let c = s.cvt_i32_f32(s.load_i32(&self.counts[k..]));
            // max(c inv, minp): Rust's max (no NaN here; equal values are the same bits)
            let x = s.maxf(s.mulf(c, inv), minp);
            // fast_log2f
            let bits = x;
            let ex = s.sra32::<23>(s.sub32(bits, off));
            let m = s.subf(s.sub32(bits, s.sll32::<23>(ex)), one);
            let mut yp = s.addf(s.mulf(p2, m), p1);
            yp = s.addf(s.mulf(yp, m), p0);
            let mut yq = s.addf(s.mulf(p5, m), p4);
            yq = s.addf(s.mulf(yq, m), p3);
            let lg = s.addf(s.divf(yp, yq), s.cvt_i32_f32(ex));
            s.store_f32(&mut self.out[k..], s.mulf(c, lg));
            k += l;
        }
    }
}

#[derive(Clone, Copy)]
struct Split {
    prop: usize,
    val: usize,
    pos: usize,
    lcost: f32,
    rcost: f32,
    lpred: u32,
    rpred: u32,
}

impl Default for Split {
    fn default() -> Self {
        Split { prop: 0, val: 0, pos: 0, lcost: f32::MAX, rcost: f32::MAX, lpred: 0, rpred: 0 }
    }
}

impl Split {
    fn cost(&self) -> f32 {
        self.lcost + self.rcost
    }
}

#[derive(Clone, Copy)]
struct CostInfo {
    cost: f32,
    extra: f32,
    pred: u32,
}

impl CostInfo {
    fn total(&self) -> f32 {
        self.cost + self.extra
    }
}

/// SplitTreeSamples: partial quickselect so that [begin, pos) has property <= the pivot
/// (indices relative to the view: the pivots are uniform_u(0, n) + begin as before)
fn split_samples(s: &mut View, begin: usize, pos: usize, end: usize, prop: usize) {
    let mut rng = Rng::new(0);
    let (mut begin, mut end) = (begin, end);
    while end > begin + 1 {
        let pivot = rng.uniform_u(begin as u64, end as u64) as usize;
        s.swap(begin, pivot);
        let mut pb = begin;
        let mut pe = pb + 1;
        for i in begin + 1..end {
            let c = s.prop(prop, i) as i32 - s.prop(prop, pb) as i32;
            if c < 0 {
                s.three_shuffle(pb, pe, i);
                pb += 1;
                pe += 1;
            } else if c == 0 {
                s.swap(pe, i);
                pe += 1;
            }
        }
        if pb > pos {
            end = pb;
        } else if pe < pos {
            begin = pe;
        } else {
            break;
        }
    }
}

/// BoxIntersects
fn box_intersects(needle: &[[u32; 2]; 2], hay: &[[u32; 2]; 2], axis: &mut usize, val: &mut u32) -> u32 {
    let mut partial = false;
    for i in 0..NUM_STATIC {
        if hay[i][0] >= needle[i][1] || hay[i][1] <= needle[i][0] {
            return 0;
        }
        if hay[i][0] <= needle[i][0] && hay[i][1] >= needle[i][1] {
            continue;
        }
        partial = true;
        *axis = i;
        if hay[i][0] > needle[i][0] && hay[i][0] < needle[i][1] {
            *val = hay[i][0] - 1;
        } else {
            *val = hay[i][1] - 1;
        }
    }
    if partial { 1 } else { 2 }
}

/// the sample columns of one node's range (TreeSamples between begin and end, indices from
/// 0): disjoint ranges can be searched on different threads  [jxl-speed]
struct View<'a> {
    res: Vec<&'a mut [(u8, u8)]>,
    props: Vec<&'a mut [u8]>,
    counts: &'a mut [u16],
}

impl<'a> View<'a> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.counts.len()
    }
    #[inline(always)]
    fn tok(&self, pred: usize, i: usize) -> usize {
        self.res[pred][i].0 as usize
    }
    #[inline(always)]
    fn nbits(&self, pred: usize, i: usize) -> usize {
        self.res[pred][i].1 as usize
    }
    #[inline(always)]
    fn prop(&self, p: usize, i: usize) -> usize {
        self.props[p][i] as usize
    }
    fn swap(&mut self, a: usize, b: usize) {
        if a == b {
            return;
        }
        for r in self.res.iter_mut() {
            r.swap(a, b);
        }
        for p in self.props.iter_mut() {
            p.swap(a, b);
        }
        self.counts.swap(a, b);
    }
    fn three_shuffle(&mut self, a: usize, b: usize, c: usize) {
        if b == c {
            self.swap(a, b);
            return;
        }
        for r in self.res.iter_mut() {
            let t = r[a];
            r[a] = r[c];
            r[c] = r[b];
            r[b] = t;
        }
        for p in self.props.iter_mut() {
            let t = p[a];
            p[a] = p[c];
            p[c] = p[b];
            p[b] = t;
        }
        let t = self.counts[a];
        self.counts[a] = self.counts[c];
        self.counts[c] = self.counts[b];
        self.counts[b] = t;
    }
    /// ([0, mid), [mid, len))
    fn split_at(self, mid: usize) -> (View<'a>, View<'a>) {
        let mut lr = Vec::with_capacity(self.res.len());
        let mut hr = Vec::with_capacity(self.res.len());
        for r in self.res {
            let (a, b) = r.split_at_mut(mid);
            lr.push(a);
            hr.push(b);
        }
        let mut lp = Vec::with_capacity(self.props.len());
        let mut hp = Vec::with_capacity(self.props.len());
        for p in self.props {
            let (a, b) = p.split_at_mut(mid);
            lp.push(a);
            hp.push(b);
        }
        let (lc, hc) = self.counts.split_at_mut(mid);
        (View { res: lr, props: lp, counts: lc }, View { res: hr, props: hp, counts: hc })
    }
}

/// stable partition of c: the entries where keep is set first
fn stable_part<T: Copy>(c: &mut [T], keep: &[bool]) {
    let mut tmp: Vec<T> = Vec::with_capacity(c.len());
    for (x, &k) in c.iter().zip(keep) {
        if k {
            tmp.push(*x);
        }
    }
    for (x, &k) in c.iter().zip(keep) {
        if !k {
            tmp.push(*x);
        }
    }
    c.copy_from_slice(&tmp);
}

/// one column of a view
enum Col<'b> {
    R(&'b mut [(u8, u8)]),
    P(&'b mut [u8]),
    C(&'b mut [u16]),
}

/// the samples with property prop <= val first (a stable partition, the columns on the pool):
/// the same parts as split_samples in another order, which the search does not depend on
fn partition(v: &mut View, prop: usize, val: usize) {
    let n = v.len();
    let keep: Vec<bool> = (0..n).map(|i| v.prop(prop, i) <= val).collect();
    let mut cols: Vec<rt::sync::SpinLock<Col>> = Vec::new();
    for r in v.res.iter_mut() {
        cols.push(rt::sync::SpinLock::new(Col::R(&mut r[..])));
    }
    for p in v.props.iter_mut() {
        cols.push(rt::sync::SpinLock::new(Col::P(&mut p[..])));
    }
    cols.push(rt::sync::SpinLock::new(Col::C(&mut v.counts[..])));
    let keep = &keep;
    crate::emod::par_map(cols.len(), true, |k| {
        let mut g = cols[k].lock();
        match &mut *g {
            Col::R(c) => stable_part(c, keep),
            Col::P(c) => stable_part(c, keep),
            Col::C(c) => stable_part(c, keep),
        }
    });
}

/// the read-only parts of the samples and the search parameters
struct Info<'a> {
    preds: &'a [u32],
    props_used: &'a [u32],
    compact: &'a [Vec<i32>],
    mapping: &'a [Vec<u8>],
    threshold: f32,
    mul_info: &'a [MulInfo],
    fast_mult: f32,
    threads: bool,
}

impl Info<'_> {
    #[inline]
    fn quantize(&self, i: usize, v: i32) -> u8 {
        let v = v.clamp(-PROP_RANGE, PROP_RANGE) + PROP_RANGE;
        self.mapping[i][v as usize]
    }
    fn num_prop_values(&self, p: usize) -> usize {
        self.compact[p].len() + 1
    }
    fn unquantize(&self, p: usize, q: usize) -> i32 {
        self.compact[p][q]
    }
    fn pred_index(&self, p: u32) -> usize {
        self.preds.iter().position(|&x| x == p).unwrap_or(0)
    }
}

/// a node of the search result: its multiplier (when a multiplier range holds it), its split
/// (property, value, predictors of the parts above / below) and the results of the parts
/// above (pos..end) and below (begin..pos) as arena indices
#[derive(Clone, Copy)]
struct FNode {
    mul: Option<u32>,
    split: Option<(i32, i32, u32, u32)>,
    above: u32,
    below: u32,
}

/// the decision at one node (FindBestSplit's loop body): (multiplier, split + property +
/// unquantized value)
fn decide(info: &Info, v: &View, cur_pred: u32, used: u64, srange: &[[u32; 2]; 2], counts: &mut Vec<i32>, scr: &mut PropScr) -> (Option<u32>, Option<(Split, i32, i32)>) {
    let (begin, end) = (0usize, v.len());
    let threshold = info.threshold;
    let npred = info.preds.len();
    let nprop = info.props_used.len();
    let mut best_static_const = Split::default();
    let mut best_static = Split::default();
    let mut best_nonstatic = Split::default();
    let mut best_nowp = Split::default();
    let mut max_sym = 0usize;
    let mut tot_extra = vec![0usize; npred];
    const CHUNK: usize = 1 << 16;
    if info.threads && end - begin >= 2 * CHUNK {
        // large nodes: chunks of samples on the pool, integer sums (jxl-speed)
        let nch = (end - begin).div_ceil(CHUNK);
        let ms: Vec<usize> = crate::emod::par_map(nch, true, |k| {
            let (a, b) = (begin + k * CHUNK, (begin + (k + 1) * CHUNK).min(end));
            let mut m = 0usize;
            for p in 0..npred {
                for i in a..b {
                    m = m.max(v.tok(p, i) + 1);
                }
            }
            m
        });
        max_sym = ms.iter().copied().max().unwrap_or(0);
        let max_sym = (max_sym + 7) & !7;
        let parts: Vec<(Vec<i32>, Vec<usize>)> = crate::emod::par_map(nch, true, |k| {
            let (a, b) = (begin + k * CHUNK, (begin + (k + 1) * CHUNK).min(end));
            let mut cn = vec![0i32; max_sym * npred];
            let mut te = vec![0usize; npred];
            for p in 0..npred {
                for i in a..b {
                    let c = v.counts[i] as i32;
                    cn[p * max_sym + v.tok(p, i)] += c;
                    te[p] += v.nbits(p, i) * c as usize;
                }
            }
            (cn, te)
        });
        counts.clear();
        counts.resize(max_sym * npred, 0);
        for (cn, te) in &parts {
            for (a, b) in counts.iter_mut().zip(cn) {
                *a += b;
            }
            for (a, b) in tot_extra.iter_mut().zip(te) {
                *a += b;
            }
        }
    } else {
        for p in 0..npred {
            for i in begin..end {
                max_sym = max_sym.max(v.tok(p, i) + 1);
            }
        }
        let max_sym = (max_sym + 7) & !7;
        counts.clear();
        counts.resize(max_sym * npred, 0);
        for p in 0..npred {
            for i in begin..end {
                let c = v.counts[i] as i32;
                counts[p * max_sym + v.tok(p, i)] += c;
                tot_extra[p] += v.nbits(p, i) * c as usize;
            }
        }
    }
    let max_sym = (max_sym + 7) & !7;
    let base_bits = {
        let p = info.pred_index(cur_pred);
        estimate_bits(&counts[p * max_sym..(p + 1) * max_sym]) + tot_extra[p] as f32
    };
    let mut mul = None;
    let mut forced = Split::default();
    let mut use_forced = false;
    for m in info.mul_info {
        let mut axis = 0usize;
        let mut val = 0u32;
        let t = box_intersects(srange, &m.range, &mut axis, &mut val);
        if t == 0 {
            continue;
        }
        if t == 2 {
            mul = Some(m.mul);
            break;
        }
        forced.val = info.quantize(axis, val as i32) as usize;
        forced.prop = axis;
        forced.lcost = base_bits / 2.0 - threshold;
        forced.rcost = forced.lcost;
        forced.lpred = cur_pred;
        forced.rpred = cur_pred;
        forced.pos = begin;
        for x in begin..end {
            if v.prop(forced.prop, x) <= forced.val {
                forced.pos += 1;
            }
        }
        use_forced = true;
        break;
    }
    let mut best: Split;
    if use_forced {
        best = forced;
    } else {
        let change_penalty = 800.0f32 / (100.0 + threshold);
        if base_bits > threshold {
            let nin = NodeIn { begin, end, max_sym, npred, counts, tot_extra: &tot_extra, cur_pred, used, change_penalty };
            // large nodes: the properties on the worker pool (results combined in property
            // order: the same tree)
            let cands: Vec<[Split; 4]> = if info.threads && (end - begin) * nprop >= 1 << 15 {
                crate::emod::par_map(nprop, true, |prop| prop_splits(info, v, prop, &nin, &mut PropScr::default()))
            } else {
                (0..nprop).map(|prop| prop_splits(info, v, prop, &nin, scr)).collect()
            };
            let mut bests = [best_static_const, best_static, best_nonstatic, best_nowp];
            for c in &cands {
                for k in 0..4 {
                    if c[k].cost() < bests[k].cost() {
                        bests[k] = c[k];
                    }
                }
            }
            best_static_const = bests[0];
            best_static = bests[1];
            best_nonstatic = bests[2];
            best_nowp = bests[3];
        }
        best = best_nonstatic;
        if best_nowp.cost() + threshold < base_bits && best_nowp.cost() <= info.fast_mult * best.cost() {
            best = best_nowp;
        }
        if best_static.cost() + threshold < base_bits && best_static.cost() <= info.fast_mult * best.cost() {
            best = best_static;
        }
        if best_static_const.cost() + threshold < base_bits {
            best = best_static_const;
        }
    }
    if best.cost() + threshold < base_bits {
        let p = info.props_used[best.prop] as i32;
        let dq = info.unquantize(best.prop, best.val);
        (mul, Some((best, p, dq)))
    } else {
        (mul, None)
    }
}

/// the samples of a split node reordered (split_samples) and the (used, range) of its parts:
/// ((below: used, range), (above: used, range))
fn split_node(v: &mut View, best: &Split, p: i32, dq: i32, used: u64, srange: &[[u32; 2]; 2], par: bool) -> (u64, [[u32; 2]; 2], [[u32; 2]; 2]) {
    let n = v.len();
    if par {
        partition(v, best.prop, best.val);
    } else {
        split_samples(v, 0, best.pos, n, best.prop);
    }
    let mut used = used;
    if (p as usize) >= NUM_STATIC {
        used |= 1u64 << best.prop;
    }
    let mut r1 = *srange;
    if (p as usize) < NUM_STATIC {
        r1[p as usize][1] = (dq + 1) as u32;
    }
    let mut r2 = *srange;
    if (p as usize) < NUM_STATIC {
        r2[p as usize][0] = (dq + 1) as u32;
    }
    (used, r1, r2)
}

/// the subtree of a node, iteratively (a stack, as FindBestSplit): its arena (index 0 = the
/// node)
fn search_serial(info: &Info, v: View, pred: u32, used: u64, range: [[u32; 2]; 2]) -> Vec<FNode> {
    struct Task<'a> {
        at: usize,
        v: View<'a>,
        pred: u32,
        used: u64,
        range: [[u32; 2]; 2],
    }
    let mut arena = vec![FNode { mul: None, split: None, above: 0, below: 0 }];
    let mut stack = vec![Task { at: 0, v, pred, used, range }];
    let mut counts: Vec<i32> = Vec::new();
    let mut scr = PropScr::default();
    while let Some(Task { at, mut v, pred, used, range }) = stack.pop() {
        if v.len() == 0 {
            continue;
        }
        let (mul, sp) = decide(info, &v, pred, used, &range, &mut counts, &mut scr);
        arena[at].mul = mul;
        let Some((best, p, dq)) = sp else { continue };
        let (used2, r1, r2) = split_node(&mut v, &best, p, dq, used, &range, false);
        let (below, above) = v.split_at(best.pos);
        let ia = arena.len();
        arena.push(FNode { mul: None, split: None, above: 0, below: 0 });
        arena.push(FNode { mul: None, split: None, above: 0, below: 0 });
        arena[at].split = Some((p, dq, best.rpred, best.lpred));
        arena[at].above = ia as u32;
        arena[at].below = ia as u32 + 1;
        stack.push(Task { at: ia + 1, v: below, pred: best.lpred, used: used2, range: r1 });
        stack.push(Task { at: ia, v: above, pred: best.rpred, used: used2, range: r2 });
    }
    arena
}

/// the subtree of a node: large nodes split here and their parts searched at the same time
/// (to a limited depth), smaller ones by search_serial
fn search(info: &Info, mut v: View, pred: u32, used: u64, range: [[u32; 2]; 2], depth: u32) -> Vec<FNode> {
    let n = v.len();
    if !info.threads || depth >= 40 || n * info.props_used.len().max(1) < 1 << 13 {
        return search_serial(info, v, pred, used, range);
    }
    let mut counts: Vec<i32> = Vec::new();
    let mut scr = PropScr::default();
    let (mul, sp) = decide(info, &v, pred, used, &range, &mut counts, &mut scr);
    let mut arena = vec![FNode { mul, split: None, above: 0, below: 0 }];
    let Some((best, p, dq)) = sp else { return arena };
    let (used2, r1, r2) = split_node(&mut v, &best, p, dq, used, &range, true);
    let (below, above) = v.split_at(best.pos);
    let (aa, ab) = crate::emod::join(
        true,
        || search(info, above, best.rpred, used2, r2, depth + 1),
        || search(info, below, best.lpred, used2, r1, depth + 1),
    );
    let oa = 1u32;
    let ob = 1 + aa.len() as u32;
    arena[0].split = Some((p, dq, best.rpred, best.lpred));
    arena[0].above = oa;
    arena[0].below = ob;
    for mut f in aa {
        if f.split.is_some() {
            f.above += oa;
            f.below += oa;
        }
        arena.push(f);
    }
    for mut f in ab {
        if f.split.is_some() {
            f.above += ob;
            f.below += ob;
        }
        arena.push(f);
    }
    arena
}

/// ComputeBestTree (FindBestSplit): the decisions of every node (independent of the order the
/// nodes are searched in), then the tree numbered as the serial search numbers it: a split
/// node appends its children (the part above the split value, then the part below) and the
/// search goes on with the part above  [jxl-speed: parts searched at the same time]
pub fn best_tree(s: &mut Samples, threshold: f32, mul_info: &[MulInfo], range0: [[u32; 2]; 2], fast_mult: f32, threads: bool) -> Tree {
    let root_pred = s.preds[0];
    let mut range0 = range0;
    for r in range0.iter_mut() {
        if r[1] == 0 {
            r[1] = u32::MAX;
        }
    }
    let n = s.distinct();
    let Samples { preds, props_used, res, counts, props, compact, mapping, .. } = s;
    let info = Info { preds, props_used, compact, mapping, threshold, mul_info, fast_mult, threads };
    let view = View {
        res: res.iter_mut().map(|r| &mut r[..n]).collect(),
        props: props.iter_mut().map(|p| &mut p[..n]).collect(),
        counts: &mut counts[..n],
    };
    let arena = search(&info, view, root_pred, 0, range0, 0);
    let mut tree: Tree = vec![Node::leaf(root_pred)];
    let mut stack = vec![(0usize, 0usize)];
    while let Some((pos, a)) = stack.pop() {
        let f = arena[a];
        if let Some(m) = f.mul {
            tree[pos].mul = m;
        }
        if let Some((p, dq, rpred, lpred)) = f.split {
            // MakeSplitNode: lchild (> split) gets rpred, rchild gets lpred
            let l = tree.len() as u32;
            tree[pos].l = l;
            tree[pos].r = l + 1;
            tree[pos].split = dq;
            tree[pos].prop = p;
            tree.push(Node::leaf(rpred));
            tree.push(Node::leaf(lpred));
            stack.push((l as usize + 1, f.below as usize));
            stack.push((l as usize, f.above as usize));
        }
    }
    tree
}

/// the per-node inputs of the split search
struct NodeIn<'a> {
    begin: usize,
    end: usize,
    max_sym: usize,
    npred: usize,
    counts: &'a [i32],
    tot_extra: &'a [usize],
    cur_pred: u32,
    used: u64,
    change_penalty: f32,
}

/// scratch of the split search of one property
#[derive(Default)]
struct PropScr {
    count_inc: Vec<i32>,
    extra_inc: Vec<usize>,
    used_cnt: Vec<i32>,
    used4: Vec<i32>,    // jxl-speed: 4 interleaved partial counts (no store-to-load chains)
    extra4: Vec<usize>,
    costs_l: Vec<CostInfo>,
    costs_r: Vec<CostInfo>,
    counts_above: Vec<i32>,
    counts_below: Vec<i32>,
}

/// the best splits on property prop of a node: [static, both sides costing; static; adds the
/// WP; no WP] (the first of equal costs, as the serial search over the properties keeps them)
fn prop_splits(info: &Info, s: &View, prop: usize, n: &NodeIn, sc: &mut PropScr) -> [Split; 4] {
    let (begin, end, max_sym, npred) = (n.begin, n.end, n.max_sym, n.npred);
    let (counts, tot_extra, cur_pred, used, change_penalty) = (n.counts, n.tot_extra, n.cur_pred, n.used, n.change_penalty);
    let PropScr { count_inc, extra_inc, used_cnt, used4, extra4, costs_l, costs_r, counts_above, counts_below } = sc;
    counts_above.resize(max_sym, 0);
    counts_below.resize(max_sym, 0);
    let mut out = [Split::default(); 4];
        costs_l.clear();
        costs_r.clear();
        let psize = info.num_prop_values(prop);
        if extra_inc.len() < psize {
            count_inc.resize(psize * max_sym, 0);
            extra_inc.resize(psize, 0);
        }
        if count_inc.len() < psize * max_sym {
            count_inc.resize(psize * max_sym, 0);
        }
        used_cnt.clear();
        used_cnt.resize(psize, 0);
        let mut first_used = psize;
        let mut last_used = 0usize;
        let pcol = &s.props[prop][begin..end];
        // large nodes: partial sums in 4 interleaved copies (the same integer totals)
        let four = end - begin >= 64 * psize;
        if four {
            used4.clear();
            used4.resize(4 * psize, 0);
            let (u0, r) = used4.split_at_mut(psize);
            let (u1, r) = r.split_at_mut(psize);
            let (u2, u3) = r.split_at_mut(psize);
            let mut it = pcol.chunks_exact(4);
            for q in &mut it {
                u0[q[0] as usize] += 1;
                u1[q[1] as usize] += 1;
                u2[q[2] as usize] += 1;
                u3[q[3] as usize] += 1;
            }
            for &p in it.remainder() {
                u0[p as usize] += 1;
            }
            for p in 0..psize {
                let c = u0[p] + u1[p] + u2[p] + u3[p];
                used_cnt[p] = c;
                if c != 0 {
                    last_used = last_used.max(p);
                    first_used = first_used.min(p);
                }
            }
        } else {
            for &p in pcol {
                let p = p as usize;
                used_cnt[p] += 1;
                last_used = last_used.max(p);
                first_used = first_used.min(p);
            }
        }
        let span = last_used.saturating_sub(first_used);
        costs_l.resize(span, CostInfo { cost: f32::MAX, extra: 0.0, pred: 0 });
        costs_r.resize(span, CostInfo { cost: f32::MAX, extra: 0.0, pred: 0 });
        let ccol = &s.counts[begin..end];
        for pi in 0..npred {
            let rcol = &s.res[pi][begin..end];
            if four {
                extra4.clear();
                extra4.resize(4 * psize, 0);
                let ci = &mut count_inc[..psize * max_sym];
                for (j, ((&(sym, nb), &p), &c)) in rcol.iter().zip(pcol).zip(ccol).enumerate() {
                    let p = p as usize;
                    ci[p * max_sym + sym as usize] += c as i32;
                    extra4[(j & 3) * psize + p] += nb as usize * c as usize;
                }
                for p in 0..psize {
                    extra_inc[p] += extra4[p] + extra4[psize + p] + extra4[2 * psize + p] + extra4[3 * psize + p];
                }
            } else {
                for ((&(sym, nb), &p), &c) in rcol.iter().zip(pcol).zip(ccol) {
                    let p = p as usize;
                    count_inc[p * max_sym + sym as usize] += c as i32;
                    extra_inc[p] += nb as usize * c as usize;
                }
            }
            counts_above.copy_from_slice(&counts[pi * max_sym..(pi + 1) * max_sym]);
            counts_below.fill(0);
            let mut extra_below = 0usize;
            for i in first_used..last_used {
                if used_cnt[i] == 0 {
                    continue;
                }
                extra_below += extra_inc[i];
                extra_inc[i] = 0;
                for sym in 0..max_sym {
                    let v = count_inc[i * max_sym + sym];
                    counts_above[sym] -= v;
                    counts_below[sym] += v;
                    count_inc[i * max_sym + sym] = 0;
                }
                let rcost = estimate_bits(&counts_above) + (tot_extra[pi] - extra_below) as f32;
                let lcost = estimate_bits(&counts_below) + extra_below as f32;
                let mut penalty = 0.0f32;
                let pr = info.preds[pi];
                if pr != cur_pred && cur_pred != PRED_WEIGHTED {
                    penalty = change_penalty;
                }
                if pr == PRED_WEIGHTED {
                    penalty += 1e-8;
                }
                if pr == PRED_ZERO {
                    penalty -= 1e-8;
                }
                let k = i - first_used;
                if rcost + penalty < costs_r[k].total() {
                    costs_r[k] = CostInfo { cost: rcost, extra: penalty, pred: pr };
                }
                if lcost + penalty < costs_l[k].total() {
                    costs_l[k] = CostInfo { cost: lcost, extra: penalty, pred: pr };
                }
            }
            // the last used value's increments are cleared below (per predictor)
            extra_inc[last_used] = 0;
            for sym in 0..max_sym {
                count_inc[last_used * max_sym + sym] = 0;
            }
        }
        let mut split = begin;
        for i in first_used..last_used {
            if used_cnt[i] == 0 {
                continue;
            }
            split += used_cnt[i] as usize;
            let k = i - first_used;
            let rcost = costs_r[k].cost;
            let lcost = costs_l[k].cost;
            let pp = info.props_used[prop] as usize;
            let adds_wp = (pp == WP_PROP && (used & (1u64 << prop)) == 0)
                || ((costs_l[k].pred == PRED_WEIGHTED || costs_r[k].pred == PRED_WEIGHTED) && cur_pred != PRED_WEIGHTED);
            let zero_side = rcost == 0.0 || lcost == 0.0;
            let b: &mut Split = if pp < NUM_STATIC {
                if zero_side { &mut out[0] } else { &mut out[1] }
            } else if adds_wp {
                &mut out[2]
            } else {
                &mut out[3]
            };
            if lcost + rcost < b.cost() {
                *b = Split { prop, val: i, pos: split, lcost, rcost, lpred: costs_l[k].pred, rpred: costs_r[k].pred };
            }
        }
    out
}

/// TokenizeTree -> (tokens, the tree in decoder order with leaf ids in `l`)
pub fn tokenize(tree: &Tree) -> (Vec<Token>, Tree) {
    let mut toks = Vec::new();
    let mut dt: Tree = Vec::with_capacity(tree.len());
    let mut q = alloc::collections::VecDeque::new();
    q.push_back(0usize);
    let mut leaf = 0u32;
    while let Some(cur) = q.pop_front() {
        let n = tree[cur];
        toks.push(Token::new(1, (n.prop + 1) as u32));
        if n.prop == -1 {
            toks.push(Token::new(2, n.pred));
            toks.push(Token::new(3, pack_signed(n.off as i32)));
            let ml = n.mul.trailing_zeros();
            let mb = (n.mul >> ml) - 1;
            toks.push(Token::new(4, ml));
            toks.push(Token::new(5, mb));
            dt.push(Node { prop: -1, split: 0, l: leaf, r: 0, pred: n.pred, off: n.off, mul: n.mul });
            leaf += 1;
            continue;
        }
        let l = (dt.len() + q.len() + 1) as u32;
        dt.push(Node { prop: n.prop, split: n.split, l, r: l + 1, pred: 0, off: 0, mul: 1 });
        q.push_back(n.l as usize);
        q.push_back(n.r as usize);
        toks.push(Token::new(0, pack_signed(n.split)));
    }
    (toks, dt)
}

/// tree kinds of PredefinedTree
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Trivial,
    Learn,
    JpegAcMeta,
    FalconAcMeta,
    AcMeta,
    WpFixedDc,
    GradientFixedDc,
}

fn make_fixed(prop: i32, cutoffs: &[i32], pred: u32, pixels: usize, bitdepth: u32) -> Tree {
    let log_px = crate::eans::ceil_log2(pixels.max(1) as u32) as usize;
    let mut min_gap = 0usize;
    if log_px < 14 {
        min_gap = 8 * (14 - log_px);
    }
    let shift = if bitdepth > 11 { (bitdepth - 11).min(4) } else { 0 };
    let mul = 1i32 << shift;
    let mut tree: Tree = vec![Node::leaf(pred)];
    let mut q = alloc::collections::VecDeque::new();
    q.push_back((0usize, cutoffs.len(), 0usize));
    while let Some((b, e, pos)) = q.pop_front() {
        if b + min_gap >= e {
            continue;
        }
        let split = (b + e) / 2;
        let cutoff = cutoffs[split] * mul;
        let l = tree.len() as u32;
        tree[pos] = Node::split(prop, cutoff, l);
        q.push_back((split + 1, e, tree.len()));
        tree.push(Node::leaf(pred));
        q.push_back((b, split, tree.len()));
        tree.push(Node::leaf(pred));
    }
    tree
}

static FIXED_CUTOFFS: [i32; 33] = [
    -500, -392, -255, -191, -127, -95, -63, -47, -31, -23, -15, -11, -7, -4, -3, -1, 0, 1, 3, 5, 7, 11, 15, 23, 31, 47,
    63, 95, 127, 191, 255, 392, 500,
];

/// PredefinedTree
pub fn fixed_tree(kind: Kind, pixels: usize, bitdepth: u32, prevprop: i32) -> Tree {
    match kind {
        Kind::JpegAcMeta | Kind::Trivial => vec![Node::leaf(PRED_ZERO)],
        Kind::FalconAcMeta => vec![Node::leaf(PRED_LEFT)],
        Kind::AcMeta => {
            if pixels < 1024 {
                return vec![Node::leaf(PRED_LEFT)];
            }
            let z = PRED_ZERO;
            let l = PRED_LEFT;
            vec![
                Node::split(0, 1, 1),
                Node::split(0, 2, 3),
                Node::split(0, 0, 5),
                Node::split(6, 3, 21),
                Node::split(2, 0, 7),
                Node::leaf(PRED_GRADIENT),
                Node::leaf(PRED_GRADIENT),
                Node::split(7, 5, 9),
                Node::split(7, 5, 15),
                Node::split(7, 11, 11),
                Node::split(7, 3, 13),
                Node::leaf(l),
                Node::leaf(l),
                Node::leaf(l),
                Node::leaf(l),
                Node::split(7, 11, 17),
                Node::split(7, 3, 19),
                Node::leaf(z),
                Node::leaf(z),
                Node::leaf(z),
                Node::leaf(z),
                Node::split(7, 3, 23),
                Node::split(7, 3, 25),
                Node::leaf(z),
                Node::leaf(z),
                Node::leaf(z),
                Node::leaf(z),
            ]
        }
        Kind::WpFixedDc => make_fixed(WP_PROP as i32, &FIXED_CUTOFFS, PRED_WEIGHTED, pixels, bitdepth),
        Kind::GradientFixedDc => {
            let p = if prevprop > 0 { NUM_NONREF as i32 + 2 } else { GRAD_PROP as i32 };
            make_fixed(p, &FIXED_CUTOFFS, PRED_GRADIENT, pixels, bitdepth)
        }
        Kind::Learn => vec![Node::leaf(PRED_GRADIENT)],
    }
}

/// MergeTrees: trees[i] covers the streams splits[i] .. splits[i + 1]
pub fn merge(trees: &[Tree], splits: &[usize], begin: usize, end: usize, out: &mut Tree) {
    if end == begin + 1 {
        let sz = out.len() as u32;
        for n in &trees[begin] {
            let mut n = *n;
            if n.prop >= 0 {
                n.l += sz;
                n.r += sz;
            }
            out.push(n);
        }
        return;
    }
    let mid = (begin + end) / 2;
    let splitval = splits[mid] as i32 - 1;
    let cur = out.len();
    out.push(Node { prop: 1, split: splitval, l: 0, r: 0, pred: 0, off: 0, mul: 1 });
    out[cur].l = out.len() as u32;
    merge(trees, splits, mid, end, out);
    out[cur].r = out.len() as u32;
    merge(trees, splits, begin, mid, out);
}
