// ipv6test -- checks of IPv6 in jfn_sim (group ipv6 of --selftest: jfn_sim::ip6, nd, l3v6,
// jobs6, trace6 and the consoles cli/ip6.rs, cli/linux6.rs, cli/print6.rs): link-local
// addresses from the MAC (EUI-64) and their duplicate address detection with its random delay
// from the device's own stream, a duplicate (Linux and IOS-like, and the problem it makes),
// router solicitations and advertisements, SLAAC with its lifetimes (deprecated, expired, the
// two-hour rule), the neighbor cache through INCOMPLETE, REACHABLE, STALE, DELAY and PROBE and
// both ways of failing, ping and traceroute across two routers (static routes, a link-local
// next hop, the hop limit, no route, a loop), Packet Too Big and the sender's fragments,
// reassembly given up, the limits of ICMPv6 errors (the IOS-like bucket, Linux per
// destination), the consoles (help, show, debug, clear, the configuration read back), power
// and routing switched off, a project without IPv6 as before, the notes in German and English,
// the checksums of every frame, determinism
//
// The network of most checks (device ids in order): PC1 and PC2 on SW1; R1 between
// 2001:db8:1::/64 (Gi0/0/0, advertised to the PCs) and 2001:db8:12::/64 (Gi0/0/1, link-local
// FE80::1), with a static route to 2001:db8:2::/64; R2 between 2001:db8:12::/64 and
// 2001:db8:2::/64 with a default route to FE80::1; SRV (2001:db8:2::10, no SLAAC) behind R2.
// 1 Gbit/s copper with 50 ns of cable: a frame of n bytes takes (max(n, 60) + 24) * 8 + 50 ns
// per cable, so a DAD probe (78 bytes) or an advertisement without MAC 866 ns, a solicitation
// with the sender's MAC or an advertisement with the target's (86 bytes) 930 ns, a router
// solicitation (70 bytes) 802 ns, a router advertisement with one prefix and a ping of 56 data
// bytes (118 bytes each) 1186 ns.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use base::rng::Pcg32;
use jfn_model::project::{PortRef, Project};
use jfn_sim::node::port_mac;
use jfn_sim::{Action, Addr6View, AddrState, Engine, Icmp6Kind, JobKind, NState, Note, Note6, Origin6, Ping6Ev, Ping6Opts, ProblemKind, Report, Src6, TraceOpts, MS, SEC};
use netpkt::decode;
use netpkt::eth::{EthHdr, ET_IPV6};
use netpkt::icmpv6::{Icmp6, Icmp6View};
use netpkt::ipv6::{self, Ipv6Hdr, Ipv6View, NH_ICMPV6};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::{Ipv6, Mac, Prefix6};

use crate::selftest::T;

const BUDGET: usize = 8_000_000;

const PC1: u32 = 1;
const PC2: u32 = 2;
const SW1: u32 = 3;
const R1: u32 = 4;
const R2: u32 = 5;
const SRV: u32 = 6;

/// what turns IPv6 on in a Linux host (it is off until then)
const ON: &str = "sysctl -w net.ipv6.conf.all.disable_ipv6=0\n";

const R1_CFG: &str = "hostname R1\n!\nipv6 unicast-routing\n!\ninterface GigabitEthernet0/0/0\n ipv6 address 2001:DB8:1::1/64\n no shutdown\n!\ninterface GigabitEthernet0/0/1\n ipv6 address FE80::1 link-local\n ipv6 address 2001:DB8:12::1/64\n no shutdown\n!\nipv6 route 2001:DB8:2::/64 2001:DB8:12::2\n!\nend\n";
const R2_CFG: &str = "hostname R2\n!\nipv6 unicast-routing\n!\ninterface GigabitEthernet0/0/0\n ipv6 address 2001:DB8:12::2/64\n no shutdown\n!\ninterface GigabitEthernet0/0/1\n ipv6 address 2001:DB8:2::1/64\n no shutdown\n!\nipv6 route ::/0 GigabitEthernet0/0/0 FE80::1\n!\nend\n";

/// the frame times of the header comment (one cable)
const NS_T: u64 = 930;
const DAD_T: u64 = 866;
const RS_T: u64 = 802;
const RA_T: u64 = 1186;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn s(x: &str) -> String {
    String::from(x)
}

fn a6(x: &str) -> Ipv6 {
    Ipv6::parse(x).unwrap_or(Ipv6::UNSPECIFIED)
}

fn p6(x: &str) -> Prefix6 {
    Prefix6::parse(x).unwrap_or(Prefix6 { addr: Ipv6::UNSPECIFIED, len: 0 })
}

fn set(p: &mut Project, id: u32, name: &str, text: String) {
    if let Some(d) = p.device_mut(id) {
        d.name = String::from(name);
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

fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
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

/// a line typed at a device, then `d` of virtual time: what it printed
fn cli_for(e: &mut Engine, dev: u32, line: &str, d: u64) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    run_for(e, d);
    out(e, dev)
}

/// a console refused a line ("%" at the start of a line; log lines start with their stamp)
fn refused(o: &str) -> bool {
    o.lines().any(|l| l.starts_with('%'))
}

/// the interface number of an interface by name
fn ifn(e: &Engine, dev: u32, name: &str) -> u16 {
    e.ifaces(dev).iter().find(|f| f.name == name).map(|f| f.index).unwrap_or(u16::MAX)
}

/// the link-local address of a port from its MAC
fn ll_of(dev: u32, port: u16) -> Ipv6 {
    Ipv6::link_local_of(port_mac(dev, port))
}

/// the address SLAAC forms in a /64 for the first port of a device
fn slaac_of(prefix: &str, dev: u32) -> Ipv6 {
    Ipv6::eui64_in(a6(prefix), port_mac(dev, 0))
}

/// the state of an address of an interface (None: it has none)
fn state_of(e: &Engine, dev: u32, iface: u16, a: Ipv6) -> Option<AddrState> {
    e.addrs6(dev, iface).iter().find(|x| x.addr == a).map(|x| x.state)
}

/// the IPv6 stream of a device's random numbers (nd.rs: v6_rand)
fn stream6(seed: u64, dev: u32) -> Pcg32 {
    Pcg32::derive(seed ^ 0x6970_7636, dev as u64)
}

/// the IPv6 decisions of a device with their times
fn notes6(e: &Engine, dev: u32) -> Vec<(u64, Note6)> {
    e.trace
        .events()
        .filter(|x| x.dev == dev)
        .filter_map(|x| match &x.note {
            Note::V6(n) => Some((x.t, n.clone())),
            _ => None,
        })
        .collect()
}

/// the times of the IPv6 decisions of a device that match
fn times6(e: &Engine, dev: u32, f: impl Fn(&Note6) -> bool) -> Vec<u64> {
    notes6(e, dev).into_iter().filter(|x| f(&x.1)).map(|x| x.0).collect()
}

/// the first time an IPv6 decision of a device matches
fn when6(e: &Engine, dev: u32, f: impl Fn(&Note6) -> bool) -> Option<u64> {
    times6(e, dev, f).first().copied()
}

/// how many IPv6 decisions of a device in [a, b) match
fn count6(e: &Engine, dev: u32, a: u64, b: u64, f: impl Fn(&Note6) -> bool) -> usize {
    times6(e, dev, f).into_iter().filter(|x| *x >= a && *x < b).count()
}

/// an Ethernet frame with an IPv6 packet
fn is6(b: &[u8]) -> bool {
    b.len() >= 54 && b[12..14] == [0x86, 0xDD]
}

/// the ICMPv6 type of an Ethernet frame without extension headers (0: none)
fn icmp_type(b: &[u8]) -> u8 {
    if is6(b) && b[20] == NH_ICMPV6 && b.len() > 54 { b[54] } else { 0 }
}

fn src6(b: &[u8]) -> Ipv6 {
    b.get(14..).and_then(Ipv6View::parse).map(|v| v.src()).unwrap_or(Ipv6::UNSPECIFIED)
}

fn dst6(b: &[u8]) -> Ipv6 {
    b.get(14..).and_then(Ipv6View::parse).map(|v| v.dst()).unwrap_or(Ipv6::UNSPECIFIED)
}

/// the first frame a device sent with an ICMPv6 type (after `from`)
fn first_sent(e: &Engine, dev: u32, typ: u8, from: u64) -> (u64, Vec<u8>) {
    e.trace.hops().find(|h| h.from.0 == dev && h.t_tx >= from && icmp_type(&h.bytes) == typ).map(|h| (h.t_tx, h.bytes.to_vec())).unwrap_or_default()
}

/// a frame decodes without a problem and its checksums hold (ICMPv6, UDP and TCP over the
/// pseudo-header; a fragment only its header)
fn frame_ok(b: &[u8]) -> bool {
    let Some(v) = b.get(14..).and_then(Ipv6View::parse) else {
        return false;
    };
    if !decode::layers(b).iter().all(|l| l.problems.is_empty()) {
        return false;
    }
    if ipv6::frag(&v).is_some() {
        return true;
    }
    let Some((nh, off)) = ipv6::upper_layer(&v) else {
        return false;
    };
    let body = &v.payload()[off..];
    match nh {
        NH_ICMPV6 => Icmp6View::parse(body).is_some_and(|ic| ic.checksum_ok(v.src(), v.dst())),
        17 => netpkt::udp::UdpView::parse(body).is_some_and(|u| u.checksum_ok_v6(v.src(), v.dst())),
        6 => netpkt::tcp::TcpView::parse(body).is_some_and(|x| x.checksum_ok_v6(v.src(), v.dst())),
        _ => true,
    }
}

fn ping6_ev(e: &Engine, id: u32) -> Vec<Ping6Ev> {
    match e.job(id).map(|j| &j.kind) {
        Some(JobKind::Ping6(p)) => p.events.clone(),
        _ => Vec::new(),
    }
}

/// an echo request from `src` to `dst` with a hop limit and 16 bytes of data
fn echo6(src: Ipv6, dst: Ipv6, seq: u16, hops: u8, data: usize) -> Vec<u8> {
    let body: Vec<u8> = (0..data).map(|i| i as u8).collect();
    let mut pk = Pkt::new(HEADROOM, &body);
    Icmp6::EchoRequest { id: 99, seq }.push_v6(&mut pk, src, dst);
    Ipv6Hdr::new(src, dst, NH_ICMPV6, hops).push_onto(&mut pk);
    pk.into_vec()
}

/// `n` echo requests with hop limit 1, sent out of a port at once
#[allow(clippy::too_many_arguments)]
fn burst(e: &mut Engine, dev: u32, port: &str, to: Mac, from: Mac, src: Ipv6, dst: Ipv6, n: u16) {
    for i in 0..n {
        let f = EthHdr { dst: to, src: from, ethertype: ET_IPV6 }.frame(&echo6(src, dst, i + 1, 1, 16));
        let _ = e.apply(Action::Inject { dev, port: String::from(port), frame: f });
    }
}

/// the two LANs of the header comment
fn two_lans(seed: u64) -> Project {
    let mut p = Project::new("ipv6", seed);
    for (i, m) in ["pc", "pc", "jf-s24p", "jf-r4", "jf-r4", "server-1u"].iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 100, 0);
    }
    wire(&mut p, PC1, "eth0", SW1, "gi1/0/1");
    wire(&mut p, PC2, "eth0", SW1, "gi1/0/2");
    wire(&mut p, SW1, "gi1/0/24", R1, "gi0/0/0");
    wire(&mut p, R1, "gi0/0/1", R2, "gi0/0/0");
    wire(&mut p, R2, "gi0/0/1", SRV, "eth0");
    set(&mut p, PC1, "PC1", format!("hostname PC1\n{}", ON));
    set(&mut p, PC2, "PC2", format!("hostname PC2\n{}", ON));
    set(&mut p, SW1, "SW1", s("hostname SW1\n!\nend\n"));
    set(&mut p, R1, "R1", s(R1_CFG));
    set(&mut p, R2, "R2", s(R2_CFG));
    set(&mut p, SRV, "SRV", format!("hostname SRV\n{}sysctl -w net.ipv6.conf.eth0.autoconf=0\nip -6 addr add 2001:db8:2::10/64 dev eth0\n", ON));
    p
}

