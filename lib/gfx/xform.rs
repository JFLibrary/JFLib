// xform -- affine matrices and the sine / cosine of the transform code     [src/img/xform.s]
//
//   Matrix {a, b, c, d, e, f}: (x, y) -> (a x + b y + c, d x + e y + f)
//   Matrix::IDENTITY, m.apply(x, y)
//   sincos(angle) -> (sin, cos)    polynomial with a two-part pi/2 reduction (xf_sincos)
//
// Used by pathdata (arcs, transformed output) and dash (round joins); img_xform (R6) builds
// on the same functions.  f64 operations in the order of the assembly.

use rt::fx::cvtd64;

/// an affine matrix (XM_*)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix { a: 1.0, b: 0.0, c: 0.0, d: 0.0, e: 1.0, f: 0.0 };

    /// xy = m * xy  [xf_apply]
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.b * y + self.c, self.d * x + self.e * y + self.f)
    }
}

const SIGN: u64 = 0x8000_0000_0000_0000;
const TWO_OPI: f64 = f64::from_bits(0x3FE4_5F30_6DC9_C883);
const PIO2HI: f64 = f64::from_bits(0x3FF9_21FB_5440_0000);
const PIO2LO: f64 = f64::from_bits(0x3DD0_B461_1A62_6331);
const SNAP: f64 = f64::from_bits(0x3D71_9799_812D_EA11); // 1e-12
const S3: f64 = f64::from_bits(0xBFC5_5555_5555_5555);
const S5: f64 = f64::from_bits(0x3F81_1111_1111_1111);
const S7: f64 = f64::from_bits(0xBF2A_01A0_1A01_A01A);
const S9: f64 = f64::from_bits(0x3EC7_1DE3_A556_C734);
const S11: f64 = f64::from_bits(0xBE5A_E645_67F5_44E4);
const S13: f64 = f64::from_bits(0x3DE6_1246_13A8_6D09);
const S15: f64 = f64::from_bits(0xBD6A_E7F3_E733_B81F);
const C2: f64 = f64::from_bits(0xBFE0_0000_0000_0000);
const C4: f64 = f64::from_bits(0x3FA5_5555_5555_5555);
const C6: f64 = f64::from_bits(0xBF56_C16C_16C1_6C17);
const C8: f64 = f64::from_bits(0x3EFA_01A0_1A01_A01A);
const C10: f64 = f64::from_bits(0xBE92_7E4F_B778_9F5C);
const C12: f64 = f64::from_bits(0x3E21_EED8_EFF8_D898);
const C14: f64 = f64::from_bits(0xBDA9_3974_A8C0_7C9D);
const C16: f64 = f64::from_bits(0x3D2A_E7F3_E733_B81F);

#[inline]
fn neg(v: f64) -> f64 {
    f64::from_bits(v.to_bits() ^ SIGN)
}

/// (sin angle, cos angle)  [xf_sincos]
pub fn sincos(angle: f64) -> (f64, f64) {
    let k = cvtd64(angle * TWO_OPI); // rint(angle * 2 / pi)
    let kd = k as f64;
    let mut r = angle - kd * PIO2HI;
    r = r - kd * PIO2LO;
    if !(f64::from_bits(r.to_bits() & !SIGN) >= SNAP) {
        r = 0.0;
    }
    let z = r * r;
    let p = (((((S15 * z + S13) * z + S11) * z + S9) * z + S7) * z + S5) * z + S3;
    let s = r * z * p + r;
    let c = ((((((C16 * z + C14) * z + C12) * z + C10) * z + C8) * z + C6) * z + C4) * z + C2;
    let c = c * z + 1.0;
    match k & 3 {
        1 => (c, neg(s)),
        2 => (neg(s), neg(c)),
        3 => (neg(c), s),
        _ => (s, c),
    }
}
