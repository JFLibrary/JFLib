// pick -- a scene's spatial index, picking, area selection and object snaps on the CPU in f64 (G6d, decisions 24 and 30;
// docs/RENDER2D.md "Auswahl und Fang").  A child of scene.rs: it reads the scene's entities, layers and blocks.
//
// The index: an R-tree (index.rs) of the entities on layers by their boxes (the centre line's, grown by how far a width in
// drawing units reaches), and one per block of its entities in its own units.  An insert sits in the scene's tree by its
// copies' boxes (its block's box through each transform, an array's corner copies), never once per entity of its block:
// a query that meets it goes down into the block through the inverse transform (nested inserts likewise).  Texts in pixels
// and inserts of blocks that hold them have no fixed box in the drawing: they are looked at by every query.  The tree is
// built in one go by the first query (or build_index) and then kept in step: every change of the scene notes what it
// touched, the next query applies it (a change of a block: its tree again, and its inserts' boxes when its box moved).
//
// What a query decides is exact against the geometry in f64: a stroke's drawn region (its width, at least a pixel; caps,
// joins, miters up to their limit; a dash pattern's gaps count as the stroke, as AutoCAD picks them), arcs and curves
// themselves (not their flattening), a fill's inside (even-odd, holes) and a hatched one's, a text's quad.  Hits are in the
// painter's order, topmost first.  Hidden layers are skipped; a filter can skip more (a layer lock of an app).

use alloc::vec;
use alloc::vec::Vec;

use super::{Cap, EntityId, Geom, Insert, Join, LayerId, Scene, Style, TextSrc, Unit, BY_BLOCK_WIDTH, NONE};
use crate::block::Xf;
use crate::curve::{Curve, Span};
use crate::geo::{self, add, cross, dot, len, mul, perp, sub, unit, Area, Bx, Prim, P};
use crate::index::RTree;
use crate::math::{sincos, sqrt};

const TAU: f64 = core::f64::consts::TAU;

/// where a pick or a snap found something: the entity on a layer, its layer and, inside an insert, the way down to the
/// entity
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub entity: EntityId,
    pub layer: LayerId,
    /// inside an insert, one step per level: the copy of the insert above (row * columns + column; 0 for a single insert)
    /// and the entity of its block the way goes on with (a nested insert, ..., last the entity itself).  Empty for an
    /// entity on a layer
    pub path: Vec<(u32, EntityId)>,
}

/// a hit of Scene::pick
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub target: Target,
    /// pixels from the point to what is drawn of it (0: on it)
    pub distance: f64,
}

/// an area selection's region in drawing units (Scene::select)
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub enum Region<'a> {
    /// two opposite corners of an axis-aligned rectangle
    Rect([f64; 2], [f64; 2]),
    /// a closed polygon (even-odd; AutoCAD's WPolygon and CPolygon)
    Polygon(&'a [[f64; 2]]),
}

/// AutoCAD's two area selections
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SelectMode {
    /// what lies wholly inside
    Window,
    /// what lies inside or touches the boundary
    Crossing,
}

/// the kinds of object snap (decision 30, and AutoCAD's insertion point)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SnapKind {
    /// a line's or arc's ends, a polyline's and a Bézier chain's vertices, a fill's corners
    Endpoint,
    /// the middle of a line, of each segment of a polyline, of an arc, of each Bézier piece and of an open ellipse or spline
    /// (by length)
    Midpoint,
    /// a circle's, arc's or ellipse's centre (looked for when the pointer is over the curve)
    Center,
    /// the points of a circle, arc or ellipse on its own axes (an ellipse made of a circle by an uneven insert: its axes'
    /// ends)
    Quadrant,
    /// where two pieces near the pointer cross (also across inserts, fills' edges, a polyline with itself)
    Intersection,
    /// the foot of the perpendicular from the base point (SnapOptions::base)
    Perpendicular,
    /// where a line from the base point touches a circle, arc or curve
    Tangent,
    /// the nearest point of what is near the pointer
    Nearest,
    /// a text's alignment point, an insert's insertion point (each copy of an array)
    Insertion,
}

pub const SNAP_END: u32 = 1;
pub const SNAP_MID: u32 = 2;
pub const SNAP_CEN: u32 = 4;
pub const SNAP_QUA: u32 = 8;
pub const SNAP_INT: u32 = 16;
pub const SNAP_PER: u32 = 32;
pub const SNAP_TAN: u32 = 64;
pub const SNAP_NEA: u32 = 128;
pub const SNAP_INS: u32 = 256;
pub const SNAP_ALL: u32 = 511;

impl SnapKind {
    /// its bit of SnapOptions::kinds
    pub const fn bit(self) -> u32 {
        match self {
            SnapKind::Endpoint => SNAP_END,
            SnapKind::Midpoint => SNAP_MID,
            SnapKind::Center => SNAP_CEN,
            SnapKind::Quadrant => SNAP_QUA,
            SnapKind::Intersection => SNAP_INT,
            SnapKind::Perpendicular => SNAP_PER,
            SnapKind::Tangent => SNAP_TAN,
            SnapKind::Nearest => SNAP_NEA,
            SnapKind::Insertion => SNAP_INS,
        }
    }

    /// its rank among kinds equally near the pointer (lower first): endpoint, intersection, midpoint, centre, quadrant,
    /// insertion, perpendicular, tangent; nearest only when nothing else is there
    pub const fn rank(self) -> u32 {
        match self {
            SnapKind::Endpoint => 0,
            SnapKind::Intersection => 1,
            SnapKind::Midpoint => 2,
            SnapKind::Center => 3,
            SnapKind::Quadrant => 4,
            SnapKind::Insertion => 5,
            SnapKind::Perpendicular => 6,
            SnapKind::Tangent => 7,
            SnapKind::Nearest => 8,
        }
    }
}

/// what Scene::snap looks for.  Made with `new` (it may grow)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapOptions {
    /// the aperture's radius in pixels: what lies within it of the pointer
    pub aperture: f64,
    /// the kinds looked for (SNAP_* bits)
    pub kinds: u32,
    /// the command's last point: perpendicular and tangent need it (None: they are not looked for)
    pub base: Option<[f64; 2]>,
}

impl SnapOptions {
    pub const fn new(aperture: f64, kinds: u32) -> SnapOptions {
        SnapOptions { aperture, kinds, base: None }
    }

    /// the same with a base point for perpendicular and tangent
    pub const fn with_base(mut self, base: [f64; 2]) -> SnapOptions {
        self.base = Some(base);
        self
    }
}

/// an object snap (Scene::snap): its kind, the point (drawing units, f64), how far from the pointer (px), what it lies on
/// (an intersection: both)
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct Snap {
    pub kind: SnapKind,
    pub point: [f64; 2],
    pub distance: f64,
    pub target: Target,
    pub other: Option<Target>,
}

/// what the spatial index holds (Scene::index_info).  It may grow
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IndexInfo {
    /// entities on layers in the scene's tree, its nodes, levels, bytes
    pub entries: u32,
    pub nodes: u32,
    pub height: u32,
    pub bytes: u64,
    /// blocks with a tree, their entries and bytes
    pub block_trees: u32,
    pub block_entries: u32,
    pub block_bytes: u64,
    /// entities looked at by every query (texts in pixels, inserts of blocks holding them)
    pub outside: u32,
    /// what the last sync applied: changes of entities on layers, blocks made again; the scene's tree made anew
    pub synced: u32,
    pub blocks_synced: u32,
    pub rebuilt: bool,
}

// ---------------------------------------------------------------- the index

#[derive(Clone, Default)]
pub(super) struct BlockIdx {
    pub(super) tree: RTree,
    /// its geometry's box in its units (relative to its base); None: nothing drawn
    pub(super) bbox: Option<Bx>,
    /// how far its widths in units reach beyond its geometry (its units: nested inserts' scaled)
    pub(super) reach: f64,
    /// it holds a text in pixels (also below): its inserts are looked at by every query
    pub(super) px: bool,
    dirty: bool,
    built: bool,
}

pub(crate) struct Index {
    pub(super) top: RTree,
    /// entity slots changed since the last sync
    pending: Vec<u32>,
    /// everything anew with the next sync
    all: bool,
    pub(super) blocks: Vec<BlockIdx>,
    /// entities on layers looked at by every query
    pub(super) px: Vec<u32>,
    info: IndexInfo,
}

/// how far a stroke of this style reaches beyond its centre line, in half widths: a miter up to its limit, a square cap's
/// corner
fn bound(st: &Style) -> f64 {
    let b = core::f64::consts::SQRT_2;
    if st.join == Join::Miter { b.max(st.miter_limit as f64) } else { b }
}

/// a style's reach in drawing units (widths in units; 0 for pixels)
fn units_reach(st: &Style) -> f64 {
    if st.unit == Unit::Drawing { 0.5 * st.width as f64 * bound(st) } else { 0.0 }
}

/// a style's reach in pixels (widths in pixels, and a hairline's half pixel for any)
fn px_reach(st: &Style) -> f64 {
    let h = if st.unit == Unit::Px { (0.5 * st.width as f64).max(0.5) } else { 0.5 };
    h * bound(st)
}

/// the box of points less a shift
fn pts_box(p: &[[f64; 2]], shift: P) -> Bx {
    let b = geo::bx_of(p);
    [b[0] - shift[0], b[1] - shift[1], b[2] - shift[0], b[3] - shift[1]]
}

