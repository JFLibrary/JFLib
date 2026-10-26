// ipv6tcptest -- checks of UDP and TCP over IPv6 and what uses them (group ipv6tcp of
// --selftest: jfn_sim::os, tcp, resolver, the services and the consoles' cli/tools.rs,
// cli/names.rs): the handshake byte for byte (port and ISN from the device's random numbers,
// the MSS of the IPv6 path, Linux's window, the pseudo-header checksum), data and the close,
// RST at a closed port, the SYN retransmissions, Packet Too Big lowering the MSS, ICMPv6 errors
// reaching a connecting TCP socket and connected UDP sockets (IPv4 too), the dual-stack sockets
// (:: takes both families, 0.0.0.0 IPv4 only, an address only itself; what may share a port;
// ss), the resolver asking for A and AAAA with the order of RFC 6724, the services and tools
// over IPv6 (curl, nc, the TCP client, ss, ping and traceroute by name, dig, host, nslookup,
// scripts), the IOS-like console (name servers and host names of either family, ping by name,
// show tcp brief), notes, masks and the capture file, determinism
//
// Two networks (device ids in order).  LAN: PC1 (10.0.0.1, 2001:db8:1::1) and PC2 (10.0.0.2,
// 2001:db8:1::2) on SW1, with PC4 (10.0.0.4, IPv6 on with its link-local address only); the
// hosts send no router solicitations (accept_ra 0) and know their neighbours by hand, so
// nothing but the checked traffic is on the cables after the first two seconds of duplicate
// address detection.  WAN: PC1 (2001:db8:1::1, 10.0.1.1) - R1 - PC3 (2001:db8:3::3, 10.0.3.3)
// with an IPv6 MTU of 1280 on R1's side to PC3, no router advertisements, the hosts' IPv6
// default routes through R1's link-local addresses and static neighbours everywhere.  1 Gbit/s
// copper with 50 ns of cable: a frame of n bytes takes (max(n, 60) + 24) * 8 + 50 ns per cable,
// so over the switch (or R1) twice that: a SYN with its MSS option (78 bytes over IPv6) 866 ns,
// a segment without data (74 bytes) 834 ns, 100 bytes of data 1634 ns, 1440 bytes 12354 ns, a
// Packet Too Big with its 1232 bytes of quote 10594 ns.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use base::rng::Pcg32;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::node::port_mac;
use jfn_sim::{Action, Engine, Hop, Metric, Note, OsApi, PcapOpts, PcapScope, Report, Service, SockOpts, SvcEvent, TcpEnd, TcpState, ANY, ANY4, MS, SEC};
use netpkt::decode::{self, P_IPV4, P_IPV6, P_TCP};
use netpkt::eth::{EthHdr, ET_IPV6};
use netpkt::ipv6::{Ipv6Hdr, Ipv6View, NH_TCP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::tcp::{TcpHdr, TcpOpts, TcpView, ACK, RST, SYN};
use netpkt::{pcapng, IpAddr, Ipv4, Ipv6, Mac};

use crate::selftest::T;

const BUDGET: usize = 8_000_000;
const SEED: u64 = 5;
/// when the checks start: duplicate address detection is over
const T0: u64 = 10 * SEC;

const PC1: u32 = 1;
const PC2: u32 = 2;
const SW1: u32 = 3;
const PC4: u32 = 4;
// the WAN
const R1: u32 = 2;
const PC3: u32 = 3;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn s(x: &str) -> String {
    String::from(x)
}

fn ip(x: &str) -> IpAddr {
    IpAddr::parse(x).unwrap_or(IpAddr::V4(Ipv4::UNSPECIFIED))
}

fn a6(x: &str) -> Ipv6 {
    Ipv6::parse(x).unwrap_or(Ipv6::UNSPECIFIED)
}

fn mac(dev: u32) -> Mac {
    port_mac(dev, 0)
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: String::from(kind), name: String::from(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Value::Null) }
}

fn set(p: &mut Project, id: u32, name: &str, text: String) {
    if let Some(d) = p.device_mut(id) {
        d.name = String::from(name);
        d.startup_config = text;
    }
}

fn wire(p: &mut Project, a: u32, ap: &str, b: u32, bp: &str) {
    let _ = p.connect(&pr(a, ap), &pr(b, bp), None);
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

/// a Linux host with IPv6 on and no router discovery: its addresses, default routes (the IPv6
/// one through the router's link-local address: a global one is not on-link before the host's
/// address passed duplicate address detection), the neighbours set by hand
fn host(name: &str, v4: &str, v6: Option<&str>, gw: Option<(&str, &str)>, neigh: &[(&str, Mac)]) -> String {
    let mut c = format!("hostname {}\nsysctl -w net.ipv6.conf.all.disable_ipv6=0\nsysctl -w net.ipv6.conf.eth0.accept_ra=0\nip addr add {} dev eth0\n", name, v4);
    if let Some(a) = v6 {
        c.push_str(&format!("ip -6 addr add {} dev eth0\n", a));
    }
    if let Some((g4, g6)) = gw {
        c.push_str(&format!("ip route add default via {}\nip -6 route add default via {} dev eth0\n", g4, g6));
    }
    for (a, m) in neigh {
        let fam = if a.contains(':') { "-6 " } else { "" };
        c.push_str(&format!("ip {}neigh add {} lladdr {} dev eth0\n", fam, a, m));
    }
    c
}

/// the LAN of the header comment
fn lan6(seed: u64) -> Engine {
    let mut p = Project::new("lan6", seed);
    for (i, m) in ["pc", "pc", "jf-s24p", "pc"].iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 100, 0);
    }
    wire(&mut p, PC1, "eth0", SW1, "gi1/0/1");
    wire(&mut p, PC2, "eth0", SW1, "gi1/0/2");
    wire(&mut p, PC4, "eth0", SW1, "gi1/0/4");
    set(&mut p, PC1, "PC1", host("PC1", "10.0.0.1/24", Some("2001:db8:1::1/64"), None, &[("10.0.0.2", mac(PC2)), ("2001:db8:1::2", mac(PC2)), ("10.0.0.4", mac(PC4))]));
    set(&mut p, PC2, "PC2", host("PC2", "10.0.0.2/24", Some("2001:db8:1::2/64"), None, &[("10.0.0.1", mac(PC1)), ("2001:db8:1::1", mac(PC1)), ("10.0.0.4", mac(PC4))]));
    set(&mut p, SW1, "SW1", s("hostname SW1\n!\nend\n"));
    set(&mut p, PC4, "PC4", host("PC4", "10.0.0.4/24", None, None, &[("10.0.0.1", mac(PC1)), ("10.0.0.2", mac(PC2))]));
    engine(&p)
}

const R1_CFG: &str = "hostname R1\n!\nipv6 unicast-routing\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.1.254 255.255.255.0\n ipv6 address 2001:DB8:1::FE/64\n ipv6 nd ra suppress all\n no shutdown\n!\ninterface GigabitEthernet0/0/1\n ip address 10.0.3.254 255.255.255.0\n ipv6 address 2001:DB8:3::FE/64\n ipv6 mtu 1280\n ipv6 nd ra suppress all\n no shutdown\n!\narp 10.0.1.1 024a.0000.0101 arpa\narp 10.0.3.3 024a.0000.0301 arpa\nipv6 neighbor 2001:DB8:1::1 GigabitEthernet0/0/0 024a.0000.0101\nipv6 neighbor 2001:DB8:3::3 GigabitEthernet0/0/1 024a.0000.0301\n!\nend\n";

