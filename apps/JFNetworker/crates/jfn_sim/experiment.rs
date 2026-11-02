// experiment -- a parameter sweep on the engine (SA2), the measuring part of the architecture lab:
// every configuration of a jfn_model::experiment::Experiment under every load level, from the same
// warm state, at full speed on virtual time
//
//   Plan::new(&experiment, &project, name) -> Result<Plan, Vec<Msg>>   what the experiment asks of
//                                      the project checked and every configuration patched
//   plan.warm(config, seed) -> Result<Engine, Msg>   boots a copy once, quiet trace, runs the warm-up
//   plan.measure(&warm, config, seed, level) -> Fig  a clone of it through one level
//   plan.run_level(&mut engine, config, seed, level) -> Fig   the same on an engine of yours
//   plan.run_all() -> Sweep            all of it on this thread (jfn_run's sweep does it on many)
//   plan.empty() / plan.collect(figs) -> Sweep   the results of figs[plan.index(config, seed, level)]
//   Sweep                              configs (ConfigOut: label, cost, knee, maximum, limit), points
//                                      (Point), table() / csv() / detail_csv(); the same bytes on
//                                      every machine and for any order of the runs
//
// How a level runs: the warm engine is cloned (Engine: Clone, nothing shared but frames that never
// change), the load generator takes the level through Action::HttpLoadSet, settle_s pass, then
// measure_s are measured.  What is measured comes from the generator's latency window for exactly that
// span and from differences of counters, so a level costs no event of its own: the clock is only
// brought to whole seconds, where the disks' backlog is read and the footprint looked at.  A level that
// grows beyond MEM_LIMIT ends where it is (Status::Memory): an overloaded generator without max_conns
// opens a connection per request.
//
// Results, for a point of a configuration and a level over its seeds: the latency histograms and the
// counters are added up and the quantiles taken of the sum (p99_min and p99_max are the seeds' own, for
// the spread), figures of devices are means or maxima.  Latencies are of the answers: a request that
// timed out has none and counts as an error, so read p99 together with the errors.  Offered is the
// rate the level asks (closed loop: what the users sent), achieved the answers without errors.
//
// The knee of a configuration is the highest level up to which every level holds the SLO.  Its limit
// names the most used resource at the first level that does not, else at the last one: the busiest CPU
// (mean of its cores), disk, cable or lock of a server (Point::bottleneck; "-" below 50 %).  The cost is
// catalog::Prices over the devices that are switched on, without the load generator's device and
// without Experiment::cost_exclude: an assumption, and the table says so.
//
// Nothing here reads a clock or names plat; worker threads are jfn_run's business.  Quiet trace:
// Trace::set_quiet (trace.rs) keeps no frames and no decisions, which changes no figure above (the
// selftest checks it).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::metrics::Hist;
use jfn_model::catalog::Prices;
use jfn_model::experiment::{patch_clones, patch_config, patch_hw, patch_load, patch_pool, Config, Experiment, Level, SloLimits};
use jfn_model::msg;
use jfn_model::msg::Msg;
use jfn_model::project::Project;
use jfn_model::scenario::Agg;

use crate::action::Action;
use crate::httpsvc::{HttpLoad, HttpServer};
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::res::HOST;
use crate::series::SeriesCfg;

/// a level that holds more than this ends where it is (keyframe::footprint)
pub const MEM_LIMIT: usize = 1 << 30;
/// a resource below this share of its time is no bottleneck, per mille
const NO_LIMIT_PM: u64 = 500;

/// how a level ended
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Ok,
    /// it held more than MEM_LIMIT and ended where it was
    Memory,
    /// the configuration did not boot
    Failed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Memory => "speicher",
            Status::Failed => "fehler",
        }
    }
}

/// a device over the measured span
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DevFig {
    pub dev: u32,
    pub name: String,
    pub cores: u32,
    /// the part of its cores' time spent, per mille
    pub cpu_pm: u64,
    /// jobs, the mean time one waited for a core (ns), the longest
    pub jobs: u64,
    pub cpu_wait: Ns,
    pub cpu_wait_max: Ns,
    /// jobs waiting for a core: the mean of the seconds in thousandths, the highest second
    pub runq_milli: u64,
    pub runq_max: u64,
    /// held in RAM and in the swap at the highest second (bytes)
    pub mem: u64,
    pub swap: u64,
    pub pageins: u64,
    pub oom_kills: u64,
    /// the part of the time its disk's channel was busy, per mille; the longest wait for it; its
    /// backlog at the worst whole second
    pub disk_pm: u64,
    pub disk_wait_max: Ns,
    pub disk_backlog_max: Ns,
    pub disk_bytes: u64,
}

/// a cable over the measured span: its busier direction
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkFig {
    pub link: u32,
    pub name: String,
    pub util_pm: u64,
    pub bytes: u64,
}

/// a service over the measured span (series::Kpi; what it does not have is 0)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SvcFig {
    pub dev: u32,
    pub dev_name: String,
    pub svc: String,
    pub kind: &'static str,
    pub rps_milli: i64,
    pub err_pm: i64,
    pub p50_us: i64,
    pub p95_us: i64,
    pub p99_us: i64,
    pub queue: i64,
    pub inflight: i64,
    pub hit_pm: i64,
    /// a KV node: its own requests and the replica reads and writes it serves per second, in thousandths
    pub load_milli: i64,
    /// a web server: the part of the time its lock was held, per mille
    pub lock_pm: u64,
}

