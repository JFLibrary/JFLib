// ospftest -- checks of OSPF in jfn_sim (group ospf of --selftest; jfn_sim::ospf, cli/ospf.rs):
// Hellos on the wire and the dead interval, the DR / BDR election with priorities (no
// preemption, the BDR takes over), an adjacency through every state with exact virtual times,
// flooding with acknowledgements, MinLSArrival and retransmission, SPF costs and equal-cost paths,
// a cable pulled and the new path, passive interfaces, redistribution and the default route
// against static routes, two areas through an ABR with a stub area, aging (refresh after 1800
// s, MaxAge after 3600 s), mismatches that keep neighbours apart, the command line (modes, help,
// show commands, debug, clear, the running configuration read back), the texts in German and
// English, determinism
//
// Times: 1 Gbit/s copper with 50 ns of cable, so a frame of n bytes takes (max(n, 60) + 24) * 8
// ns on the wire plus 50: a Hello without neighbours (78 bytes) 866 ns, with one 898 ns, a DD
// without LSA headers 770 ns and with one 930 ns, a Link State Request for one LSA 802 ns, an
// update with a router-LSA of two links (110 bytes) 1122 ns and of four links 1314 ns, an
// acknowledgement of one LSA 866 ns; frames out of one port follow each other.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::project::{PortRef, Project};
use jfn_sim::{Action, Engine, IfState, JobKind, LsuWhy, NbrEvent, NbrState, Note, OrigWhy, OspfDrop, Report, RouteSrc, RtChange, RtKind, TraceOpts, MS, SEC};
use netpkt::decode;
use netpkt::ospf::{self as wire, Body, OspfView, Packet};
use netpkt::{Ipv4, Prefix4};

use crate::selftest::T;

const BUDGET: usize = 20_000_000;

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

/// the usual process: area 0 for 10.0.0.0/8 and the loopback, the router ID set
fn ospf(rid: &str) -> String {
    format!("router ospf 1\n router-id {}\n network 10.0.0.0 0.255.255.255 area 0\n network {} 0.0.0.0 area 0\n!\n", rid, rid)
}

const P2P: &[&str] = &["ip ospf network point-to-point"];
const NONE: &[&str] = &[];

fn set(p: &mut Project, id: u32, text: String) {
    if let Some(d) = p.device_mut(id) {
        d.startup_config = text;
    }
}

