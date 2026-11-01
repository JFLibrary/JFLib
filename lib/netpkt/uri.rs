// uri -- URI references as RFC 3986 reads them: the generic parts of an address and the
// resolution of a relative reference, percent encoding, HTML's form encoding.  No scheme's own
// rules (http's default port, say) and no I/O; netio::url is the socket side's absolute URL
// (that crate needs plat), jfn_sim::http::Url the simulator's.
//
//   Uri::parse(s)                  every text is a reference (RFC 3986 appendix B): scheme (lower
//                                  case; only when it has the syntax of one), authority, path,
//                                  query, fragment, all as written
//   u.host() / port() / userinfo() the parts of the authority (an IPv6 address without its
//                                  brackets)
//   base.resolve(&r) / base.join(s)   the target of a reference against a base (5.2.2, strict)
//   remove_dot_segments(path)      "." and ".." out of a path (5.2.4)
//   u.text()                       the reference written again (5.3)
//   u.target()                     path and query as a request line wants them ("/" for no path)
//   u.encoded()                    the same reference with what a URI cannot hold as %XX (spaces,
//                                  umlauts, a lone %); what is escaped already stays
//   percent_encode(bytes, keep) / percent_decode(s) / percent_text(s)   %XX; keep: the bytes that
//                                  stay besides the unreserved ones; percent_decode leaves a bad
//                                  escape as it is, percent_text is UTF-8 with U+FFFD
//   form_encode(pairs) / form_decode(text)   application/x-www-form-urlencoded (HTML, WHATWG URL):
//                                  a space is "+", only * - . _ and alphanumerics stay

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// a URI reference (see the module header)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Uri {
    pub scheme: Option<String>,
    /// what follows "//": [userinfo@]host[:port]
    pub authority: Option<String>,
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

// ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
fn is_scheme(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b[0].is_ascii_alphabetic() && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
}

impl Uri {
    /// a reference: never fails, a text without any of the delimiters is a relative path
    pub fn parse(s: &str) -> Uri {
        let mut u = Uri::default();
        let (rest, frag) = match s.split_once('#') {
            Some((r, f)) => (r, Some(String::from(f))),
            None => (s, None),
        };
        u.fragment = frag;
        let (rest, query) = match rest.split_once('?') {
            Some((r, q)) => (r, Some(String::from(q))),
            None => (rest, None),
        };
        u.query = query;
        let mut rest = rest;
        if let Some((sch, r)) = rest.split_once(':')
            && is_scheme(sch)
        {
            u.scheme = Some(sch.to_ascii_lowercase());
            rest = r;
        }
        if let Some(r) = rest.strip_prefix("//") {
            let end = r.find('/').unwrap_or(r.len());
            u.authority = Some(String::from(&r[..end]));
            rest = &r[end..];
        }
        u.path = String::from(rest);
        u
    }

    /// the user information of the authority
    pub fn userinfo(&self) -> Option<&str> {
        self.authority.as_deref()?.rsplit_once('@').map(|x| x.0)
    }

    // the authority after its user information
    fn hostport(&self) -> Option<&str> {
        let a = self.authority.as_deref()?;
        Some(a.rsplit_once('@').map_or(a, |x| x.1))
    }

    /// the host as written (an IPv6 address without its brackets; "" when the authority has none)
    pub fn host(&self) -> Option<&str> {
        let h = self.hostport()?;
        if let Some(r) = h.strip_prefix('[') {
            return Some(r.split_once(']').map_or(r, |x| x.0));
        }
        Some(h.rsplit_once(':').map_or(h, |x| x.0))
    }

    /// the port (None: not given, or no number up to 65535)
    pub fn port(&self) -> Option<u16> {
        let h = self.hostport()?;
        let p = match h.strip_prefix('[') {
            Some(r) => r.split_once(']')?.1.strip_prefix(':')?,
            None => h.rsplit_once(':')?.1,
        };
        p.parse().ok()
    }

    /// the target of reference `r` against this base (RFC 3986 5.2.2, strict: a scheme equal to
    /// the base's still makes the reference absolute)
    pub fn resolve(&self, r: &Uri) -> Uri {
        let mut t = Uri { fragment: r.fragment.clone(), ..Uri::default() };
        if r.scheme.is_some() {
            t.scheme = r.scheme.clone();
            t.authority = r.authority.clone();
            t.path = remove_dot_segments(&r.path);
            t.query = r.query.clone();
            return t;
        }
        t.scheme = self.scheme.clone();
        if r.authority.is_some() {
            t.authority = r.authority.clone();
            t.path = remove_dot_segments(&r.path);
            t.query = r.query.clone();
            return t;
        }
        t.authority = self.authority.clone();
        if r.path.is_empty() {
            t.path = self.path.clone();
            t.query = r.query.clone().or_else(|| self.query.clone());
        } else {
            t.path = if r.path.starts_with('/') { remove_dot_segments(&r.path) } else { remove_dot_segments(&self.merge(&r.path)) };
            t.query = r.query.clone();
        }
        t
    }

