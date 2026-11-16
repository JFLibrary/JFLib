// cpu -- the CPU path (G6e1, decision 29; docs/RENDER2D.md "Der CPU-Weg"): a scene drawn through lib/gfx's rasterizer
// without a device, with the camera, the painter's order, the visible layers and the look of a frame on the GPU.  A child
// of scene.rs: it reads the scene's entities, layers, styles, blocks, texts, highlights and overlay, and its R-tree (pick.rs).
//
// A frame: the view's box in the drawing (f64), grown by the widest reach in pixels, through the R-tree; the entities found
// sorted into the painter's order (layer, then place in the layer), inserts walked down into their blocks through the
// inverse transforms as pick.rs walks them (the copies of an array that meet the view, each block's own tree, nested
// inserts).  Every point goes from its units to pixels relative to the view in f64 (a block's through its insert's
// transform, the camera's centre taken off before anything is scaled), so ±1e7 at 1e-6 units per pixel stays exact; centre
// lines are clipped in f64 to the target grown by the stroke's reach, and every piece handed to lib/gfx is clipped to the
// target and GUARD px around it: lib/gfx sees small f32 numbers only.  Each entity is one fill of lib/gfx (exact area
// coverage by signed accumulation), blended once in its colour as the GPU blends it (premultiplied sRGB codes).
//
// Strokes are convex pieces whose areas add up to the union the GPU computes: per segment its strip, cut at the bisector of
// each join where the cut fits both segments (miter: the tip comes from the cut, bevel: its line cuts too, round: the
// perpendicular through the vertex, the outer sector a piece of its own), else the GPU's union |a| + |w| + |b| - |a ∩ b|
// (both strips end at the perpendicular, the join's piece beyond both, their intersection subtracted: the accumulation's
// sign takes it); caps as planes (butt, square) or half disks.  Arcs are flattened within ARC_TOL px only where they come
// near the view; curves and fills as the GPU flattens them (the scene's flattening while it holds for the camera, else the
// same function for this camera; a block's per zoom band), fills even-odd from oriented rings.  Dash patterns cut in f64
// along each entity's lengths, flat tones and mixes at the GPU's thresholds; hatches clipped to the fill in f64 and
// multiplied by its coverage as the GPU does; text from its glyphs' outlines in G6c's layout, bars below TEXT_BAR_BELOW px
// per em; highlights and the overlay as G6d draws them.

use alloc::vec;
use alloc::vec::Vec;

use gfx::raster::Raster;
use gfx::{Dc, Surface, RULE_EVENODD, RULE_NONZERO};

use super::overlay;
use super::pick::{copies_meeting, copy_xf, inv_box};
use super::{flat_whole, hl, stroke_reach, Cap, FillG, FlatCtx, Geom, Insert, Join, Scene, Style, TextG, Transparency, Unit, FILL_REACH, HATCH_MAX, NONE, TEXT_BAR_BELOW};
use crate::block::{self, Fam, Xf};
use crate::curve;
use crate::geo::{self, Bx};
use crate::math::{sincos, sqrt};
use crate::msdf::Seg;

type P = [f64; 2];

/// arcs, round caps and joins, glyph outlines: chords within this many pixels of the curve
const ARC_TOL: f64 = 1.0 / 128.0;
/// what every piece handed to lib/gfx is clipped to: the target and this many pixels around it
const GUARD: f64 = 2.0;
/// a dash pattern of a period up to DASH_FLAT pixels is a flat tone of its duty, from DASH_EXACT on exact, mixed between
/// (stroke.wgsl); hatch lines by their spacing likewise (fill.wgsl)
const DASH_FLAT: f64 = 2.0;
const DASH_EXACT: f64 = 4.0;
const LINE_FLAT: f64 = 2.0;
const LINE_EXACT: f64 = 4.0;

/// what the last draw_cpu did (Scene::draw_cpu, cpu_info).  It may grow
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuInfo {
    /// entities on layers the index found near the view (an insert once), and what was drawn: entities on layers, each copy
    /// of a block's entity in an insert once
    pub found: u32,
    pub drawn: u32,
    /// fills lib/gfx made (an entity one, a bar of small text one, a hatch's coverages more) and the lines it rasterized
    pub fills: u32,
    pub lines: u64,
    /// curves and fills flattened on the way (the scene's flattening was made for another zoom band or window; a block's
    /// curves and fills each time)
    pub flattened: u32,
    /// overlay items drawn
    pub overlay: u32,
}

/// what the CPU path keeps between frames: lib/gfx's rasterizer and scratch buffers (no geometry); one thread only
pub(crate) struct CpuState {
    ras: Raster,
    info: CpuInfo,
    cov: [Vec<f32>; 3],
    single: bool,
}

impl Default for CpuState {
    fn default() -> CpuState {
        CpuState { ras: Raster::default(), info: CpuInfo::default(), cov: Default::default(), single: false }
    }
}

/// the most row bands drawn apart on lib/gfx's parallel loop (each band a whole frame clipped to its rows), a band at least
/// this many rows high
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const BANDS: i32 = 16;
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const BAND_ROWS: i32 = 32;

/// a point of an entity's units -> pixels of the view (x right, y down from its top left), in f64: off + lin (p - sub)
#[derive(Clone, Copy, Debug)]
struct Place {
    sub: P,
    lin: [[f64; 2]; 2],
    off: P,
}

impl Place {
    fn px(&self, p: P) -> P {
        let (x, y) = (p[0] - self.sub[0], p[1] - self.sub[1]);
        [self.off[0] + self.lin[0][0] * x + self.lin[0][1] * y, self.off[1] + self.lin[1][0] * x + self.lin[1][1] * y]
    }

    fn vec(&self, v: P) -> P {
        [self.lin[0][0] * v[0] + self.lin[0][1] * v[1], self.lin[1][0] * v[0] + self.lin[1][1] * v[1]]
    }

    /// the map's largest stretch (its largest singular value)
    fn smax(&self) -> f64 {
        let (a, b, c, d) = (self.lin[0][0], self.lin[0][1], self.lin[1][0], self.lin[1][1]);
        let s1 = a * a + b * b + c * c + d * d;
        let dt = (a * d - b * c).abs();
        sqrt(0.5 * (s1 + sqrt((s1 * s1 - 4.0 * dt * dt).max(0.0))))
    }
}

/// where an entity is drawn: its place; its units per pixel for widths and patterns in units (the camera's scale, inside an
/// insert through sqrt |det|); its layer's transparency; its highlight; an insert's zoom band for its block's curves and
/// fills (None: on a layer, the scene's flattening); the overlay's frame
#[derive(Clone, Copy)]
struct Where {
    place: Place,
    upp: f64,
    sw: f64,
    layer_t: u32,
    hl: u8,
    band: Option<i32>,
    ovl: bool,
    /// an insert's transform from its block's units (less its base) into the drawing
    xf: Option<(Xf, P)>,
}

/// a stroke's pen in pixels
#[derive(Clone, Copy)]
struct Pen {
    h: f64,
    cap: Cap,
    join: Join,
    limit: f64,
}

impl Pen {
    /// how far its pieces reach beyond the centre line (px): a miter up to its limit, a square cap's corner, and a margin
    fn reach(&self) -> f64 {
        let b = if self.join == Join::Miter { self.limit.max(core::f64::consts::SQRT_2) } else { core::f64::consts::SQRT_2 };
        self.h * (b + 1.0) + 2.0
    }
}

/// a run of a centre line in pixels, clipped to a box: its points, per segment its lengths along the entity (local units:
/// where a dash pattern stands), whether its ends were cut (no cap, no join there), closed (nothing cut)
#[derive(Clone, Default)]
struct Run {
    pts: Vec<P>,
    segl: Vec<(f64, f64)>,
    open_s: bool,
    open_e: bool,
    closed: bool,
}

/// a convex polygon of a few corners (a stroke's piece)
#[derive(Clone, Copy)]
struct Poly {
    v: [P; 24],
    n: usize,
}

impl Poly {
    fn of(p: &[P]) -> Poly {
        let mut q = Poly { v: [[0.0; 2]; 24], n: 0 };
        for &x in p.iter().take(24) {
            q.v[q.n] = x;
            q.n += 1;
        }
        q
    }

    fn pts(&self) -> &[P] {
        &self.v[..self.n]
    }

    /// what lies on the side (p - q) . k >= 0 (Sutherland-Hodgman, one plane)
    fn clip(&self, q: P, k: P) -> Poly {
        let mut out = Poly { v: [[0.0; 2]; 24], n: 0 };
        let side = |p: P| (p[0] - q[0]) * k[0] + (p[1] - q[1]) * k[1];
        for i in 0..self.n {
            let (a, b) = (self.v[i], self.v[(i + 1) % self.n]);
            let (sa, sb) = (side(a), side(b));
            if sa >= 0.0 && out.n < 24 {
                out.v[out.n] = a;
                out.n += 1;
            }
            if (sa >= 0.0) != (sb >= 0.0) && out.n < 24 {
                out.v[out.n] = lerp(a, b, sa / (sa - sb));
                out.n += 1;
            }
        }
        out
    }

    /// the part inside another convex polygon (its corners in either turn)
    fn inside(&self, o: &Poly) -> Poly {
        let s = if area(o.pts()) >= 0.0 { 1.0 } else { -1.0 };
        let mut r = *self;
        for i in 0..o.n {
            let (a, b) = (o.v[i], o.v[(i + 1) % o.n]);
            r = r.clip(a, [-(b[1] - a[1]) * s, (b[0] - a[0]) * s]);
            if r.n == 0 {
                break;
            }
        }
        r
    }
}

/// twice the signed area (shoelace)
fn area(p: &[P]) -> f64 {
    let mut a = 0.0;
    for i in 0..p.len() {
        let (u, v) = (p[i], p[(i + 1) % p.len()]);
        a += u[0] * v[1] - v[0] * u[1];
    }
    a
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}

fn mul(a: P, s: f64) -> P {
    [a[0] * s, a[1] * s]
}

fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn len(a: P) -> f64 {
    sqrt(a[0] * a[0] + a[1] * a[1])
}

fn rot(d: P) -> P {
    [-d[1], d[0]]
}

