// firewalltest -- checks of the zone firewall (part of --selftest, group "firewall"): every default of
// the zone rules; "inspect" with its return traffic and an unsolicited packet; each TCP state with its
// timeout; UDP and ICMP timers; the half-open limit; NAT and access lists together with zones in both
// directions; the self zone; drop and reject; IPv6; a configuration round trip through running and
// startup configuration; a power cycle; determinism over two runs, a clone in mid-run and a replay
// from keyframes; the prediction of Firewall::probe against packets sent for real; the problems; a
// device without zones
//
// The network (device ids in order): PC (192.168.1.10) on Gi0/0 of FW, zone INSIDE; WEB (10.0.0.10,
// TCP echo on port 80) on Gi0/1, zone DMZ; EXT (203.0.113.10, TCP echo on 80) on Gi0/2, zone OUTSIDE;
// LAB (172.16.0.10) on Gi0/3 without a zone.  All but LAB speak IPv6 as well.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::fw::{FwFlow, FwProbe, Match, TcpState, FwAction};
use jfn_sim::fwtrace::FwNote;
use jfn_sim::keyframe::digest;
use jfn_sim::{Budget, Cmd, Engine, Inline, JobKind, Note, PduProto, PduSender, PduSpec, PingEv, PingOpts, ProblemKind, Report, MS, SEC};
use netpkt::ipv4::{PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use netpkt::nat::Flow;
use netpkt::tcp::{ACK, SYN};
use netpkt::Ipv4;

use crate::doc::console;
use crate::selftest::T;

const BUDGET: usize = 4_000_000;
const PC: u32 = 1;
const WEB: u32 = 2;
const EXT: u32 = 3;
const LAB: u32 = 4;
const FW: u32 = 5;

const IFS: &str = "interface GigabitEthernet0/0\n ip address 192.168.1.1 255.255.255.0\n ipv6 address 2001:DB8:1::1/64\n no shutdown\n!\ninterface GigabitEthernet0/1\n ip address 10.0.0.1 255.255.255.0\n ipv6 address 2001:DB8:2::1/64\n no shutdown\n!\ninterface GigabitEthernet0/2\n ip address 203.0.113.1 255.255.255.0\n ipv6 address 2001:DB8:3::1/64\n no shutdown\n!\ninterface GigabitEthernet0/3\n ip address 172.16.0.1 255.255.255.0\n no shutdown\n!\n";

// the zones and policies of most checks; INSIDE may go anywhere, the outside only to the web server
const ZONES: &str = "class-map type inspect match-any C-ALL\n match protocol tcp\n match protocol udp\n match protocol icmp\n!\nclass-map type inspect C-WEB\n match protocol http\n!\npolicy-map type inspect P-ALL\n class type inspect C-ALL\n  inspect\n!\npolicy-map type inspect P-WEB\n class type inspect C-WEB\n  inspect\n!\nzone security INSIDE\n!\nzone security DMZ\n!\nzone security OUTSIDE\n!\nzone-pair security IN-OUT source INSIDE destination OUTSIDE\n service-policy type inspect P-ALL\n!\nzone-pair security IN-DMZ source INSIDE destination DMZ\n service-policy type inspect P-ALL\n!\nzone-pair security OUT-DMZ source OUTSIDE destination DMZ\n service-policy type inspect P-WEB\n!\n";
const MEMBERS: &str = "interface GigabitEthernet0/0\n zone-member security INSIDE\n!\ninterface GigabitEthernet0/1\n zone-member security DMZ\n!\ninterface GigabitEthernet0/2\n zone-member security OUTSIDE\n!\n";

fn s(x: &str) -> String {
    String::from(x)
}

fn ip(x: &str) -> Ipv4 {
    Ipv4::parse(x).unwrap_or(Ipv4::UNSPECIFIED)
}

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: s(port) }
}

fn svc(kind: &str, name: &str, port: u64) -> ServiceDef {
    let config = agent_core::json::parse(format!("{{\"port\": {}}}", port).as_bytes()).unwrap_or(Value::Null);
    ServiceDef { kind: s(kind), name: s(name), disabled: false, config }
}

// the network with this firewall model and these lines at the end of its startup configuration
fn project(model: &str, cfg: &str) -> Project {
    let mut p = Project::new("zones", 11);
    for m in ["pc", "server-1u", "server-1u", "pc", model] {
        let _ = p.add_device(m, 0, 0);
    }
    let _ = p.connect(&pr(PC, "eth0"), &pr(FW, "gi0/0"), None);
    let _ = p.connect(&pr(WEB, "eth0"), &pr(FW, "gi0/1"), None);
    let _ = p.connect(&pr(EXT, "eth0"), &pr(FW, "gi0/2"), None);
    let _ = p.connect(&pr(LAB, "eth0"), &pr(FW, "gi0/3"), None);
    let host = |name: &str, a: &str, gw: &str, a6: Option<&str>| {
        let mut c = format!("hostname {}\nip addr add {} dev eth0\nip route add default via {}\n", name, a, gw);
        if let Some(x) = a6 {
            c.push_str(&format!("sysctl -w net.ipv6.conf.all.disable_ipv6=0\nsysctl -w net.ipv6.conf.eth0.accept_ra=0\nip -6 addr add {} dev eth0\n", x));
        }
        c
    };
    let cfgs = [
        (PC, "PC", host("PC", "192.168.1.10/24", "192.168.1.1", Some("2001:db8:1::10/64"))),
        (WEB, "WEB", host("WEB", "10.0.0.10/24", "10.0.0.1", Some("2001:db8:2::10/64"))),
        (EXT, "EXT", host("EXT", "203.0.113.10/24", "203.0.113.1", Some("2001:db8:3::10/64"))),
        (LAB, "LAB", host("LAB", "172.16.0.10/24", "172.16.0.1", None)),
        (FW, "FW", format!("hostname FW\n!\nipv6 unicast-routing\n!\n{}{}end\n", IFS, cfg)),
    ];
    for (id, name, c) in cfgs {
        if let Some(d) = p.device_mut(id) {
            d.name = s(name);
            d.startup_config = c;
        }
    }
    for dev in [WEB, EXT] {
        if let Some(d) = p.device_mut(dev) {
            d.services.push(svc("tcp-echo", "echo", 80));
        }
    }
    p
}

fn engine(cfg: &str) -> Engine {
    let mut e = Engine::from_project(&project("jf-fw", cfg)).unwrap_or_else(|_| Engine::new(0));
    run_for(&mut e, SEC);
    warm(&mut e);
    e
}

// IOS drops the packet that needs ARP: the device asks for its neighbours first
fn warm(e: &mut Engine) {
    for a in ["192.168.1.10", "10.0.0.10", "203.0.113.10", "172.16.0.10"] {
        let _ = e.ping(FW, ip(a), PingOpts { count: 1, ..PingOpts::default() });
    }
    run_for(e, 3 * SEC);
    let _ = e.take_reports();
}

fn zoned() -> Engine {
    engine(&format!("{}{}", ZONES, MEMBERS))
}

fn run_for(e: &mut Engine, span: u64) {
    let t = e.now() + span;
    e.run_until(t, BUDGET);
}

// what a device printed since the last call
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

// lines on the console of FW in EXEC mode: what it said
fn exec(e: &mut Engine, line: &str) -> String {
    let _ = out(e, FW);
    let _ = e.cli_line(FW, "enable");
    run_for(e, 10 * MS);
    let _ = out(e, FW);
    let _ = e.cli_line(FW, line);
    run_for(e, 10 * MS);
    out(e, FW)
}

// lines in the configuration mode of FW: what it said
fn conf(e: &mut Engine, ls: &[&str]) -> String {
    for l in ["enable", "configure terminal"] {
        let _ = e.cli_line(FW, l);
    }
    run_for(e, 10 * MS);
    let _ = out(e, FW);
    let mut o = String::new();
    for l in ls {
        let _ = e.cli_line(FW, l);
        run_for(e, 10 * MS);
        o.push_str(&out(e, FW));
    }
    let _ = e.cli_line(FW, "end");
    run_for(e, 10 * MS);
    let _ = out(e, FW);
    o
}

fn pdu(e: &mut Engine, dev: u32, proto: PduProto, dst: &str, sport: u16, dport: u16) -> String {
    let name = format!("pdu{}", e.now());
    let spec = PduSpec { proto, dst: s(dst), sport, dport, size: 20, ttl: None, count: 1, interval: MS };
    let _ = e.add_service(dev, &name, Box::new(PduSender::new(spec)), true);
    name
}

// did a TCP connection go through: no failure and something echoed
fn tcp_ok(e: &mut Engine, dev: u32, dst: &str, sport: u16, dport: u16) -> bool {
    let name = pdu(e, dev, PduProto::Tcp, dst, sport, dport);
    run_for(e, 2 * SEC);
    e.service::<PduSender>(dev, &name).is_some_and(|x| x.failed == 0 && x.replies >= 1)
}

// answers to `n` pings
fn pings(e: &mut Engine, dev: u32, dst: &str, n: u32) -> usize {
    let id = e.ping(dev, ip(dst), PingOpts { count: n, ..PingOpts::default() }).unwrap_or(0);
    run_for(e, (n as u64 + 3) * SEC);
    match e.job(id).map(|j| &j.kind) {
        Some(JobKind::Ping(p)) => p.events.iter().filter(|x| matches!(x, PingEv::Reply { .. })).count(),
        _ => 0,
    }
}

// the firewall's notes since event `at`
fn notes(e: &Engine, at: usize) -> Vec<FwNote> {
    e.trace.events().skip(at).filter_map(|x| if x.dev == FW { if let Note::Fw(n) = &x.note { Some(n.clone()) } else { None } } else { None }).collect()
}

