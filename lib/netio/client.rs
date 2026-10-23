// client -- a blocking HTTP/1.1 client: one connection per request, timeouts, a size limit,
// an optional connector for https (TLS wraps the TCP connection)
//
//   Client::new().request("GET", &url, &headers, body) -> Result<Response, HttpError>
//   client.get_to(&url, &headers, max, on)   a download piece by piece (Part::Head, Part::Data)
//   client.with_connector(Arc<dyn Fn(&Url, TcpStream) -> Result<Box<dyn Stream>, HttpError>>)
//   client.request_async(method, &url, &headers, body) -> future of the answer (the request on
//   plat::task's blocking pool: ui.spawn / block_on await it without blocking)
//   client.download(&url, &headers, path, &ctl) -> future of the status: the body into the file
//   (plat::AtomicFile: replaced only by a whole 2xx body), progress and cancel in ctl

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;

use plat::aio::Ctl;
use plat::net::{NetError, TcpStream};
use plat::task::CancelToken;

use crate::http::{self, Body, Headers, ReadError, Reader, Response};
use crate::url::Url;
use crate::Stream;

#[derive(Clone, Debug, PartialEq, Eq)]
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
        }
    }
}

impl From<NetError> for HttpError {
    fn from(e: NetError) -> Self {
        HttpError::Net(e)
    }
}

pub type Connector = Arc<dyn Fn(&Url, TcpStream) -> Result<Box<dyn Stream>, HttpError> + Send + Sync>;

