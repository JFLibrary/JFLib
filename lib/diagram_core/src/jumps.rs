// jumps -- (JD21, CN-06) line jumps: where two dynamic connectors of a page cross, one of them
// hops over the other (an arc, a gap, a square, a bump of 2 to 7 sides).  As in Visio they are
// rows of the connector's Geometry, written when connectors are laid (routed): the connector laid
// and every connector it crosses or crossed get their jumps anew.  Nothing is worked out while
// drawing and nothing when a file opens -- a file shows what its Geometry holds.
//
//   Laid / laid.note(doc, c)      the connectors a gesture lays, in the order it lays them
//   lay(tx, eng, &laid)           (glue::run, route_edit::set_way) after they are laid: their
//                                 jumps and those of every connector they cross, and of those
//                                 whose jumps the crossings no longer explain (a connector they
//                                 jumped over went away)
//   lay_again(tx, eng, &conns)    these connectors laid again where they are (the app routes
//                                 them instead, as Visio does when their own cells change)
//   lay_page(tx, eng, page)       every connector of the page laid again, in z-order (the page's
//                                 line jump cells changed: Seite einrichten, Linienübergänge
//                                 anzeigen)
//   forget(tx, eng, &deleted)     (glue::detach) before shapes are deleted: the jumps over their
//                                 connectors go
//   way(doc, c) -> Option<points> a connector's way on the page without its jumps (None: it has
//                                 none): glue::path and route_edit read ways through it, so a
//                                 file with jumps gives the same handles, ways and routes
//   read(doc, c) -> Vec<Jump>     the jumps a connector's Geometry holds (tests, diagramiotest)
//   Jump, JumpStyle
//
// Visio's rows (Visio 16 checked, JD21v: visio-run2 [jumps], visio-run3 [jumps2], visio-run4
// [jumprows], visio-jumps-1): a connector that jumps has, on the straight piece crossed, a LineTo
// to the jump's start and one row to its end -- ArcTo with the bow (A) half the jump's width
// (style 0 and 1), MoveTo (2, the gap) or PolylineTo with A = POLYLINE(0, 0, x / Width, y /
// Height, ...) through the bump's corners (3 square, 4 .. 9 two to seven sides); the jumps of a
// piece in the order the piece runs.  The corners in units of half the width, along the piece from the jump's start
// and across it, are Visio's own (not exactly on a half circle): square (0, 1) (2, 1); 2 sides
// (1, 1); 3 (0.5, 0.86) (1.5, 0.86); 4 (0.3, 0.7) (1, 1) (1.7, 0.7); 5 (0.2, 0.58) (0.7, 0.96)
// (1.3, 0.96) (1.8, 0.58); 6 (0.14, 0.5) (0.5, 0.86) (1, 1) (1.5, 0.86) (1.86, 0.5); 7 (0.1,
// 0.44) (0.38, 0.78) (0.78, 0.98) (1.22, 0.98) (1.62, 0.78) (1.9, 0.44).  Visio writes the rows'
// numbers as constants in the drawing's unit; here they are constants in inches.
// Visio's rules -- belegt by the check:
// * When a connector is laid (new, moved, an end changed), Visio works out the jumps of it and
//   of every connector it crosses: partners gain or lose their jumps over it (all of a
//   partner's jumps by the rule, not only the one over it).  A connector deleted or moved away
//   takes the jumps over it along.  A page's line jump cell changed: every connector of the page
//   gets its jumps anew where it is (no routing); a connector's ConLineJumpCode changed: Visio
//   routes it again.  A file opened shows its rows (Visio draws the jumps of a file of ours as
//   they are, and lays them anew to the same rows, 126 of 135 connectors; the others below).
// * LineJumpCode (the page): 0 none; 1 at a crossing the more horizontal piece jumps (the
//   smaller |dy / dx|; equal slopes, as two 45 degree lines: neither); 2 the more vertical one;
//   3 the one laid last (it jumps over its partners, they lose their jumps over it); 4 the upper
//   one in the z-order; 5 the lower one.  Missing: 1.  (Codes 4 and 5 in a file Visio has just
//   opened: no connector jumps over another until a shape is brought to the front -- Visio's
//   display order of the shapes read is not known before; here the z-order counts from the
//   start.)
// * LineJumpStyle 0 and 1 arc, 2 gap, 3 square, 4 .. 9 two to seven sides.  Missing: 0.
// * The width: LineJumpFactorX x LineToLineX on pieces that run more along x (LineJumpFactorY x
//   LineToLineY on the others), the jump centred on the crossing, as high as half its width.
//   Missing: the factors 2/3, LineToLineX / Y 2.5 mm (a page without the cells: 0.0656 in).
// * Jumps closer than their width merge into one from the first one's start to the last one's
//   end, as high as half of that (the arc a half circle again).
// * PageLineJumpDirX: pieces that run along x jump 0 / 1 up, 2 down; PageLineJumpDirY: the
//   others 0 / 1 left, 2 right.
// * ConLineJumpCode (the connector): 0 the page's, 1 never, 2 always, 3 the other connector
//   jumps, 4 neither; ConLineJumpStyle 0 the page's, else as LineJumpStyle; ConLineJumpDirX / Y
//   0 the page's, else as the page's cells.  "Always" works on a page with LineJumpCode 0.
// Angenommen (no source; earlier assumptions kept):
// * "Last laid" for connectors not laid now keeps what their rows show; a whole page laid
//   anew (lay_page) is laid in z-order.
// * The conflicts of the connector codes: "neither" on either side: no jump; "always" on a side,
//   or "other" on the other side, makes that side jump; asked on both sides, the page's rule
//   decides (on a tie the upper one); "never" never jumps.
// * Only straight pieces of connectors with MoveTo and LineTo rows (and jump rows) take part
//   (curved connectors neither jump nor are jumped over); a jump that would reach over the end of
//   its piece (a bend, the end) is left out; a connector's end on another line (a T) is no
//   crossing, nor are lines along each other; no connector jumps over itself.  Rounding does
//   not move the jumps (they lie on the way without it).
// * Connectors in groups take part (where they lie on the page); connectors without a line
//   (LinePattern 0) or on hidden layers only do not.
// Lengths: the page's cells are lengths on paper (as LineToNodeX for the router); the ways are
// in drawing units on the page, the widths turned into drawing units by the drawing scale.
//
// Large pages: the pieces lie in a uniform grid (at most 64 x 64 cells), a crossing test looks
// only at the pieces in the cells of a piece's box; nothing compares all pairs.  A connector's
// rows are written only when they change.  Who crossed a laid connector before is not known
// (its cells have moved when it is laid): a jump with no crossing under it, or one wider on one
// side of its crossings than on the other (a merged jump that lost one), is laid anew.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cell::Value;
use crate::document::{Document, Owner, PageRef, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::geometry::{ceil, floor, parse_args, sqrt, Xform};
use crate::glue::{self, Cv, GeoRow};
use crate::names;
use crate::shape::ShapeKind;
use crate::sheet::CellAddr;
use crate::strings::Sym;

/// how a jump looks (LineJumpStyle)
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum JumpStyle {
    /// a half circle (0, 1)
    Arc,
    /// the line stops and goes on (2)
    Gap,
    /// three sides of a box (3)
    Square,
    /// 2 .. 7 sides (4 .. 9)
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

    /// the corners of the bump in units of half the jump's width: along the piece from the
    /// jump's start, across it (Visio's numbers)
    pub fn corners(self) -> &'static [[f64; 2]] {
        match self {
            JumpStyle::Square => &[[0.0, 1.0], [2.0, 1.0]],
            JumpStyle::Sides(2) => &[[1.0, 1.0]],
            JumpStyle::Sides(3) => &[[0.5, 0.86], [1.5, 0.86]],
            JumpStyle::Sides(4) => &[[0.3, 0.7], [1.0, 1.0], [1.7, 0.7]],
            JumpStyle::Sides(5) => &[[0.2, 0.58], [0.7, 0.96], [1.3, 0.96], [1.8, 0.58]],
            JumpStyle::Sides(6) => &[[0.14, 0.5], [0.5, 0.86], [1.0, 1.0], [1.5, 0.86], [1.86, 0.5]],
            JumpStyle::Sides(7) => &[[0.1, 0.44], [0.38, 0.78], [0.78, 0.98], [1.22, 0.98], [1.62, 0.78], [1.9, 0.44]],
            _ => &[],
        }
    }
}

