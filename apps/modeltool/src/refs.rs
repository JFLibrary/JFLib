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

// ---------------------------------------------------------------- Boolean operations and bevels
//
// For the Boolean operations: which side of a convex mesh a point lies on, from the planes of its triangles,
// and whether a point lies inside any closed mesh, by the parity of a ray's crossings with its triangles (both
// in plain f64, with margins: points too close to a face are left out).  For axis-aligned boxes the volumes
// of the overlap are a product of interval lengths.  A box bevelled by `radius` with `k` segments is the
// Minkowski sum of the box shrunk by the radius and the polytope P whose corners are radius * the bevel's
// directions (per octant the grid normalize(i, j, k - i - j)): its volume is the shrunk box, its faces times
// the radius, its edges times the area of P's outline across them, and P's own volume.

/// a closed interval's overlap length (negative: the gap)
pub fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    a1.min(b1) - a0.max(b0)
}

/// Some(true) inside, Some(false) outside, None within `margin` of a face's plane: a convex closed mesh
pub fn inside_convex(m: &geom::Mesh, p: geom::Vec3, margin: f64) -> Option<bool> {
    let mut inside = true;
    for t in 0..m.triangle_count() {
        let [a, b, c] = m.corners(t);
        let n = (b - a).cross(c - a);
        let l = n.length();
        if l == 0.0 {
            continue;
        }
        let d = n.dot(p - (a + b + c) / 3.0) / l;
        if d > margin {
            return Some(false);
        }
        if d > -margin {
            inside = false;
        }
    }
    if inside { Some(true) } else { None }
}

/// Some(true) when p lies inside the closed mesh (an odd number of crossings of a ray), None when every ray
/// tried passes too close to an edge
pub fn inside_mesh(m: &geom::Mesh, p: geom::Vec3) -> Option<bool> {
    let dirs = [
        geom::Vec3::new(0.5773, 0.6234, 0.5271),
        geom::Vec3::new(-0.3197, 0.8411, -0.4363),
        geom::Vec3::new(0.7071, -0.1289, -0.6951),
        geom::Vec3::new(-0.2213, -0.5519, 0.8040),
    ];
    'dir: for d in dirs {
        let mut hits = 0;
        for t in 0..m.triangle_count() {
            let [a, b, c] = m.corners(t);
            let (e1, e2) = (b - a, c - a);
            let h = d.cross(e2);
            let det = e1.dot(h);
            let scale = e1.length() * e2.length();
            if geom::math::abs(det) <= 1e-12 * scale {
                continue; // parallel to the triangle
            }
            let s = p - a;
            let u = s.dot(h) / det;
            let q = s.cross(e1);
            let v = d.dot(q) / det;
            let tt = e2.dot(q) / det;
            if tt <= 0.0 {
                continue;
            }
            let eps = 1e-9;
            if u < -eps || v < -eps || u + v > 1.0 + eps {
                continue;
            }
            if u < eps || v < eps || u + v > 1.0 - eps {
                continue 'dir; // too close to an edge: another direction
            }
            hits += 1;
        }
        return Some(hits % 2 == 1);
    }
    None
}

/// the directions of a bevel's octant grid: normalize(i a + j b + (k - i - j) c)
fn grid_dir(a: [f64; 3], b: [f64; 3], c: [f64; 3], i: usize, j: usize, k: usize) -> [f64; 3] {
    let l = (k - i - j) as f64;
    let v = [a[0] * i as f64 + b[0] * j as f64 + c[0] * l, a[1] * i as f64 + b[1] * j as f64 + c[1] * l, a[2] * i as f64 + b[2] * j as f64 + c[2] * l];
    let n = sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    [v[0] / n, v[1] / n, v[2] / n]
}

/// the area of the bevel's outline across an edge: 4 quarter arcs of k chords through normalize(k - i, i)
pub fn bevel_outline_area(radius: f64, k: usize) -> f64 {
    let mut a = 0.0;
    for i in 0..k {
        let (x0, y0) = ((k - i) as f64, i as f64);
        let (x1, y1) = ((k - i - 1) as f64, (i + 1) as f64);
        let (l0, l1) = (sqrt(x0 * x0 + y0 * y0), sqrt(x1 * x1 + y1 * y1));
        a += 0.5 * (x0 * y1 - x1 * y0) / (l0 * l1);
    }
    4.0 * a * radius * radius
}

/// the volume of the bevel's polytope P (8 octants, each the cones over its k^2 grid triangles)
pub fn bevel_corner_volume(radius: f64, k: usize) -> f64 {
    let (ex, ey, ez) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
    let det = |a: [f64; 3], b: [f64; 3], c: [f64; 3]| -> f64 {
        geom::math::abs(a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0]))
    };
    let mut v = 0.0;
    for i in 0..k {
        for j in 0..k - i {
            v += det(grid_dir(ex, ey, ez, i, j, k), grid_dir(ex, ey, ez, i + 1, j, k), grid_dir(ex, ey, ez, i, j + 1, k));
            if i + j + 2 <= k {
                v += det(grid_dir(ex, ey, ez, i + 1, j, k), grid_dir(ex, ey, ez, i + 1, j + 1, k), grid_dir(ex, ey, ez, i, j + 1, k));
            }
        }
    }
    8.0 * v / 6.0 * radius * radius * radius
}

/// the volume of a box w x h x d with every edge and corner bevelled (radius, k segments)
pub fn bevel_box_volume(w: f64, h: f64, d: f64, radius: f64, k: usize) -> f64 {
    let (a, b, c) = (w - 2.0 * radius, h - 2.0 * radius, d - 2.0 * radius);
    a * b * c + 2.0 * radius * (a * b + b * c + c * a) + (a + b + c) * bevel_outline_area(radius, k) + bevel_corner_volume(radius, k)
}
