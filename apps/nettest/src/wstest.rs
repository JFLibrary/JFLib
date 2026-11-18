// wstest -- netio's WebSocket against itself and against a client by hand, on loopback: handshake,
// echo, fragments both ways, ping / pong, close, every malformed frame with its close code, the
// limits, handshake errors on both sides, server stop, many clients at once

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use netio::ws::{self, Acceptor, ClientConfig, Limits, Message, WsError};
use netio::{Request, Response, Server, ServerConfig};
use plat::net::{IpAddr, SocketAddr, TcpListener};
use plat::sync::Mutex;

use crate::raw;
use crate::{pattern, Ctx, Rng};

/// what the echo server noticed: the ends of its connections
pub type Log = Arc<Mutex<Vec<String>>>;

fn lo() -> SocketAddr {
    SocketAddr::new(IpAddr::LOCALHOST_V4, 0)
}

/// the echo loop: text and binary back as they came; "frag:N" -> N bytes of pattern in frames of
/// 1000; "blast:N:SIZE" -> N binary messages of SIZE; "close4001" -> the server closes
pub fn echo(ws: ws::WebSocket, log: Option<Log>) {
    loop {
        match ws.recv() {
            Ok(Message::Text(t)) => {
                if let Some(n) = t.strip_prefix("frag:").and_then(|n| n.parse::<usize>().ok()) {
                    let _ = ws.send_fragmented(ws::OP_BINARY, &pattern(n), 1000);
                } else if let Some(rest) = t.strip_prefix("blast:") {
                    let mut it = rest.split(':').filter_map(|v| v.parse::<usize>().ok());
                    let (n, size) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
                    let b = pattern(size);
                    for _ in 0..n {
                        if ws.send_binary(&b).is_err() {
                            break;
                        }
                    }
                } else if t == "close4001" {
                    ws.close(4001, "server says bye");
                    if let Some(l) = &log {
                        l.lock().push(String::from("closed 4001"));
                    }
                    return;
                } else if ws.send_text(&t).is_err() {
                    return;
                }
            }
            Ok(Message::Binary(b)) => {
                if ws.send_binary(&b).is_err() {
                    return;
                }
            }
            Err(e) => {
                if let Some(l) = &log {
                    l.lock().push(format!("{:?}", e));
                }
                return;
            }
        }
    }
}

/// an HTTP server whose /echo takes WebSockets with these limits (the Acceptor as `make` builds it)
pub fn echo_server(lim: Limits, make: impl FnOnce(Acceptor) -> Acceptor, log: Log) -> (Server, Arc<Acceptor>, SocketAddr) {
    let acc = Arc::new(make(Acceptor::new(lim)));
    let a = acc.clone();
    let handler: netio::server::Handler = Arc::new(move |req: &Request| {
        if req.path() == "/echo" {
            let l = log.clone();
            a.accept(req, move |ws| echo(ws, Some(l)))
        } else if req.path() == "/plain" {
            Response::text(200, "no websocket here")
        } else {
            Response::new(404)
        }
    });
    let cfg = ServerConfig { threads: 4, read_ms: 3000, idle_ms: 3000, ..ServerConfig::default() };
    let s = Server::start(&lo(), cfg, handler).expect("echo server");
    let addr = s.local_addr();
    (s, acc, addr)
}

fn url(addr: &SocketAddr, path: &str) -> String {
    format!("ws://{}{}", addr, path)
}

fn client(addr: &SocketAddr) -> Result<ws::WebSocket, WsError> {
    ws::connect(&url(addr, "/echo"), &ClientConfig::default())
}

fn wait_log(log: &Log, pred: impl Fn(&str) -> bool, ms: u64) -> bool {
    let end = plat::time_us() + ms * 1000;
    while plat::time_us() < end {
        if log.lock().iter().any(|l| pred(l)) {
            return true;
        }
        plat::sleep_ms(10);
    }
    false
}

