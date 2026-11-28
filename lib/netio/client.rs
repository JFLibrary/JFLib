// client -- a blocking HTTP/1.1 client
//
//   Client::new().request("GET", &url, &headers, body) -> Result<Response, HttpError>
//   client.fetch(method, &url, &headers, body) -> Fetched { url: the last one, response }
//   client.get_to(&url, &headers, max, on)   a download piece by piece (Part::Head, Part::Data)
//   client.request_on(&stream, method, &url, &headers, body)   one request over a connection the
//                                            caller opened (no proxy, no pool, no redirects)
//   client.with_connector(Arc<dyn Fn(&Url, TcpStream) -> Result<Box<dyn Stream>, HttpError>>)
//   client.with_proxy(Proxy)          HTTP proxies (netio::proxy; Proxy::from_env)
//   client.with_pool(Arc<Pool>)       connections kept open for the next requests
//   client.with_cookies(Arc<Mutex<Jar>>)   netpkt::cookie's jar: Set-Cookie stored, Cookie sent
//   client.with_redirect_check(f)     f(from, to) before each redirect (false: the 3xx answer)
//   client.request_async(method, &url, &headers, body) -> future of the answer (the request on
//   plat::task's blocking pool: ui.spawn / block_on await it without blocking)
//   client.download(&url, &headers, path, &ctl) -> future of the status: the body into the file
//   (plat::AtomicFile: replaced only by a whole 2xx body), progress and cancel in ctl
//
// The fields: connect_ms, timeout_ms (any single read or write), deadline_ms (a whole request,
// its redirects included; 0: none), max_body (the body as it came and after decoding),
// user_agent, redirects (followed at most; 0: a 3xx answer comes back as it is) and decode.
//
// decode (on by default): without an Accept-Encoding or Range header of the caller's, a request
// offers "gzip, br" and a body in gzip, br, zstd (window up to 8 MiB, RFC 9659) or deflate (zlib)
// is decoded as it arrives, through compress's stream decoders; Content-Encoding and
// Content-Length then leave the answer's headers.  A body that does not decode is
// HttpError::Encoding; stacked or unknown codings come back as they are.
//
// Redirects (301, 302, 303, 307, 308 with a Location, RFC 9110 15.4): the Location is resolved
// against the request's URL (Url::join); from https to http is refused (HttpError::Redirect);
// 303 (not after HEAD) and 301 / 302 after POST go on as GET without the body and its headers;
// to another origin, Authorization, Cookie, Proxy-Authorization and Host stay behind (the jar's
// cookies are those of the new URL).
//
// Answers are read as RFC 9112 6.3 says: a status line "HTTP/1.x NNN" with a code of 100 to 599,
// interim answers (1xx) skipped up to 16 of them, 101 refused (no upgrade was asked for), the
// body by Transfer-Encoding: chunked, else Content-Length, else up to the end of the connection.
// A connection carries another request (the pool, Session) only after a body of known length
// that was read to its end with no byte beyond it, and when neither side said close.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::future::Future;

use compress::stream::{Decode, Status};
use netpkt::cookie::{Asks, Jar, Origin};
use plat::aio::Ctl;
use plat::net::{NetError, NetErrorKind, TcpStream};
use plat::sync::Mutex;
use plat::task::CancelToken;

use crate::http::{self, Body, Headers, ReadError, Reader, Response};
use crate::proxy::{Endpoint, Proxy};
use crate::url::Url;
use crate::Stream;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HttpError {
    /// not an http(s) URL, or https without a connector
    Url(String),
    Net(NetError),
    /// the TLS layer refused (certificate, handshake)
    Tls(String),
    /// a malformed answer
    Bad,
    /// an answer over the size limit
    TooLarge,
    /// (download) the target file could not be created, written or put in place
    File,
    /// (download) cancelled through the Ctl, or the future was dropped
    Cancelled,
    /// a redirect that is not followed: from https to http, a Location that is not a URL, more
    /// than Client::redirects of them
    Redirect(String),
    /// the proxy refused the tunnel (its status; 407: it wants other credentials)
    Proxy(u16),
    /// the body does not decode as its Content-Encoding says
    Encoding,
}

impl core::fmt::Display for HttpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HttpError::Url(s) => write!(f, "invalid address: {}", s),
            HttpError::Net(e) => write!(f, "{}", e),
            HttpError::Tls(s) => write!(f, "TLS: {}", s),
            HttpError::Bad => f.write_str("malformed HTTP answer"),
            HttpError::TooLarge => f.write_str("answer too large"),
            HttpError::File => f.write_str("the file cannot be written"),
            HttpError::Cancelled => f.write_str("cancelled"),
            HttpError::Redirect(s) => write!(f, "redirect: {}", s),
            HttpError::Proxy(407) => f.write_str("the proxy wants credentials (407)"),
            HttpError::Proxy(s) => write!(f, "the proxy refused the connection ({})", s),
            HttpError::Encoding => f.write_str("the body does not decode as its Content-Encoding says"),
        }
    }
}

