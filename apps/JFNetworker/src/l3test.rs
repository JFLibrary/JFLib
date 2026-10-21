// l3test -- checks of the IPv4 part of jfn_sim (part of --selftest): ARP, ping and traceroute
// with exact virtual round trip times, routing through a router and an L3 switch, ICMP errors
// (no route, ARP failure, TTL, fragmentation needed) and their rate limits, proxy ARP,
// directed broadcasts,
// fragments and reassembly, 802.1Q subinterfaces, serial lines with HDLC, loopbacks and
// martians, refused settings, the views and the end of jobs
//
// Times: 1 Gbit/s copper with 50 ns of cable.  A frame of n bytes takes (max(n, 60) + 24) * 8
// ns on the wire, so an ARP frame 672 + 50 = 722 ns and a 98-byte echo frame 976 + 50 =
// 1026 ns per cable; devices add no time of their own.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use jfn_model::project::{PortRef, Project};
use jfn_sim::node::port_mac;
use jfn_sim::{ActErr, Action, Engine, IcmpKind, JobKind, Note, PingEv, PingOpts, RouteSrc, TraceHop, TraceOpts, TraceProbe, VlanSet, MS, SEC};
use netpkt::decode;
use netpkt::eth::{EthHdr, VlanTag, ET_ARP, ET_IPV4};
use netpkt::icmp::Icmp;
use netpkt::ipv4::{Ipv4Hdr, PROTO_ICMP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::{Ipv4, Mac, Prefix4};

use crate::selftest::T;

const BUDGET: usize = 1_000_000;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn ip(s: &str) -> Ipv4 {
    Ipv4::parse(s).unwrap_or(Ipv4::UNSPECIFIED)
}

fn pf(s: &str) -> Prefix4 {
    Prefix4::parse(s).unwrap_or(Prefix4 { addr: Ipv4::UNSPECIFIED, len: 0 })
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn addr(e: &mut Engine, dev: u32, iface: &str, p: &str) -> Result<(), ActErr> {
    e.apply(Action::IfAddr { dev, iface: String::from(iface), prefix: pf(p), add: true })
}

fn if_up(e: &mut Engine, dev: u32, iface: &str, up: bool) -> Result<(), ActErr> {
    e.apply(Action::IfUp { dev, iface: String::from(iface), up })
}

fn gw(e: &mut Engine, dev: u32, via: &str) -> Result<(), ActErr> {
    e.apply(Action::Route { dev, prefix: pf("0.0.0.0/0"), via: Some(ip(via)), iface: None, ad: 1, add: true })
}

fn once() -> PingOpts {
    PingOpts { count: 1, ..PingOpts::default() }
}

fn ping(e: &mut Engine, dev: u32, dst: &str, o: PingOpts) -> u32 {
    e.ping(dev, ip(dst), o).unwrap_or(0)
}

/// what a ping job saw
fn ping_ev(e: &Engine, id: u32) -> Vec<PingEv> {
    match e.job(id).map(|j| &j.kind) {
        Some(JobKind::Ping(p)) => p.events.clone(),
        _ => Vec::new(),
    }
}

fn sent(e: &Engine, id: u32) -> u32 {
    match e.job(id).map(|j| &j.kind) {
        Some(JobKind::Ping(p)) => p.sent,
        _ => 0,
    }
}

fn done(e: &Engine, id: u32) -> bool {
    e.job(id).is_some_and(|j| j.done)
}

fn reply(seq: u16, from: &str, bytes: u16, ttl: u8, rtt: u64) -> PingEv {
    PingEv::Reply { seq, from: ip(from), bytes, ttl, rtt }
}

fn error(seq: u16, from: &str, kind: IcmpKind) -> PingEv {
    PingEv::Error { seq, from: ip(from), kind }
}

/// the notes of a device, in order
fn notes(e: &Engine, dev: u32) -> Vec<Note> {
    e.trace.events().filter(|x| x.dev == dev).map(|x| x.note.clone()).collect()
}

/// PC1 (10.0.0.1) and PC2 (10.0.0.2) on SW: (engine, pc1, pc2, sw, set-up ok)
fn lan2() -> (Engine, u32, u32, u32, bool) {
    let mut p = Project::new("lan", 11);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 200, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 100).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(pc2, "eth0"), &pr(sw, "gi1/0/2"), None);
    let mut e = engine(&p);
    let ok = [addr(&mut e, pc1, "eth0", "10.0.0.1/24"), addr(&mut e, pc2, "eth0", "10.0.0.2/24")];
    (e, pc1, pc2, sw, ok.iter().all(|r| r.is_ok()))
}

/// PC1 (10.0.1.10) - Gi0/0/0 R1 Gi0/0/1 - PC2 (10.0.2.10), default routes on the PCs:
/// (engine, pc1, r1, pc2, set-up ok)
fn routed() -> (Engine, u32, u32, u32, bool) {
    let mut p = Project::new("routed", 12);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 100, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(r1, "gi0/0/0"), None);
    let _ = p.connect(&pr(r1, "gi0/0/1"), &pr(pc2, "eth0"), None);
    let mut e = engine(&p);
    let ok = [
        addr(&mut e, pc1, "eth0", "10.0.1.10/24"),
        gw(&mut e, pc1, "10.0.1.1"),
        addr(&mut e, pc2, "eth0", "10.0.2.10/24"),
        gw(&mut e, pc2, "10.0.2.1"),
        addr(&mut e, r1, "Gi0/0/0", "10.0.1.1/24"),
        addr(&mut e, r1, "gi0/0/1", "10.0.2.1/24"),
        if_up(&mut e, r1, "GigabitEthernet0/0/0", true),
        if_up(&mut e, r1, "Gi0/0/1", true),
    ];
    (e, pc1, r1, pc2, ok.iter().all(|r| r.is_ok()))
}

pub fn run(t: &mut T) {
    same_subnet(t);
    through_router(t);
    failures(t);
    proxy_arp_broadcast(t);
    fragments_mtu(t);
    l3_switch(t);
    router_on_a_stick(t);
    management_svi(t);
    linux_router(t);
    icmp_rates(t);
    serial(t);
    loopbacks(t);
    refused(t);
    views(t);
    jobs_end(t);
    determinism(t);
}