/// the WAN of the header comment
fn wan6(seed: u64) -> Engine {
    let mut p = Project::new("wan6", seed);
    for (i, m) in ["pc", "jf-r4", "server-1u"].iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 200, 0);
    }
    wire(&mut p, PC1, "eth0", R1, "gi0/0/0");
    wire(&mut p, R1, "gi0/0/1", PC3, "eth0");
    let (r0, r1) = (port_mac(R1, 0), port_mac(R1, 1));
    set(&mut p, PC1, "PC1", host("PC1", "10.0.1.1/24", Some("2001:db8:1::1/64"), Some(("10.0.1.254", "fe80::4a:ff:fe00:201")), &[("10.0.1.254", r0), ("fe80::4a:ff:fe00:201", r0)]));
    set(&mut p, R1, "R1", s(R1_CFG));
    set(&mut p, PC3, "PC3", host("PC3", "10.0.3.3/24", Some("2001:db8:3::3/64"), Some(("10.0.3.254", "fe80::4a:ff:fe00:202")), &[("10.0.3.254", r1), ("fe80::4a:ff:fe00:202", r1)]));
    engine(&p)
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

/// a line typed at a device, then `d` of virtual time: what it printed
fn cli_for(e: &mut Engine, dev: u32, line: &str, d: u64) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    let t = e.now() + d;
    run_to(e, t);
    out(e, dev)
}

/// lines typed at a device now: what it printed
fn cli(e: &mut Engine, dev: u32, lines: &[&str]) -> String {
    let _ = e.take_reports();
    for l in lines {
        let _ = e.cli_line(dev, l);
    }
    let now = e.now();
    run_to(e, now);
    out(e, dev)
}

/// the words in column `n` of a table's rows after its head, sorted
fn column(table: &str, n: usize) -> Vec<String> {
    let mut v: Vec<String> = table.lines().skip(1).filter_map(|l| l.split_whitespace().nth(n)).map(String::from).collect();
    v.sort();
    v
}

/// the line of a text that starts with `head`
fn line_of(text: &str, head: &str) -> Option<String> {
    text.lines().find(|l| l.starts_with(head)).map(String::from)
}

/// the port, ISN of an active open over IPv6 (no IP identification is drawn)
fn linux_connect(seed: u64, dev: u32) -> (u16, u32) {
    let mut g = Pcg32::derive(seed, dev as u64);
    let port = 32768 + ((g.next_u32() % 28232) & !1) as u16;
    (port, g.next_u32())
}

/// the ISN of a passive open
fn linux_accept(seed: u64, dev: u32) -> u32 {
    Pcg32::derive(seed, dev as u64).next_u32()
}

// ---------------------------------------------------------------- a scripted service

/// what a Probe does at a time
#[derive(Clone, Debug)]
enum Step {
    /// listen on :: (both families)
    Listen(u16),
    ListenOn(IpAddr, u16),
    Connect { dst: IpAddr, dport: u16 },
    Send(usize),
    Shutdown,
    Close,
    /// a UDP socket on an address and port, connected to a peer or not
    Udp { addr: IpAddr, port: u16, peer: Option<(IpAddr, u16)> },
    /// a datagram of n bytes from the UDP socket
    UdpSend { dst: IpAddr, dport: u16, n: usize },
}

/// a service that does what a test says at given times and keeps what it hears
#[derive(Clone, Debug, Default)]
struct Probe {
    plan: Vec<(u64, Step)>,
    listener: Option<u32>,
    sock: Option<u32>,
    udp: Option<u16>,
    sent: usize,
    pending: usize,
    got: Vec<u8>,
    /// what listen and bind said (Err: the port is taken)
    binds: Vec<bool>,
    log: Vec<(u64, SvcEvent)>,
}

