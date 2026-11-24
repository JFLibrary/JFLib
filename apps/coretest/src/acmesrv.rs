// acmesrv -- an ACME server (RFC 8555) in this process for netio::acme's checks: https on
// 127.0.0.1 under the test CA, JWS checked (nonce, url, kid or jwk, the signature), accounts,
// orders, authorizations, challenges validated for real (http-01 against the test's HTTP server,
// tls-alpn-01 by a TLS handshake with "acme-tls/1", dns-01 against a TXT table the test fills),
// finalize from the CSR, certificates with an alternate chain, revocation, key change, renewal
// information (RFC 9773), external account binding, profiles; faults on demand

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use agent_core::json::{self, Value};
use crypto::b64;
use crypto::ecdsa::Curve;
use crypto::HashAlg;
use netio::http::{Request, Response};
use netio::tls::{self, ClientConfig, ServerConfig, Trust};
use netio::x509::{Cert, CertBuilder, Csr, IpAddr, PrivateKey, PubKey, SigAlg};
use netio::Stream;
use plat::net::{SocketAddr, TcpStream};
use plat::sync::Mutex;

use crate::tlsweb::Pki;

/// faults and switches a check sets
#[derive(Default, Clone)]
pub struct Faults {
    /// answer the next signed request with badNonce
    pub bad_nonce: u32,
    /// newOrder answers 429 rateLimited (Retry-After: retry_after, else 30 seconds)
    pub rate_limited: bool,
    pub retry_after: Option<&'static str>,
    /// the certificate is issued for another key
    pub wrong_key: bool,
    /// the certificate leaves the order's last name out
    pub drop_name: bool,
    /// challenge tokens with characters outside base64url
    pub bad_token: bool,
    /// Location and Link headers relative to the request: the account's with its own host
    /// ("//127.0.0.1:N/acct/1"), the order's with a dot segment ("../order/1"), the alternate
    /// chain's beside the certificate ("1/alt")
    pub relative: bool,
    /// challenges are never decided (an authorization stays pending)
    pub stall: bool,
    /// the directory names an http URL
    pub http_url: bool,
    /// accounts need an external account binding (kid, MAC key)
    pub eab: Option<(String, Vec<u8>)>,
}

struct Acct {
    key: PubKey,
    status: String,
    contact: Vec<String>,
}

struct Ch {
    kind: &'static str,
    token: String,
    status: String,
    error: Option<Value>,
}

struct Authz {
    acct: usize,
    ty: String,
    value: String,
    wildcard: bool,
    status: String,
    challenges: Vec<Ch>,
    /// looks left that still say pending after it was decided
    lag: u32,
}

struct Order {
    acct: usize,
    ids: Vec<(String, String)>,
    authz: Vec<usize>,
    status: String,
    cert: Option<usize>,
    lag: u32,
    /// (ms) when the Retry-After of the finalize answer is over
    retry_at_ms: u64,
    profile: Option<String>,
    replaces: Option<String>,
}

struct Issued {
    acct: usize,
    chain: Vec<Vec<u8>>,
    revoked: bool,
}

pub struct State {
    pub base: String,
    nonces: Vec<String>,
    accts: Vec<Acct>,
    orders: Vec<Order>,
    authz: Vec<Authz>,
    certs: Vec<Issued>,
    pub faults: Faults,
    /// where http-01 and tls-alpn-01 are checked (the test's servers)
    pub http_port: u16,
    pub tls_port: u16,
    /// the CSRs finalize got
    pub csrs: Vec<Csr>,
    /// looks at an order before the Retry-After of its finalize answer was over
    pub early_polls: u32,
}

pub struct AcmeServer {
    pub srv: netio::Server,
    pub state: Arc<Mutex<State>>,
    /// TXT records for dns-01 (name -> values)
    pub dns: Arc<Mutex<BTreeMap<String, Vec<String>>>>,
}

impl AcmeServer {
    pub fn directory(&self) -> String {
        format!("{}/dir", self.state.lock().base)
    }

    /// what the account of each order the server made asked for (replaces, profile)
    pub fn last_order(&self) -> (Option<String>, Option<String>) {
        let s = self.state.lock();
        s.orders.last().map(|o| (o.replaces.clone(), o.profile.clone())).unwrap_or((None, None))
    }

    pub fn revoked(&self, der: &[u8]) -> bool {
        self.state.lock().certs.iter().any(|c| c.chain[0] == der && c.revoked)
    }
}

fn err(status: u16, kind: &str, detail: &str) -> Response {
    let v = Value::object([("type", Value::string(format!("urn:ietf:params:acme:error:{}", kind))), ("detail", Value::string(detail)), ("status", Value::u64(status as u64))]);
    Response::bytes(status, "application/problem+json", json::stringify(&v).unwrap_or_default().into_bytes())
}