/// one level of one configuration with one seed
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fig {
    pub sent: u64,
    pub responses: u64,
    pub ok: u64,
    pub http_errors: u64,
    pub failed: u64,
    pub timeouts: u64,
    pub hist: Hist,
    pub devs: Vec<DevFig>,
    pub links: Vec<LinkFig>,
    pub svcs: Vec<SvcFig>,
    pub status: Status,
    pub seconds: u64,
    /// events the level ran
    pub events: u64,
}

/// a configuration with what it does to the project
#[derive(Clone, Debug, PartialEq)]
pub struct Cfg {
    pub config: Config,
    pub project: Project,
    /// euros per hour in millionths (an assumption)
    pub cost_uh: u64,
}

/// a sweep ready to run
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub exp: Experiment,
    /// the file's name, for the head of the table
    pub name: String,
    pub cfgs: Vec<Cfg>,
    pub levels: Vec<Level>,
    pub seeds: Vec<u64>,
    pub slo: SloLimits,
    pub prices: Prices,
    /// bytes a level may hold (MEM_LIMIT)
    pub mem_limit: usize,
    /// a quiet trace (Trace::set_quiet), as a sweep has it; off only to compare
    pub quiet: bool,
    gen_dev: u32,
    keep: usize,
}

/// a host's address for the load balancer at `lb`: on a network they share, else its first
fn addr_of(e: &Engine, host: u32, lb: Option<u32>) -> Option<String> {
    let ifs = |d: u32| e.stack(e.node_index(d)?).map(|s| s.ifaces.iter().filter(|f| !f.hidden && !f.is_loopback()).flat_map(|f| f.addrs.iter().copied()).collect::<Vec<_>>());
    let mine = ifs(host)?;
    let near = lb.and_then(ifs).unwrap_or_default();
    let pick = mine.iter().find(|a| near.iter().any(|b| a.trunc() == b.trunc())).or(mine.first())?;
    Some(format!("{}", pick.addr))
}

