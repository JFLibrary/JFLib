// tcp -- TCP after RFC 9293 in the virtual OS of a device: the eleven states with the
// three-way handshake, simultaneous open and close, the MSS option from the MTU of the route,
// the send and receive windows, retransmission with an RTO after RFC 6298, Reno congestion
// control after RFC 5681 (slow start, congestion avoidance, fast retransmit and fast recovery,
// limited transmit after RFC 3042), delayed ACKs, Nagle, zero-window probes, the RST rules
// with the challenge ACKs of RFC 5961, TIME-WAIT and keepalive
//
//   TcpCfg::linux() / ios()       the values of a system: RTO bounds, timers, buffers, windows
//   TcpStack                      the TCP part of a device's OS (Os::tcp): connections,
//                                 listening sockets, counters
//   Tcb, TcpState                 one connection and its state
//   Engine::tcp_input             from ip_local: a segment goes to its connection or to a
//                                 listening socket, or a RST answers it
//   Engine::tcp_timer             node timer SLOT_TCP: the segments a device sent to itself,
//                                 then every expired timer of every connection (in id order)
//   socket calls (crate)          listen, connect, accept, send, recv, shutdown, close, abort
//                                 behind OsApi (os.rs); their events reach a service through
//                                 its inbox (SvcEvent::Tcp*)
//   Engine::tcp_icmp_error        ICMP errors about own segments: a connection attempt ends,
//                                 an established one remembers the error, "fragmentation
//                                 needed" lowers the MSS
//   Engine::tcp_conns / tcp_conn / tcp_stats   what ss and "show tcp brief" show
//   TcpEnd::text / RstWhy::text   German reasons (the app translates them: engine.tsv);
//                                 refusals of listen and connect are a Msg (jfn_model::msg)
//   SockOpts::ttl                 the TTL of a connection's IPv4 segments (0: the system's;
//                                 BGP: 1 towards an external neighbour, bgp.rs); a connection
//                                 from a given source address: Engine::tcp_connect_src (BGP's
//                                 update-source)
//
// The system decides the numbers (TcpCfg).  Linux: ISNs and ports come from the device's
// random numbers, connect takes ports from 32768-60999 (even ones first), the MSS is the MTU
// of the route's interface - 40, the receive buffer 128 KiB, so without window scaling the
// SYN offers 64240 bytes (65535 rounded down to the MSS); 10 segments of initial window
// (RFC 6928); RTO 1 s at first, then 200 ms to 120 s; delayed ACKs after 40 ms, with quick
// ACKs at the start of a transfer, an ACK at once when more than one full segment waits, and
// ping-pong mode for request and answer; Nagle with Minshall's variant; zero-window probes
// without data just below SND.UNA; TIME-WAIT 60 s; keepalive after 7200 s, then every 75 s,
// nine times; 6 SYN retries (it gives up after 127 s), 5 SYN-ACK retries, 15 for data.
// IOS-like: MSS 536 towards destinations beyond the connected subnets, a window of 4128
// bytes, RTO from 300 ms, ACK hold 200 ms, classic Nagle, probes with a byte of new data,
// connection attempts end after 30 s.
//
// Not in v1: SACK, window scaling, timestamps (RFC 7323), urgent data (URG is ignored, the
// bytes count as ordinary data), TCP Fast Open, ECN and SYN cookies.  Linux's congestion
// control is CUBIC; here every system runs Reno as RFC 5681 describes it, counted in bytes.
// Buffer sizes are bytes of data (Linux doubles SO_RCVBUF for its own bookkeeping; not here).
//
// A segment a device sends to itself waits for the TCP timer, which fires at the same virtual
// time: TCP never runs inside its own call.  Each connection is processed with the engine at
// hand but owns nothing of it: Tcb methods collect what is to happen (Out: segments, notes,
// events) and the engine does it in that order afterwards.
//
// IPv6 (RFC 9293 with RFC 8200 8.1): a connection's addresses are netpkt::IpAddr, both of one
// family; its segments carry the checksum over the IPv6 pseudo header and go to l3v6.rs with the
// hop limit of the way out (Engine::tcp6_input takes them in).  The MSS is the MTU of the route's
// interface (or the path MTU learned for the destination, RFC 8201) - 60, so at least 1220 for
// the minimum MTU of 1280; a peer that names none counts as 1220 (RFC 9293 3.7.1); IOS-like
// devices offer 1220 beyond their connected prefixes.  Packet Too Big about a segment in flight
// lowers the MSS to the MTU - 60 and sends what was lost again; a segment that still does not
// fit the path is not fragmented by the sender (as Linux: TCP adapts its MSS instead).  ICMPv6
// errors (Engine::tcp_icmp6_error) end a connection attempt at once and are remembered by an
// established connection, as the IPv4 ones are: port unreachable means refused, the others
// unreachable.  Ports and ISNs come from the device's random numbers as for IPv4.  A listening
// socket on :: takes both families (os.rs: ANY); a connection to a link-local address needs
// the interface of its source and is not offered by the socket calls.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::msg;
use jfn_model::msg::Msg;

use netpkt::icmp::{T_TIME_EXCEEDED, T_UNREACH, UNREACH_FRAG_NEEDED, UNREACH_PORT, UNREACH_PROTO};
use netpkt::icmpv6;
use netpkt::ipv4::{Ipv4Hdr, Ipv4View, PROTO_TCP};
use netpkt::ipv6::{Ipv6Hdr, Ipv6View, NH_TCP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::tcp::{seq_ge, seq_gt, seq_le, seq_lt, TcpHdr, TcpOpts, TcpView, ACK, FIN, PSH, RST, SYN};
use netpkt::{IpAddr, Ipv4, Ipv6};

use crate::ip::Origin;
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Job, SvcEvent};
use crate::trace::{HopId, Note};

/// the node timer slot of TCP: every connection of a device shares it
pub const SLOT_TCP: u32 = 5;
/// the MSS of a peer that names none (RFC 9293 3.7.1)
pub const MSS_DEFAULT: u16 = 536;
/// the same over IPv6: the minimum MTU 1280 - 60 (RFC 9293 3.7.1)
pub const MSS_DEFAULT6: u16 = 1220;
/// the smallest MSS a path MTU may bring (Linux TCP_MIN_MSS)
pub const MSS_MIN: u16 = 88;
/// the largest window without window scaling
pub const WND_MAX: u32 = 65535;
/// ssthresh before the first loss ("arbitrarily high", RFC 5681 3.1)
pub const SSTHRESH_INF: u32 = 0x7FFF_FFFF;
/// the largest send buffer that grows by itself (Linux tcp_wmem[2])
pub const SNDBUF_AUTO_MAX: u32 = 4 << 20;

/// the state of a connection (RFC 9293 3.3.2)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

impl TcpState {
    /// the RFC's name ("SYN-SENT")
    pub fn name(self) -> &'static str {
        match self {
            TcpState::Closed => "CLOSED",
            TcpState::Listen => "LISTEN",
            TcpState::SynSent => "SYN-SENT",
            TcpState::SynReceived => "SYN-RECEIVED",
            TcpState::Established => "ESTABLISHED",
            TcpState::FinWait1 => "FIN-WAIT-1",
            TcpState::FinWait2 => "FIN-WAIT-2",
            TcpState::CloseWait => "CLOSE-WAIT",
            TcpState::Closing => "CLOSING",
            TcpState::LastAck => "LAST-ACK",
            TcpState::TimeWait => "TIME-WAIT",
        }
    }
    /// the name in the State column of ss ("ESTAB", "SYN-RECV", "UNCONN")
    pub fn ss_name(self) -> &'static str {
        match self {
            TcpState::Closed => "UNCONN",
            TcpState::Listen => "LISTEN",
            TcpState::SynSent => "SYN-SENT",
            TcpState::SynReceived => "SYN-RECV",
            TcpState::Established => "ESTAB",
            TcpState::FinWait1 => "FIN-WAIT-1",
            TcpState::FinWait2 => "FIN-WAIT-2",
            TcpState::CloseWait => "CLOSE-WAIT",
            TcpState::Closing => "CLOSING",
            TcpState::LastAck => "LAST-ACK",
            TcpState::TimeWait => "TIME-WAIT",
        }
    }
    /// the name in the State column of "show tcp brief" ("ESTAB", "SYNRCVD", "TIMEWAIT")
    pub fn ios_name(self) -> &'static str {
        match self {
            TcpState::Closed => "CLOSED",
            TcpState::Listen => "LISTEN",
            TcpState::SynSent => "SYNSENT",
            TcpState::SynReceived => "SYNRCVD",
            TcpState::Established => "ESTAB",
            TcpState::FinWait1 => "FINWAIT1",
            TcpState::FinWait2 => "FINWAIT2",
            TcpState::CloseWait => "CLOSEWAIT",
            TcpState::Closing => "CLOSING",
            TcpState::LastAck => "LASTACK",
            TcpState::TimeWait => "TIMEWAIT",
        }
    }
}

/// why a connection ended for its service
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpEnd {
    /// both sides closed: the orderly end
    Closed,
    /// the peer answered the SYN with a RST, nobody listens there (ECONNREFUSED)
    Refused,
    /// the peer reset the connection (ECONNRESET)
    Reset,
    /// retransmissions, the connection attempt or the keepalive probes went unanswered
    /// (ETIMEDOUT)
    Timeout,
    /// an ICMP error: network or host unreachable (EHOSTUNREACH)
    Unreachable,
    /// this side reset it (unread data at close, its listening socket closed, abort)
    Aborted,
}

impl TcpEnd {
    /// German, for notes and status lines (the app translates it)
    pub fn text(self) -> &'static str {
        // i18n
        match self {
            TcpEnd::Closed => "ordentlich geschlossen",
            TcpEnd::Refused => "abgelehnt (RST auf den SYN)",
            TcpEnd::Reset => "von der Gegenseite zurückgesetzt",
            TcpEnd::Timeout => "keine Antwort mehr (Zeitüberschreitung)",
            TcpEnd::Unreachable => "Ziel nicht erreichbar (ICMP)",
            TcpEnd::Aborted => "von dieser Seite abgebrochen (RST)",
        }
    }
    /// what strerror says for the error of the socket ("Connection refused"); None for the
    /// orderly end
    pub fn errno(self) -> Option<&'static str> {
        match self {
            TcpEnd::Closed => None,
            TcpEnd::Refused => Some("Connection refused"),
            TcpEnd::Reset => Some("Connection reset by peer"),
            TcpEnd::Timeout => Some("Connection timed out"),
            TcpEnd::Unreachable => Some("No route to host"),
            TcpEnd::Aborted => Some("Software caused connection abort"),
        }
    }
}

/// why a RST went out
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RstWhy {
    /// a segment for no connection
    NoConn,
    /// an acknowledgement of something never sent (LISTEN, SYN-SENT, SYN-RECEIVED)
    BadAck,
    /// the service aborted the connection
    Abort,
    /// closed with bytes it never read (Linux)
    Unread,
    /// bytes arrived after the service closed the socket (Linux)
    DataAfterClose,
    /// the keepalive probes went unanswered
    Keepalive,
    /// the listening socket closed before the connection was accepted
    ListenerClosed,
}

impl RstWhy {
    /// German, for notes (the app translates it)
    pub fn text(self) -> &'static str {
        // i18n
        match self {
            RstWhy::NoConn => "keine solche Verbindung",
            RstWhy::BadAck => "Bestätigung für nie Gesendetes",
            RstWhy::Abort => "der Dienst bricht die Verbindung ab",
            RstWhy::Unread => "geschlossen, obwohl noch ungelesene Bytes da waren",
            RstWhy::DataAfterClose => "Daten nach dem Schließen",
            RstWhy::Keepalive => "keine Antwort auf die Keepalive-Proben",
            RstWhy::ListenerClosed => "der lauschende Socket wurde geschlossen",
        }
    }
}

/// the timer a connection waits for (the timer column of ss -o)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpTimer {
    /// retransmission ("on")
    Retransmit,
    /// zero-window probes
    Persist,
    Keepalive,
    /// TIME-WAIT, and FIN-WAIT-2 of a closed socket
    TimeWait,
    DelayedAck,
}

/// the TCP values of a system
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcpCfg {
    /// the RTO before the first measurement, and its bounds (RFC 6298)
    pub rto_init: Ns,
    pub rto_min: Ns,
    pub rto_max: Ns,
    /// the clock granularity G of RFC 6298
    pub granularity: Ns,
    /// how long an ACK may wait (delayed ACK; IOS "ACK hold")
    pub delack: Ns,
    /// quick ACKs at the start of a transfer (Linux: up to 16); 0: none
    pub quickack: u8,
    /// a reply soon after data came makes the connection interactive: no quick ACKs, the
    /// ACK rides on the answer, also the ACK of a FIN (Linux ping-pong mode)
    pub pingpong: bool,
    /// an ACK goes at once when more than one full segment waits (Linux); else at the second
    /// full segment (RFC 5681 4.2)
    pub ack_over_one: bool,
    /// retransmissions of a SYN, of a SYN-ACK, of data, and of data of a closed socket
    pub syn_retries: u8,
    pub synack_retries: u8,
    pub retries2: u8,
    pub orphan_retries: u8,
    /// a connection attempt ends after this long (IOS "ip tcp synwait-time")
    pub syn_wait: Option<Ns>,
    pub time_wait: Ns,
    /// a connection its service closed may stay in FIN-WAIT-2 this long (tcp_fin_timeout)
    pub fin_timeout: Ns,
    pub keep_idle: Ns,
    pub keep_intvl: Ns,
    pub keep_probes: u8,
    /// the buffers of new sockets (bytes of data)
    pub rcvbuf: u32,
    pub sndbuf: u32,
    /// the send buffer grows with the congestion window unless a service sets it (Linux)
    pub snd_auto: bool,
    /// the initial window in segments (0: the formula of RFC 5681 3.1)
    pub init_cwnd: u32,
    /// the MSS towards destinations beyond the connected subnets (IOS: 536)
    pub mss_remote: Option<u16>,
    /// Nagle holds a small segment only while another small one is unacknowledged (Minshall)
    pub minshall: bool,
    /// zero-window probes carry a byte of new data (else: empty, just below SND.UNA)
    pub probe_data: bool,
    /// windows are multiples of the MSS and never shrink (Linux); else the receiver's SWS
    /// rule of RFC 9293 3.8.6.2.2
    pub wnd_round: bool,
    /// the congestion window starts again after an idle RTO (RFC 5681 4.1)
    pub idle_restart: bool,
    /// segments carry Don't Fragment and an IP identification per connection (Linux)
    pub df: bool,
    /// the ports of connect, and whether even ones come first (Linux)
    pub ports: (u16, u16),
    pub even_ports: bool,
    /// the backlog of a listening socket when a service names none
    pub backlog: u32,
}

impl TcpCfg {
    /// Linux 6
    pub fn linux() -> TcpCfg {
        TcpCfg {
            rto_init: SEC,
            rto_min: 200 * MS,
            rto_max: 120 * SEC,
            granularity: MS,
            delack: 40 * MS,
            quickack: 16,
            pingpong: true,
            ack_over_one: true,
            syn_retries: 6,
            synack_retries: 5,
            retries2: 15,
            orphan_retries: 8,
            syn_wait: None,
            time_wait: 60 * SEC,
            fin_timeout: 60 * SEC,
            keep_idle: 7200 * SEC,
            keep_intvl: 75 * SEC,
            keep_probes: 9,
            rcvbuf: 131_072,
            sndbuf: 16_384,
            snd_auto: true,
            init_cwnd: 10,
            mss_remote: None,
            minshall: true,
            probe_data: false,
            wnd_round: true,
            idle_restart: true,
            df: true,
            ports: (32768, 60999),
            even_ports: true,
            backlog: 128,
        }
    }
    /// the IOS-like system of routers and switches
    pub fn ios() -> TcpCfg {
        TcpCfg {
            rto_init: SEC,
            rto_min: 300 * MS,
            rto_max: 60 * SEC,
            granularity: MS,
            delack: 200 * MS,
            quickack: 0,
            pingpong: false,
            ack_over_one: false,
            syn_retries: 4,
            synack_retries: 4,
            retries2: 15,
            orphan_retries: 8,
            syn_wait: Some(30 * SEC),
            time_wait: 60 * SEC,
            fin_timeout: 60 * SEC,
            keep_idle: 60 * SEC,
            keep_intvl: 60 * SEC,
            keep_probes: 5,
            rcvbuf: 4128,
            sndbuf: 4128,
            snd_auto: false,
            init_cwnd: 0,
            mss_remote: Some(MSS_DEFAULT),
            minshall: false,
            probe_data: true,
            wnd_round: false,
            idle_restart: true,
            df: false,
            ports: (49152, 65535),
            even_ports: false,
            backlog: 16,
        }
    }
}

