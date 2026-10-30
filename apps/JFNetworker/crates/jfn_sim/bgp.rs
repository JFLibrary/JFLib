// bgp -- BGP-4 after RFC 4271 (4-octet AS numbers after RFC 6793, route refresh after RFC 2918)
// on IOS-like routers, L3 switches and the firewall: a service of the device's OS ("bgp") with
// its configuration ("router bgp ASN"), the sessions to its neighbours over TCP port 179 through
// the OS sockets, the Adj-RIBs-In, the Loc-RIB with the decision process, the Adj-RIBs-Out and
// the routes it gives the RIB; the command lines are in cli/bgp.rs, the notes in bgptrace.rs
//
//   BgpCfg / NbrCfg            the AS, router ID, logging of neighbour changes, timers, the
//                              default local preference, network statements, redistribution;
//                              per neighbour remote-as, description, update-source,
//                              next-hop-self, ebgp-multihop, shutdown, weight
//   BgpSet / Action::Bgp       changes of the configuration (Engine::bgp_apply); Process { on:
//                              false } is "no router bgp", Clear is "clear ip bgp"
//   Bgp                        the service: router_id(), peers() (Peer: state(), what the OPENs
//                              negotiated, counters, the Adj-RIB-In and Adj-RIB-Out), loc_rib()
//                              (Dest: the candidate paths, the best one and the step that chose
//                              it), local() (the paths it originates), version(), counters `n`;
//                              metrics neighbours, established, prefixes, paths (gauges) and the
//                              messages counted
//   Engine::bgp(dev)           the service of a device (None before the first BGP command)
//   Engine::bgp_kick(ni)       interfaces, addresses or routes of the device may have changed
//                              (link.rs, Engine::apply, the routes of RIP and OSPF): BGP looks
//                              again at once (sessions, next hops, network statements)
//   Engine::bgp_problems       neighbours that do not come up and why, network statements
//                              without a route (problems.rs)
//
// Sessions (RFC 4271 8): one per configured neighbour address.  A neighbour in the own AS is
// internal (iBGP), any other external (eBGP).  The service listens on port 179 of every IPv4
// address; a connection from an address that is no neighbour, or from one shut down, is reset as
// soon as it is there (IOS refuses the SYN itself).  The states are the RFC's: Idle, Connect
// (an own connection attempt runs), Active (waiting for the neighbour or for ConnectRetry),
// OpenSent, OpenConfirm, Established.  A neighbour leaves Idle once the process has a router ID
// and the neighbour is not shut down; an external one only while its address lies in a connected
// network or ebgp-multihop is set ("not directly connected"), one with update-source while that
// interface has an address.  It opens a connection -- TTL 1 to an external neighbour, the
// ebgp-multihop value, the system's 255 inside the AS; from the update-source address, else from
// the address of the way out; TCP_NODELAY -- and starts ConnectRetry (120 s, RFC 4271 10).  An
// attempt that fails (no route, refused, timed out, reset) leaves it Active until ConnectRetry
// expires or, after "no route", until a route to the neighbour appears (it tries at once then,
// as next-hop tracking would: our choice).  A connection that comes up sends OPEN (version 4,
// the AS or AS_TRANS above 65535, the hold time, the router ID, the capabilities IPv4 unicast,
// route refresh and 4-octet AS) and waits for the neighbour's with a hold time of 240 s (8.2.2).
//
// OPEN (6.2): the codec checks the version, the hold time (0 or at least 3 s) and the identifier;
// here the AS (of the 4-octet capability, else of the header) must be remote-as (Bad Peer AS)
// and the BGP identifier another than the own one (Bad BGP Identifier).  The hold time is the
// smaller of the two, the keepalive interval the configured one but at most a third of it (none
// with hold time 0); 4-octet AS numbers and route refresh are used when both offer them.  A
// KEEPALIVE answers the OPEN (OpenConfirm), the neighbour's KEEPALIVE brings Established.  What
// comes in restarts the hold timer; when it expires, NOTIFICATION Hold Timer Expired.  Each
// KEEPALIVE or UPDATE sent restarts the keepalive timer.  No timer is jittered (RFC 4271 10
// suggests it; runs are deterministic either way).
//
// Collisions (6.8): both sides may open a connection at once.  An OPEN on one while the other is
// in OpenConfirm keeps the connection opened by the side with the higher BGP identifier and
// closes the other with Cease / Connection Collision Resolution; one that comes while a session
// is Established is closed the same way, and an incoming connection while the own attempt is
// still in SYN-SENT replaces it.
//
// The end of a session: a NOTIFICATION (sent or received), a TCP connection that is reset,
// times out or is closed by the neighbour, an expired hold timer.  Its routes leave the Loc-RIB at
// once and the neighbour goes Idle; it starts again at once after an established session, 60 s
// later (IdleHold, our choice) after an OPEN was refused or a NOTIFICATION other than Cease
// came before Established, so a neighbour with a wrong AS does not flap every second.  Fast
// external fallover (as IOS): when the network towards a directly connected external neighbour
// goes away (its interface down), the session ends at once without waiting for the hold timer.
// clear ip bgp sends Cease / Administrative Reset, neighbor shutdown Cease / Administrative
// Shutdown, no neighbor Cease / Peer De-configured, a new router ID, remote-as or update-source
// Cease / Other Configuration Change.  An incoming connection is taken in every state but shut
// down, also while the neighbour waits Idle.
//
// UPDATE (6.3, 9): the codec checks the message (a NOTIFICATION each); from an external
// neighbour the first AS of the path must be the neighbour's (Malformed AS_PATH, as IOS's
// enforce-first-as).  Routes with the own AS in their AS_PATH are dropped (9.1.2: loops), so are
// routes whose next hop is an address of the device or no unicast address (6.3: no
// NOTIFICATION); LOCAL_PREF counts only from internal neighbours; a 2-octet session's AS4_PATH
// is merged into the AS_PATH (RFC 6793 4.2.3).  What remains replaces the neighbour's earlier
// path to the prefix in its Adj-RIB-In.  ROUTE-REFRESH sends the whole Adj-RIB-Out again.
//
// Local paths: "network A mask M" originates the prefix while the RIB holds exactly that prefix
// from another source (ORIGIN IGP, MED the metric of that route); "redistribute connected"
// the networks of the interfaces that are up, "redistribute static|rip|ospf" the routes of that
// source that win in the RIB (ORIGIN INCOMPLETE, MED their metric or the configured one); a
// network statement goes before redistribution of the same prefix.  Local paths have the next
// hop 0.0.0.0 (the session's address on the wire), weight 32768 and the default local
// preference.
//
// The decision (9.1): a path is a candidate when its next hop resolves through the RIB over a
// route that is not BGP's (the interface and the metric of that route: the IGP metric; local
// paths always).  Among the candidates of a prefix each step keeps the best and the step that
// leaves one path is noted: 1. the highest weight (32768 local, else the neighbour's weight, 0
// without one); 2. the highest LOCAL_PREF (the default, 100, where a path has none); 3.
// locally originated; 4. the shortest AS_PATH (an AS_SET counts as one); 5. the lowest ORIGIN
// (IGP, EGP, INCOMPLETE); 6. the lowest MED among paths from the same neighbouring AS (the first
// AS of the path, the own AS for an empty one; no MED counts as 0, as on IOS); 7. external before
// internal; 8. the lowest IGP metric to the next hop; 9. the lowest BGP identifier of the
// neighbour; 10. the lowest neighbour address.  IOS's "oldest external path" is not a step
// here.  The best path goes into the RIB with distance 20 (external) or 200 (internal), its MED
// as metric, its next hop and the interface towards it (RouteSrc::Bgp, "B"); local paths are
// not installed (the RIB has them already).
//
// Advertisement (9.2): every Established neighbour gets the best path of each prefix, except the
// neighbour it came from and an internal neighbour's path to other internal ones (no route
// reflection).  External neighbours get the own AS prepended, the session's local address as
// next hop, no LOCAL_PREF and a MED only for paths that started in the own AS; internal ones the
// path's next hop (the session's local address for local paths and with next-hop-self),
// LOCAL_PREF and MED.  Unknown optional transitive attributes pass on with the Partial bit,
// others do not.  Changes go out in UPDATEs with as many prefixes of the same attributes as fit
// in 4096 bytes: withdrawals at once, announcements at most every MinRouteAdvertisementInterval
// per neighbour (30 s external, 0 internal, as IOS; per neighbour rather than per prefix,
// 9.2.1.1), the first table after Established at once.
//
// Configuration takes effect at once (IOS wants clear ip bgp ... soft for weight, next-hop-self
// and the local preference); timers bgp counts for the next session.
//
// Not here: route maps, prefix lists and other policies (neighbor ... route-map, distribute-list,
// filter-list, prefix-list), communities of the own (COMMUNITIES pass as received), aggregation
// (aggregate-address), route reflection, confederations, peer groups, MD5 authentication,
// maximum-prefix, BGP over IPv6 and other address families, graceful restart, dampening,
// synchronization, multipath.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use jfn_model::catalog::F_ROUTER;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::bgp::{self as wire, Attr, AttrVal, Cap, Message, Notification, Open, Segment, Update};
use netpkt::{IpAddr, Ipv4, Prefix4};

use crate::action::ActErr;
use crate::bgptrace::BNote;
use crate::ip::{Route, RouteSrc};
use crate::kernel::{Engine, Ns, SEC};
use crate::os::{Job, Metric, OsApi, Service, SvcEvent, ANY4, SLOT_SVC, SVC_TIMERS};
use crate::series::MetricKind;
use crate::tcp::{SockOpts, TcpEnd};
use crate::trace::Note;

/// the kind and the name of the service
pub const KIND: &str = "bgp";
/// administrative distances of external and internal routes
pub const AD_EBGP: u8 = 20;
pub const AD_IBGP: u8 = 200;
/// the hold time and the keepalive interval by default (IOS: "timers bgp 60 180")
pub const HOLD: u16 = 180;
pub const KEEPALIVE: u16 = 60;
/// ConnectRetry (RFC 4271 10), the hold time while waiting for the OPEN (8.2.2), the pause
/// after a refused OPEN (our choice)
pub const CONNECT_RETRY: Ns = 120 * SEC;
pub const OPEN_HOLD: Ns = 240 * SEC;
pub const IDLE_HOLD: Ns = 60 * SEC;
/// MinRouteAdvertisementInterval towards external and internal neighbours (IOS)
pub const MRAI_EBGP: Ns = 30 * SEC;
pub const MRAI_IBGP: Ns = 0;
/// the weight of local paths and the local preference by default
pub const WEIGHT_LOCAL: u32 = 32768;
pub const LOCAL_PREF: u32 = 100;
/// the TTL of "ebgp-multihop" without a number
pub const MULTIHOP: u8 = 255;

/// timer keys: the deadline of the process, a look at the device (Engine::bgp_kick)
const T_TICK: u32 = 0;
const T_LOOK: u32 = 3;

// ---------------------------------------------------------------- states and reasons

/// the state of a session (RFC 4271 8.2.2)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Idle,
    Connect,
    Active,
    OpenSent,
    OpenConfirm,
    Established,
}

impl State {
    /// the RFC's name
    pub fn name(self) -> &'static str {
        match self {
            State::Idle => "Idle",
            State::Connect => "Connect",
            State::Active => "Active",
            State::OpenSent => "OpenSent",
            State::OpenConfirm => "OpenConfirm",
            State::Established => "Established",
        }
    }
}

/// why a neighbour stays Idle
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleWhy {
    /// neighbor shutdown
    Admin,
    /// the process has no router ID yet
    NoRouterId,
    /// an external neighbour outside the connected networks without ebgp-multihop
    NotConnected,
    /// the update-source interface has no address or is down
    NoSource,
    /// the pause after a refused OPEN (IDLE_HOLD)
    Hold,
}

impl IdleWhy {
    /// German, inside sentences (the app translates it)
    pub fn text(self) -> &'static str {
        // i18n
        match self {
            IdleWhy::Admin => "abgeschaltet (neighbor shutdown)",
            IdleWhy::NoRouterId => "noch keine Router-ID",
            IdleWhy::NotConnected => "nicht direkt verbunden, ebgp-multihop fehlt",
            IdleWhy::NoSource => "die update-source hat keine Adresse",
            IdleWhy::Hold => "Pause nach einem Fehler",
        }
    }
    /// English, for the console
    pub fn en(self) -> &'static str {
        match self {
            IdleWhy::Admin => "shut down",
            IdleWhy::NoRouterId => "no router ID yet",
            IdleWhy::NotConnected => "not directly connected, no ebgp-multihop",
            IdleWhy::NoSource => "the update-source has no address",
            IdleWhy::Hold => "a pause after an error",
        }
    }
}

/// why a session ended
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownWhy {
    /// this side sent a NOTIFICATION (code, subcode)
    Sent(u8, u8),
    /// the neighbour sent one
    Rcvd(u8, u8),
    /// the TCP connection ended without one
    Tcp(TcpEnd),
    /// the network towards a directly connected external neighbour went away
    IfDown,
}

