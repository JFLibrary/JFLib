// labtest -- checks of the distributed-systems lab (part of --selftest, group "lab"): load
// balancing (round robin, weighted, least connections, consistent hashing against an own
// ring), health checks that take a backend down and up at exact virtual times, draining, the
// TCP mode with a refused backend and a drain that runs out; the KV store (strong quorums,
// failure detection by heartbeats at exact times, eventual writes and reads, forwarding to
// the leader, a stale read and its read repair, a full disk, a slow disk to the nanosecond);
// web servers with the store behind them and the client's failover; the cache (hits at an
// exact time, misses, expiry, collapsed misses, invalidation); the queue (redelivery after
// exactly the visibility timeout, nack, dead letters, the durable publish); every kind of
// fault with its marks, overlapping and timed faults, partitions; fault and heal in
// scenarios; lbctl, kvctl, cachectl, mqctl, workerctl; the placeholders of the load
// generator; determinism
//
// Times: 1 Gbit/s copper with 50 ns of cable and a switch between the hosts (as httptest):
// a frame of n bytes takes (max(n, 60) + 24) * 8 + 50 ns per cable.  Every ARP entry is set
// by hand, so nothing waits for ARP.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use base::hashring::{hash64, Ring};
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_model::scenario::{self, FaultSpec, HealSpec, StepKind};
use jfn_sim::fault::combine_loss;
use jfn_sim::http::{Agent, Req, Url};
use jfn_sim::lb::Admin;
use jfn_sim::node::port_mac;
use jfn_sim::{Action, Cache, Engine, Fault, KvNode, Metric, Note, OsApi, PingOpts, Report, Service, SvcEvent, MS, SEC, US};
use netpkt::http::{Headers, Version};
use netpkt::{Ipv4, Mac, Prefix4};

use crate::selftest::T;

const BUDGET: usize = 50_000_000;
const SEED: u64 = 23;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn mac(dev: u32) -> Mac {
    port_mac(dev, 0)
}

/// the address of host i (0-based): 10.0.0.(i + 1)
fn addr(i: usize) -> Ipv4 {
    Ipv4([10, 0, 0, i as u8 + 1])
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: String::from(kind), name: String::from(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Value::Null) }
}

/// adds a service from a definition; true when the engine took it
fn add(e: &mut Engine, dev: u32, kind: &str, name: &str, config: &str) -> bool {
    e.apply(Action::Service { dev, def: def(kind, name, config) }).is_ok()
}

/// the project of `net`: hosts SRV1..SRVn ("server-1u") on SW1
fn net_project(n: usize) -> (Project, Vec<u32>) {
    let mut p = Project::new("lab", SEED);
    let sw = p.add_device("jf-s24p", 0, 200).unwrap_or(0);
    let hosts: Vec<u32> = (0..n).map(|i| p.add_device("server-1u", i as i32 * 100, 0).unwrap_or(0)).collect();
    for (i, h) in hosts.iter().enumerate() {
        let _ = p.connect(&pr(*h, "eth0"), &pr(sw, &format!("gi1/0/{}", i + 1)), None);
    }
    (p, hosts)
}

/// hosts SRV1..SRVn (10.0.0.1..n) on one switch, every ARP entry set by hand
fn net(n: usize) -> (Engine, Vec<u32>) {
    let (p, hosts) = net_project(n);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    for (i, h) in hosts.iter().enumerate() {
        let _ = e.apply(Action::IfAddr { dev: *h, iface: String::from("eth0"), prefix: Prefix4 { addr: addr(i), len: 24 }, add: true });
    }
    for (i, a) in hosts.iter().enumerate() {
        for (j, b) in hosts.iter().enumerate() {
            if i != j {
                let _ = e.apply(Action::StaticArp { dev: *a, ip: addr(j), mac: Some(mac(*b)) });
            }
        }
    }
    (e, hosts)
}

fn run_to(e: &mut Engine, t: u64) {
    e.run_until(t, BUDGET);
}

/// runs `d` of virtual time
fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
}

/// the project id of the cable at a device's first port
fn cable(e: &Engine, dev: u32) -> u32 {
    let ni = e.node_index(dev);
    (1..500).find(|&id| e.link_rt(id).is_some_and(|l| l.ends.iter().any(|end| Some(end.0) == ni && end.1 == 0))).unwrap_or(0)
}

fn metric(e: &Engine, dev: u32, svc: &str, name: &str) -> i64 {
    e.metric(dev, svc, name).unwrap_or(-1)
}

fn notes_of(e: &Engine, dev: u32, f: impl Fn(&Note) -> bool) -> Vec<(u64, Note)> {
    e.trace.events().filter(|x| x.dev == dev && f(&x.note)).map(|x| (x.t, x.note.clone())).collect()
}

/// what a device printed since the last call
fn out(e: &mut Engine, dev: u32) -> String {
    let mut s = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            s.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    s
}

/// a command line on a device and what it printed (it runs at once)
fn sh(e: &mut Engine, dev: u32, line: &str) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    run_for(e, MS);
    out(e, dev)
}

// ---------------------------------------------------------------- a client for the checks

/// an answer a Probe got
#[derive(Clone, Debug, PartialEq, Eq)]
struct Got {
    /// the number of the request in the plan
    i: usize,
    end: u64,
    status: u16,
    body: String,
    headers: Headers,
}

impl Got {
    fn header(&self, h: &str) -> String {
        String::from(self.headers.get(h).unwrap_or(""))
    }
}

/// a service that sends the requests of a plan at their times and keeps the answers
#[derive(Clone, Debug)]
struct Probe {
    plan: Vec<(u64, Req)>,
    agent: Agent,
    ids: BTreeMap<u64, usize>,
    next: usize,
    got: Vec<Got>,
}

impl Probe {
    fn new(plan: Vec<(u64, Req)>) -> Probe {
        Probe { plan, agent: Agent::new(1), ids: BTreeMap::new(), next: 0, got: Vec::new() }
    }
    fn arm(&mut self, os: &mut OsApi) {
        if let Some(p) = self.plan.get(self.next) {
            let at = p.0.max(os.now());
            os.timer(0, at);
        }
    }
}