fn ok_json(status: u16, v: &Value) -> Response {
    Response::bytes(status, "application/json", json::stringify(v).unwrap_or_default().into_bytes())
}

fn jwk_key(v: &Value) -> Option<PubKey> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str());
    match s("kty")? {
        "EC" => {
            let curve = match s("crv")? {
                "P-256" => Curve::P256,
                "P-384" => Curve::P384,
                _ => return None,
            };
            let (x, y) = (b64::decode_url(s("x")?)?, b64::decode_url(s("y")?)?);
            if x.len() != curve.bytes() || y.len() != curve.bytes() {
                return None;
            }
            let mut point = alloc::vec![4u8];
            point.extend_from_slice(&x);
            point.extend_from_slice(&y);
            Some(PubKey::Ec { curve, point })
        }
        "RSA" => Some(PubKey::Rsa { n: b64::decode_url(s("n")?)?, e: b64::decode_url(s("e")?)? }),
        "OKP" if s("crv")? == "Ed25519" => Some(PubKey::Ed25519(<[u8; 32]>::try_from(b64::decode_url(s("x")?)?.as_slice()).ok()?)),
        _ => None,
    }
}

/// a flattened JWS checked against `key` -> (protected header, payload; None for "")
fn open_jws(v: &Value, key: Option<&PubKey>) -> Result<(Value, Option<Value>, PubKey), &'static str> {
    let p64 = v.get("protected").and_then(|x| x.as_str()).ok_or("protected")?;
    let b64p = v.get("payload").and_then(|x| x.as_str()).ok_or("payload")?;
    let sig = b64::decode_url(v.get("signature").and_then(|x| x.as_str()).ok_or("signature")?).ok_or("signature")?;
    let prot = json::parse(&b64::decode_url(p64).ok_or("protected")?).map_err(|_| "protected")?;
    let alg = prot.get("alg").and_then(|x| x.as_str()).ok_or("alg")?;
    let k = match (key, prot.get("jwk")) {
        (Some(k), _) => k.clone(),
        (None, Some(j)) => jwk_key(j).ok_or("jwk")?,
        _ => return Err("no key"),
    };
    let input = format!("{}.{}", p64, b64p);
    let probe = Cert { key: k.clone(), ..Cert::default() };
    let good = match (alg, &k) {
        ("ES256", PubKey::Ec { curve: Curve::P256, .. }) => sig.len() == 64 && crypto::ecdsa::der_from_p1363(&sig).is_some_and(|d| probe.verify(SigAlg::Ecdsa(HashAlg::Sha256), input.as_bytes(), &d)),
        ("ES384", PubKey::Ec { curve: Curve::P384, .. }) => sig.len() == 96 && crypto::ecdsa::der_from_p1363(&sig).is_some_and(|d| probe.verify(SigAlg::Ecdsa(HashAlg::Sha384), input.as_bytes(), &d)),
        ("RS256", PubKey::Rsa { .. }) => probe.verify(SigAlg::RsaPkcs1(HashAlg::Sha256), input.as_bytes(), &sig),
        ("EdDSA", PubKey::Ed25519(_)) => probe.verify(SigAlg::Ed25519, input.as_bytes(), &sig),
        _ => false,
    };
    if !good {
        return Err("signature");
    }
    let payload = if b64p.is_empty() { None } else { Some(json::parse(&b64::decode_url(b64p).ok_or("payload")?).map_err(|_| "payload")?) };
    Ok((prot, payload, k))
}

fn now() -> i64 {
    plat::unix_time_ns().div_euclid(1_000_000_000)
}

fn tp(k: &PubKey) -> String {
    netio::acme::thumbprint(k).unwrap_or_default()
}

/// the acmeIdentifier extension of a tls-alpn-01 certificate: (critical, its OCTET STRING)
fn acme_identifier(c: &Cert) -> Option<(bool, Vec<u8>)> {
    use netio::x509::der::{self as d, Der};
    let mut t = Der::new(&c.tbs).seq()?;
    t.optional(0xa0);
    for tag in [d::INTEGER, d::SEQUENCE, d::SEQUENCE, d::SEQUENCE, d::SEQUENCE, d::SEQUENCE] {
        t.read(tag)?;
    }
    let mut list = Der::new(t.read(0xa3)?).seq()?;
    while !list.is_empty() {
        let mut e = list.seq()?;
        let id = e.read(d::OID)?;
        let critical = if e.peek_tag() == Some(d::BOOLEAN) { e.bool()? } else { false };
        let val = e.read(d::OCTET_STRING)?;
        if id == d::oid_bytes(&[1, 3, 6, 1, 5, 5, 7, 1, 31]) {
            return Some((critical, Vec::from(Der::new(val).read(d::OCTET_STRING)?)));
        }
    }
    None
}

