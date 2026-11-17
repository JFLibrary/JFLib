// dxf -- a DXF drawing into a Scene (G6e2; docs/RENDER2D.md "DXF"): lib/dxf reads the file (streaming, bounded), this
// module maps what it reads onto render2d as it comes: a render2d layer per run of entities on one DXF layer (the file's
// order is the painter's order, as in AutoCAD), styles from colours, linetypes, lineweights and transparency resolved
// by layer and by block, blocks as blocks (DIMENSION and ACAD_TABLE through their anonymous blocks), the object
// coordinate systems projected onto the drawing's plane, polyline bulges as arcs, text in substitute fonts of the
// repository, MTEXT's format codes laid out here into single-line texts.
//
//   dxf::load(&mut scene, &bytes, &Options) -> Result<Loaded, ::dxf::Error>      a file in memory
//   dxf::read(&mut scene, &mut source, &Options) -> Result<Loaded, ::dxf::Error> piece by piece (a file of a gigabyte)
//   loaded.extents, kinds (per DXF kind: read, drawn, skipped in paper space, hidden, unsupported, failed, approximated),
//        layers (each DXF layer's render2d layers), set_layer_visible, group_of / members (the render2d entities one
//        DXF entity became)
//
// What render2d refuses of an entity (a degenerate arc, a ring without area) is counted as failed, the rest goes on.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::f64::consts::{FRAC_PI_2, PI, TAU};

use ::dxf as d;

use crate::math::{sincos, sqrt};
use crate::scene::{Block, BlockId, Cap, EntityId, Insert, Join, Layer, LayerId, Scene, Shape, Style, StyleId, Transparency, Unit, BY_BLOCK_COLOR, BY_BLOCK_DASH, BY_BLOCK_WIDTH,
                   DASH_MAX, TRANSPARENCY_MAX};
use crate::text::{FontId, TextStyleId};

pub use ::dxf::Family;

/// how a drawing is mapped.  Made with `new` and changed field by field (it may grow)
#[non_exhaustive]
#[derive(Clone, Copy)]
pub struct Options<'a> {
    /// the view's background (0xFFRRGGBB): colour 7 is black on a light one and white on a dark one, wipeouts take it
    pub background: u32,
    /// lineweights shown (None: as the header's $LWDISPLAY says, off without it); pixels per millimetre of lineweight
    /// (96 / 25.4: 0.25 mm and less a hairline, as AutoCAD shows them in model space)
    pub lineweights: Option<bool>,
    pub px_per_mm: f64,
    /// the bytes of a family of the repository's fonts (None, or a family without bytes: Family::Sans's; no fonts: no
    /// text)
    pub fonts: Option<&'a dyn Fn(Family) -> Option<&'a [u8]>>,
    pub limits: d::Limits,
    /// keep which render2d entities came from one DXF entity (Loaded::group_of, members; 12 bytes per entity)
    pub groups: bool,
    /// render2d layers at most for the runs of DXF layers (beyond: an entity goes to the last run of its layer, the order
    /// across layers then approximate)
    pub max_layers: u32,
    /// the order of the first render2d layer (the next ones count up)
    pub order: i32,
}

impl<'a> Options<'a> {
    pub fn new() -> Options<'a> {
        Options { background: 0xFFFF_FFFF, lineweights: None, px_per_mm: 96.0 / 25.4, fonts: None, limits: d::Limits::DEFAULT, groups: false, max_layers: 1024, order: 0 }
    }
}

impl Default for Options<'_> {
    fn default() -> Self {
        Options::new()
    }
}

/// what happened to the entities of one DXF kind (an entity in a block once, however often it is inserted)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Count {
    /// read (top level and in blocks), of which drawn (one render2d entity or more), skipped in paper space, hidden
    /// (invisible, or in a block on a layer that is off or frozen), unsupported (a kind this module does not draw),
    /// failed (render2d refused its geometry, or there was none), approximated (drawn, but simplified: see the notes)
    pub read: u64,
    pub drawn: u64,
    pub paper: u64,
    pub hidden: u64,
    pub unsupported: u64,
    pub failed: u64,
    pub approximated: u64,
}

/// a DXF layer: its render2d layers (one per run of entities on it, in order), whether it is shown
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct LayerOf {
    pub name: String,
    pub runs: Vec<LayerId>,
    pub visible: bool,
    pub transparency: u32,
}

/// what a drawing became
#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    pub header: d::Header,
    pub layers: Vec<LayerOf>,
    /// the box of what was drawn in model space (min x, min y, max x, max y), without rays and xlines
    pub extents: Option<[f64; 4]>,
    /// per DXF kind
    pub kinds: BTreeMap<String, Count>,
    /// render2d entities inserted (on layers and in blocks), blocks made
    pub entities: u64,
    pub blocks: u32,
    /// the reader's counts and notes
    pub stats: d::Stats,
    /// what was simplified or skipped, each once (at most 64)
    pub notes: Vec<String>,
    groups: Vec<(u32, u32)>,
    parts: Vec<EntityId>,
    slot_group: Vec<u32>,
}

impl Loaded {
    /// the DXF entity a render2d entity on a layer came from (Options::groups): its group, numbered from 1 in the order
    /// of the drawing's entities that drew something
    pub fn group_of(&self, id: EntityId) -> Option<u32> {
        self.slot_group.get(id.slot as usize).copied().filter(|&g| g > 0 && (g as usize) <= self.groups.len() && self.members(g).contains(&id))
    }

    /// the render2d entities a DXF entity became (group_of)
    pub fn members(&self, g: u32) -> &[EntityId] {
        match self.groups.get((g as usize).wrapping_sub(1)) {
            Some(&(a, n)) => &self.parts[a as usize..(a + n) as usize],
            None => &[],
        }
    }

    /// a DXF layer shown or hidden: its render2d layers (entities of blocks on other layers keep what they had when they
    /// were read: docs/RENDER2D.md "DXF")
    pub fn set_layer_visible(&mut self, scene: &mut Scene, name: &str, on: bool) -> gpu::Result<bool> {
        let Some(l) = self.layers.iter_mut().find(|l| l.name.eq_ignore_ascii_case(name)) else { return Ok(false) };
        l.visible = on;
        for &id in &l.runs {
            let mut x = scene.layer(id)?;
            x.visible = on;
            scene.set_layer(id, &x)?;
        }
        Ok(true)
    }

    fn note(&mut self, s: String) {
        if self.notes.len() < 64 && !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }
}

/// a drawing in memory into a scene
pub fn load(scene: &mut Scene, bytes: &[u8], o: &Options) -> Result<Loaded, d::Error> {
    if d::is_binary(bytes) {
        return Err(d::Error::Binary);
    }
    let mut at = 0usize;
    let mut src = |buf: &mut [u8]| {
        let n = buf.len().min(bytes.len() - at);
        buf[..n].copy_from_slice(&bytes[at..at + n]);
        at += n;
        n
    };
    read(scene, &mut src, o)
}

/// a drawing read piece by piece into a scene (::dxf::read: only the tables and blocks are kept)
pub fn read(scene: &mut Scene, source: &mut dyn FnMut(&mut [u8]) -> usize, o: &Options) -> Result<Loaded, d::Error> {
    let mut m = Map::new(scene, *o);
    let (stats, _names) = d::read(source, &o.limits, &mut m)?;
    let mut out = m.out;
    for n in &stats.notes {
        out.note(n.clone());
    }
    out.stats = stats;
    Ok(out)
}

/// a linetype's or hatch line's dashes as DXF has them (positive on, negative off, 0 a dot), scaled, as render2d's
/// pattern (on, off, ... at most `max` entries) and its phase; None: solid (no gap in it).  Runs of one sign merge, a
/// leading gap moves to the end (the phase says where an entity starts), too many entries lose their shortest dashes
/// (dots first) to the gaps around them, so the period stays
pub fn linetype_pattern(src: &[f64], scale: f64, max: usize) -> Option<(Vec<f64>, f64)> {
    Map::pattern(src, scale, max)
}

// ---------------------------------------------------------------------------------------------------- geometry

pub(crate) type P = [f64; 2];

pub(crate) fn atan2(y: f64, x: f64) -> f64 {
    let a = crate::geo::angle_of([x, y]);
    if a > PI { a - TAU } else { a }
}

fn rad(deg: f64) -> f64 {
    deg * (PI / 180.0)
}