fn wire(p: &mut Project, a: u32, ap: &str, b: u32, bp: &str) -> u32 {
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

/// a console refused a line ("% ..." at the start of a line; log lines start with their stamp)
fn refused(o: &str) -> bool {
    o.lines().any(|l| l.starts_with('%'))
}

/// the OSPF decisions of a device with their times
fn notes(e: &Engine, dev: u32) -> Vec<(u64, Note)> {
    e.trace.events().filter(|x| x.dev == dev && x.note.is_ospf()).map(|x| (x.t, x.note.clone())).collect()
}

/// the first time a decision of a device matches
fn when(e: &Engine, dev: u32, f: impl Fn(&Note) -> bool) -> Option<u64> {
    e.trace.events().find(|x| x.dev == dev && f(&x.note)).map(|x| x.t)
}

/// the times a decision of a device matches
fn times(e: &Engine, dev: u32, f: impl Fn(&Note) -> bool) -> Vec<u64> {
    e.trace.events().filter(|x| x.dev == dev && f(&x.note)).map(|x| x.t).collect()
}

/// a neighbour reached a state (the first time)
fn reached(e: &Engine, dev: u32, nbr: &str, st: NbrState) -> Option<u64> {
    when(e, dev, |n| matches!(n, Note::OspfNbr { nbr: x, to, .. } if *x == ip(nbr) && *to == st))
}

/// the OSPF routes of a device's RIB: (prefix, next hop, metric, distance)
fn ospf_routes(e: &Engine, dev: u32) -> Vec<(String, Option<Ipv4>, u32, u8)> {
    e.routes(dev).into_iter().filter(|r| r.src == RouteSrc::Ospf).map(|r| (format!("{}", r.prefix), r.via, r.metric, r.ad)).collect()
}

/// the routes of a prefix that win in the RIB (best distance): (source code, next hop)
fn best(e: &Engine, dev: u32, p: &str) -> Vec<(&'static str, Option<Ipv4>)> {
    let rs: Vec<_> = e.routes(dev).into_iter().filter(|r| r.prefix == pf(p)).collect();
    let ad = rs.iter().map(|r| r.ad).min();
    rs.into_iter().filter(|r| Some(r.ad) == ad).map(|r| (r.src.code(), r.via)).collect()
}

/// the state of a neighbour as the device sees it now
fn state(e: &Engine, dev: u32, nbr: &str) -> Option<NbrState> {
    e.ospf(dev)?.ifaces().values().flat_map(|f| f.nbrs.values()).find(|n| n.id == ip(nbr)).map(|n| n.state)
}

/// the OSPF packets that crossed a cable from a device: (time sent, the packet)
fn frames(e: &Engine, link: u32, from: u32) -> Vec<(u64, Packet, Vec<u8>)> {
    e.trace
        .hops()
        .filter(|h| h.link == link && h.from.0 == from && h.mask & decode::P_OSPF != 0)
        .filter_map(|h| {
            let st = decode::stack(&h.bytes);
            let (o, l) = st.find(decode::Proto::Ospf)?;
            let v = OspfView::parse(&h.bytes[o..o + l])?;
            Packet::read(&v).ok().map(|p| (h.t_tx, p, h.bytes.to_vec()))
        })
        .collect()
}

/// R1 (Lo0 1.1.1.1) - Gi0/0/0 10.0.12.0/24 Gi0/0/0 - R2 (Lo0 2.2.2.2); `p2p`: the link is a
/// point-to-point network (else broadcast); the link
fn pair(p2p: bool) -> (Project, u32, u32, u32) {
    let mut p = Project::new("ospf pair", 41);
    let r1 = p.add_device("jf-r4", 0, 0).unwrap_or(0);
    let r2 = p.add_device("jf-r4", 200, 0).unwrap_or(0);
    let l = wire(&mut p, r1, "gi0/0/0", r2, "gi0/0/0");
    let x = if p2p { P2P } else { NONE };
    set(&mut p, r1, cfg("R1", &[("Loopback0", "1.1.1.1/32", NONE), ("GigabitEthernet0/0/0", "10.0.12.1/24", x)], &ospf("1.1.1.1")));
    set(&mut p, r2, cfg("R2", &[("Loopback0", "2.2.2.2/32", NONE), ("GigabitEthernet0/0/0", "10.0.12.2/24", x)], &ospf("2.2.2.2")));
    (p, r1, r2, l)
}

/// R1 - R2 - R3 over point-to-point Ethernet (10.0.12.0/24, 10.0.23.0/24), loopbacks n.n.n.n
fn chain() -> (Project, [u32; 3], [u32; 2]) {
    let mut p = Project::new("ospf chain", 42);
    let r: Vec<u32> = (0..3).map(|i| p.add_device("jf-r4", 200 * i, 0).unwrap_or(0)).collect();
    let l1 = wire(&mut p, r[0], "gi0/0/0", r[1], "gi0/0/0");
    let l2 = wire(&mut p, r[1], "gi0/0/1", r[2], "gi0/0/0");
    set(&mut p, r[0], cfg("R1", &[("Loopback0", "1.1.1.1/32", NONE), ("GigabitEthernet0/0/0", "10.0.12.1/24", P2P)], &ospf("1.1.1.1")));
    set(&mut p, r[1], cfg("R2", &[("Loopback0", "2.2.2.2/32", NONE), ("GigabitEthernet0/0/0", "10.0.12.2/24", P2P), ("GigabitEthernet0/0/1", "10.0.23.2/24", P2P)], &ospf("2.2.2.2")));
    set(&mut p, r[2], cfg("R3", &[("Loopback0", "3.3.3.3/32", NONE), ("GigabitEthernet0/0/0", "10.0.23.3/24", P2P)], &ospf("3.3.3.3")));
    (p, [r[0], r[1], r[2]], [l1, l2])
}

/// the triangle of the golden scenario: R1 - R2 a LAN cable (broadcast), R2 - R3 serial (2048
/// kbit/s: cost 48), R1 - R3 point-to-point Ethernet; a LAN 10.0.n.0/24 on Gi0/0/2 of each
/// (passive) with PCn at 10.0.n.10; the routers, the links R1-R2, R2-R3, R1-R3, the PCs
fn triangle() -> (Project, [u32; 3], [u32; 3], [u32; 3]) {
    let mut p = Project::new("ospf triangle", 43);
    let r: Vec<u32> = (0..3).map(|i| p.add_device("jf-r4", 200 * i, 0).unwrap_or(0)).collect();
    let l12 = wire(&mut p, r[0], "gi0/0/0", r[1], "gi0/0/0");
    let l23 = wire(&mut p, r[1], "se0/2/0", r[2], "se0/2/0");
    let l13 = wire(&mut p, r[0], "gi0/0/1", r[2], "gi0/0/1");
    let mut pcs = [0u32; 3];
    for (k, pc) in pcs.iter_mut().enumerate() {
        *pc = p.add_device("pc", 200 * k as i32, 300).unwrap_or(0);
        wire(&mut p, *pc, "eth0", r[k], "gi0/0/2");
        set(&mut p, *pc, format!("hostname PC{}\nip addr add 10.0.{}.10/24 dev eth0\nip route add default via 10.0.{}.1\n", k + 1, k + 1, k + 1));
    }
    let lan = |rid: &str| format!("router ospf 1\n router-id {}\n passive-interface GigabitEthernet0/0/2\n network 10.0.0.0 0.255.255.255 area 0\n network {} 0.0.0.0 area 0\n!\n", rid, rid);
    set(&mut p, r[0], cfg("R1", &[("Loopback0", "1.1.1.1/32", NONE), ("GigabitEthernet0/0/0", "10.0.12.1/24", NONE), ("GigabitEthernet0/0/1", "10.0.13.1/24", P2P), ("GigabitEthernet0/0/2", "10.0.1.1/24", NONE)], &lan("1.1.1.1")));
    set(&mut p, r[1], cfg("R2", &[("Loopback0", "2.2.2.2/32", NONE), ("GigabitEthernet0/0/0", "10.0.12.2/24", NONE), ("Serial0/2/0", "10.0.23.1/30", NONE), ("GigabitEthernet0/0/2", "10.0.2.1/24", NONE)], &lan("2.2.2.2")));
    set(&mut p, r[2], cfg("R3", &[("Loopback0", "3.3.3.3/32", NONE), ("GigabitEthernet0/0/1", "10.0.13.3/24", P2P), ("Serial0/2/0", "10.0.23.2/30", NONE), ("GigabitEthernet0/0/2", "10.0.3.1/24", NONE)], &lan("3.3.3.3")));
    (p, [r[0], r[1], r[2]], [l12, l23, l13], pcs)
}

/// four routers on a switch (10.0.0.0/24): priorities R1 50, R2 100, R3 0, R4 1; the switch
fn lan4() -> (Project, [u32; 4], u32) {
    let mut p = Project::new("ospf lan", 44);
    let sw = p.add_device("jf-s24p", 300, 200).unwrap_or(0);
    let r: Vec<u32> = (0..4).map(|i| p.add_device("jf-r4", 200 * i, 0).unwrap_or(0)).collect();
    for (k, &d) in r.iter().enumerate() {
        wire(&mut p, d, "gi0/0/0", sw, &format!("gi1/0/{}", k + 1));
    }
    for (k, prio) in [Some(50), Some(100), Some(0), None].iter().enumerate() {
        let n = k + 1;
        let more: Vec<String> = prio.map(|x| format!("ip ospf priority {}", x)).into_iter().collect();
        let refs: Vec<&str> = more.iter().map(|x| x.as_str()).collect();
        let rid = format!("{}.{}.{}.{}", n, n, n, n);
        set(&mut p, r[k], cfg(&format!("R{}", n), &[("Loopback0", &format!("{}/32", rid), NONE), ("GigabitEthernet0/0/0", &format!("10.0.0.{}/24", n), &refs)], &ospf(&rid)));
    }
    (p, [r[0], r[1], r[2], r[3]], sw)
}

pub fn run(t: &mut T) {
    hellos(t);
    election(t);
    adjacency(t);
    broadcast_pair(t);
    flooding(t);
    spf(t);
    failure(t);
    passive(t);
    redistribution(t);
    areas(t);
    aging(t);
    mismatches(t);
    command_line(t);
    texts(t);
    determinism(t);
}

fn hellos(t: &mut T) {
    let (p, r1, r2, l) = pair(true);
    let mut e = engine(&p);
    t.eq("boot errors", (e.boot_errors(r1).len(), e.boot_errors(r2).len()), (0, 0));
    run_to(&mut e, 35 * SEC);
    // a Hello every 10 s from the moment the interface came up
    let hs: Vec<(u64, Packet, Vec<u8>)> = frames(&e, l, r1).into_iter().filter(|f| matches!(f.1.body, Body::Hello(_))).collect();
    t.eq("hello times", hs.iter().map(|h| h.0).collect::<Vec<_>>(), alloc::vec![0, 10 * SEC, 20 * SEC, 30 * SEC]);
    match hs.first() {
        Some((_, pkt, b)) => {
            // to AllSPFRouters with TTL 1 and IP precedence 6 (TOS 0xc0), IP protocol 89
            t.eq("to the group", (&b[0..6], b[15], b[22], b[23], &b[30..34]), (&[0x01u8, 0x00, 0x5E, 0x00, 0x00, 0x05][..], 0xC0, 1, 89, &[224u8, 0, 0, 5][..]));
            t.eq("frame length", b.len(), 78);
            let Body::Hello(h) = &pkt.body else {
                return t.check("a hello", false);
            };
            t.eq("hello fields", (pkt.hdr.router, pkt.hdr.area, h.mask, h.interval, h.options, h.priority, h.dead, h.dr, h.neighbors.len()), (ip("1.1.1.1"), 0, ip("255.255.255.0"), 10, wire::OPT_E, 1, 40, Ipv4::UNSPECIFIED, 0));
            t.check("checksum", OspfView::parse(&b[34..]).is_some_and(|v| v.checksum_ok()));
        }
        None => t.check("first hello", false),
    }
    // the second one names the neighbour it heard
    t.check("neighbour named", hs.get(1).is_some_and(|h| matches!(&h.1.body, Body::Hello(x) if x.neighbors == alloc::vec![ip("2.2.2.2")])));
    let h0 = e.trace.hops().find(|h| h.link == l && h.from.0 == r1).cloned();
    t.eq("summary", h0.map(|h| jfn_sim::view::hop_info(&h).1), Some(s("Hello Packet, 0 neighbors")));
    // the neighbour: Init with its first Hello (78 bytes: 866 ns), ExStart when it names this
    // router (82 bytes: 898 ns) on a point-to-point network
    t.eq("init", reached(&e, r1, "2.2.2.2", NbrState::Init), Some(866));
    t.eq("2-way and exstart at once", (reached(&e, r1, "2.2.2.2", NbrState::TwoWay), reached(&e, r1, "2.2.2.2", NbrState::ExStart)), (None, Some(10 * SEC + 898)));
    // the groups: AllSPFRouters where OSPF speaks, not on the loopback
    let st = e.node(r1).and_then(|n| n.ip.as_ref());
    let gi = st.and_then(|s| s.iface_by_name("gi0/0/0")).unwrap_or(99);
    let lo = st.and_then(|s| s.iface_by_name("lo0")).unwrap_or(99);
    t.check("groups", st.is_some_and(|s| s.joined(gi, wire::ALL_SPF_ROUTERS) && !s.joined(gi, wire::ALL_D_ROUTERS) && !s.joined(lo, wire::ALL_SPF_ROUTERS)));
    t.check("service listed", e.services(r1).iter().any(|x| x.kind == "ospf" && x.name == "ospf" && x.running));
    // the dead interval: the cable loses everything from 100 s on; R2's last Hello came at 90 s
    // and 898 ns, so R1 gives up on it 40 s after that
    run_to(&mut e, 100 * SEC - 1);
    let _ = e.apply(Action::Impair { link: l, kbps: None, delay_ns: None, jitter_ns: None, loss_ppm: Some(1_000_000) });
    run_to(&mut e, 131 * SEC);
    t.eq("inactivity", when(&e, r1, |n| matches!(n, Note::OspfNbr { ev: NbrEvent::InactivityTimer, from: NbrState::Full, to: NbrState::Down, .. })), Some(130 * SEC + 898));
    t.eq("routes gone", ospf_routes(&e, r1), Vec::new());
    t.eq("metrics", (e.metric(r1, "ospf", "neighbours"), e.metric(r1, "ospf", "full")), (Some(0), Some(0)));
}

fn election(t: &mut T) {
    let (p, r, _) = lan4();
    let mut e = engine(&p);
    run_to(&mut e, 10 * SEC);
    // R3 (priority 0) is DR Other at once; the others wait RouterDeadInterval
    let st = |e: &Engine, d: u32| e.ospf(d).and_then(|o| o.ifaces().values().find(|f| f.name == "GigabitEthernet0/0/0").map(|f| f.state));
    t.eq("waiting", (st(&e, r[0]), st(&e, r[1]), st(&e, r[2]), st(&e, r[3])), (Some(IfState::Waiting), Some(IfState::Waiting), Some(IfState::DrOther), Some(IfState::Waiting)));
    run_to(&mut e, 60 * SEC);
    // R2 (100) is DR, R1 (50) BDR; R3 and R4 talk to them only
    t.eq("roles", (st(&e, r[0]), st(&e, r[1]), st(&e, r[2]), st(&e, r[3])), (Some(IfState::Backup), Some(IfState::Dr), Some(IfState::DrOther), Some(IfState::DrOther)));
    t.eq("first election", when(&e, r[1], |n| matches!(n, Note::OspfDr { .. })), Some(40 * SEC));
    t.eq("r4's neighbours", (state(&e, r[3], "1.1.1.1"), state(&e, r[3], "2.2.2.2"), state(&e, r[3], "3.3.3.3")), (Some(NbrState::Full), Some(NbrState::Full), Some(NbrState::TwoWay)));
    let nb = cli(&mut e, r[3], &["enable", "show ip ospf neighbor"]);
    t.eq(
        "show ip ospf neighbor",
        nb,
        s("\nNeighbor ID     Pri   State           Dead Time   Address         Interface\n1.1.1.1          50   FULL/BDR        00:00:30    10.0.0.1        GigabitEthernet0/0/0\n2.2.2.2         100   FULL/DR         00:00:30    10.0.0.2        GigabitEthernet0/0/0\n3.3.3.3           0   2WAY/DROTHER    00:00:30    10.0.0.3        GigabitEthernet0/0/0\n"),
    );
    // the DR's network-LSA names every router fully adjacent to it
    let net = e.ospf(r[0]).and_then(|o| o.database(0)).and_then(|d| d.values().find(|x| x.lsa.hdr.kind == wire::LSA_NETWORK).map(|x| x.lsa.clone()));
    match net.map(|x| (x.hdr.id, x.hdr.adv, x.body)) {
        Some((id, adv, wire::LsaBody::Network(n))) => t.eq("network-lsa", (id, adv, n.routers), (ip("10.0.0.2"), ip("2.2.2.2"), alloc::vec![ip("2.2.2.2"), ip("1.1.1.1"), ip("3.3.3.3"), ip("4.4.4.4")])),
        _ => t.check("network-lsa", false),
    }
    // a higher priority does not take a role away (9.4: a DR and a BDR that declare themselves
    // stay)
    let _ = cli(&mut e, r[3], &["conf t", "interface gi0/0/0", "ip ospf priority 200", "end"]);
    run_to(&mut e, 75 * SEC);
    t.eq("no preemption", (st(&e, r[0]), st(&e, r[1]), st(&e, r[3])), (Some(IfState::Backup), Some(IfState::Dr), Some(IfState::DrOther)));
    // the DR goes: the BDR takes over when its dead interval is over, R4 (200) becomes BDR
    let _ = e.apply(Action::Power { dev: r[1], on: false });
    run_to(&mut e, 130 * SEC);
    t.eq("after the dr", (st(&e, r[0]), st(&e, r[3])), (Some(IfState::Dr), Some(IfState::Backup)));
    let lost = when(&e, r[0], |n| matches!(n, Note::OspfNbr { nbr, to: NbrState::Down, ev: NbrEvent::InactivityTimer, .. } if *nbr == ip("2.2.2.2")));
    let took = when(&e, r[0], |n| matches!(n, Note::OspfIfState { to: IfState::Dr, .. }));
    t.check("the bdr takes over when the dr is dead", lost.is_some() && lost == took && lost.is_some_and(|x| x > 110 * SEC && x < 111 * SEC));
    t.eq("r3 to the new dr", state(&e, r[2], "1.1.1.1"), Some(NbrState::Full));
}

fn adjacency(t: &mut T) {
    let (p, r1, r2, l) = pair(true);
    let mut e = engine(&p);
    run_to(&mut e, 11 * SEC);
    // R2 has the higher router ID: it is the master; R1 answers as slave
    t.check("r1 is slave", notes(&e, r1).iter().any(|x| x.1 == Note::OspfMaster { nbr: ip("2.2.2.2"), iface: 0, master: false }));
    t.check("r2 is master", notes(&e, r2).iter().any(|x| x.1 == Note::OspfMaster { nbr: ip("1.1.1.1"), iface: 0, master: true }));
    // the states on R1 with their times: both start ExStart with a DD of I, M and MS (770 ns),
    // R1 takes R2's and answers with its header (930 ns), R2 sends its own header, R1 answers
    // without one and asks for R2's router-LSA right behind it (720 + 802 ns); the update with
    // that LSA of two links (110 bytes) takes 1122 ns
    let at = |st: NbrState| reached(&e, r1, "2.2.2.2", st);
    t.eq("r1 states", (at(NbrState::Init), at(NbrState::ExStart), at(NbrState::Exchange), at(NbrState::Loading), at(NbrState::Full)), (Some(866), Some(10 * SEC + 898), Some(10 * SEC + 1668), Some(10 * SEC + 3528), Some(10 * SEC + 6172)));
    let dd: Vec<(u64, u8, usize)> = frames(&e, l, r1).into_iter().filter_map(|f| if let Body::Dd(d) = &f.1.body { Some((f.0, d.flags, d.lsas.len())) } else { None }).collect();
    t.eq("r1's dds", dd, alloc::vec![(10 * SEC + 898, wire::DD_I | wire::DD_M | wire::DD_MS, 0), (10 * SEC + 1668, 0, 1), (10 * SEC + 3528, 0, 0)]);
    let dd2: Vec<(u64, u8, usize)> = frames(&e, l, r2).into_iter().filter_map(|f| if let Body::Dd(d) = &f.1.body { Some((f.0, d.flags, d.lsas.len())) } else { None }).collect();
    t.eq("r2's dds", dd2, alloc::vec![(10 * SEC + 898, wire::DD_I | wire::DD_M | wire::DD_MS, 0), (10 * SEC + 2598, wire::DD_MS, 1)]);
    // the slave takes the master's sequence number, the master counts on
    let seqs: Vec<u32> = notes(&e, r1).into_iter().filter_map(|x| if let Note::OspfDd { out: true, seq, .. } = x.1 { Some(seq) } else { None }).collect();
    let m: Vec<u32> = notes(&e, r2).into_iter().filter_map(|x| if let Note::OspfDd { out: true, seq, .. } = x.1 { Some(seq) } else { None }).collect();
    t.check("sequence numbers", seqs.len() == 3 && m.len() == 2 && seqs[1] == m[0] && seqs[2] == m[1] && m[1] == m[0].wrapping_add(1));
    // a Link State Request for one LSA each, answered in an update
    t.check("requests", frames(&e, l, r1).iter().any(|f| matches!(&f.1.body, Body::Lsr(k) if *k == alloc::vec![wire::LsaKey { kind: 1, id: ip("2.2.2.2"), adv: ip("2.2.2.2") }])));
    t.check("answers", notes(&e, r2).iter().any(|x| matches!(x.1, Note::OspfLsu { out: true, lsas: 1, why: LsuWhy::Answer, .. })));
    // Full: both send a new router-LSA with the point-to-point link (the links in the order of
    // the interfaces: the Ethernet port comes before the loopback)
    run_to(&mut e, 20 * SEC);
    t.eq("r1 routes", ospf_routes(&e, r1), alloc::vec![(s("2.2.2.2/32"), Some(ip("10.0.12.2")), 1, 110)]);
    t.eq("r2 routes", ospf_routes(&e, r2), alloc::vec![(s("1.1.1.1/32"), Some(ip("10.0.12.1")), 1, 110)]);
    let rl = e.ospf(r1).and_then(|o| o.database(0)).and_then(|d| d.get(&wire::LsaKey { kind: 1, id: ip("1.1.1.1"), adv: ip("1.1.1.1") })).map(|x| x.lsa.clone());
    match rl.map(|x| (x.hdr.seq, x.body)) {
        Some((seq, wire::LsaBody::Router(r))) => {
            let links: Vec<(u8, Ipv4, Ipv4, u16)> = r.links.iter().map(|x| (x.kind, x.id, x.data, x.metric)).collect();
            t.eq("r1's router-lsa", (seq, links), (0x8000_0002, alloc::vec![(wire::LINK_P2P, ip("2.2.2.2"), ip("10.0.12.1"), 1), (wire::LINK_STUB, ip("10.0.12.0"), ip("255.255.255.0"), 1), (wire::LINK_STUB, ip("1.1.1.1"), ip("255.255.255.255"), 0)]));
        }
        _ => t.check("r1's router-lsa", false),
    }
    t.eq("metrics", (e.metric(r1, "ospf", "full"), e.metric(r1, "ospf", "lsas"), e.metric(r1, "ospf", "routes")), (Some(1), Some(2), Some(3)));
}

fn broadcast_pair(t: &mut T) {
    let (p, r1, r2, _) = pair(false);
    let mut e = engine(&p);
    run_to(&mut e, 60 * SEC);
    // a broadcast network: 2-Way after 10 s, the election when the wait is over (40 s); the
    // first DD goes to the neighbour's address, and the IOS-like ARP drops it while it asks: it
    // goes again after RxmtInterval
    t.eq("2-way", reached(&e, r1, "2.2.2.2", NbrState::TwoWay), Some(10 * SEC + 898));
    t.eq("exstart", reached(&e, r1, "2.2.2.2", NbrState::ExStart), Some(40 * SEC));
    t.check("dd waits for arp", e.trace.events().any(|x| x.dev == r1 && x.t == 40 * SEC && x.note == Note::ArpIncomplete { ip: ip("10.0.12.2") }));
    let dd_out = times(&e, r1, |n| matches!(n, Note::OspfDd { out: true, flags: 7, .. }));
    t.eq("dd again after 5 s", dd_out, alloc::vec![40 * SEC, 45 * SEC]);
    t.eq("full", reached(&e, r1, "2.2.2.2", NbrState::Full), Some(45 * SEC + 5274));
    // R2 is DR (higher router ID), R1 BDR: R2 makes the network-LSA
    t.check("roles", e.ospf(r1).is_some_and(|o| o.ifaces().values().any(|f| f.state == IfState::Backup && f.dr == ip("10.0.12.2") && f.bdr == ip("10.0.12.1"))));
    t.check("network-lsa", e.ospf(r1).and_then(|o| o.database(0)).is_some_and(|d| d.contains_key(&wire::LsaKey { kind: 2, id: ip("10.0.12.2"), adv: ip("2.2.2.2") })));
    // R2's router-LSA came as the answer to R1's request; the next instance (with the transit
    // link, flooded on Full together with the network-LSA) follows within MinLSArrival: dropped
    // unacknowledged, it comes again RxmtInterval later and is taken then
    let dropped = times(&e, r1, |n| matches!(n, Note::OspfLsaDropped { lsa, why: jfn_sim::LsaDrop::MinArrival } if lsa.adv == ip("2.2.2.2") && lsa.seq == 0x8000_0002));
    let taken = times(&e, r1, |n| matches!(n, Note::OspfLsaNew { lsa, .. } if lsa.kind == 1 && lsa.adv == ip("2.2.2.2") && lsa.seq == 0x8000_0002));
    t.eq("dropped, then retransmitted", (dropped, taken), (alloc::vec![45 * SEC + 6702], alloc::vec![50 * SEC + 6446]));
    t.eq("route only then", when(&e, r1, |n| matches!(n, Note::OspfRoute { prefix, what: RtChange::Added, .. } if *prefix == pf("2.2.2.2/32"))), Some(50 * SEC + 200 * MS + 6446));
    t.check("r2 counted the retransmission", e.metric(r2, "ospf", "retransmitted").is_some_and(|n| n >= 1));
    t.eq("routes", ospf_routes(&e, r1), alloc::vec![(s("2.2.2.2/32"), Some(ip("10.0.12.2")), 1, 110)]);
}

fn flooding(t: &mut T) {
    let (p, r, l) = chain();
    let mut e = engine(&p);
    run_to(&mut e, 100 * SEC);
    t.eq("converged", ospf_routes(&e, r[0]).iter().map(|x| (x.0.clone(), x.2)).collect::<Vec<_>>(), alloc::vec![(s("2.2.2.2/32"), 1), (s("3.3.3.3/32"), 2), (s("10.0.23.0/24"), 2)]);
    // R3 gets a second loopback at 100 s: a new router-LSA at once (four links, 72 bytes), flooded
    // to R2 behind the Hello of that moment (848 + 1314 ns) and on to R1 (1314 ns)
    let o = cli(&mut e, r[2], &["enable", "conf t", "interface loopback1", "ip address 10.3.3.33 255.255.255.255", "router ospf 1", "network 10.3.3.33 0.0.0.0 area 0", "end"]);
    t.check("configured", !refused(&o));
    run_to(&mut e, 103 * SEC);
    const SEQ: u32 = 0x8000_0003;
    let mine = |e: &Engine, d: u32, seq: u32| when(e, d, |n| matches!(n, Note::OspfLsaNew { lsa, .. } if lsa.adv == ip("3.3.3.3") && lsa.seq == seq));
    t.eq("originated", when(&e, r[2], |n| matches!(n, Note::OspfLsaOrig { lsa, why: OrigWhy::Changed } if lsa.kind == 1 && lsa.seq == SEQ)), Some(100 * SEC));
    t.eq("flooded on", (mine(&e, r[1], SEQ), mine(&e, r[0], SEQ)), (Some(100 * SEC + 2162), Some(100 * SEC + 3476)));
    // delayed acknowledgements 1 s later to the group; R3 hears R2's (866 ns) and takes the LSA
    // off its list
    let late = |e: &Engine, d: u32, from: u64| times(e, d, |n| matches!(n, Note::OspfAck { out: true, delayed: true, .. })).into_iter().find(|x| *x > from);
    t.eq("delayed acks", (late(&e, r[1], 100 * SEC), late(&e, r[0], 100 * SEC)), (Some(101 * SEC + 2162), Some(101 * SEC + 3476)));
    t.eq("r3 heard it", when(&e, r[2], |n| matches!(n, Note::OspfLsaAcked { nbr, lsa } if *nbr == ip("2.2.2.2") && lsa.seq == SEQ)), Some(101 * SEC + 3028));
    let rxmt_empty = |e: &Engine, d: u32| e.ospf(d).is_some_and(|o| o.ifaces().values().all(|f| f.nbrs.values().all(|n| n.rxmt.is_empty())));
    t.check("nothing left to retransmit", rxmt_empty(&e, r[2]));
    // SPF 200 ms after the new LSA: the route to the loopback
    t.eq("spf after 200 ms", when(&e, r[0], |n| matches!(n, Note::OspfRoute { prefix, what: RtChange::Added, .. } if *prefix == pf("10.3.3.33/32"))), Some(100 * SEC + 200 * MS + 3476));
    t.eq("route", best(&e, r[0], "10.3.3.33/32"), alloc::vec![("O", Some(ip("10.0.12.2")))]);
    // MinLSInterval: another change at 103 s waits until 105 s
    let _ = cli(&mut e, r[2], &["conf t", "interface loopback1", "ip address 10.3.3.34 255.255.255.255", "end"]);
    run_to(&mut e, 104 * SEC);
    t.check("not yet", !notes(&e, r[2]).iter().any(|x| x.0 > 103 * SEC && matches!(x.1, Note::OspfLsaOrig { .. })));
    run_to(&mut e, 106 * SEC);
    t.eq("after minlsinterval", times(&e, r[2], |n| matches!(n, Note::OspfLsaOrig { lsa, .. } if lsa.kind == 1)).last().copied(), Some(105 * SEC));
    t.eq("r2 has it", mine(&e, r[1], SEQ + 1), Some(105 * SEC + 1314));
    // R2's delayed acknowledgement (106 s + 1314 ns) is lost on the cable: R3 sends the update
    // again after RxmtInterval (behind its Hello), R2 has that instance already and answers with
    // a direct acknowledgement at once (13.5: a duplicate that is no implied acknowledgement)
    let before = e.metric(r[2], "ospf", "retransmitted").unwrap_or(0);
    let _ = e.apply(Action::Impair { link: l[1], kbps: None, delay_ns: None, jitter_ns: None, loss_ppm: Some(1_000_000) });
    run_to(&mut e, 108 * SEC);
    t.eq("lost", late(&e, r[1], 105 * SEC + 1314), Some(106 * SEC + 1314));
    let _ = e.apply(Action::Impair { link: l[1], kbps: None, delay_ns: None, jitter_ns: None, loss_ppm: Some(0) });
    run_to(&mut e, 125 * SEC);
    let rx: Vec<u64> = times(&e, r[2], |n| matches!(n, Note::OspfLsu { out: true, why: LsuWhy::Rxmt, .. })).into_iter().filter(|x| *x > 106 * SEC).collect();
    t.eq("retransmitted once", rx, alloc::vec![110 * SEC]);
    let direct = times(&e, r[1], |n| matches!(n, Note::OspfAck { out: true, delayed: false, .. })).into_iter().find(|x| *x > 106 * SEC);
    t.eq("direct ack", direct, Some(110 * SEC + 2162));
    t.eq("acknowledged", when(&e, r[2], |n| matches!(n, Note::OspfLsaAcked { nbr, lsa } if *nbr == ip("2.2.2.2") && lsa.seq == SEQ + 1)), Some(110 * SEC + 3028));
    t.check("the list is empty again", rxmt_empty(&e, r[2]));
    t.eq("r1 has it", best(&e, r[0], "10.3.3.34/32"), alloc::vec![("O", Some(ip("10.0.12.2")))]);
    t.eq("counted", e.metric(r[2], "ospf", "retransmitted"), Some(before + 1));
}

fn spf(t: &mut T) {
    let (p, r, _, _) = triangle();
    let mut e = engine(&p);
    run_to(&mut e, 60 * SEC);
    // costs: 1 for 1 Gbit/s, 48 for the serial line (100 Mbit/s / 2048 kbit/s), 0 for loopbacks
    t.eq(
        "r1's routes",
        ospf_routes(&e, r[0]),
        alloc::vec![
            (s("2.2.2.2/32"), Some(ip("10.0.12.2")), 1, 110),
            (s("3.3.3.3/32"), Some(ip("10.0.13.3")), 1, 110),
            (s("10.0.2.0/24"), Some(ip("10.0.12.2")), 2, 110),
            (s("10.0.3.0/24"), Some(ip("10.0.13.3")), 2, 110),
            (s("10.0.23.0/30"), Some(ip("10.0.12.2")), 49, 110),
            (s("10.0.23.0/30"), Some(ip("10.0.13.3")), 49, 110),
        ],
    );
    t.eq("r2 over the lan, not the serial line", best(&e, r[1], "10.0.3.0/24"), alloc::vec![("O", Some(ip("10.0.12.1")))]);
    let shown = cli(&mut e, r[0], &["enable", "show ip route ospf"]);
    t.check("show ip route ospf", shown.contains("O        10.0.23.0/30 [110/49] via 10.0.12.2, ") && shown.contains("                      [110/49] via 10.0.13.3, ") && !shown.contains("C        "));
    // a cost of 100 on R1's LAN cable: R2's LAN goes round over R3 and the serial line
    let _ = cli(&mut e, r[0], &["conf t", "interface gi0/0/0", "ip ospf cost 100", "end"]);
    run_to(&mut e, 70 * SEC);
    t.eq("over r3", ospf_routes(&e, r[0]).into_iter().filter(|x| x.0 == "10.0.2.0/24").collect::<Vec<_>>(), alloc::vec![(s("10.0.2.0/24"), Some(ip("10.0.13.3")), 50, 110)]);
    t.check("cost in the running config", e.running_config(r[0]).contains("interface GigabitEthernet0/0/0\n ip address 10.0.12.1 255.255.255.0\n ip ospf cost 100\n!\n"));
    let _ = cli(&mut e, r[0], &["conf t", "interface gi0/0/0", "no ip ospf cost", "end"]);
    // a reference bandwidth of 10 Gbit/s everywhere: 10 for 1 Gbit/s, 4882 for the serial line
    for d in r {
        let _ = cli(&mut e, d, &["enable", "conf t", "router ospf 1", "auto-cost reference-bandwidth 10000", "end"]);
    }
    run_to(&mut e, 80 * SEC);
    t.eq("new costs", ospf_routes(&e, r[0]).into_iter().filter(|x| x.0 == "10.0.23.0/30").map(|x| x.2).collect::<Vec<_>>(), alloc::vec![4892, 4892]);
    let brief = cli(&mut e, r[1], &["show ip ospf interface brief"]);
    t.check("brief", brief.starts_with("Interface    PID   Area            IP Address/Mask    Cost  State Nbrs F/C\n") && brief.contains("Se0/2/0      1     0               10.0.23.1/30       4882  P2P   1/1\n") && brief.contains("Gi0/0/0      1     0               10.0.12.2/24       10    DR    1/1\n"));
    // maximum-paths 1: one of the two equal paths
    let _ = cli(&mut e, r[0], &["conf t", "router ospf 1", "maximum-paths 1", "end"]);
    t.eq("one path", best(&e, r[0], "10.0.23.0/30"), alloc::vec![("O", Some(ip("10.0.12.2")))]);
}

fn failure(t: &mut T) {
    let (p, r, l, _) = triangle();
    let mut e = engine(&p);
    run_to(&mut e, 60 * SEC);
    // the LAN cable R1-R2 goes: R1 loses R2 at once, makes a new router-LSA, and SPF 200 ms later
    // sends R2's LAN over R3 and the serial line
    let _ = e.apply(Action::Unplug { link: l[0], down: true });
    run_to(&mut e, 61 * SEC);
    t.eq("killed", when(&e, r[0], |n| matches!(n, Note::OspfNbr { to: NbrState::Down, ev: NbrEvent::KillNbr, .. })), Some(60 * SEC));
    t.check("new router-lsa", notes(&e, r[0]).iter().any(|x| x.0 == 60 * SEC && matches!(x.1, Note::OspfLsaOrig { why: OrigWhy::Changed, lsa } if lsa.kind == 1)));
    let changed = times(&e, r[0], |n| matches!(n, Note::OspfRoute { prefix, what: RtChange::Changed, .. } if *prefix == pf("10.0.2.0/24")));
    t.eq("rerouted", changed.into_iter().find(|x| *x >= 60 * SEC), Some(60 * SEC + 200 * MS));
    t.eq("over r3", ospf_routes(&e, r[0]).into_iter().filter(|x| x.0 == "10.0.2.0/24").collect::<Vec<_>>(), alloc::vec![(s("10.0.2.0/24"), Some(ip("10.0.13.3")), 50, 110)]);
    t.check("the lan between them is gone", best(&e, r[2], "10.0.12.0/24").is_empty());
    // R2 flushed its network-LSA: it is no DR of a network any more
    t.check("network-lsa flushed", notes(&e, r[1]).iter().any(|x| x.0 == 60 * SEC && matches!(x.1, Note::OspfLsaFlush { lsa } if lsa.kind == 2)));
    // back: a new election on the LAN and the old way
    let _ = e.apply(Action::Unplug { link: l[0], down: false });
    run_to(&mut e, 120 * SEC);
    t.eq("back", ospf_routes(&e, r[0]).into_iter().filter(|x| x.0 == "10.0.2.0/24").collect::<Vec<_>>(), alloc::vec![(s("10.0.2.0/24"), Some(ip("10.0.12.2")), 2, 110)]);
}

fn passive(t: &mut T) {
    let (p, r, _, pcs) = triangle();
    let mut e = engine(&p);
    run_to(&mut e, 60 * SEC);
    let lan = p.link_at(pcs[0], "eth0").map_or(0, |x| x.id);
    t.eq("no ospf on the lan", e.trace.hops().filter(|h| h.link == lan && h.mask & decode::P_OSPF != 0).count(), 0);
    t.eq("the lan is advertised", best(&e, r[1], "10.0.1.0/24"), alloc::vec![("O", Some(ip("10.0.12.1")))]);
    t.check("passive and dr at once", e.ospf(r[0]).is_some_and(|o| o.ifaces().values().any(|f| f.name == "GigabitEthernet0/0/2" && f.passive && f.state == IfState::Dr && f.since == 0)));
    // PC1 reaches PC3 over OSPF's routes: R1, then R3 over the point-to-point cable (cost 2
    // against 50 over R2 and the serial line)
    let tr = e.traceroute(pcs[0], ip("10.0.3.10"), TraceOpts::default()).unwrap_or(0);
    run_to(&mut e, 90 * SEC);
    let (hops, reached) = match e.job(tr).map(|j| &j.kind) {
        Some(JobKind::Trace(x)) => (x.hops.iter().map(|h| (h.ttl, h.probes.iter().find_map(|p| p.from))).collect::<Vec<_>>(), x.reached),
        _ => (Vec::new(), false),
    };
    t.eq("traceroute", hops, alloc::vec![(1, Some(ip("10.0.1.1"))), (2, Some(ip("10.0.13.3"))), (3, Some(ip("10.0.3.10")))]);
    t.check("reached", reached);
    // every interface passive: the adjacencies go (the hellos stop, the neighbours forget R1
    // after their dead interval), the networks stay advertised
    let _ = cli(&mut e, r[0], &["enable", "conf t", "router ospf 1", "passive-interface default", "end"]);
    t.check("running config", e.running_config(r[0]).contains("router ospf 1\n router-id 1.1.1.1\n passive-interface default\n network 10.0.0.0 0.255.255.255 area 0\n network 1.1.1.1 0.0.0.0 area 0\n!\n"));
    run_to(&mut e, 140 * SEC);
    t.eq("r1 alone", e.metric(r[0], "ospf", "neighbours"), Some(0));
    t.eq("r2 gave up on r1", state(&e, r[1], "1.1.1.1"), None);
    // one exception again
    let _ = cli(&mut e, r[0], &["conf t", "router ospf 1", "no passive-interface gi0/0/1", "end"]);
    t.check("exception", e.running_config(r[0]).contains(" passive-interface default\n no passive-interface GigabitEthernet0/0/1\n"));
    run_to(&mut e, 160 * SEC);
    t.eq("r3 again", state(&e, r[0], "3.3.3.3"), Some(NbrState::Full));
}

fn redistribution(t: &mut T) {
    let (mut p, r1, r2, _) = pair(true);
    // R2: static routes to a class B subnet and to a whole class C network, two loopbacks outside
    // the networks of OSPF
    let tail2 = "router ospf 1\n router-id 2.2.2.2\n network 10.0.0.0 0.255.255.255 area 0\n network 2.2.2.2 0.0.0.0 area 0\n!\nip route 172.16.5.0 255.255.255.0 10.9.9.2\nip route 192.168.7.0 255.255.255.0 10.9.9.2\n";
    set(&mut p, r2, cfg("R2", &[("Loopback0", "2.2.2.2/32", NONE), ("GigabitEthernet0/0/0", "10.0.12.2/24", P2P), ("Loopback1", "10.9.9.1/24", NONE), ("Loopback2", "198.51.100.1/24", NONE)], tail2).replace("network 10.0.0.0 0.255.255.255 area 0\n network 2.2.2.2", "network 10.0.12.0 0.0.0.255 area 0\n network 2.2.2.2"));
    let mut e = engine(&p);
    t.eq("boot", e.boot_errors(r2), Vec::new());
    run_to(&mut e, 20 * SEC);
    t.eq("nothing external", ospf_routes(&e, r1).len(), 1);
    // without "subnets" only classful networks: 192.168.7.0/24, not 172.16.5.0/24
    let o = cli(&mut e, r2, &["enable", "conf t", "router ospf 1", "redistribute static"]);
    t.check("warned", o.contains("% Only classful networks will be redistributed\n"));
    run_to(&mut e, 30 * SEC);
    t.eq("classful only", ospf_routes(&e, r1).into_iter().filter(|x| x.2 == 20).map(|x| x.0).collect::<Vec<_>>(), alloc::vec![s("192.168.7.0/24")]);
    let shown = cli(&mut e, r1, &["enable", "show ip route"]);
    t.check("E2 in show ip route", shown.contains("O E2  192.168.7.0/24 [110/20] via 10.0.12.2, ") && shown.contains("       O - learned by OSPF, IA - from another area, E1 / E2 - external of type 1 / 2,\n"));
    t.check("asbr", e.ospf(r2).is_some_and(|o| o.is_asbr()) && e.ospf(r1).and_then(|o| o.database(0)).and_then(|d| d.get(&wire::LsaKey { kind: 1, id: ip("2.2.2.2"), adv: ip("2.2.2.2") })).is_some_and(|x| matches!(&x.lsa.body, wire::LsaBody::Router(r) if r.flags & wire::RTR_E != 0)));
    // with subnets, type 1 and a metric of 30: the cost to R2 counts
    let _ = cli(&mut e, r2, &["redistribute static subnets metric 30 metric-type 1", "redistribute connected subnets", "end"]);
    run_to(&mut e, 40 * SEC);
    t.eq(
        "externals",
        ospf_routes(&e, r1).into_iter().filter(|x| x.0 != "2.2.2.2/32").map(|x| (x.0, x.2)).collect::<Vec<_>>(),
        alloc::vec![(s("10.9.9.0/24"), 20), (s("172.16.5.0/24"), 31), (s("192.168.7.0/24"), 31), (s("198.51.100.0/24"), 20)],
    );
    t.eq("e1 kind", e.ospf(r1).and_then(|o| o.route_of(pf("172.16.5.0/24"))).map(|x| x.kind), Some(RtKind::Ext1));
    t.check("running config", e.running_config(r2).contains("router ospf 1\n router-id 2.2.2.2\n redistribute connected subnets\n redistribute static metric 30 metric-type 1 subnets\n network 10.0.12.0 0.0.0.255 area 0\n"));
    // the default route: only with "always" while R2 has none of its own
    let _ = cli(&mut e, r2, &["conf t", "router ospf 1", "default-information originate", "end"]);
    run_to(&mut e, 50 * SEC);
    t.eq("no default yet", best(&e, r1, "0.0.0.0/0"), Vec::new());
    let _ = cli(&mut e, r2, &["conf t", "ip route 0.0.0.0 0.0.0.0 10.9.9.2", "end"]);
    run_to(&mut e, 60 * SEC);
    t.eq("default", best(&e, r1, "0.0.0.0/0"), alloc::vec![("O", Some(ip("10.0.12.2")))]);
    let shown = cli(&mut e, r1, &["show ip route"]);
    t.check("O*E2", shown.contains("Gateway of last resort is 10.0.12.2 to network 0.0.0.0\n") && shown.contains("O*E2  0.0.0.0/0 [110/1] via 10.0.12.2, "));
    let _ = cli(&mut e, r2, &["conf t", "no ip route 0.0.0.0 0.0.0.0 10.9.9.2", "end"]);
    run_to(&mut e, 70 * SEC);
    t.eq("withdrawn", best(&e, r1, "0.0.0.0/0"), Vec::new());
    let _ = cli(&mut e, r2, &["conf t", "router ospf 1", "default-information originate always metric 5", "end"]);
    run_to(&mut e, 80 * SEC);
    t.eq("always", ospf_routes(&e, r1).into_iter().find(|x| x.0 == "0.0.0.0/0").map(|x| (x.1, x.2)), Some((Some(ip("10.0.12.2")), 5)));
    // distances: a static route (1) beats OSPF, a floating one (200) does not; "distance 250"
    // makes OSPF lose against it
    let _ = cli(&mut e, r1, &["conf t", "ip route 192.168.7.0 255.255.255.0 10.0.12.2", "end"]);
    t.eq("static wins", best(&e, r1, "192.168.7.0/24"), alloc::vec![("S", Some(ip("10.0.12.2")))]);
    let _ = cli(&mut e, r1, &["conf t", "no ip route 192.168.7.0 255.255.255.0 10.0.12.2", "ip route 192.168.7.0 255.255.255.0 10.0.12.2 200", "end"]);
    t.eq("ospf over 200", best(&e, r1, "192.168.7.0/24"), alloc::vec![("O", Some(ip("10.0.12.2")))]);
    let _ = cli(&mut e, r1, &["conf t", "router ospf 1", "distance 250", "end"]);
    t.eq("distance 250", best(&e, r1, "192.168.7.0/24"), alloc::vec![("S", Some(ip("10.0.12.2")))]);
}

fn areas(t: &mut T) {
    // R2 (area 0) - R1 (ABR) - R3 (area 1), point-to-point Ethernet; R3 has a LAN 10.1.3.0/24
    // with a PC
    let mut p = Project::new("ospf areas", 45);
    let r: Vec<u32> = (0..3).map(|i| p.add_device("jf-r4", 200 * i, 0).unwrap_or(0)).collect();
    wire(&mut p, r[0], "gi0/0/0", r[1], "gi0/0/0");
    wire(&mut p, r[0], "gi0/0/1", r[2], "gi0/0/0");
    let pc = p.add_device("pc", 400, 300).unwrap_or(0);
    wire(&mut p, pc, "eth0", r[2], "gi0/0/2");
    set(&mut p, pc, s("hostname PC3\nip addr add 10.1.3.10/24 dev eth0\nip route add default via 10.1.3.1\n"));
    let abr = "router ospf 1\n router-id 1.1.1.1\n network 10.0.12.0 0.0.0.255 area 0\n network 1.1.1.1 0.0.0.0 area 0\n network 10.1.0.0 0.0.255.255 area 1\n!\n";
    set(&mut p, r[0], cfg("R1", &[("Loopback0", "1.1.1.1/32", NONE), ("GigabitEthernet0/0/0", "10.0.12.1/24", P2P), ("GigabitEthernet0/0/1", "10.1.13.1/24", P2P)], abr));
    set(&mut p, r[1], cfg("R2", &[("Loopback0", "2.2.2.2/32", NONE), ("GigabitEthernet0/0/0", "10.0.12.2/24", P2P)], &ospf("2.2.2.2")));
    let r3 = "router ospf 1\n router-id 3.3.3.3\n network 10.1.0.0 0.0.255.255 area 1\n!\n";
    set(&mut p, r[2], cfg("R3", &[("GigabitEthernet0/0/0", "10.1.13.3/24", P2P), ("GigabitEthernet0/0/2", "10.1.3.1/24", NONE)], r3));
    let mut e = engine(&p);
    run_to(&mut e, 40 * SEC);
    t.check("abr", e.ospf(r[0]).is_some_and(|o| o.is_abr() && o.attached() == alloc::vec![0, 1]));
    // inter-area routes with the cost to the ABR plus the summary's
    t.eq(
        "r3's routes",
        ospf_routes(&e, r[2]).into_iter().map(|x| (x.0, x.2)).collect::<Vec<_>>(),
        alloc::vec![(s("1.1.1.1/32"), 1), (s("2.2.2.2/32"), 2), (s("10.0.12.0/24"), 2)],
    );
    t.eq("kind", e.ospf(r[2]).and_then(|o| o.route_of(pf("2.2.2.2/32"))).map(|x| x.kind), Some(RtKind::Inter));
    t.eq("r2 sees area 1", ospf_routes(&e, r[1]).into_iter().map(|x| (x.0, x.2)).collect::<Vec<_>>(), alloc::vec![(s("1.1.1.1/32"), 1), (s("10.1.3.0/24"), 3), (s("10.1.13.0/24"), 2)]);
    let shown = cli(&mut e, r[1], &["enable", "show ip route ospf"]);
    t.check("O IA", shown.contains("      10.0.0.0/24 is subnetted, 2 subnets\nO IA     10.1.3.0 [110/3] via 10.0.12.1, "));
    // the summary-LSAs of the ABR in area 0: area 1's networks
    let sums: Vec<(Ipv4, u32)> = e.ospf(r[1]).and_then(|o| o.database(0)).map(|d| d.values().filter_map(|x| if let wire::LsaBody::Summary(sm) = &x.lsa.body { Some((x.lsa.hdr.id, sm.metric)) } else { None }).collect()).unwrap_or_default();
    t.eq("summaries in area 0", sums, alloc::vec![(ip("10.1.3.0"), 2), (ip("10.1.13.0"), 1)]);
    let db = cli(&mut e, r[1], &["show ip ospf database"]);
    t.check("database", db.contains("\n                Summary Net Link States (Area 0)\n\nLink ID         ADV Router      Age         Seq#       Checksum\n10.1.3.0        1.1.1.1         "));
    // area 1 becomes a stub area: no externals, a default summary of cost 1; with no-summary
    // only that one
    let _ = cli(&mut e, r[0], &["enable", "conf t", "router ospf 1", "area 1 stub", "end"]);
    run_to(&mut e, 50 * SEC);
    // R3 does not agree yet: the E bit of its Hellos differs, R1 ignores them
    t.check("stub mismatch", notes(&e, r[0]).iter().any(|x| matches!(x.1, Note::OspfIgnored { why: OspfDrop::Stub, .. })));
    let _ = cli(&mut e, r[2], &["enable", "conf t", "router ospf 1", "area 1 stub", "end"]);
    run_to(&mut e, 100 * SEC);
    t.eq("full again", state(&e, r[2], "1.1.1.1"), Some(NbrState::Full));
    t.eq("stub default", ospf_routes(&e, r[2]).into_iter().find(|x| x.0 == "0.0.0.0/0").map(|x| (x.1, x.2)), Some((Some(ip("10.1.13.1")), 2)));
    let _ = cli(&mut e, r[0], &["conf t", "router ospf 1", "area 1 stub no-summary", "end"]);
    run_to(&mut e, 110 * SEC);
    t.eq("no-summary", ospf_routes(&e, r[2]).into_iter().map(|x| x.0).collect::<Vec<_>>(), alloc::vec![s("0.0.0.0/0")]);
    t.check("running config", e.running_config(r[0]).contains("router ospf 1\n router-id 1.1.1.1\n area 1 stub no-summary\n network 10.0.12.0 0.0.0.255 area 0\n"));
    let shown = cli(&mut e, r[2], &["show ip route"]);
    t.check("O*IA", shown.contains("O*IA  0.0.0.0/0 [110/2] via 10.1.13.1, "));
}

fn aging(t: &mut T) {
    let (p, r1, r2, _) = pair(true);
    let mut e = engine(&p);
    run_to(&mut e, 20 * SEC);
    let seq = |e: &Engine, d: u32, id: &str| e.ospf(d).and_then(|o| o.database(0)).and_then(|x| x.get(&wire::LsaKey { kind: 1, id: ip(id), adv: ip(id) })).map(|x| x.lsa.hdr.seq);
    let made = times(&e, r1, |n| matches!(n, Note::OspfLsaOrig { lsa, .. } if lsa.kind == 1)).last().copied().unwrap_or(0);
    // every own LSA is renewed after LSRefreshTime (1800 s)
    run_to(&mut e, made + 1800 * SEC + MS);
    t.eq("refreshed", times(&e, r1, |n| matches!(n, Note::OspfLsaOrig { why: OrigWhy::Refresh, .. })), alloc::vec![made + 1800 * SEC]);
    t.eq("r2 has the new one", seq(&e, r2, "1.1.1.1"), Some(0x8000_0003));
    // R2 goes away: its router-LSA stays in R1's database until it is MaxAge, then is flooded once
    // more and removed (nobody is left to acknowledge it)
    let off = e.now();
    let _ = e.apply(Action::Power { dev: r2, on: false });
    let r2made = e.ospf(r1).and_then(|o| o.database(0)).and_then(|x| x.get(&wire::LsaKey { kind: 1, id: ip("2.2.2.2"), adv: ip("2.2.2.2") })).map(|x| x.at - x.age0 as u64 * SEC).unwrap_or(0);
    run_to(&mut e, off + 60 * SEC);
    t.eq("unreachable at once", ospf_routes(&e, r1), Vec::new());
    t.check("still in the database", seq(&e, r1, "2.2.2.2").is_some());
    run_to(&mut e, r2made + 3600 * SEC + 10 * SEC);
    let aged = when(&e, r1, |n| matches!(n, Note::OspfLsaAged { lsa } if lsa.adv == ip("2.2.2.2")));
    t.check("maxage", aged.is_some_and(|a| a + 2 * SEC > r2made + 3600 * SEC && a <= r2made + 3601 * SEC));
    t.eq("removed", when(&e, r1, |n| matches!(n, Note::OspfLsaGone { lsa } if lsa.adv == ip("2.2.2.2"))), aged);
    t.eq("gone", seq(&e, r1, "2.2.2.2"), None);
}

fn mismatches(t: &mut T) {
    let (p, r1, r2, _) = pair(true);
    let mut e = engine(&p);
    run_to(&mut e, 20 * SEC);
    // another hello interval on R2: R1 ignores its Hellos and loses it after the dead interval
    let _ = cli(&mut e, r2, &["enable", "conf t", "interface gi0/0/0", "ip ospf hello-interval 5", "end"]);
    run_to(&mut e, 70 * SEC);
    t.check("hello mismatch", notes(&e, r1).iter().any(|x| matches!(x.1, Note::OspfIgnored { why: OspfDrop::HelloTime, .. })));
    t.check("dead interval follows the hello interval", e.ospf(r2).is_some_and(|o| o.ifaces().values().any(|f| f.hello == 5 && f.dead == 20)));
    t.eq("apart", state(&e, r1, "2.2.2.2"), None);
    let _ = cli(&mut e, r2, &["conf t", "interface gi0/0/0", "no ip ospf hello-interval", "end"]);
    run_to(&mut e, 100 * SEC);
    t.eq("together", state(&e, r1, "2.2.2.2"), Some(NbrState::Full));
    // another area on the link: ignored
    let _ = cli(&mut e, r2, &["conf t", "interface gi0/0/0", "ip ospf 1 area 5", "end"]);
    run_to(&mut e, 120 * SEC);
    t.check("area mismatch", notes(&e, r1).iter().any(|x| matches!(x.1, Note::OspfIgnored { why: OspfDrop::Area, .. })));
    t.check("interface line", e.running_config(r2).contains(" ip ospf 1 area 5\n"));
    let _ = cli(&mut e, r2, &["conf t", "interface gi0/0/0", "no ip ospf 1 area", "end"]);
    // a smaller MTU on R1: R2's DDs are refused there, the neighbours stay in ExStart and Exchange
    let _ = cli(&mut e, r1, &["enable", "conf t", "interface gi0/0/0", "ip mtu 1400", "end"]);
    let _ = e.apply(Action::Unplug { link: 1, down: false });
    let _ = cli(&mut e, r1, &["clear ip ospf process", "yes"]);
    run_to(&mut e, 180 * SEC);
    t.check("mtu mismatch", notes(&e, r1).iter().any(|x| x.0 > 120 * SEC && matches!(x.1, Note::OspfIgnored { why: OspfDrop::Mtu, .. })));
    t.check("stuck", state(&e, r1, "2.2.2.2").is_some_and(|s| s == NbrState::ExStart) && state(&e, r2, "1.1.1.1").is_some_and(|s| s == NbrState::Exchange || s == NbrState::ExStart));
}

fn command_line(t: &mut T) {
    // an L3 switch needs "ip routing"
    let mut p = Project::new("ospf cli", 46);
    let mls = p.add_device("jf-s24l3", 0, 0).unwrap_or(0);
    let (q, r1, _, _) = pair(true);
    let mut e = engine(&p);
    let o = cli(&mut e, mls, &["enable", "conf t", "router ospf 1"]);
    t.check("routing needed", o.ends_with("% IP routing not enabled\n") && e.cli_prompt(mls) == "MLS1(config)#");
    let o = cli(&mut e, mls, &["ip routing", "router ospf 7"]);
    t.check("router mode", !refused(&o) && e.cli_prompt(mls) == "MLS1(config-router)#");
    let h = e.cli_help(mls, "");
    t.check("help of the mode", h.starts_with("Commands for this OSPF process:\n") && h.contains("  network ") && h.contains("  passive-interface ") && h.contains("  log-adjacency-changes ") && !h.contains("  version "));
    t.eq("network help", e.cli_help(mls, "network 10.0.0.0 0.0.0.255 area "), s("  <0-4294967295>  Area number (0: the backbone)\n  A.B.C.D         Area number written like an address\n\n"));
    let o = cli(&mut e, mls, &["exit", "router ospf 8"]);
    t.eq("one process", o, s("% Only one OSPF process per device here: process 7 runs\n"));
    // RIP keeps its own mode behind the same prompt
    let _ = cli(&mut e, mls, &["router rip"]);
    t.check("rip mode", e.cli_prompt(mls) == "MLS1(config-router)#" && e.cli_help(mls, "").starts_with("Commands for this routing process:\n"));
    let _ = cli(&mut e, mls, &["no router rip", "end"]);
    // the pair with its show commands
    let mut e = engine(&q);
    run_to(&mut e, 30 * SEC);
    let o = cli(&mut e, r1, &["enable", "show ip ospf"]);
    t.check("show ip ospf", o.starts_with("OSPF process 1 with router ID 1.1.1.1 (RFC 2328)\n") && o.contains("  Areas: 1 (1 normal, 0 stub)\n    Area 0 (backbone)\n      Interfaces: 2 (0 passive), full adjacencies: 1\n      LSAs: 2\n"));
    let o = cli(&mut e, r1, &["show ip ospf interface gi0/0/0"]);
    t.check("show ip ospf interface", o.starts_with("GigabitEthernet0/0/0 is up, line protocol is up\n  Internet address 10.0.12.1/24, area 0, process 1, router ID 1.1.1.1\n  Network type POINT_TO_POINT, cost 1, transmit delay 1 s, state P2P, priority 1\n") && o.contains("  Neighbours: 1, adjacent: 1 (2.2.2.2)\n"));
    let o = cli(&mut e, r1, &["show ip ospf database"]);
    t.check("show ip ospf database", o.starts_with("\n            OSPF router 1.1.1.1, process 1\n\n                Router Link States (Area 0)\n\nLink ID         ADV Router      Age         Seq#       Checksum Link count\n1.1.1.1         1.1.1.1         ") && o.contains(" 0x80000002 0x"));
    let o = cli(&mut e, r1, &["show ip protocols"]);
    t.check("show ip protocols", o.starts_with("Routing process \"ospf 1\" with router ID 1.1.1.1 (RFC 2328)\n") && o.contains("  Routing for networks:\n    10.0.0.0 0.255.255.255 area 0\n    1.1.1.1 0.0.0.0 area 0\n") && o.contains("    2.2.2.2              110      00:00:"));
    // debug ip ospf hello: one line per Hello in and out
    let o = cli(&mut e, r1, &["debug ip ospf hello"]);
    t.eq("debug on", o, s("Debug output for OSPF hellos: on\n"));
    t.check("show debugging", cli(&mut e, r1, &["show debugging"]) == "Debug output for OSPF hellos: on\n");
    let _ = e.take_reports();
    run_to(&mut e, 41 * SEC);
    let o = out(&mut e, r1);
    t.check("debug lines", o.contains(": OSPF: hello out of GigabitEthernet0/0/0 to 224.0.0.5, 1 neighbour named\n") && o.contains(": OSPF: hello from 2.2.2.2 on GigabitEthernet0/0/0\n") && !o.contains("DD"));
    let _ = cli(&mut e, r1, &["undebug all"]);
    // the running configuration boots the same router again
    let _ = cli(&mut e, r1, &["conf t", "router ospf 1", "log-adjacency-changes detail", "auto-cost reference-bandwidth 1000", "distance 115", "maximum-paths 2", "default-information originate always metric-type 1", "interface gi0/0/0", "ip ospf priority 7", "ip ospf dead-interval 60", "end"]);
    let text = e.running_config(r1);
    t.check("block", text.contains("router ospf 1\n router-id 1.1.1.1\n log-adjacency-changes detail\n auto-cost reference-bandwidth 1000\n network 10.0.0.0 0.255.255.255 area 0\n network 1.1.1.1 0.0.0.0 area 0\n default-information originate always metric-type 1\n distance 115\n maximum-paths 2\n!\n"));
    t.check("interface lines", text.contains("interface GigabitEthernet0/0/0\n ip address 10.0.12.1 255.255.255.0\n ip ospf network point-to-point\n ip ospf dead-interval 60\n ip ospf priority 7\n!\n"));
    let mut q2 = q.clone();
    set(&mut q2, r1, text.clone());
    let e2 = engine(&q2);
    t.eq("boots without errors", e2.boot_errors(r1), Vec::new());
    t.eq("same configuration", e2.running_config(r1), text);
    // "log-adjacency-changes detail": every state on the console (the dead interval agrees with
    // R2's again first)
    let _ = cli(&mut e, r1, &["conf t", "interface gi0/0/0", "no ip ospf dead-interval", "end"]);
    let _ = cli(&mut e, r1, &["clear ip ospf process"]);
    t.eq("question", e.cli_prompt(r1), s("Start every OSPF process anew? [no]: "));
    let mut o = cli(&mut e, r1, &["yes"]);
    let now = e.now();
    run_to(&mut e, now + 12 * SEC);
    o.push_str(&out(&mut e, r1));
    t.check("adjacency log", o.contains("%OSPF-5-ADJCHG: process 1, neighbour 2.2.2.2 on GigabitEthernet0/0/0: FULL -> DOWN (KillNbr)\n") && o.contains("%OSPF-5-ADJCHG: process 1, neighbour 2.2.2.2 on GigabitEthernet0/0/0: DOWN -> INIT (HelloReceived)\n"));
    // its own LSAs came back from R2 newer than the fresh start: a new instance one further
    t.check("reclaimed", notes(&e, r1).iter().any(|x| matches!(x.1, Note::OspfLsaOrig { why: OrigWhy::Reclaim, .. })));
    // no router ospf: the routes go, the interface settings stay
    let _ = cli(&mut e, r1, &["conf t", "no router ospf 1", "end"]);
    t.eq("withdrawn", ospf_routes(&e, r1), Vec::new());
    let run = e.running_config(r1);
    t.check("block gone, interface lines kept", !run.contains("router ospf") && run.contains(" ip ospf priority 7\n"));
    t.check("stopped", e.services(r1).iter().any(|x| x.kind == "ospf" && !x.running && !x.enabled));
}

fn texts(t: &mut T) {
    let rows: Vec<(&str, &str)> = include_str!("locales/engine.tsv").lines().skip(1).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect();
    let en = move |k: &'static str| rows.iter().find(|r| r.0 == k).map_or(k, |r| r.1);
    let name = |i: u16| format!("Gi0/0/{}", i);
    let port = |p: u16| format!("p{}", p);
    let lsa = jfn_sim::LsaRef { kind: 1, id: ip("2.2.2.2"), adv: ip("2.2.2.2"), seq: 0x8000_0003, area: Some(0) };
    let cases: Vec<(Note, &str, &str)> = alloc::vec![
        (
            Note::OspfNbr { nbr: ip("2.2.2.2"), iface: 0, from: NbrState::Loading, to: NbrState::Full, ev: NbrEvent::LoadingDone },
            "OSPF: Nachbar 2.2.2.2 an Gi0/0/0: Loading \u{2192} Full (Loading Done).",
            "OSPF: neighbour 2.2.2.2 on Gi0/0/0: Loading \u{2192} Full (Loading Done).",
        ),
        (
            Note::OspfIgnored { from: ip("10.0.12.2"), iface: 0, why: OspfDrop::DeadTime },
            "OSPF: Paket von 10.0.12.2 über Gi0/0/0 ignoriert: das Dead-Intervall passt nicht.",
            "OSPF: packet from 10.0.12.2 on Gi0/0/0 ignored: the dead interval does not match.",
        ),
        (Note::OspfDr { iface: 1, dr: ip("2.2.2.2"), bdr: Ipv4::UNSPECIFIED }, "OSPF: Wahl an Gi0/0/1: DR ist 2.2.2.2, BDR ist keiner.", "OSPF: election on Gi0/0/1: the DR is 2.2.2.2, the BDR is none."),
        (Note::OspfLsaNew { lsa, from: ip("3.3.3.3") }, "OSPF: Router-LSA 2.2.2.2 von 2.2.2.2 (0x80000003, Bereich 0) von 3.3.3.3 übernommen.", "OSPF: Router-LSA 2.2.2.2 from 2.2.2.2 (0x80000003, area 0) taken from 3.3.3.3."),
        (
            Note::OspfDd { nbr: ip("2.2.2.2"), iface: 0, out: true, flags: 7, seq: 0xa1b2, lsas: 0 },
            "OSPF: Database Description an 2.2.2.2 über Gi0/0/0 (Flags I, M, MS, Nummer 0x0000a1b2, LSA-Köpfe: 0).",
            "OSPF: database description to 2.2.2.2 on Gi0/0/0 (flags I, M, MS, sequence 0x0000a1b2, LSA headers: 0).",
        ),
        (
            Note::OspfRoute { prefix: pf("10.0.2.0/24"), kind: RtKind::Inter, cost: 3, via: ip("10.0.12.1"), what: RtChange::Added },
            "OSPF: Route 10.0.2.0/24 (aus einem anderen Bereich) über 10.0.12.1, Kosten 3.",
            "OSPF: route 10.0.2.0/24 (inter-area) via 10.0.12.1, cost 3.",
        ),
        (Note::OspfSpf { run: 4, routes: 7, added: 1, changed: 2, removed: 0 }, "OSPF: SPF-Lauf 4: Routen: 7, neu: 1, geändert: 2, entfallen: 0.", "OSPF: SPF run 4: routes: 7, new: 1, changed: 2, gone: 0."),
    ];
    for (n, de, english) in cases {
        let m = n.message(&port, &name);
        t.eq(&format!("german {:?}", n), m.text(), s(de));
        t.eq(&format!("english {:?}", n), m.text_with(&en), s(english));
    }
    let n = Note::OspfHelloSent { iface: 0, nbrs: 1 };
    t.eq("layer and mask", (n.layer(), n.mask() & decode::P_OSPF, n.is_drop()), (3, decode::P_OSPF, false));
    t.check("drops", Note::OspfIgnored { from: ip("10.0.0.1"), iface: 0, why: OspfDrop::Area }.is_drop());
}

fn determinism(t: &mut T) {
    let (p, _, l, _) = triangle();
    let digest = |e: &Engine| -> Vec<(u64, u32, Vec<u8>)> { e.trace.hops().filter(|h| h.mask & decode::P_OSPF != 0).map(|h| (h.t_tx, h.link, h.bytes.to_vec())).collect() };
    let mut a = engine(&p);
    let mut b = engine(&p);
    run_to(&mut a, 50 * SEC);
    let mut c = a.clone();
    let _ = a.apply(Action::Unplug { link: l[2], down: true });
    let _ = c.apply(Action::Unplug { link: l[2], down: true });
    run_to(&mut b, 50 * SEC);
    let _ = b.apply(Action::Unplug { link: l[2], down: true });
    for e in [&mut a, &mut b, &mut c] {
        run_to(e, 200 * SEC);
    }
    let (da, db, dc) = (digest(&a), digest(&b), digest(&c));
    t.check("two runs alike", da == db && !da.is_empty());
    t.check("a clone goes on alike", da == dc);
    let ns = |e: &Engine| e.trace.events().filter(|x| x.note.is_ospf()).map(|x| (x.t, x.dev, x.note.clone())).collect::<Vec<_>>();
    t.check("the same decisions", ns(&a) == ns(&b));
    t.eq("ospf frames", da.len(), 159);
}
