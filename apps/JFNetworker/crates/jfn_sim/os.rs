// os -- the operating system of a device with IP: UDP sockets, services, the log with its
// syslog hosts, static host names and the device clock
//
//   Os                        per node (Node::os): sockets, services and their inbox, the
//                             resolver (resolver.rs), host names, the log, the clock
//   trait Service             start / on(SvcEvent, &mut OsApi) / stop / metrics / status; the
//                             built-in ones: dhcpc.rs (DHCP client), dhcpd.rs (DHCP server
//                             and relay), dnsd.rs (DNS server), syslogd.rs, ntpd.rs;
//                             status_msg: the status line as a Msg for the app's catalogue
//                             (SvcView::status_msg; the built-in services word it so);
//                             metric_kind and windows: how its numbers become time series
//                             (series.rs)
//   SvcEvent                  Udp(Dgram) | Timer(key) | EchoReply | ArpConflict |
//                             Resolved(lookup) | Link | Ctl; Ip(IpPkt) for a protocol a service
//                             takes without a transport header (OSPF: ospf.rs)
//   OsApi                     what a running service may do: bind / bind_raw / close, send /
//                             send_from / send_raw, timers (four keys per service), notes,
//                             the log, interfaces, addresses and routes, resolver settings
//                             and lookups, ICMP echo (ping check), ARP probes and watches,
//                             the clock
//   Clock                     device time: virtual time + offset with a rate error (drift);
//                             NTP steps or slews it
//   Engine::udp_input         from ip_local: the socket of the port (bound to the receiving
//                             interface before a wildcard one) gets the datagram; false when
//                             nobody listens (port unreachable)
//   Engine::os_raw_udp        from ip_input: a datagram to an address the device does not
//                             have yet reaches a DHCP client's socket on that interface (a
//                             packet socket, as dhclient uses)
//   Engine::add_service / remove_service / set_service_running / services(dev) /
//   service::<T>(dev, name) / metric(dev, svc, name) / log_lines(dev) / clock(dev) /
//   hosts(dev)
//
//   TCP sockets (tcp.rs)      OsApi::tcp_listen / tcp_accept / tcp_connect / tcp_send / tcp_recv /
//                             tcp_shutdown / tcp_close / tcp_abort / tcp_setopts / tcp_info; their
//                             events: SvcEvent::TcpAccept, TcpConnected, TcpData, TcpWritable,
//                             TcpEof, TcpClosed; bind_msg, tcp_listen_msg, tcp_connect_msg and
//                             tcp_connect_from_msg give the refusal as a Msg (the others as
//                             German text)
//   console programs          a program the shell starts (curl, nc: cli/tools.rs) is a service
//                             on the console: OsApi::print writes to it, OsApi::exit ends the
//                             program, typed lines come as SvcEvent::Console; such services are
//                             not in services(); Service::metrics_at gives numbers that depend
//                             on the time (windows of the HTTP services)
//   OsApi::halt / svc_name    a service ends itself (a script after an error: scriptsvc.rs);
//                             its name on the device
//   Engine::rename_service    Action::RenameService: a service goes on under another name
//                             (Service::renamed tells it; its time series follow)
//
// Everything a service sees goes through its node's inbox, which a node timer (SLOT_OS)
// empties at the same virtual time: a service never runs inside another one's call, and a
// datagram a device sends to itself arrives after the sender returned.  Services start that
// way too (after the project is loaded or the power comes back), so they find their cables
// plugged in.  Power off stops every service and closes every socket; what a service keeps
// (leases on disk, bindings in RAM) is up to it.  Timers of services use node timer slots from
// SLOT_SVC on (four per service id; ids are never reused).

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;

use base::rng::Pcg32;
use jfn_model::catalog::{CliStyle, Model};
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::arp::{Arp, OP_REQUEST};
use netpkt::eth::{ET_ARP, ET_IPV4};
use netpkt::icmp::Icmp;
use netpkt::ipv4::{Ipv4Hdr, Ipv4View, PROTO_ICMP, PROTO_UDP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::syslog;
use netpkt::udp::{UdpHdr, UdpView, PORT_SYSLOG};
use netpkt::{Ipv4, Mac, Prefix4};

use crate::action::ActErr;
use crate::ip::{IfKind, Iface, Origin, Route};
use crate::kernel::{Engine, Ns};
use crate::resolver::{Lookup, Owner as LookupOwner, Resolver};
use crate::trace::{HopId, Note};

/// node timer slots of the OS: the inbox and the resolver
pub const SLOT_OS: u32 = 3;
pub const SLOT_RESOLVER: u32 = 4;
/// services: SLOT_SVC + id * SVC_TIMERS + key
pub const SLOT_SVC: u32 = 16;
pub const SVC_TIMERS: u32 = 4;
/// the wall-clock time of virtual time 0: 2026-01-01 00:00:00 UTC (ns since 1970)
pub const EPOCH_UNIX_NS: i64 = 1_767_225_600 * 1_000_000_000;
/// log lines a device keeps
pub const LOG_KEEP: usize = 1000;

/// who owns a socket
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Svc(u32),
    /// a query of a lookup of the resolver
    Resolver(u32),
    /// the sender of syslog messages
    Log,
}

/// a bound UDP port
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sock {
    pub port: u16,
    /// only datagrams that arrive on this interface (SO_BINDTODEVICE)
    pub iface: Option<u16>,
    /// also datagrams to addresses the device does not have yet (a packet socket)
    pub raw: bool,
    pub owner: Owner,
    pub rx: u64,
    pub tx: u64,
}

/// a received UDP datagram
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dgram {
    pub src: Ipv4,
    pub sport: u16,
    pub dst: Ipv4,
    pub dport: u16,
    /// the IPv4 interface it arrived on
    pub iface: u16,
    pub ttl: u8,
    pub data: Vec<u8>,
    /// the frame that brought it (0: made by the device itself)
    pub hop: HopId,
}

/// a received IP packet of a protocol a service takes without a transport header (OSPF,
/// protocol 89: Engine::ospf_rx)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpPkt {
    pub src: Ipv4,
    pub dst: Ipv4,
    pub proto: u8,
    /// the IPv4 interface it arrived on
    pub iface: u16,
    pub ttl: u8,
    /// the IP payload
    pub data: Vec<u8>,
    /// the frame that brought it
    pub hop: HopId,
}

/// a request from outside for a service
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctl {
    /// give the lease back ("dhclient -r", "release dhcp")
    Release,
    /// ask again now ("dhclient", "renew dhcp")
    Renew,
}

/// what happens to a service
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SvcEvent {
    Udp(Dgram),
    /// one of its timers (key 0 to 3)
    Timer(u32),
    /// an answer to one of its ICMP echoes
    EchoReply { from: Ipv4, ident: u16, seq: u16 },
    /// another device uses or probes an address it watches
    ArpConflict { iface: u16, ip: Ipv4, mac: Mac },
    /// a lookup it started is done (OsApi::lookup)
    Resolved(u32),
    /// an interface came up or went down
    Link { iface: u16, up: bool },
    Ctl(Ctl),
    /// TCP (tcp.rs): connections wait in the queue of a listening socket; tcp_accept takes
    /// them until it gives None (the next TcpAccept comes after that)
    TcpAccept { listener: u32, hop: HopId },
    /// a connection of tcp_connect is established
    TcpConnected { sock: u32, hop: HopId },
    /// bytes to read; the next TcpData comes after a tcp_recv
    TcpData { sock: u32, hop: HopId },
    /// the send buffer has room again after tcp_send took less than it was given
    TcpWritable { sock: u32 },
    /// the peer sends nothing more (its FIN came after the bytes still to read)
    TcpEof { sock: u32, hop: HopId },
    /// the connection is over for the service; its socket id is no longer valid
    TcpClosed { sock: u32, end: crate::tcp::TcpEnd },
    /// a console program: what the console gives it
    Console(ConsoleEv),
    /// an IP packet of a protocol the service takes directly (OSPF: ospf.rs)
    Ip(IpPkt),
}

