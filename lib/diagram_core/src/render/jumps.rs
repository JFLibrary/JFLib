// jumps -- (JD21, CN-06) line jumps: where two dynamic connectors of a page cross, one of them
// hops over the other (an arc, a gap, a square, a bump of 2 to 7 sides); worked out while the
// display list is built, never written into a cell, never routed
//
//   PageJumps                    a page's settings and connectors: their straight pieces in paper
//                                inches in painter's order, a grid over the pieces, the jumps
//                                each connector got
//   builder.page_jumps(page, k)  (build) every connector of a page and its jumps, before the
//                                page's shapes are drawn (k: the index of its first top-level
//                                shape in the drawing)
//   builder.jumped(shape, line)  a connector's line (paper inches) with its jumps put in: own()
//                                draws that line, its shadow and its line ends
//   plan / refresh               (rebuild, affected) the connectors that cross a changed
//                                top-level shape's connectors before or after the change: their
//                                jumps change with it, so their top-level shapes are made anew
//   Jump, JumpStyle              one jump as drawn (drawing.jumps(): tests, diagramiotest)
//
// Visio's rules.  Documented (learn.microsoft.com, Page Layout and Shape Layout sections; the
// Visio 2007 XML reference, LineJumpCode element; Visio 2000 user's guide, "Displaying line
// jumps on crossing connectors"; support.microsoft.com, "Add or remove connector line jumps"):
// * Page: LineJumpCode which line jumps: 0 none, 1 horizontal lines, 2 vertical lines, 3 the last
//   routed line, 4 the last displayed line (the top one in the z-order), 5 the first displayed
//   line (the bottom one); "display order is the order of the Shape elements in the page".
//   LineJumpStyle 0 and 1 arc, 2 gap, 3 square, 4 .. 9 two to seven sides.  LineJumpFactorX / Y:
//   the size of the jumps on horizontal / vertical connectors relative to LineToLineX / Y (0 ..
//   10; LineToLineX 0.1 in with the factor 0.5 gives jumps of 0.05 in; Visio's own files: 0.125
//   in or 2.5 mm, the factor 2/3).  PageLineJumpDirX: horizontal connectors jump 1 up, 2 down;
//   PageLineJumpDirY: vertical ones 1 left, 2 right; 0 the default.
// * Connector: ConLineJumpCode 0 as the page says, 1 never, 2 always, 3 the other connector
//   jumps, 4 neither connector jumps; ConLineJumpStyle 0 the page's, 1 .. 9 as LineJumpStyle;
//   ConLineJumpDirX / Y 0 the page's, else as the page's cells (Developer › Behavior ›
//   Connector, "Line jumps").
// * Line jumps belong to dynamic connectors ("routable connectors display line jumps when they
//   cross other connectors"): only 1-D shapes with the routable flag in ObjType take part
//   (glue::is_connector), plain lines neither jump nor are jumped over.  Only the page shown
//   counts; the jumps are not part of the connector's geometry (the corpus' 104 connectors have
//   MoveTo / LineTo rows only).
// Assumed (no source found; the Visio check of JD21 shows them to the user):
// * The default directions are up (horizontal pieces) and left (vertical pieces); the docs'
//   text for value 0 names them swapped ("left" for X, "up" for Y).
// * The size is the jump's width along the line, its height half of it (the arc a half
//   circle); square: the box of that half circle; n sides: the corners on the half circle.
// * A piece is horizontal when it runs more along x than along y (slanted pieces of straight
//   connectors too); "horizontal lines": at a crossing the more horizontal piece jumps, at a tie
//   the upper one in the z-order; "vertical lines" alike.
// * "Last routed": Visio keeps no record of it in the file; the connector with the higher ID
//   (the one added later) jumps.
// * The connector codes: "neither" on either side: no jump; "always" on a side, or "other" on
//   the other side, makes that side jump; when both sides are asked, the page's rule decides
//   between them (else the upper one); a side with "never" never jumps; without such codes the
//   page's rule decides.  "Always" works on a page with LineJumpCode 0 too.
// * Only straight pieces jump and are jumped over (curved connectors and the rounded corners of
//   a connector with Rounding take no part); a jump that would reach over the end of its piece
//   (a bend, the line's end) is left out; a connector's end on another line (a T) is no
//   crossing, nor are lines that run along each other; a connector does not jump over itself.
// * Jumps on one piece closer than their width merge into one wider jump of the same height.
// * Connectors in groups take part like top-level ones (where they are drawn); shapes not drawn
//   (hidden layers, not printed, LinePattern 0) take no part.
// * Missing cells: LineJumpCode 1, LineJumpStyle 0, LineToLineX / Y 0.125 in, the factors 2/3,
//   the directions 0 (Visio's "No Style" holds these values).
// Lengths: the page layout cells are lengths on paper (as LineToNodeX for the router, glue.rs),
// the display list is in paper inches.
//
// Large pages: the pieces lie in a uniform grid (at most 64 x 64 cells), a crossing test looks
// only at the pieces in the cells of a piece's box; nothing compares all pairs.  A rebuild works
// out the changed shapes' connectors and the connectors they cross (before and after), nothing
// else.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use super::{Builder, Top};
use crate::document::{Owner, PageRef, ShapeRef, SheetRef};
use crate::geometry::{ceil, floor, sqrt, Path, Seg, Sub, Xform};
use crate::names;
use crate::shape::ShapeKind;

