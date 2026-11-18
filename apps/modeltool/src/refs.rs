// refs -- independent references for the checks of lib/geom: closed formulas for the volumes and areas of the
// polyhedra the generators make, written from the geometry, not from geom's code.  A surface of revolution with
// n segments is a polyhedron whose bands are planar trapezoids: its volume is the discrete form of Pappus'
// theorem, V = n sin(2 pi / n) * (first moment of the profile about the axis) (the solid between two meridian
// planes is a linear image of the profile's prism with Jacobian rho sin(delta)), and each band's area is that
// of n isosceles trapezoids.  Boxes, stairs, prisms and mitred sweeps have their textbook formulas.

/// sin and cos of radians from rt's portable math (the trigonometry is not what is checked here)
pub fn sincos(x: f64) -> (f64, f64) {
    #[cfg(not(target_arch = "wasm32"))]
    let (c, s) = rt::portable::math::sincos(x);
    #[cfg(target_arch = "wasm32")]
    let (c, s) = rt::x87::sincos(x);
    (s, c)
}

pub fn sqrt(x: f64) -> f64 {
    geom::math::sqrt(x)
}

pub const PI: f64 = core::f64::consts::PI;

/// twice the signed area of a polygon
pub fn area2(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1]).sum()
}

/// the perimeter of a closed polygon
pub fn perimeter(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| {
            let (a, b) = (p[i], p[(i + 1) % n]);
            sqrt((b[0] - a[0]) * (b[0] - a[0]) + (b[1] - a[1]) * (b[1] - a[1]))
        })
        .sum()
}

/// the first moment about the y axis, the integral of x over the polygon (counter-clockwise: positive)
pub fn moment_x(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| {
            let (a, b) = (p[i], p[(i + 1) % n]);
            (a[0] * b[1] - b[0] * a[1]) * (a[0] + b[0])
        })
        .sum::<f64>()
        / 6.0
}

/// the volume of a profile (radius, y; closed, counter-clockwise) turned in n segments
pub fn lathe_volume(p: &[[f64; 2]], n: usize) -> f64 {
    n as f64 * sincos(2.0 * PI / n as f64).0 * moment_x(p)
}

/// the area of a profile (closed) turned in n segments: per edge n trapezoids (edges on the axis none)
pub fn lathe_area(p: &[[f64; 2]], n: usize) -> f64 {
    let (s, c) = sincos(PI / n as f64);
    let m = p.len();
    let mut a = 0.0;
    for i in 0..m {
        let (q, r) = (p[i], p[(i + 1) % m]);
        if q[0] == 0.0 && r[0] == 0.0 {
            continue;
        }
        let (dr, dy) = (r[0] - q[0], r[1] - q[1]);
        a += (q[0] + r[0]) * s * sqrt(dr * dr * c * c + dy * dy);
    }
    a * n as f64
}

/// the profile of a cylinder r x h (radius, y)
pub fn cylinder_profile(r: f64, h: f64) -> [[f64; 2]; 4] {
    [[0.0, 0.0], [r, 0.0], [r, h], [0.0, h]]
}

/// the profile of a UV sphere's meridian with `stacks` segments (degrees exact at the poles and the equator)
pub fn sphere_profile(r: f64, stacks: usize) -> alloc::vec::Vec<[f64; 2]> {
    let mut v: alloc::vec::Vec<[f64; 2]> = (0..=stacks)
        .map(|i| {
            if i == 0 {
                return [0.0, 0.0];
            }
            if i == stacks {
                return [0.0, 2.0 * r];
            }
            let (s, c) = sincos(PI * i as f64 / stacks as f64);
            [r * s, r - r * c]
        })
        .collect();
    v.dedup();
    v
}

/// the volume of a torus polyhedron: n segments about the axis, `sides` about the tube
pub fn torus_volume(major: f64, minor: f64, n: usize, sides: usize) -> f64 {
    let a = sides as f64 * 0.5 * minor * minor * sincos(2.0 * PI / sides as f64).0;
    n as f64 * sincos(2.0 * PI / n as f64).0 * a * major
}

/// box: (volume, area)
pub fn cuboid(w: f64, h: f64, d: f64) -> (f64, f64) {
    (w * h * d, 2.0 * (w * h + w * d + h * d))
}

/// stairs w x h x d with n steps: (volume, area)
pub fn stairs(w: f64, h: f64, d: f64, n: usize) -> (f64, f64) {
    let n = n as f64;
    let side = d * h * (n + 1.0) / (2.0 * n); // the profile: n columns of width d/n and heights h/n .. h
    (w * side, 2.0 * side + 2.0 * w * h + 2.0 * w * d)
}

/// an arch's profile area and perimeter, from its definition: a rectangle w x h without the opening, which
/// is (w - 2t) wide, straight up to h - t - r and closed by a half circle of n segments (r = w/2 - t)
pub fn arch(w: f64, h: f64, t: f64, n: usize) -> (f64, f64) {
    let r = w / 2.0 - t;
    let spring = h - t - r;
    let half = n as f64 * 0.5 * r * r * sincos(PI / n as f64).0; // n triangles at the centre
    let area = w * h - (2.0 * r * spring + half);
    let arc = n as f64 * 2.0 * r * sincos(PI / (2.0 * n as f64)).0;
    (area, 2.0 * h + w + 2.0 * t + 2.0 * spring + arc)
}
