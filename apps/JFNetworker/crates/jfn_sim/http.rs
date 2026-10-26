// http -- HTTP/1.1 over the TCP sockets of the virtual OS (netpkt::http has the bytes): the
// parts the HTTP services (httpsvc.rs), the console programs (cli/tools.rs) and later services
// (load balancers, scripts) are built from
//
//   Wheel<T>      many deadlines of a service on one of its four timer keys
//   Url           http://host[:port][/target] ("host/path" alone means http); the host may be
//                 a name, which the device's resolver looks up, or an IPv6 address in brackets
//                 (http://[2001:db8::1]:8080/, RFC 3986 3.2.2; Url::host is without them, the
//                 Host header and text() put them back)
//   Serve         the server side: a listening socket and its connections, requests read
//                 incrementally (pipelined ones answered in their order), keep-alive with an
//                 idle timeout and a request limit, a header timeout, 100-continue, 400 / 413 /
//                 414 / 431 / 505 for what cannot be read; the owner takes complete requests
//                 (take) and answers each when it likes (respond)
//   Agent         the client side: requests to hosts (names through the resolver), kept-alive
//                 connections reused per host (a limit per host, optional pipelining),
//                 connect and total timeouts, one retry when a reused connection turns out to
//                 be closed before any byte came; the owner takes Done records with the times
//                 of every step and, when it traces, each step as it happens (AgentEv, for
//                 curl -v); names are looked up for the families of Agent::family (getaddrinfo
//                 order: resolver.rs), and when they have addresses of both families a
//                 connection that cannot be set up goes on to the next address (as curl tries
//                 them one after the other; no parallel attempts of Happy Eyeballs)
//
// Both live inside a service: the service hands them its SvcEvents (on() says whether an event
// was theirs) and the firing of their timer key.  Times are the engine's virtual time; a Date
// header comes from the device clock.  Fail::msg() words a failure as a Msg for the app's
// catalogue (Fail::text() is its German text).

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::msg;
use jfn_model::msg::{phrase, Msg};
use netpkt::http::{self, Event, Framing, Head, Headers, HttpErr, Parser, Version};
use netpkt::{IpAddr, Ipv4, Ipv6};

use crate::kernel::Ns;
use crate::os::{OsApi, SvcEvent};
use crate::resolver::{Family, LookupErr};
use crate::tcp::{SockOpts, TcpEnd};
use crate::trace::{HopId, Note};

// ---------------------------------------------------------------- deadlines

/// deadlines on one timer key of a service: the key fires at the earliest one
#[derive(Clone, Debug)]
pub struct Wheel<T> {
    key: u32,
    at: BTreeMap<(Ns, u64), T>,
    ids: BTreeMap<u64, Ns>,
    next: u64,
    armed: Option<Ns>,
}

impl<T: Clone> Wheel<T> {
    pub fn new(key: u32) -> Wheel<T> {
        Wheel { key, at: BTreeMap::new(), ids: BTreeMap::new(), next: 0, armed: None }
    }
    pub fn key(&self) -> u32 {
        self.key
    }
    /// a deadline; its id cancels it
    pub fn add(&mut self, at: Ns, v: T) -> u64 {
        self.next += 1;
        let id = self.next;
        self.at.insert((at, id), v);
        self.ids.insert(id, at);
        id
    }
    pub fn cancel(&mut self, id: u64) -> Option<T> {
        let at = self.ids.remove(&id)?;
        self.at.remove(&(at, id))
    }
    /// the deadlines that passed, earliest first
    pub fn due(&mut self, now: Ns) -> Vec<(u64, T)> {
        let mut out = Vec::new();
        while let Some((&(t, id), _)) = self.at.iter().next() {
            if t > now {
                break;
            }
            if let Some(v) = self.at.remove(&(t, id)) {
                self.ids.remove(&id);
                out.push((id, v));
            }
        }
        out
    }
    pub fn next_at(&self) -> Option<Ns> {
        self.at.keys().next().map(|k| k.0)
    }
    pub fn len(&self) -> usize {
        self.at.len()
    }
    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }
    pub fn clear(&mut self) {
        self.at.clear();
        self.ids.clear();
        self.armed = None;
    }
    /// the key fired: it is not armed any more
    pub fn fired(&mut self) {
        self.armed = None;
    }
    /// sets the service's timer to the earliest deadline (or stops it)
    pub fn arm(&mut self, os: &mut OsApi) {
        let next = self.next_at();
        if next == self.armed {
            return;
        }
        self.armed = next;
        match next {
            Some(t) => {
                let at = t.max(os.now());
                os.timer(self.key, at);
            }
            None => os.cancel(self.key),
        }
    }
}

// ---------------------------------------------------------------- URLs

/// an http URL
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub host: String,
    pub port: u16,
    /// path and query ("/index.html?x=1")
    pub target: String,
}

/// why a URL is not one
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UrlErr {
    /// a scheme other than http
    Scheme(String),
    /// no host
    NoHost,
    /// the port is no number from 0 to 65535
    Port,
    /// characters a host cannot have
    BadHost,
}

impl Url {
    /// "http://10.0.0.2:8080/a?b", "10.0.0.2/a", "web.lab"
    pub fn parse(s: &str) -> Result<Url, UrlErr> {
        let s = s.trim();
        let rest = match s.find("://") {
            Some(i) => {
                let scheme = &s[..i];
                if !scheme.eq_ignore_ascii_case("http") {
                    return Err(UrlErr::Scheme(String::from(scheme)));
                }
                &s[i + 3..]
            }
            None => s,
        };
        let (auth, target) = match rest.find(['/', '?', '#']) {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        // user info is accepted and left out
        let auth = auth.rsplit_once('@').map(|x| x.1).unwrap_or(auth);
        // an IPv6 address in brackets, maybe with a port after them
        let (bracketed, after) = match auth.strip_prefix('[') {
            Some(r) => match r.split_once(']') {
                Some((a, rest)) => (Some(a), rest),
                None => return Err(UrlErr::BadHost),
            },
            None => (None, ""),
        };
        let port_of = |p: &str| -> Result<u16, UrlErr> {
            if p.is_empty() {
                Ok(80)
            } else if p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()) {
                Err(UrlErr::Port)
            } else {
                Ok(p.parse::<u32>().ok().filter(|&n| n <= 65535).ok_or(UrlErr::Port)? as u16)
            }
        };
        if let Some(a) = bracketed {
            let port = match after.strip_prefix(':') {
                Some(p) => port_of(p)?,
                None if after.is_empty() => 80,
                None => return Err(UrlErr::BadHost),
            };
            if a.is_empty() {
                return Err(UrlErr::NoHost);
            }
            let Some(ip) = Ipv6::parse(a) else {
                return Err(UrlErr::BadHost);
            };
            let target = Url::target_of(target);
            return Ok(Url { host: format!("{}", ip), port, target });
        }
        let (host, port) = match auth.rsplit_once(':') {
            Some((h, p)) => (h, port_of(p)?),
            None => (auth, 80),
        };
        if host.is_empty() {
            return Err(UrlErr::NoHost);
        }
        if !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_') {
            return Err(UrlErr::BadHost);
        }
        Ok(Url { host: String::from(host), port, target: Url::target_of(target) })
    }
    /// the path and query of a URL ("/" when empty, the fragment left out)
    fn target_of(target: &str) -> String {
        let target = target.split('#').next().unwrap_or("");
        if target.is_empty() {
            String::from("/")
        } else if target.starts_with('?') {
            format!("/{}", target)
        } else {
            String::from(target)
        }
    }
    /// the host as a URL writes it: an IPv6 address in brackets
    pub fn host_text(&self) -> String {
        if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() }
    }
    /// the Host header: the port only when it is not 80 (an IPv6 address in brackets)
    pub fn host_header(&self) -> String {
        if self.port == 80 { self.host_text() } else { format!("{}:{}", self.host_text(), self.port) }
    }
    /// the whole URL again ("http://10.0.0.2/")
    pub fn text(&self) -> String {
        format!("http://{}{}", self.host_header(), self.target)
    }
    /// a target relative to this URL (a Location header): absolute URLs, absolute paths
    pub fn join(&self, loc: &str) -> Option<Url> {
        if loc.contains("://") {
            return Url::parse(loc).ok();
        }
        if loc.starts_with('/') {
            return Some(Url { host: self.host.clone(), port: self.port, target: String::from(loc) });
        }
        let base = match self.target.rfind('/') {
            Some(i) => &self.target[..=i],
            None => "/",
        };
        Some(Url { host: self.host.clone(), port: self.port, target: format!("{}{}", base, loc) })
    }
}

