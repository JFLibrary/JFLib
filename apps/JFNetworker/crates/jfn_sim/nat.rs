// nat -- network address translation of IOS-like devices (RFC 3022): interfaces on the inside
// and the outside, dynamic translation from an address pool or of many hosts behind one
// address with ports (PAT, "overload"), static translations of addresses and of ports; TCP,
// UDP and ICMP queries, ICMP errors with the packet they quote (RFC 5508), fragments,
// timeouts
//
//   Nat { sides, pools, rules, timeouts, stats, .. }   the configuration and the translations
//                                    of a device (IpStack::nat)
//   Rule::List { acl, to, overload } / Rule::Static { proto, local, lport, global, gport }
//   Engine::nat_in(..)               a packet that arrives on an outside interface (l3.rs):
//                                    its destination translated back, then forwarded
//   Engine::nat_out(..)              a packet on its way out (l3.rs, after routing): the source
//                                    of what goes from inside to outside, the quote of ICMP
//                                    errors that leave through an outside interface
//   Engine::nat_arp(..)              global addresses an outside interface answers ARP for
//   Engine::nat_translations(dev) / nat(dev)   rows of "show ip nat translations", the state
//   SLOT_NAT                         the node timer: translations and fragments that time out
//
// The order of a router's decisions, as the original makes them: an arriving packet meets the
// inbound access list of its interface first (acl.rs).  If it came in on an outside interface
// and its destination is a global address with a translation, the destination becomes the
// inside address now, before the route is looked up, so the inside address decides the way;
// the TTL is checked on the packet as it arrived.  Then routing.  A packet from an inside
// interface that leaves through an outside one gets its source translated after the route was
// found, and only then meets the outbound access list of that interface, which therefore sees
// the global source (and, coming in, the inside destination).  ICMP answers the router sends
// about such a packet quote it as it arrived.  What the router sends itself is not
// translated, except the quote of its own ICMP errors about packets translated on the way in.
//
// Behaviour (RFC 4787, 5382, 5508): a mapping belongs to the inside endpoint (address and port,
// or ICMP identifier) and is the same for every destination (endpoint-independent mapping).
// The global port keeps the inside port's number when it is free, else the next free one of
// the same parity in the same range (1 to 1023, 1024 to 65535), counting up and starting over
// at the range's beginning; ICMP identifiers likewise over 0 to 65535; free means used by no
// other mapping, by no static rule and, on an address of the device itself, by none of its
// own sockets.  A pool with "overload" gives a host the address it already uses when it can
// (paired pooling), else the first address with a free port; a pool without it gives each host
// the first free address of its own (basic NAT).  From outside, a static translation and an
// address of basic NAT take packets from anyone; a mapping with ports only from addresses the
// inside host has sent to (address-dependent filtering).  An ICMP query mapping takes the
// replies only.  Every flow is a session with its own timer, and a dynamic mapping with ports
// ends with its last session: UDP 300 s (DNS to port 53: 60 s), ICMP 60 s, TCP 24 h once the
// handshake was seen in both directions, 240 s before that and after a RST or FIN from both
// ends; an address of basic NAT stays for "ip nat translation timeout" (24 h) after its last
// packet.  ICMP errors refresh nothing.  A new translation that finds no address or port is
// refused with "administratively prohibited" to the sender (RFC 5508 REQ-8).  An ICMP error
// about a translated packet gets the quote translated back with its checksums; from inside,
// its source becomes the global address of a host with basic NAT or the device's outside
// address; one about nothing NAT knows is dropped (RFC 5508 REQ-4 / REQ-5).
//
// Fragments: the first fragment decides for its datagram (it has the ports); later ones that
// arrive before it wait up to 30 s and follow it then, in order.  Not done here: hairpinning
// (an inside host reaching another one by its global address), "ip nat outside source",
// route maps and the application helpers (FTP and the like).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::msg;
use netpkt::icmp::{self, Icmp, UNREACH_ADMIN};
use netpkt::ipv4::{self, Ipv4View, PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use netpkt::nat::{self as rw, Flow, Side as RwSide};
use netpkt::tcp::{FIN, RST, SYN};
use netpkt::{Ipv4, Prefix4};

use crate::action::ActErr;
use crate::ip::{Origin, REASM_TIME};
use crate::kernel::{Engine, Ns, SEC};
use crate::trace::{HopId, Note};

/// the node timer of NAT
pub const SLOT_NAT: u32 = 14;
/// fragments that wait for the first one of their datagram, at most per datagram
const FRAG_HOLD: usize = 64;
/// datagrams whose fragments NAT follows at once, at most
const FRAG_SETS: usize = 1024;

/// the side of an interface
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Inside,
    Outside,
}

/// an address pool ("ip nat pool NAME START END netmask M")
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pool {
    pub start: Ipv4,
    pub end: Ipv4,
    /// the prefix length of its network
    pub len: u8,
}

impl Pool {
    pub fn contains(&self, a: Ipv4) -> bool {
        self.start.to_u32() <= a.to_u32() && a.to_u32() <= self.end.to_u32()
    }
    pub fn size(&self) -> u32 {
        self.end.to_u32().saturating_sub(self.start.to_u32()) + 1
    }
}

/// where a dynamic rule takes its global addresses from
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Pool(String),
    /// the address of an interface (with "overload")
    Iface(String),
}

/// the global side of a static rule
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Global {
    Addr(Ipv4),
    /// the address the interface has at the time
    Iface(String),
}

/// a rule of inside source translation
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    /// "ip nat inside source list L pool P [overload]", "... interface I overload"
    List { acl: String, to: Target, overload: bool },
    /// "ip nat inside source static [tcp|udp] LOCAL [PORT] GLOBAL|interface I [PORT]": proto 0
    /// translates the address with every protocol and port
    Static { proto: u8, local: Ipv4, lport: u16, global: Global, gport: u16, extendable: bool },
}

/// which timeout ("ip nat translation ...")
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tmo {
    /// "timeout": addresses of basic NAT
    Generic,
    Udp,
    Tcp,
    Icmp,
    Dns,
    Finrst,
    Syn,
}

impl Tmo {
    pub const ALL: [Tmo; 7] = [Tmo::Generic, Tmo::Udp, Tmo::Dns, Tmo::Tcp, Tmo::Finrst, Tmo::Syn, Tmo::Icmp];
    /// the configuration word
    pub fn word(self) -> &'static str {
        match self {
            Tmo::Generic => "timeout",
            Tmo::Udp => "udp-timeout",
            Tmo::Tcp => "tcp-timeout",
            Tmo::Icmp => "icmp-timeout",
            Tmo::Dns => "dns-timeout",
            Tmo::Finrst => "finrst-timeout",
            Tmo::Syn => "syn-timeout",
        }
    }
    /// the default in seconds (RFC 4787 REQ-5, RFC 5382 REQ-5, RFC 5508 REQ-1)
    pub fn default_secs(self) -> u32 {
        match self {
            Tmo::Generic | Tmo::Tcp => 86_400,
            Tmo::Udp => 300,
            Tmo::Icmp | Tmo::Dns => 60,
            Tmo::Finrst | Tmo::Syn => 240,
        }
    }
}

/// the timeouts in seconds (NEVER: no timeout)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    pub secs: [u32; 7],
}

pub const NEVER: u32 = u32::MAX;

impl Default for Timeouts {
    fn default() -> Timeouts {
        let mut secs = [0; 7];
        for (i, t) in Tmo::ALL.iter().enumerate() {
            secs[i] = t.default_secs();
        }
        Timeouts { secs }
    }
}

impl Timeouts {
    pub fn get(&self, t: Tmo) -> u32 {
        self.secs[Tmo::ALL.iter().position(|x| *x == t).unwrap_or(0)]
    }
    pub fn set(&mut self, t: Tmo, v: u32) {
        if let Some(i) = Tmo::ALL.iter().position(|x| *x == t) {
            self.secs[i] = v;
        }
    }
    fn ns(&self, t: Tmo) -> Option<Ns> {
        let s = self.get(t);
        if s == NEVER { None } else { Some(s as Ns * SEC) }
    }
}