impl Plan {
    /// the experiment against the project: every finding, or the configurations patched
    pub fn new(exp: &Experiment, base: &Project, name: &str) -> Result<Plan, Vec<Msg>> {
        let found = exp.problems(base);
        if !found.is_empty() {
            return Err(found);
        }
        let boot = Engine::from_project(base).map_err(|e| alloc::vec![msg!("Das Projekt läuft nicht: {}", e.msg())])?;
        let prices = exp.prices_of();
        let gen_dev = base.find(&exp.load.dev).map_or(0, |d| d.id);
        let t = &exp.timing;
        let mut errs = Vec::new();
        let mut cfgs = Vec::new();
        for config in exp.configs() {
            match patched(base, &boot, exp, &config) {
                Ok(project) => {
                    let cost_uh = project.devices.iter().filter(|d| !d.off && d.id != gen_dev && !exp.cost_exclude.iter().any(|n| n.eq_ignore_ascii_case(&d.name))).map(|d| prices.of_device(&d.model, d.hw.as_ref())).sum();
                    cfgs.push(Cfg { config, project, cost_uh });
                }
                Err(e) => errs.push(msg!("Konfiguration {}: {}", config.label(), e)),
            }
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        let keep = ((t.warmup_s + t.settle_s + t.measure_s + 4) as usize).clamp(300, crate::series::KEEP_MAX);
        Ok(Plan { exp: exp.clone(), name: String::from(name), cfgs, levels: exp.levels(), seeds: exp.seeds_or(base.seed), slo: exp.slo.limits(), prices, mem_limit: MEM_LIMIT, quiet: true, gen_dev, keep })
    }

    /// the runs of a sweep in the order index() gives them
    pub fn runs(&self) -> usize {
        self.cfgs.len() * self.seeds.len() * self.levels.len()
    }

    /// where a run's result goes in collect()
    pub fn index(&self, config: usize, seed: usize, level: usize) -> usize {
        (config * self.seeds.len() + seed) * self.levels.len() + level
    }

    /// a configuration with a seed: booted once, quiet, warmed up
    pub fn warm(&self, config: usize, seed: usize) -> Result<Engine, Msg> {
        let mut p = self.cfgs[config].project.clone();
        p.seed = self.seeds[seed];
        let mut e = Engine::from_project(&p).map_err(|x| x.msg())?;
        if self.quiet {
            e.trace.set_quiet();
        }
        // the cables come from their counters: no series of their own
        e.set_series_cfg(SeriesCfg { every: SEC, keep: self.keep, links: false });
        e.run_until(self.exp.timing.warmup_s * SEC, usize::MAX);
        Ok(e)
    }

    /// a clone of the warm engine through one level
    pub fn measure(&self, warm: &Engine, config: usize, seed: usize, level: usize) -> Fig {
        let mut e = warm.clone();
        self.run_level(&mut e, config, seed, level)
    }

    /// one level on this engine: its load, settling, measuring
    pub fn run_level(&self, e: &mut Engine, _config: usize, _seed: usize, level: usize) -> Fig {
        let t = &self.exp.timing;
        let svc = self.exp.load.svc.clone();
        let (rate_m, users) = match self.levels[level] {
            Level::Rps(r) => (Some(r * 1000), None),
            Level::Users(u) => (None, Some(u)),
        };
        if e.apply(Action::HttpLoadSet { dev: self.gen_dev, svc: svc.clone(), rate_m, users }).is_err() {
            return Fig { status: Status::Failed, ..Fig::default() };
        }
        e.run_until(e.now() + t.settle_s * SEC, usize::MAX);
        let from = e.now();
        let done0 = e.events_done();
        let before = Snap::of(e, self.gen_dev, &svc);
        // the longest waits count from here (nothing but this reads them)
        e.res_marks_reset();
        let mut backlog: Vec<(u32, Ns)> = Vec::new();
        let mut status = Status::Ok;
        let mut seconds = 0;
        for s in 1..=t.measure_s {
            e.run_until(from + s * SEC, usize::MAX);
            seconds = s;
            e.backlog_peaks(&mut backlog);
            if s % 4 == 0 && crate::keyframe::footprint(e) > self.mem_limit {
                status = Status::Memory;
                break;
            }
        }
        let to = e.now();
        let after = Snap::of(e, self.gen_dev, &svc);
        let hist = e.service::<HttpLoad>(self.gen_dev, &svc).map_or_else(Hist::new, |l| l.lat_between(from, to));
        Fig {
            sent: after.sent - before.sent,
            responses: after.responses - before.responses,
            ok: after.ok - before.ok,
            http_errors: after.http_errors - before.http_errors,
            failed: after.failed - before.failed,
            timeouts: after.timeouts - before.timeouts,
            hist,
            devs: e.fig_devs(&before, &after, &backlog, from, to),
            links: e.fig_links(&before, &after, from, to),
            svcs: e.fig_svcs(&before, &after, from, to),
            status,
            seconds,
            events: e.events_done() - done0,
        }
    }

    /// all of it on this thread
    pub fn run_all(&self) -> Sweep {
        let mut figs = alloc::vec![None; self.runs()];
        for c in 0..self.cfgs.len() {
            for s in 0..self.seeds.len() {
                let warm = self.warm(c, s);
                for l in 0..self.levels.len() {
                    figs[self.index(c, s, l)] = Some(match &warm {
                        Ok(w) => self.measure(w, c, s, l),
                        Err(_) => Fig { status: Status::Failed, ..Fig::default() },
                    });
                }
            }
        }
        self.collect(figs)
    }

    /// the results of a sweep from what its runs gave
    pub fn collect(&self, figs: Vec<Option<Fig>>) -> Sweep {
        let mut configs = Vec::new();
        let mut points = Vec::new();
        for (c, cf) in self.cfgs.iter().enumerate() {
            let from = points.len();
            for l in 0..self.levels.len() {
                let of: Vec<&Fig> = (0..self.seeds.len()).filter_map(|s| figs.get(self.index(c, s, l)).and_then(|f| f.as_ref())).collect();
                points.push(self.point(c, l, cf, &of));
            }
            let mine = &mut points[from..];
            // the knee: every level up to it holds the SLO
            let mut knee = None;
            if self.slo != SloLimits::default() {
                for (l, p) in mine.iter().enumerate() {
                    if !p.slo_ok {
                        break;
                    }
                    knee = Some(l);
                }
                if let Some(k) = knee {
                    mine[k].knee = true;
                }
            }
            // what limits it: at the first level that does not hold, without an SLO at the last one
            let last = mine.len().saturating_sub(1);
            let first_bad = if self.slo == SloLimits::default() { last } else { knee.map_or(0, |k| k + 1).min(last) };
            configs.push(ConfigOut {
                label: cf.config.label(),
                series: cf.config.series.clone(),
                labels: cf.config.labels(),
                cost_uh: cf.cost_uh,
                knee,
                max_achieved_m: mine.iter().map(|p| p.achieved_m).max().unwrap_or(0),
                limit: mine.get(first_bad).map(|p| p.bottleneck.clone()).unwrap_or_default(),
            });
        }
        let mut columns: Vec<String> = Vec::new();
        for c in &configs {
            for (n, _) in &c.labels {
                if !columns.contains(n) {
                    columns.push(n.clone());
                }
            }
        }
        let t = &self.exp.timing;
        Sweep {
            name: self.name.clone(),
            title: self.exp.title.clone(),
            columns,
            configs,
            levels: self.levels.clone(),
            seeds: self.seeds.clone(),
            slo: self.slo,
            warmup_s: t.warmup_s,
            settle_s: t.settle_s,
            measure_s: t.measure_s,
            points,
            details: figs.into_iter().flatten().collect(),
        }
    }

    fn point(&self, c: usize, l: usize, cf: &Cfg, of: &[&Fig]) -> Point {
        let mut h = Hist::new();
        let mut p = Point { config: c, level: l, cost_uh: cf.cost_uh, status: Status::Ok, ..Point::default() };
        let mut secs = 0;
        let mut p99s: Vec<u64> = Vec::new();
        for f in of {
            h.merge(&f.hist);
            p.sent += f.sent;
            p.responses += f.responses;
            p.ok += f.ok;
            p.errors += f.http_errors + f.failed;
            p.failed += f.failed;
            p.timeouts += f.timeouts;
            secs += f.seconds;
            if f.status != Status::Ok {
                p.status = f.status;
            }
            if let Some(q) = f.hist.quantile(990) {
                p99s.push(q);
            }
        }
        p.seconds = secs;
        let n = of.len().max(1) as u64;
        let per_s = |x: u64| if secs == 0 { 0 } else { x * 1000 / secs };
        p.offered_m = match self.levels[l] {
            Level::Rps(r) => r * 1000,
            Level::Users(_) => per_s(p.sent),
        };
        p.achieved_m = per_s(p.ok);
        let finished = p.responses + p.failed;
        p.err_ppm = if finished == 0 { if p.sent > 0 { 1_000_000 } else { 0 } } else { p.errors * 1_000_000 / finished };
        p.p50 = h.quantile(500);
        p.p95 = h.quantile(950);
        p.p99 = h.quantile(990);
        p.p999 = h.quantile(999);
        p.mean = h.mean();
        p.max = h.max();
        p.p99_min = p99s.iter().copied().min();
        p.p99_max = p99s.iter().copied().max();
        p.slo_ok = p.status == Status::Ok && self.slo.held(p.p50, p.p95, p.p99, p.p999, p.err_ppm);
        // the busiest of each kind: the mean over the seeds of a device's figure
        let mut cpu: Vec<(String, u64, u64, u64)> = Vec::new();
        for f in of {
            for d in &f.devs {
                match cpu.iter_mut().find(|x| x.0 == d.name) {
                    Some(x) => {
                        x.1 += d.cpu_pm;
                        x.2 += d.runq_milli;
                        x.3 += d.cpu_wait;
                    }
                    None => cpu.push((d.name.clone(), d.cpu_pm, d.runq_milli, d.cpu_wait)),
                }
                p.mem_max = p.mem_max.max(d.mem);
                p.swap_max = p.swap_max.max(d.swap);
                p.pageins += d.pageins;
                p.oom_kills += d.oom_kills;
                if d.disk_pm > p.disk_max_pm || p.disk_dev.is_empty() {
                    p.disk_max_pm = d.disk_pm.max(p.disk_max_pm);
                    p.disk_dev = d.name.clone();
                }
                p.disk_wait_max = p.disk_wait_max.max(d.disk_wait_max);
                p.disk_backlog_max = p.disk_backlog_max.max(d.disk_backlog_max);
            }
            for x in &f.links {
                if x.util_pm > p.link_max_pm || p.link_name.is_empty() {
                    p.link_max_pm = x.util_pm.max(p.link_max_pm);
                    p.link_name = x.name.clone();
                }
            }
            for s in &f.svcs {
                if s.lock_pm > p.lock_max_pm {
                    p.lock_max_pm = s.lock_pm;
                    p.lock_name = format!("{} {}", s.dev_name, s.svc);
                }
            }
        }
        for (name, pm, q, w) in &cpu {
            if pm / n > p.cpu_max_pm || p.cpu_dev.is_empty() {
                p.cpu_max_pm = pm / n;
                p.cpu_dev = name.clone();
                p.runq_milli = q / n;
                p.cpu_wait = w / n;
            }
        }
        // what limits it: the most used of them
        let mut best = (0, String::new());
        for (pm, what) in [(p.cpu_max_pm, format!("CPU {}", p.cpu_dev)), (p.lock_max_pm, format!("Sperre {}", p.lock_name)), (p.disk_max_pm, format!("Platte {}", p.disk_dev)), (p.link_max_pm, format!("Kabel {}", p.link_name))] {
            if pm > best.0 {
                best = (pm, what);
            }
        }
        p.bottleneck = if best.0 >= NO_LIMIT_PM { best.1 } else { String::from("-") };
        // euros per hour over requests per hour, for 1000 requests
        p.cost_per_1000_u = if p.achieved_m == 0 { 0 } else { cf.cost_uh * 1_000_000 / (p.achieved_m * 3600) };
        p
    }
}

/// a copy of the project as a configuration has it
fn patched(base: &Project, boot: &Engine, exp: &Experiment, config: &Config) -> Result<Project, Msg> {
    let mut p = base.clone();
    patch_load(&mut p, &exp.load)?;
    for (a, i) in &config.picks {
        match a.kind.as_str() {
            "hw" => patch_hw(&mut p, a, *i)?,
            "config" => patch_config(&mut p, a, *i)?,
            _ => {
                let lb = base.find(&a.dev).map(|d| d.id);
                let addr = |n: &str| base.find(n).and_then(|d| addr_of(boot, d.id, lb));
                if a.template.is_empty() {
                    patch_pool(&mut p, a, *i, &addr)?
                } else {
                    patch_clones(&mut p, a, *i, &addr)?
                }
            }
        }
    }
    // a service whose settings do not fit would simply be left out when the project boots
    for d in &p.devices {
        let mut bare = d.clone();
        bare.services.clear();
        bare.off = false;
        let mut e = Engine::new(0);
        e.add_node(&bare).map_err(|x| x.msg())?;
        for s in &d.services {
            e.service_def(d.id, s).map_err(|x| msg!("{}, Dienst {}: {}", d.name.clone(), s.name.clone(), x.msg()))?;
        }
    }
    Engine::from_project(&p).map_err(|x| x.msg())?;
    Ok(p)
}

// ---------------------------------------------------------------- what is read

/// a device's counters at one moment
struct DevSnap {
    dev: u32,
    /// core time spent, jobs, the time they waited
    done: u64,
    jobs: u64,
    waited: u64,
    pageins: u64,
    oom_kills: u64,
    disk_busy: u64,
    disk_bytes: u64,
}

/// the counters at one moment
struct Snap {
    sent: u64,
    responses: u64,
    ok: u64,
    http_errors: u64,
    failed: u64,
    timeouts: u64,
    devs: Vec<DevSnap>,
    /// cable: bytes either way
    links: Vec<(u32, u64, u64)>,
    /// (device, service, the time its lock was held)
    locks: Vec<(u32, String, u64)>,
}

impl Snap {
    fn of(e: &Engine, gen_dev: u32, svc: &str) -> Snap {
        let l = e.service::<HttpLoad>(gen_dev, svc);
        let num = |f: &dyn Fn(&HttpLoad) -> u64| l.map_or(0, f);
        let now = e.now();
        let mut devs = Vec::new();
        let mut locks = Vec::new();
        for (&dev, &ni) in &e.dev_idx {
            let Some(o) = e.os(ni) else { continue };
            if let Some(r) = o.res.as_deref() {
                devs.push(DevSnap { dev, done: r.done_at(now), jobs: r.jobs, waited: r.wait_ns, pageins: r.pageins, oom_kills: r.oom_kills, disk_busy: o.disk.busy_ns, disk_bytes: o.disk.bytes_read + o.disk.bytes_written });
            }
            for s in o.svcs.iter().filter(|s| !s.console) {
                if let Some(h) = s.svc.as_ref().and_then(|x| x.as_any().downcast_ref::<HttpServer>()) {
                    locks.push((dev, s.name.clone(), h.lock_busy));
                }
            }
        }
        let mut links = Vec::new();
        for (&id, &li) in &e.link_idx {
            let Some(Some(k)) = e.links.get(li) else { continue };
            let tx = |end: usize| -> u64 {
                let (n, p) = k.ends[end];
                match e.nodes.get(n) {
                    Some(Some(node)) => node.ports.get(p as usize).map_or(0, |x| x.counters.tx_bytes),
                    _ => 0,
                }
            };
            links.push((id, tx(0), tx(1)));
        }
        Snap { sent: num(&|x| x.sent), responses: num(&|x| x.responses), ok: num(&|x| x.ok), http_errors: num(&|x| x.http_errors), failed: num(&|x| x.failed), timeouts: num(&|x| x.timeouts), devs, links, locks }
    }
}

impl Engine {
    /// the longest waits count from now on (they are only read by a sweep)
    fn res_marks_reset(&mut self) {
        for ni in 0..self.nodes.len() {
            if let Some(o) = self.os_mut(ni) {
                o.disk.max_wait = 0;
                if let Some(r) = o.res.as_deref_mut() {
                    r.max_wait = 0;
                }
            }
        }
    }

