// nattest -- checks of access lists and NAT in jfn_sim (part of --selftest, group "nat"): the
// matching of every port operator, wildcard and protocol, "established", later fragments, the
// order of lines and the implicit deny; the console (numbered and named lists, sequence
// numbers, remarks, "no", resequence, show access-lists, help, the running configuration read
// back); lists on interfaces with their counters, notes and ICMP answers under the limit, the
// outbound list after NAT and the device's own packets; PAT with several inside hosts and
// colliding ports, identifiers and parity, a pool that runs out, static translations of an
// address and of a port with ARP for the global address, filtering from outside, ICMP queries
// and errors through NAT in both directions (RFC 5508), fragments in and out of order,
// timeouts, clearing, power, debug ip nat, the notes in German and English, and the
// checksums of every frame
//
// The network (device ids in order): PC1 (192.168.1.10), PC2 (.11), WEB (.20, TCP echo on
// port 80) and PC3 (.30) on SW1; R1 between 192.168.1.0/24 (inside, Gi0/0/0) and
// 203.0.113.0/24 (outside, Gi0/0/1: PAT on its address 203.0.113.1, WEB:80 forwarded from
// 203.0.113.1:80, PC3 statically 203.0.113.30); ISP between 203.0.113.0/24 (.254),
// 198.51.100.0/24 (SRV .10, TCP echo on port 7) and 192.0.2.0/24 (EXT .20).  ISP knows no
// route to 192.168.1.0/24: only translated packets find their way back.  1 Gbit/s copper with
// 50 ns of cable: a 98-byte echo frame takes 1026 ns per cable, so a ping from PC1 to SRV over
// four cables and back takes 8208 ns once ARP is done -- NAT itself costs no time.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::acl::{Ace, Acl, AclKind, Item, Ports, Wild};
use jfn_sim::node::port_mac;
use jfn_sim::{Action, Engine, IcmpKind, JobKind, Note, PduProto, PduSender, PduSpec, PingEv, PingOpts, Report, TraceOpts, MS, SEC};
use netpkt::checksum;
use netpkt::decode;
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::icmp::{self, Icmp, IcmpView};
use netpkt::ipv4::{self, Ipv4Hdr, Ipv4View, Reassembly, PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use netpkt::nat::Flow;
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::tcp::{ACK, RST, SYN};
use netpkt::udp::UdpView;
use netpkt::Ipv4;

use crate::selftest::T;

const BUDGET: usize = 4_000_000;

const PC1: u32 = 1;
const PC2: u32 = 2;
const WEB: u32 = 3;
const PC3: u32 = 4;
const R1: u32 = 6;
const ISP: u32 = 7;
const SRV: u32 = 8;
const EXT: u32 = 9;

const R1_CFG: &str = "hostname R1\n!\ninterface GigabitEthernet0/0/0\n ip address 192.168.1.1 255.255.255.0\n ip nat inside\n no shutdown\n!\ninterface GigabitEthernet0/0/1\n ip address 203.0.113.1 255.255.255.0\n ip nat outside\n no shutdown\n!\nip nat inside source list 1 interface GigabitEthernet0/0/1 overload\nip nat inside source static tcp 192.168.1.20 80 203.0.113.1 80\nip nat inside source static 192.168.1.30 203.0.113.30\nip route 0.0.0.0 0.0.0.0 203.0.113.254\n!\naccess-list 1 permit 192.168.1.0 0.0.0.255\n!\nend\n";
const ISP_CFG: &str = "hostname ISP\n!\ninterface GigabitEthernet0/0/0\n ip address 203.0.113.254 255.255.255.0\n no shutdown\n!\ninterface GigabitEthernet0/0/1\n ip address 198.51.100.1 255.255.255.0\n no shutdown\n!\ninterface GigabitEthernet0/0/2\n ip address 192.0.2.1 255.255.255.0\n no shutdown\n!\nend\n";

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn ip(s: &str) -> Ipv4 {
    Ipv4::parse(s).unwrap_or(Ipv4::UNSPECIFIED)
}

fn s(x: &str) -> String {
    String::from(x)
}

fn echo_svc(port: u16) -> ServiceDef {
    let cfg = format!("{{\"port\": {}}}", port);
    ServiceDef { kind: s("tcp-echo"), name: s("echo"), disabled: false, config: agent_core::json::parse(cfg.as_bytes()).unwrap_or(Value::Null) }
}

/// the office behind R1 and the outside behind ISP
fn office() -> Project {
    let mut p = Project::new("nat", 31);
    let models = [("pc", "PC1"), ("pc", "PC2"), ("server-1u", "WEB"), ("pc", "PC3"), ("jf-s24p", "SW1"), ("jf-r4", "R1"), ("jf-r4", "ISP"), ("server-1u", "SRV"), ("pc", "EXT")];
    for (i, (m, _)) in models.iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 100, 0);
    }
    let _ = p.connect(&pr(PC1, "eth0"), &pr(5, "gi1/0/1"), None);
    let _ = p.connect(&pr(PC2, "eth0"), &pr(5, "gi1/0/2"), None);
    let _ = p.connect(&pr(WEB, "eth0"), &pr(5, "gi1/0/3"), None);
    let _ = p.connect(&pr(PC3, "eth0"), &pr(5, "gi1/0/4"), None);
    let _ = p.connect(&pr(5, "gi1/0/24"), &pr(R1, "gi0/0/0"), None);
    let _ = p.connect(&pr(R1, "gi0/0/1"), &pr(ISP, "gi0/0/0"), None);
    let _ = p.connect(&pr(ISP, "gi0/0/1"), &pr(SRV, "eth0"), None);
    let _ = p.connect(&pr(ISP, "gi0/0/2"), &pr(EXT, "eth0"), None);
    let host = |name: &str, a: &str, gw: &str| format!("hostname {}\nip addr add {} dev eth0\nip route add default via {}\n", name, a, gw);
    let cfgs = [
        (PC1, host("PC1", "192.168.1.10/24", "192.168.1.1")),
        (PC2, host("PC2", "192.168.1.11/24", "192.168.1.1")),
        (WEB, host("WEB", "192.168.1.20/24", "192.168.1.1")),
        (PC3, host("PC3", "192.168.1.30/24", "192.168.1.1")),
        (5, s("hostname SW1\n!\nend\n")),
        (R1, s(R1_CFG)),
        (ISP, s(ISP_CFG)),
        (SRV, host("SRV", "198.51.100.10/24", "198.51.100.1")),
        (EXT, host("EXT", "192.0.2.20/24", "192.0.2.1")),
    ];
    for (id, c) in cfgs {
        if let Some(d) = p.device_mut(id) {
            d.name = String::from(models[id as usize - 1].1);
            d.startup_config = c;
        }
    }
    if let Some(d) = p.device_mut(WEB) {
        d.services.push(echo_svc(80));
    }
    if let Some(d) = p.device_mut(SRV) {
        d.services.push(echo_svc(7));
    }
    p
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
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

/// a line at a console, then 100 ms: what it printed
fn cli(e: &mut Engine, dev: u32, line: &str) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    let t = e.now() + 100 * MS;
    e.run_until(t, BUDGET);
    out(e, dev)
}

fn lines(e: &mut Engine, dev: u32, ls: &[&str]) -> String {
    let mut o = String::new();
    for l in ls {
        o.push_str(&cli(e, dev, l));
    }
    o
}

/// enable and configure terminal on R1, then the lines, then end
fn conf(e: &mut Engine, dev: u32, ls: &[&str]) -> String {
    let _ = lines(e, dev, &["enable", "configure terminal"]);
    let o = lines(e, dev, ls);
    let _ = cli(e, dev, "end");
    o
}

fn ping_once(e: &mut Engine, dev: u32, dst: &str) -> u32 {
    e.ping(dev, ip(dst), PingOpts { count: 1, ..PingOpts::default() }).unwrap_or(0)
}

/// three pings from PC1 to SRV teach R1 and ISP their neighbours (IOS drops the packet it asks
/// ARP for); the clock then stands 4 s later
fn warm(e: &mut Engine) -> u32 {
    let id = e.ping(PC1, ip("198.51.100.10"), PingOpts { count: 3, ..PingOpts::default() }).unwrap_or(0);
    let t = e.now() + 4 * SEC;
    e.run_until(t, BUDGET);
    id
}

fn ping_ev(e: &Engine, id: u32) -> Vec<PingEv> {
    match e.job(id).map(|j| &j.kind) {
        Some(JobKind::Ping(p)) => p.events.clone(),
        _ => Vec::new(),
    }
}

fn ident(e: &Engine, id: u32) -> u16 {
    match e.job(id).map(|j| &j.kind) {
        Some(JobKind::Ping(p)) => p.ident,
        _ => 0,
    }
}

/// the notes of a device with their times
fn notes(e: &Engine, dev: u32) -> Vec<(u64, Note)> {
    e.trace.events().filter(|x| x.dev == dev).map(|x| (x.t, x.note.clone())).collect()
}

fn has_note(e: &Engine, dev: u32, f: impl Fn(&Note) -> bool) -> bool {
    e.trace.events().any(|x| x.dev == dev && f(&x.note))
}

/// the IPv4 packets that went from one device to another (Ethernet frames)
fn packets(e: &Engine, from: u32, to: u32) -> Vec<(u64, Vec<u8>)> {
    e.trace.hops().filter(|h| h.from.0 == from && h.to.0 == to && h.bytes.len() > 14 && h.bytes[12..14] == [0x08, 0x00]).filter_map(|h| Ipv4View::parse(&h.bytes[14..]).map(|v| (h.t_tx, v.packet().to_vec()))).collect()
}