/// how a jump looks (LineJumpStyle)
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum JumpStyle {
    /// a half circle (0, 1)
    Arc,
    /// the line stops and goes on (2)
    Gap,
    /// three sides of a box (3)
    Square,
    /// 2 .. 7 sides with their corners on a half circle (4 .. 9)
    Sides(u8),
}

impl JumpStyle {
    /// LineJumpStyle / ConLineJumpStyle 1 .. 9 (0 and other values: the arc)
    pub fn from_code(v: u32) -> JumpStyle {
        match v {
            2 => JumpStyle::Gap,
            3 => JumpStyle::Square,
            4..=9 => JumpStyle::Sides((v - 2) as u8),
            _ => JumpStyle::Arc,
        }
    }
}

/// a jump as drawn
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Jump {
    /// the straight piece it lies on: Path::subs[sub].segs[seg] of the connector's line as
    /// drawn without jumps (paper inches)
    pub sub: u32,
    pub seg: u32,
    /// where it starts and ends on the piece (distances from the piece's start, inches)
    pub from: f64,
    pub to: f64,
    /// those two points
    pub start: [f64; 2],
    pub end: [f64; 2],
    /// the side it rises to (a unit vector across the piece) and how high (inches)
    pub normal: [f64; 2],
    pub height: f64,
    pub style: JumpStyle,
}

/// a page's line jump cells
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Settings {
    /// LineJumpCode 0 .. 5
    code: u8,
    /// LineJumpStyle 0 .. 9
    style: u8,
    /// the width of a jump on a horizontal and on a vertical piece (paper inches)
    size: [f64; 2],
    /// PageLineJumpDirX / Y (0 .. 2)
    dir: [u8; 2],
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { code: 1, style: 0, size: [0.125 * 2.0 / 3.0; 2], dir: [0; 2] }
    }
}

/// a straight piece of a connector's line
#[derive(Copy, Clone, Debug, PartialEq)]
struct Piece {
    a: [f64; 2],
    b: [f64; 2],
    sub: u32,
    seg: u32,
}

/// a connector of the page
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Conn {
    shape: ShapeRef,
    top: ShapeRef,
    /// painter's order: its top-level shape's index in the drawing, then its place under it
    key: (u32, u32),
    /// its ID (the "last routed" stand-in)
    id: u32,
    /// ConLineJumpCode 0 .. 4, ConLineJumpStyle 0 .. 9, ConLineJumpDirX / Y 0 .. 2
    code: u8,
    style: u8,
    dir: [u8; 2],
    pieces: Vec<Piece>,
    /// what it got
    jumps: Vec<Jump>,
}