fn mark(e: &Engine) -> usize {
    e.trace.events().count()
}

fn flow(proto: u8, src: &str, dst: &str, sport: u16, dport: u16, flags: u8) -> FwFlow {
    let ports = proto == PROTO_TCP || proto == PROTO_UDP;
    let f = Flow { proto, src: ip(src), dst: ip(dst), sport, dport, ports, icmp: None, flags, later: false, mf: false, id: 0 };
    FwFlow::of_v4(&f, 60, None)
}

pub fn run(t: &mut T) {
    defaults(t);
    return_traffic(t);
    tcp_states(t);
    timers(t);
    half_open(t);
    with_nat(t);
    with_acls(t);
    self_zone(t);
    drop_reject(t);
    ipv6(t);
    round_trip(t);
    power_cycle(t);
    determinism(t);
    prediction(t);
    forms(t);
    shows(t);
    problems(t);
    without_zones(t);
    host_firewall(t);
    window_steps(t);
}

// every default of the rules, read from the pure function and from packets
fn defaults(t: &mut T) {
    let mut e = engine(&format!("zone security A\n!\nzone security B\n!\nzone-pair security AB source A destination B\n!\n{}", "interface GigabitEthernet0/0\n zone-member security A\n!\ninterface GigabitEthernet0/1\n zone-member security A\n!\ninterface GigabitEthernet0/2\n zone-member security B\n!\n"));
    let fw = e.firewall(FW).cloned().unwrap_or_default();
    let acls = jfn_sim::acl::Acls::default();
    let f = flow(PROTO_UDP, "192.168.1.10", "10.0.0.10", 1000, 53, 0);
    t.eq("no zones on both sides: not its business", fw.probe(None, None, &f, &acls, 0), FwProbe::NoZones);
    t.eq("one zone: it passes", fw.probe(Some("A"), Some("A"), &f, &acls, 0), FwProbe::SameZone(s("A")));
    t.eq("a zone and none: dropped", fw.probe(Some("A"), None, &f, &acls, 0), FwProbe::Unzoned { zone: s("A") });
    t.eq("none and a zone: dropped", fw.probe(None, Some("B"), &f, &acls, 0), FwProbe::Unzoned { zone: s("B") });
    t.eq("a pair without a policy: dropped", fw.probe(Some("A"), Some("B"), &f, &acls, 0), FwProbe::NoRule { from: s("A"), to: s("B") });
    t.eq("no pair that way: dropped", fw.probe(Some("B"), Some("A"), &f, &acls, 0), FwProbe::NoRule { from: s("B"), to: s("A") });
    t.eq("to the device: it passes", fw.probe(Some("A"), Some("self"), &f, &acls, 0), FwProbe::SelfPass { from: s("A"), to: s("self") });
    t.eq("from the device: it passes", fw.probe(Some("self"), Some("B"), &f, &acls, 0), FwProbe::SelfPass { from: s("self"), to: s("B") });
    t.eq("no zone to the device: it passes", fw.probe(None, Some("self"), &f, &acls, 0), FwProbe::SelfPass { from: s("-"), to: s("self") });
    t.check("no rule does not pass", !fw.probe(Some("A"), Some("B"), &f, &acls, 0).passes());
    // with packets: PC and WEB in A, EXT in B, LAB in none
    t.eq("inside a zone a ping passes", pings(&mut e, PC, "10.0.0.10", 2), 2);
    let at = mark(&e);
    t.eq("across zones nothing comes back", pings(&mut e, PC, "203.0.113.10", 2), 0);
    t.check("the note says why", notes(&e, at).iter().any(|n| matches!(n, FwNote::NoRule { from, to, .. } if from == "A" && to == "B") && n.is_drop()));
    t.eq("from the unzoned one nothing arrives", pings(&mut e, LAB, "192.168.1.10", 2), 0);
    let at = mark(&e);
    t.eq("the device answers from a zone", pings(&mut e, PC, "192.168.1.1", 2), 2);
    t.check("through the self zone", notes(&e, at).iter().any(|n| matches!(n, FwNote::SelfPass { to, .. } if to == "self")));
    t.eq("the device answers without a zone", pings(&mut e, LAB, "172.16.0.1", 2), 2);
    t.check("the drops counted", e.firewall(FW).is_some_and(|f| f.stats.no_rule >= 4));
    // a pair for one direction
    let mut e = engine("zone security A\n!\ninterface GigabitEthernet0/0\n zone-member security A\n!\n");
    let at = mark(&e);
    t.eq("from a zone to no zone: dropped", pings(&mut e, PC, "172.16.0.10", 1), 0);
    t.check("as unzoned", notes(&e, at).iter().any(|n| matches!(n, FwNote::Unzoned { zone, .. } if zone == "A")));
    let mut e = engine("zone security UNUSED\n!\n");
    let at = mark(&e);
    t.eq("a zone nobody uses changes nothing", pings(&mut e, PC, "172.16.0.10", 2), 2);
    t.check("and no notes", notes(&e, at).is_empty());
    t.check("not active", e.firewall(FW).is_some_and(|f| !f.active()));
}

// "inspect": what began inside comes back; nothing begins outside
fn return_traffic(t: &mut T) {
    let mut e = zoned();
    let at = mark(&e);
    t.eq("ping from inside and back", pings(&mut e, PC, "203.0.113.10", 2), 2);
    let n = notes(&e, at);
    t.check("a session began", n.iter().any(|x| matches!(x, FwNote::NewSession { pair, class, .. } if pair == "IN-OUT" && class == "C-ALL")));
    t.check("the answers came by it", n.iter().any(|x| matches!(x, FwNote::Session { back: true, .. })));
    t.eq("ping from outside: nothing", pings(&mut e, EXT, "192.168.1.10", 2), 0);
    t.check("TCP from inside", tcp_ok(&mut e, PC, "203.0.113.10", 40000, 80));
    t.check("TCP from outside to inside is not allowed", !tcp_ok(&mut e, EXT, "192.168.1.10", 40001, 80));
    t.check("TCP from outside to the web server", tcp_ok(&mut e, EXT, "10.0.0.10", 40002, 80));
    t.check("UDP from outside to the web server is not", {
        let at = mark(&e);
        let _ = pdu(&mut e, EXT, PduProto::Udp, "10.0.0.10", 40003, 53);
        run_for(&mut e, SEC);
        notes(&e, at).iter().any(|x| matches!(x, FwNote::Drop { class, .. } if class == "class-default"))
    });
    t.check("UDP from inside comes back", {
        let at = mark(&e);
        let _ = pdu(&mut e, PC, PduProto::Udp, "203.0.113.10", 40004, 5000);
        run_for(&mut e, 200 * MS);
        let _ = pdu(&mut e, EXT, PduProto::Udp, "192.168.1.10", 5000, 40004);
        run_for(&mut e, 200 * MS);
        let n = notes(&e, at);
        n.iter().any(|x| matches!(x, FwNote::NewSession { .. })) && n.iter().any(|x| matches!(x, FwNote::Session { back: true, .. }))
    });
    // an unsolicited segment and an echo reply: nothing begins with them
    let fw = e.firewall(FW).cloned().unwrap_or_default();
    let acls = jfn_sim::acl::Acls::default();
    let ack = flow(PROTO_TCP, "192.168.1.10", "203.0.113.10", 41000, 80, ACK);
    t.check("a segment without a SYN begins nothing", matches!(fw.probe(Some("INSIDE"), Some("OUTSIDE"), &ack, &acls, e.now()), FwProbe::NotNew { .. }));
    let syn = flow(PROTO_TCP, "192.168.1.10", "203.0.113.10", 41000, 80, SYN);
    t.check("a SYN does", matches!(fw.probe(Some("INSIDE"), Some("OUTSIDE"), &syn, &acls, e.now()), FwProbe::Policy { action: FwAction::Inspect, .. }));
    let synack = flow(PROTO_TCP, "192.168.1.10", "203.0.113.10", 41000, 80, SYN | ACK);
    t.check("a SYN+ACK does not", matches!(fw.probe(Some("INSIDE"), Some("OUTSIDE"), &synack, &acls, e.now()), FwProbe::NotNew { .. }));
    let mut reply = flow(PROTO_ICMP, "192.168.1.10", "203.0.113.10", 7, 7, 0);
    reply.icmp = Some((0, 0));
    reply.ports = true;
    t.check("an echo reply does not", matches!(fw.probe(Some("INSIDE"), Some("OUTSIDE"), &reply, &acls, e.now()), FwProbe::NotNew { .. }));
    let mut req = reply.clone();
    req.icmp = Some((8, 0));
    t.check("an echo request does", matches!(fw.probe(Some("INSIDE"), Some("OUTSIDE"), &req, &acls, e.now()), FwProbe::Policy { action: FwAction::Inspect, .. }));
    // traceroute: the errors of the far side are related to the session
    let mut e = zoned();
    let at = mark(&e);
    let _ = e.cli_line(PC, "traceroute -n 203.0.113.10");
    run_for(&mut e, 6 * SEC);
    let shown = out(&mut e, PC);
    t.check("traceroute through the firewall", shown.contains("192.168.1.1") && shown.contains("203.0.113.10"));
    t.check("the port-unreachable passed as related", notes(&e, at).iter().any(|n| matches!(n, FwNote::Related { .. })));
}

