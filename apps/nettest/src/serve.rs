// serve -- the server of the browser test and a desktop client against it: the files of a web
// build, /ws netio's echo WebSocket, /chan the channel (and the channel over UDP on the same port
// number), /quit ends it.  Desktop and browser reach the same server (docs/GAME.md G7d).

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use netchan::endpoint::{ClientConfig, ClientEvent, ServerConfig, ServerEvent};
use netchan::{Client, Endpoint};
use netio::ws::{Acceptor, Limits};
use netio::{Request, Response, ServerConfig as HttpConfig};
use plat::net::{IpAddr, SocketAddr};

use crate::{median_p99, out, PROTOCOL};

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}

/// a file of `dir` for a request path ("/" -> index.html); None for anything outside it
fn file(dir: &str, path: &str) -> Option<(Vec<u8>, &'static str)> {
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.split('/').any(|s| s.is_empty() || s == ".." || s == "." || s.contains('\\') || s.contains(':')) {
        return None;
    }
    let full = format!("{}/{}", dir.trim_end_matches(['/', '\\']), rel);
    plat::read_file(&full).map(|b| (b, content_type(rel)))
}

fn start_http(addr: &SocketAddr, dir: &str, ep: netchan::WsEndpoint, quit: Arc<AtomicBool>) -> plat::net::NetResult<netio::Server> {
    let echo = Arc::new(Acceptor::new(Limits { messages_per_s: 100_000, bytes_per_s: 1 << 30, max_message: 4 << 20, max_frame: 4 << 20, ..Limits::default() }));
    let dir = String::from(dir);
    let handler: netio::server::Handler = Arc::new(move |req: &Request| match req.path() {
        "/ws" => echo.accept(req, |ws| crate::wstest::echo(ws, None)),
        "/chan" => ep.accept(req),
        "/quit" => {
            quit.store(true, Ordering::Release);
            Response::text(200, "bye")
        }
        p => match file(&dir, p) {
            Some((b, t)) => Response::bytes(200, t, b).header("Cache-Control", "no-store"),
            None => Response::new(404),
        },
    });
    netio::Server::start(addr, HttpConfig { threads: 8, read_ms: 10_000, ..HttpConfig::default() }, handler)
}

/// the server of the browser test (see the top of this file) -> exit code
pub fn serve(dir: &str, port: u16) -> i32 {
    let mut chan = netchan::Server::new(ServerConfig { protocol: PROTOCOL, packets_per_s: 100_000, bytes_per_s: 1 << 28, ..ServerConfig::default() });
    let quit = Arc::new(AtomicBool::new(false));
    let lo = |p: u16| SocketAddr::new(IpAddr::LOCALHOST_V4, p);
    let mut http = match start_http(&lo(port), dir, chan.ws_endpoint(), quit.clone()) {
        Ok(h) => h,
        Err(e) => {
            out(&format!("nettest serve: no HTTP server on port {}: {}\n", port, e));
            return 1;
        }
    };
    // UDP on the same port number; Windows hands out ports in order and a parallel test may hold
    // that one for UDP: then a random pair, then any UDP port
    let mut udp = chan.bind_udp(&lo(http.local_addr().port));
    let mut seed = plat::random_u64();
    let mut tries = 0;
    while udp.is_err() && port == 0 && tries < 20 {
        tries += 1;
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let p = 20000 + (seed >> 33) as u16 % 40000;
        if let Ok(h) = start_http(&lo(p), dir, chan.ws_endpoint(), quit.clone()) {
            match chan.bind_udp(&lo(p)) {
                Ok(a) => {
                    core::mem::replace(&mut http, h).stop();
                    udp = Ok(a);
                }
                Err(_) => h.stop(),
            }
        }
    }
    let udp = match udp.or_else(|_| chan.bind_udp(&lo(0))) {
        Ok(a) => a,
        Err(e) => {
            out(&format!("nettest serve: no UDP socket: {}\n", e));
            return 1;
        }
    };
    out(&format!("nettest serve {} {}\n", http.local_addr().port, udp.port));
    while !quit.load(Ordering::Acquire) {
        // (a sleep would cost a timer tick: 15.6 ms on Windows)
        chan.wait(20);
        while let Some(e) = chan.poll() {
            match e {
                ServerEvent::Message { id, data, .. } => {
                    let _ = chan.send(id, &data);
                }
                ServerEvent::Connected { id, transport, addr } => out(&format!("nettest serve: channel {} connected over {:?} from {:?}\n", id, transport, addr)),
                ServerEvent::Disconnected { id, reason } => out(&format!("nettest serve: channel {} gone: {:?}\n", id, reason)),
                _ => {}
            }
        }
    }
    out("nettest serve: quit\n");
    http.stop();
    0
}

/// round trips of the channel against a --serve server -> (median ms, p99 ms), or the error
fn channel_rtts(ep: Endpoint, n: usize) -> Result<(f64, f64, bool), String> {
    let mut cl = Client::connect(ep, ClientConfig { protocol: PROTOCOL, ..ClientConfig::default() }).map_err(|e| format!("{}", e))?;
    let end = plat::time_us() + 5_000_000;
    let mut connected = false;
    while !connected && plat::time_us() < end {
        while let Some(e) = cl.poll() {
            match e {
                ClientEvent::Connected => connected = true,
                ClientEvent::Disconnected(r) => return Err(format!("{:?}", r)),
                _ => {}
            }
        }
    }
    if !connected {
        return Err(String::from("not connected"));
    }
    let mut rtts = Vec::with_capacity(n);
    for i in 0..n {
        let msg = format!("rtt {}", i);
        let t0 = plat::time_us();
        cl.send(msg.as_bytes()).map_err(|e| format!("{}", e))?;
        let deadline = t0 + 2_000_000;
        'wait: loop {
            while let Some(e) = cl.poll() {
                if let ClientEvent::Message { data, .. } = e {
                    if data == msg.as_bytes() {
                        break 'wait;
                    }
                }
            }
            if plat::time_us() > deadline {
                return Err(format!("echo {} did not come", i));
            }
        }
        rtts.push((plat::time_us() - t0) as f64 / 1000.0);
    }
    let reliable = cl.reliable();
    cl.disconnect();
    let (m, p) = median_p99(&mut rtts);
    Ok((m, p, reliable))
}

/// a desktop client against --serve: the channel over UDP and over the WebSocket -> exit code
pub fn client(port: u16, udp: u16) -> i32 {
    let mut failed = 0;
    let lo = |p: u16| SocketAddr::new(IpAddr::LOCALHOST_V4, p);
    for (name, ep) in [("udp", Endpoint::Udp(lo(udp))), ("ws", Endpoint::WebSocket(format!("ws://127.0.0.1:{}/chan", port)))] {
        match channel_rtts(ep, 500) {
            Ok((m, p, reliable)) => out(&format!("nettest client {}: 500 round trips, median {:.3} ms, p99 {:.3} ms, reliable {}\n", name, m, p, reliable)),
            Err(e) => {
                failed += 1;
                out(&format!("FAIL nettest client {}: {}\n", name, e));
            }
        }
    }
    if failed > 0 { 1 } else { 0 }
}
