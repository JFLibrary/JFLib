// ocspt -- OCSP (RFC 6960) and certificate requests (RFC 2986) of jflib-x509; OCSP stapling in
// netio's TLS (RFC 6066 8, RFC 8446 4.4.2.1, must-staple of RFC 7633); netio::ocsp's fetch and
// refresh against a responder in this process

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use netio::http::{Request, Response};
use netio::tls::{self, ClientConfig, Conn, Identity, Revocation, ServerConfig, Staples, Tickets, TlsError, Trust};
use netio::x509::ocsp::{self, CertId, IdHash, ResponseBuilder, Status};
use netio::x509::{Cert, CertBuilder, Csr, CsrBuilder, IpAddr, PrivateKey};
use netio::x509 as pkix;
use plat::net::SocketAddr;

use crate::tlsweb::{key, pki, pump, Kind, Pki};
use crate::T;

const OID_AIA: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 1, 1];
const OID_TLS_FEATURE: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 1, 24];

pub fn run(t: &mut T) {
    let p = pki(70, Kind::P256);
    extensions(t, &p);
    responses(t, &p);
    delegated(t, &p);
    times(t, &p);
    malformed(t, &p);
    csr(t, &p);
    stapling(t, &p);
    records(t, &p);
    fetch(t, &p);
}

fn now() -> i64 {
    plat::unix_time_ns().div_euclid(1_000_000_000)
}

/// a leaf of the test CA: (certificate, DER, key)
fn leaf(p: &Pki, serial: u8, edit: impl FnOnce(&mut CertBuilder)) -> (Cert, Vec<u8>, PrivateKey) {
    let k = key(Kind::P256, serial);
    let mut b = CertBuilder {
        serial: [serial; 16],
        cn: String::from("localhost"),
        not_before: now() - 3600,
        not_after: now() + 90 * 86_400,
        server_auth: true,
        dns: alloc::vec![String::from("localhost")],
        spki: k.spki(),
        ..CertBuilder::default()
    };
    edit(&mut b);
    let der = b.sign_key(Some(&p.ca), &p.ca_key, &[]).unwrap_or_default();
    (Cert::parse(&der).unwrap_or_default(), der, k)
}

/// a certificate the CA issued for OCSP (id-kp-OCSPSigning), or one without that purpose
fn responder(serial: u8, signing: bool, not_after: i64, issuer: (&Cert, &PrivateKey)) -> (Cert, PrivateKey) {
    let k = key(Kind::P256, serial);
    let b = CertBuilder {
        serial: [serial; 16],
        cn: String::from("OCSP responder"),
        not_before: now() - 3600,
        not_after,
        ocsp_signing: signing,
        server_auth: !signing,
        spki: k.spki(),
        ..CertBuilder::default()
    };
    (Cert::parse(&b.sign_key(Some(issuer.0), issuer.1, &[]).unwrap_or_default()).unwrap_or_default(), k)
}

fn good(p: &Pki, cert: &Cert) -> Vec<u8> {
    ResponseBuilder::good(now()).sign(cert, &p.ca, None, &p.ca_key).unwrap_or_default()
}

fn extensions(t: &mut T, p: &Pki) {
    let (c, _, k) = leaf(p, 1, |b| {
        b.ocsp_urls = alloc::vec![String::from("http://ocsp.example.test/")];
        b.must_staple = true;
    });
    t.check("x509: the authority information access names the OCSP responder", c.ocsp_urls == ["http://ocsp.example.test/"]);
    t.check("x509: the TLS feature status_request makes it must-staple (RFC 7633)", c.must_staple() && c.tls_features == [5]);
    t.check("x509: the authority key identifier is the CA's subject key identifier", !c.aki.is_empty() && c.aki == p.ca.ski);
    t.check("x509: spki() is the key's SubjectPublicKeyInfo", c.spki() == k.spki());
    let (plain, _, _) = leaf(p, 2, |_| {});
    t.check("x509: a certificate without them names no responder and needs no staple", plain.ocsp_urls.is_empty() && !plain.must_staple());
    // caIssuers, a URI-less access location, a critical AIA, a malformed TLS feature
    let ad = |m: u64, loc: &[u8]| pkix::der::seq(&[&pkix::der::oid(&[1, 3, 6, 1, 5, 5, 7, 48, m]), loc]);
    let aia = pkix::der::seq(&[&ad(2, &pkix::der::tlv(0x86, b"http://ca.example.test/ca.der")), &ad(1, &pkix::der::tlv(0x82, b"dns.example.test"))]);
    let with = |exts: &[Vec<u8>]| {
        let kk = key(Kind::P256, 3);
        let b = CertBuilder { serial: [3; 16], cn: String::from("x"), not_before: now() - 60, not_after: now() + 86_400, spki: kk.spki(), ..CertBuilder::default() };
        b.sign_key(Some(&p.ca), &p.ca_key, exts).and_then(|d| Cert::parse(&d))
    };
    let c = with(&[pkix::extension(OID_AIA, false, &aia)]);
    t.check("x509: caIssuers is read, a location of another form left aside", c.as_ref().is_some_and(|c| c.ca_issuers == ["http://ca.example.test/ca.der"] && c.ocsp_urls.is_empty()));
    t.check("x509: a critical authority information access is refused (RFC 5280 4.2.2.1)", with(&[pkix::extension(OID_AIA, true, &aia)]).is_none());
    t.check("x509: an empty authority information access is refused", with(&[pkix::extension(OID_AIA, false, &pkix::der::seq(&[]))]).is_none());
    t.check("x509: a TLS feature that is not a list of integers is refused", with(&[pkix::extension(OID_TLS_FEATURE, false, &pkix::der::seq(&[&pkix::der::utf8("5")]))]).is_none());
    t.check("x509: a TLS feature over 65535 is refused", with(&[pkix::extension(OID_TLS_FEATURE, false, &pkix::der::seq(&[&pkix::der::small_int(70_000)]))]).is_none());
    // bytes after the value's SEQUENCE; an access location that is no GeneralName (NULL)
    let five = pkix::der::seq(&[&pkix::der::small_int(5)]);
    t.check("x509: a TLS feature with bytes after its list is refused", with(&[pkix::extension(OID_TLS_FEATURE, false, &[five.clone(), five.clone()].concat())]).is_none() && with(&[pkix::extension(OID_TLS_FEATURE, false, &five)]).is_some_and(|c| c.must_staple()));
    t.check("x509: an authority information access with bytes after its list is refused", with(&[pkix::extension(OID_AIA, false, &[aia.clone(), pkix::der::seq(&[])].concat())]).is_none());
    let null_loc = pkix::der::seq(&[&ad(1, &[pkix::der::NULL, 0])]);
    t.check("x509: an access location that is no GeneralName is refused", with(&[pkix::extension(OID_AIA, false, &null_loc)]).is_none());
    let (r, _) = responder(4, true, now() + 86_400, (&p.ca, &p.ca_key));
    t.check("x509: a responder certificate signs OCSP and carries ocsp-nocheck", r.ocsp_signer() && r.ocsp_nocheck);
}