/// the flows of the packets between two devices
fn flows(e: &Engine, from: u32, to: u32) -> Vec<Flow> {
    packets(e, from, to).iter().filter_map(|p| Flow::of(&p.1)).collect()
}

/// every checksum of an unfragmented IPv4 packet holds (header, TCP / UDP / ICMP, and the
/// quote of an ICMP error: its header and a UDP checksum it holds)
fn sums_ok(p: &[u8]) -> bool {
    let Some(v) = Ipv4View::parse(p) else {
        return false;
    };
    if !v.checksum_ok() {
        return false;
    }
    if v.is_fragment() {
        return true;
    }
    let pl = v.payload();
    match v.proto() {
        PROTO_TCP => netpkt::tcp::TcpView::parse(pl).is_some_and(|t| t.checksum_ok_v4(v.src(), v.dst())),
        PROTO_UDP => UdpView::parse(pl).is_some_and(|u| u.checksum_ok_v4(v.src(), v.dst())),
        PROTO_ICMP => {
            let Some(ic) = IcmpView::parse(pl) else {
                return false;
            };
            if !ic.checksum_ok() {
                return false;
            }
            match ic.quoted() {
                Some(q) => {
                    let inner_ok = checksum::verify(q.header());
                    let qp = q.payload();
                    // a quoted UDP header of 8 bytes holds its checksum, over the whole datagram:
                    // check it when the quote has all of it
                    let udp_ok = q.proto() != PROTO_UDP || qp.len() < 8 || q.is_truncated() || UdpView::parse(qp).is_some_and(|u| u.checksum_ok_v4(q.src(), q.dst()));
                    inner_ok && udp_ok
                }
                None => true,
            }
        }
        _ => true,
    }
}

/// every frame on every cable decodes without a problem (whole packets: every checksum)
fn all_frames_ok(e: &Engine) -> (usize, usize) {
    let mut n = 0;
    let mut bad = 0;
    for h in e.trace.hops() {
        if h.bytes.len() <= 14 || h.bytes[12..14] != [0x08, 0x00] {
            continue;
        }
        n += 1;
        let pkt = Ipv4View::parse(&h.bytes[14..]).map(|v| v.packet().to_vec()).unwrap_or_default();
        let frag = Ipv4View::parse(&pkt).is_some_and(|v| v.is_fragment());
        let decoded = frag || decode::layers(&h.bytes).iter().all(|l| l.problems.is_empty());
        if !(decoded && sums_ok(&pkt)) {
            bad += 1;
        }
    }
    (n, bad)
}

/// an Ethernet frame with an IPv4 packet
fn frame(dst: netpkt::Mac, src: netpkt::Mac, pkt: &[u8]) -> Vec<u8> {
    let mut f = EthHdr { dst, src, ethertype: ET_IPV4 }.frame(pkt);
    netpkt::eth::pad(&mut f);
    f
}

/// an ICMP echo packet
fn echo(src: &str, dst: &str, reply: bool, id: u16, seq: u16, data: &[u8], ipid: u16, ttl: u8) -> Vec<u8> {
    let mut p = Pkt::new(HEADROOM, data);
    if reply {
        Icmp::EchoReply { id, seq }.push_onto(&mut p);
    } else {
        Icmp::EchoRequest { id, seq }.push_onto(&mut p);
    }
    let mut h = Ipv4Hdr::new(ip(src), ip(dst), PROTO_ICMP, ttl);
    h.id = ipid;
    h.push_onto(&mut p);
    p.into_vec()
}

fn udp_pkt(src: &str, sport: u16, dst: &str, dport: u16, ttl: u8) -> Vec<u8> {
    let mut p = Pkt::new(HEADROOM, b"hello");
    netpkt::udp::UdpHdr::new(sport, dport).push_v4(&mut p, ip(src), ip(dst));
    Ipv4Hdr::new(ip(src), ip(dst), PROTO_UDP, ttl).push_onto(&mut p);
    p.into_vec()
}

fn pdu(e: &mut Engine, dev: u32, name: &str, proto: PduProto, dst: &str, sport: u16, dport: u16) -> bool {
    let spec = PduSpec { proto, dst: s(dst), sport, dport, size: 20, ttl: None, count: 1, interval: MS };
    e.add_service(dev, name, Box::new(PduSender::new(spec)), true).is_ok()
}

fn sender(e: &Engine, dev: u32, name: &str) -> Option<(u32, u32, u32)> {
    e.service::<PduSender>(dev, name).map(|x| (x.sent, x.replies, x.failed))
}

pub fn run(t: &mut T) {
    matching(t);
    list_lines(t);
    console(t);
    acl_traffic(t);
    outbound_after_nat(t);
    established(t);
    pat(t);
    statics(t);
    pool(t);
    icmp_errors(t);
    fragments(t);
    timeouts(t);
    clear_power_debug(t);
    texts(t);
    other_devices(t);
    determinism(t);
}

// lists on the VLAN interface of an L3 switch and on a subinterface; the firewall's console
fn other_devices(t: &mut T) {
    let mut p = Project::new("svi", 32);
    let a = p.add_device("pc", 0, 0).unwrap_or(0);
    let b = p.add_device("pc", 0, 100).unwrap_or(0);
    let mls = p.add_device("jf-s24l3", 100, 50).unwrap_or(0);
    let fw = p.add_device("jf-fw", 200, 50).unwrap_or(0);
    let r = p.add_device("jf-r4", 300, 50).unwrap_or(0);
    let _ = p.connect(&pr(a, "eth0"), &pr(mls, "gi1/0/1"), None);
    let _ = p.connect(&pr(b, "eth0"), &pr(mls, "gi1/0/2"), None);
    let mls_cfg = "hostname MLS1\n!\nip routing\n!\nvlan 10\n!\nvlan 20\n!\ninterface GigabitEthernet1/0/1\n switchport access vlan 10\n switchport mode access\n!\ninterface GigabitEthernet1/0/2\n switchport access vlan 20\n switchport mode access\n!\ninterface Vlan10\n ip address 10.0.10.1 255.255.255.0\n ip access-group 100 in\n!\ninterface Vlan20\n ip address 10.0.20.1 255.255.255.0\n!\naccess-list 100 deny icmp host 10.0.10.10 any echo\naccess-list 100 permit ip any any\n!\nend\n";
    let fw_cfg = "hostname FW1\n!\ninterface GigabitEthernet0/0\n ip address 10.1.0.1 255.255.255.0\n ip access-group INSIDE in\n ip nat inside\n no shutdown\n!\ninterface GigabitEthernet0/1\n ip address 198.18.0.1 255.255.255.0\n ip nat outside\n no shutdown\n!\nip nat pool OUT 198.18.0.10 198.18.0.20 prefix-length 24\nip nat inside source list 2 pool OUT overload\n!\nip access-list extended INSIDE\n permit tcp 10.1.0.0 0.0.0.255 any eq www\n deny ip any any log\n!\naccess-list 2 permit 10.1.0.0 0.0.0.255\n!\nend\n";
    let r_cfg = "hostname R9\n!\ninterface GigabitEthernet0/0/2\n no shutdown\n!\ninterface GigabitEthernet0/0/2.10\n encapsulation dot1Q 10\n ip address 10.9.10.1 255.255.255.0\n ip access-group 101 out\n!\naccess-list 101 permit ip any any\n!\nend\n";
    for (id, c) in [(a, s("hostname PCA\nip addr add 10.0.10.10/24 dev eth0\nip route add default via 10.0.10.1\n")), (b, s("hostname PCB\nip addr add 10.0.20.10/24 dev eth0\nip route add default via 10.0.20.1\n")), (mls, s(mls_cfg)), (fw, s(fw_cfg)), (r, s(r_cfg))] {
        if let Some(d) = p.device_mut(id) {
            d.startup_config = c;
        }
    }
    let mut e = engine(&p);
    t.eq("devices: boot errors", (e.boot_errors(mls).len(), e.boot_errors(fw).len(), e.boot_errors(r).len()), (0, 0, 0));
    t.eq("devices: svi bound", e.access_groups(mls), vec![(s("Vlan10"), false, s("100"))]);
    t.eq("devices: subinterface bound", e.access_groups(r), vec![(s("GigabitEthernet0/0/2.10"), true, s("101"))]);
    let id = e.ping(a, ip("10.0.20.10"), PingOpts { count: 1, ..PingOpts::default() }).unwrap_or(0);
    e.run_until(3 * SEC, BUDGET);
    t.eq("devices: denied at the svi", ping_ev(&e, id).first().cloned(), Some(PingEv::Error { seq: 1, from: ip("10.0.10.1"), kind: IcmpKind::Prohibited }));
    let id = e.ping(b, ip("10.0.10.10"), PingOpts { count: 2, ..PingOpts::default() }).unwrap_or(0);
    e.run_until(e.now() + 3 * SEC, BUDGET);
    t.check("devices: the other way", ping_ev(&e, id).iter().any(|x| matches!(x, PingEv::Reply { .. })));
    // the firewall takes the configuration and writes it back the same
    let run = e.running_config(fw);
    t.check("devices: firewall nat", run.contains("ip nat pool OUT 198.18.0.10 198.18.0.20 netmask 255.255.255.0\nip nat inside source list 2 pool OUT overload\n"));
    t.check("devices: firewall acl", run.contains("ip access-list extended INSIDE\n permit tcp 10.1.0.0 0.0.0.255 any eq www\n deny ip any any log\n!\naccess-list 2 permit 10.1.0.0 0.0.0.255\n!\n"));
    t.check("devices: firewall interface", run.contains(" ip address 10.1.0.1 255.255.255.0\n ip access-group INSIDE in\n ip nat inside\n"));
    // a switch that does not route has access lists, but no NAT
    let mut q = Project::new("l2", 33);
    let sw = q.add_device("jf-s24p", 0, 0).unwrap_or(0);
    let mut e2 = engine(&q);
    let o = conf(&mut e2, sw, &["access-list 5 permit host 10.0.0.9", "interface Vlan1", "ip access-group 5 in", "ip nat inside"]);
    t.check("devices: no nat on a layer-2 switch", o.contains("% Invalid input detected at '^' marker.") && e2.access_groups(sw) == vec![(s("Vlan1"), false, s("5"))]);
}

