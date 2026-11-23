// server -- a blocking HTTP/1.1 server: one accept thread, a pool of worker threads, each
// worker serves one connection at a time (keep-alive, pipelined requests in order)
//
//   Server::start(&addr, ServerConfig, handler) -> NetResult<Server>
//   Server::start_with(.., Some(wrap))   wrap turns an accepted TcpStream into the Stream HTTP
//                                        runs over (TLS) and the identity it proved
//   server.local_addr() / server.stop()
//
// (G7d) A handler's answer with status 101 and Body::Upgrade (ws::Acceptor::accept) takes the
// connection out of HTTP: the head goes out, the upgrade runs on a thread of its own (the worker
// is free again), and stop() sets the Upgraded's stop flag and waits for those threads too.
//
// The handler runs on a worker thread; it must not panic (panics abort the process).
// Requests with both Content-Length and Transfer-Encoding are refused (request smuggling).
// A slow client holds a worker only so long: the head must arrive within read_ms, a body at
// MIN_BODY_RATE on average after a first read_ms (over TLS too: records without data end a read
// after read_ms).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use plat::net::{NetResult, SocketAddr, TcpListener, TcpStream};
use plat::sync::{channel, Mutex, Receiver};

use crate::http::{self, Body, ReadError, Reader, Request, Response, Upgrade, Upgraded};
use crate::Stream;

/// the slowest request body a server waits for, in bytes a second on average (after a first
/// read_ms): slower ones end the connection
pub const MIN_BODY_RATE: u32 = 1024;

#[derive(Clone, Copy, Debug)]
pub struct ServerConfig {
    /// worker threads (connections served at once)
    pub threads: usize,
    /// the largest request body
    pub max_body: usize,
    /// how long an idle keep-alive connection stays open
    pub idle_ms: u32,
    /// the longest wait for the next bytes of a request in progress; the head must be complete
    /// within it, a body must bring 1 KiB a second on average after it (MIN_BODY_RATE)
    pub read_ms: u32,
    pub write_ms: u32,
    /// requests per connection
    pub max_requests: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig { threads: 8, max_body: 64 << 20, idle_ms: 30_000, read_ms: 30_000, write_ms: 60_000, max_requests: 1000 }
    }
}

pub type Handler = Arc<dyn Fn(&Request) -> Response + Send + Sync>;
/// turns an accepted connection into the stream HTTP runs over and the identity it proved
/// (None: drop the connection, e.g. a failed TLS handshake)
pub type Wrap = Arc<dyn Fn(TcpStream) -> Option<(Box<dyn Stream>, Option<String>)> + Send + Sync>;

pub struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    threads: Vec<plat::Thread>,
    upgraded: Upgrades,
}

type Queue = Arc<Mutex<Receiver<(TcpStream, SocketAddr)>>>;
/// (G7d) the threads of upgraded connections, each with the flag it sets when it is done
type Upgrades = Arc<Mutex<Vec<(Arc<AtomicBool>, plat::Thread)>>>;

impl Server {
    pub fn start(addr: &SocketAddr, cfg: ServerConfig, handler: Handler) -> NetResult<Server> {
        Server::start_with(addr, cfg, handler, None)
    }