/// the date of a Date header from the device clock
pub fn date_now(os: &OsApi) -> String {
    http::http_date(os.clock().div_euclid(1_000_000_000))
}

// ---------------------------------------------------------------- the server side

/// the settings of a server
#[derive(Clone, Debug)]
pub struct ServeCfg {
    pub port: u16,
    /// the listen backlog (0: the system's)
    pub backlog: u32,
    pub opts: SockOpts,
    /// a kept-alive connection without a request closes after this long (0: never)
    pub keepalive: Ns,
    /// requests per connection (0: no limit); the last one is answered with Connection: close
    pub max_requests: u32,
    /// a connection without a complete request head for this long closes (0: never)
    pub header_timeout: Ns,
    /// the largest request body (413 beyond)
    pub max_body: u64,
    /// the Server header ("" leaves it out)
    pub server: String,
}

impl Default for ServeCfg {
    fn default() -> ServeCfg {
        // the values of a common Linux web server
        ServeCfg { port: 80, backlog: 0, opts: SockOpts::default(), keepalive: 75 * crate::kernel::SEC, max_requests: 1000, header_timeout: 60 * crate::kernel::SEC, max_body: 1 << 20, server: String::from("jfn-httpd/1.0") }
    }
}

/// a complete request for the owner of a Serve
#[derive(Clone, Debug)]
pub struct ReqIn {
    pub id: u64,
    pub sock: u32,
    pub peer: (IpAddr, u16),
    pub local: (IpAddr, u16),
    pub head: Head,
    pub body: Vec<u8>,
    /// when its last byte came
    pub at: Ns,
    /// the frame that completed it
    pub hop: HopId,
}

/// an answer
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    /// the headers besides Server, Date, Content-Length, Transfer-Encoding and Connection
    pub headers: Headers,
    pub body: Vec<u8>,
    /// > 0: the body goes chunked, in pieces of this size (HTTP/1.1 requests only)
    pub chunk: usize,
    /// the connection closes after it
    pub close: bool,
}

impl Reply {
    /// a plain-text answer
    pub fn text(status: u16, body: &str) -> Reply {
        let mut h = Headers::new();
        h.add("Content-Type", "text/plain");
        Reply { status, headers: h, body: Vec::from(body.as_bytes()), chunk: 0, close: false }
    }
    /// "404 Not Found\n"
    pub fn status_page(status: u16) -> Reply {
        Reply::text(status, &format!("{} {}\n", status, http::reason(status)))
    }
}

/// a request of a connection waiting for its answer
#[derive(Clone, Debug)]
struct Waiting {
    id: u64,
    keep: bool,
    head_only: bool,
    version: Version,
}

/// what a connection's timer is for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SWait {
    /// no request yet, or a head that is not complete
    Header,
    /// kept alive, nothing to do
    Idle,
}

#[derive(Clone, Debug)]
struct SConn {
    peer: (IpAddr, u16),
    local: (IpAddr, u16),
    parser: Parser,
    /// requests in the order they came, until their answers went (pipelining)
    order: VecDeque<Waiting>,
    /// answers ready before the ones in front of them
    answers: BTreeMap<u64, (Vec<u8>, bool)>,
    /// bytes the socket did not take yet
    out: Vec<u8>,
    /// the request being read: its head, body and the frame of its head
    cur: Option<(Head, Vec<u8>)>,
    served: u32,
    /// no more requests are read; it closes after the answers
    closing: bool,
    eof: bool,
    timer: Option<(u64, SWait)>,
}

/// what a server counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServeStats {
    pub connections: u64,
    pub requests: u64,
    pub responses: u64,
    /// responses by class: 1xx .. 5xx
    pub by_class: [u64; 5],
    /// requests it could not read (400, 413, 414, 431, 505)
    pub bad: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// connections closed because they idled (keep-alive) or sent no head in time
    pub idle_closed: u64,
    pub header_timeouts: u64,
    /// answers that found their connection gone
    pub lost: u64,
}

/// the server side of HTTP in a service
#[derive(Clone, Debug)]
pub struct Serve {
    pub cfg: ServeCfg,
    listener: Option<u32>,
    conns: BTreeMap<u32, SConn>,
    /// request id -> socket
    reqs: BTreeMap<u64, u32>,
    next: u64,
    ready: Vec<ReqIn>,
    wheel: Wheel<u32>,
    pub stats: ServeStats,
}

