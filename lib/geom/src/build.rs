// build -- the generators: primitives (box, cylinder, cone, sphere, torus, stairs, arch) and the operations
// that make solids from profiles (extrude, prism, sweep along a path, lathe).  Every one gives a closed
// mesh whose faces point outward, with normals and box-projected UV in metres.  Every primitive stands on
// the ground: its lowest point at y = 0, centred over the origin (the rule a script relies on to stack
// parts); profiles are counter-clockwise or not, the generators turn them as needed.

use alloc::vec;
use alloc::vec::Vec;

use crate::math::{Vec3, abs, sin_cos_deg};
use crate::mesh::Mesh;
use crate::poly;
use crate::{Error, MAX_SEGMENTS, MAX_TRIANGLES};

/// the threshold in degrees for the normals of extrusions, sweeps and lathes: edges sharper than it stay hard
pub const DEFAULT_SMOOTH_DEG: f64 = 30.0;

/// a positive finite length, else Err naming it
fn length(v: f64, what: &str) -> Result<f64, Error> {
    if v > 0.0 && v.is_finite() { Ok(v) } else { Err(Error::Param(alloc::format!("{} muss eine Zahl größer 0 sein, nicht {}", what, num(v)))) }
}

/// a segment count in lo ..= MAX_SEGMENTS, else Err naming it
fn segments(n: usize, lo: usize, what: &str) -> Result<usize, Error> {
    if n >= lo && n <= MAX_SEGMENTS { Ok(n) } else { Err(Error::Param(alloc::format!("{} muss zwischen {} und {} liegen, nicht {}", what, lo, MAX_SEGMENTS, n))) }
}

/// a number for a message
fn num(v: f64) -> alloc::string::String {
    alloc::format!("{}", v)
}

/// normals and UV for a raw mesh: smooth with the threshold (or flat for 0), box UV in metres
fn finish(raw: Mesh, smooth_deg: f64) -> Mesh {
    let m = if smooth_deg <= 0.0 { raw.flat() } else { raw.smooth(smooth_deg) };
    m.uv_box(1.0)
}

/// A box `w` x `h` x `d` (x, y, z) standing on y = 0, centred over the origin: 8 corners, 12 triangles, flat
/// normals (24 vertices), box UV in metres.
pub fn cuboid(w: f64, h: f64, d: f64) -> Result<Mesh, Error> {
    let (w, h, d) = (length(w, "die Breite")?, length(h, "die Höhe")?, length(d, "die Tiefe")?);
    let (x, z) = (w * 0.5, d * 0.5);
    let p = vec![
        Vec3::new(-x, 0.0, -z),
        Vec3::new(x, 0.0, -z),
        Vec3::new(x, 0.0, z),
        Vec3::new(-x, 0.0, z),
        Vec3::new(-x, h, -z),
        Vec3::new(x, h, -z),
        Vec3::new(x, h, z),
        Vec3::new(-x, h, z),
    ];
    #[rustfmt::skip]
    let idx = vec![
        0, 1, 2, 0, 2, 3, // bottom (-y)
        4, 7, 6, 4, 6, 5, // top (+y)
        3, 2, 6, 3, 6, 7, // front (+z)
        1, 0, 4, 1, 4, 5, // back (-z)
        2, 1, 5, 2, 5, 6, // right (+x)
        0, 3, 7, 0, 7, 4, // left (-x)
    ];
    Ok(finish(Mesh::from_triangles(p, idx)?, 0.0))
}

/// the threshold that keeps the round sides of a primitive whose faces turn by up to `step` degrees from one
/// to the next smooth and its rims hard; below 85 degrees, so a very coarse one (a three-sided cylinder)
/// stays flat rather than getting normals that lean away from its faces
fn round_threshold(step: f64) -> f64 {
    (step * 1.01).clamp(DEFAULT_SMOOTH_DEG, 85.0)
}

/// A cylinder of radius `r` and height `h` on y = 0 about the Y axis, `n` segments: smooth sides, flat caps.
pub fn cylinder(r: f64, h: f64, n: usize) -> Result<Mesh, Error> {
    let (r, h, n) = (length(r, "der Radius")?, length(h, "die Höhe")?, segments(n, 3, "die Zahl der Segmente")?);
    lathe_raw(&[[0.0, 0.0], [r, 0.0], [r, h], [0.0, h]], n).map(|m| finish(m, round_threshold(360.0 / n as f64)))
}