/// the two LANs after 20 s: addresses formed, routers known
fn settled(seed: u64) -> Engine {
    let mut e = engine(&two_lans(seed));
    run_to(&mut e, 20 * SEC);
    e
}

/// PC1 straight on R1's Gi0/0/0 (2001:DB8:1::1/64, IPv6 routing on, `ifcfg` in the block)
fn pair(seed: u64, ifcfg: &str) -> (Project, u32, u32) {
    let mut p = Project::new("pair", seed);
    let pc = p.add_device("pc", 0, 0).unwrap_or(0);
    let r = p.add_device("jf-r4", 200, 0).unwrap_or(0);
    wire(&mut p, pc, "eth0", r, "gi0/0/0");
    set(&mut p, pc, "PC1", format!("hostname PC1\n{}", ON));
    set(&mut p, r, "R1", format!("hostname R1\n!\nipv6 unicast-routing\n!\ninterface GigabitEthernet0/0/0\n ipv6 address 2001:DB8:1::1/64\n{} no shutdown\n!\nend\n", ifcfg));
    (p, pc, r)
}

pub fn run(t: &mut T) {
    link_local(t);
    dad(t);
    discovery(t);
    lifetimes(t);
    two_hours(t);
    neighbors(t);
    routing(t);
    too_big(t);
    limits(t);
    console_ios(t);
    console_linux(t);
    power(t);
    ipv4_only(t);
    texts(t);
    frames(t);
    determinism(t);
}

// ---------------------------------------------------------------- addresses

// link-local addresses from the MAC, DAD with its delay, lo, the probe on the wire
fn link_local(t: &mut T) {
    let mut p = Project::new("ll", 7);
    let pc = p.add_device("pc", 0, 0).unwrap_or(0);
    let r = p.add_device("jf-r4", 200, 0).unwrap_or(0);
    wire(&mut p, pc, "eth0", r, "gi0/0/0");
    set(&mut p, pc, "PC1", format!("hostname PC1\n{}", ON));
    set(&mut p, r, "R1", s("hostname R1\n!\ninterface GigabitEthernet0/0/0\n ipv6 enable\n no shutdown\n!\nend\n"));
    let mut e = engine(&p);
    t.eq("ll: boot", (e.boot_errors(pc).len(), e.boot_errors(r).len()), (0, 0));
    // the interface identifier is the MAC with the universal/local bit flipped and ff:fe in the
    // middle (RFC 4291 appendix A): 02:4a:00:00:02:01 -> fe80::4a:ff:fe00:201
    let (pll, rll) = (a6("fe80::4a:ff:fe00:101"), a6("fe80::4a:ff:fe00:201"));
    t.eq("ll: eui-64", (ll_of(pc, 0), ll_of(r, 0)), (pll, rll));
    let gi = ifn(&e, r, "GigabitEthernet0/0/0");
    let (eth, lo) = (ifn(&e, pc, "eth0"), ifn(&e, pc, "lo"));
    run_to(&mut e, 500 * MS);
    // tentative while duplicate address detection runs (at least 1 s)
    t.eq("ll: tentative", e.addrs6(r, gi).iter().map(|a| (a.addr, a.state)).collect::<Vec<_>>(), vec![(rll, AddrState::Tentative)]);
    t.check("ll: iproute2 tentative", cli(&mut e, pc, &["ip -6 addr show dev eth0"]).contains("    inet6 fe80::4a:ff:fe00:101/64 scope link tentative \n       valid_lft forever preferred_lft forever\n"));
    run_to(&mut e, 3 * SEC);
    let view = |a: Ipv6, len: u8, origin: Origin6| Addr6View { addr: a, len, origin, state: AddrState::Preferred, anycast: false, valid: None, preferred: None };
    t.eq("ll: router", e.addrs6(r, gi), vec![view(rll, 64, Origin6::LinkLocal)]);
    t.eq("ll: host", (e.addrs6(pc, eth), e.addrs6(pc, lo)), (vec![view(pll, 64, Origin6::LinkLocal)], vec![view(Ipv6::LOCALHOST, 128, Origin6::Loopback)]));
    // the device's own random numbers: the reachable time of the interface, then the delay of
    // each address before its probe
    let mut g = stream6(7, r);
    let reach = g.below(1000) as u64;
    let delay = g.below(1000) as u64 * MS;
    let loud = |n: &Note6| !matches!(n, Note6::GroupIgnored { .. } | Note6::NsIgnored { .. });
    t.eq(
        "ll: router notes",
        notes6(&e, r).into_iter().filter(|x| loud(&x.1)).collect::<Vec<_>>(),
        vec![
            (0, Note6::Up { iface: gi, ll: Some(rll) }),
            (0, Note6::DadStart { iface: gi, addr: rll, delay }),
            (delay, Note6::DadProbe { iface: gi, addr: rll }),
            (delay + SEC, Note6::DadOk { iface: gi, addr: rll }),
        ],
    );
    t.eq(
        "ll: show",
        cli(&mut e, r, &["show ipv6 interface GigabitEthernet0/0/0"]),
        format!("GigabitEthernet0/0/0 is up, line protocol is up\n  IPv6 is enabled, link-local address is FE80::4A:FF:FE00:201\n  No Virtual link-local address(es):\n  No global unicast address is configured\n  Joined group address(es):\n    FF02::1\n    FF02::1:FF00:201\n  MTU is 1500 bytes\n  ICMP error messages limited to one every 100 milliseconds\n  ICMP redirects are disabled\n  ICMP unreachables are sent\n  ND DAD is enabled, number of DAD attempts: 1\n  ND reachable time is 30000 milliseconds (using {})\n", 30 * (500 + reach)),
    );
    // the probe: from the unspecified address to the solicited-node group, hop limit 255, no
    // link-layer address (78 bytes)
    let (at, b) = first_sent(&e, r, 135, 0);
    t.eq("ll: probe sent", at, delay);
    t.eq("ll: probe macs", (b.get(0..6).map(|x| x.to_vec()), b.get(6..12).map(|x| x.to_vec())), (Some(vec![0x33, 0x33, 0xFF, 0x00, 0x02, 0x01]), Some(port_mac(r, 0).0.to_vec())));
    t.eq("ll: probe header", (src6(&b), dst6(&b), b.get(21).copied(), b.len()), (Ipv6::UNSPECIFIED, a6("ff02::1:ff00:201"), Some(255), 78));
    t.eq("ll: probe summary", decode::info(&b).1, s("Neighbor Solicitation for fe80::4a:ff:fe00:201"));
    t.check("ll: probe sums", frame_ok(&b));
    // the host: ::1 on lo at once (no detection there), eth0 after its own delay
    let mut g = stream6(7, pc);
    if lo < eth {
        let _ = (g.below(1000), g.below(1000));
    }
    let _ = g.below(1000);
    let pdelay = g.below(1000) as u64 * MS;
    t.eq("ll: host detection", times6(&e, pc, |n| matches!(n, Note6::DadOk { .. })), vec![pdelay + SEC]);
    t.eq(
        "ll: iproute2",
        cli(&mut e, pc, &["ip -6 addr"]),
        s("1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 state UNKNOWN qlen 1000\n    inet6 ::1/128 scope host \n       valid_lft forever preferred_lft forever\n2: eth0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 state UP qlen 1000\n    inet6 fe80::4a:ff:fe00:101/64 scope link \n       valid_lft forever preferred_lft forever\n"),
    );
}

// a duplicate: Linux (dadfailed, the problem) and IOS-like (%IPV6_ND-4-DUPLICATE, [DUP])
fn dad(t: &mut T) {
    let mut p = Project::new("dad", 5);
    let a = p.add_device("pc", 0, 0).unwrap_or(0);
    let b = p.add_device("pc", 0, 100).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 50).unwrap_or(0);
    let r = p.add_device("jf-r4", 200, 50).unwrap_or(0);
    wire(&mut p, a, "eth0", sw, "gi1/0/1");
    wire(&mut p, b, "eth0", sw, "gi1/0/2");
    wire(&mut p, sw, "gi1/0/24", r, "gi0/0/0");
    set(&mut p, a, "PCA", format!("hostname PCA\n{}ip -6 addr add 2001:db8::5/64 dev eth0\n", ON));
    set(&mut p, b, "PCB", format!("hostname PCB\n{}", ON));
    set(&mut p, sw, "SW1", s("hostname SW1\n!\nend\n"));
    set(&mut p, r, "R1", s("hostname R1\n!\ninterface GigabitEthernet0/0/0\n ipv6 enable\n no shutdown\n!\nend\n"));
    let mut e = engine(&p);
    run_to(&mut e, 5 * SEC);
    let (ea, eb) = (ifn(&e, a, "eth0"), ifn(&e, b, "eth0"));
    let five = a6("2001:db8::5");
    t.eq("dad: first holder", state_of(&e, a, ea, five), Some(AddrState::Preferred));
    // PCB takes the same address at 5 s: its probe reaches PCA through SW1 (2 x 866 ns), PCA
    // defends it with an advertisement to all nodes (2 x 930 ns back)
    t.eq("dad: added", cli(&mut e, b, &["ip -6 addr add 2001:db8::5/64 dev eth0"]), String::new());
    run_to(&mut e, 7 * SEC);
    let mut g = stream6(5, b);
    for _ in 0..4 {
        let _ = g.below(1000);
    }
    let d = g.below(1000) as u64 * MS;
    t.eq("dad: start", notes6(&e, b).into_iter().find(|x| matches!(x.1, Note6::DadStart { addr, .. } if addr == five)), Some((5 * SEC, Note6::DadStart { iface: eb, addr: five, delay: d })));
    let probe = 5 * SEC + d;
    t.eq("dad: probe", when6(&e, b, |n| matches!(n, Note6::DadProbe { addr, .. } if *addr == five)), Some(probe));
    t.eq("dad: heard", when6(&e, a, |n| matches!(n, Note6::NsHeard { target, from, .. } if *target == five && from.is_unspecified())), Some(probe + 2 * DAD_T));
    t.eq("dad: defended", when6(&e, a, |n| matches!(n, Note6::NaSent { target, to, solicited: false, .. } if *target == five && *to == Ipv6::ALL_NODES)), Some(probe + 2 * DAD_T));
    t.eq(
        "dad: duplicate",
        notes6(&e, b).into_iter().filter(|x| matches!(x.1, Note6::DadDup { .. })).collect::<Vec<_>>(),
        vec![(probe + 2 * DAD_T + 2 * NS_T, Note6::DadDup { iface: eb, addr: five, mac: Some(port_mac(a, 0)) })],
    );
    let (_, na) = first_sent(&e, a, 136, 0);
    t.eq("dad: advertisement", (src6(&na), dst6(&na), na.get(0..6).map(|x| x.to_vec()), na.len()), (five, Ipv6::ALL_NODES, Some(vec![0x33, 0x33, 0, 0, 0, 1]), 86));
    t.eq("dad: advertisement summary", decode::info(&na).1, s("Neighbor Advertisement 2001:db8::5 (ovr) is at 02:4a:00:00:01:01"));
    t.check("dad: advertisement sums", frame_ok(&na));
    // the address stays unused: iproute2 marks it, it routes nothing, the problems name both
    t.check("dad: dadfailed", cli(&mut e, b, &["ip -6 addr show dev eth0"]).contains("    inet6 2001:db8::5/64 scope global dadfailed tentative \n       valid_lft forever preferred_lft forever\n"));
    t.check("dad: no route from it", !e.ip6_routes(b).iter().any(|x| x.prefix == p6("2001:db8::/64")));
    let probs: Vec<(u32, Option<u16>, Vec<u32>, String)> = e.problems().into_iter().filter(|x| x.kind == ProblemKind::DuplicateIpv6).map(|x| (x.dev, x.port, x.others.clone(), x.text.clone())).collect();
    t.eq("dad: problem", probs, vec![(b, Some(0), vec![a], s("2001:db8::5 ist an eth0 doppelt: die Adressprüfung (DAD) fand 02:4a:00:00:01:01 damit."))]);
    // the IOS-like router: a log line, [DUP] in the brief table
    let _ = cli(&mut e, r, &["enable", "configure terminal", "interface GigabitEthernet0/0/0", "ipv6 address 2001:DB8::5/64", "end"]);
    run_for(&mut e, 3 * SEC);
    let log = out(&mut e, r);
    t.check("dad: ios log", log.contains(": %IPV6_ND-4-DUPLICATE: Duplicate address 2001:DB8::5 on GigabitEthernet0/0/0\n"));
    t.check("dad: ios brief", cli(&mut e, r, &["show ipv6 interface brief"]).contains("GigabitEthernet0/0/0   [up/up]\n    FE80::4A:FF:FE00:401\n    2001:DB8::5 [DUP]\n"));
    // no router on the link: three solicitations 4 s apart once the link-local address is
    // unique, then quiet (RFC 4861 6.3.7)
    run_to(&mut e, 30 * SEC);
    let ok = when6(&e, b, |n| matches!(n, Note6::DadOk { addr, .. } if addr.is_link_local())).unwrap_or(0);
    t.eq("dad: three solicitations", times6(&e, b, |n| matches!(n, Note6::RsSent { .. })), vec![ok, ok + 4 * SEC, ok + 8 * SEC]);
    let (rs_at, rs) = first_sent(&e, b, 133, 0);
    t.eq("dad: solicitation", (rs_at, src6(&rs), dst6(&rs), rs.len()), (ok, ll_of(b, 0), Ipv6::ALL_ROUTERS, 70));
    t.eq("dad: solicitation summary", decode::info(&rs).1, s("Router Solicitation from 02:4a:00:00:02:01"));
    // PCA gives the address up, PCB tries again and keeps it now
    let _ = cli(&mut e, a, &["ip -6 addr del 2001:db8::5/64 dev eth0"]);
    let _ = cli(&mut e, b, &["ip -6 addr del 2001:db8::5/64 dev eth0", "ip -6 addr add 2001:db8::5/64 dev eth0"]);
    run_for(&mut e, 3 * SEC);
    t.eq("dad: second try", state_of(&e, b, eb, five), Some(AddrState::Preferred));
    t.eq("dad: problem gone", e.problems().iter().filter(|x| x.kind == ProblemKind::DuplicateIpv6 && x.dev == b).count(), 0);
}

