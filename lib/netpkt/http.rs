// http -- HTTP/1.1 messages byte for byte without I/O: an incremental parser that takes the
// bytes as they arrive and says when it needs more, and the encoders of requests, responses
// and chunks (RFC 9110, 9112), with the wire rules of lib/netio/http.rs
//
//   Headers              header lines in order; names compare case-insensitively: get / set /
//                        add / remove / has_token / content_length / chunked
//   Version, Head        a start line with its headers: a request (method, target) or a
//                        response (status, reason); Head::keep_alive / path
//   Parser::requests() / responses()
//                        push(bytes) -> Event: More (more bytes are needed), Head, Data (body
//                        bytes as they came), End (the message is complete, with the trailers
//                        of a chunked body); poll() gives the next event of bytes pushed before
//                        (pipelined messages), finish() at the end of the connection (a
//                        response body without a length ends there); expect(method) tells a
//                        response parser which request it answers (HEAD: no body); idle,
//                        buffered, framing, max_body
//   Framing              how a body is delimited: None, Length(n), Chunked, UntilClose;
//                        request_framing / response_framing after RFC 9112 6.3
//   keep_alive(version, headers)   the connection stays open: HTTP/1.1 unless "Connection:
//                        close", HTTP/1.0 only with "Connection: keep-alive"
//   encode_request / encode_response / encode_head / encode_chunk / encode_chunked /
//   encode_last_chunk    Content-Length or chunked as netio writes them; lines whose value has
//                        CR or LF (or whose name has CR, LF or ':') are left out
//   parse_head(bytes, request)  one head on its own (the dissector): Ok(Some(head)), Ok(None)
//                        while it is incomplete; start_kind(bytes) tells a request from a
//                        response from anything else
//   reason(status), http_date(unix seconds) ("Tue, 20 Oct 2026 12:00:00 GMT")
//
// Limits: a head of at most 16 KiB (start line and header lines with their line ends) and 100
// header lines (431), a request target of at most 8 KiB (414), chunk-size lines of at most 1024
// bytes, bodies up to max_body (413).  Obsolete line folding, a header name with spaces or
// control characters, a line that is not UTF-8, a Content-Length that is no number or differs
// from another one, a request with both Transfer-Encoding and Content-Length, and a
// Transfer-Encoding other than chunked are errors (400); a request of a version other than
// HTTP/1.0 and HTTP/1.1 is 505.  Up to four empty lines before a start line are skipped; LF
// alone ends a line too.  A response to HEAD, a 1xx, 204 or 304 response has no body; a
// response without a length runs until the connection ends; a request without one has none.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const MAX_HEAD: usize = 16 * 1024;
pub const MAX_HEADERS: usize = 100;
pub const MAX_TARGET: usize = 8 * 1024;
pub const MAX_CHUNK_LINE: usize = 1024;
/// the body limit of a new parser
pub const MAX_BODY: u64 = 64 << 20;
/// the last chunk of a chunked body without trailers
pub const LAST_CHUNK: &[u8] = b"0\r\n\r\n";

/// header lines; names compare case-insensitively
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Headers(pub Vec<(String, String)>);

impl Headers {
    pub fn new() -> Headers {
        Headers(Vec::new())
    }
    /// the first value of a header
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    /// replaces (or adds) a header
    pub fn set(&mut self, name: &str, value: &str) {
        self.remove(name);
        self.add(name, value);
    }
    pub fn add(&mut self, name: &str, value: &str) {
        self.0.push((String::from(name), String::from(value)));
    }
    pub fn remove(&mut self, name: &str) {
        self.0.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// whether a comma-separated header names a token ("Connection: close")
    pub fn has_token(&self, name: &str, token: &str) -> bool {
        self.0.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name)).any(|(_, v)| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
    }
    /// the Content-Length: None without one, an error when it is no number or two differ
    pub fn content_length(&self) -> Result<Option<u64>, HttpErr> {
        let mut n: Option<u64> = None;
        for (_, v) in self.0.iter().filter(|(k, _)| k.eq_ignore_ascii_case("content-length")) {
            for part in v.split(',') {
                let p = part.trim();
                if p.is_empty() || p.len() > 19 || !p.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(HttpErr::Bad("Content-Length is no number"));
                }
                let x: u64 = p.parse().map_err(|_| HttpErr::Bad("Content-Length is no number"))?;
                if n.is_some_and(|m| m != x) {
                    return Err(HttpErr::Bad("two different Content-Lengths"));
                }
                n = Some(x);
            }
        }
        Ok(n)
    }
    /// the body is chunked (Transfer-Encoding)
    pub fn chunked(&self) -> bool {
        self.get("transfer-encoding").is_some_and(|v| v.trim().eq_ignore_ascii_case("chunked"))
    }
}