fn responses(t: &mut T, p: &Pki) {
    let (c, _, _) = leaf(p, 10, |_| {});
    let n = now();
    let r = ocsp::check(&good(p, &c), &c, &p.ca, n);
    t.check("ocsp: a good response signed by the issuer", r.as_ref().is_ok_and(|r| r.status == Status::Good && r.next_update == Some(r.this_update + 4 * 86_400)));
    let build = |b: ResponseBuilder| b.sign(&c, &p.ca, None, &p.ca_key).unwrap_or_default();
    let revoked = build(ResponseBuilder { status: Status::Revoked { time: n - 7200, reason: Some(1) }, ..ResponseBuilder::good(n) });
    t.check("ocsp: revoked, with its time and reason", ocsp::check(&revoked, &c, &p.ca, n).is_ok_and(|r| r.status == Status::Revoked { time: n - 7200, reason: Some(1) }));
    let unknown = build(ResponseBuilder { status: Status::Unknown, ..ResponseBuilder::good(n) });
    t.check("ocsp: unknown", ocsp::check(&unknown, &c, &p.ca, n).is_ok_and(|r| r.status == Status::Unknown));
    t.check("ocsp: the responder named by its name (byName)", ocsp::check(&build(ResponseBuilder { by_key: false, ..ResponseBuilder::good(n) }), &c, &p.ca, n).is_ok());
    t.check("ocsp: a CertID of SHA-256", ocsp::check(&build(ResponseBuilder { hash: IdHash::Sha256, ..ResponseBuilder::good(n) }), &c, &p.ca, n).is_ok());
    let nonce = build(ResponseBuilder { nonce: Some(alloc::vec![9; 16]), ..ResponseBuilder::good(n) });
    t.check("ocsp: the nonce of the response is read", ocsp::check(&nonce, &c, &p.ca, n).is_ok_and(|r| r.nonce.as_deref() == Some(&[9u8; 16][..])));
    // a request, and what a responder reads of it
    let id = CertId::new(&c, &p.ca);
    let req = id.as_ref().map(|i| ocsp::request(core::slice::from_ref(i), Some(&[5; 20])));
    let back = req.as_ref().and_then(|r| ocsp::Request::parse(r));
    t.check("ocsp: a request is read back (its CertID and nonce)", back.is_some_and(|b| Some(&b.ids[0]) == id.as_ref() && b.nonce.as_deref() == Some(&[5u8; 20][..])));
    t.check("ocsp: CertId needs the certificate's own issuer", CertId::new(&c, &c).is_none());
    // not about this certificate, not of this issuer, not signed by it
    let (other, _, _) = leaf(p, 11, |_| {});
    t.check("ocsp: a response about another certificate is refused", ocsp::check(&good(p, &other), &c, &p.ca, n).is_err());
    let q = pki(71, Kind::P256);
    t.check("ocsp: the response checked against another CA is refused", ocsp::check(&good(p, &c), &c, &q.ca, n).is_err());
    let forged = ResponseBuilder { by_key: false, ..ResponseBuilder::good(n) }.sign(&c, &p.ca, None, &q.ca_key).unwrap_or_default();
    t.check("ocsp: a response in the issuer's name with another key is refused", ocsp::check(&forged, &c, &p.ca, n).is_err());
    let mut flipped = good(p, &c);
    let l = flipped.len();
    flipped[l - 10] ^= 1;
    t.check("ocsp: a changed signature is refused", ocsp::check(&flipped, &c, &p.ca, n).is_err());
    let busy = ocsp::check(&ocsp::error_response(ocsp::TRY_LATER), &c, &p.ca, n);
    t.check("ocsp: an unsuccessful response (tryLater) says so", busy.is_err_and(|e| e.contains("tryLater")));
    // extensions: an unknown non-critical one is ignored, a critical one refuses the response
    let ext = |critical: bool| build(ResponseBuilder { extensions: alloc::vec![pkix::extension(&[1, 3, 6, 1, 4, 1, 99, 1], critical, &pkix::der::boolean(true))], ..ResponseBuilder::good(n) });
    t.check("ocsp: an unknown non-critical response extension is ignored", ocsp::check(&ext(false), &c, &p.ca, n).is_ok());
    t.check("ocsp: an unknown critical response extension refuses it", ocsp::check(&ext(true), &c, &p.ca, n).is_err());
}