/// a copy's transform of an insert (block::insert_xfs, one copy)
pub(super) fn copy_xf(ins: &Insert, origin: P, col: u32, row: u32) -> Xf {
    let (s, c) = sincos(ins.rotation);
    let m = [[c * ins.scale[0], -s * ins.scale[1]], [s * ins.scale[0], c * ins.scale[1]]];
    let (dx, dy) = (col as f64 * ins.spacing[0], row as f64 * ins.spacing[1]);
    Xf { t: [ins.at[0] - origin[0] + (c * dx - s * dy), ins.at[1] - origin[1] + (s * dx + c * dy)], m }
}

/// a box through a transform: the box of its corners' images
fn xf_box(xf: &Xf, b: &Bx) -> Bx {
    geo::bx_of(&[xf.apply([b[0], b[1]]), xf.apply([b[2], b[1]]), xf.apply([b[0], b[3]]), xf.apply([b[2], b[3]])])
}

/// a box back through a transform (the box of its corners' preimages)
pub(super) fn inv_box(xf: &Xf, b: &Bx) -> Bx {
    let n = xf.inv();
    let back = |p: P| {
        let v = sub(p, xf.t);
        [n[0][0] * v[0] + n[0][1] * v[1], n[1][0] * v[0] + n[1][1] * v[1]]
    };
    geo::bx_of(&[back([b[0], b[1]]), back([b[2], b[1]]), back([b[0], b[3]]), back([b[2], b[3]])])
}

/// an insert's copies' geometry box (its block's box through the corner copies of an array) and their insertion points'
/// box, where it sits (origin: the base of the block it sits in, else none)
fn insert_boxes(ins: &Insert, origin: P, bbox: &Bx) -> (Bx, Bx) {
    let (c1, r1) = (ins.columns.max(1) - 1, ins.rows.max(1) - 1);
    let mut g = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let mut at = g;
    for (c, r) in [(0, 0), (c1, 0), (0, r1), (c1, r1)] {
        let xf = copy_xf(ins, origin, c, r);
        g = geo::bx_union(&g, &xf_box(&xf, bbox));
        at = geo::bx_union(&at, &geo::bx_of(&[xf.t]));
    }
    (g, at)
}

/// the copies (columns, rows; ends included) of an array whose block box (grown by m in the parent's units) may meet q
/// (q in the units the insert sits in, less origin)
pub(super) fn copies_meeting(ins: &Insert, origin: P, bbox: &Bx, q: &Bx, m: f64) -> (u32, u32, u32, u32) {
    let (cols, rows) = (ins.columns.max(1), ins.rows.max(1));
    if cols == 1 && rows == 1 {
        return (0, 0, 0, 0);
    }
    let (s, c) = sincos(ins.rotation);
    let at = sub(ins.at, origin);
    let back = |p: P| {
        let v = sub(p, at);
        [c * v[0] + s * v[1], -s * v[0] + c * v[1]]
    };
    let qq = geo::bx_grow(q, m);
    let qf = geo::bx_of(&[back([qq[0], qq[1]]), back([qq[2], qq[1]]), back([qq[0], qq[3]]), back([qq[2], qq[3]])]);
    let (sx, sy) = (ins.scale[0], ins.scale[1]);
    let bf = [(sx * bbox[0]).min(sx * bbox[2]), (sy * bbox[1]).min(sy * bbox[3]), (sx * bbox[0]).max(sx * bbox[2]), (sy * bbox[1]).max(sy * bbox[3])];
    let range = |lo: f64, hi: f64, b0: f64, b1: f64, sp: f64, n: u32| -> (u32, u32) {
        if n == 1 {
            return (0, 0);
        }
        if sp == 0.0 {
            return if b1 >= lo && b0 <= hi { (0, n - 1) } else { (1, 0) };
        }
        let (a, b) = ((lo - b1) / sp, (hi - b0) / sp);
        let (a, b) = if sp > 0.0 { (a, b) } else { (b, a) };
        let a = (a.fl() - 1.0).max(0.0);
        let b = (b.cl() + 1.0).min((n - 1) as f64);
        if !(a <= b) {
            return (1, 0);
        }
        (a as u32, b as u32)
    };
    let (c0, c1) = range(qf[0], qf[2], bf[0], bf[2], ins.spacing[0], cols);
    let (r0, r1) = range(qf[1], qf[3], bf[1], bf[3], ins.spacing[1], rows);
    (c0, c1, r0, r1)
}

trait Floor {
    fn fl(self) -> f64;
    fn cl(self) -> f64;
}

impl Floor for f64 {
    /// floor without libm
    fn fl(self) -> f64 {
        if !(self.abs() < 4.5e15) {
            return self;
        }
        let t = self as i64 as f64;
        if t > self { t - 1.0 } else { t }
    }

    fn cl(self) -> f64 {
        -(-self).fl()
    }
}

/// a text's insertion point (the alignment point)
fn text_at(src: &TextSrc) -> P {
    match src {
        TextSrc::Single(_, t) => t.at,
        TextSrc::Multi(_, t) => t.at,
    }
}

impl Scene {
    // ------------------------------------------------------------ keeping the index in step (scene.rs calls these)

    /// an entity on a layer changed (added, moved, gone)
    pub(crate) fn idx_top(&mut self, slot: u32) {
        if let Some(ix) = self.idx.as_mut() {
            ix.pending.push(slot);
        }
    }

    /// a block's definition changed
    pub(crate) fn idx_block(&mut self, b: usize) {
        if let Some(ix) = self.idx.as_mut() {
            if b >= ix.blocks.len() {
                ix.blocks.resize(b + 1, BlockIdx::default());
            }
            ix.blocks[b].dirty = true;
        }
    }

    /// everything anew with the next query (a style's reach changed)
    pub(crate) fn idx_all(&mut self) {
        if let Some(ix) = self.idx.as_mut() {
            ix.all = true;
        }
    }

    /// builds the spatial index now (else the first pick, select or snap does)
    pub fn build_index(&mut self) {
        self.sync_index();
    }

    /// what the index holds
    pub fn index_info(&self) -> IndexInfo {
        self.idx.as_ref().map_or(IndexInfo::default(), |ix| ix.info)
    }

