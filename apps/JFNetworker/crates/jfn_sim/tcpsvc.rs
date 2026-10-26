// tcpsvc -- small TCP services to try TCP with: an echo server (RFC 862), a discard server
// (RFC 863) and a client that sends a number of bytes and reads what comes back
//
//   kind          config (JSON; every member may be left out except the client's server)
//   tcp-echo      {"port": 7, "backlog": 0, "rcvbuf": 0, "sndbuf": 0, "nodelay": false}
//   tcp-discard   {"port": 9, "backlog": 0, "rcvbuf": 0, "read": true, "read_after_ms": 0}
//   tcp-client    {"server": "10.0.0.2" (or an IPv6 address), "port": 7, "bytes": 10000,
//                  "chunk": 0, "interval_ms": 0, "delay_ms": 0, "close": "shutdown",
//                  "sport": 0, "nodelay": false, "keepalive": false, "rcvbuf": 0, "sndbuf": 0}
//                  (the servers listen on both families: os.rs ANY)
//
//   from_def(kind, &config)   the service of a project's definition (svcact.rs); from_def_msg:
//                             the refusal as a Msg for the app's catalogue
//   pattern(i)                the byte the client sends at position i (i mod 256)
//
// The client connects when it starts (after delay_ms), from `sport` (0: an ephemeral port),
// and writes `bytes` bytes in writes of `chunk` bytes (0: what the send buffer takes) every
// interval_ms (0: as fast as the buffer allows).  Then it ends as `close` says: "shutdown"
// (FIN, read until the server closes, then close), "close", "abort" (RST) or "keep" (stays
// open).  The echo server sends back what it reads and closes after the client's FIN; the
// discard server reads and drops (with "read": false never, so its window fills up;
// read_after_ms starts reading later) and closes after the client's FIN.  Backlog and buffer
// sizes 0 are the system's.
//
// Metrics: the servers count connections (and "active" ones), bytes and resets; the client
// "connected", "sent", "received", "done" (everything sent and an orderly end), "failed",
// "connect_ns" (the handshake from its start) and "done_ns" (from its start to the end).  For
// the time series (series.rs) the discard server's bytes count up and the client's done is a
// flag.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use jfn_model::msg;
use jfn_model::msg::{phrase, Msg};
use netpkt::IpAddr;

use crate::kernel::{Ns, MS};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::series::{metric_kind_of, MetricKind};
use crate::tcp::{SockOpts, TcpEnd};
use crate::trace::Note;

const K_START: u32 = 0;
const K_SEND: u32 = 1;
const K_READ: u32 = 0;
/// the most the client hands to tcp_send at once
const WRITE_MAX: u64 = 65536;

/// the byte the client sends at position `i`
pub fn pattern(i: u64) -> u8 {
    (i % 256) as u8
}

fn listen(os: &mut OsApi, port: u16, backlog: u32, opts: SockOpts, kind: &str) -> Option<u32> {
    match os.tcp_listen(port, backlog, opts) {
        Ok(l) => Some(l),
        Err(_) => {
            os.note(Note::PortInUse { port, svc: String::from(kind) });
            None
        }
    }
}

// ---------------------------------------------------------------- echo

/// the echo server (RFC 862)
#[derive(Clone, Debug)]
pub struct TcpEcho {
    pub port: u16,
    pub backlog: u32,
    pub opts: SockOpts,
    listener: Option<u32>,
    /// per connection: bytes read and not sent back yet, and whether the client's FIN came
    conns: BTreeMap<u32, (Vec<u8>, bool)>,
    pub connections: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub resets: u64,
}

impl TcpEcho {
    pub fn new(port: u16) -> TcpEcho {
        TcpEcho { port, backlog: 0, opts: SockOpts::default(), listener: None, conns: BTreeMap::new(), connections: 0, bytes_in: 0, bytes_out: 0, resets: 0 }
    }
    /// sends back what waits; closes once the client is done and everything went
    fn flush(&mut self, sock: u32, os: &mut OsApi) {
        let Some((buf, eof)) = self.conns.get_mut(&sock) else {
            return;
        };
        if !buf.is_empty() {
            let n = os.tcp_send(sock, buf);
            self.bytes_out += n as u64;
            buf.drain(..n);
        }
        if buf.is_empty() && *eof {
            os.tcp_close(sock);
            self.conns.remove(&sock);
        }
    }
}