/// the largest integer not above x (no libm)
pub(crate) fn floor(x: f64) -> f64 {
    if !x.is_finite() || x.abs() >= 4.5e15 {
        return x;
    }
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

/// x modulo m in [0, m)
pub(crate) fn rem(x: f64, m: f64) -> f64 {
    let r = x - floor(x / m) * m;
    if r >= m || r < 0.0 { 0.0 } else { r }
}

fn len(v: P) -> f64 {
    sqrt(v[0] * v[0] + v[1] * v[1])
}

fn fine(v: f64) -> bool {
    v.is_finite() && v.abs() < 1e29
}

fn fine_p(p: P) -> bool {
    fine(p[0]) && fine(p[1])
}

/// an object coordinate system projected onto the drawing's plane: p -> t + x a + y b (+ z c)
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ocs {
    pub a: P,
    pub b: P,
    pub c: P,
}

impl Ocs {
    pub const WCS: Ocs = Ocs { a: [1.0, 0.0], b: [0.0, 1.0], c: [0.0, 0.0] };

    /// AutoCAD's arbitrary axis algorithm for a normal
    pub fn of(n: d::P3) -> Ocs {
        let l = sqrt(n[0] * n[0] + n[1] * n[1] + n[2] * n[2]);
        if !(l > 0.0) || !l.is_finite() {
            return Ocs::WCS;
        }
        let n = [n[0] / l, n[1] / l, n[2] / l];
        if n[0].abs() < 1e-12 && n[1].abs() < 1e-12 && n[2] > 0.0 {
            return Ocs::WCS;
        }
        let cross = |u: d::P3, v: d::P3| [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let ax = if n[0].abs() < 1.0 / 64.0 && n[1].abs() < 1.0 / 64.0 { cross([0.0, 1.0, 0.0], n) } else { cross([0.0, 0.0, 1.0], n) };
        let al = sqrt(ax[0] * ax[0] + ax[1] * ax[1] + ax[2] * ax[2]);
        let ax = [ax[0] / al, ax[1] / al, ax[2] / al];
        let ay = cross(n, ax);
        Ocs { a: [ax[0], ax[1]], b: [ay[0], ay[1]], c: [n[0], n[1]] }
    }

    pub fn p(&self, q: d::P3) -> P {
        [q[0] * self.a[0] + q[1] * self.b[0] + q[2] * self.c[0], q[0] * self.a[1] + q[1] * self.b[1] + q[2] * self.c[1]]
    }

    /// a direction (no z)
    pub fn v(&self, x: f64, y: f64) -> P {
        [x * self.a[0] + y * self.b[0], x * self.a[1] + y * self.b[1]]
    }

    pub fn det(&self) -> f64 {
        self.a[0] * self.b[1] - self.a[1] * self.b[0]
    }

    /// whether circles stay circles (a rotation, maybe mirrored)
    pub fn conformal(&self) -> bool {
        let (la, lb) = (len(self.a), len(self.b));
        (la - 1.0).abs() < 1e-9 && (lb - 1.0).abs() < 1e-9 && (self.a[0] * self.b[0] + self.a[1] * self.b[1]).abs() < 1e-9
    }

    /// the angle of the OCS x axis in the drawing
    pub fn angle(&self) -> f64 {
        atan2(self.a[1], self.a[0])
    }
}

/// a shape with its points owned (render2d's Shape borrows them)
#[derive(Clone, Debug)]
pub(crate) enum Own {
    Line(P, P),
    Poly(Vec<P>, bool),
    Arc(P, f64, f64, f64),
    Circle(P, f64),
    Ellipse(P, [f64; 2], f64, f64, f64),
    Spline(u32, Vec<P>, Vec<f64>, Vec<f64>, bool),
    Cubic(Vec<P>),
}

impl Own {
    pub fn shape(&self) -> Shape<'_> {
        match self {
            Own::Line(a, b) => Shape::Line { a: *a, b: *b },
            Own::Poly(p, c) => Shape::Polyline { points: p, closed: *c },
            Own::Arc(c, r, s, w) => Shape::Arc { center: *c, radius: *r, start: *s, sweep: *w },
            Own::Circle(c, r) => Shape::Circle { center: *c, radius: *r },
            Own::Ellipse(c, r, rot, s, w) => Shape::Ellipse { center: *c, radii: *r, rotation: *rot, start: *s, sweep: *w },
            Own::Spline(deg, p, w, k, c) => Shape::Spline { degree: *deg, points: p, weights: w, knots: k, closed: *c },
            Own::Cubic(p) => Shape::Cubic { points: p, closed: false },
        }
    }

    /// a box around it (control points for curves)
    pub fn bounds(&self) -> [f64; 4] {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let mut add = |p: P| {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        };
        match self {
            Own::Line(a, c) => {
                add(*a);
                add(*c);
            }
            Own::Poly(p, _) | Own::Spline(_, p, _, _, _) | Own::Cubic(p) => p.iter().for_each(|q| add(*q)),
            Own::Circle(c, r) => {
                add([c[0] - r, c[1] - r]);
                add([c[0] + r, c[1] + r]);
            }
            // an arc's own box (a polyline's flat arcs have radii of millions of units: not their circle's), an
            // ellipse's from its points every 1/64 turn and the sagitta of that step
            Own::Arc(c, r, s, w) => arc_box(*c, [*r, *r], 0.0, *s, *w, &mut add),
            Own::Ellipse(c, r, rot, s, w) => arc_box(*c, *r, *rot, *s, *w, &mut add),
        }
        b
    }
}

/// the box of an elliptic arc into `add`: its ends, the extremes of a circle's arc along the axes, an ellipse's points at
/// every 1/64 turn grown by the sagitta of that step
fn arc_box(c: P, r: [f64; 2], rot: f64, s: f64, w: f64, add: &mut dyn FnMut(P)) {
    let sweep = if w.abs() >= TAU { TAU } else { w.abs() };
    let start = if w < 0.0 { s + w } else { s };
    let (sr, cr) = sincos(rot);
    let at = |t: f64| {
        let (st, ct) = sincos(t);
        let (x, y) = (r[0] * ct, r[1] * st);
        [c[0] + cr * x - sr * y, c[1] + sr * x + cr * y]
    };
    add(at(start));
    add(at(start + sweep));
    if r[0] == r[1] {
        // the axis points within the sweep
        let k0 = floor(start / FRAC_PI_2) as i64 + 1;
        let mut k = k0;
        while (k as f64) * FRAC_PI_2 < start + sweep && k < k0 + 5 {
            add(at(k as f64 * FRAC_PI_2));
            k += 1;
        }
        return;
    }
    let n = ((sweep / (TAU / 64.0)) as usize + 1).min(64);
    let step = sweep / n as f64;
    let sag = r[0].max(r[1]) * (1.0 - sincos(step / 2.0).1);
    for k in 0..=n {
        let p = at(start + step * k as f64);
        add([p[0] - sag, p[1] - sag]);
        add([p[0] + sag, p[1] + sag]);
    }
}

/// c + cos t u + sin t v for t from t0 over sweep as render2d's ellipse (its principal axes from the conjugate ones);
/// None when it is flat
pub(crate) fn ellipse_of(c: P, u: P, v: P, t0: f64, sweep: f64) -> Option<Own> {
    let dot = |p: P, q: P| p[0] * q[0] + p[1] * q[1];
    let (uu, vv, uv) = (dot(u, u), dot(v, v), dot(u, v));
    let ts = 0.5 * atan2(2.0 * uv, uu - vv);
    let (s, co) = sincos(ts);
    let a = [u[0] * co + v[0] * s, u[1] * co + v[1] * s];
    let b = [-u[0] * s + v[0] * co, -u[1] * s + v[1] * co];
    let (ra, rb) = (len(a), len(b));
    let big = ra.max(rb);
    if !(big > 0.0) || ra < big * 1e-9 || rb < big * 1e-9 || !fine(ra) || !fine(rb) {
        return None;
    }
    let rot = atan2(a[1], a[0]);
    let cross = a[0] * b[1] - a[1] * b[0];
    if cross >= 0.0 {
        Some(Own::Ellipse(c, [ra, rb], rot, t0 - ts, sweep))
    } else {
        Some(Own::Ellipse(c, [ra, rb], rot, -(t0 - ts), -sweep))
    }
}

/// a circle's arc (centre, radius, start and sweep in OCS radians) through an OCS
pub(crate) fn arc_shape(o: &Ocs, c: d::P3, r: f64, start: f64, sweep: f64) -> Option<Own> {
    let cc = o.p(c);
    if !(r > 0.0) || !fine(r) || !fine_p(cc) || sweep == 0.0 || !sweep.is_finite() {
        return None;
    }
    if o.conformal() {
        let phi = o.angle();
        let full = sweep.abs() >= TAU - 1e-12;
        if o.det() > 0.0 {
            return Some(if full { Own::Circle(cc, r) } else { Own::Arc(cc, r, start + phi, sweep) });
        }
        // mirrored: the OCS angle t lands at phi - t, and the arc runs the other way
        return Some(if full { Own::Circle(cc, r) } else { Own::Arc(cc, r, phi - start, -sweep) });
    }
    ellipse_of(cc, o.v(r, 0.0), o.v(0.0, r), start, sweep)
}

/// a polyline's segment with a bulge as an arc: centre, radius, start angle, sweep (signed); None: straight
pub(crate) fn bulge_of(p: P, q: P, b: f64) -> Option<(P, f64, f64, f64)> {
    if !(b.abs() > 1e-10) || !b.is_finite() {
        return None;
    }
    let dvec = [q[0] - p[0], q[1] - p[1]];
    let l = len(dvec);
    if !(l > 0.0) {
        return None;
    }
    let theta = 4.0 * atan2(b, 1.0);
    let r = l * (1.0 + b * b) / (4.0 * b.abs());
    let sag = b.abs() * l / 2.0;
    let mid = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
    // the centre lies left of p -> q for a counter-clockwise arc (b > 0), at r - sagitta from the chord's middle
    let left = [-dvec[1] / l, dvec[0] / l];
    let k = b.signum() * (r - sag);
    let c = [mid[0] + left[0] * k, mid[1] + left[1] * k];
    let start = atan2(p[1] - c[1], p[0] - c[0]);
    (fine(r) && fine_p(c)).then_some((c, r, start, theta))
}

/// a polyline's segments (OCS points with bulges) as one rational quadratic B-spline through the drawing: straight
/// pieces with their middle point, arcs in pieces of at most a quarter turn (exact; an affine map keeps them exact)
fn bulged_spline(o: &Ocs, v: &[(d::P3, f64)], closed: bool) -> Option<Own> {
    let n = v.len();
    if n < 2 {
        return None;
    }
    let segs = if closed { n } else { n - 1 };
    let mut pts: Vec<P> = Vec::new();
    let mut w: Vec<f64> = Vec::new();
    // the pieces in OCS: (start, control, end, weight)
    let ocs2 = |p: d::P3| [p[0], p[1]];
    let z = v[0].0[2];
    let to = |p: P| o.p([p[0], p[1], z]);
    let mut pieces: Vec<(P, P, P, f64)> = Vec::new();
    for i in 0..segs {
        let (a, b) = (ocs2(v[i].0), ocs2(v[(i + 1) % n].0));
        if a == b {
            continue;
        }
        match bulge_of(a, b, v[i].1) {
            Some((c, r, s, theta)) => {
                let k = ((theta.abs() / FRAC_PI_2) - 1e-9).max(0.0) as usize + 1;
                let phi = theta / k as f64;
                for j in 0..k {
                    let t0 = s + phi * j as f64;
                    let t1 = t0 + phi;
                    let (s0, c0) = sincos(t0);
                    let (s1, c1) = sincos(t1);
                    let p0 = if j == 0 { a } else { [c[0] + r * c0, c[1] + r * s0] };
                    let p1 = if j + 1 == k { b } else { [c[0] + r * c1, c[1] + r * s1] };
                    let (sm, cm) = sincos(0.5 * (t0 + t1));
                    let (_, ch) = sincos(0.5 * phi);
                    // the control point where the tangents meet: on the middle radius at r / cos(phi / 2)
                    let pc = [c[0] + r * cm / ch, c[1] + r * sm / ch];
                    pieces.push((p0, pc, p1, ch));
                }
            }
            None => pieces.push((a, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], b, 1.0)),
        }
    }
    if pieces.is_empty() {
        return None;
    }
    let mut knots = vec![0.0, 0.0, 0.0];
    for (k, &(p0, pc, p1, wc)) in pieces.iter().enumerate() {
        if k == 0 {
            pts.push(to(p0));
            w.push(1.0);
        }
        pts.push(to(pc));
        w.push(wc);
        pts.push(to(p1));
        w.push(1.0);
        let kv = (k + 1) as f64;
        if k + 1 < pieces.len() {
            knots.push(kv);
            knots.push(kv);
        } else {
            knots.extend_from_slice(&[kv, kv, kv]);
        }
    }
    if !pts.iter().all(|p| fine_p(*p)) {
        return None;
    }
    Some(Own::Spline(2, pts, w, knots, false))
}

