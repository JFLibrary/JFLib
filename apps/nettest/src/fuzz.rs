// fuzz -- a fixed-seed random corpus against the parsers of untrusted bytes: netio's frame decoder
// (random bytes, mutated valid streams, both roles, fed in random pieces), the channel's packet
// parser (random and mutated packets, round trips of valid ones), the server's and the client's
// protocol (random and mutated datagrams and stream messages; UDP clients after a real handshake,
// so the sealed phase gets them too), one connection's acks, sealed packets (every changed byte
// fails the tag, the genuine ones open to what was sent).  Nothing may panic; the decoder's
// buffer and messages stay within their limits, the server within its caps.

use alloc::format;
use alloc::vec::Vec;

use netchan::conn::{Conn, Settings};
use netchan::endpoint::{ClientCore, Out, Route, ServerConfig, ServerCore, State};
use netchan::packet::{self, Packet};
use netchan::seal::Keys;
use plat::net::{IpAddr, SocketAddr};

use crate::sim::{client_cfg, server_cfg};
use crate::{Ctx, Rng, PROTOCOL};

/// flip, drop, insert or overwrite a few bytes
fn mutate(rng: &mut Rng, b: &mut Vec<u8>) {
    for _ in 0..1 + rng.below(4) {
        match rng.below(6) {
            0 if !b.is_empty() => {
                let i = rng.below(b.len());
                b[i] ^= 1 << rng.below(8);
            }
            1 if !b.is_empty() => {
                let i = rng.below(b.len());
                b.remove(i);
            }
            2 => {
                let i = rng.below(b.len() + 1);
                b.insert(i, rng.next_u64() as u8);
            }
            3 if !b.is_empty() => {
                let i = rng.below(b.len());
                b[i] = [0, 0x7f, 0x80, 0xff, 126, 127][rng.below(6)];
            }
            4 => {
                let n = rng.below(b.len() + 1);
                b.truncate(n);
            }
            _ => {
                let extra = rng.some_bytes(0, 20);
                b.extend_from_slice(&extra);
            }
        }
    }
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn ws_decoder(c: &mut Ctx, rng: &mut Rng) -> u64 {
    use netio::ws::{self, Decoder, Frame, Limits, Message, Role};
    let lim = Limits { max_frame: 2048, max_message: 4096, max_fragments: 16, ..Limits::default() };
    let bound = lim.max_frame + 14 + 64 * 1024;
    let mut cases = 0u64;
    let mut over = 0u64;
    let n = c.n(200_000);
    for k in 0..n {
        let role = if k % 2 == 0 { Role::Server } else { Role::Client };
        // half random bytes, half a valid stream of frames with a few mutations
        let data = if k % 4 < 2 {
            let len = rng.below(300);
            let mut d = rng.bytes(len);
            if !d.is_empty() && rng.chance(0.5) {
                d[0] = [0x81, 0x82, 0x01, 0x00, 0x80, 0x88, 0x89, 0x8a][rng.below(8)];
            }
            d
        } else {
            let mut d = Vec::new();
            for _ in 0..1 + rng.below(6) {
                let op = [ws::OP_TEXT, ws::OP_BINARY, ws::OP_CONT, ws::OP_PING, ws::OP_PONG, ws::OP_CLOSE][rng.below(6)];
                let len = if op >= 8 { rng.below(126) } else { [0, 1, 125, 126, 1000, 2047, 2048, 2049, 70000][rng.below(9)] };
                // (text cut anywhere: sometimes inside a character, which is 1007)
                let p = if op == ws::OP_TEXT { "äöü✓".repeat(len / 9 + 1).into_bytes()[..len].to_vec() } else { rng.bytes(len) };
                let mask = if role == Role::Server { Some([rng.next_u64() as u8, 1, 2, 3]) } else { None };
                ws::encode_frame(&mut d, op, op >= 8 || rng.chance(0.7), &p, mask);
            }
            mutate(rng, &mut d);
            d
        };
        let mut dec = Decoder::new(role, lim);
        let mut pos = 0;
        'feed: while pos < data.len() {
            let step = 1 + rng.below(512);
            let end = (pos + step).min(data.len());
            if dec.feed(&data[pos..end]).is_err() {
                break;
            }
            pos = end;
            if dec.buffered() > bound {
                over += 1;
            }
            loop {
                match dec.next() {
                    Ok(Some(Frame::Message(m))) => {
                        let len = match &m {
                            Message::Text(t) => t.len(),
                            Message::Binary(b) => b.len(),
                        };
                        if len > lim.max_message {
                            over += 1;
                        }
                    }
                    Ok(Some(Frame::Ping(p) | Frame::Pong(p))) => {
                        if p.len() > 125 {
                            over += 1;
                        }
                    }
                    Ok(Some(Frame::Close(_))) => {}
                    Ok(None) => break,
                    Err(code) => {
                        if !matches!(code, ws::CLOSE_PROTOCOL | ws::CLOSE_BAD_DATA | ws::CLOSE_POLICY | ws::CLOSE_TOO_BIG) {
                            over += 1;
                        }
                        break 'feed;
                    }
                }
            }
        }
        cases += 1;
    }
    c.check(over == 0, || format!("ws decoder: {} cases over a limit or with an unknown close code", over));
    cases
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
fn ws_decoder(_c: &mut Ctx, _rng: &mut Rng) -> u64 {
    0
}

fn arr<const N: usize>(rng: &mut Rng) -> [u8; N] {
    let b = rng.bytes(N);
    let mut m = [0u8; N];
    m.copy_from_slice(&b);
    m
}

/// a valid packet of any kind
fn any_packet(rng: &mut Rng, payload: &[u8]) -> Vec<u8> {
    let mac = arr::<16>(rng);
    let tag = if rng.chance(0.5) { rng.bytes(packet::TAG) } else { Vec::new() };
    let p = match rng.below(7) {
        0 => Packet::Connect { protocol: PROTOCOL, nonce: rng.next_u64() },
        1 => Packet::Challenge { nonce: rng.next_u64(), time: rng.next_u64() as u32, mac },
        2 => Packet::Response { protocol: PROTOCOL, nonce: rng.next_u64(), time: rng.next_u64() as u32, mac, flags: rng.below(3) as u8, key: arr(rng) },
        3 => Packet::Accept { nonce: rng.next_u64(), token: rng.next_u64() as u32, max_payload: rng.next_u64() as u16, key: arr(rng), confirm: arr(rng) },
        4 => Packet::Deny { nonce: rng.next_u64(), reason: rng.next_u64() as u8 },
        5 => Packet::Data { token: rng.next_u64() as u32, seq: rng.next_u64() as u16, ack: rng.next_u64() as u16, bits: rng.next_u64() as u32, payload },
        _ => Packet::Disconnect { token: rng.next_u64() as u32, seq: rng.next_u64() as u16, reason: rng.next_u64() as u8, tag: &tag },
    };
    p.to_vec(rng.chance(0.5))
}

/// a UDP client through a real handshake with `s` from `from` (None: it did not connect)
fn handshake(s: &mut ServerCore, cl: &mut ClientCore, from: SocketAddr, now: u64) -> Option<()> {
    cl.start(now);
    for _ in 0..4 {
        while let Some(b) = cl.take_out() {
            s.datagram(from, &b, now);
        }
        while let Some(o) = s.take_out() {
            if let Out::Send(Route::Udp(a), b) = o {
                if a == from {
                    cl.packet(&b, now);
                }
            }
        }
        while s.poll_event().is_some() {}
    }
    (cl.state() == State::Connected).then_some(())
}

pub fn run(quick: bool) -> Ctx {
    let mut c = Ctx::new("fuzz", quick);
    let mut rng = Rng::new(0x6e65_7474);
    let mut cases = ws_decoder(&mut c, &mut rng);

    // the packet parser: random bytes never panic; valid packets survive a round trip
    let n = c.n(500_000);
    let mut rt_bad = 0u64;
    for _ in 0..n {
        let len = rng.below(80);
        let mut b = rng.bytes(len);
        if !b.is_empty() {
            b[0] = (b[0] & 0x0f) | if rng.chance(0.9) { packet::VERSION << 4 } else { b[0] & 0xf0 };
        }
        let _ = packet::parse(&b);
        let payload = rng.some_bytes(0, 40);
        let v = any_packet(&mut rng, &payload);
        match packet::parse(&v) {
            Some(p) => {
                let pad = v.len() == packet::HANDSHAKE_SIZE;
                if p.to_vec(pad) != v {
                    rt_bad += 1;
                }
            }
            None => rt_bad += 1,
        }
        let mut m = v.clone();
        mutate(&mut rng, &mut m);
        let _ = packet::parse(&m);
        cases += 3;
    }
    c.check(rt_bad == 0, || format!("packets: {} valid ones did not survive parse and write", rt_bad));

    // the server: random and mutated datagrams from many addresses, stream messages
    let scfg = ServerConfig { max_connections: 8, max_per_ip: 4, ..server_cfg() };
    let mut s = ServerCore::new(scfg, [9; 32], 1_000_000);
    let mut now = 1_000_000u64;
    let mut over = 0u64;
    let mut out_bytes = 0usize;
    let n = c.n(300_000);
    for k in 0..n {
        now += 50 + rng.below(200) as u64;
        let from = SocketAddr::new(IpAddr::V4([10, 1, rng.below(3) as u8, rng.below(6) as u8]), 1000 + rng.below(4) as u16);
        let mut b = if rng.chance(0.5) {
            let payload = rng.some_bytes(0, 1500);
            any_packet(&mut rng, &payload)
        } else {
            rng.some_bytes(0, 100)
        };
        if rng.chance(0.5) {
            mutate(&mut rng, &mut b);
        }
        if k % 7 == 0 {
            let st = rng.below(20) as u32;
            match rng.below(4) {
                0 => s.stream_open(st, Some(from), now),
                1 => s.stream_closed(st, 1000),
                _ => s.stream_message(st, &b, now),
            }
        } else {
            s.datagram(from, &b, now);
        }
        if k % 13 == 0 {
            s.tick(now);
        }
        while let Some(o) = s.take_out() {
            if let Out::Send(_, b) = o {
                out_bytes += b.len();
            }
        }
        while s.poll_event().is_some() {}
        if s.connections() > scfg.max_connections {
            over += 1;
        }
        cases += 1;
    }
    let _ = out_bytes;
    c.check(over == 0, || format!("server: over max_connections {} times", over));

    // the client: random and mutated packets in every phase; a WebSocket one connects with a
    // matching ACCEPT now and then, a UDP one after a real handshake half of the time, and gets
    // mutated copies of the server's sealed packets too
    let n = c.n(100_000);
    let mut hs_failed = 0u64;
    for k in 0..n {
        let stream = k % 3 == 0;
        let mut cl = ClientCore::new(client_cfg(), [42; 32], stream);
        let mut now = 5_000_000u64;
        let mut srv = None;
        if !stream && rng.chance(0.5) {
            let mut s2 = ServerCore::new(server_cfg(), [7; 32], now);
            let from = SocketAddr::new(IpAddr::V4([10, 2, 0, 1]), 4000);
            if handshake(&mut s2, &mut cl, from, now).is_none() {
                hs_failed += 1;
            }
            srv = Some((s2, from));
        } else {
            cl.start(now);
        }
        // the nonce of the attempt, for a matching stream ACCEPT
        let nonce = match cl.take_out().as_deref().and_then(packet::parse) {
            Some(Packet::Connect { nonce, .. }) => nonce,
            _ => 0,
        };
        for _ in 0..8 {
            now += rng.below(200_000) as u64;
            let payload = rng.some_bytes(0, 200);
            let mut b = any_packet(&mut rng, &payload);
            if stream && rng.chance(0.2) {
                b = Packet::Accept { nonce, token: 0, max_payload: 1200, key: [0; 32], confirm: [0; 16] }.to_vec(false);
            } else if let Some((s2, _)) = srv.as_mut().filter(|_| rng.chance(0.5)) {
                // a genuine sealed packet of the server, mutated or not
                let id = s2.ids().first().copied().unwrap_or(0);
                let _ = s2.send(id, &payload, now);
                b = match s2.take_out() {
                    Some(Out::Send(_, x)) => x,
                    _ => b,
                };
                if rng.chance(0.7) {
                    mutate(&mut rng, &mut b);
                }
            } else if rng.chance(0.5) {
                mutate(&mut rng, &mut b);
            }
            cl.packet(&b, now);
            cl.tick(now);
            let _ = cl.send(&payload, now);
            while let Some(x) = cl.take_out() {
                if let Some((s2, from)) = srv.as_mut() {
                    s2.datagram(*from, &x, now);
                }
            }
            while cl.poll_event().is_some() {}
        }
        cases += 1;
    }
    c.check(hs_failed == 0, || format!("client: {} real handshakes did not connect", hs_failed));

    // sealed packets: a genuine one opens to its payload once; one with any byte changed, cut
    // or lengthened never opens, and the connection's state stays as it was
    let n = c.n(50_000);
    let (mut opened_wrong, mut forged_opened) = (0u64, 0u64);
    let set = Settings { timeout_ms: 5000, keepalive_ms: 100, packets_per_s: 1_000_000, bytes_per_s: 1 << 30 };
    let (ka, kb) = (arr::<32>(&mut rng), arr::<32>(&mut rng));
    let mut a = Conn::new(7, false, set, 0).with_keys(Keys::new(ka, kb));
    let mut b = Conn::new(7, false, set, 0).with_keys(Keys::new(kb, ka));
    let mut out = Vec::new();
    for _ in 0..n {
        let payload = rng.some_bytes(0, 300);
        out.clear();
        let seq = if rng.chance(0.05) {
            a.write_disconnect(rng.next_u64() as u8, &mut out);
            u16::from_le_bytes([out[5], out[6]])
        } else {
            a.write_data(&payload, 0, &mut out)
        };
        let head = if out.len() == packet::DISCONNECT_HEADER + packet::TAG { packet::DISCONNECT_HEADER } else { packet::DATA_HEADER };
        let mut m = out.clone();
        mutate(&mut rng, &mut m);
        if m != out {
            let mseq = if m.len() >= 7 { u16::from_le_bytes([m[5], m[6]]) } else { seq };
            if b.open(&m, head, mseq).is_some() {
                forged_opened += 1;
            }
        }
        match b.open(&out, head, seq) {
            Some(p) if head == packet::DISCONNECT_HEADER && p.is_empty() => {}
            Some(p) if head == packet::DATA_HEADER && p == payload => {}
            _ => opened_wrong += 1,
        }
        cases += 2;
    }
    c.check(forged_opened == 0, || format!("sealed: {} changed packets opened", forged_opened));
    c.check(opened_wrong == 0, || format!("sealed: {} genuine packets did not open to what was sent", opened_wrong));
    c.check(b.stats().forged > 0, || "sealed: no forged packet counted".into());

    // one connection's acks: random numbers, acks and bit fields, never more decided than sent
    let n = c.n(20_000);
    let mut bad = 0u64;
    for _ in 0..n {
        let set = Settings { timeout_ms: 5000, keepalive_ms: 100, packets_per_s: 1000, bytes_per_s: 1 << 20 };
        let mut conn = Conn::new(1, rng.chance(0.3), set, 0);
        let mut out = Vec::new();
        let mut now = 0u64;
        let mut decided = 0u64;
        let mut sent = 0u64;
        for _ in 0..200 {
            now += rng.below(5000) as u64;
            if rng.chance(0.5) {
                out.clear();
                conn.write_data(b"m", now, &mut out);
                sent += 1;
            }
            let near = conn.next_seq().wrapping_sub(rng.below(100) as u16);
            let ack = if rng.chance(0.5) { near } else { rng.next_u64() as u16 };
            conn.on_data(rng.next_u64() as u16, ack, rng.next_u64() as u32, b"x", now);
            while let Some(e) = conn.take_event() {
                if matches!(e, netchan::conn::Event::Delivered(_) | netchan::conn::Event::Lost(_)) {
                    decided += 1;
                }
            }
        }
        if decided > sent {
            bad += 1;
        }
        let st = conn.stats();
        if st.delivered + st.lost != decided {
            bad += 1;
        }
        cases += 1;
    }
    c.check(bad == 0, || format!("conn: {} runs decided more than they sent or miscounted", bad));
    crate::out(&format!("fuzz    {} cases (seed 0x6e657474)\n", cases));
    c
}
