// series -- time series for the lab: a sampler on virtual time keeps the metrics of every
// service and the traffic of every cable at a fixed interval, for the charts and KPI tiles of
// the lab's window and for the windowed expectations of scenarios
//
//   SeriesCfg { every, keep, links }   Engine::series_cfg() / set_series_cfg(cfg): the interval
//                                      (whole seconds, default 1 s), the points kept per series
//                                      (default 300; 0: no sampling), cables too (default yes);
//                                      a new setting starts the series afresh
//   Engine::series_gen()               changes whenever points arrive or the series start
//                                      afresh: the window redraws only then
//   Engine::series_times()             the moments of the kept samples, oldest first
//   Engine::series_services()          the services with series: SvcSeries (device, service,
//                                      kind; name, kind and unit of each series)
//   Engine::series(dev, svc)           every series of a service as SeriesView (name, kind,
//                                      unit, points (t, value)); series_one(dev, svc, name)
//   Engine::cable_series(link)         a cable's frames_ab, bytes_ab, frames_ba, bytes_ba per
//                                      second (a: the end the project names first)
//   Engine::series_value(dev, svc, name, agg, from, to)   one value of the samples taken in
//                                      (from, to] in thousandths (AggOut, else SeriesErr): what
//                                      "expect ... over 5s" checks (Agg: jfn_model::scenario)
//   Engine::kpi(dev, svc, span) / kpis(span)   the KPI tiles of a service over the last span
//                                      (Kpi): requests per second, p50 / p95 / p99, errors per
//                                      mille, queue and work in flight, cache hits, a KV node's
//                                      load, keys and bytes
//   Engine::shard_load(span)           the load of every KV node and its share of all of them
//   Engine::chart_marks(from, to)      the faults (fault.rs FaultMark) that overlap a span: the
//                                      marks of the charts
//   Engine::series_csv() / series_size()   every kept point as CSV, a column per series and a
//                                      row per sample (the runner's "series FILE") / how many
//                                      series and values there are
//
//   MetricKind, metric_kind_of(name)   what a service says of a metric (Service::metric_kind)
//   LatWindow::new(&window, LAT)       what it says of a latency window (Service::windows): LAT
//                                      names p50_us, p95_us, p99_us; LatStat a quantile, the
//                                      mean or the maximum of each interval
//
// What a metric becomes: Service::metric_kind (by default metric_kind_of, which goes by the
// names the built-in services use) makes it a counter, whose series is its rate per second
// (the change since the previous sample; a counter that went down, as a replaced service's,
// counts from 0 again), a gauge, whose series is its value, or nothing.  Service::windows
// names a service's latency windows (base::metrics::Window): the p50 / p95 / p99 of each
// interval (and its mean and maximum where a service asks for them) become series in µs and
// stand for the metrics of the same names, which cover five minutes.  Over a span a rate is
// the change across it, a gauge the mean of its samples, a latency the quantile of all that
// the window holds for the span (it keeps five minutes).
//
// Sampling: the sample at t covers [t - every, t).  It is taken before the first event at or
// after t (Engine::step) or when run_until brings the clock to t, and it only reads: the
// services' metrics_at(t - 1), their windows and the ports' counters.  It schedules no event
// and writes no note, so runs, traces and reports stay exactly as they were.  The first sample
// only takes the counters as they are; a service that appears later counts from 0, a cable from
// the first sample that sees it (its ports may have counted for another cable before).  Nothing
// is kept per request: a point is one i64 per series and sample, and a quiet stretch longer
// than the kept points is not sampled one by one.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::metrics::{Hist, Window};
use jfn_model::ifname;
pub use jfn_model::scenario::Agg;

use crate::fault::FaultMark;
use crate::kernel::{Engine, Ns, SEC};
use crate::os::Service;

/// points a series keeps at most (a day of one-second samples)
pub const KEEP_MAX: usize = 86_400;

/// no value at a sample
const NONE: i64 = i64::MIN;

// ---------------------------------------------------------------- what the services say

/// what a metric of Service::metrics_at is to the time series
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricKind {
    /// it only counts up: the series is its rate per second
    Counter,
    /// a value now: the series is the value at each sample
    Gauge,
    /// no series
    Skip,
}

/// metric names of levels, not counts
const GAUGES: &[&str] = &["active", "inflight", "queued", "max_queued", "depth", "max_depth", "entries", "bytes", "keys", "busy", "users", "topics", "leases", "records", "backends", "backends_up", "members_up", "bound", "synced", "connected", "rps"];

