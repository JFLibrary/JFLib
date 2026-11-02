// fw -- the zone firewall of firewall models: interfaces belong to zones, policies decide between
// two zones, and connection tracking lets the answers of what a policy inspected back in
//
//   Firewall { zones, members, classes, policies, pairs, timers, max_half_open, .. }   a device's
//                                    configuration and sessions (IpStack::fw); active(): an
//                                    interface has a zone, else nothing here runs at all
//   ClassMap { any, matches } / Match::Protocol(name) / Match::Access(list)   what a class selects
//   PolicyMap { classes, default } / PolicyClass { class, action, log, hits }   what happens to it
//   ZonePair { src, dst, policy }    a direction between two zones (SELF_ZONE: the device itself)
//   FwFlow                           a packet as the firewall reads it, IPv4 or IPv6
//   Firewall::probe(from, to, &FwFlow, &Acls, now) -> FwProbe   what the zones and sessions say
//                                    about a packet, counting nothing: the answer Engine::fw_pass
//                                    acts on (the Firewall tab's Test asks the same function)
//   Engine::fw_test(dev, iface, out, &Flow) -> FwTest   the same for a packet that is not sent:
//                                    which zones it would pass and what they say
//   Engine::fw_in / fw_out / fw_in6 / fw_out6   the hooks in l3.rs and l3v6.rs
//   Engine::fw_sessions(dev) -> Vec<FwRow>   the live sessions of a device
//   FwOp                             what Action::Fw does
//   HostFw { enabled, deny_in, deny_out, rules }   the small firewall of a Linux host (cli/ufw.rs): the
//                                    same sessions, its own rules; Firewall::host
//
// The rules.  An interface without a zone is outside the firewall's business only towards other
// interfaces without a zone.  Between two zones a packet is dropped unless a zone pair for that
// direction has a policy that lets it through; inside one zone it passes; between an interface in a
// zone and one without it is dropped.  The device itself is the zone "self": what comes to it or
// leaves it passes unless a zone pair names self and has a policy, and then that policy decides as
// between any two zones.  In a policy the first class that matches decides; what no class matches
// meets "class-default", which drops.  A class that names no action drops as well.
//
// The actions.  "inspect" starts a session, and a session lets the rest of its connection through
// in both directions without asking a policy again.  "pass" lets the one packet through and
// remembers nothing, so an answer needs its own rule.  "drop" drops silently, "reject" drops and
// tells the sender: ICMP "administratively prohibited" (ICMPv6 likewise), under the device's limit
// of unreachables.  Access lists keep their own behaviour (acl.rs).  "log" writes a line to the
// console and the device's log (and so to its syslog hosts): "inspect" when a session starts and
// ends, the others at most every five minutes with the number of packets since.
//
// The order of a packet's way through a device, for all of acl.rs, nat.rs and this file:
//
//   arriving          1. inbound access list of the interface (sees the packet as it arrived)
//                     2. NAT from outside: the destination becomes the inside address
//   for the device    3. zones: from the interface's zone to self
//   to be forwarded   3. route lookup
//                     4. zones and sessions: from the zone of the way in to the zone of the way out
//                     5. NAT from inside: the source becomes the global address
//                     6. outbound access list (sees the translated source)
//   made by the device   route lookup, zones from self to the zone of the way out, NAT (only the
//                     quote of ICMP errors), no outbound list
//
// So zones and sessions see the inside addresses in both directions, before the source is
// translated on the way out and after the destination was translated on the way in: a class matches
// the real address of a host, and a session reads the same on the way there and back.  A packet an
// access list dropped never reaches the zones, and one the zones dropped starts no translation.
// IPv6 has no access lists and no NAT here, so only step 3 or 4 happens.  Neighbor discovery and MLD
// are the neighbours' business, as ARP is for IPv4, and no firewall step sees them.
//
// Sessions.  TCP follows SYN (session begins), SYN+ACK, ACK (established), FIN from one end, FIN
// from both, RST (session ends at once).  Only a SYN without ACK begins a session; "inspect" drops
// any other segment without one.  There is no check of sequence numbers.  UDP and other protocols
// have an idle timer, a UDP datagram to port 53 a shorter one.  ICMP queries begin a session and
// their answers need it; an answer without one is dropped.  An ICMP error about a packet of a session
// passes as related and refreshes nothing.  A later fragment has no ports: it passes if its two
// addresses have a session of its protocol, else a policy sees it without ports.  Half-open TCP
// sessions are limited ("ip inspect max-incomplete"); a SYN beyond the limit is dropped.
//
// A session is a BTreeMap entry with no timer of its own: when a packet reaches the firewall, what
// has timed out goes first (`due`), and the views skip what has ended.  So no event or timer slot
// exists for sessions, and their notes come when the next packet looks.  A power cycle, a restart
// with another configuration and "clear zone-pair inspect sessions" forget them; when a zone pair goes
// or changes its policy, its sessions go along.  What this does not do: no application inspection ("match protocol
// http" is TCP port 80), no IPS, no user or URL filtering, no inspection of FTP's data connections.
//
// The host firewall.  On a Linux host every interface counts as the zone "net" and the host is self.
// While it is on, what comes in meets its rules in their order and then its default (deny), what
// goes out likewise (allow).  An allowed packet that can begin a session begins one, so answers come
// back whatever the default says; one that cannot passes alone.  Sessions, timers, notes and the
// half-open limit are the ones above.  As with the original, ICMP errors and echo requests and a DHCP
// client's answers pass without a rule.  What the host forwards stays in "net" and passes.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::msg;
use netpkt::icmp::{self, Icmp, UNREACH_ADMIN};
use netpkt::icmpv6::{self, Icmp6, Icmp6View};
use netpkt::ipv4::{PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use netpkt::ipv6::{self, Ipv6View, NH_ICMPV6};
use netpkt::nat::{reply_of, Flow};
use netpkt::tcp::{ACK, FIN, RST, SYN};
use netpkt::{IpAddr, Ipv4, Prefix4};

use crate::acl::Acls;
use crate::action::ActErr;
use crate::fwtrace::FwNote;
use crate::kernel::{Engine, Ns, SEC};
use crate::trace::{HopId, Note};

/// the zone of the device itself
pub const SELF_ZONE: &str = "self";
/// the zone of a host's interfaces while its firewall is on
pub const HOST_NET: &str = "net";
/// the name of the host firewall in notes and sessions
pub const HOST_PAIR: &str = "ufw";
/// the class of what no other class matches
pub const CLASS_DEFAULT: &str = "class-default";
/// half-open TCP sessions a device keeps at most, unless configured
pub const HALF_OPEN_MAX: u32 = 1000;
/// "log" of a class without sessions reports again after this long at the earliest
const LOG_EVERY: Ns = 300 * SEC;

/// what "match protocol" knows: (name, IP protocol, 0: TCP and UDP; destination port, 0: any; the
/// help text).  A name with a port means that port and nothing more: no look into the data.
pub const FW_PROTOCOLS: &[(&str, u8, u16, &str)] = &[
    ("dns", 0, 53, "DNS, TCP or UDP port 53"),
    ("ftp", PROTO_TCP, 21, "FTP control, TCP port 21"),
    ("http", PROTO_TCP, 80, "Web, TCP port 80"),
    ("https", PROTO_TCP, 443, "Encrypted web, TCP port 443"),
    ("icmp", PROTO_ICMP, 0, "ICMP and ICMPv6"),
    ("imap", PROTO_TCP, 143, "Mail retrieval, TCP port 143"),
    ("ntp", PROTO_UDP, 123, "Network time, UDP port 123"),
    ("pop3", PROTO_TCP, 110, "Mail retrieval, TCP port 110"),
    ("smtp", PROTO_TCP, 25, "Mail transport, TCP port 25"),
    ("snmp", PROTO_UDP, 161, "SNMP, UDP port 161"),
    ("ssh", PROTO_TCP, 22, "Secure shell, TCP port 22"),
    ("syslog", PROTO_UDP, 514, "Syslog, UDP port 514"),
    ("tcp", PROTO_TCP, 0, "Every TCP connection"),
    ("telnet", PROTO_TCP, 23, "Telnet, TCP port 23"),
    ("tftp", PROTO_UDP, 69, "Trivial file transfer, UDP port 69"),
    ("udp", PROTO_UDP, 0, "Every UDP flow"),
];

/// what a class of a policy does with a packet
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwAction {
    Inspect,
    Pass,
    Drop,
    Reject,
}

impl FwAction {
    pub const ALL: [FwAction; 4] = [FwAction::Inspect, FwAction::Pass, FwAction::Drop, FwAction::Reject];
    /// the configuration word
    pub fn word(self) -> &'static str {
        match self {
            FwAction::Inspect => "inspect",
            FwAction::Pass => "pass",
            FwAction::Drop => "drop",
            FwAction::Reject => "reject",
        }
    }
}