impl Probe {
    fn new(plan: Vec<(u64, Step)>) -> Probe {
        Probe { plan, ..Probe::default() }
    }
    fn arm(&mut self, os: &mut OsApi) {
        if let Some(t) = self.plan.iter().map(|p| p.0).min() {
            let at = t.max(os.now());
            os.timer(0, at);
        }
    }
    fn run(&mut self, os: &mut OsApi) {
        let now = os.now();
        let (due, rest): (Vec<(u64, Step)>, Vec<(u64, Step)>) = self.plan.drain(..).partition(|p| p.0 <= now);
        self.plan = rest;
        for (_, st) in due {
            match st {
                Step::Listen(port) => {
                    let r = os.tcp_listen(port, 0, SockOpts::default());
                    self.binds.push(r.is_ok());
                    if let Ok(l) = r {
                        self.listener = Some(l);
                    }
                }
                Step::ListenOn(addr, port) => {
                    let r = os.tcp_listen_on(addr, port, 0, SockOpts::default());
                    self.binds.push(r.is_ok());
                    if let Ok(l) = r {
                        self.listener = Some(l);
                    }
                }
                Step::Connect { dst, dport } => self.sock = os.tcp_connect(dst, dport, SockOpts::default()).ok(),
                Step::Send(n) => {
                    self.pending += n;
                    self.write(os);
                }
                Step::Shutdown => {
                    if let Some(s) = self.sock {
                        os.tcp_shutdown(s);
                    }
                }
                Step::Close => {
                    if let Some(s) = self.sock.take() {
                        os.tcp_close(s);
                    }
                }
                Step::Udp { addr, port, peer } => {
                    let r = os.bind_on(addr, port, None);
                    self.binds.push(r.is_ok());
                    if let Ok(p) = r {
                        self.udp = Some(p);
                        if peer.is_some() {
                            os.udp_connect(p, peer);
                        }
                    }
                }
                Step::UdpSend { dst, dport, n } => {
                    if let Some(p) = self.udp {
                        let data: Vec<u8> = (0..n).map(|i| i as u8).collect();
                        os.send(p, dst, dport, &data);
                    }
                }
            }
        }
        self.arm(os);
    }
    fn write(&mut self, os: &mut OsApi) {
        if let Some(s) = self.sock
            && self.pending > 0
        {
            let data: Vec<u8> = (0..self.pending).map(|i| ((self.sent + i) % 256) as u8).collect();
            let n = os.tcp_send(s, &data);
            self.sent += n;
            self.pending -= n;
        }
    }
    /// when it heard an event of this kind first
    fn first(&self, f: impl Fn(&SvcEvent) -> bool) -> Option<u64> {
        self.log.iter().find(|(_, e)| f(e)).map(|(t, _)| *t)
    }
    /// the senders of the datagrams it got
    fn senders(&self) -> Vec<IpAddr> {
        self.log
            .iter()
            .filter_map(|(_, e)| match e {
                SvcEvent::Udp(d) => Some(d.src),
                _ => None,
            })
            .collect()
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
        self.log.push((os.now(), ev.clone()));
        match ev {
            SvcEvent::Timer(0) => self.run(os),
            SvcEvent::TcpAccept { listener, .. } => {
                while let Some(s) = os.tcp_accept(listener) {
                    self.sock = Some(s);
                }
            }
            SvcEvent::TcpData { sock, .. } => {
                let d = os.tcp_recv(sock, usize::MAX);
                self.got.extend_from_slice(&d);
            }
            SvcEvent::TcpWritable { .. } => self.write(os),
            SvcEvent::TcpClosed { sock, .. } => {
                let d = os.tcp_recv(sock, usize::MAX);
                self.got.extend_from_slice(&d);
                os.tcp_close(sock);
                if Some(sock) == self.sock {
                    self.sock = None;
                }
            }
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        vec![Metric::new("got", self.got.len() as i64)]
    }
    fn status(&self) -> String {
        format!("{} Bytes empfangen", self.got.len())
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

fn probe<'a>(e: &'a Engine, dev: u32, name: &str) -> Option<&'a Probe> {
    e.service::<Probe>(dev, name)
}

fn add_probe(e: &mut Engine, dev: u32, name: &str, plan: Vec<(u64, Step)>) -> bool {
    e.add_service(dev, name, Box::new(Probe::new(plan)), true).is_ok()
}

// ---------------------------------------------------------------- segments on the cables

/// a TCP segment over IPv6 on a cable
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seg {
    t: u64,
    from: u32,
    sport: u16,
    dport: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    wnd: u16,
    len: usize,
    mss: Option<u16>,
    /// the checksum over the IPv6 pseudo-header holds
    sum: bool,
}

fn seg_of(h: &Hop) -> Option<Seg> {
    let v = Ipv6View::parse(h.bytes.get(14..)?)?;
    if v.next_header() != NH_TCP {
        return None;
    }
    let s = TcpView::parse(v.payload())?;
    Some(Seg { t: h.t_tx, from: h.from.0, sport: s.src_port(), dport: s.dst_port(), seq: s.seq(), ack: s.ack(), flags: s.flags(), wnd: s.window(), len: s.payload().len(), mss: s.opts().mss, sum: s.checksum_ok_v6(v.src(), v.dst()) })
}

/// the TCP segments over IPv6 a device sent
fn sent(e: &Engine, dev: u32) -> Vec<Seg> {
    e.trace.hops().filter(|h| h.from.0 == dev).filter_map(seg_of).collect()
}

/// the times and states a device's connections went to
fn states(e: &Engine, dev: u32) -> Vec<(u64, TcpState)> {
    e.trace.events().filter(|x| x.dev == dev).filter_map(|x| if let Note::TcpStateChange { to, .. } = x.note { Some((x.t, to)) } else { None }).collect()
}

/// a TCP segment over IPv6 in its frame as the stacks build it
fn frame6(from: Mac, to: Mac, src: Ipv6, dst: Ipv6, hops: u8, h: TcpHdr, data: &[u8]) -> Vec<u8> {
    let mut p = Pkt::new(HEADROOM, data);
    h.push_v6(&mut p, src, dst);
    Ipv6Hdr::new(src, dst, NH_TCP, hops).push_onto(&mut p);
    EthHdr { dst: to, src: from, ethertype: ET_IPV6 }.push_onto(&mut p);
    p.into_vec()
}

fn hdr(sport: u16, dport: u16, seq: u32, ack: u32, flags: u8, window: u16, mss: Option<u16>) -> TcpHdr {
    TcpHdr { src_port: sport, dst_port: dport, seq, ack, flags, window, urgent: 0, opts: TcpOpts { mss, ..TcpOpts::default() } }
}

pub fn run(t: &mut T) {
    handshake(t);
    refused(t);
    syn_retries(t);
    too_big(t);
    unreachable(t);
    udp_errors(t);
    dual_stack(t);
    resolver(t);
    services(t);
    tools(t);
    ios(t);
    notes(t);
    determinism(t);
}

// ---------------------------------------------------------------- TCP

// the handshake byte for byte, 100 bytes, the close
fn handshake(t: &mut T) {
    let mut e = lan6(SEED);
    let srv = vec![(0, Step::Listen(7)), (T0 + 3 * MS, Step::Close)];
    let cli = vec![(T0, Step::Connect { dst: ip("2001:db8:1::2"), dport: 7 }), (T0 + MS, Step::Send(100)), (T0 + 2 * MS, Step::Shutdown)];
    t.check("probes", add_probe(&mut e, PC2, "srv", srv) && add_probe(&mut e, PC1, "cli", cli));
    run_to(&mut e, T0 + 500 * 1000);
    let (port, iss) = linux_connect(SEED, PC1);
    let irs = linux_accept(SEED, PC2);
    t.eq("port and isn from the device's numbers", sent(&e, PC1).first().map(|s| (s.sport, s.seq)), Some((port, iss)));
    // the SYN and the SYN-ACK 1732 ns apart over the switch, the ACK 1668 ns
    t.eq("client states", states(&e, PC1), vec![(T0, TcpState::SynSent), (T0 + 3464, TcpState::Established)]);
    t.eq("server states", states(&e, PC2), vec![(0, TcpState::Listen), (T0 + 1732, TcpState::SynReceived), (T0 + 5132, TcpState::Established)]);
    // byte for byte: hop limit 64, the MSS of the 1500-byte MTU less 60, 65535 rounded down to
    // it, the checksum over the IPv6 pseudo-header
    let bytes = |dev: u32| e.trace.hops().find(|h| h.from.0 == dev && seg_of(h).is_some()).map(|h| h.bytes.to_vec());
    let (a, b) = (a6("2001:db8:1::1"), a6("2001:db8:1::2"));
    t.eq("syn bytes", bytes(PC1), Some(frame6(mac(PC1), mac(PC2), a, b, 64, hdr(port, 7, iss, 0, SYN, 64800, Some(1440)), &[])));
    t.eq("syn-ack bytes", bytes(PC2), Some(frame6(mac(PC2), mac(PC1), b, a, 64, hdr(7, port, irs, iss.wrapping_add(1), SYN | ACK, 64800, Some(1440)), &[])));
    let c = e.tcp_conns(PC1);
    t.eq(
        "client row",
        c.first().map(|r| (r.state, r.local, r.remote, r.mss, r.snd_wnd, r.srtt)),
        Some((TcpState::Established, (IpAddr::V6(a), port), (IpAddr::V6(b), 7), 1440, 64800, Some(3464))),
    );
    run_to(&mut e, T0 + 10 * MS);
    // 100 bytes 3268 ns over the switch
    t.eq("data at the server", probe(&e, PC2, "srv").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpData { .. }))), Some(T0 + MS + 3268));
    t.eq("bytes", probe(&e, PC2, "srv").map(|p| p.got.clone()), Some((0..100).map(|i| i as u8).collect::<Vec<u8>>()));
    // the client's FIN, the server's FIN-ACK: TIME-WAIT, and the last ACK closes the server
    let cs = states(&e, PC1);
    t.eq("client ends in time-wait", cs.last().copied(), Some((T0 + 3 * MS + 1668, TcpState::TimeWait)));
    t.eq("server closed", states(&e, PC2).last().copied(), Some((T0 + 3 * MS + 3336, TcpState::Closed)));
    t.check("every checksum holds", sent(&e, PC1).iter().chain(sent(&e, PC2).iter()).all(|s| s.sum));
    t.eq("stats", e.tcp_stats(PC1).map(|s| (s.active_opens, s.out_segs)), Some((1, sent(&e, PC1).len() as u64)));
}

