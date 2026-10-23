// http -- HTTP/1.1 messages and their wire format
//
//   Headers (case-insensitive names), Request, Response, Body
//   Reader: buffered reading of a Stream (lines, heads, bodies by Content-Length or chunked;
//           body_to hands a body over piece by piece)
//   write_response / write_request
//
// Limits: a message head of at most 16 KiB and 100 header lines, bodies up to the caller's
// limit, chunk sizes checked before use.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::json::{self, Value};
use plat::net::{NetError, NetErrorKind, SocketAddr};

use crate::Stream;

pub const MAX_HEAD: usize = 16 * 1024;
pub const MAX_HEADERS: usize = 100;

/// header lines; names compare case-insensitively
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Headers(pub Vec<(String, String)>);

impl Headers {
    pub fn new() -> Headers {
        Headers(Vec::new())
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// replace (or add) a header
    pub fn set(&mut self, name: &str, value: &str) {
        self.remove(name);
        self.0.push((String::from(name), String::from(value)));
    }

    pub fn add(&mut self, name: &str, value: &str) {
        self.0.push((String::from(name), String::from(value)));
    }

    /// the Content-Length: Ok(None) without one, Err for a value that is not plain digits or
    /// headers that disagree
    pub fn content_length(&self) -> Result<Option<u64>, ()> {
        let mut found: Option<u64> = None;
        for (k, v) in &self.0 {
            if !k.eq_ignore_ascii_case("content-length") {
                continue;
            }
            let v = v.trim();
            if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
                return Err(());
            }
            let n: u64 = v.parse().map_err(|_| ())?;
            if found.is_some_and(|f| f != n) {
                return Err(());
            }
            found = Some(n);
        }
        Ok(found)
    }

    pub fn remove(&mut self, name: &str) {
        self.0.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }

    /// whether a comma-separated header contains a token (Connection: close, ...)
    pub fn has_token(&self, name: &str, token: &str) -> bool {
        self.0.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name)).any(|(_, v)| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
    }
}

/// a request as the server sees it (and as the client sends it)
#[derive(Debug, Default)]
pub struct Request {
    pub method: String,
    /// path and query, as sent ("/api/jobs?limit=5")
    pub target: String,
    pub headers: Headers,
    pub body: Vec<u8>,
    /// the remote address (server side)
    pub peer: Option<SocketAddr>,
    /// the identity the transport proved (TLS client certificate), if any
    pub client_id: Option<String>,
}

impl Request {
    pub fn new(method: &str, target: &str) -> Request {
        Request { method: String::from(method), target: String::from(target), ..Request::default() }
    }

    /// the path without the query
    pub fn path(&self) -> &str {
        crate::url::split_target(&self.target).0
    }

    /// a query parameter, decoded
    pub fn query(&self, key: &str) -> Option<String> {
        crate::url::query_pairs(crate::url::split_target(&self.target).1).into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)
    }

    /// the body as JSON (None: not JSON or over the limits)
    pub fn json(&self, limits: json::Limits) -> Option<Value> {
        json::parse_with_limits(&self.body, limits).ok()
    }

    /// the bearer token of the Authorization header
    pub fn bearer(&self) -> Option<&str> {
        let v = self.header("authorization")?;
        let (scheme, token) = v.split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
    }
}

/// a response body
pub enum Body {
    Empty,
    Bytes(Vec<u8>),
    /// produced piece by piece and sent chunked: the callback appends the next piece to the
    /// buffer and returns false after the last one (Content-Length when `len` is known)
    Stream { len: Option<u64>, next: Box<dyn FnMut(&mut Vec<u8>) -> bool + Send> },
}

impl core::fmt::Debug for Body {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Body::Empty => f.write_str("Empty"),
            Body::Bytes(b) => write!(f, "Bytes({})", b.len()),
            Body::Stream { len, .. } => write!(f, "Stream({:?})", len),
        }
    }
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Headers,
    pub body: Body,
}

impl Response {
    pub fn new(status: u16) -> Response {
        Response { status, headers: Headers::new(), body: Body::Empty }
    }

