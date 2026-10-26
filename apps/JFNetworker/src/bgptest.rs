// bgptest -- checks of BGP in jfn_sim (group bgp of --selftest; jfn_sim::bgp, bgptrace, cli/bgp.rs):
// an eBGP session through its states with exact virtual times, the OPEN on the wire, TTL 1,
// collisions, keepalives and the negotiated hold time; iBGP over loopbacks (update-source, TTL
// 255) with a next hop that does not resolve until next-hop-self, and no iBGP route passed to
// another internal neighbour; every step of the decision process; an AS path loop; a wrong
// remote-as with its NOTIFICATION, the pause after it and the problem it shows; failover by the
// hold timer, by a TCP reset after a restart and by fast external fallover; redistribution and
// a network statement without its route; the command line (modes, help, errors, show
// commands, clear with ROUTE-REFRESH and Cease, debug, the running configuration read back); the
// texts in German and English; determinism
//
// Times: 1 Gbit/s copper with 50 ns of cable.  The first SYN of a session waits for ARP and is
// lost (the IOS-like ARP keeps nothing), so TCP sends it again after its first RTO of 1 s; both
// routers do that at once, the two connections collide and the one opened by the higher BGP
// identifier stays.  A KEEPALIVE (19 bytes in a TCP segment of 73) takes (73 + 24) * 8 + 50 ns =
// 826 ns on the wire.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::project::{PortRef, Project};
use jfn_sim::bgp::{CState, Src, State};
use jfn_sim::{Action, BNote, DownWhy, DropWhy, Engine, IdleWhy, Note, RefuseWhy, Report, RouteSrc, Step, TcpEnd, MS, SEC};
use netpkt::bgp::{self as wire, Message};
use netpkt::decode;
use netpkt::ipv4::Ipv4View;
use netpkt::{Ipv4, Prefix4};

use crate::selftest::T;

const BUDGET: usize = 50_000_000;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn ip(s: &str) -> Ipv4 {
    Ipv4::parse(s).unwrap_or(Ipv4::UNSPECIFIED)
}

fn pf(s: &str) -> Prefix4 {
    Prefix4::parse(s).unwrap_or_default()
}

fn s(x: &str) -> String {
    String::from(x)
}

/// a startup configuration: host name, interfaces with an address and more lines each, then
/// `tail`
fn cfg(name: &str, ifs: &[(&str, &str, &[&str])], tail: &str) -> String {
    let mut c = format!("hostname {}\n!\n", name);
    for (i, a, more) in ifs {
        let p = pf(a);
        c.push_str(&format!("interface {}\n ip address {} {}\n", i, p.addr, p.mask()));
        for m in *more {
            c.push_str(&format!(" {}\n", m));
        }
        c.push_str(" no shutdown\n!\n");
    }
    c.push_str(tail);
    c.push_str("end\n");
    c
}

const NONE: &[&str] = &[];
const P2P: &[&str] = &["ip ospf network point-to-point"];

fn set(p: &mut Project, id: u32, text: String) {
    if let Some(d) = p.device_mut(id) {
        d.startup_config = text;
    }
}

/// one more interface with an address in a device's startup configuration (before its routing)
fn add_if(p: &mut Project, id: u32, name: &str, addr: &str) {
    if let Some(d) = p.device_mut(id) {
        let a = pf(addr);
        let at = d.startup_config.find("router ").unwrap_or(d.startup_config.len());
        d.startup_config.insert_str(at, &format!("interface {}\n ip address {} {}\n no shutdown\n!\n", name, a.addr, a.mask()));
    }
}

