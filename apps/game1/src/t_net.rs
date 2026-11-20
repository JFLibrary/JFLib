// t_net -- the groups "netsim" and "lagcomp": game 1's clients and server over game::net's simulated network
// (local.rs), the clients driven by the bots' minds from what each client sees (its predicted figure, the others
// interpolated).  The reference of a prediction is the server itself: its state of every figure in every tick.
//
//   netsim    a clean link: after joining, every first prediction of every client equals the server's state, not one
//             correction; typical weather (30 ms +- 5, 1 % loss) for 10 s, then clear: after 2 s every prediction exact
//             again, the corrections in the bad weather bounded; a match with shooting (four bots on the server, four
//             clients, one over WebSocket): everybody plays, every snapshot decodes against a base it has, none over
//             1200 bytes, figures are ended and come back
//   lagcomp   a client shoots at another that strafes at full speed, aiming at where it sees it, at round trips of
//             50 to 250 ms: with the lag compensation (300 ms) nearly every shot hits, without it most miss
//   session_sum   the checksum of a recorded session for --sums

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::vec::Vec;

use game::net::client::{lerp_field, NetClient, Remote};
use game::net::input::NetInput;
use game::net::quant::{angle_q, pitch_q, POS_STEPS};
use game::net::server::ServerOptions;
use game::net::sim::Weather;
use game1_core::bot::{Brain, Seen, Skill};
use game1_core::map::{fnv, Map, FNV0};
use game1_core::movement::{MoveState, Mover, F_CROUCH, F_DEAD};
use game1_core::rules::*;
use game1_core::sim::{Event, MatchConfig};
use physics::Vec3;

use crate::local::Local;
use crate::Ctx;

/// the other figures as a client sees them (interpolated), for a bot's mind
pub fn seen_of(c: &NetClient<Mover>, mover: &Mover, rem: &mut Vec<Remote>, out: &mut Vec<Seen>) {
    c.remotes(rem);
    out.clear();
    for r in rem.iter() {
        if r.a.kind != KIND_FIG {
            continue;
        }
        let f = |i: usize| lerp_field(r.a.fields[i], r.b.fields[i], r.t) / POS_STEPS;
        let near = if r.t < 0.5 { &r.a } else { &r.b };
        let flags = near.fields[6] as u32;
        let feet = Vec3::new(f(0), f(1), f(2));
        let centre = feet + Vec3::new(0.0, 0.5 * mover.height(flags & F_CROUCH != 0), 0.0);
        let hittable = flags & F_DEAD == 0 && near.fields[F_INFO] & INFO_PROTECTED == 0;
        out.push(Seen { id: r.id, centre, hittable });
    }
}

/// the minds of a Local's clients (index = client)
pub struct Drivers {
    pub brains: Vec<Brain>,
    pub fire: bool,
    rem: Vec<Remote>,
    seen: Vec<Seen>,
    mover: Mover,
}

impl Drivers {
    pub fn new(n: usize, seed: u64, fire: bool) -> Drivers {
        Drivers { brains: (0..n).map(|k| Brain::new(seed.wrapping_mul(31).wrapping_add(k as u64 + 1), Skill::default(), 0.0, RATE)).collect(), fire, rem: Vec::new(), seen: Vec::new(), mover: Mover::default() }
    }

    /// the input of client k's tick from what it sees
    pub fn input(&mut self, map: &Map, k: usize, c: &NetClient<Mover>) -> NetInput {
        let Some(own) = c.predicted().filter(|_| c.predictor().known()) else { return NetInput::default() };
        seen_of(c, &self.mover, &mut self.rem, &mut self.seen);
        let mut i = self.brains[k].think(c.tick(), &own, &self.mover, map, &self.seen);
        if !self.fire {
            i.buttons &= !BTN_FIRE;
        }
        i
    }
}

/// every client's first prediction of each tick, from the steps of a millisecond
fn record_first(l: &Local, stepped: &[(usize, u32)], first: &mut BTreeMap<(usize, u32), MoveState>) {
    for &(k, t) in stepped {
        if let Some(s) = l.clients[k].client.predicted() {
            first.insert((k, t), s);
        }
    }
}