impl Serve {
    /// a server that uses timer key `key` of its service
    pub fn new(cfg: ServeCfg, key: u32) -> Serve {
        Serve { cfg, listener: None, conns: BTreeMap::new(), reqs: BTreeMap::new(), next: 0, ready: Vec::new(), wheel: Wheel::new(key), stats: ServeStats::default() }
    }
    /// listens; false when the port is taken (a note says so)
    pub fn start(&mut self, os: &mut OsApi, who: &str) -> bool {
        self.stop();
        match os.tcp_listen(self.cfg.port, self.cfg.backlog, self.cfg.opts) {
            Ok(l) => {
                self.listener = Some(l);
                true
            }
            Err(_) => {
                os.note(Note::PortInUse { port: self.cfg.port, svc: String::from(who) });
                false
            }
        }
    }
    /// forgets everything (the service stopped: the OS closes its sockets)
    pub fn stop(&mut self) {
        self.listener = None;
        self.conns.clear();
        self.reqs.clear();
        self.ready.clear();
        self.wheel.clear();
    }
    pub fn listening(&self) -> bool {
        self.listener.is_some()
    }
    /// open connections
    pub fn conns(&self) -> usize {
        self.conns.len()
    }
    /// requests read and not answered yet
    pub fn pending(&self) -> usize {
        self.reqs.len()
    }
    pub fn owns(&self, sock: u32) -> bool {
        self.conns.contains_key(&sock)
    }
    /// complete requests since the last call, in the order they came
    pub fn take(&mut self) -> Vec<ReqIn> {
        core::mem::take(&mut self.ready)
    }
    /// handles an event of its sockets; false when it is not one of them
    pub fn on(&mut self, ev: &SvcEvent, os: &mut OsApi) -> bool {
        match *ev {
            SvcEvent::TcpAccept { listener, .. } if Some(listener) == self.listener => {
                while let Some(s) = os.tcp_accept(listener) {
                    let none = (IpAddr::V4(Ipv4::UNSPECIFIED), 0);
                    let (local, peer) = os.tcp_info(s).map(|r| (r.local, r.remote)).unwrap_or((none, none));
                    let mut parser = Parser::requests();
                    parser.max_body = self.cfg.max_body;
                    self.conns.insert(s, SConn { peer, local, parser, order: VecDeque::new(), answers: BTreeMap::new(), out: Vec::new(), cur: None, served: 0, closing: false, eof: false, timer: None });
                    self.stats.connections += 1;
                    self.retime(os, s);
                }
                self.wheel.arm(os);
                true
            }
            SvcEvent::TcpData { sock, hop } if self.owns(sock) => {
                self.read(os, sock, hop);
                self.wheel.arm(os);
                true
            }
            SvcEvent::TcpWritable { sock } if self.owns(sock) => {
                self.flush(os, sock);
                self.wheel.arm(os);
                true
            }
            SvcEvent::TcpEof { sock, hop } if self.owns(sock) => {
                self.read(os, sock, hop);
                if let Some(c) = self.conns.get_mut(&sock) {
                    c.eof = true;
                    c.closing = true;
                }
                self.flush(os, sock);
                self.wheel.arm(os);
                true
            }
            SvcEvent::TcpClosed { sock, .. } if self.owns(sock) => {
                if let Some(c) = self.conns.remove(&sock) {
                    if let Some((t, _)) = c.timer {
                        self.wheel.cancel(t);
                    }
                    for w in &c.order {
                        self.reqs.remove(&w.id);
                    }
                }
                os.tcp_close(sock);
                self.wheel.arm(os);
                true
            }
            SvcEvent::Timer(k) if k == self.wheel.key() => {
                self.timer(os);
                true
            }
            _ => false,
        }
    }
    /// its timer key fired: connections that idled or sent no head in time close
    fn timer(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (tid, sock) in self.wheel.due(now) {
            let Some(c) = self.conns.get(&sock) else {
                continue;
            };
            let Some((t, why)) = c.timer else {
                continue;
            };
            if t != tid {
                continue;
            }
            match why {
                SWait::Idle => self.stats.idle_closed += 1,
                SWait::Header => self.stats.header_timeouts += 1,
            }
            self.conns.remove(&sock);
            os.note(Note::HttpIdleClose { header: why == SWait::Header });
            os.tcp_close(sock);
        }
        self.wheel.arm(os);
    }
    /// the timer a connection needs now
    fn retime(&mut self, os: &mut OsApi, sock: u32) {
        let now = os.now();
        let Some(c) = self.conns.get_mut(&sock) else {
            return;
        };
        let want = if c.closing || !c.order.is_empty() || !c.out.is_empty() || c.cur.is_some() {
            None
        } else if c.served > 0 && c.parser.idle() {
            Some(SWait::Idle)
        } else {
            Some(SWait::Header)
        };
        if c.timer.map(|t| t.1) == want {
            return;
        }
        if let Some((t, _)) = c.timer.take() {
            self.wheel.cancel(t);
        }
        let dur = match want {
            Some(SWait::Idle) => self.cfg.keepalive,
            Some(SWait::Header) => self.cfg.header_timeout,
            None => 0,
        };
        if let Some(w) = want
            && dur > 0
        {
            let id = self.wheel.add(now + dur, sock);
            c.timer = Some((id, w));
        }
    }
    /// reads what came and turns it into requests
    fn read(&mut self, os: &mut OsApi, sock: u32, hop: HopId) {
        let data = os.tcp_recv(sock, usize::MAX);
        self.stats.bytes_in += data.len() as u64;
        let now = os.now();
        let max_req = self.cfg.max_requests;
        let mut bad: Option<HttpErr> = None;
        let mut cont = false;
        {
            let Some(c) = self.conns.get_mut(&sock) else {
                return;
            };
            if c.closing || data.is_empty() {
                return;
            }
            let mut ev = c.parser.push(&data);
            loop {
                match ev {
                    Ok(Event::More) => break,
                    Ok(Event::Head(h)) => {
                        // 100 Continue when the client waits for it and nothing is in front
                        if h.version == Version::Http11 && h.headers.has_token("expect", "100-continue") && !matches!(c.parser.framing(), Framing::None) && c.order.is_empty() && c.out.is_empty() {
                            cont = true;
                        }
                        c.cur = Some((h, Vec::new()));
                    }
                    Ok(Event::Data(d)) => {
                        if let Some(cur) = c.cur.as_mut() {
                            cur.1.extend_from_slice(&d);
                        }
                    }
                    Ok(Event::End(_)) => {
                        let Some((head, body)) = c.cur.take() else {
                            break;
                        };
                        self.next += 1;
                        let id = self.next;
                        c.served += 1;
                        let keep = head.keep_alive() && (max_req == 0 || c.served < max_req);
                        c.order.push_back(Waiting { id, keep, head_only: head.method == "HEAD", version: head.version });
                        self.reqs.insert(id, sock);
                        self.stats.requests += 1;
                        self.ready.push(ReqIn { id, sock, peer: c.peer, local: c.local, head, body, at: now, hop });
                        if !keep {
                            c.closing = true;
                            break;
                        }
                    }
                    Err(e) => {
                        bad = Some(e);
                        break;
                    }
                }
                ev = c.parser.poll();
            }
            if cont {
                c.out.extend_from_slice(b"HTTP/1.1 100 Continue\r\n\r\n");
            }
        }
        if let Some(e) = bad {
            // what cannot be read is answered and the connection closes after it
            self.next += 1;
            let id = self.next;
            if let Some(c) = self.conns.get_mut(&sock) {
                c.cur = None;
                c.closing = true;
                c.order.push_back(Waiting { id, keep: false, head_only: false, version: Version::Http11 });
            }
            self.reqs.insert(id, sock);
            self.stats.bad += 1;
            os.note(Note::HttpBadRequest { status: e.status() });
            let mut r = Reply::status_page(e.status());
            r.close = true;
            self.respond(os, id, r);
        } else {
            self.flush(os, sock);
        }
        self.retime(os, sock);
    }
    /// answers a request; false when its connection is gone
    pub fn respond(&mut self, os: &mut OsApi, id: u64, r: Reply) -> bool {
        let Some(sock) = self.reqs.remove(&id) else {
            self.stats.lost += 1;
            return false;
        };
        let date = date_now(os);
        let Some(c) = self.conns.get_mut(&sock) else {
            self.stats.lost += 1;
            return false;
        };
        let Some(w) = c.order.iter().find(|w| w.id == id).cloned() else {
            return false;
        };
        let keep = w.keep && !r.close && !c.eof;
        let no_body = r.status < 200 || r.status == 204 || r.status == 304;
        let chunked = r.chunk > 0 && w.version == Version::Http11 && !no_body;
        let mut h = Headers::new();
        if !self.cfg.server.is_empty() && r.headers.get("server").is_none() {
            h.add("Server", &self.cfg.server);
        }
        if r.headers.get("date").is_none() {
            h.add("Date", &date);
        }
        if let Some(ct) = r.headers.get("content-type") {
            h.add("Content-Type", ct);
        }
        if chunked {
            h.add("Transfer-Encoding", "chunked");
        } else if !no_body {
            h.add("Content-Length", &format!("{}", r.body.len()));
        }
        h.add("Connection", if keep { "keep-alive" } else { "close" });
        for (k, v) in &r.headers.0 {
            if !["content-type", "content-length", "transfer-encoding", "connection"].iter().any(|x| k.eq_ignore_ascii_case(x)) {
                h.add(k, v);
            }
        }
        let head = Head { status: r.status, reason: String::from(http::reason(r.status)), version: Version::Http11, headers: h, ..Head::default() };
        let mut bytes = http::encode_head(&head);
        if !w.head_only && !no_body {
            if chunked {
                bytes.extend_from_slice(&http::encode_chunked(&r.body, r.chunk));
            } else {
                bytes.extend_from_slice(&r.body);
            }
        }
        c.answers.insert(id, (bytes, keep));
        self.stats.responses += 1;
        let class = (r.status / 100).clamp(1, 5) as usize - 1;
        self.stats.by_class[class] += 1;
        os.note(Note::HttpAnswered { status: r.status, bytes: r.body.len() as u32 });
        self.flush(os, sock);
        self.retime(os, sock);
        self.wheel.arm(os);
        true
    }
    /// sends what is ready in order; closes when everything went and no more comes
    fn flush(&mut self, os: &mut OsApi, sock: u32) {
        let Some(c) = self.conns.get_mut(&sock) else {
            return;
        };
        while let Some(w) = c.order.front() {
            let Some((bytes, keep)) = c.answers.remove(&w.id) else {
                break;
            };
            c.order.pop_front();
            c.out.extend_from_slice(&bytes);
            if !keep {
                // the connection ends after this answer: later requests are not answered
                c.closing = true;
                for w in c.order.drain(..) {
                    self.reqs.remove(&w.id);
                }
                c.answers.clear();
            }
        }
        if !c.out.is_empty() {
            let n = os.tcp_send(sock, &c.out);
            c.out.drain(..n);
            self.stats.bytes_out += n as u64;
        }
        if c.out.is_empty() && c.order.is_empty() && c.closing {
            if let Some((t, _)) = c.timer.take() {
                self.wheel.cancel(t);
            }
            self.conns.remove(&sock);
            os.tcp_close(sock);
        }
    }
}

