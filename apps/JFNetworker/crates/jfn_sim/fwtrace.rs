// fwtrace -- the decisions of the zone firewall as notes (Note::Fw) with their German sentences:
// a packet between two zones, what decided about it, a session that starts, ends or is cleared
//
//   FwNote::message() -> Msg     the sentence
//   FwNote::is_drop() / mask()   the packet went no further; the P_* bits (the protocol it is about)
//
// `what` is a packet as fw.rs words it ("tcp 10.0.0.1:1024 -> 10.0.0.2:80"), zones are named as
// configured ("self" is the device itself).

use alloc::string::String;

use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::decode::{P_ICMP, P_ICMPV6, P_IPV4, P_IPV6, P_TCP, P_UDP};
use netpkt::ipv4::{PROTO_ICMP, PROTO_TCP, PROTO_UDP};

/// a decision of the firewall
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FwNote {
    /// both interfaces are in one zone
    SameZone { zone: String, what: String, proto: u8, v6: bool },
    /// to or from the device itself, and no policy says otherwise
    SelfPass { from: String, to: String, what: String, proto: u8, v6: bool },
    /// one interface in a zone, the other in none: dropped
    Unzoned { zone: String, what: String, proto: u8, v6: bool },
    /// no zone pair with a policy between the zones: dropped
    NoRule { from: String, to: String, what: String, proto: u8, v6: bool },
    /// a class chose "inspect": a new session
    NewSession { id: u64, pair: String, class: String, what: String, proto: u8, v6: bool },
    /// a packet of a session, on the way there or back
    Session { id: u64, back: bool, what: String, proto: u8, v6: bool },
    /// an ICMP error about a packet of a session
    Related { id: u64, what: String, proto: u8, v6: bool },
    /// a class chose "pass": no session, nothing comes back by itself
    Pass { pair: String, class: String, what: String, proto: u8, v6: bool },
    /// a class chose "drop", silently
    Drop { pair: String, class: String, what: String, proto: u8, v6: bool },
    /// a class chose "reject": dropped, and the sender is told
    Reject { pair: String, class: String, what: String, proto: u8, v6: bool },
    /// "inspect" met a packet that begins nothing (TCP without SYN, an answer nobody asked for)
    NotNew { pair: String, what: String, proto: u8, v6: bool },
    /// the limit of half-open sessions is reached
    Limit { pair: String, max: u32, what: String, proto: u8, v6: bool },
    /// a TCP session ended by RST
    Reset { id: u64, what: String, proto: u8, v6: bool },
    /// a session timed out
    Expired { id: u64, what: String, proto: u8, v6: bool },
}

impl FwNote {
    /// the packet went no further
    pub fn is_drop(&self) -> bool {
        matches!(self, FwNote::Unzoned { .. } | FwNote::NoRule { .. } | FwNote::Drop { .. } | FwNote::Reject { .. } | FwNote::NotNew { .. } | FwNote::Limit { .. })
    }

    fn proto(&self) -> (u8, bool) {
        match self {
            FwNote::SameZone { proto, v6, .. }
            | FwNote::SelfPass { proto, v6, .. }
            | FwNote::Unzoned { proto, v6, .. }
            | FwNote::NoRule { proto, v6, .. }
            | FwNote::NewSession { proto, v6, .. }
            | FwNote::Session { proto, v6, .. }
            | FwNote::Related { proto, v6, .. }
            | FwNote::Pass { proto, v6, .. }
            | FwNote::Drop { proto, v6, .. }
            | FwNote::Reject { proto, v6, .. }
            | FwNote::NotNew { proto, v6, .. }
            | FwNote::Limit { proto, v6, .. }
            | FwNote::Reset { proto, v6, .. }
            | FwNote::Expired { proto, v6, .. } => (*proto, *v6),
        }
    }

    /// the netpkt::decode P_* bits of the protocol the decision is about
    pub fn mask(&self) -> u32 {
        let (proto, v6) = self.proto();
        let family = if v6 { P_IPV6 } else { P_IPV4 };
        family
            | match proto {
                PROTO_ICMP => P_ICMP,
                58 => P_ICMPV6,
                PROTO_UDP => P_UDP,
                PROTO_TCP => P_TCP,
                _ => 0,
            }
    }

    /// the decision as a sentence
    pub fn message(&self) -> Msg {
        match self {
            FwNote::SameZone { zone, what, .. } => msg!("Firewall: {} bleibt in Zone {}, es passiert.", what, zone),
            FwNote::SelfPass { from, to, what, .. } => msg!("Firewall: {} von {} nach {}: das Gerät selbst, keine Richtlinie, es passiert.", what, from, to),
            FwNote::Unzoned { zone, what, .. } => msg!("Firewall: {} zwischen Zone {} und einer Schnittstelle ohne Zone, verworfen.", what, zone),
            FwNote::NoRule { from, to, what, .. } => msg!("Firewall: {} von {} nach {}: keine Regel zwischen den Zonen, verworfen.", what, from, to),
            FwNote::NewSession { id, pair, class, what, .. } => msg!("Firewall: {} beginnt Sitzung {} ({}, Klasse {}).", what, id, pair, class),
            FwNote::Session { id, back: false, what, .. } => msg!("Firewall: {} gehört zu Sitzung {} (Hinweg).", what, id),
            FwNote::Session { id, back: true, what, .. } => msg!("Firewall: {} gehört zu Sitzung {} (Rückweg).", what, id),
            FwNote::Related { id, what, .. } => msg!("Firewall: {} ist ein ICMP-Fehler zu Sitzung {} und passiert.", what, id),
            FwNote::Pass { pair, class, what, .. } => msg!("Firewall: {} passiert ohne Sitzung ({}, Klasse {}).", what, pair, class),
            FwNote::Drop { pair, class, what, .. } => msg!("Firewall: {} verworfen ({}, Klasse {}).", what, pair, class),
            FwNote::Reject { pair, class, what, .. } => msg!("Firewall: {} abgelehnt, der Absender erfährt es ({}, Klasse {}).", what, pair, class),
            FwNote::NotNew { pair, what, .. } => msg!("Firewall: {} beginnt keine Verbindung und gehört zu keiner Sitzung, verworfen ({}).", what, pair),
            FwNote::Limit { pair, max, what, .. } => msg!("Firewall: {} verworfen, schon {} halboffene Sitzungen ({}).", what, max, pair),
            FwNote::Reset { id, what, .. } => msg!("Firewall: {} beendet Sitzung {} (RST).", what, id),
            FwNote::Expired { id, what, .. } => msg!("Firewall: Sitzung {} ({}) ist abgelaufen.", id, what),
        }
    }
}
