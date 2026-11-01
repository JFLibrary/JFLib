// cookie -- HTTP cookies as RFC 6265 reads them: what a Set-Cookie header says, what a user agent
// stores and what its Cookie header sends.  No I/O and no clock: the caller passes the time in, in
// seconds since 1970, so a jar on a simulated clock works like one on a real one
//
//   SetCookie::parse(text)         the header's value (5.2): name, value and the attributes, what
//                                  cannot be read left out; None without a name-value pair
//   parse_date(text)               the tolerant date of an Expires attribute (5.1.1)
//   domain_match(host, domain)     5.1.3; path_match(request, cookie) 5.1.4; default_path(path) 5.1.4
//   canonical_host(host)           lower case, without the dot at the end (5.1.2, no IDNA here)
//   site_of(host)                  what "same site" compares: an address itself, else the last two
//                                  labels of the name (no public suffix list)
//   Origin                         who stores: host, path, secure (https), http (false: a script)
//   Asks                           who reads: host, path, secure, http, the site that asks (None:
//                                  the user), a top-level navigation, a safe method
//   Cookie                         a stored cookie: name, value, domain, host_only, path, expires
//                                  (None: until the session ends), secure, http_only, same_site
//   Jar                            store(text, origin, now) -> Result<(), Refused> (5.3),
//                                  header(asks, now) -> String (5.4: longer paths first, then the
//                                  older ones; "" for none), cookies() in order of creation,
//                                  matching(asks, now) -> the same ones as Cookies, script_text(..),
//                                  purge(now), drop_session(), remove(name, domain, path), clear(),
//                                  bytes()
//
// From RFC 6265bis, as far as a small browser needs it: SameSite (Strict is sent only within the
// site, Lax and no attribute also on a top-level navigation with a safe method, None always but
// only with Secure), a page without https cannot set Secure nor overwrite a Secure cookie, name and
// value together hold at most MAX_COOKIE bytes.  Without a public suffix list a Domain without a dot
// counts as a public suffix (5.3 step 5): the same name as the host makes a host-only cookie, any
// other is refused.  A jar holds at most MAX_PER_DOMAIN cookies of a domain and MAX_COOKIES in all;
// beyond that the oldest go.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::IpAddr;

/// bytes of a cookie's name and value together at most (RFC 6265bis 5.6)
pub const MAX_COOKIE: usize = 4096;
/// cookies of a domain at most (RFC 6265 6.1 asks for at least 50)
pub const MAX_PER_DOMAIN: usize = 50;
/// cookies of a jar at most
pub const MAX_COOKIES: usize = 500;

/// the SameSite attribute
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum SameSite {
    /// none given (reads as Lax)
    #[default]
    Unset,
    Strict,
    Lax,
    None,
}

impl SameSite {
    /// as a header writes it ("" for none)
    pub fn name(self) -> &'static str {
        match self {
            SameSite::Unset => "",
            SameSite::Strict => "Strict",
            SameSite::Lax => "Lax",
            SameSite::None => "None",
        }
    }
}

/// what a Set-Cookie header says (see the module header)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetCookie {
    pub name: String,
    pub value: String,
    /// in lower case without a leading dot
    pub domain: Option<String>,
    /// only one that starts with "/"
    pub path: Option<String>,
    /// seconds since 1970
    pub expires: Option<i64>,
    /// seconds from now; <= 0 means at once
    pub max_age: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
}

fn wsp(s: &str) -> &str {
    s.trim_matches(|c| c == ' ' || c == '\t')
}