/// what the console gives a program the shell started
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleEv {
    /// a line typed while it runs (without the line end)
    Line(String),
    /// the end of its input (a pipe ran dry: "echo x | nc ...")
    Eof,
}

/// a number a service reports ("offers", "leases", "queries")
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metric {
    pub name: &'static str,
    pub value: i64,
}

impl Metric {
    pub fn new(name: &'static str, value: i64) -> Metric {
        Metric { name, value }
    }
}

/// a program on a device, driven by events
pub trait Service: Send {
    /// "dhcp-client", "dhcp-server", "dhcp-relay", "dns-server", "syslog-server", "ntp"
    fn kind(&self) -> &'static str;
    /// the device runs it: bind sockets, arm timers
    fn start(&mut self, os: &mut OsApi);
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi);
    /// it is stopped (by request, power off or removal); its sockets and timers go by
    /// themselves; with the power off nothing can be sent any more
    fn stop(&mut self, os: &mut OsApi) {
        let _ = os;
    }
    fn metrics(&self) -> Vec<Metric>;
    /// one German line for the inspector ("gebunden: 10.0.1.10/24 bis ...")
    fn status(&self) -> String;
    /// the status line as a message for the app's catalogue (the built-in services word it so,
    /// and their status() is its German text); the default does not translate status()
    fn status_msg(&self) -> Msg {
        Msg::plain(self.status())
    }
    fn clone_box(&self) -> Box<dyn Service>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// the numbers at virtual time `now` (windows of the last seconds); the default is
    /// metrics()
    fn metrics_at(&self, now: Ns) -> Vec<Metric> {
        let _ = now;
        self.metrics()
    }
    /// what a metric of metrics_at() is to the time series (series.rs): a counter (its rate
    /// per second), a gauge or none; the default goes by the name (series::metric_kind_of)
    fn metric_kind(&self, name: &str) -> crate::series::MetricKind {
        crate::series::metric_kind_of(name)
    }
    /// the latency windows of the time series (series.rs): what each interval of them gives
    /// becomes the series of the names they list, in place of the metrics of those names
    fn windows(&self) -> Vec<crate::series::LatWindow<'_>> {
        Vec::new()
    }
    /// it got another name (Engine::rename_service) and goes on; one that keeps a copy of its
    /// name (a script tags its log lines with it) takes the new one
    fn renamed(&mut self, name: &str) {
        let _ = name;
    }
}

impl Clone for Box<dyn Service> {
    fn clone(&self) -> Box<dyn Service> {
        self.clone_box()
    }
}

impl fmt::Debug for dyn Service {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Service({}: {})", self.kind(), self.status())
    }
}

/// a service on a device
#[derive(Clone, Debug)]
pub struct SvcSlot {
    pub id: u32,
    pub name: String,
    pub kind: &'static str,
    /// it should run (not disabled)
    pub enabled: bool,
    pub running: bool,
    /// when it last started
    pub since: Ns,
    pub(crate) svc: Option<Box<dyn Service>>,
    /// a program on the console (curl, nc), not a configured service
    pub console: bool,
    /// the console program ended with this exit status
    pub exit: Option<i32>,
}

/// a service as the inspector shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvcView {
    pub name: String,
    pub kind: &'static str,
    pub enabled: bool,
    pub running: bool,
    pub since: Ns,
    pub status: String,
    pub metrics: Vec<Metric>,
    /// the status as a message for the app's catalogue (status is its German text)
    pub status_msg: Msg,
}

/// work for the inbox
#[derive(Clone, Debug)]
pub(crate) enum Job {
    Start(u32),
    Ev(u32, SvcEvent),
    Resolver(u32, Dgram),
}

/// a line of the device's log
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub t: Ns,
    /// the device clock (ns since 1970)
    pub clock: i64,
    pub seq: u32,
    pub fac: u8,
    pub sev: u8,
    /// "dhclient", "%DHCPD-4-PING_CONFLICT"
    pub tag: String,
    pub text: String,
}

/// the log and where it goes
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogCfg {
    pub lines: VecDeque<LogLine>,
    /// syslog servers ("logging host", "*.* @host")
    pub hosts: Vec<Ipv4>,
    /// the highest severity that goes to the hosts ("logging trap": IOS 6, Linux 7)
    pub trap: u8,
    seq: u32,
    port: Option<u16>,
}

/// a device clock: virtual time + offset, with a rate error
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clock {
    /// device time (ns since 1970) at virtual time `at`
    base: i64,
    at: Ns,
    /// the rate error of the oscillator in ppb (positive: runs fast)
    pub drift_ppb: i64,
    /// the rate correction NTP applies, ppb
    pub freq_ppb: i64,
    slew_ppb: i64,
    slew_until: Ns,
    /// set by NTP or by hand
    pub synced: bool,
    pub stratum: u8,
    /// the NTP server it follows
    pub source: Option<Ipv4>,
    /// when NTP last corrected it (virtual time)
    pub updated: Option<Ns>,
    /// root delay and dispersion of the time it has (ns)
    pub root_delay: Ns,
}

fn ppb_of(dt: i64, ppb: i64) -> i64 {
    ((dt as i128 * ppb as i128) / 1_000_000_000) as i64
}

/// the most a slew may change the rate: 500 ppm (adjtime)
pub const SLEW_MAX_PPB: i64 = 500_000;

impl Clock {
    pub fn new(offset_ns: i64, drift_ppb: i64) -> Clock {
        Clock { base: EPOCH_UNIX_NS + offset_ns, at: 0, drift_ppb, freq_ppb: 0, slew_ppb: 0, slew_until: 0, synced: false, stratum: 16, source: None, updated: None, root_delay: 0 }
    }
    /// the device time at virtual time `t` (ns since 1970)
    pub fn read(&self, t: Ns) -> i64 {
        let dt = t.saturating_sub(self.at) as i64;
        let sdt = self.slew_until.min(t).saturating_sub(self.at) as i64;
        self.base + dt + ppb_of(dt, self.drift_ppb + self.freq_ppb) + ppb_of(sdt, self.slew_ppb)
    }
    fn rebase(&mut self, t: Ns) {
        self.base = self.read(t);
        self.at = t;
        if self.slew_until <= t {
            self.slew_ppb = 0;
        }
    }
    /// jumps by `delta` ns
    pub fn step(&mut self, t: Ns, delta: i64) {
        self.rebase(t);
        self.base += delta;
        self.slew_ppb = 0;
        self.slew_until = t;
    }
    /// removes `offset` ns over `over` by running faster or slower (at most 500 ppm)
    pub fn slew(&mut self, t: Ns, offset: i64, over: Ns) {
        self.rebase(t);
        let over = over.max(1);
        self.slew_ppb = ((offset as i128 * 1_000_000_000) / over as i128).clamp(-SLEW_MAX_PPB as i128, SLEW_MAX_PPB as i128) as i64;
        self.slew_until = t + over;
    }
    /// changes the rate correction from now on
    pub fn set_freq(&mut self, t: Ns, ppb: i64) {
        self.rebase(t);
        self.freq_ppb = ppb.clamp(-SLEW_MAX_PPB, SLEW_MAX_PPB);
    }
    /// sets the time ("clock set", "date -s")
    pub fn set(&mut self, t: Ns, unix_ns: i64) {
        self.rebase(t);
        self.base = unix_ns;
        self.slew_ppb = 0;
        self.slew_until = t;
    }
}

