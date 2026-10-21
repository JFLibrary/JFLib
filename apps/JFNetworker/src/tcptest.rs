// tcptest -- checks of TCP in jfn_sim (part of --selftest): the handshake byte for byte (port,
// ISN and IP identification from the device's random numbers, MSS from the MTU, Linux's
// window), a transfer with echo and its end (quick ACKs, ping-pong, the FIN-ACK, TIME-WAIT
// 60 s), loss with RTO (Karn, backoff, the 924.6 s until Linux gives up, a lost SYN and the
// 127 s of six SYN retries), fast retransmit and recovery, reordering and duplicates, the
// zero window with its probes, simultaneous open and close, RST at a closed port and the RST
// rules (challenge ACKs), TIME-WAIT with a repeated FIN and a new SYN, Nagle and delayed
// ACKs, keepalive, the backlog and closing with unread data, IOS-like values (MSS 536,
// window 4128, ACK hold, probes with a byte), ICMP errors and path MTU, connections to the
// device itself, the views and determinism
//
// Times: 1 Gbit/s copper with 50 ns of cable and a switch between the hosts: a frame of n
// bytes takes (max(n, 60) + 24) * 8 + 50 ns per cable.  A segment without data (54 bytes,
// padded) and a SYN with the MSS option take 722 ns, 100 bytes of data 1474 ns, 1460 bytes
// 12354 ns; over the switch twice that.  ARP is set up by hand, so no lookups get in between.

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
use jfn_sim::{Action, Engine, Hop, Metric, Note, OsApi, RstWhy, Service, SockOpts, SvcEvent, TcpEnd, TcpState, TcpTimer, MS, SEC, US};
use netpkt::eth::{self, EthHdr, ET_IPV4};
use netpkt::ipv4::{Ipv4Hdr, Ipv4View, PROTO_TCP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::tcp::{TcpHdr, TcpOpts, TcpView, ACK, FIN, PSH, RST, SYN};
use netpkt::{decode, Ipv4, Mac, Prefix4};

use crate::selftest::T;

const BUDGET: usize = 4_000_000;
const SEED: u64 = 3;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn ip(s: &str) -> Ipv4 {
    Ipv4::parse(s).unwrap_or(Ipv4::UNSPECIFIED)
}

fn pf(s: &str) -> Prefix4 {
    Prefix4::parse(s).unwrap_or(Prefix4 { addr: Ipv4::UNSPECIFIED, len: 0 })
}

fn mac(dev: u32) -> Mac {
    port_mac(dev, 0)
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: String::from(kind), name: String::from(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Value::Null) }
}

// ---------------------------------------------------------------- a scripted service

/// what a Probe does at a time
#[derive(Clone, Debug)]
enum Step {
    Listen(u16),
    /// listen with a backlog
    Backlog(u16, u32),
    Connect { sport: u16, dst: Ipv4, dport: u16 },
    Send(usize),
    Accept,
    Shutdown,
    Close,
    CloseListener,
}

/// a service that does what a test says at given times and keeps what it hears
#[derive(Clone, Debug, Default)]
struct Probe {
    plan: Vec<(u64, Step)>,
    /// read data at once (else nothing is read)
    read: bool,
    /// take connections at once (else only Step::Accept takes them)
    accept: bool,
    opts: SockOpts,
    listener: Option<u32>,
    sock: Option<u32>,
    sent: usize,
    /// bytes a Send step still wants to write
    pending: usize,
    got: Vec<u8>,
    log: Vec<(u64, SvcEvent)>,
}

impl Probe {
    fn new(plan: Vec<(u64, Step)>) -> Probe {
        Probe { plan, read: true, accept: true, ..Probe::default() }
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
                Step::Listen(port) => self.listener = os.tcp_listen(port, 0, self.opts).ok(),
                Step::Backlog(port, b) => self.listener = os.tcp_listen(port, b, self.opts).ok(),
                Step::Connect { sport, dst, dport } => self.sock = os.tcp_connect_from(sport, dst, dport, self.opts).ok(),
                Step::Send(n) => {
                    self.pending += n;
                    self.write(os);
                }
                Step::Accept => {
                    if let Some(l) = self.listener {
                        while let Some(c) = os.tcp_accept(l) {
                            self.sock = Some(c);
                        }
                    }
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
                Step::CloseListener => {
                    if let Some(l) = self.listener.take() {
                        os.tcp_close(l);
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
            SvcEvent::TcpAccept { listener, .. } if self.accept => {
                while let Some(s) = os.tcp_accept(listener) {
                    self.sock = Some(s);
                }
            }
            SvcEvent::TcpData { sock, .. } if self.read => {
                let d = os.tcp_recv(sock, usize::MAX);
                self.got.extend_from_slice(&d);
            }
            SvcEvent::TcpWritable { .. } => self.write(os),
            SvcEvent::TcpClosed { sock, .. } => {
                if self.read {
                    let d = os.tcp_recv(sock, usize::MAX);
                    self.got.extend_from_slice(&d);
                }
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

// ---------------------------------------------------------------- set-ups and helpers

/// PC1 (10.0.0.1) and PC2 (10.0.0.2) on a switch, ARP entries set by hand: (engine, pc1,
/// pc2, ok)
fn lan_seed(seed: u64) -> (Engine, u32, u32, bool) {
    let mut p = Project::new("tcp", seed);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 200, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 100).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(pc2, "eth0"), &pr(sw, "gi1/0/2"), None);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let ok = [
        e.apply(Action::IfAddr { dev: pc1, iface: String::from("eth0"), prefix: pf("10.0.0.1/24"), add: true }),
        e.apply(Action::IfAddr { dev: pc2, iface: String::from("eth0"), prefix: pf("10.0.0.2/24"), add: true }),
        e.apply(Action::StaticArp { dev: pc1, ip: ip("10.0.0.2"), mac: Some(mac(pc2)) }),
        e.apply(Action::StaticArp { dev: pc2, ip: ip("10.0.0.1"), mac: Some(mac(pc1)) }),
    ];
    (e, pc1, pc2, ok.iter().all(|r| r.is_ok()))
}

fn lan() -> (Engine, u32, u32, bool) {
    lan_seed(SEED)
}

/// PC1 (10.0.0.1) and FAKE on a switch; FAKE has no address, and PC1 finds 10.0.0.9 at FAKE's
/// MAC: frames from FAKE's port play a peer byte for byte, what PC1 sends there is dropped
fn fake_lan() -> (Engine, u32, u32, bool) {
    let mut p = Project::new("fake", SEED);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let fake = p.add_device("pc", 200, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 100).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(fake, "eth0"), &pr(sw, "gi1/0/2"), None);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let ok = [
        e.apply(Action::IfAddr { dev: pc1, iface: String::from("eth0"), prefix: pf("10.0.0.1/24"), add: true }),
        e.apply(Action::StaticArp { dev: pc1, ip: ip("10.0.0.9"), mac: Some(mac(fake)) }),
    ];
    (e, pc1, fake, ok.iter().all(|r| r.is_ok()))
}

/// the frame of a TCP segment as the stacks build it (Ethernet, IPv4, TCP, padding)
fn frame(from: Mac, to: Mac, src: Ipv4, dst: Ipv4, ttl: u8, df: bool, id: u16, h: TcpHdr, data: &[u8]) -> Vec<u8> {
    let mut p = Pkt::new(HEADROOM, data);
    h.push_v4(&mut p, src, dst);
    let mut ih = Ipv4Hdr::new(src, dst, PROTO_TCP, ttl);
    ih.df = df;
    ih.id = id;
    ih.push_onto(&mut p);
    EthHdr { dst: to, src: from, ethertype: ET_IPV4 }.push_onto(&mut p);
    let mut f = p.into_vec();
    eth::pad(&mut f);
    f
}

fn hdr(sport: u16, dport: u16, seq: u32, ack: u32, flags: u8, window: u16, mss: Option<u16>) -> TcpHdr {
    TcpHdr { src_port: sport, dst_port: dport, seq, ack, flags, window, urgent: 0, opts: TcpOpts { mss, ..TcpOpts::default() } }
}

/// a segment from the fake peer 10.0.0.9 to PC1, sent now
fn inject(e: &mut Engine, fake: u32, pc1: u32, sport: u16, dport: u16, seq: u32, ack: u32, flags: u8, wnd: u16, mss: Option<u16>, data: &[u8]) -> bool {
    let f = frame(mac(fake), mac(pc1), ip("10.0.0.9"), ip("10.0.0.1"), 64, true, 1, hdr(sport, dport, seq, ack, flags, wnd, mss), data);
    e.apply(Action::Inject { dev: fake, port: String::from("eth0"), frame: f }).is_ok()
}

/// a TCP segment on a cable
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
}

fn seg_of(h: &Hop) -> Option<Seg> {
    let v = Ipv4View::parse(h.bytes.get(14..)?)?;
    if v.proto() != PROTO_TCP {
        return None;
    }
    let s = TcpView::parse(v.payload())?;
    Some(Seg { t: h.t_tx, from: h.from.0, sport: s.src_port(), dport: s.dst_port(), seq: s.seq(), ack: s.ack(), flags: s.flags(), wnd: s.window(), len: s.payload().len(), mss: s.opts().mss })
}

/// the TCP segments a device sent (the switch's copies left out)
fn sent(e: &Engine, dev: u32) -> Vec<Seg> {
    e.trace.hops().filter(|h| h.from.0 == dev).filter_map(seg_of).collect()
}

/// (time, flags, sequence number - iss, acknowledgement - irs (0 without ACK), window,
/// length) of what a device sent
fn rel(e: &Engine, dev: u32, iss: u32, irs: u32) -> Vec<(u64, u8, u32, u32, u16, usize)> {
    sent(e, dev).iter().map(|s| (s.t, s.flags, s.seq.wrapping_sub(iss), if s.flags & ACK != 0 { s.ack.wrapping_sub(irs) } else { 0 }, s.wnd, s.len)).collect()
}

/// the times and states a device's connections went to
fn states(e: &Engine, dev: u32) -> Vec<(u64, TcpState)> {
    e.trace.events().filter(|x| x.dev == dev).filter_map(|x| if let Note::TcpStateChange { to, .. } = x.note { Some((x.t, to)) } else { None }).collect()
}

/// the notes of a device, with their times
fn notes(e: &Engine, dev: u32) -> Vec<(u64, Note)> {
    e.trace.events().filter(|x| x.dev == dev).map(|x| (x.t, x.note.clone())).collect()
}

fn note_times(e: &Engine, dev: u32, f: impl Fn(&Note) -> bool) -> Vec<u64> {
    e.trace.events().filter(|x| x.dev == dev && f(&x.note)).map(|x| x.t).collect()
}

fn link_of(e: &Engine, dev: u32) -> u32 {
    e.view().links.iter().find(|l| l.ends.iter().any(|x| x.0 == dev)).map(|l| l.id).unwrap_or(0)
}

fn set_loss(e: &mut Engine, link: u32, ppm: u32) {
    let _ = e.apply(Action::Impair { link, kbps: None, delay_ns: None, jitter_ns: None, loss_ppm: Some(ppm) });
}

/// loses what a link starts to send at `t`
fn lose_at(e: &mut Engine, link: u32, t: u64) {
    e.run_until(t - 1, BUDGET);
    set_loss(e, link, 1_000_000);
    e.run_until(t, BUDGET);
    set_loss(e, link, 0);
}

fn metric(e: &Engine, dev: u32, svc: &str, name: &str) -> i64 {
    e.metric(dev, svc, name).unwrap_or(-1)
}

/// the first port and ISN of a Linux connect, and the IP identification before its first
/// segment: an even port of 32768-60999 first, then the ISN (the device's random numbers)
fn linux_connect(seed: u64, dev: u32) -> (u16, u32, u16) {
    let mut g = Pcg32::derive(seed, dev as u64);
    let port = 32768 + ((g.next_u32() % 28232) & !1) as u16;
    (port, g.next_u32(), g.next_u32() as u16)
}

/// the ISN and IP identification of a passive open
fn linux_accept(seed: u64, dev: u32) -> (u32, u16) {
    let mut g = Pcg32::derive(seed, dev as u64);
    (g.next_u32(), g.next_u32() as u16)
}

pub fn run(t: &mut T) {
    handshake(t);
    transfer(t);
    rto(t);
    fast_retransmit(t);
    reorder_rst(t);
    zero_window(t);
    simultaneous(t);
    refused(t);
    time_wait(t);
    nagle_delack(t);
    keepalive(t);
    backlog_close(t);
    ios(t);
    icmp_pmtu(t);
    loopback(t);
    views(t);
    determinism(t);
}

// ---------------------------------------------------------------- the handshake

fn handshake(t: &mut T) {
    let (mut e, pc1, pc2, ok) = lan();
    t.check("lan set-up", ok);
    let srv = Probe::new(vec![(0, Step::Listen(7))]);
    let cli = Probe::new(vec![(MS, Step::Connect { sport: 0, dst: ip("10.0.0.2"), dport: 7 })]);
    t.check("probes", e.add_service(pc2, "srv", Box::new(srv), true).is_ok() && e.add_service(pc1, "cli", Box::new(cli), true).is_ok());
    e.run_until(10 * MS, BUDGET);
    let (port, iss, id1) = linux_connect(SEED, pc1);
    let (irs, id2) = linux_accept(SEED, pc2);
    t.eq("port and isn", sent(&e, pc1).first().map(|s| (s.sport, s.seq)), Some((port, iss)));
    t.eq("even port in the range", (port % 2, (32768..=60999).contains(&port)), (0, true));
    // three segments, 1444 ns apart over the switch
    t.eq("client segments", rel(&e, pc1, iss, irs), vec![(MS, SYN, 0, 0, 64240, 0), (MS + 2888, ACK, 1, 1, 64240, 0)]);
    t.eq("server segments", rel(&e, pc2, irs, iss), vec![(MS + 1444, SYN | ACK, 0, 1, 64240, 0)]);
    // byte for byte: DF, TTL 64, an IP identification per connection, the MSS of the MTU and
    // the 128 KiB buffer as 65535 rounded down to the MSS
    let bytes = |dev: u32, i: usize| e.trace.hops().filter(|h| h.from.0 == dev).nth(i).map(|h| h.bytes.to_vec());
    let (a, b) = (ip("10.0.0.1"), ip("10.0.0.2"));
    t.eq("syn bytes", bytes(pc1, 0), Some(frame(mac(pc1), mac(pc2), a, b, 64, true, id1.wrapping_add(1), hdr(port, 7, iss, 0, SYN, 64240, Some(1460)), &[])));
    t.eq("syn-ack bytes", bytes(pc2, 0), Some(frame(mac(pc2), mac(pc1), b, a, 64, true, id2.wrapping_add(1), hdr(7, port, irs, iss.wrapping_add(1), SYN | ACK, 64240, Some(1460)), &[])));
    t.eq("ack bytes", bytes(pc1, 1), Some(frame(mac(pc1), mac(pc2), a, b, 64, true, id1.wrapping_add(2), hdr(port, 7, iss.wrapping_add(1), irs.wrapping_add(1), ACK, 64240, None), &[])));
    t.check("syn in the dissector", e.trace.hops().find(|h| h.from.0 == pc1).is_some_and(|h| decode::info(&h.bytes).1 == format!("{} \u{2192} 7 [SYN] Seq={} Win=64240 Len=0 MSS=1460", port, iss)));
    t.eq("client states", states(&e, pc1), vec![(MS, TcpState::SynSent), (MS + 2888, TcpState::Established)]);
    t.eq("server states", states(&e, pc2), vec![(0, TcpState::Listen), (MS + 1444, TcpState::SynReceived), (MS + 4332, TcpState::Established)]);
    // the services hear of it
    t.eq("connected", probe(&e, pc1, "cli").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpConnected { .. }))), Some(MS + 2888));
    t.eq("accepted", probe(&e, pc2, "srv").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpAccept { .. }))), Some(MS + 4332));
    // as ss shows it: the RTT of the handshake, RTO at Linux's floor of 200 ms, ten segments
    let c = e.tcp_conns(pc1);
    t.eq(
        "client row",
        c.first().map(|r| (r.state, r.local, r.remote, r.owner.clone(), r.mss, r.snd_wnd, r.rcv_wnd, r.cwnd, r.srtt, r.rttvar, r.rto)),
        Some((TcpState::Established, (a, port), (b, 7), Some(String::from("cli")), 1460, 64240, 64240, 14600, Some(2888), 1444, 200 * MS)),
    );
    let s = e.tcp_conns(pc2);
    t.eq("server rows", s.iter().map(|r| (r.state, r.local, r.remote, r.recv_q, r.send_q)).collect::<Vec<_>>(), vec![(TcpState::Listen, (Ipv4::UNSPECIFIED, 7), (Ipv4::UNSPECIFIED, 0), 0, 128), (TcpState::Established, (b, 7), (a, port), 0, 0)]);
    t.eq("server rtt from the syn-ack", s.get(1).map(|r| r.srtt), Some(Some(2888)));
    let est = e.trace.events().find(|x| x.dev == pc1 && matches!(x.note, Note::TcpStateChange { to: TcpState::Established, .. })).cloned();
    t.eq("describe", est.map(|x| e.describe(&x)), Some(format!("TCP {} \u{2194} 10.0.0.2:7: SYN-SENT \u{2192} ESTABLISHED.", port)));
    t.eq("stats", e.tcp_stats(pc1).map(|s| (s.active_opens, s.curr_estab, s.out_segs, s.in_segs)), Some((1, 1, 2, 1)));
    t.eq("server stats", e.tcp_stats(pc2).map(|s| (s.passive_opens, s.curr_estab)), Some((1, 1)));
}