/// why a connection from outside was reset
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefuseWhy {
    /// no neighbour has this address
    Unknown,
    /// the neighbour is shut down
    Shutdown,
    /// its session is Established already
    Established,
}

/// why a route of an UPDATE was not taken
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropWhy {
    /// the own AS is in the path
    Loop(u32),
    /// the next hop is an address of this device
    OwnNextHop(Ipv4),
    /// the next hop is no unicast address
    BadNextHop(Ipv4),
}

/// the step of the decision process that left one path
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    /// there was only one candidate
    Only,
    Weight,
    LocalPref,
    Local,
    AsPath,
    Origin,
    Med,
    External,
    IgpMetric,
    RouterId,
    Address,
}

impl Step {
    /// German, inside sentences (the app translates it)
    pub fn text(self) -> &'static str {
        // i18n
        match self {
            Step::Only => "der einzige Weg",
            Step::Weight => "höheres Gewicht",
            Step::LocalPref => "höhere LOCAL_PREF",
            Step::Local => "selbst erzeugt",
            Step::AsPath => "kürzerer AS-Pfad",
            Step::Origin => "niedrigerer ORIGIN",
            Step::Med => "niedrigere MED",
            Step::External => "extern vor intern",
            Step::IgpMetric => "kürzerer Weg zum Next Hop",
            Step::RouterId => "niedrigere BGP-ID",
            Step::Address => "niedrigere Nachbaradresse",
        }
    }
    /// English, for the console and show commands
    pub fn en(self) -> &'static str {
        match self {
            Step::Only => "the only path",
            Step::Weight => "higher weight",
            Step::LocalPref => "higher local preference",
            Step::Local => "originated here",
            Step::AsPath => "shorter AS path",
            Step::Origin => "lower origin",
            Step::Med => "lower MED",
            Step::External => "external before internal",
            Step::IgpMetric => "nearer next hop",
            Step::RouterId => "lower BGP identifier",
            Step::Address => "lower neighbour address",
        }
    }
}

/// where a local path comes from (and a source of redistribution)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum How {
    Network,
    Connected,
    Static,
    Rip,
    Ospf,
}

impl How {
    /// the words of the configuration line
    pub fn word(self) -> &'static str {
        match self {
            How::Network => "network",
            How::Connected => "redistribute connected",
            How::Static => "redistribute static",
            How::Rip => "redistribute rip",
            How::Ospf => "redistribute ospf",
        }
    }
}

// ---------------------------------------------------------------- configuration

/// "redistribute connected|static|rip|ospf N [metric M]"
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Redist {
    pub what: How,
    /// the OSPF process (0 for the others)
    pub pid: u16,
    /// the MED the routes get (else their metric)
    pub metric: Option<u32>,
}

/// one neighbour
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NbrCfg {
    pub remote_as: u32,
    pub descr: String,
    /// the interface whose address the session uses (full name)
    pub update_source: Option<String>,
    pub next_hop_self: bool,
    /// the TTL towards an external neighbour that is not directly connected
    pub multihop: Option<u8>,
    pub shutdown: bool,
    /// the weight of its paths (0 without one)
    pub weight: Option<u32>,
}

impl NbrCfg {
    pub fn new(remote_as: u32) -> NbrCfg {
        NbrCfg { remote_as, descr: String::new(), update_source: None, next_hop_self: false, multihop: None, shutdown: false, weight: None }
    }
}

/// what "router bgp" says
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BgpCfg {
    pub on: bool,
    pub asn: u32,
    pub router_id: Option<Ipv4>,
    /// "bgp log-neighbor-changes" (on by default)
    pub log_changes: bool,
    /// "timers bgp KEEPALIVE HOLD"
    pub keepalive: u16,
    pub hold: u16,
    /// "bgp default local-preference"
    pub local_pref: u32,
    /// "network A mask M"
    pub networks: Vec<Prefix4>,
    pub redistribute: Vec<Redist>,
    pub nbrs: BTreeMap<Ipv4, NbrCfg>,
}

impl Default for BgpCfg {
    fn default() -> BgpCfg {
        BgpCfg { on: false, asn: 0, router_id: None, log_changes: true, keepalive: KEEPALIVE, hold: HOLD, local_pref: LOCAL_PREF, networks: Vec::new(), redistribute: Vec::new(), nbrs: BTreeMap::new() }
    }
}

impl BgpCfg {
    /// the neighbour is in another AS
    pub fn ebgp(&self, a: Ipv4) -> bool {
        self.nbrs.get(&a).is_some_and(|n| n.remote_as != self.asn)
    }
    fn reset(&mut self) {
        *self = BgpCfg::default();
    }
}

/// how "clear ip bgp" resets
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Soft {
    /// a new session (Cease / Administrative Reset)
    Hard,
    /// ask the neighbour for its routes again (ROUTE-REFRESH)
    In,
    /// send the own ones again
    Out,
    Both,
}

/// a change of the configuration (Action::Bgp)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BgpSet {
    /// "router bgp ASN" / "no router bgp ASN"
    Process { asn: u32, on: bool },
    RouterId(Option<Ipv4>),
    LogChanges(bool),
    /// "timers bgp K H" (None: the defaults)
    Timers(Option<(u16, u16)>),
    LocalPref(Option<u32>),
    Network { net: Prefix4, add: bool },
    Redistribute { r: Redist, add: bool },
    /// "neighbor X remote-as N"; None: "no neighbor X"
    RemoteAs { addr: Ipv4, asn: Option<u32> },
    Descr { addr: Ipv4, text: Option<String> },
    UpdateSource { addr: Ipv4, iface: Option<String> },
    NextHopSelf { addr: Ipv4, on: bool },
    Multihop { addr: Ipv4, ttl: Option<u8> },
    Shutdown { addr: Ipv4, on: bool },
    Weight { addr: Ipv4, weight: Option<u32> },
    /// "clear ip bgp * | X [soft [in|out]]"
    Clear { addr: Option<Ipv4>, soft: Soft },
}

/// what the configuration asks of the running service
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Req {
    /// ends the sessions of one neighbour (None: all) with Cease and this subcode
    Reset(Option<Ipv4>, u8),
    SoftIn(Option<Ipv4>),
    SoftOut(Option<Ipv4>),
    /// "bgp router-id" changed
    NewId,
}

// ---------------------------------------------------------------- paths

/// the path attributes of a route as they go on the wire
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attrs {
    pub origin: u8,
    pub as_path: Vec<Segment>,
    pub next_hop: Ipv4,
    pub med: Option<u32>,
    pub local_pref: Option<u32>,
    pub atomic: bool,
    pub aggregator: Option<(u32, Ipv4)>,
    pub communities: Vec<u32>,
    /// unknown optional transitive attributes, passed on
    pub other: Vec<Attr>,
}

impl Attrs {
    fn local(origin: u8, med: Option<u32>) -> Attrs {
        Attrs { origin, as_path: Vec::new(), next_hop: Ipv4::UNSPECIFIED, med, local_pref: None, atomic: false, aggregator: None, communities: Vec::new(), other: Vec::new() }
    }

    /// the attributes of a received UPDATE; a 2-octet session's AS4_PATH and AS4_AGGREGATOR go
    /// into AS_PATH and AGGREGATOR (RFC 6793 4.2.3)
    fn of(u: &Update, as4: bool) -> Attrs {
        let mut as_path: Vec<Segment> = u.as_path().map(|s| s.to_vec()).unwrap_or_default();
        if !as4 && let Some(p4) = u.as4_path() {
            let (n, m) = (wire::path_len(&as_path), wire::path_len(p4));
            if m <= n {
                // the first n - m numbers of AS_PATH, then AS4_PATH
                let mut keep = n - m;
                let mut out: Vec<Segment> = Vec::new();
                for s in &as_path {
                    if keep == 0 {
                        break;
                    }
                    if s.set {
                        out.push(s.clone());
                        keep -= 1;
                    } else {
                        let k = keep.min(s.asns.len());
                        out.push(Segment::seq(s.asns[..k].to_vec()));
                        keep -= k;
                    }
                }
                out.extend(p4.iter().cloned());
                as_path = out;
            }
        }
        let mut aggregator = match u.get(wire::A_AGGREGATOR) {
            Some(AttrVal::Aggregator { asn, id }) => Some((*asn, *id)),
            _ => None,
        };
        if !as4
            && aggregator.is_some_and(|a| a.0 == wire::AS_TRANS as u32)
            && let Some(AttrVal::As4Aggregator { asn, id }) = u.get(wire::A_AS4_AGGREGATOR)
        {
            aggregator = Some((*asn, *id));
        }
        let other = u
            .attrs
            .iter()
            .filter(|a| matches!(a.val, AttrVal::Other { .. }) && a.flags & wire::F_OPTIONAL != 0 && a.flags & wire::F_TRANSITIVE != 0 && a.kind() != wire::A_AS4_PATH && a.kind() != wire::A_AS4_AGGREGATOR)
            .map(|a| Attr { flags: a.flags | wire::F_PARTIAL, val: a.val.clone() })
            .collect();
        Attrs {
            origin: u.origin().unwrap_or(wire::ORIGIN_INCOMPLETE),
            as_path,
            next_hop: u.next_hop().unwrap_or(Ipv4::UNSPECIFIED),
            med: u.med(),
            local_pref: u.local_pref(),
            atomic: u.get(wire::A_ATOMIC_AGGREGATE).is_some(),
            aggregator,
            communities: u.communities().map(|c| c.to_vec()).unwrap_or_default(),
            other,
        }
    }

    /// the attributes for an UPDATE, in the order of their types; on a 2-octet session AS
    /// numbers above 65535 are AS_TRANS with AS4_PATH and AS4_AGGREGATOR beside (RFC 6793 4.2.2)
    fn wire(&self, as4: bool) -> Vec<Attr> {
        let mut v = alloc::vec![Attr::new(AttrVal::Origin(self.origin)), Attr::new(AttrVal::AsPath(self.as_path.clone())), Attr::new(AttrVal::NextHop(self.next_hop))];
        if let Some(m) = self.med {
            v.push(Attr::new(AttrVal::Med(m)));
        }
        if let Some(l) = self.local_pref {
            v.push(Attr::new(AttrVal::LocalPref(l)));
        }
        if self.atomic {
            v.push(Attr::new(AttrVal::AtomicAggregate));
        }
        if let Some((asn, id)) = self.aggregator {
            v.push(Attr::new(AttrVal::Aggregator { asn, id }));
        }
        if !self.communities.is_empty() {
            v.push(Attr::new(AttrVal::Communities(self.communities.clone())));
        }
        let big = |a: u32| a > 0xFFFF;
        if !as4 && self.as_path.iter().flat_map(|s| s.asns.iter()).any(|&a| big(a)) {
            v.push(Attr::new(AttrVal::As4Path(self.as_path.clone())));
        }
        if !as4 && let Some((asn, id)) = self.aggregator.filter(|a| big(a.0)) {
            v.push(Attr::new(AttrVal::As4Aggregator { asn, id }));
        }
        v.extend(self.other.iter().cloned());
        v
    }

    /// the neighbouring AS of the path for the MED step: the first AS of a leading sequence,
    /// `own` for an empty path, None for a path that starts with a set
    fn first_as(&self, own: u32) -> Option<u32> {
        match self.as_path.first() {
            None => Some(own),
            Some(s) if !s.set => s.asns.first().copied(),
            Some(_) => None,
        }
    }
}

/// where a path came from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Src {
    Local(How),
    Peer(Ipv4),
}

/// a path to a prefix
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path {
    pub attrs: Attrs,
    pub from: Src,
    /// the BGP identifier of the neighbour (the own router ID for local paths)
    pub peer_id: Ipv4,
    pub ebgp: bool,
    /// when it came
    pub since: Ns,
}

/// a candidate of the decision process
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cand {
    pub path: Path,
    /// its next hop resolves (local paths always)
    pub ok: bool,
    /// the metric of the route towards the next hop, and its interface
    pub igp: u32,
    pub iface: Option<u16>,
    pub weight: u32,
    /// the local preference it is compared with
    pub lp: u32,
}

/// a prefix in the Loc-RIB
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dest {
    pub cands: Vec<Cand>,
    pub best: Option<usize>,
    /// the step that chose it
    pub step: Step,
    /// since when the best path is as it is
    pub since: Ns,
    /// the table version of its last change
    pub ver: u64,
}

impl Dest {
    pub fn best(&self) -> Option<&Cand> {
        self.cands.get(self.best?)
    }
}

// ---------------------------------------------------------------- sessions

/// the state of a TCP connection of a session
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CState {
    /// SYN sent, not connected yet
    Connecting,
    OpenSent,
    OpenConfirm,
    Established,
}

