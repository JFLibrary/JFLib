// wlantrace -- the decisions of WLAN as notes (Note::Wlan) with their German sentences: a
// station's scan, authentication, association, the 4-way handshake on both sides, roaming,
// signal and range, disassociation and its back-off; an access point's stations, what it
// drops (a station that has not finished its handshake, one that is not there, client
// isolation and the guest network)
//
//   WNote::message(port) -> Msg     the sentence (port names a port index of the device)
//   WNote::is_drop() / mask()       something thrown away; the P_* bits (EAPOL for the
//                                   handshake, nothing for management, which is no frame here)
//
// Names of the other side are the device names at the time of the decision; signals are dBm,
// rates kbit/s (written as Mbit/s).

use alloc::format;
use alloc::string::String;

use jfn_model::msg;
use jfn_model::msg::{phrase, Msg};
use jfn_model::wlan::Band;
use netpkt::Mac;

/// a decision of a station or an access point
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WNote {
    /// a station looks for a network on its bands (bit mask) for `ms` milliseconds
    ScanStart { ssid: String, bands: u8, ms: u32 },
    /// the scan found `n` access points with the network and takes the best one
    ScanFound { ssid: String, n: u16, ap: String, band: Band, ch: u16, dbm: i32 },
    /// the scan found nothing in range
    ScanNone { ssid: String },
    /// the network wants a passphrase the station has not got
    NoPassphrase { ssid: String },
    /// the station authenticates (open system or SAE), or reassociates while roaming
    AuthStart { ap: String, sae: bool, reassoc: bool },
    /// the access point: a station authenticates
    AuthBy { sta: String, sae: bool },
    /// the access point: the SAE confirm of a station does not fit (another password)
    SaeMismatch { sta: String },
    /// the station: no confirm came, the authentication failed
    AuthFailed { ap: String },
    /// the station is associated
    Assoc { ap: String, aid: u16, band: Band, ch: u16, dbm: i32, kbps: u32, reassoc: bool },
    /// the access point took a station on a radio port
    ApAssoc { sta: String, aid: u16, ssid: String, port: u16, reassoc: bool },
    /// the access point announces the station to the switches (Layer 2 Update frame)
    L2Update { sta: String },
    /// a message of the 4-way handshake went out or came in
    Eapol { msg: u8, out: bool, peer: String },
    /// the access point: the MIC of a message does not fit (another passphrase)
    MicBad { msg: u8, sta: String },
    /// the access point repeats a message (attempt n)
    HsRetry { msg: u8, sta: String, n: u8 },
    /// the handshake is done: the keys are in place, data may flow
    Authorized { peer: String },
    /// the access point gives up the handshake and ends the association (reason 15)
    HsTimeout { sta: String },
    /// the station: the access point ended the association (reason code)
    Deauthed { ap: String, reason: u16 },
    /// the station leaves its access point
    Left { ap: String },
    /// the access point: a station is gone (reason code)
    StaLeft { sta: String, reason: u16 },
    /// the station roams to another access point of its network
    Roam { from: String, to: String, old: i32, new: i32 },
    /// the access point: a station went over to another access point
    RoamedAway { sta: String, to: String },
    /// the station is out of the range of its access point / back in it
    OutOfRange { ap: String },
    BackInRange { ap: String, dbm: i32 },
    /// no beacons of the access point any more: the connection is lost
    BeaconLoss { ap: String },
    /// the signal changed the rate
    Rate { ap: String, dbm: i32, kbps: u32 },
    /// the next attempt after `secs` seconds (failure n)
    Backoff { secs: u32, n: u8 },
    /// the station gives up after n failures
    GiveUp { n: u8 },
    /// the access point drops data of a station that has not finished its handshake
    NotAuthorized { mac: Mac },
    /// the access point has no station with this address
    NoStation { mac: Mac },
    /// client isolation or the guest network keeps the frame away from a station or the LAN
    Isolated { mac: Mac, guest: bool },
}

/// the bands of band bits ("2,4 GHz, 5 GHz")
fn bands_text(b: u8) -> Msg {
    jfn_model::msg::list(Band::ALL.iter().filter(|x| b & x.bit() != 0).map(|x| phrase(x.title())).collect())
}