// ---------------------------------------------------------------- routers and SLAAC

fn discovery(t: &mut T) {
    let mut e = engine(&two_lans(11));
    t.eq("ra: boot", [PC1, PC2, SW1, R1, R2, SRV].iter().map(|d| e.boot_errors(*d).len()).sum::<usize>(), 0);
    run_to(&mut e, 60 * SEC);
    let (g0, eth) = (ifn(&e, R1, "GigabitEthernet0/0/0"), ifn(&e, PC1, "eth0"));
    let r1ll = ll_of(R1, 0);
    let pc1g = slaac_of("2001:db8:1::", PC1);
    t.eq("ra: address from the prefix", pc1g, a6("2001:db8:1::4a:ff:fe00:101"));
    // PC1 asks as soon as its link-local address is unique, then every 4 s until it hears one
    let ok = when6(&e, PC1, |n| matches!(n, Note6::DadOk { addr, .. } if addr.is_link_local())).unwrap_or(0);
    let rs = times6(&e, PC1, |n| matches!(n, Note6::RsSent { .. }));
    let got = when6(&e, PC1, |n| matches!(n, Note6::RaHeard { .. })).unwrap_or(0);
    t.eq("ra: first solicitation when unique", rs.first().copied(), Some(ok));
    t.check("ra: 4 s apart until answered", !rs.is_empty() && rs.len() <= 3 && rs.windows(2).all(|w| w[1] - w[0] == 4 * SEC) && rs.iter().all(|x| *x < got));
    // R1 answers a solicitation it heard (2 x 802 ns) after a random 0 to 500 ms, to all nodes
    let heard = times6(&e, R1, |n| matches!(n, Note6::RsHeard { iface, .. } if *iface == g0));
    let sol = notes6(&e, R1).into_iter().find(|x| matches!(x.1, Note6::RaSent { solicited: true, iface, .. } if iface == g0));
    let sol_t = sol.as_ref().map(|x| x.0).unwrap_or(0);
    t.check("ra: heard", heard.iter().any(|h| rs.iter().any(|x| *h == x + 2 * RS_T)));
    t.check("ra: answer within 500 ms", heard.iter().any(|h| sol_t >= *h && sol_t < h + 500 * MS));
    t.eq("ra: answer", sol.map(|x| x.1), Some(Note6::RaSent { iface: g0, to: Ipv6::ALL_NODES, prefixes: 1, lifetime: 1800, solicited: true }));
    // PC1 takes it (2 x 1186 ns later): the default router, the on-link prefix, R1's address in
    // its neighbor cache, an address of its own from the prefix and its MAC
    t.eq("ra: arrives", got, sol_t + 2 * RA_T);
    let learned: Vec<Note6> = notes6(&e, PC1).into_iter().filter(|x| x.0 == got && !matches!(x.1, Note6::Local { .. })).map(|x| x.1).take(5).collect();
    t.eq(
        "ra: what PC1 learns",
        learned,
        vec![
            Note6::RaHeard { iface: eth, from: r1ll, lifetime: 1800, prefixes: 1 },
            Note6::PrefixOnLink { iface: eth, prefix: p6("2001:db8:1::/64"), valid: 2_592_000 },
            Note6::RouterAdded { iface: eth, router: r1ll, lifetime: 1800 },
            Note6::Neigh { iface: eth, addr: r1ll, from: None, to: NState::Stale, mac: Some(port_mac(R1, 0)) },
            Note6::AddrNew { iface: eth, addr: Prefix6 { addr: pc1g, len: 64 }, origin: Origin6::Slaac },
        ],
    );
    // the advertisement on the wire: R1's link-local address to all nodes, hop limit 255
    let (_, ra) = first_sent(&e, R1, 134, 0);
    t.eq("ra: frame", (src6(&ra), dst6(&ra), ra.get(21).copied(), ra.len()), (r1ll, Ipv6::ALL_NODES, Some(255), 118));
    t.eq("ra: frame summary", decode::info(&ra).1, s("Router Advertisement from 02:4a:00:00:04:01"));
    t.check("ra: frame sums", frame_ok(&ra));
    // three advertisements in the first minute: at most 16 s apart, at least 3 s (RFC 4861
    // 6.2.4); the next one 150 to 200 s later
    let r1ok = when6(&e, R1, |n| matches!(n, Note6::DadOk { iface, addr } if *iface == g0 && addr.is_link_local())).unwrap_or(0);
    let ras = times6(&e, R1, |n| matches!(n, Note6::RaSent { iface, .. } if *iface == g0));
    t.check("ra: three at first", ras.len() == 3 && ras[0] <= r1ok + 16 * SEC && ras.windows(2).all(|w| w[1] - w[0] >= 3 * SEC && w[1] - w[0] <= 16 * SEC));
    // its address goes through DAD, then is used; the lifetimes count from the last advertisement
    let now = e.now();
    let last = times6(&e, PC1, |n| matches!(n, Note6::RaHeard { .. })).last().copied().unwrap_or(0);
    let left = |life: u64| last + life * SEC - now;
    t.eq(
        "ra: slaac address",
        e.addrs6(PC1, eth).into_iter().find(|x| x.origin == Origin6::Slaac),
        Some(Addr6View { addr: pc1g, len: 64, origin: Origin6::Slaac, state: AddrState::Preferred, anycast: false, valid: Some(left(2_592_000)), preferred: Some(left(604_800)) }),
    );
    t.eq(
        "ra: ip -6 route",
        cli(&mut e, PC1, &["ip -6 route"]),
        format!("::1 dev lo proto kernel metric 256 pref medium\n2001:db8:1::/64 dev eth0 proto kernel metric 256 expires {}sec pref medium\nfe80::/64 dev eth0 proto kernel metric 256 pref medium\ndefault via {} dev eth0 proto ra metric 1024 expires {}sec hoplimit 64 pref medium\n", left(2_592_000) / SEC, r1ll, left(1800) / SEC),
    );
    t.eq(
        "ra: ip -6 addr",
        cli(&mut e, PC1, &["ip -6 addr show dev eth0"]),
        format!("2: eth0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 state UP qlen 1000\n    inet6 2001:db8:1:0:4a:ff:fe00:101/64 scope global dynamic \n       valid_lft {}sec preferred_lft {}sec\n    inet6 fe80::4a:ff:fe00:101/64 scope link \n       valid_lft forever preferred_lft forever\n", left(2_592_000) / SEC, left(604_800) / SEC),
    );
    t.check("ra: router in the cache", cli(&mut e, PC1, &["ip -6 neigh"]).contains(&format!("{} dev eth0 lladdr 02:4a:00:00:04:01 router ", r1ll)));
    // R1's view of the interface: groups, the advertisement settings
    let mut g = stream6(11, R1);
    let reach = g.below(1000) as u64;
    t.eq(
        "ra: show ipv6 interface",
        cli(&mut e, R1, &["show ipv6 interface GigabitEthernet0/0/0"]),
        format!("GigabitEthernet0/0/0 is up, line protocol is up\n  IPv6 is enabled, link-local address is FE80::4A:FF:FE00:401\n  No Virtual link-local address(es):\n  Global unicast address(es):\n    2001:DB8:1::1, subnet is 2001:DB8:1::/64\n  Joined group address(es):\n    FF02::1\n    FF02::2\n    FF02::1:FF00:401\n    FF02::1:FF00:1\n  MTU is 1500 bytes\n  ICMP error messages limited to one every 100 milliseconds\n  ICMP redirects are disabled\n  ICMP unreachables are sent\n  ND DAD is enabled, number of DAD attempts: 1\n  ND reachable time is 30000 milliseconds (using {})\n  ND advertised reachable time is 0 (unspecified)\n  ND advertised retransmit interval is 0 (unspecified)\n  ND router advertisements are sent every 200 seconds\n  ND router advertisements live for 1800 seconds\n  ND advertised default router preference is Medium\n  Hosts use stateless autoconfig for addresses.\n", 30 * (500 + reach)),
    );
    // R2 routes itself: R1's advertisements on the link between them are not for it
    t.check("ra: routers ignore advertisements", times6(&e, R2, |n| matches!(n, Note6::RaIgnored { why: jfn_sim::RaWhy::Router, .. })).len() >= 2);
    // the inspector's view: the interface with its IPv6 addresses, the link-local one first
    let v = e.ifaces(R1).into_iter().find(|f| f.index == g0).map(|f| f.addrs6.iter().map(|a| (a.addr, a.origin)).collect::<Vec<_>>());
    t.eq("ra: view", v, Some(vec![(r1ll, Origin6::LinkLocal), (a6("2001:db8:1::1"), Origin6::Manual)]));
}