fn delegated(t: &mut T, p: &Pki) {
    let (c, _, _) = leaf(p, 20, |_| {});
    let n = now();
    let by = |r: &(Cert, PrivateKey)| ResponseBuilder::good(n).sign(&c, &p.ca, Some(&r.0), &r.1).unwrap_or_default();
    let ok = responder(21, true, n + 30 * 86_400, (&p.ca, &p.ca_key));
    t.check("ocsp: a responder the issuer delegated to (id-kp-OCSPSigning)", ocsp::check(&by(&ok), &c, &p.ca, n).is_ok());
    let plain = responder(22, false, n + 30 * 86_400, (&p.ca, &p.ca_key));
    t.check("ocsp: a certificate of the issuer without id-kp-OCSPSigning may not answer", ocsp::check(&by(&plain), &c, &p.ca, n).is_err());
    let q = pki(72, Kind::P256);
    let foreign = responder(23, true, n + 30 * 86_400, (&q.ca, &q.ca_key));
    t.check("ocsp: a responder of another CA may not answer", ocsp::check(&by(&foreign), &c, &p.ca, n).is_err());
    let expired = responder(24, true, n - 60, (&p.ca, &p.ca_key));
    t.check("ocsp: an expired responder may not answer", ocsp::check(&by(&expired), &c, &p.ca, n).is_err());
    // a responder whose key usage lacks digitalSignature (a CA's: keyCertSign, cRLSign)
    let kr = key(Kind::P256, 25);
    let eku = pkix::extension(&[2, 5, 29, 37], false, &pkix::der::seq(&[&pkix::der::oid(&[1, 3, 6, 1, 5, 5, 7, 3, 9])]));
    let b = CertBuilder { serial: [25; 16], cn: String::from("OCSP responder"), not_before: n - 3600, not_after: n + 30 * 86_400, is_ca: true, spki: kr.spki(), ..CertBuilder::default() };
    let ku = (Cert::parse(&b.sign_key(Some(&p.ca), &p.ca_key, &[eku]).unwrap_or_default()).unwrap_or_default(), kr);
    t.check("ocsp: a responder without digitalSignature in its key usage may not answer", ku.0.ocsp_signer() && ocsp::check(&by(&ku), &c, &p.ca, n).is_err());
    // a responder with its CA's name: byName names both, the one whose key verifies counts
    let kn = key(Kind::P256, 26);
    let b = CertBuilder { serial: [26; 16], cn: p.ca.subject_cn.clone(), org: p.ca.subject_org.clone(), not_before: n - 3600, not_after: n + 30 * 86_400, ocsp_signing: true, spki: kn.spki(), ..CertBuilder::default() };
    let twin = (Cert::parse(&b.sign_key(Some(&p.ca), &p.ca_key, &[]).unwrap_or_default()).unwrap_or_default(), kn);
    let by_name = ResponseBuilder { by_key: false, ..ResponseBuilder::good(n) }.sign(&c, &p.ca, Some(&twin.0), &twin.1).unwrap_or_default();
    t.check("ocsp: a responder named like its CA (byName) is found", twin.0.subject == p.ca.subject && ocsp::check(&by_name, &c, &p.ca, n).is_ok());
    // a response counts no longer than its delegated responder's certificate
    let soon = responder(27, true, n + 600, (&p.ca, &p.ca_key));
    let r = ocsp::check(&by(&soon), &c, &p.ca, n);
    t.check("ocsp: a response expires with its delegated responder", r.is_ok_and(|r| r.responder_until == Some(soon.0.not_after) && r.expires() == soon.0.not_after && r.refresh_at() <= n));
    t.check("ocsp: ... one signed by the issuer has no such end", ocsp::check(&good(p, &c), &c, &p.ca, n).is_ok_and(|r| r.responder_until.is_none()));
    // a thisUpdate ahead within the clock skew, a responder that ends before it
    let brief = responder(28, true, n + 60, (&p.ca, &p.ca_key));
    let ahead = ResponseBuilder { this_update: n + 120, ..ResponseBuilder::good(n) }.sign(&c, &p.ca, Some(&brief.0), &brief.1).unwrap_or_default();
    let r = ocsp::check(&ahead, &c, &p.ca, n);
    t.check("ocsp: refresh_at is never after the end (a thisUpdate ahead, a responder about to end)", r.is_ok_and(|r| r.expires() == brief.0.not_after && r.refresh_at() <= r.expires()));
    // the next fetch: a minute away at the earliest, before the end when that comes first
    let r = ocsp::check(&by(&soon), &c, &p.ca, n);
    t.check("ocsp: next_fetch: a minute away when it is due now", r.as_ref().is_ok_and(|r| r.next_fetch(n) == n + 60));
    let r = ocsp::check(&by(&brief), &c, &p.ca, n);
    t.check("ocsp: ... before the end when that is under a minute away (10 s from now at the earliest)", r.as_ref().is_ok_and(|r| r.next_fetch(n) == brief.0.not_after - 10 && r.next_fetch(brief.0.not_after) == brief.0.not_after + 10));
    let r = ocsp::check(&good(p, &c), &c, &p.ca, n);
    t.check("ocsp: ... refresh_at when that is later", r.is_ok_and(|r| r.next_fetch(n) == r.refresh_at()));
    t.check("ocsp: ResponseBuilder::sign refuses a key that is not the signer's", ResponseBuilder::good(n).sign(&c, &p.ca, None, &ok.1).is_none() && ResponseBuilder::good(n).sign(&c, &p.ca, Some(&ok.0), &p.ca_key).is_none());
}

fn times(t: &mut T, p: &Pki) {
    let (c, _, _) = leaf(p, 30, |_| {});
    let n = now();
    let at = |this: i64, next: Option<i64>| ResponseBuilder { this_update: this, next_update: next, ..ResponseBuilder::good(n) }.sign(&c, &p.ca, None, &p.ca_key).unwrap_or_default();
    let ok = |b: &[u8]| ocsp::check(b, &c, &p.ca, n).is_ok();
    t.check("ocsp: thisUpdate an hour ahead is not valid yet", !ok(&at(n + 3600, Some(n + 86_400))));
    t.check("ocsp: ... two minutes ahead is clock skew", ok(&at(n + 120, Some(n + 86_400))));
    t.check("ocsp: nextUpdate an hour ago is out of date", !ok(&at(n - 86_400, Some(n - 3600))));
    t.check("ocsp: ... two minutes ago is clock skew", ok(&at(n - 86_400, Some(n - 120))));
    t.check("ocsp: without nextUpdate current for a day", ok(&at(n - 3600, None)) && !ok(&at(n - 2 * 86_400, None)));
    // (both times current within the clock skew: only their order is wrong)
    t.check("ocsp: nextUpdate before thisUpdate is refused", !ok(&at(n - 60, Some(n - 120))));
    let r = ocsp::check(&at(n - 3600, Some(n + 4 * 86_400 - 3600)), &c, &p.ca, n);
    t.check("ocsp: refresh_at is halfway, expires is nextUpdate", r.is_ok_and(|r| r.refresh_at() == n - 3600 + 2 * 86_400 && r.expires() == n + 4 * 86_400 - 3600));
}