impl From<NetError> for HttpError {
    fn from(e: NetError) -> Self {
        HttpError::Net(e)
    }
}

pub type Connector = Arc<dyn Fn(&Url, TcpStream) -> Result<Box<dyn Stream>, HttpError> + Send + Sync>;

/// asked before a redirect is followed: (from, to) -> false: the 3xx answer is the result
pub type RedirectCheck = Arc<dyn Fn(&Url, &Url) -> bool + Send + Sync>;

/// a piece of an answer that Client::get_to streams
pub enum Part<'a> {
    /// the status and the headers (first)
    Head(u16, &'a Headers),
    /// the next bytes of the body
    Data(&'a [u8]),
}

/// an answer and the URL it came from (after redirects)
#[derive(Debug)]
pub struct Fetched {
    pub url: Url,
    pub response: Response,
}

/// interim answers (1xx) read before the final one at most
const MAX_INTERIM: usize = 16;
/// the body of a redirect that is followed: read (so the connection can go on) up to this
const REDIRECT_BODY: u64 = 64 << 10;
/// what a request offers when it decodes
const ACCEPT_ENCODING: &str = "gzip, br";
/// zstd's window in HTTP (RFC 9659 3: a decoder need not take more than 8 MiB)
const ZSTD_WINDOW: usize = 8 << 20;

#[derive(Clone)]
pub struct Client {
    pub connect_ms: u32,
    /// the longest wait for any read or write
    pub timeout_ms: u32,
    /// the longest a whole request may take, from connecting to the last byte of the body,
    /// redirects included (0: no limit beyond timeout_ms per read)
    pub deadline_ms: u32,
    pub max_body: usize,
    pub user_agent: String,
    /// redirects followed at most (0: a 3xx answer comes back as it is)
    pub redirects: u8,
    /// offer gzip and br and decode the body (see the module header)
    pub decode: bool,
    connector: Option<Connector>,
    proxy: Option<Proxy>,
    pool: Option<Arc<Pool>>,
    cookies: Option<Arc<Mutex<Jar>>>,
    redirect_check: Option<RedirectCheck>,
}

impl Default for Client {
    fn default() -> Self {
        Client::new()
    }
}

/// one request as it goes out
struct Hop<'a> {
    method: &'a str,
    url: &'a Url,
    headers: &'a Headers,
    body: &'a [u8],
}

/// the head of the final answer
struct Head {
    /// x of HTTP/1.x
    minor: u8,
    status: u16,
    headers: Headers,
}

/// "HTTP/1.x NNN[ reason]" -> (x, NNN)
fn status_line(start: &str) -> Option<(u8, u16)> {
    let b = start.strip_prefix("HTTP/1.")?.as_bytes();
    if b.len() < 5 || !b[0].is_ascii_digit() || b[1] != b' ' || !b[2..5].iter().all(u8::is_ascii_digit) || (b.len() > 5 && b[5] != b' ') {
        return None;
    }
    let status = (b[2] - b'0') as u16 * 100 + (b[3] - b'0') as u16 * 10 + (b[4] - b'0') as u16;
    (100..=599).contains(&status).then_some((b[0] - b'0', status))
}

/// the final answer's head, after up to MAX_INTERIM interim ones
fn read_head(rd: &mut Reader<'_>) -> Result<Head, ReadError> {
    for i in 0..MAX_INTERIM {
        // only an end before the first answer is a closed connection (a reused one, to retry)
        let (start, headers) = rd.head().map_err(|e| if i > 0 && e == ReadError::Closed { ReadError::Bad } else { e })?;
        let (minor, status) = status_line(&start).ok_or(ReadError::Bad)?;
        if status == 101 {
            return Err(ReadError::Bad); // no upgrade was asked for
        }
        if status >= 200 {
            return Ok(Head { minor, status, headers });
        }
    }
    Err(ReadError::Bad)
}

fn read_err(e: ReadError) -> HttpError {
    match e {
        ReadError::Net(n) => HttpError::Net(n),
        ReadError::BodyTooLarge | ReadError::HeadTooLarge => HttpError::TooLarge,
        ReadError::Closed | ReadError::Bad => HttpError::Bad,
    }
}