/// socket settings (setsockopt)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SockOpts {
    /// TCP_NODELAY: no Nagle
    pub nodelay: bool,
    /// SO_KEEPALIVE
    pub keepalive: bool,
    /// SO_RCVBUF / SO_SNDBUF in bytes of data (0: the system's)
    pub rcvbuf: u32,
    pub sndbuf: u32,
    /// IP_TTL: the TTL of its IPv4 segments (0: the system's)
    pub ttl: u8,
}

/// the counters of a connection
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TcpCounters {
    pub segs_in: u64,
    pub segs_out: u64,
    /// data bytes sent for the first time, acknowledged by the peer, received in order
    pub bytes_sent: u64,
    pub bytes_acked: u64,
    pub bytes_received: u64,
    /// segments sent again (timeouts, fast retransmits, a SYN-ACK for a repeated SYN)
    pub retrans: u64,
    pub timeouts: u64,
    pub fast_retrans: u64,
    pub dupacks_in: u64,
    /// segments that came out of order, and segments with bytes that came before
    pub ooo_in: u64,
    pub dup_in: u64,
    /// zero-window and keepalive probes
    pub probes: u64,
}

/// the TCP counters of a device (RFC 4022 names; netstat -s, "show tcp statistics")
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TcpStats {
    pub active_opens: u64,
    pub passive_opens: u64,
    /// connection attempts that ended in CLOSED (or back in LISTEN)
    pub attempt_fails: u64,
    /// established connections that went to CLOSED directly (resets)
    pub estab_resets: u64,
    /// connections in ESTABLISHED or CLOSE-WAIT now (filled in by tcp_stats)
    pub curr_estab: u64,
    pub in_segs: u64,
    pub out_segs: u64,
    pub retrans_segs: u64,
    /// segments with a bad checksum or header
    pub in_errs: u64,
    pub out_rsts: u64,
    /// SYNs and handshake ACKs dropped because the accept queue was full
    pub listen_drops: u64,
    pub timeouts: u64,
    pub delayed_acks: u64,
}

impl TcpStats {
    fn add(&mut self, o: &TcpStats) {
        self.active_opens += o.active_opens;
        self.passive_opens += o.passive_opens;
        self.attempt_fails += o.attempt_fails;
        self.estab_resets += o.estab_resets;
        self.in_segs += o.in_segs;
        self.out_segs += o.out_segs;
        self.retrans_segs += o.retrans_segs;
        self.in_errs += o.in_errs;
        self.out_rsts += o.out_rsts;
        self.listen_drops += o.listen_drops;
        self.timeouts += o.timeouts;
        self.delayed_acks += o.delayed_acks;
    }
}

/// a connection or listening socket as ss and "show tcp brief" show it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TcpRow {
    pub id: u32,
    pub state: TcpState,
    /// a listening socket on :: takes both families (os.rs: ANY)
    pub local: (IpAddr, u16),
    /// the unspecified address of the local one's family and port 0 for a listening socket
    pub remote: (IpAddr, u16),
    /// the service that holds it
    pub owner: Option<String>,
    /// bytes not read yet and bytes not acknowledged yet; for a listening socket the
    /// connections waiting to be accepted and the backlog (as ss shows them)
    pub recv_q: u32,
    pub send_q: u32,
    pub mss: u16,
    /// the window the peer offers and the one offered to it
    pub snd_wnd: u32,
    pub rcv_wnd: u32,
    pub cwnd: u32,
    pub ssthresh: u32,
    pub srtt: Option<Ns>,
    pub rttvar: Ns,
    pub rto: Ns,
    pub iss: u32,
    pub irs: u32,
    pub snd_una: u32,
    pub snd_nxt: u32,
    pub rcv_nxt: u32,
    /// the next timer and how long until it fires
    pub timer: Option<(TcpTimer, Ns)>,
    /// timeouts in a row for the same data
    pub retries: u8,
    pub counters: TcpCounters,
}

/// a listening socket
#[derive(Clone, Debug)]
pub struct Listener {
    pub id: u32,
    /// the service
    pub owner: u32,
    /// the address it listens on (0.0.0.0: every IPv4 address, ::: every address of both
    /// families)
    pub addr: IpAddr,
    pub port: u16,
    pub backlog: u32,
    /// connections that completed the handshake and wait for tcp_accept
    pub queue: VecDeque<u32>,
    /// connections in SYN-RECEIVED
    pub half: u32,
    /// what its connections get
    pub opts: SockOpts,
    pub accepted: u64,
    pub dropped: u64,
    /// a TcpAccept event is on its way
    ev: bool,
}

/// the TCP part of a device's OS
#[derive(Clone, Debug)]
pub struct TcpStack {
    pub cfg: TcpCfg,
    pub conns: BTreeMap<u32, Tcb>,
    pub listeners: BTreeMap<u32, Listener>,
    pub stats: TcpStats,
    /// (local address, local port, remote address, remote port) -> connection
    tuples: BTreeMap<(IpAddr, u16, IpAddr, u16), u32>,
    next_id: u32,
    /// IPv4 and IPv6 packets the device sent to itself; the TCP timer delivers them
    loopq: VecDeque<Vec<u8>>,
    /// when the TCP timer fires next
    armed: Option<Ns>,
}

impl TcpStack {
    pub fn new(ios: bool) -> TcpStack {
        TcpStack {
            cfg: if ios { TcpCfg::ios() } else { TcpCfg::linux() },
            conns: BTreeMap::new(),
            listeners: BTreeMap::new(),
            stats: TcpStats::default(),
            tuples: BTreeMap::new(),
            next_id: 1,
            loopq: VecDeque::new(),
            armed: None,
        }
    }
    /// the power went off: every connection is gone without a word; ids go on
    pub(crate) fn power_off(&mut self) {
        let next = self.next_id;
        *self = TcpStack { cfg: self.cfg, next_id: next, ..TcpStack::new(false) };
    }
    /// the listening socket for a segment to `addr`:`port` (one on the address before one on
    /// every address of its family, before one on :: for both)
    fn listener_for(&self, addr: IpAddr, port: u16) -> Option<u32> {
        let mut best: Option<(u8, u32)> = None;
        for l in self.listeners.values().filter(|l| l.port == port && crate::os::takes(l.addr, addr)) {
            let score = if l.addr == addr { 2 } else if l.addr.is_v4() == addr.is_v4() { 1 } else { 0 };
            if best.is_none_or(|b| score > b.0) {
                best = Some((score, l.id));
            }
        }
        best.map(|b| b.1)
    }
}

// ---------------------------------------------------------------- one connection

/// data in flight, for the RTT measurement (Karn: nothing from what went twice)
#[derive(Clone, Copy, Debug)]
struct Sent {
    end: u32,
    at: Ns,
    retx: bool,
}

/// a connection (RFC 9293 3.3.1 names the variables)
#[derive(Clone, Debug)]
pub struct Tcb {
    pub id: u32,
    pub state: TcpState,
    /// both of one family
    pub local: (IpAddr, u16),
    pub remote: (IpAddr, u16),
    /// the service that holds it (None: not accepted yet, or closed by its service)
    pub owner: Option<u32>,
    /// the listening socket it came from, until it is accepted
    pub listener: Option<u32>,
    /// opened by a SYN to a listening socket
    pub passive: bool,
    /// its service closed it: no more events, and data that still arrives resets it
    pub orphan: bool,
    pub opts: SockOpts,
    // the send side; snd_max is the RFC's SND.NXT, snd_nxt the next byte to (re)send, which
    // goes back to SND.UNA after a timeout
    pub iss: u32,
    pub snd_una: u32,
    pub snd_nxt: u32,
    pub snd_max: u32,
    pub snd_wnd: u32,
    snd_wl1: u32,
    snd_wl2: u32,
    /// the largest window the peer offered
    pub max_sndwnd: u32,
    sbuf: VecDeque<u8>,
    /// the sequence number of sbuf[0]
    sbuf_seq: u32,
    /// a FIN follows the data (shutdown or close)
    fin_queued: bool,
    /// the service has more to write (its last write did not fit): only full segments go
    more: bool,
    /// the service waits for room in the send buffer
    want_write: bool,
    /// the end of the last segment smaller than the MSS (Minshall's Nagle)
    snd_sml: u32,
    /// a zero-window probe byte is outstanding (probe_data)
    probe_byte: bool,
    // the receive side
    pub irs: u32,
    pub rcv_nxt: u32,
    /// the right edge of the window offered last
    rcv_adv: u32,
    /// RCV.NXT when the last ACK went
    rcv_wup: u32,
    rbuf: VecDeque<u8>,
    /// segments beyond a gap: (sequence number, bytes), in order
    ooo: Vec<(u32, Vec<u8>)>,
    /// the sequence number of a FIN beyond a gap
    ooo_fin: Option<u32>,
    pub fin_rcvd: bool,
    /// the window offered last was 0
    zero_adv: bool,
    // segment sizes and buffers
    /// the MSS this side announced, the peer's, and the one used for sending
    pub adv_mss: u16,
    pub peer_mss: u16,
    pub mss: u16,
    /// the window of the own SYN
    syn_wnd: u16,
    pub rcvbuf: u32,
    pub sndbuf: u32,
    /// the send buffer grows with cwnd (Linux, unless the service set it)
    snd_auto: bool,
    // congestion control (RFC 5681)
    pub cwnd: u32,
    pub ssthresh: u32,
    dupacks: u8,
    recovery: bool,
    /// the window of the last ACK (a duplicate ACK carries the same)
    last_wnd: u32,
    /// when new data went out last (restart after idle)
    lsndtime: Ns,
    /// the SYN or the SYN-ACK went again
    syn_retx: bool,
    // round-trip time and retransmission (RFC 6298)
    pub srtt: Option<Ns>,
    pub rttvar: Ns,
    pub rto: Ns,
    /// timeouts in a row for the same data
    pub retries: u8,
    segs: VecDeque<Sent>,
    // timers: the virtual time they fire at
    t_rto: Option<Ns>,
    t_persist: Option<Ns>,
    t_delack: Option<Ns>,
    t_keep: Option<Ns>,
    t_wait: Option<Ns>,
    /// zero-window probes without an answer, the backoff of their interval, probes of this
    /// zero window, keepalive probes without an answer
    persist_n: u8,
    persist_backoff: u8,
    probe_count: u8,
    keep_n: u8,
    // ACKs
    /// data or a FIN came that no ACK has covered yet
    ack_owed: bool,
    /// quick ACKs left (Linux)
    quick: u8,
    pingpong: bool,
    /// when data came last, and whether any came yet
    lrcvtime: Ns,
    got_data: bool,
    /// when any segment came last (keepalive)
    last_rcv: Ns,
    /// an ICMP error seen meanwhile: what a timeout reports
    soft_err: Option<TcpEnd>,
    /// a TcpData event is on its way (until the service reads)
    ev_data: bool,
    ip_id: u16,
    /// the amount Nagle holds back, as noted last
    nagle_noted: u32,
    pub since: Ns,
    pub c: TcpCounters,
}

/// what processing a connection asks the engine to do, in order
#[derive(Clone, Debug)]
enum Out {
    Seg(Seg),
    Note(Note),
    /// for a service's inbox
    Ev(u32, SvcEvent),
    /// a passive open completed: the connection joins the queue of its listening socket
    Accept,
    /// the connection is gone (CLOSED)
    Gone,
    /// CLOSED, but its service still holds the socket (it may read what is left): the
    /// address pair is free again, the socket goes with tcp_close
    Unindex,
}

/// a segment to send
#[derive(Clone, Debug)]
struct Seg {
    seq: u32,
    ack: u32,
    flags: u8,
    wnd: u16,
    mss: Option<u16>,
    data: Vec<u8>,
    ip_id: u16,
}

/// what a connection is processed with
struct Cx<'a> {
    now: Ns,
    cfg: &'a TcpCfg,
    /// the frame that led to this processing
    cause: Option<HopId>,
    out: Vec<Out>,
    stats: TcpStats,
}

impl Cx<'_> {
    fn hop(&self) -> HopId {
        self.cause.unwrap_or(0)
    }
}

/// a received segment
struct SegIn<'a> {
    seq: u32,
    ack: u32,
    flags: u8,
    wnd: u32,
    mss: Option<u16>,
    data: &'a [u8],
}

/// the acceptance test of RFC 9293 3.10.7.4
fn acceptable(seq: u32, len: u32, nxt: u32, wnd: u32) -> bool {
    let inside = |s: u32| seq_le(nxt, s) && seq_lt(s, nxt.wrapping_add(wnd));
    match (len, wnd) {
        (0, 0) => seq == nxt,
        (0, _) => inside(seq),
        (_, 0) => false,
        _ => inside(seq) || inside(seq.wrapping_add(len - 1)),
    }
}

/// the initial congestion window (RFC 6928 for Linux, RFC 5681 3.1 otherwise)
fn initial_window(cfg: &TcpCfg, mss: u32) -> u32 {
    if cfg.init_cwnd > 0 {
        cfg.init_cwnd * mss
    } else if mss > 2190 {
        2 * mss
    } else if mss > 1095 {
        3 * mss
    } else {
        4 * mss
    }
}

/// the window of a SYN: the buffer, at most 65535 (Linux: rounded down to the MSS)
fn syn_window(cfg: &TcpCfg, rcvbuf: u32, mss: u16) -> u16 {
    let w = rcvbuf.min(WND_MAX);
    let m = (mss as u32).max(1);
    (if cfg.wnd_round && w >= m { w / m * m } else { w }) as u16
}

impl Tcb {
    #[allow(clippy::too_many_arguments)]
    fn new(id: u32, local: (IpAddr, u16), remote: (IpAddr, u16), iss: u32, ip_id: u16, adv_mss: u16, opts: SockOpts, cfg: &TcpCfg, now: Ns) -> Tcb {
        let rcvbuf = if opts.rcvbuf > 0 { opts.rcvbuf } else { cfg.rcvbuf };
        let sndbuf = if opts.sndbuf > 0 { opts.sndbuf } else { cfg.sndbuf };
        let peer_mss = if remote.0.is_v4() { MSS_DEFAULT } else { MSS_DEFAULT6 };
        Tcb {
            id,
            state: TcpState::Closed,
            local,
            remote,
            owner: None,
            listener: None,
            passive: false,
            orphan: false,
            opts,
            iss,
            snd_una: iss,
            snd_nxt: iss,
            snd_max: iss,
            snd_wnd: 0,
            snd_wl1: 0,
            snd_wl2: 0,
            max_sndwnd: 0,
            sbuf: VecDeque::new(),
            sbuf_seq: iss.wrapping_add(1),
            fin_queued: false,
            more: false,
            want_write: false,
            snd_sml: iss,
            probe_byte: false,
            irs: 0,
            rcv_nxt: 0,
            rcv_adv: 0,
            rcv_wup: 0,
            rbuf: VecDeque::new(),
            ooo: Vec::new(),
            ooo_fin: None,
            fin_rcvd: false,
            zero_adv: false,
            adv_mss,
            peer_mss,
            mss: adv_mss,
            syn_wnd: syn_window(cfg, rcvbuf, adv_mss),
            rcvbuf,
            sndbuf,
            snd_auto: cfg.snd_auto && opts.sndbuf == 0,
            cwnd: initial_window(cfg, adv_mss as u32),
            ssthresh: SSTHRESH_INF,
            dupacks: 0,
            recovery: false,
            last_wnd: 0,
            lsndtime: now,
            syn_retx: false,
            srtt: None,
            rttvar: 0,
            rto: cfg.rto_init,
            retries: 0,
            segs: VecDeque::new(),
            t_rto: None,
            t_persist: None,
            t_delack: None,
            t_keep: None,
            t_wait: None,
            persist_n: 0,
            persist_backoff: 0,
            probe_count: 0,
            keep_n: 0,
            ack_owed: false,
            quick: 0,
            pingpong: false,
            lrcvtime: now,
            got_data: false,
            last_rcv: now,
            soft_err: None,
            ev_data: false,
            ip_id,
            nagle_noted: 0,
            since: now,
            c: TcpCounters::default(),
        }
    }

