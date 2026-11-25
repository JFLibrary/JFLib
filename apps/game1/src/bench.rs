// bench -- game 1's measurements (`game1 --bench`, release build; docs/GAME1.md, "Messwerte")
//
//   frame      the client's scene offscreen at 1920 x 1080 with 16 figures walking, turning, shooting (muzzle flashes,
//              tracers, impacts), the weapon in front of the camera: the time of a frame waited for (CPU and GPU one
//              after the other), the frames a second with two in flight, the GPU's time from its timestamps
//   server     a room with 16 bots and nobody connected: the time of a tick (bots thinking, moving, shooting); and a
//              server with 16 clients over the simulated network (typical weather): the time of NetServer::step
//              (with the snapshots of every second tick)
//   bandwidth  per client of that session: up and down, payload with netchan's header and with IPv4 and UDP

use alloc::format;
use alloc::vec::Vec;

use game::net::server::{RoomSim, ServerOptions};
use game::net::sim::Weather;
use game1_core::rules::RATE;
use game1_core::sim::{Match, MatchConfig};

use crate::local::Local;
use crate::out;
use crate::scene::{Fx, View};
use crate::t_render::{device, test_figures, test_frame, Off};

fn stats(v: &mut [f64]) -> (f64, f64, f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    (mean, v[v.len() / 2], v[(v.len() * 99 / 100).min(v.len() - 1)], v[v.len() - 1])
}

fn frames(quick: bool) {
    let dev = match device("bench frame") {
        Ok(d) => d,
        Err(e) => {
            out(&format!("bench frame: SKIP: {}\n", e));
            return;
        }
    };
    let (w, h) = (1920u32, 1080u32);
    let mut v = View::new(game1_core::map::werft());
    if let Err(e) = v.attach_to(&dev) {
        out(&format!("bench frame: attach: {}\n", e));
        return;
    }
    let mut off = match Off::new(&dev, w, h) {
        Ok(o) => o,
        Err(e) => {
            out(&format!("bench frame: target: {}\n", e));
            return;
        }
    };
    if let Some((_, s)) = v.parts_mut() {
        s.set_stats(true);
    }
    let n = if quick { 60 } else { 300 };
    let mut times = Vec::new();
    let set = |v: &mut View, k: usize| {
        let t = k as f64 / 60.0;
        let mut f = test_frame();
        f.eye = game::Vec3::new(4.0 * physics::math::sin(t * 0.3), 1.65, 18.0);
        f.pitch = -0.05;
        f.figures = test_figures(16, t);
        f.show_vm = true;
        f.vm.bob = t * 9.0;
        f.vm.bob_amount = 1.0;
        f.time = t;
        v.frame = f;
        if k % 7 == 0 {
            v.fx.push(Fx::MuzzleOwn);
            let to = game::Vec3::new(-6.0 + (k % 13) as f64, 0.0, 2.0);
            v.fx.push(Fx::TracerOwn { to });
            v.fx.push(Fx::Impact { at: to, normal: game::Vec3::new(0.0, 1.0, 0.0) });
        }
        if k % 5 == 0 {
            v.fx.push(Fx::Muzzle { at: game::Vec3::new(-5.0 + (k % 11) as f64, 1.2, 10.0), dir: game::Vec3::new(0.0, 0.0, 1.0) });
        }
    };
    // waited for: every frame alone
    for k in 0..n + 30 {
        set(&mut v, k);
        let t0 = plat::time_us();
        if let Err(e) = off.frame(&dev, &mut v, false, true) {
            out(&format!("bench frame: {}\n", e));
            return;
        }
        if k >= 30 {
            times.push((plat::time_us() - t0) as f64 / 1000.0);
        }
    }
    let (mean, p50, p99, max) = stats(&mut times);
    // the GPU's own time
    let mut gpu = Vec::new();
    if let Some((_, s)) = v.parts_mut() {
        s.collect_stats();
        for st in s.take_stats() {
            if let Some(t) = st.stamps {
                gpu.push(t[3] - t[0]);
            }
        }
    }
    let (_, g50, g99, _) = stats(&mut gpu);
    // two in flight: frames a second
    let t0 = plat::time_us();
    for k in 0..n {
        set(&mut v, k);
        let last = k + 1 == n;
        if let Err(e) = off.frame(&dev, &mut v, false, last) {
            out(&format!("bench frame: {}\n", e));
            return;
        }
    }
    let fps = n as f64 / ((plat::time_us() - t0) as f64 * 1e-6);
    out(&format!(
        "bench frame 1920x1080, 16 figures, {} instances: waited {:.2} ms mean, p50 {:.2}, p99 {:.2}, max {:.2}; GPU p50 {:.2} ms, p99 {:.2} ({} stamps); pipelined {:.0} frames/s\n",
        v.stats.instances,
        mean,
        p50,
        p99,
        max,
        g50,
        g99,
        gpu.len(),
        fps
    ));
}