/// a rate in Mbit/s with one decimal ("1201,0 Mbit/s"; the language sets the decimal mark)
pub(crate) fn mbit(kbps: u32) -> Msg {
    msg!("{},{} Mbit/s", kbps / 1000, kbps % 1000 / 100)
}

/// a rate in Mbit/s with one decimal point, as command lines write it ("1201.0")
pub(crate) fn mbit_dot(kbps: u32) -> String {
    format!("{}.{}", kbps / 1000, kbps % 1000 / 100)
}

/// an 802.11 reason code with its meaning
fn reason_text(r: u16) -> Msg {
    match r {
        1 => msg!("Grund {}: ohne Angabe", r),
        3 => msg!("Grund {}: der Absender verlässt das Netz", r),
        4 => msg!("Grund {}: zu lange still", r),
        7 => msg!("Grund {}: nicht assoziiert", r),
        15 => msg!("Grund {}: Zeitüberschreitung im 4-Wege-Handshake", r),
        _ => msg!("Grund {}", r),
    }
}

impl WNote {
    /// thrown away
    pub fn is_drop(&self) -> bool {
        matches!(self, WNote::NotAuthorized { .. } | WNote::NoStation { .. } | WNote::Isolated { .. } | WNote::MicBad { .. })
    }
    /// the P_* bits: EAPOL for the handshake (management is no frame here)
    pub fn mask(&self) -> u32 {
        match self {
            WNote::Eapol { .. } | WNote::MicBad { .. } | WNote::HsRetry { .. } | WNote::Authorized { .. } | WNote::HsTimeout { .. } => netpkt::decode::P_EAPOL,
            _ => 0,
        }
    }
    /// the sentence as a message
    pub fn message(&self, port: &dyn Fn(u16) -> String) -> Msg {
        match self {
            WNote::ScanStart { ssid, bands, ms } => msg!("WLAN: sucht \u{201e}{}\u{201c} auf {} ({} ms).", ssid, bands_text(*bands), ms),
            WNote::ScanFound { ssid, n: 1, ap, band, ch, dbm } => msg!("WLAN: \u{201e}{}\u{201c} gefunden bei {} ({}, Kanal {}, {} dBm).", ssid, ap, phrase(band.title()), ch, dbm),
            WNote::ScanFound { ssid, n, ap, band, ch, dbm } => msg!("WLAN: \u{201e}{}\u{201c} bei {} Access Points gefunden, der beste ist {} ({}, Kanal {}, {} dBm).", ssid, n, ap, phrase(band.title()), ch, dbm),
            WNote::ScanNone { ssid } => msg!("WLAN: \u{201e}{}\u{201c} ist nicht in Reichweite.", ssid),
            WNote::NoPassphrase { ssid } => msg!("WLAN: \u{201e}{}\u{201c} braucht eine Passphrase, die Station hat keine.", ssid),
            WNote::AuthStart { ap, sae: false, reassoc: false } => msg!("WLAN: Authentifizierung bei {} (Open System).", ap),
            WNote::AuthStart { ap, sae: true, reassoc: false } => msg!("WLAN: Authentifizierung bei {} (SAE).", ap),
            WNote::AuthStart { ap, sae: false, reassoc: true } => msg!("WLAN: Authentifizierung bei {} für den Wechsel (Open System).", ap),
            WNote::AuthStart { ap, sae: true, reassoc: true } => msg!("WLAN: Authentifizierung bei {} für den Wechsel (SAE).", ap),
            WNote::AuthBy { sta, sae: false } => msg!("WLAN: {} authentifiziert sich (Open System).", sta),
            WNote::AuthBy { sta, sae: true } => msg!("WLAN: {} authentifiziert sich (SAE).", sta),
            WNote::SaeMismatch { sta } => msg!("WLAN: die SAE-Bestätigung von {} passt nicht: die Passphrase ist eine andere.", sta),
            WNote::AuthFailed { ap } => msg!("WLAN: keine Bestätigung von {}: Authentifizierung gescheitert.", ap),
            WNote::Assoc { ap, aid, band, ch, dbm, kbps, reassoc: false } => msg!("WLAN: assoziiert mit {} (AID {}, {}, Kanal {}, {} dBm, {}).", ap, aid, phrase(band.title()), ch, dbm, mbit(*kbps)),
            WNote::Assoc { ap, aid, band, ch, dbm, kbps, reassoc: true } => msg!("WLAN: reassoziiert mit {} (AID {}, {}, Kanal {}, {} dBm, {}).", ap, aid, phrase(band.title()), ch, dbm, mbit(*kbps)),
            WNote::ApAssoc { sta, aid, ssid, port: p, reassoc: false } => msg!("WLAN: {} ist an {} in \u{201e}{}\u{201c} assoziiert (AID {}).", sta, port(*p), ssid, aid),
            WNote::ApAssoc { sta, aid, ssid, port: p, reassoc: true } => msg!("WLAN: {} wechselt an {} in \u{201e}{}\u{201c} (AID {}).", sta, port(*p), ssid, aid),
            WNote::L2Update { sta } => msg!("WLAN: meldet {} den Switches (Layer-2-Update).", sta),
            WNote::Eapol { msg: m, out: true, peer } => msg!("WLAN: 4-Wege-Handshake, Nachricht {} von 4 an {}.", m, peer),
            WNote::Eapol { msg: m, out: false, peer } => msg!("WLAN: 4-Wege-Handshake, Nachricht {} von 4 von {}.", m, peer),
            WNote::MicBad { msg: m, sta } => msg!("WLAN: der MIC von Nachricht {} von {} passt nicht: die Passphrase ist eine andere. Verworfen.", m, sta),
            WNote::HsRetry { msg: m, sta, n } => msg!("WLAN: Nachricht {} an {} noch einmal (Versuch {}).", m, sta, n),
            WNote::Authorized { peer } => msg!("WLAN: Schlüssel mit {} gesetzt (CCMP): Daten dürfen fließen.", peer),
            WNote::HsTimeout { sta } => msg!("WLAN: der 4-Wege-Handshake mit {} kommt nicht zustande: Verbindung beendet (Grund 15).", sta),
            WNote::Deauthed { ap, reason } => msg!("WLAN: {} beendet die Verbindung ({}).", ap, reason_text(*reason)),
            WNote::Left { ap } => msg!("WLAN: verlässt {}.", ap),
            WNote::StaLeft { sta, reason } => msg!("WLAN: {} ist abgemeldet ({}).", sta, reason_text(*reason)),
            WNote::Roam { from, to, old, new } => msg!("WLAN: wechselt von {} ({} dBm) zu {} ({} dBm).", from, old, to, new),
            WNote::RoamedAway { sta, to } => msg!("WLAN: {} ist zu {} gewechselt.", sta, to),
            WNote::OutOfRange { ap } => msg!("WLAN: {} ist außer Reichweite.", ap),
            WNote::BackInRange { ap, dbm } => msg!("WLAN: {} ist wieder in Reichweite ({} dBm).", ap, dbm),
            WNote::BeaconLoss { ap } => msg!("WLAN: keine Beacons mehr von {}: die Verbindung ist verloren.", ap),
            WNote::Rate { ap, dbm, kbps } => msg!("WLAN: Signal von {} jetzt {} dBm, {}.", ap, dbm, mbit(*kbps)),
            WNote::Backoff { secs, n } => msg!("WLAN: neuer Versuch in {} s ({}. Fehlschlag).", secs, n),
            WNote::GiveUp { n } => msg!("WLAN: gibt nach {} Fehlschlägen auf, bis sich etwas ändert.", n),
            WNote::NotAuthorized { mac } => msg!("WLAN: Frame von {} verworfen: der 4-Wege-Handshake ist noch nicht fertig.", mac),
            WNote::NoStation { mac } => msg!("WLAN: Frame an {} verworfen: keine solche Station.", mac),
            WNote::Isolated { mac, guest: false } => msg!("WLAN: Frame an {} verworfen: Client-Isolation.", mac),
            WNote::Isolated { mac, guest: true } => msg!("WLAN: Frame an {} verworfen: das Gastnetz erreicht nur den Router.", mac),
        }
    }
}