// the states of one TCP connection, as it runs
fn tcp_states(t: &mut T) {
    let mut e = zoned();
    let _ = pings(&mut e, PC, "203.0.113.10", 1);
    let _ = pdu(&mut e, PC, PduProto::Tcp, "203.0.113.10", 42000, 80);
    let mut seen: Vec<TcpState> = Vec::new();
    for _ in 0..100_000 {
        run_for(&mut e, 200);
        let row = e.fw_sessions(FW).into_iter().find(|r| r.session.sport == 42000);
        if let Some(st) = row.and_then(|r| r.session.tcp)
            && seen.last() != Some(&st)
        {
            seen.push(st);
        }
    }
    t.check("SYN", seen.contains(&TcpState::Syn));
    t.check("SYN+ACK", seen.contains(&TcpState::SynAck));
    t.check("established", seen.contains(&TcpState::Established));
    t.check("a FIN", seen.contains(&TcpState::Fin) || seen.contains(&TcpState::Closing));
    t.eq("the order", seen.first().copied(), Some(TcpState::Syn));
    run_for(&mut e, SEC);
    let row = e.fw_sessions(FW).into_iter().find(|r| r.session.sport == 42000);
    t.eq("closing at the end", row.as_ref().and_then(|r| r.session.tcp), Some(TcpState::Closing));
    t.check("packets both ways", row.as_ref().is_some_and(|r| r.session.packets[0] >= 3 && r.session.packets[1] >= 3 && r.session.bytes[0] > 0));
    // a new connection on the same ports while the old one winds down starts over
    let fl = flow(PROTO_TCP, "192.168.1.10", "203.0.113.10", 42000, 80, SYN);
    t.check("a SYN on the ports of a closing session belongs to it", e.firewall(FW).is_some_and(|f| matches!(f.probe(Some("INSIDE"), Some("OUTSIDE"), &fl, &jfn_sim::acl::Acls::default(), e.now()), FwProbe::Session { back: false, .. })));
    // closing times out after the FIN timer
    let _ = conf(&mut e, &["ip inspect tcp finwait-time 2"]);
    run_for(&mut e, 3 * SEC);
    t.check("it ends after the FIN timer", !e.fw_sessions(FW).iter().any(|r| r.session.sport == 42000));
    t.check("gone only when a packet looks", e.firewall(FW).is_some_and(|f| f.sessions().any(|x| x.sport == 42000)));
    let at = mark(&e);
    let _ = pings(&mut e, PC, "203.0.113.10", 1);
    t.check("then a note", notes(&e, at).iter().any(|n| matches!(n, FwNote::Expired { .. })));
    t.check("and it is gone", e.firewall(FW).is_some_and(|f| !f.sessions().any(|x| x.sport == 42000)));
    // a SYN nobody answers: half-open until its timer
    let mut e = zoned();
    let _ = conf(&mut e, &["ip inspect tcp synwait-time 5"]);
    let _ = pdu(&mut e, PC, PduProto::Tcp, "203.0.113.99", 42100, 80);
    run_for(&mut e, 100 * MS);
    let _ = e.apply(jfn_sim::Action::Power { dev: PC, on: false });
    t.eq("a SYN alone", e.fw_sessions(FW).iter().find(|r| r.session.sport == 42100).and_then(|r| r.session.tcp), Some(TcpState::Syn));
    t.eq("half-open", e.firewall(FW).map(|f| f.half_open(e.now())), Some(1));
    run_for(&mut e, 6 * SEC);
    t.check("over after the SYN timer", !e.fw_sessions(FW).iter().any(|r| r.session.sport == 42100));
    // an established one lives for its idle timer
    let mut e = zoned();
    let _ = conf(&mut e, &["ip inspect tcp idle-time 7"]);
    t.check("a RST ends a session at once", {
        // nobody listens on port 81: the far end answers the SYN with a RST
        let at = mark(&e);
        let _ = pdu(&mut e, PC, PduProto::Tcp, "203.0.113.10", 42200, 81);
        run_for(&mut e, SEC);
        notes(&e, at).iter().any(|n| matches!(n, FwNote::Reset { .. })) && !e.fw_sessions(FW).iter().any(|r| r.session.sport == 42200)
    });
    t.eq("the reset counted", e.firewall(FW).map(|f| f.stats.reset >= 1), Some(true));
}

// the idle timers of UDP and ICMP
fn timers(t: &mut T) {
    let mut e = zoned();
    let _ = pings(&mut e, PC, "203.0.113.10", 1);
    let _ = pdu(&mut e, PC, PduProto::Udp, "203.0.113.10", 43000, 5000);
    run_for(&mut e, 100 * MS);
    let row = e.fw_sessions(FW).into_iter().find(|r| r.session.sport == 43000);
    t.eq("UDP lives 30 s", row.as_ref().map(|r| (r.ends - r.session.last) / SEC), Some(30));
    run_for(&mut e, 29 * SEC);
    t.check("after 29 s still there", e.fw_sessions(FW).iter().any(|r| r.session.sport == 43000));
    run_for(&mut e, 2 * SEC);
    t.check("after 31 s gone", !e.fw_sessions(FW).iter().any(|r| r.session.sport == 43000));
    let _ = pdu(&mut e, PC, PduProto::Udp, "203.0.113.10", 43001, 53);
    run_for(&mut e, 100 * MS);
    let row = e.fw_sessions(FW).into_iter().find(|r| r.session.sport == 43001);
    t.eq("to port 53: 5 s", row.as_ref().map(|r| (r.ends - r.session.last) / SEC), Some(5));
    let _ = e.ping(PC, ip("203.0.113.10"), PingOpts { count: 1, ..PingOpts::default() });
    run_for(&mut e, 500 * MS);
    let row = e.fw_sessions(FW).into_iter().find(|r| r.session.proto == PROTO_ICMP);
    t.eq("ICMP: 10 s", row.as_ref().map(|r| (r.ends - r.session.last) / SEC), Some(10));
    run_for(&mut e, 11 * SEC);
    t.check("ICMP gone", !e.fw_sessions(FW).iter().any(|r| r.session.proto == PROTO_ICMP));
    let _ = conf(&mut e, &["ip inspect udp idle-time 2", "ip inspect icmp idle-time 3", "ip inspect dns-timeout 1"]);
    let _ = pdu(&mut e, PC, PduProto::Udp, "203.0.113.10", 43002, 5000);
    run_for(&mut e, 100 * MS);
    let row = e.fw_sessions(FW).into_iter().find(|r| r.session.sport == 43002);
    t.eq("a timer as configured", row.as_ref().map(|r| (r.ends - r.session.last) / SEC), Some(2));
    let _ = conf(&mut e, &["no ip inspect udp idle-time"]);
    let row = e.fw_sessions(FW).into_iter().find(|r| r.session.sport == 43002);
    t.eq("back to the default", row.as_ref().map(|r| (r.ends - r.session.last) / SEC), Some(30));
}

// the limit of half-open sessions
fn half_open(t: &mut T) {
    let mut e = zoned();
    let _ = conf(&mut e, &["ip inspect max-incomplete high 2"]);
    let at = mark(&e);
    for i in 0..4 {
        let _ = pdu(&mut e, PC, PduProto::Tcp, "203.0.113.99", 44000 + i, 80);
        run_for(&mut e, 10 * MS);
    }
    t.eq("two at most", e.firewall(FW).map(|f| f.half_open(e.now())), Some(2));
    t.check("the note", notes(&e, at).iter().any(|n| matches!(n, FwNote::Limit { max: 2, .. }) && n.is_drop()));
    t.check("counted", e.firewall(FW).is_some_and(|f| f.stats.limited >= 2));
    let _ = conf(&mut e, &["no ip inspect max-incomplete high"]);
    t.eq("the default limit", e.firewall(FW).map(|f| f.max_half_open), Some(jfn_sim::fw::HALF_OPEN_MAX));
}

// NAT with zones, both directions: the firewall sees the inside addresses
fn with_nat(t: &mut T) {
    let cfg = format!("{}{}access-list 1 permit 192.168.1.0 0.0.0.255\nip nat inside source list 1 interface GigabitEthernet0/2 overload\nip nat inside source static tcp 10.0.0.10 80 203.0.113.1 8080\ninterface GigabitEthernet0/0\n ip nat inside\n!\ninterface GigabitEthernet0/1\n ip nat inside\n!\ninterface GigabitEthernet0/2\n ip nat outside\n!\n", ZONES, MEMBERS);
    let mut e = engine(&cfg);
    t.eq("no refusal at boot", e.boot_errors(FW).len(), 0);
    let at = mark(&e);
    t.eq("ping out through PAT and back", pings(&mut e, PC, "203.0.113.10", 2), 2);
    t.check("the session has the inside source", notes(&e, at).iter().any(|n| matches!(n, FwNote::NewSession { what, .. } if what.contains("192.168.1.10"))));
    t.check("TCP out", tcp_ok(&mut e, PC, "203.0.113.10", 45000, 80));
    t.check("the translation exists", !e.nat_translations(FW).is_empty());
    t.check("the session reads inside to outside", e.fw_sessions(FW).iter().any(|r| r.session.sport == 45000 && format!("{:?}", r.session.src).contains("192")));
    let at = mark(&e);
    t.check("from outside by the port forward", tcp_ok(&mut e, EXT, "203.0.113.1", 45001, 8080));
    t.check("the class saw the real address of the web server", notes(&e, at).iter().any(|n| matches!(n, FwNote::NewSession { pair, what, .. } if pair == "OUT-DMZ" && what.contains("10.0.0.10:80"))));
    t.check("the session of the forward", e.fw_sessions(FW).iter().any(|r| r.session.sport == 45001 && r.session.dport == 80));
    // a port forward the policy does not allow: translated, then dropped
    let cfg2 = format!("{}{}ip nat inside source static tcp 192.168.1.10 22 203.0.113.1 2222\ninterface GigabitEthernet0/0\n ip nat inside\n!\ninterface GigabitEthernet0/2\n ip nat outside\n!\n", ZONES, MEMBERS);
    let mut e = engine(&cfg2);
    let at = mark(&e);
    t.check("a forward into a zone without a rule goes nowhere", !tcp_ok(&mut e, EXT, "203.0.113.1", 45002, 2222));
    t.check("the note names the zones", notes(&e, at).iter().any(|n| matches!(n, FwNote::NoRule { from, to, what, .. } if from == "OUTSIDE" && to == "INSIDE" && what.contains("192.168.1.10:22"))));
}

