// net_sim -- the netcode on the simulated link: sessions of bots under clear, typical, rough and stormy
// weather, the full state of a join (fetched over a WebSocket, or filled in by priority), the wrap of the
// tick numbers, players leaving and joining; the measurements with 16 bots (group bots, --netbench) and
// the checksum of a recorded session (--netsum)
//
// The reference for the prediction is the server itself: the harness keeps the server's state of every
// figure per tick and every client's first prediction per tick.  After the link heals (the weather turns
// clear and the clocks settle), every first prediction must be the server's state bit for bit, and no
// reconciliation may find a difference.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use game::net::arena::ArenaConfig;
use game::net::client::Phase;
use game::net::server::ServerOptions;
use game::net::sim::{Weather, Which};
use game::net::snapshot::tick_diff;
use netchan::endpoint::ServerConfig;

use crate::net_session::{stats_of, Session, SessionConfig, PROTOCOL};
use crate::Ctx;

pub fn run(c: &mut Ctx) {
    let q = c.quick;
    weather(c, "clear", 1, Weather::CLEAR, 8, if q { 2000 } else { 6000 }, 3000);
    weather(c, "typical", 2, Weather::TYPICAL, 8, if q { 3000 } else { 10_000 }, 3000);
    weather(c, "rough", 3, Weather::ROUGH, 6, if q { 3000 } else { 10_000 }, 3000);
    weather(c, "storm", 4, Weather::STORM, 4, if q { 3000 } else { 10_000 }, 3000);
    wrap(c);
    fetch(c, true);
    fetch(c, false);
    leave_join(c);
    if !q {
        seq_wrap(c);
    }
}

fn base_cfg(seed: u64, players: usize, w: Weather) -> SessionConfig {
    let mut cfg = SessionConfig::new(seed, players, w);
    cfg.chan = ServerConfig { protocol: PROTOCOL, max_per_ip: 64, ..ServerConfig::default() };
    cfg
}

/// run in `w` for bad_ms, then clear: after it heals, every first prediction is the server's state
fn weather(c: &mut Ctx, name: &str, seed: u64, w: Weather, players: usize, bad_ms: u64, clear_ms: u64) {
    let mut cfg = base_cfg(seed, players, w);
    cfg.arena.props = 40;
    cfg.arena.moving = 8;
    let mut s = Session::new(cfg);
    s.run(bad_ms);
    let bad_mism: u64 = s.clients.iter().map(|k| k.client.predictor().stats().mismatches).sum();
    s.set_weather(Weather::CLEAR);
    s.run(2000);
    let healed = s.server.tick();
    s.mark();
    s.run(clear_ms);
    let playing = s.playing();
    c.check(playing == players, || format!("{}: {} of {} playing; {}", name, playing, players, s.describe(0)));
    let (n, diff) = s.first_vs_server(Some(healed));
    c.check(n > (players as u64) * 100 && diff == 0, || format!("{}: after healing {} of {} first predictions differ from the server", name, diff, n));
    for k in 0..s.clients.len() {
        let cl = &s.clients[k];
        let st = cl.client.stats();
        let after = cl.client.predictor().stats().mismatches - cl.mark_mismatches;
        c.check(st.no_base == 0 && st.bad == 0 && after == 0, || format!("{}: {}; mismatches after healing {}", name, s.describe(k), after));
    }
    let max = s.server.stats().max_snapshot;
    c.check(max <= 1200, || format!("{}: a snapshot of {} bytes", name, max));
    let total: u64 = s.clients.iter().map(|k| k.client.predictor().stats().steps).sum();
    crate::out(&format!("  {:<8} {} players, {:>5.1} s: {} mismatches of {} predicted ticks in the bad weather, 0 after healing ({} compared)\n", name, players, bad_ms as f64 / 1000.0, bad_mism, total, n));
}