/// a dynamic mapping: an inside endpoint and its global one (proto 0: the address alone)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub proto: u8,
    pub local: (Ipv4, u16),
    pub global: (Ipv4, u16),
    /// the rule it came from (Nat::rules ids)
    pub rule: u32,
    pub created: Ns,
    pub last: Ns,
    pub sessions: u32,
}

/// what NAT saw of a TCP connection (index 0: from inside, 1: from outside)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TcpSeen {
    pub syn: [bool; 2],
    pub fin: [bool; 2],
    pub rst: bool,
    pub established: bool,
}

/// one translated flow ("extended" entry)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub id: u64,
    pub proto: u8,
    pub local: (Ipv4, u16),
    pub global: (Ipv4, u16),
    /// the other end; an ICMP query's identifier as the remote end sees it
    pub remote: (Ipv4, u16),
    /// the dynamic mapping it uses (None: a static rule)
    pub map: Option<u64>,
    pub created: Ns,
    pub last: Ns,
    pub tcp: TcpSeen,
}

/// the counters of "show ip nat statistics"
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NatStats {
    /// packets translated by an existing session
    pub hits: u64,
    /// packets that needed a new session
    pub misses: u64,
    /// translations that timed out
    pub expired: u64,
    /// new translations refused for want of an address or port
    pub failed: u64,
}

/// what the first fragment decided for its datagram
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FragTo {
    /// the address of this side changes
    To(Ipv4),
    Unchanged,
    Dropped,
}

/// the fragments of one datagram
#[derive(Clone, Debug)]
struct FragSet {
    to: Option<FragTo>,
    /// fragments waiting for the first one: the packet, its frame, where it came in
    held: Vec<(Vec<u8>, Option<HopId>, u16)>,
    until: Ns,
}

/// the NAT part of a device: configuration and translations
#[derive(Clone, Debug, Default)]
pub struct Nat {
    /// "ip nat inside" / "ip nat outside" by interface
    pub sides: BTreeMap<u16, Side>,
    pub pools: BTreeMap<String, Pool>,
    /// the rules with their ids, in the order they were made
    pub rules: Vec<(u32, Rule)>,
    pub timeouts: Timeouts,
    pub stats: NatStats,
    pub(crate) maps: BTreeMap<u64, Mapping>,
    map_local: BTreeMap<(u8, Ipv4, u16), u64>,
    map_global: BTreeMap<(u8, Ipv4, u16), u64>,
    pub(crate) sessions: BTreeMap<u64, Session>,
    /// (proto, local address, local port, remote address, remote port)
    sess_in: BTreeMap<(u8, Ipv4, u16, Ipv4, u16), u64>,
    /// (proto, global address, global port, remote address, remote port)
    sess_out: BTreeMap<(u8, Ipv4, u16, Ipv4, u16), u64>,
    /// (outbound, source, destination, protocol, identification)
    frags: BTreeMap<(bool, Ipv4, Ipv4, u8, u16), FragSet>,
    next_id: u64,
    next_rule: u32,
    armed: Option<Ns>,
}

/// the remote port of a session key: ICMP sessions are found without it
fn rkey(proto: u8, p: u16) -> u16 {
    if proto == PROTO_ICMP { 0 } else { p }
}

/// a flow that has ports (TCP, UDP) or an identifier (an ICMP query)
fn port_flow(f: &Flow) -> bool {
    f.ports && matches!(f.proto, PROTO_TCP | PROTO_UDP | PROTO_ICMP)
}

/// an ICMP query request (an answer comes back with its identifier)
fn icmp_request(f: &Flow) -> bool {
    f.icmp.is_some_and(|(t, _)| rw::reply_of(t).is_some())
}

fn is_icmp_error(f: &Flow) -> bool {
    f.proto == PROTO_ICMP && f.icmp.is_some_and(|(t, _)| matches!(t, icmp::T_UNREACH | icmp::T_TIME_EXCEEDED | icmp::T_PARAM_PROBLEM | icmp::T_REDIRECT | 4))
}

impl Nat {
    /// something to do: an interface has a side
    pub fn active(&self) -> bool {
        !self.sides.is_empty()
    }
    pub fn side(&self, iface: u16) -> Option<Side> {
        self.sides.get(&iface).copied()
    }
    /// forgets every translation (the device was switched off; the configuration stays)
    pub fn reset(&mut self) {
        self.maps.clear();
        self.map_local.clear();
        self.map_global.clear();
        self.sessions.clear();
        self.sess_in.clear();
        self.sess_out.clear();
        self.frags.clear();
        self.armed = None;
    }
    /// an interface went away
    pub fn unbind(&mut self, iface: u16) {
        self.sides.remove(&iface);
    }
    /// adds a rule (an equal one is kept); Err with the rule it clashes with
    pub fn add_rule(&mut self, r: Rule) -> Result<(), Rule> {
        if self.rules.iter().any(|(_, x)| *x == r) {
            return Ok(());
        }
        if let Rule::Static { proto, local, lport, .. } = &r
            && let Some((_, x)) = self.rules.iter().find(|(_, x)| matches!(x, Rule::Static { proto: p, local: l, lport: lp, .. } if p == proto && l == local && lp == lport))
        {
            return Err(x.clone());
        }
        self.next_rule += 1;
        self.rules.push((self.next_rule, r));
        Ok(())
    }
    /// removes a rule: a static one by protocol, inside address and port, a dynamic one by its
    /// access list; false when there is none
    pub fn remove_rule(&mut self, r: &Rule) -> bool {
        let n = self.rules.len();
        self.rules.retain(|(_, x)| match (x, r) {
            (Rule::Static { proto: a, local: b, lport: c, .. }, Rule::Static { proto: d, local: e, lport: f, .. }) => !(a == d && b == e && c == f),
            (Rule::List { acl: a, .. }, Rule::List { acl: b, .. }) => a != b,
            _ => true,
        });
        self.rules.len() != n
    }
    /// forgets every dynamic translation and session ("clear ip nat translation *")
    pub fn clear(&mut self) {
        let stats = self.stats;
        self.reset();
        self.stats = stats;
    }
    /// when a session ends without traffic (None: never)
    pub fn session_end(&self, s: &Session) -> Option<Ns> {
        let t = &self.timeouts;
        let span = match s.proto {
            PROTO_UDP if s.remote.1 == 53 => t.ns(Tmo::Dns),
            PROTO_UDP => t.ns(Tmo::Udp),
            PROTO_ICMP => t.ns(Tmo::Icmp),
            PROTO_TCP if s.tcp.rst || (s.tcp.fin[0] && s.tcp.fin[1]) => t.ns(Tmo::Finrst),
            PROTO_TCP if s.tcp.established => t.ns(Tmo::Tcp),
            PROTO_TCP => t.ns(Tmo::Syn),
            _ => t.ns(Tmo::Generic),
        }?;
        Some(s.last.saturating_add(span))
    }
    /// when a dynamic mapping of basic NAT ends (None: it has sessions, or never)
    pub fn mapping_end(&self, m: &Mapping) -> Option<Ns> {
        if m.proto != 0 || m.sessions > 0 {
            return None;
        }
        Some(m.last.saturating_add(self.timeouts.ns(Tmo::Generic)?))
    }
    /// the earliest time something times out
    fn next_due(&self) -> Option<Ns> {
        let s = self.sessions.values().filter_map(|s| self.session_end(s));
        let m = self.maps.values().filter_map(|m| self.mapping_end(m));
        let f = self.frags.values().map(|f| f.until);
        s.chain(m).chain(f).min()
    }
    fn session_by_in(&self, proto: u8, local: (Ipv4, u16), remote: (Ipv4, u16)) -> Option<u64> {
        self.sess_in.get(&(proto, local.0, local.1, remote.0, rkey(proto, remote.1))).copied()
    }
    fn session_by_out(&self, proto: u8, global: (Ipv4, u16), remote: (Ipv4, u16)) -> Option<u64> {
        self.sess_out.get(&(proto, global.0, global.1, remote.0, rkey(proto, remote.1))).copied()
    }
    /// a session of the mapping's global endpoint talks to this remote address
    fn talks_to(&self, proto: u8, global: (Ipv4, u16), remote: Ipv4) -> bool {
        self.sess_out.range((proto, global.0, global.1, remote, 0)..=(proto, global.0, global.1, remote, u16::MAX)).next().is_some()
    }
    fn add_mapping(&mut self, m: Mapping) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.map_local.insert((m.proto, m.local.0, m.local.1), id);
        self.map_global.insert((m.proto, m.global.0, m.global.1), id);
        self.maps.insert(id, m);
        id
    }
    fn drop_mapping(&mut self, id: u64) -> Option<Mapping> {
        let m = self.maps.remove(&id)?;
        self.map_local.remove(&(m.proto, m.local.0, m.local.1));
        self.map_global.remove(&(m.proto, m.global.0, m.global.1));
        Some(m)
    }
    fn add_session(&mut self, mut s: Session) -> u64 {
        self.next_id += 1;
        s.id = self.next_id;
        self.sess_in.insert((s.proto, s.local.0, s.local.1, s.remote.0, rkey(s.proto, s.remote.1)), s.id);
        self.sess_out.insert((s.proto, s.global.0, s.global.1, s.remote.0, rkey(s.proto, s.remote.1)), s.id);
        if let Some(m) = s.map.and_then(|m| self.maps.get_mut(&m)) {
            m.sessions += 1;
        }
        let id = s.id;
        self.sessions.insert(id, s);
        id
    }
    fn drop_session(&mut self, id: u64) -> Option<Session> {
        let s = self.sessions.remove(&id)?;
        self.sess_in.remove(&(s.proto, s.local.0, s.local.1, s.remote.0, rkey(s.proto, s.remote.1)));
        self.sess_out.remove(&(s.proto, s.global.0, s.global.1, s.remote.0, rkey(s.proto, s.remote.1)));
        if let Some(mid) = s.map {
            let gone = match self.maps.get_mut(&mid) {
                Some(m) => {
                    m.sessions = m.sessions.saturating_sub(1);
                    m.sessions == 0 && m.proto != 0
                }
                None => false,
            };
            // a mapping with ports lives as long as its sessions
            if gone {
                self.drop_mapping(mid);
            }
        }
        Some(s)
    }
    /// the global address is in use by any dynamic mapping
    fn global_used(&self, a: Ipv4) -> bool {
        [0u8, PROTO_ICMP, PROTO_TCP, PROTO_UDP].iter().any(|&p| self.map_global.range((p, a, 0)..=(p, a, u16::MAX)).next().is_some())
    }
    /// the mappings and sessions there are
    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values()
    }
    pub fn mappings(&self) -> impl Iterator<Item = &Mapping> {
        self.maps.values()
    }
}

