// client -- game 1's headless clients against a server (`game1 --client SERVER`): clients over UDP or the WebSocket,
// played by the bots' minds from what each client sees (t_net::Drivers), for some seconds; then what the network did
// to them: the round trip (netchan's), the jitter of the snapshots' arrival, the loss both ways, the corrections a
// minute, the clock's margin, the reliable events (kills, hits).  Also in the browser: the page's own server over
// the WebSocket (tests/browser.mjs starts it with these arguments)
//
//   game1 --client SERVER [--clients N] [--secs S] [--ws] [--room CODE] [--fire] [--leave] [--margin T] [--interp MS]
//       SERVER     host:port (UDP; --ws: ws://host:port/game1), ws://..., or /game1 (the page's own server)
//       --clients  how many (2); the first makes a room (or joins --room), the others join it 150 ms apart
//       --fire     the minds shoot (corrections then also count deaths and returns); without it they only walk
//       --leave    at the end every client leaves with LEAVE (the server lets the figures go at once)
//       --margin   the input margin the clock aims for (ticks; the server's 2 unless set), --interp the buffer of the
//                  others (ms; 100): the knobs of the tuning in NETCODE.md
//
// The lines: one per client ("game1 client K ..."), then "game1 clients: ..." with the means and the verdict
// ("bestanden": every client played, decoded every snapshot against a base it had, and without --fire predicted
// every tick after the first second exactly but for the corrections the network caused).
//
// Jitter: a snapshot of tick T leaves the server at T / 60 s; its transit is the arrival time minus that (plus an
// unknown offset).  Reported: the mean change of the transit from one snapshot to the next (RFC 3550's |D|) and the
// spread of the transit (p99 - p1).  Loss down: snapshot ticks that never came (over UDP); loss up: netchan's lost
// packets of the client.

#[cfg(target_arch = "wasm32")]
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use game::net::client::{ClientOptions, NetClient, Phase};
use game::net::link::NetLink;
use game::net::RoomCode;
use game1_core::map::Map;
use game1_core::movement::Mover;
use game1_core::rules::{schema, GameEvent, PROTOCOL, RATE, WS_PATH};
use game1_core::sim::Match;
use netchan::{ClientConfig, Endpoint};

use crate::out;
use crate::t_net::Drivers;

/// what a client measured
#[derive(Default)]
struct Probe {
    /// (arrival in us, tick) of every newer snapshot
    arrivals: Vec<(u64, u32)>,
    last_snaps: u64,
    last_tick: Option<u32>,
    /// the first snapshot's tick and the snapshots decoded before it
    first: Option<(u32, u64)>,
    rtt: Vec<f64>,
    next_rtt_us: u64,
    joined_us: Option<u64>,
    started_us: u64,
    /// predictor mismatches when the first second after joining ended
    mism0: Option<u64>,
    kills: u64,
    hits: u64,
    left: bool,
    /// rounds after joining, and of those the ones whose render time of the others had reached the newest snapshot
    /// (the interpolation buffer ran dry: the others hold)
    rounds: u64,
    dry: u64,
}

/// a run of headless clients (see the module header)
pub struct Run {
    links: Vec<Option<NetLink<Mover>>>,
    probes: Vec<Probe>,
    d: Drivers,
    map: Arc<Map>,
    ep: Endpoint,
    room: Option<RoomCode>,
    clients: usize,
    secs: u64,
    fire: bool,
    leave: bool,
    margin: Option<f64>,
    interp_ms: u32,
    started: usize,
    next_at: u64,
    end_at: u64,
    done: bool,
    /// the exit code once done
    pub code: i32,
}

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1))
}

fn pct(v: &mut [f64], p: usize) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    v[(v.len() * p / 100).min(v.len() - 1)]
}