/// A cone (or with `top` > 0 a frustum) of base radius `r`, top radius `top` and height `h` on y = 0 about
/// the Y axis, `n` segments.
pub fn cone(r: f64, h: f64, n: usize, top: f64) -> Result<Mesh, Error> {
    let (r, h, n) = (length(r, "der Radius")?, length(h, "die Höhe")?, segments(n, 3, "die Zahl der Segmente")?);
    if !(top >= 0.0 && top.is_finite()) {
        return Err(Error::Param(alloc::format!("der obere Radius muss 0 oder größer sein, nicht {}", num(top))));
    }
    let prof: Vec<[f64; 2]> = if top > 0.0 { vec![[0.0, 0.0], [r, 0.0], [top, h], [0.0, h]] } else { vec![[0.0, 0.0], [r, 0.0], [0.0, h]] };
    lathe_raw(&prof, n).map(|m| finish(m, round_threshold(360.0 / n as f64)))
}

/// A UV sphere of radius `r` resting on y = 0 (centre at y = r), `n` segments around and `stacks` from pole
/// to pole, smooth (flat where a step is 85 degrees or more).
pub fn sphere(r: f64, n: usize, stacks: usize) -> Result<Mesh, Error> {
    let (r, n, stacks) = (length(r, "der Radius")?, segments(n, 3, "die Zahl der Segmente")?, segments(stacks, 2, "die Zahl der Ringe")?);
    let prof: Vec<[f64; 2]> = (0..=stacks)
        .map(|i| {
            let (s, c) = sin_cos_deg(180.0 * i as f64 / stacks as f64);
            [r * s + 0.0, r - r * c]
        })
        .collect();
    lathe_raw(&prof, n).map(|m| finish(m, round_threshold((360.0 / n as f64).max(180.0 / stacks as f64))))
}

/// A torus lying flat on y = 0 about the Y axis: `major` from the axis to the middle of the tube, tube
/// radius `minor` (less than `major`), `n` segments around the axis and `sides` around the tube, smooth
/// (flat where a step is 85 degrees or more).
pub fn torus(major: f64, minor: f64, n: usize, sides: usize) -> Result<Mesh, Error> {
    let (major, minor) = (length(major, "der große Radius")?, length(minor, "der kleine Radius")?);
    let (n, sides) = (segments(n, 3, "die Zahl der Segmente")?, segments(sides, 3, "die Zahl der Seiten")?);
    if minor >= major {
        return Err(Error::Param(alloc::format!("der kleine Radius ({}) muss kleiner als der große ({}) sein", num(minor), num(major))));
    }
    let prof: Vec<[f64; 2]> = poly::circle(minor, sides).into_iter().map(|p| [major + p[0], minor + p[1]]).collect();
    lathe_raw(&prof, n).map(|m| finish(m, round_threshold((360.0 / n as f64).max(360.0 / sides as f64))))
}

/// Stairs `w` wide (x), `h` high and `d` deep (z) with `steps` steps on y = 0, centred over the origin: the
/// first riser at the front (z = d / 2), rising towards the back, a solid block under the steps.  Flat.
pub fn stairs(w: f64, h: f64, d: f64, steps: usize) -> Result<Mesh, Error> {
    let (w, h, d) = (length(w, "die Breite")?, length(h, "die Höhe")?, length(d, "die Tiefe")?);
    let steps = segments(steps, 1, "die Zahl der Stufen")?;
    // the profile in (z, y): up and back step by step, down the back, along the floor
    let mut prof = Vec::with_capacity(2 * steps + 2);
    let (rise, run) = (h / steps as f64, d / steps as f64);
    for i in 0..steps {
        let z = d * 0.5 - run * i as f64;
        prof.push([z, rise * i as f64]);
        prof.push([z, rise * (i + 1) as f64]);
    }
    prof.push([-d * 0.5, h]);
    prof.push([-d * 0.5, 0.0]);
    let path = [Vec3::new(-w * 0.5, 0.0, 0.0), Vec3::new(w * 0.5, 0.0, 0.0)];
    sweep_raw(&prof, &path, false).map(|m| finish(m, 0.0))
}

