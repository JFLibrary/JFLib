// ospf -- OSPF version 2 after RFC 2328 on IOS-like routers and L3 switches: a service of the
// device's OS ("ospf") with its configuration ("router ospf"), its interfaces and neighbours, the
// link-state database of every area, flooding, the shortest-path tree and the routes it gives
// the RIB; the command lines are in cli/ospf.rs
//
//   OspfCfg                    process ID, router ID, networks with their areas, passive
//                              interfaces, reference bandwidth, the default route,
//                              redistribution, stub areas, distance, maximum-paths, logging of
//                              adjacency changes; per interface cost, priority, hello and dead
//                              interval, network type and area ("ip ospf ...")
//   OspfSet / Action::Ospf     changes of the configuration (Engine::ospf_apply); Process {
//                              on: false } is "no router ospf" (the interface settings stay),
//                              Clear is "clear ip ospf process"
//   Ospf                       the service: router_id(), ifaces() (OIf with its neighbours
//                              Nbr), database(area) and externals() (Entry), routes() (ORoute),
//                              border_routers(), spf_runs(), counters `n`; metrics neighbours,
//                              full, lsas, routes (gauges) and the packets counted
//   Engine::ospf(dev)          the service of a device (None before the first OSPF command)
//   Engine::ospf_rx            IP protocol 89 for the device (from ip_local in l3.rs) goes to the
//                              service's inbox as SvcEvent::Ip
//   Engine::ospf_kick(ni)      interfaces, addresses or routes of the device may have changed
//                              (link.rs, Engine::apply, RIP's routes): it looks again at once
//
// The protocol (section numbers of RFC 2328 in the code):
//
// Interfaces (9): an interface whose primary address a network statement covers (the most
// specific one, then the first) runs OSPF in that statement's area; "ip ospf N area M" on the
// interface goes before the statements.  Serial ports are point-to-point, loopbacks are in the
// state Loopback and advertised as host routes with cost 0 (12.4.1), everything else is a
// broadcast network; "ip ospf network" changes that.  The cost is the reference bandwidth (100
// Mbit/s, "auto-cost reference-bandwidth") divided by the speed of the link, at least 1, unless
// "ip ospf cost" sets it.  HelloInterval 10 s, RouterDeadInterval four times that unless set,
// RxmtInterval 5 s, InfTransDelay 1 s, priority 1.  A broadcast interface waits
// RouterDeadInterval (Waiting) unless a Hello names a DR or BDR first (BackupSeen), then elects
// the DR and the BDR exactly as 9.4 says (highest priority, then router ID; a DR stays); a
// passive interface sends and takes no packets and is the DR of its network at once.  Hellos go
// to AllSPFRouters every HelloInterval from the moment the interface comes up, with TTL 1 and
// IP precedence 6 (Internetwork Control, A.1); the DR and BDR also listen to AllDRouters.
//
// Input (8.2, 10.5): the checksum, version 2, the area of the interface, null authentication,
// a sender on the interface's network (not checked on point-to-point links) other than this
// router, AllDRouters only for the DR and the BDR; Hellos must agree on HelloInterval,
// RouterDeadInterval, the network mask (not on point-to-point links) and the E bit (stub areas).
// What fails is ignored with a note of its reason.
//
// Neighbours (10): the state machine of 10.3 from Down through Init, 2-Way, ExStart, Exchange
// and Loading to Full, with the adjacency rule of 10.4 (point-to-point always; on a broadcast
// network only with the DR and the BDR).  The exchange (10.6, 10.8): the higher router ID is
// master; the master's DD sequence number starts from the device's random numbers; it
// retransmits its last DD every RxmtInterval, the slave repeats its last one for a duplicate; a
// DD whose MTU the interface cannot take is refused (the neighbour stays in ExStart).  LSAs the
// neighbour has newer go on the request list; Loading asks for them with one Link State Request
// at a time (10.9), retransmitted every RxmtInterval.  Neighbours are known by their interface
// address; one that goes Down is forgotten.
//
// Flooding (13): a received LSA is checked (Fletcher checksum, type, no AS-external-LSA in a
// stub area), compared with the database copy (13.1), installed when newer (not within
// MinLSArrival of the last one), flooded out of the interfaces of its area (AS-external-LSAs out
// of every interface that is not in a stub area) with the rules of 13.3 (neighbours from
// Exchange on, the request lists of Exchange and Loading, not back to the sender, not back onto
// a broadcast network it came from by the DR or BDR, the BDR leaves it to the DR), to
// AllSPFRouters (DR, BDR, point-to-point) or AllDRouters; acknowledged by Table 19 (delayed
// acknowledgements after 1 s to the group, direct ones to the neighbour), a duplicate on the
// retransmission list is an implied acknowledgement, an older instance gets the database copy
// back.  Every LSA on a retransmission list is sent again every RxmtInterval until acknowledged
// (13.6).  LS age counts in whole seconds from the moment of installation, plus InfTransDelay
// on the wire.
//
// Origination (12.4): the router-LSA of every area (links after 12.4.1: Loopback host routes,
// point-to-point links to a Full neighbour plus the stub link of the subnet, a transit link when
// fully adjacent to the DR (or as DR to anyone), else a stub link; secondary addresses as stub
// links; bits B and E), the network-LSA of every network it is DR on with a Full neighbour, as
// ABR the summary-LSAs of 12.4.3 (intra-area routes into the other areas, inter-area ones only
// into non-backbone areas, type 4 for AS boundary routers, never into the area they come from,
// into a stub area only the default with StubDefaultCost 1 and with no-summary nothing else),
// and as AS boundary router the AS-external-LSAs of 12.4.4 (redistribution, the default route;
// forwarding address 0.0.0.0, tag 0).  A new instance waits until MinLSInterval (5 s) after the
// last one; every LSA of its own is renewed after LSRefreshTime (1800 s); one it no longer wants
// is flushed by premature aging (14.1); an own LSA that comes back newer (from before a restart)
// gets a new instance one sequence number further, or is flushed (13.4).  An LSA that reaches
// MaxAge is flooded once more and removed when no retransmission list holds it and no neighbour
// is in Exchange or Loading (14).
//
// Routes (16): SPF runs SPF_DELAY (200 ms, our choice: the RFC sets no delay) after a change of
// the contents of the database (13.2): Dijkstra per area with the bidirectional check, transit
// networks before routers at equal cost, next hops after 16.1.1 and all equal-cost paths, then
// the stub networks; inter-area routes from the summary-LSAs of the backbone on an ABR (of the
// one area otherwise, 16.2); AS-external routes over the ASBR or its forwarding address,
// type 1 before type 2, type 2 by its metric and then by the distance to the ASBR (16.4).  Intra
// before inter-area before external.  The RIB gets one route per next hop (up to maximum-paths,
// default 4) with distance 110 ("distance") and the cost as metric (the type 2 metric for E2).
//
// The rest: redistribution of connected networks (those without OSPF), static and RIP routes
// that win in the RIB; without "subnets" only classful networks (as IOS-like routers do), never
// the default route (that is default-information originate: with "always" or while the RIB has a
// default route of another source); default metrics 20 and 1, type 2.  The router ID is the
// configured one, else the highest loopback address, else the highest address of an interface
// that is up, chosen when the process starts; a new one takes effect with "clear ip ospf
// process".  Power off and "clear ip ospf process" forget neighbours, database and routes.
//
// Not here: virtual links (and transit areas, 16.3), area ranges, NSSA, demand circuits,
// NBMA and point-to-multipoint networks, authentication other than null, TOS other than 0,
// more than one process per device; two networks with the same address but different masks
// get one summary- or AS-external-LSA (appendix E is not done).

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;
use core::cmp::Ordering;

use jfn_model::catalog::{Media, F_ROUTER};
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::eth::ET_IPV4;
use netpkt::ipv4::{Ipv4Hdr, Ipv4View, PROTO_OSPF};
use netpkt::ospf::{self as wire, Body, Dd, Hello, Lsa, LsaBody, LsaHeader, LsaKey, OspfView, Packet};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::{wildcard_match, Ipv4, Mac, Prefix4};

use crate::action::ActErr;
use crate::ip::{IfKind, Origin as IpOrigin, Route, RouteSrc};
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Ctl, IpPkt, Job, Metric, OsApi, Service, SvcEvent, SLOT_SVC, SVC_TIMERS};
use crate::series::MetricKind;
use crate::trace::Note;

/// the kind and the name of the service
pub const KIND: &str = "ospf";
/// the administrative distance of OSPF routes
pub const DISTANCE: u8 = 110;
/// equal-cost paths in the RIB by default
pub const MAX_PATHS: u8 = 4;
/// the reference bandwidth of interface costs in Mbit/s ("auto-cost reference-bandwidth")
pub const REF_BW: u32 = 100;
/// HelloInterval, RxmtInterval, InfTransDelay (seconds) and the router priority by default
pub const HELLO: u16 = 10;
pub const RXMT: u16 = 5;
pub const TRANS_DELAY: u16 = 1;
pub const PRIORITY: u8 = 1;
/// the metrics of redistributed routes and of the default route when none is given
pub const EXT_METRIC: u32 = 20;
pub const DEFAULT_METRIC: u32 = 1;
/// StubDefaultCost: the cost of the default summary an ABR gives a stub area
pub const STUB_COST: u32 = 1;
/// SPF runs this long after the first change of the database it has not taken in yet
pub const SPF_DELAY: Ns = 200 * MS;
/// delayed acknowledgements go out this long after the first one waits (less than
/// RxmtInterval, 13.5)
pub const ACK_DELAY: Ns = SEC;

// the service's timer keys: the earliest of its deadlines, a look at the interfaces
const T_TICK: u32 = 0;
const T_LOOK: u32 = 3;

/// IP precedence 6 (Internetwork Control) as DSCP: what OSPF packets carry (A.1)
const DSCP_CS6: u8 = 48;

/// the cost of an interface from its speed: reference bandwidth (Mbit/s) / speed, 1 to 65535
pub fn auto_cost(ref_mbps: u32, kbps: u32) -> u16 {
    let c = (ref_mbps as u64 * 1000) / (kbps.max(1) as u64);
    c.clamp(1, 65535) as u16
}

/// "0" or "0.0.0.1": an area ID as the configuration wrote it
pub fn area_str(a: u32, dotted: bool) -> String {
    if dotted { format!("{}", Ipv4::from_u32(a)) } else { format!("{}", a) }
}

fn classful_len(a: Ipv4) -> u8 {
    match a.0[0] {
        0..=127 => 8,
        128..=191 => 16,
        192..=223 => 24,
        _ => 32,
    }
}

/// signed comparison of LS sequence numbers
fn seq_cmp(a: u32, b: u32) -> Ordering {
    (a as i32).cmp(&(b as i32))
}

// ---------------------------------------------------------------- the words of states

/// what kind of network an interface is on
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NetType {
    Broadcast,
    PointToPoint,
    /// a loopback interface (the interface state Loopback)
    Loopback,
}

impl NetType {
    /// as "show ip ospf interface" and the configuration name it
    pub fn word(self) -> &'static str {
        match self {
            NetType::Broadcast => "BROADCAST",
            NetType::PointToPoint => "POINT_TO_POINT",
            NetType::Loopback => "LOOPBACK",
        }
    }
}

/// the states of an interface (9.1)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IfState {
    Down,
    Loopback,
    Waiting,
    PointToPoint,
    DrOther,
    Backup,
    Dr,
}

impl IfState {
    /// the name of RFC 2328
    pub fn name(self) -> &'static str {
        match self {
            IfState::Down => "Down",
            IfState::Loopback => "Loopback",
            IfState::Waiting => "Waiting",
            IfState::PointToPoint => "Point-to-point",
            IfState::DrOther => "DR Other",
            IfState::Backup => "Backup",
            IfState::Dr => "DR",
        }
    }
    /// the short word of the tables ("show ip ospf interface brief")
    pub fn word(self) -> &'static str {
        match self {
            IfState::Down => "DOWN",
            IfState::Loopback => "LOOP",
            IfState::Waiting => "WAIT",
            IfState::PointToPoint => "P2P",
            IfState::DrOther => "DROTH",
            IfState::Backup => "BDR",
            IfState::Dr => "DR",
        }
    }
}

/// the states of a neighbour (10.1)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NbrState {
    Down,
    Attempt,
    Init,
    TwoWay,
    ExStart,
    Exchange,
    Loading,
    Full,
}

impl NbrState {
    /// the name of RFC 2328
    pub fn name(self) -> &'static str {
        match self {
            NbrState::Down => "Down",
            NbrState::Attempt => "Attempt",
            NbrState::Init => "Init",
            NbrState::TwoWay => "2-Way",
            NbrState::ExStart => "ExStart",
            NbrState::Exchange => "Exchange",
            NbrState::Loading => "Loading",
            NbrState::Full => "Full",
        }
    }
    /// the word of the tables ("show ip ospf neighbor")
    pub fn word(self) -> &'static str {
        match self {
            NbrState::Down => "DOWN",
            NbrState::Attempt => "ATTEMPT",
            NbrState::Init => "INIT",
            NbrState::TwoWay => "2WAY",
            NbrState::ExStart => "EXSTART",
            NbrState::Exchange => "EXCHANGE",
            NbrState::Loading => "LOADING",
            NbrState::Full => "FULL",
        }
    }
}

/// the events of the neighbour state machine (10.2)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NbrEvent {
    HelloReceived,
    TwoWayReceived,
    NegotiationDone,
    ExchangeDone,
    BadLsReq,
    LoadingDone,
    AdjOk,
    SeqNumberMismatch,
    OneWay,
    KillNbr,
    InactivityTimer,
    LlDown,
}

impl NbrEvent {
    pub fn name(self) -> &'static str {
        match self {
            NbrEvent::HelloReceived => "HelloReceived",
            NbrEvent::TwoWayReceived => "2-WayReceived",
            NbrEvent::NegotiationDone => "NegotiationDone",
            NbrEvent::ExchangeDone => "ExchangeDone",
            NbrEvent::BadLsReq => "BadLSReq",
            NbrEvent::LoadingDone => "Loading Done",
            NbrEvent::AdjOk => "AdjOK?",
            NbrEvent::SeqNumberMismatch => "SeqNumberMismatch",
            NbrEvent::OneWay => "1-WayReceived",
            NbrEvent::KillNbr => "KillNbr",
            NbrEvent::InactivityTimer => "InactivityTimer",
            NbrEvent::LlDown => "LLDown",
        }
    }
}

/// why a packet was ignored
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OspfDrop {
    Checksum,
    Version,
    Malformed,
    Area,
    Auth,
    OwnId,
    Subnet,
    Mask,
    HelloTime,
    DeadTime,
    Stub,
    AllDRouters,
    Passive,
    Disabled,
    /// from no neighbour, or one whose state is too low for the packet
    State,
    Mtu,
    /// the master ignores a repeat of the slave's last DD
    Duplicate,
}

impl OspfDrop {
    /// the reason in German (a phrase of the catalogue)
    pub fn text(self) -> &'static str {
        match self {
            // i18n
            OspfDrop::Checksum => "die Prüfsumme stimmt nicht",
            OspfDrop::Version => "es ist nicht OSPF Version 2",
            OspfDrop::Malformed => "das Paket ist unlesbar",
            OspfDrop::Area => "es gehört zu einem anderen Bereich",
            OspfDrop::Auth => "die Authentifizierung passt nicht",
            OspfDrop::OwnId => "es trägt die eigene Router-ID",
            OspfDrop::Subnet => "der Absender liegt nicht im Netz des Interfaces",
            OspfDrop::Mask => "die Netzmaske passt nicht",
            OspfDrop::HelloTime => "das Hello-Intervall passt nicht",
            OspfDrop::DeadTime => "das Dead-Intervall passt nicht",
            OspfDrop::Stub => "die Stub-Einstellung des Bereichs passt nicht (E-Bit)",
            OspfDrop::AllDRouters => "es geht an AllDRouters, das Interface ist weder DR noch BDR",
            OspfDrop::Passive => "das Interface ist passiv",
            OspfDrop::Disabled => "OSPF läuft auf diesem Interface nicht",
            OspfDrop::State => "der Nachbar ist dafür nicht weit genug",
            OspfDrop::Mtu => "die MTU des Nachbarn ist größer als die des Interfaces",
            OspfDrop::Duplicate => "eine Wiederholung der letzten Database Description",
        }
    }
    /// the reason for the console
    pub fn en(self) -> &'static str {
        match self {
            OspfDrop::Checksum => "the checksum is wrong",
            OspfDrop::Version => "it is not OSPF version 2",
            OspfDrop::Malformed => "the packet is unreadable",
            OspfDrop::Area => "it belongs to another area",
            OspfDrop::Auth => "the authentication does not match",
            OspfDrop::OwnId => "it carries this router's own router ID",
            OspfDrop::Subnet => "the sender is not on the interface's network",
            OspfDrop::Mask => "the network mask does not match",
            OspfDrop::HelloTime => "the hello interval does not match",
            OspfDrop::DeadTime => "the dead interval does not match",
            OspfDrop::Stub => "the stub setting of the area does not match (E bit)",
            OspfDrop::AllDRouters => "it goes to AllDRouters, the interface is neither DR nor BDR",
            OspfDrop::Passive => "the interface is passive",
            OspfDrop::Disabled => "OSPF does not run on this interface",
            OspfDrop::State => "the neighbour is not far enough for it",
            OspfDrop::Mtu => "the neighbour's MTU is larger than the interface's",
            OspfDrop::Duplicate => "a repeat of the last database description",
        }
    }
}

/// why a received LSA was dropped
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LsaDrop {
    Checksum,
    Type,
    StubExternal,
    /// the last instance came less than MinLSArrival ago
    MinArrival,
}

impl LsaDrop {
    pub fn text(self) -> &'static str {
        match self {
            // i18n
            LsaDrop::Checksum => "seine Prüfsumme stimmt nicht",
            LsaDrop::Type => "der LSA-Typ ist unbekannt",
            LsaDrop::StubExternal => "ein AS-externes LSA gehört nicht in einen Stub-Bereich",
            LsaDrop::MinArrival => "die letzte Instanz kam vor weniger als 1 s (MinLSArrival)",
        }
    }
    pub fn en(self) -> &'static str {
        match self {
            LsaDrop::Checksum => "its checksum is wrong",
            LsaDrop::Type => "the LSA type is unknown",
            LsaDrop::StubExternal => "an AS-external LSA does not belong in a stub area",
            LsaDrop::MinArrival => "the last instance came less than 1 s ago (MinLSArrival)",
        }
    }
}

