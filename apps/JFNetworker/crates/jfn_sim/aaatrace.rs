// aaatrace -- the decisions of logins, AAA and RADIUS as notes (Note::Aaa) with their German
// sentences: a console or vty login accepted or refused, "enable" with its secret, RADIUS
// requests and their answers, retransmissions, timeouts, a fallback to the next method, a
// dropped answer (a wrong shared secret or authenticator), accounting records, and the vty
// telnet sessions (open, refused, closed, the line limit)
//
//   ANote::message() -> Msg     the sentence
//   ANote::is_drop() / mask()   something refused or dropped; the P_* bits (RADIUS over UDP,
//                               telnet over TCP; a local decision has none)

use alloc::string::String;

use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::decode::{P_IPV4, P_RADIUS, P_TCP, P_TELNET, P_UDP};
use netpkt::Ipv4;

/// where a login happened
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Where {
    Console,
    /// a vty line (its number)
    Vty(u16),
}

impl Where {
    /// the place as a message ("Konsole", "vty 1"); "vty" is a line name we keep
    fn name(&self) -> Msg {
        match self {
            Where::Console => msg::phrase("Konsole"),
            Where::Vty(n) => msg!("vty {}", n),
        }
    }
}

/// a decision of logins, AAA and RADIUS
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ANote {
    /// a login was accepted (the user, how: "lokal", "RADIUS", "Leitungspasswort", "ohne")
    LoginOk { at: Where, user: String, how: String, level: u8 },
    /// a login was refused after a wrong password
    LoginBad { at: Where, user: String },
    /// three tries failed: the session is closed
    LoginDenied { at: Where },
    /// "enable" to a privilege level was accepted or refused
    EnableOk { level: u8 },
    EnableBad { level: u8 },
    /// a RADIUS Access-Request went to a server (its address, the user, the attempt)
    RadiusRequest { to: Ipv4, user: String, attempt: u32 },
    /// the request went out again after no answer (attempt n of retransmit + 1)
    RadiusRetransmit { to: Ipv4, attempt: u32 },
    /// an Access-Accept came back (the privilege level from Service-Type, if any)
    RadiusAccept { from: Ipv4, user: String, level: Option<u8> },
    RadiusReject { from: Ipv4, user: String },
    RadiusChallenge { from: Ipv4, user: String },
    /// a server did not answer after the last attempt
    RadiusTimeout { to: Ipv4 },
    /// an answer was dropped: a wrong authenticator or Message-Authenticator
    RadiusDropped { from: Ipv4, why: String },
    /// no server answered: the next method is tried (its name)
    MethodFallback { next: String },
    /// no method is left (no server and no local fallback): the login is refused
    NoMethod,
    /// an accounting record went to a server (Start or Stop)
    AcctSent { to: Ipv4, user: String, start: bool },
    /// a telnet session to a vty line opened
    VtyOpen { line: u16, from: Ipv4 },
    /// a telnet connection was refused (no free line, transport input, or the line forbids it)
    VtyRefused { from: Ipv4, why: String },
    /// a vty session ended (exit / logout / idle timeout)
    VtyClosed { line: u16, idle: bool },
    /// a telnet client connected to or was refused by a remote host
    TelnetOut { to: Ipv4, port: u16 },
    TelnetRefused { to: Ipv4, port: u16, why: String },
}

