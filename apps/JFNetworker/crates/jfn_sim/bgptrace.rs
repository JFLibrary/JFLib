// bgptrace -- the decisions of BGP as notes (Note::Bgp) with their German sentences: the router
// ID, the states of a session and why it ends, the TCP connections (opened, taken, refused,
// failed), OPEN and what it negotiated, KEEPALIVE, UPDATE, ROUTE-REFRESH and NOTIFICATION in
// both directions, collisions, the hold timer, routes of an UPDATE that are not taken, the
// decision process (the best path and the step that chose it, a prefix without a path), the
// routes the router originates, announcements that wait for the minimum interval
//
//   BNote::message() -> Msg     the sentence (English in src/locales/engine.tsv)
//   BNote::is_drop() / mask()   something refused or thrown away; the P_* bits (IPv4, TCP, BGP)
//   BNote::is_event() / is_update()   what "debug ip bgp events" and "debug ip bgp updates" show
//                               (cli/bgp.rs)
//
// Error codes and subcodes keep their RFC names (netpkt::bgp::error_name), as protocol terms.

use alloc::format;

use jfn_model::msg;
use jfn_model::msg::{dur, phrase, Msg};
use netpkt::{Ipv4, Prefix4};

use crate::bgp::{DownWhy, DropWhy, How, IdleWhy, RefuseWhy, State, Step};
use crate::kernel::Ns;
use crate::tcp::TcpEnd;

/// a decision of BGP
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BNote {
    /// the process takes its router ID
    RouterId { id: Ipv4 },
    /// a session changed its state
    State { peer: Ipv4, from: State, to: State },
    /// a TCP connection to the neighbour's port 179 is opened (from the update-source address)
    Connect { peer: Ipv4, src: Option<Ipv4> },
    /// no route to the neighbour: no connection
    NoRoute { peer: Ipv4 },
    /// the connection attempt ended (refused, timed out, reset, unreachable)
    ConnFailed { peer: Ipv4, end: TcpEnd },
    /// a connection from a neighbour is taken (at this local address)
    Accepted { peer: Ipv4, local: Ipv4 },
    /// a connection from outside is reset
    Refused { from: Ipv4, why: RefuseWhy },
    /// a neighbour stays Idle
    Idle { peer: Ipv4, why: IdleWhy },
    OpenSent { peer: Ipv4, asn: u32, hold: u16, id: Ipv4 },
    OpenRcvd { peer: Ipv4, asn: u32, hold: u16, id: Ipv4 },
    /// what the two OPENs give: the hold time and the keepalive interval (seconds)
    Negotiated { peer: Ipv4, hold: u16, keepalive: u16 },
    Keepalive { peer: Ipv4, out: bool },
    /// an UPDATE with its prefixes and withdrawn routes
    Update { peer: Ipv4, out: bool, adv: u16, wd: u16 },
    Refresh { peer: Ipv4, out: bool },
    Notification { peer: Ipv4, out: bool, code: u8, sub: u8 },
    /// two connections to one neighbour: the one opened by the higher BGP identifier stays
    Collision { peer: Ipv4, keep_ours: bool },
    /// nothing came from the neighbour within the hold time
    HoldExpired { peer: Ipv4 },
    /// the session ended (its routes are withdrawn)
    Down { peer: Ipv4, why: DownWhy },
    /// a route of an UPDATE is not taken
    Dropped { peer: Ipv4, prefix: Prefix4, why: DropWhy },
    /// the decision process chose a new best path (from a neighbour, None: originated here)
    Best { prefix: Prefix4, via: Ipv4, from: Option<Ipv4>, step: Step },
    /// no usable path to a prefix any more
    Lost { prefix: Prefix4 },
    /// the router originates a prefix (network statement, redistribution), or no longer
    Local { prefix: Prefix4, how: How },
    LocalGone { prefix: Prefix4 },
    /// new paths for a neighbour wait this long (MinRouteAdvertisementInterval)
    Mrai { peer: Ipv4, wait: Ns },
}