pub fn run(quick: bool) -> Ctx {
    let mut c = Ctx::new("ws", quick);
    // RFC 6455 1.3's example
    c.check(ws::accept_key("dGhlIHNhbXBsZSBub25jZQ==") == "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=", || String::from("accept key of RFC 6455"));
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let (server, acc, addr) = echo_server(Limits::default(), |a| a.protocols(&["game.v2", "game.v1"]), log.clone());

    // echo: text, binary, empty, a megabyte, fragments both ways
    match client(&addr) {
        Ok(w) => {
            let mut rng = Rng::new(7);
            let bin = rng.bytes(1000);
            let big = rng.bytes(1 << 20);
            let ok_t = w.send_text("hallo Würfel ✓").is_ok() && w.recv() == Ok(Message::Text(String::from("hallo Würfel ✓")));
            c.check(ok_t, || String::from("text echo"));
            c.check(w.send_binary(&bin).is_ok() && w.recv() == Ok(Message::Binary(bin.clone())), || String::from("binary echo"));
            c.check(w.send_binary(&[]).is_ok() && w.recv() == Ok(Message::Binary(Vec::new())), || String::from("empty binary"));
            c.check(w.send_text("").is_ok() && w.recv() == Ok(Message::Text(String::new())), || String::from("empty text"));
            c.check(w.send_binary(&big).is_ok() && w.recv().map(|m| m == Message::Binary(big.clone())).unwrap_or(false), || String::from("1 MiB echo (sent as 16 frames)"));
            let txt = "Grüße aus dem Würfelspiel – ✓✓✓ ".repeat(40);
            let ok = w.send_fragmented(ws::OP_TEXT, txt.as_bytes(), 3).is_ok() && w.recv() == Ok(Message::Text(txt.clone()));
            c.check(ok, || String::from("text in frames of 3 bytes (UTF-8 split across frames)"));
            let ok = w.send_text("frag:100000").is_ok() && w.recv() == Ok(Message::Binary(pattern(100000)));
            c.check(ok, || String::from("server -> client in frames of 1000"));
            c.check(w.ping(b"p").is_ok() && w.send_text("after ping").is_ok() && w.recv() == Ok(Message::Text(String::from("after ping"))), || String::from("ping, the pong taken by recv"));
            c.check(w.ping(&[0u8; 126]) == Err(WsError::TooLarge), || String::from("a ping over 125 bytes is refused"));
            c.check(w.send_binary(&alloc::vec![0u8; (1 << 20) + 1]) == Err(WsError::TooLarge), || String::from("a message over max_message is refused"));
            // another thread sends while this one reads
            let w = Arc::new(w);
            let w2 = w.clone();
            let t = plat::thread_spawn(256 * 1024, Box::new(move || {
                for i in 0..100u32 {
                    let _ = w2.send_binary(&i.to_le_bytes());
                }
            }));
            let mut got = 0u32;
            for i in 0..100u32 {
                if w.recv() == Ok(Message::Binary(Vec::from(i.to_le_bytes()))) {
                    got += 1;
                }
            }
            if let Some(t) = t {
                t.join();
            }
            c.check(got == 100, || format!("send on another thread: {} of 100 in order", got));
            w.close(1000, "bye");
            c.check(wait_log(&log, |l| l.contains("Closed(1000, \"bye\")"), 2000), || format!("the server saw the client's close: {:?}", log.lock().clone()));
        }
        Err(e) => {
            c.check(false, || format!("connect: {}", e));
        }
    }
    // the server closes
    if let Ok(w) = client(&addr) {
        let _ = w.send_text("close4001");
        c.check(w.recv() == Err(WsError::Closed(4001, String::from("server says bye"))), || String::from("the server's close (4001) reaches the client"));
    }
    // subprotocols
    let cfg = ClientConfig { protocols: alloc::vec![String::from("game.v1")], ..ClientConfig::default() };
    match ws::connect(&url(&addr, "/echo"), &cfg) {
        Ok(w) => {
            c.check(w.protocol() == Some("game.v1"), || format!("subprotocol: {:?}", w.protocol()));
        }
        Err(e) => {
            c.check(false, || format!("subprotocol connect: {}", e));
        }
    }

    // a client by hand: ping / pong, a ping between fragments
    if let Some(mut r) = raw::upgrade(&addr, "/echo") {
        r.frame(0x89, b"abc", true);
        c.check(r.read_frame(2000) == Some((0x8A, Vec::from(&b"abc"[..]))), || String::from("ping -> pong with the same bytes"));
        r.frame(0x01, b"Hel", true);
        r.frame(0x89, b"x", true);
        r.frame(0x80, b"lo", true);
        let pong = r.read_frame(2000);
        let text = r.read_frame(2000);
        c.check(pong == Some((0x8A, Vec::from(&b"x"[..]))) && text == Some((0x81, Vec::from(&b"Hello"[..]))), || format!("ping between fragments: {:?} {:?}", pong, text));
    } else {
        c.check(false, || String::from("raw upgrade"));
    }

    // malformed frames: the close code each gets
    let cases: &[(&str, &[(u8, &[u8], bool)], u16)] = &[
        ("an unmasked frame", &[(0x81, b"hi", false)], ws::CLOSE_PROTOCOL),
        ("RSV1 without an extension", &[(0xC1, b"hi", true)], ws::CLOSE_PROTOCOL),
        ("opcode 3", &[(0x83, b"", true)], ws::CLOSE_PROTOCOL),
        ("opcode 11", &[(0x8B, b"", true)], ws::CLOSE_PROTOCOL),
        ("a fragmented ping", &[(0x09, b"x", true)], ws::CLOSE_PROTOCOL),
        ("a continuation of nothing", &[(0x80, b"x", true)], ws::CLOSE_PROTOCOL),
        ("a new message inside a fragmented one", &[(0x01, b"a", true), (0x81, b"b", true)], ws::CLOSE_PROTOCOL),
        ("text that is not UTF-8", &[(0x81, &[0xff, 0xfe], true)], ws::CLOSE_BAD_DATA),
        ("UTF-8 broken across fragments", &[(0x01, &[0xe2, 0x82], true), (0x80, &[0x41], true)], ws::CLOSE_BAD_DATA),
        ("close code 1005", &[(0x88, &[0x03, 0xed], true)], ws::CLOSE_PROTOCOL),
        ("close code 999", &[(0x88, &[0x03, 0xe7], true)], ws::CLOSE_PROTOCOL),
        ("close code 5000", &[(0x88, &[0x13, 0x88], true)], ws::CLOSE_PROTOCOL),
        ("a close of one byte", &[(0x88, &[0x03], true)], ws::CLOSE_PROTOCOL),
        ("a close reason that is not UTF-8", &[(0x88, &[0x03, 0xe8, 0xff], true)], ws::CLOSE_BAD_DATA),
    ];
    for (name, frames, code) in cases {
        let Some(mut r) = raw::upgrade(&addr, "/echo") else {
            c.check(false, || format!("{}: no upgrade", name));
            continue;
        };
        for (b0, p, m) in frames.iter() {
            r.frame(*b0, p, *m);
        }
        let got = r.close_code(3000);
        c.check(got == Some(*code), || format!("{}: close {:?}, want {}", name, got, code));
    }
    // headers built by hand: a long ping, lengths not in their shortest form, the top bit
    let heads: &[(&str, &[u8], u16)] = &[
        ("a ping of 126 bytes", &[0x89, 0xFE, 0x00, 0x7E, 1, 2, 3, 4], ws::CLOSE_PROTOCOL),
        ("length 10 in 16 bits", &[0x82, 0xFE, 0x00, 0x0A, 1, 2, 3, 4], ws::CLOSE_PROTOCOL),
        ("length 200 in 64 bits", &[0x82, 0xFF, 0, 0, 0, 0, 0, 0, 0, 200, 1, 2, 3, 4], ws::CLOSE_PROTOCOL),
        ("a 64-bit length with the top bit", &[0x82, 0xFF, 0x80, 0, 0, 0, 0, 1, 0, 0, 1, 2, 3, 4], ws::CLOSE_PROTOCOL),
        ("a frame of 4 GiB announced", &[0x82, 0xFF, 0, 0, 0, 1, 0, 0, 0, 0, 1, 2, 3, 4], ws::CLOSE_TOO_BIG),
    ];
    for (name, bytes, code) in heads {
        let Some(mut r) = raw::upgrade(&addr, "/echo") else { continue };
        r.send(bytes);
        let got = r.close_code(3000);
        c.check(got == Some(*code), || format!("{}: close {:?}, want {}", name, got, code));
    }
    drop(server);

    // the limits: frame, message, fragments, rates, idle, ping
    let lim = Limits { max_frame: 1024, max_message: 4096, max_fragments: 8, ..Limits::default() };
    let (server, _, addr) = echo_server(lim, |a| a, log.clone());
    if let Some(mut r) = raw::upgrade(&addr, "/echo") {
        r.send(&[0x82, 0xFE, 0x07, 0xD0, 1, 2, 3, 4]); // 2000 bytes announced
        c.check(r.close_code(3000) == Some(ws::CLOSE_TOO_BIG), || String::from("a frame over max_frame: 1009 before its payload"));
    }
    if let Some(mut r) = raw::upgrade(&addr, "/echo") {
        for b0 in [0x02, 0x00, 0x00, 0x00, 0x00] {
            r.frame(b0, &[7u8; 1000], true);
        }
        c.check(r.close_code(3000) == Some(ws::CLOSE_TOO_BIG), || String::from("fragments over max_message: 1009"));
    }
    if let Some(mut r) = raw::upgrade(&addr, "/echo") {
        r.frame(0x02, b"a", true);
        for _ in 0..9 {
            r.frame(0x00, b"a", true);
        }
        c.check(r.close_code(3000) == Some(ws::CLOSE_POLICY), || String::from("more than max_fragments: 1008"));
    }
    drop(server);
    let lim = Limits { messages_per_s: 50, ..Limits::default() };
    let (server, _, addr) = echo_server(lim, |a| a, log.clone());
    if let Some(mut r) = raw::upgrade(&addr, "/echo") {
        let mut burst = Vec::new();
        for _ in 0..300 {
            ws::encode_frame(&mut burst, ws::OP_BINARY, true, b"x", Some([1, 2, 3, 4]));
        }
        r.send(&burst);
        c.check(r.close_code(3000) == Some(ws::CLOSE_POLICY), || String::from("300 messages against 50 a second: 1008"));
    }
    drop(server);
    let lim = Limits { bytes_per_s: 10_000, ..Limits::default() };
    let (server, _, addr) = echo_server(lim, |a| a, log.clone());
    if let Some(mut r) = raw::upgrade(&addr, "/echo") {
        r.frame(0x82, &[0u8; 60_000], true);
        c.check(r.close_code(3000) == Some(ws::CLOSE_POLICY), || String::from("60 KB against 10 KB a second: 1008"));
    }
    drop(server);
    let lim = Limits { idle_ms: 300, ping_ms: 0, ..Limits::default() };
    let (server, _, addr) = echo_server(lim, |a| a, log.clone());
    if let Some(mut r) = raw::upgrade(&addr, "/echo") {
        let t0 = plat::time_us();
        let code = r.close_code(3000);
        let ms = (plat::time_us() - t0) / 1000;
        c.check(code == Some(ws::CLOSE_GOING_AWAY) && (250..2000).contains(&ms), || format!("idle 300 ms: close {:?} after {} ms", code, ms));
    }
    drop(server);
    let lim = Limits { idle_ms: 3000, ping_ms: 200, ..Limits::default() };
    let (server, _, addr) = echo_server(lim, |a| a, log.clone());
    if let Some(mut r) = raw::upgrade(&addr, "/echo") {
        let f = r.read_frame(2000);
        c.check(f.as_ref().is_some_and(|(b0, _)| *b0 == 0x89), || format!("a ping after 200 ms of quiet: {:?}", f));
    }
    drop(server);

    // handshake errors on the server side
    let (server, acc2, addr) = echo_server(Limits { max_connections: 2, ..Limits::default() }, |a| a.origins(&["https://good.example"]), log.clone());
    let status = |req: &str| raw::open(&addr, req).map(|(s, h, _)| (s, h));
    let bad: &[(&str, String, u16)] = &[
        ("no key", String::from("GET /echo HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nOrigin: https://good.example\r\n\r\n"), 400),
        ("a short key", String::from("GET /echo HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: abc\r\nSec-WebSocket-Version: 13\r\nOrigin: https://good.example\r\n\r\n"), 400),
        ("POST", raw::request("/echo", "Origin: https://good.example\r\n").replacen("GET", "POST", 1), 400),
        ("no Upgrade", raw::request("/echo", "Origin: https://good.example\r\n").replace("Upgrade: websocket\r\n", ""), 400),
        ("a foreign origin", raw::request("/echo", "Origin: https://evil.example\r\n"), 403),
        ("no origin", raw::request("/echo", ""), 403),
    ];
    for (name, req, want) in bad {
        let got = status(req).map(|x| x.0);
        c.check(got == Some(*want), || format!("handshake with {}: {:?}, want {}", name, got, want));
    }
    let v8 = raw::request("/echo", "Origin: https://good.example\r\n").replace("Version: 13", "Version: 8");
    let got = status(&v8);
    c.check(got.as_ref().is_some_and(|(s, h)| *s == 426 && h.get("sec-websocket-version") == Some("13")), || format!("version 8: {:?}", got.map(|x| x.0)));
    let good = ClientConfig { origin: Some(String::from("https://good.example")), ..ClientConfig::default() };
    let a1 = ws::connect(&url(&addr, "/echo"), &good);
    let a2 = ws::connect(&url(&addr, "/echo"), &good);
    let a3 = ws::connect(&url(&addr, "/echo"), &good);
    c.check(a1.is_ok() && a2.is_ok() && matches!(a3, Err(WsError::Handshake(ref s)) if s.contains("503")), || format!("max_connections 2: the third gets 503 ({:?})", a3.as_ref().err()));
    c.check(acc2.connections() == 2, || format!("connections {}", acc2.connections()));
    drop(a1);
    drop(a2);
    let end = plat::time_us() + 2_000_000;
    while acc2.connections() > 0 && plat::time_us() < end {
        plat::sleep_ms(10);
    }
    c.check(acc2.connections() == 0, || format!("slots back after the close: {}", acc2.connections()));
    c.check(ws::connect(&url(&addr, "/echo"), &good).is_ok(), || String::from("a slot free again"));
    let plain = ws::connect(&url(&addr, "/plain"), &good);
    c.check(matches!(plain, Err(WsError::Handshake(ref s)) if s.contains("200")), || format!("a path without WebSocket: {:?}", plain.err()));
    drop(server);

    // handshake errors on the client side: a wrong accept, an answer that never comes
    let l = TcpListener::bind(&lo()).expect("listener");
    let la = l.local_addr().expect("addr");
    let t = plat::thread_spawn(64 * 1024, Box::new(move || {
        if let Ok(Some((s, _))) = l.accept_timeout(3000) {
            let mut b = [0u8; 2048];
            let _ = s.read(&mut b);
            let _ = s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: AAAAAAAAAAAAAAAAAAAAAAAAAAA=\r\n\r\n");
            plat::sleep_ms(200);
        }
        if let Ok(Some((_s, _))) = l.accept_timeout(3000) {
            plat::sleep_ms(1500); // never answers
        }
    }));
    let wrong = ws::connect(&url(&la, "/"), &ClientConfig::default());
    c.check(matches!(wrong, Err(WsError::Handshake(ref s)) if s.contains("Accept")), || format!("a wrong Sec-WebSocket-Accept: {:?}", wrong.err()));
    let t0 = plat::time_us();
    let slow = ws::connect(&url(&la, "/"), &ClientConfig { limits: Limits { handshake_ms: 300, ..Limits::default() }, ..ClientConfig::default() });
    let ms = (plat::time_us() - t0) / 1000;
    c.check(slow.as_ref().err() == Some(&WsError::TimedOut) && ms < 1200, || format!("handshake_ms 300: {:?} after {} ms", slow.err(), ms));
    if let Some(t) = t {
        t.join();
    }
    c.check(matches!(ws::connect("http://x/", &ClientConfig::default()), Err(WsError::Url(_))), || String::from("http:// is no WebSocket URL"));
    c.check(matches!(ws::connect("wss://localhost:1/", &ClientConfig::default()), Err(WsError::Url(_) | WsError::Net(_))), || String::from("wss:// without a connector"));

    // the server stops under open connections: 1001
    let (server, _, addr) = echo_server(Limits::default(), |a| a, log.clone());
    if let Ok(w) = client(&addr) {
        let _ = w.send_text("x");
        let _ = w.recv();
        let t0 = plat::time_us();
        server.stop();
        let r = w.recv_timeout(Some(3000));
        c.check(matches!(r, Err(WsError::Closed(1001, _))), || format!("server stop: {:?}", r));
        c.check((plat::time_us() - t0) / 1000 < 3000, || String::from("the stop waits for its WebSockets"));
    }

    // many clients at once, each echoing on a thread of its own
    let (server, acc3, addr) = echo_server(Limits::default(), |a| a, log.clone());
    let n_clients = if quick { 8 } else { 32 };
    let rounds = c.n(2000).max(50) / 10;
    let ok = Arc::new(Mutex::new(0usize));
    let mut threads = Vec::new();
    for k in 0..n_clients {
        let ok = ok.clone();
        if let Some(t) = plat::thread_spawn(256 * 1024, Box::new(move || {
            let Ok(w) = client(&addr) else { return };
            let mut rng = Rng::new(100 + k as u64);
            for _ in 0..rounds {
                let b = rng.some_bytes(1, 3000);
                if w.send_binary(&b).is_err() || w.recv() != Ok(Message::Binary(b)) {
                    return;
                }
            }
            *ok.lock() += 1;
        })) {
            threads.push(t);
        }
    }
    for t in threads {
        t.join();
    }
    let n_ok = *ok.lock();
    c.check(n_ok == n_clients, || format!("{} clients at once: {} finished their {} echoes", n_clients, n_ok, rounds));
    let end = plat::time_us() + 3_000_000;
    while acc3.connections() > 0 && plat::time_us() < end {
        plat::sleep_ms(10);
    }
    c.check(acc3.connections() == 0, || format!("all slots back: {}", acc3.connections()));
    drop(server);
    drop(acc);
    c
}