// ---------------------------------------------------------------- a transfer

fn transfer(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let ok = [
        e.apply(Action::Service { dev: pc2, def: def("tcp-echo", "echo", "{}") }),
        e.apply(Action::Service { dev: pc1, def: def("tcp-client", "cli", r#"{"server": "10.0.0.2", "port": 7, "bytes": 5000, "delay_ms": 1}"#) }),
    ];
    t.check("echo set-up", ok.iter().all(|r| r.is_ok()));
    e.run_until(100 * SEC, BUDGET);
    let (_, iss, _) = linux_connect(SEED, pc1);
    let (irs, _) = linux_accept(SEED, pc2);
    // the client writes 5000 bytes and shuts down at once: four segments (IW10 lets them all
    // go) and the FIN behind them on the cable
    t.eq(
        "client segments",
        rel(&e, pc1, iss, irs),
        vec![
            (MS, SYN, 0, 0, 64240, 0),
            (1_002_888, ACK, 1, 1, 64240, 0),
            (1_003_560, ACK, 1, 1, 64240, 1460),
            (1_015_864, ACK, 1461, 1, 64240, 1460),
            (1_028_168, ACK, 2921, 1, 64240, 1460),
            (1_040_472, PSH | ACK, 4381, 1, 64240, 620),
            (1_046_056, FIN | ACK, 5001, 1, 64240, 0),
            (1_053_648, ACK, 5002, 1461, 64240, 0),
            (1_065_952, ACK, 5002, 2921, 64240, 0),
            (1_078_256, ACK, 5002, 4381, 64240, 0),
            (1_083_840, ACK, 5002, 5001, 64240, 0),
            (1_084_512, ACK, 5002, 5002, 64240, 0),
        ],
    );
    // the server ACKs the first segment at once (quick ACKs), then echoes it; answering that
    // fast makes the connection interactive (ping-pong): later ACKs ride on the echoes, and
    // the ACK of the FIN on its own FIN
    t.eq(
        "server segments",
        rel(&e, pc2, irs, iss),
        vec![
            (1_001_444, SYN | ACK, 0, 1, 64240, 0),
            (1_028_268, ACK, 1, 1461, 64240, 0),
            (1_028_940, PSH | ACK, 1, 1461, 64240, 1460),
            (1_041_244, PSH | ACK, 1461, 2921, 64240, 1460),
            (1_053_548, PSH | ACK, 2921, 4381, 64240, 1460),
            (1_065_852, PSH | ACK, 4381, 5001, 64240, 620),
            (1_071_436, FIN | ACK, 5001, 5002, 64240, 0),
        ],
    );
    t.eq("ping-pong delays the acks", note_times(&e, pc2, |n| matches!(n, Note::TcpAckDelayed { delay } if *delay == 40 * MS)), vec![1_040_572, 1_052_876, 1_058_460, 1_059_132]);
    t.eq("client states", states(&e, pc1), vec![(MS, TcpState::SynSent), (1_002_888, TcpState::Established), (1_002_888, TcpState::FinWait1), (1_084_512, TcpState::FinWait2), (1_084_512, TcpState::TimeWait), (60 * SEC + 1_084_512, TcpState::Closed)]);
    t.eq("server states", states(&e, pc2), vec![(0, TcpState::Listen), (1_001_444, TcpState::SynReceived), (1_004_332, TcpState::Established), (1_059_132, TcpState::CloseWait), (1_059_132, TcpState::LastAck), (1_085_956, TcpState::Closed)]);
    t.eq("echoed", (metric(&e, pc2, "echo", "bytes_in"), metric(&e, pc2, "echo", "bytes_out"), metric(&e, pc2, "echo", "connections")), (5000, 5000, 1));
    t.eq("client metrics", ["sent", "received", "done", "failed", "connect_ns", "done_ns"].map(|m| metric(&e, pc1, "cli", m)), [5000, 5000, 1, 0, 2888, 84_512]);
    // slow start: every ACK adds what it acknowledges, at most an MSS
    let cwnd: Vec<u32> = e.trace.events().filter(|x| x.dev == pc1).filter_map(|x| if let Note::TcpAcked { cwnd, .. } = x.note { Some(cwnd) } else { None }).collect();
    t.eq("slow start", cwnd, vec![16060, 17520, 18980, 19600]);
    t.check("nothing left", e.tcp_conns(pc1).is_empty() && e.tcp_conns(pc2).len() == 1);
    t.check("client status", e.services(pc1).first().is_some_and(|s| s.status.contains("5000 von 5000 Bytes gesendet, 5000 empfangen")));
    // the trace: the echo of the first segment was caused by that segment reaching PC2, and
    // so was the quick ACK before it
    let arrived = e.trace.hops().find(|h| h.to.0 == pc2 && seg_of(h).is_some_and(|s| s.len == 1460 && s.seq == iss.wrapping_add(1))).map(|h| h.id);
    let caused: Vec<(u64, usize)> = e.trace.hops().filter(|h| h.from.0 == pc2 && arrived.is_some() && h.cause == arrived).filter_map(seg_of).map(|s| (s.t, s.len)).collect();
    t.eq("cause of the echo", caused, vec![(1_028_268, 0), (1_028_940, 1460)]);
    t.check("tcp in the event list", e.event_rows(&jfn_sim::Filter::protocols(decode::P_TCP), 0, 10_000).iter().any(|r| r.dev == pc2 && r.layer == 4 && r.text == "1460 Bytes in Reihenfolge angenommen."));

    // a send buffer of 3000 bytes: the write of 20000 takes 3000, TcpWritable asks for more
    // whenever a third of the buffer is free again
    let (mut e, pc1, pc2, _) = lan();
    let srv = Probe::new(vec![(0, Step::Listen(9))]);
    let mut cli = Probe::new(vec![(MS, Step::Connect { sport: 0, dst: ip("10.0.0.2"), dport: 9 }), (2 * MS, Step::Send(20_000))]);
    cli.opts = SockOpts { sndbuf: 3000, ..SockOpts::default() };
    let _ = e.add_service(pc2, "srv", Box::new(srv), true);
    let _ = e.add_service(pc1, "cli", Box::new(cli), true);
    e.run_until(2 * MS, BUDGET);
    // the send queue holds what is unacknowledged and what is not sent yet
    t.eq("short write", e.tcp_conns(pc1).first().map(|r| (r.send_q, r.snd_nxt.wrapping_sub(r.snd_una))), Some((3000, 2920)));
    e.run_until(SEC, BUDGET);
    let writable = probe(&e, pc1, "cli").map(|p| p.log.iter().filter(|x| matches!(x.1, SvcEvent::TcpWritable { .. })).count()).unwrap_or(0);
    t.check("writable events", writable >= 6);
    t.eq("all of it", (probe(&e, pc1, "cli").map(|p| p.sent), probe(&e, pc2, "srv").map(|p| p.got.len())), (Some(20_000), Some(20_000)));
    t.check("in order", probe(&e, pc2, "srv").is_some_and(|p| p.got.iter().enumerate().all(|(i, &b)| b == (i % 256) as u8)));
    // a shutdown after a short write: the rest in the buffer goes with the FIN, small tail
    // included (the service will not write more)
    let (mut e, pc1, pc2, _) = lan();
    let srv = Probe::new(vec![(0, Step::Listen(9))]);
    let mut cli = Probe::new(vec![(MS, Step::Connect { sport: 0, dst: ip("10.0.0.2"), dport: 9 }), (2 * MS, Step::Send(20_000)), (2 * MS, Step::Shutdown)]);
    cli.opts = SockOpts { sndbuf: 3000, ..SockOpts::default() };
    let _ = e.add_service(pc2, "srv", Box::new(srv), true);
    let _ = e.add_service(pc1, "cli", Box::new(cli), true);
    e.run_until(SEC, BUDGET);
    let (_, iss, _) = linux_connect(SEED, pc1);
    t.eq("tail with the fin", sent(&e, pc1).iter().filter(|s| s.len > 0 || s.flags & FIN != 0).map(|s| (s.seq.wrapping_sub(iss), s.len, s.flags & FIN != 0)).collect::<Vec<_>>(), vec![(1, 1460, false), (1461, 1460, false), (2921, 80, true)]);
    t.eq("what was taken arrived", probe(&e, pc2, "srv").map(|p| p.got.len()), Some(3000));
}

// ---------------------------------------------------------------- retransmission timeouts

fn rto(t: &mut T) {
    // a single segment lost: no duplicate ACKs, the RTO of 200 ms sends it again
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "discard", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "cli", r#"{"server": "10.0.0.2", "port": 9, "bytes": 100, "delay_ms": 1, "close": "keep"}"#) });
    let l1 = link_of(&e, pc1);
    lose_at(&mut e, l1, 1_003_560);
    e.run_until(10 * SEC, BUDGET);
    let (_, iss, _) = linux_connect(SEED, pc1);
    let (irs, _) = linux_accept(SEED, pc2);
    t.eq("lost and sent again", rel(&e, pc1, iss, irs).into_iter().skip(2).collect::<Vec<_>>(), vec![(1_003_560, PSH | ACK, 1, 1, 64240, 100), (201_002_888, PSH | ACK, 1, 1, 64240, 100)]);
    t.eq("timeout noted", notes(&e, pc1).into_iter().filter(|n| matches!(n.1, Note::TcpRto { .. })).collect::<Vec<_>>(), vec![(201_002_888, Note::TcpRto { seq: iss.wrapping_add(1), rto: 400 * MS, n: 1 })]);
    // one segment after the timeout (ssthresh 2 MSS); Karn: no RTT from what went twice, so
    // the RTO stays doubled
    let r = e.tcp_conns(pc1).into_iter().next();
    t.eq("after the timeout", r.as_ref().map(|r| (r.cwnd, r.ssthresh, r.rto, r.srtt, r.retries, r.counters.retrans, r.counters.timeouts)), Some((1560, 2920, 400 * MS, Some(2888), 0, 1, 1)));
    t.eq("acked without rtt", notes(&e, pc1).into_iter().filter(|n| matches!(n.1, Note::TcpAcked { .. })).map(|n| n.1).collect::<Vec<_>>(), vec![Note::TcpAcked { bytes: 100, cwnd: 1560, rtt: None }]);
    t.eq("discarded", metric(&e, pc2, "discard", "bytes"), 100);

    // the cable is pulled: the RTO doubles up to 120 s; after 15 retransmissions (924.6 s)
    // Linux gives up without a RST
    let (mut e, pc1, pc2, _) = lan();
    let srv = Probe::new(vec![(0, Step::Listen(9))]);
    let cli = Probe::new(vec![(MS, Step::Connect { sport: 0, dst: ip("10.0.0.2"), dport: 9 }), (2 * MS, Step::Send(100))]);
    let _ = e.add_service(pc2, "srv", Box::new(srv), true);
    let _ = e.add_service(pc1, "cli", Box::new(cli), true);
    e.run_until(2 * MS - 1, BUDGET);
    let l2 = link_of(&e, pc2);
    let _ = e.apply(Action::Unplug { link: l2, down: true });
    e.run_until(1000 * SEC, BUDGET);
    let tries: Vec<u64> = note_times(&e, pc1, |n| matches!(n, Note::TcpRto { .. })).iter().map(|x| (x - 2 * MS) / (100 * MS)).collect();
    t.eq("backoff in 100 ms", tries, vec![2, 6, 14, 30, 62, 126, 254, 510, 1022, 2046, 3246, 4446, 5646, 6846, 8046]);
    t.eq("gave up", note_times(&e, pc1, |n| *n == Note::TcpGiveUp { why: TcpEnd::Timeout }), vec![2 * MS + 924_600 * MS]);
    t.eq("service told", probe(&e, pc1, "cli").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpClosed { end: TcpEnd::Timeout, .. }))), Some(2 * MS + 924_600 * MS));
    t.check("no reset sent", !notes(&e, pc1).iter().any(|n| matches!(n.1, Note::TcpRstSent { .. })) && e.tcp_conns(pc1).is_empty());
    t.eq("timeout counted", e.tcp_stats(pc1).map(|s| (s.timeouts, s.retrans_segs, s.estab_resets)), Some((1, 15, 1)));

    // the SYN lost: again after 1 s; then one segment of initial window (RFC 5681 3.1) and an
    // RTO of 3 s until a measurement (RFC 6298 5.7)
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "discard", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "cli", r#"{"server": "10.0.0.2", "port": 9, "bytes": 3000, "delay_ms": 1, "close": "keep"}"#) });
    let l1 = link_of(&e, pc1);
    lose_at(&mut e, l1, MS);
    e.run_until(SEC + 2 * MS, BUDGET);
    let (_, iss, _) = linux_connect(SEED, pc1);
    let (irs, _) = linux_accept(SEED, pc2);
    let s = rel(&e, pc1, iss, irs);
    t.eq("syn twice", s.iter().take(2).map(|x| (x.0, x.1)).collect::<Vec<_>>(), vec![(MS, SYN), (SEC + MS, SYN)]);
    t.eq("one segment first", s.iter().skip(3).map(|x| (x.0, x.2, x.5)).collect::<Vec<_>>(), vec![(1_001_003_560, 1, 1460), (1_001_029_712, 1461, 1460), (1_001_042_016, 2921, 80)]);
    t.eq("rto after the handshake", note_times(&e, pc1, |n| matches!(n, Note::TcpRto { rto, .. } if *rto == 2 * SEC)), vec![SEC + MS]);
    t.eq("measured later", e.tcp_conns(pc1).first().map(|r| (r.rto, r.cwnd)), Some((200 * MS, 4460)));

    // nobody answers the SYN: six retries, 127 s
    let (mut e, pc1, _, _) = lan();
    let _ = e.apply(Action::StaticArp { dev: pc1, ip: ip("10.0.0.77"), mac: Some(Mac([2, 0, 0, 0, 0, 0x77])) });
    let cli = Probe::new(vec![(0, Step::Connect { sport: 0, dst: ip("10.0.0.77"), dport: 80 })]);
    let _ = e.add_service(pc1, "cli", Box::new(cli), true);
    e.run_until(200 * SEC, BUDGET);
    t.eq("syn retries", sent(&e, pc1).iter().map(|s| s.t / SEC).collect::<Vec<_>>(), vec![0, 1, 3, 7, 15, 31, 63]);
    t.eq("syn given up", probe(&e, pc1, "cli").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpClosed { end: TcpEnd::Timeout, .. }))), Some(127 * SEC));
    t.eq("attempt failed", e.tcp_stats(pc1).map(|s| (s.attempt_fails, s.timeouts)), Some((1, 1)));
}