fn reset() -> HttpError {
    HttpError::Net(NetError::new(NetErrorKind::Reset, 0))
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn idempotent(method: &str) -> bool {
    matches!(method, "GET" | "HEAD" | "PUT" | "DELETE" | "OPTIONS" | "TRACE")
}

/// the decoder of a body's Content-Encoding (None: identity, or one this client does not decode)
/// and whether its format may hold several streams in a row (gzip members, zstd frames)
fn decoder_of(h: &Headers) -> Option<(Box<dyn Decode>, bool)> {
    let mut codings = h
        .0
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("content-encoding"))
        .flat_map(|(_, v)| v.split(','))
        .map(|t| t.trim())
        .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("identity"));
    let c = codings.next()?;
    if codings.next().is_some() {
        return None; // stacked codings: left as they are
    }
    let c = c.to_ascii_lowercase();
    Some(match c.as_str() {
        "gzip" | "x-gzip" => (Box::new(compress::gzip::Decoder::new()), true),
        "br" => (Box::new(compress::brotli::Decoder::new()), false),
        "zstd" => (Box::new(compress::zstd::Decoder::new().with_window_max(ZSTD_WINDOW)), true),
        // "deflate" is zlib (RFC 9110 8.4.1.2)
        "deflate" => (Box::new(compress::zlib::Decoder::new()), false),
        _ => return None,
    })
}

/// a body's bytes through its decoder into `sink` (at most `max` decoded bytes in all)
struct Decoding {
    dec: Box<dyn Decode>,
    multi: bool,
    buf: Vec<u8>,
    total: u64,
    max: u64,
    ended: bool,
    fail: Option<HttpError>,
}

impl Decoding {
    fn feed(&mut self, mut input: &[u8], sink: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), HttpError> {
        loop {
            if self.ended {
                if input.is_empty() {
                    return Ok(());
                }
                if !self.multi {
                    return Err(HttpError::Encoding); // bytes after the stream
                }
                self.ended = false; // the next gzip member or zstd frame
            }
            let s = self.dec.decode(input, &mut self.buf).map_err(|_| HttpError::Encoding)?;
            input = &input[s.read..];
            if s.written > 0 {
                self.total += s.written as u64;
                if self.total > self.max {
                    return Err(HttpError::TooLarge);
                }
                if !sink(&self.buf[..s.written]) {
                    return Err(HttpError::Bad);
                }
            }
            match s.status {
                Status::End if s.read == 0 && s.written == 0 && !input.is_empty() && self.multi => return Err(HttpError::Encoding),
                Status::End => self.ended = true,
                Status::Output => {}
                Status::Input => return Ok(()),
            }
        }
    }
}

/// the body of an answer as a reader of it takes it: framed (chunked, Content-Length, to the
/// end), decoded where the client asked for it, at most `max` bytes either way
struct Incoming<'r, 's> {
    rd: &'r mut Reader<'s>,
    /// the headers as they came (the framing)
    raw: &'r Headers,
    until_close: bool,
    dec: Option<(Box<dyn Decode>, bool)>,
    /// no body at all (HEAD, 204, 304)
    none: bool,
    /// read to its end
    done: bool,
}

impl Incoming<'_, '_> {
    /// the body piece by piece to `sink` (false from it: stop, Err(Bad)) -> its length (decoded)
    fn to(&mut self, max: u64, sink: &mut dyn FnMut(&[u8]) -> bool) -> Result<u64, HttpError> {
        if self.none {
            self.done = true;
            return Ok(0);
        }
        let Some((dec, multi)) = self.dec.take() else {
            let n = self.rd.body_to(self.raw, max, self.until_close, sink).map_err(read_err)?;
            self.done = true;
            return Ok(n);
        };
        let mut d = Decoding { dec, multi, buf: vec![0u8; 16 << 10], total: 0, max, ended: false, fail: None };
        let mut raw = 0u64;
        let r = self.rd.body_to(self.raw, max, self.until_close, &mut |piece| match {
            raw += piece.len() as u64;
            d.feed(piece, sink)
        } {
            Ok(()) => true,
            Err(e) => {
                d.fail = Some(e);
                false
            }
        });
        if let Some(e) = d.fail.take() {
            return Err(e);
        }
        r.map_err(read_err)?;
        // the body ended: the stream must have ended with it (all its output is written); an
        // empty body is empty, whatever its Content-Encoding says (servers send that for nothing)
        if raw > 0 && !d.ended && d.dec.finish().is_err() {
            return Err(HttpError::Encoding);
        }
        self.done = true;
        Ok(d.total)
    }

