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
//                                                 HTTP's layer 7 (http.rs, httpsvc.rs)
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
use netpkt::{Ipv4, Mac, Prefix4};

use crate::kernel::{Frame, Ns};
use crate::resolver::LookupErr;
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
    ResolveQuery { name: String, qtype: u16, server: Ipv4 },
    ResolveAnswer { name: String, addr: Option<Ipv4>, cached: bool },
    ResolveNegative { name: String, nx: bool },
    ResolveTimeout { name: String, server: Ipv4 },
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
    TcpStateChange { port: u16, peer: Ipv4, pport: u16, from: TcpState, to: TcpState },
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
    /// lab (fault.rs): a fault started / was healed (its number and what it does)
    FaultStart { id: u32, what: String },
    FaultEnd { id: u32, what: String },
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
            _ => 2,
        }
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
        )
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
                if *pport == 0 {
                    msg!("TCP Port {}: {} \u{2192} {}.", p, from.name(), to.name())
                } else {
                    msg!("TCP {} \u{2194} {}:{}: {} \u{2192} {}.", p, peer, pport, from.name(), to.name())
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
        }
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
        let bits = hop.map_or(0, |h| h.mask) | e.note.mask();
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
            _ => 0,
        }
    }
}