/// the protocol version of a message
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Version {
    Http10,
    #[default]
    Http11,
}

impl Version {
    pub fn as_str(self) -> &'static str {
        match self {
            Version::Http10 => "HTTP/1.0",
            Version::Http11 => "HTTP/1.1",
        }
    }
}

/// why a message could not be read
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpErr {
    /// malformed (400), with what was wrong
    Bad(&'static str),
    /// a head over 16 KiB or with more than 100 header lines (431)
    HeadTooLarge,
    /// a request target over 8 KiB (414)
    UriTooLong,
    /// a request of a version other than HTTP/1.0 and HTTP/1.1 (505)
    Version,
    /// a body over the limit (413)
    BodyTooLarge,
    /// the connection ended in the middle of a message
    Truncated,
}

impl HttpErr {
    /// the status a server answers with
    pub fn status(self) -> u16 {
        match self {
            HttpErr::Bad(_) | HttpErr::Truncated => 400,
            HttpErr::HeadTooLarge => 431,
            HttpErr::UriTooLong => 414,
            HttpErr::Version => 505,
            HttpErr::BodyTooLarge => 413,
        }
    }
    /// what was wrong, in a few English words (dissector, notes)
    pub fn text(self) -> &'static str {
        match self {
            HttpErr::Bad(w) => w,
            HttpErr::HeadTooLarge => "head too large",
            HttpErr::UriTooLong => "request target too long",
            HttpErr::Version => "unsupported version",
            HttpErr::BodyTooLarge => "body too large",
            HttpErr::Truncated => "the connection ended inside the message",
        }
    }
}

/// a start line and its headers
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Head {
    /// a request: its method and target ("GET", "/index.html?x=1"); empty in a response
    pub method: String,
    pub target: String,
    /// a response: the status and its reason phrase; 0 in a request
    pub status: u16,
    pub reason: String,
    pub version: Version,
    pub headers: Headers,
    /// the bytes of the head up to and including the empty line (with skipped empty lines
    /// before the start line)
    pub len: usize,
}

impl Head {
    pub fn is_request(&self) -> bool {
        self.status == 0
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)
    }
    /// the connection stays open after this message
    pub fn keep_alive(&self) -> bool {
        keep_alive(self.version, &self.headers)
    }
    /// the target without its query
    pub fn path(&self) -> &str {
        match self.target.find(['?', '#']) {
            Some(i) => &self.target[..i],
            None => &self.target,
        }
    }
    /// the start line ("GET / HTTP/1.1", "HTTP/1.1 200 OK")
    pub fn start_line(&self) -> String {
        if self.is_request() {
            format!("{} {} {}", self.method, self.target, self.version.as_str())
        } else if self.reason.is_empty() {
            format!("{} {}", self.version.as_str(), self.status)
        } else {
            format!("{} {} {}", self.version.as_str(), self.status, self.reason)
        }
    }
}

/// whether the connection stays open after a message
pub fn keep_alive(v: Version, h: &Headers) -> bool {
    match v {
        Version::Http11 => !h.has_token("connection", "close"),
        Version::Http10 => h.has_token("connection", "keep-alive"),
    }
}