impl Run {
    /// from the command line (`--client SERVER ...`)
    pub fn new(args: &[String]) -> Result<Run, String> {
        let server = arg(args, "--client").cloned().unwrap_or_default();
        let ws = args.iter().any(|a| a == "--ws");
        let ep = if server.starts_with("ws://") || server.starts_with("wss://") || server.starts_with('/') {
            Endpoint::WebSocket(server.clone())
        } else if ws {
            Endpoint::WebSocket(format!("ws://{}{}", server, WS_PATH))
        } else {
            crate::app::endpoint(&server)?
        };
        let num = |name: &str, d: u64| arg(args, name).and_then(|v| v.parse::<u64>().ok()).unwrap_or(d);
        let clients = num("--clients", 2).clamp(1, 64) as usize;
        let room = match arg(args, "--room") {
            Some(r) => Some(RoomCode::parse(r).ok_or_else(|| format!("{} ist kein Raumcode", r))?),
            None => None,
        };
        let fire = args.iter().any(|a| a == "--fire");
        let t0 = plat::time_us();
        let secs = num("--secs", 20);
        Ok(Run {
            links: (0..clients).map(|_| None).collect(),
            probes: (0..clients).map(|_| Probe::default()).collect(),
            d: Drivers::new(clients, t0 ^ 0x51, fire),
            map: game1_core::map::werft(),
            ep,
            room,
            clients,
            secs,
            fire,
            leave: args.iter().any(|a| a == "--leave"),
            margin: arg(args, "--margin").and_then(|v| v.parse::<f64>().ok()),
            interp_ms: num("--interp", 100) as u32,
            started: 0,
            next_at: t0,
            // (the run ends `secs` after the last client joined, at the latest 30 s later)
            end_at: t0 + (secs + 30) * 1_000_000,
            done: false,
            code: 0,
        })
    }

    fn start_client(&mut self, k: usize) {
        self.probes[k].started_us = plat::time_us();
        let opts = ClientOptions { name: format!("Messung {}", k + 1), room: self.room, margin: self.margin, interp_ms: self.interp_ms, ..ClientOptions::default() };
        let nc = NetClient::new(opts, Mover::default(), schema(), Match::empty_baseline());
        let cfg = ClientConfig { protocol: PROTOCOL, ..ClientConfig::default() };
        match NetLink::connect(self.ep.clone(), cfg, nc, WS_PATH) {
            Ok(l) => self.links[k] = Some(l),
            Err(e) => out(&format!("game1 client {}: keine Verbindung: {}\n", k, e)),
        }
    }

    /// one round: start clients, pump, tick, measure -> false when the run is over (the report is out)
    pub fn step(&mut self) -> bool {
        if self.done {
            return false;
        }
        let now_us = plat::time_us();
        let now = now_us * 1000;
        if self.started < self.clients && now_us >= self.next_at && (self.started == 0 || self.room.is_some()) {
            let k = self.started;
            self.start_client(k);
            self.started += 1;
            self.next_at = now_us + 150_000;
        }
        for k in 0..self.clients {
            let Some(l) = self.links[k].as_mut() else { continue };
            l.pump(&self.map.world, now);
            if self.room.is_none() && l.game.phase() == Phase::Playing {
                self.room = l.game.welcome().map(|w| w.room);
            }
            for _ in 0..l.game.advance(now) {
                let i = self.d.input(&self.map, k, &l.game);
                l.game.step(&self.map.world, i);
            }
            l.flush();
            let p = &mut self.probes[k];
            if p.joined_us.is_none() && l.game.phase() == Phase::Playing {
                p.joined_us = Some(now_us);
            }
            if p.mism0.is_none() && p.joined_us.is_some_and(|j| now_us > j + 1_000_000) {
                p.mism0 = Some(l.game.predictor().stats().mismatches);
            }
            let snaps = l.game.stats().snapshots;
            if snaps != p.last_snaps {
                if let Some(t) = l.game.newest().map(|s| s.tick) {
                    match p.last_tick {
                        Some(lt) if (t.wrapping_sub(lt) as i32) > 0 => {
                            p.arrivals.push((now_us, t));
                            p.last_tick = Some(t);
                        }
                        None => {
                            // (the loss counts from here: the snapshots decoded before this one do not)
                            p.first = Some((t, p.last_snaps));
                            p.arrivals.push((now_us, t));
                            p.last_tick = Some(t);
                        }
                        _ => {}
                    }
                }
                p.last_snaps = snaps;
            }
            if p.mism0.is_some() {
                if let (Some((rt, frac)), Some(n)) = (l.game.render_time(), l.game.newest().map(|s| s.tick)) {
                    p.rounds += 1;
                    if (rt.wrapping_sub(n) as i32) as f64 + frac >= 0.0 {
                        p.dry += 1;
                    }
                }
            }
            if now_us >= p.next_rtt_us && l.game.phase() == Phase::Playing {
                p.next_rtt_us = now_us + 100_000;
                let r = l.channel().rtt_ms() as f64;
                if r > 0.0 {
                    p.rtt.push(r);
                }
            }
            while let Some(e) = l.game.take_event() {
                match GameEvent::decode(e.channel, &e.data) {
                    Some(GameEvent::Kill { .. }) => p.kills += 1,
                    Some(GameEvent::Hit { .. }) => p.hits += 1,
                    _ => {}
                }
            }
        }
        let all_in = self.started == self.clients && self.probes.iter().all(|p| p.joined_us.is_some());
        let last_join = self.probes.iter().filter_map(|p| p.joined_us).max().unwrap_or(now_us);
        if now_us >= self.end_at || (all_in && now_us >= last_join + (self.secs + 1) * 1_000_000) {
            self.report();
            if self.leave {
                for (k, l) in self.links.iter_mut().enumerate() {
                    if let Some(l) = l.as_mut() {
                        l.leave();
                        self.probes[k].left = true;
                    }
                }
            }
            self.done = true;
            return false;
        }
        true
    }