/// the line jumps of a page
#[derive(Clone, Debug, Default)]
pub(crate) struct PageJumps {
    pub page: Option<PageRef>,
    set: Settings,
    /// in painter's order (ascending key)
    conns: Vec<Conn>,
    by_shape: BTreeMap<ShapeRef, u32>,
    grid: PieceGrid,
}

impl PageJumps {
    /// the connectors that got jumps, with them
    pub(crate) fn drawn(&self) -> impl Iterator<Item = (ShapeRef, &[Jump])> + '_ {
        self.conns.iter().filter(|c| !c.jumps.is_empty()).map(|c| (c.shape, c.jumps.as_slice()))
    }

    fn jumps_of(&self, shape: ShapeRef) -> Option<&[Jump]> {
        let i = *self.by_shape.get(&shape)? as usize;
        Some(self.conns.get(i)?.jumps.as_slice())
    }

    /// the map by shape and the grid after the connectors changed
    fn index(&mut self) {
        self.by_shape = self.conns.iter().enumerate().map(|(i, c)| (c.shape, i as u32)).collect();
        self.grid = PieceGrid::build(&self.conns);
    }

    /// connector i's jumps worked out anew
    fn compute(&mut self, i: usize) {
        let c = &self.conns[i];
        let mut found: Vec<(u32, f64)> = Vec::new();
        let mut hits = Vec::new();
        for (pi, p) in c.pieces.iter().enumerate() {
            self.grid.query(piece_box(p), &mut hits);
            for &e in &hits {
                let (cj, pj) = self.grid.entries[e as usize];
                if cj as usize == i {
                    continue;
                }
                let d = &self.conns[cj as usize];
                let q = &d.pieces[pj as usize];
                let Some(t) = cross(p.a, p.b, q.a, q.b) else { continue };
                if jumper(c, p, d, q, self.set.code) {
                    found.push((pi as u32, t));
                }
            }
        }
        let jumps = make(c, &self.set, found);
        self.conns[i].jumps = jumps;
    }

    /// the connectors (indices; not under the changed top-level shapes) that cross a connector
    /// under them as it was (the page's pieces) or as it is (`new`)
    fn partners(&self, changed: &BTreeSet<ShapeRef>, new: &[Conn]) -> BTreeSet<u32> {
        let mut out = BTreeSet::new();
        let mut hits = Vec::new();
        let olds = self.conns.iter().filter(|c| changed.contains(&c.top));
        for c in olds.chain(new.iter()) {
            for p in &c.pieces {
                self.grid.query(piece_box(p), &mut hits);
                for &e in &hits {
                    let (cj, pj) = self.grid.entries[e as usize];
                    let d = &self.conns[cj as usize];
                    if changed.contains(&d.top) || out.contains(&cj) {
                        continue;
                    }
                    let q = &d.pieces[pj as usize];
                    if cross(p.a, p.b, q.a, q.b).is_some() {
                        out.insert(cj);
                    }
                }
            }
        }
        out
    }

    /// the changed top-level shapes' connectors replaced by `new`, the jumps of those and of
    /// the partners (old indices) worked out anew
    fn update(&mut self, changed: &BTreeSet<ShapeRef>, new: Vec<Conn>, partners: &BTreeSet<u32>) {
        let again: BTreeSet<ShapeRef> = partners.iter().filter_map(|&i| self.conns.get(i as usize)).map(|c| c.shape).collect();
        self.conns.retain(|c| !changed.contains(&c.top));
        self.conns.extend(new);
        self.conns.sort_by_key(|c| c.key);
        self.index();
        for i in 0..self.conns.len() {
            if changed.contains(&self.conns[i].top) || again.contains(&self.conns[i].shape) {
                self.compute(i);
            }
        }
    }
}

/// per page with changed top-level shapes (index in `pages`): those shapes, their connectors
/// made anew, the connectors crossing them before or after (indices in that page's list)
pub(super) type Plan = Vec<(usize, BTreeSet<ShapeRef>, Vec<Conn>, BTreeSet<u32>)>;

