// math -- what the scene computes on the CPU in f64 (no libm in no_std; render3d's math.rs has the same)

/// the square root (x <= 0: 0)
pub fn sqrt(x: f64) -> f64 {
    if x <= 0.0 || x.is_nan() {
        return 0.0;
    }
    if x.is_infinite() {
        return x;
    }
    let mut r = f64::from_bits((x.to_bits() >> 1) + (1023u64 << 51));
    for _ in 0..6 {
        r = 0.5 * (r + x / r);
    }
    r
}

/// sine and cosine: reduced to -pi ..= pi, then their series
pub fn sincos(x: f64) -> (f64, f64) {
    let tau = core::f64::consts::TAU;
    let k = (x / tau + if x < 0.0 { -0.5 } else { 0.5 }) as i64;
    let r = x - k as f64 * tau;
    let (r2, mut s, mut c, mut ts, mut tc) = (r * r, 0.0, 0.0, r, 1.0);
    for n in 0..30 {
        let n = n as f64;
        s += ts;
        c += tc;
        ts *= -r2 / ((2.0 * n + 2.0) * (2.0 * n + 3.0));
        tc *= -r2 / ((2.0 * n + 1.0) * (2.0 * n + 2.0));
    }
    (s, c)
}

/// a coordinate as the two f32 the GPU holds of it: hi = f32(x), lo = f32(x - hi) (decision 28; docs/RENDER2D.md)
pub fn split(x: f64) -> (f32, f32) {
    let hi = x as f32;
    (hi, (x - hi as f64) as f32)
}