fn lerp(a: P, b: P, t: f64) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn floor(x: f64) -> f64 {
    if !(x.abs() < 4.5e15) {
        return x;
    }
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

fn round(x: f64) -> f64 {
    floor(x + 0.5)
}

/// x - floor(x / p) p, in [0, p)
fn modp(x: f64, p: f64) -> f64 {
    let r = x - floor(x / p) * p;
    if (0.0..p).contains(&r) { r } else { 0.0 }
}

/// a colour with its transparency (percent; by layer: the layer's) multiplied into its alpha in whole numbers, as block.wgsl
/// ink does (by block outside a block: as it is)
fn ink(color: u32, t: Transparency, layer_t: u32) -> u32 {
    let p = match t {
        Transparency::ByLayer => layer_t,
        Transparency::Percent(p) => p,
        _ => return color,
    };
    if p == 0 || p > 100 {
        return color;
    }
    let a = color >> 24;
    (color & 0xFF_FFFF) | (((a * (100 - p) + 50) / 100) << 24)
}

/// a highlighted colour (hl.wgsl hl_ink): toward HL_HOVER_RGB by 224 / 256, else toward HL_SELECTED_RGB by 192 / 256, alpha
/// at least HL_ALPHA
fn hl_ink(c: u32, bits: u8) -> u32 {
    if bits == 0 {
        return c;
    }
    let (t, k) = if bits & hl::HL_HOVER != 0 { (hl::HL_HOVER_RGB, 224u32) } else { (hl::HL_SELECTED_RGB, 192u32) };
    let ch = |s: u32| ((((c >> s) & 255) * (256 - k) + ((t >> s) & 255) * k + 128) >> 8) << s;
    ((c >> 24).max(hl::HL_ALPHA) << 24) | ch(16) | ch(8) | ch(0)
}

/// a highlight's extra half width (hl.wgsl hl_wider)
fn hl_wider(bits: u8) -> f64 {
    if bits & hl::HL_HOVER != 0 {
        1.0
    } else if bits != 0 {
        0.25
    } else {
        0.0
    }
}

/// a colour's alpha times f (0 ..= 1), rounded
fn alpha_times(c: u32, f: f64) -> u32 {
    let a = ((c >> 24) as f64 * f.clamp(0.0, 1.0) + 0.5) as u32;
    (c & 0xFF_FFFF) | (a.min(255) << 24)
}

/// Liang-Barsky: the part of a -> b inside box bx (x0, y0, x1, y1) as parameters, None: none
fn clip_seg(a: P, b: P, bx: &Bx) -> Option<(f64, f64)> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, a[0] - bx[0]), (dx, bx[2] - a[0]), (-dy, a[1] - bx[1]), (dy, bx[3] - a[1])] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            if r > t1 {
                return None;
            }
            t0 = t0.max(r);
        } else {
            if r < t0 {
                return None;
            }
            t1 = t1.min(r);
        }
    }
    Some((t0, t1))
}

/// a centre line (pixels; the lengths along the entity at its points; the length all around when closed) cut into the runs
/// that lie in box bx.  A closed line cut elsewhere than at its first point is one run through that point
fn runs_of(px: &[P], lens: &[f64], closed: bool, total: f64, bx: &Bx, out: &mut Vec<Run>) {
    out.clear();
    let n = px.len();
    if n == 0 {
        return;
    }
    if n == 1 {
        let p = px[0];
        if p[0] >= bx[0] && p[0] <= bx[2] && p[1] >= bx[1] && p[1] <= bx[3] {
            out.push(Run { pts: vec![p], segl: Vec::new(), open_s: false, open_e: false, closed: false });
        }
        return;
    }
    let segs = if closed { n } else { n - 1 };
    let mut cut = false;
    let mut cur: Option<Run> = None;
    for i in 0..segs {
        let (a, b) = (px[i], px[(i + 1) % n]);
        let la = lens[i];
        let lb = if i + 1 == n { total } else { lens[i + 1] };
        let Some((t0, t1)) = clip_seg(a, b, bx) else {
            if let Some(r) = cur.take() {
                out.push(r);
            }
            cut = true;
            continue;
        };
        if t0 > 0.0 || cur.is_none() {
            if let Some(r) = cur.take() {
                out.push(r);
            }
            cut |= t0 > 0.0;
            let start = if t0 > 0.0 { lerp(a, b, t0) } else { a };
            // (an open line's first point takes its cap; every other start lies on the box's edge, or is a closed line's
            // first point, joined below)
            cur = Some(Run { pts: vec![start], segl: Vec::new(), open_s: t0 > 0.0 || i > 0 || closed, open_e: false, closed: false });
        }
        let Some(r) = cur.as_mut() else { continue };
        r.segl.push((la + t0 * (lb - la), la + t1 * (lb - la)));
        if t1 < 1.0 {
            r.pts.push(lerp(a, b, t1));
            r.open_e = true;
            cut = true;
            if let Some(r) = cur.take() {
                out.push(r);
            }
        } else {
            r.pts.push(b);
        }
    }
    if let Some(r) = cur.take() {
        out.push(r);
    }
    if !closed {
        return;
    }
    if !cut && out.len() == 1 {
        // the whole closed line: its last point is its first again
        let r = &mut out[0];
        r.pts.pop();
        r.closed = true;
        r.open_s = false;
        r.open_e = false;
        return;
    }
    let k = out.len();
    if k >= 2 && !out[k - 1].open_e && out[k - 1].pts.last() == Some(&px[0]) && out[0].pts.first() == Some(&px[0]) {
        let first = out.remove(0);
        let last = &mut out[k - 2];
        last.pts.extend_from_slice(&first.pts[1..]);
        last.segl.extend_from_slice(&first.segl);
        last.open_e = first.open_e;
    }
}

/// what lib/gfx gets of an entity: polygons clipped to the guard box (f64), as f32 lines in the target's pixels
struct Em<'a> {
    ras: &'a mut Raster,
    /// the guard box in pixels of the view; where the view's top left lies in the target
    g: Bx,
    at: P,
    lines: &'a mut u64,
    tmp: &'a mut Vec<P>,
    tmp2: &'a mut Vec<P>,
}

impl Em<'_> {
    fn line(&mut self, a: P, b: P) {
        self.ras.emit_line((a[0] + self.at[0]) as f32, (a[1] + self.at[1]) as f32, (b[0] + self.at[0]) as f32, (b[1] + self.at[1]) as f32);
        *self.lines += 1;
    }

    /// a polygon as it is (a ring of a fill, a glyph's contour: its turn kept), clipped to the guard box
    fn ring(&mut self, p: &[P]) {
        self.poly_dir(p, false);
    }

    /// a piece of a stroke, turned so that its area has the sign of `sign` (+1 adds coverage, -1 takes it off)
    fn piece(&mut self, p: &[P], sign: f64) {
        if p.len() < 3 {
            return;
        }
        let a = area(p);
        if a == 0.0 {
            return;
        }
        self.poly_dir(p, a * sign < 0.0);
    }

    fn poly_dir(&mut self, p: &[P], rev: bool) {
        let n = p.len();
        if n < 2 {
            return;
        }
        let b = geo::bx_of(p);
        let g = self.g;
        if b[2] < g[0] || b[0] > g[2] || b[3] < g[1] || b[1] > g[3] {
            return;
        }
        if b[0] >= g[0] && b[2] <= g[2] && b[1] >= g[1] && b[3] <= g[3] {
            for i in 0..n {
                let (u, v) = (p[i], p[(i + 1) % n]);
                if rev { self.line(v, u) } else { self.line(u, v) }
            }
            return;
        }
        // Sutherland-Hodgman against the guard box's four sides (the area inside it and its turn are kept)
        let mut a = core::mem::take(self.tmp);
        let mut c = core::mem::take(self.tmp2);
        a.clear();
        a.extend_from_slice(p);
        for (q, k) in [([g[0], 0.0], [1.0, 0.0]), ([g[2], 0.0], [-1.0, 0.0]), ([0.0, g[1]], [0.0, 1.0]), ([0.0, g[3]], [0.0, -1.0])] {
            c.clear();
            let m = a.len();
            let side = |x: P| (x[0] - q[0]) * k[0] + (x[1] - q[1]) * k[1];
            for i in 0..m {
                let (u, v) = (a[i], a[(i + 1) % m]);
                let (su, sv) = (side(u), side(v));
                if su >= 0.0 {
                    c.push(u);
                }
                if (su >= 0.0) != (sv >= 0.0) {
                    c.push(lerp(u, v, su / (su - sv)));
                }
            }
            core::mem::swap(&mut a, &mut c);
            if a.is_empty() {
                break;
            }
        }
        let m = a.len();
        for i in 0..m {
            let (u, v) = (a[i], a[(i + 1) % m]);
            if rev { self.line(v, u) } else { self.line(u, v) }
        }
        *self.tmp = a;
        *self.tmp2 = c;
    }

    /// a disk
    fn disk(&mut self, c: P, h: f64, sign: f64) {
        let mut v = Vec::new();
        let q = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]];
        for k in 0..4 {
            arc_pts(c, h, q[k], q[(k + 1) % 4], &mut v);
            v.pop();
        }
        self.piece(&v, sign);
    }
}

/// a circle's arc around c of radius h from unit direction u1 to u2 (the shorter way; more than a right angle: through
/// their middle) as points into out, within ARC_TOL: halving the angle, no trigonometry.  u1's point first, u2's last
fn arc_pts(c: P, h: f64, u1: P, u2: P, out: &mut Vec<P>) {
    fn rec(c: P, h: f64, a: P, b: P, depth: u32, out: &mut Vec<P>) {
        // (the chord's sagitta h (1 - cos(phi / 2)), with cos(phi / 2) = |a + b| / 2)
        let m = add(a, b);
        let ml = len(m);
        if h * (1.0 - 0.5 * ml) <= ARC_TOL || depth >= 20 || ml == 0.0 {
            out.push(add(c, mul(b, h)));
            return;
        }
        let mid = mul(m, 1.0 / ml);
        rec(c, h, a, mid, depth + 1, out);
        rec(c, h, mid, b, depth + 1, out);
    }
    out.push(add(c, mul(u1, h)));
    if dot(u1, u2) < 0.0 {
        let s = if cross(u1, u2) >= 0.0 { 1.0 } else { -1.0 };
        let m = add(u1, u2);
        let ml = len(m);
        let m = if ml > 1e-9 { mul(m, 1.0 / ml) } else { mul(rot(u1), s) };
        rec(c, h, u1, m, 0, out);
        rec(c, h, m, u2, 0, out);
    } else {
        rec(c, h, u1, u2, 0, out);
    }
}