/// the operating system part of a device
#[derive(Clone, Debug)]
pub struct Os {
    /// IOS behaviour (else Linux)
    pub ios: bool,
    pub socks: Vec<Sock>,
    pub svcs: Vec<SvcSlot>,
    next_svc: u32,
    pub(crate) inbox: VecDeque<Job>,
    pub resolver: Resolver,
    /// static host names (/etc/hosts, IOS "ip host")
    pub hosts: Vec<(String, Ipv4)>,
    pub log: LogCfg,
    pub clock: Clock,
    /// ICMP echo identifiers of services
    pub(crate) echo: Vec<(u16, u32)>,
    /// (interface, address, service) watched for conflicts
    pub(crate) arp_watch: Vec<(u16, Ipv4, u32)>,
    /// the range ephemeral ports come from (Linux 32768-60999, IOS 49152-65535)
    pub ephemeral: (u16, u16),
    /// TCP: connections, listening sockets, the values of the system (tcp.rs)
    pub tcp: crate::tcp::TcpStack,
    /// the disk: access times and capacity for services that store data (disk.rs)
    pub disk: crate::disk::Disk,
}

impl Os {
    /// the OS of a device of `model`; its clock is a little off and drifts (from the seed)
    pub fn new(model: &Model, dev: u32, seed: u64) -> Os {
        let ios = model.cli == CliStyle::Ios;
        let mut g = Pcg32::derive(seed ^ 0x434C_4F43, dev as u64);
        let offset = g.range(-2000, 2000) * 1_000_000;
        let drift = g.range(-30_000, 30_000);
        Os {
            ios,
            socks: Vec::new(),
            svcs: Vec::new(),
            next_svc: 1,
            inbox: VecDeque::new(),
            resolver: Resolver::new(ios),
            hosts: Vec::new(),
            log: LogCfg { lines: VecDeque::new(), hosts: Vec::new(), trap: if ios { syslog::SEV_INFO } else { syslog::SEV_DEBUG }, seq: 0, port: None },
            clock: Clock::new(offset, drift),
            echo: Vec::new(),
            arp_watch: Vec::new(),
            ephemeral: if ios { (49152, 65535) } else { (32768, 60999) },
            tcp: crate::tcp::TcpStack::new(ios),
            disk: crate::disk::Disk::default(),
        }
    }
    /// a service by name
    pub fn svc(&self, name: &str) -> Option<&SvcSlot> {
        self.svcs.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }
    pub(crate) fn svc_id(&self, name: &str) -> Option<u32> {
        self.svc(name).map(|s| s.id)
    }
    pub(crate) fn running(&self, id: u32) -> bool {
        self.svcs.iter().any(|s| s.id == id && s.running)
    }
    /// a port is free for a new socket on `iface` (None: every interface)
    pub fn port_free(&self, port: u16, iface: Option<u16>) -> bool {
        !self.socks.iter().any(|s| s.port == port && (s.iface.is_none() || iface.is_none() || s.iface == iface))
    }
}

// ---------------------------------------------------------------- the API of services

/// what a running service may do; it exists only during one of its calls
pub struct OsApi<'a> {
    pub(crate) eng: &'a mut Engine,
    pub(crate) ni: usize,
    pub(crate) id: u32,
    /// the frame that led to this call: what the service sends is caused by it
    pub(crate) cause: Option<HopId>,
}