fn malformed(t: &mut T, p: &Pki) {
    let (c, _, _) = leaf(p, 40, |_| {});
    let n = now();
    let g = good(p, &c);
    t.check("ocsp: a cut response is refused", (1..g.len()).step_by(7).all(|i| ocsp::check(&g[..i], &c, &p.ca, n).is_err()));
    let mut more = g.clone();
    more.push(0);
    t.check("ocsp: bytes after the response are refused", ocsp::check(&more, &c, &p.ca, n).is_err());
    // every byte changed in turn: nothing but an error (or, for bytes outside what is signed,
    // the same answer), no panic
    let mut same = true;
    for i in 0..g.len() {
        let mut b = g.clone();
        b[i] ^= 0x41;
        if let Ok(r) = ocsp::check(&b, &c, &p.ca, n) {
            same &= r.status == Status::Good;
        }
    }
    t.check("ocsp: a changed byte never makes another status", same);
    t.check("ocsp: a request with a requestor name (signed) is not read", ocsp::Request::parse(&pkix::der::seq(&[&pkix::der::seq(&[&pkix::der::explicit(1, &[]), &pkix::der::seq(&[])])])).is_none());
    let single_ext = |critical: bool| {
        let id = CertId::new(&c, &p.ca).map(|i| i.encode()).unwrap_or_default();
        let e = pkix::extension(&[1, 3, 6, 1, 4, 1, 99999, 1], critical, &pkix::der::octets(&[]));
        let one = pkix::der::seq(&[&id, &pkix::der::explicit(0, &pkix::der::seq(&[&e]))]);
        pkix::der::seq(&[&pkix::der::seq(&[&pkix::der::seq(&[&one])])])
    };
    t.check("ocsp: a request with an unknown critical singleRequestExtension is not read", ocsp::Request::parse(&single_ext(true)).is_none());
    t.check("ocsp: ... a non-critical one is skipped", ocsp::Request::parse(&single_ext(false)).is_some_and(|r| r.ids.len() == 1));
}

fn csr(t: &mut T, p: &Pki) {
    for kind in [Kind::P256, Kind::P384, Kind::Rsa, Kind::Ed25519] {
        let k = key(kind, 50);
        let der = CsrBuilder::for_names(&["a.example.test", "b.example.test"]).sign(&k);
        let r = der.as_deref().and_then(Csr::parse);
        t.check(&format!("csr: {:?}: built and read back (names, key, common name)", kind),
                r.is_some_and(|r| r.dns == ["a.example.test", "b.example.test"] && r.key == k.public_key() && r.cn == "a.example.test" && r.spki == k.spki()));
    }
    let k = key(Kind::P256, 51);
    let b = CsrBuilder { ips: alloc::vec![IpAddr::LOCALHOST_V4, IpAddr::parse("2001:db8::1").unwrap_or(IpAddr::LOCALHOST_V4)], must_staple: true, ..CsrBuilder::default() };
    let r = b.sign(&k).as_deref().and_then(Csr::parse);
    t.check("csr: addresses and must-staple, no common name", r.is_some_and(|r| r.ips.len() == 2 && r.tls_features == [5] && r.cn.is_empty() && r.dns.is_empty()));
    let none = CsrBuilder::default().sign(&k).as_deref().and_then(Csr::parse);
    t.check("csr: one without names or attributes", none.is_some_and(|r| r.dns.is_empty() && r.ips.is_empty()));
    t.check("csr: a name not in the preferred syntax is not put in", CsrBuilder::for_names(&["not a name"]).sign(&k).is_none());
    let short = crypto::rsa::PrivateKey::generate(1024, &mut crate::tlsweb::xorshift(52)).and_then(PrivateKey::rsa);
    t.check("csr: an RSA key under 2048 bits signs none (none would be read)", short.is_some_and(|s| CsrBuilder::for_names(&["a.example.test"]).sign(&s).is_none()));
    // requests with extensions of our choosing, signed with k (ECDSA P-256 with SHA-256)
    let custom = |exts: &[Vec<u8>]| {
        use pkix::der;
        let parts: Vec<&[u8]> = exts.iter().map(|v| v.as_slice()).collect();
        let attrs = der::explicit(0, &der::seq(&[&der::oid(&[1, 2, 840, 113549, 1, 9, 14]), &der::set(&[&der::seq(&parts)])]));
        let info = der::seq(&[&der::small_int(0), &der::seq(&[]), &k.spki(), &attrs]);
        let sig = k.sign(k.cert_alg(), &info, &[]).unwrap_or_default();
        Csr::parse(&der::seq(&[&info, &der::seq(&[&der::oid(&[1, 2, 840, 10045, 4, 3, 2])]), &der::bits(&sig)]))
    };
    let san = |n: &[u8]| pkix::extension(&[2, 5, 29, 17], false, &pkix::der::seq(&[&pkix::der::tlv(0x82, n)]));
    let five = pkix::der::seq(&[&pkix::der::small_int(5)]);
    let feature = |v: &[u8]| pkix::extension(OID_TLS_FEATURE, false, v);
    t.check("csr: a name and a TLS feature are read (the requests below are well signed)", custom(&[san(b"a.example.test"), feature(&five)]).is_some_and(|r| r.dns == ["a.example.test"] && r.tls_features == [5]));
    t.check("csr: two subject alternative names are refused", custom(&[san(b"a.example.test"), san(b"b.example.test")]).is_none());
    t.check("csr: an empty subject alternative name is refused", custom(&[pkix::extension(&[2, 5, 29, 17], false, &pkix::der::seq(&[]))]).is_none());
    t.check("csr: a TLS feature with bytes after its list is refused", custom(&[feature(&[five.clone(), five.clone()].concat())]).is_none());
    let der = CsrBuilder::for_names(&["a.example.test"]).sign(&k).unwrap_or_default();
    let mut sig = der.clone();
    let l = sig.len();
    sig[l - 5] ^= 1;
    t.check("csr: a changed signature is refused", Csr::parse(&sig).is_none());
    let at = der.windows(14).position(|w| w == b"a.example.test").unwrap_or(0);
    let mut name = der.clone();
    name[at] = b'c';
    t.check("csr: a changed name is refused (the signature)", at > 0 && Csr::parse(&name).is_none());
    // a private CA signs what the request asks
    let r = Csr::parse(&der).unwrap_or_default();
    let cert = CertBuilder { serial: [52; 16], cn: r.cn.clone(), not_before: now(), not_after: now() + 86_400, dns: r.dns.clone(), server_auth: true, spki: r.spki.clone(), ..CertBuilder::default() }
        .sign_key(Some(&p.ca), &p.ca_key, &[])
        .and_then(|d| Cert::parse(&d));
    t.check("csr: a CA's certificate from it names it and holds its key", cert.is_some_and(|c| c.names("a.example.test") && c.key == k.public_key()));
}

