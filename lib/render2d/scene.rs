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
// against the frame before): the view; the styles when they changed; the visible layers' entries and table when they
// changed; the changed records, in runs (few runs: a copy each) or through the scatter shader (many); the changed
// instances.  A prepare that never reached a submission: the next sends everything again.
//
// Blocks (G6b2, docs/RENDER2D.md "Blöcke"): a block's entities are Ents of no layer, in its list; its records lie once
// in the pool, in variants (one per zoom band its instances need where it holds curves or fills, one with its arcs as
// curves for uneven scales) whose regions follow the layers'.  An insert on a layer holds one record of zeros there (its
// place in the order).  The instances (each copy of an insert on a layer, each copy of a nested insert per instance of
// its block, depth first) are laid out anew when inserts or blocks change, else only moved inserts' transforms go up;
// the visibility's entries name a page, an instance, a range of the page (several small instances in one entry).

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;

use alloc::boxed::Box;

use crate::block::{self, Fam, Xf};
use crate::curve::{self, Curve, Flat};
use crate::fill;
use crate::math::{sincos, split, sqrt};
use crate::{Renderer, Tag, CHANGE_WORDS, DRAW_BYTES, ENTRY_BYTES, HATCH_BYTES, HATCH_WORDS, INST_BYTES, INST_WORDS, LAYERS_MAX, PAGE, RECORD_BYTES, RECORD_WORDS, SPACE_BYTES, STAMPS, STYLE_BYTES,
            STYLE_WORDS, VIEW_BYTES};

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
    /// rotation, the size; the rotation's map to pixels (cos, -sin, sin, cos: to_px with it is bit for bit what it was
    /// with the rotation alone, G6b2)
    fn words(&self, size: (u32, u32), all: bool) -> [f32; 20] {
        let (hx, lx) = split(self.center[0]);
        let (hy, ly) = split(self.center[1]);
        let rx = ((self.center[0] - hx as f64) - lx as f64) as f32;
        let ry = ((self.center[1] - hy as f64) - ly as f64) as f32;
        let (s, c) = sincos(self.rotation);
        let (s, c) = (s as f32, c as f32);
        [hx, hy, lx, ly, rx, ry, (1.0 / self.scale) as f32, self.scale as f32, c, s, size.0 as f32, size.1 as f32, all as u32 as f32, 0.0, 0.0, 0.0, c, -s, s, c]
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
    /// AutoCAD's transparency of what the style draws: its layer's (the default), its block insert's, or its own; the
    /// colour's alpha times 1 - percent / 100 (G6b2)
    pub transparency: Transparency,
    /// inside a block, what an entity of this style takes from its insert's style instead of its own (BY_BLOCK_COLOR,
    /// BY_BLOCK_WIDTH, BY_BLOCK_DASH; DXF's ByBlock); outside a block the style's own (G6b2)
    pub by_block: u32,
}

/// AutoCAD's transparency of an entity (Style::transparency): 0 to TRANSPARENCY_MAX percent, multiplied into the colour's
/// alpha (alpha (100 - percent) / 100, rounded); not group compositing: overlapping entities blend each (docs/RENDER2D.md
/// "Transparenz")
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transparency {
    /// its layer's (Layer::transparency)
    ByLayer,
    /// inside a block its insert's style's (that one's ByLayer: the insert's layer); outside a block none
    ByBlock,
    /// its own, in percent
    Percent(u32),
}

/// the most transparency AutoCAD has (percent)
pub const TRANSPARENCY_MAX: u32 = 90;
/// Style::by_block: inside a block the colour, the width (and its unit), the dash pattern (entries, unit, phase) come from
/// the insert's style
pub const BY_BLOCK_COLOR: u32 = 1;
pub const BY_BLOCK_WIDTH: u32 = 2;
pub const BY_BLOCK_DASH: u32 = 4;
/// (block.wgsl Style.r1.x: a transparency by layer, by block)
const T_BY_LAYER: u32 = 0xFFFF_FFFF;
const T_BY_BLOCK: u32 = 0xFFFF_FFFE;

impl Style {
    /// a hairline of a colour: width 0, butt caps, miter joins with the limit 4, solid
    pub const fn new(color: u32) -> Style {
        Style { color, width: 0.0, unit: Unit::Px, cap: Cap::Butt, join: Join::Miter, miter_limit: 4.0, dash: [0.0; DASH_MAX], dashes: 0, dash_unit: Unit::Px,
                dash_phase: 0.0, hatch: [HatchLine::new(0.0, 1.0); HATCH_MAX], hatches: 0, hatch_width: 0.0, transparency: Transparency::ByLayer, by_block: 0 }
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
        if matches!(self.transparency, Transparency::Percent(p) if p > TRANSPARENCY_MAX) || self.by_block & !(BY_BLOCK_COLOR | BY_BLOCK_WIDTH | BY_BLOCK_DASH) != 0 {
            return Err(GpuError::Invalid("Style: a transparency of at most TRANSPARENCY_MAX percent, by_block of BY_BLOCK_* only"));
        }
        Ok(())
    }

    /// this style where an entity of `own` takes what its by_block says from it: what the instances of a nested insert of
    /// style `own` give their block's entities by block, when `self` is what their parent gives its (G6b2)
    pub(crate) fn by_block_from(&self, own: &Style) -> Style {
        let mut s = *own;
        if own.by_block & BY_BLOCK_COLOR != 0 {
            s.color = self.color;
        }
        if own.by_block & BY_BLOCK_WIDTH != 0 {
            s.width = self.width;
            s.unit = self.unit;
        }
        if own.by_block & BY_BLOCK_DASH != 0 {
            s.dash = self.dash;
            s.dashes = self.dashes;
            s.dash_unit = self.dash_unit;
            s.dash_phase = self.dash_phase;
        }
        if own.transparency == Transparency::ByBlock {
            s.transparency = self.transparency;
        }
        s
    }

    /// what an entity of this style takes from somewhere else inside a block
    pub(crate) fn takes(&self) -> bool {
        self.by_block != 0 || self.transparency == Transparency::ByBlock
    }

    /// a dash pattern in pixels: its words change with the scale
    fn px_dashes(&self) -> bool {
        self.dashes > 0 && self.dash_unit == Unit::Px
    }

    /// fill.wgsl's Hatch of this style for a camera and a target (scene.rs computes the lines' places at the camera in f64,
    /// the shader goes on from there in pixels: docs/RENDER2D.md "Schraffuren")
    fn hatch_words(&self, cam: &Camera, size: (u32, u32)) -> [f32; HATCH_WORDS] {
        let (f, n) = block::fams(self);
        self.hatch_words_of(&f[..n], cam, size)
    }

    /// the same for families of lines (Style::hatch_words: the style's own; an insert's: through its transform, G6b2)
    pub(crate) fn hatch_words_of(&self, fams: &[Fam], cam: &Camera, size: (u32, u32)) -> [f32; HATCH_WORDS] {
        let mut w = [0.0f32; HATCH_WORDS];
        let n = fams.len().min(HATCH_MAX);
        w[0] = n as f32;
        w[1] = (0.5 * self.hatch_width).max(0.5);
        let (sr, cr) = sincos(cam.rotation);
        // a direction of the drawing in pixels: turned, y down
        let px = |v: [f64; 2]| [cr * v[0] - sr * v[1], -(sr * v[0] + cr * v[1])];
        let _ = size;
        let hw = w[1] as f64;
        for (f, h) in fams[..n].iter().enumerate() {
            let dir = h.dir;
            let nrm = [-dir[1], dir[0]];
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
        // (G6b2, block.wgsl: the transparency, what is taken by block)
        w[28] = match self.transparency {
            Transparency::ByLayer => T_BY_LAYER,
            Transparency::ByBlock => T_BY_BLOCK,
            Transparency::Percent(p) => p,
        };
        w[29] = self.by_block;
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
    /// AutoCAD's transparency of the entities whose style says Transparency::ByLayer (percent, 0 to TRANSPARENCY_MAX; G6b2):
    /// changing it sends no records
    pub transparency: u32,
}

impl Layer {
    pub const fn new(order: i32) -> Layer {
        Layer { visible: true, order, transparency: 0 }
    }
}

/// a block of a Scene: entities defined once (Scene::insert_in, in the block's units) and inserted many times
/// (Shape::Insert); dead after remove_block
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockId {
    pub slot: u32,
    era: u32,
}

/// a block's settings.  Made with `new` and changed field by field (it may grow)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    /// the point of the block that lands on an insert's `at` (DXF's base point)
    pub base: [f64; 2],
}

impl Block {
    pub const fn new() -> Block {
        Block { base: [0.0, 0.0] }
    }
}

impl Default for Block {
    fn default() -> Block {
        Block::new()
    }
}

/// the most levels of blocks inside blocks below an insert on a layer (nested inserts; a cycle is refused anyway)
pub const BLOCK_DEPTH: u32 = 16;

/// a block placed by an entity (Shape::Insert): its base point to `at`, scaled (each axis; a negative scale mirrors, unequal
/// magnitudes scale unevenly: circles become ellipses), turned by `rotation` (radians, counter-clockwise); an array of
/// `columns` by `rows` copies `spacing` apart along the insert's turned x and y (drawing units, not scaled: DXF's MINSERT).
/// Made with `new` and changed field by field (it may grow)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Insert {
    pub block: BlockId,
    pub at: [f64; 2],
    pub rotation: f64,
    pub scale: [f64; 2],
    pub columns: u32,
    pub rows: u32,
    pub spacing: [f64; 2],
}