impl OsApi<'_> {
    pub fn now(&self) -> Ns {
        self.eng.now
    }
    pub fn dev(&self) -> u32 {
        self.eng.dev_of(self.ni)
    }
    /// the device name
    pub fn hostname(&self) -> String {
        match self.eng.nodes.get(self.ni) {
            Some(Some(n)) => n.name.clone(),
            _ => String::new(),
        }
    }
    /// the device behaves as IOS (else Linux)
    pub fn ios(&self) -> bool {
        self.eng.os(self.ni).is_some_and(|o| o.ios)
    }
    pub fn powered(&self) -> bool {
        matches!(self.eng.nodes.get(self.ni), Some(Some(n)) if n.power)
    }
    /// the device's random numbers
    pub fn rand(&mut self) -> u32 {
        match self.eng.nodes.get_mut(self.ni) {
            Some(Some(n)) => n.rng.next_u32(),
            _ => 0,
        }
    }
    /// a socket on `port` (0: a free ephemeral port), on one interface or all
    pub fn bind(&mut self, port: u16, iface: Option<u16>) -> Result<u16, String> {
        self.bind_msg(port, iface).map_err(|m| m.text())
    }
    /// bind(), the refusal as a message
    pub fn bind_msg(&mut self, port: u16, iface: Option<u16>) -> Result<u16, Msg> {
        self.eng.sock_bind(self.ni, port, iface, false, Owner::Svc(self.id))
    }
    /// a packet socket on one interface: datagrams to `port` arrive even before the device has
    /// an address there
    pub fn bind_raw(&mut self, port: u16, iface: u16) -> Result<u16, String> {
        self.eng.sock_bind(self.ni, port, Some(iface), true, Owner::Svc(self.id)).map_err(|m| m.text())
    }
    pub fn close(&mut self, port: u16) {
        let id = self.id;
        if let Some(os) = self.eng.os_mut(self.ni) {
            os.socks.retain(|s| !(s.port == port && s.owner == Owner::Svc(id)));
        }
    }
    /// sends over the routing table (the source address is the outgoing interface's); false
    /// without a route
    pub fn send(&mut self, sport: u16, dst: Ipv4, dport: u16, data: &[u8]) -> bool {
        self.eng.udp_send(self.ni, None, sport, dst, dport, data, self.cause)
    }
    /// sends over the routing table from a given address
    pub fn send_from(&mut self, src: Ipv4, sport: u16, dst: Ipv4, dport: u16, data: &[u8]) -> bool {
        self.eng.udp_send(self.ni, Some(src), sport, dst, dport, data, self.cause)
    }
    /// sends out of an interface to a MAC address, without routing and ARP (DHCP to a client
    /// without an address, broadcasts on one interface)
        pub fn send_raw(&mut self, iface: u16, mac: Mac, src: Ipv4, sport: u16, dst: Ipv4, dport: u16, data: &[u8]) -> bool {
        self.eng.udp_send_raw(self.ni, iface, mac, src, sport, dst, dport, data, self.cause)
    }
    /// (re)arms timer `key` (0 to 3) to fire at `at`
    pub fn timer(&mut self, key: u32, at: Ns) {
        self.eng.set_timer(self.ni, SLOT_SVC + self.id * SVC_TIMERS + key.min(SVC_TIMERS - 1), at);
    }
    pub fn cancel(&mut self, key: u32) {
        self.eng.cancel_timer(self.ni, SLOT_SVC + self.id * SVC_TIMERS + key.min(SVC_TIMERS - 1));
    }
    /// records a decision
    pub fn note(&mut self, n: Note) {
        let dev = self.dev();
        self.eng.note(dev, self.cause, n);
    }
    /// writes to the device's log (and to its syslog hosts): `tag` is the program
    /// ("dhclient") or the IOS mnemonic ("%DHCPD-4-PING_CONFLICT")
    pub fn log(&mut self, sev: u8, tag: &str, text: &str) {
        self.eng.os_log(self.ni, sev, tag, text, self.cause);
    }
    /// the IPv4 interfaces
    pub fn ifaces(&self) -> &[Iface] {
        self.eng.stack(self.ni).map(|s| s.ifaces.as_slice()).unwrap_or(&[])
    }
    pub fn iface(&self, i: u16) -> Option<&Iface> {
        self.ifaces().get(i as usize).filter(|f| !f.hidden)
    }
    pub fn iface_up(&self, i: u16) -> bool {
        self.eng.iface_up(self.ni, i)
    }
    pub fn iface_by_name(&self, name: &str) -> Option<u16> {
        self.eng.stack(self.ni)?.iface_by_name(name)
    }
    /// the interface that has `ip`
    pub fn iface_of(&self, ip: Ipv4) -> Option<u16> {
        self.ifaces().iter().position(|f| !f.hidden && f.has(ip)).map(|i| i as u16)
    }
    pub fn is_mine(&self, ip: Ipv4) -> bool {
        self.eng.stack(self.ni).is_some_and(|s| s.is_mine(ip))
    }
    /// the address a packet to `dst` would leave with
    pub fn source_for(&self, dst: Ipv4) -> Option<Ipv4> {
        self.eng.source_for(self.ni, dst)
    }
    pub fn add_addr(&mut self, i: u16, p: Prefix4) -> Result<(), String> {
        match self.eng.stack_mut(self.ni) {
            Some(s) => s.add_addr(i, p),
            None => Err(String::from("kein IP")),
        }
    }
    pub fn remove_addr(&mut self, i: u16, p: Prefix4) -> bool {
        self.eng.stack_mut(self.ni).is_some_and(|s| s.remove_addr(i, p))
    }
    pub fn add_route(&mut self, r: Route) {
        if let Some(s) = self.eng.stack_mut(self.ni) {
            s.add_route(r);
        }
    }
    pub fn remove_route(&mut self, prefix: Prefix4, via: Option<Ipv4>) -> bool {
        self.eng.stack_mut(self.ni).is_some_and(|s| s.remove_route(prefix, via))
    }
    /// name servers and search domains learned from DHCP (None: forget them)
    pub fn set_dhcp_dns(&mut self, from: Option<(Vec<Ipv4>, Vec<String>)>) {
        if let Some(os) = self.eng.os_mut(self.ni) {
            os.resolver.set_dhcp(from);
        }
    }
    /// starts a lookup; SvcEvent::Resolved(id) follows
    pub fn resolve(&mut self, name: &str, qtype: u16) -> u32 {
        let id = self.id;
        self.eng.lookup_start(self.ni, name, qtype, LookupOwner::Svc(id))
    }
    pub fn lookup(&self, id: u32) -> Option<&Lookup> {
        self.eng.os(self.ni)?.resolver.lookups.get(&id)
    }
    /// sends an ICMP echo request; replies with this identifier come as EchoReply
    pub fn echo(&mut self, dst: Ipv4, ident: u16, seq: u16) -> bool {
        let id = self.id;
        if let Some(os) = self.eng.os_mut(self.ni)
            && !os.echo.contains(&(ident, id))
        {
            os.echo.push((ident, id));
        }
        self.eng.os_echo(self.ni, dst, ident, seq, self.cause)
    }
    /// stops listening for echo replies with this identifier
    pub fn echo_done(&mut self, ident: u16) {
        let id = self.id;
        if let Some(os) = self.eng.os_mut(self.ni) {
            os.echo.retain(|e| *e != (ident, id));
        }
    }
    /// an ARP probe for `ip` (sender 0.0.0.0, RFC 5227)
    pub fn arp_probe(&mut self, iface: u16, ip: Ipv4) {
        let Some(mac) = self.iface(iface).map(|f| f.mac) else {
            return;
        };
        self.eng.arp_out(self.ni, iface, Arp::probe(mac, ip), self.cause);
    }
    /// a gratuitous ARP for the own `ip`
    pub fn arp_announce(&mut self, iface: u16, ip: Ipv4) {
        let Some(mac) = self.iface(iface).map(|f| f.mac) else {
            return;
        };
        self.eng.arp_out(self.ni, iface, Arp::announce(mac, ip), self.cause);
    }
    /// reports ARP traffic of other devices for `ip` on `iface` as ArpConflict
    pub fn arp_watch(&mut self, iface: u16, ip: Ipv4, on: bool) {
        let id = self.id;
        if let Some(os) = self.eng.os_mut(self.ni) {
            os.arp_watch.retain(|w| *w != (iface, ip, id));
            if on {
                os.arp_watch.push((iface, ip, id));
            }
        }
    }
    /// the device clock (ns since 1970)
    pub fn clock(&self) -> i64 {
        let now = self.eng.now;
        self.eng.os(self.ni).map(|o| o.clock.read(now)).unwrap_or(EPOCH_UNIX_NS + now as i64)
    }
    pub fn clock_ref(&self) -> Option<&Clock> {
        self.eng.os(self.ni).map(|o| &o.clock)
    }
    /// changes the clock (NTP)
    pub fn clock_mut(&mut self) -> Option<&mut Clock> {
        self.eng.os_mut(self.ni).map(|o| &mut o.clock)
    }
}

// ---------------------------------------------------------------- the engine side

/// "Oct 19 12:00:05" of a device clock
fn bsd_stamp(clock: i64) -> String {
    let c = datetime::Civil::from_unix(clock.div_euclid(1_000_000_000));
    syslog::bsd_time(c.month, c.day, c.hour, c.minute, c.second)
}

/// "*Oct 19 12:00:05.123" (the star while the clock is not synchronised)
pub fn ios_stamp(clock: i64, synced: bool) -> String {
    format!("{}{}.{:03}", if synced { "" } else { "*" }, bsd_stamp(clock), clock.rem_euclid(1_000_000_000) / 1_000_000)
}

impl Engine {
    pub(crate) fn os(&self, ni: usize) -> Option<&Os> {
        self.nodes.get(ni)?.as_ref()?.os.as_ref()
    }
    pub(crate) fn os_mut(&mut self, ni: usize) -> Option<&mut Os> {
        self.nodes.get_mut(ni)?.as_mut()?.os.as_mut()
    }

    /// queues work for the inbox and makes sure it is emptied now
    pub(crate) fn os_push(&mut self, ni: usize, job: Job) {
        let now = self.now;
        let powered = matches!(self.nodes.get(ni), Some(Some(n)) if n.power);
        let Some(os) = self.os_mut(ni) else {
            return;
        };
        if !powered {
            return;
        }
        os.inbox.push_back(job);
        self.set_timer(ni, SLOT_OS, now);
    }

    /// a free ephemeral port (random, as Linux and IOS pick them)
    pub(crate) fn ephemeral_port(&mut self, ni: usize) -> u16 {
        let (lo, hi) = self.os(ni).map(|o| o.ephemeral).unwrap_or((49152, 65535));
        for _ in 0..64 {
            let p = match self.nodes.get_mut(ni) {
                Some(Some(n)) => lo + n.rng.below((hi - lo) as u32 + 1) as u16,
                _ => lo,
            };
            if self.os(ni).is_some_and(|o| o.port_free(p, None)) {
                return p;
            }
        }
        (lo..=hi).find(|&p| self.os(ni).is_some_and(|o| o.port_free(p, None))).unwrap_or(0)
    }