// ---------------------------------------------------------------- fast retransmit

/// PC1 sends 30000 bytes to PC2's discard server; the fifth data segment is lost
fn lossy_transfer(seed: u64) -> (Engine, u32, u32) {
    let (mut e, pc1, pc2, _) = lan_seed(seed);
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "discard", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "cli", r#"{"server": "10.0.0.2", "port": 9, "bytes": 30000, "delay_ms": 1}"#) });
    let l1 = link_of(&e, pc1);
    lose_at(&mut e, l1, 1_003_560 + 4 * 12_304);
    e.run_until(100 * SEC, BUDGET);
    (e, pc1, pc2)
}

fn fast_retransmit(t: &mut T) {
    let (e, pc1, pc2) = lossy_transfer(SEED);
    let (_, iss, _) = linux_connect(SEED, pc1);
    let lost = iss.wrapping_add(1 + 4 * 1460);
    // the segments behind the gap: each makes PC2 hold it and ACK the gap at once
    t.eq("held back", note_times(&e, pc2, |n| matches!(n, Note::TcpOutOfOrder { .. })).len(), 15);
    t.eq("first held", notes(&e, pc2).into_iter().find(|n| matches!(n.1, Note::TcpOutOfOrder { .. })), Some((1_089_788, Note::TcpOutOfOrder { seq: lost.wrapping_add(1460), bytes: 1460 })));
    // the third duplicate ACK: ssthresh half the flight (16 segments), cwnd ssthresh + 3 MSS,
    // the lost segment again; every further duplicate inflates cwnd by an MSS
    t.eq("dupacks", note_times(&e, pc1, |n| matches!(n, Note::TcpDupAck { .. })).iter().take(3).copied().collect::<Vec<_>>(), vec![1_091_232, 1_103_536, 1_115_840]);
    t.eq("fast retransmit", notes(&e, pc1).into_iter().find(|n| matches!(n.1, Note::TcpFastRetransmit { .. })), Some((1_115_840, Note::TcpFastRetransmit { seq: lost, ssthresh: 11_680, cwnd: 16_060 })));
    // it waits in the queue of the cable behind what was sent before
    t.eq("the lost segment again", sent(&e, pc1).iter().filter(|s| s.seq == lost).map(|s| s.t).collect::<Vec<_>>(), vec![1_052_776, 1_249_640]);
    t.eq("gap closed", notes(&e, pc2).into_iter().find(|n| matches!(n.1, Note::TcpGapFilled { .. })), Some((1_274_348, Note::TcpGapFilled { bytes: 21_900 })));
    t.eq("recovery ends at ssthresh", notes(&e, pc1).into_iter().find(|n| matches!(n.1, Note::TcpRecoveryEnd { .. })), Some((1_275_792, Note::TcpRecoveryEnd { cwnd: 11_680 })));
    t.eq("all there", (metric(&e, pc2, "discard", "bytes"), metric(&e, pc1, "cli", "done")), (30_000, 1));
    t.eq("counted", e.tcp_stats(pc1).map(|s| s.retrans_segs), Some(1));
    // a longer transfer goes on in congestion avoidance after the recovery: cwnd grows by
    // MSS * MSS / cwnd per ACK
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "discard", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "cli", r#"{"server": "10.0.0.2", "port": 9, "bytes": 100000, "delay_ms": 1}"#) });
    let l1 = link_of(&e, pc1);
    lose_at(&mut e, l1, 1_003_560 + 4 * 12_304);
    e.run_until(100 * SEC, BUDGET);
    // (the ACK that ends the recovery sets ssthresh itself, the next ones add 182 and 179)
    let after: Vec<u32> = notes(&e, pc1).into_iter().skip_while(|n| !matches!(n.1, Note::TcpRecoveryEnd { .. })).filter_map(|n| if let Note::TcpAcked { cwnd, .. } = n.1 { Some(cwnd) } else { None }).take(3).collect();
    t.eq("congestion avoidance", after, vec![11_680, 11_680 + 182, 11_862 + 179]);
    t.eq("all of the long one", metric(&e, pc2, "discard", "bytes"), 100_000);
}

