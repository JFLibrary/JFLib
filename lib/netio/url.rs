// url -- absolute http(s) URLs, references resolved against them, percent encoding, query
// strings
//
//   Url::parse("https://host:9443/api?x=1")   the host checked: a name of letters, digits,
//        - . _ ~ (no IDN: give its A-label, "xn--..."), an IPv4 address in dotted form, or an IPv6
//        literal in brackets; a name that ends in a number must be a dotted IPv4 address (no
//        "0x7f.1" or "2130706433", which some resolvers read as 127.0.0.1); no user:password@
//   url.join(reference)   RFC 3986 5.2 (netpkt::uri): "https://other/x", "//host/x", "/x", "../x"
//   url.same_origin(&other), url.authority() ("host:port", for CONNECT), url.path()
//
// Bytes beyond ASCII and " < > ` { } in a path or query are percent-encoded; a fragment is
// dropped (it never goes to a server).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// an absolute URL: scheme://host[:port][/path][?query]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    /// "http" or "https" (lower case)
    pub scheme: String,
    /// host name or IP literal (IPv6 without brackets)
    pub host: String,
    pub port: u16,
    /// the path and query ("/" at least)
    pub target: String,
}

/// whether a name's last label is a number (digits, or 0x and hex digits): such a name is an IPv4
/// address in one of the forms inet_aton reads, or nothing
fn ends_in_number(host: &str) -> bool {
    let h = host.strip_suffix('.').unwrap_or(host);
    let last = h.rsplit('.').next().unwrap_or("");
    if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    let hex = last.strip_prefix("0x").or_else(|| last.strip_prefix("0X"));
    hex.is_some_and(|x| x.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// a host as a URL may name it (lower case; IPv6 without brackets) -> None when it is not one
fn check_host(auth_host: &str, bracketed: bool) -> Option<String> {
    if bracketed {
        // an IPv6 literal, with a zone as RFC 6874 writes it ("fe80::1%25eth0")
        let h = auth_host.replacen("%25", "%", 1);
        let (addr, zone) = h.split_once('%').map_or((h.as_str(), None), |(a, z)| (a, Some(z)));
        let ok = matches!(plat::net::IpAddr::parse(addr), Some(plat::net::IpAddr::V6(_)))
            && zone.is_none_or(|z| !z.is_empty() && z.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b)));
        return ok.then(|| h.to_ascii_lowercase());
    }
    let h = auth_host;
    if h.is_empty() || h.len() > 253 || h.starts_with('.') || !h.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b)) {
        return None;
    }
    if ends_in_number(h) && !matches!(plat::net::IpAddr::parse(h.strip_suffix('.').unwrap_or(h)), Some(plat::net::IpAddr::V4(_))) {
        return None;
    }
    Some(h.to_ascii_lowercase())
}

/// a path and query as they go into a request: bytes beyond ASCII and " < > ` { } percent-encoded
/// (what is already encoded stays as it is)
fn encode_target(t: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(t.len());
    for b in t.bytes() {
        if b >= 0x80 || b"\"<>`{}".contains(&b) {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 15) as usize] as char);
        } else {
            out.push(b as char);
        }
    }
    out
}

