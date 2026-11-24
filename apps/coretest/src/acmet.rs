// acmet -- netio::acme (RFC 8555) against the ACME server in this process (acmesrv): the
// issuance with http-01, tls-alpn-01 and dns-01 (wildcards), IP identifiers, the account keys
// (ES256, ES384, RS256, EdDSA), accounts (existing, onlyReturnExisting, terms, contact,
// deactivation), a nonce refused, invalid challenges, a chain for another key, rate limits,
// renewal information, revocation, key change, external account binding, profiles, the wait's
// limit; the JWK thumbprint of RFC 7638 and the time formats

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use netio::acme::{self, Acme, AcmeError, Challenge, Eab, Http01, Identifier, NewAccount, OrderOptions, Solver, Solvers, TlsAlpn01};
use netio::http::{Request, Response};
use netio::tls::{self, ClientConfig, ServerConfig, Trust};
use netio::x509::{Cert, PubKey};
use netio::Stream;
use plat::net::{SocketAddr, TcpStream};
use plat::sync::Mutex;

use crate::acmesrv::{self, ids, AcmeServer, Faults};
use crate::tlsweb::{key, pki, Kind, Pki};
use crate::T;

pub fn run(t: &mut T) {
    vectors(t);
    let p = pki(110, Kind::P256);
    let Some(rig) = Rig::start(&p, Faults::default()) else {
        t.check("acme: the servers start", false);
        return;
    };
    issuance(t, &p, &rig);
    account_keys(t, &p, &rig);
    accounts(t, &p, &rig);
    failures(t, &p, &rig);
    lifecycle(t, &p, &rig);
    eab(t, &p);
    faults(t, &p);
    rig.stop();
}

/// the ACME server, an HTTP server for http-01 (port 80's stand-in), a TLS server for
/// tls-alpn-01 (port 443's)
struct Rig {
    acme: AcmeServer,
    http: netio::Server,
    http01: Http01,
    tls: netio::Server,
    alpn: TlsAlpn01,
}

impl Rig {
    fn start(p: &Pki, faults: Faults) -> Option<Rig> {
        let acme = acmesrv::start(p, faults)?;
        let http01 = Http01::new();
        let fallback: netio::server::Handler = Arc::new(|_r: &Request| Response::text(404, "no"));
        let http = netio::Server::start(&SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, 0), netio::ServerConfig { threads: 2, ..netio::ServerConfig::default() }, http01.handler(fallback)).ok()?;
        let alpn = TlsAlpn01::new();
        let mut cfg = ServerConfig { alpn: alloc::vec![String::from("http/1.1")], ..ServerConfig::new(p.server.clone()) };
        alpn.setup(&mut cfg);
        let wrap: netio::server::Wrap = Arc::new(move |tcp: TcpStream| {
            let s = tls::accept(Box::new(tcp), cfg.clone()).ok()?;
            Some((Box::new(s) as Box<dyn Stream>, None))
        });
        let tls = netio::Server::start_with(&SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, 0), netio::ServerConfig { threads: 2, ..netio::ServerConfig::default() }, Arc::new(|_r: &Request| Response::text(200, "site")), Some(wrap)).ok()?;
        {
            let mut s = acme.state.lock();
            s.http_port = http.local_addr().port;
            s.tls_port = tls.local_addr().port;
        }
        Some(Rig { acme, http, http01, tls, alpn })
    }

    fn stop(self) {
        self.acme.srv.stop();
        self.http.stop();
        self.tls.stop();
    }
}

/// https to the ACME server: the test CA
fn client(p: &Pki) -> netio::Client {
    let ca = p.ca.clone();
    let connector: netio::client::Connector = Arc::new(move |url: &netio::Url, tcp: TcpStream| {
        let cfg = ClientConfig::new(Trust::Anchors(alloc::vec![ca.clone()]), &url.host);
        tls::connect(Box::new(tcp), cfg).map(|s| Box::new(s) as Box<dyn Stream>).map_err(|e| netio::HttpError::Tls(format!("{}", e)))
    });
    netio::Client::new().with_connector(connector)
}

fn agreed() -> NewAccount {
    NewAccount { terms_agreed: true, contact: alloc::vec![String::from("mailto:admin@example.test")], ..NewAccount::default() }
}

fn new_acme(p: &Pki, rig: &AcmeServer) -> Result<Acme, AcmeError> {
    let mut a = Acme::new(client(p), &rig.directory())?;
    a.poll_ms = 200;
    Ok(a)
}

/// dns-01 into the server's TXT table
struct Dns(Arc<Mutex<BTreeMap<String, Vec<String>>>>);

impl Solver for Dns {
    fn kinds(&self) -> Vec<&'static str> {
        alloc::vec!["dns-01"]
    }
    fn present(&mut self, id: &Identifier, _ch: &Challenge, key_auth: &str) -> Result<(), String> {
        self.0.lock().entry(acme::dns01_name(id)).or_default().push(acme::dns01_value(key_auth));
        Ok(())
    }
    fn cleanup(&mut self, id: &Identifier, _ch: &Challenge) {
        self.0.lock().remove(&acme::dns01_name(id));
    }
}

/// a solver that shows the wrong key authorization
/// what a group needs to go on: an Err counts as a failed check by name, so no group ends
/// without its checks unnoticed
fn need<X, E: core::fmt::Display>(t: &mut T, what: &str, r: Result<X, E>) -> Option<X> {
    match r {
        Ok(x) => Some(x),
        Err(e) => {
            t.check(&format!("acme: {}: {}", what, e), false);
            None
        }
    }
}

/// a test CA with these faults; None (and a failed check) when it does not start
fn start_rig(t: &mut T, p: &Pki, what: &str, faults: Faults) -> Option<Rig> {
    let r = Rig::start(p, faults);
    if r.is_none() {
        t.check(&format!("acme: the test CA starts ({})", what), false);
    }
    r
}

/// a solver that keeps the tokens of the challenges it set up
struct Seen<S>(S, Arc<Mutex<Vec<String>>>);

