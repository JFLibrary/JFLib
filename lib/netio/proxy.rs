// proxy -- which HTTP proxy a request goes through (Client::with_proxy)
//
//   Proxy::new("http://user:pw@proxy.lan:3128")   one proxy for http and https
//   Proxy::none().http(..).https(..)               a proxy per scheme (Endpoint::parse)
//   proxy.no_proxy("localhost, .corp.example, 10.0.0.0/8")
//   Proxy::from_env()    http_proxy (lower case only), https_proxy / HTTPS_PROXY, all_proxy /
//                        ALL_PROXY, no_proxy / NO_PROXY -- as curl reads them: HTTP_PROXY in
//                        upper case is left alone, since a CGI program gets the request's
//                        "Proxy:" header as that variable (httpoxy, CVE-2016-5385 and others)
//   proxy.for_url(&url) -> Option<&Endpoint>
//
// http:// goes to the proxy as an absolute-form request (RFC 9112 3.2.2); https:// through a
// CONNECT tunnel (RFC 9110 9.3.6), in which TLS checks the server itself, not the proxy.  Only
// plain-HTTP proxies: an https:// or socks proxy URL is refused (None).  Loopback addresses and
// localhost never go through a proxy (as Go's net/http): a local service is not sent elsewhere.
// NO_PROXY: "*" for all, names (a leading "." or "*." optional) for themselves and the names
// below them, IP addresses and CIDR ranges for URLs that name an address (no lookup); an entry
// may end in ":port".

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use plat::net::IpAddr;

use crate::url::{percent_decode, Url};

/// a proxy server: where to connect and the credentials it gets (Basic)
#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    /// the Proxy-Authorization header's value ("Basic ..."), when the URL had user:password
    auth: Option<String>,
}

impl core::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // the credentials stay out of logs
        write!(f, "Endpoint({}:{}{})", self.host, self.port, if self.auth.is_some() { ", with credentials" } else { "" })
    }
}

impl Endpoint {
    /// "http://[user[:password]@]host[:port][/]" or "host[:port]" (http assumed); the port
    /// defaults to 80.  None for another scheme (https://, socks5://), a path, or a bad host
    pub fn parse(s: &str) -> Option<Endpoint> {
        let s = s.trim();
        let rest = match s.split_once("://") {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => rest,
            Some(_) => return None,
            None => s,
        };
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains(['/', '?', '#']) {
            return None;
        }
        let (userinfo, hostport) = match rest.rsplit_once('@') {
            Some((u, h)) => (Some(u), h),
            None => (None, rest),
        };
        let u = Url::parse(&format!("http://{}/", hostport))?;
        let auth = userinfo.map(|ui| {
            let (user, pw) = ui.split_once(':').unwrap_or((ui, ""));
            let plain = format!("{}:{}", percent_decode(user, false), percent_decode(pw, false));
            format!("Basic {}", crate::smtp::b64(plain.as_bytes()))
        });
        Some(Endpoint { host: u.host, port: u.port, auth })
    }

    /// with these credentials (Basic: "user:password" in base64)
    pub fn with_auth(mut self, user: &str, password: &str) -> Endpoint {
        self.auth = Some(format!("Basic {}", crate::smtp::b64(format!("{}:{}", user, password).as_bytes())));
        self
    }

    /// the Proxy-Authorization header's value
    pub fn authorization(&self) -> Option<&str> {
        self.auth.as_deref()
    }

    /// the proxy as a URL of its own (where the client connects)
    pub fn url(&self) -> Url {
        Url { scheme: String::from("http"), host: self.host.clone(), port: self.port, target: String::from("/") }
    }