/// check a challenge as a CA does -> None when it passes, else the problem
fn validate(kind: &str, ty: &str, value: &str, token: &str, ka: &str, http_port: u16, tls_port: u16, dns: &Mutex<BTreeMap<String, Vec<String>>>) -> Option<Value> {
    challenge(kind, ty, value, token, ka, http_port, tls_port, dns)
        .err()
        .map(|detail| Value::object([("type", Value::string("urn:ietf:params:acme:error:incorrectResponse")), ("detail", Value::string(detail))]))
}

/// a challenge checked: every way it can go wrong is an Err (a connection refused among them)
fn challenge(kind: &str, ty: &str, value: &str, token: &str, ka: &str, http_port: u16, tls_port: u16, dns: &Mutex<BTreeMap<String, Vec<String>>>) -> Result<(), String> {
    let e = |s: &str| String::from(s);
    match kind {
        "http-01" => {
            let url = netio::Url::parse(&format!("http://127.0.0.1:{}/.well-known/acme-challenge/{}", http_port, token)).ok_or_else(|| e("http-01: the URL"))?;
            match netio::Client::new().get(&url, &netio::Headers::new()) {
                Ok(r) if r.status == 200 && r.body_bytes() == ka.as_bytes() => Ok(()),
                Ok(r) => Err(format!("http-01: HTTP {}, {} bytes", r.status, r.body_bytes().len())),
                Err(x) => Err(format!("http-01: {}", x)),
            }
        }
        "tls-alpn-01" => {
            let ip = if ty == "ip" { Some(IpAddr::parse(value).ok_or_else(|| e("tls-alpn-01: not an address"))?) } else { None };
            let id = match ip {
                Some(a) => netio::acme::Identifier::Ip(a),
                None => netio::acme::Identifier::Dns(String::from(value)),
            };
            let tcp = TcpStream::connect(&SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, tls_port), 3000).map_err(|_| e("tls-alpn-01: no connection to the challenge port"))?;
            tcp.set_read_timeout(Some(5000));
            let cfg = ClientConfig { alpn: alloc::vec![String::from("acme-tls/1")], ..ClientConfig::new(Trust::Any, &id.tls_name()) };
            let s = tls::connect(Box::new(tcp), cfg).map_err(|x| format!("tls-alpn-01: {}", x))?;
            let certs = s.peer_certs();
            let alpn = s.alpn();
            s.shutdown();
            let c = certs.first().ok_or_else(|| e("tls-alpn-01: no certificate"))?;
            let names_ok = match ip {
                Some(a) => c.ips == [a] && c.dns.is_empty(),
                None => c.dns == [String::from(value)] && c.ips.is_empty(),
            };
            let ext = acme_identifier(c);
            let want = crypto::sha256(ka.as_bytes());
            if alpn.as_deref() == Some("acme-tls/1") && names_ok && ext.is_some_and(|(crit, v)| crit && v == want) && c.self_signed() {
                Ok(())
            } else {
                Err(e("tls-alpn-01: not the challenge's certificate"))
            }
        }
        "dns-01" => {
            let want = b64::encode_url(&crypto::sha256(ka.as_bytes()));
            if dns.lock().get(&format!("_acme-challenge.{}", value)).is_some_and(|v| v.contains(&want)) { Ok(()) } else { Err(e("dns-01: no such TXT record")) }
        }
        _ => Err(e("unknown challenge")),
    }
}

impl State {
    fn nonce(&mut self) -> String {
        let n = b64::encode_url(&plat::random_bytes::<16>());
        self.nonces.push(n.clone());
        if self.nonces.len() > 512 {
            self.nonces.remove(0);
        }
        n
    }

    fn order_json(&self, i: usize) -> Value {
        let o = &self.orders[i];
        let ids: Vec<Value> = o.ids.iter().map(|(t, v)| Value::object([("type", Value::string(t.as_str())), ("value", Value::string(v.as_str()))])).collect();
        let az: Vec<Value> = o.authz.iter().map(|a| Value::string(format!("{}/authz/{}", self.base, a))).collect();
        let mut v = Value::object([
            ("status", Value::string(o.status.as_str())),
            ("expires", Value::string(netio::acme::rfc3339_text(now() + 7 * 86_400))),
            ("identifiers", Value::Array(ids)),
            ("authorizations", Value::Array(az)),
            ("finalize", Value::string(format!("{}/finalize/{}", self.base, i))),
        ]);
        if let (Some(c), "valid") = (o.cert, o.status.as_str()) {
            v.insert("certificate", Value::string(format!("{}/cert/{}", self.base, c)));
        }
        if let Some(p) = &o.profile {
            v.insert("profile", Value::string(p.as_str()));
        }
        v
    }

