//! Brush stroke rasterization and configurable bitmap tips.
pub(crate) mod custom;
mod presets;
mod preset_ui;

// brush_engine -- PaintCraft: stroke coverage of the brush-like tools (agent T1).  A stroke
// is one A8 coverage TMASK of the image size. Dabs default to max(); imported presets
// can accumulate coverage up to a separate opacity cap. The colour is blended once
// through the tool layer (tool.rs TL).
// Reused by the clone stamp / recolor tools (contract: inc/pn_tooli.inc).
//                                              [apps/PaintCraft/src/brush_engine.s]
//
//   be_begin(doc, &BEP) -> bool               new stroke of doc's size (mask cleared)
//   be_dab(x_fx, y_fx, pressure)              first dab at the press point (24.8 image px)
//   be_to(x_fx, y_fx, pressure) -> bool       dabs along the path from the last point (every
//                                             spacing % of the width, the remainder carried to
//                                             the next segment); BEF_SMOOTH: along a
//                                             centripetal Catmull-Rom spline through the points
//                                             (one point behind the pointer)
//   be_finish() -> bool                       smoothed strokes: the last piece (release)
//   be_dirty() -> Option<[i32; 4]>            area changed since the last call (cleared)
//   be_box() -> Option<[i32; 4]>              bounding box of the whole stroke
//   be_mask() -> Option<&TMask>   be_end()    the stroke mask; free it
//   be_ui_pressure(inp, i) -> pressure        (PN1) pressure of the UiInput's move point i
//                                             (i < 0 or >= the point count: the pointer now)
// pressure: 0 = none (full width), else 1 .. BE_PRESSURE_ONE (with BEF_PRESSURE).  (PN1)
// Without smoothing the radius is interpolated along a segment from the last point's
// pressure to the new one; the dab step follows the smaller of the two radii.
//
// Dab coverage (antialiased, BEF_AA): d = distance of the pixel centre from the dab centre,
// r = radius, R = r + 1/2, inner = r * hardness, edge = clamp(R - d, 0, 1),
// s = smoothstep(clamp((R - d) / (R - inner), 0, 1)), coverage = min(edge, s) (f32, 4
// pixels per step as the SSE code).  Hard dabs (no AA): w = max(1, round(2 r)); the centre
// snaps to a pixel centre (odd w) or a pixel corner (even w); coverage 255 where d < w / 2.
// BEF_PENCIL: 1 px Bresenham lines between the integer pixels of the points.  Whole 64 x 64
// tiles inside the full-coverage radius become full without per-pixel work.
//
// Float code keeps the asm's operation order (rt::fx: cvtd / cvttd / maxsd / minsd / sqrt).

use gfx::tmask::{Span, TMask};
use rt::fx::{cvtd, cvtt, cvttd, maxsd, maxss, minsd, minss, sqrt, sqrtd};
use ui::{UiInput, UI_PRESSURE_ONE};

use crate::document::model::DocId;
use crate::app::Pn;

const BEF_AA: u32 = crate::app::constants::BEF_AA as u32;
const BEF_PENCIL: u32 = crate::app::constants::BEF_PENCIL as u32;
const BEF_PRESSURE: u32 = crate::app::constants::BEF_PRESSURE as u32;
const BEF_SMOOTH: u32 = crate::app::constants::BEF_SMOOTH as u32;
const BE_PRESSURE_ONE: u32 = crate::app::constants::BE_PRESSURE_ONE as u32;

const TILE: i32 = 64;
const C_INV256: f64 = 0.00390625;
const C_PCT: f64 = 0.01;
const C_PONE: f64 = 0.0009765625; // 1 / BE_PRESSURE_ONE
const C_RMIN: f64 = 0.25; // smallest dab radius
const C_STEPMIN: f64 = 0.25; // smallest dab distance (px)
const C_EPS: f64 = 0.000001;
const C_KMIN: f64 = 0.001;
const C_WMIN: f64 = 1.0;
const C_BIGR: f64 = 40.0; // radius from which whole tiles are filled at once
const C_NEARLY1: f64 = 0.999999;
const C_KMIN4: f64 = 0.0001;

