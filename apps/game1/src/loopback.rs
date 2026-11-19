// loopback -- game 1 on real sockets (`game1 --loopback`, tests/loopback.uis): the server's loop of game1_core::host
// on 127.0.0.1 (UDP and the WebSocket path /game1, never 0.0.0.0) with four bots in the room, two clients over UDP and
// two over WebSocket (NetLink), driven by the bots' minds from what each client sees, all in one thread for a few real
// seconds; then the counts: every client plays, snapshots come and decode against a base the client has, shots fall

use alloc::format;
use alloc::vec::Vec;

use game::net::client::{ClientOptions, NetClient, Phase};
use game::net::link::NetLink;
use game1_core::host::{Host, HostOptions};
use game1_core::movement::Mover;
use game1_core::rules::{schema, PROTOCOL, WS_PATH};
use game1_core::sim::{Match, MatchConfig};
use netchan::{ClientConfig, Endpoint, Server, ServerConfig};
use plat::net::{IpAddr, SocketAddr};

use crate::out;
use crate::t_net::Drivers;

/// -> exit code (1 when a check fails)
pub fn run(secs: u64) -> i32 {
    let map = game1_core::map::werft();
    let mut chan = Server::new(ServerConfig { protocol: PROTOCOL, max_per_ip: 64, ..ServerConfig::default() });
    let lo = SocketAddr::new(IpAddr::LOCALHOST_V4, 0);
    let udp = match chan.bind_udp(&lo) {
        Ok(a) => a,
        Err(e) => {
            out(&format!("loopback: no UDP socket: {}\n", e));
            return 1;
        }
    };
    let ws = match chan.listen_ws(&lo, WS_PATH) {
        Ok(a) => a,
        Err(e) => {
            out(&format!("loopback: no WebSocket server: {}\n", e));
            return 1;
        }
    };
    let opts = HostOptions { room: MatchConfig { bots: 4, ..MatchConfig::default() }, ..HostOptions::default() };
    let mut host = Host::new(chan, ws.port, opts, map.clone(), plat::random_u64());
    out(&format!("loopback: UDP {}, WebSocket ws://{}{}\n", udp, ws, WS_PATH));
    let ccfg = ClientConfig { protocol: PROTOCOL, ..ClientConfig::default() };
    let mut links: Vec<NetLink<Mover>> = Vec::new();
    let mut d = Drivers::new(4, 41, true);
    let mut room = None;
    let t0 = plat::time_us();
    let mut next = 0usize;
    loop {
        let now_us = plat::time_us();
        let now = now_us * 1000;
        if next < 4 && (next == 0 || room.is_some()) && now_us > t0 + next as u64 * 150_000 {
            let is_ws = next >= 2;
            let nc = NetClient::new(ClientOptions { name: format!("Echt {}", next + 1), room, ..ClientOptions::default() }, Mover::default(), schema(), Match::empty_baseline());
            let ep = if is_ws { Endpoint::WebSocket(format!("ws://{}{}", ws, WS_PATH)) } else { Endpoint::Udp(udp) };
            match NetLink::connect(ep, ccfg, nc, WS_PATH) {
                Ok(l) => links.push(l),
                Err(e) => out(&format!("loopback: client {} did not connect: {}\n", next, e)),
            }
            next += 1;
        }
        host.round(now);
        for (k, l) in links.iter_mut().enumerate() {
            l.pump(&map.world, now);
            if room.is_none() && l.game.phase() == Phase::Playing {
                room = l.game.welcome().map(|w| w.room);
            }
            for _ in 0..l.game.advance(now) {
                let i = d.input(&map, k, &l.game);
                l.game.step(&map.world, i);
            }
            l.flush();
        }
        if now_us > t0 + secs * 1_000_000 {
            break;
        }
        host.wait(1);
    }
    let mut failed = 0;
    for (k, l) in links.iter().enumerate() {
        let st = l.game.stats();
        let ok = l.game.phase() == Phase::Playing && st.snapshots >= secs * 15 && st.bad == 0 && st.no_base == 0;
        out(&format!(
            "loopback: client {} ({}) {:?}: {} snapshots, {} bad, {} no base, {} corrections, round trip {:.1} ms{}\n",
            k,
            if k >= 2 { "WebSocket" } else { "UDP" },
            l.game.phase(),
            st.snapshots,
            st.bad,
            st.no_base,
            l.game.predictor().stats().mismatches,
            l.channel().rtt_ms(),
            if ok { "" } else { "  FAIL" }
        ));
        if !ok {
            failed += 1;
        }
    }
    let stats = room.and_then(|r| host.game.room(&r)).map(|m| m.stats()).unwrap_or_default();
    out(&format!("loopback: the room: {} shots, {} hits, {} kills\n", stats.shots, stats.hits, stats.kills));
    if links.len() != 4 || stats.shots == 0 {
        failed += 1;
    }
    out(&format!("loopback: {}\n", if failed == 0 { "bestanden" } else { "FEHLGESCHLAGEN" }));
    if failed == 0 { 0 } else { 1 }
}
