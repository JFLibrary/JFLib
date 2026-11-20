// t_match -- the group "match": a room of game 1 on the server (game1_core::sim::Match) without a network.  Sixteen
// bots play a minute: they move, see, shoot and hit, die and come back at free spawn points, nobody sticks in the map,
// deaths are kills plus falls; a player who joins a full room takes a bot's place, and the bot comes back when the
// player leaves; a shot by hand: body and head damage, the end of a figure after enough hits, its return, the
// protection after it; lag compensation in the room: a shot at where the target was ten ticks ago hits with that
// view and misses without it.  Also the determinism line of --sums (map, rays, movement, match, networked session).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use game::net::input::NetInput;
use game::net::quant::{angle_q, axis_q, pitch_q};
use game::net::server::RoomSim;
use game1_core::map::{fnv, Map, FNV0};
use game1_core::movement::{MoveState, Mover};
use game1_core::rules::*;
use game1_core::sim::{Event, Match, MatchConfig};
use physics::{Filter, Iso, Vec3};

use crate::Ctx;

const DT: f64 = 1.0 / RATE as f64;

/// a room with `bots` bots for `ticks` ticks -> the checksum (every figure's replicated state every tick) and the room
pub fn bot_match(map: &alloc::sync::Arc<Map>, bots: usize, ticks: u32, seed: u64) -> (u64, Match, Vec<Event>) {
    let mut m = Match::new(MatchConfig { bots, seed, ..MatchConfig::default() }, map.clone());
    m.set_record(true);
    let mut h = FNV0;
    let mut events = Vec::new();
    let mut snap = game::net::snapshot::Snapshot::new(0);
    for t in 1..=ticks {
        m.step(t, DT, &[]);
        m.snapshot(t, &mut snap);
        for e in &snap.ents {
            h = fnv(h, &e.id.to_le_bytes());
            for v in &e.state.fields {
                h = fnv(h, &v.to_le_bytes());
            }
        }
        events.extend(m.take_events());
    }
    (h, m, events)
}

/// the line of --sums: map (collision mesh), rays, move (random figures), match (16 bots, a minute), net (a session
/// over the simulated network)
pub fn sums(quick: bool) -> String {
    let map = game1_core::map::werft();
    let rays = crate::t_map::ray_sum(&map, 20_000);
    let (mv, _) = crate::t_move::random_run(&map, 16, 1200, 9);
    let (mt, m, _) = bot_match(&map, 16, if quick { 1200 } else { 3600 }, 7);
    let st = m.stats();
    let (net, values) = crate::t_net::session_sum(&map, quick);
    format!(
        "map={:016x} rays={:016x} move={:016x} match={:016x} net={:016x} (match: {} shots, {} hits, {} kills; net: {} values)",
        map.collision_sum(),
        rays,
        mv,
        mt,
        net,
        st.shots,
        st.hits,
        st.kills,
        values
    )
}

fn input_at(yaw: f64, pitch: f64, buttons: u16) -> NetInput {
    NetInput { buttons, yaw: angle_q(yaw), pitch: pitch_q(pitch), ..NetInput::default() }
}