/// a condition of a class
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Match {
    /// a name of FW_PROTOCOLS
    Protocol(String),
    /// an access list that permits the packet (IPv4 only)
    Access(String),
}

impl Match {
    /// the configuration line in a class-map
    pub fn config(&self) -> String {
        match self {
            Match::Protocol(p) => format!("match protocol {}", p),
            Match::Access(l) if l.bytes().all(|b| b.is_ascii_digit()) => format!("match access-group {}", l),
            Match::Access(l) => format!("match access-group name {}", l),
        }
    }
}

/// "class-map type inspect [match-any|match-all] NAME"
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClassMap {
    /// one condition is enough (match-any), else all must hold (match-all, the default)
    pub any: bool,
    pub matches: Vec<Match>,
}

/// a class in a policy with its action
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyClass {
    pub class: String,
    /// None: no action named, it drops
    pub action: Option<FwAction>,
    pub log: bool,
    /// packets it decided
    pub hits: u64,
    pub(crate) logged: Option<Ns>,
    pub(crate) unlogged: u64,
}

impl PolicyClass {
    fn new(class: &str, action: Option<FwAction>) -> PolicyClass {
        PolicyClass { class: String::from(class), action, log: false, hits: 0, logged: None, unlogged: 0 }
    }
}

/// "policy-map type inspect NAME"
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyMap {
    pub classes: Vec<PolicyClass>,
    /// what no class matches: "class class-default" (drops unless changed)
    pub default: PolicyClass,
}

impl Default for PolicyMap {
    fn default() -> PolicyMap {
        PolicyMap { classes: Vec::new(), default: PolicyClass::new(CLASS_DEFAULT, Some(FwAction::Drop)) }
    }
}

/// "zone-pair security NAME source A destination B" with its service policy
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZonePair {
    pub src: String,
    pub dst: String,
    pub policy: Option<String>,
}

/// which timer of a session ("ip inspect ...")
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwTimer {
    /// a TCP session before the handshake is done
    Syn,
    /// an established TCP session without traffic
    Tcp,
    /// after a FIN
    Fin,
    Udp,
    /// a UDP datagram to port 53
    Dns,
    Icmp,
}

impl FwTimer {
    pub const ALL: [FwTimer; 6] = [FwTimer::Syn, FwTimer::Tcp, FwTimer::Fin, FwTimer::Udp, FwTimer::Dns, FwTimer::Icmp];
    /// the words after "ip inspect"
    pub fn words(self) -> &'static str {
        match self {
            FwTimer::Syn => "tcp synwait-time",
            FwTimer::Tcp => "tcp idle-time",
            FwTimer::Fin => "tcp finwait-time",
            FwTimer::Udp => "udp idle-time",
            FwTimer::Dns => "dns-timeout",
            FwTimer::Icmp => "icmp idle-time",
        }
    }
    /// seconds, unless configured
    pub fn default_secs(self) -> u32 {
        match self {
            FwTimer::Syn | FwTimer::Fin | FwTimer::Udp => 30,
            FwTimer::Tcp => 3600,
            FwTimer::Dns => 5,
            FwTimer::Icmp => 10,
        }
    }
}

/// the timers in seconds
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FwTimers {
    pub secs: [u32; 6],
}

impl Default for FwTimers {
    fn default() -> FwTimers {
        FwTimers { secs: FwTimer::ALL.map(FwTimer::default_secs) }
    }
}

impl FwTimers {
    pub fn get(&self, t: FwTimer) -> u32 {
        self.secs[FwTimer::ALL.iter().position(|x| *x == t).unwrap_or(0)]
    }
    pub fn set(&mut self, t: FwTimer, v: u32) {
        if let Some(i) = FwTimer::ALL.iter().position(|x| *x == t) {
            self.secs[i] = v;
        }
    }
    fn span(&self, t: FwTimer) -> Ns {
        self.get(t) as Ns * SEC
    }
}

/// how far a TCP connection has come
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpState {
    /// the SYN went through
    Syn,
    /// the SYN+ACK came back
    SynAck,
    Established,
    /// a FIN from one end
    Fin,
    /// a FIN from both ends
    Closing,
}

impl TcpState {
    /// the word of the show commands
    pub fn word(self) -> &'static str {
        match self {
            TcpState::Syn => "SYN",
            TcpState::SynAck => "SYN+ACK",
            TcpState::Established => "established",
            TcpState::Fin => "FIN",
            TcpState::Closing => "closing",
        }
    }
    pub fn half_open(self) -> bool {
        matches!(self, TcpState::Syn | TcpState::SynAck)
    }
}

/// a connection the firewall follows; `src` began it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FwSession {
    pub id: u64,
    pub proto: u8,
    pub src: IpAddr,
    pub sport: u16,
    pub dst: IpAddr,
    pub dport: u16,
    /// the zones of its first packet
    pub from: String,
    pub to: String,
    pub pair: String,
    pub class: String,
    pub tcp: Option<TcpState>,
    pub created: Ns,
    pub last: Ns,
    /// packets and bytes from the side that began it, and back
    pub packets: [u64; 2],
    pub bytes: [u64; 2],
    /// its class has "log": its end is reported as well
    pub log: bool,
    fin: [bool; 2],
}

/// a session's place in the index: as a packet has its addresses and ports
type Key = (u8, IpAddr, IpAddr, u16, u16);

impl FwSession {
    fn key(&self, back: bool) -> Key {
        if back { (self.proto, self.dst, self.src, self.dport, self.sport) } else { (self.proto, self.src, self.dst, self.sport, self.dport) }
    }
    /// when it ends without further traffic
    pub fn ends(&self, t: &FwTimers) -> Ns {
        let span = match (self.proto, self.tcp) {
            (_, Some(TcpState::Syn | TcpState::SynAck)) => t.span(FwTimer::Syn),
            (_, Some(TcpState::Established)) => t.span(FwTimer::Tcp),
            (_, Some(TcpState::Fin | TcpState::Closing)) => t.span(FwTimer::Fin),
            (PROTO_ICMP | NH_ICMPV6, _) => t.span(FwTimer::Icmp),
            (PROTO_UDP, _) if self.dport == 53 => t.span(FwTimer::Dns),
            _ => t.span(FwTimer::Udp),
        };
        self.last.saturating_add(span)
    }
    /// the session as a packet reads ("tcp 10.0.0.1:1024 -> 10.0.0.2:80")
    pub fn text(&self) -> String {
        flow_text(self.proto, self.src, self.sport, self.dst, self.dport, true)
    }
}

/// a protocol as the firewall's texts write it
pub fn proto_word(p: u8) -> String {
    match p {
        PROTO_TCP => String::from("tcp"),
        PROTO_UDP => String::from("udp"),
        PROTO_ICMP => String::from("icmp"),
        NH_ICMPV6 => String::from("icmpv6"),
        n => format!("protocol {}", n),
    }
}

fn ep_text(a: IpAddr, p: u16, ports: bool) -> String {
    match a {
        IpAddr::V4(x) if ports => format!("{}:{}", x, p),
        IpAddr::V6(x) if ports => format!("[{}]:{}", x, p),
        IpAddr::V4(x) => format!("{}", x),
        IpAddr::V6(x) => format!("{}", x),
    }
}

/// "tcp 10.0.0.1:1024 -> 10.0.0.2:80"; ICMP queries and protocols without ports by their addresses
fn flow_text(proto: u8, src: IpAddr, sport: u16, dst: IpAddr, dport: u16, ports: bool) -> String {
    let ports = ports && matches!(proto, PROTO_TCP | PROTO_UDP);
    format!("{} {} -> {}", proto_word(proto), ep_text(src, sport, ports), ep_text(dst, dport, ports))
}

