// reltest -- lib/netchan's reliable channel (reliable.rs) on the simulated network of sim.rs.
// The reference is the bookkeeping here: the messages each side handed to send_reliable, in
// order, against what came out of the other side.
//
//   every message comes exactly once, whole and unchanged, in the order it was sent, both ways,
//   beside unreliable traffic, under loss, duplicates and reordering, also past the wrap of the
//   16-bit chunk numbers; large messages in chunks of a packet; resends only where packets were
//   lost (none over a WebSocket); the pace keeps a 1 MiB message within the server's default
//   rate limits; Size and Backlog; a peer that breaks the protocol (a malformed chunk, a message
//   over max_message, chunks far ahead beyond the early buffer) is disconnected

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netchan::endpoint::{ClientConfig, ClientCore, ClientEvent, Out, Route, ServerConfig, ServerCore, ServerEvent, State};
use netchan::packet::Packet;
use netchan::{Error, Reason};

use crate::sim::{client_addr, client_cfg, server_cfg, Sim, Weather, CLEAR};
use crate::{Ctx, Rng};

/// a message: its number, its length and filler from both (checked whole on arrival)
fn body(n: u64, len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(len.max(12));
    v.extend_from_slice(&n.to_le_bytes());
    v.extend_from_slice(&(len as u32).to_le_bytes());
    let mut x = (n as u32) ^ 0x1234_5678;
    while v.len() < len.max(12) {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        v.push((x >> 24) as u8);
    }
    v
}

fn number(d: &[u8]) -> Option<u64> {
    let n = u64::from_le_bytes(d.get(..8)?.try_into().ok()?);
    let len = u32::from_le_bytes(d.get(8..12)?.try_into().ok()?) as usize;
    (d.len() == len.max(12) && body(n, len) == d).then_some(n)
}

/// a size: mostly small, some of several packets, a few large ones and exact chunk borders
fn size(rng: &mut Rng, chunk: usize) -> usize {
    match rng.below(20) {
        0 => 1 + rng.below(200_000),
        1 => chunk * (1 + rng.below(4)),
        2 => chunk * (1 + rng.below(4)) + 1,
        3..=6 => 1 + rng.below(chunk * 3),
        _ => 1 + rng.below(300),
    }
}

/// what one direction carried: the numbers in the order they came, damaged or unknown ones
#[derive(Default)]
struct Got {
    order: Vec<u64>,
    bad: u64,
}

impl Got {
    fn take(&mut self, d: &[u8]) {
        match number(d) {
            Some(n) => self.order.push(n),
            None => self.bad += 1,
        }
    }
}

fn generous(s: ServerConfig) -> ServerConfig {
    ServerConfig { packets_per_s: 1_000_000, bytes_per_s: 1 << 30, timeout_ms: 30_000, reliable_bytes_per_s: 4 << 20, max_reliable_queued: 64 << 20, ..s }
}

fn generous_client(c: ClientConfig) -> ClientConfig {
    ClientConfig { packets_per_s: 1_000_000, bytes_per_s: 1 << 30, timeout_ms: 30_000, connect_ms: 30_000, reliable_bytes_per_s: 4 << 20, max_reliable_queued: 64 << 20, ..c }
}

/// a connected client on a sim -> (sim, client index, server id)
fn connect(seed: u64, w: Weather, scfg: ServerConfig, ccfg: ClientConfig) -> Option<(Sim, usize, u32)> {
    let mut sim = Sim::new(seed, w, scfg);
    let ci = sim.add_client(client_addr(0), ccfg);
    sim.settle(30_000);
    let sid = sim.server_events().iter().find_map(|e| if let ServerEvent::Connected { id, .. } = e { Some(*id) } else { None })?;
    (sim.clients[ci].1.state() == State::Connected).then_some((sim, ci, sid))
}