impl<S: Solver> Solver for Seen<S> {
    fn kinds(&self) -> Vec<&'static str> {
        self.0.kinds()
    }
    fn present(&mut self, id: &Identifier, ch: &Challenge, key_auth: &str) -> Result<(), String> {
        self.1.lock().push(ch.token.clone());
        self.0.present(id, ch, key_auth)
    }
    fn cleanup(&mut self, id: &Identifier, ch: &Challenge) {
        self.0.cleanup(id, ch)
    }
}

struct Liar(Http01);

impl Solver for Liar {
    fn kinds(&self) -> Vec<&'static str> {
        alloc::vec!["http-01"]
    }
    fn present(&mut self, id: &Identifier, ch: &Challenge, _key_auth: &str) -> Result<(), String> {
        self.0.present(id, ch, "not.it")
    }
    fn cleanup(&mut self, id: &Identifier, ch: &Challenge) {
        self.0.cleanup(id, ch)
    }
}

fn vectors(t: &mut T) {
    // RFC 7638 3.1: the RSA key of RFC 7517 A.1 and its thumbprint
    let n = crypto::b64::decode_url("0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw").unwrap_or_default();
    let k = PubKey::Rsa { n, e: alloc::vec![1, 0, 1] };
    t.check("acme: the JWK thumbprint of RFC 7638 3.1", acme::thumbprint(&k).as_deref() == Some("NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs"));
    // RFC 8037 A.3 (Ed25519); the EC key of RFC 7517 A.1 (its thumbprint from Python's json with
    // sorted keys)
    let ed = crypto::b64::decode_url("11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo").and_then(|x| <[u8; 32]>::try_from(x.as_slice()).ok()).unwrap_or([0; 32]);
    t.check("acme: the JWK thumbprint of RFC 8037 A.3 (Ed25519)", acme::thumbprint(&PubKey::Ed25519(ed)).as_deref() == Some("kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k"));
    let mut point = alloc::vec![4u8];
    point.extend(crypto::b64::decode_url("MKBCTNIcKUSDii11ySs3526iDZ8AiTo7Tu6KPAqv7D4").unwrap_or_default());
    point.extend(crypto::b64::decode_url("4Etl6SRW2YiLUrN5vfvVHuhp7x8PxltmWWlbbM4IFyM").unwrap_or_default());
    let ec = PubKey::Ec { curve: crypto::ecdsa::Curve::P256, point };
    t.check("acme: the JWK thumbprint of a P-256 key (RFC 7517 A.1)", acme::thumbprint(&ec).as_deref() == Some("cn-I_WNMClehiVp51i_0VpOENW1upEerA8sEam5hn-s"));
    let cut = |point: Vec<u8>| acme::thumbprint(&PubKey::Ec { curve: crypto::ecdsa::Curve::P256, point }).is_none();
    t.check("acme: ... an EC point that is not uncompressed or of another length has none", cut(alloc::vec![4]) && cut(alloc::vec![2; 33]) && cut(alloc::vec![4; 64]));
    t.check("acme: RFC 3339 times (Z, fractions, offsets)", acme::rfc3339("2026-11-24T08:00:00Z") == Some(1_795_507_200) && acme::rfc3339("2026-11-24T08:00:00.123456Z") == Some(1_795_507_200) && acme::rfc3339("2026-11-24T09:30:00+01:30") == Some(1_795_507_200));
    t.check("acme: ... a malformed one is refused", ["2026-11-24", "2026-13-24T08:00:00Z", "2026-11-24T08:00:00", "2026-11-24T08:00:00.Z", "2026-11-24T08:00:00+0100"].iter().all(|s| acme::rfc3339(s).is_none()));
    t.check("acme: rfc3339_text writes them back", acme::rfc3339_text(1_795_507_200) == "2026-11-24T08:00:00Z");
    t.check("acme: dns-01's name and value (RFC 8555 8.4)", acme::dns01_name(&Identifier::parse("*.Example.test")) == "_acme-challenge.example.test" && acme::dns01_value("tok.thumb").len() == 43);
    t.check("acme: tls-alpn-01's name of an address (RFC 8738 6)", Identifier::parse("192.0.2.1").tls_name() == "1.2.0.192.in-addr.arpa" && Identifier::parse("2001:db8::1").tls_name().ends_with(".8.b.d.0.1.0.0.2.ip6.arpa"));
}