    /// the whole body (at most max bytes)
    fn all(&mut self, max: usize) -> Result<Vec<u8>, HttpError> {
        let mut out = Vec::new();
        if self.dec.is_none() && self.raw.get("transfer-encoding").is_none() {
            if let Ok(Some(n)) = self.raw.content_length() {
                out.reserve((n.min(max as u64) as usize).min(1 << 20));
            }
        }
        self.to(max as u64, &mut |b| {
            out.extend_from_slice(b);
            true
        })?;
        Ok(out)
    }
}

type ReadFn<'f> = &'f mut dyn FnMut(u16, &Headers, &mut Incoming<'_, '_>) -> Result<Response, HttpError>;

/// a kept connection: the stream and the proxy it goes through as absolute-form requests
struct Idle {
    key: String,
    conn: Box<dyn Stream>,
    via: Option<Endpoint>,
    since_us: u64,
}

/// connections kept open between requests (Client::with_pool): per origin, proxy and TLS
/// layer, at most `per_host` of them and `idle_ms` long
pub struct Pool {
    idle: Mutex<Vec<Idle>>,
    pub per_host: usize,
    pub max_idle: usize,
    pub idle_ms: u32,
}

impl Pool {
    /// 4 per host, 64 in all, 60 s
    pub fn new() -> Arc<Pool> {
        Pool::with_limits(4, 64, 60_000)
    }

    pub fn with_limits(per_host: usize, max_idle: usize, idle_ms: u32) -> Arc<Pool> {
        Arc::new(Pool { idle: Mutex::new(Vec::new()), per_host, max_idle, idle_ms })
    }

    /// the connections waiting now
    pub fn idle(&self) -> usize {
        self.idle.lock().len()
    }

    /// close them all
    pub fn clear(&self) {
        self.idle.lock().clear();
    }

    fn take(&self, key: &str) -> Option<(Box<dyn Stream>, Option<Endpoint>)> {
        let now = plat::time_us();
        let mut v = self.idle.lock();
        v.retain(|c| now.saturating_sub(c.since_us) < self.idle_ms as u64 * 1000);
        while let Some(i) = v.iter().rposition(|c| c.key == key) {
            let c = v.remove(i);
            // bytes or an end while idle: the server closed it, or sent what nobody asked for
            if !c.conn.wait_readable(0) {
                return Some((c.conn, c.via));
            }
        }
        None
    }

    fn put(&self, key: String, conn: Box<dyn Stream>, via: Option<Endpoint>) {
        if self.per_host == 0 || self.max_idle == 0 {
            return;
        }
        let mut v = self.idle.lock();
        if v.iter().filter(|c| c.key == key).count() >= self.per_host {
            if let Some(i) = v.iter().position(|c| c.key == key) {
                v.remove(i);
            }
        }
        if v.len() >= self.max_idle {
            v.remove(0);
        }
        v.push(Idle { key, conn, via, since_us: plat::time_us() });
    }
}

impl Client {
    pub fn new() -> Client {
        Client {
            connect_ms: 5000,
            timeout_ms: 30_000,
            deadline_ms: 0,
            max_body: 64 << 20,
            user_agent: String::from("JFLib-netio"),
            redirects: 0,
            decode: true,
            connector: None,
            proxy: None,
            pool: None,
            cookies: None,
            redirect_check: None,
        }
    }

    /// the TLS layer for https URLs
    pub fn with_connector(mut self, c: Connector) -> Client {
        self.connector = Some(c);
        self
    }

    /// requests go through these proxies (netio::proxy)
    pub fn with_proxy(mut self, p: Proxy) -> Client {
        self.proxy = Some(p);
        self
    }

    /// connections stay open for the next requests (keep-alive); clones of the client share them
    pub fn with_pool(mut self, p: Arc<Pool>) -> Client {
        self.pool = Some(p);
        self
    }

    /// cookies: what answers set is stored in the jar, a request without a Cookie header of the
    /// caller's sends the jar's for its URL (netpkt::cookie, with the public suffix list)
    pub fn with_cookies(mut self, jar: Arc<Mutex<Jar>>) -> Client {
        self.cookies = Some(jar);
        self
    }

    /// redirects: followed up to `n` (0: none)
    pub fn with_redirects(mut self, n: u8) -> Client {
        self.redirects = n;
        self
    }

    /// each redirect first asks f(from, to); false: the 3xx answer is the result
    pub fn with_redirect_check(mut self, f: RedirectCheck) -> Client {
        self.redirect_check = Some(f);
        self
    }

    /// the jar given to with_cookies
    pub fn cookies(&self) -> Option<&Arc<Mutex<Jar>>> {
        self.cookies.as_ref()
    }

    fn deadline(&self) -> Option<u64> {
        (self.deadline_ms > 0).then(|| plat::time_us() + self.deadline_ms as u64 * 1000)
    }