/// brush engine parameters (BEP)
#[derive(Clone, Copy, Default)]
pub(crate) struct Bep {
    /// BEP_WIDTH: width (image px)
    pub(crate) width: f64,
    /// BEP_HARD: hardness 0 .. 100
    pub(crate) hard: u32,
    /// BEP_SPACING: spacing 1 .. 100 % of the width
    pub(crate) spacing: u32,
    /// BEP_FLAGS: BEF_*
    pub(crate) flags: u32,
}

/// the state of brush_engine.s (its g_* globals)
#[derive(Default)]
pub(crate) struct Be {
    dynamics: crate::tools::brushes::custom::Dynamics,
    random: u32,
    direction: f64,
    /// the stroke coverage (g_be_mask)
    pub(crate) mask: Option<TMask>,
    width: f64,
    lx: f64, // last point
    ly: f64,
    walk_x: f64, // target of walk
    walk_y: f64,
    sp: [[f64; 2]; 4], // smoothed path: the last 4 points
    rem: f64,          // distance to the next dab (< 0: one step)
    dab_cx: f64,
    dab_cy: f64,
    dab_r: f64,
    dirty: [i32; 4], // since be_dirty
    bx: [i32; 4],    // the whole stroke
    w: i32,
    h: i32,
    hard: u32,
    spc: i32,
    flags: u32,
    px: i32, // pencil: last pixel
    py: i32,
    sn: u32, // points in sp
    lp: i32, // (PN1) pressure of the last point
}

/// floor the way flr does it: truncate, then step down (cvttsd2si)
#[inline]
fn flr(x: f64) -> i32 {
    let t = cvttd(x);
    if (t as f64) > x { t - 1 } else { t }
}

/// be_ui_pressure(ui, i): (PN1) the library's pen pressure (0 none, 1..UI_PRESSURE_ONE) of
/// move point i, rounded to the engine's scale (at least 1 while there is pressure); i < 0 or
/// >= the point count: the pointer now
pub(crate) fn be_ui_pressure(inp: &UiInput, i: i32) -> u32 {
    let mut p = inp.pressure;
    if i >= 0 && (i as usize) < inp.pts.len() {
        if let Some(&v) = inp.ppress.get(i as usize) {
            p = v;
        }
    }
    if p == 0 {
        return 0;
    }
    let p = p.min(UI_PRESSURE_ONE);
    ((p * BE_PRESSURE_ONE + UI_PRESSURE_ONE / 2) / UI_PRESSURE_ONE).max(1)
}

impl Be {
    /// be_begin(doc, &BEP): a new stroke of w x h (the mask is reused when the size fits);
    /// false when the mask cannot be made
    fn begin(&mut self, w: i32, h: i32, bep: &Bep) -> bool {
        let reuse = matches!(&self.mask, Some(m) if m.w as i32 == w && m.h as i32 == h);
        if reuse {
            if let Some(m) = self.mask.as_mut() {
                m.clear(0);
            }
        } else {
            self.mask = None;
            self.mask = TMask::new(w as u32, h as u32);
            if self.mask.is_none() {
                return false;
            }
        }
        self.w = w;
        self.h = h;
        self.width = maxsd(bep.width, C_WMIN);
        self.hard = bep.hard.min(100);
        self.spc = (bep.spacing as i32).clamp(1, 100);
        self.flags = bep.flags;
        self.dynamics = Default::default();
        self.random = 0x50435246;
        self.direction = 0.0;
        self.dirty = [0; 4];
        self.bx = [0; 4];
        true
    }

    /// be_end(): free the stroke mask
    fn end(&mut self) {
        self.mask = None;
        self.dynamics.tip = None;
    }