    pub(crate) fn sock_bind(&mut self, ni: usize, port: u16, iface: Option<u16>, raw: bool, owner: Owner) -> Result<u16, Msg> {
        let port = if port == 0 { self.ephemeral_port(ni) } else { port };
        let os = self.os_mut(ni).ok_or_else(|| msg!("kein IP"))?;
        if port == 0 || !os.port_free(port, iface) {
            return Err(msg!("Port {} ist schon belegt.", port));
        }
        os.socks.push(Sock { port, iface, raw, owner, rx: 0, tx: 0 });
        Ok(port)
    }

    /// a UDP datagram as an IPv4 packet
    fn udp_packet(&mut self, ni: usize, src: Ipv4, sport: u16, dst: Ipv4, dport: u16, data: &[u8]) -> Option<Vec<u8>> {
        let st = self.stack_mut(ni)?;
        let (ttl, id) = (st.ttl, st.next_id());
        let mut p = Pkt::new(HEADROOM, data);
        UdpHdr::new(sport, dport).push_v4(&mut p, src, dst);
        let mut h = Ipv4Hdr::new(src, dst, PROTO_UDP, ttl);
        h.id = id;
        h.push_onto(&mut p);
        Some(p.into_vec())
    }

    fn count_tx(&mut self, ni: usize, sport: u16) {
        if let Some(s) = self.os_mut(ni).and_then(|o| o.socks.iter_mut().find(|s| s.port == sport)) {
            s.tx += 1;
        }
    }

    /// sends a datagram over the routing table; false without a route
        pub(crate) fn udp_send(&mut self, ni: usize, src: Option<Ipv4>, sport: u16, dst: Ipv4, dport: u16, data: &[u8], cause: Option<HopId>) -> bool {
        let dev = self.dev_of(ni);
        let src = match src.or_else(|| self.source_for(ni, dst)) {
            Some(s) => s,
            None => {
                self.note(dev, cause, Note::IpNoRoute { dst });
                return false;
            }
        };
        let Some(pkt) = self.udp_packet(ni, src, sport, dst, dport, data) else {
            return false;
        };
        self.count_tx(ni, sport);
        self.ip_output(ni, pkt, Origin::Local { job: None }, cause);
        true
    }

    /// sends a datagram out of an interface to a MAC address (no routing, no ARP)
        pub(crate) fn udp_send_raw(&mut self, ni: usize, iface: u16, mac: Mac, src: Ipv4, sport: u16, dst: Ipv4, dport: u16, data: &[u8], cause: Option<HopId>) -> bool {
        let Some(pkt) = self.udp_packet(ni, src, sport, dst, dport, data) else {
            return false;
        };
        self.count_tx(ni, sport);
        self.frame_out(ni, iface, mac, ET_IPV4, &pkt, cause);
        true
    }

    pub(crate) fn arp_out(&mut self, ni: usize, iface: u16, a: Arp, cause: Option<HopId>) {
        let mut b = [0u8; 28];
        a.write(&mut b);
        self.frame_out(ni, iface, Mac::BROADCAST, ET_ARP, &b, cause);
    }

    /// an ICMP echo request of a service (Linux: 56 data bytes, IOS: 72 as its 100-byte ping)
    pub(crate) fn os_echo(&mut self, ni: usize, dst: Ipv4, ident: u16, seq: u16, cause: Option<HopId>) -> bool {
        let Some(src) = self.source_for(ni, dst) else {
            return false;
        };
        let size = if self.os(ni).is_some_and(|o| o.ios) { 72 } else { 56 };
        let data: Vec<u8> = (0..size).map(|i| (i & 0xFF) as u8).collect();
        let Some(st) = self.stack_mut(ni) else {
            return false;
        };
        let (ttl, id) = (st.ttl, st.next_id());
        let mut p = Pkt::new(HEADROOM, &data);
        Icmp::EchoRequest { id: ident, seq }.push_onto(&mut p);
        let mut h = Ipv4Hdr::new(src, dst, PROTO_ICMP, ttl);
        h.id = id;
        h.push_onto(&mut p);
        self.ip_output(ni, p.into_vec(), Origin::Local { job: None }, cause);
        true
    }

    /// an echo reply reached the device: services waiting for its identifier hear of it
    pub(crate) fn os_echo_reply(&mut self, ni: usize, from: Ipv4, ident: u16, seq: u16) {
        let ids: Vec<u32> = match self.os(ni) {
            Some(os) => os.echo.iter().filter(|e| e.0 == ident).map(|e| e.1).collect(),
            None => return,
        };
        for id in ids {
            self.os_push(ni, Job::Ev(id, SvcEvent::EchoReply { from, ident, seq }));
        }
    }

    /// ARP from another device on `iface`: a conflict for services that watch the address
    pub(crate) fn os_arp_seen(&mut self, ni: usize, iface: u16, a: &Arp) {
        let mine = self.stack(ni).and_then(|s| s.ifaces.get(iface as usize)).map(|f| f.mac);
        let hits: Vec<(u32, Ipv4)> = match self.os(ni) {
            Some(os) if !os.arp_watch.is_empty() && Some(a.sha) != mine => os
                .arp_watch
                .iter()
                .filter(|w| w.0 == iface && (a.spa == w.1 || (a.op == OP_REQUEST && a.spa.is_unspecified() && a.tpa == w.1)))
                .map(|w| (w.2, w.1))
                .collect(),
            _ => return,
        };
        for (id, ip) in hits {
            self.os_push(ni, Job::Ev(id, SvcEvent::ArpConflict { iface, ip, mac: a.sha }));
        }
    }

    /// the name of a socket's owner for notes
    fn owner_name(&self, ni: usize, o: Owner) -> String {
        match o {
            Owner::Svc(id) => self.os(ni).and_then(|os| os.svcs.iter().find(|s| s.id == id)).map(|s| s.name.clone()).unwrap_or_default(),
            Owner::Resolver(_) => String::from("Resolver"),
            Owner::Log => String::from("Syslog"),
        }
    }

    /// a UDP datagram for this device; false when nobody listens on its port
    pub(crate) fn udp_input(&mut self, ni: usize, iface: u16, v: &Ipv4View, u: &UdpView, hop: HopId) -> bool {
        let port = u.dst_port();
        let Some(os) = self.os(ni) else {
            return false;
        };
        let pick = os.socks.iter().position(|s| s.port == port && s.iface == Some(iface)).or_else(|| os.socks.iter().position(|s| s.port == port && s.iface.is_none()));
        let Some(si) = pick else {
            return false;
        };
        let owner = os.socks[si].owner;
        let d = Dgram { src: v.src(), sport: u.src_port(), dst: v.dst(), dport: port, iface, ttl: v.ttl(), data: u.payload().to_vec(), hop };
        if let Some(os) = self.os_mut(ni) {
            os.socks[si].rx += 1;
        }
        let dev = self.dev_of(ni);
        let to = self.owner_name(ni, owner);
        self.note(dev, Some(hop), Note::UdpToSocket { port, to });
        match owner {
            Owner::Svc(id) => self.os_push(ni, Job::Ev(id, SvcEvent::Udp(d))),
            Owner::Resolver(l) => self.os_push(ni, Job::Resolver(l, d)),
            Owner::Log => {}
        }
        true
    }

    /// a UDP datagram to an address the device does not have: a packet socket on the
    /// interface takes it (DHCP clients before they have an address)
    pub(crate) fn os_raw_udp(&mut self, ni: usize, iface: u16, v: &Ipv4View, hop: HopId) -> bool {
        if v.proto() != PROTO_UDP || v.is_fragment() {
            return false;
        }
        let Some(u) = UdpView::parse(v.payload()) else {
            return false;
        };
        let raw = self.os(ni).is_some_and(|os| os.socks.iter().any(|s| s.raw && s.iface == Some(iface) && s.port == u.dst_port()));
        if !raw || !u.checksum_ok_v4(v.src(), v.dst()) {
            return false;
        }
        self.udp_input(ni, iface, v, &u, hop)
    }