/// a row of "show ip nat translations"
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XlateRow {
    /// 0: an address alone ("---")
    pub proto: u8,
    pub inside_global: (Ipv4, Option<u16>),
    pub inside_local: (Ipv4, Option<u16>),
    /// the other end of a flow (None: "---")
    pub outside: Option<(Ipv4, u16)>,
    pub dynamic: bool,
    /// a flow with its ports
    pub extended: bool,
    pub created: Ns,
    pub used: Ns,
    /// when it times out (None: never, static)
    pub ends: Option<Ns>,
    /// TCP: what was seen ("established", "closing")
    pub tcp: Option<TcpSeen>,
}

/// where a packet's other side goes: the new address and, for flows with ports, the port
type To = (Ipv4, Option<u16>);

/// what a lookup of a new outbound flow found
enum Out {
    To(To),
    NoRule,
    Exhausted(String),
}

impl Engine {
    pub(crate) fn nat_part(&self, ni: usize) -> Option<&Nat> {
        Some(&self.stack(ni)?.nat)
    }
    fn nat_mut(&mut self, ni: usize) -> Option<&mut Nat> {
        Some(&mut self.stack_mut(ni)?.nat)
    }
    /// the NAT part of a device (None without IP)
    pub fn nat(&self, dev: u32) -> Option<&Nat> {
        self.nat_part(self.node_index(dev)?)
    }
    fn nat_side(&self, ni: usize, iface: u16) -> Option<Side> {
        self.nat_part(ni)?.side(iface)
    }

    /// timeouts that are due go first (a packet at the very moment a translation ends finds
    /// it gone, whatever ran first)
    fn nat_due(&mut self, ni: usize) {
        let now = self.now;
        if self.nat_part(ni).is_some_and(|n| n.armed.is_some_and(|t| t <= now)) {
            self.nat_sweep(ni);
        }
    }
    /// the timer at `t` unless it goes off earlier anyway
    fn nat_arm(&mut self, ni: usize, t: Option<Ns>) {
        let Some(t) = t else {
            return;
        };
        let Some(n) = self.nat_mut(ni) else {
            return;
        };
        if n.armed.is_none_or(|a| t < a) {
            n.armed = Some(t);
            self.set_timer(ni, SLOT_NAT, t);
        }
    }
    /// the node timer SLOT_NAT
    pub(crate) fn nat_timer(&mut self, ni: usize) {
        if let Some(n) = self.nat_mut(ni) {
            n.armed = None;
        }
        self.nat_sweep(ni);
    }
    /// ends what timed out (with its notes) and sets the timer for the next
    fn nat_sweep(&mut self, ni: usize) {
        let now = self.now;
        let dev = self.dev_of(ni);
        let Some(n) = self.nat_mut(ni) else {
            return;
        };
        n.armed = None;
        let mut notes: Vec<Note> = Vec::new();
        let due: Vec<u64> = n.sessions.values().filter(|s| n.session_end(s).is_some_and(|t| t <= now)).map(|s| s.id).collect();
        for id in due {
            if let Some(s) = n.drop_session(id) {
                n.stats.expired += 1;
                notes.push(Note::NatExpire { proto: s.proto, local: s.local, global: s.global, remote: Some(s.remote) });
            }
        }
        let due: Vec<u64> = n.maps.iter().filter(|(_, m)| n.mapping_end(m).is_some_and(|t| t <= now)).map(|(id, _)| *id).collect();
        for id in due {
            if let Some(m) = n.drop_mapping(id) {
                n.stats.expired += 1;
                notes.push(Note::NatExpire { proto: 0, local: m.local, global: m.global, remote: None });
            }
        }
        let due: Vec<_> = n.frags.iter().filter(|(_, f)| f.until <= now).map(|(k, _)| *k).collect();
        for k in due {
            if let Some(f) = n.frags.remove(&k)
                && !f.held.is_empty()
            {
                notes.push(Note::NatFragDropped { id: k.4, parts: f.held.len() as u16 });
            }
        }
        let next = n.next_due();
        for note in notes {
            self.note(dev, None, note);
        }
        self.nat_arm(ni, next);
    }

    // ---------------------------------------------------------------- outside to inside