    // a TCP connection, within the deadline
    fn tcp(&self, url: &Url, deadline: Option<u64>) -> Result<TcpStream, HttpError> {
        let mut ms = self.connect_ms;
        if let Some(d) = deadline {
            let left = d.saturating_sub(plat::time_us()) / 1000;
            if left == 0 {
                return Err(HttpError::Net(NetError::new(NetErrorKind::TimedOut, 0)));
            }
            ms = ms.min(left.min(u32::MAX as u64) as u32);
        }
        let addrs = crate::dns::resolve(&url.host, url.port)?;
        let tcp = TcpStream::connect_any(&addrs, ms)?;
        let _ = tcp.set_nodelay(true);
        tcp.set_read_timeout(Some(self.timeout_ms));
        tcp.set_write_timeout(Some(self.timeout_ms));
        Ok(tcp)
    }

    fn tls(&self, url: &Url, tcp: TcpStream) -> Result<Box<dyn Stream>, HttpError> {
        let c = self.connector.as_ref().ok_or_else(|| HttpError::Url(String::from("https needs a TLS connector")))?;
        c(url, tcp)
    }

    /// open the connection a request to `url` goes over, directly (no proxy)
    pub fn connect(&self, url: &Url) -> Result<Box<dyn Stream>, HttpError> {
        let tcp = self.tcp(url, None)?;
        if url.is_https() { self.tls(url, tcp) } else { Ok(Box::new(tcp)) }
    }

    // a CONNECT tunnel through the proxy to url's host and port (RFC 9110 9.3.6)
    fn tunnel(&self, tcp: &TcpStream, ep: &Endpoint, url: &Url, deadline: Option<u64>) -> Result<(), HttpError> {
        let mut h = Headers::new();
        h.set("Host", &url.authority());
        h.set("User-Agent", &self.user_agent);
        if let Some(a) = ep.authorization() {
            h.set("Proxy-Authorization", a);
        }
        if !http::write_request(tcp, "CONNECT", &url.authority(), &h, &[]) {
            return Err(reset());
        }
        let mut rd = Reader::new(tcp);
        rd.set_deadline(deadline);
        let head = read_head(&mut rd).map_err(read_err)?;
        if !(200..300).contains(&head.status) {
            return Err(HttpError::Proxy(head.status));
        }
        // the proxy's own bytes past its answer would pass for the server's
        if rd.has_buffered() {
            return Err(HttpError::Bad);
        }
        Ok(())
    }

    // a new connection for url: direct, to the proxy (http), or through its tunnel (https)
    fn open(&self, url: &Url, deadline: Option<u64>) -> Result<(Box<dyn Stream>, Option<Endpoint>), HttpError> {
        let Some(ep) = self.proxy.as_ref().and_then(|p| p.for_url(url)).cloned() else {
            let tcp = self.tcp(url, deadline)?;
            return Ok((if url.is_https() { self.tls(url, tcp)? } else { Box::new(tcp) }, None));
        };
        let tcp = self.tcp(&ep.url(), deadline)?;
        if !url.is_https() {
            return Ok((Box::new(tcp), Some(ep)));
        }
        self.tunnel(&tcp, &ep, url, deadline)?;
        Ok((self.tls(url, tcp)?, None))
    }

    // what tells kept connections apart: the origin, the proxy, the TLS layer
    fn pool_key(&self, url: &Url) -> String {
        let via = self.proxy.as_ref().and_then(|p| p.for_url(url)).map(|e| e.key()).unwrap_or_default();
        let tls = self.connector.as_ref().map_or(0, |c| Arc::as_ptr(c) as *const () as usize);
        format!("{}|{}|{}|{:x}", url.scheme, url.authority(), via, tls)
    }

    // the cookies of the jar for a request (None: no jar, the caller's own header, or none)
    fn cookie_header(&self, req: &Hop) -> Option<String> {
        let jar = self.cookies.as_ref()?;
        if req.headers.get("cookie").is_some() {
            return None;
        }
        let now = plat::unix_time_ns().div_euclid(1_000_000_000);
        let safe = matches!(req.method, "GET" | "HEAD");
        let a = Asks { host: &req.url.host, path: req.url.path(), secure: req.url.is_https(), http: true, site: None, top: true, safe };
        Some(jar.lock().header(&a, now)).filter(|h| !h.is_empty())
    }