/// how a body is delimited
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// no body
    None,
    Length(u64),
    Chunked,
    /// until the connection ends (responses only)
    UntilClose,
}

/// the framing of a request body (RFC 9112 6.3; netio's server rejects TE with CL)
pub fn request_framing(h: &Headers) -> Result<Framing, HttpErr> {
    if let Some(te) = h.get("transfer-encoding") {
        if h.get("content-length").is_some() {
            return Err(HttpErr::Bad("Transfer-Encoding and Content-Length"));
        }
        if !te.trim().eq_ignore_ascii_case("chunked") {
            return Err(HttpErr::Bad("Transfer-Encoding other than chunked"));
        }
        return Ok(Framing::Chunked);
    }
    Ok(match h.content_length()? {
        Some(0) | None => Framing::None,
        Some(n) => Framing::Length(n),
    })
}

/// the framing of a response to a request of `method` (RFC 9112 6.3)
pub fn response_framing(status: u16, h: &Headers, method: &str) -> Result<Framing, HttpErr> {
    if method.eq_ignore_ascii_case("HEAD") || status < 200 || status == 204 || status == 304 {
        return Ok(Framing::None);
    }
    if method.eq_ignore_ascii_case("CONNECT") && (200..300).contains(&status) {
        return Ok(Framing::None);
    }
    if let Some(te) = h.get("transfer-encoding") {
        if !te.trim().eq_ignore_ascii_case("chunked") {
            return Err(HttpErr::Bad("Transfer-Encoding other than chunked"));
        }
        return Ok(Framing::Chunked);
    }
    Ok(match h.content_length()? {
        Some(0) => Framing::None,
        Some(n) => Framing::Length(n),
        None => Framing::UntilClose,
    })
}

// ---------------------------------------------------------------- heads

/// a line from `pos` without its line end, and where the next one starts; Ok(None) while the
/// line end is missing, HeadTooLarge when more than `max` bytes wait without one
fn take_line(b: &[u8], pos: usize, max: usize) -> Result<Option<(&[u8], usize)>, HttpErr> {
    match b[pos..].iter().position(|&c| c == b'\n') {
        Some(i) => {
            let mut l = &b[pos..pos + i];
            if l.last() == Some(&b'\r') {
                l = &l[..l.len() - 1];
            }
            Ok(Some((l, pos + i + 1)))
        }
        None if b.len() - pos > max => Err(HttpErr::HeadTooLarge),
        None => Ok(None),
    }
}

fn text(l: &[u8]) -> Result<&str, HttpErr> {
    core::str::from_utf8(l).map_err(|_| HttpErr::Bad("a line that is not UTF-8"))
}

/// what a message starts with: Some(true) a request ("GET "), Some(false) a response
/// ("HTTP/1."), None something else (a continuation, another protocol); empty lines before it
/// are skipped
pub fn start_kind(b: &[u8]) -> Option<bool> {
    let mut p = 0;
    while p < b.len() && (b[p] == b'\r' || b[p] == b'\n') && p < 8 {
        p += 1;
    }
    let b = &b[p..];
    if b.starts_with(b"HTTP/1.") {
        return Some(false);
    }
    let m = b.iter().take(17).position(|&c| c == b' ')?;
    (m > 0 && b[..m].iter().all(|c| c.is_ascii_uppercase())).then_some(true)
}

/// the start line of a request: method, target, version
fn request_line(l: &str) -> Result<(String, String, Version), HttpErr> {
    let mut parts = l.split(' ');
    let (Some(method), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(HttpErr::Bad("request line is not METHOD TARGET VERSION"));
    };
    let v = match version {
        "HTTP/1.1" => Version::Http11,
        "HTTP/1.0" => Version::Http10,
        _ => return Err(HttpErr::Version),
    };
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(HttpErr::Bad("bad method"));
    }
    if !(target.starts_with('/') || target == "*") {
        return Err(HttpErr::Bad("bad request target"));
    }
    if target.len() > MAX_TARGET {
        return Err(HttpErr::UriTooLong);
    }
    Ok((String::from(method), String::from(target), v))
}