impl Service for TcpEcho {
    fn kind(&self) -> &'static str {
        "tcp-echo"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.listener = listen(os, self.port, self.backlog, self.opts, "tcp-echo");
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::TcpAccept { listener, .. } if Some(listener) == self.listener => {
                while let Some(s) = os.tcp_accept(listener) {
                    self.conns.insert(s, (Vec::new(), false));
                    self.connections += 1;
                }
            }
            SvcEvent::TcpData { sock, .. } => {
                let d = os.tcp_recv(sock, usize::MAX);
                self.bytes_in += d.len() as u64;
                if let Some(c) = self.conns.get_mut(&sock) {
                    c.0.extend_from_slice(&d);
                }
                self.flush(sock, os);
            }
            SvcEvent::TcpWritable { sock } => self.flush(sock, os),
            SvcEvent::TcpEof { sock, .. } => {
                if let Some(c) = self.conns.get_mut(&sock) {
                    c.1 = true;
                }
                self.flush(sock, os);
            }
            SvcEvent::TcpClosed { sock, end } => {
                self.conns.remove(&sock);
                os.tcp_close(sock);
                if end == TcpEnd::Reset {
                    self.resets += 1;
                }
            }
            _ => {}
        }
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.listener = None;
        self.conns.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("connections", self.connections as i64),
            Metric::new("active", self.conns.len() as i64),
            Metric::new("bytes_in", self.bytes_in as i64),
            Metric::new("bytes_out", self.bytes_out as i64),
            Metric::new("resets", self.resets as i64),
        ]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        msg!("Port {}: {} Verbindungen, {} Bytes zurückgeschickt", self.port, self.connections, self.bytes_out)
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------- discard

/// the discard server (RFC 863)
#[derive(Clone, Debug)]
pub struct TcpDiscard {
    pub port: u16,
    pub backlog: u32,
    pub opts: SockOpts,
    /// it reads at all, and from how long after its start
    pub read: bool,
    pub read_after: Ns,
    listener: Option<u32>,
    /// per connection: whether the client's FIN came
    conns: BTreeMap<u32, bool>,
    reading: bool,
    pub connections: u64,
    pub bytes: u64,
    pub resets: u64,
}

impl TcpDiscard {
    pub fn new(port: u16) -> TcpDiscard {
        TcpDiscard { port, backlog: 0, opts: SockOpts::default(), read: true, read_after: 0, listener: None, conns: BTreeMap::new(), reading: false, connections: 0, bytes: 0, resets: 0 }
    }
    /// reads what is there; closes after the client's FIN
    fn drain(&mut self, sock: u32, os: &mut OsApi) {
        if !self.reading {
            return;
        }
        self.bytes += os.tcp_recv(sock, usize::MAX).len() as u64;
        if self.conns.get(&sock) == Some(&true) {
            os.tcp_close(sock);
            self.conns.remove(&sock);
        }
    }
}