/// why the router made a new instance of an LSA of its own
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrigWhy {
    New,
    Changed,
    /// LSRefreshTime is over
    Refresh,
    /// an older instance of its own came back newer (13.4)
    Reclaim,
}

impl OrigWhy {
    pub fn text(self) -> &'static str {
        match self {
            // i18n
            OrigWhy::New => "neu",
            OrigWhy::Changed => "geändert",
            OrigWhy::Refresh => "erneuert nach 1800 s",
            OrigWhy::Reclaim => "eine eigene Instanz von früher kam zurück",
        }
    }
    pub fn en(self) -> &'static str {
        match self {
            OrigWhy::New => "new",
            OrigWhy::Changed => "changed",
            OrigWhy::Refresh => "refreshed after 1800 s",
            OrigWhy::Reclaim => "an own instance from before came back",
        }
    }
}

/// why a Link State Update went out
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LsuWhy {
    /// flooding (13.3)
    Flood,
    /// the answer to a Link State Request (10.7)
    Answer,
    /// LSAs not acknowledged in time (13.6)
    Rxmt,
    /// the database copy for a neighbour that sent an older instance (13, step 8)
    Back,
}

/// the kind of a route: intra-area, inter-area, external type 1 or 2
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RtKind {
    Intra,
    Inter,
    Ext1,
    Ext2,
}

impl RtKind {
    /// what show ip route writes after the O: "", "IA", "E1", "E2"
    pub fn code(self) -> &'static str {
        match self {
            RtKind::Intra => "",
            RtKind::Inter => "IA",
            RtKind::Ext1 => "E1",
            RtKind::Ext2 => "E2",
        }
    }
    pub fn text(self) -> &'static str {
        match self {
            // i18n
            RtKind::Intra => "im Bereich",
            RtKind::Inter => "aus einem anderen Bereich",
            RtKind::Ext1 => "extern Typ 1",
            RtKind::Ext2 => "extern Typ 2",
        }
    }
    pub fn en(self) -> &'static str {
        match self {
            RtKind::Intra => "intra-area",
            RtKind::Inter => "inter-area",
            RtKind::Ext1 => "external type 1",
            RtKind::Ext2 => "external type 2",
        }
    }
}

/// what happened to a route in an SPF run
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RtChange {
    Added,
    Changed,
    Removed,
}

/// an LSA instance in notes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LsaRef {
    pub kind: u8,
    pub id: Ipv4,
    pub adv: Ipv4,
    pub seq: u32,
    /// the area of an area-scoped LSA (None: AS-external)
    pub area: Option<u32>,
}

impl LsaRef {
    pub fn of(h: &LsaHeader, sc: Scope) -> LsaRef {
        LsaRef { kind: h.kind, id: h.id, adv: h.adv, seq: h.seq, area: sc.area() }
    }
    /// "Router-LSA 1.1.1.1 von 1.1.1.1 (0x80000002, Bereich 0)"
    pub fn msg(&self) -> Msg {
        match self.area {
            Some(a) => msg!("{} {} von {} (0x{}, Bereich {})", lsa_short(self.kind), self.id, self.adv, format!("{:08x}", self.seq), a),
            None => msg!("{} {} von {} (0x{})", lsa_short(self.kind), self.id, self.adv, format!("{:08x}", self.seq)),
        }
    }
    /// the same in English (debug lines)
    pub fn en(&self) -> String {
        match self.area {
            Some(a) => format!("{} {} from {} (0x{:08x}, area {})", lsa_short(self.kind), self.id, self.adv, self.seq, a),
            None => format!("{} {} from {} (0x{:08x})", lsa_short(self.kind), self.id, self.adv, self.seq),
        }
    }
}

/// the short name of an LSA type in notes and tables
pub fn lsa_short(kind: u8) -> &'static str {
    match kind {
        wire::LSA_ROUTER => "Router-LSA",
        wire::LSA_NETWORK => "Network-LSA",
        wire::LSA_SUMMARY => "Summary-LSA",
        wire::LSA_ASBR => "ASBR-Summary-LSA",
        wire::LSA_EXTERNAL => "AS-External-LSA",
        _ => "LSA",
    }
}

// ---------------------------------------------------------------- configuration

/// "network A.B.C.D WILDCARD area N"
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetArea {
    pub addr: Ipv4,
    pub wild: Ipv4,
    pub area: u32,
    /// the area ID was written dotted ("area 0.0.0.1")
    pub dotted: bool,
}

impl NetArea {
    pub fn covers(&self, a: Ipv4) -> bool {
        wildcard_match(a, self.addr, self.wild)
    }
    /// how many bits must match: the more, the more specific
    fn bits(&self) -> u32 {
        (!self.wild.to_u32()).count_ones()
    }
}

/// what redistribution takes in
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Connected,
    Static,
    Rip,
}

impl Source {
    pub fn word(self) -> &'static str {
        match self {
            Source::Connected => "connected",
            Source::Static => "static",
            Source::Rip => "rip",
        }
    }
}

/// "redistribute connected|static|rip [subnets] [metric N] [metric-type 1|2]"
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Redist {
    pub what: Source,
    pub subnets: bool,
    pub metric: Option<u32>,
    /// external type 1 (else 2)
    pub e1: bool,
}

/// "default-information originate [always] [metric N] [metric-type 1|2]"
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefaultOrig {
    pub always: bool,
    pub metric: Option<u32>,
    pub e1: bool,
}

/// the configuration of OSPF on a device
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OspfCfg {
    /// "router ospf N" is there
    pub on: bool,
    pub pid: u16,
    /// "router-id A.B.C.D"
    pub router_id: Option<Ipv4>,
    /// the network statements in the order they were made
    pub networks: Vec<NetArea>,
    /// "passive-interface default"; `passive` then lists the exceptions
    pub passive_default: bool,
    pub passive: Vec<String>,
    /// Mbit/s
    pub ref_bw: u32,
    pub default_orig: Option<DefaultOrig>,
    /// by source
    pub redistribute: Vec<Redist>,
    /// stub areas: no-summary
    pub stubs: BTreeMap<u32, bool>,
    pub distance: u8,
    pub max_paths: u8,
    /// "log-adjacency-changes [detail]" (on by default)
    pub log_adj: bool,
    pub log_detail: bool,
    /// the "ip ospf ..." settings by interface name
    pub cost: BTreeMap<String, u16>,
    pub priority: BTreeMap<String, u8>,
    pub hello: BTreeMap<String, u16>,
    pub dead: BTreeMap<String, u32>,
    pub net_type: BTreeMap<String, NetType>,
    /// "ip ospf PID area A": (process, area, written dotted)
    pub if_area: BTreeMap<String, (u16, u32, bool)>,
}

impl Default for OspfCfg {
    fn default() -> OspfCfg {
        OspfCfg {
            on: false,
            pid: 0,
            router_id: None,
            networks: Vec::new(),
            passive_default: false,
            passive: Vec::new(),
            ref_bw: REF_BW,
            default_orig: None,
            redistribute: Vec::new(),
            stubs: BTreeMap::new(),
            distance: DISTANCE,
            max_paths: MAX_PATHS,
            log_adj: true,
            log_detail: false,
            cost: BTreeMap::new(),
            priority: BTreeMap::new(),
            hello: BTreeMap::new(),
            dead: BTreeMap::new(),
            net_type: BTreeMap::new(),
            if_area: BTreeMap::new(),
        }
    }
}

fn get_ci<'a, V>(m: &'a BTreeMap<String, V>, name: &str) -> Option<&'a V> {
    m.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v)
}

impl OspfCfg {
    pub fn is_passive(&self, ifname: &str) -> bool {
        self.passive_default != self.passive.iter().any(|p| p.eq_ignore_ascii_case(ifname))
    }
    /// the area an interface with this name and primary address runs in: "ip ospf N area M"
    /// first, then the most specific network statement (the first of equally specific ones)
    pub fn area_of(&self, ifname: &str, a: Ipv4) -> Option<u32> {
        if let Some(&(pid, area, _)) = get_ci(&self.if_area, ifname)
            && pid == self.pid
        {
            return Some(area);
        }
        let mut best: Option<&NetArea> = None;
        for n in self.networks.iter().filter(|n| n.covers(a)) {
            if best.is_none_or(|b| n.bits() > b.bits()) {
                best = Some(n);
            }
        }
        best.map(|n| n.area)
    }
    /// how the configuration names an area ("0", "0.0.0.1")
    pub fn area_text(&self, area: u32) -> String {
        let dotted = self.networks.iter().find(|n| n.area == area).map(|n| n.dotted).or_else(|| self.if_area.values().find(|x| x.1 == area).map(|x| x.2)).unwrap_or(false);
        area_str(area, dotted)
    }
    pub fn is_stub(&self, area: u32) -> bool {
        area != 0 && self.stubs.contains_key(&area)
    }
    /// the device originates AS-external-LSAs (redistribution or the default route)
    pub fn asbr(&self) -> bool {
        !self.redistribute.is_empty() || self.default_orig.is_some()
    }
    pub fn hello_of(&self, ifname: &str) -> u16 {
        get_ci(&self.hello, ifname).copied().unwrap_or(HELLO)
    }
    /// RouterDeadInterval: as set, else four times HelloInterval
    pub fn dead_of(&self, ifname: &str) -> u32 {
        get_ci(&self.dead, ifname).copied().unwrap_or(self.hello_of(ifname) as u32 * 4)
    }
    /// "no router ospf": the process settings back to their defaults; the interfaces keep theirs
    fn reset(&mut self) {
        let keep = (
            core::mem::take(&mut self.cost),
            core::mem::take(&mut self.priority),
            core::mem::take(&mut self.hello),
            core::mem::take(&mut self.dead),
            core::mem::take(&mut self.net_type),
            core::mem::take(&mut self.if_area),
        );
        *self = OspfCfg { cost: keep.0, priority: keep.1, hello: keep.2, dead: keep.3, net_type: keep.4, if_area: keep.5, ..OspfCfg::default() };
    }
}

/// a change of the configuration (Action::Ospf)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OspfSet {
    /// "router ospf N" (on) / "no router ospf N"
    Process { pid: u16, on: bool },
    RouterId(Option<Ipv4>),
    Network { net: NetArea, add: bool },
    /// "passive-interface IF" (Some) or "passive-interface default" (None); `on: false` for the
    /// no-form
    Passive { iface: Option<String>, on: bool },
    /// "auto-cost reference-bandwidth N" (None: the default)
    RefBw(Option<u32>),
    DefaultOrig(Option<DefaultOrig>),
    Redistribute { r: Redist, add: bool },
    /// "area N stub [no-summary]"
    Stub { area: u32, no_summary: bool, add: bool },
    Distance(Option<u8>),
    MaxPaths(Option<u8>),
    LogAdj { on: bool, detail: bool },
    /// the "ip ospf ..." settings of an interface (None: the default)
    IfCost { iface: String, cost: Option<u16> },
    IfPriority { iface: String, priority: Option<u8> },
    IfHello { iface: String, secs: Option<u16> },
    IfDead { iface: String, secs: Option<u32> },
    IfNetwork { iface: String, kind: Option<NetType> },
    IfArea { iface: String, area: Option<(u16, u32, bool)> },
    /// "clear ip ospf process"
    Clear,
}

// ---------------------------------------------------------------- what runs

/// a neighbour (10)
#[derive(Clone, Debug)]
pub struct Nbr {
    /// its router ID
    pub id: Ipv4,
    /// its interface address
    pub addr: Ipv4,
    pub state: NbrState,
    pub priority: u8,
    /// the DR and BDR its Hello declares (interface addresses)
    pub dr: Ipv4,
    pub bdr: Ipv4,
    pub options: u8,
    /// it is the master of the exchange (this router the slave)
    pub master: bool,
    pub dd_seq: u32,
    /// the flags, options and sequence number of the last DD received
    last_rx: Option<(u8, u8, u32)>,
    /// the last DD sent (repeated by the master after RxmtInterval, by the slave for a duplicate)
    last_tx: Option<Packet>,
    /// the last DD sent had the M bit
    more: bool,
    /// the database summary list (10.3)
    summary: VecDeque<(Scope, LsaKey)>,
    /// the link state request list: what the neighbour has newer
    pub requests: BTreeMap<LsaKey, LsaHeader>,
    /// what the outstanding Link State Request asks for
    asked: Vec<LsaKey>,
    /// the link state retransmission list: the instance flooded and when it was sent last
    pub rxmt: BTreeMap<LsaKey, (LsaHeader, Ns)>,
    /// direct acknowledgements and database copies to send at the end of this event
    direct: Vec<LsaHeader>,
    back: Vec<LsaKey>,
    /// when the database copy of an LSA was last sent back to it
    sent_back: BTreeMap<LsaKey, Ns>,
    /// the inactivity timer
    pub dead_at: Ns,
    dd_at: Option<Ns>,
    lsr_at: Option<Ns>,
    /// when it reached its state
    pub since: Ns,
}

impl Nbr {
    fn new(id: Ipv4, addr: Ipv4, now: Ns) -> Nbr {
        Nbr {
            id,
            addr,
            state: NbrState::Down,
            priority: 0,
            dr: Ipv4::UNSPECIFIED,
            bdr: Ipv4::UNSPECIFIED,
            options: 0,
            master: false,
            dd_seq: 0,
            last_rx: None,
            last_tx: None,
            more: false,
            summary: VecDeque::new(),
            requests: BTreeMap::new(),
            asked: Vec::new(),
            rxmt: BTreeMap::new(),
            direct: Vec::new(),
            back: Vec::new(),
            sent_back: BTreeMap::new(),
            dead_at: now,
            dd_at: None,
            lsr_at: None,
            since: now,
        }
    }
    /// the lists of the exchange are emptied (10.3: back to 2-Way, Init, ExStart or Down)
    fn clear_lists(&mut self) {
        self.summary.clear();
        self.requests.clear();
        self.asked.clear();
        self.rxmt.clear();
        self.last_rx = None;
        self.dd_at = None;
        self.lsr_at = None;
    }
}

/// an interface OSPF runs on (9)
#[derive(Clone, Debug)]
pub struct OIf {
    pub name: String,
    /// its primary address and the secondary ones
    pub addr: Prefix4,
    pub secondaries: Vec<Prefix4>,
    pub area: u32,
    pub kind: NetType,
    pub state: IfState,
    pub cost: u16,
    pub priority: u8,
    pub hello: u16,
    pub dead: u32,
    pub passive: bool,
    /// a loopback interface: it sends nothing, whatever its network type
    pub lo: bool,
    pub mtu: u16,
    /// the DR and BDR (interface addresses; 0.0.0.0: none)
    pub dr: Ipv4,
    pub bdr: Ipv4,
    /// the neighbours by their interface address
    pub nbrs: BTreeMap<Ipv4, Nbr>,
    pub hello_at: Option<Ns>,
    pub wait_at: Option<Ns>,
    ack_at: Option<Ns>,
    delayed: Vec<LsaHeader>,
    /// LSAs to flood out of it at the end of this event
    flood_q: Vec<(Scope, LsaKey)>,
    /// a neighbour changed in a way the DR election must hear of (NeighborChange), a Hello
    /// named a DR or BDR while it waits (BackupSeen)
    elect: bool,
    seen: bool,
    /// when it reached its state
    pub since: Ns,
}

impl OIf {
    /// the role of a neighbour on this network: "DR", "BDR", "DROTHER", "  -" (point-to-point)
    pub fn role(&self, addr: Ipv4) -> &'static str {
        if self.kind == NetType::PointToPoint {
            "  -"
        } else if addr == self.dr {
            "DR"
        } else if addr == self.bdr {
            "BDR"
        } else {
            "DROTHER"
        }
    }
    /// the router ID of the router with this interface address on the network
    pub fn rid_of(&self, addr: Ipv4, me: Ipv4) -> Ipv4 {
        if addr == self.addr.addr {
            return me;
        }
        self.nbrs.get(&addr).map(|n| n.id).unwrap_or(Ipv4::UNSPECIFIED)
    }
    fn running(&self) -> bool {
        self.state != IfState::Down
    }
    /// it sends and takes packets (up, not passive, not a loopback)
    fn speaks(&self) -> bool {
        self.state != IfState::Down && self.state != IfState::Loopback && !self.passive && !self.lo
    }
}

/// where an LSA floods: an area, or the whole AS (AS-external-LSAs)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    Area(u32),
    As,
}

fn scope_of(kind: u8, area: u32) -> Scope {
    if kind == wire::LSA_EXTERNAL { Scope::As } else { Scope::Area(area) }
}

/// an interface belongs to where an LSA floods: its area, or (AS-external-LSAs) any area that
/// is not a stub area
fn in_scope(f: &OIf, sc: Scope, stubs: &[u32]) -> bool {
    match sc {
        Scope::Area(a) => f.area == a,
        Scope::As => !stubs.contains(&f.area),
    }
}

impl Scope {
    /// the area of an area-scoped LSA (notes)
    pub fn area(self) -> Option<u32> {
        match self {
            Scope::Area(a) => Some(a),
            Scope::As => None,
        }
    }
}

/// an LSA in the database
#[derive(Clone, Debug)]
pub struct Entry {
    /// the LSA as received or originated (its age field as installed)
    pub raw: Vec<u8>,
    pub lsa: Lsa,
    /// installed at, with this age: the age counts on from there
    pub at: Ns,
    pub age0: u16,
    /// the neighbour (router ID) it came from; None: originated here
    pub from: Option<Ipv4>,
    /// it reached MaxAge and was flooded so
    aged: bool,
}

impl Entry {
    pub fn age(&self, now: Ns) -> u16 {
        (self.age0 as u64 + now.saturating_sub(self.at) / SEC).min(wire::MAX_AGE as u64) as u16
    }
    pub fn hdr(&self, now: Ns) -> LsaHeader {
        LsaHeader { age: self.age(now), ..self.lsa.hdr }
    }
    pub fn maxage(&self, now: Ns) -> bool {
        self.age(now) >= wire::MAX_AGE
    }
    /// the bytes to send: the age now plus InfTransDelay, at most MaxAge
    fn wire(&self, now: Ns) -> Vec<u8> {
        let mut b = self.raw.clone();
        wire::set_age(&mut b, (self.age(now) + TRANS_DELAY).min(wire::MAX_AGE));
        b
    }
}

