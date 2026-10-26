// trace -- what happened: every frame on every cable (Hop) and every decision of a device
// (DevEvent with a Note), in two bounded rings
//
//   Trace::hops() / hop(id) / hops_after(id)      frames in send order; ids are consecutive
//   Trace::events() / events_of(hop)              decisions in time order
//   Note::describe(port, iface) -> German text     "MAC 02:4a:.. an Gi1/0/1 gelernt (VLAN 1)"
//   Note::message(port, iface) -> Msg             the same as a template with arguments, for
//                                                 the app's catalogue (jfn_model::msg)
//   Note::layer()                                 1 cable, 2 bridge and ARP, 3 IPv4, 4 UDP/TCP,
//                                                 7 services (DHCP, DNS, syslog, NTP); TCP's
//                                                 states and decisions are layer 4 (tcp.rs),
//                                                 HTTP's layer 7 (http.rs, httpsvc.rs), and so
//                                                 are RIP's (rip.rs: Note::is_rip); spanning
//                                                 tree's layer 2 (stp.rs; is_stp, is_stp_event);
//                                                 OSPF's layer 3, as IP carries it directly
//                                                 (ospf.rs: Note::is_ospf), and IPv6's layer 3
//                                                 (trace6.rs: Note::V6); WLAN's layer 2
//                                                 (wlantrace.rs: Note::Wlan)
//   fmt_ns(ns)                                    "1,250 s", "3,200 ms", "450 µs" (in a Msg:
//                                                 msg::dur, which follows the language)
//   Trace::event(seq) / events_after(seq)         decisions by their consecutive number
//   Trace::on_wire(t) / prev_send / next_send     frames on a cable at a moment; the moments
//                                                 frames were sent (simulation mode steps)
//   Trace::batch_events(batch, dev)               what one device did in one batch
//   Filter                                        protocols and devices (steps, lists, files)
//
// The inspector decodes the bytes of a hop (netpkt::decode); there is no second model of the
// packet.  Notes are data, so the app can word them in any language: message() gives the German
// template and its arguments, src/locales/engine.tsv the English one.
//
// Batches: every kernel event is one batch, and so is every change from outside between them
// (an action, a ping: Trace::new_batch).  The decisions and frames of one device in one batch
// are what it did with one frame (Hop::cause) or on its own (a command, a job, a timer); the
// packet tab shows that as the device's inbound and outbound PDUs with the decisions between
// them.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;

use jfn_model::msg;
use jfn_model::msg::{dur, phrase, Msg};
use netpkt::{IpAddr, Ipv4, Mac, Prefix4};

use crate::kernel::{Frame, Ns};
use crate::resolver::LookupErr;
use crate::rip::{RipDrop, RipHow, RipLoss, RipOut};
use crate::tcp::{RstWhy, TcpEnd, TcpState};

pub type HopId = u64;

/// why a frame on a cable did not arrive
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HopDrop {
    /// the loss rate of the cable took it
    Loss,
    /// the cable went down while the frame was on it
    LinkDown,
}

/// one frame on one cable
#[derive(Clone, Debug)]
pub struct Hop {
    pub id: HopId,
    /// the sender starts serialising
    pub t_tx: Ns,
    /// the last bit reaches the receiver
    pub t_rx: Ns,
    /// project link id
    pub link: u32,
    /// (device id, port index) of the sender and of the receiver
    pub from: (u32, u16),
    pub to: (u32, u16),
    pub bytes: Frame,
    /// netpkt::decode P_* bits
    pub mask: u32,
    pub drop: Option<HopDrop>,
    /// the frame whose arrival made the sender send this one (None: the sender acted on its
    /// own: a command, a job, a timer)
    pub cause: Option<HopId>,
    /// the batch in which the sender handed it to its port
    pub batch: u64,
    /// when the sender handed it to its port (before t_tx when it waited in the queue)
    pub t_queued: Ns,
    /// a Cisco HDLC frame of a serial line (else Ethernet)
    pub hdlc: bool,
}