fn issuance(t: &mut T, p: &Pki, rig: &Rig) {
    let acme_srv = &rig.acme;
    let Ok(a) = new_acme(p, acme_srv) else {
        t.check("acme: the directory is read", false);
        return;
    };
    t.check("acme: the directory (terms, profiles, renewal information)", a.directory.terms_of_service.is_some() && a.directory.profiles.len() == 2 && a.directory.renewal_info.is_some() && !a.directory.external_account_required);
    let acc = a.account(key(Kind::P256, 111), &agreed());
    t.check("acme: an account is made (ES256)", acc.as_ref().is_ok_and(|x| x.created && x.status == "valid"));
    let Ok(acc) = acc else { return };
    // http-01 for two names
    let ck = key(Kind::P256, 112);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut h = Seen(rig.http01.clone(), seen.clone());
    let r = acc.issue(&ids(&["a.example.test", "b.example.test"]), &ck, &mut h, &OrderOptions::default());
    let leaf = r.as_ref().ok().and_then(|i| Cert::parse(&i.chain[0]));
    t.check("acme: http-01: issued for both names and the certificate key", leaf.as_ref().is_some_and(|c| c.names("a.example.test") && c.names("b.example.test") && c.key == ck.public_key()));
    t.check("acme: ... the chain ends in the CA, the PEM holds it", r.as_ref().is_ok_and(|i| i.chain.len() == 2 && i.chain[1] == p.ca.der && i.pem.matches("BEGIN CERTIFICATE").count() == 2 && i.order.status == "valid"));
    let csr = acme_srv.state.lock().csrs.last().cloned();
    t.check("acme: ... the CSR asked for the names, under the certificate key", csr.is_some_and(|c| c.dns == ["a.example.test", "b.example.test"] && c.spki == ck.spki() && c.cn == "a.example.test"));
    let alt = r.as_ref().ok().and_then(|i| i.alternates.first().cloned()).map(|u| acc.certificate(&u));
    t.check("acme: ... an alternate chain (Link rel=alternate)", alt.is_some_and(|x| x.is_ok_and(|(c, _, _)| c.len() == 1)));
    let tokens = seen.lock().clone();
    t.check("acme: ... the challenge tokens are gone again", tokens.len() == 2 && tokens.iter().all(|tk| rig.http01.answer(&Request::new("GET", &format!("/.well-known/acme-challenge/{}", tk))).is_none()));
    // tls-alpn-01, and the site's own certificate for every other client
    let mut alpn = rig.alpn.clone();
    let r = acc.issue(&ids(&["c.example.test"]), &ck, &mut alpn, &OrderOptions::default());
    t.check("acme: tls-alpn-01: issued", r.as_ref().is_ok_and(|i| Cert::parse(&i.chain[0]).is_some_and(|c| c.names("c.example.test"))));
    let tcp = TcpStream::connect(&SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, rig.tls.local_addr().port), 3000);
    let site = tcp.ok().and_then(|tcp| tls::connect(Box::new(tcp), ClientConfig { alpn: alloc::vec![String::from("http/1.1")], ..ClientConfig::new(Trust::Anchors(alloc::vec![p.ca.clone()]), "localhost") }).ok());
    t.check("acme: ... the server shows its own certificate to a client of http/1.1", site.is_some_and(|s| s.alpn().as_deref() == Some("http/1.1")));
    // while a challenge is up: its certificate for "acme-tls/1" alone, the site's for the rest
    let mut up = rig.alpn.clone();
    let ch = Challenge { kind: String::from("tls-alpn-01"), url: String::new(), status: String::new(), token: String::from("t0k3nt0k3nt0k3nt0k3n"), error: None };
    let id = Identifier::parse("c.example.test");
    let _ = up.present(&id, &ch, "t0k3n.thumb");
    let shown = |alpn: &str| {
        let tcp = TcpStream::connect(&SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, rig.tls.local_addr().port), 3000).ok()?;
        let s = tls::connect(Box::new(tcp), ClientConfig { alpn: alloc::vec![String::from(alpn)], ..ClientConfig::new(Trust::Any, "c.example.test") }).ok()?;
        let c = s.peer_certs().first().cloned();
        s.shutdown();
        c
    };
    let challenge = shown("acme-tls/1");
    let site = shown("http/1.1");
    up.cleanup(&id, &ch);
    t.check("acme: tls-alpn-01: the challenge certificate goes to \"acme-tls/1\" only", challenge.is_some_and(|c| c.self_signed() && c.unknown_critical && c.dns == ["c.example.test"]) && site.is_some_and(|c| !c.self_signed()));
    // a server without protocols of its own: setup makes it answer "acme-tls/1" and leaves every
    // other client as it was (no ALPN, no no_application_protocol)
    let mut bare = ServerConfig::new(p.server.clone());
    let solo = TlsAlpn01::new();
    solo.setup(&mut bare);
    let mut up = solo.clone();
    let _ = up.present(&id, &ch, "t0k3n.thumb");
    let hs = |offer: &[&str]| {
        let mut c = tls::Conn::client(ClientConfig { alpn: offer.iter().map(|s| String::from(*s)).collect(), ..ClientConfig::new(Trust::Any, "c.example.test") });
        let mut s = tls::Conn::server(bare.clone());
        crate::tlsweb::pump(&mut c, &mut s).ok().map(|_| (c.alpn().map(String::from), c.peer_certs().first().is_some_and(|x| x.self_signed())))
    };
    t.check("acme: TlsAlpn01::setup on a server without ALPN: a client of h2 is still served (no ALPN)", hs(&["h2", "http/1.1"]) == Some((None, false)));
    t.check("acme: ... \"acme-tls/1\" alone gets it and the challenge certificate", hs(&["acme-tls/1"]) == Some((Some(String::from("acme-tls/1")), true)));
    t.check("acme: ... \"acme-tls/1\" beside others counts as not offered", hs(&["acme-tls/1", "h2"]) == Some((None, false)));
    // one challenge per name: a second is refused, and its cleanup leaves the first
    let ch2 = Challenge { token: String::from("an0th3rt0k3nan0th3r"), ..ch.clone() };
    let mut other = solo.clone();
    let second = other.present(&id, &ch2, "an0th3r.thumb");
    other.cleanup(&id, &ch2);
    t.check("acme: tls-alpn-01: a second challenge for a name is refused, its cleanup leaves the first", second.is_err() && hs(&["acme-tls/1"]) == Some((Some(String::from("acme-tls/1")), true)));
    up.cleanup(&id, &ch);
    t.check("acme: ... once the first is cleaned up the name is free", other.present(&id, &ch2, "an0th3r.thumb").is_ok());
    other.cleanup(&id, &ch2);
    // dns-01 with a wildcard, http-01 for the plain name next to it
    let mut both = Solvers(alloc::vec![Box::new(rig.http01.clone()), Box::new(Dns(acme_srv.dns.clone()))]);
    let r = acc.issue(&ids(&["*.example.test", "example.test"]), &ck, &mut both, &OrderOptions::default());
    t.check("acme: dns-01 for a wildcard, http-01 beside it", r.as_ref().is_ok_and(|i| Cert::parse(&i.chain[0]).is_some_and(|c| c.dns.contains(&String::from("*.example.test")) && c.names("www.example.test") && c.names("example.test"))));
    t.check("acme: ... the TXT records are gone again", acme_srv.dns.lock().is_empty());
    let mut only_http = rig.http01.clone();
    t.check("acme: a wildcard with http-01 alone cannot be proved", matches!(acc.issue(&ids(&["*.other.test"]), &ck, &mut only_http, &OrderOptions::default()), Err(AcmeError::Solver(_))));
    // an IP address (RFC 8738)
    let r = acc.issue(&ids(&["127.0.0.1"]), &ck, &mut rig.http01.clone(), &OrderOptions::default());
    t.check("acme: an IP address by http-01", r.as_ref().is_ok_and(|i| Cert::parse(&i.chain[0]).is_some_and(|c| c.names("127.0.0.1") && c.dns.is_empty())));
    let r = acc.issue(&ids(&["127.0.0.1"]), &ck, &mut rig.alpn.clone(), &OrderOptions::default());
    t.check("acme: ... and by tls-alpn-01 (the reverse name)", r.is_ok());
    // profile and must-staple
    let r = acc.issue(&ids(&["d.example.test"]), &ck, &mut rig.http01.clone(), &OrderOptions { profile: Some(String::from("short")), must_staple: true, ..OrderOptions::default() });
    t.check("acme: a profile and must-staple asked for", r.as_ref().is_ok_and(|i| i.order.profile.as_deref() == Some("short") && Cert::parse(&i.chain[0]).is_some_and(|c| c.must_staple())) && acme_srv.last_order().1.as_deref() == Some("short"));
    t.check("acme: a profile the CA does not have", matches!(acc.new_order(&ids(&["e.example.test"]), &OrderOptions { profile: Some(String::from("none")), ..OrderOptions::default() }), Err(AcmeError::Problem(ref p)) if p.is("invalidProfile")));
    t.check("acme: the account key is not the certificate key", matches!(acc.issue(&ids(&["f.example.test"]), acc.key(), &mut rig.http01.clone(), &OrderOptions::default()), Err(AcmeError::Protocol(_))));
    t.check("acme: after finalize the client waits out its Retry-After before it looks", acme_srv.state.lock().early_polls == 0);
}

