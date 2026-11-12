// scene -- a scene's entities, layers, styles and camera, and what goes up of them (lib.rs has the overview,
// docs/RENDER2D.md the reasons)
//
// On the CPU: one Ent per entity with its f64 geometry, one LayerSlot per layer.  On the GPU: one record of 64 bytes
// per segment (common.wgsl Record) in one pool.  Every layer owns a region of the pool, whole pages of 256 records:
// its entities' records lie there in the order of insertion, each entity's together, so the pages of the visible
// layers in their draw order are the painter's order the visibility keeps (cull.wgsl).  An insert appends at its
// layer's end; an update in place when its records fit what the entity holds (alloc), else the layer is laid out
// anew with room for it (repack); a remove leaves free records (zeros), and a layer whose free records outgrow its
// live ones is laid out anew without them.  A region that is too small, a repack, a new or removed layer: a new
// pool, the GPU copies what stays across (no copy within one buffer: WebGPU forbids it), only changed entities'
// records come from the CPU.
//
// What a frame sends (prepare, then Renderer::record copies it inside the frame's commands, behind the barrier
// against the frame before): the view; the styles when they changed; the visible layers' pages and table when they
// changed; the changed records, in runs (few runs: a copy each) or through the scatter shader (many).  A prepare
// that never reached a submission: the next sends everything again.

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;

use alloc::boxed::Box;

use crate::curve::{self, Curve, Flat};
use crate::fill;
use crate::math::{sincos, split, sqrt};
use crate::{Renderer, Tag, CHANGE_WORDS, DRAW_BYTES, HATCH_BYTES, HATCH_WORDS, LAYERS_MAX, PAGE, RECORD_BYTES, RECORD_WORDS, STAMPS, STYLE_BYTES, STYLE_WORDS, VIEW_BYTES};

/// what a scene shows: the point of the drawing at the target's middle, drawing units per pixel, and the rotation
/// (radians, counter-clockwise: the drawing turns that way on the screen).  The y axis points up on the screen
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub center: [f64; 2],
    pub scale: f64,
    pub rotation: f64,
}

impl Camera {
    pub const fn new(center: [f64; 2], scale: f64) -> Camera {
        Camera { center, scale, rotation: 0.0 }
    }

    /// a point of the drawing in pixels of a target of `size` (x right, y down from its top left corner; a pixel's
    /// centre at + 0.5)
    pub fn to_screen(&self, p: [f64; 2], size: (u32, u32)) -> [f64; 2] {
        let (s, c) = sincos(self.rotation);
        let (x, y) = (p[0] - self.center[0], p[1] - self.center[1]);
        let (rx, ry) = ((c * x - s * y) / self.scale, (s * x + c * y) / self.scale);
        [0.5 * size.0 as f64 + rx, 0.5 * size.1 as f64 - ry]
    }

    /// the point of the drawing under a pixel position of a target of `size`
    pub fn to_drawing(&self, q: [f64; 2], size: (u32, u32)) -> [f64; 2] {
        let (s, c) = sincos(self.rotation);
        let (rx, ry) = ((q[0] - 0.5 * size.0 as f64) * self.scale, (0.5 * size.1 as f64 - q[1]) * self.scale);
        [self.center[0] + c * rx + s * ry, self.center[1] - s * rx + c * ry]
    }

    /// the drawing follows a pointer that moved by (dx, dy) pixels (a drag)
    pub fn pan(&mut self, dx: f64, dy: f64) {
        let (s, c) = sincos(self.rotation);
        let (rx, ry) = (dx * self.scale, -dy * self.scale);
        self.center = [self.center[0] - (c * rx + s * ry), self.center[1] - (-s * rx + c * ry)];
    }

    /// zoom in by `factor` (below 1: out) about the pixel position q of a target of `size`: the point under q stays
    pub fn zoom_at(&mut self, q: [f64; 2], factor: f64, size: (u32, u32)) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let p = self.to_drawing(q, size);
        self.scale /= factor;
        let (s, c) = sincos(self.rotation);
        let (rx, ry) = ((q[0] - 0.5 * size.0 as f64) * self.scale, (0.5 * size.1 as f64 - q[1]) * self.scale);
        self.center = [p[0] - (c * rx + s * ry), p[1] - (-s * rx + c * ry)];
    }

    fn check(&self) -> Result<()> {
        let fine = self.center.iter().all(|v| v.is_finite() && v.abs() < 1e30) && self.scale.is_finite() && self.scale > 0.0 && self.rotation.is_finite();
        if fine { Ok(()) } else { Err(GpuError::Invalid("Camera: a finite centre, a scale above 0, a finite rotation")) }
    }

    /// common.wgsl's View for a target of `size`: the centre as hi + lo and what that leaves of it, the scale, the
    /// rotation, the size
    fn words(&self, size: (u32, u32), all: bool) -> [f32; 16] {
        let (hx, lx) = split(self.center[0]);
        let (hy, ly) = split(self.center[1]);
        let rx = ((self.center[0] - hx as f64) - lx as f64) as f32;
        let ry = ((self.center[1] - hy as f64) - ly as f64) as f32;
        let (s, c) = sincos(self.rotation);
        [hx, hy, lx, ly, rx, ry, (1.0 / self.scale) as f32, self.scale as f32, c as f32, s as f32, size.0 as f32, size.1 as f32, all as u32 as f32, 0.0, 0.0, 0.0]
    }
}

/// how a style's width is measured
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    /// pixels of the target, whatever the zoom
    Px,
    /// units of the drawing: the stroke grows and shrinks with the zoom
    Drawing,
}

/// how an open end ends
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cap {
    /// at the end point
    Butt,
    /// a half circle around it
    Round,
    /// half the width beyond it
    Square,
}

/// how two segments of a polyline meet on the outer side of the turn
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Join {
    /// the outer edges meet, up to the miter limit; beyond it a bevel
    Miter,
    /// a circle around the vertex
    Round,
    /// a straight cut between the outer corners
    Bevel,
}

/// the most entries of a dash pattern (Style::dash)
pub const DASH_MAX: usize = 8;
/// the most line families of a hatch (Style::hatch)
pub const HATCH_MAX: usize = 4;
/// the most entries of a hatch family's dash pattern (HatchLine::dash)
pub const HATCH_DASH_MAX: usize = 6;

/// a family of a hatch's parallel lines, as DXF has them: their angle (radians, counter-clockwise from the x axis), the
/// base point one of them runs through, the spacing between neighbouring lines (at right angles to them) and how far each
/// line's pattern is shifted along against the one before; a dash pattern (on, off, ... in drawing units; none: solid
/// lines; an entry of 0 is a dot).  Made with `new` and changed field by field (it may grow)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HatchLine {
    pub angle: f64,
    pub base: [f64; 2],
    pub spacing: f64,
    pub shift: f64,
    pub dash: [f64; HATCH_DASH_MAX],
    pub dashes: u32,
}

impl HatchLine {
    /// solid lines at an angle, `spacing` apart, through the origin
    pub const fn new(angle: f64, spacing: f64) -> HatchLine {
        HatchLine { angle, base: [0.0, 0.0], spacing, shift: 0.0, dash: [0.0; HATCH_DASH_MAX], dashes: 0 }
    }

    /// the same with a dash pattern (at most HATCH_DASH_MAX entries, drawing units)
    pub fn dashed(mut self, pattern: &[f64]) -> HatchLine {
        self.dash = [0.0; HATCH_DASH_MAX];
        for (d, p) in self.dash.iter_mut().zip(pattern) {
            *d = *p;
        }
        self.dashes = pattern.len() as u32;
        self
    }

    fn check(&self) -> bool {
        let fine = |v: f64| v.is_finite() && v.abs() < 1e30;
        let pat = &self.dash[..(self.dashes as usize).min(HATCH_DASH_MAX)];
        fine(self.angle) && fine(self.base[0]) && fine(self.base[1]) && fine(self.shift) && self.spacing.is_finite() && self.spacing > 0.0 && self.spacing < 1e30
            && self.dashes as usize <= HATCH_DASH_MAX && pat.iter().all(|&v| fine(v) && v >= 0.0) && (pat.is_empty() || pat.iter().sum::<f64>() > 0.0)
    }
}

/// a stroke's look.  Made with `new` and changed field by field (it may grow)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// 0xAARRGGBB: sRGB codes as the target shows them, straight alpha
    pub color: u32,
    /// in `unit`s; 0 (or anything thinner than a pixel): a hairline, 1 px wide
    pub width: f32,
    pub unit: Unit,
    pub cap: Cap,
    pub join: Join,
    /// the longest miter over the width (lib/gfx's and SVG's ratio); at least 1
    pub miter_limit: f32,
    /// a dash pattern: lengths on and off in turn, starting with on, the first `dashes` entries (0: solid).  An odd count
    /// runs twice (on, off, on, then off, on, off), as in SVG; every dash ends in `cap`, joins only where a dash runs
    /// through a vertex.  The pattern runs along each entity from its start (a closed polyline's first point, an arc's
    /// start, a circle's angle 0), across the joins of a polyline
    pub dash: [f32; DASH_MAX],
    pub dashes: u32,
    /// the unit of the pattern's lengths and of its phase
    pub dash_unit: Unit,
    /// how far into the pattern an entity starts (in `dash_unit`s)
    pub dash_phase: f32,
    /// a fill's hatch: the first `hatches` line families (0: the fill is solid), drawn in `color` over nothing, the lines
    /// `hatch_width` px wide (0, or thinner than a pixel: 1 px); strokes ignore it
    pub hatch: [HatchLine; HATCH_MAX],
    pub hatches: u32,
    pub hatch_width: f32,
}

impl Style {
    /// a hairline of a colour: width 0, butt caps, miter joins with the limit 4, solid
    pub const fn new(color: u32) -> Style {
        Style { color, width: 0.0, unit: Unit::Px, cap: Cap::Butt, join: Join::Miter, miter_limit: 4.0, dash: [0.0; DASH_MAX], dashes: 0, dash_unit: Unit::Px,
                dash_phase: 0.0, hatch: [HatchLine::new(0.0, 1.0); HATCH_MAX], hatches: 0, hatch_width: 0.0 }
    }

    /// the same with a hatch (at most HATCH_MAX families; none: a solid fill) of lines `width` px wide
    pub fn hatched(mut self, families: &[HatchLine], width: f32) -> Style {
        self.hatch = [HatchLine::new(0.0, 1.0); HATCH_MAX];
        for (d, f) in self.hatch.iter_mut().zip(families) {
            *d = *f;
        }
        self.hatches = families.len() as u32;
        self.hatch_width = width;
        self
    }

    /// the same with a dash pattern (at most DASH_MAX entries; an empty one: solid), its unit and phase
    pub fn dashed(mut self, pattern: &[f32], unit: Unit, phase: f32) -> Style {
        self.dash = [0.0; DASH_MAX];
        for (d, p) in self.dash.iter_mut().zip(pattern) {
            *d = *p;
        }
        self.dashes = pattern.len() as u32;
        self.dash_unit = unit;
        self.dash_phase = phase;
        self
    }

    fn check(&self) -> Result<()> {
        if !(self.width.is_finite() && self.width >= 0.0 && self.miter_limit.is_finite() && self.miter_limit >= 1.0) {
            return Err(GpuError::Invalid("Style: a finite width, not negative, and a miter limit of at least 1"));
        }
        if self.dashes as usize > DASH_MAX {
            return Err(GpuError::Invalid("Style: at most DASH_MAX dash entries"));
        }
        let pat = &self.dash[..self.dashes as usize];
        if pat.iter().any(|v| !(v.is_finite() && *v >= 0.0 && *v < 1e30)) || !self.dash_phase.is_finite() || (!pat.is_empty() && !(pat.iter().map(|&v| v as f64).sum::<f64>() > 0.0)) {
            return Err(GpuError::Invalid("Style: dash entries finite and not negative, not all 0, a finite phase"));
        }
        if self.hatches as usize > HATCH_MAX || !(self.hatch_width.is_finite() && self.hatch_width >= 0.0) || !self.hatch[..self.hatches as usize].iter().all(|h| h.check()) {
            return Err(GpuError::Invalid("Style: at most HATCH_MAX hatch families of a spacing above 0, finite numbers, dash entries not negative and not all 0, a width not negative"));
        }
        Ok(())
    }