// a SYN to a closed port: RST-ACK at once, ECONNREFUSED
fn refused(t: &mut T) {
    let mut e = lan6(SEED);
    let ok = add_probe(&mut e, PC1, "cli", vec![(T0, Step::Connect { dst: ip("2001:db8:1::2"), dport: 9 })]);
    t.check("refused: probe", ok);
    run_to(&mut e, T0 + MS);
    let rst = sent(&e, PC2);
    t.eq("rst", rst.iter().map(|s| (s.t, s.flags, s.sum)).collect::<Vec<_>>(), vec![(T0 + 1732, RST | ACK, true)]);
    t.eq("refused", probe(&e, PC1, "cli").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpClosed { end: TcpEnd::Refused, .. }))), Some(T0 + 3400));
    t.eq("counted", e.tcp_stats(PC2).map(|s| s.out_rsts), Some(1));
}

// the SYN to a neighbour that never answers: 1 s, then doubled, six times, then ETIMEDOUT
fn syn_retries(t: &mut T) {
    let mut e = lan6(SEED);
    let _ = cli(&mut e, PC1, &["ip -6 neigh add 2001:db8:1::99 lladdr 02:00:00:00:00:99 dev eth0"]);
    let ok = add_probe(&mut e, PC1, "cli", vec![(T0, Step::Connect { dst: ip("2001:db8:1::99"), dport: 80 })]);
    t.check("retries: probe", ok);
    run_to(&mut e, T0 + 200 * SEC);
    let syns: Vec<u64> = sent(&e, PC1).iter().filter(|s| s.flags == SYN).map(|s| s.t - T0).collect();
    t.eq("syn times", syns, vec![0, SEC, 3 * SEC, 7 * SEC, 15 * SEC, 31 * SEC, 63 * SEC]);
    t.eq("given up", probe(&e, PC1, "cli").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpClosed { end: TcpEnd::Timeout, .. }))), Some(T0 + 127 * SEC));
}

// Packet Too Big from R1 (MTU 1280 towards PC3): the MSS drops to 1220, what was lost goes again
fn too_big(t: &mut T) {
    let mut e = wan6(SEED);
    let ok = add_probe(&mut e, PC3, "sink", vec![(0, Step::Listen(9))]) && add_probe(&mut e, PC1, "cli", vec![(T0, Step::Connect { dst: ip("2001:db8:3::3"), dport: 9 }), (T0 + MS, Step::Send(5000))]);
    t.check("too big: probes", ok);
    run_to(&mut e, T0 + MS / 2);
    t.eq("mss of the path before", e.tcp_conns(PC1).first().map(|r| r.mss), Some(1440));
    run_to(&mut e, T0 + 100 * MS);
    // the first 1440-byte segment reaches R1 after 12354 ns, its error is back 10594 ns later
    let ptb: Vec<(u64, Note)> = e.trace.events().filter(|x| x.dev == PC1 && matches!(x.note, Note::TcpPtb { .. })).map(|x| (x.t, x.note.clone())).collect();
    t.eq("packet too big", ptb.first().cloned(), Some((T0 + MS + 22948, Note::TcpPtb { mss: 1220 })));
    t.eq("mss after", e.tcp_conns(PC1).first().map(|r| r.mss), Some(1220));
    t.eq("all bytes arrived", probe(&e, PC3, "sink").map(|p| p.got.len()), Some(5000));
    let to_pc3: Vec<usize> = e.trace.hops().filter(|h| h.to.0 == PC3).filter_map(seg_of).map(|s| s.len).filter(|&n| n > 0).collect();
    t.check("no segment over 1220 bytes behind R1", !to_pc3.is_empty() && to_pc3.iter().all(|&n| n <= 1220));
    let ss = cli(&mut e, PC1, &["ss -ti"]);
    t.check("ss: mss and pmtu", ss.contains(" mss:1220 ") && ss.contains(" pmtu:1280 "));
}

// an ICMPv6 error ends a connection attempt: no route behind R1
fn unreachable(t: &mut T) {
    let mut e = wan6(SEED);
    let ok = add_probe(&mut e, PC1, "cli", vec![(T0, Step::Connect { dst: ip("2001:db8:99::1"), dport: 80 })]);
    t.check("unreachable: probe", ok);
    run_to(&mut e, T0 + MS);
    // the SYN to R1 866 ns, the error with its quote (126 bytes) 1250 ns back
    t.eq("unreachable", probe(&e, PC1, "cli").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpClosed { end: TcpEnd::Unreachable, .. }))), Some(T0 + 2116));
    let nc = cli_for(&mut e, PC1, "nc -6 -z -v 2001:db8:99::1 80", MS);
    t.check("nc says so", nc.contains("No route to host"));
}

// port unreachable reaches a connected UDP socket (IPv6 and IPv4), never an unconnected one
fn udp_errors(t: &mut T) {
    let mut e = lan6(SEED);
    let plan = |dst: &str, at: u64| vec![(at, Step::Udp { addr: if dst.contains(':') { ANY } else { ANY4 }, port: 0, peer: Some((ip(dst), 9999)) }), (at, Step::UdpSend { dst: ip(dst), dport: 9999, n: 10 })];
    let ok = add_probe(&mut e, PC1, "c6", plan("2001:db8:1::2", T0))
        && add_probe(&mut e, PC4, "c4", plan("10.0.0.2", T0 + MS))
        && add_probe(&mut e, PC1, "free", vec![(T0 + 2 * MS, Step::Udp { addr: ANY, port: 0, peer: None }), (T0 + 2 * MS, Step::UdpSend { dst: ip("2001:db8:1::2"), dport: 9999, n: 10 })]);
    t.check("udp: probes", ok);
    run_to(&mut e, T0 + 10 * MS);
    let err = |name: &str, dev: u32| probe(&e, dev, name).and_then(|p| p.first(|x| matches!(x, SvcEvent::UdpError { err: TcpEnd::Refused, .. })));
    // 72 bytes there (818 ns per cable), the error with the datagram quoted (120 bytes) back
    t.eq("ipv6: refused", err("c6", PC1), Some(T0 + 1636 + 2404));
    // 60 bytes (722 ns), the ICMP error with 38 bytes of quote (80 bytes, 882 ns)
    t.eq("ipv4: refused", err("c4", PC4), Some(T0 + MS + 1444 + 1764));
    t.eq("unconnected: nothing", probe(&e, PC1, "free").map(|p| p.log.iter().any(|(_, x)| matches!(x, SvcEvent::UdpError { .. }))), Some(false));
}