// SLAAC lifetimes: preferred, deprecated, expired; the router's lifetime
fn lifetimes(t: &mut T) {
    let (p, pc, r) = pair(13, " ipv6 nd prefix 2001:DB8:1::/64 300 120\n ipv6 nd ra suppress\n");
    let mut e = engine(&p);
    t.eq("life: boot", e.boot_errors(r).len(), 0);
    run_to(&mut e, 20 * SEC);
    let eth = ifn(&e, pc, "eth0");
    let a = slaac_of("2001:db8:1::", pc);
    // suppressed: R1 only answers solicitations
    t.eq("life: suppressed", count6(&e, r, 0, u64::MAX, |n| matches!(n, Note6::RaSent { solicited: false, .. })), 0);
    let got = when6(&e, pc, |n| matches!(n, Note6::RaHeard { .. })).unwrap_or(0);
    t.check("life: answered", got > 0);
    t.eq(
        "life: learned",
        notes6(&e, pc).into_iter().filter(|x| x.0 == got && matches!(x.1, Note6::PrefixOnLink { .. } | Note6::AddrNew { .. })).map(|x| x.1).collect::<Vec<_>>(),
        vec![Note6::PrefixOnLink { iface: eth, prefix: p6("2001:db8:1::/64"), valid: 300 }, Note6::AddrNew { iface: eth, addr: Prefix6 { addr: a, len: 64 }, origin: Origin6::Slaac }],
    );
    // preferred for 120 s, then deprecated: still valid, no longer for new traffic
    run_to(&mut e, got + 120 * SEC - 1);
    t.eq("life: preferred", state_of(&e, pc, eth, a), Some(AddrState::Preferred));
    run_to(&mut e, got + 150 * SEC);
    t.eq("life: deprecated at", when6(&e, pc, |n| matches!(n, Note6::AddrDeprecated { .. })), Some(got + 120 * SEC));
    t.eq("life: deprecated", state_of(&e, pc, eth, a), Some(AddrState::Deprecated));
    t.check("life: iproute2 deprecated", cli(&mut e, pc, &["ip -6 addr show dev eth0"]).contains("    inet6 2001:db8:1:0:4a:ff:fe00:101/64 scope global deprecated dynamic \n       valid_lft 150sec preferred_lft 0sec\n"));
    // valid for 300 s: then the address and the on-link prefix go together
    run_to(&mut e, got + 400 * SEC);
    t.eq(
        "life: expired",
        notes6(&e, pc).into_iter().filter(|x| matches!(x.1, Note6::AddrExpired { .. } | Note6::PrefixGone { .. })).collect::<Vec<_>>(),
        vec![(got + 300 * SEC, Note6::AddrExpired { iface: eth, addr: a }), (got + 300 * SEC, Note6::PrefixGone { iface: eth, prefix: p6("2001:db8:1::/64") })],
    );
    t.eq("life: gone", state_of(&e, pc, eth, a), None);
    let now = e.now();
    t.eq("life: routes left", cli(&mut e, pc, &["ip -6 route"]), format!("::1 dev lo proto kernel metric 256 pref medium\nfe80::/64 dev eth0 proto kernel metric 256 pref medium\ndefault via {} dev eth0 proto ra metric 1024 expires {}sec hoplimit 64 pref medium\n", ll_of(r, 0), (got + 1800 * SEC - now) / SEC));
    // the router counts for 1800 s
    run_to(&mut e, got + 1801 * SEC);
    t.eq("life: router gone", when6(&e, pc, |n| matches!(n, Note6::RouterGone { .. })), Some(got + 1800 * SEC));
    t.eq("life: no default", cli(&mut e, pc, &["ip -6 route"]), s("::1 dev lo proto kernel metric 256 pref medium\nfe80::/64 dev eth0 proto kernel metric 256 pref medium\n"));
}

// RFC 4862 5.5.3 (e): a short valid lifetime does not cut one of more than two hours below two
// hours; within two hours only a longer one counts
fn two_hours(t: &mut T) {
    let (p, pc, r) = pair(17, " ipv6 nd prefix 2001:DB8:1::/64 86400 3600\n ipv6 nd ra interval 4\n");
    let mut e = engine(&p);
    t.eq("2h: boot", e.boot_errors(r).len(), 0);
    run_to(&mut e, 30 * SEC);
    let eth = ifn(&e, pc, "eth0");
    let a = slaac_of("2001:db8:1::", pc);
    // every advertisement (3 to 4 s apart) renews the day
    let ras = times6(&e, r, |n| matches!(n, Note6::RaSent { solicited: false, .. }));
    t.check("2h: every 3 to 4 s", ras.len() >= 4 && ras.windows(2).skip(3).all(|w| w[1] - w[0] >= 3 * SEC && w[1] - w[0] <= 4 * SEC));
    let renew = notes6(&e, pc).into_iter().filter(|x| matches!(x.1, Note6::AddrLifetime { .. })).last();
    t.eq("2h: a day", renew.map(|x| x.1), Some(Note6::AddrLifetime { iface: eth, addr: a, valid: 86_400, preferred: 3600 }));
    let o = cli(&mut e, r, &["enable", "configure terminal", "interface GigabitEthernet0/0/0", "ipv6 nd prefix 2001:DB8:1::/64 600 300", "end"]);
    t.check("2h: set", !refused(&o));
    let t0 = e.now();
    run_to(&mut e, t0 + 10 * SEC);
    let after: Vec<(u64, Note6)> = notes6(&e, pc).into_iter().filter(|x| x.0 > t0 && matches!(x.1, Note6::AddrLifetime { .. })).collect();
    t.check("2h: two more", after.len() >= 2);
    if after.len() >= 2 {
        let (t1, t2) = (after[0].0, after[1].0);
        t.eq("2h: cut to two hours", after[0].1.clone(), Note6::AddrLifetime { iface: eth, addr: a, valid: 7200, preferred: 300 });
        t.eq("2h: two hours count down", after[1].1.clone(), Note6::AddrLifetime { iface: eth, addr: a, valid: ((t1 + 7200 * SEC - t2) / SEC) as u32, preferred: 300 });
        // the on-link prefix has no such rule: 600 s from the last advertisement
        let now = e.now();
        let last = after.last().map(|x| x.0).unwrap_or(0);
        t.check("2h: prefix follows", cli(&mut e, pc, &["ip -6 route"]).contains(&format!("2001:db8:1::/64 dev eth0 proto kernel metric 256 expires {}sec pref medium\n", (last + 600 * SEC - now) / SEC)));
    }
    // a longer one than what is left counts at once
    let _ = cli(&mut e, r, &["configure terminal", "interface GigabitEthernet0/0/0", "ipv6 nd prefix 2001:DB8:1::/64 10000 300", "end"]);
    let t3 = e.now();
    run_to(&mut e, t3 + 5 * SEC);
    let next = notes6(&e, pc).into_iter().find(|x| x.0 > t3 && matches!(x.1, Note6::AddrLifetime { .. })).map(|x| x.1);
    t.eq("2h: longer counts", next, Some(Note6::AddrLifetime { iface: eth, addr: a, valid: 10_000, preferred: 300 }));
}

// ---------------------------------------------------------------- the neighbor cache

fn neighbors(t: &mut T) {
    let mut p = Project::new("nud", 19);
    let a = p.add_device("pc", 0, 0).unwrap_or(0);
    let b = p.add_device("pc", 0, 100).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 50).unwrap_or(0);
    wire(&mut p, a, "eth0", sw, "gi1/0/1");
    wire(&mut p, b, "eth0", sw, "gi1/0/2");
    set(&mut p, a, "PCA", format!("hostname PCA\n{}ip -6 addr add 2001:db8::1/64 dev eth0\n", ON));
    set(&mut p, b, "PCB", format!("hostname PCB\n{}ip -6 addr add 2001:db8::2/64 dev eth0\n", ON));
    set(&mut p, sw, "SW1", s("hostname SW1\n!\nend\n"));
    let mut e = engine(&p);
    let t0 = 5 * SEC;
    run_to(&mut e, t0);
    let (ea, eb) = (ifn(&e, a, "eth0"), ifn(&e, b, "eth0"));
    let (one, two) = (a6("2001:db8::1"), a6("2001:db8::2"));
    let (ma, mb) = (port_mac(a, 0), port_mac(b, 0));
    let id = e.ping6(a, two, Ping6Opts { count: 1, ..Ping6Opts::default() }).unwrap_or(0);
    run_to(&mut e, t0 + SEC);
    // PCA: INCOMPLETE and a solicitation to ff02::1:ff00:2 from the address of the waiting
    // packet; PCB learns PCA from it (STALE), answers (DELAY: it sent to a STALE entry), and
    // the answer makes PCA's entry REACHABLE; the echo goes then
    let ns = 2 * NS_T;
    let of = |e: &Engine, dev: u32, who: Ipv6| notes6(e, dev).into_iter().filter(|x| matches!(&x.1, Note6::Neigh { addr, .. } if *addr == who)).collect::<Vec<_>>();
    t.eq(
        "nud: a resolves b",
        of(&e, a, two),
        vec![(t0, Note6::Neigh { iface: ea, addr: two, from: None, to: NState::Incomplete, mac: None }), (t0 + 2 * ns, Note6::Neigh { iface: ea, addr: two, from: Some(NState::Incomplete), to: NState::Reachable, mac: Some(mb) })],
    );
    t.eq(
        "nud: b learns a",
        of(&e, b, one),
        vec![(t0 + ns, Note6::Neigh { iface: eb, addr: one, from: None, to: NState::Stale, mac: Some(ma) }), (t0 + ns, Note6::Neigh { iface: eb, addr: one, from: Some(NState::Stale), to: NState::Delay, mac: Some(ma) })],
    );
    t.eq("nud: echo", ping6_ev(&e, id), vec![Ping6Ev::Reply { seq: 1, from: two, bytes: 64, hops: 64, rtt: 2 * ns + 4 * RA_T }]);
    let (at, nsf) = first_sent(&e, a, 135, t0);
    t.eq("nud: solicitation", (at, src6(&nsf), dst6(&nsf), nsf.len()), (t0, one, a6("ff02::1:ff00:2"), 86));
    t.eq("nud: solicitation summary", decode::info(&nsf).1, s("Neighbor Solicitation for 2001:db8::2 from 02:4a:00:00:01:01"));
    let (at, naf) = first_sent(&e, b, 136, t0);
    t.eq("nud: advertisement", (at, src6(&naf), dst6(&naf), naf.len()), (t0 + ns, two, one, 86));
    t.eq("nud: advertisement summary", decode::info(&naf).1, s("Neighbor Advertisement 2001:db8::2 (sol, ovr) is at 02:4a:00:00:02:01"));
    t.eq("nud: tables", (cli(&mut e, a, &["ip -6 neigh"]), cli(&mut e, b, &["ip -6 neigh"])), (s("2001:db8::2 dev eth0 lladdr 02:4a:00:00:02:01 REACHABLE\n"), s("2001:db8::1 dev eth0 lladdr 02:4a:00:00:01:01 DELAY\n")));
    // 5 s in DELAY without a confirmation: PROBE with a solicitation straight to PCA (from PCB's
    // link-local address, 86 bytes); PCA's answer to it carries no MAC (78 bytes)
    run_to(&mut e, t0 + 10 * SEC);
    let tp = t0 + ns + 5 * SEC;
    let steps: Vec<(u64, Note6)> = notes6(&e, b).into_iter().filter(|x| x.0 >= tp && matches!(&x.1, Note6::Neigh { addr, .. } | Note6::NsSent { target: addr, .. } if *addr == one)).collect();
    t.eq(
        "nud: delay, probe, reachable",
        steps,
        vec![
            (tp, Note6::Neigh { iface: eb, addr: one, from: Some(NState::Delay), to: NState::Probe, mac: Some(ma) }),
            (tp, Note6::NsSent { iface: eb, target: one, unicast: true }),
            (tp + ns + 2 * DAD_T, Note6::Neigh { iface: eb, addr: one, from: Some(NState::Probe), to: NState::Reachable, mac: Some(ma) }),
        ],
    );
    let (at, probe) = first_sent(&e, b, 135, tp);
    t.eq("nud: probe", (at, probe.get(0..6).map(|x| x.to_vec()), src6(&probe), dst6(&probe)), (tp, Some(ma.0.to_vec()), ll_of(b, 0), one));
    // REACHABLE lasts PCA's ReachableTime: 15 to 45 s from its own stream (lo first, then eth0)
    let mut g = stream6(19, a);
    let _ = (g.below(1000), g.below(1000));
    let reach = 30 * MS * (500 + g.below(1000) as u64);
    run_to(&mut e, t0 + 50 * SEC);
    t.eq("nud: stale after its time", when6(&e, a, |n| matches!(n, Note6::Neigh { addr, to: NState::Stale, .. } if *addr == two)), Some(t0 + 2 * ns + reach));
    // nobody has 2001:db8::99: three solicitations a second apart, then both waiting echoes
    // fail with "address unreachable"
    let t1 = 60 * SEC;
    run_to(&mut e, t1);
    let _ = e.take_reports();
    let _ = e.cli_line(a, "ping -c 2 2001:db8::99");
    run_for(&mut e, 500 * MS);
    let nn = a6("2001:db8::99");
    t.eq("nud: incomplete", e.ip6_neighbors(a).iter().find(|x| x.ip == nn).map(|x| (x.state, x.mac)), Some((NState::Incomplete, None)));
    run_for(&mut e, 4 * SEC);
    t.eq(
        "nud: unreachable",
        out(&mut e, a),
        s("PING 2001:db8::99(2001:db8::99) 56 data bytes\nFrom 2001:db8::1 icmp_seq=1 Destination unreachable: Address unreachable\nFrom 2001:db8::1 icmp_seq=2 Destination unreachable: Address unreachable\n\n--- 2001:db8::99 ping statistics ---\n2 packets transmitted, 0 received, +2 errors, 100% packet loss, time 1000ms\n"),
    );
    t.eq("nud: three solicitations", times6(&e, a, |n| matches!(n, Note6::NsSent { target, .. } if *target == nn)), vec![t1, t1 + SEC, t1 + 2 * SEC]);
    t.eq("nud: given up", when6(&e, a, |n| matches!(n, Note6::NeighFailed { addr } if *addr == nn)), Some(t1 + 3 * SEC));
    // PCB is switched off while PCA's entry is STALE: DELAY with the next packet, PROBE 5 s
    // later, three probes a second apart, then the entry goes
    let t2 = 100 * SEC;
    run_to(&mut e, t2);
    t.eq("nud: stale before", e.ip6_neighbors(a).iter().find(|x| x.ip == two).map(|x| x.state), Some(NState::Stale));
    let _ = e.apply(Action::Power { dev: b, on: false });
    let _ = e.ping6(a, two, Ping6Opts { count: 1, ..Ping6Opts::default() });
    run_to(&mut e, t2 + 10 * SEC);
    let steps: Vec<(u64, Note6)> = notes6(&e, a).into_iter().filter(|x| x.0 >= t2 && matches!(&x.1, Note6::Neigh { addr, .. } | Note6::NsSent { target: addr, .. } | Note6::NeighFailed { addr } if *addr == two)).collect();
    t.eq(
        "nud: probes fail",
        steps,
        vec![
            (t2, Note6::Neigh { iface: ea, addr: two, from: Some(NState::Stale), to: NState::Delay, mac: Some(mb) }),
            (t2 + 5 * SEC, Note6::Neigh { iface: ea, addr: two, from: Some(NState::Delay), to: NState::Probe, mac: Some(mb) }),
            (t2 + 5 * SEC, Note6::NsSent { iface: ea, target: two, unicast: true }),
            (t2 + 6 * SEC, Note6::NsSent { iface: ea, target: two, unicast: true }),
            (t2 + 7 * SEC, Note6::NsSent { iface: ea, target: two, unicast: true }),
            (t2 + 8 * SEC, Note6::NeighFailed { addr: two }),
        ],
    );
    t.check("nud: entry gone", !e.ip6_neighbors(a).iter().any(|x| x.ip == two));
}