impl Service for Probe {
    fn kind(&self) -> &'static str {
        "probe"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.arm(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if let SvcEvent::Timer(0) = ev {
            let now = os.now();
            while let Some(p) = self.plan.get(self.next) {
                if p.0 > now {
                    break;
                }
                let r = p.1.clone();
                let id = self.agent.submit(os, r);
                self.ids.insert(id, self.next);
                self.next += 1;
            }
            self.arm(os);
            return;
        }
        if self.agent.on(&ev, os) {
            for d in self.agent.take_done() {
                let i = self.ids.remove(&d.id).unwrap_or(usize::MAX);
                let g = match d.result {
                    Ok(r) => Got { i, end: d.end, status: r.head.status, body: String::from_utf8_lossy(&r.body).into_owned(), headers: r.head.headers },
                    Err(f) => Got { i, end: d.end, status: 0, body: f.text(), headers: Headers::new() },
                };
                self.got.push(g);
            }
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        vec![Metric::new("got", self.got.len() as i64)]
    }
    fn status(&self) -> String {
        format!("{} Antworten", self.got.len())
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

/// a request of a plan
fn req(method: &str, url: &str, body: &str) -> Req {
    let u = Url::parse(url).unwrap_or(Url { host: String::new(), port: 80, target: String::from("/") });
    Req { method: String::from(method), url: u, version: Version::Http11, headers: Headers::new(), body: Vec::from(body.as_bytes()), timeout: 0, connect_timeout: 0, reuse: true, keep_body: true }
}

/// starts a probe on a device
fn probe(e: &mut Engine, dev: u32, plan: Vec<(u64, Req)>) -> bool {
    e.add_service(dev, "probe", Box::new(Probe::new(plan)), true).is_ok()
}

/// the answers of a device's probe in the order of its plan
fn got(e: &Engine, dev: u32) -> Vec<Got> {
    let mut g = e.service::<Probe>(dev, "probe").map(|p| p.got.clone()).unwrap_or_default();
    g.sort_by_key(|x| x.i);
    g
}

fn served(g: &[Got]) -> Vec<String> {
    g.iter().map(|x| x.header("x-served-by")).collect()
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| String::from(*s)).collect()
}

pub fn run(t: &mut T) {
    lb_algorithms(t);
    lb_hash(t);
    lb_health(t);
    lb_drain(t);
    lb_tcp(t);
    kv_quorum(t);
    kv_repair(t);
    kv_disk(t);
    web_kv(t);
    cache_t(t);
    queue_t(t);
    faults_t(t);
    partition_t(t);
    scenario_t(t);
    shell_t(t);
    determinism(t);
}

// ---------------------------------------------------------------- load balancing

/// LB (10.0.0.1) before web servers on 10.0.0.2.. with their own times, a probe on the last
/// host
fn lb_net(lb: &str, times: &[u64]) -> (Engine, Vec<u32>, bool) {
    let (mut e, h) = net(times.len() + 2);
    let mut ok = add(&mut e, h[0], "lb", "lb", lb);
    for (i, tm) in times.iter().enumerate() {
        ok &= add(&mut e, h[i + 1], "http-server", "web", &format!("{{\"time_ms\": {}}}", tm));
    }
    (e, h, ok)
}

fn lb_algorithms(t: &mut T) {
    // round robin over three web servers
    let backends = "[{\"name\": \"web1\", \"addr\": \"10.0.0.2\"}, {\"name\": \"web2\", \"addr\": \"10.0.0.3\"}, {\"name\": \"web3\", \"addr\": \"10.0.0.4\"}]";
    let (mut e, h, ok) = lb_net(&format!("{{\"backends\": {}, \"health\": {{\"type\": \"none\"}}}}", backends), &[0, 0, 0]);
    t.check("rr: services", ok);
    let plan: Vec<(u64, Req)> = (1..=9).map(|k| (k * 10 * MS, req("GET", "http://10.0.0.1/", ""))).collect();
    t.check("rr: probe", probe(&mut e, h[4], plan));
    run_to(&mut e, SEC);
    let g = got(&e, h[4]);
    t.eq("rr: answers", g.len(), 9);
    t.eq("rr: order", served(&g), names(&["web1", "web2", "web3", "web1", "web2", "web3", "web1", "web2", "web3"]));
    t.eq("rr: bodies", g.iter().take(3).map(|x| x.body.clone()).collect::<Vec<_>>(), names(&["Hallo von SRV2\n", "Hallo von SRV3\n", "Hallo von SRV4\n"]));
    t.eq("rr: per backend", [metric(&e, h[0], "lb", "b1_requests"), metric(&e, h[0], "lb", "b2_requests"), metric(&e, h[0], "lb", "b3_requests")], [3, 3, 3]);
    t.eq("rr: totals", [metric(&e, h[0], "lb", "requests"), metric(&e, h[0], "lb", "responses"), metric(&e, h[0], "lb", "status_2xx"), metric(&e, h[0], "lb", "errors")], [9, 9, 9, 0]);
    t.eq("rr: picks noted", notes_of(&e, h[0], |n| matches!(n, Note::LbPick { algo: "round-robin", .. })).len(), 9);
    t.eq("rr: web2 served 3", metric(&e, h[2], "web", "requests"), 3);

    // weighted: 3 to 1, spread out (smooth weighted round robin: a a b a)
    let (mut e, h, ok) = lb_net("{\"algorithm\": \"weighted\", \"backends\": [{\"name\": \"a\", \"addr\": \"10.0.0.2\", \"weight\": 3}, {\"name\": \"b\", \"addr\": \"10.0.0.3\", \"weight\": 1}], \"health\": {\"type\": \"none\"}}", &[0, 0]);
    let plan: Vec<(u64, Req)> = (1..=8).map(|k| (k * 10 * MS, req("GET", "http://10.0.0.1/", ""))).collect();
    t.check("weighted: set up", ok && probe(&mut e, h[3], plan));
    run_to(&mut e, SEC);
    t.eq("weighted: order", served(&got(&e, h[3])), names(&["a", "a", "b", "a", "a", "a", "b", "a"]));

    // least connections: the slow server keeps its request, the fast one gets the rest
    let (mut e, h, ok) = lb_net("{\"algorithm\": \"least-conn\", \"backends\": [{\"name\": \"slow\", \"addr\": \"10.0.0.2\"}, {\"name\": \"fast\", \"addr\": \"10.0.0.3\"}], \"health\": {\"type\": \"none\"}}", &[300, 1]);
    let plan: Vec<(u64, Req)> = (1..=10).map(|k| (k * 20 * MS, req("GET", "http://10.0.0.1/", ""))).collect();
    t.check("least-conn: set up", ok && probe(&mut e, h[3], plan));
    run_to(&mut e, SEC);
    let g = got(&e, h[3]);
    t.eq("least-conn: order", served(&g), names(&["slow", "fast", "fast", "fast", "fast", "fast", "fast", "fast", "fast", "fast"]));
    // the same with round robin sends every other request to the slow one
    let (mut e, h, _) = lb_net("{\"backends\": [{\"name\": \"slow\", \"addr\": \"10.0.0.2\"}, {\"name\": \"fast\", \"addr\": \"10.0.0.3\"}], \"health\": {\"type\": \"none\"}}", &[300, 1]);
    let plan: Vec<(u64, Req)> = (1..=10).map(|k| (k * 20 * MS, req("GET", "http://10.0.0.1/", ""))).collect();
    probe(&mut e, h[3], plan);
    run_to(&mut e, 2 * SEC);
    t.eq("rr instead: half slow", served(&got(&e, h[3])).iter().filter(|s| *s == "slow").count(), 5);

    // configuration errors
    let mut e2 = net(1).0;
    let d = e2.devices().last().unwrap_or(0);
    t.check("lb: backends needed", !add(&mut e2, d, "lb", "x", "{}"));
    t.check("lb: unknown algorithm", !add(&mut e2, d, "lb", "x", "{\"algorithm\": \"fastest\", \"backends\": [{\"addr\": \"10.0.0.9\"}]}"));
    t.check("lb: twice the same backend", !add(&mut e2, d, "lb", "x", "{\"backends\": [{\"name\": \"a\", \"addr\": \"10.0.0.9\"}, {\"name\": \"a\", \"addr\": \"10.0.0.8\"}]}"));
}

fn lb_hash(t: &mut T) {
    let backends = "[{\"name\": \"web1\", \"addr\": \"10.0.0.2\"}, {\"name\": \"web2\", \"addr\": \"10.0.0.3\"}, {\"name\": \"web3\", \"addr\": \"10.0.0.4\"}]";
    let (mut e, h, ok) = lb_net(&format!("{{\"algorithm\": \"hash\", \"vnodes\": 32, \"backends\": {}, \"health\": {{\"type\": \"none\"}}}}", backends), &[0, 0, 0]);
    let paths: Vec<String> = (0..24).map(|k| format!("/k{}", k)).collect();
    let mut plan: Vec<(u64, Req)> = Vec::new();
    for (k, p) in paths.iter().enumerate() {
        plan.push(((k as u64 + 1) * 5 * MS, req("GET", &format!("http://10.0.0.1{}", p), "")));
        plan.push(((k as u64 + 1) * 5 * MS + 300 * MS, req("GET", &format!("http://10.0.0.1{}", p), "")));
    }
    plan.sort_by_key(|x| x.0);
    t.check("hash: set up", ok && probe(&mut e, h[4], plan));
    run_to(&mut e, SEC);
    // the ring the checks build themselves: the same names, vnodes and hash
    let mut ring: Ring<String> = Ring::new();
    for n in ["web1", "web2", "web3"] {
        ring.add(String::from(n), n, 32);
    }
    let g = got(&e, h[4]);
    let by_path = |g: &[Got], i: usize| g.get(i).map(|x| x.header("x-served-by")).unwrap_or_default();
    // the plan is sorted by time: first every path once, then every path again
    let n = paths.len();
    let mut want = Vec::new();
    let mut have = Vec::new();
    for k in 0..n {
        want.push(ring.owner(hash64(paths[k].as_bytes())).cloned().unwrap_or_default());
        have.push(by_path(&g, k));
    }
    t.eq("hash: as the ring says", have.clone(), want.clone());
    t.check("hash: the same path again", (0..n).all(|k| by_path(&g, k) == by_path(&g, n + k)));
    t.check("hash: all three used", ["web1", "web2", "web3"].iter().all(|n| want.iter().any(|w| w == n)));
    // web2 goes: only its paths move, to the next backend on the ring
    t.check("hash: remove web2", e.apply(Action::LbBackend { dev: h[0], svc: String::from("lb"), op: jfn_sim::LbOp::Remove { name: String::from("web2") } }).is_ok());
    t.check("hash: removed at once (nothing in flight)", e.lb_view(h[0], "lb").is_some_and(|v| v.backends.iter().all(|b| b.name != "web2")));
    let plan2: Vec<(u64, Req)> = paths.iter().enumerate().map(|(k, p)| (SEC + (k as u64 + 1) * 5 * MS, req("GET", &format!("http://10.0.0.1{}", p), ""))).collect();
    let _ = e.remove_service(h[4], "probe");
    probe(&mut e, h[4], plan2);
    run_to(&mut e, 2 * SEC);
    let g2 = got(&e, h[4]);
    let mut moved_ok = true;
    let mut stayed = 0;
    for k in 0..paths.len() {
        let before = &want[k];
        let after = by_path(&g2, k);
        let next = ring.preference_by(hash64(paths[k].as_bytes()), 1, |m| m != "web2").first().cloned().unwrap_or_default();
        if before != "web2" {
            stayed += (after == *before) as usize;
            moved_ok &= after == *before;
        } else {
            moved_ok &= after == next;
        }
    }
    t.check("hash: web1 and web3 keep their paths, web2's go to the next", moved_ok);
    t.eq("hash: kept paths", stayed, want.iter().filter(|w| *w != "web2").count());
}

fn lb_health(t: &mut T) {
    // checks every second with half a second to answer; down after 3 failed, up after 2
    let lb = "{\"backends\": [{\"name\": \"web1\", \"addr\": \"10.0.0.2\"}, {\"name\": \"web2\", \"addr\": \"10.0.0.3\"}], \"timeout_ms\": 400, \"connect_timeout_ms\": 300, \"health\": {\"type\": \"http\", \"path\": \"/\", \"interval_ms\": 1000, \"timeout_ms\": 500, \"rise\": 2, \"fall\": 3}}";
    let (mut e, h, ok) = lb_net(lb, &[0, 0]);
    let plan: Vec<(u64, Req)> = (1..=40).map(|k| (k * 250 * MS, req("GET", "http://10.0.0.1/", ""))).collect();
    t.check("health: set up", ok && probe(&mut e, h[3], plan));
    run_to(&mut e, 2200 * MS);
    let view = |e: &Engine| e.lb_view(h[0], "lb").map(|v| v.backends).unwrap_or_default();
    t.eq("health: checks so far", view(&e).iter().map(|b| b.checks).collect::<Vec<_>>(), vec![3, 3]);
    t.check("health: both up", view(&e).iter().all(|b| b.up));
    t.check("health: web2 off", e.apply(Action::Fault { fault: Fault::Power { dev: h[2] }, dur: None }).is_ok());
    run_to(&mut e, 5400 * MS);
    t.check("health: still up after two failed checks", view(&e).get(1).is_some_and(|b| b.up && b.check_fails == 2));
    run_to(&mut e, 6200 * MS);
    let b2 = view(&e).get(1).cloned();
    t.eq("health: down after the third failed check, at 5.5 s", b2.as_ref().map(|b| (b.up, b.since, b.downs)), Some((false, 5500 * MS, 1)));
    t.eq("health: noted", notes_of(&e, h[0], |n| matches!(n, Note::LbHealth { up: false, .. })).iter().map(|x| x.0).collect::<Vec<_>>(), vec![5500 * MS]);
    t.check("health: logged", e.log_lines(h[0]).iter().any(|l| l.tag == "lb" && l.text.contains("web2 fällt aus")));
    t.eq("health: no backend lost a request", metric(&e, h[0], "lb", "errors"), 0);
    t.check("health: requests to the dead one went again", metric(&e, h[0], "lb", "retries") > 0);
    // power back: the checks at 7 s and 8 s succeed
    t.check("health: heal", e.apply(Action::HealAll).is_ok());
    run_to(&mut e, 7500 * MS);
    t.check("health: one success is not enough", view(&e).get(1).is_some_and(|b| !b.up));
    run_to(&mut e, 9 * SEC);
    // the check at 8 s: a new connection (SYN and SYN-ACK 1444 ns each over the switch, the
    // ACK 672 ns ahead of the request), GET / and the answer: 14 156 ns
    let b2 = view(&e).get(1).cloned();
    t.eq("health: up again with the check at 8 s", b2.as_ref().map(|b| (b.up, b.since)), Some((true, 8 * SEC + 14_156)));
    t.eq("health: noted then", notes_of(&e, h[0], |n| matches!(n, Note::LbHealth { up: true, .. })).iter().map(|x| x.0).collect::<Vec<_>>(), vec![8 * SEC + 14_156]);
    run_to(&mut e, 11 * SEC);
    let g = got(&e, h[3]);
    t.eq("health: every request answered", (g.len(), g.iter().filter(|x| x.status == 200).count()), (40, 40));
    // while web2 counted as down, web1 took everything
    let mid: Vec<String> = g.iter().filter(|x| x.end > 5600 * MS && x.end < 8 * SEC).map(|x| x.header("x-served-by")).collect();
    t.check("health: only web1 while web2 is down", !mid.is_empty() && mid.iter().all(|s| s == "web1"));
    let late: Vec<String> = g.iter().filter(|x| x.end > 8100 * MS).map(|x| x.header("x-served-by")).collect();
    t.check("health: both again afterwards", late.iter().any(|s| s == "web1") && late.iter().any(|s| s == "web2"));
}

fn lb_drain(t: &mut T) {
    let lb = "{\"backends\": [{\"name\": \"slow\", \"addr\": \"10.0.0.2\"}, {\"name\": \"fast\", \"addr\": \"10.0.0.3\"}], \"health\": {\"type\": \"none\"}}";
    let (mut e, h, ok) = lb_net(lb, &[500, 1]);
    let plan = vec![(100 * MS, req("GET", "http://10.0.0.1/a", "")), (300 * MS, req("GET", "http://10.0.0.1/b", "")), (400 * MS, req("GET", "http://10.0.0.1/c", ""))];
    t.check("drain: set up", ok && probe(&mut e, h[3], plan));
    run_to(&mut e, 200 * MS);
    t.check("drain: remove the slow one", e.apply(Action::LbBackend { dev: h[0], svc: String::from("lb"), op: jfn_sim::LbOp::Remove { name: String::from("slow") } }).is_ok());
    let b = e.lb_view(h[0], "lb").map(|v| v.backends).unwrap_or_default();
    t.eq("drain: draining with its request", b.first().map(|x| (x.admin, x.active)), Some((Admin::Draining { since: 200 * MS, remove: true }, 1)));
    run_to(&mut e, SEC);
    let g = got(&e, h[3]);
    t.eq("drain: who served", served(&g), names(&["slow", "fast", "fast"]));
    let done = notes_of(&e, h[0], |n| matches!(n, Note::LbDrained { .. }));
    t.eq("drain: gone when its request was answered", done.len(), 1);
    // the slow request came at 100 ms + its way to the LB and on to the backend; 500 ms of
    // work and the way back to the LB: 600 ms + 16 012 ns
    t.eq("drain: gone at the answer of the slow request", done.first().map(|d| d.0), Some(600 * MS + 16_012));
    t.eq("drain: the view", e.lb_view(h[0], "lb").map(|v| v.backends.iter().map(|b| b.name.clone()).collect::<Vec<_>>()), Some(names(&["fast"])));
    t.eq("drain: backends metric", metric(&e, h[0], "lb", "backends"), 1);
    // add it back, and change weights and the algorithm
    let add_op = jfn_sim::LbOp::Add { name: String::from("slow"), addr: addr(1).into(), port: 80, weight: 2 };
    t.check("drain: add again", e.apply(Action::LbBackend { dev: h[0], svc: String::from("lb"), op: add_op.clone() }).is_ok());
    t.check("drain: not twice", e.apply(Action::LbBackend { dev: h[0], svc: String::from("lb"), op: add_op }).is_err());
    t.check("drain: unknown backend", e.apply(Action::LbBackend { dev: h[0], svc: String::from("lb"), op: jfn_sim::LbOp::Drain { name: String::from("nope") } }).is_err());
    t.check("drain: algorithm", e.apply(Action::LbAlgorithm { dev: h[0], svc: String::from("lb"), algo: jfn_sim::Algo::LeastConn }).is_ok() && e.lb_view(h[0], "lb").is_some_and(|v| v.algo == jfn_sim::Algo::LeastConn));
    t.check("drain: wrong service kind", e.apply(Action::LbAlgorithm { dev: h[1], svc: String::from("web"), algo: jfn_sim::Algo::Hash }).is_err());
}

fn lb_tcp(t: &mut T) {
    // three echo backends, the third one has nothing on its port: refused, the next one takes it
    let (mut e, h) = net(5);
    let lb = "{\"mode\": \"tcp\", \"port\": 7, \"backends\": [{\"name\": \"e1\", \"addr\": \"10.0.0.2\"}, {\"name\": \"e2\", \"addr\": \"10.0.0.3\"}, {\"name\": \"e3\", \"addr\": \"10.0.0.4\"}], \"health\": {\"type\": \"none\"}, \"drain_ms\": 500}";
    let mut ok = add(&mut e, h[0], "lb", "lb", lb);
    ok &= add(&mut e, h[1], "tcp-echo", "echo", "{}");
    ok &= add(&mut e, h[2], "tcp-echo", "echo", "{}");
    for (k, d) in [0u64, 50, 100].iter().enumerate() {
        ok &= add(&mut e, h[4], "tcp-client", &format!("c{}", k + 1), &format!("{{\"server\": \"10.0.0.1\", \"port\": 7, \"bytes\": 3000, \"delay_ms\": {}}}", d));
    }
    t.check("tcp: set up", ok);
    run_to(&mut e, SEC);
    for c in ["c1", "c2", "c3"] {
        t.eq(&format!("tcp: {} echoed", c), (metric(&e, h[4], c, "received"), metric(&e, h[4], c, "done")), (3000, 1));
    }
    t.eq("tcp: connections", metric(&e, h[0], "lb", "connections"), 3);
    t.eq("tcp: per backend", [metric(&e, h[0], "lb", "b1_requests"), metric(&e, h[0], "lb", "b2_requests"), metric(&e, h[0], "lb", "b3_requests")], [2, 1, 1]);
    t.eq("tcp: the refused one", (metric(&e, h[0], "lb", "b3_errors"), metric(&e, h[0], "lb", "retries")), (1, 1));
    t.eq("tcp: echo servers", (metric(&e, h[1], "echo", "connections"), metric(&e, h[2], "echo", "connections")), (2, 1));
    t.eq("tcp: nothing open", metric(&e, h[0], "lb", "active"), 0);
    // a connection that stays open: removing its backend cuts it after drain_ms
    t.check("tcp: a client that stays", add(&mut e, h[4], "tcp-client", "keep", "{\"server\": \"10.0.0.1\", \"port\": 7, \"bytes\": 100, \"close\": \"keep\"}"));
    run_to(&mut e, 2 * SEC);
    let at = e.lb_view(h[0], "lb").and_then(|v| v.backends.iter().find(|b| b.active == 1).map(|b| b.name.clone())).unwrap_or_default();
    t.eq("tcp: it went to e2 (round robin)", at.as_str(), "e2");
    t.check("tcp: remove e2", e.apply(Action::LbBackend { dev: h[0], svc: String::from("lb"), op: jfn_sim::LbOp::Remove { name: String::from("e2") } }).is_ok());
    run_to(&mut e, 3 * SEC);
    t.eq("tcp: cut after drain_ms", notes_of(&e, h[0], |n| matches!(n, Note::LbDrained { .. })).iter().map(|x| x.0).collect::<Vec<_>>(), vec![2500 * MS]);
    t.eq("tcp: the client saw its end", metric(&e, h[4], "keep", "failed"), 1);
    // a large echo through the pipe with a small receive buffer at the client: both sides
    // wait for each other, and the backend's orderly end still brings every byte
    t.check("tcp: a big one", add(&mut e, h[4], "tcp-client", "big", "{\"server\": \"10.0.0.1\", \"port\": 7, \"bytes\": 200000, \"rcvbuf\": 4096}"));
    run_to(&mut e, 4 * SEC);
    t.eq("tcp: 200 kB echoed", (metric(&e, h[4], "big", "received"), metric(&e, h[4], "big", "done")), (200_000, 1));
    t.eq("tcp: nothing left open", metric(&e, h[0], "lb", "active"), 0);
}

// ---------------------------------------------------------------- the KV store

const KV3: &str = "\"nodes\": [\"10.0.0.1\", \"10.0.0.2\", \"10.0.0.3\"]";

/// the device of a KV node's address among hosts 10.0.0.1..
fn host_of(h: &[u32], a: Ipv4) -> u32 {
    h.get(a.0[3] as usize - 1).copied().unwrap_or(0)
}

fn kv_quorum(t: &mut T) {
    let (mut e, h) = net(4);
    let mut ok = true;
    for d in &h[..3] {
        ok &= add(&mut e, *d, "kv", "kv", &format!("{{{}, \"rf\": 2, \"consistency\": \"strong\"}}", KV3));
    }
    t.check("kv: set up", ok);
    let reps = e.service::<KvNode>(h[0], "kv").map(|k| k.replicas("alpha")).unwrap_or_default();
    t.eq("kv: two replicas", reps.len(), 2);
    let (lead, follow) = (reps.first().copied().unwrap_or(addr(0)), reps.get(1).copied().unwrap_or(addr(1)));
    let other = (0..3).map(addr).find(|a| !reps.contains(a)).unwrap_or(addr(2));
    t.eq("kv: quorums of strong", e.service::<KvNode>(h[0], "kv").map(|k| k.quorums(None, None, None)), Some((2, 2)));
    let url = |a: Ipv4, q: &str| format!("http://{}:7000/kv/alpha{}", a, q);
    let plan = vec![
        // a write through the node that is no replica: forwarded to the leader
        (100 * MS, req("PUT", &url(other, ""), "eins")),
        (200 * MS, req("GET", &url(other, ""), "")),
        (300 * MS, req("GET", &url(follow, ""), "")),
        // the follower is off from 1.2 s and counts as down from 3 s
        (3500 * MS, req("PUT", &url(lead, ""), "zwei")),
        (3600 * MS, req("PUT", &url(lead, "?consistency=eventual"), "drei")),
        (3700 * MS, req("GET", &url(lead, ""), "")),
        (3800 * MS, req("GET", &url(lead, "?r=1"), "")),
        (3900 * MS, req("DELETE", &url(lead, "?w=1"), "")),
        (4000 * MS, req("GET", &url(lead, "?r=1"), "")),
        (4100 * MS, req("GET", &format!("http://{}:7000/kv", lead), "")),
    ];
    t.check("kv: probe", probe(&mut e, h[3], plan));
    run_to(&mut e, 1200 * MS);
    let g = got(&e, h[3]);
    t.eq("kv: write", g.first().map(|x| (x.status, x.header("x-acks"))), Some((201, String::from("2"))));
    t.eq("kv: read through any node", g.get(1).map(|x| (x.status, x.body.clone())), Some((200, String::from("eins"))));
    t.eq("kv: read at the follower", g.get(2).map(|x| (x.status, x.body.clone())), Some((200, String::from("eins"))));
    let lead_dev = host_of(&h, lead);
    let follow_dev = host_of(&h, follow);
    let other_dev = host_of(&h, other);
    let lead_no = (lead.0[3]) as u16;
    t.eq("kv: version of the leader", g.first().map(|x| x.header("x-version")), Some(format!("1.{}", lead_no)));
    t.eq("kv: forwarded", (metric(&e, other_dev, "kv", "forwards"), metric(&e, lead_dev, "kv", "led")), (1, 1));
    t.eq("kv: both replicas hold it", [lead_dev, follow_dev, other_dev].iter().map(|d| e.service::<KvNode>(*d, "kv").and_then(|k| k.entry("alpha")).map(|x| x.value.clone())).collect::<Vec<_>>(), vec![Some(Vec::from(&b"eins"[..])), Some(Vec::from(&b"eins"[..])), None]);
    t.check("kv: follower off", e.apply(Action::Fault { fault: Fault::Power { dev: follow_dev }, dur: None }).is_ok());
    run_to(&mut e, 2900 * MS);
    t.check("kv: not yet down at 2.9 s", e.kv_view(lead_dev, "kv").is_some_and(|v| v.members.iter().all(|m| m.up)));
    run_to(&mut e, 3 * SEC);
    let downs = notes_of(&e, lead_dev, |n| matches!(n, Note::KvMember { up: false, .. }));
    t.eq("kv: down at 3 s (last heartbeat at 1 s)", downs.iter().map(|x| x.0).collect::<Vec<_>>(), vec![3 * SEC]);
    t.eq("kv: the other node too", notes_of(&e, other_dev, |n| matches!(n, Note::KvMember { up: false, .. })).iter().map(|x| x.0).collect::<Vec<_>>(), vec![3 * SEC]);
    run_to(&mut e, 5 * SEC);
    let g = got(&e, h[3]);
    t.eq("kv: strong write with one replica", g.get(3).map(|x| x.status), Some(503));
    t.eq("kv: eventual write", g.get(4).map(|x| (x.status, x.header("x-acks"))), Some((201, String::from("1"))));
    t.eq("kv: strong read with one replica", g.get(5).map(|x| x.status), Some(503));
    t.eq("kv: read with r=1", g.get(6).map(|x| (x.status, x.body.clone())), Some((200, String::from("drei"))));
    t.eq("kv: delete", g.get(7).map(|x| x.status), Some(204));
    t.eq("kv: gone", g.get(8).map(|x| x.status), Some(404));
    t.eq("kv: no key", g.get(9).map(|x| x.status), Some(400));
    t.eq("kv: counted", [metric(&e, lead_dev, "kv", "unavailable"), metric(&e, lead_dev, "kv", "not_found"), metric(&e, lead_dev, "kv", "keys"), metric(&e, lead_dev, "kv", "members_up")], [2, 1, 0, 2]);
    t.eq("kv: change the consistency", e.apply(Action::KvSet { dev: lead_dev, svc: String::from("kv"), consistency: Some(jfn_sim::Consistency::Eventual), w: None, r: None }).is_ok(), true);
    t.eq("kv: eventual quorums", e.service::<KvNode>(lead_dev, "kv").map(|k| k.quorums(None, None, None)), Some((1, 1)));
    t.check("kv: nodes needed", !add(&mut e, h[3], "kv", "x", "{}") && !add(&mut e, h[3], "kv", "x", "{\"nodes\": [\"10.0.0.1\"], \"consistency\": \"maybe\"}"));
}

fn kv_repair(t: &mut T) {
    // eventual: the follower misses a write while it is off, then answers a read with its old
    // value first; the other replica's newer one repairs it
    let (mut e, h) = net(4);
    let cfg = format!("{{{}, \"rf\": 2, \"consistency\": \"eventual\", \"disk\": {{\"latency_us\": 20, \"iops\": 0}}}}", KV3);
    let mut ok = true;
    for d in &h[..3] {
        ok &= add(&mut e, *d, "kv", "kv", &cfg);
    }
    let reps = e.service::<KvNode>(h[0], "kv").map(|k| k.replicas("k")).unwrap_or_default();
    let (lead, follow) = (reps.first().copied().unwrap_or(addr(0)), reps.get(1).copied().unwrap_or(addr(1)));
    let (lead_dev, follow_dev) = (host_of(&h, lead), host_of(&h, follow));
    let plan = vec![
        (100 * MS, req("PUT", &format!("http://{}:7000/kv/k", lead), "v1")),
        (3 * SEC, req("PUT", &format!("http://{}:7000/kv/k", lead), "v2")),
        (5 * SEC, req("GET", &format!("http://{}:7000/kv/k", follow), "")),
        (5500 * MS, req("GET", &format!("http://{}:7000/kv/k", follow), "")),
    ];
    t.check("repair: set up", ok && probe(&mut e, h[3], plan));
    run_to(&mut e, 500 * MS);
    t.check("repair: follower off", e.apply(Action::Fault { fault: Fault::Power { dev: follow_dev }, dur: None }).is_ok());
    run_to(&mut e, 4 * SEC);
    t.eq("repair: the follower kept v1 on its disk", e.service::<KvNode>(follow_dev, "kv").and_then(|k| k.entry("k")).map(|x| x.value.clone()), Some(Vec::from(&b"v1"[..])));
    t.eq("repair: the leader has v2", e.service::<KvNode>(lead_dev, "kv").and_then(|k| k.entry("k")).map(|x| (x.value.clone(), x.ver.n)), Some((Vec::from(&b"v2"[..]), 2)));
    t.check("repair: follower on", e.apply(Action::HealAll).is_ok());
    run_to(&mut e, 6 * SEC);
    let g = got(&e, h[3]);
    t.eq("repair: the stale read", g.get(2).map(|x| (x.status, x.body.clone())), Some((200, String::from("v1"))));
    t.eq("repair: then the new value", g.get(3).map(|x| (x.status, x.body.clone())), Some((200, String::from("v2"))));
    t.eq("repair: counted", [metric(&e, follow_dev, "kv", "stale_reads"), metric(&e, follow_dev, "kv", "read_repairs"), metric(&e, follow_dev, "kv", "repaired")], [1, 1, 1]);
    t.eq("repair: noted", notes_of(&e, follow_dev, |n| matches!(n, Note::KvReadRepair { .. })).len(), 1);
    t.eq("repair: the follower holds v2 now", e.service::<KvNode>(follow_dev, "kv").and_then(|k| k.entry("k")).map(|x| x.value.clone()), Some(Vec::from(&b"v2"[..])));
    let w = e.kv_where(follow_dev, "kv", "k");
    t.eq("repair: where", w.iter().map(|r| (r.addr, r.leader, r.up, r.entry.as_ref().map(|x| x.ver.n))).collect::<Vec<_>>(), vec![(lead, true, true, Some(2)), (follow, false, true, Some(2))]);
}

fn kv_disk(t: &mut T) {
    // one node, a disk of 1 MiB / 1000 (1048 bytes): three entries of 318 bytes fit
    let (mut e, h) = net(2);
    let ok = add(&mut e, h[0], "kv", "kv", "{\"nodes\": [\"10.0.0.1\"], \"rf\": 1, \"disk\": {\"capacity_mb\": 0.001, \"latency_us\": 1000, \"iops\": 0}}");
    let v300 = "x".repeat(300);
    let mut plan: Vec<(u64, Req)> = (1..=4).map(|k| (k * 100 * MS, req("PUT", &format!("http://10.0.0.1:7000/kv/k{}", k), &v300))).collect();
    plan.push((SEC, req("PUT", "http://10.0.0.1:7000/kv/k1", "y")));
    plan.push((1100 * MS, req("PUT", "http://10.0.0.1:7000/kv/k4", "z")));
    // the same write before and during a slow-disk fault (20 ms more per access)
    plan.push((2 * SEC, req("PUT", "http://10.0.0.1:7000/kv/s", "1")));
    plan.push((3 * SEC, req("PUT", "http://10.0.0.1:7000/kv/s", "2")));
    t.check("disk: set up", ok && probe(&mut e, h[1], plan));
    run_to(&mut e, 2500 * MS);
    let g = got(&e, h[1]);
    t.eq("disk: until it is full", g.iter().take(4).map(|x| x.status).collect::<Vec<_>>(), vec![201, 201, 201, 507]);
    t.eq("disk: smaller value fits again", g.get(4).map(|x| x.status), Some(201));
    t.eq("disk: and then the fourth", g.get(5).map(|x| x.status), Some(201));
    t.eq("disk: full counted", metric(&e, h[0], "kv", "full"), 1);
    let used = e.node(h[0]).and_then(|n| n.os.as_ref()).map(|o| o.disk.used());
    t.eq("disk: bytes on the disk", used, Some((2 * 318 + (2 + 1 + 16) + (2 + 1 + 16) + (1 + 1 + 16)) as u64));
    t.eq("disk: what the node counts", metric(&e, h[0], "kv", "bytes"), used.unwrap_or(0) as i64);
    t.check("disk: slow", e.apply(Action::Fault { fault: Fault::SlowDisk { dev: h[0], extra: 20 * MS, factor_pm: 1000 }, dur: None }).is_ok());
    run_to(&mut e, 4 * SEC);
    let g = got(&e, h[1]);
    let lat = |i: usize, sent: u64| g.get(i).map(|x| x.end - sent).unwrap_or(0);
    t.eq("disk: a slow disk adds exactly 20 ms", lat(7, 3 * SEC) - lat(6, 2 * SEC), 20 * MS);
    t.check("disk: one millisecond of latency", lat(6, 2 * SEC) > MS && lat(6, 2 * SEC) < 2 * MS);
    t.eq("disk: full noted", notes_of(&e, h[0], |n| matches!(n, Note::DiskFull)).len(), 1);
}

fn web_kv(t: &mut T) {
    // a web server with the store behind it; the first replica of the key goes off: the
    // client inside the web server asks the next one
    let (mut e, h) = net(5);
    let kv_nodes = "\"nodes\": [\"10.0.0.2\", \"10.0.0.3\", \"10.0.0.4\"]";
    let mut ok = add(&mut e, h[0], "http-server", "web", &format!("{{\"routes\": [{{\"path\": \"/item/*\", \"method\": \"*\", \"backend\": \"kv\"}}], \"kv\": {{{}, \"rf\": 2}}}}", kv_nodes));
    for d in &h[1..4] {
        ok &= add(&mut e, *d, "kv", "kv", &format!("{{{}, \"rf\": 2, \"consistency\": \"eventual\"}}", kv_nodes));
    }
    let plan = vec![(100 * MS, req("PUT", "http://10.0.0.1/item/a", "x")), (200 * MS, req("GET", "http://10.0.0.1/item/a", "")), (SEC, req("GET", "http://10.0.0.1/item/a", "")), (2500 * MS, req("GET", "http://10.0.0.1/item/a", "")), (2600 * MS, req("GET", "http://10.0.0.1/item/", ""))];
    t.check("web+kv: set up", ok && probe(&mut e, h[4], plan));
    let reps = e.service::<KvNode>(h[1], "kv").map(|k| k.replicas("a")).unwrap_or_default();
    let first = reps.first().copied().unwrap_or(addr(1));
    let second = reps.get(1).copied().unwrap_or(addr(2));
    run_to(&mut e, 500 * MS);
    let g = got(&e, h[4]);
    t.eq("web+kv: written", g.first().map(|x| (x.status, x.header("x-kv-node"))), Some((201, format!("{}", first))));
    t.eq("web+kv: read", g.get(1).map(|x| (x.status, x.body.clone())), Some((200, String::from("x"))));
    t.check("web+kv: first replica off", e.apply(Action::Fault { fault: Fault::Power { dev: host_of(&h, first) }, dur: None }).is_ok());
    run_to(&mut e, 3 * SEC);
    let g = got(&e, h[4]);
    let third = g.get(2).cloned();
    t.eq("web+kv: failed over", third.as_ref().map(|x| (x.status, x.body.clone(), x.header("x-kv-node"))), Some((200, String::from("x"), format!("{}", second))));
    // the request reached the web server 2600 ns after 1 s; its client waited 1 s for the
    // first replica, then a new connection to the second one, its disk and the way back
    t.eq("web+kv: after the client's timeout of 1 s", third.as_ref().map(|x| x.end), Some(2 * SEC + 138_040));
    // the first one counts as down until 4 s: straight to the second, over its connection
    t.eq("web+kv: then at once", g.get(3).map(|x| (x.status, x.end)), Some((200, 2500 * MS + 134_480)));
    t.eq("web+kv: no key", g.get(4).map(|x| x.status), Some(400));
    t.eq("web+kv: numbers", [metric(&e, h[0], "web", "failovers"), metric(&e, h[0], "web", "backend_calls"), metric(&e, h[0], "web", "backend_errors")], [1, 4, 0]);
    t.check("web+kv: route without kv refused", !add(&mut e, h[0], "http-server", "w2", "{\"port\": 81, \"routes\": [{\"path\": \"/x\", \"backend\": \"kv\"}]}"));
}

// ---------------------------------------------------------------- cache

fn cache_t(t: &mut T) {
    let (mut e, h) = net(3);
    let mut ok = add(&mut e, h[0], "cache", "cache", "{\"backend\": \"http://10.0.0.2/\", \"ttl_ms\": 1000, \"hit_us\": 50}");
    ok &= add(&mut e, h[1], "http-server", "web", "{\"time_ms\": 10, \"routes\": [{\"path\": \"/a\", \"body\": \"A\"}, {\"path\": \"/b\", \"body\": \"B\"}, {\"path\": \"/a\", \"method\": \"POST\", \"status\": 204}, {\"path\": \"/n\", \"body\": \"nie\", \"headers\": {\"Cache-Control\": \"no-store\"}}]}");
    let plan = vec![
        (100 * MS, req("GET", "http://10.0.0.1/a", "")),
        (200 * MS, req("GET", "http://10.0.0.1/a", "")),
        (1500 * MS, req("GET", "http://10.0.0.1/a", "")),
        (2 * SEC, req("GET", "http://10.0.0.1/b", "")),
        (2 * SEC, req("GET", "http://10.0.0.1/b", "")),
        (2500 * MS, req("POST", "http://10.0.0.1/a", "neu")),
        (2600 * MS, req("GET", "http://10.0.0.1/a", "")),
        (2700 * MS, req("GET", "http://10.0.0.1/n", "")),
        (2800 * MS, req("GET", "http://10.0.0.1/n", "")),
    ];
    t.check("cache: set up", ok && probe(&mut e, h[2], plan));
    run_to(&mut e, 4 * SEC);
    let g = got(&e, h[2]);
    t.eq("cache: hit and miss", g.iter().map(|x| (x.status, x.header("x-cache"))).collect::<Vec<_>>(), vec![(200, String::from("MISS")), (200, String::from("HIT")), (200, String::from("MISS")), (200, String::from("MISS")), (200, String::from("MISS")), (204, String::new()), (200, String::from("MISS")), (200, String::from("MISS")), (200, String::from("MISS"))]);
    t.eq("cache: body of a hit", g.get(1).map(|x| x.body.clone()), Some(String::from("A")));
    t.eq("cache: counted", [metric(&e, h[0], "cache", "hits"), metric(&e, h[0], "cache", "misses"), metric(&e, h[0], "cache", "collapsed"), metric(&e, h[0], "cache", "expired"), metric(&e, h[0], "cache", "invalidations")], [1, 7, 1, 1, 1]);
    t.eq("cache: the backend asked", (metric(&e, h[0], "cache", "backend_requests"), metric(&e, h[1], "web", "requests")), (7, 7));
    t.eq("cache: a hit takes hit_us at the cache", metric(&e, h[0], "cache", "hit_p50_us"), 50);
    t.check("cache: a miss takes the backend's 10 ms", metric(&e, h[0], "cache", "miss_p50_us") >= 10_000);
    t.check("cache: the hit is faster at the client", g.get(1).zip(g.first()).is_some_and(|(b, a)| b.end - 200 * MS + 9 * MS < a.end - 100 * MS));
    t.eq("cache: no-store is not kept", e.service::<Cache>(h[0], "cache").map(|c| c.keys()), Some(names(&["/b", "/a"])));
    t.eq("cache: hit per mille", metric(&e, h[0], "cache", "hit_pm"), 125);
    t.check("cache: clear", e.apply(Action::CacheClear { dev: h[0], svc: String::from("cache") }).is_ok() && metric(&e, h[0], "cache", "entries") == 0);
}

// ---------------------------------------------------------------- queue

fn queue_t(t: &mut T) {
    let (mut e, h) = net(4);
    let mut ok = add(&mut e, h[0], "mq", "mq", "{\"visibility_ms\": 1000, \"max_deliveries\": 3}");
    ok &= add(&mut e, h[1], "worker", "worker", "{\"queue\": \"http://10.0.0.1:9000/\", \"topic\": \"jobs\", \"time_ms\": 100, \"wait_ms\": 2000}");
    ok &= add(&mut e, h[3], "worker", "bad", "{\"queue\": \"http://10.0.0.1:9000/\", \"topic\": \"bad\", \"time_ms\": 10, \"error_rate\": 1}");
    let plan = vec![(100 * MS, req("POST", "http://10.0.0.1:9000/topics/jobs", "m1")), (SEC, req("POST", "http://10.0.0.1:9000/topics/jobs", "m2")), (1100 * MS, req("POST", "http://10.0.0.1:9000/topics/bad", "m3")), (1200 * MS, req("GET", "http://10.0.0.1:9000/topics/jobs", "")), (1300 * MS, req("POST", "http://10.0.0.1:9000/topics/jobs/ack/77", ""))];
    t.check("mq: set up", ok && probe(&mut e, h[2], plan));
    run_to(&mut e, 1050 * MS);
    t.eq("mq: the first one done", (metric(&e, h[1], "worker", "done"), metric(&e, h[0], "mq", "acked")), (1, 1));
    // the worker stops while it works on m2: the queue hands it out again after 1 s
    t.check("mq: stop the worker", e.apply(Action::Fault { fault: Fault::Stop { dev: h[1], svc: String::from("worker") }, dur: None }).is_ok());
    run_to(&mut e, 1500 * MS);
    t.check("mq: worker on again", e.apply(Action::HealAll).is_ok());
    run_to(&mut e, 3 * SEC);
    let dl = notes_of(&e, h[0], |n| matches!(n, Note::MqDeliver { id: 2, .. }));
    let rd = notes_of(&e, h[0], |n| matches!(n, Note::MqRedeliver { id: 2, .. }));
    t.eq("mq: m2 handed out twice", dl.iter().map(|x| x.1.clone()).collect::<Vec<_>>(), vec![Note::MqDeliver { topic: String::from("jobs"), id: 2, n: 1 }, Note::MqDeliver { topic: String::from("jobs"), id: 2, n: 2 }]);
    t.eq("mq: back exactly after the visibility timeout", rd.first().zip(dl.first()).map(|(r, d)| r.0 - d.0), Some(SEC));
    t.eq("mq: and out again at once (a worker waited)", dl.get(1).zip(rd.first()).map(|(d, r)| d.0 - r.0), Some(0));
    t.eq("mq: jobs done", (metric(&e, h[1], "worker", "done"), metric(&e, h[0], "mq", "redelivered")), (2, 3));
    // m3: the bad worker gives it back every time, after 3 deliveries it is put aside
    t.eq("mq: dead letter", (metric(&e, h[0], "mq", "dead"), metric(&e, h[0], "mq", "nacked"), metric(&e, h[3], "bad", "failed")), (1, 3, 3));
    t.eq("mq: dead noted", notes_of(&e, h[0], |n| matches!(n, Note::MqDead { id: 3, .. })).len(), 1);
    let g = got(&e, h[2]);
    t.eq("mq: publish answers", g.iter().take(3).map(|x| (x.status, x.header("x-message-id"))).collect::<Vec<_>>(), vec![(201, String::from("1")), (201, String::from("2")), (201, String::from("3"))]);
    t.check(&format!("mq: topic text {:?}", g.get(3).map(|x| x.body.clone())), g.get(3).is_some_and(|x| x.status == 200 && x.body.contains("Topic jobs") && x.body.contains("1 bestätigt")));
    t.eq("mq: ack of a message that is not out", g.get(4).map(|x| x.status), Some(404));
    t.eq("mq: empty at the end", (metric(&e, h[0], "mq", "depth"), metric(&e, h[0], "mq", "inflight")), (0, 0));
    // the durable publish waits for the disk: 100 µs and a slot of the iops limit
    let pub1 = notes_of(&e, h[0], |n| matches!(n, Note::MqPublish { id: 1, .. })).first().map(|x| x.0).unwrap_or(0);
    // handshake 2888 ns, the ACK 672 ns ahead of the request, the request 2500 ns, then the
    // disk: 100 µs latency + a slot of the iops limit (20 µs)
    t.eq("mq: published after the disk", pub1, 100 * MS + 2888 + 672 + 2500 + 120 * US);
    t.eq("mq: every delivery", notes_of(&e, h[0], |n| matches!(n, Note::MqDeliver { .. })).iter().map(|x| x.0).collect::<Vec<_>>(), vec![100 * MS + 126_060, SEC + 122_500, 1100 * MS + 122_484, 1110 * MS + 138_524, 1120 * MS + 151_916, 2 * SEC + 122_500]);
    t.check("mq: more workers", e.apply(Action::WorkerSet { dev: h[1], svc: String::from("worker"), concurrency: Some(3), time_ns: None, error_ppm: None }).is_ok() && e.worker_view(h[1], "worker").is_some_and(|v| v.concurrency == 3 && v.slots.len() == 3));
}

// ---------------------------------------------------------------- faults

/// one ping from `from` to `to`: its round trip (0: no answer)
fn ping_rtt(e: &mut Engine, from: u32, to: Ipv4) -> u64 {
    let Ok(id) = e.ping(from, to, PingOpts { count: 1, ..PingOpts::default() }) else {
        return 0;
    };
    let t = e.now() + 3 * SEC;
    run_to(e, t);
    match e.job(id).map(|j| &j.kind) {
        Some(jfn_sim::JobKind::Ping(p)) if p.received == 1 => p.rtt_min,
        _ => 0,
    }
}

fn faults_t(t: &mut T) {
    t.eq("combined loss", [combine_loss(0, 50_000), combine_loss(500_000, 500_000), combine_loss(1_000_000, 10)], [50_000, 750_000, 1_000_000]);
    let (mut e, h) = net(3);
    let lid = cable(&e, h[1]);
    t.check("fault: the cable", lid > 0);
    run_to(&mut e, SEC);
    let base = ping_rtt(&mut e, h[0], addr(1));
    t.check("fault: ping works", base > 0);
    // more delay on SRV2's cable: both ways, 20 ms each
    t.eq("fault: next number", e.next_fault_id(), 1);
    t.check("fault: impair", e.apply(Action::Fault { fault: Fault::Impair { link: lid, delay: 20 * MS, jitter: 0, loss_ppm: 0, kbps: None }, dur: None }).is_ok());
    t.eq("fault: +40 ms round trip", ping_rtt(&mut e, h[0], addr(1)), base + 40 * MS);
    t.eq("fault: the cable's own values stay", e.link_rt(lid).map(|l| l.delay_ns), Some(50));
    // a rate cap of 1 Mbit/s: the 98-byte frames take 976 µs instead of 976 ns on that cable
    t.check("fault: rate", e.apply(Action::Fault { fault: Fault::Impair { link: lid, delay: 0, jitter: 0, loss_ppm: 0, kbps: Some(1000) }, dur: None }).is_ok());
    t.eq("fault: slower", ping_rtt(&mut e, h[0], addr(1)), base + 40 * MS + 2 * (976_000 - 976));
    t.check("fault: heal the delay", e.apply(Action::Heal { id: 1 }).is_ok());
    t.eq("fault: only the rate is left", ping_rtt(&mut e, h[0], addr(1)), base + 2 * (976_000 - 976));
    t.check("fault: heal twice", e.apply(Action::Heal { id: 1 }).is_err());
    t.check("fault: no such fault", e.apply(Action::Heal { id: 99 }).is_err());
    t.check("fault: all loss", e.apply(Action::Fault { fault: Fault::Impair { link: lid, delay: 0, jitter: 0, loss_ppm: 1_000_000, kbps: None }, dur: None }).is_ok());
    t.eq("fault: nothing arrives", ping_rtt(&mut e, h[0], addr(1)), 0);
    t.check("fault: heal all", e.apply(Action::HealAll).is_ok());
    t.eq("fault: as before", ping_rtt(&mut e, h[0], addr(1)), base);
    // the cable pulled
    let now = e.now();
    t.check("fault: unplug", e.apply(Action::Fault { fault: Fault::Unplug { link: lid }, dur: None }).is_ok());
    t.eq("fault: link down at once", notes_of(&e, h[1], |n| matches!(n, Note::LinkDown { .. })).iter().map(|x| x.0).collect::<Vec<_>>(), vec![now]);
    t.eq("fault: no answer", ping_rtt(&mut e, h[0], addr(1)), 0);
    let id = e.next_fault_id() - 1;
    t.eq("fault: the mark", e.faults().last().map(|m| (m.id, m.text.clone(), m.at, m.healed)), Some((id, String::from("Kabel SRV2 eth0 \u{2013} SW1 Gi1/0/2 getrennt"), now, None)));
    let now = e.now();
    t.check("fault: plug in again", e.apply(Action::Heal { id }).is_ok());
    t.eq("fault: link up", notes_of(&e, h[1], |n| matches!(n, Note::LinkUp { .. })).last().map(|x| x.0), Some(now));
    t.check("fault: answers again", ping_rtt(&mut e, h[0], addr(1)) > 0);
    t.check("fault: notes of start and end", notes_of(&e, h[1], |n| matches!(n, Note::FaultStart { .. })).len() >= 3 && notes_of(&e, h[1], |n| matches!(n, Note::FaultEnd { .. })).len() >= 3);
    t.check("fault: unknown cable", e.apply(Action::Fault { fault: Fault::Unplug { link: 9999 }, dur: None }).is_err());
    // power: two faults on one device, it comes back with the last one
    let p1 = e.next_fault_id();
    t.check("fault: power", e.apply(Action::Fault { fault: Fault::Power { dev: h[2] }, dur: None }).is_ok());
    t.check("fault: power again", e.apply(Action::Fault { fault: Fault::Power { dev: h[2] }, dur: None }).is_ok());
    t.eq("fault: off", e.node(h[2]).map(|n| n.power), Some(false));
    t.check("fault: heal one", e.apply(Action::Heal { id: p1 }).is_ok());
    t.eq("fault: still off", e.node(h[2]).map(|n| n.power), Some(false));
    t.check("fault: heal the other", e.apply(Action::Heal { id: p1 + 1 }).is_ok());
    t.eq("fault: on", e.node(h[2]).map(|n| n.power), Some(true));
    // a device the user switched off stays off
    t.check("fault: user off", e.apply(Action::Power { dev: h[2], on: false }).is_ok());
    let p3 = e.next_fault_id();
    t.check("fault: power of an off device", e.apply(Action::Fault { fault: Fault::Power { dev: h[2] }, dur: None }).is_ok() && e.apply(Action::Heal { id: p3 }).is_ok());
    t.eq("fault: stays off", e.node(h[2]).map(|n| n.power), Some(false));
    t.check("fault: user on", e.apply(Action::Power { dev: h[2], on: true }).is_ok());
    // a service stopped and started again
    t.check("fault: a service", add(&mut e, h[1], "http-server", "web", "{}"));
    run_for(&mut e, MS);
    let running = |e: &Engine| e.services(h[1]).iter().find(|s| s.name == "web").map(|s| s.running);
    let s1 = e.next_fault_id();
    t.check("fault: stop", e.apply(Action::Fault { fault: Fault::Stop { dev: h[1], svc: String::from("web") }, dur: None }).is_ok());
    t.eq("fault: stopped", running(&e), Some(false));
    t.check("fault: heal stop", e.apply(Action::Heal { id: s1 }).is_ok());
    run_for(&mut e, MS);
    t.eq("fault: running again", running(&e), Some(true));
    t.check("fault: no such service", e.apply(Action::Fault { fault: Fault::Stop { dev: h[1], svc: String::from("nope") }, dur: None }).is_err());
    // slow disks add up
    let d1 = e.next_fault_id();
    t.check("fault: slow disk", e.apply(Action::Fault { fault: Fault::SlowDisk { dev: h[0], extra: 10 * MS, factor_pm: 2000 }, dur: None }).is_ok());
    t.check("fault: slower disk", e.apply(Action::Fault { fault: Fault::SlowDisk { dev: h[0], extra: 20 * MS, factor_pm: 3000 }, dur: None }).is_ok());
    let disk = |e: &Engine| e.node(h[0]).and_then(|n| n.os.as_ref()).map(|o| (o.disk.slow_extra, o.disk.slow_pm));
    t.eq("fault: both", disk(&e), Some((30 * MS, 6000)));
    t.check("fault: heal the first", e.apply(Action::Heal { id: d1 }).is_ok());
    t.eq("fault: the second one", disk(&e), Some((20 * MS, 3000)));
    t.check("fault: heal it", e.apply(Action::Heal { id: d1 + 1 }).is_ok());
    t.eq("fault: normal again", disk(&e), Some((0, 1000)));
    t.check("fault: factor 0", e.apply(Action::Fault { fault: Fault::SlowDisk { dev: h[0], extra: 0, factor_pm: 0 }, dur: None }).is_err());
    // a fault with a duration heals by itself
    let start = e.now();
    let tid = e.next_fault_id();
    t.check("fault: power for 2 s", e.apply(Action::Fault { fault: Fault::Power { dev: h[2] }, dur: Some(2 * SEC) }).is_ok());
    run_to(&mut e, start + 1999 * MS);
    t.eq("fault: still off before", e.node(h[2]).map(|n| n.power), Some(false));
    run_to(&mut e, start + 2 * SEC);
    t.eq("fault: healed at exactly 2 s", e.faults().iter().find(|m| m.id == tid).map(|m| (m.until, m.healed)), Some((Some(start + 2 * SEC), Some(start + 2 * SEC))));
    t.eq("fault: on again", e.node(h[2]).map(|n| n.power), Some(true));
    t.eq("fault: nothing active", e.active_faults().count(), 0);
    t.eq("fault: marks kept", e.faults().len() as u32, tid);
}

fn partition_t(t: &mut T) {
    let (mut e, h) = net(3);
    run_to(&mut e, SEC);
    t.check("partition: before", ping_rtt(&mut e, h[0], addr(1)) > 0);
    let id = e.next_fault_id();
    t.check("partition: SRV1 | SRV2 SRV3", e.apply(Action::Fault { fault: Fault::Partition { a: vec![h[0]], b: vec![h[1], h[2]] }, dur: None }).is_ok());
    t.eq("partition: across", ping_rtt(&mut e, h[0], addr(1)), 0);
    t.check("partition: inside a side", ping_rtt(&mut e, h[1], addr(2)) > 0);
    t.check("partition: dropped at the receiver", !notes_of(&e, h[1], |n| matches!(n, Note::PartitionDrop { .. })).is_empty());
    t.eq("partition: the mark", e.faults().last().map(|m| m.text.clone()), Some(String::from("Partition: SRV1 | SRV2, SRV3")));
    t.check("partition: heal", e.apply(Action::Heal { id }).is_ok());
    t.check("partition: across again", ping_rtt(&mut e, h[0], addr(1)) > 0);
    // one side only: against every other device
    t.check("partition: SRV1 alone", e.apply(Action::Fault { fault: Fault::Partition { a: vec![h[0]], b: Vec::new() }, dur: None }).is_ok());
    t.eq("partition: SRV1 hears nobody", (ping_rtt(&mut e, h[0], addr(1)), ping_rtt(&mut e, h[2], addr(0))), (0, 0));
    t.check("partition: the others still talk", ping_rtt(&mut e, h[1], addr(2)) > 0);
    t.eq("partition: its text", e.faults().last().map(|m| m.text.clone()), Some(String::from("Partition: SRV1 | alle anderen")));
    t.check("partition: on both sides refused", e.apply(Action::Fault { fault: Fault::Partition { a: vec![h[0]], b: vec![h[0]] }, dur: None }).is_err());
    t.check("partition: empty refused", e.apply(Action::Fault { fault: Fault::Partition { a: Vec::new(), b: Vec::new() }, dur: None }).is_err());
    let _ = e.apply(Action::HealAll);
    // KV nodes on both sides of a partition lose each other at the next check of the
    // heartbeats and find each other again with the first heartbeat after the healing
    let (mut e, h) = net(3);
    let mut ok = true;
    for d in &h[..3] {
        ok &= add(&mut e, *d, "kv", "kv", &format!("{{{}, \"rf\": 2}}", KV3));
    }
    t.check("partition: kv nodes", ok);
    run_to(&mut e, 1200 * MS);
    t.check("partition: kv split", e.apply(Action::Fault { fault: Fault::Partition { a: vec![h[0]], b: vec![h[1], h[2]] }, dur: Some(2 * SEC) }).is_ok());
    run_to(&mut e, 4 * SEC);
    let down1 = notes_of(&e, h[0], |n| matches!(n, Note::KvMember { up: false, .. }));
    let down2 = notes_of(&e, h[1], |n| matches!(n, Note::KvMember { up: false, .. }));
    t.eq("partition: SRV1 loses both at 3 s", down1.iter().map(|x| x.0).collect::<Vec<_>>(), vec![3 * SEC, 3 * SEC]);
    t.eq("partition: SRV2 loses SRV1 at 3 s", down2.iter().map(|x| (x.0, x.1.clone())).collect::<Vec<_>>(), vec![(3 * SEC, Note::KvMember { member: addr(0), up: false })]);
    let up1 = notes_of(&e, h[0], |n| matches!(n, Note::KvMember { up: true, .. }));
    // the heartbeats of 3.5 s: SRV2's after 1444 ns over the switch, SRV3's waits 672 ns behind
    // it on the switch port to SRV1
    t.eq("partition: back with the first heartbeats after 3.2 s", up1.iter().map(|x| (x.0, x.1.clone())).collect::<Vec<_>>(), vec![(3500 * MS + 1444, Note::KvMember { member: addr(1), up: true }), (3500 * MS + 2116, Note::KvMember { member: addr(2), up: true })]);
    t.eq("partition: members up again", metric(&e, h[0], "kv", "members_up"), 3);
}

// ---------------------------------------------------------------- scenarios

fn scenario_t(t: &mut T) {
    let text = "fault link R1 Gi0/1 for 5s\nfault impair R1 Gi0/1 delay 20ms loss 5%\nfault power KV2\nfault power off KV2 for 1.5s\nfault stop WEB2 web\nfault disk KV1 delay 20ms factor 2.5\nfault partition KV1 KV2 | KV3\nfault partition KV3\nheal 2\nheal all\nheal power KV2\nheal partition\nheal stop WEB2 web\n";
    match scenario::parse(text) {
        Ok(sc) => {
            let k: Vec<&StepKind> = sc.steps.iter().map(|s| &s.kind).collect();
            t.eq("scn: steps", k.len(), 13);
            t.eq("scn: link", k[0], &StepKind::Fault { fault: FaultSpec::Link { dev: String::from("R1"), port: String::from("Gi0/1") }, dur: Some(5 * SEC) });
            t.eq("scn: impair", k[1], &StepKind::Fault { fault: FaultSpec::Impair { dev: String::from("R1"), port: String::from("Gi0/1"), imp: jfn_model::scenario::Impair { loss_ppm: Some(50_000), delay_ns: Some(20 * MS), jitter_ns: None, bps: None } }, dur: None });
            t.eq("scn: power", (k[2], k[3]), (&StepKind::Fault { fault: FaultSpec::Power { dev: String::from("KV2") }, dur: None }, &StepKind::Fault { fault: FaultSpec::Power { dev: String::from("KV2") }, dur: Some(1500 * MS) }));
            t.eq("scn: stop", k[4], &StepKind::Fault { fault: FaultSpec::Stop { dev: String::from("WEB2"), svc: String::from("web") }, dur: None });
            t.eq("scn: disk", k[5], &StepKind::Fault { fault: FaultSpec::Disk { dev: String::from("KV1"), delay_ns: 20 * MS, factor_pm: 2500 }, dur: None });
            t.eq("scn: partition", k[6], &StepKind::Fault { fault: FaultSpec::Partition { a: names(&["KV1", "KV2"]), b: names(&["KV3"]) }, dur: None });
            t.eq("scn: partition alone", k[7], &StepKind::Fault { fault: FaultSpec::Partition { a: names(&["KV3"]), b: Vec::new() }, dur: None });
            t.eq("scn: heals", (k[8], k[9]), (&StepKind::Heal(HealSpec::Id(2)), &StepKind::Heal(HealSpec::All)));
            t.eq("scn: heal by target", (k[10], k[11]), (&StepKind::Heal(HealSpec::Like(FaultSpec::Power { dev: String::from("KV2") })), &StepKind::Heal(HealSpec::Like(FaultSpec::Partition { a: Vec::new(), b: Vec::new() }))));
        }
        Err(e) => t.check(&format!("scn parses: {:?}", e), false),
    }
    let bad = "fault disk KV1\nfault partition | KV3\nheal nothing\nfault power KV2 for x\nfault melt KV1\nfault disk KV1 factor 0\n";
    match scenario::parse(bad) {
        Ok(_) => t.check("scn: errors found", false),
        Err(e) => t.eq("scn: error lines", e.iter().map(|x| x.line).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5, 6]),
    }
    // a run: a cable pulled for 2 s, a power fault healed by name
    let (mut p, h) = net_project(2);
    let macs: Vec<Mac> = h.iter().map(|d| mac(*d)).collect();
    for (i, d) in p.devices.iter_mut().filter(|d| h.contains(&d.id)).enumerate() {
        d.startup_config = format!("ip addr add 10.0.0.{}/24 dev eth0\nip neigh add 10.0.0.{} lladdr {} dev eth0\n", i + 1, 2 - i, macs[1 - i]);
    }
    let sc = "at 1s fault link SRV2 eth0 for 2s\nat 1500ms SRV1: ping -c 1 -W 1 10.0.0.2\nat 4s SRV1: ping -c 1 10.0.0.2\nat 5s fault power SRV2\nat 5500ms SRV1: ping -c 1 -W 1 10.0.0.2\nrun 2s\nexpect SRV1 output contains \"0 received\"\nheal power SRV2\nheal power SRV2\nfault stop SRV2 nope\nrun 1s\n";
    match scenario::parse(sc) {
        Ok(s) => {
            let o = jfn_sim::run_scenario(&p, &s, "lab.scn", false);
            let lines: Vec<&str> = o.report.lines().collect();
            t.check("scn run: fault line", lines.iter().any(|l| l.ends_with("--      fault 1: Kabel SRV2 eth0 \u{2013} SW1 Gi1/0/2 getrennt (für 2s)") && l.starts_with("   1.000000000")));
            t.check("scn run: healed by itself at 3 s", lines.iter().any(|l| l.starts_with("   3.000000000") && l.contains("Fehler 1 behoben: Kabel SRV2")));
            t.check("scn run: no answer while pulled", o.report.contains("1 packets transmitted, 0 received"));
            t.check("scn run: answer after", o.report.contains("1 packets transmitted, 1 received"));
            t.check("scn run: power fault and heal by name", lines.iter().any(|l| l.contains("fault 2: SRV2 ausgeschaltet")) && lines.iter().any(|l| l.contains("heal 2: SRV2 ausgeschaltet")));
            t.check("scn run: nothing left to heal", lines.iter().any(|l| l.contains("heal: kein aktiver Fehler passt")));
            t.check("scn run: refused fault is an error", lines.iter().any(|l| l.contains("FEHLER") && l.contains("nope")) && !o.ok);
        }
        Err(e) => t.check(&format!("scn run parses: {:?}", e), false),
    }
}

// ---------------------------------------------------------------- shell

fn shell_t(t: &mut T) {
    let (mut e, h) = net(6);
    let mut ok = add(&mut e, h[0], "lb", "lb", "{\"backends\": [{\"name\": \"web1\", \"addr\": \"10.0.0.2\"}], \"health\": {\"interval_ms\": 1000}}");
    ok &= add(&mut e, h[1], "http-server", "web", "{}");
    ok &= add(&mut e, h[2], "kv", "kv", "{\"nodes\": [\"10.0.0.3\", \"10.0.0.4\"], \"vnodes\": 2, \"rf\": 2}");
    ok &= add(&mut e, h[3], "kv", "kv", "{\"nodes\": [\"10.0.0.3\", \"10.0.0.4\"], \"vnodes\": 2, \"rf\": 2}");
    ok &= add(&mut e, h[4], "cache", "cache", "{\"backend\": \"http://10.0.0.2/\"}");
    ok &= add(&mut e, h[5], "mq", "mq", "{\"topics\": [\"jobs\"]}");
    ok &= add(&mut e, h[5], "worker", "worker", "{\"queue\": \"http://10.0.0.6:9000/\", \"concurrency\": 2}");
    t.check("shell: set up", ok && probe(&mut e, h[1], vec![(100 * MS, req("PUT", "http://10.0.0.3:7000/kv/hallo", "welt"))]));
    run_to(&mut e, 2 * SEC);
    let o = sh(&mut e, h[0], "lbctl");
    t.check(&format!("lbctl: head {:?}", o.lines().next()), o.starts_with("Loadbalancer lb: HTTP auf Port 80, Verfahren round-robin, Health Checks http GET / alle 1,000 s (Timeout 1,000 s, verfügbar nach 2, ausgefallen nach 3 Prüfungen)\n"));
    t.check(&format!("lbctl: table {}", o), o.contains("Backend  Adresse      Gewicht  Zustand    seit  offen  gesamt  Fehler  p99\n") && o.contains("web1     10.0.0.2:80  1        verfügbar"));
    t.check("lbctl: help", sh(&mut e, h[0], "lbctl -h").starts_with("lbctl [status]"));
    t.check("lbctl: unknown word", sh(&mut e, h[0], "lbctl frobnicate").starts_with("lbctl: frobnicate kenne ich nicht"));
    t.eq("lbctl: no lb here", sh(&mut e, h[1], "lbctl"), String::from("lbctl: auf diesem Gerät läuft kein Loadbalancer\n"));
    let o = sh(&mut e, h[2], "kvctl");
    t.check(&format!("kvctl: head {:?}", o.lines().next()), o.starts_with("KV-Knoten kv: 10.0.0.3, Port 7000, Heartbeats UDP 7946 alle 500,000 ms (ausgefallen nach 1,500 s)\n"));
    t.check("kvctl: ring line", o.contains("Ring: 2 Knoten mit je 2 vnodes, RF 2, Konsistenz strong (W 2, R 2, W + R > RF), Read Repair an\n"));
    t.check("kvctl: members", o.contains("10.0.0.3  dieser Knoten") && o.contains("10.0.0.4  erreichbar"));
    t.check("kvctl: numbers", o.contains("Schlüssel 1 (") && o.contains("Anfragen 1: gelesen 0, geschrieben 1"));
    let ring = sh(&mut e, h[2], "kvctl ring");
    t.eq("kvctl ring: 4 tokens", ring.lines().count(), 5);
    t.check("kvctl ring: sorted", {
        let toks: Vec<&str> = ring.lines().skip(1).map(|l| l.split_whitespace().next().unwrap_or("")).collect();
        toks.windows(2).all(|w| w[0] <= w[1])
    });
    let w = sh(&mut e, h[2], "kvctl where hallo");
    t.check(&format!("kvctl where: {}", w), w.starts_with(&format!("Schlüssel hallo: Hash 0x{:016x}, Replikate (RF 2):\n", hash64(b"hallo"))) && w.contains("Leader") && w.contains("Follower") && w.matches("Version 1.").count() == 2);
    t.check("kvctl get", sh(&mut e, h[3], "kvctl get hallo").starts_with("hallo = welt (Version 1."));
    t.eq("kvctl get missing", sh(&mut e, h[3], "kvctl get nix"), String::from("nix: nicht auf diesem Knoten\n"));
    t.check("kvctl keys", sh(&mut e, h[2], "kvctl keys").contains("hallo"));
    let o = sh(&mut e, h[4], "cachectl");
    t.check(&format!("cachectl: {:?}", o.lines().next()), o.starts_with("Cache cache: Port 80 vor http://10.0.0.2/, Lebensdauer 30,000 s, höchstens 1000 Einträge und 64 MiB\n") && o.contains("Treffer 0, Fehlschläge 0 (0,0 % Treffer)"));
    let o = sh(&mut e, h[5], "mqctl");
    t.check(&format!("mqctl: {}", o), o.starts_with("Queue mq: Port 9000, Sichtbarkeit 5,000 s, höchstens 5 Zustellungen, auf der Disk\n") && o.contains("jobs   0        0") && o.contains("Topic  wartend  unterwegs"));
    let o = sh(&mut e, h[5], "workerctl");
    t.check(&format!("workerctl: {}", o), o.starts_with("Worker worker: http://10.0.0.6:9000/ Topic jobs, 2 Plätze, 50,000 ms je Nachricht, Fehlerquote 0 %\n") && o.contains("1      wartet auf eine Nachricht"));
    t.eq("shell: tab", e.cli_complete(h[0], "lbc").0, Some(String::from("lbctl ")));
}

// ---------------------------------------------------------------- determinism

/// a small lab run: what came out of it
fn lab_run() -> (Vec<i64>, usize, usize) {
    let (mut e, h) = net(5);
    let kv_nodes = "\"nodes\": [\"10.0.0.3\", \"10.0.0.4\", \"10.0.0.5\"]";
    let _ = add(&mut e, h[1], "http-server", "web", &format!("{{\"routes\": [{{\"path\": \"/item/*\", \"method\": \"*\", \"backend\": \"kv\"}}], \"kv\": {{{}, \"rf\": 2}}}}", kv_nodes));
    for d in &h[2..5] {
        let _ = add(&mut e, *d, "kv", "kv", &format!("{{{}, \"rf\": 2, \"consistency\": \"eventual\"}}", kv_nodes));
    }
    let _ = add(&mut e, h[0], "http-load", "load", "{\"targets\": [\"http://10.0.0.2/\"], \"mode\": \"open\", \"rps\": 200, \"arrival\": \"poisson\", \"duration_ms\": 3000, \"mix\": [{\"method\": \"PUT\", \"path\": \"/item/k{rand:50}\", \"body\": \"v{seq}\"}, {\"path\": \"/item/k{hot:50}\", \"weight\": 3}]}");
    run_to(&mut e, SEC);
    let _ = e.apply(Action::Fault { fault: Fault::Power { dev: h[3] }, dur: Some(SEC) });
    run_to(&mut e, 5 * SEC);
    let m: Vec<i64> = ["sent", "ok", "http_errors", "failed", "status_5xx", "status_4xx", "p99_us"].iter().map(|n| metric(&e, h[0], "load", n)).collect();
    let keys = e.service::<KvNode>(h[2], "kv").map(|k| k.keys()).unwrap_or(0);
    (m, e.trace.hops().len(), keys)
}

fn determinism(t: &mut T) {
    let a = lab_run();
    let b = lab_run();
    t.eq("determinism: the same run twice", a.clone(), b);
    // keys read before anyone wrote them answer 404 (http_errors); nothing else fails
    t.check(&format!("determinism: the load got through {:?}", a.0), a.0[0] > 500 && a.0[1] + a.0[2] == a.0[0] && a.0[3] == 0 && a.0[4] == 0 && a.0[5] == a.0[2]);
    t.check("determinism: keys from the placeholders", a.2 > 10 && a.2 <= 50);
}
