// refimg -- the reference on the CPU: the same drawing through lib/gfx (dash_stroke's outline, filled non-zero with
// its exact-area coverage, blended in sRGB codes), and how two images are compared.  Arcs are flattened within 1/1000
// px; dash patterns are cut on the CPU in f64 (dash_pieces: SVG's rules, checked against lib/gfx's own dashes on open
// lines) and each dash stroked by dash_stroke with the style's caps and joins.
//
// What may differ, and why (docs/RENDER2D.md "Tests"):
//   - both sides compute the exact area of a pixel inside a polygon (render2d: the box filter of an edge, the square
//     clipped at corners; lib/gfx: signed areas); lib/gfx keeps it as a byte and blends in integers, the GPU in
//     floats: 1 to 2 codes (Tol::STRAIGHT)
//   - a circle: the GPU cuts it by its tangent at the pixel, lib/gfx draws a polygon whose edges lie up to 0.05 px
//     inside it (dash.rs: arc steps of 2 sqrt(0.1 / hw) rad, a sagitta of hw (1 - cos(sqrt(0.1 / hw))) ~ 0.05 px):
//     up to 13 codes along every arc, more at a circle of half a pixel's radius (a hairline's round cap) (Tol::round)
//   - inner joins: lib/gfx's outline pivots through the vertex and its non-zero fill sums the windings of a pixel
//     before it clamps, so where the outline overlaps itself a pixel's share can come out too high, up to full; the
//     GPU computes the union (checked pixel by pixel against the exact share of a supersampled union: GPU 0.6026,
//     exact 0.6025, lib/gfx 1.0) (Tol::corners); a polyline that folds back over itself does the same in many places,
//     its pieces that meet away from a join blend twice on the GPU, once in lib/gfx's fill, and the GPU's union of a
//     short segment between two sharp turns is an approximation (docs/RENDER2D.md) (Tol::folded)
//   - so: almost every pixel within `small` codes, a bounded number beyond it, none beyond `max`, and the ink (the
//     sum of every pixel's distance from the background) within `ink` of the reference's

use alloc::vec;
use alloc::vec::Vec;

use gfx::dash::{dash_stroke, DStyle};
use gfx::path::Path;
use gfx::raster::Raster;
use gfx::{Dc, Surface, FMT_XRGB, RULE_NONZERO};
use render2d::math::{sincos, sqrt};
use render2d::{Camera, Cap, Join, Style, Unit};

use crate::gallery::{Drawing, Geo, Item};

/// the items in draw order: visible layers by order (then as added), within a layer as inserted
fn draw_order(d: &Drawing) -> Vec<&Item> {
    let mut ls: Vec<usize> = (0..d.layers.len()).filter(|&l| d.layers[l].0).collect();
    ls.sort_by_key(|&l| (d.layers[l].1, l));
    let mut out = Vec::new();
    for l in ls {
        out.extend(d.items.iter().filter(|i| i.layer == l));
    }
    out
}

/// a style's width in pixels as render2d draws it (at least one)
pub fn width_px(s: &Style, cam: &Camera) -> f64 {
    let w = match s.unit {
        Unit::Drawing => s.width as f64 / cam.scale,
        _ => s.width as f64,
    };
    w.max(1.0)
}

/// a parametric curve sampled in pixels until every chord lies within 1/2000 px of it (at least 8 pieces per call)
fn sample(f: &dyn Fn(f64) -> [f64; 2], t0: f64, t1: f64, out: &mut Vec<[f64; 2]>) {
    fn rec(f: &dyn Fn(f64) -> [f64; 2], t0: f64, t1: f64, p0: [f64; 2], p1: [f64; 2], depth: u32, out: &mut Vec<[f64; 2]>) {
        let tm = 0.5 * (t0 + t1);
        let pm = f(tm);
        let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
        let l = sqrt(dx * dx + dy * dy);
        let dist = if l > 0.0 { ((pm[0] - p0[0]) * dy - (pm[1] - p0[1]) * dx).abs() / l } else { sqrt((pm[0] - p0[0]) * (pm[0] - p0[0]) + (pm[1] - p0[1]) * (pm[1] - p0[1])) };
        if depth >= 3 && (dist <= 0.0005 || depth >= 26) {
            out.push(p1);
            return;
        }
        rec(f, t0, tm, p0, pm, depth + 1, out);
        rec(f, tm, t1, pm, p1, depth + 1, out);
    }
    let p0 = f(t0);
    if out.last() != Some(&p0) {
        out.push(p0);
    }
    rec(f, t0, t1, p0, f(t1), 0, out);
}

/// B-spline knots as the API describes them when none are given: clamped and uniform, or (closed) periodic with the
/// points wrapped
pub fn spline_knots(deg: usize, pts: &[[f64; 2]], weights: &[f64], knots: &[f64], closed: bool) -> (Vec<[f64; 3]>, Vec<f64>, bool) {
    let w = |k: usize| if weights.is_empty() { 1.0 } else { weights[k] };
    let mut cp: Vec<[f64; 3]> = (0..pts.len()).map(|k| [pts[k][0] * w(k), pts[k][1] * w(k), w(k)]).collect();
    if !knots.is_empty() {
        return (cp, knots.to_vec(), false);
    }
    if closed {
        for k in 0..deg {
            cp.push(cp[k]);
        }
        let u = (0..cp.len() + deg + 1).map(|k| k as f64).collect();
        return (cp, u, true);
    }
    let n = cp.len();
    let mut u = vec![0.0; deg + 1];
    u.extend((1..n - deg).map(|k| k as f64));
    u.extend(core::iter::repeat_n((n - deg) as f64, deg + 1));
    (cp, u, false)
}

