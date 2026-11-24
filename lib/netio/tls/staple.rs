// staple -- the OCSP responses a server staples (RFC 6066 8, RFC 8446 4.4.2.1)
//
//   Staples::new()                    one store for all of a server's certificates
//   staples.set(&chain, issuer, der)  a response for the leaf of `chain`, checked against it and
//                                     its issuer (chain[1], or `issuer` when the chain has none)
//   staples.get(&leaf) / response(&leaf) / remove(&leaf)
//   netio::ocsp::refresh(&staples, &client, &chain, issuer)   fetches and sets when one is due
//
// A server with ServerConfig::ocsp sends a client that asks (status_request) the current
// response of the certificate it shows; one past its nextUpdate is not sent (and dropped).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI64, Ordering};

use plat::sync::Mutex;

use crate::x509::ocsp::{self, Response};
use crate::x509::Cert;

/// the longest response kept (responses are a few KiB; one with a responder certificate and
/// an RSA signature under 4 KiB)
pub const MAX_STAPLE: usize = 32 * 1024;

struct Entry {
    der: Arc<Vec<u8>>,
    response: Response,
}

/// OCSP responses by certificate (SHA-256 of the leaf's DER)
#[derive(Default)]
pub struct Staples {
    map: Mutex<BTreeMap<[u8; 32], Entry>>,
    shift_s: AtomicI64,
}

impl Staples {
    pub fn new() -> Staples {
        Staples::default()
    }

    /// Unix seconds (tests shift the clock: test_shift)
    pub fn now(&self) -> i64 {
        plat::unix_time_ns().div_euclid(1_000_000_000) + self.shift_s.load(Ordering::Relaxed)
    }

    #[doc(hidden)]
    pub fn test_shift(&self, secs: i64) {
        self.shift_s.fetch_add(secs, Ordering::Relaxed);
    }

    /// (tests) keep any bytes for a leaf, unchecked, current for a day
    #[doc(hidden)]
    pub fn test_insert(&self, leaf: &[u8], der: Vec<u8>) {
        let now = self.now();
        let response = Response { status: ocsp::Status::Good, this_update: now, next_update: Some(now + 86_400), produced_at: now, nonce: None };
        self.map.lock().insert(crypto::sha256(leaf), Entry { der: Arc::new(der), response });
    }

    /// keep `der` for the leaf of `chain` (DER, leaf first): checked against the leaf and its
    /// issuer (`issuer`, else the certificate of the chain that signed the leaf) and current
    /// now.  A revoked or unknown status is kept too (the server shows what its CA says).
    /// -> the response read
    pub fn set(&self, chain: &[Vec<u8>], issuer: Option<&Cert>, der: Vec<u8>) -> Result<Response, String> {
        if der.len() > MAX_STAPLE {
            return Err(String::from("the OCSP response is too long to staple"));
        }
        let leaf = chain.first().and_then(|c| Cert::parse(c)).ok_or_else(|| String::from("no certificate"))?;
        let found;
        let issuer = match issuer {
            Some(i) => i,
            None => {
                found = chain[1..].iter().filter_map(|c| Cert::parse(c)).find(|c| leaf.signed_by(c)).ok_or_else(|| String::from("the chain holds no issuer of the certificate"))?;
                &found
            }
        };
        let response = ocsp::check(&der, &leaf, issuer, self.now())?;
        self.map.lock().insert(leaf.fingerprint_bytes(), Entry { der: Arc::new(der), response: response.clone() });
        Ok(response)
    }

    /// the response to staple for this leaf (DER): None when there is none or it is out of date
    /// (it is dropped then)
    pub fn get(&self, leaf: &[u8]) -> Option<Arc<Vec<u8>>> {
        let fp = crypto::sha256(leaf);
        let now = self.now();
        let mut m = self.map.lock();
        let e = m.get(&fp)?;
        if now > e.response.expires() {
            m.remove(&fp);
            return None;
        }
        Some(e.der.clone())
    }

    /// what the kept response for this leaf says (also one out of date)
    pub fn response(&self, leaf: &[u8]) -> Option<Response> {
        self.map.lock().get(&crypto::sha256(leaf)).map(|e| e.response.clone())
    }

    pub fn remove(&self, leaf: &[u8]) {
        self.map.lock().remove(&crypto::sha256(leaf));
    }

    pub fn len(&self) -> usize {
        self.map.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        self.map.lock().clear();
    }
}