/// what a rebuild of the top-level shapes `want` (indices in `tops`) changes in the line jumps
pub(super) fn plan(b: &mut Builder<'_, '_>, tops: &[Top], pages: &[PageJumps], want: &[usize]) -> Plan {
    let mut out = Vec::new();
    for (pi, pj) in pages.iter().enumerate() {
        let Some(page) = pj.page else { continue };
        let mine: Vec<usize> = want.iter().copied().filter(|&k| tops.get(k).is_some_and(|t| t.shape.owner == Owner::Page(page))).collect();
        if mine.is_empty() {
            continue;
        }
        b.context(page);
        let changed: BTreeSet<ShapeRef> = mine.iter().map(|&k| tops[k].shape).collect();
        let mut new = Vec::new();
        for &k in &mine {
            b.connectors_of(tops[k].shape, k as u32, &mut new);
        }
        let partners = pj.partners(&changed, &new);
        out.push((pi, changed, new, partners));
    }
    out
}

/// the top-level shapes (indices in `tops`) of the partners a plan names
pub(super) fn partner_tops(tops: &[Top], pages: &[PageJumps], plan: &Plan) -> Vec<usize> {
    let mut shapes: BTreeSet<ShapeRef> = BTreeSet::new();
    for (pi, _, _, partners) in plan {
        for &i in partners {
            if let Some(c) = pages[*pi].conns.get(i as usize) {
                shapes.insert(c.top);
            }
        }
    }
    if shapes.is_empty() {
        return Vec::new();
    }
    tops.iter().enumerate().filter(|(_, t)| shapes.contains(&t.shape)).map(|(k, _)| k).collect()
}

/// the plan carried out on the pages' jumps
pub(super) fn refresh(pages: &mut [PageJumps], plan: Plan) {
    for (pi, changed, new, partners) in plan {
        pages[pi].update(&changed, new, &partners);
    }
}

impl Builder<'_, '_> {
    /// a page's line jump cells (the page's own, else its styles', else Visio's values)
    fn jump_settings(&self, page: PageRef) -> Settings {
        let ps = SheetRef::Page(page);
        let n = |c: &str| self.num(ps, c);
        let code = |c: &str, max: f64, d: u8| n(c).map_or(d, |v| v.clamp(0.0, max) as u8);
        let factor = |c: &str| n(c).map_or(2.0 / 3.0, |v| v.clamp(0.0, 10.0));
        let spacing = |c: &str| n(c).filter(|v| *v >= 0.0).map_or(0.125, |v| v.min(100.0));
        Settings {
            code: code("LineJumpCode", 5.0, 1),
            style: code("LineJumpStyle", 9.0, 0),
            size: [factor("LineJumpFactorX") * spacing("LineToLineX"), factor("LineJumpFactorY") * spacing("LineToLineY")],
            dir: [code("PageLineJumpDirX", 2.0, 0), code("PageLineJumpDirY", 2.0, 0)],
        }
    }

    /// every connector of a page with its jumps (k: the index of the page's first top-level
    /// shape in the drawing); the context is the page's
    pub(super) fn page_jumps(&mut self, page: PageRef, k: u32) -> PageJumps {
        let set = self.jump_settings(page);
        let mut conns = Vec::new();
        let tops: Vec<ShapeRef> = self.doc.top_shapes(Owner::Page(page)).collect();
        for (i, r) in tops.into_iter().enumerate() {
            self.connectors_of(r, k + i as u32, &mut conns);
        }
        let mut pj = PageJumps { page: Some(page), set, conns, ..PageJumps::default() };
        pj.index();
        for i in 0..pj.conns.len() {
            pj.compute(i);
        }
        pj
    }

    /// the connectors a top-level shape draws (itself, its subshapes), in the order shape()
    /// draws them (k: its index in the drawing); the context is its page's
    pub(super) fn connectors_of(&self, top: ShapeRef, k: u32, out: &mut Vec<Conn>) {
        let m = Xform::scale(self.scale, self.scale);
        let mut seq = 0u32;
        self.walk(top, &m, 0, (top, k), &mut seq, out);
    }