// ---------------------------------------------------------------- forwarding

fn routing(t: &mut T) {
    let mut e = settled(11);
    let (srv, pc1g) = (a6("2001:db8:2::10"), slaac_of("2001:db8:1::", PC1));
    let g1 = ifn(&e, R1, "GigabitEthernet0/0/1");
    // R1's table: connected and local routes of both interfaces, the static route resolved to
    // its interface, the multicast range
    let codes = "Codes: C - connected network, L - address of this device, S - static route,\n       ND - default router from advertisements, NDp - prefix from advertisements\n\n";
    t.eq(
        "route: show R1",
        cli(&mut e, R1, &["show ipv6 route"]),
        format!("IPv6 Routing Table - default - 6 entries\n{}C   2001:DB8:1::/64 [0/0]\n     via GigabitEthernet0/0/0, directly connected\nL   2001:DB8:1::1/128 [0/0]\n     via GigabitEthernet0/0/0, receive\nS   2001:DB8:2::/64 [1/0]\n     via 2001:DB8:12::2\nC   2001:DB8:12::/64 [0/0]\n     via GigabitEthernet0/0/1, directly connected\nL   2001:DB8:12::1/128 [0/0]\n     via GigabitEthernet0/0/1, receive\nL   FF00::/8 [0/0]\n     via Null0, receive\n", codes),
    );
    t.check("route: static resolved", e.ip6_routes(R1).iter().any(|r| r.src == Src6::Static && r.prefix == p6("2001:db8:2::/64") && r.via == Some(a6("2001:db8:12::2")) && r.iface == Some(g1)));
    t.eq("route: link-local next hop", cli(&mut e, R2, &["show ipv6 route static"]), format!("IPv6 Routing Table - default - 1 entries\n{}S   ::/0 [1/0]\n     via FE80::1, GigabitEthernet0/0/0\n", codes));
    // PC1 to SRV: two routers, so the answer comes with hop limit 62
    let o = cli_for(&mut e, PC1, "ping -c 2 2001:db8:2::10", 3 * SEC);
    t.check("route: ping", o.starts_with("PING 2001:db8:2::10(2001:db8:2::10) 56 data bytes\n64 bytes from 2001:db8:2::10: icmp_seq=1 ttl=62 time=") && o.contains("\n64 bytes from 2001:db8:2::10: icmp_seq=2 ttl=62 time=") && o.contains("2 packets transmitted, 2 received, 0% packet loss, time 1000ms\n"));
    // the second echo finds every neighbor: 8 cables of 1186 ns
    let id = e.ping6(PC1, srv, Ping6Opts { count: 1, ..Ping6Opts::default() }).unwrap_or(0);
    run_for(&mut e, SEC);
    t.eq("route: round trip", ping6_ev(&e, id), vec![Ping6Ev::Reply { seq: 1, from: srv, bytes: 64, hops: 62, rtt: 8 * RA_T }]);
    // traceroute: the routers answer from the address of the interface the probe came in on
    let o = cli_for(&mut e, PC1, "traceroute6 -n 2001:db8:2::10", 5 * SEC);
    let hops: Vec<String> = o.lines().map(|l| l.split("  ").take(2).collect::<Vec<_>>().join("  ")).collect();
    t.eq("route: traceroute6", hops, vec![s("traceroute to 2001:db8:2::10 (2001:db8:2::10), 30 hops max, 80 byte packets"), s(" 1  2001:db8:1::1"), s(" 2  2001:db8:12::2"), s(" 3  2001:db8:2::10")]);
    let o = cli_for(&mut e, R1, "ping 2001:DB8:2::10", 2 * SEC);
    t.eq("route: ios ping", o, s("Ctrl+Shift+6 stops the ping.\nPinging 2001:DB8:2::10 with 5 echo requests of 100 bytes, 2 s timeout each:\n!!!!!\nSuccess rate is 100 percent (5/5), round-trip min/avg/max = 0/0/0 ms\n"));
    let o = cli_for(&mut e, R1, "traceroute ipv6 2001:DB8:2::10", 2 * SEC);
    t.eq("route: ios traceroute", o, s("Ctrl+Shift+6 stops the traceroute.\nRoute to 2001:DB8:2::10, up to 30 hops:\n  1 2001:DB8:12::2 0 msec 0 msec 0 msec\n  2 2001:DB8:2::10 0 msec 0 msec 0 msec\n"));
    // hop limit 1: R1 says so
    let o = cli_for(&mut e, PC1, "ping -c 1 -t 1 2001:db8:2::10", 2 * SEC);
    t.check("route: hop limit", o.contains("From 2001:db8:1::1 icmp_seq=1 Time exceeded: Hop limit\n"));
    t.check("route: hop limit note", when6(&e, R1, |n| *n == Note6::HopLimit { src: pc1g }).is_some());
    // no route at R1
    let o = cli_for(&mut e, PC1, "ping -c 1 2001:db8:99::1", 2 * SEC);
    t.check("route: no route", o.contains("From 2001:db8:1::1 icmp_seq=1 Destination unreachable: No route\n"));
    // a loop between R1 and R2: the hop limit ends it at R2 (63, 61, ... 1)
    let o = cli(&mut e, R1, &["enable", "configure terminal", "ipv6 route 2001:DB8:3::/64 2001:DB8:12::2", "end"]);
    t.check("route: loop set", !refused(&o));
    let t0 = e.now();
    let o = cli_for(&mut e, PC1, "ping -c 1 2001:db8:3::1", 2 * SEC);
    t.check("route: loop ends", o.contains("From 2001:db8:12::2 icmp_seq=1 Time exceeded: Hop limit\n"));
    let fwd = |dev: u32| count6(&e, dev, t0, u64::MAX, |n| matches!(n, Note6::Routed { dst, .. } if *dst == a6("2001:db8:3::1")));
    t.eq("route: forwarded until the hop limit ends", (fwd(R1), fwd(R2)), (32, 31));
    // a wider static route with a higher distance does not win over the /64
    let o = cli(&mut e, R1, &["configure terminal", "ipv6 route 2001:DB8::/32 2001:DB8:12::2 200", "end"]);
    t.check("route: wide set", !refused(&o));
    t.check("route: wide shown", cli(&mut e, R1, &["show ipv6 route static"]).contains("S   2001:DB8::/32 [200/0]\n     via 2001:DB8:12::2\n"));
    let id = e.ping6(PC1, srv, Ping6Opts { count: 1, ..Ping6Opts::default() }).unwrap_or(0);
    run_for(&mut e, SEC);
    t.check("route: longest prefix", ping6_ev(&e, id).iter().any(|x| matches!(x, Ping6Ev::Reply { .. })));
    // the console's checks
    let o = cli(&mut e, R1, &["configure terminal", "ipv6 route 2001:DB8:9::/64 FE80::2", "ipv6 route 2001:DB8:9::1/64 2001:DB8:12::2", "no ipv6 route 2001:DB8:8::/64 2001:DB8:12::2", "end"]);
    t.eq("route: refusals", o.lines().filter(|l| l.starts_with('%')).collect::<Vec<_>>(), vec!["% Interface has to be specified for a link-local nexthop", "% Invalid prefix: the address has bits beyond its length", "%No matching route to delete"]);
    let o = cli(&mut e, R1, &["configure terminal", "no ipv6 route 2001:DB8::/32", "no ipv6 route 2001:DB8:3::/64 2001:DB8:12::2", "end"]);
    t.check("route: removed", !refused(&o) && !e.ip6_routes(R1).iter().any(|r| r.prefix.len == 32 || r.prefix == p6("2001:db8:3::/64")));
    // the neighbors R1 knows now
    let o = cli(&mut e, R1, &["show ipv6 neighbors"]);
    t.check("route: show neighbors", o.starts_with("IPv6 Address                              Age Link-layer Addr State Interface\n") && o.contains("2001:DB8:12::2                              0 024a.0000.0501 ") && o.contains(" Gi0/0/1\n"));
}