fn account_keys(t: &mut T, p: &Pki, rig: &Rig) {
    let Some(a) = need(t, "account keys: the directory", new_acme(p, &rig.acme)) else { return };
    for (kind, alg) in [(Kind::P384, "ES384"), (Kind::Rsa, "RS256"), (Kind::Ed25519, "EdDSA")] {
        let acc = a.account(key(kind, 120), &agreed());
        let r = acc.as_ref().map_err(|e| e.clone()).and_then(|acc| acc.issue(&ids(&["k.example.test"]), &key(Kind::P256, 121), &mut rig.http01.clone(), &OrderOptions::default()));
        t.check(&format!("acme: an account key of {} orders and gets the certificate", alg), r.is_ok());
    }
}

fn accounts(t: &mut T, p: &Pki, rig: &Rig) {
    let Some(a) = need(t, "accounts: the directory", new_acme(p, &rig.acme)) else { return };
    let k = key(Kind::P256, 130);
    let first = a.account(k.clone(), &agreed());
    let again = a.account(k.clone(), &agreed());
    t.check("acme: the same key finds its account (200, not made again)", first.as_ref().is_ok_and(|x| x.created) && again.as_ref().is_ok_and(|x| !x.created && Some(&x.url) == first.as_ref().ok().map(|f| &f.url)));
    let none = a.account(key(Kind::P256, 131), &NewAccount { only_existing: true, ..NewAccount::default() });
    t.check("acme: onlyReturnExisting for a new key: accountDoesNotExist", matches!(none, Err(AcmeError::Problem(ref p)) if p.is("accountDoesNotExist") && p.status == 400));
    let terms = a.account(key(Kind::P256, 132), &NewAccount::default());
    t.check("acme: the terms not agreed: the CA refuses", matches!(terms, Err(AcmeError::Problem(ref p)) if p.is("userActionRequired")));
    let Ok(mut acc) = again else { return };
    let r = acc.update_contact(&[String::from("mailto:ops@example.test")]);
    t.check("acme: the contact is replaced", r.is_ok() && acc.contact == ["mailto:ops@example.test"]);
    let r = acc.deactivate();
    t.check("acme: deactivated", r.is_ok() && acc.status == "deactivated");
    t.check("acme: ... it orders nothing more", matches!(acc.new_order(&ids(&["x.example.test"]), &OrderOptions::default()), Err(AcmeError::Problem(ref p)) if p.is("unauthorized")));
}

fn failures(t: &mut T, p: &Pki, rig: &Rig) {
    let Some(a) = need(t, "failures: the directory", new_acme(p, &rig.acme)) else { return };
    let Some(acc) = need(t, "failures: the account", a.account(key(Kind::P256, 140), &agreed())) else { return };
    let ck = key(Kind::P256, 141);
    let mut liar = Liar(rig.http01.clone());
    let r = acc.issue(&ids(&["g.example.test"]), &ck, &mut liar, &OrderOptions::default());
    t.check("acme: a wrong key authorization: the authorization is invalid, with the CA's reason", matches!(r, Err(AcmeError::Invalid(_, Some(ref p))) if p.is("incorrectResponse")));
    // a nonce refused once: the request goes again with the nonce of that answer
    rig.acme.state.lock().faults.bad_nonce = 1;
    let r = acc.new_order(&ids(&["h.example.test"]), &OrderOptions::default());
    t.check("acme: badNonce: tried again with a fresh nonce", r.is_ok());
    rig.acme.state.lock().faults.bad_nonce = 3;
    let r = acc.new_order(&ids(&["h.example.test"]), &OrderOptions::default());
    t.check("acme: ... three in a row: the CA's problem", matches!(r, Err(AcmeError::Problem(ref p)) if p.is("badNonce")));
    rig.acme.state.lock().faults.bad_nonce = 0;
}

