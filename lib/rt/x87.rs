// x87 -- the extended-precision steps of the decimal <-> double conversions   [src/core/ini.s]
//
//   scale(m, e) -> f64          m * 10^e (e < 0: m / 10^-e), one rounding to double  (f64_parse)
//   scale_round(v, k) -> i64    round(v * 10^k) (k < 0: v / 10^-k), to nearest       (f64_format)
//   log10_round(v) -> i32       round(log10 v) as fyl2x computes it                  (f64_format)
//   pow(x, y) -> f64            x^y as fx_pow computes it (R6-icc)       [src/fx/fx_lut.s]
//   exp2 / log2 / cos / sin     fx_exp2 / fx_log2 / fx_cos / fx_sin (r10-fx)  [src/fx/fx_lut.s]
//   rz_exp2 / rz_log2           PaintCraft's rz_exp2 / rz_log2 (pn-ui; other rounding)  [dlg_rotzoom.s]
//
// ASMLIB computes these on the x87 unit with 64-bit precision and round to nearest (the
// control word is saved and restored in every call), 10^n as fld1 times the double table
// 1e0..1e22 (1e22 repeated for larger n).  The same instructions run here, so the text a
// double is written as and the double a text reads as are bit-identical to the assembly.

use core::arch::asm;

static POW10: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17,
    1e18, 1e19, 1e20, 1e21, 1e22,
];