// ---------------------------------------------------------------- matching

fn flow(proto: u8, src: &str, sport: u16, dst: &str, dport: u16) -> Flow {
    Flow { proto, src: ip(src), dst: ip(dst), sport, dport, ports: true, icmp: None, flags: SYN, later: false, mf: false, id: 1 }
}

fn ext(permit: bool, proto: Option<u8>, src: Wild, sport: Ports, dst: Wild, dport: Ports) -> Ace {
    Ace { permit, proto, src, sport, dst, dport, established: false, icmp: None, log: false }
}

fn matching(t: &mut T) {
    let any = Wild::ANY;
    let w = |a: &str, m: &str| Wild::new(ip(a), ip(m));
    // wildcards: the address loses the free bits; non-contiguous masks
    t.eq("wild normalised", w("10.1.2.3", "0.0.0.255").addr, ip("10.1.2.0"));
    t.check("wild any", any.matches(ip("1.2.3.4")) && any.is_any());
    t.check("wild host", Wild::host(ip("10.0.0.5")).matches(ip("10.0.0.5")) && !Wild::host(ip("10.0.0.5")).matches(ip("10.0.0.6")));
    let odd = w("10.1.0.3", "0.0.255.0");
    t.check("wild non-contiguous", odd.matches(ip("10.1.77.3")) && !odd.matches(ip("10.1.77.4")) && !odd.matches(ip("10.2.77.3")));
    let f = flow(PROTO_TCP, "10.1.2.3", 40000, "10.9.0.1", 80);
    // every port operator
    let port = |p: Ports| ext(true, Some(PROTO_TCP), any, Ports::Any, any, p).matches(&f);
    t.eq("eq", (port(Ports::Eq(80)), port(Ports::Eq(81))), (Some(true), None));
    t.eq("neq", (port(Ports::Neq(81)), port(Ports::Neq(80))), (Some(true), None));
    t.eq("gt", (port(Ports::Gt(79)), port(Ports::Gt(80))), (Some(true), None));
    t.eq("lt", (port(Ports::Lt(81)), port(Ports::Lt(80))), (Some(true), None));
    t.eq("range", (port(Ports::Range(80, 80)), port(Ports::Range(1, 79)), port(Ports::Range(81, 65535))), (Some(true), None, None));
    let sp = |p: Ports| ext(false, Some(PROTO_TCP), any, p, any, Ports::Any).matches(&f);
    t.eq("source port", (sp(Ports::Gt(1023)), sp(Ports::Lt(1024))), (Some(false), None));
    // protocols
    t.eq("ip matches tcp", ext(true, None, any, Ports::Any, any, Ports::Any).matches(&f), Some(true));
    t.eq("udp is not tcp", ext(true, Some(PROTO_UDP), any, Ports::Any, any, Ports::Any).matches(&f), None);
    let gre = Flow { proto: 47, ports: false, ..f };
    t.eq("gre by number", (ext(true, Some(47), any, Ports::Any, any, Ports::Any).matches(&gre), ext(true, Some(50), any, Ports::Any, any, Ports::Any).matches(&gre)), (Some(true), None));
    t.eq("addresses", (ext(true, None, w("10.1.0.0", "0.0.255.255"), Ports::Any, Wild::host(ip("10.9.0.1")), Ports::Any).matches(&f), ext(true, None, w("10.2.0.0", "0.0.255.255"), Ports::Any, any, Ports::Any).matches(&f)), (Some(true), None));
    // established: ACK or RST
    let est = Ace { established: true, ..ext(true, Some(PROTO_TCP), any, Ports::Any, any, Ports::Any) };
    t.eq("established", (est.matches(&f), est.matches(&Flow { flags: SYN | ACK, ..f }), est.matches(&Flow { flags: RST, ..f }), est.matches(&Flow { proto: PROTO_UDP, flags: ACK, ..f })), (None, Some(true), Some(true), None));
    // ICMP types and codes
    let ping = Flow { proto: PROTO_ICMP, icmp: Some((8, 0)), ..f };
    let prohibited = Flow { proto: PROTO_ICMP, icmp: Some((3, 13)), ports: false, ..f };
    let ic = |ty: u8, c: Option<u8>, x: &Flow| Ace { icmp: Some((ty, c)), ..ext(true, Some(PROTO_ICMP), any, Ports::Any, any, Ports::Any) }.matches(x);
    t.eq("icmp echo", (ic(8, None, &ping), ic(0, None, &ping)), (Some(true), None));
    t.eq("icmp code", (ic(3, Some(13), &prohibited), ic(3, Some(1), &prohibited), ic(3, None, &prohibited)), (Some(true), None, Some(true)));
    // a later fragment: a permit rule with ports lets it through, a deny rule with ports is skipped
    let later = Flow { later: true, ports: false, sport: 0, dport: 0, ..f };
    t.eq("later fragment, permit with ports", ext(true, Some(PROTO_TCP), any, Ports::Any, any, Ports::Eq(80)).matches(&later), Some(true));
    t.eq("later fragment, deny with ports", ext(false, Some(PROTO_TCP), any, Ports::Any, any, Ports::Eq(80)).matches(&later), None);
    t.eq("later fragment, deny without ports", ext(false, Some(PROTO_TCP), any, Ports::Any, any, Ports::Any).matches(&later), Some(false));
    // the first matching line decides; nothing: the implicit deny (seq 0)
    let mut l = Acl::new(AclKind::Extended, true);
    let _ = l.add(None, Item::Rule(ext(false, Some(PROTO_TCP), any, Ports::Any, any, Ports::Eq(23))));
    let _ = l.add(None, Item::Rule(ext(true, Some(PROTO_TCP), any, Ports::Any, any, Ports::Any)));
    t.eq("first match", l.check(&flow(PROTO_TCP, "1.1.1.1", 1, "2.2.2.2", 23)).0.seq, 10);
    t.eq("second line", (l.check(&f).0.permit, l.check(&f).0.seq), (true, 20));
    let u = flow(PROTO_UDP, "1.1.1.1", 1, "2.2.2.2", 53);
    t.eq("implicit deny", (l.check(&u).0.permit, l.check(&u).0.seq, l.check(&u).1), (false, 0, None));
    // standard rules look at the source only
    let st = Ace::standard(true, w("192.168.0.0", "0.0.255.255"));
    t.eq("standard", (st.matches(&flow(PROTO_UDP, "192.168.4.4", 1, "8.8.8.8", 53)), st.matches(&flow(PROTO_UDP, "10.0.0.1", 1, "192.168.1.1", 53))), (Some(true), None));
    // the texts of rules as IOS writes them
    let e1 = ext(true, Some(PROTO_TCP), any, Ports::Any, Wild::host(ip("10.0.0.5")), Ports::Eq(80));
    t.eq("config www", e1.config(AclKind::Extended), s("permit tcp any host 10.0.0.5 eq www"));
    let e2 = ext(false, Some(PROTO_UDP), w("10.1.0.0", "0.0.255.255"), Ports::Range(1024, 65535), any, Ports::Eq(53));
    t.eq("config range domain", e2.config(AclKind::Extended), s("deny udp 10.1.0.0 0.0.255.255 range 1024 65535 any eq domain"));
    t.eq("config echo-reply", Ace { icmp: Some((0, None)), ..ext(true, Some(PROTO_ICMP), any, Ports::Any, any, Ports::Any) }.config(AclKind::Extended), s("permit icmp any any echo-reply"));
    t.eq("config icmp numbers", Ace { icmp: Some((3, Some(9))), ..ext(true, Some(PROTO_ICMP), any, Ports::Any, any, Ports::Any) }.config(AclKind::Extended), s("permit icmp any any 3 9"));
    t.eq("config established log", Ace { established: true, log: true, ..ext(true, Some(PROTO_TCP), any, Ports::Gt(1023), any, Ports::Any) }.config(AclKind::Extended), s("permit tcp any gt 1023 any established log"));
    t.eq("config protocol names", (ext(true, Some(47), any, Ports::Any, any, Ports::Any).config(AclKind::Extended), ext(true, Some(112), any, Ports::Any, any, Ports::Any).config(AclKind::Extended)), (s("permit gre any any"), s("permit 112 any any")));
    t.eq("show standard", (Ace::standard(false, w("10.0.0.0", "0.0.0.255")).show(AclKind::Standard), Ace::standard(true, Wild::host(ip("10.0.0.1"))).show(AclKind::Standard), Ace::standard(true, any).config(AclKind::Standard)), (s("deny 10.0.0.0, wildcard bits 0.0.0.255"), s("permit 10.0.0.1"), s("permit any")));
}