/// a route of the OSPF routing table
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ORoute {
    pub kind: RtKind,
    /// the area of its paths (externals: of the ASBR)
    pub area: u32,
    /// the cost (type 2: the distance to the ASBR or forwarding address)
    pub cost: u32,
    /// the type 2 metric
    pub cost2: u32,
    /// the next hops: (address, interface); 0.0.0.0 for a network the router is on
    pub hops: Vec<(Ipv4, u16)>,
    /// since when it is as it is
    pub since: Ns,
}

impl ORoute {
    /// the metric the RIB shows
    pub fn metric(&self) -> u32 {
        if self.kind == RtKind::Ext2 { self.cost2 } else { self.cost }
    }
    /// what makes one route better than another of the same destination (lower is better)
    fn rank(&self) -> (RtKind, u32, u32) {
        match self.kind {
            RtKind::Ext2 => (self.kind, self.cost2, self.cost),
            k => (k, self.cost, 0),
        }
    }
}

/// a route to an area border or AS boundary router
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BRoute {
    pub cost: u32,
    pub hops: Vec<(Ipv4, u16)>,
    /// RTR_B, RTR_E of its router-LSA (an ASBR known from a summary: RTR_E)
    pub flags: u8,
    /// known from a type 4 summary-LSA
    pub inter: bool,
}

/// what the router keeps about an LSA of its own
#[derive(Clone, Copy, Debug, Default)]
struct Own {
    /// when it made the last instance, and its sequence number
    last: Option<Ns>,
    seq: u32,
    /// a new instance waits for MinLSInterval
    due: Option<Ns>,
}

/// what OSPF counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub hellos_sent: u64,
    pub hellos_rcvd: u64,
    pub dd_sent: u64,
    pub dd_rcvd: u64,
    pub lsr_sent: u64,
    pub lsr_rcvd: u64,
    pub lsu_sent: u64,
    pub lsu_rcvd: u64,
    pub acks_sent: u64,
    pub acks_rcvd: u64,
    pub retransmitted: u64,
    pub originated: u64,
    pub ignored: u64,
    pub adj_changes: u64,
}

/// OSPF on a device
#[derive(Clone, Debug)]
pub struct Ospf {
    pub cfg: OspfCfg,
    running: bool,
    /// the router ID in use (None: no address to take one from yet)
    rid: Option<Ipv4>,
    /// when the process started
    started: Ns,
    ifs: BTreeMap<u16, OIf>,
    lsdb: BTreeMap<u32, BTreeMap<LsaKey, Entry>>,
    ext: BTreeMap<LsaKey, Entry>,
    rt: BTreeMap<Prefix4, ORoute>,
    brt: BTreeMap<(u32, Ipv4), BRoute>,
    own: BTreeMap<(Scope, LsaKey), Own>,
    spf_at: Option<Ns>,
    last_spf: Option<Ns>,
    spf_runs: u64,
    /// the deadline timer as armed
    armed: Option<Ns>,
    pub n: Counters,
}

impl Default for Ospf {
    fn default() -> Ospf {
        Ospf::new()
    }
}

// ---------------------------------------------------------------- what others read

impl Ospf {
    pub fn new() -> Ospf {
        Ospf {
            cfg: OspfCfg::default(),
            running: false,
            rid: None,
            started: 0,
            ifs: BTreeMap::new(),
            lsdb: BTreeMap::new(),
            ext: BTreeMap::new(),
            rt: BTreeMap::new(),
            brt: BTreeMap::new(),
            own: BTreeMap::new(),
            spf_at: None,
            last_spf: None,
            spf_runs: 0,
            armed: None,
            n: Counters::default(),
        }
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
    /// the interfaces OSPF runs on, by IPv4 interface
    pub fn ifaces(&self) -> &BTreeMap<u16, OIf> {
        &self.ifs
    }
    /// the areas with their LSAs
    pub fn areas(&self) -> &BTreeMap<u32, BTreeMap<LsaKey, Entry>> {
        &self.lsdb
    }
    pub fn database(&self, area: u32) -> Option<&BTreeMap<LsaKey, Entry>> {
        self.lsdb.get(&area)
    }
    /// the AS-external-LSAs
    pub fn externals(&self) -> &BTreeMap<LsaKey, Entry> {
        &self.ext
    }
    /// the routing table by destination
    pub fn routes(&self) -> &BTreeMap<Prefix4, ORoute> {
        &self.rt
    }
    /// the routes to area border and AS boundary routers, by area and router
    pub fn border_routers(&self) -> &BTreeMap<(u32, Ipv4), BRoute> {
        &self.brt
    }
    pub fn spf_runs(&self) -> u64 {
        self.spf_runs
    }
    pub fn last_spf(&self) -> Option<Ns> {
        self.last_spf
    }
    pub fn next_spf(&self) -> Option<Ns> {
        self.spf_at
    }
    /// the areas with an interface that is not down
    pub fn attached(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.ifs.values().filter(|f| f.running()).map(|f| f.area).collect();
        v.sort();
        v.dedup();
        v
    }
    /// an area border router: attached to more than one area
    pub fn is_abr(&self) -> bool {
        self.attached().len() > 1
    }
    pub fn is_asbr(&self) -> bool {
        self.cfg.asbr()
    }
    /// neighbours in any state, and those that are Full
    pub fn neighbours(&self) -> (usize, usize) {
        let all = self.ifs.values().map(|f| f.nbrs.len()).sum();
        let full = self.ifs.values().map(|f| f.nbrs.values().filter(|n| n.state == NbrState::Full).count()).sum();
        (all, full)
    }
    /// how many LSAs the databases hold
    pub fn lsa_count(&self) -> usize {
        self.lsdb.values().map(|d| d.len()).sum::<usize>() + self.ext.len()
    }
    /// how the RIB shows a route of OSPF: its kind and since when it is as it is
    pub fn route_of(&self, p: Prefix4) -> Option<&ORoute> {
        self.rt.get(&p)
    }

    fn db(&self, sc: Scope) -> Option<&BTreeMap<LsaKey, Entry>> {
        match sc {
            Scope::Area(a) => self.lsdb.get(&a),
            Scope::As => Some(&self.ext),
        }
    }
    fn db_mut(&mut self, sc: Scope) -> &mut BTreeMap<LsaKey, Entry> {
        match sc {
            Scope::Area(a) => self.lsdb.entry(a).or_default(),
            Scope::As => &mut self.ext,
        }
    }
    fn entry(&self, sc: Scope, k: &LsaKey) -> Option<&Entry> {
        self.db(sc)?.get(k)
    }
    /// the LSA is one of this router's own (13.4): its router ID advertises it, or it is a
    /// network-LSA for one of its interface addresses
    fn is_own(&self, k: &LsaKey) -> bool {
        Some(k.adv) == self.rid || (k.kind == wire::LSA_NETWORK && self.ifs.values().any(|f| f.addr.addr == k.id))
    }
    fn is_stub(&self, area: u32) -> bool {
        self.cfg.is_stub(area)
    }
    /// the stub areas (AS-external-LSAs stay out of them)
    fn stub_areas(&self) -> Vec<u32> {
        self.cfg.stubs.keys().copied().filter(|a| *a != 0).collect()
    }
    /// the options of this router's packets and LSAs in an area: E unless it is a stub area
    fn options(&self, area: u32) -> u8 {
        if self.is_stub(area) { 0 } else { wire::OPT_E }
    }
    /// a neighbour is in Exchange or Loading (14: MaxAge LSAs stay meanwhile)
    fn exchanging(&self) -> bool {
        self.ifs.values().any(|f| f.nbrs.values().any(|n| matches!(n.state, NbrState::Exchange | NbrState::Loading)))
    }
}

// ---------------------------------------------------------------- the interfaces

/// what an interface should be from the configuration and the IPv4 part
struct Want {
    name: String,
    addr: Prefix4,
    secondaries: Vec<Prefix4>,
    area: u32,
    kind: NetType,
    cost: u16,
    priority: u8,
    hello: u16,
    dead: u32,
    passive: bool,
    lo: bool,
    mtu: u16,
    up: bool,
}

impl Ospf {
    /// the interface speed in kbit/s (the link's, else the port's; VLAN interfaces 1 Gbit/s,
    /// loopbacks 8 Gbit/s)
    fn speed(os: &OsApi, i: u16) -> u32 {
        let ni = os.ni;
        let Some(f) = os.iface(i) else {
            return 1_000_000;
        };
        let port = match f.kind {
            IfKind::Port(p) | IfKind::Sub { port: p, .. } => p,
            IfKind::Svi(_) => return 1_000_000,
            IfKind::Loopback(_) => return 8_000_000,
        };
        match os.eng.nodes.get(ni) {
            Some(Some(n)) => match n.ports.get(port as usize) {
                Some(p) => p.link.map(|li| os.eng.link_kbps(li)).filter(|&k| k > 0).unwrap_or(p.spec.kbps),
                None => 1_000_000,
            },
            _ => 1_000_000,
        }
    }

    /// the interfaces the configuration wants, with their settings
    fn wanted_ifs(&self, os: &OsApi) -> BTreeMap<u16, Want> {
        let mut want = BTreeMap::new();
        let forwarding = os.eng.stack(os.ni).is_some_and(|s| s.forwarding);
        if !self.cfg.on || !forwarding {
            return want;
        }
        for (i, f) in os.ifaces().iter().enumerate() {
            let i = i as u16;
            let Some(p) = f.addrs.first().copied() else {
                continue;
            };
            if f.hidden {
                continue;
            }
            let Some(area) = self.cfg.area_of(&f.name, p.addr) else {
                continue;
            };
            let serial = matches!(f.kind, IfKind::Port(pi) if matches!(os.eng.nodes.get(os.ni), Some(Some(n)) if n.ports.get(pi as usize).is_some_and(|x| x.spec.media == Media::Serial)));
            let kind = match get_ci(&self.cfg.net_type, &f.name).copied() {
                Some(k) => k,
                None if f.is_loopback() => NetType::Loopback,
                None if serial => NetType::PointToPoint,
                None => NetType::Broadcast,
            };
            let cost = get_ci(&self.cfg.cost, &f.name).copied().unwrap_or_else(|| auto_cost(self.cfg.ref_bw, Ospf::speed(os, i)));
            let secondaries: Vec<Prefix4> = f.addrs.iter().skip(1).filter(|q| self.cfg.area_of(&f.name, q.addr) == Some(area) || get_ci(&self.cfg.if_area, &f.name).is_some()).copied().collect();
            want.insert(
                i,
                Want {
                    name: f.name.clone(),
                    addr: p,
                    secondaries,
                    area,
                    kind,
                    cost,
                    priority: get_ci(&self.cfg.priority, &f.name).copied().unwrap_or(PRIORITY),
                    hello: self.cfg.hello_of(&f.name),
                    dead: self.cfg.dead_of(&f.name),
                    passive: self.cfg.is_passive(&f.name),
                    lo: f.is_loopback(),
                    mtu: f.mtu,
                    up: os.iface_up(i),
                },
            );
        }
        want
    }

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

    /// looks at the configuration and the interfaces again: interfaces come and go, go up and
    /// down, change their settings
    fn sync(&mut self, os: &mut OsApi) {
        if !self.running {
            return;
        }
        if self.rid.is_none() && self.cfg.on {
            self.rid = self.pick_rid(os);
            if let Some(id) = self.rid {
                os.note(Note::OspfRouterId { id });
            }
        }
        let want = if self.rid.is_some() { self.wanted_ifs(os) } else { BTreeMap::new() };
        let now = os.now();
        // interfaces that went away, or whose area, type or address changed: down (and gone)
        let keys: Vec<u16> = self.ifs.keys().copied().collect();
        for i in keys {
            let reset = match (self.ifs.get(&i), want.get(&i)) {
                (Some(f), Some(w)) => f.area != w.area || f.kind != w.kind || f.addr != w.addr || f.passive != w.passive || (f.running() && !w.up),
                _ => true,
            };
            if reset {
                self.if_down(os, i);
                if !want.contains_key(&i) {
                    self.ifs.remove(&i);
                }
            }
        }
        for (i, w) in want {
            let f = self.ifs.entry(i).or_insert_with(|| OIf {
                name: w.name.clone(),
                addr: w.addr,
                secondaries: Vec::new(),
                area: w.area,
                kind: w.kind,
                state: IfState::Down,
                cost: w.cost,
                priority: w.priority,
                hello: w.hello,
                dead: w.dead,
                passive: w.passive,
                lo: w.lo,
                mtu: w.mtu,
                dr: Ipv4::UNSPECIFIED,
                bdr: Ipv4::UNSPECIFIED,
                nbrs: BTreeMap::new(),
                hello_at: None,
                wait_at: None,
                ack_at: None,
                delayed: Vec::new(),
                flood_q: Vec::new(),
                elect: false,
                seen: false,
                since: now,
            });
            f.name = w.name;
            f.addr = w.addr;
            f.area = w.area;
            f.kind = w.kind;
            f.passive = w.passive;
            f.secondaries = w.secondaries;
            f.cost = w.cost;
            f.mtu = w.mtu;
            f.hello = w.hello;
            f.dead = w.dead;
            if f.priority != w.priority {
                f.priority = w.priority;
                f.elect = true;
            }
            self.lsdb.entry(w.area).or_default();
            if w.up && self.ifs.get(&i).is_some_and(|f| f.state == IfState::Down) {
                self.if_up(os, i);
            }
        }
        // areas no interface is in any more lose their database
        let areas: Vec<u32> = self.ifs.values().map(|f| f.area).collect();
        self.lsdb.retain(|a, _| areas.contains(a));
    }

    fn set_if_state(&mut self, os: &mut OsApi, i: u16, to: IfState) {
        let now = os.now();
        let Some(f) = self.ifs.get_mut(&i) else {
            return;
        };
        let from = f.state;
        if from == to {
            return;
        }
        f.state = to;
        f.since = now;
        os.note(Note::OspfIfState { iface: i, from, to });
    }

    /// InterfaceUp (9.3)
    fn if_up(&mut self, os: &mut OsApi, i: u16) {
        let now = os.now();
        let Some(f) = self.ifs.get_mut(&i) else {
            return;
        };
        f.dr = Ipv4::UNSPECIFIED;
        f.bdr = Ipv4::UNSPECIFIED;
        let (kind, passive, prio, dead, lo) = (f.kind, f.passive, f.priority, f.dead, f.lo);
        let to = match kind {
            NetType::Loopback => IfState::Loopback,
            NetType::PointToPoint => IfState::PointToPoint,
            NetType::Broadcast if passive => {
                // nobody to wait for: the DR of its network at once
                f.dr = f.addr.addr;
                IfState::Dr
            }
            NetType::Broadcast if prio == 0 => IfState::DrOther,
            NetType::Broadcast => {
                f.wait_at = Some(now + dead as Ns * SEC);
                IfState::Waiting
            }
        };
        if to != IfState::Loopback && !passive && !lo {
            f.hello_at = Some(now);
        }
        self.set_if_state(os, i, to);
    }

    /// InterfaceDown (9.3): the neighbours go (KillNbr), the timers stop
    fn if_down(&mut self, os: &mut OsApi, i: u16) {
        let Some(f) = self.ifs.get(&i) else {
            return;
        };
        if f.state == IfState::Down {
            return;
        }
        let nbrs: Vec<Ipv4> = f.nbrs.keys().copied().collect();
        for a in nbrs {
            self.nbr_event(os, i, a, NbrEvent::KillNbr);
        }
        if let Some(f) = self.ifs.get_mut(&i) {
            f.nbrs.clear();
            f.hello_at = None;
            f.wait_at = None;
            f.ack_at = None;
            f.delayed.clear();
            f.flood_q.clear();
            f.dr = Ipv4::UNSPECIFIED;
            f.bdr = Ipv4::UNSPECIFIED;
            f.elect = false;
            f.seen = false;
        }
        self.set_if_state(os, i, IfState::Down);
    }

    /// the DR election of 9.4 on a broadcast network (WaitTimer, BackupSeen, NeighborChange)
    fn elect(&mut self, os: &mut OsApi, i: u16) {
        let Some(rid) = self.rid else {
            return;
        };
        let Some(f) = self.ifs.get_mut(&i) else {
            return;
        };
        f.elect = false;
        f.seen = false;
        f.wait_at = None;
        if f.kind != NetType::Broadcast || f.passive || !f.running() {
            return;
        }
        let me = f.addr.addr;
        // the candidates: (address, router ID, priority, declared DR, declared BDR)
        let mut cands: Vec<(Ipv4, Ipv4, u8, Ipv4, Ipv4)> = Vec::new();
        if f.priority > 0 {
            cands.push((me, rid, f.priority, f.dr, f.bdr));
        }
        for n in f.nbrs.values().filter(|n| n.state >= NbrState::TwoWay && n.priority > 0) {
            cands.push((n.addr, n.id, n.priority, n.dr, n.bdr));
        }
        let best = |v: &[&(Ipv4, Ipv4, u8, Ipv4, Ipv4)]| v.iter().max_by_key(|c| (c.2, c.1)).map(|c| c.0);
        let calc = |cands: &[(Ipv4, Ipv4, u8, Ipv4, Ipv4)]| -> (Ipv4, Ipv4) {
            // step 2: the BDR among those that do not declare themselves DR
            let not_dr: Vec<&(Ipv4, Ipv4, u8, Ipv4, Ipv4)> = cands.iter().filter(|c| c.3 != c.0).collect();
            let decl: Vec<&(Ipv4, Ipv4, u8, Ipv4, Ipv4)> = not_dr.iter().copied().filter(|c| c.4 == c.0).collect();
            let bdr = best(if decl.is_empty() { &not_dr } else { &decl }).unwrap_or(Ipv4::UNSPECIFIED);
            // step 3: the DR among those that declare themselves DR, else the BDR
            let drs: Vec<&(Ipv4, Ipv4, u8, Ipv4, Ipv4)> = cands.iter().filter(|c| c.3 == c.0).collect();
            let dr = best(&drs).unwrap_or(bdr);
            (dr, bdr)
        };
        let (was_dr, was_bdr) = (f.dr == me, f.bdr == me);
        let (mut dr, mut bdr) = calc(&cands);
        // step 4: this router newly is or no longer is DR or BDR: once more with its new view
        if (dr == me) != was_dr || (bdr == me) != was_bdr {
            if let Some(c) = cands.iter_mut().find(|c| c.0 == me) {
                c.3 = dr;
                c.4 = bdr;
            }
            (dr, bdr) = calc(&cands);
        }
        let changed = (dr, bdr) != (f.dr, f.bdr);
        f.dr = dr;
        f.bdr = bdr;
        let state = if dr == me {
            IfState::Dr
        } else if bdr == me {
            IfState::Backup
        } else {
            IfState::DrOther
        };
        let (dr_id, bdr_id) = (f.rid_of(dr, rid), f.rid_of(bdr, rid));
        let first = f.state == IfState::Waiting;
        self.set_if_state(os, i, state);
        if changed || first {
            os.note(Note::OspfDr { iface: i, dr: dr_id, bdr: bdr_id });
        }
        if changed {
            // step 7: AdjOK? for every neighbour from 2-Way on
            let nbrs: Vec<Ipv4> = self.ifs.get(&i).map(|f| f.nbrs.values().filter(|n| n.state >= NbrState::TwoWay).map(|n| n.addr).collect()).unwrap_or_default();
            for a in nbrs {
                self.nbr_event(os, i, a, NbrEvent::AdjOk);
            }
        }
    }

