// metrics -- measurements of running systems without keeping single values: counters, gauges,
// log-linear histograms and a sliding window of them (simulators, servers, benchmarks)
//
//   Counter        inc / add / get (saturating u64)
//   Gauge          set / add / get / max (i64; max is the highest value seen)
//   Hist           log-linear buckets: values below 2^b exactly, above in 2^b linear steps
//                  per power of two (b = 5: at most 1/32 = 3.1 % bucket width, the midpoint
//                  is off by half of that); record / record_n / count / sum / min / max /
//                  mean / quantile(per_mille) / p50 / p95 / p99 / merge / clear / buckets
//   Window         n slots of `span` time units each (Window::new(300, 1_000_000_000): five
//                  minutes in 1 s slots of nanoseconds); record(t, v) / quantile(t, pm) /
//                  p50 / p95 / p99 / count / sum / mean / min / max over the slots still in
//                  the window at t / per_slot(t, pm) for charts / hist(t)
//                  hist_between(from, to): the slots that start in [from, to) as one
//                  histogram (a time series: one point per interval, quantiles of any span);
//                  bytes(): the heap memory it holds -- its slots come with the first value,
//                  so a window that never got one costs nothing (a simulator keeps one per
//                  device and copies them often)
//
// Quantiles use the nearest rank (rank = ceil(pm * n / 1000)) and give the midpoint of that
// rank's bucket, clamped to the smallest and largest value seen (the first and the last rank
// are those values), so a run of equal values reports that value exactly.  A window slot keeps a sparse histogram (bucket, count) and its
// own count, sum, min and max; nothing stores the values themselves.  All integer arithmetic:
// the same values give the same results on every target.

use alloc::vec::Vec;

/// a monotonic count
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counter(u64);

impl Counter {
    pub const fn new() -> Counter {
        Counter(0)
    }
    pub fn inc(&mut self) {
        self.0 = self.0.saturating_add(1);
    }
    pub fn add(&mut self, n: u64) {
        self.0 = self.0.saturating_add(n);
    }
    pub fn get(&self) -> u64 {
        self.0
    }
}

/// a value that goes up and down, with the highest value it reached
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gauge {
    v: i64,
    max: i64,
}

impl Gauge {
    pub const fn new() -> Gauge {
        Gauge { v: 0, max: 0 }
    }
    pub fn set(&mut self, v: i64) {
        self.v = v;
        self.max = self.max.max(v);
    }
    pub fn add(&mut self, d: i64) {
        self.set(self.v.saturating_add(d));
    }
    pub fn get(&self) -> i64 {
        self.v
    }
    pub fn max(&self) -> i64 {
        self.max
    }
}

/// the default resolution: 32 linear steps per power of two
pub const HIST_BITS: u8 = 5;

/// the bucket of `v` with `b` sub-bucket bits
fn bucket_of(b: u32, v: u64) -> usize {
    let sub = 1u64 << b;
    if v < sub {
        return v as usize;
    }
    let e = 63 - v.leading_zeros();
    let s = (v >> (e - b)) - sub;
    (((e - b + 1) as usize) << b) | s as usize
}

/// the smallest value of bucket `i` and its width
fn bucket_range(b: u32, i: usize) -> (u64, u64) {
    let sub = 1usize << b;
    if i < sub {
        return (i as u64, 1);
    }
    let k = (i >> b) as u32;
    let s = (i & (sub - 1)) as u64;
    (((1u64 << b) + s) << (k - 1), 1u64 << (k - 1))
}

/// the value reported for a bucket: its midpoint, within what was seen
fn estimate(b: u32, i: usize, min: u64, max: u64) -> u64 {
    let (lo, w) = bucket_range(b, i);
    (lo + (w - 1) / 2).clamp(min, max)
}

/// the nearest rank of a quantile in per mille (1-based)
fn rank(pm: u32, n: u64) -> u64 {
    let r = (pm.min(1000) as u128 * n as u128).div_ceil(1000) as u64;
    r.clamp(1, n)
}

/// a log-linear histogram
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hist {
    bits: u8,
    counts: Vec<u64>,
    n: u64,
    sum: u128,
    min: u64,
    max: u64,
}

impl Default for Hist {
    fn default() -> Hist {
        Hist::new()
    }
}