    /// runs a service's call with the service taken out of its slot
    pub(crate) fn svc_call<F: FnOnce(&mut dyn Service, &mut OsApi)>(&mut self, ni: usize, id: u32, cause: Option<HopId>, f: F) {
        let svc = self.os_mut(ni).and_then(|os| os.svcs.iter_mut().find(|s| s.id == id)).and_then(|s| s.svc.take());
        let Some(mut svc) = svc else {
            return;
        };
        {
            let mut api = OsApi { eng: self, ni, id, cause };
            f(svc.as_mut(), &mut api);
        }
        if let Some(s) = self.os_mut(ni).and_then(|os| os.svcs.iter_mut().find(|s| s.id == id)) {
            s.svc = Some(svc);
        }
    }

    fn svc_start(&mut self, ni: usize, id: u32) {
        let now = self.now;
        match self.os_mut(ni).and_then(|os| os.svcs.iter_mut().find(|s| s.id == id)) {
            Some(s) if s.enabled && !s.running => {
                s.running = true;
                s.since = now;
            }
            _ => return,
        }
        self.svc_call(ni, id, None, |s, api| s.start(api));
    }

    /// stops a running service: its sockets, timers, watches and queued events go
    pub(crate) fn svc_stop(&mut self, ni: usize, id: u32) {
        if !self.os(ni).is_some_and(|o| o.running(id)) {
            return;
        }
        self.svc_call(ni, id, None, |s, api| s.stop(api));
        // its TCP sockets close as when a process ends
        self.tcp_owner_gone(ni, id);
        for k in 0..SVC_TIMERS {
            self.cancel_timer(ni, SLOT_SVC + id * SVC_TIMERS + k);
        }
        if let Some(os) = self.os_mut(ni) {
            os.socks.retain(|s| s.owner != Owner::Svc(id));
            os.echo.retain(|e| e.1 != id);
            os.arp_watch.retain(|w| w.2 != id);
            os.inbox.retain(|j| !matches!(j, Job::Ev(i, _) | Job::Start(i) if *i == id));
            if let Some(s) = os.svcs.iter_mut().find(|s| s.id == id) {
                s.running = false;
            }
        }
    }

    /// node timers from SLOT_OS on
    pub(crate) fn os_timer(&mut self, ni: usize, slot: u32) {
        match slot {
            SLOT_OS => self.os_drain(ni),
            SLOT_RESOLVER => self.resolver_timer(ni),
            crate::tcp::SLOT_TCP => self.tcp_timer(ni),
            crate::nat::SLOT_NAT => self.nat_timer(ni),
            s if s >= SLOT_SVC => {
                let id = (s - SLOT_SVC) / SVC_TIMERS;
                let key = (s - SLOT_SVC) % SVC_TIMERS;
                if self.os(ni).is_some_and(|o| o.running(id)) {
                    self.svc_call(ni, id, None, |svc, api| svc.on(SvcEvent::Timer(key), api));
                }
            }
            _ => {}
        }
    }

    /// works through what was in the inbox (what arrives meanwhile waits for the next round)
    fn os_drain(&mut self, ni: usize) {
        let n = self.os(ni).map(|o| o.inbox.len()).unwrap_or(0);
        for _ in 0..n {
            let Some(job) = self.os_mut(ni).and_then(|o| o.inbox.pop_front()) else {
                break;
            };
            match job {
                Job::Start(id) => self.svc_start(ni, id),
                Job::Ev(id, ev) => {
                    if self.os(ni).is_some_and(|o| o.running(id)) {
                        let cause = match &ev {
                            SvcEvent::Udp(d) => Some(d.hop),
                            SvcEvent::Ip(p) => Some(p.hop).filter(|&h| h != 0),
                            SvcEvent::TcpAccept { hop, .. } | SvcEvent::TcpConnected { hop, .. } | SvcEvent::TcpData { hop, .. } | SvcEvent::TcpEof { hop, .. } => Some(*hop).filter(|&h| h != 0),
                            _ => None,
                        };
                        self.svc_call(ni, id, cause, |s, api| s.on(ev, api));
                    }
                }
                Job::Resolver(l, d) => self.resolver_input(ni, l, d),
            }
        }
    }

    /// the power of a node changed (after the node itself was switched)
    pub(crate) fn os_power(&mut self, ni: usize, on: bool) {
        let Some(os) = self.os(ni) else {
            return;
        };
        if on {
            let ids: Vec<u32> = os.svcs.iter().filter(|s| s.enabled).map(|s| s.id).collect();
            for id in ids {
                self.os_push(ni, Job::Start(id));
            }
            return;
        }
        let ids: Vec<u32> = os.svcs.iter().filter(|s| s.running).map(|s| s.id).collect();
        for id in ids {
            self.svc_stop(ni, id);
        }
        let now = self.now;
        if let Some(os) = self.os_mut(ni) {
            os.socks.clear();
            os.inbox.clear();
            os.echo.clear();
            os.arp_watch.clear();
            os.log.port = None;
            os.resolver.power_off(now);
            os.tcp.power_off();
        }
    }

    /// the link of a port changed: services hear about the interfaces on it
    pub(crate) fn os_link(&mut self, ni: usize, port: u16, up: bool) {
        let Some(st) = self.stack(ni) else {
            return;
        };
        let ifs: Vec<u16> = (0..st.ifaces.len() as u16).filter(|&i| matches!(st.ifaces[i as usize].kind, IfKind::Port(p) | IfKind::Sub { port: p, .. } if p == port)).collect();
        let ids: Vec<u32> = match self.os(ni) {
            Some(os) => os.svcs.iter().filter(|s| s.running).map(|s| s.id).collect(),
            None => return,
        };
        for i in ifs {
            for &id in &ids {
                self.os_push(ni, Job::Ev(id, SvcEvent::Link { iface: i, up }));
            }
        }
    }

    /// a line for the device's log; it goes to the syslog hosts when severe enough
    pub(crate) fn os_log(&mut self, ni: usize, sev: u8, tag: &str, text: &str, cause: Option<HopId>) {
        let now = self.now;
        let (dev, host) = match self.nodes.get(ni) {
            Some(Some(n)) => (n.dev, n.name.clone()),
            _ => return,
        };
        if self.os(ni).is_none() {
            return;
        }
        // the user interface's log sees every line (Report::Log)
        self.cli.reports.push(crate::driver::Report::Log { dev, at: now, sev, tag: String::from(tag), text: String::from(text) });
        let Some(os) = self.os_mut(ni) else {
            return;
        };
        let clock = os.clock.read(now);
        os.log.seq += 1;
        let seq = os.log.seq;
        let fac = if os.ios { syslog::FAC_LOCAL7 } else { syslog::FAC_DAEMON };
        if os.log.lines.len() >= LOG_KEEP {
            os.log.lines.pop_front();
        }
        os.log.lines.push_back(LogLine { t: now, clock, seq, fac, sev, tag: String::from(tag), text: String::from(text) });
        let quiet = sev > os.log.trap || os.log.hosts.is_empty();
        // a console that follows the log (dhclient -v: cli/netsvc.rs)
        self.cli_log_seen(ni);
        if quiet {
            return;
        }
        let Some(os) = self.os_mut(ni) else {
            return;
        };
        let pri = syslog::pri(fac, sev);
        let msg = if os.ios {
            syslog::ios(pri, seq, &ios_stamp(clock, os.clock.synced), &format!("{}: {}", tag, text))
        } else {
            syslog::bsd(pri, &bsd_stamp(clock), &host, tag, text)
        };
        let hosts = os.log.hosts.clone();
        let port = match os.log.port {
            Some(p) => p,
            None => {
                let Ok(p) = self.sock_bind(ni, 0, None, false, Owner::Log) else {
                    return;
                };
                if let Some(os) = self.os_mut(ni) {
                    os.log.port = Some(p);
                }
                p
            }
        };
        for h in hosts {
            if self.udp_send(ni, None, port, h, PORT_SYSLOG, &msg, cause) {
                self.note(dev, cause, Note::SyslogSent { to: h, sev });
            }
        }
    }