/// a jump in a connector's Geometry
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Jump {
    /// where it starts and ends on the connector's way (page coordinates)
    pub start: [f64; 2],
    pub end: [f64; 2],
    /// the side it rises to (a unit vector across the way; a gap: 0, 0) and how high (drawing
    /// units; a gap: half its width)
    pub normal: [f64; 2],
    pub height: f64,
    pub style: JumpStyle,
}

impl Jump {
    /// its width along the way
    pub fn width(&self) -> f64 {
        dist(self.start, self.end)
    }
}

// ---------------------------------------------------------------- reading rows

// a row of a connector's first Geometry section
#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Move,
    Line,
    /// ArcTo with its bow (A)
    Arc(f64),
    /// PolylineTo with its points (local coordinates)
    Poly(Vec<[f64; 2]>),
    /// anything else (a curve): no jumps there
    Other,
}

fn num(doc: &Document, sheet: SheetRef, cell: &str) -> Option<f64> {
    let c = doc.sym(cell)?;
    doc.resolve(sheet, &CellAddr::single(c)).num().filter(|v| v.is_finite())
}

fn num_sym(doc: &Document, sheet: SheetRef, cell: Sym) -> Option<f64> {
    doc.resolve(sheet, &CellAddr::single(cell)).num().filter(|v| v.is_finite())
}

