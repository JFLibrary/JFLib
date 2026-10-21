// path -- vector paths: build, flatten (tolerance 0.2 px), fill        [src/gfx/path.s]
//
// Integer API in 24.8 fixed point relative to the draw origin (move_fx, line_fx, quad_fx,
// cubic_fx, rect_fx, rrect_fx, ellipse_fx), float API in absolute target pixels (move_f,
// line_f, quad_f, add_pt; used by the text engine).  Curves are flattened at once; fill()
// hands the closed subpaths to the rasterizer, stroke() (stroke.rs) their outlines.
// The origin is taken from the Dc at begin() (set_origin changes it).

use alloc::vec::Vec;

use super::raster::Raster;
use super::{Dc, Surface};
use rt::fx::{maxss, sqrt};

const KAPPA_FX: i32 = 141; // 0.5523 * 256 (circle approximation by cubics)

#[derive(Clone, Copy)]
pub(crate) struct Sub {
    pub start: usize,
    pub count: usize,
    pub closed: bool,
}

#[derive(Default)]
pub struct Path {
    pub(crate) pts: Vec<(f32, f32)>,
    pub(crate) subs: Vec<Sub>,
    open: bool,
    start: (f32, f32),
    cur: (f32, f32),
    ox: i32,
    oy: i32,
    tolerance: f32,
}

/// ucomiss + je: equal or unordered
#[inline]
pub(crate) fn zf(a: f32, b: f32) -> bool {
    !(a < b || a > b)
}

// clamp(ceil(sqrt(m * k)), 1, 128)
fn seg_count(m: f32, k: f32) -> i32 {
    let v = sqrt(m * k);
    let mut n = rt::fx::cvtt(v);
    if !((n as f32) >= v) {
        n += 1;
    }
    n.clamp(1, 128)
}

fn vlen(x: f32, y: f32) -> f32 {
    sqrt(x * x + y * y)
}

impl Path {
    /// Set curve flattening tolerance in target pixels and return the old value.
    /// Invalid values preserve the setting; 0.2 is the legacy default.
    pub fn set_tolerance(&mut self, tolerance: f32) -> f32 {
        let old = if self.tolerance > 0.0 { self.tolerance } else { 0.2 };
        if tolerance.is_finite() && tolerance > 0.0 {
            self.tolerance = tolerance.clamp(0.001, 10.0);
        }
        old
    }

    fn curve_factor(&self) -> f32 {
        if self.tolerance > 0.0 { 0.2 / self.tolerance } else { 1.0 }
    }
    /// path_begin: an empty path; ox, oy = the draw origin used by the integer API
    pub fn begin(&mut self, dc: &Dc) {
        self.pts.clear();
        self.subs.clear();
        self.open = false;
        self.ox = dc.ox;
        self.oy = dc.oy;
    }

    /// an empty path, origin kept (path_begin for the float API)
    pub fn reset(&mut self) {
        self.pts.clear();
        self.subs.clear();
        self.open = false;
    }

    pub fn set_origin(&mut self, ox: i32, oy: i32) {
        self.ox = ox;
        self.oy = oy;
    }

    pub fn is_empty(&self) -> bool {
        self.subs.is_empty()
    }

    /// the flattened points of every subpath (target pixels), e.g. to shear them before a
    /// fill (the asm's g_pts / g_npts)  // paint-tools: Kritzel's italic text
    pub fn pts_mut(&mut self) -> &mut [(f32, f32)] {
        &mut self.pts
    }

    fn to_xy(&self, x: i32, y: i32) -> (f32, f32) {
        (x as f32 * (1.0 / 256.0) + self.ox as f32, y as f32 * (1.0 / 256.0) + self.oy as f32)
    }

    fn new_sub(&mut self, p: (f32, f32)) {
        self.subs.push(Sub { start: self.pts.len(), count: 0, closed: false });
        self.open = true;
        self.start = p;
    }

    /// path_add_pt: append to the open subpath (a new one starts at the current point
    /// if none is open); a duplicate of the previous point is skipped
    pub fn add_pt(&mut self, p: (f32, f32)) {
        if !self.open {
            if !self.subs.is_empty() {
                let c = self.cur;
                self.new_sub(c);
                self.add_pt(c);
            } else {
                self.new_sub(p);
            }
        }
        let si = self.subs.len() - 1;
        if self.subs[si].count != 0 {
            let last = self.pts[self.pts.len() - 1];
            if zf(p.0, last.0) && zf(p.1, last.1) {
                return;
            }
        }
        self.pts.push(p);
        self.subs[si].count += 1;
        self.cur = p;
    }

    pub fn move_fx(&mut self, x: i32, y: i32) {
        let p = self.to_xy(x, y);
        self.new_sub(p);
        self.add_pt(p);
    }

    pub fn move_f(&mut self, x: f32, y: f32) {
        self.new_sub((x, y));
        self.add_pt((x, y));
    }

    pub fn line_fx(&mut self, x: i32, y: i32) {
        let p = self.to_xy(x, y);
        self.add_pt(p);
    }

    pub fn line_f(&mut self, x: f32, y: f32) {
        self.add_pt((x, y));
    }

    pub fn close(&mut self) {
        if self.open {
            let si = self.subs.len() - 1;
            self.subs[si].closed = true;
            self.open = false;
            self.cur = self.start;
        }
    }