/// `n` reliable messages each way, unreliable ones beside them, under `w`
fn exchange(c: &mut Ctx, name: &str, seed: u64, w: Weather, n: usize) {
    let Some((mut sim, ci, sid)) = connect(seed, w, generous(server_cfg()), generous_client(client_cfg())) else {
        c.check(false, || format!("{}: no connection", name));
        return;
    };
    let chunk = sim.clients[ci].1.max_payload() - netchan::reliable::CHUNK_HEAD;
    let mut rng = Rng::new(seed ^ 0x5151);
    let (mut up, mut down) = (Got::default(), Got::default());
    let (mut sent_up, mut sent_down) = (0u64, 0u64);
    let (mut bytes, mut refused) = (0usize, 0u32);
    let t0 = sim.now;
    while sim.now < t0 + 120_000_000 && (up.order.len() < n || down.order.len() < n) {
        if sent_up < n as u64 && rng.below(4) == 0 {
            let len = size(&mut rng, chunk);
            match sim.clients[ci].1.send_reliable(&body(sent_up, len), sim.now) {
                Ok(()) => {
                    sent_up += 1;
                    bytes += len;
                }
                Err(_) => refused += 1,
            }
        }
        if sent_down < n as u64 && rng.below(4) == 0 {
            let len = size(&mut rng, chunk);
            match sim.server.send_reliable(sid, &body(sent_down, len), sim.now) {
                Ok(()) => {
                    sent_down += 1;
                    bytes += len;
                }
                Err(_) => refused += 1,
            }
        }
        // unreliable traffic beside it (its own checks are sim.rs's)
        if rng.below(3) == 0 {
            let _ = sim.clients[ci].1.send(&[7u8; 40], sim.now);
            let _ = sim.server.send(sid, &[8u8; 60], sim.now);
        }
        sim.flush();
        sim.step(500);
        for e in sim.server_events() {
            match e {
                ServerEvent::Reliable { data, .. } => up.take(&data),
                ServerEvent::Disconnected { reason, .. } => {
                    c.check(false, || format!("{}: the server lost the client: {:?}", name, reason));
                }
                _ => {}
            }
        }
        for e in sim.client_events(ci) {
            match e {
                ClientEvent::Reliable(data) => down.take(&data),
                ClientEvent::Disconnected(r) => {
                    c.check(false, || format!("{}: the client was disconnected: {:?}", name, r));
                }
                _ => {}
            }
        }
    }
    let secs = (sim.now - t0) as f64 / 1e6;
    // ten seconds more: the last acks come (resends back off up to a second), and nothing may
    // arrive a second time
    for _ in 0..20_000 {
        sim.step(500);
        for e in sim.server_events() {
            if let ServerEvent::Reliable { data, .. } = e {
                up.take(&data);
            }
        }
        for e in sim.client_events(ci) {
            if let ClientEvent::Reliable(data) = e {
                down.take(&data);
            }
        }
    }
    for (dir, g, sent) in [("up", &up, sent_up), ("down", &down, sent_down)] {
        let in_order = g.order.iter().enumerate().all(|(i, &k)| k == i as u64);
        c.check(g.bad == 0 && in_order && g.order.len() as u64 == sent && sent == n as u64, || {
            format!("{} {}: {} of {} messages, in order {}, damaged {} ({:.1} s, {} refused)", name, dir, g.order.len(), sent, in_order, g.bad, secs, refused)
        });
    }
    let resent = sim.clients[ci].1.stats().resent + sim.server.stats(sid).map_or(0, |s| s.resent);
    let lossy = w.loss > 0.0;
    c.check(lossy == (resent > 0), || format!("{}: {} chunks sent again (loss {})", name, resent, w.loss));
    c.check(sim.clients[ci].1.stats().reliable_queued == 0 && sim.server.stats(sid).is_some_and(|s| s.reliable_queued == 0), || format!("{}: nothing left queued", name));
    crate::out(&format!("  reliable {}: {} messages each way, {:.1} MB, {:.1} s simulated, {} chunks again\n", name, n, bytes as f64 / 1e6, secs, resent));
}

/// a 1 MiB message within the server's default limits (300 packets and 256 KiB a second)
fn pace(c: &mut Ctx) {
    let Some((mut sim, ci, sid)) = connect(70, Weather { delay_us: 20_000, ..CLEAR }, server_cfg(), client_cfg()) else {
        c.check(false, || String::from("pace: no connection"));
        return;
    };
    let big = body(1, 1 << 20);
    let t0 = sim.now;
    c.check(sim.clients[ci].1.send_reliable(&big, sim.now).is_ok(), || String::from("pace: send 1 MiB"));
    let mut got = None;
    while sim.now < t0 + 30_000_000 && got.is_none() {
        sim.step(1000);
        for e in sim.server_events() {
            if let ServerEvent::Reliable { data, .. } = e {
                got = Some((data, sim.now - t0));
            }
        }
    }
    let ok = got.as_ref().is_some_and(|(d, _)| *d == big);
    let t = got.as_ref().map_or(0, |g| g.1);
    let dropped = sim.server.stats(sid).map_or(u64::MAX, |s| s.dropped);
    c.check(ok, || format!("pace: the message whole ({:?} µs)", got.as_ref().map(|g| g.1)));
    // 128 KiB a second after a burst of two seconds' worth: about six seconds
    let resent = sim.clients[ci].1.stats().resent;
    c.check((4_500_000..9_000_000).contains(&t), || format!("pace: 1 MiB in {} µs at 128 KiB/s ({} chunks sent again; {:?})", t, resent, sim.clients[ci].1.stats()));
    c.check(dropped == 0, || format!("pace: the server dropped {} packets of the client", dropped));
    crate::out(&format!("  reliable pace: 1 MiB in {:.2} s at the default 128 KiB/s, {} chunks again, none dropped by the server: {}\n", t as f64 / 1e6, resent, dropped == 0));
}