// the rows of a shape's first Geometry section (the one glue writes): kind and point (local)
fn rows_of(doc: &Document, c: ShapeRef) -> Option<Vec<(Kind, [f64; 2])>> {
    let sheet = SheetRef::Shape(c);
    let sk = *doc.geometry(sheet).first()?;
    let (w, h) = (num_sym(doc, sheet, names::WIDTH).unwrap_or(0.0), num_sym(doc, sheet, names::HEIGHT).unwrap_or(0.0));
    let rows = doc.rows(sheet, sk);
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let cell = |s: Sym| doc.resolve(sheet, &CellAddr::row(sk, r.key, s));
        let p = [cell(names::X).num().unwrap_or(0.0), cell(names::Y).num().unwrap_or(0.0)];
        let kind = match r.kind {
            Some(k) if k == names::MOVE_TO => Kind::Move,
            Some(k) if k == names::LINE_TO => Kind::Line,
            Some(k) if k == names::ARC_TO => Kind::Arc(cell(names::A).num().unwrap_or(0.0)),
            Some(k) if k == names::POLYLINE_TO => {
                let text = match cell(names::A).value() {
                    Value::Text(s) => Some(doc.str(s)),
                    _ => None,
                };
                match text.and_then(|t| parse_args(t, "POLYLINE")) {
                    Some(args) if args.len() >= 2 => {
                        let (rx, ry) = (args[0] == 0.0, args[1] == 0.0);
                        Kind::Poly(args[2..].chunks_exact(2).map(|q| [if rx { q[0] * w } else { q[0] }, if ry { q[1] * h } else { q[1] }]).collect())
                    }
                    _ => Kind::Other,
                }
            }
            _ => Kind::Other,
        };
        if !(p[0].is_finite() && p[1].is_finite()) {
            return None;
        }
        out.push((kind, p));
    }
    Some(out)
}

// a connector's rows taken apart: its way without jumps and the jumps on it
#[derive(Clone, Debug)]
struct Parsed {
    /// the way's points (local)
    base: Vec<[f64; 2]>,
    /// the jumps: the piece they lie on (from base[k]), start, end (local), the row
    jumps: Vec<(usize, [f64; 2], [f64; 2], Kind)>,
}

// the rows as a way with jumps (None: no way of straight pieces, or rows that are no jumps)
fn parse(rows: &[(Kind, [f64; 2])]) -> Option<Parsed> {
    let mut base: Vec<[f64; 2]> = Vec::with_capacity(rows.len());
    let mut jumps = Vec::new();
    // the end of the jump just before (the next row starts there)
    let mut after: Option<[f64; 2]> = None;
    for (i, (kind, p)) in rows.iter().enumerate() {
        match kind {
            Kind::Other => return None,
            Kind::Move | Kind::Line if i == 0 => base.push(*p),
            Kind::Line => {
                base.push(*p);
                after = None;
            }
            k => {
                // a jump from the point before it
                let s = match after {
                    Some(e) => e,
                    None => {
                        if base.len() < 2 {
                            return None;
                        }
                        base.pop()?
                    }
                };
                jumps.push((base.len() - 1, s, *p, k.clone()));
                after = Some(*p);
            }
        }
    }
    if base.len() < 2 || after.is_some() {
        return None;
    }
    // every jump lies on its piece
    for (k, s, e, _) in &jumps {
        let (a, b) = (base[*k], base[*k + 1]);
        if !(on_piece(*s, a, b) && on_piece(*e, a, b)) {
            return None;
        }
    }
    Some(Parsed { base, jumps })
}

