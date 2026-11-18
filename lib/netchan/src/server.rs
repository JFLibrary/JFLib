// server -- a game server's channels (G7d): UDP and WebSocket clients in one table
//
//   Server::new(ServerConfig) -> Server
//   server.bind_udp(&addr) -> NetResult<SocketAddr>        a UDP socket with its reading thread
//   server.listen_ws(&addr, "/game") -> NetResult<SocketAddr>   a netio HTTP server for the
//                                        WebSockets (404 elsewhere); or server.ws_endpoint() and
//                                        endpoint.accept(&req) in the app's own handler (with
//                                        its page's files beside it, say)
//   server.poll() -> Option<ServerEvent>   send(id, payload) -> seq   disconnect(id)
//   server.wait(ms)                        sleep until something arrives (a loop without a tick)
//   connections() / ids() / stats(id) / peer(id) / server_stats() / stop()
//
// Threads: one reads the UDP socket; each WebSocket has a reading one (netio's thread of the
// upgrade) and a writing one, so neither a full socket buffer nor a slow client holds the
// game's loop.  What they read goes into one bounded queue (UDP drops when it is full, a
// WebSocket waits); poll() takes it on the caller's thread, where the protocol runs
// (endpoint::ServerCore).  send to a WebSocket client with LINK_UNSENT bytes waiting fails with
// Error::Backlog; one that still does not take what the server's own packets add is closed.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use netio::ws::{self as nws, Message};
use netio::{Request, Response};
use plat::net::{NetResult, SocketAddr, UdpSocket};
use plat::sync::{channel, Condvar, Mutex, Receiver, Sender};

use crate::conn::Stats;
use crate::endpoint::{Out, Route, ServerConfig, ServerCore, ServerEvent, ServerStats};
use crate::packet::{self, DATA_HEADER};
use crate::{Error, Transport};

/// what the reading threads keep until poll takes it
const QUEUE: usize = 8192;
/// bytes a WebSocket client may have waiting to be written; more close it
pub const LINK_UNSENT: usize = 1 << 20;
/// what poll takes from the queue at most per call
const POLL_TAKE: usize = 4096;

enum Inbound {
    Datagram(SocketAddr, Vec<u8>),
    Open(u32, Option<SocketAddr>),
    Msg(u32, Vec<u8>),
    Closed(u32, u16),
}

struct LinkQ {
    q: VecDeque<Vec<u8>>,
    bytes: usize,
    close: bool,
    gone: bool,
}

// the writing side of one WebSocket
struct Link {
    q: Mutex<LinkQ>,
    cv: Condvar,
    close_since: AtomicU64,
}

impl Link {
    fn push(&self, b: Vec<u8>) -> bool {
        let mut q = self.q.lock();
        if q.close || q.gone {
            return true;
        }
        if q.bytes + b.len() > LINK_UNSENT {
            return false;
        }
        q.bytes += b.len();
        q.q.push_back(b);
        drop(q);
        self.cv.notify_all();
        true
    }

    fn unsent(&self) -> usize {
        self.q.lock().bytes
    }

    fn close(&self) {
        self.q.lock().close = true;
        let _ = self.close_since.compare_exchange(0, plat::time_us().max(1), Ordering::AcqRel, Ordering::Acquire);
        self.cv.notify_all();
    }
}

struct Shared {
    tx: Sender<Inbound>,
    links: Mutex<BTreeMap<u32, Arc<Link>>>,
    next_stream: AtomicU32,
    acceptor: nws::Acceptor,
    stop: AtomicBool,
}

/// what an HTTP handler needs to hand WebSockets to a Server (clone it into the handler)
#[derive(Clone)]
pub struct WsEndpoint(Arc<Shared>);

impl WsEndpoint {
    /// answer an upgrade request on the game's path: 101 and the client joins the server's
    /// table once it sent CONNECT, or the Acceptor's 4xx / 503
    pub fn accept(&self, req: &Request) -> Response {
        let sh = self.0.clone();
        self.0.acceptor.accept(req, move |ws| ws_conn(ws, sh))
    }