// access lists with zones: the inbound list first, the outbound list after the zones and NAT
fn with_acls(t: &mut T) {
    let cfg = format!("{}{}access-list 110 deny icmp host 192.168.1.10 any\naccess-list 110 permit ip any any\ninterface GigabitEthernet0/0\n ip access-group 110 in\n!\naccess-list 120 deny tcp any any eq www\naccess-list 120 permit ip any any\ninterface GigabitEthernet0/2\n ip access-group 120 out\n!\n", ZONES, MEMBERS);
    let mut e = engine(&cfg);
    let at = mark(&e);
    t.eq("the inbound list drops the ping", pings(&mut e, PC, "203.0.113.10", 1), 0);
    t.check("before any zone", !notes(&e, at).iter().any(|n| matches!(n, FwNote::NewSession { .. })));
    let at = mark(&e);
    t.check("the outbound list stops what the zones allowed", !tcp_ok(&mut e, PC, "203.0.113.10", 46000, 80));
    t.check("a session began all the same", notes(&e, at).iter().any(|n| matches!(n, FwNote::NewSession { .. })));
    t.check("an ACL keeps its ICMP answer", e.trace.events().any(|x| x.dev == FW && matches!(&x.note, Note::IcmpSent { typ: 3, code: 13, .. })));
    // a class by an access list
    let cfg = format!("access-list 150 permit tcp host 192.168.1.10 any eq www\nclass-map type inspect C-L\n match access-group 150\n!\nip access-list extended NAMED\n permit udp any any eq domain\n!\nclass-map type inspect C-N\n match access-group name NAMED\n!\npolicy-map type inspect P\n class type inspect C-L\n  inspect\n class type inspect C-N\n  pass\n!\nzone security A\n!\nzone security B\n!\nzone-pair security AB source A destination B\n service-policy type inspect P\n!\ninterface GigabitEthernet0/0\n zone-member security A\n!\ninterface GigabitEthernet0/2\n zone-member security B\n!\n");
    let mut e = engine(&cfg);
    t.check("the list permits it", tcp_ok(&mut e, PC, "203.0.113.10", 46100, 80));
    let hits = e.access_lists(FW).into_iter().find(|l| l.name == "150").map_or(0, |l| l.rules[0].2);
    t.eq("and counts it", hits, 1);
    t.check("the list does not permit port 81", !tcp_ok(&mut e, PC, "203.0.113.10", 46101, 81));
    let at = mark(&e);
    let _ = pdu(&mut e, PC, PduProto::Udp, "203.0.113.10", 46102, 53);
    run_for(&mut e, SEC);
    let n = notes(&e, at);
    t.check("a named list with pass", n.iter().any(|x| matches!(x, FwNote::Pass { class, .. } if class == "C-N")));
    t.check("pass remembers nothing: the answer is dropped", n.iter().any(|x| matches!(x, FwNote::NoRule { from, to, .. } if from == "B" && to == "A")));
}

// the self zone: a policy to or from the device decides as any other
fn self_zone(t: &mut T) {
    let cfg = format!("{}{}policy-map type inspect P-SELF\n class type inspect C-WEB\n  pass\n!\nzone-pair security OUT-SELF source OUTSIDE destination self\n service-policy type inspect P-SELF\n!\n", ZONES, MEMBERS);
    let mut e = engine(&cfg);
    let at = mark(&e);
    t.eq("a ping from outside to the device is dropped now", pings(&mut e, EXT, "203.0.113.1", 1), 0);
    t.check("by class-default", notes(&e, at).iter().any(|n| matches!(n, FwNote::Drop { pair, class, .. } if pair == "OUT-SELF" && class == "class-default")));
    t.eq("from inside still", pings(&mut e, PC, "192.168.1.1", 1), 1);
    // from the device: without a policy it passes, but the answer meets the policy above
    let _ = exec(&mut e, "ping 203.0.113.10 repeat 2");
    run_for(&mut e, 4 * SEC);
    let at = mark(&e);
    let _ = conf(&mut e, &["no zone-pair security OUT-SELF"]);
    let o = exec(&mut e, "ping 203.0.113.10 repeat 3");
    run_for(&mut e, 4 * SEC);
    let shown = format!("{}{}", o, out(&mut e, FW));
    t.check("without the pair the device reaches the outside", shown.contains("!!"));
    t.check("from self without a policy", notes(&e, at).iter().any(|n| matches!(n, FwNote::SelfPass { from, .. } if from == "self")));
    // self to a zone with inspect: the answer comes back by the session even with a strict way in
    let _ = conf(&mut e, &["zone-pair security SELF-OUT source self destination OUTSIDE", "service-policy type inspect P-ALL", "exit", "zone-pair security OUT-SELF source OUTSIDE destination self", "service-policy type inspect P-SELF"]);
    let at = mark(&e);
    let o = exec(&mut e, "ping 203.0.113.10 repeat 3");
    run_for(&mut e, 4 * SEC);
    let shown = format!("{}{}", o, out(&mut e, FW));
    t.check("inspect from the device", shown.contains("!!"));
    t.check("a session from self", notes(&e, at).iter().any(|n| matches!(n, FwNote::NewSession { pair, .. } if pair == "SELF-OUT")));
    t.check("the zone name self is the device's", conf(&mut e, &["zone security self"]).contains("% The name self"));
}

// drop silently, reject with an answer
fn drop_reject(t: &mut T) {
    let reject = |act: &str| format!("class-map type inspect C\n match protocol tcp\n!\npolicy-map type inspect P\n class type inspect C\n  {}\n!\nzone security A\n!\nzone security B\n!\nzone-pair security AB source A destination B\n service-policy type inspect P\n!\ninterface GigabitEthernet0/0\n zone-member security A\n!\ninterface GigabitEthernet0/2\n zone-member security B\n!\n", act);
    let mut e = engine(&reject("drop"));
    let _ = pings(&mut e, PC, "192.168.1.1", 1);
    let at = mark(&e);
    let _ = pdu(&mut e, PC, PduProto::Tcp, "203.0.113.10", 47000, 80);
    run_for(&mut e, 500 * MS);
    t.check("drop", notes(&e, at).iter().any(|n| matches!(n, FwNote::Drop { .. })));
    t.check("silently", !e.trace.events().skip(at).any(|x| x.dev == FW && matches!(&x.note, Note::IcmpSent { .. })));
    let mut e = engine(&reject("reject"));
    let _ = pings(&mut e, PC, "192.168.1.1", 1);
    let at = mark(&e);
    let _ = pdu(&mut e, PC, PduProto::Tcp, "203.0.113.10", 47001, 80);
    run_for(&mut e, 500 * MS);
    t.check("reject", notes(&e, at).iter().any(|n| matches!(n, FwNote::Reject { .. }) && n.is_drop()));
    t.check("with administratively prohibited", e.trace.events().skip(at).any(|x| x.dev == FW && matches!(&x.note, Note::IcmpSent { typ: 3, code: 13, .. })));
    let _ = out(&mut e, PC);
    let _ = e.cli_line(PC, "curl -sS http://203.0.113.10/");
    run_for(&mut e, 2 * SEC);
    t.check("the client hears of it at once", out(&mut e, PC).contains("curl: (7)"));
    // class-default as the choice
    let cfg = reject("inspect").replace("zone security A", "policy-map type inspect Q\n class class-default\n  reject\n!\nzone security A").replace("service-policy type inspect P", "service-policy type inspect Q");
    let mut e = engine(&cfg);
    let _ = pings(&mut e, PC, "192.168.1.1", 1);
    let at = mark(&e);
    let _ = e.ping(PC, ip("203.0.113.10"), PingOpts { count: 1, ..PingOpts::default() });
    run_for(&mut e, 2 * SEC);
    t.check("class-default rejects", notes(&e, at).iter().any(|n| matches!(n, FwNote::Reject { class, .. } if class == "class-default")));
    // a class without an action drops
    let mut e = engine(&reject("no drop"));
    let at = mark(&e);
    let _ = pdu(&mut e, PC, PduProto::Tcp, "203.0.113.10", 47002, 80);
    run_for(&mut e, 500 * MS);
    t.check("no action named: it drops", notes(&e, at).iter().any(|n| matches!(n, FwNote::Drop { class, .. } if class == "C")));
}

fn ping6(e: &mut Engine, dev: u32, dst: &str, n: u32) -> String {
    let _ = out(e, dev);
    let _ = e.cli_line(dev, &format!("ping -6 -c {} {}", n, dst));
    run_for(e, (n as u64 + 2) * SEC);
    out(e, dev)
}

// a default route over IPv6 once the hosts' addresses passed their check (DAD), and the firewall's
// neighbours (the first packet to a neighbour is lost at IOS)
fn routes6(e: &mut Engine) {
    run_for(e, 3 * SEC);
    for (dev, gw) in [(PC, "2001:db8:1::1"), (WEB, "2001:db8:2::1"), (EXT, "2001:db8:3::1")] {
        let _ = e.cli_line(dev, &format!("ip -6 route add default via {} dev eth0", gw));
        let _ = e.cli_line(dev, &format!("ping -6 -c 1 {}", gw));
    }
    run_for(e, 3 * SEC);
    let _ = e.take_reports();
}