// ---------------------------------------------------------------------------------------------------- the mapping

/// properties resolved for a style
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Props {
    pub color: u32,
    pub by_block: u32,
    /// the dash pattern (on, off, ... drawing units, scaled) and its phase
    pub dash: Option<(Vec<f32>, f32)>,
    pub lw_px: f32,
    pub transparency: Transparency,
    /// a complex linetype was simplified
    pub complex: bool,
}

#[derive(Clone, Copy)]
struct LayerProps {
    color: u32,
    on: bool,
    frozen: bool,
    linetype: Option<d::Name>,
    lineweight: d::Lineweight,
    transparency: u32,
}

/// where shapes go: a run (a render2d layer) or a block
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tgt {
    Layer(LayerId),
    Block(BlockId),
}

#[derive(Clone)]
struct Blk {
    id: BlockId,
    base: P,
    defined: bool,
    bounds: Option<[f64; 4]>,
}

pub(crate) struct Map<'s, 'o> {
    pub s: &'s mut Scene,
    pub o: Options<'o>,
    pub h: d::Header,
    pub t: d::Tables,
    /// colour 7
    pub fg: u32,
    lw_on: bool,
    /// a layer's index in t.layers by its name
    layer_ix: BTreeMap<u32, usize>,
    /// a DXF layer's entry in out.layers by its name
    layer_of: BTreeMap<u32, usize>,
    run: Option<(u32, LayerId)>,
    runs: u32,
    next_order: i32,
    styles: BTreeMap<Vec<u32>, StyleId>,
    /// linetypes (by name): their pattern unscaled, whether complex; None: continuous
    lts: BTreeMap<u32, Option<(Vec<f64>, bool)>>,
    pub fonts: BTreeMap<Family, Option<FontId>>,
    pub tstyles: BTreeMap<(Family, u64, u64), TextStyleId>,
    pub corr: BTreeMap<Family, f64>,
    blocks: BTreeMap<u32, Blk>,
    pub out: Loaded,
    ordinal: u32,
    part0: usize,
    /// the entity being mapped: its kind's entry, whether it was approximated
    pub approx: bool,
    /// the box of what the current model space entity drew
    bounds: Option<[f64; 4]>,
    /// the block being defined (its bounds grow)
    cur_block: Option<u32>,
    /// the text style STANDARD (a text without 7 takes it)
    pub standard: Option<d::Name>,
}

fn grow(b: &mut Option<[f64; 4]>, o: [f64; 4]) {
    if !(o[0] <= o[2] && o[1] <= o[3]) || !o.iter().all(|v| v.is_finite()) {
        return;
    }
    *b = Some(match *b {
        Some(a) => [a[0].min(o[0]), a[1].min(o[1]), a[2].max(o[2]), a[3].max(o[3])],
        None => o,
    });
}

impl<'s, 'o> Map<'s, 'o> {
    fn new(s: &'s mut Scene, o: Options<'o>) -> Map<'s, 'o> {
        let bg = o.background;
        let lum = 0.299 * ((bg >> 16) & 255) as f64 + 0.587 * ((bg >> 8) & 255) as f64 + 0.114 * (bg & 255) as f64;
        let fg = if lum > 128.0 { 0xFF00_0000 } else { 0xFFFF_FFFF };
        Map { s, o, h: d::Header::default(), t: d::Tables::default(), fg, lw_on: false, layer_ix: BTreeMap::new(), layer_of: BTreeMap::new(), run: None, runs: 0, next_order: o.order,
              styles: BTreeMap::new(), lts: BTreeMap::new(), fonts: BTreeMap::new(), tstyles: BTreeMap::new(), corr: BTreeMap::new(), blocks: BTreeMap::new(), out: Loaded::default(),
              ordinal: 0, part0: 0, approx: false, bounds: None, cur_block: None, standard: None }
    }

    pub fn note(&mut self, s: String) {
        self.out.note(s);
    }

    /// an ACI or true colour as 0xFFRRGGBB (7 and 255 white: the foreground)
    pub fn rgb(&self, c: d::Color) -> u32 {
        match c {
            d::Color::Rgb(v) => 0xFF00_0000 | (v & 0xFF_FFFF),
            d::Color::Aci(7) => self.fg,
            d::Color::Aci(i) => 0xFF00_0000 | d::aci_rgb(i),
            _ => self.fg,
        }
    }

    fn layer_props(&self, n: d::Name) -> LayerProps {
        match self.layer_ix.get(&n.0).and_then(|&i| self.t.layers.get(i)) {
            Some(l) => LayerProps {
                color: self.rgb(l.color),
                on: l.on,
                frozen: l.frozen,
                linetype: l.linetype,
                lineweight: l.lineweight,
                transparency: l.transparency.map_or(0, |a| alpha_percent(a)),
            },
            None => LayerProps { color: self.fg, on: true, frozen: false, linetype: None, lineweight: d::Lineweight::Default, transparency: 0 },
        }
    }

    /// the render2d layer for an entity on a DXF layer: the current run, or a new one (the file's order across layers)
    fn run_for(&mut self, n: d::Name, names: &d::Names) -> Option<LayerId> {
        if let Some((l, id)) = self.run {
            if l == n.0 {
                return Some(id);
            }
        }
        let k = match self.layer_of.get(&n.0) {
            Some(&k) => k,
            None => {
                let lp = self.layer_props(n);
                self.out.layers.push(LayerOf { name: String::from(names.get(n)), runs: Vec::new(), visible: lp.on && !lp.frozen, transparency: lp.transparency });
                let k = self.out.layers.len() - 1;
                self.layer_of.insert(n.0, k);
                k
            }
        };
        let id = if self.runs >= self.o.max_layers && !self.out.layers[k].runs.is_empty() {
            self.note(format!("more than {} runs of layers: later entities join the last run of their layer (the order across layers approximate)", self.o.max_layers));
            *self.out.layers[k].runs.last()?
        } else {
            let lo = &self.out.layers[k];
            let mut l = Layer::new(self.next_order);
            l.visible = lo.visible;
            l.transparency = lo.transparency.min(TRANSPARENCY_MAX);
            let id = match self.s.add_layer(&l) {
                Ok(id) => id,
                Err(_) => {
                    self.note(String::from("render2d refused a layer"));
                    return None;
                }
            };
            self.next_order = self.next_order.saturating_add(1);
            self.runs += 1;
            self.out.layers[k].runs.push(id);
            id
        };
        self.run = Some((n.0, id));
        Some(id)
    }

    /// a linetype's pattern (unscaled, signed as DXF has it) and whether it is complex; None: continuous
    fn linetype(&mut self, n: d::Name, names: &d::Names) -> Option<(Vec<f64>, bool)> {
        if let Some(v) = self.lts.get(&n.0) {
            return v.clone();
        }
        let up = names.get(n).to_ascii_uppercase();
        let v = if up == "CONTINUOUS" || up.is_empty() {
            None
        } else {
            match self.t.linetype(n) {
                Some(lt) if !lt.dashes.is_empty() => {
                    let complex = lt.dashes.iter().any(|d| d.complex != 0);
                    Some((lt.dashes.iter().map(|d| d.length).collect(), complex))
                }
                _ => None,
            }
        };
        self.lts.insert(n.0, v.clone());
        v
    }