/// the kind of a metric after its name, as the built-in services name them: a gauge when it is
/// a level (active, inflight, queued, depth, entries, bytes, keys, busy, users, leases ...),
/// ends in _active or _up, starts with last_ or carries the unit of a value (_us, _ns, _ms,
/// _pm, _mrps); else a counter
pub fn metric_kind_of(name: &str) -> MetricKind {
    if GAUGES.contains(&name) || name.starts_with("last_") || ["_active", "_up", "_us", "_ns", "_ms", "_pm", "_mrps"].iter().any(|s| name.ends_with(s)) {
        MetricKind::Gauge
    } else {
        MetricKind::Counter
    }
}

/// what a latency series shows of each interval
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LatStat {
    /// a quantile in per mille (500: the median)
    Quantile(u16),
    Mean,
    Max,
}

/// a latency window of a service for the time series (Service::windows): the window and the
/// series it makes (name, what of each interval)
#[derive(Clone, Copy, Debug)]
pub struct LatWindow<'a> {
    pub win: &'a Window,
    pub stats: &'static [(&'static str, LatStat)],
}

/// the series most latency windows make
pub const LAT: &[(&str, LatStat)] = &[("p50_us", LatStat::Quantile(500)), ("p95_us", LatStat::Quantile(950)), ("p99_us", LatStat::Quantile(990))];

impl<'a> LatWindow<'a> {
    pub fn new(win: &'a Window, stats: &'static [(&'static str, LatStat)]) -> LatWindow<'a> {
        LatWindow { win, stats }
    }
}

/// what a latency window gives of a histogram (ns)
fn lat_of(h: &Hist, stat: LatStat) -> Option<u64> {
    match stat {
        LatStat::Quantile(pm) => h.quantile(pm as u32),
        LatStat::Mean => h.mean(),
        LatStat::Max => h.max(),
    }
}

// ---------------------------------------------------------------- series

/// what a series is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesKind {
    /// a counter's rate per second
    Rate,
    /// a gauge's value at each sample
    Gauge,
    /// a latency window's quantile, mean or maximum of each interval (µs)
    Lat(LatStat),
}

/// the unit of a series' values
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// a number of things (connections, keys; a flag)
    Count,
    /// things per second
    PerSec,
    Bytes,
    BytesPerSec,
    Us,
    Ms,
    /// milliseconds per second (busy time: 1000 is one worker all the time)
    MsPerSec,
    Ns,
    /// per thousand
    PerMille,
    /// thousandths per second (rate_mrps)
    MilliPerSec,
}

impl Unit {
    /// how charts and the CSV write it
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Count => "",
            Unit::PerSec => "/s",
            Unit::Bytes => "B",
            Unit::BytesPerSec => "B/s",
            Unit::Us => "\u{b5}s",
            Unit::Ms => "ms",
            Unit::MsPerSec => "ms/s",
            Unit::Ns => "ns",
            Unit::PerMille => "\u{2030}",
            Unit::MilliPerSec => "/1000 s",
        }
    }
}

/// the unit of a series after its name
fn unit_of(name: &str, kind: SeriesKind) -> Unit {
    let bytes = name == "bytes" || name.starts_with("bytes_") || name.ends_with("_bytes");
    match kind {
        SeriesKind::Lat(_) => Unit::Us,
        SeriesKind::Rate if bytes => Unit::BytesPerSec,
        SeriesKind::Rate if name.ends_with("_ms") => Unit::MsPerSec,
        SeriesKind::Rate => Unit::PerSec,
        SeriesKind::Gauge if name.ends_with("_us") => Unit::Us,
        SeriesKind::Gauge if name.ends_with("_ms") => Unit::Ms,
        SeriesKind::Gauge if name.ends_with("_ns") => Unit::Ns,
        SeriesKind::Gauge if name.ends_with("_pm") => Unit::PerMille,
        SeriesKind::Gauge if name.ends_with("_mrps") => Unit::MilliPerSec,
        SeriesKind::Gauge if name == "rps" => Unit::PerSec,
        SeriesKind::Gauge if bytes => Unit::Bytes,
        SeriesKind::Gauge => Unit::Count,
    }
}

/// how the sampler works
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeriesCfg {
    /// the interval: whole seconds (other values are rounded up), the slots of the services'
    /// latency windows
    pub every: Ns,
    /// points kept per series (0: no sampling; at most KEEP_MAX)
    pub keep: usize,
    /// the cables too
    pub links: bool,
}

