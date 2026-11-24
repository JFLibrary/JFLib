// ocsp -- OCSP over HTTP (RFC 6960 A.1, RFC 5019): a certificate's status from its CA's
// responder, and the stapled responses a server keeps current
//
//   fetch(&client, &cert, &issuer) -> Result<(DER, Response), String>
//                                   a POST of the request to the certificate's responder (its
//                                   first http(s) OCSP URI), the answer checked
//   refresh(&staples, &client, &chain, issuer) -> Result<i64, String>
//                                   a server's staple for the leaf of `chain`: fetched when there
//                                   is none or it is due, kept in `staples` (tls::Staples) ->
//                                   when to call again (Unix seconds)
//
// A server calls refresh for each of its certificates at start and then at the times it
// returns; after an error again within the hour (the kept response serves until its
// nextUpdate).  Responders speak plain http as a rule: the response is signed, the request
// carries no secret (RFC 6960 A.1).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::client::Client;
use crate::http::Headers;
use crate::tls::Staples;
use crate::url::Url;
use crate::x509::ocsp::{self, CertId, Response};
use crate::x509::Cert;

/// the longest answer taken from a responder
pub const MAX_RESPONSE: usize = 64 * 1024;
/// how soon refresh asks again after a failed fetch
pub const RETRY_S: i64 = 3600;

/// the status of `cert` from its CA's OCSP responder: the first http or https URI of its
/// authority information access gets the request by POST; the answer must be a response
/// that check takes now -> its DER (to staple) and what it says
pub fn fetch(client: &Client, cert: &Cert, issuer: &Cert) -> Result<(Vec<u8>, Response), String> {
    let uri = cert.ocsp_urls.iter().find(|u| u.starts_with("http://") || u.starts_with("https://")).ok_or_else(|| String::from("the certificate names no OCSP responder"))?;
    let url = Url::parse(uri).ok_or_else(|| format!("the OCSP responder's address cannot be read: {}", uri))?;
    let id = CertId::new(cert, issuer).ok_or_else(|| String::from("the issuer given is not the certificate's"))?;
    let req = ocsp::request(&[id], None);
    let mut h = Headers::new();
    h.set("Content-Type", "application/ocsp-request");
    h.set("Accept", "application/ocsp-response");
    let mut c = client.clone();
    c.max_body = MAX_RESPONSE;
    let resp = c.request("POST", &url, &h, &req).map_err(|e| format!("OCSP responder {}: {}", uri, e))?;
    if resp.status != 200 {
        return Err(format!("OCSP responder {}: HTTP {}", uri, resp.status));
    }
    let der = Vec::from(resp.body_bytes());
    let now = plat::unix_time_ns().div_euclid(1_000_000_000);
    let r = ocsp::check(&der, cert, issuer, now)?;
    Ok((der, r))
}

/// keep the staple of the leaf of `chain` (DER, leaf first) current: fetched when there is
/// none, it is due (Response::refresh_at) or out of date.  `issuer`: the leaf's issuer when the
/// chain does not hold it.  -> when to call again (Unix seconds); Err: why the fetch failed
/// (call again within RETRY_S: the kept response serves until it is out of date)
pub fn refresh(staples: &Staples, client: &Client, chain: &[Vec<u8>], issuer: Option<&Cert>) -> Result<i64, String> {
    let now = staples.now();
    let leaf_der = chain.first().ok_or_else(|| String::from("no certificate"))?;
    if let Some(r) = staples.response(leaf_der) {
        if now < r.refresh_at() {
            return Ok(r.refresh_at());
        }
    }
    let leaf = Cert::parse(leaf_der).ok_or_else(|| String::from("the certificate cannot be read"))?;
    let found;
    let issuer = match issuer {
        Some(i) => i,
        None => {
            found = chain[1..].iter().filter_map(|c| Cert::parse(c)).find(|c| leaf.signed_by(c)).ok_or_else(|| String::from("the chain holds no issuer of the certificate"))?;
            &found
        }
    };
    let (der, _) = fetch(client, &leaf, issuer)?;
    let r = staples.set(chain, Some(issuer), der)?;
    Ok(r.refresh_at().max(now + 60))
}