/// Size, Backlog, NotConnected
fn limits(c: &mut Ctx) {
    let scfg = ServerConfig { max_message: 4096, max_reliable_queued: 10_000, ..server_cfg() };
    let ccfg = ClientConfig { max_message: 4096, max_reliable_queued: 10_000, ..client_cfg() };
    let Some((mut sim, ci, sid)) = connect(71, CLEAR, scfg, ccfg) else {
        c.check(false, || String::from("limits: no connection"));
        return;
    };
    let cl = &mut sim.clients[ci].1;
    let now = sim.now;
    c.check(cl.send_reliable(&[], now) == Err(Error::Size), || String::from("limits: an empty message is Size"));
    c.check(cl.send_reliable(&[1; 4097], now) == Err(Error::Size), || String::from("limits: over max_message is Size"));
    c.check(cl.send_reliable(&[1; 4096], now).is_ok() && cl.send_reliable(&[1; 4096], now).is_ok(), || String::from("limits: max_message twice"));
    c.check(cl.send_reliable(&[1; 4096], now) == Err(Error::Backlog), || String::from("limits: past max_reliable_queued is Backlog"));
    c.check(sim.server.send_reliable(sid + 100, b"x", now) == Err(Error::NotConnected), || String::from("limits: no such client"));
    let mut fresh = ClientCore::new(client_cfg(), [1; 32], false);
    c.check(fresh.send_reliable(b"x", 0) == Err(Error::NotConnected), || String::from("limits: before Connected"));
}

/// the reliable channel over a stream (WebSocket): in order, never sent again
fn stream(c: &mut Ctx, n: usize) {
    let scfg = ServerConfig { packets_per_s: 1_000_000, bytes_per_s: 1 << 30, max_stream_payload: 8192, reliable_bytes_per_s: 8 << 20, ..server_cfg() };
    let mut server = ServerCore::new(scfg, [3u8; 32], 0);
    let mut cl = ClientCore::new(ClientConfig { packets_per_s: 1_000_000, bytes_per_s: 1 << 30, reliable_bytes_per_s: 8 << 20, ..client_cfg() }, [99; 32], true);
    let mut now = 1_000_000u64;
    server.stream_open(7, Some(client_addr(0)), now);
    cl.start(now);
    let mut rng = Rng::new(52);
    let (mut sent, mut got, mut sid) = (0u64, Got::default(), None);
    let mut max_packet = 0usize;
    for _ in 0..200_000 {
        now += 250;
        while let Some(b) = cl.take_out() {
            server.stream_message(7, &b, now);
        }
        while let Some(o) = server.take_out() {
            if let Out::Send(Route::Stream(7), b) = o {
                max_packet = max_packet.max(b.len());
                cl.packet(&b, now);
            }
        }
        server.tick(now);
        cl.tick(now);
        while let Some(e) = server.poll_event() {
            if let ServerEvent::Connected { id, .. } = e {
                sid = Some(id);
            }
        }
        while let Some(e) = cl.poll_event() {
            if let ClientEvent::Reliable(d) = e {
                got.take(&d);
            }
        }
        if let Some(id) = sid {
            if sent < n as u64 && server.send_reliable(id, &body(sent, size(&mut rng, 8000)), now).is_ok() {
                sent += 1;
            }
        }
        if got.order.len() == n {
            break;
        }
    }
    let in_order = got.order.iter().enumerate().all(|(i, &k)| k == i as u64);
    let resent = sid.and_then(|id| server.stats(id)).map_or(u64::MAX, |s| s.resent);
    c.check(got.order.len() == n && in_order && got.bad == 0, || format!("stream: {} of {} messages, in order {}", got.order.len(), n, in_order));
    c.check(resent == 0, || format!("stream: {} chunks sent again over a stream", resent));
    c.check(max_packet <= 8192 + netchan::packet::DATA_HEADER, || format!("stream: a packet of {} bytes over max_stream_payload 8192", max_packet));
}

/// a client on a stream that writes what it likes -> whether the server ended it for the protocol
fn hostile(packets: &[Vec<u8>], scfg: ServerConfig) -> bool {
    let mut server = ServerCore::new(scfg, [3u8; 32], 0);
    let mut cl = ClientCore::new(client_cfg(), [5; 32], true);
    let now = 1_000_000;
    server.stream_open(9, Some(client_addr(1)), now);
    cl.start(now);
    while let Some(b) = cl.take_out() {
        server.stream_message(9, &b, now);
    }
    while let Some(o) = server.take_out() {
        if let Out::Send(Route::Stream(9), b) = o {
            cl.packet(&b, now);
        }
    }
    for (k, p) in packets.iter().enumerate() {
        server.stream_message(9, p, now + 1000 + k as u64);
    }
    let mut ended = false;
    while let Some(e) = server.poll_event() {
        ended |= matches!(e, ServerEvent::Disconnected { reason: Reason::Protocol, .. });
    }
    ended
}

