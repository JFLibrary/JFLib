// perlin -- deterministic 2-D gradient noise for the fx kernels (Clouds, Turbulence, Dents,
// Frosted Glass)                                                            [src/fx/fx_perlin.s]
//
//   perlin(x, y, seed, stream) -> f64        gradient noise, about -1 .. 1
//   perlin_row(rec, acc)                     acc[j] += amp * noise(x_j, y) along a row
//
// Improved Perlin noise (quintic fade 6t^5 - 15t^4 + 10t^3) on the integer lattice; the gradient
// of lattice point (ix, iy) is G[h & 15], h = hash32(ix, iy, seed, stream), G_k = sqrt(2)
// (cos, sin)(2 pi (k + 0.5) / 16).  g00 = G00.x fx + G00.y fy, g10 = G10.x (fx - 1) + G10.y fy,
// g01 = G01.x fx + G01.y (fy - 1), g11 = G11.x (fx - 1) + G11.y (fy - 1); a = g00 + u (g10 -
// g00), b = g01 + u (g11 - g01), noise = a + v (b - a), u / v = fade(fx / fy) computed as
// ((((f 6) - 15) f + 10) f) f f.  Coordinates are clamped to +-2^30; fixed operation order:
// perlin and perlin_row give bit-identical values.

use rt::fx::{cvttd64, maxsd, minsd};

const LO: f64 = -1073741824.0;
const HI: f64 = 1073741824.0;
const C1: u32 = 0x8DA6B343;
const C2: u32 = 0xD8163841;
const C3: u32 = 0xCB1AB31F;
const C4: u32 = 0x165667B1;

static GTAB: [[f64; 2]; 16] = [
    [1.3870398453221475, 0.275899379282943],
    [1.1758756024193588, 0.78569495838710213],
    [0.78569495838710235, 1.1758756024193588],
    [0.27589937928294311, 1.3870398453221475],
    [-0.27589937928294295, 1.3870398453221475],
    [-0.7856949583871019, 1.1758756024193588],
    [-1.1758756024193588, 0.78569495838710213],
    [-1.3870398453221475, 0.2758993792829435],
    [-1.3870398453221475, -0.27589937928294317],
    [-1.1758756024193591, -0.7856949583871019],
    [-0.78569495838710213, -1.1758756024193588],
    [-0.27589937928294361, -1.3870398453221473],
    [0.27589937928294306, -1.3870398453221475],
    [0.78569495838710168, -1.1758756024193591],
    [1.1758756024193588, -0.78569495838710213],
    [1.3870398453221473, -0.27589937928294367],
];

#[inline(always)]
fn fmix(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EBCA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2AE35);
    h ^ (h >> 16)
}

/// floor (i64) and fraction of a clamped coordinate  [FLOOR64]
#[inline(always)]
fn floor64(x: f64) -> (i64, f64) {
    let mut t = cvttd64(x);
    if t as f64 > x {
        t -= 1;
    }
    (t, x - t as f64)
}

#[inline(always)]
fn fade(f: f64) -> f64 {
    (((f * 6.0 - 15.0) * f + 10.0) * f) * f * f
}

#[inline(always)]
fn grad(h: u32, dx: f64, dy: f64) -> f64 {
    let g = &GTAB[(h & 15) as usize];
    g[0] * dx + g[1] * dy
}

/// gradient noise at (x, y)  [fxn_perlin]
pub fn perlin(x: f64, y: f64, seed: u32, stream: u32) -> f64 {
    let (ix, fx) = floor64(minsd(maxsd(x, LO), HI));
    let (iy, fy) = floor64(minsd(maxsd(y, LO), HI));
    let k = seed.wrapping_mul(C3) ^ stream.wrapping_mul(C4);
    let x0 = (ix as u32).wrapping_mul(C1);
    let x1 = x0.wrapping_add(C1);
    let y0 = (iy as u32).wrapping_mul(C2);
    let y1 = y0.wrapping_add(C2);
    let u = fade(fx);
    let v = fade(fy);
    let g00 = grad(fmix(x0 ^ y0 ^ k), fx, fy);
    let g10 = grad(fmix(x1 ^ y0 ^ k), fx - 1.0, fy - 0.0);
    let a = g00 + (g10 - g00) * u;
    let g01 = grad(fmix(x0 ^ y1 ^ k), fx - 0.0, fy - 1.0);
    let g11 = grad(fmix(x1 ^ y1 ^ k), fx - 1.0, fy - 1.0);
    let b = g01 + (g11 - g01) * u;
    a + (b - a) * v
}

/// a row of noise samples (PERLIN_ROW): x_j = (x0 + j step) * inv + ofsx
pub struct PerlinRow {
    pub n: usize,
    pub x0: i32,
    pub step: i32,
    pub abs: bool,
    pub seed: u32,
    pub stream: u32,
    pub inv: f64,
    pub ofsx: f64,
    pub y: f64,
    pub amp: f64,
}

/// acc[j] += amp * noise(x_j, y) (|noise| when abs); the lattice gradients are reused while
/// consecutive x_j stay in one cell  [fxn_perlin_row]
pub fn perlin_row(r: &PerlinRow, acc: &mut [f64]) {
    let (iy, fy) = floor64(minsd(maxsd(r.y, LO), HI));
    let fy1 = fy - 1.0;
    let v = fade(fy);
    let y0 = (iy as u32).wrapping_mul(C2);
    let y1 = y0.wrapping_add(C2);
    let k = r.seed.wrapping_mul(C3) ^ r.stream.wrapping_mul(C4);
    let (mut a0, mut b0, mut a1, mut b1) = ([0f64; 2], [0f64; 2], [0f64; 2], [0f64; 2]);
    let mut cached: Option<u32> = None;
    let mut xi = r.x0;
    for a in acc.iter_mut().take(r.n) {
        let x = minsd(maxsd(xi as f64 * r.inv + r.ofsx, LO), HI);
        let (ix, fx) = floor64(x);
        if cached != Some(ix as u32) {
            cached = Some(ix as u32);
            let x0 = (ix as u32).wrapping_mul(C1);
            let x1 = x0.wrapping_add(C1);
            let g00 = &GTAB[(fmix(x0 ^ y0 ^ k) & 15) as usize];
            let g01 = &GTAB[(fmix(x0 ^ y1 ^ k) & 15) as usize];
            let g10 = &GTAB[(fmix(x1 ^ y0 ^ k) & 15) as usize];
            let g11 = &GTAB[(fmix(x1 ^ y1 ^ k) & 15) as usize];
            a0 = [g00[0], g01[0]];
            b0 = [g00[1] * fy, g01[1] * fy1];
            a1 = [g10[0], g11[0]];
            b1 = [g10[1] * fy, g11[1] * fy1];
        }
        let p = [fx * a0[0] + b0[0], fx * a0[1] + b0[1]];
        let fxm = fx - 1.0;
        let q = [fxm * a1[0] + b1[0], fxm * a1[1] + b1[1]];
        let u = fade(fx);
        let ab = [p[0] + (q[0] - p[0]) * u, p[1] + (q[1] - p[1]) * u];
        let mut nv = ab[0] + (ab[1] - ab[0]) * v;
        if r.abs {
            nv = f64::from_bits(nv.to_bits() & 0x7FFF_FFFF_FFFF_FFFF);
        }
        *a += nv * r.amp;
        xi = xi.wrapping_add(r.step);
    }
}