// ---------------------------------------------------------------- strokes

/// a run's stroke as pieces into em (the file's head): cap_s, cap_e: its ends take the pen's cap (else they were cut:
/// butt); dot_dir: a single point's square's direction (none: the pixels' axes, as the vertex shader turns it)
fn stroke_run(em: &mut Em, pts_in: &[P], closed_in: bool, cap_s: bool, cap_e: bool, pen: &Pen, dot_dir: Option<P>) {
    let mut pts: Vec<P> = Vec::with_capacity(pts_in.len());
    for &p in pts_in {
        if pts.last() != Some(&p) {
            pts.push(p);
        }
    }
    let closed = closed_in;
    if closed && pts.len() >= 2 && pts[0] == pts[pts.len() - 1] {
        pts.pop();
    }
    let n = pts.len();
    let h = pen.h;
    if n == 0 {
        return;
    }
    if n == 1 {
        // a point: its caps (round: a disk; square: a square along dot_dir or the pixels' axes)
        let p = pts[0];
        match pen.cap {
            Cap::Round => em.disk(p, h, 1.0),
            Cap::Square => {
                let d = dot_dir.unwrap_or([1.0, 0.0]);
                let (a, b) = (mul(d, h), mul(rot(d), h));
                em.piece(&[add(add(p, a), b), sub(add(p, a), b), sub(sub(p, a), b), add(sub(p, a), b)], 1.0);
            }
            _ => {}
        }
        return;
    }
    let segs = if closed { n } else { n - 1 };
    let mut d: Vec<P> = Vec::with_capacity(segs);
    let mut sl: Vec<f64> = Vec::with_capacity(segs);
    for i in 0..segs {
        let v = sub(pts[(i + 1) % n], pts[i]);
        let l = len(v);
        sl.push(l);
        d.push(if l > 0.0 { mul(v, 1.0 / l) } else { [1.0, 0.0] });
    }
    // the joints: vertex v between segment v - 1 and v (a closed line: every vertex; an open one: the inner ones).  A turn's
    // inner side (+1: towards rot(d)), its cut's length along both segments (h tan(phi / 2)), a miter within its limit
    let prev = |v: usize| if v == 0 { segs - 1 } else { v - 1 };
    let mut turn = vec![false; n];
    let mut cutl = vec![0.0f64; n];
    let mut side = vec![1.0f64; n];
    let mut miter = vec![false; n];
    let mut valid = vec![true; n];
    for v in 0..n {
        if !closed && (v == 0 || v == n - 1) {
            continue;
        }
        let (d1, d2) = (d[prev(v)], d[v % segs]);
        let (cr, dt) = (cross(d1, d2), dot(d1, d2));
        if cr.abs() <= 1e-12 && dt > 0.0 {
            continue;
        }
        turn[v] = true;
        side[v] = if cr >= 0.0 { 1.0 } else { -1.0 };
        let ch = sqrt(((1.0 + dt) * 0.5).max(0.0));
        cutl[v] = if 1.0 + dt > 1e-12 { h * cr.abs() / (1.0 + dt) } else { f64::INFINITY };
        miter[v] = pen.join == Join::Miter && ch > 0.0 && 1.0 / ch <= pen.limit;
    }
    // a cut at the bisector fits when its chord lies in both segments (and in a segment both its cuts on one side)
    for i in 0..segs {
        let (vs, ve) = (i, (i + 1) % n);
        let (cs, ce) = (if turn[vs] { cutl[vs] } else { 0.0 }, if turn[ve] { cutl[ve] } else { 0.0 });
        if cs > sl[i] {
            valid[vs] = false;
        }
        if ce > sl[i] {
            valid[ve] = false;
        }
        if turn[vs] && turn[ve] && side[vs] == side[ve] && cs + ce > sl[i] {
            valid[vs] = false;
            valid[ve] = false;
        }
    }
    let ext = h * (2.0 + sqrt((pen.limit * pen.limit - 1.0).max(0.0)));
    let mut tiles: Vec<Poly> = Vec::with_capacity(segs);
    for i in 0..segs {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let di = d[i];
        let nn = mul(rot(di), h);
        let (a2, b2) = (sub(a, mul(di, ext)), add(b, mul(di, ext)));
        let mut t = Poly::of(&[add(a2, nn), sub(a2, nn), sub(b2, nn), add(b2, nn)]);
        let (vs, ve) = (i, (i + 1) % n);
        if !closed && i == 0 {
            let q = if cap_s && pen.cap == Cap::Square { sub(a, mul(di, h)) } else { a };
            t = t.clip(q, di);
        } else {
            t = cut_tile(t, a, d[prev(vs)], di, turn[vs], valid[vs], miter[vs], side[vs], pen, true);
        }
        if !closed && i + 1 == segs {
            let q = if cap_e && pen.cap == Cap::Square { add(b, mul(di, h)) } else { b };
            t = t.clip(q, mul(di, -1.0));
        } else {
            t = cut_tile(t, b, di, d[ve % segs], turn[ve], valid[ve], miter[ve], side[ve], pen, false);
        }
        em.piece(t.pts(), 1.0);
        tiles.push(t);
    }
    // the joins' own pieces: a round join's outer sector; where the cut did not fit, the join's piece beyond both strips and
    // their intersection taken off
    let mut arc = Vec::new();
    for v in 0..n {
        if !turn[v] {
            continue;
        }
        let (pi, ni) = (prev(v), v % segs);
        let (d1, d2) = (d[pi], d[ni]);
        let p = pts[v];
        let (n1, n2) = (mul(rot(d1), -side[v]), mul(rot(d2), -side[v]));
        let (o1, o2) = (add(p, mul(n1, h)), add(p, mul(n2, h)));
        if pen.join == Join::Round {
            arc.clear();
            arc.push(p);
            arc_pts(p, h, n1, n2, &mut arc);
            em.piece(&arc, 1.0);
        } else if !valid[v] {
            if miter[v] {
                em.piece(&[p, o1, add(p, mul(add(n1, n2), h / (1.0 + dot(d1, d2)))), o2], 1.0);
            } else {
                em.piece(&[p, o1, o2], 1.0);
            }
        }
        if !valid[v] {
            let x = tiles[pi].inside(&tiles[ni]);
            if x.n >= 3 {
                em.piece(x.pts(), -1.0);
            }
        }
    }
    // round caps: half disks beyond the open ends (from one normal over the back to the other)
    if !closed && pen.cap == Cap::Round {
        if cap_s {
            let (p, di) = (pts[0], d[0]);
            arc.clear();
            arc_pts(p, h, rot(di), mul(di, -1.0), &mut arc);
            arc.pop();
            arc_pts(p, h, mul(di, -1.0), mul(rot(di), -1.0), &mut arc);
            em.piece(&arc, 1.0);
        }
        if cap_e {
            let (p, di) = (pts[n - 1], d[segs - 1]);
            arc.clear();
            arc_pts(p, h, mul(rot(di), -1.0), di, &mut arc);
            arc.pop();
            arc_pts(p, h, di, rot(di), &mut arc);
            em.piece(&arc, 1.0);
        }
    }
}

/// a tile cut at a joint p between directions d1 (in) and d2 (out); start: the tile of the segment after p, else before
#[allow(clippy::too_many_arguments)]
fn cut_tile(t: Poly, p: P, d1: P, d2: P, turn: bool, valid: bool, miter: bool, side: f64, pen: &Pen, start: bool) -> Poly {
    let own = if start { d2 } else { mul(d1, -1.0) };
    let bis = add(d1, d2);
    if !turn || !valid || len(bis) <= 1e-12 {
        return t.clip(p, own);
    }
    // the bisector (its normal d1 + d2 along the line)
    let mut t = t.clip(p, if start { bis } else { mul(bis, -1.0) });
    match pen.join {
        Join::Round => t = t.clip(p, own),
        _ if !miter => {
            // the bevel's line through the outer corners, the vertex's side kept
            let (n1, n2) = (mul(rot(d1), -side), mul(rot(d2), -side));
            let m = add(n1, n2);
            let ml = len(m);
            if ml > 1e-12 {
                t = t.clip(add(p, mul(n1, pen.h)), mul(m, -1.0 / ml));
            }
        }
        _ => {}
    }
    t
}

/// a style's dash pattern in an entity's units: the entries' ends (an odd count twice), the period, the phase, the share
/// on and the dashes per period
struct Pattern {
    ends: [f64; 16],
    n: usize,
    period: f64,
    phase: f64,
    on: f64,
    ons: f64,
}

/// upp: the entity's units per pixel (a pattern in pixels); None: solid
fn pattern_of(st: &Style, upp: f64) -> Option<Pattern> {
    let m = (st.dashes as usize).min(crate::scene::DASH_MAX);
    if m == 0 {
        return None;
    }
    let n = if m % 2 == 1 { 2 * m } else { m };
    let k = if st.dash_unit == Unit::Px { upp } else { 1.0 };
    // the GPU's numbers (Style::words): the entries' ends as f32 of their sums, the period the last of them, the phase as an
    // f32 fraction of a period (at 1e12 periods the last bits of the period decide where a dash lies)
    let mut p = Pattern { ends: [0.0; 16], n, period: 0.0, phase: 0.0, on: 0.0, ons: 0.0 };
    let mut sum = 0.0f64;
    for i in 0..n {
        let v = st.dash[i % m] as f64;
        sum += v;
        p.ends[i] = (sum as f32) as f64;
        if i % 2 == 0 {
            p.on += v;
            p.ons += 1.0;
        }
    }
    let period = p.ends[n - 1];
    if !(period > 0.0) {
        return None;
    }
    let ph = st.dash_phase as f64 / period;
    p.phase = ((ph - floor(ph)) as f32) as f64 * period * k;
    for e in &mut p.ends[..n] {
        *e *= k;
    }
    p.on *= k;
    p.period = period * k;
    Some(p)
}