/// the counters of a device's firewall
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FwStats {
    /// sessions that began, that timed out, that a RST ended
    pub created: u64,
    pub expired: u64,
    pub reset: u64,
    /// packets a session let through
    pub session_hits: u64,
    /// packets dropped for want of a rule between the zones, or for lacking a zone
    pub no_rule: u64,
    /// packets "inspect" dropped: not the beginning of anything; the half-open limit
    pub not_new: u64,
    pub limited: u64,
}

/// the firewall part of a device: configuration and sessions
#[derive(Clone, Debug)]
pub struct Firewall {
    /// zone name -> description
    pub zones: BTreeMap<String, String>,
    /// interface -> zone
    pub members: BTreeMap<u16, String>,
    pub classes: BTreeMap<String, ClassMap>,
    pub policies: BTreeMap<String, PolicyMap>,
    pub pairs: BTreeMap<String, ZonePair>,
    pub timers: FwTimers,
    pub max_half_open: u32,
    pub stats: FwStats,
    /// the firewall of a Linux host (cli/ufw.rs)
    pub host: HostFw,
    sessions: BTreeMap<u64, FwSession>,
    index: BTreeMap<Key, (u64, bool)>,
    next_id: u64,
    /// the earliest a session can have ended
    due: Option<Ns>,
}

impl Default for Firewall {
    fn default() -> Firewall {
        Firewall { zones: BTreeMap::new(), members: BTreeMap::new(), classes: BTreeMap::new(), policies: BTreeMap::new(), pairs: BTreeMap::new(), timers: FwTimers::default(), max_half_open: HALF_OPEN_MAX, stats: FwStats::default(), host: HostFw::default(), sessions: BTreeMap::new(), index: BTreeMap::new(), next_id: 0, due: None }
    }
}

/// a packet as the firewall reads it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FwFlow {
    pub proto: u8,
    pub src: IpAddr,
    pub dst: IpAddr,
    /// TCP and UDP ports; an ICMP query's identifier in both
    pub sport: u16,
    pub dport: u16,
    /// the ports (or the identifier) are known
    pub ports: bool,
    pub icmp: Option<(u8, u8)>,
    pub flags: u8,
    /// a fragment after the first
    pub later: bool,
    /// an ICMP error: the packet it quotes (protocol, source, destination, ports)
    pub quoted: Option<Key>,
    /// what access lists read (IPv4)
    pub v4: Option<Flow>,
    pub len: u32,
}

impl FwFlow {
    /// a packet from what acl.rs and nat.rs read of it; `quoted`: of an ICMP error
    pub fn of_v4(f: &Flow, len: u32, quoted: Option<Flow>) -> FwFlow {
        let quoted = quoted.map(|q| (q.proto, IpAddr::V4(q.src), IpAddr::V4(q.dst), q.sport, q.dport));
        FwFlow { proto: f.proto, src: IpAddr::V4(f.src), dst: IpAddr::V4(f.dst), sport: f.sport, dport: f.dport, ports: f.ports, icmp: f.icmp, flags: f.flags, later: f.later, quoted, v4: Some(*f), len }
    }
    fn of_packet(pkt: &[u8]) -> Option<FwFlow> {
        let f = Flow::of(pkt)?;
        let error = f.proto == PROTO_ICMP && f.icmp.is_some_and(|(t, _)| is_error4(t));
        Some(FwFlow::of_v4(&f, pkt.len() as u32, if error && !f.later { Flow::quoted(pkt) } else { None }))
    }
    /// an IPv6 packet; None: neighbor discovery and MLD, which no firewall step sees
    fn of_packet6(pkt: &[u8]) -> Option<FwFlow> {
        let v = Ipv6View::parse(pkt)?;
        let later = ipv6::frag(&v).is_some_and(|(h, _)| h.offset > 0);
        let (nh, off) = ipv6::upper_layer(&v)?;
        let body = v.payload().get(off..).unwrap_or(&[]);
        let mut f = FwFlow { proto: nh, src: IpAddr::V6(v.src()), dst: IpAddr::V6(v.dst()), sport: 0, dport: 0, ports: false, icmp: None, flags: 0, later, quoted: None, v4: None, len: pkt.len() as u32 };
        if later {
            return Some(f);
        }
        match nh {
            PROTO_TCP | PROTO_UDP if body.len() >= 4 => {
                f.sport = netpkt::be16(body, 0);
                f.dport = netpkt::be16(body, 2);
                f.ports = true;
                if nh == PROTO_TCP && body.len() >= 14 {
                    f.flags = body[13];
                }
            }
            NH_ICMPV6 if body.len() >= 2 => {
                let ic = Icmp6View::parse(body)?;
                if ic.is_nd() || matches!(ic.typ(), icmpv6::T_MLD_QUERY | icmpv6::T_MLD_REPORT | icmpv6::T_MLD_DONE | icmpv6::T_MLD2_REPORT) {
                    return None;
                }
                f.icmp = Some((body[0], body[1]));
                if matches!(ic.typ(), icmpv6::T_ECHO_REQUEST | icmpv6::T_ECHO_REPLY) && body.len() >= 6 {
                    f.sport = netpkt::be16(body, 4);
                    f.dport = f.sport;
                    f.ports = true;
                }
                if ic.is_error()
                    && let Some(q) = ic.quoted()
                {
                    let (qn, qoff) = ipv6::upper_layer(&q).unwrap_or((q.next_header(), 0));
                    let qb = q.payload().get(qoff..).unwrap_or(&[]);
                    let (qs, qd) = match qn {
                        PROTO_TCP | PROTO_UDP if qb.len() >= 4 => (netpkt::be16(qb, 0), netpkt::be16(qb, 2)),
                        NH_ICMPV6 if qb.len() >= 6 => (netpkt::be16(qb, 4), netpkt::be16(qb, 4)),
                        _ => (0, 0),
                    };
                    f.quoted = Some((qn, IpAddr::V6(q.src()), IpAddr::V6(q.dst()), qs, qd));
                }
            }
            _ => {}
        }
        Some(f)
    }
    fn key(&self) -> Key {
        if self.ports { (self.proto, self.src, self.dst, self.sport, self.dport) } else { (self.proto, self.src, self.dst, 0, 0) }
    }
    /// the packet as the texts write it
    pub fn text(&self) -> String {
        flow_text(self.proto, self.src, self.sport, self.dst, self.dport, self.ports)
    }
    fn is_icmp(&self) -> bool {
        matches!(self.proto, PROTO_ICMP | NH_ICMPV6)
    }
    /// an ICMP query that asks for an answer
    fn icmp_request(&self) -> bool {
        match (self.proto, self.icmp) {
            (PROTO_ICMP, Some((t, _))) => reply_of(t).is_some(),
            (NH_ICMPV6, Some((t, _))) => t == icmpv6::T_ECHO_REQUEST,
            _ => false,
        }
    }
    /// it can be the first packet of a session
    fn begins(&self) -> bool {
        if self.later {
            return false;
        }
        match self.proto {
            PROTO_TCP => self.ports && self.flags & SYN != 0 && self.flags & (ACK | RST) == 0,
            PROTO_UDP => self.ports,
            PROTO_ICMP | NH_ICMPV6 => self.icmp_request() || !self.ports && self.quoted.is_none() && self.icmp.is_some_and(|(t, _)| !is_error_any(self.proto, t)),
            _ => true,
        }
    }
}

fn is_error4(t: u8) -> bool {
    matches!(t, icmp::T_UNREACH | icmp::T_TIME_EXCEEDED | icmp::T_PARAM_PROBLEM | icmp::T_REDIRECT | 4)
}

fn is_error_any(proto: u8, t: u8) -> bool {
    if proto == PROTO_ICMP { is_error4(t) } else { t < 128 }
}