    /// WebSockets open now (also those that have not sent CONNECT yet)
    pub fn connections(&self) -> usize {
        self.0.acceptor.connections()
    }
}

// queue what a reader read: wait while the queue is full, unless the server stops
fn queue(sh: &Shared, mut v: Inbound) -> bool {
    loop {
        match sh.tx.try_send(v) {
            Ok(()) => return true,
            Err(back) => {
                if sh.stop.load(Ordering::Acquire) {
                    return false;
                }
                v = back;
                plat::sleep_ms(1);
            }
        }
    }
}

// one WebSocket: its link and writer, then reading until it ends (netio's thread of the upgrade)
fn ws_conn(ws: nws::WebSocket, sh: Arc<Shared>) {
    let id = sh.next_stream.fetch_add(1, Ordering::Relaxed);
    let ws = Arc::new(ws);
    let link = Arc::new(Link { q: Mutex::new(LinkQ { q: VecDeque::new(), bytes: 0, close: false, gone: false }), cv: Condvar::new(), close_since: AtomicU64::new(0) });
    sh.links.lock().insert(id, link.clone());
    let (w, l) = (ws.clone(), link.clone());
    if !queue(&sh, Inbound::Open(id, ws.peer())) || !crate::reap::spawn(128 * 1024, move || ws_writer(w, l)) {
        sh.links.lock().remove(&id);
        ws.close(nws::CLOSE_INTERNAL, "");
        return;
    }
    let code = loop {
        match ws.recv_timeout(Some(200)) {
            Ok(Some(Message::Binary(b))) => {
                if !queue(&sh, Inbound::Msg(id, b)) {
                    break nws::CLOSE_GOING_AWAY;
                }
            }
            Ok(Some(Message::Text(_))) => {
                ws.close(nws::CLOSE_UNSUPPORTED, "binary only");
                break nws::CLOSE_UNSUPPORTED;
            }
            Ok(None) => {
                let since = link.close_since.load(Ordering::Acquire);
                if since != 0 && plat::time_us().saturating_sub(since) > 1_000_000 {
                    break nws::CLOSE_NORMAL;
                }
            }
            Err(nws::WsError::Closed(c, _)) => break c,
            Err(nws::WsError::Protocol(c)) => break c,
            Err(_) => break nws::CLOSE_ABNORMAL,
        }
    };
    {
        let mut q = link.q.lock();
        q.gone = true;
    }
    link.cv.notify_all();
    sh.links.lock().remove(&id);
    queue(&sh, Inbound::Closed(id, code));
}

// write what the server queued for one WebSocket; close when asked
fn ws_writer(ws: Arc<nws::WebSocket>, link: Arc<Link>) {
    loop {
        let job = {
            let mut q = link.q.lock();
            loop {
                if let Some(b) = q.q.pop_front() {
                    q.bytes -= b.len();
                    break Some(b);
                }
                if q.close || q.gone {
                    break None;
                }
                q = link.cv.wait_timeout(q, Some(1000)).0;
            }
        };
        match job {
            Some(b) => {
                if ws.send_binary(&b).is_err() {
                    return;
                }
            }
            None => {
                if link.q.lock().close {
                    ws.close(nws::CLOSE_NORMAL, "");
                }
                return;
            }
        }
    }
}

/// a game server's channels (see the top of this file).  Dropping it stops it
pub struct Server {
    core: ServerCore,
    sh: Arc<Shared>,
    rx: Receiver<Inbound>,
    udp: Option<(Arc<UdpSocket>, Arc<AtomicBool>, plat::Thread)>,
    http: Option<netio::Server>,
}

