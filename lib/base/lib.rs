// base -- portable basics above the runtime and the platform     [src/core/str.s, args.s, ini.s]
//
//   utf8_decode(s, i) -> (codepoint, next)    invalid bytes decode as themselves
//   Args: value(name) / flag(name) / int(name, default) over the command line
//   num:  f64_parse / f64_format (bit-identical to ASMLIB), parse_i64
//   ini:  Ini -- settings files (load / parse / save / get* / set* / remove)
//   settings: Key / Schema / Settings -- typed settings over an Ini (versions, migrations)
//   rng:  Pcg32 / SplitMix64 -- deterministic pseudo-random numbers (simulations, tests)
//   metrics: Counter / Gauge / Hist (log-linear) / Window (n slots, quantiles without the
//         values) -- measurements of running systems
//   cmdtree: Tree / Parsed / Help -- IOS-like command trees (abbreviations, ?, Tab, "no", modes)
//   hashring: Ring / hash64 -- consistent hashing with virtual nodes (shards, sticky balancing)
//   lru:  Lru -- a map that forgets the least recently used entries (count and weight limits)
//
// Strings and formatting are Rust's own (str, String, format!); the thread pool, cpu
// features and files are in plat (they need unsafe).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

pub mod ini;
pub mod encoding;
pub mod xml;
pub mod num;
pub mod settings;
pub mod rng;
pub mod metrics;
pub mod cmdtree;
pub mod hashring;
pub mod lru;
pub use ini::Ini;
pub use settings::{Key, Schema, Settings};
pub use num::{f64_format, f64_parse, parse_i64};
pub use rng::{Pcg32, SplitMix64};

/// UTF-8 decoding: (codepoint, index after it); an invalid / overlong / surrogate / too large
/// sequence decodes as U+FFFD and advances one byte  [src/core/str.s utf8_decode]
pub fn utf8_decode(s: &[u8], i: usize) -> (u32, usize) {
    // invalid bytes, overlong forms, surrogates and values above U+10FFFF: U+FFFD, one byte
    // (as the assembly's utf8_decode; a sequence cut by the end of s is invalid)
    const BAD: u32 = 0xFFFD;
    let Some(&b0) = s.get(i) else { return (BAD, i.saturating_add(1)) };
    let b0 = b0 as u32;
    if b0 < 0x80 {
        return (b0, i + 1);
    }
    let (n, init, min) = match b0 {
        0xC2..=0xDF => (1, b0 & 0x1F, 0x80),
        0xE0..=0xEF => (2, b0 & 0x0F, 0x800),
        0xF0..=0xF4 => (3, b0 & 7, 0x10000),
        _ => return (BAD, i + 1), // continuation byte, overlong lead C0 / C1, F5..FF
    };
    let mut cp = init;
    let mut j = i + 1;
    for _ in 0..n {
        if j >= s.len() || s[j] & 0xC0 != 0x80 {
            return (BAD, i + 1);
        }
        cp = cp << 6 | (s[j] & 0x3F) as u32;
        j += 1;
    }
    if cp < min || cp > 0x10FFFF || cp & 0xFFFF_F800 == 0xD800 {
        return (BAD, i + 1); // overlong, too large, surrogate
    }
    (cp, j)
}

/// the command line (argument 0 is the program)
pub struct Args(pub Vec<String>);

impl Args {
    /// the argument after `name` (`--shot FILE`); argument 0 (the program) is skipped
    pub fn value(&self, name: &str) -> Option<&str> {
        let i = self.0.iter().skip(1).position(|a| a == name)? + 1;
        self.0.get(i + 1).map(|s| s.as_str())
    }

    pub fn flag(&self, name: &str) -> bool {
        self.0.iter().skip(1).any(|a| a == name)
    }

    /// a decimal value, `def` when missing or invalid
    pub fn int(&self, name: &str, def: i32) -> i32 {
        self.value(name).map_or(def, |v| v.parse().unwrap_or(def))
    }
}