/// the pattern's dashes on a segment from length la over l (local units): (start, end) within [0, l]
fn on_intervals(la: f64, l: f64, pat: &Pattern, last: bool, out: &mut Vec<(f64, f64)>) {
    out.clear();
    let x = modp(la + pat.phase, pat.period);
    let mut k = 0usize;
    while k < pat.n && pat.ends[k] <= x {
        k += 1;
    }
    if k == pat.n {
        k = 0;
    }
    let mut start = if k == 0 { 0.0 } else { pat.ends[k - 1] };
    let mut pos = start - x.min(pat.ends[k]);
    let mut guard = 0u32;
    while pos <= l && guard < 1_000_000 {
        guard += 1;
        let e = pat.ends[k] - start;
        if k % 2 == 0 {
            if e <= 0.0 {
                if pos >= 0.0 && (pos < l || (last && pos <= l)) {
                    out.push((pos, pos));
                }
            } else {
                let (s0, s1) = (pos.max(0.0), (pos + e).min(l));
                if s1 > s0 {
                    out.push((s0, s1));
                }
            }
        }
        pos += e;
        k += 1;
        if k == pat.n {
            k = 0;
        }
        start = if k == 0 { 0.0 } else { pat.ends[k - 1] };
    }
}

/// a dash: its points, its caps, closed (a closed line on all the way round), a dot's direction
struct Dash {
    pts: Vec<P>,
    cap_s: bool,
    cap_e: bool,
    closed: bool,
    dir: Option<P>,
}

/// a run cut into its dashes (SVG's rules; a closed line on at both ends of its way round: the last dash and the first are
/// one, joined at the first point, as the GPU and Skia join them)
fn dash_run(run: &Run, pat: &Pattern, out: &mut Vec<Dash>) {
    out.clear();
    let n = run.pts.len();
    let segs = run.segl.len();
    if n == 0 {
        return;
    }
    if segs == 0 {
        // a single point: on where the pattern is on at its length
        return;
    }
    let mut ivs = Vec::new();
    let mut cur: Option<Dash> = None;
    let mut first_at0 = false;
    let mut reached_end = false;
    for i in 0..segs {
        let (a, b) = (run.pts[i], run.pts[(i + 1) % n]);
        let (la, lb) = run.segl[i];
        let l = lb - la;
        let dv = sub(b, a);
        let dl = len(dv);
        if !(l > 0.0) || dl == 0.0 {
            continue;
        }
        let dir = mul(dv, 1.0 / dl);
        on_intervals(la, l, pat, i + 1 == segs && !run.closed, &mut ivs);
        if ivs.first().is_none_or(|v| v.0 > 0.0) {
            if let Some(c) = cur.take() {
                out.push(Dash { cap_e: true, ..c });
            }
        }
        for (k, &(s0, s1)) in ivs.iter().enumerate() {
            let (p0, p1) = (lerp(a, b, s0 / l), lerp(a, b, s1 / l));
            match cur.as_mut() {
                Some(c) if s0 == 0.0 && k == 0 => c.pts.push(p1),
                _ => {
                    if let Some(c) = cur.take() {
                        out.push(Dash { cap_e: true, ..c });
                    }
                    if i == 0 && s0 == 0.0 {
                        first_at0 = true;
                    }
                    cur = Some(Dash { pts: vec![p0, p1], cap_s: !(i == 0 && s0 == 0.0 && run.open_s), cap_e: true, closed: false, dir: Some(dir) });
                }
            }
            if s1 < l {
                if let Some(c) = cur.take() {
                    out.push(Dash { cap_e: true, ..c });
                }
            }
        }
        reached_end = cur.is_some();
    }
    if let Some(c) = cur.take() {
        if run.closed && first_at0 && reached_end {
            if out.is_empty() {
                // (on all the way round)
                out.push(Dash { closed: true, cap_s: false, cap_e: false, ..c });
                return;
            }
            let first = out.remove(0);
            let mut pts = c.pts;
            pts.extend_from_slice(&first.pts[1..]);
            out.push(Dash { pts, cap_s: c.cap_s, cap_e: first.cap_e, closed: false, dir: c.dir });
            return;
        }
        out.push(Dash { cap_e: !run.open_e, ..c });
    }
}

// ---------------------------------------------------------------- a frame

/// a frame's target and scratch
struct Frame<'a> {
    dc: &'a mut Dc,
    sf: &'a mut Surface,
    ras: Raster,
    cov: [Vec<f32>; 3],
    /// the view's top left in the target, its size, the part of it this frame looks for entities in (pixels of the view: all
    /// of it, or a band of its rows; the geometry is the whole view's, clipped as for all of it: the same bytes in bands),
    /// the guard box, the clip (target pixels)
    at: P,
    size: (u32, u32),
    vb: Bx,
    g: Bx,
    clip: [i32; 4],
    info: CpuInfo,
    runs: Vec<Run>,
    dashes: Vec<Dash>,
    pts: Vec<P>,
    lens: Vec<f64>,
    /// Em's clipping buffers
    tmp: (Vec<P>, Vec<P>),
}

impl<'a> Frame<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(dc: &'a mut Dc, sf: &'a mut Surface, ras: Raster, cov: [Vec<f32>; 3], at: P, size: (u32, u32), vb: Bx, clip: [i32; 4]) -> Frame<'a> {
        Frame { dc, sf, ras, cov, at, size, vb, g: [-GUARD, -GUARD, size.0 as f64 + GUARD, size.1 as f64 + GUARD], clip, info: CpuInfo::default(), runs: Vec::new(), dashes: Vec::new(), pts: Vec::new(),
                lens: Vec::new(), tmp: (Vec::new(), Vec::new()) }
    }

    fn em(&mut self) -> Em<'_> {
        Em { ras: &mut self.ras, g: self.g, at: self.at, lines: &mut self.info.lines, tmp: &mut self.tmp.0, tmp2: &mut self.tmp.1 }
    }

    fn begin(&mut self) {
        self.ras.lines_begin();
    }

    /// what was emitted, filled once in a colour
    fn fill(&mut self, color: u32, rule: u32) {
        if !self.ras.has_lines() || color >> 24 == 0 {
            return;
        }
        self.dc.set_color(color);
        self.ras.fill(self.dc, self.sf, rule);
        self.info.fills += 1;
    }

    /// the box (target pixels, x0 y0 x1 y1) of what was emitted, within the clip
    fn lbox(&self) -> Option<[i32; 4]> {
        if !self.ras.has_lines() {
            return None;
        }
        let b = self.ras.lbox();
        let x0 = (floor(b[0] as f64) as i32 - 1).max(self.clip[0]);
        let y0 = (floor(b[1] as f64) as i32 - 1).max(self.clip[1]);
        let x1 = (floor(b[2] as f64) as i32 + 2).min(self.clip[2]);
        let y1 = (floor(b[3] as f64) as i32 + 2).min(self.clip[3]);
        if x1 <= x0 || y1 <= y0 { None } else { Some([x0, y0, x1, y1]) }
    }

    /// the box (target pixels) of what was emitted within the whole view (not a band's clip: what the geometry depends on)
    fn lbox_view(&self) -> [f64; 4] {
        let b = self.ras.lbox();
        let (w, h) = (self.size.0 as f64, self.size.1 as f64);
        [(b[0] as f64).max(self.at[0] - GUARD), (b[1] as f64).max(self.at[1] - GUARD), (b[2] as f64).min(self.at[0] + w + GUARD), (b[3] as f64).min(self.at[1] + h + GUARD)]
    }

    /// what was emitted, as coverage (0 ..= 1) over box bx into buffer k
    fn cover(&mut self, bx: [i32; 4], rule: u32, k: usize) {
        let (w, h) = ((bx[2] - bx[0]) as usize, (bx[3] - bx[1]) as usize);
        let buf = &mut self.cov[k];
        buf.clear();
        buf.resize(w * h, 0.0);
        if !self.ras.has_lines() {
            return;
        }
        self.info.fills += 1;
        self.ras.fill_rows(bx, rule, &mut |y, x, c| {
            let o = (y - bx[1]) as usize * w + (x - bx[0]) as usize;
            for (j, &v) in c.iter().enumerate() {
                if let Some(t) = buf.get_mut(o + j) {
                    *t = v as f32 * (1.0 / 255.0);
                }
            }
        });
    }

    /// coverage of buffer 0 over box bx blended in a colour (lib/gfx's mask_over, as Raster::fill blends)
    fn blend(&mut self, bx: [i32; 4], color: u32) {
        if color >> 24 == 0 {
            return;
        }
        self.dc.set_color(color);
        let w = (bx[2] - bx[0]) as usize;
        let stride = self.sf.stride as usize;
        let mut row: Vec<u8> = vec![0; w];
        for y in bx[1]..bx[3] {
            let c = &self.cov[0][(y - bx[1]) as usize * w..][..w];
            let mut any = false;
            for (r, &v) in row.iter_mut().zip(c) {
                *r = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                any |= *r != 0;
            }
            if !any {
                continue;
            }
            let o = (y - self.dc.row0) as usize * stride + bx[0] as usize * 4;
            if let Some(px) = self.sf.px.get_mut(o..o + w * 4) {
                gfx::blend::mask_over(px, &row, &self.dc.pcol);
            }
        }
    }
}