// zones and sessions over IPv6; neighbor discovery stays out of it
fn ipv6(t: &mut T) {
    let mut e = zoned();
    routes6(&mut e);
    let at = mark(&e);
    let o = ping6(&mut e, PC, "2001:db8:3::10", 3);
    t.check("IPv6 from inside and back", o.contains("3 received") || o.contains("2 received"));
    let n = notes(&e, at);
    t.check("an IPv6 session", n.iter().any(|x| matches!(x, FwNote::NewSession { v6: true, .. })));
    t.check("the answers by it", n.iter().any(|x| matches!(x, FwNote::Session { back: true, v6: true, .. })));
    t.check("neighbor discovery is not asked", !n.iter().any(|x| matches!(x, FwNote::SelfPass { what, .. } if what.contains("ff02"))));
    let o = ping6(&mut e, EXT, "2001:db8:1::10", 2);
    t.check("IPv6 from outside: nothing", !o.contains("bytes from"));
    t.check("a session reads IPv6 addresses", e.fw_sessions(FW).iter().any(|r| r.session.text().contains("2001:db8:1::10")));
    // to the device and reject
    let o = ping6(&mut e, PC, "2001:db8:1::1", 1);
    t.check("the device answers over IPv6", o.contains("1 received"));
    let cfg = format!("{}{}policy-map type inspect Q\n class class-default\n  reject\n!\nzone-pair security OUT-IN source OUTSIDE destination INSIDE\n service-policy type inspect Q\n!\n", ZONES, MEMBERS);
    let mut e = engine(&cfg);
    routes6(&mut e);
    let at = mark(&e);
    let _ = ping6(&mut e, EXT, "2001:db8:1::10", 1);
    t.check("reject over IPv6", notes(&e, at).iter().any(|n| matches!(n, FwNote::Reject { v6: true, .. })));
}

// the configuration through running and startup configuration
fn round_trip(t: &mut T) {
    let cfg = format!("{}{}policy-map type inspect P-L\n class type inspect C-WEB\n  inspect log\n class class-default\n  drop log\n!\nzone security LOG\n description a zone with a note\n!\nzone-pair security OUT-SELF source OUTSIDE destination self\n!\nip inspect tcp idle-time 100\nip inspect max-incomplete high 77\n", ZONES, MEMBERS);
    let e = engine(&cfg);
    t.eq("no refusal at boot", e.boot_errors(FW).len(), 0);
    let run1 = e.running_config(FW);
    for l in ["class-map type inspect match-any C-ALL\n match protocol tcp\n", "class-map type inspect C-WEB\n match protocol http\n", "  inspect log\n class class-default\n  drop log\n", "zone security LOG\n description a zone with a note\n", "zone-pair security IN-OUT source INSIDE destination OUTSIDE\n service-policy type inspect P-ALL\n", "zone-pair security OUT-SELF source OUTSIDE destination self\n", "ip inspect tcp idle-time 100\nip inspect max-incomplete high 77\n", " zone-member security OUTSIDE\n"] {
        t.check(&format!("the running configuration has: {}", l.lines().next().unwrap_or("")), run1.contains(l));
    }
    // as a startup configuration through the same parser
    let mut p = project("jf-fw", "");
    if let Some(d) = p.device_mut(FW) {
        d.startup_config = run1.clone();
    }
    let mut e2 = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    run_for(&mut e2, SEC);
    t.eq("it boots without a refusal", e2.boot_errors(FW).len(), 0);
    t.eq("and reads the same", e2.running_config(FW), run1);
    warm(&mut e2);
    t.check("and works", pings(&mut e2, PC, "203.0.113.10", 1) == 1);
    let kept = e2.firewall(FW).cloned().unwrap_or_default();
    t.eq("the timer", kept.timers.get(jfn_sim::fw::FwTimer::Tcp), 100);
    t.eq("the limit", kept.max_half_open, 77);
    t.eq("the actions", kept.policies.get("P-L").map(|x| (x.classes[0].action, x.classes[0].log, x.default.action, x.default.log)), Some((Some(FwAction::Inspect), true, Some(FwAction::Drop), true)));
    t.eq("the conditions", kept.classes.get("C-WEB").map(|c| c.matches.clone()), Some(alloc::vec![Match::Protocol(s("http"))]));
    // refusals in the console's words
    let mut e = zoned();
    let refuse = |e: &mut Engine, line: &str, word: &str| {
        let o = conf(e, &[line]);
        o.contains(word)
    };
    t.check("an unknown zone on an interface", {
        let o = conf(&mut e, &["interface GigabitEthernet0/3", "zone-member security NOPE"]);
        o.contains("% Zone NOPE does not exist")
    });
    t.check("an unknown zone in a pair", refuse(&mut e, "zone-pair security X source NOPE destination DMZ", "% Zone NOPE does not exist"));
    t.check("a pair of one zone", refuse(&mut e, "zone-pair security X source DMZ destination DMZ", "% A zone pair joins two different zones"));
    t.check("a second pair for one direction", refuse(&mut e, "zone-pair security X source INSIDE destination OUTSIDE", "% There is already a zone pair"));
    t.check("a zone with interfaces stays", refuse(&mut e, "no zone security DMZ", "% Zone DMZ still has interfaces"));
    t.check("a policy in use stays", refuse(&mut e, "no policy-map type inspect P-ALL", "% Policy P-ALL still applies"));
    t.check("a class in use stays", refuse(&mut e, "no class-map type inspect C-ALL", "% Class C-ALL is still part"));
    t.check("an unknown class in a policy", conf(&mut e, &["policy-map type inspect P-ALL", "class type inspect NOPE"]).contains("% Class NOPE does not exist"));
    t.check("an unknown policy in a pair", conf(&mut e, &["zone-pair security IN-OUT source INSIDE destination OUTSIDE", "service-policy type inspect NOPE"]).contains("% Policy NOPE does not exist"));
    // a pair and its sessions go together
    t.check("a session", {
        let _ = pings(&mut e, PC, "203.0.113.10", 1);
        e.fw_sessions(FW).iter().any(|r| r.session.pair == "IN-OUT")
    });
    let _ = conf(&mut e, &["no zone-pair security IN-OUT"]);
    t.check("no session of a pair that went", !e.firewall(FW).is_some_and(|f| f.sessions().any(|x| x.pair == "IN-OUT")));
    // only firewalls have the commands
    let mut r = Engine::from_project(&project("jf-r4", "")).unwrap_or_else(|_| Engine::new(0));
    run_for(&mut r, SEC);
    t.check("a router does not know zones", conf(&mut r, &["zone security A"]).contains("% Invalid input"));
    t.check("nor the action", r.apply(jfn_sim::Action::Fw { dev: FW, op: jfn_sim::FwOp::Zone { name: s("A"), add: true } }).is_err());
}

// a power cycle forgets sessions and counters, the configuration stays
fn power_cycle(t: &mut T) {
    let mut e = zoned();
    let _ = pings(&mut e, PC, "203.0.113.10", 1);
    t.check("a session", !e.fw_sessions(FW).is_empty());
    let cfg = e.running_config(FW);
    let _ = e.apply(jfn_sim::Action::Power { dev: FW, on: false });
    run_for(&mut e, 100 * MS);
    let _ = e.apply(jfn_sim::Action::Power { dev: FW, on: true });
    run_for(&mut e, SEC);
    t.check("no session after a power cycle", e.firewall(FW).is_some_and(|f| f.sessions().next().is_none()));
    t.eq("counters at 0", e.firewall(FW).map(|f| f.stats.created), Some(0));
    t.eq("the configuration stays", e.running_config(FW), cfg);
    warm(&mut e);
    t.eq("and it works again", pings(&mut e, PC, "203.0.113.10", 1), 1);
    // clear
    let _ = exec(&mut e, "clear zone-pair inspect sessions IN-DMZ");
    t.check("clear of another pair leaves it", e.fw_sessions(FW).iter().any(|r| r.session.pair == "IN-OUT"));
    let _ = exec(&mut e, "clear zone-pair inspect sessions");
    t.check("clear empties", e.fw_sessions(FW).is_empty());
    let _ = exec(&mut e, "clear zone-pair counter");
    t.eq("clear counter", e.firewall(FW).map(|f| f.stats.created), Some(0));
}

fn traffic(e: &mut Inline) {
    for (i, line) in ["ping -c 2 203.0.113.10", "curl -s http://203.0.113.10/", "ping -c 1 10.0.0.10"].iter().enumerate() {
        let _ = e.send(Cmd::Cli { dev: PC, line: s(line) }, 0);
        let _ = e.send(Cmd::RunFor(3 * SEC + i as u64 * MS), 0);
        let _ = e.reports();
    }
    let _ = e.send(Cmd::Cli { dev: EXT, line: s("ping -c 1 192.168.1.10") }, 0);
    let _ = e.send(Cmd::RunFor(40 * SEC), 0);
    let _ = e.reports();
    let _ = e.send(Cmd::Cli { dev: PC, line: s("ping -c 1 203.0.113.10") }, 0);
    let _ = e.send(Cmd::RunFor(2 * SEC), 0);
    let _ = e.reports();
}