// ---------------------------------------------------------------- reordering and RST rules

fn reorder_rst(t: &mut T) {
    let (mut e, pc1, fake, ok) = fake_lan();
    t.check("fake set-up", ok);
    let srv = Probe::new(vec![(0, Step::Listen(7))]);
    let _ = e.add_service(pc1, "srv", Box::new(srv), true);
    e.run_until(MS, BUDGET);
    // the peer opens: SYN at 1 ms, the SYN-ACK 1444 ns later, its ACK at 2 ms
    t.check("syn", inject(&mut e, fake, pc1, 4000, 7, 1000, 0, SYN, 8192, Some(1460), &[]));
    e.run_until(2 * MS, BUDGET);
    let (iss, _) = linux_accept(SEED, pc1);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1001, iss.wrapping_add(1), ACK, 8192, None, &[]);
    e.run_until(3 * MS, BUDGET);
    let a: Vec<u8> = vec![b'a'; 100];
    let b: Vec<u8> = vec![b'b'; 100];
    // B before A: B is held, the ACK still names A's place
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1101, iss.wrapping_add(1), PSH | ACK, 8192, None, &b);
    e.run_until(4 * MS, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1001, iss.wrapping_add(1), PSH | ACK, 8192, None, &a);
    e.run_until(5 * MS, BUDGET);
    // A again: a duplicate, answered at once
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1001, iss.wrapping_add(1), PSH | ACK, 8192, None, &a);
    e.run_until(6 * MS, BUDGET);
    let acks: Vec<(u64, u8, u32, u16)> = sent(&e, pc1).iter().map(|s| (s.t, s.flags, s.ack, s.wnd)).collect();
    t.eq("acks", acks, vec![(MS + 1444, SYN | ACK, 1001, 64240), (3 * MS + 2948, ACK, 1001, 64240), (4 * MS + 2948, ACK, 1201, 64240), (5 * MS + 2948, ACK, 1201, 64240)]);
    t.eq(
        "reorder notes",
        notes(&e, pc1).into_iter().filter(|n| matches!(n.1, Note::TcpOutOfOrder { .. } | Note::TcpReceived { .. } | Note::TcpGapFilled { .. } | Note::TcpDuplicate { .. })).collect::<Vec<_>>(),
        vec![
            (3 * MS + 2948, Note::TcpOutOfOrder { seq: 1101, bytes: 100 }),
            (4 * MS + 2948, Note::TcpReceived { bytes: 100 }),
            (4 * MS + 2948, Note::TcpGapFilled { bytes: 100 }),
            (5 * MS + 2948, Note::TcpDuplicate { bytes: 100 }),
        ],
    );
    let mut want = a.clone();
    want.extend_from_slice(&b);
    t.eq("in order for the service", probe(&e, pc1, "srv").map(|p| p.got.clone()), Some(want));
    // RFC 5961: a RST or SYN in the window but not at RCV.NXT gets a challenge ACK
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1250, 0, RST, 0, None, &[]);
    e.run_until(7 * MS, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1300, 0, SYN, 8192, None, &[]);
    e.run_until(8 * MS, BUDGET);
    t.eq("challenge acks", sent(&e, pc1).iter().skip(4).map(|s| (s.t, s.flags, s.seq, s.ack)).collect::<Vec<_>>(), vec![(6 * MS + 1444, ACK, iss.wrapping_add(1), 1201), (7 * MS + 1444, ACK, iss.wrapping_add(1), 1201)]);
    t.eq("challenge notes", note_times(&e, pc1, |n| matches!(n, Note::TcpChallenge { .. })), vec![6 * MS + 1444, 7 * MS + 1444]);
    t.check("still there", e.tcp_conns(pc1).iter().any(|r| r.state == TcpState::Established));
    // a segment far outside the window: ACK and drop
    let _ = inject(&mut e, fake, pc1, 4000, 7, 900_000, iss.wrapping_add(1), ACK, 8192, None, b"x");
    e.run_until(9 * MS, BUDGET);
    t.check("unacceptable", notes(&e, pc1).iter().any(|n| n.1 == Note::TcpUnacceptable { seq: 900_000, len: 1 }) && sent(&e, pc1).last().is_some_and(|s| s.flags == ACK && s.ack == 1201));
    // the exact RST resets it, and the service hears why
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1201, 0, RST, 0, None, &[]);
    e.run_until(10 * MS, BUDGET);
    t.eq("reset", probe(&e, pc1, "srv").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpClosed { end: TcpEnd::Reset, .. }))), Some(9 * MS + 1444));
    t.check("gone", e.tcp_conns(pc1).iter().all(|r| r.state == TcpState::Listen));
    t.eq("reset counted", e.tcp_stats(pc1).map(|s| s.estab_resets), Some(1));
    // now only the listening socket is left: an ACK gets a RST with its acknowledgement
    // number (RFC 9293 3.10.7.2); without one it is the same (3.10.7.1)
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1201, 4242, ACK, 8192, None, &[]);
    e.run_until(11 * MS, BUDGET);
    t.eq("rst for a stray ack", sent(&e, pc1).last().map(|s| (s.t, s.flags, s.seq, s.ack, s.wnd)), Some((10 * MS + 1444, RST, 4242, 0, 0)));
    t.check("bad ack noted", notes(&e, pc1).iter().any(|n| n.1 == Note::TcpRstSent { why: RstWhy::BadAck }));
    let _ = inject(&mut e, fake, pc1, 4000, 8, 1201, 4343, ACK, 8192, None, &[]);
    e.run_until(12 * MS, BUDGET);
    t.eq("rst without a listener", sent(&e, pc1).last().map(|s| (s.t, s.flags, s.seq, s.sport)), Some((11 * MS + 1444, RST, 4343, 8)));
    t.check("no connection noted", notes(&e, pc1).iter().any(|n| n.1 == Note::TcpNoConn { port: 8 }));
    // a broken checksum: dropped before TCP looks at it
    let mut f = frame(mac(fake), mac(pc1), ip("10.0.0.9"), ip("10.0.0.1"), 64, true, 1, hdr(4000, 7, 1201, 1, PSH | ACK, 8192, None), b"zz");
    f[34 + 16] ^= 0xFF;
    let _ = e.apply(Action::Inject { dev: fake, port: String::from("eth0"), frame: f });
    e.run_until(13 * MS, BUDGET);
    t.check("bad checksum", notes(&e, pc1).iter().any(|n| n.1 == Note::IpBadChecksum) && e.tcp_stats(pc1).is_some_and(|s| s.in_errs == 1) && sent(&e, pc1).last().is_some_and(|s| s.t == 11 * MS + 1444));
    // sequence numbers across 2^32: the peer starts just below the wrap, its data crosses it,
    // the second segment comes first
    let wrap = 0xFFFF_FFC0u32;
    let _ = inject(&mut e, fake, pc1, 4001, 7, wrap, 0, SYN, 8192, Some(1460), &[]);
    e.run_until(14 * MS, BUDGET);
    let iss2 = sent(&e, pc1).last().map(|s| s.seq).unwrap_or(0);
    let _ = inject(&mut e, fake, pc1, 4001, 7, wrap.wrapping_add(1), iss2.wrapping_add(1), ACK, 8192, None, &[]);
    e.run_until(15 * MS, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4001, 7, wrap.wrapping_add(101), iss2.wrapping_add(1), PSH | ACK, 8192, None, &b);
    e.run_until(16 * MS, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4001, 7, wrap.wrapping_add(1), iss2.wrapping_add(1), PSH | ACK, 8192, None, &a);
    e.run_until(17 * MS, BUDGET);
    t.eq("acks across the wrap", sent(&e, pc1).iter().rev().take(2).map(|s| s.ack).collect::<Vec<_>>(), vec![wrap.wrapping_add(201), wrap.wrapping_add(1)]);
    t.check("wrapped data in order", probe(&e, pc1, "srv").is_some_and(|p| p.got.len() == 400 && p.got[200..300] == a[..] && p.got[300..] == b[..]));
}

// ---------------------------------------------------------------- the zero window