/// first predictions from `from_tick` on against the server's states -> (compared, different)
fn compare(l: &Local, first: &BTreeMap<(usize, u32), MoveState>, from: &BTreeMap<usize, u32>) -> (u64, u64) {
    let Some(log) = &l.tick_log else { return (0, 0) };
    let (mut n, mut bad) = (0, 0);
    for ((k, t), s) in first {
        let Some(f) = from.get(k) else { continue };
        if (t.wrapping_sub(*f) as i32) < 0 {
            continue;
        }
        let own = l.clients[*k].client.own_id();
        if let Some(srv) = log.get(&(own, *t)) {
            n += 1;
            if srv != s {
                bad += 1;
            }
        }
    }
    (n, bad)
}

fn mismatches(l: &Local) -> Vec<u64> {
    l.clients.iter().map(|c| c.client.predictor().stats().mismatches).collect()
}

fn ticks_now(l: &Local) -> BTreeMap<usize, u32> {
    l.clients.iter().enumerate().map(|(k, c)| (k, c.client.tick())).collect()
}

/// a Local with `players` bot-driven clients (every fourth over WebSocket) and `bots` bots on the server
fn session(map: &alloc::sync::Arc<Map>, seed: u64, players: usize, bots: usize, w: Weather) -> Local {
    let mut l = Local::new(seed, ServerOptions::default(), MatchConfig { bots, seed, ..MatchConfig::default() }, map.clone());
    l.tick_log = Some(BTreeMap::new());
    for k in 0..players {
        l.add_client(&format!("Test {}", k + 1), k % 4 == 3, w, k as u64 * 100);
    }
    l
}

fn run_for(l: &mut Local, d: &mut Drivers, map: &Map, ms: u64, first: &mut BTreeMap<(usize, u32), MoveState>) {
    for _ in 0..ms {
        let st = l.step_ms(&mut |k, c| d.input(map, k, c));
        record_first(l, &st, first);
    }
}

pub fn netsim(c: &mut Ctx) {
    let map = game1_core::map::werft();
    // a clean link: not one correction after joining
    let mut l = session(&map, 21, 4, 0, Weather::CLEAR);
    let mut d = Drivers::new(4, 21, false);
    let mut first = BTreeMap::new();
    run_for(&mut l, &mut d, &map, 2500, &mut first);
    c.check(l.playing() == 4, || format!("{} of 4 clients play: {}", l.playing(), l.describe(0)));
    let m0 = mismatches(&l);
    let from = ticks_now(&l);
    first.clear();
    run_for(&mut l, &mut d, &map, c.n(6000).max(2000) as u64, &mut first);
    let (n, bad) = compare(&l, &first, &from);
    let m1 = mismatches(&l);
    let corrected: u64 = m1.iter().zip(&m0).map(|(a, b)| a - b).sum();
    c.check(n > 400 && bad == 0 && corrected == 0, || format!("clear: {} predictions compared, {} different, {} corrections", n, bad, corrected));
    c.note(&format!("clear: {} first predictions compared, {} different, {} corrections", n, bad, corrected));
    // typical weather, then clear: exact again after 2 s
    for k in 0..4 {
        l.set_weather(k, Weather::TYPICAL);
    }
    let m0 = mismatches(&l);
    let secs = if c.quick { 4 } else { 10 };
    run_for(&mut l, &mut d, &map, secs * 1000, &mut first);
    let m1 = mismatches(&l);
    let corrected: u64 = m1.iter().zip(&m0).map(|(a, b)| a - b).sum();
    let per_min = corrected as f64 / 4.0 / (secs as f64 / 60.0);
    c.check(per_min < 60.0, || format!("typical: {:.1} corrections a minute per client", per_min));
    let errs: Vec<f32> = l.clients.iter().flat_map(|c| c.client.predictor().stats().errors.iter().copied()).collect();
    let max_err = errs.iter().fold(0.0f32, |a, b| a.max(*b));
    c.note(&format!("typical {} s: {} corrections ({:.1} a minute per client), largest error {:.3} m", secs, corrected, per_min, max_err));
    for k in 0..4 {
        l.set_weather(k, Weather::CLEAR);
    }
    run_for(&mut l, &mut d, &map, 2000, &mut first);
    let from = ticks_now(&l);
    first.clear();
    run_for(&mut l, &mut d, &map, 3000, &mut first);
    let (n, bad) = compare(&l, &first, &from);
    c.check(n > 300 && bad == 0, || format!("after the weather: {} compared, {} different", n, bad));
    for k in 0..4 {
        c.check(l.clients[k].client.stats().no_base == 0 && l.clients[k].client.stats().bad == 0, || l.describe(k));
    }
    // shooting: four bots on the server, four clients
    let mut l = session(&map, 33, 4, 4, Weather::TYPICAL);
    let mut d = Drivers::new(4, 33, true);
    let mut first = BTreeMap::new();
    let secs = if c.quick { 6 } else { 15 };
    run_for(&mut l, &mut d, &map, secs * 1000, &mut first);
    c.check(l.playing() == 4, || format!("{} of 4 clients play", l.playing()));
    let st = l.room_match().map(|m| m.stats()).unwrap_or_default();
    c.check(st.kills > 0 && st.spawns > 8, || format!("no fighting in {} s: {:?}", secs, st));
    let ss = l.server.stats();
    c.check(ss.max_snapshot <= 1200, || format!("a snapshot of {} bytes", ss.max_snapshot));
    for k in 0..4 {
        let cs = l.clients[k].client.stats();
        c.check(cs.no_base == 0 && cs.bad == 0 && cs.snapshots > (secs as u64 * 20), || l.describe(k));
    }
    let corr: u64 = mismatches(&l).iter().sum();
    c.note(&format!(
        "shooting {} s, typical: {} shots, {} hits, {} kills; snapshots up to {} bytes; {} corrections over 4 clients (deaths and returns count)",
        secs, st.shots, st.hits, st.kills, ss.max_snapshot, corr
    ));
}