// two runs, a clone in mid-run, a replay from keyframes
fn determinism(t: &mut T) {
    let p = project("jf-fw", &format!("{}{}", ZONES, MEMBERS));
    let make = || Inline::new(Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0)));
    let (mut a, mut b) = (make(), make());
    traffic(&mut a);
    traffic(&mut b);
    t.eq("two runs", digest(&a.eng), digest(&b.eng));
    t.check("with sessions behind them", a.eng.firewall(FW).is_some_and(|f| f.stats.created >= 2));
    // a clone in mid-run
    let mut c = make();
    let _ = c.send(Cmd::Cli { dev: PC, line: s("ping -c 5 203.0.113.10") }, 0);
    let _ = c.send(Cmd::RunFor(2500 * MS), 0);
    let mut e1 = c.eng.clone();
    let mut e2 = c.eng.clone();
    run_for(&mut e1, 40 * SEC);
    run_for(&mut e2, 40 * SEC);
    t.eq("a clone in mid-run", digest(&e1), digest(&e2));
    // a replay from keyframes reaches the same state
    let mut d = make();
    d.rewind_on(Budget { gap_events: 20, ..Budget::default() });
    traffic(&mut d);
    let pos = d.pos();
    let past = d.restore_pos(pos, None).and_then(|mut r| d.rewind().and_then(|w| r.run(w, usize::MAX).then(|| r.finish())));
    t.eq("a replay from keyframes", past.as_ref().map(|x| digest(&x.eng)), Some(digest(&d.eng)));
    t.eq("with its sessions", past.as_ref().map(|x| x.eng.firewall(FW).map(|f| f.stats.created)), Some(d.eng.firewall(FW).map(|f| f.stats.created)));
    t.check("the keyframe counts the firewall", jfn_sim::keyframe::footprint(&d.eng) > 0);
}

// what the Test part asks against what the engine does with the packet
fn prediction(t: &mut T) {
    let mut e = zoned();
    let _ = pings(&mut e, PC, "192.168.1.1", 1);
    let _ = pings(&mut e, EXT, "203.0.113.1", 1);
    let _ = pings(&mut e, WEB, "10.0.0.1", 1);
    let table: [(&str, u32, u16, &str, &str, u8, u16, u16, bool); 7] = [
        ("inside to outside, tcp", PC, 0, "192.168.1.10", "203.0.113.10", PROTO_TCP, 48000, 80, true),
        ("inside to outside, udp", PC, 0, "192.168.1.10", "203.0.113.10", PROTO_UDP, 48001, 5000, true),
        ("outside to inside, tcp", EXT, 2, "203.0.113.10", "192.168.1.10", PROTO_TCP, 48002, 80, false),
        ("outside to the web server", EXT, 2, "203.0.113.10", "10.0.0.10", PROTO_TCP, 48003, 80, true),
        ("outside to the web server, other port", EXT, 2, "203.0.113.10", "10.0.0.10", PROTO_TCP, 48004, 22, false),
        ("web server to inside", WEB, 1, "10.0.0.10", "192.168.1.10", PROTO_TCP, 48005, 80, false),
        ("web server to outside", WEB, 1, "10.0.0.10", "203.0.113.10", PROTO_UDP, 48006, 53, false),
    ];
    for (name, from, iface, src, dst, proto, sport, dport, pass) in table {
        let fl = Flow { proto, src: ip(src), dst: ip(dst), sport, dport, ports: true, icmp: None, flags: if proto == PROTO_TCP { SYN } else { 0 }, later: false, mf: false, id: 0 };
        let told = e.fw_test(FW, iface, false, &fl);
        t.eq(&format!("{}: predicted", name), told.as_ref().map(|x| x.probe.passes()), Some(pass));
        let before = e.firewall(FW).map(|f| f.stats);
        let at = mark(&e);
        t.eq(&format!("{}: asking changes nothing", name), e.firewall(FW).map(|f| f.stats), before);
        let _ = pdu(&mut e, from, if proto == PROTO_TCP { PduProto::Tcp } else { PduProto::Udp }, dst, sport, dport);
        run_for(&mut e, 50 * MS);
        let first = notes(&e, at).into_iter().find(|n| !matches!(n, FwNote::SelfPass { .. } | FwNote::Expired { .. }));
        t.eq(&format!("{}: as the engine decides", name), first.as_ref().map(|n| !n.is_drop()), Some(pass));
    }
    // by the outbound question: the zones follow from the routes
    let fl = Flow { proto: PROTO_TCP, src: ip("192.168.1.10"), dst: ip("203.0.113.10"), sport: 48100, dport: 80, ports: true, icmp: None, flags: SYN, later: false, mf: false, id: 0 };
    let told = e.fw_test(FW, 2, true, &fl);
    t.eq("outbound: the way in by the routes", told.as_ref().map(|x| (x.from.clone(), x.to.clone())), Some((Some(s("INSIDE")), Some(s("OUTSIDE")))));
    // an existing session
    t.check("a connection", {
        let _ = pdu(&mut e, PC, PduProto::Udp, "203.0.113.10", 48200, 5000);
        run_for(&mut e, 200 * MS);
        !e.fw_sessions(FW).is_empty()
    });
    let back = Flow { proto: PROTO_UDP, src: ip("203.0.113.10"), dst: ip("192.168.1.10"), sport: 5000, dport: 48200, ports: true, icmp: None, flags: 0, later: false, mf: false, id: 0 };
    let told = e.fw_test(FW, 2, false, &back);
    t.check("the answer: an existing connection", told.is_some_and(|x| matches!(x.probe, FwProbe::Session { back: true, .. })));
    // with NAT: from outside the translation first
    let cfg = format!("{}{}ip nat inside source static tcp 10.0.0.10 80 203.0.113.1 8080\ninterface GigabitEthernet0/1\n ip nat inside\n!\ninterface GigabitEthernet0/2\n ip nat outside\n!\n", ZONES, MEMBERS);
    let mut e = engine(&cfg);
    let fl = Flow { proto: PROTO_TCP, src: ip("203.0.113.10"), dst: ip("203.0.113.1"), sport: 48300, dport: 8080, ports: true, icmp: None, flags: SYN, later: false, mf: false, id: 0 };
    let told = e.fw_test(FW, 2, false, &fl);
    t.check("NAT from outside: the real address", told.as_ref().is_some_and(|x| x.flow.dport == 80 && x.to.as_deref() == Some("DMZ") && x.probe.passes()));
    t.check("and for real", tcp_ok(&mut e, EXT, "203.0.113.1", 48300, 8080));
}

