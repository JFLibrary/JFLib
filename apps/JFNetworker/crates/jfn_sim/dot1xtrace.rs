// dot1xtrace -- the decisions of port access control as notes (Note::Dot1x) with their German
// sentences: EAP on a port in both directions, a port that opens or stays closed, the wait after a
// refusal, a repeated request, RADIUS that does not answer, the supplicant's side, frames a closed
// port drops
//
//   DNote::message() -> Msg     the sentence
//   DNote::is_drop() / mask()   something refused or dropped; the P_* bit (EAPOL)
//
// A port is named as the device names it (a WLAN station by its device); `what` is the line
// netpkt::eap::info gives for a packet ("Request, Identity"), protocol words kept in English.

use alloc::string::String;

use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::decode::P_EAPOL;

/// a decision of 802.1X
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DNote {
    /// the authenticator sent an EAP packet
    TxEap { port: String, what: String },
    /// the authenticator got one
    RxEap { port: String, what: String },
    /// EAPOL-Start or EAPOL-Logoff came in
    Start { port: String },
    Logoff { port: String },
    /// a port opens for a user (with the VLAN RADIUS assigned)
    Open { port: String, user: String, vlan: Option<u16> },
    /// a port is refused: it stays closed for the quiet period (seconds)
    Refused { port: String, user: String, quiet: u32 },
    /// the quiet period is over, the authenticator asks again
    Again { port: String },
    /// no answer to a request: it goes out again (try n)
    Retry { port: String, n: u8 },
    /// the supplicant does not answer any more
    NoAnswer { port: String },
    /// no RADIUS server answered and no other method is left
    NoServer { port: String },
    /// another authentication begins on an open port
    Reauth { port: String },
    /// the port goes back to closed (link down, EAPOL-Logoff, configuration)
    Closed { port: String },
    /// the VLAN RADIUS assigned does not exist here: the port stays closed
    NoVlan { port: String, vlan: u16 },
    /// EAP-Success without keys for a WLAN station: EAP-MD5 cannot secure a network
    NoKeys { sta: String },
    /// a closed port dropped a frame that is not EAPOL
    Blocked { port: String },
    /// the supplicant sent EAPOL-Start
    SuppStart { port: String },
    /// the supplicant got an EAP packet and answers
    SuppEap { port: String, what: String },
    /// the supplicant is in
    SuppOk { port: String, user: String, method: String },
    /// the supplicant was refused (or its method ended)
    SuppFail { port: String, user: String },
    /// nobody answers EAPOL-Start: the port goes on without 802.1X
    SuppNoPeer { port: String },
    /// the supplicant has nothing to log in with
    SuppNoCreds { port: String },
    /// the supplicant logged off
    SuppLogoff { port: String },
}

impl DNote {
    pub fn message(&self) -> Msg {
        match self {
            DNote::TxEap { port, what } => msg!("802.1X: EAP {} an {} gesendet.", what, port),
            DNote::RxEap { port, what } => msg!("802.1X: EAP {} von {}.", what, port),
            DNote::Start { port } => msg!("802.1X: EAPOL-Start an {}.", port),
            DNote::Logoff { port } => msg!("802.1X: EAPOL-Logoff an {}.", port),
            DNote::Open { port, user, vlan: Some(v) } => msg!("802.1X: {} ist für {} offen, VLAN {} von RADIUS.", port, user, v),
            DNote::Open { port, user, vlan: None } => msg!("802.1X: {} ist für {} offen.", port, user),
            DNote::Refused { port, user, quiet } => msg!("802.1X: {} abgelehnt, {} bleibt {} s geschlossen.", user, port, quiet),
            DNote::Again { port } => msg!("802.1X: Wartezeit von {} vorbei, neue Anfrage.", port),
            DNote::Retry { port, n } => msg!("802.1X: keine Antwort an {}, Anfrage erneut (Versuch {}).", port, n),
            DNote::NoAnswer { port } => msg!("802.1X: {} antwortet nicht mehr.", port),
            DNote::NoServer { port } => msg!("802.1X: kein RADIUS-Server für {} erreichbar, keine Methode übrig.", port),
            DNote::Reauth { port } => msg!("802.1X: {} meldet sich neu an.", port),
            DNote::Closed { port } => msg!("802.1X: {} ist wieder geschlossen.", port),
            DNote::NoVlan { port, vlan } => msg!("802.1X: VLAN {} von RADIUS gibt es hier nicht, {} bleibt geschlossen.", vlan, port),
            DNote::NoKeys { sta } => msg!("802.1X: EAP-Success für {} ohne Schlüssel, die Methode taugt nicht für WPA-Enterprise.", sta),
            DNote::Blocked { port } => msg!("802.1X: {} ist nicht autorisiert, Frame verworfen.", port),
            DNote::SuppStart { port } => msg!("Supplicant: EAPOL-Start auf {}.", port),
            DNote::SuppEap { port, what } => msg!("Supplicant: EAP {} auf {}.", what, port),
            DNote::SuppOk { port, user, method } => msg!("Supplicant: {} auf {} angemeldet ({}).", user, port, method),
            DNote::SuppFail { port, user } => msg!("Supplicant: Anmeldung von {} auf {} abgelehnt.", user, port),
            DNote::SuppNoPeer { port } => msg!("Supplicant: niemand antwortet auf {}, der Port läuft ohne 802.1X.", port),
            DNote::SuppNoCreds { port } => msg!("Supplicant: keine Zugangsdaten für {}.", port),
            DNote::SuppLogoff { port } => msg!("Supplicant: auf {} abgemeldet.", port),
        }
    }
    /// something was refused or dropped
    pub fn is_drop(&self) -> bool {
        matches!(self, DNote::Refused { .. } | DNote::NoAnswer { .. } | DNote::NoServer { .. } | DNote::NoVlan { .. } | DNote::NoKeys { .. } | DNote::Blocked { .. } | DNote::SuppFail { .. } | DNote::SuppNoCreds { .. })
    }
    /// the protocols of the note
    pub fn mask(&self) -> u32 {
        P_EAPOL
    }
}