/// a decision of a device (port numbers are the device's port indexes)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// the cable on a port came up / went down
    LinkUp { port: u16 },
    LinkDown { port: u16 },
    /// a frame was sent / dropped at the sender
    DropNoLink { port: u16 },
    DropQueueFull { port: u16 },
    /// a frame that was on a cable when it went down
    LostInFlight { port: u16 },
    /// bridge: source address learned / moved to another port
    Learned { mac: Mac, port: u16, vlan: u16 },
    Moved { mac: Mac, from: u16, to: u16, vlan: u16 },
    TableFull { vlan: u16 },
    /// bridge: known destination / unknown or group destination / destination behind the
    /// ingress port
    Forwarded { vlan: u16, port: u16 },
    Flooded { vlan: u16, ports: u16 },
    Filtered { vlan: u16 },
    /// bridge: refused at the ingress port
    DropTagged { port: u16, vid: u16 },
    DropVlan { port: u16, vid: u16 },
    DropMalformed { port: u16 },
    /// bridge: a link-local control frame (STP, LLDP, ...) for the bridge itself
    Consumed { port: u16 },
    /// hub: repeated to every other port
    Repeated { ports: u16 },
    /// host: the frame is for this device / for another one
    Delivered { port: u16 },
    NotForMe { port: u16 },
    /// ARP (interface numbers are the device's IPv4 interfaces)
    ArpRequest { iface: u16, ip: Ipv4 },
    ArpReply { iface: u16, ip: Ipv4, mac: Mac },
    ArpLearned { ip: Ipv4, mac: Mac },
    ArpWaiting { ip: Ipv4 },
    /// dropped while ARP asks (IOS keeps nothing back)
    ArpIncomplete { ip: Ipv4 },
    /// proxy ARP: answered for a host behind another interface
    ArpProxied { iface: u16, ip: Ipv4 },
    ArpFailed { ip: Ipv4 },
    /// an ARP request for an address that is not this device's
    ArpIgnored { ip: Ipv4 },
    /// IPv4
    IpLocal { proto: u8 },
    IpRouted { dst: Ipv4, gw: Ipv4, iface: u16 },
    IpNoRoute { dst: Ipv4 },
    IpTtlExpired { src: Ipv4 },
    IpNotForMe { dst: Ipv4 },
    /// an address that must never be on a wire (127/8, a multicast or broadcast source)
    IpMartian { src: Ipv4, dst: Ipv4 },
    /// a directed broadcast is not forwarded into its subnet
    IpDirectedBroadcast { dst: Ipv4 },
    IpBadChecksum,
    IpMalformed,
    IpFragmented { parts: u16, mtu: u16 },
    IpFragNeeded { mtu: u16 },
    IpReassembled { bytes: u32 },
    IpReasmTimeout,
    /// ICMP
    EchoAnswered { to: Ipv4 },
    IcmpSent { typ: u8, code: u8, to: Ipv4 },
    IcmpLimited,
    /// UDP / TCP without a listener
    UdpNoPort { port: u16 },
    TcpNoListener { port: u16 },
    /// a datagram went to a socket: of a service ("dhcp-server"), the resolver or the log
    UdpToSocket { port: u16, to: String },
    /// a service could not bind its port
    PortInUse { port: u16, svc: String },
    /// DHCP client (interface numbers are IPv4 interfaces)
    DhcpDiscover { iface: u16, xid: u32, secs: u16 },
    DhcpOffered { ip: Ipv4, server: Ipv4 },
    DhcpRequest { iface: u16, ip: Ipv4, kind: DhcpReq },
    DhcpAcked { ip: Ipv4, server: Ipv4, lease: u32 },
    DhcpNaked { server: Ipv4 },
    /// address conflict detection before the address is used (ARP probe, RFC 5227)
    DhcpProbe { ip: Ipv4 },
    DhcpConflict { ip: Ipv4, mac: Mac },
    DhcpBound { iface: u16, addr: Prefix4, router: Option<Ipv4>, renew: u32 },
    DhcpExpired { ip: Ipv4 },
    DhcpReleased { ip: Ipv4, server: Ipv4 },
    DhcpNoOffers { iface: u16, retry: u32 },
    /// DHCP server
    DhcpdOffer { ip: Ipv4, mac: Mac },
    DhcpdPing { ip: Ipv4 },
    DhcpdPingConflict { ip: Ipv4 },
    DhcpdAck { ip: Ipv4, mac: Mac, lease: u32 },
    DhcpdNak { ip: Ipv4, mac: Mac },
    DhcpdNoPool { net: Ipv4 },
    DhcpdExhausted { pool: String },
    DhcpdDecline { ip: Ipv4, mac: Mac },
    DhcpdRelease { ip: Ipv4 },
    DhcpdOtherServer { mac: Mac, server: Ipv4 },
    DhcpdSilent { ip: Ipv4, mac: Mac },
    DhcpdInform { to: Ipv4 },
    /// DHCP relay agent ("ip helper-address")
    DhcpRelayed { to: Ipv4, giaddr: Ipv4 },
    DhcpRelayedBack { to: Ipv4, iface: u16 },
    DhcpRelayDrop { why: RelayDrop },
    /// DNS server
    DnsAnswer { name: String, qtype: u16, n: u16 },
    DnsNxDomain { name: String },
    DnsNoData { name: String, qtype: u16 },
    DnsReferral { name: String, zone: String },
    DnsRefused { name: String },
    DnsForward { name: String },
    /// resolver of the device
    ResolveQuery { name: String, qtype: u16, server: IpAddr },
    ResolveAnswer { name: String, addr: Option<IpAddr>, cached: bool },
    ResolveNegative { name: String, nx: bool },
    ResolveTimeout { name: String, server: IpAddr },
    ResolveFailed { name: String, why: LookupErr },
    /// syslog
    SyslogSent { to: Ipv4, sev: u8 },
    SyslogStored { from: Ipv4, sev: u8 },
    /// NTP (offsets and delays in ns)
    NtpRequest { server: Ipv4 },
    NtpSample { server: Ipv4, offset: i64, delay: i64 },
    NtpStep { offset: i64 },
    NtpSlew { offset: i64 },
    NtpServed { to: Ipv4, stratum: u8 },
    NtpNoTime { to: Ipv4 },
    /// TCP (tcp.rs; `port` is this device's, `peer`:`pport` the other end): a state change
    TcpStateChange { port: u16, peer: IpAddr, pport: u16, from: TcpState, to: TcpState },
    /// a segment (not a SYN) for no connection: answered with RST
    TcpNoConn { port: u16 },
    /// the accept queue of a listening socket is full: a SYN, or the last ACK of a
    /// handshake, is dropped
    TcpSynDropped { port: u16 },
    TcpAckDropped { port: u16 },
    /// the SYN came again: the SYN-ACK goes again
    TcpSynAgain,
    /// outside the receive window: an ACK goes back, the segment is dropped (its sequence
    /// number and length in sequence space)
    TcpUnacceptable { seq: u32, len: u32 },
    /// a RST or SYN in the window but not where expected: a challenge ACK (RFC 5961)
    TcpChallenge { syn: bool },
    /// a RST arrived: the connection attempt was refused, or the connection reset
    TcpReset { refused: bool },
    /// a RST went out
    TcpRstSent { why: RstWhy },
    /// receiver: bytes taken in order, held beyond a gap, the gap closed, bytes that came
    /// twice
    TcpReceived { bytes: u32 },
    TcpOutOfOrder { seq: u32, bytes: u32 },
    TcpGapFilled { bytes: u32 },
    TcpDuplicate { bytes: u32 },
    /// receiver: the ACK waits (delayed ACK); the delayed ACK went
    TcpAckDelayed { delay: Ns },
    TcpDelayedAck,
    /// receiver: the buffer is full, the window 0; after reading the window opens again
    TcpZeroWindow,
    TcpWindowUpdate { wnd: u16 },
    /// sender: new data acknowledged (and the congestion window after it), a duplicate ACK
    TcpAcked { bytes: u32, cwnd: u32, rtt: Option<Ns> },
    TcpDupAck { n: u8 },
    /// sender: fast retransmit after three duplicate ACKs; fast recovery ends
    TcpFastRetransmit { seq: u32, ssthresh: u32, cwnd: u32 },
    TcpRecoveryEnd { cwnd: u32 },
    /// sender: the retransmission timer expired (the doubled RTO, the attempt)
    TcpRto { seq: u32, rto: Ns, n: u8 },
    /// sender: Nagle holds back a small segment
    TcpNagle { bytes: u32 },
    /// sender: nothing in flight and the window too small: a probe follows; a probe went
    TcpPersist { after: Ns },
    TcpProbe { n: u8 },
    TcpKeepalive { n: u8 },
    /// the connection is given up
    TcpGiveUp { why: TcpEnd },
    /// "fragmentation needed": smaller segments from now on
    TcpPmtu { mss: u16 },
    /// HTTP (http.rs, httpsvc.rs): a server read a request
    HttpRequest { method: String, path: String },
    /// a server answered (status, body bytes)
    HttpAnswered { status: u16, bytes: u32 },
    /// every worker is busy: the request waits (with `waiting` others)
    HttpQueued { waiting: u32 },
    /// the queue of waiting requests is full: 503 at once
    HttpOverload,
    /// the error rate turned this answer into an error
    HttpInjected { status: u16 },
    /// a request that could not be read: answered with this status, the connection closes
    HttpBadRequest { status: u16 },
    /// a connection closed because it idled (keep-alive) or sent no head in time
    HttpIdleClose { header: bool },
    /// a client got an answer after `ms` milliseconds
    HttpReply { status: u16, ms: u32 },
    /// a client's request failed (a timeout or another error)
    HttpFailed { timeout: bool },
    /// lab (fault.rs): a fault started / was healed (its number and what it does, as the
    /// fault's message: FaultMark::msg)
    FaultStart { id: u32, what: Msg },
    FaultEnd { id: u32, what: Msg },
    /// a partition: the packet comes from a device on the other side and is dropped
    PartitionDrop { src: Ipv4 },
    /// load balancer (lb.rs): a backend was picked by the algorithm
    LbPick { backend: String, algo: &'static str },
    /// no backend can take the request or connection
    LbNoBackend,
    /// a backend could not be reached: the next one is tried
    LbRetry { backend: String },
    /// health checks changed the state of a backend
    LbHealth { backend: String, up: bool },
    /// a draining backend has nothing left (or its time ran out) and is gone
    LbDrained { backend: String },
    /// KV store (kv.rs): this node coordinates a request for a key over its replicas
    KvCoordinate { key: String, op: &'static str, replicas: u8 },
    /// the write goes on to the leader of the key
    KvForward { key: String, to: Ipv4 },
    /// the leader stored a version and waits for `need` acknowledgements in all
    KvWrite { key: String, version: String, need: u8 },
    /// the quorum of a read or write was reached or missed
    KvQuorum { key: String, got: u8, need: u8, ok: bool },
    /// a replica took a version from the leader (or had it already)
    KvReplica { key: String, version: String, stored: bool },
    /// a replica with an older version gets the newest one
    KvReadRepair { key: String, to: Ipv4 },
    /// a member of the cluster stopped or started answering heartbeats
    KvMember { member: Ipv4, up: bool },
    /// the disk is full: a write is refused
    DiskFull,
    /// cache (cache.rs): answered from memory / asked the backend
    CacheHit { key: String },
    CacheMiss { key: String },
    /// message queue (mq.rs): a message was taken, handed out (n-th delivery), acknowledged,
    /// put back after its visibility timeout, given up after too many deliveries
    MqPublish { topic: String, id: u64 },
    MqDeliver { topic: String, id: u64, n: u32 },
    MqAck { topic: String, id: u64 },
    MqRedeliver { topic: String, id: u64 },
    MqDead { topic: String, id: u64 },
    /// scripts (scriptsvc.rs): an event waits for the CPU, a datagram is dropped because too
    /// many wait, an error ended a callback (line, column and the VM's German message)
    ScriptBusy { svc: String, waiting: u32 },
    ScriptDropped { svc: String },
    ScriptError { svc: String, line: u32, col: u32, msg: String },
    /// spanning tree (stp.rs, stp98.rs, rstp.rs): a BPDU arrived / went out on a port
    StpRx { port: u16, bpdu: netpkt::stp::Bpdu },
    StpTx { port: u16, bpdu: netpkt::stp::Bpdu },
    /// a BPDU the protocol does not take (a type it does not know, Message Age at Max Age, its
    /// own BPDU back over a loop)
    StpDropped { port: u16, why: crate::stp::BpduDrop },
    /// a frame met a port that does not forward (where it came in, or the port of its
    /// destination)
    StpBlocked { port: u16, state: crate::stp::PortState },
    /// a port's role / state changed
    StpRole { port: u16, from: crate::stp::PortRole, to: crate::stp::PortRole },
    StpState { port: u16, from: crate::stp::PortState, to: crate::stp::PortState },
    /// the root bridge as this bridge sees it now (port None: this bridge is the root)
    StpRoot { root: netpkt::stp::BridgeId, port: Option<u16>, cost: u32 },
    /// a topology change: detected, TCN sent or received, acknowledged, TC flag
    StpTopology { port: Option<u16>, what: crate::stp::TcWhat },
    /// RSTP: what was learned on a port is forgotten after a topology change
    StpFlush { port: u16, entries: u32 },
    /// MAC entries age after `secs` (fast: Forward Delay during an 802.1D topology change)
    StpAging { secs: u16, fast: bool },
    /// the information about a port's LAN aged out (no BPDU in time)
    StpAged { port: u16 },
    /// a port is an edge port now / is none any more
    StpEdge { port: u16, edge: bool },
    /// RSTP: a port speaks 802.1D because its neighbour does / tries RSTP again
    StpMigrate { port: u16, rstp: bool },
    /// BPDU guard shut a port off (err-disabled)
    StpErrDisable { port: u16 },
    /// RIP (rip.rs; `iface` is an IPv4 interface): a request for the whole table went out
    RipRequest { iface: u16, ver: u8 },
    /// a request arrived and is answered (for the whole table, or for single routes)
    RipAsked { from: Ipv4, iface: u16, whole: bool },
    /// an update went out (its routes with the metrics sent, how many split horizon poisoned)
    RipSent { iface: u16, ver: u8, to: Ipv4, kind: RipOut, routes: Vec<(Prefix4, u8)>, poisoned: u16 },
    /// an update arrived and is taken (its routes with the metrics as sent)
    RipHeard { from: Ipv4, iface: u16, ver: u8, routes: Vec<(Prefix4, u8)> },
    /// a message was ignored, or an entry of an update
    RipIgnored { from: Ipv4, iface: u16, why: RipDrop },
    RipBadEntry { from: Ipv4, addr: Ipv4, metric: u32 },
    /// the table: a route learned, changed, one more equal-cost path
    RipLearned { prefix: Prefix4, via: Ipv4, metric: u8 },
    RipChanged { prefix: Prefix4, via: Ipv4, metric: u8, old: u8 },
    RipPathAdded { prefix: Prefix4, via: Ipv4, metric: u8 },
    /// a route lost its last path: unreachable (metric 16) until the garbage collection; a path
    /// of several lost
    RipDown { prefix: Prefix4, via: Ipv4, why: RipLoss },
    RipPathGone { prefix: Prefix4, via: Ipv4, why: RipLoss },
    /// held down: an offer no better than the route was is ignored
    RipHeld { prefix: Prefix4, from: Ipv4, metric: u8 },
    /// the garbage collection removed a route
    RipFlushed { prefix: Prefix4 },
    /// a network of the device itself is advertised now / no longer
    RipLocal { prefix: Prefix4, how: RipHow },
    RipLocalGone { prefix: Prefix4 },
    /// IPv4: a multicast packet for a group no interface here listens to
    IpGroupIgnored { dst: Ipv4 },
    /// access lists (acl.rs): line `seq` of the list on interface `iface` (inbound or
    /// outbound) let the packet through / dropped it; seq 0 is the implicit deny at the end
    AclPermit { list: String, seq: u32, iface: u16, out: bool },
    AclDeny { list: String, seq: u32, iface: u16, out: bool },
    /// NAT (nat.rs; an endpoint is an address and a port or ICMP identifier, 0 when the
    /// address alone counts): a new translation of a flow with its remote end, or a new
    /// mapping (remote None; protocol 0: an address with every protocol)
    NatCreate { proto: u8, local: (Ipv4, u16), global: (Ipv4, u16), remote: Option<(Ipv4, u16)> },
    /// the source (inside to outside) or the destination (outside to inside) of a packet
    NatTranslate { proto: u8, inbound: bool, from: (Ipv4, u16), to: (Ipv4, u16) },
    /// an ICMP error about a translated packet: the packet it quotes translated back (RFC 5508)
    NatQuote { inbound: bool, from: (Ipv4, u16), to: (Ipv4, u16) },
    /// a translation ended after its time without traffic
    NatExpire { proto: u8, local: (Ipv4, u16), global: (Ipv4, u16), remote: Option<(Ipv4, u16)> },
    /// a packet to a global address with no translation for it: it goes on as it is
    NatNoEntry { proto: u8, dst: (Ipv4, u16) },
    /// a packet from inside that no rule translates: it leaves as it is
    NatNoRule { src: Ipv4 },
    /// no free address or port for a new translation ("pool NAME", "interface X"): dropped,
    /// the sender gets "administratively prohibited"
    NatExhausted { what: String },
    /// an ICMP error about nothing NAT knows: dropped (RFC 5508)
    NatErrorDropped,
    /// a fragment waits for the first one of its datagram (its identification); the fragments
    /// of a datagram whose first part never came are dropped
    NatFragWait { id: u16 },
    NatFragDropped { id: u16, parts: u16 },
    /// IPv6 (trace6.rs: neighbor discovery, addresses, forwarding, ICMPv6)
    V6(crate::trace6::Note6),
    /// OSPF (ospf.rs; `iface` is an IPv4 interface, a neighbour is named by its router ID): the
    /// process took this router ID
    OspfRouterId { id: Ipv4 },
    /// a Hello went out (with the neighbours it names) / was taken
    OspfHelloSent { iface: u16, nbrs: u16 },
    OspfHelloRcvd { from: Ipv4, iface: u16 },
    /// a packet was ignored (`from`: its source address)
    OspfIgnored { from: Ipv4, iface: u16, why: crate::ospf::OspfDrop },
    /// an interface changed its state (9.3)
    OspfIfState { iface: u16, from: crate::ospf::IfState, to: crate::ospf::IfState },
    /// the DR election of an interface: DR and BDR by router ID (0.0.0.0: none)
    OspfDr { iface: u16, dr: Ipv4, bdr: Ipv4 },
    /// a neighbour changed its state, and the event that did it (10.3)
    OspfNbr { nbr: Ipv4, iface: u16, from: crate::ospf::NbrState, to: crate::ospf::NbrState, ev: crate::ospf::NbrEvent },
    /// the negotiation of a database exchange: this router is the master (or the slave)
    OspfMaster { nbr: Ipv4, iface: u16, master: bool },
    /// a Database Description packet went out / came in (flags, sequence number, LSA headers)
    OspfDd { nbr: Ipv4, iface: u16, out: bool, flags: u8, seq: u32, lsas: u16 },
    /// a Link State Request
    OspfLsr { nbr: Ipv4, iface: u16, out: bool, lsas: u16 },
    /// a Link State Update (`nbr` None: flooded to a group)
    OspfLsu { nbr: Option<Ipv4>, iface: u16, out: bool, lsas: u16, why: crate::ospf::LsuWhy },
    /// a Link State Acknowledgment (delayed ones go to a group)
    OspfAck { nbr: Option<Ipv4>, iface: u16, out: bool, lsas: u16, delayed: bool },
    /// the database: a newer instance from a neighbour installed, an own one made, flooded out of
    /// so many interfaces, acknowledged by a neighbour (explicitly or implied), an own one flushed
    /// (premature aging), one at MaxAge, one removed, an older one answered with the database
    /// copy, one dropped
    OspfLsaNew { lsa: crate::ospf::LsaRef, from: Ipv4 },
    OspfLsaOrig { lsa: crate::ospf::LsaRef, why: crate::ospf::OrigWhy },
    OspfLsaFlooded { lsa: crate::ospf::LsaRef, ifaces: u16 },
    OspfLsaAcked { lsa: crate::ospf::LsaRef, nbr: Ipv4 },
    OspfLsaFlush { lsa: crate::ospf::LsaRef },
    OspfLsaAged { lsa: crate::ospf::LsaRef },
    OspfLsaGone { lsa: crate::ospf::LsaRef },
    OspfLsaOlder { lsa: crate::ospf::LsaRef, nbr: Ipv4 },
    OspfLsaDropped { lsa: crate::ospf::LsaRef, why: crate::ospf::LsaDrop },
    /// SPF ran (its number, the routes now, how many came, changed and went), and a route that
    /// came, changed (next hop or cost) or went (`via` 0.0.0.0: a network the router is on)
    OspfSpf { run: u32, routes: u16, added: u16, changed: u16, removed: u16 },
    OspfRoute { prefix: Prefix4, kind: crate::ospf::RtKind, cost: u32, via: Ipv4, what: crate::ospf::RtChange },
    /// TCP over IPv6 (tcp.rs): Packet Too Big about a segment in flight, smaller segments from
    /// now on
    TcpPtb { mss: u16 },
    /// WLAN (wlan.rs, wlantrace.rs): scans, authentication, association, the 4-way handshake,
    /// roaming, range, what an access point drops (layer 2)
    Wlan(crate::wlantrace::WNote),
}

/// the kind of DHCPREQUEST a client sends
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DhcpReq {
    /// takes an offer (SELECTING)
    Select,
    /// keeps the address it had before a restart (INIT-REBOOT)
    Reboot,
    /// extends the lease at its server (RENEWING, unicast)
    Renew,
    /// extends the lease at any server (REBINDING, broadcast)
    Rebind,
}

/// why a relay agent dropped a DHCP message
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayDrop {
    /// more relay agents than allowed on the way
    Hops,
    /// no helper address on the interface
    NoHelper,
    /// the interface has no address for giaddr
    NoAddress,
    /// a reply whose giaddr is no address of this device
    NotMine,
}

/// a span of time as people read it ("1,250 s", "3,200 ms", "450 µs", "12 ns"), with a sign
pub fn fmt_ns(ns: i64) -> String {
    let (sign, a) = if ns < 0 { ("-", ns.unsigned_abs()) } else { ("", ns as u64) };
    if a >= 1_000_000_000 {
        format!("{}{},{:03} s", sign, a / 1_000_000_000, a % 1_000_000_000 / 1_000_000)
    } else if a >= 1_000_000 {
        format!("{}{},{:03} ms", sign, a / 1_000_000, a % 1_000_000 / 1000)
    } else if a >= 1000 {
        format!("{}{} \u{b5}s", sign, a / 1000)
    } else {
        format!("{}{} ns", sign, a)
    }
}

impl Note {
    /// 1: cable / port, 2: bridge and ARP, 3: IPv4 and ICMP, 4: UDP / TCP, 7: services
    pub fn layer(&self) -> u8 {
        match self {
            Note::LinkUp { .. } | Note::LinkDown { .. } | Note::DropNoLink { .. } | Note::DropQueueFull { .. } | Note::LostInFlight { .. } | Note::Repeated { .. } => 1,
            Note::IpLocal { .. }
            | Note::IpRouted { .. }
            | Note::IpNoRoute { .. }
            | Note::IpTtlExpired { .. }
            | Note::IpNotForMe { .. }
            | Note::IpMartian { .. }
            | Note::IpDirectedBroadcast { .. }
            | Note::IpBadChecksum
            | Note::IpMalformed
            | Note::IpFragmented { .. }
            | Note::IpFragNeeded { .. }
            | Note::IpReassembled { .. }
            | Note::IpReasmTimeout
            | Note::EchoAnswered { .. }
            | Note::IcmpSent { .. }
            | Note::IcmpLimited => 3,
            Note::UdpNoPort { .. } | Note::TcpNoListener { .. } => 4,
            Note::UdpToSocket { .. } | Note::PortInUse { .. } => 4,
            n if n.is_tcp() => 4,
            Note::DhcpDiscover { .. }
            | Note::DhcpOffered { .. }
            | Note::DhcpRequest { .. }
            | Note::DhcpAcked { .. }
            | Note::DhcpNaked { .. }
            | Note::DhcpProbe { .. }
            | Note::DhcpConflict { .. }
            | Note::DhcpBound { .. }
            | Note::DhcpExpired { .. }
            | Note::DhcpReleased { .. }
            | Note::DhcpNoOffers { .. }
            | Note::DhcpdOffer { .. }
            | Note::DhcpdPing { .. }
            | Note::DhcpdPingConflict { .. }
            | Note::DhcpdAck { .. }
            | Note::DhcpdNak { .. }
            | Note::DhcpdNoPool { .. }
            | Note::DhcpdExhausted { .. }
            | Note::DhcpdDecline { .. }
            | Note::DhcpdRelease { .. }
            | Note::DhcpdOtherServer { .. }
            | Note::DhcpdSilent { .. }
            | Note::DhcpdInform { .. }
            | Note::DhcpRelayed { .. }
            | Note::DhcpRelayedBack { .. }
            | Note::DhcpRelayDrop { .. }
            | Note::DnsAnswer { .. }
            | Note::DnsNxDomain { .. }
            | Note::DnsNoData { .. }
            | Note::DnsReferral { .. }
            | Note::DnsRefused { .. }
            | Note::DnsForward { .. }
            | Note::ResolveQuery { .. }
            | Note::ResolveAnswer { .. }
            | Note::ResolveNegative { .. }
            | Note::ResolveTimeout { .. }
            | Note::ResolveFailed { .. }
            | Note::SyslogSent { .. }
            | Note::SyslogStored { .. }
            | Note::NtpRequest { .. }
            | Note::NtpSample { .. }
            | Note::NtpStep { .. }
            | Note::NtpSlew { .. }
            | Note::NtpServed { .. }
            | Note::NtpNoTime { .. } => 7,
            n if n.is_http() => 7,
            Note::FaultStart { .. } | Note::FaultEnd { .. } => 1,
            Note::PartitionDrop { .. } => 3,
            n if n.is_lab() => 7,
            Note::ScriptBusy { .. } | Note::ScriptDropped { .. } | Note::ScriptError { .. } => 7,
            n if n.is_rip() => 7,
            Note::IpGroupIgnored { .. } => 3,
            n if n.is_filter() => 3,
            Note::V6(_) => 3,
            n if n.is_ospf() => 3,
            _ => 2,
        }
    }
    /// a decision of RIP (rip.rs)
    pub fn is_rip(&self) -> bool {
        matches!(
            self,
            Note::RipRequest { .. }
                | Note::RipAsked { .. }
                | Note::RipSent { .. }
                | Note::RipHeard { .. }
                | Note::RipIgnored { .. }
                | Note::RipBadEntry { .. }
                | Note::RipLearned { .. }
                | Note::RipChanged { .. }
                | Note::RipPathAdded { .. }
                | Note::RipDown { .. }
                | Note::RipPathGone { .. }
                | Note::RipHeld { .. }
                | Note::RipFlushed { .. }
                | Note::RipLocal { .. }
                | Note::RipLocalGone { .. }
        )
    }
    /// a decision of an access list or of NAT (acl.rs, nat.rs)
    pub fn is_filter(&self) -> bool {
        matches!(self, Note::AclPermit { .. } | Note::AclDeny { .. }) || self.is_nat()
    }
    /// a decision of NAT (nat.rs)
    pub fn is_nat(&self) -> bool {
        matches!(
            self,
            Note::NatCreate { .. }
                | Note::NatTranslate { .. }
                | Note::NatQuote { .. }
                | Note::NatExpire { .. }
                | Note::NatNoEntry { .. }
                | Note::NatNoRule { .. }
                | Note::NatExhausted { .. }
                | Note::NatErrorDropped
                | Note::NatFragWait { .. }
                | Note::NatFragDropped { .. }
        )
    }
    /// a decision of the lab's services (lb.rs, kv.rs, cache.rs, mq.rs)
    pub fn is_lab(&self) -> bool {
        matches!(
            self,
            Note::LbPick { .. }
                | Note::LbNoBackend
                | Note::LbRetry { .. }
                | Note::LbHealth { .. }
                | Note::LbDrained { .. }
                | Note::KvCoordinate { .. }
                | Note::KvForward { .. }
                | Note::KvWrite { .. }
                | Note::KvQuorum { .. }
                | Note::KvReplica { .. }
                | Note::KvReadRepair { .. }
                | Note::KvMember { .. }
                | Note::DiskFull
                | Note::CacheHit { .. }
                | Note::CacheMiss { .. }
                | Note::MqPublish { .. }
                | Note::MqDeliver { .. }
                | Note::MqAck { .. }
                | Note::MqRedeliver { .. }
                | Note::MqDead { .. }
        )
    }
    /// an HTTP decision (http.rs, httpsvc.rs)
    pub fn is_http(&self) -> bool {
        matches!(
            self,
            Note::HttpRequest { .. }
                | Note::HttpAnswered { .. }
                | Note::HttpQueued { .. }
                | Note::HttpOverload
                | Note::HttpInjected { .. }
                | Note::HttpBadRequest { .. }
                | Note::HttpIdleClose { .. }
                | Note::HttpReply { .. }
                | Note::HttpFailed { .. }
        )
    }
    /// something was thrown away
    pub fn is_drop(&self) -> bool {
        matches!(
            self,
            Note::DropNoLink { .. }
                | Note::DropQueueFull { .. }
                | Note::LostInFlight { .. }
                | Note::DropTagged { .. }
                | Note::DropVlan { .. }
                | Note::DropMalformed { .. }
                | Note::Filtered { .. }
                | Note::NotForMe { .. }
                | Note::ArpFailed { .. }
                | Note::ArpIgnored { .. }
                | Note::ArpIncomplete { .. }
                | Note::IpNoRoute { .. }
                | Note::IpTtlExpired { .. }
                | Note::IpNotForMe { .. }
                | Note::IpMartian { .. }
                | Note::IpDirectedBroadcast { .. }
                | Note::IpBadChecksum
                | Note::IpMalformed
                | Note::IpFragNeeded { .. }
                | Note::IpReasmTimeout
                | Note::IcmpLimited
                | Note::PortInUse { .. }
                | Note::DhcpdNoPool { .. }
                | Note::DhcpdSilent { .. }
                | Note::DhcpRelayDrop { .. }
                | Note::DnsRefused { .. }
                | Note::ResolveTimeout { .. }
                | Note::NtpNoTime { .. }
                | Note::TcpSynDropped { .. }
                | Note::TcpAckDropped { .. }
                | Note::TcpUnacceptable { .. }
                | Note::TcpChallenge { .. }
                | Note::PartitionDrop { .. }
                | Note::LbNoBackend
                | Note::DiskFull
                | Note::MqDead { .. }
                | Note::ScriptDropped { .. }
                | Note::StpDropped { .. }
                | Note::StpBlocked { .. }
                | Note::StpErrDisable { .. }
                | Note::RipIgnored { .. }
                | Note::IpGroupIgnored { .. }
                | Note::AclDeny { .. }
                | Note::NatExhausted { .. }
                | Note::NatErrorDropped
                | Note::NatFragDropped { .. }
                | Note::OspfIgnored { .. }
                | Note::OspfLsaDropped { .. }
        ) || matches!(self, Note::V6(n) if n.is_drop())
            || matches!(self, Note::Wlan(n) if n.is_drop())
    }
    /// a TCP decision (tcp.rs)
    pub fn is_tcp(&self) -> bool {
        matches!(
            self,
            Note::TcpNoListener { .. }
                | Note::TcpStateChange { .. }
                | Note::TcpNoConn { .. }
                | Note::TcpSynDropped { .. }
                | Note::TcpAckDropped { .. }
                | Note::TcpSynAgain
                | Note::TcpUnacceptable { .. }
                | Note::TcpChallenge { .. }
                | Note::TcpReset { .. }
                | Note::TcpRstSent { .. }
                | Note::TcpReceived { .. }
                | Note::TcpOutOfOrder { .. }
                | Note::TcpGapFilled { .. }
                | Note::TcpDuplicate { .. }
                | Note::TcpAckDelayed { .. }
                | Note::TcpDelayedAck
                | Note::TcpZeroWindow
                | Note::TcpWindowUpdate { .. }
                | Note::TcpAcked { .. }
                | Note::TcpDupAck { .. }
                | Note::TcpFastRetransmit { .. }
                | Note::TcpRecoveryEnd { .. }
                | Note::TcpRto { .. }
                | Note::TcpNagle { .. }
                | Note::TcpPersist { .. }
                | Note::TcpProbe { .. }
                | Note::TcpKeepalive { .. }
                | Note::TcpGiveUp { .. }
                | Note::TcpPmtu { .. }
                | Note::TcpPtb { .. }
        )
    }
    /// a German sentence; `port` names a port index, `iface` an IPv4 interface of the
    /// deciding device
    pub fn describe(&self, port: &dyn Fn(u16) -> String, iface: &dyn Fn(u16) -> String) -> String {
        self.message(port, iface).text()
    }
    /// the sentence as a message for the app's catalogue (describe() is its German text)
    pub fn message(&self, port: &dyn Fn(u16) -> String, iface: &dyn Fn(u16) -> String) -> Msg {
        match self {
            Note::LinkUp { port: p } => msg!("Leitung an {} ist aktiv.", port(*p)),
            Note::LinkDown { port: p } => msg!("Leitung an {} ist unterbrochen.", port(*p)),
            Note::DropNoLink { port: p } => msg!("Verworfen: an {} ist keine aktive Leitung.", port(*p)),
            Note::DropQueueFull { port: p } => msg!("Verworfen: die Warteschlange von {} ist voll.", port(*p)),
            Note::LostInFlight { port: p } => msg!("Verloren: die Leitung an {} fiel aus, während der Frame unterwegs war.", port(*p)),
            Note::Learned { mac, port: p, vlan } => msg!("MAC {} an {} gelernt (VLAN {}).", mac, port(*p), vlan),
            Note::Moved { mac, from, to, vlan } => msg!("MAC {} ist von {} nach {} umgezogen (VLAN {}).", mac, port(*from), port(*to), vlan),
            Note::TableFull { vlan } => msg!("MAC-Tabelle voll: nichts gelernt (VLAN {}).", vlan),
            Note::Forwarded { vlan, port: p } => msg!("Ziel bekannt: weitergeleitet an {} (VLAN {}).", port(*p), vlan),
            Note::Flooded { vlan, ports: 1 } => msg!("Ziel unbekannt oder Gruppe: an einen Port geflutet (VLAN {}).", vlan),
            Note::Flooded { vlan, ports } => msg!("Ziel unbekannt oder Gruppe: an {} Ports geflutet (VLAN {}).", ports, vlan),
            Note::Filtered { vlan } => msg!("Gefiltert: das Ziel liegt hinter dem Eingangsport (VLAN {}).", vlan),
            Note::DropTagged { port: p, vid } => msg!("Verworfen: Access-Port {} nimmt keine Frames mit Tag {} an.", port(*p), vid),
            Note::DropVlan { port: p, vid } => msg!("Verworfen: VLAN {} ist an {} nicht erlaubt.", vid, port(*p)),
            Note::DropMalformed { port: p } => msg!("Verworfen: unlesbarer Frame an {}.", port(*p)),
            Note::Consumed { port: p } => msg!("Steuer-Frame an {} für die Bridge selbst.", port(*p)),
            Note::Repeated { ports: 1 } => msg!("Hub: an einen Port wiederholt."),
            Note::Repeated { ports } => msg!("Hub: an {} Ports wiederholt.", ports),
            Note::Delivered { port: p } => msg!("Empfangen an {}.", port(*p)),
            Note::NotForMe { port: p } => msg!("Verworfen an {}: nicht für dieses Gerät.", port(*p)),
            Note::ArpRequest { iface: i, ip } => msg!("ARP-Anfrage an {}: Wer hat {}?", iface(*i), ip),
            Note::ArpReply { iface: i, ip, mac } => msg!("ARP-Antwort an {}: {} ist {}.", iface(*i), ip, mac),
            Note::ArpLearned { ip, mac } => msg!("ARP-Eintrag {} \u{2192} {}.", ip, mac),
            Note::ArpWaiting { ip } => msg!("Wartet auf die ARP-Antwort von {}.", ip),
            Note::ArpIncomplete { ip } => msg!("Verworfen: die MAC von {} ist noch unbekannt (ARP fragt).", ip),
            Note::ArpProxied { iface: i, ip } => msg!("Proxy-ARP an {}: antwortet für {}, das hinter einem anderen Interface liegt.", iface(*i), ip),
            Note::ArpFailed { ip } => msg!("{} antwortet nicht auf ARP: wartende Pakete verworfen.", ip),
            Note::ArpIgnored { ip } => msg!("ARP-Anfrage nach {} ist nicht für dieses Gerät: verworfen.", ip),
            Note::IpLocal { proto } => msg!("IPv4-Paket für dieses Gerät ({}).", netpkt::ipv4::proto_name(*proto).unwrap_or("?")),
            Note::IpRouted { dst, gw, iface: i } => {
                if dst == gw {
                    msg!("Route zu {}: direkt angeschlossen an {}.", dst, iface(*i))
                } else {
                    msg!("Route zu {}: über {} an {}.", dst, gw, iface(*i))
                }
            }
            Note::IpNoRoute { dst } => msg!("Keine Route zu {}.", dst),
            Note::IpTtlExpired { src } => msg!("TTL abgelaufen: Paket von {} verworfen.", src),
            Note::IpNotForMe { dst } => msg!("Paket an {} verworfen: nicht für dieses Gerät, es leitet nicht weiter.", dst),
            Note::IpMartian { src, dst } => msg!("Verworfen: {} an {} darf nicht über ein Kabel kommen.", src, dst),
            Note::IpDirectedBroadcast { dst } => msg!("Broadcast an {} nicht weitergeleitet (gerichtete Broadcasts bleiben draußen).", dst),
            Note::IpBadChecksum => msg!("Verworfen: Prüfsumme falsch."),
            Note::IpMalformed => msg!("Verworfen: unlesbares IPv4-Paket."),
            Note::IpFragmented { parts, mtu } => msg!("Fragmentiert in {} Teile (MTU {}).", parts, mtu),
            Note::IpFragNeeded { mtu } => msg!("Zu groß für die MTU {} und \u{201e}nicht fragmentieren\u{201c} gesetzt.", mtu),
            Note::IpReassembled { bytes } => msg!("Fragmente zu {} Bytes zusammengesetzt.", bytes),
            Note::IpReasmTimeout => msg!("Fragmente nach 30 s verworfen: nicht vollständig."),
            Note::EchoAnswered { to } => msg!("Echo-Anfrage beantwortet an {}.", to),
            Note::IcmpSent { typ, code, to } => msg!("ICMP an {}: {}.", to, netpkt::icmp::name(*typ, *code)),
            Note::IcmpLimited => msg!("ICMP-Meldung unterdrückt (Ratenbegrenzung)."),
            Note::UdpNoPort { port: p } => msg!("UDP an Port {}: niemand lauscht.", p),
            Note::TcpNoListener { port: p } => msg!("TCP an Port {}: niemand lauscht, Antwort RST.", p),
            Note::UdpToSocket { port: p, to } => msg!("UDP an Port {}: an {}.", p, to),
            Note::PortInUse { port: p, svc } => msg!("Port {} ist schon belegt: {} startet nicht.", p, svc),
            Note::DhcpDiscover { iface: i, xid, secs } => msg!("DHCPDISCOVER über {} (xid {}, seit {} s).", iface(*i), format!("0x{:08x}", xid), secs),
            Note::DhcpOffered { ip, server } => msg!("DHCPOFFER: {} von {}.", ip, server),
            Note::DhcpRequest { iface: i, ip, kind } => match kind {
                DhcpReq::Select => msg!("DHCPREQUEST über {}: nimmt das Angebot {} an.", iface(*i), ip),
                DhcpReq::Reboot => msg!("DHCPREQUEST über {}: möchte {} nach dem Neustart weiter nutzen.", iface(*i), ip),
                DhcpReq::Renew => msg!("DHCPREQUEST über {}: verlängert {} beim eigenen Server.", iface(*i), ip),
                DhcpReq::Rebind => msg!("DHCPREQUEST über {}: verlängert {} bei jedem Server (Rebind).", iface(*i), ip),
            },
            Note::DhcpAcked { ip, server, lease } => msg!("DHCPACK: {} für {} s von {}.", ip, lease, server),
            Note::DhcpNaked { server } => msg!("DHCPNAK von {}: alles von vorn.", server),
            Note::DhcpProbe { ip } => msg!("ARP-Probe: benutzt schon jemand {}?", ip),
            Note::DhcpConflict { ip, mac } => msg!("{} wird schon von {} benutzt: DHCPDECLINE.", ip, mac),
            Note::DhcpBound { iface: i, addr, router, renew } => match router {
                Some(r) => msg!("{} hat {} (Router {}); Verlängerung in {} s.", iface(*i), addr, r, renew),
                None => msg!("{} hat {}; Verlängerung in {} s.", iface(*i), addr, renew),
            },
            Note::DhcpExpired { ip } => msg!("Lease für {} abgelaufen: Adresse entfernt.", ip),
            Note::DhcpReleased { ip, server } => msg!("DHCPRELEASE: {} an {} zurückgegeben.", ip, server),
            Note::DhcpNoOffers { iface: i, retry } => msg!("Kein DHCPOFFER über {}: neuer Versuch in {} s.", iface(*i), retry),
            Note::DhcpdOffer { ip, mac } => msg!("DHCPOFFER: {} für {}.", ip, mac),
            Note::DhcpdPing { ip } => msg!("Ping-Prüfung: ist {} frei?", ip),
            Note::DhcpdPingConflict { ip } => msg!("{} antwortet auf den Ping: als Konflikt gesperrt.", ip),
            Note::DhcpdAck { ip, mac, lease } => msg!("DHCPACK: {} für {}, {} s.", ip, mac, lease),
            Note::DhcpdNak { ip, mac } => msg!("DHCPNAK: {} passt nicht für {}.", ip, mac),
            Note::DhcpdNoPool { net } => msg!("Kein Pool für das Netz von {}: keine Antwort.", net),
            Note::DhcpdExhausted { pool } => msg!("Pool {} hat keine freie Adresse mehr.", pool),
            Note::DhcpdDecline { ip, mac } => msg!("DHCPDECLINE von {}: {} als Konflikt gesperrt.", mac, ip),
            Note::DhcpdRelease { ip } => msg!("DHCPRELEASE: {} ist wieder frei.", ip),
            Note::DhcpdOtherServer { mac, server } => msg!("{} nimmt das Angebot von {}: eigenes Angebot zurückgezogen.", mac, server),
            Note::DhcpdSilent { ip, mac } => msg!("Nichts über {} für {} bekannt: keine Antwort.", ip, mac),
            Note::DhcpdInform { to } => msg!("DHCPINFORM von {}: Einstellungen geschickt.", to),
            Note::DhcpRelayed { to, giaddr } => msg!("DHCP-Anfrage an {} weitergeleitet (giaddr {}).", to, giaddr),
            Note::DhcpRelayedBack { to, iface: i } => msg!("DHCP-Antwort an {} über {} weitergegeben.", to, iface(*i)),
            Note::DhcpRelayDrop { why } => match why {
                RelayDrop::Hops => msg!("DHCP verworfen: zu viele Relay-Hops."),
                RelayDrop::NoHelper => msg!("DHCP verworfen: an diesem Interface ist kein DHCP-Server eingetragen (helper-address)."),
                RelayDrop::NoAddress => msg!("DHCP verworfen: das Interface hat keine IPv4-Adresse für giaddr."),
                RelayDrop::NotMine => msg!("DHCP verworfen: giaddr gehört nicht zu diesem Gerät."),
            },
            Note::DnsAnswer { name, qtype, n } => {
                if *n == 1 {
                    msg!("DNS: {} {} beantwortet ({} Eintrag).", name, netpkt::dns::type_name(*qtype), n)
                } else {
                    msg!("DNS: {} {} beantwortet ({} Einträge).", name, netpkt::dns::type_name(*qtype), n)
                }
            }
            Note::DnsNxDomain { name } => msg!("DNS: {} gibt es nicht (NXDOMAIN).", name),
            Note::DnsNoData { name, qtype } => msg!("DNS: {} hat keinen {}-Eintrag.", name, netpkt::dns::type_name(*qtype)),
            Note::DnsReferral { name, zone } => msg!("DNS: {} liegt in der delegierten Zone {}: Verweis.", name, zone),
            Note::DnsRefused { name } => msg!("DNS: für {} nicht zuständig (REFUSED).", name),
            Note::DnsForward { name } => msg!("DNS: {} an den nächsten Server weitergereicht.", name),
            Note::ResolveQuery { name, qtype, server } => msg!("Frage {} nach {} ({}).", server, name, netpkt::dns::type_name(*qtype)),
            Note::ResolveAnswer { name, addr, cached } => match (addr, *cached) {
                (Some(a), true) => msg!("{} ist {} (aus dem Cache).", name, a),
                (Some(a), false) => msg!("{} ist {}.", name, a),
                (None, true) => msg!("{} aufgelöst (aus dem Cache).", name),
                (None, false) => msg!("{} aufgelöst.", name),
            },
            Note::ResolveNegative { name, nx } => {
                if *nx {
                    msg!("{} gibt es nicht (NXDOMAIN).", name)
                } else {
                    msg!("{} hat keinen passenden Eintrag.", name)
                }
            }
            Note::ResolveTimeout { name, server } => msg!("{} antwortet nicht auf die Frage nach {}.", server, name),
            Note::ResolveFailed { name, why } => msg!("{} nicht aufgelöst: {}.", name, phrase(why.text())),
            Note::SyslogSent { to, sev } => msg!("Syslog-Meldung ({}) an {}.", netpkt::syslog::severity_name(*sev), to),
            Note::SyslogStored { from, sev } => msg!("Syslog-Meldung ({}) von {} gespeichert.", netpkt::syslog::severity_name(*sev), from),
            Note::NtpRequest { server } => msg!("NTP-Anfrage an {}.", server),
            Note::NtpSample { server, offset, delay } => msg!("NTP-Antwort von {}: Abweichung {}, Laufzeit {}.", server, dur(*offset), dur(*delay)),
            Note::NtpStep { offset } => msg!("Uhr um {} gestellt.", dur(*offset)),
            Note::NtpSlew { offset } => msg!("Uhr wird um {} nachgeführt.", dur(*offset)),
            Note::NtpServed { to, stratum } => msg!("NTP-Zeit an {} (Stratum {}).", to, stratum),
            Note::NtpNoTime { to } => msg!("NTP-Anfrage von {} unbeantwortet: die eigene Uhr ist nicht synchronisiert.", to),
            Note::TcpStateChange { port: p, peer, pport, from, to } => {
                // an IPv6 peer in brackets before its port
                let who = if peer.is_v4() { format!("{}", peer) } else { format!("[{}]", peer) };
                if *pport == 0 {
                    msg!("TCP Port {}: {} \u{2192} {}.", p, from.name(), to.name())
                } else {
                    msg!("TCP {} \u{2194} {}:{}: {} \u{2192} {}.", p, who, pport, from.name(), to.name())
                }
            }
            Note::TcpNoConn { port: p } => msg!("TCP an Port {}: keine solche Verbindung, Antwort RST.", p),
            Note::TcpSynDropped { port: p } => msg!("Warteschlange von Port {} voll: SYN verworfen.", p),
            Note::TcpAckDropped { port: p } => msg!("Warteschlange von Port {} voll: das letzte ACK des Handshakes wird ignoriert.", p),
            Note::TcpSynAgain => msg!("Der SYN kam noch einmal: SYN-ACK wiederholt."),
            Note::TcpUnacceptable { seq, len } => msg!("Segment außerhalb des Empfangsfensters (Seq {}, Länge {}): ACK zurück, verworfen.", seq, len),
            Note::TcpChallenge { syn } => msg!("{} im Fenster, aber nicht an der erwarteten Stelle: Challenge-ACK (RFC 5961).", if *syn { "SYN" } else { "RST" }),
            Note::TcpReset { refused } => {
                if *refused {
                    msg!("RST empfangen: Verbindung abgelehnt.")
                } else {
                    msg!("RST empfangen: Verbindung zurückgesetzt.")
                }
            }
            Note::TcpRstSent { why } => msg!("RST gesendet: {}.", phrase(why.text())),
            Note::TcpReceived { bytes } => msg!("{} Bytes in Reihenfolge angenommen.", bytes),
            Note::TcpOutOfOrder { seq, bytes } => msg!("Außer der Reihe (Seq {}, {} Bytes): zurückgehalten, sofortige Bestätigung.", seq, bytes),
            Note::TcpGapFilled { bytes } => msg!("Lücke geschlossen: {} Bytes aus der Warteschlange nachgereicht.", bytes),
            Note::TcpDuplicate { bytes } => msg!("{} Bytes kamen doppelt: sofortige Bestätigung.", bytes),
            Note::TcpAckDelayed { delay } => msg!("Bestätigung verzögert um {}.", dur(*delay as i64)),
            Note::TcpDelayedAck => msg!("Verzögerte Bestätigung gesendet."),
            Note::TcpZeroWindow => msg!("Empfangspuffer voll: Fenster 0."),
            Note::TcpWindowUpdate { wnd } => msg!("Fenster wieder offen: {} Bytes.", wnd),
            Note::TcpAcked { bytes, cwnd, rtt } => match rtt {
                Some(r) => msg!("{} Bytes bestätigt, RTT {}, cwnd {} Bytes.", bytes, dur(*r as i64), cwnd),
                None => msg!("{} Bytes bestätigt, cwnd {} Bytes.", bytes, cwnd),
            },
            Note::TcpDupAck { n } => msg!("Doppelte Bestätigung Nr. {}.", n),
            Note::TcpFastRetransmit { seq, ssthresh, cwnd } => msg!("Drei doppelte Bestätigungen: Fast Retransmit ab Seq {}, ssthresh {}, cwnd {} Bytes.", seq, ssthresh, cwnd),
            Note::TcpRecoveryEnd { cwnd } => msg!("Fast Recovery beendet: cwnd {} Bytes.", cwnd),
            Note::TcpRto { seq, rto, n } => msg!("Zeitüberschreitung: Neuübertragung ab Seq {} (Versuch {}), nächstes RTO {}.", seq, n, dur(*rto as i64)),
            Note::TcpNagle { bytes } => msg!("Nagle: {} Bytes warten auf die Bestätigung der offenen Daten.", bytes),
            Note::TcpPersist { after } => msg!("Fenster zu klein, nichts unterwegs: Fenstersonde in {}.", dur(*after as i64)),
            Note::TcpProbe { n } => msg!("Fenstersonde Nr. {}.", n),
            Note::TcpKeepalive { n } => msg!("Keepalive-Probe Nr. {}.", n),
            Note::TcpGiveUp { why } => msg!("Verbindung aufgegeben: {}.", phrase(why.text())),
            Note::TcpPmtu { mss } => msg!("Fragmentierung nötig: kleinere Segmente, MSS {}.", mss),
            Note::HttpRequest { method, path } => msg!("HTTP-Anfrage {} {}.", method, path),
            Note::HttpAnswered { status, bytes } => msg!("HTTP-Antwort {} {} ({} Bytes).", status, netpkt::http::reason(*status), bytes),
            Note::HttpQueued { waiting } => msg!("Alle Worker beschäftigt: die Anfrage wartet ({} vor ihr).", waiting),
            Note::HttpOverload => msg!("Warteschlange voll: sofort 503."),
            Note::HttpInjected { status } => msg!("Fehlerquote: Antwort {} statt der Route.", status),
            Note::HttpBadRequest { status } => msg!("Anfrage nicht lesbar: {}, die Verbindung wird geschlossen.", status),
            Note::HttpIdleClose { header } => {
                if *header {
                    msg!("Kein vollständiger Anfragekopf in der Zeit: Verbindung geschlossen.")
                } else {
                    msg!("Keep-Alive-Verbindung zu lange ungenutzt: geschlossen.")
                }
            }
            Note::HttpReply { status, ms } => msg!("HTTP-Antwort {} nach {} ms.", status, ms),
            Note::HttpFailed { timeout } => {
                if *timeout {
                    msg!("HTTP-Anfrage: Zeitüberschreitung.")
                } else {
                    msg!("HTTP-Anfrage gescheitert.")
                }
            }
            Note::FaultStart { id, what } => msg!("Fehler {} beginnt: {}.", id, what),
            Note::FaultEnd { id, what } => msg!("Fehler {} behoben: {}.", id, what),
            Note::PartitionDrop { src } => msg!("Partition: Paket von {} verworfen.", src),
            Note::LbPick { backend, algo } => msg!("Loadbalancer: Backend {} gewählt ({}).", backend, algo),
            Note::LbNoBackend => msg!("Loadbalancer: kein Backend verfügbar."),
            Note::LbRetry { backend } => msg!("Loadbalancer: {} nicht erreichbar, nächster Versuch.", backend),
            Note::LbHealth { backend, up } => {
                if *up {
                    msg!("Health Check: {} ist wieder verfügbar.", backend)
                } else {
                    msg!("Health Check: {} fällt aus.", backend)
                }
            }
            Note::LbDrained { backend } => msg!("{} ist leer gelaufen und entfernt.", backend),
            Note::KvCoordinate { key, op, replicas } => msg!("KV: {} {} koordiniert ({} Replikate).", op, key, replicas),
            Note::KvForward { key, to } => msg!("KV: Schreiben von {} an den Leader {} weitergereicht.", key, to),
            Note::KvWrite { key, version, need } => msg!("KV: {} als Version {} gespeichert, wartet auf {} Bestätigungen.", key, version, need),
            Note::KvQuorum { key, got, need, ok } => {
                if *ok {
                    msg!("KV: Quorum für {} erreicht ({} von {}).", key, got, need)
                } else {
                    msg!("KV: Quorum für {} verfehlt ({} von {}).", key, got, need)
                }
            }
            Note::KvReplica { key, version, stored } => {
                if *stored {
                    msg!("KV: Replikat von {} auf Version {} gebracht.", key, version)
                } else {
                    msg!("KV: Replikat von {} hat schon Version {} oder neuer.", key, version)
                }
            }
            Note::KvReadRepair { key, to } => msg!("KV: Read Repair von {} an {}.", key, to),
            Note::KvMember { member, up } => {
                if *up {
                    msg!("KV: {} antwortet wieder.", member)
                } else {
                    msg!("KV: {} antwortet nicht mehr und gilt als ausgefallen.", member)
                }
            }
            Note::DiskFull => msg!("Disk voll: Schreiben abgelehnt."),
            Note::CacheHit { key } => msg!("Cache: Treffer für {}.", key),
            Note::CacheMiss { key } => msg!("Cache: kein Treffer für {}, frage das Backend.", key),
            Note::MqPublish { topic, id } => msg!("Queue {}: Nachricht {} angenommen.", topic, id),
            Note::MqDeliver { topic, id, n } => msg!("Queue {}: Nachricht {} ausgeliefert ({}. Zustellung).", topic, id, n),
            Note::MqAck { topic, id } => msg!("Queue {}: Nachricht {} bestätigt.", topic, id),
            Note::MqRedeliver { topic, id } => msg!("Queue {}: Nachricht {} nicht bestätigt, wieder in der Warteschlange.", topic, id),
            Note::MqDead { topic, id } => msg!("Queue {}: Nachricht {} zu oft zugestellt, aussortiert.", topic, id),
            Note::ScriptBusy { svc, waiting } => msg!("Skript {} rechnet noch: das Ereignis wartet ({} vor ihm).", svc, waiting),
            Note::ScriptDropped { svc } => msg!("Skript {} überlastet: Datagramm verworfen.", svc),
            Note::ScriptError { svc, line, col, msg: text } => msg!("Skript {}: Fehler in Zeile {}, Spalte {}: {}", svc, line, col, text),
            Note::StpRx { port: p, bpdu } => match bpdu.kind {
                netpkt::stp::Kind::Tcn => msg!("TCN-BPDU an {} empfangen.", port(*p)),
                _ => msg!("BPDU an {}: {} von {}, Root {}, Kosten {}{}.", port(*p), bpdu_kind(bpdu), stp_id(bpdu.bridge), stp_id(bpdu.root), bpdu.cost, bpdu_flags(bpdu)),
            },
            Note::StpTx { port: p, bpdu } => match bpdu.kind {
                netpkt::stp::Kind::Tcn => msg!("TCN-BPDU über {} an die Root-Bridge gesendet.", port(*p)),
                _ => msg!("BPDU über {}: {}, Root {}, Kosten {}{}.", port(*p), bpdu_kind(bpdu), stp_id(bpdu.root), bpdu.cost, bpdu_flags(bpdu)),
            },
            Note::StpDropped { port: p, why } => match why {
                crate::stp::BpduDrop::Type => msg!("BPDU an {} verworfen: diese Bridge spricht 802.1D (STP) und kennt RST-BPDUs nicht.", port(*p)),
                crate::stp::BpduDrop::Aged => msg!("BPDU an {} verworfen: ihr Message Age hat Max Age erreicht.", port(*p)),
                crate::stp::BpduDrop::Looped => msg!("BPDU an {} verworfen: die eigene BPDU kam über eine Schleife zurück.", port(*p)),
            },
            Note::StpBlocked { port: p, state } => msg!("Verworfen: {} ist im Zustand {} (Spanning Tree).", port(*p), state.msg()),
            Note::StpRole { port: p, from, to } => msg!("Spanning Tree: {} ist jetzt {} (vorher {}).", port(*p), to.msg(), from.msg()),
            Note::StpState { port: p, from, to } => msg!("Spanning Tree: {} wechselt von {} nach {}.", port(*p), from.msg(), to.msg()),
            Note::StpRoot { root, port: None, .. } => msg!("Spanning Tree: diese Bridge ist die Root-Bridge ({}).", stp_id(*root)),
            Note::StpRoot { root, port: Some(p), cost } => msg!("Spanning Tree: die Root-Bridge ist {}, über {} mit Kosten {}.", stp_id(*root), port(*p), cost),
            Note::StpTopology { port: p, what } => match (what, p) {
                (crate::stp::TcWhat::Detected, Some(p)) => msg!("Topologieänderung an {} erkannt.", port(*p)),
                (crate::stp::TcWhat::Detected, None) => msg!("Topologieänderung erkannt."),
                (crate::stp::TcWhat::TcnSent, Some(p)) => msg!("Topologieänderung: TCN-BPDU über {} zur Root-Bridge.", port(*p)),
                (crate::stp::TcWhat::TcnSent, None) => msg!("Topologieänderung: TCN-BPDU zur Root-Bridge."),
                (crate::stp::TcWhat::TcnReceived, Some(p)) => msg!("Topologieänderung: TCN-BPDU an {}, wird bestätigt und weitergegeben.", port(*p)),
                (crate::stp::TcWhat::TcnReceived, None) => msg!("Topologieänderung: TCN-BPDU empfangen."),
                (crate::stp::TcWhat::Acked, Some(p)) => msg!("Topologieänderung: die TCN-BPDU über {} ist bestätigt.", port(*p)),
                (crate::stp::TcWhat::Acked, None) => msg!("Topologieänderung: die TCN-BPDU ist bestätigt."),
                (crate::stp::TcWhat::Received, Some(p)) => msg!("Topologieänderung gemeldet an {} (TC-Flag).", port(*p)),
                (crate::stp::TcWhat::Received, None) => msg!("Topologieänderung gemeldet (TC-Flag)."),
                (crate::stp::TcWhat::RootFlag(true), _) => msg!("Root-Bridge: ihre BPDUs tragen das TC-Flag für Max Age + Forward Delay."),
                (crate::stp::TcWhat::RootFlag(false), _) => msg!("Root-Bridge: die Topologieänderung ist vorbei, ihre BPDUs tragen kein TC-Flag mehr."),
            },
            Note::StpFlush { port: p, entries: 1 } => msg!("Topologieänderung: ein MAC-Eintrag an {} gelöscht.", port(*p)),
            Note::StpFlush { port: p, entries } => msg!("Topologieänderung: {} MAC-Einträge an {} gelöscht.", entries, port(*p)),
            Note::StpAging { secs, fast: true } => msg!("Topologieänderung: MAC-Einträge altern jetzt nach {} s (Forward Delay).", secs),
            Note::StpAging { secs, fast: false } => msg!("MAC-Einträge altern wieder nach {} s.", secs),
            Note::StpAged { port: p } => msg!("Spanning Tree: die Information an {} ist abgelaufen, keine BPDU in der Zeit.", port(*p)),
            Note::StpEdge { port: p, edge: true } => msg!("Spanning Tree: {} ist ein Edge-Port.", port(*p)),
            Note::StpEdge { port: p, edge: false } => msg!("Spanning Tree: an {} kam eine BPDU, er ist kein Edge-Port mehr.", port(*p)),
            Note::StpMigrate { port: p, rstp: false } => msg!("Spanning Tree: an {} spricht der Nachbar 802.1D, der Port sendet Configuration BPDUs.", port(*p)),
            Note::StpMigrate { port: p, rstp: true } => msg!("Spanning Tree: {} versucht wieder RSTP.", port(*p)),
            Note::StpErrDisable { port: p } => msg!("BPDU Guard: BPDU an {}, der Port wird abgeschaltet (err-disabled).", port(*p)),
            Note::RipRequest { iface: i, ver } => msg!("RIP: fragt über {} nach der ganzen Tabelle (Request, Version {}).", iface(*i), ver),
            Note::RipAsked { from, iface: i, whole } => {
                if *whole {
                    msg!("RIP: {} fragt über {} nach der ganzen Tabelle: Antwort folgt.", from, iface(*i))
                } else {
                    msg!("RIP: {} fragt über {} nach einzelnen Routen: Antwort folgt.", from, iface(*i))
                }
            }
            Note::RipSent { iface: i, ver, to, kind, routes, poisoned } => {
                let n = routes.len();
                match (kind, *poisoned) {
                    (RipOut::Regular, 0) => msg!("RIP: Update (Version {}) über {} an {}, Routen: {}.", ver, iface(*i), to, n),
                    (RipOut::Regular, p) => msg!("RIP: Update (Version {}) über {} an {}, Routen: {}, davon per Split Horizon vergiftet: {}.", ver, iface(*i), to, n, p),
                    (RipOut::Triggered, 0) => msg!("RIP: ausgelöstes Update (Version {}) über {} an {}, Routen: {}.", ver, iface(*i), to, n),
                    (RipOut::Triggered, p) => msg!("RIP: ausgelöstes Update (Version {}) über {} an {}, Routen: {}, davon per Split Horizon vergiftet: {}.", ver, iface(*i), to, n, p),
                    (RipOut::Answer, 0) => msg!("RIP: Antwort (Version {}) über {} an {}, Routen: {}.", ver, iface(*i), to, n),
                    (RipOut::Answer, p) => msg!("RIP: Antwort (Version {}) über {} an {}, Routen: {}, davon per Split Horizon vergiftet: {}.", ver, iface(*i), to, n, p),
                }
            }
            Note::RipHeard { from, iface: i, ver, routes } => msg!("RIP: Update (Version {}) von {} über {}, Routen: {}.", ver, from, iface(*i), routes.len()),
            Note::RipIgnored { from, iface: i, why } => msg!("RIP: Nachricht von {} über {} ignoriert: {}.", from, iface(*i), phrase(why.text())),
            Note::RipBadEntry { from, addr, metric } => msg!("RIP: Eintrag {} mit Metrik {} von {} ist ungültig: ignoriert.", addr, metric, from),
            Note::RipLearned { prefix, via, metric } => msg!("RIP: Route {} gelernt: über {}, Metrik {}.", prefix, via, metric),
            Note::RipChanged { prefix, via, metric, old } => msg!("RIP: Route {} jetzt über {}, Metrik {} (vorher {}).", prefix, via, metric, old),
            Note::RipPathAdded { prefix, via, metric } => msg!("RIP: ein weiterer gleich guter Weg zu {} über {} (Metrik {}).", prefix, via, metric),
            Note::RipDown { prefix, via, why } => msg!("RIP: {} über {} ist weg ({}): unerreichbar, wird mit Metrik 16 gemeldet.", prefix, via, why.msg()),
            Note::RipPathGone { prefix, via, why } => msg!("RIP: der Weg zu {} über {} entfällt ({}); gleich gute Wege bleiben.", prefix, via, why.msg()),
            Note::RipHeld { prefix, from, metric } => msg!("RIP: {} im Holddown: das Angebot von {} mit Metrik {} wird ignoriert.", prefix, from, metric),
            Note::RipFlushed { prefix } => msg!("RIP: {} aus der Tabelle gelöscht (Garbage Collection abgelaufen).", prefix),
            Note::RipLocal { prefix, how } => msg!("RIP: meldet {} ({}).", prefix, phrase(how.text())),
            Note::RipLocalGone { prefix } => msg!("RIP: {} gibt es hier nicht mehr: wird mit Metrik 16 gemeldet.", prefix),
            Note::IpGroupIgnored { dst } => msg!("Multicast an {} verworfen: kein Interface hört auf diese Gruppe.", dst),
            Note::AclPermit { list, seq, iface: i, out } => {
                if *out {
                    msg!("Zugriffsliste {} ausgehend an {}: Zeile {} erlaubt das Paket.", list, iface(*i), seq)
                } else {
                    msg!("Zugriffsliste {} eingehend an {}: Zeile {} erlaubt das Paket.", list, iface(*i), seq)
                }
            }
            Note::AclDeny { list, seq, iface: i, out } => match (*out, *seq) {
                (true, 0) => msg!("Zugriffsliste {} ausgehend an {}: keine Zeile passt, verworfen (implizites deny).", list, iface(*i)),
                (false, 0) => msg!("Zugriffsliste {} eingehend an {}: keine Zeile passt, verworfen (implizites deny).", list, iface(*i)),
                (true, s) => msg!("Zugriffsliste {} ausgehend an {}: Zeile {} verwirft das Paket.", list, iface(*i), s),
                (false, s) => msg!("Zugriffsliste {} eingehend an {}: Zeile {} verwirft das Paket.", list, iface(*i), s),
            },
            Note::NatCreate { proto, local, global, remote } => match remote {
                Some(r) => msg!("NAT: neue Übersetzung {} {} = {} (Gegenstelle {}).", proto_word(*proto), ep(*proto, *local), ep(*proto, *global), ep(*proto, *r)),
                None if *proto == 0 => msg!("NAT: {} bekommt {} für alle Protokolle.", local.0, global.0),
                None => msg!("NAT: neue Zuordnung {} {} = {}.", proto_word(*proto), ep(*proto, *local), ep(*proto, *global)),
            },
            Note::NatTranslate { proto, inbound, from, to } => {
                if *inbound {
                    msg!("NAT: Ziel {} \u{2192} {} ({}).", ep(*proto, *from), ep(*proto, *to), proto_word(*proto))
                } else {
                    msg!("NAT: Quelle {} \u{2192} {} ({}).", ep(*proto, *from), ep(*proto, *to), proto_word(*proto))
                }
            }
            Note::NatQuote { inbound, from, to } => {
                if *inbound {
                    msg!("NAT: ICMP-Fehler zurückübersetzt, zitierte Quelle {} \u{2192} {}.", ep(0, *from), ep(0, *to))
                } else {
                    msg!("NAT: ICMP-Fehler übersetzt, zitiertes Ziel {} \u{2192} {}.", ep(0, *from), ep(0, *to))
                }
            }
            Note::NatExpire { proto, local, global, remote } => match remote {
                Some(r) => msg!("NAT: Übersetzung {} {} = {} (Gegenstelle {}) abgelaufen.", proto_word(*proto), ep(*proto, *local), ep(*proto, *global), ep(*proto, *r)),
                None => msg!("NAT: Zuordnung {} = {} abgelaufen.", local.0, global.0),
            },
            Note::NatNoEntry { proto, dst } => msg!("NAT: keine Übersetzung für {} ({}): unverändert weiter.", ep(*proto, *dst), proto_word(*proto)),
            Note::NatNoRule { src } => msg!("NAT: keine Regel übersetzt {}: unverändert weiter.", src),
            Note::NatExhausted { what } => msg!("NAT: keine freie Adresse und kein freier Port ({}): verworfen.", what),
            Note::NatErrorDropped => msg!("NAT: ICMP-Fehler ohne passende Übersetzung verworfen."),
            Note::NatFragWait { id } => msg!("NAT: Fragment (ID {}) wartet auf das erste Fragment.", id),
            Note::NatFragDropped { id, parts } => msg!("NAT: {} Fragmente (ID {}) ohne erstes Fragment verworfen.", parts, id),
            Note::V6(n) => n.message(port, iface),
            n @ (Note::OspfRouterId { .. }
            | Note::OspfHelloSent { .. }
            | Note::OspfHelloRcvd { .. }
            | Note::OspfIgnored { .. }
            | Note::OspfIfState { .. }
            | Note::OspfDr { .. }
            | Note::OspfNbr { .. }
            | Note::OspfMaster { .. }
            | Note::OspfDd { .. }
            | Note::OspfLsr { .. }
            | Note::OspfLsu { .. }
            | Note::OspfAck { .. }
            | Note::OspfLsaNew { .. }
            | Note::OspfLsaOrig { .. }
            | Note::OspfLsaFlooded { .. }
            | Note::OspfLsaAcked { .. }
            | Note::OspfLsaFlush { .. }
            | Note::OspfLsaAged { .. }
            | Note::OspfLsaGone { .. }
            | Note::OspfLsaOlder { .. }
            | Note::OspfLsaDropped { .. }
            | Note::OspfSpf { .. }
            | Note::OspfRoute { .. }) => ospf_message(n, iface),
            Note::TcpPtb { mss } => msg!("Packet Too Big: kleinere Segmente, MSS {}.", mss),
            Note::Wlan(n) => n.message(port),
        }
    }
}

// ---------------------------------------------------------------- OSPF

impl Note {
    /// a decision of OSPF (ospf.rs)
    pub fn is_ospf(&self) -> bool {
        matches!(
            self,
            Note::OspfRouterId { .. }
                | Note::OspfHelloSent { .. }
                | Note::OspfHelloRcvd { .. }
                | Note::OspfIgnored { .. }
                | Note::OspfIfState { .. }
                | Note::OspfDr { .. }
                | Note::OspfNbr { .. }
                | Note::OspfMaster { .. }
                | Note::OspfDd { .. }
                | Note::OspfLsr { .. }
                | Note::OspfLsu { .. }
                | Note::OspfAck { .. }
                | Note::OspfLsaNew { .. }
                | Note::OspfLsaOrig { .. }
                | Note::OspfLsaFlooded { .. }
                | Note::OspfLsaAcked { .. }
                | Note::OspfLsaFlush { .. }
                | Note::OspfLsaAged { .. }
                | Note::OspfLsaGone { .. }
                | Note::OspfLsaOlder { .. }
                | Note::OspfLsaDropped { .. }
                | Note::OspfSpf { .. }
                | Note::OspfRoute { .. }
        )
    }
}

/// "DR 2.2.2.2" or "keiner"
fn ospf_who(id: Ipv4) -> Msg {
    if id.is_unspecified() { phrase("keiner") } else { Msg::plain(format!("{}", id)) }
}

/// the sentence of an OSPF decision
fn ospf_message(n: &Note, iface: &dyn Fn(u16) -> String) -> Msg {
    use crate::ospf::{LsuWhy, RtChange};
    let flags = |f: u8| if f == 0 { phrase("keine") } else { Msg::plain(netpkt::ospf::dd_flags_text(f)) };
    match n {
        Note::OspfRouterId { id } => msg!("OSPF: der Prozess arbeitet mit der Router-ID {}.", id),
        Note::OspfHelloSent { iface: i, nbrs: 0 } => msg!("OSPF: Hello über {} an 224.0.0.5, noch kein Nachbar gesehen.", iface(*i)),
        Note::OspfHelloSent { iface: i, nbrs } => msg!("OSPF: Hello über {} an 224.0.0.5, gesehene Nachbarn: {}.", iface(*i), nbrs),
        Note::OspfHelloRcvd { from, iface: i } => msg!("OSPF: Hello von {} über {}.", from, iface(*i)),
        Note::OspfIgnored { from, iface: i, why } => msg!("OSPF: Paket von {} über {} ignoriert: {}.", from, iface(*i), phrase(why.text())),
        Note::OspfIfState { iface: i, from, to } => msg!("OSPF: {} wechselt von {} nach {}.", iface(*i), from.name(), to.name()),
        Note::OspfDr { iface: i, dr, bdr } => msg!("OSPF: Wahl an {}: DR ist {}, BDR ist {}.", iface(*i), ospf_who(*dr), ospf_who(*bdr)),
        Note::OspfNbr { nbr, iface: i, from, to, ev } => msg!("OSPF: Nachbar {} an {}: {} \u{2192} {} ({}).", nbr, iface(*i), from.name(), to.name(), ev.name()),
        Note::OspfMaster { nbr, iface: i, master: true } => msg!("OSPF: Austausch mit {} an {}: dieser Router ist Master.", nbr, iface(*i)),
        Note::OspfMaster { nbr, iface: i, master: false } => msg!("OSPF: Austausch mit {} an {}: dieser Router ist Slave.", nbr, iface(*i)),
        Note::OspfDd { nbr, iface: i, out: true, flags: f, seq, lsas } => msg!("OSPF: Database Description an {} über {} (Flags {}, Nummer 0x{}, LSA-Köpfe: {}).", nbr, iface(*i), flags(*f), format!("{:08x}", seq), lsas),
        Note::OspfDd { nbr, iface: i, out: false, flags: f, seq, lsas } => msg!("OSPF: Database Description von {} über {} (Flags {}, Nummer 0x{}, LSA-Köpfe: {}).", nbr, iface(*i), flags(*f), format!("{:08x}", seq), lsas),
        Note::OspfLsr { nbr, iface: i, out: true, lsas } => msg!("OSPF: Link State Request an {} über {}, LSAs: {}.", nbr, iface(*i), lsas),
        Note::OspfLsr { nbr, iface: i, out: false, lsas } => msg!("OSPF: Link State Request von {} über {}, LSAs: {}.", nbr, iface(*i), lsas),
        Note::OspfLsu { nbr, iface: i, out: true, lsas, why } => match (why, nbr) {
            (LsuWhy::Flood, _) | (_, None) => msg!("OSPF: Link State Update über {} an die Gruppe, LSAs: {} (Flooding).", iface(*i), lsas),
            (LsuWhy::Answer, Some(a)) => msg!("OSPF: Link State Update an {} über {}, LSAs: {} (Antwort auf den Request).", a, iface(*i), lsas),
            (LsuWhy::Rxmt, Some(a)) => msg!("OSPF: Link State Update an {} über {}, LSAs: {} (Wiederholung, nicht bestätigt).", a, iface(*i), lsas),
            (LsuWhy::Back, Some(a)) => msg!("OSPF: Link State Update an {} über {}, LSAs: {} (die neuere Kopie der Datenbank).", a, iface(*i), lsas),
        },
        Note::OspfLsu { nbr, iface: i, out: false, lsas, .. } => msg!("OSPF: Link State Update von {} über {}, LSAs: {}.", ospf_who(nbr.unwrap_or_default()), iface(*i), lsas),
        Note::OspfAck { iface: i, out: true, lsas, delayed: true, .. } => msg!("OSPF: verzögerte Bestätigung über {} an die Gruppe, LSAs: {}.", iface(*i), lsas),
        Note::OspfAck { nbr, iface: i, out: true, lsas, delayed: false } => msg!("OSPF: direkte Bestätigung an {} über {}, LSAs: {}.", ospf_who(nbr.unwrap_or_default()), iface(*i), lsas),
        Note::OspfAck { nbr, iface: i, out: false, lsas, .. } => msg!("OSPF: Bestätigung von {} über {}, LSAs: {}.", ospf_who(nbr.unwrap_or_default()), iface(*i), lsas),
        Note::OspfLsaNew { lsa, from } => msg!("OSPF: {} von {} übernommen.", lsa.msg(), from),
        Note::OspfLsaOrig { lsa, why } => msg!("OSPF: eigenes {} ({}).", lsa.msg(), phrase(why.text())),
        Note::OspfLsaFlooded { lsa, ifaces } => msg!("OSPF: {} geflutet, Interfaces: {}.", lsa.msg(), ifaces),
        Note::OspfLsaAcked { lsa, nbr } => msg!("OSPF: {} hat {} bestätigt.", nbr, lsa.msg()),
        Note::OspfLsaFlush { lsa } => msg!("OSPF: eigenes {} wird mit MaxAge zurückgezogen.", lsa.msg()),
        Note::OspfLsaAged { lsa } => msg!("OSPF: {} hat MaxAge (3600 s) erreicht und wird noch einmal geflutet.", lsa.msg()),
        Note::OspfLsaGone { lsa } => msg!("OSPF: {} aus der Datenbank entfernt.", lsa.msg()),
        Note::OspfLsaOlder { lsa, nbr } => msg!("OSPF: {} schickte eine ältere Instanz von {}: es bekommt die neuere.", nbr, lsa.msg()),
        Note::OspfLsaDropped { lsa, why } => msg!("OSPF: {} verworfen: {}.", lsa.msg(), phrase(why.text())),
        Note::OspfSpf { run, routes, added, changed, removed } => msg!("OSPF: SPF-Lauf {}: Routen: {}, neu: {}, geändert: {}, entfallen: {}.", run, routes, added, changed, removed),
        Note::OspfRoute { prefix, kind, via, cost, what } => {
            let hop = if via.is_unspecified() { phrase("direkt angeschlossen") } else { Msg::plain(format!("{}", via)) };
            match what {
                RtChange::Added => msg!("OSPF: Route {} ({}) über {}, Kosten {}.", prefix, phrase(kind.text()), hop, cost),
                RtChange::Changed => msg!("OSPF: Route {} ({}) jetzt über {}, Kosten {}.", prefix, phrase(kind.text()), hop, cost),
                RtChange::Removed => msg!("OSPF: Route {} ({}) entfällt.", prefix, phrase(kind.text())),
            }
        }
        _ => Msg::default(),
    }
}

/// a protocol as notes name it ("TCP", "ICMP"; "IP" for every protocol)
fn proto_word(p: u8) -> String {
    match p {
        0 => String::from("IP"),
        n => match netpkt::ipv4::proto_name(n) {
            Some(s) => String::from(s),
            None => format!("{}", n),
        },
    }
}

/// an endpoint in a note: "10.0.0.1:80", an ICMP query's identifier in words, the address
/// alone without a port
fn ep(proto: u8, e: (Ipv4, u16)) -> Msg {
    match (proto, e.1) {
        (_, 0) => Msg::plain(format!("{}", e.0)),
        (netpkt::ipv4::PROTO_ICMP, id) => msg!("{} (Kennung {})", e.0, id),
        (_, p) => Msg::plain(format!("{}:{}", e.0, p)),
    }
}

/// a decision with its time and device
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevEvent {
    pub t: Ns,
    pub dev: u32,
    /// the frame it is about
    pub hop: Option<HopId>,
    pub note: Note,
    /// consecutive number (Trace::event)
    pub seq: u64,
    /// the batch it belongs to
    pub batch: u64,
    /// about traffic over IPv6 (a TCP connection or a UDP datagram of that family): mask()
    /// says IPv6 where the note says IPv4
    pub v6: bool,
}

impl DevEvent {
    /// the P_* bits of the decision (Note::mask), IPv6 in place of IPv4 when it is about IPv6
    /// traffic (`v6`, or the frame it is about, of mask `hop_mask`, carries IPv6)
    pub fn mask(&self, hop_mask: u32) -> u32 {
        use netpkt::decode::{P_IPV4, P_IPV6};
        let m = self.note.mask();
        let six = self.v6 || (hop_mask & P_IPV6 != 0 && hop_mask & P_IPV4 == 0);
        if six && m & P_IPV4 != 0 { (m & !P_IPV4) | P_IPV6 } else { m }
    }
}

/// the recording of a run
#[derive(Clone, Debug)]
pub struct Trace {
    hops: VecDeque<Hop>,
    events: VecDeque<DevEvent>,
    next_hop: HopId,
    pub max_hops: usize,
    pub max_events: usize,
    next_ev: u64,
    /// the current batch, the last one given out, the moment of the current one (None: not
    /// used yet), and whether it is open for what comes from outside (between kernel events)
    batch: u64,
    batches: u64,
    batch_t: Option<Ns>,
    outside: bool,
    /// the longest time a frame spent on a cable
    max_flight: Ns,
    /// frames that waited in a queue, with their batch (they leave later than their batch)
    late: VecDeque<(HopId, u64)>,
}

impl Default for Trace {
    fn default() -> Trace {
        Trace::new()
    }
}

impl Trace {
    pub fn new() -> Trace {
        Trace {
            hops: VecDeque::new(),
            events: VecDeque::new(),
            next_hop: 1,
            max_hops: 200_000,
            max_events: 400_000,
            next_ev: 1,
            batch: 1,
            batches: 1,
            batch_t: None,
            outside: true,
            max_flight: 0,
            late: VecDeque::new(),
        }
    }
    pub(crate) fn push_hop(&mut self, mut h: Hop) -> HopId {
        h.id = self.next_hop;
        self.next_hop += 1;
        if self.hops.len() >= self.max_hops {
            self.hops.pop_front();
        }
        let first = self.hops.front().map_or(h.id, |f| f.id);
        while self.late.front().is_some_and(|l| l.0 < first) {
            self.late.pop_front();
        }
        if h.t_queued < h.t_tx {
            self.late.push_back((h.id, h.batch));
        }
        self.max_flight = self.max_flight.max(h.t_rx.saturating_sub(h.t_tx));
        let id = h.id;
        self.hops.push_back(h);
        id
    }
    pub(crate) fn push_event(&mut self, mut e: DevEvent) {
        e.seq = self.next_ev;
        self.next_ev += 1;
        e.batch = self.stamp(e.t);
        if self.events.len() >= self.max_events {
            self.events.pop_front();
        }
        self.events.push_back(e);
    }
    /// a kernel event at `t` begins: what it records is a batch of its own
    pub(crate) fn begin_batch(&mut self, t: Ns) {
        self.batches += 1;
        self.batch = self.batches;
        self.batch_t = Some(t);
        self.outside = false;
    }
    /// the event is over: what comes from outside next is another batch
    pub(crate) fn end_batch(&mut self) {
        self.outside = true;
        self.new_batch();
    }
    /// a change from outside begins (an action, a command, a ping): what it records is a
    /// batch of its own; nothing happens inside a kernel event
    pub fn new_batch(&mut self) {
        if self.outside {
            self.batches += 1;
            self.batch = self.batches;
            self.batch_t = None;
        }
    }
    /// the batch of something recorded now (`t`); from outside, every moment is a batch
    pub(crate) fn stamp(&mut self, t: Ns) -> u64 {
        if self.outside && self.batch_t.is_some_and(|b| b != t) {
            self.batches += 1;
            self.batch = self.batches;
        }
        self.batch_t = Some(t);
        self.batch
    }
    pub(crate) fn mark_lost(&mut self, id: HopId) {
        if let Some(h) = self.hop_mut(id) {
            h.drop = Some(HopDrop::LinkDown);
        }
    }
    fn hop_mut(&mut self, id: HopId) -> Option<&mut Hop> {
        let first = self.hops.front()?.id;
        if id < first {
            return None;
        }
        self.hops.get_mut((id - first) as usize)
    }
    /// every recorded frame, oldest first
    pub fn hops(&self) -> impl DoubleEndedIterator<Item = &Hop> + ExactSizeIterator {
        self.hops.iter()
    }
    /// a frame by id (None once it left the ring)
    pub fn hop(&self, id: HopId) -> Option<&Hop> {
        let first = self.hops.front()?.id;
        if id < first {
            return None;
        }
        self.hops.get((id - first) as usize)
    }
    /// frames sent after hop `id`
    pub fn hops_after(&self, id: HopId) -> impl Iterator<Item = &Hop> {
        let first = self.hops.front().map(|h| h.id).unwrap_or(self.next_hop);
        let skip = if id < first { 0 } else { (id - first + 1) as usize };
        self.hops.iter().skip(skip)
    }
    /// the id the next frame gets
    pub fn next_hop_id(&self) -> HopId {
        self.next_hop
    }
    pub fn events(&self) -> impl DoubleEndedIterator<Item = &DevEvent> + ExactSizeIterator {
        self.events.iter()
    }
    /// the decisions about one frame
    pub fn events_of(&self, hop: HopId) -> impl Iterator<Item = &DevEvent> {
        self.events.iter().filter(move |e| e.hop == Some(hop))
    }
    pub fn clear(&mut self) {
        self.hops.clear();
        self.events.clear();
        self.late.clear();
    }
}

// ---------------------------------------------------------------- simulation mode

use alloc::vec::Vec;

impl Trace {
    /// a decision by its number (None once it left the ring)
    pub fn event(&self, seq: u64) -> Option<&DevEvent> {
        let first = self.events.front()?.seq;
        if seq < first {
            return None;
        }
        self.events.get((seq - first) as usize)
    }
    /// the number the next decision gets
    pub fn next_event_seq(&self) -> u64 {
        self.next_ev
    }
    /// decisions recorded after number `seq` (0: all of them)
    pub fn events_after(&self, seq: u64) -> impl Iterator<Item = &DevEvent> {
        let first = self.events.front().map_or(self.next_ev, |e| e.seq);
        let skip = if seq < first { 0 } else { ((seq - first + 1) as usize).min(self.events.len()) };
        self.events.range(skip..)
    }
    /// the decisions of one device in one batch, in order
    pub fn batch_events(&self, batch: u64, dev: u32) -> impl Iterator<Item = &DevEvent> {
        let from = self.events.partition_point(|e| e.batch < batch);
        self.events.range(from..).take_while(move |e| e.batch == batch).filter(move |e| e.dev == dev)
    }
    /// decisions made after `t0` up to and including `t1`
    pub fn events_between(&self, t0: Ns, t1: Ns) -> impl Iterator<Item = &DevEvent> {
        let a = self.events.partition_point(|e| e.t <= t0);
        let b = self.events.partition_point(|e| e.t <= t1).max(a);
        self.events.range(a..b)
    }
    /// the number of the last decision made at or before `t` (None: nothing yet)
    pub fn last_event_at(&self, t: Ns) -> Option<u64> {
        let end = self.events.partition_point(|e| e.t <= t);
        self.events.get(end.checked_sub(1)?).map(|e| e.seq)
    }
    /// the frames on a cable at `t` (t_tx <= t < t_rx), in send order; it looks at most at the
    /// `limit` latest frames sent up to `t`
    pub fn on_wire(&self, t: Ns, limit: usize) -> Vec<&Hop> {
        let end = self.hops.partition_point(|h| h.t_tx <= t);
        let flight = self.max_flight;
        let mut v: Vec<&Hop> = self.hops.range(..end).rev().take(limit).take_while(|h| h.t_tx.saturating_add(flight) > t).filter(|h| t < h.t_rx).collect();
        v.reverse();
        v
    }
    /// frames whose last bit reached the far end (or would have, when lost) after `t0` up to
    /// and including `t1`
    pub fn arrived_between(&self, t0: Ns, t1: Ns) -> impl Iterator<Item = &Hop> {
        let flight = self.max_flight;
        let a = self.hops.partition_point(|h| h.t_tx.saturating_add(flight) <= t0);
        let b = self.hops.partition_point(|h| h.t_tx <= t1).max(a);
        self.hops.range(a..b).filter(move |h| h.t_rx > t0 && h.t_rx <= t1)
    }
    /// the latest moment before `t` at which a frame the filter passes was sent
    pub fn prev_send(&self, t: Ns, f: &Filter) -> Option<Ns> {
        let end = self.hops.partition_point(|h| h.t_tx < t);
        self.hops.range(..end).rev().find(|h| f.hop(h)).map(|h| h.t_tx)
    }
    /// the first moment after `t`, and not after `until`, at which a frame the filter passes
    /// was sent
    pub fn next_send(&self, t: Ns, until: Ns, f: &Filter) -> Option<Ns> {
        let from = self.hops.partition_point(|h| h.t_tx <= t);
        self.hops.range(from..).take_while(|h| h.t_tx <= until).find(|h| f.hop(h)).map(|h| h.t_tx)
    }
    /// the frames that left at `t` without waiting
    pub(crate) fn sent_at(&self, t: Ns) -> impl Iterator<Item = &Hop> {
        let from = self.hops.partition_point(|h| h.t_tx < t);
        self.hops.range(from..).take_while(move |h| h.t_tx == t)
    }
    /// frames that waited in a queue, with their batch
    pub(crate) fn late(&self) -> impl Iterator<Item = &(HopId, u64)> {
        self.late.iter()
    }
}

/// which frames and decisions to show, to step to or to capture: protocols and devices
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// netpkt::decode P_* bits: a frame passes when it carries one of them; 0: every protocol
    pub mask: u32,
    /// device ids: a frame passes when it leaves or reaches one of them; empty: every device
    pub devices: Vec<u32>,
}