    /// (tests) the trees' invariants hold
    pub fn index_check(&mut self) -> Result<u32, &'static str> {
        self.sync_index();
        let ix = self.idx.as_ref().ok_or("no index")?;
        for b in &ix.blocks {
            if b.built {
                b.tree.check()?;
            }
        }
        ix.top.check()
    }

    /// the index brought up to date with the scene
    pub(super) fn sync_index(&mut self) {
        if self.idx.as_ref().is_none_or(|ix| ix.all) {
            self.idx = Some(alloc::boxed::Box::new(Index { top: RTree::new(), pending: Vec::new(), all: false, blocks: Vec::new(), px: Vec::new(), info: IndexInfo::default() }));
            let nb = self.blocks.len();
            if let Some(ix) = self.idx.as_mut() {
                ix.blocks = vec![BlockIdx { dirty: true, ..BlockIdx::default() }; nb];
            }
            let changed = self.sync_blocks();
            let _ = changed;
            self.rebuild_top();
            return;
        }
        let changed = self.sync_blocks();
        let Some(ix) = self.idx.as_mut() else { return };
        ix.info.rebuilt = false;
        let mut pending = core::mem::take(&mut ix.pending);
        if changed.iter().any(|&c| c) {
            // (a block's box or reach moved: its inserts on layers again)
            for (k, e) in self.ents.iter().enumerate() {
                if e.live && e.block == NONE {
                    if let Geom::Insert(ins) = &e.geom {
                        if changed.get(ins.block.slot as usize).copied().unwrap_or(false) {
                            pending.push(k as u32);
                        }
                    }
                }
            }
        }
        let n = self.idx.as_ref().map_or(0, |ix| ix.top.len());
        if pending.len() as u32 > n / 4 + 4096 {
            self.rebuild_top();
            return;
        }
        pending.sort_unstable();
        pending.dedup();
        let count = pending.len() as u32;
        for slot in pending {
            let entry = self.entry_of(slot);
            let Some(ix) = self.idx.as_mut() else { return };
            match entry {
                Entry::Tree(b) => {
                    ix.px.retain(|&x| x != slot);
                    ix.top.update(slot, b);
                }
                Entry::Px => {
                    ix.top.remove(slot);
                    if !ix.px.contains(&slot) {
                        ix.px.push(slot);
                    }
                }
                Entry::None => {
                    ix.top.remove(slot);
                    ix.px.retain(|&x| x != slot);
                }
            }
        }
        if let Some(ix) = self.idx.as_mut() {
            ix.info.synced = count;
            Self::fill_info(ix);
        }
    }

    fn fill_info(ix: &mut Index) {
        let st = ix.top.stats();
        ix.info.entries = ix.top.len();
        ix.info.nodes = st.nodes;
        ix.info.height = st.height;
        ix.info.bytes = st.bytes + ix.px.capacity() as u64 * 4 + ix.blocks.capacity() as u64 * core::mem::size_of::<BlockIdx>() as u64;
        ix.info.block_trees = ix.blocks.iter().filter(|b| b.built && b.tree.len() > 0).count() as u32;
        ix.info.block_entries = ix.blocks.iter().map(|b| b.tree.len()).sum();
        ix.info.block_bytes = ix.blocks.iter().map(|b| b.tree.stats().bytes).sum();
        ix.info.outside = ix.px.len() as u32;
    }

    /// the scene's tree in one go (bulk loading)
    fn rebuild_top(&mut self) {
        let mut items = Vec::with_capacity(self.live as usize);
        let mut px = Vec::new();
        for slot in 0..self.ents.len() as u32 {
            let e = &self.ents[slot as usize];
            if !(e.live && e.block == NONE) {
                continue;
            }
            match self.entry_of(slot) {
                Entry::Tree(b) => items.push((b, slot)),
                Entry::Px => px.push(slot),
                Entry::None => {}
            }
        }
        let n = items.len() as u32;
        let Some(ix) = self.idx.as_mut() else { return };
        ix.top = RTree::bulk(items, true);
        ix.px = px;
        ix.pending.clear();
        ix.info.rebuilt = true;
        ix.info.synced = n;
        Self::fill_info(ix);
    }

    /// blocks whose definition changed, or whose nested blocks' boxes moved, made again (children first); which blocks'
    /// boxes, reach or texts in pixels changed
    fn sync_blocks(&mut self) -> Vec<bool> {
        let nb = self.blocks.len();
        let mut changed = vec![false; nb];
        let Some(ix) = self.idx.as_mut() else { return changed };
        if ix.blocks.len() < nb {
            ix.blocks.resize(nb, BlockIdx { dirty: true, ..BlockIdx::default() });
        }
        let mut order: Vec<usize> = (0..nb).collect();
        order.sort_by_key(|&b| self.blocks[b].height);
        let mut made = 0u32;
        for b in order {
            let live = self.blocks[b].live;
            let ix = self.idx.as_ref().expect("index");
            let kids = self.blocks[b].nested.iter().any(|&c| changed[c as usize]);
            if !live {
                if ix.blocks[b].built || ix.blocks[b].bbox.is_some() {
                    let ix = self.idx.as_mut().expect("index");
                    ix.blocks[b] = BlockIdx::default();
                    changed[b] = true;
                }
                continue;
            }
            if !(ix.blocks[b].dirty || kids || !ix.blocks[b].built) {
                continue;
            }
            made += 1;
            let (old_box, old_reach, old_px) = (ix.blocks[b].bbox, ix.blocks[b].reach, ix.blocks[b].px);
            let bi = self.make_block_idx(b);
            changed[b] = bi.bbox != old_box || bi.reach != old_reach || bi.px != old_px;
            let ix = self.idx.as_mut().expect("index");
            ix.blocks[b] = bi;
        }
        if let Some(ix) = self.idx.as_mut() {
            ix.info.blocks_synced = made;
        }
        changed
    }

    /// block b's tree, box, reach (its nested blocks' are up to date)
    fn make_block_idx(&self, b: usize) -> BlockIdx {
        let ix = self.idx.as_ref().expect("index");
        let bl = &self.blocks[b];
        let max_units = self.styles.iter().map(units_reach).fold(0.0f64, f64::max);
        let mut items = Vec::with_capacity(bl.ents.len());
        let mut bbox: Option<Bx> = None;
        let (mut reach, mut px) = (0.0f64, false);
        for (k, &slot) in bl.ents.iter().enumerate() {
            let e = &self.ents[slot as usize];
            let st = &self.styles[e.style as usize];
            let bx = match &e.geom {
                Geom::Insert(ni) => {
                    let Some(child) = ix.blocks.get(ni.block.slot as usize) else { continue };
                    px |= child.px;
                    let Some(cb) = child.bbox else { continue };
                    let (g, _) = insert_boxes(ni, bl.base, &cb);
                    reach = reach.max(child.reach * copy_xf(ni, bl.base, 0, 0).sw());
                    g
                }
                Geom::Text(t) if t.lay.px => {
                    px = true;
                    pts_box(&[t.lay.origin], bl.base)
                }
                g => match geom_box(g, bl.base) {
                    Some(bx) => {
                        reach = reach.max(if st.by_block & BY_BLOCK_WIDTH != 0 { max_units } else { units_reach(st) });
                        bx
                    }
                    None => continue,
                },
            };
            bbox = Some(match bbox {
                Some(o) => geo::bx_union(&o, &bx),
                None => bx,
            });
            items.push((bx, k as u32));
        }
        BlockIdx { tree: RTree::bulk(items, false), bbox, reach, px, dirty: false, built: true }
    }

    /// where an entity on a layer goes in the index
    fn entry_of(&self, slot: u32) -> Entry {
        let e = &self.ents[slot as usize];
        if !(e.live && e.block == NONE) {
            return Entry::None;
        }
        let ix = self.idx.as_ref().expect("index");
        let st = &self.styles[e.style as usize];
        match &e.geom {
            Geom::Insert(ins) => {
                let Some(bi) = ix.blocks.get(ins.block.slot as usize) else { return Entry::None };
                if bi.px {
                    return Entry::Px;
                }
                let Some(bb) = bi.bbox else { return Entry::None };
                let (g, at) = insert_boxes(ins, [0.0, 0.0], &bb);
                let r = bi.reach * copy_xf(ins, [0.0, 0.0], 0, 0).sw();
                Entry::Tree(geo::bx_union(&geo::bx_grow(&g, r), &at))
            }
            Geom::Text(t) if t.lay.px => Entry::Px,
            g => match geom_box(g, [0.0, 0.0]) {
                Some(b) => Entry::Tree(geo::bx_grow(&b, units_reach(st))),
                None => Entry::None,
            },
        }
    }
}

enum Entry {
    Tree(Bx),
    Px,
    None,
}

/// the box of an entity's geometry less a shift (a block's base); a text's quad with its alignment point; None for an
/// insert (its block's box) and nothing drawn
fn geom_box(g: &Geom, shift: P) -> Option<Bx> {
    Some(match g {
        Geom::Line(p) => pts_box(p, shift),
        Geom::Poly(p, _) => pts_box(p, shift),
        Geom::Arc(c, r, a0, sw) => geo::arc_box(sub(*c, shift), *r, *a0, *sw),
        Geom::Curve(c) => pts_box(&[[c.bounds[0], c.bounds[1]], [c.bounds[2], c.bounds[3]]], shift),
        Geom::Fill(f) => {
            if f.rings.is_empty() {
                return None;
            }
            pts_box(&[[f.bounds[0], f.bounds[1]], [f.bounds[2], f.bounds[3]]], shift)
        }
        Geom::Text(t) => {
            let q = t.lay.quad();
            pts_box(&[q[0], q[1], q[2], q[3], text_at(&t.src)], shift)
        }
        Geom::Insert(_) => return None,
    })
}

// ---------------------------------------------------------------- walking the scene

/// where a query stands while it walks into inserts
struct Ctx {
    /// the entity on a layer it went down from
    top: u32,
    /// (copy, entity slot) per level
    path: Vec<(u32, u32)>,
    /// the painter's order: the layer's rank, the entity's place, then per level the copy and the place in the block
    key: Vec<u32>,
    /// the transform into the drawing (None: on a layer), the block's base taken off first
    xf: Option<Xf>,
    base: P,
    /// widths in units scale by this (sqrt |det|)
    sw: f64,
    /// what its entities take by block (the insert's style, derived down nested inserts)
    by: Option<Style>,
}

/// the scale and rotation of the view a query is made for (pixels to drawing units, texts in pixels)
#[derive(Clone, Copy)]
struct View {
    scale: f64,
    rot: f64,
}

impl Scene {
    /// the draw order of the live layers (by order, then by when they were added): rank per layer slot
    pub(super) fn layer_ranks(&self) -> Vec<u32> {
        let mut v: Vec<usize> = (0..self.layers.len()).filter(|&i| self.layers[i].live).collect();
        v.sort_by_key(|&i| (self.layers[i].order, self.layers[i].seq));
        let mut r = vec![0u32; self.layers.len()];
        for (k, &i) in v.iter().enumerate() {
            r[i] = k as u32;
        }
        r
    }

    fn layer_id(&self, li: u32) -> LayerId {
        LayerId { slot: li, era: self.layers[li as usize].era }
    }

    fn ent_id(&self, slot: u32) -> EntityId {
        EntityId { slot, era: self.ents[slot as usize].era }
    }

    fn target_of(&self, c: &Ctx, _slot: u32) -> Target {
        let top = c.top;
        let mut path = Vec::with_capacity(c.path.len());
        for &(copy, s) in &c.path {
            path.push((copy, self.ent_id(s)));
        }
        Target { entity: self.ent_id(top), layer: self.layer_id(self.ents[top as usize].layer), path }
    }

    /// an entity on a layer is looked at: its layer shown and not filtered out
    fn shown(&self, slot: u32, filter: Option<&dyn Fn(LayerId) -> bool>) -> bool {
        let e = &self.ents[slot as usize];
        if !(e.live && e.block == NONE) {
            return false;
        }
        let l = &self.layers[e.layer as usize];
        l.live && l.visible && filter.is_none_or(|f| f(self.layer_id(e.layer)))
    }

    /// every entity whose drawn region may meet q (drawing units, already grown by what reaches in pixels): f(ctx, slot).
    /// brute: without the trees (every entity, every entity of each block)
    fn walk(&self, q: &Bx, brute: bool, filter: Option<&dyn Fn(LayerId) -> bool>, f: &mut dyn FnMut(&Ctx, u32)) {
        let Some(ix) = self.idx.as_ref() else { return };
        let ranks = self.layer_ranks();
        let mut tops: Vec<u32> = Vec::new();
        if brute {
            tops.extend((0..self.ents.len() as u32).filter(|&s| self.ents[s as usize].live && self.ents[s as usize].block == NONE));
        } else {
            ix.top.query(q, &mut |s, _| {
                tops.push(s);
                true
            });
            tops.extend_from_slice(&ix.px);
        }
        let mut c = Ctx { top: 0, path: Vec::new(), key: Vec::new(), xf: None, base: [0.0, 0.0], sw: 1.0, by: None };
        for slot in tops {
            if !self.shown(slot, filter) {
                continue;
            }
            let e = &self.ents[slot as usize];
            c.top = slot;
            c.key.clear();
            c.key.push(ranks[e.layer as usize]);
            c.key.push(e.at);
            if let Geom::Insert(ins) = &e.geom {
                let st = self.styles[e.style as usize];
                self.walk_insert(ins, [0.0, 0.0], None, q, brute, &st, &mut c, f);
            } else {
                f(&c, slot);
            }
        }
    }