    fn store_cookies(&self, url: &Url, h: &Headers) {
        let Some(jar) = self.cookies.as_ref() else { return };
        let now = plat::unix_time_ns().div_euclid(1_000_000_000);
        let o = Origin { host: &url.host, path: url.path(), secure: url.is_https(), http: true };
        let mut jar = jar.lock();
        for (_, v) in h.0.iter().filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")) {
            let _ = jar.store(v, o, now);
        }
    }

    /// One request over `s` and its answer: the head to `read`, which reads the body.  `via`:
    /// the proxy that gets it in absolute form; `keep`: ask for keep-alive.  -> the answer and
    /// whether the connection may carry another request, or the error and whether the
    /// connection ended before any answer (a kept one the server had closed: worth a retry)
    fn on_stream(&self, s: &dyn Stream, req: &Hop, via: Option<&Endpoint>, keep: bool, deadline: Option<u64>, read: ReadFn) -> Result<(Response, bool), (HttpError, bool)> {
        let mut h = req.headers.clone();
        if h.get("host").is_none() {
            h.set("Host", &req.url.host_header());
        }
        if h.get("user-agent").is_none() {
            h.set("User-Agent", &self.user_agent);
        }
        h.set("Connection", if keep { "keep-alive" } else { "close" });
        let decode = self.decode && req.method != "HEAD" && h.get("accept-encoding").is_none() && h.get("range").is_none();
        if decode {
            h.set("Accept-Encoding", ACCEPT_ENCODING);
        }
        if let Some(c) = self.cookie_header(req) {
            h.set("Cookie", &c);
        }
        let target = match via {
            Some(ep) => {
                if let Some(a) = ep.authorization() {
                    h.set("Proxy-Authorization", a);
                }
                format!("{}", req.url) // absolute form (RFC 9112 3.2.2)
            }
            None => req.url.target.clone(),
        };
        if !http::write_request(s, req.method, &target, &h, req.body) {
            return Err((reset(), true));
        }
        let mut rd = Reader::new(s);
        rd.set_deadline(deadline);
        let head = match read_head(&mut rd) {
            Ok(h) => h,
            Err(ReadError::Closed) => return Err((HttpError::Bad, true)),
            Err(e) => return Err((read_err(e), false)),
        };
        self.store_cookies(req.url, &head.headers);
        let none = req.method == "HEAD" || head.status == 204 || head.status == 304;
        let te = head.headers.get("transfer-encoding").is_some();
        let cl = head.headers.get("content-length").is_some();
        let until_close = !none && !te && !cl;
        let mut shown = head.headers.clone();
        let dec = if decode && !none { decoder_of(&head.headers) } else { None };
        if dec.is_some() {
            shown.remove("content-encoding");
            shown.remove("content-length");
        }
        let mut body = Incoming { rd: &mut rd, raw: &head.headers, until_close, dec, none, done: false };
        let r = read(head.status, &shown, &mut body).map_err(|e| (e, false))?;
        let done = body.done;
        let alive = !head.headers.has_token("connection", "close") && (head.minor >= 1 || head.headers.has_token("connection", "keep-alive"));
        // TE with CL is a smuggling attempt or a broken server: the connection goes (RFC 9112 6.3)
        let again = keep && alive && done && !until_close && !(te && cl) && !rd.has_buffered();
        Ok((r, again))
    }

    // one request on a kept or a new connection (a kept one that turns out closed: once more)
    fn hop(&self, req: &Hop, deadline: Option<u64>, read: ReadFn) -> Result<Response, HttpError> {
        let key = self.pool.as_ref().map(|_| self.pool_key(req.url));
        for attempt in 0..2 {
            let kept = match (&self.pool, &key) {
                (Some(p), Some(k)) if attempt == 0 => p.take(k),
                _ => None,
            };
            let reused = kept.is_some();
            let (s, via) = match kept {
                Some(c) => c,
                None => self.open(req.url, deadline)?,
            };
            match self.on_stream(&*s, req, via.as_ref(), key.is_some(), deadline, read) {
                Ok((r, again)) => {
                    match (&self.pool, key) {
                        (Some(p), Some(k)) if again => p.put(k, s, via),
                        _ => s.shutdown(),
                    }
                    return Ok(r);
                }
                Err((e, unanswered)) => {
                    if !(reused && unanswered && idempotent(req.method)) {
                        return Err(e);
                    }
                }
            }
        }
        Err(HttpError::Bad)
    }

    // a request with its redirects -> the last URL and its answer
    fn run(&self, method: &str, url: &Url, headers: &Headers, body: &[u8], read: ReadFn) -> Result<(Url, Response), HttpError> {
        let deadline = self.deadline();
        let (mut m, mut u, mut h, mut b) = (String::from(method), url.clone(), headers.clone(), body);
        let mut hops = 0usize;
        loop {
            let mut next: Option<(Url, u16)> = None;
            let follow = self.redirects > 0;
            let r = {
                let (cur, check, limit) = (&u, &self.redirect_check, self.redirects as usize);
                self.hop(&Hop { method: &m, url: &u, headers: &h, body: b }, deadline, &mut |status, hd, inc| {
                    if follow && is_redirect(status) {
                        if let Some(loc) = hd.get("location") {
                            let to = cur.join(loc.trim()).ok_or_else(|| HttpError::Redirect(String::from("a Location that is not an http(s) URL")))?;
                            if cur.is_https() && !to.is_https() {
                                return Err(HttpError::Redirect(format!("from https to http ({})", to)));
                            }
                            if check.as_ref().is_none_or(|f| f(cur, &to)) {
                                if hops >= limit {
                                    return Err(HttpError::Redirect(format!("more than {} redirects", limit)));
                                }
                                // its body is read (the connection can go on), up to a limit
                                let _ = inc.to(REDIRECT_BODY, &mut |_| true);
                                next = Some((to, status));
                                return Ok(Response { status, headers: hd.clone(), body: Body::Empty });
                            }
                        }
                    }
                    read(status, hd, inc)
                })?
            };
            let Some((to, status)) = next else { return Ok((u, r)) };
            hops += 1;
            // 303 (but HEAD), and 301 / 302 after a POST, go on as GET without the body
            if (status == 303 && m != "HEAD") || ((status == 301 || status == 302) && m == "POST") {
                m = String::from("GET");
                b = &[];
                for n in ["content-type", "content-length", "content-encoding", "content-language", "content-location", "transfer-encoding"] {
                    h.remove(n);
                }
            }
            // credentials stay with their origin
            if !u.same_origin(&to) {
                for n in ["authorization", "cookie", "proxy-authorization", "host"] {
                    h.remove(n);
                }
            }
            u = to;
        }
    }

    /// send one request and read the whole answer
    pub fn request(&self, method: &str, url: &Url, headers: &Headers, body: &[u8]) -> Result<Response, HttpError> {
        self.fetch(method, url, headers, body).map(|f| f.response)
    }

    /// request(), and the URL the answer came from (after redirects)
    pub fn fetch(&self, method: &str, url: &Url, headers: &Headers, body: &[u8]) -> Result<Fetched, HttpError> {
        let max = self.max_body;
        let (url, response) = self.run(method, url, headers, body, &mut |status, headers, inc| {
            let body = inc.all(max)?;
            Ok(Response { status, headers: headers.clone(), body: Body::Bytes(body) })
        })?;
        Ok(Fetched { url, response })
    }

    /// One request over a connection the caller opened (a socket of its own, a test's stream):
    /// the client's headers, cookies and decoding, no proxy, pool or redirects; the request asks
    /// to close the connection, and the stream is left as it is
    pub fn request_on(&self, s: &dyn Stream, method: &str, url: &Url, headers: &Headers, body: &[u8]) -> Result<Response, HttpError> {
        let max = self.max_body;
        let req = Hop { method, url, headers, body };
        self.on_stream(s, &req, None, false, self.deadline(), &mut |status, headers, inc| {
            let body = inc.all(max)?;
            Ok(Response { status, headers: headers.clone(), body: Body::Bytes(body) })
        })
        .map(|(r, _)| r)
        .map_err(|(e, _)| e)
    }

    /// a GET whose answer goes piece by piece to `on` (downloads larger than memory): first
    /// Part::Head (false: skip the body), then Part::Data for every piece of the body (false:
    /// stop, Err(HttpError::Bad)).  The body may have up to `max` bytes.  Redirects the client
    /// follows do not reach `on`.  -> the status and the headers
    pub fn get_to(&self, url: &Url, headers: &Headers, max: u64, on: &mut dyn FnMut(Part<'_>) -> bool) -> Result<(u16, Headers), HttpError> {
        self.run("GET", url, headers, &[], &mut |status, headers, inc| {
            if on(Part::Head(status, headers)) {
                inc.to(max, &mut |b| on(Part::Data(b)))?;
            }
            Ok(Response { status, headers: headers.clone(), body: Body::Empty })
        })
        .map(|(_, r)| (r.status, r.headers))
    }

    pub fn get(&self, url: &Url, headers: &Headers) -> Result<Response, HttpError> {
        self.request("GET", url, headers, &[])
    }

    /// request() on the blocking pool (plat::task::spawn_blocking): a future of the answer
    pub fn request_async(&self, method: &str, url: &Url, headers: &Headers, body: Vec<u8>) -> impl Future<Output = Result<Response, HttpError>> + use<> {
        let (c, m, u, h) = (self.clone(), String::from(method), url.clone(), headers.clone());
        plat::task::spawn_blocking(move || c.request(&m, &u, &h, &body))
    }

    /// A GET whose body goes into the file at path, on the blocking pool.  The file is replaced
    /// like plat::write_file (plat::AtomicFile) and only by the whole body of a 2xx answer;
    /// other answers leave it alone.  ctl.progress counts the bytes (the total from
    /// Content-Length); ctl.cancel, or dropping the future, stops between pieces:
    /// Err(Cancelled), the file unchanged.  -> the status
    pub fn download(&self, url: &Url, headers: &Headers, path: &str, ctl: &Ctl) -> impl Future<Output = Result<u16, HttpError>> + use<> {
        let (c, u, h, p, ctl) = (self.clone(), url.clone(), headers.clone(), String::from(path), ctl.clone());
        plat::task::spawn_cancellable(move |gone| c.download_to(&u, &h, &p, &ctl, gone))
    }

    fn download_to(&self, url: &Url, headers: &Headers, path: &str, ctl: &Ctl, gone: &CancelToken) -> Result<u16, HttpError> {
        let stop = || ctl.cancel.is_cancelled() || gone.is_cancelled();
        if stop() {
            return Err(HttpError::Cancelled);
        }
        let mut file: Option<plat::AtomicFile> = None;
        let mut off = 0u64;
        let mut failed = false;
        let r = self.get_to(url, headers, u64::MAX, &mut |part| match part {
            Part::Head(status, h) => {
                if !(200..300).contains(&status) || stop() {
                    return false; // the body is skipped
                }
                if let Some(n) = h.get("content-length").and_then(|v| v.trim().parse::<u64>().ok()) {
                    ctl.progress.set_total(n);
                }
                file = plat::AtomicFile::create(path);
                failed = file.is_none();
                !failed
            }
            Part::Data(b) => {
                let Some(f) = file.as_ref().filter(|_| !stop()) else { return false };
                if f.write_at(off, b) != Some(b.len()) {
                    failed = true;
                    return false;
                }
                off += b.len() as u64;
                ctl.progress.set(off);
                true
            }
        });
        // the AtomicFile is dropped unless committed: its sibling goes, the file stays
        if stop() {
            return Err(HttpError::Cancelled);
        }
        if failed {
            return Err(HttpError::File);
        }
        let (status, _) = r?;
        if let Some(f) = file {
            if !f.commit() {
                return Err(HttpError::File);
            }
        }
        Ok(status)
    }
}