/// one lag compensation run: a shooter and a target that strafes, `rtt_ms` round trip -> (shots, hits, the mean view
/// lag in ms, shots clamped to the limit)
pub fn lag_run(map: &alloc::sync::Arc<Map>, rtt_ms: u64, compensate: bool, secs: u64) -> (u32, u32, f64, u64) {
    let back = if compensate { MAX_REWIND } else { 0 };
    let opts = ServerOptions { max_rewind_ms: back * 1000 / RATE, ..ServerOptions::default() };
    let mut l = Local::new(5, opts, MatchConfig { damage: false, max_rewind: back, ..MatchConfig::default() }, map.clone());
    let w = Weather { loss: 0.0, dup: 0.0, reorder: 0.0, extra_us: 0, delay_us: rtt_ms * 500, jitter_us: 0 };
    l.add_client("Schütze", false, w, 0);
    l.add_client("Ziel", false, w, 100);
    let mover = Mover::default();
    let mut tick_k = 0u32;
    let mut rem = Vec::new();
    let mut seen = Vec::new();
    let still = |_: usize, _: &NetClient<Mover>| NetInput::default();
    // join, then put both in place
    for _ in 0..2500 {
        l.step_ms(&mut { still });
    }
    let shooter = l.clients[0].client.own_id();
    let target = l.clients[1].client.own_id();
    if let Some(m) = l.room_match_mut() {
        m.set_record(true);
        for (id, at) in [(shooter, Vec3::new(-10.5, 0.0, 8.0)), (target, Vec3::new(-12.0, 0.0, 17.0))] {
            if let Some(f) = m.figures_mut().get_mut(&id) {
                f.state = MoveState::at(at);
                f.safe_until = 0;
            }
        }
    }
    for _ in 0..1500 {
        l.step_ms(&mut { still });
    }
    if let Some(m) = l.room_match_mut() {
        m.take_events();
    }
    let (mut shots, mut hits, mut view) = (0u32, 0u32, 0u64);
    let clamped0 = l.room_match().map_or(0, |m| m.stats().clamped);
    for _ in 0..secs * 1000 {
        let mut input = |k: usize, c: &NetClient<Mover>| -> NetInput {
            if k == 1 {
                // the target strafes left and right at full speed, 0.6 s each way
                tick_k += 1;
                let dir = if (tick_k / 36) % 2 == 0 { 127 } else { -127 };
                return NetInput { move_x: dir, yaw: angle_q(0.0), ..NetInput::default() };
            }
            let Some(own) = c.predicted() else { return NetInput::default() };
            seen_of(c, &mover, &mut rem, &mut seen);
            let Some(t) = seen.iter().find(|s| s.id == target) else { return NetInput::default() };
            let eye = mover.eye(&own);
            let d = t.centre - eye;
            let yaw = crate::look_yaw(d.x, d.z);
            let pitch = physics::math::atan2(d.y, Vec3::new(d.x, 0.0, d.z).length());
            NetInput { buttons: BTN_FIRE, yaw: angle_q(yaw), pitch: pitch_q(pitch), ..NetInput::default() }
        };
        l.step_ms(&mut input);
        if let Some(m) = l.room_match_mut() {
            for e in m.take_events() {
                if let Event::Shot { shooter: s, hit, view: v, .. } = e {
                    if s == shooter {
                        shots += 1;
                        view += v as u64;
                        if hit.is_some_and(|h| h.0 == target) {
                            hits += 1;
                        }
                    }
                }
            }
        }
    }
    let clamped = l.room_match().map_or(0, |m| m.stats().clamped) - clamped0;
    (shots, hits, view as f64 / shots.max(1) as f64 / 256.0 * 1000.0 / RATE as f64, clamped)
}