// whether p lies on the piece a-b
fn on_piece(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> bool {
    let l = dist(a, b);
    if !(l > 0.0) {
        return false;
    }
    let u = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
    let d = [p[0] - a[0], p[1] - a[1]];
    let along = d[0] * u[0] + d[1] * u[1];
    let across = d[0] * u[1] - d[1] * u[0];
    let tol = 1e-6 * (1.0 + l);
    across.abs() <= tol && along >= -tol && along <= l + tol
}

/// a connector's way on the page without its line jumps (None: no connector, or no jump rows:
/// read its Geometry as it is)
pub fn way(doc: &Document, c: ShapeRef) -> Option<Vec<[f64; 2]>> {
    if !glue::is_connector(doc, c) {
        return None;
    }
    let p = parse(&rows_of(doc, c)?)?;
    if p.jumps.is_empty() {
        return None;
    }
    let m = doc.page_xform(c);
    Some(p.base.iter().map(|q| m.apply(*q)).collect())
}

/// the line jumps a connector's Geometry holds, in the order of its rows
pub fn read(doc: &Document, c: ShapeRef) -> Vec<Jump> {
    let Some(p) = rows_of(doc, c).and_then(|r| parse(&r)) else { return Vec::new() };
    let m = doc.page_xform(c);
    let mut out = Vec::with_capacity(p.jumps.len());
    for (_, s, e, kind) in &p.jumps {
        let (ps, pe) = (m.apply(*s), m.apply(*e));
        let l = dist(*s, *e);
        if !(l > 0.0) {
            continue;
        }
        let mid = [(s[0] + e[0]) * 0.5, (s[1] + e[1]) * 0.5];
        // the farthest point of the bump (local)
        let (style, apex) = match kind {
            Kind::Move => (JumpStyle::Gap, None),
            Kind::Arc(bow) => {
                // a positive bow: the middle of the arc lies right of the chord
                let right = [(e[1] - s[1]) / l, -(e[0] - s[0]) / l];
                (JumpStyle::Arc, Some([mid[0] + right[0] * bow, mid[1] + right[1] * bow]))
            }
            Kind::Poly(q) => {
                let u = [(e[0] - s[0]) / l, (e[1] - s[1]) / l];
                let across = |p: &[f64; 2]| ((p[0] - s[0]) * u[1] - (p[1] - s[1]) * u[0]).abs();
                let far = q.iter().copied().max_by(|a, b| across(a).total_cmp(&across(b)));
                let first_along = q.first().map_or(1.0, |p| ((p[0] - s[0]) * u[0] + (p[1] - s[1]) * u[1]) / (l * 0.5));
                let style = if q.len() == 2 && first_along.abs() < 0.01 { JumpStyle::Square } else { JumpStyle::Sides((q.len() + 1).min(7) as u8) };
                (style, far)
            }
            _ => continue,
        };
        let (normal, height) = match apex.map(|a| m.apply(a)) {
            Some(a) => {
                let u = [(pe[0] - ps[0]) / l, (pe[1] - ps[1]) / l];
                let left = [-u[1], u[0]];
                let off = (a[0] - ps[0]) * left[0] + (a[1] - ps[1]) * left[1];
                let n = if off >= 0.0 { left } else { [-left[0], -left[1]] };
                (n, off.abs())
            }
            None => ([0.0, 0.0], l * 0.5),
        };
        out.push(Jump { start: ps, end: pe, normal, height, style });
    }
    out
}

// ---------------------------------------------------------------- the page's connectors

// Visio's LineToLineX / Y when the page lacks them (2.5 mm)
const LINE_TO_LINE: f64 = 2.5 / 25.4;
// LineJumpFactorX / Y when the page lacks them
const FACTOR: f64 = 2.0 / 3.0;

/// a page's line jump cells
#[derive(Copy, Clone, Debug, PartialEq)]
struct Settings {
    /// LineJumpCode 0 .. 5
    code: u8,
    /// LineJumpStyle 0 .. 9
    style: u8,
    /// the width of a jump on a piece along x and on one along y (drawing units)
    size: [f64; 2],
    /// PageLineJumpDirX / Y (0 .. 2)
    dir: [u8; 2],
}

fn settings(doc: &Document, page: PageRef) -> Settings {
    let ps = SheetRef::Page(page);
    let n = |c: &str| num(doc, ps, c);
    let code = |c: &str, max: f64, d: u8| n(c).map_or(d, |v| v.clamp(0.0, max) as u8);
    let factor = |c: &str| n(c).map_or(FACTOR, |v| v.clamp(0.0, 10.0));
    let spacing = |c: &str| n(c).filter(|v| *v >= 0.0).map_or(LINE_TO_LINE, |v| v.min(100.0));
    // paper -> drawing units
    let scale = match (n("PageScale"), n("DrawingScale")) {
        (Some(p), Some(d)) if p > 0.0 && d > 0.0 && (d / p).is_finite() => d / p,
        _ => 1.0,
    };
    Settings {
        code: code("LineJumpCode", 5.0, 1),
        style: code("LineJumpStyle", 9.0, 0),
        size: [factor("LineJumpFactorX") * spacing("LineToLineX") * scale, factor("LineJumpFactorY") * spacing("LineToLineY") * scale],
        dir: [code("PageLineJumpDirX", 2.0, 0), code("PageLineJumpDirY", 2.0, 0)],
    }
}

// a connector of the page
#[derive(Clone, Debug)]
struct Conn {
    shape: ShapeRef,
    /// painter's order: its top-level shape's index, then its place under it
    key: (u32, u32),
    /// ConLineJumpCode 0 .. 4, ConLineJumpStyle 0 .. 9, ConLineJumpDirX / Y 0 .. 2
    code: u8,
    style: u8,
    dir: [u8; 2],
    /// its rows as they are, taken apart, its Width and Height, local -> page
    rows: Vec<(Kind, [f64; 2])>,
    parsed: Parsed,
    wh: [f64; 2],
    xf: Xform,
    /// its way on the page
    pts: Vec<[f64; 2]>,
    /// the jumps its rows hold now (start, end on the page)
    now: Vec<([f64; 2], [f64; 2])>,
    /// the order it is laid in now (0: not laid)
    seq: u32,
}

impl Conn {
    // whether its rows jump over the point now
    fn jumps_now(&self, x: [f64; 2]) -> bool {
        self.now.iter().any(|&(s, e)| on_piece(x, s, e))
    }
}

// the connectors of a page with a grid over their pieces
struct PageConns {
    set: Settings,
    conns: Vec<Conn>,
    by_shape: BTreeMap<ShapeRef, usize>,
    grid: PieceGrid,
}

// the layers of a page: (index, Visible)
fn layer_states(doc: &Document, page: PageRef) -> Vec<(u32, bool)> {
    doc.layers(page)
        .into_iter()
        .filter_map(|l| match l.key() {
            crate::sheet::RowKey::Index(ix) => Some((ix, l.num("Visible").is_none_or(|v| v != 0.0))),
            _ => None,
        })
        .collect()
}

// whether a shape is on a shown layer (or on none)
fn shown(doc: &Document, r: ShapeRef, layers: &[(u32, bool)]) -> bool {
    let members = doc.layer_members(r);
    if members.is_empty() || layers.is_empty() {
        return true;
    }
    members.iter().any(|m| layers.iter().find(|(ix, _)| ix == m).is_none_or(|l| l.1))
}

// the connectors of a page (those under `gone` apart: their ways)
fn collect(doc: &Document, page: PageRef, gone: &BTreeSet<ShapeRef>) -> (PageConns, Vec<Vec<[f64; 2]>>) {
    let set = settings(doc, page);
    let layers = layer_states(doc, page);
    let mut conns = Vec::new();
    let mut away = Vec::new();
    let tops: Vec<ShapeRef> = doc.top_shapes(Owner::Page(page)).collect();
    for (i, top) in tops.into_iter().enumerate() {
        let mut seq = 0u32;
        let mut w = Walk { doc, top: i as u32, seq: &mut seq, gone, layers: &layers, out: &mut conns, away: &mut away };
        w.shape(top, 0, false);
    }
    let by_shape = conns.iter().enumerate().map(|(i, c)| (c.shape, i)).collect();
    let grid = PieceGrid::build(&conns);
    (PageConns { set, conns, by_shape, grid }, away)
}

// the way through a top-level shape and its subshapes
struct Walk<'a, 'd> {
    doc: &'d Document,
    top: u32,
    seq: &'a mut u32,
    gone: &'a BTreeSet<ShapeRef>,
    layers: &'a [(u32, bool)],
    out: &'a mut Vec<Conn>,
    away: &'a mut Vec<Vec<[f64; 2]>>,
}

impl Walk<'_, '_> {
    fn shape(&mut self, r: ShapeRef, depth: u32, under_gone: bool) {
        let doc = self.doc;
        let Some(s) = doc.shape(r) else { return };
        let header = s.header();
        if header.del || depth > 256 || matches!(header.kind, ShapeKind::Unspecified | ShapeKind::Guide) {
            return;
        }
        *self.seq += 1;
        let gone = under_gone || self.gone.contains(&r);
        if glue::is_connector(doc, r) && shown(doc, r, self.layers) {
            if let Some(c) = conn(doc, r, (self.top, *self.seq)) {
                if gone {
                    self.away.push(c.pts);
                } else {
                    self.out.push(c);
                }
            }
        }
        let children: Vec<ShapeRef> = s.children().collect();
        for c in children {
            self.shape(c, depth + 1, gone);
        }
    }
}

// a connector that takes part: a line, a way of straight pieces
fn conn(doc: &Document, r: ShapeRef, key: (u32, u32)) -> Option<Conn> {
    let sheet = SheetRef::Shape(r);
    if num(doc, sheet, "LinePattern").unwrap_or(1.0) as u32 == 0 {
        return None;
    }
    let rows = rows_of(doc, r)?;
    let parsed = parse(&rows)?;
    let xf = doc.page_xform(r);
    let pts: Vec<[f64; 2]> = parsed.base.iter().map(|p| xf.apply(*p)).collect();
    let now = parsed.jumps.iter().map(|(_, s, e, _)| (xf.apply(*s), xf.apply(*e))).collect();
    let code = |c: &str, max: f64| num(doc, sheet, c).map_or(0, |v| v.clamp(0.0, max) as u8);
    let wh = [num_sym(doc, sheet, names::WIDTH).unwrap_or(0.0), num_sym(doc, sheet, names::HEIGHT).unwrap_or(0.0)];
    Some(Conn {
        shape: r,
        key,
        code: code("ConLineJumpCode", 4.0),
        style: code("ConLineJumpStyle", 9.0),
        dir: [code("ConLineJumpDirX", 2.0), code("ConLineJumpDirY", 2.0)],
        rows,
        parsed,
        wh,
        xf,
        pts,
        now,
        seq: 0,
    })
}

// a jump worked out (page coordinates)
#[derive(Copy, Clone, Debug)]
struct NewJump {
    piece: usize,
    start: [f64; 2],
    end: [f64; 2],
    normal: [f64; 2],
    /// half its width: the arc's bow, the unit of the corners
    half: f64,
    style: JumpStyle,
}

// what a row should be (local coordinates)
#[derive(Clone, Debug)]
enum Want {
    Move([f64; 2]),
    Line([f64; 2]),
    Arc([f64; 2], f64),
    Poly([f64; 2], Vec<[f64; 2]>),
}

impl PageConns {
    // connectors (not `skip`) whose pieces cross the way
    fn partners(&self, pts: &[[f64; 2]], skip: usize, out: &mut BTreeSet<usize>) {
        let mut hits = Vec::new();
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if !(dist(a, b) > 1e-9) {
                continue;
            }
            self.grid.query(seg_box(a, b), &mut hits);
            for &e in &hits {
                let (j, q) = self.grid.entries[e as usize];
                let j = j as usize;
                if j == skip || out.contains(&j) {
                    continue;
                }
                let d = &self.conns[j];
                if cross(a, b, d.pts[q as usize], d.pts[q as usize + 1]).is_some() {
                    out.insert(j);
                }
            }
        }
    }

    // connectors whose jumps the crossings do not explain: a jump without a crossing under it,
    // or wider on one side of its crossings than on the other (a merged one that lost one)
    fn stale(&self, out: &mut BTreeSet<usize>) {
        let mut hits = Vec::new();
        for (i, c) in self.conns.iter().enumerate() {
            if c.now.is_empty() || out.contains(&i) {
                continue;
            }
            for (n, (k, _, _, _)) in c.parsed.jumps.iter().enumerate() {
                let (a, b) = (c.pts[*k], c.pts[*k + 1]);
                let len = dist(a, b);
                if !(len > 1e-9) {
                    continue;
                }
                let u = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
                let along = |p: [f64; 2]| (p[0] - a[0]) * u[0] + (p[1] - a[1]) * u[1];
                let (s, e) = c.now[n];
                let (ds, de) = (along(s).min(along(e)), along(s).max(along(e)));
                let tol = 1e-6 * (1.0 + len);
                let (mut first, mut last) = (f64::INFINITY, f64::NEG_INFINITY);
                self.grid.query(seg_box(a, b), &mut hits);
                for &h in &hits {
                    let (j, q) = self.grid.entries[h as usize];
                    if j as usize == i {
                        continue;
                    }
                    let d = &self.conns[j as usize];
                    if let Some(t) = cross(a, b, d.pts[q as usize], d.pts[q as usize + 1]) {
                        let x = t * len;
                        if x > ds - tol && x < de + tol {
                            first = first.min(x);
                            last = last.max(x);
                        }
                    }
                }
                if !(first <= last) || ((first - ds) - (de - last)).abs() > tol {
                    out.insert(i);
                    break;
                }
            }
        }
    }

    // connector i's jumps worked out anew
    fn compute(&self, i: usize) -> Vec<NewJump> {
        let c = &self.conns[i];
        let mut found: Vec<(usize, f64)> = Vec::new();
        let mut hits = Vec::new();
        for (k, w) in c.pts.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            if !(dist(a, b) > 1e-9) {
                continue;
            }
            self.grid.query(seg_box(a, b), &mut hits);
            for &e in &hits {
                let (j, q) = self.grid.entries[e as usize];
                if j as usize == i {
                    continue;
                }
                let d = &self.conns[j as usize];
                let (qa, qb) = (d.pts[q as usize], d.pts[q as usize + 1]);
                let Some(t) = cross(a, b, qa, qb) else { continue };
                let x = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                if jumper(c, (a, b), d, (qa, qb), x, self.set.code) {
                    found.push((k, t));
                }
            }
        }
        make(c, &self.set, found)
    }

    // the rows connector i should have with these jumps
    fn wanted(&self, i: usize, jumps: &[NewJump]) -> Vec<Want> {
        let c = &self.conns[i];
        let inv = c.xf.inverse().unwrap_or(Xform::IDENTITY);
        let base = &c.parsed.base;
        let mut out = Vec::with_capacity(base.len() + 2 * jumps.len());
        out.push(Want::Move(base[0]));
        for k in 0..base.len() - 1 {
            for j in jumps.iter().filter(|j| j.piece == k) {
                let (ls, le) = (inv.apply(j.start), inv.apply(j.end));
                out.push(Want::Line(ls));
                match j.style {
                    JumpStyle::Gap => out.push(Want::Move(le)),
                    JumpStyle::Arc => {
                        // the bow: how far right of the chord (local) the arc's middle lies
                        let mid = [(j.start[0] + j.end[0]) * 0.5, (j.start[1] + j.end[1]) * 0.5];
                        let apex = inv.apply([mid[0] + j.normal[0] * j.half, mid[1] + j.normal[1] * j.half]);
                        let l = dist(ls, le);
                        let lm = [(ls[0] + le[0]) * 0.5, (ls[1] + le[1]) * 0.5];
                        let right = [(le[1] - ls[1]) / l, -(le[0] - ls[0]) / l];
                        let bow = (apex[0] - lm[0]) * right[0] + (apex[1] - lm[1]) * right[1];
                        out.push(Want::Arc(le, bow));
                    }
                    style => {
                        let u = [(j.end[0] - j.start[0]) / (2.0 * j.half), (j.end[1] - j.start[1]) / (2.0 * j.half)];
                        let pts = style
                            .corners()
                            .iter()
                            .map(|q| inv.apply([j.start[0] + (u[0] * q[0] + j.normal[0] * q[1]) * j.half, j.start[1] + (u[1] * q[0] + j.normal[1] * q[1]) * j.half]))
                            .collect();
                        out.push(Want::Poly(le, pts));
                    }
                }
            }
            out.push(Want::Line(base[k + 1]));
        }
        out
    }

    // the connectors `todo` get their jumps anew: rows written where they change
    fn write(&self, tx: &mut Transaction, eng: &mut Engine, todo: &BTreeSet<usize>) -> Result<(), EditError> {
        let mut writes: Vec<(ShapeRef, Vec<Want>, [f64; 2])> = Vec::new();
        for &i in todo {
            let js = self.compute(i);
            let want = self.wanted(i, &js);
            let c = &self.conns[i];
            if !same(&c.rows, &want) {
                writes.push((c.shape, want, c.wh));
            }
        }
        for (s, want, wh) in writes {
            let rows: Vec<GeoRow> = want.into_iter().map(|w| geo_row(w, wh)).collect();
            glue::write_geometry(tx, eng, s, &rows)?;
        }
        Ok(())
    }
}