    /// a packet that arrived on interface `iface` (after its inbound list): on an outside
    /// interface its destination is translated back and the packet forwarded; true when NAT
    /// took it (forwarded, waiting as a fragment, dropped), false when it goes on as it is
    pub(crate) fn nat_in(&mut self, ni: usize, iface: u16, v: &Ipv4View, hop: HopId) -> bool {
        if self.nat_side(ni, iface) != Some(Side::Outside) || !self.stack(ni).is_some_and(|s| s.forwarding) {
            return false;
        }
        self.nat_due(ni);
        let Some(f) = Flow::of(v.packet()) else {
            return false;
        };
        if f.later || f.mf {
            return self.nat_frag_in(ni, iface, v, &f, hop);
        }
        if is_icmp_error(&f) {
            return self.nat_error_in(ni, iface, v, hop);
        }
        match self.nat_lookup_in(ni, &f, hop) {
            Some(to) => {
                let mut p = v.packet().to_vec();
                rw::rewrite(&mut p, RwSide::Dst, to.0, to.1);
                self.note(self.dev_of(ni), Some(hop), Note::NatTranslate { proto: f.proto, inbound: true, from: (f.dst, to.1.map(|_| f.dport).unwrap_or(0)), to: (to.0, to.1.unwrap_or(0)) });
                self.nat_forward(ni, iface, v, p, hop);
                true
            }
            None => {
                // (what comes to the device's own addresses is its own traffic)
                let mine = self.stack(ni).is_some_and(|s| s.is_mine(f.dst));
                if !icmp_request(&f) && !mine && self.nat_owns(ni, f.dst) {
                    let port = if port_flow(&f) { f.dport } else { 0 };
                    self.note(self.dev_of(ni), Some(hop), Note::NatNoEntry { proto: f.proto, dst: (f.dst, port) });
                }
                false
            }
        }
    }

    /// forwards a packet translated on the way in: the TTL of the packet as it arrived
    fn nat_forward(&mut self, ni: usize, iface: u16, orig: &Ipv4View, mut p: Vec<u8>, hop: HopId) {
        if orig.ttl() <= 1 {
            self.note(self.dev_of(ni), Some(hop), Note::IpTtlExpired { src: orig.src() });
            self.icmp_error(ni, iface, orig.packet(), Icmp::TimeExceeded { code: icmp::EXCEEDED_TTL }, Some(hop));
            return;
        }
        let hl = orig.header_len().min(p.len());
        ipv4::dec_ttl(&mut p[..hl]);
        self.ip_output(ni, p, Origin::Forwarded { iface }, Some(hop));
    }

    /// a packet of an existing session: counted, its time and TCP state refreshed; the session
    /// as it is now
    fn nat_touch(&mut self, ni: usize, id: u64, inbound: bool, flags: u8) -> Option<Session> {
        let now = self.now;
        let n = self.nat_mut(ni)?;
        n.stats.hits += 1;
        let s = n.sessions.get_mut(&id)?;
        let closing = s.tcp.rst || (s.tcp.fin[0] && s.tcp.fin[1]);
        s.last = now;
        track(s, inbound, flags);
        let s = s.clone();
        // a TCP connection that just closed times out sooner
        if !closing && (s.tcp.rst || (s.tcp.fin[0] && s.tcp.fin[1])) {
            let end = self.nat_part(ni).and_then(|n| n.session_end(&s));
            self.nat_arm(ni, end);
        }
        Some(s)
    }

    /// the inside endpoint a packet from outside goes to (a session made or refreshed)
    fn nat_lookup_in(&mut self, ni: usize, f: &Flow, hop: HopId) -> Option<To> {
        let g = (f.dst, f.dport);
        let r = (f.src, f.sport);
        // a query from outside meets no mapping with ports: they take the answers to queries
        // from inside
        if port_flow(f) && !icmp_request(f) {
            if let Some(id) = self.nat_part(ni)?.session_by_out(f.proto, g, r) {
                return self.nat_touch(ni, id, true, f.flags).map(|s| (s.local.0, Some(s.local.1)));
            }
            if f.proto != PROTO_ICMP {
                // a static port translation
                if let Some(l) = self.static_port_in(ni, f.proto, g) {
                    self.nat_new_session(ni, f, l, g, r, None, true, Some(hop));
                    return Some((l.0, Some(l.1)));
                }
                // a dynamic mapping with ports, for addresses the inside host sent to
                let n = self.nat_part(ni)?;
                if let Some(&mid) = n.map_global.get(&(f.proto, g.0, g.1))
                    && n.talks_to(f.proto, g, r.0)
                {
                    let l = n.maps.get(&mid)?.local;
                    self.nat_new_session(ni, f, l, g, r, Some(mid), true, Some(hop));
                    return Some((l.0, Some(l.1)));
                }
            }
        }
        self.nat_simple_in(ni, f, hop)
    }

    /// a packet from outside to an address of basic NAT or of a static address translation
    fn nat_simple_in(&mut self, ni: usize, f: &Flow, hop: HopId) -> Option<To> {
        let now = self.now;
        let (g, r) = ((f.dst, f.dport), (f.src, f.sport));
        let (local, map) = match self.static_addr_in(ni, f.dst) {
            Some(l) => (l, None),
            None => {
                let n = self.nat_mut(ni)?;
                let mid = *n.map_global.get(&(0, f.dst, 0))?;
                let m = n.maps.get_mut(&mid)?;
                m.last = now;
                (m.local.0, Some(mid))
            }
        };
        if !port_flow(f) {
            return Some((local, None));
        }
        // a flow has its session (an ICMP request from outside makes one for its answer)
        match self.nat_part(ni)?.session_by_out(f.proto, g, r) {
            Some(id) => {
                self.nat_touch(ni, id, true, f.flags);
            }
            None => self.nat_new_session(ni, f, (local, g.1), g, r, map, true, Some(hop)),
        }
        Some((local, Some(g.1)))
    }

    /// an ICMP error from outside: the packet it quotes left translated, so the quote gets
    /// its inside source back and the error goes to that host (RFC 5508 REQ-5)
    fn nat_error_in(&mut self, ni: usize, iface: u16, v: &Ipv4View, hop: HopId) -> bool {
        let dev = self.dev_of(ni);
        let Some(q) = Flow::quoted(v.packet()) else {
            return false;
        };
        let to = self.nat_quote_local(ni, &q);
        match to {
            Some(l) => {
                let mut p = v.packet().to_vec();
                rw::rewrite_quoted(&mut p, RwSide::Src, l.0, l.1);
                rw::rewrite(&mut p, RwSide::Dst, l.0, None);
                self.note(dev, Some(hop), Note::NatQuote { inbound: true, from: (q.src, l.1.map(|_| q.sport).unwrap_or(0)), to: (l.0, l.1.unwrap_or(0)) });
                self.nat_forward(ni, iface, v, p, hop);
                true
            }
            None => {
                let mine = self.stack(ni).is_some_and(|s| s.is_mine(v.dst()));
                if !mine && self.nat_owns(ni, v.dst()) {
                    self.note(dev, Some(hop), Note::NatErrorDropped);
                    return true;
                }
                false
            }
        }
    }

    /// the inside endpoint of a packet that left translated (the quote of an error from
    /// outside); nothing is refreshed (RFC 5508 REQ-6)
    fn nat_quote_local(&self, ni: usize, q: &Flow) -> Option<To> {
        let n = self.nat_part(ni)?;
        let g = (q.src, q.sport);
        let r = (q.dst, q.dport);
        if port_flow(q) {
            if let Some(s) = n.session_by_out(q.proto, g, r).and_then(|id| n.sessions.get(&id)) {
                return Some((s.local.0, Some(s.local.1)));
            }
            if let Some(l) = self.static_port_in(ni, q.proto, g) {
                return Some((l.0, Some(l.1)));
            }
            if let Some(m) = n.map_global.get(&(q.proto, g.0, g.1)).and_then(|id| n.maps.get(id)) {
                return Some((m.local.0, Some(m.local.1)));
            }
        }
        if let Some(l) = self.static_addr_in(ni, q.src) {
            return Some((l, None));
        }
        n.map_global.get(&(0, q.src, 0)).and_then(|id| n.maps.get(id)).map(|m| (m.local.0, None))
    }

    // ---------------------------------------------------------------- inside to outside