impl Insert {
    /// the block at a point, unscaled, unturned, one copy
    pub const fn new(block: BlockId, at: [f64; 2]) -> Insert {
        Insert { block, at, rotation: 0.0, scale: [1.0, 1.0], columns: 1, rows: 1, spacing: [0.0, 0.0] }
    }

    fn check(&self) -> Result<()> {
        let fine = |v: f64| v.is_finite() && v.abs() < 1e30;
        let scale = |v: f64| fine(v) && v != 0.0 && v.abs() > 1e-30;
        if !(fine(self.at[0]) && fine(self.at[1]) && self.rotation.is_finite() && self.rotation.abs() < 1e15 && scale(self.scale[0]) && scale(self.scale[1]) && fine(self.spacing[0])
             && fine(self.spacing[1]) && self.columns >= 1 && self.rows >= 1 && self.columns as u64 * self.rows as u64 <= 1 << 24) {
            return Err(GpuError::Invalid("Insert: a finite point and rotation, scales not 0, finite spacing, at least one column and row"));
        }
        Ok(())
    }
}

/// what the blocks of a scene hold and what the last prepare sent of them (Scene::instance_info).  It may grow
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InstanceInfo {
    /// live blocks; their variants in the pool (a block with curves or fills one per zoom band its inserts need, one with
    /// arcs another for inserts that scale unevenly) and those variants' records
    pub blocks: u32,
    pub variants: u32,
    pub block_records: u32,
    /// instances (an insert's copies, nested inserts each once per instance of their block) and the entries of the
    /// visibility
    pub instances: u32,
    pub entries: u32,
    /// the instances laid out anew (an insert added or removed, a block changed, a variant came or went)
    pub rebuilt: bool,
    /// instances whose transforms went up, their bytes; block records that went up (in UploadInfo's records too)
    pub sent_instances: u32,
    pub sent_bytes: u64,
    pub sent_block_records: u32,
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
    /// a block (G6b2): on a layer in its place in the order, or inside another block; its entities in their order, each with
    /// its style or by block the insert's (docs/RENDER2D.md "Blöcke")
    Insert(Insert),
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
    /// an insert of a block (G6b2): on a layer one record of zeros (its place in the order), in a block none
    Insert(Insert),
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
    /// its layer, or (NONE) its block's entity
    layer: u32,
    /// the block it belongs to (NONE: it lies on a layer)
    block: u32,
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
    /// AutoCAD's transparency of its entities by layer (percent; the layer table carries it, G6b2)
    transparency: u32,
    /// its live inserts of blocks; their entries of the visibility as last made, and for which layout, size and instances
    inserts: u32,
    cache: Vec<[u32; 4]>,
    cache_key: (u32, u32, u64),
}

/// a block of the scene (G6b2): its entities in their order, its variants in the pool (docs/RENDER2D.md "Blöcke")
struct BlockSlot {
    live: bool,
    era: u32,
    base: [f64; 2],
    /// its entities' slots in their order
    ents: Vec<u32>,
    /// the inserts of it (on layers and in blocks), the blocks it inserts (once per insert), the blocks that insert it (once
    /// per insert), the most levels of blocks below it
    inserted: u32,
    nested: Vec<u32>,
    parents: Vec<u32>,
    height: u32,
    /// its definition changed: what it holds is looked at again, its variants are made again with the next prepare
    changed: bool,
    /// curves or fills (a variant per zoom band its instances need), arcs (one more for uneven scales)
    banded: bool,
    arcs: bool,
    /// its box in its own units (relative to its base; None: nothing drawn), how far its strokes reach beyond it (px, its
    /// units), whether one takes its width by block (the insert's style then counts too)
    bounds: Option<[f64; 4]>,
    reach: (f64, f64),
    reach_bb: bool,
    /// the pipeline its entities need (CLASS_*), more where one takes its dash pattern by block
    class: u8,
    bb_dash: bool,
    variants: Vec<Variant>,
}

/// a block's records as one of its instances needs them: its curves and fills flattened for a zoom band, its arcs as curves
/// for an uneven scale (or neither: one for every instance)
struct Variant {
    /// the band (i32::MIN: none) and arcs as curves
    band: i32,
    uneven: bool,
    /// its records (block units, relative to the block's base) and its region in the pool
    recs: Vec<[u32; RECORD_WORDS]>,
    base: u32,
    cap: u32,
    /// the record before which each of its nested inserts sits (in their order in the block)
    splits: Vec<u32>,
    /// the records of its fills' edges (first, end): their places in epx per instance
    fills: Option<(u32, u32)>,
    /// its hatched styles (the hatch rows of each instance, in this order; a fill's records name their place + 1)
    hatched: Vec<u32>,
    /// its records go up with the next prepare
    dirty: bool,
    used: bool,
}

/// an instance of a block: a copy of an insert on a layer, or a nested insert's copy in an instance of its block
struct Inst {
    block: u32,
    variant: u32,
    xf: Xf,
    /// the style its block's entities take by block (one of the scene's, or a derived one after them)
    ins: u32,
    /// its zoom band, its scale uneven (the variant it takes)
    band: i32,
    uneven: bool,
    /// a copy of an insert on a layer (else a nested one)
    root: bool,
    /// its nested instances: (the nested insert's place among the block's nested inserts, instance)
    kids: Vec<(u32, u32)>,
    /// its first row of the hatch table (its variant's hatched styles), its first record of epx (after the pool's: its
    /// variant's fill edges)
    hatch: u32,
    epx: u32,
}

/// the bind groups of the scene's buffers (made again when one of them is new)
pub(crate) struct Groups {
    pub count: BindGroup,
    pub scan: BindGroup,
    pub write: BindGroup,
    pub inst: BindGroup,
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
    insts: Option<Buffer>,
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
    /// instance runs (G6b2): (source, destination, bytes)
    pub insts: Option<(Buffer, Vec<(u64, u64, u64)>)>,
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
    /// the largest storage binding (bytes)
    max_binding: u64,
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
    /// per entry of the visibility: page, instance + 1, first | count << 16, epx offset (cull.wgsl einfo)
    pub(crate) einfo_buf: Buffer,
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
    /// per place of the visible list: the instance of an insert's record there (G6b2; as large as `visible` while there are
    /// instances, else one word)
    vinst: Buffer,
    vinst_cap: u32,
    pub(crate) view_ubo: Buffer,
    pub(crate) groups: Option<Groups>,
    pub(crate) frames: Vec<FrameSlot>,
    pub(crate) ts: Option<Timestamps>,
    /// the tables as they went up last, the visible layers in draw order; the visible list's most entries
    last_order: Vec<[u32; 4]>,
    last_layers: Vec<[u32; 4]>,
    vis_items: u64,
    /// what they were made from (tables_sig)
    tables_sig: u64,
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
    // ---- blocks and their inserts (G6b2; docs/RENDER2D.md "Blöcke")
    blocks: Vec<BlockSlot>,
    free_blocks: Vec<u32>,
    /// the instances of blocks in the order the inserts on layers expand to (each copy, then its nested instances); per
    /// entity slot of an insert on a layer its instances (first, count)
    insts: Vec<Inst>,
    ins_range: Vec<(u32, u32)>,
    /// the derived styles by block (a nested insert's style taking from its parent's: (own, parent) and the style), after
    /// the scene's own in the styles buffer
    derived: Vec<(u32, u32, Style)>,
    /// lay out the instances anew with the next prepare; inserts on layers that moved (their instances' transforms only)
    insts_dirty: bool,
    moved: Vec<u32>,
    /// the camera's scale the instances' zoom bands were last looked at for
    band_scale: f64,
    /// instances whose words go up (first, count), or all of them
    inst_send: Vec<(u32, u32)>,
    inst_send_all: bool,
    pub(crate) inst_buf: Buffer,
    inst_cap: u32,
    spaces: Buffer,
    space_cap: u32,
    pub(crate) inst_count: u32,
    /// the hatch table's rows for instances (after the hatched styles'): (instance, style)
    inst_hatch: Vec<(u32, u32)>,
    /// epx records of the instances' fill edges (after the pool's), per instance where they start
    epx_blocks: u32,
    /// bumped whenever the pool's layout or the instances change (a layer's cached entries are made again)
    layout_gen: u64,
    inst_info: InstanceInfo,
}