    /// should the router become adjacent with this neighbour (10.4)
    fn adj_ok(&self, i: u16, addr: Ipv4) -> bool {
        let Some(f) = self.ifs.get(&i) else {
            return false;
        };
        match f.kind {
            NetType::PointToPoint => true,
            NetType::Loopback => false,
            NetType::Broadcast => {
                let me = f.addr.addr;
                f.dr == me || f.bdr == me || f.dr == addr || f.bdr == addr
            }
        }
    }
}

// ---------------------------------------------------------------- the neighbour state machine

impl Ospf {
    fn set_nbr_state(&mut self, os: &mut OsApi, i: u16, a: Ipv4, to: NbrState, ev: NbrEvent) {
        let now = os.now();
        let Some(f) = self.ifs.get_mut(&i) else {
            return;
        };
        let Some(n) = f.nbrs.get_mut(&a) else {
            return;
        };
        let from = n.state;
        if from == to {
            return;
        }
        n.state = to;
        n.since = now;
        let id = n.id;
        // crossing 2-Way is a NeighborChange of the interface (9.2)
        if (from >= NbrState::TwoWay) != (to >= NbrState::TwoWay) {
            f.elect = true;
        }
        let ifname = f.name.clone();
        self.n.adj_changes += 1;
        os.note(Note::OspfNbr { nbr: id, iface: i, from, to, ev });
        // "log-adjacency-changes": Full and Down, with detail every change
        if self.cfg.log_adj && os.ios() && (self.cfg.log_detail || to == NbrState::Full || from == NbrState::Full || to == NbrState::Down) {
            let line = format!("%OSPF-5-ADJCHG: process {}, neighbour {} on {}: {} -> {} ({})", self.cfg.pid, id, ifname, from.word(), to.word(), ev.name());
            let ni = os.ni;
            os.eng.ios_log(ni, &line);
        }
    }

    /// an event of the neighbour state machine (10.3)
    fn nbr_event(&mut self, os: &mut OsApi, i: u16, a: Ipv4, ev: NbrEvent) {
        let now = os.now();
        let Some(st) = self.ifs.get(&i).and_then(|f| f.nbrs.get(&a)).map(|n| n.state) else {
            return;
        };
        use NbrState as S;
        match ev {
            NbrEvent::HelloReceived => {
                let dead = self.ifs.get(&i).map(|f| f.dead).unwrap_or(40) as Ns;
                if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) {
                    n.dead_at = now + dead * SEC;
                }
                if st == S::Down {
                    self.set_nbr_state(os, i, a, S::Init, ev);
                }
            }
            NbrEvent::TwoWayReceived => {
                if st == S::Init {
                    if self.adj_ok(i, a) {
                        self.set_nbr_state(os, i, a, S::ExStart, ev);
                        self.exstart(os, i, a);
                    } else {
                        self.set_nbr_state(os, i, a, S::TwoWay, ev);
                    }
                }
            }
            NbrEvent::NegotiationDone => {
                if st == S::ExStart {
                    self.set_nbr_state(os, i, a, S::Exchange, ev);
                    self.fill_summary(os, i, a);
                }
            }
            NbrEvent::ExchangeDone => {
                if st == S::Exchange {
                    let empty = self.ifs.get(&i).and_then(|f| f.nbrs.get(&a)).is_none_or(|n| n.requests.is_empty());
                    if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) {
                        n.dd_at = None;
                    }
                    if empty {
                        self.set_nbr_state(os, i, a, S::Full, ev);
                    } else {
                        self.set_nbr_state(os, i, a, S::Loading, ev);
                        self.send_lsr(os, i, a);
                    }
                }
            }
            NbrEvent::LoadingDone => {
                if st == S::Loading {
                    if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) {
                        n.lsr_at = None;
                    }
                    self.set_nbr_state(os, i, a, S::Full, ev);
                }
            }
            NbrEvent::AdjOk => {
                let ok = self.adj_ok(i, a);
                if st == S::TwoWay && ok {
                    self.set_nbr_state(os, i, a, S::ExStart, ev);
                    self.exstart(os, i, a);
                } else if st >= S::ExStart && !ok {
                    if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) {
                        n.clear_lists();
                    }
                    self.set_nbr_state(os, i, a, S::TwoWay, ev);
                }
            }
            NbrEvent::SeqNumberMismatch | NbrEvent::BadLsReq => {
                if st >= S::Exchange {
                    if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) {
                        n.clear_lists();
                    }
                    self.set_nbr_state(os, i, a, S::ExStart, ev);
                    self.exstart(os, i, a);
                }
            }
            NbrEvent::OneWay => {
                if st >= S::TwoWay {
                    if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) {
                        n.clear_lists();
                    }
                    self.set_nbr_state(os, i, a, S::Init, ev);
                }
            }
            NbrEvent::KillNbr | NbrEvent::InactivityTimer | NbrEvent::LlDown => {
                if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) {
                    n.clear_lists();
                }
                self.set_nbr_state(os, i, a, S::Down, ev);
                // a neighbour that is down is forgotten
                if let Some(f) = self.ifs.get_mut(&i) {
                    f.nbrs.remove(&a);
                    f.elect = true;
                }
            }
        }
    }

    /// entering ExStart (10.3, 10.8): a DD sequence number of its own, this router the master
    /// until the negotiation says otherwise, the first DD with I, M and MS
    fn exstart(&mut self, os: &mut OsApi, i: u16, a: Ipv4) {
        let seq = os.rand();
        let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) else {
            return;
        };
        n.dd_seq = if n.dd_seq == 0 { seq } else { n.dd_seq.wrapping_add(1) };
        n.master = false;
        n.more = true;
        n.last_rx = None;
        self.send_dd(os, i, a, true);
    }

    /// NegotiationDone: the database summary list is the area's database (and the AS-external
    /// LSAs outside stub areas); LSAs at MaxAge go on the retransmission list instead (10.3)
    fn fill_summary(&mut self, os: &mut OsApi, i: u16, a: Ipv4) {
        let now = os.now();
        let Some(area) = self.ifs.get(&i).map(|f| f.area) else {
            return;
        };
        let mut summary: VecDeque<(Scope, LsaKey)> = VecDeque::new();
        let mut aged: Vec<(LsaKey, LsaHeader)> = Vec::new();
        let mut scopes = alloc::vec![Scope::Area(area)];
        if !self.is_stub(area) {
            scopes.push(Scope::As);
        }
        for sc in scopes {
            if let Some(db) = self.db(sc) {
                for (k, e) in db {
                    if e.maxage(now) {
                        aged.push((*k, e.hdr(now)));
                    } else {
                        summary.push_back((sc, *k));
                    }
                }
            }
        }
        if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) {
            n.summary = summary;
            for (k, h) in aged {
                n.rxmt.insert(k, (h, now));
            }
        }
    }
}

// ---------------------------------------------------------------- input

impl Ospf {
    fn ignore(&mut self, os: &mut OsApi, from: Ipv4, i: u16, why: OspfDrop) {
        self.n.ignored += 1;
        os.note(Note::OspfIgnored { from, iface: i, why });
    }

    /// a packet for OSPF (8.2): checked, then by its type
    fn input(&mut self, os: &mut OsApi, p: IpPkt) {
        let i = p.iface;
        let Some(rid) = self.rid else {
            return self.ignore(os, p.src, i, OspfDrop::Disabled);
        };
        let Some((passive, state, area, kind, net)) = self.ifs.get(&i).filter(|f| f.running()).map(|f| (f.passive, f.state, f.area, f.kind, f.addr)) else {
            return self.ignore(os, p.src, i, OspfDrop::Disabled);
        };
        if passive || state == IfState::Loopback {
            return self.ignore(os, p.src, i, OspfDrop::Passive);
        }
        let Some(v) = OspfView::parse(&p.data) else {
            return self.ignore(os, p.src, i, OspfDrop::Malformed);
        };
        if v.version() != wire::VERSION {
            return self.ignore(os, p.src, i, OspfDrop::Version);
        }
        if !v.checksum_ok() {
            return self.ignore(os, p.src, i, OspfDrop::Checksum);
        }
        if v.area() != area {
            return self.ignore(os, p.src, i, OspfDrop::Area);
        }
        if v.autype() != wire::AU_NULL {
            return self.ignore(os, p.src, i, OspfDrop::Auth);
        }
        if v.router() == rid {
            return self.ignore(os, p.src, i, OspfDrop::OwnId);
        }
        if kind != NetType::PointToPoint && !net.contains(p.src) {
            return self.ignore(os, p.src, i, OspfDrop::Subnet);
        }
        if p.dst == wire::ALL_D_ROUTERS && !matches!(state, IfState::Dr | IfState::Backup) {
            return self.ignore(os, p.src, i, OspfDrop::AllDRouters);
        }
        let Ok(pkt) = Packet::read(&v) else {
            return self.ignore(os, p.src, i, OspfDrop::Malformed);
        };
        let from = pkt.hdr.router;
        match pkt.body {
            Body::Hello(h) => self.hello_in(os, i, p.src, from, h),
            body => {
                // the other packets come from a known neighbour (on point-to-point links its
                // address may be one of another subnet)
                let known = self.ifs.get(&i).and_then(|f| f.nbrs.get(&p.src)).is_some_and(|n| n.id == from);
                if !known {
                    return self.ignore(os, p.src, i, OspfDrop::State);
                }
                match body {
                    Body::Dd(d) => self.dd_in(os, i, p.src, d),
                    Body::Lsr(keys) => self.lsr_in(os, i, p.src, keys),
                    Body::Lsu(lsas) => self.lsu_in(os, i, p.src, lsas),
                    Body::Ack(hs) => self.ack_in(os, i, p.src, hs),
                    Body::Hello(_) => {}
                }
            }
        }
    }

    /// a Hello (10.5)
    fn hello_in(&mut self, os: &mut OsApi, i: u16, src: Ipv4, rid: Ipv4, h: Hello) {
        let Some(me) = self.rid else {
            return;
        };
        let Some((area, kind, mask, hello, dead, state)) = self.ifs.get(&i).map(|f| (f.area, f.kind, f.addr.mask(), f.hello, f.dead, f.state)) else {
            return;
        };
        let stub = self.is_stub(area);
        if kind == NetType::Broadcast && h.mask != mask {
            return self.ignore(os, src, i, OspfDrop::Mask);
        }
        if h.interval != hello {
            return self.ignore(os, src, i, OspfDrop::HelloTime);
        }
        if h.dead != dead {
            return self.ignore(os, src, i, OspfDrop::DeadTime);
        }
        if (h.options & wire::OPT_E != 0) == stub {
            return self.ignore(os, src, i, OspfDrop::Stub);
        }
        self.n.hellos_rcvd += 1;
        os.note(Note::OspfHelloRcvd { from: rid, iface: i });
        let now = os.now();
        let waiting = state == IfState::Waiting;
        let bcast = kind == NetType::Broadcast;
        let Some(f) = self.ifs.get_mut(&i) else {
            return;
        };
        // a neighbour at this address with another router ID starts again
        if f.nbrs.get(&src).is_some_and(|n| n.id != rid) {
            f.nbrs.remove(&src);
            f.elect = true;
        }
        let n = f.nbrs.entry(src).or_insert_with(|| Nbr::new(rid, src, now));
        let (old_prio, old_dr, old_bdr) = (n.priority, n.dr, n.bdr);
        let fresh = n.state == NbrState::Down;
        n.priority = h.priority;
        n.dr = h.dr;
        n.bdr = h.bdr;
        n.options = h.options;
        self.nbr_event(os, i, src, NbrEvent::HelloReceived);
        if h.neighbors.contains(&me) {
            self.nbr_event(os, i, src, NbrEvent::TwoWayReceived);
        } else {
            self.nbr_event(os, i, src, NbrEvent::OneWay);
            return;
        }
        if !bcast {
            return;
        }
        let Some(f) = self.ifs.get_mut(&i) else {
            return;
        };
        let (decl_dr, decl_bdr) = (h.dr == src, h.bdr == src);
        let (was_dr, was_bdr) = (!fresh && old_dr == src, !fresh && old_bdr == src);
        if !fresh && old_prio != h.priority {
            f.elect = true;
        }
        if decl_dr && h.bdr.is_unspecified() && waiting {
            f.seen = true;
        } else if decl_dr != was_dr {
            f.elect = true;
        }
        if decl_bdr && waiting {
            f.seen = true;
        } else if decl_bdr != was_bdr {
            f.elect = true;
        }
    }