    fn report(&mut self) {
        let (mut ok, mut lines) = (true, 0);
        let mut all_rtt = Vec::new();
        let (mut sum_jit, mut sum_loss_down, mut sum_loss_up, mut sum_corr, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for k in 0..self.clients {
            let Some(l) = self.links[k].as_ref() else {
                ok = false;
                continue;
            };
            let p = &mut self.probes[k];
            let g = &l.game;
            let st = g.stats();
            let cs = l.channel().stats();
            let ps = g.predictor().stats();
            // jitter: transit = arrival - tick / rate (the offset cancels)
            let tick_us = 1_000_000.0 / RATE as f64;
            let base = p.arrivals.first().map_or(0, |a| a.1);
            let transit: Vec<f64> = p.arrivals.iter().map(|(at, t)| *at as f64 - t.wrapping_sub(base) as f64 * tick_us).collect();
            let d: Vec<f64> = transit.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
            let jit = if d.is_empty() { 0.0 } else { d.iter().sum::<f64>() / d.len() as f64 / 1000.0 };
            let mut tr = transit.clone();
            let spread = (pct(&mut tr, 99) - pct(&mut tr, 1)) / 1000.0;
            let got = p.arrivals.len() as f64;
            // loss down: the snapshot ticks from the first to the newest against the snapshots decoded (a late one
            // that came after a newer counts as come)
            let every = g.welcome().map_or(2, |w| w.snapshot_every.max(1) as u32);
            let loss_down = match (p.first, p.last_tick) {
                (Some((t0, s0)), Some(t1)) => {
                    let expected = (t1.wrapping_sub(t0) / every) as f64 + 1.0;
                    let came = (st.snapshots - s0) as f64;
                    ((expected - came) / expected * 100.0).max(0.0)
                }
                _ => 0.0,
            };
            let loss_up = if cs.sent > 0 { cs.lost as f64 / cs.sent as f64 * 100.0 } else { 0.0 };
            let played_s = p.joined_us.map_or(0.0, |j| (plat::time_us().saturating_sub(j + 1_000_000)) as f64 / 1e6).max(0.001);
            let corr = ps.mismatches - p.mism0.unwrap_or(ps.mismatches);
            let per_min = corr as f64 / played_s * 60.0;
            let errs = &ps.errors;
            let err_mean = if errs.is_empty() { 0.0 } else { errs.iter().map(|e| *e as f64).sum::<f64>() / errs.len() as f64 };
            let err_max = errs.iter().fold(0.0f32, |a, b| a.max(*b));
            let mut rtt = p.rtt.clone();
            all_rtt.extend_from_slice(&p.rtt);
            let rtt_mean = if rtt.is_empty() { 0.0 } else { rtt.iter().sum::<f64>() / rtt.len() as f64 };
            // (without shooting a correction is the network's: bounded as in the typical weather of the tests)
            let good = g.phase() == Phase::Playing && st.bad == 0 && st.no_base == 0 && got >= (self.secs as f64 * 10.0) && (self.fire || per_min < 60.0);
            ok &= good;
            out(&format!(
                "game1 client {} ({}) {:?}: rtt mean {:.1} p50 {:.1} p99 {:.1} ms (var {:.1}), jitter {:.2} ms (spread {:.1} ms), loss down {:.2} % up {:.2} %, joined after {} ms, {} snapshots ({} bad, {} no base, {} stale), predictions compared {}, corrections {} ({:.1}/min, error mean {:.3} max {:.3} m, smoothed {} snapped {}), margin {:.2} ticks, clock {} ppm, {} jumps, buffer dry {:.2} %, kills {} hits {}{}\n",
                k,
                if l.channel().reliable() { "WebSocket" } else { "UDP" },
                (g.phase(), l.channel().state()),
                rtt_mean,
                pct(&mut rtt, 50),
                pct(&mut rtt, 99),
                cs.rtt_var_ms,
                jit,
                spread,
                loss_down,
                loss_up,
                p.joined_us.map_or(-1, |j| (j - p.started_us) as i64 / 1000),
                st.snapshots,
                st.bad,
                st.no_base,
                st.stale,
                ps.checks,
                corr,
                per_min,
                err_mean,
                err_max,
                g.smoother().corrections,
                g.smoother().snaps,
                g.clock().margin().unwrap_or(0.0),
                g.clock().speed_ppm() as i64 - 1_000_000,
                g.clock().jumps(),
                p.dry as f64 * 100.0 / p.rounds.max(1) as f64,
                p.kills,
                p.hits,
                if good { "" } else { "  FAIL" }
            ));
            lines += 1;
            sum_jit += jit;
            sum_loss_down += loss_down;
            sum_loss_up += loss_up;
            sum_corr += per_min;
            n += 1.0;
        }
        let n = if n > 0.0 { n } else { 1.0 };
        let rtt_mean = if all_rtt.is_empty() { 0.0 } else { all_rtt.iter().sum::<f64>() / all_rtt.len() as f64 };
        out(&format!(
            "game1 clients: {} of {}, {} s{}: rtt mean {:.1} p99 {:.1} ms, jitter {:.2} ms, loss down {:.2} % up {:.2} %, corrections {:.1}/min per client: {}\n",
            lines,
            self.clients,
            self.secs,
            if self.fire { ", shooting" } else { "" },
            rtt_mean,
            pct(&mut all_rtt, 99),
            sum_jit / n,
            sum_loss_down / n,
            sum_loss_up / n,
            sum_corr / n,
            if ok { "bestanden" } else { "FEHLGESCHLAGEN" }
        ));
        self.code = if ok { 0 } else { 1 };
    }
}

/// the run on the desktop: until it ends, then the LEAVEs go out -> exit code
#[cfg(not(target_arch = "wasm32"))]
pub fn run(args: &[String]) -> i32 {
    let mut r = match Run::new(args) {
        Ok(r) => r,
        Err(e) => {
            out(&format!("game1 --client: {}\n", e));
            return 2;
        }
    };
    out(&format!("game1 client: {} clients for {} s against {:?}\n", r.clients, r.secs, r.ep));
    while r.step() {
        // (Windows' Sleep(1) can take a timer tick of 15.6 ms: there the loop yields instead)
        if cfg!(windows) {
            let until = plat::time_us() + 500;
            while plat::time_us() < until {
                plat::thread_yield();
            }
        } else {
            plat::sleep_ms(1);
        }
    }
    // the LEAVEs and DISCONNECTs leave with the last flush; a moment for the sockets
    plat::sleep_ms(50);
    r.code
}

/// the run in the browser: the page's app without a window, driven by the host's ticks
#[cfg(target_arch = "wasm32")]
pub fn start(args: &[String]) -> i32 {
    match Run::new(args) {
        Ok(r) => {
            out(&format!("game1 client: {} clients for {} s against {:?}\n", r.clients, r.secs, r.ep));
            plat::web::install(Box::new(r));
            0
        }
        Err(e) => {
            out(&format!("game1 --client: {}\n", e));
            2
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl plat::web::WebApplication for Run {
    fn frame_delay(&self) -> u32 {
        if self.done { 1000 } else { 1 }
    }

    fn tick(&mut self, _time_ms: u32) {
        self.step();
    }

    fn control(&mut self, _line: &[u8]) -> Option<String> {
        None
    }
}

