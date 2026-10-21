// url -- absolute http(s) URLs, percent encoding, query strings

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

impl Url {
    /// parse; the port defaults to 80 / 443
    pub fn parse(s: &str) -> Option<Url> {
        let (scheme, rest) = s.split_once("://")?;
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
        let (host, port) = if let Some(r) = auth.strip_prefix('[') {
            let (h, p) = r.split_once(']')?;
            let port = match p.strip_prefix(':') {
                Some(n) => n.parse().ok()?,
                None if p.is_empty() => default,
                None => return None,
            };
            (String::from(h), port)
        } else {
            match auth.rsplit_once(':') {
                Some((h, p)) => (String::from(h), p.parse().ok()?),
                None => (String::from(auth), default),
            }
        };
        if host.is_empty() {
            return None;
        }
        let target = if target.starts_with('?') { format!("/{}", target) } else { String::from(target) };
        Some(Url { scheme, host: host.to_ascii_lowercase(), port, target })
    }

    pub fn is_https(&self) -> bool {
        self.scheme == "https"
    }

    /// "host:port" for the Host header (port left out when it is the default)
    pub fn host_header(&self) -> String {
        let h = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if (self.port == 80 && !self.is_https()) || (self.port == 443 && self.is_https()) { h } else { format!("{}:{}", h, self.port) }
    }

    /// the URL with another target ("/api/jobs")
    pub fn with_target(&self, target: &str) -> Url {
        Url { target: String::from(target), ..self.clone() }
    }

    /// "https://host:port"
    pub fn origin(&self) -> String {
        format!("{}://{}", self.scheme, self.host_header())
    }
}

impl core::fmt::Display for Url {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}{}", self.origin(), self.target)
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
