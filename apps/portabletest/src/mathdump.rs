// --math-dump: rt::x87 and rt::portable::math on the same fixed inputs, one line each:
//   GROUP A B X87 PORTABLE      (all u64 in hex: f64 bits, integers as i64; B 0 for one argument)
// On x86-64 rt::x87 is the x87 unit, on ARM64 it is portable/math.rs (docs/ARM64.md, ARM2), so
// tests/math_diff.py measures from two dumps what ARM64 computes differently from x86-64.  The inputs
// follow what the library hands over: f64_parse (scale), f64_format (log10_round, then scale_round
// with 1..17 digits), the effects, audio and UI (exp2, log2, pow, sin, cos, atan2), PaintCraft's
// rotate / zoom dialog (rz_exp2, rz_log2).

use alloc::string::String;
use core::fmt::Write;

use rt::portable::math as pm;
use rt::x87;

const N: usize = 20000;

struct Dump {
    text: String,
    seed: u64,
}

impl Dump {
    fn line(&mut self, group: &str, a: u64, b: u64, x87: u64, portable: u64) {
        let _ = writeln!(self.text, "{group} {a:x} {b:x} {x87:x} {portable:x}");
        if self.text.len() > 1 << 16 {
            rt::os::write_stdout(self.text.as_bytes());
            self.text.clear();
        }
    }

    fn f(&mut self, group: &str, a: f64, b: f64, x87: f64, portable: f64) {
        self.line(group, a.to_bits(), b.to_bits(), x87.to_bits(), portable.to_bits());
    }

    /// uniform in [lo, hi)
    fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (super::next(&mut self.seed) >> 11) as f64 / (1u64 << 53) as f64 * (hi - lo)
    }

    /// any positive finite double, denormals included (uniform in the bits)
    fn any_positive(&mut self) -> f64 {
        loop {
            let bits = (super::next(&mut self.seed) >> 1) % 0x7FF0_0000_0000_0000;
            if bits != 0 {
                return f64::from_bits(bits);
            }
        }
    }
}

pub fn run() -> i32 {
    let mut d = Dump { text: String::new(), seed: 0x2545_F491_4F6C_DD1D };
    for _ in 0..N {
        let m = super::next(&mut d.seed) >> 1;
        let e = (super::next(&mut d.seed) % 649) as i32 - 340;
        d.line("scale", m, e as i64 as u64, x87::scale(m, e).to_bits(), pm::scale(m, e).to_bits());
    }
    for _ in 0..N {
        let v = d.any_positive();
        d.line("log10_round", v.to_bits(), 0, x87::log10_round(v) as i64 as u64, pm::log10_round(v) as i64 as u64);
        let k = (super::next(&mut d.seed) % 17) as i32 - pm::log10_round(v);
        d.line("scale_round", v.to_bits(), k as i64 as u64, x87::scale_round(v, k) as u64, pm::scale_round(v, k) as u64);
    }
    for _ in 0..N {
        let x = d.uniform(-1100.0, 1030.0);
        d.f("exp2", x, 0.0, x87::exp2(x), pm::exp2(x));
        let x = d.uniform(-1.0, 1.0);
        d.f("exp2_unit", x, 0.0, x87::exp2(x), pm::exp2(x));
        let x = d.any_positive();
        d.f("log2", x, 0.0, x87::log2(x), pm::log2(x));
        let x = d.uniform(0.5, 2.0);
        d.f("log2_unit", x, 0.0, x87::log2(x), pm::log2(x));
    }
    for _ in 0..N {
        let (x, y) = (d.uniform(0.0, 100.0), d.uniform(-20.0, 20.0));
        d.f("pow", x, y, x87::pow(x, y), pm::pow(x, y));
        let (x, y) = (d.uniform(0.0, 1.0), d.uniform(0.1, 10.0));
        d.f("pow_gamma", x, y, x87::pow(x, y), pm::pow(x, y));
    }
    for _ in 0..N {
        let x = d.uniform(-20.0, 20.0);
        d.f("sin", x, 0.0, x87::sin(x), pm::sin(x));
        d.f("cos", x, 0.0, x87::cos(x), pm::cos(x));
        let ((c, s), (pc, ps)) = (x87::sincos(x), pm::sincos(x));
        d.f("sincos_sin", x, 0.0, s, ps);
        d.f("sincos_cos", x, 0.0, c, pc);
        let x = d.uniform(-1e6, 1e6);
        d.f("sin_wide", x, 0.0, x87::sin(x), pm::sin(x));
        d.f("cos_wide", x, 0.0, x87::cos(x), pm::cos(x));
    }
    for _ in 0..N {
        let (y, x) = (d.uniform(-100.0, 100.0), d.uniform(-100.0, 100.0));
        d.f("atan2", y, x, x87::atan2(y, x), pm::atan2(y, x));
        let x = d.uniform(-16.0, 16.0);
        d.f("rz_exp2", x, 0.0, x87::rz_exp2(x), pm::rz_exp2(x));
        let x = d.uniform(0.0, 256.0);
        d.f("rz_log2", x, 0.0, x87::rz_log2(x), pm::rz_log2(x));
    }
    rt::os::write_stdout(d.text.as_bytes());
    0
}