fn ccfg(p: &Pki, rev: Revocation) -> ClientConfig {
    ClientConfig { revocation: rev, ..ClientConfig::new(Trust::Anchors(alloc::vec![p.ca.clone()]), "localhost") }
}

fn scfg(id: &Identity, staples: Option<&Arc<Staples>>) -> ServerConfig {
    ServerConfig { ocsp: staples.cloned(), ..ServerConfig::new(id.clone()) }
}

fn go(c: ClientConfig, s: ServerConfig) -> Result<Conn, TlsError> {
    let mut c = Conn::client(c);
    let mut s = Conn::server(s);
    pump(&mut c, &mut s)?;
    Ok(c)
}

fn alert(r: &Result<Conn, TlsError>) -> Option<u8> {
    match r {
        Err(TlsError::Alert(a, _)) => Some(*a),
        _ => None,
    }
}

fn stapling(t: &mut T, p: &Pki) {
    use netio::tls::conn::{AL_BAD_CERTIFICATE_STATUS_RESPONSE as BAD_STATUS, AL_CERTIFICATE_REVOKED, AL_DECODE_ERROR, AL_UNSUPPORTED_EXTENSION};
    let (c, der, k) = leaf(p, 60, |_| {});
    let id = Identity::new(alloc::vec![der.clone()], k);
    let st = Arc::new(Staples::new());
    let g = good(p, &c);
    let set = st.set(&id.chain, Some(&p.ca), g.clone());
    t.check("stapling: the server keeps a good response for its certificate", set.is_ok_and(|r| r.status == Status::Good) && st.len() == 1);
    t.check("stapling: without the issuer (a chain of the leaf alone) it cannot be checked", st.set(&id.chain, None, g.clone()).is_err());
    let (other, _, _) = leaf(p, 61, |_| {});
    t.check("stapling: an issuer given that did not sign the leaf is refused", st.set(&id.chain, Some(&other), g.clone()).is_err_and(|e| e.contains("did not sign")));
    t.check("stapling: a response about another certificate is not kept", st.set(&id.chain, Some(&p.ca), good(p, &other)).is_err());
    t.check("stapling: one too long is not kept", st.set(&id.chain, Some(&p.ca), alloc::vec![0; tls::staple::MAX_STAPLE + 1]).is_err());
    let r = go(ccfg(p, Revocation::Stapled), scfg(&id, Some(&st)));
    t.check("stapling: the client asks, the server staples, the client checks it", r.as_ref().is_ok_and(|c| c.ocsp_response() == Some(&g[..])));
    let r = go(ccfg(p, Revocation::Required), scfg(&id, Some(&st)));
    t.check("stapling: Revocation::Required with a good one", r.is_ok());
    let r = go(ccfg(p, Revocation::Off), scfg(&id, Some(&st)));
    t.check("stapling: Revocation::Off asks for none and gets none", r.as_ref().is_ok_and(|c| c.ocsp_response().is_none()));
    let pinned = ClientConfig::new(Trust::Pinned(alloc::vec![crypto::sha256(&der)]), "localhost");
    t.check("stapling: a pinned certificate asks for none", go(pinned, scfg(&id, Some(&st))).as_ref().is_ok_and(|c| c.ocsp_response().is_none()));
    let none = go(ccfg(p, Revocation::Stapled), scfg(&id, None));
    t.check("stapling: a server without staples is taken (Stapled)", none.as_ref().is_ok_and(|c| c.ocsp_response().is_none()));
    t.check("stapling: ... and refused with Required (bad_certificate_status_response)", alert(&go(ccfg(p, Revocation::Required), scfg(&id, None))) == Some(BAD_STATUS));
    // revoked: refused, whatever the mode
    let rv = Arc::new(Staples::new());
    let revoked = ResponseBuilder { status: Status::Revoked { time: now() - 60, reason: None }, ..ResponseBuilder::good(now()) }.sign(&c, &p.ca, None, &p.ca_key).unwrap_or_default();
    let _ = rv.set(&id.chain, Some(&p.ca), revoked);
    t.check("stapling: a revoked certificate is refused (certificate_revoked)", alert(&go(ccfg(p, Revocation::Stapled), scfg(&id, Some(&rv)))) == Some(AL_CERTIFICATE_REVOKED));
    t.check("stapling: ... unless revocation is off", go(ccfg(p, Revocation::Off), scfg(&id, Some(&rv))).is_ok());
    // unknown: taken softly, refused when required
    let un = Arc::new(Staples::new());
    let _ = un.set(&id.chain, Some(&p.ca), ResponseBuilder { status: Status::Unknown, ..ResponseBuilder::good(now()) }.sign(&c, &p.ca, None, &p.ca_key).unwrap_or_default());
    t.check("stapling: unknown is taken (Stapled), refused (Required)", go(ccfg(p, Revocation::Stapled), scfg(&id, Some(&un))).is_ok() && alert(&go(ccfg(p, Revocation::Required), scfg(&id, Some(&un)))) == Some(BAD_STATUS));
    // responses that are not usable: another certificate's, out of date, not OCSP at all
    let unusable = [("another certificate's", good(p, &other)),
                    ("out of date", ResponseBuilder { this_update: now() - 5 * 86_400, next_update: Some(now() - 86_400), ..ResponseBuilder::good(now()) }.sign(&c, &p.ca, None, &p.ca_key).unwrap_or_default()),
                    ("not OCSP", alloc::vec![0x30, 3, 1, 1, 0])];
    for (what, bytes) in unusable {
        let bad = Arc::new(Staples::new());
        bad.test_insert(&der, bytes);
        t.check(&format!("stapling: a response {} counts as none: taken (Stapled), refused (Required)", what),
                go(ccfg(p, Revocation::Stapled), scfg(&id, Some(&bad))).is_ok() && alert(&go(ccfg(p, Revocation::Required), scfg(&id, Some(&bad)))) == Some(BAD_STATUS));
    }
    let empty = Arc::new(Staples::new());
    empty.test_insert(&der, Vec::new());
    t.check("stapling: an empty response in the certificate entry is decode_error", alert(&go(ccfg(p, Revocation::Stapled), scfg(&id, Some(&empty)))) == Some(AL_DECODE_ERROR));
    // must-staple (RFC 7633)
    let (mc, mder, mk) = leaf(p, 62, |b| b.must_staple = true);
    let mid = Identity::new(alloc::vec![mder], mk);
    let ms = Arc::new(Staples::new());
    t.check("stapling: a must-staple certificate without a staple is refused", alert(&go(ccfg(p, Revocation::Stapled), scfg(&mid, None))) == Some(BAD_STATUS));
    let _ = ms.set(&mid.chain, Some(&p.ca), good(p, &mc));
    t.check("stapling: ... with one it is taken", go(ccfg(p, Revocation::Stapled), scfg(&mid, Some(&ms))).is_ok());
    t.check("stapling: ... Revocation::Off does not look", go(ccfg(p, Revocation::Off), scfg(&mid, None)).is_ok());
    // a staple not asked for: unsupported_extension (RFC 8446 4.2, 4.4.2)
    let mut cl = Conn::client(ccfg(p, Revocation::Off));
    let mut sv = Conn::server(scfg(&id, Some(&st)));
    sv.test_force_status();
    let r = pump(&mut cl, &mut sv);
    t.check("stapling: a response the client did not ask for is unsupported_extension", matches!(r, Err(TlsError::Alert(AL_UNSUPPORTED_EXTENSION, _))));
    // the server drops a response past its nextUpdate
    let old = Arc::new(Staples::new());
    let _ = old.set(&id.chain, Some(&p.ca), g.clone());
    old.test_shift(5 * 86_400);
    t.check("stapling: a response out of date is not sent (and dropped)", alert(&go(ccfg(p, Revocation::Required), scfg(&id, Some(&old)))) == Some(BAD_STATUS) && old.is_empty());
    // after a HelloRetryRequest; a resumption has no certificate and no staple
    let mut cl = Conn::client(ccfg(p, Revocation::Required));
    let mut sv = Conn::server(scfg(&id, Some(&st)));
    sv.test_server_prefs(Vec::new(), alloc::vec![tls::kx::SECP256R1]);
    let r = pump(&mut cl, &mut sv);
    t.check("stapling: after a HelloRetryRequest", r.is_ok() && cl.ocsp_response() == Some(&g[..]));
    let ss = Arc::new(tls::Sessions::default());
    let tk = Arc::new(Tickets::new());
    let resume = |ss: &Arc<tls::Sessions>| {
        let c = ClientConfig { sessions: Some(ss.clone()), ..ccfg(p, Revocation::Required) };
        let s = ServerConfig { tickets: Some(tk.clone()), ..scfg(&id, Some(&st)) };
        let mut c = Conn::client(c);
        let mut s = Conn::server(s);
        pump(&mut c, &mut s).map(|_| c)
    };
    let first = resume(&ss);
    let second = resume(&ss);
    t.check("stapling: a resumption (Required) needs none: its first handshake had one", first.as_ref().is_ok_and(|c| c.ocsp_response().is_some()) && second.as_ref().is_ok_and(|c| c.resumed() && c.ocsp_response().is_none()));
    let with = |ss: &Arc<tls::Sessions>, rev: Revocation, staples: Option<&Arc<Staples>>| {
        let c = ClientConfig { sessions: Some(ss.clone()), ..ccfg(p, rev) };
        let s = ServerConfig { tickets: Some(tk.clone()), ..scfg(&id, staples) };
        go(c, s)
    };
    // a session of Revocation::Off is never offered by Required: it checks in full (and stays)
    let ss = Arc::new(tls::Sessions::default());
    let off = with(&ss, Revocation::Off, None);
    let n = ss.len();
    let req = with(&ss, Revocation::Required, None);
    t.check("stapling: Required does not resume a session of Revocation::Off (no staple: refused)", off.is_ok() && n > 0 && alert(&req) == Some(BAD_STATUS) && ss.len() == n);
    // a session goes when the staple its handshake checked does (here after an hour)
    let short = ResponseBuilder { next_update: Some(now() + 3600), ..ResponseBuilder::good(now()) }.sign(&c, &p.ca, None, &p.ca_key).unwrap_or_default();
    let sst = Arc::new(Staples::new());
    let _ = sst.set(&id.chain, Some(&p.ca), short);
    let ss = Arc::new(tls::Sessions::default());
    let a = with(&ss, Revocation::Stapled, Some(&sst));
    ss.test_shift(30 * 60 * 1000);
    let b = with(&ss, Revocation::Stapled, Some(&sst));
    ss.test_shift(2 * 3600 * 1000);
    let c2 = with(&ss, Revocation::Stapled, Some(&sst));
    t.check("stapling: a session is resumed while the staple its handshake checked is current", a.is_ok() && b.as_ref().is_ok_and(|c| c.resumed()));
    t.check("stapling: ... and not after it (a full handshake checks a new one)", c2.as_ref().is_ok_and(|c| !c.resumed() && c.ocsp_response().is_some()));
    // ... nor after the end of the delegated responder that signed it (no clock skew there:
    // a full handshake refuses the responder from then on)
    let rs = responder(63, true, now() + 600, (&p.ca, &p.ca_key));
    let by_rs = ResponseBuilder::good(now()).sign(&c, &p.ca, Some(&rs.0), &rs.1).unwrap_or_default();
    let srs = Arc::new(Staples::new());
    let _ = srs.set(&id.chain, Some(&p.ca), by_rs);
    let ss = Arc::new(tls::Sessions::default());
    let a = with(&ss, Revocation::Stapled, Some(&srs));
    ss.test_shift(660 * 1000);
    let b = with(&ss, Revocation::Stapled, Some(&srs));
    t.check("stapling: a session goes with the delegated responder of its staple", a.is_ok() && srs.len() == 1 && b.as_ref().is_ok_and(|c| !c.resumed()));
}