/// requests over one kept-alive connection (reconnected when the server closed it); the
/// client's proxy, cookies and decoding apply, its pool and redirects do not
pub struct Session {
    client: Client,
    url: Url,
    conn: Option<(Box<dyn Stream>, Option<Endpoint>)>,
}

impl Session {
    pub fn new(client: Client, url: Url) -> Session {
        Session { client, url, conn: None }
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    fn once(&mut self, method: &str, target: &str, headers: &Headers, body: &[u8], max_body: usize) -> Result<Response, (HttpError, bool)> {
        let deadline = self.client.deadline();
        let reused = self.conn.is_some();
        if self.conn.is_none() {
            self.conn = Some(self.client.open(&self.url, deadline).map_err(|e| (e, false))?);
        }
        let (s, via) = self.conn.as_ref().ok_or((HttpError::Bad, false))?;
        let url = self.url.with_target(target);
        let req = Hop { method, url: &url, headers, body };
        let r = self.client.on_stream(&**s, &req, via.as_ref(), true, deadline, &mut |status, headers, inc| {
            let body = inc.all(max_body)?;
            Ok(Response { status, headers: headers.clone(), body: Body::Bytes(body) })
        });
        match r {
            Ok((r, again)) => {
                if !again {
                    self.conn = None;
                }
                Ok(r)
            }
            Err((e, unanswered)) => {
                self.conn = None;
                Err((e, reused && unanswered))
            }
        }
    }

    /// one request (an answer of at most `max_body` bytes)
    pub fn request(&mut self, method: &str, target: &str, headers: &Headers, body: &[u8], max_body: usize) -> Result<Response, HttpError> {
        for attempt in 0..2 {
            match self.once(method, target, headers, body, max_body) {
                Ok(r) => return Ok(r),
                // a request that may have run already is not sent twice
                Err((e, retry)) => {
                    if !(retry && idempotent(method) && attempt == 0) {
                        return Err(e);
                    }
                }
            }
        }
        Err(HttpError::Bad)
    }
}

/// "Bearer <token>" headers plus JSON content type
pub fn json_headers(token: &str) -> Headers {
    let mut h = Headers::new();
    h.set("Accept", "application/json");
    h.set("Content-Type", "application/json");
    if !token.is_empty() {
        h.set("Authorization", &format!("Bearer {}", token));
    }
    h
}
