// net_loop -- the netcode on real sockets (--loopback): a netchan::Server on 127.0.0.1 with UDP and a
// WebSocket path, the arena with 300 props, 150 of them away from the map's places (so the UDP
// clients fetch their full state over a WebSocket of their own), bots as NetLinks over UDP and over WebSocket, all in one thread for a few real seconds
// (the server's wait() sleeps until a packet comes); then the counts

use alloc::format;
use alloc::vec::Vec;

use game::net::arena::{self, Arena, ArenaConfig};
use game::net::bot::{remote_feet, Bot, BotConfig};
use game::net::client::{ClientOptions, NetClient, Phase};
use game::net::link::{serve, NetLink};
use game::net::server::{NetServer, ServerOptions};
use game::net::walker::Walker;
use netchan::{ClientConfig, Endpoint, Server, ServerConfig};
use plat::net::{IpAddr, SocketAddr};

use crate::net_session::PROTOCOL;

struct Player {
    link: NetLink<Walker>,
    bot: Bot,
    ws: bool,
    rem: Vec<game::net::Remote>,
    feet: Vec<(u16, game::Vec3)>,
}

/// -> exit code (1 when a check fails)
pub fn run(secs: u64, udp: usize, ws: usize) -> i32 {
    let out = crate::out;
    let cfg = ServerConfig { protocol: PROTOCOL, max_per_ip: 64, ..ServerConfig::default() };
    let mut server = Server::new(cfg);
    let any = SocketAddr::new(IpAddr::LOCALHOST_V4, 0);
    let udp_addr = match server.bind_udp(&any) {
        Ok(a) => a,
        Err(e) => {
            out(&format!("loopback: no UDP socket: {}\n", e));
            return 1;
        }
    };
    let ws_addr = match server.listen_ws(&any, "/game") {
        Ok(a) => a,
        Err(e) => {
            out(&format!("loopback: no WebSocket server: {}\n", e));
            return 1;
        }
    };
    let acfg = ArenaConfig { props: 300, moving: 30, displaced: 150, fire: true, ..ArenaConfig::default() };
    let opts = ServerOptions { ws_port: ws_addr.port, ..ServerOptions::default() };
    let mut game = NetServer::new(opts, plat::random_u64(), alloc::boxed::Box::new(move |_| Arena::new(acfg)));
    out(&format!("loopback: UDP {}, WebSocket ws://{}/game\n", udp_addr, ws_addr));
    let map = arena::map(&acfg);
    let ccfg = ClientConfig { protocol: PROTOCOL, ..ClientConfig::default() };
    let mut players: Vec<Player> = Vec::new();
    let mut room = None;
    let t_start = plat::time_us();
    let mut next = 0usize;
    // (where the loop's time goes: serving, the clients, waiting; the slowest round)
    let (mut t_serve, mut t_clients, mut t_wait, mut rounds, mut slowest) = (0u64, 0u64, 0u64, 0u64, 0u64);
    loop {
        let now_us = plat::time_us();
        let now = now_us * 1000;
        // the clients join one after another once the first has made the room
        if next < udp + ws && (next == 0 || room.is_some()) && now_us > t_start + next as u64 * 100_000 {
            let is_ws = next >= udp;
            let opts = ClientOptions { name: format!("Echt {}", next + 1), room, ..ClientOptions::default() };
            let nc = NetClient::new(opts, Walker::default(), arena::schema(), arena::baseline(&acfg));
            let ep = if is_ws { Endpoint::WebSocket(format!("ws://{}/game", ws_addr)) } else { Endpoint::Udp(udp_addr) };
            match NetLink::connect(ep, ccfg, nc, "/game") {
                Ok(link) => {
                    let bot = Bot::new(BotConfig { seed: 100 + next as u64, fire: true, ..BotConfig::default() });
                    players.push(Player { link, bot, ws: is_ws, rem: Vec::new(), feet: Vec::new() });
                }
                Err(e) => out(&format!("loopback: client {} did not connect: {}\n", next, e)),
            }
            next += 1;
        }
        let a = plat::time_us();
        serve(&mut server, &mut game, now);
        let b = plat::time_us();
        for p in players.iter_mut() {
            p.link.pump(&map, now);
            if room.is_none() {
                room = p.link.game.welcome().map(|w| w.room);
            }
            let n = p.link.game.advance(now);
            for _ in 0..n {
                p.link.game.remotes(&mut p.rem);
                remote_feet(&p.rem, &mut p.feet);
                let own = p.link.game.predicted().filter(|_| p.link.game.predictor().known()).map(|s| s.feet());
                let input = p.bot.think(own, &p.feet);
                p.link.game.step(&map, input);
            }
            p.link.flush();
        }
        let c = plat::time_us();
        serve(&mut server, &mut game, now);
        if now_us > t_start + secs * 1_000_000 {
            break;
        }
        let d = plat::time_us();
        server.wait(1);
        let e = plat::time_us();
        t_serve += (b - a) + (d - c);
        t_clients += c - b;
        t_wait += e - d;
        rounds += 1;
        slowest = slowest.max(e - a);
    }
    out(&format!("  loop: {} rounds, serving {} ms, clients {} ms, waiting {} ms, slowest round {:.1} ms\n", rounds, t_serve / 1000, t_clients / 1000, t_wait / 1000, slowest as f64 / 1000.0));
    // the counts
    let mut failed = 0;
    let mut check = |ok: bool, msg: &str| {
        if !ok {
            failed += 1;
            out(&format!("FAIL loopback: {}\n", msg));
        }
    };
    check(players.len() == udp + ws, &format!("{} of {} clients connected", players.len(), udp + ws));
    for (k, p) in players.iter().enumerate() {
        let g = &p.link.game;
        let st = g.stats();
        let ps = g.predictor().stats();
        out(&format!(
            "  client {} {:<9} {:?}: snapshots {:>4} ({} bytes), FULL {}, no base {}, bad {}, inputs {}, predicted {} ticks, mismatches {}, margin {:.1}, rtt {:.2} ms, send errors {}\n",
            k + 1,
            if p.ws { "WebSocket" } else { "UDP" },
            g.phase(),
            st.snapshots,
            st.bytes_in,
            st.fulls,
            st.no_base,
            st.bad,
            st.inputs_sent,
            ps.steps,
            ps.mismatches,
            g.clock().margin().unwrap_or(f64::NAN),
            p.link.channel().rtt_ms(),
            p.link.send_errors
        ));
        check(g.phase() == Phase::Playing, &format!("client {} not playing: {:?}", k + 1, g.phase()));
        check(st.no_base == 0 && st.bad == 0, &format!("client {}: no base {}, bad {}", k + 1, st.no_base, st.bad));
        check(st.snapshots as f64 >= (secs as f64 - 2.0) * 20.0, &format!("client {}: only {} snapshots", k + 1, st.snapshots));
        check(p.ws || st.fulls == 1, &format!("client {} (UDP): {} FULLs", k + 1, st.fulls));
    }
    let ss = game.stats();
    let a = room.and_then(|r| game.room(&r)).map(|a| a.stats()).unwrap_or_default();
    out(&format!("  server: {} ticks, {} snapshots ({} bytes, max {}), {} cut, {} FULL ({} bytes), shots {} hits {}\n", ss.ticks, ss.snapshots, ss.snapshot_bytes, ss.max_snapshot, ss.partial, ss.fulls, ss.full_bytes, a.shots, a.hits));
    check(ss.max_snapshot <= 1200 || ws > 0, "a UDP snapshot over 1200 bytes");
    out(&format!("gametest loopback: {}\n", if failed == 0 { "bestanden" } else { "FEHLER" }));
    if failed == 0 { 0 } else { 1 }
}