/// status_request in a ClientHello as it comes (RFC 6066 8), and where a client takes none
fn records(t: &mut T, p: &Pki) {
    use netio::tls::conn::{AL_DECODE_ERROR, AL_ILLEGAL_PARAMETER};
    let ch_rec = Conn::client(ccfg(p, Revocation::Off)).take_out();
    let ch = ch_rec[5..].to_vec();
    let with_ext = |x: &[u8]| {
        let sid = 4 + 2 + 32;
        let suites = sid + 1 + ch[sid] as usize;
        let comp = suites + 2 + ((ch[suites] as usize) << 8 | ch[suites + 1] as usize);
        let exts = comp + 1 + ch[comp] as usize;
        let mut m = ch.to_vec();
        m.extend_from_slice(x);
        let ext_len = m.len() - exts - 2;
        m[exts..exts + 2].copy_from_slice(&(ext_len as u16).to_be_bytes());
        let body = m.len() - 4;
        m[1..4].copy_from_slice(&(body as u32).to_be_bytes()[1..]);
        let mut r = alloc::vec![22, 3, 1, (m.len() >> 8) as u8, m.len() as u8];
        r.extend_from_slice(&m);
        r
    };
    let feed = |bytes: &[u8]| {
        let mut s = Conn::server(ServerConfig::new(p.server.clone()));
        match s.feed(bytes) {
            Ok(()) => Some(0),
            Err(TlsError::Alert(a, _)) => Some(a),
            Err(_) => None,
        }
    };
    t.check("records: status_request ocsp with empty lists is taken", feed(&with_ext(&[0, 5, 0, 5, 1, 0, 0, 0, 0])) == Some(0));
    t.check("records: ... with a responder id and request extensions", feed(&with_ext(&[0, 5, 0, 10, 1, 0, 3, 0, 1, 7, 0, 2, 0xaa, 0xbb])) == Some(0));
    t.check("records: a status_request of another type is ignored", feed(&with_ext(&[0, 5, 0, 3, 2, 0xff, 0xff])) == Some(0));
    t.check("records: a cut status_request is decode_error", feed(&with_ext(&[0, 5, 0, 3, 1, 0, 0])) == Some(AL_DECODE_ERROR));
    t.check("records: an empty responder id is decode_error", feed(&with_ext(&[0, 5, 0, 7, 1, 0, 2, 0, 0, 0, 0])) == Some(AL_DECODE_ERROR));
    t.check("records: bytes after the request extensions are decode_error", feed(&with_ext(&[0, 5, 0, 6, 1, 0, 0, 0, 0, 9])) == Some(AL_DECODE_ERROR));
    // a client: status_request belongs in the certificate entry, not in the ServerHello
    let mut c = Conn::client(ccfg(p, Revocation::Stapled));
    let mut s = Conn::server(ServerConfig::new(p.server.clone()));
    let _ = s.feed(&c.take_out());
    let r = c.feed(&crate::tlst::sh_with_ext(&s.take_out(), &[0, 5, 0, 0]));
    t.check("records: status_request in a TLS 1.3 ServerHello is illegal_parameter", matches!(r, Err(TlsError::Alert(AL_ILLEGAL_PARAMETER, _))));
}

