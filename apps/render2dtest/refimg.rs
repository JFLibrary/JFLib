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

/// an item's centre line in pixels of the target (f64): its points and whether it is closed; an arc flattened so finely
/// that its chords lie within 1/1000 px of the circle (a circle starts at angle 0, an arc at its start, along its sweep)
pub fn centre_line(g: &Geo, cam: &Camera, size: (u32, u32)) -> (Vec<[f64; 2]>, bool) {
    let pt = |q: [f64; 2]| cam.to_screen(q, size);
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