    /// a signed DXF pattern (positive on, negative off, 0 a dot) scaled: render2d's on / off entries (at most `max`) and
    /// the phase; None: solid
    pub fn pattern(src: &[f64], scale: f64, max: usize) -> Option<(Vec<f64>, f64)> {
        // merge runs of one sign (a dot is "on" of length 0)
        let mut runs: Vec<(bool, f64)> = Vec::new();
        for &v in src {
            if !v.is_finite() {
                continue;
            }
            let on = v >= 0.0;
            let l = v.abs() * scale;
            match runs.last_mut() {
                Some(last) if last.0 == on && !(on && (l == 0.0 || last.1 == 0.0)) => last.1 += l,
                _ => runs.push((on, l)),
            }
        }
        if runs.is_empty() || !runs.iter().any(|r| !r.0) {
            return None;
        }
        // (a pattern of one sign after merging: solid)
        if runs.iter().all(|r| r.0) {
            return None;
        }
        // start with "on": the gaps before the first dash go to the end, the phase says where the entity starts
        let mut phase = 0.0;
        let first_on = runs.iter().position(|r| r.0)?;
        if first_on > 0 {
            let head: Vec<(bool, f64)> = runs.drain(..first_on).collect();
            phase = -head.iter().map(|r| r.1).sum::<f64>();
            runs.extend(head);
        }
        // two "off" or two "on" in a row after the rotation: merge again
        let mut m: Vec<(bool, f64)> = Vec::new();
        for r in runs {
            match m.last_mut() {
                Some(last) if last.0 == r.0 && !(r.0 && (r.1 == 0.0 || last.1 == 0.0)) => last.1 += r.1,
                Some(last) if last.0 == r.0 => {
                    // two dots or a dot and a dash in a row: a gap of 0 between them
                    let on = r.0;
                    let _ = last;
                    m.push((!on, 0.0));
                    m.push(r);
                }
                _ => m.push(r),
            }
        }
        // the last must be "off" (render2d's pattern alternates on, off, ...)
        if m.last().is_some_and(|r| r.0) {
            if m.len() >= 2 && m[0].0 {
                // join the last dash with the first: the pattern starts that much earlier
                let l = m.pop().map_or(0.0, |r| r.1);
                m[0].1 += l;
                phase += l;
            } else {
                m.push((false, 0.0));
            }
        }
        // too many entries: the shortest dashes (dots first) become gaps
        while m.len() > max && m.len() >= 4 {
            let mut best = usize::MAX;
            for k in (0..m.len()).step_by(2) {
                if k > 0 && (best == usize::MAX || m[k].1 < m[best].1) {
                    best = k;
                }
            }
            if best == usize::MAX {
                break;
            }
            let merged = m[best - 1].1 + m[best].1 + m.get(best + 1).map_or(0.0, |r| r.1);
            m[best - 1].1 = merged;
            m.remove(best);
            if best < m.len() {
                m.remove(best);
            }
        }
        let total: f64 = m.iter().map(|r| r.1).sum();
        if !(total > 0.0) || !total.is_finite() || m.len() > max {
            return None;
        }
        let phase = rem(phase, total);
        Some((m.iter().map(|r| r.1).collect(), if phase.abs() < 1e-12 * total { 0.0 } else { phase }))
    }

    /// an entity's properties in model space or in a block (in_block: ByBlock is the insert's; entities on layer 0 take
    /// what is by layer from the insert as well: docs/RENDER2D.md "DXF")
    pub fn props(&mut self, c: &d::Common, in_block: bool, names: &d::Names) -> Props {
        let lp = self.layer_props(c.layer);
        let layer0 = c.layer == d::Name(0);
        let mut by_block = 0;
        let color = match c.color {
            d::Color::ByLayer if in_block && layer0 => {
                by_block |= BY_BLOCK_COLOR;
                self.fg
            }
            d::Color::ByLayer => lp.color,
            d::Color::ByBlock => {
                if in_block {
                    by_block |= BY_BLOCK_COLOR;
                }
                self.fg
            }
            other => self.rgb(other),
        };
        // the linetype
        let lt_name = match c.linetype {
            d::Linetype::ByLayer if in_block && layer0 => {
                by_block |= BY_BLOCK_DASH;
                None
            }
            d::Linetype::ByLayer => lp.linetype,
            d::Linetype::ByBlock => {
                if in_block {
                    by_block |= BY_BLOCK_DASH;
                }
                None
            }
            d::Linetype::Named(n) => Some(n),
        };
        let mut complex = false;
        let dash = lt_name.and_then(|n| self.linetype(n, names)).and_then(|(pat, cx)| {
            complex = cx;
            let scale = self.h.ltscale * c.ltscale;
            Map::pattern(&pat, scale, DASH_MAX).map(|(p, ph)| (p.iter().map(|&v| v as f32).collect(), ph as f32))
        });
        // the lineweight (pixels; 0 a hairline)
        let lw = match c.lineweight {
            d::Lineweight::ByLayer if in_block && layer0 => {
                by_block |= BY_BLOCK_WIDTH;
                d::Lineweight::Default
            }
            d::Lineweight::ByLayer => lp.lineweight,
            d::Lineweight::ByBlock => {
                if in_block {
                    by_block |= BY_BLOCK_WIDTH;
                }
                d::Lineweight::Default
            }
            v => v,
        };
        let mm = match lw {
            d::Lineweight::Mm100(v) => v as f64 / 100.0,
            _ => self.h.lwdefault as f64 / 100.0,
        };
        let lw_px = if self.lw_on && mm > 0.25 { (mm * self.o.px_per_mm) as f32 } else { 0.0 };
        let transparency = match c.transparency {
            d::Transparency::ByLayer if in_block && !layer0 => Transparency::Percent(lp.transparency.min(TRANSPARENCY_MAX)),
            d::Transparency::ByLayer => Transparency::ByLayer,
            d::Transparency::ByBlock if in_block => Transparency::ByBlock,
            d::Transparency::ByBlock => Transparency::Percent(0),
            d::Transparency::Alpha(a) => Transparency::Percent(alpha_percent(a).min(TRANSPARENCY_MAX)),
        };
        Props { color, by_block, dash, lw_px, transparency, complex }
    }

    fn style_id(&mut self, st: &Style) -> Option<StyleId> {
        let mut key: Vec<u32> = vec![st.color, st.width.to_bits(), st.unit as u32, st.cap as u32, st.join as u32, st.miter_limit.to_bits(), st.dashes, st.dash_unit as u32,
                                     st.dash_phase.to_bits(), st.by_block];
        key.extend(st.dash[..st.dashes as usize].iter().map(|v| v.to_bits()));
        key.push(match st.transparency {
            Transparency::ByLayer => 1000,
            Transparency::ByBlock => 1001,
            Transparency::Percent(p) => p,
        });
        if st.hatches > 0 {
            // (hatched styles differ per entity: not shared)
            return self.s.add_style(st).ok();
        }
        if let Some(&id) = self.styles.get(&key) {
            return Some(id);
        }
        let id = self.s.add_style(st).ok()?;
        self.styles.insert(key, id);
        Some(id)
    }

    /// a stroke's style: lineweights round, a pattern in drawing units
    pub fn stroke(&mut self, p: &Props) -> Option<StyleId> {
        let mut st = Style::new(p.color);
        if p.lw_px > 0.0 {
            st.width = p.lw_px;
            st.cap = Cap::Round;
            st.join = Join::Round;
        }
        if let Some((pat, ph)) = &p.dash {
            st = st.dashed(pat, Unit::Drawing, *ph);
            if p.lw_px == 0.0 {
                // (a hairline's dashes and dots at least a pixel long, as AutoCAD draws them: square ends)
                st.cap = Cap::Square;
            }
        }
        st.by_block = p.by_block;
        st.transparency = p.transparency;
        self.style_id(&st)
    }

    /// a fill's style (colour, transparency, by block)
    pub fn fill_style(&mut self, p: &Props) -> Option<StyleId> {
        let mut st = Style::new(p.color);
        st.by_block = p.by_block & BY_BLOCK_COLOR;
        st.transparency = p.transparency;
        self.style_id(&st)
    }

    /// a wide polyline's stroke (drawing units, butt ends, mitered joins as AutoCAD draws them)
    fn wide_stroke(&mut self, p: &Props, w: f64) -> Option<StyleId> {
        let mut st = Style::new(p.color);
        st.width = w as f32;
        st.unit = Unit::Drawing;
        st.cap = Cap::Butt;
        st.join = Join::Miter;
        st.miter_limit = 10.0;
        st.by_block = p.by_block & BY_BLOCK_COLOR;
        st.transparency = p.transparency;
        self.style_id(&st)
    }

    /// a point's style: a square of a pixel (or the lineweight)
    fn point_style(&mut self, p: &Props) -> Option<StyleId> {
        let mut st = Style::new(p.color);
        st.width = p.lw_px;
        st.cap = Cap::Square;
        st.by_block = p.by_block & (BY_BLOCK_COLOR | BY_BLOCK_WIDTH);
        st.transparency = p.transparency;
        self.style_id(&st)
    }

    /// a shape into the target; false when render2d refused it
    pub fn put(&mut self, tgt: Tgt, style: StyleId, sh: &Shape) -> bool {
        let r = match tgt {
            Tgt::Layer(l) => self.s.insert(l, style, sh),
            Tgt::Block(b) => self.s.insert_in(b, style, sh),
        };
        match r {
            Ok(id) => {
                self.out.entities += 1;
                if let Tgt::Layer(_) = tgt {
                    if self.o.groups {
                        self.out.parts.push(id);
                    }
                }
                true
            }
            Err(_) => false,
        }
    }

    /// an owned shape in, its box counted
    pub fn put_own(&mut self, tgt: Tgt, style: StyleId, sh: &Own) -> bool {
        let ok = self.put(tgt, style, &sh.shape());
        if ok {
            self.grow_by(sh.bounds());
        }
        ok
    }

    pub fn grow_by(&mut self, b: [f64; 4]) {
        grow(&mut self.bounds, b);
    }

    /// a ring set as a fill
    pub fn put_fill(&mut self, tgt: Tgt, style: StyleId, rings: &[Vec<Own>]) -> bool {
        let shapes: Vec<Vec<Shape>> = rings.iter().map(|r| r.iter().map(|s| s.shape()).collect()).collect();
        let refs: Vec<&[Shape]> = shapes.iter().map(|r| r.as_slice()).collect();
        let ok = self.put(tgt, style, &Shape::Fill { rings: &refs });
        if ok {
            for r in rings {
                for s in r {
                    self.grow_by(s.bounds());
                }
            }
        }
        ok
    }