impl Hist {
    pub fn new() -> Hist {
        Hist::with_bits(HIST_BITS)
    }
    /// `bits` linear steps per power of two as a power of two (1..=8)
    pub fn with_bits(bits: u8) -> Hist {
        Hist { bits: bits.clamp(1, 8), counts: Vec::new(), n: 0, sum: 0, min: u64::MAX, max: 0 }
    }
    pub fn bits(&self) -> u8 {
        self.bits
    }
    /// the bucket a value falls into
    pub fn bucket(&self, v: u64) -> usize {
        bucket_of(self.bits as u32, v)
    }
    /// the smallest value and the width of a bucket
    pub fn range(&self, i: usize) -> (u64, u64) {
        bucket_range(self.bits as u32, i)
    }
    pub fn record(&mut self, v: u64) {
        self.record_n(v, 1);
    }
    pub fn record_n(&mut self, v: u64, n: u64) {
        if n == 0 {
            return;
        }
        let i = self.bucket(v);
        if self.counts.len() <= i {
            self.counts.resize(i + 1, 0);
        }
        self.counts[i] = self.counts[i].saturating_add(n);
        self.n = self.n.saturating_add(n);
        self.sum = self.sum.saturating_add(v as u128 * n as u128);
        self.min = self.min.min(v);
        self.max = self.max.max(v);
    }
    pub fn count(&self) -> u64 {
        self.n
    }
    pub fn sum(&self) -> u128 {
        self.sum
    }
    pub fn min(&self) -> Option<u64> {
        if self.n == 0 { None } else { Some(self.min) }
    }
    pub fn max(&self) -> Option<u64> {
        if self.n == 0 { None } else { Some(self.max) }
    }
    /// the mean, rounded down
    pub fn mean(&self) -> Option<u64> {
        if self.n == 0 { None } else { Some((self.sum / self.n as u128) as u64) }
    }
    /// the value at `per_mille` (500: median, 990: p99, 1000: the largest)
    pub fn quantile(&self, per_mille: u32) -> Option<u64> {
        if self.n == 0 {
            return None;
        }
        let r = rank(per_mille, self.n);
        if r == self.n {
            return Some(self.max);
        }
        if r == 1 {
            return Some(self.min);
        }
        let mut cum = 0;
        for (i, &c) in self.counts.iter().enumerate() {
            cum += c;
            if cum >= r {
                return Some(estimate(self.bits as u32, i, self.min, self.max));
            }
        }
        Some(self.max)
    }
    pub fn p50(&self) -> Option<u64> {
        self.quantile(500)
    }
    pub fn p95(&self) -> Option<u64> {
        self.quantile(950)
    }
    pub fn p99(&self) -> Option<u64> {
        self.quantile(990)
    }
    /// adds the values of another histogram of the same resolution (false otherwise)
    pub fn merge(&mut self, o: &Hist) -> bool {
        if o.bits != self.bits {
            return false;
        }
        if self.counts.len() < o.counts.len() {
            self.counts.resize(o.counts.len(), 0);
        }
        for (i, &c) in o.counts.iter().enumerate() {
            self.counts[i] = self.counts[i].saturating_add(c);
        }
        self.n = self.n.saturating_add(o.n);
        self.sum = self.sum.saturating_add(o.sum);
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
        true
    }
    pub fn clear(&mut self) {
        *self = Hist::with_bits(self.bits);
    }
    /// the buckets that hold values: (smallest value, width, count)
    pub fn buckets(&self) -> Vec<(u64, u64, u64)> {
        self.counts.iter().enumerate().filter(|(_, c)| **c > 0).map(|(i, &c)| {
            let (lo, w) = self.range(i);
            (lo, w, c)
        }).collect()
    }
}

/// one slot of a window: a sparse histogram of one span of time
#[derive(Clone, Debug, PartialEq, Eq)]
struct Slot {
    /// the slot number (time / span); u64::MAX: never used
    idx: u64,
    n: u64,
    sum: u128,
    min: u64,
    max: u64,
    /// (bucket, count), ascending
    cells: Vec<(u16, u32)>,
}

impl Slot {
    fn empty() -> Slot {
        Slot { idx: u64::MAX, n: 0, sum: 0, min: u64::MAX, max: 0, cells: Vec::new() }
    }
    fn reset(&mut self, idx: u64) {
        *self = Slot { idx, cells: core::mem::take(&mut self.cells), ..Slot::empty() };
        self.cells.clear();
    }
    fn add(&mut self, bucket: usize, v: u64, n: u32) {
        let b = bucket as u16;
        match self.cells.binary_search_by_key(&b, |c| c.0) {
            Ok(i) => self.cells[i].1 = self.cells[i].1.saturating_add(n),
            Err(i) => self.cells.insert(i, (b, n)),
        }
        self.n = self.n.saturating_add(n as u64);
        self.sum = self.sum.saturating_add(v as u128 * n as u128);
        self.min = self.min.min(v);
        self.max = self.max.max(v);
    }
}

/// adds the values of a slot to a histogram of the same resolution
fn add_slot(h: &mut Hist, s: &Slot) {
    for &(b, c) in &s.cells {
        let i = b as usize;
        if h.counts.len() <= i {
            h.counts.resize(i + 1, 0);
        }
        h.counts[i] = h.counts[i].saturating_add(c as u64);
    }
    h.n = h.n.saturating_add(s.n);
    h.sum = h.sum.saturating_add(s.sum);
    h.min = h.min.min(s.min);
    h.max = h.max.max(s.max);
}

/// a histogram over the last `slots` spans of time
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    span: u64,
    bits: u8,
    /// `len` slots, made when the first value comes (empty until then)
    ring: Vec<Slot>,
    len: usize,
    /// the newest slot number recorded into
    head: u64,
}

