// ops_jp2 -- lib/img_jp2 from the command line, for tools/jp2_check.py
//
//   --jp2 IN --to OUT [--reduce N]       decode; OUT: "J2D1", u32 width, height, components,
//                                        colour (0 unknown, 1 grey, 2 sRGB, 3 CMYK, 4 Lab, 5 ICC),
//                                        alpha (index + 1, 0 = none), premultiplied; per component
//                                        u32 width, height, dx, dy, precision, signed, then its
//                                        samples as i32 (little-endian).  Exit code 2 and the
//                                        error on stderr when the file is refused.
//   --jp2-mutate IN [--count N] [--seed S]
//                                        N damaged copies (bytes flipped, the file cut, spans
//                                        repeated or zeroed) decoded one after the other: none may
//                                        panic; prints how many decoded, how many were refused and
//                                        the slowest decode in ms
//   --jp2-bench IN [--reps N]            the decode time of IN (the best of N runs, ms)

use crate::Ct;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

fn out(s: String) {
    plat::write_stdout(s.as_bytes());
}

pub fn run_mode(ct: &mut Ct) -> Option<i32> {
    let a = &ct.args;
    let v = |n: &str| a.value(n).map(String::from);
    let num = |n: &str, d: u64| a.value(n).and_then(|s| s.parse::<u64>().ok()).unwrap_or(d);
    if let Some(p) = v("--jp2") {
        let Some(to) = v("--to") else {
            plat::write_stderr(b"error: --jp2 needs --to OUT\n");
            return Some(1);
        };
        return Some(cmd_decode(&p, &to, num("--reduce", 0) as u8));
    }
    if let Some(p) = v("--jp2-mutate") {
        return Some(cmd_mutate(&p, num("--count", 1000), num("--seed", 1)));
    }
    if let Some(p) = v("--jp2-bench") {
        return Some(cmd_bench(&p, num("--reps", 3)));
    }
    None
}

fn cmd_decode(p: &str, to: &str, reduce: u8) -> i32 {
    let Some(d) = crate::read_file(p) else { return 1 };
    let img = match img_jp2::decode(&d, &img_jp2::Options { reduce, ..img_jp2::Options::default() }) {
        Ok(i) => i,
        Err(e) => {
            plat::write_stderr(format!("error: {e:?}\n").as_bytes());
            return 2;
        }
    };
    let mut o = Vec::new();
    let mut u = |v: u32| o.extend_from_slice(&v.to_le_bytes());
    let colour = match img.color {
        img_jp2::Color::Unknown => 0,
        img_jp2::Color::Gray => 1,
        img_jp2::Color::Srgb => 2,
        img_jp2::Color::Cmyk => 3,
        img_jp2::Color::Lab => 4,
        img_jp2::Color::Icc(_) => 5,
    };
    u(img.width);
    u(img.height);
    u(img.components.len() as u32);
    u(colour);
    u(img.alpha.map_or(0, |a| a as u32 + 1));
    u(img.premultiplied as u32);
    let mut head = Vec::new();
    head.extend_from_slice(b"J2D1");
    head.extend_from_slice(&o);
    for c in &img.components {
        for v in [c.width, c.height, c.dx, c.dy, c.precision as u32, c.signed as u32] {
            head.extend_from_slice(&v.to_le_bytes());
        }
        for &s in &c.data {
            head.extend_from_slice(&s.to_le_bytes());
        }
    }
    if !plat::write_file(to, &head) {
        plat::write_stderr(format!("error: cannot write {to}\n").as_bytes());
        return 1;
    }
    0
}

/// xorshift64*
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn cmd_mutate(p: &str, count: u64, seed: u64) -> i32 {
    let Some(d) = crate::read_file(p) else { return 1 };
    if d.is_empty() {
        plat::write_stderr(format!("error: {p} is empty (nothing to damage)\n").as_bytes());
        return 1;
    }
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let (mut ok, mut refused, mut slowest) = (0u64, 0u64, 0u64);
    let opt = img_jp2::Options { max_pixels: 1 << 24, ..img_jp2::Options::default() };
    for _ in 0..count {
        let mut m = d.clone();
        match rng.below(5) {
            0 | 1 => {
                for _ in 0..1 + rng.below(8) {
                    let i = rng.below(m.len());
                    m[i] ^= 1 << rng.below(8);
                }
            }
            2 => {
                let n = rng.below(m.len());
                m.truncate(n);
            }
            3 => {
                let (a, n) = (rng.below(m.len()), 1 + rng.below(64));
                let span: Vec<u8> = m[a..(a + n).min(m.len())].to_vec();
                let at = rng.below(m.len());
                m.splice(at..at, span);
            }
            _ => {
                let (a, n) = (rng.below(m.len()), 1 + rng.below(32));
                let e = (a + n).min(m.len());
                for b in &mut m[a..e] {
                    *b = if rng.below(2) == 0 { 0 } else { 0xFF };
                }
            }
        }
        let t = plat::time_us();
        match img_jp2::decode(&m, &opt) {
            Ok(_) => ok += 1,
            Err(_) => refused += 1,
        }
        slowest = slowest.max(plat::time_us() - t);
    }
    out(format!("jp2-mutate {p}: {count} copies, {ok} decoded, {refused} refused, slowest {} ms\n", slowest / 1000));
    0
}

fn cmd_bench(p: &str, reps: u64) -> i32 {
    let Some(d) = crate::read_file(p) else { return 1 };
    let mut best = u64::MAX;
    for _ in 0..reps.max(1) {
        let t = plat::time_us();
        if let Err(e) = img_jp2::decode(&d, &img_jp2::Options::default()) {
            plat::write_stderr(format!("error: {e:?}\n").as_bytes());
            return 2;
        }
        best = best.min(plat::time_us() - t);
    }
    out(format!("jp2-bench {p}: {}.{:03} ms\n", best / 1000, best % 1000));
    0
}