    // ------------------------------------------------------------ small helpers

    fn note(&self, n: Note, cx: &mut Cx) {
        cx.out.push(Out::Note(n));
    }
    fn event(&self, ev: SvcEvent, cx: &mut Cx) {
        if let Some(o) = self.owner.filter(|_| !self.orphan) {
            cx.out.push(Out::Ev(o, ev));
        }
    }
    fn set_state(&mut self, to: TcpState, cx: &mut Cx) {
        let from = self.state;
        if from == to {
            return;
        }
        self.state = to;
        match (from, to) {
            (TcpState::SynSent | TcpState::SynReceived, TcpState::Closed) => cx.stats.attempt_fails += 1,
            (TcpState::Established | TcpState::CloseWait, TcpState::Closed) => cx.stats.estab_resets += 1,
            _ => {}
        }
        self.note(Note::TcpStateChange { port: self.local.1, peer: self.remote.0, pport: self.remote.1, from, to }, cx);
    }
    /// the end of the data the service gave (the FIN's sequence number)
    fn data_end(&self) -> u32 {
        self.sbuf_seq.wrapping_add(self.sbuf.len() as u32)
    }
    fn fin_acked(&self) -> bool {
        self.fin_queued && seq_gt(self.snd_una, self.data_end())
    }
    /// sequence space sent and not acknowledged
    fn flight(&self) -> u32 {
        self.snd_max.wrapping_sub(self.snd_una)
    }
    /// data or a FIN not sent yet
    fn has_unsent(&self) -> bool {
        let end = self.data_end();
        seq_lt(self.snd_nxt, end) || (self.fin_queued && seq_le(self.snd_nxt, end))
    }
    fn unsent(&self) -> u32 {
        let end = self.data_end();
        if seq_lt(self.snd_nxt, end) { end.wrapping_sub(self.snd_nxt) } else { 0 }
    }
    /// what the send buffer holds at most
    fn snd_cap(&self) -> u32 {
        if self.snd_auto { self.sndbuf.max(self.cwnd.saturating_mul(2)).min(SNDBUF_AUTO_MAX) } else { self.sndbuf }
    }
    fn send_space(&self) -> u32 {
        self.snd_cap().saturating_sub(self.sbuf.len() as u32)
    }
    /// the window still offered to the peer from RCV.NXT
    fn offered(&self) -> u32 {
        if seq_gt(self.rcv_adv, self.rcv_nxt) { self.rcv_adv.wrapping_sub(self.rcv_nxt) } else { 0 }
    }
    /// the size of the peer's full segments
    fn rcv_mss(&self) -> u32 {
        (self.mss as u32).max(1)
    }
    /// the window the free buffer allows now (before the rule that it never shrinks)
    fn wnd_target(&self, cfg: &TcpCfg) -> u32 {
        let free = self.rcvbuf.saturating_sub(self.rbuf.len() as u32).min(WND_MAX);
        let mss = self.rcv_mss();
        if cfg.wnd_round {
            free / mss * mss
        } else {
            // RFC 9293 3.8.6.2.2: the right edge moves only by at least min(buffer / 2, MSS)
            let cur = self.offered();
            if free >= cur + (self.rcvbuf / 2).min(mss) { free } else { cur }
        }
    }
    /// the window to offer in a segment now
    fn rcv_window(&mut self, cfg: &TcpCfg) -> u16 {
        let edge = self.rcv_nxt.wrapping_add(self.wnd_target(cfg));
        if seq_gt(edge, self.rcv_adv) {
            self.rcv_adv = edge;
        }
        self.offered().min(WND_MAX) as u16
    }
    /// the room the windows leave for sending from snd_nxt (with limited transmit, RFC 3042)
    fn room(&self) -> u32 {
        let mut cwnd = self.cwnd;
        if !self.recovery && (1..=2).contains(&self.dupacks) {
            cwnd = cwnd.saturating_add(self.dupacks as u32 * self.mss as u32);
        }
        let edge = self.snd_una.wrapping_add(self.snd_wnd.min(cwnd));
        if seq_lt(self.snd_nxt, edge) { edge.wrapping_sub(self.snd_nxt) } else { 0 }
    }
    /// Nagle: unacknowledged data holds back a small segment (Minshall: only a small one)
    fn small_unacked(&self, cfg: &TcpCfg) -> bool {
        if self.flight() == 0 {
            return false;
        }
        !cfg.minshall || (seq_gt(self.snd_sml, self.snd_una) && seq_le(self.snd_sml, self.snd_max))
    }
    fn clear_timers(&mut self) {
        self.t_rto = None;
        self.t_persist = None;
        self.t_delack = None;
        self.t_keep = None;
        self.t_wait = None;
    }
    /// the earliest timer
    fn next_timer(&self) -> Option<Ns> {
        [self.t_rto, self.t_persist, self.t_delack, self.t_keep, self.t_wait].into_iter().flatten().min()
    }
    /// the timer ss shows and how long until it fires
    fn shown_timer(&self, now: Ns) -> Option<(TcpTimer, Ns)> {
        let left = |t: Ns| t.saturating_sub(now);
        if let Some(t) = self.t_rto {
            Some((TcpTimer::Retransmit, left(t)))
        } else if let Some(t) = self.t_persist {
            Some((TcpTimer::Persist, left(t)))
        } else if let Some(t) = self.t_wait.filter(|_| matches!(self.state, TcpState::TimeWait | TcpState::FinWait2)) {
            Some((TcpTimer::TimeWait, left(t)))
        } else if let Some(t) = self.t_keep {
            Some((TcpTimer::Keepalive, left(t)))
        } else {
            self.t_delack.map(|t| (TcpTimer::DelayedAck, left(t)))
        }
    }
    /// the peer's MSS option: the size of what this side sends (without one: 536 over IPv4,
    /// 1220 over IPv6)
    fn peer(&mut self, mss: Option<u16>) {
        let default = if self.remote.0.is_v4() { MSS_DEFAULT } else { MSS_DEFAULT6 };
        self.peer_mss = mss.unwrap_or(default).max(MSS_MIN);
        self.mss = self.peer_mss.min(self.adv_mss);
    }

    // ------------------------------------------------------------ sending segments

    /// a segment; one with ACK covers the received data (no delayed ACK left)
    fn emit(&mut self, seq: u32, flags: u8, data: Vec<u8>, mss: Option<u16>, cx: &mut Cx) {
        let wnd = if flags & RST != 0 {
            0
        } else if flags & SYN != 0 {
            self.rcv_adv = self.rcv_nxt.wrapping_add(self.syn_wnd as u32);
            self.syn_wnd
        } else {
            self.rcv_window(cx.cfg)
        };
        let ack = if flags & ACK != 0 { self.rcv_nxt } else { 0 };
        if flags & ACK != 0 && flags & RST == 0 {
            self.rcv_wup = self.rcv_nxt;
            self.ack_owed = false;
            self.t_delack = None;
            self.quick = self.quick.saturating_sub(1);
            let zero = wnd == 0 && flags & SYN == 0;
            if zero && !self.zero_adv {
                self.note(Note::TcpZeroWindow, cx);
            }
            self.zero_adv = zero;
        }
        self.ip_id = self.ip_id.wrapping_add(1);
        self.c.segs_out += 1;
        cx.stats.out_segs += 1;
        if flags & RST != 0 {
            cx.stats.out_rsts += 1;
        }
        cx.out.push(Out::Seg(Seg { seq, ack, flags, wnd, mss, data, ip_id: self.ip_id }));
    }
    /// <SEQ=SND.NXT><ACK=RCV.NXT><CTL=ACK>
    fn send_ack(&mut self, cx: &mut Cx) {
        self.emit(self.snd_max, ACK, Vec::new(), None, cx);
    }
    /// the SYN (SYN-SENT) or the SYN-ACK (SYN-RECEIVED), with the MSS option
    fn send_syn(&mut self, cx: &mut Cx) {
        let flags = if self.state == TcpState::SynSent { SYN } else { SYN | ACK };
        let mss = self.adv_mss;
        self.emit(self.iss, flags, Vec::new(), Some(mss), cx);
    }
    /// a RST that ends the connection from this side (Linux sends it with ACK)
    fn send_reset(&mut self, why: RstWhy, cx: &mut Cx) {
        self.note(Note::TcpRstSent { why }, cx);
        self.emit(self.snd_max, RST | ACK, Vec::new(), None, cx);
    }
    /// marks what went out before `upto` as sent again (Karn)
    fn mark_retx(&mut self, upto: u32) {
        let mut start = self.snd_una;
        for s in self.segs.iter_mut() {
            if !seq_lt(start, upto) {
                break;
            }
            s.retx = true;
            start = s.end;
        }
    }
    /// the first unacknowledged segment again, whatever the windows say; its length in
    /// sequence space
    fn retransmit_head(&mut self, cx: &mut Cx) -> u32 {
        let end = self.data_end();
        let una = self.snd_una;
        let avail = if seq_lt(una, end) { end.wrapping_sub(una) } else { 0 };
        let len = avail.min(self.mss as u32);
        let fin = self.fin_queued && seq_gt(self.snd_max, end) && len == avail;
        if len == 0 && !fin {
            return 0;
        }
        let off = una.wrapping_sub(self.sbuf_seq) as usize;
        let data: Vec<u8> = self.sbuf.range(off..off + len as usize).copied().collect();
        let mut flags = ACK;
        if len > 0 && una.wrapping_add(len) == end {
            flags |= PSH;
        }
        if fin {
            flags |= FIN;
        }
        self.emit(una, flags, data, None, cx);
        let n = len + fin as u32;
        self.mark_retx(una.wrapping_add(n));
        self.c.retrans += 1;
        cx.stats.retrans_segs += 1;
        if self.t_rto.is_none() {
            self.t_rto = Some(cx.now + self.rto);
        }
        n
    }

    /// sends what the windows, Nagle and the sender's SWS avoidance allow (RFC 9293 3.7.4,
    /// 3.8.6.2.1); with nothing in flight and too small a window the persist timer starts
    fn output(&mut self, cx: &mut Cx) {
        if !matches!(self.state, TcpState::Established | TcpState::CloseWait | TcpState::FinWait1 | TcpState::Closing | TcpState::LastAck) {
            return;
        }
        self.idle_restart(cx);
        let mss = self.mss as u32;
        loop {
            let end = self.data_end();
            let unsent = self.unsent();
            let fin_left = self.fin_queued && seq_le(self.snd_nxt, end);
            if unsent == 0 && !fin_left {
                break;
            }
            let room = self.room();
            let len = unsent.min(mss).min(room);
            let retx = seq_lt(self.snd_nxt, self.snd_max);
            let all = len == unsent;
            let fin = fin_left && all && room > len;
            let nagle_ok = self.opts.nodelay || !self.small_unacked(cx.cfg);
            let go = if len == 0 {
                fin
            } else {
                len == mss || retx || (all && !self.more && nagle_ok) || (len < unsent.min(mss) && len >= self.max_sndwnd / 2)
            };
            if !go {
                if len > 0 && all && len < mss && !self.more && !nagle_ok && self.nagle_noted == 0 {
                    self.nagle_noted = len;
                    self.note(Note::TcpNagle { bytes: len }, cx);
                }
                break;
            }
            let seq = self.snd_nxt;
            let off = seq.wrapping_sub(self.sbuf_seq) as usize;
            let data: Vec<u8> = self.sbuf.range(off..off + len as usize).copied().collect();
            let mut flags = ACK;
            if len > 0 && seq.wrapping_add(len) == end {
                flags |= PSH;
            }
            if fin {
                flags |= FIN;
            }
            self.emit(seq, flags, data, None, cx);
            let next = seq.wrapping_add(len + fin as u32);
            if retx {
                self.c.retrans += 1;
                cx.stats.retrans_segs += 1;
            }
            if seq_gt(next, self.snd_max) {
                // new sequence space: its time for the RTT measurement (RFC 6298)
                let new = next.wrapping_sub(if retx { self.snd_max } else { seq });
                self.segs.push_back(Sent { end: next, at: cx.now, retx });
                self.c.bytes_sent += new.saturating_sub(fin as u32) as u64;
                self.lsndtime = cx.now;
                // a reply soon after data came: an interactive connection (Linux ping-pong)
                if cx.cfg.pingpong && self.got_data && cx.now.saturating_sub(self.lrcvtime) < cx.cfg.delack {
                    self.pingpong = true;
                }
                self.snd_max = next;
            }
            self.snd_nxt = next;
            if len < mss {
                self.snd_sml = next;
            }
            self.nagle_noted = 0;
            if self.t_rto.is_none() {
                self.t_rto = Some(cx.now + self.rto);
            }
            if fin {
                break;
            }
        }
        // nothing in flight, but data waits for the window: probe it (RFC 9293 3.8.6.1)
        if self.t_persist.is_none() && self.t_rto.is_none() && (self.flight() == 0 || self.probe_byte) && self.has_unsent() {
            self.persist_backoff = 0;
            self.probe_count = 0;
            let after = self.persist_interval(cx.cfg);
            self.t_persist = Some(cx.now + after);
            self.note(Note::TcpPersist { after }, cx);
        }
    }

    /// RFC 5681 4.1: after an RTO without sending, the window starts again (Linux halves it
    /// for every RTO of idle time, down to the initial window)
    fn idle_restart(&mut self, cx: &mut Cx) {
        if !cx.cfg.idle_restart || self.flight() > 0 || !self.has_unsent() {
            return;
        }
        let idle = cx.now.saturating_sub(self.lsndtime);
        if idle <= self.rto {
            return;
        }
        let restart = initial_window(cx.cfg, self.mss as u32).min(self.cwnd);
        self.ssthresh = self.ssthresh.max(self.cwnd / 2 + self.cwnd / 4);
        let mut cwnd = self.cwnd;
        let mut d = idle;
        while d > self.rto && cwnd > restart {
            d -= self.rto;
            cwnd >>= 1;
        }
        self.cwnd = cwnd.max(restart);
        self.lsndtime = cx.now;
    }

    fn persist_interval(&self, cfg: &TcpCfg) -> Ns {
        let base = self.rto.max(cfg.rto_min);
        base.saturating_mul(1 << self.persist_backoff.min(20)).min(cfg.rto_max)
    }

    // ------------------------------------------------------------ opening