/// the start line of a response: version, status, reason
fn status_line(l: &str) -> Result<(Version, u16, String), HttpErr> {
    let (v, rest) = l.split_once(' ').ok_or(HttpErr::Bad("status line without a status"))?;
    if !v.starts_with("HTTP/1.") || v.len() != 8 {
        return Err(HttpErr::Bad("status line without HTTP/1.x"));
    }
    let version = if v == "HTTP/1.0" { Version::Http10 } else { Version::Http11 };
    let (code, reason) = match rest.split_once(' ') {
        Some((c, r)) => (c, r),
        None => (rest, ""),
    };
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(HttpErr::Bad("status is not three digits"));
    }
    let status: u16 = code.parse().map_err(|_| HttpErr::Bad("status is not three digits"))?;
    if status < 100 {
        return Err(HttpErr::Bad("status below 100"));
    }
    Ok((version, status, String::from(reason)))
}

/// a head at the start of `b` (Ok(None): incomplete); `request` picks the kind of start line
pub fn parse_head(b: &[u8], request: bool) -> Result<Option<Head>, HttpErr> {
    let mut budget = MAX_HEAD;
    let mut pos = 0;
    let mut skipped = 0;
    let start = loop {
        let Some((l, next)) = take_line(b, pos, budget)? else {
            return Ok(None);
        };
        pos = next;
        if l.is_empty() && skipped < 4 {
            skipped += 1;
            continue;
        }
        break l;
    };
    if start.len() > budget {
        return Err(HttpErr::HeadTooLarge);
    }
    budget -= start.len();
    let start = text(start)?;
    let mut h = Head::default();
    if request {
        let (m, t, v) = request_line(start)?;
        h.method = m;
        h.target = t;
        h.version = v;
    } else {
        let (v, s, r) = status_line(start)?;
        h.version = v;
        h.status = s;
        h.reason = r;
    }
    loop {
        let Some((l, next)) = take_line(b, pos, budget)? else {
            return Ok(None);
        };
        pos = next;
        if l.is_empty() {
            break;
        }
        budget = budget.checked_sub(l.len() + 2).ok_or(HttpErr::HeadTooLarge)?;
        if h.headers.len() >= MAX_HEADERS {
            return Err(HttpErr::HeadTooLarge);
        }
        if l[0] == b' ' || l[0] == b'\t' {
            return Err(HttpErr::Bad("obsolete line folding"));
        }
        let l = text(l)?;
        let (k, v) = l.split_once(':').ok_or(HttpErr::Bad("header line without a colon"))?;
        if k.is_empty() || k.bytes().any(|c| c <= b' ' || c == 0x7f) {
            return Err(HttpErr::Bad("bad header name"));
        }
        h.headers.add(k, v.trim());
    }
    h.len = pos;
    Ok(Some(h))
}

// ---------------------------------------------------------------- the parser

/// what the bytes so far make up
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// more bytes are needed
    More,
    /// the start line and the headers of a message
    Head(Head),
    /// body bytes as they came (chunk framing removed)
    Data(Vec<u8>),
    /// the message is complete; the trailers of a chunked body
    End(Headers),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Head,
    /// bytes of a body with a length still to come (0: End next)
    Body(u64),
    ChunkSize,
    ChunkData(u64),
    /// the line end after a chunk
    ChunkEnd,
    Trailers,
    UntilClose,
    Failed(HttpErr),
}

/// an incremental parser of the messages of one connection, in one direction
#[derive(Clone, Debug)]
pub struct Parser {
    requests: bool,
    buf: Vec<u8>,
    pos: usize,
    st: St,
    /// a Head was given out and its End not yet
    in_msg: bool,
    /// the largest body it takes (BodyTooLarge beyond)
    pub max_body: u64,
    body: u64,
    framing: Framing,
    trailers: Headers,
    trailer_budget: usize,
    /// a response parser: the methods of the requests not answered yet, oldest first
    methods: VecDeque<String>,
    /// messages completed
    pub messages: u64,
}