/// de Boor's algorithm in homogeneous coordinates: the spline's point at t (independent of render2d's knot insertion)
pub fn de_boor(deg: usize, u: &[f64], cp: &[[f64; 3]], t: f64) -> [f64; 2] {
    let n = cp.len();
    let mut k = deg;
    while k + 1 < n && u[k + 1] <= t {
        k += 1;
    }
    let mut d: Vec<[f64; 3]> = (0..=deg).map(|j| cp[j + k - deg]).collect();
    for r in 1..=deg {
        for j in (r..=deg).rev() {
            let i = j + k - deg;
            let den = u[i + deg + 1 - r] - u[i];
            let a = if den > 0.0 { (t - u[i]) / den } else { 0.0 };
            for c in 0..3 {
                d[j][c] = (1.0 - a) * d[j - 1][c] + a * d[j][c];
            }
        }
    }
    [d[deg][0] / d[deg][2], d[deg][1] / d[deg][2]]
}

/// a curve's points in drawing units as a parametric function per piece: (f, t0, t1) pieces and closed
#[allow(clippy::type_complexity)]
fn pieces_of(g: &Geo) -> (Vec<(alloc::boxed::Box<dyn Fn(f64) -> [f64; 2] + '_>, f64, f64)>, bool) {
    use alloc::boxed::Box;
    let mut v: Vec<(Box<dyn Fn(f64) -> [f64; 2] + '_>, f64, f64)> = Vec::new();
    match g {
        Geo::Bez(p, deg, closed) => {
            let deg = *deg as usize;
            let mut k = 0;
            while k + deg < p.len() {
                let c = &p[k..k + deg + 1];
                v.push((Box::new(move |t: f64| {
                    let s = 1.0 - t;
                    if c.len() == 3 {
                        [s * s * c[0][0] + 2.0 * s * t * c[1][0] + t * t * c[2][0], s * s * c[0][1] + 2.0 * s * t * c[1][1] + t * t * c[2][1]]
                    } else {
                        let (a, b, cc, d) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
                        [a * c[0][0] + b * c[1][0] + cc * c[2][0] + d * c[3][0], a * c[0][1] + b * c[1][1] + cc * c[2][1] + d * c[3][1]]
                    }
                }), 0.0, 1.0));
                k += deg;
            }
            let (a, b) = (p[0], p[p.len() - 1]);
            if *closed && a != b {
                v.push((Box::new(move |t: f64| [b[0] + (a[0] - b[0]) * t, b[1] + (a[1] - b[1]) * t]), 0.0, 1.0));
            }
            (v, *closed)
        }
        Geo::Ellipse(c, r, rot, a0, sweep) => {
            let full = sweep.abs() >= core::f64::consts::TAU;
            let sw = if full { core::f64::consts::TAU.copysign(*sweep) } else { *sweep };
            let (sr, cr) = sincos(*rot);
            let (c, r) = (*c, *r);
            v.push((Box::new(move |t: f64| {
                let (s, co) = sincos(t);
                let (x, y) = (r[0] * co, r[1] * s);
                [c[0] + cr * x - sr * y, c[1] + sr * x + cr * y]
            }), *a0, *a0 + sw));
            (v, full)
        }
        Geo::Spline(deg, p, w, k, closed) => {
            let deg = *deg as usize;
            let (cp, u, periodic) = spline_knots(deg, p, w, k, *closed);
            let n = cp.len();
            let cp: alloc::rc::Rc<Vec<[f64; 3]>> = alloc::rc::Rc::new(cp);
            let u: alloc::rc::Rc<Vec<f64>> = alloc::rc::Rc::new(u);
            for s in deg..n {
                if u[s] < u[s + 1] {
                    let (cp, uu) = (cp.clone(), u.clone());
                    let (t0, t1) = (u[s], u[s + 1]);
                    v.push((Box::new(move |t: f64| de_boor(deg, &uu, &cp, t.min(t1 - (t1 - t0) * 1e-15).max(t0))), t0, t1));
                }
            }
            // (the end: exactly at the domain's end)
            let (cpe, ue) = (cp.clone(), u.clone());
            let end = de_boor(deg, &ue, &cpe, u[n] - (u[n] - u[n - 1]) * 1e-15);
            let start = de_boor(deg, &u, &cp, u[deg]);
            if *closed && !periodic && start != end {
                v.push((Box::new(move |t: f64| [end[0] + (start[0] - end[0]) * t, end[1] + (start[1] - end[1]) * t]), 0.0, 1.0));
            }
            (v, *closed)
        }
        Geo::Arc(c, r, a0, sweep) => {
            let (c, r) = (*c, *r);
            v.push((Box::new(move |t: f64| {
                let (s, co) = sincos(t);
                [c[0] + r * co, c[1] + r * s]
            }), *a0, *a0 + *sweep));
            (v, false)
        }
        Geo::Circle(c, r) => {
            let (c, r) = (*c, *r);
            v.push((Box::new(move |t: f64| {
                let (s, co) = sincos(t);
                [c[0] + r * co, c[1] + r * s]
            }), 0.0, core::f64::consts::TAU));
            (v, true)
        }
        Geo::Line(a, b) => {
            let (a, b) = (*a, *b);
            v.push((Box::new(move |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]), 0.0, 1.0));
            (v, false)
        }
        Geo::Poly(p, _) => {
            for w in p.windows(2) {
                let (a, b) = (w[0], w[1]);
                v.push((Box::new(move |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]), 0.0, 1.0));
            }
            (v, false)
        }
        Geo::Fill(_) => (v, false),
    }
}