fn wire_up(p: &mut Project, a: u32, ap: &str, b: u32, bp: &str) -> u32 {
    p.connect(&pr(a, ap), &pr(b, bp), None).unwrap_or(0)
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn run_to(e: &mut Engine, t: u64) {
    e.run_until(t, BUDGET);
}

/// what a device printed since the last call
fn out(e: &mut Engine, dev: u32) -> String {
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

/// lines typed at a device now (the engine does what is due at this moment); its output
fn cli(e: &mut Engine, dev: u32, lines: &[&str]) -> String {
    let _ = e.take_reports();
    for l in lines {
        let _ = e.cli_line(dev, l);
    }
    let now = e.now();
    e.run_until(now, BUDGET);
    out(e, dev)
}

/// the BGP decisions of a device with their times
fn notes(e: &Engine, dev: u32) -> Vec<(u64, BNote)> {
    e.trace
        .events()
        .filter(|x| x.dev == dev)
        .filter_map(|x| match &x.note {
            Note::Bgp(n) => Some((x.t, n.clone())),
            _ => None,
        })
        .collect()
}

/// the times a decision of a device matches
fn times(e: &Engine, dev: u32, f: impl Fn(&BNote) -> bool) -> Vec<u64> {
    notes(e, dev).into_iter().filter(|(_, n)| f(n)).map(|x| x.0).collect()
}

/// the first time a decision of a device matches
fn when(e: &Engine, dev: u32, f: impl Fn(&BNote) -> bool) -> Option<u64> {
    times(e, dev, f).first().copied()
}

/// a session reached a state (every time)
fn reached(e: &Engine, dev: u32, nbr: &str, st: State) -> Vec<u64> {
    times(e, dev, |n| matches!(n, BNote::State { peer, to, .. } if *peer == ip(nbr) && *to == st))
}

/// the state of a session now
fn state(e: &Engine, dev: u32, nbr: &str) -> Option<State> {
    e.bgp(dev)?.peer(ip(nbr)).map(|p| p.state())
}

/// the BGP routes of a device's RIB: (prefix, next hop, distance, metric)
fn b_routes(e: &Engine, dev: u32) -> Vec<(String, Option<Ipv4>, u8, u32)> {
    e.routes(dev).into_iter().filter(|r| r.src == RouteSrc::Bgp).map(|r| (format!("{}", r.prefix), r.via, r.ad, r.metric)).collect()
}

/// the best path of a prefix: from whom (None: originated here), its next hop and the step
fn best(e: &Engine, dev: u32, p: &str) -> Option<(Option<Ipv4>, Ipv4, Step)> {
    let d = e.bgp(dev)?.loc_rib().get(&pf(p))?;
    let c = d.best()?;
    let from = match c.path.from {
        Src::Peer(a) => Some(a),
        Src::Local(_) => None,
    };
    Some((from, c.path.attrs.next_hop, d.step))
}

/// the BGP messages a device sent over a cable: (time, IP TTL, message)
fn msgs(e: &Engine, link: u32, from: u32) -> Vec<(u64, u8, Message)> {
    let mut out = Vec::new();
    for h in e.trace.hops().filter(|h| h.link == link && h.from.0 == from && h.mask & decode::P_BGP != 0) {
        let st = decode::stack(&h.bytes);
        let (Some((o, l)), Some((io, _))) = (st.find(decode::Proto::Bgp), st.find(decode::Proto::Ipv4)) else {
            continue;
        };
        let ttl = Ipv4View::parse(&h.bytes[io..]).map(|v| v.ttl()).unwrap_or(0);
        let mut b = &h.bytes[o..o + l];
        while let Ok(Some(n)) = wire::frame(b) {
            if let Ok(m) = Message::read(&b[..n], true) {
                out.push((h.t_tx, ttl, m));
            }
            b = &b[n..];
        }
    }
    out
}

/// R1 (AS 65001, Lo0 1.1.1.1) - Gi0/0/0 10.0.12.0/30 Gi0/0/0 - R2 (AS 65002, Lo0 2.2.2.2), each
/// with its loopback as a network; `r1` / `r2`: more lines of each router bgp block
fn pair(r1: &str, r2: &str) -> (Project, u32, u32, u32) {
    let mut p = Project::new("bgp pair", 61);
    let a = p.add_device("jf-r4", 0, 0).unwrap_or(0);
    let b = p.add_device("jf-r4", 200, 0).unwrap_or(0);
    let l = wire_up(&mut p, a, "gi0/0/0", b, "gi0/0/0");
    set(&mut p, a, cfg("R1", &[("Loopback0", "1.1.1.1/32", NONE), ("GigabitEthernet0/0/0", "10.0.12.1/30", NONE)], &format!("router bgp 65001\n network 1.1.1.1 mask 255.255.255.255\n neighbor 10.0.12.2 remote-as 65002\n{}!\n", r1)));
    set(&mut p, b, cfg("R2", &[("Loopback0", "2.2.2.2/32", NONE), ("GigabitEthernet0/0/0", "10.0.12.2/30", NONE)], &format!("router bgp 65002\n network 2.2.2.2 mask 255.255.255.255\n neighbor 10.0.12.1 remote-as 65001\n{}!\n", r2)));
    (p, a, b, l)
}

pub fn run(t: &mut T) {
    ebgp_session(t);
    timers(t);
    ibgp(t);
    decision(t);
    as_loop(t);
    wrong_as(t);
    hold_timer(t);
    tcp_reset(t);
    fast_fallover(t);
    redistribution(t);
    command_line(t);
    texts(t);
    determinism(t);
}

// ---------------------------------------------------------------- a session

fn ebgp_session(t: &mut T) {
    let (p, r1, r2, l) = pair("", "");
    let mut e = engine(&p);
    run_to(&mut e, 5 * SEC);
    // both start at 0; the SYNs at 1 s collide, R2 (2.2.2.2) keeps the connection it opened
    t.eq("connect at 0", when(&e, r1, |n| matches!(n, BNote::Connect { .. })), Some(0));
    t.eq("r1 states", times(&e, r1, |n| matches!(n, BNote::State { .. })), alloc::vec![0, 1_000_001_444, 1_000_003_150, 1_000_007_150]);
    t.eq("r2 up", reached(&e, r2, "10.0.12.1", State::Established), alloc::vec![1_000_004_910]);
    t.eq("collision", notes(&e, r1).into_iter().find(|x| matches!(x.1, BNote::Collision { .. })), Some((1_000_004_134, BNote::Collision { peer: ip("10.0.12.2"), keep_ours: false })));
    t.eq("one connection left", e.bgp(r1).and_then(|b| b.peer(ip("10.0.12.2"))).map(|p| (p.conns.len(), p.conns[0].st, p.conns[0].out)), Some((1, CState::Established, false)));
    // the OPEN on the wire: version 4, the AS, hold time, router ID and the three capabilities;
    // every BGP segment towards an external neighbour has TTL 1
    let m = msgs(&e, l, r1);
    match m.iter().find(|x| matches!(x.2, Message::Open(_))) {
        Some((_, _, Message::Open(o))) => {
            t.eq("open", (o.version, o.asn, o.hold, o.id, o.peer_as()), (4, 65001, 180, ip("1.1.1.1"), 65001));
            t.check("capabilities", o.ipv4_unicast() && o.has_cap(wire::CAP_ROUTE_REFRESH) && o.as4() == Some(65001));
        }
        _ => t.check("an OPEN", false),
    }
    t.check("ttl 1", !m.is_empty() && m.iter().all(|x| x.1 == 1));
    // two OPENs (one per connection), the KEEPALIVE that answers the first OPEN, the Cease on the
    // connection that goes, the first UPDATE
    t.eq("kinds", m.iter().map(|x| x.2.kind()).collect::<Vec<u8>>(), alloc::vec![1, 1, 4, 3, 2]);
    // the routes: AD 20, the MED as metric
    t.eq("r1 routes", b_routes(&e, r1), alloc::vec![(s("2.2.2.2/32"), Some(ip("10.0.12.2")), 20, 0)]);
    t.eq("best", best(&e, r1, "2.2.2.2/32"), Some((Some(ip("10.0.12.2")), ip("10.0.12.2"), Step::Only)));
    let p1 = e.bgp(r1).and_then(|b| b.peer(ip("10.0.12.2"))).map(|p| (p.peer_id, p.peer_as, p.hold, p.keepalive, p.as4, p.refresh, p.local));
    t.eq("negotiated", p1, Some((ip("2.2.2.2"), 65002, 180, 60, true, true, ip("10.0.12.1"))));
    // keepalives every 60 s from the last message sent (the UPDATE at Established)
    run_to(&mut e, 200 * SEC);
    let ka: Vec<u64> = msgs(&e, l, r1).into_iter().filter(|x| matches!(x.2, Message::Keepalive)).map(|x| x.0).collect();
    t.eq("keepalives", ka, alloc::vec![1_000_004_084, 61_000_007_150, 121_000_007_150, 181_000_007_150]);
    t.eq("still up", state(&e, r1, "10.0.12.2"), Some(State::Established));
    t.eq("metrics", (e.metric(r1, "bgp", "established"), e.metric(r1, "bgp", "prefixes"), e.metric(r1, "bgp", "flaps")), (Some(1), Some(2), Some(0)));
    t.eq("status", e.services(r1).iter().find(|v| v.kind == "bgp").map(|v| v.status.clone()), Some(s("BGP in AS 65001, Router-ID 1.1.1.1: Nachbarn: 1, davon Established: 1; Netze: 2.")));
}

/// timers bgp: the smaller hold time counts, the keepalive interval is a third of it
fn timers(t: &mut T) {
    let (p, r1, r2, l) = pair("", " timers bgp 10 30\n");
    let mut e = engine(&p);
    run_to(&mut e, 40 * SEC);
    let hk = |e: &Engine, d: u32, n: &str| e.bgp(d).and_then(|b| b.peer(ip(n))).map(|p| (p.hold, p.keepalive));
    t.eq("r1 negotiated", hk(&e, r1, "10.0.12.2"), Some((30, 10)));
    t.eq("r2 negotiated", hk(&e, r2, "10.0.12.1"), Some((30, 10)));
    let ka: Vec<u64> = msgs(&e, l, r1).into_iter().filter(|x| matches!(x.2, Message::Keepalive)).map(|x| x.0).collect();
    t.eq("every 10 s", ka.windows(2).skip(1).map(|w| w[1] - w[0]).collect::<Vec<u64>>(), alloc::vec![10 * SEC, 10 * SEC]);
    // in the OPEN, each its own
    let open_hold = |d: u32| msgs(&e, l, d).into_iter().find_map(|x| if let Message::Open(o) = x.2 { Some(o.hold) } else { None });
    t.eq("open hold times", (open_hold(r1), open_hold(r2)), (Some(180), Some(30)));
}

// ---------------------------------------------------------------- iBGP

/// X (AS 65100) - R1 - R2 - R3 (AS 65000, OSPF inside, iBGP R1-R2 and R2-R3 between loopbacks)
fn ibgp(t: &mut T) {
    let mut p = Project::new("bgp ibgp", 62);
    let x = p.add_device("jf-r4", 0, 0).unwrap_or(0);
    let r: Vec<u32> = (0..3).map(|i| p.add_device("jf-r4", 200 * (i + 1), 0).unwrap_or(0)).collect();
    wire_up(&mut p, x, "gi0/0/0", r[0], "gi0/0/0");
    let l12 = wire_up(&mut p, r[0], "gi0/0/1", r[1], "gi0/0/0");
    wire_up(&mut p, r[1], "gi0/0/1", r[2], "gi0/0/0");
    set(&mut p, x, cfg("X", &[("Loopback0", "9.9.9.9/32", NONE), ("Loopback1", "10.99.0.1/24", NONE), ("GigabitEthernet0/0/0", "10.0.1.2/30", NONE)], "router bgp 65100\n network 10.99.0.0 mask 255.255.255.0\n neighbor 10.0.1.1 remote-as 65000\n!\n"));
    let ospf = "router ospf 1\n network 0.0.0.0 255.255.255.255 area 0\n!\n";
    set(&mut p, r[0], cfg("R1", &[("Loopback0", "1.1.1.1/32", NONE), ("GigabitEthernet0/0/0", "10.0.1.1/30", NONE), ("GigabitEthernet0/0/1", "10.0.12.1/30", P2P)], &format!("router ospf 1\n network 1.1.1.1 0.0.0.0 area 0\n network 10.0.12.0 0.0.0.3 area 0\n!\nrouter bgp 65000\n neighbor 10.0.1.2 remote-as 65100\n neighbor 2.2.2.2 remote-as 65000\n neighbor 2.2.2.2 update-source Loopback0\n!\n")));
    set(&mut p, r[1], cfg("R2", &[("Loopback0", "2.2.2.2/32", NONE), ("GigabitEthernet0/0/0", "10.0.12.2/30", P2P), ("GigabitEthernet0/0/1", "10.0.23.1/30", P2P)], &format!("{}router bgp 65000\n neighbor 1.1.1.1 remote-as 65000\n neighbor 1.1.1.1 update-source Loopback0\n neighbor 3.3.3.3 remote-as 65000\n neighbor 3.3.3.3 update-source Loopback0\n!\n", ospf)));
    set(&mut p, r[2], cfg("R3", &[("Loopback0", "3.3.3.3/32", NONE), ("GigabitEthernet0/0/0", "10.0.23.2/30", P2P)], &format!("{}router bgp 65000\n neighbor 2.2.2.2 remote-as 65000\n neighbor 2.2.2.2 update-source Loopback0\n!\n", ospf)));
    let mut e = engine(&p);
    run_to(&mut e, 120 * SEC);
    t.eq("ibgp up", (state(&e, r[0], "2.2.2.2"), state(&e, r[1], "1.1.1.1"), state(&e, r[1], "3.3.3.3")), (Some(State::Established), Some(State::Established), Some(State::Established)));
    // before OSPF has the loopbacks there is no route: Active, then at once when one comes
    t.eq("no route first", when(&e, r[0], |n| matches!(n, BNote::NoRoute { peer } if *peer == ip("2.2.2.2"))), Some(0));
    let p12 = e.bgp(r[0]).and_then(|b| b.peer(ip("2.2.2.2"))).map(|p| p.local);
    t.eq("from the loopback", p12, Some(ip("1.1.1.1")));
    t.check("ttl 255 inside", msgs(&e, l12, r[0]).iter().all(|x| x.1 == 255));
    // R2 has the path with X's address as next hop, which it cannot reach: not usable
    let d = e.bgp(r[1]).and_then(|b| b.loc_rib().get(&pf("10.99.0.0/24")).cloned());
    t.eq("unresolved", d.map(|d| (d.cands.len(), d.best, d.cands[0].ok, d.cands[0].path.attrs.next_hop)), Some((1, None, false, ip("10.0.1.2"))));
    t.eq("not in the RIB", b_routes(&e, r[1]), Vec::new());
    let tbl = cli(&mut e, r[1], &["enable", "show ip bgp"]);
    t.check("shown without *", tbl.contains("\n   i 10.99.0.0/24     10.0.1.2                 0    100      0 65100 i\n"));
    // R3 gets nothing: R2 does not pass an internal neighbour's path to another one
    t.eq("r3 nothing", e.bgp(r[2]).map(|b| b.loc_rib().len()), Some(0));
    // next-hop-self on R1: the next hop is R1's loopback, which OSPF knows
    let o = cli(&mut e, r[0], &["enable", "conf t", "router bgp 65000", "neighbor 2.2.2.2 next-hop-self", "end"]);
    t.check("taken", !o.contains("% "));
    run_to(&mut e, 125 * SEC);
    t.eq("r2 route", b_routes(&e, r[1]), alloc::vec![(s("10.99.0.0/24"), Some(ip("1.1.1.1")), 200, 0)]);
    t.eq("r2 best", best(&e, r[1], "10.99.0.0/24"), Some((Some(ip("1.1.1.1")), ip("1.1.1.1"), Step::Only)));
    let lp = e.bgp(r[1]).and_then(|b| b.loc_rib().get(&pf("10.99.0.0/24")).and_then(|d| d.best()).map(|c| (c.path.attrs.local_pref, c.path.ebgp, c.igp)));
    t.eq("local preference, internal, IGP metric", lp, Some((Some(100), false, 1)));
    t.eq("r3 still nothing", e.bgp(r[2]).map(|b| b.loc_rib().len()), Some(0));
    // forwarding follows the next hop one step further: R2 sends to R1 over 10.0.12.1
    let tr = cli(&mut e, r[1], &["show ip route bgp"]);
    t.check("show ip route bgp", tr.contains("      10.0.0.0/24 is subnetted, 1 subnets
B        10.99.0.0 [200/0] via 1.1.1.1, 00:00:0") && tr.contains("B - learned by BGP"));
}

// ---------------------------------------------------------------- the decision process

/// R1 (AS 100, its router ID 100.0.0.1) with a neighbour on each of Gi0/0/0 .. on 10.0.k.0/30
/// (R1 .1, the neighbour .2 on its Gi0/0/0, router ID 1.0.0.k, Loopback1 10.200.0.1/24):
/// (AS, lines of the neighbour's router bgp block, lines of R1's for it); `r1`: more of R1's
fn hub(seed: u64, nbrs: &[(u32, &str, &str)], r1: &str, more: &[(usize, &str, usize, &str)]) -> (Project, u32, Vec<u32>) {
    let mut p = Project::new("bgp hub", seed);
    let h = p.add_device("jf-r4", 0, 0).unwrap_or(0);
    let mut ids = Vec::new();
    let mut r1_ifs: Vec<(String, String)> = Vec::new();
    let mut r1_bgp = String::from("router bgp 100\n bgp router-id 100.0.0.1\n");
    for (k, (asn, lines, mine)) in nbrs.iter().enumerate() {
        let n = k + 1;
        let d = p.add_device("jf-r4", 200 * n as i32, 0).unwrap_or(0);
        ids.push(d);
        wire_up(&mut p, h, &format!("gi0/0/{}", k), d, "gi0/0/0");
        r1_ifs.push((format!("GigabitEthernet0/0/{}", k), format!("10.0.{}.1/30", n)));
        r1_bgp.push_str(&format!(" neighbor 10.0.{}.2 remote-as {}\n{}", n, asn, mine));
        let me = format!("10.0.{}.2/30", n);
        let tail = format!("router bgp {}\n bgp router-id 1.0.0.{}\n neighbor 10.0.{}.1 remote-as 100\n{}!\n", asn, n, n, lines);
        set(&mut p, d, cfg(&format!("N{}", n), &[("Loopback1", "10.200.0.1/24", NONE), ("GigabitEthernet0/0/0", &me, NONE)], &tail));
    }
    for (a, ap, b, bp) in more {
        wire_up(&mut p, ids[*a], ap, ids[*b], bp);
    }
    r1_bgp.push_str(r1);
    r1_bgp.push_str("!\n");
    let ifs: Vec<(&str, &str, &[&str])> = r1_ifs.iter().map(|(i, a)| (i.as_str(), a.as_str(), NONE)).collect();
    set(&mut p, h, cfg("R1", &ifs, &r1_bgp));
    (p, h, ids)
}

const NET: &str = " network 10.200.0.0 mask 255.255.255.0\n";

fn decision(t: &mut T) {
    let at = 100 * SEC;
    let prefix = "10.200.0.0/24";
    // weight: two equal paths, the lower router ID first, then the neighbour with weight 100
    let (p, r1, _) = hub(71, &[(1, NET, ""), (2, NET, "")], "", &[]);
    let mut e = engine(&p);
    run_to(&mut e, at);
    t.eq("router id", best(&e, r1, prefix), Some((Some(ip("10.0.1.2")), ip("10.0.1.2"), Step::RouterId)));
    cli(&mut e, r1, &["enable", "conf t", "router bgp 100", "neighbor 10.0.2.2 weight 100", "end"]);
    run_to(&mut e, at + SEC);
    t.eq("weight", best(&e, r1, prefix), Some((Some(ip("10.0.2.2")), ip("10.0.2.2"), Step::Weight)));
    t.eq("weight noted", notes(&e, r1).into_iter().rev().find(|x| matches!(x.1, BNote::Best { .. })).map(|x| x.1), Some(BNote::Best { prefix: pf(prefix), via: ip("10.0.2.2"), from: Some(ip("10.0.2.2")), step: Step::Weight }));
    t.eq("installed", b_routes(&e, r1).into_iter().find(|x| x.0 == prefix), Some((s(prefix), Some(ip("10.0.2.2")), 20, 0)));
    // local preference: two internal neighbours originate it, N1 with a default of 200
    let (p, r1, _) = hub(72, &[(100, " bgp default local-preference 200\n network 10.200.0.0 mask 255.255.255.0\n", ""), (100, NET, "")], "", &[]);
    let mut e = engine(&p);
    run_to(&mut e, at);
    t.eq("local preference", best(&e, r1, prefix), Some((Some(ip("10.0.1.2")), ip("10.0.1.2"), Step::LocalPref)));
    let lps: Vec<Option<u32>> = e.bgp(r1).and_then(|b| b.loc_rib().get(&pf(prefix)).map(|d| d.cands.iter().map(|c| c.path.attrs.local_pref).collect())).unwrap_or_default();
    t.eq("received", lps, alloc::vec![Some(200), Some(100)]);
    // locally originated: R1 has the network too, and N1's weight is that of a local path
    let (mut p, r1, _) = hub(73, &[(1, NET, " neighbor 10.0.1.2 weight 32768\n")], NET, &[]);
    add_if(&mut p, r1, "Loopback1", "10.200.0.1/24");
    let mut e = engine(&p);
    run_to(&mut e, at);
    t.eq("local", best(&e, r1, prefix), Some((None, Ipv4::UNSPECIFIED, Step::Local)));
    // AS path: N2 has it from N1 as well (2 1 against 1)
    let (mut p, r1, ids) = hub(74, &[(1, " network 10.200.0.0 mask 255.255.255.0\n neighbor 10.1.2.2 remote-as 2\n", ""), (2, " neighbor 10.1.2.1 remote-as 1\n", "")], "", &[(0, "gi0/0/1", 1, "gi0/0/1")]);
    for (d, a) in [(ids[0], "10.1.2.1/30"), (ids[1], "10.1.2.2/30")] {
        add_if(&mut p, d, "GigabitEthernet0/0/1", a);
    }
    let mut e = engine(&p);
    run_to(&mut e, at);
    t.eq("as path", best(&e, r1, prefix), Some((Some(ip("10.0.1.2")), ip("10.0.1.2"), Step::AsPath)));
    let paths: Vec<String> = e.bgp(r1).and_then(|b| b.loc_rib().get(&pf(prefix)).map(|d| d.cands.iter().map(|c| wire::path_text(&c.path.attrs.as_path)).collect())).unwrap_or_default();
    t.eq("paths", paths, alloc::vec![s("1"), s("2 1")]);
    // origin: IGP (network) before INCOMPLETE (redistributed)
    let (p, r1, _) = hub(75, &[(1, " redistribute connected\n", ""), (2, NET, "")], "", &[]);
    let mut e = engine(&p);
    run_to(&mut e, at);
    t.eq("origin", best(&e, r1, prefix), Some((Some(ip("10.0.2.2")), ip("10.0.2.2"), Step::Origin)));
    // MED: both in AS 1, the lower MED wins
    let (p, r1, _) = hub(76, &[(1, " redistribute connected metric 50\n", ""), (1, " redistribute connected metric 10\n", "")], "", &[]);
    let mut e = engine(&p);
    run_to(&mut e, at);
    t.eq("med", best(&e, r1, prefix), Some((Some(ip("10.0.2.2")), ip("10.0.2.2"), Step::Med)));
    // MED between different neighbouring ASes is not compared: the router ID decides
    let (p, r1, _) = hub(77, &[(1, " redistribute connected metric 50\n", ""), (2, " redistribute connected metric 10\n", "")], "", &[]);
    let mut e = engine(&p);
    run_to(&mut e, at);
    t.eq("med of other ASes", best(&e, r1, prefix), Some((Some(ip("10.0.1.2")), ip("10.0.1.2"), Step::RouterId)));
    // external before internal: N2 (internal, next-hop-self) has it from N3 in AS 2
    let (mut p, r1, ids) = hub(78, &[(1, NET, ""), (100, " neighbor 10.0.2.1 next-hop-self\n neighbor 10.2.3.2 remote-as 2\n", "")], "", &[]);
    let n3 = p.add_device("jf-r4", 800, 0).unwrap_or(0);
    wire_up(&mut p, ids[1], "gi0/0/1", n3, "gi0/0/0");
    add_if(&mut p, ids[1], "GigabitEthernet0/0/1", "10.2.3.1/30");
    set(&mut p, n3, cfg("N3", &[("Loopback1", "10.200.0.1/24", NONE), ("GigabitEthernet0/0/0", "10.2.3.2/30", NONE)], "router bgp 2\n bgp router-id 1.0.0.3\n network 10.200.0.0 mask 255.255.255.0\n neighbor 10.2.3.1 remote-as 100\n!\n"));
    let mut e = engine(&p);
    run_to(&mut e, at);
    t.eq("external", best(&e, r1, prefix), Some((Some(ip("10.0.1.2")), ip("10.0.1.2"), Step::External)));
    t.eq("candidates", e.bgp(r1).and_then(|b| b.loc_rib().get(&pf(prefix)).map(|d| d.cands.iter().filter(|c| c.ok).count())), Some(2));
    // the IGP metric to the next hop: A over Ethernet (cost 1), B over a serial line (48)
    igp_metric(t);
    // the neighbour address: two sessions to one router over two cables
    let (mut p, r1, ids) = hub(79, &[(1, " network 10.200.0.0 mask 255.255.255.0\n neighbor 10.0.3.1 remote-as 100\n", " neighbor 10.0.3.2 remote-as 1\n")], "", &[]);
    wire_up(&mut p, r1, "gi0/0/1", ids[0], "gi0/0/1");
    for (d, a) in [(r1, "10.0.3.1/30"), (ids[0], "10.0.3.2/30")] {
        add_if(&mut p, d, "GigabitEthernet0/0/1", a);
    }
    let mut e = engine(&p);
    run_to(&mut e, at);
    t.eq("address", best(&e, r1, prefix), Some((Some(ip("10.0.1.2")), ip("10.0.1.2"), Step::Address)));
}

/// R1 iBGP to A and B over loopbacks with OSPF: A over Ethernet (cost 1), B over a serial line
/// (cost 48); both originate the network, B has the lower router ID
fn igp_metric(t: &mut T) {
    let mut p = Project::new("bgp igp", 80);
    let r1 = p.add_device("jf-r4", 0, 0).unwrap_or(0);
    let a = p.add_device("jf-r4", 200, 0).unwrap_or(0);
    let b = p.add_device("jf-r4", 200, 200).unwrap_or(0);
    wire_up(&mut p, r1, "gi0/0/0", a, "gi0/0/0");
    wire_up(&mut p, r1, "s0/2/0", b, "s0/2/0");
    let ib = |rid: &str, peers: &[&str], extra: &str| -> String {
        let mut x = format!("router ospf 1\n router-id {}\n network 0.0.0.0 255.255.255.255 area 0\n!\nrouter bgp 100\n bgp router-id {}\n{}", rid, rid, extra);
        for q in peers {
            x.push_str(&format!(" neighbor {} remote-as 100\n neighbor {} update-source Loopback0\n", q, q));
        }
        x.push_str("!\n");
        x
    };
    set(&mut p, r1, cfg("R1", &[("Loopback0", "1.1.1.1/32", NONE), ("GigabitEthernet0/0/0", "10.0.1.1/30", P2P), ("Serial0/2/0", "10.0.2.1/30", NONE)], &ib("1.1.1.1", &["2.2.2.2", "3.3.3.3"], "")));
    set(&mut p, a, cfg("A", &[("Loopback0", "2.2.2.2/32", NONE), ("Loopback1", "10.200.0.1/24", NONE), ("GigabitEthernet0/0/0", "10.0.1.2/30", P2P)], &ib("9.0.0.9", &["1.1.1.1"], NET)));
    set(&mut p, b, cfg("B", &[("Loopback0", "3.3.3.3/32", NONE), ("Loopback1", "10.200.0.1/24", NONE), ("Serial0/2/0", "10.0.2.2/30", NONE)], &ib("8.0.0.8", &["1.1.1.1"], NET)));
    let mut e = engine(&p);
    run_to(&mut e, 120 * SEC);
    t.eq("igp metric", best(&e, r1, "10.200.0.0/24"), Some((Some(ip("2.2.2.2")), ip("2.2.2.2"), Step::IgpMetric)));
    let igps: Vec<(Ipv4, u32)> = e.bgp(r1).and_then(|x| x.loc_rib().get(&pf("10.200.0.0/24")).map(|d| d.cands.iter().map(|c| (c.path.attrs.next_hop, c.igp)).collect())).unwrap_or_default();
    t.eq("metrics", igps, alloc::vec![(ip("2.2.2.2"), 1), (ip("3.3.3.3"), 48)]);
}

// ---------------------------------------------------------------- loops and errors

/// R1 (AS 1) - R2 (AS 2) - R3 (AS 1): what R3 hears of R1 has its own AS in the path
fn as_loop(t: &mut T) {
    let mut p = Project::new("bgp loop", 63);
    let r: Vec<u32> = (0..3).map(|i| p.add_device("jf-r4", 200 * i, 0).unwrap_or(0)).collect();
    wire_up(&mut p, r[0], "gi0/0/0", r[1], "gi0/0/0");
    wire_up(&mut p, r[1], "gi0/0/1", r[2], "gi0/0/0");
    set(&mut p, r[0], cfg("R1", &[("Loopback1", "10.1.0.1/24", NONE), ("GigabitEthernet0/0/0", "10.0.12.1/30", NONE)], "router bgp 1\n network 10.1.0.0 mask 255.255.255.0\n neighbor 10.0.12.2 remote-as 2\n!\n"));
    set(&mut p, r[1], cfg("R2", &[("GigabitEthernet0/0/0", "10.0.12.2/30", NONE), ("GigabitEthernet0/0/1", "10.0.23.1/30", NONE)], "router bgp 2\n neighbor 10.0.12.1 remote-as 1\n neighbor 10.0.23.2 remote-as 1\n!\n"));
    set(&mut p, r[2], cfg("R3", &[("Loopback1", "10.3.0.1/24", NONE), ("GigabitEthernet0/0/0", "10.0.23.2/30", NONE)], "router bgp 1\n network 10.3.0.0 mask 255.255.255.0\n neighbor 10.0.23.1 remote-as 2\n!\n"));
    let mut e = engine(&p);
    run_to(&mut e, 100 * SEC);
    t.check("r2 has both", best(&e, r[1], "10.1.0.0/24").is_some() && best(&e, r[1], "10.3.0.0/24").is_some());
    let dropped = |e: &Engine, d: u32| notes(e, d).into_iter().filter_map(|(_, n)| if let BNote::Dropped { prefix, why, .. } = n { Some((prefix, why)) } else { None }).collect::<Vec<_>>();
    t.eq("r3 drops", dropped(&e, r[2]), alloc::vec![(pf("10.1.0.0/24"), DropWhy::Loop(1))]);
    t.eq("r1 drops", dropped(&e, r[0]), alloc::vec![(pf("10.3.0.0/24"), DropWhy::Loop(1))]);
    t.eq("r3 table", e.bgp(r[2]).map(|b| b.loc_rib().keys().map(|k| format!("{}", k)).collect::<Vec<_>>()), Some(alloc::vec![s("10.3.0.0/24")]));
    t.check("dropped is a drop", Note::Bgp(BNote::Dropped { peer: ip("10.0.23.2"), prefix: pf("10.1.0.0/24"), why: DropWhy::Loop(1) }).is_drop());
}

/// a wrong remote-as: NOTIFICATION Bad Peer AS, a pause of 60 s, the problem; then fixed
fn wrong_as(t: &mut T) {
    let (mut p, r1, r2, l) = pair("", "");
    if let Some(d) = p.device_mut(r1) {
        d.startup_config = d.startup_config.replace("neighbor 10.0.12.2 remote-as 65002", "neighbor 10.0.12.2 remote-as 65099");
    }
    let mut e = engine(&p);
    run_to(&mut e, 10 * SEC);
    let n: Vec<(u64, Message)> = msgs(&e, l, r1).into_iter().filter(|x| matches!(x.2, Message::Notification(_))).map(|x| (x.0, x.2)).collect();
    t.eq("notifications", n.iter().map(|x| x.1.clone()).collect::<Vec<_>>(), alloc::vec![Message::Notification(wire::Notification { code: 2, sub: 2, data: alloc::vec![0xfd, 0xea] }); 2]);
    t.eq("r2 heard it", e.bgp(r2).and_then(|b| b.peer(ip("10.0.12.1"))).and_then(|p| p.last_error).map(|x| (x.1, x.2, x.3)), Some((false, 2, 2)));
    t.eq("r1 idle", e.bgp(r1).and_then(|b| b.peer(ip("10.0.12.2"))).map(|p| (p.state(), p.idle)), Some((State::Idle, Some(IdleWhy::Hold))));
    t.eq("never up", reached(&e, r1, "10.0.12.2", State::Established), Vec::new());
    let probs: Vec<String> = e.problems().into_iter().filter(|x| x.kind == jfn_sim::ProblemKind::BgpNeighbor && x.dev == r1).map(|x| x.text).collect();
    t.eq("problem", probs, alloc::vec![s("R1: der Nachbar 10.0.12.2 (R2) ist in AS 65002, nicht in AS 65099 (neighbor 10.0.12.2 remote-as).")]);
    // after the pause they try again
    run_to(&mut e, 70 * SEC);
    let again = times(&e, r1, |n| matches!(n, BNote::Notification { out: true, code: 2, sub: 2, .. }));
    t.check("again after 60 s", again.len() >= 3 && again.iter().any(|&x| x > 60 * SEC && x < 63 * SEC));
    // fixed: the neighbour starts at once
    let fix = e.now();
    let o = cli(&mut e, r1, &["enable", "conf t", "router bgp 65001", "neighbor 10.0.12.2 remote-as 65002", "end"]);
    t.check("fix taken", !o.contains("% "));
    run_to(&mut e, fix + 2 * SEC);
    let up = reached(&e, r1, "10.0.12.2", State::Established);
    t.check("up after the fix", up.len() == 1 && up[0] > fix && up[0] < fix + 10 * MS);
    t.eq("no problem now", e.problems().iter().filter(|x| x.kind == jfn_sim::ProblemKind::BgpNeighbor).count(), 0);
}

// ---------------------------------------------------------------- failover

/// R1 - switch - R2: a pulled cable at R2 leaves R1's interface up, the hold timer ends it
fn switched(seed: u64, timers: &str) -> (Project, u32, u32, u32, u32) {
    let (mut p, r1, r2, l) = pair(timers, timers);
    p.seed = seed;
    let _ = p.disconnect(l);
    let sw = p.add_device("jf-s24p", 100, 100).unwrap_or(0);
    let a = wire_up(&mut p, r1, "gi0/0/0", sw, "gi1/0/1");
    let b = wire_up(&mut p, r2, "gi0/0/0", sw, "gi1/0/2");
    (p, r1, r2, a, b)
}

fn hold_timer(t: &mut T) {
    let (p, r1, r2, _, b) = switched(64, " timers bgp 10 30\n");
    let mut e = engine(&p);
    run_to(&mut e, 20 * SEC);
    t.eq("up", state(&e, r1, "10.0.12.2"), Some(State::Established));
    let _ = e.apply(Action::Unplug { link: b, down: true });
    run_to(&mut e, 60 * SEC);
    // R2 sees its interface go and ends the session at once (fast external fallover)
    t.eq("r2 at once", notes(&e, r2).into_iter().find(|x| matches!(x.1, BNote::Down { .. })), Some((20 * SEC, BNote::Down { peer: ip("10.0.12.1"), why: DownWhy::IfDown })));
    // R1 waits for 30 s after the last keepalive it heard
    let last = times(&e, r1, |n| matches!(n, BNote::Keepalive { out: false, .. })).into_iter().filter(|&x| x < 20 * SEC).max().unwrap_or(0);
    t.eq("hold expired", when(&e, r1, |n| matches!(n, BNote::HoldExpired { .. })), Some(last + 30 * SEC));
    t.eq("down", notes(&e, r1).into_iter().find(|x| matches!(x.1, BNote::Down { .. })).map(|x| x.1), Some(BNote::Down { peer: ip("10.0.12.2"), why: DownWhy::Sent(4, 0) }));
    t.eq("withdrawn", b_routes(&e, r1), Vec::new());
    t.check("lost noted", times(&e, r1, |n| matches!(n, BNote::Lost { prefix } if *prefix == pf("2.2.2.2/32"))) == alloc::vec![last + 30 * SEC]);
    t.check("trying again", matches!(state(&e, r1, "10.0.12.2"), Some(State::Connect | State::Active)));
}

/// R2 restarts behind the switch between two keepalives: R1's session is up still and refuses
/// R2's new connection; its next keepalive (at 61 s) meets a TCP that knows nothing and gets a RST
fn tcp_reset(t: &mut T) {
    let (p, r1, r2, _, _) = switched(65, "");
    let mut e = engine(&p);
    run_to(&mut e, 20 * SEC);
    let _ = e.apply(Action::Power { dev: r2, on: false });
    run_to(&mut e, 21 * SEC);
    let _ = e.apply(Action::Power { dev: r2, on: true });
    run_to(&mut e, 70 * SEC);
    let refused = when(&e, r1, |n| matches!(n, BNote::Refused { why: RefuseWhy::Established, .. }));
    t.check("refused while up", refused.is_some_and(|x| x > 21 * SEC && x < 23 * SEC));
    let down = notes(&e, r1).into_iter().find(|x| matches!(x.1, BNote::Down { .. }));
    t.check("reset", down.as_ref().is_some_and(|x| x.1 == BNote::Down { peer: ip("10.0.12.2"), why: DownWhy::Tcp(TcpEnd::Reset) } && x.0 > 61 * SEC && x.0 < 62 * SEC));
    let up = reached(&e, r1, "10.0.12.2", State::Established);
    t.check("up again", up.len() == 2 && down.is_some_and(|d| up[1] > d.0 && up[1] < d.0 + 2 * SEC));
    t.eq("route back", b_routes(&e, r1), alloc::vec![(s("2.2.2.2/32"), Some(ip("10.0.12.2")), 20, 0)]);
}

/// a direct cable pulled: the session ends at once, and comes back with the cable
fn fast_fallover(t: &mut T) {
    let (p, r1, _r2, l) = pair("", "");
    let mut e = engine(&p);
    run_to(&mut e, 20 * SEC);
    let _ = e.apply(Action::Unplug { link: l, down: true });
    run_to(&mut e, 30 * SEC);
    t.eq("down at once", notes(&e, r1).into_iter().find(|x| matches!(x.1, BNote::Down { .. })), Some((20 * SEC, BNote::Down { peer: ip("10.0.12.2"), why: DownWhy::IfDown })));
    t.eq("idle", e.bgp(r1).and_then(|b| b.peer(ip("10.0.12.2"))).map(|p| (p.state(), p.idle)), Some((State::Idle, Some(IdleWhy::NotConnected))));
    t.eq("gone", b_routes(&e, r1), Vec::new());
    let _ = e.apply(Action::Unplug { link: l, down: false });
    run_to(&mut e, 35 * SEC);
    let up = reached(&e, r1, "10.0.12.2", State::Established);
    t.check("back", up.len() == 2 && up[1] > 30 * SEC && up[1] < 32 * SEC);
    t.eq("route back", b_routes(&e, r1).len(), 1);
}

// ---------------------------------------------------------------- local paths

fn redistribution(t: &mut T) {
    let (mut p, r1, r2, _) = pair(" redistribute connected\n redistribute static metric 77\n network 10.77.0.0 mask 255.255.0.0\n", "");
    if let Some(d) = p.device_mut(r1) {
        d.startup_config = d.startup_config.replace("router bgp", "interface Loopback1\n ip address 192.168.1.1 255.255.255.0\n!\nip route 172.16.0.0 255.255.0.0 192.168.1.254\n!\nrouter bgp");
    }
    let mut e = engine(&p);
    run_to(&mut e, 10 * SEC);
    let tbl = |e: &Engine, d: u32| -> Vec<(String, String, Option<u32>)> {
        e.bgp(d).map(|b| b.loc_rib().iter().filter_map(|(k, x)| x.best().map(|c| (format!("{}", k), format!("{} {}", wire::path_text(&c.path.attrs.as_path), wire::origin_code(c.path.attrs.origin)), c.path.attrs.med))).collect()).unwrap_or_default()
    };
    let got = tbl(&e, r2);
    t.eq("r2 table", got, alloc::vec![
        (s("1.1.1.1/32"), s("65001 i"), Some(0)),
        (s("2.2.2.2/32"), s(" i"), Some(0)),
        (s("10.0.12.0/30"), s("65001 ?"), Some(0)),
        (s("172.16.0.0/16"), s("65001 ?"), Some(77)),
        (s("192.168.1.0/24"), s("65001 ?"), Some(0)),
    ]);
    // the connected network R2 has itself stays out of its forwarding (r)
    let o = cli(&mut e, r2, &["enable", "show ip bgp"]);
    t.eq("rib failure", o.lines().find(|l| l.contains("10.0.12.0/30")), Some(" r>  10.0.12.0/30     10.0.12.1                0             0 65001 ?"));
    // the network statement without its route: a problem, nothing originated
    let probs: Vec<String> = e.problems().into_iter().filter(|x| x.kind == jfn_sim::ProblemKind::BgpNetwork).map(|x| x.text).collect();
    t.eq("network problem", probs, alloc::vec![s("R1: zu network 10.77.0.0/16 fehlt die Route im Routing-Table: BGP kündigt das Netz nicht an.")]);
    // the route comes: R1 originates it, R2 gets it after the minimum interval
    let now = e.now();
    cli(&mut e, r1, &["enable", "conf t", "interface Loopback2", "ip address 10.77.0.1 255.255.0.0", "end"]);
    t.check("local noted", when(&e, r1, |n| matches!(n, BNote::Local { prefix, .. } if *prefix == pf("10.77.0.0/16"))) == Some(now));
    let wait = notes(&e, r1).into_iter().find_map(|(_, n)| if let BNote::Mrai { wait, .. } = n { Some(wait) } else { None });
    t.check("waits", wait.is_some_and(|w| w > 20 * SEC && w <= 30 * SEC));
    run_to(&mut e, now + 31 * SEC);
    t.check("r2 has it", best(&e, r2, "10.77.0.0/16").is_some());
    t.eq("no problem now", e.problems().iter().filter(|x| x.kind == jfn_sim::ProblemKind::BgpNetwork).count(), 0);
}

// ---------------------------------------------------------------- the command line

fn command_line(t: &mut T) {
    let (p, r1, r2, l) = pair(" neighbor 10.0.12.2 description zu R2\n", "");
    let mut e = engine(&p);
    run_to(&mut e, 5 * SEC);
    cli(&mut e, r1, &["enable", "conf t", "router bgp 65001"]);
    t.eq("mode", e.cli_prompt(r1), s("R1(config-router)#"));
    t.check("help", e.cli_help(r1, "").starts_with("Commands for this BGP process:\n"));
    let h = e.cli_help(r1, "neighbor 10.0.12.2 ");
    t.eq("neighbor help", h, s("  description    A note about the neighbour\n  ebgp-multihop  Allow an external neighbour that is not directly connected\n  next-hop-self  Send own address as next hop\n  remote-as      The neighbour's AS: the own one for iBGP, another for eBGP\n  shutdown       Keep the session down\n  update-source  Use the address of this interface for the session\n  weight         Preference of the neighbour's paths on this router\n\n"));
    t.eq("tab", e.cli_complete(r1, "nei").0, Some(s("neighbor ")));
    t.eq("remote-as first", cli(&mut e, r1, &["neighbor 9.9.9.9 next-hop-self"]), s("% Specify remote-as or peer-group commands first\n"));
    t.eq("one process", cli(&mut e, r1, &["exit", "router bgp 65009"]), s("% BGP is already running; AS is 65001\n"));
    let run = cli(&mut e, r1, &["end", "show running-config"]);
    t.check("running config", run.contains("router bgp 65001\n network 1.1.1.1 mask 255.255.255.255\n neighbor 10.0.12.2 remote-as 65002\n neighbor 10.0.12.2 description zu R2\n!\n"));
    // the summary, the table and a neighbour
    let sum = cli(&mut e, r1, &["show ip bgp summary"]);
    t.check("summary", sum.starts_with("Router ID 1.1.1.1, local AS 65001\nTable version 3; networks: 2, paths: 2\n\nNeighbor        V           AS MsgRcvd MsgSent   TblVer  InQ OutQ Up/Down  State/PfxRcd\n10.0.12.2       4        65002       4       5        3    0    0 00:00:03        1\n"));
    let tb = cli(&mut e, r1, &["show ip bgp"]);
    t.check("table", tb.contains("     Network          Next Hop            Metric LocPrf Weight Path\n *>  1.1.1.1/32       0.0.0.0                  0         32768 i\n *>  2.2.2.2/32       10.0.12.2                0             0 65002 i\n"));
    let nb = cli(&mut e, r1, &["show ip bgp neighbors 10.0.12.2"]);
    t.check("neighbour", nb.starts_with("BGP neighbour 10.0.12.2, AS 65002, external (eBGP), \"zu R2\"\n  State Established for 00:00:03, BGP identifier 2.2.2.2\n"));
    let rr = cli(&mut e, r1, &["show ip bgp neighbors 10.0.12.2 routes"]);
    t.check("routes", rr.contains(" *>  2.2.2.2/32       10.0.12.2") && !rr.contains("1.1.1.1/32") && rr.ends_with("\nNetworks listed: 1\n"));
    let ar = cli(&mut e, r1, &["show ip bgp neighbors 10.0.12.2 advertised-routes"]);
    t.check("advertised", ar.contains(" *>  1.1.1.1/32       0.0.0.0") && !ar.contains("2.2.2.2/32"));
    let one = cli(&mut e, r1, &["show ip bgp 2.2.2.2"]);
    t.check("one network", one.starts_with("Network 2.2.2.2/32 in the BGP table, version 3\n  Paths: 1, the best is #1 (the only path)\n"));
    t.check("protocols", cli(&mut e, r1, &["show ip protocols"]).contains("Routing process \"bgp 65001\" with router ID 1.1.1.1 (RFC 4271)\n"));
    // debug ip bgp updates; soft in asks for the routes again (ROUTE-REFRESH), R2 sends them
    let d = cli(&mut e, r1, &["debug ip bgp updates", "show debugging"]);
    t.eq("debug", d, s("Debug output for BGP updates: on\nDebug output for BGP updates: on\n"));
    let at = e.now();
    let mut o = cli(&mut e, r1, &["clear ip bgp 10.0.12.2 soft in"]);
    run_to(&mut e, at + 10 * MS);
    o.push_str(&out(&mut e, r1));
    t.check("refresh answered", o.contains("BGP: ROUTE-REFRESH to 10.0.12.2\n") && o.contains("BGP: UPDATE from 10.0.12.2: 1 networks, 0 withdrawn\n"));
    t.check("on the wire", msgs(&e, l, r1).iter().any(|x| x.0 >= at && matches!(x.2, Message::RouteRefresh { afi: 1, safi: 1 })));
    t.eq("same session", reached(&e, r1, "10.0.12.2", State::Established).len(), 1);
    // a hard clear: Cease / Administrative Reset, a new session
    let at = e.now();
    cli(&mut e, r1, &["undebug all", "clear ip bgp *"]);
    run_to(&mut e, at + 3 * SEC);
    t.check("cease", msgs(&e, l, r1).iter().any(|x| x.0 >= at && x.2 == Message::Notification(wire::Notification::new(6, 4))));
    t.eq("new session", reached(&e, r1, "10.0.12.2", State::Established).len(), 2);
    // shutdown: Cease / Administrative Shutdown, Idle (Admin)
    let at = e.now();
    cli(&mut e, r1, &["conf t", "router bgp 65001", "neighbor 10.0.12.2 shutdown", "end"]);
    run_to(&mut e, at + SEC);
    t.check("shutdown cease", times(&e, r2, |n| matches!(n, BNote::Notification { out: false, code: 6, sub: 2, .. })).len() == 1);
    t.check("admin", cli(&mut e, r1, &["show ip bgp summary"]).ends_with(" Idle (Admin)\n"));
    t.check("shutdown line", cli(&mut e, r1, &["show running-config"]).contains(" neighbor 10.0.12.2 shutdown\n"));
    cli(&mut e, r1, &["conf t", "router bgp 65001", "no neighbor 10.0.12.2 shutdown", "end"]);
    let now = e.now();
    run_to(&mut e, now + 2 * SEC);
    t.eq("back", state(&e, r1, "10.0.12.2"), Some(State::Established));
    // the running configuration boots back to the same
    let saved = e.running_config(r1);
    let mut q = p.clone();
    if let Some(d) = q.device_mut(r1) {
        d.startup_config = saved.clone();
    }
    let mut e2 = engine(&q);
    run_to(&mut e2, SEC);
    t.eq("read back", e2.running_config(r1), saved);
    // no router bgp: the process goes, the neighbour hears Cease
    let at = e.now();
    cli(&mut e, r1, &["conf t", "no router bgp 65001", "end"]);
    run_to(&mut e, at + SEC);
    t.check("gone", e.bgp(r1).is_some_and(|b| !b.cfg.on) && b_routes(&e, r1).is_empty() && !e.running_config(r1).contains("router bgp"));
    t.check("deconfigured", times(&e, r2, |n| matches!(n, BNote::Notification { out: false, code: 6, sub: 3, .. })).len() == 1);
    // an L3 switch needs ip routing
    let mut q = Project::new("bgp l3", 66);
    let sw = q.add_device("jf-s24l3", 0, 0).unwrap_or(0);
    let mut e = engine(&q);
    let o = cli(&mut e, sw, &["enable", "conf t", "router bgp 65001"]);
    t.eq("ip routing", o, s("Configuration mode: one command per line; end or Ctrl+Z leaves it.\n% IP routing not enabled\n"));
}

// ---------------------------------------------------------------- texts

fn texts(t: &mut T) {
    let rows: Vec<(&str, &str)> = include_str!("locales/engine.tsv").lines().skip(1).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect();
    let en = move |k: &'static str| rows.iter().find(|r| r.0 == k).map_or(k, |r| r.1);
    let name = |i: u16| format!("Gi0/0/{}", i);
    let port = |p: u16| format!("p{}", p);
    let cases: Vec<(BNote, &str, &str)> = alloc::vec![
        (BNote::State { peer: ip("10.0.12.2"), from: State::OpenConfirm, to: State::Established }, "BGP: Nachbar 10.0.12.2: OpenConfirm \u{2192} Established.", "BGP: neighbour 10.0.12.2: OpenConfirm \u{2192} Established."),
        (BNote::Best { prefix: pf("10.2.0.0/24"), via: ip("10.0.12.2"), from: Some(ip("10.0.12.2")), step: Step::AsPath }, "BGP: bester Weg zu 10.2.0.0/24 über 10.0.12.2 (von 10.0.12.2): kürzerer AS-Pfad.", "BGP: best path to 10.2.0.0/24 via 10.0.12.2 (from 10.0.12.2): shorter AS path."),
        (BNote::Down { peer: ip("10.0.12.2"), why: DownWhy::Sent(4, 0) }, "BGP: die Sitzung mit 10.0.12.2 ist beendet (Hold Timer abgelaufen); seine Wege sind zurückgezogen.", "BGP: the session with 10.0.12.2 has ended (hold timer expired); its paths are withdrawn."),
        (BNote::Notification { peer: ip("10.0.12.1"), out: true, code: 2, sub: 2 }, "BGP: NOTIFICATION an 10.0.12.1: 2/2 (Bad Peer AS).", "BGP: NOTIFICATION to 10.0.12.1: 2/2 (Bad Peer AS)."),
        (BNote::Dropped { peer: ip("10.0.23.2"), prefix: pf("10.1.0.0/24"), why: DropWhy::Loop(1) }, "BGP: 10.1.0.0/24 von 10.0.23.2 verworfen: das eigene AS 1 steht im AS-Pfad (Schleife).", "BGP: 10.1.0.0/24 from 10.0.23.2 dropped: the own AS 1 is in the AS path (a loop)."),
        (BNote::Idle { peer: ip("10.9.9.9"), why: IdleWhy::NotConnected }, "BGP: Nachbar 10.9.9.9 bleibt Idle: nicht direkt verbunden, ebgp-multihop fehlt.", "BGP: neighbour 10.9.9.9 stays idle: not directly connected, ebgp-multihop missing."),
    ];
    for (n, de, english) in cases {
        let m = Note::Bgp(n.clone()).message(&port, &name);
        t.eq(&format!("german {:?}", n), m.text(), s(de));
        t.eq(&format!("english {:?}", n), m.text_with(&en), s(english));
    }
    let n = Note::Bgp(BNote::Keepalive { peer: ip("10.0.12.2"), out: true });
    t.eq("layer and mask", (n.layer(), n.mask() & decode::P_BGP, n.is_drop()), (7, decode::P_BGP, false));
}

// ---------------------------------------------------------------- determinism

fn determinism(t: &mut T) {
    let run = || {
        let (p, _, _, _) = pair(" redistribute connected\n", "");
        let mut e = engine(&p);
        run_to(&mut e, 70 * SEC);
        let hops: Vec<(u64, u32, Vec<u8>)> = e.trace.hops().filter(|h| h.mask & decode::P_BGP != 0).map(|h| (h.t_tx, h.link, h.bytes.to_vec())).collect();
        let evs: Vec<(u64, u32, Note)> = e.trace.events().filter(|x| matches!(x.note, Note::Bgp(_))).map(|x| (x.t, x.dev, x.note.clone())).collect();
        (hops, evs)
    };
    let (a, b) = (run(), run());
    t.check("frames", !a.0.is_empty() && a.0 == b.0);
    t.check("decisions", !a.1.is_empty() && a.1 == b.1);
}