    /// a Database Description packet (10.6)
    fn dd_in(&mut self, os: &mut OsApi, i: u16, src: Ipv4, d: Dd) {
        let Some(me) = self.rid else {
            return;
        };
        let Some((mtu, nid, master, mut st)) = self.ifs.get(&i).and_then(|f| f.nbrs.get(&src).map(|n| (f.mtu, n.id, n.master, n.state))) else {
            return;
        };
        self.n.dd_rcvd += 1;
        os.note(Note::OspfDd { nbr: nid, iface: i, out: false, flags: d.flags, seq: d.seq, lsas: d.lsas.len() as u16 });
        if d.mtu > mtu {
            return self.ignore(os, src, i, OspfDrop::Mtu);
        }
        if st == NbrState::Init {
            self.nbr_event(os, i, src, NbrEvent::TwoWayReceived);
            st = self.ifs.get(&i).and_then(|f| f.nbrs.get(&src)).map(|n| n.state).unwrap_or(NbrState::Down);
        }
        match st {
            NbrState::Down | NbrState::Attempt | NbrState::Init | NbrState::TwoWay => self.ignore(os, src, i, OspfDrop::State),
            NbrState::ExStart => {
                let init = d.flags & (wire::DD_I | wire::DD_M | wire::DD_MS) == wire::DD_I | wire::DD_M | wire::DD_MS;
                let our_seq = self.ifs.get(&i).and_then(|f| f.nbrs.get(&src)).map(|n| n.dd_seq).unwrap_or(0);
                if init && d.lsas.is_empty() && nid > me {
                    // the neighbour is master: take its sequence number and answer as slave
                    if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&src)) {
                        n.master = true;
                        n.dd_seq = d.seq;
                        n.options = d.options;
                        n.dd_at = None;
                        n.last_rx = Some((d.flags, d.options, d.seq));
                    }
                    os.note(Note::OspfMaster { nbr: nid, iface: i, master: false });
                    self.nbr_event(os, i, src, NbrEvent::NegotiationDone);
                    self.send_dd(os, i, src, false);
                    self.slave_done(os, i, src, d.flags);
                } else if d.flags & (wire::DD_I | wire::DD_MS) == 0 && d.seq == our_seq && nid < me {
                    // this router is master: the slave answered with its first headers
                    if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&src)) {
                        n.master = false;
                        n.options = d.options;
                    }
                    os.note(Note::OspfMaster { nbr: nid, iface: i, master: true });
                    self.nbr_event(os, i, src, NbrEvent::NegotiationDone);
                    self.master_takes(os, i, src, d);
                }
                // anything else waits for the negotiation
            }
            NbrState::Exchange => {
                let Some(n) = self.ifs.get(&i).and_then(|f| f.nbrs.get(&src)) else {
                    return;
                };
                if n.last_rx == Some((d.flags, d.options, d.seq)) {
                    // a duplicate: the master drops it, the slave repeats its last DD
                    if master {
                        self.resend_dd(os, i, src);
                    } else {
                        self.ignore(os, src, i, OspfDrop::Duplicate);
                    }
                    return;
                }
                let bad = (d.flags & wire::DD_MS != 0) != master || d.flags & wire::DD_I != 0 || n.last_rx.is_some_and(|x| x.1 != d.options);
                let expect = if master { n.dd_seq.wrapping_add(1) } else { n.dd_seq };
                if bad || d.seq != expect {
                    return self.nbr_event(os, i, src, NbrEvent::SeqNumberMismatch);
                }
                if master {
                    // this router is slave: take the headers, answer with the next ones
                    if !self.take_headers(os, i, src, &d) {
                        return;
                    }
                    if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&src)) {
                        n.dd_seq = d.seq;
                        n.last_rx = Some((d.flags, d.options, d.seq));
                    }
                    self.send_dd(os, i, src, false);
                    self.slave_done(os, i, src, d.flags);
                } else {
                    self.master_takes(os, i, src, d);
                }
            }
            NbrState::Loading | NbrState::Full => {
                let Some(n) = self.ifs.get(&i).and_then(|f| f.nbrs.get(&src)) else {
                    return;
                };
                if n.last_rx == Some((d.flags, d.options, d.seq)) {
                    if master {
                        self.resend_dd(os, i, src);
                    } else {
                        self.ignore(os, src, i, OspfDrop::Duplicate);
                    }
                } else {
                    self.nbr_event(os, i, src, NbrEvent::SeqNumberMismatch);
                }
            }
        }
    }

    /// the slave after answering: the master's DD had no M bit, and neither had the answer
    fn slave_done(&mut self, os: &mut OsApi, i: u16, src: Ipv4, flags: u8) {
        let more = self.ifs.get(&i).and_then(|f| f.nbrs.get(&src)).map(|n| n.more).unwrap_or(true);
        let st = self.ifs.get(&i).and_then(|f| f.nbrs.get(&src)).map(|n| n.state);
        if st == Some(NbrState::Exchange) && flags & wire::DD_M == 0 && !more {
            self.nbr_event(os, i, src, NbrEvent::ExchangeDone);
        }
    }

    /// the master takes an answer of the slave: the headers, then the next DD or the end
    fn master_takes(&mut self, os: &mut OsApi, i: u16, src: Ipv4, d: Dd) {
        if !self.take_headers(os, i, src, &d) {
            return;
        }
        let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&src)) else {
            return;
        };
        n.last_rx = Some((d.flags, d.options, d.seq));
        n.dd_at = None;
        if !n.more && d.flags & wire::DD_M == 0 {
            self.nbr_event(os, i, src, NbrEvent::ExchangeDone);
        } else {
            n.dd_seq = n.dd_seq.wrapping_add(1);
            self.send_dd(os, i, src, false);
        }
    }

    /// the LSA headers of a DD: unknown types end the exchange, newer ones go on the request
    /// list; false when the exchange restarted
    fn take_headers(&mut self, os: &mut OsApi, i: u16, src: Ipv4, d: &Dd) -> bool {
        let now = os.now();
        let Some(area) = self.ifs.get(&i).map(|f| f.area) else {
            return false;
        };
        let stub = self.is_stub(area);
        let mut want: Vec<(LsaKey, LsaHeader)> = Vec::new();
        for h in &d.lsas {
            if !(wire::LSA_ROUTER..=wire::LSA_EXTERNAL).contains(&h.kind) || (h.kind == wire::LSA_EXTERNAL && stub) {
                self.nbr_event(os, i, src, NbrEvent::SeqNumberMismatch);
                return false;
            }
            let sc = scope_of(h.kind, area);
            let newer = match self.entry(sc, &h.key()) {
                None => true,
                Some(e) => wire::compare(h, &e.hdr(now)) == Ordering::Greater,
            };
            if newer {
                want.push((h.key(), *h));
            }
        }
        if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&src)) {
            for (k, h) in want {
                n.requests.insert(k, h);
            }
        }
        true
    }

    /// a Link State Request (10.7): the LSAs asked for in updates to the neighbour
    fn lsr_in(&mut self, os: &mut OsApi, i: u16, src: Ipv4, keys: Vec<LsaKey>) {
        let now = os.now();
        let Some(f) = self.ifs.get(&i) else {
            return;
        };
        let area = f.area;
        let Some(n) = f.nbrs.get(&src) else {
            return;
        };
        let nid = n.id;
        if n.state < NbrState::Exchange {
            return self.ignore(os, src, i, OspfDrop::State);
        }
        self.n.lsr_rcvd += 1;
        os.note(Note::OspfLsr { nbr: nid, iface: i, out: false, lsas: keys.len() as u16 });
        let mut out: Vec<Vec<u8>> = Vec::new();
        for k in &keys {
            match self.entry(scope_of(k.kind, area), k) {
                Some(e) => out.push(e.wire(now)),
                None => return self.nbr_event(os, i, src, NbrEvent::BadLsReq),
            }
        }
        self.send_lsu(os, i, Some(src), out, LsuWhy::Answer);
    }

    /// a Link State Update (13)
    fn lsu_in(&mut self, os: &mut OsApi, i: u16, src: Ipv4, lsas: Vec<Vec<u8>>) {
        let now = os.now();
        let Some(f) = self.ifs.get(&i) else {
            return;
        };
        let area = f.area;
        let Some(n) = f.nbrs.get(&src) else {
            return;
        };
        let nid = n.id;
        if n.state < NbrState::Exchange {
            return self.ignore(os, src, i, OspfDrop::State);
        }
        self.n.lsu_rcvd += 1;
        os.note(Note::OspfLsu { nbr: Some(nid), iface: i, out: false, lsas: lsas.len() as u16, why: LsuWhy::Flood });
        let stub = self.is_stub(area);
        for raw in lsas {
            let Some(h) = LsaHeader::of(&raw) else {
                continue;
            };
            let r = LsaRef::of(&h, scope_of(h.kind, area));
            // (1) the checksum, (2) the type, (3) no AS-external-LSA in a stub area
            if !wire::lsa_checksum_ok(&raw) {
                os.note(Note::OspfLsaDropped { lsa: r, why: LsaDrop::Checksum });
                continue;
            }
            if !(wire::LSA_ROUTER..=wire::LSA_EXTERNAL).contains(&h.kind) {
                os.note(Note::OspfLsaDropped { lsa: r, why: LsaDrop::Type });
                continue;
            }
            if h.kind == wire::LSA_EXTERNAL && stub {
                os.note(Note::OspfLsaDropped { lsa: r, why: LsaDrop::StubExternal });
                continue;
            }
            let Ok(lsa) = Lsa::parse(&raw) else {
                os.note(Note::OspfLsaDropped { lsa: r, why: LsaDrop::Checksum });
                continue;
            };
            let k = h.key();
            let sc = scope_of(h.kind, area);
            let cur = self.entry(sc, &k).map(|e| (e.hdr(now), e.at, e.from.is_some()));
            // (4) MaxAge and nothing to compare with: acknowledged and dropped
            if h.age >= wire::MAX_AGE && cur.is_none() && !self.exchanging() {
                self.direct_ack(i, src, h);
                continue;
            }
            let newer = cur.is_none_or(|c| wire::compare(&h, &c.0) == Ordering::Greater);
            if newer {
                // (5a) MinLSArrival since the last one came by flooding
                if cur.is_some_and(|c| c.2 && now < c.1 + wire::MIN_LS_ARRIVAL as Ns * SEC) {
                    os.note(Note::OspfLsaDropped { lsa: r, why: LsaDrop::MinArrival });
                    continue;
                }
                // (5b..d) installed (the old instance leaves the retransmission lists), flooded
                self.install(os, sc, raw, lsa, Some(nid));
                os.note(Note::OspfLsaNew { lsa: r, from: nid });
                let back = self.flood(os, sc, k, Some((i, src)));
                // (5e) acknowledged unless it went back out of this interface (Table 19)
                if !back
                    && let Some(f) = self.ifs.get(&i)
                    && (f.state != IfState::Backup || src == f.dr)
                {
                    self.delayed_ack(os, i, h);
                }
                continue;
            }
            // (6) an instance it asked for that is not newer: the exchange went wrong
            if self.ifs.get(&i).and_then(|f| f.nbrs.get(&src)).is_some_and(|n| n.requests.contains_key(&k)) {
                return self.nbr_event(os, i, src, NbrEvent::BadLsReq);
            }
            let Some((cur_h, _, _)) = cur else {
                continue;
            };
            if wire::compare(&h, &cur_h) == Ordering::Equal {
                // (7) a duplicate: an implied acknowledgement where it waits, else a direct one
                let implied = self.ifs.get(&i).and_then(|f| f.nbrs.get(&src)).and_then(|n| n.rxmt.get(&k)).is_some_and(|x| wire::compare(&h, &x.0) == Ordering::Equal);
                if implied {
                    if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&src)) {
                        n.rxmt.remove(&k);
                    }
                    os.note(Note::OspfLsaAcked { lsa: r, nbr: nid });
                    if self.ifs.get(&i).is_some_and(|f| f.state == IfState::Backup && src == f.dr) {
                        self.delayed_ack(os, i, h);
                    }
                } else {
                    self.direct_ack(i, src, h);
                }
                continue;
            }
            // (8) the database copy is newer: it goes back to the neighbour (not more often than
            // MinLSArrival)
            if cur_h.age >= wire::MAX_AGE && cur_h.seq == wire::MAX_SEQ {
                continue;
            }
            if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&src)) {
                let recent = n.sent_back.get(&k).is_some_and(|&t| now < t + wire::MIN_LS_ARRIVAL as Ns * SEC);
                if !recent {
                    n.sent_back.insert(k, now);
                    n.back.push(k);
                }
            }
            os.note(Note::OspfLsaOlder { lsa: r, nbr: nid });
        }
        self.after_requests(os, i, src);
    }

    /// what Loading waits for came: the next request, or Full
    fn after_requests(&mut self, os: &mut OsApi, i: u16, src: Ipv4) {
        let Some(n) = self.ifs.get(&i).and_then(|f| f.nbrs.get(&src)) else {
            return;
        };
        if n.state != NbrState::Loading {
            return;
        }
        let answered = !n.asked.iter().any(|k| n.requests.contains_key(k));
        if n.requests.is_empty() {
            self.nbr_event(os, i, src, NbrEvent::LoadingDone);
        } else if answered {
            self.send_lsr(os, i, src);
        }
    }

    /// a Link State Acknowledgment (13.7)
    fn ack_in(&mut self, os: &mut OsApi, i: u16, src: Ipv4, hs: Vec<LsaHeader>) {
        let Some((area, n)) = self.ifs.get(&i).and_then(|f| f.nbrs.get(&src).map(|n| (f.area, n))) else {
            return;
        };
        let nid = n.id;
        if n.state < NbrState::Exchange {
            return self.ignore(os, src, i, OspfDrop::State);
        }
        self.n.acks_rcvd += 1;
        os.note(Note::OspfAck { nbr: Some(nid), iface: i, out: false, lsas: hs.len() as u16, delayed: false });
        for h in hs {
            let k = h.key();
            let hit = self.ifs.get(&i).and_then(|f| f.nbrs.get(&src)).and_then(|n| n.rxmt.get(&k)).is_some_and(|x| wire::compare(&h, &x.0) == Ordering::Equal);
            if hit {
                if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&src)) {
                    n.rxmt.remove(&k);
                }
                os.note(Note::OspfLsaAcked { lsa: LsaRef::of(&h, scope_of(h.kind, area)), nbr: nid });
            }
        }
    }
}

// ---------------------------------------------------------------- the database and flooding

impl Ospf {
    fn schedule_spf(&mut self, now: Ns) {
        if self.spf_at.is_none() {
            self.spf_at = Some(now + SPF_DELAY);
        }
    }

    /// a new instance goes into the database; the old one leaves every retransmission list
    /// (13, step 5c); a change of its contents (13.2) calls for SPF
    fn install(&mut self, os: &mut OsApi, sc: Scope, raw: Vec<u8>, lsa: Lsa, from: Option<Ipv4>) {
        let now = os.now();
        let k = lsa.key();
        let stubs = self.stub_areas();
        for f in self.ifs.values_mut().filter(|f| in_scope(f, sc, &stubs)) {
            for n in f.nbrs.values_mut() {
                n.rxmt.remove(&k);
            }
        }
        let age = lsa.hdr.age;
        let old = self.db_mut(sc).remove(&k);
        let changed = match &old {
            None => true,
            Some(e) => e.lsa.hdr.options != lsa.hdr.options || e.maxage(now) != (age >= wire::MAX_AGE) || e.lsa.hdr.len != lsa.hdr.len || e.lsa.body != lsa.body,
        };
        self.db_mut(sc).insert(k, Entry { raw, lsa, at: now, age0: age, from, aged: age >= wire::MAX_AGE });
        if changed {
            self.schedule_spf(now);
        }
    }

    /// floods the database copy of an LSA (13.3); `from`: the interface and neighbour it came
    /// from; true when it goes back out of the receiving interface
    fn flood(&mut self, os: &mut OsApi, sc: Scope, k: LsaKey, from: Option<(u16, Ipv4)>) -> bool {
        let now = os.now();
        let Some(hdr) = self.entry(sc, &k).map(|e| e.hdr(now)) else {
            return false;
        };
        let stubs = self.stub_areas();
        let mut back = false;
        let mut out = 0u16;
        for (&i, f) in self.ifs.iter_mut() {
            if !f.speaks() || !in_scope(f, sc, &stubs) {
                continue;
            }
            let mut added = false;
            for (addr, n) in f.nbrs.iter_mut() {
                // (1a) not yet exchanging
                if n.state < NbrState::Exchange {
                    continue;
                }
                // (1b) what Exchange and Loading still ask for
                if n.state != NbrState::Full
                    && let Some(req) = n.requests.get(&k).copied()
                {
                    match wire::compare(&hdr, &req) {
                        Ordering::Less => continue,
                        Ordering::Equal => {
                            n.requests.remove(&k);
                            continue;
                        }
                        Ordering::Greater => {
                            n.requests.remove(&k);
                        }
                    }
                }
                // (1c) not back to the sender
                if from == Some((i, *addr)) {
                    continue;
                }
                // (1d) on the retransmission list
                n.rxmt.insert(k, (hdr, now));
                added = true;
            }
            if !added {
                continue;
            }
            if let Some((fi, fa)) = from
                && fi == i
            {
                // (3, 4) it came in here from the DR or BDR, or this router is the BDR: the DR
                // floods it
                if fa == f.dr || fa == f.bdr || f.state == IfState::Backup {
                    continue;
                }
                back = true;
            }
            f.flood_q.push((sc, k));
            out += 1;
        }
        if out > 0 {
            os.note(Note::OspfLsaFlooded { lsa: LsaRef::of(&hdr, sc), ifaces: out });
        }
        back
    }

    fn delayed_ack(&mut self, os: &mut OsApi, i: u16, h: LsaHeader) {
        let now = os.now();
        if let Some(f) = self.ifs.get_mut(&i) {
            f.delayed.push(h);
            if f.ack_at.is_none() {
                f.ack_at = Some(now + ACK_DELAY);
            }
        }
    }

    fn direct_ack(&mut self, i: u16, src: Ipv4, h: LsaHeader) {
        if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&src)) {
            n.direct.push(h);
        }
    }

    /// what waits for the end of the event: floods per interface, direct acknowledgements and
    /// database copies per neighbour
    fn send_queues(&mut self, os: &mut OsApi) {
        let now = os.now();
        let ifs: Vec<u16> = self.ifs.keys().copied().collect();
        for i in ifs {
            let q = self.ifs.get_mut(&i).map(|f| core::mem::take(&mut f.flood_q)).unwrap_or_default();
            if !q.is_empty() {
                let mut lsas: Vec<Vec<u8>> = Vec::new();
                for (n, (sc, k)) in q.iter().enumerate() {
                    // an LSA flooded twice in one event goes once, as it is now
                    if q[..n].contains(&(*sc, *k)) {
                        continue;
                    }
                    if let Some(e) = self.entry(*sc, k) {
                        lsas.push(e.wire(now));
                    }
                }
                self.send_lsu(os, i, None, lsas, LsuWhy::Flood);
            }
            let nbrs: Vec<Ipv4> = self.ifs.get(&i).map(|f| f.nbrs.keys().copied().collect()).unwrap_or_default();
            let area = self.ifs.get(&i).map(|f| f.area).unwrap_or(0);
            for a in nbrs {
                let (acks, back) = match self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) {
                    Some(n) => (core::mem::take(&mut n.direct), core::mem::take(&mut n.back)),
                    None => continue,
                };
                if !back.is_empty() {
                    let lsas: Vec<Vec<u8>> = back.iter().filter_map(|k| self.entry(scope_of(k.kind, area), k).map(|e| e.wire(now))).collect();
                    self.send_lsu(os, i, Some(a), lsas, LsuWhy::Back);
                }
                if !acks.is_empty() {
                    self.send_ack(os, i, Some(a), acks);
                }
            }
        }
    }

    /// LSAs at MaxAge leave the database once no retransmission list holds them and nobody
    /// exchanges databases (14)
    fn maxage_sweep(&mut self, os: &mut OsApi) {
        let now = os.now();
        if self.exchanging() {
            return;
        }
        let stubs = self.stub_areas();
        let listed = |me: &Ospf, sc: Scope, k: &LsaKey| me.ifs.values().filter(|f| in_scope(f, sc, &stubs)).any(|f| f.nbrs.values().any(|n| n.rxmt.contains_key(k)));
        let mut gone: Vec<(Scope, LsaKey, LsaHeader)> = Vec::new();
        for (a, db) in &self.lsdb {
            for (k, e) in db {
                if e.maxage(now) && !listed(self, Scope::Area(*a), k) {
                    gone.push((Scope::Area(*a), *k, e.hdr(now)));
                }
            }
        }
        for (k, e) in &self.ext {
            if e.maxage(now) && !listed(self, Scope::As, k) {
                gone.push((Scope::As, *k, e.hdr(now)));
            }
        }
        for (sc, k, h) in gone {
            self.db_mut(sc).remove(&k);
            os.note(Note::OspfLsaGone { lsa: LsaRef::of(&h, sc) });
        }
    }
}

// ---------------------------------------------------------------- origination

impl Ospf {
    /// the router-LSA of an area (12.4.1)
    fn router_lsa(&self, area: u32) -> LsaBody {
        let mut links: Vec<wire::RouterLink> = Vec::new();
        let stub = |p: Prefix4, cost: u16| wire::RouterLink { id: p.network(), data: p.mask(), kind: wire::LINK_STUB, metric: cost, tos: Vec::new() };
        for f in self.ifs.values().filter(|f| f.area == area) {
            match f.state {
                IfState::Down => continue,
                IfState::Loopback => {
                    links.push(wire::RouterLink { id: f.addr.addr, data: Ipv4::BROADCAST, kind: wire::LINK_STUB, metric: 0, tos: Vec::new() });
                    for p in &f.secondaries {
                        links.push(wire::RouterLink { id: p.addr, data: Ipv4::BROADCAST, kind: wire::LINK_STUB, metric: 0, tos: Vec::new() });
                    }
                    continue;
                }
                IfState::PointToPoint => {
                    for n in f.nbrs.values().filter(|n| n.state == NbrState::Full) {
                        links.push(wire::RouterLink { id: n.id, data: f.addr.addr, kind: wire::LINK_P2P, metric: f.cost, tos: Vec::new() });
                    }
                    links.push(stub(f.addr, f.cost));
                }
                IfState::Waiting => links.push(stub(f.addr, f.cost)),
                IfState::DrOther | IfState::Backup | IfState::Dr => {
                    let me = f.addr.addr;
                    let full_dr = f.nbrs.get(&f.dr).is_some_and(|n| n.state == NbrState::Full);
                    let dr_full = f.dr == me && f.nbrs.values().any(|n| n.state == NbrState::Full);
                    if full_dr || dr_full {
                        links.push(wire::RouterLink { id: f.dr, data: me, kind: wire::LINK_TRANSIT, metric: f.cost, tos: Vec::new() });
                    } else {
                        links.push(stub(f.addr, f.cost));
                    }
                }
            }
            for p in &f.secondaries {
                links.push(stub(*p, f.cost));
            }
        }
        let mut flags = 0;
        if self.is_abr() {
            flags |= wire::RTR_B;
        }
        if self.cfg.asbr() && !self.is_stub(area) {
            flags |= wire::RTR_E;
        }
        LsaBody::Router(wire::RouterLsa { flags, links })
    }