/// across the wrap of the u32 ticks
fn wrap(c: &mut Ctx) {
    let mut cfg = base_cfg(5, 3, Weather::ROUGH);
    cfg.server.start_tick = u32::MAX - 240;
    let mut s = Session::new(cfg);
    s.run(4000);
    s.set_weather(Weather::CLEAR);
    s.run(2000);
    let healed = s.server.tick();
    s.mark();
    s.run(2000);
    let crossed = tick_diff(s.server.tick(), u32::MAX - 240) > 240 && s.server.tick() < 1000;
    let (n, diff) = s.first_vs_server(Some(healed));
    let ok_clients = s.clients.iter().all(|k| k.client.phase() == Phase::Playing && k.client.stats().no_base == 0 && k.client.predictor().stats().mismatches == k.mark_mismatches);
    c.check(crossed && n > 100 && diff == 0 && ok_clients, || format!("tick wrap: crossed {} (tick {}), {} of {} differ; {}", crossed, s.server.tick(), diff, n, s.describe(0)));
}

/// the full state of a join with 300 props: fetched over a WebSocket (`fetch`) or filled in by priority
fn fetch(c: &mut Ctx, fetch: bool) {
    let name = if fetch { "fetch" } else { "fill" };
    let mut cfg = base_cfg(6, 4, Weather::TYPICAL);
    cfg.arena.props = 300;
    cfg.arena.moving = 30;
    cfg.arena.displaced = 150;
    cfg.join_after_ms = 1000;
    if !fetch {
        cfg.server.ws_port = 0;
    }
    let mut s = Session::new(cfg);
    // when is a client's state complete (its newest snapshot equal to the server's world of that tick)?
    let mut complete_at: Vec<Option<f64>> = alloc::vec![None; s.clients.len()];
    for _ in 0..60 {
        s.run(100);
        for (k, cl) in s.clients.iter().enumerate() {
            if complete_at[k].is_some() {
                continue;
            }
            if let (Some(n), Some(j)) = (cl.client.newest(), cl.joined_us) {
                if s.worlds.get(&n.tick) == Some(n) {
                    complete_at[k] = Some((s.net.now_us - j) as f64 / 1000.0);
                }
            }
        }
    }
    let fulls: Vec<u64> = s.clients.iter().map(|k| k.client.stats().fulls).collect();
    let udp_fulls_ok = s.clients.iter().all(|k| k.client.stats().fulls == if fetch && !k.ws { 1 } else { 0 });
    c.check(udp_fulls_ok, || format!("{}: FULLs per client {:?}", name, fulls));
    let all = complete_at.iter().all(|x| x.is_some());
    c.check(all, || format!("{}: complete states {:?}", name, complete_at));
    // and still at the end
    let still = s.clients.iter().all(|k| k.client.newest().is_some_and(|n| s.worlds.get(&n.tick) == Some(n)));
    c.check(still, || format!("{}: a client's state is not the server's at the end", name));
    let st = s.server.stats();
    c.check(st.max_snapshot <= 1200 || st.sizes.iter().filter(|&&x| x > 1200).count() <= 1, || format!("{}: max snapshot {}", name, st.max_snapshot));
    let times: Vec<String> = complete_at.iter().map(|x| format!("{:.0}", x.unwrap_or(-1.0))).collect();
    crate::out(&format!("  {:<8} 300 props (30 moving, 150 moved): complete after [{}] ms (UDP, UDP, UDP, WebSocket), partial snapshots {}, FULL {} bytes\n", name, times.join(", "), st.partial, if st.fulls > 0 { st.full_bytes / st.fulls } else { 0 }));
}

/// players leave and come: the others see the figure go, a new one takes the freed id with a new generation
fn leave_join(c: &mut Ctx) {
    let mut cfg = base_cfg(7, 4, Weather::TYPICAL);
    cfg.ws_every = 2;
    let mut s = Session::new(cfg);
    s.run(3000);
    let gone = s.clients[1].client.own_id();
    let peer = s.clients[1].peer;
    s.net.link_close(peer, Which::Main);
    s.run(1500);
    let seen_gone = [0usize, 2, 3].iter().all(|&k| s.clients[k].client.newest().is_some_and(|n| n.get(gone).is_none()));
    c.check(gone != 0 && seen_gone, || format!("leave: figure {} still seen", gone));
    let code = s.room;
    let figs = code.and_then(|r| s.server.room(&r)).map(|a| a.figures().len());
    c.check(figs == Some(3), || format!("leave: {:?} figures on the server", figs));
    // a new client into the same room: the freed id, generation 1
    let k = s.add_client(false);
    s.run(2000);
    let new_id = s.clients[k].client.own_id();
    let g = s.clients[0].client.newest().and_then(|n| n.get(new_id)).map(|e| e.generation);
    c.check(new_id == gone && g == Some(1) && s.clients[k].client.phase() == Phase::Playing, || format!("join: id {} (freed {}), generation {:?}", new_id, gone, g));
}

