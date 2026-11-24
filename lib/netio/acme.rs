// acme -- certificates from a CA that speaks ACME (RFC 8555): Let's Encrypt, ZeroSSL, Google
// Trust Services, a company's step-ca.  http-01, tls-alpn-01 (RFC 8737) and dns-01 challenges,
// DNS names and IP addresses (RFC 8738), renewal information (ARI, RFC 9773), external
// account binding (RFC 8555 7.3.4), profiles, key change, revocation
//
//   Acme::new(client, directory_url) -> Result<Acme, AcmeError>     reads the CA's directory
//   acme.account(key, &NewAccount { contact, terms_agreed, eab, .. }) -> Account
//   account.issue(&ids, &cert_key, &mut solver, &OrderOptions) -> Issued { chain, pem, .. }
//       the whole issuance: an order, each authorization proved by `solver`, a certificate
//       request for `cert_key`, the chain once the CA has issued it
//   step by step: new_order, authorization, respond, wait_authorization, finalize,
//       wait_order, certificate
//   acme.renewal_info(&cert) (ARI: when to renew), account.revoke, change_key, update_contact,
//       deactivate; revoke_with_key (by the certificate's own key)
//   Solver: what proves control of a name -- Http01 (a netio Server's handler answers
//       /.well-known/acme-challenge/), TlsAlpn01 (a tls::ServerConfig that answers
//       "acme-tls/1"), a DNS provider's code of the program's own (dns01_name, dns01_value)
//
// Every request is a JWS (RFC 7515) signed with the account key: ES256 (P-256), ES384 (P-384),
// RS256 (RSA) or EdDSA (Ed25519; not every CA takes it).  The CA's URLs must be https; its
// answers are bounded (64 KiB, a certificate chain 1 MiB) and checked: the chain issued must be
// for the certificate key and every identifier ordered.  A wait for an authorization or the
// order follows Retry-After and ends after Acme::timeout_ms.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use agent_core::json::{self, Value};
use crypto::b64;
use crypto::ecdsa::Curve;
use crypto::HashAlg;
use plat::sync::Mutex;

use crate::client::{Client, HttpError};
use crate::http::{Headers, Request, Response};
use crate::server::Handler;
use crate::tls::{Hello, Identity, Select, ServerConfig};
use crate::url::Url;
use crate::x509::{self, Cert, CertBuilder, CsrBuilder, IpAddr, PrivateKey, PubKey, SigAlg};

/// Let's Encrypt's directory, and its staging one (for tests: higher rate limits, an untrusted
/// root)
pub const LETS_ENCRYPT: &str = "https://acme-v02.api.letsencrypt.org/directory";
pub const LETS_ENCRYPT_STAGING: &str = "https://acme-staging-v02.api.letsencrypt.org/directory";

/// the ALPN protocol of tls-alpn-01 (RFC 8737 6.2)
pub const ACME_TLS_ALPN: &str = "acme-tls/1";

/// the longest answer of the CA (JSON), and of a certificate chain
pub const MAX_ANSWER: usize = 64 * 1024;
pub const MAX_CHAIN: usize = 1 << 20;

const OID_ACME_IDENTIFIER: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 1, 31];

/// a problem document of the CA (RFC 8555 6.7, RFC 7807)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Problem {
    /// the HTTP status of the answer (0 inside an object: an authorization's error)
    pub status: u16,
    /// "urn:ietf:params:acme:error:rateLimited", ...
    pub kind: String,
    pub detail: String,
    /// the identifier a subproblem is about (RFC 8555 6.7.1)
    pub identifier: Option<Identifier>,
    pub subproblems: Vec<Problem>,
    /// seconds to wait before trying again (Retry-After), when the CA says
    pub retry_after: Option<u64>,
}

impl Problem {
    /// whether it is the ACME error `name` ("badNonce", "rateLimited", "unauthorized", ...)
    pub fn is(&self, name: &str) -> bool {
        self.kind.strip_prefix("urn:ietf:params:acme:error:") == Some(name)
    }

    fn from_json(v: &Value, status: u16) -> Problem {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let subproblems = v.get("subproblems").and_then(|x| x.as_array()).map(|a| a.iter().take(64).map(|p| Problem::from_json(p, 0)).collect()).unwrap_or_default();
        Problem { status, kind: s("type"), detail: s("detail"), identifier: v.get("identifier").and_then(Identifier::from_json), subproblems, retry_after: None }
    }
}

impl core::fmt::Display for Problem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let kind = self.kind.strip_prefix("urn:ietf:params:acme:error:").unwrap_or(&self.kind);
        write!(f, "{}: {}", kind, self.detail)?;
        for p in &self.subproblems {
            write!(f, "; {}", p)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AcmeError {
    /// the request did not reach the CA, or no HTTP answer came
    Http(HttpError),
    /// the CA refused (its problem document)
    Problem(Problem),
    /// an answer of the CA that does not follow RFC 8555
    Protocol(String),
    /// an authorization or the order ended invalid (the CA's reason, when it gave one)
    Invalid(String, Option<Problem>),
    /// the solver could not set a challenge up
    Solver(String),
    /// the CA did not decide within Acme::timeout_ms
    Timeout(String),
}

impl core::fmt::Display for AcmeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AcmeError::Http(e) => write!(f, "ACME: {}", e),
            AcmeError::Problem(p) => write!(f, "ACME: the CA refused: {}", p),
            AcmeError::Protocol(s) => write!(f, "ACME: {}", s),
            AcmeError::Invalid(s, Some(p)) => write!(f, "ACME: {} ({})", s, p),
            AcmeError::Invalid(s, None) => write!(f, "ACME: {}", s),
            AcmeError::Solver(s) => write!(f, "ACME: the challenge could not be set up: {}", s),
            AcmeError::Timeout(s) => write!(f, "ACME: {}", s),
        }
    }
}

impl From<HttpError> for AcmeError {
    fn from(e: HttpError) -> Self {
        AcmeError::Http(e)
    }
}

fn proto(s: &str) -> AcmeError {
    AcmeError::Protocol(String::from(s))
}

/// what a certificate is for
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Identifier {
    /// a DNS name, lower case; "*.example.com" for a wildcard (dns-01 only)
    Dns(String),
    /// an IP address (RFC 8738)
    Ip(IpAddr),
}

impl Identifier {
    /// a name or an address ("example.com", "*.example.com", "192.0.2.1", "2001:db8::1")
    pub fn parse(s: &str) -> Identifier {
        match IpAddr::parse(s.trim_matches(['[', ']'])) {
            Some(ip) => Identifier::Ip(ip),
            None => Identifier::Dns(s.to_ascii_lowercase()),
        }
    }

    /// the text of the identifier (an address in its RFC 5952 form)
    pub fn value(&self) -> String {
        match self {
            Identifier::Dns(d) => d.clone(),
            Identifier::Ip(ip) => format!("{}", ip),
        }
    }

    fn to_json(&self) -> Value {
        let ty = if matches!(self, Identifier::Dns(_)) { "dns" } else { "ip" };
        Value::object([("type", Value::string(ty)), ("value", Value::string(self.value()))])
    }

    fn from_json(v: &Value) -> Option<Identifier> {
        let value = v.get("value")?.as_str()?;
        match v.get("type")?.as_str()? {
            "dns" => Some(Identifier::Dns(value.to_ascii_lowercase())),
            "ip" => IpAddr::parse(value).map(Identifier::Ip),
            _ => None,
        }
    }