/// a responder in this process (http, as CAs run them): answers for the certificates it knows
struct Responder {
    srv: netio::Server,
    hits: Arc<core::sync::atomic::AtomicUsize>,
}

fn responder_at(p: &Pki, mode: Arc<core::sync::atomic::AtomicU8>) -> Option<Responder> {
    use core::sync::atomic::Ordering;
    let (ca, ca_key) = (p.ca.clone(), p.ca_key.clone());
    let hits = Arc::new(core::sync::atomic::AtomicUsize::new(0));
    let h2 = hits.clone();
    let handler = move |r: &Request| -> Response {
        h2.fetch_add(1, Ordering::Relaxed);
        if r.method != "POST" || r.header("Content-Type") != Some("application/ocsp-request") {
            return Response::new(400);
        }
        let Some(req) = ocsp::Request::parse(&r.body) else { return Response::bytes(200, "application/ocsp-response", ocsp::error_response(ocsp::MALFORMED_REQUEST)) };
        match mode.load(Ordering::Relaxed) {
            1 => return Response::new(500),
            2 => return Response::bytes(200, "application/ocsp-response", ocsp::error_response(ocsp::TRY_LATER)),
            _ => {}
        }
        // a stand-in certificate with the serial asked for: the CertID names it
        let probe = Cert { serial: req.ids[0].serial.clone(), issuer: ca.subject.clone(), ..Cert::default() };
        let b = ResponseBuilder { nonce: req.nonce.clone(), ..ResponseBuilder::good(now()) };
        match b.sign(&probe, &ca, None, &ca_key) {
            Some(d) => Response::bytes(200, "application/ocsp-response", d),
            None => Response::new(500),
        }
    };
    let srv = netio::Server::start(&SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, 0), netio::ServerConfig { threads: 2, ..netio::ServerConfig::default() }, Arc::new(handler)).ok()?;
    Some(Responder { srv, hits })
}