// Packet Too Big, the sender's fragments, path MTUs and reassembly given up
fn too_big(t: &mut T) {
    let mut e = settled(11);
    let o = cli(&mut e, R2, &["enable", "configure terminal", "interface GigabitEthernet0/0/1", "ipv6 mtu 1280", "ipv6 nd ra mtu suppress", "end"]);
    t.check("pmtu: set", !refused(&o));
    let srv = a6("2001:db8:2::10");
    let t0 = e.now();
    let o = cli_for(&mut e, PC1, "ping -c 3 -s 1400 2001:db8:2::10", 4 * SEC);
    // the first is too big for R2's link to SRV: R2 drops it (routers never fragment) and names
    // the MTU from the address the probe came in on
    t.check("pmtu: too big", o.starts_with("PING 2001:db8:2::10(2001:db8:2::10) 1400 data bytes\nFrom 2001:db8:12::2 icmp_seq=1 Packet too big: mtu=1280\n1408 bytes from 2001:db8:2::10: icmp_seq=2 ttl=62 time="));
    t.check("pmtu: then through", o.contains("\n1408 bytes from 2001:db8:2::10: icmp_seq=3 ttl=62 time=") && o.contains("3 packets transmitted, 2 received, +1 errors, "));
    t.eq("pmtu: router drops", count6(&e, R2, t0, u64::MAX, |n| *n == Note6::TooBig { mtu: 1280 }), 1);
    t.eq("pmtu: learned", e.ip6(PC1).map(|v| v.pmtu.get(&srv).map(|x| x.0)), Some(Some(1280)));
    // PC1 sends the others in two fragments: 1232 bytes of data (1280 in all), then 176
    let frags: Vec<(u32, usize, bool, usize)> = e
        .trace
        .hops()
        .filter(|h| h.from.0 == PC1 && h.t_tx > t0 && is6(&h.bytes))
        .filter_map(|h| {
            let v = Ipv6View::parse(&h.bytes[14..])?;
            let (f, _) = ipv6::frag(&v)?;
            Some((f.id, f.offset, f.more, v.packet().len()))
        })
        .collect();
    t.eq("pmtu: fragments", frags.iter().map(|x| (x.1, x.2, x.3)).collect::<Vec<_>>(), vec![(0, true, 1280), (1232, false, 224), (0, true, 1280), (1232, false, 224)]);
    t.check("pmtu: one id per packet", frags.len() == 4 && frags[0].0 == frags[1].0 && frags[2].0 == frags[3].0 && frags[0].0 != frags[2].0);
    t.eq("pmtu: srv puts them together", count6(&e, SRV, t0, u64::MAX, |n| *n == Note6::Reassembled { bytes: 1448 }), 2);
    t.eq("pmtu: fragmented", count6(&e, PC1, t0, u64::MAX, |n| *n == Note6::Fragmented { parts: 2, mtu: 1280 }), 2);
    // "-M do": no fragments, iputils' local error
    let o = cli_for(&mut e, PC1, "ping -c 1 -M do -s 1400 2001:db8:2::10", 2 * SEC);
    t.check("pmtu: may not fragment", o.contains("ping: local error: message too long, mtu=1280\n"));
    // the path MTU is tried again after 10 minutes (RFC 8201)
    let learned = when6(&e, PC1, |n| matches!(n, Note6::PmtuLearned { .. })).unwrap_or(0);
    run_to(&mut e, learned + 600 * SEC - 1);
    t.check("pmtu: kept 10 minutes", e.ip6(PC1).is_some_and(|v| v.pmtu.contains_key(&srv)));
    run_to(&mut e, learned + 600 * SEC);
    t.check("pmtu: forgotten", e.ip6(PC1).is_some_and(|v| v.pmtu.is_empty()));
    // a first fragment alone: SRV gives up after 60 s and tells the sender (Time Exceeded,
    // code 1: the first fragment had come)
    let pc1g = slaac_of("2001:db8:1::", PC1);
    let parts = ipv6::fragment(&echo6(pc1g, srv, 1, 60, 2000), 1280, 0x4242).unwrap_or_default();
    let f = EthHdr { dst: port_mac(SRV, 0), src: port_mac(R2, 1), ethertype: ET_IPV6 }.frame(&parts.first().cloned().unwrap_or_default());
    let t3 = e.now();
    let _ = e.apply(Action::Inject { dev: R2, port: s("gi0/0/1"), frame: f });
    run_to(&mut e, t3 + 61 * SEC);
    let arrive = t3 + (1294 + 24) * 8 + 50;
    t.eq("reasm: given up", when6(&e, SRV, |n| *n == Note6::ReasmTimeout), Some(arrive + 60 * SEC));
    t.eq("reasm: told", when6(&e, SRV, |n| matches!(n, Note6::IcmpSent { typ: 3, code: 1, .. })), Some(arrive + 60 * SEC));
}

// the limits of ICMPv6 errors: the IOS-like bucket (10, one more every 100 ms), Linux per
// destination (one every 250 ms behind a /64, six saved up)
fn limits(t: &mut T) {
    let mut e = settled(11);
    let pc1g = slaac_of("2001:db8:1::", PC1);
    let pc2g = slaac_of("2001:db8:1::", PC2);
    // R1 and PC2 learn PC1's address first
    let _ = e.ping6(PC1, a6("2001:db8:1::1"), Ping6Opts { count: 1, ..Ping6Opts::default() });
    let _ = e.ping6(PC1, pc2g, Ping6Opts { count: 1, ..Ping6Opts::default() });
    run_for(&mut e, SEC);
    let sent = |e: &Engine, dev: u32, a: u64, b: u64| count6(e, dev, a, b, |n| matches!(n, Note6::IcmpSent { typ: 3, code: 0, .. }));
    let held = |e: &Engine, dev: u32, a: u64, b: u64| count6(e, dev, a, b, |n| *n == Note6::IcmpLimited);
    let t0 = e.now();
    burst(&mut e, SW1, "gi1/0/24", port_mac(R1, 0), port_mac(PC1, 0), pc1g, a6("2001:db8:2::10"), 15);
    run_for(&mut e, 250 * MS);
    let t1 = e.now();
    burst(&mut e, SW1, "gi1/0/24", port_mac(R1, 0), port_mac(PC1, 0), pc1g, a6("2001:db8:2::10"), 3);
    run_for(&mut e, SEC);
    let t2 = e.now();
    t.eq("limit: ios at once", (sent(&e, R1, t0, t1), held(&e, R1, t0, t1)), (10, 5));
    t.eq("limit: ios 250 ms later", (sent(&e, R1, t1, t2), held(&e, R1, t1, t2)), (2, 1));
    // a Linux host that forwards: errors to PC1 (behind a /64 route) one every 250 ms
    let o = cli(&mut e, PC2, &["sysctl -w net.ipv6.conf.all.forwarding=1"]);
    t.eq("limit: forwarding on", o, s("net.ipv6.conf.all.forwarding = 1\n"));
    burst(&mut e, SW1, "gi1/0/2", port_mac(PC2, 0), port_mac(PC1, 0), pc1g, a6("2001:db8:77::1"), 15);
    run_for(&mut e, 600 * MS);
    let t3 = e.now();
    burst(&mut e, SW1, "gi1/0/2", port_mac(PC2, 0), port_mac(PC1, 0), pc1g, a6("2001:db8:77::1"), 3);
    run_for(&mut e, SEC);
    let t4 = e.now();
    t.eq("limit: linux at once", (sent(&e, PC2, t2, t3), held(&e, PC2, t2, t3)), (6, 9));
    t.eq("limit: linux 600 ms later", (sent(&e, PC2, t3, t4), held(&e, PC2, t3, t4)), (2, 1));
    // a Linux device that forwards sends no advertisements of its own (no radvd)
    run_for(&mut e, 60 * SEC);
    t.eq("limit: linux router quiet", count6(&e, PC2, 0, u64::MAX, |n| matches!(n, Note6::RaSent { .. })), 0);
}

// ---------------------------------------------------------------- consoles

fn console_ios(t: &mut T) {
    let mut e = settled(11);
    let o = cli(&mut e, R1, &["enable", "configure terminal"]);
    t.check("ios: modes", !refused(&o) && e.cli_prompt(R1) == "R1(config)#");
    let h = e.cli_help(R1, "ipv6 ");
    t.check("ios: global help", h.contains("unicast-routing") && h.contains("Forward IPv6 packets and act as an IPv6 router") && h.contains("route") && h.contains("Add a static IPv6 route"));
    let _ = cli(&mut e, R1, &["interface GigabitEthernet0/0/0"]);
    let h = e.cli_help(R1, "ipv6 ");
    t.check("ios: interface help", h.contains("enable") && h.contains("Use IPv6 here, with a link-local address only") && h.contains("address") && h.contains("nd"));
    let h = e.cli_help(R1, "ipv6 address 2001:DB8:1::1/64 ");
    t.check("ios: address help", h.contains("eui-64") && h.contains("Take the interface identifier from the MAC") && h.contains("anycast"));
    let o = cli(&mut e, R1, &["ipv6 address FF02::5/64", "ipv6 address 2001:DB8:12::7/64", "ipv6 address 2001:DB8:5::/64 eui-64", "ipv6 nd prefix 2001:DB8:5::/64 3600 1800", "ipv6 nd ra lifetime 600", "ipv6 nd ra interval 30", "end"]);
    t.eq("ios: refusals", o.lines().filter(|l| l.starts_with('%')).collect::<Vec<_>>(), vec!["% FF02::5/64 is not a valid IPv6 address for an interface", "% 2001:DB8:12::/64 overlaps with GigabitEthernet0/0/1"]);
    run_for(&mut e, 3 * SEC);
    let eui = Ipv6::eui64_in(a6("2001:db8:5::"), port_mac(R1, 0));
    let g0 = ifn(&e, R1, "GigabitEthernet0/0/0");
    t.eq("ios: eui-64 address", state_of(&e, R1, g0, eui), Some(AddrState::Preferred));
    t.eq(
        "ios: brief",
        cli(&mut e, R1, &["show ipv6 interface brief"]),
        s("GigabitEthernet0/0/0   [up/up]\n    FE80::4A:FF:FE00:401\n    2001:DB8:1::1\n    2001:DB8:5:0:4A:FF:FE00:401\nGigabitEthernet0/0/1   [up/up]\n    FE80::1\n    2001:DB8:12::1\nGigabitEthernet0/0/2   [administratively down/down]\n    unassigned\nGigabitEthernet0/0/3   [administratively down/down]\n    unassigned\nTenGigabitEthernet0/1/0 [administratively down/down]\n    unassigned\nTenGigabitEthernet0/1/1 [administratively down/down]\n    unassigned\nSerial0/2/0            [administratively down/down]\n    unassigned\nSerial0/2/1            [administratively down/down]\n    unassigned\n"),
    );
    let run = e.running_config(R1);
    t.check("ios: running routing", run.contains("\nipv6 unicast-routing\n!\n"));
    t.check("ios: running interface", run.contains("interface GigabitEthernet0/0/0\n no ip address\n ipv6 address 2001:DB8:1::1/64\n ipv6 address 2001:DB8:5::/64 eui-64\n ipv6 nd prefix 2001:DB8:5::/64 3600 1800\n ipv6 nd ra lifetime 600\n ipv6 nd ra interval 30\n!\n"));
    t.check("ios: running link-local", run.contains("interface GigabitEthernet0/0/1\n no ip address\n ipv6 address FE80::1 link-local\n ipv6 address 2001:DB8:12::1/64\n!\n"));
    t.check("ios: running route", run.contains("\nipv6 route 2001:DB8:2::/64 2001:DB8:12::2\n!\n"));
    // read back: the same configuration
    let mut p = two_lans(11);
    if let Some(d) = p.device_mut(R1) {
        d.startup_config = run.clone();
    }
    let e2 = engine(&p);
    t.eq("ios: read back: boot errors", e2.boot_errors(R1).len(), 0);
    t.eq("ios: read back", e2.running_config(R1), run);
    // the advertisements say what was set: two prefixes, 600 s
    let t0 = e.now();
    run_for(&mut e, 40 * SEC);
    t.check("ios: advertised", notes6(&e, R1).iter().any(|x| x.0 >= t0 && x.1 == Note6::RaSent { iface: g0, to: Ipv6::ALL_NODES, prefixes: 2, lifetime: 600, solicited: false }));
    let eth = ifn(&e, PC1, "eth0");
    t.eq("ios: second slaac address", state_of(&e, PC1, eth, slaac_of("2001:db8:5::", PC1)), Some(AddrState::Preferred));
    // debugging: Neighbor Discovery and ICMPv6 in our own words
    let o = cli(&mut e, R1, &["debug ipv6 nd", "debug ipv6 icmp", "clear ipv6 neighbors"]);
    t.eq("ios: debug on", o, s("Debug output for IPv6 Neighbor Discovery: on\nDebug output for ICMPv6: on\n"));
    t.eq("ios: cleared", cli(&mut e, R1, &["show ipv6 neighbors"]), s("IPv6 Address                              Age Link-layer Addr State Interface\n"));
    let _ = e.take_reports();
    let _ = e.ping6(PC1, a6("2001:db8:1::1"), Ping6Opts { count: 1, ..Ping6Opts::default() });
    run_for(&mut e, SEC);
    let o = out(&mut e, R1);
    t.check("ios: debug nd", o.contains(": ICMPv6-ND: NS for 2001:DB8:1::1 from 2001:DB8:1:0:4A:FF:FE00:101 on GigabitEthernet0/0/0\n") && o.contains(": ICMPv6-ND: new entry 2001:DB8:1:0:4A:FF:FE00:101 on GigabitEthernet0/0/0 (STALE)\n"));
    t.check("ios: debug icmp", o.contains(": ICMPv6: echo request from 2001:DB8:1:0:4A:FF:FE00:101\n") && o.contains(": ICMPv6: echo reply sent to 2001:DB8:1:0:4A:FF:FE00:101\n"));
    t.check("ios: show debugging", cli(&mut e, R1, &["show debugging"]).contains("Debug output for ICMPv6: on\nDebug output for IPv6 Neighbor Discovery: on\n"));
    let _ = cli(&mut e, R1, &["undebug all"]);
    let _ = e.ping6(PC1, a6("2001:db8:1::1"), Ping6Opts { count: 1, ..Ping6Opts::default() });
    run_for(&mut e, SEC);
    t.check("ios: debug off", !out(&mut e, R1).contains("ICMPv6"));
    // a link-local destination: IOS asks for the interface
    let o = cli(&mut e, R1, &["ping FE80::4A:FF:FE00:101"]);
    t.eq("ios: asks", (o, e.cli_prompt(R1)), (String::new(), s("Output Interface: ")));
    let o = cli_for(&mut e, R1, "GigabitEthernet0/0/0", 2 * SEC);
    t.eq("ios: link-local ping", o, s("Ctrl+Shift+6 stops the ping.\nPinging FE80::4A:FF:FE00:101 with 5 echo requests of 100 bytes, 2 s timeout each:\nOutput interface: GigabitEthernet0/0/0\n!!!!!\nSuccess rate is 100 percent (5/5), round-trip min/avg/max = 0/0/0 ms\n"));
    t.eq("ios: bad host", cli(&mut e, R1, &["ping ipv6 server"]), s("% Unrecognized host or address, or protocol not running.\n\n"));
    // removing the addresses: IPv6 stays off where nothing asks for it
    let o = cli(&mut e, R1, &["configure terminal", "interface GigabitEthernet0/0/1", "no ipv6 address", "end"]);
    t.check("ios: no address", !refused(&o));
    let g1 = ifn(&e, R1, "GigabitEthernet0/0/1");
    t.eq("ios: off again", (e.addrs6(R1, g1).len(), e.ip6(R1).is_some_and(|v| v.on(g1))), (0, false));
}

