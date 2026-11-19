// physicstest -- lib/physics against independent references, its checksums and its measurements
//
//   physicstest --check [--quick]   every test group (tests/check.uis); exit code 1 when a check fails
//   physicstest --test NAME         one group: math hull ray distance cast tunnel seams bvh world fuzz edge
//                                   character charfuzz
//   physicstest --checksum          the determinism harness: "physicstest checksum=HEX" over the bits of every
//                                   result of a fixed set of scenarios, "physicstest controller checksum=HEX"
//                                   over figures moved by the character controller; the same on every target
//   physicstest --bench [PART] [--runs N]  PART sqrt, map or controller (default all): the square roots,
//                                   the BVH build of a 50 000-triangle map, rays and capsule casts per second,
//                                   bytes per triangle, controllers stepped per ms (medians of N runs, default 3)
//   physicstest --version | --help
//
// The references (refs.rs) share no code with lib/physics: closest points by projection and edge
// minimum, polytope distances by brute force over feature pairs, penetration by the separating axes,
// times of impact by bisection on the convex gap function, the broad phases against loops over every
// body and triangle.  docs/PHYSICS.md lists what each group covers.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod bench;
mod character;
mod checksum;
mod refs;
mod scene;
mod tests;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const APP_VERSION: &str = rt::app_version!();

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

    pub fn vec(&mut self, lo: f64, hi: f64) -> physics::Vec3 {
        physics::Vec3::new(self.range(lo, hi), self.range(lo, hi), self.range(lo, hi))
    }

    /// a unit vector, uniform on the sphere
    pub fn dir(&mut self) -> physics::Vec3 {
        loop {
            let v = self.vec(-1.0, 1.0);
            let l = v.length_sq();
            if l > 1e-6 && l <= 1.0 {
                return v / physics::math::sqrt(l);
            }
        }
    }

    /// a random rotation
    pub fn quat(&mut self) -> physics::Quat {
        loop {
            let q = physics::Quat::new(self.range(-1.0, 1.0), self.range(-1.0, 1.0), self.range(-1.0, 1.0), self.range(-1.0, 1.0));
            let l = q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w;
            if l > 1e-6 && l <= 1.0 {
                return q.normalize();
            }
        }
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

fn usage() -> i32 {
    out("Aufruf: physicstest --check [--quick] | --test NAME | --checksum | --bench [sqrt|map|controller] [--runs N] | --version | --help\n");
    0
}

fn run() -> i32 {
    let args: Vec<String> = plat::args().into_iter().skip(1).collect();
    let quick = args.iter().any(|a| a == "--quick");
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return usage();
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        out(&format!("physicstest {} (JFLib {})\n", APP_VERSION, plat::VERSION));
        return 0;
    }
    if args.iter().any(|a| a == "--checksum") {
        let (sum, n) = checksum::run();
        out(&format!("physicstest checksum={:016x} results={}\n", sum, n));
        let (sum, n) = checksum::controller();
        out(&format!("physicstest controller checksum={:016x} results={}\n", sum, n));
        return 0;
    }
    if args.iter().any(|a| a == "--bench") {
        let runs = value("--runs").and_then(|v| v.parse::<usize>().ok()).unwrap_or(3).max(1);
        let part = value("--bench").filter(|v| !v.starts_with("--")).unwrap_or_else(|| "all".into());
        bench::run(runs, &part);
        return 0;
    }
    if let Some(name) = value("--test") {
        return tests::run(&[name.as_str()], quick);
    }
    if args.iter().any(|a| a == "--check") || args.is_empty() || args.iter().all(|a| a == "--quick" || a == "--script" || a.ends_with(".uis")) {
        return tests::run(tests::ALL, quick);
    }
    plat::write_stderr(format!("physicstest: unbekannte Argumente {:?}\n", args).as_bytes());
    2
}

rt::main!(run());