/// An arch `w` wide (x), `h` high and `d` deep (z) on y = 0, centred over the origin: two piers
/// `thickness` thick, the opening below a half circle of `n` segments, `thickness` of material over its
/// crown.  The opening is `w - 2 thickness` wide and needs room for the half circle.
pub fn arch(w: f64, h: f64, d: f64, thickness: f64, n: usize) -> Result<Mesh, Error> {
    let (w, h, d) = (length(w, "die Breite")?, length(h, "die Höhe")?, length(d, "die Tiefe")?);
    let t = length(thickness, "die Stärke")?;
    let n = segments(n, 2, "die Zahl der Segmente")?;
    let r = w * 0.5 - t;
    let spring = h - t - r; // where the half circle starts
    if !(r > 0.0) || !(spring >= 0.0) {
        return Err(Error::Param(alloc::format!(
            "der Bogen passt nicht: Breite {} und Stärke {} lassen eine Öffnung von {} m, deren Halbkreis höher als {} m wäre",
            num(w), num(t), num(2.0 * r), num(h - t)
        )));
    }
    let x = w * 0.5;
    let mut prof = vec![[-x, 0.0], [-r, 0.0]];
    if spring > 0.0 {
        prof.push([-r, spring]);
    }
    for i in 1..n {
        let (s, c) = sin_cos_deg(180.0 - 180.0 * i as f64 / n as f64);
        prof.push([r * c + 0.0, spring + r * s]);
    }
    if spring > 0.0 {
        prof.push([r, spring]);
    }
    prof.extend_from_slice(&[[r, 0.0], [x, 0.0], [x, h], [-x, h]]);
    extrude(&prof, d)
}

/// The profile (x, y; the front view) extruded `depth` along Z, centred on z = 0: the front cap at
/// z = depth / 2.  Smooth up to [`DEFAULT_SMOOTH_DEG`].
pub fn extrude(profile: &[[f64; 2]], depth: f64) -> Result<Mesh, Error> {
    let depth = length(depth, "die Tiefe")?;
    let path = [Vec3::new(0.0, 0.0, depth * 0.5), Vec3::new(0.0, 0.0, -depth * 0.5)];
    sweep_raw(profile, &path, false).map(|m| finish(m, DEFAULT_SMOOTH_DEG))
}

/// The footprint (x, z; the top view) raised `height` along Y from y = 0: walls from a floor plan.
pub fn prism(footprint: &[[f64; 2]], height: f64) -> Result<Mesh, Error> {
    let height = length(height, "die Höhe")?;
    let path = [Vec3::ZERO, Vec3::new(0.0, height, 0.0)];
    sweep_raw(footprint, &path, false).map(|m| finish(m, DEFAULT_SMOOTH_DEG))
}

/// The profile swept along a path of points.  The profile's x runs to the right of the path, y up (the
/// world's Y where the path is not vertical; for a vertical path x is X and y is Z), seen looking along
/// the path; frames turn with the path without twisting (rotation-minimising), corners are mitred, the ends
/// capped.  A closed path joins its last point to the first (no caps; a twist the loop leaves is spread
/// over its segments).  Smooth up to `smooth_deg` degrees (0: flat).
pub fn sweep(profile: &[[f64; 2]], path: &[Vec3], closed: bool, smooth_deg: f64) -> Result<Mesh, Error> {
    sweep_raw(profile, path, closed).map(|m| finish(m, smooth_deg))
}

/// The profile (radius, height) turned about the Y axis in `n` segments: a solid of revolution.  The
/// profile is a closed polygon in the half plane radius >= 0; edges on the axis make no faces, points on it
/// become poles, so a profile from the axis around to the axis gives a solid without a hole (a column, a
/// vase), one away from the axis a ring.  Smooth up to `smooth_deg` degrees (0: flat).
pub fn lathe(profile: &[[f64; 2]], n: usize, smooth_deg: f64) -> Result<Mesh, Error> {
    let n = segments(n, 3, "die Zahl der Segmente")?;
    lathe_raw(profile, n).map(|m| finish(m, smooth_deg))
}

// ---------------------------------------------------------------- raw generators

