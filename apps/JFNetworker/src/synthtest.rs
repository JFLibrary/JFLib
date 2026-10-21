// synthtest -- checks of the synthetic projects (jfn_model::synth, --gen-topology) and of the
// load jfn_sim's ping_many makes (part of --selftest): sizes from the smallest to 500 devices,
// no finding of Project::check, every line of every startup configuration taken, nothing in
// Engine::problems (no duplicate address, no gateway off the net, no loop without spanning
// tree), the same project again for the same size; with 500 devices 800 cables, 300 pings
// whose first frames are on the cables at the same moment and, once ARP has learned the way,
// every one of them answered across the core.  The routing table a lookup keeps (ip.rs) follows
// the interfaces and the static routes: a shut uplink hands over to the floating default
// route, with both shut there is no route, a new static route is used at once.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::synth::{topology, MIN_DEVICES};
use jfn_sim::{Action, Engine, Filter, IcmpKind, JobKind, PingEv, PingOpts};
use netpkt::{Ipv4, Prefix4};

use crate::selftest::T;

const BUDGET: usize = 10_000_000;

/// the pings of jobs after `first` that got their answer
fn answered(e: &Engine, first: u32) -> usize {
    e.jobs().filter(|j| j.id > first).filter(|j| matches!(&j.kind, JobKind::Ping(p) if p.received == 1)).count()
}

/// the newest job's ping failed for want of a route
fn no_route(e: &Engine) -> bool {
    e.jobs().last().is_some_and(|j| matches!(&j.kind, JobKind::Ping(p) if p.events.iter().any(|v| matches!(v, PingEv::Error { kind: IcmpKind::NoRoute, .. }))))
}

pub(crate) fn run(t: &mut T) {
    for n in [MIN_DEVICES, 30, 100, 500] {
        let p = topology(n);
        t.eq(&format!("{} devices", n), p.devices.len(), n);
        t.check(&format!("{}: more cables than devices", n), p.links.len() > n);
        t.eq(&format!("{}: consistent", n), p.check().len(), 0);
        t.check(&format!("{}: the same again", n), topology(n).to_text() == p.to_text());
        let Ok(e) = Engine::from_project(&p) else {
            t.check(&format!("{}: runs", n), false);
            continue;
        };
        let bad: usize = p.devices.iter().map(|d| e.boot_errors(d.id).len()).sum();
        t.eq(&format!("{}: every configuration line taken", n), bad, 0);
        let probs: Vec<String> = e.problems().into_iter().map(|x| x.text).collect();
        t.eq(&format!("{}: no problems", n), probs, Vec::<String>::new());
    }
    // 500 devices: 800 cables, 300 frames at once, answers once ARP knows the way
    let p = topology(500);
    t.eq("800 cables", p.links.len(), 800);
    let Ok(mut e) = Engine::from_project(&p) else {
        t.check("500 run", false);
        return;
    };
    t.eq("300 pings", e.ping_many(300), 300);
    t.eq("300 frames on the cables", e.frames_at(e.now(), &Filter::all(), 512).len(), 300);
    e.run_idle(BUDGET);
    t.check("most answered while ARP learns", answered(&e, 0) >= 200);
    let first = e.jobs().map(|j| j.id).max().unwrap_or(0);
    t.eq("again: 300 pings", e.ping_many(300), 300);
    e.run_idle(BUDGET);
    t.eq("again: every one answered", answered(&e, first), 300);
    t.eq("more pings than hosts: one per host", e.ping_many(5000), 436);
    e.run_idle(BUDGET);
    // R1's default route leaves over its first uplink; shut, the floating one over the second
    // takes over; with both shut there is no route until a static one over the serial line
    let (Some(r1), Some(pc1)) = (p.find("R1").map(|d| d.id), p.find("PC1").map(|d| d.id)) else {
        t.check("R1 and PC1", false);
        return;
    };
    let far = Ipv4::parse("10.19.1.10").unwrap_or(Ipv4::UNSPECIFIED);
    let ping = |e: &mut Engine, dev: u32| {
        let _ = e.ping(dev, far, PingOpts { count: 1, ..PingOpts::default() });
        e.run_idle(BUDGET);
    };
    let first = e.jobs().map(|j| j.id).max().unwrap_or(0);
    ping(&mut e, pc1);
    ping(&mut e, pc1);
    t.check("answered over the first uplink", answered(&e, first) >= 1);
    // (the answers cannot come back: the core routers' static routes still point at the shut
    // uplink; what leaves over the second one shows the routing table moved on)
    let sent = |e: &Engine| {
        let d = e.view().devices.into_iter().find(|d| d.dev == r1);
        d.and_then(|d| d.ports.into_iter().find(|p| p.name == "TenGigabitEthernet0/1/1")).map_or(0, |p| p.counters.tx_frames)
    };
    let _ = e.apply(Action::IfUp { dev: r1, iface: String::from("TenGigabitEthernet0/1/0"), up: false });
    let before = sent(&e);
    ping(&mut e, pc1);
    ping(&mut e, pc1);
    t.check("over the second uplink", sent(&e) > before);
    let _ = e.apply(Action::IfUp { dev: r1, iface: String::from("TenGigabitEthernet0/1/1"), up: false });
    ping(&mut e, r1);
    t.check("no route with both uplinks shut", no_route(&e));
    let net = Prefix4 { addr: Ipv4::parse("10.19.0.0").unwrap_or(Ipv4::UNSPECIFIED), len: 16 };
    let _ = e.apply(Action::Route { dev: r1, prefix: net, via: None, iface: Some(String::from("Serial0/2/0")), ad: 1, add: true });
    ping(&mut e, r1);
    t.check("a new static route counts at once", !no_route(&e));
}
