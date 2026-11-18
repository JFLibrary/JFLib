// chantest -- lib/netchan on real loopback sockets: the UDP channel and the channel over a
// WebSocket, one server for both, echoes, acks, disconnects, timeouts, caps, the protocol id, the
// app's permissions; netchan's own WebSocket client type against netio's echo server

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use netchan::endpoint::{ClientConfig, ClientEvent, ServerConfig, ServerEvent};
use netchan::{packet, Client, Endpoint, Error, Reason, Server, Transport, WebSocket, WsMessage, WsState};
use plat::net::{IpAddr, NetErrorKind, SocketAddr};
use plat::sync::Mutex;

use crate::{Ctx, Rng, PROTOCOL};

fn lo4() -> SocketAddr {
    SocketAddr::new(IpAddr::LOCALHOST_V4, 0)
}

pub fn scfg() -> ServerConfig {
    ServerConfig { protocol: PROTOCOL, packets_per_s: 100_000, bytes_per_s: 1 << 28, max_per_ip: 256, ..ServerConfig::default() }
}

pub fn ccfg() -> ClientConfig {
    ClientConfig { protocol: PROTOCOL, ..ClientConfig::default() }
}

/// a client's events so far
#[derive(Default)]
pub struct Seen {
    pub connected: bool,
    pub msgs: Vec<(u16, Vec<u8>)>,
    pub delivered: usize,
    pub lost: usize,
    pub end: Option<Reason>,
}

/// poll the server (echoing every message back to its sender) and the clients for `ms`, or until
/// `done` holds
pub fn pump(server: &mut Server, clients: &mut [Client], seen: &mut [Seen], ms: u64, done: impl Fn(&[Seen]) -> bool) -> Vec<ServerEvent> {
    let end = plat::time_us() + ms * 1000;
    let mut events = Vec::new();
    loop {
        while let Some(e) = server.poll() {
            if let ServerEvent::Message { id, data, .. } = &e {
                let _ = server.send(*id, data);
            }
            events.push(e);
        }
        for (c, s) in clients.iter_mut().zip(seen.iter_mut()) {
            while let Some(e) = c.poll() {
                match e {
                    ClientEvent::Connected => s.connected = true,
                    ClientEvent::Message { seq, data } => s.msgs.push((seq, data)),
                    ClientEvent::Delivered(_) => s.delivered += 1,
                    ClientEvent::Lost(_) => s.lost += 1,
                    ClientEvent::Disconnected(r) => s.end = Some(r),
                }
            }
        }
        if done(seen) || plat::time_us() >= end {
            return events;
        }
        plat::sleep_ms(1);
    }
}

/// connect, echo `n` messages of random sizes up to `max`, check each came back unchanged
fn echo_run(c: &mut Ctx, name: &str, server: &mut Server, ep: Endpoint, n: usize, max: usize) -> Option<Client> {
    let cl = match Client::connect(ep, ccfg()) {
        Ok(cl) => cl,
        Err(e) => {
            c.check(false, || format!("{}: connect: {}", name, e));
            return None;
        }
    };
    let mut cls = [cl];
    let mut seen = [Seen::default()];
    pump(server, &mut cls, &mut seen, 3000, |s| s[0].connected || s[0].end.is_some());
    if !c.check(seen[0].connected, || format!("{}: not connected ({:?})", name, seen[0].end)) {
        return None;
    }
    let mut rng = Rng::new(n as u64 ^ max as u64);
    let mut sent = Vec::new();
    for _ in 0..n {
        let b = rng.some_bytes(1, max);
        match cls[0].send(&b) {
            Ok(_) => sent.push(b),
            Err(e) => {
                c.check(false, || format!("{}: send: {}", name, e));
                break;
            }
        }
        if sent.len() % 32 == 0 {
            pump(server, &mut cls, &mut seen, 1, |_| false);
        }
    }
    let want = sent.len();
    pump(server, &mut cls, &mut seen, 5000, |s| s[0].msgs.len() >= want && s[0].delivered >= want);
    let back: Vec<Vec<u8>> = seen[0].msgs.iter().map(|m| m.1.clone()).collect();
    let all = back.len() == sent.len() && sent.iter().all(|s| back.contains(s));
    c.check(all, || format!("{}: {} of {} came back unchanged", name, back.len(), sent.len()));
    c.check(seen[0].delivered >= want && seen[0].lost == 0, || format!("{}: delivered {}, lost {} of {}", name, seen[0].delivered, seen[0].lost, want));
    if cls[0].reliable() {
        c.check(back == sent, || format!("{}: the WebSocket keeps the order", name));
    }
    let [cl] = cls;
    Some(cl)
}