    fn authz_json(&self, i: usize, status: &str) -> Value {
        let a = &self.authz[i];
        let chs: Vec<Value> = a
            .challenges
            .iter()
            .enumerate()
            .map(|(k, c)| {
                let mut v = Value::object([("type", Value::string(c.kind)), ("url", Value::string(format!("{}/chall/{}/{}", self.base, i, k))), ("status", Value::string(c.status.as_str())), ("token", Value::string(c.token.as_str()))]);
                if let Some(e) = &c.error {
                    v.insert("error", e.clone());
                }
                v
            })
            .collect();
        let mut v = Value::object([
            ("status", Value::string(status)),
            ("identifier", Value::object([("type", Value::string(a.ty.as_str())), ("value", Value::string(a.value.as_str()))])),
            ("challenges", Value::Array(chs)),
        ]);
        if a.wildcard {
            v.insert("wildcard", Value::Bool(true));
        }
        v
    }

    /// an order is ready once all its authorizations are valid
    fn order_status(&mut self, i: usize) {
        let o = &self.orders[i];
        if o.status == "pending" {
            let st: Vec<&str> = o.authz.iter().map(|a| self.authz[*a].status.as_str()).collect();
            if st.iter().all(|s| *s == "valid") {
                self.orders[i].status = String::from("ready");
            } else if st.iter().any(|s| *s == "invalid") {
                self.orders[i].status = String::from("invalid");
            }
        }
    }
}

/// start it on 127.0.0.1 with the test CA's server identity; it issues under that CA
pub fn start(p: &Pki, faults: Faults) -> Option<AcmeServer> {
    let state = Arc::new(Mutex::new(State {
        base: String::new(),
        nonces: Vec::new(),
        accts: Vec::new(),
        orders: Vec::new(),
        authz: Vec::new(),
        certs: Vec::new(),
        faults,
        http_port: 0,
        tls_port: 0,
        csrs: Vec::new(),
        early_polls: 0,
    }));
    let dns: Arc<Mutex<BTreeMap<String, Vec<String>>>> = Arc::new(Mutex::new(BTreeMap::new()));
    let (st, dn, ca, ca_key) = (state.clone(), dns.clone(), p.ca.clone(), p.ca_key.clone());
    let handler = move |r: &Request| -> Response {
        let mut resp = handle(&st, &dn, &ca, &ca_key, r);
        let n = st.lock().nonce();
        resp = resp.header("Replay-Nonce", &n).header("Cache-Control", "no-store");
        resp
    };
    let scfg = ServerConfig::new(p.server.clone());
    let wrap: netio::server::Wrap = Arc::new(move |tcp: TcpStream| {
        let s = tls::accept(Box::new(tcp), scfg.clone()).ok()?;
        Some((Box::new(s) as Box<dyn Stream>, None))
    });
    let srv = netio::Server::start_with(&SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, 0), netio::ServerConfig { threads: 6, ..netio::ServerConfig::default() }, Arc::new(handler), Some(wrap)).ok()?;
    state.lock().base = format!("https://127.0.0.1:{}", srv.local_addr().port);
    Some(AcmeServer { srv, state, dns })
}