fn lifecycle(t: &mut T, p: &Pki, rig: &Rig) {
    let Some(a) = need(t, "lifecycle: the directory", new_acme(p, &rig.acme)) else { return };
    let Some(mut acc) = need(t, "lifecycle: the account", a.account(key(Kind::P256, 150), &agreed())) else { return };
    let ck = key(Kind::P256, 151);
    let Ok(issued) = acc.issue(&ids(&["r.example.test"]), &ck, &mut rig.http01.clone(), &OrderOptions::default()) else {
        t.check("acme: lifecycle: issued", false);
        return;
    };
    let leaf = Cert::parse(&issued.chain[0]).unwrap_or_default();
    let info = a.renewal_info(&leaf);
    t.check("acme: renewal information: the CA's window, its Retry-After", info.as_ref().is_ok_and(|w| w.start == leaf.not_after - 30 * 86_400 && w.end == leaf.not_after - 28 * 86_400 && w.retry_after == 21_600));
    t.check("acme: ... renew_at lies in it", info.as_ref().is_ok_and(|w| w.renew_at(0.0) == w.start && w.renew_at(0.5) > w.start && w.renew_at(1.0) == w.end));
    let id = acme::ari_id(&leaf);
    let r = acc.issue(&ids(&["r.example.test"]), &ck, &mut rig.http01.clone(), &OrderOptions { replaces: id.clone(), ..OrderOptions::default() });
    t.check("acme: an order replaces the certificate (its ARI id)", r.is_ok() && id.is_some() && rig.acme.last_order().0 == id);
    let unknown = Cert { aki: alloc::vec![1, 2, 3], serial: alloc::vec![9], ..Cert::default() };
    t.check("acme: renewal information of a certificate the CA does not know", a.renewal_info(&unknown).is_err());
    // revocation, by the account and by the certificate's key
    let r = acc.revoke(&issued.chain[0], Some(4));
    t.check("acme: revoked by the account (superseded)", r.is_ok() && rig.acme.revoked(&issued.chain[0]));
    t.check("acme: ... a second time: alreadyRevoked", matches!(acc.revoke(&issued.chain[0], None), Err(AcmeError::Problem(ref p)) if p.is("alreadyRevoked")));
    t.check("acme: ... reason 7 does not exist", matches!(acc.revoke(&issued.chain[0], Some(7)), Err(AcmeError::Problem(ref p)) if p.is("badRevocationReason")));
    if let Some(i2) = need(t, "revoke_with_key: the certificate to revoke", acc.issue(&ids(&["s.example.test"]), &ck, &mut rig.http01.clone(), &OrderOptions::default())) {
        let other = key(Kind::P256, 152);
        t.check("acme: revoke_with_key: another key may not", acme::revoke_with_key(&a, &i2.chain[0], &other, Some(1)).is_err());
        t.check("acme: ... the certificate's own key may (keyCompromise)", acme::revoke_with_key(&a, &i2.chain[0], &ck, Some(1)).is_ok() && rig.acme.revoked(&i2.chain[0]));
    }
    // key change
    let new_key = key(Kind::P384, 153);
    let r = acc.change_key(new_key.clone());
    t.check("acme: the account key changed (keyChange)", r.is_ok() && acc.key().public_key() == new_key.public_key());
    t.check("acme: ... it orders with the new key", acc.new_order(&ids(&["t.example.test"]), &OrderOptions::default()).is_ok());
    let old = a.account(key(Kind::P256, 150), &NewAccount { only_existing: true, ..NewAccount::default() });
    t.check("acme: ... the old key has no account now", matches!(old, Err(AcmeError::Problem(ref p)) if p.is("accountDoesNotExist")));
    let found = a.account(new_key, &NewAccount { only_existing: true, ..NewAccount::default() });
    t.check("acme: ... the new key finds it", found.is_ok_and(|f| f.url == acc.url));
}

fn eab(t: &mut T, p: &Pki) {
    let mac = alloc::vec![7u8; 32];
    let Some(rig) = start_rig(t, p, "eab", Faults { eab: Some((String::from("kid-1"), mac.clone())), ..Faults::default() }) else { return };
    let Some(a) = need(t, "eab: the directory", new_acme(p, &rig.acme)) else {
        rig.stop();
        return;
    };
    t.check("acme: the directory asks for an external account binding", a.directory.external_account_required);
    t.check("acme: ... none given: refused before a request", matches!(a.account(key(Kind::P256, 160), &agreed()), Err(AcmeError::Protocol(_))));
    let good = Eab::new("kid-1", &crypto::b64::encode_url(&mac));
    let r = a.account(key(Kind::P256, 161), &NewAccount { eab: good, ..agreed() });
    t.check("acme: ... bound with the CA's key id and MAC key (HS256)", r.is_ok_and(|x| x.created));
    let bad = Eab::new("kid-1", &crypto::b64::encode_url(&[8u8; 32]));
    t.check("acme: ... a wrong MAC key is refused", matches!(a.account(key(Kind::P256, 162), &NewAccount { eab: bad, ..agreed() }), Err(AcmeError::Problem(ref p)) if p.is("unauthorized")));
    t.check("acme: Eab::new takes base64url only", Eab::new("k", "not base64!").is_none());
    rig.stop();
}