    /// a dash pattern in pixels: its words change with the scale
    fn px_dashes(&self) -> bool {
        self.dashes > 0 && self.dash_unit == Unit::Px
    }

    /// fill.wgsl's Hatch of this style for a camera and a target (scene.rs computes the lines' places at the camera in f64,
    /// the shader goes on from there in pixels: docs/RENDER2D.md "Schraffuren")
    fn hatch_words(&self, cam: &Camera, size: (u32, u32)) -> [f32; HATCH_WORDS] {
        let mut w = [0.0f32; HATCH_WORDS];
        let n = (self.hatches as usize).min(HATCH_MAX);
        w[0] = n as f32;
        w[1] = (0.5 * self.hatch_width).max(0.5);
        let (sr, cr) = sincos(cam.rotation);
        // a direction of the drawing in pixels: turned, y down
        let px = |v: [f64; 2]| [cr * v[0] - sr * v[1], -(sr * v[0] + cr * v[1])];
        let _ = size;
        let hw = w[1] as f64;
        for (f, h) in self.hatch[..n].iter().enumerate() {
            let (sa, ca) = sincos(h.angle);
            let dir = [ca, sa];
            let nrm = [-sa, ca];
            let (cx, cy) = (cam.center[0] - h.base[0], cam.center[1] - h.base[1]);
            // the line nearest the camera's centre, k0, and the centre's distance from it
            let u = (cx * nrm[0] + cy * nrm[1]) / h.spacing;
            let k0 = round(u);
            let d0 = (cx * nrm[0] + cy * nrm[1]) - k0 * h.spacing;
            let (np, dp) = (px(nrm), px(dir));
            let o = 4 + 24 * f;
            w[o] = np[0] as f32;
            w[o + 1] = np[1] as f32;
            w[o + 2] = (d0 / cam.scale) as f32;
            w[o + 3] = (h.spacing / cam.scale) as f32;
            w[o + 4] = dp[0] as f32;
            w[o + 5] = dp[1] as f32;
            let m = (h.dashes as usize).min(HATCH_DASH_MAX);
            let m2 = if m % 2 == 1 { 2 * m } else { m };
            let period: f64 = (0..m2).map(|k| h.dash[k % m]).sum();
            let mut duty = 1.0;
            if m2 > 0 && period > 0.0 {
                // the place along line k0 at the centre's foot and the shift between lines, within a period, the pattern's
                // ends (drawing units: as precise as a stroke's pattern, docs/RENDER2D.md "Schraffuren")
                let t0 = (cx * dir[0] + cy * dir[1]) - k0 * h.shift;
                w[o + 6] = (t0 - (t0 / period).floor_f() * period) as f32;
                w[o + 7] = period as f32;
                w[o + 8] = (h.shift - (h.shift / period).floor_f() * period) as f32;
                let (mut sum, mut on) = (0.0f64, 0.0f64);
                for k in 0..m2 {
                    let v = h.dash[k % m];
                    sum += v;
                    if k % 2 == 0 {
                        // (a dot as long as the line is wide)
                        on += v.max(2.0 * hw * cam.scale);
                    }
                    if k < 12 {
                        w[o + 12 + k] = sum as f32;
                    }
                }
                duty = (on / period).min(1.0);
                w[o + 10] = m2.min(12) as f32;
            }
            w[o + 9] = duty as f32;
        }
        w
    }

    /// common.wgsl's Style at a camera's scale; hatch: its place in the hatch table + 1 (0: none)
    fn words(&self, scale: f64, hatch: u32) -> [u32; STYLE_WORDS] {
        let unit = |u: Unit| match u {
            Unit::Px => 0,
            Unit::Drawing => 1,
        };
        let cap = match self.cap {
            Cap::Butt => 0,
            Cap::Round => 1,
            Cap::Square => 2,
        };
        let join = match self.join {
            Join::Miter => 0,
            Join::Round => 1,
            Join::Bevel => 2,
        };
        let mut w = [0u32; STYLE_WORDS];
        let n = if self.dashes % 2 == 1 { 2 * self.dashes } else { self.dashes } as usize;
        w[0] = self.color;
        w[1] = self.width.to_bits();
        w[2] = unit(self.unit) | cap << 4 | join << 8 | (n as u32) << 12 | unit(self.dash_unit) << 20;
        w[3] = self.miter_limit.to_bits();
        if n > 0 {
            // the entries' ends from the period's start, the period, the share on and the dashes per period
            let (mut sum, mut on, mut ons) = (0.0f64, 0.0f64, 0u32);
            for k in 0..n {
                let v = self.dash[k % self.dashes as usize] as f64;
                sum += v;
                if k % 2 == 0 {
                    on += v;
                    ons += 1;
                }
                w[12 + k] = (sum as f32).to_bits();
            }
            let period = f32::from_bits(w[12 + n - 1]) as f64;
            // periods per drawing unit at this scale, as three f32 (stroke.wgsl: the pattern's place from a length along the entity)
            let k = 1.0 / (period * if self.dash_unit == Unit::Px { scale } else { 1.0 });
            let (k0, k1, k2) = split3(k);
            let p = self.dash_phase as f64 / period;
            w[4] = k0.to_bits();
            w[5] = k1.to_bits();
            w[6] = k2.to_bits();
            w[7] = (frac(p) as f32).to_bits();
            w[8] = (period as f32).to_bits();
            w[9] = ((on / period) as f32).to_bits();
            w[10] = (ons as f32).to_bits();
        }
        w[11] = hatch;
        w
    }
}

/// x rounded to the nearest whole number (halves away from 0; beyond 2^52 x itself)
fn round(x: f64) -> f64 {
    if !(x.abs() < 4.5e15) {
        return x;
    }
    (x + 0.5).floor_f()
}

trait FloorF {
    fn floor_f(self) -> f64;
}

impl FloorF for f64 {
    /// floor without libm
    fn floor_f(self) -> f64 {
        if !(self.abs() < 4.5e15) {
            return self;
        }
        let t = self as i64 as f64;
        if t > self { t - 1.0 } else { t }
    }
}

/// x as three f32 whose sum is x (exactly: f64 has 53 bits)
pub(crate) fn split3(x: f64) -> (f32, f32, f32) {
    let a = x as f32;
    let r = x - a as f64;
    let b = r as f32;
    (a, b, (r - b as f64) as f32)
}

/// x - floor(x), in [0, 1) (0 beyond 2^52: every f64 there is whole)
pub(crate) fn frac(x: f64) -> f64 {
    if !(x.abs() < 4.5e15) {
        return 0.0;
    }
    let t = x as i64 as f64;
    let f = x - if t > x { t - 1.0 } else { t };
    if f >= 1.0 { 0.0 } else { f }
}

/// a style of a Scene
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StyleId(pub u32);

/// a layer's settings.  Made with `new` and changed field by field (it may grow)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layer {
    pub visible: bool,
    /// layers are drawn by this, lowest first; equal ones in the order they were added
    pub order: i32,
}

impl Layer {
    pub const fn new(order: i32) -> Layer {
        Layer { visible: true, order }
    }
}

/// a layer of a Scene; dead after remove_layer
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerId {
    pub slot: u32,
    era: u32,
}

/// an entity of a Scene; dead after remove, also when its place has gone to another one
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityId {
    pub slot: u32,
    era: u32,
}

