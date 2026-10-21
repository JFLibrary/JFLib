// seriestest -- checks of the lab's time series (part of --selftest, group "series"): a test
// service that ticks every 100 ms gives exact rates, levels and latencies per second (the first
// sample only takes the counters, a tick on the boundary counts for the next second, the
// latency windows stand for the metrics of their names, a metric left out); values over spans
// (a rate, a level's mean, min and max, the quantiles of a span against those of its seconds,
// a span the window no longer holds); another interval with fewer points, no sampling; a
// stopped service's gaps, a service that comes later, a replaced and a removed one; the cables'
// frames and bytes, a cable plugged in later; the CSV; that sampling changes nothing of a run;
// the scenario syntax and the runner's lines; KPIs, the load of the KV shards and the fault
// marks on the lab of tests/fixtures/lab_shards.jfnet
//
// The ticker's latencies are 2 ms, in its fourth second (3 s to 4 s) 30 ms: equal values come
// out exactly, and the quantiles of a span of 20 times 2 ms and 10 times 30 ms are worked out
// below from the buckets of base::metrics (32 steps per power of two).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use base::metrics::Window;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_model::scenario::{self, Agg, Cmp, Expect, Span, StepKind};
use jfn_sim::node::port_mac;
use jfn_sim::series::{metric_kind_of, LAT};
use jfn_sim::{Action, DevEvent, Engine, Fault, HttpServer, LatStat, LatWindow, Metric, MetricKind, OsApi, SeriesCfg, SeriesErr, SeriesKind, Service, SvcEvent, Unit, MS, SEC};
use netpkt::eth::EthHdr;
use netpkt::Mac;

use crate::selftest::T;

const BUDGET: usize = 50_000_000;
const LAB: &str = include_str!("../tests/fixtures/lab_shards.jfnet");

fn s(x: &str) -> String {
    String::from(x)
}

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: s(port) }
}

/// a test service: a tick every 100 ms from its start, 100 bytes each, with a latency of 2 ms
/// (30 ms in the fourth second)
#[derive(Clone, Debug)]
struct Ticker {
    ticks: u64,
    lat: Window,
}

impl Ticker {
    fn new() -> Ticker {
        Ticker { ticks: 0, lat: Window::new(300, SEC) }
    }
}