impl Default for SeriesCfg {
    fn default() -> SeriesCfg {
        SeriesCfg { every: SEC, keep: 300, links: true }
    }
}

/// one series of the sampler
#[derive(Clone, Debug)]
struct Ser {
    name: &'static str,
    kind: SeriesKind,
    unit: Unit,
    /// a counter at the previous sample (None: not known)
    last: Option<i64>,
    /// one value per sample, the newest that of the newest sample: a rate's change over the
    /// interval, a gauge's value, a latency in µs; NONE where there was none
    vals: VecDeque<i64>,
    /// the sample that wrote it last
    stamp: u64,
    /// samples in a row its owner did not report it (then it goes)
    miss: usize,
}

impl Ser {
    fn new(name: &'static str, kind: SeriesKind, last: Option<i64>) -> Ser {
        Ser { name, kind, unit: unit_of(name, kind), last, vals: VecDeque::new(), stamp: 0, miss: 0 }
    }
    /// the value of a sample (`base`: the sampler's first, which only takes the counters)
    fn put(&mut self, v: Option<i64>, stamp: u64, base: bool, keep: usize) {
        let x = match (self.kind, v) {
            (SeriesKind::Rate, Some(v)) => {
                let d = match self.last {
                    Some(l) if v >= l => v - l,
                    // it went down: it counts from 0 again
                    Some(_) => v,
                    None => NONE,
                };
                self.last = Some(v);
                d
            }
            (_, Some(v)) => v,
            (_, None) => NONE,
        };
        self.stamp = stamp;
        if !base {
            self.vals.push_back(x);
            if self.vals.len() > keep {
                self.vals.pop_front();
            }
        }
    }
    /// a kept value as a point: a rate per second, the others as they are
    fn value(&self, v: i64, every: Ns) -> i64 {
        match self.kind {
            SeriesKind::Rate => (v as i128 * SEC as i128 / every.max(1) as i128) as i64,
            _ => v,
        }
    }
}

/// the series of one owner: a service of a device or a cable
#[derive(Clone, Debug)]
struct Rec {
    /// the service's id (another one: the service was replaced) or the cable's project id
    id: u32,
    /// the service's name ("" for a cable)
    name: String,
    kind: &'static str,
    /// the sample that saw it last
    stamp: u64,
    ser: Vec<Ser>,
}

impl Rec {
    /// the series `name`, made when it is new (`unknown`: its counter has no earlier value);
    /// `hint` is where the previous one was, as the series come in the same order each time
    fn slot(&mut self, hint: &mut usize, name: &'static str, kind: SeriesKind, unknown: bool) -> &mut Ser {
        let i = match self.ser.get(*hint) {
            Some(x) if x.name == name => *hint,
            _ => match self.ser.iter().position(|x| x.name == name) {
                Some(i) => i,
                None => {
                    self.ser.push(Ser::new(name, kind, if unknown { None } else { Some(0) }));
                    self.ser.len() - 1
                }
            },
        };
        *hint = i + 1;
        let x = &mut self.ser[i];
        if x.kind != kind {
            // the same name now means something else: a new series
            *x = Ser::new(name, kind, Some(0));
        }
        x.miss = 0;
        x
    }
    fn find(&self, name: &str) -> Option<&Ser> {
        self.ser.iter().find(|x| x.name == name)
    }
}

/// the time series of an engine (Engine::series_* below)
#[derive(Clone, Debug)]
pub struct Sampler {
    cfg: SeriesCfg,
    /// when the next sample is due (Ns::MAX: no sampling)
    pub(crate) next: Ns,
    /// samples taken so far (the first one only takes the counters)
    taken: u64,
    /// counts the samples with points and the fresh starts (Engine::series_gen)
    version: u64,
    /// the moments of the kept samples, oldest first
    times: VecDeque<Ns>,
    /// per device its services, in the order the device has them
    svcs: BTreeMap<u32, Vec<Rec>>,
    /// per cable (project id)
    links: BTreeMap<u32, Rec>,
}

impl Default for Sampler {
    fn default() -> Sampler {
        Sampler::new(SeriesCfg::default(), 0)
    }
}

