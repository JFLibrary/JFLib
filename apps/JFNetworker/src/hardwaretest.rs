// hardwaretest -- CPU and RAM of devices (part of --selftest, group hardware; jfn_sim's res.rs): the
// run queue against the closed forms of queueing theory, first directly on Res (millions of jobs,
// tight tolerances) and then end to end over HTTP and TCP (thousands of requests, wider ones, as a
// run of that length has that spread); what the services pay; memory, swap and the out-of-memory
// kill; resizes; determinism; the unmetered path; the steps of a scenario and the window's edits
//
// The biggest risk of this feature is believable wrong numbers, so each check names its formula:
//
//   saturation     X = cores * speed / cost
//   M/M/c          Wq = C(c, a) * S / (c - a), a = lambda * S (Erlang's C)
//   M/D/1          Wq = rho * S / (2 * (1 - rho))
//   USL            X(N) = N / (1 + sigma (N - 1) + kappa N (N - 1)) * X(1), peak at sqrt((1 - sigma) / kappa)
//   utilisation    rho = lambda * S / c
//
// Floating point is fine here: it only compares results, nothing here feeds the simulation.
// "--selftest --only=hardware --numbers" prints every such comparison with its numbers.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::jmap::Value;
use base::rng::Pcg32;
use jfn_model::catalog::{self, DiskClass, Hw};
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_model::scenario::{self, FaultSpec, HealSpec, StepKind};
use jfn_sim::httpsvc::exp_sample;
use jfn_sim::keyframe::{digest, Budget};
use jfn_sim::res::{Res, Usl, HOST, OOM_EXIT, QUANTUM};
use jfn_sim::{Action, Cmd, Engine, Fault, Inline, Report, MS, SEC, US};

use crate::doc::{edits, Doc};
use crate::selftest::T;

const BUDGET: usize = 80_000_000;
const SEED: u64 = 17;
const GEN_IP: &str = "10.0.0.1";
const SRV_IP: &str = "10.0.0.2";

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: String::from(kind), name: String::from(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Value::Null) }
}

fn hw(cores: u32, speed_pm: u32, ram_gb: u64) -> Hw {
    Hw { cores, speed_pm, ram_mb: ram_gb << 10, swap_mb: 1 << 10, disk: DiskClass::Sata, disk_gb: 240 }
}

/// a load generator (no hardware, it never pays) and a server on a 10G cable
struct Net {
    p: Project,
    gen_: u32,
    srv: u32,
}

fn net(srv_hw: Option<Hw>, server: &[ServiceDef], load: &str) -> Net {
    let mut p = Project::new("hardware", SEED);
    let gen_ = p.add_device("traffic-gen", 0, 0).unwrap_or(0);
    let srv = p.add_device("server-1u", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(gen_, "eth0"), &pr(srv, "eth1"), None);
    if let Some(d) = p.device_mut(gen_) {
        d.startup_config = format!("ip addr add {}/24 dev eth0\n", GEN_IP);
        if !load.is_empty() {
            d.services.push(def("http-load", "load", load));
        }
    }
    if let Some(d) = p.device_mut(srv) {
        d.startup_config = format!("ip addr add {}/24 dev eth1\n", SRV_IP);
        d.services = server.to_vec();
        d.hw = srv_hw;
    }
    Net { p, gen_, srv }
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
    let _ = e.take_reports();
}