fn server(quick: bool) {
    let map = game1_core::map::werft();
    let mut m = Match::new(MatchConfig { bots: 16, seed: 3, ..MatchConfig::default() }, map.clone());
    let ticks = if quick { 600 } else { 3600 };
    let mut t = Vec::new();
    let mut snap = game::net::snapshot::Snapshot::new(0);
    for k in 1..=ticks + 120 {
        let t0 = plat::time_us();
        m.step(k, 1.0 / RATE as f64, &[]);
        if k % 2 == 0 {
            m.snapshot(k, &mut snap);
        }
        if k > 120 {
            t.push((plat::time_us() - t0) as f64);
        }
    }
    let st = m.stats();
    let (mean, p50, p99, max) = stats(&mut t);
    out(&format!(
        "bench server, a room with 16 bots, {} s: a tick {:.0} us mean, p50 {:.0}, p99 {:.0}, max {:.0} ({:.2} % of a core at 60 Hz); {} shots, {} kills\n",
        ticks / RATE,
        mean,
        p50,
        p99,
        max,
        mean * RATE as f64 / 1e4,
        st.shots,
        st.kills
    ));
    // 16 clients over the simulated network
    let mut l = Local::new(9, ServerOptions::default(), MatchConfig { seed: 9, ..MatchConfig::default() }, map.clone());
    l.tick_us = Some(Vec::new());
    for k in 0..16 {
        l.add_client(&format!("Last {}", k + 1), k % 4 == 3, Weather::TYPICAL, k as u64 * 60);
    }
    let mut d = crate::t_net::Drivers::new(16, 9, true);
    for _ in 0..3000 {
        l.step_ms(&mut |k, c| d.input(&map, k, c));
    }
    let before: Vec<_> = l.net.peers.iter().map(|p| p.stats).collect();
    if let Some(v) = &mut l.tick_us {
        v.clear();
    }
    let secs = if quick { 5 } else { 30 };
    for _ in 0..secs * 1000 {
        l.step_ms(&mut |k, c| d.input(&map, k, c));
    }
    let mut ticks: Vec<f64> = l.tick_us.take().unwrap_or_default().iter().map(|v| *v as f64).collect();
    let (mean, p50, p99, max) = stats(&mut ticks);
    out(&format!(
        "bench server, 16 clients over the simulated network (typical weather), {} s: NetServer::step {:.0} us mean, p50 {:.0}, p99 {:.0}, max {:.0}; {} playing\n",
        secs,
        mean,
        p50,
        p99,
        max,
        l.playing()
    ));
    let (mut up, mut down, mut upp, mut downp) = (0.0, 0.0, 0.0, 0.0);
    for (p, b) in l.net.peers.iter().zip(&before) {
        up += (p.stats.up_bytes - b.up_bytes) as f64;
        down += (p.stats.down_bytes - b.down_bytes) as f64;
        upp += (p.stats.up_packets - b.up_packets) as f64;
        downp += (p.stats.down_packets - b.down_packets) as f64;
    }
    let n = l.net.peers.len().max(1) as f64 * secs as f64;
    let ss = l.server.stats();
    out(&format!(
        "bench bandwidth per client: up {:.2} KB/s in {:.0} packets/s ({:.2} KB/s with IPv4+UDP), down {:.2} KB/s in {:.0} packets/s ({:.2} KB/s); snapshots up to {} bytes\n",
        up / n / 1000.0,
        upp / n,
        (up + upp * 28.0) / n / 1000.0,
        down / n / 1000.0,
        downp / n,
        (down + downp * 28.0) / n / 1000.0,
        ss.max_snapshot
    ));
}

/// -> exit code
pub fn run(quick: bool) -> i32 {
    frames(quick);
    server(quick);
    0
}