/// a curve (or a fill's ring piece) in pixels of the target, sampled within 1/2000 px; closed
pub fn curve_px(g: &Geo, cam: &Camera, size: (u32, u32)) -> (Vec<[f64; 2]>, bool) {
    let (ps, closed) = pieces_of(g);
    let mut out = Vec::new();
    for (f, t0, t1) in &ps {
        let fp = |t: f64| cam.to_screen(f(t), size);
        sample(&fp, *t0, *t1, &mut out);
    }
    if closed && out.len() >= 2 && out[0] == out[out.len() - 1] {
        out.pop();
    }
    (out, closed)
}

/// an item's centre line in pixels of the target (f64): its points and whether it is closed; an arc flattened so finely
/// that its chords lie within 1/1000 px of the circle (a circle starts at angle 0, an arc at its start, along its sweep)
pub fn centre_line(g: &Geo, cam: &Camera, size: (u32, u32)) -> (Vec<[f64; 2]>, bool) {
    let pt = |q: [f64; 2]| cam.to_screen(q, size);
    if matches!(g, Geo::Bez(..) | Geo::Ellipse(..) | Geo::Spline(..)) {
        return curve_px(g, cam, size);
    }
    match g {
        Geo::Line(a, b) => (vec![pt(*a), pt(*b)], false),
        Geo::Poly(v, closed) => (v.iter().map(|q| pt(*q)).collect(), *closed),
        Geo::Arc(..) | Geo::Circle(..) => {
            let (c, r, a0, sweep) = match g {
                Geo::Arc(c, r, a0, sweep) => (c, r, *a0, *sweep),
                Geo::Circle(c, r) => (c, r, 0.0, core::f64::consts::TAU),
                _ => unreachable!(),
            };
            let full = sweep.abs() >= core::f64::consts::TAU;
            let sweep = if full { core::f64::consts::TAU.copysign(sweep) } else { sweep };
            let rp = r / cam.scale;
            let step = 2.0 * sqrt(2.0 * 0.001 / rp.max(1e-3));
            let n = ((sweep.abs() / step) as usize + 1).clamp(8, 200_000);
            let k_end = if full { n - 1 } else { n };
            let pts = (0..=k_end).map(|k| {
                let (s, co) = sincos(a0 + sweep * k as f64 / n as f64);
                pt([c[0] + r * co, c[1] + r * s])
            }).collect();
            (pts, full)
        }
        _ => (Vec::new(), false),
    }
}

/// a style's dash pattern in pixels as lib/gfx and SVG run it (an odd count twice), and its phase; empty: solid
pub fn pattern_px(s: &Style, cam: &Camera) -> (Vec<f64>, f64) {
    let k = if s.dash_unit == Unit::Drawing { 1.0 / cam.scale } else { 1.0 };
    let n = s.dashes as usize;
    let mut v: Vec<f64> = (0..n).map(|i| s.dash[i] as f64 * k).collect();
    if n % 2 == 1 {
        v.extend_from_within(..);
    }
    (v, s.dash_phase as f64 * k)
}