    pub fn bytes(status: u16, content_type: &str, body: Vec<u8>) -> Response {
        let mut r = Response::new(status);
        r.headers.set("Content-Type", content_type);
        r.body = Body::Bytes(body);
        r
    }

    pub fn text(status: u16, s: &str) -> Response {
        Response::bytes(status, "text/plain; charset=utf-8", Vec::from(s.as_bytes()))
    }

    pub fn json(status: u16, v: &Value) -> Response {
        let lim = json::Limits { bytes: 64 << 20, depth: 32, values: 4_000_000, string_bytes: 16 << 20, members: 1_000_000 };
        match json::stringify_with_limits(v, lim) {
            Ok(s) => Response::bytes(status, "application/json", s.into_bytes()),
            Err(_) => Response::text(500, "response too large"),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Response {
        self.headers.set(name, value);
        self
    }

    /// the body bytes (client side: always Bytes)
    pub fn body_bytes(&self) -> &[u8] {
        match &self.body {
            Body::Bytes(b) => b,
            _ => &[],
        }
    }

    pub fn body_json(&self, limits: json::Limits) -> Option<Value> {
        json::parse_with_limits(self.body_bytes(), limits).ok()
    }
}

/// the reason phrase of a status code
pub fn reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        423 => "Locked",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        _ => "Unknown",
    }
}

// ------------------------------------------------------------------ reading

/// why reading a message failed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// the connection ended before a message started (normal end of keep-alive)
    Closed,
    Net(NetError),
    /// malformed (400)
    Bad,
    /// head over 16 KiB or too many header lines (431)
    HeadTooLarge,
    /// body over the limit (413)
    BodyTooLarge,
}

impl From<NetError> for ReadError {
    fn from(e: NetError) -> Self {
        ReadError::Net(e)
    }
}

/// buffered reading of a stream
pub struct Reader<'a> {
    s: &'a dyn Stream,
    buf: Vec<u8>,
    pos: usize,
    deadline_us: Option<u64>,
}

impl<'a> Reader<'a> {
    pub fn new(s: &'a dyn Stream) -> Reader<'a> {
        Reader { s, buf: Vec::with_capacity(8192), pos: 0, deadline_us: None }
    }

    /// reads fail with TimedOut once plat::time_us() is past this (a limit for a whole head,
    /// where the stream's timeout counts each read)
    pub fn set_deadline(&mut self, deadline_us: Option<u64>) {
        self.deadline_us = deadline_us;
    }

    /// whether unread bytes are buffered (a pipelined next request)
    pub fn has_buffered(&self) -> bool {
        self.pos < self.buf.len()
    }

    fn fill(&mut self) -> Result<usize, NetError> {
        if self.pos > 0 && self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        }
        if self.deadline_us.is_some_and(|d| plat::time_us() > d) {
            return Err(NetError::new(NetErrorKind::TimedOut, 0));
        }
        let old = self.buf.len();
        self.buf.resize(old + 16 * 1024, 0);
        let n = self.s.read(&mut self.buf[old..]);
        self.buf.truncate(old + *n.as_ref().unwrap_or(&0));
        n
    }

    /// one line without CRLF (LF alone accepted); at most `max` bytes
    fn line(&mut self, max: usize, first: bool) -> Result<String, ReadError> {
        loop {
            if let Some(i) = self.buf[self.pos..].iter().position(|&c| c == b'\n') {
                let mut l = &self.buf[self.pos..self.pos + i];
                self.pos += i + 1;
                if l.last() == Some(&b'\r') {
                    l = &l[..l.len() - 1];
                }
                return core::str::from_utf8(l).map(String::from).map_err(|_| ReadError::Bad);
            }
            if self.buf.len() - self.pos > max {
                return Err(ReadError::HeadTooLarge);
            }
            let n = self.fill().map_err(|e| if first && self.buf.len() == self.pos && e.kind == NetErrorKind::Reset { ReadError::Closed } else { ReadError::Net(e) })?;
            if n == 0 {
                return Err(if first && self.buf.len() == self.pos { ReadError::Closed } else { ReadError::Bad });
            }
        }
    }