/// a connection of a session (two while a collision is resolved)
#[derive(Clone, Debug)]
pub struct Conn {
    pub sock: u32,
    /// this side opened it
    pub out: bool,
    pub st: CState,
    /// the local address and the ports (once connected)
    pub local: Ipv4,
    pub lport: u16,
    pub rport: u16,
    rbuf: Vec<u8>,
    wbuf: Vec<u8>,
    hold_at: Option<Ns>,
    ka_at: Option<Ns>,
    /// what was negotiated on it (seconds)
    hold: u16,
    ka: u16,
    /// the neighbour's OPEN
    open: Option<Open>,
}

impl Conn {
    fn new(sock: u32, out: bool, st: CState) -> Conn {
        Conn { sock, out, st, local: Ipv4::UNSPECIFIED, lport: 0, rport: 0, rbuf: Vec::new(), wbuf: Vec::new(), hold_at: None, ka_at: None, hold: 0, ka: 0, open: None }
    }
    /// 4-octet AS numbers on this connection (both sides offered them; ours always does)
    fn as4(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.as4().is_some())
    }
}

/// what a neighbour's sessions counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PeerCounters {
    pub msgs_in: u64,
    pub msgs_out: u64,
    pub opens_in: u64,
    pub opens_out: u64,
    pub updates_in: u64,
    pub updates_out: u64,
    pub keepalives_in: u64,
    pub keepalives_out: u64,
    pub notifications_in: u64,
    pub notifications_out: u64,
    pub refresh_in: u64,
    pub refresh_out: u64,
    /// sessions that came up, and that ended
    pub established: u32,
    pub dropped: u32,
    /// connection attempts
    pub attempts: u32,
}

/// a neighbour and its session
#[derive(Clone, Debug)]
pub struct Peer {
    pub addr: Ipv4,
    /// Idle, Connect or Active while no connection is beyond its handshake
    base: State,
    /// why it stays Idle
    pub idle: Option<IdleWhy>,
    pub conns: Vec<Conn>,
    retry_at: Option<Ns>,
    idle_until: Option<Ns>,
    mrai_at: Option<Ns>,
    /// the last attempt found no route: try again as soon as there is one
    no_route: bool,
    /// announcements wait for MRAI
    waiting: bool,
    /// the state the notes last gave
    shown: State,
    /// since when the session is up, or down (None: never up)
    pub since: Option<Ns>,
    /// what the established session uses
    pub peer_id: Ipv4,
    pub peer_as: u32,
    pub hold: u16,
    pub keepalive: u16,
    pub as4: bool,
    pub refresh: bool,
    pub local: Ipv4,
    pub lport: u16,
    pub rport: u16,
    pub n: PeerCounters,
    /// the end of the last session and why
    pub last_down: Option<(Ns, DownWhy)>,
    /// the last NOTIFICATION: when, sent (else received), code, subcode
    pub last_error: Option<(Ns, bool, u8, u8)>,
    /// the last failed connection attempt
    pub fail: Option<TcpEnd>,
    /// the routes it sent (accepted), and those sent to it
    pub adj_in: BTreeMap<Prefix4, Path>,
    pub adj_out: BTreeMap<Prefix4, Attrs>,
    /// the table version of the last UPDATE sent
    pub tbl_ver: u64,
}

impl Peer {
    fn new(addr: Ipv4) -> Peer {
        Peer {
            addr,
            base: State::Idle,
            idle: None,
            conns: Vec::new(),
            retry_at: None,
            idle_until: None,
            mrai_at: None,
            no_route: false,
            waiting: false,
            shown: State::Idle,
            since: None,
            peer_id: Ipv4::UNSPECIFIED,
            peer_as: 0,
            hold: 0,
            keepalive: 0,
            as4: false,
            refresh: false,
            local: Ipv4::UNSPECIFIED,
            lport: 0,
            rport: 0,
            n: PeerCounters::default(),
            last_down: None,
            last_error: None,
            fail: None,
            adj_in: BTreeMap::new(),
            adj_out: BTreeMap::new(),
            tbl_ver: 0,
        }
    }
    /// the state of the session: the furthest connection, else Idle, Connect or Active
    pub fn state(&self) -> State {
        match self.conns.iter().map(|c| c.st).max() {
            Some(CState::Established) => State::Established,
            Some(CState::OpenConfirm) => State::OpenConfirm,
            Some(CState::OpenSent) => State::OpenSent,
            Some(CState::Connecting) => State::Connect,
            None => self.base,
        }
    }
    /// the established connection
    pub fn session(&self) -> Option<&Conn> {
        self.conns.iter().find(|c| c.st == CState::Established)
    }
    /// when ConnectRetry fires next
    pub fn retry_at(&self) -> Option<Ns> {
        self.retry_at
    }
    /// the hold timer and the keepalive timer of the established session
    pub fn timers(&self) -> (Option<Ns>, Option<Ns>) {
        self.session().map(|c| (c.hold_at, c.ka_at)).unwrap_or((None, None))
    }
    /// when the waiting announcements may go
    pub fn mrai_at(&self) -> Option<Ns> {
        if self.waiting { self.mrai_at } else { None }
    }
    /// whole messages received and not handled yet, and those waiting for room in TCP's send
    /// buffer (show ip bgp summary: InQ, OutQ)
    pub fn queued(&self) -> (usize, usize) {
        let count = |b: &[u8]| {
            let mut n = 0;
            let mut i = 0;
            while let Ok(Some(l)) = wire::frame(&b[i..]) {
                n += 1;
                i += l;
            }
            n
        };
        (self.conns.iter().map(|c| count(&c.rbuf)).sum(), self.conns.iter().map(|c| count(&c.wbuf)).sum())
    }
}

/// what the process counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub opens_sent: u64,
    pub opens_rcvd: u64,
    pub updates_sent: u64,
    pub updates_rcvd: u64,
    pub keepalives_sent: u64,
    pub keepalives_rcvd: u64,
    pub notifications_sent: u64,
    pub notifications_rcvd: u64,
    pub refresh_sent: u64,
    pub refresh_rcvd: u64,
    pub connects: u64,
    pub refused: u64,
    pub flaps: u64,
    pub dropped_routes: u64,
}

/// BGP on a device
#[derive(Clone, Debug)]
pub struct Bgp {
    pub cfg: BgpCfg,
    running: bool,
    /// the router ID in use (None: no address to take one from yet)
    rid: Option<Ipv4>,
    started: Ns,
    listener: Option<u32>,
    peers: BTreeMap<Ipv4, Peer>,
    rib: BTreeMap<Prefix4, Dest>,
    local: BTreeMap<Prefix4, Path>,
    /// the table version: one more with every change of a best path
    ver: u64,
    reqs: Vec<Req>,
    armed: Option<Ns>,
    pub n: Counters,
}

impl Default for Bgp {
    fn default() -> Bgp {
        Bgp::new()
    }
}

fn note(os: &mut OsApi, n: BNote) {
    os.note(Note::Bgp(n));
}

// ---------------------------------------------------------------- what others read

impl Bgp {
    pub fn new() -> Bgp {
        Bgp { cfg: BgpCfg::default(), running: false, rid: None, started: 0, listener: None, peers: BTreeMap::new(), rib: BTreeMap::new(), local: BTreeMap::new(), ver: 1, reqs: Vec::new(), armed: None, n: Counters::default() }
    }
    pub fn running(&self) -> bool {
        self.running
    }
    pub fn router_id(&self) -> Option<Ipv4> {
        self.rid
    }
    /// when the process started
    pub fn started(&self) -> Ns {
        self.started
    }
    pub fn peers(&self) -> &BTreeMap<Ipv4, Peer> {
        &self.peers
    }
    pub fn peer(&self, a: Ipv4) -> Option<&Peer> {
        self.peers.get(&a)
    }
    pub fn loc_rib(&self) -> &BTreeMap<Prefix4, Dest> {
        &self.rib
    }
    /// the paths this router originates
    pub fn local(&self) -> &BTreeMap<Prefix4, Path> {
        &self.local
    }
    /// the table version
    pub fn version(&self) -> u64 {
        self.ver
    }
    /// (neighbours, established ones)
    pub fn neighbours(&self) -> (usize, usize) {
        (self.cfg.nbrs.len(), self.peers.values().filter(|p| p.state() == State::Established).count())
    }
    /// the number of paths in the Loc-RIB
    pub fn path_count(&self) -> usize {
        self.rib.values().map(|d| d.cands.len()).sum()
    }
}

// ---------------------------------------------------------------- the device and the RIB

/// the RIB's best route for an address among `rib` (longest prefix, distance, metric)
fn rib_best(rib: &[Route], ip: Ipv4) -> Option<&Route> {
    let mut b: Option<&Route> = None;
    for r in rib {
        if !r.prefix.contains(ip) {
            continue;
        }
        let better = match b {
            None => true,
            Some(c) => (r.prefix.len, core::cmp::Reverse(r.ad), core::cmp::Reverse(r.metric)) > (c.prefix.len, core::cmp::Reverse(c.ad), core::cmp::Reverse(c.metric)),
        };
        if better {
            b = Some(r);
        }
    }
    b
}

/// an address a route may lead to (not 0, not a group, not 127/8, not the broadcast)
fn unicast(a: Ipv4) -> bool {
    !(a.is_unspecified() || a.is_multicast() || a.is_broadcast() || a.is_loopback() || a.0[0] >= 240)
}

impl Bgp {
    /// the router ID: as configured, else the highest loopback address, else the highest
    /// address of an interface that is up
    fn pick_rid(&self, os: &OsApi) -> Option<Ipv4> {
        if let Some(r) = self.cfg.router_id {
            return Some(r);
        }
        let mut lo: Option<Ipv4> = None;
        let mut other: Option<Ipv4> = None;
        for (i, f) in os.ifaces().iter().enumerate() {
            if f.hidden || !os.iface_up(i as u16) {
                continue;
            }
            for p in &f.addrs {
                let slot = if f.is_loopback() { &mut lo } else { &mut other };
                if slot.is_none_or(|x| p.addr > x) {
                    *slot = Some(p.addr);
                }
            }
        }
        lo.or(other)
    }

    /// the RIB without BGP's routes
    fn igp_rib(os: &OsApi) -> Vec<Route> {
        let ni = os.ni;
        let Some(st) = os.eng.stack(ni) else {
            return Vec::new();
        };
        let mut v = st.rib(&|i| os.eng.iface_up(ni, i));
        v.retain(|r| r.src != RouteSrc::Bgp);
        v
    }

    /// the neighbour lies in a network of an interface that is up
    fn connected(os: &OsApi, a: Ipv4) -> bool {
        os.ifaces().iter().enumerate().any(|(i, f)| !f.hidden && !f.is_loopback() && os.iface_up(i as u16) && f.on_link(a))
    }

    /// the address a session to `a` starts from: Some(None) the one of the way out, None when
    /// the update-source has no address (or is down)
    fn source(&self, os: &OsApi, a: Ipv4) -> Option<Option<Ipv4>> {
        match self.cfg.nbrs.get(&a).and_then(|n| n.update_source.clone()) {
            None => Some(None),
            Some(name) => {
                let i = os.iface_by_name(&name)?;
                if !os.iface_up(i) {
                    return None;
                }
                os.iface(i)?.primary().map(Some)
            }
        }
    }

    /// the TTL of a session's segments
    fn ttl_for(&self, a: Ipv4) -> u8 {
        match self.cfg.nbrs.get(&a) {
            Some(n) if n.remote_as != self.cfg.asn => n.multihop.unwrap_or(1),
            _ => 0,
        }
    }

    fn opts_for(&self, a: Ipv4) -> SockOpts {
        SockOpts { nodelay: true, ttl: self.ttl_for(a), ..SockOpts::default() }
    }
}

// ---------------------------------------------------------------- messages

impl Bgp {
    fn conn_of(&self, sock: u32) -> Option<(Ipv4, usize)> {
        self.peers.iter().find_map(|(a, p)| p.conns.iter().position(|c| c.sock == sock).map(|i| (*a, i)))
    }