    pub fn start_with(addr: &SocketAddr, cfg: ServerConfig, handler: Handler, wrap: Option<Wrap>) -> NetResult<Server> {
        let listener = TcpListener::bind(addr)?;
        let local = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let upgraded: Upgrades = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = channel::<(TcpStream, SocketAddr)>(cfg.threads.max(1) * 4);
        let rx: Queue = Arc::new(Mutex::new(rx));
        let mut threads = Vec::new();
        for _ in 0..cfg.threads.max(1) {
            let (rx, handler, wrap, stop, up) = (rx.clone(), handler.clone(), wrap.clone(), stop.clone(), upgraded.clone());
            if let Some(t) = plat::thread_spawn(256 * 1024, Box::new(move || worker(rx, cfg, handler, wrap, stop, up))) {
                threads.push(t);
            }
        }
        let st = stop.clone();
        let accept = plat::thread_spawn(64 * 1024, Box::new(move || {
            while !st.load(Ordering::Relaxed) {
                match listener.accept_timeout(200) {
                    Ok(Some(c)) => {
                        if tx.send(c).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(_) => plat::sleep_ms(20),
                }
            }
            // dropping tx ends the workers once the queue is empty
        }));
        if let Some(t) = accept {
            threads.push(t);
        }
        Ok(Server { addr: local, stop, threads, upgraded })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// (G7d) upgraded connections (WebSockets) still running
    pub fn upgraded(&self) -> usize {
        self.upgraded.lock().iter().filter(|(done, _)| !done.load(Ordering::Acquire)).count()
    }

    /// stop accepting, finish the requests in progress, wait for the threads; upgraded
    /// connections see their stop flag set and are waited for as well
    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads {
            t.join();
        }
        let up = core::mem::take(&mut *self.upgraded.lock());
        for (_, t) in up {
            t.join();
        }
    }
}

/// (G7d) run an upgrade on a thread of its own; finished ones are joined on the way
fn spawn_upgrade(upgrades: &Upgrades, up: Upgrade, conn: Upgraded) {
    let done = Arc::new(AtomicBool::new(false));
    let d = done.clone();
    let t = plat::thread_spawn(256 * 1024, Box::new(move || {
        up.run(conn);
        d.store(true, Ordering::Release);
    }));
    let mut list = upgrades.lock();
    let mut i = 0;
    while i < list.len() {
        if list[i].0.load(Ordering::Acquire) {
            let (_, t) = list.swap_remove(i);
            t.join();
        } else {
            i += 1;
        }
    }
    if let Some(t) = t {
        list.push((done, t));
    }
}

fn worker(rx: Queue, cfg: ServerConfig, handler: Handler, wrap: Option<Wrap>, stop: Arc<AtomicBool>, upgrades: Upgrades) {
    loop {
        let next = rx.lock().recv();
        let Some((tcp, peer)) = next else { return };
        let _ = tcp.set_nodelay(true);
        let (stream, id): (Box<dyn Stream>, Option<String>) = match &wrap {
            Some(w) => {
                tcp.set_read_timeout(Some(cfg.read_ms));
                tcp.set_write_timeout(Some(cfg.write_ms));
                match w(tcp) {
                    Some(x) => x,
                    None => continue,
                }
            }
            None => (Box::new(tcp), None),
        };
        if let Some((up, buffered)) = serve(&*stream, peer, id, &cfg, &handler, &stop) {
            stream.set_timeouts(None, None);
            spawn_upgrade(&upgrades, up, Upgraded { stream, buffered, peer: Some(peer), stop: stop.clone() });
        }
    }
}

/// how long and how much a connection ended by an error answer still reads (see linger)
const LINGER_MS: u64 = 2000;
const LINGER_BYTES: usize = 1 << 20;

/// answer an error and end the connection (-> None: serve's end without an upgrade)
fn error<T>(s: &dyn Stream, status: u16, stop: &AtomicBool) -> Option<T> {
    let mut r = Response::text(status, http::reason(status));
    if http::write_response(s, &mut r, false, false) {
        linger(s, stop);
    }
    None
}

/// end a connection whose request was not read to its end: send the end, then read and drop
/// what the client still sends until it closes (at most LINGER_MS / LINGER_BYTES).  Closing a
/// socket with unread bytes sends a reset, and a reset destroys the answer the client has not
/// read yet (Windows drops its received bytes).
fn linger(s: &dyn Stream, stop: &AtomicBool) {
    s.shutdown();
    s.set_timeouts(Some(250), Some(250));
    let end = plat::time_us() + LINGER_MS * 1000;
    let mut buf = alloc::vec![0u8; 16 * 1024];
    let mut left = LINGER_BYTES;
    while left > 0 && !stop.load(Ordering::Relaxed) && plat::time_us() < end {
        if s.wait_readable(250) {
            match s.read(&mut buf) {
                Ok(n) if n > 0 => left = left.saturating_sub(n),
                _ => return,
            }
        }
    }
}

/// serve the requests of one connection -> an upgrade that takes it over (101 + Body::Upgrade),
/// with the bytes read after its request
fn serve(s: &dyn Stream, peer: SocketAddr, id: Option<String>, cfg: &ServerConfig, handler: &Handler, stop: &AtomicBool) -> Option<(Upgrade, Vec<u8>)> {
    let mut rd = Reader::new(s);
    for n in 0..cfg.max_requests {
        if n > 0 && !rd.has_buffered() {
            // idle keep-alive: wait in slices so a stop is noticed
            let mut waited = 0;
            loop {
                if stop.load(Ordering::Relaxed) {
                    s.shutdown();
                    return None;
                }
                if s.wait_readable(250) {
                    break;
                }
                waited += 250;
                if waited >= cfg.idle_ms {
                    s.shutdown();
                    return None;
                }
            }
        }
        s.set_timeouts(Some(cfg.read_ms), Some(cfg.write_ms));
        rd.set_deadline(Some(plat::time_us() + cfg.read_ms as u64 * 1000));
        let head = rd.head();
        rd.set_deadline(None);
        let (start, headers) = match head {
            Ok(x) => x,
            // the client's end (its close_notify over TLS) is answered with ours (RFC 8446 6.1)
            Err(ReadError::Closed) | Err(ReadError::Net(_)) => {
                s.shutdown();
                return None;
            }
            Err(ReadError::HeadTooLarge) => return error(s, 431, stop),
            Err(_) => return error(s, 400, stop),
        };
        let mut parts = start.split(' ');
        let (Some(method), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            return error(s, 400, stop);
        };
        let http11 = match version {
            "HTTP/1.1" => true,
            "HTTP/1.0" => false,
            _ => return error(s, 505, stop),
        };
        if method.is_empty() || !method.bytes().all(|b| b.is_ascii_uppercase()) || !(target.starts_with('/') || target == "*") {
            return error(s, 400, stop);
        }
        if target.len() > 8 * 1024 {
            return error(s, 414, stop);
        }
        if headers.get("transfer-encoding").is_some() && headers.get("content-length").is_some() {
            return error(s, 400, stop);
        }
        let keep = if http11 { !headers.has_token("connection", "close") } else { headers.has_token("connection", "keep-alive") };
        if headers.has_token("expect", "100-continue") {
            match headers.content_length() {
                Err(()) => return error(s, 400, stop),
                Ok(Some(n)) if n > cfg.max_body as u64 => return error(s, 413, stop),
                _ => {}
            }
            if s.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").is_err() {
                return None;
            }
        }
        // the body must keep moving: MIN_BODY_RATE bytes a second on average after a first
        // read_ms (the head has its deadline; a body sent a byte at a time held a worker)
        rd.set_min_rate(Some((MIN_BODY_RATE, cfg.read_ms)));
        let body = rd.body(&headers, cfg.max_body, false);
        rd.set_min_rate(None);
        let body = match body {
            Ok(b) => b,
            Err(ReadError::BodyTooLarge) => return error(s, 413, stop),
            Err(ReadError::Net(_)) | Err(ReadError::Closed) => {
                s.shutdown();
                return None;
            }
            Err(_) => return error(s, 400, stop),
        };
        let req = Request { method: String::from(method), target: String::from(target), headers, body, peer: Some(peer), client_id: id.clone() };
        let mut resp = handler(&req);
        // (G7d) a protocol takes the connection over: the head, then the upgrade's thread
        if resp.status == 101 {
            let Body::Upgrade(up) = core::mem::replace(&mut resp.body, Body::Empty) else {
                return error(s, 500, stop);
            };
            if !http::write_switch(s, &resp) {
                return None;
            }
            return Some((up, rd.take_buffered()));
        }
        let keep = keep && !stop.load(Ordering::Relaxed) && !resp.headers.has_token("connection", "close");
        if !http::write_response(s, &mut resp, method == "HEAD", keep) || !keep {
            s.shutdown();
            return None;
        }
    }
    s.shutdown();
    None
}