// whether the rows are what is wanted
fn same(rows: &[(Kind, [f64; 2])], want: &[Want]) -> bool {
    const TOL: f64 = 1e-9;
    let eq = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() <= TOL * (1.0 + b[0].abs()) && (a[1] - b[1]).abs() <= TOL * (1.0 + b[1].abs());
    rows.len() == want.len()
        && rows.iter().zip(want).all(|((k, p), w)| match (k, w) {
            (Kind::Move, Want::Move(q)) | (Kind::Line, Want::Line(q)) => eq(*p, *q),
            (Kind::Arc(a), Want::Arc(q, b)) => eq(*p, *q) && (a - b).abs() <= TOL * (1.0 + b.abs()),
            (Kind::Poly(a), Want::Poly(q, b)) => eq(*p, *q) && a.len() == b.len() && a.iter().zip(b).all(|(x, y)| eq(*x, *y)),
            _ => false,
        })
}

// a wanted row as glue writes it
fn geo_row(w: Want, wh: [f64; 2]) -> GeoRow {
    let xy = |p: [f64; 2]| alloc::vec![(names::X, Cv::N(p[0])), (names::Y, Cv::N(p[1]))];
    match w {
        Want::Move(p) => GeoRow { kind: names::MOVE_TO, kind_name: "MoveTo", cells: xy(p) },
        Want::Line(p) => GeoRow { kind: names::LINE_TO, kind_name: "LineTo", cells: xy(p) },
        Want::Arc(p, bow) => {
            let mut cells = xy(p);
            cells.push((names::A, Cv::N(bow)));
            GeoRow { kind: names::ARC_TO, kind_name: "ArcTo", cells }
        }
        Want::Poly(p, pts) => {
            let mut cells = xy(p);
            cells.push((names::A, Cv::F(polyline(&pts, wh))));
            GeoRow { kind: names::POLYLINE_TO, kind_name: "PolylineTo", cells }
        }
    }
}

