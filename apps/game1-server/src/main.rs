// game1-server -- the server of Spiel 1 (working name; docs/GAME1.md): rooms by code, free for all, server-side bots,
// the clients over UDP (desktop) and WebSocket (browser and desktop); one thread, a static Linux program without libc,
// also Windows.  The game's logic is game1_core (apps/game1/crates/game1_core), the same the client predicts with.
//
//   game1-server [--bind ADDR] [--port P] [--bots N] [--web DIR] [--secs S]
//       --bind      the address to listen on (127.0.0.1 unless set: only this computer; 0.0.0.0 for a LAN, which
//                   the system's firewall may ask about)
//       --port      UDP and TCP port (27960)
//       --bots      bots in every room while it has space (they make room for players; 0 unless set)
//       --web DIR   also serve the files of the browser build (build/web/game1) on the same port: a browser opens
//                   http://ADDR:PORT/ and plays over the WebSocket /game1 of the page's own server
//       --secs S    stop after S seconds (tests)
//   game1-server --check     the self-check of tests/check.uis: a room with 16 bots for a minute of ticks, then the
//                            server on 127.0.0.1 with a client over UDP that joins a room and gets snapshots
//   game1-server --version | --help
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use game::net::client::{ClientOptions, NetClient, Phase};
use game::net::input::NetInput;
use game::net::link::NetLink;
use game::net::server::RoomSim;
use game1_core::host::{Host, HostOptions};
use game1_core::movement::Mover;
use game1_core::rules::{schema, DEFAULT_PORT, GAME_NAME, PROTOCOL, RATE, WS_PATH};
use game1_core::sim::{Match, MatchConfig};
use netchan::{ClientConfig, Endpoint, ServerConfig};
use netio::{Request, Response, ServerConfig as HttpConfig};
use plat::net::{IpAddr, SocketAddr};

/// the app's own version: version= of apps/game1-server/PACKAGE (the library's is plat::VERSION)
const APP_VERSION: &str = rt::app_version!();

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