impl Filter {
    /// everything
    pub fn all() -> Filter {
        Filter::default()
    }
    /// the frames of some protocols (P_* bits; 0: all), anywhere
    pub fn protocols(mask: u32) -> Filter {
        Filter { mask, devices: Vec::new() }
    }
    fn has_dev(&self, d: u32) -> bool {
        self.devices.is_empty() || self.devices.contains(&d)
    }
    /// a frame passes
    pub fn hop(&self, h: &Hop) -> bool {
        (self.mask == 0 || h.mask & self.mask != 0) && (self.has_dev(h.from.0) || self.has_dev(h.to.0))
    }
    /// a decision passes: one of the devices made it or sent the frame it is about, and that
    /// frame or the decision itself (Note::mask) carries one of the protocols
    pub fn event(&self, tr: &Trace, e: &DevEvent) -> bool {
        let hop = e.hop.and_then(|h| tr.hop(h));
        let hm = hop.map_or(0, |h| h.mask);
        let bits = hm | e.mask(hm);
        (self.mask == 0 || bits & self.mask != 0) && (self.has_dev(e.dev) || hop.is_some_and(|h| self.has_dev(h.from.0)))
    }
}

impl Note {
    /// the netpkt::decode P_* bits of the protocol the decision itself is about (ARP, IPv4,
    /// ICMP, UDP, TCP); 0 for cables and bridging, which are about whatever frame they carry
    pub fn mask(&self) -> u32 {
        use netpkt::decode::{P_ARP, P_ICMP, P_IPV4, P_TCP, P_UDP};
        match self {
            Note::ArpRequest { .. }
            | Note::ArpReply { .. }
            | Note::ArpLearned { .. }
            | Note::ArpWaiting { .. }
            | Note::ArpIncomplete { .. }
            | Note::ArpProxied { .. }
            | Note::ArpFailed { .. }
            | Note::ArpIgnored { .. } => P_ARP,
            Note::IpLocal { proto } => {
                P_IPV4
                    | match *proto {
                        netpkt::ipv4::PROTO_ICMP => P_ICMP,
                        netpkt::ipv4::PROTO_UDP => P_UDP,
                        netpkt::ipv4::PROTO_TCP => P_TCP,
                        _ => 0,
                    }
            }
            Note::IpRouted { .. }
            | Note::IpNoRoute { .. }
            | Note::IpTtlExpired { .. }
            | Note::IpNotForMe { .. }
            | Note::IpMartian { .. }
            | Note::IpDirectedBroadcast { .. }
            | Note::IpBadChecksum
            | Note::IpMalformed
            | Note::IpFragmented { .. }
            | Note::IpFragNeeded { .. }
            | Note::IpReassembled { .. }
            | Note::IpReasmTimeout => P_IPV4,
            Note::EchoAnswered { .. } | Note::IcmpSent { .. } | Note::IcmpLimited => P_IPV4 | P_ICMP,
            Note::UdpNoPort { .. } => P_IPV4 | P_UDP,
            Note::TcpNoListener { .. } => P_IPV4 | P_TCP,
            n if n.is_tcp() => P_IPV4 | P_TCP,
            n if n.is_http() => P_IPV4 | P_TCP | netpkt::decode::P_HTTP,
            Note::PartitionDrop { .. } => P_IPV4,
            Note::KvMember { .. } => P_IPV4 | P_UDP,
            Note::DiskFull => 0,
            n if n.is_lab() => P_IPV4 | P_TCP | netpkt::decode::P_HTTP,
            n if n.is_stp() => netpkt::decode::P_STP,
            n if n.is_rip() => P_IPV4 | P_UDP | netpkt::decode::P_RIP,
            Note::IpGroupIgnored { .. } => P_IPV4,
            Note::NatCreate { proto, .. } | Note::NatTranslate { proto, .. } | Note::NatExpire { proto, .. } | Note::NatNoEntry { proto, .. } => {
                P_IPV4
                    | match *proto {
                        netpkt::ipv4::PROTO_ICMP => P_ICMP,
                        netpkt::ipv4::PROTO_UDP => P_UDP,
                        netpkt::ipv4::PROTO_TCP => P_TCP,
                        _ => 0,
                    }
            }
            Note::NatQuote { .. } | Note::NatErrorDropped => P_IPV4 | P_ICMP,
            n if n.is_filter() => P_IPV4,
            Note::V6(n) => n.mask(),
            n if n.is_ospf() => P_IPV4 | netpkt::decode::P_OSPF,
            Note::Wlan(n) => n.mask(),
            _ => 0,
        }
    }
}