    /// the copies of an insert that may meet q: down into its block (parent: the transform of the block it sits in)
    #[allow(clippy::too_many_arguments)]
    fn walk_insert(&self, ins: &Insert, origin: P, parent: Option<Xf>, q: &Bx, brute: bool, ins_style: &Style, c: &mut Ctx, f: &mut dyn FnMut(&Ctx, u32)) {
        let Some(ix) = self.idx.as_ref() else { return };
        let b = ins.block.slot as usize;
        let Some(bi) = ix.blocks.get(b) else { return };
        let Some(bb) = bi.bbox else { return };
        // (q in the units the insert sits in)
        let ql = match &parent {
            Some(p) => inv_box(p, q),
            None => *q,
        };
        let m0 = copy_xf(ins, origin, 0, 0).m;
        let sw0 = Xf { t: [0.0, 0.0], m: m0 }.sw();
        let pm = parent.map_or(1.0, |p| p.sw());
        let (c0, c1, r0, r1) = if brute { (0, ins.columns.max(1) - 1, 0, ins.rows.max(1) - 1) } else { copies_meeting(ins, origin, &bb, &ql, bi.reach * sw0 + 1e-9 * (1.0 + pm)) };
        let bl = &self.blocks[b];
        for row in r0..=r1.min(ins.rows.max(1) - 1) {
            for col in c0..=c1.min(ins.columns.max(1) - 1) {
                let local = copy_xf(ins, origin, col, row);
                let xf = match &parent {
                    Some(p) => p.then(&local),
                    None => local,
                };
                let sw = xf.sw();
                // (the query in the block's units, grown by its widths in units)
                let qd = geo::bx_grow(q, bi.reach * sw);
                let qb = inv_box(&xf, &qd);
                if !brute && !geo::bx_meets(&qb, &bb) {
                    continue;
                }
                let copy = row * ins.columns.max(1) + col;
                let mut ks: Vec<u32> = Vec::new();
                if brute || bi.px {
                    ks.extend(0..bl.ents.len() as u32);
                } else {
                    bi.tree.query(&qb, &mut |k, _| {
                        ks.push(k);
                        true
                    });
                }
                let saved = (c.xf, c.base, c.sw, c.by);
                for k in ks {
                    let Some(&slot) = bl.ents.get(k as usize) else { continue };
                    let e = &self.ents[slot as usize];
                    if !e.live {
                        continue;
                    }
                    c.path.push((copy, slot));
                    c.key.push(copy);
                    c.key.push(k);
                    c.xf = Some(xf);
                    c.base = bl.base;
                    c.sw = sw;
                    c.by = Some(*ins_style);
                    if let Geom::Insert(ni) = &e.geom {
                        // (what the nested insert's block takes by block)
                        let own = self.styles[e.style as usize];
                        let next = if own.takes() { ins_style.by_block_from(&own) } else { own };
                        self.walk_insert(ni, bl.base, Some(xf), q, brute, &next, c, f);
                    } else {
                        f(c, slot);
                    }
                    c.path.pop();
                    c.key.pop();
                    c.key.pop();
                }
                (c.xf, c.base, c.sw, c.by) = saved;
            }
        }
    }

    /// the style an entity draws with where it sits (its width by block from the insert's)
    fn style_for(&self, c: &Ctx, slot: u32) -> Style {
        let st = self.styles[self.ents[slot as usize].style as usize];
        match c.by {
            Some(b) if st.by_block & BY_BLOCK_WIDTH != 0 => {
                let mut s = st;
                s.width = b.width;
                s.unit = b.unit;
                s
            }
            _ => st,
        }
    }

    /// the half width of an entity's stroke in drawing units (at least half a pixel)
    fn half_width(&self, c: &Ctx, st: &Style, v: View) -> f64 {
        let w = st.width as f64;
        if st.unit == Unit::Drawing { (0.5 * w * c.sw).max(0.5 * v.scale) } else { (0.5 * w).max(0.5) * v.scale }
    }
}

// ---------------------------------------------------------------- an entity's geometry in the drawing

/// a point of a block's entity in the drawing, relative to o
fn place(c: &Ctx, p: P, o: P) -> P {
    match &c.xf {
        Some(xf) => sub(xf.apply(sub(p, c.base)), o),
        None => sub(p, o),
    }
}

/// a span of a curve in the drawing (its origin relative to o, its control points through the transform's linear part)
fn place_span(c: &Ctx, origin: P, s: &Span, o: P) -> Prim {
    match &c.xf {
        None => Prim::Span(sub(origin, o), *s),
        Some(xf) => {
            let mut t = *s;
            for k in 0..s.deg as usize + 1 {
                let v = xf.lin([s.p[k][0], s.p[k][1]]);
                t.p[k] = [v[0], v[1], s.p[k][2]];
            }
            Prim::Span(place(c, origin, o), t)
        }
    }
}

fn curve_prims(c: &Ctx, cv: &Curve, o: P, out: &mut Vec<Prim>) {
    for s in &cv.spans {
        if s.deg == 1 {
            let a = add(cv.origin, [s.p[0][0] / s.p[0][2], s.p[0][1] / s.p[0][2]]);
            let b = add(cv.origin, [s.p[1][0] / s.p[1][2], s.p[1][1] / s.p[1][2]]);
            out.push(Prim::Seg(place(c, a, o), place(c, b, o)));
        } else {
            out.push(place_span(c, cv.origin, s, o));
        }
    }
}

/// a circular arc through the transform: an arc again under a similarity (None: under an uneven one it is an ellipse)
fn place_arc(c: &Ctx, ctr: P, r: f64, a0: f64, sw: f64, o: P) -> Option<Prim> {
    match &c.xf {
        None => Some(Prim::Arc(sub(ctr, o), r, a0, sw)),
        Some(xf) if xf.even() => {
            let col = [xf.m[0][0], xf.m[1][0]];
            let phi = geo::angle_of(col);
            let s = xf.sw();
            let (a0, sw) = if xf.det() < 0.0 { (phi - a0, -sw) } else { (phi + a0, sw) };
            Some(Prim::Arc(place(c, ctr, o), r * s, a0, sw))
        }
        Some(_) => None,
    }
}

/// a text's quad in the drawing, relative to o
fn text_quad(c: &Ctx, lay: &crate::text::Layout, v: View, o: P) -> [P; 4] {
    let q = lay.quad();
    if lay.px {
        // pixels from the origin, turning with the drawing: drawing units by the scale (in an insert through its map over
        // its scale for widths, as textc.wgsl's text_map)
        let org = lay.origin;
        let mut out = [[0.0; 2]; 4];
        for k in 0..4 {
            let off = mul(sub(q[k], org), v.scale);
            let off = match &c.xf {
                Some(xf) => mul(xf.lin(off), 1.0 / xf.sw()),
                None => off,
            };
            out[k] = add(place(c, org, o), off);
        }
        return out;
    }
    [place(c, q[0], o), place(c, q[1], o), place(c, q[2], o), place(c, q[3], o)]
}

/// what one entity is in the drawing for picking and selection: strokes (pieces, closed; a polyline's points), a fill's
/// rings, a text's quad
enum Shape2 {
    Poly(Vec<P>, bool),
    Arc(Prim),
    Curve(Vec<Prim>, bool),
    Fill(Vec<Prim>),
    Quad([P; 4]),
}

impl Scene {
    /// an entity's geometry in the drawing relative to o (None: nothing; an insert is walked, never shaped)
    fn shape2(&self, c: &Ctx, slot: u32, v: View, o: P) -> Option<Shape2> {
        let e = &self.ents[slot as usize];
        Some(match &e.geom {
            Geom::Line(p) => Shape2::Poly(vec![place(c, p[0], o), place(c, p[1], o)], false),
            Geom::Poly(p, closed) => {
                let mut pts: Vec<P> = p.iter().map(|q| place(c, *q, o)).collect();
                // (as records() takes them: repeated points once, a closed one's last equal to its first once)
                pts.dedup();
                if *closed && pts.len() >= 2 && pts[0] == pts[pts.len() - 1] {
                    pts.pop();
                }
                Shape2::Poly(pts, *closed)
            }
            &Geom::Arc(ctr, r, a0, sw) => match place_arc(c, ctr, r, a0, sw, o) {
                Some(p) => Shape2::Arc(p),
                None => {
                    let cv = Curve::ellipse(ctr, [r, r], 0.0, a0, sw);
                    let mut v = Vec::new();
                    curve_prims(c, &cv, o, &mut v);
                    Shape2::Curve(v, cv.closed)
                }
            },
            Geom::Curve(g) => {
                let mut v = Vec::new();
                curve_prims(c, &g.c, o, &mut v);
                Shape2::Curve(v, g.c.closed)
            }
            Geom::Fill(g) => {
                let mut v = Vec::new();
                for r in &g.rings {
                    curve_prims(c, r, o, &mut v);
                }
                Shape2::Fill(v)
            }
            Geom::Text(t) => Shape2::Quad(text_quad(c, &t.lay, v, o)),
            Geom::Insert(_) => return None,
        })
    }
}

// ---------------------------------------------------------------- picking

/// a stroke's look for picking: half width (drawing units), cap, join, miter limit
#[derive(Clone, Copy)]
struct Pen {
    h: f64,
    cap: u32,
    join: u32,
    limit: f64,
}