/// the lathe's positions and triangles (normals not yet set)
fn lathe_raw(profile: &[[f64; 2]], n: usize) -> Result<Mesh, Error> {
    if profile.iter().any(|p| p[0] < 0.0) {
        return Err(Error::Profile("Drehkörper: ein Radius ist negativ (das Profil muss rechts der Achse liegen)".into()));
    }
    let prof = poly::ccw(poly::clean(profile, "Drehkörper")?);
    poly::check_simple(&prof, "Drehkörper")?;
    let m = prof.len();
    let rings: usize = prof.iter().map(|p| if p[0] == 0.0 { 1 } else { n }).sum();
    if m.saturating_mul(n).saturating_mul(2) > MAX_TRIANGLES {
        return Err(Error::TooBig(alloc::format!("Drehkörper: {} Punkte mal {} Segmente sind zu viele Dreiecke", m, n)));
    }
    // the angles once: exact at the axes for n a multiple of 4
    let sc: Vec<(f64, f64)> = (0..n).map(|i| sin_cos_deg(360.0 * i as f64 / n as f64)).collect();
    let mut pos = Vec::with_capacity(rings);
    let mut first = Vec::with_capacity(m);
    for p in &prof {
        first.push(pos.len() as u32);
        if p[0] == 0.0 {
            pos.push(Vec3::new(0.0, p[1], 0.0));
        } else {
            for &(s, c) in &sc {
                pos.push(Vec3::new(p[0] * c, p[1], p[0] * s).unsigned_zeros());
            }
        }
    }
    let at = |j: usize, i: usize| -> u32 { if prof[j][0] == 0.0 { first[j] } else { first[j] + (i % n) as u32 } };
    let mut idx = Vec::with_capacity(6 * m * n);
    for j in 0..m {
        let j1 = (j + 1) % m;
        let (pole0, pole1) = (prof[j][0] == 0.0, prof[j1][0] == 0.0);
        if pole0 && pole1 {
            continue; // on the axis
        }
        for i in 0..n {
            let (a, b, c, d) = (at(j, i), at(j1, i), at(j1, i + 1), at(j, i + 1));
            if pole0 {
                idx.extend_from_slice(&[a, b, c]);
            } else if pole1 {
                idx.extend_from_slice(&[a, c, d]);
            } else {
                idx.extend_from_slice(&[a, b, c, a, c, d]);
            }
        }
    }
    Mesh::from_triangles(pos, idx)
}

/// the turn taking unit vector a to unit vector b about a x b, applied to v (Rodrigues with the vectors'
/// own sine and cosine, no trigonometry); v unchanged when a and b point the same way
fn turn_like(a: Vec3, b: Vec3, v: Vec3) -> Vec3 {
    let axis = a.cross(b);
    let s = axis.length();
    if s == 0.0 {
        return v;
    }
    let k = axis / s;
    let c = a.dot(b);
    v * c + k.cross(v) * s + k * (k.dot(v) * (1.0 - c))
}