fn zero_window(t: &mut T) {
    // the discard server reads nothing for 3 s and has 4 KiB of buffer: its window closes
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "discard", r#"{"rcvbuf": 4096, "read_after_ms": 3000}"#) });
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "cli", r#"{"server": "10.0.0.2", "port": 9, "bytes": 10000, "delay_ms": 1}"#) });
    e.run_until(100 * SEC, BUDGET);
    let (_, iss, _) = linux_connect(SEED, pc1);
    let (irs, _) = linux_accept(SEED, pc2);
    // 4096 bytes of buffer: 2920 in the SYN-ACK (rounded down to the MSS), then 1460, then 0;
    // half the window is one full segment, so Linux grants a single quick ACK and the ACK that
    // closes the window waits the 40 ms of a delayed ACK
    t.eq("server windows", rel(&e, pc2, irs, iss).iter().take(3).map(|s| (s.0, s.3, s.4)).collect::<Vec<_>>(), vec![(1_001_444, 1, 2920), (1_028_268, 1461, 1460), (41_040_572, 2921, 0)]);
    t.eq("window full", note_times(&e, pc2, |n| *n == Note::TcpZeroWindow), vec![41_040_572]);
    // nothing in flight, data waiting: probes after the RTO, then twice as long each time;
    // Linux probes with an empty segment just below SND.UNA, which is answered with the window
    t.eq("persist", notes(&e, pc1).into_iter().find(|n| matches!(n.1, Note::TcpPersist { .. })), Some((41_042_016, Note::TcpPersist { after: 200 * MS })));
    let probes: Vec<(u64, u32, usize)> = rel(&e, pc1, iss, irs).iter().filter(|s| s.0 > 2 * MS && s.0 < 3 * SEC).map(|s| (s.0, s.2, s.5)).collect();
    t.eq("probes", probes, vec![(241_042_016, 2920, 0), (641_042_016, 2920, 0), (1_441_042_016, 2920, 0)]);
    t.eq("answers", rel(&e, pc2, irs, iss).iter().filter(|s| s.0 > 2 * MS && s.0 < 3 * SEC).map(|s| (s.0, s.4)).collect::<Vec<_>>(), vec![(41_040_572, 0), (241_043_460, 0), (641_043_460, 0), (1_441_043_460, 0)]);
    t.eq("probe notes", note_times(&e, pc1, |n| matches!(n, Note::TcpProbe { .. })), vec![241_042_016, 641_042_016, 1_441_042_016]);
    // at 3 s it reads: the window update brings the rest
    t.eq("window update", notes(&e, pc2).into_iter().find(|n| matches!(n.1, Note::TcpWindowUpdate { .. })), Some((3 * SEC, Note::TcpWindowUpdate { wnd: 2920 })));
    t.eq("update segment", rel(&e, pc2, irs, iss).into_iter().find(|s| s.0 == 3 * SEC).map(|s| (s.1, s.3, s.4)), Some((ACK, 2921, 2920)));
    t.eq("resumed", rel(&e, pc1, iss, irs).into_iter().find(|s| s.0 > 3 * SEC).map(|s| (s.0, s.2, s.5)), Some((3 * SEC + 1444, 2921, 1460)));
    t.eq("everything arrived", (metric(&e, pc2, "discard", "bytes"), metric(&e, pc1, "cli", "done")), (10_000, 1));
    t.check("probes counted", e.tcp_conns(pc1).is_empty() && note_times(&e, pc1, |n| matches!(n, Note::TcpGiveUp { .. })).is_empty());
}

// ---------------------------------------------------------------- simultaneous open and close

fn simultaneous(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let p1 = Probe::new(vec![(MS, Step::Connect { sport: 5001, dst: ip("10.0.0.2"), dport: 5002 }), (2 * MS, Step::Send(100)), (3 * MS, Step::Close)]);
    let p2 = Probe::new(vec![(MS, Step::Connect { sport: 5002, dst: ip("10.0.0.1"), dport: 5001 }), (3 * MS, Step::Close)]);
    let _ = e.add_service(pc1, "p1", Box::new(p1), true);
    let _ = e.add_service(pc2, "p2", Box::new(p2), true);
    e.run_until(100 * SEC, BUDGET);
    // with a fixed port no random port is drawn: the ISN is the first number
    let iss1 = Pcg32::derive(SEED, pc1 as u64).next_u32();
    let iss2 = Pcg32::derive(SEED, pc2 as u64).next_u32();
    // RFC 9293 figure 8: both SYNs cross, both answer with SYN-ACK, both are established
    // without a further ACK
    t.eq("pc1 open", rel(&e, pc1, iss1, iss2).into_iter().take(2).collect::<Vec<_>>(), vec![(MS, SYN, 0, 0, 64240, 0), (MS + 1444, SYN | ACK, 0, 1, 64240, 0)]);
    t.eq("pc2 open", rel(&e, pc2, iss2, iss1).into_iter().take(2).collect::<Vec<_>>(), vec![(MS, SYN, 0, 0, 64240, 0), (MS + 1444, SYN | ACK, 0, 1, 64240, 0)]);
    let want = vec![(MS, TcpState::SynSent), (MS + 1444, TcpState::SynReceived), (MS + 2888, TcpState::Established), (3 * MS, TcpState::FinWait1), (3 * MS + 1444, TcpState::Closing), (3 * MS + 2888, TcpState::TimeWait), (60 * SEC + 3 * MS + 2888, TcpState::Closed)];
    t.eq("pc1 states", states(&e, pc1), want.clone());
    // RFC 9293 figure 13: both FINs cross: CLOSING, then TIME-WAIT on both sides
    t.eq("pc2 states", states(&e, pc2), want);
    t.eq("pc1 close", rel(&e, pc1, iss1, iss2).into_iter().skip(2).collect::<Vec<_>>(), vec![(2 * MS, PSH | ACK, 1, 1, 64240, 100), (3 * MS, FIN | ACK, 101, 1, 64240, 0), (3 * MS + 1444, ACK, 102, 2, 64240, 0)]);
    t.eq("pc2 close", rel(&e, pc2, iss2, iss1).into_iter().skip(2).collect::<Vec<_>>(), vec![(2 * MS + 2948, ACK, 1, 101, 64240, 0), (3 * MS, FIN | ACK, 1, 101, 64240, 0), (3 * MS + 1444, ACK, 2, 102, 64240, 0)]);
    t.eq("data", probe(&e, pc2, "p2").map(|p| p.got.len()), Some(100));
    t.check("both told", [(pc1, "p1"), (pc2, "p2")].iter().all(|&(d, n)| probe(&e, d, n).is_some_and(|p| p.first(|x| matches!(x, SvcEvent::TcpConnected { .. })) == Some(MS + 2888))));
}

// ---------------------------------------------------------------- RST at a closed port

fn refused(t: &mut T) {
    let (mut e, pc1, pc2, _) = lan();
    let cli = Probe::new(vec![(0, Step::Connect { sport: 0, dst: ip("10.0.0.2"), dport: 80 })]);
    let _ = e.add_service(pc1, "cli", Box::new(cli), true);
    e.run_until(SEC, BUDGET);
    let (port, iss, _) = linux_connect(SEED, pc1);
    // RFC 9293 3.10.7.1: <SEQ=0><ACK=SEG.SEQ+SEG.LEN><CTL=RST,ACK>, window 0; Linux sends it
    // with DF and IP identification 0
    let rst = frame(mac(pc2), mac(pc1), ip("10.0.0.2"), ip("10.0.0.1"), 64, true, 0, hdr(80, port, 0, iss.wrapping_add(1), RST | ACK, 0, None), &[]);
    t.eq("rst bytes", e.trace.hops().find(|h| h.from.0 == pc2).map(|h| (h.t_tx, h.bytes.to_vec())), Some((1444, rst)));
    t.eq("noted", notes(&e, pc2).into_iter().filter(|n| n.1.is_tcp()).collect::<Vec<_>>(), vec![(1444, Note::TcpNoListener { port: 80 })]);
    t.eq("described", e.trace.events().find(|x| x.dev == pc2 && matches!(x.note, Note::TcpNoListener { .. })).cloned().map(|x| e.describe(&x)), Some(String::from("TCP an Port 80: niemand lauscht, Antwort RST.")));
    t.eq("refused", probe(&e, pc1, "cli").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpClosed { end: TcpEnd::Refused, .. }))), Some(2888));
    t.eq(
        "client notes",
        notes(&e, pc1).into_iter().filter(|n| n.1.is_tcp()).collect::<Vec<_>>(),
        vec![
            (0, Note::TcpStateChange { port, peer: ip("10.0.0.2"), pport: 80, from: TcpState::Closed, to: TcpState::SynSent }),
            (2888, Note::TcpReset { refused: true }),
            (2888, Note::TcpStateChange { port, peer: ip("10.0.0.2"), pport: 80, from: TcpState::SynSent, to: TcpState::Closed }),
        ],
    );
    t.eq("counted", (e.tcp_stats(pc2).map(|s| s.out_rsts), e.tcp_stats(pc1).map(|s| s.attempt_fails)), (Some(1), Some(1)));
    t.eq("errno", TcpEnd::Refused.errno(), Some("Connection refused"));
    // the test client says so too
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "c2", r#"{"server": "10.0.0.2", "port": 81, "bytes": 10}"#) });
    e.run_until(2 * SEC, BUDGET);
    t.eq("client failed", (metric(&e, pc1, "c2", "failed"), metric(&e, pc1, "c2", "done")), (1, 0));
    t.check("client status", e.services(pc1).iter().any(|s| s.name == "c2" && s.status.contains("abgelehnt")));
    // "close": the client closes after its last write, the kernel delivers the rest; "abort":
    // a RST right after the data, which the server hears as a reset
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "discard", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "c", r#"{"server": "10.0.0.2", "port": 9, "bytes": 50000, "close": "close"}"#) });
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "a", r#"{"server": "10.0.0.2", "port": 9, "bytes": 10, "close": "abort", "delay_ms": 500}"#) });
    e.run_until(2 * SEC, BUDGET);
    t.eq("close mode", (metric(&e, pc2, "discard", "bytes"), metric(&e, pc1, "c", "done")), (50_010, 1));
    t.eq("abort mode", (metric(&e, pc2, "discard", "resets"), notes(&e, pc1).iter().filter(|n| n.1 == Note::TcpRstSent { why: RstWhy::Abort }).count()), (1, 1));
}

// ---------------------------------------------------------------- TIME-WAIT