/// past the wrap of netchan's 16-bit sequence numbers: 70 000 snapshots to one client
fn seq_wrap(c: &mut Ctx) {
    let mut cfg = base_cfg(8, 1, Weather { loss: 0.05, ..Weather::CLEAR });
    cfg.server.snapshot_every = 1;
    let mut s = Session::new(cfg);
    s.run(10_000);
    let mut ms = 10_000;
    while s.server.stats().snapshots < 70_000 && ms < 1_500_000 {
        s.run(10_000);
        ms += 10_000;
    }
    s.set_weather(Weather::CLEAR);
    s.run(2000);
    let healed = s.server.tick();
    s.mark();
    s.run(3000);
    let (n, diff) = s.first_vs_server(Some(healed));
    let cl = &s.clients[0];
    let ok = cl.client.phase() == Phase::Playing && cl.client.stats().no_base == 0 && cl.client.predictor().stats().mismatches == cl.mark_mismatches;
    c.check(s.server.stats().snapshots >= 70_000 && n > 100 && diff == 0 && ok, || format!("sequence wrap: {} snapshots, {} of {} differ; {}", s.server.stats().snapshots, diff, n, s.describe(0)));
    crate::out(&format!("  seqwrap  {} snapshots ({:.0} s), then 0 of {} first predictions differ\n", s.server.stats().snapshots, ms as f64 / 1000.0, n));
}

/// the numbers of one session with bots
pub struct Measure {
    pub name: String,
    pub players: usize,
    pub secs: f64,
    pub mismatch_per_min: f64,
    pub err: (f64, f64, f64, f64),
    pub shift: (f64, f64, f64, f64),
    pub up_bps: f64,
    pub down_bps: f64,
    pub up_pps: f64,
    pub down_pps: f64,
    pub wire_up_bps: f64,
    pub wire_down_bps: f64,
    pub snap: (f64, f64, f64, f64),
    pub partial: f64,
    pub tick_us: (f64, f64, f64, f64),
    pub full_bytes: u32,
    pub shots: u64,
    pub hits: u64,
    pub clamped: u64,
    pub view_ms: (f64, f64),
    pub playing: usize,
    pub disconnected: usize,
}

/// a session of `players` bots in `w` for `secs`, measured after the joins (the first 3 s)
pub fn measure(name: &str, seed: u64, players: usize, w: Weather, props: usize, moving: usize, every: u32, secs: u64, fire: bool) -> Measure {
    measure_rewind(name, seed, players, w, props, moving, every, secs, fire, 200)
}