    /// "host:port"
    pub fn key(&self) -> String {
        self.url().authority()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NoProxy {
    All,
    /// a name and the names below it; the port when the entry gave one
    Name(String, Option<u16>),
    /// an address range (an address alone: the full length)
    Net(IpAddr, u8, Option<u16>),
}

fn cidr_match(a: &IpAddr, net: &IpAddr, bits: u8) -> bool {
    let (x, y): (&[u8], &[u8]) = match (a, net) {
        (IpAddr::V4(x), IpAddr::V4(y)) => (x, y),
        (IpAddr::V6(x), IpAddr::V6(y)) => (x, y),
        _ => return false,
    };
    let full = bits as usize / 8;
    if x[..full] != y[..full] {
        return false;
    }
    let rest = bits % 8;
    rest == 0 || (x[full] ^ y[full]) >> (8 - rest) == 0
}

fn no_proxy_entry(e: &str) -> Option<NoProxy> {
    let e = e.trim().to_ascii_lowercase();
    if e.is_empty() {
        return None;
    }
    if e == "*" {
        return Some(NoProxy::All);
    }
    // "[v6]:port", "name:port", "v4:port"; a bare v6 address has colons of its own
    let (body, port) = if let Some(r) = e.strip_prefix('[') {
        let (h, p) = r.split_once(']')?;
        (String::from(h), p.strip_prefix(':').and_then(|p| p.parse().ok()))
    } else if e.matches(':').count() == 1 {
        let (h, p) = e.split_once(':')?;
        (String::from(h), Some(p.parse().ok()?))
    } else {
        (e.clone(), None)
    };
    let (addr, bits) = match body.split_once('/') {
        Some((a, b)) => (a, Some(b.parse::<u8>().ok()?)),
        None => (body.as_str(), None),
    };
    if let Some(ip) = IpAddr::parse(addr) {
        let max = if matches!(ip, IpAddr::V4(_)) { 32 } else { 128 };
        let bits = bits.unwrap_or(max);
        return (bits <= max).then_some(NoProxy::Net(ip, bits, port));
    }
    if bits.is_some() {
        return None;
    }
    let name = body.trim_start_matches("*.").trim_start_matches('.').trim_end_matches('.');
    (!name.is_empty()).then(|| NoProxy::Name(String::from(name), port))
}

/// the proxies of a client: one per scheme, and the hosts that go direct
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Proxy {
    http: Option<Endpoint>,
    https: Option<Endpoint>,
    no: Vec<NoProxy>,
}

impl Proxy {
    /// no proxy (to add them one by one)
    pub fn none() -> Proxy {
        Proxy::default()
    }

    /// one proxy for http and https (Endpoint::parse) -> None when it is not one
    pub fn new(url: &str) -> Option<Proxy> {
        let e = Endpoint::parse(url)?;
        Some(Proxy { http: Some(e.clone()), https: Some(e), no: Vec::new() })
    }

    /// the proxy for http:// URLs
    pub fn http(mut self, e: Endpoint) -> Proxy {
        self.http = Some(e);
        self
    }

    /// the proxy for https:// URLs (a CONNECT tunnel)
    pub fn https(mut self, e: Endpoint) -> Proxy {
        self.https = Some(e);
        self
    }

    /// hosts that go direct: a comma-separated list as NO_PROXY writes it (entries that are not
    /// one are left out)
    pub fn no_proxy(mut self, list: &str) -> Proxy {
        self.no.extend(list.split(',').filter_map(no_proxy_entry));
        self
    }

    /// the proxies the environment names (see the module header); None when it names none
    pub fn from_env() -> Option<Proxy> {
        Proxy::from_vars(&|name| plat::env(name))
    }

    /// from_env with the variables from `get` (tests, or a configuration of the app's own)
    pub fn from_vars(get: &dyn Fn(&str) -> Option<String>) -> Option<Proxy> {
        let var = |names: &[&str]| names.iter().find_map(|n| get(n).filter(|v| !v.trim().is_empty()));
        let all = var(&["all_proxy", "ALL_PROXY"]).and_then(|v| Endpoint::parse(&v));
        // http_proxy in lower case only (httpoxy)
        let http = var(&["http_proxy"]).and_then(|v| Endpoint::parse(&v)).or_else(|| all.clone());
        let https = var(&["https_proxy", "HTTPS_PROXY"]).and_then(|v| Endpoint::parse(&v)).or(all);
        if http.is_none() && https.is_none() {
            return None;
        }
        let p = Proxy { http, https, no: Vec::new() };
        Some(match var(&["no_proxy", "NO_PROXY"]) {
            Some(list) => p.no_proxy(&list),
            None => p,
        })
    }

    /// whether a URL goes direct: loopback, localhost, or an entry of no_proxy
    pub fn bypass(&self, url: &Url) -> bool {
        let host = url.host.trim_end_matches('.');
        let ip = IpAddr::parse(host.split('%').next().unwrap_or(host));
        if ip.is_some_and(|a| a.is_loopback()) || host == "localhost" || host.ends_with(".localhost") {
            return true;
        }
        self.no.iter().any(|n| match n {
            NoProxy::All => true,
            NoProxy::Name(name, port) => {
                port.is_none_or(|p| p == url.port) && (host == name || (host.len() > name.len() && host.ends_with(name.as_str()) && host.as_bytes()[host.len() - name.len() - 1] == b'.'))
            }
            NoProxy::Net(net, bits, port) => port.is_none_or(|p| p == url.port) && ip.is_some_and(|a| cidr_match(&a, net, *bits)),
        })
    }

    /// the proxy a request to `url` goes through (None: direct)
    pub fn for_url(&self, url: &Url) -> Option<&Endpoint> {
        if self.bypass(url) {
            return None;
        }
        if url.is_https() { self.https.as_ref() } else { self.http.as_ref() }
    }
}