impl Sampler {
    fn new(cfg: SeriesCfg, first: Ns) -> Sampler {
        Sampler { next: if cfg.keep == 0 { Ns::MAX } else { first }, cfg, taken: 0, version: 0, times: VecDeque::new(), svcs: BTreeMap::new(), links: BTreeMap::new() }
    }
    fn rec(&self, dev: u32, svc: &str) -> Option<&Rec> {
        self.svcs.get(&dev)?.iter().find(|r| r.name.eq_ignore_ascii_case(svc))
    }
    /// the kept samples of a series: (moment, value or NONE), oldest first
    fn samples<'a>(&'a self, x: &'a Ser) -> impl Iterator<Item = (Ns, i64)> + 'a {
        let n = x.vals.len().min(self.times.len());
        let (vo, to) = (x.vals.len() - n, self.times.len() - n);
        (0..n).map(move |i| (self.times[to + i], x.vals[vo + i]))
    }
    fn view(&self, x: &Ser) -> SeriesView {
        let every = self.cfg.every;
        SeriesView { name: x.name, kind: x.kind, unit: x.unit, points: self.samples(x).filter(|s| s.1 != NONE).map(|(t, v)| (t, x.value(v, every))).collect() }
    }
    /// the samples taken in (from, to]
    fn span<'a>(&'a self, x: &'a Ser, from: Ns, to: Ns) -> impl Iterator<Item = (Ns, i64)> + 'a {
        self.samples(x).filter(move |s| s.0 > from && s.0 <= to)
    }
    /// a rate over a span in thousandths per second: the change over the samples with a
    /// value, divided by their time
    fn rate_milli(&self, x: &Ser, from: Ns, to: Ns) -> Option<i64> {
        let (mut sum, mut n) = (0i128, 0i128);
        for (_, v) in self.span(x, from, to).filter(|s| s.1 != NONE) {
            sum += v as i128;
            n += 1;
        }
        if n == 0 { None } else { Some((sum * 1000 * SEC as i128 / (n * self.cfg.every.max(1) as i128)) as i64) }
    }
    /// the change of counters over a span
    fn change(&self, rec: &Rec, names: &[&str], from: Ns, to: Ns) -> Option<i128> {
        let mut found = None;
        for x in names.iter().filter_map(|n| rec.find(n)).filter(|x| x.kind == SeriesKind::Rate) {
            let d: i128 = self.span(x, from, to).filter(|s| s.1 != NONE).map(|s| s.1 as i128).sum();
            found = Some(found.unwrap_or(0) + d);
        }
        found
    }
    /// the newest value of a series in a span
    fn last_in(&self, rec: &Rec, name: &str, from: Ns, to: Ns) -> Option<i64> {
        let x = rec.find(name)?;
        self.span(x, from, to).filter(|s| s.1 != NONE).last().map(|s| x.value(s.1, self.cfg.every))
    }
}

/// a series as the charts take it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeriesView {
    pub name: &'static str,
    pub kind: SeriesKind,
    pub unit: Unit,
    /// (moment of the sample, value), oldest first: a rate per second, a gauge as it was, a
    /// latency in µs; samples without a value are left out
    pub points: Vec<(Ns, i64)>,
}

/// a service that has series
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvcSeries {
    pub dev: u32,
    pub svc: String,
    pub kind: &'static str,
    /// name, kind and unit of each of its series, in the order they came
    pub series: Vec<(&'static str, SeriesKind, Unit)>,
}

/// one value of a series over a span (Engine::series_value)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AggOut {
    /// thousandths of the series' unit (a rate: per second; a latency of the span: ns)
    pub milli: i64,
    /// the samples with a value it took
    pub n: usize,
}

/// why a series gives no value over a span
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesErr {
    /// the device has no service of that name with series
    NoService,
    /// the service has no series of that name
    NoSeries,
    /// no sample of the span had a value
    NoValues,
    /// the latency window no longer holds the span (it keeps five minutes)
    Gone,
}

/// the KPI tiles of a service over a span (Engine::kpi); None where the service has no such
/// number or the span no sample
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kpi {
    pub dev: u32,
    pub svc: String,
    pub kind: &'static str,
    /// requests (connections, messages) per second, in thousandths
    pub rps_milli: Option<i64>,
    /// the latency of the span (µs)
    pub p50_us: Option<i64>,
    pub p95_us: Option<i64>,
    pub p99_us: Option<i64>,
    /// errors per thousand requests
    pub err_pm: Option<i64>,
    /// waiting work (queued requests, a queue's depth) and work in flight, at the newest sample
    pub queue: Option<i64>,
    pub inflight: Option<i64>,
    /// a cache's hits per thousand GET and HEAD
    pub hit_pm: Option<i64>,
    /// a KV node's load: its own requests and the replica reads and writes it serves per
    /// second, in thousandths
    pub load_milli: Option<i64>,
    /// keys and bytes it holds, at the newest sample
    pub keys: Option<i64>,
    pub bytes: Option<i64>,
}