    /// be_dirty(): the area changed since the last call (cleared), None = nothing changed
    fn dirty(&mut self) -> Option<[i32; 4]> {
        if self.dirty[0] >= self.dirty[2] {
            return None;
        }
        let r = self.dirty;
        self.dirty = [0; 4];
        Some(r)
    }

    /// be_box(): the stroke's bounding box (None = empty)
    fn bbox(&self) -> Option<[i32; 4]> {
        if self.bx[0] < self.bx[2] { Some(self.bx) } else { None }
    }

    // be_union: grow the dirty area and the stroke box
    fn union(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        for r in [&mut self.dirty, &mut self.bx] {
            if r[0] >= r[2] {
                *r = [x0, y0, x1, y1];
            } else {
                r[0] = r[0].min(x0);
                r[1] = r[1].min(y0);
                r[2] = r[2].max(x1);
                r[3] = r[3].max(y1);
            }
        }
    }

    // radius(pressure): the dab radius (width / 2, scaled by the pressure)
    fn radius(&self, p: i32) -> f64 {
        let mut r = self.width * 0.5;
        if self.flags & BEF_PRESSURE != 0 && p > 0 && p < BE_PRESSURE_ONE as i32 {
            r = r * p as f64;
            r = r * C_PONE;
        }
        maxsd(r, C_RMIN)
    }

    /// be_dab(x_fx, y_fx, pressure): the first dab of a stroke
    fn dab(&mut self, x: i32, y: i32, p: u32) {
        let p = p as i32;
        self.lp = p;
        self.lx = x as f64 * C_INV256;
        self.ly = y as f64 * C_INV256;
        self.rem = -1.0; // < 0: the next dab one step away
        self.sp[0] = [self.lx, self.ly]; // the smoothed path starts here
        self.sn = 1;
        if self.flags & BEF_PENCIL != 0 {
            self.px = x >> 8;
            self.py = y >> 8;
            self.plot(self.px, self.py);
        } else {
            self.dab_r = self.radius(p);
            self.dab_cx = self.lx;
            self.dab_cy = self.ly;
            self.dab_raster();
        }
    }

    /// be_to(x_fx, y_fx, pressure) -> true when the mask changed
    fn to(&mut self, x: i32, y: i32, p: u32) -> bool {
        let p = p as i32;
        if self.flags & BEF_PENCIL != 0 {
            let (x, y) = (x >> 8, y >> 8);
            if x == self.px && y == self.py {
                return false;
            }
            self.line(self.px, self.py, x, y);
            self.px = x;
            self.py = y;
            return true;
        }
        let xf = x as f64 * C_INV256;
        let yf = y as f64 * C_INV256;
        if self.flags & BEF_SMOOTH == 0 {
            self.walk_x = xf;
            self.walk_y = yf;
            return self.walk(p);
        }
        // a new point of the smoothed path (points closer than 1/2 px to the last one are
        // skipped -- repeated passes of a frame bring the same point again)
        if self.sn != 0 {
            let l = self.sp[((self.sn - 1) & 3) as usize];
            let dx = xf - l[0];
            let dy = yf - l[1];
            let d2 = dx * dx + dy * dy;
            if !(d2 >= 0.25) {
                return false;
            }
        }
        if self.sn >= 4 {
            self.sp[0] = self.sp[1]; // drop the oldest point
            self.sp[1] = self.sp[2];
            self.sp[2] = self.sp[3];
            self.sn = 3;
        }
        self.sp[(self.sn & 3) as usize] = [xf, yf];
        self.sn += 1;
        let sp = self.sp;
        match self.sn {
            3 => self.spline_seg(sp[0], sp[0], sp[1], sp[2], p), // p0 -> p1 (p0 doubled)
            4 => self.spline_seg(sp[0], sp[1], sp[2], sp[3], p), // p1 -> p2
            _ => false,
        }
    }