impl BNote {
    /// something refused or thrown away
    pub fn is_drop(&self) -> bool {
        matches!(self, BNote::Refused { .. } | BNote::Dropped { .. })
    }
    /// the P_* bits of the decision
    pub fn mask(&self) -> u32 {
        use netpkt::decode::{P_BGP, P_IPV4, P_TCP};
        match self {
            BNote::RouterId { .. } | BNote::Best { .. } | BNote::Lost { .. } | BNote::Local { .. } | BNote::LocalGone { .. } => P_BGP,
            _ => P_IPV4 | P_TCP | P_BGP,
        }
    }
    /// what "debug ip bgp events" shows: sessions, connections, OPEN, NOTIFICATION, timers
    pub fn is_event(&self) -> bool {
        !self.is_update() && !matches!(self, BNote::Keepalive { .. })
    }
    /// what "debug ip bgp updates" shows: UPDATEs, routes taken or not, decisions
    pub fn is_update(&self) -> bool {
        matches!(self, BNote::Update { .. } | BNote::Refresh { .. } | BNote::Dropped { .. } | BNote::Best { .. } | BNote::Lost { .. } | BNote::Local { .. } | BNote::LocalGone { .. } | BNote::Mrai { .. })
    }

    /// the German sentence
    pub fn message(&self) -> Msg {
        let ename = |c: u8, s: u8| Msg::plain(netpkt::bgp::error_name(c, s));
        match self {
            BNote::RouterId { id } => msg!("BGP: der Prozess arbeitet mit der Router-ID {}.", id),
            BNote::State { peer, from, to } => msg!("BGP: Nachbar {}: {} \u{2192} {}.", peer, from.name(), to.name()),
            BNote::Connect { peer, src: Some(s) } => msg!("BGP: öffnet eine TCP-Verbindung zu {}, Port 179, von {}.", peer, s),
            BNote::Connect { peer, src: None } => msg!("BGP: öffnet eine TCP-Verbindung zu {}, Port 179.", peer),
            BNote::NoRoute { peer } => msg!("BGP: keine Route zu {}: keine Verbindung.", peer),
            BNote::ConnFailed { peer, end } => msg!("BGP: die Verbindung zu {} kam nicht zustande: {}.", peer, phrase(end.text())),
            BNote::Accepted { peer, local } => msg!("BGP: Verbindung von {} an {} angenommen.", peer, local),
            BNote::Refused { from, why: RefuseWhy::Unknown } => msg!("BGP: Verbindung von {} zurückgesetzt: kein Nachbar mit dieser Adresse.", from),
            BNote::Refused { from, why: RefuseWhy::Shutdown } => msg!("BGP: Verbindung von {} zurückgesetzt: der Nachbar ist abgeschaltet (neighbor shutdown).", from),
            BNote::Refused { from, why: RefuseWhy::Established } => msg!("BGP: Verbindung von {} zurückgesetzt: die Sitzung besteht schon.", from),
            BNote::Idle { peer, why } => msg!("BGP: Nachbar {} bleibt Idle: {}.", peer, phrase(why.text())),
            BNote::OpenSent { peer, asn, hold, id } => msg!("BGP: OPEN an {} (AS {}, Haltezeit {} s, BGP-ID {}).", peer, asn, hold, id),
            BNote::OpenRcvd { peer, asn, hold, id } => msg!("BGP: OPEN von {} (AS {}, Haltezeit {} s, BGP-ID {}).", peer, asn, hold, id),
            BNote::Negotiated { peer, hold: 0, .. } => msg!("BGP: mit {} vereinbart: keine Haltezeit, keine Keepalives.", peer),
            BNote::Negotiated { peer, hold, keepalive } => msg!("BGP: mit {} vereinbart: Haltezeit {} s, Keepalive alle {} s.", peer, hold, keepalive),
            BNote::Keepalive { peer, out: true } => msg!("BGP: KEEPALIVE an {}.", peer),
            BNote::Keepalive { peer, out: false } => msg!("BGP: KEEPALIVE von {}.", peer),
            BNote::Update { peer, out: true, adv, wd } => msg!("BGP: UPDATE an {}: Netze: {}, zurückgezogen: {}.", peer, adv, wd),
            BNote::Update { peer, out: false, adv, wd } => msg!("BGP: UPDATE von {}: Netze: {}, zurückgezogen: {}.", peer, adv, wd),
            BNote::Refresh { peer, out: true } => msg!("BGP: ROUTE-REFRESH an {}: er schickt seine Netze noch einmal.", peer),
            BNote::Refresh { peer, out: false } => msg!("BGP: ROUTE-REFRESH von {}: er bekommt alle Netze noch einmal.", peer),
            BNote::Notification { peer, out: true, code, sub } => msg!("BGP: NOTIFICATION an {}: {}/{} ({}).", peer, code, sub, ename(*code, *sub)),
            BNote::Notification { peer, out: false, code, sub } => msg!("BGP: NOTIFICATION von {}: {}/{} ({}).", peer, code, sub, ename(*code, *sub)),
            BNote::Collision { peer, keep_ours: true } => msg!("BGP: zwei Verbindungen mit {}: die eigene bleibt (höhere BGP-ID), die andere wird geschlossen.", peer),
            BNote::Collision { peer, keep_ours: false } => msg!("BGP: zwei Verbindungen mit {}: die des Nachbarn bleibt (höhere BGP-ID), die eigene wird geschlossen.", peer),
            BNote::HoldExpired { peer } => msg!("BGP: von {} kam in der Haltezeit nichts: der Hold Timer ist abgelaufen.", peer),
            BNote::Down { peer, why } => msg!("BGP: die Sitzung mit {} ist beendet ({}); seine Wege sind zurückgezogen.", peer, why.msg()),
            BNote::Dropped { peer, prefix, why: DropWhy::Loop(asn) } => msg!("BGP: {} von {} verworfen: das eigene AS {} steht im AS-Pfad (Schleife).", prefix, peer, asn),
            BNote::Dropped { peer, prefix, why: DropWhy::OwnNextHop(nh) } => msg!("BGP: {} von {} verworfen: der Next Hop {} ist eine eigene Adresse.", prefix, peer, nh),
            BNote::Dropped { peer, prefix, why: DropWhy::BadNextHop(nh) } => msg!("BGP: {} von {} verworfen: {} taugt nicht als Next Hop.", prefix, peer, nh),
            BNote::Best { prefix, via, from: Some(p), step } => msg!("BGP: bester Weg zu {} über {} (von {}): {}.", prefix, via, p, phrase(step.text())),
            BNote::Best { prefix, from: None, step, .. } => msg!("BGP: bester Weg zu {}: selbst erzeugt ({}).", prefix, phrase(step.text())),
            BNote::Lost { prefix } => msg!("BGP: kein Weg mehr zu {}.", prefix),
            BNote::Local { prefix, how } => msg!("BGP: erzeugt {} selbst ({}).", prefix, Msg::plain(how.word())),
            BNote::LocalGone { prefix } => msg!("BGP: erzeugt {} nicht mehr selbst.", prefix),
            BNote::Mrai { peer, wait } => msg!("BGP: neue Wege für {} warten noch {} (Mindestabstand zwischen Ankündigungen).", peer, dur(*wait as i64)),
        }
    }
}

