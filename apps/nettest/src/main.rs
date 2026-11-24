// nettest -- the transport for games (G7d): netio's WebSocket, lib/netchan's channel, the browser's
// network; tests, fuzzing, measurements and the server of the browser test
//
//   nettest --check [--quick]       every test group (tests/check.uis); exit code 1 when a check fails
//   nettest --test NAME             one group: codec ws sim reliable udp wschan fuzz
//   nettest --bench [--n N]         round-trip time and throughput on loopback: UDP channel, WebSocket,
//                                   channel over WebSocket (medians, docs/TRANSPORT.md)
//   nettest --serve DIR [--port P]  the server of the browser test: DIR's files (a web build), /ws an
//                                   echo WebSocket, /chan the channel (UDP on the same port as well);
//                                   prints "nettest serve PORT UDPPORT", ends with GET /quit
//   nettest --client PORT [--udp U] a desktop client against --serve: the channel over UDP and over
//                                   the WebSocket, echoes and round trips
//   nettest --version | --help
//
// In the browser (a web build, no arguments) it runs the browser test against the server it came
// from (browser.rs): plat's WebSocket and the channel over it; its lines go to the console.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod bench;
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod chantest;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod codec;
#[cfg(not(target_arch = "wasm32"))]
mod fuzz;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod raw;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod serve;
#[cfg(not(target_arch = "wasm32"))]
mod reltest;
#[cfg(not(target_arch = "wasm32"))]
mod sim;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod wstest;

use alloc::format;
use alloc::string::String;
#[cfg(not(target_arch = "wasm32"))]
use alloc::vec::Vec;

#[cfg(not(target_arch = "wasm32"))]
const APP_VERSION: &str = rt::app_version!();

/// the protocol id of nettest's channel
pub const PROTOCOL: u32 = 0x4e54_0001;

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

/// a pattern of n bytes (the echo server's "frag:N" and "blast:N:SIZE")
pub fn pattern(n: usize) -> alloc::vec::Vec<u8> {
    (0..n).map(|i| (i * 31 + i / 251) as u8).collect()
}

/// the median and the 99th percentile of samples (sorted in place)
pub fn median_p99(v: &mut [f64]) -> (f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    (v[v.len() / 2], v[(v.len() * 99 / 100).min(v.len() - 1)])
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

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// true with probability p
    pub fn chance(&mut self, p: f64) -> bool {
        ((self.next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)) < p
    }

    pub fn bytes(&mut self, n: usize) -> alloc::vec::Vec<u8> {
        (0..n).map(|_| self.next_u64() as u8).collect()
    }

    /// min + below(spread) random bytes
    pub fn some_bytes(&mut self, min: usize, spread: usize) -> alloc::vec::Vec<u8> {
        let n = min + self.below(spread);
        self.bytes(n)
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

/// the test groups in order
#[cfg(not(target_arch = "wasm32"))]
const ALL: &[&str] = &["codec", "ws", "sim", "reliable", "udp", "wschan", "fuzz"];

#[cfg(not(target_arch = "wasm32"))]
fn run_groups(names: &[&str], quick: bool) -> i32 {
    let (mut checks, mut failed) = (0u64, 0u64);
    for &name in names {
        let t0 = plat::time_us();
        let mut c = match name {
            #[cfg(not(target_os = "android"))]
            "codec" => codec::run(quick),
            #[cfg(not(target_os = "android"))]
            "ws" => wstest::run(quick),
            "sim" => sim::run(quick),
            "reliable" => reltest::run(quick),
            #[cfg(not(target_os = "android"))]
            "udp" => chantest::run_udp(quick),
            #[cfg(not(target_os = "android"))]
            "wschan" => chantest::run_ws(quick),
            "fuzz" => fuzz::run(quick),
            other => {
                out(&format!("nettest: no group {} on this target\n", other));
                return 2;
            }
        };
        let ms = (plat::time_us() - t0) / 1000;
        out(&format!("{:<7} {:>8} checks  {:>3} failed  {:>6} ms\n", c.group, c.checks, c.failed, ms));
        checks += c.checks;
        failed += core::mem::take(&mut c.failed);
    }
    out(&format!("nettest: {} checks, {} failed\n", checks, failed));
    if failed > 0 { 1 } else { 0 }
}

#[cfg(not(target_arch = "wasm32"))]
fn usage() -> i32 {
    out("Aufruf: nettest --check [--quick] | --test NAME | --bench [--n N] | --serve DIR [--port P] | --client PORT [--udp U] | --version | --help\n");
    0
}

fn run() -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        browser::start();
        0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let args: Vec<String> = plat::args().into_iter().skip(1).collect();
        let quick = args.iter().any(|a| a == "--quick");
        let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
        if args.iter().any(|a| a == "--help" || a == "-h") {
            return usage();
        }
        if args.iter().any(|a| a == "--version" || a == "-V") {
            out(&format!("nettest {} (JFLib {})\n", APP_VERSION, plat::VERSION));
            return 0;
        }
        #[cfg(not(target_os = "android"))]
        {
            if args.iter().any(|a| a == "--bench") {
                let n = value("--n").and_then(|v| v.parse::<usize>().ok()).unwrap_or(2000).max(10);
                return bench::run(n);
            }
            if let Some(dir) = value("--serve") {
                let port = value("--port").and_then(|v| v.parse::<u16>().ok()).unwrap_or(0);
                return serve::serve(&dir, port);
            }
            if let Some(port) = value("--client").and_then(|v| v.parse::<u16>().ok()) {
                let udp = value("--udp").and_then(|v| v.parse::<u16>().ok()).unwrap_or(port);
                return serve::client(port, udp);
            }
        }
        if let Some(name) = value("--test") {
            return run_groups(&[name.as_str()], quick);
        }
        if args.iter().any(|a| a == "--check") || args.is_empty() || args.iter().all(|a| a == "--quick" || a == "--script" || a.ends_with(".uis")) {
            #[cfg(target_os = "android")]
            return run_groups(&["sim", "fuzz"], quick);
            #[cfg(not(target_os = "android"))]
            return run_groups(ALL, quick);
        }
        plat::write_stderr(format!("nettest: unbekannte Argumente {:?}\n", args).as_bytes());
        2
    }
}

rt::main!(run());