/// measure with another limit of the lag compensation (ms)
#[allow(clippy::too_many_arguments)]
pub fn measure_rewind(name: &str, seed: u64, players: usize, w: Weather, props: usize, moving: usize, every: u32, secs: u64, fire: bool, rewind_ms: u32) -> Measure {
    let displaced = (props / 2).min(props - moving);
    let mut cfg = base_cfg(seed, players, w);
    cfg.arena = ArenaConfig { props, moving, displaced, fire, max_rewind: rewind_ms * 60 / 1000, ..ArenaConfig::default() };
    cfg.fire = fire;
    cfg.server = ServerOptions { snapshot_every: every, ws_port: 7777, max_rewind_ms: rewind_ms, ..ServerOptions::default() };
    cfg.time_ticks = true;
    cfg.join_gap_ms = 20;
    let mut s = Session::new(cfg);
    s.run(3000);
    // from here on
    let peers0: Vec<_> = s.clients.iter().map(|k| s.net.peers[k.peer].stats).collect();
    let mism0: Vec<u64> = s.clients.iter().map(|k| k.client.predictor().stats().mismatches).collect();
    let err0: Vec<usize> = s.clients.iter().map(|k| k.client.predictor().stats().errors.len()).collect();
    let sizes0 = s.server.stats().sizes.len();
    let partial0 = s.server.stats().partial;
    let snaps0 = s.server.stats().snapshots;
    s.tick_us.clear();
    let shots0 = s.server.room(&s.room.unwrap_or(game::net::RoomCode(*b"AAAAAA"))).map(|a| a.stats()).unwrap_or_default();
    s.run(secs * 1000);
    let t = secs as f64;
    let n = players as f64;
    let mut up = 0u64;
    let mut down = 0u64;
    let mut upp = 0u64;
    let mut downp = 0u64;
    for (k, cl) in s.clients.iter().enumerate() {
        let p = s.net.peers[cl.peer].stats;
        up += p.up_bytes - peers0[k].up_bytes;
        down += p.down_bytes - peers0[k].down_bytes;
        upp += p.up_packets - peers0[k].up_packets;
        downp += p.down_packets - peers0[k].down_packets;
    }
    let mut errs: Vec<f64> = Vec::new();
    let mut shifts: Vec<f64> = Vec::new();
    let mut mism = 0u64;
    for (k, cl) in s.clients.iter().enumerate() {
        let st = cl.client.predictor().stats();
        mism += st.mismatches - mism0[k];
        errs.extend(st.errors[err0[k]..].iter().map(|&x| x as f64));
        shifts.extend(st.shifts[err0[k]..].iter().map(|&x| x as f64));
    }
    let mut sizes: Vec<f64> = s.server.stats().sizes[sizes0..].iter().map(|&x| x as f64).collect();
    let mut ticks: Vec<f64> = s.tick_us.iter().map(|&x| x as f64).collect();
    let arena = s.server.room(&s.room.unwrap_or(game::net::RoomCode(*b"AAAAAA"))).map(|a| a.stats()).unwrap_or_default();
    let snaps = (s.server.stats().snapshots - snaps0).max(1);
    let full_bytes = s.clients.iter().filter_map(|k| k.client.welcome().map(|w| w.full_bytes)).max().unwrap_or(0);
    let playing = s.playing();
    let disconnected = s.clients.iter().filter(|k| k.client.phase() == Phase::Closed).count();
    Measure {
        name: String::from(name),
        players,
        secs: t,
        mismatch_per_min: mism as f64 / n / t * 60.0,
        err: stats_of(&mut errs),
        shift: stats_of(&mut shifts),
        up_bps: up as f64 / n / t,
        down_bps: down as f64 / n / t,
        up_pps: upp as f64 / n / t,
        down_pps: downp as f64 / n / t,
        wire_up_bps: (up + upp * 28) as f64 / n / t,
        wire_down_bps: (down + downp * 28) as f64 / n / t,
        snap: stats_of(&mut sizes),
        partial: (s.server.stats().partial - partial0) as f64 / snaps as f64,
        tick_us: stats_of(&mut ticks),
        full_bytes,
        shots: arena.shots - shots0.shots,
        hits: arena.hits - shots0.hits,
        clamped: arena.clamped - shots0.clamped,
        view_ms: ((arena.view_sum - shots0.view_sum) as f64 / 256.0 / (arena.shots - shots0.shots).max(1) as f64 * 1000.0 / 60.0, arena.view_max as f64 / 256.0 * 1000.0 / 60.0),
        playing,
        disconnected,
    }
}

