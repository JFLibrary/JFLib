// trace6 -- what a device decides about IPv6, as notes of the trace (Note::V6) with their
// German sentences
//
//   Note6                  interfaces starting and stopping, duplicate address detection,
//                          addresses formed, deprecated and expired, neighbor solicitations and
//                          advertisements, the states of the neighbor cache, router
//                          solicitations and advertisements, default routers and on-link
//                          prefixes, local delivery, forwarding and why a packet was dropped,
//                          fragments and path MTUs, ICMPv6 echo and errors with their limit
//   Note6::message(port, iface)   the sentence as a jfn_model::msg template (English rows in
//                          src/locales/engine.tsv); `iface` names an IPv4 interface number
//                          (IPv6 runs on the same interfaces)
//   Note6::mask() / is_drop() / is_nd() / is_icmp()   filter bits, the red crosses of the
//                          simulation mode, what "debug ipv6 nd" and "debug ipv6 icmp" print
//
// Every note is layer 3 (Neighbor Discovery is ICMPv6, unlike ARP).

use alloc::string::String;

use jfn_model::msg;
use jfn_model::msg::{phrase, Msg};
use netpkt::icmpv6::NdErr;
use netpkt::decode::{P_ICMPV6, P_IPV6, P_TCP, P_UDP};
use netpkt::{Ipv6, Mac, Prefix6};

use crate::ip6::{NState, Origin6};
use crate::kernel::{Ns, MS};

/// why a router advertisement was not taken
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaWhy {
    /// the device routes: advertisements are for hosts
    Router,
    /// the interface takes none (IOS without "ipv6 address autoconfig", Linux accept_ra 0)
    Off,
}

/// a decision of a device about IPv6
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note6 {
    /// IPv6 runs on an interface now (its link-local address) / no longer
    Up { iface: u16, ll: Option<Ipv6> },
    Down { iface: u16 },
    /// duplicate address detection: starts after a random delay, a probe went out, nobody
    /// answered (the address is used now), another node has it
    DadStart { iface: u16, addr: Ipv6, delay: Ns },
    DadProbe { iface: u16, addr: Ipv6 },
    DadOk { iface: u16, addr: Ipv6 },
    DadDup { iface: u16, addr: Ipv6, mac: Option<Mac> },
    /// an address was formed (SLAAC) or configured
    AddrNew { iface: u16, addr: Prefix6, origin: Origin6 },
    /// an advertisement renewed the lifetimes of an autoconfigured address (seconds)
    AddrLifetime { iface: u16, addr: Ipv6, valid: u32, preferred: u32 },
    AddrDeprecated { iface: u16, addr: Ipv6 },
    AddrExpired { iface: u16, addr: Ipv6 },
    /// a configured address was taken away
    AddrRemoved { iface: u16, addr: Ipv6 },
    /// neighbor solicitations: sent (to the solicited-node group, or unicast to probe), heard,
    /// not for one of this device's addresses
    NsSent { iface: u16, target: Ipv6, unicast: bool },
    NsHeard { iface: u16, target: Ipv6, from: Ipv6 },
    NsIgnored { target: Ipv6 },
    /// neighbor advertisements: sent, heard (with the target's MAC and the flags), about a
    /// neighbor the cache does not know
    NaSent { iface: u16, target: Ipv6, to: Ipv6, solicited: bool },
    NaHeard { iface: u16, target: Ipv6, mac: Option<Mac>, router: bool, solicited: bool },
    NaIgnored { target: Ipv6 },
    /// a neighbor cache entry is new (`from` None) or changes its state
    Neigh { iface: u16, addr: Ipv6, from: Option<NState>, to: NState, mac: Option<Mac> },
    /// a packet waits for address resolution / an older one gave way / resolution failed (the
    /// entry is gone)
    NeighWait { addr: Ipv6 },
    NeighDrop { addr: Ipv6 },
    NeighFailed { addr: Ipv6 },
    /// an ND message a receiver drops (RFC 4861 6.1, 7.1)
    NdBad { why: NdErr },
    /// router solicitations and advertisements
    RsSent { iface: u16 },
    RsHeard { iface: u16, from: Ipv6 },
    RaSent { iface: u16, to: Ipv6, prefixes: u16, lifetime: u16, solicited: bool },
    RaHeard { iface: u16, from: Ipv6, lifetime: u16, prefixes: u16 },
    RaIgnored { iface: u16, why: RaWhy },
    /// the default router list (hosts)
    RouterAdded { iface: u16, router: Ipv6, lifetime: u16 },
    RouterGone { iface: u16, router: Ipv6 },
    /// the prefix list (hosts): an on-link prefix with its valid lifetime (seconds), gone
    PrefixOnLink { iface: u16, prefix: Prefix6, valid: u32 },
    PrefixGone { iface: u16, prefix: Prefix6 },
    /// a packet for this device (its next header)
    Local { nh: u8 },
    Routed { dst: Ipv6, gw: Ipv6, iface: u16 },
    NoRoute { dst: Ipv6 },
    HopLimit { src: Ipv6 },
    TooBig { mtu: u16 },
    NotForMe { dst: Ipv6 },
    GroupIgnored { dst: Ipv6 },
    /// a link-local address never leaves its link (RFC 4291 2.5.6)
    LinkScope { src: Ipv6, dst: Ipv6 },
    /// a group or loopback address as the source
    Martian { src: Ipv6 },
    Malformed,
    BadChecksum,
    /// IPv6 is off on the interface: the packet is dropped
    Off { iface: u16 },
    Fragmented { parts: u16, mtu: u16 },
    Reassembled { bytes: u32 },
    ReasmTimeout,
    ReasmBad,
    PmtuLearned { dst: Ipv6, mtu: u16 },
    /// ICMPv6
    EchoAnswered { to: Ipv6 },
    IcmpSent { typ: u8, code: u8, to: Ipv6 },
    IcmpLimited,
    UdpNoPort { port: u16 },
    TcpReset { port: u16 },
}