    /// the disks' backlog now, kept where it is the highest so far
    fn backlog_peaks(&self, peaks: &mut Vec<(u32, Ns)>) {
        let now = self.now;
        for (&dev, &ni) in &self.dev_idx {
            let Some(o) = self.os(ni) else { continue };
            if o.res.is_none() {
                continue;
            }
            let b = o.disk.backlog(now);
            match peaks.iter_mut().find(|p| p.0 == dev) {
                Some(p) => p.1 = p.1.max(b),
                None => peaks.push((dev, b)),
            }
        }
    }

    /// what a series gives over (from, to], in thousandths
    fn host_value(&self, dev: u32, name: &str, agg: Agg, from: Ns, to: Ns) -> u64 {
        self.series_value(dev, HOST, name, agg, from, to).map_or(0, |v| v.milli.max(0) as u64)
    }

    fn fig_devs(&self, a: &Snap, b: &Snap, backlog: &[(u32, Ns)], from: Ns, to: Ns) -> Vec<DevFig> {
        let span = to.saturating_sub(from).max(1);
        let mut out = Vec::new();
        for y in &b.devs {
            let Some(x) = a.devs.iter().find(|x| x.dev == y.dev) else { continue };
            let Some(ni) = self.node_index(y.dev) else { continue };
            let Some(o) = self.os(ni) else { continue };
            let Some(r) = o.res.as_deref() else { continue };
            let cores = r.hw.cores as u64;
            let jobs = y.jobs - x.jobs;
            out.push(DevFig {
                dev: y.dev,
                name: self.node(y.dev).map_or(String::new(), |n| n.name.clone()),
                cores: r.hw.cores,
                cpu_pm: ((y.done.saturating_sub(x.done)) as u128 * 1000 / (span as u128 * cores.max(1) as u128)).min(1000) as u64,
                jobs,
                cpu_wait: if jobs == 0 { 0 } else { (y.waited - x.waited) / jobs },
                cpu_wait_max: r.max_wait,
                runq_milli: self.host_value(y.dev, "runq", Agg::Mean, from, to),
                runq_max: self.host_value(y.dev, "runq", Agg::Max, from, to) / 1000,
                mem: self.host_value(y.dev, "mem_bytes", Agg::Max, from, to) / 1000,
                swap: self.host_value(y.dev, "swap_bytes", Agg::Max, from, to) / 1000,
                pageins: y.pageins - x.pageins,
                oom_kills: y.oom_kills - x.oom_kills,
                disk_pm: ((y.disk_busy - x.disk_busy) as u128 * 1000 / span as u128).min(1000) as u64,
                disk_wait_max: o.disk.max_wait,
                disk_backlog_max: backlog.iter().find(|p| p.0 == y.dev).map_or(0, |p| p.1),
                disk_bytes: y.disk_bytes - x.disk_bytes,
            });
        }
        out
    }

