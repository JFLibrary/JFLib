// poly -- 2D profiles: signed area, orientation, cleaning, triangulation by ear clipping, and the profiles
// the modelling API offers (rectangle, circle, rounded rectangle, arc).  A profile is a simple polygon
// (no self-intersections, no holes) as a list of points; the closing edge from the last point to the
// first is implied.

use alloc::vec::Vec;

use crate::Error;
use crate::math::{abs, sin_cos_deg};

/// the most points a profile or path may have
pub const MAX_PROFILE_POINTS: usize = 65_536;

/// twice the signed area (shoelace): positive counter-clockwise (x right, y up)
pub fn signed_area2(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    let mut a = 0.0;
    for i in 0..n {
        let (q, r) = (p[i], p[(i + 1) % n]);
        a += q[0] * r[1] - r[0] * q[1];
    }
    a
}

/// the signed area in square metres: positive counter-clockwise
pub fn area(p: &[[f64; 2]]) -> f64 {
    signed_area2(p) * 0.5
}

/// Points of a profile or path closer than this fraction of its extent count as one (a script's
/// arithmetic, `(y + r) - r`, leaves such near-duplicates; they would make triangles without area).
pub const MERGE_RATIO: f64 = 1e-9;

/// the profile checked and cleaned: finite numbers, consecutive points closer than [`MERGE_RATIO`] of its
/// extent merged (the first one stays) and so a repeated first point at the end, at least 3 points with
/// an area; `what` names it in the message
pub fn clean(p: &[[f64; 2]], what: &str) -> Result<Vec<[f64; 2]>, Error> {
    if p.len() > MAX_PROFILE_POINTS {
        return Err(Error::TooBig(alloc::format!("{}: mehr als {} Punkte", what, MAX_PROFILE_POINTS)));
    }
    if !p.iter().all(|q| q[0].is_finite() && q[1].is_finite()) {
        return Err(Error::Profile(alloc::format!("{}: eine Koordinate ist nicht endlich", what)));
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for q in p {
        for k in 0..2 {
            lo[k] = crate::math::min(lo[k], q[k]);
            hi[k] = crate::math::max(hi[k], q[k]);
        }
    }
    let eps = if p.is_empty() { 0.0 } else { MERGE_RATIO * ((hi[0] - lo[0]) + (hi[1] - lo[1])) };
    let near = |a: [f64; 2], b: [f64; 2]| abs(a[0] - b[0]) <= eps && abs(a[1] - b[1]) <= eps;
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(p.len());
    for &q in p {
        let q = [q[0] + 0.0, q[1] + 0.0];
        if out.last().is_none_or(|&l| !near(l, q)) {
            out.push(q);
        }
    }
    while out.len() > 1 && near(out[0], out[out.len() - 1]) {
        out.pop();
    }
    if out.len() < 3 {
        return Err(Error::Profile(alloc::format!("{}: braucht mindestens 3 verschiedene Punkte, hat {}", what, out.len())));
    }
    if signed_area2(&out) == 0.0 {
        return Err(Error::Profile(alloc::format!("{}: hat keine Fläche (alle Punkte auf einer Linie?)", what)));
    }
    Ok(out)
}

/// the profile counter-clockwise (reversed if it ran clockwise)
pub fn ccw(mut p: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    if signed_area2(&p) < 0.0 {
        p.reverse();
    }
    p
}

/// cross product of (b - a) and (c - b): positive for a left turn at b
#[inline]
fn turn(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
}

/// whether p lies in the closed triangle a, b, c (counter-clockwise)
#[inline]
fn in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let e = |u: [f64; 2], v: [f64; 2]| (v[0] - u[0]) * (p[1] - u[1]) - (v[1] - u[1]) * (p[0] - u[0]);
    e(a, b) >= 0.0 && e(b, c) >= 0.0 && e(c, a) >= 0.0
}

/// whether two segments cross at a point inside both (touching at an end is no crossing)
fn segments_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let o = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let (d1, d2, d3, d4) = (o(c, d, a), o(c, d, b), o(a, b, c), o(a, b, d));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Err when two edges of the profile cross (it must be simple); O(n^2), profiles are small
pub fn check_simple(p: &[[f64; 2]], what: &str) -> Result<(), Error> {
    let n = p.len();
    for i in 0..n {
        let (a, b) = (p[i], p[(i + 1) % n]);
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue; // neighbours through the closing edge
            }
            let (c, d) = (p[j], p[(j + 1) % n]);
            if segments_cross(a, b, c, d) {
                return Err(Error::Profile(alloc::format!(
                    "{}: die Kanten {} und {} schneiden sich (das Profil muss ein einfaches Vieleck sein)",
                    what, i, j
                )));
            }
        }
    }
    Ok(())
}