    // shape()'s way through a shape and its subshapes: what it leaves out, what it draws
    fn walk(&self, r: ShapeRef, parent: &Xform, depth: u32, top: (ShapeRef, u32), seq: &mut u32, out: &mut Vec<Conn>) {
        let doc = self.doc;
        let Some(s) = doc.shape(r) else { return };
        let header = s.header();
        if header.del || depth > 256 || matches!(header.kind, ShapeKind::Unspecified | ShapeKind::Guide) {
            return;
        }
        let sheet = SheetRef::Shape(r);
        let m = doc.local_xform(sheet).then(parent);
        let (visible, _) = self.layer_state(r);
        let children: Vec<ShapeRef> = s.children().collect();
        if header.kind == ShapeKind::Group || !children.is_empty() {
            let mode = self.num(sheet, "DisplayMode").unwrap_or(2.0) as u32;
            if visible && mode == 1 {
                self.conn(r, &m, top, seq, out);
            }
            for c in children {
                self.walk(c, &m, depth + 1, top, seq, out);
            }
            if visible && mode != 1 && mode != 0 {
                self.conn(r, &m, top, seq, out);
            }
        } else if visible {
            self.conn(r, &m, top, seq, out);
        }
    }

    // a shape drawn: a connector when it is one and draws a line
    fn conn(&self, r: ShapeRef, m: &Xform, top: (ShapeRef, u32), seq: &mut u32, out: &mut Vec<Conn>) {
        let doc = self.doc;
        *seq += 1;
        if !crate::glue::is_connector(doc, r) {
            return;
        }
        let sheet = SheetRef::Shape(r);
        if self.num(sheet, "LinePattern").unwrap_or(1.0) as u32 == 0 {
            return;
        }
        let w = self.num_sym(sheet, names::WIDTH).unwrap_or(0.0);
        let h = self.num_sym(sheet, names::HEIGHT).unwrap_or(0.0);
        let (_, line) = self.outlines(sheet, w, h);
        let line = line.map(m);
        let mut pieces = Vec::new();
        for (si, sub) in line.subs.iter().enumerate() {
            let mut prev = sub.start;
            for (gi, seg) in sub.segs.iter().enumerate() {
                if let Seg::Line(q) = *seg {
                    if dist(prev, q) > 1e-9 && prev.iter().chain(q.iter()).all(|v| v.is_finite()) {
                        pieces.push(Piece { a: prev, b: q, sub: si as u32, seg: gi as u32 });
                    }
                }
                prev = seg.end();
            }
        }
        if pieces.is_empty() {
            return;
        }
        let code = |c: &str, max: f64| self.num(sheet, c).map_or(0, |v| v.clamp(0.0, max) as u8);
        out.push(Conn {
            shape: r,
            top: top.0,
            key: (top.1, *seq),
            id: doc.shape(r).map_or(0, |s| s.id()),
            code: code("ConLineJumpCode", 4.0),
            style: code("ConLineJumpStyle", 9.0),
            dir: [code("ConLineJumpDirX", 2.0), code("ConLineJumpDirY", 2.0)],
            pieces,
            jumps: Vec::new(),
        });
    }

    /// a connector's line (paper inches) with the jumps it got on the page being built
    pub(super) fn jumped(&self, r: ShapeRef, line: Path) -> Path {
        match self.cur_jumps.and_then(|i| self.jumps.get(i)).and_then(|p| p.jumps_of(r)) {
            Some(js) if !js.is_empty() => apply(&line, js),
            _ => line,
        }
    }
}

// ---------------------------------------------------------------- who jumps