    /// queues a message on a connection and sends what TCP takes
    fn send(&mut self, os: &mut OsApi, a: Ipv4, ci: usize, m: &Message) {
        let now = os.now();
        let Some(c) = self.peers.get_mut(&a).and_then(|p| p.conns.get_mut(ci)) else {
            return;
        };
        let bytes = m.to_bytes(c.as4());
        c.wbuf.extend_from_slice(&bytes);
        if matches!(m, Message::Keepalive | Message::Update(_)) && c.ka > 0 && c.st >= CState::OpenConfirm {
            c.ka_at = Some(now + c.ka as Ns * SEC);
        }
        let sock = c.sock;
        let n = os.tcp_send(sock, &c.wbuf);
        c.wbuf.drain(..n);
        let p = self.peers.get_mut(&a).map(|p| &mut p.n);
        if let Some(pc) = p {
            pc.msgs_out += 1;
            match m {
                Message::Open(_) => pc.opens_out += 1,
                Message::Update(_) => pc.updates_out += 1,
                Message::Keepalive => pc.keepalives_out += 1,
                Message::Notification(_) => pc.notifications_out += 1,
                Message::RouteRefresh { .. } => pc.refresh_out += 1,
            }
        }
        match m {
            Message::Open(o) => {
                self.n.opens_sent += 1;
                note(os, BNote::OpenSent { peer: a, asn: self.cfg.asn, hold: o.hold, id: o.id });
            }
            Message::Update(u) => {
                self.n.updates_sent += 1;
                note(os, BNote::Update { peer: a, out: true, adv: u.nlri.len() as u16, wd: u.withdrawn.len() as u16 });
            }
            Message::Keepalive => {
                self.n.keepalives_sent += 1;
                note(os, BNote::Keepalive { peer: a, out: true });
            }
            Message::Notification(x) => {
                self.n.notifications_sent += 1;
                note(os, BNote::Notification { peer: a, out: true, code: x.code, sub: x.sub });
                if os.ios() {
                    let ni = os.ni;
                    os.eng.ios_log(ni, &alloc::format!("%BGP-3-NOTIFICATION: sent to neighbour {}: {}/{} ({})", a, x.code, x.sub, wire::error_name(x.code, x.sub)));
                }
            }
            Message::RouteRefresh { .. } => {
                self.n.refresh_sent += 1;
                note(os, BNote::Refresh { peer: a, out: true });
            }
        }
    }

    /// what waits for room in the send buffer
    fn flush(&mut self, os: &mut OsApi, sock: u32) {
        let Some((a, ci)) = self.conn_of(sock) else {
            return;
        };
        if let Some(c) = self.peers.get_mut(&a).and_then(|p| p.conns.get_mut(ci))
            && !c.wbuf.is_empty()
        {
            let n = os.tcp_send(sock, &c.wbuf);
            c.wbuf.drain(..n);
        }
    }

    /// the OPEN of this router
    fn open_msg(&self) -> Message {
        let caps = alloc::vec![Cap::Mp { afi: wire::AFI_IPV4, safi: wire::SAFI_UNICAST }, Cap::RouteRefresh, Cap::As4(self.cfg.asn)];
        Message::Open(Open::new(self.cfg.asn, self.cfg.hold, self.rid.unwrap_or_default(), caps))
    }

    /// a connection is up: the OPEN goes out, the long hold time of OpenSent starts
    fn conn_up(&mut self, os: &mut OsApi, a: Ipv4, ci: usize) {
        let now = os.now();
        let info = self.peers.get(&a).and_then(|p| p.conns.get(ci)).and_then(|c| os.tcp_info(c.sock));
        let m = self.open_msg();
        if let Some(c) = self.peers.get_mut(&a).and_then(|p| p.conns.get_mut(ci)) {
            c.st = CState::OpenSent;
            c.hold_at = Some(now + OPEN_HOLD);
            if let Some(r) = info {
                if let IpAddr::V4(l) = r.local.0 {
                    c.local = l;
                }
                c.lport = r.local.1;
                c.rport = r.remote.1;
            }
        }
        if let Some(p) = self.peers.get_mut(&a) {
            p.retry_at = None;
            p.fail = None;
        }
        self.send(os, a, ci, &m);
    }

    /// closes a connection: `notify` sends that NOTIFICATION first (then FIN), `abort` resets,
    /// else the socket is gone already or closes with FIN; the session ends when it was the
    /// established one, else the neighbour waits as `why` says
    fn drop_conn(&mut self, os: &mut OsApi, a: Ipv4, ci: usize, why: DownWhy, notify: Option<Notification>, abort: bool, gone: bool) {
        let now = os.now();
        if let Some(n) = &notify {
            self.send(os, a, ci, &Message::Notification(n.clone()));
            if let Some(p) = self.peers.get_mut(&a) {
                p.last_error = Some((now, true, n.code, n.sub));
            }
        }
        let Some(p) = self.peers.get_mut(&a) else {
            return;
        };
        if ci >= p.conns.len() {
            return;
        }
        let c = p.conns.remove(ci);
        if !gone {
            if abort {
                os.tcp_abort(c.sock);
            } else {
                os.tcp_close(c.sock);
            }
        }
        if c.st == CState::Established {
            self.session_down(os, a, why);
            return;
        }
        let Some(p) = self.peers.get_mut(&a) else {
            return;
        };
        if !p.conns.is_empty() {
            return;
        }
        // nothing left: an error before Established pauses, a TCP failure waits for ConnectRetry
        match why {
            DownWhy::Sent(c, _) if c != wire::E_CEASE => {
                p.base = State::Idle;
                p.idle_until = Some(now + IDLE_HOLD);
            }
            DownWhy::Rcvd(c, _) if c != wire::E_CEASE => {
                p.base = State::Idle;
                p.idle_until = Some(now + IDLE_HOLD);
            }
            DownWhy::Tcp(_) | DownWhy::Sent(..) | DownWhy::Rcvd(..) => {
                p.base = State::Active;
                if p.retry_at.is_none() {
                    p.retry_at = Some(now + CONNECT_RETRY);
                }
            }
            DownWhy::IfDown => {
                p.base = State::Idle;
                p.idle_until = None;
            }
        }
        p.retry_at = if p.base == State::Idle { None } else { p.retry_at };
    }

    /// the established session ends: its routes go, the neighbour starts again at once
    fn session_down(&mut self, os: &mut OsApi, a: Ipv4, why: DownWhy) {
        let now = os.now();
        let rest: Vec<u32> = match self.peers.get_mut(&a) {
            Some(p) => {
                p.adj_in.clear();
                p.adj_out.clear();
                p.waiting = false;
                p.mrai_at = None;
                p.since = Some(now);
                p.last_down = Some((now, why));
                p.n.dropped += 1;
                p.base = State::Idle;
                p.idle_until = None;
                p.retry_at = None;
                p.conns.drain(..).map(|c| c.sock).collect()
            }
            None => return,
        };
        // a connection left from a collision goes too
        for s in rest {
            os.tcp_abort(s);
        }
        self.n.flaps += 1;
        note(os, BNote::Down { peer: a, why });
    }

    /// the state notes, and the console lines of a session that comes or goes
    fn track(&mut self, os: &mut OsApi) {
        let ni = os.ni;
        let log = self.cfg.log_changes;
        let mut lines: Vec<String> = Vec::new();
        let mut notes: Vec<BNote> = Vec::new();
        for (a, p) in self.peers.iter_mut() {
            let st = p.state();
            if st == p.shown {
                continue;
            }
            notes.push(BNote::State { peer: *a, from: p.shown, to: st });
            if st == State::Established {
                lines.push(alloc::format!("%BGP-5-ADJCHANGE: neighbour {} is up (AS {})", a, p.peer_as));
            } else if p.shown == State::Established {
                let why = p.last_down.map(|x| x.1.en()).unwrap_or_default();
                lines.push(alloc::format!("%BGP-5-ADJCHANGE: neighbour {} is down: {}", a, why));
            }
            p.shown = st;
        }
        for n in notes {
            note(os, n);
        }
        if log && os.ios() {
            for l in lines {
                os.eng.ios_log(ni, &l);
            }
        }
    }
}

// ---------------------------------------------------------------- input

impl Bgp {
    /// connections waiting at the listening socket: a neighbour's are taken, others reset
    fn accept(&mut self, os: &mut OsApi, listener: u32) {
        while let Some(s) = os.tcp_accept(listener) {
            let Some(row) = os.tcp_info(s) else {
                os.tcp_abort(s);
                continue;
            };
            let (IpAddr::V4(from), IpAddr::V4(local)) = (row.remote.0, row.local.0) else {
                os.tcp_abort(s);
                continue;
            };
            let why = match (self.cfg.nbrs.get(&from), self.peers.get(&from)) {
                (None, _) | (_, None) => Some(RefuseWhy::Unknown),
                (Some(n), _) if n.shutdown => Some(RefuseWhy::Shutdown),
                (_, Some(p)) if p.session().is_some() => Some(RefuseWhy::Established),
                _ => None,
            };
            if let Some(w) = why {
                self.n.refused += 1;
                note(os, BNote::Refused { from, why: w });
                os.tcp_abort(s);
                continue;
            }
            // an own attempt still in SYN-SENT gives way
            let mine: Vec<u32> = self.peers.get(&from).map(|p| p.conns.iter().filter(|c| c.st == CState::Connecting).map(|c| c.sock).collect()).unwrap_or_default();
            for m in mine {
                os.tcp_abort(m);
            }
            let opts = self.opts_for(from);
            os.tcp_setopts(s, opts);
            note(os, BNote::Accepted { peer: from, local });
            let ci = match self.peers.get_mut(&from) {
                Some(p) => {
                    p.conns.retain(|c| c.st != CState::Connecting);
                    p.idle = None;
                    p.conns.push(Conn::new(s, false, CState::Connecting));
                    p.conns.len() - 1
                }
                None => continue,
            };
            self.conn_up(os, from, ci);
        }
    }

    /// bytes of a connection: every whole message in turn
    fn data(&mut self, os: &mut OsApi, sock: u32) {
        let d = os.tcp_recv(sock, usize::MAX);
        let Some((a, ci)) = self.conn_of(sock) else {
            return;
        };
        if let Some(c) = self.peers.get_mut(&a).and_then(|p| p.conns.get_mut(ci)) {
            c.rbuf.extend_from_slice(&d);
        }
        loop {
            // the connection may be gone or have moved by now
            let Some((a, ci)) = self.conn_of(sock) else {
                return;
            };
            let Some(c) = self.peers.get(&a).and_then(|p| p.conns.get(ci)) else {
                return;
            };
            let as4 = c.as4();
            let len = match wire::frame(&c.rbuf) {
                Ok(Some(n)) => n,
                Ok(None) => return,
                Err(e) => {
                    self.drop_conn(os, a, ci, DownWhy::Sent(e.code, e.sub), Some(e.notification()), false, false);
                    self.track(os);
                    return;
                }
            };
            let raw: Vec<u8> = match self.peers.get_mut(&a).and_then(|p| p.conns.get_mut(ci)) {
                Some(c) => c.rbuf.drain(..len).collect(),
                None => return,
            };
            if let Some(p) = self.peers.get_mut(&a) {
                p.n.msgs_in += 1;
            }
            match Message::read(&raw, as4) {
                Ok(m) => self.message(os, a, ci, m),
                Err(e) => self.drop_conn(os, a, ci, DownWhy::Sent(e.code, e.sub), Some(e.notification()), false, false),
            }
            self.track(os);
        }
    }

    /// one message on a connection, by the state of the connection
    fn message(&mut self, os: &mut OsApi, a: Ipv4, ci: usize, m: Message) {
        let now = os.now();
        let Some(c) = self.peers.get_mut(&a).and_then(|p| p.conns.get_mut(ci)) else {
            return;
        };
        let st = c.st;
        // whatever comes restarts the hold timer once the hold time is agreed
        if st >= CState::OpenConfirm && c.hold > 0 {
            c.hold_at = Some(now + c.hold as Ns * SEC);
        }
        match (st, m) {
            (_, Message::Notification(n)) => {
                self.n.notifications_rcvd += 1;
                if let Some(p) = self.peers.get_mut(&a) {
                    p.n.notifications_in += 1;
                    p.last_error = Some((now, false, n.code, n.sub));
                }
                note(os, BNote::Notification { peer: a, out: false, code: n.code, sub: n.sub });
                if os.ios() {
                    let ni = os.ni;
                    os.eng.ios_log(ni, &alloc::format!("%BGP-3-NOTIFICATION: received from neighbour {}: {}/{} ({})", a, n.code, n.sub, wire::error_name(n.code, n.sub)));
                }
                self.drop_conn(os, a, ci, DownWhy::Rcvd(n.code, n.sub), None, false, false);
            }
            (CState::OpenSent, Message::Open(o)) => self.open_in(os, a, ci, o),
            (CState::OpenConfirm, Message::Keepalive) => {
                self.count_ka(os, a);
                self.established(os, a, ci);
            }
            (CState::Established, Message::Keepalive) => self.count_ka(os, a),
            (CState::Established, Message::Update(u)) => self.update_in(os, a, ci, u),
            (CState::Established, Message::RouteRefresh { afi, safi }) => {
                self.n.refresh_rcvd += 1;
                if let Some(p) = self.peers.get_mut(&a) {
                    p.n.refresh_in += 1;
                }
                note(os, BNote::Refresh { peer: a, out: false });
                // another family than IPv4 unicast is ignored (RFC 2918 4)
                if afi == wire::AFI_IPV4 && safi == wire::SAFI_UNICAST
                    && let Some(p) = self.peers.get_mut(&a)
                {
                    p.adj_out.clear();
                    p.mrai_at = None;
                }
            }
            (st, _) => {
                // RFC 6608: the subcode names the state that got the unexpected message
                let sub = match st {
                    CState::OpenSent | CState::Connecting => 1,
                    CState::OpenConfirm => 2,
                    CState::Established => 3,
                };
                self.drop_conn(os, a, ci, DownWhy::Sent(wire::E_FSM, sub), Some(Notification::new(wire::E_FSM, sub)), false, false);
            }
        }
    }