fn pen_of(st: &Style, h: f64) -> Pen {
    let cap = match st.cap {
        Cap::Butt => 0,
        Cap::Round => 1,
        _ => 2,
    };
    let join = match st.join {
        Join::Miter => 0,
        Join::Round => 1,
        _ => 2,
    };
    Pen { h, cap, join, limit: st.miter_limit as f64 }
}

/// the distance from q to a piece's band (its points within h across it; beyond an end, the end's line across)
fn band_dist(q: P, p: &Prim, h: f64) -> f64 {
    let (d, t, _) = geo::nearest(q, p);
    if t <= 0.0 {
        let s = p.start();
        let out = unit(mul(p.tangent(0.0), -1.0));
        if dot(sub(q, s), out) > 0.0 {
            let n = mul(perp(out), h);
            return geo::nearest(q, &Prim::Seg(sub(s, n), add(s, n))).0;
        }
    }
    if t >= 1.0 {
        let e = p.end();
        let out = unit(p.tangent(1.0));
        if dot(sub(q, e), out) > 0.0 {
            let n = mul(perp(out), h);
            return geo::nearest(q, &Prim::Seg(sub(e, n), add(e, n))).0;
        }
    }
    (d - h).max(0.0)
}

/// the distance from q to a stroke along pieces end to end (closed: the last joins the first): their bands, the joins
/// at kinks (a polyline: every vertex), the caps at open ends.  Pieces farther than lim (and what they reach) are not
/// looked at: the result is exact up to lim, beyond it at least lim
fn stroke_dist(q: P, ps: &[Prim], closed: bool, pen: &Pen, lim: f64) -> f64 {
    let mut d = f64::MAX;
    let reach = lim + pen.h * core::f64::consts::SQRT_2.max(pen.limit) + 1e-9 * (1.0 + lim);
    let far = |p: &P| dot(sub(*p, q), sub(*p, q)) > reach * reach;
    let mut boxes: Vec<bool> = Vec::with_capacity(ps.len());
    for p in ps {
        let near = geo::bx_dist2(&p.bx(), q) <= reach * reach;
        boxes.push(near);
        if near {
            d = d.min(band_dist(q, p, pen.h));
        }
    }
    let n = ps.len();
    let joints = if closed { n } else { n.saturating_sub(1) };
    for k in 0..joints {
        if !(boxes[k] && boxes[(k + 1) % n]) {
            continue;
        }
        let (a, b) = (&ps[k], &ps[(k + 1) % n]);
        let v = a.end();
        if far(&v) {
            continue;
        }
        let (d1, d2) = (unit(a.tangent(1.0)), unit(b.tangent(0.0)));
        // (a kink: a turn the flattening shows as a join of the style; smooth joints have none)
        if cross(d1, d2).abs() > 1e-9 || dot(d1, d2) < 0.0 {
            d = d.min(geo::join_dist(q, v, d1, d2, pen.h, pen.join, pen.limit));
        }
    }
    if !closed && n > 0 && pen.cap != 0 {
        let s = ps[0].start();
        if !far(&s) {
            d = d.min(geo::cap_dist(q, s, unit(mul(ps[0].tangent(0.0), -1.0)), pen.h, pen.cap));
        }
        let e = ps[n - 1].end();
        if !far(&e) {
            d = d.min(geo::cap_dist(q, e, unit(ps[n - 1].tangent(1.0)), pen.h, pen.cap));
        }
    }
    d
}

impl Scene {
    /// the distance in drawing units from q (relative coordinates: 0) to what an entity draws, exact up to lim (beyond it
    /// at least lim); o: q in the drawing
    fn hit_dist(&self, c: &Ctx, slot: u32, v: View, o: P, lim: f64) -> f64 {
        let Some(sh) = self.shape2(c, slot, v, o) else { return f64::MAX };
        let q = [0.0, 0.0];
        let st = self.style_for(c, slot);
        let pen = pen_of(&st, self.half_width(c, &st, v));
        match sh {
            Shape2::Poly(pts, closed) => {
                if pts.len() == 1 {
                    // a point: what its caps draw (a square one along the pixels' axes, as the vertex shader turns it)
                    return match pen.cap {
                        1 => (len(pts[0]) - pen.h).max(0.0),
                        2 => {
                            let (s, co) = sincos(v.rot);
                            let d = sub(q, pts[0]);
                            let (x, y) = (co * d[0] - s * d[1], s * d[0] + co * d[1]);
                            let (dx, dy) = ((x.abs() - pen.h).max(0.0), (y.abs() - pen.h).max(0.0));
                            sqrt(dx * dx + dy * dy)
                        }
                        _ => f64::MAX,
                    };
                }
                let n = pts.len();
                let segs = if closed { n } else { n - 1 };
                let ps: Vec<Prim> = (0..segs).map(|k| Prim::Seg(pts[k], pts[(k + 1) % n])).collect();
                stroke_dist(q, &ps, closed, &pen, lim)
            }
            Shape2::Arc(p) => {
                let closed = matches!(p, Prim::Arc(_, _, _, sw) if sw.abs() >= TAU);
                stroke_dist(q, &[p], closed, &pen, lim)
            }
            Shape2::Curve(ps, closed) => stroke_dist(q, &ps, closed, &pen, lim),
            Shape2::Fill(ps) => {
                if geo::inside(q, &ps) {
                    return 0.0;
                }
                let r2 = (lim * (1.0 + 1e-9) + 1e-300) * (lim * (1.0 + 1e-9) + 1e-300);
                ps.iter().filter(|p| geo::bx_dist2(&p.bx(), q) <= r2).map(|p| geo::nearest(q, p).0).fold(f64::MAX, f64::min)
            }
            Shape2::Quad(quad) => geo::convex_dist(q, &quad),
        }
    }

    /// the most any stroke reaches in pixels beyond its centre line (widths in pixels, hairlines), and of the style by block
    pub(super) fn max_px_reach(&self) -> f64 {
        self.styles.iter().map(px_reach).fold(0.5 * core::f64::consts::SQRT_2, f64::max)
    }

    /// what is drawn within `tolerance` pixels of a point of the drawing (f64), topmost first (the painter's order: an app
    /// cycles through overlapping ones); inside an insert the hit names the insert and the way down to the entity
    pub fn pick(&mut self, at: [f64; 2], tolerance: f64) -> Vec<Hit> {
        self.pick_with(at, tolerance, None, false)
    }

    /// pick with a filter of layers (false: skipped; hidden layers are skipped anyway): a layer lock of an app
    pub fn pick_filtered(&mut self, at: [f64; 2], tolerance: f64, filter: &dyn Fn(LayerId) -> bool) -> Vec<Hit> {
        self.pick_with(at, tolerance, Some(filter), false)
    }

    /// (tests) pick without the index: every entity, every entity of every block
    pub fn pick_brute(&mut self, at: [f64; 2], tolerance: f64) -> Vec<Hit> {
        self.pick_with(at, tolerance, None, true)
    }

    fn pick_with(&mut self, at: [f64; 2], tolerance: f64, filter: Option<&dyn Fn(LayerId) -> bool>, brute: bool) -> Vec<Hit> {
        self.sync_index();
        let v = View { scale: self.camera.scale, rot: self.camera.rotation };
        let tol = tolerance.max(0.0) * v.scale;
        let reach = (tolerance.max(0.0) + self.max_px_reach()) * v.scale;
        let q = [at[0] - reach, at[1] - reach, at[0] + reach, at[1] + reach];
        let mut hits: Vec<(Vec<u32>, Hit)> = Vec::new();
        self.walk(&q, brute, filter, &mut |c, slot| {
            let d = self.hit_dist(c, slot, v, at, tol);
            if d <= tol {
                hits.push((c.key.clone(), Hit { target: self.target_of(c, slot), distance: d / v.scale }));
            }
        });
        // topmost first: the painter's order backwards
        hits.sort_by(|a, b| b.0.cmp(&a.0));
        hits.into_iter().map(|h| h.1).collect()
    }
}

// ---------------------------------------------------------------- area selection

impl Scene {
    /// the entities on layers a region selects (an insert as a whole): Window what lies wholly inside, Crossing what lies
    /// inside or touches it (AutoCAD's); by the centre lines (widths do not count, as in AutoCAD), a fill by its rings (a
    /// crossing region inside a fill touches it), a text by its quad.  In no particular order
    pub fn select(&mut self, region: Region, mode: SelectMode) -> Vec<EntityId> {
        self.select_with(region, mode, None, false)
    }

    /// select with a filter of layers (false: skipped)
    pub fn select_filtered(&mut self, region: Region, mode: SelectMode, filter: &dyn Fn(LayerId) -> bool) -> Vec<EntityId> {
        self.select_with(region, mode, Some(filter), false)
    }

    /// (tests) select without the index
    pub fn select_brute(&mut self, region: Region, mode: SelectMode) -> Vec<EntityId> {
        self.select_with(region, mode, None, true)
    }