/// what the zones and sessions say about a packet (Firewall::probe)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FwProbe {
    /// neither interface has a zone: not the firewall's business
    NoZones,
    /// both interfaces are in this zone: it passes
    SameZone(String),
    /// to or from the device itself, and no policy says otherwise: it passes
    SelfPass { from: String, to: String },
    /// one interface has this zone, the other none: dropped
    Unzoned { zone: String },
    /// no pair with a policy between the zones: dropped
    NoRule { from: String, to: String },
    /// a packet of this session (back: on the way back)
    Session { id: u64, back: bool, pair: String },
    /// an ICMP error about a packet of this session: it passes
    Related { id: u64, pair: String },
    /// a policy decides; `at` is the index of the class (None: class-default)
    Policy { pair: String, policy: String, class: String, at: Option<usize>, action: FwAction, log: bool },
    /// "inspect" met a packet that begins nothing: dropped
    NotNew { pair: String, class: String },
    /// "inspect" met a SYN beyond the limit of half-open sessions: dropped
    Limit { pair: String, class: String, max: u32 },
}

impl FwProbe {
    /// the packet goes on
    pub fn passes(&self) -> bool {
        match self {
            FwProbe::NoZones | FwProbe::SameZone(_) | FwProbe::SelfPass { .. } | FwProbe::Session { .. } | FwProbe::Related { .. } => true,
            FwProbe::Policy { action, .. } => matches!(action, FwAction::Inspect | FwAction::Pass),
            FwProbe::Unzoned { .. } | FwProbe::NoRule { .. } | FwProbe::NotNew { .. } | FwProbe::Limit { .. } => false,
        }
    }
}