impl Scene {
    /// the scene drawn on the CPU through lib/gfx (G6e1, decision 29): the camera's view of a target of r's size into the
    /// rectangle r (x, y, w, h, relative to dc's origin) of sf, clipped to dc's clip, blended over what is there (the app
    /// clears it first, as a frame's load does); the painter's order, the visible layers, the look, the highlights and the
    /// overlay of a frame on the GPU (docs/RENDER2D.md "Der CPU-Weg").  With and without a device.  dc keeps its colour,
    /// clip and origin
    pub fn draw_cpu(&mut self, dc: &mut Dc, sf: &mut Surface, r: [i32; 4]) -> CpuInfo {
        if r[2] <= 0 || r[3] <= 0 {
            self.cpu.info = CpuInfo::default();
            return self.cpu.info;
        }
        // (text decides glyphs or bars by a scale that moves only when the scale leaves HYST around it, as prepare does)
        let sc = self.camera.scale;
        if !(self.text_scale > 0.0) || sc < self.text_scale / curve::HYST || sc > self.text_scale * curve::HYST {
            self.text_scale = sc;
        }
        self.sync_index();
        let saved = dc.get_color();
        dc.clip_push(r[0], r[1], r[2], r[3]);
        let size = (r[2] as u32, r[3] as u32);
        let at = [(r[0] + dc.ox) as f64, (r[1] + dc.oy) as f64];
        let clip = dc.clip;
        // (the browser: no bands, plat's loop runs one after the other there)
        #[cfg(not(target_arch = "wasm32"))]
        let bands = if self.cpu.single { 1 } else { ((clip[3] - clip[1]) / BAND_ROWS).clamp(1, BANDS) };
        let info = match gfx::parallel() {
            #[cfg(not(target_arch = "wasm32"))]
            Some(pf) if bands > 1 && clip[2] > clip[0] => self.cpu_bands(pf, dc, sf, at, size, clip, bands),
            _ => {
                let st = core::mem::take(&mut self.cpu);
                let single = st.single;
                let vb = [0.0, 0.0, size.0 as f64, size.1 as f64];
                let mut fr = Frame::new(dc, sf, st.ras, st.cov, at, size, vb, clip);
                self.cpu_frame(&mut fr);
                let info = fr.info;
                self.cpu = CpuState { ras: fr.ras, info, cov: fr.cov, single };
                info
            }
        };
        dc.clip_pop();
        dc.set_color(saved);
        self.cpu.info = info;
        info
    }

    /// draw_cpu on one thread (false: row bands on lib/gfx's parallel loop where one is set, gfx::set_parallel: lib/ui sets
    /// its worker pool there; the same bytes either way)
    pub fn set_cpu_single(&mut self, single: bool) {
        self.cpu.single = single;
    }