// :: takes both families (a peer over IPv4 stays IPv4), 0.0.0.0 IPv4 only, an address only
// itself; which bindings may share a port; ss
fn dual_stack(t: &mut T) {
    let mut e = lan6(SEED);
    let a2 = ip("2001:db8:1::2");
    // (an address can be bound once it passed duplicate address detection)
    let up = 5 * SEC;
    let srv = vec![
        (up, Step::Listen(7)),
        (up, Step::ListenOn(ANY4, 8)),
        (up, Step::ListenOn(a2, 9)),
        (up, Step::Udp { addr: ANY, port: 5000, peer: None }),
        (up, Step::ListenOn(a2, 10)),
        // taken: the IPv4 part of ::, an address under ::, 0.0.0.0 under 0.0.0.0, :: over an address
        (up + MS, Step::ListenOn(ANY4, 7)),
        (up + MS, Step::ListenOn(a2, 7)),
        (up + MS, Step::ListenOn(ip("10.0.0.2"), 8)),
        (up + MS, Step::ListenOn(ANY, 9)),
        // free: an address of the other family beside an address
        (up + MS, Step::ListenOn(ip("10.0.0.2"), 10)),
    ];
    let srv2 = vec![(up, Step::Udp { addr: ANY4, port: 5001, peer: None })];
    let c = |dst: &str, port: u16, at: u64| vec![(at, Step::Connect { dst: ip(dst), dport: port })];
    let ok = add_probe(&mut e, PC2, "srv", srv)
        && add_probe(&mut e, PC2, "srv2", srv2)
        && add_probe(&mut e, PC1, "v4to7", c("10.0.0.2", 7, T0))
        && add_probe(&mut e, PC1, "v6to7", c("2001:db8:1::2", 7, T0 + MS))
        && add_probe(&mut e, PC1, "v4to8", c("10.0.0.2", 8, T0 + 2 * MS))
        && add_probe(&mut e, PC1, "v6to8", c("2001:db8:1::2", 8, T0 + 3 * MS))
        && add_probe(&mut e, PC1, "v4to9", c("10.0.0.2", 9, T0 + 4 * MS))
        && add_probe(&mut e, PC1, "v6to9", c("2001:db8:1::2", 9, T0 + 5 * MS))
        && add_probe(&mut e, PC1, "u", vec![(T0, Step::Udp { addr: ANY, port: 0, peer: None }), (T0 + 6 * MS, Step::UdpSend { dst: ip("10.0.0.2"), dport: 5000, n: 4 }), (T0 + 7 * MS, Step::UdpSend { dst: a2, dport: 5000, n: 4 }), (T0 + 8 * MS, Step::UdpSend { dst: a2, dport: 5001, n: 4 }), (T0 + 9 * MS, Step::UdpSend { dst: ip("10.0.0.2"), dport: 5001, n: 4 })]);
    t.check("dual: probes", ok);
    run_to(&mut e, T0 + 20 * MS);
    t.eq("what may share a port", probe(&e, PC2, "srv").map(|p| p.binds.clone()), Some(vec![true, true, true, true, true, false, false, false, false, true]));
    let fate = |name: &str| {
        probe(&e, PC1, name).map(|p| {
            if p.first(|x| matches!(x, SvcEvent::TcpConnected { .. })).is_some() {
                "connected"
            } else if p.first(|x| matches!(x, SvcEvent::TcpClosed { end: TcpEnd::Refused, .. })).is_some() {
                "refused"
            } else {
                "?"
            }
        })
    };
    let all: Vec<Option<&str>> = ["v4to7", "v6to7", "v4to8", "v6to8", "v4to9", "v6to9"].iter().map(|n| fate(n)).collect();
    t.eq("who takes what", all, vec![Some("connected"), Some("connected"), Some("connected"), Some("refused"), Some("refused"), Some("connected")]);
    // the IPv4 peer of :: stays an IPv4 address
    let rows: Vec<(TcpState, IpAddr, IpAddr)> = e.tcp_conns(PC2).iter().filter(|r| r.local.1 == 7).map(|r| (r.state, r.local.0, r.remote.0)).collect();
    t.eq("rows on port 7", rows, vec![(TcpState::Listen, ANY, ANY), (TcpState::Established, ip("10.0.0.2"), ip("10.0.0.1")), (TcpState::Established, a2, ip("2001:db8:1::1"))]);
    t.eq("udp on ::", probe(&e, PC2, "srv").map(|p| p.senders()), Some(vec![ip("10.0.0.1"), ip("2001:db8:1::1")]));
    t.eq("udp on 0.0.0.0", probe(&e, PC2, "srv2").map(|p| p.senders()), Some(vec![ip("10.0.0.1")]));
    // ss: * for :: on a device with IPv6, brackets for an IPv6 address; -4 and -6
    t.eq("ss: the listeners", column(&cli(&mut e, PC2, &["ss -tln"]), 3), vec![s("*:7"), s("0.0.0.0:8"), s("10.0.0.2:10"), s("[2001:db8:1::2]:10"), s("[2001:db8:1::2]:9")]);
    t.eq("ss -4", column(&cli(&mut e, PC2, &["ss -tln -4"]), 3), vec![s("0.0.0.0:8"), s("10.0.0.2:10")]);
    t.eq("ss -6", column(&cli(&mut e, PC2, &["ss -tln -6"]), 3), vec![s("*:7"), s("[2001:db8:1::2]:10"), s("[2001:db8:1::2]:9")]);
    t.eq("ss: the peers of PC1", column(&cli(&mut e, PC1, &["ss -tn"]), 4), vec![s("10.0.0.2:7"), s("10.0.0.2:8"), s("[2001:db8:1::2]:7"), s("[2001:db8:1::2]:9")]);
    t.eq("ss: udp", column(&cli(&mut e, PC2, &["ss -uln"]), 3), vec![s("*:5000"), s("0.0.0.0:5001")]);
}

// ---------------------------------------------------------------- names

const ZONE: &str = r#"{"zones": [{"origin": "lab", "ttl": 300, "records": ["www A 10.0.0.2", "www AAAA 2001:db8:1::2", "v4 A 10.0.0.3", "v6 AAAA 2001:db8:1::3", "web CNAME www"]}]}"#;

/// the LAN with a name server on PC2; PC1 asks it over IPv6, PC4 over IPv4
fn named(seed: u64) -> Engine {
    let mut e = lan6(seed);
    let _ = e.apply(Action::Service { dev: PC2, def: def("dns-server", "named", ZONE) });
    let _ = e.apply(Action::NameServers { dev: PC1, servers: vec![ip("2001:db8:1::2")] });
    let _ = e.apply(Action::Resolver { dev: PC1, servers: None, search: Some(vec![s("lab")]) });
    let _ = e.apply(Action::NameServers { dev: PC4, servers: vec![ip("10.0.0.2")] });
    let _ = e.apply(Action::Resolver { dev: PC4, servers: None, search: Some(vec![s("lab")]) });
    e
}