fn usage() -> i32 {
    out("Aufruf: game1-server [--bind ADRESSE] [--port P] [--bots N] [--web ORDNER] [--secs S] | --check | --version | --help\n");
    0
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// a file of `dir` for a request path ("/" -> index.html); None for anything outside it
fn file(dir: &str, path: &str) -> Option<(Vec<u8>, &'static str)> {
    let rel = path.split(['?', '#']).next().unwrap_or("").trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.split('/').any(|s| s.is_empty() || s == ".." || s == "." || s.contains('\\') || s.contains(':')) {
        return None;
    }
    let full = format!("{}/{}", dir.trim_end_matches(['/', '\\']), rel);
    plat::read_file(&full).map(|b| (b, content_type(rel)))
}

/// the netchan server: UDP on addr, the WebSocket on the same port (with the web files when dir is set) -> (server,
/// the HTTP server that holds the WebSocket when it serves files, the UDP address, the WebSocket's port)
fn bind(addr: SocketAddr, web: Option<&str>) -> Result<(netchan::Server, Option<netio::Server>, SocketAddr, u16), String> {
    let mut chan = netchan::Server::new(ServerConfig { protocol: PROTOCOL, ..ServerConfig::default() });
    match web {
        Some(dir) => {
            let ep = chan.ws_endpoint();
            let dir = String::from(dir);
            let handler: netio::server::Handler = Arc::new(move |req: &Request| {
                if req.path() == WS_PATH {
                    return ep.accept(req);
                }
                match file(&dir, req.path()) {
                    Some((b, t)) => Response::bytes(200, t, b).header("Cache-Control", "no-store"),
                    None => Response::new(404),
                }
            });
            let http = netio::Server::start(&addr, HttpConfig { threads: 4, read_ms: 10_000, ..HttpConfig::default() }, handler).map_err(|e| format!("HTTP auf {}: {}", addr, e))?;
            let port = http.local_addr().port;
            let udp = chan.bind_udp(&SocketAddr::new(addr.ip, port)).map_err(|e| format!("UDP auf Port {}: {}", port, e))?;
            Ok((chan, Some(http), udp, port))
        }
        None => {
            let udp = chan.bind_udp(&addr).map_err(|e| format!("UDP auf {}: {}", addr, e))?;
            let ws = chan.listen_ws(&SocketAddr::new(addr.ip, udp.port), WS_PATH).map_err(|e| format!("WebSocket auf Port {}: {}", udp.port, e))?;
            Ok((chan, None, udp, ws.port))
        }
    }
}

/// the server's loop -> exit code
fn serve(addr: SocketAddr, bots: usize, web: Option<&str>, secs: Option<u64>) -> i32 {
    let (chan, http, udp, ws_port) = match bind(addr, web) {
        Ok(x) => x,
        Err(e) => {
            out(&format!("game1-server: {}\n", e));
            return 1;
        }
    };
    let map = game1_core::map::werft();
    let opts = HostOptions { room: MatchConfig { bots, seed: plat::random_u64(), ..MatchConfig::default() }, ..HostOptions::default() };
    let mut host = Host::new(chan, ws_port, opts, map, plat::random_u64());
    out(&format!("game1-server {}: {} auf UDP {} und ws://{}:{}{}{}\n", APP_VERSION, GAME_NAME, udp, udp.ip, ws_port, WS_PATH, if web.is_some() { format!(", die Seite auf http://{}:{}/", udp.ip, ws_port) } else { String::new() }));
    let t0 = plat::time_us();
    let mut last_report = t0;
    loop {
        let now = plat::time_us();
        host.round(now * 1000);
        if now - last_report > 30_000_000 {
            last_report = now;
            let rooms = host.game.rooms();
            let players: usize = rooms.iter().map(|r| host.game.players(r).len()).sum();
            out(&format!("game1-server: {} Räume, {} Spieler\n", rooms.len(), players));
        }
        if secs.is_some_and(|s| now - t0 > s * 1_000_000) {
            break;
        }
        // (a sleep would cost a timer tick on Windows: the channel's wait wakes on a packet)
        host.wait(1000 / RATE / 2);
    }
    if let Some(h) = http {
        h.stop();
    }
    0
}

/// the self-check (see the module header) -> exit code
fn check() -> i32 {
    let mut failed = 0;
    let mut ok = |cond: bool, what: &str| {
        if !cond {
            failed += 1;
            out(&format!("FAIL {}\n", what));
        }
    };
    // a room with 16 bots for a minute of ticks
    let map = game1_core::map::werft();
    let mut m = Match::new(MatchConfig { bots: 16, seed: 5, ..MatchConfig::default() }, map.clone());
    for t in 1..=3600 {
        m.step(t, 1.0 / RATE as f64, &[]);
    }
    let st = m.stats();
    out(&format!("game1-server: 16 bots, 60 s: {} shots, {} hits, {} kills\n", st.shots, st.hits, st.kills));
    ok(m.figures().len() == 16 && st.kills > 0, "the bots' match");
    // the server on 127.0.0.1 with one client over UDP
    let lo = SocketAddr::new(IpAddr::LOCALHOST_V4, 0);
    match bind(lo, None) {
        Ok((chan, _, udp, ws)) => {
            let opts = HostOptions { room: MatchConfig { bots: 3, ..MatchConfig::default() }, ..HostOptions::default() };
            let mut host = Host::new(chan, ws, opts, map.clone(), 99);
            let nc = NetClient::new(ClientOptions { name: String::from("Prüfer"), ..ClientOptions::default() }, Mover::default(), schema(), Match::empty_baseline());
            match NetLink::connect(Endpoint::Udp(udp), ClientConfig { protocol: PROTOCOL, ..ClientConfig::default() }, nc, WS_PATH) {
                Ok(mut link) => {
                    let t0 = plat::time_us();
                    while plat::time_us() - t0 < 2_000_000 {
                        let now = plat::time_us() * 1000;
                        host.round(now);
                        link.pump(&map.world, now);
                        for _ in 0..link.game.advance(now) {
                            link.game.step(&map.world, NetInput::default());
                        }
                        link.flush();
                        host.wait(1);
                    }
                    let figs = link.game.newest().map_or(0, |s| s.ents.iter().filter(|e| e.state.kind == game1_core::rules::KIND_FIG).count());
                    out(&format!("game1-server: a client over UDP: {:?}, {} snapshots, {} figures seen\n", link.game.phase(), link.game.stats().snapshots, figs));
                    ok(link.game.phase() == Phase::Playing && link.game.stats().snapshots > 30 && figs == 4, "the client over UDP");
                }
                Err(e) => ok(false, &format!("connect: {}", e)),
            }
        }
        Err(e) => ok(false, &e),
    }
    out(&format!("game1-server: {}\n", if failed == 0 { "bestanden" } else { "FEHLGESCHLAGEN" }));
    if failed == 0 { 0 } else { 1 }
}

fn run() -> i32 {
    let args: Vec<String> = plat::args().into_iter().skip(1).collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return usage();
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        out(&format!("game1-server {} (JFLib {})\n", APP_VERSION, plat::VERSION));
        return 0;
    }
    if args.iter().any(|a| a == "--check") {
        return check();
    }
    let ip = match value("--bind") {
        Some(b) => match IpAddr::parse(&b) {
            Some(ip) => ip,
            None => {
                out(&format!("game1-server: {} ist keine Adresse\n", b));
                return 2;
            }
        },
        None => IpAddr::LOCALHOST_V4,
    };
    let port = value("--port").and_then(|p| p.parse::<u16>().ok()).unwrap_or(DEFAULT_PORT);
    let bots = value("--bots").and_then(|b| b.parse::<usize>().ok()).unwrap_or(0).min(game1_core::rules::MAX_FIGURES);
    let secs = value("--secs").and_then(|s| s.parse::<u64>().ok());
    let web = value("--web");
    serve(SocketAddr::new(ip, port), bots, web.as_deref(), secs)
}

rt::main!(run());