// sequence numbers, remarks, removing and renumbering lines
fn list_lines(t: &mut T) {
    let r = |p: bool, port: u16| Item::Rule(ext(p, Some(PROTO_TCP), Wild::ANY, Ports::Any, Wild::ANY, Ports::Eq(port)));
    let mut l = Acl::new(AclKind::Extended, false);
    let _ = l.add(None, r(true, 80));
    let _ = l.add(None, Item::Remark(s("mail")));
    let _ = l.add(None, r(true, 25));
    let _ = l.add(Some(15), r(false, 23));
    let seqs = |l: &Acl| l.lines.iter().map(|x| (x.seq, matches!(x.item, Item::Remark(_)))).collect::<Vec<_>>();
    // a line with a number goes before the remark of the next one
    t.eq("insert before the remark", seqs(&l), vec![(10, false), (15, false), (0, true), (20, false)]);
    t.check("duplicate number", l.add(Some(15), r(true, 22)).is_err());
    let before = l.lines.len();
    let _ = l.add(None, r(true, 80));
    t.eq("an equal line is not added twice", l.lines.len(), before);
    t.check("remove by number", l.remove_seq(15) && !l.remove_seq(15));
    t.check("remove by content", l.remove_item(&r(true, 25)) && l.lines.len() == 2);
    let _ = l.add(None, r(true, 443));
    t.check("resequence", l.resequence(100, 5));
    t.eq("renumbered", seqs(&l), vec![(100, false), (0, true), (105, false)]);
}

// ---------------------------------------------------------------- the console

fn console(t: &mut T) {
    let mut e = engine(&office());
    t.eq("console: boot errors", e.boot_errors(R1).len(), 0);
    let o = conf(
        &mut e,
        R1,
        &[
            "access-list 101 deny tcp host 192.168.1.11 any eq telnet",
            "access-list 101 permit icmp any any echo",
            "access-list 101 remark the rest",
            "access-list 101 permit ip any any",
            "access-list 10 permit 192.168.1.0 0.0.0.255",
            "access-list 10 deny any log",
            "ip access-list extended WEB",
            "permit tcp any any eq www",
            "deny udp any range 1 1023 any log",
            "15 permit tcp any host 203.0.113.1 eq 443 established",
            "remark the end",
            "exit",
            "ip access-list standard MGMT",
            "permit host 192.168.1.10",
            "deny any",
        ],
    );
    t.eq("console: no complaints", o, String::new());
    let show = cli(&mut e, R1, "show access-lists");
    t.eq(
        "show access-lists",
        show,
        s("Standard IP access list 1\n    10 permit 192.168.1.0, wildcard bits 0.0.0.255\nStandard IP access list 10\n    10 permit 192.168.1.0, wildcard bits 0.0.0.255\n    20 deny any log\nExtended IP access list 101\n    10 deny tcp host 192.168.1.11 any eq telnet\n    20 permit icmp any any echo\n    30 permit ip any any\nStandard IP access list MGMT\n    10 permit 192.168.1.10\n    20 deny any\nExtended IP access list WEB\n    10 permit tcp any any eq www\n    15 permit tcp any host 203.0.113.1 eq 443 established\n    20 deny udp any range 1 1023 any log\n"),
    );
    t.eq("show one list", cli(&mut e, R1, "show ip access-lists MGMT"), s("Standard IP access list MGMT\n    10 permit 192.168.1.10\n    20 deny any\n"));
    let run = e.running_config(R1);
    t.check("running config: named", run.contains("!\nip access-list standard MGMT\n permit 192.168.1.10\n deny any\n!\nip access-list extended WEB\n permit tcp any any eq www\n permit tcp any host 203.0.113.1 eq 443 established\n deny udp any range 1 1023 any log\n remark the end\n!\naccess-list 1 permit"));
    t.check("running config: numbered", run.contains("access-list 10 permit 192.168.1.0 0.0.0.255\naccess-list 10 deny any log\naccess-list 101 deny tcp host 192.168.1.11 any eq telnet\naccess-list 101 permit icmp any any echo\naccess-list 101 remark the rest\naccess-list 101 permit ip any any\n!\nline con 0\n"));
    t.check("running config: nat before the routes", run.contains("!\nip nat inside source list 1 interface GigabitEthernet0/0/1 overload\nip nat inside source static tcp 192.168.1.20 80 203.0.113.1 80\nip nat inside source static 192.168.1.30 203.0.113.30\n!\nip route 0.0.0.0 0.0.0.0 203.0.113.254\n"));
    t.check("running config: interfaces", run.contains("interface GigabitEthernet0/0/0\n ip address 192.168.1.1 255.255.255.0\n ip nat inside\n!\n") && run.contains(" ip address 203.0.113.1 255.255.255.0\n ip nat outside\n!\n"));
    // the modes and their prompts
    let _ = lines(&mut e, R1, &["configure terminal", "ip access-list extended WEB"]);
    t.eq("prompt ext", e.cli_prompt(R1), s("R1(config-ext-nacl)#"));
    t.eq("duplicate sequence number", cli(&mut e, R1, "10 permit tcp any any eq 22"), s("% Duplicate sequence number\n"));
    let _ = cli(&mut e, R1, "no 15");
    let _ = cli(&mut e, R1, "no deny udp any range 1 1023 any log");
    t.check("help after eq", {
        let h = e.cli_help(R1, "permit tcp any any eq ");
        h.contains("  <0-65535>") && h.contains("  www ") && h.contains("  telnet ")
    });
    t.eq("tab", e.cli_complete(R1, "perm").0, Some(s("permit ")));
    // a global command falls back to the global configuration
    let _ = cli(&mut e, R1, "ip access-list standard MGMT");
    t.eq("prompt std", e.cli_prompt(R1), s("R1(config-std-nacl)#"));
    t.check("extended syntax refused in a standard list", cli(&mut e, R1, "permit tcp any any").contains("% Invalid input detected at '^' marker."));
    let _ = cli(&mut e, R1, "interface GigabitEthernet0/0/0");
    t.eq("fallback", e.cli_prompt(R1), s("R1(config-if)#"));
    let _ = cli(&mut e, R1, "exit");
    t.eq("wrong kind", cli(&mut e, R1, "ip access-list extended MGMT"), s("% MGMT is already a standard access list\n"));
    t.eq("number of the other kind", cli(&mut e, R1, "ip access-list standard 101"), s("% 101 is no number of a standard list\n"));
    let _ = cli(&mut e, R1, "ip access-list resequence WEB 100 5");
    let _ = cli(&mut e, R1, "end");
    t.eq("after no and resequence", cli(&mut e, R1, "show access-lists WEB"), s("Extended IP access list WEB\n    100 permit tcp any any eq www\n"));
    // "no access-list N ..." removes the whole list, as on the original
    let _ = conf(&mut e, R1, &["no access-list 101 permit ip any any"]);
    t.check("no access-list removes the list", !cli(&mut e, R1, "show access-lists").contains("list 101"));
    // the running configuration read back gives the same configuration
    let run = e.running_config(R1);
    let mut p = office();
    if let Some(d) = p.device_mut(R1) {
        d.startup_config = run.clone();
    }
    let e2 = engine(&p);
    t.eq("read back: boot errors", e2.boot_errors(R1).len(), 0);
    t.eq("read back: same configuration", e2.running_config(R1), run);
}

// ---------------------------------------------------------------- lists on interfaces