impl DownWhy {
    /// why a session ended, as part of a sentence
    pub fn msg(&self) -> Msg {
        let ename = |c: u8, s: u8| Msg::plain(netpkt::bgp::error_name(c, s));
        match *self {
            DownWhy::Sent(netpkt::bgp::E_HOLD, _) => msg!("Hold Timer abgelaufen"),
            DownWhy::Sent(c, s) => msg!("NOTIFICATION {}/{} gesendet: {}", c, s, ename(c, s)),
            DownWhy::Rcvd(c, s) => msg!("NOTIFICATION {}/{} empfangen: {}", c, s, ename(c, s)),
            DownWhy::Tcp(TcpEnd::Closed) => msg!("der Nachbar hat die TCP-Verbindung geschlossen"),
            DownWhy::Tcp(e) => msg!("TCP: {}", phrase(e.text())),
            DownWhy::IfDown => msg!("das Interface zum Nachbarn ist ausgefallen"),
        }
    }
    /// the same in English (console lines)
    pub fn en(&self) -> alloc::string::String {
        match *self {
            DownWhy::Sent(netpkt::bgp::E_HOLD, _) => alloc::string::String::from("hold timer expired"),
            DownWhy::Sent(c, s) => format!("NOTIFICATION {}/{} sent ({})", c, s, netpkt::bgp::error_name(c, s)),
            DownWhy::Rcvd(c, s) => format!("NOTIFICATION {}/{} received ({})", c, s, netpkt::bgp::error_name(c, s)),
            DownWhy::Tcp(TcpEnd::Closed) => alloc::string::String::from("the peer closed the session"),
            DownWhy::Tcp(e) => format!("TCP: {}", e.errno().unwrap_or("closed")),
            DownWhy::IfDown => alloc::string::String::from("the interface went down"),
        }
    }
}