pub fn print_measure(m: &Measure) {
    crate::out(&format!(
        "  {:<22} {:>2} bots {:>3.0} s | predict: {:>6.1} mismatches/min/player, error mean {:.3} p99 {:.3} max {:.3} m, shift p99 {:.3} m | up {:>5.0} B/s {:>4.1} pkt/s (wire {:>5.0}), down {:>6.0} B/s {:>4.1} pkt/s (wire {:>6.0}) | snapshot mean {:>4.0} p50 {:>4.0} p99 {:>4.0} max {:>4.0} B, cut {:.1} % | tick mean {:>4.0} p50 {:>4.0} p99 {:>5.0} max {:>6.0} us | full {} B | shots {} hits {} clamped {} view lag mean {:.0} max {:.0} ms | playing {} closed {}\n",
        m.name, m.players, m.secs, m.mismatch_per_min, m.err.0, m.err.2, m.err.3, m.shift.2, m.up_bps, m.up_pps, m.wire_up_bps, m.down_bps, m.down_pps, m.wire_down_bps, m.snap.0, m.snap.1, m.snap.2, m.snap.3, m.partial * 100.0, m.tick_us.0, m.tick_us.1, m.tick_us.2, m.tick_us.3, m.full_bytes, m.shots, m.hits, m.clamped, m.view_ms.0, m.view_ms.1, m.playing, m.disconnected
    ));
}

/// the group bots: 16 bots in typical weather with 300 props and shooting, measured and checked against
/// the limits
pub fn bots(c: &mut Ctx) {
    let secs = if c.quick { 5 } else { 20 };
    let m = measure("typical, 300 props", 9, 16, Weather::TYPICAL, 300, 30, 2, secs, true);
    print_measure(&m);
    c.check(m.playing == 16 && m.disconnected == 0, || format!("16 bots: {} playing, {} closed", m.playing, m.disconnected));
    c.check(m.snap.3 <= 1200.0, || format!("16 bots: a snapshot of {} bytes", m.snap.3));
    c.check(m.up_pps <= 300.0 && m.up_bps <= 256.0 * 1024.0, || format!("16 bots: up {:.0} B/s, {:.1} packets/s", m.up_bps, m.up_pps));
    c.check(m.shots > 0, || String::from("16 bots: no shot fired"));
}

/// --netbench: the tables of docs/NETCODE.md (best in a release build)
pub fn bench() -> i32 {
    crate::out("gametest netbench: 16 bots, the arena, 30 s each after the joins (virtual time; the tick times are real)\n");
    let rows: [(&str, Weather, usize, usize, u32, bool); 9] = [
        ("clear, no props", Weather::CLEAR, 0, 0, 2, true),
        ("clear, 300 props", Weather::CLEAR, 300, 30, 2, true),
        ("typical, 300 props", Weather::TYPICAL, 300, 30, 2, true),
        ("typical, 300, 60 Hz", Weather::TYPICAL, 300, 30, 1, true),
        ("typical, 300, no fire", Weather::TYPICAL, 300, 30, 2, false),
        ("rough, 300 props", Weather::ROUGH, 300, 30, 2, true),
        ("storm, 300 props", Weather::STORM, 300, 30, 2, true),
        ("typical, 500 props", Weather::TYPICAL, 500, 100, 2, true),
        ("typical, 300 all move", Weather::TYPICAL, 300, 300, 2, true),
    ];
    for (k, (name, w, props, moving, every, fire)) in rows.iter().enumerate() {
        let m = measure(name, 100 + k as u64, 16, *w, *props, *moving, *every, 30, *fire);
        print_measure(&m);
    }
    let rewinds: [(&str, Weather, u32); 3] = [("clear, rewind 300 ms", Weather::CLEAR, 300), ("typical, rewind 300 ms", Weather::TYPICAL, 300), ("typical, rewind 400 ms", Weather::TYPICAL, 400)];
    for (k, (name, w, ms)) in rewinds.iter().enumerate() {
        let m = measure_rewind(name, 200 + k as u64, 16, *w, 300, 30, 2, 30, true, *ms);
        print_measure(&m);
    }
    // the size of a join's full state against the map's baseline and against nothing
    crate::out("join sizes (16 players):\n");
    for props in [0usize, 100, 200, 300, 500] {
        let cfg = ArenaConfig { props, moving: props / 10, displaced: props / 2, ..ArenaConfig::default() };
        let mut a = game::net::arena::Arena::new(cfg);
        for k in 0..16 {
            let _ = game::net::server::RoomSim::join(&mut a, &format!("P{}", k));
        }
        // half a minute of play: the moving props are away from their baseline places
        let ins: Vec<(u16, game::net::NetInput, bool)> = (1..=16u16).map(|i| (i, game::net::NetInput { move_z: 100, yaw: i * 4000, ..Default::default() }, true)).collect();
        for t in 1..=1800u32 {
            game::net::server::RoomSim::step(&mut a, t, 1.0 / 60.0, &ins);
        }
        let mut w = game::net::Snapshot::new(1800);
        game::net::server::RoomSim::snapshot(&a, 1800, &mut w);
        let schema = game::net::arena::schema();
        let bl = game::net::arena::baseline(&cfg);
        let vs_map = game::net::snapshot::full_bits(&schema, Some(&bl), &w, Some(&bl)).div_ceil(8) + 16;
        let vs_none = game::net::snapshot::full_bits(&schema, None, &w, Some(&bl)).div_ceil(8) + 16;
        let none_no_bl = game::net::snapshot::full_bits(&schema, None, &w, None).div_ceil(8) + 16;
        crate::out(&format!("  {:>3} props ({:>2} moving, half moved): against the map's baseline {:>5} B, against nothing {:>5} B (adds from the baseline), without a baseline {:>5} B\n", props, props / 10, vs_map, vs_none, none_no_bl));
    }
    0
}