fn acl_traffic(t: &mut T) {
    let mut e = engine(&office());
    let _ = warm(&mut e);
    let o = conf(&mut e, R1, &["access-list 110 deny icmp host 192.168.1.11 any echo", "access-list 110 permit ip any any", "interface GigabitEthernet0/0/0", "ip access-group 110 in"]);
    t.check("acl: set-up", !o.contains('%'));
    // PC2's echoes stop at R1: "administratively prohibited" from R1's inside address, once in
    // 500 ms (probes every 200 ms: seq 1 and 4 hear of it)
    let id = e.ping(PC2, ip("198.51.100.10"), PingOpts { count: 4, interval: 200 * MS, timeout: SEC, ..PingOpts::default() }).unwrap_or(0);
    e.run_until(e.now() + 3 * SEC, BUDGET);
    let ev = ping_ev(&e, id);
    let errors: Vec<(u16, Ipv4, IcmpKind)> = ev.iter().filter_map(|x| if let PingEv::Error { seq, from, kind } = x { Some((*seq, *from, *kind)) } else { None }).collect();
    t.eq("acl: prohibited, limited", errors, vec![(1, ip("192.168.1.1"), IcmpKind::Prohibited), (4, ip("192.168.1.1"), IcmpKind::Prohibited)]);
    t.eq("acl: deny notes", notes(&e, R1).iter().filter(|n| matches!(&n.1, Note::AclDeny { list, seq: 10, iface: 0, out: false } if list == "110")).count(), 4);
    t.eq("acl: limited twice", notes(&e, R1).iter().filter(|n| n.1 == Note::IcmpLimited).count(), 2);
    let pc2_id = ident(&e, id);
    t.eq("acl: nothing of PC2 left R1", flows(&e, R1, ISP).iter().filter(|f| f.proto == PROTO_ICMP && f.sport == pc2_id).count(), 0);
    // the answer quotes PC2's packet and leaves 0 ns after the echo arrived at R1
    let deny_t = notes(&e, R1).iter().find(|n| matches!(n.1, Note::AclDeny { .. })).map(|n| n.0).unwrap_or(0);
    let answer = packets(&e, R1, 5).into_iter().find(|p| Flow::of(&p.1).is_some_and(|f| f.icmp == Some((3, 13))));
    t.check("acl: answered at once", answer.as_ref().is_some_and(|a| a.0 == deny_t));
    // 70 bytes (the IOS quote: header and 8 bytes) take 802 ns per cable to PC2
    let at_pc2 = e.trace.hops().find(|h| h.to.0 == PC2 && h.bytes.len() > 34 && h.bytes[12..14] == [0x08, 0x00] && h.bytes[23] == PROTO_ICMP && h.bytes[34] == 3).map(|h| (h.bytes.len(), h.t_rx));
    t.eq("acl: the answer at PC2", at_pc2, Some((70, deny_t + 2 * 802)));
    t.check("acl: the answer's checksums", answer.as_ref().is_some_and(|a| sums_ok(&a.1)));
    t.eq("acl: quote", answer.and_then(|a| Flow::quoted(&a.1)).map(|q| (q.src, q.dst, q.icmp)), Some((ip("192.168.1.11"), ip("198.51.100.10"), Some((8, 0)))));
    // PC1 gets through, line 20 counts it
    let id = ping_once(&mut e, PC1, "198.51.100.10");
    e.run_until(e.now() + 3 * SEC, BUDGET);
    t.check("acl: PC1 permitted", ping_ev(&e, id).iter().any(|x| matches!(x, PingEv::Reply { .. })));
    t.check("acl: permit note", has_note(&e, R1, |n| matches!(n, Note::AclPermit { list, seq: 20, iface: 0, out: false } if list == "110")));
    let show = cli(&mut e, R1, "show access-lists 110");
    t.check("acl: counters", show.contains("    10 deny icmp host 192.168.1.11 any echo (4 matches)\n") && show.contains("    20 permit ip any any ("));
    let _ = lines(&mut e, R1, &["clear access-list counters 110"]);
    t.eq("acl: cleared", cli(&mut e, R1, "show access-lists 110"), s("Extended IP access list 110\n    10 deny icmp host 192.168.1.11 any echo\n    20 permit ip any any\n"));
    // a list with no line for it: the implicit deny, seq 0
    let _ = conf(&mut e, R1, &["access-list 120 permit tcp any any", "interface GigabitEthernet0/0/0", "ip access-group 120 in"]);
    let _ = ping_once(&mut e, PC1, "198.51.100.10");
    e.run_until(e.now() + 3 * SEC, BUDGET);
    t.check("acl: implicit deny", has_note(&e, R1, |n| matches!(n, Note::AclDeny { list, seq: 0, .. } if list == "120")));
    // a list that does not exist lets everything through
    let _ = conf(&mut e, R1, &["interface GigabitEthernet0/0/0", "ip access-group 199 in"]);
    let id = ping_once(&mut e, PC1, "198.51.100.10");
    e.run_until(e.now() + 3 * SEC, BUDGET);
    t.check("acl: missing list", ping_ev(&e, id).iter().any(|x| matches!(x, PingEv::Reply { .. })));
    t.check("acl: interface line", e.running_config(R1).contains(" ip address 192.168.1.1 255.255.255.0\n ip access-group 199 in\n ip nat inside\n"));
    let _ = conf(&mut e, R1, &["interface GigabitEthernet0/0/0", "no ip access-group 199 in"]);
    t.check("acl: removed", !e.running_config(R1).contains("ip access-group"));
    // the inbound list guards the router itself
    let _ = conf(&mut e, R1, &["access-list 130 deny icmp any host 192.168.1.1", "access-list 130 permit ip any any", "interface GigabitEthernet0/0/0", "ip access-group 130 in"]);
    let id = ping_once(&mut e, PC1, "192.168.1.1");
    e.run_until(e.now() + 3 * SEC, BUDGET);
    t.eq("acl: guards the router", ping_ev(&e, id).first().cloned().map(|x| matches!(x, PingEv::Error { kind: IcmpKind::Prohibited, .. })), Some(true));
    let (n, bad) = all_frames_ok(&e);
    t.check("acl: frames", n > 20 && bad == 0);
}

// the outbound list sees the translated source; what R1 sends itself passes
fn outbound_after_nat(t: &mut T) {
    let mut e = engine(&office());
    let _ = warm(&mut e);
    let o = conf(&mut e, R1, &["ip access-list extended OUT", "deny icmp host 203.0.113.1 host 198.51.100.10 echo", "permit ip any any", "interface GigabitEthernet0/0/1", "ip access-group OUT out"]);
    t.check("out: set-up", !o.contains('%'));
    let id = ping_once(&mut e, PC1, "198.51.100.10");
    e.run_until(e.now() + 3 * SEC, BUDGET);
    t.check("out: denied after translation", has_note(&e, R1, |n| matches!(n, Note::AclDeny { list, seq: 10, iface: 1, out: true } if list == "OUT")));
    // the answer goes to PC1 and quotes its packet as it arrived (not the translated one)
    let answer = packets(&e, R1, 5).into_iter().find(|p| Flow::of(&p.1).is_some_and(|f| f.icmp == Some((3, 13))));
    t.eq("out: quote untranslated", answer.as_ref().and_then(|a| Flow::quoted(&a.1)).map(|q| (q.src, q.dst)), Some((ip("192.168.1.10"), ip("198.51.100.10"))));
    t.eq("out: to PC1 from the inside interface", answer.as_ref().and_then(|a| Flow::of(&a.1)).map(|f| (f.src, f.dst)), Some((ip("192.168.1.1"), ip("192.168.1.10"))));
    t.check("out: PC1 heard", ping_ev(&e, id).iter().any(|x| matches!(x, PingEv::Error { kind: IcmpKind::Prohibited, .. })));
    // R1's own ping from 203.0.113.1 is not filtered on the way out
    let o = lines(&mut e, R1, &["enable", "ping 198.51.100.10"]);
    e.run_until(e.now() + 12 * SEC, BUDGET);
    let o = o + &out(&mut e, R1);
    t.check("out: own packets pass", o.contains("!!!!!"));
}

// "established" lets the answers in, not new connections
fn established(t: &mut T) {
    let mut e = engine(&office());
    let _ = warm(&mut e);
    let o = conf(&mut e, R1, &["access-list 150 permit tcp any any established", "access-list 150 permit icmp any any", "interface GigabitEthernet0/0/1", "ip access-group 150 in"]);
    t.check("est: set-up", !o.contains('%'));
    t.check("est: pdus", pdu(&mut e, PC1, "out", PduProto::Tcp, "198.51.100.10", 0, 7) && pdu(&mut e, EXT, "in", PduProto::Tcp, "203.0.113.1", 0, 80));
    e.run_until(e.now() + 5 * SEC, BUDGET);
    t.eq("est: from inside", sender(&e, PC1, "out"), Some((1, 1, 0)));
    t.eq("est: from outside", sender(&e, EXT, "in"), Some((1, 0, 1)));
    t.check("est: the SYN from outside denied", has_note(&e, R1, |n| matches!(n, Note::AclDeny { list, seq: 0, iface: 1, out: false } if list == "150")));
}

// ---------------------------------------------------------------- PAT