/// the dashes of a centre line (SVG's rules, in f64): the pattern from its start (a closed one's first point) along it;
/// a closed line whose pattern is on at both ends of its way round: the last dash and the first are one, joined at the
/// first point (as Chromium's Skia; lib/gfx's dash_stroke caps them there), all of it on: the closed line.  A dash of
/// length 0 is a single point.  Each piece: its points, closed
pub fn dash_pieces(pts: &[[f64; 2]], closed: bool, pat: &[f64], phase: f64) -> Vec<(Vec<[f64; 2]>, bool)> {
    let mut p: Vec<[f64; 2]> = Vec::new();
    for q in pts {
        if p.last() != Some(q) {
            p.push(*q);
        }
    }
    if closed && p.len() >= 2 && p[0] == p[p.len() - 1] {
        p.pop();
    }
    let period: f64 = pat.iter().sum();
    if p.len() < 2 || !(period > 0.0) {
        return vec![(p, closed)];
    }
    let m = p.len();
    let segs = if closed { m } else { m - 1 };
    // the entry the phase stands in, what is left of it
    let mut x = phase - floor(phase / period) * period;
    let mut k = 0usize;
    while x > pat[k] || (x == pat[k] && pat[k] > 0.0) {
        x -= pat[k];
        k = (k + 1) % pat.len();
    }
    let mut rem = pat[k] - x;
    let mut on = k % 2 == 0;
    let start_on = on;
    let mut out: Vec<(Vec<[f64; 2]>, bool)> = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    let push = |v: &mut Vec<[f64; 2]>, q: [f64; 2]| {
        if v.last() != Some(&q) {
            v.push(q);
        }
    };
    if on {
        cur.push(p[0]);
    }
    for s in 0..segs {
        let (a, b) = (p[s], p[(s + 1) % m]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = sqrt(dx * dx + dy * dy);
        let mut t = 0.0;
        // (the entries that end within this segment)
        while len - t > rem {
            t += rem;
            let q = [a[0] + dx * (t / len), a[1] + dy * (t / len)];
            if on {
                push(&mut cur, q);
                out.push((core::mem::take(&mut cur), false));
            } else {
                cur.clear();
                cur.push(q);
            }
            on = !on;
            k = (k + 1) % pat.len();
            rem = pat[k];
        }
        rem -= len - t;
        if on {
            push(&mut cur, b);
        }
    }
    if on && !cur.is_empty() {
        if closed && start_on {
            if out.is_empty() {
                // (on all the way round)
                cur.pop();
                return vec![(cur, true)];
            }
            // the last dash goes on into the first through the first point
            let first = out.remove(0).0;
            for q in first.into_iter().skip(1) {
                push(&mut cur, q);
            }
        }
        out.push((cur, false));
    }
    out
}

fn floor(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

/// the dashes along a length `len` (px) of an open or closed curve: intervals of the length (SVG's rules as dash_pieces;
/// a closed curve on at both ends: the last and the first are one, past the end: (a, len + b); all on: (0, len) and true)
pub fn dash_intervals(len: f64, closed: bool, pat: &[f64], phase: f64) -> (Vec<(f64, f64)>, bool) {
    let period: f64 = pat.iter().sum();
    if !(period > 0.0) {
        return (vec![(0.0, len)], closed);
    }
    let mut x = phase - floor(phase / period) * period;
    let mut k = 0usize;
    while x > pat[k] || (x == pat[k] && pat[k] > 0.0) {
        x -= pat[k];
        k = (k + 1) % pat.len();
    }
    // the place along the curve where entry k started
    let mut at = -x;
    let start_on = k % 2 == 0;
    let mut out: Vec<(f64, f64)> = Vec::new();
    while at <= len {
        let e = pat[k];
        if k % 2 == 0 {
            let (a, b) = (at.max(0.0), (at + e).min(len));
            if a <= b && (at + e >= 0.0) {
                out.push((a, b));
            }
        }
        at += e;
        k = (k + 1) % pat.len();
    }
    let end_on = out.last().is_some_and(|v| v.1 >= len);
    if closed && start_on && end_on && !out.is_empty() {
        if out.len() == 1 {
            return (vec![(0.0, len)], true);
        }
        let first = out.remove(0);
        let last = out.pop().unwrap_or((len, len));
        out.push((last.0, len + first.1));
    }
    (out, false)
}

/// the outline of an arc's stroke, exactly (no stroker's joins): a circle (c, r in px, screen angles) from angle t0 to t1
/// (either way round), half width hw: the outer arc, the end's cap, the inner arc back, the start's cap; a ring: both
/// circles (the inner one the other way round), a disk where the inner radius is gone
fn arc_outline(p: &mut Path, c: [f64; 2], r: f64, t0: f64, t1: f64, hw: f64, cap: Cap, ring: bool) {
    let at = |t: f64, rad: f64| [c[0] + rad * cos_of(t), c[1] + rad * sin_of(t)];
    let steps = |rad: f64, sweep: f64| ((sweep.abs() / (2.0 * sqrt(2.0 * 0.0005 / rad.max(1e-3)))) as usize + 2).min(400_000);
    let mut pts: Vec<[f64; 2]> = Vec::new();
    let close = |p: &mut Path, pts: &mut Vec<[f64; 2]>| {
        for (k, q) in pts.iter().enumerate() {
            if k == 0 {
                p.move_f(q[0] as f32, q[1] as f32);
            } else {
                p.line_f(q[0] as f32, q[1] as f32);
            }
        }
        p.close();
        pts.clear();
    };
    let tau = core::f64::consts::TAU;
    if ring {
        for (rad, dir) in [(r + hw, 1.0), (r - hw, -1.0)] {
            if rad <= 0.0 {
                continue;
            }
            let n = steps(rad, tau);
            for k in 0..n {
                pts.push(at(dir * tau * k as f64 / n as f64, rad));
            }
            close(p, &mut pts);
        }
        return;
    }
    let sweep = t1 - t0;
    let sg = if sweep < 0.0 { -1.0 } else { 1.0 };
    // the direction of travel at an angle (screen: d/dt of (cos t, sin t), along the sweep)
    let tan = |t: f64| [-sin_of(t) * sg, cos_of(t) * sg];
    let n = steps(r + hw, sweep);
    for k in 0..=n {
        pts.push(at(t0 + sweep * k as f64 / n as f64, r + hw));
    }
    let cap_at = |pts: &mut Vec<[f64; 2]>, t: f64, out: [f64; 2], from: [f64; 2], to: [f64; 2]| {
        let mid = at(t, r);
        match cap {
            Cap::Square => {
                pts.push([from[0] + hw * out[0], from[1] + hw * out[1]]);
                pts.push([to[0] + hw * out[0], to[1] + hw * out[1]]);
            }
            Cap::Round => {
                // half a circle around the end, from `from` over `out` to `to`
                let v = [from[0] - mid[0], from[1] - mid[1]];
                let cross = out[0] * v[1] - out[1] * v[0];
                let turn = if cross > 0.0 { -core::f64::consts::PI } else { core::f64::consts::PI };
                let m = steps(hw, core::f64::consts::PI);
                for k in 1..m {
                    let (sa, ca) = sincos(turn * k as f64 / m as f64);
                    pts.push([mid[0] + v[0] * ca - v[1] * sa, mid[1] + v[0] * sa + v[1] * ca]);
                }
            }
            _ => {}
        }
    };
    let ri = r - hw;
    cap_at(&mut pts, t1, tan(t1), at(t1, r + hw), at(t1, ri));
    let n = steps(ri.abs().max(1e-3), sweep);
    for k in 0..=n {
        pts.push(at(t1 - sweep * k as f64 / n as f64, ri));
    }
    let t = tan(t0);
    cap_at(&mut pts, t0, [-t[0], -t[1]], at(t0, ri), at(t0, r + hw));
    close(p, &mut pts);
}

fn cos_of(t: f64) -> f64 {
    sincos(t).1
}

fn sin_of(t: f64) -> f64 {
    sincos(t).0
}

/// an arc or circle item (drawing units) as the outlines of its stroke and dashes in pixels of the target
fn arc_item(p: &mut Path, g: &Geo, s: &Style, cam: &Camera, size: (u32, u32)) {
    let (c, r, a0, sweep) = match g {
        Geo::Arc(c, r, a0, sweep) => (*c, *r, *a0, *sweep),
        Geo::Circle(c, r) => (*c, *r, 0.0, core::f64::consts::TAU),
        _ => return,
    };
    let full = sweep.abs() >= core::f64::consts::TAU;
    let sweep = if full { core::f64::consts::TAU.copysign(sweep) } else { sweep };
    let cs = cam.to_screen(c, size);
    let rp = r / cam.scale;
    let hw = 0.5 * width_px(s, cam);
    // a drawing angle t is the screen angle -(t + rotation) (y points down there)
    let screen = |t: f64| -(t + cam.rotation);
    let (pat, phase) = pattern_px(s, cam);
    if pat.is_empty() {
        arc_outline(p, cs, rp, screen(a0), screen(a0 + sweep), hw, s.cap, full);
        return;
    }
    let len = rp * sweep.abs();
    let (ivs, ring) = dash_intervals(len, full, &pat, phase);
    if ring {
        arc_outline(p, cs, rp, 0.0, 0.0, hw, s.cap, true);
        return;
    }
    let sg = if sweep < 0.0 { -1.0 } else { 1.0 };
    for (a, b) in ivs {
        arc_outline(p, cs, rp, screen(a0 + sg * a / rp), screen(a0 + sg * b / rp), hw, s.cap, false);
    }
}

/// the drawing as lib/gfx draws it on a background (rows of B, G, R, x): each item's centre line (dashed by dash_pieces)
/// as subpaths of one path, stroked by dash_stroke (caps, joins) and filled non-zero once
pub fn draw(d: &Drawing, cam: &Camera, size: (u32, u32), bg: u32) -> Vec<u8> {
    draw_with(d, cam, size, bg, false)
}

/// gfx_dash: dashes by lib/gfx's own dash_stroke instead of dash_pieces (open lines only: a check of dash_pieces)
pub fn draw_with(d: &Drawing, cam: &Camera, size: (u32, u32), bg: u32, gfx_dash: bool) -> Vec<u8> {
    let (w, h) = (size.0 as i32, size.1 as i32);
    let mut sf = Surface::new(w, h, FMT_XRGB);
    let mut dc = Dc::new(w, h);
    dc.begin(&sf);
    dc.clear(&mut sf, bg);
    let mut ras = Raster::default();
    for it in draw_order(d) {
        let mut p = Path::default();
        let s = &it.style;
        if let Geo::Fill(rings) = &it.geo {
            fill_item(&mut p, rings, cam, size);
            if s.hatches == 0 {
                dc.set_color(s.color);
                p.fill(&mut ras, &dc, &mut sf, gfx::RULE_EVENODD);
            } else {
                // a hatch: the fill's coverage times the lines' (each line stroked by dash_stroke, the union filled), blended
                // as the GPU does
                let mf = mask_of(&mut ras, &p, gfx::RULE_EVENODD, w, h);
                // (the lines' union exactly: crossing lines would add up in lib/gfx's non-zero fill)
                let mut lines = Vec::new();
                hatch_lines(&mut lines, rings, s, cam, size);
                let mh: Vec<u8> = union_mask(&lines, w, h).iter().map(|v| (v * 255.0 + 0.5) as u8).collect();
                let a = (s.color >> 24) as f64 / 255.0;
                let col = [(s.color & 255) as f64, ((s.color >> 8) & 255) as f64, ((s.color >> 16) & 255) as f64];
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let k = y * w as usize + x;
                        let al = a * mf[k] as f64 / 255.0 * mh[k] as f64 / 255.0;
                        if al > 0.0 {
                            let o = y * sf.stride as usize + x * 4;
                            for c in 0..3 {
                                let v = sf.px[o + c] as f64;
                                sf.px[o + c] = (v + (col[c] - v) * al + 0.5) as u8;
                            }
                        }
                    }
                }
            }
            continue;
        }
        if matches!(it.geo, Geo::Arc(..) | Geo::Circle(..)) {
            // (arcs: their outlines exactly, filled; lib/gfx's stroker would pivot the inner side of every vertex of a fine
            // polygon through it, and its non-zero fill sums those windings: a ring 0.25 px too wide inside at a half width
            // of 4.5)
            arc_item(&mut p, &it.geo, s, cam, size);
            dc.set_color(s.color);
            p.fill(&mut ras, &dc, &mut sf, RULE_NONZERO);
            continue;
        }
        let (line, closed) = centre_line(&it.geo, cam, size);
        let (pat, phase) = pattern_px(s, cam);
        let wpx = width_px(s, cam);
        if matches!(it.geo, Geo::Bez(..) | Geo::Ellipse(..) | Geo::Spline(..)) {
            // curves: the exact union (stroke_union), their dashes cut along the fine line as for polylines
            let pieces = if pat.is_empty() { vec![(line, closed)] } else { dash_pieces(&line, closed, &pat, phase) };
            let mut parts = Vec::new();
            for (piece, closed) in &pieces {
                stroke_union(&mut parts, piece, *closed, 0.5 * wpx, s.cap, s.join, s.miter_limit as f64);
            }
            let m = union_mask(&parts, w, h);
            blend_mask(&mut sf, &m, s.color, w, h);
            continue;
        }
        let pieces = if pat.is_empty() || gfx_dash { vec![(line, closed)] } else { dash_pieces(&line, closed, &pat, phase) };
        for (piece, closed) in &pieces {
            for (k, q) in piece.iter().enumerate() {
                if k == 0 {
                    p.move_f(q[0] as f32, q[1] as f32);
                } else {
                    p.line_f(q[0] as f32, q[1] as f32);
                }
            }
            if *closed {
                p.close();
            }
        }
        let cap = match s.cap {
            Cap::Round => gfx::dash::LCAP_ROUND,
            Cap::Square => gfx::dash::LCAP_SQUARE,
            _ => gfx::dash::LCAP_FLAT,
        };
        let join = match s.join {
            Join::Round => gfx::JOIN_ROUND,
            Join::Bevel => gfx::JOIN_BEVEL,
            _ => gfx::JOIN_MITER,
        };
        // (lib/gfx's pattern is in widths)
        let gpat: Vec<f64> = if gfx_dash { pat.iter().map(|v| v / wpx).collect() } else { Vec::new() };
        let st = DStyle { width: wpx, pat: &gpat, join, cap0: cap, cap1: cap, dcap: cap, offset: phase / wpx, miter: s.miter_limit as f64 };
        if dash_stroke(&mut p, &st) != 0 {
            continue;
        }
        dc.set_color(s.color);
        p.fill(&mut ras, &dc, &mut sf, RULE_NONZERO);
    }
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        let at = y * sf.stride as usize;
        out.extend_from_slice(&sf.px[at..at + w as usize * 4]);
    }
    out
}