    /// every LSA the router wants to originate now: router- and network-LSAs, summaries as
    /// ABR, AS-external-LSAs as ASBR
    fn wanted(&self, os: &OsApi) -> BTreeMap<(Scope, LsaKey), (u8, LsaBody)> {
        let mut w: BTreeMap<(Scope, LsaKey), (u8, LsaBody)> = BTreeMap::new();
        let Some(rid) = self.rid else {
            return w;
        };
        let attached = self.attached();
        for &area in &attached {
            w.insert((Scope::Area(area), LsaKey { kind: wire::LSA_ROUTER, id: rid, adv: rid }), (self.options(area), self.router_lsa(area)));
        }
        // network-LSAs where it is DR with a Full neighbour (12.4.2)
        for f in self.ifs.values() {
            if f.state == IfState::Dr && !f.passive {
                let full: Vec<Ipv4> = f.nbrs.values().filter(|n| n.state == NbrState::Full).map(|n| n.id).collect();
                if !full.is_empty() {
                    let mut routers = alloc::vec![rid];
                    routers.extend(full);
                    w.insert((Scope::Area(f.area), LsaKey { kind: wire::LSA_NETWORK, id: f.addr.addr, adv: rid }), (self.options(f.area), LsaBody::Network(wire::NetworkLsa { mask: f.addr.mask(), routers })));
                }
            }
        }
        // summary-LSAs of an ABR (12.4.3)
        if attached.len() > 1 {
            for &into in &attached {
                let stub = self.cfg.stubs.get(&into).copied().filter(|_| into != 0);
                if let Some(no_summary) = stub {
                    let body = LsaBody::Summary(wire::SummaryLsa { mask: Ipv4::UNSPECIFIED, metric: STUB_COST, tos: Vec::new() });
                    w.insert((Scope::Area(into), LsaKey { kind: wire::LSA_SUMMARY, id: Ipv4::UNSPECIFIED, adv: rid }), (0, body));
                    if no_summary {
                        continue;
                    }
                }
                for (p, r) in &self.rt {
                    if r.area == into || !matches!(r.kind, RtKind::Intra | RtKind::Inter) || r.cost >= wire::LS_INFINITY {
                        continue;
                    }
                    // only intra-area routes go into the backbone
                    if r.kind == RtKind::Inter && into == 0 {
                        continue;
                    }
                    if p.len == 0 && stub.is_some() {
                        continue;
                    }
                    let body = LsaBody::Summary(wire::SummaryLsa { mask: p.mask(), metric: r.cost, tos: Vec::new() });
                    w.insert((Scope::Area(into), LsaKey { kind: wire::LSA_SUMMARY, id: p.network(), adv: rid }), (self.options(into), body));
                }
                if stub.is_some() {
                    continue;
                }
                // the AS boundary routers of the other areas (type 4)
                let mut asbrs: BTreeMap<Ipv4, u32> = BTreeMap::new();
                for ((a, id), b) in &self.brt {
                    if *a == into || b.flags & wire::RTR_E == 0 || *id == rid || (b.inter && into == 0) {
                        continue;
                    }
                    let c = asbrs.entry(*id).or_insert(b.cost);
                    *c = (*c).min(b.cost);
                }
                for (id, cost) in asbrs {
                    let body = LsaBody::Summary(wire::SummaryLsa { mask: Ipv4::UNSPECIFIED, metric: cost, tos: Vec::new() });
                    w.insert((Scope::Area(into), LsaKey { kind: wire::LSA_ASBR, id, adv: rid }), (self.options(into), body));
                }
            }
        }
        // AS-external-LSAs of an ASBR (12.4.4), only with an area that takes them
        if self.cfg.asbr() && attached.iter().any(|a| !self.is_stub(*a)) {
            for (p, (metric, e2)) in self.external_routes(os) {
                let body = LsaBody::External(wire::ExternalLsa { mask: p.mask(), e2, metric, fwd: Ipv4::UNSPECIFIED, tag: 0, tos: Vec::new() });
                w.insert((Scope::As, LsaKey { kind: wire::LSA_EXTERNAL, id: p.network(), adv: rid }), (wire::OPT_E, body));
            }
        }
        w
    }

    /// what redistribution and the default route bring in: destination -> (metric, type 2)
    fn external_routes(&self, os: &OsApi) -> BTreeMap<Prefix4, (u32, bool)> {
        let mut out: BTreeMap<Prefix4, (u32, bool)> = BTreeMap::new();
        let ni = os.ni;
        let Some(st) = os.eng.stack(ni) else {
            return out;
        };
        let rib = st.rib(&|i| os.eng.iface_up(ni, i));
        let best = |p: Prefix4| rib.iter().filter(|x| x.prefix == p).map(|x| x.ad).min();
        for r in &self.cfg.redistribute {
            let metric = r.metric.unwrap_or(EXT_METRIC).min(wire::LS_INFINITY - 1);
            let mut take = |p: Prefix4| {
                let p = p.trunc();
                if p.len == 0 || (!r.subnets && p.len != classful_len(p.addr)) {
                    return;
                }
                out.entry(p).or_insert((metric, !r.e1));
            };
            match r.what {
                Source::Connected => {
                    for (i, f) in os.ifaces().iter().enumerate() {
                        let i = i as u16;
                        if f.hidden || !os.iface_up(i) {
                            continue;
                        }
                        let ospf = self.ifs.get(&i).is_some_and(|x| x.running());
                        for p in &f.addrs {
                            if p.addr.is_loopback() || (ospf && (*p == f.addrs[0] || self.ifs.get(&i).is_some_and(|x| x.secondaries.contains(p)))) {
                                continue;
                            }
                            take(*p);
                        }
                    }
                }
                Source::Static | Source::Rip => {
                    let src = if r.what == Source::Static { RouteSrc::Static } else { RouteSrc::Rip };
                    for x in rib.iter().filter(|x| x.src == src) {
                        if best(x.prefix) == Some(x.ad) {
                            take(x.prefix);
                        }
                    }
                }
            }
        }
        if let Some(d) = self.cfg.default_orig {
            let dflt = Prefix4 { addr: Ipv4::UNSPECIFIED, len: 0 };
            let have = rib.iter().any(|x| x.prefix.len == 0 && x.src != RouteSrc::Ospf);
            if d.always || have {
                out.insert(dflt, (d.metric.unwrap_or(DEFAULT_METRIC).min(wire::LS_INFINITY - 1), !d.e1));
            }
        }
        out
    }

    /// the LSAs of its own as the router wants them: new instances where they differ (after
    /// MinLSInterval), flushes where it wants none (14.1)
    fn reconcile(&mut self, os: &mut OsApi) {
        let Some(_) = self.rid else {
            return;
        };
        let now = os.now();
        let want = self.wanted(os);
        for ((sc, k), (opts, body)) in &want {
            let cur = self.entry(*sc, k);
            let same = cur.is_some_and(|e| e.from.is_none() && !e.maxage(now) && e.lsa.hdr.options == *opts && e.lsa.body == *body);
            if same {
                continue;
            }
            let why = match cur {
                None => OrigWhy::New,
                Some(e) if e.from.is_some() => OrigWhy::Reclaim,
                Some(e) if e.maxage(now) => OrigWhy::New,
                Some(_) => OrigWhy::Changed,
            };
            let o = self.own.entry((*sc, *k)).or_default();
            if let Some(last) = o.last
                && now < last + wire::MIN_LS_INTERVAL as Ns * SEC
            {
                o.due = Some(last + wire::MIN_LS_INTERVAL as Ns * SEC);
                continue;
            }
            self.originate(os, *sc, *k, *opts, body.clone(), why);
        }
        // own LSAs nobody wants: flushed
        let mut flush: Vec<(Scope, LsaKey)> = Vec::new();
        for (a, db) in &self.lsdb {
            for (k, e) in db {
                if self.is_own(k) && !e.maxage(now) && !want.contains_key(&(Scope::Area(*a), *k)) {
                    flush.push((Scope::Area(*a), *k));
                }
            }
        }
        for (k, e) in &self.ext {
            if self.is_own(k) && !e.maxage(now) && !want.contains_key(&(Scope::As, *k)) {
                flush.push((Scope::As, *k));
            }
        }
        for (sc, k) in flush {
            self.flush(os, sc, k);
        }
        // a pending instance that is not needed any more stops waiting
        for (key, o) in self.own.iter_mut() {
            if o.due.is_some() && !want.contains_key(key) {
                o.due = None;
            }
        }
    }

    /// a new instance of an own LSA: one sequence number after the last one (13.4)
    fn originate(&mut self, os: &mut OsApi, sc: Scope, k: LsaKey, opts: u8, body: LsaBody, why: OrigWhy) {
        let now = os.now();
        let cur = self.entry(sc, &k).map(|e| e.lsa.hdr.seq);
        let o = self.own.get(&(sc, k)).copied().unwrap_or_default();
        let last = match (cur, o.last.map(|_| o.seq)) {
            (Some(a), Some(b)) => Some(if seq_cmp(a, b) == Ordering::Greater { a } else { b }),
            (a, b) => a.or(b),
        };
        let seq = match last {
            None => wire::INITIAL_SEQ,
            Some(s) => s.wrapping_add(1),
        };
        let lsa = Lsa::new(k.kind, k.id, k.adv, seq, opts, body);
        let raw = lsa.to_bytes();
        let Ok(parsed) = Lsa::parse(&raw) else {
            return;
        };
        self.own.insert((sc, k), Own { last: Some(now), seq, due: None });
        self.n.originated += 1;
        let r = LsaRef::of(&parsed.hdr, sc);
        self.install(os, sc, raw, parsed, None);
        os.note(Note::OspfLsaOrig { lsa: r, why });
        self.flood(os, sc, k, None);
    }

    /// premature aging of an own LSA (14.1): MaxAge, flooded, removed when acknowledged
    fn flush(&mut self, os: &mut OsApi, sc: Scope, k: LsaKey) {
        let Some(e) = self.entry(sc, &k) else {
            return;
        };
        let mut raw = e.raw.clone();
        let mut lsa = e.lsa.clone();
        let from = e.from;
        wire::set_age(&mut raw, wire::MAX_AGE);
        lsa.hdr.age = wire::MAX_AGE;
        let r = LsaRef::of(&lsa.hdr, sc);
        self.install(os, sc, raw, lsa, from);
        os.note(Note::OspfLsaFlush { lsa: r });
        self.flood(os, sc, k, None);
    }
}

// ---------------------------------------------------------------- the routing table (16)

/// a vertex of the shortest-path tree; networks come before routers at equal cost (16.1)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Vertex {
    Net(Ipv4),
    Rtr(Ipv4),
}

type Hops = Vec<(Ipv4, u16)>;

fn merge_hops(a: &mut Hops, b: &Hops) {
    for h in b {
        if !a.contains(h) {
            a.push(*h);
        }
    }
    a.sort();
}

impl Ospf {
    /// the interface with this address
    fn if_with(&self, a: Ipv4) -> Option<u16> {
        self.ifs.iter().find(|(_, f)| f.running() && f.addr.addr == a).map(|(i, _)| *i)
    }

    /// Dijkstra over the router- and network-LSAs of an area (16.1)
    fn spf_area(&self, now: Ns, area: u32, rt: &mut BTreeMap<Prefix4, ORoute>, brt: &mut BTreeMap<(u32, Ipv4), BRoute>) {
        let Some(rid) = self.rid else {
            return;
        };
        let Some(db) = self.lsdb.get(&area) else {
            return;
        };
        let rtr = |id: Ipv4| -> Option<&wire::RouterLsa> {
            let e = db.get(&LsaKey { kind: wire::LSA_ROUTER, id, adv: id })?;
            if e.maxage(now) {
                return None;
            }
            match &e.lsa.body {
                LsaBody::Router(r) => Some(r),
                _ => None,
            }
        };
        let net = |id: Ipv4| -> Option<&wire::NetworkLsa> {
            db.iter().filter(|(k, e)| k.kind == wire::LSA_NETWORK && k.id == id && !e.maxage(now)).find_map(|(_, e)| match &e.lsa.body {
                LsaBody::Network(n) => Some(n),
                _ => None,
            })
        };
        if rtr(rid).is_none() {
            return;
        }
        let mut tree: BTreeMap<Vertex, (u32, Hops)> = BTreeMap::new();
        let mut cand: BTreeMap<Vertex, (u32, Hops)> = BTreeMap::new();
        let mut v = Vertex::Rtr(rid);
        let (mut vd, mut vh): (u32, Hops) = (0, Vec::new());
        tree.insert(v, (0, Vec::new()));
        let relax = |cand: &mut BTreeMap<Vertex, (u32, Hops)>, w: Vertex, d: u32, h: Hops| match cand.get_mut(&w) {
            Some(c) if c.0 < d => {}
            Some(c) if c.0 == d => merge_hops(&mut c.1, &h),
            _ => {
                let mut h = h;
                h.sort();
                h.dedup();
                cand.insert(w, (d, h));
            }
        };
        loop {
            match v {
                Vertex::Rtr(vid) => {
                    let Some(r) = rtr(vid) else {
                        break;
                    };
                    for l in &r.links {
                        match l.kind {
                            wire::LINK_P2P => {
                                let w = Vertex::Rtr(l.id);
                                if tree.contains_key(&w) {
                                    continue;
                                }
                                let Some(wr) = rtr(l.id) else {
                                    continue;
                                };
                                if !wr.links.iter().any(|x| x.kind == wire::LINK_P2P && x.id == vid) {
                                    continue;
                                }
                                let d = vd + l.metric as u32;
                                let hops = if vid == rid {
                                    // the interface of this link; the neighbour's address on it
                                    let Some(i) = self.if_with(l.data) else {
                                        continue;
                                    };
                                    let mine = self.ifs.get(&i).map(|f| f.addr).unwrap_or_default();
                                    let back = wr.links.iter().filter(|x| x.kind == wire::LINK_P2P && x.id == rid).map(|x| x.data).find(|a| mine.contains(*a));
                                    let addr = back.or_else(|| self.ifs.get(&i).and_then(|f| f.nbrs.values().find(|n| n.id == l.id)).map(|n| n.addr));
                                    match addr {
                                        Some(a) => alloc::vec![(a, i)],
                                        None => continue,
                                    }
                                } else {
                                    vh.clone()
                                };
                                relax(&mut cand, w, d, hops);
                            }
                            wire::LINK_TRANSIT => {
                                let w = Vertex::Net(l.id);
                                if tree.contains_key(&w) {
                                    continue;
                                }
                                let Some(n) = net(l.id) else {
                                    continue;
                                };
                                if !n.routers.contains(&vid) {
                                    continue;
                                }
                                let d = vd + l.metric as u32;
                                let hops = if vid == rid {
                                    match self.if_with(l.data) {
                                        Some(i) => alloc::vec![(Ipv4::UNSPECIFIED, i)],
                                        None => continue,
                                    }
                                } else {
                                    vh.clone()
                                };
                                relax(&mut cand, w, d, hops);
                            }
                            _ => {}
                        }
                    }
                }
                Vertex::Net(nid) => {
                    let Some(n) = net(nid) else {
                        break;
                    };
                    let direct = !vh.is_empty() && vh.iter().all(|h| h.0.is_unspecified());
                    for &wid in &n.routers {
                        let w = Vertex::Rtr(wid);
                        if tree.contains_key(&w) {
                            continue;
                        }
                        let Some(wr) = rtr(wid) else {
                            continue;
                        };
                        let Some(back) = wr.links.iter().find(|x| x.kind == wire::LINK_TRANSIT && x.id == nid) else {
                            continue;
                        };
                        // a router on a network the root is on: its address there is the next hop
                        let hops: Hops = if direct { vh.iter().map(|h| (back.data, h.1)).collect() } else { vh.clone() };
                        relax(&mut cand, w, vd, hops);
                    }
                }
            }
            // the next vertex: the least cost, networks first
            let Some(next) = cand.iter().min_by_key(|(w, c)| (c.0, **w)).map(|(w, _)| *w) else {
                break;
            };
            let Some((d, h)) = cand.remove(&next) else {
                break;
            };
            tree.insert(next, (d, h.clone()));
            v = next;
            vd = d;
            vh = h;
        }
        let add = |rt: &mut BTreeMap<Prefix4, ORoute>, p: Prefix4, cost: u32, hops: Hops| match rt.get_mut(&p) {
            Some(r) if r.kind == RtKind::Intra && r.cost < cost => {}
            Some(r) if r.kind == RtKind::Intra && r.cost == cost => merge_hops(&mut r.hops, &hops),
            _ => {
                rt.insert(p, ORoute { kind: RtKind::Intra, area, cost, cost2: 0, hops, since: now });
            }
        };
        // transit networks and the routers that matter for other areas and externals
        for (v, (d, h)) in &tree {
            match v {
                Vertex::Net(nid) => {
                    if let Some(n) = net(*nid)
                        && let Some(len) = n.mask.mask_len()
                    {
                        add(rt, Prefix4 { addr: *nid, len }.trunc(), *d, h.clone());
                    }
                }
                Vertex::Rtr(id) if *id != rid => {
                    if let Some(r) = rtr(*id)
                        && r.flags & (wire::RTR_B | wire::RTR_E) != 0
                    {
                        brt.insert((area, *id), BRoute { cost: *d, hops: h.clone(), flags: r.flags & (wire::RTR_B | wire::RTR_E), inter: false });
                    }
                }
                _ => {}
            }
        }
        // stub networks of every router of the tree (16.1, step 5)
        for (v, (d, h)) in &tree {
            let Vertex::Rtr(id) = v else {
                continue;
            };
            let Some(r) = rtr(*id) else {
                continue;
            };
            for l in r.links.iter().filter(|l| l.kind == wire::LINK_STUB) {
                let Some(len) = l.data.mask_len() else {
                    continue;
                };
                let p = Prefix4 { addr: l.id, len }.trunc();
                let hops = if *id == rid {
                    let i = self.ifs.iter().find(|(_, f)| f.area == area && (f.addr.trunc() == p || f.addr.addr == l.id || f.secondaries.iter().any(|s| s.trunc() == p || s.addr == l.id))).map(|(i, _)| *i).unwrap_or(0);
                    alloc::vec![(Ipv4::UNSPECIFIED, i)]
                } else {
                    h.clone()
                };
                add(rt, p, d + l.metric as u32, hops);
            }
        }
    }

