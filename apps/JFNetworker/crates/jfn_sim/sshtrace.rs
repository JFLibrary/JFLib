// sshtrace -- the decisions of SSH as notes (Note::Ssh) with their German sentences: what a server
// takes, refuses and ends (connections, the agreed algorithms, logins, sessions, a login that took
// too long, a broken protocol), what a client does (it connects, trusts or refuses a host key, logs
// in, ends) and the host keys a device makes or forgets
//
//   SshNote::message() -> Msg     the sentence
//   SshNote::is_drop() / mask()   something refused or given up; the P_* bits (SSH over TCP)
//
// The logins of IOS-like devices also have the notes of AAA (aaatrace.rs), since they run through
// its gate.

use alloc::string::String;

use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::decode::{P_IPV4, P_SSH, P_TCP};
use netpkt::IpAddr;

/// a decision of SSH
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshNote {
    /// a device made a host key or a user key
    KeyMade { kind: String, bits: u32, fingerprint: String },
    /// a device forgot its host keys ("crypto key zeroize")
    KeyGone { kind: String },
    /// a server took a connection (line: the vty line of an IOS-like device)
    Accept { from: IpAddr, line: Option<u16> },
    /// a server refused a connection (no free line, transport input, access-class, no host key)
    Refused { from: IpAddr, why: String },
    /// the key exchange is done
    Keyed { peer: IpAddr, cipher: String, host_key: String },
    /// a server let someone in
    LoginOk { from: IpAddr, user: String, how: String },
    /// a server refused a login
    LoginBad { from: IpAddr, user: String, how: String },
    /// too many tries: the server ends the connection
    TooManyTries { from: IpAddr, user: String },
    /// the login did not end in time ("ip ssh time-out")
    LoginTimeout { from: IpAddr },
    /// a session started: a shell, or a command
    Session { from: IpAddr, what: String },
    /// a session ended
    Closed { from: IpAddr, idle: bool },
    /// the connection ended on a protocol error or a message of the peer
    Broken { peer: IpAddr, why: String },
    /// a client reached a server
    Connected { to: IpAddr, port: u16 },
    /// a client could not reach a server
    Unreachable { to: IpAddr, port: u16, why: String },
    /// a client met a host key it did not know and remembered it
    HostKeyNew { host: String, fingerprint: String },
    /// a client's list of known hosts has another key for that host: it refuses
    HostKeyChanged { host: String, fingerprint: String },
    /// a client did not get to trust a new key
    HostKeyRefused { host: String, fingerprint: String },
    /// a client's login went through (how: password, publickey, none)
    ClientLogin { to: IpAddr, user: String, how: String },
    /// a client gave up on the logins
    ClientDenied { to: IpAddr, user: String },
    /// a client ended with the remote's exit status
    ClientEnd { to: IpAddr, status: i32 },
    /// a file went over scp
    Scp { peer: IpAddr, file: String, bytes: usize, upload: bool },
}

impl SshNote {
    pub fn message(&self) -> Msg {
        match self {
            SshNote::KeyMade { kind, bits, fingerprint } => msg!("SSH: neuer Schlüssel {} ({} Bit, {}).", kind, bits, fingerprint),
            SshNote::KeyGone { kind } => msg!("SSH: Schlüssel {} gelöscht.", kind),
            SshNote::Accept { from, line: Some(n) } => msg!("SSH: Verbindung von {} auf vty {}.", from, n),
            SshNote::Accept { from, line: None } => msg!("SSH: Verbindung von {}.", from),
            SshNote::Refused { from, why } => msg!("SSH: Verbindung von {} abgelehnt ({}).", from, why),
            SshNote::Keyed { peer, cipher, host_key } => msg!("SSH: Schlüsselaustausch mit {} fertig ({}, Schlüssel {}), die Verbindung ist verschlüsselt.", peer, cipher, host_key),
            SshNote::LoginOk { from, user, how } => msg!("SSH: {} von {} angemeldet ({}).", user, from, how),
            SshNote::LoginBad { from, user, how } => msg!("SSH: Anmeldung von {} aus {} abgelehnt ({}).", user, from, how),
            SshNote::TooManyTries { from, user } => msg!("SSH: zu viele Versuche von {} aus {}: die Verbindung endet.", user, from),
            SshNote::LoginTimeout { from } => msg!("SSH: Anmeldung von {} dauerte zu lange: die Verbindung endet.", from),
            SshNote::Session { from, what } => msg!("SSH: Sitzung von {}: {}.", from, what),
            SshNote::Closed { from, idle: true } => msg!("SSH: Sitzung von {} nach Leerlauf geschlossen.", from),
            SshNote::Closed { from, idle: false } => msg!("SSH: Sitzung von {} geschlossen.", from),
            SshNote::Broken { peer, why } => msg!("SSH: Verbindung mit {} abgebrochen ({}).", peer, why),
            SshNote::Connected { to, port } => msg!("SSH: verbunden mit {} Port {}.", to, port),
            SshNote::Unreachable { to, port, why } => msg!("SSH: {} Port {} nicht erreicht ({}).", to, port, why),
            SshNote::HostKeyNew { host, fingerprint } => msg!("SSH: Schlüssel von {} neu gemerkt ({}).", host, fingerprint),
            SshNote::HostKeyChanged { host, fingerprint } => msg!("SSH: Schlüssel von {} hat sich geändert ({}): Verbindung verweigert.", host, fingerprint),
            SshNote::HostKeyRefused { host, fingerprint } => msg!("SSH: Schlüssel von {} nicht angenommen ({}).", host, fingerprint),
            SshNote::ClientLogin { to, user, how } => msg!("SSH: als {} bei {} angemeldet ({}).", user, to, how),
            SshNote::ClientDenied { to, user } => msg!("SSH: Anmeldung als {} bei {} aufgegeben.", user, to),
            SshNote::ClientEnd { to, status } => msg!("SSH: Sitzung mit {} beendet (Status {}).", to, status),
            SshNote::Scp { peer, file, bytes, upload: true } => msg!("SCP: {} ({} Byte) an {} geschickt.", file, bytes, peer),
            SshNote::Scp { peer, file, bytes, upload: false } => msg!("SCP: {} ({} Byte) von {} geholt.", file, bytes, peer),
        }
    }
    /// something was refused or given up
    pub fn is_drop(&self) -> bool {
        matches!(
            self,
            SshNote::Refused { .. } | SshNote::LoginBad { .. } | SshNote::TooManyTries { .. } | SshNote::LoginTimeout { .. } | SshNote::Broken { .. } | SshNote::Unreachable { .. } | SshNote::HostKeyChanged { .. } | SshNote::HostKeyRefused { .. } | SshNote::ClientDenied { .. }
        )
    }
    /// the protocols of the note (a key made on the device has none)
    pub fn mask(&self) -> u32 {
        match self {
            SshNote::KeyMade { .. } | SshNote::KeyGone { .. } => 0,
            _ => P_IPV4 | P_TCP | P_SSH,
        }
    }
}