    /// the start line and the headers of a message
    pub fn head(&mut self) -> Result<(String, Headers), ReadError> {
        let mut budget = MAX_HEAD;
        let mut start = self.line(budget, true)?;
        // tolerate empty lines before a request (RFC 9112 2.2)
        let mut skipped = 0;
        while start.is_empty() && skipped < 4 {
            start = self.line(budget, false)?;
            skipped += 1;
        }
        budget = budget.saturating_sub(start.len());
        let mut h = Headers::new();
        loop {
            let l = self.line(budget, false)?;
            if l.is_empty() {
                break;
            }
            budget = budget.checked_sub(l.len() + 2).ok_or(ReadError::HeadTooLarge)?;
            if h.0.len() >= MAX_HEADERS {
                return Err(ReadError::HeadTooLarge);
            }
            if l.starts_with([' ', '\t']) {
                return Err(ReadError::Bad); // obsolete line folding
            }
            let (k, v) = l.split_once(':').ok_or(ReadError::Bad)?;
            if k.is_empty() || k.bytes().any(|b| b <= b' ' || b == 0x7f) {
                return Err(ReadError::Bad);
            }
            let v = v.trim();
            if v.bytes().any(|b| (b < b' ' && b != b'\t') || b == 0x7f) {
                return Err(ReadError::Bad);
            }
            h.add(k, v);
        }
        Ok((start, h))
    }

    // n bytes of the body to sink (false from sink: Bad)
    fn pass(&mut self, n: u64, sink: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), ReadError> {
        let mut need = n;
        while need > 0 {
            if self.pos == self.buf.len() && self.fill()? == 0 {
                return Err(ReadError::Bad);
            }
            let k = need.min((self.buf.len() - self.pos) as u64) as usize;
            if !sink(&self.buf[self.pos..self.pos + k]) {
                return Err(ReadError::Bad);
            }
            self.pos += k;
            need -= k as u64;
        }
        Ok(())
    }

    /// a message body by Content-Length or chunked transfer (`until_close`: a response without
    /// either reads to the end)
    pub fn body(&mut self, h: &Headers, max: usize, until_close: bool) -> Result<Vec<u8>, ReadError> {
        let mut out = Vec::new();
        if h.get("transfer-encoding").is_none() {
            if let Ok(Some(n)) = h.content_length() {
                out.reserve((n.min(max as u64) as usize).min(1 << 20));
            }
        }
        self.body_to(h, max as u64, until_close, &mut |b| {
            out.extend_from_slice(b);
            true
        })?;
        Ok(out)
    }

    /// the body like body(), but each piece goes to `sink` as it arrives (downloads of any
    /// size); `sink` returning false stops reading with Err(Bad) -> the number of body bytes
    pub fn body_to(&mut self, h: &Headers, max: u64, until_close: bool, sink: &mut dyn FnMut(&[u8]) -> bool) -> Result<u64, ReadError> {
        if let Some(te) = h.get("transfer-encoding") {
            if !te.trim().eq_ignore_ascii_case("chunked") {
                return Err(ReadError::Bad);
            }
            let mut total: u64 = 0;
            loop {
                let l = self.line(1024, false)?;
                let size = l.split(';').next().unwrap_or("").trim();
                let n = u64::from_str_radix(size, 16).map_err(|_| ReadError::Bad)?;
                if n == 0 {
                    // trailers up to the empty line
                    let mut budget = MAX_HEAD;
                    for _ in 0..=MAX_HEADERS {
                        let l = self.line(budget, false)?;
                        if l.is_empty() {
                            return Ok(total);
                        }
                        budget = budget.checked_sub(l.len() + 2).ok_or(ReadError::HeadTooLarge)?;
                    }
                    return Err(ReadError::HeadTooLarge);
                }
                if total.saturating_add(n) > max {
                    return Err(ReadError::BodyTooLarge);
                }
                self.pass(n, sink)?;
                total += n;
                if !self.line(2, false)?.is_empty() {
                    return Err(ReadError::Bad);
                }
            }
        }
        if let Some(n) = h.content_length().map_err(|_| ReadError::Bad)? {
            if n > max {
                return Err(ReadError::BodyTooLarge);
            }
            self.pass(n, sink)?;
            return Ok(n);
        }
        if until_close {
            let mut total: u64 = 0;
            loop {
                let piece = &self.buf[self.pos..];
                total += piece.len() as u64;
                if total > max {
                    return Err(ReadError::BodyTooLarge);
                }
                if !piece.is_empty() && !sink(piece) {
                    return Err(ReadError::Bad);
                }
                self.pos = self.buf.len();
                match self.fill() {
                    Ok(0) => return Ok(total),
                    Ok(_) => {}
                    Err(e) if e.kind == NetErrorKind::Reset => return Ok(total),
                    Err(e) => return Err(ReadError::Net(e)),
                }
            }
        }
        Ok(0)
    }
}