    /// an active open: the SYN goes (CLOSED -> SYN-SENT)
    fn open(&mut self, cx: &mut Cx) {
        self.set_state(TcpState::SynSent, cx);
        self.send_syn(cx);
        self.snd_nxt = self.iss.wrapping_add(1);
        self.snd_max = self.snd_nxt;
        self.segs.push_back(Sent { end: self.snd_nxt, at: cx.now, retx: false });
        self.t_rto = Some(cx.now + self.rto);
    }
    /// a SYN to a listening socket (LISTEN -> SYN-RECEIVED, RFC 9293 3.10.7.2); data in the
    /// SYN is not taken (no TCP Fast Open)
    fn accept_syn(&mut self, s: &SegIn, cx: &mut Cx) {
        self.passive = true;
        self.state = TcpState::Listen;
        self.irs = s.seq;
        self.rcv_nxt = s.seq.wrapping_add(1);
        self.rcv_wup = self.rcv_nxt;
        self.rcv_adv = self.rcv_nxt;
        self.peer(s.mss);
        self.snd_wnd = s.wnd;
        self.snd_wl1 = s.seq;
        self.max_sndwnd = s.wnd;
        self.last_wnd = s.wnd;
        self.set_state(TcpState::SynReceived, cx);
        self.send_syn(cx);
        self.snd_nxt = self.iss.wrapping_add(1);
        self.snd_max = self.snd_nxt;
        self.segs.push_back(Sent { end: self.snd_nxt, at: cx.now, retx: false });
        self.t_rto = Some(cx.now + self.rto);
    }
    /// the handshake is complete
    fn established(&mut self, cx: &mut Cx) {
        self.set_state(TcpState::Established, cx);
        let mss = self.mss as u32;
        // one segment after a lost SYN (RFC 5681 3.1), an RTO of 3 s without a measurement
        // (RFC 6298 5.7)
        self.cwnd = if self.syn_retx { mss } else { initial_window(cx.cfg, mss) };
        if self.syn_retx && self.srtt.is_none() {
            self.rto = self.rto.max(3 * SEC);
        }
        self.retries = 0;
        self.lsndtime = cx.now;
        self.t_wait = None;
        if self.opts.keepalive {
            self.t_keep = Some(cx.now + cx.cfg.keep_idle);
        }
        if self.passive {
            cx.stats.passive_opens += 1;
            cx.out.push(Out::Accept);
        } else {
            let hop = cx.hop();
            self.event(SvcEvent::TcpConnected { sock: self.id, hop }, cx);
        }
    }

    // ------------------------------------------------------------ ending

    /// a service holds the socket (and has not closed it)
    fn held(&self) -> bool {
        self.owner.is_some() && !self.orphan
    }
    /// the connection is over for the service; it keeps the socket until tcp_close, so it can
    /// still read what came (as a Linux socket in CLOSE)
    fn release(&mut self, why: TcpEnd, cx: &mut Cx) {
        self.event(SvcEvent::TcpClosed { sock: self.id, end: why }, cx);
    }
    /// gone, or kept for the service that still holds it
    fn bury(&self, cx: &mut Cx) {
        cx.out.push(if self.held() { Out::Unindex } else { Out::Gone });
    }
    /// CLOSED: the service hears why
    fn end(&mut self, why: TcpEnd, cx: &mut Cx) {
        if why == TcpEnd::Timeout {
            cx.stats.timeouts += 1;
        }
        self.set_state(TcpState::Closed, cx);
        self.clear_timers();
        self.release(why, cx);
        self.bury(cx);
    }
    /// a connection attempt of a listening socket goes quietly (RFC 9293: back to LISTEN)
    fn drop_half(&mut self, cx: &mut Cx) {
        self.set_state(TcpState::Closed, cx);
        self.clear_timers();
        cx.out.push(Out::Gone);
    }
    fn time_wait(&mut self, cx: &mut Cx) {
        self.set_state(TcpState::TimeWait, cx);
        self.clear_timers();
        self.t_wait = Some(cx.now + cx.cfg.time_wait);
        self.release(TcpEnd::Closed, cx);
    }

    // ------------------------------------------------------------ segment arrives

    /// a segment for this connection (RFC 9293 3.10.7.3 and 3.10.7.4)
    fn input(&mut self, s: &SegIn, cx: &mut Cx) {
        self.c.segs_in += 1;
        self.last_rcv = cx.now;
        self.keep_n = 0;
        if self.state == TcpState::SynSent {
            self.syn_sent(s, cx);
            return;
        }
        let rst = s.flags & RST != 0;
        let has_ack = s.flags & ACK != 0;
        let mut syn = s.flags & SYN != 0;
        let mut fin = s.flags & FIN != 0;
        let mut seq = s.seq;
        let mut data = s.data;
        if self.state == TcpState::SynReceived && syn && !rst {
            if self.passive && !has_ack && seq == self.irs {
                // the SYN again: our SYN-ACK was lost or is late; it goes again at once (Linux)
                self.note(Note::TcpSynAgain, cx);
                self.c.retrans += 1;
                cx.stats.retrans_segs += 1;
                self.send_syn(cx);
                return;
            }
            if !self.passive && has_ack && seq.wrapping_add(1) == self.rcv_nxt && s.ack == self.snd_max {
                // simultaneous open: the peer's SYN-ACK repeats its SYN, which we have, and
                // acknowledges ours (RFC 9293 figure 8)
                syn = false;
                seq = self.rcv_nxt;
            }
        }
        // first: the sequence number
        let len = data.len() as u32 + syn as u32 + fin as u32;
        let wnd = self.offered();
        let mut ack_now = false;
        if !acceptable(seq, len, self.rcv_nxt, wnd) {
            if rst {
                return;
            }
            if len > 0 && !data.is_empty() && seq_le(seq.wrapping_add(len), self.rcv_nxt) {
                // all of it came before (a retransmission): the ACK tells the sender at once
                self.c.dup_in += 1;
                self.note(Note::TcpDuplicate { bytes: data.len() as u32 }, cx);
            } else {
                self.note(Note::TcpUnacceptable { seq, len }, cx);
            }
            if wnd == 0 && seq == self.rcv_nxt && !syn && has_ack {
                // a zero window takes no data, but the ACK still counts (RFC 9293 3.10.7.4)
                data = &[];
                fin = false;
                ack_now = true;
            } else {
                if self.state == TcpState::TimeWait && fin {
                    // the peer's FIN again: our last ACK was lost, TIME-WAIT starts over
                    self.t_wait = Some(cx.now + cx.cfg.time_wait);
                }
                self.send_ack(cx);
                return;
            }
        }
        // second: RST (RFC 5961 3.2: only an exact one counts)
        if rst {
            if seq != self.rcv_nxt {
                self.note(Note::TcpChallenge { syn: false }, cx);
                self.send_ack(cx);
                return;
            }
            self.reset_in(cx);
            return;
        }
        // fourth: SYN (RFC 5961 4.2)
        if syn {
            if self.state == TcpState::SynReceived && self.passive {
                self.drop_half(cx);
                return;
            }
            self.note(Note::TcpChallenge { syn: true }, cx);
            self.send_ack(cx);
            return;
        }
        // what lies before RCV.NXT came before, what lies beyond the window does not fit
        if seq_lt(seq, self.rcv_nxt) {
            let old = self.rcv_nxt.wrapping_sub(seq) as usize;
            if old > data.len() {
                fin = false;
            }
            let cut = old.min(data.len());
            if cut > 0 {
                self.c.dup_in += 1;
                self.note(Note::TcpDuplicate { bytes: cut as u32 }, cx);
                ack_now = true;
            }
            data = &data[cut..];
            seq = seq.wrapping_add(cut as u32);
        }
        let room = self.rcv_nxt.wrapping_add(wnd).wrapping_sub(seq) as usize;
        if data.len() > room {
            data = &data[..room];
            fin = false;
        }
        // fifth: ACK
        if !has_ack {
            return;
        }
        if self.state == TcpState::SynReceived {
            if seq_lt(self.snd_una, s.ack) && seq_le(s.ack, self.snd_max) {
                self.snd_wnd = s.wnd;
                self.snd_wl1 = s.seq;
                self.snd_wl2 = s.ack;
                self.max_sndwnd = self.max_sndwnd.max(s.wnd);
                self.last_wnd = s.wnd;
                self.established(cx);
            } else {
                self.note(Note::TcpRstSent { why: RstWhy::BadAck }, cx);
                self.emit(s.ack, RST, Vec::new(), None, cx);
                return;
            }
        }
        if seq_gt(s.ack, self.snd_max) {
            // an acknowledgement of something not sent yet
            self.send_ack(cx);
            return;
        }
        if seq_ge(s.ack, self.snd_una) {
            self.ack_in(s, cx);
        }
        if self.fin_acked() {
            match self.state {
                TcpState::FinWait1 => {
                    self.set_state(TcpState::FinWait2, cx);
                    if self.orphan {
                        self.t_wait = Some(cx.now + cx.cfg.fin_timeout);
                    }
                }
                TcpState::Closing => self.time_wait(cx),
                TcpState::LastAck => {
                    self.end(TcpEnd::Closed, cx);
                    return;
                }
                _ => {}
            }
        }
        // seventh and eighth: the text and the FIN
        if (!data.is_empty() || fin) && matches!(self.state, TcpState::Established | TcpState::FinWait1 | TcpState::FinWait2) {
            if self.orphan && !data.is_empty() {
                // data for a socket its service closed: the connection is reset (Linux)
                self.send_reset(RstWhy::DataAfterClose, cx);
                self.end(TcpEnd::Aborted, cx);
                return;
            }
            ack_now |= self.data_in(seq, data, fin, cx);
        }
        if ack_now {
            self.ack_owed = true;
        }
        self.output(cx);
        self.ack_check(ack_now, cx);
    }

    /// sends the owed ACK now, or lets it wait (delayed ACK)
    fn ack_check(&mut self, now: bool, cx: &mut Cx) {
        if !self.ack_owed {
            return;
        }
        let quick = self.quick > 0 && !self.pingpong;
        let unacked = self.rcv_nxt.wrapping_sub(self.rcv_wup);
        let full = if cx.cfg.ack_over_one { unacked > self.rcv_mss() } else { unacked >= 2 * self.rcv_mss() };
        if now || quick || full || !self.ooo.is_empty() {
            self.send_ack(cx);
        } else if self.t_delack.is_none() {
            self.t_delack = Some(cx.now + cx.cfg.delack);
            self.note(Note::TcpAckDelayed { delay: cx.cfg.delack }, cx);
        }
    }

    /// the reply to our SYN (RFC 9293 3.10.7.3)
    fn syn_sent(&mut self, s: &SegIn, cx: &mut Cx) {
        let rst = s.flags & RST != 0;
        let has_ack = s.flags & ACK != 0;
        if has_ack && (seq_le(s.ack, self.iss) || seq_gt(s.ack, self.snd_max)) {
            if !rst {
                self.note(Note::TcpRstSent { why: RstWhy::BadAck }, cx);
                self.emit(s.ack, RST, Vec::new(), None, cx);
            }
            return;
        }
        if rst {
            if has_ack {
                self.note(Note::TcpReset { refused: true }, cx);
                self.end(TcpEnd::Refused, cx);
            }
            return;
        }
        if s.flags & SYN == 0 {
            return;
        }
        self.irs = s.seq;
        self.rcv_nxt = s.seq.wrapping_add(1);
        self.rcv_wup = self.rcv_nxt;
        self.rcv_adv = self.rcv_nxt.wrapping_add(self.syn_wnd as u32);
        self.peer(s.mss);
        self.snd_wnd = s.wnd;
        self.snd_wl1 = s.seq;
        self.snd_wl2 = s.ack;
        self.max_sndwnd = s.wnd;
        self.last_wnd = s.wnd;
        if has_ack {
            // our SYN is acknowledged
            self.snd_una = s.ack;
            if let Some(r) = self.rtt_sample(s.ack, cx.now) {
                self.rtt_update(r, cx.cfg);
            }
            self.t_rto = None;
            self.established(cx);
            self.send_ack(cx);
            self.output(cx);
        } else {
            // simultaneous open: the SYNs crossed (RFC 9293 figure 8)
            self.set_state(TcpState::SynReceived, cx);
            self.send_syn(cx);
            self.t_rto = Some(cx.now + self.rto);
        }
    }

    /// an exact RST (RFC 9293 3.10.7.4, second)
    fn reset_in(&mut self, cx: &mut Cx) {
        match self.state {
            TcpState::SynReceived if self.passive => {
                self.note(Note::TcpReset { refused: false }, cx);
                self.drop_half(cx);
            }
            TcpState::SynReceived => {
                self.note(Note::TcpReset { refused: true }, cx);
                self.end(TcpEnd::Refused, cx);
            }
            TcpState::TimeWait => {
                // the service heard of the end already when TIME-WAIT began
                self.note(Note::TcpReset { refused: false }, cx);
                self.set_state(TcpState::Closed, cx);
                self.clear_timers();
                self.bury(cx);
            }
            _ => {
                self.note(Note::TcpReset { refused: false }, cx);
                self.end(TcpEnd::Reset, cx);
            }
        }
    }

    /// the ACK field of an acceptable segment (RFC 9293 3.10.7.4 fifth; RFC 5681; RFC 6298)
    fn ack_in(&mut self, s: &SegIn, cx: &mut Cx) {
        let (ack, seq) = (s.ack, s.seq);
        self.persist_n = 0;
        if self.probe_byte && ack == self.snd_una && s.wnd > 0 {
            // the window opened, the probe byte was not taken: it goes again with the data
            self.snd_nxt = self.snd_una;
            self.snd_max = self.snd_una;
            self.probe_byte = false;
        }
        if seq_gt(ack, self.snd_una) {
            self.probe_byte = false;
            let mut n = ack.wrapping_sub(self.snd_una);
            if self.snd_una == self.iss {
                n -= 1;
            }
            let fin_now = self.fin_queued && seq_gt(ack, self.data_end());
            if fin_now {
                n -= 1;
            }
            let bytes = n.min(self.sbuf.len() as u32);
            self.sbuf.drain(..bytes as usize);
            self.sbuf_seq = self.sbuf_seq.wrapping_add(bytes);
            self.snd_una = ack;
            if seq_lt(self.snd_nxt, ack) {
                self.snd_nxt = ack;
            }
            self.c.bytes_acked += bytes as u64;
            let rtt = self.rtt_sample(ack, cx.now);
            if let Some(r) = rtt {
                self.rtt_update(r, cx.cfg);
            }
            self.retries = 0;
            let mss = self.mss as u32;
            if self.recovery {
                // Reno: the first new ACK ends fast recovery (RFC 5681 3.2, step 6)
                self.recovery = false;
                self.cwnd = self.ssthresh;
                self.note(Note::TcpRecoveryEnd { cwnd: self.cwnd }, cx);
            } else if bytes > 0 {
                self.cwnd = if self.cwnd < self.ssthresh {
                    // slow start: at most one MSS per ACK
                    self.cwnd.saturating_add(bytes.min(mss))
                } else {
                    // congestion avoidance: about one MSS per round trip
                    self.cwnd.saturating_add(((mss as u64 * mss as u64) / self.cwnd.max(1) as u64).max(1) as u32)
                };
            }
            self.dupacks = 0;
            // RFC 6298 5.2 and 5.3
            self.t_rto = if self.flight() == 0 { None } else { Some(cx.now + self.rto) };
            if bytes > 0 {
                self.note(Note::TcpAcked { bytes, cwnd: self.cwnd, rtt }, cx);
            }
            if self.want_write && self.send_space() >= self.snd_cap() / 3 {
                self.want_write = false;
                self.event(SvcEvent::TcpWritable { sock: self.id }, cx);
            }
        } else if ack == self.snd_una && s.data.is_empty() && s.flags & (SYN | FIN) == 0 && s.wnd == self.last_wnd && self.flight() > 0 && !self.probe_byte {
            // a duplicate ACK (RFC 5681 2)
            self.dupacks = self.dupacks.saturating_add(1);
            self.c.dupacks_in += 1;
            self.note(Note::TcpDupAck { n: self.dupacks }, cx);
            if self.recovery {
                // one more segment left the network
                self.cwnd = self.cwnd.saturating_add(self.mss as u32);
            } else if self.dupacks == 3 {
                self.fast_retransmit(cx);
            }
        }
        // the send window; SND.WL1 and SND.WL2 keep older segments from changing it
        if seq_le(self.snd_una, ack) && (seq_lt(self.snd_wl1, seq) || (self.snd_wl1 == seq && seq_le(self.snd_wl2, ack))) {
            self.snd_wnd = s.wnd;
            self.snd_wl1 = seq;
            self.snd_wl2 = ack;
            self.max_sndwnd = self.max_sndwnd.max(s.wnd);
        }
        self.last_wnd = s.wnd;
        // the window takes the next segment again: no more probes (Linux tcp_ack_probe)
        if self.t_persist.is_some() {
            let unsent = self.unsent();
            let need = unsent.min(self.mss as u32).max(1);
            let free = self.snd_wnd.saturating_sub(self.snd_nxt.wrapping_sub(self.snd_una));
            if free >= need {
                self.t_persist = None;
                self.persist_backoff = 0;
                self.probe_count = 0;
            }
        }
    }

