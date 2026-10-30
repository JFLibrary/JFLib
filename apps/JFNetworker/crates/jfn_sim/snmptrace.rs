// snmptrace -- the decisions of SNMP as notes (Note::Snmp) with their German sentences: what an
// agent takes, answers and refuses (requests, responses with their errors, unknown communities,
// access lists, versions it does not speak, unreadable messages, a Set from a read-only
// community, what a Set changed), its notifications (traps and informs sent, informs acknowledged,
// sent again and given up), what a trap receiver logs, and the tries of the managers of a shell
//
//   SNote::message() -> Msg     the sentence
//   SNote::is_drop() / mask()   something refused, dropped or given up; the P_* bits (SNMP over
//                               UDP)
//
// Addresses are as the datagrams came; PDUs by their names in RFC 3416, notifications by the name
// of their object (linkDown); error statuses by their names (notWritable).

use alloc::string::String;

use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::decode::{P_IPV4, P_SNMP, P_UDP};
use netpkt::snmp::{error_name, Kind, Version};
use netpkt::{IpAddr, Ipv4};

/// a decision of SNMP
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SNote {
    /// an agent takes a request
    Request { from: IpAddr, kind: Kind, community: String, id: i32, vars: u16 },
    /// it answers (error status 0: no error; the index counts from 1)
    Response { to: IpAddr, id: i32, status: i32, index: i32, vars: u16 },
    /// a community the agent does not know: no answer
    BadCommunity { from: IpAddr, community: String },
    /// the access list (or network) of the community does not let the sender in: no answer
    Denied { from: IpAddr, community: String, acl: String },
    /// a version the agent does not speak (SNMPv3): dropped
    BadVersion { from: IpAddr, version: i64 },
    /// a message that is no SNMP the agent can read: dropped
    Malformed { from: IpAddr },
    /// a Set with a community that may only read
    ReadOnly { from: IpAddr, community: String },
    /// a Set changed an object
    Set { var: String, value: String },
    /// a notification goes out (a trap, or an inform that waits for its acknowledgement)
    Notify { to: Ipv4, what: String, inform: bool, version: Version },
    /// no answer to an inform in time: sent again (attempt n)
    InformAgain { to: Ipv4, what: String, n: u8 },
    /// the receiver acknowledged an inform
    InformAcked { to: Ipv4, what: String },
    /// the last attempt went unanswered: the inform is given up
    InformLost { to: Ipv4, what: String },
    /// no way to a notification's receiver
    NoRoute { to: Ipv4 },
    /// a trap receiver logs a notification (an inform is acknowledged)
    Received { from: IpAddr, what: String, inform: bool },
    /// a trap receiver drops a notification with a community it does not take
    TrapRefused { from: IpAddr, community: String },
    /// a manager of the shell asks an agent
    Ask { to: IpAddr, kind: Kind, id: i32 },
    /// no answer in time: it asks again (attempt n)
    AskAgain { to: IpAddr, n: u8 },
    /// no answer after the last attempt
    NoAnswer { to: IpAddr },
}

impl SNote {
    pub fn message(&self) -> Msg {
        match self {
            SNote::Request { from, kind, community, id, vars } => msg!("SNMP: {} von {} (Community „{}“, Kennung {}, {} Variablen).", kind.name(), from, community, id, vars),
            SNote::Response { to, id, status: 0, vars, .. } => msg!("SNMP: Antwort an {} (Kennung {}, {} Variablen).", to, id, vars),
            SNote::Response { to, id, status, index, .. } => msg!("SNMP: Antwort an {} mit dem Fehler {} bei Variable {} (Kennung {}).", to, error_name(*status), index, id),
            SNote::BadCommunity { from, community } => msg!("SNMP: unbekannte Community „{}“ von {}: keine Antwort.", community, from),
            SNote::Denied { from, community, acl } => msg!("SNMP: {} darf die Community „{}“ nicht nutzen ({}): keine Antwort.", from, community, acl),
            SNote::BadVersion { from, version } => msg!("SNMP: Version {} von {} versteht der Agent nicht: verworfen.", version, from),
            SNote::Malformed { from } => msg!("SNMP: Nachricht von {} nicht lesbar: verworfen.", from),
            SNote::ReadOnly { from, community } => msg!("SNMP: Set von {} mit der Community „{}“, die nur lesen darf: abgelehnt.", from, community),
            SNote::Set { var, value } => msg!("SNMP: {} ist jetzt {}.", var, value),
            SNote::Notify { to, what, inform: false, version } => msg!("SNMP: Trap {} an {} (Version {}).", what, to, version.word()),
            SNote::Notify { to, what, inform: true, version } => msg!("SNMP: Inform {} an {} (Version {}), wartet auf Bestätigung.", what, to, version.word()),
            SNote::InformAgain { to, what, n } => msg!("SNMP: keine Bestätigung des Informs {} von {}: noch einmal gesendet (Versuch {}).", what, to, n),
            SNote::InformAcked { to, what } => msg!("SNMP: {} hat den Inform {} bestätigt.", to, what),
            SNote::InformLost { to, what } => msg!("SNMP: der Inform {} an {} blieb unbestätigt: aufgegeben.", what, to),
            SNote::NoRoute { to } => msg!("SNMP: kein Weg zum Empfänger {}: die Meldung geht verloren.", to),
            SNote::Received { from, what, inform: false } => msg!("SNMP: Trap {} von {} empfangen.", what, from),
            SNote::Received { from, what, inform: true } => msg!("SNMP: Inform {} von {} empfangen und bestätigt.", what, from),
            SNote::TrapRefused { from, community } => msg!("SNMP: Meldung von {} mit der unbekannten Community „{}“ verworfen.", from, community),
            SNote::Ask { to, kind, id } => msg!("SNMP: {} an {} (Kennung {}).", kind.name(), to, id),
            SNote::AskAgain { to, n } => msg!("SNMP: keine Antwort von {}: noch einmal gefragt (Versuch {}).", to, n),
            SNote::NoAnswer { to } => msg!("SNMP: {} antwortet nicht: aufgegeben.", to),
        }
    }
    /// something was refused, dropped or given up
    pub fn is_drop(&self) -> bool {
        matches!(
            self,
            SNote::BadCommunity { .. } | SNote::Denied { .. } | SNote::BadVersion { .. } | SNote::Malformed { .. } | SNote::ReadOnly { .. } | SNote::InformLost { .. } | SNote::NoRoute { .. } | SNote::TrapRefused { .. } | SNote::NoAnswer { .. }
        )
    }
    /// the protocols of the note
    pub fn mask(&self) -> u32 {
        P_IPV4 | P_UDP | P_SNMP
    }
}