    /// the frame in row bands of the clip, each drawn on its own surface (a copy of the target's rows, Dc::begin_rows) on
    /// lib/gfx's parallel loop, then copied back: each band culls, builds and rasterizes what meets its rows
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_arguments)]
    fn cpu_bands(&mut self, pf: gfx::ParallelFor, dc: &Dc, sf: &mut Surface, at: P, size: (u32, u32), clip: [i32; 4], bands: i32) -> CpuInfo {
        let rows = clip[3] - clip[1];
        let bpp = sf.bpp();
        let (x0, x1) = (clip[0], clip[2]);
        let band = |b: i32| (clip[1] + rows * b / bands, clip[1] + rows * (b + 1) / bands);
        // each band's rows of the target, copied out (the frame blends over what is there)
        let parts: Vec<rt::sync::SpinLock<(Surface, CpuInfo)>> = (0..bands)
            .map(|b| {
                let (ya, yb) = band(b);
                let mut part = Surface::new(sf.w, yb - ya, sf.fmt);
                let n = sf.w as usize * bpp;
                for y in ya..yb {
                    let (s0, d0) = ((y - dc.row0) as usize * sf.stride as usize, (y - ya) as usize * part.stride as usize);
                    part.px[d0..d0 + n].copy_from_slice(&sf.px[s0..s0 + n]);
                }
                rt::sync::SpinLock::new((part, CpuInfo::default()))
            })
            .collect();
        let me: &Scene = self;
        let (opac, h) = (dc.opac, dc.h);
        pf(bands as usize, &|b| {
            let Some(cell) = parts.get(b) else { return };
            let mut g = cell.lock();
            let (ya, yb) = band(b as i32);
            let (part, info) = &mut *g;
            let mut bdc = Dc::new(part.w, yb - ya);
            bdc.begin_rows(part, ya, h);
            bdc.set_opacity(opac);
            bdc.clip_push(x0, ya, x1 - x0, yb - ya);
            let bclip = bdc.clip;
            // (the band's rows in pixels of the view)
            let vb = [0.0, ya as f64 - at[1], size.0 as f64, yb as f64 - at[1]];
            let mut fr = Frame::new(&mut bdc, part, Raster::default(), Default::default(), at, size, vb, bclip);
            me.cpu_frame(&mut fr);
            *info = fr.info;
        });
        let mut sum = CpuInfo::default();
        for (b, cell) in parts.iter().enumerate() {
            let g = cell.lock();
            let (part, info) = &*g;
            let (ya, yb) = band(b as i32);
            let n = (x1 - x0) as usize * bpp;
            for y in ya..yb {
                let (s0, d0) = ((y - ya) as usize * part.stride as usize + x0 as usize * bpp, (y - dc.row0) as usize * sf.stride as usize + x0 as usize * bpp);
                sf.px[d0..d0 + n].copy_from_slice(&part.px[s0..s0 + n]);
            }
            // (an entity on several bands is found and drawn on each: the sums say what the bands did)
            sum.found += info.found;
            sum.drawn += info.drawn;
            sum.fills += info.fills;
            sum.lines += info.lines;
            sum.flattened += info.flattened;
            sum.overlay = sum.overlay.max(info.overlay);
        }
        sum
    }

    /// what the last draw_cpu did
    pub fn cpu_info(&self) -> CpuInfo {
        self.cpu.info
    }

    fn cpu_frame(&self, fr: &mut Frame) {
        let cam = self.camera;
        let (w, h) = (fr.size.0 as f64, fr.size.1 as f64);
        let sc = cam.scale;
        let (sn, cs) = sincos(cam.rotation);
        // drawing units relative to the centre -> pixels from the view's top left (Camera::to_screen)
        let top = Place { sub: cam.center, lin: [[cs / sc, -sn / sc], [-sn / sc, -cs / sc]], off: [0.5 * w, 0.5 * h] };
        let ctx = FlatCtx::new(&cam, fr.size);
        let Some(ix) = self.idx.as_ref() else { return };
        // the box in the drawing of the part this frame draws (the view, or a band of it), grown by what reaches in pixels
        // (the widest style, a hover's pixel, a smoothed edge)
        let vb = fr.vb;
        let ai = [[sc * cs, -sc * sn], [-sc * sn, -sc * cs]];
        let corner = |x: f64, y: f64| {
            let (u, v) = (x - 0.5 * w, y - 0.5 * h);
            [cam.center[0] + ai[0][0] * u + ai[0][1] * v, cam.center[1] + ai[1][0] * u + ai[1][1] * v]
        };
        let q = geo::bx_grow(&geo::bx_of(&[corner(vb[0], vb[1]), corner(vb[2], vb[1]), corner(vb[0], vb[3]), corner(vb[2], vb[3])]), (self.max_px_reach() + 3.0) * sc);
        let ranks = self.layer_ranks();
        let mut tops: Vec<(u32, u32, u32)> = Vec::new();
        let mut take = |s: u32| {
            let e = &self.ents[s as usize];
            if !(e.live && e.block == NONE) {
                return;
            }
            let l = &self.layers[e.layer as usize];
            if l.live && l.visible {
                tops.push((ranks[e.layer as usize], e.at, s));
            }
        };
        ix.top.query(&q, &mut |s, _| {
            take(s);
            true
        });
        for &s in &ix.px {
            take(s);
        }
        tops.sort_unstable();
        tops.dedup();
        fr.info.found = tops.len() as u32;
        for &(_, _, slot) in &tops {
            let e = &self.ents[slot as usize];
            let l = &self.layers[e.layer as usize];
            let st = &self.styles[e.style as usize];
            let hlb = self.hl.bits.get(slot as usize).copied().unwrap_or(0);
            match &e.geom {
                Geom::Insert(ins) => self.cpu_insert(fr, &top, &q, ins, [0.0, 0.0], None, st, l.transparency, hlb),
                g => {
                    let wh = Where { place: top, upp: sc, sw: 1.0, layer_t: l.transparency, hl: hlb, band: None, ovl: false, xf: None };
                    self.cpu_geom(fr, &ctx, g, &wh, st, Some(e.style));
                }
            }
        }
        // the overlay over everything (overlay.rs makes it into the overlay's frame: target pixels with y up)
        if !self.ov.is_empty() {
            let b = overlay::build(self.ov.items(), &cam, fr.size);
            let place = Place { sub: [0.0, 0.0], lin: [[1.0, 0.0], [0.0, -1.0]], off: [0.0, 0.0] };
            let octx = FlatCtx::new(&super::Camera::new([0.5 * w, -0.5 * h], 1.0), fr.size);
            for (g, s) in &b.geoms {
                let wh = Where { place, upp: 1.0, sw: 1.0, layer_t: 0, hl: 0, band: None, ovl: true, xf: None };
                self.cpu_geom(fr, &octx, g, &wh, &b.styles[*s as usize], None);
            }
            fr.info.overlay = self.ov.len() as u32;
        }
    }

    /// an insert's copies that may meet q (drawing units): down into its block (parent: the transform of the block it sits in),
    /// its entities in their order, nested inserts in their place (pick.rs walk_insert's way)
    #[allow(clippy::too_many_arguments)]
    fn cpu_insert(&self, fr: &mut Frame, top: &Place, q: &Bx, ins: &Insert, origin: P, parent: Option<Xf>, ins_style: &Style, layer_t: u32, hlb: u8) {
        let Some(ix) = self.idx.as_ref() else { return };
        let b = ins.block.slot as usize;
        let Some(bi) = ix.blocks.get(b) else { return };
        let Some(bb) = bi.bbox else { return };
        let ql = match &parent {
            Some(p) => inv_box(p, q),
            None => *q,
        };
        let sw0 = Xf { t: [0.0, 0.0], m: copy_xf(ins, origin, 0, 0).m }.sw();
        let pm = parent.map_or(1.0, |p| p.sw());
        let (c0, c1, r0, r1) = copies_meeting(ins, origin, &bb, &ql, bi.reach * sw0 + 1e-9 * (1.0 + pm));
        let bl = &self.blocks[b];
        let sc = self.camera.scale;
        let ctx = FlatCtx::new(&self.camera, fr.size);
        let mut ks: Vec<u32> = Vec::new();
        for row in r0..=r1.min(ins.rows.max(1) - 1) {
            for col in c0..=c1.min(ins.columns.max(1) - 1) {
                let local = copy_xf(ins, origin, col, row);
                let xf = match &parent {
                    Some(p) => p.then(&local),
                    None => local,
                };
                let sw = xf.sw();
                let qb = inv_box(&xf, &geo::bx_grow(q, bi.reach * sw));
                if !geo::bx_meets(&qb, &bb) {
                    continue;
                }
                ks.clear();
                if bi.px {
                    ks.extend(0..bl.ents.len() as u32);
                } else {
                    bi.tree.query(&qb, &mut |k, _| {
                        ks.push(k);
                        true
                    });
                    ks.sort_unstable();
                }
                // the block's units -> pixels: the camera after the insert's transform, its translation taken off the centre first
                let a = top.lin;
                let m = xf.m;
                let lin = [[a[0][0] * m[0][0] + a[0][1] * m[1][0], a[0][0] * m[0][1] + a[0][1] * m[1][1]], [a[1][0] * m[0][0] + a[1][1] * m[1][0], a[1][0] * m[0][1] + a[1][1] * m[1][1]]];
                let place = Place { sub: bl.base, lin, off: add(top.off, top.vec(sub(xf.t, top.sub))) };
                let (hi, _) = xf.singular();
                let band = curve::band_of(sc / hi);
                let wh = Where { place, upp: sc / sw, sw, layer_t, hl: hlb, band: Some(band), ovl: false, xf: Some((xf, bl.base)) };
                for &k in &ks {
                    let Some(&slot) = bl.ents.get(k as usize) else { continue };
                    let e = &self.ents[slot as usize];
                    if !e.live {
                        continue;
                    }
                    let own = self.styles[e.style as usize];
                    let st = if own.takes() { ins_style.by_block_from(&own) } else { own };
                    if let Geom::Insert(ni) = &e.geom {
                        self.cpu_insert(fr, top, q, ni, bl.base, Some(xf), &st, layer_t, hlb);
                    } else {
                        self.cpu_geom(fr, &ctx, &e.geom, &wh, &st, None);
                    }
                }
            }
        }
    }

    /// an entity's geometry where wh says, in its style (own: its style's slot on a layer, for its curves' reach)
    fn cpu_geom(&self, fr: &mut Frame, ctx: &FlatCtx, g: &Geom, wh: &Where, st: &Style, own: Option<u32>) {
        let color = hl_ink(ink(st.color, st.transparency, wh.layer_t), wh.hl);
        fr.info.drawn += 1;
        match g {
            Geom::Line(p) => {
                let l = len(sub(p[1], p[0]));
                let px = [wh.place.px(p[0]), wh.place.px(p[1])];
                self.cpu_stroke(fr, wh, st, color, &px, &[0.0, l], false, l);
            }
            Geom::Poly(p, closed) => {
                let mut px = core::mem::take(&mut fr.pts);
                let mut lens = core::mem::take(&mut fr.lens);
                px.clear();
                lens.clear();
                let mut acc = 0.0;
                for (k, q) in p.iter().enumerate() {
                    if k > 0 {
                        acc += len(sub(*q, p[k - 1]));
                    }
                    px.push(wh.place.px(*q));
                    lens.push(acc);
                }
                let total = if *closed && !p.is_empty() { acc + len(sub(p[0], p[p.len() - 1])) } else { acc };
                self.cpu_stroke(fr, wh, st, color, &px, &lens, *closed, total);
                fr.pts = px;
                fr.lens = lens;
            }
            &Geom::Arc(c, r, a0, sw) => {
                let bx = geo::bx_grow(&[0.0, 0.0, fr.size.0 as f64, fr.size.1 as f64], self.pen_of(st, wh).reach());
                let mut px = core::mem::take(&mut fr.pts);
                let mut lens = core::mem::take(&mut fr.lens);
                let (closed, total) = arc_line(&wh.place, c, r, a0, sw, &bx, &mut px, &mut lens);
                self.cpu_stroke(fr, wh, st, color, &px, &lens, closed, total);
                fr.pts = px;
                fr.lens = lens;
            }
            Geom::Curve(cg) => {
                let fresh;
                let flat = match (wh.band, own) {
                    (None, Some(s)) => {
                        let reach = stroke_reach(&self.styles[s as usize], ctx.scale);
                        if ctx.stale(&cg.st, reach) {
                            fr.info.flattened += 1;
                            fresh = ctx.flatten(&cg.c, reach).0;
                            &fresh
                        } else {
                            &cg.flat
                        }
                    }
                    (Some(band), _) => {
                        fr.info.flattened += 1;
                        fresh = flat_whole(&cg.c, curve::FLAT_TOL * curve::pow2(band));
                        &fresh
                    }
                    _ => &cg.flat,
                };
                let mut px = core::mem::take(&mut fr.pts);
                px.clear();
                px.extend(flat.pts.iter().map(|q| wh.place.px(*q)));
                self.cpu_stroke(fr, wh, st, color, &px, &flat.lens, flat.closed, flat.total);
                fr.pts = px;
            }
            Geom::Fill(f) => self.cpu_fill(fr, ctx, f, wh, st, color),
            Geom::Text(t) => self.cpu_text(fr, t, wh, color),
            Geom::Insert(_) => {}
        }
    }

    /// a style's pen where wh says (widths in units through the insert's scale, at least half a pixel, a highlight wider)
    fn pen_of(&self, st: &Style, wh: &Where) -> Pen {
        let w = if st.unit == Unit::Drawing { st.width as f64 / wh.upp } else { st.width as f64 };
        Pen { h: (0.5 * w).max(0.5) + hl_wider(wh.hl), cap: st.cap, join: st.join, limit: st.miter_limit as f64 }
    }

    /// a centre line in pixels (its lengths along the entity in its units) stroked in its style: solid, dashed, a flat tone
    /// or a mix of both as the GPU draws it
    #[allow(clippy::too_many_arguments)]
    fn cpu_stroke(&self, fr: &mut Frame, wh: &Where, st: &Style, color: u32, px: &[P], lens: &[f64], closed: bool, total: f64) {
        let pen = self.pen_of(st, wh);
        let bx = geo::bx_grow(&[0.0, 0.0, fr.size.0 as f64, fr.size.1 as f64], pen.reach());
        let mut runs = core::mem::take(&mut fr.runs);
        runs_of(px, lens, closed, total, &bx, &mut runs);
        if runs.is_empty() {
            fr.runs = runs;
            return;
        }
        let pat = pattern_of(st, wh.upp);
        let (tf, duty) = match &pat {
            Some(p) => {
                let period_px = p.period / wh.upp;
                let capl = match pen.cap {
                    Cap::Square => 2.0 * pen.h,
                    Cap::Round => 0.5 * core::f64::consts::PI * pen.h,
                    _ => 0.0,
                } * wh.upp;
                (((DASH_EXACT - period_px) / (DASH_EXACT - DASH_FLAT)).clamp(0.0, 1.0), (p.on / p.period + p.ons * capl / p.period).min(1.0))
            }
            None => (1.0, 1.0),
        };
        let solid = |fr: &mut Frame, runs: &[Run]| {
            let mut em = fr.em();
            for run in runs {
                stroke_run(&mut em, &run.pts, run.closed, !run.open_s, !run.open_e, &pen, None);
            }
        };
        let dashed = |fr: &mut Frame, runs: &[Run], pat: &Pattern| {
            let mut ds = core::mem::take(&mut fr.dashes);
            for run in runs {
                if run.segl.is_empty() {
                    // (a single point: its caps where the pattern is on at its length 0)
                    let x = modp(pat.phase, pat.period);
                    if pat.ends[..pat.n].iter().position(|&e| e > x).unwrap_or(0) % 2 == 0 {
                        stroke_run(&mut fr.em(), &run.pts, false, true, true, &pen, None);
                    }
                    continue;
                }
                dash_run(run, pat, &mut ds);
                let mut em = fr.em();
                for dsh in &ds {
                    stroke_run(&mut em, &dsh.pts, dsh.closed, dsh.cap_s, dsh.cap_e, &pen, dsh.dir);
                }
            }
            fr.dashes = ds;
        };
        match &pat {
            None => {
                fr.begin();
                solid(fr, &runs);
                fr.fill(color, RULE_NONZERO);
            }
            Some(_) if tf >= 1.0 => {
                fr.begin();
                solid(fr, &runs);
                fr.fill(alpha_times(color, duty), RULE_NONZERO);
            }
            Some(p) if tf <= 0.0 => {
                fr.begin();
                dashed(fr, &runs, p);
                fr.fill(color, RULE_NONZERO);
            }
            Some(p) => {
                // c = mix(dashes, solid * duty, tf) (full_body.wgsl)
                fr.begin();
                solid(fr, &runs);
                if let Some(bx) = fr.lbox() {
                    fr.cover(bx, RULE_NONZERO, 1);
                    fr.begin();
                    dashed(fr, &runs, p);
                    fr.cover(bx, RULE_NONZERO, 0);
                    let (a, b) = (1.0 - tf as f32, (tf * duty) as f32);
                    let [c0, c1, _] = &mut fr.cov;
                    for (x, y) in c0.iter_mut().zip(c1.iter()) {
                        *x = a * *x + b * *y;
                    }
                    fr.blend(bx, color);
                }
            }
        }
        fr.runs = runs;
    }

    /// a fill's rings in pixels (oriented: islands one way, holes the other), even-odd
    fn fill_rings(&self, fr: &mut Frame, ctx: &FlatCtx, f: &FillG, wh: &Where) -> Vec<Vec<P>> {
        let mut rings: Vec<Vec<P>> = Vec::new();
        let fresh = match wh.band {
            None if wh.ovl || !ctx.stale(&f.st, FILL_REACH) => None,
            None => {
                fr.info.flattened += 1;
                Some(f.rings.iter().map(|c| ctx.flatten(c, FILL_REACH).0.pts).collect::<Vec<_>>())
            }
            Some(band) => {
                fr.info.flattened += 1;
                let tol = curve::FLAT_TOL * curve::pow2(band);
                Some(f.rings.iter().map(|c| flat_whole(c, tol).pts).collect::<Vec<_>>())
            }
        };
        match fresh {
            None => {
                let o = f.prep.origin;
                for r in &f.prep.rings {
                    rings.push(r.iter().map(|q| wh.place.px([o[0] + q[0], o[1] + q[1]])).collect());
                }
            }
            Some(mut rs) => {
                orient(&mut rs);
                for r in &rs {
                    rings.push(r.iter().map(|q| wh.place.px(*q)).collect());
                }
            }
        }
        rings
    }

    /// a fill: its colour, or its hatch's lines times its coverage
    fn cpu_fill(&self, fr: &mut Frame, ctx: &FlatCtx, f: &FillG, wh: &Where, st: &Style, color: u32) {
        let rings = self.fill_rings(fr, ctx, f, wh);
        if rings.is_empty() {
            return;
        }
        fr.begin();
        {
            let mut em = fr.em();
            for r in &rings {
                em.ring(r);
            }
        }
        if st.hatches == 0 {
            fr.fill(color, RULE_EVENODD);
            return;
        }
        // the hatch's families where the fill is drawn (an insert's: through its transform, drawing units)
        let (fams, nf) = block::fams(st);
        let mut fam = [fams[0]; HATCH_MAX];
        for k in 0..nf {
            fam[k] = match &wh.xf {
                Some((xf, base)) => fams[k].through(xf, *base),
                None => fams[k],
            };
        }
        let hw = (0.5 * st.hatch_width as f64).max(0.5);
        let cam = self.camera;
        let sc = if wh.ovl { 1.0 } else { cam.scale };
        // per family: the flat tone's share, the mix of tone and lines
        let mut tones = [0.0f64; HATCH_MAX];
        let mut tfs = [1.0f64; HATCH_MAX];
        for k in 0..nf {
            let h = &fam[k];
            let sp = h.spacing / sc;
            tfs[k] = ((LINE_EXACT - sp) / (LINE_EXACT - LINE_FLAT)).clamp(0.0, 1.0);
            tones[k] = (2.0 * hw / sp).min(1.0) * hatch_duty(h, hw, sc);
        }
        if tfs[..nf].iter().all(|&t| t >= 1.0) {
            // all flat: the fill in a tone
            let keep: f64 = tones[..nf].iter().map(|t| 1.0 - t).product();
            fr.fill(alpha_times(color, 1.0 - keep), RULE_EVENODD);
            return;
        }
        let Some(bx) = fr.lbox() else { return };
        let fb = fr.lbox_view();
        fr.cover(bx, RULE_EVENODD, 0);
        let (bw, bh) = ((bx[2] - bx[0]) as usize, (bx[3] - bx[1]) as usize);
        let mut keep = vec![1.0f32; bw * bh];
        for k in 0..nf {
            let tf = tfs[k];
            let cf: Vec<f32> = if tf >= 1.0 {
                vec![tones[k] as f32; bw * bh]
            } else {
                let ex = self.hatch_family(fr, bx, fb, &rings, &fam[k], hw, sc);
                if tf <= 0.0 { ex } else { ex.iter().map(|&e| ((1.0 - tf) * e as f64 + tf * tones[k]) as f32).collect() }
            };
            for (kp, c) in keep.iter_mut().zip(cf) {
                *kp *= 1.0 - c.min(1.0);
            }
        }
        for (m, kp) in fr.cov[0].iter_mut().zip(&keep) {
            *m *= 1.0 - kp;
        }
        fr.blend(bx, color);
    }

    /// a hatch family's exact lines (fill.wgsl hatch_line: strips, their dashes by the pattern along the line; dots as long
    /// as the line is wide; a mix of dashes and their duty between DASH_FLAT and DASH_EXACT) over box bx: coverage per pixel
    #[allow(clippy::too_many_arguments)]
    fn hatch_family(&self, fr: &mut Frame, bx: [i32; 4], fb: [f64; 4], rings: &[Vec<P>], h: &Fam, hw: f64, sc: f64) -> Vec<f32> {
        let cam = self.camera;
        let (sn, cs) = sincos(cam.rotation);
        let a = [[cs / sc, -sn / sc], [-sn / sc, -cs / sc]];
        let off = [0.5 * fr.size.0 as f64, 0.5 * fr.size.1 as f64];
        let to_px = |v: P| [a[0][0] * v[0] + a[0][1] * v[1], a[1][0] * v[0] + a[1][1] * v[1]];
        let dir = h.dir;
        let nrm = [-dir[1], dir[0]];
        let (npx, dpx) = (to_px(nrm), to_px(dir));
        let dl = len(dpx);
        // relative to the camera's centre: the nearest line k0 and the centre's distance from it, the place along it
        let cx = sub(cam.center, h.base);
        let k0 = round(dot(cx, nrm) / h.spacing);
        let d0 = dot(cx, nrm) - k0 * h.spacing;
        let pat = hatch_pattern(h);
        let t0 = match &pat {
            Some((_, per, _)) => modp(dot(cx, dir) - k0 * h.shift, *per),
            None => 0.0,
        };
        // the lines over the box (view pixels): its corners back in the drawing relative to the centre
        let ai = [[sc * cs, -sc * sn], [-sc * sn, -sc * cs]];
        // (the fill's box in the view, not a band's: the lines and their ends are the same in every band)
        let corners = [[fb[0], fb[1]], [fb[2], fb[1]], [fb[0], fb[3]], [fb[2], fb[3]]];
        let (mut u0, mut u1, mut s0, mut s1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for c in corners {
            let v = sub(sub(c, fr.at), off);
            let rel = [ai[0][0] * v[0] + ai[0][1] * v[1], ai[1][0] * v[0] + ai[1][1] * v[1]];
            u0 = u0.min(dot(rel, nrm));
            u1 = u1.max(dot(rel, nrm));
            s0 = s0.min(dot(rel, dir));
            s1 = s1.max(dot(rel, dir));
        }
        let (j0, j1) = (floor((u0 + d0) / h.spacing) as i64 - 1, floor((u1 + d0) / h.spacing) as i64 + 2);
        let per_px = pat.as_ref().map_or(0.0, |p| p.1 / sc);
        let tfa = if pat.is_some() { ((DASH_EXACT - per_px) / (DASH_EXACT - DASH_FLAT)).clamp(0.0, 1.0) } else { 0.0 };
        let duty = hatch_duty(h, hw, sc);
        let nhat = mul(npx, 1.0 / len(npx));
        let margin = (hw + 1.5) / dl;
        let mut xs: Vec<f64> = Vec::new();
        let mut ivs: Vec<(f64, f64)> = Vec::new();
        // pass 0: the strips (solid), pass 1: the dashes; what each pass needs
        let passes: &[bool] = if pat.is_none() || tfa >= 1.0 { &[false] } else if tfa <= 0.0 { &[true] } else { &[false, true] };
        let mut out = vec![0.0f32; ((bx[2] - bx[0]) * (bx[3] - bx[1])) as usize];
        for &dashes in passes {
            fr.begin();
            for j in j0..=j1.min(j0 + 100_000) {
                let jf = j as f64;
                let p0 = add(off, mul(npx, jf * h.spacing - d0));
                // where the line's strip meets the fill (s along it, drawing units): the inside of the centre line and of the
                // two lines a half width and a pixel to either side (a line along the fill's edge, within its smoothed
                // edge, shows there as the GPU's product of both coverages shows it), each grown by the margin
                ivs.clear();
                for o in [-(hw + 1.0), 0.0, hw + 1.0] {
                    let pl = add(p0, mul(nhat, o));
                    xs.clear();
                    for r in rings {
                        let m = r.len();
                        for i in 0..m {
                            let (e0, e1) = (r[i], r[(i + 1) % m]);
                            let ed = sub(e1, e0);
                            let den = cross(dpx, ed);
                            if den == 0.0 {
                                continue;
                            }
                            let w0 = sub(e0, pl);
                            let t = cross(w0, dpx) / den;
                            if (0.0..1.0).contains(&t) {
                                xs.push(cross(w0, ed) / den);
                            }
                        }
                    }
                    xs.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
                    for pr in xs.chunks_exact(2) {
                        let (a, b) = ((pr[0] - margin).max(s0 - margin), (pr[1] + margin).min(s1 + margin));
                        if b > a {
                            ivs.push((a, b));
                        }
                    }
                }
                if ivs.is_empty() {
                    continue;
                }
                ivs.sort_unstable_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
                let mut k = 0;
                for i in 1..ivs.len() {
                    if ivs[i].0 <= ivs[k].1 {
                        ivs[k].1 = ivs[k].1.max(ivs[i].1);
                    } else {
                        k += 1;
                        ivs[k] = ivs[i];
                    }
                }
                ivs.truncate(k + 1);
                let mut em = fr.em();
                let rect = |em: &mut Em, a: f64, b: f64| {
                    let (q0, q1) = (add(p0, mul(dpx, a)), add(p0, mul(dpx, b)));
                    let nn = mul(nhat, hw);
                    em.piece(&[add(q0, nn), add(q1, nn), sub(q1, nn), sub(q0, nn)], 1.0);
                };
                for &(a, b) in &ivs {
                    if !dashes {
                        rect(&mut em, a, b);
                        continue;
                    }
                    let Some((ends, per, n)) = &pat else { continue };
                    // the pattern's place at s: t0 + s - j shift (drawing units)
                    let base = t0 - jf * h.shift;
                    let x0 = modp(base + a, *per);
                    let mut m0 = a - x0;
                    while m0 < b {
                        let mut c0 = 0.0;
                        for (i, &c1) in ends[..*n].iter().enumerate() {
                            if i % 2 == 0 {
                                let (mut da, mut db) = (m0 + c0, m0 + c1);
                                if (db - da) * dl < 2.0 * hw {
                                    let mid = 0.5 * (da + db);
                                    da = mid - hw / dl;
                                    db = mid + hw / dl;
                                }
                                let (da, db) = (da.max(a), db.min(b));
                                if db > da {
                                    rect(&mut em, da, db);
                                }
                            }
                            c0 = c1;
                        }
                        m0 += per;
                    }
                }
            }
            fr.cover(bx, RULE_NONZERO, 2);
            let w = if passes.len() == 2 {
                if dashes { (1.0 - tfa) as f32 } else { (tfa * duty) as f32 }
            } else if dashes {
                1.0
            } else {
                // (solid lines, or the dashes' flat tone along them)
                if pat.is_some() { duty as f32 } else { 1.0 }
            };
            for (o, c) in out.iter_mut().zip(&fr.cov[2]) {
                *o += w * c;
            }
        }
        out
    }

    /// a text: its glyphs' outlines (G6c's layout through its map into pixels), or below TEXT_BAR_BELOW px per em its bars
    fn cpu_text(&self, fr: &mut Frame, t: &TextG, wh: &Where, color: u32) {
        let lay = &t.lay;
        let m = lay.m;
        let o = wh.place.px(lay.origin);
        let sc = self.camera.scale;
        // the em -> pixel map (a style in pixels: turned with the camera, through an insert's map without its scale)
        let l = wh.place.lin;
        let k = if lay.px { sc / wh.sw } else { 1.0 };
        let m2 = [[(l[0][0] * m[0] + l[0][1] * m[2]) * k, (l[0][0] * m[1] + l[0][1] * m[3]) * k], [(l[1][0] * m[0] + l[1][1] * m[2]) * k, (l[1][0] * m[1] + l[1][1] * m[3]) * k]];
        let at = |e: P| [o[0] + m2[0][0] * e[0] + m2[0][1] * e[1], o[1] + m2[1][0] * e[0] + m2[1][1] * e[1]];
        let em_px = len([m2[0][1], m2[1][1]]) * sc / self.text_scale;
        if em_px < TEXT_BAR_BELOW {
            // small: a bar per word, as dark as the ink it stands for (each blended for itself)
            for b in &t.bars {
                let (x0, x1, y0, y1) = (b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64);
                fr.begin();
                fr.em().piece(&[at([x0, y0]), at([x1, y0]), at([x1, y1]), at([x0, y1])], 1.0);
                fr.fill(alpha_times(color, b[4] as f64), RULE_NONZERO);
            }
            return;
        }
        fr.begin();
        let mut ring: Vec<P> = Vec::new();
        let tol = ARC_TOL;
        let g = fr.g;
        for line in &lay.lines {
            for gi in line.glyphs.start..line.glyphs.end {
                let pg = lay.glyphs[gi as usize];
                if pg.blank {
                    continue;
                }
                let Some(sh) = self.fonts.shape_cached(pg.font, pg.glyph) else { continue };
                let pen = [line.origin[0] + pg.pen[0], line.origin[1] + pg.pen[1]];
                if let Some(bb) = sh.bbox {
                    let c = [at(add(pen, [bb[0], bb[1]])), at(add(pen, [bb[2], bb[1]])), at(add(pen, [bb[0], bb[3]])), at(add(pen, [bb[2], bb[3]]))];
                    let b = geo::bx_of(&c);
                    if b[2] < g[0] || b[0] > g[2] || b[3] < g[1] || b[1] > g[3] {
                        continue;
                    }
                }
                for cn in &sh.contours {
                    ring.clear();
                    for s in cn {
                        match *s {
                            Seg::Line(a, _) => ring.push(at(add(pen, a))),
                            Seg::Quad(a, b, c) => {
                                let (p0, p1, p2) = (at(add(pen, a)), at(add(pen, b)), at(add(pen, c)));
                                let dd = len(add(sub(p0, mul(p1, 2.0)), p2));
                                let n = (sqrt(dd / (4.0 * tol)) as usize + 1).min(512);
                                for i in 0..n {
                                    let t = i as f64 / n as f64;
                                    let s = 1.0 - t;
                                    ring.push(add(add(mul(p0, s * s), mul(p1, 2.0 * s * t)), mul(p2, t * t)));
                                }
                            }
                            Seg::Cubic(a, b, c, d) => {
                                let (p0, p1, p2, p3) = (at(add(pen, a)), at(add(pen, b)), at(add(pen, c)), at(add(pen, d)));
                                let dd = len(add(sub(p0, mul(p1, 2.0)), p2)).max(len(add(sub(p1, mul(p2, 2.0)), p3)));
                                let n = (sqrt(3.0 * dd / (4.0 * tol)) as usize + 1).min(512);
                                for i in 0..n {
                                    let t = i as f64 / n as f64;
                                    let s = 1.0 - t;
                                    ring.push(add(add(mul(p0, s * s * s), mul(p1, 3.0 * s * s * t)), add(mul(p2, 3.0 * s * t * t), mul(p3, t * t * t))));
                                }
                            }
                        }
                    }
                    if ring.len() >= 3 {
                        fr.em().ring(&ring);
                    }
                }
            }
        }
        fr.fill(color, RULE_NONZERO);
    }
}

