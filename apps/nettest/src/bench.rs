// bench -- round-trip time and throughput on loopback (docs/TRANSPORT.md "Messungen"):
//
//   the channel over UDP and over a WebSocket: n round trips of 32 bytes one at a time (median,
//   99th percentile), then a second of messages as fast as they go (1200 bytes over UDP, 16 KiB
//   over the WebSocket): what the server took per second, and what was lost
//   netio's WebSocket alone: n round trips of 32 bytes, 64 MiB from the server, 64 MiB echoed
//   the seal in-process: DATA of 1200 bytes written and opened, sealed and not; the server's time
//   for a RESPONSE (the X25519 of an ACCEPT), with ephemeral keys and with a static key, and the
//   client's for its ACCEPT
//
// Both ends poll without sleeping (the server on a thread of its own), so the numbers are the
// transport's, without a game's tick.  They include the hop from the reading thread to poll().

use alloc::boxed::Box;
use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use netchan::endpoint::{ClientConfig, ClientEvent, ServerConfig, ServerEvent};
use netchan::{Client, Endpoint, Error};
use netio::ws::{self, ClientConfig as WsClientConfig, Limits, Message};
use plat::net::{IpAddr, SocketAddr};
use plat::sync::{channel, Mutex};

use crate::{median_p99, out, PROTOCOL};

struct ChanServer {
    stop: Arc<AtomicBool>,
    bytes: Arc<AtomicU64>,
    t: plat::Thread,
    udp: SocketAddr,
    ws: SocketAddr,
}

// a channel server on a thread: echoes messages that start with b'e', counts the bytes of the rest
fn chan_server() -> Option<ChanServer> {
    let stop = Arc::new(AtomicBool::new(false));
    let bytes = Arc::new(AtomicU64::new(0));
    let (tx, rx) = channel::<(SocketAddr, SocketAddr)>(1);
    let (st, by) = (stop.clone(), bytes.clone());
    let t = plat::thread_spawn(256 * 1024, Box::new(move || {
        let cfg = ServerConfig { protocol: PROTOCOL, packets_per_s: 10_000_000, bytes_per_s: u32::MAX, max_stream_payload: 64 * 1024, timeout_ms: 10_000, ..ServerConfig::default() };
        let mut s = netchan::Server::new(cfg);
        let lo = SocketAddr::new(IpAddr::LOCALHOST_V4, 0);
        let (Ok(u), Ok(w)) = (s.bind_udp(&lo), s.listen_ws(&lo, "/chan")) else { return };
        let _ = tx.send((u, w));
        while !st.load(Ordering::Acquire) {
            while let Some(e) = s.poll() {
                if let ServerEvent::Message { id, data, .. } = e {
                    if data.first() == Some(&b'e') {
                        let _ = s.send(id, &data);
                    } else {
                        by.fetch_add(data.len() as u64, Ordering::Relaxed);
                    }
                }
            }
            core::hint::spin_loop();
        }
    }))?;
    let (udp, ws) = rx.recv_timeout(5000).ok()?;
    Some(ChanServer { stop, bytes, t, udp, ws })
}

fn connected(cl: &mut Client) -> bool {
    let end = plat::time_us() + 5_000_000;
    while plat::time_us() < end {
        while let Some(e) = cl.poll() {
            match e {
                ClientEvent::Connected => return true,
                ClientEvent::Disconnected(_) => return false,
                _ => {}
            }
        }
    }
    false
}

// n round trips of 32 bytes -> the samples in ms
fn chan_rtt(cl: &mut Client, n: usize) -> Vec<f64> {
    let mut v = Vec::with_capacity(n);
    let mut msg = alloc::vec![b'e'; 32];
    for i in 0..n + 100 {
        msg[1..9].copy_from_slice(&(i as u64).to_le_bytes());
        let t0 = plat::time_us();
        if cl.send(&msg).is_err() {
            break;
        }
        let end = t0 + 1_000_000;
        'wait: while plat::time_us() < end {
            while let Some(e) = cl.poll() {
                if let ClientEvent::Message { data, .. } = e {
                    if data == msg {
                        break 'wait;
                    }
                }
            }
        }
        if i >= 100 {
            v.push((plat::time_us() - t0) as f64 / 1000.0);
        }
    }
    v
}