// ------------------------------------------------------------------ writing

fn head_bytes(start: &str, h: &Headers) -> Vec<u8> {
    let mut s = String::with_capacity(256);
    s.push_str(start);
    s.push_str("\r\n");
    for (k, v) in &h.0 {
        // no header injection: values with CR / LF are dropped
        if v.contains(['\r', '\n']) || k.contains(['\r', '\n', ':']) {
            continue;
        }
        s.push_str(k);
        s.push_str(": ");
        s.push_str(v);
        s.push_str("\r\n");
    }
    s.push_str("\r\n");
    s.into_bytes()
}

/// send a response; `head_only` for HEAD requests; false when the connection broke
pub fn write_response(s: &dyn Stream, r: &mut Response, head_only: bool, keep_alive: bool) -> bool {
    let mut h = core::mem::take(&mut r.headers);
    h.set("Connection", if keep_alive { "keep-alive" } else { "close" });
    let chunked = match &r.body {
        Body::Empty => {
            if r.status >= 200 && r.status != 204 && r.status != 304 {
                h.set("Content-Length", "0");
            }
            false
        }
        Body::Bytes(b) => {
            h.set("Content-Length", &format!("{}", b.len()));
            false
        }
        Body::Stream { len: Some(n), .. } => {
            h.set("Content-Length", &format!("{}", n));
            false
        }
        Body::Stream { len: None, .. } => {
            h.set("Transfer-Encoding", "chunked");
            true
        }
    };
    let start = format!("HTTP/1.1 {} {}", r.status, reason(r.status));
    let head = head_bytes(&start, &h);
    r.headers = h;
    match &mut r.body {
        Body::Bytes(b) if !head_only && b.len() <= 64 * 1024 => {
            // one write for small answers
            let mut all = head;
            all.extend_from_slice(b);
            s.write_all(&all).is_ok()
        }
        Body::Bytes(b) => s.write_all(&head).is_ok() && (head_only || s.write_all(b).is_ok()),
        Body::Empty => s.write_all(&head).is_ok(),
        Body::Stream { next, .. } => {
            if s.write_all(&head).is_err() {
                return false;
            }
            if head_only {
                return true;
            }
            let mut piece = Vec::with_capacity(256 * 1024);
            loop {
                piece.clear();
                let more = next(&mut piece);
                if !piece.is_empty() {
                    let ok = if chunked {
                        let mut frame = format!("{:x}\r\n", piece.len()).into_bytes();
                        frame.extend_from_slice(&piece);
                        frame.extend_from_slice(b"\r\n");
                        s.write_all(&frame).is_ok()
                    } else {
                        s.write_all(&piece).is_ok()
                    };
                    if !ok {
                        return false;
                    }
                }
                if !more {
                    break;
                }
            }
            !chunked || s.write_all(b"0\r\n\r\n").is_ok()
        }
    }
}

/// send a request (the body as Content-Length)
pub fn write_request(s: &dyn Stream, method: &str, target: &str, h: &Headers, body: &[u8]) -> bool {
    let token = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b);
    if method.is_empty() || !method.bytes().all(token) || target.is_empty() || target.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return false;
    }
    let mut h = h.clone();
    if !body.is_empty() || matches!(method, "POST" | "PUT" | "PATCH") {
        h.set("Content-Length", &format!("{}", body.len()));
    }
    let mut all = head_bytes(&format!("{} {} HTTP/1.1", method, target), &h);
    all.extend_from_slice(body);
    s.write_all(&all).is_ok()
}
