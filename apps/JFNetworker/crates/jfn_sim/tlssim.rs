// tlssim -- TLS in the simulator: real TLS 1.3 records and a real handshake (netpkt::tls) on the
// sockets of the virtual OS, with certificates of a laboratory: what a server's certificate says, the
// one CA every device trusts, the decisions of TLS as notes
//
//   TlsSpec                what a server's "tls" says (JSON, every member may be left out):
//                          {"names": ["web.lab", "10.0.0.1"], "issuer": "lab", "org": "Bäckerei",
//                           "days": 365, "age_days": 0, "starts_in_days": 0}
//                          names: the DNS names and addresses it holds (default: the device's name);
//                          issuer "lab" (signed by the lab CA) or "self" (self-signed); days: how long
//                          it lasts from its start; age_days: how long before the simulation's start
//                          that was, starts_in_days: how long after it (decimals allowed)
//   TlsSpec::of(&Value) -> Result<_, Msg>, TlsSpec::of_script(lookup)   read and checked
//   TlsSpec::identity(os, svc) -> Identity    the key and the certificate: the same for the same
//                          project, device, service and spec (a restart keeps its fingerprint)
//   lab_ca() -> (Cert, Keypair), anchors() -> Vec<Cert>   the CA of the laboratory, what devices trust
//   seed_of(os)            32 bytes of the device's random stream for a connection
//   fault_msg(faults)      the worst of x509 F_* faults as words for a sentence
//   TlsNote                the decisions as notes (Note::Tls) with their sentences
//
// What is simulated and what is not: the bytes on the wire are what TLS 1.3 sends (ClientHello,
// ServerHello, the encrypted flight, application data under ChaCha20-Poly1305) and the certificates are
// X.509 with Ed25519.  The keys come from the project's seed, so they protect nothing and a replay is
// the same bit for bit; the handshake costs no virtual time beyond the network's; the lab CA is the
// only root there is and it exists only here.  Times are the device clock's, whose start is
// EPOCH_UNIX_NS: a certificate's times are reckoned from there, so a restart does not change it.

use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use crypto::ed25519::Keypair;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::decode::{P_IPV4, P_TCP, P_TLS};
use netpkt::tls::Identity;
use netpkt::x509::{Cert, CertSpec, F_BAD_SIGNATURE, F_EXPIRED, F_NAME, F_NOT_YET, F_SELF_SIGNED, F_UNKNOWN_ISSUER};
use netpkt::IpAddr;

use crate::httpsvc::decimal;
use crate::os::{OsApi, EPOCH_UNIX_NS};

const DAY: i64 = 86_400;

/// who signs a server's certificate
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issuer {
    /// the lab CA, which every device trusts
    Lab,
    SelfSigned,
}

/// what a server's "tls" says (see the module header)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsSpec {
    /// empty: the device's name
    pub names: Vec<String>,
    pub issuer: Issuer,
    pub org: String,
    /// seconds it lasts
    pub lasts: i64,
    /// seconds its start lies before (< 0: after) the simulation's start
    pub age: i64,
}

impl Default for TlsSpec {
    fn default() -> TlsSpec {
        TlsSpec { names: Vec::new(), issuer: Issuer::Lab, org: String::new(), lasts: 365 * DAY, age: 0 }
    }
}

impl TlsSpec {
    /// from a server's JSON
    pub fn of(c: &Value) -> Result<TlsSpec, Msg> {
        if c.as_object().is_none() {
            return Err(msg!("\u{201e}tls\u{201c} muss ein Objekt sein."));
        }
        let names: Vec<String> = c.get("names").and_then(|v| v.as_array()).unwrap_or(&[]).iter().filter_map(|v| v.as_str()).map(String::from).collect();
        let days = |k: &str| c.get(k).and_then(|v| decimal(v, DAY as u64));
        TlsSpec::make(names, c.get("issuer").and_then(|v| v.as_str()), c.get("org").and_then(|v| v.as_str()), days("days"), days("age_days"), days("starts_in_days"))
    }

    /// from its parts (a script's map: http.serve); the numbers in seconds
    pub fn make(names: Vec<String>, issuer: Option<&str>, org: Option<&str>, days: Option<u64>, age: Option<u64>, starts_in: Option<u64>) -> Result<TlsSpec, Msg> {
        let issuer = match issuer {
            None | Some("lab") => Issuer::Lab,
            Some("self") => Issuer::SelfSigned,
            Some(x) => return Err(msg!("Den Aussteller \u{201e}{}\u{201c} gibt es nicht: lab oder self.", x)),
        };
        for n in &names {
            if n.is_empty() || n.contains(char::is_whitespace) {
                return Err(msg!("\u{201e}{}\u{201c} ist kein Name für ein Zertifikat.", n));
            }
        }
        let secs = |x: Option<u64>| x.unwrap_or(0).min(i64::MAX as u64 / 4) as i64;
        Ok(TlsSpec { names, issuer, org: org.map(String::from).unwrap_or_default(), lasts: days.map_or(365 * DAY, |_| secs(days)), age: secs(age) - secs(starts_in) })
    }