// A and AAAA at once, the order of RFC 6724; one family; an IPv4-only device as before
fn resolver(t: &mut T) {
    let mut e = named(SEED);
    run_to(&mut e, T0);
    let look = |e: &mut Engine, dev: u32, name: &str, fam: jfn_sim::Family| -> u32 { e.lookup_host_of(dev, name, fam).unwrap_or(0) };
    let any = look(&mut e, PC1, "www", jfn_sim::Family::Any);
    let v4 = look(&mut e, PC1, "v4", jfn_sim::Family::Any);
    let v6 = look(&mut e, PC1, "v6.lab", jfn_sim::Family::V6);
    let only4 = look(&mut e, PC1, "www.lab", jfn_sim::Family::V4);
    let ll = look(&mut e, PC4, "www", jfn_sim::Family::Any);
    run_to(&mut e, T0 + SEC);
    let sorted = |e: &Engine, dev: u32, id: u32| e.lookup_state(dev, id).and_then(|l| l.result.clone()).and_then(|r| r.ok()).map(|a| a.sorted);
    // a global IPv6 address: IPv6 first (precedence); only a link-local one: IPv4 first (scope)
    t.eq("www: IPv6 first", sorted(&e, PC1, any), Some(vec![ip("2001:db8:1::2"), ip("10.0.0.2")]));
    t.eq("www from a link-local host: IPv4 first", sorted(&e, PC4, ll), Some(vec![ip("10.0.0.2"), ip("2001:db8:1::2")]));
    t.eq("an A record alone", sorted(&e, PC1, v4), Some(vec![ip("10.0.0.3")]));
    t.eq("AAAA only", sorted(&e, PC1, v6), Some(vec![ip("2001:db8:1::3")]));
    t.eq("A only", sorted(&e, PC1, only4), Some(vec![ip("10.0.0.2")]));
    // both questions leave together from one socket over IPv6, each with its own id
    let tries = |e: &Engine, dev: u32, id: u32| e.lookup_state(dev, id).map(|l| l.tries.iter().map(|x| (x.name.clone(), x.qtype, x.server, x.sent)).collect::<Vec<_>>()).unwrap_or_default();
    t.eq("www: A and AAAA to the server's IPv6 address", tries(&e, PC1, any), vec![(s("www.lab"), 1, ip("2001:db8:1::2"), T0), (s("www.lab"), 28, ip("2001:db8:1::2"), T0)]);
    t.eq("-4: A alone", tries(&e, PC1, only4).iter().map(|x| x.1).collect::<Vec<_>>(), vec![1]);
    // a device without IPv6 asks for A alone, as before
    let mut p = Project::new("lan4", SEED);
    for (i, m) in ["pc", "pc", "jf-s24p"].iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 100, 0);
    }
    wire(&mut p, PC1, "eth0", SW1, "gi1/0/1");
    wire(&mut p, PC2, "eth0", SW1, "gi1/0/2");
    set(&mut p, PC1, "PC1", s("hostname PC1\nip addr add 10.0.0.1/24 dev eth0\n"));
    set(&mut p, PC2, "PC2", s("hostname PC2\nip addr add 10.0.0.2/24 dev eth0\n"));
    let mut v4only = engine(&p);
    let _ = v4only.apply(Action::Service { dev: PC2, def: def("dns-server", "named", ZONE) });
    let _ = v4only.apply(Action::Resolver { dev: PC1, servers: Some(vec![netpkt::Ipv4([10, 0, 0, 2])]), search: None });
    run_to(&mut v4only, T0);
    let id = v4only.lookup_host_of(PC1, "www.lab", jfn_sim::Family::Any).unwrap_or(0);
    run_to(&mut v4only, T0 + SEC);
    t.eq("no IPv6: A alone", tries(&v4only, PC1, id).iter().map(|x| (x.1, x.2)).collect::<Vec<_>>(), vec![(1, ip("10.0.0.2"))]);
    t.eq("no IPv6: the address", sorted(&v4only, PC1, id), Some(vec![ip("10.0.0.2")]));
}