/// m * 10^e or m / 10^-e (m < 2^63), rounded once to double  [f64_parse .fp_calc]
pub fn scale(m: u64, e: i32) -> f64 {
    let mi = m as i64;
    let mut out = 0f64;
    let mut cw = [0u16; 2];
    // SAFETY: the block reads mi / POW10 and writes cw / out through valid pointers, leaves
    // the x87 stack empty and restores the control word.
    unsafe {
        asm!(
            "fnstcw word ptr [{cw}]",
            "movzx {t:e}, word ptr [{cw}]",
            "and {t:e}, 0xF3FF",
            "or {t:e}, 0x0300",
            "mov word ptr [{cw} + 2], {t:x}",
            "fldcw word ptr [{cw} + 2]",
            "fld1",
            "2:",
            "cmp {n:e}, 22",
            "jbe 3f",
            "fmul qword ptr [{tab} + 176]",
            "sub {n:e}, 22",
            "jmp 2b",
            "3:",
            "fmul qword ptr [{tab} + {n} * 8]",
            "fild qword ptr [{m}]",
            "test {neg:e}, {neg:e}",
            "jnz 4f",
            "fmul st(0), st(1)",
            "jmp 5f",
            "4:",
            "fdiv st(0), st(1)",
            "5:",
            "fstp st(1)",
            "fstp qword ptr [{out}]",
            "fldcw word ptr [{cw}]",
            cw = in(reg) cw.as_mut_ptr(),
            t = out(reg) _,
            n = inout(reg) e.unsigned_abs() as u64 => _,
            tab = in(reg) POW10.as_ptr(),
            m = in(reg) &mi,
            neg = in(reg) (e < 0) as u32,
            out = in(reg) &mut out,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    out
}

/// frndint(v * 10^k) or frndint(v / 10^-k) as a 64-bit integer  [f64_format .ff_scale]
pub fn scale_round(v: f64, k: i32) -> i64 {
    let mut out = 0i64;
    let mut cw = [0u16; 2];
    // SAFETY: as in scale.
    unsafe {
        asm!(
            "fnstcw word ptr [{cw}]",
            "movzx {t:e}, word ptr [{cw}]",
            "and {t:e}, 0xF3FF",
            "or {t:e}, 0x0300",
            "mov word ptr [{cw} + 2], {t:x}",
            "fldcw word ptr [{cw} + 2]",
            "fld1",
            "2:",
            "cmp {n:e}, 22",
            "jbe 3f",
            "fmul qword ptr [{tab} + 176]",
            "sub {n:e}, 22",
            "jmp 2b",
            "3:",
            "fmul qword ptr [{tab} + {n} * 8]",
            "fld qword ptr [{v}]",
            "test {neg:e}, {neg:e}",
            "jnz 4f",
            "fmul st(0), st(1)",
            "jmp 5f",
            "4:",
            "fdiv st(0), st(1)",
            "5:",
            "fstp st(1)",
            "frndint",
            "fistp qword ptr [{out}]",
            "fldcw word ptr [{cw}]",
            cw = in(reg) cw.as_mut_ptr(),
            t = out(reg) _,
            n = inout(reg) k.unsigned_abs() as u64 => _,
            tab = in(reg) POW10.as_ptr(),
            v = in(reg) &v,
            neg = in(reg) (k < 0) as u32,
            out = in(reg) &mut out,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    out
}

/// fldlg2 / fyl2x / fistp: log10 v rounded to the nearest integer  [f64_format]
pub fn log10_round(v: f64) -> i32 {
    let mut out = 0i32;
    let mut cw = [0u16; 2];
    // SAFETY: as in scale.
    unsafe {
        asm!(
            "fnstcw word ptr [{cw}]",
            "movzx {t:e}, word ptr [{cw}]",
            "and {t:e}, 0xF3FF",
            "or {t:e}, 0x0300",
            "mov word ptr [{cw} + 2], {t:x}",
            "fldcw word ptr [{cw} + 2]",
            "fldlg2",
            "fld qword ptr [{v}]",
            "fyl2x",
            "fistp dword ptr [{out}]",
            "fldcw word ptr [{cw}]",
            cw = in(reg) cw.as_mut_ptr(),
            t = out(reg) _,
            v = in(reg) &v,
            out = in(reg) &mut out,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    out
}

/// fpatan: atan2(y, x) on the x87 unit, rounded once to double (the precision control does
/// not apply to fpatan)  [pathdata.s pd_atan2, dash.s dc_atan2]
pub fn atan2(y: f64, x: f64) -> f64 {
    let mut out = 0f64;
    // SAFETY: reads y / x and writes out through valid pointers; the x87 stack ends empty.
    unsafe {
        asm!(
            "fld qword ptr [{y}]",
            "fld qword ptr [{x}]",
            "fpatan",
            "fstp qword ptr [{out}]",
            y = in(reg) &y,
            x = in(reg) &x,
            out = in(reg) &mut out,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    out
}

/// fsincos: (cos x, sin x) on the x87 unit, each rounded once to double (the precision
/// control does not apply to fsincos)  [ui_value.s deg_sincos]
// ui-list: added for the angle dial and the colour wheel marker (styles/winui value.s, misc.s)
pub fn sincos(x: f64) -> (f64, f64) {
    let mut c = 0f64;
    let mut s = 0f64;
    // SAFETY: reads x and writes c / s through valid pointers; the x87 stack ends empty.
    unsafe {
        asm!(
            "fld qword ptr [{x}]",
            "fsincos",
            "fstp qword ptr [{c}]",
            "fstp qword ptr [{s}]",
            x = in(reg) &x,
            c = in(reg) &mut c,
            s = in(reg) &mut s,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    (c, s)
}

/// fx_pow: x^y -- NaN when x or y is NaN, 1 for y == 0, 0 for x <= 0, x for y == 1, else
/// 2^(y log2 x) on the x87 unit with 64-bit precision and round to nearest (fyl2x, then
/// frndint / f2xm1 / fscale), rounded once to double  [fx_lut.s fx_pow]
// R6-icc: added for the ICC curves and the Lab maths (icc_xform.s cv_eval / lab_f)
pub fn pow(x: f64, y: f64) -> f64 {
    const NAN: u64 = 0x7FF8000000000000;
    if y.is_nan() {
        return f64::from_bits(NAN);
    }
    if y == 0.0 {
        return 1.0;
    }
    if x.is_nan() {
        return f64::from_bits(NAN);
    }
    if x <= 0.0 {
        return 0.0;
    }
    if y == 1.0 {
        return x;
    }
    let mut out = 0f64;
    let mut cw = [0u16; 2];
    // SAFETY: reads x / y and writes cw / out through valid pointers, leaves the x87 stack
    // empty and restores the control word.
    unsafe {
        asm!(
            "fnstcw word ptr [{cw}]",
            "movzx {t:e}, word ptr [{cw}]",
            "and {t:e}, 0xF3FF",
            "or {t:e}, 0x0300",
            "mov word ptr [{cw} + 2], {t:x}",
            "fldcw word ptr [{cw} + 2]",
            "fld qword ptr [{y}]",
            "fld qword ptr [{x}]",
            "fyl2x",
            "fld st(0)",
            "frndint",
            "fsub st(1), st(0)",
            "fxch",
            "f2xm1",
            "fld1",
            "faddp",
            "fscale",
            "fstp st(1)",
            "fstp qword ptr [{out}]",
            "fldcw word ptr [{cw}]",
            cw = in(reg) cw.as_mut_ptr(),
            t = out(reg) _,
            x = in(reg) &x,
            y = in(reg) &y,
            out = in(reg) &mut out,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    out
}

/// fyl2x with y = 1: log2(x) on the x87 unit, rounded once to double (fyl2x is not subject
/// to the precision control)  [PaintCraft dlg_rotzoom.s rz_log2]
// pn-ui: added for the Rotationszoom zoom slider (fx's log2 below sets the precision control)
pub fn rz_log2(x: f64) -> f64 {
    let mut out = 0f64;
    // SAFETY: reads x and writes out through valid pointers; the x87 stack ends empty.
    unsafe {
        asm!(
            "fld1",
            "fld qword ptr [{x}]",
            "fyl2x",
            "fstp qword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut out,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    out
}

/// 2^x as rz_exp2 computes it (|x| < 2^30): i = cvtsd2si x, f = x - i (SSE), then on the x87
/// unit f2xm1 f, + 1 (faddp: rounded by the process's precision control, as in the
/// assembly), fscale by i, rounded to double  [PaintCraft dlg_rotzoom.s rz_exp2]
// pn-ui: added for the Rotationszoom zoom slider (fx's exp2 below rounds with frndint)
pub fn rz_exp2(x: f64) -> f64 {
    let i = crate::fx::cvtd(x);
    let f = x - i as f64;
    let mut out = 0f64;
    // SAFETY: reads i / f and writes out through valid pointers; the x87 stack ends empty.
    unsafe {
        asm!(
            "fild dword ptr [{i}]",
            "fld qword ptr [{f}]",
            "f2xm1",
            "fld1",
            "faddp",
            "fscale",
            "fstp qword ptr [{out}]",
            "fstp st(0)",
            i = in(reg) &i,
            f = in(reg) &f,
            out = in(reg) &mut out,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    out
}

// r10-fx: the remaining x87 helpers of the effect kernels (fx_lut.s fx_exp2 / fx_log2 /
// fx_cos / fx_sin): 64-bit precision and round to nearest (the control word is set and
// restored as X87_ENTER / X87_LEAVE do), the result rounded once to double
macro_rules! x87_unary {
    ($(#[$m:meta])* $name:ident, $($ins:literal),*) => {
        $(#[$m])*
        pub fn $name(x: f64) -> f64 {
            let mut out = 0f64;
            let mut cw = [0u16; 2];
            // SAFETY: reads x and writes cw / out through valid pointers, leaves the x87
            // stack empty and restores the control word.
            unsafe {
                asm!(
                    "fnstcw word ptr [{cw}]",
                    "movzx {t:e}, word ptr [{cw}]",
                    "and {t:e}, 0xF3FF",
                    "or {t:e}, 0x0300",
                    "mov word ptr [{cw} + 2], {t:x}",
                    "fldcw word ptr [{cw} + 2]",
                    $($ins,)*
                    "fstp qword ptr [{out}]",
                    "fldcw word ptr [{cw}]",
                    cw = in(reg) cw.as_mut_ptr(),
                    t = out(reg) _,
                    x = in(reg) &x,
                    out = in(reg) &mut out,
                    out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                    out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                );
            }
            out
        }
    };
}

x87_unary!(
    /// fx_exp2: 2^x (frndint / f2xm1 / fscale)  [fx_lut.s fx_exp2]
    exp2, "fld qword ptr [{x}]", "fld st(0)", "frndint", "fsub st(1), st(0)", "fxch", "f2xm1", "fld1",
    "faddp", "fscale", "fstp st(1)"
);
x87_unary!(
    /// fx_log2: log2 x (fld1 / fyl2x; x <= 0: -inf / NaN as the FPU delivers)  [fx_lut.s fx_log2]
    log2, "fld1", "fld qword ptr [{x}]", "fyl2x"
);
x87_unary!(
    /// fx_cos: fcos (|x| < 2^62)  [fx_lut.s fx_cos]
    cos, "fld qword ptr [{x}]", "fcos"
);
x87_unary!(
    /// fx_sin: fsin (|x| < 2^62)  [fx_lut.s fx_sin]
    sin, "fld qword ptr [{x}]", "fsin"
);