impl Firewall {
    /// something to do: an interface has a zone, or the host firewall is on
    pub fn active(&self) -> bool {
        !self.members.is_empty() || self.host.enabled
    }
    /// the zone of an interface; None as the interface: the device itself
    pub fn zone_of(&self, iface: Option<u16>) -> Option<&str> {
        match iface {
            None => Some(SELF_ZONE),
            Some(_) if self.host.enabled => Some(HOST_NET),
            Some(i) => self.members.get(&i).map(String::as_str),
        }
    }
    /// the pair for a direction between two zones
    pub fn pair_of(&self, from: &str, to: &str) -> Option<(&String, &ZonePair)> {
        self.pairs.iter().find(|(_, p)| p.src == from && p.dst == to)
    }
    /// the sessions, the ended ones too until a packet looks
    pub fn sessions(&self) -> impl Iterator<Item = &FwSession> {
        self.sessions.values()
    }
    /// the configuration and counters without the sessions (what a window reads every frame)
    pub fn without_sessions(&self) -> Firewall {
        Firewall { zones: self.zones.clone(), members: self.members.clone(), classes: self.classes.clone(), policies: self.policies.clone(), pairs: self.pairs.clone(), timers: self.timers, max_half_open: self.max_half_open, stats: self.stats, host: self.host.clone(), sessions: BTreeMap::new(), index: BTreeMap::new(), next_id: self.next_id, due: None }
    }
    /// the half-open TCP sessions at `now`
    pub fn half_open(&self, now: Ns) -> u32 {
        self.sessions.values().filter(|s| s.tcp.is_some_and(TcpState::half_open) && s.ends(&self.timers) > now).count() as u32
    }
    /// forgets every session and counter (the device was switched off; the configuration stays)
    pub fn reset(&mut self) {
        self.sessions.clear();
        self.index.clear();
        self.due = None;
        self.stats = FwStats::default();
        self.clear_counters();
    }
    /// the counters of the classes and the firewall back to 0 ("clear zone-pair counter")
    pub fn clear_counters(&mut self) {
        self.stats = FwStats::default();
        for p in self.policies.values_mut() {
            for c in p.classes.iter_mut().chain(core::iter::once(&mut p.default)) {
                c.hits = 0;
                c.logged = None;
                c.unlogged = 0;
            }
        }
    }
    /// an interface went away
    pub fn unbind(&mut self, iface: u16) {
        self.members.remove(&iface);
    }
    /// what the configuration and sessions hold (keyframe.rs node_bytes)
    pub fn footprint(&self) -> usize {
        let named = |n: usize| n * 96;
        named(self.zones.len() + self.members.len() + self.pairs.len())
            + self.classes.values().map(|c| 96 + c.matches.len() * 48).sum::<usize>()
            + self.policies.values().map(|p| 96 + p.classes.len() * core::mem::size_of::<PolicyClass>()).sum::<usize>()
            + self.sessions.len() * (core::mem::size_of::<FwSession>() + 64)
            + self.index.len() * core::mem::size_of::<(Key, (u64, bool))>()
    }
    /// forgets sessions (of one pair); how many
    pub fn clear_sessions(&mut self, pair: Option<&str>) -> usize {
        let gone: Vec<u64> = self.sessions.values().filter(|s| pair.is_none_or(|p| p == s.pair)).map(|s| s.id).collect();
        for id in &gone {
            self.remove(*id);
        }
        gone.len()
    }
    fn remove(&mut self, id: u64) -> Option<FwSession> {
        let s = self.sessions.remove(&id)?;
        self.index.remove(&s.key(false));
        self.index.remove(&s.key(true));
        Some(s)
    }
    /// the session a packet between these zones belongs to: (id, on the way back)
    fn find(&self, f: &FwFlow, from: &str, to: &str, now: Ns) -> Option<(u64, bool)> {
        let fits = |id: u64, back: bool| -> bool {
            let Some(s) = self.sessions.get(&id) else {
                return false;
            };
            let (a, b) = if back { (s.to.as_str(), s.from.as_str()) } else { (s.from.as_str(), s.to.as_str()) };
            a == from && b == to && s.ends(&self.timers) > now
        };
        if f.later {
            // the first fragment had the ports: any session of these two addresses
            let lo = (f.proto, f.src, f.dst, 0u16, 0u16);
            let hi = (f.proto, f.src, f.dst, u16::MAX, u16::MAX);
            return self.index.range(lo..=hi).map(|(_, v)| *v).find(|(id, back)| fits(*id, *back));
        }
        let (id, back) = *self.index.get(&f.key())?;
        // the answer's direction takes answers: a query from there begins nothing
        if back && f.icmp_request() {
            return None;
        }
        fits(id, back).then_some((id, back))
    }
    /// the session an ICMP error is about: the quoted packet went the other way than the error
    fn related(&self, f: &FwFlow, from: &str, to: &str, now: Ns) -> Option<u64> {
        let q = f.quoted?;
        let (id, quoted_back) = *self.index.get(&q)?;
        let s = self.sessions.get(&id).filter(|s| s.ends(&self.timers) > now)?;
        let (a, b) = if quoted_back { (s.from.as_str(), s.to.as_str()) } else { (s.to.as_str(), s.from.as_str()) };
        // (the device itself tells about a packet it forwarded)
        ((a == from || from == SELF_ZONE) && b == to).then_some(id)
    }
    /// the condition holds for a packet
    fn holds(m: &Match, f: &FwFlow, acls: &Acls) -> bool {
        match m {
            Match::Protocol(name) => {
                let Some(p) = FW_PROTOCOLS.iter().find(|x| x.0 == name) else {
                    return false;
                };
                let proto = match p.1 {
                    0 => matches!(f.proto, PROTO_TCP | PROTO_UDP),
                    PROTO_ICMP => f.is_icmp(),
                    x => f.proto == x,
                };
                proto && (p.2 == 0 || (f.ports && f.dport == p.2))
            }
            Match::Access(l) => match (&f.v4, acls.lists.get(l).filter(|a| a.has_rules())) {
                (Some(v), Some(a)) => a.check(v).0.permit,
                _ => false,
            },
        }
    }
    /// a class selects a packet: a class without conditions selects nothing
    fn class_holds(&self, name: &str, f: &FwFlow, acls: &Acls) -> bool {
        let Some(c) = self.classes.get(name) else {
            return false;
        };
        if c.matches.is_empty() {
            return false;
        }
        if c.any { c.matches.iter().any(|m| Firewall::holds(m, f, acls)) } else { c.matches.iter().all(|m| Firewall::holds(m, f, acls)) }
    }
    /// what zones and sessions say about a packet from zone `from` to zone `to` (None: an interface
    /// without a zone), counting nothing and changing nothing
    pub fn probe(&self, from: Option<&str>, to: Option<&str>, f: &FwFlow, acls: &Acls, now: Ns) -> FwProbe {
        if self.host.enabled {
            return self.host_probe(from.unwrap_or(HOST_NET), to.unwrap_or(HOST_NET), f, now);
        }
        let (from, to) = match (from, to) {
            (None, None) => return FwProbe::NoZones,
            (Some(SELF_ZONE), None) | (None, Some(SELF_ZONE)) => return FwProbe::SelfPass { from: String::from(from.unwrap_or("-")), to: String::from(to.unwrap_or("-")) },
            (Some(z), None) | (None, Some(z)) => return FwProbe::Unzoned { zone: String::from(z) },
            (Some(a), Some(b)) if a == b => return FwProbe::SameZone(String::from(a)),
            (Some(a), Some(b)) => (a, b),
        };
        let pair_name = |id: u64| self.sessions.get(&id).map(|s| s.pair.clone()).unwrap_or_default();
        if let Some((id, back)) = self.find(f, from, to, now) {
            return FwProbe::Session { id, back, pair: pair_name(id) };
        }
        if let Some(id) = self.related(f, from, to, now) {
            return FwProbe::Related { id, pair: pair_name(id) };
        }
        let has_self = from == SELF_ZONE || to == SELF_ZONE;
        let Some((pname, pair)) = self.pair_of(from, to).filter(|(_, p)| p.policy.is_some()) else {
            if has_self {
                return FwProbe::SelfPass { from: String::from(from), to: String::from(to) };
            }
            return FwProbe::NoRule { from: String::from(from), to: String::from(to) };
        };
        let pol_name = pair.policy.clone().unwrap_or_default();
        let Some(policy) = self.policies.get(&pol_name) else {
            return FwProbe::NoRule { from: String::from(from), to: String::from(to) };
        };
        let at = policy.classes.iter().position(|c| self.class_holds(&c.class, f, acls));
        let c = at.and_then(|i| policy.classes.get(i)).unwrap_or(&policy.default);
        let action = c.action.unwrap_or(FwAction::Drop);
        if action == FwAction::Inspect {
            if !f.begins() {
                return FwProbe::NotNew { pair: pname.clone(), class: c.class.clone() };
            }
            if f.proto == PROTO_TCP && self.half_open(now) >= self.max_half_open {
                return FwProbe::Limit { pair: pname.clone(), class: c.class.clone(), max: self.max_half_open };
            }
        }
        FwProbe::Policy { pair: pname.clone(), policy: pol_name, class: c.class.clone(), at, action, log: c.log }
    }
    /// the host firewall: sessions first, then the rules in their order, then the default
    fn host_probe(&self, from: &str, to: &str, f: &FwFlow, now: Ns) -> FwProbe {
        if from == to {
            return FwProbe::SameZone(String::from(from));
        }
        if let Some((id, back)) = self.find(f, from, to, now) {
            return FwProbe::Session { id, back, pair: String::from(HOST_PAIR) };
        }
        if let Some(id) = self.related(f, from, to, now) {
            return FwProbe::Related { id, pair: String::from(HOST_PAIR) };
        }
        let out = from == SELF_ZONE;
        if !out {
            // what the original's own rules let in: ICMP errors, echo requests, a DHCP client's answers
            let icmp_ok = f.is_icmp() && f.icmp.is_some_and(|(t, _)| is_error_any(f.proto, t) || f.icmp_request());
            let dhcp = f.proto == PROTO_UDP && f.ports && f.sport == 67 && f.dport == 68;
            if icmp_ok || dhcp {
                return FwProbe::SelfPass { from: String::from(from), to: String::from(to) };
            }
        }
        let (class, action) = match self.host.rules.iter().find(|r| r.holds(f, out)) {
            Some(r) => (r.words(), r.action),
            None => (String::from(CLASS_DEFAULT), if out { self.host.default_out } else { self.host.default_in }),
        };
        // an allowed packet begins a session when it can; one that cannot passes alone
        let action = match action {
            FwAction::Inspect | FwAction::Pass if f.begins() => FwAction::Inspect,
            FwAction::Inspect | FwAction::Pass => FwAction::Pass,
            other => other,
        };
        if action == FwAction::Inspect && f.proto == PROTO_TCP && self.half_open(now) >= self.max_half_open {
            return FwProbe::Limit { pair: String::from(HOST_PAIR), class, max: self.max_half_open };
        }
        FwProbe::Policy { pair: String::from(HOST_PAIR), policy: String::from(HOST_PAIR), class, at: None, action, log: false }
    }
    /// a new session for a packet
    fn begin(&mut self, f: &FwFlow, from: &str, to: &str, pair: &str, class: &str, log: bool, now: Ns) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let key = f.key();
        let tcp = if f.proto == PROTO_TCP { Some(TcpState::Syn) } else { None };
        let s = FwSession { id, proto: key.0, src: key.1, sport: key.3, dst: key.2, dport: key.4, from: String::from(from), to: String::from(to), pair: String::from(pair), class: String::from(class), tcp, created: now, last: now, packets: [1, 0], bytes: [f.len as u64, 0], log, fin: [false; 2] };
        // a later session of the same addresses and ports takes the place of an older one
        for k in [s.key(false), s.key(true)] {
            if let Some((old, _)) = self.index.get(&k).copied() {
                self.remove(old);
            }
        }
        self.index.insert(s.key(false), (id, false));
        self.index.insert(s.key(true), (id, true));
        let end = s.ends(&self.timers);
        self.sessions.insert(id, s);
        self.stats.created += 1;
        self.due = Some(self.due.map_or(end, |d| d.min(end)));
        id
    }
    /// a packet of a session; false: a RST ended it
    fn touch(&mut self, id: u64, back: bool, f: &FwFlow, now: Ns) -> bool {
        self.stats.session_hits += 1;
        let Some(s) = self.sessions.get_mut(&id) else {
            return true;
        };
        let d = back as usize;
        s.last = now;
        s.packets[d] += 1;
        s.bytes[d] += f.len as u64;
        if let Some(st) = s.tcp {
            if f.flags & RST != 0 {
                self.stats.reset += 1;
                return false;
            }
            // a new connection on the same ports while the old one still winds down
            if !back && f.flags & SYN != 0 && f.flags & ACK == 0 && (s.fin[0] || s.fin[1]) {
                s.fin = [false; 2];
                s.tcp = Some(TcpState::Syn);
                let end = s.ends(&self.timers);
                self.due = Some(self.due.map_or(end, |d| d.min(end)));
                return true;
            }
            if f.flags & FIN != 0 {
                s.fin[d] = true;
            }
            s.tcp = Some(if s.fin[0] && s.fin[1] {
                TcpState::Closing
            } else if s.fin[0] || s.fin[1] {
                TcpState::Fin
            } else {
                match st {
                    TcpState::Syn if back && f.flags & SYN != 0 => TcpState::SynAck,
                    TcpState::SynAck if !back && f.flags & ACK != 0 && f.flags & SYN == 0 => TcpState::Established,
                    other => other,
                }
            });
        }
        // (a timer may have become shorter: the next look is no later than its end)
        let end = s.ends(&self.timers);
        self.due = Some(self.due.map_or(end, |d| d.min(end)));
        true
    }
    /// takes out what has ended at `now`
    fn sweep(&mut self, now: Ns) -> Vec<FwSession> {
        if self.due.is_none_or(|d| d > now) {
            return Vec::new();
        }
        let timers = self.timers;
        let gone: Vec<u64> = self.sessions.values().filter(|s| s.ends(&timers) <= now).map(|s| s.id).collect();
        let out: Vec<FwSession> = gone.iter().filter_map(|id| self.remove(*id)).collect();
        self.stats.expired += out.len() as u64;
        self.due = self.sessions.values().map(|s| s.ends(&timers)).min();
        out
    }
    /// after a change of the configuration: sessions of a pair that went or lost its policy go too
    fn prune(&mut self) {
        let gone: Vec<u64> = self.sessions.values().filter(|s| s.pair != HOST_PAIR && !self.pairs.get(&s.pair).is_some_and(|p| p.policy.is_some())).map(|s| s.id).collect();
        for id in gone {
            self.remove(id);
        }
    }
}