fn same_subnet(t: &mut T) {
    let (mut e, pc1, pc2, sw, ok) = lan2();
    t.check("lan set-up", ok);
    let id = ping(&mut e, pc1, "10.0.0.2", PingOpts::default());
    e.run_idle(BUDGET);
    // seq 1 first asks for the MAC: 4 x 722 ns of ARP, then 4 x 1026 ns of echo; later
    // probes find it in the cache
    t.eq("ping", ping_ev(&e, id), vec![reply(1, "10.0.0.2", 84, 64, 6992), reply(2, "10.0.0.2", 84, 64, 4104), reply(3, "10.0.0.2", 84, 64, 4104), reply(4, "10.0.0.2", 84, 64, 4104)]);
    t.check("ping done", done(&e, id));
    t.eq("probe times", e.trace.hops().filter(|h| h.from.0 == pc1 && h.mask & decode::P_ICMP != 0).map(|h| h.t_tx).collect::<Vec<_>>(), vec![2888, SEC, 2 * SEC, 3 * SEC]);
    // eth0 is interface 1 of a PC (lo is 0)
    t.check("arp asked", notes(&e, pc1).contains(&Note::ArpRequest { iface: 1, ip: ip("10.0.0.2") }));
    t.check("arp learned from the request", notes(&e, pc2).contains(&Note::ArpLearned { ip: ip("10.0.0.1"), mac: port_mac(pc1, 0) }));
    t.check("arp learned from the reply", notes(&e, pc1).contains(&Note::ArpLearned { ip: ip("10.0.0.2"), mac: port_mac(pc2, 0) }));
    t.eq("one arp exchange", e.trace.hops().filter(|h| h.mask & decode::P_ARP != 0).count(), 4);
    t.eq("echo answered", notes(&e, pc2).iter().filter(|n| **n == Note::EchoAnswered { to: ip("10.0.0.1") }).count(), 4);
    t.check("switch only bridges", !notes(&e, sw).iter().any(|n| n.layer() >= 3));
    let arp: Vec<_> = e.arp_table(pc1).iter().map(|r| (r.iface, r.ip, r.mac, r.fixed)).collect();
    t.eq("arp table", arp, vec![(1, ip("10.0.0.2"), Some(port_mac(pc2, 0)), false)]);
}

fn through_router(t: &mut T) {
    let (mut e, pc1, r1, pc2, ok) = routed();
    t.check("router set-up", ok);
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts::default());
    e.run_idle(BUDGET);
    // IOS drops seq 1 while it asks for PC2 (".!!!!"); then the same times as on the switch,
    // and the reply lost one TTL on the way
    t.eq("routed ping", ping_ev(&e, id), vec![reply(2, "10.0.2.10", 84, 63, 4104), PingEv::Timeout { seq: 1 }, reply(3, "10.0.2.10", 84, 63, 4104), reply(4, "10.0.2.10", 84, 63, 4104)]);
    t.check("dropped while asking", notes(&e, r1).contains(&Note::ArpIncomplete { ip: ip("10.0.2.10") }));
    t.check("routed", notes(&e, r1).contains(&Note::IpRouted { dst: ip("10.0.2.10"), gw: ip("10.0.2.10"), iface: 1 }));
    t.check("routed back over the gateway", notes(&e, r1).contains(&Note::IpRouted { dst: ip("10.0.1.10"), gw: ip("10.0.1.10"), iface: 0 }));
    let routed_ev = e.trace.events().find(|x| x.dev == r1 && matches!(x.note, Note::IpRouted { iface: 1, .. })).cloned();
    t.eq("describe", routed_ev.map(|ev| e.describe(&ev)), Some(String::from("Route zu 10.0.2.10: direkt angeschlossen an GigabitEthernet0/0/1.")));
    let ask = e.trace.events().find(|x| x.dev == pc1 && matches!(x.note, Note::ArpRequest { .. })).cloned();
    t.eq("describe arp", ask.map(|ev| e.describe(&ev)), Some(String::from("ARP-Anfrage an eth0: Wer hat 10.0.1.1?")));
    // traceroute: TTL 1 ends at the router (Time Exceeded from the inbound interface, 28
    // quoted bytes from IOS: 834 + 802 ns), TTL 2 reaches PC2 (Port Unreachable quoting the
    // whole 60-byte probe from Linux: 2 x 834 + 2 x 1058 ns)
    let tr = e.traceroute(pc1, ip("10.0.2.10"), TraceOpts::default()).unwrap_or(0);
    e.run_idle(BUDGET);
    let (hops, reached) = match e.job(tr).map(|j| &j.kind) {
        Some(JobKind::Trace(x)) => (x.hops.clone(), x.reached),
        _ => (Vec::new(), false),
    };
    let p1 = TraceProbe { from: Some(ip("10.0.1.1")), rtt: 1636, kind: Some(IcmpKind::TtlExceeded) };
    let p2 = TraceProbe { from: Some(ip("10.0.2.10")), rtt: 3784, kind: Some(IcmpKind::PortUnreachable) };
    t.eq("traceroute", hops, vec![TraceHop { ttl: 1, probes: vec![p1; 3] }, TraceHop { ttl: 2, probes: vec![p2; 3] }]);
    t.check("traceroute done", reached && done(&e, tr));
    t.check("ttl expired at the router", notes(&e, r1).contains(&Note::IpTtlExpired { src: ip("10.0.1.10") }));
    t.check("time exceeded sent", notes(&e, r1).contains(&Note::IcmpSent { typ: 11, code: 0, to: ip("10.0.1.10") }));
    t.check("no listener at the target", notes(&e, pc2).contains(&Note::UdpNoPort { port: 33437 }));
    t.check("port unreachable sent", notes(&e, pc2).contains(&Note::IcmpSent { typ: 3, code: 3, to: ip("10.0.1.10") }));
    let sizes: Vec<usize> = e.trace.hops().filter(|h| h.mask & decode::P_ICMP != 0 && h.to.0 == pc1 && h.t_tx > 4 * SEC).map(|h| h.bytes.len()).collect();
    t.eq("icmp error sizes", sizes, vec![70, 70, 70, 102, 102, 102]);
}

