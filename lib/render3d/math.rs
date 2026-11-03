// math -- what the scene computes on the CPU, in f64 (no libm in no_std; the matrices are in scene.rs)

pub type V3 = [f64; 3];

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

/// the tangent of a small angle (a field of view below pi): sine over cosine by their series
pub fn tan(x: f64) -> f64 {
    let (x2, mut s, mut c, mut ts, mut tc) = (x * x, 0.0, 0.0, x, 1.0);
    for n in 0..24 {
        let n = n as f64;
        s += ts;
        c += tc;
        ts *= -x2 / ((2.0 * n + 2.0) * (2.0 * n + 3.0));
        tc *= -x2 / ((2.0 * n + 1.0) * (2.0 * n + 2.0));
    }
    s / c
}

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn mul(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn len(a: V3) -> f64 {
    sqrt(dot(a, a))
}

/// a unit vector (zero stays zero)
pub fn norm(a: V3) -> V3 {
    let l = len(a);
    if l > 0.0 { mul(a, 1.0 / l) } else { a }
}

/// the rotation matrix's columns of a quaternion (x, y, z, w; any length except 0)
pub fn rotation(q: [f32; 4]) -> [V3; 3] {
    let (x, y, z, w) = (q[0] as f64, q[1] as f64, q[2] as f64, q[3] as f64);
    let l = x * x + y * y + z * z + w * w;
    let s = if l > 0.0 { 2.0 / l } else { 0.0 };
    [
        [1.0 - s * (y * y + z * z), s * (x * y + w * z), s * (x * z - w * y)],
        [s * (x * y - w * z), 1.0 - s * (x * x + z * z), s * (y * z + w * x)],
        [s * (x * z + w * y), s * (y * z - w * x), 1.0 - s * (x * x + y * y)],
    ]
}