// what the forms of the tab type, read by the console as a person's lines would be
fn forms(t: &mut T) {
    use crate::workspace::devwin::fwzones::*;
    let mut e = engine("");
    let fw = |e: &Engine| e.firewall(FW).cloned().unwrap_or_default();
    let run = |e: &mut Engine, ls: Result<Vec<String>, String>| -> String {
        match ls {
            Ok(v) => {
                let refs: Vec<&str> = v.iter().map(String::as_str).collect();
                conf(e, &refs)
            }
            Err(x) => x,
        }
    };
    t.eq("a zone", zone_lines(" A ", &fw(&e)), Ok(alloc::vec![s("zone security A")]));
    t.check("two words are no name", zone_lines("A B", &fw(&e)).is_err());
    t.check("self is taken", zone_lines("SELF", &fw(&e)).is_err());
    for z in ["A", "B"] {
        let l = zone_lines(z, &fw(&e));
        t.eq(&format!("zone {}: the console takes it", z), run(&mut e, l), s(""));
    }
    t.check("a zone twice is refused", zone_lines("A", &fw(&e)).is_err());
    let m = Match::Protocol(s("http"));
    t.eq("a class", class_lines("C", true, &m), Ok(alloc::vec![s("class-map type inspect match-any C"), s("match protocol http")]));
    let l = class_lines("C", true, &m);
    t.eq("the console takes the class", run(&mut e, l), s(""));
    let l = class_lines("C", true, &Match::Access(s("101")));
    t.eq("a second condition", run(&mut e, l), s(""));
    t.eq("both conditions", fw(&e).classes.get("C").map(|c| (c.any, c.matches.len())), Some((true, 2)));
    let l = match_del_lines("C", true, &Match::Access(s("101")));
    t.eq("one condition goes", conf(&mut e, &l.iter().map(String::as_str).collect::<Vec<_>>()), s(""));
    t.eq("one left", fw(&e).classes.get("C").map(|c| c.matches.clone()), Some(alloc::vec![m.clone()]));
    let l = rule_lines("P", "C", FwAction::Inspect, true);
    t.eq("a rule", l.clone(), Ok(alloc::vec![s("policy-map type inspect P"), s("class type inspect C"), s("inspect log")]));
    t.eq("the console takes the rule", run(&mut e, l), s(""));
    let l = rule_lines("P", "class-default", FwAction::Reject, false);
    t.eq("class-default", l.clone(), Ok(alloc::vec![s("policy-map type inspect P"), s("class class-default"), s("reject")]));
    t.eq("the console takes it", run(&mut e, l), s(""));
    t.eq("as set", fw(&e).policies.get("P").map(|p| (p.classes[0].action, p.classes[0].log, p.default.action)), Some((Some(FwAction::Inspect), true, Some(FwAction::Reject))));
    t.check("a class in use is refused with its reason", class_del_lines("C", &fw(&e)).is_err_and(|x| x.contains("Richtlinie P")));
    let l = pair_lines("", "A", "B", Some("P"), &fw(&e));
    t.eq("a pair takes its name from the zones", l.clone(), Ok(alloc::vec![s("zone-pair security A-B source A destination B"), s("service-policy type inspect P")]));
    t.eq("the console takes the pair", run(&mut e, l), s(""));
    t.check("one zone twice", pair_lines("", "A", "A", None, &fw(&e)).is_err());
    t.check("a second pair the same way", pair_lines("X", "A", "B", None, &fw(&e)).is_err());
    t.check("a policy in use", policy_del_lines("P", &fw(&e)).is_err_and(|x| x.contains("A-B")));
    t.check("a zone in a pair", zone_del_lines("A", &fw(&e)).is_err_and(|x| x.contains("A-B")));
    let l = member_lines("GigabitEthernet0/0", Some("A"));
    t.eq("an interface", conf(&mut e, &l.iter().map(String::as_str).collect::<Vec<_>>()), s(""));
    t.check("a zone with an interface", zone_del_lines("A", &fw(&e)).is_err_and(|x| x.contains("Schnittstellen")));
    // the picture
    let m = matrix(&fw(&e));
    t.eq("the picture's zones", m.zones.clone(), alloc::vec![s("A"), s("B"), s("self")]);
    t.eq("A to B", m.cells[0][1].clone(), Reach::Policy { name: s("P"), open: true });
    t.eq("B to A", m.cells[1][0].clone(), Reach::Closed);
    t.eq("A to A", m.cells[0][0].clone(), Reach::Free);
    t.eq("A to the device", m.cells[0][2].clone(), Reach::Free);
    // what the buttons take away
    let l = pair_del_lines("A-B");
    t.eq("the pair goes", conf(&mut e, &l.iter().map(String::as_str).collect::<Vec<_>>()), s(""));
    let l = policy_del_lines("P", &fw(&e));
    t.eq("the policy goes", run(&mut e, l), s(""));
    let l = class_del_lines("C", &fw(&e));
    t.eq("the class goes", run(&mut e, l), s(""));
    let l = member_lines("GigabitEthernet0/0", None);
    t.eq("the interface leaves", conf(&mut e, &l.iter().map(String::as_str).collect::<Vec<_>>()), s(""));
    let l = zone_del_lines("A", &fw(&e));
    t.eq("the zone goes", run(&mut e, l), s(""));
    t.eq("only B is left", fw(&e).zones.keys().cloned().collect::<Vec<_>>(), alloc::vec![s("B")]);
    // the rule of a policy goes, class-default falls back
    let l = rule_lines("Q", "class-default", FwAction::Pass, false);
    let _ = run(&mut e, l);
    let l = rule_del_lines("Q", "class-default");
    t.eq("class-default back", conf(&mut e, &l.iter().map(String::as_str).collect::<Vec<_>>()), s(""));
    t.eq("it drops again", fw(&e).policies.get("Q").and_then(|p| p.default.action), Some(FwAction::Drop));
    // the rows of the grid and the Test part's words
    let mut e = zoned();
    let _ = pings(&mut e, PC, "203.0.113.10", 1);
    let rows = session_rows(&e.fw_sessions(FW), e.now());
    t.eq("a row", rows.first().map(|r| (r[1].clone(), r[2].clone(), r[3].clone(), r[5].clone(), r[6].clone())), Some((s("icmp"), s("192.168.1.10"), s("203.0.113.10"), s("IN-OUT"), s("C-ALL"))));
    let fl = Flow { proto: PROTO_TCP, src: ip("203.0.113.10"), dst: ip("192.168.1.10"), sport: 1, dport: 80, ports: true, icmp: None, flags: SYN, later: false, mf: false, id: 0 };
    let told = e.fw_test(FW, 2, false, &fl).map(|x| told_zones(&x));
    t.check("no rule: in words", told.is_some_and(|x| !x.pass && x.text.contains("Keine Regel zwischen den Zonen OUTSIDE und INSIDE")));
    let _ = pdu(&mut e, PC, PduProto::Udp, "203.0.113.10", 49500, 5000);
    run_for(&mut e, 100 * MS);
    let fl = Flow { proto: PROTO_UDP, src: ip("203.0.113.10"), dst: ip("192.168.1.10"), sport: 5000, dport: 49500, ports: true, icmp: None, flags: 0, later: false, mf: false, id: 0 };
    let told = e.fw_test(FW, 2, false, &fl).map(|x| told_zones(&x));
    t.check("a session: in words", told.is_some_and(|x| x.pass && x.text.contains("Bestehende Verbindung: Rückweg von Sitzung")));
}

// the show commands, debug and log
fn shows(t: &mut T) {
    let cfg = format!("{}{}policy-map type inspect P-L\n class type inspect C-WEB\n  inspect log\n class class-default\n  drop log\n!\nzone-pair security OUT-IN source OUTSIDE destination INSIDE\n service-policy type inspect P-L\n!\n", ZONES, MEMBERS);
    let mut e = engine(&cfg);
    let o = exec(&mut e, "show zone security");
    t.check("show zone security", o.contains("zone INSIDE\n  interfaces: GigabitEthernet0/0\n") && o.contains("zone self"));
    let o = exec(&mut e, "show zone security DMZ");
    t.check("one zone", o.contains("zone DMZ") && !o.contains("zone INSIDE"));
    let _ = pings(&mut e, PC, "203.0.113.10", 1);
    let o = exec(&mut e, "show zone-pair security");
    t.check("show zone-pair security", o.contains("pair IN-OUT: INSIDE -> OUTSIDE, policy P-ALL\n") && o.contains("Sessions: 1 now"));
    let o = exec(&mut e, "show zone-pair security source INSIDE destination DMZ");
    t.check("one direction", o.contains("pair IN-DMZ") && !o.contains("pair IN-OUT"));
    let o = exec(&mut e, "show policy-map type inspect zone-pair IN-OUT");
    t.check("show policy-map", o.contains("policy P-ALL") && o.contains("class C-ALL (match-any)") && o.contains("match protocol icmp") && o.contains("action: inspect; 1 packets") && o.contains("sessions now: 1") && o.contains("class class-default"));
    let o = exec(&mut e, "show policy-map type inspect zone-pair IN-OUT sessions");
    t.check("sessions of a pair", o.contains("pair IN-OUT: 1 sessions") && o.contains("icmp 192.168.1.10 -> 203.0.113.10") && o.contains("packets 1 there, 1 back"));
    let o = exec(&mut e, "show policy-map type inspect zone-pair sessions");
    t.check("sessions of all", o.contains("pair IN-DMZ: 0 sessions") && o.contains("pair IN-OUT: 1 sessions"));
    // debug
    let o = exec(&mut e, "debug policy-firewall");
    t.check("debug on", o.contains("Debug output for the firewall: on"));
    let o = exec(&mut e, "show debugging");
    t.check("show debugging", o.contains("the firewall: on"));
    let _ = e.ping(EXT, ip("192.168.1.10"), PingOpts { count: 1, ..PingOpts::default() });
    run_for(&mut e, SEC);
    t.check("the debug line", out(&mut e, FW).contains("FW: icmp 203.0.113.10 -> 192.168.1.10 dropped (OUT-IN, class class-default)"));
    let _ = exec(&mut e, "undebug all");
    // log: the console and the device's log, and with it syslog
    let _ = e.ping(EXT, ip("192.168.1.10"), PingOpts { count: 1, ..PingOpts::default() });
    run_for(&mut e, SEC);
    let printed = out(&mut e, FW);
    let lines = e.log_lines(FW);
    t.check("the first drop was logged above, the next waits", !printed.contains("%FW-6-DROP") || lines.iter().any(|l| l.tag == "%FW-6-DROP"));
    t.check("in the device's log", lines.iter().any(|l| l.tag == "%FW-6-DROP" && l.text.contains("class-default")));
    let _ = pdu(&mut e, EXT, PduProto::Tcp, "192.168.1.10", 49000, 80);
    run_for(&mut e, 100 * MS);
    t.check("inspect logs the start", e.log_lines(FW).iter().any(|l| l.tag == "%FW-6-SESSION_START"));
    t.check("the simulation mode reads it", e.trace.events().any(|x| matches!(&x.note, Note::Fw(n) if n.message().text().contains("Firewall:"))));
    t.check("its layer and mask", e.trace.events().any(|x| matches!(&x.note, Note::Fw(_)) && x.note.layer() == 3 && x.note.mask() != 0));
}

fn problems(t: &mut T) {
    let e = engine("policy-map type inspect UNUSED\n!\nzone security A\n!\nzone security B\n!\nzone-pair security AB source A destination B\n!\ninterface GigabitEthernet0/0\n zone-member security A\n!\ninterface GigabitEthernet0/1\n zone-member security B\n!\n");
    let all = e.problems();
    let of = |k: ProblemKind| all.iter().filter(|p| p.kind == k).count();
    t.eq("interfaces without a zone", of(ProblemKind::FwNoZone), 2);
    t.eq("zones without a pair with a policy", of(ProblemKind::FwNoPair), 2);
    t.eq("a pair without a policy", of(ProblemKind::FwPairNoPolicy), 1);
    t.eq("a policy nobody uses", of(ProblemKind::FwPolicyUnused), 1);
    let e = zoned();
    let all = e.problems();
    t.eq("the three zones are fine except the fourth port", all.iter().filter(|p| matches!(p.kind, ProblemKind::FwNoPair | ProblemKind::FwPairNoPolicy | ProblemKind::FwPolicyUnused)).count(), 0);
    let e = engine("");
    t.check("no problem without zones", !e.problems().iter().any(|p| matches!(p.kind, ProblemKind::FwNoZone | ProblemKind::FwNoPair | ProblemKind::FwPairNoPolicy | ProblemKind::FwPolicyUnused)));
}

