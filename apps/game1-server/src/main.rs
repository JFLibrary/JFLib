// game1-server -- the server of Spiel 1 (working name; docs/GAME1.md): rooms by code, free for all, server-side bots,
// the clients over UDP (desktop) and WebSocket (browser and desktop); one loop, the rooms of a tick side by side on the
// worker pool (one room per core; `--threads N`), a static Linux program without libc, also Windows.  Room codes and
// tickets from the system's random numbers.  The game's logic is game1_core (apps/game1/crates/game1_core), the same the client predicts with.
//
//   game1-server [--bind ADDR] [--port P] [--bots N] [--web DIR] [--secs S] [--log]
//       --bind      the address to listen on (127.0.0.1 unless set: only this computer; 0.0.0.0 for a LAN, which
//                   the system's firewall may ask about)
//       --port      UDP and TCP port (27960)
//       --bots      bots in every room while it has space (they make room for players; 0 unless set)
//       --web DIR   also serve the files of the browser build (build/web/game1) on the same port: a browser opens
//                   http://ADDR:PORT/ and plays over the WebSocket /game1 of the page's own server
//       --secs S    stop after S seconds (tests)
//       --log       every connection that comes and goes (netchan's reason), and every 10 s and at the end each
//                   player's counts as the server sees them (snapshots, inputs that came late or not at all: the
//                   loss upwards, events) and the time of the loop's rounds with ticks (the measurements, NETCODE.md)
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
    out("Aufruf: game1-server [--bind ADRESSE] [--port P] [--bots N] [--web ORDNER] [--secs S] [--log] | --check | --version | --help\n");
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

/// what bind gives: the netchan server, the HTTP server that holds the WebSocket when it serves files, the UDP
/// address, the WebSocket's port
type Bound = (netchan::Server, Option<netio::Server>, SocketAddr, u16);

/// the netchan server: UDP on addr, the WebSocket on the same port (with the web files when dir is set).  Port 0
/// takes a free port of one protocol and asks the other for the same number; Windows excludes ranges of ports for
/// one protocol only (os error 10013) and hands out free ports in order, so the next free one would likely fail
/// too: after a failure a few random ports of the dynamic range (49152..65535) instead
fn bind(addr: SocketAddr, web: Option<&str>) -> Result<Bound, String> {
    let mut r = bind_on(addr, web);
    if addr.port != 0 {
        return r;
    }
    for _ in 0..16 {
        if r.is_ok() {
            break;
        }
        let port = 49152 + (plat::random_u64() % 16384) as u16;
        r = bind_on(SocketAddr::new(addr.ip, port), web);
    }
    r
}

/// bind on one address (its port 0: a free one for the first protocol)
fn bind_on(addr: SocketAddr, web: Option<&str>) -> Result<Bound, String> {
    let mut chan = netchan::Server::new(ServerConfig { protocol: PROTOCOL, ..ServerConfig::default() });
    match web {
        Some(dir) => {
            let ep = chan.ws_endpoint();
            let dir = String::from(dir);
            // (a browser keeps up to six connections to the page's server open after loading it; with few workers and
            // a long keep-alive the WebSocket's upgrade waited for one of them: Firefox joined after 5.7 s)
            let handler: netio::server::Handler = Arc::new(move |req: &Request| {
                if req.path() == WS_PATH {
                    return ep.accept(req);
                }
                match file(&dir, req.path()) {
                    Some((b, t)) => Response::bytes(200, t, b).header("Cache-Control", "no-store"),
                    None => Response::new(404),
                }
            });
            let http = netio::Server::start(&addr, HttpConfig { threads: 16, idle_ms: 2_000, read_ms: 10_000, ..HttpConfig::default() }, handler).map_err(|e| format!("HTTP auf {}: {}", addr, e))?;
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
fn serve(addr: SocketAddr, bots: usize, web: Option<&str>, secs: Option<u64>, log: bool) -> i32 {
    let (chan, http, udp, ws_port) = match bind(addr, web) {
        Ok(x) => x,
        Err(e) => {
            out(&format!("game1-server: {}\n", e));
            return 1;
        }
    };
    let map = game1_core::map::werft();
    let opts = HostOptions { room: MatchConfig { bots, seed: plat::random_u64(), ..MatchConfig::default() }, ..HostOptions::default() };
    let mut host = Host::new(chan, ws_port, opts, map);
    host.log = log;
    out(&format!("game1-server {}: {} auf UDP {} und ws://{}:{}{}{}\n", APP_VERSION, GAME_NAME, udp, udp.ip, ws_port, WS_PATH, if web.is_some() { format!(", die Seite auf http://{}:{}/", udp.ip, ws_port) } else { String::new() }));
    let t0 = plat::time_us();
    let mut last_report = t0;
    let mut last_log = t0;
    let mut rounds: Vec<u32> = Vec::new();
    loop {
        let now = plat::time_us();
        let n = host.round(now * 1000);
        if log && n > 0 {
            rounds.push((plat::time_us() - now).min(u32::MAX as u64) as u32);
        }
        if log && now - last_log > 10_000_000 {
            last_log = now;
            report(&host, &mut rounds);
        }
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
    if log {
        report(&host, &mut rounds);
    }
    if let Some(h) = http {
        h.stop();
    }
    0
}

/// --log: each player's counts and the loop's round times since the last report
fn report(host: &Host, rounds: &mut Vec<u32>) {
    let st = host.game.stats();
    rounds.sort_unstable();
    let p = |q: usize| rounds.get((rounds.len() * q / 100).min(rounds.len().saturating_sub(1))).copied().unwrap_or(0);
    out(&format!(
        "game1-server: {} ticks, {} joins, {} left, {} snapshots ({} bytes, max {}), events {} out {} in; rounds with ticks: {} p50 {} p99 {} max {} us\n",
        st.ticks,
        st.joins,
        st.left,
        st.snapshots,
        st.snapshot_bytes,
        st.max_snapshot,
        st.events_out,
        st.events_in,
        rounds.len(),
        p(50),
        p(99),
        rounds.last().copied().unwrap_or(0)
    ));
    rounds.clear();
    for code in host.game.rooms() {
        for (conn, id, name) in host.game.players(&code) {
            if let Some(l) = host.game.link_stats(conn) {
                let i = l.inputs;
                out(&format!(
                    "game1-server:   {} {} ({}): {} snapshots, inputs {} taken, {} missed, {} late, {} repeated, events {} sent {} resent {} acknowledged, {} in\n",
                    code, id, name, l.snapshots, i.received, i.missed, i.late, i.repeated, l.events.sent, l.events.resent, l.events.done, l.events_in.delivered
                ));
            }
        }
    }
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
            let mut host = Host::new(chan, ws, opts, map.clone());
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
    let log = args.iter().any(|a| a == "--log");
    serve(SocketAddr::new(ip, port), bots, web.as_deref(), secs, log)
}

rt::main!(run());