// ---------------------------------------------------------------- the client side

/// a request of an Agent
#[derive(Clone, Debug)]
pub struct Req {
    pub method: String,
    pub url: Url,
    pub version: Version,
    /// sent after Host (which comes first unless given here)
    pub headers: Headers,
    pub body: Vec<u8>,
    /// from the submit to the last byte of the answer (0: no limit)
    pub timeout: Ns,
    /// from the connect to the established connection (0: TCP's own limits)
    pub connect_timeout: Ns,
    /// may use a kept-alive connection and leave it open (else a fresh one, closed after)
    pub reuse: bool,
    /// keep the body (else only count it)
    pub keep_body: bool,
}

impl Req {
    pub fn get(url: Url) -> Req {
        Req { method: String::from("GET"), url, version: Version::Http11, headers: Headers::new(), body: Vec::new(), timeout: 0, connect_timeout: 0, reuse: true, keep_body: true }
    }
}

/// an answer an Agent got
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resp {
    pub head: Head,
    pub body: Vec<u8>,
    pub trailers: Headers,
    /// 1xx answers before it
    pub interim: Vec<Head>,
}

/// why a request got no answer
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fail {
    /// the name was not found
    Resolve(LookupErr),
    /// the connection could not be set up: refused, unreachable, or TCP gave up
    Connect(TcpEnd),
    /// the device has no route (the connect was refused at once: why)
    NoRoute(Msg),
    /// the connect timeout passed
    ConnectTimeout,
    /// the total timeout passed: bytes of the body so far and the length announced
    Timeout { got: u64, of: Option<u64> },
    /// the connection ended by a RST or an ICMP error after it was set up
    Reset(TcpEnd),
    /// it closed before any byte of the answer
    Empty,
    /// it closed inside the answer
    Truncated { got: u64, of: Option<u64> },
    /// the answer could not be read
    Bad(HttpErr),
    /// the connection was closed under it (the owner, a pipelined neighbour)
    Aborted,
}

impl Fail {
    /// German, for status lines and notes
    pub fn text(&self) -> String {
        self.msg().text()
    }
    /// text() as a message for the app's catalogue
    pub fn msg(&self) -> Msg {
        match self {
            Fail::Resolve(e) => msg!("Name nicht aufgelöst: {}", phrase(e.text())),
            Fail::Connect(e) => msg!("keine Verbindung: {}", phrase(e.text())),
            Fail::NoRoute(w) => msg!("keine Verbindung: {}", w),
            Fail::ConnectTimeout => msg!("Verbindungsaufbau dauerte zu lange"),
            Fail::Timeout { got, .. } => msg!("Zeitüberschreitung nach {} Bytes", got),
            Fail::Reset(e) => msg!("Verbindung abgebrochen: {}", phrase(e.text())),
            Fail::Empty => msg!("Verbindung ohne Antwort geschlossen"),
            Fail::Truncated { got, .. } => msg!("Antwort unvollständig ({} Bytes)", got),
            Fail::Bad(e) => msg!("Antwort nicht lesbar: {}", e.text()),
            Fail::Aborted => msg!("abgebrochen"),
        }
    }
    /// a timeout (total or connect)
    pub fn is_timeout(&self) -> bool {
        matches!(self, Fail::Timeout { .. } | Fail::ConnectTimeout) || *self == Fail::Connect(TcpEnd::Timeout)
    }
}

/// a finished request
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done {
    pub id: u64,
    pub method: String,
    pub url: Url,
    pub result: Result<Resp, Fail>,
    /// the submit, the address known, the connection up, the request handed to TCP, the first
    /// byte of the answer, the end
    pub start: Ns,
    pub resolved: Option<Ns>,
    pub connected: Option<Ns>,
    pub sent: Option<Ns>,
    pub first: Option<Ns>,
    pub end: Ns,
    /// the number of its connection (0, 1, ... as the Agent opened them) and whether it was
    /// reused
    pub conn: Option<u32>,
    pub reused: bool,
    pub local: Option<(IpAddr, u16)>,
    pub remote: Option<(IpAddr, u16)>,
    /// body bytes (kept or not), bytes of the answer's heads, bytes of the request and of its
    /// head
    pub body_len: u64,
    pub head_len: u64,
    pub req_len: u64,
    pub req_head: u64,
}