    /// the name a validator puts in server_name for tls-alpn-01: the name, or for an address
    /// its reverse DNS name (RFC 8738 6)
    pub fn tls_name(&self) -> String {
        match self {
            Identifier::Dns(d) => d.clone(),
            Identifier::Ip(IpAddr::V4(a)) => format!("{}.{}.{}.{}.in-addr.arpa", a[3], a[2], a[1], a[0]),
            Identifier::Ip(IpAddr::V6(a)) => {
                let mut s = String::new();
                for b in a.iter().rev() {
                    s.push_str(&format!("{:x}.{:x}.", b & 0xf, b >> 4));
                }
                s.push_str("ip6.arpa");
                s
            }
        }
    }
}

/// the CA's directory (RFC 8555 7.1.1)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Directory {
    pub new_nonce: String,
    pub new_account: String,
    pub new_order: String,
    pub revoke_cert: String,
    pub key_change: String,
    /// pre-authorization (most CAs have none)
    pub new_authz: Option<String>,
    /// renewal information (RFC 9773)
    pub renewal_info: Option<String>,
    /// what an account must agree to (NewAccount::terms_agreed)
    pub terms_of_service: Option<String>,
    pub website: Option<String>,
    /// the names a CAA record must name for this CA
    pub caa_identities: Vec<String>,
    /// accounts need an external account binding (NewAccount::eab)
    pub external_account_required: bool,
    /// the certificate profiles offered: (name, description)
    pub profiles: Vec<(String, String)>,
}

/// an answer of the CA, and the URL it answered
struct Answer {
    status: u16,
    headers: Headers,
    body: Vec<u8>,
    url: String,
}

impl Answer {
    fn json(&self) -> Result<Value, AcmeError> {
        let limits = json::Limits { bytes: MAX_ANSWER, ..json::Limits::default() };
        json::parse_with_limits(&self.body, limits).map_err(|_| proto("an answer of the CA that is not JSON"))
    }

    fn location(&self) -> Result<String, AcmeError> {
        let l = self.headers.get("Location").ok_or_else(|| proto("an answer without Location"))?;
        https_url(&resolve(&self.url, l))
    }

    fn retry_after(&self) -> Option<u64> {
        retry_after(self.headers.get("Retry-After")?)
    }

    /// the URLs of Link headers with this relation (RFC 8288: <url>;rel="up")
    fn links(&self, rel: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (k, v) in &self.headers.0 {
            if !k.eq_ignore_ascii_case("link") {
                continue;
            }
            for part in v.split(',') {
                let Some((url, params)) = part.trim().strip_prefix('<').and_then(|p| p.split_once('>')) else { continue };
                let has = params.split(';').any(|p| {
                    let p = p.trim();
                    p.strip_prefix("rel=").is_some_and(|r| r.trim_matches('"').split_ascii_whitespace().any(|x| x.eq_ignore_ascii_case(rel)))
                });
                if has {
                    if let Ok(u) = https_url(&resolve(&self.url, url)) {
                        out.push(u);
                    }
                }
            }
        }
        out
    }
}

/// a URL of a Location or Link header against the URL of the request (RFC 9110 10.2.2,
/// RFC 8288 3.2), as RFC 3986 5.2 resolves a reference: absolute as it is, "//host/path" on the
/// request's scheme, "/path" on its origin, else beside its path ("." and ".." taken out)
fn resolve(base: &str, r: &str) -> String {
    // a scheme: an absolute URL
    let scheme_end = r.find(':').filter(|&i| i > 0 && !r[..i].contains(['/', '?', '#']));
    if scheme_end.is_some() {
        return String::from(r);
    }
    let Some(u) = Url::parse(base) else { return String::from(r) };
    let scheme = base.split_once(':').map_or("https", |(s, _)| s);
    if r.starts_with("//") {
        return format!("{}:{}", scheme, r);
    }
    let path = u.target.split(['?', '#']).next().unwrap_or("");
    if r.is_empty() || r.starts_with('#') {
        let no_frag = u.target.split('#').next().unwrap_or("");
        return format!("{}{}{}", u.origin(), no_frag, r);
    }
    if r.starts_with('?') {
        return format!("{}{}{}", u.origin(), path, r);
    }
    let merged = if r.starts_with('/') { String::from(r) } else { format!("{}/{}", path.rsplit_once('/').map_or("", |(d, _)| d), r) };
    format!("{}{}", u.origin(), remove_dots(&merged))
}