/// a hatch family's pattern: the entries' ends (an odd count twice), the period, the count; None: solid lines
fn hatch_pattern(h: &Fam) -> Option<([f64; 12], f64, usize)> {
    let m = (h.dashes as usize).min(crate::scene::HATCH_DASH_MAX);
    if m == 0 {
        return None;
    }
    let n = if m % 2 == 1 { 2 * m } else { m };
    let mut ends = [0.0f64; 12];
    let mut sum = 0.0;
    for (i, e) in ends.iter_mut().enumerate().take(n) {
        sum += h.dash[i % m];
        *e = sum;
    }
    if !(sum > 0.0) {
        return None;
    }
    Some((ends, sum, n))
}

/// the share of a hatch family's lines its pattern covers (dots as long as the line is wide), as Style::hatch_words
fn hatch_duty(h: &Fam, hw: f64, sc: f64) -> f64 {
    let Some((ends, per, n)) = hatch_pattern(h) else { return 1.0 };
    let (mut on, mut prev) = (0.0, 0.0);
    for (i, &e) in ends[..n].iter().enumerate() {
        if i % 2 == 0 {
            on += (e - prev).max(2.0 * hw * sc);
        }
        prev = e;
    }
    (on / per).min(1.0)
}

/// rings turned by their nesting: those inside an even number of others counter-clockwise (drawing units, y up), the
/// others clockwise, so that a pixel with edges of both adds up even-odd as it should
fn orient(rings: &mut [Vec<P>]) {
    let boxes: Vec<Bx> = rings.iter().map(|r| geo::bx_of(r)).collect();
    let mut flip = vec![false; rings.len()];
    for i in 0..rings.len() {
        if rings[i].len() < 3 {
            continue;
        }
        let p = rings[i][0];
        let mut depth = 0;
        for j in 0..rings.len() {
            if j == i || rings[j].len() < 3 || !geo::bx_has(&boxes[j], p) {
                continue;
            }
            // (even-odd: a ray to the right)
            let r = &rings[j];
            let mut inside = false;
            for k in 0..r.len() {
                let (a, b) = (r[k], r[(k + 1) % r.len()]);
                if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) {
                    inside = !inside;
                }
            }
            depth += inside as u32;
        }
        let ccw = area(&rings[i]) > 0.0;
        flip[i] = ccw != (depth % 2 == 0);
    }
    for (r, f) in rings.iter_mut().zip(flip) {
        if f {
            r.reverse();
        }
    }
}