/// whether connector a jumps at the crossing of its piece pa with b's piece pb
fn jumper(a: &Conn, pa: &Piece, b: &Conn, pb: &Piece, page_code: u8) -> bool {
    if a.code == 4 || b.code == 4 {
        return false;
    }
    let asked = matches!(a.code, 2 | 3) || matches!(b.code, 2 | 3);
    if asked {
        let wa = (a.code == 2 || b.code == 3) && a.code != 1;
        let wb = (b.code == 2 || a.code == 3) && b.code != 1;
        return match (wa, wb) {
            (true, false) => true,
            (true, true) => page_pick(a, pa, b, pb, page_code).unwrap_or(a.key > b.key),
            _ => false,
        };
    }
    page_pick(a, pa, b, pb, page_code) == Some(true) && a.code != 1
}

/// the page's rule (LineJumpCode): Some(whether a jumps), None: no jump
fn page_pick(a: &Conn, pa: &Piece, b: &Conn, pb: &Piece, code: u8) -> Option<bool> {
    let (ua, ub) = (unit(pa), unit(pb));
    let upper = a.key > b.key;
    let less = |x: f64, y: f64| if (x - y).abs() > 1e-9 { x < y } else { upper };
    match code {
        // the more horizontal piece (the smaller y part)
        1 => Some(less(ua[1].abs(), ub[1].abs())),
        // the more vertical piece
        2 => Some(less(ua[0].abs(), ub[0].abs())),
        3 => Some(if a.id != b.id { a.id > b.id } else { upper }),
        4 => Some(upper),
        5 => Some(!upper),
        _ => None,
    }
}

/// where the pieces p-q and r-s cross: the parameter on p-q; None: they do not cross inside
/// both (parallel, along each other, one ends on the other or beyond it)
fn cross(p: [f64; 2], q: [f64; 2], r: [f64; 2], s: [f64; 2]) -> Option<f64> {
    const TOL: f64 = 1e-6;
    let d1 = [q[0] - p[0], q[1] - p[1]];
    let d2 = [s[0] - r[0], s[1] - r[1]];
    let den = d1[0] * d2[1] - d1[1] * d2[0];
    let (l1, l2) = (dist(p, q), dist(r, s));
    if !(den.abs() > 1e-9 * l1 * l2) {
        return None;
    }
    let w = [r[0] - p[0], r[1] - p[1]];
    let t = (w[0] * d2[1] - w[1] * d2[0]) / den;
    let u = (w[0] * d1[1] - w[1] * d1[0]) / den;
    let inside = |k: f64, l: f64| k * l > TOL && (1.0 - k) * l > TOL;
    (inside(t, l1) && inside(u, l2)).then_some(t)
}

/// the jumps of connector c from its crossings (piece, parameter on it)
fn make(c: &Conn, set: &Settings, mut found: Vec<(u32, f64)>) -> Vec<Jump> {
    const TOL: f64 = 1e-6;
    let mut out = Vec::new();
    if found.is_empty() {
        return out;
    }
    found.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.partial_cmp(&y.1).unwrap_or(core::cmp::Ordering::Equal)));
    let style = JumpStyle::from_code(if c.style != 0 { c.style } else { set.style } as u32);
    let mut k = 0;
    while k < found.len() {
        let pi = found[k].0;
        let mut e = k;
        while e < found.len() && found[e].0 == pi {
            e += 1;
        }
        let p = &c.pieces[pi as usize];
        let len = dist(p.a, p.b);
        let u = unit(p);
        let flat = u[0].abs() >= u[1].abs();
        let w = set.size[if flat { 0 } else { 1 }];
        if w > 1e-9 && len > 0.0 {
            let half = w * 0.5;
            // the side: up / down on a horizontal piece, left / right on a vertical one
            let left = [-u[1], u[0]];
            let normal = if flat {
                let up = (if c.dir[0] != 0 { c.dir[0] } else { set.dir[0] }) != 2;
                if (left[1] > 0.0) == up { left } else { [-left[0], -left[1]] }
            } else {
                let leftward = (if c.dir[1] != 0 { c.dir[1] } else { set.dir[1] }) != 2;
                if (left[0] < 0.0) == leftward { left } else { [-left[0], -left[1]] }
            };
            // each jump that fits on the piece, the overlapping ones merged
            let mut spans: Vec<(f64, f64)> = Vec::new();
            for &(_, t) in &found[k..e] {
                let at = t * len;
                let (from, to) = (at - half, at + half);
                if from <= TOL || to >= len - TOL {
                    continue;
                }
                match spans.last_mut() {
                    Some(last) if from <= last.1 + TOL => last.1 = last.1.max(to),
                    _ => spans.push((from, to)),
                }
            }
            for (from, to) in spans {
                let at = |d: f64| [p.a[0] + u[0] * d, p.a[1] + u[1] * d];
                out.push(Jump { sub: p.sub, seg: p.seg, from, to, start: at(from), end: at(to), normal, height: half, style });
            }
        }
        k = e;
    }
    out
}