/// a step of a request as it happens (when the Agent traces)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEv {
    Trying { id: u64, addr: IpAddr, port: u16 },
    Connected { id: u64, conn: u32, local: (IpAddr, u16), remote: (IpAddr, u16) },
    Reusing { id: u64, conn: u32 },
    /// the connection was not set up (local: the port it tried from)
    ConnectFailed { id: u64, addr: IpAddr, port: u16, local: Option<(IpAddr, u16)>, end: Option<TcpEnd> },
    /// the request went: its head as bytes and the body length
    Sent { id: u64, head: Vec<u8>, body: usize },
    Interim { id: u64, head: Head },
    Head { id: u64, head: Head },
    Data { id: u64, data: Vec<u8> },
    /// after the answer the connection stays (left intact) or closes
    Kept { id: u64, conn: u32 },
    Closing { id: u64, conn: u32 },
    Done(Done),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CState {
    Connecting,
    Open,
}

#[derive(Clone, Debug)]
struct CConn {
    no: u32,
    key: (IpAddr, u16),
    state: CState,
    parser: Parser,
    /// assigned, waiting for the connection
    queue: VecDeque<u64>,
    /// sent, waiting for their answers (the first is answered next)
    inflight: VecDeque<u64>,
    out: Vec<u8>,
    /// it may carry more requests
    reusable: bool,
    /// bytes of the current answer came
    got: bool,
    used: u32,
    timer: Option<u64>,
    local: Option<(IpAddr, u16)>,
}

#[derive(Clone, Debug)]
struct AReq {
    req: Req,
    addr: Option<IpAddr>,
    /// the name's other addresses to try when this one cannot be reached (an answer with both
    /// families)
    alts: VecDeque<IpAddr>,
    sock: Option<u32>,
    start: Ns,
    resolved: Option<Ns>,
    connected: Option<Ns>,
    sent: Option<Ns>,
    first: Option<Ns>,
    head: Option<Head>,
    interim: Vec<Head>,
    body: Vec<u8>,
    body_len: u64,
    head_len: u64,
    req_len: u64,
    req_head: u64,
    conn: Option<u32>,
    reused: bool,
    retried: bool,
    timer: Option<u64>,
    local: Option<(IpAddr, u16)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ATimer {
    Req(u64),
    Connect(u32),
    Idle(u32),
}

/// what an Agent counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgentStats {
    pub requests: u64,
    pub connects: u64,
    pub reused: u64,
    pub retries: u64,
    pub bytes_out: u64,
    pub bytes_in: u64,
}

/// the client side of HTTP in a service
#[derive(Clone, Debug)]
pub struct Agent {
    /// requests in flight on one connection (1: no pipelining)
    pub pipeline: usize,
    /// connections to one host and port at most (0: no limit); more requests wait
    pub max_per_host: usize,
    /// a kept-alive connection without requests closes after this long (0: never)
    pub idle_timeout: Ns,
    pub opts: SockOpts,
    /// record every step (AgentEv)
    pub trace: bool,
    /// the address families names are looked up for (curl -4 / -6)
    pub family: Family,
    reqs: BTreeMap<u64, AReq>,
    conns: BTreeMap<u32, CConn>,
    lookups: BTreeMap<u32, Vec<u64>>,
    waiting: VecDeque<u64>,
    wheel: Wheel<ATimer>,
    next: u64,
    next_conn: u32,
    done: Vec<Done>,
    events: Vec<AgentEv>,
    pub stats: AgentStats,
}

impl Agent {
    /// an Agent that uses timer key `key` of its service
    pub fn new(key: u32) -> Agent {
        Agent {
            pipeline: 1,
            max_per_host: 0,
            idle_timeout: 0,
            opts: SockOpts::default(),
            trace: false,
            family: Family::Any,
            reqs: BTreeMap::new(),
            conns: BTreeMap::new(),
            lookups: BTreeMap::new(),
            waiting: VecDeque::new(),
            wheel: Wheel::new(key),
            next: 0,
            next_conn: 0,
            done: Vec::new(),
            events: Vec::new(),
            stats: AgentStats::default(),
        }
    }
    /// forgets everything (the service stopped: the OS closes its sockets)
    pub fn reset(&mut self) {
        self.reqs.clear();
        self.conns.clear();
        self.lookups.clear();
        self.waiting.clear();
        self.wheel.clear();
        self.done.clear();
        self.events.clear();
    }
    /// requests not finished yet
    pub fn active(&self) -> usize {
        self.reqs.len()
    }
    /// open connections (also those being set up)
    pub fn conns(&self) -> usize {
        self.conns.len()
    }
    /// open connections without a request
    pub fn idle_conns(&self) -> usize {
        self.conns.values().filter(|c| c.state == CState::Open && c.inflight.is_empty() && c.queue.is_empty()).count()
    }
    pub fn owns(&self, sock: u32) -> bool {
        self.conns.contains_key(&sock)
    }
    /// finished requests since the last call
    pub fn take_done(&mut self) -> Vec<Done> {
        core::mem::take(&mut self.done)
    }
    /// the steps since the last call (when it traces)
    pub fn take_events(&mut self) -> Vec<AgentEv> {
        core::mem::take(&mut self.events)
    }
    fn ev(&mut self, e: AgentEv) {
        if self.trace {
            self.events.push(e);
        }
    }

    /// sends a request (when its address and a connection are there); its id
    pub fn submit(&mut self, os: &mut OsApi, req: Req) -> u64 {
        self.next += 1;
        let id = self.next;
        let now = os.now();
        let timer = if req.timeout > 0 { Some(self.wheel.add(now + req.timeout, ATimer::Req(id))) } else { None };
        // an address of the URL (of a family the agent may use); else a name to look up
        let literal = IpAddr::parse(&req.url.host).filter(|a| match self.family {
            Family::Any => true,
            Family::V4 => a.is_v4(),
            Family::V6 => !a.is_v4(),
        });
        let host = req.url.host.clone();
        self.reqs.insert(
            id,
            AReq {
                req,
                addr: literal,
                alts: VecDeque::new(),
                sock: None,
                start: now,
                resolved: literal.map(|_| now),
                connected: None,
                sent: None,
                first: None,
                head: None,
                interim: Vec::new(),
                body: Vec::new(),
                body_len: 0,
                head_len: 0,
                req_len: 0,
                req_head: 0,
                conn: None,
                reused: false,
                retried: false,
                timer,
                local: None,
            },
        );
        self.stats.requests += 1;
        if literal.is_some() {
            self.dispatch(os, id);
        } else {
            let l = os.resolve_host(&host, self.family);
            self.lookups.entry(l).or_default().push(id);
        }
        self.wheel.arm(os);
        id
    }