/// what an entity is (more kinds come: text)
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub enum Shape<'a> {
    Line { a: [f64; 2], b: [f64; 2] },
    /// at least one point; closed: a segment from the last back to the first, joined at every point
    Polyline { points: &'a [[f64; 2]], closed: bool },
    /// a circular arc: its centre, radius (above 0), the angle it starts at and how far it turns (radians; positive:
    /// counter-clockwise, the drawing's y pointing up; not 0).  A sweep of a full turn or more is a circle whose dash
    /// pattern starts at `start`.  Drawn in pieces of at most ARC_STEP, coverage from the circle itself
    Arc { center: [f64; 2], radius: f64, start: f64, sweep: f64 },
    /// a circle (its dash pattern starts at angle 0 and runs counter-clockwise)
    Circle { center: [f64; 2], radius: f64 },
    /// quadratic Bézier pieces end to end: the start, then per piece its control point and its end (1 + 2k points, k at
    /// least 1); closed: the end joins the start (a straight segment where they differ).  Curves are flattened into
    /// segments per zoom band within curve::FLAT_TOL px (docs/RENDER2D.md "Kurven"); the joins between pieces are the
    /// style's, the dash pattern runs along the curve's length
    Quadratic { points: &'a [[f64; 2]], closed: bool },
    /// cubic Bézier pieces: per piece its two control points and its end (1 + 3k points)
    Cubic { points: &'a [[f64; 2]], closed: bool },
    /// an elliptic arc: the point at the parameter t is center + R(rotation) (radii[0] cos t, radii[1] sin t), from t =
    /// start over `sweep` (radians, not 0; a full turn or more: the closed ellipse, starting at `start`); radii above 0
    Ellipse { center: [f64; 2], radii: [f64; 2], rotation: f64, start: f64, sweep: f64 },
    /// a B-spline of degree 1 to 3 (at least degree + 1 points), rational where `weights` has one positive weight per
    /// point (empty: all 1), on `knots` (empty: clamped and uniform, through the first and last point; else points +
    /// degree + 1 non-decreasing values, the curve from knots[degree] to knots[points]); closed with empty knots: periodic
    /// (smooth all round), else the end joins the start.  As DXF's SPLINE
    Spline { degree: u32, points: &'a [[f64; 2]], weights: &'a [f64], knots: &'a [f64], closed: bool },
    /// a filled area: its rings, each the shapes of the list end to end (a shape that does not start where the one before
    /// ended: a straight segment between them; the last back to the first), even-odd between rings (a ring inside a hole is
    /// an island).  Triangulated on the CPU, its edge smoothed as strokes are; the style's colour, or its hatch
    /// (docs/RENDER2D.md "Flächen")
    Fill { rings: &'a [&'a [Shape<'a>]] },
}

/// the largest angle of one piece of an arc (a record): a full circle is 16 of them.  Its quad covers the piece's ring
/// between the chord inside and the tangent outside, so that the pixels the fragment shader looks at follow the stroke
/// (docs/RENDER2D.md "Bögen und Kreise")
pub const ARC_STEP: f64 = core::f64::consts::PI / 8.0;

/// what prepare sent for this frame
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadInfo {
    /// records that came from the CPU (changed entities, free places), and in how many runs
    pub records: u32,
    pub runs: u32,
    /// they went through the scatter shader (many runs), else a copy per run
    pub scatter: bool,
    /// a new pool (a layout changed), and the records the GPU copied across into it
    pub rebuild: bool,
    pub moved: u64,
    /// the visible layers' pages and table went up
    pub tables: bool,
    /// everything prepare wrote for the GPU, the view included
    pub bytes: u64,
}

/// what a finished frame measured (Scene::set_stats)
#[derive(Clone, Copy, Debug)]
pub struct FrameStats {
    /// counts the scene's frames from 0
    pub frame: u64,
    /// records the visibility found in the target
    pub visible: u64,
    /// the GPU's clock (ms) at the start, after the uploads, after the visibility, after the draw (None: no
    /// timestamps here)
    pub stamps: Option<[f64; 4]>,
}

#[derive(Clone)]
enum Geom {
    Line([[f64; 2]; 2]),
    Poly(Vec<[f64; 2]>, bool),
    /// centre, radius, start, sweep (a full circle: +-TAU)
    Arc([f64; 2], f64, f64, f64),
    /// a curve and its flattening at the zoom band it was made for
    Curve(Box<CurveG>),
    /// a fill: its rings and their flattening, triangulated
    Fill(Box<FillG>),
}

/// how a curve or fill was last flattened: its zoom band, the tolerance's coarsening (times 4 each), the window (None:
/// all of it)
#[derive(Clone, Copy, Debug, PartialEq)]
struct FlatState {
    band: i32,
    coarse: i32,
    window: Option<[f64; 4]>,
}

#[derive(Clone)]
struct CurveG {
    c: Curve,
    flat: Flat,
    st: FlatState,
    /// the curve's box (its control hulls)
    bounds: [f64; 4],
}

#[derive(Clone)]
struct FillG {
    rings: Vec<Curve>,
    prep: fill::Prepared,
    st: FlatState,
    bounds: [f64; 4],
}

/// a curve's or fill's flattening state and box
fn flat_meta(g: &Geom) -> Option<(FlatState, [f64; 4])> {
    match g {
        Geom::Curve(c) => Some((c.st, c.bounds)),
        Geom::Fill(f) => Some((f.st, f.bounds)),
        _ => None,
    }
}

/// the box of curves' boxes
fn bounds_of<'a>(cs: impl Iterator<Item = &'a Curve>) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for c in cs {
        let o = c.bounds();
        b = [b[0].min(o[0]), b[1].min(o[1]), b[2].max(o[2]), b[3].max(o[3])];
    }
    b
}

/// the camera as the flattening sees it: its scale, the target's box in the drawing (axis-aligned), the target's larger
/// side (px)
struct FlatCtx {
    scale: f64,
    view: [f64; 4],
    side: f64,
}

impl FlatCtx {
    fn new(cam: &Camera, size: (u32, u32)) -> FlatCtx {
        let size = if size.0 == 0 || size.1 == 0 { (1920, 1080) } else { size };
        // (the target's corners in the drawing: Camera::to_drawing with one sincos)
        let (sn, cs) = sincos(cam.rotation);
        let mut v = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for q in [[0.0, 0.0], [size.0 as f64, 0.0], [0.0, size.1 as f64], [size.0 as f64, size.1 as f64]] {
            let (rx, ry) = ((q[0] - 0.5 * size.0 as f64) * cam.scale, (0.5 * size.1 as f64 - q[1]) * cam.scale);
            let p = [cam.center[0] + cs * rx + sn * ry, cam.center[1] - sn * rx + cs * ry];
            v = [v[0].min(p[0]), v[1].min(p[1]), v[2].max(p[0]), v[3].max(p[1])];
        }
        FlatCtx { scale: cam.scale, view: v, side: size.0.max(size.1) as f64 }
    }

    /// a flattening made for st no longer holds: the scale left its band (and the hysteresis), or the view (grown by
    /// reach px) left its window
    fn stale(&self, st: &FlatState, reach: f64) -> bool {
        if self.scale < curve::pow2(st.band) / curve::HYST || self.scale >= curve::pow2(st.band + 1) * curve::HYST {
            return true;
        }
        if let Some(w) = st.window {
            let m = reach * self.scale;
            let v = self.view;
            return v[0] - m < w[0] || v[1] - m < w[1] || v[2] + m > w[2] || v[3] + m > w[3];
        }
        false
    }

    /// a box (drawing units) meets the view grown by half the target's side and reach px: a curve outside keeps the
    /// flattening it has (whatever its band: nothing of it shows) until it comes that near
    fn near(&self, b: &[f64; 4], reach: f64) -> bool {
        let m = (0.5 * self.side + reach) * self.scale;
        let v = self.view;
        !(b[2] < v[0] - m || b[0] > v[2] + m || b[3] < v[1] - m || b[1] > v[3] + m)
    }

    /// a curve flattened for this camera: all of it within the band's tolerance, or (too many segments) finely in a window
    /// of the view and half the target's side around it, plus reach px
    fn flatten(&self, c: &Curve, reach: f64) -> (Flat, FlatState) {
        let band = curve::band_of(self.scale);
        let tol = curve::FLAT_TOL * curve::pow2(band);
        if let Some(f) = curve::flatten(c, tol, None, 0.0, curve::FULL_MAX + 1) {
            return (f, FlatState { band, coarse: 0, window: None });
        }
        let m = (0.5 * self.side + reach) * self.scale;
        let w = [self.view[0] - m, self.view[1] - m, self.view[2] + m, self.view[3] + m];
        let (mut t, mut leaf) = (tol, 0.25 * self.side * self.scale);
        for coarse in 0..40 {
            if let Some(f) = curve::flatten(c, t, Some(w), leaf, curve::CURVE_MAX + 1) {
                return (f, FlatState { band, coarse, window: Some(w) });
            }
            t *= 4.0;
            leaf *= 4.0;
        }
        // (never: a chord per span is far below the bound)
        let f = curve::flatten(c, f64::MAX, Some(w), f64::MAX, usize::MAX).unwrap_or_default();
        (f, FlatState { band, coarse: 40, window: Some(w) })
    }
}

/// a stroke's curve flattened for a camera: what reaches beyond its centre line (px) for the window
fn stroke_reach(st: &Style, scale: f64) -> f64 {
    let w = if st.unit == Unit::Drawing { st.width as f64 / scale } else { st.width as f64 };
    16.0 + w
}

/// a fill's reach beyond its outline (px): its smoothed edge
const FILL_REACH: f64 = 4.0;

#[derive(Clone)]
struct Ent {
    live: bool,
    era: u32,
    layer: u32,
    style: u32,
    /// its first record in its layer's region, the records it holds there, the records its geometry has
    at: u32,
    alloc: u32,
    count: u32,
    /// its records go up with the next prepare
    dirty: bool,
    /// which pipeline its layer needs (CLASS_*: an arc or a dashed style the second, a fill the third: lib.rs)
    class: u8,
    geom: Geom,
}

/// the pipelines a layer can take: solid segments only, arcs or dash patterns too, fills too (lib.rs)
pub(crate) const CLASS_PLAIN: u8 = 0;
pub(crate) const CLASS_FULL: u8 = 1;
pub(crate) const CLASS_FILL: u8 = 2;

/// a place of the layer region nobody holds
const NONE: u32 = u32::MAX;

struct LayerSlot {
    live: bool,
    era: u32,
    visible: bool,
    order: i32,
    /// when it was added: equal orders draw in this order
    seq: u64,
    /// its region in the pool (records), and how much of it is laid out (used) or free inside that (holes)
    base: u32,
    cap: u32,
    used: u32,
    holes: u32,
    /// who holds each laid-out record (NONE: free)
    owners: Vec<u32>,
    /// lay it out anew with the next prepare: entities that grew out of what they hold, free records dropped
    repack: bool,
    /// laid out anew by this prepare: its free places need no zeros
    repacked: bool,
    /// its live entities of CLASS_FULL and of CLASS_FILL: more than none, the layer takes fs_full or fs_fill
    full: u32,
    fills: u32,
}

/// the bind groups of the scene's buffers (made again when one of them is new)
pub(crate) struct Groups {
    pub count: BindGroup,
    pub scan: BindGroup,
    pub write: BindGroup,
    pub draw: BindGroup,
}

/// a frame in flight's own data
pub(crate) struct FrameSlot {
    pub stage: Buffer,
    changes: Option<Buffer>,
    styles: Option<Buffer>,
    hatch: Option<Buffer>,
    pub scatter_group: Option<BindGroup>,
    runs: Option<Buffer>,
    pub readback: Option<Buffer>,
    last: Option<(SubmitId, u64)>,
}

/// what the last prepare left for Renderer::record
#[derive(Default)]
pub(crate) struct Uploads {
    /// the old pool and the copies (source, destination, bytes) into the new one
    pub rebuild: Option<(Buffer, Vec<(u64, u64, u64)>)>,
    pub styles: Option<(Buffer, u64)>,
    /// the pages (bytes) and the layer table (bytes) behind them
    pub tables: Option<(Buffer, u64, u64)>,
    /// the hatch table (bytes)
    pub hatch: Option<(Buffer, u64)>,
    /// record runs: (source, destination, bytes)
    pub runs: Option<(Buffer, Vec<(u64, u64, u64)>)>,
    /// rows in the frame slot's change buffer
    pub scatter: u32,
}

fn storage(dev: &Device, size: u64, more: BufferUsage) -> Result<Buffer> {
    dev.create_buffer(&BufferDesc { size: size.max(16), usage: BufferUsage::STORAGE | more, memory: MemoryKind::Device })
}

fn entry(binding: u32, buffer: &Buffer) -> BindEntry<'_> {
    BindEntry { binding, resource: Resource::Buffer { buffer, offset: 0, size: 0 } }
}

fn upload(dev: &Device, size: u64) -> Result<Buffer> {
    dev.create_buffer(&BufferDesc { size: size.max(16), usage: BufferUsage::NONE, memory: MemoryKind::Upload })
}

fn pages(n: u32) -> u32 {
    n.div_ceil(PAGE) * PAGE
}

/// the records of the layer's layout that entity `slot` holds from `at` on (its alloc can be more: an update in the
/// middle of the layer raises it at once, the layout follows with the next prepare)
fn held(l: &LayerSlot, at: u32, slot: u32) -> u32 {
    l.owners[at as usize..].iter().take_while(|&&o| o == slot).count() as u32
}

/// entities, layers, styles and the camera of one scene of a Renderer
pub struct Scene {
    dev: Device,
    pub(crate) tag: Arc<Tag>,
    max_records: u32,
    max_entries: u32,
    styles: Vec<Style>,
    styles_dirty: bool,
    /// a style has a dash pattern in pixels: the styles go up again when the scale changes (their periods per unit), and
    /// the scale they last went up with
    px_dash: bool,
    styles_scale: f64,
    /// per style its place in the hatch table + 1 (0: no hatch), and the hatched styles in table order; the camera and
    /// target the table last went up for
    hatch_of: Vec<u32>,
    hatched: Vec<u32>,
    hatch_for: Option<(Camera, (u32, u32))>,
    pub(crate) hatch_buf: Buffer,
    hatch_cap: u32,
    /// per record of the pool: a fill edge's segment in pixels (cull.wgsl count writes it, fill.wgsl reads it); one record's
    /// worth while no layer holds a fill
    epx: Buffer,
    epx_cap: u32,
    /// the entities that are curves or fills (flattened per zoom band: prepare looks at them; dead or reused slots are
    /// skipped and dropped), and how many prepare flattened again
    curves: Vec<(u32, FlatState, [f64; 4], u32)>,
    curves_dirty: bool,
    reflat: u32,
    layers: Vec<LayerSlot>,
    free_layers: Vec<u32>,
    layer_seq: u64,
    ents: Vec<Ent>,
    free_ents: Vec<u32>,
    live: u32,
    dirty: Vec<u32>,
    /// free places to clear: (layer, its era, first, records)
    freed: Vec<(u32, u32, u32, u32)>,
    /// a region is too small, a layer came or went: a new pool
    layout_dirty: bool,
    camera: Camera,
    pub(crate) size: (u32, u32),
    culling: bool,
    pub(crate) stats_on: bool,
    pub(crate) pool: Buffer,
    pool_cap: u32,
    pub(crate) styles_buf: Buffer,
    styles_cap: u32,
    pub(crate) order_buf: Buffer,
    masks: Buffer,
    counts: Buffer,
    offsets: Buffer,
    entries_cap: u32,
    pub(crate) layers_buf: Buffer,
    pub(crate) lists_buf: Buffer,
    pub(crate) draws_buf: Buffer,
    layers_cap: u32,
    visible: Buffer,
    visible_cap: u32,
    pub(crate) view_ubo: Buffer,
    pub(crate) groups: Option<Groups>,
    pub(crate) frames: Vec<FrameSlot>,
    pub(crate) ts: Option<Timestamps>,
    /// the tables as they went up last, the visible layers in draw order
    last_order: Vec<u32>,
    last_layers: Vec<[u32; 4]>,
    drawn: Vec<u32>,
    /// per visible layer in draw order: the pipeline it takes (CLASS_*, Renderer::record)
    pub(crate) full: Vec<u8>,
    pub(crate) entries: u32,
    pub(crate) vis_layers: u32,
    pub(crate) up: Uploads,
    upload_info: UploadInfo,
    frame: u64,
    pub(crate) slot: usize,
    pub(crate) prepared: bool,
    /// a prepare has run since the last submission (what it sent has not reached the GPU)
    sent: bool,
    /// the scene changed after prepare
    pub(crate) stale: bool,
    waited: bool,
    /// the pool was ever filled (else the first prepare sends everything)
    filled: bool,
    finished: Vec<FrameStats>,
}

impl Scene {
    pub fn new(r: &Renderer) -> Result<Scene> {
        let dev = r.dev.clone();
        let none = BufferUsage::NONE;
        let (entries_cap, layers_cap) = (64u32, 16u32);
        let mut frames = Vec::new();
        for _ in 0..r.desc.frames_in_flight {
            frames.push(FrameSlot { stage: upload(&dev, VIEW_BYTES)?, changes: None, styles: None, hatch: None, scatter_group: None, runs: None, readback: None, last: None });
        }
        Ok(Scene {
            tag: r.tag.clone(),
            max_records: r.max_records,
            max_entries: r.max_entries,
            styles: Vec::new(),
            styles_dirty: false,
            px_dash: false,
            styles_scale: 0.0,
            hatch_of: Vec::new(),
            hatched: Vec::new(),
            hatch_for: None,
            hatch_buf: storage(&dev, HATCH_BYTES, none)?,
            hatch_cap: 1,
            epx: storage(&dev, 16, none)?,
            epx_cap: 1,
            curves: Vec::new(),
            curves_dirty: false,
            reflat: 0,
            layers: Vec::new(),
            free_layers: Vec::new(),
            layer_seq: 0,
            ents: Vec::new(),
            free_ents: Vec::new(),
            live: 0,
            dirty: Vec::new(),
            freed: Vec::new(),
            layout_dirty: true,
            camera: Camera::new([0.0, 0.0], 1.0),
            size: (0, 0),
            culling: true,
            stats_on: false,
            pool: storage(&dev, PAGE as u64 * RECORD_BYTES, none)?,
            pool_cap: PAGE,
            styles_buf: storage(&dev, 16 * STYLE_BYTES, none)?,
            styles_cap: 16,
            order_buf: storage(&dev, entries_cap as u64 * 4, none)?,
            masks: storage(&dev, entries_cap as u64 * 32, none)?,
            counts: storage(&dev, entries_cap as u64 * 4, none)?,
            offsets: storage(&dev, entries_cap as u64 * 4, none)?,
            entries_cap,
            layers_buf: storage(&dev, layers_cap as u64 * 16, none)?,
            lists_buf: storage(&dev, layers_cap as u64 * 16, none)?,
            draws_buf: storage(&dev, layers_cap as u64 * DRAW_BYTES, BufferUsage::INDIRECT)?,
            layers_cap,
            visible: storage(&dev, PAGE as u64 * 4, none)?,
            visible_cap: PAGE,
            view_ubo: dev.create_buffer(&BufferDesc { size: VIEW_BYTES, usage: BufferUsage::UNIFORM, memory: MemoryKind::Device })?,
            groups: None,
            frames,
            ts: None,
            last_order: Vec::new(),
            last_layers: Vec::new(),
            drawn: Vec::new(),
            full: Vec::new(),
            entries: 0,
            vis_layers: 0,
            up: Uploads::default(),
            upload_info: UploadInfo::default(),
            frame: 0,
            slot: 0,
            prepared: false,
            sent: false,
            stale: false,
            waited: false,
            filled: false,
            finished: Vec::new(),
            dev,
        })
    }

    fn touch(&mut self) {
        if self.prepared {
            self.stale = true;
        }
    }

    // ------------------------------------------------------------ styles

    /// a style for entities; the next frame shows it
    pub fn add_style(&mut self, s: &Style) -> Result<StyleId> {
        s.check()?;
        self.touch();
        self.styles.push(*s);
        self.styles_dirty = true;
        self.px_dash |= s.px_dashes();
        self.rehatch();
        Ok(StyleId(self.styles.len() as u32 - 1))
    }

    /// the hatch table's places of the styles
    fn rehatch(&mut self) {
        self.hatched.clear();
        self.hatch_of.clear();
        for (k, s) in self.styles.iter().enumerate() {
            if s.hatches > 0 {
                self.hatched.push(k as u32);
                self.hatch_of.push(self.hatched.len() as u32);
            } else {
                self.hatch_of.push(0);
            }
        }
        self.hatch_for = None;
    }

    /// a style changed: every entity of it shows the change with the next frame (nothing else goes up)
    pub fn set_style(&mut self, id: StyleId, s: &Style) -> Result<()> {
        s.check()?;
        let Some(old) = self.styles.get_mut(id.0 as usize) else { return Err(GpuError::Invalid("set_style: no such style")) };
        let was = old.dashes > 0;
        *old = *s;
        self.styles_dirty = true;
        if was != (s.dashes > 0) {
            // (dashed or no longer: its entities' layers may take the other pipeline)
            for i in 0..self.ents.len() {
                if self.ents[i].live && self.ents[i].style == id.0 {
                    let class = class_of(&self.ents[i].geom, s);
                    self.set_class(i, class);
                }
            }
        }
        self.px_dash = self.styles.iter().any(|s| s.px_dashes());
        self.rehatch();
        self.touch();
        Ok(())
    }

    pub fn style(&self, id: StyleId) -> Option<Style> {
        self.styles.get(id.0 as usize).copied()
    }

    // ------------------------------------------------------------ layers

    /// a new layer, empty, on top of those of its order
    pub fn add_layer(&mut self, l: &Layer) -> Result<LayerId> {
        self.touch();
        let seq = self.layer_seq;
        self.layer_seq += 1;
        let fresh = LayerSlot { live: true, era: 0, visible: l.visible, order: l.order, seq, base: 0, cap: 0, used: 0, holes: 0, owners: Vec::new(), repack: false, repacked: false, full: 0, fills: 0 };
        let slot = match self.free_layers.pop() {
            Some(s) => {
                let era = self.layers[s as usize].era;
                self.layers[s as usize] = LayerSlot { era, ..fresh };
                s
            }
            None => {
                self.layers.push(fresh);
                self.layers.len() as u32 - 1
            }
        };
        // (no region until it holds something)
        Ok(LayerId { slot, era: self.layers[slot as usize].era })
    }

    fn layer_index(&self, id: LayerId) -> Result<usize> {
        match self.layers.get(id.slot as usize) {
            Some(l) if l.live && l.era == id.era => Ok(id.slot as usize),
            _ => Err(GpuError::Invalid("Scene: this layer was removed")),
        }
    }

    /// shown or hidden, its place in the draw order (nothing of its entities goes up)
    pub fn set_layer(&mut self, id: LayerId, l: &Layer) -> Result<()> {
        let i = self.layer_index(id)?;
        self.touch();
        let s = &mut self.layers[i];
        if s.order != l.order {
            // (a new order: on top of the layers of that order)
            s.seq = self.layer_seq;
            self.layer_seq += 1;
        }
        s.visible = l.visible;
        s.order = l.order;
        Ok(())
    }

    pub fn layer(&self, id: LayerId) -> Result<Layer> {
        let l = &self.layers[self.layer_index(id)?];
        Ok(Layer { visible: l.visible, order: l.order })
    }

    /// the layer goes, with every entity on it
    pub fn remove_layer(&mut self, id: LayerId) -> Result<()> {
        let i = self.layer_index(id)?;
        self.touch();
        let owners = core::mem::take(&mut self.layers[i].owners);
        let mut last = NONE;
        for o in owners {
            if o != NONE && o != last && self.ents[o as usize].live {
                let e = &mut self.ents[o as usize];
                e.live = false;
                e.era = e.era.wrapping_add(1);
                self.free_ents.push(o);
                self.live -= 1;
            }
            last = o;
        }
        let l = &mut self.layers[i];
        l.live = false;
        l.era = l.era.wrapping_add(1);
        l.used = 0;
        l.holes = 0;
        l.cap = 0;
        // (its region stays unused until the pool is laid out again)
        self.free_layers.push(i as u32);
        Ok(())
    }

    /// the visible layers in the order the last prepare drew them (the lists' order: lists_buffer)
    pub fn drawn_layers(&self) -> Vec<LayerId> {
        self.drawn.iter().map(|&s| LayerId { slot: s, era: self.layers[s as usize].era }).collect()
    }

    // ------------------------------------------------------------ entities

    fn check_points(p: &[[f64; 2]]) -> Result<()> {
        if p.is_empty() {
            return Err(GpuError::Invalid("Shape: at least one point"));
        }
        if p.iter().all(|q| q[0].is_finite() && q[1].is_finite() && q[0].abs() < 1e30 && q[1].abs() < 1e30) {
            Ok(())
        } else {
            Err(GpuError::Invalid("Shape: finite points, below 1e30"))
        }
    }

    fn check_arc(center: [f64; 2], radius: f64, start: f64, sweep: f64) -> Result<()> {
        Scene::check_points(&[center])?;
        if !(radius.is_finite() && radius > 0.0 && radius < 1e30 && start.is_finite() && start.abs() < 1e15 && sweep.is_finite() && sweep != 0.0) {
            return Err(GpuError::Invalid("Shape: an arc of a finite radius above 0, a finite start and a sweep not 0"));
        }
        Ok(())
    }

    /// a curve shape (or one of a fill's ring) as a Curve; None: not a curve (lines, polylines, arcs, fills)
    fn curve(s: &Shape, ring: bool) -> Result<Option<Curve>> {
        let bad = |what: &'static str| Err(GpuError::Invalid(what));
        Ok(Some(match *s {
            Shape::Quadratic { points, closed } | Shape::Cubic { points, closed } => {
                let deg = if matches!(s, Shape::Quadratic { .. }) { 2 } else { 3 };
                Scene::check_points(points)?;
                if points.len() < deg + 1 || (points.len() - 1) % deg != 0 {
                    return bad("Shape: Bézier pieces of 1 + 2k (quadratic) or 1 + 3k (cubic) points, k at least 1");
                }
                Curve::bezier(points, deg, closed && !ring)
            }
            Shape::Ellipse { center, radii, rotation, start, sweep } => {
                Scene::check_points(&[center])?;
                let r = |v: f64| v.is_finite() && v > 0.0 && v < 1e30;
                if !(r(radii[0]) && r(radii[1]) && rotation.is_finite() && rotation.abs() < 1e15 && start.is_finite() && start.abs() < 1e15 && sweep.is_finite() && sweep != 0.0) {
                    return bad("Shape: an ellipse of finite radii above 0, a finite rotation and start, a sweep not 0");
                }
                Curve::ellipse(center, radii, rotation, start, sweep)
            }
            Shape::Spline { degree, points, weights, knots, closed } => {
                Scene::check_points(points)?;
                let p = degree as usize;
                let n = points.len();
                let w_ok = weights.is_empty() || (weights.len() == n && weights.iter().all(|w| w.is_finite() && *w > 0.0 && *w < 1e30));
                let k_ok = knots.is_empty() || (knots.len() == n + p + 1 && knots.iter().all(|k| k.is_finite() && k.abs() < 1e30) && knots.windows(2).all(|w| w[0] <= w[1]) && knots[p] < knots[n]);
                if !(1..=3).contains(&p) || n < p + 1 || !w_ok || !k_ok {
                    return bad("Shape: a spline of degree 1 to 3, at least degree + 1 points, a weight above 0 per point or none, points + degree + 1 knots not decreasing or none");
                }
                Curve::spline(p, points, weights, knots, closed && !(ring && !knots.is_empty()))
            }
            Shape::Line { a, b } if ring => {
                Scene::check_points(&[a, b])?;
                Curve::polyline(&[a, b], false)
            }
            Shape::Polyline { points, .. } if ring => {
                Scene::check_points(points)?;
                Curve::polyline(points, false)
            }
            Shape::Arc { center, radius, start, sweep } if ring => {
                Scene::check_arc(center, radius, start, sweep)?;
                Curve::ellipse(center, [radius, radius], 0.0, start, sweep)
            }
            Shape::Circle { center, radius } if ring => {
                Scene::check_arc(center, radius, 0.0, 1.0)?;
                Curve::ellipse(center, [radius, radius], 0.0, 0.0, core::f64::consts::TAU)
            }
            Shape::Fill { .. } if ring => return bad("Shape: a fill's ring is made of lines, polylines, arcs, circles and curves, not of fills"),
            _ => return Ok(None),
        }))
    }

    /// a shape's geometry; curves and fills flattened for this camera and target (the flattening's view only for them:
    /// it costs four sincos, a third of a plain insert)
    fn geom(s: &Shape, cam: &Camera, size: (u32, u32), st: &Style) -> Result<Geom> {
        if let Some(c) = Scene::curve(s, false)? {
            let ctx = FlatCtx::new(cam, size);
            let (flat, fs) = ctx.flatten(&c, stroke_reach(st, ctx.scale));
            let bounds = c.bounds();
            return Ok(Geom::Curve(Box::new(CurveG { c, flat, st: fs, bounds })));
        }
        match *s {
            Shape::Line { a, b } => {
                Scene::check_points(&[a, b])?;
                Ok(Geom::Line([a, b]))
            }
            Shape::Polyline { points, closed } => {
                Scene::check_points(points)?;
                Ok(Geom::Poly(points.to_vec(), closed))
            }
            Shape::Arc { center, radius, start, sweep } => {
                Scene::check_arc(center, radius, start, sweep)?;
                let tau = core::f64::consts::TAU;
                Ok(Geom::Arc(center, radius, start, if sweep.abs() >= tau { tau.copysign(sweep) } else { sweep }))
            }
            Shape::Circle { center, radius } => {
                Scene::check_arc(center, radius, 0.0, 1.0)?;
                Ok(Geom::Arc(center, radius, 0.0, core::f64::consts::TAU))
            }
            Shape::Fill { rings } => {
                let mut cs = Vec::with_capacity(rings.len());
                for ring in rings {
                    let mut c: Option<Curve> = None;
                    for piece in ring.iter() {
                        let Some(pc) = Scene::curve(piece, true)? else { return Err(GpuError::Invalid("Shape: a fill's ring is made of shapes")) };
                        match &mut c {
                            Some(c) => c.append(&pc),
                            None => c = Some(pc),
                        }
                    }
                    if let Some(c) = c {
                        cs.push(c.ring());
                    }
                }
                let (prep, fs) = flatten_fill(&cs, &FlatCtx::new(cam, size));
                let bounds = bounds_of(cs.iter());
                Ok(Geom::Fill(Box::new(FillG { rings: cs, prep, st: fs, bounds })))
            }
            _ => Err(GpuError::Invalid("Shape: a kind this scene does not know")),
        }
    }

    fn mark(&mut self, slot: u32) {
        let e = &mut self.ents[slot as usize];
        if !e.dirty {
            e.dirty = true;
            self.dirty.push(slot);
        }
    }

    /// a new entity at the end of its layer (drawn over everything there)
    pub fn insert(&mut self, layer: LayerId, style: StyleId, s: &Shape) -> Result<EntityId> {
        let li = self.layer_index(layer)?;
        if style.0 as usize >= self.styles.len() {
            return Err(GpuError::Invalid("Scene: a style it does not have"));
        }
        let geom = Scene::geom(s, &self.camera, self.size, &self.styles[style.0 as usize])?;
        let count = record_count(&geom);
        self.touch();
        let slot = match self.free_ents.pop() {
            Some(f) => f,
            None => {
                self.ents.push(Ent { live: false, era: 0, layer: 0, style: 0, at: 0, alloc: 0, count: 0, dirty: false, class: CLASS_PLAIN, geom: Geom::Line([[0.0; 2]; 2]) });
                self.ents.len() as u32 - 1
            }
        };
        if let Some(m) = flat_meta(&geom) {
            self.curves.push((slot, m.0, m.1, if matches!(geom, Geom::Fill(_)) { u32::MAX } else { style.0 }));
            self.curves_dirty = true;
        }
        let l = &mut self.layers[li];
        let at = l.used;
        l.used += count;
        l.owners.resize(l.used as usize, slot);
        if l.used > l.cap {
            self.layout_dirty = true;
        }
        let class = class_of(&geom, &self.styles[style.0 as usize]);
        l.full += (class == CLASS_FULL) as u32;
        l.fills += (class == CLASS_FILL) as u32;
        let e = &mut self.ents[slot as usize];
        e.live = true;
        e.layer = li as u32;
        e.style = style.0;
        e.at = at;
        e.alloc = count;
        e.count = count;
        e.class = class;
        e.geom = geom;
        let era = e.era;
        self.live += 1;
        self.mark(slot);
        Ok(EntityId { slot, era })
    }

    fn get(&self, id: EntityId) -> Result<usize> {
        match self.ents.get(id.slot as usize) {
            Some(e) if e.live && e.era == id.era => Ok(id.slot as usize),
            _ => Err(GpuError::Invalid("Scene: this entity was removed")),
        }
    }

    /// new geometry; the entity keeps its place in the draw order
    pub fn update(&mut self, id: EntityId, s: &Shape) -> Result<()> {
        let i = self.get(id)?;
        let geom = Scene::geom(s, &self.camera, self.size, &self.styles[self.ents[i].style as usize])?;
        if let Some(m) = flat_meta(&geom) {
            // (the later entry of a slot counts)
            self.curves.push((i as u32, m.0, m.1, if matches!(geom, Geom::Fill(_)) { u32::MAX } else { self.ents[i].style }));
            self.curves_dirty = true;
        }
        self.touch();
        self.regeom(i, geom);
        Ok(())
    }

    /// an entity's new geometry (update, a curve flattened again): in place when its records fit, else room for them
    fn regeom(&mut self, i: usize, geom: Geom) {
        let count = record_count(&geom);
        let (li, at, alloc) = (self.ents[i].layer as usize, self.ents[i].at, self.ents[i].alloc);
        if count > alloc {
            let l = &mut self.layers[li];
            if at + held(l, at, i as u32) == l.used {
                // the last of its layer: it grows in place
                l.used = at + count;
                l.owners.resize(l.used as usize, i as u32);
                if l.used > l.cap {
                    self.layout_dirty = true;
                }
                self.ents[i].alloc = count;
            } else {
                // room in the middle: the layer is laid out anew, with room to grow again
                l.repack = true;
                self.ents[i].alloc = count.max(2 * alloc);
            }
        }
        let class = class_of(&geom, &self.styles[self.ents[i].style as usize]);
        self.set_class(i, class);
        let e = &mut self.ents[i];
        e.geom = geom;
        e.count = count;
        self.mark(i as u32);
    }

    /// an entity needs another pipeline for its layer (CLASS_*)
    fn set_class(&mut self, i: usize, class: u8) {
        let e = &mut self.ents[i];
        if e.class != class {
            let l = &mut self.layers[e.layer as usize];
            l.full -= (e.class == CLASS_FULL) as u32;
            l.fills -= (e.class == CLASS_FILL) as u32;
            l.full += (class == CLASS_FULL) as u32;
            l.fills += (class == CLASS_FILL) as u32;
            e.class = class;
        }
    }

    /// another style: its records go up again
    pub fn set_entity_style(&mut self, id: EntityId, style: StyleId) -> Result<()> {
        let i = self.get(id)?;
        if style.0 as usize >= self.styles.len() {
            return Err(GpuError::Invalid("Scene: a style it does not have"));
        }
        self.touch();
        self.ents[i].style = style.0;
        // (a curve's reach for its window follows the style)
        self.curves_dirty |= matches!(self.ents[i].geom, Geom::Curve(_));
        let class = class_of(&self.ents[i].geom, &self.styles[style.0 as usize]);
        self.set_class(i, class);
        self.mark(i as u32);
        Ok(())
    }

    /// the entity goes; its records become free places (and its id dead)
    pub fn remove(&mut self, id: EntityId) -> Result<()> {
        let i = self.get(id)?;
        self.touch();
        self.set_class(i, CLASS_PLAIN);
        let e = &mut self.ents[i];
        e.live = false;
        e.era = e.era.wrapping_add(1);
        let (li, at) = (e.layer as usize, e.at);
        let l = &mut self.layers[li];
        let n = held(l, at, i as u32);
        for o in &mut l.owners[at as usize..(at + n) as usize] {
            *o = NONE;
        }
        l.holes += n;
        if !l.repack && l.holes >= 1024 && l.holes * 2 > l.used {
            l.repack = true;
        }
        self.freed.push((li as u32, l.era, at, n));
        self.free_ents.push(i as u32);
        self.live -= 1;
        Ok(())
    }

    /// entities alive
    pub fn len(&self) -> u32 {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// (tests, tools) an entity's records as the last prepare laid them out: its first record's place in the pool
    /// and the records of its geometry (a polyline of n points: n - 1, closed n; a point: 1)
    pub fn locate(&self, id: EntityId) -> Option<(u32, u32)> {
        let i = self.get(id).ok()?;
        let e = &self.ents[i];
        Some((self.layers[e.layer as usize].base + e.at, e.count))
    }

    pub fn set_camera(&mut self, c: &Camera) -> Result<()> {
        c.check()?;
        self.camera = *c;
        Ok(())
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    /// false: every live record of a visible layer is drawn (a test's comparison, a debugging aid)
    pub fn set_culling(&mut self, on: bool) {
        self.culling = on;
    }

    /// FrameStats of each frame: the records found visible (a small copy per frame), the GPU's times
    pub fn set_stats(&mut self, on: bool) {
        self.stats_on = on;
    }

    /// what the last prepare sent
    pub fn upload_info(&self) -> UploadInfo {
        self.upload_info
    }

    /// (tests, tools) after a frame's visibility: per visible layer in draw order (drawn_layers) its first place in the
    /// visible list and its count (16 bytes each); the visible list (record places in the pool); the indirect draws
    pub fn lists_buffer(&self) -> &Buffer {
        &self.lists_buf
    }

    pub fn visible_buffer(&self) -> &Buffer {
        &self.visible
    }

    pub fn draws_buffer(&self) -> &Buffer {
        &self.draws_buf
    }

    /// (tests, tools) the pool of records (common.wgsl Record, 64 bytes each)
    pub fn pool_buffer(&self) -> &Buffer {
        &self.pool
    }

    // ------------------------------------------------------------ a frame

    /// wait for the frame that last used this frame's slot, so its data can be written; prepare does it too.  The
    /// browser cannot wait, and needs not: its writes go in queue order
    pub fn wait_slot(&mut self) -> Result<()> {
        if self.waited {
            return Ok(());
        }
        self.waited = true;
        let slot = (self.frame % self.frames.len() as u64) as usize;
        if let Some((id, f)) = self.frames[slot].last {
            let done = self.dev.wait(id, 10_000_000_000)?;
            if !done && !cfg!(target_arch = "wasm32") {
                return Err(GpuError::Timeout);
            }
            if done {
                self.collect(slot, f);
            }
            self.frames[slot].last = None;
        }
        Ok(())
    }

    fn collect(&mut self, slot: usize, frame: u64) {
        if !self.stats_on {
            return;
        }
        let Some(rb) = &self.frames[slot].readback else { return };
        let mut lists = vec![0u32; rb.size() as usize / 4];
        if rb.read(0, &mut lists).is_err() {
            return;
        }
        let n = (self.vis_layers as usize).min(lists.len() / 4);
        let visible: u64 = (0..n).map(|k| lists[k * 4 + 1] as u64).sum();
        let stamps = self.ts.as_ref().and_then(|ts| ts.read_ms(slot as u32 * STAMPS, STAMPS).ok()).map(|v| [v[0], v[1], v[2], v[3]]);
        self.finished.push(FrameStats { frame, visible, stamps });
    }

    /// the finished frames' stats since the last call
    pub fn take_stats(&mut self) -> Vec<FrameStats> {
        core::mem::take(&mut self.finished)
    }

    /// after Device::wait_idle: the stats of what was still in flight
    pub fn collect_stats(&mut self) {
        for slot in 0..self.frames.len() {
            if let Some((_, f)) = self.frames[slot].last.take() {
                self.collect(slot, f);
            }
        }
        self.finished.sort_by_key(|s| s.frame);
    }

    /// a layer to lay out anew: its live entities in their order, side by side, each with what it holds; the copies
    /// (old place, new place, records) of those whose records stay
    fn repack_layer(&mut self, li: usize) -> Vec<(u32, u32, u32)> {
        let owners = core::mem::take(&mut self.layers[li].owners);
        let mut fresh = Vec::with_capacity(owners.len());
        let mut moves: Vec<(u32, u32, u32)> = Vec::new();
        let mut k = 0usize;
        while k < owners.len() {
            let o = owners[k];
            if o == NONE {
                k += 1;
                continue;
            }
            let e = &self.ents[o as usize];
            // (its records start here, k is its old place)
            let (old, n_old, n_new, dirty) = (k as u32, (owners[k..].iter().take_while(|&&x| x == o).count()) as u32, e.alloc, e.dirty);
            let at = fresh.len() as u32;
            fresh.resize(fresh.len() + n_new as usize, o);
            if !dirty {
                match moves.last_mut() {
                    Some(m) if m.0 + m.2 == old && m.1 + m.2 == at => m.2 += n_old,
                    _ => moves.push((old, at, n_old)),
                }
            }
            self.ents[o as usize].at = at;
            k += n_old as usize;
        }
        let l = &mut self.layers[li];
        l.used = fresh.len() as u32;
        l.owners = fresh;
        l.holes = 0;
        l.repack = false;
        l.repacked = true;
        moves
    }

    /// lay out what has to be laid out anew; a new pool when a region changes.  full: no copies, everything goes up
    fn layout(&mut self, full: bool) -> Result<()> {
        for l in &mut self.layers {
            l.repacked = false;
        }
        let repack = self.layers.iter().any(|l| l.live && l.repack);
        if !(self.layout_dirty || repack || full) {
            return Ok(());
        }
        // (old place, new place, records) per layer, local to its region
        let mut local: Vec<Option<Vec<(u32, u32, u32)>>> = vec![None; self.layers.len()];
        for li in 0..self.layers.len() {
            if self.layers[li].live && (self.layers[li].repack || (full && self.layers[li].holes > 0)) {
                local[li] = Some(self.repack_layer(li));
            }
        }
        // the regions: every live layer's, grown (doubling) where it is too small, packed in slot order
        let mut base = 0u64;
        let mut moves = Vec::new();
        for (li, l) in self.layers.iter_mut().enumerate() {
            if !l.live {
                continue;
            }
            if l.used == 0 && !l.repacked {
                // (nothing to hold: no region)
                l.cap = 0;
                continue;
            }
            let cap = if l.used > l.cap { pages(l.used.max(l.cap.saturating_mul(2)).max(PAGE)) } else if l.repacked && l.cap > 4 * l.used + PAGE { pages(2 * l.used).max(PAGE) } else { l.cap.max(PAGE) };
            // (what its old region held: a layer without one held nothing; records added since are sent anyway)
            if !full && self.filled && l.cap > 0 {
                let (ob, nb) = (l.base as u64, base);
                match &local[li] {
                    Some(m) => moves.extend(m.iter().map(|&(o, n, c)| ((ob + o as u64) * RECORD_BYTES, (nb + n as u64) * RECORD_BYTES, c as u64 * RECORD_BYTES))),
                    None if l.used > 0 => moves.push((ob * RECORD_BYTES, nb * RECORD_BYTES, l.used.min(l.cap) as u64 * RECORD_BYTES)),
                    None => {}
                }
            }
            l.base = base as u32;
            l.cap = cap;
            base += cap as u64;
        }
        let need = base.max(PAGE as u64);
        if need > self.max_records as u64 {
            return Err(GpuError::Unsupported("Scene: more records than this device's bindings and one dispatch reach"));
        }
        let old = core::mem::replace(&mut self.pool, storage(&self.dev, need * RECORD_BYTES, BufferUsage::NONE)?);
        self.pool_cap = need as u32;
        self.upload_info.rebuild = true;
        self.upload_info.moved = moves.iter().map(|m| m.2 / RECORD_BYTES).sum();
        self.up.rebuild = Some((old, moves));
        self.groups = None;
        for f in &mut self.frames {
            f.scatter_group = None;
        }
        self.layout_dirty = false;
        self.filled = true;
        Ok(())
    }

    /// the buffers of the visibility big enough (new ones: new bind groups)
    fn grow(&mut self) -> Result<()> {
        let dev = self.dev.clone();
        let none = BufferUsage::NONE;
        let entries = self.last_order.len() as u32;
        if entries > self.max_entries {
            return Err(GpuError::Unsupported("Scene: more pages in the visible layers than one dispatch reaches"));
        }
        if entries > self.entries_cap {
            self.entries_cap = entries.max(self.entries_cap * 2);
            let n = self.entries_cap as u64;
            self.order_buf = storage(&dev, n * 4, none)?;
            self.masks = storage(&dev, n * 32, none)?;
            self.counts = storage(&dev, n * 4, none)?;
            self.offsets = storage(&dev, n * 4, none)?;
            self.groups = None;
        }
        let layers = self.last_layers.len() as u32;
        if layers > self.layers_cap {
            self.layers_cap = layers.max(self.layers_cap * 2);
            let n = self.layers_cap as u64;
            self.layers_buf = storage(&dev, n * 16, none)?;
            self.lists_buf = storage(&dev, n * 16, none)?;
            self.draws_buf = storage(&dev, n * DRAW_BYTES, BufferUsage::INDIRECT)?;
            self.groups = None;
        }
        if self.pool_cap > self.visible_cap {
            self.visible_cap = self.pool_cap;
            self.visible = storage(&dev, self.visible_cap as u64 * 4, none)?;
            self.groups = None;
        }
        let epx = if self.layers.iter().any(|l| l.live && l.fills > 0) { self.pool_cap } else { 1 };
        if epx > self.epx_cap {
            self.epx_cap = epx;
            self.epx = storage(&dev, epx as u64 * 16, none)?;
            self.groups = None;
        }
        if self.hatched.len() as u32 > self.hatch_cap {
            self.hatch_cap = (self.hatched.len() as u32).max(self.hatch_cap * 2);
            self.hatch_buf = storage(&dev, self.hatch_cap as u64 * HATCH_BYTES, none)?;
            self.hatch_for = None;
            self.groups = None;
        }
        if self.styles.len() as u32 > self.styles_cap {
            self.styles_cap = (self.styles.len() as u32).max(self.styles_cap * 2);
            self.styles_buf = storage(&dev, self.styles_cap as u64 * STYLE_BYTES, none)?;
            self.styles_dirty = true;
            self.groups = None;
        }
        Ok(())
    }

    fn make_groups(&mut self, r: &Renderer) -> Result<()> {
        if self.groups.is_some() {
            return Ok(());
        }
        let dev = &self.dev;
        let g = |layout: &BindGroupLayout, entries: &[BindEntry]| dev.create_bind_group(&BindGroupDesc { layout, entries });
        self.groups = Some(Groups {
            count: g(&r.count_layout, &[entry(0, &self.view_ubo), entry(1, &self.pool), entry(2, &self.styles_buf), entry(3, &self.order_buf), entry(4, &self.masks),
                                         entry(5, &self.counts), entry(11, &self.epx)])?,
            scan: g(&r.scan_layout, &[entry(5, &self.counts), entry(6, &self.layers_buf), entry(7, &self.offsets), entry(8, &self.lists_buf), entry(9, &self.draws_buf)])?,
            write: g(&r.write_layout, &[entry(3, &self.order_buf), entry(4, &self.masks), entry(7, &self.offsets), entry(10, &self.visible)])?,
            draw: g(&r.draw_layout, &[entry(0, &self.view_ubo), entry(1, &self.pool), entry(2, &self.styles_buf), entry(3, &self.visible), entry(4, &self.lists_buf), entry(5, &self.hatch_buf), entry(6, &self.epx)])?,
        });
        for f in &mut self.frames {
            f.scatter_group = None;
        }
        Ok(())
    }

    /// the visible layers' pages in draw order and their table
    fn tables(&mut self) -> (Vec<u32>, Vec<[u32; 4]>) {
        let mut vis: Vec<usize> = (0..self.layers.len()).filter(|&i| self.layers[i].live && self.layers[i].visible).collect();
        vis.sort_by_key(|&i| (self.layers[i].order, self.layers[i].seq));
        let mut order = Vec::new();
        let mut table = Vec::with_capacity(vis.len());
        for &i in &vis {
            let l = &self.layers[i];
            let first = order.len() as u32;
            let p0 = l.base / PAGE;
            order.extend(p0..p0 + l.used.div_ceil(PAGE));
            table.push([first, order.len() as u32 - first, 0, 0]);
        }
        self.drawn = vis.iter().map(|&i| i as u32).collect();
        self.full = vis.iter().map(|&i| if self.layers[i].fills > 0 { CLASS_FILL } else if self.layers[i].full > 0 { CLASS_FULL } else { CLASS_PLAIN }).collect();
        (order, table)
    }

    /// the changed records and free places, by their place in the pool
    fn changed_records(&mut self) -> Vec<(u32, [u32; RECORD_WORDS])> {
        let mut rows: Vec<(u32, [u32; RECORD_WORDS])> = Vec::new();
        for (li, era, at, n) in core::mem::take(&mut self.freed) {
            let l = &self.layers[li as usize];
            // (a layer gone or laid out anew has no such places any more)
            if !l.live || l.era != era || l.repacked {
                continue;
            }
            for k in 0..n {
                rows.push((l.base + at + k, [0; RECORD_WORDS]));
            }
        }
        let mut words = Vec::new();
        for slot in core::mem::take(&mut self.dirty) {
            let e = &mut self.ents[slot as usize];
            e.dirty = false;
            if !e.live {
                continue;
            }
            let e = &self.ents[slot as usize];
            words.clear();
            records(&e.geom, e.style, &mut words);
            words.resize(e.alloc as usize, [0; RECORD_WORDS]);
            let at = self.layers[e.layer as usize].base + e.at;
            for (k, w) in words.iter().enumerate() {
                rows.push((at + k as u32, *w));
            }
        }
        // (the later of two rows for one place wins: a removed entity's place taken in the same frame by none other,
        // since free places are not handed out again; sorted stably)
        rows.sort_by_key(|r| r.0);
        rows.dedup_by(|b, a| {
            if a.0 == b.0 {
                *a = *b;
                true
            } else {
                false
            }
        });
        rows
    }

    /// everything goes up again (a prepare that never reached a submission, the first frame)
    fn resend_all(&mut self) {
        self.dirty.clear();
        self.freed.clear();
        for i in 0..self.ents.len() {
            self.ents[i].dirty = self.ents[i].live;
            if self.ents[i].live {
                self.dirty.push(i as u32);
            }
        }
        self.styles_dirty = true;
        self.hatch_for = None;
        self.last_order.clear();
        self.last_layers.clear();
        self.last_layers.push([u32::MAX; 4]);
    }

    /// curves and fills whose flattening no longer holds for the camera (another zoom band, or the view left a window)
    /// are flattened again; those whose points changed go up (docs/RENDER2D.md "Kurven")
    fn reflatten(&mut self) {
        self.reflat = 0;
        if self.curves.is_empty() {
            return;
        }
        if self.curves_dirty {
            // (a slot can come twice: removed, then taken by another curve, the later one counts; dead ones and others go)
            self.curves.sort_by_key(|c| c.0);
            self.curves.dedup_by(|b, a| {
                if a.0 == b.0 {
                    *a = *b;
                    true
                } else {
                    false
                }
            });
            let ents = &self.ents;
            self.curves.retain(|c| ents[c.0 as usize].live && matches!(ents[c.0 as usize].geom, Geom::Curve(_) | Geom::Fill(_)));
            // (the styles as they are now)
            for c in &mut self.curves {
                if c.3 != u32::MAX {
                    c.3 = ents[c.0 as usize].style;
                }
            }
            self.curves_dirty = false;
        }
        let ctx = FlatCtx::new(&self.camera, self.size);
        for n in 0..self.curves.len() {
            // (what decides lies here, beside the list: the geometry is looked at only when it must be flattened again)
            let (slot, st0, bounds, style) = self.curves[n];
            let i = slot as usize;
            let reach = if style == u32::MAX { FILL_REACH } else { stroke_reach(&self.styles[style as usize], ctx.scale) };
            if !ctx.stale(&st0, reach) || !ctx.near(&bounds, reach) {
                continue;
            }
            let e = &self.ents[i];
            if !e.live || !matches!(e.geom, Geom::Curve(_) | Geom::Fill(_)) || (style != u32::MAX && e.style != style) {
                // (gone, or another style since: the list is made again)
                self.curves_dirty = true;
                continue;
            }
            let fresh = match &self.ents[i].geom {
                Geom::Curve(g) => {
                    let (flat, fs) = ctx.flatten(&g.c, reach);
                    self.curves[n].1 = fs;
                    if flat == g.flat {
                        // (the same points: nothing goes up)
                        if let Geom::Curve(g) = &mut self.ents[i].geom {
                            g.st = fs;
                        }
                        continue;
                    }
                    Geom::Curve(Box::new(CurveG { c: g.c.clone(), flat, st: fs, bounds: g.bounds }))
                }
                Geom::Fill(g) => {
                    let (prep, fs) = flatten_fill(&g.rings, &ctx);
                    self.curves[n].1 = fs;
                    if prep == g.prep {
                        if let Geom::Fill(g) = &mut self.ents[i].geom {
                            g.st = fs;
                        }
                        continue;
                    }
                    Geom::Fill(Box::new(FillG { rings: g.rings.clone(), prep, st: fs, bounds: g.bounds }))
                }
                _ => continue,
            };
            self.reflat += 1;
            self.regeom(i, fresh);
        }
    }

    /// curves and fills the last prepare flattened again (their zoom band changed, or the view left their window)
    pub fn reflattened(&self) -> u32 {
        self.reflat
    }

    /// the frame's data for Renderer::record: waits for this slot's last frame, sends what changed.  size: the
    /// target's in pixels
    pub fn prepare(&mut self, r: &mut Renderer, size: (u32, u32)) -> Result<()> {
        if !Arc::ptr_eq(&self.tag, &r.tag) {
            return Err(GpuError::Invalid("Scene::prepare: another Renderer than its own"));
        }
        if size.0 == 0 || size.1 == 0 {
            return Err(GpuError::Invalid("Scene::prepare: a target of at least 1 x 1"));
        }
        self.wait_slot()?;
        let full = self.sent || !self.filled;
        if full {
            self.resend_all();
        }
        self.sent = true;
        self.prepared = false;
        self.stale = false;
        let slot = (self.frame % self.frames.len() as u64) as usize;
        self.slot = slot;
        self.size = size;
        self.up = Uploads::default();
        self.upload_info = UploadInfo::default();
        self.reflatten();
        self.layout(full)?;
        let (order, table) = self.tables();
        if table.len() as u32 > LAYERS_MAX {
            return Err(GpuError::Unsupported("Scene: more than 65535 visible layers"));
        }
        let tables_new = order != self.last_order || table != self.last_layers;
        self.last_order = order;
        self.last_layers = table;
        self.grow()?;
        // (new buffers hold nothing yet: the tables go up again)
        let tables_new = tables_new || self.groups.is_none();
        self.make_groups(r)?;
        self.entries = self.last_order.len() as u32;
        self.vis_layers = self.last_layers.len() as u32;
        let dev = self.dev.clone();
        let mut bytes = VIEW_BYTES;
        let rescale = self.px_dash && self.camera.scale != self.styles_scale;
        if (self.styles_dirty || rescale) && !self.styles.is_empty() {
            let scale = self.camera.scale;
            let words: Vec<[u32; STYLE_WORDS]> = self.styles.iter().enumerate().map(|(k, s)| s.words(scale, self.hatch_of[k])).collect();
            let size = words.len() as u64 * STYLE_BYTES;
            let f = &mut self.frames[slot];
            if f.styles.as_ref().is_none_or(|b| b.size() < size) {
                f.styles = Some(upload(&dev, size.next_power_of_two())?);
            }
            let Some(up) = &f.styles else { return Err(GpuError::Invalid("Scene::prepare")) };
            up.write(0, &words)?;
            self.up.styles = Some((up.clone(), size));
            self.styles_scale = scale;
            bytes += size;
        }
        self.styles_dirty = false;
        // the hatch table: the lines' places at the camera (in f64 here, so that the shader's numbers stay small), whenever
        // the camera, the target or a hatched style changed
        if !self.hatched.is_empty() && self.hatch_for != Some((self.camera, size)) {
            let mut words: Vec<[f32; HATCH_WORDS]> = Vec::with_capacity(self.hatched.len());
            for &k in &self.hatched {
                words.push(self.styles[k as usize].hatch_words(&self.camera, size));
            }
            let hb = words.len() as u64 * HATCH_BYTES;
            let f = &mut self.frames[slot];
            if f.hatch.as_ref().is_none_or(|b| b.size() < hb) {
                f.hatch = Some(upload(&dev, hb.next_power_of_two())?);
            }
            let Some(up) = &f.hatch else { return Err(GpuError::Invalid("Scene::prepare")) };
            up.write(0, &words)?;
            self.up.hatch = Some((up.clone(), hb));
            self.hatch_for = Some((self.camera, size));
            bytes += hb;
        }
        if tables_new || self.upload_info.rebuild {
            let (ob, lb) = (self.last_order.len() as u64 * 4, self.last_layers.len() as u64 * 16);
            let up = upload(&dev, ob + lb)?;
            up.write(0, &self.last_order)?;
            up.write(ob, &self.last_layers)?;
            self.up.tables = Some((up, ob, lb));
            self.upload_info.tables = true;
            bytes += ob + lb;
        }
        // the records: runs of neighbouring places, a copy each when they are few, else the scatter shader
        let rows = self.changed_records();
        let mut runs = 0u32;
        for (k, row) in rows.iter().enumerate() {
            if k == 0 || rows[k - 1].0 + 1 != row.0 {
                runs += 1;
            }
        }
        self.upload_info.records = rows.len() as u32;
        self.upload_info.runs = runs;
        if !rows.is_empty() {
            let f = &mut self.frames[slot];
            if runs <= 256 {
                let size = rows.len() as u64 * RECORD_BYTES;
                if f.runs.as_ref().is_none_or(|b| b.size() < size) {
                    f.runs = Some(upload(&dev, size.next_power_of_two().max(1 << 14))?);
                }
                let Some(buf) = &f.runs else { return Err(GpuError::Invalid("Scene::prepare")) };
                let words: Vec<[u32; RECORD_WORDS]> = rows.iter().map(|r| r.1).collect();
                buf.write(0, &words)?;
                let mut list: Vec<(u64, u64, u64)> = Vec::with_capacity(runs as usize);
                for (k, row) in rows.iter().enumerate() {
                    match list.last_mut() {
                        Some(m) if k > 0 && rows[k - 1].0 + 1 == row.0 => m.2 += RECORD_BYTES,
                        _ => list.push((k as u64 * RECORD_BYTES, row.0 as u64 * RECORD_BYTES, RECORD_BYTES)),
                    }
                }
                self.up.runs = Some((buf.clone(), list));
                bytes += size;
            } else {
                let size = rows.len() as u64 * CHANGE_WORDS as u64 * 4;
                if f.changes.as_ref().is_none_or(|b| b.size() < size) {
                    f.changes = Some(dev.create_buffer(&BufferDesc { size: size.next_power_of_two().max(1 << 14), usage: BufferUsage::STORAGE, memory: MemoryKind::Upload })?);
                    f.scatter_group = None;
                }
                let Some(buf) = &f.changes else { return Err(GpuError::Invalid("Scene::prepare")) };
                let words: Vec<[u32; CHANGE_WORDS]> = rows.iter().map(|r| {
                    let mut w = [0u32; CHANGE_WORDS];
                    w[0] = r.0;
                    w[4..].copy_from_slice(&r.1);
                    w
                }).collect();
                buf.write(0, &words)?;
                if f.scatter_group.is_none() {
                    f.scatter_group = Some(dev.create_bind_group(&BindGroupDesc { layout: &r.scatter_layout, entries: &[entry(0, buf), entry(1, &self.pool)] })?);
                }
                self.up.scatter = rows.len() as u32;
                self.upload_info.scatter = true;
                bytes += size;
            }
        }
        if self.stats_on {
            if self.ts.is_none() && dev.features().timestamps {
                self.ts = Some(dev.create_timestamps(STAMPS * self.frames.len() as u32)?);
            }
            let size = self.layers_cap as u64 * 16;
            let f = &mut self.frames[slot];
            if f.readback.as_ref().is_none_or(|b| b.size() < size) {
                f.readback = Some(dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?);
            }
        }
        self.frames[slot].stage.write(0, &self.camera.words(size, !self.culling))?;
        self.upload_info.bytes = bytes;
        self.prepared = true;
        Ok(())
    }

    /// the frame went to the GPU: Queue::submit's or Swapchain::present's answer
    pub fn submitted(&mut self, id: SubmitId) {
        let slot = self.slot;
        self.frames[slot].last = Some((id, self.frame));
        self.frame += 1;
        self.prepared = false;
        self.sent = false;
        self.waited = false;
        self.up = Uploads::default();
    }
}

/// the pipeline an entity's layer needs: a fill the third, an arc or a dashed style the second (lib.rs)
fn class_of(g: &Geom, st: &Style) -> u8 {
    if matches!(g, Geom::Fill(..)) {
        CLASS_FILL
    } else if matches!(g, Geom::Arc(..)) || st.dashes > 0 {
        CLASS_FULL
    } else {
        CLASS_PLAIN
    }
}

/// a fill's rings flattened for a camera, triangulated
fn flatten_fill(rings: &[Curve], ctx: &FlatCtx) -> (fill::Prepared, FlatState) {
    let mut pts = Vec::with_capacity(rings.len());
    let mut st = FlatState { band: curve::band_of(ctx.scale), coarse: 0, window: None };
    for c in rings {
        let (f, s) = ctx.flatten(c, FILL_REACH);
        if s.window.is_some() {
            st.window = s.window;
        }
        st.coarse = st.coarse.max(s.coarse);
        pts.push(f.pts);
    }
    let mut p = fill::prepare(&pts);
    // (the edges near each triangle and edge: within a pixel at the coarse end of the band)
    p.near(curve::pow2(st.band + 1) * curve::HYST);
    (p, st)
}

/// the pieces of an arc of this sweep (at most ARC_STEP each; a full circle 16)
fn arc_pieces(sweep: f64) -> u32 {
    let n = (sweep.abs() / ARC_STEP - 1e-9).max(0.0) as u32 + 1;
    n.min(16)
}

/// the records an entity's geometry has: a segment between neighbouring points (repeated points count once), a closed
/// polyline's last back to its first; a single point: one (a dot, drawn by round and square caps); an arc: a head
/// (holding its start, never drawn) and its pieces
fn record_count(g: &Geom) -> u32 {
    match g {
        Geom::Line(_) => 1,
        Geom::Poly(p, closed) => {
            let mut n = distinct(p);
            if *closed && n >= 2 && p[0] == p[p.len() - 1] {
                n -= 1;
            }
            if n <= 1 { 1 } else if *closed { n as u32 } else { n as u32 - 1 }
        }
        Geom::Arc(_, _, _, sweep) => arc_pieces(*sweep) + 1,
        Geom::Curve(g) => {
            let n = g.flat.pts.len();
            if n <= 1 { 1 } else if g.flat.closed { n as u32 } else { n as u32 - 1 }
        }
        Geom::Fill(g) => {
            let e: usize = g.prep.rings.iter().map(|r| r.len()).sum();
            (e + g.prep.tris.len()).max(1) as u32
        }
    }
}

/// points that differ from the one before (a closed polyline's last equal to its first counts once)
fn distinct(p: &[[f64; 2]]) -> usize {
    let mut n = 0;
    for k in 0..p.len() {
        if k == 0 || p[k] != p[k - 1] {
            n += 1;
        }
    }
    n
}

// common.wgsl's Record: the kinds and the flags of its last word
const K_SEG: u32 = 0;
const K_ARC: u32 = 1;
const K_HEAD: u32 = 2;
/// a fill's boundary edge (a ring is a closed polyline of them) and triangle (its corners: the edges that start there)
const K_EDGE: u32 = 3;
const K_TRI: u32 = 4;
/// a segment: its previous point is the polyline's first (a cap there); its next point is the last
const F_CAP_PREV: u32 = 1;
const F_CAP_NEXT: u32 = 2;
/// a segment joined at its start, at its end
const F_PREV: u32 = 4;
const F_NEXT: u32 = 8;
/// the first record of its entity: the length along it starts at 0
const F_FIRST: u32 = 16;
/// a closed polyline's first record (its previous one lies `aux` records on) and its last (the next one lies `aux` back)
const F_WRAP_PREV: u32 = 32;
const F_WRAP_NEXT: u32 = 64;
/// an arc piece: its start is the arc's (a cap), its end is the arc's
const F_CAP_A: u32 = 1;
const F_CAP_B: u32 = 2;
/// an arc counter-clockwise (y up)
const F_CCW: u32 = 4;
/// a circle's first piece (its start is where the dash pattern starts again; the last piece lies `aux` on) and its last
const F_RESTART_A: u32 = 32;
const F_RESTART_B: u32 = 64;

/// common.wgsl's Records of an entity
fn records(g: &Geom, style: u32, out: &mut Vec<[u32; RECORD_WORDS]>) {
    let b = |v: f32| v.to_bits();
    let point = |p: [f64; 2]| -> [u32; 4] {
        let (x, lx) = split(p[0]);
        let (y, ly) = split(p[1]);
        [b(x), b(y), b(lx), b(ly)]
    };
    // (the length along the entity at a record's end: three f32 whose sum is exact)
    let rec = |a: [u32; 4], e: [u32; 4], v: [u32; 2], l: f64, aux: u32, style: u32, flags: u32| -> [u32; RECORD_WORDS] {
        let (l0, l1, l2) = split3(l);
        [a[0], a[1], a[2], a[3], e[0], e[1], e[2], e[3], v[0], v[1], b(l0), b(l1), b(l2), aux, style, flags]
    };
    let own = |a: [f64; 2], e: [f64; 2]| [b((e[0] - a[0]) as f32), b((e[1] - a[1]) as f32)];
    if let Geom::Curve(cg) = g {
        // a polyline of the flattened points, the length at each the curve's (Flat::lens)
        let f = &cg.flat;
        let n = f.pts.len();
        if n <= 1 {
            let p = f.pts.first().copied().unwrap_or(cg.c.origin);
            out.push(rec(point(p), point(p), [0; 2], 0.0, 0, style + 1, F_FIRST | K_SEG << 8));
            return;
        }
        let closed = f.closed;
        let segs = if closed { n } else { n - 1 };
        for k in 0..segs {
            let (a, e) = (f.pts[k], f.pts[(k + 1) % n]);
            let len = if k + 1 < n { f.lens[k + 1] } else { f.total };
            let mut fl = K_SEG << 8;
            if !closed {
                fl |= (k == 1) as u32 * F_CAP_PREV | (segs >= 2 && k + 2 == segs) as u32 * F_CAP_NEXT;
            }
            fl |= (closed || k > 0) as u32 * F_PREV | (closed || k + 1 < segs) as u32 * F_NEXT | (k == 0) as u32 * F_FIRST;
            let mut aux = 0;
            if closed && k == 0 {
                fl |= F_WRAP_PREV;
                aux = segs as u32 - 1;
            }
            if closed && k + 1 == segs {
                fl |= F_WRAP_NEXT;
                aux = segs as u32 - 1;
            }
            out.push(rec(point(a), point(e), own(a, e), len, aux, style + 1, fl));
        }
        return;
    }
    if let Geom::Fill(fg) = g {
        // the rings' edges as closed polylines, then the triangles (fill.wgsl's layouts): an edge holds how far back the
        // fill's first record lies (v.z) and up to 4 edges near it (u16 from there, words 11, 12; their number from bit 16
        // of w.w); a triangle that distance (a.x), its corners' edges (a.yzw) and up to 20 edges near it (words 4 to 13)
        let p = &fg.prep;
        let at = |q: [f64; 2]| [p.origin[0] + q[0], p.origin[1] + q[1]];
        let first = out.len();
        let mut base = Vec::with_capacity(p.rings.len());
        let put16 = |w: &mut [u32; RECORD_WORDS], j0: usize, list: &[u32]| -> u32 {
            let mut n = 0u32;
            for &e in list {
                if e > 0xffff {
                    continue;
                }
                let (j, sh) = (j0 + n as usize / 2, 16 * (n % 2));
                w[j] |= e << sh;
                n += 1;
            }
            n
        };
        let mut gi = 0usize;
        for r in &p.rings {
            base.push((out.len() - first) as u32);
            let n = r.len();
            for k in 0..n {
                let (a, e) = (at(r[k]), at(r[(k + 1) % n]));
                let mut fl = K_EDGE << 8 | F_PREV | F_NEXT;
                let mut aux = 0;
                if k == 0 {
                    fl |= F_WRAP_PREV;
                    aux = n as u32 - 1;
                }
                if k + 1 == n {
                    fl |= F_WRAP_NEXT;
                    aux = n as u32 - 1;
                }
                let mut w = rec(point(a), point(e), own(a, e), 0.0, aux, style + 1, fl);
                w[10] = (out.len() - first) as u32;
                w[11] = 0;
                w[12] = 0;
                let near = p.edge_near.get(gi).map_or(&[][..], |v| &v[..]);
                let cnt = put16(&mut w, 11, near);
                w[15] |= cnt << 16;
                out.push(w);
                gi += 1;
            }
        }
        for (ti, t) in p.tris.iter().enumerate() {
            let me = (out.len() - first) as u32;
            let at = |(r, k): (u32, u32)| base[r as usize] + k;
            let mut w = [0u32; RECORD_WORDS];
            w[0] = me;
            w[1] = at(t[0]);
            w[2] = at(t[1]);
            w[3] = at(t[2]);
            let near = p.tri_near.get(ti).map_or(&[][..], |v| &v[..]);
            let cnt = put16(&mut w, 4, near);
            w[14] = style + 1;
            w[15] = K_TRI << 8 | cnt << 16;
            out.push(w);
        }
        if out.len() == first {
            out.push([0; RECORD_WORDS]);
        }
        return;
    }
    if let Geom::Arc(c, r, a0, sweep) = *g {
        let n = arc_pieces(sweep);
        let full = sweep.abs() >= core::f64::consts::TAU;
        let at = |t: f64| {
            let (s, co) = sincos(t);
            point([c[0] + r * co, c[1] + r * s])
        };
        let start = at(a0);
        // the head: the arc's start where the pieces have their ends, length 0, no style (never drawn)
        out.push(rec([0; 4], start, [0; 2], 0.0, 0, 0, K_HEAD << 8));
        let (rh, rl) = split(r);
        let ccw = if sweep > 0.0 { F_CCW } else { 0 };
        let aux = if full { n - 1 } else { 0 };
        for k in 0..n {
            // (a circle's last piece ends in its start's very bits: both pieces judge the pixels there alike)
            let e = if full && k + 1 == n { start } else { at(a0 + sweep * (k + 1) as f64 / n as f64) };
            let mut f = ccw | K_ARC << 8;
            if full {
                f |= (k == 0) as u32 * F_RESTART_A | (k + 1 == n) as u32 * F_RESTART_B;
            } else {
                f |= (k == 0) as u32 * F_CAP_A | (k + 1 == n) as u32 * F_CAP_B;
            }
            out.push(rec(point(c), e, [b(rh), b(rl)], r * sweep.abs() * (k + 1) as f64 / n as f64, aux, style + 1, f));
        }
        return;
    }
    let mut pts: Vec<[f64; 2]> = Vec::new();
    let closed = match g {
        Geom::Line(p) => {
            pts.extend_from_slice(p);
            false
        }
        Geom::Poly(p, c) => {
            pts.extend_from_slice(p);
            *c
        }
        _ => false,
    };
    pts.dedup();
    if closed && pts.len() >= 2 && pts[0] == pts[pts.len() - 1] {
        pts.pop();
    }
    let n = pts.len();
    // (own(): the same differences in f64 rounded once: a segment's own vector is exactly the negated previous vector of the
    // next one and the next vector of the one before (the vertex shader reads its neighbours' own vectors), so both compute
    // their join's bisector bit for bit)
    if n == 1 {
        out.push(rec(point(pts[0]), point(pts[0]), [0; 2], 0.0, 0, style + 1, F_FIRST | K_SEG << 8));
        return;
    }
    let segs = if closed { n } else { n - 1 };
    let mut len = 0.0f64;
    for k in 0..segs {
        let a = pts[k];
        let e = pts[(k + 1) % n];
        let (dx, dy) = (e[0] - a[0], e[1] - a[1]);
        len += sqrt(dx * dx + dy * dy);
        let mut f = K_SEG << 8;
        if !closed {
            // the previous point is the polyline's first (a cap there), the next point its last
            f |= (k == 1) as u32 * F_CAP_PREV | (segs >= 2 && k + 2 == segs) as u32 * F_CAP_NEXT;
        }
        f |= (closed || k > 0) as u32 * F_PREV | (closed || k + 1 < segs) as u32 * F_NEXT | (k == 0) as u32 * F_FIRST;
        let mut aux = 0;
        if closed && k == 0 {
            f |= F_WRAP_PREV;
            aux = segs as u32 - 1;
        }
        if closed && k + 1 == segs {
            f |= F_WRAP_NEXT;
            aux = segs as u32 - 1;
        }
        out.push(rec(point(a), point(e), own(a, e), len, aux, style + 1, f));
    }
}