fn faults(t: &mut T, p: &Pki) {
    // the directory names an http URL: refused
    if let Some(rig) = start_rig(t, p, "http_url", Faults { http_url: true, ..Faults::default() }) {
        t.check("acme: a directory with an http URL is refused", matches!(new_acme(p, &rig.acme), Err(AcmeError::Protocol(_))));
        rig.stop();
    }
    // the certificate for another key
    if let Some(rig) = start_rig(t, p, "wrong_key", Faults { wrong_key: true, ..Faults::default() }) {
        let r = new_acme(p, &rig.acme).and_then(|a| a.account(key(Kind::P256, 170), &agreed())).and_then(|acc| acc.issue(&ids(&["w.example.test"]), &key(Kind::P256, 171), &mut rig.http01.clone(), &OrderOptions::default()));
        t.check("acme: a certificate the CA issued for another key is refused", matches!(r, Err(AcmeError::Protocol(_))));
        rig.stop();
    }
    // a certificate without one of the names ordered
    if let Some(rig) = start_rig(t, p, "drop_name", Faults { drop_name: true, ..Faults::default() }) {
        let r = new_acme(p, &rig.acme).and_then(|a| a.account(key(Kind::P256, 175), &agreed())).and_then(|acc| acc.issue(&ids(&["m.example.test", "n.example.test"]), &key(Kind::P256, 176), &mut rig.http01.clone(), &OrderOptions::default()));
        t.check("acme: a certificate that leaves a name out is refused", matches!(r, Err(AcmeError::Protocol(ref s)) if s.contains("n.example.test")));
        rig.stop();
    }
    // Location and Link relative to the request (RFC 9110 10.2.2, RFC 8288 3.2)
    if let Some(rig) = start_rig(t, p, "relative", Faults { relative: true, ..Faults::default() }) {
        let r = new_acme(p, &rig.acme).and_then(|a| a.account(key(Kind::P256, 179), &agreed())).and_then(|acc| {
            let i = acc.issue(&ids(&["q.example.test"]), &key(Kind::P256, 180), &mut rig.http01.clone(), &OrderOptions::default())?;
            Ok((acc.url.clone(), i.order.url.clone(), i.alternates.clone()))
        });
        let base = rig.acme.state.lock().base.clone();
        t.check("acme: relative Location and Link headers are resolved against the request",
                r.as_ref().is_ok_and(|(acct, order, alt)| acct.starts_with(&base) && order.starts_with(&format!("{}/order/", base)) && alt.first().is_some_and(|a| a.ends_with("/alt") && a.starts_with(&base))));
        rig.stop();
    }
    // a token that is not base64url (it goes into a path and a file name)
    if let Some(rig) = start_rig(t, p, "bad_token", Faults { bad_token: true, ..Faults::default() }) {
        let r = new_acme(p, &rig.acme).and_then(|a| a.account(key(Kind::P256, 177), &agreed())).and_then(|acc| acc.issue(&ids(&["o.example.test"]), &key(Kind::P256, 178), &mut rig.http01.clone(), &OrderOptions::default()));
        t.check("acme: a challenge token outside base64url is refused", matches!(r, Err(AcmeError::Protocol(_))));
        rig.stop();
    }
    // rate limited: the problem with its Retry-After
    if let Some(rig) = start_rig(t, p, "rate_limited", Faults { rate_limited: true, ..Faults::default() }) {
        let r = new_acme(p, &rig.acme).and_then(|a| a.account(key(Kind::P256, 172), &agreed())).and_then(|acc| acc.new_order(&ids(&["z.example.test"]), &OrderOptions::default()));
        t.check("acme: rateLimited with Retry-After", matches!(r, Err(AcmeError::Problem(ref p)) if p.is("rateLimited") && p.status == 429 && p.retry_after == Some(30)));
        rig.stop();
    }
    // a Retry-After date whose year is not four digits (its days would overflow): as if none
    if let Some(rig) = start_rig(t, p, "retry_after", Faults { rate_limited: true, retry_after: Some("Sun, 06 Nov 9223372036854775807 08:49:37 GMT"), ..Faults::default() }) {
        let r = new_acme(p, &rig.acme).and_then(|a| a.account(key(Kind::P256, 183), &agreed())).and_then(|acc| acc.new_order(&ids(&["z.example.test"]), &OrderOptions::default()));
        t.check("acme: ... a Retry-After date with a year of 19 digits counts as none", matches!(r, Err(AcmeError::Problem(ref p)) if p.is("rateLimited") && p.retry_after.is_none()));
        rig.stop();
    }
    // a CA that does not decide: the wait ends
    if let Some(rig) = start_rig(t, p, "stall", Faults { stall: true, ..Faults::default() }) {
        let r = new_acme(p, &rig.acme).and_then(|mut a| {
            a.timeout_ms = 1500;
            a.account(key(Kind::P256, 173), &agreed())
        });
        let start = plat::time_us();
        let r = r.and_then(|acc| acc.issue(&ids(&["y.example.test"]), &key(Kind::P256, 174), &mut rig.http01.clone(), &OrderOptions::default()));
        let ms = (plat::time_us() - start) / 1000;
        t.check("acme: an authorization never decided: Timeout after timeout_ms", matches!(r, Err(AcmeError::Timeout(_))) && (1000..10_000).contains(&ms));
        // a limit on Retry-After under half a second (the CA asks for one): no panic, the limit
        let r = new_acme(p, &rig.acme).and_then(|mut a| {
            a.timeout_ms = 800;
            a.max_poll_ms = 100;
            a.account(key(Kind::P256, 181), &agreed())
        });
        let r = r.and_then(|acc| acc.issue(&ids(&["u.example.test"]), &key(Kind::P256, 182), &mut rig.http01.clone(), &OrderOptions::default()));
        t.check("acme: ... with max_poll_ms under 500 ms too", matches!(r, Err(AcmeError::Timeout(_))));
        rig.stop();
    }
}

/// dns-01 through pebble-challtestsrv's management API (/set-txt, /clear-txt)
struct ChallTestSrv(String);

