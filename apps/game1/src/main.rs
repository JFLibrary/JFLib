// game1 -- Spiel 1 (working name), part 1 of stage G7h (docs/GAME.md, docs/GAME1.md): a fast cartoon shooter, free for
// all with one hitscan weapon, against people over the network (a server: apps/game1-server) or against bots in this
// process; Windows, Linux, macOS and the browser
//
//   game1                          the window: start menu, practice, rooms by code, settings (src/app.rs)
//   game1 --check [--quick]        every test group without a window (tests/check.uis); exit code 1 on a failure
//   game1 --test NAME[,NAME]       groups: map move match netsim lagcomp render
//   game1 --sums                   the determinism checksums: one line "game1 sums map=.. rays=.. move=.. match=..
//                                  net=.."; the same on every target (the browser: tests/web.mjs in Node)
//   game1 --bench                  the measurements: frame time offscreen at 1080p with 16 figures, the server's time
//                                  per tick with 16 bots, the bandwidth per player (release build)
//   game1 --loopback [--secs S]    a server on 127.0.0.1 (UDP and WebSocket) and clients over real sockets
//   game1 --bake IN.glb OUT.g1m    modeltool's glTF into the game's format (tools/content.py runs it)
//
// The parts: game1_core (crates/game1_core: rules, map, movement, the room, bots, the server's loop; no GPU), the
// client's connection and prediction (play.rs, local.rs), the scene (scene.rs, look.rs: render3d in a ui.gpu_view),
// the window (app.rs), the bake (bake.rs), the tests (t_*.rs) and the measurements (bench.rs).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod app;
mod bake;
mod bench;
mod local;
#[cfg(not(target_arch = "wasm32"))]
mod loopback;
mod look;
mod play;
mod scene;
mod t_map;
mod t_match;
mod t_move;
mod t_net;
mod t_render;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use ui::*;

/// the app's own version: version= of apps/game1/PACKAGE
pub const APP_VERSION: &str = rt::app_version!();

static CMDS: [Command; 1] = [Command::new(app::CMD_QUIT, "&Beenden", 0, MOD_CTRL, b'Q' as u32, 0)];

const DESC: AppDesc = AppDesc {
    title: game1_core::rules::GAME_NAME,
    flags: AF_RESIZABLE | AF_GPU_3D,
    w: 1280,
    h: 720,
    min_w: 480,
    min_h: 320,
    cmds: &CMDS,
    ..AppDesc::NONE
};

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

/// the yaw that looks along (dx, dz) (yaw 0 looks along -Z)
pub fn look_yaw(dx: f64, dz: f64) -> f64 {
    physics::math::atan2(-dx, -dz)
}

/// xorshift64*: the same numbers on every target
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// uniform in [0, 1)
    pub fn f(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }

    /// uniform in [lo, hi)
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f()
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
}

/// the counters of a test group and its first failures
pub struct Ctx {
    pub group: &'static str,
    pub checks: u64,
    pub failed: u64,
    pub quick: bool,
    shown: u32,
}

impl Ctx {
    pub fn new(group: &'static str, quick: bool) -> Ctx {
        Ctx { group, checks: 0, failed: 0, quick, shown: 0 }
    }

    /// count a check; print the message of the first 12 failures of the group
    pub fn check(&mut self, ok: bool, msg: impl FnOnce() -> String) -> bool {
        self.checks += 1;
        if !ok {
            self.failed += 1;
            if self.shown < 12 {
                self.shown += 1;
                out(&format!("FAIL {}: {}\n", self.group, msg()));
            }
        }
        ok
    }

    /// a number of cases, a tenth with --quick
    pub fn n(&self, full: usize) -> usize {
        if self.quick { (full / 10).max(1) } else { full }
    }

    /// a note in the output (measured values)
    pub fn note(&self, s: &str) {
        out(&format!("  {}: {}\n", self.group, s));
    }
}

pub const ALL: &[&str] = &["map", "move", "match", "netsim", "lagcomp", "render"];

fn run_groups(names: &[&str], quick: bool) -> i32 {
    let (mut checks, mut failed) = (0u64, 0u64);
    for &name in names {
        let Some(&group) = ALL.iter().find(|g| **g == name) else {
            out(&format!("game1: unknown group {} (groups: {})\n", name, ALL.join(" ")));
            return 2;
        };
        let mut c = Ctx::new(group, quick);
        let t0 = plat::time_us();
        match group {
            "map" => t_map::run(&mut c),
            "move" => t_move::run(&mut c),
            "match" => t_match::run(&mut c),
            "netsim" => t_net::netsim(&mut c),
            "lagcomp" => t_net::lagcomp(&mut c),
            _ => t_render::run(&mut c),
        }
        let ms = (plat::time_us() - t0) / 1000;
        out(&format!("{:<8} {:>9} checks, {} failed ({} ms)\n", group, c.checks, c.failed, ms));
        checks += c.checks;
        failed += c.failed;
    }
    out(&format!("game1: {} von {} Prüfungen bestanden\n", checks - failed, checks));
    if failed == 0 { 0 } else { 1 }
}

fn run() -> i32 {
    let args: Vec<String> = plat::args().into_iter().skip(1).collect();
    let quick = args.iter().any(|a| a == "--quick");
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        out(&format!("game1 {} (JFLib {})\n", APP_VERSION, plat::VERSION));
        return 0;
    }
    if let Some(i) = args.iter().position(|a| a == "--bake") {
        return match (args.get(i + 1), args.get(i + 2)) {
            (Some(a), Some(b)) => bake::run(a, b),
            _ => {
                out("Aufruf: game1 --bake IN.glb OUT.g1m\n");
                2
            }
        };
    }
    if args.iter().any(|a| a == "--sums") {
        let s = t_match::sums(quick);
        out(&format!("game1 sums {}\n", s));
        return 0;
    }
    if args.iter().any(|a| a == "--bench") {
        return bench::run(quick);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if args.iter().any(|a| a == "--loopback") {
        let secs = value("--secs").and_then(|v| v.parse::<u64>().ok()).unwrap_or(5);
        return loopback::run(secs);
    }
    if let Some(name) = value("--test") {
        let names: Vec<&str> = name.split(',').collect();
        return run_groups(&names, quick);
    }
    if args.iter().any(|a| a == "--check") {
        return run_groups(ALL, quick);
    }
    let map = game1_core::map::werft();
    ui::launch(&DESC, app::Game1::new(map), &style::STYLE)
}

ui::main!(run());