/// the load of a KV node (Engine::shard_load)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardLoad {
    pub dev: u32,
    pub svc: String,
    /// requests and replica operations per second, in thousandths
    pub load_milli: i64,
    /// its share of the load of all KV nodes, per mille
    pub share_pm: i64,
    pub keys: i64,
    pub bytes: i64,
}

/// which metrics make the KPI tiles of a kind of service
struct Roles {
    /// counters of the requests (connections, messages) it handled
    req: &'static [&'static str],
    /// counters of those that failed
    err: &'static [&'static str],
    /// gauges of the waiting work and of the work in flight
    queue: Option<&'static str>,
    inflight: Option<&'static str>,
}

fn roles(kind: &str) -> Roles {
    let r = |req, err, queue, inflight| Roles { req, err, queue, inflight };
    match kind {
        "http-server" => r(&["requests"], &["status_5xx"], Some("queued"), Some("inflight")),
        "http-load" => r(&["responses", "failed"], &["errors"], None, Some("inflight")),
        "http-client" => r(&["responses"], &["errors"], None, None),
        "lb" => r(&["requests", "connections"], &["errors"], None, Some("active")),
        "kv" => r(&["requests"], &["unavailable", "full"], None, None),
        "cache" => r(&["requests"], &["backend_errors"], None, None),
        "mq" => r(&["published"], &["rejected", "dead"], Some("depth"), Some("inflight")),
        "worker" => r(&["done", "failed"], &["failed"], None, Some("busy")),
        "dns-server" => r(&["queries"], &["servfail"], None, None),
        _ => r(&["requests"], &["errors"], Some("queued"), Some("inflight")),
    }
}

/// a field of the CSV, quoted when it must be
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { String::from(s) }
}

// ---------------------------------------------------------------- sampling

impl Engine {
    /// takes the samples due up to time `t` (Engine::step before an event at t, run_until
    /// when the clock stands at t)
    pub(crate) fn sample_until(&mut self, t: Ns) {
        if t < self.series.next {
            return;
        }
        let mut s = core::mem::take(&mut self.series);
        while s.next <= t {
            let at = s.next;
            self.sample(&mut s, at);
            s.next = at + s.cfg.every;
            // nothing happened between the samples still due: those that would fall out of the
            // kept points at once are not taken
            if s.next <= t {
                let due = (t - s.next) / s.cfg.every + 1;
                if due > s.cfg.keep as u64 {
                    s.next += (due - s.cfg.keep as u64) * s.cfg.every;
                }
            }
        }
        self.series = s;
    }

    /// one sample at `at`
    fn sample(&self, s: &mut Sampler, at: Ns) {
        let base = s.taken == 0;
        s.taken += 1;
        let stamp = s.taken;
        let (keep, from) = (s.cfg.keep, at.saturating_sub(s.cfg.every));
        for (&dev, &ni) in &self.dev_idx {
            let Some(os) = self.os(ni) else {
                continue;
            };
            if !os.svcs.iter().any(|x| !x.console && x.svc.is_some()) {
                continue;
            }
            let recs = s.svcs.entry(dev).or_default();
            for slot in os.svcs.iter().filter(|x| !x.console) {
                let Some(svc) = slot.svc.as_deref() else {
                    continue;
                };
                let ri = match recs.iter().position(|r| r.name == slot.name) {
                    Some(i) => i,
                    None => {
                        recs.push(Rec { id: slot.id, name: slot.name.clone(), kind: slot.kind, stamp, ser: Vec::new() });
                        recs.len() - 1
                    }
                };
                let rec = &mut recs[ri];
                if rec.id != slot.id {
                    // another service under the name: it counts from 0
                    rec.id = slot.id;
                    rec.kind = slot.kind;
                    for x in rec.ser.iter_mut() {
                        x.last = Some(0);
                    }
                }
                rec.stamp = stamp;
                sample_service(rec, svc, from, at, stamp, base, keep);
            }
        }
        // what is gone goes
        for recs in s.svcs.values_mut() {
            recs.retain(|r| r.stamp == stamp);
        }
        s.svcs.retain(|_, v| !v.is_empty());
        if s.cfg.links {
            self.sample_links(s, stamp, base, keep);
        }
        if !base {
            s.times.push_back(at);
            if s.times.len() > keep {
                s.times.pop_front();
            }
            s.version += 1;
        }
    }