impl Scene {
    pub fn new(r: &Renderer) -> Result<Scene> {
        let dev = r.dev.clone();
        let none = BufferUsage::NONE;
        let (entries_cap, layers_cap) = (64u32, 16u32);
        let mut frames = Vec::new();
        for _ in 0..r.desc.frames_in_flight {
            frames.push(FrameSlot { stage: upload(&dev, VIEW_BYTES)?, changes: None, styles: None, hatch: None, scatter_group: None, runs: None, insts: None, readback: None, last: None });
        }
        Ok(Scene {
            tag: r.tag.clone(),
            max_records: r.max_records,
            max_entries: r.max_entries,
            max_binding: r.dev.limits().max_storage_binding as u64,
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
            einfo_buf: storage(&dev, entries_cap as u64 * ENTRY_BYTES, none)?,
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
            vinst: storage(&dev, 4, none)?,
            vinst_cap: 1,
            view_ubo: dev.create_buffer(&BufferDesc { size: VIEW_BYTES, usage: BufferUsage::UNIFORM, memory: MemoryKind::Device })?,
            groups: None,
            frames,
            ts: None,
            last_order: Vec::new(),
            last_layers: Vec::new(),
            vis_items: 0,
            tables_sig: 0,
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
            blocks: Vec::new(),
            free_blocks: Vec::new(),
            insts: Vec::new(),
            ins_range: Vec::new(),
            derived: Vec::new(),
            insts_dirty: false,
            moved: Vec::new(),
            band_scale: 0.0,
            inst_send: Vec::new(),
            inst_send_all: false,
            inst_buf: storage(&dev, INST_BYTES, none)?,
            inst_cap: 1,
            spaces: storage(&dev, SPACE_BYTES, none)?,
            space_cap: 1,
            inst_count: 0,
            inst_hatch: Vec::new(),
            epx_blocks: 0,
            layout_gen: 0,
            inst_info: InstanceInfo::default(),
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
        let hatched = old.hatches > 0;
        *old = *s;
        self.styles_dirty = true;
        if !self.blocks.is_empty() {
            // (blocks: what they need of it (reach, pipeline, the styles derived by block) is looked at again; their records
            // only where a style became hatched or solid: a fill's records name its hatch's place)
            self.insts_dirty = true;
            if hatched != (s.hatches > 0) {
                for b in &mut self.blocks {
                    b.changed |= b.live;
                }
            }
        }
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
        if l.transparency > TRANSPARENCY_MAX {
            return Err(GpuError::Invalid("Layer: a transparency of at most TRANSPARENCY_MAX percent"));
        }
        let fresh = LayerSlot { live: true, era: 0, visible: l.visible, order: l.order, seq, base: 0, cap: 0, used: 0, holes: 0, owners: Vec::new(), repack: false, repacked: false, full: 0, fills: 0,
                                transparency: l.transparency, inserts: 0, cache: Vec::new(), cache_key: (0, 0, u64::MAX) };
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

    /// shown or hidden, its place in the draw order, its transparency (nothing of its entities goes up: the layer table)
    pub fn set_layer(&mut self, id: LayerId, l: &Layer) -> Result<()> {
        let i = self.layer_index(id)?;
        if l.transparency > TRANSPARENCY_MAX {
            return Err(GpuError::Invalid("Layer: a transparency of at most TRANSPARENCY_MAX percent"));
        }
        self.touch();
        let s = &mut self.layers[i];
        if s.order != l.order {
            // (a new order: on top of the layers of that order)
            s.seq = self.layer_seq;
            self.layer_seq += 1;
        }
        s.visible = l.visible;
        s.order = l.order;
        s.transparency = l.transparency;
        Ok(())
    }

    pub fn layer(&self, id: LayerId) -> Result<Layer> {
        let l = &self.layers[self.layer_index(id)?];
        Ok(Layer { visible: l.visible, order: l.order, transparency: l.transparency })
    }

    /// the layer goes, with every entity on it
    pub fn remove_layer(&mut self, id: LayerId) -> Result<()> {
        let i = self.layer_index(id)?;
        self.touch();
        let owners = core::mem::take(&mut self.layers[i].owners);
        let mut last = NONE;
        for o in owners {
            if o != NONE && o != last && self.ents[o as usize].live {
                if let Geom::Insert(ins) = self.ents[o as usize].geom {
                    // (an insert goes: its block is inserted once less, the instances are laid out anew)
                    self.blocks[ins.block.slot as usize].inserted -= 1;
                    self.insts_dirty = true;
                }
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
    /// it costs four sincos, a third of a plain insert).  An insert's block is checked by the caller (check_insert)
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
            Shape::Insert(ins) => {
                ins.check()?;
                Ok(Geom::Insert(ins))
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

    /// a free entity slot
    fn new_slot(&mut self) -> u32 {
        match self.free_ents.pop() {
            Some(f) => f,
            None => {
                self.ents.push(Ent { live: false, era: 0, layer: 0, block: NONE, style: 0, at: 0, alloc: 0, count: 0, dirty: false, class: CLASS_PLAIN, geom: Geom::Line([[0.0; 2]; 2]) });
                self.ents.len() as u32 - 1
            }
        }
    }

    /// an insert's block is alive (and, inside block `into`, makes no cycle and no more than BLOCK_DEPTH levels)
    fn check_insert(&self, s: &Shape, into: Option<usize>) -> Result<()> {
        if let Shape::Insert(ins) = s {
            let b = self.block_index(ins.block)?;
            if let Some(a) = into {
                if a == b || self.reaches(b, a) {
                    return Err(GpuError::Invalid("Insert: a block inside itself (a cycle)"));
                }
                // (the levels below a: those below b and b itself; above a, the deepest chain of blocks holding it)
                if self.blocks[b].height + 1 + self.depth_above(a) + 1 > BLOCK_DEPTH {
                    return Err(GpuError::Invalid("Insert: more than BLOCK_DEPTH levels of blocks inside blocks"));
                }
            }
        }
        Ok(())
    }

    /// a new entity at the end of its layer (drawn over everything there); an insert of a block (Shape::Insert) sits there
    /// with its block's entities
    pub fn insert(&mut self, layer: LayerId, style: StyleId, s: &Shape) -> Result<EntityId> {
        let li = self.layer_index(layer)?;
        if style.0 as usize >= self.styles.len() {
            return Err(GpuError::Invalid("Scene: a style it does not have"));
        }
        self.check_insert(s, None)?;
        let geom = Scene::geom(s, &self.camera, self.size, &self.styles[style.0 as usize])?;
        let count = record_count(&geom);
        self.touch();
        let slot = self.new_slot();
        if let Some(m) = flat_meta(&geom) {
            self.curves.push((slot, m.0, m.1, if matches!(geom, Geom::Fill(_)) { u32::MAX } else { style.0 }));
            self.curves_dirty = true;
        }
        if let Geom::Insert(ins) = geom {
            self.blocks[ins.block.slot as usize].inserted += 1;
            self.layers[li].inserts += 1;
            self.insts_dirty = true;
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
        e.block = NONE;
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

    /// new geometry; the entity keeps its place in the draw order (in its layer or its block).  An insert on a layer that
    /// stays an insert of the same block with as many copies only moves: its instances' transforms go up, nothing else
    pub fn update(&mut self, id: EntityId, s: &Shape) -> Result<()> {
        let i = self.get(id)?;
        let blk = self.ents[i].block;
        self.check_insert(s, if blk == NONE { None } else { Some(blk as usize) })?;
        let geom = Scene::geom(s, &self.camera, self.size, &self.styles[self.ents[i].style as usize])?;
        self.touch();
        if blk != NONE {
            self.block_regeom(i, geom);
            return Ok(());
        }
        if let (Geom::Insert(old), Geom::Insert(new)) = (&self.ents[i].geom, &geom) {
            if old.block == new.block && old.columns == new.columns && old.rows == new.rows {
                self.ents[i].geom = Geom::Insert(*new);
                self.moved.push(i as u32);
                return Ok(());
            }
        }
        if let Some(m) = flat_meta(&geom) {
            // (the later entry of a slot counts)
            self.curves.push((i as u32, m.0, m.1, if matches!(geom, Geom::Fill(_)) { u32::MAX } else { self.ents[i].style }));
            self.curves_dirty = true;
        }
        self.insert_counts(i, -1);
        self.regeom(i, geom);
        self.insert_counts(i, 1);
        Ok(())
    }

    /// a layer's insert counted (+1) or no longer (-1): its block's inserts, its layer's, the instances laid out anew
    fn insert_counts(&mut self, i: usize, d: i32) {
        if let Geom::Insert(ins) = self.ents[i].geom {
            let b = &mut self.blocks[ins.block.slot as usize];
            b.inserted = (b.inserted as i32 + d) as u32;
            let l = &mut self.layers[self.ents[i].layer as usize];
            l.inserts = (l.inserts as i32 + d) as u32;
            self.insts_dirty = true;
        }
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

    /// an entity needs another pipeline for its layer (CLASS_*; a block's entity has none: its inserts' layers count it)
    fn set_class(&mut self, i: usize, class: u8) {
        let e = &mut self.ents[i];
        if e.class != class && e.layer != NONE {
            let l = &mut self.layers[e.layer as usize];
            l.full -= (e.class == CLASS_FULL) as u32;
            l.fills -= (e.class == CLASS_FILL) as u32;
            l.full += (class == CLASS_FULL) as u32;
            l.fills += (class == CLASS_FILL) as u32;
        }
        e.class = class;
    }

    /// another style: its records go up again (a block's entity: its block's, once for all its inserts; an insert: what its
    /// block's entities take by block)
    pub fn set_entity_style(&mut self, id: EntityId, style: StyleId) -> Result<()> {
        let i = self.get(id)?;
        if style.0 as usize >= self.styles.len() {
            return Err(GpuError::Invalid("Scene: a style it does not have"));
        }
        self.touch();
        self.ents[i].style = style.0;
        if self.ents[i].block != NONE {
            self.blocks[self.ents[i].block as usize].changed = true;
            self.insts_dirty |= matches!(self.ents[i].geom, Geom::Insert(_));
            return Ok(());
        }
        if matches!(self.ents[i].geom, Geom::Insert(_)) {
            self.insts_dirty = true;
            return Ok(());
        }
        // (a curve's reach for its window follows the style)
        self.curves_dirty |= matches!(self.ents[i].geom, Geom::Curve(_));
        let class = class_of(&self.ents[i].geom, &self.styles[style.0 as usize]);
        self.set_class(i, class);
        self.mark(i as u32);
        Ok(())
    }

    /// the entity goes; its records become free places (and its id dead); a block's entity leaves its block (its records
    /// go up once for all its inserts)
    pub fn remove(&mut self, id: EntityId) -> Result<()> {
        let i = self.get(id)?;
        self.touch();
        if self.ents[i].block != NONE {
            let b = self.ents[i].block as usize;
            if let Geom::Insert(ins) = self.ents[i].geom {
                self.unnest(b, ins.block.slot as usize);
            }
            let bl = &mut self.blocks[b];
            if let Some(k) = bl.ents.iter().position(|&x| x == i as u32) {
                bl.ents.remove(k);
            }
            bl.changed = true;
            self.insts_dirty = true;
            let e = &mut self.ents[i];
            e.live = false;
            e.era = e.era.wrapping_add(1);
            self.free_ents.push(i as u32);
            self.live -= 1;
            return Ok(());
        }
        self.insert_counts(i, -1);
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

    /// entities alive (a block's entities and inserts each once)
    pub fn len(&self) -> u32 {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// (tests, tools) an entity's records as the last prepare laid them out: its first record's place in the pool
    /// and the records of its geometry (a polyline of n points: n - 1, closed n; a point: 1; an insert on a layer: its one
    /// record of zeros); None for a block's entity (its records lie in its block's variants)
    pub fn locate(&self, id: EntityId) -> Option<(u32, u32)> {
        let i = self.get(id).ok()?;
        let e = &self.ents[i];
        if e.block != NONE {
            return None;
        }
        Some((self.layers[e.layer as usize].base + e.at, e.count))
    }

    // ------------------------------------------------------------ blocks (G6b2)

    /// a new block, empty: entities go into it with insert_in, Shape::Insert places it
    pub fn add_block(&mut self, b: &Block) -> Result<BlockId> {
        if !(b.base[0].is_finite() && b.base[1].is_finite() && b.base[0].abs() < 1e30 && b.base[1].abs() < 1e30) {
            return Err(GpuError::Invalid("Block: a finite base point"));
        }
        self.touch();
        let fresh = BlockSlot { live: true, era: 0, base: b.base, ents: Vec::new(), inserted: 0, nested: Vec::new(), parents: Vec::new(), height: 0, changed: true, banded: false, arcs: false,
                                bounds: None, reach: (0.0, 0.0), reach_bb: false, class: CLASS_PLAIN, bb_dash: false, variants: Vec::new() };
        let slot = match self.free_blocks.pop() {
            Some(s) => {
                let era = self.blocks[s as usize].era;
                self.blocks[s as usize] = BlockSlot { era, ..fresh };
                s
            }
            None => {
                self.blocks.push(fresh);
                self.blocks.len() as u32 - 1
            }
        };
        Ok(BlockId { slot, era: self.blocks[slot as usize].era })
    }

    fn block_index(&self, id: BlockId) -> Result<usize> {
        match self.blocks.get(id.slot as usize) {
            Some(b) if b.live && b.era == id.era => Ok(id.slot as usize),
            _ => Err(GpuError::Invalid("Scene: this block was removed")),
        }
    }

    pub fn block(&self, id: BlockId) -> Result<Block> {
        Ok(Block { base: self.blocks[self.block_index(id)?].base })
    }

    /// a block's base point changed: every insert shows it (its records go up once)
    pub fn set_block(&mut self, id: BlockId, b: &Block) -> Result<()> {
        let k = self.block_index(id)?;
        if !(b.base[0].is_finite() && b.base[1].is_finite() && b.base[0].abs() < 1e30 && b.base[1].abs() < 1e30) {
            return Err(GpuError::Invalid("Block: a finite base point"));
        }
        self.touch();
        self.blocks[k].base = b.base;
        self.blocks[k].changed = true;
        self.insts_dirty = true;
        Ok(())
    }

    /// the block goes with its entities; refused while an insert places it
    pub fn remove_block(&mut self, id: BlockId) -> Result<()> {
        let k = self.block_index(id)?;
        if self.blocks[k].inserted > 0 {
            return Err(GpuError::Invalid("remove_block: the block is inserted (remove its inserts first)"));
        }
        self.touch();
        for i in core::mem::take(&mut self.blocks[k].ents) {
            if let Geom::Insert(ins) = self.ents[i as usize].geom {
                self.unnest(k, ins.block.slot as usize);
            }
            let e = &mut self.ents[i as usize];
            e.live = false;
            e.era = e.era.wrapping_add(1);
            self.free_ents.push(i);
            self.live -= 1;
        }
        let b = &mut self.blocks[k];
        b.live = false;
        b.era = b.era.wrapping_add(1);
        if !b.variants.is_empty() {
            b.variants.clear();
            self.layout_dirty = true;
        }
        self.free_blocks.push(k as u32);
        self.insts_dirty = true;
        Ok(())
    }

    /// a new entity at the end of a block (its units: the block's base lands on an insert's point); a Shape::Insert of
    /// another block nests it (no cycles, at most BLOCK_DEPTH levels).  Every insert of the block shows it with the next
    /// frame, the block's records going up once
    pub fn insert_in(&mut self, block: BlockId, style: StyleId, s: &Shape) -> Result<EntityId> {
        let b = self.block_index(block)?;
        if style.0 as usize >= self.styles.len() {
            return Err(GpuError::Invalid("Scene: a style it does not have"));
        }
        self.check_insert(s, Some(b))?;
        let geom = Scene::geom(s, &self.camera, self.size, &self.styles[style.0 as usize])?;
        self.touch();
        let slot = self.new_slot();
        if let Geom::Insert(ins) = geom {
            self.nest(b, ins.block.slot as usize);
        }
        let e = &mut self.ents[slot as usize];
        e.live = true;
        e.layer = NONE;
        e.block = b as u32;
        e.style = style.0;
        e.at = 0;
        e.alloc = 0;
        e.count = 0;
        e.dirty = false;
        e.class = CLASS_PLAIN;
        e.geom = geom;
        let era = e.era;
        self.live += 1;
        let bl = &mut self.blocks[b];
        bl.ents.push(slot);
        bl.changed = true;
        self.insts_dirty = true;
        Ok(EntityId { slot, era })
    }

    /// a block's entity got new geometry: its block's records are made again (a nested insert changed: the nesting too)
    fn block_regeom(&mut self, i: usize, geom: Geom) {
        let b = self.ents[i].block as usize;
        if let Geom::Insert(ins) = self.ents[i].geom {
            self.unnest(b, ins.block.slot as usize);
        }
        if let Geom::Insert(ins) = geom {
            self.nest(b, ins.block.slot as usize);
        }
        self.ents[i].geom = geom;
        self.blocks[b].changed = true;
        self.insts_dirty = true;
    }

    /// block a inserts block b once more (the caller checked cycles and depth)
    fn nest(&mut self, a: usize, b: usize) {
        self.blocks[a].nested.push(b as u32);
        self.blocks[b].parents.push(a as u32);
        self.blocks[b].inserted += 1;
        self.heights_from(a);
    }

    /// block a inserts block b once less
    fn unnest(&mut self, a: usize, b: usize) {
        if let Some(k) = self.blocks[a].nested.iter().position(|&x| x == b as u32) {
            self.blocks[a].nested.swap_remove(k);
        }
        if let Some(k) = self.blocks[b].parents.iter().position(|&x| x == a as u32) {
            self.blocks[b].parents.swap_remove(k);
        }
        self.blocks[b].inserted -= 1;
        self.heights_from(a);
    }

    /// the levels of blocks below a and every block holding it, again (from those below, which are right)
    fn heights_from(&mut self, a: usize) {
        let mut work = vec![a as u32];
        while let Some(x) = work.pop() {
            let x = x as usize;
            let h = self.blocks[x].nested.iter().map(|&c| self.blocks[c as usize].height + 1).max().unwrap_or(0);
            if h != self.blocks[x].height || x == a {
                self.blocks[x].height = h;
                work.extend_from_slice(&self.blocks[x].parents);
            }
        }
    }

    /// block a holds block b somewhere below it
    fn reaches(&self, a: usize, b: usize) -> bool {
        let mut work = vec![a as u32];
        let mut seen = vec![false; self.blocks.len()];
        while let Some(x) = work.pop() {
            if x as usize == b {
                return true;
            }
            if !core::mem::replace(&mut seen[x as usize], true) {
                work.extend_from_slice(&self.blocks[x as usize].nested);
            }
        }
        false
    }

    /// the deepest chain of blocks holding block a (0: none holds it)
    fn depth_above(&self, a: usize) -> u32 {
        let mut d = 0;
        let mut level = vec![a as u32];
        while !level.is_empty() && d <= BLOCK_DEPTH {
            let mut next: Vec<u32> = level.iter().flat_map(|&x| self.blocks[x as usize].parents.iter().copied()).collect();
            next.sort_unstable();
            next.dedup();
            if next.is_empty() {
                break;
            }
            d += 1;
            level = next;
        }
        d
    }

    /// what the blocks hold and what the last prepare sent of them
    pub fn instance_info(&self) -> InstanceInfo {
        self.inst_info
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
        self.layout_gen += 1;
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
        // then the blocks' variants, each made whole (a variant that did not change is copied across, a new or changed one
        // goes up)
        for b in &mut self.blocks {
            if !b.live {
                continue;
            }
            for v in &mut b.variants {
                let n = v.recs.len() as u32;
                if n == 0 {
                    v.cap = 0;
                    continue;
                }
                let cap = if n > v.cap { pages(n) } else { v.cap };
                if !full && self.filled && v.cap > 0 && !v.dirty {
                    moves.push((v.base as u64 * RECORD_BYTES, base * RECORD_BYTES, n as u64 * RECORD_BYTES));
                } else {
                    v.dirty = true;
                }
                v.base = base as u32;
                v.cap = cap;
                base += cap as u64;
            }
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
        // (an instance's words hold its fill edges' offset in epx, which follows the pool's size and its variant's place)
        self.inst_send_all |= !self.insts.is_empty();
        Ok(())
    }

    /// the buffers of the visibility big enough (new ones: new bind groups)
    fn grow(&mut self) -> Result<()> {
        let dev = self.dev.clone();
        let none = BufferUsage::NONE;
        let entries = self.last_order.len() as u32;
        if entries > self.max_entries || entries >= 1 << 23 {
            return Err(GpuError::Unsupported("Scene: more pages in the visible layers than one dispatch reaches"));
        }
        if entries > self.entries_cap {
            self.entries_cap = entries.max(self.entries_cap * 2);
            let n = self.entries_cap as u64;
            self.einfo_buf = storage(&dev, n * ENTRY_BYTES, none)?;
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
        // (the visible list: every record the entries cover could be in it; an insert's records once per instance)
        let items = self.vis_items.max(self.pool_cap as u64);
        if items > self.visible_cap as u64 {
            if items * 4 > self.max_binding || items > 1 << 31 {
                return Err(GpuError::Unsupported("Scene: more records in the visible layers (an insert's once per instance) than this device's bindings reach"));
            }
            self.visible_cap = items as u32;
            self.visible = storage(&dev, items * 4, none)?;
            self.groups = None;
        }
        let vi = if self.insts.is_empty() { 1 } else { self.visible_cap };
        if vi > self.vinst_cap {
            self.vinst_cap = vi;
            self.vinst = storage(&dev, vi as u64 * 4, none)?;
            self.groups = None;
        }
        let epx = if self.layers.iter().any(|l| l.live && l.fills > 0) || self.epx_blocks > 0 { self.pool_cap as u64 + self.epx_blocks as u64 } else { 1 };
        if epx > self.epx_cap as u64 {
            if epx * 16 > self.max_binding {
                return Err(GpuError::Unsupported("Scene: more fill edges (an insert's once per instance) than this device's bindings reach"));
            }
            self.epx_cap = epx as u32;
            self.epx = storage(&dev, epx * 16, none)?;
            self.groups = None;
        }
        let insts = self.insts.len() as u32;
        if insts > self.inst_cap {
            self.inst_cap = insts.max(self.inst_cap * 2);
            self.inst_buf = storage(&dev, self.inst_cap as u64 * INST_BYTES, none)?;
            self.inst_send_all = true;
            self.groups = None;
        }
        if insts > self.space_cap {
            self.space_cap = insts.max(self.space_cap * 2);
            self.spaces = storage(&dev, self.space_cap as u64 * SPACE_BYTES, none)?;
            self.groups = None;
        }
        let rows = (self.hatched.len() + self.inst_hatch.len()) as u32;
        if rows > self.hatch_cap {
            self.hatch_cap = rows.max(self.hatch_cap * 2);
            self.hatch_buf = storage(&dev, self.hatch_cap as u64 * HATCH_BYTES, none)?;
            self.hatch_for = None;
            self.groups = None;
        }
        let styles = (self.styles.len() + self.derived.len()) as u32;
        if styles > self.styles_cap {
            self.styles_cap = styles.max(self.styles_cap * 2);
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
            count: g(&r.count_layout, &[entry(0, &self.view_ubo), entry(1, &self.pool), entry(2, &self.styles_buf), entry(3, &self.einfo_buf), entry(4, &self.masks),
                                         entry(5, &self.counts), entry(11, &self.epx), entry(12, &self.spaces)])?,
            scan: g(&r.scan_layout, &[entry(5, &self.counts), entry(6, &self.layers_buf), entry(7, &self.offsets), entry(8, &self.lists_buf), entry(9, &self.draws_buf)])?,
            write: g(&r.write_layout, &[entry(3, &self.einfo_buf), entry(4, &self.masks), entry(7, &self.offsets), entry(10, &self.visible), entry(14, &self.vinst)])?,
            inst: g(&r.inst_layout, &[entry(0, &self.view_ubo), entry(12, &self.spaces), entry(13, &self.inst_buf)])?,
            draw: g(&r.draw_layout, &[entry(0, &self.view_ubo), entry(1, &self.pool), entry(2, &self.styles_buf), entry(3, &self.visible), entry(4, &self.lists_buf), entry(5, &self.hatch_buf), entry(6, &self.epx),
                                       entry(7, &self.vinst), entry(8, &self.spaces)])?,
        });
        for f in &mut self.frames {
            f.scatter_group = None;
        }
        Ok(())
    }

    /// the entries of the visibility of the visible layers in draw order (a layer's pages; where an insert sits, its
    /// instances' block pages, split where their nested inserts sit) and the layer table (first entry, entries,
    /// transparency); how many records the entries cover (the visible list's most)
    fn tables(&mut self) -> (Vec<[u32; 4]>, Vec<[u32; 4]>) {
        let mut vis: Vec<usize> = (0..self.layers.len()).filter(|&i| self.layers[i].live && self.layers[i].visible).collect();
        vis.sort_by_key(|&i| (self.layers[i].order, self.layers[i].seq));
        let mut order: Vec<[u32; 4]> = Vec::new();
        let mut table = Vec::with_capacity(vis.len());
        let mut items = 0u64;
        for &i in &vis {
            let first = order.len() as u32;
            let l = &self.layers[i];
            if l.inserts == 0 {
                let p0 = l.base / PAGE;
                let n = l.used.div_ceil(PAGE);
                order.extend((p0..p0 + n).map(|p| [p, 0, (PAGE - 1) << 8, 0]));
                items += n as u64 * PAGE as u64;
            } else {
                let key = (l.base, l.used, self.layout_gen);
                if l.cache_key != key {
                    let mut c = Vec::new();
                    self.layer_entries(i, &mut c);
                    let l = &mut self.layers[i];
                    l.cache = c;
                    l.cache_key = key;
                }
                let l = &self.layers[i];
                order.extend_from_slice(&l.cache);
                items += l.cache.iter().map(|e| (((e[2] >> 8) & 255) as u64 + 1) * (((e[2] >> 16) & 255) as u64 + 1)).sum::<u64>();
            }
            table.push([first, order.len() as u32 - first, self.layers[i].transparency, 0]);
        }
        self.drawn = vis.iter().map(|&i| i as u32).collect();
        self.full = vis.iter().map(|&i| if self.layers[i].fills > 0 { CLASS_FILL } else if self.layers[i].full > 0 { CLASS_FULL } else { CLASS_PLAIN }).collect();
        self.vis_items = items;
        (order, table)
    }

    /// what the entries and the layer table come from (FNV-1a over the layers and the layout's generation)
    fn tables_sig(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut eat = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x0100_0000_01b3);
        };
        eat(self.layout_gen);
        for (k, l) in self.layers.iter().enumerate() {
            if l.live {
                for v in [k as u64, l.base as u64, l.used as u64, l.visible as u64, l.order as u32 as u64, l.seq, l.transparency as u64, l.inserts as u64, l.full as u64, l.fills as u64] {
                    eat(v);
                }
            }
        }
        h
    }

    /// a layer's entries where it holds inserts: its records up to each insert, the insert's instances, the rest.  Inserts of
    /// a small block one after the other (no records between them, consecutive instances of one variant without nested
    /// inserts and fills, at most 128 records) share entries: as many instances as fit a page's 256 records each
    fn layer_entries(&self, li: usize, out: &mut Vec<[u32; 4]>) {
        let l = &self.layers[li];
        let (mut cursor, mut k) = (0u32, 0usize);
        // (the pack being gathered: block, variant, records each, first instance, instances)
        let mut pack: Option<(u32, u32, u32, u32, u32)> = None;
        let flush = |pack: &mut Option<(u32, u32, u32, u32, u32)>, out: &mut Vec<[u32; 4]>| {
            if let Some((b, v, n, first, count)) = pack.take() {
                let base = self.blocks[b as usize].variants[v as usize].base;
                out.push([base / PAGE, first + 1, (n - 1) << 8 | (count - 1) << 16, 0]);
            }
        };
        while k < l.used as usize {
            let o = l.owners[k];
            let mut n = 1;
            while k + n < l.used as usize && l.owners[k + n] == o {
                n += 1;
            }
            if o != NONE && matches!(self.ents[o as usize].geom, Geom::Insert(_)) {
                if (cursor as usize) < k {
                    flush(&mut pack, out);
                    entries_of(l.base, cursor, k as u32, 0, 0, out);
                }
                let (first, count) = self.ins_range.get(o as usize).copied().unwrap_or((0, 0));
                for x in first..first + count {
                    let Some(it) = self.insts.get(x as usize) else { continue };
                    if !it.root {
                        continue;
                    }
                    let v = &self.blocks[it.block as usize].variants[it.variant as usize];
                    let m = v.recs.len() as u32;
                    if v.splits.is_empty() && v.fills.is_none() && (1..=128).contains(&m) {
                        match &mut pack {
                            Some(p) if p.0 == it.block && p.1 == it.variant && p.3 + p.4 == x && (p.4 + 1) * m <= PAGE => p.4 += 1,
                            _ => {
                                flush(&mut pack, out);
                                pack = Some((it.block, it.variant, m, x, 1));
                            }
                        }
                    } else {
                        flush(&mut pack, out);
                        self.inst_entries(x, out);
                    }
                }
                cursor = (k + n) as u32;
            }
            k += n;
        }
        flush(&mut pack, out);
        entries_of(l.base, cursor, l.used, 0, 0, out);
    }

    /// an instance's entries: its variant's records, at each nested insert that one's instances
    fn inst_entries(&self, x: u32, out: &mut Vec<[u32; 4]>) {
        let it = &self.insts[x as usize];
        let v = &self.blocks[it.block as usize].variants[it.variant as usize];
        // (its fill edges' places in epx: its own run after the pool's, epx index = record + eoff)
        let eoff = v.fills.map_or(0, |(f0, _)| (self.pool_cap.wrapping_add(it.epx)).wrapping_sub(v.base + f0));
        let (mut cursor, mut kid) = (0u32, 0usize);
        for (j, &off) in v.splits.iter().enumerate() {
            entries_of(v.base, cursor, off, x + 1, eoff, out);
            cursor = off;
            while kid < it.kids.len() && it.kids[kid].0 == j as u32 {
                self.inst_entries(it.kids[kid].1, out);
                kid += 1;
            }
        }
        entries_of(v.base, cursor, v.recs.len() as u32, x + 1, eoff, out);
    }

    /// the changed records and free places, by their place in the pool (a block's changed variants whole)
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
            if !e.live || e.block != NONE {
                continue;
            }
            let e = &self.ents[slot as usize];
            words.clear();
            records(&e.geom, e.style, [0.0, 0.0], &mut words);
            words.resize(e.alloc as usize, [0; RECORD_WORDS]);
            let at = self.layers[e.layer as usize].base + e.at;
            for (k, w) in words.iter().enumerate() {
                rows.push((at + k as u32, *w));
            }
        }
        for b in &mut self.blocks {
            if !b.live {
                continue;
            }
            for v in &mut b.variants {
                if v.dirty {
                    v.dirty = false;
                    self.inst_info.sent_block_records += v.recs.len() as u32;
                    rows.extend(v.recs.iter().enumerate().map(|(k, w)| (v.base + k as u32, *w)));
                }
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
            let on = self.ents[i].live && self.ents[i].block == NONE;
            self.ents[i].dirty = on;
            if on {
                self.dirty.push(i as u32);
            }
        }
        for b in &mut self.blocks {
            for v in &mut b.variants {
                v.dirty = true;
            }
        }
        self.inst_send_all = true;
        self.styles_dirty = true;
        self.hatch_for = None;
        self.last_order.clear();
        self.last_layers.clear();
        self.last_layers.push([u32::MAX; 4]);
    }

    // ------------------------------------------------------------ blocks in a frame (G6b2)

    /// blocks and their inserts for this frame: what changed blocks hold and their variants made again, the instances
    /// (laid out anew when inserts or blocks changed or an instance needs another zoom band, else the moved inserts'
    /// transforms), the variants they need (made, dropped), the styles derived by block
    fn blocks_prepare(&mut self) -> Result<()> {
        let sent = (self.inst_info.sent_instances, self.inst_info.sent_bytes);
        self.inst_info = InstanceInfo::default();
        let _ = sent;
        let changed = self.blocks.iter().any(|b| b.live && b.changed);
        if changed || self.insts_dirty {
            for k in 0..self.blocks.len() {
                if self.blocks[k].live {
                    self.block_meta(k);
                }
            }
        }
        // changed blocks: their variants made again where they are (a region too small: a new pool)
        if changed {
            for k in 0..self.blocks.len() {
                if !(self.blocks[k].live && self.blocks[k].changed) {
                    continue;
                }
                for j in 0..self.blocks[k].variants.len() {
                    let (band, uneven) = (self.blocks[k].variants[j].band, self.blocks[k].variants[j].uneven);
                    let fresh = self.make_variant(k, band, uneven);
                    let v = &mut self.blocks[k].variants[j];
                    if fresh.recs.len() as u32 > v.cap {
                        self.layout_dirty = true;
                    }
                    let (base, cap) = (v.base, v.cap);
                    *v = Variant { base, cap, ..fresh };
                }
                self.blocks[k].changed = false;
            }
        }
        let mut rebuild = self.insts_dirty || changed;
        if !rebuild && !self.moved.is_empty() {
            rebuild = self.move_insts();
        }
        if !rebuild && self.camera.scale != self.band_scale {
            rebuild = self.bands_stale();
        }
        self.band_scale = self.camera.scale;
        self.moved.clear();
        if rebuild {
            self.rebuild_insts()?;
        }
        self.insts_dirty = false;
        self.inst_count = self.insts.len() as u32;
        Ok(())
    }

    /// what block k holds: curves or fills, arcs, its box, its strokes' reach, its pipeline
    fn block_meta(&mut self, k: usize) {
        let base = self.blocks[k].base;
        let (mut banded, mut arcs, mut bb, mut bb_dash, mut class) = (false, false, false, false, CLASS_PLAIN);
        let mut bounds: Option<[f64; 4]> = None;
        let mut reach = (0.0f64, 0.0f64);
        let mut grow = |b: [f64; 4]| {
            let b = [b[0] - base[0], b[1] - base[1], b[2] - base[0], b[3] - base[1]];
            bounds = Some(match bounds {
                Some(o) => [o[0].min(b[0]), o[1].min(b[1]), o[2].max(b[2]), o[3].max(b[3])],
                None => b,
            });
        };
        for &slot in &self.blocks[k].ents {
            let e = &self.ents[slot as usize];
            let st = &self.styles[e.style as usize];
            match &e.geom {
                Geom::Insert(_) => continue,
                Geom::Line(p) => grow([p[0][0].min(p[1][0]), p[0][1].min(p[1][1]), p[0][0].max(p[1][0]), p[0][1].max(p[1][1])]),
                Geom::Poly(p, _) => {
                    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
                    for q in p {
                        b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
                    }
                    grow(b);
                }
                Geom::Arc(c, r, _, _) => {
                    arcs = true;
                    grow([c[0] - r, c[1] - r, c[0] + r, c[1] + r]);
                }
                Geom::Curve(g) => {
                    banded = true;
                    grow(g.bounds);
                }
                Geom::Fill(g) => {
                    banded = true;
                    grow(g.bounds);
                }
            }
            if st.by_block & BY_BLOCK_WIDTH != 0 {
                bb = true;
            } else {
                let r = style_reach(st);
                if st.unit == Unit::Drawing { reach.1 = reach.1.max(r) } else { reach.0 = reach.0.max(r) }
            }
            let mut own = *st;
            if st.by_block & BY_BLOCK_DASH != 0 {
                bb_dash = true;
                own.dashes = 0;
            }
            class = class.max(class_of(&e.geom, &own));
        }
        let b = &mut self.blocks[k];
        b.banded = banded;
        b.arcs = arcs;
        b.bounds = bounds;
        b.reach = reach;
        b.reach_bb = bb;
        b.bb_dash = bb_dash;
        b.class = class;
    }

    /// block k's records for a zoom band (i32::MIN: none; no curves then) and arcs as curves (uneven scales): its
    /// entities in their order, in its units (less its base); its curves and fills flattened whole for the band (no
    /// window: an insert's view of them differs per insert), at most CURVE_MAX segments each
    fn make_variant(&self, k: usize, band: i32, uneven: bool) -> Variant {
        let bl = &self.blocks[k];
        let mut recs = Vec::new();
        let (mut splits, mut hatched) = (Vec::new(), Vec::new());
        let mut fills: Option<(u32, u32)> = None;
        let tol = if band == i32::MIN { 1.0 } else { curve::FLAT_TOL * curve::pow2(band) };
        for &slot in &bl.ents {
            let e = &self.ents[slot as usize];
            let at = recs.len() as u32;
            match &e.geom {
                Geom::Insert(_) => splits.push(at),
                Geom::Curve(g) => {
                    let flat = flat_whole(&g.c, tol);
                    records(&Geom::Curve(Box::new(CurveG { c: g.c.clone(), flat, st: g.st, bounds: g.bounds })), e.style, bl.base, &mut recs);
                }
                &Geom::Arc(c, r, a0, sw) if uneven => {
                    let cv = Curve::ellipse(c, [r, r], 0.0, a0, sw);
                    let flat = flat_whole(&cv, tol);
                    let bounds = cv.bounds();
                    records(&Geom::Curve(Box::new(CurveG { c: cv, flat, st: FlatState { band, coarse: 0, window: None }, bounds })), e.style, bl.base, &mut recs);
                }
                Geom::Fill(g) => {
                    let pts: Vec<Vec<[f64; 2]>> = g.rings.iter().map(|c| flat_whole(c, tol).pts).collect();
                    let mut p = fill::prepare(&pts);
                    p.near(curve::pow2(band + 1) * curve::HYST);
                    let edges = p.rings.iter().map(|r| r.len()).sum::<usize>() as u32;
                    records(&Geom::Fill(Box::new(FillG { rings: Vec::new(), prep: p, st: g.st, bounds: g.bounds })), e.style, bl.base, &mut recs);
                    if edges > 0 {
                        fills = Some(match fills {
                            Some((a, b)) => (a.min(at), b.max(at + edges)),
                            None => (at, at + edges),
                        });
                    }
                    if self.styles[e.style as usize].hatches > 0 {
                        // (its hatch: the style's place among the variant's hatched ones + 1, in the top byte of each record)
                        let ls = match hatched.iter().position(|&x| x == e.style) {
                            Some(p) => p,
                            None => {
                                hatched.push(e.style);
                                hatched.len() - 1
                            }
                        };
                        if ls < 255 {
                            for r in &mut recs[at as usize..] {
                                r[15] |= (ls as u32 + 1) << 24;
                            }
                        }
                    }
                }
                g => records(g, e.style, bl.base, &mut recs),
            }
        }
        Variant { band, uneven, recs, base: 0, cap: 0, splits, fills, hatched, dirty: true, used: true }
    }

    /// the instances of a block at xf with the style by block `ins` (and of its nested inserts, depth first) into out; its
    /// index
    fn expand(&self, b: u32, xf: Xf, ins: u32, root: bool, out: &mut Vec<Inst>, derived: &mut Vec<(u32, u32)>) -> u32 {
        let k = out.len() as u32;
        out.push(Inst { block: b, variant: 0, xf, ins, band: i32::MIN, uneven: false, root, kids: Vec::new(), hatch: 0, epx: 0 });
        let bl = &self.blocks[b as usize];
        let mut j = 0u32;
        let mut xfs = Vec::new();
        for &slot in &bl.ents {
            let e = &self.ents[slot as usize];
            let Geom::Insert(ni) = e.geom else { continue };
            // (what its block's entities take by block: the nested insert's style, itself taking from its parent's)
            let ins2 = if self.styles[e.style as usize].takes() {
                let key = (e.style, ins);
                let p = match derived.iter().position(|&d| d == key) {
                    Some(p) => p,
                    None => {
                        derived.push(key);
                        derived.len() - 1
                    }
                };
                (self.styles.len() + p) as u32
            } else {
                e.style
            };
            xfs.clear();
            block::insert_xfs(&ni, bl.base, &mut xfs);
            for x2 in xfs.clone() {
                let kid = self.expand(ni.block.slot, xf.then(&x2), ins2, false, out, derived);
                out[k as usize].kids.push((j, kid));
            }
            j += 1;
        }
        k
    }

    /// the zoom band and evenness an instance of block b at xf needs (its variant's key)
    fn band_for(&self, b: u32, xf: &Xf) -> (i32, bool) {
        let bl = &self.blocks[b as usize];
        let uneven = bl.arcs && !xf.even();
        if bl.banded || uneven {
            let (hi, _) = xf.singular();
            (curve::band_of(self.camera.scale / hi), uneven)
        } else {
            (i32::MIN, false)
        }
    }

    /// a style by block (one of the scene's or a derived one)
    fn style_at(&self, k: u32) -> &Style {
        let n = self.styles.len();
        if (k as usize) < n { &self.styles[k as usize] } else { &self.derived[k as usize - n].2 }
    }

    /// the instances laid out anew: every insert on a layer expanded (its copies, depth first through its nested inserts),
    /// their variants (made where new, dropped where no instance takes them), the derived styles, epx runs, hatch rows, the
    /// inserts' pipelines
    fn rebuild_insts(&mut self) -> Result<()> {
        self.layout_gen += 1;
        self.inst_info.rebuilt = true;
        let mut insts = Vec::new();
        let mut derived = Vec::new();
        let mut ranges = vec![(0u32, 0u32); self.ents.len()];
        let mut xfs = Vec::new();
        // (the inserts in their layers' order: consecutive inserts of a layer get consecutive instances, which entries share)
        let mut order = Vec::new();
        for l in &self.layers {
            if !l.live || l.inserts == 0 {
                continue;
            }
            let mut last = NONE;
            for &o in &l.owners {
                if o != NONE && o != last && matches!(self.ents[o as usize].geom, Geom::Insert(_)) {
                    order.push(o as usize);
                }
                last = o;
            }
        }
        for slot in order {
            let e = &self.ents[slot];
            if !e.live || e.block != NONE {
                continue;
            }
            let Geom::Insert(ins) = e.geom else { continue };
            let first = insts.len() as u32;
            xfs.clear();
            block::insert_xfs(&ins, [0.0, 0.0], &mut xfs);
            for xf in xfs.clone() {
                self.expand(ins.block.slot, xf, e.style, true, &mut insts, &mut derived);
                if insts.len() as u64 * INST_BYTES > self.max_binding || insts.len() >= 1 << 24 {
                    return Err(GpuError::Unsupported("Scene: more instances of blocks than this device's bindings reach"));
                }
            }
            ranges[slot] = (first, insts.len() as u32 - first);
        }
        self.ins_range = ranges;
        // the derived styles, parents before children
        let mut dv: Vec<(u32, u32, Style)> = Vec::with_capacity(derived.len());
        for &(own, parent) in &derived {
            let p = if (parent as usize) < self.styles.len() { self.styles[parent as usize] } else { dv[parent as usize - self.styles.len()].2 };
            dv.push((own, parent, p.by_block_from(&self.styles[own as usize])));
        }
        if dv.iter().map(|d| (d.0, d.1)).ne(self.derived.iter().map(|d| (d.0, d.1))) || dv.iter().map(|d| d.2).ne(self.derived.iter().map(|d| d.2)) {
            self.styles_dirty = true;
        }
        self.derived = dv;
        // their variants
        for b in &mut self.blocks {
            for v in &mut b.variants {
                v.used = false;
            }
        }
        for it in &mut insts {
            let (band, uneven) = self.band_for(it.block, &it.xf);
            it.band = band;
            it.uneven = uneven;
            let b = it.block as usize;
            let j = match self.blocks[b].variants.iter().position(|v| v.band == band && v.uneven == uneven) {
                Some(j) => j,
                None => {
                    let v = self.make_variant(b, band, uneven);
                    self.blocks[b].variants.push(v);
                    self.layout_dirty = true;
                    self.blocks[b].variants.len() - 1
                }
            };
            self.blocks[b].variants[j].used = true;
            it.variant = j as u32;
        }
        // (variants no instance takes go; the others' places renumbered)
        for b in 0..self.blocks.len() {
            if self.blocks[b].variants.iter().all(|v| v.used) {
                continue;
            }
            let mut map = Vec::with_capacity(self.blocks[b].variants.len());
            let mut n = 0u32;
            for v in &self.blocks[b].variants {
                map.push(n);
                n += v.used as u32;
            }
            self.blocks[b].variants.retain(|v| v.used);
            for it in &mut insts {
                if it.block as usize == b {
                    it.variant = map[it.variant as usize];
                }
            }
            self.layout_dirty = true;
        }
        // epx runs of their fill edges, the hatch table's rows
        self.epx_blocks = 0;
        self.inst_hatch.clear();
        let plain = self.hatched.len() as u32;
        for (k, it) in insts.iter_mut().enumerate() {
            let v = &self.blocks[it.block as usize].variants[it.variant as usize];
            if let Some((f0, f1)) = v.fills {
                it.epx = self.epx_blocks;
                self.epx_blocks += f1 - f0;
            }
            if !v.hatched.is_empty() {
                it.hatch = plain + self.inst_hatch.len() as u32;
                self.inst_hatch.extend(v.hatched.iter().map(|&s| (k as u32, s)));
            }
        }
        self.insts = insts;
        // the inserts' pipelines: the most any of their instances' blocks needs
        for slot in 0..self.ents.len() {
            if !self.ents[slot].live || self.ents[slot].block != NONE || !matches!(self.ents[slot].geom, Geom::Insert(_)) {
                continue;
            }
            let (first, count) = self.ins_range[slot];
            let mut class = CLASS_PLAIN;
            for x in first..first + count {
                let it = &self.insts[x as usize];
                let bl = &self.blocks[it.block as usize];
                class = class.max(bl.class);
                if bl.bb_dash && self.style_at(it.ins).dashes > 0 {
                    class = class.max(CLASS_FULL);
                }
            }
            self.set_class(slot, class);
        }
        self.inst_send_all = true;
        self.inst_send.clear();
        self.hatch_for = None;
        Ok(())
    }

    /// the inserts that moved: their instances' transforms again (the same instances, depth first); true when one needs
    /// another variant (the instances are then laid out anew)
    fn move_insts(&mut self) -> bool {
        let mut moved = core::mem::take(&mut self.moved);
        moved.sort_unstable();
        moved.dedup();
        let mut tmp = Vec::new();
        let mut derived = Vec::new();
        let mut xfs = Vec::new();
        for &slot in &moved {
            let e = &self.ents[slot as usize];
            let Geom::Insert(ins) = e.geom else { continue };
            if !e.live || e.block != NONE {
                continue;
            }
            tmp.clear();
            xfs.clear();
            block::insert_xfs(&ins, [0.0, 0.0], &mut xfs);
            for xf in xfs.clone() {
                self.expand(ins.block.slot, xf, e.style, true, &mut tmp, &mut derived);
            }
            let (first, count) = self.ins_range[slot as usize];
            if tmp.len() != count as usize {
                return true;
            }
            for (j, t) in tmp.iter().enumerate() {
                let k = first as usize + j;
                if self.band_for(t.block, &t.xf) != (self.insts[k].band, self.insts[k].uneven) {
                    return true;
                }
                if !self.blocks[t.block as usize].variants[self.insts[k].variant as usize].hatched.is_empty() {
                    // (its hatch rows follow its transform)
                    self.hatch_for = None;
                }
                self.insts[k].xf = t.xf;
            }
            self.inst_send.push((first, count));
        }
        false
    }

    /// an instance's zoom band no longer holds for the camera's scale (beyond the band and its hysteresis)
    fn bands_stale(&self) -> bool {
        let s = self.camera.scale;
        self.insts.iter().any(|it| {
            if it.band == i32::MIN {
                return false;
            }
            let (hi, _) = it.xf.singular();
            let x = s / hi;
            x < curve::pow2(it.band) / curve::HYST || x >= curve::pow2(it.band + 1) * curve::HYST
        })
    }

    /// cull.wgsl's Inst of instance k
    fn inst_words(&self, k: usize) -> [u32; INST_WORDS] {
        let it = &self.insts[k];
        let bl = &self.blocks[it.block as usize];
        let mut reach = bl.reach;
        if bl.reach_bb {
            let st = self.style_at(it.ins);
            let r = style_reach(st);
            if st.unit == Unit::Drawing { reach.1 = reach.1.max(r) } else { reach.0 = reach.0.max(r) }
        }
        let v = &bl.variants[it.variant as usize];
        let eoff = v.fills.map_or(0, |(f0, _)| (self.pool_cap.wrapping_add(it.epx)).wrapping_sub(v.base + f0));
        block::inst_words(&it.xf, bl.bounds.unwrap_or([0.0; 4]), (reach.0 as f32, reach.1 as f32), it.ins, it.hatch, eoff)
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
        self.blocks_prepare()?;
        self.layout(full)?;
        // (the entries and the layer table again only when what they come from changed: with many inserts they are long)
        let sig = self.tables_sig();
        let mut tables_new = false;
        if full || sig != self.tables_sig {
            let (order, table) = self.tables();
            if table.len() as u32 > LAYERS_MAX {
                return Err(GpuError::Unsupported("Scene: more than 65535 visible layers"));
            }
            tables_new = order != self.last_order || table != self.last_layers;
            self.last_order = order;
            self.last_layers = table;
            self.tables_sig = sig;
        }
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
            // (the scene's styles, then those derived by block: what nested inserts give their blocks' entities)
            let words: Vec<[u32; STYLE_WORDS]> = self.styles.iter().enumerate().map(|(k, s)| s.words(scale, self.hatch_of[k])).chain(self.derived.iter().map(|d| d.2.words(scale, 0))).collect();
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
        if !(self.hatched.is_empty() && self.inst_hatch.is_empty()) && self.hatch_for != Some((self.camera, size)) {
            let mut words: Vec<[f32; HATCH_WORDS]> = Vec::with_capacity(self.hatched.len() + self.inst_hatch.len());
            for &k in &self.hatched {
                words.push(self.styles[k as usize].hatch_words(&self.camera, size));
            }
            // (G6b2: an instance's hatched fills, the hatch's lines through its transform in f64; only near the view, the
            // others' rows are never read: their instances are out of view, the GPU's test of that is the stricter one)
            let ctx = FlatCtx::new(&self.camera, size);
            for &(k, st) in &self.inst_hatch {
                let it = &self.insts[k as usize];
                let bl = &self.blocks[it.block as usize];
                let near = bl.bounds.is_some_and(|b| {
                    let mut w = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
                    for q in [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]] {
                        let p = it.xf.apply(q);
                        w = [w[0].min(p[0]), w[1].min(p[1]), w[2].max(p[0]), w[3].max(p[1])];
                    }
                    ctx.near(&w, 8.0)
                });
                if near {
                    let style = &self.styles[st as usize];
                    let (f, n) = block::fams(style);
                    let mut t = [f[0]; HATCH_MAX];
                    for j in 0..n {
                        t[j] = f[j].through(&it.xf, bl.base);
                    }
                    words.push(style.hatch_words_of(&t[..n], &self.camera, size));
                } else {
                    words.push([0.0; HATCH_WORDS]);
                }
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
            let (ob, lb) = (self.last_order.len() as u64 * ENTRY_BYTES, self.last_layers.len() as u64 * 16);
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
        // the instances' transforms (G6b2): those of inserts that moved, or all when they were laid out anew; runs of
        // neighbouring instances, a copy each (more than 4096: all of them in one)
        if self.inst_send_all || !self.inst_send.is_empty() {
            let n = self.insts.len() as u32;
            let mut ranges = if self.inst_send_all { vec![(0, n)] } else { core::mem::take(&mut self.inst_send) };
            ranges.sort_unstable();
            let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
            for (a, c) in ranges {
                match merged.last_mut() {
                    Some(m) if m.0 + m.1 >= a => m.1 = m.1.max(a + c - m.0),
                    _ => merged.push((a, c)),
                }
            }
            if merged.len() > 4096 {
                merged = vec![(0, n)];
            }
            merged.retain(|m| m.1 > 0);
            let rows: u32 = merged.iter().map(|m| m.1).sum();
            if rows > 0 {
                let size = rows as u64 * INST_BYTES;
                let f = &mut self.frames[slot];
                if f.insts.as_ref().is_none_or(|b| b.size() < size) {
                    f.insts = Some(upload(&dev, size.next_power_of_two().max(1 << 12))?);
                }
                let Some(buf) = f.insts.clone() else { return Err(GpuError::Invalid("Scene::prepare")) };
                let mut words: Vec<[u32; INST_WORDS]> = Vec::with_capacity(rows as usize);
                let mut list = Vec::with_capacity(merged.len());
                for &(a, c) in &merged {
                    list.push((words.len() as u64 * INST_BYTES, a as u64 * INST_BYTES, c as u64 * INST_BYTES));
                    words.extend((a..a + c).map(|k| self.inst_words(k as usize)));
                }
                buf.write(0, &words)?;
                self.up.insts = Some((buf, list));
                self.inst_info.sent_instances = rows;
                self.inst_info.sent_bytes = size;
                bytes += size;
            }
            self.inst_send_all = false;
            self.inst_send.clear();
        }
        self.inst_info.blocks = self.blocks.iter().filter(|b| b.live).count() as u32;
        self.inst_info.variants = self.blocks.iter().map(|b| b.variants.len() as u32).sum();
        self.inst_info.block_records = self.blocks.iter().flat_map(|b| b.variants.iter()).map(|v| v.recs.len() as u32).sum();
        self.inst_info.instances = self.insts.len() as u32;
        self.inst_info.entries = self.entries;
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

/// records from `from` to `to` (of a region starting at record `base`) as entries of the visibility, page by page: (page,
/// instance + 1, first | (count - 1) << 8 (one instance), epx offset)
fn entries_of(base: u32, from: u32, to: u32, inst: u32, eoff: u32, out: &mut Vec<[u32; 4]>) {
    if from >= to {
        return;
    }
    let (a, b) = (base + from, base + to);
    let mut p = a / PAGE;
    while p * PAGE < b {
        let lo = a.max(p * PAGE) - p * PAGE;
        let hi = b.min((p + 1) * PAGE) - p * PAGE;
        out.push([p, inst, lo | (hi - lo - 1) << 8, eoff]);
        p += 1;
    }
}

/// a curve flattened whole within a tolerance (coarser, times 4 each, until it takes at most CURVE_MAX segments: a block's
/// curve, the same for every insert)
fn flat_whole(c: &Curve, tol: f64) -> Flat {
    let mut t = tol;
    for _ in 0..40 {
        if let Some(f) = curve::flatten(c, t, None, 0.0, curve::CURVE_MAX + 1) {
            return f;
        }
        t *= 4.0;
    }
    curve::flatten(c, f64::MAX, None, f64::MAX, usize::MAX).unwrap_or_default()
}

/// how far a style's stroke reaches beyond its centre line, in its width's unit (an insert's culling: half the width,
/// at least half a pixel, times a miter's bound or a square cap's corner, and a cap of a dash beyond a vertex)
fn style_reach(st: &Style) -> f64 {
    let h = (0.5 * st.width as f64).max(if st.unit == Unit::Px { 0.5 } else { 0.0 });
    let bound = if st.join == Join::Miter { sqrt((st.miter_limit as f64 * st.miter_limit as f64 - 1.0).max(0.0)).max(1.0) } else { 1.0 };
    h * (bound.max(1.18) + 1.0)
}

/// the pipeline an entity's layer needs: a fill the third, an arc or a dashed style the second (lib.rs); an insert's is
/// its blocks' (Scene::rebuild_insts)
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
        Geom::Insert(_) => 1,
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

/// common.wgsl's Records of an entity; shift: taken off every point first (a block's base: its records in its own units;
/// on a layer none).  An insert on a layer: one record of zeros (its place in the order)
fn records(g: &Geom, style: u32, shift: [f64; 2], out: &mut Vec<[u32; RECORD_WORDS]>) {
    if let Geom::Insert(_) = g {
        out.push([0; RECORD_WORDS]);
        return;
    }
    let b = |v: f32| v.to_bits();
    let point = |p: [f64; 2]| -> [u32; 4] {
        let (x, lx) = split(p[0] - shift[0]);
        let (y, ly) = split(p[1] - shift[1]);
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
        // fill's first record lies (v.z) and up to 4 edges near it (u16 from there, words 11, 12; their number in bits 16 to
        // 23 of w.w); a triangle that distance (a.x), its corners' edges (a.yzw) and up to 20 edges near it (words 4 to 13).
        // Bits 24 to 31 of w.w: in a block's variant, its hatched style's place among the variant's + 1 (make_variant)
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