    /// a packet on its way out of interface `out` (after routing): what leaves, each with the
    /// packet as it was before NAT changed it and the frame it came from (several when the
    /// first fragment of a datagram freed the ones that waited for it, none when NAT dropped
    /// it or holds it)
    pub(crate) fn nat_out(&mut self, ni: usize, origin: Origin, out: u16, pkt: Vec<u8>, cause: Option<HopId>) -> Vec<(Vec<u8>, Option<Vec<u8>>, Option<HopId>)> {
        let out_side = self.nat_side(ni, out);
        let in_side = match origin {
            Origin::Forwarded { iface } => self.nat_side(ni, iface),
            Origin::Local { .. } => None,
        };
        if out_side.is_none() && in_side.is_none() {
            return alloc::vec![(pkt, None, cause)];
        }
        let Some(f) = Flow::of(&pkt) else {
            return alloc::vec![(pkt, None, cause)];
        };
        self.nat_due(ni);
        if out_side == Some(Side::Outside) && is_icmp_error(&f) && !f.later {
            return self.nat_error_out(ni, origin, out, pkt, cause);
        }
        let Origin::Forwarded { iface: inif } = origin else {
            return alloc::vec![(pkt, None, cause)];
        };
        if in_side != Some(Side::Inside) || out_side != Some(Side::Outside) {
            return alloc::vec![(pkt, None, cause)];
        }
        if f.later || f.mf {
            return self.nat_frag_out(ni, inif, &f, pkt, cause);
        }
        match self.nat_translate_out(ni, inif, &f, &pkt, cause) {
            Some(Some(to)) => {
                let mut p = pkt.clone();
                rw::rewrite(&mut p, RwSide::Src, to.0, to.1);
                alloc::vec![(p, Some(pkt), cause)]
            }
            Some(None) => alloc::vec![(pkt, None, cause)],
            None => Vec::new(),
        }
    }

    /// the source a packet from inside leaves with: Some(Some(to)) translated (noted),
    /// Some(None) unchanged, None refused (noted, the sender told)
    fn nat_translate_out(&mut self, ni: usize, inif: u16, f: &Flow, pkt: &[u8], cause: Option<HopId>) -> Option<Option<To>> {
        let dev = self.dev_of(ni);
        match self.nat_lookup_out(ni, f, cause) {
            Out::To(to) => {
                self.note(dev, cause, Note::NatTranslate { proto: f.proto, inbound: false, from: (f.src, to.1.map(|_| f.sport).unwrap_or(0)), to: (to.0, to.1.unwrap_or(0)) });
                Some(Some(to))
            }
            Out::NoRule => {
                self.note(dev, cause, Note::NatNoRule { src: f.src });
                Some(None)
            }
            Out::Exhausted(what) => {
                if let Some(n) = self.nat_mut(ni) {
                    n.stats.failed += 1;
                }
                self.note(dev, cause, Note::NatExhausted { what });
                // RFC 5508 REQ-8: the sender hears of it
                let q = crate::l3::as_received(pkt);
                self.icmp_error(ni, inif, &q, Icmp::Unreachable { code: UNREACH_ADMIN, mtu: 0 }, cause);
                None
            }
        }
    }

    /// the global endpoint of a packet from inside (a session made or refreshed)
    fn nat_lookup_out(&mut self, ni: usize, f: &Flow, cause: Option<HopId>) -> Out {
        let now = self.now;
        let Some(n) = self.nat_part(ni) else {
            return Out::NoRule;
        };
        let nrules = n.rules.len();
        let ports = port_flow(f);
        let l = (f.src, f.sport);
        let r = (f.dst, f.dport);
        if ports {
            if let Some(id) = n.session_by_in(f.proto, l, r) {
                return match self.nat_touch(ni, id, false, f.flags) {
                    Some(s) => Out::To((s.global.0, Some(s.global.1))),
                    None => Out::NoRule,
                };
            }
            if f.proto != PROTO_ICMP
                && let Some(g) = self.static_port_out(ni, f.proto, l)
            {
                self.nat_new_session(ni, f, l, g, r, None, false, cause);
                return Out::To((g.0, Some(g.1)));
            }
        }
        if let Some(g) = self.static_addr_out(ni, l.0) {
            if ports {
                self.nat_new_session(ni, f, l, (g, l.1), r, None, false, cause);
                return Out::To((g, Some(l.1)));
            }
            return Out::To((g, None));
        }
        if ports && let Some(&mid) = self.nat_part(ni).and_then(|n| n.map_local.get(&(f.proto, l.0, l.1))) {
            let g = self.nat_part(ni).and_then(|n| n.maps.get(&mid)).map(|m| m.global).unwrap_or(l);
            self.nat_new_session(ni, f, l, g, r, Some(mid), false, cause);
            return Out::To((g.0, Some(g.1)));
        }
        if let Some(&mid) = self.nat_part(ni).and_then(|n| n.map_local.get(&(0, l.0, 0))) {
            let g = match self.nat_mut(ni).and_then(|n| n.maps.get_mut(&mid)) {
                Some(m) => {
                    m.last = now;
                    m.global.0
                }
                None => l.0,
            };
            if ports {
                self.nat_new_session(ni, f, l, (g, l.1), r, Some(mid), false, cause);
                return Out::To((g, Some(l.1)));
            }
            return Out::To((g, None));
        }
        // the dynamic rules in their order: the first whose list permits the packet
        for i in 0..nrules {
            let Some((rid, Rule::List { acl, to, overload })) = self.rules(ni).get(i).cloned() else {
                continue;
            };
            if self.acl_match(ni, &acl, f) != Some(true) {
                continue;
            }
            let what = match &to {
                Target::Pool(p) => format!("pool {}", p),
                Target::Iface(x) => format!("interface {}", x),
            };
            if overload {
                if !ports {
                    // no ports to tell hosts apart: the address alone, without a mapping
                    return match self.overload_addrs(ni, &to).first() {
                        Some(a) => Out::To((*a, None)),
                        None => Out::Exhausted(what),
                    };
                }
                return match self.alloc_pat(ni, rid, &to, f.proto, l) {
                    Some(g) => {
                        // the mapping lives with its sessions: the session's note tells of both
                        let mid = self.nat_mut(ni).map(|n| n.add_mapping(Mapping { proto: f.proto, local: l, global: g, rule: rid, created: now, last: now, sessions: 0 }));
                        self.nat_new_session(ni, f, l, g, r, mid, false, cause);
                        Out::To((g.0, Some(g.1)))
                    }
                    None => Out::Exhausted(what),
                };
            }
            let Target::Pool(pname) = &to else {
                return Out::Exhausted(what);
            };
            return match self.alloc_addr(ni, pname) {
                Some(a) => {
                    let mid = self.nat_mut(ni).map(|n| n.add_mapping(Mapping { proto: 0, local: (l.0, 0), global: (a, 0), rule: rid, created: now, last: now, sessions: 0 }));
                    self.note(self.dev_of(ni), cause, Note::NatCreate { proto: 0, local: (l.0, 0), global: (a, 0), remote: None });
                    if ports {
                        self.nat_new_session(ni, f, l, (a, l.1), r, mid, false, cause);
                        Out::To((a, Some(l.1)))
                    } else {
                        let end = self.nat_part(ni).and_then(|n| mid.and_then(|m| n.maps.get(&m)).and_then(|m| n.mapping_end(m)));
                        self.nat_arm(ni, end);
                        Out::To((a, None))
                    }
                }
                None => Out::Exhausted(what),
            };
        }
        Out::NoRule
    }