/// an arc (its units: centre, radius, start, sweep; a full turn or more: closed) as a centre line in pixels within ARC_TOL,
/// finely only where it comes near box bx (elsewhere chords, which the runs cut away), and its lengths along it (its
/// units).  Returns closed and the length all around
#[allow(clippy::too_many_arguments)]
fn arc_line(pl: &Place, c: P, r: f64, a0: f64, sw: f64, bx: &Bx, pts: &mut Vec<P>, lens: &mut Vec<f64>) -> (bool, f64) {
    pts.clear();
    lens.clear();
    let tau = core::f64::consts::TAU;
    let full = sw.abs() >= tau;
    let sw = if full { tau.copysign(sw) } else { sw };
    let rp = r * pl.smax();
    let at = |t: f64| {
        let (s, co) = sincos(t);
        pl.px([c[0] + r * co, c[1] + r * s])
    };
    #[allow(clippy::too_many_arguments)]
    fn rec(pl: &Place, c: P, r: f64, rp: f64, t0: f64, t1: f64, p1: P, a0: f64, bx: &Bx, depth: u32, at: &dyn Fn(f64) -> P, pts: &mut Vec<P>, lens: &mut Vec<f64>) {
        let half = 0.5 * (t1 - t0);
        let (sq, _) = sincos(0.5 * half);
        let sag = rp * 2.0 * sq * sq;
        let p0 = *pts.last().unwrap_or(&p1);
        // (the hull: both ends and where the tangents there meet)
        let ch = sincos(half).1;
        let (sm, cm) = sincos(t0 + half);
        let ctrl = pl.px([c[0] + r / ch * cm, c[1] + r / ch * sm]);
        let hb = geo::bx_of(&[p0, p1, ctrl]);
        let meets = !(hb[2] < bx[0] || hb[0] > bx[2] || hb[3] < bx[1] || hb[1] > bx[3]);
        if !meets || sag <= ARC_TOL || depth >= 56 {
            pts.push(p1);
            lens.push(r * (t1 - a0).abs());
            return;
        }
        let tm = t0 + half;
        let pm = at(tm);
        rec(pl, c, r, rp, t0, tm, pm, a0, bx, depth + 1, at, pts, lens);
        rec(pl, c, r, rp, tm, t1, p1, a0, bx, depth + 1, at, pts, lens);
    }
    pts.push(at(a0));
    lens.push(0.0);
    let k = (sw.abs() / (0.5 * core::f64::consts::PI)).ceil_f().max(1.0) as usize;
    for i in 0..k {
        let (t0, t1) = (a0 + sw * i as f64 / k as f64, a0 + sw * (i + 1) as f64 / k as f64);
        let p1 = at(t1);
        rec(pl, c, r, rp, t0, t1, p1, a0, bx, 0, &at, pts, lens);
    }
    let total = r * sw.abs();
    if full && pts.len() >= 2 {
        pts.pop();
        lens.pop();
    }
    (full, total)
}

trait CeilF {
    fn ceil_f(self) -> f64;
}

impl CeilF for f64 {
    fn ceil_f(self) -> f64 {
        -floor(-self)
    }
}
