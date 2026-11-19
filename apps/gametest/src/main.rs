// gametest -- lib/game (and its bridge to render3d, lib/game_render3d) against references of its own
//
//   gametest --check [--quick]   every test group (tests/check.uis); exit code 1 when a check fails
//   gametest --test NAME         one group: world tick input camera bridge netcodec lagcomp room netsim bots
//   gametest --checksum          the determinism harness: one line "gametest checksum=HEX" over the bits
//                                of a fixed run (world, systems, poses, input replayed, cameras); the same on
//                                every target
//   gametest --netsum            the netcode's: "gametest netsum=HEX" over a recorded networked session (a
//                                server and six bots on the simulated link, rough then clear, a fixed seed)
//   gametest --netbench          the netcode's measurements with 16 bots (docs/NETCODE.md; release build)
//   gametest --loopback [--secs S] [--udp N] [--ws N]   the netcode on real sockets on 127.0.0.1
//   gametest [--script S]        the probe window (src/probe.rs): ui's real input path into an Input, for
//                                the scripts tests/input.uis and tests/tick.uis (headless)
//
// The references share no code with lib/game: the world against a model of plain vectors (which entity
// lives with which generation, each column's values and its order of iteration by the rule push / swap
// remove), the tick against floor(time x rate / 10^9) in integers, the input against the rules of
// docs/GAMECORE.md written out case by case, the cameras against Quat::from_yaw_pitch and closed forms,
// the bridge against a render3d scene built directly (0 pixels different).  docs/GAMECORE.md lists what
// each group covers.  Without Vulkan or a usable GPU the bridge prints SKIP and passes.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod bridge;
mod camera_t;
mod checksum;
mod input_t;
mod net_codec;
mod net_lag;
// (real sockets: not in the browser, where only the console paths run in Node: tests/web.mjs)
#[cfg(not(target_arch = "wasm32"))]
mod net_loop;
mod net_room;
mod net_session;
mod net_sim;
mod probe;
mod tick_t;
mod world_t;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// the app's own version: version= of apps/gametest/PACKAGE
pub const APP_VERSION: &str = rt::app_version!();

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
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
        (self.next_u64() % n as u64) as usize
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
}

pub const ALL: &[&str] = &["world", "tick", "input", "camera", "bridge", "netcodec", "lagcomp", "room", "netsim", "bots"];

fn run_groups(names: &[&str], quick: bool) -> i32 {
    let (mut checks, mut failed) = (0u64, 0u64);
    for &name in names {
        let Some(&group) = ALL.iter().find(|g| **g == name) else {
            out(&format!("gametest: unknown group {} (groups: {})\n", name, ALL.join(" ")));
            return 2;
        };
        let mut c = Ctx::new(group, quick);
        let t0 = plat::time_us();
        match group {
            "world" => world_t::run(&mut c),
            "tick" => tick_t::run(&mut c),
            "input" => input_t::run(&mut c),
            "camera" => camera_t::run(&mut c),
            "netcodec" => net_codec::run(&mut c),
            "lagcomp" => net_lag::run(&mut c),
            "room" => net_room::run(&mut c),
            "netsim" => net_sim::run(&mut c),
            "bots" => net_sim::bots(&mut c),
            _ => bridge::run(&mut c),
        }
        let ms = (plat::time_us() - t0) / 1000;
        out(&format!("{:<8} {:>9} checks, {} failed ({} ms)\n", group, c.checks, c.failed, ms));
        checks += c.checks;
        failed += c.failed;
    }
    out(&format!("gametest: {} von {} Prüfungen bestanden\n", checks - failed, checks));
    if failed == 0 { 0 } else { 1 }
}

fn run() -> i32 {
    let args: Vec<String> = plat::args().into_iter().skip(1).collect();
    let quick = args.iter().any(|a| a == "--quick");
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        out(&format!("gametest {} (JFLib {})\n", APP_VERSION, plat::VERSION));
        return 0;
    }
    if args.iter().any(|a| a == "--checksum") {
        let (sum, n) = checksum::run();
        out(&format!("gametest checksum={:016x} results={}\n", sum, n));
        return 0;
    }
    if args.iter().any(|a| a == "--netsum") {
        let (sum, n) = net_sim::checksum();
        out(&format!("gametest netsum={:016x} values={}\n", sum, n));
        return 0;
    }
    if args.iter().any(|a| a == "--netbench") {
        return net_sim::bench();
    }
    #[cfg(not(target_arch = "wasm32"))]
    if args.iter().any(|a| a == "--loopback") {
        let num = |name: &str, d: u64| value(name).and_then(|v| v.parse::<u64>().ok()).unwrap_or(d);
        return net_loop::run(num("--secs", 6), num("--udp", 4) as usize, num("--ws", 4) as usize);
    }
    if let Some(name) = value("--test") {
        let names: Vec<&str> = name.split(',').collect();
        return run_groups(&names, quick);
    }
    if args.iter().any(|a| a == "--check") {
        return run_groups(ALL, quick);
    }
    probe::launch()
}

ui::main!(run());