    fn fig_links(&self, a: &Snap, b: &Snap, from: Ns, to: Ns) -> Vec<LinkFig> {
        let span = to.saturating_sub(from).max(1);
        let mut out = Vec::new();
        for y in &b.links {
            let Some(x) = a.links.iter().find(|x| x.0 == y.0) else { continue };
            let Some(l) = self.link_rt(y.0) else { continue };
            let bytes = (y.1 - x.1).max(y.2 - x.2);
            // bits over the bits the cable carries in the span (frames without preamble and gap), per mille
            let util = (bytes as u128 * 8 * SEC as u128 / (span as u128 * (l.kbps as u128).max(1))).min(1000) as u64;
            out.push(LinkFig { link: y.0, name: self.cable_label(y.0), util_pm: util, bytes });
        }
        out
    }

    fn fig_svcs(&self, a: &Snap, b: &Snap, from: Ns, to: Ns) -> Vec<SvcFig> {
        let span = to.saturating_sub(from).max(1);
        let mut out = Vec::new();
        for k in self.kpis_between(from, to) {
            if k.svc == HOST {
                continue;
            }
            let lock = |s: &Snap| s.locks.iter().find(|l| l.0 == k.dev && l.1 == k.svc).map_or(0, |l| l.2);
            out.push(SvcFig {
                dev: k.dev,
                dev_name: self.node(k.dev).map_or(String::new(), |n| n.name.clone()),
                kind: k.kind,
                rps_milli: k.rps_milli.unwrap_or(0),
                err_pm: k.err_pm.unwrap_or(0),
                p50_us: k.p50_us.unwrap_or(0),
                p95_us: k.p95_us.unwrap_or(0),
                p99_us: k.p99_us.unwrap_or(0),
                queue: k.queue.unwrap_or(0),
                inflight: k.inflight.unwrap_or(0),
                hit_pm: k.hit_pm.unwrap_or(0),
                load_milli: k.load_milli.unwrap_or(0),
                lock_pm: ((lock(b) - lock(a)) as u128 * 1000 / span as u128).min(1000) as u64,
                svc: k.svc,
            });
        }
        out
    }
}

// ---------------------------------------------------------------- the results

/// a configuration and a level over its seeds
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub config: usize,
    pub level: usize,
    /// requests per second in thousandths
    pub offered_m: u64,
    pub achieved_m: u64,
    pub sent: u64,
    pub responses: u64,
    pub ok: u64,
    pub errors: u64,
    pub failed: u64,
    pub timeouts: u64,
    pub err_ppm: u64,
    /// ns
    pub p50: Option<u64>,
    pub p95: Option<u64>,
    pub p99: Option<u64>,
    pub p999: Option<u64>,
    pub mean: Option<u64>,
    pub max: Option<u64>,
    /// the seeds' own p99, for the spread
    pub p99_min: Option<u64>,
    pub p99_max: Option<u64>,
    pub slo_ok: bool,
    pub knee: bool,
    pub cpu_max_pm: u64,
    pub cpu_dev: String,
    pub runq_milli: u64,
    pub cpu_wait: Ns,
    pub mem_max: u64,
    pub swap_max: u64,
    pub pageins: u64,
    pub oom_kills: u64,
    pub disk_max_pm: u64,
    pub disk_dev: String,
    pub disk_wait_max: Ns,
    pub disk_backlog_max: Ns,
    pub link_max_pm: u64,
    pub link_name: String,
    pub lock_max_pm: u64,
    pub lock_name: String,
    pub bottleneck: String,
    pub cost_uh: u64,
    /// euros in millionths per 1000 requests answered without error
    pub cost_per_1000_u: u64,
    pub status: Status,
    pub seconds: u64,
}