/// Triangles of a simple counter-clockwise polygon by ear clipping: n - 2 triangles over its own points,
/// counter-clockwise, none without area (a point on a straight run is never the tip of an ear, so it stays
/// on the boundary and the walls next to a cap meet it).  Err when no ear is left (a self-intersecting or
/// degenerate polygon).  Deterministic: the first ear from where the last one was cut.
pub fn triangulate(p: &[[f64; 2]]) -> Result<Vec<[u32; 3]>, Error> {
    let n = p.len();
    if n < 3 {
        return Err(Error::Profile("ein Vieleck braucht mindestens 3 Punkte".into()));
    }
    let mut idx: Vec<u32> = (0..n as u32).collect();
    let mut out = Vec::with_capacity(n - 2);
    let mut start = 0usize;
    while idx.len() > 3 {
        let m = idx.len();
        let mut cut = None;
        for s in 0..m {
            let i = (start + s) % m;
            let (a, b, c) = (p[idx[(i + m - 1) % m] as usize], p[idx[i] as usize], p[idx[(i + 1) % m] as usize]);
            let la = abs(b[0] - a[0]) + abs(b[1] - a[1]);
            let lc = abs(c[0] - b[0]) + abs(c[1] - b[1]);
            if turn(a, b, c) <= 1e-12 * la * lc {
                continue; // reflex or straight
            }
            let blocked = idx.iter().enumerate().any(|(k, &q)| {
                if k == (i + m - 1) % m || k == i || k == (i + 1) % m {
                    return false;
                }
                let q = p[q as usize];
                q != a && q != b && q != c && in_triangle(q, a, b, c)
            });
            if !blocked {
                cut = Some(i);
                break;
            }
        }
        let Some(i) = cut else {
            return Err(Error::Profile("das Vieleck lässt sich nicht in Dreiecke zerlegen (überschneidet es sich?)".into()));
        };
        let m = idx.len();
        out.push([idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]]);
        idx.remove(i);
        start = if i == 0 { 0 } else { i - 1 };
    }
    let (a, b, c) = (p[idx[0] as usize], p[idx[1] as usize], p[idx[2] as usize]);
    if turn(a, b, c) <= 0.0 {
        return Err(Error::Profile("das Vieleck lässt sich nicht in Dreiecke zerlegen (das letzte Dreieck hat keine Fläche)".into()));
    }
    out.push([idx[0], idx[1], idx[2]]);
    Ok(out)
}

// ---------------------------------------------------------------- profiles

/// a rectangle `w` x `h` centred on the origin, counter-clockwise from the lower left corner
pub fn rect(w: f64, h: f64) -> Vec<[f64; 2]> {
    let (x, y) = (w * 0.5, h * 0.5);
    alloc::vec![[-x, -y], [x, -y], [x, y], [-x, y]]
}

/// a regular polygon of `n` corners on a circle of radius `r` about the origin, counter-clockwise from
/// (r, 0); exact at the axes when n is a multiple of 4
pub fn circle(r: f64, n: usize) -> Vec<[f64; 2]> {
    (0..n)
        .map(|i| {
            let (s, c) = sin_cos_deg(360.0 * i as f64 / n as f64);
            [r * c + 0.0, r * s + 0.0]
        })
        .collect()
}

/// points on an arc of radius `r` about the origin from `from_deg` to `to_deg` (counter-clockwise for
/// to > from), `n` segments, both ends included (n + 1 points): an open polyline for profiles and paths
pub fn arc(r: f64, from_deg: f64, to_deg: f64, n: usize) -> Vec<[f64; 2]> {
    (0..=n)
        .map(|i| {
            let a = from_deg + (to_deg - from_deg) * i as f64 / n as f64;
            let (s, c) = sin_cos_deg(a);
            [r * c + 0.0, r * s + 0.0]
        })
        .collect()
}

/// a rectangle `w` x `h` centred on the origin with corners rounded by radius `r` (at most half the shorter
/// side), `n` segments per corner, counter-clockwise
pub fn rounded_rect(w: f64, h: f64, r: f64, n: usize) -> Vec<[f64; 2]> {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    let (x, y) = (w * 0.5 - r, h * 0.5 - r);
    let mut out = Vec::with_capacity(4 * (n + 1));
    for (k, (cx, cy)) in [(x, -y), (x, y), (-x, y), (-x, -y)].into_iter().enumerate() {
        let start = -90.0 + 90.0 * k as f64;
        for p in arc(r, start, start + 90.0, n.max(1)) {
            out.push([cx + p[0], cy + p[1]]);
        }
    }
    out.dedup();
    out
}