    /// the block of a name, made when it is first named
    fn block_id(&mut self, n: d::Name) -> Option<BlockId> {
        if let Some(b) = self.blocks.get(&n.0) {
            return Some(b.id);
        }
        let id = self.s.add_block(&Block::new()).ok()?;
        self.out.blocks += 1;
        self.blocks.insert(n.0, Blk { id, base: [0.0, 0.0], defined: false, bounds: None });
        Some(id)
    }

    // ---------------------------------------------------------------------------------------------------- entities

    fn count(&mut self, kind: &str) -> &mut Count {
        if !self.out.kinds.contains_key(kind) {
            self.out.kinds.insert(String::from(kind), Count::default());
        }
        self.out.kinds.get_mut(kind).expect("just inserted")
    }

    /// an entity into a block (blk) or model space
    fn map_entity(&mut self, e: &d::Entity, names: &d::Names, blk: Option<BlockId>) {
        let kind = names.get(e.common.kind).to_ascii_uppercase();
        self.count(&kind).read += 1;
        if e.common.invisible {
            self.count(&kind).hidden += 1;
            return;
        }
        let in_block = blk.is_some();
        if in_block && e.common.layer != d::Name(0) {
            let lp = self.layer_props(e.common.layer);
            if !lp.on || lp.frozen {
                self.count(&kind).hidden += 1;
                return;
            }
        }
        if matches!(e.kind, d::Kind::Other) {
            self.count(&kind).unsupported += 1;
            return;
        }
        if matches!(&e.kind, d::Kind::BlockRef(r) if r.block.is_none()) {
            // a dimension written without its block (QCAD does): drawing it from its points is not built
            self.count(&kind).unsupported += 1;
            self.note(format!("{kind} without its block: not drawn (dimensions are drawn through their blocks only)"));
            return;
        }
        let tgt = match blk {
            Some(b) => Tgt::Block(b),
            None => match self.run_for(e.common.layer, names) {
                Some(l) => Tgt::Layer(l),
                None => {
                    self.count(&kind).failed += 1;
                    return;
                }
            },
        };
        self.approx = false;
        self.bounds = None;
        self.part0 = self.out.parts.len();
        let p = self.props(&e.common, in_block, names);
        if p.complex {
            self.approx = true;
            self.note(String::from("complex linetypes: their shapes and texts left out, the dashes kept"));
        }
        let o = Ocs::of(e.common.extrusion);
        let drawn = match self.kind(e, &kind, tgt, &p, &o, names, in_block) {
            Some(d) => d,
            None => {
                self.count(&kind).hidden += 1;
                return;
            }
        };
        let c = self.count(&kind);
        if drawn {
            c.drawn += 1;
        } else {
            c.failed += 1;
        }
        if drawn && self.approx {
            self.count(&kind).approximated += 1;
        }
        // the box: model space's extents, a block's own
        let b = self.bounds;
        if let Some(b) = b {
            if !matches!(e.kind, d::Kind::Ray { .. }) {
                if in_block {
                    if let Some(name) = self.cur_block {
                        if let Some(bl) = self.blocks.get_mut(&name) {
                            grow(&mut bl.bounds, b);
                        }
                    }
                } else {
                    grow(&mut self.out.extents, b);
                }
            }
        }
        if !in_block && self.o.groups {
            self.ordinal += 1;
            let n = (self.out.parts.len() - self.part0) as u32;
            if n > 0 {
                self.out.groups.push((self.part0 as u32, n));
                let g = self.out.groups.len() as u32;
                for k in self.part0..self.out.parts.len() {
                    let slot = self.out.parts[k].slot as usize;
                    if self.out.slot_group.len() <= slot {
                        self.out.slot_group.resize(slot + 1, 0);
                    }
                    self.out.slot_group[slot] = g;
                }
            }
        }
    }

    /// an entity's geometry: Some(true) drawn, Some(false) refused or empty, None hidden (an invisible attribute, an
    /// ATTDEF in a block, a point of PDMODE 1, an empty text)
    #[allow(clippy::too_many_arguments)]
    fn kind(&mut self, e: &d::Entity, kind: &str, tgt: Tgt, p: &Props, o: &Ocs, names: &d::Names, in_block: bool) -> Option<bool> {
        let done = |b: bool| Some(b);
        match &e.kind {
            d::Kind::Line { a, b } => {
                let (a, b) = ([a[0], a[1]], [b[0], b[1]]);
                if !fine_p(a) || !fine_p(b) {
                    return Some(false);
                }
                let Some(st) = self.stroke(p) else { return Some(false) };
                done(self.put_own(tgt, st, &Own::Line(a, b)))
            }
            d::Kind::Point { at, angle } => self.point(tgt, p, [at[0], at[1]], *angle),
            d::Kind::Circle { center, radius } => {
                let Some(sh) = arc_shape(o, *center, *radius, 0.0, TAU) else { return Some(false) };
                let Some(st) = self.stroke(p) else { return Some(false) };
                done(self.put_own(tgt, st, &sh))
            }
            d::Kind::Arc { center, radius, start, end } => {
                let s = rad(*start);
                let mut sweep = rad(*end) - s;
                sweep = rem(sweep, TAU);
                if sweep < 1e-12 {
                    sweep = TAU;
                }
                let Some(sh) = arc_shape(o, *center, *radius, s, sweep) else { return Some(false) };
                let Some(st) = self.stroke(p) else { return Some(false) };
                done(self.put_own(tgt, st, &sh))
            }
            d::Kind::Ellipse { center, major, ratio, start, end } => {
                let n = e.common.extrusion;
                let nl = sqrt(n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).max(1e-300);
                let n = [n[0] / nl, n[1] / nl, n[2] / nl];
                // the minor axis: ratio (N x major)
                let minor = [ratio * (n[1] * major[2] - n[2] * major[1]), ratio * (n[2] * major[0] - n[0] * major[2]), ratio * (n[0] * major[1] - n[1] * major[0])];
                let mut sweep = rem(end - start, TAU);
                if sweep < 1e-12 {
                    sweep = TAU;
                }
                let Some(sh) = ellipse_of([center[0], center[1]], [major[0], major[1]], [minor[0], minor[1]], *start, sweep) else { return Some(false) };
                let Some(st) = self.stroke(p) else { return Some(false) };
                done(self.put_own(tgt, st, &sh))
            }
            d::Kind::Spline(sp) => done(self.spline(tgt, p, sp)),
            d::Kind::Polyline(pl) => done(self.polyline(tgt, p, o, pl)),
            // (a polyface without faces, a polygon mesh without its size: nothing to draw)
            d::Kind::Mesh(m) if m.faces.is_empty() && (m.m == 0 || m.n == 0) => None,
            d::Kind::Mesh(m) => done(self.mesh(tgt, p, m)),
            d::Kind::Text(t) => {
                if kind == "ATTDEF" && in_block && t.attr_flags & 2 == 0 {
                    // (an ATTDEF in a block shows through its ATTRIBs; a constant one is drawn with the block)
                    return None;
                }
                if t.attr_flags & 1 != 0 {
                    return None;
                }
                crate::dxftext::text(self, tgt, p, o, t, kind == "ATTDEF" && !in_block)
            }
            d::Kind::MText(m) => crate::dxftext::mtext(self, tgt, p, o, m),
            d::Kind::Insert(ins) => done(self.insert(tgt, p, o, ins, names)),
            d::Kind::BlockRef(r) => {
                let Some(n) = r.block else { return Some(false) };
                let Some(id) = self.block_id(n) else { return Some(false) };
                let base = self.blocks.get(&n.0).map_or([0.0, 0.0], |b| b.base);
                let mut ins = if kind == "ACAD_TABLE" { Insert::new(id, [r.at[0], r.at[1]]) } else { Insert::new(id, base) };
                if let Some(x) = r.x_dir {
                    ins.rotation = atan2(x[1], x[0]);
                }
                if !o.conformal() || o.det() < 0.0 {
                    // (a dimension with its own OCS: its block's geometry is already in it)
                    self.approx = true;
                }
                let Some(st) = self.stroke(p) else { return Some(false) };
                let ok = self.put(tgt, st, &Shape::Insert(ins));
                if ok {
                    self.insert_bounds(n.0, &ins);
                }
                done(ok)
            }
            d::Kind::Hatch(h) => crate::dxftext::hatch(self, tgt, p, o, h),
            d::Kind::Quad(q) => {
                let pts: Vec<P> = if q.face {
                    q.points.iter().map(|v| [v[0], v[1]]).collect()
                } else {
                    // SOLID and TRACE: 1, 2, 4, 3 in OCS
                    [q.points[0], q.points[1], q.points[3], q.points[2]].iter().map(|v| o.p(*v)).collect()
                };
                let mut ring: Vec<P> = Vec::new();
                for v in pts {
                    if ring.last() != Some(&v) && fine_p(v) {
                        ring.push(v);
                    }
                }
                if ring.len() > 1 && ring.first() == ring.last() {
                    ring.pop();
                }
                if ring.len() < 3 {
                    // a line or a point: drawn as a stroke
                    if ring.len() == 2 {
                        let Some(st) = self.stroke(p) else { return Some(false) };
                        return done(self.put_own(tgt, st, &Own::Line(ring[0], ring[1])));
                    }
                    let Some(q) = ring.first().copied() else { return Some(false) };
                    let Some(st) = self.point_style(p) else { return Some(false) };
                    return done(self.put_own(tgt, st, &Own::Line(q, q)));
                }
                let Some(st) = self.fill_style(p) else { return Some(false) };
                done(self.put_fill(tgt, st, &[vec![Own::Poly(ring, true)]]))
            }
            d::Kind::Leader(l) => done(self.leader(tgt, p, l)),
            d::Kind::Wipeout(w) => {
                let mut v: Vec<[f64; 2]> = w.clip.clone();
                if w.clip_type == 1 && v.len() == 2 {
                    v = vec![v[0], [v[1][0], v[0][1]], v[1], [v[0][0], v[1][1]]];
                }
                if v.len() < 3 {
                    // no boundary: the whole image
                    v = vec![[-0.5, -0.5], [w.size[0] - 0.5, -0.5], [w.size[0] - 0.5, w.size[1] - 0.5], [-0.5, w.size[1] - 0.5]];
                }
                // pixel coordinates (the top left corner at -0.5, -0.5, y down) into the drawing
                let ring: Vec<P> = v.iter().map(|c| {
                    let (x, y) = (c[0] + 0.5, w.size[1] - 0.5 - c[1]);
                    [w.at[0] + x * w.u[0] + y * w.v[0], w.at[1] + x * w.u[1] + y * w.v[1]]
                }).collect();
                if !ring.iter().all(|q| fine_p(*q)) {
                    return Some(false);
                }
                let mut st = Style::new(self.o.background | 0xFF00_0000);
                st.transparency = Transparency::Percent(0);
                let Some(st) = self.style_id(&st) else { return Some(false) };
                done(self.put_fill(tgt, st, &[vec![Own::Poly(ring, true)]]))
            }
            d::Kind::Ray { at, dir, both } => {
                let l = len([dir[0], dir[1]]);
                if !(l > 0.0) {
                    return Some(false);
                }
                // a long line (1e7 units each way): render2d draws lines of that length exactly
                let u = [dir[0] / l * 1e7, dir[1] / l * 1e7];
                let a = if *both { [at[0] - u[0], at[1] - u[1]] } else { [at[0], at[1]] };
                let b = [at[0] + u[0], at[1] + u[1]];
                if !fine_p(a) || !fine_p(b) {
                    return Some(false);
                }
                let Some(st) = self.stroke(p) else { return Some(false) };
                done(self.put(tgt, st, &Shape::Line { a, b }))
            }
            _ => Some(false),
        }
    }