/// a configuration over its levels
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigOut {
    pub label: String,
    pub series: String,
    pub labels: Vec<(String, String)>,
    pub cost_uh: u64,
    /// the highest level up to which every level holds the SLO
    pub knee: Option<usize>,
    pub max_achieved_m: u64,
    /// what limits it (Point::bottleneck at the first level that does not hold)
    pub limit: String,
}

/// what a sweep gives
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sweep {
    pub name: String,
    pub title: String,
    /// the axes' columns over all series
    pub columns: Vec<String>,
    pub configs: Vec<ConfigOut>,
    pub levels: Vec<Level>,
    pub seeds: Vec<u64>,
    pub slo: SloLimits,
    pub warmup_s: u64,
    pub settle_s: u64,
    pub measure_s: u64,
    /// by configuration, then level
    pub points: Vec<Point>,
    /// every run in the plan's index order
    pub details: Vec<Fig>,
}

/// a number with three decimals from thousandths
fn milli(v: u64) -> String {
    format!("{}.{:03}", v / 1000, v % 1000)
}

/// a number of ns as milliseconds with one decimal, "-" without a value
fn ms1(v: Option<u64>) -> String {
    match v {
        Some(ns) => format!("{}.{}", ns / MS, ns % MS / (MS / 10)),
        None => String::from("-"),
    }
}