    /// an ICMP error on its way out of an outside interface: when the packet it quotes came in
    /// translated, the quote gets the global destination back, and the error's source becomes a
    /// global address (RFC 5508 REQ-4); one from inside about nothing NAT knows is dropped
    fn nat_error_out(&mut self, ni: usize, origin: Origin, out: u16, pkt: Vec<u8>, cause: Option<HopId>) -> Vec<(Vec<u8>, Option<Vec<u8>>, Option<HopId>)> {
        let dev = self.dev_of(ni);
        let from_inside = matches!(origin, Origin::Forwarded { iface } if self.nat_side(ni, iface) == Some(Side::Inside));
        let (Some(q), Some(v)) = (Flow::quoted(&pkt), Ipv4View::parse(&pkt)) else {
            return alloc::vec![(pkt, None, cause)];
        };
        let src = v.src();
        let Some(g) = self.nat_quote_global(ni, &q) else {
            if from_inside {
                self.note(dev, cause, Note::NatErrorDropped);
                return Vec::new();
            }
            return alloc::vec![(pkt, None, cause)];
        };
        // the error's own source: a host's global address of basic NAT, else the device's
        // address on the outside
        let new_src = match self.static_addr_out(ni, src).or_else(|| self.nat_part(ni).and_then(|n| n.map_local.get(&(0, src, 0)).and_then(|id| n.maps.get(id)).map(|m| m.global.0))) {
            Some(a) => a,
            None => {
                let outside = self.stack(ni).is_some_and(|st| st.ifaces.iter().enumerate().any(|(i, f)| f.has(src) && self.nat_side(ni, i as u16) == Some(Side::Outside)));
                if outside { src } else { self.stack(ni).and_then(|st| st.ifaces.get(out as usize)).and_then(|f| f.primary()).unwrap_or(src) }
            }
        };
        let mut p = pkt.clone();
        rw::rewrite_quoted(&mut p, RwSide::Dst, g.0, g.1);
        if new_src != src {
            rw::rewrite(&mut p, RwSide::Src, new_src, None);
        }
        self.note(dev, cause, Note::NatQuote { inbound: false, from: (q.dst, g.1.map(|_| q.dport).unwrap_or(0)), to: (g.0, g.1.unwrap_or(0)) });
        alloc::vec![(p, Some(pkt), cause)]
    }

    /// the global endpoint of a packet that came in translated (the quote of an error from
    /// inside)
    fn nat_quote_global(&self, ni: usize, q: &Flow) -> Option<To> {
        let n = self.nat_part(ni)?;
        let l = (q.dst, q.dport);
        let r = (q.src, q.sport);
        if port_flow(q) {
            if let Some(s) = n.session_by_in(q.proto, l, r).and_then(|id| n.sessions.get(&id)) {
                return Some((s.global.0, Some(s.global.1)));
            }
            if q.proto != PROTO_ICMP
                && let Some(g) = self.static_port_out(ni, q.proto, l)
            {
                return Some((g.0, Some(g.1)));
            }
            if let Some(m) = n.map_local.get(&(q.proto, l.0, l.1)).and_then(|id| n.maps.get(id)) {
                return Some((m.global.0, Some(m.global.1)));
            }
        }
        if let Some(g) = self.static_addr_out(ni, q.dst) {
            return Some((g, None));
        }
        n.map_local.get(&(0, q.dst, 0)).and_then(|id| n.maps.get(id)).map(|m| (m.global.0, None))
    }

    /// a new session (noted); `inbound`: its first packet came from outside
    #[allow(clippy::too_many_arguments)]
    fn nat_new_session(&mut self, ni: usize, f: &Flow, local: (Ipv4, u16), global: (Ipv4, u16), remote: (Ipv4, u16), map: Option<u64>, inbound: bool, cause: Option<HopId>) {
        let now = self.now;
        let dev = self.dev_of(ni);
        let mut tcp = TcpSeen { established: f.proto == PROTO_TCP && f.flags & SYN == 0, ..TcpSeen::default() };
        let d = inbound as usize;
        tcp.syn[d] = f.flags & SYN != 0;
        tcp.fin[d] = f.flags & FIN != 0;
        tcp.rst = f.flags & RST != 0;
        // an ICMP query's remote end sees the global identifier
        let remote = if f.proto == PROTO_ICMP { (remote.0, global.1) } else { remote };
        let Some(n) = self.nat_mut(ni) else {
            return;
        };
        n.stats.misses += 1;
        if let Some(m) = map.and_then(|m| n.maps.get_mut(&m)) {
            m.last = now;
        }
        let id = n.add_session(Session { id: 0, proto: f.proto, local, global, remote, map, created: now, last: now, tcp });
        let end = n.sessions.get(&id).and_then(|s| n.session_end(s));
        self.note(dev, cause, Note::NatCreate { proto: f.proto, local, global, remote: Some(remote) });
        self.nat_arm(ni, end);
    }

    // ---------------------------------------------------------------- fragments

    /// a fragment from outside: the first decides, later ones that come before it wait
    fn nat_frag_in(&mut self, ni: usize, iface: u16, v: &Ipv4View, f: &Flow, hop: HopId) -> bool {
        let now = self.now;
        let dev = self.dev_of(ni);
        let key = (false, f.src, f.dst, f.proto, f.id);
        if !f.later {
            // the first fragment: translated like a whole packet
            let to = if is_icmp_error(f) {
                None
            } else {
                self.nat_lookup_in(ni, f, hop)
            };
            let held = self.frag_decide(ni, key, Some(match to {
                Some(t) => FragTo::To(t.0),
                None => FragTo::Unchanged,
            }));
            let took = match to {
                Some(t) => {
                    let mut p = v.packet().to_vec();
                    rw::rewrite(&mut p, RwSide::Dst, t.0, t.1);
                    self.note(dev, Some(hop), Note::NatTranslate { proto: f.proto, inbound: true, from: (f.dst, t.1.map(|_| f.dport).unwrap_or(0)), to: (t.0, t.1.unwrap_or(0)) });
                    self.nat_forward(ni, iface, v, p, hop);
                    true
                }
                None => false,
            };
            if !took {
                // the first fragment goes its normal way before the ones that waited for it
                self.ip_deliver(ni, iface, v, hop);
            }
            for (p, h, inif) in held {
                let Some(hv) = Ipv4View::parse(&p) else {
                    continue;
                };
                match to {
                    Some(t) => {
                        let mut q = p.clone();
                        rw::rewrite(&mut q, RwSide::Dst, t.0, None);
                        self.nat_forward(ni, inif, &hv, q, h.unwrap_or(0));
                    }
                    None => self.ip_deliver(ni, inif, &hv, h.unwrap_or(0)),
                }
            }
            return true;
        }
        let decided = self.nat_part(ni).and_then(|n| n.frags.get(&key)).and_then(|s| s.to);
        match decided {
            Some(FragTo::To(a)) => {
                let mut p = v.packet().to_vec();
                rw::rewrite(&mut p, RwSide::Dst, a, None);
                self.nat_forward(ni, iface, v, p, hop);
                true
            }
            Some(FragTo::Unchanged) => false,
            Some(FragTo::Dropped) => true,
            None => {
                if !self.nat_owns(ni, f.dst) {
                    return false;
                }
                self.frag_hold(ni, key, v.packet().to_vec(), Some(hop), iface, now);
                true
            }
        }
    }

    /// a fragment from inside to outside (in ip_output); what leaves now
    fn nat_frag_out(&mut self, ni: usize, inif: u16, f: &Flow, pkt: Vec<u8>, cause: Option<HopId>) -> Vec<(Vec<u8>, Option<Vec<u8>>, Option<HopId>)> {
        let now = self.now;
        let key = (true, f.src, f.dst, f.proto, f.id);
        if !f.later {
            let r = self.nat_translate_out(ni, inif, f, &pkt, cause);
            let to = match r {
                Some(Some(t)) => FragTo::To(t.0),
                Some(None) => FragTo::Unchanged,
                None => FragTo::Dropped,
            };
            let held = self.frag_decide(ni, key, Some(to));
            let mut out = Vec::new();
            match r {
                Some(Some(t)) => {
                    let mut p = pkt.clone();
                    rw::rewrite(&mut p, RwSide::Src, t.0, t.1);
                    out.push((p, Some(pkt), cause));
                }
                Some(None) => out.push((pkt, None, cause)),
                None => {}
            }
            for (p, h, _) in held {
                match to {
                    FragTo::To(a) => {
                        let mut q = p.clone();
                        rw::rewrite(&mut q, RwSide::Src, a, None);
                        out.push((q, Some(p), h));
                    }
                    FragTo::Unchanged => out.push((p, None, h)),
                    FragTo::Dropped => {}
                }
            }
            return out;
        }
        let decided = self.nat_part(ni).and_then(|n| n.frags.get(&key)).and_then(|s| s.to);
        match decided {
            Some(FragTo::To(a)) => {
                let mut p = pkt.clone();
                rw::rewrite(&mut p, RwSide::Src, a, None);
                alloc::vec![(p, Some(pkt), cause)]
            }
            Some(FragTo::Unchanged) => alloc::vec![(pkt, None, cause)],
            Some(FragTo::Dropped) => Vec::new(),
            None => {
                self.frag_hold(ni, key, pkt, cause, inif, now);
                Vec::new()
            }
        }
    }