    fn point(&mut self, tgt: Tgt, p: &Props, at: P, angle: f64) -> Option<bool> {
        if !fine_p(at) {
            return Some(false);
        }
        let mode = self.h.pdmode;
        let dot = self.point_style(p)?;
        if !(0..=100).contains(&mode) || mode == 0 {
            return Some(self.put_own(tgt, dot, &Own::Line(at, at)));
        }
        if mode & 7 == 1 && mode & (32 | 64) == 0 {
            return None;
        }
        // a symbol: its size absolute, or a share of the drawing's height (negative or 0: 5 %)
        let ext_h = match (self.h.extmin, self.h.extmax) {
            (Some(a), Some(b)) if b[1] > a[1] && (b[1] - a[1]).is_finite() && b[1] - a[1] < 1e20 => b[1] - a[1],
            _ => 1.0,
        };
        let size = if self.h.pdsize > 0.0 { self.h.pdsize } else if self.h.pdsize < 0.0 { -self.h.pdsize / 100.0 * ext_h } else { 0.05 * ext_h };
        let r = size / 2.0;
        let st = self.stroke(p)?;
        let (sn, cs) = sincos(rad(angle));
        let rot = |x: f64, y: f64| [at[0] + x * cs - y * sn, at[1] + x * sn + y * cs];
        let mut ok = false;
        match mode & 7 {
            0 => ok |= self.put_own(tgt, dot, &Own::Line(at, at)),
            2 => {
                ok |= self.put_own(tgt, st, &Own::Line(rot(-r, 0.0), rot(r, 0.0)));
                ok |= self.put_own(tgt, st, &Own::Line(rot(0.0, -r), rot(0.0, r)));
            }
            3 => {
                ok |= self.put_own(tgt, st, &Own::Line(rot(-r, -r), rot(r, r)));
                ok |= self.put_own(tgt, st, &Own::Line(rot(-r, r), rot(r, -r)));
            }
            4 => ok |= self.put_own(tgt, st, &Own::Line(at, rot(0.0, r))),
            _ => {}
        }
        if mode & 32 != 0 {
            ok |= self.put_own(tgt, st, &Own::Circle(at, r));
        }
        if mode & 64 != 0 {
            ok |= self.put_own(tgt, st, &Own::Poly(vec![rot(-r, -r), rot(r, -r), rot(r, r), rot(-r, r)], true));
        }
        Some(ok)
    }

    fn spline(&mut self, tgt: Tgt, p: &Props, sp: &d::Spline) -> bool {
        let Some(st) = self.stroke(p) else { return false };
        let ctrl: Vec<P> = sp.control.iter().map(|c| [c[0], c[1]]).collect();
        if ctrl.iter().any(|c| !fine_p(*c)) {
            return false;
        }
        let deg = sp.degree as usize;
        let n = ctrl.len();
        if n >= 2 && deg >= 1 {
            let weights: Vec<f64> = if sp.weights.len() == n && sp.weights.iter().any(|w| *w != 1.0) && sp.weights.iter().all(|w| *w > 0.0 && fine(*w)) { sp.weights.clone() } else { Vec::new() };
            let knots_ok = sp.knots.len() == n + deg + 1 && sp.knots.windows(2).all(|w| w[0] <= w[1]) && sp.knots.iter().all(|k| fine(*k)) && sp.knots[deg] < sp.knots[n];
            if (1..=3).contains(&deg) && n > deg && n <= 4096 {
                let knots = if knots_ok { sp.knots.clone() } else { Vec::new() };
                if !knots_ok {
                    self.approx = true;
                    self.note(String::from("SPLINE: knots that do not fit its control points: clamped and uniform instead"));
                }
                // closed with knots: the control points close it already; closed without: periodic
                let closed = sp.flags & 1 != 0 && knots.is_empty();
                return self.put_own(tgt, st, &Own::Spline(deg as u32, ctrl, weights, knots, closed));
            }
            // a degree above 3 (or a very long spline): evaluated here into a polyline
            if knots_ok && n > deg {
                self.approx = true;
                self.note(String::from("SPLINE: a degree above 3 or more than 4096 control points: evaluated into a polyline"));
                let pts = deboor_points(deg, &ctrl, &weights, &sp.knots, (n * 8).clamp(64, 65536));
                return pts.len() >= 2 && self.put_own(tgt, st, &Own::Poly(pts, false));
            }
            if n >= 2 && n > deg.min(3) {
                // knots that do not fit and a degree render2d cannot: a clamped uniform spline of degree min(3, n - 1)
                self.approx = true;
                let dg = deg.clamp(1, 3).min(n - 1);
                return self.put_own(tgt, st, &Own::Spline(dg as u32, ctrl, Vec::new(), Vec::new(), false));
            }
        }
        // fit points only: a cubic through them (Catmull-Rom tangents, the file's end tangents where it has them)
        let fit: Vec<P> = sp.fit.iter().map(|c| [c[0], c[1]]).filter(|c| fine_p(*c)).collect();
        if fit.len() >= 2 {
            self.approx = true;
            self.note(String::from("SPLINE of fit points only: a cubic through them (Catmull-Rom tangents), not AutoCAD's fit"));
            let t0 = sp.start_tangent.map(|t| [t[0], t[1]]);
            let t1 = sp.end_tangent.map(|t| [t[0], t[1]]);
            let pts = through(&fit, t0, t1, sp.flags & 1 != 0);
            return self.put_own(tgt, st, &Own::Cubic(pts));
        }
        if ctrl.len() == 1 || fit.len() == 1 {
            return false;
        }
        false
    }