impl Solver for ChallTestSrv {
    fn kinds(&self) -> Vec<&'static str> {
        alloc::vec!["dns-01"]
    }
    fn present(&mut self, id: &Identifier, _ch: &Challenge, key_auth: &str) -> Result<(), String> {
        let body = format!(r#"{{"host":"{}.","value":"{}"}}"#, acme::dns01_name(id), acme::dns01_value(key_auth));
        let url = netio::Url::parse(&format!("{}/set-txt", self.0)).ok_or("url")?;
        let r = netio::Client::new().request("POST", &url, &netio::Headers::new(), body.as_bytes()).map_err(|e| format!("{}", e))?;
        if r.status == 200 { Ok(()) } else { Err(format!("set-txt: HTTP {}", r.status)) }
    }
    fn cleanup(&mut self, id: &Identifier, _ch: &Challenge) {
        let body = format!(r#"{{"host":"{}."}}"#, acme::dns01_name(id));
        if let Some(url) = netio::Url::parse(&format!("{}/clear-txt", self.0)) {
            let _ = netio::Client::new().request("POST", &url, &netio::Headers::new(), body.as_bytes());
        }
    }
}

/// --acme-pebble DIR --directory URL --roots URL --http PORT --tls PORT --chall URL [--eab KID:KEY]:
/// netio::acme against Pebble, Let's Encrypt's ACME test server (tools/pebble_check.py starts
/// it): DIR holds the PKI of --tls-pki (Pebble's https certificate is its server.pem), the
/// challenges are answered on the ports Pebble's config names, dns-01 through challtestsrv.
/// One line "ok NAME" / "FAIL NAME: why" per check; exit code 1 on a failure
pub fn pebble() -> i32 {
    let arg = |k: &str| plat::arg_value(k).unwrap_or_default();
    // the PKI of --tls-pki DIR --key p256: Pebble's https certificate is its server.pem
    let p = pki(60, Kind::P256);
    let same = plat::read_file(&plat::fs::join(&arg("--acme-pebble"), "ca.pem")).and_then(|b| netio::pem::decode(&String::from_utf8_lossy(&b), "CERTIFICATE"));
    if same.as_deref() != Some(&p.ca.der[..]) {
        plat::write_stderr(b"--acme-pebble DIR: not the PKI of --tls-pki DIR --key p256\n");
        return 2;
    }
    let failed = core::cell::Cell::new(0u32);
    let check = |name: &str, ok: bool, why: &str| {
        if !ok {
            failed.set(failed.get() + 1);
        }
        plat::write_stdout(format!("{} {}{}\n", if ok { "ok  " } else { "FAIL" }, name, if ok { String::new() } else { format!(": {}", why.chars().take(300).collect::<String>()) }).as_bytes());
    };
    fn err<X>(r: &Result<X, AcmeError>) -> String {
        match r {
            Err(e) => format!("{}", e),
            Ok(_) => String::new(),
        }
    }
    let new = || -> Result<Acme, AcmeError> {
        let mut a = Acme::new(client(&p), &arg("--directory"))?;
        a.poll_ms = 500;
        a.timeout_ms = 60_000;
        Ok(a)
    };
    let a = match new() {
        Ok(a) => a,
        Err(e) => {
            check("pebble: the directory", false, &format!("{}", e));
            return 1;
        }
    };
    // EAB only: the run of a Pebble that requires a binding
    if let Some((kid, mac)) = plat::arg_value("--eab").and_then(|e| e.split_once(':').map(|(a, b)| (String::from(a), String::from(b)))) {
        check("pebble eab: the directory asks for an external account binding", a.directory.external_account_required, "");
        let r = a.account(key(Kind::P256, 200), &agreed());
        check("pebble eab: none given: refused before a request", matches!(r, Err(AcmeError::Protocol(_))), &err(&r));
        let r = a.account(key(Kind::P256, 201), &NewAccount { eab: Eab::new(&kid, &mac), ..agreed() });
        check("pebble eab: bound with the key id and MAC key (HS256)", r.as_ref().is_ok_and(|x| x.created), &err(&r));
        let r = a.account(key(Kind::P256, 202), &NewAccount { eab: Eab::new(&kid, &crypto::b64::encode_url(&[1u8; 32])), ..agreed() });
        check("pebble eab: a wrong MAC key is refused", matches!(r, Err(AcmeError::Problem(_))), &err(&r));
        return if failed.get() == 0 { 0 } else { 1 };
    }
    check("pebble: the directory (renewal information, profiles)", a.directory.renewal_info.is_some() && a.directory.profiles.iter().any(|(n, _)| n == "shortlived"), &format!("{:?}", a.directory));
    // the servers Pebble checks the challenges against
    let http01 = Http01::new();
    let fallback: netio::server::Handler = Arc::new(|_r: &Request| Response::text(404, "no"));
    let port = |k: &str| arg(k).parse::<u16>().unwrap_or(0);
    let http = netio::Server::start(&SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, port("--http")), netio::ServerConfig { threads: 4, ..netio::ServerConfig::default() }, http01.handler(fallback));
    let alpn = TlsAlpn01::new();
    let mut cfg = ServerConfig::new(p.server.clone());
    alpn.setup(&mut cfg);
    let wrap: netio::server::Wrap = Arc::new(move |tcp: TcpStream| {
        let s = tls::accept(Box::new(tcp), cfg.clone()).ok()?;
        Some((Box::new(s) as Box<dyn Stream>, None))
    });
    let tls_srv = netio::Server::start_with(&SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, port("--tls")), netio::ServerConfig { threads: 4, ..netio::ServerConfig::default() }, Arc::new(|_r: &Request| Response::text(200, "site")), Some(wrap));
    let (Ok(http), Ok(tls_srv)) = (http, tls_srv) else {
        check("pebble: the challenge servers start", false, "");
        return 1;
    };
    // Pebble's root, to check the chains it issues
    let root = netio::Url::parse(&arg("--roots")).and_then(|u| client(&p).get(&u, &netio::Headers::new()).ok()).and_then(|r| netio::pem::decode(&String::from_utf8_lossy(r.body_bytes()), "CERTIFICATE")).and_then(|d| Cert::parse(&d));
    check("pebble: its root", root.is_some(), "");
    // (the time of each check: a certificate issued a moment ago is valid from then)
    let chain_ok = |i: &acme::Issued, name: &str| {
        let certs: Vec<Cert> = i.chain.iter().filter_map(|d| Cert::parse(d)).collect();
        let now = plat::unix_time_ns().div_euclid(1_000_000_000) + 1;
        root.as_ref().is_some_and(|r| netio::x509::verify_chain(&certs, core::slice::from_ref(r), now, Some(name), netio::x509::Usage::Server).is_ok())
    };
    let acc = a.account(key(Kind::P256, 210), &agreed());
    check("pebble: an account (ES256)", acc.as_ref().is_ok_and(|x| x.created && x.status == "valid"), &err(&acc));
    let Ok(mut acc) = acc else { return 1 };
    let again = a.account(key(Kind::P256, 210), &agreed());
    check("pebble: the same key finds it", again.as_ref().is_ok_and(|x| !x.created && x.url == acc.url), &err(&again));
    let ck = key(Kind::P256, 211);
    let r = acc.issue(&ids(&["a.example.test", "b.example.test"]), &ck, &mut http01.clone(), &OrderOptions::default());
    check("pebble: http-01: issued, the chain verifies to Pebble's root", r.as_ref().is_ok_and(|i| chain_ok(i, "a.example.test") && chain_ok(i, "b.example.test")), &err(&r));
    let first = r.ok();
    let r = acc.issue(&ids(&["c.example.test"]), &ck, &mut alpn.clone(), &OrderOptions::default());
    check("pebble: tls-alpn-01: issued", r.as_ref().is_ok_and(|i| chain_ok(i, "c.example.test")), &err(&r));
    let mut both = Solvers(alloc::vec![Box::new(http01.clone()), Box::new(ChallTestSrv(arg("--chall")))]);
    let r = acc.issue(&ids(&["*.w.example.test", "w.example.test"]), &ck, &mut both, &OrderOptions::default());
    check("pebble: dns-01 for a wildcard (challtestsrv), http-01 beside it", r.as_ref().is_ok_and(|i| chain_ok(i, "x.w.example.test") && chain_ok(i, "w.example.test")), &err(&r));
    let r = acc.issue(&ids(&["127.0.0.1"]), &ck, &mut http01.clone(), &OrderOptions::default());
    check("pebble: an IP address (RFC 8738) by http-01", r.as_ref().is_ok_and(|i| chain_ok(i, "127.0.0.1")), &err(&r));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let r = acc.issue(&ids(&["127.0.0.1"]), &key(Kind::P256, 212), &mut Seen(alpn.clone(), seen.clone()), &OrderOptions::default());
    let proved = seen.lock().len();
    check("pebble: ... and by tls-alpn-01 (the reverse name in server_name; proved, not reused)", r.as_ref().is_ok_and(|i| chain_ok(i, "127.0.0.1")) && proved == 1, &format!("{} (challenges set up: {})", err(&r), proved));
    let r = acc.issue(&ids(&["s.example.test"]), &ck, &mut http01.clone(), &OrderOptions { profile: Some(String::from("shortlived")), ..OrderOptions::default() });
    let short = r.as_ref().ok().and_then(|i| Cert::parse(&i.chain[0]));
    check("pebble: the profile shortlived (a shorter validity)", short.is_some_and(|c| c.not_after - c.not_before < 10 * 86_400), &err(&r));
    let mut liar = Liar(http01.clone());
    let r = acc.issue(&ids(&["l.example.test"]), &ck, &mut liar, &OrderOptions::default());
    check("pebble: a wrong key authorization: invalid, with Pebble's reason", matches!(r, Err(AcmeError::Invalid(_, Some(_)))), &err(&r));
    for (kind, alg) in [(Kind::P384, "ES384"), (Kind::Rsa, "RS256")] {
        let r = a.account(key(kind, 220), &agreed()).and_then(|x| x.issue(&ids(&["k.example.test"]), &key(Kind::P256, 221), &mut http01.clone(), &OrderOptions::default()));
        check(&format!("pebble: an account key of {} gets its certificate", alg), r.is_ok(), &err(&r));
    }
    // Pebble, like Let's Encrypt, takes no EdDSA account keys: its problem comes back as it is
    let r = a.account(key(Kind::Ed25519, 222), &agreed());
    check("pebble: an EdDSA account key: badSignatureAlgorithm (Pebble takes RS256, ES256, ES384, ES512)", matches!(r, Err(AcmeError::Problem(ref p)) if p.is("badSignatureAlgorithm")), &err(&r));
    if let Some(i) = &first {
        let leaf = Cert::parse(&i.chain[0]).unwrap_or_default();
        let info = a.renewal_info(&leaf);
        check("pebble: renewal information (ARI): a window within the validity", info.as_ref().is_ok_and(|w| w.start >= leaf.not_before && w.end <= leaf.not_after && w.start <= w.end), &err(&info));
        let r = acc.issue(&ids(&["a.example.test", "b.example.test"]), &ck, &mut http01.clone(), &OrderOptions { replaces: acme::ari_id(&leaf), ..OrderOptions::default() });
        check("pebble: an order that replaces it (its ARI id)", r.is_ok(), &err(&r));
        let r = acc.revoke(&i.chain[0], Some(4));
        check("pebble: revoked by the account", r.is_ok(), &err(&r));
        let r = acc.revoke(&i.chain[0], None);
        check("pebble: ... again: alreadyRevoked", matches!(r, Err(AcmeError::Problem(ref p)) if p.is("alreadyRevoked")), &err(&r));
    }
    match acc.issue(&ids(&["v.example.test"]), &ck, &mut http01.clone(), &OrderOptions::default()) {
        Ok(i) => {
            let r = acme::revoke_with_key(&a, &i.chain[0], &ck, Some(1));
            check("pebble: revoked by the certificate's key (keyCompromise)", r.is_ok(), &err(&r));
        }
        Err(e) => check("pebble: revoked by the certificate's key (keyCompromise)", false, &format!("its certificate: {}", e)),
    }
    let nk = key(Kind::P384, 230);
    let r = acc.change_key(nk.clone());
    check("pebble: the account key changed (keyChange)", r.is_ok(), &err(&r));
    let r = acc.new_order(&ids(&["n.example.test"]), &OrderOptions::default());
    check("pebble: ... it orders with the new key", r.is_ok(), &err(&r));
    let r = a.account(key(Kind::P256, 210), &NewAccount { only_existing: true, ..NewAccount::default() });
    check("pebble: ... the old key has no account", matches!(r, Err(AcmeError::Problem(ref p)) if p.is("accountDoesNotExist")), &err(&r));
    let r = acc.update_contact(&[String::from("mailto:ops@example.test")]);
    check("pebble: the contact replaced", r.is_ok() && acc.contact == ["mailto:ops@example.test"], &format!("{:?} {:?}", r, acc.contact));
    let r = acc.deactivate();
    check("pebble: deactivated", r.is_ok() && acc.status == "deactivated", &format!("{:?}", r));
    let r = acc.new_order(&ids(&["d.example.test"]), &OrderOptions::default());
    check("pebble: ... it orders nothing more", matches!(r, Err(AcmeError::Problem(_))), &err(&r));
    http.stop();
    tls_srv.stop();
    if failed.get() == 0 { 0 } else { 1 }
}