    /// what the first fragment decided; the fragments that waited, in order
    fn frag_decide(&mut self, ni: usize, key: (bool, Ipv4, Ipv4, u8, u16), to: Option<FragTo>) -> Vec<(Vec<u8>, Option<HopId>, u16)> {
        let until = self.now + REASM_TIME;
        let Some(n) = self.nat_mut(ni) else {
            return Vec::new();
        };
        if n.frags.len() >= FRAG_SETS && !n.frags.contains_key(&key) {
            return Vec::new();
        }
        let set = n.frags.entry(key).or_insert(FragSet { to: None, held: Vec::new(), until });
        set.to = to;
        let mut held = core::mem::take(&mut set.held);
        held.sort_by_key(|(p, _, _)| Ipv4View::parse(p).map(|v| v.frag_off()).unwrap_or(0));
        self.nat_arm(ni, Some(until));
        held
    }

    fn frag_hold(&mut self, ni: usize, key: (bool, Ipv4, Ipv4, u8, u16), pkt: Vec<u8>, hop: Option<HopId>, iface: u16, now: Ns) {
        let dev = self.dev_of(ni);
        let until = now + REASM_TIME;
        let Some(n) = self.nat_mut(ni) else {
            return;
        };
        if n.frags.len() >= FRAG_SETS && !n.frags.contains_key(&key) {
            return;
        }
        let set = n.frags.entry(key).or_insert(FragSet { to: None, held: Vec::new(), until });
        if set.held.len() >= FRAG_HOLD {
            return;
        }
        set.held.push((pkt, hop, iface));
        self.note(dev, hop, Note::NatFragWait { id: key.4 });
        self.nat_arm(ni, Some(until));
    }

    // ---------------------------------------------------------------- rules and addresses

    /// the address of a static rule's global side now
    fn global_addr(&self, ni: usize, g: &Global) -> Option<Ipv4> {
        match g {
            Global::Addr(a) => Some(*a),
            Global::Iface(name) => {
                let st = self.stack(ni)?;
                st.ifaces.get(st.iface_by_name(name)? as usize)?.primary()
            }
        }
    }
    /// the rules of a device
    fn rules(&self, ni: usize) -> &[(u32, Rule)] {
        self.nat_part(ni).map(|n| n.rules.as_slice()).unwrap_or(&[])
    }
    fn static_port_in(&self, ni: usize, proto: u8, g: (Ipv4, u16)) -> Option<(Ipv4, u16)> {
        self.rules(ni).iter().find_map(|(_, r)| match r {
            Rule::Static { proto: p, local, lport, global, gport, .. } if *p == proto && *gport == g.1 && self.global_addr(ni, global) == Some(g.0) => Some((*local, *lport)),
            _ => None,
        })
    }
    fn static_port_out(&self, ni: usize, proto: u8, l: (Ipv4, u16)) -> Option<(Ipv4, u16)> {
        self.rules(ni).iter().find_map(|(_, r)| match r {
            Rule::Static { proto: p, local, lport, global, gport, .. } if *p == proto && *local == l.0 && *lport == l.1 => self.global_addr(ni, global).map(|a| (a, *gport)),
            _ => None,
        })
    }
    fn static_addr_in(&self, ni: usize, g: Ipv4) -> Option<Ipv4> {
        self.rules(ni).iter().find_map(|(_, r)| match r {
            Rule::Static { proto: 0, local, global, .. } if self.global_addr(ni, global) == Some(g) => Some(*local),
            _ => None,
        })
    }
    fn static_addr_out(&self, ni: usize, l: Ipv4) -> Option<Ipv4> {
        self.rules(ni).iter().find_map(|(_, r)| match r {
            Rule::Static { proto: 0, local, global, .. } if *local == l => self.global_addr(ni, global),
            _ => None,
        })
    }

    /// an address NAT stands for: of an outside interface, of a pool, of a static rule, of a
    /// translation
    pub(crate) fn nat_owns(&self, ni: usize, a: Ipv4) -> bool {
        let (Some(st), Some(n)) = (self.stack(ni), self.nat_part(ni)) else {
            return false;
        };
        let outside = st.ifaces.iter().enumerate().any(|(i, f)| !f.hidden && f.has(a) && n.side(i as u16) == Some(Side::Outside));
        outside
            || n.pools.values().any(|p| p.contains(a))
            || n.rules.iter().any(|(_, r)| matches!(r, Rule::Static { global, .. } if self.global_addr(ni, global) == Some(a)))
            || n.global_used(a)
    }

    /// an outside interface answers ARP for the global addresses of its network: pools and
    /// static rules (the device itself has them)
    pub(crate) fn nat_arp(&self, ni: usize, iface: u16, a: Ipv4) -> bool {
        let (Some(st), Some(n)) = (self.stack(ni), self.nat_part(ni)) else {
            return false;
        };
        if n.side(iface) != Some(Side::Outside) {
            return false;
        }
        let Some(f) = st.ifaces.get(iface as usize) else {
            return false;
        };
        if !f.on_link(a) || f.has(a) || f.is_broadcast(a) {
            return false;
        }
        n.pools.values().any(|p| p.contains(a)) || n.rules.iter().any(|(_, r)| matches!(r, Rule::Static { global: Global::Addr(g), .. } if *g == a))
    }

    /// the addresses of an overload target in their order
    fn overload_addrs(&self, ni: usize, to: &Target) -> Vec<Ipv4> {
        match to {
            Target::Iface(name) => self.global_addr(ni, &Global::Iface(name.clone())).into_iter().collect(),
            Target::Pool(p) => match self.nat_part(ni).and_then(|n| n.pools.get(p)) {
                Some(pool) => (pool.start.to_u32()..=pool.end.to_u32()).take(65_536).map(Ipv4::from_u32).collect(),
                None => Vec::new(),
            },
        }
    }

    /// a global endpoint for an inside endpoint with ports: paired with the host's address of
    /// the rule when it has one, the port kept when it is free
    fn alloc_pat(&self, ni: usize, rule: u32, to: &Target, proto: u8, l: (Ipv4, u16)) -> Option<(Ipv4, u16)> {
        let n = self.nat_part(ni)?;
        let mut addrs = self.overload_addrs(ni, to);
        // paired pooling (RFC 4787 REQ-2): the address the host already has from this rule first
        if let Some(a) = n.maps.values().find(|m| m.rule == rule && m.local.0 == l.0).map(|m| m.global.0)
            && let Some(i) = addrs.iter().position(|x| *x == a)
        {
            let a = addrs.remove(i);
            addrs.insert(0, a);
        }
        for a in addrs {
            if n.map_global.contains_key(&(0, a, 0)) || self.static_addr_in(ni, a).is_some() {
                continue;
            }
            if let Some(p) = self.free_port(ni, proto, a, l.1) {
                return Some((a, p));
            }
        }
        None
    }