impl SetCookie {
    /// a header's value (RFC 6265 5.2); None: no "name=value" with a name (the user agent ignores
    /// the whole header)
    pub fn parse(text: &str) -> Option<SetCookie> {
        let (pair, rest) = text.split_once(';').unwrap_or((text, ""));
        let (name, value) = pair.split_once('=')?;
        let (name, value) = (wsp(name), wsp(value));
        if name.is_empty() {
            return None;
        }
        let mut c = SetCookie { name: String::from(name), value: String::from(value), ..SetCookie::default() };
        for part in rest.split(';') {
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            let (k, v) = (wsp(k), wsp(v));
            if k.eq_ignore_ascii_case("expires") {
                if let Some(t) = parse_date(v) {
                    c.expires = Some(t);
                }
            } else if k.eq_ignore_ascii_case("max-age") {
                if let Some(n) = max_age(v) {
                    c.max_age = Some(n);
                }
            } else if k.eq_ignore_ascii_case("domain") {
                if !v.is_empty() {
                    c.domain = Some(v.strip_prefix('.').unwrap_or(v).to_ascii_lowercase());
                }
            } else if k.eq_ignore_ascii_case("path") {
                c.path = v.starts_with('/').then(|| String::from(v));
            } else if k.eq_ignore_ascii_case("secure") {
                c.secure = true;
            } else if k.eq_ignore_ascii_case("httponly") {
                c.http_only = true;
            } else if k.eq_ignore_ascii_case("samesite") {
                c.same_site = if v.eq_ignore_ascii_case("strict") {
                    SameSite::Strict
                } else if v.eq_ignore_ascii_case("lax") {
                    SameSite::Lax
                } else if v.eq_ignore_ascii_case("none") {
                    SameSite::None
                } else {
                    SameSite::Unset
                };
            }
        }
        Some(c)
    }
}

// a Max-Age value (5.2.2): a digit or "-" first, then digits only; a number too long stays the
// largest there is
fn max_age(v: &str) -> Option<i64> {
    let (neg, digits) = match v.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, v),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut n: i64 = 0;
    for b in digits.bytes() {
        n = n.saturating_mul(10).saturating_add((b - b'0') as i64);
    }
    Some(if neg { -n } else { n })
}

// days since 1970 of a civil date (Howard Hinnant's algorithm)
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

// the first digits of a token, 1 to `max` of them, and that nothing but a non-digit follows
fn leading(t: &str, max: usize) -> Option<(i64, &str)> {
    let n = t.bytes().take_while(|b| b.is_ascii_digit()).count();
    if n == 0 || n > max {
        return None;
    }
    Some((t[..n].parse().ok()?, &t[n..]))
}

// time = 1*2DIGIT ":" 1*2DIGIT ":" 1*2DIGIT ( non-digit *OCTET )
fn time_of(t: &str) -> Option<(i64, i64, i64)> {
    let (h, r) = leading(t, 2)?;
    let (m, r) = leading(r.strip_prefix(':')?, 2)?;
    let (s, _) = leading(r.strip_prefix(':')?, 2)?;
    Some((h, m, s))
}

fn month_of(t: &str) -> Option<i64> {
    let low = t.get(..3)?.to_ascii_lowercase();
    ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"].iter().position(|m| *m == low).map(|i| i as i64 + 1)
}

/// the date of an Expires attribute (RFC 6265 5.1.1) -> seconds since 1970; None: not one
pub fn parse_date(text: &str) -> Option<i64> {
    let delim = |c: char| matches!(c, '\t' | ' '..='/' | ';'..='@' | '['..='`' | '{'..='~');
    let (mut time, mut day, mut month, mut year) = (None, None, None, None);
    for t in text.split(delim).filter(|t| !t.is_empty()) {
        if time.is_none() {
            if let Some(x) = time_of(t) {
                time = Some(x);
                continue;
            }
        }
        if day.is_none() {
            if let Some((d, _)) = leading(t, 2) {
                day = Some(d);
                continue;
            }
        }
        if month.is_none() {
            if let Some(m) = month_of(t) {
                month = Some(m);
                continue;
            }
        }
        if year.is_none() {
            if let Some((y, _)) = leading(t, 4) {
                let digits = t.bytes().take_while(|b| b.is_ascii_digit()).count();
                if digits >= 2 {
                    year = Some(match y {
                        70..=99 => y + 1900,
                        0..=69 => y + 2000,
                        _ => y,
                    });
                }
            }
        }
    }
    let ((h, mi, s), d, mo, y) = (time?, day?, month?, year?);
    if d < 1 || d > days_in_month(y, mo) || y < 1601 || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s)
}

/// a host name in lower case, without the dot at the end
pub fn canonical_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// a host matches a domain (RFC 6265 5.1.3): the same, or a name that ends in ".domain"; an address
/// matches only itself
pub fn domain_match(host: &str, domain: &str) -> bool {
    if host == domain {
        return true;
    }
    host.len() > domain.len() && host.ends_with(domain) && host.as_bytes()[host.len() - domain.len() - 1] == b'.' && IpAddr::parse(host).is_none()
}