    // ---------------------------------------------------------------- services

    /// adds a service to a device (one with the same name is stopped and replaced, and what it
    /// stored on the disk goes with it); it starts when the engine runs next, if `enabled` and
    /// the device is on
    pub fn add_service(&mut self, dev: u32, name: &str, svc: Box<dyn Service>, enabled: bool) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let old = self.os(ni).ok_or(ActErr::NoIp(dev))?.svc_id(name);
        if let Some(id) = old {
            self.svc_stop(ni, id);
        }
        let now = self.now;
        let os = self.os_mut(ni).ok_or(ActErr::NoIp(dev))?;
        os.svcs.retain(|s| !s.name.eq_ignore_ascii_case(name));
        // the replaced one's bytes on the disk go with it (the new service starts empty)
        if let Some(id) = old {
            os.disk.forget(id);
        }
        let id = os.next_svc;
        os.next_svc += 1;
        let kind = svc.kind();
        os.svcs.push(SvcSlot { id, name: String::from(name), kind, enabled, running: false, since: now, svc: Some(svc), console: false, exit: None });
        if enabled {
            self.os_push(ni, Job::Start(id));
        }
        Ok(())
    }

    /// removes a service (stopping it first)
    pub fn remove_service(&mut self, dev: u32, name: &str) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let id = self.os(ni).ok_or(ActErr::NoIp(dev))?.svc_id(name).ok_or_else(|| ActErr::Refused(msg!("Den Dienst \u{201e}{}\u{201c} gibt es nicht.", name)))?;
        self.svc_stop(ni, id);
        if let Some(os) = self.os_mut(ni) {
            os.svcs.retain(|s| s.id != id);
            // what it stored on the disk goes with it
            os.disk.forget(id);
        }
        Ok(())
    }

    /// gives a service another name (Action::RenameService): it goes on as it was, without a
    /// restart -- its sockets, its disk and numbers, its time series (series.rs) -- and hears
    /// of it (Service::renamed); the name must be free on the device (case-insensitive) and
    /// one word
    pub fn rename_service(&mut self, dev: u32, name: &str, to: &str) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let os = self.os(ni).ok_or(ActErr::NoIp(dev))?;
        let id = os.svc_id(name).ok_or_else(|| ActErr::Refused(msg!("Den Dienst \u{201e}{}\u{201c} gibt es nicht.", name)))?;
        let to = to.trim();
        if to.is_empty() || to.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(ActErr::Refused(msg!("\u{201e}{}\u{201c} taugt nicht als Name eines Dienstes.", to)));
        }
        if os.svcs.iter().any(|s| s.id != id && s.name.eq_ignore_ascii_case(to)) {
            return Err(ActErr::Refused(msg!("Den Dienst \u{201e}{}\u{201c} gibt es schon.", to)));
        }
        if let Some(s) = self.os_mut(ni).and_then(|os| os.svcs.iter_mut().find(|s| s.id == id)) {
            s.name = String::from(to);
            if let Some(x) = s.svc.as_mut() {
                x.renamed(to);
            }
        }
        self.series.rename(dev, id, to);
        Ok(())
    }

    /// starts or stops a service ("systemctl start|stop", IOS "[no] service dhcp")
    pub fn set_service_running(&mut self, dev: u32, name: &str, on: bool) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let id = self.os(ni).ok_or(ActErr::NoIp(dev))?.svc_id(name).ok_or_else(|| ActErr::Refused(msg!("Den Dienst \u{201e}{}\u{201c} gibt es nicht.", name)))?;
        if let Some(s) = self.os_mut(ni).and_then(|os| os.svcs.iter_mut().find(|s| s.id == id)) {
            s.enabled = on;
        }
        if on {
            self.os_push(ni, Job::Start(id));
        } else {
            self.svc_stop(ni, id);
        }
        Ok(())
    }

    /// sends a request to a running service (Release, Renew)
    pub(crate) fn svc_ctl(&mut self, ni: usize, id: u32, c: Ctl) {
        self.os_push(ni, Job::Ev(id, SvcEvent::Ctl(c)));
    }

    /// the services of a device
    pub fn services(&self, dev: u32) -> Vec<SvcView> {
        let Some(os) = self.node_index(dev).and_then(|ni| self.os(ni)) else {
            return Vec::new();
        };
        let now = self.now;
        os.svcs
            .iter()
            .filter(|s| !s.console)
            .map(|s| {
                let status_msg = s.svc.as_ref().map(|x| x.status_msg()).unwrap_or_default();
                SvcView {
                    name: s.name.clone(),
                    kind: s.kind,
                    enabled: s.enabled,
                    running: s.running,
                    since: s.since,
                    status: status_msg.text(),
                    metrics: s.svc.as_ref().map(|x| x.metrics_at(now)).unwrap_or_default(),
                    status_msg,
                }
            })
            .collect()
    }

    /// a service of a device as its own type
    pub fn service<T: Service + 'static>(&self, dev: u32, name: &str) -> Option<&T> {
        let os = self.os(self.node_index(dev)?)?;
        os.svc(name)?.svc.as_ref()?.as_any().downcast_ref::<T>()
    }

    /// the first service of a type on a device
    pub fn service_of<T: Service + 'static>(&self, dev: u32) -> Option<&T> {
        let os = self.os(self.node_index(dev)?)?;
        os.svcs.iter().find_map(|s| s.svc.as_ref()?.as_any().downcast_ref::<T>())
    }

    /// a number a service reports (`expect SRV1 svc web metric requests == 4`)
    pub fn metric(&self, dev: u32, svc: &str, name: &str) -> Option<i64> {
        let os = self.os(self.node_index(dev)?)?;
        os.svc(svc)?.svc.as_ref()?.metrics_at(self.now).into_iter().find(|m| m.name == name).map(|m| m.value)
    }

    /// the device's log, oldest first
    pub fn log_lines(&self, dev: u32) -> Vec<LogLine> {
        self.node_index(dev).and_then(|ni| self.os(ni)).map(|o| o.log.lines.iter().cloned().collect()).unwrap_or_default()
    }

    /// the device clock now (ns since 1970)
    pub fn clock(&self, dev: u32) -> Option<i64> {
        let now = self.now;
        self.node_index(dev).and_then(|ni| self.os(ni)).map(|o| o.clock.read(now))
    }

    /// the device's clock state (NTP status)
    pub fn clock_state(&self, dev: u32) -> Option<Clock> {
        self.node_index(dev).and_then(|ni| self.os(ni)).map(|o| o.clock.clone())
    }

    /// the static host names of a device
    pub fn hosts(&self, dev: u32) -> Vec<(String, Ipv4)> {
        self.node_index(dev).and_then(|ni| self.os(ni)).map(|o| o.hosts.clone()).unwrap_or_default()
    }

    /// the bound UDP ports of a device ("ss -ulpn", "show ip sockets")
    pub fn sockets(&self, dev: u32) -> Vec<Sock> {
        self.node_index(dev).and_then(|ni| self.os(ni)).map(|o| o.socks.clone()).unwrap_or_default()
    }
}