fn time_wait(t: &mut T) {
    let (mut e, pc1, fake, _) = fake_lan();
    let srv = Probe::new(vec![(0, Step::Listen(7)), (3 * MS, Step::Close)]);
    let _ = e.add_service(pc1, "srv", Box::new(srv), true);
    e.run_until(MS, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1000, 0, SYN, 8192, Some(1460), &[]);
    e.run_until(2 * MS, BUDGET);
    let (iss, _) = linux_accept(SEED, pc1);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1001, iss.wrapping_add(1), ACK, 8192, None, &[]);
    // PC1 closes at 3 ms (FIN-WAIT-1), the peer's FIN-ACK at 4 ms: TIME-WAIT
    e.run_until(4 * MS, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1001, iss.wrapping_add(2), FIN | ACK, 8192, None, &[]);
    e.run_until(5 * MS, BUDGET);
    t.eq("close", sent(&e, pc1).iter().skip(1).map(|s| (s.t, s.flags, s.seq.wrapping_sub(iss), s.ack)).collect::<Vec<_>>(), vec![(3 * MS, FIN | ACK, 1, 1001), (4 * MS + 1444, ACK, 2, 1002)]);
    let tw = |e: &Engine| e.tcp_conns(pc1).into_iter().find(|r| r.state == TcpState::TimeWait);
    t.eq("time-wait timer", tw(&e).map(|r| (r.owner, r.timer)), Some((None, Some((TcpTimer::TimeWait, 60 * SEC - (MS - 1444))))));
    // the FIN again (our ACK "was lost"): ACK again, and TIME-WAIT starts over
    e.run_until(10 * SEC, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1001, iss.wrapping_add(2), FIN | ACK, 8192, None, &[]);
    e.run_until(10 * SEC + 2 * MS, BUDGET);
    t.eq("fin acked again", sent(&e, pc1).last().map(|s| (s.t, s.flags, s.ack)), Some((10 * SEC + 1444, ACK, 1002)));
    e.run_until(70 * SEC, BUDGET);
    t.check("still waiting at 70 s", tw(&e).is_some());
    e.run_until(70 * SEC + 1444, BUDGET);
    t.check("over 60 s after the repeated fin", tw(&e).is_none());
    t.eq("closed", states(&e, pc1).last().copied(), Some((70 * SEC + 1444, TcpState::Closed)));

    // a new SYN of the same pair in TIME-WAIT, beyond what came before, opens the connection
    // again at once; its ISN lies above everything the old one used (RFC 1122 4.2.2.13)
    let (mut e, pc1, fake, _) = fake_lan();
    let srv = Probe::new(vec![(0, Step::Listen(7)), (3 * MS, Step::Close)]);
    let _ = e.add_service(pc1, "srv", Box::new(srv), true);
    e.run_until(MS, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1000, 0, SYN, 8192, Some(1460), &[]);
    e.run_until(2 * MS, BUDGET);
    let (iss, _) = linux_accept(SEED, pc1);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1001, iss.wrapping_add(1), ACK, 8192, None, &[]);
    e.run_until(4 * MS, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1001, iss.wrapping_add(2), FIN | ACK, 8192, None, &[]);
    e.run_until(SEC, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 50_000, 0, SYN, 8192, Some(1460), &[]);
    e.run_until(SEC + MS, BUDGET);
    t.eq("syn-ack of the new incarnation", sent(&e, pc1).last().map(|s| (s.t, s.flags, s.seq, s.ack)), Some((SEC + 1444, SYN | ACK, iss.wrapping_add(2 + 65_537), 50_001)));
    t.eq("states", states(&e, pc1).into_iter().skip(6).collect::<Vec<_>>(), vec![(SEC + 1444, TcpState::Closed), (SEC + 1444, TcpState::SynReceived)]);
    // an exact RST ends TIME-WAIT at once (RFC 9293; Linux with tcp_rfc1337 0)
    let (mut e, pc1, fake, _) = fake_lan();
    let srv = Probe::new(vec![(0, Step::Listen(7)), (3 * MS, Step::Close)]);
    let _ = e.add_service(pc1, "srv", Box::new(srv), true);
    e.run_until(MS, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1000, 0, SYN, 8192, Some(1460), &[]);
    e.run_until(2 * MS, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1001, iss.wrapping_add(1), ACK, 8192, None, &[]);
    e.run_until(4 * MS, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1001, iss.wrapping_add(2), FIN | ACK, 8192, None, &[]);
    e.run_until(5 * MS, BUDGET);
    let _ = inject(&mut e, fake, pc1, 4000, 7, 1002, 0, RST, 0, None, &[]);
    e.run_until(6 * MS, BUDGET);
    t.eq("rst in time-wait", states(&e, pc1).into_iter().skip(5).collect::<Vec<_>>(), vec![(4 * MS + 1444, TcpState::TimeWait), (5 * MS + 1444, TcpState::Closed)]);
    t.check("nothing left but the listener", e.tcp_conns(pc1).iter().all(|r| r.state == TcpState::Listen));
    // the IOS-like system waits 60 s as well
    t.eq("ios time-wait", jfn_sim::TcpCfg::ios().time_wait, 60 * SEC);
}

// ---------------------------------------------------------------- Nagle and delayed ACKs

fn nagle_delack(t: &mut T) {
    // ten writes of 100 bytes at once: the first goes, the rest wait for its ACK (Minshall:
    // a small segment is unacknowledged), then 900 bytes in one segment
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "discard", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "cli", r#"{"server": "10.0.0.2", "port": 9, "bytes": 1000, "chunk": 100, "delay_ms": 1, "close": "keep"}"#) });
    e.run_until(SEC, BUDGET);
    let (_, iss, _) = linux_connect(SEED, pc1);
    let data = |e: &Engine| sent(e, pc1).iter().filter(|s| s.len > 0).map(|s| (s.t, s.seq.wrapping_sub(iss), s.len)).collect::<Vec<_>>();
    t.eq("nagle", data(&e), vec![(1_003_560, 1, 100), (1_007_952, 101, 900)]);
    t.eq("nagle note", notes(&e, pc1).into_iter().filter(|n| matches!(n.1, Note::TcpNagle { .. })).collect::<Vec<_>>(), vec![(1_002_888, Note::TcpNagle { bytes: 100 })]);
    // TCP_NODELAY: every write a segment
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "discard", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "cli", r#"{"server": "10.0.0.2", "port": 9, "bytes": 1000, "chunk": 100, "delay_ms": 1, "close": "keep", "nodelay": true}"#) });
    e.run_until(SEC, BUDGET);
    t.eq("nodelay", data(&e).iter().map(|d| d.2).collect::<Vec<_>>(), vec![100; 10]);
    // back to back on the cable: 1424 ns per 154-byte frame
    t.eq("nodelay times", data(&e).iter().map(|d| d.0).take(3).collect::<Vec<_>>(), vec![1_003_560, 1_004_984, 1_006_408]);
    // delayed ACKs: the ACK of a FIN waits 40 ms on Linux (ping-pong), so that the own FIN
    // can carry it; here the service does not close, so the delayed ACK goes alone
    let (mut e, pc1, pc2, _) = lan();
    let srv = Probe::new(vec![(0, Step::Listen(7))]);
    let cli = Probe::new(vec![(MS, Step::Connect { sport: 0, dst: ip("10.0.0.2"), dport: 7 }), (2 * MS, Step::Close)]);
    let _ = e.add_service(pc2, "srv", Box::new(srv), true);
    let _ = e.add_service(pc1, "cli", Box::new(cli), true);
    e.run_until(SEC, BUDGET);
    t.eq("fin delayed", note_times(&e, pc2, |n| matches!(n, Note::TcpAckDelayed { .. })), vec![2 * MS + 1444]);
    t.eq("delayed ack", note_times(&e, pc2, |n| *n == Note::TcpDelayedAck), vec![42 * MS + 1444]);
    t.eq("its segment", sent(&e, pc2).last().map(|s| (s.t, s.flags)), Some((42 * MS + 1444, ACK)));
    t.eq("delayed counted", e.tcp_stats(pc2).map(|s| s.delayed_acks), Some(1));
    // a closed socket in FIN-WAIT-2 gives up after 60 s (tcp_fin_timeout)
    t.eq("fin-wait-2 timeout", states(&e, pc1).into_iter().skip(3).collect::<Vec<_>>(), vec![(42 * MS + 2888, TcpState::FinWait2)]);
    e.run_until(61 * SEC, BUDGET);
    t.eq("fin-wait-2 over", states(&e, pc1).last().copied(), Some((60 * SEC + 42 * MS + 2888, TcpState::Closed)));
}

// ---------------------------------------------------------------- keepalive

fn keepalive(t: &mut T) {
    // the server goes away silently: probes after 7200 s idle, every 75 s, nine of them, then
    // a RST and the end
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "discard", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "cli", r#"{"server": "10.0.0.2", "port": 9, "bytes": 10, "delay_ms": 1, "close": "keep", "keepalive": true}"#) });
    e.run_until(7000 * SEC, BUDGET);
    // the timer was set when the connection opened; it fires then, finds the last segment
    // younger than 7200 s and waits for the rest
    t.eq("timer", e.tcp_conns(pc1).first().map(|r| r.timer), Some(Some((TcpTimer::Keepalive, 200 * SEC + 1_002_888))));
    let _ = e.apply(Action::Power { dev: pc2, on: false });
    e.run_until(9000 * SEC, BUDGET);
    let (_, iss, _) = linux_connect(SEED, pc1);
    let probes: Vec<(u64, u8, u32)> = sent(&e, pc1).iter().filter(|s| s.t > SEC).map(|s| (s.t - 1_006_512, s.flags, s.seq.wrapping_sub(iss))).collect();
    let mut want: Vec<(u64, u8, u32)> = (0..9).map(|i| ((7200 + 75 * i) * SEC, ACK, 10)).collect();
    want.push((7875 * SEC, RST | ACK, 11));
    t.eq("probes and reset", probes, want);
    t.eq("probe notes", note_times(&e, pc1, |n| matches!(n, Note::TcpKeepalive { .. })).len(), 9);
    t.eq("ended", (metric(&e, pc1, "cli", "failed"), states(&e, pc1).last().copied()), (1, Some((7875 * SEC + 1_006_512, TcpState::Closed))));
    t.check("reset noted", notes(&e, pc1).iter().any(|n| n.1 == Note::TcpRstSent { why: RstWhy::Keepalive }));
    // an answered probe: the peer is there, the connection stays and waits 7200 s again
    let (mut e, pc1, pc2, _) = lan();
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "discard", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "cli", r#"{"server": "10.0.0.2", "port": 9, "bytes": 10, "delay_ms": 1, "close": "keep", "keepalive": true}"#) });
    e.run_until(7300 * SEC, BUDGET);
    t.eq("answered", sent(&e, pc2).iter().filter(|s| s.t > SEC).map(|s| (s.t - 1_006_512, s.flags)).collect::<Vec<_>>(), vec![(7200 * SEC + 1444, ACK)]);
    t.eq("next probe", e.tcp_conns(pc1).first().map(|r| r.timer), Some(Some((TcpTimer::Keepalive, 7100 * SEC + 2888 + 1_006_512))));
}

// ---------------------------------------------------------------- backlog and closing