fn failures(t: &mut T) {
    // the host has no route at all
    let (mut e, pc1, r1, _pc2, _) = routed();
    let _ = e.apply(Action::Route { dev: pc1, prefix: pf("0.0.0.0/0"), via: Some(ip("10.0.1.1")), iface: None, ad: 1, add: false });
    let id = ping(&mut e, pc1, "192.0.2.1", PingOpts::default());
    t.eq("no route on the host", ping_ev(&e, id), vec![error(1, "0.0.0.0", IcmpKind::NoRoute)]);
    t.check("no route ends the job", done(&e, id) && e.trace.hops().count() == 0);
    // the router has none: net unreachable from its inbound interface
    let (mut e, pc1, r1b, _pc2, _) = routed();
    let id = ping(&mut e, pc1, "192.0.2.1", once());
    e.run_idle(BUDGET);
    t.eq("net unreachable", ping_ev(&e, id), vec![error(1, "10.0.1.1", IcmpKind::NetUnreachable)]);
    t.check("router has no route", notes(&e, r1b).contains(&Note::IpNoRoute { dst: ip("192.0.2.1") }));
    // nobody answers ARP: three requests a second apart, then host unreachable
    let (mut e, pc1, _r1, _pc2, _) = routed();
    let id = ping(&mut e, pc1, "10.0.1.99", PingOpts { count: 1, timeout: 5 * SEC, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("arp fails", ping_ev(&e, id), vec![error(1, "10.0.1.10", IcmpKind::HostUnreachable)]);
    let asked: Vec<u64> = e.trace.events().filter(|x| x.dev == pc1 && x.note == Note::ArpRequest { iface: 1, ip: ip("10.0.1.99") }).map(|x| x.t).collect();
    t.eq("three requests", asked, vec![0, SEC, 2 * SEC]);
    t.check("given up after 3 s", e.trace.events().any(|x| x.dev == pc1 && x.note == Note::ArpFailed { ip: ip("10.0.1.99") } && x.t == 3 * SEC));
    // ... behind an IOS router: it kept nothing back, so it stays silent
    let id = ping(&mut e, pc1, "10.0.2.99", PingOpts { count: 1, timeout: 5 * SEC, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("ios router stays silent", ping_ev(&e, id), vec![PingEv::Timeout { seq: 1 }]);
    // TTL 1 ends at the router
    let (mut e, pc1, _r1, _pc2, _) = routed();
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts { count: 1, ttl: Some(1), ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("ttl exceeded", ping_ev(&e, id), vec![error(1, "10.0.1.1", IcmpKind::TtlExceeded)]);
    // an interface that goes down takes its connected route with it
    let (mut e, pc1, r1c, _pc2, _) = routed();
    t.check("shutdown", if_up(&mut e, r1c, "gi0/0/1", false).is_ok());
    let id = ping(&mut e, pc1, "10.0.2.10", once());
    e.run_idle(BUDGET);
    t.eq("shut interface", ping_ev(&e, id), vec![error(1, "10.0.1.1", IcmpKind::NetUnreachable)]);
    // a cable that loses everything: timeouts after 2 s
    let (mut e, pc1, r1d, pc2, _) = routed();
    let id = ping(&mut e, pc1, "10.0.2.10", once());
    e.run_idle(BUDGET);
    t.eq("warm-up dropped by IOS", ping_ev(&e, id), vec![PingEv::Timeout { seq: 1 }]);
    t.check("but the router learned PC2", e.arp_table(r1d).iter().any(|r| r.ip == ip("10.0.2.10") && r.mac == Some(port_mac(pc2, 0))));
    let l = e.view().links.iter().find(|l| l.ends.iter().any(|x| x.0 == pc2)).map(|l| l.id).unwrap_or(0);
    let _ = e.apply(Action::Impair { link: l, kbps: None, delay_ns: None, jitter_ns: None, loss_ppm: Some(1_000_000) });
    let t0 = e.now();
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts { count: 2, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("timeouts", ping_ev(&e, id), vec![PingEv::Timeout { seq: 1 }, PingEv::Timeout { seq: 2 }]);
    t.eq("timed out at", e.now(), t0 + 3 * SEC);
    let _ = r1;
}

fn proxy_arp_broadcast(t: &mut T) {
    // PC1 with a /8 and no default route asks for 10.0.2.10 itself; IOS answers for it
    let (mut e, pc1, r1, _pc2, _) = routed();
    let ok = [
        e.apply(Action::Route { dev: pc1, prefix: pf("0.0.0.0/0"), via: Some(ip("10.0.1.1")), iface: None, ad: 1, add: false }),
        e.apply(Action::IfAddr { dev: pc1, iface: String::from("eth0"), prefix: pf("10.0.1.10/24"), add: false }),
        addr(&mut e, pc1, "eth0", "10.0.1.10/8"),
    ];
    t.check("wrong mask set-up", ok.iter().all(|r| r.is_ok()));
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts { count: 2, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("proxy arp", ping_ev(&e, id), vec![reply(2, "10.0.2.10", 84, 63, 4104), PingEv::Timeout { seq: 1 }]);
    t.check("answered for PC2", notes(&e, r1).contains(&Note::ArpProxied { iface: 0, ip: ip("10.0.2.10") }));
    t.check("the router's MAC for PC2", e.arp_table(pc1).iter().any(|r| r.ip == ip("10.0.2.10") && r.mac == Some(port_mac(r1, 0))));
    t.check("proxy arp shown", e.ifaces(r1)[0].proxy_arp && !e.ifaces(pc1)[1].proxy_arp);
    // "no ip proxy-arp": nobody answers
    let ok = [e.apply(Action::ProxyArp { dev: r1, iface: String::from("gi0/0/0"), on: false }), e.apply(Action::ClearArp { dev: pc1 })];
    t.check("no ip proxy-arp", ok.iter().all(|r| r.is_ok()));
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts { count: 1, timeout: 5 * SEC, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("without proxy arp", ping_ev(&e, id), vec![error(1, "10.0.1.10", IcmpKind::HostUnreachable)]);
    // a directed broadcast is not forwarded into its subnet
    let (mut e, pc1, r1, pc2, _) = routed();
    let id = ping(&mut e, pc1, "10.0.2.255", once());
    e.run_idle(BUDGET);
    t.eq("directed broadcast", ping_ev(&e, id), vec![PingEv::Timeout { seq: 1 }]);
    t.check("not forwarded", notes(&e, r1).contains(&Note::IpDirectedBroadcast { dst: ip("10.0.2.255") }) && !e.trace.hops().any(|h| h.to.0 == pc2));
}

fn fragments_mtu(t: &mut T) {
    let (mut e, pc1, r1, pc2, _) = routed();
    // warm the ARP caches (the router drops what needs an answer)
    let _ = ping(&mut e, pc1, "10.0.2.10", once());
    e.run_idle(BUDGET);
    // 3000 data bytes: 3028 bytes of IPv4 in three fragments each way, reassembled at both ends
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts { count: 1, size: 3000, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.check("big ping answered", matches!(ping_ev(&e, id).as_slice(), [PingEv::Reply { seq: 1, bytes: 3028, ttl: 63, .. }]));
    t.check("fragmented", notes(&e, pc1).contains(&Note::IpFragmented { parts: 3, mtu: 1500 }) && notes(&e, pc2).contains(&Note::IpFragmented { parts: 3, mtu: 1500 }));
    t.check("reassembled", notes(&e, pc2).contains(&Note::IpReassembled { bytes: 3028 }) && notes(&e, pc1).contains(&Note::IpReassembled { bytes: 3028 }));
    let sizes: Vec<usize> = e.trace.hops().filter(|h| h.from.0 == r1 && h.to.0 == pc2 && h.mask & decode::P_IPV4 != 0).map(|h| h.bytes.len()).collect();
    t.eq("the router forwards fragments", sizes, vec![1514, 1514, 82]);
    // a smaller MTU on the way and "don't fragment": the router says how much fits
    t.check("mtu", e.apply(Action::Mtu { dev: r1, iface: String::from("gi0/0/1"), mtu: 1400 }).is_ok());
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts { count: 1, size: 1472, df: true, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("fragmentation needed", ping_ev(&e, id), vec![error(1, "10.0.1.1", IcmpKind::FragNeeded(1400))]);
    t.check("noted", notes(&e, r1).contains(&Note::IpFragNeeded { mtu: 1400 }));
    // without DF the router fragments
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts { count: 1, size: 1472, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.check("fragmented by the router", matches!(ping_ev(&e, id).as_slice(), [PingEv::Reply { bytes: 1500, .. }]) && notes(&e, r1).contains(&Note::IpFragmented { parts: 2, mtu: 1400 }));
    // too big for the own interface: a local error and nothing on the cable
    let before = e.trace.next_hop_id();
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts { count: 1, size: 1473, df: true, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("local mtu", ping_ev(&e, id), vec![error(1, "10.0.1.10", IcmpKind::FragNeeded(1500))]);
    t.eq("nothing sent", e.trace.hops().filter(|h| h.id >= before).count(), 0);
}

fn l3_switch(t: &mut T) {
    let mut p = Project::new("mls", 13);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 200, 0).unwrap_or(0);
    let mls = p.add_device("jf-s24l3", 100, 100).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(mls, "gi1/0/1"), None);
    let _ = p.connect(&pr(pc2, "eth0"), &pr(mls, "gi1/0/2"), None);
    let mut e = engine(&p);
    let ok = [
        e.apply(Action::Access { dev: mls, port: String::from("gi1/0/1"), vlan: 10 }),
        e.apply(Action::Access { dev: mls, port: String::from("gi1/0/2"), vlan: 20 }),
        e.apply(Action::Svi { dev: mls, vlan: 10, add: true }),
        e.apply(Action::Svi { dev: mls, vlan: 20, add: true }),
        addr(&mut e, mls, "Vlan10", "10.0.10.1/24"),
        addr(&mut e, mls, "vlan20", "10.0.20.1/24"),
        addr(&mut e, pc1, "eth0", "10.0.10.10/24"),
        gw(&mut e, pc1, "10.0.10.1"),
        addr(&mut e, pc2, "eth0", "10.0.20.10/24"),
        gw(&mut e, pc2, "10.0.20.1"),
    ];
    t.check("l3 switch set-up", ok.iter().all(|r| r.is_ok()));
    // a new SVI is shut down
    t.check("svi starts down", e.ifaces(mls).iter().any(|f| f.name == "Vlan10" && !f.admin_up && !f.up));
    let _ = if_up(&mut e, mls, "Vlan10", true);
    let _ = if_up(&mut e, mls, "Vl20", true);
    t.check("svis up", e.ifaces(mls).iter().filter(|f| f.name.starts_with("Vlan") && f.up).count() == 2);
    // without "ip routing" the switch is a host: its own addresses answer, nothing is routed
    let id = ping(&mut e, pc1, "10.0.20.1", once());
    e.run_idle(BUDGET);
    t.check("own address in the other vlan", matches!(ping_ev(&e, id).as_slice(), [PingEv::Reply { ttl: 255, .. }]));
    let id = ping(&mut e, pc1, "10.0.20.10", once());
    e.run_idle(BUDGET);
    t.eq("no routing", ping_ev(&e, id), vec![PingEv::Timeout { seq: 1 }]);
    t.check("not routed", notes(&e, mls).contains(&Note::IpNotForMe { dst: ip("10.0.20.10") }));
    // "ip routing": the switch routes between the VLANs (seq 1 goes while it asks for PC2)
    t.check("ip routing", e.apply(Action::Forwarding { dev: mls, on: true }).is_ok());
    let id = ping(&mut e, pc1, "10.0.20.10", PingOpts { count: 2, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("inter-vlan", ping_ev(&e, id), vec![reply(2, "10.0.20.10", 84, 63, 4104), PingEv::Timeout { seq: 1 }]);
    t.check("frames stay untagged on access ports", e.trace.hops().all(|h| netpkt::eth::tag_of(&h.bytes).is_none()));
    // the SVI goes: its network with it
    t.check("no interface vlan20", e.apply(Action::Svi { dev: mls, vlan: 20, add: false }).is_ok());
    t.check("svi gone", !e.ifaces(mls).iter().any(|f| f.name == "Vlan20"));
    let id = ping(&mut e, pc1, "10.0.20.10", once());
    e.run_idle(BUDGET);
    t.eq("network gone", ping_ev(&e, id), vec![error(1, "10.0.10.1", IcmpKind::NetUnreachable)]);
    // a routed port ("no switchport") to a router
    let mut p2 = p.clone();
    let r1 = p2.add_device("jf-r4", 300, 100).unwrap_or(0);
    let _ = p2.connect(&pr(mls, "gi1/0/24"), &pr(r1, "gi0/0/0"), None);
    let mut e2 = engine(&p2);
    let ok = [
        e2.apply(Action::RoutedPort { dev: mls, port: String::from("gi1/0/24"), routed: true }),
        addr(&mut e2, mls, "Gi1/0/24", "192.168.99.1/30"),
        addr(&mut e2, r1, "gi0/0/0", "192.168.99.2/30"),
        if_up(&mut e2, r1, "gi0/0/0", true),
    ];
    t.check("routed port set-up", ok.iter().all(|r| r.is_ok()));
    let id = ping(&mut e2, mls, "192.168.99.2", PingOpts { count: 2, ..PingOpts::default() });
    e2.run_idle(BUDGET);
    t.eq("routed port", ping_ev(&e2, id), vec![reply(2, "192.168.99.2", 84, 255, 2052), PingEv::Timeout { seq: 1 }]);
    t.check("not a bridge port any more", e2.bridge(mls).is_some_and(|b| !b.ports[23].member));
    t.check("switchport again", e2.apply(Action::RoutedPort { dev: mls, port: String::from("gi1/0/24"), routed: false }).is_ok());
    t.check("interface gone", !e2.ifaces(mls).iter().any(|f| f.name == "GigabitEthernet1/0/24") && e2.bridge(mls).is_some_and(|b| b.ports[23].member));
}

fn router_on_a_stick(t: &mut T) {
    // one router port as a trunk to the switch: its 802.1Q subinterfaces route between VLANs
    let mut p = Project::new("stick", 18);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 200, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 100).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 100, 200).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(pc2, "eth0"), &pr(sw, "gi1/0/2"), None);
    let trunk = p.connect(&pr(sw, "gi1/0/24"), &pr(r1, "gi0/0/0"), None).unwrap_or(0);
    let mut e = engine(&p);
    let ok = [
        e.apply(Action::Access { dev: sw, port: String::from("gi1/0/1"), vlan: 10 }),
        e.apply(Action::Access { dev: sw, port: String::from("gi1/0/2"), vlan: 20 }),
        e.apply(Action::Trunk { dev: sw, port: String::from("gi1/0/24"), native: 1, allowed: VlanSet::all() }),
        if_up(&mut e, r1, "gi0/0/0", true),
        e.apply(Action::SubIf { dev: r1, name: String::from("gi0/0/0.10"), vlan: 10, add: true }),
        e.apply(Action::SubIf { dev: r1, name: String::from("Gi0/0/0.20"), vlan: 20, add: true }),
        addr(&mut e, r1, "Gi0/0/0.10", "10.0.10.1/24"),
        addr(&mut e, r1, "GigabitEthernet0/0/0.20", "10.0.20.1/24"),
        addr(&mut e, pc1, "eth0", "10.0.10.10/24"),
        gw(&mut e, pc1, "10.0.10.1"),
        addr(&mut e, pc2, "eth0", "10.0.20.10/24"),
        gw(&mut e, pc2, "10.0.20.1"),
    ];
    t.check("router on a stick set-up", ok.iter().all(|r| r.is_ok()));
    t.eq("subinterfaces", e.ifaces(r1).iter().filter(|f| f.name.contains('.')).map(|f| f.name.as_str()).collect::<Vec<_>>().join(" "), String::from("GigabitEthernet0/0/0.10 GigabitEthernet0/0/0.20"));
    let id = ping(&mut e, pc1, "10.0.20.10", PingOpts { count: 2, ..PingOpts::default() });
    e.run_idle(BUDGET);
    // seq 2: four cables each way, the two trunk ones with a 4-byte tag (102 bytes: 1058 ns)
    t.eq("between the vlans", ping_ev(&e, id), vec![reply(2, "10.0.20.10", 84, 63, 8336), PingEv::Timeout { seq: 1 }]);
    let tags: Vec<Option<u16>> = e.trace.hops().filter(|h| h.link == trunk && h.mask & decode::P_ICMP != 0 && h.t_tx > SEC).map(|h| netpkt::eth::tag_of(&h.bytes).map(|g| g.vid)).collect();
    t.eq("tags on the trunk", tags, vec![Some(10), Some(20), Some(20), Some(10)]);
    // a VLAN without a subinterface
    let f = netpkt::eth::tagged(&EthHdr { dst: Mac::BROADCAST, src: port_mac(sw, 23), ethertype: ET_ARP }.frame(&[0u8; 46]), VlanTag::new(30));
    t.check("inject", e.apply(Action::Inject { dev: sw, port: String::from("gi1/0/24"), frame: f }).is_ok());
    e.run_idle(BUDGET);
    t.check("unknown vlan dropped", notes(&e, r1).contains(&Note::DropVlan { port: 0, vid: 30 }));
    t.check("one vlan per subinterface", e.apply(Action::SubIf { dev: r1, name: String::from("gi0/0/0.30"), vlan: 10, add: true }).is_err());
    t.check("no subinterface on a switch port", matches!(e.apply(Action::SubIf { dev: sw, name: String::from("gi1/0/1.10"), vlan: 10, add: true }), Err(ActErr::Refused(_))));
    t.check("not a subinterface name", e.apply(Action::SubIf { dev: r1, name: String::from("gi0/0/1"), vlan: 10, add: true }).is_err());
    // the main interface shut down takes them along
    let _ = if_up(&mut e, r1, "gi0/0/0", false);
    t.check("subinterfaces down with the port", e.ifaces(r1).iter().filter(|f| f.name.contains('.')).all(|f| f.admin_up && !f.up));
    t.check("remove", e.apply(Action::SubIf { dev: r1, name: String::from("gi0/0/0.20"), vlan: 0, add: false }).is_ok() && e.ifaces(r1).iter().filter(|f| f.name.contains('.')).count() == 1);
}

fn management_svi(t: &mut T) {
    let (mut e, pc1, pc2, sw, _) = lan2();
    // Vlan1 of an IOS switch is shut down: nobody answers for its address
    t.check("vlan1 address", addr(&mut e, sw, "Vlan1", "10.0.0.254/24").is_ok());
    let id = ping(&mut e, pc1, "10.0.0.254", PingOpts { count: 1, timeout: 5 * SEC, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("vlan1 down", ping_ev(&e, id), vec![error(1, "10.0.0.1", IcmpKind::HostUnreachable)]);
    t.check("no shutdown", if_up(&mut e, sw, "vlan1", true).is_ok());
    // one cable each way: 2 x 722 ns of ARP for seq 1, then 2 x 1026; IOS answers with TTL 255
    let id = ping(&mut e, pc1, "10.0.0.254", PingOpts { count: 2, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("switch answers", ping_ev(&e, id), vec![reply(1, "10.0.0.254", 84, 255, 3496), reply(2, "10.0.0.254", 84, 255, 2052)]);
    // and pings from itself: ".!" (its first probe goes while it asks for PC2)
    let id = ping(&mut e, sw, "10.0.0.2", PingOpts { count: 2, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("switch pings", ping_ev(&e, id), vec![reply(2, "10.0.0.2", 84, 64, 2052), PingEv::Timeout { seq: 1 }]);
    t.check("pc2 learned the switch", e.arp_table(pc2).iter().any(|r| r.ip == ip("10.0.0.254") && r.mac == Some(jfn_sim::node::base_mac(sw))));
}

/// PC1 (192.168.1.10) - lan1 home router (br0 192.168.1.1, wan 10.0.2.1) - PC2 (10.0.2.10):
/// (engine, pc1, home, pc2, set-up ok)
fn home() -> (Engine, u32, u32, u32, bool) {
    let mut p = Project::new("home", 17);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let home = p.add_device("home-router", 100, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(home, "lan1"), None);
    let _ = p.connect(&pr(home, "wan"), &pr(pc2, "eth0"), None);
    let mut e = engine(&p);
    let ok = [
        addr(&mut e, home, "br0", "192.168.1.1/24"),
        addr(&mut e, home, "wan", "10.0.2.1/24"),
        addr(&mut e, pc1, "eth0", "192.168.1.10/24"),
        gw(&mut e, pc1, "192.168.1.1"),
        addr(&mut e, pc2, "eth0", "10.0.2.10/24"),
        gw(&mut e, pc2, "10.0.2.1"),
    ];
    (e, pc1, home, pc2, ok.iter().all(|r| r.is_ok()))
}

fn linux_router(t: &mut T) {
    // a home router: LAN ports bridged (br0), WAN routed; Linux keeps packets while ARP asks
    // and says "host unreachable" when nobody answers
    let (mut e, pc1, home, _pc2, ok) = home();
    t.check("home router set-up", ok);
    t.eq("home router interfaces", e.ifaces(home).iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(" "), String::from("lo wan br0"));
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts { count: 2, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("through the home router", ping_ev(&e, id), vec![reply(1, "10.0.2.10", 84, 63, 6992), reply(2, "10.0.2.10", 84, 63, 4104)]);
    t.check("it waited for ARP", notes(&e, home).iter().any(|n| matches!(n, Note::ArpRequest { .. })) && !notes(&e, home).iter().any(|n| matches!(n, Note::ArpIncomplete { .. })));
    let id = ping(&mut e, pc1, "10.0.2.99", PingOpts { count: 1, timeout: 5 * SEC, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("host unreachable from linux", ping_ev(&e, id), vec![error(1, "192.168.1.1", IcmpKind::HostUnreachable)]);
}

fn icmp_rates(t: &mut T) {
    // IOS sends an unreachable every 500 ms at most: probes 300 ms apart give "U.U"
    let (mut e, pc1, r1, _pc2, _) = routed();
    let id = ping(&mut e, pc1, "192.0.2.1", PingOpts { count: 3, interval: 300 * MS, timeout: SEC, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("ios unreachables", ping_ev(&e, id), vec![error(1, "10.0.1.1", IcmpKind::NetUnreachable), error(3, "10.0.1.1", IcmpKind::NetUnreachable), PingEv::Timeout { seq: 2 }]);
    t.eq("ios suppressed one", notes(&e, r1).iter().filter(|n| **n == Note::IcmpLimited).count(), 1);
    // ... but no limit on time exceeded
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts { count: 4, ttl: Some(1), interval: 10 * MS, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("ios time exceeded", ping_ev(&e, id).iter().filter(|x| matches!(x, PingEv::Error { kind: IcmpKind::TtlExceeded, .. })).count(), 4);
    // Linux: six to one destination at once, then one a second
    let (mut e, pc1, home, _pc2, _) = home();
    let id = ping(&mut e, pc1, "203.0.113.1", PingOpts { count: 8, interval: 10 * MS, timeout: SEC, ..PingOpts::default() });
    e.run_idle(BUDGET);
    let ev = ping_ev(&e, id);
    let want: Vec<PingEv> = (1..=6).map(|s| error(s, "192.168.1.1", IcmpKind::NetUnreachable)).chain([PingEv::Timeout { seq: 7 }, PingEv::Timeout { seq: 8 }]).collect();
    t.eq("linux per destination", ev, want);
    t.eq("linux suppressed two", notes(&e, home).iter().filter(|n| **n == Note::IcmpLimited).count(), 2);
    // a second later there is credit for one more
    let id = ping(&mut e, pc1, "203.0.113.1", once());
    e.run_idle(BUDGET);
    t.check("credit again", matches!(ping_ev(&e, id).as_slice(), [PingEv::Error { kind: IcmpKind::NetUnreachable, .. }]));
}

fn serial(t: &mut T) {
    let mut p = Project::new("wan", 14);
    let r1 = p.add_device("jf-r4", 0, 0).unwrap_or(0);
    let r2 = p.add_device("jf-r4", 200, 0).unwrap_or(0);
    t.check("serial cable", p.connect(&pr(r1, "se0/2/0"), &pr(r2, "Serial0/2/0"), None).is_ok());
    let mut e = engine(&p);
    let ok = [addr(&mut e, r1, "Se0/2/0", "10.0.12.1/30"), addr(&mut e, r2, "s0/2/0", "10.0.12.2/30"), if_up(&mut e, r1, "se0/2/0", true), if_up(&mut e, r2, "Serial0/2/0", true)];
    t.check("serial set-up", ok.iter().all(|r| r.is_ok()));
    // 100-byte datagrams in 104 bytes of HDLC: 406250 ns at 2048 kbit/s plus 2 ms of line
    // each way; no ARP on a serial line
    let id = ping(&mut e, r1, "10.0.12.2", PingOpts { count: 2, size: 72, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("serial ping", ping_ev(&e, id), vec![reply(1, "10.0.12.2", 100, 255, 4_812_500), reply(2, "10.0.12.2", 100, 255, 4_812_500)]);
    t.check("hdlc framing", e.trace.hops().count() == 4 && e.trace.hops().all(|h| h.bytes.len() == 104 && h.bytes[..4] == [0x0F, 0, 8, 0]));
    t.check("decoded as ICMP", e.trace.hops().all(|h| h.mask & decode::P_ICMP != 0 && h.mask & decode::P_ARP == 0));
    // the far end shut down: the line goes down, and with it the network
    let _ = if_up(&mut e, r2, "se0/2/0", false);
    let id = ping(&mut e, r1, "10.0.12.2", once());
    e.run_idle(BUDGET);
    t.eq("line down", ping_ev(&e, id), vec![error(1, "0.0.0.0", IcmpKind::NoRoute)]);
}

fn loopbacks(t: &mut T) {
    let (mut e, pc1, pc2, _sw, _) = lan2();
    // Linux: 127.0.0.0/8 is the device itself, nothing goes on a cable
    let id = ping(&mut e, pc1, "127.0.0.1", once());
    let id5 = ping(&mut e, pc1, "127.0.0.5", once());
    e.run_idle(BUDGET);
    t.eq("localhost", ping_ev(&e, id), vec![reply(1, "127.0.0.1", 84, 64, 0)]);
    t.eq("all of 127/8", ping_ev(&e, id5), vec![reply(1, "127.0.0.5", 84, 64, 0)]);
    t.eq("nothing on the cable", e.trace.hops().count(), 0);
    // a packet for 127.0.0.1 from the cable is a martian
    let mut pk = Pkt::new(HEADROOM, &[0u8; 8]);
    Icmp::EchoRequest { id: 7, seq: 1 }.push_onto(&mut pk);
    Ipv4Hdr::new(ip("10.0.0.1"), Ipv4::LOCALHOST, PROTO_ICMP, 64).push_onto(&mut pk);
    let f = EthHdr { dst: port_mac(pc2, 0), src: port_mac(pc1, 0), ethertype: ET_IPV4 }.frame(&pk.into_vec());
    t.check("inject", e.apply(Action::Inject { dev: pc1, port: String::from("eth0"), frame: f }).is_ok());
    e.run_idle(BUDGET);
    t.check("martian", notes(&e, pc2).contains(&Note::IpMartian { src: ip("10.0.0.1"), dst: Ipv4::LOCALHOST }));
    t.check("not answered", !notes(&e, pc2).iter().any(|n| matches!(n, Note::EchoAnswered { .. })));
    // an IOS loopback answers from afar (weak host model)
    let (mut e, pc1, r1, _pc2, _) = routed();
    t.check("loopback0", e.apply(Action::Loopback { dev: r1, n: 0, add: true }).is_ok() && addr(&mut e, r1, "lo0", "192.0.2.1/32").is_ok());
    let id = ping(&mut e, pc1, "192.0.2.1", PingOpts { count: 2, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("loopback answers", ping_ev(&e, id), vec![reply(1, "192.0.2.1", 84, 255, 3496), reply(2, "192.0.2.1", 84, 255, 2052)]);
    t.check("loopback route", e.routes(r1).iter().any(|r| r.prefix == pf("192.0.2.1/32") && r.src == RouteSrc::Connected));
    t.check("no loopback1", e.apply(Action::Loopback { dev: r1, n: 1, add: false }).is_err());
    t.check("remove loopback0", e.apply(Action::Loopback { dev: r1, n: 0, add: false }).is_ok() && !e.ifaces(r1).iter().any(|f| f.name == "Loopback0"));
}

fn refused(t: &mut T) {
    let (mut e, pc1, r1, _pc2, _) = routed();
    t.eq("unknown interface", addr(&mut e, r1, "Gi9/9/9", "10.9.9.1/24"), Err(ActErr::UnknownIface { dev: r1, iface: String::from("Gi9/9/9") }));
    t.eq("overlap on IOS", addr(&mut e, r1, "Gi0/0/2", "10.0.1.5/24").map_err(|x| format!("{}", x)), Err(String::from("10.0.1.0/24 überschneidet sich mit GigabitEthernet0/0/0.")));
    t.check("network address", addr(&mut e, r1, "Gi0/0/2", "10.0.5.0/24").is_err());
    t.check("broadcast address", addr(&mut e, r1, "Gi0/0/2", "10.0.5.255/24").is_err());
    t.check("loopback address on a port", addr(&mut e, pc1, "eth0", "127.0.0.2/8").is_err());
    t.check("class E", addr(&mut e, pc1, "eth0", "240.0.0.1/24").is_err());
    t.check("overlap allowed on Linux", e.apply(Action::Loopback { dev: pc1, n: 0, add: true }).is_ok() && addr(&mut e, pc1, "dummy0", "10.0.1.200/24").is_ok());
    t.eq("mtu too small", e.apply(Action::Mtu { dev: r1, iface: String::from("gi0/0/0"), mtu: 60 }), Err(ActErr::BadMtu(60)));
    t.check("route needs a target", e.apply(Action::Route { dev: r1, prefix: pf("10.9.0.0/16"), via: None, iface: None, ad: 1, add: true }).is_err());
    t.check("own address as next hop", e.apply(Action::Route { dev: r1, prefix: pf("10.9.0.0/16"), via: Some(ip("10.0.1.1")), iface: None, ad: 1, add: true }).is_err());
    t.check("no such route", e.apply(Action::Route { dev: r1, prefix: pf("10.9.0.0/16"), via: None, iface: None, ad: 1, add: false }).is_err());
    t.check("svi on a router", e.apply(Action::Svi { dev: r1, vlan: 10, add: true }) == Err(ActErr::NoBridge(r1)));
    t.eq("error text", format!("{}", ActErr::UnknownIface { dev: 3, iface: String::from("Gi0/9") }), String::from("Gerät 3 hat kein Interface \u{201e}Gi0/9\u{201c}."));
    let mut p = Project::new("hub", 15);
    let hub = p.add_device("hub", 0, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 0).unwrap_or(0);
    let mut e2 = engine(&p);
    t.eq("no ip on a hub", e2.ping(hub, ip("10.0.0.1"), once()), Err(ActErr::NoIp(hub)));
    t.check("a layer 2 switch routes no port", matches!(e2.apply(Action::RoutedPort { dev: sw, port: String::from("gi1/0/1"), routed: true }), Err(ActErr::Refused(_))));
    // a static ARP entry: no question needed (the router still asks for the PC)
    t.check("static arp", e.apply(Action::StaticArp { dev: pc1, ip: ip("10.0.1.1"), mac: Some(port_mac(r1, 0)) }).is_ok());
    let id = ping(&mut e, pc1, "10.0.1.1", PingOpts { count: 2, ..PingOpts::default() });
    e.run_idle(BUDGET);
    t.eq("static arp ping", ping_ev(&e, id), vec![reply(2, "10.0.1.1", 84, 255, 2052), PingEv::Timeout { seq: 1 }]);
    t.check("pc1 asked nothing", !notes(&e, pc1).iter().any(|n| matches!(n, Note::ArpRequest { .. })));
    t.check("clear arp keeps static", e.apply(Action::ClearArp { dev: pc1 }).is_ok() && e.arp_table(pc1).iter().map(|r| (r.ip, r.fixed)).collect::<Vec<_>>() == vec![(ip("10.0.1.1"), true)]);
    t.check("remove static", e.apply(Action::StaticArp { dev: pc1, ip: ip("10.0.1.1"), mac: None }).is_ok() && e.arp_table(pc1).is_empty());
    t.check("static arp off-net", e.apply(Action::StaticArp { dev: pc1, ip: ip("192.0.2.9"), mac: Some(port_mac(r1, 0)) }).is_err());
}

fn views(t: &mut T) {
    let (mut e, pc1, r1, _pc2, _) = routed();
    let _ = e.apply(Action::Route { dev: r1, prefix: pf("172.16.0.0/16"), via: Some(ip("10.0.2.10")), iface: None, ad: 1, add: true });
    let _ = e.apply(Action::Route { dev: r1, prefix: pf("172.17.0.0/16"), via: Some(ip("172.16.0.1")), iface: None, ad: 1, add: true });
    let table = |e: &Engine| e.routes(r1).iter().map(|r| (format!("{}", r.prefix), r.src.code(), r.via, r.iface)).collect::<Vec<_>>();
    let want = vec![
        (String::from("10.0.1.0/24"), "C", None, Some(0)),
        (String::from("10.0.1.1/32"), "L", None, Some(0)),
        (String::from("10.0.2.0/24"), "C", None, Some(1)),
        (String::from("10.0.2.1/32"), "L", None, Some(1)),
        (String::from("172.16.0.0/16"), "S", Some(ip("10.0.2.10")), Some(1)),
        (String::from("172.17.0.0/16"), "S", Some(ip("172.16.0.1")), Some(1)),
    ];
    t.eq("routing table", table(&e), want);
    // a recursive static route resolves to the neighbour of its next hop
    t.eq("recursive next hop", e.ping(r1, ip("172.17.5.5"), once()).ok().and(e.trace.events().find(|x| x.dev == r1 && matches!(x.note, Note::ArpRequest { .. })).map(|x| x.note.clone())), Some(Note::ArpRequest { iface: 1, ip: ip("10.0.2.10") }));
    let _ = if_up(&mut e, r1, "gi0/0/1", false);
    t.eq("routes over a shut interface go", table(&e).len(), 2);
    let ifs: Vec<(String, Vec<Prefix4>, bool)> = e.ifaces(pc1).into_iter().map(|f| (f.name, f.addrs, f.up)).collect();
    t.eq("pc interfaces", ifs, vec![(String::from("lo"), vec![pf("127.0.0.1/8")], true), (String::from("eth0"), vec![pf("10.0.1.10/24")], true)]);
    let rif = e.ifaces(r1);
    t.eq("router interfaces", rif.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(" "), String::from("GigabitEthernet0/0/0 GigabitEthernet0/0/1 GigabitEthernet0/0/2 GigabitEthernet0/0/3 TenGigabitEthernet0/1/0 TenGigabitEthernet0/1/1 Serial0/2/0 Serial0/2/1"));
    t.check("router port state", rif[0].admin_up && rif[0].up && !rif[1].admin_up && !rif[1].up && !rif[2].admin_up);
    t.check("in the view", e.view().devices.iter().find(|d| d.dev == r1).is_some_and(|d| d.ifaces == rif));
    t.check("a hub has none", {
        let mut p = Project::new("hub", 16);
        let hub = p.add_device("hub", 0, 0).unwrap_or(0);
        engine(&p).ifaces(hub).is_empty()
    });
}

fn jobs_end(t: &mut T) {
    let (mut e, pc1, _r1, _pc2, _) = routed();
    // count 0 runs until cancelled
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts { count: 0, ..PingOpts::default() });
    e.run_until(3 * SEC + 500 * MS, BUDGET);
    t.check("cancel", e.cancel_job(id));
    let hops = e.trace.hops().count();
    e.run_until(10 * SEC, BUDGET);
    t.eq("nothing after cancel", e.trace.hops().count(), hops);
    t.eq("sent until cancel", sent(&e, id), 4);
    t.check("cancel once", !e.cancel_job(id));
    // switching the device off ends its jobs and empties its ARP cache
    let id = ping(&mut e, pc1, "10.0.2.10", PingOpts { count: 0, ..PingOpts::default() });
    e.run_until(e.now() + 1500 * MS, BUDGET);
    t.check("arp cache filled", !e.arp_table(pc1).is_empty());
    t.check("power off", e.apply(Action::Power { dev: pc1, on: false }).is_ok());
    t.check("job ended", done(&e, id));
    t.check("arp cache forgotten", e.arp_table(pc1).is_empty());
    let jobs = e.jobs().count();
    t.eq("jobs kept for the record", jobs, 2);
    // removing a device ends them too
    let (mut e, pc1, r1, _pc2, _) = routed();
    let id = ping(&mut e, r1, "10.0.1.10", PingOpts { count: 0, ..PingOpts::default() });
    e.run_until(SEC + 500 * MS, BUDGET);
    t.check("remove", e.apply(Action::RemoveDevice(r1)).is_ok() && done(&e, id));
    e.run_idle(BUDGET);
    t.check("pc lost its link", e.ifaces(pc1).iter().any(|f| f.name == "eth0" && !f.up));
}

fn determinism(t: &mut T) {
    let run = || {
        let (mut e, pc1, _r1, _pc2, _) = routed();
        let _ = e.ping(pc1, ip("10.0.2.10"), PingOpts { count: 3, interval: 0, ..PingOpts::default() });
        let _ = e.traceroute(pc1, ip("10.0.2.10"), TraceOpts::default());
        e.run_idle(BUDGET);
        e.trace.hops().map(|h| (h.t_tx, h.t_rx, h.from, h.to, h.bytes.to_vec())).collect::<Vec<_>>()
    };
    let a = run();
    t.check("ping and traceroute side by side", a.len() > 20);
    t.check("two runs alike", a == run());
}