/// a convex piece of a union: a polygon (either way round) or a disk
pub enum Part {
    Poly(Vec<[f64; 2]>),
    Disk([f64; 2], f64),
}

fn piece(p: &mut Vec<Part>, v: &[[f64; 2]]) {
    p.push(Part::Poly(v.to_vec()));
}

fn disk(p: &mut Vec<Part>, c: [f64; 2], h: f64) {
    p.push(Part::Disk(c, h));
}

/// the union of convex parts as each pixel's share (rows of w), 16 x 16 samples per pixel (1/256: a code at most); a pixel
/// whose corners all lie in one part is wholly inside
pub fn union_mask(parts: &[Part], w: i32, h: i32) -> Vec<f32> {
    let (wu, hu) = (w as usize, h as usize);
    let mut bits = vec![[0u64; 4]; wu * hu];
    let mut full = vec![false; wu * hu];
    for part in parts {
        // (the part's test and box)
        let (poly, c, r) = match part {
            Part::Disk(c, r) => (None, *c, *r),
            Part::Poly(v) => (Some(v), [0.0; 2], 0.0),
        };
        let mut b = [c[0] - r, c[1] - r, c[0] + r, c[1] + r];
        let mut sg = 1.0;
        if let Some(v) = poly {
            let mut a = 0.0;
            for i in 0..v.len() {
                let (q, s) = (v[i], v[(i + 1) % v.len()]);
                a += q[0] * s[1] - s[0] * q[1];
            }
            if a == 0.0 {
                continue;
            }
            sg = if a > 0.0 { 1.0 } else { -1.0 };
            b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
            for q in v {
                b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
            }
        }
        let inside = |x: f64, y: f64| -> bool {
            match poly {
                None => (x - c[0]) * (x - c[0]) + (y - c[1]) * (y - c[1]) <= r * r,
                Some(v) => (0..v.len()).all(|i| {
                    let (q, s) = (v[i], v[(i + 1) % v.len()]);
                    sg * ((s[0] - q[0]) * (y - q[1]) - (s[1] - q[1]) * (x - q[0])) >= 0.0
                }),
            }
        };
        let x0 = (floor(b[0]) as i64).max(0) as usize;
        let y0 = (floor(b[1]) as i64).max(0) as usize;
        let x1 = (floor(b[2]) as i64).min(w as i64 - 1);
        let y1 = (floor(b[3]) as i64).min(h as i64 - 1);
        if x1 < 0 || y1 < 0 {
            continue;
        }
        for y in y0..=y1 as usize {
            for x in x0..=x1 as usize {
                let k = y * wu + x;
                if full[k] {
                    continue;
                }
                let (fx, fy) = (x as f64, y as f64);
                if inside(fx, fy) && inside(fx + 1.0, fy) && inside(fx, fy + 1.0) && inside(fx + 1.0, fy + 1.0) {
                    full[k] = true;
                    continue;
                }
                for j in 0..16 {
                    for i in 0..16 {
                        if inside(fx + (i as f64 + 0.5) / 16.0, fy + (j as f64 + 0.5) / 16.0) {
                            let n = j * 16 + i;
                            bits[k][n / 64] |= 1 << (n % 64);
                        }
                    }
                }
            }
        }
    }
    (0..wu * hu).map(|k| if full[k] { 1.0 } else { bits[k].iter().map(|v| v.count_ones()).sum::<u32>() as f32 / 256.0 }).collect()
}