    /// three duplicate ACKs: fast retransmit and fast recovery (RFC 5681 3.2)
    fn fast_retransmit(&mut self, cx: &mut Cx) {
        let mss = self.mss as u32;
        self.ssthresh = (self.flight() / 2).max(2 * mss);
        self.recovery = true;
        self.c.fast_retrans += 1;
        let cwnd = self.ssthresh + 3 * mss;
        self.note(Note::TcpFastRetransmit { seq: self.snd_una, ssthresh: self.ssthresh, cwnd }, cx);
        self.retransmit_head(cx);
        self.cwnd = cwnd;
    }

    /// the RTT of the oldest segment this ACK covers completely, unless any of them went
    /// twice (Karn)
    fn rtt_sample(&mut self, ack: u32, now: Ns) -> Option<Ns> {
        let mut first = None;
        let mut retx = false;
        while let Some(s) = self.segs.front() {
            if !seq_le(s.end, ack) {
                break;
            }
            retx |= s.retx;
            if first.is_none() {
                first = Some(s.at);
            }
            self.segs.pop_front();
        }
        if retx { None } else { first.map(|t| now.saturating_sub(t)) }
    }

    /// RFC 6298 2.2 and 2.3; the bounds of the system; a valid measurement ends the backoff
    fn rtt_update(&mut self, r: Ns, cfg: &TcpCfg) {
        let r = r.max(1);
        match self.srtt {
            None => {
                self.srtt = Some(r);
                self.rttvar = r / 2;
            }
            Some(s) => {
                let diff = s.abs_diff(r);
                self.rttvar = (3 * self.rttvar + diff) / 4;
                self.srtt = Some((7 * s + r) / 8);
            }
        }
        let s = self.srtt.unwrap_or(r);
        self.rto = (s + cfg.granularity.max(4 * self.rttvar)).clamp(cfg.rto_min, cfg.rto_max);
    }

    /// Linux's bookkeeping when data comes: quick ACKs at the start and after a long pause
    fn rcv_event(&mut self, cx: &mut Cx) {
        if cx.cfg.quickack > 0 {
            // tcp_incr_quickack: half the window in full segments (2 when that is none)
            let half = self.offered() / (2 * self.rcv_mss());
            let quickacks = (if half == 0 { 2 } else { half }).min(cx.cfg.quickack as u32) as u8;
            if !self.got_data || cx.now.saturating_sub(self.lrcvtime) > self.rto {
                self.quick = self.quick.max(quickacks);
            }
        }
        self.got_data = true;
        self.lrcvtime = cx.now;
    }

    /// data (and a FIN) at or beyond RCV.NXT inside the window; true when the ACK must go
    /// at once
    fn data_in(&mut self, seq: u32, data: &[u8], fin: bool, cx: &mut Cx) -> bool {
        if !data.is_empty() {
            self.rcv_event(cx);
        }
        let had_ooo = !self.ooo.is_empty() || self.ooo_fin.is_some();
        if seq != self.rcv_nxt {
            // beyond a gap: kept for later; the duplicate ACK tells the sender at once
            // (RFC 5681 4.2)
            if !data.is_empty() {
                self.ooo_add(seq, data);
            }
            if fin {
                self.ooo_fin = Some(seq.wrapping_add(data.len() as u32));
            }
            self.c.ooo_in += 1;
            self.note(Note::TcpOutOfOrder { seq, bytes: data.len() as u32 }, cx);
            self.ack_owed = true;
            return true;
        }
        let mut now = had_ooo;
        let mut got = data.len() as u32;
        self.rbuf.extend(data);
        self.rcv_nxt = self.rcv_nxt.wrapping_add(got);
        if got > 0 {
            self.note(Note::TcpReceived { bytes: got }, cx);
        }
        let filled = self.ooo_pull();
        if filled > 0 {
            self.note(Note::TcpGapFilled { bytes: filled }, cx);
            got += filled;
            now = true;
        }
        self.c.bytes_received += got as u64;
        if got > 0 {
            self.ack_owed = true;
            if !self.ev_data && self.owner.is_some() && !self.orphan {
                self.ev_data = true;
                let hop = cx.hop();
                self.event(SvcEvent::TcpData { sock: self.id, hop }, cx);
            }
        }
        if fin || self.ooo_fin == Some(self.rcv_nxt) {
            self.ooo_fin = None;
            self.ack_owed = true;
            now |= self.fin_in(cx);
        }
        now
    }

    /// keeps a segment beyond a gap, in order of its sequence number
    fn ooo_add(&mut self, seq: u32, data: &[u8]) {
        let nxt = self.rcv_nxt;
        let off = |s: u32| s.wrapping_sub(nxt);
        let pos = self.ooo.partition_point(|(s, _)| off(*s) < off(seq));
        if self.ooo.get(pos).is_some_and(|(s, d)| *s == seq && d.len() >= data.len()) {
            return;
        }
        self.ooo.insert(pos, (seq, data.to_vec()));
    }
    /// what the gap held back, now in order; the bytes it gave
    fn ooo_pull(&mut self) -> u32 {
        let mut n = 0;
        while let Some((s, _)) = self.ooo.first() {
            if seq_gt(*s, self.rcv_nxt) {
                break;
            }
            let (s, d) = self.ooo.remove(0);
            let skip = self.rcv_nxt.wrapping_sub(s) as usize;
            if skip < d.len() {
                let part = &d[skip..];
                self.rbuf.extend(part);
                self.rcv_nxt = self.rcv_nxt.wrapping_add(part.len() as u32);
                n += part.len() as u32;
            }
        }
        n
    }

    /// the peer's FIN in order (RFC 9293 3.10.7.4 eighth); true when the ACK goes at once
    fn fin_in(&mut self, cx: &mut Cx) -> bool {
        self.rcv_nxt = self.rcv_nxt.wrapping_add(1);
        self.fin_rcvd = true;
        let hop = cx.hop();
        self.event(SvcEvent::TcpEof { sock: self.id, hop }, cx);
        match self.state {
            TcpState::SynReceived | TcpState::Established => {
                self.set_state(TcpState::CloseWait, cx);
                // Linux: the ACK waits, so that the own FIN can carry it
                if cx.cfg.pingpong {
                    self.pingpong = true;
                    false
                } else {
                    true
                }
            }
            TcpState::FinWait1 => {
                // simultaneous close
                self.set_state(TcpState::Closing, cx);
                true
            }
            TcpState::FinWait2 => {
                self.time_wait(cx);
                true
            }
            _ => true,
        }
    }

    // ------------------------------------------------------------ timers

    /// every timer that is due, in a fixed order
    fn on_timers(&mut self, cx: &mut Cx) {
        let now = cx.now;
        let due = |t: Option<Ns>| t.is_some_and(|t| t <= now);
        if due(self.t_delack) {
            self.on_delack(cx);
        }
        if due(self.t_rto) && self.state != TcpState::Closed {
            self.on_rto(cx);
        }
        if due(self.t_persist) && self.state != TcpState::Closed {
            self.on_persist(cx);
        }
        if due(self.t_keep) && self.state != TcpState::Closed {
            self.on_keep(cx);
        }
        if due(self.t_wait) && self.state != TcpState::Closed {
            self.on_wait(cx);
        }
    }

    fn on_delack(&mut self, cx: &mut Cx) {
        self.t_delack = None;
        if !self.ack_owed {
            return;
        }
        // a delayed ACK that had to go on its own ends ping-pong mode (Linux)
        self.pingpong = false;
        cx.stats.delayed_acks += 1;
        self.note(Note::TcpDelayedAck, cx);
        self.send_ack(cx);
    }

    /// the retransmission timer (RFC 6298 5.4 to 5.7, RFC 5681 3.1)
    fn on_rto(&mut self, cx: &mut Cx) {
        self.t_rto = None;
        let cfg = cx.cfg;
        match self.state {
            TcpState::SynSent | TcpState::SynReceived => {
                let limit = if self.state == TcpState::SynSent { cfg.syn_retries } else { cfg.synack_retries };
                if self.retries >= limit {
                    let why = self.soft_err.unwrap_or(TcpEnd::Timeout);
                    self.note(Note::TcpGiveUp { why }, cx);
                    if self.passive {
                        self.drop_half(cx);
                    } else {
                        self.end(why, cx);
                    }
                    return;
                }
                self.retries += 1;
                self.syn_retx = true;
                self.rto = (self.rto * 2).min(cfg.rto_max);
                self.c.timeouts += 1;
                self.c.retrans += 1;
                cx.stats.retrans_segs += 1;
                self.note(Note::TcpRto { seq: self.iss, rto: self.rto, n: self.retries }, cx);
                self.mark_retx(self.iss.wrapping_add(1));
                self.send_syn(cx);
                self.t_rto = Some(cx.now + self.rto);
            }
            TcpState::Established | TcpState::CloseWait | TcpState::FinWait1 | TcpState::Closing | TcpState::LastAck => {
                if self.flight() == 0 || self.probe_byte {
                    return;
                }
                let limit = if self.orphan { cfg.orphan_retries } else { cfg.retries2 };
                if self.retries >= limit {
                    let why = self.soft_err.unwrap_or(TcpEnd::Timeout);
                    self.note(Note::TcpGiveUp { why }, cx);
                    self.end(why, cx);
                    return;
                }
                self.retries += 1;
                let mss = self.mss as u32;
                // RFC 5681 (4): ssthresh from the first timeout; held while the same data
                // times out again
                if self.retries == 1 {
                    self.ssthresh = (self.flight() / 2).max(2 * mss);
                }
                self.cwnd = mss;
                self.recovery = false;
                self.dupacks = 0;
                self.rto = (self.rto * 2).min(cfg.rto_max);
                self.c.timeouts += 1;
                self.note(Note::TcpRto { seq: self.snd_una, rto: self.rto, n: self.retries }, cx);
                // go back: everything after SND.UNA goes again as the window grows
                self.mark_retx(self.snd_max);
                let n = self.retransmit_head(cx);
                self.snd_nxt = self.snd_una.wrapping_add(n);
                self.t_rto = Some(cx.now + self.rto);
            }
            _ => {}
        }
    }

    /// the persist timer: a probe of the zero window (RFC 9293 3.8.6.1)
    fn on_persist(&mut self, cx: &mut Cx) {
        self.t_persist = None;
        if !(self.flight() == 0 || self.probe_byte) || !self.has_unsent() {
            return;
        }
        let limit = if self.orphan { cx.cfg.orphan_retries } else { cx.cfg.retries2 };
        // answered probes go on for ever, except for a socket its service closed (Linux)
        if self.persist_n >= limit || (self.orphan && self.persist_backoff >= limit) {
            self.note(Note::TcpGiveUp { why: TcpEnd::Timeout }, cx);
            self.end(TcpEnd::Timeout, cx);
            return;
        }
        let mss = self.mss as u32;
        let end = self.data_end();
        let unsent = self.unsent();
        let free = self.snd_wnd.saturating_sub(self.snd_nxt.wrapping_sub(self.snd_una));
        self.probe_count = self.probe_count.saturating_add(1);
        self.c.probes += 1;
        if free > 0 && unsent > 0 && !self.probe_byte {
            // the window is open, only too small for the SWS rule: what fits goes (Linux
            // tcp_write_wakeup); the retransmission timer takes over
            let len = free.min(mss).min(unsent);
            let seq = self.snd_nxt;
            let off = seq.wrapping_sub(self.sbuf_seq) as usize;
            let data: Vec<u8> = self.sbuf.range(off..off + len as usize).copied().collect();
            self.note(Note::TcpProbe { n: self.probe_count }, cx);
            self.emit(seq, ACK | PSH, data, None, cx);
            let next = seq.wrapping_add(len);
            self.segs.push_back(Sent { end: next, at: cx.now, retx: false });
            self.c.bytes_sent += len as u64;
            self.snd_nxt = next;
            self.snd_max = next;
            self.snd_sml = next;
            self.lsndtime = cx.now;
            self.t_rto = Some(cx.now + self.rto);
            self.persist_backoff = 0;
            return;
        }
        self.persist_n += 1;
        self.note(Note::TcpProbe { n: self.probe_count }, cx);
        if cx.cfg.probe_data && seq_lt(self.snd_una, end) {
            // a byte beyond the window
            let seq = self.snd_una;
            let off = seq.wrapping_sub(self.sbuf_seq) as usize;
            let byte = self.sbuf.get(off).copied().unwrap_or(0);
            self.emit(seq, ACK | PSH, alloc::vec![byte], None, cx);
            if !self.probe_byte {
                self.probe_byte = true;
                self.snd_nxt = seq.wrapping_add(1);
                if seq_gt(self.snd_nxt, self.snd_max) {
                    self.snd_max = self.snd_nxt;
                }
            }
        } else {
            // Linux: an empty segment just below SND.UNA, which the peer answers with its window
            self.emit(self.snd_una.wrapping_sub(1), ACK, Vec::new(), None, cx);
        }
        self.persist_backoff = self.persist_backoff.saturating_add(1);
        self.t_persist = Some(cx.now + self.persist_interval(cx.cfg));
    }

    /// keepalive (RFC 9293 3.8.4): probes after an idle time, a RST when none is answered
    fn on_keep(&mut self, cx: &mut Cx) {
        self.t_keep = None;
        if !self.opts.keepalive || matches!(self.state, TcpState::Closed | TcpState::Listen | TcpState::SynSent | TcpState::SynReceived | TcpState::TimeWait) {
            return;
        }
        let cfg = cx.cfg;
        if self.flight() > 0 || self.has_unsent() {
            // data is under way: the retransmission timer watches the peer
            self.t_keep = Some(cx.now + cfg.keep_idle);
            return;
        }
        let idle = cx.now.saturating_sub(self.last_rcv);
        if idle < cfg.keep_idle {
            self.t_keep = Some(self.last_rcv + cfg.keep_idle);
            return;
        }
        if self.keep_n >= cfg.keep_probes {
            self.note(Note::TcpGiveUp { why: TcpEnd::Timeout }, cx);
            self.send_reset(RstWhy::Keepalive, cx);
            self.end(TcpEnd::Timeout, cx);
            return;
        }
        self.keep_n += 1;
        self.c.probes += 1;
        self.note(Note::TcpKeepalive { n: self.keep_n }, cx);
        // an empty segment just below SND.UNA: the peer must answer it with an ACK
        self.emit(self.snd_una.wrapping_sub(1), ACK, Vec::new(), None, cx);
        self.t_keep = Some(cx.now + cfg.keep_intvl);
    }