/// a request's path matches a cookie's (RFC 6265 5.1.4)
pub fn path_match(path: &str, cookie: &str) -> bool {
    path == cookie || (path.starts_with(cookie) && (cookie.ends_with('/') || path.as_bytes().get(cookie.len()) == Some(&b'/')))
}

/// the path a cookie without one gets from its request's (RFC 6265 5.1.4)
pub fn default_path(path: &str) -> String {
    if !path.starts_with('/') {
        return String::from("/");
    }
    match path.rfind('/') {
        Some(i) if i > 0 => String::from(&path[..i]),
        _ => String::from("/"),
    }
}

/// what "same site" compares: an address itself, else the last two labels of the name
pub fn site_of(host: &str) -> String {
    let h = canonical_host(host);
    if IpAddr::parse(&h).is_some() {
        return h;
    }
    let mut it = h.rmatch_indices('.');
    let _ = it.next();
    match it.next() {
        Some((i, _)) => String::from(&h[i + 1..]),
        None => h,
    }
}

/// a stored cookie
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    /// only the very host gets it (no Domain attribute)
    pub host_only: bool,
    pub path: String,
    /// seconds since 1970; None: until the session ends
    pub expires: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
    /// its place among the cookies: the older first (5.4), the oldest go first
    pub order: u64,
}

impl Cookie {
    pub fn expired(&self, now: i64) -> bool {
        self.expires.is_some_and(|e| e <= now)
    }
}

/// who stores a cookie
#[derive(Clone, Copy, Debug)]
pub struct Origin<'a> {
    pub host: &'a str,
    pub path: &'a str,
    /// the request went over https
    pub secure: bool,
    /// a Set-Cookie header; false: a script of the page
    pub http: bool,
}

/// who reads the cookies
#[derive(Clone, Copy, Debug)]
pub struct Asks<'a> {
    pub host: &'a str,
    pub path: &'a str,
    pub secure: bool,
    /// a request; false: a script of the page
    pub http: bool,
    /// the site of the page that asks (None: the user, by the address line or a reload)
    pub site: Option<&'a str>,
    /// a navigation of the window (a link, a form); false: an image, a sheet, a script's request
    pub top: bool,
    /// GET or HEAD
    pub safe: bool,
}

/// why a jar does not take a cookie
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// no "name=value" with a name
    Syntax,
    /// name and value hold more than MAX_COOKIE bytes
    TooLarge,
    /// the Domain does not fit the host, or is a public suffix
    Domain,
    /// a script may not set HttpOnly nor overwrite such a cookie
    HttpOnly,
    /// Secure from a page without https, or overwriting a Secure cookie from there
    Secure,
    /// SameSite=None without Secure
    SameSiteNone,
}

/// the cookies of a browser (see the module header)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Jar {
    cookies: Vec<Cookie>,
    next: u64,
}

impl Jar {
    pub fn new() -> Jar {
        Jar::default()
    }