fn pat(t: &mut T) {
    let mut e = engine(&office());
    // warm ARP up, then a ping through PAT takes the time of four cables each way
    let id = e.ping(PC1, ip("198.51.100.10"), PingOpts { count: 3, ..PingOpts::default() }).unwrap_or(0);
    e.run_until(4 * SEC, BUDGET);
    let ev = ping_ev(&e, id);
    t.check("pat: echo through", ev.iter().any(|x| matches!(x, PingEv::Reply { seq: 3, rtt: 8208, ttl: 62, .. })));
    let pid = ident(&e, id);
    t.check("pat: the identifier kept", flows(&e, R1, ISP).iter().any(|f| f.proto == PROTO_ICMP && f.src == ip("203.0.113.1") && f.sport == pid));
    // the packet tab: R1's step has PC1's echo in, the translation among its decisions and the
    // translated echo out (the last probe: the first one waited for ARP)
    let src_of = |h: Option<u64>| h.and_then(|h| e.trace.hop(h)).filter(|h| h.bytes.len() > 34 && h.bytes[12..14] == [0x08, 0x00]).and_then(|h| Ipv4View::parse(&h.bytes[14..]).map(|v| v.src()));
    let step = e.trace.events().filter(|x| x.dev == R1 && matches!(x.note, Note::NatTranslate { inbound: false, .. })).map(|x| e.step_of(x)).last();
    let (in_src, out_src) = (src_of(step.as_ref().and_then(|s| s.inbound)), src_of(step.as_ref().and_then(|s| s.outbound.first().copied())));
    t.eq("pat: the step of R1", (in_src, out_src), (Some(ip("192.168.1.10")), Some(ip("203.0.113.1"))));
    t.check("pat: the step's decisions", step.is_some_and(|s| s.decisions.iter().any(|d| matches!(d.note, Note::IpRouted { .. })) && s.decisions.iter().any(|d| matches!(d.note, Note::NatTranslate { .. }))));
    // two hosts, the same UDP source port: the second one gets the next port of the same parity
    t.check("pat: udp pdus", pdu(&mut e, PC1, "u1", PduProto::Udp, "198.51.100.10", 5000, 9) && pdu(&mut e, PC2, "u2", PduProto::Udp, "198.51.100.10", 5000, 9));
    // and the same TCP port: an odd one stays odd
    t.check("pat: tcp pdus", pdu(&mut e, PC1, "t1", PduProto::Tcp, "198.51.100.10", 40001, 7) && pdu(&mut e, PC2, "t2", PduProto::Tcp, "198.51.100.10", 40001, 7));
    e.run_until(e.now() + 3 * SEC, BUDGET);
    let udp: Vec<(Ipv4, u16)> = flows(&e, R1, ISP).iter().filter(|f| f.proto == PROTO_UDP).map(|f| (f.src, f.sport)).collect();
    t.eq("pat: udp ports", udp, vec![(ip("203.0.113.1"), 5000), (ip("203.0.113.1"), 5002)]);
    let syn: Vec<u16> = flows(&e, R1, ISP).iter().filter(|f| f.proto == PROTO_TCP && f.flags == SYN).map(|f| f.sport).collect();
    t.eq("pat: tcp ports", syn, vec![40001, 40003]);
    t.eq("pat: tcp both answered", (sender(&e, PC1, "t1"), sender(&e, PC2, "t2")), (Some((1, 1, 0)), Some((1, 1, 0))));
    // the port unreachables of SRV find their hosts: the quote has the inside port again
    let back: Vec<(Ipv4, u16)> = packets(&e, R1, 5).iter().filter_map(|p| Flow::of(&p.1).filter(|f| f.icmp == Some((3, 3))).and_then(|_| Flow::quoted(&p.1))).map(|q| (q.src, q.sport)).collect();
    t.eq("pat: errors translated back", back, vec![(ip("192.168.1.10"), 5000), (ip("192.168.1.11"), 5000)]);
    t.eq("pat: create notes", notes(&e, R1).iter().filter(|n| matches!(n.1, Note::NatCreate { proto: PROTO_UDP, .. })).map(|n| n.1.clone()).collect::<Vec<_>>(), vec![
        Note::NatCreate { proto: PROTO_UDP, local: (ip("192.168.1.10"), 5000), global: (ip("203.0.113.1"), 5000), remote: Some((ip("198.51.100.10"), 9)) },
        Note::NatCreate { proto: PROTO_UDP, local: (ip("192.168.1.11"), 5000), global: (ip("203.0.113.1"), 5002), remote: Some((ip("198.51.100.10"), 9)) },
    ]);
    // an echo of PC2 with PC1's identifier gets the next one, and its reply the old one back
    let r1_in = port_mac(R1, 0);
    let req = echo("192.168.1.11", "198.51.100.10", false, pid, 9, b"collide", 77, 64);
    let _ = e.apply(Action::Inject { dev: PC2, port: s("eth0"), frame: frame(r1_in, port_mac(PC2, 0), &req) });
    e.run_until(e.now() + SEC, BUDGET);
    let ids: Vec<u16> = flows(&e, R1, ISP).iter().filter(|f| f.proto == PROTO_ICMP && f.icmp == Some((8, 0)) && f.id == 77).map(|f| f.sport).collect();
    t.eq("pat: colliding identifier", ids, vec![pid.wrapping_add(1)]);
    let replies: Vec<(Ipv4, u16)> = flows(&e, R1, 5).iter().filter(|f| f.icmp == Some((0, 0)) && f.dst == ip("192.168.1.11")).map(|f| (f.dst, f.dport)).collect();
    t.eq("pat: reply to PC2 with its identifier", replies, vec![(ip("192.168.1.11"), pid)]);
    // the table
    let table = cli(&mut e, R1, "show ip nat translations");
    t.check("pat: table header", table.starts_with("Pro Inside global         Inside local          Outside local         Outside global\n"));
    t.check("pat: table rows", table.contains("udp 203.0.113.1:5000      192.168.1.10:5000     198.51.100.10:9       198.51.100.10:9\n") && table.contains("udp 203.0.113.1:5002      192.168.1.11:5000     198.51.100.10:9       198.51.100.10:9\n"));
    t.check("pat: static rows", table.contains("tcp 203.0.113.1:80        192.168.1.20:80       ---                   ---\n") && table.ends_with("--- 203.0.113.30          192.168.1.30          ---                   ---\n"));
    let stats = cli(&mut e, R1, "show ip nat statistics");
    t.check("pat: statistics", stats.contains("Outside interfaces:\n  GigabitEthernet0/0/1\nInside interfaces:\n  GigabitEthernet0/0/0\n") && stats.contains("  list 1 interface GigabitEthernet0/0/1 overload: "));
    t.check("pat: hits and misses", e.nat(R1).is_some_and(|n| n.stats.hits > 0 && n.stats.misses == 6 && n.stats.failed == 0));
    // filtering from outside: EXT never talked to PC1, so its datagram to PC1's mapping stays at R1
    let stray = udp_pkt("192.0.2.20", 999, "203.0.113.1", 5000, 64);
    let _ = e.apply(Action::Inject { dev: ISP, port: s("gi0/0/0"), frame: frame(port_mac(R1, 1), port_mac(ISP, 0), &stray) });
    e.run_until(e.now() + SEC, BUDGET);
    t.check("pat: no stray datagram inside", !flows(&e, R1, 5).iter().any(|f| f.src == ip("192.0.2.20")));
    let (n, bad) = all_frames_ok(&e);
    t.eq("pat: frames checked", (n > 30, bad), (true, 0));
}

// ---------------------------------------------------------------- static translations

fn statics(t: &mut T) {
    let mut e = engine(&office());
    // port forwarding: EXT reaches WEB's echo on port 80 through 203.0.113.1:80
    t.check("static: pdu", pdu(&mut e, EXT, "web", PduProto::Tcp, "203.0.113.1", 0, 80));
    e.run_until(6 * SEC, BUDGET);
    t.eq("static: forwarded", sender(&e, EXT, "web"), Some((1, 1, 0)));
    t.check("static: to WEB:80", flows(&e, R1, 5).iter().any(|f| f.proto == PROTO_TCP && f.dst == ip("192.168.1.20") && f.dport == 80 && f.flags == SYN));
    t.check("static: from 203.0.113.1:80", flows(&e, R1, ISP).iter().any(|f| f.proto == PROTO_TCP && f.src == ip("203.0.113.1") && f.sport == 80 && f.flags == SYN | ACK));
    // an address: R1 answers ARP for 203.0.113.30 and takes SRV's pings to PC3
    let id = e.ping(SRV, ip("203.0.113.30"), PingOpts { count: 3, ..PingOpts::default() }).unwrap_or(0);
    e.run_until(e.now() + 5 * SEC, BUDGET);
    t.check("static: ARP for the global address", has_note(&e, R1, |n| matches!(n, Note::ArpReply { iface: 1, ip, .. } if *ip == Ipv4::new(203, 0, 113, 30))));
    t.check("static: inbound ping", ping_ev(&e, id).iter().any(|x| matches!(x, PingEv::Reply { from, .. } if *from == Ipv4::new(203, 0, 113, 30))));
    // PC3 leaves with its own global address, not with PAT's
    let _ = ping_once(&mut e, PC3, "198.51.100.10");
    e.run_until(e.now() + 3 * SEC, BUDGET);
    t.check("static: outbound address", flows(&e, R1, ISP).iter().any(|f| f.icmp == Some((8, 0)) && f.src == ip("203.0.113.30")));
    // a second static rule for the same inside address and port is refused
    t.eq("static: clash", conf(&mut e, R1, &["ip nat inside source static tcp 192.168.1.20 80 203.0.113.2 8080"]), s("% 192.168.1.20 already has a static translation\n"));
    let (n, bad) = all_frames_ok(&e);
    t.eq("static: frames", (n > 10, bad), (true, 0));
}

// ---------------------------------------------------------------- a pool

fn pool(t: &mut T) {
    let mut e = engine(&office());
    let _ = warm(&mut e);
    let _ = lines(&mut e, R1, &["enable", "clear ip nat translation *"]);
    let o = conf(&mut e, R1, &["no ip nat inside source list 1 interface GigabitEthernet0/0/1 overload", "ip nat pool OFFICE 203.0.113.100 203.0.113.101 netmask 255.255.255.0", "ip nat inside source list 1 pool OFFICE", "ip nat translation timeout 30"]);
    t.check("pool: set-up", !o.contains('%'));
    // (ISP asks ARP for each pool address first: the first echo of each is lost there)
    let a = e.ping(PC1, ip("198.51.100.10"), PingOpts { count: 2, ..PingOpts::default() }).unwrap_or(0);
    e.run_until(e.now() + 3 * SEC, BUDGET);
    let b = e.ping(PC2, ip("198.51.100.10"), PingOpts { count: 2, ..PingOpts::default() }).unwrap_or(0);
    e.run_until(e.now() + 3 * SEC, BUDGET);
    // WEB is the third host: no address left
    let c = ping_once(&mut e, WEB, "198.51.100.10");
    e.run_until(e.now() + 3 * SEC, BUDGET);
    let src: Vec<Ipv4> = flows(&e, R1, ISP).iter().filter(|f| f.icmp == Some((8, 0))).map(|f| f.src).collect();
    t.check("pool: one address each", src.contains(&ip("203.0.113.100")) && src.contains(&ip("203.0.113.101")) && !src.iter().any(|x| *x == ip("192.168.1.20")));
    t.check("pool: replies", [a, b].iter().all(|id| ping_ev(&e, *id).iter().any(|x| matches!(x, PingEv::Reply { .. }))));
    t.eq("pool: exhausted", ping_ev(&e, c).first().cloned(), Some(PingEv::Error { seq: 1, from: ip("192.168.1.1"), kind: IcmpKind::Prohibited }));
    t.check("pool: exhausted note", has_note(&e, R1, |n| *n == Note::NatExhausted { what: s("pool OFFICE") }));
    t.eq("pool: refused counted", e.nat(R1).map(|n| n.stats.failed), Some(1));
    t.check("pool: create notes", has_note(&e, R1, |n| *n == Note::NatCreate { proto: 0, local: (ip("192.168.1.10"), 0), global: (ip("203.0.113.100"), 0), remote: None }));
    // basic NAT takes packets from anyone: SRV reaches PC1 at its pool address
    let id = ping_once(&mut e, SRV, "203.0.113.100");
    e.run_until(e.now() + 3 * SEC, BUDGET);
    t.check("pool: inbound to a pool address", ping_ev(&e, id).iter().any(|x| matches!(x, PingEv::Reply { .. })));
    let table = cli(&mut e, R1, "show ip nat translations");
    t.check("pool: address rows", table.contains("--- 203.0.113.100         192.168.1.10          ---                   ---\n"));
    t.eq("pool: in use", conf(&mut e, R1, &["no ip nat pool OFFICE"]), s("% Pool OFFICE is in use\n"));
    t.check("pool: statistics", cli(&mut e, R1, "show ip nat statistics").contains("Pool OFFICE: 203.0.113.100 to 203.0.113.101, netmask 255.255.255.0\n  2 addresses, 2 in use (100%)\n"));
    // after the ICMP sessions (60 s) the addresses go after their own 30 s, which have passed
    let last = e.now();
    e.run_until(last + 90 * SEC, BUDGET);
    let gone: Vec<(u64, Note)> = notes(&e, R1).into_iter().filter(|n| matches!(n.1, Note::NatExpire { proto: 0, .. })).collect();
    t.eq("pool: addresses expired", gone.len(), 2);
    // an ICMP error about nothing NAT knows, to a pool address: dropped (RFC 5508)
    let q = udp_pkt("203.0.113.100", 4444, "198.51.100.10", 9, 63);
    let mut p = Pkt::new(HEADROOM, icmp::error_quote(&q));
    Icmp::Unreachable { code: 3, mtu: 0 }.push_onto(&mut p);
    Ipv4Hdr::new(ip("198.51.100.10"), ip("203.0.113.100"), PROTO_ICMP, 63).push_onto(&mut p);
    let _ = e.apply(Action::Inject { dev: ISP, port: s("gi0/0/0"), frame: frame(port_mac(R1, 1), port_mac(ISP, 0), &p.into_vec()) });
    e.run_until(e.now() + SEC, BUDGET);
    t.check("pool: stray error dropped", has_note(&e, R1, |n| *n == Note::NatErrorDropped));
    t.check("pool: running config", e.running_config(R1).contains("!\nip nat translation timeout 30\nip nat pool OFFICE 203.0.113.100 203.0.113.101 netmask 255.255.255.0\nip nat inside source list 1 pool OFFICE\nip nat inside source static tcp"));
}