/// a colour blended over the surface by a mask of shares
fn blend_mask(sf: &mut Surface, m: &[f32], color: u32, w: i32, h: i32) {
    let a = (color >> 24) as f64 / 255.0;
    let col = [(color & 255) as f64, ((color >> 8) & 255) as f64, ((color >> 16) & 255) as f64];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let al = a * m[y * w as usize + x] as f64;
            if al > 0.0 {
                let o = y * sf.stride as usize + x * 4;
                for c in 0..3 {
                    let v = sf.px[o + c] as f64;
                    sf.px[o + c] = (v + (col[c] - v) * al + 0.5) as u8;
                }
            }
        }
    }
}

/// a stroke of a centre line (px) exactly as the union of its pieces (union_mask): per segment its rectangle, per vertex
/// where it turns its join's piece (miter within the limit, else bevel; round: a disk), per open end its cap (square: half a
/// width more of the rectangle, round: a disk).  For curves: lib/gfx's stroker pivots the inner side of every vertex of a
/// fine polygon, and its non-zero fill sums the windings of a pixel before it clamps (the arcs' reason, see the head), so
/// overlapping pieces cannot go through it either
pub fn stroke_union(p: &mut Vec<Part>, pts: &[[f64; 2]], closed: bool, hw: f64, cap: Cap, join: Join, miter: f64) {
    let n = pts.len();
    if n == 0 {
        return;
    }
    if n == 1 {
        match cap {
            Cap::Round => disk(p, pts[0], hw),
            Cap::Square => piece(p, &[[pts[0][0] - hw, pts[0][1] - hw], [pts[0][0] + hw, pts[0][1] - hw], [pts[0][0] + hw, pts[0][1] + hw], [pts[0][0] - hw, pts[0][1] + hw]]),
            _ => {}
        }
        return;
    }
    let segs = if closed { n } else { n - 1 };
    let dir = |k: usize| {
        let (a, b) = (pts[k % n], pts[(k + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = sqrt(dx * dx + dy * dy);
        if l > 0.0 { [dx / l, dy / l] } else { [1.0, 0.0] }
    };
    for k in 0..segs {
        let (a, b) = (pts[k], pts[(k + 1) % n]);
        let d = dir(k);
        let nn = [-d[1] * hw, d[0] * hw];
        let (mut a2, mut b2) = (a, b);
        if !closed && cap == Cap::Square {
            if k == 0 {
                a2 = [a[0] - d[0] * hw, a[1] - d[1] * hw];
            }
            if k + 1 == segs {
                b2 = [b[0] + d[0] * hw, b[1] + d[1] * hw];
            }
        }
        piece(p, &[[a2[0] + nn[0], a2[1] + nn[1]], [b2[0] + nn[0], b2[1] + nn[1]], [b2[0] - nn[0], b2[1] - nn[1]], [a2[0] - nn[0], a2[1] - nn[1]]]);
    }
    // the joins
    let joints: Vec<usize> = if closed { (0..n).collect() } else { (1..n - 1).collect() };
    for v in joints {
        let (d1, d2) = (dir((v + n - 1) % n), dir(v));
        let cr = d1[0] * d2[1] - d1[1] * d2[0];
        let c = d1[0] * d2[0] + d1[1] * d2[1];
        if cr == 0.0 && c > 0.0 {
            continue;
        }
        let q = pts[v];
        if join == Join::Round && !(cr.abs() < 0.05 && c > 0.0) {
            disk(p, q, hw);
            continue;
        }
        // (a round join turning by less than 0.05 rad: its bevel, h (1 - cos(phi / 2)) below 4e-4 h from the arc)
        let s = if cr > 0.0 { -1.0 } else { 1.0 };
        let n1 = [-d1[1] * s, d1[0] * s];
        let n2 = [-d2[1] * s, d2[0] * s];
        let (o1, o2) = ([q[0] + n1[0] * hw, q[1] + n1[1] * hw], [q[0] + n2[0] * hw, q[1] + n2[1] * hw]);
        let cos_half = sqrt(((1.0 + c) * 0.5).max(0.0));
        if join == Join::Miter && cos_half > 0.0 && 1.0 / cos_half <= miter {
            let (mx, my) = (n1[0] + n2[0], n1[1] + n2[1]);
            let ml = sqrt(mx * mx + my * my);
            let tip = [q[0] + mx / ml * hw / cos_half, q[1] + my / ml * hw / cos_half];
            piece(p, &[q, o1, tip, o2]);
        } else {
            piece(p, &[q, o1, o2]);
        }
    }
    if !closed && cap == Cap::Round {
        disk(p, pts[0], hw);
        disk(p, pts[n - 1], hw);
    }
}

/// a fill's rings in pixels as the subpaths of a path (each piece sampled within 1/2000 px, end to end, closed)
fn fill_item(p: &mut Path, rings: &[Vec<Geo>], cam: &Camera, size: (u32, u32)) {
    for ring in rings {
        let mut pts: Vec<[f64; 2]> = Vec::new();
        for piece in ring {
            let (v, _) = curve_px(piece, cam, size);
            for q in v {
                if pts.last() != Some(&q) {
                    pts.push(q);
                }
            }
        }
        for (k, q) in pts.iter().enumerate() {
            if k == 0 {
                p.move_f(q[0] as f32, q[1] as f32);
            } else {
                p.line_f(q[0] as f32, q[1] as f32);
            }
        }
        p.close();
    }
}

/// a path's exact-area coverage per pixel (rows of w bytes)
fn mask_of(ras: &mut Raster, p: &Path, rule: u32, w: i32, h: i32) -> Vec<u8> {
    let mut m = vec![0u8; (w * h) as usize];
    p.fill_lines(ras);
    ras.fill_rows([0, 0, w, h], rule, &mut |y, x, cov| {
        let o = y as usize * w as usize + x as usize;
        for (k, c) in cov.iter().enumerate() {
            if o + k < m.len() {
                m[o + k] = m[o + k].max(*c);
            }
        }
    });
    m
}

/// a style's hatch lines over the fill's box as rectangles in pixels (render2d's model: line k through base + k (shift
/// dir + spacing n), its pattern from s = (p - base) . dir - k shift, a dash shorter than the line's width as long as it)
pub fn hatch_lines(p: &mut Vec<Part>, rings: &[Vec<Geo>], s: &render2d::Style, cam: &Camera, size: (u32, u32)) {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for ring in rings {
        for piece in ring {
            let (v, _) = curve_px(piece, cam, size);
            for q in v {
                let d = cam.to_drawing(q, size);
                b = [b[0].min(d[0]), b[1].min(d[1]), b[2].max(d[0]), b[3].max(d[1])];
            }
        }
    }
    let hw = (0.5 * s.hatch_width as f64).max(0.5);
    let corners = [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]];
    for h in &s.hatch[..s.hatches as usize] {
        let (sa, ca) = sincos(h.angle);
        let (dir, n) = ([ca, sa], [-sa, ca]);
        let proj = |q: [f64; 2], v: [f64; 2]| (q[0] - h.base[0]) * v[0] + (q[1] - h.base[1]) * v[1];
        let (mut u0, mut u1, mut s0, mut s1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for c in corners {
            u0 = u0.min(proj(c, n));
            u1 = u1.max(proj(c, n));
            s0 = s0.min(proj(c, dir));
            s1 = s1.max(proj(c, dir));
        }
        let m = h.dashes as usize;
        let pat: Vec<f64> = (0..if m % 2 == 1 { 2 * m } else { m }).map(|k| h.dash[k % m]).collect();
        let period: f64 = pat.iter().sum();
        let k0 = floor(u0 / h.spacing) as i64 - 1;
        let k1 = floor(u1 / h.spacing) as i64 + 1;
        for k in k0..=k1 {
            let kf = k as f64;
            let at = |sv: f64| {
                let q = [h.base[0] + kf * h.spacing * n[0] + (sv + kf * h.shift) * dir[0], h.base[1] + kf * h.spacing * n[1] + (sv + kf * h.shift) * dir[1]];
                cam.to_screen(q, size)
            };
            // (the along coordinate of this line runs from s0 - k shift - margin)
            let (a, e) = (s0 - kf * h.shift - 2.0 * h.spacing - period, s1 - kf * h.shift + 2.0 * h.spacing + period);
            let mut seg = |x0: f64, x1: f64| {
                // (the dash as its rectangle, hw on either side)
                let (q0, q1) = (at(x0), at(x1));
                let (dx, dy) = (q1[0] - q0[0], q1[1] - q0[1]);
                let l = sqrt(dx * dx + dy * dy);
                if l > 0.0 {
                    let n = [-dy / l * hw, dx / l * hw];
                    p.push(Part::Poly(vec![[q0[0] + n[0], q0[1] + n[1]], [q1[0] + n[0], q1[1] + n[1]], [q1[0] - n[0], q1[1] - n[1]], [q0[0] - n[0], q0[1] - n[1]]]));
                }
            };
            if period <= 0.0 {
                seg(a, e);
                continue;
            }
            let mut m0 = floor(a / period) * period;
            while m0 < e {
                let mut c0 = 0.0;
                for (i, v) in pat.iter().enumerate() {
                    let c1 = c0 + v;
                    if i % 2 == 0 {
                        let (mut x0, mut x1) = (m0 + c0, m0 + c1);
                        let wu = 2.0 * hw * cam.scale;
                        if x1 - x0 < wu {
                            let mid = 0.5 * (x0 + x1);
                            x0 = mid - 0.5 * wu;
                            x1 = mid + 0.5 * wu;
                        }
                        seg(x0, x1);
                    }
                    c0 = c1;
                }
                m0 += period;
            }
        }
    }
}

/// what may differ (see the file's head)
#[derive(Clone, Copy, Debug)]
pub struct Tol {
    pub small: u8,
    pub over: u32,
    pub max: u8,
    pub ink: f64,
}

impl Tol {
    /// straight edges and polygonal corners: what blending in integers against floats costs
    pub const STRAIGHT: Tol = Tol { small: 2, over: 8, max: 4, ink: 0.001 };

    /// round caps and joins (n of them): along the arcs 16 codes, beyond that at most 4 pixels each
    pub const fn round(n: u32) -> Tol {
        Tol { small: 16, over: 4 * n, max: 48, ink: 0.006 }
    }

    /// inner joins and tips (n of them): 24 pixels each beyond 2 codes, at most 96 codes (lib/gfx's winding sum)
    pub const fn corners(n: u32) -> Tol {
        Tol { small: 2, over: 24 * n, max: 96, ink: 0.005 }
    }

    /// random polylines (n entities) that fold back over themselves
    pub const fn folded(n: u32) -> Tol {
        Tol { small: 2, over: 30 + 24 * n, max: 128, ink: 0.01 }
    }

    /// arcs and circles: their rings cut by the tangent at the pixel (see the head)
    pub const ARCS: Tol = Tol { small: 16, over: 400, max: 64, ink: 0.006 };

    /// dash patterns: the ends of dashes straight lines; near joins and with round caps more (see the head)
    pub const DASH: Tol = Tol { small: 2, over: 400, max: 96, ink: 0.006 };
    pub const DASH_ROUND: Tol = Tol { small: 16, over: 600, max: 96, ink: 0.02 };
}

/// two images side by side
#[derive(Clone, Copy, Debug, Default)]
pub struct Cmp {
    /// pixels beyond `small` codes in a channel, the largest difference, where
    pub over: u32,
    pub max: u8,
    pub at: (u32, u32),
    /// the sum of every pixel's distance from the background (three channels), GPU and reference
    pub ink: [f64; 2],
}

impl Cmp {
    pub fn fine(&self, t: &Tol) -> bool {
        let ink = (self.ink[0] - self.ink[1]).abs() <= t.ink * self.ink[1].max(1.0);
        self.over <= t.over && self.max <= t.max && ink
    }

    pub fn text(&self) -> alloc::string::String {
        alloc::format!("{} pixels beyond, largest {} at {:?}, ink {:.0} against {:.0} ({:+.3} %)", self.over, self.max, self.at, self.ink[0], self.ink[1],
                       100.0 * (self.ink[0] - self.ink[1]) / self.ink[1].max(1.0))
    }
}

/// rows of B, G, R, x of both (the fourth byte means nothing)
pub fn compare(gpu: &[u8], reference: &[u8], w: u32, bg: u32, small: u8) -> Cmp {
    let mut c = Cmp::default();
    let bgc = [(bg & 255) as i32, ((bg >> 8) & 255) as i32, ((bg >> 16) & 255) as i32];
    for (i, (p, q)) in gpu.chunks_exact(4).zip(reference.chunks_exact(4)).enumerate() {
        let mut d = 0u8;
        for k in 0..3 {
            d = d.max(p[k].abs_diff(q[k]));
            c.ink[0] += (p[k] as i32 - bgc[k]).abs() as f64;
            c.ink[1] += (q[k] as i32 - bgc[k]).abs() as f64;
        }
        if d > small {
            c.over += 1;
        }
        if d > c.max {
            c.max = d;
            c.at = (i as u32 % w, i as u32 / w);
        }
    }
    c
}