/// the sweep's positions and triangles (normals not yet set)
fn sweep_raw(profile: &[[f64; 2]], path_in: &[Vec3], closed: bool) -> Result<Mesh, Error> {
    let prof = poly::ccw(poly::clean(profile, "Profil")?);
    poly::check_simple(&prof, "Profil")?;
    let caps = poly::triangulate(&prof)?;
    if path_in.len() > poly::MAX_PROFILE_POINTS {
        return Err(Error::TooBig(alloc::format!("Pfad: mehr als {} Punkte", poly::MAX_PROFILE_POINTS)));
    }
    if !path_in.iter().all(|p| p.is_finite()) {
        return Err(Error::Path("Pfad: eine Koordinate ist nicht endlich".into()));
    }
    // points closer than MERGE_RATIO of the path's extent count as one
    let ext = path_in.iter().skip(1).fold(path_in.first().map(|&p| crate::math::Aabb::point(p)), |b, &p| b.map(|b| b.union(crate::math::Aabb::point(p))));
    let eps = ext.map_or(0.0, |b| poly::MERGE_RATIO * (b.size().x + b.size().y + b.size().z));
    let near = |a: Vec3, b: Vec3| abs(a.x - b.x) <= eps && abs(a.y - b.y) <= eps && abs(a.z - b.z) <= eps;
    let mut path: Vec<Vec3> = Vec::with_capacity(path_in.len());
    for &p in path_in {
        if path.last().is_none_or(|&l| !near(l, p)) {
            path.push(p);
        }
    }
    if closed {
        while path.len() > 1 && near(path[0], path[path.len() - 1]) {
            path.pop();
        }
    }
    let k = path.len();
    if k < 2 || (closed && k < 3) {
        return Err(Error::Path(alloc::format!("Pfad: braucht mindestens {} verschiedene Punkte, hat {}", if closed { 3 } else { 2 }, k)));
    }
    let segs = if closed { k } else { k - 1 };
    let m = prof.len();
    if m.saturating_mul(segs).saturating_mul(2) + 2 * caps.len() > MAX_TRIANGLES {
        return Err(Error::TooBig(alloc::format!("Sweep: {} Punkte mal {} Abschnitte sind zu viele Dreiecke", m, segs)));
    }
    let tan: Vec<Vec3> = (0..segs).map(|s| (path[(s + 1) % k] - path[s]).normalize_or_zero()).collect();
    for s in 0..segs {
        let next = if closed { Some((s + 1) % segs) } else if s + 1 < segs { Some(s + 1) } else { None };
        if let Some(n1) = next {
            if tan[s].dot(tan[n1]) < -0.99 {
                return Err(Error::Path(alloc::format!("Pfad: am Punkt {} knickt er um mehr als 172 Grad zurück", (s + 1) % k)));
            }
        }
    }
    // the frame of segment 0, then turned along without twisting
    let t0 = tan[0];
    let up = if abs(t0.y) < 0.999 { Vec3::Y } else { Vec3::Z };
    let u0 = (up - t0 * up.dot(t0)).normalize_or_zero();
    let mut frames: Vec<(Vec3, Vec3)> = Vec::with_capacity(segs); // (right, up) of each segment
    frames.push((t0.cross(u0), u0));
    for s in 1..segs {
        let (r, u) = frames[s - 1];
        frames.push((turn_like(tan[s - 1], tan[s], r), turn_like(tan[s - 1], tan[s], u)));
    }
    if closed {
        // the twist the loop leaves at segment 0, spread evenly: segment s turns back by s / segs of it
        let (r_end, _) = frames[segs - 1];
        let r_back = turn_like(tan[segs - 1], tan[0], r_end);
        let (r0, _) = frames[0];
        let twist = crate::math::atan2(r0.cross(r_back).dot(t0), r0.dot(r_back)) / (core::f64::consts::PI / 180.0);
        for (s, f) in frames.iter_mut().enumerate().skip(1) {
            let a = -twist * s as f64 / segs as f64;
            if let Some(rot) = crate::math::Affine::rotation_about(tan[s], a) {
                *f = (rot.apply_vector(f.0), rot.apply_vector(f.1));
            }
        }
    }
    // the rings: at a joint the profile in the outgoing segment's frame, cut by the mitre plane
    let rings = k;
    let mut pos = Vec::with_capacity(rings * m);
    for j in 0..rings {
        let p = path[j];
        let (seg, plane) = if closed {
            (j, Some(((j + segs - 1) % segs, j)))
        } else if j == 0 {
            (0, None)
        } else if j == k - 1 {
            (segs - 1, None)
        } else {
            (j, Some((j - 1, j)))
        };
        let (r, u) = frames[seg];
        let t = tan[seg];
        let mitre = plane.map(|(a, b)| (tan[a] + tan[b]).normalize_or_zero());
        for q in &prof {
            let off = r * q[0] + u * q[1];
            let at = match mitre {
                Some(mn) if mn != Vec3::ZERO => p + off - t * (off.dot(mn) / t.dot(mn)),
                _ => p + off,
            };
            pos.push(at.unsigned_zeros());
        }
    }
    let ring = |j: usize, i: usize| -> u32 { ((j % rings) * m + i % m) as u32 };
    let mut idx = Vec::with_capacity(6 * m * segs + 6 * caps.len());
    for s in 0..segs {
        for i in 0..m {
            let (a0, b0, a1, b1) = (ring(s, i), ring(s, i + 1), ring(s + 1, i), ring(s + 1, i + 1));
            idx.extend_from_slice(&[a0, a1, b1, a0, b1, b0]);
        }
    }
    if !closed {
        for c in &caps {
            idx.extend_from_slice(&[ring(0, c[0] as usize), ring(0, c[1] as usize), ring(0, c[2] as usize)]);
        }
        for c in &caps {
            idx.extend_from_slice(&[ring(k - 1, c[0] as usize), ring(k - 1, c[2] as usize), ring(k - 1, c[1] as usize)]);
        }
    }
    Mesh::from_triangles(pos, idx)
}