impl Parser {
    fn new(requests: bool) -> Parser {
        Parser {
            requests,
            buf: Vec::new(),
            pos: 0,
            st: St::Head,
            in_msg: false,
            max_body: MAX_BODY,
            body: 0,
            framing: Framing::None,
            trailers: Headers::new(),
            trailer_budget: MAX_HEAD,
            methods: VecDeque::new(),
            messages: 0,
        }
    }
    /// a parser of requests (a server reads them)
    pub fn requests() -> Parser {
        Parser::new(true)
    }
    /// a parser of responses (a client reads them)
    pub fn responses() -> Parser {
        Parser::new(false)
    }
    /// a response parser: a request of `method` went out; its response is next after the
    /// ones before (a response to HEAD has no body)
    pub fn expect(&mut self, method: &str) {
        self.methods.push_back(String::from(method));
    }
    /// requests sent and not answered yet
    pub fn outstanding(&self) -> usize {
        self.methods.len()
    }
    /// bytes pushed and not used yet
    pub fn buffered(&self) -> usize {
        self.buf.len() - self.pos
    }
    /// between messages with nothing buffered
    pub fn idle(&self) -> bool {
        !self.in_msg && self.buffered() == 0 && self.st == St::Head
    }
    /// inside a message (its head came, its end not yet)
    pub fn in_message(&self) -> bool {
        self.in_msg
    }
    /// the framing of the message being read
    pub fn framing(&self) -> Framing {
        self.framing
    }
    /// body bytes of the current message so far
    pub fn body_len(&self) -> u64 {
        self.body
    }
    /// takes more bytes and gives the first event they complete (More: nothing yet)
    pub fn push(&mut self, data: &[u8]) -> Result<Event, HttpErr> {
        if let St::Failed(e) = self.st {
            return Err(e);
        }
        if self.pos > 0 && self.pos * 2 >= self.buf.len() {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        self.buf.extend_from_slice(data);
        self.poll()
    }
    fn fail(&mut self, e: HttpErr) -> Result<Event, HttpErr> {
        self.st = St::Failed(e);
        Err(e)
    }
    /// the next event of the bytes pushed so far
    pub fn poll(&mut self) -> Result<Event, HttpErr> {
        loop {
            match self.st {
                St::Failed(e) => return Err(e),
                St::Head => {
                    if self.pos == self.buf.len() {
                        return Ok(Event::More);
                    }
                    let h = match parse_head(&self.buf[self.pos..], self.requests) {
                        Ok(Some(h)) => h,
                        Ok(None) => return Ok(Event::More),
                        Err(e) => return self.fail(e),
                    };
                    let framing = if self.requests {
                        request_framing(&h.headers)
                    } else {
                        let m = if h.status >= 200 { self.methods.pop_front() } else { None };
                        response_framing(h.status, &h.headers, m.as_deref().unwrap_or("GET"))
                    };
                    let framing = match framing {
                        Ok(f) => f,
                        Err(e) => return self.fail(e),
                    };
                    if let Framing::Length(n) = framing
                        && n > self.max_body
                    {
                        return self.fail(HttpErr::BodyTooLarge);
                    }
                    self.pos += h.len;
                    self.framing = framing;
                    self.body = 0;
                    self.in_msg = true;
                    self.trailers = Headers::new();
                    self.st = match framing {
                        Framing::None => St::Body(0),
                        Framing::Length(n) => St::Body(n),
                        Framing::Chunked => St::ChunkSize,
                        Framing::UntilClose => St::UntilClose,
                    };
                    return Ok(Event::Head(h));
                }
                St::Body(left) | St::ChunkData(left) => {
                    if left == 0 {
                        if self.st == St::Body(0) {
                            return Ok(self.end());
                        }
                        self.st = St::ChunkEnd;
                        continue;
                    }
                    let avail = (self.buf.len() - self.pos) as u64;
                    if avail == 0 {
                        return Ok(Event::More);
                    }
                    let n = left.min(avail) as usize;
                    let d = self.buf[self.pos..self.pos + n].to_vec();
                    self.pos += n;
                    self.body += n as u64;
                    self.st = if matches!(self.st, St::Body(_)) { St::Body(left - n as u64) } else { St::ChunkData(left - n as u64) };
                    return Ok(Event::Data(d));
                }
                St::ChunkSize => {
                    let (l, next) = match take_line(&self.buf, self.pos, MAX_CHUNK_LINE) {
                        Ok(Some(x)) => x,
                        Ok(None) => return Ok(Event::More),
                        Err(_) => return self.fail(HttpErr::Bad("chunk-size line too long")),
                    };
                    let size = l.split(|&c| c == b';').next().unwrap_or(&[]);
                    let size = size.trim_ascii();
                    if size.is_empty() || size.len() > 15 || !size.iter().all(|c| c.is_ascii_hexdigit()) {
                        return self.fail(HttpErr::Bad("bad chunk size"));
                    }
                    let n = size.iter().fold(0u64, |a, &c| a * 16 + (c as char).to_digit(16).unwrap_or(0) as u64);
                    self.pos = next;
                    if n == 0 {
                        self.st = St::Trailers;
                        self.trailer_budget = MAX_HEAD;
                        continue;
                    }
                    if self.body.saturating_add(n) > self.max_body {
                        return self.fail(HttpErr::BodyTooLarge);
                    }
                    self.st = St::ChunkData(n);
                }
                St::ChunkEnd => {
                    let rest = &self.buf[self.pos..];
                    match rest {
                        [] | [b'\r'] => return Ok(Event::More),
                        [b'\n', ..] => self.pos += 1,
                        [b'\r', b'\n', ..] => self.pos += 2,
                        _ => return self.fail(HttpErr::Bad("no line end after a chunk")),
                    }
                    self.st = St::ChunkSize;
                }
                St::Trailers => {
                    let (l, next) = match take_line(&self.buf, self.pos, self.trailer_budget) {
                        Ok(Some(x)) => x,
                        Ok(None) => return Ok(Event::More),
                        Err(e) => return self.fail(e),
                    };
                    if l.is_empty() {
                        self.pos = next;
                        return Ok(self.end());
                    }
                    let Some(budget) = self.trailer_budget.checked_sub(l.len() + 2) else {
                        return self.fail(HttpErr::HeadTooLarge);
                    };
                    self.trailer_budget = budget;
                    if let Ok(t) = core::str::from_utf8(l)
                        && let Some((k, v)) = t.split_once(':')
                        && !k.is_empty()
                        && !k.bytes().any(|c| c <= b' ' || c == 0x7f)
                    {
                        self.trailers.add(k, v.trim());
                    }
                    self.pos = next;
                }
                St::UntilClose => {
                    let avail = self.buf.len() - self.pos;
                    if avail == 0 {
                        return Ok(Event::More);
                    }
                    if self.body.saturating_add(avail as u64) > self.max_body {
                        return self.fail(HttpErr::BodyTooLarge);
                    }
                    let d = self.buf[self.pos..].to_vec();
                    self.pos = self.buf.len();
                    self.body += avail as u64;
                    return Ok(Event::Data(d));
                }
            }
        }
    }
    fn end(&mut self) -> Event {
        self.st = St::Head;
        self.in_msg = false;
        self.messages += 1;
        Event::End(core::mem::take(&mut self.trailers))
    }
    /// the connection ended: a body that runs until the close is complete (End); between
    /// messages with nothing buffered More (a clean end); inside a message Truncated
    pub fn finish(&mut self) -> Result<Event, HttpErr> {
        match self.st {
            St::Failed(e) => Err(e),
            St::UntilClose if self.buf.len() == self.pos => Ok(self.end()),
            St::UntilClose => self.poll(),
            St::Head if !self.in_msg && self.buf.len() == self.pos => Ok(Event::More),
            _ => self.fail(HttpErr::Truncated),
        }
    }
}

// ---------------------------------------------------------------- writing

fn head_bytes(start: &str, h: &Headers) -> Vec<u8> {
    let mut s = String::with_capacity(256);
    // no request / response splitting through method, target or reason either
    s.extend(start.chars().filter(|c| !matches!(c, '\r' | '\n')));
    s.push_str("\r\n");
    for (k, v) in &h.0 {
        // no header injection: values with CR / LF are left out
        if v.contains(['\r', '\n']) || k.contains(['\r', '\n', ':']) || k.is_empty() {
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

/// a head exactly as given: a request when `status` is 0, else a response
pub fn encode_head(h: &Head) -> Vec<u8> {
    head_bytes(&h.start_line(), &h.headers)
}

/// a request; Content-Length when it has a body or the method is POST, PUT or PATCH (unless
/// the headers say chunked: then the body goes as one chunk)
pub fn encode_request(method: &str, target: &str, version: Version, h: &Headers, body: &[u8]) -> Vec<u8> {
    let start = format!("{} {} {}", method, target, version.as_str());
    let mut h = h.clone();
    if h.chunked() {
        h.remove("Content-Length");
        let mut out = head_bytes(&start, &h);
        out.extend_from_slice(&encode_chunked(body, body.len().max(1)));
        return out;
    }
    if !body.is_empty() || matches!(method, "POST" | "PUT" | "PATCH") {
        h.set("Content-Length", &format!("{}", body.len()));
    }
    let mut out = head_bytes(&start, &h);
    out.extend_from_slice(body);
    out
}

/// a response with its body (only the head for HEAD requests): Content-Length for every status
/// from 200 on but 204 and 304, unless the headers say chunked (then the body goes as one
/// chunk and the last chunk)
pub fn encode_response(version: Version, status: u16, h: &Headers, body: &[u8], head_only: bool) -> Vec<u8> {
    let start = format!("{} {} {}", version.as_str(), status, reason(status));
    let no_body = status < 200 || status == 204 || status == 304;
    let mut h = h.clone();
    if h.chunked() && !no_body {
        h.remove("Content-Length");
        let mut out = head_bytes(&start, &h);
        if !head_only {
            out.extend_from_slice(&encode_chunked(body, body.len().max(1)));
        }
        return out;
    }
    if !no_body {
        h.set("Content-Length", &format!("{}", body.len()));
    }
    let mut out = head_bytes(&start, &h);
    if !head_only && !no_body {
        out.extend_from_slice(body);
    }
    out
}

/// one chunk; nothing for empty data (an empty chunk would end the body)
pub fn encode_chunk(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }
    let mut out = format!("{:x}\r\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\r\n");
    out
}

/// the last chunk with trailers
pub fn encode_last_chunk(trailers: &Headers) -> Vec<u8> {
    let mut out = head_bytes("0", trailers);
    if trailers.is_empty() {
        out.clear();
        out.extend_from_slice(LAST_CHUNK);
    }
    out
}

/// a whole body as chunks of `size` bytes and the last chunk
pub fn encode_chunked(body: &[u8], size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + body.len() / size.max(1) * 8 + 8);
    for c in body.chunks(size.max(1)) {
        out.extend_from_slice(&encode_chunk(c));
    }
    out.extend_from_slice(LAST_CHUNK);
    out
}

/// the reason phrase of a status code
pub fn reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
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
        507 => "Insufficient Storage",
        _ => "Unknown",
    }
}

/// the date of a Date header (IMF-fixdate) for seconds since 1970
pub fn http_date(unix: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (y, mo, d, h, mi, s) = crate::ntp::civil(unix);
    let wd = DAYS[unix.div_euclid(86_400).rem_euclid(7) as usize];
    format!("{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT", wd, d, MONTHS[(mo as usize).clamp(1, 12) - 1], y, h, mi, s)
}