    /// be_finish() -> true when the mask changed: a smoothed stroke draws its last piece
    fn finish(&mut self) -> bool {
        if self.flags & BEF_SMOOTH == 0 {
            return false;
        }
        let n = self.sn;
        self.sn = 0;
        let sp = self.sp;
        match n {
            2 => self.spline_seg(sp[0], sp[0], sp[1], sp[1], 0), // p0 p0 p1 p1
            3 => self.spline_seg(sp[0], sp[1], sp[2], sp[2], 0), // p0 p1 p2 p2
            4 => self.spline_seg(sp[1], sp[2], sp[3], sp[3], 0), // p1 p2 p3 p3
            _ => false,
        }
    }

    // spline_seg(p0, p1, p2, p3, pressure) -> changed: dabs along the centripetal
    // Catmull-Rom curve from p1 to p2 in pieces of about 2 px
    fn spline_seg(&mut self, p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], p: i32) -> bool {
        let t1 = knot(p0, p1); // t0 = 0
        let t21 = knot(p1, p2);
        let t2 = t21 + t1;
        let t32 = knot(p2, p3);
        let t3 = t32 + t2;
        let t31 = t3 - t1;
        // pieces: ceil(|p2 - p1| / 2), 1 .. 64
        let dx = p2[0] - p1[0];
        let dy = p2[1] - p1[1];
        let d = sqrtd(dx * dx + dy * dy) * 0.5 + C_NEARLY1;
        let steps = cvttd(d).clamp(1, 64);
        let mut changed = false;
        for i in 1..=steps {
            // t = t1 + (t2 - t1) * i / steps
            let t = (i as f64 / steps as f64) * t21 + t1;
            let a = t1 - t;
            let b = t2 - t;
            let c = t3 - t;
            let d1 = t - t1;
            let d2 = t - t2;
            let a1 = lerp2(a, p0, t, p1, t1);
            let a2 = lerp2(b, p1, d1, p2, t21);
            let a3 = lerp2(c, p2, d2, p3, t32);
            let b1 = lerp2(b, a1, t, a2, t2);
            let b2 = lerp2(c, a2, d1, a3, t31);
            let cc = lerp2(b, b1, d1, b2, t21);
            self.walk_x = cc[0];
            self.walk_y = cc[1];
            changed |= self.walk(p);
        }
        changed
    }

    // walk(pressure) -> changed: dabs along the straight line from the last point to
    // (walk_x, walk_y), every spacing * width (at least 1/4 px); the distance left to the
    // next dab is carried to the next call
    fn walk(&mut self, p: i32) -> bool {
        let dx = self.walk_x - self.lx;
        let dy = self.walk_y - self.ly;
        if dx != 0.0 || dy != 0.0 { self.direction = rt::x87::atan2(dy, dx); }
        let dist = sqrtd(dx * dx + dy * dy);
        if !(dist > C_EPS) {
            return false;
        }
        // (PN1) pressure changes along the segment: interpolate the radius per dab, the
        // step follows the smaller pressure
        let mut interp = false;
        let mut dp = 0;
        let mut rp = p;
        if self.flags & BEF_PRESSURE != 0 && self.lp > 0 && p > 0 && p != self.lp {
            dp = p - self.lp;
            interp = true;
            rp = rp.min(self.lp);
        }
        let r = self.radius(rp);
        self.dab_r = r;
        let fraction = if self.dynamics.density != 0 { 1.0 / self.dynamics.density as f64 } else { self.spc as f64 * C_PCT };
        let step = maxsd((r + r) * fraction, C_STEPMIN);
        let mut t = self.rem;
        if !(t >= 0.0) {
            t = step; // first segment: one step from the press point
        }
        let mut changed = false;
        while !(t > dist) {
            let f = t / dist;
            self.dab_cx = f * dx + self.lx;
            self.dab_cy = f * dy + self.ly;
            if interp {
                let q = cvtd((t / dist) * dp as f64) + self.lp; // lp + (p - lp) * t / dist
                self.dab_r = self.radius(q);
            }
            self.dab_raster();
            changed = true;
            t = t + step;
        }
        self.rem = t - dist;
        self.lx = self.walk_x;
        self.ly = self.walk_y;
        self.lp = p;
        changed
    }

    // plot(x, y): pencil pixel, coverage 255
    fn plot(&mut self, x: i32, y: i32) {
        if x as u32 >= self.w as u32 || y as u32 >= self.h as u32 {
            return;
        }
        let Some(m) = self.mask.as_mut() else { return };
        let Some(row) = m.row_w(x, y) else { return };
        row[0] = 255;
        self.union(x, y, x + 1, y + 1);
    }

    // line(x0, y0, x1, y1): Bresenham, both ends plotted
    fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let (mut x, mut y) = (x0, y0);
        let dx = (x1 - x0).abs();
        let sx = if x1 - x0 >= 0 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y1 - y0 >= 0 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.plot(x, y);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = err + err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    // dab_raster(): one dab at (dab_cx, dab_cy) with radius dab_r into the stroke mask (max)
    fn dab_raster(&mut self) {
        if self.dynamics.tip.is_some() || self.dynamics.scatter != 0 || self.dynamics.size_jitter != 0 || self.dynamics.flow != 100 || self.dynamics.accumulate || self.dynamics.flow_byte.is_some() || self.dynamics.opacity_cap != 255 {
            self.custom_dab();
            return;
        }
        if self.mask.is_none() {
            return;
        }
        let mut cx = self.dab_cx;
        let mut cy = self.dab_cy;
        let mut r = self.dab_r;
        let hard = self.flags & BEF_AA == 0;
        let full: f64; // full coverage: d^2 < full
        let kf: f32; // k (AA) / r^2 (hard)
        let r2: f64; // extent radius^2
        if hard {
            // hard dab: w = max(1, round(2r)), centre snapped
            let w = cvtd(r + r).max(1);
            r = w as f64 * 0.5;
            if w & 1 != 0 {
                cx = flr(cx) as f64 + 0.5;
                cy = flr(cy) as f64 + 0.5;
            } else {
                cx = flr(cx + 0.5) as f64;
                cy = flr(cy + 0.5) as f64;
            }
            r2 = r * r;
            full = r2;
            kf = r2 as f32;
        } else {
            // R = r + 1/2, inner = r * hardness, k = 1 / max(R - inner, 1e-3)
            let inner = self.hard as f64 * C_PCT * r;
            let fr = maxsd(minsd(r - 0.5, inner), 0.0);
            full = fr * fr;
            r = r + 0.5;
            let k = 1.0 / maxsd(r - inner, C_KMIN);
            kf = k as f32;
            r2 = r * r;
        }
        let rf = r as f32;
        // bounding box: floor(c - R) - 1 .. floor(c + R) + 2, clipped
        let x0 = (flr(cx - r) - 1).max(0);
        let y0 = (flr(cy - r) - 1).max(0);
        let x1 = (flr(cx + r) + 2).min(self.w);
        let y1 = (flr(cy + r) + 2).min(self.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        self.union(x0, y0, x1, y1);
        let cxf = cx as f32;
        let Some(m) = self.mask.as_mut() else { return };
        // big dabs: whole tiles inside the full radius become full
        if r >= C_BIGR {
            let mut ty = y0 & -TILE;
            while ty < y1 {
                let mut tx = x0 & -TILE;
                while tx < x1 {
                    // farthest pixel centre of the tile from the centre
                    let ax = maxsd(((tx as f64 + 0.5) - cx).abs(), ((tx as f64 + 63.5) - cx).abs());
                    let ay = maxsd(((ty as f64 + 0.5) - cy).abs(), ((ty as f64 + 63.5) - cy).abs());
                    let d2 = ax * ax + ay * ay;
                    if !(d2 >= full) {
                        m.tile_set((tx >> 6) as u32, (ty >> 6) as u32, true);
                    }
                    tx += TILE;
                }
                ty += TILE;
            }
        }
        let mut cov = [0u8; 64];
        for y in y0..y1 {
            // dy^2 and the x extent of this row
            let dy = (y as f64 + 0.5) - cy;
            let dy2 = dy * dy;
            let dy2f = dy2 as f32;
            let rest = r2 - dy2;
            if !(rest >= 0.0) {
                continue; // the row misses the dab
            }
            let hw = sqrtd(rest);
            let mut x = flr((cx - hw) - 0.5).max(x0);
            let xb = (flr((cx + hw) - 0.5) + 2).min(x1);
            while x < xb {
                // a full tile: skip to its end
                if let (Span::Full, _) = m.span(x, y, 1) {
                    x = (x & -TILE) + TILE;
                    continue;
                }
                let Some(dst) = m.row_w(x, y) else { return }; // out of memory: stop
                let n = ((xb - x) as usize).min(dst.len()).min(64);
                // coverage of n pixels from x, 4 at a time (lanes as the SSE code)
                let v = (x as f32 + 0.5f32) - cxf;
                let mut lane = [v + 0.0, v + 1.0, v + 2.0, v + 3.0];
                let mut i = 0;
                while i < n {
                    for k in 0..4 {
                        let d2 = lane[k] * lane[k] + dy2f;
                        let c = if hard {
                            if d2 < kf { 255.0f32 } else { 0.0 }
                        } else {
                            let e = rf - sqrt(d2);
                            let edge = minss(maxss(e, 0.0), 1.0);
                            let s = minss(maxss(e * kf, 0.0), 1.0);
                            let sm = (3.0 - (s + s)) * s * s;
                            minss(edge, sm) * 255.0 + 0.5
                        };
                        if i + k < 64 {
                            cov[i + k] = cvtt(c).clamp(0, 255) as u8;
                        }
                        lane[k] = lane[k] + 4.0;
                    }
                    i += 4;
                }
                for (d, &c) in dst[..n].iter_mut().zip(&cov[..n]) {
                    if c > *d {
                        *d = c;
                    }
                }
                x += n as i32;
            }
        }
    }
}