    /// the cables: what each end put on it
    fn sample_links(&self, s: &mut Sampler, stamp: u64, base: bool, keep: usize) {
        for (&id, &li) in &self.link_idx {
            let Some(Some(l)) = self.links.get(li) else {
                continue;
            };
            let tx = |end: usize| -> (i64, i64) {
                let (n, p) = l.ends[end];
                match self.nodes.get(n) {
                    Some(Some(node)) => node.ports.get(p as usize).map_or((0, 0), |x| (x.counters.tx_frames as i64, x.counters.tx_bytes as i64)),
                    _ => (0, 0),
                }
            };
            let ((fa, ba), (fb, bb)) = (tx(0), tx(1));
            let fresh = !s.links.contains_key(&id);
            let rec = s.links.entry(id).or_insert_with(|| Rec { id, name: String::new(), kind: "cable", stamp, ser: Vec::new() });
            rec.stamp = stamp;
            let mut hint = 0;
            for (name, v) in [("frames_ab", fa), ("bytes_ab", ba), ("frames_ba", fb), ("bytes_ba", bb)] {
                rec.slot(&mut hint, name, SeriesKind::Rate, base || fresh).put(Some(v), stamp, base, keep);
            }
        }
        s.links.retain(|_, r| r.stamp == stamp);
    }
}

/// a service's metrics and windows at a sample covering [from, to)
fn sample_service(rec: &mut Rec, svc: &dyn Service, from: Ns, to: Ns, stamp: u64, base: bool, keep: usize) {
    let wins = svc.windows();
    let mut hint = 0;
    for w in &wins {
        let h = w.win.hist_between(from, to);
        for &(name, stat) in w.stats {
            let v = lat_of(&h, stat).map(|ns| (ns / 1000) as i64);
            rec.slot(&mut hint, name, SeriesKind::Lat(stat), base).put(v, stamp, base, keep);
        }
    }
    for m in svc.metrics_at(to.saturating_sub(1)) {
        // the windows' series stand for the metrics of their names
        if wins.iter().any(|w| w.stats.iter().any(|x| x.0 == m.name)) {
            continue;
        }
        let kind = match svc.metric_kind(m.name) {
            MetricKind::Counter => SeriesKind::Rate,
            MetricKind::Gauge => SeriesKind::Gauge,
            MetricKind::Skip => continue,
        };
        rec.slot(&mut hint, m.name, kind, base).put(Some(m.value), stamp, base, keep);
    }
    // the series it did not report this time
    for x in rec.ser.iter_mut().filter(|x| x.stamp != stamp) {
        x.miss += 1;
        x.put(None, stamp, base, keep);
    }
    rec.ser.retain(|x| x.miss < keep.max(1));
}

// ---------------------------------------------------------------- what the lab reads

impl Engine {
    pub fn series_cfg(&self) -> SeriesCfg {
        self.series.cfg
    }

    /// a new setting: the series start afresh (the first sample, at the next whole interval,
    /// only takes the counters)
    pub fn set_series_cfg(&mut self, cfg: SeriesCfg) {
        let every = cfg.every.max(1).div_ceil(SEC) * SEC;
        let cfg = SeriesCfg { every, keep: cfg.keep.min(KEEP_MAX), links: cfg.links };
        let first = if self.events_done() == 0 && self.now == 0 { 0 } else { (self.now / every + 1) * every };
        let version = self.series.version + 1;
        self.series = Sampler::new(cfg, first);
        self.series.version = version;
    }

    /// changes whenever points arrive or the series start afresh
    pub fn series_gen(&self) -> u64 {
        self.series.version
    }

    /// the moments of the kept samples, oldest first
    pub fn series_times(&self) -> Vec<Ns> {
        self.series.times.iter().copied().collect()
    }

    /// the services that have series, by device and in the order of the device
    pub fn series_services(&self) -> Vec<SvcSeries> {
        let mut out = Vec::new();
        for (&dev, recs) in &self.series.svcs {
            for r in recs {
                out.push(SvcSeries { dev, svc: r.name.clone(), kind: r.kind, series: r.ser.iter().map(|x| (x.name, x.kind, x.unit)).collect() });
            }
        }
        out
    }

    /// every series of a service
    pub fn series(&self, dev: u32, svc: &str) -> Vec<SeriesView> {
        let s = &self.series;
        s.rec(dev, svc).map(|r| r.ser.iter().map(|x| s.view(x)).collect()).unwrap_or_default()
    }

    /// one series of a service
    pub fn series_one(&self, dev: u32, svc: &str, name: &str) -> Option<SeriesView> {
        let s = &self.series;
        s.rec(dev, svc)?.find(name).map(|x| s.view(x))
    }