    /// the port (or ICMP identifier) `want` when it is free on `a`, else the next free one
    fn free_port(&self, ni: usize, proto: u8, a: Ipv4, want: u16) -> Option<u16> {
        let n = self.nat_part(ni)?;
        let mine = self.stack(ni).is_some_and(|s| s.is_mine(a));
        let free = |p: u16| !n.map_global.contains_key(&(proto, a, p)) && self.static_port_in(ni, proto, (a, p)).is_none() && !(mine && self.own_port_used(ni, proto, p));
        if proto == PROTO_ICMP {
            return (0..=u16::MAX as u32).map(|i| want.wrapping_add(i as u16)).find(|&p| free(p));
        }
        // the range and parity of the inside port (RFC 4787 REQ-3, REQ-4)
        let (lo, hi) = if want < 1024 { (1u32, 1023u32) } else { (1024u32, 65535u32) };
        let want = (want as u32).clamp(lo, hi);
        let parity = want & 1;
        let first = if lo & 1 == parity { lo } else { lo + 1 };
        let count = (hi - first) / 2 + 1;
        let start = (want - first) / 2;
        (0..count).map(|i| first + (start + i) % count * 2).map(|p| p as u16).find(|&p| free(p))
    }

    /// the device itself uses this port (a socket of its own)
    fn own_port_used(&self, ni: usize, proto: u8, p: u16) -> bool {
        match proto {
            PROTO_UDP => self.os(ni).is_some_and(|o| !o.port_free(p, None)),
            PROTO_TCP => self.tcp(ni).is_some_and(|t| t.listeners.values().any(|l| l.port == p) || t.conns.values().any(|c| c.local.1 == p)),
            _ => false,
        }
    }

    /// the first free address of a pool for basic NAT
    fn alloc_addr(&self, ni: usize, pool: &str) -> Option<Ipv4> {
        let n = self.nat_part(ni)?;
        let p = n.pools.get(pool)?;
        let st = self.stack(ni)?;
        (p.start.to_u32()..=p.end.to_u32()).map(Ipv4::from_u32).find(|&a| !n.global_used(a) && self.static_addr_in(ni, a).is_none() && !st.is_mine(a))
    }

    // ---------------------------------------------------------------- views

    /// the rows of "show ip nat translations": flows, then the addresses alone, by global
    /// address
    pub fn nat_translations(&self, dev: u32) -> Vec<XlateRow> {
        let Some(ni) = self.node_index(dev) else {
            return Vec::new();
        };
        let Some(n) = self.nat_part(ni) else {
            return Vec::new();
        };
        let mut rows: Vec<XlateRow> = Vec::new();
        for s in n.sessions.values() {
            rows.push(XlateRow {
                proto: s.proto,
                inside_global: (s.global.0, Some(s.global.1)),
                inside_local: (s.local.0, Some(s.local.1)),
                outside: Some(s.remote),
                dynamic: s.map.is_some(),
                extended: true,
                created: s.created,
                used: s.last,
                ends: n.session_end(s),
                tcp: if s.proto == PROTO_TCP { Some(s.tcp) } else { None },
            });
        }
        for m in n.maps.values().filter(|m| m.proto == 0) {
            rows.push(XlateRow { proto: 0, inside_global: (m.global.0, None), inside_local: (m.local.0, None), outside: None, dynamic: true, extended: false, created: m.created, used: m.last, ends: n.mapping_end(m), tcp: None });
        }
        for (_, r) in &n.rules {
            if let Rule::Static { proto, local, lport, global, gport, .. } = r
                && let Some(g) = self.global_addr(ni, global)
            {
                let (gp, lp) = if *proto == 0 { (None, None) } else { (Some(*gport), Some(*lport)) };
                rows.push(XlateRow { proto: *proto, inside_global: (g, gp), inside_local: (*local, lp), outside: None, dynamic: false, extended: *proto != 0, created: 0, used: 0, ends: None, tcp: None });
            }
        }
        rows.sort_by_key(|r| (r.inside_global.0.to_u32(), r.outside.is_none() && r.proto == 0, r.inside_global.1.unwrap_or(0), r.proto, r.outside.map(|o| (o.0.to_u32(), o.1)).unwrap_or((0, 0))));
        rows
    }

    // ---------------------------------------------------------------- actions

    /// the NAT actions (action.rs)
    pub(crate) fn nat_apply(&mut self, ni: usize, dev: u32, a: NatAct) -> Result<(), ActErr> {
        let st = self.stack_mut(ni).ok_or(ActErr::NoIp(dev))?;
        match a {
            NatAct::Iface { iface, side } => {
                let i = st.iface_by_name(&iface).ok_or(ActErr::UnknownIface { dev, iface: iface.clone() })?;
                match side {
                    Some(s) => {
                        st.nat.sides.insert(i, s);
                    }
                    None => {
                        st.nat.sides.remove(&i);
                    }
                }
                Ok(())
            }
            NatAct::Pool { name, pool } => match pool {
                Some(p) => {
                    let net = Prefix4 { addr: p.start, len: p.len }.trunc();
                    if p.len == 0 || p.len > 32 || p.start.to_u32() > p.end.to_u32() || !net.contains(p.end) {
                        return Err(ActErr::Refused(msg!("Pool {}: Anfang und Ende liegen nicht im selben Netz.", name)));
                    }
                    if let Some(old) = st.nat.pools.get(&name)
                        && *old != p
                        && st.nat.maps.values().any(|m| old.contains(m.global.0))
                    {
                        return Err(ActErr::Refused(msg!("Pool {} ist in Gebrauch.", name)));
                    }
                    st.nat.pools.insert(name, p);
                    Ok(())
                }
                None => {
                    let Some(old) = st.nat.pools.get(&name).copied() else {
                        return Err(ActErr::Refused(msg!("Pool {} gibt es nicht.", name)));
                    };
                    if st.nat.maps.values().any(|m| old.contains(m.global.0)) {
                        return Err(ActErr::Refused(msg!("Pool {} ist in Gebrauch.", name)));
                    }
                    st.nat.pools.remove(&name);
                    Ok(())
                }
            },
            NatAct::Rule { rule, add } => {
                if add {
                    st.nat.add_rule(rule).map_err(|_| ActErr::Refused(msg!("Diese Adresse hat schon eine statische Übersetzung.")))
                } else if st.nat.remove_rule(&rule) {
                    Ok(())
                } else {
                    Err(ActErr::Refused(msg!("Keine solche NAT-Regel.")))
                }
            }
            NatAct::Timeout { kind, secs } => {
                st.nat.timeouts.set(kind, secs.unwrap_or(kind.default_secs()));
                Ok(())
            }
            NatAct::Clear { stats } => {
                if stats {
                    st.nat.stats = NatStats::default();
                } else {
                    st.nat.clear();
                }
                Ok(())
            }
        }
    }
}

/// what Action::Nat does
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NatAct {
    /// the side of an interface ("ip nat inside|outside"; None: neither)
    Iface { iface: String, side: Option<Side> },
    /// a pool ("ip nat pool"; None removes it)
    Pool { name: String, pool: Option<Pool> },
    /// a rule of inside source translation (add or remove)
    Rule { rule: Rule, add: bool },
    /// a timeout in seconds (None: the default; NEVER: none)
    Timeout { kind: Tmo, secs: Option<u32> },
    /// forgets the dynamic translations, or (stats) the counters
    Clear { stats: bool },
}

/// what a packet of a flow says about its TCP connection
fn track(s: &mut Session, inbound: bool, flags: u8) {
    if s.proto != PROTO_TCP {
        return;
    }
    let d = inbound as usize;
    if flags & SYN != 0 {
        s.tcp.syn[d] = true;
    }
    if flags & FIN != 0 {
        s.tcp.fin[d] = true;
    }
    if flags & RST != 0 {
        s.tcp.rst = true;
    }
    if s.tcp.syn[0] && s.tcp.syn[1] {
        s.tcp.established = true;
    }
}