// ---------------------------------------------------------------- drawing them

/// a line with its jumps put into its straight pieces
pub(crate) fn apply(path: &Path, jumps: &[Jump]) -> Path {
    let mut out = Path { subs: Vec::with_capacity(path.subs.len() + 1) };
    for (si, sub) in path.subs.iter().enumerate() {
        let mine: Vec<&Jump> = jumps.iter().filter(|j| j.sub as usize == si).collect();
        if mine.is_empty() {
            out.subs.push(sub.clone());
            continue;
        }
        let mut cur = Sub { start: sub.start, segs: Vec::with_capacity(sub.segs.len() + 4 * mine.len()), closed: false };
        let mut split = false;
        for (gi, seg) in sub.segs.iter().enumerate() {
            let Seg::Line(q) = *seg else {
                cur.segs.push(*seg);
                continue;
            };
            let mut here: Vec<&Jump> = mine.iter().copied().filter(|j| j.seg as usize == gi).collect();
            here.sort_by(|a, b| a.from.partial_cmp(&b.from).unwrap_or(core::cmp::Ordering::Equal));
            for j in here {
                cur.segs.push(Seg::Line(j.start));
                if j.style == JumpStyle::Gap {
                    out.subs.push(core::mem::take(&mut cur));
                    cur.start = j.end;
                    split = true;
                } else {
                    bump(j, &mut cur.segs);
                }
            }
            cur.segs.push(Seg::Line(q));
        }
        cur.closed = sub.closed && !split;
        out.subs.push(cur);
    }
    out
}

/// the pieces of a jump from its start to its end (not a gap)
fn bump(j: &Jump, segs: &mut Vec<Seg>) {
    let a = (j.to - j.from) * 0.5;
    if !(a > 0.0) {
        segs.push(Seg::Line(j.end));
        return;
    }
    let u = [(j.end[0] - j.start[0]) / (2.0 * a), (j.end[1] - j.start[1]) / (2.0 * a)];
    let (n, h) = (j.normal, j.height);
    let c = [(j.start[0] + j.end[0]) * 0.5, (j.start[1] + j.end[1]) * 0.5];
    // a point by its parts along the line and across it, from the middle
    let at = |x: f64, y: f64| [c[0] + u[0] * x + n[0] * y, c[1] + u[1] * x + n[1] * y];
    match j.style {
        JumpStyle::Arc => {
            // a half ellipse a wide (each way) and h high in two quarter curves
            const K: f64 = 0.552_284_749_830_793_4;
            segs.push(Seg::Cubic(at(-a, K * h), at(-K * a, h), at(0.0, h)));
            segs.push(Seg::Cubic(at(K * a, h), at(a, K * h), at(a, 0.0)));
        }
        JumpStyle::Square => {
            segs.push(Seg::Line(at(-a, h)));
            segs.push(Seg::Line(at(a, h)));
            segs.push(Seg::Line(at(a, 0.0)));
        }
        JumpStyle::Sides(k) => {
            let k = k.clamp(2, 7) as u32;
            for i in 1..k {
                let (s, co) = crate::geometry::sincos(core::f64::consts::PI * i as f64 / k as f64);
                segs.push(Seg::Line(at(-a * co, h * s)));
            }
            segs.push(Seg::Line(at(a, 0.0)));
        }
        JumpStyle::Gap => segs.push(Seg::Line(j.end)),
    }
}