impl Service for Ticker {
    fn kind(&self) -> &'static str {
        "ticker"
    }
    fn start(&mut self, os: &mut OsApi) {
        let at = os.now() + 100 * MS;
        os.timer(0, at);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if ev == SvcEvent::Timer(0) {
            let now = os.now();
            self.ticks += 1;
            self.lat.record(now, if now / SEC == 3 { 30 * MS } else { 2 * MS });
            os.timer(0, now + 100 * MS);
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        vec![Metric::new("ticks", self.ticks as i64), Metric::new("bytes_out", (self.ticks * 100) as i64), Metric::new("queued", (self.ticks % 7) as i64), Metric::new("secret", 42), Metric::new("p50_us", 1)]
    }
    fn metric_kind(&self, name: &str) -> MetricKind {
        if name == "secret" { MetricKind::Skip } else { metric_kind_of(name) }
    }
    fn windows(&self) -> Vec<LatWindow<'_>> {
        vec![LatWindow::new(&self.lat, LAT)]
    }
    fn status(&self) -> String {
        String::new()
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// SRV1 alone with a ticker "tick"
fn ticker() -> (Engine, u32) {
    let mut p = Project::new("series", 5);
    let dev = p.add_device("server-1u", 0, 0).unwrap_or(0);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let _ = e.add_service(dev, "tick", Box::new(Ticker::new()), true);
    (e, dev)
}

/// a series' points with the times in seconds
fn pts(e: &Engine, dev: u32, svc: &str, name: &str) -> Vec<(u64, i64)> {
    e.series_one(dev, svc, name).map(|x| x.points.iter().map(|p| (p.0 / SEC, p.1)).collect()).unwrap_or_default()
}

pub fn run(t: &mut T) {
    ticks(t);
    spans(t);
    config(t);
    gaps(t);
    owners(t);
    cables(t);
    unchanged(t);
    syntax(t);
    runner(t);
    lab(t);
}

fn ticks(t: &mut T) {
    let (mut e, dev) = ticker();
    e.run_until(5 * SEC, BUDGET);
    t.eq("times: the first sample only takes the counters", e.series_times(), vec![SEC, 2 * SEC, 3 * SEC, 4 * SEC, 5 * SEC]);
    t.eq("generation", e.series_gen(), 5);
    let svcs = e.series_services();
    t.eq("services", svcs.iter().map(|x| (x.dev, x.svc.as_str(), x.kind)).collect::<Vec<_>>(), vec![(dev, "tick", "ticker")]);
    let lat = |pm| SeriesKind::Lat(LatStat::Quantile(pm));
    let want = vec![("p50_us", lat(500), Unit::Us), ("p95_us", lat(950), Unit::Us), ("p99_us", lat(990), Unit::Us), ("ticks", SeriesKind::Rate, Unit::PerSec), ("bytes_out", SeriesKind::Rate, Unit::BytesPerSec), ("queued", SeriesKind::Gauge, Unit::Count)];
    t.eq("series: the windows first, the skipped metric left out", svcs.first().map(|x| x.series.clone()), Some(want));
    // nine ticks before 1 s (0.1 .. 0.9 s); the one at 1.0 s counts for the next second
    t.eq("rate", pts(&e, dev, "tick", "ticks"), vec![(1, 9), (2, 10), (3, 10), (4, 10), (5, 10)]);
    t.eq("bytes per second", pts(&e, dev, "tick", "bytes_out"), vec![(1, 900), (2, 1000), (3, 1000), (4, 1000), (5, 1000)]);
    t.eq("level at the end of each second", pts(&e, dev, "tick", "queued"), vec![(1, 2), (2, 5), (3, 1), (4, 4), (5, 0)]);
    t.eq("p99 of each second from the window, not the metric", pts(&e, dev, "tick", "p99_us"), vec![(1, 2000), (2, 2000), (3, 2000), (4, 30000), (5, 2000)]);
    t.eq("p50 of each second", pts(&e, dev, "tick", "p50_us"), vec![(1, 2000), (2, 2000), (3, 2000), (4, 30000), (5, 2000)]);
    t.eq("a skipped metric has no series", e.series_one(dev, "tick", "secret"), None);
    t.eq("service names ignore case", e.series(dev, "TICK").len(), 6);
    let csv = e.series_csv();
    let lines: Vec<&str> = csv.lines().collect();
    t.eq("csv head", lines.first().copied(), Some("t [s],SRV1 tick p50_us [\u{b5}s],SRV1 tick p95_us [\u{b5}s],SRV1 tick p99_us [\u{b5}s],SRV1 tick ticks [/s],SRV1 tick bytes_out [B/s],SRV1 tick queued"));
    t.eq("csv rows", lines.get(1..).map(|l| l.to_vec()), Some(vec!["1,2000,2000,2000,9,900,2", "2,2000,2000,2000,10,1000,5", "3,2000,2000,2000,10,1000,1", "4,30000,30000,30000,10,1000,4", "5,2000,2000,2000,10,1000,0"]));
    t.eq("size", e.series_size(), (6, 30));
    t.eq("kinds by name", ["requests", "active", "b3_up", "p99_us", "oldest_ms", "last_status", "rate_mrps", "keys", "member_downs"].map(metric_kind_of), [MetricKind::Counter, MetricKind::Gauge, MetricKind::Gauge, MetricKind::Gauge, MetricKind::Gauge, MetricKind::Gauge, MetricKind::Gauge, MetricKind::Gauge, MetricKind::Counter]);
    t.eq("a web server's busy time counts up", HttpServer::new(80).metric_kind("busy_ms"), MetricKind::Counter);
}

fn spans(t: &mut T) {
    let (mut e, dev) = ticker();
    e.run_until(5 * SEC, BUDGET);
    let v = |name: &str, agg: Agg, from: u64, to: u64| e.series_value(dev, "tick", name, agg, from * SEC, to * SEC).map(|x| x.milli);
    t.eq("rate over the last 4 s", v("ticks", Agg::Natural, 1, 5), Ok(10_000));
    t.eq("rate over 5 s: 49 ticks", v("ticks", Agg::Natural, 0, 5), Ok(9_800));
    t.eq("rate between 0 and 2 s", v("ticks", Agg::Natural, 0, 2), Ok(9_500));
    t.eq("min and max of a rate", (v("ticks", Agg::Min, 0, 5), v("ticks", Agg::Max, 0, 5)), (Ok(9_000), Ok(10_000)));
    t.eq("a level's mean", (v("queued", Agg::Natural, 0, 5), v("queued", Agg::Mean, 0, 5)), (Ok(2_400), Ok(2_400)));
    t.eq("a level's min and max", (v("queued", Agg::Min, 0, 5), v("queued", Agg::Max, 0, 5)), (Ok(0), Ok(5_000)));
    // 2 .. 5 s: 20 latencies of 2 ms and 10 of 30 ms; the p99 (rank 30) and the p95 (rank 29)
    // are the largest value, the median (rank 15) the middle of 2 ms's bucket, 1 998 848 ns
    // and 32 768 wide (thousandths of a µs are ns)
    t.eq("p99 of the span", v("p99_us", Agg::Natural, 2, 5), Ok(30_000_000));
    t.eq("p95 of the span", v("p95_us", Agg::Natural, 2, 5), Ok(30_000_000));
    t.eq("median of the span", v("p50_us", Agg::Natural, 2, 5), Ok(2_015_231));
    t.eq("max and mean of the seconds' p99", (v("p99_us", Agg::Max, 2, 5), v("p99_us", Agg::Mean, 2, 5)), (Ok(30_000_000), Ok(11_333_333)));
    t.eq("samples with a value", e.series_value(dev, "tick", "ticks", Agg::Natural, SEC, 5 * SEC).map(|x| x.n), Ok(4));
    t.eq("no such series", v("nope", Agg::Natural, 0, 5), Err(SeriesErr::NoSeries));
    t.eq("no such service", e.series_value(dev, "tock", "ticks", Agg::Natural, 0, 5 * SEC).map(|x| x.milli), Err(SeriesErr::NoService));
    t.eq("no sample in the span", v("ticks", Agg::Natural, 10, 12), Err(SeriesErr::NoValues));
}

fn config(t: &mut T) {
    let (mut e, dev) = ticker();
    e.run_until(5 * SEC, BUDGET);
    e.set_series_cfg(SeriesCfg { every: 1500 * MS, keep: 3, links: false });
    t.eq("cfg: whole seconds", e.series_cfg(), SeriesCfg { every: 2 * SEC, keep: 3, links: false });
    t.eq("cfg: a fresh start", (e.series_times(), e.series_gen()), (Vec::new(), 6));
    // the first sample (6 s) takes the counters; points at 8, 10, 12 and 14 s, three kept
    e.run_until(14 * SEC, BUDGET);
    t.eq("cfg: every 2 s, three kept", e.series_times(), vec![10 * SEC, 12 * SEC, 14 * SEC]);
    t.eq("cfg: rates per second", pts(&e, dev, "tick", "ticks"), vec![(10, 10), (12, 10), (14, 10)]);
    t.eq("cfg: latency of two seconds", pts(&e, dev, "tick", "p99_us"), vec![(10, 2000), (12, 2000), (14, 2000)]);
    t.eq("cfg: generation", e.series_gen(), 10);
    t.eq("cfg: csv rows", e.series_csv().lines().count(), 4);
    e.set_series_cfg(SeriesCfg { keep: 0, ..SeriesCfg::default() });
    e.run_until(20 * SEC, BUDGET);
    t.eq("cfg: no sampling", (e.series_times().len(), e.series_gen(), e.series_services().len()), (0, 11, 0));
    // more points than the latency windows hold
    let (mut e2, d2) = ticker();
    e2.set_series_cfg(SeriesCfg { keep: 600, ..SeriesCfg::default() });
    e2.run_until(400 * SEC, BUDGET);
    t.eq("cfg: 400 points kept", e2.series_times().len(), 400);
    t.eq("cfg: a span the window no longer holds", e2.series_value(d2, "tick", "p99_us", Agg::Natural, 2 * SEC, 5 * SEC).map(|x| x.milli), Err(SeriesErr::Gone));
    t.eq("cfg: its seconds are still there", e2.series_value(d2, "tick", "p99_us", Agg::Max, 2 * SEC, 5 * SEC).map(|x| x.milli), Ok(30_000_000));
}

fn gaps(t: &mut T) {
    let (mut e, dev) = ticker();
    // stopped after its tick at 2.0 s, started again at 4.0 s (first tick at 4.1 s)
    e.run_until(2 * SEC, BUDGET);
    let _ = e.set_service_running(dev, "tick", false);
    e.run_until(4 * SEC, BUDGET);
    let _ = e.set_service_running(dev, "tick", true);
    e.run_until(6 * SEC, BUDGET);
    t.eq("stopped: no ticks", pts(&e, dev, "tick", "ticks"), vec![(1, 9), (2, 10), (3, 1), (4, 0), (5, 9), (6, 10)]);
    t.eq("stopped: a gap in the latencies", pts(&e, dev, "tick", "p99_us").iter().map(|p| p.0).collect::<Vec<_>>(), vec![1, 2, 3, 5, 6]);
    t.eq("stopped: the level stays", pts(&e, dev, "tick", "queued"), vec![(1, 2), (2, 5), (3, 6), (4, 6), (5, 1), (6, 4)]);
}

fn owners(t: &mut T) {
    let (mut e, dev) = ticker();
    e.run_until(2500 * MS, BUDGET);
    let _ = e.add_service(dev, "tock", Box::new(Ticker::new()), true);
    e.run_until(4 * SEC, BUDGET);
    // ticks at 2.6 .. 2.9 s, then ten a second
    t.eq("a later service counts from 0", pts(&e, dev, "tock", "ticks"), vec![(3, 4), (4, 10)]);
    t.eq("two services", e.series_services().iter().map(|x| x.svc.as_str()).collect::<Vec<_>>(), vec!["tick", "tock"]);
    e.run_until(4500 * MS, BUDGET);
    // "tick" replaced at 4.5 s: the new one ticks at 4.6 .. 4.9 s and counts from 0
    let _ = e.add_service(dev, "tick", Box::new(Ticker::new()), true);
    let _ = e.remove_service(dev, "tock");
    e.run_until(6 * SEC, BUDGET);
    t.eq("a replaced service counts from 0", pts(&e, dev, "tick", "ticks"), vec![(1, 9), (2, 10), (3, 10), (4, 10), (5, 4), (6, 10)]);
    t.eq("a removed service goes", e.series_services().iter().map(|x| x.svc.as_str()).collect::<Vec<_>>(), vec!["tick"]);
}

/// a cable's series with the times in seconds
fn cable_pts(e: &Engine, link: u32) -> Vec<(&'static str, Vec<(u64, i64)>)> {
    e.cable_series(link).iter().map(|x| (x.name, x.points.iter().map(|p| (p.0 / SEC, p.1)).collect())).collect()
}

fn cables(t: &mut T) {
    let mut p = Project::new("cables", 5);
    let sw = p.add_device("jf-s24p", 0, 100).unwrap_or(0);
    let a = p.add_device("server-1u", 0, 0).unwrap_or(0);
    let b = p.add_device("server-1u", 100, 0).unwrap_or(0);
    let la = p.connect(&pr(a, "eth0"), &pr(sw, "gi1/0/1"), None).unwrap_or(0);
    let lb = p.connect(&pr(b, "eth0"), &pr(sw, "gi1/0/2"), None).unwrap_or(0);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    e.run_until(500 * MS, BUDGET);
    // a broadcast from SRV1 at 0.5 s: SRV1's cable towards the switch, flooded to SRV2
    let f = EthHdr { dst: Mac::BROADCAST, src: port_mac(a, 0), ethertype: 0x88B5 }.frame(&[0u8; 50]);
    let n = f.len() as i64;
    t.check("cables: injected", e.apply(Action::Inject { dev: a, port: s("eth0"), frame: f }).is_ok());
    e.run_until(2 * SEC, BUDGET);
    t.eq("cables: SRV1's", cable_pts(&e, la), vec![("frames_ab", vec![(1, 1), (2, 0)]), ("bytes_ab", vec![(1, n), (2, 0)]), ("frames_ba", vec![(1, 0), (2, 0)]), ("bytes_ba", vec![(1, 0), (2, 0)])]);
    t.eq("cables: SRV2's from the switch", cable_pts(&e, lb).get(2).cloned(), Some(("frames_ba", vec![(1, 1), (2, 0)])));
    // a cable plugged in at 2.5 s: the first sample that sees it only takes its counters
    let lc = p.connect(&pr(a, "eth1"), &pr(sw, "gi1/0/3"), None).unwrap_or(0);
    e.run_until(2500 * MS, BUDGET);
    t.check("cables: plugged in", p.link(lc).cloned().is_some_and(|l| e.apply(Action::Connect(l)).is_ok()));
    e.run_until(4 * SEC, BUDGET);
    t.eq("cables: a new one from its second sample", cable_pts(&e, lc).first().cloned(), Some(("frames_ab", vec![(4, 0)])));
    let csv = e.series_csv();
    t.check("cables: in the csv", csv.lines().next().is_some_and(|h| h.contains(",SRV1 eth0 \u{2013} SW1 Gi1/0/1 frames_ab [/s],") && h.contains(",SRV2 eth0 \u{2013} SW1 Gi1/0/2 bytes_ba [B/s]")));
    e.set_series_cfg(SeriesCfg { links: false, ..SeriesCfg::default() });
    e.run_until(6 * SEC, BUDGET);
    t.eq("cables: not without links", (e.cable_series(la).len(), e.series_times().len()), (0, 1));
}

/// what a run leaves in the trace
type Run = (Vec<(u64, u64, u64, u32, Vec<u8>)>, Vec<DevEvent>, u64, usize, Option<i64>, u64);

fn lab_run(keep: usize, until: u64) -> Run {
    let p = Project::from_text(LAB).unwrap_or_else(|_| Project::new("lab", 1));
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    e.set_series_cfg(SeriesCfg { keep, ..SeriesCfg::default() });
    e.run_until(until, BUDGET);
    let lb = p.find("LB1").map(|d| d.id).unwrap_or(0);
    let hops = e.trace.hops().map(|h| (h.id, h.t_tx, h.t_rx, h.link, h.bytes.to_vec())).collect();
    (hops, e.trace.events().cloned().collect(), e.events_done(), e.pending(), e.metric(lb, "lb", "requests"), e.series_gen())
}

fn unchanged(t: &mut T) {
    let (a, b) = (lab_run(300, 8 * SEC), lab_run(0, 8 * SEC));
    t.check("sampling: the same frames", !a.0.is_empty() && a.0 == b.0);
    t.check("sampling: the same events", !a.1.is_empty() && a.1 == b.1);
    t.eq("sampling: the same counts", (a.2, a.3, a.4), (b.2, b.3, b.4));
    t.eq("sampling: samples only with it", (a.5, b.5), (9, 1));
}

fn syntax(t: &mut T) {
    let text = "expect LB1 svc lb metric requests >= 99.5 over 5s\nexpect LB1 svc lb metric max p99_us < 5000 between 14s and 19s\nexpect KV1 svc kv metric min members_up == 3 over 1500ms\nexpect KV1 svc kv metric mean keys != -2 between 0s and 1s\nseries build/run.csv\n";
    let x = |dev: &str, svc: &str, agg, name: &str, cmp, milli, span| StepKind::Expect(Expect::Series { dev: s(dev), svc: s(svc), name: s(name), agg, cmp, milli, span });
    match scenario::parse(text) {
        Ok(sc) => {
            let k: Vec<&StepKind> = sc.steps.iter().map(|x| &x.kind).collect();
            t.eq("syntax: steps", k.len(), 5);
            t.eq("syntax: over", k[0], &x("LB1", "lb", Agg::Natural, "requests", Cmp::Ge, 99_500, Span::Last(5 * SEC)));
            t.eq("syntax: max between", k[1], &x("LB1", "lb", Agg::Max, "p99_us", Cmp::Lt, 5_000_000, Span::Between(14 * SEC, 19 * SEC)));
            t.eq("syntax: min", k[2], &x("KV1", "kv", Agg::Min, "members_up", Cmp::Eq, 3_000, Span::Last(1500 * MS)));
            t.eq("syntax: mean, below 0", k[3], &x("KV1", "kv", Agg::Mean, "keys", Cmp::Ne, -2_000, Span::Between(0, SEC)));
            t.eq("syntax: series", k[4], &StepKind::Series(s("build/run.csv")));
        }
        Err(e) => t.check(&format!("syntax parses: {:?}", e), false),
    }
    let bad = "expect LB1 svc lb metric avg requests > 1 over 5s\nexpect LB1 svc lb metric requests > 1 between 5s and 2s\nexpect LB1 svc lb metric requests > 1 over 0s\nexpect LB1 svc lb metric max requests > 1\nexpect LB1 svc lb metric requests > 1.2345 over 5s\nexpect LB1 svc lb metric requests > 1 over\nseries\n";
    match scenario::parse(bad) {
        Ok(_) => t.check("syntax: errors found", false),
        Err(e) => t.eq("syntax: error lines", e.iter().map(|x| x.line).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5, 6, 7]),
    }
    t.eq("syntax: thousandths", ["99.5", "-2.25", "0.001", "7", "1.2345", "x", "-"].map(scenario::parse_milli), [Some(99_500), Some(-2_250), Some(1), Some(7_000), None, None, None]);
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: s(kind), name: s(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Value::Null) }
}