    fn polyline(&mut self, tgt: Tgt, p: &Props, o: &Ocs, pl: &d::Polyline) -> bool {
        let o = if pl.three_d { Ocs::WCS } else { *o };
        let z = pl.elevation;
        let mut v: Vec<&d::Vertex> = Vec::with_capacity(pl.vertices.len());
        for x in &pl.vertices {
            if !(fine(x.at[0]) && fine(x.at[1])) {
                continue;
            }
            // (repeated points without a bulge between them: one)
            if let Some(l) = v.last() {
                if l.at[0] == x.at[0] && l.at[1] == x.at[1] && l.bulge == 0.0 && l.start_width == x.start_width {
                    continue;
                }
            }
            v.push(x);
        }
        let mut closed = pl.closed;
        if closed && v.len() > 1 && v[0].at[..2] == v[v.len() - 1].at[..2] && v[v.len() - 1].bulge == 0.0 {
            v.pop();
            closed = true;
        }
        let n = v.len();
        if n == 0 {
            return false;
        }
        let pt = |x: &d::Vertex| if pl.three_d { [x.at[0], x.at[1]] } else { o.p([x.at[0], x.at[1], z]) };
        if n == 1 {
            let Some(st) = self.point_style(p) else { return false };
            let q = pt(v[0]);
            return self.put_own(tgt, st, &Own::Line(q, q));
        }
        let segs = if closed { n } else { n - 1 };
        let bulged = (0..segs).any(|i| v[i].bulge.abs() > 1e-10 && v[i].bulge.is_finite());
        let widths: Vec<(f64, f64)> = (0..segs).map(|i| (v[i].start_width.max(0.0), v[i].end_width.max(0.0))).collect();
        let wide = widths.iter().any(|w| w.0 > 0.0 || w.1 > 0.0);
        let constant = wide && widths.iter().all(|w| w.0 == widths[0].0 && w.1 == widths[0].0);
        // a mirrored OCS turns a bulge's arc the other way
        let flip = if o.det() < 0.0 { -1.0 } else { 1.0 };
        if wide && !constant {
            return self.tapered(tgt, p, &o, &v, closed, z);
        }
        if constant {
            // a constant width: a stroke in drawing units (bulged: as one spline, so its joins stay)
            let Some(st) = self.wide_stroke(p, widths[0].0) else { return false };
            if !bulged {
                let pts: Vec<P> = v.iter().map(|x| pt(x)).collect();
                return self.put_own(tgt, st, &Own::Poly(pts, closed));
            }
            let vv: Vec<(d::P3, f64)> = v.iter().map(|x| ([x.at[0], x.at[1], z], x.bulge)).collect();
            let Some(sp) = bulged_spline(&o, &vv, closed) else { return false };
            return self.put_own(tgt, st, &sp);
        }
        let Some(st) = self.stroke(p) else { return false };
        let dashed = p.dash.is_some() || p.by_block & BY_BLOCK_DASH != 0;
        if !bulged && (!dashed || pl.plinegen) {
            let pts: Vec<P> = v.iter().map(|x| pt(x)).collect();
            return self.put_own(tgt, st, &Own::Poly(pts, closed));
        }
        if bulged && ((dashed && pl.plinegen) || !o.conformal()) {
            // the pattern runs on across the vertices (PLINEGEN), or the arcs are elliptic: one spline
            let vv: Vec<(d::P3, f64)> = v.iter().map(|x| ([x.at[0], x.at[1], z], x.bulge)).collect();
            let Some(sp) = bulged_spline(&o, &vv, closed) else { return false };
            return self.put_own(tgt, st, &sp);
        }
        // runs of straight segments as polylines, arcs as arcs; a dashed polyline without PLINEGEN: every segment for
        // itself (AutoCAD starts the pattern anew at each vertex)
        let mut ok = false;
        let mut run: Vec<P> = Vec::new();
        fn flush(m: &mut Map, tgt: Tgt, st: StyleId, run: &mut Vec<P>, ok: &mut bool) {
            if run.len() >= 2 {
                let pts = core::mem::take(run);
                *ok |= m.put_own(tgt, st, &if pts.len() == 2 { Own::Line(pts[0], pts[1]) } else { Own::Poly(pts, false) });
            }
            run.clear();
        }
        for i in 0..segs {
            let (a, b) = (v[i], v[(i + 1) % n]);
            let (pa, pb) = (pt(a), pt(b));
            match bulge_of(pa, pb, a.bulge * flip) {
                Some((c, r, s, theta)) => {
                    flush(self, tgt, st, &mut run, &mut ok);
                    ok |= self.put_own(tgt, st, &Own::Arc(c, r, s, theta));
                }
                None => {
                    if dashed {
                        ok |= self.put_own(tgt, st, &Own::Line(pa, pb));
                    } else {
                        if run.is_empty() {
                            run.push(pa);
                        }
                        run.push(pb);
                    }
                }
            }
        }
        flush(self, tgt, st, &mut run, &mut ok);
        ok
    }

    /// a polyline whose widths change: each segment a fill (straight: a trapezoid; an arc: its band, sampled)
    fn tapered(&mut self, tgt: Tgt, p: &Props, o: &Ocs, v: &[&d::Vertex], closed: bool, z: f64) -> bool {
        self.approx = true;
        self.note(String::from("polylines of changing width: each segment a fill of its own, joined without the outer wedge"));
        let Some(st) = self.fill_style(p) else { return false };
        let Some(hair) = self.stroke(p) else { return false };
        let n = v.len();
        let segs = if closed { n } else { n - 1 };
        let mut ok = false;
        for i in 0..segs {
            let (a, b) = (v[i], v[(i + 1) % n]);
            let (pa, pb) = ([a.at[0], a.at[1]], [b.at[0], b.at[1]]);
            let (w0, w1) = (a.start_width.max(0.0), a.end_width.max(0.0));
            if w0 == 0.0 && w1 == 0.0 {
                let (qa, qb) = (o.p([pa[0], pa[1], z]), o.p([pb[0], pb[1], z]));
                ok |= match bulge_of(pa, pb, a.bulge) {
                    Some(_) => {
                        let vv = [([pa[0], pa[1], z], a.bulge), ([pb[0], pb[1], z], 0.0)];
                        bulged_spline(o, &vv, false).is_some_and(|s| self.put_own(tgt, hair, &s))
                    }
                    None => self.put_own(tgt, hair, &Own::Line(qa, qb)),
                };
                continue;
            }
            // the segment's two edges in OCS, sampled along an arc
            let samples: Vec<(P, P)> = match bulge_of(pa, pb, a.bulge) {
                Some((c, r, s, theta)) => {
                    let k = ((theta.abs() / (PI / 32.0)) as usize + 1).min(256);
                    (0..=k).map(|j| {
                        let f = j as f64 / k as f64;
                        let t = s + theta * f;
                        let (sn, cs) = sincos(t);
                        let w = (w0 + (w1 - w0) * f) / 2.0;
                        let (ro, ri) = (r + w, (r - w).max(0.0));
                        ([c[0] + ro * cs, c[1] + ro * sn], [c[0] + ri * cs, c[1] + ri * sn])
                    }).collect()
                }
                None => {
                    let dvec = [pb[0] - pa[0], pb[1] - pa[1]];
                    let l = len(dvec);
                    if !(l > 0.0) {
                        continue;
                    }
                    let nrm = [-dvec[1] / l, dvec[0] / l];
                    let e = |q: P, w: f64| ([q[0] + nrm[0] * w / 2.0, q[1] + nrm[1] * w / 2.0], [q[0] - nrm[0] * w / 2.0, q[1] - nrm[1] * w / 2.0]);
                    vec![e(pa, w0), e(pb, w1)]
                }
            };
            let mut ring: Vec<P> = samples.iter().map(|s| o.p([s.0[0], s.0[1], z])).collect();
            ring.extend(samples.iter().rev().map(|s| o.p([s.1[0], s.1[1], z])));
            ring.dedup();
            if ring.len() >= 3 && ring.iter().all(|q| fine_p(*q)) {
                ok |= self.put_fill(tgt, st, &[vec![Own::Poly(ring, true)]]);
            }
        }
        ok
    }

    fn mesh(&mut self, tgt: Tgt, p: &Props, m: &d::Mesh) -> bool {
        let Some(st) = self.stroke(p) else { return false };
        let pt = |i: usize| m.vertices.get(i).map(|v| [v[0], v[1]]);
        let mut ok = false;
        if !m.faces.is_empty() {
            // a polyface: each face's visible edges (an index below 0: its edge is invisible)
            for f in &m.faces {
                let idx: Vec<i32> = f.iter().copied().filter(|&i| i != 0).collect();
                for k in 0..idx.len() {
                    if idx[k] < 0 {
                        continue;
                    }
                    let (a, b) = (pt(idx[k].unsigned_abs() as usize - 1), pt(idx[(k + 1) % idx.len()].unsigned_abs() as usize - 1));
                    if let (Some(a), Some(b)) = (a, b) {
                        if fine_p(a) && fine_p(b) && a != b {
                            ok |= self.put_own(tgt, st, &Own::Line(a, b));
                        }
                    }
                }
            }
            return ok;
        }
        // a polygon mesh: its m by n grid
        let (mm, nn) = (m.m as usize, m.n as usize);
        if mm == 0 || nn == 0 || mm.checked_mul(nn).is_none_or(|k| k > m.vertices.len()) {
            return false;
        }
        for i in 0..mm {
            let row: Vec<P> = (0..nn).filter_map(|j| pt(i * nn + j)).collect();
            ok |= row.len() >= 2 && self.put_own(tgt, st, &Own::Poly(row, m.closed_n));
        }
        for j in 0..nn {
            let col: Vec<P> = (0..mm).filter_map(|i| pt(i * nn + j)).collect();
            ok |= col.len() >= 2 && self.put_own(tgt, st, &Own::Poly(col, m.closed_m));
        }
        ok
    }

    fn leader(&mut self, tgt: Tgt, p: &Props, l: &d::Leader) -> bool {
        let pts: Vec<P> = l.vertices.iter().map(|v| [v[0], v[1]]).filter(|q| fine_p(*q)).collect();
        if pts.len() < 2 {
            return false;
        }
        let Some(st) = self.stroke(p) else { return false };
        let mut ok = if l.spline && pts.len() > 2 {
            self.approx = true;
            let c = through(&pts, None, None, false);
            self.put_own(tgt, st, &Own::Cubic(c))
        } else {
            self.put_own(tgt, st, &Own::Poly(pts.clone(), false))
        };
        if l.arrow {
            let (size, scale) = match l.dimstyle.and_then(|n| self.t.dimstyle(n)) {
                Some(ds) => (ds.arrow_size, ds.scale),
                None => (self.h.dimasz, self.h.dimscale),
            };
            let size = size * scale;
            let (a, b) = (pts[0], pts[1]);
            let dv = [b[0] - a[0], b[1] - a[1]];
            let dl = len(dv);
            if size > 0.0 && dl > 0.0 && fine(size) {
                // AutoCAD's closed filled arrow: a third of its length wide
                let u = [dv[0] / dl * size, dv[1] / dl * size];
                let nrm = [-u[1] / 6.0, u[0] / 6.0];
                let base = [a[0] + u[0], a[1] + u[1]];
                let tri = vec![a, [base[0] + nrm[0], base[1] + nrm[1]], [base[0] - nrm[0], base[1] - nrm[1]]];
                if let Some(fs) = self.fill_style(p) {
                    ok |= self.put_fill(tgt, fs, &[vec![Own::Poly(tri, true)]]);
                }
            }
        }
        ok
    }