/// what Action::Fw does
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FwOp {
    /// "zone security NAME" (add or remove)
    Zone { name: String, add: bool },
    /// the zone's description (None removes it)
    ZoneDescr { name: String, text: Option<String> },
    /// "zone-member security NAME" of an interface (None: no zone)
    Member { iface: String, zone: Option<String> },
    /// "class-map type inspect [match-any] NAME"
    Class { name: String, any: bool, add: bool },
    /// a "match" line of a class
    ClassMatch { class: String, m: Match, add: bool },
    /// "policy-map type inspect NAME"
    Policy { name: String, add: bool },
    /// "class type inspect NAME" in a policy ("class-default" is always there)
    PolicyClass { policy: String, class: String, add: bool },
    /// the action of a class in a policy (None: no action named)
    PolicyAction { policy: String, class: String, action: Option<FwAction>, log: bool },
    /// "zone-pair security NAME source A destination B"
    Pair { name: String, src: String, dst: String },
    /// removes a pair
    PairGone { name: String },
    /// "service-policy type inspect NAME" of a pair (None takes it away)
    PairPolicy { name: String, policy: Option<String> },
    /// a timer in seconds (None: the default)
    Timer { which: FwTimer, secs: Option<u32> },
    /// the limit of half-open sessions (None: the default)
    MaxHalfOpen(Option<u32>),
    /// forgets sessions (of one pair)
    ClearSessions { pair: Option<String> },
    /// the counters back to 0
    ClearCounters,
    /// the firewall of a Linux host
    Host(HostOp),
}

/// a rule of the host firewall
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRule {
    pub action: FwAction,
    /// outgoing, else incoming
    pub out: bool,
    /// the port (destination; None: any)
    pub port: Option<u16>,
    /// TCP or UDP (None: both, or any protocol without a port)
    pub proto: Option<u8>,
    /// from these addresses (None: anywhere; IPv4 only)
    pub from: Option<Prefix4>,
}

impl HostRule {
    fn holds(&self, f: &FwFlow, out: bool) -> bool {
        if self.out != out {
            return false;
        }
        if self.proto.is_some_and(|p| p != f.proto) {
            return false;
        }
        if let Some(p) = self.port
            && !(matches!(f.proto, PROTO_TCP | PROTO_UDP) && f.ports && f.dport == p)
        {
            return false;
        }
        match (&self.from, f.src) {
            (Some(net), IpAddr::V4(a)) => net.contains(a),
            (Some(_), IpAddr::V6(_)) => false,
            (None, _) => true,
        }
    }
    /// the rule's words after the action ("80/tcp", "out 25/tcp", "from 10.0.0.0/8 port 22")
    pub fn words(&self) -> String {
        let mut s = String::new();
        if self.out {
            s.push_str("out ");
        }
        let port = match (self.port, self.proto) {
            (Some(p), Some(PROTO_TCP)) => format!("{}/tcp", p),
            (Some(p), Some(PROTO_UDP)) => format!("{}/udp", p),
            (Some(p), _) => format!("{}", p),
            (None, _) => String::new(),
        };
        match &self.from {
            Some(net) => {
                s.push_str(&format!("from {}", net));
                if !port.is_empty() {
                    s.push_str(&format!(" to any port {}", port));
                }
            }
            None => s.push_str(&port),
        }
        s
    }
}

/// the firewall of a Linux host (cli/ufw.rs)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostFw {
    pub enabled: bool,
    pub default_in: FwAction,
    pub default_out: FwAction,
    pub rules: Vec<HostRule>,
}

impl Default for HostFw {
    fn default() -> HostFw {
        HostFw { enabled: false, default_in: FwAction::Drop, default_out: FwAction::Inspect, rules: Vec::new() }
    }
}

/// what Action::Fw does to the host firewall
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostOp {
    Enable(bool),
    Default { out: bool, action: FwAction },
    Add(HostRule),
    Remove(HostRule),
    /// off, the defaults back, no rules, no sessions
    Reset,
}

/// a live session for the views
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FwRow {
    pub session: FwSession,
    /// when it ends without further traffic
    pub ends: Ns,
}

/// what the zones say about a packet that is not sent (the Firewall tab's Test)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FwTest {
    /// the zones it goes between (None: no zone; "self": the device)
    pub from: Option<String>,
    pub to: Option<String>,
    /// the packet as the firewall sees it, after NAT from outside
    pub flow: FwFlow,
    pub probe: FwProbe,
}

impl Engine {
    /// the firewall of a device (None without IP)
    pub fn firewall(&self, dev: u32) -> Option<&Firewall> {
        Some(&self.stack(self.node_index(dev)?)?.fw)
    }
    /// the firewall works on this node
    pub(crate) fn fw_on(&self, ni: usize) -> bool {
        self.stack(ni).is_some_and(|s| s.fw.active())
    }
    /// the live sessions of a device, the oldest first
    pub fn fw_sessions(&self, dev: u32) -> Vec<FwRow> {
        let Some(fw) = self.firewall(dev) else {
            return Vec::new();
        };
        let now = self.now;
        fw.sessions().filter_map(|s| {
            let ends = s.ends(&fw.timers);
            (ends > now).then(|| FwRow { session: s.clone(), ends })
        }).collect()
    }

    /// a packet for this device, after the inbound list and NAT; false: dropped
    pub(crate) fn fw_in(&mut self, ni: usize, iface: u16, pkt: &[u8], cause: Option<HopId>) -> bool {
        let Some(f) = FwFlow::of_packet(pkt) else {
            return true;
        };
        self.fw_pass(ni, Some(iface), None, &f, pkt, cause)
    }
    /// a packet on its way out of `out`, after routing and before NAT changes its source; `from`:
    /// where it came in (None: the device made it); false: dropped
    pub(crate) fn fw_out(&mut self, ni: usize, from: Option<u16>, out: u16, pkt: &[u8], cause: Option<HopId>) -> bool {
        let Some(f) = FwFlow::of_packet(pkt) else {
            return true;
        };
        self.fw_pass(ni, from, Some(out), &f, pkt, cause)
    }
    /// the same for IPv6
    pub(crate) fn fw_in6(&mut self, ni: usize, iface: u16, pkt: &[u8], cause: Option<HopId>) -> bool {
        let Some(f) = FwFlow::of_packet6(pkt) else {
            return true;
        };
        self.fw_pass(ni, Some(iface), None, &f, pkt, cause)
    }
    pub(crate) fn fw_out6(&mut self, ni: usize, from: Option<u16>, out: u16, pkt: &[u8], cause: Option<HopId>) -> bool {
        let Some(f) = FwFlow::of_packet6(pkt) else {
            return true;
        };
        self.fw_pass(ni, from, Some(out), &f, pkt, cause)
    }

    fn fw_note(&mut self, ni: usize, v6: bool, cause: Option<HopId>, n: FwNote) {
        let dev = self.dev_of(ni);
        if v6 {
            self.note_v6(dev, cause, Note::Fw(n));
        } else {
            self.note(dev, cause, Note::Fw(n));
        }
    }

    /// what has timed out goes first, with its notes
    fn fw_due(&mut self, ni: usize, cause: Option<HopId>) {
        let now = self.now;
        let Some(st) = self.stack_mut(ni) else {
            return;
        };
        for s in st.fw.sweep(now) {
            let v6 = matches!(s.src, IpAddr::V6(_));
            let what = s.text();
            if s.log {
                self.fw_log(ni, &format!("%FW-6-SESSION_END: session {} ({}) ended, {} packets there, {} back", s.id, what, s.packets[0], s.packets[1]), cause);
            }
            self.fw_note(ni, v6, cause, FwNote::Expired { id: s.id, what, proto: s.proto, v6 });
        }
    }

    /// a line of "log": the console, the device's log and with it its syslog hosts
    fn fw_log(&mut self, ni: usize, line: &str, cause: Option<HopId>) {
        self.ios_console_log(ni, line);
        let (tag, text) = line.split_once(": ").unwrap_or(("", line));
        self.os_log(ni, 6, tag, text, cause);
    }