// a firewall without zones is the router it was: no note, no line, no help changed for others
fn without_zones(t: &mut T) {
    let mut e = engine("");
    let at = mark(&e);
    t.eq("it forwards", pings(&mut e, PC, "203.0.113.10", 2), 2);
    t.eq("no decision of the firewall", notes(&e, at).len(), 0);
    let run = e.running_config(FW);
    t.check("no line in its configuration", !run.contains("zone") && !run.contains("class-map") && !run.contains("policy-map") && !run.contains("ip inspect"));
    t.check("not active", e.firewall(FW).is_some_and(|f| !f.active()));
    // the same traffic through a router's engine and a firewall's without zones: the same events
    let count = |model: &str| {
        let mut e = Engine::from_project(&project(model, "")).unwrap_or_else(|_| Engine::new(0));
        run_for(&mut e, SEC);
        let _ = e.ping(PC, ip("203.0.113.10"), PingOpts { count: 3, ..PingOpts::default() });
        run_for(&mut e, 5 * SEC);
        (e.trace.events().count(), e.trace.events().filter(|x| matches!(x.note, Note::Fw(_))).count())
    };
    let (a, fa) = count("jf-fw");
    let (b, fb) = count("jf-fw");
    t.eq("the same each time", a, b);
    t.eq("without firewall notes", (fa, fb), (0, 0));
}

// the tab's way of changing things on a document: one undo step, a restart clears the sessions, and
// a view in the past shows what was then and changes nothing
fn window_steps(t: &mut T) {
    let mut n = crate::app::Net::new();
    n.doc.replace(project("jf-fw", &format!("{}{}", ZONES, MEMBERS)), None);
    n.doc.sim.start();
    let _ = n.doc.sim.send(Cmd::RunFor(SEC));
    let _ = n.doc.sim.with_engine("warm", warm);
    let _ = n.doc.sim.with_engine("ping", |e| {
        let _ = e.ping(PC, ip("203.0.113.10"), PingOpts { count: 1, ..PingOpts::default() });
    });
    let _ = n.doc.sim.send(Cmd::RunFor(2 * SEC));
    let then = n.doc.sim.state().now() - SEC;
    let startup = |n: &crate::app::Net| n.doc.proj.device(FW).map(|d| d.startup_config.clone()).unwrap_or_default();
    t.eq("a session in the document", n.doc.sim.state().fw_sessions(FW).len(), 1);
    let first = startup(&n);
    let lines: Vec<String> = alloc::vec![s("zone security LABOR")];
    t.eq("a step of the tab", console::run_lines_running(&mut n, FW, "Zone anlegen", &lines), Ok(()));
    t.check("it is in the startup configuration", startup(&n).contains("zone security LABOR\n"));
    t.check("and running", n.doc.sim.state().firewall(FW).is_some_and(|f| f.zones.contains_key("LABOR")));
    t.eq("the session stays", n.doc.sim.state().fw_sessions(FW).len(), 1);
    t.check("a refusal says why", console::run_lines_running(&mut n, FW, "x", &[s("no zone security DMZ")]).err().is_some_and(|x| x.contains("still has interfaces")));
    // the view in the past
    let _ = n.doc.sim.send(Cmd::RunFor(SEC));
    let _ = n.doc.sim.send(Cmd::Seek(then));
    t.check("in the past", n.doc.sim.past().is_some());
    t.check("the zone was not there yet", n.doc.sim.state().firewall(FW).is_some_and(|f| !f.zones.contains_key("LABOR")));
    t.eq("the session of then", n.doc.sim.state().fw_sessions(FW).len(), 1);
    let _ = n.doc.sim.send(Cmd::Live);
    // undo restarts the device: its sessions are gone
    t.check("undo", n.doc.undo());
    t.eq("the project as it was", startup(&n), first);
    t.check("the zone is gone", n.doc.sim.state().firewall(FW).is_some_and(|f| !f.zones.contains_key("LABOR")));
    t.eq("a reload clears the sessions", n.doc.sim.state().firewall(FW).map(|f| f.sessions().count()), Some(0));
}

// a line on a host's shell: what it printed
fn sh(e: &mut Engine, dev: u32, line: &str) -> String {
    let _ = out(e, dev);
    let _ = e.cli_line(dev, line);
    run_for(e, 10 * MS);
    out(e, dev)
}

// the small firewall of a Linux host
fn host_firewall(t: &mut T) {
    let mut e = engine("");
    t.check("off: TCP reaches the server", tcp_ok(&mut e, PC, "10.0.0.10", 50000, 80));
    t.check("the command", sh(&mut e, WEB, "ufw status").contains("firewall of this host: off"));
    t.check("on", sh(&mut e, WEB, "ufw enable").contains("is on"));
    // incoming: deny without a rule, silently; ping and ICMP errors pass
    let at = mark(&e);
    let _ = pdu(&mut e, PC, PduProto::Tcp, "10.0.0.10", 50001, 80);
    run_for(&mut e, 500 * MS);
    let drop = e.trace.events().skip(at).any(|x| x.dev == WEB && matches!(&x.note, Note::Fw(FwNote::Drop { pair, .. }) if pair == "ufw"));
    t.check("a connection in is dropped", drop);
    t.eq("a ping is answered", pings(&mut e, PC, "10.0.0.10", 1), 1);
    // a rule for it
    t.check("a rule", sh(&mut e, WEB, "ufw allow 80/tcp").contains("added"));
    t.check("the rule lets it in", tcp_ok(&mut e, PC, "10.0.0.10", 50002, 80));
    t.check("with a session", e.firewall(WEB).is_some_and(|f| f.sessions().any(|s| s.sport == 50002 && s.pair == "ufw")));
    t.check("another port stays out", !tcp_ok(&mut e, PC, "10.0.0.10", 50003, 81));
    t.check("a rule's protocol counts", {
        let _ = sh(&mut e, WEB, "ufw reject 8080/udp");
        let at = mark(&e);
        let _ = pdu(&mut e, PC, PduProto::Udp, "10.0.0.10", 50004, 8080);
        run_for(&mut e, 200 * MS);
        e.trace.events().skip(at).any(|x| x.dev == WEB && matches!(&x.note, Note::Fw(FwNote::Reject { .. })))
    });
    // going out: allowed, the answer comes back through the session
    let at = mark(&e);
    t.check("a connection out", tcp_ok(&mut e, WEB, "203.0.113.10", 50005, 80));
    t.check("its session", e.trace.events().skip(at).any(|x| x.dev == WEB && matches!(&x.note, Note::Fw(FwNote::Session { back: true, .. }))));
    let _ = sh(&mut e, WEB, "ufw default deny outgoing");
    t.check("deny outgoing", !tcp_ok(&mut e, WEB, "203.0.113.10", 50006, 80));
    let _ = sh(&mut e, WEB, "ufw allow out 80/tcp");
    t.check("a rule out", tcp_ok(&mut e, WEB, "203.0.113.10", 50007, 80));
    // from a network
    let _ = sh(&mut e, WEB, "ufw allow from 192.168.1.0/24 to any port 22/tcp");
    let shown = sh(&mut e, WEB, "ufw status");
    t.check("status", shown.contains("firewall of this host: on") && shown.contains("incoming deny, outgoing deny") && shown.contains("allow 80/tcp") && shown.contains("reject 8080/udp") && shown.contains("allow out 80/tcp") && shown.contains("allow from 192.168.1.0/24 to any port 22/tcp"));
    t.check("a wrong rule is refused", sh(&mut e, WEB, "ufw allow hugo").contains("no port"));
    t.check("man knows it", sh(&mut e, WEB, "man ufw").contains("die kleine Firewall"));
    // through the device's configuration
    let cfg = e.running_config(WEB);
    t.check("in the configuration", cfg.contains("ufw default deny outgoing\n") && cfg.contains("ufw allow 80/tcp\n") && cfg.contains("ufw enable\n"));
    let mut p = project("jf-fw", "");
    if let Some(d) = p.device_mut(WEB) {
        d.startup_config = cfg.clone();
    }
    let mut e2 = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    run_for(&mut e2, SEC);
    t.eq("it boots to the same", e2.running_config(WEB), cfg);
    t.check("on after booting", e2.firewall(WEB).is_some_and(|f| f.host.enabled && f.active()));
    // a power cycle forgets sessions; off and reset
    let _ = e.apply(jfn_sim::Action::Power { dev: WEB, on: false });
    run_for(&mut e, 100 * MS);
    let _ = e.apply(jfn_sim::Action::Power { dev: WEB, on: true });
    run_for(&mut e, SEC);
    t.check("no session after a power cycle", e.firewall(WEB).is_some_and(|f| f.sessions().next().is_none()));
    let _ = sh(&mut e, WEB, "ufw delete allow 80/tcp");
    t.check("a rule goes", !sh(&mut e, WEB, "ufw status").contains("  1 allow 80/tcp"));
    let _ = sh(&mut e, WEB, "ufw disable");
    t.check("off: not active", e.firewall(WEB).is_some_and(|f| !f.active()));
    let _ = sh(&mut e, WEB, "ufw reset");
    t.eq("reset: as new", e.firewall(WEB).map(|f| f.host.clone()), Some(jfn_sim::fw::HostFw::default()));
    // a router has no such command, and a host without it no note
    let mut e = engine("");
    let at = mark(&e);
    t.check("a host without it is untouched", tcp_ok(&mut e, PC, "10.0.0.10", 50010, 80) && !e.trace.events().skip(at).any(|x| matches!(&x.note, Note::Fw(_))));
    t.check("no IOS device takes the action", e.apply(jfn_sim::Action::Fw { dev: FW, op: jfn_sim::FwOp::Host(jfn_sim::fw::HostOp::Enable(true)) }).is_err());
}