// ---------------------------------------------------------------- ICMP errors

fn icmp_errors(t: &mut T) {
    let mut e = engine(&office());
    // traceroute from inside: R1 itself, then ISP and SRV from outside, translated back (R1
    // learns PC3's address first, for later)
    let _ = warm(&mut e);
    let _ = ping_once(&mut e, PC3, "192.168.1.1");
    e.run_until(e.now() + SEC, BUDGET);
    let tr = e.traceroute(PC1, ip("198.51.100.10"), TraceOpts::default()).unwrap_or(0);
    e.run_until(e.now() + 20 * SEC, BUDGET);
    let (hops, reached) = match e.job(tr).map(|j| &j.kind) {
        Some(JobKind::Trace(x)) => (x.hops.iter().map(|h| (h.ttl, h.probes.iter().map(|p| (p.from, p.kind)).collect::<Vec<_>>())).collect::<Vec<_>>(), x.reached),
        _ => (Vec::new(), false),
    };
    let hop = |a: &str, k: IcmpKind| vec![(Some(ip(a)), Some(k)); 3];
    t.eq("errors: traceroute", hops, vec![(1, hop("192.168.1.1", IcmpKind::TtlExceeded)), (2, hop("203.0.113.254", IcmpKind::TtlExceeded)), (3, hop("198.51.100.10", IcmpKind::PortUnreachable))]);
    t.check("errors: reached", reached);
    t.eq("errors: quotes translated back", notes(&e, R1).iter().filter(|n| matches!(n.1, Note::NatQuote { inbound: true, .. })).count(), 6);
    // every translated error is whole: outer header, ICMP, the quoted header and its UDP sum
    let back: Vec<Vec<u8>> = packets(&e, R1, 5).into_iter().filter(|p| Flow::of(&p.1).is_some_and(|f| f.icmp.is_some_and(|i| i.0 == 3 || i.0 == 11) && f.src != ip("192.168.1.1"))).map(|p| p.1).collect();
    t.check("errors: checksums", back.len() == 6 && back.iter().all(|p| sums_ok(p)));
    // from inside (RFC 5508 REQ-4): PC3 answers EXT's datagram to its global address with port
    // unreachable; the quote and the source become 203.0.113.30
    let d = udp_pkt("192.0.2.20", 777, "203.0.113.30", 9, 64);
    let _ = e.apply(Action::Inject { dev: ISP, port: s("gi0/0/0"), frame: frame(port_mac(R1, 1), port_mac(ISP, 0), &d) });
    e.run_until(e.now() + 2 * SEC, BUDGET);
    let err = packets(&e, R1, ISP).into_iter().find(|p| Flow::of(&p.1).is_some_and(|f| f.icmp == Some((3, 3)) && f.dst == ip("192.0.2.20")));
    t.eq("errors: from inside", err.as_ref().and_then(|p| Flow::of(&p.1)).map(|f| f.src), Some(ip("203.0.113.30")));
    t.eq("errors: quote from inside", err.as_ref().and_then(|p| Flow::quoted(&p.1)).map(|q| (q.src, q.sport, q.dst, q.dport)), Some((ip("192.0.2.20"), 777, ip("203.0.113.30"), 9)));
    t.check("errors: sums from inside", err.as_ref().is_some_and(|p| sums_ok(&p.1)));
    t.check("errors: note", has_note(&e, R1, |n| *n == Note::NatQuote { inbound: false, from: (ip("192.168.1.30"), 9), to: (ip("203.0.113.30"), 9) }));
}

// ---------------------------------------------------------------- fragments

fn fragments(t: &mut T) {
    let mut e = engine(&office());
    // a large echo leaves PC1 in three fragments; R1 translates each, SRV answers in three
    let _ = warm(&mut e);
    let id = e.ping(PC1, ip("198.51.100.10"), PingOpts { count: 1, size: 3000, ..PingOpts::default() }).unwrap_or(0);
    e.run_until(e.now() + 3 * SEC, BUDGET);
    t.check("frag: big echo answered", ping_ev(&e, id).iter().any(|x| matches!(x, PingEv::Reply { bytes: 3028, .. })));
    let outf = flows(&e, R1, ISP).into_iter().filter(|f| f.proto == PROTO_ICMP && (f.mf || f.later)).count();
    let inf = flows(&e, R1, 5).into_iter().filter(|f| f.proto == PROTO_ICMP && (f.mf || f.later)).count();
    t.eq("frag: fragments both ways", (outf, inf), (3, 3));
    // the fragments PC1 got make a datagram whose ICMP checksum holds
    let mut re = Reassembly::new();
    let mut whole = None;
    for (_, p) in packets(&e, R1, 5) {
        if let Some(v) = Ipv4View::parse(&p)
            && v.is_fragment()
        {
            whole = re.add(&v).or(whole);
        }
    }
    let ok = whole.as_deref().and_then(Ipv4View::parse).is_some_and(|v| v.dst() == ip("192.168.1.10") && IcmpView::parse(v.payload()).is_some_and(|ic| ic.checksum_ok() && ic.id() == ident(&e, id)));
    t.check("frag: reassembled reply", ok);
    // out of order: the later fragments of an answer come first and wait for the first one
    let pid = ident(&e, id);
    let reply = echo("198.51.100.10", "203.0.113.1", true, pid, 40, &[7u8; 1200], 4242, 63);
    let parts = ipv4::fragment(&reply, 576).unwrap_or_default();
    t.eq("frag: three parts", parts.len(), 3);
    let (dst, src) = (port_mac(R1, 1), port_mac(ISP, 0));
    let from = e.trace.next_hop_id();
    for p in parts.iter().rev() {
        let _ = e.apply(Action::Inject { dev: ISP, port: s("gi0/0/0"), frame: frame(dst, src, p) });
        e.run_until(e.now() + 10 * MS, BUDGET);
    }
    t.eq("frag: two waited", notes(&e, R1).iter().filter(|n| n.1 == Note::NatFragWait { id: 4242 }).count(), 2);
    let offs: Vec<(usize, Ipv4)> = e.trace.hops_after(from - 1).filter(|h| h.from.0 == R1 && h.to.0 == 5).filter_map(|h| Ipv4View::parse(&h.bytes[14..]).filter(|v| v.id() == 4242).map(|v| (v.frag_off(), v.dst()))).collect();
    t.eq("frag: in order, translated", offs, vec![(0, ip("192.168.1.10")), (552, ip("192.168.1.10")), (1104, ip("192.168.1.10"))]);
    // a later fragment whose first never comes is dropped after 30 s
    let lone = &parts[2];
    let mut l2 = lone.clone();
    l2[4..6].copy_from_slice(&4343u16.to_be_bytes());
    let ck = {
        l2[10] = 0;
        l2[11] = 0;
        checksum::checksum(&l2[..20])
    };
    l2[10..12].copy_from_slice(&ck.to_be_bytes());
    let at = e.now();
    let _ = e.apply(Action::Inject { dev: ISP, port: s("gi0/0/0"), frame: frame(dst, src, &l2) });
    e.run_until(at + 40 * SEC, BUDGET);
    let dropped: Vec<u64> = notes(&e, R1).into_iter().filter(|n| n.1 == Note::NatFragDropped { id: 4343, parts: 1 }).map(|n| n.0).collect();
    let arrived = notes(&e, R1).into_iter().find(|n| n.1 == Note::NatFragWait { id: 4343 }).map(|n| n.0).unwrap_or(0);
    t.eq("frag: lone fragment dropped after 30 s", dropped, vec![arrived + 30 * SEC]);
}

// ---------------------------------------------------------------- timeouts