// ---------------------------------------------------------------- the grid of pieces

const CELLS: usize = 64;
const BIG: usize = 512;

/// a uniform grid over the pieces of a page's connectors
#[derive(Clone, Debug, Default)]
struct PieceGrid {
    origin: [f64; 2],
    cell: [f64; 2],
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
    /// entries over more than BIG cells: every query looks at them
    big: Vec<u32>,
    /// (connector, piece) of each entry
    entries: Vec<(u32, u32)>,
    boxes: Vec<[f64; 4]>,
}

impl PieceGrid {
    fn build(conns: &[Conn]) -> PieceGrid {
        let mut bounds: Option<[f64; 4]> = None;
        for c in conns {
            for p in &c.pieces {
                let b = piece_box(p);
                bounds = Some(bounds.map_or(b, |a| super::union(a, b)));
            }
        }
        let Some(bb) = bounds else { return PieceGrid::default() };
        let w = (bb[2] - bb[0]).max(1e-6);
        let h = (bb[3] - bb[1]).max(1e-6);
        let side = w.max(h) / CELLS as f64;
        let nx = (ceil(w / side) as usize).clamp(1, CELLS);
        let ny = (ceil(h / side) as usize).clamp(1, CELLS);
        let mut g = PieceGrid { origin: [bb[0], bb[1]], cell: [w / nx as f64, h / ny as f64], nx, ny, cells: alloc::vec![Vec::new(); nx * ny], ..PieceGrid::default() };
        for (ci, c) in conns.iter().enumerate() {
            for (pi, p) in c.pieces.iter().enumerate() {
                let e = g.entries.len() as u32;
                let b = piece_box(p);
                g.entries.push((ci as u32, pi as u32));
                g.boxes.push(b);
                let (x0, y0, x1, y1) = g.range(b);
                if (x1 - x0 + 1) * (y1 - y0 + 1) > BIG {
                    g.big.push(e);
                    continue;
                }
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        g.cells[y * nx + x].push(e);
                    }
                }
            }
        }
        g
    }

    // the cells a box touches, clamped to the grid
    fn range(&self, b: [f64; 4]) -> (usize, usize, usize, usize) {
        let cx = |x: f64| (floor((x - self.origin[0]) / self.cell[0]).max(0.0) as usize).min(self.nx - 1);
        let cy = |y: f64| (floor((y - self.origin[1]) / self.cell[1]).max(0.0) as usize).min(self.ny - 1);
        (cx(b[0]), cy(b[1]), cx(b[2]), cy(b[3]))
    }

    /// the entries whose boxes touch b, ascending
    fn query(&self, b: [f64; 4], out: &mut Vec<u32>) {
        out.clear();
        if self.nx == 0 || !b.iter().all(|v| v.is_finite()) {
            return;
        }
        let touches = |e: u32| {
            let o = &self.boxes[e as usize];
            o[0] <= b[2] && o[2] >= b[0] && o[1] <= b[3] && o[3] >= b[1]
        };
        let (x0, y0, x1, y1) = self.range(b);
        for y in y0..=y1 {
            for x in x0..=x1 {
                out.extend(self.cells[y * self.nx + x].iter().copied().filter(|&e| touches(e)));
            }
        }
        out.extend(self.big.iter().copied().filter(|&e| touches(e)));
        out.sort_unstable();
        out.dedup();
    }
}

fn piece_box(p: &Piece) -> [f64; 4] {
    [p.a[0].min(p.b[0]), p.a[1].min(p.b[1]), p.a[0].max(p.b[0]), p.a[1].max(p.b[1])]
}

fn unit(p: &Piece) -> [f64; 2] {
    let l = dist(p.a, p.b);
    if l > 0.0 {
        [(p.b[0] - p.a[0]) / l, (p.b[1] - p.a[1]) / l]
    } else {
        [1.0, 0.0]
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    sqrt((a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]))
}

#[cfg(test)]
#[path = "jumps_tests.rs"]
mod tests;