    fn select_with(&mut self, region: Region, mode: SelectMode, filter: Option<&dyn Fn(LayerId) -> bool>, brute: bool) -> Vec<EntityId> {
        self.sync_index();
        let area = match region {
            Region::Rect(a, b) => Area::Rect([a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]),
            Region::Polygon(p) => {
                if p.len() < 3 {
                    return Vec::new();
                }
                Area::Poly(p.to_vec())
            }
        };
        let v = View { scale: self.camera.scale, rot: self.camera.rotation };
        let bx = area.bx();
        // (relative to the region's middle: small numbers near it)
        let o = [0.5 * (bx[0] + bx[2]), 0.5 * (bx[1] + bx[3])];
        let rel = match &area {
            Area::Rect(b) => Area::Rect([b[0] - o[0], b[1] - o[1], b[2] - o[0], b[3] - o[1]]),
            Area::Poly(p) => Area::Poly(p.iter().map(|x| sub(*x, o)).collect()),
        };
        let window = mode == SelectMode::Window;
        let Some(ix) = self.idx.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        let mut look: Vec<u32> = Vec::new();
        if brute {
            look.extend((0..self.ents.len() as u32).filter(|&s| self.ents[s as usize].live && self.ents[s as usize].block == NONE));
        } else {
            match &area {
                Area::Rect(r) => ix.top.within(r, &mut |s| {
                    // (a box wholly inside: its geometry too)
                    if self.shown(s, filter) {
                        out.push(self.ent_id(s));
                    }
                }, &mut |s, _| look.push(s)),
                Area::Poly(_) => ix.top.query(&bx, &mut |s, _| {
                    look.push(s);
                    true
                }),
            }
            look.extend_from_slice(&ix.px);
        }
        let mut c = Ctx { top: 0, path: Vec::new(), key: Vec::new(), xf: None, base: [0.0, 0.0], sw: 1.0, by: None };
        for s in look {
            if !self.shown(s, filter) {
                continue;
            }
            c.top = s;
            let e = &self.ents[s as usize];
            let ok = match &e.geom {
                Geom::Insert(ins) => self.insert_selected(ins, [0.0, 0.0], None, &rel, o, window, v, brute, &mut c),
                _ => self.selected(&c, s, &rel, o, window, v),
            };
            if ok {
                out.push(self.ent_id(s));
            }
        }
        out
    }

    /// an entity (its geometry relative to o) lies in the region (window) or meets it (crossing)
    fn selected(&self, c: &Ctx, slot: u32, area: &Area, o: P, window: bool, v: View) -> bool {
        let Some(sh) = self.shape2(c, slot, v, o) else { return false };
        let ps: Vec<Prim> = match sh {
            Shape2::Poly(pts, closed) => {
                if pts.len() == 1 {
                    return area.has(pts[0]);
                }
                let n = pts.len();
                let segs = if closed { n } else { n - 1 };
                (0..segs).map(|k| Prim::Seg(pts[k], pts[(k + 1) % n])).collect()
            }
            Shape2::Arc(p) => vec![p],
            Shape2::Curve(ps, _) => ps,
            Shape2::Fill(ps) => {
                if ps.is_empty() {
                    return false;
                }
                if window {
                    return ps.iter().all(|p| geo::prim_in(p, area));
                }
                return ps.iter().any(|p| geo::prim_meets(p, area)) || geo::inside(area.some_point(), &ps);
            }
            Shape2::Quad(q) => {
                let ps: Vec<Prim> = (0..4).map(|k| Prim::Seg(q[k], q[(k + 1) % 4])).collect();
                if window {
                    return ps.iter().all(|p| geo::prim_in(p, area));
                }
                return ps.iter().any(|p| geo::prim_meets(p, area)) || geo::inside(area.some_point(), &ps);
            }
        };
        if window { ps.iter().all(|p| geo::prim_in(p, area)) } else { ps.iter().any(|p| geo::prim_meets(p, area)) }
    }

    /// an insert (each copy) in the region (window: every copy wholly) or meeting it (crossing: any copy); a copy whose
    /// block box through its transform lies wholly in the region or wholly beside it is decided by that box
    #[allow(clippy::too_many_arguments)]
    fn insert_selected(&self, ins: &Insert, origin: P, parent: Option<Xf>, area: &Area, o: P, window: bool, v: View, brute: bool, c: &mut Ctx) -> bool {
        let Some(ix) = self.idx.as_ref() else { return false };
        let b = ins.block.slot as usize;
        let Some(bi) = ix.blocks.get(b) else { return false };
        let Some(bb) = bi.bbox else { return false };
        let bl = &self.blocks[b];
        for row in 0..ins.rows.max(1) {
            for col in 0..ins.columns.max(1) {
                let local = copy_xf(ins, origin, col, row);
                let xf = match &parent {
                    Some(p) => p.then(&local),
                    None => local,
                };
                // the block's box through the transform: a parallelogram (relative to o)
                let corners = [[bb[0], bb[1]], [bb[2], bb[1]], [bb[2], bb[3]], [bb[0], bb[3]]].map(|p| sub(xf.apply(p), o));
                let edges: Vec<Prim> = (0..4).map(|k| Prim::Seg(corners[k], corners[(k + 1) % 4])).collect();
                // (texts in pixels may reach beyond the box: no shortcut for their blocks)
                let fast = !brute && !bi.px;
                let strictly_in = |x: P| geo::convex_dist(x, &corners) == 0.0 && edges.iter().all(|e| geo::nearest(x, e).0 > 0.0);
                let inside = fast && edges.iter().all(|p| geo::prim_in(p, area)) && match area {
                    Area::Rect(_) => true,
                    Area::Poly(pv) => !pv.iter().any(|x| strictly_in(*x)),
                };
                let beside = fast && !edges.iter().any(|p| geo::prim_meets(p, area)) && geo::convex_dist(area.some_point(), &corners) > 0.0;
                let this = if inside {
                    true
                } else if beside {
                    false
                } else {
                    // down into the block: every entity (window) or any (crossing)
                    let saved = (c.xf, c.base, c.sw);
                    c.xf = Some(xf);
                    c.base = bl.base;
                    c.sw = xf.sw();
                    let mut any = false;
                    let mut all = true;
                    let mut drew = false;
                    for &slot in &bl.ents {
                        let e = &self.ents[slot as usize];
                        if !e.live {
                            continue;
                        }
                        let r = match &e.geom {
                            Geom::Insert(ni) => {
                                if ix.blocks.get(ni.block.slot as usize).is_none_or(|x| x.bbox.is_none()) {
                                    continue;
                                }
                                self.insert_selected(ni, bl.base, Some(xf), area, o, window, v, brute, c)
                            }
                            _ => {
                                if self.shape2(c, slot, v, o).is_none() {
                                    continue;
                                }
                                self.selected(c, slot, area, o, window, v)
                            }
                        };
                        drew = true;
                        any |= r;
                        all &= r;
                        if (window && !all) || (!window && any) {
                            break;
                        }
                    }
                    (c.xf, c.base, c.sw) = saved;
                    if window { drew && all } else { any }
                };
                if window && !this {
                    return false;
                }
                if !window && this {
                    return true;
                }
            }
        }
        window
    }
}

// ---------------------------------------------------------------- snapping

/// what one entity offers a snap where it sits: its pieces near the pointer (relative to it), its points of each kind
#[derive(Default)]
struct Offer {
    ps: Vec<Prim>,
    /// a polyline's: each piece's segment number (neighbours meet at their vertex: no intersection there)
    ks: Vec<u32>,
    chain: bool,
    segs: u32,
    closed: bool,
    pts: Vec<(SnapKind, P)>,
    /// centres (offered when the pointer is over the curve, wherever they lie)
    cens: Vec<P>,
}