// ---------------------------------------------------------------- spanning tree

impl Note {
    /// a decision of the spanning tree itself (StpBlocked is about the frame it drops)
    pub fn is_stp(&self) -> bool {
        matches!(
            self,
            Note::StpRx { .. }
                | Note::StpTx { .. }
                | Note::StpDropped { .. }
                | Note::StpRole { .. }
                | Note::StpState { .. }
                | Note::StpRoot { .. }
                | Note::StpTopology { .. }
                | Note::StpFlush { .. }
                | Note::StpAging { .. }
                | Note::StpAged { .. }
                | Note::StpEdge { .. }
                | Note::StpMigrate { .. }
                | Note::StpErrDisable { .. }
        )
    }
    /// what "debug spanning-tree events" prints: the protocol's decisions without the BPDUs
    pub fn is_stp_event(&self) -> bool {
        self.is_stp() && !matches!(self, Note::StpRx { .. } | Note::StpTx { .. })
    }
}

/// a bridge ID in notes: "32768.024a.0000.0100"
pub fn stp_id(id: netpkt::stp::BridgeId) -> String {
    format!("{}.{}", id.priority(), id.mac().cisco())
}

/// the kind of a BPDU in notes (protocol terms): "Configuration BPDU", "RST BPDU (Designated)"
fn bpdu_kind(b: &netpkt::stp::Bpdu) -> String {
    match b.kind {
        netpkt::stp::Kind::Config => String::from("Configuration BPDU"),
        netpkt::stp::Kind::Tcn => String::from("TCN BPDU"),
        netpkt::stp::Kind::Rst => format!("RST BPDU ({})", b.role().name()),
    }
}

/// the flags of a BPDU in notes: " [TC, Proposal]", "" without any that matter
fn bpdu_flags(b: &netpkt::stp::Bpdu) -> String {
    let mut v: Vec<&str> = Vec::new();
    if b.tc() {
        v.push("TC");
    }
    if b.tca() {
        v.push("TCA");
    }
    if b.proposal() {
        v.push("Proposal");
    }
    if b.agreement() {
        v.push("Agreement");
    }
    if b.forwarding() {
        v.push("Forwarding");
    } else if b.learning() {
        v.push("Learning");
    }
    if v.is_empty() { String::new() } else { format!(" [{}]", v.join(", ")) }
}
