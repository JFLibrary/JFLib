// ops_jbig2 -- lib/img_jbig2 from the command line, for tools/jbig2_check.py
//
//   --jbig2 IN --to OUT                  a JBIG2 file: each page as a PBM (P4) into OUT, OUT-2, ...
//                                        (exit code 2 and the error on stderr when refused)
//   --jbig2-embedded IN [--globals G] --to OUT
//                                        PDF's form (segments without the file header) and its
//                                        globals: page 1 as a PBM
//   --jbig2-mutate IN [--count N] [--seed S]
//                                        N damaged copies of a file (bytes flipped, cut, spans
//                                        repeated or zeroed): none may panic; prints how many
//                                        decoded, were refused, and the slowest decode in ms

use crate::Ct;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

fn pbm(b: &img_jbig2::Bitmap) -> Vec<u8> {
    let mut o = format!("P4\n{} {}\n", b.width, b.height).into_bytes();
    o.extend_from_slice(&b.bits);
    o
}

pub fn run_mode(ct: &mut Ct) -> Option<i32> {
    let a = &ct.args;
    let v = |n: &str| a.value(n).map(String::from);
    let num = |n: &str, d: u64| a.value(n).and_then(|s| s.parse::<u64>().ok()).unwrap_or(d);
    if let Some(p) = v("--jbig2") {
        let Some(to) = v("--to") else {
            plat::write_stderr(b"error: --jbig2 needs --to OUT\n");
            return Some(1);
        };
        let Some(d) = crate::read_file(&p) else { return Some(1) };
        return Some(match img_jbig2::decode_file(&d, &img_jbig2::Options::default()) {
            Ok(pages) => {
                for (i, pg) in pages.iter().enumerate() {
                    let name = if i == 0 { to.clone() } else { format!("{to}-{}", i + 1) };
                    if !plat::write_file(&name, &pbm(pg)) {
                        return Some(1);
                    }
                }
                0
            }
            Err(e) => {
                plat::write_stderr(format!("error: {e:?}\n").as_bytes());
                2
            }
        });
    }
    if let Some(p) = v("--jbig2-embedded") {
        let Some(to) = v("--to") else { return Some(1) };
        let Some(d) = crate::read_file(&p) else { return Some(1) };
        let g = match v("--globals") {
            Some(gp) => match crate::read_file(&gp) {
                Some(g) => Some(g),
                None => return Some(1),
            },
            None => None,
        };
        return Some(match img_jbig2::decode_embedded(&d, g.as_deref(), &img_jbig2::Options::default()) {
            Ok(b) => {
                if plat::write_file(&to, &pbm(&b)) {
                    0
                } else {
                    1
                }
            }
            Err(e) => {
                plat::write_stderr(format!("error: {e:?}\n").as_bytes());
                2
            }
        });
    }
    if let Some(p) = v("--jbig2-mutate") {
        return Some(mutate(&p, num("--count", 1000), num("--seed", 1), v("--slow-out")));
    }
    None
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

fn mutate(p: &str, count: u64, seed: u64, slow_out: Option<String>) -> i32 {
    let Some(d) = crate::read_file(p) else { return 1 };
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let (mut ok, mut refused, mut slowest) = (0u64, 0u64, 0u64);
    let mut slow_copy: Vec<u8> = Vec::new();
    let opt = img_jbig2::Options { max_pixels: 1 << 24 };
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
        match img_jbig2::decode_file(&m, &opt) {
            Ok(_) => ok += 1,
            Err(_) => refused += 1,
        }
        let dt = plat::time_us() - t;
        if dt > slowest {
            slowest = dt;
            slow_copy = m;
        }
    }
    if let Some(f) = slow_out {
        plat::write_file(&f, &slow_copy);
    }
    plat::write_stdout(format!("jbig2-mutate {p}: {count} copies, {ok} decoded, {refused} refused, slowest {} ms\n", slowest / 1000).as_bytes());
    0
}