// the services over IPv6: the web server through curl, the TCP echo through the TCP client, a
// script's http.request and dns.resolve
fn services(t: &mut T) {
    let mut e = named(SEED);
    let ok = e.apply(Action::Service { dev: PC2, def: def("http-server", "web", r#"{"routes": [{"path": "/", "body": "hello\n"}]}"#) }).is_ok()
        && e.apply(Action::Service { dev: PC2, def: def("tcp-echo", "echo", r#"{"port": 7}"#) }).is_ok();
    t.check("services: set up", ok);
    run_to(&mut e, T0);
    t.eq("curl by address", cli_for(&mut e, PC1, "curl -s http://[2001:db8:1::2]/", SEC), s("hello\n"));
    let v = cli_for(&mut e, PC1, "curl -v http://www.lab/", SEC);
    t.eq("curl by name: IPv6 first", line_of(&v, "*   Trying"), Some(s("*   Trying [2001:db8:1::2]:80...")));
    t.check("curl by name: the page", v.contains("> Host: www.lab") && v.contains("\nhello\n"));
    let v4 = cli_for(&mut e, PC1, "curl -4 -v http://www.lab/", SEC);
    t.eq("curl -4", line_of(&v4, "*   Trying"), Some(s("*   Trying 10.0.0.2:80...")));
    t.check("curl -4: the page", v4.contains("\nhello\n"));
    t.eq("curl -6 without AAAA", cli_for(&mut e, PC1, "curl -6 -s -S http://v4.lab/", SEC), s("curl: (6) Could not resolve host: v4.lab\n"));
    let ll = cli_for(&mut e, PC4, "curl -v http://www.lab/", SEC);
    t.eq("curl from a link-local host: IPv4 first", line_of(&ll, "*   Trying"), Some(s("*   Trying 10.0.0.2:80...")));
    let h = cli_for(&mut e, PC1, "curl -s http://[2001:db8:1::2]:80/", SEC);
    t.eq("curl with a port in brackets", h, s("hello\n"));
    // the TCP client against the echo over IPv6
    let ok = e.apply(Action::Service { dev: PC1, def: def("tcp-client", "client", r#"{"server": "2001:db8:1::2", "port": 7, "bytes": 10000}"#) }).is_ok();
    t.check("tcp client: set up", ok);
    run_for(&mut e, SEC);
    t.eq("tcp client: echoed", (e.metric(PC1, "client", "sent"), e.metric(PC1, "client", "received")), (Some(10000), Some(10000)));
    t.eq("echo server: bytes", e.metric(PC2, "echo", "bytes_in"), Some(10000));
    // a script: an IPv6 URL, a name with both families
    let src = "http.request(\"http://[2001:db8:1::2]/\", fn(err, resp) { log(\"v6: ${resp.status} ${resp.body}\") })\ndns.resolve(\"www\", fn(err, a) { log(\"www: ${a}\") })\n";
    let mut c = agent_core::jmap::Value::Object(Vec::new());
    c.insert("source", agent_core::jmap::Value::string(src));
    let ok = e.apply(Action::Service { dev: PC1, def: ServiceDef { kind: s("script"), name: s("sc"), disabled: false, config: c } }).is_ok();
    t.check("script: set up", ok);
    run_for(&mut e, SEC);
    let logs: Vec<String> = e.log_lines(PC1).into_iter().filter(|l| l.tag == "sc").map(|l| l.text).collect();
    t.eq("script: http and dns", logs, vec![s("www: [\"2001:db8:1::2\", \"10.0.0.2\"]"), s("v6: 200 hello\n")]);
}

// the shell's tools by name and by address over both families
fn tools(t: &mut T) {
    let mut e = named(SEED);
    let _ = e.apply(Action::Service { dev: PC2, def: def("http-server", "web", r#"{"routes": [{"path": "/", "body": "hello\n"}]}"#) });
    run_to(&mut e, T0);
    let last_syn = |e: &Engine| e.trace.hops().filter(|h| h.from.0 == PC1 && h.mask & P_TCP != 0).last().map(|h| h.mask & (P_IPV4 | P_IPV6));
    let nc = cli_for(&mut e, PC1, "nc -z -v www.lab 80", SEC);
    t.eq("nc -z by name", (nc, last_syn(&e)), (s("Connection to www.lab 80 port [tcp/http] succeeded!\n"), Some(P_IPV6)));
    let nc4 = cli_for(&mut e, PC1, "nc -4 -z -v www.lab 80", SEC);
    t.eq("nc -4", (nc4, last_syn(&e)), (s("Connection to www.lab 80 port [tcp/http] succeeded!\n"), Some(P_IPV4)));
    let p = cli_for(&mut e, PC1, "ping -c 1 www", 2 * SEC);
    t.check("ping by name over IPv6", p.starts_with("PING www.lab(2001:db8:1::2) 56 data bytes\n64 bytes from 2001:db8:1::2: icmp_seq=1 ttl=64 time=") && p.contains("1 packets transmitted, 1 received, 0% packet loss"));
    let p4 = cli_for(&mut e, PC1, "ping -4 -c 1 www", 2 * SEC);
    t.check("ping -4 by name", p4.starts_with("PING www.lab (10.0.0.2) 56(84) bytes of data.\n64 bytes from 10.0.0.2: icmp_seq=1 ttl=64 time="));
    let p6 = cli_for(&mut e, PC1, "ping -6 -c 1 v6", 5 * SEC);
    t.check("ping -6 by name (no answer from ::3)", p6.starts_with("PING v6.lab(2001:db8:1::3) 56 data bytes\n"));
    let tr = cli_for(&mut e, PC1, "traceroute -6 -n www", 5 * SEC);
    t.check("traceroute -6 by name", tr.starts_with("traceroute to www.lab (2001:db8:1::2), 30 hops max, 80 byte packets\n 1  2001:db8:1::2  "));
    // dig, host, nslookup: their own questions to the name server over IPv6
    let d = cli_for(&mut e, PC1, "dig AAAA www.lab", SEC);
    t.check("dig: header", d.starts_with("\n; dig of the JFNetworker shell: AAAA www.lab\n;; ->>HEADER<<- opcode: QUERY, status: NOERROR, id: "));
    t.check("dig: flags", d.contains(";; flags: qr aa rd; QUERY: 1, ANSWER: 1, AUTHORITY: 0, ADDITIONAL: 1\n;; WARNING: recursion requested but not available\n\n;; OPT PSEUDOSECTION:\n; EDNS: version: 0, flags:; udp: 1232\n"));
    t.check("dig: question and answer", d.contains(";; QUESTION SECTION:\n;www.lab.\t\t\tIN\tAAAA\n\n;; ANSWER SECTION:\nwww.lab.\t\t300\tIN\tAAAA\t2001:db8:1::2\n"));
    t.check("dig: server", d.contains(";; SERVER: 2001:db8:1::2#53(2001:db8:1::2) (UDP)\n;; WHEN: ") && d.contains(";; MSG SIZE  rcvd: "));
    t.eq("dig +short", cli_for(&mut e, PC1, "dig +short web.lab", SEC), s("www.lab.\n10.0.0.2\n"));
    let d4 = cli_for(&mut e, PC4, "dig @10.0.0.2 www.lab", SEC);
    t.check("dig over IPv4", d4.contains(";; ANSWER SECTION:\nwww.lab.\t\t300\tIN\tA\t10.0.0.2\n") && d4.contains(";; SERVER: 10.0.0.2#53(10.0.0.2) (UDP)\n"));
    let nx = cli_for(&mut e, PC1, "dig nope.lab", SEC);
    t.check("dig: NXDOMAIN with the SOA", nx.contains("status: NXDOMAIN") && nx.contains(";; AUTHORITY SECTION:\nlab.\t\t\t300\tIN\tSOA\tns.lab. hostmaster.lab. 1 3600 600 86400 300\n"));
    t.eq("host", cli_for(&mut e, PC1, "host www", SEC), s("www.lab has address 10.0.0.2\nwww.lab has IPv6 address 2001:db8:1::2\n"));
    t.eq("host: an alias", cli_for(&mut e, PC1, "host web", SEC), s("web.lab is an alias for www.lab.\nwww.lab has address 10.0.0.2\nwww.lab has IPv6 address 2001:db8:1::2\n"));
    t.eq("host -t AAAA without one", cli_for(&mut e, PC1, "host -t AAAA v4", SEC), s("v4.lab has no AAAA record\n"));
    t.eq("host: not found", cli_for(&mut e, PC1, "host nope.lab", SEC), s("Host nope.lab not found: 3(NXDOMAIN)\n"));
    t.eq("host with a server", cli_for(&mut e, PC4, "host v6.lab 10.0.0.2", SEC), s("Using domain server:\nName: 10.0.0.2\nAddress: 10.0.0.2#53\nAliases: \n\nv6.lab has IPv6 address 2001:db8:1::3\n"));
    t.eq("nslookup", cli_for(&mut e, PC1, "nslookup www", SEC), s("Server:\t\t2001:db8:1::2\nAddress:\t2001:db8:1::2#53\n\nName:\twww.lab\nAddress: 10.0.0.2\nName:\twww.lab\nAddress: 2001:db8:1::2\n\n"));
    t.eq("nslookup: not found", cli_for(&mut e, PC1, "nslookup nope.lab", SEC), s("Server:\t\t2001:db8:1::2\nAddress:\t2001:db8:1::2#53\n\n** server can't find nope.lab: NXDOMAIN\n\n"));
    // nobody at the address: three tries of 5 s
    let lost = cli_for(&mut e, PC1, "dig @2001:db8:1::77 www.lab", 16 * SEC);
    t.eq("dig: no answer", lost, s(";; communications error to 2001:db8:1::77#53: timed out\n;; communications error to 2001:db8:1::77#53: timed out\n;; communications error to 2001:db8:1::77#53: timed out\n\n; dig of the JFNetworker shell: @2001:db8:1::77 www.lab\n;; no servers could be reached\n"));
    // a port nobody listens on: refused at once
    let refused = cli_for(&mut e, PC1, "dig @2001:db8:1::1 www.lab", SEC);
    t.check("dig: refused", refused.starts_with(";; communications error to 2001:db8:1::1#53: connection refused\n;; communications error to 2001:db8:1::1#53: connection refused\n;; communications error to 2001:db8:1::1#53: connection refused\n"));
}

// the IOS-like console: name servers and host names of both families, ping by name, the
// configuration, show tcp brief with an IPv6 connection
fn ios(t: &mut T) {
    let mut e = wan6(SEED);
    let zone = r#"{"zones": [{"origin": "lab", "ttl": 300, "records": ["www A 10.0.3.3", "www AAAA 2001:db8:3::3", "v6 AAAA 2001:db8:3::3"]}]}"#;
    let ok = e.apply(Action::Service { dev: PC3, def: def("dns-server", "named", zone) }).is_ok() && e.apply(Action::Service { dev: R1, def: def("tcp-echo", "echo", r#"{"port": 7}"#) }).is_ok();
    t.check("ios: set up", ok);
    run_to(&mut e, T0);
    let _ = cli(&mut e, R1, &["enable", "configure terminal", "ip name-server 2001:DB8:3::3", "ip host srv4 10.0.3.3", "ipv6 host srv6 2001:DB8:3::3", "end"]);
    let p = cli_for(&mut e, R1, "ping www.lab", 3 * SEC);
    t.check("ping by name: IPv4 first", p.starts_with("Resolving \"www.lab\" via DNS (2001:DB8:3::3) ...\nCtrl+Shift+6 stops the ping.\nPinging 10.0.3.3 with 5 echo requests of 100 bytes, 2 s timeout each:\n!!!!!"));
    let p6 = cli_for(&mut e, R1, "ping ipv6 www.lab", 3 * SEC);
    t.check("ping ipv6 by name", p6.starts_with("Resolving \"www.lab\" via DNS (2001:DB8:3::3) ...\nCtrl+Shift+6 stops the ping.\nPinging 2001:DB8:3::3 with 5 echo requests of 100 bytes, 2 s timeout each:\n!!!!!"));
    let only6 = cli_for(&mut e, R1, "ping v6.lab", 3 * SEC);
    t.check("ping of a name with IPv6 only", only6.contains("Pinging 2001:DB8:3::3 with 5 echo requests of 100 bytes") && only6.contains("!!!!!"));
    let h6 = cli_for(&mut e, R1, "ping srv6", 3 * SEC);
    t.check("ipv6 host: no name server asked", h6.starts_with("Ctrl+Shift+6 stops the ping.\nPinging 2001:DB8:3::3 with") && h6.contains("!!!!!"));
    let h4 = cli_for(&mut e, R1, "ping srv4", 3 * SEC);
    t.check("ip host", h4.starts_with("Ctrl+Shift+6 stops the ping.\nPinging 10.0.3.3 with") && h4.contains("!!!!!"));
    let nx = cli_for(&mut e, R1, "ping nope.lab", 3 * SEC);
    t.eq("unknown name", nx, s("Resolving \"nope.lab\" via DNS (2001:DB8:3::3) ...\n% Unrecognized host or address, or protocol not running.\n\n"));
    let run = cli(&mut e, R1, &["show running-config"]);
    t.check("configuration", run.contains("!\nip host srv4 10.0.3.3\nipv6 host srv6 2001:DB8:3::3\nip name-server 2001:DB8:3::3\n!\n"));
    let _ = cli(&mut e, R1, &["configure terminal", "no ipv6 host srv6", "no ip name-server", "end"]);
    let run = cli(&mut e, R1, &["show running-config"]);
    t.check("configuration: removed", run.contains("ip host srv4 10.0.3.3\n!\n") && !run.contains("ipv6 host") && !run.contains("ip name-server"));
    t.eq("no name server", cli_for(&mut e, R1, "ping www.lab", SEC), s("Resolving \"www.lab\" via DNS (255.255.255.255) ...\n% Unrecognized host or address, or protocol not running.\n\n"));
    t.eq("ping ipv6: no name server", cli_for(&mut e, R1, "ping ipv6 www.lab", SEC), s("% Unrecognized host or address, or protocol not running.\n\n"));
    // the echo on R1 over IPv6: show tcp brief in capitals
    let ok = e.apply(Action::Service { dev: PC1, def: def("tcp-client", "client", r#"{"server": "2001:db8:1::fe", "port": 7, "bytes": 1000000, "chunk": 1000, "interval_ms": 100}"#) }).is_ok();
    t.check("ios: client", ok);
    run_for(&mut e, 50 * MS);
    let b = cli(&mut e, R1, &["show tcp brief"]);
    t.check("show tcp brief", b.contains("  2001:DB8:1::FE.7 ") && b.contains(" 2001:DB8:1::1.") && b.contains("ESTAB"));
    let open = cli(&mut e, R1, &["show control-plane host open-ports"]);
    t.check("open ports", open.contains("*:7") && open.contains("[2001:DB8:1::FE]:7"));
}

// ---------------------------------------------------------------- notes, capture, determinism

// IPv6 in the masks and sentences, the capture file, the dissector
fn notes(t: &mut T) {
    let mut e = lan6(SEED);
    let ok = add_probe(&mut e, PC2, "srv", vec![(0, Step::Listen(7))]) && add_probe(&mut e, PC1, "cli", vec![(T0, Step::Connect { dst: ip("2001:db8:1::2"), dport: 7 })]);
    t.check("notes: probes", ok);
    run_to(&mut e, T0 + MS);
    let hop_mask = |id: Option<u64>| id.and_then(|h| e.trace.hop(h)).map_or(0, |h| h.mask);
    let est = e.trace.events().find(|x| x.dev == PC1 && matches!(x.note, Note::TcpStateChange { to: TcpState::Established, .. })).cloned();
    t.eq("mask: IPv6, not IPv4", est.as_ref().map(|x| x.mask(hop_mask(x.hop)) & (P_IPV4 | P_IPV6 | P_TCP)), Some(P_IPV6 | P_TCP));
    let (port, _) = linux_connect(SEED, PC1);
    t.eq("describe", est.map(|x| e.describe(&x)), Some(format!("TCP {} \u{2194} [2001:db8:1::2]:7: SYN-SENT \u{2192} ESTABLISHED.", port)));
    let syn = e.trace.hops().find(|h| h.from.0 == PC1 && seg_of(h).is_some()).cloned();
    t.check("dissector", syn.as_ref().is_some_and(|h| decode::info(&h.bytes).1.starts_with(&format!("{} \u{2192} 7 [SYN] Seq=", port)) && decode::layers(&h.bytes).iter().all(|l| l.problems.is_empty())));
    t.check("hop mask", syn.as_ref().is_some_and(|h| h.mask & (P_IPV6 | P_TCP) == P_IPV6 | P_TCP && h.mask & P_IPV4 == 0));
    let cap = pcapng::read(&e.pcapng(&PcapOpts { scope: PcapScope::Device(PC1), filter: jfn_sim::Filter::protocols(P_TCP), ..PcapOpts::default() }).bytes).unwrap_or_default();
    t.eq("capture: the three segments", cap.packets.len(), 3);
    t.check("capture: the frames as sent", cap.packets.first().zip(syn.as_ref()).is_some_and(|(p, h)| p.data == *h.bytes));
}

/// what a run of the services did: (time, from, bytes) of every frame
fn digest(seed: u64) -> Vec<(u64, u32, Vec<u8>)> {
    let mut e = named(seed);
    let _ = e.apply(Action::Service { dev: PC2, def: def("http-server", "web", r#"{"routes": [{"path": "/", "body": "hello\n"}]}"#) });
    run_to(&mut e, T0);
    let _ = cli_for(&mut e, PC1, "curl -s http://www.lab/", SEC);
    let _ = cli_for(&mut e, PC1, "dig AAAA www.lab", SEC);
    e.trace.hops().map(|h| (h.t_tx, h.from.0, h.bytes.to_vec())).collect()
}

fn determinism(t: &mut T) {
    let (a, b) = (digest(SEED), digest(SEED));
    t.check("two runs, one result", !a.is_empty() && a == b);
    t.check("another seed, other ports", digest(SEED + 1) != a);
}