    /// resolve() of a reference given as text
    pub fn join(&self, r: &str) -> Uri {
        self.resolve(&Uri::parse(r))
    }

    // 5.2.3: a relative path against this base's path
    fn merge(&self, path: &str) -> String {
        if self.authority.is_some() && self.path.is_empty() {
            return format!("/{}", path);
        }
        match self.path.rfind('/') {
            Some(i) => format!("{}{}", &self.path[..=i], path),
            None => String::from(path),
        }
    }

    /// the reference written again (5.3)
    pub fn text(&self) -> String {
        let mut s = String::new();
        if let Some(x) = &self.scheme {
            s.push_str(x);
            s.push(':');
        }
        if let Some(x) = &self.authority {
            s.push_str("//");
            s.push_str(x);
        }
        s.push_str(&self.path);
        if let Some(x) = &self.query {
            s.push('?');
            s.push_str(x);
        }
        if let Some(x) = &self.fragment {
            s.push('#');
            s.push_str(x);
        }
        s
    }

    /// the path and query of a request line ("/" for an empty path, no fragment)
    pub fn target(&self) -> String {
        let p = if self.path.is_empty() { "/" } else { self.path.as_str() };
        match &self.query {
            Some(q) => format!("{}?{}", p, q),
            None => String::from(p),
        }
    }

    /// the reference with every character a URI cannot hold as %XX: the path's, the query's and
    /// the fragment's; what is escaped already stays, a % without two hex digits becomes %25
    pub fn encoded(&self) -> Uri {
        let mut u = self.clone();
        u.path = encode_part(&u.path);
        u.query = u.query.map(|x| encode_part(&x));
        u.fragment = u.fragment.map(|x| encode_part(&x));
        u
    }
}

// a part of a URI with its reserved characters left alone
fn encode_part(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(b.len());
    for (i, &c) in b.iter().enumerate() {
        let escape = c == b'%' && i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit();
        if escape || c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' | b':' | b'@' | b'/' | b'?') {
            out.push(c as char);
        } else {
            push_escape(&mut out, c);
        }
    }
    out
}

fn push_escape(out: &mut String, c: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(HEX[(c >> 4) as usize] as char);
    out.push(HEX[(c & 15) as usize] as char);
}

/// "." and ".." out of a path (RFC 3986 5.2.4); ".." never climbs above the root
pub fn remove_dot_segments(path: &str) -> String {
    let mut input = path;
    let mut out = String::with_capacity(path.len());
    while !input.is_empty() {
        if let Some(r) = input.strip_prefix("../").or_else(|| input.strip_prefix("./")) {
            input = r;
        } else if input.starts_with("/./") {
            input = &input[2..];
        } else if input == "/." {
            input = "/";
        } else if input.starts_with("/../") || input == "/.." {
            input = if input.len() > 3 { &input[3..] } else { "/" };
            let cut = out.rfind('/').unwrap_or(0);
            out.truncate(cut);
        } else if input == "." || input == ".." {
            input = "";
        } else {
            // the first segment with its "/" in front
            let from = usize::from(input.starts_with('/'));
            let end = input[from..].find('/').map_or(input.len(), |i| i + from);
            out.push_str(&input[..end]);
            input = &input[end..];
        }
    }
    out
}

/// %XX for every byte but the unreserved ones (RFC 3986 2.3) and those of `keep`
pub fn percent_encode(b: &[u8], keep: &[u8]) -> String {
    let mut out = String::with_capacity(b.len());
    for &c in b {
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~') || keep.contains(&c) {
            out.push(c as char);
        } else {
            push_escape(&mut out, c);
        }
    }
    out
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// the bytes behind %XX; an escape without two hex digits stays as it is
pub fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// percent_decode as text: UTF-8, what is none becomes U+FFFD
pub fn percent_text(s: &str) -> String {
    String::from_utf8_lossy(&percent_decode(s)).into_owned()
}

/// name / value pairs as a form sends them (application/x-www-form-urlencoded)
pub fn form_encode(pairs: &[(String, String)]) -> String {
    let mut out = String::new();
    let part = |s: &str, out: &mut String| {
        for &c in s.as_bytes() {
            if c == b' ' {
                out.push('+');
            } else if c.is_ascii_alphanumeric() || matches!(c, b'*' | b'-' | b'.' | b'_') {
                out.push(c as char);
            } else {
                push_escape(out, c);
            }
        }
    };
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        part(k, &mut out);
        out.push('=');
        part(v, &mut out);
    }
    out
}

/// the name / value pairs of a query or a form's body, decoded in their order ("+" is a space)
pub fn form_decode(text: &str) -> Vec<(String, String)> {
    let dec = |s: &str| percent_text(&s.replace('+', " "));
    text.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (dec(k), dec(v))
        })
        .collect()
}