    /// TIME-WAIT is over; a closed socket gave up waiting in FIN-WAIT-2; IOS gave up the
    /// connection attempt
    fn on_wait(&mut self, cx: &mut Cx) {
        self.t_wait = None;
        match self.state {
            TcpState::TimeWait => {
                self.set_state(TcpState::Closed, cx);
                self.clear_timers();
                self.bury(cx);
            }
            TcpState::FinWait2 if self.orphan => {
                self.note(Note::TcpGiveUp { why: TcpEnd::Timeout }, cx);
                self.set_state(TcpState::Closed, cx);
                self.clear_timers();
                cx.out.push(Out::Gone);
            }
            TcpState::SynSent => {
                let why = self.soft_err.unwrap_or(TcpEnd::Timeout);
                self.note(Note::TcpGiveUp { why }, cx);
                self.end(why, cx);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ the service's calls

    /// queues bytes for sending; how many the send buffer took (RFC 9293 3.10.2)
    fn write(&mut self, data: &[u8], cx: &mut Cx) -> usize {
        if self.fin_queued || !matches!(self.state, TcpState::SynSent | TcpState::SynReceived | TcpState::Established | TcpState::CloseWait) {
            return 0;
        }
        let n = (self.send_space() as usize).min(data.len());
        self.sbuf.extend(&data[..n]);
        self.more = n < data.len();
        self.want_write = self.more;
        self.output(cx);
        n
    }

    /// reads up to `max` bytes; a window update follows when the window grew a lot (Linux
    /// tcp_cleanup_rbuf)
    fn read(&mut self, max: usize, cx: &mut Cx) -> Vec<u8> {
        let n = max.min(self.rbuf.len());
        let out: Vec<u8> = self.rbuf.drain(..n).collect();
        self.ev_data = false;
        if n > 0 && !self.fin_rcvd && matches!(self.state, TcpState::Established | TcpState::FinWait1 | TcpState::FinWait2) {
            let cur = self.offered();
            let new = self.wnd_target(cx.cfg);
            let clamp = WND_MAX.min(self.rcvbuf);
            if new > cur && (cur == 0 || (2 * cur <= clamp && new >= 2 * cur)) {
                self.note(Note::TcpWindowUpdate { wnd: new.min(WND_MAX) as u16 }, cx);
                self.send_ack(cx);
            } else if self.ack_owed && self.rcv_nxt.wrapping_sub(self.rcv_wup) > self.rcv_mss() {
                self.send_ack(cx);
            }
        }
        out
    }

    /// no more sending: a FIN after the queued data (RFC 9293 3.10.4, CLOSE)
    fn shutdown(&mut self, cx: &mut Cx) {
        if self.fin_queued {
            return;
        }
        // nothing more comes from the service: what is queued goes, small rest included
        self.more = false;
        self.want_write = false;
        match self.state {
            TcpState::SynSent => {
                // nothing was said yet: the attempt just ends (Linux)
                self.set_state(TcpState::Closed, cx);
                self.clear_timers();
                self.release(TcpEnd::Closed, cx);
                self.bury(cx);
            }
            TcpState::SynReceived | TcpState::Established => {
                self.fin_queued = true;
                self.set_state(TcpState::FinWait1, cx);
                self.output(cx);
            }
            TcpState::CloseWait => {
                self.fin_queued = true;
                self.set_state(TcpState::LastAck, cx);
                self.output(cx);
            }
            _ => {}
        }
    }

    /// the service closes the socket; the connection ends in the kernel.  Unread bytes reset
    /// it (Linux), so the peer learns they were lost.
    fn close(&mut self, cx: &mut Cx) {
        let open = !matches!(self.state, TcpState::Closed | TcpState::TimeWait | TcpState::SynSent | TcpState::Listen);
        if open && (!self.rbuf.is_empty() || !self.ooo.is_empty()) {
            self.orphan = true;
            self.send_reset(RstWhy::Unread, cx);
            self.end(TcpEnd::Aborted, cx);
            return;
        }
        self.orphan = true;
        self.owner = None;
        match self.state {
            // the connection was over already: the socket goes
            TcpState::Closed => cx.out.push(Out::Gone),
            TcpState::SynSent | TcpState::SynReceived | TcpState::Established | TcpState::CloseWait => self.shutdown(cx),
            TcpState::FinWait2 => self.t_wait = Some(cx.now + cx.cfg.fin_timeout),
            _ => {}
        }
    }

    /// resets the connection (RFC 9293 3.10.5, ABORT)
    fn abort(&mut self, why: RstWhy, cx: &mut Cx) {
        if matches!(self.state, TcpState::SynReceived | TcpState::Established | TcpState::FinWait1 | TcpState::FinWait2 | TcpState::CloseWait) {
            self.send_reset(why, cx);
        }
        self.end(TcpEnd::Aborted, cx);
    }

    /// an ICMP error about a segment of this connection (RFC 5927: only about data in flight)
    fn icmp(&mut self, typ: u8, code: u8, mtu: u16, seq: u32, cx: &mut Cx) {
        if !(seq_le(self.snd_una, seq) && seq_lt(seq, self.snd_max)) {
            return;
        }
        if typ == T_UNREACH && code == UNREACH_FRAG_NEEDED {
            // path MTU discovery (RFC 1191): smaller segments, and the lost ones go again
            let m = mtu.saturating_sub(40).max(MSS_MIN);
            if mtu >= 68 && m < self.mss {
                self.mss = m;
                self.note(Note::TcpPmtu { mss: m }, cx);
                self.mark_retx(self.snd_max);
                self.snd_nxt = self.snd_una;
                self.output(cx);
            }
            return;
        }
        if typ != T_UNREACH && typ != T_TIME_EXCEEDED {
            return;
        }
        let why = if typ == T_UNREACH && matches!(code, UNREACH_PORT | UNREACH_PROTO) { TcpEnd::Refused } else { TcpEnd::Unreachable };
        if self.state == TcpState::SynSent {
            // the attempt ends at once (Linux)
            self.note(Note::TcpGiveUp { why }, cx);
            self.end(why, cx);
        } else {
            self.soft_err = Some(why);
        }
    }

    /// an ICMPv6 error about a segment of this connection (RFC 4443, RFC 5927: only about data
    /// in flight): Packet Too Big lowers the MSS to the MTU - 60 (never below 1280 - 60, RFC
    /// 8201) and the lost segments go again; the other errors end an attempt or are kept
    fn icmp6(&mut self, typ: u8, code: u8, mtu: u32, seq: u32, cx: &mut Cx) {
        if !(seq_le(self.snd_una, seq) && seq_lt(seq, self.snd_max)) {
            return;
        }
        if typ == icmpv6::T_PACKET_TOO_BIG {
            let mtu = (mtu as usize).clamp(netpkt::ipv6::MIN_MTU, 65535) as u16;
            let m = mtu.saturating_sub(60).max(MSS_MIN);
            if m < self.mss {
                self.mss = m;
                self.note(Note::TcpPtb { mss: m }, cx);
                self.mark_retx(self.snd_max);
                self.snd_nxt = self.snd_una;
                self.output(cx);
            }
            return;
        }
        if !matches!(typ, icmpv6::T_DEST_UNREACH | icmpv6::T_TIME_EXCEEDED | icmpv6::T_PARAM_PROBLEM) {
            return;
        }
        let why = if typ == icmpv6::T_DEST_UNREACH && code == icmpv6::UNREACH_PORT { TcpEnd::Refused } else { TcpEnd::Unreachable };
        if self.state == TcpState::SynSent {
            // the attempt ends at once (Linux: tcp_v6_err)
            self.note(Note::TcpGiveUp { why }, cx);
            self.end(why, cx);
        } else {
            self.soft_err = Some(why);
        }
    }

    fn row(&self, now: Ns, owner: Option<String>) -> TcpRow {
        TcpRow {
            id: self.id,
            state: self.state,
            local: self.local,
            remote: self.remote,
            owner,
            recv_q: self.rbuf.len() as u32,
            send_q: self.sbuf.len() as u32,
            mss: self.mss,
            snd_wnd: self.snd_wnd,
            rcv_wnd: self.offered(),
            cwnd: self.cwnd,
            ssthresh: self.ssthresh,
            srtt: self.srtt,
            rttvar: self.rttvar,
            rto: self.rto,
            iss: self.iss,
            irs: self.irs,
            snd_una: self.snd_una,
            snd_nxt: self.snd_max,
            rcv_nxt: self.rcv_nxt,
            timer: self.shown_timer(now),
            retries: self.retries,
            counters: self.c,
        }
    }
}

// ---------------------------------------------------------------- the engine side

impl Engine {
    pub(crate) fn tcp(&self, ni: usize) -> Option<&TcpStack> {
        self.os(ni).map(|o| &o.tcp)
    }
    pub(crate) fn tcp_mut(&mut self, ni: usize) -> Option<&mut TcpStack> {
        self.os_mut(ni).map(|o| &mut o.tcp)
    }
    fn rand32(&mut self, ni: usize) -> u32 {
        match self.nodes.get_mut(ni) {
            Some(Some(n)) => n.rng.next_u32(),
            _ => 0,
        }
    }

    /// processes a connection: takes it out, lets it work, puts it back, does what it asked
    fn with_tcb<R>(&mut self, ni: usize, id: u32, cause: Option<HopId>, f: impl FnOnce(&mut Tcb, &mut Cx) -> R) -> Option<R> {
        let now = self.now;
        let tcp = self.tcp_mut(ni)?;
        let cfg = tcp.cfg;
        let mut tcb = tcp.conns.remove(&id)?;
        let mut cx = Cx { now, cfg: &cfg, cause, out: Vec::new(), stats: TcpStats::default() };
        let r = f(&mut tcb, &mut cx);
        let Cx { out, stats, .. } = cx;
        let ends = (tcb.local, tcb.remote);
        let listener = tcb.listener;
        if let Some(tcp) = self.tcp_mut(ni) {
            tcp.stats.add(&stats);
            tcp.conns.insert(id, tcb);
        }
        self.tcp_apply(ni, id, ends, listener, out, cause);
        self.tcp_arm(ni);
        Some(r)
    }

    fn tcp_apply(&mut self, ni: usize, id: u32, ends: ((IpAddr, u16), (IpAddr, u16)), listener: Option<u32>, out: Vec<Out>, cause: Option<HopId>) {
        let dev = self.dev_of(ni);
        let v6 = !ends.1.0.is_v4();
        for o in out {
            match o {
                Out::Note(n) if v6 => self.note_v6(dev, cause, n),
                Out::Note(n) => self.note(dev, cause, n),
                Out::Seg(s) => self.tcp_xmit(ni, id, ends.0, ends.1, &s, cause),
                Out::Ev(svc, ev) => self.os_push(ni, Job::Ev(svc, ev)),
                Out::Accept => self.tcp_queue(ni, id, listener, cause),
                Out::Gone => self.tcp_forget(ni, id),
                Out::Unindex => self.tcp_unindex(ni, id),
            }
        }
    }

    /// a passive connection joins the accept queue of its listening socket
    fn tcp_queue(&mut self, ni: usize, id: u32, lid: Option<u32>, cause: Option<HopId>) {
        let Some(l) = lid.and_then(|l| self.tcp_mut(ni)?.listeners.get_mut(&l)) else {
            return;
        };
        l.half = l.half.saturating_sub(1);
        l.queue.push_back(id);
        if l.ev {
            return;
        }
        l.ev = true;
        let (owner, listener) = (l.owner, l.id);
        self.os_push(ni, Job::Ev(owner, SvcEvent::TcpAccept { listener, hop: cause.unwrap_or(0) }));
    }

    /// a connection is gone
    fn tcp_forget(&mut self, ni: usize, id: u32) {
        let Some(tcp) = self.tcp_mut(ni) else {
            return;
        };
        let Some(c) = tcp.conns.remove(&id) else {
            return;
        };
        let key = (c.local.0, c.local.1, c.remote.0, c.remote.1);
        if tcp.tuples.get(&key) == Some(&id) {
            tcp.tuples.remove(&key);
        }
        if let Some(l) = c.listener.and_then(|l| tcp.listeners.get_mut(&l)) {
            match l.queue.iter().position(|&x| x == id) {
                Some(p) => {
                    l.queue.remove(p);
                }
                None => l.half = l.half.saturating_sub(1),
            }
        }
    }

    /// a closed connection its service still holds: the pair of addresses is free again
    fn tcp_unindex(&mut self, ni: usize, id: u32) {
        let Some(tcp) = self.tcp_mut(ni) else {
            return;
        };
        if let Some(c) = tcp.conns.get(&id) {
            let key = (c.local.0, c.local.1, c.remote.0, c.remote.1);
            if tcp.tuples.get(&key) == Some(&id) {
                tcp.tuples.remove(&key);
            }
        }
    }

    /// the TCP timer at the earliest deadline (it may fire early and find nothing)
    fn tcp_arm(&mut self, ni: usize) {
        let now = self.now;
        let Some(tcp) = self.tcp_mut(ni) else {
            return;
        };
        let next = if tcp.loopq.is_empty() { tcp.conns.values().filter_map(|c| c.next_timer()).min() } else { Some(now) };
        let Some(t) = next else {
            return;
        };
        if tcp.armed.is_some_and(|a| a <= t) {
            return;
        }
        tcp.armed = Some(t);
        self.set_timer(ni, SLOT_TCP, t);
    }

    /// node timer SLOT_TCP
    pub(crate) fn tcp_timer(&mut self, ni: usize) {
        let now = self.now;
        let Some(tcp) = self.tcp_mut(ni) else {
            return;
        };
        tcp.armed = None;
        let q: Vec<Vec<u8>> = tcp.loopq.drain(..).collect();
        for p in q {
            if p.first().is_some_and(|b| b >> 4 == 6) {
                if let Some(v) = Ipv6View::parse(&p)
                    && let Some(t) = TcpView::parse(v.payload())
                {
                    self.tcp_segment(ni, IpAddr::V6(v.src()), IpAddr::V6(v.dst()), &t, 0);
                }
            } else if let Some(v) = Ipv4View::parse(&p)
                && let Some(t) = TcpView::parse(v.payload())
            {
                self.tcp_segment(ni, IpAddr::V4(v.src()), IpAddr::V4(v.dst()), &t, 0);
            }
        }
        let due: Vec<u32> = match self.tcp(ni) {
            Some(tcp) => tcp.conns.values().filter(|c| c.next_timer().is_some_and(|t| t <= now)).map(|c| c.id).collect(),
            None => return,
        };
        for id in due {
            self.with_tcb(ni, id, None, |c, cx| c.on_timers(cx));
        }
        self.tcp_arm(ni);
    }

    /// puts a segment of a connection on its way: to the routing table of its family, or to the
    /// device itself through the TCP timer
    fn tcp_xmit(&mut self, ni: usize, id: u32, (lip, lport): (IpAddr, u16), (rip, rport): (IpAddr, u16), s: &Seg, cause: Option<HopId>) {
        let (lip, rip) = match (lip, rip) {
            (IpAddr::V4(l), IpAddr::V4(r)) => (l, r),
            (IpAddr::V6(l), IpAddr::V6(r)) => return self.tcp6_xmit(ni, id, (l, lport), (r, rport), s, cause),
            _ => return,
        };
        let df = self.tcp(ni).is_some_and(|t| t.cfg.df);
        let ios = self.os(ni).is_some_and(|o| o.ios);
        let own_ttl = self.tcp(ni).and_then(|t| t.conns.get(&id)).map(|c| c.opts.ttl).unwrap_or(0);
        let Some(st) = self.stack_mut(ni) else {
            return;
        };
        let ttl = if own_ttl != 0 { own_ttl } else { st.ttl };
        let id = if df { s.ip_id } else { st.next_id() };
        let local = st.is_mine(rip) || (rip.is_loopback() && !ios);
        let mut p = Pkt::new(HEADROOM, &s.data);
        let opts = TcpOpts { mss: s.mss, ..TcpOpts::default() };
        TcpHdr { src_port: lport, dst_port: rport, seq: s.seq, ack: s.ack, flags: s.flags, window: s.wnd, urgent: 0, opts }.push_v4(&mut p, lip, rip);
        let mut h = Ipv4Hdr::new(lip, rip, PROTO_TCP, ttl);
        h.df = df;
        h.id = id;
        h.push_onto(&mut p);
        let pkt = p.into_vec();
        if local {
            if let Some(tcp) = self.tcp_mut(ni) {
                tcp.loopq.push_back(pkt);
            }
            self.tcp_arm(ni);
            return;
        }
        self.ip_output(ni, pkt, Origin::Local { job: None }, cause);
    }

    /// a segment over IPv6: the hop limit of the way out, a link-local peer on the interface of
    /// the local address; one larger than the path MTU is not sent (the sender never
    /// fragments TCP): its connection lowers the MSS as for Packet Too Big
    fn tcp6_xmit(&mut self, ni: usize, id: u32, (lip, lport): (Ipv6, u16), (rip, rport): (Ipv6, u16), s: &Seg, cause: Option<HopId>) {
        let local = self.v6(ni).is_some_and(|v| v.find(rip).is_some());
        let oif = if rip.is_link_local() { self.v6(ni).and_then(|v| v.find(lip)) } else { None };
        let out = oif.or_else(|| self.ip6_route(ni, rip, None).map(|nh| nh.iface));
        let hops = self.ip6_hops(ni, out);
        let mut p = Pkt::new(HEADROOM, &s.data);
        let opts = TcpOpts { mss: s.mss, ..TcpOpts::default() };
        TcpHdr { src_port: lport, dst_port: rport, seq: s.seq, ack: s.ack, flags: s.flags, window: s.wnd, urgent: 0, opts }.push_v6(&mut p, lip, rip);
        Ipv6Hdr::new(lip, rip, NH_TCP, hops).push_onto(&mut p);
        let pkt = p.into_vec();
        if local {
            if let Some(tcp) = self.tcp_mut(ni) {
                tcp.loopq.push_back(pkt);
            }
            self.tcp_arm(ni);
            return;
        }
        let path = self.v6(ni).and_then(|v| v.pmtu.get(&rip)).map(|x| x.0 as usize);
        let mtu = out.map(|i| self.ip6_mtu(ni, i)).unwrap_or(netpkt::ipv6::MIN_MTU).min(path.unwrap_or(usize::MAX));
        if pkt.len() > mtu {
            let seq = s.seq;
            self.with_tcb(ni, id, cause, |c, cx| c.icmp6(icmpv6::T_PACKET_TOO_BIG, 0, mtu as u32, seq, cx));
            return;
        }
        self.ip6_output(ni, pkt, oif, Origin::Local { job: None }, cause);
    }

    /// the RST for a segment that belongs to nothing (RFC 9293 3.10.7.1); Linux sends it
    /// with Don't Fragment and IP identification 0
    fn tcp_reset_reply(&mut self, ni: usize, src: IpAddr, dst: IpAddr, t: &TcpView, cause: Option<HopId>) {
        let (src, dst) = match (src, dst) {
            (IpAddr::V4(s), IpAddr::V4(d)) => (s, d),
            (IpAddr::V6(s), IpAddr::V6(d)) => return self.tcp6_reset_reply(ni, s, d, t, cause),
            _ => return,
        };
        let (seq, ack, flags) = if t.has(ACK) { (t.ack(), 0, RST) } else { (0, t.seq().wrapping_add(t.seg_len()), RST | ACK) };
        let df = self.tcp(ni).is_some_and(|x| x.cfg.df);
        let ios = self.os(ni).is_some_and(|o| o.ios);
        let Some(st) = self.stack_mut(ni) else {
            return;
        };
        let ttl = st.ttl;
        let id = if df { 0 } else { st.next_id() };
        let local = st.is_mine(src) || (src.is_loopback() && !ios);
        let mut p = Pkt::new(HEADROOM, &[]);
        TcpHdr { src_port: t.dst_port(), dst_port: t.src_port(), seq, ack, flags, window: 0, urgent: 0, opts: TcpOpts::default() }.push_v4(&mut p, dst, src);
        let mut h = Ipv4Hdr::new(dst, src, PROTO_TCP, ttl);
        h.df = df;
        h.id = id;
        h.push_onto(&mut p);
        let pkt = p.into_vec();
        if let Some(tcp) = self.tcp_mut(ni) {
            tcp.stats.out_rsts += 1;
            tcp.stats.out_segs += 1;
        }
        if local {
            if let Some(tcp) = self.tcp_mut(ni) {
                tcp.loopq.push_back(pkt);
            }
            self.tcp_arm(ni);
            return;
        }
        self.ip_output(ni, pkt, Origin::Local { job: None }, cause);
    }

    /// the RST for a segment over IPv6 that belongs to nothing: back with the device's hop
    /// limit, to a link-local sender on the interface it came to
    fn tcp6_reset_reply(&mut self, ni: usize, src: Ipv6, dst: Ipv6, t: &TcpView, cause: Option<HopId>) {
        let (seq, ack, flags) = if t.has(ACK) { (t.ack(), 0, RST) } else { (0, t.seq().wrapping_add(t.seg_len()), RST | ACK) };
        let local = self.v6(ni).is_some_and(|v| v.find(src).is_some());
        let mut p = Pkt::new(HEADROOM, &[]);
        TcpHdr { src_port: t.dst_port(), dst_port: t.src_port(), seq, ack, flags, window: 0, urgent: 0, opts: TcpOpts::default() }.push_v6(&mut p, dst, src);
        let hops = self.ip6_hops(ni, None);
        Ipv6Hdr::new(dst, src, NH_TCP, hops).push_onto(&mut p);
        let pkt = p.into_vec();
        if let Some(tcp) = self.tcp_mut(ni) {
            tcp.stats.out_rsts += 1;
            tcp.stats.out_segs += 1;
        }
        if local {
            if let Some(tcp) = self.tcp_mut(ni) {
                tcp.loopq.push_back(pkt);
            }
            self.tcp_arm(ni);
            return;
        }
        let oif = if src.is_link_local() { self.v6(ni).and_then(|v| v.find(dst)) } else { None };
        self.ip6_output(ni, pkt, oif, Origin::Local { job: None }, cause);
    }

    /// a TCP segment for this device (from ip_local)
    pub(crate) fn tcp_input(&mut self, ni: usize, v: &Ipv4View, bcast: bool, hop: HopId) {
        let dev = self.dev_of(ni);
        let Some(t) = TcpView::parse(v.payload()) else {
            self.note(dev, Some(hop), Note::IpMalformed);
            return;
        };
        if !t.checksum_ok_v4(v.src(), v.dst()) {
            if let Some(tcp) = self.tcp_mut(ni) {
                tcp.stats.in_errs += 1;
            }
            self.note(dev, Some(hop), Note::IpBadChecksum);
            return;
        }
        // never to a broadcast or multicast address (RFC 1122 4.2.3.10)
        if bcast || v.dst().is_multicast() {
            return;
        }
        self.tcp_segment(ni, IpAddr::V4(v.src()), IpAddr::V4(v.dst()), &t, hop);
    }

    /// a TCP segment over IPv6 for this device (from l3v6.rs; `body`: what follows the
    /// extension headers): the checksum over the IPv6 pseudo header (RFC 8200 8.1), never to a
    /// group
    pub(crate) fn tcp6_input(&mut self, ni: usize, v: &Ipv6View, body: &[u8], hop: HopId) {
        let Some(t) = TcpView::parse(body) else {
            self.note6(ni, Some(hop), crate::trace6::Note6::Malformed);
            return;
        };
        if !t.checksum_ok_v6(v.src(), v.dst()) {
            if let Some(tcp) = self.tcp_mut(ni) {
                tcp.stats.in_errs += 1;
            }
            self.note6(ni, Some(hop), crate::trace6::Note6::BadChecksum);
            return;
        }
        if v.dst().is_multicast() {
            return;
        }
        self.tcp_segment(ni, IpAddr::V6(v.src()), IpAddr::V6(v.dst()), &t, hop);
    }

    /// a segment to its connection, to a listening socket, or answered with a RST
    fn tcp_segment(&mut self, ni: usize, src: IpAddr, dst: IpAddr, t: &TcpView, hop: HopId) {
        let dev = self.dev_of(ni);
        let cause = if hop == 0 { None } else { Some(hop) };
        let Some(tcp) = self.tcp_mut(ni) else {
            return;
        };
        tcp.stats.in_segs += 1;
        let conn = tcp.tuples.get(&(dst, t.dst_port(), src, t.src_port())).copied();
        let lid = tcp.listener_for(dst, t.dst_port());
        if let Some(id) = conn {
            let Some(c) = tcp.conns.get(&id) else {
                return;
            };
            let (state, rcv_nxt, snd_max, parent) = (c.state, c.rcv_nxt, c.snd_max, c.listener);
            // TIME-WAIT: a new SYN beyond what came before opens a new connection at once, its
            // ISN above everything the old one used (RFC 1122 4.2.2.13, Linux)
            if state == TcpState::TimeWait
                && t.flags() & (SYN | ACK | RST) == SYN
                && seq_gt(t.seq(), rcv_nxt)
                && let Some(lid) = lid
            {
                self.with_tcb(ni, id, cause, |c, cx| {
                    c.set_state(TcpState::Closed, cx);
                    c.clear_timers();
                    c.bury(cx);
                });
                self.tcp_syn(ni, lid, src, dst, t, cause, Some(snd_max.wrapping_add(65535 + 2)));
                return;
            }
            // the handshake's last ACK while the accept queue is full: ignored, the SYN-ACK
            // goes again later (Linux with tcp_abort_on_overflow 0)
            if state == TcpState::SynReceived
                && t.flags() & (SYN | RST) == 0
                && t.has(ACK)
                && let Some(l) = parent.and_then(|p| tcp.listeners.get_mut(&p))
                && l.queue.len() as u32 > l.backlog
            {
                l.dropped += 1;
                let port = l.port;
                tcp.stats.listen_drops += 1;
                self.note(dev, cause, Note::TcpAckDropped { port });
                return;
            }
            let opts = t.opts();
            let s = SegIn { seq: t.seq(), ack: t.ack(), flags: t.flags(), wnd: t.window() as u32, mss: opts.mss, data: t.payload() };
            self.with_tcb(ni, id, cause, |c, cx| c.input(&s, cx));
            return;
        }
        if let Some(lid) = lid {
            // LISTEN (RFC 9293 3.10.7.2)
            if t.has(RST) {
                return;
            }
            if t.has(ACK) {
                self.note(dev, cause, Note::TcpRstSent { why: RstWhy::BadAck });
                self.tcp_reset_reply(ni, src, dst, t, cause);
                return;
            }
            if t.has(SYN) {
                self.tcp_syn(ni, lid, src, dst, t, cause, None);
            }
            return;
        }
        // CLOSED: nobody (RFC 9293 3.10.7.1)
        if t.has(RST) {
            return;
        }
        let port = t.dst_port();
        self.note(dev, cause, if t.flags() & (SYN | ACK) == SYN { Note::TcpNoListener { port } } else { Note::TcpNoConn { port } });
        self.tcp_reset_reply(ni, src, dst, t, cause);
    }

    /// a SYN for a listening socket: a new connection in SYN-RECEIVED, unless more
    /// connections wait for tcp_accept than the backlog allows (Linux then drops the SYN)
    #[allow(clippy::too_many_arguments)]
    fn tcp_syn(&mut self, ni: usize, lid: u32, src: IpAddr, dst: IpAddr, t: &TcpView, cause: Option<HopId>, iss: Option<u32>) {
        let now = self.now;
        let dev = self.dev_of(ni);
        let Some(tcp) = self.tcp_mut(ni) else {
            return;
        };
        let Some(l) = tcp.listeners.get_mut(&lid) else {
            return;
        };
        if l.queue.len() as u32 > l.backlog {
            l.dropped += 1;
            let port = l.port;
            tcp.stats.listen_drops += 1;
            self.note(dev, cause, Note::TcpSynDropped { port });
            return;
        }
        l.half += 1;
        let opts = l.opts;
        let id = tcp.next_id;
        tcp.next_id += 1;
        let iss = match iss {
            Some(i) => i,
            None => self.rand32(ni),
        };
        let ip_id = self.rand32(ni) as u16;
        let adv_mss = self.tcp_adv_mss(ni, src);
        let Some(tcp) = self.tcp_mut(ni) else {
            return;
        };
        let cfg = tcp.cfg;
        let mut c = Tcb::new(id, (dst, t.dst_port()), (src, t.src_port()), iss, ip_id, adv_mss, opts, &cfg, now);
        c.listener = Some(lid);
        tcp.tuples.insert((dst, t.dst_port(), src, t.src_port()), id);
        tcp.conns.insert(id, c);
        let o = t.opts();
        let s = SegIn { seq: t.seq(), ack: t.ack(), flags: t.flags(), wnd: t.window() as u32, mss: o.mss, data: &[] };
        self.with_tcb(ni, id, cause, |c, cx| c.accept_syn(&s, cx));
    }

    /// the MSS this side announces towards `dst`: the MTU of the route's interface - 40
    /// (IOS-like: 536 beyond the connected subnets); over IPv6 the MTU (or the path MTU) - 60
    /// (IOS-like: 1220 beyond the connected prefixes)
    fn tcp_adv_mss(&self, ni: usize, dst: IpAddr) -> u16 {
        let dst = match dst {
            IpAddr::V4(d) => d,
            IpAddr::V6(d) => return self.tcp_adv_mss6(ni, d),
        };
        let Some(st) = self.stack(ni) else {
            return MSS_DEFAULT;
        };
        let remote = self.tcp(ni).and_then(|t| t.cfg.mss_remote);
        let mine = st.ifaces.iter().find(|f| !f.hidden && (f.has(dst) || (f.is_loopback() && dst.is_loopback() && f.on_link(dst))));
        if let Some(f) = mine {
            // to itself: Linux routes every own address over lo
            let lo = st.ifaces.iter().find(|x| x.is_loopback() && x.name == "lo").unwrap_or(f);
            return lo.mtu.saturating_sub(40).max(MSS_MIN);
        }
        let Some(nh) = self.route(ni, dst) else {
            return MSS_DEFAULT;
        };
        let mtu = st.ifaces.get(nh.iface as usize).map(|f| f.mtu).unwrap_or(1500);
        let mss = mtu.saturating_sub(40).max(MSS_MIN);
        match remote {
            Some(r) if nh.gw != dst => r.min(mss),
            _ => mss,
        }
    }

    /// the MSS towards an IPv6 destination: the IPv6 MTU of the way out, or the path MTU
    /// learned for it, - 60 (the loopback's to the device itself)
    fn tcp_adv_mss6(&self, ni: usize, dst: Ipv6) -> u16 {
        let remote = self.tcp(ni).and_then(|t| t.cfg.mss_remote);
        if self.v6(ni).is_some_and(|v| v.find(dst).is_some()) {
            let lo = self.stack(ni).and_then(|st| st.ifaces.iter().find(|x| x.is_loopback() && x.name == "lo")).map(|f| f.mtu as usize);
            return (lo.unwrap_or(65536).min(65535) as u16).saturating_sub(60).max(MSS_MIN);
        }
        let Some(nh) = self.ip6_route(ni, dst, None) else {
            return MSS_DEFAULT6;
        };
        let path = self.v6(ni).and_then(|v| v.pmtu.get(&dst)).map(|p| p.0 as usize);
        let mtu = self.ip6_mtu(ni, nh.iface).min(path.unwrap_or(usize::MAX)).min(65535) as u16;
        let mss = mtu.saturating_sub(60).max(MSS_MIN);
        match remote {
            Some(_) if nh.gw != dst => MSS_DEFAULT6.min(mss),
            _ => mss,
        }
    }

    /// a free port for a connection to `dst`:`dport` (Linux: even ones first)
    fn tcp_port(&mut self, ni: usize, src: IpAddr, dst: IpAddr, dport: u16) -> Option<u16> {
        let cfg = self.tcp(ni)?.cfg;
        let (lo, hi) = cfg.ports;
        let n = (hi as u32).saturating_sub(lo as u32) + 1;
        let r = self.rand32(ni) % n;
        let tcp = self.tcp(ni)?;
        let free = |p: u16| !tcp.tuples.contains_key(&(src, p, dst, dport)) && !tcp.listeners.values().any(|l| l.port == p);
        for k in 0..n {
            let off = if cfg.even_ports {
                let evens = n.div_ceil(2);
                if k < evens { ((r / 2 + k) % evens) * 2 } else { ((r / 2 + k - evens) % (n / 2).max(1)) * 2 + 1 }
            } else {
                (r + k) % n
            };
            let p = (lo as u32 + off) as u16;
            if free(p) {
                return Some(p);
            }
        }
        None
    }

    // ------------------------------------------------------------ the socket calls

    /// a listening socket (0.0.0.0: every IPv4 address, ::: every address of both families)
    pub(crate) fn tcp_listen(&mut self, ni: usize, owner: u32, addr: IpAddr, port: u16, backlog: u32, opts: SockOpts) -> Result<u32, Msg> {
        let any_of = if addr.is_v4() { IpAddr::V4(Ipv4::UNSPECIFIED) } else { IpAddr::V6(Ipv6::UNSPECIFIED) };
        let port = if port == 0 { self.tcp_port(ni, addr, any_of, 0).ok_or_else(|| msg!("Keine freien Ports."))? } else { port };
        let dev = self.dev_of(ni);
        let tcp = self.tcp_mut(ni).ok_or_else(|| msg!("kein IP"))?;
        if tcp.listeners.values().any(|l| l.port == port && crate::os::overlap(l.addr, addr)) {
            return Err(msg!("Port {} ist schon belegt.", port));
        }
        let id = tcp.next_id;
        tcp.next_id += 1;
        let backlog = if backlog == 0 { tcp.cfg.backlog } else { backlog };
        tcp.listeners.insert(id, Listener { id, owner, addr, port, backlog, queue: VecDeque::new(), half: 0, opts, accepted: 0, dropped: 0, ev: false });
        self.note(dev, None, Note::TcpStateChange { port, peer: any_of, pport: 0, from: TcpState::Closed, to: TcpState::Listen });
        Ok(id)
    }

    /// an active open from `sport` (0: an ephemeral port) to an IPv4 or IPv6 address
    pub(crate) fn tcp_connect(&mut self, ni: usize, owner: u32, sport: u16, dst: IpAddr, dport: u16, opts: SockOpts, cause: Option<HopId>) -> Result<u32, Msg> {
        self.tcp_connect_src(ni, owner, None, sport, dst, dport, opts, cause)
    }

    /// the same from a source address of the device (None: the one of the way out); the
    /// destination still needs a route
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tcp_connect_src(&mut self, ni: usize, owner: u32, from: Option<IpAddr>, sport: u16, dst: IpAddr, dport: u16, opts: SockOpts, cause: Option<HopId>) -> Result<u32, Msg> {
        let bad = match dst {
            IpAddr::V4(d) => d.is_unspecified() || d.is_broadcast() || d.is_multicast(),
            IpAddr::V6(d) => d.is_unspecified() || d.is_multicast() || d.to_ipv4_mapped().is_some(),
        };
        if dport == 0 || bad {
            return Err(match dst {
                IpAddr::V4(d) => msg!("{}:{} ist kein Ziel für TCP.", d, dport),
                IpAddr::V6(d) => msg!("[{}]:{} ist kein Ziel für TCP.", d, dport),
            });
        }
        let src = self.source_ip(ni, dst).ok_or_else(|| msg!("Das Netz ist nicht erreichbar."))?;
        let src = from.unwrap_or(src);
        let sport = if sport == 0 {
            self.tcp_port(ni, src, dst, dport).ok_or_else(|| msg!("Keine freien Ports."))?
        } else {
            let tcp = self.tcp(ni).ok_or_else(|| msg!("kein IP"))?;
            if tcp.tuples.contains_key(&(src, sport, dst, dport)) || tcp.listeners.values().any(|l| l.port == sport) {
                return Err(msg!("Port {} ist schon belegt.", sport));
            }
            sport
        };
        let iss = self.rand32(ni);
        let ip_id = self.rand32(ni) as u16;
        let adv_mss = self.tcp_adv_mss(ni, dst);
        let now = self.now;
        let tcp = self.tcp_mut(ni).ok_or_else(|| msg!("kein IP"))?;
        let cfg = tcp.cfg;
        let id = tcp.next_id;
        tcp.next_id += 1;
        let mut c = Tcb::new(id, (src, sport), (dst, dport), iss, ip_id, adv_mss, opts, &cfg, now);
        c.owner = Some(owner);
        if let Some(w) = cfg.syn_wait {
            c.t_wait = Some(now + w);
        }
        tcp.tuples.insert((src, sport, dst, dport), id);
        tcp.conns.insert(id, c);
        tcp.stats.active_opens += 1;
        self.with_tcb(ni, id, cause, |c, cx| c.open(cx));
        Ok(id)
    }

    /// the next connection of a listening socket's queue
    pub(crate) fn tcp_accept(&mut self, ni: usize, owner: u32, lid: u32) -> Option<u32> {
        let tcp = self.tcp_mut(ni)?;
        let l = tcp.listeners.get_mut(&lid).filter(|l| l.owner == owner)?;
        l.ev = false;
        let id = l.queue.pop_front()?;
        l.accepted += 1;
        let c = tcp.conns.get_mut(&id)?;
        c.owner = Some(owner);
        c.listener = None;
        // what came before the service took the connection
        let mut evs = Vec::new();
        if !c.rbuf.is_empty() {
            c.ev_data = true;
            evs.push(SvcEvent::TcpData { sock: id, hop: 0 });
        }
        if c.fin_rcvd {
            evs.push(SvcEvent::TcpEof { sock: id, hop: 0 });
        }
        for e in evs {
            self.os_push(ni, Job::Ev(owner, e));
        }
        Some(id)
    }

    fn tcp_owned(&self, ni: usize, owner: u32, id: u32) -> bool {
        self.tcp(ni).and_then(|t| t.conns.get(&id)).is_some_and(|c| c.owner == Some(owner))
    }

    pub(crate) fn tcp_write(&mut self, ni: usize, owner: u32, id: u32, data: &[u8], cause: Option<HopId>) -> usize {
        if !self.tcp_owned(ni, owner, id) {
            return 0;
        }
        self.with_tcb(ni, id, cause, |c, cx| c.write(data, cx)).unwrap_or(0)
    }

    pub(crate) fn tcp_read(&mut self, ni: usize, owner: u32, id: u32, max: usize, cause: Option<HopId>) -> Vec<u8> {
        if !self.tcp_owned(ni, owner, id) {
            return Vec::new();
        }
        self.with_tcb(ni, id, cause, |c, cx| c.read(max, cx)).unwrap_or_default()
    }

    pub(crate) fn tcp_shutdown(&mut self, ni: usize, owner: u32, id: u32, cause: Option<HopId>) {
        if self.tcp_owned(ni, owner, id) {
            self.with_tcb(ni, id, cause, |c, cx| c.shutdown(cx));
        }
    }

    /// closes a connection or a listening socket of the service
    pub(crate) fn tcp_close(&mut self, ni: usize, owner: u32, id: u32, cause: Option<HopId>) {
        if self.tcp(ni).and_then(|t| t.listeners.get(&id)).is_some_and(|l| l.owner == owner) {
            self.tcp_close_listener(ni, id, cause);
        } else if self.tcp_owned(ni, owner, id) {
            self.with_tcb(ni, id, cause, |c, cx| c.close(cx));
        }
    }

    pub(crate) fn tcp_abort(&mut self, ni: usize, owner: u32, id: u32, cause: Option<HopId>) {
        if self.tcp_owned(ni, owner, id) {
            self.with_tcb(ni, id, cause, |c, cx| {
                c.orphan = true;
                c.abort(RstWhy::Abort, cx);
            });
        }
    }

    /// changes the settings of a connection (TCP_NODELAY, SO_KEEPALIVE, SO_SNDBUF; the receive
    /// buffer stays as the handshake offered it)
    pub(crate) fn tcp_setopts(&mut self, ni: usize, owner: u32, id: u32, opts: SockOpts, cause: Option<HopId>) {
        if !self.tcp_owned(ni, owner, id) {
            return;
        }
        self.with_tcb(ni, id, cause, |c, cx| {
            if opts.keepalive && !c.opts.keepalive && c.t_keep.is_none() {
                c.t_keep = Some(cx.now + cx.cfg.keep_idle);
            }
            if !opts.keepalive {
                c.t_keep = None;
            }
            if opts.sndbuf > 0 {
                c.sndbuf = opts.sndbuf;
                c.snd_auto = false;
            }
            c.opts = SockOpts { rcvbuf: c.opts.rcvbuf, ..opts };
            c.output(cx);
        });
    }

    /// a listening socket closes: connections waiting in its queue are reset, half-open ones
    /// go quietly
    fn tcp_close_listener(&mut self, ni: usize, lid: u32, cause: Option<HopId>) {
        let dev = self.dev_of(ni);
        let Some(tcp) = self.tcp_mut(ni) else {
            return;
        };
        let Some(l) = tcp.listeners.remove(&lid) else {
            return;
        };
        let kids: Vec<u32> = tcp.conns.values().filter(|c| c.listener == Some(lid)).map(|c| c.id).collect();
        let none = if l.addr.is_v4() { IpAddr::V4(Ipv4::UNSPECIFIED) } else { IpAddr::V6(Ipv6::UNSPECIFIED) };
        self.note(dev, cause, Note::TcpStateChange { port: l.port, peer: none, pport: 0, from: TcpState::Listen, to: TcpState::Closed });
        for id in kids {
            let queued = l.queue.contains(&id);
            self.with_tcb(ni, id, cause, |c, cx| {
                c.listener = None;
                if queued {
                    c.abort(RstWhy::ListenerClosed, cx);
                } else {
                    c.drop_half(cx);
                }
            });
        }
    }

    /// a service stopped: its sockets close as at the end of a process
    pub(crate) fn tcp_owner_gone(&mut self, ni: usize, owner: u32) {
        let powered = matches!(self.nodes.get(ni), Some(Some(n)) if n.power);
        let Some(tcp) = self.tcp(ni) else {
            return;
        };
        if !powered {
            return;
        }
        let lids: Vec<u32> = tcp.listeners.values().filter(|l| l.owner == owner).map(|l| l.id).collect();
        let cids: Vec<u32> = tcp.conns.values().filter(|c| c.owner == Some(owner)).map(|c| c.id).collect();
        for l in lids {
            self.tcp_close_listener(ni, l, None);
        }
        for c in cids {
            self.with_tcb(ni, c, None, |c, cx| c.close(cx));
        }
    }

    /// an ICMP error quoting one of this device's TCP segments (from icmp_input)
    pub(crate) fn tcp_icmp_error(&mut self, ni: usize, typ: u8, code: u8, mtu: u16, q: &Ipv4View) {
        let p = q.payload();
        if q.proto() != PROTO_TCP || p.len() < 8 {
            return;
        }
        let sport = u16::from_be_bytes([p[0], p[1]]);
        let dport = u16::from_be_bytes([p[2], p[3]]);
        let seq = u32::from_be_bytes([p[4], p[5], p[6], p[7]]);
        let Some(id) = self.tcp(ni).and_then(|t| t.tuples.get(&(IpAddr::V4(q.src()), sport, IpAddr::V4(q.dst()), dport))).copied() else {
            return;
        };
        self.with_tcb(ni, id, None, |c, cx| c.icmp(typ, code, mtu, seq, cx));
    }

    /// an ICMPv6 error quoting one of this device's TCP segments over IPv6 (from l3v6.rs and
    /// a neighbor that never answered, nd.rs); `value`: the MTU of Packet Too Big
    pub(crate) fn tcp_icmp6_error(&mut self, ni: usize, typ: u8, code: u8, value: u32, q: &Ipv6View) {
        let Some((nh, off)) = netpkt::ipv6::upper_layer(q) else {
            return;
        };
        let p = q.payload().get(off..).unwrap_or(&[]);
        if nh != NH_TCP || p.len() < 8 {
            return;
        }
        let sport = u16::from_be_bytes([p[0], p[1]]);
        let dport = u16::from_be_bytes([p[2], p[3]]);
        let seq = u32::from_be_bytes([p[4], p[5], p[6], p[7]]);
        let Some(id) = self.tcp(ni).and_then(|t| t.tuples.get(&(IpAddr::V6(q.src()), sport, IpAddr::V6(q.dst()), dport))).copied() else {
            return;
        };
        self.with_tcb(ni, id, None, |c, cx| c.icmp6(typ, code, value, seq, cx));
    }

    // ------------------------------------------------------------ queries

    fn tcp_owner_name(&self, ni: usize, owner: Option<u32>) -> Option<String> {
        let o = owner?;
        self.os(ni)?.svcs.iter().find(|s| s.id == o).map(|s| s.name.clone())
    }

    /// a connection or listening socket by its id
    pub(crate) fn tcp_row(&self, ni: usize, id: u32) -> Option<TcpRow> {
        let tcp = self.tcp(ni)?;
        if let Some(c) = tcp.conns.get(&id) {
            return Some(c.row(self.now, self.tcp_owner_name(ni, c.owner)));
        }
        let l = tcp.listeners.get(&id)?;
        let none = if l.addr.is_v4() { IpAddr::V4(Ipv4::UNSPECIFIED) } else { IpAddr::V6(Ipv6::UNSPECIFIED) };
        Some(TcpRow {
            id: l.id,
            state: TcpState::Listen,
            local: (l.addr, l.port),
            remote: (none, 0),
            owner: self.tcp_owner_name(ni, Some(l.owner)),
            recv_q: l.queue.len() as u32,
            send_q: l.backlog,
            mss: 0,
            snd_wnd: 0,
            rcv_wnd: 0,
            cwnd: 0,
            ssthresh: SSTHRESH_INF,
            srtt: None,
            rttvar: 0,
            rto: tcp.cfg.rto_init,
            iss: 0,
            irs: 0,
            snd_una: 0,
            snd_nxt: 0,
            rcv_nxt: 0,
            timer: None,
            retries: 0,
            counters: TcpCounters::default(),
        })
    }

    /// the TCP connections and listening sockets of a device in the order they were made
    /// ("ss -tan", "show tcp brief all")
    pub fn tcp_conns(&self, dev: u32) -> Vec<TcpRow> {
        let Some(ni) = self.node_index(dev) else {
            return Vec::new();
        };
        let Some(tcp) = self.tcp(ni) else {
            return Vec::new();
        };
        let mut ids: Vec<u32> = tcp.conns.keys().chain(tcp.listeners.keys()).copied().collect();
        ids.sort_unstable();
        ids.into_iter().filter_map(|id| self.tcp_row(ni, id)).collect()
    }

    /// one connection or listening socket
    pub fn tcp_conn(&self, dev: u32, id: u32) -> Option<TcpRow> {
        self.tcp_row(self.node_index(dev)?, id)
    }

    /// the TCP counters of a device
    pub fn tcp_stats(&self, dev: u32) -> Option<TcpStats> {
        let tcp = self.tcp(self.node_index(dev)?)?;
        let mut s = tcp.stats;
        s.curr_estab = tcp.conns.values().filter(|c| matches!(c.state, TcpState::Established | TcpState::CloseWait)).count() as u64;
        Some(s)
    }

    /// the TCP values of a device's system
    pub fn tcp_cfg(&self, dev: u32) -> Option<TcpCfg> {
        Some(self.tcp(self.node_index(dev)?)?.cfg)
    }
}
