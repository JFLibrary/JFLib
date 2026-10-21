// trust -- https to a server with its own private CA (Ed25519): the client either has the CA
// (a PEM file), has pinned the CA's fingerprint, or learns it on the first connection and lets
// the user decide (trust on first use, like SSH host keys)
//
//   presented_ca(&chain, name, now)  the CA at the end of a server's chain that issued the
//                                    server certificate, valid now and for `name`
//   connector(ServerTrust, identity, seen)   for Client::with_connector; a CA that is unknown
//                                    (Ask) or not the pinned one ends the handshake with
//                                    HttpError::Tls(UNTRUSTED) and lands in `seen`
//   fingerprint_text(&digest)        "AB:CD:..." for people to compare
//   system_connector(extra_cas)      https to public servers: the system's CAs (and these),
//                                    TLS 1.3 or 1.2 (mail servers, update servers, web hooks)

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use plat::net::TcpStream;
use plat::sync::Mutex;

use crate::client::{Connector, HttpError};
use crate::tls::{self, ClientConfig, Identity, Trust};
use crate::url::Url;
use crate::x509::{verify_chain, Cert, Usage};
use crate::Stream;

/// the text of HttpError::Tls when the server's CA needs the user's decision
pub const UNTRUSTED: &str = "untrusted-ca";

#[derive(Clone)]
pub enum ServerTrust {
    /// certificates of this CA only
    Ca(Cert),
    /// the CA with this SHA-256 (of its DER)
    Pin([u8; 32]),
    /// learn the CA: every connection stops with UNTRUSTED after the handshake
    Ask,
}

/// a CA a server presented that is not (yet) trusted
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presented {
    pub fingerprint: [u8; 32],
    /// the CA's common name
    pub name: String,
    /// a different CA than the pinned one (a reinstalled server, or someone in between)
    pub changed: bool,
    /// the CA itself (PEM), to keep instead of the fingerprint
    pub pem: String,
}

pub fn presented_ca(chain: &[Cert], name: &str, now: i64) -> Result<Cert, String> {
    let ca = match chain.last() {
        Some(c) if chain.len() >= 2 && c.is_ca => c,
        _ => return Err(String::from("der Server zeigt keine eigene Zertifizierungsstelle")),
    };
    verify_chain(&chain[..chain.len() - 1], core::slice::from_ref(ca), now, Some(name), Usage::Server)?;
    Ok(ca.clone())
}

/// "AB:CD:EF:..." of a digest
pub fn fingerprint_text(d: &[u8]) -> String {
    let h = crypto::hex(d).to_uppercase();
    h.as_bytes().chunks(2).map(|c| core::str::from_utf8(c).unwrap_or("")).collect::<Vec<_>>().join(":")
}

/// "AB:CD:.." / "abcd.." / with spaces -> the 32 bytes
pub fn parse_fingerprint(s: &str) -> Option<[u8; 32]> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let b = crypto::unhex(&hex)?;
    let mut out = [0u8; 32];
    (b.len() == 32).then(|| {
        out.copy_from_slice(&b);
        out
    })
}

/// https to public servers (the system's certificate authorities and `extra`)
pub fn system_connector(extra: Vec<Cert>) -> Connector {
    Arc::new(move |url: &Url, tcp: TcpStream| {
        let cfg = ClientConfig { trust: Trust::System(extra.clone()), identity: None, server_name: url.host.clone() };
        tls::connect(Box::new(tcp), cfg).map(|s| Box::new(s) as Box<dyn Stream>).map_err(|e| HttpError::Tls(format!("{}", e)))
    })
}

pub fn connector(trust: ServerTrust, identity: Option<Identity>, seen: Arc<Mutex<Option<Presented>>>) -> Connector {
    Arc::new(move |url: &Url, tcp: TcpStream| {
        let host = url.host.clone();
        let cfg = match &trust {
            ServerTrust::Ca(ca) => ClientConfig { trust: Trust::Anchors(alloc::vec![ca.clone()]), identity: identity.clone(), server_name: host },
            // the chain is checked below, against the CA it ends in
            _ => ClientConfig { trust: Trust::Any, identity: identity.clone(), server_name: String::new() },
        };
        let s = tls::connect(Box::new(tcp), cfg).map_err(|e| HttpError::Tls(format!("{}", e)))?;
        if let ServerTrust::Ca(_) = trust {
            return Ok(Box::new(s) as Box<dyn Stream>);
        }
        let now = plat::unix_time_ns().div_euclid(1_000_000_000);
        let ca = presented_ca(&s.peer_certs(), &url.host, now).map_err(HttpError::Tls)?;
        let fp = crypto::sha256(&ca.der);
        match trust {
            ServerTrust::Pin(p) if crypto::ct_eq(&p, &fp) => Ok(Box::new(s) as Box<dyn Stream>),
            _ => {
                let changed = matches!(trust, ServerTrust::Pin(_));
                *seen.lock() = Some(Presented { fingerprint: fp, name: ca.subject_cn.clone(), changed, pem: crate::pem::encode("CERTIFICATE", &ca.der) });
                s.shutdown();
                Err(HttpError::Tls(String::from(UNTRUSTED)))
            }
        }
    })
}