fn reliable_packet(seq: u16, chunks: &[(u16, u8, &[u8])]) -> Vec<u8> {
    let mut payload = Vec::new();
    for (id, flags, data) in chunks {
        payload.extend_from_slice(&id.to_le_bytes());
        payload.push(*flags);
        payload.extend_from_slice(&(data.len() as u16).to_le_bytes());
        payload.extend_from_slice(data);
    }
    Packet::Reliable { token: 0, seq, ack: 0xffff, bits: 0, payload: &payload }.to_vec(false)
}

fn protocol(c: &mut Ctx) {
    let scfg = ServerConfig { max_message: 10_000, max_stream_payload: 64 * 1024, packets_per_s: 1_000_000, bytes_per_s: 1 << 30, ..server_cfg() };
    // a well-formed message is taken (the harness itself)
    c.check(!hostile(&[reliable_packet(0, &[(0, 0, b"hello")])], scfg), || String::from("protocol: a good chunk is taken"));
    let mut short = reliable_packet(0, &[(0, 0, b"hello")]);
    short.pop();
    c.check(hostile(&[short], scfg), || String::from("protocol: a chunk cut short"));
    c.check(hostile(&[reliable_packet(0, &[(0, 4, b"x")])], scfg), || String::from("protocol: an unknown flag"));
    c.check(hostile(&[reliable_packet(0, &[(0, 0, b"")])], scfg), || String::from("protocol: an empty chunk"));
    let piece = [7u8; 6000];
    c.check(hostile(&[reliable_packet(0, &[(0, 1, &piece)]), reliable_packet(1, &[(1, 0, &piece)])], scfg), || String::from("protocol: a message over max_message"));
    // chunks far ahead of the one missing: beyond the early buffer
    let big = alloc::vec![1u8; 60_000];
    let ahead: Vec<Vec<u8>> = (1..12u16).map(|i| reliable_packet(i, &[(i, 1, &big)])).collect();
    c.check(hostile(&ahead, ServerConfig { max_message: 1 << 20, ..scfg }), || String::from("protocol: chunks before their turn over the early buffer"));
    // beyond the window: dropped, not kept (and nothing breaks)
    c.check(!hostile(&[reliable_packet(0, &[(5000, 0, b"far")])], scfg), || String::from("protocol: a chunk beyond the window is dropped"));
}

pub fn run(quick: bool) -> Ctx {
    let mut c = Ctx::new("reliable", quick);
    let n = c.n(600).max(120);
    exchange(&mut c, "clear", 1, CLEAR, n);
    let rough = Weather { loss: 0.2, dup: 0.1, reorder: 0.2, extra_us: 30_000, delay_us: 20_000, jitter_us: 5_000 };
    for seed in 0..if quick { 1 } else { 3 } {
        exchange(&mut c, "rough", 10 + seed, rough, n);
    }
    let storm = Weather { loss: 0.5, dup: 0.2, reorder: 0.3, extra_us: 60_000, delay_us: 40_000, jitter_us: 20_000 };
    exchange(&mut c, "storm", 30, storm, n / 3);
    exchange(&mut c, "duplicates", 31, Weather { dup: 0.5, ..CLEAR }, n);
    pace(&mut c);
    limits(&mut c);
    stream(&mut c, n);
    protocol(&mut c);
    wrap(&mut c, if quick { 20_000 } else { 70_000 });
    c
}

/// more chunks than the 16-bit numbers hold, with loss
fn wrap(c: &mut Ctx, n: usize) {
    let Some((mut sim, ci, sid)) = connect(80, Weather { loss: 0.05, ..CLEAR }, generous(server_cfg()), generous_client(client_cfg())) else {
        c.check(false, || String::from("wrap: no connection"));
        return;
    };
    let (mut sent, mut got) = (0u64, Got::default());
    let t0 = sim.now;
    while sim.now < t0 + 600_000_000 && got.order.len() < n {
        for _ in 0..8 {
            if sent < n as u64 && sim.clients[ci].1.send_reliable(&body(sent, 12 + (sent % 30) as usize), sim.now).is_ok() {
                sent += 1;
            }
        }
        sim.step(500);
        for e in sim.server_events() {
            if let ServerEvent::Reliable { data, .. } = e {
                got.take(&data);
            }
        }
    }
    let _ = sid;
    let in_order = got.order.iter().enumerate().all(|(i, &k)| k == i as u64);
    c.check(got.order.len() == n && in_order && got.bad == 0, || format!("wrap: {} of {} messages past the wrap, in order {}", got.order.len(), n, in_order));
}