// a second of messages of `size` as fast as they go -> (MB/s the server took, sent, the share of
// the bytes that never arrived).  (The acks ride on the server's keep-alives here, every 100 ms:
// Lost would name most of a flood, so the loss comes from the bytes.)
fn chan_flood(cl: &mut Client, srv: &ChanServer, size: usize) -> (f64, u64, f64) {
    let msg = alloc::vec![b'f'; size];
    let before = srv.bytes.load(Ordering::Relaxed);
    let t0 = plat::time_us();
    let mut sent = 0u64;
    let mut k = 0u64;
    while plat::time_us() < t0 + 1_000_000 {
        match cl.send(&msg) {
            Ok(_) => sent += 1,
            Err(Error::Backlog) => {}
            Err(_) => break,
        }
        k += 1;
        if k % 16 == 0 {
            while cl.poll().is_some() {}
        }
    }
    // what is still on its way
    let end = plat::time_us() + 5_000_000;
    let mut last = srv.bytes.load(Ordering::Relaxed);
    let mut quiet = plat::time_us();
    while plat::time_us() < end && plat::time_us() < quiet + 300_000 {
        while cl.poll().is_some() {}
        let now = srv.bytes.load(Ordering::Relaxed);
        if now != last {
            last = now;
            quiet = plat::time_us();
        }
    }
    let took = (last - before) as f64;
    let secs = (quiet - t0) as f64 / 1e6;
    let loss = 1.0 - took / (sent as f64 * size as f64).max(1.0);
    (took / secs / 1e6, sent, loss)
}

fn line(name: &str, v: &mut [f64]) {
    let (m, p) = median_p99(v);
    out(&format!("{:<34} median {:>7.3} ms   p99 {:>7.3} ms   ({} round trips)\n", name, m, p, v.len()));
}

// DATA of 1200 bytes through a pair of connections -> MB/s
fn conn_pass(sealed: bool, n: usize) -> f64 {
    use netchan::conn::{Conn, Settings};
    use netchan::seal::Keys;
    let set = Settings { timeout_ms: 5000, keepalive_ms: 100, packets_per_s: u32::MAX, bytes_per_s: u32::MAX };
    let (mut a, mut b) = (Conn::new(1, false, set, 0), Conn::new(1, false, set, 0));
    if sealed {
        a = a.with_keys(Keys::new([1; 32], [2; 32]));
        b = b.with_keys(Keys::new([2; 32], [1; 32]));
    }
    let payload = alloc::vec![7u8; 1200];
    let mut wire = Vec::with_capacity(1300);
    let t0 = plat::time_us();
    let mut ok = 0usize;
    for _ in 0..n {
        wire.clear();
        let seq = a.write_data(&payload, 0, &mut wire);
        if let Some(p) = b.open(&wire, netchan::packet::DATA_HEADER, seq) {
            ok += p.len();
        }
    }
    let us = (plat::time_us() - t0).max(1) as f64;
    ok as f64 / us
}

// handshakes in memory -> (the server's µs per RESPONSE, the client's per ACCEPT)
fn handshakes(stat: bool, n: usize) -> (f64, f64) {
    use netchan::endpoint::{ClientCore, Out, ServerCore, State};
    let cfg = ServerConfig { protocol: PROTOCOL, max_connections: n + 1, max_per_ip: n + 1, challenges_per_s: 100_000_000, ..ServerConfig::default() };
    let mut s = ServerCore::new(cfg, [5; 32], 1_000_000_000);
    if stat {
        s.set_static_key([6; 32]);
    }
    let ccfg = ClientConfig { protocol: PROTOCOL, server_key: s.public_key(), ..ClientConfig::default() };
    let (mut t_server, mut t_client, mut done) = (0u64, 0u64, 0usize);
    let now = 1_000_000_000u64;
    for k in 0..n {
        let mut r = [0u8; 32];
        r[..8].copy_from_slice(&(k as u64).to_le_bytes());
        let mut cl = ClientCore::new(ccfg, r, false);
        let from = SocketAddr::new(IpAddr::V4([10, (k >> 16) as u8, (k >> 8) as u8, k as u8]), 5000);
        cl.start(now);
        for round in 0..3 {
            while let Some(b) = cl.take_out() {
                let t0 = plat::time_us();
                s.datagram(from, &b, now);
                if round == 1 {
                    t_server += plat::time_us() - t0;
                }
            }
            while let Some(o) = s.take_out() {
                if let Out::Send(_, b) = o {
                    let t0 = plat::time_us();
                    cl.packet(&b, now);
                    if round == 1 {
                        t_client += plat::time_us() - t0;
                    }
                }
            }
        }
        while s.poll_event().is_some() {}
        done += (cl.state() == State::Connected) as usize;
    }
    let d = done.max(1) as f64;
    (t_server as f64 / d, t_client as f64 / d)
}