/// a number of ns as microseconds, empty without a value
fn us_or_empty(v: Option<u64>) -> String {
    v.map_or(String::new(), |ns| format!("{}", ns / 1000))
}

/// a field of a CSV, quoted when it must be
fn field(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { String::from(s) }
}

/// millionths of a euro with six decimals
fn euro(v: u64) -> String {
    format!("{}.{:06}", v / 1_000_000, v % 1_000_000)
}

/// millionths of a euro with three decimals
fn euro3(v: u64) -> String {
    format!("{}.{:03}", v / 1_000_000, v % 1_000_000 / 1000)
}

/// a table's rows with the columns as wide as they need (the first column left, the others right)
fn aligned(rows: &[Vec<String>], out: &mut String) {
    let n = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut w = alloc::vec![0; n];
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            w[i] = w[i].max(c.chars().count());
        }
    }
    for r in rows {
        let mut line = String::new();
        for (i, c) in r.iter().enumerate() {
            let pad = w[i] - c.chars().count();
            if i > 0 {
                line.push_str("  ");
            }
            let last = i + 1 == r.len();
            if i == 0 || (last && c.parse::<f32>().is_err()) {
                line.push_str(c);
                if !last {
                    for _ in 0..pad {
                        line.push(' ');
                    }
                }
            } else {
                for _ in 0..pad {
                    line.push(' ');
                }
                line.push_str(c);
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
}

impl Sweep {
    /// a configuration's points
    pub fn points_of(&self, config: usize) -> impl Iterator<Item = &Point> {
        self.points.iter().filter(move |p| p.config == config)
    }

    /// the results as a plain table: the same bytes on every machine, whatever the threads
    pub fn table(&self) -> String {
        let mut s = String::new();
        let t = if self.title.is_empty() { String::new() } else { format!(": \u{201e}{}\u{201c}", self.title) };
        s.push_str(&format!("# JFNetworker-Experiment {}{}\n", self.name, t));
        s.push_str(&format!("# {} Konfigurationen, {} Laststufen, {} Seeds; {} s Warmlauf, je Stufe {} s Übergang und {} s Messung (virtuelle Zeit)\n", self.configs.len(), self.levels.len(), self.seeds.len(), self.warmup_s, self.settle_s, self.measure_s));
        s.push_str(&format!("# SLO: {}\n", self.slo.text()));
        s.push_str("# Ergebnis des Modells mit seinen Annahmen; die Kosten sind angenommene Preise (catalog::Prices), keine Angebote\n");
        for (c, cfg) in self.configs.iter().enumerate() {
            s.push('\n');
            s.push_str(&format!("{} ({} \u{20ac}/h)\n", cfg.label, euro3(cfg.cost_uh)));
            let mut rows = alloc::vec![["Last", "erreicht/s", "p50 ms", "p95 ms", "p99 ms", "p99,9 ms", "Fehler %", "CPU %", "\u{20ac}/1000", "SLO", "Engpass"].iter().map(|x| String::from(*x)).collect::<Vec<String>>()];
            for p in self.points_of(c) {
                rows.push(alloc::vec![
                    self.levels.get(p.level).map_or(String::new(), |l| l.text()),
                    milli(p.achieved_m),
                    ms1(p.p50),
                    ms1(p.p95),
                    ms1(p.p99),
                    ms1(p.p999),
                    format!("{}.{:02}", p.err_ppm / 10_000, p.err_ppm % 10_000 / 100),
                    format!("{}.{}", p.cpu_max_pm / 10, p.cpu_max_pm % 10),
                    euro(p.cost_per_1000_u),
                    String::from(if p.status != Status::Ok { p.status.as_str() } else if p.knee { "Knie" } else if p.slo_ok { "ok" } else { "nein" }),
                    p.bottleneck.clone(),
                ]);
            }
            aligned(&rows, &mut s);
        }
        s.push('\n');
        s.push_str("Zusammenfassung\n");
        let mut rows = alloc::vec![["Konfiguration", "\u{20ac}/h", "Knie", "Maximum/s", "\u{20ac}/1000 am Knie", "begrenzt durch"].iter().map(|x| String::from(*x)).collect::<Vec<String>>()];
        for (c, cfg) in self.configs.iter().enumerate() {
            let at_knee = cfg.knee.and_then(|k| self.points_of(c).nth(k));
            rows.push(alloc::vec![
                cfg.label.clone(),
                euro3(cfg.cost_uh),
                cfg.knee.and_then(|k| self.levels.get(k)).map_or(String::from("-"), |l| l.text()),
                milli(cfg.max_achieved_m),
                at_knee.map_or(String::from("-"), |p| euro(p.cost_per_1000_u)),
                cfg.limit.clone(),
            ]);
        }
        aligned(&rows, &mut s);
        s
    }

    /// a row per configuration and level; the axes' columns stay empty where a series has none
    pub fn csv(&self) -> String {
        let mut s = String::from("series");
        for c in &self.columns {
            s.push(',');
            s.push_str(&field(c));
        }
        s.push_str(",offered_rps,users,achieved_rps,sent,ok,errors,timeouts,err_pct,p50_us,p95_us,p99_us,p999_us,mean_us,max_us,p99_min_us,p99_max_us,slo_ok,knee,cpu_max_pct,cpu_dev,runq_mean,cpu_wait_us,mem_max_mib,swap_max_mib,pageins,oom_kills,disk_max_pct,disk_dev,disk_wait_max_us,disk_backlog_max_us,link_max_pct,link,lock_max_pct,lock,bottleneck,cost_eur_h_assumed,cost_eur_per_1000_assumed,status,seconds\n");
        for p in &self.points {
            let Some(cfg) = self.configs.get(p.config) else { continue };
            s.push_str(&field(&cfg.series));
            for c in &self.columns {
                s.push(',');
                if let Some((_, v)) = cfg.labels.iter().find(|(n, _)| n == c) {
                    s.push_str(&field(v));
                }
            }
            let users = match self.levels.get(p.level) {
                Some(Level::Users(u)) => format!("{}", u),
                _ => String::new(),
            };
            s.push_str(&format!(",{},{},{},{},{},{},{},{}.{:04},", milli(p.offered_m), users, milli(p.achieved_m), p.sent, p.ok, p.errors, p.timeouts, p.err_ppm / 10_000, p.err_ppm % 10_000));
            for q in [p.p50, p.p95, p.p99, p.p999, p.mean, p.max, p.p99_min, p.p99_max] {
                s.push_str(&us_or_empty(q));
                s.push(',');
            }
            s.push_str(&format!("{},{},{}.{},{},{},{},{},{},{},{},{}.{},{},{},{},{}.{},{},{}.{},{},{},{},{},{},{}\n",
                if p.slo_ok { 1 } else { 0 },
                if p.knee { 1 } else { 0 },
                p.cpu_max_pm / 10,
                p.cpu_max_pm % 10,
                field(&p.cpu_dev),
                milli(p.runq_milli),
                p.cpu_wait / 1000,
                p.mem_max >> 20,
                p.swap_max >> 20,
                p.pageins,
                p.oom_kills,
                p.disk_max_pm / 10,
                p.disk_max_pm % 10,
                field(&p.disk_dev),
                p.disk_wait_max / 1000,
                p.disk_backlog_max / 1000,
                p.link_max_pm / 10,
                p.link_max_pm % 10,
                field(&p.link_name),
                p.lock_max_pm / 10,
                p.lock_max_pm % 10,
                field(&p.lock_name),
                field(&p.bottleneck),
                euro(p.cost_uh),
                euro(p.cost_per_1000_u),
                p.status.as_str(),
                p.seconds,
            ));
        }
        s
    }

    /// what is behind a row: a line per run and thing measured (host, cable, service)
    pub fn detail_csv(&self) -> String {
        const COLS: [&str; 24] = ["cores", "cpu_pm", "cpu_wait_us", "runq_mean", "mem_mib", "swap_mib", "pageins", "oom_kills", "disk_pm", "disk_wait_max_us", "disk_backlog_max_us", "disk_bytes", "util_pm", "bytes", "rps", "err_pm", "p50_us", "p95_us", "p99_us", "queue", "inflight", "hit_pm", "load", "lock_pm"];
        let mut s = format!("config,level,seed,what,name,{}\n", COLS.join(","));
        let (nc, ns, nl) = (self.configs.len(), self.seeds.len(), self.levels.len());
        let mut line = |head: &str, what: &str, name: &str, from: usize, vals: &[String]| {
            s.push_str(&format!("{},{},{}", head, field(what), field(name)));
            for i in 0..COLS.len() {
                s.push(',');
                if i >= from && i < from + vals.len() {
                    s.push_str(&vals[i - from]);
                }
            }
            s.push('\n');
        };
        let n = |v: u64| format!("{}", v);
        let i = |v: i64| format!("{}", v);
        for c in 0..nc {
            for sd in 0..ns {
                for l in 0..nl {
                    let Some(f) = self.details.get((c * ns + sd) * nl + l) else { continue };
                    let head = format!("{},{},{}", field(&self.configs[c].label), self.levels[l].value(), self.seeds[sd]);
                    for d in &f.devs {
                        line(&head, "host", &d.name, 0, &[n(d.cores as u64), n(d.cpu_pm), n(d.cpu_wait / 1000), milli(d.runq_milli), n(d.mem >> 20), n(d.swap >> 20), n(d.pageins), n(d.oom_kills), n(d.disk_pm), n(d.disk_wait_max / 1000), n(d.disk_backlog_max / 1000), n(d.disk_bytes)]);
                    }
                    for x in &f.links {
                        line(&head, "cable", &x.name, 12, &[n(x.util_pm), n(x.bytes)]);
                    }
                    for v in &f.svcs {
                        line(&head, v.kind, &format!("{} {}", v.dev_name, v.svc), 14, &[milli(v.rps_milli.max(0) as u64), i(v.err_pm), i(v.p50_us), i(v.p95_us), i(v.p99_us), i(v.queue), i(v.inflight), i(v.hit_pm), milli(v.load_milli.max(0) as u64), n(v.lock_pm)]);
                    }
                }
            }
        }
        s
    }
}
