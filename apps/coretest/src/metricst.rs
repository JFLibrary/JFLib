// metricst -- checks of base::metrics: counters and gauges, the bucket layout of the
// log-linear histogram (exact below 32, then 32 steps per power of two, the last bucket at
// u64::MAX), quantiles by nearest rank with their error bound, equal values reported exactly,
// merging, and the window of 300 one-second slots (expiry, gaps, values per slot, the slots
// of a time span)

use alloc::vec::Vec;

use base::metrics::{Counter, Gauge, Hist, Window};

use crate::T;

const SEC: u64 = 1_000_000_000;

pub fn run(t: &mut T) {
    counters(t);
    layout(t);
    quantiles(t);
    window(t);
}

fn counters(t: &mut T) {
    let mut c = Counter::new();
    c.inc();
    c.add(41);
    t.eq("counter", c.get(), 42);
    c.add(u64::MAX);
    t.eq("counter saturates", c.get(), u64::MAX);
    let mut g = Gauge::new();
    g.set(5);
    g.add(-7);
    g.add(3);
    t.eq("gauge", (g.get(), g.max()), (1, 5));
}

fn layout(t: &mut T) {
    let h = Hist::new();
    t.eq("exact below 32", (h.bucket(0), h.bucket(31)), (0, 31));
    t.eq("first log bucket", (h.bucket(32), h.bucket(63), h.range(63)), (32, 63, (63, 1)));
    t.eq("width two", (h.bucket(64), h.bucket(65), h.bucket(66), h.range(64)), (64, 64, 65, (64, 2)));
    t.eq("one thousand", (h.bucket(1000), h.range(h.bucket(1000))), (190, (992, 16)));
    t.eq("largest", (h.bucket(u64::MAX), h.range(1919)), (1919, (63u64 << 58, 1u64 << 58)));
    // every value lies in its bucket, and bucket ranges follow each other without gaps
    let ok = (0..2000u64).chain([4095, 4096, 1 << 40, (1 << 40) + 12345, u64::MAX - 1]).all(|v| {
        let (lo, w) = h.range(h.bucket(v));
        lo <= v && v - lo < w
    });
    t.check("value in its bucket", ok);
    t.check("buckets touch", (0..1918).all(|i| {
        let (lo, w) = h.range(i);
        h.range(i + 1).0 == lo + w
    }));
    let h3 = Hist::with_bits(3);
    t.eq("three bits", (h3.bucket(7), h3.bucket(8), h3.bucket(16), h3.range(h3.bucket(17))), (7, 8, 16, (16, 2)));
}

fn quantiles(t: &mut T) {
    let mut h = Hist::new();
    t.eq("empty", (h.p50(), h.min(), h.mean()), (None, None, None));
    for v in 0..32u64 {
        h.record(v);
    }
    t.eq("exact quantiles", (h.p50(), h.quantile(1000), h.quantile(0), h.quantile(990)), (Some(15), Some(31), Some(0), Some(31)));
    t.eq("stats", (h.count(), h.sum(), h.min(), h.max(), h.mean()), (32, 496, Some(0), Some(31), Some(15)));
    // a ping over a switch: always 4104 ns
    let mut p = Hist::new();
    p.record_n(4104, 100);
    t.eq("equal values exact", (p.p50(), p.p95(), p.p99()), (Some(4104), Some(4104), Some(4104)));
    // 1..=100000: the median and p99 within half a bucket (1/64)
    let mut u = Hist::new();
    for v in 1..=100_000u64 {
        u.record(v);
    }
    let near = |got: Option<u64>, want: u64| got.is_some_and(|g| g.abs_diff(want) <= want / 64 + 1);
    t.check("median within 1.6 %", near(u.p50(), 50_000));
    t.check("p95 within 1.6 %", near(u.p95(), 95_000));
    t.check("p99 within 1.6 %", near(u.p99(), 99_000));
    t.eq("p99 value", u.p99(), Some(99_327));
    let mut a = Hist::new();
    a.record(10);
    a.record(1_000_000);
    let mut b = Hist::new();
    b.record(20);
    t.check("merge", a.merge(&b) && a.count() == 3 && a.min() == Some(10) && a.max() == Some(1_000_000) && a.p50() == Some(20));
    t.check("merge other resolution refused", !a.merge(&Hist::with_bits(4)));
    t.eq("buckets", a.buckets(), alloc::vec![(10, 1, 1), (20, 1, 1), (999_424, 16_384, 1)]);
    a.clear();
    t.eq("cleared", (a.count(), a.p50(), a.bits()), (0, None, 5));
}

fn window(t: &mut T) {
    let mut w = Window::new(300, SEC);
    t.eq("shape", (w.slots(), w.span()), (300, SEC));
    t.eq("empty window", (w.count(0), w.p99(0), w.min(0)), (0, None, None));
    // one value per second for ten minutes: the value is the second
    for s in 0..600u64 {
        t.check(&alloc::format!("record {}", s), w.record(s * SEC + SEC / 2, s));
    }
    let now = 599 * SEC + 900_000_000;
    t.eq("last five minutes", (w.count(now), w.min(now), w.max(now)), (300, Some(300), Some(599)));
    t.eq("sum and mean", (w.sum(now), w.mean(now)), ((300..600u128).sum(), Some(449)));
    t.eq("median", w.p50(now), Some(451));
    t.eq("p99", w.p99(now), Some(599));
    t.check("too old", !w.record(10 * SEC, 1));
    t.eq("hist of the window", w.hist(now).count(), 300);
    // the slots of a time span as one histogram (JFNetworker's time series)
    let mut want = Hist::new();
    for s in 400..410u64 {
        want.record(s);
    }
    t.eq("hist between", w.hist_between(400 * SEC, 410 * SEC), want);
    t.eq("between: slots that start in the span", (w.hist_between(400 * SEC + 1, 410 * SEC).count(), w.hist_between(400 * SEC, 410 * SEC + 1).count()), (9, 11));
    t.eq("between: what the ring still holds", (w.hist_between(0, 600 * SEC).count(), w.hist_between(0, 300 * SEC).count(), w.hist_between(5 * SEC, 5 * SEC).count()), (300, 0, 0));
    // a quiet minute: the old seconds leave the window as time goes on
    let later = now + 60 * SEC;
    t.eq("quiet minute", (w.count(later), w.min(later)), (240, Some(360)));
    let gone = now + 400 * SEC;
    t.eq("all gone", (w.count(gone), w.p50(gone)), (0, None));
    // values per slot for a chart: oldest first, empty slots without a value
    let per = w.per_slot(later, 500);
    t.eq("slots", (per.len(), per[0].0, per[0].2, per[239].2, per[240]), (300, 360 * SEC, Some(360), Some(599), (600 * SEC, 0, None)));
    let mut b = Window::new(10, 100);
    b.record_n(5, 7, 3);
    b.record(5, 9);
    b.record(150, 1000);
    t.eq("several in a slot", b.per_slot(199, 500).iter().filter(|s| s.1 > 0).map(|s| (s.0, s.1, s.2)).collect::<Vec<_>>(), alloc::vec![(0, 4, Some(7)), (100, 1, Some(1000))]);
    t.eq("small window quantile", (b.p50(199), b.quantile(199, 1000)), (Some(7), Some(1000)));
}