fn handle(st: &Mutex<State>, dns: &Mutex<BTreeMap<String, Vec<String>>>, ca: &Cert, ca_key: &PrivateKey, r: &Request) -> Response {
    let path = String::from(r.path());
    let base = st.lock().base.clone();
    if path == "/dir" {
        let s = st.lock();
        let u = |p: &str| Value::string(format!("{}{}", base, p));
        let mut meta = Value::object([("termsOfService", Value::string("https://127.0.0.1/terms")), ("profiles", Value::object([("classic", Value::string("90 days")), ("short", Value::string("6 days"))]))]);
        if s.faults.eab.is_some() {
            meta.insert("externalAccountRequired", Value::Bool(true));
        }
        let order = if s.faults.http_url { Value::string(format!("http://127.0.0.1/{}", "new-order")) } else { u("/new-order") };
        let v = Value::object([("newNonce", u("/nonce")), ("newAccount", u("/new-acct")), ("newOrder", order), ("revokeCert", u("/revoke")), ("keyChange", u("/key-change")), ("renewalInfo", u("/renewal")), ("meta", meta)]);
        return ok_json(200, &v);
    }
    if path == "/nonce" {
        return Response::new(if r.method == "HEAD" { 200 } else { 204 });
    }
    if let Some(id) = path.strip_prefix("/renewal/") {
        let s = st.lock();
        for c in &s.certs {
            let leaf = Cert::parse(&c.chain[0]).unwrap_or_default();
            if format!("{}.{}", b64::encode_url(&leaf.aki), b64::encode_url(&leaf.serial)) == id {
                let w = Value::object([("start", Value::string(netio::acme::rfc3339_text(leaf.not_after - 30 * 86_400))), ("end", Value::string(netio::acme::rfc3339_text(leaf.not_after - 28 * 86_400)))]);
                return ok_json(200, &Value::object([("suggestedWindow", w)])).header("Retry-After", "21600");
            }
        }
        return err(404, "malformed", "no such certificate");
    }
    if r.method != "POST" {
        return err(405, "malformed", "POST");
    }
    if r.header("Content-Type") != Some("application/jose+json") {
        return err(415, "malformed", "application/jose+json");
    }
    let Ok(body) = json::parse(&r.body) else { return err(400, "malformed", "not JSON") };
    // the header: a nonce of ours, this URL, kid or jwk
    let Some(prot) = body.get("protected").and_then(|x| x.as_str()).and_then(b64::decode_url).and_then(|b| json::parse(&b).ok()) else { return err(400, "malformed", "protected") };
    let url = format!("{}{}", base, r.target);
    {
        let mut s = st.lock();
        if s.faults.bad_nonce > 0 {
            s.faults.bad_nonce -= 1;
            return err(400, "badNonce", "try again");
        }
        let n = prot.get("nonce").and_then(|x| x.as_str()).unwrap_or("");
        let Some(at) = s.nonces.iter().position(|x| x == n) else { return err(400, "badNonce", "not a nonce of ours") };
        s.nonces.remove(at);
    }
    if prot.get("url").and_then(|x| x.as_str()) != Some(url.as_str()) {
        return err(401, "unauthorized", "url");
    }
    let kid = prot.get("kid").and_then(|x| x.as_str()).map(String::from);
    if kid.is_some() == prot.get("jwk").is_some() {
        return err(400, "malformed", "kid or jwk");
    }
    let acct = match &kid {
        Some(k) => {
            let s = st.lock();
            let i = k.strip_prefix(&format!("{}/acct/", base)).and_then(|x| x.parse::<usize>().ok()).filter(|i| *i < s.accts.len());
            match i {
                Some(i) if s.accts[i].status == "valid" => Some(i),
                Some(_) => return err(401, "unauthorized", "account deactivated"),
                None => return err(400, "accountDoesNotExist", "kid"),
            }
        }
        None => None,
    };
    let key = acct.map(|i| st.lock().accts[i].key.clone());
    let (_, payload, jwk) = match open_jws(&body, key.as_ref()) {
        Ok(x) => x,
        Err(e) => return err(400, "malformed", e),
    };
    // newAccount and revokeCert by the certificate's key take a jwk, every other request a kid
    if acct.is_none() && path != "/new-acct" && path != "/revoke" {
        return err(400, "malformed", "kid needed");
    }
    let empty = Value::Object(Vec::new());
    let pl = payload.as_ref().unwrap_or(&empty);
    match path.as_str() {
        "/new-acct" => {
            let mut s = st.lock();
            if let Some(i) = s.accts.iter().position(|a| a.key == jwk) {
                let v = Value::object([("status", Value::string(s.accts[i].status.as_str()))]);
                return ok_json(200, &v).header("Location", &format!("{}/acct/{}", base, i));
            }
            if pl.get("onlyReturnExisting").and_then(|x| x.as_bool()) == Some(true) {
                return err(400, "accountDoesNotExist", "no account of this key");
            }
            if pl.get("termsOfServiceAgreed").and_then(|x| x.as_bool()) != Some(true) {
                return err(403, "userActionRequired", "agree to the terms");
            }
            if let Some((ekid, mac)) = s.faults.eab.clone() {
                let Some(e) = pl.get("externalAccountBinding") else { return err(400, "externalAccountRequired", "binding") };
                let prot_ok = e.get("protected").and_then(|x| x.as_str()).and_then(b64::decode_url).and_then(|b| json::parse(&b).ok());
                let p64 = e.get("protected").and_then(|x| x.as_str()).unwrap_or("");
                let b64p = e.get("payload").and_then(|x| x.as_str()).unwrap_or("");
                let want = crypto::hmac::hmac_sha256(&mac, format!("{}.{}", p64, b64p).as_bytes());
                let sig = e.get("signature").and_then(|x| x.as_str()).and_then(b64::decode_url).unwrap_or_default();
                let inner_key = b64::decode_url(b64p).and_then(|b| json::parse(&b).ok()).and_then(|v| jwk_key(&v));
                let hdr_ok = prot_ok.is_some_and(|h| h.get("alg").and_then(|x| x.as_str()) == Some("HS256") && h.get("kid").and_then(|x| x.as_str()) == Some(ekid.as_str()) && h.get("url").and_then(|x| x.as_str()) == Some(url.as_str()));
                if !(hdr_ok && crypto::ct_eq(&want, &sig) && inner_key.as_ref() == Some(&jwk)) {
                    return err(401, "unauthorized", "external account binding");
                }
            }
            let contact = pl.get("contact").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default();
            s.accts.push(Acct { key: jwk, status: String::from("valid"), contact });
            let i = s.accts.len() - 1;
            let v = Value::object([("status", Value::string("valid")), ("orders", Value::string(format!("{}/acct/{}/orders", base, i)))]);
            let loc = if s.faults.relative { format!("//{}/acct/{}", base.trim_start_matches("https://"), i) } else { format!("{}/acct/{}", base, i) };
            ok_json(201, &v).header("Location", &loc)
        }
        "/new-order" => {
            let a = acct.unwrap_or(0);
            let mut s = st.lock();
            if s.faults.rate_limited {
                return err(429, "rateLimited", "too many orders").header("Retry-After", s.faults.retry_after.unwrap_or("30"));
            }
            let profile = pl.get("profile").and_then(|x| x.as_str()).map(String::from);
            if profile.as_deref().is_some_and(|p| p != "classic" && p != "short") {
                return err(400, "invalidProfile", "no such profile");
            }
            let Some(ids) = pl.get("identifiers").and_then(|x| x.as_array()) else { return err(400, "malformed", "identifiers") };
            let mut list = Vec::new();
            let mut az = Vec::new();
            for id in ids {
                let (Some(ty), Some(v)) = (id.get("type").and_then(|x| x.as_str()), id.get("value").and_then(|x| x.as_str())) else { return err(400, "malformed", "identifier") };
                if ty != "dns" && ty != "ip" {
                    return err(400, "unsupportedIdentifier", ty);
                }
                list.push((String::from(ty), String::from(v)));
                let (wildcard, value) = match v.strip_prefix("*.") {
                    Some(b) => (true, b),
                    None => (false, v),
                };
                let bad = s.faults.bad_token;
                let tok = || if bad { String::from("../../etc/passwd-token") } else { b64::encode_url(&plat::random_bytes::<16>()) };
                let mut challenges = Vec::new();
                if !wildcard {
                    challenges.push(Ch { kind: "http-01", token: tok(), status: String::from("pending"), error: None });
                    challenges.push(Ch { kind: "tls-alpn-01", token: tok(), status: String::from("pending"), error: None });
                }
                if ty == "dns" {
                    challenges.push(Ch { kind: "dns-01", token: tok(), status: String::from("pending"), error: None });
                }
                s.authz.push(Authz { acct: a, ty: String::from(ty), value: String::from(value), wildcard, status: String::from("pending"), challenges, lag: 0 });
                az.push(s.authz.len() - 1);
            }
            let replaces = pl.get("replaces").and_then(|x| x.as_str()).map(String::from);
            s.orders.push(Order { acct: a, ids: list, authz: az, status: String::from("pending"), cert: None, lag: 0, retry_at_ms: 0, profile, replaces });
            let i = s.orders.len() - 1;
            let loc = if s.faults.relative { format!("../order/{}", i) } else { format!("{}/order/{}", base, i) };
            ok_json(201, &s.order_json(i)).header("Location", &loc)
        }
        p if p.starts_with("/acct/") => {
            let Some(i) = p[6..].parse::<usize>().ok().filter(|i| Some(*i) == acct) else { return err(401, "unauthorized", "another account") };
            let mut s = st.lock();
            if let Some(c) = pl.get("contact").and_then(|x| x.as_array()) {
                s.accts[i].contact = c.iter().filter_map(|x| x.as_str().map(String::from)).collect();
            }
            if pl.get("status").and_then(|x| x.as_str()) == Some("deactivated") {
                s.accts[i].status = String::from("deactivated");
            }
            let contact = Value::Array(s.accts[i].contact.iter().map(|c| Value::string(c.as_str())).collect());
            ok_json(200, &Value::object([("status", Value::string(s.accts[i].status.as_str())), ("contact", contact)]))
        }
        p if p.starts_with("/order/") => {
            let mut s = st.lock();
            let Some(i) = p[7..].parse::<usize>().ok().filter(|i| *i < s.orders.len() && Some(s.orders[*i].acct) == acct) else { return err(404, "malformed", "order") };
            s.order_status(i);
            // processing until the finalize answer's Retry-After is over (a look before it counts)
            if s.orders[i].status == "valid" && s.orders[i].lag > 0 {
                if plat::time_us() / 1000 < s.orders[i].retry_at_ms {
                    s.early_polls += 1;
                    let mut v = s.order_json(i);
                    v.insert("status", Value::string("processing"));
                    return ok_json(200, &v).header("Retry-After", "1");
                }
                s.orders[i].lag = 0;
            }
            ok_json(200, &s.order_json(i))
        }
        p if p.starts_with("/authz/") => {
            let mut s = st.lock();
            let Some(i) = p[7..].parse::<usize>().ok().filter(|i| *i < s.authz.len() && Some(s.authz[*i].acct) == acct) else { return err(404, "malformed", "authorization") };
            let a = &mut s.authz[i];
            if a.lag > 0 {
                a.lag -= 1;
                return ok_json(200, &s.authz_json(i, "pending")).header("Retry-After", "1");
            }
            let status = s.authz[i].status.clone();
            let r = ok_json(200, &s.authz_json(i, &status));
            if status == "pending" { r.header("Retry-After", "1") } else { r }
        }
        p if p.starts_with("/chall/") => {
            let parts: Vec<usize> = p[7..].split('/').filter_map(|x| x.parse().ok()).collect();
            let (i, k) = match parts[..] {
                [i, k] => (i, k),
                _ => return err(404, "malformed", "challenge"),
            };
            let (kind, ty, value, token, key, ports, stall) = {
                let s = st.lock();
                if i >= s.authz.len() || k >= s.authz[i].challenges.len() || Some(s.authz[i].acct) != acct {
                    return err(404, "malformed", "challenge");
                }
                let a = &s.authz[i];
                let c = &a.challenges[k];
                (c.kind, a.ty.clone(), a.value.clone(), c.token.clone(), s.accts[a.acct].key.clone(), (s.http_port, s.tls_port), s.faults.stall)
            };
            // a look (POST-as-GET), or an answer to an authorization that is decided: as it is
            {
                let s = st.lock();
                if payload.is_none() || s.authz[i].status != "pending" {
                    let c = &s.authz[i].challenges[k];
                    return ok_json(200, &Value::object([("type", Value::string(c.kind)), ("status", Value::string(c.status.as_str())), ("token", Value::string(c.token.as_str()))]));
                }
            }
            if stall {
                return ok_json(200, &Value::object([("type", Value::string(kind)), ("status", Value::string("processing")), ("token", Value::string(token))]));
            }
            // checked now, outside the lock (the check makes connections); the authorization
            // says pending once more, so the client waits
            let ka = format!("{}.{}", token, tp(&key));
            let problem = validate(kind, &ty, &value, &token, &ka, ports.0, ports.1, dns);
            let mut s = st.lock();
            let a = &mut s.authz[i];
            // (another answer may have decided it meanwhile: that decision stays)
            if a.status == "pending" {
                a.challenges[k].status = String::from(if problem.is_none() { "valid" } else { "invalid" });
                a.challenges[k].error = problem.clone();
                a.status = String::from(if problem.is_none() { "valid" } else { "invalid" });
                a.lag = 1;
            }
            ok_json(200, &Value::object([("type", Value::string(kind)), ("status", Value::string("processing")), ("token", Value::string(token))])).header("Link", &format!("<{}/authz/{}>;rel=\"up\"", base, i))
        }
        p if p.starts_with("/finalize/") => {
            let mut s = st.lock();
            let Some(i) = p[10..].parse::<usize>().ok().filter(|i| *i < s.orders.len() && Some(s.orders[*i].acct) == acct) else { return err(404, "malformed", "order") };
            s.order_status(i);
            if s.orders[i].status != "ready" {
                return err(403, "orderNotReady", "the order is not ready");
            }
            let Some(csr) = pl.get("csr").and_then(|x| x.as_str()).and_then(b64::decode_url).and_then(|d| Csr::parse(&d)) else { return err(400, "badCSR", "the CSR cannot be read") };
            let mut want: Vec<String> = s.orders[i].ids.iter().map(|(_, v)| v.to_ascii_lowercase()).collect();
            let mut got: Vec<String> = csr.dns.clone();
            got.extend(csr.ips.iter().map(|ip| format!("{}", ip)));
            want.sort();
            got.sort();
            if want != got {
                return err(400, "badCSR", "the CSR names other identifiers");
            }
            if Some(&csr.key) == s.accts.get(s.orders[i].acct).map(|a| &a.key) {
                return err(400, "badCSR", "the account key");
            }
            let spki = if s.faults.wrong_key { crate::tlsweb::key(crate::tlsweb::Kind::P256, 199).spki() } else { csr.spki.clone() };
            let n = now();
            let mut dns_names = csr.dns.clone();
            if s.faults.drop_name {
                dns_names.pop();
            }
            let b = CertBuilder {
                serial: plat::random_bytes::<16>(),
                cn: csr.cn.clone(),
                not_before: n - 60,
                not_after: n + 90 * 86_400,
                dns: dns_names,
                ips: csr.ips.clone(),
                server_auth: true,
                spki,
                must_staple: csr.tls_features.contains(&5),
                ..CertBuilder::default()
            };
            let Some(leaf) = b.sign_key(Some(ca), ca_key, &[]) else { return err(500, "serverInternal", "signing") };
            s.csrs.push(csr);
            let acct_i = s.orders[i].acct;
            s.certs.push(Issued { acct: acct_i, chain: alloc::vec![leaf, ca.der.clone()], revoked: false });
            let c = s.certs.len() - 1;
            let o = &mut s.orders[i];
            o.status = String::from("valid");
            o.cert = Some(c);
            o.lag = 1;
            o.retry_at_ms = plat::time_us() / 1000 + 1000;
            let mut v = s.order_json(i);
            v.insert("status", Value::string("processing"));
            ok_json(200, &v).header("Retry-After", "1")
        }
        p if p.starts_with("/cert/") => {
            let s = st.lock();
            let (num, alt) = match p[6..].strip_suffix("/alt") {
                Some(x) => (x, true),
                None => (&p[6..], false),
            };
            let Some(i) = num.parse::<usize>().ok().filter(|i| *i < s.certs.len() && Some(s.certs[*i].acct) == acct) else { return err(404, "malformed", "certificate") };
            let chain = &s.certs[i].chain;
            let parts: &[Vec<u8>] = if alt { &chain[..1] } else { chain };
            let pem: String = parts.iter().map(|d| netio::pem::encode("CERTIFICATE", d)).collect();
            let mut resp = Response::bytes(200, "application/pem-certificate-chain", pem.into_bytes());
            if !alt {
                let at = if s.faults.relative { format!("{}/alt", i) } else { format!("{}/cert/{}/alt", base, i) };
                resp = resp.header("Link", &format!("<{}>;rel=\"alternate\", <{}/dir>;rel=\"index\"", at, base));
            }
            resp
        }
        "/revoke" => {
            let Some(der) = pl.get("certificate").and_then(|x| x.as_str()).and_then(b64::decode_url) else { return err(400, "malformed", "certificate") };
            let reason = pl.get("reason").and_then(|x| x.as_u64());
            if reason.is_some_and(|r| r == 7 || r > 10) {
                return err(400, "badRevocationReason", "reason");
            }
            let mut s = st.lock();
            let Some(i) = s.certs.iter().position(|c| c.chain[0] == der) else { return err(404, "malformed", "not ours") };
            let leaf = Cert::parse(&der).unwrap_or_default();
            let allowed = Some(s.certs[i].acct) == acct || (acct.is_none() && leaf.key == jwk);
            if !allowed {
                return err(403, "unauthorized", "not yours");
            }
            if s.certs[i].revoked {
                return err(400, "alreadyRevoked", "already revoked");
            }
            s.certs[i].revoked = true;
            Response::new(200)
        }
        "/key-change" => {
            let Some(i) = acct else { return err(400, "malformed", "kid") };
            let Ok((iprot, ipl, newkey)) = open_jws(pl, None) else { return err(400, "malformed", "inner JWS") };
            let ipl = ipl.unwrap_or(Value::Null);
            let mut s = st.lock();
            let inner_ok = iprot.get("url").and_then(|x| x.as_str()) == Some(url.as_str())
                && iprot.get("nonce").is_none()
                && ipl.get("account").and_then(|x| x.as_str()) == kid.as_deref()
                && ipl.get("oldKey").and_then(jwk_key).as_ref() == Some(&s.accts[i].key);
            if !inner_ok {
                return err(400, "malformed", "inner JWS");
            }
            if s.accts.iter().any(|a| a.key == newkey) {
                return err(409, "malformed", "the key has an account").header("Location", &format!("{}/acct/{}", base, i));
            }
            s.accts[i].key = newkey;
            Response::new(200)
        }
        _ => err(404, "malformed", "no such resource"),
    }
}

pub fn ids(v: &[&str]) -> Vec<netio::acme::Identifier> {
    v.iter().map(|s| netio::acme::Identifier::parse(s)).collect()
}