    /// a cable's series (project link id)
    pub fn cable_series(&self, link: u32) -> Vec<SeriesView> {
        let s = &self.series;
        s.links.get(&link).map(|r| r.ser.iter().map(|x| s.view(x)).collect()).unwrap_or_default()
    }

    /// one value of a series of the samples taken in (from, to]: `agg` Natural gives a rate
    /// over the span, a gauge's mean, a latency's quantile (mean, maximum) of everything the
    /// service's window holds for the span; Min, Max and Mean go over the samples' values
    pub fn series_value(&self, dev: u32, svc: &str, name: &str, agg: Agg, from: Ns, to: Ns) -> Result<AggOut, SeriesErr> {
        let s = &self.series;
        let rec = s.rec(dev, svc).ok_or(SeriesErr::NoService)?;
        let x = rec.find(name).ok_or(SeriesErr::NoSeries)?;
        let every = s.cfg.every.max(1);
        let picked: Vec<(Ns, i64)> = s.span(x, from, to).collect();
        let vals: Vec<i128> = picked.iter().filter(|p| p.1 != NONE).map(|p| p.1 as i128).collect();
        let (Some(first), Some(last)) = (picked.first(), picked.last()) else {
            return Err(SeriesErr::NoValues);
        };
        if vals.is_empty() {
            return Err(SeriesErr::NoValues);
        }
        let n = vals.len() as i128;
        // a kept value in thousandths of the series' unit
        let milli = |v: i128| match x.kind {
            SeriesKind::Rate => v * 1000 * SEC as i128 / every as i128,
            _ => v * 1000,
        };
        let v = match (agg, x.kind) {
            (Agg::Natural, SeriesKind::Lat(stat)) => {
                let ni = self.node_index(dev).ok_or(SeriesErr::NoService)?;
                let obj = self.os(ni).and_then(|o| o.svc(svc)).and_then(|sl| sl.svc.as_deref()).ok_or(SeriesErr::NoService)?;
                let wins = obj.windows();
                let w = wins.iter().find(|w| w.stats.iter().any(|st| st.0 == name)).ok_or(SeriesErr::Gone)?;
                let (lo, hi) = (first.0.saturating_sub(every), last.0);
                let span = w.win.span();
                let oldest = (self.now / span + 1).saturating_sub(w.win.slots() as u64) * span;
                if lo < oldest {
                    return Err(SeriesErr::Gone);
                }
                // ns are thousandths of µs
                lat_of(&w.win.hist_between(lo, hi), stat).ok_or(SeriesErr::NoValues)? as i128
            }
            (Agg::Natural | Agg::Mean, _) => milli(vals.iter().sum::<i128>()) / n,
            (Agg::Min, _) => milli(vals.iter().copied().min().unwrap_or(0)),
            (Agg::Max, _) => milli(vals.iter().copied().max().unwrap_or(0)),
        };
        Ok(AggOut { milli: v.clamp(i64::MIN as i128 + 1, i64::MAX as i128) as i64, n: vals.len() })
    }

    /// the KPI tiles of a service over the samples of the last `span`
    pub fn kpi(&self, dev: u32, svc: &str, span: Ns) -> Option<Kpi> {
        let s = &self.series;
        let rec = s.rec(dev, svc)?;
        let (from, to) = (self.now.saturating_sub(span), self.now);
        let r = roles(rec.kind);
        let rates = |names: &[&str]| -> Option<i64> {
            let mut out = None;
            for x in names.iter().filter_map(|n| rec.find(n)).filter(|x| x.kind == SeriesKind::Rate) {
                if let Some(v) = s.rate_milli(x, from, to) {
                    out = Some(out.unwrap_or(0) + v);
                }
            }
            out
        };
        let per_mille = |part: Option<i128>, whole: Option<i128>| match (part, whole) {
            (Some(p), Some(w)) if w > 0 => Some((p * 1000 / w) as i64),
            _ => None,
        };
        let lat = |name: &str| self.series_value(dev, &rec.name, name, Agg::Natural, from, to).ok().map(|v| v.milli / 1000);
        let gauge = |name: Option<&str>| name.and_then(|n| s.last_in(rec, n, from, to));
        let kv = rec.kind == "kv";
        Some(Kpi {
            dev,
            svc: rec.name.clone(),
            kind: rec.kind,
            rps_milli: rates(r.req),
            p50_us: lat("p50_us"),
            p95_us: lat("p95_us"),
            p99_us: lat("p99_us"),
            err_pm: per_mille(s.change(rec, r.err, from, to), s.change(rec, r.req, from, to)),
            queue: gauge(r.queue),
            inflight: gauge(r.inflight),
            hit_pm: if rec.kind == "cache" { per_mille(s.change(rec, &["hits"], from, to), s.change(rec, &["hits", "misses"], from, to)) } else { None },
            load_milli: if kv { rates(&["requests", "replica_reads", "replica_writes"]) } else { None },
            keys: gauge(Some("keys")),
            bytes: if rec.find("bytes").is_some_and(|x| x.kind == SeriesKind::Gauge) { gauge(Some("bytes")) } else { None },
        })
    }

