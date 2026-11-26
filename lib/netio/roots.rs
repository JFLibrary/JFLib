// roots -- the system's certificate authorities for TLS clients of public servers
//
//   verify_system(chain, host, ocsp, now) -> Result<(), Refused>
//        Windows, macOS, iOS: the system checks the chain (plat::certs: its roots and policy,
//        the stapled OCSP response), then netio the leaf once more (x509::check_leaf: its name,
//        usage and extensions, and a key netio takes: the same rules on every system); Linux:
//        a path to one of the bundle's CAs (verify_chain)
//   verify_system_roots(chain, host, roots, now)
//        the same with only these CAs trusted, nothing fetched: the system's checks against a
//        PKI of one's own (tools/limbo_check.py --system); an empty host checks no name
//   system_anchors()
//        the bundle's CAs (Linux: plat::certs::root_bundle; macOS: /etc/ssl/cert.pem, where a
//        stapled response's issuer is looked for), parsed once and again when the bundle
//        changed (looked at once a minute at most); reload(): look again at the next check
//   Refused { reason, text }, Reason (UnknownCa, Expired, Name, Revoked, Usage, Other):
//        plat::certs's

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use plat::sync::Mutex;

use crate::x509::{check_leaf, verify_chain, Cert, PubKey, Usage};

pub use plat::certs::{Reason, Refused};

struct Bundle {
    anchors: Arc<Vec<Cert>>,
    /// SHA-256 of the bundle's bytes
    hash: [u8; 32],
    /// when the bundle was last read (Unix ns)
    read_ns: i64,
}

static BUNDLE: Mutex<Option<Bundle>> = Mutex::new(None);

/// how long the bundle read last counts before it is read again (ns)
const LOOK_NS: i64 = 60_000_000_000;

/// read the bundle again at the next check, not only after a minute (a program that has just
/// set SSL_CERT_FILE or SSL_CERT_DIR, or updated the bundle itself)
pub fn reload() {
    if let Some(b) = BUNDLE.lock().as_mut() {
        b.read_ns = i64::MIN;
    }
}

/// the CA certificates of the system bundle (empty on Windows and iOS: the system decides there)
pub fn system_anchors() -> Arc<Vec<Cert>> {
    let now = plat::unix_time_ns();
    let mut g = BUNDLE.lock();
    if let Some(b) = g.as_ref() {
        // (a clock set back counts as a minute gone)
        if now >= b.read_ns && now.saturating_sub(b.read_ns) < LOOK_NS {
            return b.anchors.clone();
        }
    }
    let bytes = plat::certs::root_bundle().unwrap_or_default();
    let hash = crypto::sha256(&bytes);
    if let Some(b) = g.as_mut() {
        if b.hash == hash {
            b.read_ns = now;
            return b.anchors.clone();
        }
    }
    let text = String::from_utf8_lossy(&bytes);
    let anchors = Arc::new(crate::pem::decode_all(&text, "CERTIFICATE").iter().filter_map(|d| Cert::parse(d)).collect::<Vec<Cert>>());
    *g = Some(Bundle { anchors: anchors.clone(), hash, read_ns: now });
    anchors
}

/// a server chain (leaf first) against the system's trust for this host name, with the
/// stapled OCSP response the server sent (the system reads it; netio checks it as well:
/// tls/conn.rs check_staple)
pub fn verify_system(chain: &[Cert], host: &str, ocsp: Option<&[u8]>, now: i64) -> Result<(), Refused> {
    if host.is_empty() {
        return Err(Refused::new(Reason::Name, "no host name to check"));
    }
    system(chain, host, ocsp, None, now)
}

/// a server chain against only these CAs, with the system's checks (Linux: verify_chain's);
/// an empty host checks no name
pub fn verify_system_roots(chain: &[Cert], host: &str, roots: &[Cert], now: i64) -> Result<(), Refused> {
    system(chain, host, None, Some(roots), now)
}

fn system(chain: &[Cert], host: &str, ocsp: Option<&[u8]>, roots: Option<&[Cert]>, now: i64) -> Result<(), Refused> {
    let Some(leaf) = chain.first() else { return Err(Refused::new(Reason::Other, "no certificate")) };
    let ders: Vec<&[u8]> = chain.iter().map(|c| c.der.as_slice()).collect();
    let os = match roots {
        None => plat::certs::verify_tls_server(&ders, host, ocsp, now),
        Some(r) => {
            let r: Vec<&[u8]> = r.iter().map(|c| c.der.as_slice()).collect();
            plat::certs::verify_tls_server_roots(&ders, host, &r, now)
        }
    };
    match os {
        // the leaf once more as netio checks it everywhere (x509::check_leaf): a system that
        // takes the subject's CN as a name, a CA's certificate as a server's, keyCertSign or
        // name constraints in a leaf, says the same here as on Linux
        Some(Ok(())) => {
            if matches!(leaf.key, PubKey::Other) {
                return Err(Refused::new(Reason::Other, "a key netio does not take (netio's check after the system's)"));
            }
            check_leaf(leaf, now, Some(host).filter(|h| !h.is_empty()), Usage::Server).map_err(|text| Refused { reason: leaf_reason(leaf, host, now), text: format!("{} (netio's check after the system's)", text) })
        }
        Some(r) => r,
        None => {
            let bundle;
            let anchors: &[Cert] = match roots {
                Some(r) => r,
                None => {
                    bundle = system_anchors();
                    &bundle
                }
            };
            verify_chain(chain, anchors, now, Some(host).filter(|h| !h.is_empty()), Usage::Server).map_err(|text| {
                let reason = match leaf_reason(leaf, host, now) {
                    Reason::Other => Reason::UnknownCa,
                    r => r,
                };
                Refused { reason, text }
            })
        }
    }
}

/// the reason a leaf is refused for: expired, another name, another usage, else Other
fn leaf_reason(leaf: &Cert, host: &str, now: i64) -> Reason {
    if now < leaf.not_before || now > leaf.not_after {
        Reason::Expired
    } else if !host.is_empty() && !leaf.names(host) {
        Reason::Name
    } else if !leaf.eku_allows(Usage::Server) {
        Reason::Usage
    } else {
        Reason::Other
    }
}