    /// inter-area routes from the summary-LSAs of an area (16.2)
    fn inter_area(&self, now: Ns, area: u32, rt: &mut BTreeMap<Prefix4, ORoute>, brt: &mut BTreeMap<(u32, Ipv4), BRoute>) {
        let Some(db) = self.lsdb.get(&area) else {
            return;
        };
        for (k, e) in db {
            if !matches!(k.kind, wire::LSA_SUMMARY | wire::LSA_ASBR) || e.maxage(now) || Some(k.adv) == self.rid {
                continue;
            }
            let LsaBody::Summary(s) = &e.lsa.body else {
                continue;
            };
            if s.metric >= wire::LS_INFINITY {
                continue;
            }
            let Some(br) = brt.get(&(area, k.adv)).filter(|b| !b.inter && b.flags & wire::RTR_B != 0).cloned() else {
                continue;
            };
            let cost = br.cost + s.metric;
            if k.kind == wire::LSA_SUMMARY {
                let Some(len) = s.mask.mask_len() else {
                    continue;
                };
                let p = Prefix4 { addr: k.id, len }.trunc();
                match rt.get_mut(&p) {
                    Some(r) if r.kind == RtKind::Intra => {}
                    Some(r) if r.kind == RtKind::Inter && r.cost < cost => {}
                    Some(r) if r.kind == RtKind::Inter && r.cost == cost => merge_hops(&mut r.hops, &br.hops),
                    _ => {
                        rt.insert(p, ORoute { kind: RtKind::Inter, area, cost, cost2: 0, hops: br.hops.clone(), since: now });
                    }
                }
            } else {
                let key = (area, k.id);
                match brt.get_mut(&key) {
                    Some(b) if !b.inter => {}
                    Some(b) if b.cost < cost => {}
                    Some(b) if b.cost == cost => merge_hops(&mut b.hops, &br.hops),
                    _ => {
                        brt.insert(key, BRoute { cost, hops: br.hops.clone(), flags: wire::RTR_E, inter: true });
                    }
                }
            }
        }
    }

    /// AS-external routes (16.4)
    fn externals_rt(&self, now: Ns, rt: &mut BTreeMap<Prefix4, ORoute>, brt: &BTreeMap<(u32, Ipv4), BRoute>) {
        let mut ext: BTreeMap<Prefix4, ORoute> = BTreeMap::new();
        for (k, e) in &self.ext {
            if e.maxage(now) || Some(k.adv) == self.rid {
                continue;
            }
            let LsaBody::External(x) = &e.lsa.body else {
                continue;
            };
            if x.metric >= wire::LS_INFINITY {
                continue;
            }
            let Some(len) = x.mask.mask_len() else {
                continue;
            };
            let p = Prefix4 { addr: k.id, len }.trunc();
            // the ASBR: the cheapest entry of any area, the largest area of equally cheap ones
            let Some(((area, _), asbr)) = brt.iter().filter(|((_, id), b)| *id == k.adv && b.flags & wire::RTR_E != 0).min_by_key(|((a, _), b)| (b.cost, core::cmp::Reverse(*a))) else {
                continue;
            };
            let (dist, hops) = if x.fwd.is_unspecified() {
                (asbr.cost, asbr.hops.clone())
            } else {
                // the forwarding address must be reached by an intra- or inter-area route
                let via = rt.iter().filter(|(q, r)| q.contains(x.fwd) && matches!(r.kind, RtKind::Intra | RtKind::Inter)).max_by_key(|(q, _)| q.len);
                match via {
                    Some((_, r)) => (r.cost, r.hops.iter().map(|h| if h.0.is_unspecified() { (x.fwd, h.1) } else { *h }).collect()),
                    None => continue,
                }
            };
            let r = if x.e2 {
                ORoute { kind: RtKind::Ext2, area: *area, cost: dist, cost2: x.metric, hops, since: now }
            } else {
                ORoute { kind: RtKind::Ext1, area: *area, cost: dist + x.metric, cost2: 0, hops, since: now }
            };
            match ext.get_mut(&p) {
                Some(old) if old.rank() < r.rank() => {}
                Some(old) if old.rank() == r.rank() => merge_hops(&mut old.hops, &r.hops),
                _ => {
                    ext.insert(p, r);
                }
            }
        }
        for (p, r) in ext {
            if !rt.contains_key(&p) {
                rt.insert(p, r);
            }
        }
    }

    /// the routing table anew (16): every area, inter-area routes, externals; the routes that
    /// changed are noted
    fn spf(&mut self, os: &mut OsApi) {
        let now = os.now();
        self.spf_at = None;
        self.spf_runs += 1;
        self.last_spf = Some(now);
        let mut rt: BTreeMap<Prefix4, ORoute> = BTreeMap::new();
        let mut brt: BTreeMap<(u32, Ipv4), BRoute> = BTreeMap::new();
        let attached = self.attached();
        for &a in &attached {
            self.spf_area(now, a, &mut rt, &mut brt);
        }
        // an ABR takes inter-area routes from the backbone only (16.2)
        let from: Vec<u32> = if attached.len() > 1 { attached.iter().copied().filter(|a| *a == 0).collect() } else { attached.clone() };
        for a in from {
            self.inter_area(now, a, &mut rt, &mut brt);
        }
        self.externals_rt(now, &mut rt, &brt);
        // what changed: notes, and the time a route is as it is
        let (mut added, mut changed, mut removed) = (0u16, 0u16, 0u16);
        let first = |r: &ORoute| r.hops.first().map(|h| h.0).unwrap_or(Ipv4::UNSPECIFIED);
        for (p, r) in rt.iter_mut() {
            match self.rt.get(p) {
                Some(o) if o.kind == r.kind && o.cost == r.cost && o.cost2 == r.cost2 && o.hops == r.hops && o.area == r.area => r.since = o.since,
                Some(_) => {
                    changed += 1;
                    os.note(Note::OspfRoute { prefix: *p, kind: r.kind, cost: r.metric(), via: first(r), what: RtChange::Changed });
                }
                None => {
                    added += 1;
                    os.note(Note::OspfRoute { prefix: *p, kind: r.kind, cost: r.metric(), via: first(r), what: RtChange::Added });
                }
            }
        }
        for (p, o) in &self.rt {
            if !rt.contains_key(p) {
                removed += 1;
                os.note(Note::OspfRoute { prefix: *p, kind: o.kind, cost: o.metric(), via: first(o), what: RtChange::Removed });
            }
        }
        os.note(Note::OspfSpf { run: self.spf_runs as u32, routes: rt.len() as u16, added, changed, removed });
        self.rt = rt;
        self.brt = brt;
    }

    /// the routes for the RIB: one per next hop (up to maximum-paths); networks the router is on
    /// stay with their connected routes
    fn install_rib(&self, os: &mut OsApi) {
        let mut v: Vec<Route> = Vec::new();
        let max = self.cfg.max_paths.max(1) as usize;
        for (p, r) in &self.rt {
            for &(via, i) in r.hops.iter().filter(|h| !h.0.is_unspecified()).take(max) {
                v.push(Route { prefix: *p, via: Some(via), iface: Some(i), src: RouteSrc::Ospf, ad: self.cfg.distance, metric: r.metric() });
            }
        }
        let ni = os.ni;
        // BGP resolves next hops over them (bgp.rs)
        if os.eng.stack_mut(ni).is_some_and(|st| st.set_dynamic(RouteSrc::Ospf, v)) {
            os.eng.bgp_kick(ni);
        }
    }
}

// ---------------------------------------------------------------- output

impl Ospf {
    /// an OSPF packet out of interface `i`: to a group straight on the link, to a neighbour over
    /// the routing table (on point-to-point links always to AllSPFRouters: 8.1)
    fn emit(&mut self, os: &mut OsApi, i: u16, to: Option<Ipv4>, body: Body) -> Option<Ipv4> {
        let rid = self.rid?;
        let f = self.ifs.get(&i)?;
        let src = f.addr.addr;
        let dst = match to {
            Some(a) if f.kind != NetType::PointToPoint => a,
            Some(_) => wire::ALL_SPF_ROUTERS,
            None if f.kind == NetType::Broadcast && !matches!(f.state, IfState::Dr | IfState::Backup) => wire::ALL_D_ROUTERS,
            None => wire::ALL_SPF_ROUTERS,
        };
        let pkt = Packet { hdr: wire::Header { kind: 0, router: rid, area: f.area, autype: wire::AU_NULL, auth: [0; 8] }, body }.to_bytes();
        let (ni, cause) = (os.ni, os.cause);
        let st = os.eng.stack_mut(ni)?;
        let id = st.next_id();
        let mut p = Pkt::new(HEADROOM, &pkt);
        let mut h = Ipv4Hdr::new(src, dst, PROTO_OSPF, 1);
        h.id = id;
        h.dscp = DSCP_CS6;
        h.push_onto(&mut p);
        let bytes = p.into_vec();
        if dst.is_multicast() {
            os.eng.frame_out(ni, i, Mac::of_ipv4_multicast(dst), ET_IPV4, &bytes, cause);
        } else {
            os.eng.ip_output(ni, bytes, IpOrigin::Local { job: None }, cause);
        }
        Some(dst)
    }

    fn send_hello(&mut self, os: &mut OsApi, i: u16) {
        let Some(f) = self.ifs.get(&i) else {
            return;
        };
        let neighbors: Vec<Ipv4> = f.nbrs.values().filter(|n| n.state >= NbrState::Init).map(|n| n.id).collect();
        let bcast = f.kind == NetType::Broadcast;
        let h = Hello {
            mask: f.addr.mask(),
            interval: f.hello,
            options: self.options(f.area),
            priority: f.priority,
            dead: f.dead,
            dr: if bcast { f.dr } else { Ipv4::UNSPECIFIED },
            bdr: if bcast { f.bdr } else { Ipv4::UNSPECIFIED },
            neighbors,
        };
        let n = h.neighbors.len() as u16;
        os.note(Note::OspfHelloSent { iface: i, nbrs: n });
        self.n.hellos_sent += 1;
        let _ = self.emit(os, i, Some(wire::ALL_SPF_ROUTERS), Body::Hello(h));
    }

    /// the next DD to a neighbour (10.8): the first with I, M and MS, then as many headers of the
    /// summary list as fit
    fn send_dd(&mut self, os: &mut OsApi, i: u16, a: Ipv4, init: bool) {
        let now = os.now();
        let Some(f) = self.ifs.get(&i) else {
            return;
        };
        let (mtu, area) = (f.mtu, f.area);
        let options = self.options(area);
        let Some(n) = f.nbrs.get(&a) else {
            return;
        };
        let max = ((mtu as usize).saturating_sub(20 + wire::HDR_LEN + wire::DD_LEN) / wire::LSA_HDR_LEN).max(1);
        let mut headers: Vec<LsaHeader> = Vec::new();
        let mut taken = 0;
        if !init {
            for (sc, k) in n.summary.iter() {
                if headers.len() == max {
                    break;
                }
                taken += 1;
                if let Some(e) = self.entry(*sc, k) {
                    headers.push(e.hdr(now));
                }
            }
        }
        let (master, seq, nid) = (n.master, n.dd_seq, n.id);
        let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) else {
            return;
        };
        for _ in 0..taken {
            n.summary.pop_front();
        }
        let more = init || !n.summary.is_empty();
        let mut flags = if more { wire::DD_M } else { 0 };
        if init {
            flags |= wire::DD_I;
        }
        if !master {
            flags |= wire::DD_MS;
        }
        let dd = Dd { mtu, options, flags, seq, lsas: headers };
        let lsas = dd.lsas.len() as u16;
        let pkt = Packet { hdr: wire::Header { kind: wire::T_DD, router: self.rid.unwrap_or_default(), area, autype: wire::AU_NULL, auth: [0; 8] }, body: Body::Dd(dd.clone()) };
        n.more = more;
        n.last_tx = Some(pkt);
        n.dd_at = if master { None } else { Some(now + RXMT as Ns * SEC) };
        self.n.dd_sent += 1;
        os.note(Note::OspfDd { nbr: nid, iface: i, out: true, flags, seq, lsas });
        let _ = self.emit(os, i, Some(a), Body::Dd(dd));
    }

    /// the last DD once more (the master after RxmtInterval, the slave for a duplicate)
    fn resend_dd(&mut self, os: &mut OsApi, i: u16, a: Ipv4) {
        let now = os.now();
        let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) else {
            return;
        };
        let Some(p) = n.last_tx.clone() else {
            return;
        };
        let Body::Dd(d) = p.body else {
            return;
        };
        if !n.master {
            n.dd_at = Some(now + RXMT as Ns * SEC);
        }
        let nid = n.id;
        self.n.dd_sent += 1;
        os.note(Note::OspfDd { nbr: nid, iface: i, out: true, flags: d.flags, seq: d.seq, lsas: d.lsas.len() as u16 });
        let _ = self.emit(os, i, Some(a), Body::Dd(d));
    }

    /// a Link State Request for the beginning of the request list (10.9)
    fn send_lsr(&mut self, os: &mut OsApi, i: u16, a: Ipv4) {
        let now = os.now();
        let Some(f) = self.ifs.get_mut(&i) else {
            return;
        };
        let max = ((f.mtu as usize).saturating_sub(20 + wire::HDR_LEN) / wire::LSR_ENTRY_LEN).max(1);
        let Some(n) = f.nbrs.get_mut(&a) else {
            return;
        };
        if n.requests.is_empty() {
            n.asked.clear();
            n.lsr_at = None;
            return;
        }
        let keys: Vec<LsaKey> = n.requests.keys().take(max).copied().collect();
        n.asked = keys.clone();
        n.lsr_at = Some(now + RXMT as Ns * SEC);
        let nid = n.id;
        self.n.lsr_sent += 1;
        os.note(Note::OspfLsr { nbr: nid, iface: i, out: true, lsas: keys.len() as u16 });
        let _ = self.emit(os, i, Some(a), Body::Lsr(keys));
    }

    /// LSAs in Link State Updates (as many as fit into the MTU each)
    fn send_lsu(&mut self, os: &mut OsApi, i: u16, to: Option<Ipv4>, lsas: Vec<Vec<u8>>, why: LsuWhy) {
        let Some(f) = self.ifs.get(&i) else {
            return;
        };
        let room = (f.mtu as usize).saturating_sub(20 + wire::HDR_LEN + 4);
        let nbr = to.and_then(|a| f.nbrs.get(&a)).map(|n| n.id);
        let mut batch: Vec<Vec<u8>> = Vec::new();
        let mut size = 0;
        let mut batches: Vec<Vec<Vec<u8>>> = Vec::new();
        for l in lsas {
            if !batch.is_empty() && size + l.len() > room {
                batches.push(core::mem::take(&mut batch));
                size = 0;
            }
            size += l.len();
            batch.push(l);
        }
        if !batch.is_empty() {
            batches.push(batch);
        }
        for b in batches {
            self.n.lsu_sent += 1;
            if why == LsuWhy::Rxmt {
                self.n.retransmitted += b.len() as u64;
            }
            os.note(Note::OspfLsu { nbr, iface: i, out: true, lsas: b.len() as u16, why });
            let _ = self.emit(os, i, to, Body::Lsu(b));
        }
    }

    fn send_ack(&mut self, os: &mut OsApi, i: u16, to: Option<Ipv4>, hs: Vec<LsaHeader>) {
        let Some(f) = self.ifs.get(&i) else {
            return;
        };
        let max = ((f.mtu as usize).saturating_sub(20 + wire::HDR_LEN) / wire::LSA_HDR_LEN).max(1);
        let nbr = to.and_then(|a| f.nbrs.get(&a)).map(|n| n.id);
        for c in hs.chunks(max) {
            self.n.acks_sent += 1;
            os.note(Note::OspfAck { nbr, iface: i, out: true, lsas: c.len() as u16, delayed: to.is_none() });
            let _ = self.emit(os, i, to, Body::Ack(c.to_vec()));
        }
    }
}

// ---------------------------------------------------------------- time

impl Ospf {
    /// the earliest deadline of the process
    fn next_deadline(&self, now: Ns) -> Option<Ns> {
        let mut t: Option<Ns> = None;
        let mut at = |x: Option<Ns>| {
            if let Some(x) = x
                && t.is_none_or(|y| x < y)
            {
                t = Some(x);
            }
        };
        for f in self.ifs.values() {
            at(f.hello_at);
            at(f.wait_at);
            at(f.ack_at);
            for n in f.nbrs.values() {
                at(Some(n.dead_at));
                at(n.dd_at);
                at(n.lsr_at);
                at(n.rxmt.values().map(|x| x.1 + RXMT as Ns * SEC).min());
            }
        }
        for (key, o) in &self.own {
            at(o.due);
            // the refresh of an own LSA after LSRefreshTime
            if let Some(e) = self.entry(key.0, &key.1)
                && e.from.is_none()
                && !e.maxage(now)
            {
                at(Some(e.at + (wire::LS_REFRESH_TIME.saturating_sub(e.age0)) as Ns * SEC));
            }
        }
        // LSAs of others that reach MaxAge
        for db in self.lsdb.values().chain(core::iter::once(&self.ext)) {
            for e in db.values() {
                if !e.aged && e.from.is_some() {
                    at(Some(e.at + (wire::MAX_AGE.saturating_sub(e.age0)) as Ns * SEC));
                }
            }
        }
        at(self.spf_at);
        t
    }

    fn arm(&mut self, os: &mut OsApi) {
        let next = self.next_deadline(os.now());
        if next != self.armed {
            self.armed = next;
            match next {
                Some(t) => os.timer(T_TICK, t),
                None => os.cancel(T_TICK),
            }
        }
    }