fn timeouts(t: &mut T) {
    let mut e = engine(&office());
    let o = conf(&mut e, R1, &["ip nat translation udp-timeout 120"]);
    t.check("timeouts: set-up", !o.contains('%'));
    // the warm-up's echoes are the ICMP session
    let _ = warm(&mut e);
    t.check("timeouts: pdus", pdu(&mut e, PC1, "u", PduProto::Udp, "198.51.100.10", 6000, 9) && pdu(&mut e, PC1, "tc", PduProto::Tcp, "198.51.100.10", 41000, 7));
    e.run_until(e.now() + 5 * SEC, BUDGET);
    // (a session counts the packets of both directions, except ICMP errors)
    let last = |e: &Engine, f: &dyn Fn(&Flow) -> bool| packets(e, R1, ISP).iter().chain(packets(e, R1, 5).iter()).filter(|p| Flow::of(&p.1).is_some_and(|x| f(&x))).map(|p| p.0).max().unwrap_or(0);
    let lu = last(&e, &|f| f.proto == PROTO_UDP && f.sport == 6000);
    let li = last(&e, &|f| f.proto == PROTO_ICMP && f.icmp.is_some_and(|i| i.0 == 0 || i.0 == 8));
    // the last segment of the TCP connection crossed R1 after both FINs: the session ends 240 s
    // later (RFC 5382), counted from the last packet either way
    let lt = packets(&e, R1, ISP).iter().chain(packets(&e, R1, 5).iter()).filter(|p| Flow::of(&p.1).is_some_and(|x| x.proto == PROTO_TCP && (x.sport == 41000 || x.dport == 41000))).map(|p| p.0).max().unwrap_or(0);
    e.run_until(400 * SEC, BUDGET);
    let ends = |e: &Engine, proto: u8| notes(e, R1).into_iter().filter(|n| matches!(n.1, Note::NatExpire { proto: p, .. } if p == proto)).map(|n| n.0).collect::<Vec<_>>();
    t.eq("timeouts: icmp 60 s", ends(&e, PROTO_ICMP), vec![li + 60 * SEC]);
    t.eq("timeouts: udp 120 s", ends(&e, PROTO_UDP), vec![lu + 120 * SEC]);
    t.eq("timeouts: tcp after fin, 240 s", ends(&e, PROTO_TCP), vec![lt + 240 * SEC]);
    t.eq("timeouts: nothing left", e.nat(R1).map(|n| (n.sessions().count(), n.mappings().count())), Some((0, 0)));
    t.eq("timeouts: expired counted", e.nat(R1).map(|n| n.stats.expired), Some(3));
    t.check("timeouts: running config", e.running_config(R1).contains("ip nat translation udp-timeout 120\n"));
    let _ = conf(&mut e, R1, &["no ip nat translation udp-timeout"]);
    t.check("timeouts: back to the default", !e.running_config(R1).contains("udp-timeout"));
}

// ---------------------------------------------------------------- clear, power, debug

fn clear_power_debug(t: &mut T) {
    let mut e = engine(&office());
    let _ = lines(&mut e, R1, &["enable", "debug ip nat"]);
    t.check("debug: on", cli(&mut e, R1, "show debugging").contains("Debug output for NAT: on\n"));
    let id = warm(&mut e);
    let o = out(&mut e, R1);
    let pid = ident(&e, id);
    t.check("debug: lines", o.contains(&format!(": NAT: new translation icmp 192.168.1.10:{} = 203.0.113.1:{}, peer 198.51.100.10:{}\n", pid, pid, pid)) && o.contains(&format!(": NAT: icmp source 192.168.1.10:{} -> 203.0.113.1:{}\n", pid, pid)) && o.contains(&format!(": NAT: icmp destination 203.0.113.1:{} -> 192.168.1.10:{}\n", pid, pid)));
    let _ = cli(&mut e, R1, "undebug all");
    t.eq("debug: off", cli(&mut e, R1, "show debugging"), String::new());
    // clear: the dynamic translations go, the static ones stay
    let _ = cli(&mut e, R1, "clear ip nat translation *");
    let table = cli(&mut e, R1, "show ip nat translations");
    t.eq("clear: statics left", table.lines().count(), 3);
    // power: translations and counters gone, the rules stay
    let _ = ping_once(&mut e, PC1, "198.51.100.10");
    e.run_until(e.now() + 3 * SEC, BUDGET);
    let _ = e.apply(Action::Power { dev: R1, on: false });
    let _ = e.apply(Action::Power { dev: R1, on: true });
    t.eq("power: no sessions", e.nat(R1).map(|n| n.sessions().count()), Some(0));
    t.eq("power: rules kept", e.nat(R1).map(|n| n.rules.len()), Some(3));
}

// ---------------------------------------------------------------- texts

fn texts(t: &mut T) {
    let rows: Vec<(&str, &str)> = include_str!("locales/engine.tsv").lines().skip(1).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect();
    let en = |k: &'static str| rows.iter().find(|r| r.0 == k).map_or(k, |r| r.1);
    let port = |p: u16| format!("Gi{}", p);
    let iface = |i: u16| if i == 0 { s("GigabitEthernet0/0/0") } else { s("GigabitEthernet0/0/1") };
    let cases: Vec<(Note, &str, &str)> = vec![
        (Note::AclDeny { list: s("110"), seq: 10, iface: 0, out: false }, "Zugriffsliste 110 eingehend an GigabitEthernet0/0/0: Zeile 10 verwirft das Paket.", "Access list 110 inbound on GigabitEthernet0/0/0: line 10 drops the packet."),
        (Note::AclDeny { list: s("OUT"), seq: 0, iface: 1, out: true }, "Zugriffsliste OUT ausgehend an GigabitEthernet0/0/1: keine Zeile passt, verworfen (implizites deny).", "Access list OUT outbound on GigabitEthernet0/0/1: no line matches, dropped (implicit deny)."),
        (Note::AclPermit { list: s("1"), seq: 20, iface: 1, out: true }, "Zugriffsliste 1 ausgehend an GigabitEthernet0/0/1: Zeile 20 erlaubt das Paket.", "Access list 1 outbound on GigabitEthernet0/0/1: line 20 lets the packet through."),
        (Note::NatTranslate { proto: PROTO_TCP, inbound: false, from: (ip("192.168.1.10"), 40000), to: (ip("203.0.113.1"), 40002) }, "NAT: Quelle 192.168.1.10:40000 \u{2192} 203.0.113.1:40002 (TCP).", "NAT: source 192.168.1.10:40000 \u{2192} 203.0.113.1:40002 (TCP)."),
        (Note::NatTranslate { proto: PROTO_ICMP, inbound: true, from: (ip("203.0.113.1"), 7), to: (ip("192.168.1.10"), 7) }, "NAT: Ziel 203.0.113.1 (Kennung 7) \u{2192} 192.168.1.10 (Kennung 7) (ICMP).", "NAT: destination 203.0.113.1 (identifier 7) \u{2192} 192.168.1.10 (identifier 7) (ICMP)."),
        (Note::NatCreate { proto: 0, local: (ip("192.168.1.10"), 0), global: (ip("203.0.113.100"), 0), remote: None }, "NAT: 192.168.1.10 bekommt 203.0.113.100 für alle Protokolle.", "NAT: 192.168.1.10 gets 203.0.113.100 for every protocol."),
        (Note::NatExpire { proto: PROTO_UDP, local: (ip("192.168.1.10"), 5000), global: (ip("203.0.113.1"), 5000), remote: Some((ip("198.51.100.10"), 9)) }, "NAT: Übersetzung UDP 192.168.1.10:5000 = 203.0.113.1:5000 (Gegenstelle 198.51.100.10:9) abgelaufen.", "NAT: translation UDP 192.168.1.10:5000 = 203.0.113.1:5000 (peer 198.51.100.10:9) expired."),
        (Note::NatExhausted { what: s("pool OFFICE") }, "NAT: keine freie Adresse und kein freier Port (pool OFFICE): verworfen.", "NAT: no free address or port (pool OFFICE): dropped."),
        (Note::NatFragWait { id: 4242 }, "NAT: Fragment (ID 4242) wartet auf das erste Fragment.", "NAT: fragment (ID 4242) waits for the first fragment."),
    ];
    for (n, de, english) in cases {
        let m = n.message(&port, &iface);
        t.eq("text: german", m.text(), s(de));
        t.eq("text: english", m.text_with(&en), s(english));
        t.check("text: layer 3", n.layer() == 3);
    }
    t.check("text: drops", Note::AclDeny { list: s("1"), seq: 0, iface: 0, out: false }.is_drop() && !Note::AclPermit { list: s("1"), seq: 1, iface: 0, out: false }.is_drop());
    t.eq("text: mask", Note::NatTranslate { proto: PROTO_UDP, inbound: true, from: (Ipv4::UNSPECIFIED, 0), to: (Ipv4::UNSPECIFIED, 0) }.mask(), decode::P_IPV4 | decode::P_UDP);
}

// the same project twice: the same frames at the same times
fn determinism(t: &mut T) {
    let run = || {
        let mut e = engine(&office());
        let _ = pdu(&mut e, PC1, "u1", PduProto::Udp, "198.51.100.10", 5000, 9);
        let _ = pdu(&mut e, PC2, "u2", PduProto::Udp, "198.51.100.10", 5000, 9);
        let _ = pdu(&mut e, EXT, "web", PduProto::Tcp, "203.0.113.1", 0, 80);
        let _ = e.ping(PC1, ip("198.51.100.10"), PingOpts { count: 2, ..PingOpts::default() });
        e.run_until(5 * SEC, BUDGET);
        e.trace.hops().map(|h| (h.t_tx, h.bytes.to_vec())).collect::<Vec<_>>()
    };
    let (a, b) = (run(), run());
    t.check("determinism", a == b && a.len() > 40);
}