    /// handles an event of its sockets, lookups and timer; false when it is not one of them
    pub fn on(&mut self, ev: &SvcEvent, os: &mut OsApi) -> bool {
        let used = match *ev {
            SvcEvent::Resolved(l) if self.lookups.contains_key(&l) => {
                self.resolved(os, l);
                true
            }
            SvcEvent::TcpConnected { sock, .. } if self.owns(sock) => {
                self.connected(os, sock);
                true
            }
            SvcEvent::TcpData { sock, .. } if self.owns(sock) => {
                self.read(os, sock, false, None);
                true
            }
            SvcEvent::TcpWritable { sock } if self.owns(sock) => {
                self.flush(os, sock);
                true
            }
            SvcEvent::TcpEof { sock, .. } if self.owns(sock) => {
                self.read(os, sock, true, None);
                true
            }
            SvcEvent::TcpClosed { sock, end } if self.owns(sock) => {
                self.read(os, sock, true, Some(end));
                true
            }
            SvcEvent::Timer(k) if k == self.wheel.key() => {
                self.timer(os);
                true
            }
            _ => false,
        };
        if used {
            self.wheel.arm(os);
        }
        used
    }

    /// closes the kept-alive connections that wait for nothing
    pub fn close_idle(&mut self, os: &mut OsApi) {
        let idle: Vec<u32> = self.conns.iter().filter(|(_, c)| c.state == CState::Open && c.inflight.is_empty() && c.queue.is_empty()).map(|(s, _)| *s).collect();
        for s in idle {
            self.drop_conn(os, s);
        }
        self.wheel.arm(os);
    }

    fn resolved(&mut self, os: &mut OsApi, l: u32) {
        let ids = self.lookups.remove(&l).unwrap_or_default();
        let res = os.lookup(l).and_then(|x| x.result.clone());
        let now = os.now();
        for id in ids {
            match &res {
                Some(Ok(a)) if !a.sorted.is_empty() => {
                    // the others only when both families came (curl goes through the list)
                    let both = a.sorted.iter().any(|x| x.is_v4()) && a.sorted.iter().any(|x| !x.is_v4());
                    if let Some(r) = self.reqs.get_mut(&id) {
                        r.addr = Some(a.sorted[0]);
                        r.alts = if both { a.sorted[1..].iter().copied().collect() } else { VecDeque::new() };
                        r.resolved = Some(now);
                    }
                    self.dispatch(os, id);
                }
                Some(Ok(_)) => self.fail(os, id, Fail::Resolve(LookupErr::NoData)),
                Some(Err(e)) => self.fail(os, id, Fail::Resolve(*e)),
                None => self.fail(os, id, Fail::Resolve(LookupErr::Timeout)),
            }
        }
    }

    /// a connection for a request: a kept-alive one, a new one, or the queue
    fn dispatch(&mut self, os: &mut OsApi, id: u64) {
        let Some(r) = self.reqs.get(&id) else {
            return;
        };
        let Some(addr) = r.addr else {
            return;
        };
        let key = (addr, r.req.url.port);
        let reuse = r.req.reuse;
        if reuse {
            let pipe = self.pipeline.max(1);
            let pick = self.conns.iter().find(|(_, c)| c.key == key && c.state == CState::Open && c.reusable && c.queue.is_empty() && c.inflight.len() < pipe && (c.inflight.is_empty() || pipe > 1)).map(|(s, _)| *s);
            // with pipelining a connection being set up takes more requests
            let setting_up = if pipe > 1 { self.conns.iter().find(|(_, c)| c.key == key && c.state == CState::Connecting && c.reusable && c.queue.len() < pipe).map(|(s, _)| *s) } else { None };
            if pick.is_none()
                && let Some(s) = setting_up
            {
                let no = self.conns.get(&s).map(|c| c.no).unwrap_or(0);
                if let Some(c) = self.conns.get_mut(&s) {
                    c.queue.push_back(id);
                }
                if let Some(r) = self.reqs.get_mut(&id) {
                    r.conn = Some(no);
                    r.sock = Some(s);
                }
                return;
            }
            if let Some(s) = pick {
                let no = self.conns.get(&s).map(|c| c.no).unwrap_or(0);
                if let Some(c) = self.conns.get_mut(&s)
                    && let Some(t) = c.timer.take()
                {
                    self.wheel.cancel(t);
                }
                if let Some(r) = self.reqs.get_mut(&id) {
                    r.reused = true;
                    r.conn = Some(no);
                    r.sock = Some(s);
                    r.connected = Some(os.now());
                }
                self.stats.reused += 1;
                self.ev(AgentEv::Reusing { id, conn: no });
                self.send(os, s, id, true);
                return;
            }
        }
        if self.max_per_host > 0 && self.conns.values().filter(|c| c.key == key).count() >= self.max_per_host {
            if !self.waiting.contains(&id) {
                self.waiting.push_back(id);
            }
            return;
        }
        self.waiting.retain(|w| *w != id);
        self.ev(AgentEv::Trying { id, addr, port: key.1 });
        let ct = self.reqs.get(&id).map(|r| r.req.connect_timeout).unwrap_or(0);
        match os.tcp_connect_msg(addr, key.1, self.opts) {
            Ok(s) => {
                let no = self.next_conn;
                self.next_conn += 1;
                self.stats.connects += 1;
                let local = os.tcp_info(s).map(|r| r.local);
                let timer = if ct > 0 { Some(self.wheel.add(os.now() + ct, ATimer::Connect(s))) } else { None };
                let mut parser = Parser::responses();
                parser.max_body = u64::MAX;
                let mut q = VecDeque::new();
                q.push_back(id);
                self.conns.insert(s, CConn { no, key, state: CState::Connecting, parser, queue: q, inflight: VecDeque::new(), out: Vec::new(), reusable: reuse, got: false, used: 0, timer, local });
                if let Some(r) = self.reqs.get_mut(&id) {
                    r.sock = Some(s);
                    r.conn = Some(no);
                    r.local = local;
                }
            }
            Err(e) => {
                self.ev(AgentEv::ConnectFailed { id, addr, port: key.1, local: None, end: None });
                if !self.next_addr(os, id) {
                    self.fail(os, id, Fail::NoRoute(e));
                }
            }
        }
    }

    /// the connection to a request's address could not be set up: its next address, when the
    /// name gave more of both families; false when there is none
    fn next_addr(&mut self, os: &mut OsApi, id: u64) -> bool {
        let Some(r) = self.reqs.get_mut(&id) else {
            return false;
        };
        let Some(next) = r.alts.pop_front() else {
            return false;
        };
        r.addr = Some(next);
        r.sock = None;
        r.conn = None;
        r.local = None;
        self.dispatch(os, id);
        true
    }

    fn connected(&mut self, os: &mut OsApi, sock: u32) {
        let now = os.now();
        let Some(c) = self.conns.get_mut(&sock) else {
            return;
        };
        c.state = CState::Open;
        if let Some(t) = c.timer.take() {
            self.wheel.cancel(t);
        }
        let info = os.tcp_info(sock);
        let (local, remote) = info.map(|r| (r.local, r.remote)).unwrap_or(((IpAddr::V4(Ipv4::UNSPECIFIED), 0), (c.key.0, c.key.1)));
        c.local = Some(local);
        let no = c.no;
        let ids: Vec<u64> = c.queue.drain(..).collect();
        for (k, id) in ids.iter().enumerate() {
            if let Some(r) = self.reqs.get_mut(id) {
                r.connected = Some(now);
                r.local = Some(local);
            }
            if k == 0 {
                self.ev(AgentEv::Connected { id: *id, conn: no, local, remote });
            }
            // what waited goes in one write
            self.send(os, sock, *id, false);
        }
        self.flush(os, sock);
    }