/// a piece of an answer that Client::get_to streams
pub enum Part<'a> {
    /// the status and the headers (first)
    Head(u16, &'a Headers),
    /// the next bytes of the body
    Data(&'a [u8]),
}

#[derive(Clone)]
pub struct Client {
    pub connect_ms: u32,
    /// the longest wait for any read or write
    pub timeout_ms: u32,
    pub max_body: usize,
    pub user_agent: String,
    connector: Option<Connector>,
}

impl Default for Client {
    fn default() -> Self {
        Client::new()
    }
}

impl Client {
    pub fn new() -> Client {
        Client { connect_ms: 5000, timeout_ms: 30_000, max_body: 64 << 20, user_agent: String::from("CraftLIB-netio"), connector: None }
    }

    /// the TLS layer for https URLs
    pub fn with_connector(mut self, c: Connector) -> Client {
        self.connector = Some(c);
        self
    }

    /// open the connection a request to `url` goes over
    pub fn connect(&self, url: &Url) -> Result<Box<dyn Stream>, HttpError> {
        let addrs = crate::dns::resolve(&url.host, url.port)?;
        let tcp = TcpStream::connect_any(&addrs, self.connect_ms)?;
        let _ = tcp.set_nodelay(true);
        tcp.set_read_timeout(Some(self.timeout_ms));
        tcp.set_write_timeout(Some(self.timeout_ms));
        if url.is_https() {
            let c = self.connector.as_ref().ok_or_else(|| HttpError::Url(String::from("https needs a TLS connector")))?;
            c(url, tcp)
        } else {
            Ok(Box::new(tcp))
        }
    }

    /// send one request and read the whole answer
    pub fn request(&self, method: &str, url: &Url, headers: &Headers, body: &[u8]) -> Result<Response, HttpError> {
        let max = self.max_body;
        self.exchange(method, url, headers, body, &mut |status, headers, rd| {
            let no_body = method == "HEAD" || status == 204 || status == 304;
            let body = if no_body { Vec::new() } else { rd.body(&headers, max, true).map_err(read_err)? };
            Ok(Response { status, headers, body: Body::Bytes(body) })
        })
    }

    /// a GET whose answer goes piece by piece to `on` (downloads larger than memory): first
    /// Part::Head (false: skip the body), then Part::Data for every piece of the body (false:
    /// stop, Err(HttpError::Bad)).  The body may have up to `max` bytes.  -> the status and the
    /// headers
    pub fn get_to(&self, url: &Url, headers: &Headers, max: u64, on: &mut dyn FnMut(Part<'_>) -> bool) -> Result<(u16, Headers), HttpError> {
        self.exchange("GET", url, headers, &[], &mut |status, headers, rd| {
            if on(Part::Head(status, &headers)) && status != 204 && status != 304 {
                rd.body_to(&headers, max, true, &mut |b| on(Part::Data(b))).map_err(read_err)?;
            }
            Ok(Response { status, headers, body: Body::Empty })
        })
        .map(|r| (r.status, r.headers))
    }

    // one request on a new connection: the answer's status and headers go to `read`, which
    // reads the body from the reader
    fn exchange(&self, method: &str, url: &Url, headers: &Headers, body: &[u8],
                read: &mut dyn FnMut(u16, Headers, &mut Reader<'_>) -> Result<Response, HttpError>) -> Result<Response, HttpError> {
        let s = self.connect(url)?;
        let mut h = headers.clone();
        if h.get("host").is_none() {
            h.set("Host", &url.host_header());
        }
        if h.get("user-agent").is_none() {
            h.set("User-Agent", &self.user_agent);
        }
        h.set("Connection", "close");
        if !http::write_request(&*s, method, &url.target, &h, body) {
            return Err(HttpError::Net(NetError::new(plat::net::NetErrorKind::Reset, 0)));
        }
        let mut rd = Reader::new(&*s);
        let (status, headers) = loop {
            let (start, headers) = rd.head().map_err(read_err)?;
            let mut p = start.splitn(3, ' ');
            let (Some(v), Some(code)) = (p.next(), p.next()) else { return Err(HttpError::Bad) };
            if !v.starts_with("HTTP/1.") {
                return Err(HttpError::Bad);
            }
            let status: u16 = code.parse().map_err(|_| HttpError::Bad)?;
            if (100..200).contains(&status) {
                continue; // interim answers (100 Continue)
            }
            break (status, headers);
        };
        let r = read(status, headers, &mut rd)?;
        s.shutdown();
        Ok(r)
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

/// requests over one kept-alive connection (reconnected when the server closed it)
pub struct Session {
    client: Client,
    url: Url,
    conn: Option<Box<dyn Stream>>,
}

impl Session {
    pub fn new(client: Client, url: Url) -> Session {
        Session { client, url, conn: None }
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    fn once(&mut self, method: &str, target: &str, headers: &Headers, body: &[u8], max_body: usize) -> Result<(Response, bool), (HttpError, bool)> {
        let reused = self.conn.is_some();
        if self.conn.is_none() {
            self.conn = Some(self.client.connect(&self.url).map_err(|e| (e, false))?);
        }
        let s = self.conn.as_ref().ok_or((HttpError::Bad, false))?;
        let mut h = headers.clone();
        if h.get("host").is_none() {
            h.set("Host", &self.url.host_header());
        }
        if h.get("user-agent").is_none() {
            h.set("User-Agent", &self.client.user_agent);
        }
        h.set("Connection", "keep-alive");
        if !http::write_request(&**s, method, target, &h, body) {
            return Err((HttpError::Net(NetError::new(plat::net::NetErrorKind::Reset, 0)), reused));
        }
        let mut rd = Reader::new(&**s);
        let (status, headers) = loop {
            let (start, headers) = match rd.head() {
                Ok(x) => x,
                // a reused connection the server had closed: no answer at all
                Err(ReadError::Closed) => return Err((HttpError::Bad, reused)),
                Err(e) => return Err((read_err(e), false)),
            };
            let mut p = start.splitn(3, ' ');
            let (Some(v), Some(code)) = (p.next(), p.next()) else { return Err((HttpError::Bad, false)) };
            if !v.starts_with("HTTP/1.") {
                return Err((HttpError::Bad, false));
            }
            let status: u16 = code.parse().map_err(|_| (HttpError::Bad, false))?;
            if (100..200).contains(&status) {
                continue;
            }
            break (status, headers);
        };
        let no_body = method == "HEAD" || status == 204 || status == 304;
        let close = headers.has_token("connection", "close");
        let until_close = close && headers.get("content-length").is_none() && headers.get("transfer-encoding").is_none();
        let body = if no_body { Vec::new() } else { rd.body(&headers, max_body, until_close).map_err(|e| (read_err(e), false))? };
        Ok((Response { status, headers, body: Body::Bytes(body) }, close))
    }

    /// one request (an answer of at most `max_body` bytes)
    pub fn request(&mut self, method: &str, target: &str, headers: &Headers, body: &[u8], max_body: usize) -> Result<Response, HttpError> {
        for attempt in 0..2 {
            match self.once(method, target, headers, body, max_body) {
                Ok((r, close)) => {
                    if close {
                        self.conn = None;
                    }
                    return Ok(r);
                }
                Err((e, retry)) => {
                    self.conn = None;
                    // a request that may have run already is not sent twice
                    let idempotent = matches!(method, "GET" | "HEAD" | "PUT" | "DELETE" | "OPTIONS" | "TRACE");
                    if !(retry && idempotent && attempt == 0) {
                        return Err(e);
                    }
                }
            }
        }
        Err(HttpError::Bad)
    }
}

fn read_err(e: ReadError) -> HttpError {
    match e {
        ReadError::Net(n) => HttpError::Net(n),
        ReadError::BodyTooLarge | ReadError::HeadTooLarge => HttpError::TooLarge,
        ReadError::Closed | ReadError::Bad => HttpError::Bad,
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