impl ANote {
    pub fn message(&self) -> Msg {
        match self {
            ANote::LoginOk { at, user, how, level } if user.is_empty() => msg!("Anmeldung an der {} angenommen ({}, Rechte {}).", at.name(), how, level),
            ANote::LoginOk { at, user, how, level } => msg!("Anmeldung von {} an der {} angenommen ({}, Rechte {}).", user, at.name(), how, level),
            ANote::LoginBad { at, user } if user.is_empty() => msg!("Falsches Passwort an der {}.", at.name()),
            ANote::LoginBad { at, user } => msg!("Falsches Passwort für {} an der {}.", user, at.name()),
            ANote::LoginDenied { at } => msg!("Anmeldung an der {} nach drei Versuchen abgelehnt.", at.name()),
            ANote::EnableOk { level } => msg!("Wechsel auf Rechte {} angenommen.", level),
            ANote::EnableBad { level } => msg!("Falsches enable-Passwort für Rechte {}.", level),
            ANote::RadiusRequest { to, user, attempt } => msg!("RADIUS: Access-Request für {} an {} (Versuch {}).", user, to, attempt),
            ANote::RadiusRetransmit { to, attempt } => msg!("RADIUS: keine Antwort von {}, erneut gesendet (Versuch {}).", to, attempt),
            ANote::RadiusAccept { from, user, level: Some(l) } => msg!("RADIUS: Access-Accept für {} von {} (Rechte {}).", user, from, l),
            ANote::RadiusAccept { from, user, level: None } => msg!("RADIUS: Access-Accept für {} von {}.", user, from),
            ANote::RadiusReject { from, user } => msg!("RADIUS: Access-Reject für {} von {}.", user, from),
            ANote::RadiusChallenge { from, user } => msg!("RADIUS: Access-Challenge für {} von {}.", user, from),
            ANote::RadiusTimeout { to } => msg!("RADIUS: {} antwortet nicht.", to),
            ANote::RadiusDropped { from, why } => msg!("RADIUS: Antwort von {} verworfen ({}).", from, why),
            ANote::MethodFallback { next } => msg!("AAA: kein Server erreichbar, nächste Methode: {}.", next),
            ANote::NoMethod => msg!("AAA: keine Methode mehr \u{00fc}brig, Anmeldung abgelehnt."),
            ANote::AcctSent { to, user, start: true } => msg!("RADIUS: Accounting-Start für {} an {}.", user, to),
            ANote::AcctSent { to, user, start: false } => msg!("RADIUS: Accounting-Stop für {} an {}.", user, to),
            ANote::VtyOpen { line, from } => msg!("Telnet: Sitzung von {} auf vty {}.", from, line),
            ANote::VtyRefused { from, why } => msg!("Telnet: Verbindung von {} abgelehnt ({}).", from, why),
            ANote::VtyClosed { line, idle: true } => msg!("Telnet: vty {} nach Leerlauf geschlossen.", line),
            ANote::VtyClosed { line, idle: false } => msg!("Telnet: vty {} geschlossen.", line),
            ANote::TelnetOut { to, port } => msg!("Telnet: verbunden mit {} Port {}.", to, port),
            ANote::TelnetRefused { to, port, why } => msg!("Telnet: {} Port {} nicht erreicht ({}).", to, port, why),
        }
    }
    /// something was refused or dropped
    pub fn is_drop(&self) -> bool {
        matches!(
            self,
            ANote::LoginBad { .. } | ANote::LoginDenied { .. } | ANote::EnableBad { .. } | ANote::RadiusReject { .. } | ANote::RadiusTimeout { .. } | ANote::RadiusDropped { .. } | ANote::NoMethod | ANote::VtyRefused { .. } | ANote::TelnetRefused { .. }
        )
    }
    /// the protocols of the note (a local login decision has none)
    pub fn mask(&self) -> u32 {
        match self {
            ANote::RadiusRequest { .. } | ANote::RadiusRetransmit { .. } | ANote::RadiusAccept { .. } | ANote::RadiusReject { .. } | ANote::RadiusChallenge { .. } | ANote::RadiusTimeout { .. } | ANote::RadiusDropped { .. } | ANote::AcctSent { .. } => P_IPV4 | P_UDP | P_RADIUS,
            ANote::VtyOpen { .. } | ANote::VtyRefused { .. } | ANote::VtyClosed { .. } | ANote::TelnetOut { .. } | ANote::TelnetRefused { .. } => P_IPV4 | P_TCP | P_TELNET,
            _ => 0,
        }
    }
}