fn fetch(t: &mut T, p: &Pki) {
    use core::sync::atomic::{AtomicU8, Ordering};
    let mode = Arc::new(AtomicU8::new(0));
    let Some(rs) = responder_at(p, mode.clone()) else {
        t.check("fetch: the responder starts", false);
        return;
    };
    let url = format!("http://127.0.0.1:{}/ocsp", rs.srv.local_addr().port);
    let (c, der, _) = leaf(p, 80, |b| b.ocsp_urls = alloc::vec![url.clone()]);
    let client = netio::Client::new();
    let r = netio::ocsp::fetch(&client, &c, &p.ca);
    t.check("fetch: the certificate's responder answers, the answer is checked", r.as_ref().is_ok_and(|(d, r)| r.status == Status::Good && !d.is_empty()));
    let (plain, _, _) = leaf(p, 81, |_| {});
    t.check("fetch: a certificate without a responder", netio::ocsp::fetch(&client, &plain, &p.ca).is_err());
    let st = Staples::new();
    let chain = alloc::vec![der.clone()];
    let first = netio::ocsp::refresh(&st, &client, &chain, Some(&p.ca));
    let hits = rs.hits.load(Ordering::Relaxed);
    t.check("refresh: fetched and kept; next time halfway (two days)", first.as_ref().is_ok_and(|at| (*at - now() - 2 * 86_400).abs() < 120) && st.get(&der).is_some());
    let again = netio::ocsp::refresh(&st, &client, &chain, Some(&p.ca));
    t.check("refresh: not due yet: no request", again.is_ok() && rs.hits.load(Ordering::Relaxed) == hits);
    st.test_shift(2 * 86_400 + 60);
    let due = netio::ocsp::refresh(&st, &client, &chain, Some(&p.ca));
    t.check("refresh: due: fetched again", due.is_ok() && rs.hits.load(Ordering::Relaxed) == hits + 1);
    mode.store(1, Ordering::Relaxed);
    st.test_shift(2 * 86_400);
    let failed = netio::ocsp::refresh(&st, &client, &chain, Some(&p.ca));
    t.check("refresh: the responder fails (HTTP 500): Err, the old response stays", failed.is_err() && st.response(&der).is_some());
    mode.store(2, Ordering::Relaxed);
    t.check("refresh: tryLater: Err", netio::ocsp::refresh(&st, &client, &chain, Some(&p.ca)).is_err_and(|e| e.contains("tryLater")));
    rs.srv.stop();
}

/// --ocsp-tool CMD ...: OCSP and certificate requests for tools/tls_interop.py, with the PKI of
/// `--tls-pki DIR --key KIND` (KIND p256 by default; not ed25519, which OpenSSL's OCSP lacks)
///   pki DIR URL          ca.key, good.der / revoked.der / unknown.der for server.pem, leaf.pem /
///                        leaf.key (a leaf whose OCSP responder is URL), csr.der (ours)
///   check RESP CERT CA   our check of a response -> "good" / "revoked" / "unknown" / "error: .."
///   fetch CERT CA        netio::ocsp::fetch from the certificate's responder -> the same
///   csr FILE             our reading of a request (DER) -> "dns=.. ips=.. features=.." / "refused"
///   req FILE             our reading of an OCSP request -> "ids=N nonce=yes|no" / "refused"
pub fn tool(args: &[String]) -> i32 {
    let kind = plat::arg_value("--key").and_then(|k| Kind::parse(&k)).unwrap_or(Kind::P256);
    let p = pki(60, kind);
    let at = args.iter().position(|a| a == "--ocsp-tool").map_or(args.len(), |i| i + 1);
    let a: Vec<&str> = args[at..].iter().map(|s| s.as_str()).collect();
    let pem_cert = |f: &str| plat::read_file(f).and_then(|b| netio::pem::decode(&String::from_utf8_lossy(&b), "CERTIFICATE")).and_then(|d| Cert::parse(&d));
    let say = |s: String| {
        plat::write_stdout(format!("{}\n", s).as_bytes());
        0
    };
    let status = |r: Result<ocsp::Response, String>| match r {
        Ok(r) => match r.status {
            Status::Good => String::from("good"),
            Status::Revoked { .. } => String::from("revoked"),
            Status::Unknown => String::from("unknown"),
        },
        Err(e) => format!("error: {}", e),
    };
    match a.as_slice() {
        ["pki", dir, url, ..] => {
            let w = |name: &str, b: &[u8]| plat::write_file(&plat::fs::join(dir, name), b);
            let server = Cert::parse(&p.server.chain[0]).unwrap_or_default();
            let n = now();
            let sign = |c: &Cert, st: Status| ResponseBuilder { status: st, ..ResponseBuilder::good(n) }.sign(c, &p.ca, None, &p.ca_key).unwrap_or_default();
            let (_, ldder, lk) = leaf(&p, 0x77, |b| b.ocsp_urls = alloc::vec![String::from(*url)]);
            let ok = w("ca.key", netio::pem::encode("PRIVATE KEY", &p.ca_key.to_pkcs8().unwrap_or_default()).as_bytes())
                && w("good.der", &sign(&server, Status::Good))
                && w("revoked.der", &sign(&server, Status::Revoked { time: n - 3600, reason: Some(1) }))
                && w("unknown.der", &sign(&server, Status::Unknown))
                && w("leaf.pem", netio::pem::encode("CERTIFICATE", &ldder).as_bytes())
                && w("leaf.key", netio::pem::encode("PRIVATE KEY", &lk.to_pkcs8().unwrap_or_default()).as_bytes())
                && w("csr.der", &CsrBuilder { ips: alloc::vec![IpAddr::LOCALHOST_V4], must_staple: true, ..CsrBuilder::for_names(&["a.example.test", "b.example.test"]) }.sign(&lk).unwrap_or_default());
            if ok { 0 } else { 1 }
        }
        ["check", resp, cert, ca, ..] => match (plat::read_file(resp), pem_cert(cert), pem_cert(ca)) {
            (Some(r), Some(c), Some(i)) => say(status(ocsp::check(&r, &c, &i, now()))),
            _ => 2,
        },
        ["fetch", cert, ca, ..] => match (pem_cert(cert), pem_cert(ca)) {
            (Some(c), Some(i)) => say(status(netio::ocsp::fetch(&netio::Client::new(), &c, &i).map(|(_, r)| r))),
            _ => 2,
        },
        ["csr", f, ..] => match plat::read_file(f).and_then(|b| Csr::parse(&b)) {
            Some(c) => say(format!("dns={} ips={} features={:?} cn={}", c.dns.join(","), c.ips.iter().map(|i| format!("{}", i)).collect::<Vec<_>>().join(","), c.tls_features, c.cn)),
            None => say(String::from("refused")),
        },
        ["req", f, ..] => match plat::read_file(f).and_then(|b| ocsp::Request::parse(&b)) {
            Some(r) => say(format!("ids={} nonce={}", r.ids.len(), if r.nonce.is_some() { "yes" } else { "no" })),
            None => say(String::from("refused")),
        },
        _ => 2,
    }
}
