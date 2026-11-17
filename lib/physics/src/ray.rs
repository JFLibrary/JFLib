// ray -- rays against the primitives in their own frames: o + t d, t in units of d, 0 <= t <= tmax.
// Convex shapes are solid: a ray that starts inside hits at t = 0 with the normal against d.  Triangles
// have two sides: the normal faces the ray's origin.

use crate::hull::ConvexHull;
use crate::math::{Vec3, sqrt};

/// the first hit of a ray: t and the unit normal (against the ray)
pub(crate) type Hit = (f64, Vec3);

#[inline]
fn against(d: Vec3) -> Vec3 {
    (-d).normalize_or_zero()
}

/// a sphere of radius r around the origin
pub(crate) fn sphere(o: Vec3, d: Vec3, r: f64, tmax: f64) -> Option<Hit> {
    let c = o.dot(o) - r * r;
    if c <= 0.0 {
        return Some((0.0, against(d)));
    }
    let l = d.length();
    if !(l > 0.0) {
        return None;
    }
    let dn = d / l;
    let b = o.dot(dn);
    if b >= 0.0 {
        return None; // outside and moving away
    }
    // the squared distance of the centre from the line, without b * b - c (Ray Tracing Gems, 7)
    let f = o - dn * b;
    let disc = r * r - f.dot(f);
    if disc < 0.0 {
        return None;
    }
    let q = -b + sqrt(disc);
    let t = (c / q) / l;
    if t > tmax {
        return None;
    }
    let p = o + d * t;
    Some((t, p.normalize_or_zero()))
}

/// a capsule around the segment (0, -h, 0) .. (0, h, 0)
pub(crate) fn capsule(o: Vec3, d: Vec3, h: f64, r: f64, tmax: f64) -> Option<Hit> {
    let y = crate::math::clamp(o.y, -h, h);
    let q = o - Vec3::new(0.0, y, 0.0);
    if q.dot(q) <= r * r {
        return Some((0.0, against(d)));
    }
    let mut best: Option<Hit> = None;
    // the side: x^2 + z^2 = r^2 between the caps
    let a = d.x * d.x + d.z * d.z;
    if a > 0.0 {
        let l = sqrt(a);
        let (dx, dz) = (d.x / l, d.z / l);
        let b = o.x * dx + o.z * dz;
        let c = o.x * o.x + o.z * o.z - r * r;
        if c > 0.0 && b < 0.0 {
            let fx = o.x - dx * b;
            let fz = o.z - dz * b;
            let disc = r * r - (fx * fx + fz * fz);
            if disc >= 0.0 {
                let t = (c / (-b + sqrt(disc))) / l;
                let py = o.y + d.y * t;
                if t <= tmax && py >= -h && py <= h {
                    let p = o + d * t;
                    best = Some((t, Vec3::new(p.x, 0.0, p.z).normalize_or_zero()));
                }
            }
        }
    }
    for cy in [-h, h] {
        let oc = o - Vec3::new(0.0, cy, 0.0);
        if let Some((t, n)) = sphere(oc, d, r, tmax) {
            if best.is_none_or(|b| t < b.0) {
                best = Some((t, n));
            }
        }
    }
    best
}

/// a box with half sizes h around the origin
pub(crate) fn cuboid(o: Vec3, d: Vec3, h: Vec3, tmax: f64) -> Option<Hit> {
    let mut t0 = 0.0f64;
    let mut t1 = tmax;
    let mut axis_n = usize::MAX;
    for a in 0..3 {
        let (oa, da, ha) = (o[a], d[a], h[a]);
        if da == 0.0 {
            if oa < -ha || oa > ha {
                return None;
            }
            continue;
        }
        let inv = 1.0 / da;
        let mut near = (-ha - oa) * inv;
        let mut far = (ha - oa) * inv;
        if near > far {
            core::mem::swap(&mut near, &mut far);
        }
        if near > t0 {
            t0 = near;
            axis_n = a;
        }
        if far < t1 {
            t1 = far;
        }
        if t0 > t1 {
            return None;
        }
    }
    if axis_n == usize::MAX {
        return Some((0.0, against(d))); // inside
    }
    let mut n = Vec3::ZERO;
    let s = if d[axis_n] > 0.0 { -1.0 } else { 1.0 };
    match axis_n {
        0 => n.x = s,
        1 => n.y = s,
        _ => n.z = s,
    }
    Some((t0, n))
}

/// a convex hull: the ray clipped by its planes
pub(crate) fn hull(o: Vec3, d: Vec3, hull: &ConvexHull, tmax: f64) -> Option<Hit> {
    let mut t0 = 0.0f64;
    let mut t1 = tmax;
    let mut normal: Option<Vec3> = None;
    for &(n, pd) in hull.planes() {
        let dist = pd - n.dot(o); // > 0: inside this plane
        let den = n.dot(d);
        if den == 0.0 {
            if dist < 0.0 {
                return None;
            }
            continue;
        }
        let t = dist / den;
        if den < 0.0 {
            if t > t0 {
                t0 = t;
                normal = Some(n);
            }
        } else if t < t1 {
            t1 = t;
        }
        if t0 > t1 {
            return None;
        }
    }
    match normal {
        None => Some((0.0, against(d))),
        Some(n) => Some((t0, n)),
    }
}

/// a triangle, watertight (Woop, Benthin, Wald, "Watertight Ray/Triangle Intersection", JCGT 2013): the
/// edge functions of a shared edge are the same products in both triangles, so no ray slips between
/// them; a ray exactly on an edge hits both.  -> t and the barycentric weights of a, b, c
pub(crate) fn triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3, tmax: f64) -> Option<(f64, [f64; 3])> {
    let ad = d.abs();
    let kz = if ad.x >= ad.y && ad.x >= ad.z {
        0
    } else if ad.y >= ad.z {
        1
    } else {
        2
    };
    if d[kz] == 0.0 {
        return None;
    }
    let mut kx = (kz + 1) % 3;
    let mut ky = (kx + 1) % 3;
    if d[kz] < 0.0 {
        core::mem::swap(&mut kx, &mut ky);
    }
    let sx = d[kx] / d[kz];
    let sy = d[ky] / d[kz];
    let sz = 1.0 / d[kz];
    let (pa, pb, pc) = (a - o, b - o, c - o);
    let ax = pa[kx] - sx * pa[kz];
    let ay = pa[ky] - sy * pa[kz];
    let bx = pb[kx] - sx * pb[kz];
    let by = pb[ky] - sy * pb[kz];
    let cx = pc[kx] - sx * pc[kz];
    let cy = pc[ky] - sy * pc[kz];
    let u = cx * by - cy * bx;
    let v = ax * cy - ay * cx;
    let w = bx * ay - by * ax;
    if (u < 0.0 || v < 0.0 || w < 0.0) && (u > 0.0 || v > 0.0 || w > 0.0) {
        return None;
    }
    let det = u + v + w;
    if det == 0.0 {
        return None;
    }
    let az = sz * pa[kz];
    let bz = sz * pb[kz];
    let cz = sz * pc[kz];
    let t = u * az + v * bz + w * cz;
    if det > 0.0 {
        if t < 0.0 || t > tmax * det {
            return None;
        }
    } else if t > 0.0 || t < tmax * det {
        return None;
    }
    let inv = 1.0 / det;
    Some((t * inv, [u * inv, v * inv, w * inv]))
}

/// the unit normal of triangle (a, b, c) facing against d
pub(crate) fn tri_normal_against(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> Vec3 {
    let n = (b - a).cross(c - a).normalize_or_zero();
    if n.dot(d) > 0.0 { -n } else { n }
}