    fn insert_bounds(&mut self, name: u32, ins: &Insert) {
        let Some(b) = self.blocks.get(&name).and_then(|b| b.bounds.map(|x| (x, b.base))) else { return };
        let (bb, base) = b;
        let (sn, cs) = sincos(ins.rotation);
        let mut out: Option<[f64; 4]> = None;
        let (cols, rows) = (ins.columns.max(1) as f64, ins.rows.max(1) as f64);
        for corner in [[bb[0], bb[1]], [bb[2], bb[1]], [bb[2], bb[3]], [bb[0], bb[3]]] {
            let x = (corner[0] - base[0]) * ins.scale[0];
            let y = (corner[1] - base[1]) * ins.scale[1];
            let q = [ins.at[0] + x * cs - y * sn, ins.at[1] + x * sn + y * cs];
            for (i, j) in [(0.0, 0.0), (cols - 1.0, 0.0), (0.0, rows - 1.0), (cols - 1.0, rows - 1.0)] {
                let ox = i * ins.spacing[0];
                let oy = j * ins.spacing[1];
                let r = [q[0] + ox * cs - oy * sn, q[1] + ox * sn + oy * cs];
                grow(&mut out, [r[0], r[1], r[0], r[1]]);
            }
        }
        if let Some(o) = out {
            self.grow_by(o);
        }
    }

    fn insert(&mut self, tgt: Tgt, p: &Props, o: &Ocs, ins: &d::Insert, names: &d::Names) -> bool {
        let Some(id) = self.block_id(ins.block) else { return false };
        let at = o.p(ins.at);
        if !fine_p(at) {
            return false;
        }
        // the insert's transform through the OCS: o.lin R(rotation) diag(sx, sy) as R(rot') diag(sx', sy')
        let r = rad(ins.rotation);
        let (sn, cs) = sincos(r);
        let col0 = o.v(cs * ins.scale[0], sn * ins.scale[0]);
        let col1 = o.v(-sn * ins.scale[1], cs * ins.scale[1]);
        let l0 = len(col0);
        if !(l0 > 0.0) || !fine(l0) {
            return false;
        }
        let rot = atan2(col0[1], col0[0]);
        let u0 = [col0[0] / l0, col0[1] / l0];
        let perp = [-u0[1], u0[0]];
        let sy = col1[0] * perp[0] + col1[1] * perp[1];
        let shear = col1[0] * u0[0] + col1[1] * u0[1];
        if shear.abs() > 1e-9 * l0.max(sy.abs()) {
            self.approx = true;
            self.note(String::from("INSERT under a tilted OCS: the shear of its transform left out"));
        }
        let mut x = Insert::new(id, at);
        x.rotation = rot;
        x.scale = [l0, sy];
        x.columns = ins.columns.max(1);
        x.rows = ins.rows.max(1);
        if x.columns > 1 || x.rows > 1 {
            // the array's steps along the insert's turned axes, through the OCS
            let ex = o.v(cs, sn);
            let ey = o.v(-sn, cs);
            let (rs, rc) = sincos(rot);
            let rx = [rc, rs];
            x.spacing = [ins.spacing[0] * (ex[0] * rx[0] + ex[1] * rx[1]), ins.spacing[1] * (ey[0] * -rx[1] + ey[1] * rx[0])];
        }
        let Some(st) = self.stroke(p) else { return false };
        let mut ok = self.put(tgt, st, &Shape::Insert(x));
        if ok {
            self.insert_bounds(ins.block.0, &x);
        } else {
            self.note(String::from("INSERT refused by render2d (a block inside itself, deeper than BLOCK_DEPTH, a scale of 0)"));
        }
        // its attributes: texts of their own (on their layers in model space, in the block that holds the insert)
        let in_block = matches!(tgt, Tgt::Block(_));
        for a in &ins.attribs {
            let d::Kind::Text(t) = &a.kind else { continue };
            self.count("ATTRIB").read += 1;
            if a.common.invisible || t.attr_flags & 1 != 0 {
                self.count("ATTRIB").hidden += 1;
                continue;
            }
            let at_tgt = match tgt {
                Tgt::Block(_) => tgt,
                Tgt::Layer(_) => match self.run_for(a.common.layer, names) {
                    Some(l) => Tgt::Layer(l),
                    None => continue,
                },
            };
            let ap = self.props(&a.common, in_block, names);
            let ao = Ocs::of(a.common.extrusion);
            if crate::dxftext::text(self, at_tgt, &ap, &ao, t, false) == Some(true) {
                ok = true;
                self.count("ATTRIB").drawn += 1;
            }
        }
        ok
    }
}

/// a transparency's alpha (255 opaque) as AutoCAD's percent
fn alpha_percent(a: u8) -> u32 {
    (((255 - a as u32) * 100 + 127) / 255).min(100)
}

/// cubic Bézier pieces through points: Catmull-Rom tangents, given end tangents (directions) where there are
pub(crate) fn through(p: &[P], t0: Option<P>, t1: Option<P>, closed: bool) -> Vec<P> {
    let n = p.len();
    let mut out = vec![p[0]];
    let seg = |i: usize| -> P { [p[(i + 1) % n][0] - p[i % n][0], p[(i + 1) % n][1] - p[i % n][1]] };
    let tan = |i: usize| -> P {
        let (prev, next) = if closed { (p[(i + n - 1) % n], p[(i + 1) % n]) } else { (p[i.saturating_sub(1)], p[(i + 1).min(n - 1)]) };
        let mut t = [(next[0] - prev[0]) / 2.0, (next[1] - prev[1]) / 2.0];
        if !closed && (i == 0 || i == n - 1) {
            let given = if i == 0 { t0 } else { t1 };
            let chord = if i == 0 { seg(0) } else { seg(n - 2) };
            let cl = len(chord);
            if let Some(g) = given {
                let gl = len(g);
                if gl > 0.0 {
                    t = [g[0] / gl * cl, g[1] / gl * cl];
                }
            } else {
                t = chord;
            }
        }
        t
    };
    let segs = if closed { n } else { n - 1 };
    for i in 0..segs {
        let (a, b) = (p[i], p[(i + 1) % n]);
        let (ta, tb) = (tan(i), tan((i + 1) % n));
        out.push([a[0] + ta[0] / 3.0, a[1] + ta[1] / 3.0]);
        out.push([b[0] - tb[0] / 3.0, b[1] - tb[1] / 3.0]);
        out.push(b);
    }
    out
}

/// a B-spline of any degree evaluated at `count` parameters (de Boor), rational with weights
fn deboor_points(deg: usize, ctrl: &[P], weights: &[f64], knots: &[f64], count: usize) -> Vec<P> {
    let n = ctrl.len();
    let (lo, hi) = (knots[deg], knots[n]);
    let w = |k: usize| if weights.len() == n { weights[k] } else { 1.0 };
    let mut out = Vec::with_capacity(count + 1);
    let mut d: Vec<[f64; 3]> = vec![[0.0; 3]; deg + 1];
    for s in 0..=count {
        let t = lo + (hi - lo) * s as f64 / count as f64;
        // the span: the last k with knots[k] <= t < knots[k + 1], within deg..n-1
        let mut k = deg;
        while k + 1 < n && knots[k + 1] <= t {
            k += 1;
        }
        for j in 0..=deg {
            let i = k + j - deg;
            let wi = w(i);
            d[j] = [ctrl[i][0] * wi, ctrl[i][1] * wi, wi];
        }
        for r in 1..=deg {
            for j in (r..=deg).rev() {
                let i = k + j - deg;
                let den = knots[i + deg + 1 - r] - knots[i];
                let a = if den > 0.0 { (t - knots[i]) / den } else { 0.0 };
                for c in 0..3 {
                    d[j][c] = (1.0 - a) * d[j - 1][c] + a * d[j][c];
                }
            }
        }
        let q = d[deg];
        if q[2] != 0.0 {
            let pt = [q[0] / q[2], q[1] / q[2]];
            if fine_p(pt) {
                out.push(pt);
            }
        }
    }
    out
}

impl d::Sink for Map<'_, '_> {
    fn header(&mut self, h: &d::Header, _: &d::Names) {
        self.h = h.clone();
        self.out.header = h.clone();
        self.lw_on = self.o.lineweights.unwrap_or(h.lwdisplay.unwrap_or(false));
    }

    fn tables(&mut self, t: &d::Tables, names: &d::Names) {
        self.t = t.clone();
        self.standard = names.find("STANDARD");
        self.layer_ix.clear();
        for (k, l) in self.t.layers.iter().enumerate() {
            self.layer_ix.insert(l.name.0, k);
        }
    }

    fn block(&mut self, b: d::Block, names: &d::Names) {
        if b.paper {
            for e in &b.entities {
                let k = names.get(e.common.kind).to_ascii_uppercase();
                let c = self.count(&k);
                c.read += 1;
                c.paper += 1;
            }
            return;
        }
        let Some(id) = self.block_id(b.name) else { return };
        let base = [b.base[0], b.base[1]];
        if let Some(bl) = self.blocks.get_mut(&b.name.0) {
            if bl.defined {
                self.out.note(format!("the block {} is defined twice: the second definition joins the first", names.get(b.name)));
            }
            bl.defined = true;
            bl.base = base;
        }
        let mut blk = Block::new();
        blk.base = base;
        if !fine_p(base) || self.s.set_block(id, &blk).is_err() {
            self.out.note(String::from("a block's base point refused"));
        }
        if b.flags & 4 != 0 && b.entities.is_empty() {
            self.out.note(format!("the external reference {} ({}) is not loaded", names.get(b.name), b.xref));
        }
        self.cur_block = Some(b.name.0);
        for e in &b.entities {
            self.map_entity(e, names, Some(id));
        }
        self.cur_block = None;
    }

    fn entity(&mut self, e: d::Entity, names: &d::Names) {
        if e.common.paper {
            let k = names.get(e.common.kind).to_ascii_uppercase();
            let c = self.count(&k);
            c.read += 1;
            c.paper += 1;
            return;
        }
        self.map_entity(&e, names, None);
    }
}