/// why a receiver drops an ND message, in German (a phrase of the catalogue)
pub fn nd_why(e: NdErr) -> &'static str {
    // i18n
    match e {
        NdErr::Code(_) => "der Code ist nicht 0",
        NdErr::Short => "kürzer als ihre festen Felder",
        NdErr::BadOption => "eine Option mit Länge 0 oder über das Ende hinaus",
        NdErr::Multicast => "eine Multicast-Adresse als Ziel",
        NdErr::NotNd => "keine Neighbor-Discovery-Nachricht",
        NdErr::HopLimit => "das Hop Limit ist nicht 255, sie kam nicht von diesem Link",
        NdErr::Source => "der Absender ist keine Link-Local-Adresse",
        NdErr::DadDest => "eine Adressprüfung muss an eine Solicited-Node-Gruppe gehen",
        NdErr::SourceLla => "ein unspezifizierter Absender mit Link-Layer-Adresse",
        NdErr::SolicitedToGroup => "eine Antwort an eine Multicast-Gruppe",
    }
}

/// the name of a neighbor state in notes (the RFC's words)
fn st(s: NState) -> &'static str {
    s.linux()
}

impl Note6 {
    /// the sentence as a message for the app's catalogue
    pub fn message(&self, _port: &dyn Fn(u16) -> String, iface: &dyn Fn(u16) -> String) -> Msg {
        match self {
            Note6::Up { iface: i, ll: Some(a) } => msg!("IPv6 läuft an {}: Link-Local-Adresse {}.", iface(*i), a),
            Note6::Up { iface: i, ll: None } => msg!("IPv6 läuft an {}.", iface(*i)),
            Note6::Down { iface: i } => msg!("IPv6 an {} angehalten: Nachbarn vergessen.", iface(*i)),
            Note6::DadStart { iface: i, addr, delay } => msg!("Adressprüfung (DAD) für {} an {} beginnt in {} ms.", addr, iface(*i), delay / MS),
            Note6::DadProbe { iface: i, addr } => msg!("DAD: Neighbor Solicitation für {} über {}: benutzt jemand diese Adresse?", addr, iface(*i)),
            Note6::DadOk { iface: i, addr } => msg!("DAD: niemand antwortet, {} gilt an {} jetzt.", addr, iface(*i)),
            Note6::DadDup { iface: i, addr, mac: Some(m) } => msg!("DAD: {} ist an {} doppelt, {} hat sie schon: die Adresse bleibt unbenutzt.", addr, iface(*i), m),
            Note6::DadDup { iface: i, addr, mac: None } => msg!("DAD: {} ist an {} doppelt: die Adresse bleibt unbenutzt.", addr, iface(*i)),
            Note6::AddrNew { iface: i, addr, origin: Origin6::Slaac } => msg!("SLAAC: {} an {} gebildet (Präfix des Routers und EUI-64).", addr, iface(*i)),
            Note6::AddrNew { iface: i, addr, .. } => msg!("Adresse {} an {} eingerichtet.", addr, iface(*i)),
            Note6::AddrLifetime { iface: i, addr, valid, preferred } => msg!("SLAAC: {} an {} gilt noch {} s, bevorzugt {} s.", addr, iface(*i), valid, preferred),
            Note6::AddrDeprecated { iface: i, addr } => msg!("{} an {} ist veraltet (Preferred Lifetime abgelaufen): nicht mehr für neue Verbindungen.", addr, iface(*i)),
            Note6::AddrExpired { iface: i, addr } => msg!("{} an {} ist abgelaufen (Valid Lifetime): entfernt.", addr, iface(*i)),
            Note6::AddrRemoved { iface: i, addr } => msg!("Adresse {} an {} entfernt.", addr, iface(*i)),
            Note6::NsSent { iface: i, target, unicast: false } => msg!("Neighbor Solicitation über {}: Wer hat {}?", iface(*i), target),
            Note6::NsSent { iface: i, target, unicast: true } => msg!("Erreichbarkeitsprüfung: Neighbor Solicitation direkt an {} über {}.", target, iface(*i)),
            Note6::NsHeard { iface: i, target, from } => msg!("Neighbor Solicitation von {} an {}: gesucht ist {}.", from, iface(*i), target),
            Note6::NsIgnored { target } => msg!("Neighbor Solicitation nach {} ist nicht für dieses Gerät: verworfen.", target),
            Note6::NaSent { iface: i, target, to, solicited: true } => msg!("Neighbor Advertisement über {} an {}: {} ist hier.", iface(*i), to, target),
            Note6::NaSent { iface: i, target, to, solicited: false } => msg!("Neighbor Advertisement über {} an {}: {} ist schon vergeben.", iface(*i), to, target),
            Note6::NaHeard { iface: i, target, mac: Some(m), router, solicited } => match (*router, *solicited) {
                (true, true) => msg!("Neighbor Advertisement an {}: {} ist {} (Router, Antwort).", iface(*i), target, m),
                (true, false) => msg!("Neighbor Advertisement an {}: {} ist {} (Router).", iface(*i), target, m),
                (false, true) => msg!("Neighbor Advertisement an {}: {} ist {} (Antwort).", iface(*i), target, m),
                (false, false) => msg!("Neighbor Advertisement an {}: {} ist {}.", iface(*i), target, m),
            },
            Note6::NaHeard { iface: i, target, mac: None, .. } => msg!("Neighbor Advertisement an {} für {} ohne Link-Layer-Adresse.", iface(*i), target),
            Note6::NaIgnored { target } => msg!("Neighbor Advertisement für {}: kein Eintrag im Neighbor Cache, verworfen.", target),
            Note6::Neigh { iface: i, addr, from: None, to, mac: Some(m) } => msg!("Neighbor Cache: {} ist {} an {} ({}).", addr, m, iface(*i), st(*to)),
            Note6::Neigh { iface: i, addr, from: None, to, mac: None } => msg!("Neighbor Cache: neuer Eintrag {} an {} ({}).", addr, iface(*i), st(*to)),
            Note6::Neigh { iface: i, addr, from: Some(f), to, .. } => msg!("Neighbor Cache: {} an {} wechselt von {} nach {}.", addr, iface(*i), st(*f), st(*to)),
            Note6::NeighWait { addr } => msg!("Wartet auf die Adressauflösung von {}.", addr),
            Note6::NeighDrop { addr } => msg!("Warteschlange für {} voll: das älteste wartende Paket wird verworfen.", addr),
            Note6::NeighFailed { addr } => msg!("{} antwortet nicht auf Neighbor Solicitations: Eintrag gelöscht, wartende Pakete verworfen.", addr),
            Note6::NdBad { why } => msg!("Neighbor-Discovery-Nachricht verworfen: {}.", phrase(nd_why(*why))),
            Note6::RsSent { iface: i } => msg!("Router Solicitation über {}: gibt es hier einen Router?", iface(*i)),
            Note6::RsHeard { iface: i, from } => msg!("Router Solicitation von {} an {}: ein Router Advertisement folgt.", from, iface(*i)),
            Note6::RaSent { iface: i, to, prefixes, lifetime, solicited: true } => msg!("Router Advertisement über {} an {} (auf Anfrage): Lebensdauer {} s, Präfixe: {}.", iface(*i), to, lifetime, prefixes),
            Note6::RaSent { iface: i, to, prefixes, lifetime, solicited: false } => msg!("Router Advertisement über {} an {}: Lebensdauer {} s, Präfixe: {}.", iface(*i), to, lifetime, prefixes),
            Note6::RaHeard { iface: i, from, lifetime, prefixes } => msg!("Router Advertisement von {} an {}: Lebensdauer {} s, Präfixe: {}.", from, iface(*i), lifetime, prefixes),
            Note6::RaIgnored { iface: i, why: RaWhy::Router } => msg!("Router Advertisement an {} ignoriert: dieses Gerät routet selbst.", iface(*i)),
            Note6::RaIgnored { iface: i, why: RaWhy::Off } => msg!("Router Advertisement an {} ignoriert: das Interface nimmt keine an.", iface(*i)),
            Note6::RouterAdded { iface: i, router, lifetime } => msg!("Standard-Router {} an {} für {} s.", router, iface(*i), lifetime),
            Note6::RouterGone { iface: i, router } => msg!("Standard-Router {} an {} ist weg (Lebensdauer abgelaufen oder 0).", router, iface(*i)),
            Note6::PrefixOnLink { iface: i, prefix, valid } => msg!("Präfix {} liegt an {} (gültig {} s).", prefix, iface(*i), valid),
            Note6::PrefixGone { iface: i, prefix } => msg!("Präfix {} an {} ist abgelaufen.", prefix, iface(*i)),
            Note6::Local { nh } => msg!("IPv6-Paket für dieses Gerät ({}).", netpkt::ipv4::proto_name(*nh).unwrap_or("?")),
            Note6::Routed { dst, gw, iface: i } => {
                if dst == gw {
                    msg!("IPv6-Route zu {}: direkt angeschlossen an {}.", dst, iface(*i))
                } else {
                    msg!("IPv6-Route zu {}: über {} an {}.", dst, gw, iface(*i))
                }
            }
            Note6::NoRoute { dst } => msg!("Keine IPv6-Route zu {}.", dst),
            Note6::HopLimit { src } => msg!("Hop Limit abgelaufen: Paket von {} verworfen.", src),
            Note6::TooBig { mtu } => msg!("Paket größer als die MTU {}: Router fragmentieren IPv6 nicht, verworfen.", mtu),
            Note6::NotForMe { dst } => msg!("IPv6-Paket an {} verworfen: nicht für dieses Gerät, es routet nicht.", dst),
            Note6::GroupIgnored { dst } => msg!("Multicast an {} verworfen: das Interface hört nicht auf diese Gruppe.", dst),
            Note6::LinkScope { src, dst } => msg!("{} an {} bleibt auf seinem Link: Link-Local-Adressen werden nicht weitergeleitet.", src, dst),
            Note6::Martian { src } => msg!("Verworfen: {} darf keine Absenderadresse sein.", src),
            Note6::Malformed => msg!("Verworfen: unlesbares IPv6-Paket."),
            Note6::BadChecksum => msg!("Verworfen: Prüfsumme falsch."),
            Note6::Off { iface: i } => msg!("IPv6 ist an {} aus: verworfen.", iface(*i)),
            Note6::Fragmented { parts, mtu } => msg!("In {} Fragmente mit Fragment Header geteilt (MTU {}).", parts, mtu),
            Note6::Reassembled { bytes } => msg!("IPv6-Fragmente zu {} Bytes zusammengesetzt.", bytes),
            Note6::ReasmTimeout => msg!("IPv6-Fragmente nach 60 s verworfen: nicht vollständig."),
            Note6::ReasmBad => msg!("IPv6-Fragment verworfen: überlappend oder mit falscher Länge."),
            Note6::PmtuLearned { dst, mtu } => msg!("Path MTU zu {} ist {}: größere Pakete werden fragmentiert.", dst, mtu),
            Note6::EchoAnswered { to } => msg!("ICMPv6-Echo-Anfrage beantwortet an {}.", to),
            Note6::IcmpSent { typ, code, to } => msg!("ICMPv6 an {}: {}.", to, netpkt::icmpv6::name(*typ, *code)),
            Note6::IcmpLimited => msg!("ICMPv6-Fehlermeldung unterdrückt (Ratenbegrenzung)."),
            Note6::UdpNoPort { port: p } => msg!("UDP über IPv6 an Port {}: niemand lauscht.", p),
            Note6::TcpReset { port: p } => msg!("TCP über IPv6 an Port {}: niemand lauscht, Antwort RST.", p),
        }
    }
    /// the P_* bits of the protocol the decision is about
    pub fn mask(&self) -> u32 {
        match self {
            Note6::UdpNoPort { .. } => P_IPV6 | P_UDP,
            Note6::TcpReset { .. } => P_IPV6 | P_TCP,
            Note6::Up { .. }
            | Note6::Down { .. }
            | Note6::Local { .. }
            | Note6::Routed { .. }
            | Note6::NoRoute { .. }
            | Note6::NotForMe { .. }
            | Note6::GroupIgnored { .. }
            | Note6::LinkScope { .. }
            | Note6::Martian { .. }
            | Note6::Malformed
            | Note6::Off { .. }
            | Note6::Fragmented { .. }
            | Note6::Reassembled { .. }
            | Note6::ReasmTimeout
            | Note6::ReasmBad
            | Note6::AddrDeprecated { .. }
            | Note6::AddrExpired { .. }
            | Note6::AddrRemoved { .. }
            | Note6::NeighWait { .. }
            | Note6::NeighDrop { .. } => P_IPV6,
            _ => P_IPV6 | P_ICMPV6,
        }
    }
    /// something was thrown away
    pub fn is_drop(&self) -> bool {
        matches!(
            self,
            Note6::NsIgnored { .. }
                | Note6::NaIgnored { .. }
                | Note6::NeighDrop { .. }
                | Note6::NeighFailed { .. }
                | Note6::NdBad { .. }
                | Note6::RaIgnored { .. }
                | Note6::NoRoute { .. }
                | Note6::HopLimit { .. }
                | Note6::TooBig { .. }
                | Note6::NotForMe { .. }
                | Note6::GroupIgnored { .. }
                | Note6::LinkScope { .. }
                | Note6::Martian { .. }
                | Note6::Malformed
                | Note6::BadChecksum
                | Note6::Off { .. }
                | Note6::ReasmTimeout
                | Note6::ReasmBad
                | Note6::IcmpLimited
                | Note6::UdpNoPort { .. }
        )
    }
    /// a decision of Neighbor Discovery or address configuration ("debug ipv6 nd")
    pub fn is_nd(&self) -> bool {
        matches!(
            self,
            Note6::DadStart { .. }
                | Note6::DadProbe { .. }
                | Note6::DadOk { .. }
                | Note6::DadDup { .. }
                | Note6::AddrNew { .. }
                | Note6::AddrLifetime { .. }
                | Note6::AddrDeprecated { .. }
                | Note6::AddrExpired { .. }
                | Note6::AddrRemoved { .. }
                | Note6::NsSent { .. }
                | Note6::NsHeard { .. }
                | Note6::NaSent { .. }
                | Note6::NaHeard { .. }
                | Note6::Neigh { .. }
                | Note6::NeighFailed { .. }
                | Note6::NdBad { .. }
                | Note6::RsSent { .. }
                | Note6::RsHeard { .. }
                | Note6::RaSent { .. }
                | Note6::RaHeard { .. }
                | Note6::RaIgnored { .. }
                | Note6::RouterAdded { .. }
                | Note6::RouterGone { .. }
                | Note6::PrefixOnLink { .. }
                | Note6::PrefixGone { .. }
        )
    }
    /// a decision about ICMPv6 echo and errors ("debug ipv6 icmp")
    pub fn is_icmp(&self) -> bool {
        matches!(self, Note6::EchoAnswered { .. } | Note6::IcmpSent { .. } | Note6::IcmpLimited | Note6::Local { nh: 58 })
    }
}