    /// hands a request to its connection (`write`: write it at once)
    fn send(&mut self, os: &mut OsApi, sock: u32, id: u64, write: bool) {
        let now = os.now();
        let Some(r) = self.reqs.get_mut(&id) else {
            return;
        };
        let Some(c) = self.conns.get_mut(&sock) else {
            return;
        };
        let q = &r.req;
        let mut h = Headers::new();
        if q.headers.get("host").is_none() {
            h.add("Host", &q.url.host_header());
        }
        for (k, v) in &q.headers.0 {
            h.add(k, v);
        }
        if !q.reuse && h.get("connection").is_none() && q.version == Version::Http11 {
            h.add("Connection", "close");
        }
        let bytes = http::encode_request(&q.method, &q.url.target, q.version, &h, &q.body);
        let head_len = bytes.len() - q.body.len();
        c.parser.expect(&q.method);
        c.inflight.push_back(id);
        c.used += 1;
        c.out.extend_from_slice(&bytes);
        r.sent = Some(now);
        r.req_len = bytes.len() as u64;
        r.req_head = head_len as u64;
        r.sock = Some(sock);
        self.stats.bytes_out += bytes.len() as u64;
        let ev = AgentEv::Sent { id, head: bytes[..head_len].to_vec(), body: bytes.len() - head_len };
        self.ev(ev);
        if write {
            self.flush(os, sock);
        }
    }

    fn flush(&mut self, os: &mut OsApi, sock: u32) {
        let Some(c) = self.conns.get_mut(&sock) else {
            return;
        };
        if c.out.is_empty() || c.state != CState::Open {
            return;
        }
        let n = os.tcp_send(sock, &c.out);
        c.out.drain(..n);
    }

    /// what came on a connection; at its end (`fin`: the peer's FIN, `end`: how TCP ended it)
    /// what is missing fails or goes again
    fn read(&mut self, os: &mut OsApi, sock: u32, fin: bool, end: Option<TcpEnd>) {
        let now = os.now();
        let data = os.tcp_recv(sock, usize::MAX);
        self.stats.bytes_in += data.len() as u64;
        // a FIN ends a body that runs until the close; a RST ends nothing in order
        let eof = fin && matches!(end, None | Some(TcpEnd::Closed));
        let connecting = self.conns.get(&sock).is_some_and(|c| c.state == CState::Connecting);
        if connecting {
            // the connection was never set up
            let Some(c) = self.conns.remove(&sock) else {
                return;
            };
            if let Some(t) = c.timer {
                self.wheel.cancel(t);
            }
            os.tcp_close(sock);
            for id in c.queue {
                self.ev(AgentEv::ConnectFailed { id, addr: c.key.0, port: c.key.1, local: c.local, end });
                if !self.next_addr(os, id) {
                    self.fail(os, id, Fail::Connect(end.unwrap_or(TcpEnd::Closed)));
                }
            }
            self.pump(os);
            return;
        }
        let mut evs: Vec<Result<Event, HttpErr>> = Vec::new();
        if let Some(c) = self.conns.get_mut(&sock) {
            if !data.is_empty() {
                c.got = true;
                let mut e = c.parser.push(&data);
                loop {
                    let stop = !matches!(e, Ok(Event::Head(_)) | Ok(Event::Data(_)) | Ok(Event::End(_)));
                    evs.push(e);
                    if stop {
                        break;
                    }
                    e = c.parser.poll();
                }
            }
            if eof {
                let mut e = c.parser.finish();
                loop {
                    let stop = !matches!(e, Ok(Event::Head(_)) | Ok(Event::Data(_)) | Ok(Event::End(_)));
                    evs.push(e);
                    if stop {
                        break;
                    }
                    e = c.parser.poll();
                }
            }
        }
        let mut closed = false;
        for e in evs {
            if closed {
                break;
            }
            match e {
                Ok(Event::More) => {}
                Ok(Event::Head(h)) => {
                    let Some(id) = self.conns.get(&sock).and_then(|c| c.inflight.front().copied()) else {
                        continue;
                    };
                    if let Some(r) = self.reqs.get_mut(&id) {
                        r.first.get_or_insert(now);
                        r.head_len += h.len as u64;
                        if h.status < 200 {
                            r.interim.push(h.clone());
                        } else {
                            r.head = Some(h.clone());
                        }
                    }
                    if h.status < 200 {
                        self.ev(AgentEv::Interim { id, head: h });
                    } else {
                        self.ev(AgentEv::Head { id, head: h });
                    }
                }
                Ok(Event::Data(d)) => {
                    let Some(id) = self.conns.get(&sock).and_then(|c| c.inflight.front().copied()) else {
                        continue;
                    };
                    if let Some(r) = self.reqs.get_mut(&id) {
                        r.body_len += d.len() as u64;
                        if r.req.keep_body {
                            r.body.extend_from_slice(&d);
                        }
                    }
                    if self.trace {
                        self.events.push(AgentEv::Data { id, data: d });
                    }
                }
                Ok(Event::End(tr)) => {
                    let Some(id) = self.conns.get_mut(&sock).and_then(|c| {
                        c.got = false;
                        c.inflight.pop_front()
                    }) else {
                        continue;
                    };
                    let keep = self.reqs.get(&id).is_some_and(|r| r.req.reuse && r.head.as_ref().is_some_and(|h| h.keep_alive()));
                    let no = self.conns.get(&sock).map(|c| c.no).unwrap_or(0);
                    if let Some(c) = self.conns.get_mut(&sock)
                        && !keep
                    {
                        c.reusable = false;
                    }
                    let still = self.conns.get(&sock).is_some_and(|c| c.reusable) && !fin;
                    if still {
                        self.ev(AgentEv::Kept { id, conn: no });
                    } else {
                        self.ev(AgentEv::Closing { id, conn: no });
                    }
                    self.complete(os, id, tr);
                    if !still {
                        closed = true;
                    }
                }
                Err(e) => {
                    // the answer cannot be read, or the connection ended inside it
                    let id = self.conns.get(&sock).and_then(|c| c.inflight.front().copied());
                    if let Some(id) = id {
                        let no = self.conns.get(&sock).map(|c| c.no).unwrap_or(0);
                        self.ev(AgentEv::Closing { id, conn: no });
                        let fail = match e {
                            HttpErr::Truncated => {
                                let (got, of) = self.reqs.get(&id).map(|r| (r.body_len, r.head.as_ref().and_then(|h| h.headers.content_length().ok().flatten()))).unwrap_or((0, None));
                                Fail::Truncated { got, of }
                            }
                            e => Fail::Bad(e),
                        };
                        if let Some(c) = self.conns.get_mut(&sock) {
                            c.inflight.pop_front();
                        }
                        self.fail(os, id, fail);
                    }
                    closed = true;
                }
            }
        }
        if closed || fin {
            self.lose_conn(os, sock, eof, end);
        } else {
            self.after_answer(os, sock);
        }
        self.pump(os);
    }