impl Scene {
    /// an entity's snap offer relative to the pointer o: pieces whose box meets rq (the aperture's box), the points of the
    /// shape it was made from (through the insert's map)
    fn offer(&self, c: &Ctx, slot: u32, o: P, rq: &Bx, kinds: u32) -> Offer {
        let e = &self.ents[slot as usize];
        let mut f = Offer::default();
        let near = |a: P, b: P| geo::bx_meets(&geo::bx_of(&[a, b]), rq);
        match &e.geom {
            Geom::Line(p) => {
                let (a, b) = (place(c, p[0], o), place(c, p[1], o));
                f.ps.push(Prim::Seg(a, b));
                f.pts.push((SnapKind::Endpoint, a));
                f.pts.push((SnapKind::Endpoint, b));
                f.pts.push((SnapKind::Midpoint, mul(add(a, b), 0.5)));
            }
            Geom::Poly(p, closed) => {
                let mut pts: Vec<P> = p.iter().map(|q| place(c, *q, o)).collect();
                pts.dedup();
                if *closed && pts.len() >= 2 && pts[0] == pts[pts.len() - 1] {
                    pts.pop();
                }
                let n = pts.len();
                for &q in &pts {
                    if geo::bx_has(rq, q) {
                        f.pts.push((SnapKind::Endpoint, q));
                    }
                }
                let segs = if *closed && n > 2 { n } else { n.saturating_sub(1) };
                for k in 0..segs {
                    let (a, b) = (pts[k], pts[(k + 1) % n]);
                    if near(a, b) {
                        f.ps.push(Prim::Seg(a, b));
                        f.ks.push(k as u32);
                        f.pts.push((SnapKind::Midpoint, mul(add(a, b), 0.5)));
                    }
                }
                f.chain = true;
                f.segs = segs as u32;
                f.closed = *closed && n > 2;
            }
            &Geom::Arc(ctr, r, a0, sw) => {
                match place_arc(c, ctr, r, a0, sw, o) {
                    Some(p) => f.ps.push(p),
                    None => curve_prims(c, &Curve::ellipse(ctr, [r, r], 0.0, a0, sw), o, &mut f.ps),
                }
                if sw.abs() < TAU {
                    f.pts.push((SnapKind::Endpoint, place(c, geo::arc_at(ctr, r, a0), o)));
                    f.pts.push((SnapKind::Endpoint, place(c, geo::arc_at(ctr, r, a0 + sw), o)));
                    f.pts.push((SnapKind::Midpoint, place(c, geo::arc_at(ctr, r, a0 + 0.5 * sw), o)));
                }
                f.cens.push(place(c, ctr, o));
                if kinds & SNAP_QUA != 0 {
                    quadrants(c, ctr, [[r, 0.0], [0.0, r]], a0, sw, o, &mut f.pts);
                }
            }
            Geom::Curve(g) => {
                let mut all = Vec::new();
                curve_prims(c, &g.c, o, &mut all);
                let cv = &g.c;
                let n = cv.spans.len();
                if n > 0 {
                    let pt = |s: &Span, t: f64| place(c, add(cv.origin, geo::sp_pt(s, t)), o);
                    match g.src {
                        super::CurveSrc::Ellipse { center, radii, rotation, start, sweep } => {
                            f.cens.push(place(c, center, o));
                            if sweep.abs() < TAU {
                                f.pts.push((SnapKind::Endpoint, pt(&cv.spans[0], 0.0)));
                                f.pts.push((SnapKind::Endpoint, pt(&cv.spans[n - 1], 1.0)));
                                if kinds & SNAP_MID != 0 {
                                    if let Some(p) = mid_by_length(cv) {
                                        f.pts.push((SnapKind::Midpoint, place(c, p, o)));
                                    }
                                }
                            }
                            if kinds & SNAP_QUA != 0 {
                                let (sr, cr) = sincos(rotation);
                                let m = [[cr * radii[0], -sr * radii[1]], [sr * radii[0], cr * radii[1]]];
                                quadrants(c, center, m, start, sweep, o, &mut f.pts);
                            }
                        }
                        super::CurveSrc::Chain => {
                            // (the vertices between pieces and each piece's middle by its length)
                            if !cv.closed {
                                f.pts.push((SnapKind::Endpoint, pt(&cv.spans[0], 0.0)));
                            }
                            for (s, p) in cv.spans.iter().zip(&all) {
                                f.pts.push((SnapKind::Endpoint, pt(s, 1.0)));
                                // (the middle lies in the piece's box)
                                if kinds & SNAP_MID != 0 && geo::bx_meets(&p.bx(), rq) {
                                    let l = geo::sp_length(s, 1.0);
                                    f.pts.push((SnapKind::Midpoint, pt(s, geo::sp_at_length(s, 0.5 * l))));
                                }
                            }
                        }
                        super::CurveSrc::Spline => {
                            if !cv.closed {
                                f.pts.push((SnapKind::Endpoint, pt(&cv.spans[0], 0.0)));
                                f.pts.push((SnapKind::Endpoint, pt(&cv.spans[n - 1], 1.0)));
                                if kinds & SNAP_MID != 0 {
                                    if let Some(p) = mid_by_length(cv) {
                                        f.pts.push((SnapKind::Midpoint, place(c, p, o)));
                                    }
                                }
                            }
                        }
                    }
                }
                f.ps = all.into_iter().filter(|p| geo::bx_meets(&p.bx(), rq)).collect();
            }
            Geom::Fill(g) => {
                for ring in &g.rings {
                    let mut all = Vec::new();
                    curve_prims(c, ring, o, &mut all);
                    // corners: where a straight piece meets anything, or the boundary turns by more than 1/1000 rad (curves
                    // given at 1e7 hold their tangents to about that: smooth joints stay no corners); middles of straight pieces
                    let n = all.len();
                    for k in 0..n {
                        let (a, b) = (&all[k], &all[(k + 1) % n]);
                        let corner = a.end();
                        if geo::bx_has(rq, corner) {
                            let (d1, d2) = (unit(a.tangent(1.0)), unit(b.tangent(0.0)));
                            if matches!(a, Prim::Seg(..)) || matches!(b, Prim::Seg(..)) || cross(d1, d2).abs() > 1e-3 || dot(d1, d2) < 0.0 {
                                f.pts.push((SnapKind::Endpoint, corner));
                            }
                        }
                        if let Prim::Seg(p, q) = a {
                            f.pts.push((SnapKind::Midpoint, mul(add(*p, *q), 0.5)));
                        }
                    }
                    f.ps.extend(all.into_iter().filter(|p| geo::bx_meets(&p.bx(), rq)));
                }
            }
            Geom::Text(t) => f.pts.push((SnapKind::Insertion, place(c, text_at(&t.src), o))),
            Geom::Insert(_) => {}
        }
        f
    }
}

/// the quadrant points of an ellipse centre + m (cos t, sin t) from t = start over sweep, through the insert's map: where
/// the whole map is a similarity the points at t = 0, 90, 180, 270 degrees (the shape's own axes), else the ends of the
/// image's axes (within the sweep)
fn quadrants(c: &Ctx, center: P, m: [[f64; 2]; 2], start: f64, sweep: f64, o: P, out: &mut Vec<(SnapKind, P)>) {
    let lin = match &c.xf {
        Some(xf) => {
            let a = xf.m;
            [[a[0][0] * m[0][0] + a[0][1] * m[1][0], a[0][0] * m[0][1] + a[0][1] * m[1][1]], [a[1][0] * m[0][0] + a[1][1] * m[1][0], a[1][0] * m[0][1] + a[1][1] * m[1][1]]]
        }
        None => m,
    };
    let (s1, s2, v1) = geo::svd2(lin);
    let dirs: [P; 4] = if s1 - s2 <= 1e-12 * s1 { [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] } else { [v1, perp(v1), mul(v1, -1.0), mul(perp(v1), -1.0)] };
    let cw = place(c, center, o);
    for d in dirs {
        if geo::arc_has(start, sweep, d) {
            let off = [lin[0][0] * d[0] + lin[0][1] * d[1], lin[1][0] * d[0] + lin[1][1] * d[1]];
            out.push((SnapKind::Quadrant, add(cw, off)));
        }
    }
}

impl Scene {
    /// the object snap at a point of the drawing (f64): of the kinds asked for within the aperture, the one nearest the
    /// pointer (equally near: endpoint, intersection, midpoint, centre, quadrant, insertion, perpendicular, tangent),
    /// nearest only where nothing else is
    pub fn snap(&mut self, at: [f64; 2], opt: &SnapOptions) -> Option<Snap> {
        self.snaps_with(at, opt, None, false, true).into_iter().next()
    }

    /// every snap candidate in that order (an app cycles through them)
    pub fn snaps(&mut self, at: [f64; 2], opt: &SnapOptions) -> Vec<Snap> {
        self.snaps_with(at, opt, None, false, false)
    }

    /// snaps with a filter of layers (false: skipped)
    pub fn snaps_filtered(&mut self, at: [f64; 2], opt: &SnapOptions, filter: &dyn Fn(LayerId) -> bool) -> Vec<Snap> {
        self.snaps_with(at, opt, Some(filter), false, false)
    }

    /// (tests) snaps without the index
    pub fn snaps_brute(&mut self, at: [f64; 2], opt: &SnapOptions) -> Vec<Snap> {
        self.snaps_with(at, opt, None, true, false)
    }