pub fn run(c: &mut Ctx) {
    let map = game1_core::map::werft();
    let mover = Mover::default();
    // sixteen bots, a minute
    let ticks = if c.quick { 1200 } else { 3600 };
    let (h, m, events) = bot_match(&map, 16, ticks, 7);
    let (h2, _, _) = bot_match(&map, 16, ticks.min(600), 7);
    let (h3, _, _) = bot_match(&map, 16, ticks.min(600), 7);
    c.check(h2 == h3, || "the same match twice gave other bits".into());
    let st = m.stats();
    c.note(&format!(
        "16 bots, {} s: {} shots, {} hits ({} heads), {} kills, {} falls, {} spawns, match={:016x}",
        ticks / RATE,
        st.shots,
        st.hits,
        st.heads,
        st.kills,
        st.falls,
        st.spawns,
        h
    ));
    c.check(m.figures().len() == 16, || format!("{} figures", m.figures().len()));
    c.check(st.shots > 100 && st.hits > 20 && st.kills > 3, || format!("too little fighting: {:?}", st));
    let deaths: i32 = m.figures().values().map(|f| f.deaths).sum();
    let kills: i32 = m.figures().values().map(|f| f.kills).sum();
    c.check(deaths as u64 == st.kills + st.falls && kills as u64 == st.kills, || format!("deaths {} kills {} against {:?}", deaths, kills, st));
    // every bot moved and shot
    let mut moved = alloc::collections::BTreeMap::new();
    let mut fired = alloc::collections::BTreeMap::new();
    for e in &events {
        if let Event::Shot { shooter, from, .. } = e {
            *fired.entry(*shooter).or_insert(0u32) += 1;
            moved.entry(*shooter).or_insert((*from, 0.0f64));
        }
    }
    let quiet: Vec<u16> = m.figures().keys().copied().filter(|id| !fired.contains_key(id)).collect();
    c.check(quiet.len() <= 2, || format!("bots that never fired: {:?}", quiet));
    // nobody inside the map (deeper than 2 mm: a landing may leave the capsule between 0 and its skin, and the grid
    // rounds by half a millimetre; the controller pushes such a figure out), health within its bounds
    let r = mover.ctrl.params().radius - 0.002;
    let mut inside = 0;
    let mut hits = Vec::new();
    for f in m.figures().values() {
        if f.state.dead() {
            continue;
        }
        hits.clear();
        let centre = mover.centre(&f.state);
        let half = 0.5 * mover.height(f.state.crouched()) - mover.ctrl.params().radius;
        map.world.overlap(&physics::Shape::capsule(half, r), &Iso::at(centre), &Filter::ALL, &mut hits);
        if !hits.is_empty() {
            inside += 1;
        }
        c.check((0..=HEALTH).contains(&f.health), || format!("health {}", f.health));
    }
    c.check(inside == 0, || format!("{} figures inside the map", inside));
    // the bots walked: the spread of their positions over the match
    let mut far = 0;
    let first: Vec<Vec3> = events.iter().filter_map(|e| if let Event::Spawn { id, spawn, .. } = e { Some((*id, *spawn)) } else { None }).take(16).map(|(_, s)| map.spawns[s].feet).collect();
    for (f, s) in m.figures().values().zip(first.iter()) {
        if (f.state.feet() - *s).length() > 3.0 {
            far += 1;
        }
    }
    c.check(far >= 8, || format!("only {} of 16 bots away from where they started", far));

    // a full room: a player takes a bot's place, and leaving brings it back
    let mut full = Match::new(MatchConfig { bots: 16, ..MatchConfig::default() }, map.clone());
    let p = full.join_player("Anna");
    c.check(p.is_some() && full.figures().len() == 16 && full.figures().values().filter(|f| f.is_bot()).count() == 15, || format!("join into a full room: {:?}, {} figures", p, full.figures().len()));
    if let Some(p) = p {
        full.leave(p);
        c.check(full.figures().len() == 16 && full.figures().values().all(|f| f.is_bot()), || "the bot did not come back".into());
    }

    // a shot by hand: A at (-10.5, 0, 8) looks south at B at (-10.5, 0, 17)
    let mut r = Match::new(MatchConfig::default(), map.clone());
    let a = r.join_player("A").unwrap_or(0);
    let b = r.join_player("B").unwrap_or(0);
    let place = |r: &mut Match, id: u16, at: Vec3| {
        if let Some(f) = r.figures_mut().get_mut(&id) {
            f.state = MoveState::at(at);
            f.safe_until = 0;
            f.health = HEALTH;
        }
    };
    let pa = Vec3::new(-10.5, 0.0, 8.0);
    let pb = Vec3::new(-10.5, 0.0, 17.0);
    place(&mut r, a, pa);
    place(&mut r, b, pb);
    let mut tick = 100u32;
    let still = NetInput::default();
    for _ in 0..20 {
        tick += 1;
        r.step(tick, DT, &[(a, still, true), (b, still, true)]);
    }
    let eye = mover.eye(&r.figures()[&a].state);
    let chest = r.figures()[&b].state.feet() + Vec3::new(0.0, 0.9, 0.0);
    let head = r.figures()[&b].state.feet() + Vec3::new(0.0, 1.62, 0.0);
    let aim = |to: Vec3| {
        let d = to - eye;
        (crate::look_yaw(d.x, d.z), physics::math::atan2(d.y, Vec3::new(d.x, 0.0, d.z).length()))
    };
    let (y1, p1) = aim(chest);
    tick += 1;
    r.step(tick, DT, &[(a, input_at(y1, p1, BTN_FIRE), true), (b, still, true)]);
    c.check(r.figures()[&b].health == HEALTH - DAMAGE, || format!("a body hit left {} health", r.figures()[&b].health));
    let (y2, p2) = aim(head);
    for _ in 0..FIRE_TICKS {
        tick += 1;
        r.step(tick, DT, &[(a, input_at(y2, p2, BTN_FIRE), true), (b, still, true)]);
    }
    c.check(r.figures()[&b].health == HEALTH - DAMAGE - HEAD_DAMAGE, || format!("a head shot left {} health", r.figures()[&b].health));
    c.check(rules_hits(&r, a) == 2, || "the shooter's hit counter".into());
    // shoot until B is down
    let mut down_at = None;
    for _ in 0..200 {
        tick += 1;
        r.step(tick, DT, &[(a, input_at(y1, p1, BTN_FIRE), true), (b, still, true)]);
        if r.figures()[&b].state.dead() {
            down_at = Some(tick);
            break;
        }
    }
    c.check(down_at.is_some() && r.figures()[&a].kills == 1 && r.figures()[&b].deaths == 1 && r.figures()[&b].killer == a, || "B was not ended by A".into());
    // B comes back after RESPAWN_TICKS, protected, with full health and magazine
    if let Some(t0) = down_at {
        let mut back = None;
        for _ in 0..RESPAWN_TICKS + 5 {
            tick += 1;
            r.step(tick, DT, &[(a, still, true), (b, still, true)]);
            if back.is_none() && !r.figures()[&b].state.dead() {
                back = Some(tick - t0);
            }
        }
        let f = &r.figures()[&b];
        c.check(back == Some(RESPAWN_TICKS), || format!("back after {:?} ticks", back));
        c.check(f.health == HEALTH && f.state.ammo() == MAGAZINE && (f.safe_until.wrapping_sub(tick) as i32) > 0, || "after coming back: health, magazine, protection".into());
    }

    // lag compensation: B runs along x; A shoots at where B was 10 ticks ago with that view (hit) and without (miss)
    let mut r = Match::new(MatchConfig::default(), map.clone());
    r.set_record(true);
    let a = r.join_player("A").unwrap_or(0);
    let b = r.join_player("B").unwrap_or(0);
    place(&mut r, a, pa);
    place(&mut r, b, Vec3::new(-14.0, 0.0, 17.0));
    let run = NetInput { move_z: axis_q(1.0), yaw: angle_q(-core::f64::consts::FRAC_PI_2), ..NetInput::default() }; // along +x
    let mut trail = Vec::new();
    let mut results = Vec::new();
    for k in 0..60 {
        tick += 1;
        let mut ia = still;
        if k == 40 || k == 50 {
            let seen: Vec3 = trail[trail.len() - 10];
            let (y, p) = aim(seen + Vec3::new(0.0, 0.9, 0.0));
            ia = input_at(y, p, BTN_FIRE);
            ia.view = if k == 40 { 10 * 256 } else { 0 };
        }
        r.step(tick, DT, &[(a, ia, true), (b, run, true)]);
        trail.push(r.figures()[&b].state.feet());
        for e in r.take_events() {
            if let Event::Shot { hit, view, .. } = e {
                results.push((view, hit.is_some()));
            }
        }
    }
    c.check(results == [(2560, true), (0, false)], || format!("lag compensated shots (view, hit): {:?}", results));
}

fn rules_hits(r: &Match, id: u16) -> u8 {
    r.figures().get(&id).map_or(0, |f| f.hits)
}