    /// a connection that answered everything: idle, or the next pipelined request
    fn after_answer(&mut self, os: &mut OsApi, sock: u32) {
        let now = os.now();
        let Some(c) = self.conns.get_mut(&sock) else {
            return;
        };
        if c.inflight.is_empty() && c.queue.is_empty() && c.state == CState::Open && c.timer.is_none() && self.idle_timeout > 0 {
            c.timer = Some(self.wheel.add(now + self.idle_timeout, ATimer::Idle(sock)));
        }
    }

    /// a connection ends (`eof`: in order, with the peer's FIN): what is left on it goes again
    /// (once, when it got no byte and the connection was reused or the request was not the
    /// first on it) or fails
    fn lose_conn(&mut self, os: &mut OsApi, sock: u32, eof: bool, end: Option<TcpEnd>) {
        let Some(c) = self.conns.remove(&sock) else {
            return;
        };
        if let Some(t) = c.timer {
            self.wheel.cancel(t);
        }
        os.tcp_close(sock);
        let mut first = true;
        for id in c.inflight.iter().chain(c.queue.iter()).copied().collect::<Vec<u64>>() {
            let (reused, retried, got) = self.reqs.get(&id).map(|r| (r.reused, r.retried, r.first.is_some())).unwrap_or((false, true, true));
            let retry = !got && !retried && (reused || !first);
            if retry {
                self.retry(os, id);
            } else if first {
                let f = if eof {
                    if got {
                        let (gotn, of) = self.reqs.get(&id).map(|r| (r.body_len, r.head.as_ref().and_then(|h| h.headers.content_length().ok().flatten()))).unwrap_or((0, None));
                        Fail::Truncated { got: gotn, of }
                    } else {
                        Fail::Empty
                    }
                } else {
                    match end {
                        Some(e) => Fail::Reset(e),
                        None => Fail::Aborted,
                    }
                };
                self.fail(os, id, f);
            } else {
                self.fail(os, id, Fail::Aborted);
            }
            first = false;
        }
    }

    /// a request goes again on a new connection
    fn retry(&mut self, os: &mut OsApi, id: u64) {
        if let Some(r) = self.reqs.get_mut(&id) {
            r.retried = true;
            r.reused = false;
            r.sock = None;
            r.head = None;
            r.interim.clear();
            r.body.clear();
            r.body_len = 0;
            r.head_len = 0;
            r.first = None;
        }
        self.stats.retries += 1;
        if let Some(r) = self.reqs.get_mut(&id) {
            r.req.reuse = false;
        }
        self.dispatch(os, id);
        if let Some(r) = self.reqs.get_mut(&id) {
            r.req.reuse = true;
        }
    }

    /// requests that waited for a connection
    fn pump(&mut self, os: &mut OsApi) {
        let n = self.waiting.len();
        for _ in 0..n {
            let Some(id) = self.waiting.pop_front() else {
                break;
            };
            self.dispatch(os, id);
        }
    }

    fn timer(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (_, t) in self.wheel.due(now) {
            match t {
                ATimer::Req(id) => {
                    let Some(r) = self.reqs.get_mut(&id) else {
                        continue;
                    };
                    r.timer = None;
                    let (got, of) = (r.body_len, r.head.as_ref().and_then(|h| h.headers.content_length().ok().flatten()));
                    let sock = r.sock;
                    self.lookups.values_mut().for_each(|v| v.retain(|x| *x != id));
                    self.waiting.retain(|x| *x != id);
                    // the connection it is on closes with it; what else is on it goes again
                    let on = sock.and_then(|s| {
                        let c = self.conns.get_mut(&s)?;
                        c.inflight.retain(|x| *x != id);
                        c.queue.retain(|x| *x != id);
                        Some((s, c.no, c.inflight.iter().chain(c.queue.iter()).copied().collect::<Vec<u64>>()))
                    });
                    if let Some((s, no, rest)) = on {
                        self.ev(AgentEv::Closing { id, conn: no });
                        self.drop_conn(os, s);
                        for x in rest {
                            self.retry(os, x);
                        }
                    }
                    self.fail(os, id, Fail::Timeout { got, of });
                }
                ATimer::Connect(s) => {
                    let Some(c) = self.conns.get(&s) else {
                        continue;
                    };
                    if c.state != CState::Connecting {
                        continue;
                    }
                    let Some(c) = self.conns.remove(&s) else {
                        continue;
                    };
                    os.tcp_close(s);
                    for id in c.queue {
                        self.ev(AgentEv::ConnectFailed { id, addr: c.key.0, port: c.key.1, local: c.local, end: None });
                        self.fail(os, id, Fail::ConnectTimeout);
                    }
                }
                ATimer::Idle(s) => {
                    let idle = self.conns.get(&s).is_some_and(|c| c.inflight.is_empty() && c.queue.is_empty());
                    if idle {
                        if let Some(c) = self.conns.get_mut(&s) {
                            c.timer = None;
                        }
                        self.drop_conn(os, s);
                    }
                }
            }
        }
        self.pump(os);
    }

    fn drop_conn(&mut self, os: &mut OsApi, s: u32) {
        if let Some(c) = self.conns.remove(&s)
            && let Some(t) = c.timer
        {
            self.wheel.cancel(t);
        }
        os.tcp_close(s);
    }

    fn finish(&mut self, id: u64, now: Ns, result: Result<Resp, Fail>) -> Option<Done> {
        let r = self.reqs.remove(&id)?;
        if let Some(t) = r.timer {
            self.wheel.cancel(t);
        }
        let remote = r.addr.map(|a| (a, r.req.url.port));
        Some(Done {
            id,
            method: r.req.method,
            url: r.req.url,
            result,
            start: r.start,
            resolved: r.resolved,
            connected: r.connected,
            sent: r.sent,
            first: r.first,
            end: now,
            conn: r.conn,
            reused: r.reused,
            local: r.local,
            remote,
            body_len: r.body_len,
            head_len: r.head_len,
            req_len: r.req_len,
            req_head: r.req_head,
        })
    }

    fn complete(&mut self, os: &mut OsApi, id: u64, trailers: Headers) {
        let now = os.now();
        let Some(r) = self.reqs.get_mut(&id) else {
            return;
        };
        let resp = Resp { head: r.head.take().unwrap_or_default(), body: core::mem::take(&mut r.body), trailers, interim: core::mem::take(&mut r.interim) };
        let status = resp.head.status;
        if let Some(d) = self.finish(id, now, Ok(resp)) {
            os.note(Note::HttpReply { status, ms: ((d.end - d.start) / crate::kernel::MS) as u32 });
            self.push_done(d);
        }
    }

    fn fail(&mut self, os: &mut OsApi, id: u64, f: Fail) {
        let now = os.now();
        self.waiting.retain(|x| *x != id);
        let timeout = f.is_timeout();
        if let Some(d) = self.finish(id, now, Err(f)) {
            os.note(Note::HttpFailed { timeout });
            self.push_done(d);
        }
    }

    fn push_done(&mut self, d: Done) {
        if self.trace {
            self.events.push(AgentEv::Done(d.clone()));
        }
        self.done.push(d);
    }
}
