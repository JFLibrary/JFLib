// roots -- the system's certificate authorities for TLS clients of public servers
//
//   verify_system(chain, host, now)   Windows: CryptoAPI checks the chain (plat::certs);
//                                     Linux: a path to one of the bundle's CAs (verify_chain)
//   system_anchors()                  Linux: the distribution's CA bundle, parsed once

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use plat::sync::Mutex;

use crate::x509::{verify_chain, Cert, Usage};

static ANCHORS: Mutex<Option<Arc<Vec<Cert>>>> = Mutex::new(None);

/// the CA certificates of the system bundle (empty on Windows: CryptoAPI decides there)
pub fn system_anchors() -> Arc<Vec<Cert>> {
    let mut g = ANCHORS.lock();
    if let Some(a) = g.as_ref() {
        return a.clone();
    }
    let text = plat::certs::root_bundle().map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let certs: Vec<Cert> = crate::pem::decode_all(&text, "CERTIFICATE").iter().filter_map(|d| Cert::parse(d)).collect();
    let a = Arc::new(certs);
    *g = Some(a.clone());
    a
}

/// a server chain (leaf first) against the system's trust for this host name
pub fn verify_system(chain: &[Cert], host: &str, now: i64) -> Result<(), String> {
    let ders: Vec<&[u8]> = chain.iter().map(|c| c.der.as_slice()).collect();
    if let Some(r) = plat::certs::verify_tls_server(&ders, host) {
        return r;
    }
    verify_chain(chain, &system_anchors(), now, Some(host).filter(|h| !h.is_empty()), Usage::Server)
}