// ---------------------------------------------------------------- TCP sockets of services (tcp.rs)

impl OsApi<'_> {
    /// a listening socket on `port` of every address (0: an ephemeral port); `backlog` 0: the
    /// system's
    pub fn tcp_listen(&mut self, port: u16, backlog: u32, opts: crate::tcp::SockOpts) -> Result<u32, String> {
        self.tcp_listen_msg(port, backlog, opts).map_err(|m| m.text())
    }
    /// tcp_listen(), the refusal as a message
    pub fn tcp_listen_msg(&mut self, port: u16, backlog: u32, opts: crate::tcp::SockOpts) -> Result<u32, Msg> {
        self.eng.tcp_listen(self.ni, self.id, Ipv4::UNSPECIFIED, port, backlog, opts)
    }
    /// takes the next connection from the queue of a listening socket
    pub fn tcp_accept(&mut self, listener: u32) -> Option<u32> {
        self.eng.tcp_accept(self.ni, self.id, listener)
    }
    /// opens a connection from an ephemeral port; TcpConnected or TcpClosed follows
    pub fn tcp_connect(&mut self, dst: Ipv4, dport: u16, opts: crate::tcp::SockOpts) -> Result<u32, String> {
        self.tcp_connect_msg(dst, dport, opts).map_err(|m| m.text())
    }
    /// tcp_connect(), the refusal as a message
    pub fn tcp_connect_msg(&mut self, dst: Ipv4, dport: u16, opts: crate::tcp::SockOpts) -> Result<u32, Msg> {
        self.eng.tcp_connect(self.ni, self.id, 0, dst, dport, opts, self.cause)
    }
    /// the same from a given local port (bind, then connect)
    pub fn tcp_connect_from(&mut self, sport: u16, dst: Ipv4, dport: u16, opts: crate::tcp::SockOpts) -> Result<u32, String> {
        self.tcp_connect_from_msg(sport, dst, dport, opts).map_err(|m| m.text())
    }
    /// tcp_connect_from(), the refusal as a message
    pub fn tcp_connect_from_msg(&mut self, sport: u16, dst: Ipv4, dport: u16, opts: crate::tcp::SockOpts) -> Result<u32, Msg> {
        self.eng.tcp_connect(self.ni, self.id, sport, dst, dport, opts, self.cause)
    }
    /// queues bytes for sending: how many the send buffer took (TcpWritable follows when it
    /// has room again)
    pub fn tcp_send(&mut self, sock: u32, data: &[u8]) -> usize {
        self.eng.tcp_write(self.ni, self.id, sock, data, self.cause)
    }
    /// reads up to `max` bytes
    pub fn tcp_recv(&mut self, sock: u32, max: usize) -> Vec<u8> {
        self.eng.tcp_read(self.ni, self.id, sock, max, self.cause)
    }
    /// sends nothing more: a FIN after the queued bytes (reading goes on)
    pub fn tcp_shutdown(&mut self, sock: u32) {
        self.eng.tcp_shutdown(self.ni, self.id, sock, self.cause);
    }
    /// closes a connection (it ends in the kernel) or a listening socket
    pub fn tcp_close(&mut self, sock: u32) {
        self.eng.tcp_close(self.ni, self.id, sock, self.cause);
    }
    /// resets a connection (close with SO_LINGER 0)
    pub fn tcp_abort(&mut self, sock: u32) {
        self.eng.tcp_abort(self.ni, self.id, sock, self.cause);
    }
    /// TCP_NODELAY, SO_KEEPALIVE and SO_SNDBUF of a connection
    pub fn tcp_setopts(&mut self, sock: u32, opts: crate::tcp::SockOpts) {
        self.eng.tcp_setopts(self.ni, self.id, sock, opts, self.cause);
    }
    /// a socket as ss shows it
    pub fn tcp_info(&self, sock: u32) -> Option<crate::tcp::TcpRow> {
        self.eng.tcp_row(self.ni, sock)
    }
}

// ---------------------------------------------------------------- console programs (cli/tools.rs)

impl OsApi<'_> {
    /// writes to the device's console (a program the shell started)
    pub fn print(&mut self, text: &str) {
        self.eng.out(self.ni, text);
    }
    /// the program ends with an exit status: its service goes and the console takes lines
    /// again (at the same virtual time, after this call)
    pub fn exit(&mut self, code: i32) {
        let (ni, id, now) = (self.ni, self.id, self.eng.now);
        if let Some(s) = self.eng.os_mut(ni).and_then(|os| os.svcs.iter_mut().find(|s| s.id == id)) {
            s.exit = Some(code);
        }
        self.eng.cli_wake_at(ni, now);
    }
    /// the service's id (ss -p shows it as the process id)
    pub fn svc_id(&self) -> u32 {
        self.id
    }
    /// the service's name on the device
    pub fn svc_name(&self) -> String {
        let id = self.id;
        self.eng.os(self.ni).and_then(|o| o.svcs.iter().find(|s| s.id == id)).map(|s| s.name.clone()).unwrap_or_default()
    }
    /// the service ends by itself (a script after an error): it stops as when stopped from
    /// outside, its sockets, timers and queued events go (Service::stop is not called: it
    /// cleaned up already); it stays enabled, so it starts again with the device or on request
    pub fn halt(&mut self) {
        let (ni, id) = (self.ni, self.id);
        self.eng.svc_stop(ni, id);
    }
}

impl Engine {
    /// starts a program on a device's console: a service that is not listed with the others;
    /// its id
    pub(crate) fn console_program(&mut self, ni: usize, name: &str, svc: Box<dyn Service>) -> Option<u32> {
        let dev = self.dev_of(ni);
        let os = self.os(ni)?;
        let mut n = String::from(name);
        let mut k = 1;
        while os.svc(&n).is_some() {
            k += 1;
            n = format!("{}#{}", name, k);
        }
        self.add_service(dev, &n, svc, true).ok()?;
        let s = self.os_mut(ni)?.svcs.iter_mut().find(|s| s.name == n)?;
        s.console = true;
        Some(s.id)
    }
    /// a console program's exit status once it ended
    pub(crate) fn console_exit(&self, ni: usize, id: u32) -> Option<i32> {
        self.os(ni)?.svcs.iter().find(|s| s.id == id).map(|s| s.exit).unwrap_or(Some(1))
    }
    /// ends a console program (Ctrl+C, its exit): its sockets close as when a process ends
    pub(crate) fn console_kill(&mut self, ni: usize, id: u32) {
        let name = self.os(ni).and_then(|o| o.svcs.iter().find(|s| s.id == id)).map(|s| s.name.clone());
        let dev = self.dev_of(ni);
        if let Some(n) = name {
            let _ = self.remove_service(dev, &n);
        }
    }
    /// gives a console program a line typed at the console, or the end of its input
    pub(crate) fn console_input(&mut self, ni: usize, id: u32, ev: ConsoleEv) {
        self.os_push(ni, Job::Ev(id, SvcEvent::Console(ev)));
    }
    /// the console programs' and services' ids, names and kinds (top, ss -p)
    pub fn service_ids(&self, dev: u32) -> Vec<(u32, String, &'static str, bool)> {
        self.node_index(dev).and_then(|ni| self.os(ni)).map(|o| o.svcs.iter().map(|s| (s.id, s.name.clone(), s.kind, s.running)).collect()).unwrap_or_default()
    }
}