fn runner(t: &mut T) {
    // SRV1 a web server, SRV2 a load generator: 20 requests a second from 1 s to 4 s
    let mut p = Project::new("series", 9);
    let sw = p.add_device("jf-s24p", 0, 100).unwrap_or(0);
    let a = p.add_device("server-1u", 0, 0).unwrap_or(0);
    let b = p.add_device("server-1u", 100, 0).unwrap_or(0);
    let _ = p.connect(&pr(a, "eth0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(b, "eth0"), &pr(sw, "gi1/0/2"), None);
    for d in p.devices.iter_mut() {
        if d.id == a {
            d.startup_config = s("ip addr add 10.0.0.1/24 dev eth0\n");
            d.services.push(def("http-server", "web", "{\"time_ms\": 2}"));
        } else if d.id == b {
            d.startup_config = s("ip addr add 10.0.0.2/24 dev eth0\n");
            d.services.push(def("http-load", "load", "{\"targets\": [\"http://10.0.0.1/\"], \"mode\": \"open\", \"rps\": 20, \"delay_ms\": 1000, \"duration_ms\": 3000}"));
        }
    }
    let sc = "run 5s\nexpect SRV1 svc web metric requests == 20 between 1s and 4s\nexpect SRV1 svc web metric max inflight <= 1 over 5s\nexpect SRV2 svc load metric p99_us < 10000 between 1s and 4s\nexpect SRV1 svc web metric nope > 0 over 5s\nexpect SRV1 svc nope metric requests > 0 over 1s\nexpect SRV1 svc web metric requests > 0 between 9s and 10s\nexpect SRV1 svc web metric requests > 0 between 4s and 5s\nexpect SRV1 svc web metric p99_us > 0 between 4s and 5s\nseries build/never.csv\n";
    let Ok(sc) = scenario::parse(sc) else {
        t.check("runner: parses", false);
        return;
    };
    let o = jfn_sim::run_scenario(&p, &sc, "series.scn", false);
    let lines: Vec<&str> = o.report.lines().collect();
    let has = |x: &str| lines.iter().any(|l| l.contains(x));
    t.check("runner: a rate", has("ok      expect SRV1 svc web metric requests == 20 between 1s and 4s (20)"));
    t.check("runner: a maximum", has("ok      expect SRV1 svc web metric max inflight <= 1 over 5s ("));
    t.check("runner: a latency of the span", has("ok      expect SRV2 svc load metric p99_us < 10000 between 1s and 4s ("));
    t.check("runner: no such series", has("FEHLER  expect SRV1 svc web metric nope > 0 over 5s (keine solche Reihe)"));
    t.check("runner: no such service", has("FEHLER  expect SRV1 svc nope metric requests > 0 over 1s (keinen solchen Dienst)"));
    t.check("runner: not yet", has("FEHLER  expect SRV1 svc web metric requests > 0 between 9s and 10s (der Zeitraum endet erst nach jetzt)"));
    t.check("runner: measured", has("FEHLER  expect SRV1 svc web metric requests > 0 between 4s and 5s (gemessen: 0)"));
    t.check("runner: no values", has("FEHLER  expect SRV1 svc web metric p99_us > 0 between 4s and 5s (keine Werte im Zeitraum)"));
    t.eq("runner: counted", (o.ok, o.expects, o.failed), (false, 8, 5));
    match &o.series {
        Some((f, csv)) => {
            t.eq("runner: the file as written", f.as_str(), "build/never.csv");
            t.check("runner: the csv", csv.starts_with("t [s],SRV1 web p50_us [\u{b5}s],") && csv.lines().count() == 6);
            t.check("runner: its line", has(&format!("--      series build/never.csv: {} Reihen, {} Werte, CRC-32 {:08x}", csv.lines().next().map_or(0, |h| h.split(',').count() - 1), csv.lines().skip(1).flat_map(|l| l.split(',').skip(1)).filter(|v| !v.is_empty()).count(), netpkt::checksum::crc32(csv.as_bytes()))));
        }
        None => t.check("runner: series", false),
    }
}

fn lab(t: &mut T) {
    let p = Project::from_text(LAB).unwrap_or_else(|_| Project::new("lab", 1));
    let id = |n: &str| p.find(n).map(|d| d.id).unwrap_or(0);
    let (lb, kv1, kv2, kv3, load) = (id("LB1"), id("KV1"), id("KV2"), id("KV3"), id("GEN"));
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    e.run_until(6 * SEC, BUDGET);
    // the numbers below are this run's (the same as tests/scen/lab_metrics.scn): 407 requests
    // from 2 s to 6 s at the balancer
    let k = e.kpi(lb, "lb", 4 * SEC);
    t.eq("lab: the balancer before the fault", k.as_ref().map(|k| (k.rps_milli, k.p99_us, k.err_pm, k.inflight)), Some((Some(101_750), Some(1133), Some(0), Some(0))));
    // requests, replica reads and writes a second; shares of all three; keys
    let shards = e.shard_load(4 * SEC);
    t.eq("lab: shards before", shards.iter().map(|x| (x.dev, x.load_milli, x.share_pm, x.keys)).collect::<Vec<_>>(), vec![(kv1, 72_750, 357, 138), (kv2, 71_250, 350, 148), (kv3, 59_500, 292, 114)]);
    let _ = e.apply(Action::Fault { fault: Fault::Power { dev: kv2 }, dur: Some(6 * SEC) });
    e.run_until(12 * SEC, BUDGET);
    let shards = e.shard_load(5 * SEC);
    t.eq("lab: shards while KV2 is off", shards.iter().map(|x| (x.dev, x.load_milli, x.share_pm)).collect::<Vec<_>>(), vec![(kv1, 75_200, 534), (kv2, 0, 0), (kv3, 65_600, 465)]);
    // the median stays, the p99 of the six seconds is a quarter of a second
    let k = e.kpi(lb, "lb", 6 * SEC);
    t.eq("lab: the balancer during the fault", k.as_ref().map(|k| (k.rps_milli, k.p50_us, k.p99_us, k.err_pm)), Some((Some(104_833), Some(1133), Some(249_561), Some(0))));
    t.eq("lab: the mark", e.chart_marks(0, 12 * SEC).iter().map(|m| (m.id, m.at, m.healed)).collect::<Vec<_>>(), vec![(1, 6 * SEC, Some(12 * SEC))]);
    t.eq("lab: marks of spans", (e.chart_marks(0, 5 * SEC).len(), e.chart_marks(12 * SEC, 13 * SEC).len(), e.chart_marks(13 * SEC, 14 * SEC).len()), (0, 1, 0));
    e.run_until(19 * SEC, BUDGET);
    let all = e.kpis(5 * SEC);
    t.eq("lab: every service", all.iter().map(|k| (k.svc.as_str(), k.kind)).collect::<Vec<_>>(), vec![("fill", "http-load"), ("load", "http-load"), ("lb", "lb"), ("web", "http-server"), ("web", "http-server"), ("web", "http-server"), ("kv", "kv"), ("kv", "kv"), ("kv", "kv")]);
    // 1637 answers in the 17 s from 2 s on, none failed
    let g = e.kpi(load, "load", 17 * SEC);
    t.eq("lab: the load generator over the run", g.as_ref().map(|k| (k.rps_milli, k.err_pm, k.p50_us, k.p99_us)), Some((Some(96_294), Some(0), Some(1141), Some(199_229))));
}