    /// the zones and sessions decide about a packet: `from` and `to` are the interfaces of its way
    /// (None: the device itself); false: dropped, noted, and with "reject" its sender told
    fn fw_pass(&mut self, ni: usize, from: Option<u16>, to: Option<u16>, f: &FwFlow, pkt: &[u8], cause: Option<HopId>) -> bool {
        self.fw_due(ni, cause);
        let now = self.now;
        let v6 = f.v4.is_none();
        let (proto, what) = (f.proto, f.text());
        let Some(st) = self.stack(ni) else {
            return true;
        };
        let fz = st.fw.zone_of(from).map(String::from);
        let tz = st.fw.zone_of(to).map(String::from);
        let probe = st.fw.probe(fz.as_deref(), tz.as_deref(), f, &st.acls, now);
        let dash = || String::from("-");
        match &probe {
            FwProbe::NoZones => return true,
            FwProbe::SameZone(zone) => self.fw_note(ni, v6, cause, FwNote::SameZone { zone: zone.clone(), what, proto, v6 }),
            FwProbe::SelfPass { from: a, to: b } => self.fw_note(ni, v6, cause, FwNote::SelfPass { from: a.clone(), to: b.clone(), what, proto, v6 }),
            FwProbe::Unzoned { zone } => {
                if let Some(st) = self.stack_mut(ni) {
                    st.fw.stats.no_rule += 1;
                }
                self.fw_note(ni, v6, cause, FwNote::Unzoned { zone: zone.clone(), what, proto, v6 });
            }
            FwProbe::NoRule { from: a, to: b } => {
                if let Some(st) = self.stack_mut(ni) {
                    st.fw.stats.no_rule += 1;
                }
                self.fw_note(ni, v6, cause, FwNote::NoRule { from: a.clone(), to: b.clone(), what, proto, v6 });
            }
            FwProbe::Session { id, back, .. } => {
                let alive = self.stack_mut(ni).is_some_and(|st| st.fw.touch(*id, *back, f, now));
                if alive {
                    self.fw_note(ni, v6, cause, FwNote::Session { id: *id, back: *back, what, proto, v6 });
                } else {
                    let s = self.stack_mut(ni).and_then(|st| st.fw.remove(*id));
                    if s.as_ref().is_some_and(|s| s.log) {
                        self.fw_log(ni, &format!("%FW-6-SESSION_END: session {} ({}) ended by a reset", id, what), cause);
                    }
                    self.fw_note(ni, v6, cause, FwNote::Reset { id: *id, what, proto, v6 });
                }
            }
            FwProbe::Related { id, .. } => self.fw_note(ni, v6, cause, FwNote::Related { id: *id, what, proto, v6 }),
            FwProbe::NotNew { pair, .. } => {
                if let Some(st) = self.stack_mut(ni) {
                    st.fw.stats.not_new += 1;
                }
                self.fw_note(ni, v6, cause, FwNote::NotNew { pair: pair.clone(), what, proto, v6 });
            }
            FwProbe::Limit { pair, max, .. } => {
                if let Some(st) = self.stack_mut(ni) {
                    st.fw.stats.limited += 1;
                }
                self.fw_note(ni, v6, cause, FwNote::Limit { pair: pair.clone(), max: *max, what, proto, v6 });
            }
            FwProbe::Policy { pair, policy, class, at, action, log } => {
                // the class counts the packet, and an access list it read counts it as well
                let mut report: Option<u64> = None;
                if let Some(st) = self.stack_mut(ni) {
                    let conds = st.fw.classes.get(class).map(|c| c.matches.clone()).unwrap_or_default();
                    if let Some(p) = st.fw.policies.get_mut(policy) {
                        let c = match *at {
                            Some(i) => p.classes.get_mut(i),
                            None => Some(&mut p.default),
                        };
                        if let Some(c) = c {
                            c.hits += 1;
                            if *log && *action != FwAction::Inspect {
                                c.unlogged += 1;
                                if c.logged.is_none_or(|t| now >= t + LOG_EVERY) {
                                    report = Some(c.unlogged);
                                    c.logged = Some(now);
                                    c.unlogged = 0;
                                }
                            }
                        }
                    }
                    if let (Some(v), true) = (&f.v4, at.is_some()) {
                        for m in &conds {
                            if let Match::Access(l) = m
                                && let Some(a) = st.acls.lists.get_mut(l)
                                && let (verdict, Some(line)) = a.check(v)
                                && verdict.permit
                            {
                                a.lines[line].hits += 1;
                            }
                        }
                    }
                }
                match action {
                    FwAction::Inspect => {
                        let (a, b) = (fz.clone().unwrap_or_else(dash), tz.clone().unwrap_or_else(dash));
                        let id = self.stack_mut(ni).map_or(0, |st| st.fw.begin(f, &a, &b, pair, class, *log, now));
                        if *log {
                            self.fw_log(ni, &format!("%FW-6-SESSION_START: session {} ({}) started, {} from {} to {}", id, what, pair, a, b), cause);
                        }
                        self.fw_note(ni, v6, cause, FwNote::NewSession { id, pair: pair.clone(), class: class.clone(), what, proto, v6 });
                    }
                    FwAction::Pass => {
                        if let Some(n) = report {
                            self.fw_log(ni, &format!("%FW-6-PASS: {} lets {} through ({} packets)", class, what, n), cause);
                        }
                        self.fw_note(ni, v6, cause, FwNote::Pass { pair: pair.clone(), class: class.clone(), what, proto, v6 });
                    }
                    FwAction::Drop => {
                        if let Some(n) = report {
                            self.fw_log(ni, &format!("%FW-6-DROP: {} drops {} ({} packets)", class, what, n), cause);
                        }
                        self.fw_note(ni, v6, cause, FwNote::Drop { pair: pair.clone(), class: class.clone(), what, proto, v6 });
                    }
                    FwAction::Reject => {
                        if let Some(n) = report {
                            self.fw_log(ni, &format!("%FW-6-REJECT: {} rejects {} ({} packets)", class, what, n), cause);
                        }
                        self.fw_note(ni, v6, cause, FwNote::Reject { pair: pair.clone(), class: class.clone(), what, proto, v6 });
                        if let Some(inif) = from {
                            if v6 {
                                self.icmp6_error(ni, inif, pkt, Icmp6::DestUnreach { code: icmpv6::UNREACH_ADMIN }, cause);
                            } else if to.is_some() {
                                self.icmp_error(ni, inif, &crate::l3::as_received(pkt), Icmp::Unreachable { code: UNREACH_ADMIN, mtu: 0 }, cause);
                            } else {
                                self.icmp_error(ni, inif, pkt, Icmp::Unreachable { code: UNREACH_ADMIN, mtu: 0 }, cause);
                            }
                        }
                    }
                }
            }
        }
        probe.passes()
    }

    /// what the zones say about a packet that is not sent: `iface` and `out` as the Test part asks
    /// (the other interface of its way follows from the routes), the packet as it arrives, so NAT
    /// from outside first (fw.rs: the order)
    pub fn fw_test(&self, dev: u32, iface: u16, out: bool, fl: &Flow) -> Option<FwTest> {
        let ni = self.node_index(dev)?;
        let st = self.stack(ni)?;
        let mut flow = *fl;
        if !out && self.nat_part(ni).is_some_and(|n| n.side(iface) == Some(crate::nat::Side::Outside)) && st.forwarding {
            if let Some((a, p)) = self.nat_peek_in(ni, fl) {
                flow.dst = a;
                if let Some(p) = p {
                    flow.dport = p;
                }
            }
        }
        let local = |a: Ipv4| st.is_mine(a) || a.is_broadcast() || a.is_multicast();
        let (a, b): (Option<u16>, Option<u16>) = if out {
            let from = if st.is_mine(flow.src) { None } else { self.route(ni, flow.src).map(|n| n.iface) };
            (from, Some(iface))
        } else if local(flow.dst) {
            (Some(iface), None)
        } else {
            (Some(iface), self.route(ni, flow.dst).map(|n| n.iface))
        };
        let from = st.fw.zone_of(a).map(String::from);
        let to = st.fw.zone_of(b).map(String::from);
        let f = FwFlow::of_v4(&flow, 0, None);
        let probe = st.fw.probe(from.as_deref(), to.as_deref(), &f, &st.acls, self.now);
        Some(FwTest { from, to, flow: f, probe })
    }