pub fn run(n: usize) -> i32 {
    out(&format!("nettest bench: {} on {} ({} threads)\n", if cfg!(windows) { "Windows" } else if cfg!(target_os = "linux") { "Linux" } else { "macOS" }, plat::cpu_brand(), plat::cpu_count()));
    let (plain, sealed) = (conn_pass(false, 50_000), conn_pass(true, 50_000));
    out(&format!("{:<34} {:>8.1} MB/s sealed and opened, {:.1} MB/s without the seal\n", "seal: DATA of 1200 B in memory", sealed, plain));
    for stat in [false, true] {
        let (srv, cli) = handshakes(stat, 500);
        out(&format!("{:<34} {:>8.1} us the server per RESPONSE, {:.1} us the client per ACCEPT\n", if stat { "handshake with the static key" } else { "handshake, ephemeral keys" }, srv, cli));
    }
    let Some(srv) = chan_server() else {
        out("FAIL bench: no channel server\n");
        return 1;
    };
    let ccfg = ClientConfig { protocol: PROTOCOL, packets_per_s: 10_000_000, bytes_per_s: u32::MAX, timeout_ms: 10_000, ..ClientConfig::default() };
    let mut failed = 0;
    for (name, ep, size) in [("channel over UDP", Endpoint::Udp(srv.udp), 1200usize), ("channel over WebSocket", Endpoint::WebSocket(format!("ws://{}/chan", srv.ws)), 16 * 1024)] {
        let Ok(mut cl) = Client::connect(ep, ccfg) else {
            failed += 1;
            continue;
        };
        if !connected(&mut cl) {
            out(&format!("FAIL bench: {} not connected\n", name));
            failed += 1;
            continue;
        }
        let mut v = chan_rtt(&mut cl, n);
        line(&format!("{}: rtt 32 B", name), &mut v);
        let (mbs, sent, loss) = chan_flood(&mut cl, &srv, size);
        out(&format!("{:<34} {:>8.1} MB/s taken by the server, {} messages of {} B sent, {:.1} % lost\n", format!("{}: flood", name), mbs, sent, size, loss * 100.0));
        cl.disconnect();
    }
    srv.stop.store(true, Ordering::Release);
    srv.t.join();

    // netio's WebSocket alone
    let log = Arc::new(Mutex::new(Vec::new()));
    let lim = Limits { messages_per_s: 10_000_000, bytes_per_s: u32::MAX, max_message: 4 << 20, max_frame: 4 << 20, ..Limits::default() };
    let (server, _, addr) = crate::wstest::echo_server(lim, |a| a, log);
    let cfg = WsClientConfig { limits: lim, ..WsClientConfig::default() };
    match ws::connect(&format!("ws://{}/echo", addr), &cfg) {
        Ok(w) => {
            let mut v = Vec::with_capacity(n);
            let msg = alloc::vec![7u8; 32];
            for i in 0..n + 100 {
                let t0 = plat::time_us();
                if w.send_binary(&msg).is_err() || w.recv().is_err() {
                    break;
                }
                if i >= 100 {
                    v.push((plat::time_us() - t0) as f64 / 1000.0);
                }
            }
            line("WebSocket (netio): rtt 32 B", &mut v);
            // 64 MiB from the server in messages of 64 KiB
            let t0 = plat::time_us();
            let _ = w.send_text("blast:1024:65536");
            let mut got = 0usize;
            while got < 1024 << 16 {
                match w.recv() {
                    Ok(Message::Binary(b)) => got += b.len(),
                    _ => break,
                }
            }
            let s = (plat::time_us() - t0) as f64 / 1e6;
            out(&format!("{:<34} {:>8.1} MB/s ({} MiB in messages of 64 KiB)\n", "WebSocket (netio): server -> client", got as f64 / s / 1e6, got >> 20));
            // 64 MiB echoed: the client writes on a thread of its own while this one reads
            let w = Arc::new(w);
            let w2 = w.clone();
            let t0 = plat::time_us();
            let writer = plat::thread_spawn(256 * 1024, Box::new(move || {
                let b = alloc::vec![9u8; 65536];
                for _ in 0..1024 {
                    if w2.send_binary(&b).is_err() {
                        break;
                    }
                }
            }));
            let mut got = 0usize;
            while got < 1024 << 16 {
                match w.recv() {
                    Ok(Message::Binary(b)) => got += b.len(),
                    _ => break,
                }
            }
            if let Some(t) = writer {
                t.join();
            }
            let s = (plat::time_us() - t0) as f64 / 1e6;
            out(&format!("{:<34} {:>8.1} MB/s each way ({} MiB echoed)\n", "WebSocket (netio): echo", got as f64 / s / 1e6, got >> 20));
        }
        Err(e) => {
            out(&format!("FAIL bench: WebSocket: {}\n", e));
            failed += 1;
        }
    }
    drop(server);
    if failed > 0 { 1 } else { 0 }
}