    /// the KPI tiles of every service with series
    pub fn kpis(&self, span: Ns) -> Vec<Kpi> {
        self.series_services().iter().filter_map(|x| self.kpi(x.dev, &x.svc, span)).collect()
    }

    /// the load of every KV node over the last `span` and its share of all of them
    pub fn shard_load(&self, span: Ns) -> Vec<ShardLoad> {
        let kv: Vec<Kpi> = self.kpis(span).into_iter().filter(|k| k.kind == "kv").collect();
        let total: i64 = kv.iter().filter_map(|k| k.load_milli).sum();
        kv.into_iter()
            .map(|k| {
                let load = k.load_milli.unwrap_or(0);
                ShardLoad { dev: k.dev, svc: k.svc, load_milli: load, share_pm: if total > 0 { (load as i128 * 1000 / total as i128) as i64 } else { 0 }, keys: k.keys.unwrap_or(0), bytes: k.bytes.unwrap_or(0) }
            })
            .collect()
    }

    /// the faults that overlap [from, to] (an active one lasts on), oldest first
    pub fn chart_marks(&self, from: Ns, to: Ns) -> Vec<&FaultMark> {
        self.faults().iter().filter(|m| m.at <= to && m.healed.unwrap_or(Ns::MAX) >= from).collect()
    }

    /// "R1 Gi0/0/1 – SW1 Gi1/0/1"
    fn cable_label(&self, link: u32) -> String {
        let Some(l) = self.link_rt(link) else {
            return format!("Kabel {}", link);
        };
        let end = |(n, p): (usize, u16)| match self.nodes.get(n) {
            Some(Some(node)) => format!("{} {}", node.name, ifname::short(node.port_name(p))),
            _ => String::from("?"),
        };
        format!("{} \u{2013} {}", end(l.ends[0]), end(l.ends[1]))
    }

    /// the columns of the CSV: (header, series)
    fn csv_columns(&self) -> Vec<(String, &Ser)> {
        let s = &self.series;
        let mut cols = Vec::new();
        for (&dev, recs) in &s.svcs {
            let dn = self.node(dev).map(|n| n.name.clone()).unwrap_or_else(|| format!("{}", dev));
            for r in recs {
                for x in &r.ser {
                    cols.push((format!("{} {} {}", dn, r.name, x.name), x));
                }
            }
        }
        for (&id, r) in &s.links {
            let label = self.cable_label(id);
            for x in &r.ser {
                cols.push((format!("{} {}", label, x.name), x));
            }
        }
        cols
    }

    /// every kept point as CSV: the time in seconds, then a column per series ("LB1 lb
    /// requests [/s]", the cables' after the services'); a row per sample, empty where a
    /// series had no value
    pub fn series_csv(&self) -> String {
        let s = &self.series;
        let cols = self.csv_columns();
        let mut out = String::from("t [s]");
        for (name, x) in &cols {
            out.push(',');
            let u = x.unit.as_str();
            out.push_str(&csv_field(&if u.is_empty() { name.clone() } else { format!("{} [{}]", name, u) }));
        }
        out.push('\n');
        let n = s.times.len();
        for (i, &t) in s.times.iter().enumerate() {
            out.push_str(&format!("{}", t / SEC));
            for (_, x) in &cols {
                out.push(',');
                // the values end with the newest sample
                let back = n - i;
                if back <= x.vals.len() {
                    let v = x.vals[x.vals.len() - back];
                    if v != NONE {
                        out.push_str(&format!("{}", x.value(v, s.cfg.every)));
                    }
                }
            }
            out.push('\n');
        }
        out
    }

    /// how many series there are and how many values they keep
    pub fn series_size(&self) -> (usize, usize) {
        let s = &self.series;
        let cols = self.csv_columns();
        let values = cols.iter().map(|(_, x)| s.samples(x).filter(|v| v.1 != NONE).count()).sum();
        (cols.len(), values)
    }
}