pub fn run_udp(quick: bool) -> Ctx {
    let mut c = Ctx::new("udp", quick);
    let n = c.n(2000).max(100);
    let mut server = Server::new(scfg());
    let addr = match server.bind_udp(&lo4()) {
        Ok(a) => a,
        Err(e) => {
            c.check(false, || format!("bind_udp: {}", e));
            return c;
        }
    };
    if let Some(mut cl) = echo_run(&mut c, "udp", &mut server, Endpoint::Udp(addr), n, 1200) {
        c.check(!cl.reliable() && cl.max_payload() == 1200, || format!("udp: reliable {}, max_payload {}", cl.reliable(), cl.max_payload()));
        c.check(cl.send(&[0u8; 1201]) == Err(Error::Size) && cl.send(&[]) == Err(Error::Size), || String::from("udp: payloads over 1200 bytes and empty ones are refused"));
        // (measured, not timed: pump sleeps a timer tick between rounds, the tail has keep-alives only)
        c.check(cl.rtt_ms() > 0.0 && cl.rtt_ms() < 250.0, || format!("udp: rtt {} ms on loopback", cl.rtt_ms()));
        let id = server.ids().first().copied().unwrap_or(0);
        c.check(server.peer(id).is_some_and(|p| p.0 == Transport::Udp), || String::from("udp: the server's peer is UDP"));
        cl.disconnect();
        let mut none: [Client; 0] = [];
        let ev = pump(&mut server, &mut none, &mut [], 1000, |_| false);
        c.check(ev.iter().any(|e| matches!(e, ServerEvent::Disconnected { reason: Reason::Peer(packet::BYE_NORMAL), .. })), || format!("udp: the server saw the disconnect: {:?}", ev));
    }
    // IPv6 loopback, where there is one
    let mut s6 = Server::new(scfg());
    match s6.bind_udp(&SocketAddr::new(IpAddr::LOCALHOST_V6, 0)) {
        Ok(a6) => {
            echo_run(&mut c, "udp ipv6", &mut s6, Endpoint::Udp(a6), n / 4, 600);
        }
        Err(e) => crate::out(&format!("udp: no IPv6 loopback ({}), skipped\n", e)),
    }
    drop(s6);
    // the server stops: its clients hear it
    {
        let cl = Client::connect(Endpoint::Udp(addr), ccfg()).ok();
        if let Some(cl) = cl {
            let mut cls = [cl];
            let mut seen = [Seen::default()];
            pump(&mut server, &mut cls, &mut seen, 3000, |s| s[0].connected);
            drop(server);
            let mut dummy = Server::new(scfg());
            pump(&mut dummy, &mut cls, &mut seen, 2000, |s| s[0].end.is_some());
            c.check(seen[0].end == Some(Reason::Peer(packet::BYE_SHUTDOWN)), || format!("udp: server stop: {:?}", seen[0].end));
        }
    }
    // a client that stops polling times out on the server; the caps; another protocol id
    let mut server = Server::new(ServerConfig { timeout_ms: 500, max_connections: 2, ..scfg() });
    let addr = server.bind_udp(&lo4()).unwrap_or(lo4());
    let mut cls: Vec<Client> = (0..3).filter_map(|_| Client::connect(Endpoint::Udp(addr), ccfg()).ok()).collect();
    let mut seen: Vec<Seen> = (0..cls.len()).map(|_| Seen::default()).collect();
    pump(&mut server, &mut cls, &mut seen, 3000, |s| s.iter().all(|x| x.connected || x.end.is_some()));
    let full = seen.iter().filter(|s| s.end == Some(Reason::Full)).count();
    c.check(server.connections() == 2 && full == 1, || format!("udp: max_connections 2 of 3: {} in, {} full", server.connections(), full));
    let t0 = plat::time_us();
    let mut none: [Client; 0] = [];
    let ev = pump(&mut server, &mut none, &mut [], 2000, |_| false);
    let gone = ev.iter().filter(|e| matches!(e, ServerEvent::Disconnected { reason: Reason::Timeout, .. })).count();
    c.check(gone == 2, || format!("udp: clients that stop polling time out: {} after {} ms", gone, (plat::time_us() - t0) / 1000));
    drop(cls);
    let other = Client::connect(Endpoint::Udp(addr), ClientConfig { protocol: PROTOCOL + 1, ..ccfg() });
    if let Ok(cl) = other {
        let mut cls = [cl];
        let mut seen = [Seen::default()];
        pump(&mut server, &mut cls, &mut seen, 2000, |s| s[0].end.is_some());
        c.check(seen[0].end == Some(Reason::Protocol), || format!("udp: another protocol id: {:?}", seen[0].end));
    }
    // nobody listens: ConnectTimeout
    let dead = SocketAddr::new(IpAddr::LOCALHOST_V4, addr.port.wrapping_add(7).max(1024));
    if let Ok(cl) = Client::connect(Endpoint::Udp(dead), ClientConfig { connect_ms: 400, ..ccfg() }) {
        let mut cls = [cl];
        let mut seen = [Seen::default()];
        pump(&mut server, &mut cls, &mut seen, 2000, |s| s[0].end.is_some());
        c.check(seen[0].end == Some(Reason::ConnectTimeout), || format!("udp: nobody there: {:?}", seen[0].end));
    }
    // the app's permissions: net.connect and net.listen name localhost only
    let far = SocketAddr::new(IpAddr::V4([192, 0, 2, 1]), 9);
    let r = Client::connect(Endpoint::Udp(far), ccfg());
    c.check(matches!(r, Err(Error::Net(ref e)) if e.kind == NetErrorKind::Denied), || format!("udp: a client to 192.0.2.1 is refused: {:?}", r.err()));
    let mut s2 = Server::new(scfg());
    let r = s2.bind_udp(&SocketAddr::new(IpAddr::UNSPECIFIED_V4, 45678));
    c.check(r.as_ref().err().is_some_and(|e| e.kind == NetErrorKind::Denied), || format!("udp: a server on 0.0.0.0:45678 is refused: {:?}", r));
    c
}