    /// all of them in order of creation, expired ones too until purge
    pub fn cookies(&self) -> &[Cookie] {
        &self.cookies
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    /// the bytes its texts hold (an estimate for a budget)
    pub fn bytes(&self) -> usize {
        self.cookies.iter().map(|c| c.name.len() + c.value.len() + c.domain.len() + c.path.len() + 64).sum()
    }

    /// a Set-Cookie value (RFC 6265 5.3) at time `now`
    pub fn store(&mut self, text: &str, o: Origin, now: i64) -> Result<(), Refused> {
        let c = SetCookie::parse(text).ok_or(Refused::Syntax)?;
        self.store_parsed(c, o, now)
    }

    /// the same with what the header says, read already
    pub fn store_parsed(&mut self, c: SetCookie, o: Origin, now: i64) -> Result<(), Refused> {
        if c.name.len() + c.value.len() > MAX_COOKIE {
            return Err(Refused::TooLarge);
        }
        let host = canonical_host(o.host);
        let expires = match c.max_age {
            Some(n) if n <= 0 => Some(i64::MIN),
            Some(n) => Some(now.saturating_add(n)),
            None => c.expires,
        };
        let (domain, host_only) = match c.domain.as_deref().map(canonical_host) {
            Some(d) if !d.contains('.') => {
                // a public suffix: only as the host's own name
                if d != host {
                    return Err(Refused::Domain);
                }
                (host.clone(), true)
            }
            Some(d) => {
                if !domain_match(&host, &d) {
                    return Err(Refused::Domain);
                }
                (d, false)
            }
            None => (host.clone(), true),
        };
        if c.http_only && !o.http {
            return Err(Refused::HttpOnly);
        }
        if c.secure && !o.secure {
            return Err(Refused::Secure);
        }
        if c.same_site == SameSite::None && !c.secure {
            return Err(Refused::SameSiteNone);
        }
        let path = c.path.unwrap_or_else(|| default_path(o.path));
        // a page without https leaves Secure cookies alone (6265bis 5.6 step 16)
        if !o.secure && self.cookies.iter().any(|x| x.secure && x.name == c.name && (domain_match(&x.domain, &domain) || domain_match(&domain, &x.domain)) && path_match(&path, &x.path)) {
            return Err(Refused::Secure);
        }
        let old = self.cookies.iter().position(|x| x.name == c.name && x.domain == domain && x.path == path);
        let mut order = None;
        if let Some(i) = old {
            if self.cookies[i].http_only && !o.http {
                return Err(Refused::HttpOnly);
            }
            order = Some(self.cookies.remove(i).order);
        }
        let cookie = Cookie { name: c.name, value: c.value, domain, host_only, path, expires, secure: c.secure, http_only: c.http_only, same_site: c.same_site, order: order.unwrap_or(self.next) };
        if cookie.expired(now) {
            return Ok(());
        }
        if order.is_none() {
            self.next += 1;
        }
        // its old place among the others
        let at = self.cookies.iter().position(|x| x.order > cookie.order).unwrap_or(self.cookies.len());
        self.cookies.insert(at, cookie);
        self.purge(now);
        self.evict();
        Ok(())
    }

    // the oldest go beyond the limits
    fn evict(&mut self) {
        let mut i = 0;
        while i < self.cookies.len() {
            let d = self.cookies[i].domain.clone();
            if self.cookies.iter().filter(|x| x.domain == d).count() > MAX_PER_DOMAIN {
                self.cookies.remove(i);
            } else {
                i += 1;
            }
        }
        if self.cookies.len() > MAX_COOKIES {
            let extra = self.cookies.len() - MAX_COOKIES;
            self.cookies.drain(..extra);
        }
    }

    /// the cookies of a request or a script (RFC 6265 5.4): longer paths first, then the older
    pub fn matching(&self, a: &Asks, now: i64) -> Vec<&Cookie> {
        let host = canonical_host(a.host);
        let cross = a.site.is_some_and(|s| s != site_of(&host));
        let mut v: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|c| {
                let place = if c.host_only { c.domain == host } else { domain_match(&host, &c.domain) };
                let site = !cross
                    || match c.same_site {
                        SameSite::None => true,
                        SameSite::Strict => false,
                        SameSite::Lax | SameSite::Unset => a.top && a.safe,
                    };
                place && path_match(a.path, &c.path) && (a.secure || !c.secure) && (a.http || !c.http_only) && site && !c.expired(now)
            })
            .collect();
        v.sort_by(|x, y| y.path.len().cmp(&x.path.len()).then(x.order.cmp(&y.order)));
        v
    }

    /// the value of a Cookie header ("" for none)
    pub fn header(&self, a: &Asks, now: i64) -> String {
        let parts: Vec<String> = self.matching(a, now).iter().map(|c| format!("{}={}", c.name, c.value)).collect();
        parts.join("; ")
    }

    /// what a script of the page at this address reads (no HttpOnly cookies)
    pub fn script_text(&self, host: &str, path: &str, secure: bool, now: i64) -> String {
        self.header(&Asks { host, path, secure, http: false, site: None, top: false, safe: true }, now)
    }

    /// the expired ones go
    pub fn purge(&mut self, now: i64) {
        self.cookies.retain(|c| !c.expired(now));
    }

    /// the browser ends: what lasts only for the session goes
    pub fn drop_session(&mut self) {
        self.cookies.retain(|c| c.expires.is_some());
    }

    /// one cookie goes -> there was one
    pub fn remove(&mut self, name: &str, domain: &str, path: &str) -> bool {
        let n = self.cookies.len();
        self.cookies.retain(|c| !(c.name == name && c.domain == domain && c.path == path));
        self.cookies.len() != n
    }

    pub fn clear(&mut self) {
        self.cookies.clear();
    }
}