impl Be {
    pub(crate) fn configure(&mut self, dynamics: &crate::tools::brushes::custom::Dynamics) {
        self.dynamics = dynamics.clone();
        let density = self.dynamics.density;
        if density != 0 && self.spc != ((100 + density/2)/density).max(1) as i32 { self.dynamics.density = 0; }
    }
    fn rand(&mut self) -> f64 {
        self.random ^= self.random << 13; self.random ^= self.random >> 17; self.random ^= self.random << 5;
        self.random as f64 / u32::MAX as f64
    }
    fn custom_dab(&mut self) {
        let r = (self.dab_r * (1.0-self.rand()*self.dynamics.size_jitter as f64*0.01)).max(C_RMIN);
        let spread = self.dab_r*2.0*self.dynamics.scatter as f64*0.01;
        let cx = self.dab_cx + (self.rand()*2.0-1.0)*spread;
        let cy = self.dab_cy + (self.rand()*2.0-1.0)*spread;
        let angle = (self.dynamics.angle as f64+(self.rand()*2.0-1.0)*self.dynamics.angle_jitter as f64)*core::f64::consts::PI/180.0
            + if self.dynamics.follow != 0 { self.direction } else { 0.0 };
        let (cs, sn) = rt::x87::sincos(angle);
        let extent = r*1.415+1.0;
        let (x0, y0) = ((flr(cx-extent)).max(0), (flr(cy-extent)).max(0));
        let (x1, y1) = ((flr(cx+extent)+1).min(self.w), (flr(cy+extent)+1).min(self.h));
        if x0 >= x1 || y0 >= y1 { return; }
        self.union(x0,y0,x1,y1);
        let Some(mask) = self.mask.as_mut() else { return; };
        let tip = self.dynamics.tip.as_deref();
        let aspect = tip.map_or(1.0, |t| t.h as f64 / t.w as f64);
        let rx = if aspect > 1.0 { r/aspect } else { r };
        let ry = if aspect < 1.0 { r*aspect } else { r };
        for y in y0..y1 {
            let mut x = x0;
            while x < x1 {
                let Some(row) = mask.row_w(x,y) else { return; };
                let n = row.len().min((x1-x) as usize);
                for i in 0..n {
                    let dx = (x+i as i32) as f64+0.5-cx; let dy = y as f64+0.5-cy;
                    let u = (dx*cs+dy*sn)/(2.0*rx)+0.5;
                    let v = (-dx*sn+dy*cs)/(2.0*ry)+0.5;
                    let coverage = if let Some(tip) = tip { tip.sample(u,v) as u32 } else {
                        let distance = sqrtd(dx*dx+dy*dy);
                        let edge = (r+0.5-distance).clamp(0.0,1.0);
                        let t = ((r+0.5-distance)/(r+0.5-r*self.hard as f64*0.01).max(0.001)).clamp(0.0,1.0);
                        (255.0*(t*t*(3.0-2.0*t)).min(edge)+0.5) as u32
                    };
                    let alpha = if let Some(flow) = self.dynamics.flow_byte { (coverage*flow as u32+127)/255 } else { (coverage*self.dynamics.flow as u32+50)/100 };
                    row[i] = (if self.dynamics.accumulate { (row[i] as u32 + (alpha*(255-row[i] as u32)+127)/255) as u8 } else { row[i].max(alpha as u8) }).min(self.dynamics.opacity_cap);
                }
                x += n as i32;
            }
        }
    }
}