    fn count_ka(&mut self, os: &mut OsApi, a: Ipv4) {
        self.n.keepalives_rcvd += 1;
        if let Some(p) = self.peers.get_mut(&a) {
            p.n.keepalives_in += 1;
        }
        note(os, BNote::Keepalive { peer: a, out: false });
    }

    /// the neighbour's OPEN (6.2, 6.8, 8.2.2 OpenSent)
    fn open_in(&mut self, os: &mut OsApi, a: Ipv4, ci: usize, o: Open) {
        let now = os.now();
        self.n.opens_rcvd += 1;
        if let Some(p) = self.peers.get_mut(&a) {
            p.n.opens_in += 1;
        }
        note(os, BNote::OpenRcvd { peer: a, asn: o.peer_as(), hold: o.hold, id: o.id });
        let want = self.cfg.nbrs.get(&a).map(|n| n.remote_as).unwrap_or(0);
        if o.peer_as() != want {
            let n = Notification { code: wire::E_OPEN, sub: wire::OE_PEER_AS, data: o.asn.to_be_bytes().to_vec() };
            self.drop_conn(os, a, ci, DownWhy::Sent(wire::E_OPEN, wire::OE_PEER_AS), Some(n), false, false);
            return;
        }
        let rid = self.rid.unwrap_or_default();
        if o.id == rid {
            self.drop_conn(os, a, ci, DownWhy::Sent(wire::E_OPEN, wire::OE_ID), Some(Notification::new(wire::E_OPEN, wire::OE_ID)), false, false);
            return;
        }
        // a collision: the connection opened by the higher identifier stays
        let Some(sock) = self.peers.get(&a).and_then(|p| p.conns.get(ci)).map(|c| c.sock) else {
            return;
        };
        let other = self.peers.get(&a).and_then(|p| p.conns.iter().enumerate().find(|(i, c)| *i != ci && c.st >= CState::OpenConfirm).map(|(i, c)| (i, c.st)));
        if let Some((oi, ost)) = other {
            let this_out = self.peers.get(&a).and_then(|p| p.conns.get(ci)).is_some_and(|c| c.out);
            let keep_out = rid > o.id;
            let cease = || Notification::new(wire::E_CEASE, wire::CE_COLLISION);
            note(os, BNote::Collision { peer: a, keep_ours: keep_out });
            if ost == CState::Established || this_out != keep_out {
                self.drop_conn(os, a, ci, DownWhy::Sent(wire::E_CEASE, wire::CE_COLLISION), Some(cease()), false, false);
                return;
            }
            self.drop_conn(os, a, oi, DownWhy::Sent(wire::E_CEASE, wire::CE_COLLISION), Some(cease()), false, false);
        }
        // (the other connection may have gone before this one)
        let Some(ci) = self.peers.get(&a).and_then(|p| p.conns.iter().position(|c| c.sock == sock)) else {
            return;
        };
        let hold = self.cfg.hold.min(o.hold);
        let ka = if hold == 0 || self.cfg.keepalive == 0 { 0 } else { self.cfg.keepalive.min(hold / 3).max(1) };
        if let Some(c) = self.peers.get_mut(&a).and_then(|p| p.conns.get_mut(ci)) {
            c.open = Some(o);
            c.hold = hold;
            c.ka = ka;
            c.st = CState::OpenConfirm;
            c.hold_at = if hold > 0 { Some(now + hold as Ns * SEC) } else { None };
            c.ka_at = if ka > 0 { Some(now + ka as Ns * SEC) } else { None };
        }
        note(os, BNote::Negotiated { peer: a, hold, keepalive: ka });
        self.send(os, a, ci, &Message::Keepalive);
    }

    /// OpenConfirm and the neighbour's KEEPALIVE: the session is up
    fn established(&mut self, os: &mut OsApi, a: Ipv4, ci: usize) {
        let now = os.now();
        let Some(p) = self.peers.get_mut(&a) else {
            return;
        };
        let Some(c) = p.conns.get_mut(ci) else {
            return;
        };
        c.st = CState::Established;
        let (id, asn, as4, refresh) = c.open.as_ref().map(|o| (o.id, o.peer_as(), o.as4().is_some(), o.has_cap(wire::CAP_ROUTE_REFRESH))).unwrap_or_default();
        p.peer_id = id;
        p.peer_as = asn;
        p.as4 = as4;
        p.refresh = refresh;
        p.hold = c.hold;
        p.keepalive = c.ka;
        p.local = c.local;
        p.lport = c.lport;
        p.rport = c.rport;
        p.since = Some(now);
        p.n.established += 1;
        p.adj_in.clear();
        p.adj_out.clear();
        p.mrai_at = None;
        p.waiting = false;
        p.idle = None;
        p.fail = None;
        p.retry_at = None;
        p.idle_until = None;
    }