pub fn run_ws(quick: bool) -> Ctx {
    let mut c = Ctx::new("wschan", quick);
    let n = c.n(2000).max(100);
    let mut server = Server::new(scfg());
    let wa = match server.listen_ws(&lo4(), "/chan") {
        Ok(a) => a,
        Err(e) => {
            c.check(false, || format!("listen_ws: {}", e));
            return c;
        }
    };
    let ua = server.bind_udp(&lo4()).unwrap_or(lo4());
    let url = format!("ws://{}/chan", wa);
    if let Some(mut cl) = echo_run(&mut c, "ws", &mut server, Endpoint::WebSocket(url.clone()), n, 8000) {
        c.check(cl.reliable(), || String::from("ws: reliable"));
        let big = alloc::vec![7u8; 60_000];
        c.check(cl.send(&big).is_ok(), || String::from("ws: 60 KB in one message"));
        cl.disconnect();
        let mut none: [Client; 0] = [];
        let ev = pump(&mut server, &mut none, &mut [], 1000, |_| false);
        c.check(ev.iter().any(|e| matches!(e, ServerEvent::Disconnected { reason: Reason::Peer(packet::BYE_NORMAL), .. })), || format!("ws: the server saw the disconnect: {:?}", ev));
    }
    // UDP and WebSocket clients together on one server
    let k = if quick { 4 } else { 12 };
    let mut cls: Vec<Client> = Vec::new();
    for i in 0..2 * k {
        let ep = if i % 2 == 0 { Endpoint::Udp(ua) } else { Endpoint::WebSocket(url.clone()) };
        if let Ok(cl) = Client::connect(ep, ccfg()) {
            cls.push(cl);
        }
    }
    let mut seen: Vec<Seen> = (0..cls.len()).map(|_| Seen::default()).collect();
    pump(&mut server, &mut cls, &mut seen, 5000, |s| s.iter().all(|x| x.connected || x.end.is_some()));
    let ids = server.ids();
    let (udp, ws) = ids.iter().fold((0, 0), |(u, w), id| match server.peer(*id).map(|p| p.0) {
        Some(Transport::Udp) => (u + 1, w),
        Some(Transport::WebSocket) => (u, w + 1),
        None => (u, w),
    });
    c.check(udp == k && ws == k, || format!("mixed: {} UDP and {} WebSocket clients (want {} each)", udp, ws, k));
    let rounds = c.n(200).max(20);
    for r in 0..rounds {
        for (i, cl) in cls.iter_mut().enumerate() {
            let _ = cl.send(format!("{}:{}", i, r).as_bytes());
        }
        pump(&mut server, &mut cls, &mut seen, 1, |_| false);
    }
    pump(&mut server, &mut cls, &mut seen, 5000, |s| s.iter().all(|x| x.msgs.len() >= rounds));
    let ok = seen.iter().enumerate().filter(|(i, s)| s.msgs.len() == rounds && s.msgs.iter().all(|m| m.1.starts_with(format!("{}:", i).as_bytes()))).count();
    c.check(ok == cls.len(), || format!("mixed: {} of {} clients got their {} echoes", ok, cls.len(), rounds));
    drop(cls);
    // a path without the channel: the handshake fails, the client ends
    if let Ok(cl) = Client::connect(Endpoint::WebSocket(format!("ws://{}/nothing", wa)), ccfg()) {
        let mut cls = [cl];
        let mut seen = [Seen::default()];
        pump(&mut server, &mut cls, &mut seen, 3000, |s| s[0].end.is_some());
        c.check(matches!(seen[0].end, Some(Reason::Closed(_))), || format!("ws: a wrong path: {:?}", seen[0].end));
    }
    c.check(matches!(Client::connect(Endpoint::WebSocket(String::from("ws://example.org/chan")), ccfg()), Err(Error::Net(ref e)) if e.kind == NetErrorKind::Denied), || String::from("ws: example.org is not in the permissions"));
    c.check(matches!(Client::connect(Endpoint::WebSocket(String::from("http://x/")), ccfg()), Err(Error::Url(_))), || String::from("ws: http:// is refused"));
    drop(server);

    // netchan's WebSocket against netio's echo server: connect, messages, close
    let log = Arc::new(Mutex::new(Vec::new()));
    let (es, _, ea) = crate::wstest::echo_server(netio::ws::Limits::default(), |a| a, log);
    match WebSocket::connect(&format!("ws://{}/echo", ea)) {
        Ok(w) => {
            let end = plat::time_us() + 3_000_000;
            while w.state() == WsState::Connecting && plat::time_us() < end {
                plat::sleep_ms(1);
            }
            c.check(w.state() == WsState::Open, || format!("netchan WebSocket: {:?}", w.state()));
            let _ = w.send_text("hallo");
            let _ = w.send_binary(&[1, 2, 3]);
            let mut got = Vec::new();
            let end = plat::time_us() + 3_000_000;
            while got.len() < 2 && plat::time_us() < end {
                if let Some(m) = w.recv() {
                    got.push(m);
                } else {
                    plat::sleep_ms(1);
                }
            }
            c.check(got == [WsMessage::Text(String::from("hallo")), WsMessage::Binary(alloc::vec![1, 2, 3])], || format!("netchan WebSocket echo: {:?}", got));
            w.close(4000, "fertig");
            let end = plat::time_us() + 3_000_000;
            while w.state() != WsState::Closed && plat::time_us() < end {
                plat::sleep_ms(1);
            }
            c.check(w.close_info().is_some_and(|i| i.0 == 4000), || format!("netchan WebSocket close: {:?} {:?}", w.state(), w.close_info()));
            c.check(w.send_text("late") == Err(Error::NotConnected), || String::from("netchan WebSocket: no send after the close"));
        }
        Err(e) => {
            c.check(false, || format!("netchan WebSocket connect: {}", e));
        }
    }
    let r = WebSocket::connect("ws://127.0.0.1:1/");
    if let Ok(w) = r {
        let end = plat::time_us() + 3_000_000;
        while w.state() != WsState::Closed && plat::time_us() < end {
            plat::sleep_ms(1);
        }
        c.check(w.close_info().is_some_and(|i| i.0 == 1006), || format!("netchan WebSocket to a closed port: {:?}", w.close_info()));
    }
    drop(es);
    c
}