fn backlog_close(t: &mut T) {
    // backlog 1: Linux lets backlog + 1 connections wait; the third SYN is dropped until the
    // service accepts
    let (mut e, pc1, pc2, _) = lan();
    let mut srv = Probe::new(vec![(0, Step::Backlog(7, 1)), (1500 * MS, Step::Accept)]);
    srv.accept = false;
    let _ = e.add_service(pc2, "srv", Box::new(srv), true);
    for (i, name) in ["c1", "c2", "c3"].iter().enumerate() {
        let c = Probe::new(vec![(MS * (i as u64 + 1), Step::Connect { sport: 6000 + i as u16, dst: ip("10.0.0.2"), dport: 7 })]);
        let _ = e.add_service(pc1, name, Box::new(c), true);
    }
    e.run_until(5 * SEC, BUDGET);
    t.eq("dropped syns", note_times(&e, pc2, |n| *n == Note::TcpSynDropped { port: 7 }), vec![3 * MS + 1444, SEC + 3 * MS + 1444]);
    t.eq("listen drops", e.tcp_stats(pc2).map(|s| s.listen_drops), Some(2));
    t.eq("third connects after the accept", probe(&e, pc1, "c3").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpConnected { .. }))), Some(3 * SEC + 3 * MS + 2888));
    // closing with bytes it never read resets the connection (Linux)
    let (mut e, pc1, pc2, _) = lan();
    let mut srv = Probe::new(vec![(0, Step::Listen(7)), (3 * MS, Step::Close)]);
    srv.read = false;
    let cli = Probe::new(vec![(MS, Step::Connect { sport: 0, dst: ip("10.0.0.2"), dport: 7 }), (2 * MS, Step::Send(100))]);
    let _ = e.add_service(pc2, "srv", Box::new(srv), true);
    let _ = e.add_service(pc1, "cli", Box::new(cli), true);
    e.run_until(SEC, BUDGET);
    t.eq("reset at close", sent(&e, pc2).last().map(|s| (s.t, s.flags)), Some((3 * MS, RST | ACK)));
    t.check("why", notes(&e, pc2).iter().any(|n| n.1 == Note::TcpRstSent { why: RstWhy::Unread }));
    t.eq("peer reset", probe(&e, pc1, "cli").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpClosed { end: TcpEnd::Reset, .. }))), Some(3 * MS + 1444));
    // data after the service closed its socket resets it too
    let (mut e, pc1, pc2, _) = lan();
    let srv = Probe::new(vec![(0, Step::Listen(7)), (2 * MS, Step::Close)]);
    let cli = Probe::new(vec![(MS, Step::Connect { sport: 0, dst: ip("10.0.0.2"), dport: 7 }), (3 * MS, Step::Send(100))]);
    let _ = e.add_service(pc2, "srv", Box::new(srv), true);
    let _ = e.add_service(pc1, "cli", Box::new(cli), true);
    e.run_until(SEC, BUDGET);
    t.check("data after close", notes(&e, pc2).iter().any(|n| n.1 == Note::TcpRstSent { why: RstWhy::DataAfterClose }));
    t.eq("client in close-wait reset", probe(&e, pc1, "cli").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpClosed { end: TcpEnd::Reset, .. }))), Some(3 * MS + 4392));
    // the listening socket closes: what waits in its queue is reset
    let (mut e, pc1, pc2, _) = lan();
    let mut srv = Probe::new(vec![(0, Step::Listen(7)), (5 * MS, Step::CloseListener)]);
    srv.accept = false;
    let cli = Probe::new(vec![(MS, Step::Connect { sport: 0, dst: ip("10.0.0.2"), dport: 7 })]);
    let _ = e.add_service(pc2, "srv", Box::new(srv), true);
    let _ = e.add_service(pc1, "cli", Box::new(cli), true);
    e.run_until(SEC, BUDGET);
    t.check("queued reset", notes(&e, pc2).iter().any(|n| n.1 == Note::TcpRstSent { why: RstWhy::ListenerClosed }));
    t.eq("listener gone", e.tcp_conns(pc2).len(), 0);
    // stopping a service closes its sockets with a FIN, as the end of a process does
    let (mut e, pc1, pc2, _) = lan();
    let srv = Probe::new(vec![(0, Step::Listen(7))]);
    let cli = Probe::new(vec![(MS, Step::Connect { sport: 0, dst: ip("10.0.0.2"), dport: 7 })]);
    let _ = e.add_service(pc2, "srv", Box::new(srv), true);
    let _ = e.add_service(pc1, "cli", Box::new(cli), true);
    e.run_until(10 * MS, BUDGET);
    let _ = e.apply(Action::ServiceRun { dev: pc2, name: String::from("srv"), on: false });
    e.run_until(20 * MS, BUDGET);
    t.eq("fin at stop", sent(&e, pc2).last().map(|s| (s.t, s.flags)), Some((10 * MS, FIN | ACK)));
    t.eq("peer told", probe(&e, pc1, "cli").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpEof { .. }))), Some(10 * MS + 1444));
    // power off: the connections are gone without a word
    let _ = e.apply(Action::Power { dev: pc2, on: false });
    let _ = e.apply(Action::Power { dev: pc2, on: true });
    t.check("gone with the power", e.tcp_conns(pc2).is_empty());
    t.check("still open on the client", probe(&e, pc1, "cli").and_then(|p| p.sock).is_some());
}

// ---------------------------------------------------------------- IOS-like values