    /// the Action::Fw changes
    pub(crate) fn fw_apply(&mut self, dev: u32, op: FwOp) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if let FwOp::Host(h) = op {
            if !matches!(self.nodes.get(ni), Some(Some(n)) if n.model.cli == jfn_model::catalog::CliStyle::Linux) {
                return Err(ActErr::Refused(msg!("Diese Firewall gibt es nur auf Linux-Geräten.")));
            }
            let fw = &mut self.stack_mut(ni).ok_or(ActErr::NoIp(dev))?.fw;
            match h {
                HostOp::Enable(on) => {
                    fw.host.enabled = on;
                    if !on {
                        fw.clear_sessions(None);
                    }
                }
                HostOp::Default { out, action } => {
                    if out {
                        fw.host.default_out = action;
                    } else {
                        fw.host.default_in = action;
                    }
                }
                HostOp::Add(r) => {
                    if !fw.host.rules.contains(&r) {
                        fw.host.rules.push(r);
                    }
                }
                HostOp::Remove(r) => fw.host.rules.retain(|x| *x != r),
                HostOp::Reset => {
                    fw.host = HostFw::default();
                    fw.clear_sessions(None);
                }
            }
            return Ok(());
        }
        if !matches!(self.nodes.get(ni), Some(Some(n)) if n.model.has(jfn_model::catalog::F_FIREWALL)) {
            return Err(ActErr::Refused(msg!("Zonen gibt es nur auf Firewall-Modellen.")));
        }
        if let FwOp::Member { iface, zone } = &op {
            let i = self.stack(ni).ok_or(ActErr::NoIp(dev))?.iface_by_name(iface).ok_or(ActErr::UnknownIface { dev, iface: iface.clone() })?;
            let st = self.stack_mut(ni).ok_or(ActErr::NoIp(dev))?;
            return match zone {
                Some(z) if !st.fw.zones.contains_key(z) => Err(ActErr::Refused(msg!("Die Zone {} gibt es nicht.", z))),
                Some(z) => {
                    st.fw.members.insert(i, z.clone());
                    Ok(())
                }
                None => {
                    st.fw.members.remove(&i);
                    Ok(())
                }
            };
        }
        let st = self.stack_mut(ni).ok_or(ActErr::NoIp(dev))?;
        let fw = &mut st.fw;
        match op {
            FwOp::Member { .. } | FwOp::Host(_) => Ok(()),
            FwOp::Zone { name, add: true } => {
                if name.eq_ignore_ascii_case(SELF_ZONE) {
                    return Err(ActErr::Refused(msg!("Der Name self gehört dem Gerät selbst.")));
                }
                fw.zones.entry(name).or_default();
                Ok(())
            }
            FwOp::Zone { name, add: false } => {
                if fw.members.values().any(|z| *z == name) {
                    return Err(ActErr::Refused(msg!("Die Zone {} hat noch Schnittstellen.", name)));
                }
                if fw.pairs.values().any(|p| p.src == name || p.dst == name) {
                    return Err(ActErr::Refused(msg!("Die Zone {} steht noch in einem Zonenpaar.", name)));
                }
                if fw.zones.remove(&name).is_none() {
                    return Err(ActErr::Refused(msg!("Die Zone {} gibt es nicht.", name)));
                }
                Ok(())
            }
            FwOp::ZoneDescr { name, text } => {
                let Some(z) = fw.zones.get_mut(&name) else {
                    return Err(ActErr::Refused(msg!("Die Zone {} gibt es nicht.", name)));
                };
                *z = text.unwrap_or_default();
                Ok(())
            }
            FwOp::Class { name, any, add: true } => {
                fw.classes.entry(name).or_default().any = any;
                Ok(())
            }
            FwOp::Class { name, add: false, .. } => {
                if fw.policies.values().any(|p| p.classes.iter().any(|c| c.class == name)) {
                    return Err(ActErr::Refused(msg!("Die Klasse {} steht noch in einer Richtlinie.", name)));
                }
                if fw.classes.remove(&name).is_none() {
                    return Err(ActErr::Refused(msg!("Die Klasse {} gibt es nicht.", name)));
                }
                Ok(())
            }
            FwOp::ClassMatch { class, m, add } => {
                let Some(c) = fw.classes.get_mut(&class) else {
                    return Err(ActErr::Refused(msg!("Die Klasse {} gibt es nicht.", class)));
                };
                if add {
                    if !c.matches.contains(&m) {
                        c.matches.push(m);
                    }
                } else {
                    c.matches.retain(|x| *x != m);
                }
                Ok(())
            }
            FwOp::Policy { name, add: true } => {
                fw.policies.entry(name).or_default();
                Ok(())
            }
            FwOp::Policy { name, add: false } => {
                if fw.pairs.values().any(|p| p.policy.as_deref() == Some(name.as_str())) {
                    return Err(ActErr::Refused(msg!("Die Richtlinie {} gilt noch für ein Zonenpaar.", name)));
                }
                if fw.policies.remove(&name).is_none() {
                    return Err(ActErr::Refused(msg!("Die Richtlinie {} gibt es nicht.", name)));
                }
                Ok(())
            }
            FwOp::PolicyClass { policy, class, add } => {
                if class == CLASS_DEFAULT {
                    return Ok(());
                }
                if add && !fw.classes.contains_key(&class) {
                    return Err(ActErr::Refused(msg!("Die Klasse {} gibt es nicht.", class)));
                }
                let Some(p) = fw.policies.get_mut(&policy) else {
                    return Err(ActErr::Refused(msg!("Die Richtlinie {} gibt es nicht.", policy)));
                };
                if add {
                    if !p.classes.iter().any(|c| c.class == class) {
                        p.classes.push(PolicyClass::new(&class, None));
                    }
                } else {
                    p.classes.retain(|c| c.class != class);
                }
                Ok(())
            }
            FwOp::PolicyAction { policy, class, action, log } => {
                let Some(p) = fw.policies.get_mut(&policy) else {
                    return Err(ActErr::Refused(msg!("Die Richtlinie {} gibt es nicht.", policy)));
                };
                let c = if class == CLASS_DEFAULT { Some(&mut p.default) } else { p.classes.iter_mut().find(|c| c.class == class) };
                let Some(c) = c else {
                    return Err(ActErr::Refused(msg!("Die Klasse {} gibt es nicht.", class)));
                };
                c.action = if class == CLASS_DEFAULT { action.or(Some(FwAction::Drop)) } else { action };
                c.log = log;
                Ok(())
            }
            FwOp::Pair { name, src, dst } => {
                let known = |fw: &Firewall, z: &str| z == SELF_ZONE || fw.zones.contains_key(z);
                if !known(fw, &src) {
                    return Err(ActErr::Refused(msg!("Die Zone {} gibt es nicht.", src)));
                }
                if !known(fw, &dst) {
                    return Err(ActErr::Refused(msg!("Die Zone {} gibt es nicht.", dst)));
                }
                if src == dst {
                    return Err(ActErr::Refused(msg!("Ein Zonenpaar verbindet zwei verschiedene Zonen.")));
                }
                if fw.pair_of(&src, &dst).is_some_and(|(n, _)| *n != name) {
                    return Err(ActErr::Refused(msg!("Von {} nach {} gibt es schon ein Zonenpaar.", src, dst)));
                }
                let p = fw.pairs.entry(name).or_default();
                p.src = src;
                p.dst = dst;
                Ok(())
            }
            FwOp::PairGone { name } => {
                if fw.pairs.remove(&name).is_none() {
                    return Err(ActErr::Refused(msg!("Das Zonenpaar {} gibt es nicht.", name)));
                }
                fw.prune();
                Ok(())
            }
            FwOp::PairPolicy { name, policy } => {
                if let Some(pol) = &policy
                    && !fw.policies.contains_key(pol)
                {
                    return Err(ActErr::Refused(msg!("Die Richtlinie {} gibt es nicht.", pol)));
                }
                let Some(p) = fw.pairs.get_mut(&name) else {
                    return Err(ActErr::Refused(msg!("Das Zonenpaar {} gibt es nicht.", name)));
                };
                p.policy = policy;
                fw.prune();
                Ok(())
            }
            FwOp::Timer { which, secs } => {
                fw.timers.set(which, secs.unwrap_or(which.default_secs()));
                // (a shorter timer is due sooner than the last look thought)
                fw.due = fw.sessions.values().map(|s| s.ends(&fw.timers)).min();
                Ok(())
            }
            FwOp::MaxHalfOpen(n) => {
                fw.max_half_open = n.unwrap_or(HALF_OPEN_MAX);
                Ok(())
            }
            FwOp::ClearSessions { pair } => {
                fw.clear_sessions(pair.as_deref());
                Ok(())
            }
            FwOp::ClearCounters => {
                fw.clear_counters();
                Ok(())
            }
        }
    }
}