// POLYLINE(xType, yType, x, y, ...) through local points: fractions of Width and Height (type
// 0) as Visio writes them, absolute (1) along a side of no length
fn polyline(pts: &[[f64; 2]], wh: [f64; 2]) -> String {
    let (rx, ry) = (wh[0].abs() > 1e-9, wh[1].abs() > 1e-9);
    let mut t = format!("POLYLINE({}, {}", if rx { 0 } else { 1 }, if ry { 0 } else { 1 });
    for p in pts {
        let x = if rx { p[0] / wh[0] } else { p[0] };
        let y = if ry { p[1] / wh[1] } else { p[1] };
        t.push_str(&format!(", {x},{y}"));
    }
    t.push(')');
    t
}

// ---------------------------------------------------------------- laying

/// the connectors a gesture lays, in its order
#[derive(Clone, Debug, Default)]
pub struct Laid {
    list: Vec<ShapeRef>,
}

impl Laid {
    /// a connector laid (other shapes are passed over)
    pub fn note(&mut self, doc: &Document, c: ShapeRef) {
        if matches!(c.owner, Owner::Page(_)) && glue::is_connector(doc, c) {
            self.list.push(c);
        }
    }

    /// whether no connector was noted
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// after the connectors noted are laid: their jumps, those of every connector they cross and of
/// every connector whose jumps the crossings no longer explain, worked out and written (one call
/// per gesture: the order of the list is the order they were laid in)
pub fn lay(tx: &mut Transaction, eng: &mut Engine, laid: &Laid) -> Result<(), EditError> {
    let mut pages: Vec<PageRef> = Vec::new();
    for c in &laid.list {
        if let Owner::Page(p) = c.owner {
            if !pages.contains(&p) {
                pages.push(p);
            }
        }
    }
    for page in pages {
        let (mut pc, _) = collect(tx.doc(), page, &BTreeSet::new());
        let mut todo = BTreeSet::new();
        for (n, c) in laid.list.iter().enumerate() {
            if c.owner != Owner::Page(page) {
                continue;
            }
            let Some(&i) = pc.by_shape.get(c) else { continue };
            pc.conns[i].seq = n as u32 + 1;
            todo.insert(i);
        }
        let laid_now: Vec<usize> = todo.iter().copied().collect();
        for i in laid_now {
            let now = pc.conns[i].pts.clone();
            pc.partners(&now, i, &mut todo);
        }
        pc.stale(&mut todo);
        pc.write(tx, eng, &todo)?;
    }
    Ok(())
}

/// these connectors laid again where they are (their line jump cells changed), in this order
pub fn lay_again(tx: &mut Transaction, eng: &mut Engine, conns: &[ShapeRef]) -> Result<(), EditError> {
    let mut laid = Laid::default();
    for &c in conns {
        laid.note(tx.doc(), c);
    }
    lay(tx, eng, &laid)
}

/// every connector of the page laid again, in z-order (the page's line jump cells changed)
pub fn lay_page(tx: &mut Transaction, eng: &mut Engine, page: PageRef) -> Result<(), EditError> {
    let (mut pc, _) = collect(tx.doc(), page, &BTreeSet::new());
    for (i, c) in pc.conns.iter_mut().enumerate() {
        c.seq = i as u32 + 1;
    }
    let todo: BTreeSet<usize> = (0..pc.conns.len()).collect();
    pc.write(tx, eng, &todo)
}

/// before shapes are deleted: the connectors that cross their connectors (theirs and those in
/// them) get their jumps anew without them
pub fn forget(tx: &mut Transaction, eng: &mut Engine, deleted: &[ShapeRef]) -> Result<(), EditError> {
    let gone: BTreeSet<ShapeRef> = deleted.iter().copied().collect();
    let mut pages: Vec<PageRef> = Vec::new();
    for s in deleted {
        if let Owner::Page(p) = s.owner {
            if !pages.contains(&p) {
                pages.push(p);
            }
        }
    }
    for page in pages {
        let (pc, away) = collect(tx.doc(), page, &gone);
        if away.is_empty() {
            continue;
        }
        let mut todo = BTreeSet::new();
        for w in &away {
            pc.partners(w, usize::MAX, &mut todo);
        }
        pc.write(tx, eng, &todo)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- who jumps

type Seg2 = ([f64; 2], [f64; 2]);

// whether connector a jumps at x, the crossing of its piece pa with b's piece pb
fn jumper(a: &Conn, pa: Seg2, b: &Conn, pb: Seg2, x: [f64; 2], page_code: u8) -> bool {
    if a.code == 4 || b.code == 4 {
        return false;
    }
    let asked = matches!(a.code, 2 | 3) || matches!(b.code, 2 | 3);
    if asked {
        let wa = (a.code == 2 || b.code == 3) && a.code != 1;
        let wb = (b.code == 2 || a.code == 3) && b.code != 1;
        return match (wa, wb) {
            (true, false) => true,
            (true, true) => page_pick(a, pa, b, pb, x, page_code).unwrap_or(a.key > b.key),
            _ => false,
        };
    }
    a.code != 1 && page_pick(a, pa, b, pb, x, page_code) == Some(true)
}

// the page's rule (LineJumpCode): Some(whether a jumps), None: neither
fn page_pick(a: &Conn, pa: Seg2, b: &Conn, pb: Seg2, x: [f64; 2], code: u8) -> Option<bool> {
    // the steepness: |sin| of the piece's angle grows with |dy / dx|
    let steep = |p: Seg2| {
        let l = dist(p.0, p.1);
        if l > 0.0 {
            (p.1[1] - p.0[1]).abs() / l
        } else {
            0.0
        }
    };
    let (sa, sb) = (steep(pa), steep(pb));
    let tie = (sa - sb).abs() <= 1e-9;
    match code {
        // the more horizontal piece; equal slopes: neither
        1 => (!tie).then_some(sa < sb),
        // the more vertical piece
        2 => (!tie).then_some(sa > sb),
        // the one laid last; neither laid now: as its rows show
        3 => Some(if a.seq != b.seq { a.seq > b.seq } else { a.jumps_now(x) }),
        4 => Some(a.key > b.key),
        5 => Some(a.key < b.key),
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

// the jumps of connector c from its crossings (piece, parameter on it), in the order of the way
fn make(c: &Conn, set: &Settings, mut found: Vec<(usize, f64)>) -> Vec<NewJump> {
    const TOL: f64 = 1e-6;
    let mut out = Vec::new();
    if found.is_empty() {
        return out;
    }
    found.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1)));
    let style = JumpStyle::from_code(if c.style != 0 { c.style } else { set.style } as u32);
    let mut k = 0;
    while k < found.len() {
        let piece = found[k].0;
        let mut e = k;
        while e < found.len() && found[e].0 == piece {
            e += 1;
        }
        let (a, b) = (c.pts[piece], c.pts[piece + 1]);
        let len = dist(a, b);
        let u = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        let flat = u[0].abs() >= u[1].abs();
        let w = set.size[if flat { 0 } else { 1 }];
        if w > 1e-9 && len > 0.0 {
            let half = w * 0.5;
            // the side: up / down on a piece along x, left / right on one along y
            let left = [-u[1], u[0]];
            let normal = if flat {
                let up = (if c.dir[0] != 0 { c.dir[0] } else { set.dir[0] }) != 2;
                if (left[1] > 0.0) == up {
                    left
                } else {
                    [-left[0], -left[1]]
                }
            } else {
                let leftward = (if c.dir[1] != 0 { c.dir[1] } else { set.dir[1] }) != 2;
                if (left[0] < 0.0) == leftward {
                    left
                } else {
                    [-left[0], -left[1]]
                }
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
                let at = |d: f64| [a[0] + u[0] * d, a[1] + u[1] * d];
                out.push(NewJump { piece, start: at(from), end: at(to), normal, half: (to - from) * 0.5, style });
            }
        }
        k = e;
    }
    out
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
        let mut pieces: Vec<(u32, u32, [f64; 4])> = Vec::new();
        for (ci, c) in conns.iter().enumerate() {
            for (pi, w) in c.pts.windows(2).enumerate() {
                if dist(w[0], w[1]) > 1e-9 {
                    pieces.push((ci as u32, pi as u32, seg_box(w[0], w[1])));
                }
            }
        }
        let Some(bb) = pieces.iter().map(|p| p.2).reduce(union) else { return PieceGrid::default() };
        let w = (bb[2] - bb[0]).max(1e-6);
        let h = (bb[3] - bb[1]).max(1e-6);
        let side = w.max(h) / CELLS as f64;
        let nx = (ceil(w / side) as usize).clamp(1, CELLS);
        let ny = (ceil(h / side) as usize).clamp(1, CELLS);
        let mut g = PieceGrid { origin: [bb[0], bb[1]], cell: [w / nx as f64, h / ny as f64], nx, ny, cells: alloc::vec![Vec::new(); nx * ny], ..PieceGrid::default() };
        for (ci, pi, b) in pieces {
            let e = g.entries.len() as u32;
            g.entries.push((ci, pi));
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

fn seg_box(a: [f64; 2], b: [f64; 2]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    sqrt((a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]))
}

#[cfg(test)]
#[path = "jumps_tests.rs"]
mod tests;