impl Server {
    /// a server with these limits and no transport yet (bind_udp, listen_ws, ws_endpoint)
    pub fn new(cfg: ServerConfig) -> Server {
        let (tx, rx) = channel(QUEUE);
        let lim = nws::Limits {
            max_message: cfg.max_stream_payload + DATA_HEADER,
            max_frame: cfg.max_stream_payload + DATA_HEADER,
            messages_per_s: cfg.packets_per_s.saturating_mul(2).saturating_add(16),
            bytes_per_s: cfg.bytes_per_s.saturating_mul(2).saturating_add(64 * 1024),
            idle_ms: cfg.timeout_ms.saturating_mul(2),
            ping_ms: 0,
            max_connections: cfg.max_connections.saturating_mul(2),
            ..nws::Limits::default()
        };
        let sh = Arc::new(Shared { tx, links: Mutex::new(BTreeMap::new()), next_stream: AtomicU32::new(1), acceptor: nws::Acceptor::new(lim), stop: AtomicBool::new(false) });
        let secret = plat::random_bytes::<32>();
        Server { core: ServerCore::new(cfg, secret, plat::time_us()), sh, rx, udp: None, http: None }
    }

    /// listen for UDP clients on `addr` (port 0: a free one) -> the address it got
    pub fn bind_udp(&mut self, addr: &SocketAddr) -> NetResult<SocketAddr> {
        let sock = Arc::new(UdpSocket::bind(addr)?);
        let local = sock.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st, sh) = (sock.clone(), stop.clone(), self.sh.clone());
        let t = plat::thread_spawn(
            128 * 1024,
            alloc::boxed::Box::new(move || {
                let mut buf = alloc::vec![0u8; 65536];
                while !st.load(Ordering::Acquire) {
                    match s.recv_from(&mut buf, 100) {
                        // (the queue full: dropped, as the network would)
                        Ok((n, from)) => {
                            let _ = sh.tx.try_send(Inbound::Datagram(from, buf[..n].to_vec()));
                        }
                        Err(e) if e.kind == plat::net::NetErrorKind::TimedOut => {}
                        Err(_) => plat::sleep_ms(5),
                    }
                }
            }),
        )
        .ok_or(plat::net::NetError::new(plat::net::NetErrorKind::Other, 0))?;
        if let Some((_, old, t)) = self.udp.replace((sock, stop, t)) {
            old.store(true, Ordering::Release);
            t.join();
        }
        Ok(local)
    }

    /// the handle an HTTP handler hands WebSockets in with
    pub fn ws_endpoint(&self) -> WsEndpoint {
        WsEndpoint(self.sh.clone())
    }

    /// a netio HTTP server on `addr` whose `path` takes the WebSockets (404 elsewhere) -> its
    /// address
    pub fn listen_ws(&mut self, addr: &SocketAddr, path: &str) -> NetResult<SocketAddr> {
        let ep = self.ws_endpoint();
        let path = String::from(path);
        let handler: netio::server::Handler = Arc::new(move |req: &Request| if req.path() == path { ep.accept(req) } else { Response::new(404) });
        let cfg = netio::ServerConfig { threads: 4, max_body: 0, idle_ms: 5000, read_ms: 5000, ..netio::ServerConfig::default() };
        let s = netio::Server::start(addr, cfg, handler)?;
        let a = s.local_addr();
        if let Some(old) = self.http.replace(s) {
            old.stop();
        }
        Ok(a)
    }

    /// the UDP socket's address
    pub fn udp_addr(&self) -> Option<SocketAddr> {
        self.udp.as_ref().and_then(|(s, _, _)| s.local_addr().ok())
    }

    /// the address of listen_ws's HTTP server
    pub fn ws_addr(&self) -> Option<SocketAddr> {
        self.http.as_ref().map(|h| h.local_addr())
    }

    fn take(&mut self, i: Inbound, now: u64) {
        match i {
            Inbound::Datagram(from, d) => self.core.datagram(from, &d, now),
            Inbound::Open(s, a) => self.core.stream_open(s, a, now),
            Inbound::Msg(s, d) => self.core.stream_message(s, &d, now),
            Inbound::Closed(s, c) => self.core.stream_closed(s, c),
        }
    }

    fn take_in(&mut self, now: u64) {
        for _ in 0..POLL_TAKE {
            let Some(i) = self.rx.try_recv() else { break };
            self.take(i, now);
        }
    }

    /// sleep until a datagram or a WebSocket message arrives, at most `ms` (a server without a
    /// tick of its own; keep `ms` below keepalive_ms).  poll() then reports it
    pub fn wait(&mut self, ms: u32) {
        if let Ok(i) = self.rx.recv_timeout(ms) {
            self.take(i, plat::time_us());
        }
    }

    fn flush(&mut self) {
        let mut full: Vec<u32> = Vec::new();
        while let Some(o) = self.core.take_out() {
            match o {
                Out::Send(Route::Udp(a), b) => {
                    if let Some((s, _, _)) = &self.udp {
                        let _ = s.send_to(&b, &a);
                    }
                }
                Out::Send(Route::Stream(id), b) => {
                    let link = self.sh.links.lock().get(&id).cloned();
                    if let Some(l) = link {
                        if !l.push(b) {
                            full.push(id);
                        }
                    }
                }
                Out::Close(id) => {
                    if let Some(l) = self.sh.links.lock().get(&id) {
                        l.close();
                    }
                }
            }
        }
        // a client that does not take what it is sent: closed (Disconnected(Closed) follows)
        for id in full {
            if let Some(l) = self.sh.links.lock().get(&id) {
                l.close();
            }
        }
    }

    /// the next event; call it until None every tick (it takes what came, sends keep-alives,
    /// ends connections that timed out)
    pub fn poll(&mut self) -> Option<ServerEvent> {
        if let Some(e) = self.core.poll_event() {
            return Some(e);
        }
        let now = plat::time_us();
        self.take_in(now);
        self.core.tick(now);
        self.flush();
        self.core.poll_event()
    }

    /// send a message to a client -> its sequence number (Backlog: a WebSocket client has
    /// LINK_UNSENT bytes waiting)
    pub fn send(&mut self, id: u32, payload: &[u8]) -> Result<u16, Error> {
        if let Some(Route::Stream(s)) = self.core.route(id) {
            let link = self.sh.links.lock().get(&s).cloned();
            if link.is_some_and(|l| l.unsent() + payload.len() + DATA_HEADER > LINK_UNSENT) {
                return Err(Error::Backlog);
            }
        }
        let seq = self.core.send(id, payload, plat::time_us())?;
        self.flush();
        Ok(seq)
    }

    /// end a client's connection (Disconnected(Local) follows)
    pub fn disconnect(&mut self, id: u32) {
        self.core.disconnect(id, packet::BYE_NORMAL);
        self.flush();
    }

    /// connections now
    pub fn connections(&self) -> usize {
        self.core.connections()
    }

    /// the ids of the connections
    pub fn ids(&self) -> Vec<u32> {
        self.core.ids()
    }

    /// a connection's counts and round-trip time
    pub fn stats(&self, id: u32) -> Option<Stats> {
        self.core.stats(id)
    }

    /// a connection's transport and address
    pub fn peer(&self, id: u32) -> Option<(Transport, Option<SocketAddr>)> {
        self.core.peer(id)
    }

    /// what the server counted beyond its connections
    pub fn server_stats(&self) -> ServerStats {
        self.core.server_stats()
    }

    /// end every connection, stop the threads and the HTTP server
    pub fn stop(self) {}

    fn shutdown(&mut self) {
        self.core.disconnect_all();
        self.flush();
        self.sh.stop.store(true, Ordering::Release);
        if let Some(h) = self.http.take() {
            h.stop();
        }
        if let Some((_, stop, t)) = self.udp.take() {
            stop.store(true, Ordering::Release);
            t.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}