/// a path without its "." and ".." segments (RFC 3986 5.2.4); the query stays as it is
fn remove_dots(p: &str) -> String {
    let (path, rest) = match p.find(['?', '#']) {
        Some(i) => p.split_at(i),
        None => (p, ""),
    };
    let mut out: Vec<&str> = Vec::new();
    let segs: Vec<&str> = path.split('/').skip(1).collect();
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        match *seg {
            "." => {
                if last {
                    out.push("");
                }
            }
            ".." => {
                out.pop();
                if last {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    format!("/{}{}", out.join("/"), rest)
}

/// an https URL of the CA (RFC 8555 6.1: every ACME URL is https)
fn https_url(s: &str) -> Result<String, AcmeError> {
    match Url::parse(s) {
        Some(u) if u.is_https() => Ok(String::from(s)),
        _ => Err(AcmeError::Protocol(format!("a URL of the CA that is not https: {}", s.chars().take(200).collect::<String>()))),
    }
}

fn url_of(v: &Value, key: &str) -> Result<String, AcmeError> {
    https_url(v.get(key).and_then(|x| x.as_str()).ok_or_else(|| AcmeError::Protocol(format!("no {} in the answer", key)))?)
}

fn opt_url(v: &Value, key: &str) -> Result<Option<String>, AcmeError> {
    v.get(key).and_then(|x| x.as_str()).map(https_url).transpose()
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// Retry-After: seconds, or an HTTP date (IMF-fixdate) -> seconds from now; at most a day
fn retry_after(v: &str) -> Option<u64> {
    let v = v.trim();
    if let Ok(s) = v.parse::<u64>() {
        return Some(s.min(86_400));
    }
    let t = http_date(v)?;
    let now = plat::unix_time_ns().div_euclid(1_000_000_000);
    Some((t - now).clamp(0, 86_400) as u64)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// "Sun, 06 Nov 1994 08:49:37 GMT" -> Unix seconds
fn http_date(s: &str) -> Option<i64> {
    let p: Vec<&str> = s.split_ascii_whitespace().collect();
    if p.len() != 6 || p[5] != "GMT" {
        return None;
    }
    // IMF-fixdate (RFC 9110 5.6.7): "Sun, 06 Nov 1994 08:49:37 GMT", every number of its digits
    let digits = |x: &str, n: usize| x.len() == n && x.bytes().all(|c| c.is_ascii_digit());
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let m = months.iter().position(|x| *x == p[2])? as i64 + 1;
    let hms: Vec<&str> = p[4].split(':').collect();
    if !digits(p[1], 2) || !digits(p[3], 4) || hms.len() != 3 || !hms.iter().all(|x| digits(x, 2)) {
        return None;
    }
    let d: i64 = p[1].parse().ok()?;
    let y: i64 = p[3].parse().ok()?;
    let hms: Vec<i64> = hms.iter().map(|x| x.parse().ok()).collect::<Option<Vec<i64>>>()?;
    if !(1..=31).contains(&d) || hms[0] > 23 || hms[1] > 59 || hms[2] > 60 {
        return None;
    }
    Some(days_from_civil(y, m, d) * 86_400 + hms[0] * 3600 + hms[1] * 60 + hms[2])
}

/// an RFC 3339 time ("2026-11-24T08:00:00Z", fractions, an offset) -> Unix seconds
pub fn rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |r: core::ops::Range<usize>| s.get(r).filter(|x| x.bytes().all(|c| c.is_ascii_digit())).and_then(|x| x.parse::<i64>().ok());
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut i = 19;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(|c| c.is_ascii_digit()) {
            i += 1;
        }
        if i == start {
            return None;
        }
    }
    let off = match b.get(i) {
        Some(b'Z' | b'z') if i + 1 == b.len() => 0,
        Some(c @ (b'+' | b'-')) if i + 6 == b.len() && b[i + 3] == b':' => {
            let (oh, om) = (n(i + 1..i + 3)?, n(i + 4..i + 6)?);
            let o = oh * 3600 + om * 60;
            if *c == b'+' { o } else { -o }
        }
        _ => return None,
    };
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - off)
}

/// Unix seconds -> "2026-11-24T08:00:00Z"
pub fn rfc3339_text(t: i64) -> String {
    let z = t.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    let s = t.rem_euclid(86_400);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, s / 3600, s / 60 % 60, s % 60)
}

// ------------------------------------------------------------------ keys and JWS

/// the JWS algorithm of an account key
fn jws_alg(key: &PrivateKey) -> Result<(&'static str, SigAlg), AcmeError> {
    match key.public_key() {
        PubKey::Ec { curve: Curve::P256, .. } => Ok(("ES256", SigAlg::Ecdsa(HashAlg::Sha256))),
        PubKey::Ec { curve: Curve::P384, .. } => Ok(("ES384", SigAlg::Ecdsa(HashAlg::Sha384))),
        PubKey::Rsa { .. } => Ok(("RS256", SigAlg::RsaPkcs1(HashAlg::Sha256))),
        PubKey::Ed25519(_) => Ok(("EdDSA", SigAlg::Ed25519)),
        _ => Err(proto("an account key of a kind ACME does not take")),
    }
}

/// the public JWK of a key (RFC 7517, 7518 6, RFC 8037), its members in the order of RFC 7638
/// (the thumbprint hashes exactly this text)
fn jwk_text(key: &PubKey) -> Result<String, AcmeError> {
    let u = b64::encode_url;
    match key {
        PubKey::Ec { curve, point } => {
            let n = curve.bytes();
            let crv = if *curve == Curve::P256 { "P-256" } else { "P-384" };
            // the uncompressed point: 0x04 x y
            if point.len() != 1 + 2 * n || point[0] != 4 {
                return Err(proto("an EC key that is not an uncompressed point"));
            }
            Ok(format!(r#"{{"crv":"{}","kty":"EC","x":"{}","y":"{}"}}"#, crv, u(&point[1..1 + n]), u(&point[1 + n..])))
        }
        PubKey::Rsa { n, e } => Ok(format!(r#"{{"e":"{}","kty":"RSA","n":"{}"}}"#, u(e), u(n))),
        PubKey::Ed25519(k) => Ok(format!(r#"{{"crv":"Ed25519","kty":"OKP","x":"{}"}}"#, u(k))),
        _ => Err(proto("an account key of a kind ACME does not take")),
    }
}

fn jwk(key: &PubKey) -> Result<Value, AcmeError> {
    json::parse(jwk_text(key)?.as_bytes()).map_err(|_| proto("JWK"))
}

/// the JWK thumbprint (RFC 7638, SHA-256), base64url
pub fn thumbprint(key: &PubKey) -> Option<String> {
    jwk_text(key).ok().map(|t| b64::encode_url(&crypto::sha256(t.as_bytes())))
}

/// a flattened JWS (RFC 7515 7.2.2) of `payload` (None: the empty payload of POST-as-GET)
fn jws(key: &PrivateKey, protected: Value, payload: Option<&Value>) -> Result<Value, AcmeError> {
    let (_, alg) = jws_alg(key)?;
    let p64 = b64::encode_url(json::stringify(&protected).map_err(|_| proto("JWS header"))?.as_bytes());
    let body64 = match payload {
        Some(v) => b64::encode_url(json::stringify(v).map_err(|_| proto("JWS payload"))?.as_bytes()),
        None => String::new(),
    };
    let input = format!("{}.{}", p64, body64);
    let random = plat::random_bytes::<32>();
    let sig = key.sign(alg, input.as_bytes(), &random).ok_or_else(|| proto("the account key cannot sign"))?;
    // ECDSA in JWS: r || s of the curve's size (RFC 7518 3.4), not DER
    let sig = match key.public_key() {
        PubKey::Ec { curve, .. } => crypto::ecdsa::p1363_from_der(curve, &sig).ok_or_else(|| proto("ECDSA signature"))?,
        _ => sig,
    };
    Ok(Value::object([("protected", Value::string(p64)), ("payload", Value::string(body64)), ("signature", Value::string(b64::encode_url(&sig)))]))
}

/// an external account binding (RFC 8555 7.3.4): the key id and MAC key the CA handed out
#[derive(Clone)]
pub struct Eab {
    pub kid: String,
    pub hmac_key: Vec<u8>,
}

impl Eab {
    /// the key id and the MAC key as the CA shows it (base64url)
    pub fn new(kid: &str, hmac_key_b64: &str) -> Option<Eab> {
        let k = b64::decode_url(hmac_key_b64.trim()).filter(|k| !k.is_empty())?;
        Some(Eab { kid: String::from(kid), hmac_key: k })
    }
}

impl core::fmt::Debug for Eab {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Eab({})", self.kid)
    }
}

impl Drop for Eab {
    fn drop(&mut self) {
        crypto::zeroize(&mut self.hmac_key);
    }
}

/// a new (or existing) account
#[derive(Clone, Debug, Default)]
pub struct NewAccount {
    /// "mailto:admin@example.com" (some CAs need one; Let's Encrypt no longer writes to them)
    pub contact: Vec<String>,
    /// the directory's terms of service agreed to
    pub terms_agreed: bool,
    /// only find the account of the key, make none (RFC 8555 7.3.1)
    pub only_existing: bool,
    pub eab: Option<Eab>,
}

/// an order's options
#[derive(Clone, Debug, Default)]
pub struct OrderOptions {
    /// the validity asked for (Unix seconds; most CAs decide it themselves)
    pub not_before: Option<i64>,
    pub not_after: Option<i64>,
    /// a profile of the directory's (Directory::profiles)
    pub profile: Option<String>,
    /// the certificate this one replaces (its ARI id, ari_id): the CA can tell a renewal
    pub replaces: Option<String>,
    /// a certificate that must come with a stapled OCSP response (RFC 7633; not every CA
    /// issues them)
    pub must_staple: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Order {
    pub url: String,
    /// "pending", "ready", "processing", "valid", "invalid"
    pub status: String,
    pub expires: Option<i64>,
    pub identifiers: Vec<Identifier>,
    pub authorizations: Vec<String>,
    pub finalize: String,
    pub certificate: Option<String>,
    pub error: Option<Problem>,
    pub profile: Option<String>,
}

impl Order {
    fn from_json(url: String, v: &Value) -> Result<Order, AcmeError> {
        let ids = v.get("identifiers").and_then(|x| x.as_array()).ok_or_else(|| proto("an order without identifiers"))?;
        let identifiers: Vec<Identifier> = ids.iter().map(Identifier::from_json).collect::<Option<Vec<_>>>().ok_or_else(|| proto("an order's identifier"))?;
        let authz = v.get("authorizations").and_then(|x| x.as_array()).ok_or_else(|| proto("an order without authorizations"))?;
        if authz.len() > 256 {
            return Err(proto("an order with too many authorizations"));
        }
        let authorizations = authz.iter().map(|a| a.as_str().ok_or_else(|| proto("an authorization URL")).and_then(https_url)).collect::<Result<Vec<_>, _>>()?;
        Ok(Order {
            url,
            status: str_of(v, "status"),
            expires: v.get("expires").and_then(|x| x.as_str()).and_then(rfc3339),
            identifiers,
            authorizations,
            finalize: url_of(v, "finalize")?,
            certificate: opt_url(v, "certificate")?,
            error: v.get("error").map(|e| Problem::from_json(e, 0)),
            profile: v.get("profile").and_then(|x| x.as_str()).map(String::from),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Challenge {
    /// "http-01", "tls-alpn-01", "dns-01", ...
    pub kind: String,
    pub url: String,
    /// "pending", "processing", "valid", "invalid"
    pub status: String,
    /// base64url (checked: only its alphabet, 16 to 256 characters)
    pub token: String,
    pub error: Option<Problem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorization {
    pub url: String,
    /// "pending", "valid", "invalid", "deactivated", "expired", "revoked"
    pub status: String,
    pub identifier: Identifier,
    /// for "*." plus the identifier (dns-01 only)
    pub wildcard: bool,
    pub expires: Option<i64>,
    pub challenges: Vec<Challenge>,
}

fn token_ok(t: &str) -> bool {
    (16..=256).contains(&t.len()) && t.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

impl Authorization {
    fn from_json(url: String, v: &Value) -> Result<Authorization, AcmeError> {
        let identifier = v.get("identifier").and_then(Identifier::from_json).ok_or_else(|| proto("an authorization's identifier"))?;
        let mut challenges = Vec::new();
        for c in v.get("challenges").and_then(|x| x.as_array()).ok_or_else(|| proto("an authorization without challenges"))?.iter().take(32) {
            let token = str_of(c, "token");
            let kind = str_of(c, "type");
            // a challenge of a type this client does not know may carry anything
            if matches!(kind.as_str(), "http-01" | "tls-alpn-01" | "dns-01") && !token_ok(&token) {
                return Err(proto("a challenge token that is not base64url"));
            }
            challenges.push(Challenge { kind, url: url_of(c, "url")?, status: str_of(c, "status"), token, error: c.get("error").map(|e| Problem::from_json(e, 0)) });
        }
        Ok(Authorization {
            url,
            status: str_of(v, "status"),
            identifier,
            wildcard: v.get("wildcard").and_then(|x| x.as_bool()).unwrap_or(false),
            expires: v.get("expires").and_then(|x| x.as_str()).and_then(rfc3339),
            challenges,
        })
    }

    /// what the CA said went wrong (the first challenge's error)
    fn problem(&self) -> Option<Problem> {
        self.challenges.iter().find_map(|c| c.error.clone())
    }
}

/// a certificate issued
#[derive(Clone, Debug)]
pub struct Issued {
    /// DER, leaf first
    pub chain: Vec<Vec<u8>>,
    /// the chain as the CA sent it (PEM): for the server's certificate file
    pub pem: String,
    /// other chains the CA offers (Link rel="alternate"; Acme::certificate fetches them)
    pub alternates: Vec<String>,
    pub order: Order,
}

impl Issued {
    /// the identity for a TLS server: the chain and the certificate key
    pub fn identity(&self, key: &PrivateKey) -> Identity {
        Identity::new(self.chain.clone(), key.clone())
    }
}

/// when to renew a certificate (ARI, RFC 9773)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenewalInfo {
    /// the window the CA suggests (Unix seconds): renew at a random time in it
    pub start: i64,
    pub end: i64,
    pub explanation: Option<String>,
    /// when to ask again (seconds, Retry-After; 6 hours when the CA does not say)
    pub retry_after: u64,
}

impl RenewalInfo {
    /// a time in the window to renew at: start + (end - start) * fraction (a random fraction in
    /// [0, 1) spreads the renewals out, as RFC 9773 4.2 asks)
    pub fn renew_at(&self, fraction: f64) -> i64 {
        let span = (self.end - self.start).max(0) as f64;
        self.start + (span * fraction.clamp(0.0, 1.0)) as i64
    }
}

/// a certificate's id in renewal information and in an order's "replaces" (RFC 9773 4.1): the
/// key identifier of its authority key identifier and its serial number; None without the
/// former
pub fn ari_id(cert: &Cert) -> Option<String> {
    if cert.aki.is_empty() || cert.serial.is_empty() {
        return None;
    }
    Some(format!("{}.{}", b64::encode_url(&cert.aki), b64::encode_url(&cert.serial)))
}

// ------------------------------------------------------------------ the client

/// the client of one CA (cheap to clone: the clones share the nonces)
#[derive(Clone)]
pub struct Acme {
    client: Client,
    pub directory: Arc<Directory>,
    nonces: Arc<Mutex<Vec<String>>>,
    /// the longest wait for an authorization or the order to be decided (ms; 3 minutes)
    pub timeout_ms: u64,
    /// the pause between two looks at an authorization or the order when the CA sends no
    /// Retry-After (ms; 2 seconds), and the longest one it takes from Retry-After (ms; 30 s)
    pub poll_ms: u32,
    pub max_poll_ms: u32,
}

impl Acme {
    /// the CA at `directory` (an https URL: LETS_ENCRYPT, ...) over `client` (with an https
    /// connector: trust::system_connector for public CAs)
    pub fn new(client: Client, directory: &str) -> Result<Acme, AcmeError> {
        let url = Url::parse(&https_url(directory)?).ok_or_else(|| proto("the directory URL"))?;
        let mut c = client;
        c.max_body = MAX_ANSWER;
        let r = c.request("GET", &url, &Headers::new(), &[])?;
        let a = Answer { status: r.status, headers: r.headers.clone(), body: Vec::from(r.body_bytes()), url: String::from(directory) };
        if a.status != 200 {
            return Err(AcmeError::Protocol(format!("the directory answered HTTP {}", a.status)));
        }
        let v = a.json()?;
        let meta = v.get("meta");
        let ms = |k: &str| meta.and_then(|m| m.get(k)).and_then(|x| x.as_str()).map(String::from);
        let dir = Directory {
            new_nonce: url_of(&v, "newNonce")?,
            new_account: url_of(&v, "newAccount")?,
            new_order: url_of(&v, "newOrder")?,
            revoke_cert: url_of(&v, "revokeCert")?,
            key_change: url_of(&v, "keyChange")?,
            new_authz: opt_url(&v, "newAuthz")?,
            renewal_info: opt_url(&v, "renewalInfo")?,
            terms_of_service: ms("termsOfService"),
            website: ms("website"),
            caa_identities: meta.and_then(|m| m.get("caaIdentities")).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default(),
            external_account_required: meta.and_then(|m| m.get("externalAccountRequired")).and_then(|x| x.as_bool()).unwrap_or(false),
            profiles: meta.and_then(|m| m.get("profiles")).and_then(|x| x.as_object()).map(|o| o.iter().map(|(k, d)| (k.clone(), d.as_str().unwrap_or("").to_string())).collect()).unwrap_or_default(),
        };
        Ok(Acme { client: c, directory: Arc::new(dir), nonces: Arc::new(Mutex::new(Vec::new())), timeout_ms: 180_000, poll_ms: 2000, max_poll_ms: 30_000 })
    }

    fn send(&self, method: &str, url: &str, headers: &Headers, body: &[u8], max: usize) -> Result<Answer, AcmeError> {
        let u = Url::parse(&https_url(url)?).ok_or_else(|| proto("a URL of the CA"))?;
        let mut c = self.client.clone();
        c.max_body = max;
        let r = c.request(method, &u, headers, body)?;
        Ok(Answer { status: r.status, headers: r.headers.clone(), body: Vec::from(r.body_bytes()), url: String::from(url) })
    }

    /// the nonce of an answer: base64url (RFC 8555 6.5.1)
    fn nonce_of(a: &Answer) -> Option<String> {
        a.headers.get("Replay-Nonce").filter(|n| !n.is_empty() && n.len() <= 256 && n.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')).map(String::from)
    }

    /// keep the nonce of an answer for the next request (of this client or a clone)
    fn keep_nonce(&self, a: &Answer) {
        if let Some(n) = Self::nonce_of(a) {
            let mut v = self.nonces.lock();
            if v.len() >= 16 {
                v.remove(0);
            }
            v.push(n);
        }
    }

    /// a nonce kept, else a new one from newNonce (that one is this request's alone)
    fn nonce(&self) -> Result<String, AcmeError> {
        if let Some(n) = self.nonces.lock().pop() {
            return Ok(n);
        }
        let dn = self.directory.new_nonce.clone();
        let a = self.send("HEAD", &dn, &Headers::new(), &[], 0)?;
        if !(a.status == 200 || a.status == 204) {
            return Err(AcmeError::Protocol(format!("newNonce answered HTTP {}", a.status)));
        }
        Self::nonce_of(&a).ok_or_else(|| proto("newNonce gave no Replay-Nonce"))
    }

    /// a signed POST: kid (an account's URL) or the key's JWK in the header; a badNonce is
    /// tried again with the nonce of that answer (RFC 8555 6.5: the retry's own, not shared
    /// with the clones); the nonces of other answers are kept for the next requests
    fn post(&self, key: &PrivateKey, kid: Option<&str>, url: &str, payload: Option<&Value>, max: usize) -> Result<Answer, AcmeError> {
        let (alg, _) = jws_alg(key)?;
        let mut next: Option<String> = None;
        for attempt in 0..3 {
            let nonce = match next.take() {
                Some(n) => n,
                None => self.nonce()?,
            };
            let mut protected = Value::object([("alg", Value::string(alg)), ("nonce", Value::string(nonce)), ("url", Value::string(url))]);
            match kid {
                Some(k) => protected.insert("kid", Value::string(k)),
                None => protected.insert("jwk", jwk(&key.public_key())?),
            };
            let body = json::stringify(&jws(key, protected, payload)?).map_err(|_| proto("JWS"))?;
            let mut h = Headers::new();
            h.set("Content-Type", "application/jose+json");
            let a = self.send("POST", url, &h, body.as_bytes(), max)?;
            if a.status < 400 {
                self.keep_nonce(&a);
                return Ok(a);
            }
            let p = self.problem(&a);
            if p.is("badNonce") && attempt < 2 {
                next = Self::nonce_of(&a);
                continue;
            }
            self.keep_nonce(&a);
            return Err(AcmeError::Problem(p));
        }
        Err(proto("the CA refused every nonce"))
    }

    fn problem(&self, a: &Answer) -> Problem {
        let mut p = match a.json() {
            Ok(v) => Problem::from_json(&v, a.status),
            Err(_) => Problem { status: a.status, detail: String::from_utf8_lossy(&a.body[..a.body.len().min(300)]).into_owned(), ..Problem::default() },
        };
        p.retry_after = a.retry_after();
        p
    }

    /// the account of `key` (made when there is none and new.only_existing is false)
    pub fn account(&self, key: PrivateKey, new: &NewAccount) -> Result<Account, AcmeError> {
        jws_alg(&key)?;
        if self.directory.external_account_required && new.eab.is_none() && !new.only_existing {
            return Err(proto("this CA needs an external account binding (NewAccount::eab)"));
        }
        let mut payload = Value::object([("termsOfServiceAgreed", Value::Bool(new.terms_agreed))]);
        if !new.contact.is_empty() {
            payload.insert("contact", Value::Array(new.contact.iter().map(|c| Value::string(c.as_str())).collect()));
        }
        if new.only_existing {
            payload.insert("onlyReturnExisting", Value::Bool(true));
        }
        let url = self.directory.new_account.clone();
        if let Some(eab) = &new.eab {
            payload.insert("externalAccountBinding", eab_jws(eab, &key, &url)?);
        }
        let a = self.post(&key, None, &url, Some(&payload), MAX_ANSWER)?;
        if a.status != 200 && a.status != 201 {
            return Err(AcmeError::Protocol(format!("newAccount answered HTTP {}", a.status)));
        }
        let v = a.json()?;
        let mut acc = Account { acme: self.clone(), key, url: a.location()?, status: String::new(), contact: Vec::new(), created: a.status == 201 };
        acc.read(&v);
        if acc.status != "valid" {
            return Err(AcmeError::Invalid(format!("the account is {}", acc.status), None));
        }
        Ok(acc)
    }

    /// when the CA suggests renewing `cert` (ARI, RFC 9773); Err when the directory has no
    /// renewalInfo or the certificate no authority key identifier
    pub fn renewal_info(&self, cert: &Cert) -> Result<RenewalInfo, AcmeError> {
        let base = self.directory.renewal_info.clone().ok_or_else(|| proto("the CA offers no renewal information"))?;
        let id = ari_id(cert).ok_or_else(|| proto("the certificate has no authority key identifier"))?;
        let url = format!("{}/{}", base.trim_end_matches('/'), id);
        let a = self.send("GET", &url, &Headers::new(), &[], MAX_ANSWER)?;
        if a.status != 200 {
            return Err(AcmeError::Problem(self.problem(&a)));
        }
        let v = a.json()?;
        let w = v.get("suggestedWindow").ok_or_else(|| proto("renewal information without suggestedWindow"))?;
        let t = |k: &str| w.get(k).and_then(|x| x.as_str()).and_then(rfc3339).ok_or_else(|| proto("a renewal window's time"));
        let (start, end) = (t("start")?, t("end")?);
        if end < start {
            return Err(proto("a renewal window that ends before it starts"));
        }
        Ok(RenewalInfo { start, end, explanation: opt_url(&v, "explanationURL")?, retry_after: a.retry_after().unwrap_or(6 * 3600).max(60) })
    }

    /// a chain of the CA (POST-as-GET to a certificate URL; for an alternate chain too), signed
    /// with `key` and `kid` of the account
    fn fetch_chain(&self, key: &PrivateKey, kid: &str, url: &str) -> Result<(Vec<Vec<u8>>, String, Vec<String>), AcmeError> {
        let a = self.post(key, Some(kid), url, None, MAX_CHAIN)?;
        if a.status != 200 {
            return Err(AcmeError::Protocol(format!("the certificate answered HTTP {}", a.status)));
        }
        let pem = String::from_utf8(a.body.clone()).map_err(|_| proto("a certificate chain that is not PEM"))?;
        let chain = crate::pem::decode_all(&pem, "CERTIFICATE");
        if chain.is_empty() || chain.iter().any(|c| Cert::parse(c).is_none()) {
            return Err(proto("a certificate chain that cannot be read"));
        }
        Ok((chain, pem, a.links("alternate")))
    }
}

/// the binding of a new account to the CA's key id (a JWS with HS256 over the account's JWK)
fn eab_jws(eab: &Eab, key: &PrivateKey, url: &str) -> Result<Value, AcmeError> {
    let protected = Value::object([("alg", Value::string("HS256")), ("kid", Value::string(eab.kid.as_str())), ("url", Value::string(url))]);
    let p64 = b64::encode_url(json::stringify(&protected).map_err(|_| proto("EAB"))?.as_bytes());
    let body64 = b64::encode_url(jwk_text(&key.public_key())?.as_bytes());
    let mac = crypto::hmac::hmac_sha256(&eab.hmac_key, format!("{}.{}", p64, body64).as_bytes());
    Ok(Value::object([("protected", Value::string(p64)), ("payload", Value::string(body64)), ("signature", Value::string(b64::encode_url(&mac)))]))
}

/// an account of a CA
#[derive(Clone)]
pub struct Account {
    acme: Acme,
    key: PrivateKey,
    /// the account's URL (its key id, "kid")
    pub url: String,
    /// "valid", "deactivated", "revoked"
    pub status: String,
    pub contact: Vec<String>,
    /// the CA made it on this call (201)
    pub created: bool,
}

impl core::fmt::Debug for Account {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Account({}, {})", self.url, self.status)
    }
}

impl Account {
    fn read(&mut self, v: &Value) {
        self.status = str_of(v, "status");
        self.contact = v.get("contact").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default();
    }

    pub fn acme(&self) -> &Acme {
        &self.acme
    }

    pub fn key(&self) -> &PrivateKey {
        &self.key
    }

    fn post(&self, url: &str, payload: Option<&Value>) -> Result<Answer, AcmeError> {
        self.acme.post(&self.key, Some(&self.url), url, payload, MAX_ANSWER)
    }

    /// what a challenge with this token must show (RFC 8555 8.1): token "." the thumbprint of
    /// the account key
    pub fn key_authorization(&self, token: &str) -> String {
        format!("{}.{}", token, thumbprint(&self.key.public_key()).unwrap_or_default())
    }

    /// a new order for these identifiers
    pub fn new_order(&self, ids: &[Identifier], opts: &OrderOptions) -> Result<Order, AcmeError> {
        if ids.is_empty() {
            return Err(proto("an order needs an identifier"));
        }
        let mut payload = Value::object([("identifiers", Value::Array(ids.iter().map(|i| i.to_json()).collect()))]);
        if let Some(t) = opts.not_before {
            payload.insert("notBefore", Value::string(rfc3339_text(t)));
        }
        if let Some(t) = opts.not_after {
            payload.insert("notAfter", Value::string(rfc3339_text(t)));
        }
        if let Some(p) = &opts.profile {
            payload.insert("profile", Value::string(p.as_str()));
        }
        if let Some(r) = &opts.replaces {
            payload.insert("replaces", Value::string(r.as_str()));
        }
        let url = self.acme.directory.new_order.clone();
        let a = self.post(&url, Some(&payload))?;
        if a.status != 201 {
            return Err(AcmeError::Protocol(format!("newOrder answered HTTP {}", a.status)));
        }
        Order::from_json(a.location()?, &a.json()?)
    }

    /// an order as it stands now
    pub fn order(&self, url: &str) -> Result<Order, AcmeError> {
        Order::from_json(https_url(url)?, &self.post(url, None)?.json()?)
    }

    pub fn authorization(&self, url: &str) -> Result<Authorization, AcmeError> {
        Authorization::from_json(https_url(url)?, &self.post(url, None)?.json()?)
    }

    /// tell the CA a challenge is ready to be checked (RFC 8555 7.5.1)
    pub fn respond(&self, ch: &Challenge) -> Result<(), AcmeError> {
        self.post(&ch.url, Some(&Value::Object(Vec::new()))).map(|_| ())
    }

    /// the pause before the next look: the CA's Retry-After within limits, else poll_ms
    /// the pause before the next look (Retry-After: half a second to max_poll_ms; else
    /// poll_ms), at most to the deadline; Timeout when that is reached (no look after it)
    fn pause(&self, retry: Option<u64>, deadline: u64) -> Result<(), AcmeError> {
        let max = self.acme.max_poll_ms as u64;
        let want = retry.map_or(self.acme.poll_ms as u64, |s| s.saturating_mul(1000).clamp(500.min(max), max));
        let now = plat::time_us() / 1000;
        if now < deadline {
            plat::sleep_ms(want.min(deadline - now) as u32);
        }
        if plat::time_us() / 1000 >= deadline {
            return Err(AcmeError::Timeout(String::from("the CA did not decide in time")));
        }
        Ok(())
    }

    /// wait until the authorization is decided: Ok when valid.  Acme::timeout_ms bounds the
    /// waiting: no look starts after it (a request under way runs out under the Client's
    /// timeouts)
    pub fn wait_authorization(&self, url: &str) -> Result<Authorization, AcmeError> {
        let deadline = (plat::time_us() / 1000) + self.acme.timeout_ms;
        loop {
            let a = self.post(url, None)?;
            let retry = a.retry_after();
            let z = Authorization::from_json(https_url(url)?, &a.json()?)?;
            match z.status.as_str() {
                "valid" => return Ok(z),
                "pending" => self.pause(retry, deadline)?,
                s => return Err(AcmeError::Invalid(format!("the authorization of {} is {}", z.identifier.value(), s), z.problem())),
            }
        }
    }

    /// wait until the order is no longer pending or processing (Acme::timeout_ms as
    /// wait_authorization)
    pub fn wait_order(&self, url: &str) -> Result<Order, AcmeError> {
        self.wait_order_after(url, None)
    }

    /// wait_order, after a pause first when an answer said "processing" (with its Retry-After)
    fn wait_order_after(&self, url: &str, first: Option<Option<u64>>) -> Result<Order, AcmeError> {
        let deadline = (plat::time_us() / 1000) + self.acme.timeout_ms;
        if let Some(retry) = first {
            self.pause(retry, deadline)?;
        }
        loop {
            let a = self.post(url, None)?;
            let retry = a.retry_after();
            let o = Order::from_json(https_url(url)?, &a.json()?)?;
            match o.status.as_str() {
                "ready" | "valid" => return Ok(o),
                "pending" | "processing" => self.pause(retry, deadline)?,
                s => return Err(AcmeError::Invalid(format!("the order is {}", s), o.error)),
            }
        }
    }

    /// send the certificate request (DER) of a ready order
    pub fn finalize(&self, order: &Order, csr: &[u8]) -> Result<Order, AcmeError> {
        self.finalize_retry(order, csr).map(|(o, _)| o)
    }

    /// finalize -> the order and the answer's Retry-After
    fn finalize_retry(&self, order: &Order, csr: &[u8]) -> Result<(Order, Option<u64>), AcmeError> {
        let payload = Value::object([("csr", Value::string(b64::encode_url(csr)))]);
        let a = self.post(&order.finalize, Some(&payload))?;
        Ok((Order::from_json(order.url.clone(), &a.json()?)?, a.retry_after()))
    }

    /// the chain at a certificate URL (an order's, or an alternate one) -> (DER chain, PEM,
    /// alternates)
    pub fn certificate(&self, url: &str) -> Result<(Vec<Vec<u8>>, String, Vec<String>), AcmeError> {
        self.acme.fetch_chain(&self.key, &self.url, url)
    }

    /// the whole issuance for `ids` and the key `cert_key` (not the account key): an order;
    /// each pending authorization proved with the first challenge type of solver.kinds() it
    /// offers (a wildcard only by dns-01), set up, answered, waited for and taken down; the
    /// certificate request; the chain, checked to be for `cert_key` and every identifier
    pub fn issue(&self, ids: &[Identifier], cert_key: &PrivateKey, solver: &mut dyn Solver, opts: &OrderOptions) -> Result<Issued, AcmeError> {
        if cert_key.spki() == self.key.spki() {
            return Err(proto("the certificate key must not be the account key (RFC 8555 11.1)"));
        }
        let order = self.new_order(ids, opts)?;
        for url in &order.authorizations {
            let z = self.authorization(url)?;
            match z.status.as_str() {
                "valid" => continue,
                "pending" => {}
                s => return Err(AcmeError::Invalid(format!("the authorization of {} is {}", z.identifier.value(), s), z.problem())),
            }
            let kinds = solver.kinds();
            let ch = kinds
                .iter()
                .filter(|k| !z.wildcard || **k == "dns-01")
                .find_map(|k| z.challenges.iter().find(|c| c.kind == *k))
                .ok_or_else(|| AcmeError::Solver(format!("no challenge for {} that the solver sets up", z.identifier.value())))?
                .clone();
            let ka = self.key_authorization(&ch.token);
            let id = if z.wildcard { Identifier::Dns(format!("*.{}", z.identifier.value())) } else { z.identifier.clone() };
            solver.present(&id, &ch, &ka).map_err(AcmeError::Solver)?;
            let r = self.respond(&ch).and_then(|_| self.wait_authorization(&z.url));
            solver.cleanup(&id, &ch);
            r?;
        }
        let order = self.wait_order(&order.url)?;
        let order = if order.status == "ready" {
            let mut b = CsrBuilder { must_staple: opts.must_staple, ..CsrBuilder::default() };
            for i in ids {
                match i {
                    Identifier::Dns(d) => b.dns.push(d.clone()),
                    Identifier::Ip(ip) => b.ips.push(*ip),
                }
            }
            b.cn = b.dns.first().filter(|d| d.len() <= 64).cloned().unwrap_or_default();
            let csr = b.sign(cert_key).ok_or_else(|| proto("the certificate request cannot be made (a name, the key)"))?;
            let (o, retry) = self.finalize_retry(&order, &csr)?;
            if o.status == "valid" { o } else { self.wait_order_after(&o.url, Some(retry))? }
        } else {
            order
        };
        if order.status != "valid" {
            return Err(AcmeError::Invalid(format!("the order is {}", order.status), order.error));
        }
        let url = order.certificate.clone().ok_or_else(|| proto("a valid order without a certificate"))?;
        let (chain, pem, alternates) = self.certificate(&url)?;
        let leaf = Cert::parse(&chain[0]).ok_or_else(|| proto("the certificate cannot be read"))?;
        if leaf.key != cert_key.public_key() {
            return Err(proto("the certificate issued is for another key"));
        }
        for i in ids {
            let ok = match i {
                Identifier::Dns(d) => leaf.dns.iter().any(|x| x == d),
                Identifier::Ip(ip) => leaf.ips.contains(ip),
            };
            if !ok {
                return Err(AcmeError::Protocol(format!("the certificate issued does not name {}", i.value())));
            }
        }
        Ok(Issued { chain, pem, alternates, order })
    }

    /// revoke a certificate of this account (RFC 8555 7.6); `reason`: a CRLReason (RFC 5280
    /// 5.3.1: 1 keyCompromise, 4 superseded, 5 cessationOfOperation, ...)
    pub fn revoke(&self, cert_der: &[u8], reason: Option<u8>) -> Result<(), AcmeError> {
        let url = self.acme.directory.revoke_cert.clone();
        self.post(&url, Some(&revocation(cert_der, reason))).map(|_| ())
    }

    /// change the account's key (RFC 8555 7.3.5)
    pub fn change_key(&mut self, new_key: PrivateKey) -> Result<(), AcmeError> {
        jws_alg(&new_key)?;
        let url = self.acme.directory.key_change.clone();
        let (alg, _) = jws_alg(&new_key)?;
        let inner_protected = Value::object([("alg", Value::string(alg)), ("jwk", jwk(&new_key.public_key())?), ("url", Value::string(url.as_str()))]);
        let inner_payload = Value::object([("account", Value::string(self.url.as_str())), ("oldKey", jwk(&self.key.public_key())?)]);
        let inner = jws(&new_key, inner_protected, Some(&inner_payload))?;
        self.post(&url, Some(&inner))?;
        self.key = new_key;
        Ok(())
    }

    /// replace the account's contact addresses
    pub fn update_contact(&mut self, contact: &[String]) -> Result<(), AcmeError> {
        let payload = Value::object([("contact", Value::Array(contact.iter().map(|c| Value::string(c.as_str())).collect()))]);
        let v = self.post(&self.url.clone(), Some(&payload))?.json()?;
        self.read(&v);
        Ok(())
    }

    /// end the account (RFC 8555 7.3.6): it cannot be used again
    pub fn deactivate(&mut self) -> Result<(), AcmeError> {
        let payload = Value::object([("status", Value::string("deactivated"))]);
        let v = self.post(&self.url.clone(), Some(&payload))?.json()?;
        self.read(&v);
        Ok(())
    }
}

fn revocation(cert_der: &[u8], reason: Option<u8>) -> Value {
    let mut p = Value::object([("certificate", Value::string(b64::encode_url(cert_der)))]);
    if let Some(r) = reason {
        p.insert("reason", Value::u64(r as u64));
    }
    p
}

/// revoke a certificate by its own key (signed with it, no account: a lost account key, a
/// compromised certificate key)
pub fn revoke_with_key(acme: &Acme, cert_der: &[u8], cert_key: &PrivateKey, reason: Option<u8>) -> Result<(), AcmeError> {
    let url = acme.directory.revoke_cert.clone();
    acme.post(cert_key, None, &url, Some(&revocation(cert_der, reason)), MAX_ANSWER).map(|_| ())
}

// ------------------------------------------------------------------ challenges

/// what proves control of an identifier
pub trait Solver {
    /// the challenge types it sets up, the preferred first ("http-01", "tls-alpn-01",
    /// "dns-01")
    fn kinds(&self) -> Vec<&'static str>;
    /// set the challenge up for `id` ("*.example.com" for a wildcard): `key_auth` is what it
    /// must show (dns-01: dns01_value(key_auth) as TXT of dns01_name(id))
    fn present(&mut self, id: &Identifier, ch: &Challenge, key_auth: &str) -> Result<(), String>;
    /// take it down once the authorization is decided
    fn cleanup(&mut self, id: &Identifier, ch: &Challenge);
}

/// the TXT record's name of dns-01 (RFC 8555 8.4): "_acme-challenge." and the name (a
/// wildcard's without "*.")
pub fn dns01_name(id: &Identifier) -> String {
    let v = id.value();
    format!("_acme-challenge.{}", v.strip_prefix("*.").unwrap_or(&v))
}

/// the TXT record's value of dns-01: base64url of SHA-256 of the key authorization
pub fn dns01_value(key_auth: &str) -> String {
    b64::encode_url(&crypto::sha256(key_auth.as_bytes()))
}

/// http-01 (RFC 8555 8.3): the key authorizations a netio HTTP server answers at
/// /.well-known/acme-challenge/<token> on port 80.  Clones share the tokens: one goes to the
/// solver, one into the server (handler or answer)
#[derive(Clone, Default)]
pub struct Http01 {
    tokens: Arc<Mutex<BTreeMap<String, String>>>,
}

impl Http01 {
    pub fn new() -> Http01 {
        Http01::default()
    }

    /// the answer to a challenge request, None for every other request
    pub fn answer(&self, req: &Request) -> Option<Response> {
        let token = req.path().strip_prefix("/.well-known/acme-challenge/")?;
        if req.method != "GET" && req.method != "HEAD" {
            return None;
        }
        let ka = self.tokens.lock().get(token).cloned()?;
        Some(Response::bytes(200, "application/octet-stream", ka.into_bytes()))
    }

    /// a handler that answers challenges and hands every other request to `next`
    pub fn handler(&self, next: Handler) -> Handler {
        let me = self.clone();
        Arc::new(move |req: &Request| me.answer(req).unwrap_or_else(|| next(req)))
    }
}

impl Solver for Http01 {
    fn kinds(&self) -> Vec<&'static str> {
        alloc::vec!["http-01"]
    }
    fn present(&mut self, _id: &Identifier, ch: &Challenge, key_auth: &str) -> Result<(), String> {
        self.tokens.lock().insert(ch.token.clone(), String::from(key_auth));
        Ok(())
    }
    fn cleanup(&mut self, _id: &Identifier, ch: &Challenge) {
        self.tokens.lock().remove(&ch.token);
    }
}

/// tls-alpn-01 (RFC 8737): a certificate for the challenge, shown on port 443 to a client that
/// offers only "acme-tls/1".  Clones share the certificates: one goes to the solver, one into
/// the server's config (setup).  A name has one challenge at a time: a second one for it fails
/// to be set up until the first is cleaned up
#[derive(Clone, Default)]
pub struct TlsAlpn01 {
    /// the server name -> the challenge's token and certificate
    ids: Arc<Mutex<BTreeMap<String, (String, Identity)>>>,
}

impl TlsAlpn01 {
    pub fn new() -> TlsAlpn01 {
        TlsAlpn01::default()
    }

    /// let `cfg` answer the challenges: "acme-tls/1" among its protocols (last; to a client that
    /// does not offer it alone the server's list is as if it were not there, so a server without
    /// protocols of its own still answers without ALPN), and a select that shows the challenge
    /// certificate for a ClientHello offering only it (before the config's own select)
    pub fn setup(&self, cfg: &mut ServerConfig) {
        if !cfg.alpn.iter().any(|p| p == ACME_TLS_ALPN) {
            cfg.alpn.push(String::from(ACME_TLS_ALPN));
        }
        let me = self.clone();
        let next = cfg.select.take();
        let sel: Select = Arc::new(move |h: &Hello| {
            if h.alpn.len() == 1 && h.alpn[0] == ACME_TLS_ALPN.as_bytes() {
                if let Some((_, id)) = me.ids.lock().get(&h.server_name).cloned() {
                    return Some(id);
                }
            }
            next.as_ref().and_then(|f| f(h))
        });
        cfg.select = Some(sel);
    }

    /// the self-signed certificate of a challenge (RFC 8737 3): the identifier as its only
    /// name, the critical acmeIdentifier with SHA-256 of the key authorization
    pub fn certificate(id: &Identifier, key_auth: &str) -> Option<Identity> {
        let key = PrivateKey::generate_ecdsa(Curve::P256, &mut |b: &mut [u8]| plat::fill_random(b));
        let now = plat::unix_time_ns().div_euclid(1_000_000_000);
        let mut b = CertBuilder { serial: plat::random_bytes::<16>(), cn: String::from("ACME challenge"), not_before: now - 3600, not_after: now + 7 * 86_400, spki: key.spki(), ..CertBuilder::default() };
        match id {
            Identifier::Dns(d) => b.dns.push(d.clone()),
            Identifier::Ip(ip) => b.ips.push(*ip),
        }
        let digest = crypto::sha256(key_auth.as_bytes());
        let ext = x509::extension(OID_ACME_IDENTIFIER, true, &x509::der::octets(&digest));
        let der = b.sign_key(None, &key, &[ext])?;
        Some(Identity::new(alloc::vec![der], key))
    }
}

impl Solver for TlsAlpn01 {
    fn kinds(&self) -> Vec<&'static str> {
        alloc::vec!["tls-alpn-01"]
    }
    fn present(&mut self, id: &Identifier, ch: &Challenge, key_auth: &str) -> Result<(), String> {
        let identity = TlsAlpn01::certificate(id, key_auth).ok_or_else(|| String::from("the challenge certificate cannot be made"))?;
        let mut ids = self.ids.lock();
        match ids.get(&id.tls_name()) {
            Some((token, _)) if *token != ch.token => Err(format!("a tls-alpn-01 challenge for {} is set up already", id.value())),
            _ => {
                ids.insert(id.tls_name(), (ch.token.clone(), identity));
                Ok(())
            }
        }
    }
    fn cleanup(&mut self, id: &Identifier, ch: &Challenge) {
        let mut ids = self.ids.lock();
        if ids.get(&id.tls_name()).is_some_and(|(token, _)| *token == ch.token) {
            ids.remove(&id.tls_name());
        }
    }
}

/// several solvers, each for the challenge types it sets up (a DNS provider for wildcards,
/// http-01 for the rest): the first that sets up a type the authorization offers takes it
pub struct Solvers(pub Vec<Box<dyn Solver + Send>>);

impl Solver for Solvers {
    fn kinds(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        for s in &self.0 {
            for k in s.kinds() {
                if !v.contains(&k) {
                    v.push(k);
                }
            }
        }
        v
    }
    fn present(&mut self, id: &Identifier, ch: &Challenge, key_auth: &str) -> Result<(), String> {
        match self.0.iter_mut().find(|s| s.kinds().contains(&ch.kind.as_str())) {
            Some(s) => s.present(id, ch, key_auth),
            None => Err(format!("no solver for {}", ch.kind)),
        }
    }
    fn cleanup(&mut self, id: &Identifier, ch: &Challenge) {
        if let Some(s) = self.0.iter_mut().find(|s| s.kinds().contains(&ch.kind.as_str())) {
            s.cleanup(id, ch);
        }
    }
}