pub fn lagcomp(c: &mut Ctx) {
    let map = game1_core::map::werft();
    let secs = if c.quick { 3 } else { 6 };
    let mut line = alloc::string::String::new();
    for rtt in [50u64, 100, 150, 200, 250] {
        let (s, h, view, clamped) = lag_run(&map, rtt, true, secs);
        let rate = h as f64 / s.max(1) as f64;
        let (s2, h2, _, _) = lag_run(&map, rtt, false, secs);
        let rate2 = h2 as f64 / s2.max(1) as f64;
        c.check(rate2 < 0.6, || format!("{} ms round trip, not compensated: {} of {} shots hit", rtt, h2, s2));
        // (the view lags about the round trip and 150 ms behind the tick: the interpolation's 100 ms, the margin of 2
        // ticks, the snapshot's age; up to 200 ms it fits the limit of 300 ms, at 250 ms it is clamped and a strafing
        // target is mostly missed: the user's limit, docs/NETCODE.md)
        if view <= (MAX_REWIND * 1000 / RATE) as f64 {
            c.check(s > 20 && rate >= 0.9 && clamped == 0, || format!("{} ms round trip, compensated: {} of {} shots hit, view {:.0} ms, {} clamped", rtt, h, s, view, clamped));
        } else {
            c.check(s > 20 && clamped > 0 && rate >= rate2, || format!("{} ms round trip beyond the limit: {} of {} hit, {} clamped", rtt, h, s, clamped));
        }
        line += &format!(" {} ms: {:.0} % (view {:.0} ms{}; {:.0} % without);", rtt, rate * 100.0, view, if clamped > 0 { ", clamped" } else { "" }, rate2 * 100.0);
    }
    c.note(&format!("hits on a target strafing at 9 m/s:{}", line));
}

/// the checksum of a recorded session (--sums): four bots on the server and three clients (one over WebSocket) that
/// shoot, rough weather then clear; folded: every client's first prediction of every tick, the server's state of
/// every figure in every tick, the clients' counters at the end -> (checksum, values folded)
pub fn session_sum(map: &alloc::sync::Arc<Map>, quick: bool) -> (u64, u64) {
    let mut l = Local::new(77, ServerOptions::default(), MatchConfig { bots: 4, seed: 77, ..MatchConfig::default() }, map.clone());
    l.tick_log = Some(BTreeMap::new());
    for k in 0..3 {
        l.add_client(&format!("Summe {}", k + 1), k == 2, Weather::ROUGH, k as u64 * 120);
    }
    let mut d = Drivers::new(3, 77, true);
    let mut first = BTreeMap::new();
    let ms = if quick { 4000 } else { 8000 };
    run_for(&mut l, &mut d, map, ms, &mut first);
    for k in 0..3 {
        l.set_weather(k, Weather::CLEAR);
    }
    run_for(&mut l, &mut d, map, ms / 2, &mut first);
    let mut h = FNV0;
    let mut n = 0u64;
    for ((k, t), s) in &first {
        h = fnv(h, &(*k as u32).to_le_bytes());
        h = fnv(h, &t.to_le_bytes());
        h = crate::t_move::fold(h, s);
        n += 1;
    }
    if let Some(log) = &l.tick_log {
        for ((id, t), s) in log {
            h = fnv(h, &id.to_le_bytes());
            h = fnv(h, &t.to_le_bytes());
            h = crate::t_move::fold(h, s);
            n += 1;
        }
    }
    for c in &l.clients {
        let st = c.client.stats();
        for v in [st.snapshots, st.bytes_in, st.inputs_sent, st.bytes_out, c.client.predictor().stats().mismatches] {
            h = fnv(h, &v.to_le_bytes());
            n += 1;
        }
    }
    (h, n)
}