    /// the key and the certificate (see the module header); svc: the service's name, for the key
    pub fn identity(&self, os: &mut OsApi, svc: &str) -> Identity {
        let names = if self.names.is_empty() { alloc::vec![String::from(os.hostname())] } else { self.names.clone() };
        let mut seed = Vec::new();
        seed.extend_from_slice(b"jfn tls key");
        seed.extend_from_slice(&os.eng.seed().to_be_bytes());
        seed.extend_from_slice(&os.dev().to_be_bytes());
        seed.extend_from_slice(svc.as_bytes());
        let k = crypto::sha256(&seed);
        let key = Keypair::from_seed(&k);
        seed.extend_from_slice(names.join(",").as_bytes());
        let serial = crypto::sha256(&seed);
        let start = EPOCH_UNIX_NS / 1_000_000_000 - self.age;
        let (mut dns, mut ips) = (Vec::new(), Vec::new());
        for n in &names {
            match IpAddr::parse(n.trim_matches(['[', ']'])) {
                Some(ip) => ips.push(ip),
                None => dns.push(n.to_ascii_lowercase()),
            }
        }
        let mut s = [0u8; 16];
        s.copy_from_slice(&serial[..16]);
        let spec = CertSpec { serial: s, cn: names[0].clone(), org: self.org.clone(), not_before: start, not_after: start.saturating_add(self.lasts), public_key: key.public, is_ca: false, dns, ips };
        let der = match self.issuer {
            Issuer::Lab => {
                let (ca, ca_key) = lab_ca();
                spec.sign(Some(&ca), &ca_key)
            }
            Issuer::SelfSigned => spec.sign(None, &key),
        };
        Identity { chain: alloc::vec![der], key }
    }
}

/// the CA of the laboratory and its key: the same everywhere
pub fn lab_ca() -> (Cert, Keypair) {
    let key = Keypair::from_seed(&crypto::sha256(b"JFNetworker lab CA key"));
    let start = EPOCH_UNIX_NS / 1_000_000_000 - 3650 * DAY;
    let mut serial = [0u8; 16];
    serial.copy_from_slice(&crypto::sha256(b"JFNetworker lab CA serial")[..16]);
    let der = CertSpec { serial, cn: String::from("JFNetworker Lab CA"), org: String::from("JFNetworker"), not_before: start, not_after: start + 100 * 365 * DAY, public_key: key.public, is_ca: true, ..CertSpec::default() }.sign(None, &key);
    (Cert::parse(&der).unwrap_or_default(), key)
}

/// what a device trusts
pub fn anchors() -> Vec<Cert> {
    alloc::vec![lab_ca().0]
}

/// 32 bytes of the device's random stream
pub fn seed_of(os: &mut OsApi) -> [u8; 32] {
    let mut s = [0u8; 32];
    for c in s.chunks_mut(4) {
        c.copy_from_slice(&os.rand().to_be_bytes());
    }
    s
}

/// the worst of F_* faults as words for the notes
pub fn fault_msg(f: u32) -> Msg {
    if f & F_BAD_SIGNATURE != 0 {
        msg!("Signatur falsch")
    } else if f & F_EXPIRED != 0 {
        msg!("abgelaufen")
    } else if f & F_NOT_YET != 0 {
        msg!("noch nicht gültig")
    } else if f & F_SELF_SIGNED != 0 {
        msg!("selbst signiert")
    } else if f & F_UNKNOWN_ISSUER != 0 {
        msg!("unbekannter Aussteller")
    } else if f & F_NAME != 0 {
        msg!("falscher Name")
    } else {
        msg!("ohne Fehler")
    }
}

/// a decision of TLS
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TlsNote {
    /// a server has its certificate
    CertMade { subject: String, issuer: String },
    /// a server read a ClientHello
    Hello { from: IpAddr, name: String },
    /// the handshake is done (peer: the other side)
    Established { peer: IpAddr, suite: String },
    /// a client refused a server's certificate (x509 F_* faults)
    CertRefused { host: String, faults: u32 },
    /// a client went on with a certificate that has faults (the user said so, or curl -k)
    CertExcused { host: String, faults: u32 },
    /// the handshake or the connection ended on an alert (ours: this side sent it)
    Failed { peer: IpAddr, alert: String, ours: bool },
}

impl TlsNote {
    pub fn message(&self) -> Msg {
        match self {
            TlsNote::CertMade { subject, issuer } => msg!("TLS: Zertifikat für {}, ausgestellt von {}.", subject, issuer),
            TlsNote::Hello { from, name } if name.is_empty() => msg!("TLS: ClientHello von {}.", from),
            TlsNote::Hello { from, name } => msg!("TLS: ClientHello von {} für {}.", from, name),
            TlsNote::Established { peer, suite } => msg!("TLS: Handshake mit {} fertig ({}), die Verbindung ist verschlüsselt.", peer, suite),
            TlsNote::CertRefused { host, faults } => msg!("TLS: Zertifikat von {} abgelehnt ({}).", host, fault_msg(*faults)),
            TlsNote::CertExcused { host, faults } => msg!("TLS: Zertifikat von {} trotz Fehler angenommen ({}).", host, fault_msg(*faults)),
            TlsNote::Failed { peer, alert, ours: true } => msg!("TLS: Verbindung mit {} beendet, Alert {} gesendet.", peer, alert),
            TlsNote::Failed { peer, alert, ours: false } => msg!("TLS: {} beendet die Verbindung mit Alert {}.", peer, alert),
        }
    }
    /// something was refused or given up
    pub fn is_drop(&self) -> bool {
        matches!(self, TlsNote::CertRefused { .. } | TlsNote::Failed { .. })
    }
    /// the protocols of the note (a certificate made on the device has none)
    pub fn mask(&self) -> u32 {
        match self {
            TlsNote::CertMade { .. } => 0,
            _ => P_IPV4 | P_TCP | P_TLS,
        }
    }
}