fn console_linux(t: &mut T) {
    let mut e = settled(11);
    let pc2g = slaac_of("2001:db8:1::", PC2);
    t.eq("linux: sysctl", cli(&mut e, PC2, &["sysctl net.ipv6.conf.all.disable_ipv6 net.ipv6.conf.eth0.forwarding net.ipv6.conf.eth0.accept_ra"]), s("net.ipv6.conf.all.disable_ipv6 = 0\nnet.ipv6.conf.eth0.forwarding = 0\nnet.ipv6.conf.eth0.accept_ra = 1\n"));
    t.eq("linux: sysctl unknown", cli(&mut e, PC2, &["sysctl net.ipv6.conf.eth9.forwarding"]), s("sysctl: cannot stat /proc/sys/net/ipv6/conf/eth9/forwarding: No such file or directory\n"));
    t.eq("linux: route get", cli(&mut e, PC2, &["ip -6 route get 2001:db8:2::10"]), format!("2001:db8:2::10 from :: via {} dev eth0 proto ra src {} metric 1024 pref medium\n", ll_of(R1, 0), pc2g));
    // routes: add, again, delete, again
    t.eq("linux: route add", cli(&mut e, PC2, &["ip -6 route add 2001:db8:9::/64 via 2001:db8:1::1"]), String::new());
    t.check("linux: route listed", cli(&mut e, PC2, &["ip -6 route"]).contains("2001:db8:9::/64 via 2001:db8:1::1 dev eth0 metric 1024 pref medium\n"));
    t.eq("linux: route twice", cli(&mut e, PC2, &["ip -6 route add 2001:db8:9::/64 via 2001:db8:1::1"]), s("RTNETLINK answers: File exists\n"));
    t.eq("linux: route via link-local without dev", cli(&mut e, PC2, &["ip -6 route add 2001:db8:8::/64 via fe80::1"]), s("Error: Nexthop has invalid gateway.\n"));
    t.eq("linux: route via link-local with dev", cli(&mut e, PC2, &["ip -6 route add 2001:db8:8::/64 via fe80::1 dev eth0"]), String::new());
    let run = e.running_config(PC2);
    t.check("linux: configuration", run.contains("sysctl -w net.ipv6.conf.all.disable_ipv6=0\n") && run.contains("ip -6 route add 2001:db8:9::/64 via 2001:db8:1::1\nip -6 route add 2001:db8:8::/64 via fe80::1 dev eth0\n"));
    t.eq("linux: route del", cli(&mut e, PC2, &["ip -6 route del 2001:db8:9::/64", "ip -6 route del 2001:db8:9::/64"]), s("RTNETLINK answers: No such process\n"));
    // neighbors: a fixed one survives the flush
    t.eq("linux: neigh add", cli(&mut e, PC2, &["ip -6 neigh add 2001:db8:1::77 lladdr 02:00:00:00:00:77 dev eth0", "ip -6 neigh flush all"]), String::new());
    t.eq("linux: neigh after flush", cli(&mut e, PC2, &["ip -6 neigh"]), s("2001:db8:1::77 dev eth0 lladdr 02:00:00:00:00:77 PERMANENT\n"));
    t.check("linux: neigh in the configuration", e.running_config(PC2).contains("ip -6 neigh add 2001:db8:1::77 lladdr 02:00:00:00:00:77 dev eth0\n"));
    // ip addr shows both families; -br puts IPv6 after IPv4
    t.check("linux: ip addr", cli(&mut e, PC2, &["ip addr show eth0"]).contains("    inet6 2001:db8:1:0:4a:ff:fe00:201/64 scope global dynamic \n"));
    t.eq("linux: ip -br addr", cli(&mut e, PC2, &["ip -br addr show eth0"]), s("eth0             UP             2001:db8:1:0:4a:ff:fe00:201/64 fe80::4a:ff:fe00:201/64\n"));
    // ping6 and traceroute6 complaints in the words of iputils and traceroute
    t.eq("linux: ping6 no target", cli(&mut e, PC2, &["ping6"]), s("ping: usage error: Destination address required\n"));
    t.eq("linux: ping6 name", cli(&mut e, PC2, &["ping6 -c 1 server"]), s("ping: server: Address family for hostname not supported\n"));
    t.eq("linux: ping6 link-local", cli(&mut e, PC2, &["ping6 -c 1 fe80::1"]), s("ping: connect: Invalid argument\n"));
    let o = cli_for(&mut e, PC2, "ping -6 -c 1 fe80::4a:ff:fe00:101%eth0", 2 * SEC);
    t.check("linux: ping with zone", o.starts_with("PING fe80::4a:ff:fe00:101%eth0(fe80::4a:ff:fe00:101%eth0) 56 data bytes\n64 bytes from fe80::4a:ff:fe00:101: icmp_seq=1 ttl=64 time="));
    let o = cli_for(&mut e, PC2, "traceroute -6 -n -q 1 2001:db8:12::2", 5 * SEC);
    t.check("linux: traceroute -6", o.starts_with("traceroute to 2001:db8:12::2 (2001:db8:12::2), 30 hops max, 80 byte packets\n 1  2001:db8:1::1  ") && o.contains("\n 2  2001:db8:12::2  "));
    // read back: the same configuration and the same addresses
    let run = e.running_config(PC2);
    let mut p = two_lans(11);
    if let Some(d) = p.device_mut(PC2) {
        d.startup_config = run.clone();
    }
    let mut e2 = engine(&p);
    t.eq("linux: read back: boot errors", e2.boot_errors(PC2).len(), 0);
    t.eq("linux: read back", e2.running_config(PC2), run);
    run_to(&mut e2, 20 * SEC);
    t.eq("linux: read back: address", state_of(&e2, PC2, ifn(&e2, PC2, "eth0"), pc2g), Some(AddrState::Preferred));
    // off again: the interface loses its addresses (the kernel's disable_ipv6)
    let o = cli(&mut e, PC2, &["sysctl -w net.ipv6.conf.eth0.disable_ipv6=1"]);
    t.eq("linux: off", o, s("net.ipv6.conf.eth0.disable_ipv6 = 1\n"));
    t.eq("linux: no addresses", (e.addrs6(PC2, ifn(&e, PC2, "eth0")).len(), cli(&mut e, PC2, &["ip -6 addr show dev eth0"])), (0, String::new()));
    t.eq("linux: refused", cli(&mut e, PC2, &["ip -6 addr add 2001:db8:1::99/64 dev eth0"]), s("RTNETLINK answers: Permission denied\n"));
}

// ---------------------------------------------------------------- power, routing off

fn power(t: &mut T) {
    let mut e = settled(11);
    let eth = ifn(&e, PC1, "eth0");
    let pc1g = slaac_of("2001:db8:1::", PC1);
    // off and on: what was learned is gone, the link-local address goes through DAD again and a
    // new solicitation brings the address back
    let _ = e.apply(Action::Power { dev: PC1, on: false });
    run_for(&mut e, SEC);
    t.eq("power: off", e.ip6_neighbors(PC1).len(), 0);
    let t0 = e.now();
    let _ = e.apply(Action::Power { dev: PC1, on: true });
    run_to(&mut e, t0);
    t.eq("power: only the link-local address", e.addrs6(PC1, eth).iter().map(|a| (a.addr, a.state)).collect::<Vec<_>>(), vec![(ll_of(PC1, 0), AddrState::Tentative)]);
    run_to(&mut e, t0 + 15 * SEC);
    t.eq("power: back", state_of(&e, PC1, eth, pc1g), Some(AddrState::Preferred));
    // routing off: one last advertisement with lifetime 0 on each interface; the hosts drop the
    // default router at once
    let t1 = e.now();
    let o = cli(&mut e, R1, &["enable", "configure terminal", "no ipv6 unicast-routing", "end"]);
    t.check("power: routing off", !refused(&o));
    run_for(&mut e, SEC);
    let (g0, g1) = (ifn(&e, R1, "GigabitEthernet0/0/0"), ifn(&e, R1, "GigabitEthernet0/0/1"));
    t.eq(
        "power: last advertisements",
        notes6(&e, R1).into_iter().filter(|x| x.0 >= t1 && matches!(x.1, Note6::RaSent { .. })).collect::<Vec<_>>(),
        vec![(t1, Note6::RaSent { iface: g0, to: Ipv6::ALL_NODES, prefixes: 1, lifetime: 0, solicited: false }), (t1, Note6::RaSent { iface: g1, to: Ipv6::ALL_NODES, prefixes: 1, lifetime: 0, solicited: false })],
    );
    t.eq("power: router gone", when6(&e, PC1, |n| matches!(n, Note6::RouterGone { .. }) ), Some(t1 + 2 * RA_T));
    t.check("power: no default", !cli(&mut e, PC1, &["ip -6 route"]).contains("default"));
    t.eq("power: no more advertisements", count6(&e, R1, t1 + 1, u64::MAX, |n| matches!(n, Note6::RaSent { .. })), 0);
}

// ---------------------------------------------------------------- without IPv6