    /// whatever is due now, in a fixed order: interfaces, neighbours, LSAs, SPF
    fn tick(&mut self, os: &mut OsApi) {
        let now = os.now();
        let ifs: Vec<u16> = self.ifs.keys().copied().collect();
        for &i in &ifs {
            let Some(f) = self.ifs.get(&i) else {
                continue;
            };
            let (hello, wait, ack) = (f.hello_at.is_some_and(|t| t <= now), f.wait_at.is_some_and(|t| t <= now), f.ack_at.is_some_and(|t| t <= now));
            // the election first: a Hello of the same moment carries its result
            if wait {
                self.elect(os, i);
            }
            if hello {
                self.send_hello(os, i);
                if let Some(f) = self.ifs.get_mut(&i) {
                    f.hello_at = Some(now + f.hello as Ns * SEC);
                }
            }
            if ack {
                let hs = self.ifs.get_mut(&i).map(|f| {
                    f.ack_at = None;
                    core::mem::take(&mut f.delayed)
                });
                if let Some(hs) = hs.filter(|h| !h.is_empty()) {
                    self.send_ack(os, i, None, hs);
                }
            }
        }
        for &i in &ifs {
            let nbrs: Vec<Ipv4> = self.ifs.get(&i).map(|f| f.nbrs.keys().copied().collect()).unwrap_or_default();
            for a in nbrs {
                let Some((dead, dd, lsr, due)) = self.ifs.get(&i).and_then(|f| f.nbrs.get(&a)).map(|n| {
                    let due: Vec<LsaKey> = n.rxmt.iter().filter(|(_, x)| x.1 + RXMT as Ns * SEC <= now).map(|(k, _)| *k).collect();
                    (n.dead_at <= now, n.dd_at.is_some_and(|t| t <= now), n.lsr_at.is_some_and(|t| t <= now), due)
                }) else {
                    continue;
                };
                if dead {
                    self.nbr_event(os, i, a, NbrEvent::InactivityTimer);
                    continue;
                }
                if dd {
                    self.resend_dd(os, i, a);
                }
                if lsr {
                    self.send_lsr(os, i, a);
                }
                if !due.is_empty() {
                    let area = self.ifs.get(&i).map(|f| f.area).unwrap_or(0);
                    let mut lsas: Vec<Vec<u8>> = Vec::new();
                    let mut keep: Vec<LsaKey> = Vec::new();
                    for k in &due {
                        match self.entry(scope_of(k.kind, area), k) {
                            Some(e) => {
                                lsas.push(e.wire(now));
                                keep.push(*k);
                            }
                            None => {}
                        }
                    }
                    if let Some(n) = self.ifs.get_mut(&i).and_then(|f| f.nbrs.get_mut(&a)) {
                        for k in &due {
                            if keep.contains(k) {
                                if let Some(x) = n.rxmt.get_mut(k) {
                                    x.1 = now;
                                }
                            } else {
                                n.rxmt.remove(k);
                            }
                        }
                    }
                    if !lsas.is_empty() {
                        self.send_lsu(os, i, Some(a), lsas, LsuWhy::Rxmt);
                    }
                }
            }
        }
        // own LSAs: refreshed after LSRefreshTime
        let keys: Vec<(Scope, LsaKey)> = self.own.keys().copied().collect();
        for (sc, k) in keys {
            let Some(e) = self.entry(sc, &k) else {
                continue;
            };
            if e.from.is_none() && !e.maxage(now) && e.age(now) >= wire::LS_REFRESH_TIME {
                let (opts, body) = (e.lsa.hdr.options, e.lsa.body.clone());
                self.originate(os, sc, k, opts, body, OrigWhy::Refresh);
            }
        }
        // LSAs of others that reached MaxAge: flooded once more (14)
        let mut aged: Vec<(Scope, LsaKey)> = Vec::new();
        for (a, db) in &self.lsdb {
            for (k, e) in db {
                if !e.aged && e.maxage(now) {
                    aged.push((Scope::Area(*a), *k));
                }
            }
        }
        for (k, e) in &self.ext {
            if !e.aged && e.maxage(now) {
                aged.push((Scope::As, *k));
            }
        }
        for (sc, k) in aged {
            let h = match self.db_mut(sc).get_mut(&k) {
                Some(e) => {
                    e.aged = true;
                    e.hdr(now)
                }
                None => continue,
            };
            os.note(Note::OspfLsaAged { lsa: LsaRef::of(&h, sc) });
            self.schedule_spf(now);
            self.flood(os, sc, k, None);
        }
        // instances that waited for MinLSInterval: reconcile() makes them
        for o in self.own.values_mut() {
            if o.due.is_some_and(|t| t <= now) {
                o.due = None;
            }
        }
        if self.spf_at.is_some_and(|t| t <= now) {
            self.spf(os);
        }
    }

    /// after every event: elections, own LSAs, what waits to go out, MaxAge LSAs, the RIB,
    /// multicast groups, the deadline timer
    fn after(&mut self, os: &mut OsApi) {
        if !self.running {
            return;
        }
        // NeighborChange counts from DR Other on, BackupSeen while it waits (9.3)
        let ifs: Vec<u16> = self.ifs.iter().filter(|(_, f)| f.elect || f.seen).map(|(i, _)| *i).collect();
        for i in ifs {
            let (st, seen) = self.ifs.get(&i).map(|f| (f.state, f.seen)).unwrap_or((IfState::Down, false));
            if matches!(st, IfState::DrOther | IfState::Backup | IfState::Dr) || (st == IfState::Waiting && seen) {
                self.elect(os, i);
            } else if let Some(f) = self.ifs.get_mut(&i) {
                f.elect = false;
                f.seen = false;
            }
        }
        self.reconcile(os);
        self.send_queues(os);
        self.maxage_sweep(os);
        self.install_rib(os);
        self.groups(os);
        self.arm(os);
    }

    /// AllSPFRouters on every interface that speaks, AllDRouters where it is DR or BDR
    fn groups(&self, os: &mut OsApi) {
        let ni = os.ni;
        let Some(st) = os.eng.stack_mut(ni) else {
            return;
        };
        st.groups.retain(|g| g.1 != wire::ALL_SPF_ROUTERS && g.1 != wire::ALL_D_ROUTERS);
        for (i, f) in &self.ifs {
            if f.speaks() {
                st.groups.push((*i, wire::ALL_SPF_ROUTERS));
                if matches!(f.state, IfState::Dr | IfState::Backup) {
                    st.groups.push((*i, wire::ALL_D_ROUTERS));
                }
            }
        }
    }

    /// everything learned goes (power off, "clear ip ospf process"); the configuration stays
    fn forget(&mut self, os: &mut OsApi) {
        let ifs: Vec<u16> = self.ifs.keys().copied().collect();
        for i in ifs {
            self.if_down(os, i);
        }
        self.ifs.clear();
        self.lsdb.clear();
        self.ext.clear();
        self.rt.clear();
        self.brt.clear();
        self.own.clear();
        self.spf_at = None;
        self.armed = None;
        self.rid = None;
    }
}

impl Service for Ospf {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn start(&mut self, os: &mut OsApi) {
        self.running = true;
        self.started = os.now();
        self.ifs.clear();
        self.lsdb.clear();
        self.ext.clear();
        self.rt.clear();
        self.brt.clear();
        self.own.clear();
        self.spf_at = None;
        self.armed = None;
        self.rid = None;
        self.sync(os);
        self.after(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        self.sync(os);
        match ev {
            SvcEvent::Ip(p) => self.input(os, p),
            SvcEvent::Timer(T_TICK) => {
                self.armed = None;
                self.tick(os);
            }
            SvcEvent::Ctl(Ctl::Renew) => {
                // "clear ip ospf process": a fresh start with the router ID configured now
                self.forget(os);
                self.started = os.now();
                self.sync(os);
            }
            _ => {}
        }
        self.after(os);
    }
    fn stop(&mut self, os: &mut OsApi) {
        self.forget(os);
        self.running = false;
        let ni = os.ni;
        if let Some(st) = os.eng.stack_mut(ni) {
            st.set_dynamic(RouteSrc::Ospf, Vec::new());
            st.groups.retain(|g| g.1 != wire::ALL_SPF_ROUTERS && g.1 != wire::ALL_D_ROUTERS);
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        let (all, full) = self.neighbours();
        let n = &self.n;
        alloc::vec![
            Metric::new("neighbours", all as i64),
            Metric::new("full", full as i64),
            Metric::new("lsas", self.lsa_count() as i64),
            Metric::new("routes", self.rt.len() as i64),
            Metric::new("spf_runs", self.spf_runs as i64),
            Metric::new("hellos_sent", n.hellos_sent as i64),
            Metric::new("hellos_rcvd", n.hellos_rcvd as i64),
            Metric::new("dd_sent", n.dd_sent as i64),
            Metric::new("dd_rcvd", n.dd_rcvd as i64),
            Metric::new("lsr_sent", n.lsr_sent as i64),
            Metric::new("lsr_rcvd", n.lsr_rcvd as i64),
            Metric::new("lsu_sent", n.lsu_sent as i64),
            Metric::new("lsu_rcvd", n.lsu_rcvd as i64),
            Metric::new("acks_sent", n.acks_sent as i64),
            Metric::new("acks_rcvd", n.acks_rcvd as i64),
            Metric::new("retransmitted", n.retransmitted as i64),
            Metric::new("originated", n.originated as i64),
            Metric::new("ignored", n.ignored as i64),
            Metric::new("adj_changes", n.adj_changes as i64),
        ]
    }
    fn metric_kind(&self, name: &str) -> MetricKind {
        match name {
            "neighbours" | "full" | "lsas" | "routes" => MetricKind::Gauge,
            _ => MetricKind::Counter,
        }
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        if !self.cfg.on {
            return msg!("OSPF ist nicht eingerichtet (kein router ospf).");
        }
        match self.rid {
            Some(id) if self.running => {
                let (all, full) = self.neighbours();
                msg!("OSPF-Prozess {}, Router-ID {}: Nachbarn: {}, davon Full: {}; LSAs: {}, Routen: {}.", self.cfg.pid, id, all, full, self.lsa_count(), self.rt.len())
            }
            None if self.running => msg!("OSPF-Prozess {}: wartet auf eine Adresse für die Router-ID.", self.cfg.pid),
            _ => msg!("OSPF-Prozess {}: wartet auf das Gerät.", self.cfg.pid),
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
    /// the OSPF service of a device (None before the first OSPF command)
    pub fn ospf(&self, dev: u32) -> Option<&Ospf> {
        self.ospf_of(self.node_index(dev)?)
    }
    pub(crate) fn ospf_of(&self, ni: usize) -> Option<&Ospf> {
        self.os(ni)?.svcs.iter().find(|s| s.kind == KIND)?.svc.as_ref()?.as_any().downcast_ref::<Ospf>()
    }
    fn ospf_mut(&mut self, ni: usize) -> Option<&mut Ospf> {
        self.os_mut(ni)?.svcs.iter_mut().find(|s| s.kind == KIND)?.svc.as_mut()?.as_any_mut().downcast_mut::<Ospf>()
    }
    /// (service id, running) of the OSPF service of node `ni`
    fn ospf_slot(&self, ni: usize) -> Option<(u32, bool)> {
        self.os(ni)?.svcs.iter().find(|s| s.kind == KIND).map(|s| (s.id, s.running))
    }
    /// "router ospf" is configured on node `ni`
    pub(crate) fn ospf_on(&self, ni: usize) -> bool {
        self.ospf_of(ni).is_some_and(|r| r.cfg.on)
    }
    /// something about node `ni`'s interfaces or routes may have changed: its OSPF looks again
    /// at the same virtual time (a timer of its own, so it never runs inside another call)
    pub(crate) fn ospf_kick(&mut self, ni: usize) {
        if let Some((id, true)) = self.ospf_slot(ni) {
            let now = self.now;
            self.set_timer(ni, SLOT_SVC + id * SVC_TIMERS + T_LOOK, now);
        }
    }
    pub(crate) fn ospf_kick_dev(&mut self, dev: u32) {
        if let Some(ni) = self.node_index(dev) {
            self.ospf_kick(ni);
        }
    }
    /// an OSPF packet for the device (ip_local): to the service's inbox; false without a running
    /// service (the packet then gets "protocol unreachable")
    pub(crate) fn ospf_rx(&mut self, ni: usize, iface: u16, v: &Ipv4View, hop: crate::trace::HopId) -> bool {
        let Some((id, true)) = self.ospf_slot(ni) else {
            return false;
        };
        let p = IpPkt { src: v.src(), dst: v.dst(), proto: v.proto(), iface, ttl: v.ttl(), data: v.payload().to_vec(), hop };
        self.os_push(ni, Job::Ev(id, SvcEvent::Ip(p)));
        true
    }

    /// Action::Ospf: a change of the configuration; the service comes with the first one
    /// (stopped until "router ospf")
    pub(crate) fn ospf_apply(&mut self, dev: u32, set: OspfSet) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let routes = matches!(self.nodes.get(ni), Some(Some(n)) if n.model.has(F_ROUTER));
        if !routes {
            return Err(ActErr::Refused(msg!("{} routet nicht: hier gibt es kein OSPF.", self.host(ni))));
        }
        if self.os(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        if self.ospf_slot(ni).is_none() {
            // undoing what was never set needs no service
            if matches!(set, OspfSet::Process { on: false, .. } | OspfSet::Clear) {
                return Ok(());
            }
            self.add_service(dev, KIND, Box::new(Ospf::new()), false)?;
        }
        let (id, running) = self.ospf_slot(ni).ok_or(ActErr::NoIp(dev))?;
        let canon = |e: &Engine, name: &str| -> Result<String, ActErr> {
            let st = e.stack(ni).ok_or(ActErr::NoIp(dev))?;
            let i = st.iface_by_name(name).ok_or_else(|| ActErr::UnknownIface { dev, iface: String::from(name) })?;
            Ok(st.ifaces[i as usize].name.clone())
        };
        let set = match set {
            OspfSet::Process { pid, on } => {
                let cur = self.ospf_of(ni).map(|o| (o.cfg.on, o.cfg.pid)).unwrap_or((false, 0));
                if on && cur.0 && cur.1 != pid {
                    return Err(ActErr::Refused(msg!("Hier läuft schon OSPF-Prozess {}: ein Gerät hat nur einen.", cur.1)));
                }
                if !on && cur.0 && cur.1 != pid {
                    return Err(ActErr::Refused(msg!("OSPF-Prozess {} gibt es hier nicht.", pid)));
                }
                if let Some(o) = self.ospf_mut(ni) {
                    if on {
                        o.cfg.on = true;
                        o.cfg.pid = pid;
                    } else {
                        o.cfg.reset();
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
            OspfSet::Clear => {
                if running {
                    self.svc_ctl(ni, id, Ctl::Renew);
                }
                return Ok(());
            }
            OspfSet::Passive { iface: Some(name), on } => OspfSet::Passive { iface: Some(canon(self, &name)?), on },
            OspfSet::IfCost { iface, cost } => OspfSet::IfCost { iface: canon(self, &iface)?, cost },
            OspfSet::IfPriority { iface, priority } => OspfSet::IfPriority { iface: canon(self, &iface)?, priority },
            OspfSet::IfHello { iface, secs } => OspfSet::IfHello { iface: canon(self, &iface)?, secs },
            OspfSet::IfDead { iface, secs } => OspfSet::IfDead { iface: canon(self, &iface)?, secs },
            OspfSet::IfNetwork { iface, kind } => OspfSet::IfNetwork { iface: canon(self, &iface)?, kind },
            OspfSet::IfArea { iface, area } => OspfSet::IfArea { iface: canon(self, &iface)?, area },
            other => other,
        };
        let o = self.ospf_mut(ni).ok_or(ActErr::NoIp(dev))?;
        let c = &mut o.cfg;
        let put = |m: &mut BTreeMap<String, u32>, k: String, v: Option<u32>| match v {
            Some(v) => {
                m.insert(k, v);
            }
            None => {
                m.remove(&k);
            }
        };
        match set {
            OspfSet::RouterId(r) => {
                if r.is_some_and(|a| a.is_unspecified() || a.is_multicast() || a.is_broadcast()) {
                    return Err(ActErr::Refused(msg!("{} taugt nicht als Router-ID.", r.unwrap_or_default())));
                }
                c.router_id = r;
            }
            OspfSet::Network { net, add } => {
                c.networks.retain(|n| !(n.addr == net.addr && n.wild == net.wild));
                if add {
                    c.networks.push(net);
                }
            }
            OspfSet::Passive { iface: None, on } => {
                c.passive_default = on;
                c.passive.clear();
            }
            OspfSet::Passive { iface: Some(name), on } => {
                c.passive.retain(|x| !x.eq_ignore_ascii_case(&name));
                if on != c.passive_default {
                    c.passive.push(name);
                }
            }
            OspfSet::RefBw(b) => c.ref_bw = b.unwrap_or(REF_BW).clamp(1, 4_294_967),
            OspfSet::DefaultOrig(d) => c.default_orig = d,
            OspfSet::Redistribute { r, add } => {
                c.redistribute.retain(|x| x.what != r.what);
                if add {
                    c.redistribute.push(r);
                    c.redistribute.sort_by_key(|x| x.what);
                }
            }
            OspfSet::Stub { area, no_summary, add } => {
                if area == 0 {
                    return Err(ActErr::Refused(msg!("Der Backbone (Bereich 0) kann kein Stub-Bereich sein.")));
                }
                if add {
                    c.stubs.insert(area, no_summary);
                } else {
                    c.stubs.remove(&area);
                }
            }
            OspfSet::Distance(d) => c.distance = d.unwrap_or(DISTANCE).max(1),
            OspfSet::MaxPaths(n) => c.max_paths = n.unwrap_or(MAX_PATHS).clamp(1, 32),
            OspfSet::LogAdj { on, detail } => {
                c.log_adj = on;
                c.log_detail = on && detail;
            }
            OspfSet::IfCost { iface, cost } => match cost {
                Some(v) => {
                    c.cost.insert(iface, v.max(1));
                }
                None => {
                    c.cost.remove(&iface);
                }
            },
            OspfSet::IfPriority { iface, priority } => match priority {
                Some(v) => {
                    c.priority.insert(iface, v);
                }
                None => {
                    c.priority.remove(&iface);
                }
            },
            OspfSet::IfHello { iface, secs } => match secs {
                Some(v) => {
                    c.hello.insert(iface, v.max(1));
                }
                None => {
                    c.hello.remove(&iface);
                }
            },
            OspfSet::IfDead { iface, secs } => put(&mut c.dead, iface, secs.map(|s| s.max(1))),
            OspfSet::IfNetwork { iface, kind } => match kind {
                Some(k) => {
                    c.net_type.insert(iface, k);
                }
                None => {
                    c.net_type.remove(&iface);
                }
            },
            OspfSet::IfArea { iface, area } => match area {
                Some(a) => {
                    c.if_area.insert(iface, a);
                }
                None => {
                    c.if_area.remove(&iface);
                }
            },
            OspfSet::Process { .. } | OspfSet::Clear => {}
        }
        if running {
            self.ospf_kick(ni);
        }
        Ok(())
    }
}