fn ios(t: &mut T) {
    // PC1 (10.0.0.1) - Gi0/0/0 R1 Gi0/0/1 - Gi0/0/0 R2 (10.0.1.1): R2 serves echo; PC1 lies
    // beyond R2's connected subnets
    let mut p = Project::new("ios", SEED);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 100, 0).unwrap_or(0);
    let r2 = p.add_device("jf-r4", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(r1, "gi0/0/0"), None);
    let _ = p.connect(&pr(r1, "gi0/0/1"), &pr(r2, "gi0/0/0"), None);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let s = String::from;
    let ok = [
        e.apply(Action::IfAddr { dev: pc1, iface: s("eth0"), prefix: pf("10.0.0.1/24"), add: true }),
        e.apply(Action::Route { dev: pc1, prefix: pf("0.0.0.0/0"), via: Some(ip("10.0.0.254")), iface: None, ad: 1, add: true }),
        e.apply(Action::IfAddr { dev: r1, iface: s("Gi0/0/0"), prefix: pf("10.0.0.254/24"), add: true }),
        e.apply(Action::IfAddr { dev: r1, iface: s("Gi0/0/1"), prefix: pf("10.0.1.254/24"), add: true }),
        e.apply(Action::IfUp { dev: r1, iface: s("Gi0/0/0"), up: true }),
        e.apply(Action::IfUp { dev: r1, iface: s("Gi0/0/1"), up: true }),
        e.apply(Action::IfAddr { dev: r2, iface: s("Gi0/0/0"), prefix: pf("10.0.1.1/24"), add: true }),
        e.apply(Action::IfUp { dev: r2, iface: s("Gi0/0/0"), up: true }),
        e.apply(Action::Route { dev: r2, prefix: pf("0.0.0.0/0"), via: Some(ip("10.0.1.254")), iface: None, ad: 1, add: true }),
        e.apply(Action::StaticArp { dev: pc1, ip: ip("10.0.0.254"), mac: Some(port_mac(r1, 0)) }),
        e.apply(Action::StaticArp { dev: r1, ip: ip("10.0.0.1"), mac: Some(mac(pc1)) }),
        e.apply(Action::StaticArp { dev: r1, ip: ip("10.0.1.1"), mac: Some(port_mac(r2, 0)) }),
        e.apply(Action::StaticArp { dev: r2, ip: ip("10.0.1.254"), mac: Some(port_mac(r1, 1)) }),
        e.apply(Action::Service { dev: r2, def: def("tcp-echo", "echo", "{}") }),
        e.apply(Action::Service { dev: pc1, def: def("tcp-client", "cli", r#"{"server": "10.0.1.1", "port": 7, "bytes": 2000, "delay_ms": 1}"#) }),
    ];
    t.check("ios set-up", ok.iter().all(|r| r.is_ok()));
    e.run_until(10 * SEC, BUDGET);
    // the SYN-ACK: MSS 536 towards a remote network, a window of 4128 bytes, TTL 255, no DF
    let synack = e.trace.hops().find(|h| h.from.0 == r2).map(|h| h.bytes.to_vec()).unwrap_or_default();
    let v = Ipv4View::parse(synack.get(14..).unwrap_or(&[]));
    t.eq("ios ip header", v.map(|v| (v.ttl(), v.df())), Some((255, false)));
    t.eq("ios syn-ack", sent(&e, r2).first().map(|s| (s.flags, s.wnd, s.mss)), Some((SYN | ACK, 4128, Some(536))));
    // PC1 sends segments of 536 bytes then; IOS holds a lone ACK up to 200 ms
    t.eq("segment sizes", sent(&e, pc1).iter().filter(|s| s.len > 0).map(|s| s.len).collect::<Vec<_>>(), vec![536, 536, 536, 392]);
    t.check("ack hold 200 ms", notes(&e, r2).iter().any(|n| n.1 == Note::TcpAckDelayed { delay: 200 * MS }));
    t.eq("echoed", (metric(&e, r2, "echo", "bytes_out"), metric(&e, pc1, "cli", "received"), metric(&e, pc1, "cli", "done")), (2000, 2000, 1));
    t.eq("ios values", e.tcp_cfg(r2).map(|c| (c.rto_min, c.delack, c.rcvbuf, c.syn_wait, c.minshall, c.probe_data)), Some((300 * MS, 200 * MS, 4128, Some(30 * SEC), false, true)));

    // an IOS sender probes a zero window with one byte of new data: R3 joins the LAN of the
    // two PCs and sends to PC2's discard server, which never reads
    let (mut e, _, pc2, _) = lan();
    let mut d = jfn_model::project::Device::default();
    d.id = 50;
    d.name = s("R50");
    d.model = s("jf-r4");
    let sw = e.devices().find(|&x| e.node(x).is_some_and(|n| n.model.id == "jf-s24p")).unwrap_or(0);
    let link = jfn_model::Link { id: 51, a: pr(50, "GigabitEthernet0/0/0"), b: pr(sw, "GigabitEthernet1/0/3"), cable: jfn_model::CableKind::CopperStraight, kbps: 1_000_000, delay_ns: 50, ..jfn_model::Link::default() };
    let ok = [
        e.apply(Action::AddDevice(d)),
        e.apply(Action::Connect(link)),
        e.apply(Action::IfAddr { dev: 50, iface: s("Gi0/0/0"), prefix: pf("10.0.0.50/24"), add: true }),
        e.apply(Action::IfUp { dev: 50, iface: s("Gi0/0/0"), up: true }),
        e.apply(Action::StaticArp { dev: 50, ip: ip("10.0.0.2"), mac: Some(mac(pc2)) }),
        e.apply(Action::StaticArp { dev: pc2, ip: ip("10.0.0.50"), mac: Some(port_mac(50, 0)) }),
        e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "discard", r#"{"rcvbuf": 2920, "read": false}"#) }),
        e.apply(Action::Service { dev: 50, def: def("tcp-client", "cli", r#"{"server": "10.0.0.2", "port": 9, "bytes": 5000, "delay_ms": 1, "close": "keep"}"#) }),
    ];
    t.check("router on the lan", ok.iter().all(|r| r.is_ok()));
    e.run_until(5 * SEC, BUDGET);
    let probes: Vec<(usize, u8)> = sent(&e, 50).iter().filter(|s| s.t > 100 * MS).map(|s| (s.len, s.flags)).collect();
    t.check("one-byte probes", probes.len() >= 3 && probes.iter().all(|p| *p == (1, ACK | PSH)));
    t.check("probes refused", sent(&e, pc2).iter().filter(|s| s.t > 100 * MS).all(|s| s.wnd == 0 && s.len == 0));
    // the same with a server that starts reading after 2 s: the window opens, the probe byte
    // goes again with the data, everything arrives once
    let _ = e.apply(Action::RemoveService { dev: 50, name: s("cli") });
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "d2", r#"{"port": 19, "rcvbuf": 2920, "read_after_ms": 2000}"#) });
    let _ = e.apply(Action::Service { dev: 50, def: def("tcp-client", "c2", r#"{"server": "10.0.0.2", "port": 19, "bytes": 5000, "delay_ms": 1}"#) });
    e.run_until(20 * SEC, BUDGET);
    t.eq("ios probe, then the window", (metric(&e, pc2, "d2", "bytes"), metric(&e, 50, "c2", "done")), (5000, 1));
}

// ---------------------------------------------------------------- ICMP errors and path MTU

fn icmp_pmtu(t: &mut T) {
    // PC1 - R1 (IOS) - PC2; R1 has no route to 10.9.0.0/16, and its link to PC2 has an MTU of
    // 1000
    let mut p = Project::new("icmp", SEED);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 100, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(r1, "gi0/0/0"), None);
    let _ = p.connect(&pr(r1, "gi0/0/1"), &pr(pc2, "eth0"), None);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let s = String::from;
    let ok = [
        e.apply(Action::IfAddr { dev: pc1, iface: s("eth0"), prefix: pf("10.0.1.10/24"), add: true }),
        e.apply(Action::Route { dev: pc1, prefix: pf("0.0.0.0/0"), via: Some(ip("10.0.1.1")), iface: None, ad: 1, add: true }),
        e.apply(Action::IfAddr { dev: pc2, iface: s("eth0"), prefix: pf("10.0.2.10/24"), add: true }),
        e.apply(Action::Route { dev: pc2, prefix: pf("0.0.0.0/0"), via: Some(ip("10.0.2.1")), iface: None, ad: 1, add: true }),
        e.apply(Action::IfAddr { dev: r1, iface: s("Gi0/0/0"), prefix: pf("10.0.1.1/24"), add: true }),
        e.apply(Action::IfAddr { dev: r1, iface: s("Gi0/0/1"), prefix: pf("10.0.2.1/24"), add: true }),
        e.apply(Action::IfUp { dev: r1, iface: s("Gi0/0/0"), up: true }),
        e.apply(Action::IfUp { dev: r1, iface: s("Gi0/0/1"), up: true }),
        e.apply(Action::Mtu { dev: r1, iface: s("Gi0/0/1"), mtu: 1000 }),
        e.apply(Action::StaticArp { dev: pc1, ip: ip("10.0.1.1"), mac: Some(port_mac(r1, 0)) }),
        e.apply(Action::StaticArp { dev: r1, ip: ip("10.0.1.10"), mac: Some(mac(pc1)) }),
        e.apply(Action::StaticArp { dev: r1, ip: ip("10.0.2.10"), mac: Some(mac(pc2)) }),
        e.apply(Action::StaticArp { dev: pc2, ip: ip("10.0.2.1"), mac: Some(port_mac(r1, 1)) }),
    ];
    t.check("icmp set-up", ok.iter().all(|r| r.is_ok()));
    // no route at R1: "network unreachable" ends the attempt at once (Linux)
    let cli = Probe::new(vec![(0, Step::Connect { sport: 0, dst: ip("10.9.9.9"), dport: 80 })]);
    let _ = e.add_service(pc1, "cli", Box::new(cli), true);
    e.run_until(SEC, BUDGET);
    // the SYN to R1 (722 ns), its ICMP error back (IOS quotes the header and 8 bytes: 70
    // bytes, 802 ns)
    t.eq("unreachable", probe(&e, pc1, "cli").and_then(|p| p.first(|x| matches!(x, SvcEvent::TcpClosed { end: TcpEnd::Unreachable, .. }))), Some(1524));
    t.eq("noted", note_times(&e, pc1, |n| *n == Note::TcpGiveUp { why: TcpEnd::Unreachable }), vec![1524]);
    // path MTU: 1460-byte segments with DF do not fit through 1000 bytes; the MSS drops to 960
    // and the lost ones go again
    let _ = e.apply(Action::Service { dev: pc2, def: def("tcp-discard", "discard", "{}") });
    let _ = e.apply(Action::Service { dev: pc1, def: def("tcp-client", "c2", r#"{"server": "10.0.2.10", "port": 9, "bytes": 3000, "delay_ms": 1}"#) });
    e.run_until(3 * SEC, BUDGET);
    t.check("mss lowered", notes(&e, pc1).iter().any(|n| n.1 == Note::TcpPmtu { mss: 960 }));
    t.eq("all through", (metric(&e, pc2, "discard", "bytes"), metric(&e, pc1, "c2", "done")), (3000, 1));
    let through: Vec<usize> = e.trace.hops().filter(|h| h.from.0 == r1 && h.to.0 == pc2).filter_map(|h| seg_of(h)).filter(|s| s.len > 0).map(|s| s.len).collect();
    t.check("segments reach pc2 at 960 bytes", !through.is_empty() && through.iter().all(|&l| l <= 960));
}

// ---------------------------------------------------------------- to the device itself

fn loopback(t: &mut T) {
    let (mut e, pc1, _, _) = lan();
    let ok = [
        e.apply(Action::Service { dev: pc1, def: def("tcp-echo", "echo", "{}") }),
        e.apply(Action::Service { dev: pc1, def: def("tcp-client", "cli", r#"{"server": "10.0.0.1", "port": 7, "bytes": 200000, "delay_ms": 1}"#) }),
        e.apply(Action::Service { dev: pc1, def: def("tcp-client", "lo", r#"{"server": "127.0.0.1", "port": 7, "bytes": 1000, "delay_ms": 1}"#) }),
    ];
    t.check("loopback set-up", ok.iter().all(|r| r.is_ok()));
    e.run_until(10 * SEC, BUDGET);
    t.eq("nothing on the cable", e.trace.hops().filter(|h| h.mask & decode::P_TCP != 0).count(), 0);
    t.eq("echoed to itself", [("cli", "received"), ("cli", "done"), ("lo", "received"), ("lo", "done")].map(|(s, m)| metric(&e, pc1, s, m)), [200_000, 1, 1000, 1]);
    t.eq("time-wait pairs", e.tcp_conns(pc1).iter().filter(|r| r.state == TcpState::TimeWait).map(|r| r.mss).collect::<Vec<_>>(), vec![65495, 65495]);
}

// ---------------------------------------------------------------- views

fn views(t: &mut T) {
    t.eq("names", [TcpState::SynReceived.name(), TcpState::SynReceived.ss_name(), TcpState::SynReceived.ios_name(), TcpState::Closed.ss_name(), TcpState::TimeWait.ios_name()], ["SYN-RECEIVED", "SYN-RECV", "SYNRCVD", "UNCONN", "TIMEWAIT"]);
    t.check("layer and mask", Note::TcpZeroWindow.layer() == 4 && Note::TcpZeroWindow.mask() == decode::P_IPV4 | decode::P_TCP && Note::TcpUnacceptable { seq: 1, len: 1 }.is_drop() && !Note::TcpReceived { bytes: 1 }.is_drop());
    let d = |n: Note| n.describe(&|p| format!("p{}", p), &|i| format!("if{}", i));
    t.eq("describe acked", d(Note::TcpAcked { bytes: 1460, cwnd: 16060, rtt: Some(26 * US) }), String::from("1460 Bytes bestätigt, RTT 26 \u{b5}s, cwnd 16060 Bytes."));
    t.eq("describe rto", d(Note::TcpRto { seq: 7, rto: 400 * MS, n: 1 }), String::from("Zeitüberschreitung: Neuübertragung ab Seq 7 (Versuch 1), nächstes RTO 400,000 ms."));
    t.eq("describe fast", d(Note::TcpFastRetransmit { seq: 9, ssthresh: 11680, cwnd: 16060 }), String::from("Drei doppelte Bestätigungen: Fast Retransmit ab Seq 9, ssthresh 11680, cwnd 16060 Bytes."));
    t.eq("describe listen", d(Note::TcpStateChange { port: 7, peer: Ipv4::UNSPECIFIED, pport: 0, from: TcpState::Closed, to: TcpState::Listen }), String::from("TCP Port 7: CLOSED \u{2192} LISTEN."));
    t.eq("describe give-up", d(Note::TcpGiveUp { why: TcpEnd::Timeout }), String::from("Verbindung aufgegeben: keine Antwort mehr (Zeitüberschreitung)."));
    // the counters of a busy connection
    let (e, pc1, _) = lossy_transfer(SEED);
    t.eq("stats", e.tcp_stats(pc1).map(|s| (s.active_opens, s.retrans_segs, s.out_rsts, s.curr_estab)), Some((1, 1, 0, 0)));
    // the project file's services
    let (mut e, pc1, _, _) = lan();
    t.check("bad server", e.apply(Action::Service { dev: pc1, def: def("tcp-client", "x", r#"{"server": "nope"}"#) }).is_err());
    t.check("bad close", e.apply(Action::Service { dev: pc1, def: def("tcp-client", "x", r#"{"server": "10.0.0.2", "close": "later"}"#) }).is_err());
    t.check("bad port", e.apply(Action::Service { dev: pc1, def: def("tcp-echo", "x", r#"{"port": 70000}"#) }).is_err());
    t.check("two on one port", e.apply(Action::Service { dev: pc1, def: def("tcp-echo", "e1", "{}") }).is_ok() && e.apply(Action::Service { dev: pc1, def: def("tcp-discard", "e2", r#"{"port": 7}"#) }).is_ok());
    e.run_until(MS, BUDGET);
    t.check("second listener refused", e.trace.events().any(|x| x.dev == pc1 && x.note == Note::PortInUse { port: 7, svc: String::from("tcp-discard") }));
    t.eq("listening", e.tcp_conns(pc1).iter().map(|r| (r.state, r.local.1, r.owner.clone())).collect::<Vec<_>>(), vec![(TcpState::Listen, 7, Some(String::from("e1")))]);
}

// ---------------------------------------------------------------- determinism

fn determinism(t: &mut T) {
    let run = |seed: u64| {
        let (e, _, _) = lossy_transfer(seed);
        e.trace.hops().map(|h| (h.t_tx, h.from, h.to, h.bytes.to_vec())).collect::<Vec<_>>()
    };
    let a = run(SEED);
    t.check("a busy run", a.len() > 80);
    t.check("same seed, same bytes", a == run(SEED));
    t.check("another seed, other isns", a != run(SEED + 1));
    // a cloned engine goes on alike
    let (mut e, pc1, _) = lossy_transfer(SEED);
    let mut c = e.clone();
    e.run_until(200 * SEC, BUDGET);
    c.run_until(200 * SEC, BUDGET);
    t.check("clone runs alike", e.trace.hops().count() == c.trace.hops().count() && e.tcp_conns(pc1) == c.tcp_conns(pc1));
}