    /// path_quad (24.8 fixed, origin-relative)
    pub fn quad_fx(&mut self, cx: i32, cy: i32, x: i32, y: i32) {
        let c = self.to_xy(cx, cy);
        let p = self.to_xy(x, y);
        self.quad_f(c.0, c.1, p.0, p.1);
    }

    /// path_quad_f: quadratic Bezier from the current point (float pixels)
    pub fn quad_f(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        if self.subs.is_empty() {
            self.new_sub((cx, cy));
            self.add_pt((cx, cy));
        }
        let (p0x, p0y) = self.cur;
        let mx = p0x - (cx + cx) + x;
        let my = p0y - (cy + cy) + y;
        let n = seg_count(vlen(mx, my), 1.25 * self.curve_factor());
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let b0 = u * u;
            let mut b1 = t * u;
            b1 = b1 + b1;
            let b2 = t * t;
            let mut px = p0x * b0 + cx * b1 + x * b2;
            let mut py = p0y * b0 + cy * b1 + y * b2;
            if i == n {
                px = x;
                py = y;
            }
            self.add_pt((px, py));
        }
    }

    /// path_cubic (24.8 fixed, origin-relative)
    pub fn cubic_fx(&mut self, c1x: i32, c1y: i32, c2x: i32, c2y: i32, x: i32, y: i32) {
        let p1 = self.to_xy(c1x, c1y);
        let p2 = self.to_xy(c2x, c2y);
        let p3 = self.to_xy(x, y);
        self.cubic_f(p1, p2, p3);
    }

    /// a cubic Bezier from the current point in float pixels (the flattening of path_cubic)
    pub fn cubic_f(&mut self, p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)) {
        if self.subs.is_empty() {
            self.new_sub(p1);
            self.add_pt(p1);
        }
        let p0 = self.cur;
        let m1 = vlen(p0.0 - (p1.0 + p1.0) + p2.0, p0.1 - (p1.1 + p1.1) + p2.1);
        let m2 = vlen(p1.0 - (p2.0 + p2.0) + p3.0, p1.1 - (p2.1 + p2.1) + p3.1);
        let n = seg_count(maxss(m2, m1), 3.75 * self.curve_factor());
        for i in 1..=n {
            if i == n {
                self.add_pt(p3);
                break;
            }
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let uu = u * u;
            let b0 = uu * u;
            let b1 = uu * t * 3.0;
            let tt = t * t;
            let b2 = tt * u * 3.0;
            let b3 = tt * t;
            let px = p0.0 * b0 + p1.0 * b1 + p2.0 * b2 + p3.0 * b3;
            let py = p0.1 * b0 + p1.1 * b1 + p2.1 * b2 + p3.1 * b3;
            self.add_pt((px, py));
        }
    }

    pub fn rect_fx(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let x1 = x + w;
        let y1 = y + h;
        self.move_fx(x, y);
        self.line_fx(x1, y);
        self.line_fx(x1, y1);
        self.line_fx(x, y1);
        self.close();
    }

    /// path_rrect: closed rounded rectangle, r clamped to min(w, h) / 2
    pub fn rrect_fx(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32) {
        let (x0, y0, x1, y1) = (x, y, x + w, y + h);
        let r = r.min(w.min(h) >> 1);
        if r <= 0 {
            self.rect_fx(x, y, w, y1 - y0);
            return;
        }
        let k = (r * (256 - KAPPA_FX)) >> 8;
        self.move_fx(x0 + r, y0);
        self.line_fx(x1 - r, y0);
        self.cubic_fx(x1 - k, y0, x1, y0 + k, x1, y0 + r);
        self.line_fx(x1, y1 - r);
        self.cubic_fx(x1, y1 - k, x1 - k, y1, x1 - r, y1);
        self.line_fx(x0 + r, y1);
        self.cubic_fx(x0 + k, y1, x0, y1 - k, x0, y1 - r);
        self.line_fx(x0, y0 + r);
        self.cubic_fx(x0, y0 + k, x0 + k, y0, x0 + r, y0);
        self.close();
    }

    /// path_ellipse: closed ellipse from four cubic arcs
    pub fn ellipse_fx(&mut self, cx: i32, cy: i32, rx: i32, ry: i32) {
        let kx = (rx * KAPPA_FX) >> 8;
        let ky = (ry * KAPPA_FX) >> 8;
        self.move_fx(cx + rx, cy);
        self.cubic_fx(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry);
        self.cubic_fx(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy);
        self.cubic_fx(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry);
        self.cubic_fx(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy);
        self.close();
    }

    /// the line list of path_fill: every subpath of 2+ points closed implicitly
    pub fn fill_lines(&self, ras: &mut Raster) {
        ras.lines_begin();
        for s in &self.subs {
            if s.count < 2 {
                continue;
            }
            for k in 0..s.count {
                let a = self.pts[s.start + k];
                let b = if k + 1 < s.count { self.pts[s.start + k + 1] } else { self.pts[s.start] };
                ras.emit_line(a.0, a.1, b.0, b.1);
            }
        }
    }

    /// path_fill: every subpath closed implicitly; rule 0 non-zero, 1 even-odd
    pub fn fill(&self, ras: &mut Raster, dc: &Dc, sf: &mut Surface, rule: u32) {
        self.fill_lines(ras);
        ras.fill(dc, sf, rule);
    }
}