// a project without IPv6 sends no IPv6 frame, notes nothing about it and shows nothing of it
fn ipv4_only(t: &mut T) {
    let mut p = Project::new("v4", 23);
    for (i, m) in ["pc", "pc", "jf-s24p", "jf-r4", "server-1u"].iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 100, 0);
    }
    wire(&mut p, 1, "eth0", 3, "gi1/0/1");
    wire(&mut p, 2, "eth0", 3, "gi1/0/2");
    wire(&mut p, 3, "gi1/0/24", 4, "gi0/0/0");
    wire(&mut p, 4, "gi0/0/1", 5, "eth0");
    let host = |n: &str, a: &str, gw: &str| format!("hostname {}\nip addr add {} dev eth0\nip route add default via {}\n", n, a, gw);
    set(&mut p, 1, "PC1", host("PC1", "192.168.1.10/24", "192.168.1.1"));
    set(&mut p, 2, "PC2", host("PC2", "192.168.1.11/24", "192.168.1.1"));
    set(&mut p, 3, "SW1", s("hostname SW1\n!\nend\n"));
    set(&mut p, 4, "R1", s("hostname R1\n!\ninterface GigabitEthernet0/0/0\n ip address 192.168.1.1 255.255.255.0\n no shutdown\n!\ninterface GigabitEthernet0/0/1\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n!\nend\n"));
    set(&mut p, 5, "SRV", host("SRV", "10.0.0.10/24", "10.0.0.1"));
    let mut e = engine(&p);
    let _ = e.ping(1, netpkt::Ipv4::new(10, 0, 0, 10), jfn_sim::PingOpts { count: 3, ..jfn_sim::PingOpts::default() });
    let _ = e.traceroute(2, netpkt::Ipv4::new(10, 0, 0, 10), TraceOpts::default());
    run_to(&mut e, 60 * SEC);
    t.check("v4: frames sent", e.trace.hops().count() > 20);
    t.eq("v4: no ipv6 frame", e.trace.hops().filter(|h| if h.hdlc { h.bytes.get(2..4) == Some(&[0x86, 0xDD][..]) } else { h.bytes.get(12..14) == Some(&[0x86, 0xDD][..]) }).count(), 0);
    t.eq("v4: no ipv6 note", e.trace.events().filter(|x| matches!(x.note, Note::V6(_))).count(), 0);
    t.check("v4: nothing configured", (1..=5).all(|d| e.ip6(d).is_none_or(|v| !v.used() && v.ifs.iter().all(|f| !f.enabled && f.addrs.is_empty()))));
    t.check("v4: views empty", (1..=5).all(|d| e.ifaces(d).iter().all(|f| f.addrs6.is_empty())));
    t.check("v4: configurations", !e.running_config(4).contains("ipv6") && !e.running_config(1).contains("ipv6"));
    t.check("v4: ip addr", !cli(&mut e, 1, &["ip addr"]).contains("inet6"));
    t.eq("v4: ip -6", cli(&mut e, 1, &["ip -6 addr", "ip -6 route", "ip -6 neigh"]), String::new());
    t.eq("v4: disabled", cli(&mut e, 1, &["sysctl net.ipv6.conf.all.disable_ipv6"]), s("net.ipv6.conf.all.disable_ipv6 = 1\n"));
    t.eq("v4: ios", cli(&mut e, 4, &["show ipv6 interface", "show ipv6 neighbors"]), s("IPv6 Address                              Age Link-layer Addr State Interface\n"));
    t.eq("v4: no problem", e.problems().iter().filter(|x| x.kind == ProblemKind::DuplicateIpv6).count(), 0);
}

// ---------------------------------------------------------------- texts

fn texts(t: &mut T) {
    let rows: Vec<(&str, &str)> = include_str!("locales/engine.tsv").lines().skip(1).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect();
    let en = |k: &'static str| rows.iter().find(|r| r.0 == k).map_or(k, |r| r.1);
    let port = |p: u16| format!("Gi0/0/{}", p);
    let iface = |i: u16| if i == 0 { s("GigabitEthernet0/0/0") } else { s("eth0") };
    let (ll, g) = (a6("fe80::4a:ff:fe00:101"), a6("2001:db8:1::4a:ff:fe00:101"));
    let m = Mac::new(0x02, 0x4a, 0, 0, 1, 1);
    let cases: Vec<(Note6, &str, &str)> = vec![
        (Note6::Up { iface: 0, ll: Some(ll) }, "IPv6 läuft an GigabitEthernet0/0/0: Link-Local-Adresse fe80::4a:ff:fe00:101.", "IPv6 runs on GigabitEthernet0/0/0: link-local address fe80::4a:ff:fe00:101."),
        (Note6::DadStart { iface: 1, addr: g, delay: 412 * MS }, "Adressprüfung (DAD) für 2001:db8:1:0:4a:ff:fe00:101 an eth0 beginnt in 412 ms.", "Duplicate address detection (DAD) for 2001:db8:1:0:4a:ff:fe00:101 on eth0 starts in 412 ms."),
        (Note6::DadDup { iface: 1, addr: g, mac: Some(m) }, "DAD: 2001:db8:1:0:4a:ff:fe00:101 ist an eth0 doppelt, 02:4a:00:00:01:01 hat sie schon: die Adresse bleibt unbenutzt.", "DAD: 2001:db8:1:0:4a:ff:fe00:101 is a duplicate on eth0, 02:4a:00:00:01:01 has it already: the address stays unused."),
        (Note6::AddrNew { iface: 1, addr: Prefix6 { addr: g, len: 64 }, origin: Origin6::Slaac }, "SLAAC: 2001:db8:1:0:4a:ff:fe00:101/64 an eth0 gebildet (Präfix des Routers und EUI-64).", "SLAAC: 2001:db8:1:0:4a:ff:fe00:101/64 formed on eth0 (the router's prefix and EUI-64)."),
        (Note6::AddrDeprecated { iface: 1, addr: g }, "2001:db8:1:0:4a:ff:fe00:101 an eth0 ist veraltet (Preferred Lifetime abgelaufen): nicht mehr für neue Verbindungen.", "2001:db8:1:0:4a:ff:fe00:101 on eth0 is deprecated (preferred lifetime over): no longer for new connections."),
        (Note6::Neigh { iface: 1, addr: g, from: Some(NState::Delay), to: NState::Probe, mac: Some(m) }, "Neighbor Cache: 2001:db8:1:0:4a:ff:fe00:101 an eth0 wechselt von DELAY nach PROBE.", "Neighbor cache: 2001:db8:1:0:4a:ff:fe00:101 on eth0 goes from DELAY to PROBE."),
        (Note6::NeighFailed { addr: g }, "2001:db8:1:0:4a:ff:fe00:101 antwortet nicht auf Neighbor Solicitations: Eintrag gelöscht, wartende Pakete verworfen.", "2001:db8:1:0:4a:ff:fe00:101 does not answer neighbor solicitations: entry deleted, waiting packets dropped."),
        (Note6::NdBad { why: netpkt::icmpv6::NdErr::HopLimit }, "Neighbor-Discovery-Nachricht verworfen: das Hop Limit ist nicht 255, sie kam nicht von diesem Link.", "Neighbor Discovery message dropped: the hop limit is not 255, it did not come from this link."),
        (Note6::RaSent { iface: 0, to: Ipv6::ALL_NODES, prefixes: 1, lifetime: 1800, solicited: true }, "Router Advertisement über GigabitEthernet0/0/0 an ff02::1 (auf Anfrage): Lebensdauer 1800 s, Präfixe: 1.", "Router advertisement via GigabitEthernet0/0/0 to ff02::1 (on request): lifetime 1800 s, prefixes: 1."),
        (Note6::RaIgnored { iface: 0, why: jfn_sim::RaWhy::Router }, "Router Advertisement an GigabitEthernet0/0/0 ignoriert: dieses Gerät routet selbst.", "Router advertisement on GigabitEthernet0/0/0 ignored: this device routes itself."),
        (Note6::TooBig { mtu: 1280 }, "Paket größer als die MTU 1280: Router fragmentieren IPv6 nicht, verworfen.", "Packet larger than the MTU 1280: routers do not fragment IPv6, dropped."),
        (Note6::LinkScope { src: ll, dst: g }, "fe80::4a:ff:fe00:101 an 2001:db8:1:0:4a:ff:fe00:101 bleibt auf seinem Link: Link-Local-Adressen werden nicht weitergeleitet.", "fe80::4a:ff:fe00:101 to 2001:db8:1:0:4a:ff:fe00:101 stays on its link: link-local addresses are not forwarded."),
        (Note6::IcmpSent { typ: 1, code: 3, to: g }, "ICMPv6 an 2001:db8:1:0:4a:ff:fe00:101: Destination Unreachable (address unreachable).", "ICMPv6 to 2001:db8:1:0:4a:ff:fe00:101: Destination Unreachable (address unreachable)."),
        (Note6::Fragmented { parts: 2, mtu: 1280 }, "In 2 Fragmente mit Fragment Header geteilt (MTU 1280).", "Split into 2 fragments with a fragment header (MTU 1280)."),
    ];
    for (n, de, english) in cases {
        let note = Note::V6(n.clone());
        let msg = n.message(&port, &iface);
        t.eq("text: german", msg.text(), s(de));
        t.eq("text: english", msg.text_with(&en), s(english));
        t.check("text: layer 3", note.layer() == 3);
    }
    t.check("text: drops", Note::V6(Note6::TooBig { mtu: 1280 }).is_drop() && !Note::V6(Note6::DadOk { iface: 0, addr: ll }).is_drop());
    t.eq("text: masks", (Note6::DadProbe { iface: 0, addr: ll }.mask(), Note6::UdpNoPort { port: 33434 }.mask()), (decode::P_IPV6 | decode::P_ICMPV6, decode::P_IPV6 | decode::P_UDP));
    // the problem of a duplicate, in both languages
    let mut e = engine(&two_lans(11));
    run_to(&mut e, 10 * SEC);
    let _ = cli(&mut e, PC2, &["ip -6 addr add 2001:db8:2::10/64 dev eth0"]);
    let _ = cli(&mut e, SRV, &["ip -6 addr add 2001:db8:1::1/64 dev eth0"]);
    let _ = cli(&mut e, R1, &["enable", "configure terminal", "interface GigabitEthernet0/0/0", "ipv6 address 2001:DB8:1::4A:FF:FE00:201/64", "end"]);
    run_for(&mut e, 3 * SEC);
    let probs: Vec<(u32, String, String)> = e.problems().into_iter().filter(|x| x.kind == ProblemKind::DuplicateIpv6).map(|x| (x.dev, x.text.clone(), x.msg.text_with(&en))).collect();
    t.eq("text: problem", probs, vec![(R1, s("2001:db8:1:0:4a:ff:fe00:201 ist an GigabitEthernet0/0/0 doppelt: die Adressprüfung (DAD) fand 02:4a:00:00:02:01 damit."), s("2001:db8:1:0:4a:ff:fe00:201 is a duplicate on GigabitEthernet0/0/0: duplicate address detection (DAD) found 02:4a:00:00:02:01 with it."))]);
}

// ---------------------------------------------------------------- frames, determinism

// every IPv6 frame of a busy run decodes without a problem and its checksums hold
fn frames(t: &mut T) {
    let mut e = settled(11);
    let srv = a6("2001:db8:2::10");
    let _ = e.ping6(PC1, srv, Ping6Opts { count: 3, ..Ping6Opts::default() });
    let _ = e.ping6(PC2, srv, Ping6Opts { count: 2, size: 3000, ..Ping6Opts::default() });
    let _ = e.traceroute6(PC1, srv, TraceOpts::default(), None);
    let _ = e.ping6(PC1, a6("2001:db8:99::1"), Ping6Opts { count: 1, ..Ping6Opts::default() });
    run_for(&mut e, 20 * SEC);
    let all: Vec<Vec<u8>> = e.trace.hops().filter(|h| is6(&h.bytes)).map(|h| h.bytes.to_vec()).collect();
    t.check("frames: many", all.len() > 150);
    t.eq("frames: all whole", all.iter().filter(|b| !frame_ok(b)).count(), 0);
    // every kind of message was there: RS, RA, NS, NA, echo both ways, errors, UDP, fragments
    let kinds: Vec<u8> = [133u8, 134, 135, 136, 128, 129, 1, 3].into_iter().filter(|k| all.iter().any(|b| icmp_type(b) == *k)).collect();
    t.eq("frames: kinds", kinds, vec![133, 134, 135, 136, 128, 129, 1, 3]);
    t.check("frames: udp and fragments", all.iter().any(|b| b[20] == 17) && all.iter().any(|b| b[20] == 44));
    // the time exceeded quotes the probe: 1232 bytes at most (RFC 4443 2.4 c)
    t.check("frames: quotes fit 1280", all.iter().filter(|b| matches!(icmp_type(b), 1..=4)).all(|b| b.len() - 14 <= 1280));
    t.eq("frames: errors are errors", Icmp6Kind::of(3, 0, 0), Icmp6Kind::HopLimit);
}

// the same project twice: the same frames at the same times; another seed, other delays
fn determinism(t: &mut T) {
    let run = |seed: u64| {
        let mut e = settled(seed);
        let _ = e.ping6(PC1, a6("2001:db8:2::10"), Ping6Opts { count: 2, ..Ping6Opts::default() });
        run_for(&mut e, 5 * SEC);
        e.trace.hops().map(|h| (h.t_tx, h.bytes.to_vec())).collect::<Vec<_>>()
    };
    let (a, b, c) = (run(11), run(11), run(12));
    t.check("determinism: same run", a == b && a.len() > 50);
    t.check("determinism: another seed", a != c);
}