// knot(pa, pb): max(|pb - pa|^(1/2), 1e-4) (centripetal parameter step)
fn knot(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    maxsd(sqrtd(sqrtd(dx * dx + dy * dy)), C_KMIN4)
}

// LERP2: (wa * a + wb * b) / den for both coordinates
#[inline]
fn lerp2(wa: f64, a: [f64; 2], wb: f64, b: [f64; 2], den: f64) -> [f64; 2] {
    [(a[0] * wa + b[0] * wb) / den, (a[1] * wa + b[1] * wb) / den]
}

// ------------------------------------------------------------------ the module API

/// be_begin(doc, &BEP) -> true = a new stroke of doc's size (mask cleared; be_mask)
pub(crate) fn be_begin(p: &mut Pn, doc: DocId, bep: &Bep) -> bool {
    let Some(d) = p.doc.get(doc) else { return false };
    let (w, h) = (d.w, d.h);
    p.be.begin(w, h, bep)
}

/// be_dab(x_fx, y_fx, pressure) -> true -- the first dab of a stroke
pub(crate) fn be_dab(be: &mut Be, x_fx: i32, y_fx: i32, pressure: u32) -> bool {
    be.dab(x_fx, y_fx, pressure);
    true
}

/// be_to(x_fx, y_fx, pressure) -> true when the mask changed
pub(crate) fn be_to(be: &mut Be, x_fx: i32, y_fx: i32, pressure: u32) -> bool {
    be.to(x_fx, y_fx, pressure)
}

/// be_finish() -> true when the mask changed -- a smoothed stroke draws its last piece
pub(crate) fn be_finish(be: &mut Be) -> bool {
    be.finish()
}

/// be_dirty() -> the changed area (cleared), None = nothing changed
pub(crate) fn be_dirty(be: &mut Be) -> Option<[i32; 4]> {
    be.dirty()
}

/// be_box() -> the stroke's bounding box, None = empty
pub(crate) fn be_box(be: &Be) -> Option<[i32; 4]> {
    be.bbox()
}

/// be_mask() -> the stroke mask
pub(crate) fn be_mask(be: &Be) -> Option<&TMask> {
    be.mask.as_ref()
}

/// be_end() -- free the stroke mask
pub(crate) fn be_end(be: &mut Be) {
    be.end();
}