/// --netsum: the checksum of a recorded session with a fixed seed (the same on every target)
pub fn checksum() -> (u64, u64) {
    let mut cfg = base_cfg(4242, 6, Weather::ROUGH);
    cfg.ws_every = 3;
    cfg.arena = ArenaConfig { props: 200, moving: 10, displaced: 100, fire: true, ..ArenaConfig::default() };
    cfg.fire = true;
    cfg.server.start_tick = u32::MAX - 600;
    let mut s = Session::new(cfg);
    s.record = Some(Vec::new());
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut n: u64 = 0;
    let fold = |h: &mut u64, n: &mut u64, v: u64| {
        for b in v.to_le_bytes() {
            *h ^= b as u64;
            *h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        *n += 1;
    };
    for k in 0..15 {
        if k == 10 {
            s.set_weather(Weather::CLEAR);
        }
        s.run(1000);
        for (id, b) in s.record.take().unwrap_or_default() {
            fold(&mut h, &mut n, id as u64);
            fold(&mut h, &mut n, b.len() as u64);
            for chunk in b.chunks(8) {
                let mut x = [0u8; 8];
                x[..chunk.len()].copy_from_slice(chunk);
                fold(&mut h, &mut n, u64::from_le_bytes(x));
            }
        }
        s.record = Some(Vec::new());
        for ((id, t), st) in s.server_states.iter() {
            fold(&mut h, &mut n, (*id as u64) << 32 | *t as u64);
            for v in st.pos.iter().chain(st.vel.iter()) {
                fold(&mut h, &mut n, *v as u32 as u64);
            }
            fold(&mut h, &mut n, st.flags as u64);
        }
        s.server_states.clear();
        for cl in &s.clients {
            for (t, st) in &cl.first {
                fold(&mut h, &mut n, *t as u64);
                for v in st.pos.iter().chain(st.vel.iter()) {
                    fold(&mut h, &mut n, *v as u32 as u64);
                }
            }
            let ps = cl.client.predictor().stats();
            fold(&mut h, &mut n, ps.mismatches);
            fold(&mut h, &mut n, ps.replayed);
            for e in &ps.errors {
                fold(&mut h, &mut n, e.to_bits() as u64);
            }
            let mut rem = Vec::new();
            cl.client.remotes(&mut rem);
            for r in &rem {
                fold(&mut h, &mut n, r.id as u64);
                fold(&mut h, &mut n, r.t.to_bits());
            }
        }
        for cl in s.clients.iter_mut() {
            cl.first.clear();
        }
    }
    let a = s.room.and_then(|r| s.server.room(&r)).map(|a| a.stats()).unwrap_or_default();
    fold(&mut h, &mut n, a.shots);
    fold(&mut h, &mut n, a.hits);
    fold(&mut h, &mut n, a.kills);
    fold(&mut h, &mut n, s.server.stats().snapshot_bytes);
    (h, n)
}