impl Window {
    /// `slots` slots of `span` time units each (at least one of each)
    pub fn new(slots: usize, span: u64) -> Window {
        Window::with_bits(slots, span, HIST_BITS)
    }
    pub fn with_bits(slots: usize, span: u64, bits: u8) -> Window {
        Window { span: span.max(1), bits: bits.clamp(1, 8), ring: Vec::new(), len: slots.max(1), head: 0 }
    }
    pub fn slots(&self) -> usize {
        self.len
    }
    /// the heap memory it holds: its slots and their histograms (none before the first value)
    pub fn bytes(&self) -> usize {
        self.ring.capacity() * core::mem::size_of::<Slot>() + self.ring.iter().map(|s| s.cells.capacity() * core::mem::size_of::<(u16, u32)>()).sum::<usize>()
    }
    pub fn span(&self) -> u64 {
        self.span
    }
    /// counts `v` at time `t`; false when t lies before the window that ends at the newest
    /// time recorded so far
    pub fn record(&mut self, t: u64, v: u64) -> bool {
        self.record_n(t, v, 1)
    }
    pub fn record_n(&mut self, t: u64, v: u64, n: u32) -> bool {
        let i = t / self.span;
        let len = self.len as u64;
        if i == u64::MAX {
            return false; // the slot number of "never used"
        }
        if i > self.head {
            self.head = i;
        }
        if i.saturating_add(len) <= self.head || n == 0 {
            return false;
        }
        let bucket = bucket_of(self.bits as u32, v);
        if self.ring.is_empty() {
            self.ring = alloc::vec![Slot::empty(); self.len];
        }
        let s = &mut self.ring[(i % len) as usize];
        if s.idx != i {
            s.reset(i);
        }
        s.add(bucket, v, n);
        true
    }
    /// the slots still in the window at time t
    fn live(&self, t: u64) -> impl Iterator<Item = &Slot> {
        let now = t / self.span;
        let len = self.len as u64;
        self.ring.iter().filter(move |s| s.idx != u64::MAX && s.idx <= now && s.idx.saturating_add(len) > now)
    }
    /// everything in the window at time t as one histogram
    pub fn hist(&self, t: u64) -> Hist {
        let mut h = Hist::with_bits(self.bits);
        for s in self.live(t) {
            add_slot(&mut h, s);
        }
        h
    }
    /// the slots that start in [from, to) as one histogram: what was recorded between two
    /// times, to the resolution of the slots; slots the ring no longer holds are left out
    pub fn hist_between(&self, from: u64, to: u64) -> Hist {
        let mut h = Hist::with_bits(self.bits);
        let len = self.len as u64;
        let (a, b) = (from.div_ceil(self.span), to.div_ceil(self.span));
        // only the newest `len` slot numbers can still be in the ring
        for i in a.max(b.saturating_sub(len))..b {
            if let Some(s) = self.ring.get((i % len) as usize)
                && s.idx == i
            {
                add_slot(&mut h, s);
            }
        }
        h
    }
    pub fn count(&self, t: u64) -> u64 {
        self.live(t).map(|s| s.n).sum()
    }
    pub fn sum(&self, t: u64) -> u128 {
        self.live(t).map(|s| s.sum).sum()
    }
    pub fn mean(&self, t: u64) -> Option<u64> {
        let n = self.count(t);
        if n == 0 { None } else { Some((self.sum(t) / n as u128) as u64) }
    }
    pub fn min(&self, t: u64) -> Option<u64> {
        self.live(t).filter(|s| s.n > 0).map(|s| s.min).min()
    }
    pub fn max(&self, t: u64) -> Option<u64> {
        self.live(t).filter(|s| s.n > 0).map(|s| s.max).max()
    }
    pub fn quantile(&self, t: u64, per_mille: u32) -> Option<u64> {
        self.hist(t).quantile(per_mille)
    }
    pub fn p50(&self, t: u64) -> Option<u64> {
        self.quantile(t, 500)
    }
    pub fn p95(&self, t: u64) -> Option<u64> {
        self.quantile(t, 950)
    }
    pub fn p99(&self, t: u64) -> Option<u64> {
        self.quantile(t, 990)
    }
    /// one value per slot of the window at time t, oldest first: (start time of the slot,
    /// count, quantile or None when the slot is empty)
    pub fn per_slot(&self, t: u64, per_mille: u32) -> Vec<(u64, u64, Option<u64>)> {
        let now = t / self.span;
        let len = self.len as u64;
        let first = now.saturating_sub(len - 1);
        (first..=now)
            .map(|i| {
                let Some(s) = self.ring.get((i % len) as usize).filter(|s| s.idx == i && s.n > 0) else {
                    return (i * self.span, 0, None);
                };
                let r = rank(per_mille, s.n);
                let mut cum = 0u64;
                let mut v = if r == 1 { s.min } else { s.max };
                for &(b, c) in s.cells.iter().filter(|_| r > 1 && r < s.n) {
                    cum += c as u64;
                    if cum >= r {
                        v = estimate(self.bits as u32, b as usize, s.min, s.max);
                        break;
                    }
                }
                (i * self.span, s.n, Some(v))
            })
            .collect()
    }
}