    /// an UPDATE (6.3, 9): withdrawals, then the new routes with their checks
    fn update_in(&mut self, os: &mut OsApi, a: Ipv4, ci: usize, u: Update) {
        let now = os.now();
        self.n.updates_rcvd += 1;
        note(os, BNote::Update { peer: a, out: false, adv: u.nlri.len() as u16, wd: u.withdrawn.len() as u16 });
        let (ebgp, peer_as, peer_id, as4) = match self.peers.get_mut(&a) {
            Some(p) => {
                p.n.updates_in += 1;
                (self.cfg.nbrs.get(&a).is_some_and(|n| n.remote_as != self.cfg.asn), p.peer_as, p.peer_id, p.as4)
            }
            None => return,
        };
        for w in &u.withdrawn {
            if let Some(p) = self.peers.get_mut(&a) {
                p.adj_in.remove(w);
            }
        }
        if u.nlri.is_empty() {
            return;
        }
        let mut attrs = Attrs::of(&u, as4);
        // the first AS of an external neighbour's path is its own (IOS: enforce-first-as)
        if ebgp && attrs.first_as(0) != Some(peer_as) {
            self.drop_conn(os, a, ci, DownWhy::Sent(wire::E_UPDATE, wire::UE_AS_PATH), Some(Notification::new(wire::E_UPDATE, wire::UE_AS_PATH)), false, false);
            return;
        }
        if ebgp {
            attrs.local_pref = None;
        }
        let looped = attrs.as_path.iter().any(|s| s.asns.contains(&self.cfg.asn));
        let nh = attrs.next_hop;
        let why = if looped {
            Some(DropWhy::Loop(self.cfg.asn))
        } else if os.is_mine(nh) {
            Some(DropWhy::OwnNextHop(nh))
        } else if !unicast(nh) {
            Some(DropWhy::BadNextHop(nh))
        } else {
            None
        };
        for p in &u.nlri {
            match why {
                Some(w) => {
                    self.n.dropped_routes += 1;
                    note(os, BNote::Dropped { peer: a, prefix: *p, why: w });
                    if let Some(x) = self.peers.get_mut(&a) {
                        x.adj_in.remove(p);
                    }
                }
                None => {
                    let path = Path { attrs: attrs.clone(), from: Src::Peer(a), peer_id, ebgp, since: now };
                    if let Some(x) = self.peers.get_mut(&a) {
                        let same = x.adj_in.get(p).is_some_and(|o| o.attrs == path.attrs);
                        if !same {
                            x.adj_in.insert(*p, path);
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- starting, timers

impl Bgp {
    /// opens a connection to a neighbour (Connect), or waits when that is not possible (Active)
    fn connect(&mut self, os: &mut OsApi, a: Ipv4) {
        let now = os.now();
        let src = self.source(os, a).flatten();
        let opts = self.opts_for(a);
        let r = match src {
            Some(s) => os.tcp_connect_src(IpAddr::V4(s), a, wire::PORT, opts),
            None => os.tcp_connect_msg(a, wire::PORT, opts),
        };
        self.n.connects += 1;
        let Some(p) = self.peers.get_mut(&a) else {
            return;
        };
        p.n.attempts += 1;
        p.retry_at = Some(now + CONNECT_RETRY);
        p.idle = None;
        match r {
            Ok(sock) => {
                p.conns.push(Conn::new(sock, true, CState::Connecting));
                p.base = State::Active;
                p.no_route = false;
                note(os, BNote::Connect { peer: a, src });
            }
            Err(_) => {
                p.base = State::Active;
                p.no_route = true;
                note(os, BNote::NoRoute { peer: a });
            }
        }
    }

    /// may the neighbour leave Idle now?
    fn may_start(&self, os: &OsApi, a: Ipv4) -> Result<(), IdleWhy> {
        let Some(n) = self.cfg.nbrs.get(&a) else {
            return Err(IdleWhy::Admin);
        };
        if n.shutdown {
            return Err(IdleWhy::Admin);
        }
        if self.rid.is_none() {
            return Err(IdleWhy::NoRouterId);
        }
        if n.remote_as != self.cfg.asn && n.multihop.is_none() && !Self::connected(os, a) {
            return Err(IdleWhy::NotConnected);
        }
        if self.source(os, a).is_none() {
            return Err(IdleWhy::NoSource);
        }
        if self.peers.get(&a).and_then(|p| p.idle_until).is_some_and(|t| t > os.now()) {
            return Err(IdleWhy::Hold);
        }
        Ok(())
    }

    /// the neighbours follow the configuration; Idle ones start, Active ones that found no
    /// route try as soon as there is one; directly connected external sessions end when their
    /// network is gone (fast external fallover)
    fn sync_peers(&mut self, os: &mut OsApi) {
        // neighbours that are no longer configured
        let gone: Vec<Ipv4> = self.peers.keys().filter(|a| !self.cfg.nbrs.contains_key(a)).copied().collect();
        for a in gone {
            self.end_all(os, a, wire::CE_DECONFIGURED);
            self.peers.remove(&a);
        }
        let new: Vec<Ipv4> = self.cfg.nbrs.keys().filter(|a| !self.peers.contains_key(a)).copied().collect();
        for a in new {
            self.peers.insert(a, Peer::new(a));
        }
        let addrs: Vec<Ipv4> = self.peers.keys().copied().collect();
        for a in addrs {
            let Some(p) = self.peers.get(&a) else {
                continue;
            };
            let st = p.state();
            let shut = self.cfg.nbrs.get(&a).is_some_and(|n| n.shutdown);
            if shut && st != State::Idle {
                self.end_all(os, a, wire::CE_SHUTDOWN);
            }
            // fast external fallover
            let single = self.cfg.nbrs.get(&a).is_some_and(|n| n.remote_as != self.cfg.asn && n.multihop.is_none());
            if single && st == State::Established && !Self::connected(os, a) {
                if let Some(ci) = self.peers.get(&a).and_then(|p| p.conns.iter().position(|c| c.st == CState::Established)) {
                    self.drop_conn(os, a, ci, DownWhy::IfDown, None, true, false);
                    self.track(os);
                }
            }
            let Some(p) = self.peers.get(&a) else {
                continue;
            };
            let (st, no_route) = (p.state(), p.no_route);
            if st == State::Idle {
                match self.may_start(os, a) {
                    Ok(()) => self.connect(os, a),
                    Err(w) => {
                        if let Some(p) = self.peers.get_mut(&a)
                            && p.idle != Some(w)
                        {
                            p.idle = Some(w);
                            note(os, BNote::Idle { peer: a, why: w });
                        }
                    }
                }
            } else if st == State::Active && no_route && os.source_for(a).is_some() && self.may_start(os, a).is_ok() {
                self.connect(os, a);
            }
        }
    }

    /// ends every connection of a neighbour with Cease and a subcode
    fn end_all(&mut self, os: &mut OsApi, a: Ipv4, sub: u8) {
        loop {
            let Some(p) = self.peers.get(&a) else {
                return;
            };
            let Some(ci) = p.conns.iter().position(|c| c.st >= CState::OpenSent).or(if p.conns.is_empty() { None } else { Some(0) }) else {
                break;
            };
            let st = p.conns[ci].st;
            if st == CState::Connecting {
                self.drop_conn(os, a, ci, DownWhy::Sent(wire::E_CEASE, sub), None, true, false);
            } else {
                self.drop_conn(os, a, ci, DownWhy::Sent(wire::E_CEASE, sub), Some(Notification::new(wire::E_CEASE, sub)), false, false);
            }
        }
        if let Some(p) = self.peers.get_mut(&a) {
            p.base = State::Idle;
            p.retry_at = None;
            p.idle_until = None;
            p.no_route = false;
        }
    }

    /// what the configuration asked for
    fn requests(&mut self, os: &mut OsApi) {
        let reqs = core::mem::take(&mut self.reqs);
        for r in reqs {
            let pick = |b: &Bgp, x: Option<Ipv4>| -> Vec<Ipv4> {
                match x {
                    Some(a) => alloc::vec![a],
                    None => b.peers.keys().copied().collect(),
                }
            };
            match r {
                Req::Reset(x, sub) => {
                    // (a neighbour that waits Idle starts again at once too)
                    for a in pick(self, x) {
                        self.end_all(os, a, sub);
                    }
                }
                Req::SoftIn(x) => {
                    for a in pick(self, x) {
                        if let Some(ci) = self.peers.get(&a).filter(|p| p.refresh).and_then(|p| p.conns.iter().position(|c| c.st == CState::Established)) {
                            self.send(os, a, ci, &Message::RouteRefresh { afi: wire::AFI_IPV4, safi: wire::SAFI_UNICAST });
                        }
                    }
                }
                Req::SoftOut(x) => {
                    for a in pick(self, x) {
                        if let Some(p) = self.peers.get_mut(&a) {
                            p.adj_out.clear();
                            p.mrai_at = None;
                        }
                    }
                }
                Req::NewId => {
                    let id = self.pick_rid(os);
                    if id != self.rid && self.cfg.router_id.is_some() {
                        let all: Vec<Ipv4> = self.peers.keys().copied().collect();
                        for a in all {
                            self.end_all(os, a, wire::CE_CONFIG);
                        }
                        self.rid = id;
                        if let Some(r) = id {
                            note(os, BNote::RouterId { id: r });
                        }
                    }
                }
            }
        }
    }

    /// the earliest deadline of the process
    fn next_deadline(&self) -> Option<Ns> {
        let mut t: Option<Ns> = None;
        let mut at = |x: Option<Ns>| {
            if let Some(x) = x
                && t.is_none_or(|y| x < y)
            {
                t = Some(x);
            }
        };
        for p in self.peers.values() {
            for c in &p.conns {
                at(c.hold_at);
                at(c.ka_at);
            }
            at(p.retry_at);
            at(p.idle_until);
            at(p.mrai_at());
        }
        t
    }

    fn arm(&mut self, os: &mut OsApi) {
        let next = self.next_deadline();
        if next != self.armed {
            self.armed = next;
            match next {
                Some(t) => os.timer(T_TICK, t),
                None => os.cancel(T_TICK),
            }
        }
    }

    /// whatever is due now, neighbour by neighbour: hold timers, keepalives, ConnectRetry, the
    /// end of a pause, announcements that waited
    fn tick(&mut self, os: &mut OsApi) {
        let now = os.now();
        let addrs: Vec<Ipv4> = self.peers.keys().copied().collect();
        for a in addrs {
            let mut ci = 0;
            loop {
                let Some(c) = self.peers.get(&a).and_then(|p| p.conns.get(ci)) else {
                    break;
                };
                if c.hold_at.is_some_and(|t| t <= now) {
                    note(os, BNote::HoldExpired { peer: a });
                    self.drop_conn(os, a, ci, DownWhy::Sent(wire::E_HOLD, 0), Some(Notification::new(wire::E_HOLD, 0)), false, false);
                    continue;
                }
                if c.ka_at.is_some_and(|t| t <= now) {
                    self.send(os, a, ci, &Message::Keepalive);
                }
                ci += 1;
            }
            let Some(p) = self.peers.get_mut(&a) else {
                continue;
            };
            if p.idle_until.is_some_and(|t| t <= now) {
                p.idle_until = None;
            }
            if p.mrai_at.is_some_and(|t| t <= now) {
                p.mrai_at = None;
            }
            if p.retry_at.is_some_and(|t| t <= now) {
                p.retry_at = None;
                // ConnectRetry: an attempt that hangs is given up, a new one starts
                let hung: Vec<u32> = p.conns.iter().filter(|c| c.st == CState::Connecting).map(|c| c.sock).collect();
                p.conns.retain(|c| c.st != CState::Connecting);
                for s in hung {
                    os.tcp_abort(s);
                }
                let st = self.peers.get(&a).map(|p| p.state());
                if matches!(st, Some(State::Active | State::Connect)) && self.may_start(os, a).is_ok() {
                    self.connect(os, a);
                }
            }
        }
        self.track(os);
    }
}

// ---------------------------------------------------------------- the decision process

impl Bgp {
    /// the paths this router originates now
    fn local_paths(&self, os: &OsApi, rib: &[Route]) -> BTreeMap<Prefix4, Path> {
        let now = os.now();
        let rid = self.rid.unwrap_or_default();
        let mut out: BTreeMap<Prefix4, Path> = BTreeMap::new();
        let lp = |how: How, origin: u8, med: Option<u32>| Path { attrs: Attrs::local(origin, med), from: Src::Local(how), peer_id: rid, ebgp: false, since: now };
        for n in &self.cfg.networks {
            let p = n.trunc();
            let best = rib.iter().filter(|r| r.prefix == p).min_by_key(|r| (r.ad, r.metric));
            if let Some(r) = best {
                out.insert(p, lp(How::Network, wire::ORIGIN_IGP, Some(r.metric)));
            }
        }
        for rd in &self.cfg.redistribute {
            let take = |p: Prefix4, metric: u32, out: &mut BTreeMap<Prefix4, Path>| {
                let p = p.trunc();
                if !out.contains_key(&p) {
                    out.insert(p, lp(rd.what, wire::ORIGIN_INCOMPLETE, Some(rd.metric.unwrap_or(metric))));
                }
            };
            match rd.what {
                How::Connected => {
                    for r in rib.iter().filter(|r| r.src == RouteSrc::Connected) {
                        take(r.prefix, 0, &mut out);
                    }
                }
                How::Static | How::Rip | How::Ospf => {
                    let src = match rd.what {
                        How::Static => RouteSrc::Static,
                        How::Rip => RouteSrc::Rip,
                        _ => RouteSrc::Ospf,
                    };
                    let mut seen: Vec<Prefix4> = Vec::new();
                    for r in rib.iter().filter(|r| r.src == src) {
                        let win = rib.iter().filter(|x| x.prefix == r.prefix).map(|x| x.ad).min() == Some(r.ad);
                        if win && !seen.contains(&r.prefix) {
                            seen.push(r.prefix);
                            take(r.prefix, r.metric, &mut out);
                        }
                    }
                }
                How::Network => {}
            }
        }
        // a path that was here before keeps its time
        for (p, x) in out.iter_mut() {
            if let Some(old) = self.local.get(p)
                && old.attrs == x.attrs
                && old.from == x.from
            {
                x.since = old.since;
            }
        }
        out
    }

    /// the step-by-step choice among the usable candidates (9.1.2.2 and the IOS steps): the
    /// best one and the step that left it alone
    fn choose(&self, cands: &[Cand]) -> Option<(usize, Step)> {
        let mut set: Vec<usize> = (0..cands.len()).filter(|&i| cands[i].ok).collect();
        if set.is_empty() {
            return None;
        }
        if set.len() == 1 {
            return Some((set[0], Step::Only));
        }
        let own = self.cfg.asn;
        // keeps the candidates with the highest key
        fn keep_max(set: &mut Vec<usize>, key: impl Fn(usize) -> u64) {
            let m = set.iter().map(|&i| key(i)).max().unwrap_or(0);
            set.retain(|&i| key(i) == m);
        }
        let steps: [Step; 10] = [Step::Weight, Step::LocalPref, Step::Local, Step::AsPath, Step::Origin, Step::Med, Step::External, Step::IgpMetric, Step::RouterId, Step::Address];
        for step in steps {
            match step {
                Step::Weight => keep_max(&mut set, |i| cands[i].weight as u64),
                Step::LocalPref => keep_max(&mut set, |i| cands[i].lp as u64),
                Step::Local => keep_max(&mut set, |i| matches!(cands[i].path.from, Src::Local(_)) as u64),
                Step::AsPath => keep_max(&mut set, |i| u64::MAX - wire::path_len(&cands[i].path.attrs.as_path) as u64),
                Step::Origin => keep_max(&mut set, |i| u64::MAX - cands[i].path.attrs.origin as u64),
                Step::Med => {
                    // a path goes when one from the same neighbouring AS has a lower MED
                    let med = |i: usize| cands[i].path.attrs.med.unwrap_or(0);
                    let nas = |i: usize| cands[i].path.attrs.first_as(own);
                    let all = set.clone();
                    set.retain(|&i| !all.iter().any(|&j| j != i && nas(i).is_some() && nas(j) == nas(i) && med(j) < med(i)));
                }
                Step::External => keep_max(&mut set, |i| cands[i].path.ebgp as u64),
                Step::IgpMetric => keep_max(&mut set, |i| u64::MAX - cands[i].igp as u64),
                Step::RouterId => keep_max(&mut set, |i| u64::MAX - cands[i].path.peer_id.to_u32() as u64),
                Step::Address => keep_max(&mut set, |i| {
                    let a = match cands[i].path.from {
                        Src::Peer(a) => a.to_u32(),
                        Src::Local(_) => 0,
                    };
                    u64::MAX - a as u64
                }),
                Step::Only => {}
            }
            if set.len() == 1 {
                return Some((set[0], step));
            }
        }
        Some((set[0], Step::Address))
    }

    /// the Loc-RIB anew: local paths, the Adj-RIBs-In of the established sessions, the best
    /// path of every prefix; notes for what changed, the RIB
    fn decide(&mut self, os: &mut OsApi) {
        let now = os.now();
        let rib = Self::igp_rib(os);
        let local = self.local_paths(os, &rib);
        for (p, x) in &local {
            if self.local.get(p).is_none_or(|o| o.from != x.from) {
                let how = match x.from {
                    Src::Local(h) => h,
                    Src::Peer(_) => How::Network,
                };
                note(os, BNote::Local { prefix: *p, how });
            }
        }
        for p in self.local.keys() {
            if !local.contains_key(p) {
                note(os, BNote::LocalGone { prefix: *p });
            }
        }
        self.local = local;
        let deflp = self.cfg.local_pref;
        let mut all: BTreeMap<Prefix4, Vec<Cand>> = BTreeMap::new();
        for (p, x) in &self.local {
            all.entry(*p).or_default().push(Cand { path: x.clone(), ok: true, igp: 0, iface: None, weight: WEIGHT_LOCAL, lp: deflp });
        }
        for (a, peer) in &self.peers {
            if peer.session().is_none() {
                continue;
            }
            let w = self.cfg.nbrs.get(a).and_then(|n| n.weight).unwrap_or(0);
            for (p, x) in &peer.adj_in {
                let r = rib_best(&rib, x.attrs.next_hop).filter(|r| r.src != RouteSrc::Local);
                let (ok, igp, iface) = match r {
                    Some(r) => (r.iface.is_some(), r.metric, r.iface),
                    None => (false, 0, None),
                };
                let lp = x.attrs.local_pref.unwrap_or(deflp);
                all.entry(*p).or_default().push(Cand { path: x.clone(), ok, igp, iface, weight: w, lp });
            }
        }
        let mut rib_new: BTreeMap<Prefix4, Dest> = BTreeMap::new();
        let mut notes: Vec<BNote> = Vec::new();
        let keys: Vec<Prefix4> = all.keys().chain(self.rib.keys()).copied().collect::<alloc::collections::BTreeSet<_>>().into_iter().collect();
        for p in keys {
            let cands = all.remove(&p).unwrap_or_default();
            let pick = self.choose(&cands);
            let old = self.rib.get(&p);
            let old_best = old.and_then(|d| d.best()).map(|c| (c.path.from, c.path.attrs.clone(), c.iface));
            let new_best = pick.map(|(i, _)| (cands[i].path.from, cands[i].path.attrs.clone(), cands[i].iface));
            let changed = old_best != new_best;
            match pick {
                Some((i, step)) => {
                    if changed {
                        self.ver += 1;
                        let c = &cands[i];
                        let from = match c.path.from {
                            Src::Peer(a) => Some(a),
                            Src::Local(_) => None,
                        };
                        notes.push(BNote::Best { prefix: p, via: c.path.attrs.next_hop, from, step });
                    }
                    let (since, ver) = match old {
                        Some(d) if !changed => (d.since, d.ver),
                        _ => (now, self.ver),
                    };
                    rib_new.insert(p, Dest { cands, best: Some(i), step, since, ver });
                }
                None => {
                    if old_best.is_some() {
                        self.ver += 1;
                        notes.push(BNote::Lost { prefix: p });
                    }
                    if !cands.is_empty() {
                        rib_new.insert(p, Dest { cands, best: None, step: Step::Only, since: now, ver: self.ver });
                    }
                }
            }
        }
        self.rib = rib_new;
        for n in notes {
            note(os, n);
        }
        self.install_rib(os);
    }

    /// the best learned paths into the RIB
    fn install_rib(&self, os: &mut OsApi) {
        let mut v: Vec<Route> = Vec::new();
        for (p, d) in &self.rib {
            let Some(c) = d.best() else {
                continue;
            };
            let (Src::Peer(_), Some(i)) = (c.path.from, c.iface) else {
                continue;
            };
            v.push(Route { prefix: *p, via: Some(c.path.attrs.next_hop), iface: Some(i), src: RouteSrc::Bgp, ad: if c.path.ebgp { AD_EBGP } else { AD_IBGP }, metric: c.path.attrs.med.unwrap_or(0) });
        }
        let ni = os.ni;
        if let Some(st) = os.eng.stack_mut(ni) {
            st.set_dynamic(RouteSrc::Bgp, v);
        }
    }
}

// ---------------------------------------------------------------- advertisement

impl Bgp {
    /// what a neighbour gets for the best path of a prefix (None: nothing)
    fn export(&self, c: &Cand, to: Ipv4, local: Ipv4) -> Option<Attrs> {
        let ebgp_to = self.cfg.ebgp(to);
        let n = self.cfg.nbrs.get(&to)?;
        if let Src::Peer(f) = c.path.from {
            if f == to {
                return None;
            }
            // no route reflection: what came from inside does not go back inside
            if !c.path.ebgp && !ebgp_to {
                return None;
            }
        }
        let mut x = c.path.attrs.clone();
        let local_path = matches!(c.path.from, Src::Local(_));
        if ebgp_to {
            let own = self.cfg.asn;
            // the MED only for a path that started in the own AS
            if !x.as_path.is_empty() {
                x.med = None;
            }
            match x.as_path.first_mut() {
                Some(s) if !s.set && s.asns.len() < 255 => s.asns.insert(0, own),
                _ => x.as_path.insert(0, Segment::seq(alloc::vec![own])),
            }
            x.next_hop = local;
            x.local_pref = None;
        } else {
            x.local_pref = Some(c.lp);
            if local_path || n.next_hop_self {
                x.next_hop = local;
            }
        }
        Some(x)
    }

    /// every established neighbour gets what changed for it: withdrawals at once, new paths when
    /// the minimum interval allows
    fn advertise(&mut self, os: &mut OsApi) {
        let now = os.now();
        let addrs: Vec<Ipv4> = self.peers.keys().copied().collect();
        for a in addrs {
            let Some(p) = self.peers.get(&a) else {
                continue;
            };
            let Some(ci) = p.conns.iter().position(|c| c.st == CState::Established) else {
                continue;
            };
            let local = p.conns[ci].local;
            let mut want: BTreeMap<Prefix4, Attrs> = BTreeMap::new();
            for (pf, d) in &self.rib {
                if let Some(c) = d.best()
                    && let Some(x) = self.export(c, a, local)
                {
                    want.insert(*pf, x);
                }
            }
            let gone: Vec<Prefix4> = p.adj_out.keys().filter(|k| !want.contains_key(k)).copied().collect();
            let mut news: Vec<(Prefix4, Attrs)> = want.into_iter().filter(|(k, x)| p.adj_out.get(k) != Some(x)).collect();
            let hold = p.mrai_at.filter(|t| *t > now);
            let mrai = if self.cfg.ebgp(a) { MRAI_EBGP } else { MRAI_IBGP };
            // withdrawals
            let mut w = gone.clone();
            while !w.is_empty() {
                let mut take: Vec<Prefix4> = Vec::new();
                let mut len = wire::UPDATE_MIN;
                while let Some(x) = w.first() {
                    let l = wire::prefix_len(x);
                    if len + l > wire::MAX_LEN {
                        break;
                    }
                    len += l;
                    take.push(w.remove(0));
                }
                self.send(os, a, ci, &Message::Update(Update { withdrawn: take, attrs: Vec::new(), nlri: Vec::new() }));
            }
            if let Some(p) = self.peers.get_mut(&a) {
                for g in &gone {
                    p.adj_out.remove(g);
                }
            }
            if news.is_empty() {
                let ver = self.ver;
                if let Some(p) = self.peers.get_mut(&a) {
                    p.waiting = false;
                    p.tbl_ver = ver;
                }
                continue;
            }
            if let Some(t) = hold {
                let first = self.peers.get(&a).is_some_and(|p| !p.waiting);
                if let Some(p) = self.peers.get_mut(&a) {
                    p.waiting = true;
                }
                if first {
                    note(os, BNote::Mrai { peer: a, wait: t - now });
                }
                continue;
            }
            // announcements: one UPDATE per set of attributes, as many prefixes as fit
            let as4 = self.peers.get(&a).and_then(|p| p.conns.get(ci)).is_some_and(|c| c.as4());
            let mut groups: Vec<(Attrs, Vec<Prefix4>)> = Vec::new();
            for (k, x) in news.drain(..) {
                match groups.iter_mut().find(|g| g.0 == x) {
                    Some(g) => g.1.push(k),
                    None => groups.push((x, alloc::vec![k])),
                }
            }
            for (x, ps) in groups {
                let attrs = x.wire(as4);
                let base = Update { withdrawn: Vec::new(), attrs: attrs.clone(), nlri: Vec::new() }.wire_len(as4);
                let mut rest = ps.clone();
                while !rest.is_empty() {
                    let mut take: Vec<Prefix4> = Vec::new();
                    let mut len = base;
                    while let Some(q) = rest.first() {
                        let l = wire::prefix_len(q);
                        if len + l > wire::MAX_LEN && !take.is_empty() {
                            break;
                        }
                        len += l;
                        take.push(rest.remove(0));
                    }
                    self.send(os, a, ci, &Message::Update(Update { withdrawn: Vec::new(), attrs: attrs.clone(), nlri: take }));
                }
                if let Some(p) = self.peers.get_mut(&a) {
                    for q in ps {
                        p.adj_out.insert(q, x.clone());
                    }
                }
            }
            let ver = self.ver;
            if let Some(p) = self.peers.get_mut(&a) {
                p.waiting = false;
                p.tbl_ver = ver;
                p.mrai_at = if mrai > 0 { Some(now + mrai) } else { None };
            }
        }
    }

    /// after every event: what the configuration asks, the neighbours, the decision, the
    /// updates, the notes, the deadline timer
    fn after(&mut self, os: &mut OsApi) {
        if !self.running {
            return;
        }
        if self.rid.is_none() && self.cfg.on {
            self.rid = self.pick_rid(os);
            if let Some(id) = self.rid {
                note(os, BNote::RouterId { id });
            }
        }
        self.requests(os);
        self.sync_peers(os);
        self.track(os);
        self.decide(os);
        self.advertise(os);
        self.track(os);
        self.arm(os);
    }

    /// everything learned goes (power off, "no router bgp"); the configuration stays
    fn forget(&mut self, os: &mut OsApi) {
        let addrs: Vec<Ipv4> = self.peers.keys().copied().collect();
        for a in addrs {
            if os.powered() {
                self.end_all(os, a, wire::CE_DECONFIGURED);
            }
        }
        self.peers.clear();
        self.rib.clear();
        self.local.clear();
        self.armed = None;
        self.rid = None;
        if let Some(l) = self.listener.take() {
            os.tcp_close(l);
        }
    }
}

impl Service for Bgp {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn start(&mut self, os: &mut OsApi) {
        self.running = true;
        self.started = os.now();
        self.peers.clear();
        self.rib.clear();
        self.local.clear();
        self.reqs.clear();
        self.armed = None;
        self.rid = None;
        self.listener = os.tcp_listen_on_msg(ANY4, wire::PORT, 0, SockOpts { nodelay: true, ..SockOpts::default() }).ok();
        self.after(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::TcpAccept { listener, .. } if Some(listener) == self.listener => self.accept(os, listener),
            SvcEvent::TcpConnected { sock, .. } => {
                if let Some((a, ci)) = self.conn_of(sock) {
                    self.conn_up(os, a, ci);
                } else {
                    os.tcp_abort(sock);
                }
            }
            SvcEvent::TcpData { sock, .. } => self.data(os, sock),
            SvcEvent::TcpWritable { sock } => self.flush(os, sock),
            SvcEvent::TcpEof { sock, .. } => {
                if let Some((a, ci)) = self.conn_of(sock) {
                    self.drop_conn(os, a, ci, DownWhy::Tcp(TcpEnd::Closed), None, false, false);
                }
            }
            SvcEvent::TcpClosed { sock, end } => {
                if let Some((a, ci)) = self.conn_of(sock) {
                    let st = self.peers.get(&a).and_then(|p| p.conns.get(ci)).map(|c| c.st);
                    if st == Some(CState::Connecting) {
                        if let Some(p) = self.peers.get_mut(&a) {
                            p.fail = Some(end);
                        }
                        note(os, BNote::ConnFailed { peer: a, end });
                    }
                    self.drop_conn(os, a, ci, DownWhy::Tcp(end), None, false, true);
                }
            }
            SvcEvent::Timer(T_TICK) => {
                self.armed = None;
                self.tick(os);
            }
            _ => {}
        }
        self.track(os);
        self.after(os);
    }
    fn stop(&mut self, os: &mut OsApi) {
        self.forget(os);
        self.running = false;
        let ni = os.ni;
        if let Some(st) = os.eng.stack_mut(ni) {
            st.set_dynamic(RouteSrc::Bgp, Vec::new());
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        let (all, up) = self.neighbours();
        let n = &self.n;
        alloc::vec![
            Metric::new("neighbours", all as i64),
            Metric::new("established", up as i64),
            Metric::new("prefixes", self.rib.values().filter(|d| d.best.is_some()).count() as i64),
            Metric::new("paths", self.path_count() as i64),
            Metric::new("opens_sent", n.opens_sent as i64),
            Metric::new("opens_rcvd", n.opens_rcvd as i64),
            Metric::new("updates_sent", n.updates_sent as i64),
            Metric::new("updates_rcvd", n.updates_rcvd as i64),
            Metric::new("keepalives_sent", n.keepalives_sent as i64),
            Metric::new("keepalives_rcvd", n.keepalives_rcvd as i64),
            Metric::new("notifications_sent", n.notifications_sent as i64),
            Metric::new("notifications_rcvd", n.notifications_rcvd as i64),
            Metric::new("connects", n.connects as i64),
            Metric::new("refused", n.refused as i64),
            Metric::new("flaps", n.flaps as i64),
            Metric::new("dropped_routes", n.dropped_routes as i64),
        ]
    }
    fn metric_kind(&self, name: &str) -> MetricKind {
        match name {
            "neighbours" | "established" | "prefixes" | "paths" => MetricKind::Gauge,
            _ => MetricKind::Counter,
        }
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        if !self.cfg.on {
            return msg!("BGP ist nicht eingerichtet (kein router bgp).");
        }
        match self.rid {
            Some(id) if self.running => {
                let (all, up) = self.neighbours();
                let nets = self.rib.values().filter(|d| d.best.is_some()).count();
                msg!("BGP in AS {}, Router-ID {}: Nachbarn: {}, davon Established: {}; Netze: {}.", self.cfg.asn, id, all, up, nets)
            }
            None if self.running => msg!("BGP in AS {}: wartet auf eine Adresse für die Router-ID.", self.cfg.asn),
            _ => msg!("BGP in AS {}: wartet auf das Gerät.", self.cfg.asn),
        }
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

// ---------------------------------------------------------------- the engine side

impl Engine {
    /// the BGP service of a device (None before the first BGP command)
    pub fn bgp(&self, dev: u32) -> Option<&Bgp> {
        self.bgp_of(self.node_index(dev)?)
    }
    pub(crate) fn bgp_of(&self, ni: usize) -> Option<&Bgp> {
        self.os(ni)?.svcs.iter().find(|s| s.kind == KIND)?.svc.as_ref()?.as_any().downcast_ref::<Bgp>()
    }
    fn bgp_mut(&mut self, ni: usize) -> Option<&mut Bgp> {
        self.os_mut(ni)?.svcs.iter_mut().find(|s| s.kind == KIND)?.svc.as_mut()?.as_any_mut().downcast_mut::<Bgp>()
    }
    /// (service id, running) of the BGP service of node `ni`
    fn bgp_slot(&self, ni: usize) -> Option<(u32, bool)> {
        self.os(ni)?.svcs.iter().find(|s| s.kind == KIND).map(|s| (s.id, s.running))
    }
    /// "router bgp" is configured on node `ni`
    pub(crate) fn bgp_on(&self, ni: usize) -> bool {
        self.bgp_of(ni).is_some_and(|b| b.cfg.on)
    }
    /// something about node `ni`'s interfaces or routes may have changed: its BGP looks again
    /// at the same virtual time (a timer of its own, so it never runs inside another call)
    pub(crate) fn bgp_kick(&mut self, ni: usize) {
        if let Some((id, true)) = self.bgp_slot(ni) {
            let now = self.now;
            self.set_timer(ni, SLOT_SVC + id * SVC_TIMERS + T_LOOK, now);
        }
    }
    pub(crate) fn bgp_kick_dev(&mut self, dev: u32) {
        if let Some(ni) = self.node_index(dev) {
            self.bgp_kick(ni);
        }
    }

    /// Action::Bgp: a change of the configuration; the service comes with the first one
    /// (stopped until "router bgp")
    pub(crate) fn bgp_apply(&mut self, dev: u32, set: BgpSet) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let routes = matches!(self.nodes.get(ni), Some(Some(n)) if n.model.has(F_ROUTER));
        if !routes {
            return Err(ActErr::Refused(msg!("{} routet nicht: hier gibt es kein BGP.", self.host(ni))));
        }
        if self.os(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        if self.bgp_slot(ni).is_none() {
            // undoing what was never set needs no service
            if matches!(set, BgpSet::Process { on: false, .. } | BgpSet::Clear { .. }) {
                return Ok(());
            }
            self.add_service(dev, KIND, Box::new(Bgp::new()), false)?;
        }
        let (id, running) = self.bgp_slot(ni).ok_or(ActErr::NoIp(dev))?;
        let set = match set {
            BgpSet::Process { asn, on } => {
                let cur = self.bgp_of(ni).map(|b| (b.cfg.on, b.cfg.asn)).unwrap_or((false, 0));
                if on && cur.0 && cur.1 != asn {
                    return Err(ActErr::Refused(msg!("Hier läuft schon BGP in AS {}: ein Gerät hat nur einen Prozess.", cur.1)));
                }
                if !on && cur.0 && cur.1 != asn {
                    return Err(ActErr::Refused(msg!("BGP in AS {} gibt es hier nicht.", asn)));
                }
                if asn == 0 {
                    return Err(ActErr::Refused(msg!("AS 0 gibt es nicht (1 bis 4294967295).")));
                }
                if let Some(b) = self.bgp_mut(ni) {
                    if on {
                        b.cfg.on = true;
                        b.cfg.asn = asn;
                    } else {
                        b.cfg.reset();
                    }
                }
                if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)) {
                    s.enabled = on;
                }
                if !on {
                    self.svc_stop(ni, id);
                } else if !running {
                    self.os_push(ni, Job::Start(id));
                }
                return Ok(());
            }
            BgpSet::UpdateSource { addr, iface: Some(name) } => {
                let st = self.stack(ni).ok_or(ActErr::NoIp(dev))?;
                let i = st.iface_by_name(&name).ok_or_else(|| ActErr::UnknownIface { dev, iface: name.clone() })?;
                BgpSet::UpdateSource { addr, iface: Some(st.ifaces[i as usize].name.clone()) }
            }
            BgpSet::RemoteAs { addr, asn: Some(n) } => {
                if self.stack(ni).is_some_and(|s| s.is_mine(addr)) {
                    return Err(ActErr::Refused(msg!("{} ist eine eigene Adresse und kein Nachbar.", addr)));
                }
                if n == 0 || addr.is_unspecified() || addr.is_multicast() || addr.is_broadcast() {
                    return Err(ActErr::Refused(msg!("{} in AS {} taugt nicht als Nachbar.", addr, n)));
                }
                BgpSet::RemoteAs { addr, asn: Some(n) }
            }
            other => other,
        };
        let b = self.bgp_mut(ni).ok_or(ActErr::NoIp(dev))?;
        let known = |b: &Bgp, a: Ipv4| -> Result<(), ActErr> {
            if b.cfg.nbrs.contains_key(&a) { Ok(()) } else { Err(ActErr::Refused(msg!("{} ist kein Nachbar: zuerst neighbor {} remote-as.", a, a))) }
        };
        match set {
            BgpSet::RouterId(r) => {
                if r.is_some_and(|a| !unicast(a)) {
                    return Err(ActErr::Refused(msg!("{} taugt nicht als Router-ID.", r.unwrap_or_default())));
                }
                if b.cfg.router_id != r {
                    b.cfg.router_id = r;
                    b.reqs.push(Req::NewId);
                }
            }
            BgpSet::LogChanges(on) => b.cfg.log_changes = on,
            BgpSet::Timers(t) => {
                let (k, h) = t.unwrap_or((KEEPALIVE, HOLD));
                if h != 0 && h < 3 {
                    return Err(ActErr::Refused(msg!("Eine Haltezeit von {} s geht nicht (0 oder 3 bis 65535).", h)));
                }
                b.cfg.keepalive = k;
                b.cfg.hold = h;
            }
            BgpSet::LocalPref(l) => b.cfg.local_pref = l.unwrap_or(LOCAL_PREF),
            BgpSet::Network { net, add } => {
                let net = net.trunc();
                b.cfg.networks.retain(|n| *n != net);
                if add {
                    b.cfg.networks.push(net);
                    b.cfg.networks.sort_by_key(|n| (n.addr.to_u32(), n.len));
                }
            }
            BgpSet::Redistribute { r, add } => {
                b.cfg.redistribute.retain(|x| x.what != r.what);
                if add {
                    b.cfg.redistribute.push(r);
                    b.cfg.redistribute.sort_by_key(|x| x.what);
                }
            }
            BgpSet::RemoteAs { addr, asn: Some(n) } => match b.cfg.nbrs.get_mut(&addr) {
                Some(x) if x.remote_as != n => {
                    x.remote_as = n;
                    b.reqs.push(Req::Reset(Some(addr), wire::CE_CONFIG));
                }
                Some(_) => {}
                None => {
                    b.cfg.nbrs.insert(addr, NbrCfg::new(n));
                }
            },
            BgpSet::RemoteAs { addr, asn: None } => {
                b.cfg.nbrs.remove(&addr);
            }
            BgpSet::Descr { addr, text } => {
                known(b, addr)?;
                if let Some(x) = b.cfg.nbrs.get_mut(&addr) {
                    x.descr = text.unwrap_or_default();
                }
            }
            BgpSet::UpdateSource { addr, iface } => {
                known(b, addr)?;
                if let Some(x) = b.cfg.nbrs.get_mut(&addr)
                    && x.update_source != iface
                {
                    x.update_source = iface;
                    b.reqs.push(Req::Reset(Some(addr), wire::CE_CONFIG));
                }
            }
            BgpSet::NextHopSelf { addr, on } => {
                known(b, addr)?;
                if let Some(x) = b.cfg.nbrs.get_mut(&addr) {
                    x.next_hop_self = on;
                }
            }
            BgpSet::Multihop { addr, ttl } => {
                known(b, addr)?;
                if let Some(x) = b.cfg.nbrs.get_mut(&addr) {
                    x.multihop = ttl.map(|t| t.max(1));
                }
            }
            BgpSet::Shutdown { addr, on } => {
                known(b, addr)?;
                if let Some(x) = b.cfg.nbrs.get_mut(&addr) {
                    x.shutdown = on;
                }
                if let Some(p) = b.peers.get_mut(&addr) {
                    p.idle = None;
                }
            }
            BgpSet::Weight { addr, weight } => {
                known(b, addr)?;
                if let Some(x) = b.cfg.nbrs.get_mut(&addr) {
                    x.weight = weight;
                }
            }
            BgpSet::Clear { addr, soft } => {
                if let Some(a) = addr {
                    known(b, a)?;
                }
                match soft {
                    Soft::Hard => b.reqs.push(Req::Reset(addr, wire::CE_RESET)),
                    Soft::In => b.reqs.push(Req::SoftIn(addr)),
                    Soft::Out => b.reqs.push(Req::SoftOut(addr)),
                    Soft::Both => {
                        b.reqs.push(Req::SoftIn(addr));
                        b.reqs.push(Req::SoftOut(addr));
                    }
                }
            }
            BgpSet::Process { .. } => {}
        }
        if running {
            self.bgp_kick(ni);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- problems

impl Engine {
    /// the node that has an address (another than `not`)
    fn bgp_owner(&self, a: Ipv4, not: usize) -> Option<usize> {
        self.dev_idx.values().copied().find(|&oi| oi != not && self.stack(oi).is_some_and(|s| s.is_mine(a)))
    }

    /// the address node `ni` starts a session to `a` from (update-source, else the way out)
    fn bgp_from(&self, ni: usize, b: &Bgp, a: Ipv4) -> Option<Ipv4> {
        match b.cfg.nbrs.get(&a).and_then(|n| n.update_source.as_deref()) {
            Some(name) => {
                let st = self.stack(ni)?;
                let i = st.iface_by_name(name)?;
                st.ifaces.get(i as usize)?.primary()
            }
            None => self.source_for(ni, a),
        }
    }

    /// configured neighbours that do not come up, network statements without their route
    pub(crate) fn bgp_problems(&self, out: &mut Vec<crate::problems::Problem>) {
        use crate::problems::{Problem, ProblemKind, Severity};
        let push = |out: &mut Vec<Problem>, kind: ProblemKind, sev: Severity, dev: u32, others: Vec<u32>, m: Msg| {
            out.push(Problem { kind, severity: sev, dev, port: None, link: None, others, text: m.text(), msg: m });
        };
        for (&dev, &ni) in &self.dev_idx {
            let Some(b) = self.bgp_of(ni).filter(|b| b.cfg.on && b.running) else {
                continue;
            };
            let Some(st) = self.stack(ni) else {
                continue;
            };
            let host = self.host(ni);
            let rib = st.rib(&|i| self.iface_up(ni, i));
            for n in &b.cfg.networks {
                if !rib.iter().any(|r| r.prefix == *n && r.src != RouteSrc::Bgp) {
                    push(out, ProblemKind::BgpNetwork, Severity::Warning, dev, Vec::new(), msg!("{}: zu network {} fehlt die Route im Routing-Table: BGP kündigt das Netz nicht an.", host, *n));
                }
            }
            for (a, nc) in &b.cfg.nbrs {
                let up = b.peers.get(a).is_some_and(|p| p.state() == State::Established);
                if nc.shutdown || up {
                    continue;
                }
                let a = *a;
                let ebgp = nc.remote_as != b.cfg.asn;
                let direct = st.ifaces.iter().enumerate().any(|(i, f)| !f.hidden && !f.is_loopback() && self.iface_up(ni, i as u16) && f.on_link(a));
                if ebgp && nc.multihop.is_none() && !direct {
                    push(out, ProblemKind::BgpNeighbor, Severity::Error, dev, Vec::new(), msg!("{}: der externe BGP-Nachbar {} liegt in keinem angeschlossenen Netz; ohne ebgp-multihop beginnt keine Sitzung.", host, a));
                    continue;
                }
                if self.route(ni, a).is_none() {
                    push(out, ProblemKind::BgpNeighbor, Severity::Error, dev, Vec::new(), msg!("{}: keine Route zum BGP-Nachbarn {}.", host, a));
                    continue;
                }
                let Some(oi) = self.bgp_owner(a, ni) else {
                    push(out, ProblemKind::BgpNeighbor, Severity::Error, dev, Vec::new(), msg!("{}: kein Gerät hat die Adresse des BGP-Nachbarn {}.", host, a));
                    continue;
                };
                let odev = self.dev_of(oi);
                let oname = self.host(oi);
                let Some(ob) = self.bgp_of(oi).filter(|x| x.cfg.on) else {
                    push(out, ProblemKind::BgpNeighbor, Severity::Error, dev, alloc::vec![odev], msg!("{}: auf {} ({}) läuft kein BGP.", host, oname, a));
                    continue;
                };
                if ob.cfg.asn != nc.remote_as {
                    push(out, ProblemKind::BgpNeighbor, Severity::Error, dev, alloc::vec![odev], msg!("{}: der Nachbar {} ({}) ist in AS {}, nicht in AS {} (neighbor {} remote-as).", host, a, oname, ob.cfg.asn, nc.remote_as, a));
                    continue;
                }
                // the addresses the other side knows this router by
                let mine: Vec<Ipv4> = ob.cfg.nbrs.keys().copied().filter(|x| st.is_mine(*x)).collect();
                let Some(&x) = mine.first() else {
                    push(out, ProblemKind::BgpNeighbor, Severity::Error, dev, alloc::vec![odev], msg!("{}: {} hat keine Adresse von {} als BGP-Nachbarn eingerichtet.", host, oname, host));
                    continue;
                };
                // a connection is taken when it comes from the address the other side expects
                let from_here = self.bgp_from(ni, b, a);
                let from_there = self.bgp_from(oi, ob, x);
                if from_here != Some(x) && from_there != Some(a) {
                    let s1 = from_here.unwrap_or_default();
                    let s2 = from_there.unwrap_or_default();
                    push(out, ProblemKind::BgpNeighbor, Severity::Error, dev, alloc::vec![odev], msg!("{}: {} erwartet diesen Router als {}, er verbindet sich aber von {}, und {} kommt von {} statt von {}: update-source passt nicht.", host, oname, x, s1, oname, s2, a));
                    continue;
                }
                if let Some((_, sent, code, sub)) = b.peers.get(&a).and_then(|p| p.last_error) {
                    let m = if sent {
                        msg!("{}: die Sitzung mit {} kommt nicht zustande, zuletzt NOTIFICATION {}/{} gesendet ({}).", host, a, code, sub, Msg::plain(wire::error_name(code, sub)))
                    } else {
                        msg!("{}: die Sitzung mit {} kommt nicht zustande, zuletzt NOTIFICATION {}/{} empfangen ({}).", host, a, code, sub, Msg::plain(wire::error_name(code, sub)))
                    };
                    push(out, ProblemKind::BgpNeighbor, Severity::Warning, dev, alloc::vec![odev], m);
                }
            }
        }
    }
}