    /// first: only the first candidate is wanted (snap): intersections farther than the nearest other candidate are not
    /// looked for
    fn snaps_with(&mut self, at: [f64; 2], opt: &SnapOptions, filter: Option<&dyn Fn(LayerId) -> bool>, brute: bool, first: bool) -> Vec<Snap> {
        self.sync_index();
        let v = View { scale: self.camera.scale, rot: self.camera.rotation };
        let ap = opt.aperture.max(0.0) * v.scale;
        // (points of several kinds in one place, within 1/1000 px: the kind of the higher rank, as AutoCAD takes an endpoint
        // where an intersection falls on it)
        // (or within a few ulps of the coordinates: where f64 itself cannot tell them apart)
        let same = (1e-3 * v.scale).max(4.0 * f64::EPSILON * (at[0].abs() + at[1].abs()));
        let q = [at[0] - ap, at[1] - ap, at[0] + ap, at[1] + ap];
        let rq = [-ap, -ap, ap, ap];
        let base = opt.base.map(|b| sub(b, at));
        let kinds = opt.kinds;
        // the entities near the pointer: their offers (pieces relative to the pointer)
        let mut near: Vec<(Target, Offer)> = Vec::new();
        self.walk(&q, brute, filter, &mut |c, slot| {
            let f = self.offer(c, slot, at, &rq, kinds);
            let on = f.ps.iter().any(|p| geo::bx_meets(&p.bx(), &rq) && geo::nearest([0.0, 0.0], p).0 <= ap);
            let ins = f.pts.iter().any(|(k, p)| *k == SnapKind::Insertion && len(*p) <= ap);
            if on || ins {
                near.push((self.target_of(c, slot), f));
            }
        });
        // insertion points of inserts on layers (each copy)
        let mut cands: Vec<(SnapKind, P, usize, Option<usize>)> = Vec::new();
        let mut ins_targets: Vec<Target> = Vec::new();
        if kinds & SNAP_INS != 0 {
            if let Some(ix) = self.idx.as_ref() {
                let mut tops = Vec::new();
                if brute {
                    tops.extend((0..self.ents.len() as u32).filter(|&s| self.ents[s as usize].live && self.ents[s as usize].block == NONE));
                } else {
                    ix.top.query(&q, &mut |s, _| {
                        tops.push(s);
                        true
                    });
                    tops.extend_from_slice(&ix.px);
                }
                for s in tops {
                    if !self.shown(s, filter) {
                        continue;
                    }
                    if let Geom::Insert(ins) = &self.ents[s as usize].geom {
                        let (cols, rows) = (ins.columns.max(1), ins.rows.max(1));
                        for row in 0..rows {
                            for col in 0..cols {
                                let p = sub(copy_xf(ins, [0.0, 0.0], col, row).t, at);
                                if len(p) <= ap {
                                    ins_targets.push(Target { entity: self.ent_id(s), layer: self.layer_id(self.ents[s as usize].layer), path: Vec::new() });
                                    cands.push((SnapKind::Insertion, p, usize::MAX - (ins_targets.len() - 1), None));
                                }
                            }
                        }
                    }
                }
            }
        }
        let want = |k: SnapKind| kinds & k.bit() != 0;
        for (k, (_, f)) in near.iter().enumerate() {
            for &(kind, p) in &f.pts {
                if want(kind) && len(p) <= ap {
                    cands.push((kind, p, k, None));
                }
            }
            if want(SnapKind::Center) {
                for &p in &f.cens {
                    cands.push((SnapKind::Center, p, k, None));
                }
            }
            if let Some(b) = base {
                let mut pts = Vec::new();
                for p in &f.ps {
                    if want(SnapKind::Perpendicular) {
                        pts.clear();
                        geo::perp_feet(b, p, &mut pts);
                        for &x in &pts {
                            cands.push((SnapKind::Perpendicular, x, k, None));
                        }
                    }
                    if want(SnapKind::Tangent) {
                        pts.clear();
                        geo::tangents(b, p, &mut pts);
                        for &x in &pts {
                            cands.push((SnapKind::Tangent, x, k, None));
                        }
                    }
                }
            }
            if want(SnapKind::Nearest) {
                if let Some(x) = f.ps.iter().map(|p| geo::nearest([0.0, 0.0], p)).min_by(|a, b| a.0.total_cmp(&b.0)) {
                    if x.0 <= ap {
                        cands.push((SnapKind::Nearest, x.2, k, None));
                    }
                }
            }
        }
        if want(SnapKind::Intersection) {
            // (snap: an intersection farther than the nearest candidate of another kind cannot come first; what could merge
            // with that one, or with what merges with it, lies within a few `same`: a margin of a tenth of a pixel)
            let bound = if first {
                let m = cands.iter().filter(|c| c.0 != SnapKind::Nearest).map(|c| len(c.1)).fold(ap, f64::min);
                (m + 0.1 * v.scale + 16.0 * same).min(ap)
            } else {
                ap
            };
            let b2 = bound * bound;
            // pieces within the aperture's box (and that bound), swept by x: pairs whose boxes meet within it
            let mut pieces: Vec<(Bx, usize, usize)> = Vec::new();
            for (k, (_, f)) in near.iter().enumerate() {
                for (j, p) in f.ps.iter().enumerate() {
                    let b = p.bx();
                    if geo::bx_meets(&b, &rq) && geo::bx_dist2(&b, [0.0, 0.0]) <= b2 {
                        pieces.push((b, k, j));
                    }
                }
            }
            pieces.sort_unstable_by(|a, b| a.0[0].total_cmp(&b.0[0]));
            let eps = 1e-9 * ap.max(1e-300);
            let mut xs = Vec::new();
            for i in 0..pieces.len() {
                let (bi, ki, ji) = pieces[i];
                for &(bj, kj, jj) in &pieces[i + 1..] {
                    if bj[0] > bi[2] {
                        break;
                    }
                    if bj[1] > bi[3] || bi[1] > bj[3] {
                        continue;
                    }
                    if first && geo::bx_dist2(&[bj[0], bi[1].max(bj[1]), bi[2].min(bj[2]), bi[3].min(bj[3])], [0.0, 0.0]) > b2 {
                        continue;
                    }
                    if ki == kj {
                        // (a polyline with itself: not its neighbours, which meet at their vertex)
                        let f = &near[ki].1;
                        if !f.chain {
                            continue;
                        }
                        let (a, b) = (f.ks[ji].min(f.ks[jj]), f.ks[ji].max(f.ks[jj]));
                        if b == a + 1 || (f.closed && a == 0 && b + 1 == f.segs) {
                            continue;
                        }
                    }
                    xs.clear();
                    geo::intersect(&near[ki].1.ps[ji], &near[kj].1.ps[jj], eps, &mut xs);
                    for &x in &xs {
                        if len(x) <= ap {
                            cands.push((SnapKind::Intersection, x, ki, Some(kj)));
                        }
                    }
                }
            }
        }
        cands.sort_by(|a, b| len(a.1).total_cmp(&len(b.1)));
        let mut gone = vec![false; cands.len()];
        for i in 0..cands.len() {
            if gone[i] {
                continue;
            }
            let di = len(cands[i].1);
            for j in i + 1..cands.len() {
                if len(cands[j].1) - di > same {
                    break;
                }
                if !gone[j] && geo::dist(cands[i].1, cands[j].1) <= same && cands[i].0 != cands[j].0 {
                    if cands[j].0.rank() < cands[i].0.rank() {
                        gone[i] = true;
                        break;
                    }
                    gone[j] = true;
                }
            }
        }
        let mut k = 0;
        cands.retain(|_| {
            k += 1;
            !gone[k - 1]
        });
        // nearest the pointer first, then by kind; nearest last
        cands.sort_by(|a, b| {
            let na = a.0 == SnapKind::Nearest;
            let nb = b.0 == SnapKind::Nearest;
            na.cmp(&nb).then(len(a.1).total_cmp(&len(b.1))).then(a.0.rank().cmp(&b.0.rank()))
        });
        let tgt = |k: usize| -> Target {
            if k > usize::MAX / 2 {
                ins_targets[usize::MAX - k].clone()
            } else {
                near[k].0.clone()
            }
        };
        let mut out: Vec<Snap> = Vec::with_capacity(cands.len());
        let tol = same / v.scale;
        for (kind, p, k, other) in cands {
            let point = add(p, at);
            let distance = len(p) / v.scale;
            // (the same point of the same kind once: the same distance to a few ulps, so among the last ones that near it)
            if out.iter().rev().take_while(|s| (distance - s.distance).abs() <= tol).any(|s| s.kind == kind && s.point == point) {
                continue;
            }
            out.push(Snap { kind, point, distance, target: tgt(k), other: other.map(tgt) });
        }
        out
    }
}

/// the point at half a curve's length (absolute)
fn mid_by_length(cv: &Curve) -> Option<P> {
    let lens: Vec<f64> = cv.spans.iter().map(|s| geo::sp_length(s, 1.0)).collect();
    let total: f64 = lens.iter().sum();
    let mut want = 0.5 * total;
    for (s, l) in cv.spans.iter().zip(&lens) {
        if want <= *l {
            return Some(add(cv.origin, geo::sp_pt(s, geo::sp_at_length(s, want))));
        }
        want -= l;
    }
    None
}

// ---------------------------------------------------------------- grips

impl Scene {
    /// AutoCAD's grips of an entity on a layer (drawing units): a line's ends and middle, a polyline's vertices and its
    /// segments' middles, an arc's centre, ends and middle, a circle's centre and quadrants, an ellipse's centre and its
    /// axes' ends (an open one's ends too), a Bézier chain's and a spline's control points, a fill's corners, a text's
    /// alignment point, an insert's insertion point (each copy of an array).  Empty for a dead id or a block's entity
    pub fn grip_points(&self, id: EntityId) -> Vec<[f64; 2]> {
        let Ok(i) = self.get(id) else { return Vec::new() };
        let e = &self.ents[i];
        if e.block != NONE {
            return Vec::new();
        }
        let mut v: Vec<[f64; 2]> = Vec::new();
        let mid = |a: P, b: P| mul(add(a, b), 0.5);
        match &e.geom {
            Geom::Line(p) => v.extend_from_slice(&[p[0], p[1], mid(p[0], p[1])]),
            Geom::Poly(p, closed) => {
                let mut pts = p.clone();
                pts.dedup();
                v.extend_from_slice(&pts);
                let n = pts.len();
                let segs = if *closed && n > 2 { n } else { n.saturating_sub(1) };
                for k in 0..segs {
                    v.push(mid(pts[k], pts[(k + 1) % n]));
                }
            }
            &Geom::Arc(c, r, a0, sw) => {
                v.push(c);
                if sw.abs() >= TAU {
                    for d in [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] {
                        v.push([c[0] + r * d[0], c[1] + r * d[1]]);
                    }
                } else {
                    v.extend_from_slice(&[geo::arc_at(c, r, a0), geo::arc_at(c, r, a0 + 0.5 * sw), geo::arc_at(c, r, a0 + sw)]);
                }
            }
            Geom::Curve(g) => match g.src {
                super::CurveSrc::Ellipse { center, radii, rotation, start, sweep } => {
                    v.push(center);
                    let (sr, cr) = sincos(rotation);
                    let at = |t: f64| {
                        let (s, c) = sincos(t);
                        let (x, y) = (radii[0] * c, radii[1] * s);
                        [center[0] + cr * x - sr * y, center[1] + sr * x + cr * y]
                    };
                    for (k, d) in [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]].into_iter().enumerate() {
                        if geo::arc_has(start, sweep, d) {
                            v.push(at(k as f64 * core::f64::consts::FRAC_PI_2));
                        }
                    }
                    if sweep.abs() < TAU {
                        v.push(at(start));
                        v.push(at(start + sweep));
                    }
                }
                _ => {
                    let cv = &g.c;
                    for (j, s) in cv.spans.iter().enumerate() {
                        for k in 0..s.deg as usize + 1 {
                            if k == 0 && j > 0 {
                                continue;
                            }
                            v.push(add(cv.origin, [s.p[k][0] / s.p[k][2], s.p[k][1] / s.p[k][2]]));
                        }
                    }
                }
            },
            Geom::Fill(g) => {
                for r in &g.rings {
                    for s in &r.spans {
                        let n = s.deg as usize;
                        v.push(add(r.origin, [s.p[n][0] / s.p[n][2], s.p[n][1] / s.p[n][2]]));
                    }
                }
            }
            Geom::Text(t) => v.push(text_at(&t.src)),
            Geom::Insert(ins) => {
                for row in 0..ins.rows.max(1) {
                    for col in 0..ins.columns.max(1) {
                        v.push(copy_xf(ins, [0.0, 0.0], col, row).t);
                    }
                }
            }
        }
        v
    }
}