impl Service for TcpDiscard {
    fn kind(&self) -> &'static str {
        "tcp-discard"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.reading = self.read && self.read_after == 0;
        if self.read && self.read_after > 0 {
            let at = os.now() + self.read_after;
            os.timer(K_READ, at);
        }
        self.listener = listen(os, self.port, self.backlog, self.opts, "tcp-discard");
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::TcpAccept { listener, .. } if Some(listener) == self.listener => {
                while let Some(s) = os.tcp_accept(listener) {
                    self.conns.insert(s, false);
                    self.connections += 1;
                }
            }
            SvcEvent::TcpData { sock, .. } => self.drain(sock, os),
            SvcEvent::TcpEof { sock, .. } => {
                if let Some(e) = self.conns.get_mut(&sock) {
                    *e = true;
                }
                self.drain(sock, os);
            }
            SvcEvent::Timer(K_READ) => {
                self.reading = true;
                let socks: Vec<u32> = self.conns.keys().copied().collect();
                for s in socks {
                    self.drain(s, os);
                }
            }
            SvcEvent::TcpClosed { sock, end } => {
                self.conns.remove(&sock);
                os.tcp_close(sock);
                if end == TcpEnd::Reset {
                    self.resets += 1;
                }
            }
            _ => {}
        }
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.listener = None;
        self.conns.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("connections", self.connections as i64),
            Metric::new("active", self.conns.len() as i64),
            Metric::new("bytes", self.bytes as i64),
            Metric::new("resets", self.resets as i64),
        ]
    }
    fn metric_kind(&self, name: &str) -> MetricKind {
        // the bytes it dropped so far, not what it holds
        if name == "bytes" { MetricKind::Counter } else { metric_kind_of(name) }
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        msg!("Port {}: {} Verbindungen, {} Bytes verworfen", self.port, self.connections, self.bytes)
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------- client

/// how the client ends after it sent everything
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseMode {
    /// FIN, read until the server closes, then close
    Shutdown,
    Close,
    /// RST
    Abort,
    /// stays open
    Keep,
}

/// a client that sends a number of bytes
#[derive(Clone, Debug)]
pub struct TcpClient {
    /// an IPv4 or IPv6 address
    pub server: IpAddr,
    pub port: u16,
    pub bytes: u64,
    /// bytes per write (0: what the send buffer takes)
    pub chunk: u32,
    /// time between writes (0: as fast as the buffer allows)
    pub interval: Ns,
    /// time from the start to the connection attempt
    pub delay: Ns,
    pub close: CloseMode,
    /// the local port (0: an ephemeral one)
    pub sport: u16,
    pub opts: SockOpts,
    sock: Option<u32>,
    closing: bool,
    pub sent: u64,
    pub received: u64,
    /// when it started the connection, when it was established, when it ended and how
    pub started: Option<Ns>,
    pub connected: Option<Ns>,
    pub finished: Option<Ns>,
    pub end: Option<TcpEnd>,
    pub failures: u64,
    /// why connect refused (no route, port in use)
    pub error: Option<String>,
    /// error as a message for the app's catalogue
    pub error_msg: Option<Msg>,
}

impl TcpClient {
    pub fn new(server: impl Into<IpAddr>, port: u16, bytes: u64) -> TcpClient {
        TcpClient {
            server: server.into(),
            port,
            bytes,
            chunk: 0,
            interval: 0,
            delay: 0,
            close: CloseMode::Shutdown,
            sport: 0,
            opts: SockOpts::default(),
            sock: None,
            closing: false,
            sent: 0,
            received: 0,
            started: None,
            connected: None,
            finished: None,
            end: None,
            failures: 0,
            error: None,
            error_msg: None,
        }
    }
    /// the socket while the connection is open
    pub fn sock(&self) -> Option<u32> {
        self.sock
    }
    fn connect(&mut self, os: &mut OsApi) {
        self.started = Some(os.now());
        match os.tcp_connect_from_msg(self.sport, self.server, self.port, self.opts) {
            Ok(s) => self.sock = Some(s),
            Err(e) => {
                self.failures += 1;
                self.error = Some(e.text());
                self.error_msg = Some(e);
            }
        }
    }
    fn finish(&mut self, os: &mut OsApi, end: TcpEnd) {
        self.sock = None;
        self.end = Some(end);
        self.finished = Some(os.now());
        if end != TcpEnd::Closed || self.sent < self.bytes {
            self.failures += 1;
        }
    }
    /// writes what is left, as the buffer and the interval allow; then ends as configured
    fn pump(&mut self, os: &mut OsApi) {
        let Some(s) = self.sock else {
            return;
        };
        if self.connected.is_none() {
            return;
        }
        while self.sent < self.bytes {
            let per = if self.chunk > 0 { self.chunk as u64 } else { WRITE_MAX };
            let want = (self.bytes - self.sent).min(per) as usize;
            let data: Vec<u8> = (0..want as u64).map(|i| pattern(self.sent + i)).collect();
            let n = os.tcp_send(s, &data);
            self.sent += n as u64;
            if n < want {
                // TcpWritable follows
                return;
            }
            if self.interval > 0 && self.sent < self.bytes {
                let at = os.now() + self.interval;
                os.timer(K_SEND, at);
                return;
            }
        }
        if self.closing {
            return;
        }
        self.closing = true;
        match self.close {
            CloseMode::Shutdown => os.tcp_shutdown(s),
            CloseMode::Close => {
                os.tcp_close(s);
                self.finish(os, TcpEnd::Closed);
            }
            CloseMode::Abort => {
                os.tcp_abort(s);
                self.finish(os, TcpEnd::Aborted);
            }
            CloseMode::Keep => {}
        }
    }
}

impl Service for TcpClient {
    fn kind(&self) -> &'static str {
        "tcp-client"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.sock = None;
        self.closing = false;
        self.sent = 0;
        self.received = 0;
        self.connected = None;
        self.finished = None;
        self.end = None;
        if self.delay == 0 {
            self.connect(os);
        } else {
            let at = os.now() + self.delay;
            os.timer(K_START, at);
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Timer(K_START) => self.connect(os),
            SvcEvent::Timer(K_SEND) => self.pump(os),
            SvcEvent::TcpConnected { sock, .. } if Some(sock) == self.sock => {
                self.connected = Some(os.now());
                self.pump(os);
            }
            SvcEvent::TcpWritable { sock } if Some(sock) == self.sock => self.pump(os),
            SvcEvent::TcpData { sock, .. } if Some(sock) == self.sock => {
                self.received += os.tcp_recv(sock, usize::MAX).len() as u64;
            }
            SvcEvent::TcpEof { sock, .. } if Some(sock) == self.sock => {
                // the server is done: so is the client
                self.received += os.tcp_recv(sock, usize::MAX).len() as u64;
                os.tcp_close(sock);
                self.finish(os, TcpEnd::Closed);
            }
            SvcEvent::TcpClosed { sock, end } if Some(sock) == self.sock => {
                // what came last can still be read before the socket goes
                self.received += os.tcp_recv(sock, usize::MAX).len() as u64;
                os.tcp_close(sock);
                self.finish(os, end);
            }
            _ => {}
        }
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.sock = None;
    }
    fn metrics(&self) -> Vec<Metric> {
        let since = |t: Option<Ns>| match (self.started, t) {
            (Some(a), Some(b)) => b.saturating_sub(a) as i64,
            _ => 0,
        };
        let done = self.end == Some(TcpEnd::Closed) && self.sent == self.bytes;
        alloc::vec![
            Metric::new("connected", self.connected.is_some() as i64),
            Metric::new("sent", self.sent as i64),
            Metric::new("received", self.received as i64),
            Metric::new("done", done as i64),
            Metric::new("failed", self.failures as i64),
            Metric::new("connect_ns", since(self.connected)),
            Metric::new("done_ns", since(self.finished)),
        ]
    }
    fn metric_kind(&self, name: &str) -> MetricKind {
        // "done" is a flag here
        if name == "done" { MetricKind::Gauge } else { metric_kind_of(name) }
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        let error = self.error_msg.clone().or_else(|| self.error.clone().map(Msg::plain));
        // an IPv6 server in brackets before its port
        let server = if self.server.is_v4() { alloc::format!("{}", self.server) } else { alloc::format!("[{}]", self.server) };
        match (error, self.end, self.connected) {
            (Some(e), _, _) => msg!("{}:{}: {}", server, self.port, e),
            (None, Some(end), _) => msg!("{}:{}: {} von {} Bytes gesendet, {} empfangen; {}", server, self.port, self.sent, self.bytes, self.received, phrase(end.text())),
            (None, None, Some(_)) => msg!("verbunden mit {}:{}: {} von {} Bytes gesendet, {} empfangen", server, self.port, self.sent, self.bytes, self.received),
            (None, None, None) => msg!("verbindet mit {}:{} ...", server, self.port),
        }
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------- project definitions

/// a TCP service from its definition in a project file (svcact.rs); the German reason when
/// the configuration does not fit
pub fn from_def(kind: &str, c: &Value) -> Result<Box<dyn Service>, String> {
    from_def_msg(kind, c).map_err(|m| m.text())
}

/// from_def, the reason as a message for the app's catalogue
pub fn from_def_msg(kind: &str, c: &Value) -> Result<Box<dyn Service>, Msg> {
    let num = |k: &str, d: u64| c.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
    let flag = |k: &str, d: bool| c.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
    let size = |k: &str| num(k, 0).min(u32::MAX as u64) as u32;
    let port = |d: u16| -> Result<u16, Msg> {
        match num("port", d as u64) {
            p @ 1..=65535 => Ok(p as u16),
            p => Err(msg!("Port {} gibt es nicht.", p)),
        }
    };
    let opts = SockOpts { nodelay: flag("nodelay", false), keepalive: flag("keepalive", false), rcvbuf: size("rcvbuf"), sndbuf: size("sndbuf") };
    Ok(match kind {
        "tcp-echo" => {
            let mut s = TcpEcho::new(port(7)?);
            s.backlog = size("backlog");
            s.opts = opts;
            Box::new(s)
        }
        "tcp-discard" => {
            let mut s = TcpDiscard::new(port(9)?);
            s.backlog = size("backlog");
            s.opts = opts;
            s.read = flag("read", true);
            s.read_after = num("read_after_ms", 0).saturating_mul(MS);
            Box::new(s)
        }
        "tcp-client" => {
            let server = c.get("server").and_then(|v| v.as_str()).and_then(|s| IpAddr::parse(s.trim())).ok_or_else(|| msg!("\u{201e}server\u{201c} fehlt oder ist keine IP-Adresse."))?;
            let mut s = TcpClient::new(server, port(7)?, num("bytes", 10_000));
            s.chunk = size("chunk");
            s.interval = num("interval_ms", 0).saturating_mul(MS);
            s.delay = num("delay_ms", 0).saturating_mul(MS);
            s.sport = num("sport", 0).min(65535) as u16;
            s.opts = opts;
            s.close = match c.get("close").and_then(|v| v.as_str()).unwrap_or("shutdown") {
                "shutdown" => CloseMode::Shutdown,
                "close" => CloseMode::Close,
                "abort" => CloseMode::Abort,
                "keep" => CloseMode::Keep,
                other => return Err(msg!("\u{201e}close\u{201c} kennt {} nicht (shutdown, close, abort, keep).", other)),
            };
            Box::new(s)
        }
        k => return Err(msg!("Unbekannte Dienstart \u{201e}{}\u{201c}.", k)),
    })
}