/// closed loop: `users` users with no thinking
fn closed(users: u32) -> String {
    format!(r#"{{"targets": ["http://{}/"], "mode": "closed", "users": {}, "timeout_ms": 600000}}"#, SRV_IP, users)
}

/// open loop: a Poisson process of `rps` per second
fn open(rps: u32) -> String {
    format!(r#"{{"targets": ["http://{}/"], "mode": "open", "rps": {}, "arrival": "poisson", "timeout_ms": 600000}}"#, SRV_IP, rps)
}

/// a web server with this CPU work per request
fn web(name: &str, port: u16, cpu_ms: &str) -> ServiceDef {
    def("http-server", name, &format!(r#"{{"port": {}, "cpu_ms": {}, "max_requests": 0, "routes": [{{"path": "*", "method": "", "body": "ok"}}]}}"#, port, cpu_ms))
}

fn metric(e: &Engine, dev: u32, svc: &str, name: &str) -> i64 {
    e.metric(dev, svc, name).unwrap_or(-1)
}

/// the answers per second of a service in `secs` seconds after `warm` seconds
fn rate(e: &mut Engine, dev: u32, svc: &str, warm: u64, secs: u64) -> f64 {
    run_for(e, warm * SEC);
    let a = metric(e, dev, svc, "responses");
    run_for(e, secs * SEC);
    let b = metric(e, dev, svc, "responses");
    (b - a) as f64 / secs as f64
}

fn near(got: f64, want: f64, tol: f64) -> bool {
    (got - want).abs() <= want.abs() * tol
}

fn check_near(t: &mut T, name: &str, got: f64, want: f64, tol: f64) {
    let line = format!("{}: {:.3} against {:.3} (within {:.1} %)", name, got, want, tol * 100.0);
    if plat::args().iter().any(|a| a == "--numbers") {
        plat::write_stdout(format!("{}, off by {:.2} %\n", line, (got - want).abs() / want.abs().max(1e-12) * 100.0).as_bytes());
    }
    t.check(&line, near(got, want, tol));
}

pub(crate) fn run(t: &mut T) {
    model_and_file(t);
    unmetered(t);
    res_alone(t);
    res_exact(t);
    saturation(t);
    md1_http(t);
    usl(t);
    shared(t);
    slices(t);
    serial(t);
    series(t);
    faults(t);
    ram(t);
    oom(t);
    oom_unit(t);
    resize(t);
    services(t);
    scripts(t);
    console(t);
    determinism(t);
    steps(t);
    window(t);
}

// ---------------------------------------------------------------- the model

fn model_and_file(t: &mut T) {
    for (id, cores) in [("server-s", 4), ("server-m", 8), ("server-l", 16), ("server-xl", 32), ("nas", 2)] {
        t.check(&format!("{} is in the catalog", id), catalog::model(id).is_some_and(|m| m.category == catalog::Category::Servers));
        t.eq(&format!("{} has its cores", id), catalog::hw_of(id).map(|h| h.cores), Some(cores));
        t.check(&format!("{} has a leaf of its own", id), jfn_sim::snmpmib::MODEL_LEAVES.iter().filter(|x| x.0 == id).count() == 1);
    }
    let leaves: Vec<u32> = jfn_sim::snmpmib::MODEL_LEAVES.iter().map(|x| x.1).collect();
    t.check("the leaves are all different", (1..leaves.len()).all(|i| !leaves[..i].contains(&leaves[i])));
    for id in ["server-1u", "server-1u-sfp", "server-tower", "load-balancer", "database", "cache", "queue", "container-host", "pc", "laptop"] {
        t.check(&format!("{} has figures", id), catalog::hw_of(id).is_some());
    }
    for id in ["traffic-gen", "jf-r4", "jf-s24p", "cloud", "note", "smartphone"] {
        t.check(&format!("{} stays without", id), catalog::hw_of(id).is_none());
    }
    t.check("every preset has figures", catalog::hw_presets().all(|m| catalog::hw_of(m.id).is_some()));
    // bigger is not simply more of everything
    let speed = |id: &str| catalog::hw_of(id).map_or(0, |h| h.speed_pm);
    t.check("more cores, a lower clock", speed("server-s") > speed("server-m") && speed("server-m") > speed("server-l") && speed("server-l") > speed("server-xl"));
    // the disks (the plain disk's numbers are the SATA class's)
    let d = jfn_sim::disk::Disk::default();
    t.eq("the class of the plain disk", DiskClass::Sata.spec(), (d.latency, d.bps, d.iops));
    t.check("a hard disk is slower, NVMe faster", DiskClass::Hdd.spec().0 > DiskClass::Sata.spec().0 && DiskClass::Nvme.spec().0 < DiskClass::Sata.spec().0 && DiskClass::NvmeFast.spec().1 > DiskClass::Nvme.spec().1);
    // the file: nothing at its default is written, what is set comes back
    let n = net(None, &[], "");
    let plain = n.p.to_text();
    t.check("no default in the file", !plain.contains("\"hw\"") && !plain.contains("\"metered\""));
    let mut p = n.p.clone();
    p.metered = true;
    if let Some(d) = p.device_mut(n.srv) {
        d.hw = Some(hw(4, 1500, 16));
    }
    let text = p.to_text();
    t.check("set ones are written", text.contains("\"metered\": true") && text.contains("\"speed_pm\": 1500"));
    let q = Project::from_text(&text);
    t.check("they come back", q.is_ok_and(|q| q == p));
    t.eq("an old file has neither", Project::from_text(&plain).map(|q| (q.metered, q.device(n.srv).map(|d| d.hw.is_none()))).ok(), Some((false, Some(true))));
    t.eq("sane figures", Hw { cores: 0, speed_pm: 0, ram_mb: 0, ..Hw::default() }.sane().cores, 1);
}

// ---------------------------------------------------------------- without hardware

fn unmetered(t: &mut T) {
    let s = def("http-server", "web", r#"{"cpu_ms": 50, "time_ms": 1, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#);
    let n = net(None, &[s], &closed(4));
    let mut e = engine(&n.p);
    t.check("no hardware without a profile", e.res_of(n.srv).is_none() && e.res_view(n.srv).is_none());
    // cpu_ms costs nothing there: 4 users of 1 ms each, not 4 of 50 ms
    let x = rate(&mut e, n.srv, "web", 1, 2);
    check_near(t, "cpu_ms is free: answers per second", x, 4000.0, 0.05);
    t.check("no series of the host", e.series(n.srv, HOST).is_empty() && !e.series_services().iter().any(|s| s.svc == HOST));
    t.eq("no number of the host", e.metric(n.srv, HOST, "cores"), None);
    t.check("no hardware, no faults of it", e.apply(Action::Fault { fault: Fault::SlowCpu { dev: n.srv, factor_pm: 2000 }, dur: None }).is_err() && e.apply(Action::Fault { fault: Fault::MemHog { dev: n.srv, bytes: 1 }, dur: None }).is_err());
    // the commands that only such a device has are not there
    let o = out(&mut e, n.srv, "free");
    t.check("free is not found", o.contains("command not found"));
    let o = out(&mut e, n.srv, "lscpu");
    t.check("lscpu is not found", o.contains("command not found"));
    let o = out(&mut e, n.srv, "top -b -n 1");
    t.check("top has no memory line", o.contains("%Cpu(s)") && !o.contains("MiB Mem"));
    // a metered project gives hardware to those with figures only
    let mut m = net(None, &[web("web", 80, "1")], "");
    m.p.metered = true;
    let e = engine(&m.p);
    t.check("a metered project: the server has its model's", e.res_of(m.srv).is_some_and(|r| !r.own && Some(r.hw.clone()) == catalog::hw_of("server-1u")));
    t.check("the load generator stays without", e.res_of(m.gen_).is_none());
    // its disk is its class's
    t.check("its disk is its class's", e.disk_of(m.srv).is_some_and(|d| d.capacity == 960 << 30));
}

/// what a device prints for a line
fn out(e: &mut Engine, dev: u32, line: &str) -> String {
    let _ = e.cli_line(dev, line);
    let t = e.now() + 100 * MS;
    e.run_until(t, BUDGET);
    let mut o = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            o.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    o
}

// ---------------------------------------------------------------- the queue alone

/// n! / the sum in Erlang's C
fn erlang_c(c: u32, a: f64) -> f64 {
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..c {
        term *= a / k as f64;
        sum += term;
    }
    let last = term * a / c as f64 * c as f64 / (c as f64 - a);
    last / (sum + last)
}

/// Poisson arrivals on Res with service times from `s(rng)`: the mean wait (ns), the mean run queue
/// (from Little's law) and the utilisation
fn poisson(c: u32, lambda: f64, n: u64, s: &mut dyn FnMut(&mut Pcg32) -> u64) -> (f64, f64) {
    let mut r = Res::new(Hw { cores: c, speed_pm: 1000, ram_mb: 1 << 20, swap_mb: 0, disk: DiskClass::Sata, disk_gb: 1 }, true);
    let mut g = Pcg32::new(SEED, 3);
    let mean = (1e9 / lambda) as u64;
    let mut now = 0;
    for _ in 0..n {
        now += exp_sample(mean, g.next_u32());
        let cost = s(&mut g);
        r.submit(1, now, cost, Usl::default());
    }
    let span = now as f64;
    (r.wait_ns as f64 / n as f64, r.done_at(now) as f64 / (span * c as f64))
}

fn res_alone(t: &mut T) {
    // The tolerances are three times the spread of such a mean over seeds, from an independent
    // simulation (Lindley's recursion with other random numbers): M/M/c at 80 % and a million jobs
    // 1.1 % (one core) to 2 % (eight), M/D/1 at 90 % and two million jobs 0.7 %.  The run itself is
    // fixed by its seed.  The formula takes the utilisation the run really had, as its arrivals are
    // random too: at 90 % a tenth of a per cent more load is a per cent more waiting.
    const S: u64 = MS;
    for c in [1u32, 2, 4, 8] {
        for rho in [0.5f64, 0.8] {
            let a = rho * c as f64;
            let lambda = a / (S as f64 / 1e9);
            let (w, u) = poisson(c, lambda, 1_000_000, &mut |g| exp_sample(S, g.next_u32()));
            let real = u * c as f64;
            let want = erlang_c(c, real) * S as f64 / (c as f64 - real);
            check_near(t, &format!("M/M/{} at {} %: the mean wait in µs", c, rho * 100.0), w / 1000.0, want / 1000.0, 0.06);
            check_near(t, &format!("M/M/{} at {} %: utilisation", c, rho * 100.0), u, rho, 0.01);
        }
    }
    // M/D/1: Pollaczek and Khinchine for a fixed service time
    for rho in [0.3f64, 0.5, 0.8, 0.9] {
        let lambda = rho / (S as f64 / 1e9);
        let (w, u) = poisson(1, lambda, 2_000_000, &mut |_| S);
        let want = u * S as f64 / (2.0 * (1.0 - u));
        check_near(t, &format!("M/D/1 at {} %: the mean wait in µs", rho * 100.0), w / 1000.0, want / 1000.0, 0.02);
        check_near(t, &format!("M/D/1 at {} %: utilisation", rho * 100.0), u, rho, 0.01);
    }
}

fn res_exact(t: &mut T) {
    let mut r = Res::new(hw(2, 1000, 1), true);
    let u = Usl::default();
    t.eq("a free core at once", r.submit(1, 0, 10, u), (0, 10));
    t.eq("the other core at once", r.submit(1, 0, 10, u), (0, 10));
    t.eq("then it waits", r.submit(1, 0, 10, u), (10, 20));
    t.eq("first come, first served", r.submit(1, 5, 10, u), (10, 20));
    t.eq("the run queue at 5", r.waiting(5), 2);
    t.eq("the run queue at 10", r.waiting(10), 0);
    t.eq("busy cores at 5", r.running(5), 2);
    t.eq("core time spent by 15", r.done_at(15), 30);
    t.eq("core time spent by the end", r.done_at(100), 40);
    t.eq("the waits", r.wait_ns, 15);
    t.eq("the longest wait", r.max_wait, 10);
    // speed: twice as fast, half the time
    let mut r = Res::new(hw(1, 2000, 1), true);
    t.eq("speed 2000", r.submit(1, 0, 10, u), (0, 5));
    let mut r = Res::new(hw(1, 500, 1), true);
    t.eq("speed 500", r.submit(1, 0, 10, u), (0, 20));
    // a slow CPU fault multiplies with it
    r.slow_pm = 3000;
    t.eq("a slow CPU", r.submit(1, 20, 10, u), (20, 80));
    // a job of 1 ns on a fast core takes 1 ns (time goes on)
    let mut r = Res::new(hw(1, 100_000, 1), true);
    t.eq("at least 1 ns", r.submit(1, 0, 1, u), (0, 1));
    // the factor of the law: the cores busy with it, this one's among them
    let l = Usl { sigma_ppm: 100_000, kappa_ppm: 10_000 };
    t.eq("USL with 1", l.factor_ppm(1), 1_000_000);
    t.eq("USL with 4", l.factor_ppm(4), 1_000_000 + 300_000 + 120_000);
    let mut r = Res::new(hw(4, 1000, 1), true);
    t.eq("alone: no factor", r.submit(1, 0, 1000, l), (0, 1000));
    t.eq("with one busy: 2", r.submit(1, 0, 1000, l), (0, 1120));
    // config
    let c = agent_core::json::parse(br#"{"usl": {"sigma": 0.1, "kappa": 0.01}, "mem_mb": 2, "conn_kb": 3, "data_mb": 4}"#).unwrap_or(Value::Null);
    t.eq("usl of a config", Usl::of(&c), l);
    let m = jfn_sim::res::SvcMem::of(&c, 9, 9);
    t.eq("memory of a config", m.bytes(2, 5), (2 << 20) + (6 << 10) + (4 << 20) + 5);
    let c = Value::Object(Vec::new());
    t.eq("defaults of its kind", jfn_sim::res::SvcMem::of(&c, 9, 7).bytes(1, 0), (9 << 20) + (7 << 10));
    // a turn holds a place, its work counts on the core it had
    let mut r = Res::new(hw(1, 1000, 1), true);
    let (at, core) = r.turn(0);
    t.eq("a turn at once", (at, core), (0, 0));
    r.charge(1, core, 0, 1000);
    t.eq("its work is on the core", r.next_free(), 1000);
    t.eq("and counts once", r.done_at(5000), 1000);
    let (at, _) = r.turn(100);
    t.eq("the next turn after it", at, 1000);
    // live changes: more cores at once; cores that go hand on their work, none is lost
    let mut r = Res::new(hw(1, 1000, 1), true);
    r.submit(1, 0, 100, u);
    r.resize(10, hw(2, 1000, 1));
    t.eq("a new core is free", r.submit(1, 10, 10, u), (10, 20));
    r.resize(10, hw(1, 1000, 1));
    t.eq("the work of a core that went", r.next_free(), 110);
    t.eq("no work is lost", r.done_at(1000), 110);
    // the power: what was queued is gone, what was done stays counted
    let mut r = Res::new(hw(1, 1000, 1), true);
    r.submit(1, 0, 100, u);
    r.power_off(40);
    t.eq("power off: the rest is gone", (r.done_at(1000), r.next_free(), r.waiting(0)), (40, 0, 0));
    // memory: the overcommit raises costs, the page-ins are counted by an accumulator
    let mut r = Res::new(Hw { cores: 1, speed_pm: 1000, ram_mb: 100, swap_mb: 100, disk: DiskClass::Sata, disk_gb: 1 }, true);
    r.set_mem(1, 100 << 20);
    t.eq("up to the RAM: as it says", r.scale(1000), 1000);
    r.set_mem(1, 150 << 20);
    t.eq("half the swap: 10.5 times", r.scale(1000), 10_500);
    r.set_mem(1, 200 << 20);
    t.eq("all of it: 20 times", r.scale(1000), 20_000);
    t.check("not yet too much", !r.exhausted());
    r.set_mem(1, (200 << 20) + 1);
    t.check("one byte more is", r.exhausted());
    r.hog = 1 << 20;
    r.set_mem(1, 10 << 20);
    t.eq("a hog counts", r.used(), 11 << 20);
    r.set_mem(2, 20 << 20);
    t.eq("the largest", r.largest(), Some(2));
    r.forget(2);
    t.eq("a service that went", r.largest(), Some(1));
}

// ---------------------------------------------------------------- end to end

fn saturation(t: &mut T) {
    // X = cores * speed / cost, whatever the load beyond saturation: within 1 %.  The load sweep:
    // twice, eight times and 32 times as many users as cores
    for cores in [1u32, 2, 4, 8] {
        for users in [2, 8, 32] {
            let n = net(Some(hw(cores, 1000, 8)), &[web("web", 80, "4")], &closed(users * cores));
            let mut e = engine(&n.p);
            let x = rate(&mut e, n.srv, "web", 2, 4);
            check_near(t, &format!("{} cores, {} users: answers per second", cores, users * cores), x, 250.0 * cores as f64, 0.01);
            if users == 32 {
                let cpu = e.series_value(n.srv, HOST, "cpu_pm", jfn_sim::series::Agg::Natural, 3 * SEC, 6 * SEC);
                t.check(&format!("{} cores saturated: cpu_pm {:?}", cores, cpu), cpu.is_ok_and(|v| v.milli >= 990_000));
                t.check("jobs wait", e.metric(n.srv, HOST, "runq").is_some_and(|q| q > 0));
            }
        }
    }
    // the speed scales it, in both directions
    for (speed, want) in [(500, 125.0), (1500, 375.0), (2000, 500.0)] {
        let n = net(Some(hw(1, speed, 8)), &[web("web", 80, "4")], &closed(8));
        let mut e = engine(&n.p);
        let x = rate(&mut e, n.srv, "web", 2, 4);
        check_near(t, &format!("speed {} per mille", speed), x, want, 0.01);
    }
    // below saturation the load decides: an open loop at half of what it could do
    let n = net(Some(hw(2, 1000, 8)), &[web("web", 80, "4")], &open(250));
    let mut e = engine(&n.p);
    let x = rate(&mut e, n.srv, "web", 2, 8);
    // (a Poisson count of 2000 spreads by 2.2 %: three times that)
    check_near(t, "half the capacity: the offered load", x, 250.0, 0.07);
    // time_ms stays waiting: it takes no core, so 100 of them run at once on one core
    let s = def("http-server", "web", r#"{"cpu_ms": 1, "time_ms": 100, "routes": [{"path": "*", "method": "", "body": "ok"}], "max_requests": 0}"#);
    let n = net(Some(hw(1, 1000, 8)), &[s], &closed(100));
    let mut e = engine(&n.p);
    let x = rate(&mut e, n.srv, "web", 2, 4);
    // 100 users, each 100 ms of waiting and 1 ms of CPU behind 100 in the queue at worst: the CPU's
    // 1000 per second is not reached, the users' 100 / 0.1 s is the limit on both sides of it
    t.check(&format!("time_ms takes no core: {:.0}/s", x), x > 400.0);
    let cpu = e.series_value(n.srv, HOST, "cpu_pm", jfn_sim::series::Agg::Natural, 3 * SEC, 6 * SEC).map_or(0, |v| v.milli / 1000);
    check_near(t, "and its CPU is what the requests take", cpu as f64, x, 0.05);
}

fn md1_http(t: &mut T) {
    // M/D/1 over HTTP: arrivals of the load generator's Poisson process, one core, a fixed cost.
    // The mean of 30 s spreads by 3 to 4 % over seeds at these loads (the independent simulation of
    // res_alone: 7500 jobs at 25 %, 22 500 at 75 %), so the tolerance is about four times that; the
    // queue alone is checked tighter
    for (rps, rho) in [(250u32, 0.25f64), (500, 0.5), (750, 0.75)] {
        let n = net(Some(hw(1, 1000, 8)), &[web("web", 80, "1")], &open(rps));
        let mut e = engine(&n.p);
        run_for(&mut e, 30 * SEC);
        let (w, jobs) = e.res_of(n.srv).map_or((0.0, 1.0), |r| (r.wait_ns as f64, r.jobs as f64));
        let want = rho * MS as f64 / (2.0 * (1.0 - rho));
        check_near(t, &format!("M/D/1 over HTTP at {} %: the mean wait in µs", rho * 100.0), w / jobs / 1000.0, want / 1000.0, 0.15);
        check_near(t, &format!("at {} %: jobs per second", rho * 100.0), jobs / 30.0, rps as f64, 0.03);
        let cpu = e.series_value(n.srv, HOST, "cpu_pm", jfn_sim::series::Agg::Natural, 5 * SEC, 30 * SEC).map_or(0.0, |v| v.milli as f64 / 1e6);
        check_near(t, &format!("at {} %: utilisation", rho * 100.0), cpu, rho, 0.03);
    }
}

fn usl(t: &mut T) {
    // sigma 5 %, kappa 2 %: the peak at sqrt(0.95 / 0.02) = 6.9 cores
    let (sigma, kappa) = (0.05f64, 0.02f64);
    let mut xs: Vec<(u32, f64)> = Vec::new();
    for c in [1u32, 2, 4, 6, 7, 8, 10, 12, 16] {
        let s = def("http-server", "web", r#"{"cpu_ms": 4, "usl": {"sigma": 0.05, "kappa": 0.02}, "routes": [{"path": "*", "method": "", "body": "ok"}], "max_requests": 0}"#);
        let n = net(Some(hw(c, 1000, 8)), &[s], &closed(4 * c));
        let mut e = engine(&n.p);
        let x = rate(&mut e, n.srv, "web", 2, 6);
        let f = c as f64;
        let want = 250.0 * f / (1.0 + sigma * (f - 1.0) + kappa * f * (f - 1.0));
        check_near(t, &format!("USL with {} cores: answers per second", c), x, want, 0.02);
        xs.push((c, x));
    }
    let best = xs.iter().fold((0, 0.0), |b, x| if x.1 > b.1 { *x } else { b });
    t.check(&format!("the peak is where the formula puts it: {} cores", best.0), (6..=8).contains(&best.0));
    let at = |c: u32| xs.iter().find(|x| x.0 == c).map_or(0.0, |x| x.1);
    t.check("it falls beyond the peak", at(8) > at(10) && at(10) > at(12) && at(12) > at(16));
    t.check("it rises before", at(1) < at(2) && at(2) < at(4) && at(4) < at(6));
}

fn shared(t: &mut T) {
    // two services on one host share one core
    let mut n = net(Some(hw(1, 1000, 8)), &[web("a", 80, "4"), web("b", 81, "4")], &closed(8));
    if let Some(d) = n.p.device_mut(n.gen_) {
        let b = format!(r#"{{"targets": ["http://{}:81/"], "mode": "closed", "users": 8, "timeout_ms": 600000}}"#, SRV_IP);
        d.services.push(def("http-load", "load2", &b));
    }
    let mut e = engine(&n.p);
    run_for(&mut e, 2 * SEC);
    let (a0, b0) = (metric(&e, n.srv, "a", "responses"), metric(&e, n.srv, "b", "responses"));
    run_for(&mut e, 6 * SEC);
    let (a, b) = ((metric(&e, n.srv, "a", "responses") - a0) as f64 / 6.0, (metric(&e, n.srv, "b", "responses") - b0) as f64 / 6.0);
    check_near(t, "one core, two services: together", a + b, 250.0, 0.01);
    check_near(t, "each its share", a, b, 0.05);
    // on two hosts they do not
    let mut n = net(Some(hw(1, 1000, 8)), &[web("a", 80, "4")], &closed(8));
    let other = n.p.add_device("server-1u", 400, 0).unwrap_or(0);
    let _ = n.p.connect(&pr(n.gen_, "eth1"), &pr(other, "eth1"), None);
    if let Some(d) = n.p.device_mut(n.gen_) {
        d.startup_config.push_str("ip addr add 10.0.1.1/24 dev eth1\n");
        let b = r#"{"targets": ["http://10.0.1.2/"], "mode": "closed", "users": 8, "timeout_ms": 600000}"#;
        d.services.push(def("http-load", "load2", b));
    }
    if let Some(d) = n.p.device_mut(other) {
        d.startup_config = String::from("ip addr add 10.0.1.2/24 dev eth1\n");
        d.services = alloc::vec![web("b", 80, "4")];
        d.hw = Some(hw(1, 1000, 8));
    }
    let mut e = engine(&n.p);
    run_for(&mut e, 2 * SEC);
    let (a0, b0) = (metric(&e, n.srv, "a", "responses"), metric(&e, other, "b", "responses"));
    run_for(&mut e, 6 * SEC);
    let (a, b) = ((metric(&e, n.srv, "a", "responses") - a0) as f64 / 6.0, (metric(&e, other, "b", "responses") - b0) as f64 / 6.0);
    check_near(t, "two hosts: the first its own core", a, 250.0, 0.01);
    check_near(t, "two hosts: the second its own core", b, 250.0, 0.01);
}

fn slices(t: &mut T) {
    // a long request comes in quanta: a short one waits for one quantum of it, not for all of it.
    // One core, a user of 100 ms requests all the time, short ones of 1 ms now and then
    let s = def("http-server", "web", r#"{"cpu_ms": 1, "max_requests": 0, "routes": [{"path": "/long", "method": "", "cpu_ms": 100, "body": "l"}, {"path": "/short", "method": "", "cpu_ms": 1, "body": "s"}]}"#);
    let mut n = net(Some(hw(1, 1000, 8)), &[s], "");
    if let Some(d) = n.p.device_mut(n.gen_) {
        d.services.push(def("http-load", "long", &format!(r#"{{"targets": ["http://{}/long"], "mode": "closed", "users": 1, "timeout_ms": 600000}}"#, SRV_IP)));
        d.services.push(def("http-load", "short", &format!(r#"{{"targets": ["http://{}/short"], "mode": "closed", "users": 1, "think_ms": 50, "timeout_ms": 600000}}"#, SRV_IP)));
    }
    let mut e = engine(&n.p);
    run_for(&mut e, 8 * SEC);
    let p99 = metric(&e, n.gen_, "short", "p99_us");
    t.check(&format!("a short request behind a long one: p99 {} µs", p99), p99 > 0 && p99 < 3 * (QUANTUM / US) as i64);
    let long = metric(&e, n.gen_, "long", "p50_us");
    t.check(&format!("the long one takes its time: p50 {} µs", long), (100_000..130_000).contains(&long));
}

fn serial(t: &mut T) {
    // a critical section is a single-server queue: X = speed / serial_us, however many cores
    let s = |cpu: &str| def("http-server", "web", &format!(r#"{{"cpu_ms": {}, "serial_us": 2000, "max_requests": 0, "routes": [{{"path": "*", "method": "", "body": "ok"}}]}}"#, cpu));
    let n = net(Some(hw(8, 1000, 8)), &[s("0")], &closed(32));
    let mut e = engine(&n.p);
    check_near(t, "a lock of 2 ms on 8 cores: answers per second", rate(&mut e, n.srv, "web", 1, 3), 500.0, 0.01);
    // faster cores hold it for less
    let n = net(Some(hw(8, 2000, 8)), &[s("0")], &closed(32));
    let mut e = engine(&n.p);
    check_near(t, "the same at twice the speed", rate(&mut e, n.srv, "web", 1, 3), 1000.0, 0.01);
    // the slower station of the two sets the pace: 8 cores of 1 ms could do 8000
    let n = net(Some(hw(8, 1000, 8)), &[s("1")], &closed(32));
    let mut e = engine(&n.p);
    check_near(t, "CPU first, then the lock", rate(&mut e, n.srv, "web", 1, 3), 500.0, 0.01);
    t.check("the lock leaves the cores idle", e.series_value(n.srv, HOST, "cpu_pm", jfn_sim::series::Agg::Natural, 2 * SEC, 4 * SEC).is_ok_and(|v| v.milli < 100_000));
    // one core of 4 ms is slower than the lock: there it is the CPU
    let n = net(Some(hw(1, 1000, 8)), &[s("4")], &closed(32));
    let mut e = engine(&n.p);
    check_near(t, "the CPU the slower one", rate(&mut e, n.srv, "web", 1, 3), 250.0, 0.01);
    // it is a real lock with or without hardware
    let n = net(None, &[s("50")], &closed(32));
    let mut e = engine(&n.p);
    check_near(t, "without hardware: the lock alone", rate(&mut e, n.srv, "web", 1, 3), 500.0, 0.01);
}

fn series(t: &mut T) {
    // only a device with hardware has them; they are what the formula says
    let n = net(Some(hw(2, 1000, 8)), &[web("web", 80, "1")], &open(1000));
    let mut e = engine(&n.p);
    run_for(&mut e, 12 * SEC);
    let names: Vec<&str> = e.series(n.srv, HOST).iter().map(|s| s.name).collect();
    t.eq("the series", names, alloc::vec!["cpu_pm", "running", "runq", "mem_bytes", "swap_bytes", "pageins", "oom_kills", "disk_bytes", "disk_pm", "disk_wait_us", "disk_backlog_us"]);
    t.check("the load generator has none", e.series(n.gen_, HOST).is_empty());
    let v = |name: &str, e: &Engine| e.series_value(n.srv, HOST, name, jfn_sim::series::Agg::Natural, 2 * SEC, 12 * SEC).map_or(-1.0, |x| x.milli as f64 / 1000.0);
    // 1000 per second of 1 ms on two cores: half of each; Little: on average one core busy
    check_near(t, "cpu_pm against lambda S / c", v("cpu_pm", &e), 500.0, 0.03);
    t.check("at most its cores busy", v("running", &e) <= 2.0);
    t.eq("the service's base and a connection each", metric(&e, n.srv, HOST, "mem_bytes"), (32 << 20) + (metric(&e, n.srv, "web", "active") << 16));
    t.eq("nothing in the swap", metric(&e, n.srv, HOST, "swap_bytes"), 0);
    t.check("its own numbers for expect", metric(&e, n.srv, HOST, "cores") == 2 && metric(&e, n.srv, HOST, "ram_bytes") == 8 << 30 && e.metric(n.srv, HOST, "cpu_pm").is_some());
    // the CSV holds them
    t.check("the CSV", e.series_csv().contains(" _host cpu_pm"));
}

fn faults(t: &mut T) {
    // a slow CPU: the capacity falls by its factor, and comes back when it heals
    let n = net(Some(hw(1, 1000, 8)), &[web("web", 80, "4")], &closed(8));
    let mut e = engine(&n.p);
    let x = rate(&mut e, n.srv, "web", 2, 4);
    check_near(t, "before a slow CPU", x, 250.0, 0.01);
    t.check("the fault starts", e.apply(Action::Fault { fault: Fault::SlowCpu { dev: n.srv, factor_pm: 4000 }, dur: None }).is_ok());
    let x = rate(&mut e, n.srv, "web", 2, 4);
    check_near(t, "under a slow CPU", x, 62.5, 0.03);
    t.check("it heals", e.apply(Action::HealAll).is_ok());
    let x = rate(&mut e, n.srv, "web", 2, 4);
    check_near(t, "after it", x, 250.0, 0.01);
    // a memory hog: half the swap is 10.5 times the cost, and the disk reads pages
    let h = Hw { cores: 1, speed_pm: 1000, ram_mb: 1 << 10, swap_mb: 1 << 10, disk: DiskClass::Nvme, disk_gb: 100 };
    let n = net(Some(h), &[web("web", 80, "4")], &closed(8));
    let mut e = engine(&n.p);
    let x = rate(&mut e, n.srv, "web", 2, 4);
    check_near(t, "before a memory hog", x, 250.0, 0.01);
    t.eq("no page-in yet", metric(&e, n.srv, HOST, "pageins"), 0);
    let used = metric(&e, n.srv, HOST, "mem_bytes") as u64;
    // RAM + half the swap in all
    let bytes = (1u64 << 30) + (1 << 29) - used;
    t.check("the hog starts", e.apply(Action::Fault { fault: Fault::MemHog { dev: n.srv, bytes }, dur: None }).is_ok());
    let x = rate(&mut e, n.srv, "web", 2, 4);
    check_near(t, "under a memory hog", x, 250.0 / 10.5, 0.1);
    t.check("pages come from the disk", metric(&e, n.srv, HOST, "pageins") > 0 && e.disk_of(n.srv).is_some_and(|d| d.reads > 0 && d.bytes_read > 0));
    t.check("it heals too", e.apply(Action::HealAll).is_ok());
    let p = metric(&e, n.srv, HOST, "pageins");
    let x = rate(&mut e, n.srv, "web", 2, 4);
    check_near(t, "after the hog", x, 250.0, 0.02);
    t.eq("no more page-ins", metric(&e, n.srv, HOST, "pageins"), p);
    // the fault is on the timeline and says what it is
    t.check("the marks' words", e.faults().iter().any(|m| m.text.contains("Speicherfresser")) && e.faults().iter().all(|m| !m.msg.text().is_empty()));
}

fn ram(t: &mut T) {
    // a service with 2 GiB of data, more and more RAM: the disk's load falls to nothing, the
    // throughput rises until the CPU is the limit
    let web_data = || def("http-server", "web", r#"{"cpu_ms": 1, "data_mb": 2048, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#);
    let mut last_disk = u64::MAX;
    let mut last_x = 0.0;
    let mut xs = Vec::new();
    for ram_mb in [1536u64, 1792, 2048, 3072, 8192] {
        let h = Hw { cores: 2, speed_pm: 1000, ram_mb, swap_mb: 2048, disk: DiskClass::Nvme, disk_gb: 100 };
        let n = net(Some(h), &[web_data()], &closed(16));
        let mut e = engine(&n.p);
        let x = rate(&mut e, n.srv, "web", 2, 4);
        let disk = e.disk_of(n.srv).map_or(0, |d| d.bytes_read) / 6;
        t.check(&format!("{} MiB of RAM: the disk reads {} B/s, not more than with less RAM", ram_mb, disk), disk <= last_disk);
        t.check(&format!("{} MiB of RAM: {:.0} answers per second, not fewer", ram_mb, x), x >= last_x * 0.99);
        last_disk = disk;
        last_x = x;
        xs.push(x);
    }
    t.eq("enough RAM: the disk reads nothing", last_disk, 0);
    check_near(t, "then the CPU is the limit", last_x, 2000.0, 0.02);
    t.check(&format!("less RAM is much slower: {:?}", xs), xs[0] * 3.0 < last_x);
}

fn oom(t: &mut T) {
    // too much for RAM and swap: the largest goes with 137, the other stays
    let h = Hw { cores: 2, speed_pm: 1000, ram_mb: 1024, swap_mb: 512, disk: DiskClass::Sata, disk_gb: 10 };
    let small = def("http-server", "small", r#"{"port": 81, "data_mb": 400, "routes": [{"path": "*", "method": "", "body": "s"}]}"#);
    let big = def("http-server", "big", r#"{"port": 80, "data_mb": 1200, "routes": [{"path": "*", "method": "", "body": "b"}]}"#);
    let n = net(Some(h), &[small, big], "");
    let mut e = engine(&n.p);
    run_for(&mut e, SEC);
    let on = |e: &Engine, name: &str| e.services(n.srv).iter().find(|s| s.name == name).is_some_and(|s| s.running);
    t.check("the large one went", !on(&e, "big"));
    t.check("the small one stays", on(&e, "small"));
    t.eq("killed one", metric(&e, n.srv, HOST, "oom_kills"), 1);
    t.eq("it holds nothing", e.res_of(n.srv).map(|r| r.used()), Some((400 + 32) << 20));
    t.check("the log says so", e.log_lines(n.srv).iter().any(|l| l.text.contains("Speicher voll") && l.text.contains("big") && l.text.contains("137")));
    t.check("the recording says so", e.trace.events().any(|x| matches!(&x.note, jfn_sim::Note::OomKill { svc, .. } if svc == "big")));
    // a hog alone with nothing to kill: nothing happens but slowness
    let n2 = net(Some(h2()), &[], "");
    let mut e = engine(&n2.p);
    run_for(&mut e, SEC);
    t.check("a hog with nothing to kill", e.apply(Action::Fault { fault: Fault::MemHog { dev: n2.srv, bytes: 1 << 40 }, dur: None }).is_ok());
    run_for(&mut e, SEC);
    t.eq("no kill", metric(&e, n2.srv, HOST, "oom_kills"), 0);
    t.eq("the constant", OOM_EXIT, 137);
}

fn h2() -> Hw {
    Hw { cores: 1, speed_pm: 1000, ram_mb: 64, swap_mb: 0, disk: DiskClass::Sata, disk_gb: 1 }
}

fn folder(path: &str) -> jfn_model::files::File {
    jfn_model::files::File { path: String::from(path), data: Vec::new(), mtime: 0, dir: true }
}

fn file(path: &str, text: &str) -> jfn_model::files::File {
    jfn_model::files::File { path: String::from(path), data: Vec::from(text.as_bytes()), mtime: 0, dir: false }
}

fn oom_unit(t: &mut T) {
    // a unit's restart policy sees the 137: a hog fills the memory for a while, the service dies
    // with it, its unit starts it again after RestartSec and the hog is gone by then
    let h = Hw { cores: 2, speed_pm: 1000, ram_mb: 1024, swap_mb: 256, disk: DiskClass::Sata, disk_gb: 10 };
    let mut n = net(Some(h), &[], "");
    if let Some(d) = n.p.device_mut(n.srv) {
        d.files = alloc::vec![
            folder("/etc/jfn"),
            folder("/etc/systemd"),
            folder("/etc/systemd/system"),
            folder("/etc/systemd/system/multi-user.target.wants"),
            file("/etc/jfn/web.json", r#"{"port": 80, "data_mb": 300, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#),
            file("/etc/systemd/system/web.service", "[Service]\nExecStart=jfn-service http-server /etc/jfn/web.json\nRestart=on-failure\nRestartSec=3s\nStartLimitBurst=5\n\n[Install]\nWantedBy=multi-user.target\n"),
            file("/etc/systemd/system/multi-user.target.wants/web.service", "/etc/systemd/system/web.service\n"),
        ];
    }
    let mut e = engine(&n.p);
    run_for(&mut e, SEC);
    let state = |e: &Engine| e.unit(n.srv, "web.service").map(|u| (u.state, u.exit, u.restarts));
    t.eq("it runs", state(&e).map(|s| s.0), Some(jfn_sim::UnitState::Active));
    t.check("the hog", e.apply(Action::Fault { fault: Fault::MemHog { dev: n.srv, bytes: 1 << 30 }, dur: Some(2 * SEC) }).is_ok());
    run_for(&mut e, SEC);
    t.eq("killed by the memory, waiting to start", state(&e), Some((jfn_sim::UnitState::AutoRestart, Some(137), 0)));
    run_for(&mut e, 4 * SEC);
    t.eq("it runs again", state(&e), Some((jfn_sim::UnitState::Active, None, 1)));
    t.eq("killed once", metric(&e, n.srv, HOST, "oom_kills"), 1);
}

fn resize(t: &mut T) {
    // a reboot: a mark with its downtime, nothing is answered meanwhile, then more capacity
    let s = def("http-server", "web", r#"{"cpu_ms": 4, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#);
    let load = format!(r#"{{"targets": ["http://{}/"], "mode": "closed", "users": 32, "timeout_ms": 500, "connect_timeout_ms": 500}}"#, SRV_IP);
    let n = net(Some(hw(1, 1000, 8)), &[s.clone()], &load);
    let mut e = engine(&n.p);
    let x = rate(&mut e, n.srv, "web", 2, 3);
    check_near(t, "before: one core", x, 250.0, 0.02);
    t.check("the resize", e.apply(Action::Resize { dev: n.srv, hw: Some(hw(4, 1000, 8)), downtime: 5 * SEC }).is_ok());
    t.check("it is off", e.node(n.srv).is_some_and(|d| !d.power));
    t.check("a mark with its end", e.faults().iter().any(|m| matches!(m.fault, Fault::Resize { .. }) && m.until == Some(e.now() + 5 * SEC)));
    run_for(&mut e, 4 * SEC);
    t.check("still off", e.node(n.srv).is_some_and(|d| !d.power));
    run_for(&mut e, 2 * SEC);
    t.check("on again", e.node(n.srv).is_some_and(|d| d.power) && e.active_faults().next().is_none());
    t.eq("the new hardware", e.res_of(n.srv).map(|r| (r.hw.cores, r.own)), Some((4, true)));
    let x = rate(&mut e, n.srv, "web", 3, 3);
    check_near(t, "after: four cores", x, 1000.0, 0.02);
    // live: no gap, no failure, more capacity at once
    let n = net(Some(hw(1, 1000, 8)), &[s.clone()], &closed(32));
    let mut e = engine(&n.p);
    run_for(&mut e, 3 * SEC);
    let failed = metric(&e, n.gen_, "load", "failed");
    t.check("live", e.apply(Action::Resize { dev: n.srv, hw: Some(hw(4, 1000, 8)), downtime: 0 }).is_ok());
    t.check("no reboot", e.node(n.srv).is_some_and(|d| d.power) && e.faults().is_empty());
    let x = rate(&mut e, n.srv, "web", 1, 3);
    check_near(t, "live: four cores", x, 1000.0, 0.02);
    t.eq("nothing failed", metric(&e, n.gen_, "load", "failed"), failed);
    // the disk stays through a reboot and takes the new class
    let kv = def("kv", "kv", &format!(r#"{{"nodes": ["{}"], "rf": 1}}"#, SRV_IP));
    let n = net(Some(hw(2, 1000, 8)), &[kv], "");
    let mut e = engine(&n.p);
    run_for(&mut e, SEC);
    let used = {
        let _ = out(&mut e, n.gen_, &format!("curl -s -X PUT -d wert http://{}:7000/kv/a", SRV_IP));
        run_for(&mut e, SEC);
        e.disk_of(n.srv).map_or(0, |d| d.used())
    };
    t.check("something on the disk", used > 0);
    let mut h = hw(2, 1000, 8);
    h.disk = DiskClass::Nvme;
    t.check("a reboot with another disk", e.apply(Action::Resize { dev: n.srv, hw: Some(h), downtime: SEC }).is_ok());
    run_for(&mut e, 2 * SEC);
    t.eq("what it held is there", e.disk_of(n.srv).map(|d| d.used()), Some(used));
    t.eq("the new class's numbers", e.disk_of(n.srv).map(|d| d.latency), Some(DiskClass::Nvme.spec().0));
    // back to none, with a project that simulates none
    t.check("back", e.apply(Action::Resize { dev: n.srv, hw: None, downtime: 0 }).is_ok());
    t.check("no hardware", e.res_of(n.srv).is_none());
    t.eq("the plain disk", e.disk_of(n.srv).map(|d| d.latency), Some(jfn_sim::disk::Disk::default().latency));
    // "Hardware simulieren" while it runs: no restart, what the services hold counts at once
    let n = net(None, &[web("web", 80, "1")], "");
    let mut e = engine(&n.p);
    run_for(&mut e, SEC);
    t.check("on", e.apply(Action::Metered { on: true }).is_ok() && e.metered);
    t.check("the model's", e.res_of(n.srv).is_some_and(|r| !r.own && r.used() == 32 << 20));
    t.check("no reboot for it", e.node(n.srv).is_some_and(|d| d.power) && e.services(n.srv).iter().all(|s| s.running));
    t.check("off", e.apply(Action::Metered { on: false }).is_ok() && e.res_of(n.srv).is_none());
    // a device of its own stays when the project's switch goes
    let n = net(Some(hw(3, 1000, 8)), &[], "");
    let mut e = engine(&n.p);
    t.check("on and off", e.apply(Action::Metered { on: true }).is_ok() && e.apply(Action::Metered { on: false }).is_ok());
    t.eq("its own figures stay", e.res_of(n.srv).map(|r| r.hw.cores), Some(3));
}

fn services(t: &mut T) {
    // the load balancer: its own work is the limit when the backends have plenty
    let lb = def("lb", "lb", &format!(r#"{{"cpu_us": 1000, "backends": [{{"name": "b", "addr": "{}", "port": 81}}], "health": {{"type": "none"}}}}"#, SRV_IP));
    let n = net(Some(hw(1, 1000, 8)), &[lb, def("http-server", "b", r#"{"port": 81, "cpu_ms": 0, "max_requests": 0, "routes": [{"path": "*", "method": "", "body": "ok"}]}"#)], &closed(16));
    let mut e = engine(&n.p);
    run_for(&mut e, 2 * SEC);
    let a = metric(&e, n.srv, "lb", "responses");
    run_for(&mut e, 4 * SEC);
    check_near(t, "a load balancer of 1 ms a request on one core", (metric(&e, n.srv, "lb", "responses") - a) as f64 / 4.0, 1000.0, 0.02);
    // the cache: hit_us is CPU now, a plain time without hardware
    let backend = def("http-server", "b", r#"{"port": 81, "cpu_ms": 0, "routes": [{"path": "*", "method": "", "body": "ok", "headers": {"Cache-Control": "max-age=3600"}}]}"#);
    let cache = def("cache", "cache", &format!(r#"{{"port": 80, "backend": "http://{}:81/", "hit_us": 1000}}"#, SRV_IP));
    let n = net(Some(hw(2, 1000, 8)), &[cache.clone(), backend.clone()], &closed(32));
    let mut e = engine(&n.p);
    run_for(&mut e, 2 * SEC);
    let a = metric(&e, n.srv, "cache", "hits");
    run_for(&mut e, 4 * SEC);
    check_near(t, "cache hits of 1 ms on two cores", (metric(&e, n.srv, "cache", "hits") - a) as f64 / 4.0, 2000.0, 0.02);
    t.check("what it keeps is memory", metric(&e, n.srv, HOST, "mem_bytes") > 48 << 20);
    let n = net(None, &[cache, backend], &closed(8));
    let mut e = engine(&n.p);
    run_for(&mut e, SEC);
    let a = metric(&e, n.srv, "cache", "hits");
    run_for(&mut e, SEC);
    check_near(t, "without hardware: hit_us is a plain time, 8 users of 1 ms", (metric(&e, n.srv, "cache", "hits") - a) as f64, 8000.0, 0.05);
    // the key-value store: its CPU work per request
    let kv = def("kv", "kv", &format!(r#"{{"nodes": ["{}"], "rf": 1, "cpu_us": 1000}}"#, SRV_IP));
    let load = format!(r#"{{"targets": ["http://{}:7000/"], "mode": "closed", "users": 16, "timeout_ms": 600000, "mix": [{{"path": "/kv/a"}}]}}"#, SRV_IP);
    let n = net(Some(hw(1, 1000, 8)), &[kv], &load);
    let mut e = engine(&n.p);
    run_for(&mut e, 2 * SEC);
    let a = metric(&e, n.gen_, "load", "responses");
    run_for(&mut e, 4 * SEC);
    check_near(t, "a KV node of 1 ms a request on one core", (metric(&e, n.gen_, "load", "responses") - a) as f64 / 4.0, 1000.0, 0.02);
    t.check("it holds memory", metric(&e, n.srv, HOST, "mem_bytes") >= 64 << 20);
    // a worker: its CPU work per message, its waiting time beside it
    let mq = def("mq", "mq", r#"{"port": 9000, "topics": ["jobs"]}"#);
    let worker = def("worker", "w", &format!(r#"{{"queue": "http://{}:9000", "topic": "jobs", "concurrency": 8, "cpu_ms": 10, "time_ms": 0, "wait_ms": 100}}"#, SRV_IP));
    let load = format!(r#"{{"targets": ["http://{}:9000/"], "mode": "open", "rps": 200, "timeout_ms": 600000, "mix": [{{"method": "POST", "path": "/topics/jobs", "body": "x"}}]}}"#, SRV_IP);
    let n = net(Some(hw(1, 1000, 8)), &[mq, worker], &load);
    let mut e = engine(&n.p);
    run_for(&mut e, 2 * SEC);
    let a = metric(&e, n.srv, "w", "done");
    run_for(&mut e, 4 * SEC);
    check_near(t, "a worker of 10 ms a message on one core", (metric(&e, n.srv, "w", "done") - a) as f64 / 4.0, 100.0, 0.05);
    // a load generator never pays
    let n = net(Some(hw(1, 1000, 8)), &[web("web", 80, "0")], &closed(16));
    let mut e = engine(&n.p);
    run_for(&mut e, 10 * MS);
    t.check("a server of no CPU work is not the limit", metric(&e, n.gen_, "load", "responses") > 100);
    t.check("the generator has no hardware", e.res_of(n.gen_).is_none());
}

fn script_cfg(port: u16, spin: u32) -> ServiceDef {
    script_at(port, spin, 50)
}

fn script_at(port: u16, spin: u32, ns_per_op: u64) -> ServiceDef {
    let src = format!("http.serve({}, fn(req, res) {{\n  let i = 0\n  while i < {} {{ i += 1 }}\n  res.send(200, \"ok\")\n}})\n", port, spin);
    let mut o = Value::Object(Vec::new());
    o.insert("source", Value::string(&src));
    o.insert("ns_per_op", Value::u64(ns_per_op));
    ServiceDef { kind: String::from("script"), name: format!("s{}", port), disabled: false, config: o }
}

fn scripts(t: &mut T) {
    // one script: single-threaded whatever the cores; the time of a call from its own fuel: at
    // speed 1000 an instruction takes what ns_per_op says, 5 ns by default
    let n = net(Some(hw(4, 1000, 8)), &[script_at(80, 20_000, 5)], &closed(16));
    let mut e = engine(&n.p);
    run_for(&mut e, SEC);
    let (c0, f0) = (metric(&e, n.srv, "s80", "calls"), metric(&e, n.srv, "s80", "fuel"));
    run_for(&mut e, SEC);
    let (c, f) = ((metric(&e, n.srv, "s80", "calls") - c0) as f64, (metric(&e, n.srv, "s80", "fuel") - f0) as f64);
    check_near(t, "a script is single-threaded: calls per second at 5 ns", c, 1e9 / (f / c.max(1.0) * 5.0), 0.03);
    let n = net(Some(hw(4, 1000, 8)), &[script_cfg(80, 2000)], &closed(16));
    let mut e = engine(&n.p);
    run_for(&mut e, 2 * SEC);
    let (c0, f0) = (metric(&e, n.srv, "s80", "calls"), metric(&e, n.srv, "s80", "fuel"));
    run_for(&mut e, 4 * SEC);
    let (c, f) = ((metric(&e, n.srv, "s80", "calls") - c0) as f64, (metric(&e, n.srv, "s80", "fuel") - f0) as f64);
    let per_call = f / c.max(1.0);
    let cap = 1e9 / (per_call * 50.0);
    check_near(t, "a script is single-threaded: calls per second", c / 4.0, cap, 0.03);
    // a faster core: faster calls
    let n = net(Some(hw(4, 2000, 8)), &[script_cfg(80, 2000)], &closed(16));
    let mut e = engine(&n.p);
    run_for(&mut e, 2 * SEC);
    let c0 = metric(&e, n.srv, "s80", "calls");
    run_for(&mut e, 4 * SEC);
    check_near(t, "twice the speed: twice the calls", (metric(&e, n.srv, "s80", "calls") - c0) as f64 / 4.0, 2.0 * cap, 0.03);
    // two scripts share one core, on two cores each has its own
    for (cores, want) in [(1u32, cap), (2, 2.0 * cap)] {
        let mut n = net(Some(hw(cores, 1000, 8)), &[script_cfg(80, 2000), script_cfg(81, 2000)], &closed(16));
        if let Some(d) = n.p.device_mut(n.gen_) {
            d.services.push(def("http-load", "load2", &format!(r#"{{"targets": ["http://{}:81/"], "mode": "closed", "users": 16, "timeout_ms": 600000}}"#, SRV_IP)));
        }
        let mut e = engine(&n.p);
        run_for(&mut e, 2 * SEC);
        let (a0, b0) = (metric(&e, n.srv, "s80", "calls"), metric(&e, n.srv, "s81", "calls"));
        run_for(&mut e, 4 * SEC);
        let (a, b) = ((metric(&e, n.srv, "s80", "calls") - a0) as f64 / 4.0, (metric(&e, n.srv, "s81", "calls") - b0) as f64 / 4.0);
        check_near(t, &format!("two scripts on {} cores: together", cores), a + b, want, 0.03);
        check_near(t, &format!("two scripts on {} cores: each its share", cores), a, b, 0.1);
    }
    // a script and a web server on one core: neither starves, the core is full
    let mut n = net(Some(hw(1, 1000, 8)), &[script_cfg(80, 2000), web("web", 81, "1")], &closed(16));
    if let Some(d) = n.p.device_mut(n.gen_) {
        d.services.push(def("http-load", "load2", &format!(r#"{{"targets": ["http://{}:81/"], "mode": "closed", "users": 16, "timeout_ms": 600000}}"#, SRV_IP)));
    }
    let mut e = engine(&n.p);
    run_for(&mut e, 2 * SEC);
    let (a0, b0) = (metric(&e, n.srv, "s80", "calls"), metric(&e, n.srv, "web", "responses"));
    run_for(&mut e, 4 * SEC);
    let (a, b) = ((metric(&e, n.srv, "s80", "calls") - a0) as f64 / 4.0, (metric(&e, n.srv, "web", "responses") - b0) as f64 / 4.0);
    // (first come, first served by the job: a script is one job at a time against 16 requests in
    // the queue, so it gets one turn per round of them, and a turn is a slice)
    t.check(&format!("a script beside a web server: {:.0} and {:.0} per second", a, b), a > 10.0 && b > 500.0);
    // the core is used up: the time of the calls and the requests fills a second
    let used = a / cap + b / 1000.0;
    check_near(t, "together they fill the core", used, 1.0, 0.05);
    // its effects keep their order: the answers come, none lost
    t.eq("no error", metric(&e, n.srv, "s80", "errors"), 0);
    t.eq("nothing failed", metric(&e, n.gen_, "load", "failed"), 0);
}

fn console(t: &mut T) {
    let n = net(Some(Hw { cores: 8, speed_pm: 1500, ram_mb: 16 << 10, swap_mb: 2 << 10, disk: DiskClass::Nvme, disk_gb: 480 }), &[web("web", 80, "4")], &closed(64));
    let mut e = engine(&n.p);
    run_for(&mut e, 5 * SEC);
    let o = out(&mut e, n.srv, "free");
    t.check(&format!("free: {}", o), o.contains("Mem:") && o.contains("16384") && o.contains("Swap:") && o.contains("2048"));
    let o = out(&mut e, n.srv, "lscpu");
    t.check(&format!("lscpu: {}", o), o.contains("CPU(s):        8") && o.contains("1.5 x"));
    let o = out(&mut e, n.srv, "top -b -n 1");
    t.check(&format!("top: {}", o), o.contains("MiB Mem :    16384 total") && o.contains("MiB Swap:     2048 total") && o.contains("web"));
    // 8 cores at 1.5: 3000 per second of 4 ms: all busy
    t.check(&format!("top sees its cores busy: {}", o), o.lines().find(|l| l.starts_with("%Cpu(s)")).is_some_and(|l| l.contains("100.0 us")));
    t.check("the desktop counts its cores", jfn_sim::cli::tools::cpus_of(&e, n.srv) == 8 && jfn_sim::cli::tools::cpus_of(&e, n.gen_) == jfn_sim::cli::tools::CPUS);
}

fn determinism(t: &mut T) {
    let load = |e: &mut Engine, n: &Net| {
        run_for(e, 2 * SEC);
        let _ = e.apply(Action::Fault { fault: Fault::SlowCpu { dev: n.srv, factor_pm: 2000 }, dur: Some(SEC) });
        run_for(e, 2 * SEC);
        let _ = e.apply(Action::Fault { fault: Fault::MemHog { dev: n.srv, bytes: 9 << 30 }, dur: Some(SEC) });
        run_for(e, 2 * SEC);
        let _ = e.apply(Action::Resize { dev: n.srv, hw: Some(hw(4, 1200, 8)), downtime: 0 });
        run_for(e, 2 * SEC);
        let _ = e.apply(Action::Resize { dev: n.srv, hw: Some(hw(2, 1000, 8)), downtime: SEC });
        run_for(e, 3 * SEC);
    };
    let n = net(Some(hw(2, 1000, 8)), &[web("web", 80, "2"), script_cfg(81, 500)], &open(800));
    // two runs
    let (mut a, mut b) = (engine(&n.p), engine(&n.p));
    load(&mut a, &n);
    load(&mut b, &n);
    t.eq("two runs: equal", digest(&a), digest(&b));
    t.check("and it ran", metric(&a, n.srv, "web", "responses") > 1000);
    // a clone in the middle goes on the same
    let mut c = engine(&n.p);
    run_for(&mut c, 3 * SEC);
    let mut d = c.clone();
    run_for(&mut c, 3 * SEC);
    run_for(&mut d, 3 * SEC);
    t.eq("a clone in the middle: equal", digest(&c), digest(&d));
    t.check("its queue too", c.res_of(n.srv).map(|r| r.busy_ns) == d.res_of(n.srv).map(|r| r.busy_ns));
    // a replay from the keyframes equals the run, inputs and all
    let mut drv = Inline::new(engine(&n.p));
    drv.rewind_on(Budget { gap_events: 2000, ..Budget::default() });
    let _ = drv.send(Cmd::RunFor(SEC), 0);
    let _ = drv.send(Cmd::Apply(Action::Fault { fault: Fault::SlowCpu { dev: n.srv, factor_pm: 2000 }, dur: Some(SEC) }), 0);
    let _ = drv.send(Cmd::RunFor(2 * SEC), 0);
    let _ = drv.send(Cmd::Apply(Action::Resize { dev: n.srv, hw: Some(hw(4, 1000, 8)), downtime: 500 * MS }), 0);
    let _ = drv.send(Cmd::RunFor(2 * SEC), 0);
    let _ = drv.reports();
    let made = drv.rewind().map_or(0, |r| r.made);
    t.check(&format!("keyframes were made: {}", made), made > 2);
    let pos = drv.pos();
    let got = drv.rewind().and_then(|rw| {
        let mut r = drv.restore_pos(pos, None)?;
        if r.run(rw, usize::MAX) { Some(r.finish()) } else { None }
    });
    t.eq("a replay from keyframes: equal", got.map(|p| (p.pos, digest(&p.eng))), Some((pos, digest(&drv.eng))));
    // what a keyframe costs counts it
    let f = jfn_sim::keyframe::footprint(&a);
    t.check(&format!("the footprint knows it: {} bytes", f), f > 0);
    let m = net(None, &[web("web", 80, "2")], "");
    t.check("the state of a project without hardware has none", engine(&m.p).node(m.srv).and_then(|d| d.os.as_ref()).is_some_and(|o| o.res.is_none()));
}

fn steps(t: &mut T) {
    let sc = scenario::parse("resize SRV1 model server-l\nresize SRV1 cores 8 speed 1.5 ram 32G swap 512M disk nvme disk_gb 960 down 10s\nresize SRV1 cores 2 live\nat 3s fault cpu SRV1 factor 2.5 for 5s\nfault mem SRV1 12G\nheal cpu SRV1\nheal mem SRV1\n");
    let Ok(sc) = sc else {
        t.check("the steps parse", false);
        return;
    };
    t.eq("as many steps", sc.steps.len(), 7);
    let hw0 = match &sc.steps[0].kind {
        StepKind::Resize { hw, down, .. } => Some((hw.model.clone(), *down)),
        _ => None,
    };
    t.eq("a model", hw0, Some((Some(String::from("server-l")), None)));
    match &sc.steps[1].kind {
        StepKind::Resize { hw, down, .. } => t.eq("single figures", (hw.cores, hw.speed_pm, hw.ram_mb, hw.swap_mb, hw.disk, hw.disk_gb, *down), (Some(8), Some(1500), Some(32 << 10), Some(512), Some(DiskClass::Nvme), Some(960), Some(10 * SEC))),
        _ => t.check("single figures", false),
    }
    t.check("live", matches!(&sc.steps[2].kind, StepKind::Resize { down: Some(0), .. }));
    t.check("a slow CPU", matches!(&sc.steps[3].kind, StepKind::Fault { fault: FaultSpec::Cpu { factor_pm: 2500, .. }, dur: Some(_) }));
    t.check("a memory hog", matches!(&sc.steps[4].kind, StepKind::Fault { fault: FaultSpec::Mem { bytes, .. }, .. } if *bytes == 12 << 30));
    t.check("heal cpu", matches!(&sc.steps[5].kind, StepKind::Heal(HealSpec::Like(FaultSpec::Cpu { .. }))));
    t.check("heal mem", matches!(&sc.steps[6].kind, StepKind::Heal(HealSpec::Like(FaultSpec::Mem { .. }))));
    for bad in ["resize SRV1", "resize SRV1 cores 0", "resize SRV1 ram x", "resize SRV1 disk floppy", "resize SRV1 foo", "fault cpu SRV1", "fault cpu SRV1 factor 0", "fault mem SRV1 viel"] {
        t.check(&format!("refused: {}", bad), scenario::parse(bad).is_err());
    }
    t.eq("a size", scenario::parse_size("1.5G"), Some(3 << 29));
    t.eq("a size in KiB", scenario::parse_size("64k"), Some(64 << 10));
    t.eq("a size with its unit", scenario::parse_size("2GiB"), Some(2 << 30));
    t.eq("no size", scenario::parse_size("viel"), None);
    // a run: the lines of a report, a window expectation on the host's series
    let n = net(Some(hw(1, 1000, 8)), &[web("web", 80, "10")], &closed(8));
    let name = n.p.device(n.srv).map_or(String::new(), |d| d.name.clone());
    let text = format!("run 3s\nexpect {0} svc _host metric cpu_pm > 900 over 2s\nexpect {0} svc _host metric cores == 1\nresize {0} cores 2 live\nrun 3s\nexpect {0} svc _host metric cores == 2\nresize {0} down 1s cores 4\nrun 2s\nexpect {0} svc web metric requests > 0\nfault cpu {0} factor 2\nrun 1s\nheal cpu {0}\nresize {0} model server-m\nrun 1s\nexpect {0} svc _host metric cores == 8\nresize {0} model nichts\n", name);
    let sc = scenario::parse(&text).unwrap_or_default();
    let o = jfn_sim::run_scenario(&n.p, &sc, "hw", false);
    t.eq("the expectations held", (o.expects, o.failed), (5, 0));
    t.check("the lines", o.report.contains("im Betrieb") && o.report.contains("1s Neustart") && o.report.contains("startet für neue Hardware neu") && o.report.contains("CPU von"));
    t.check("a model without figures fails the scenario", !o.ok && o.report.contains("das Modell nichts hat keine Hardware-Angaben"));
}

// ---------------------------------------------------------------- the window's edits

fn window(t: &mut T) {
    let mut doc = Doc::new();
    let n = net(None, &[web("web", 80, "1")], "");
    doc.replace(n.p.clone(), None);
    let srv = n.srv;
    // the project's switch: one undo step, the engine follows, no reboot
    let Some(e) = edits::metered(&doc.proj, true) else {
        t.check("the switch makes a step", false);
        return;
    };
    doc.apply(e);
    t.check("on", doc.proj.metered && doc.sim.engine().metered && doc.sim.engine().res_of(srv).is_some());
    t.check("no step when it is so already", edits::metered(&doc.proj, true).is_none());
    t.check("undo", doc.undo());
    t.check("off again", !doc.proj.metered && doc.sim.engine().res_of(srv).is_none());
    t.check("redo", doc.redo());
    t.check("on again", doc.proj.metered && doc.sim.engine().res_of(srv).is_some());
    // a device's figures: one undo step, a reboot unless it says otherwise
    let Some(e) = edits::device_hw(&doc.proj, srv, Some(hw(6, 1000, 8))) else {
        t.check("figures make a step", false);
        return;
    };
    doc.apply(e);
    t.eq("the project has them", doc.proj.device(srv).and_then(|d| d.hw.as_ref()).map(|h| h.cores), Some(6));
    t.check("the engine too", doc.sim.engine().res_of(srv).is_some_and(|r| r.hw.cores == 6 && r.own));
    t.check("it reboots", doc.sim.engine().node(srv).is_some_and(|d| !d.power));
    t.check("a mark on the timeline", doc.sim.engine().active_faults().any(|m| matches!(m.fault, Fault::Resize { .. })));
    t.check("no step for the same", edits::device_hw(&doc.proj, srv, Some(hw(6, 1000, 8))).is_none());
    // live: the downtime of the next change says so, once
    doc.sim.hw_down = 0;
    let Some(e) = edits::device_hw(&doc.proj, srv, Some(hw(2, 1000, 8))) else {
        t.check("a live step", false);
        return;
    };
    let marks = doc.sim.engine().faults().len();
    doc.apply(e);
    t.eq("live: no new mark", doc.sim.engine().faults().len(), marks);
    t.eq("live: the figures", doc.sim.engine().res_of(srv).map(|r| r.hw.cores), Some(2));
    t.eq("the next one reboots again", doc.sim.hw_down, jfn_sim::res::RESIZE_DOWN);
    t.check("undo takes the figures back", doc.undo() && doc.sim.engine().res_of(srv).is_some_and(|r| r.hw.cores == 6));
    // its label is one of the app's texts
    let e: Option<Box<dyn ui::Edit<Project>>> = edits::metered(&doc.proj, false);
    t.eq("the label", e.map(|x| x.label().to_string()), Some(String::from("Hardware simulieren ändern")));
}