impl Url {
    /// parse; the port defaults to 80 / 443
    pub fn parse(s: &str) -> Option<Url> {
        let (scheme, rest) = s.split_once("://")?;
        let rest = rest.split('#').next().unwrap_or("");
        if rest.bytes().any(|b| b <= b' ' || b == 0x7f || b == b'\\') {
            return None;
        }
        let scheme = scheme.to_ascii_lowercase();
        let default = match scheme.as_str() {
            "http" => 80,
            "https" => 443,
            _ => return None,
        };
        let (auth, target) = match rest.find(['/', '?']) {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        if auth.is_empty() || auth.contains('@') {
            return None;
        }
        let port_of = |p: &str| -> Option<u16> {
            if p.is_empty() {
                return Some(default); // "host:" is the default port (RFC 3986 3.2.3)
            }
            if !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            p.parse::<u16>().ok().filter(|&n| n != 0)
        };
        let (host, port) = if let Some(r) = auth.strip_prefix('[') {
            let (h, p) = r.split_once(']')?;
            let port = match p.strip_prefix(':') {
                Some(n) => port_of(n)?,
                None if p.is_empty() => default,
                None => return None,
            };
            (check_host(h, true)?, port)
        } else {
            match auth.rsplit_once(':') {
                Some((h, p)) => (check_host(h, false)?, port_of(p)?),
                None => (check_host(auth, false)?, default),
            }
        };
        let target = if target.starts_with('?') { format!("/{}", target) } else { String::from(target) };
        Some(Url { scheme, host, port, target: encode_target(&target) })
    }

    pub fn is_https(&self) -> bool {
        self.scheme == "https"
    }

    /// the host as a URL writes it: IPv6 in brackets
    fn host_text(&self) -> String {
        if self.host.contains(':') { format!("[{}]", self.host.replacen('%', "%25", 1)) } else { self.host.clone() }
    }

    /// "host:port" for the Host header (port left out when it is the default)
    pub fn host_header(&self) -> String {
        let h = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if (self.port == 80 && !self.is_https()) || (self.port == 443 && self.is_https()) { h } else { format!("{}:{}", h, self.port) }
    }

    /// "host:port" with the port always (a CONNECT request's target)
    pub fn authority(&self) -> String {
        let h = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        format!("{}:{}", h, self.port)
    }

    /// the URL with another target ("/api/jobs")
    pub fn with_target(&self, target: &str) -> Url {
        Url { target: String::from(target), ..self.clone() }
    }

    /// "https://host:port"
    pub fn origin(&self) -> String {
        format!("{}://{}", self.scheme, self.host_header())
    }

    /// the same scheme, host and port (RFC 6454): what a redirect must keep for credentials to
    /// go along
    pub fn same_origin(&self, other: &Url) -> bool {
        self.scheme == other.scheme && self.host == other.host && self.port == other.port
    }

    /// the path without the query
    pub fn path(&self) -> &str {
        split_target(&self.target).0
    }

    /// a reference resolved against this URL as RFC 3986 5.2 does it (netpkt::uri): absolute
    /// ("https://other/x"), network-path ("//host/x"), absolute path ("/x"), relative path ("x",
    /// "../x"), query ("?q") or empty; dot segments removed, the fragment dropped.  None for
    /// another scheme, a reference with spaces or controls, or a URL Url::parse refuses
    pub fn join(&self, reference: &str) -> Option<Url> {
        if reference.bytes().any(|b| b <= b' ' || b == 0x7f || b == b'\\') {
            return None;
        }
        let base = netpkt::uri::Uri::parse(&format!("{}", self));
        let t = netpkt::uri::Uri { fragment: None, ..base.join(reference) };
        Url::parse(&t.text())
    }
}

impl core::fmt::Display for Url {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let port = if (self.port == 80 && !self.is_https()) || (self.port == 443 && self.is_https()) { String::new() } else { format!(":{}", self.port) };
        write!(f, "{}://{}{}{}", self.scheme, self.host_text(), port, self.target)
    }
}

/// percent-encode everything but the unreserved characters (RFC 3986)
pub fn percent_encode(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 15) as usize] as char);
        }
    }
    out
}

/// decode %XX (and '+' as a space in query strings when `plus`); invalid escapes stay as they
/// are, invalid UTF-8 becomes U+FFFD
pub fn percent_decode(s: &str, plus: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |c: u8| (c as char).to_digit(16);
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                        continue;
                    }
                    _ => out.push(b'%'),
                }
            }
            b'+' if plus => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// the (key, value) pairs of a query string ("a=1&b=x%20y"), decoded
pub fn query_pairs(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k, true), percent_decode(v, true))
        })
        .collect()
}

/// split a target into its path and query ("" when there is none)
pub fn split_target(target: &str) -> (&str, &str) {
    target.split_once('?').unwrap_or((target, ""))
}
