// layout -- (JD21) a page laid out anew (Seitenlayout neu anordnen: CN-13, RW-09), connected
// shapes aligned and spaced (CN-14, RS-25), shapes off the page breaks (PR-03), the diagram
// turned or flipped with its shapes upright (RS-25)
//
//   Place::from_code(PlaceStyle) / place.code()   Visio's placement codes <-> style, direction,
//                                  the side a compact tree steps to, a hierarchy's alignment
//   Opts::from_page(doc, page)     the page's cells: PlaceStyle, PlaceDepth, AvenueSizeX/Y (the
//                                  space between shapes), BlockSizeX/Y (the layout grid)
//   diagram(doc, owner, only) -> Diagram   what a layout moves: the placeable shapes (nodes),
//                                  their boxes, the connections Begin -> End between them, the
//                                  boxes that stay (obstacles)
//   arrange(&diagram, &opts) -> centres    where the nodes' middles go (pure: no document)
//   plan(doc, owner, only, &opts) -> moves the nodes' moves
//   relayout(tx, eng, owner, only, &opts, routing) -> moved   the command: moves, members and
//                                  callouts along, containers settled, connectors routed
//   store(tx, eng, page, &opts, route, resize)   the page's cells as the dialog sets them
//   enlarge_page(tx, eng, page, margin)    Seite vergrößern: the page around its drawing
//   align_space(doc, owner, only, align, space, spacing) -> moves   (adjust.rs) Automatische
//                                  Ausrichtung und Abstände, Automatischer Abstand
//   turn(doc, owner, only, Turn, spacing) -> moves   (adjust.rs) Diagramm drehen
//   off_breaks(doc, owner, only, &xs, &ys, margin) -> moves   (adjust.rs) Von Seitenumbrüchen
//                                  verschieben, Seitenumbrüche vermeiden
//   move_nodes(tx, eng, owner, &moves) / reroute(tx, eng, owner, &moved, routing, all)
//
// Coordinates are the page's (drawing units, inches, y up).  Everything is deterministic: the
// same drawing gives the same layout (orders come from positions and shape ids, never from a
// hash).  Nothing runs on its own: opening a file lays out nothing (PLAN.md, "Fallen").
//
// Visio's rules -- belegt (Microsoft Learn: the ShapeSheet reference of the Page Layout and
// Shape Layout sections, VisCellVals, Page.Layout, Page.LayoutIncremental, VisLayoutDirection;
// support.microsoft.com: "Re-layout a diagram", "Align and position shapes in a diagram",
// "Configure Layout dialog box", "Layout and Routing Spacing dialog box"; the Visio blog
// "Layout improvements in Visio 2010" and the Office developer blog on LayoutChangeDirection;
// the German ribbon of Visio for Microsoft 365):
// * Design > Layout > Re-Layout Page (Entwurf › Layout › Zeichenblattlayout neu anordnen): a
//   gallery (flowchart, hierarchy, compact tree, radial, circular) with a live preview while
//   pointing; More Layout Options = Configure Layout: placement style, direction (flowchart,
//   compact tree and hierarchy only), alignment (hierarchy only), spacing; connector style and
//   appearance (straight / curved), "Apply routing style to connectors" (to all connectors of
//   the page or the selected ones); "Apply settings to" the selection or the current page;
//   "Enlarge page to fit drawing".  A layout of a selection lays out the selected shapes only,
//   with nothing selected the whole page.
// * PlaceStyle (visPLOPlace*/visLOPlace*): 0 default, 1 top to bottom, 2 left to right,
//   3 radial, 4 bottom to top, 5 right to left, 6 circular, 7..14 compact trees (7 down-right,
//   8 right-down, 9 right-up, 10 up-right, 11 up-left, 12 left-up, 13 left-down, 14 down-left),
//   15 parent default (ShapePlaceStyle), 16..27 hierarchies (top to bottom left/centre/right
//   16-18, bottom to top 19-21, left to right top/middle/bottom 22-24, right to left 25-27).
// * PlaceDepth: 0 page default, 1 medium, 2 deep, 3 shallow ("the method by which the drawing
//   is analyzed before creating the layout").  PlaceFlip / ShapePlaceFlip: 0 default, 1 flip x,
//   2 flip y, 4 in 90 degree steps, 8 none.  ResizePage: TRUE enlarges the page to enclose the
//   drawing.  AvenueSizeX/Y: the space between shapes of a layout ("Space between shapes" of
//   Layout and Routing Spacing); BlockSizeX/Y: "the area in which each of your shapes must fit"
//   ("Average shape size"); LineToLineX/Y and LineToNodeX/Y: the clearance between connectors and
//   between connectors and shapes; LineAdjustFrom/To: which overlapping connectors are spaced
//   apart; PlowCode / ShapePlowCode: placeable shapes move away on a drop; PageShapeSplit: a
//   shape dropped on a connector splits it.  A metric document's defaults (local corpus):
//   AvenueSize 7.5 mm, BlockSize 5 mm, LineToLine and LineToNode 2.5 mm.
// * ObjType: 0 Visio decides by context (a shape becomes placeable, 1, when the Connector tool
//   glues a connector to it), 1 placeable, 2 routable (1-D), 4 neither -- it wins over the
//   others --, 8 a group with placeable or routable shapes.  ShapeFixedCode: &H1 not moved by a
//   layout, &H2 not moved and plowing shapes stay off it, &H4 not moved and they may lie on it,
//   &H20 / &H40 connection points ignored / only, &H80 glue to the alignment box.
// * Auto Align & Space (Home > Arrange > Position; Automatische Ausrichtung und Abstände):
//   connected shapes; a shape downstream of another (over a connector) is its child; a child is
//   aligned with its parent and set at the default page spacing (or the one of Spacing
//   Options), shapes further downstream are carried along; the selected shapes, all shapes with
//   nothing selected.  Auto Space only spaces, Auto Align only aligns.  Avoid Page Breaks (a
//   toggle) keeps those commands off the page breaks; Move Off Page Breaks moves the selected
//   shapes (all with nothing selected) off them.
// * Rotate Diagram: the connected shapes (every set of two or more connected shapes; the
//   selected ones) turned 90 degrees right or left or flipped vertically or horizontally as a
//   unit; the shapes themselves keep their orientation (visLayoutDirRotateRight 0, ..Left 1,
//   ..FlipVert 2, ..FlipHorz 3).
//
// Angenommen (no source says it; chosen here, the user's check in Visio may correct it):
// * The flow follows Begin -> End of each connector (FEATURES CN-13); a shape glued to a 1-D
//   shape counts as connected whatever its glue (static or dynamic).  A container is laid out
//   as one block with its members (and every callout moves with its target); a connection to a
//   member is one to the outermost container.
// * Shapes that are not placeable (ObjType 0 without glue), fixed shapes (ShapeFixedCode &H7,
//   LockMoveX/Y) and, for a selection, the placeable shapes outside it stay where they are and
//   are kept clear: the laid-out drawing is shifted across past them.  A shape that encloses
//   all the moved ones (a frame) is no obstacle.  ObjType 4 shapes are ignored altogether.
// * The laid-out drawing keeps the corner of the moved shapes' box where the flow starts (top
//   left for top to bottom, ...); separate parts (components) follow each other across the
//   flow, in the order they had; placeable shapes without connections are parts of their own.
// * PlaceStyle 0 is a flowchart top to bottom, PlaceDepth 0 is medium; the depth is how hard
//   the crossings of connectors are reduced (shallow: two sweeps, medium: eight, deep:
//   twenty-four and exchanges of neighbours).  Middles snap to the layout grid (BlockSizeX/Y)
//   when Opts::grid is set; the distances between middles are then whole grid steps.
// * Compact tree "Down then right" (7): the tree grows downwards, each level steps right by
//   half its parent's width plus half the spacing (an outline); the other seven alike.
// * Hierarchy: the levels as rows, each row packed at the spacing and aligned at its start,
//   centred or at its end; flowchart: each shape centred over (and under) its connections.
// * Radial: the root is the shape with the most connections (among equals one without
//   incoming connections, then the one that reaches most), the others on circles around it in
//   wedges by the leaves of their subtrees, the root's first child at the top, clockwise.  Circular: a ring per part in the order
//   of a walk along the connections, the first at the top, clockwise.
// * Rotate Diagram turns each connected part (two or more shapes; a selection as one part)
//   about the middle of its box; where shapes would come too close after a turn the distances
//   grow along that axis (to the page spacing).  Avoid Page Breaks also applies to Re-Layout
//   Page (the dialog offers it).
//
// How a layered style works (flowchart, hierarchy; layered.rs): cycles broken by a depth-first
// walk (the connector leading back is reversed for the layout only), levels by the longest
// path (a source moved down next to its first successor), long connections through dummy
// points, the order in each level by barycentre sweeps with the crossings counted (the best
// order kept), levels at the spacing apart; then across: flowchart nodes as near the median of
// their neighbours as the order and the spacing allow (isotonic regression, pool adjacent
// violators), hierarchy rows packed and aligned.

mod adjust;
mod layered;
mod radial;
#[cfg(test)]
mod tests;

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::cell::Value;
use crate::document::{Document, Owner, PageRef, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::math;
use crate::formula::Engine;
use crate::glue::{self, Target};
use crate::ops;
use crate::shape::ShapeKind;
use crate::sheet::CellAddr;
use crate::structure;
use crate::units::{Dimension, Unit};

pub use adjust::{align_space, off_breaks, turn, Turn};

const EPS: f64 = 1e-9;
/// Visio's default space between shapes of a metric drawing (7.5 mm)
pub const AVENUE: f64 = 7.5 / 25.4;
/// Visio's default layout block of a metric drawing (5 mm)
pub const BLOCK: f64 = 5.0 / 25.4;

/// a layout style of Seitenlayout neu anordnen
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Style {
    /// levels in the flow, each shape centred over its connections
    Flowchart,
    /// levels in the flow as rows, aligned at their start, centre or end
    Hierarchy,
    /// a tree as an outline: down (right, ...) with the levels stepping aside
    CompactTree,
    /// the root in the middle, the others on circles around it
    Radial,
    /// a ring
    Circular,
}

/// a direction on the page
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Dir {
    /// top to bottom
    Down,
    /// bottom to top
    Up,
    /// left to right
    Right,
    /// right to left
    Left,
}

impl Dir {
    /// the unit vector on the page (y up)
    pub fn vector(self) -> [f64; 2] {
        match self {
            Dir::Down => [0.0, -1.0],
            Dir::Up => [0.0, 1.0],
            Dir::Right => [1.0, 0.0],
            Dir::Left => [-1.0, 0.0],
        }
    }

    /// up or down
    pub fn vertical(self) -> bool {
        matches!(self, Dir::Down | Dir::Up)
    }

    // the index in Visio's order of the flowchart and hierarchy codes: down, up, right, left
    fn index(self) -> usize {
        match self {
            Dir::Down => 0,
            Dir::Up => 1,
            Dir::Right => 2,
            Dir::Left => 3,
        }
    }

    // the side across a flow that its levels spread to: right for a vertical flow, down for a
    // horizontal one
    fn across(self) -> Dir {
        if self.vertical() { Dir::Right } else { Dir::Down }
    }
}

/// where a hierarchy's rows line up
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Align {
    /// left (top for a horizontal flow)
    Start,
    /// centred (middle)
    Centre,
    /// right (bottom)
    End,
}

/// how hard a layout works on the crossings (PlaceDepth)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Depth {
    /// 3: few sweeps
    Shallow,
    /// 1 (and the page default 0)
    Medium,
    /// 2: many sweeps and exchanges
    Deep,
}

impl Depth {
    /// PlaceDepth -> depth (0 and unknown values: medium)
    pub fn from_code(code: i64) -> Depth {
        match code {
            2 => Depth::Deep,
            3 => Depth::Shallow,
            _ => Depth::Medium,
        }
    }

    /// depth -> PlaceDepth
    pub fn code(self) -> i64 {
        match self {
            Depth::Medium => 1,
            Depth::Deep => 2,
            Depth::Shallow => 3,
        }
    }
}

/// a placement: style, direction, the side a compact tree steps to, a hierarchy's alignment
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Place {
    pub style: Style,
    /// the flow (flowchart, hierarchy), the way a compact tree grows; radial, circular: Down
    pub dir: Dir,
    /// across `dir`: the side a compact tree's levels step to; else where the levels spread
    pub then: Dir,
    /// a hierarchy's rows (else Centre)
    pub align: Align,
}

impl Place {
    /// a style in a direction, the rest as Visio's gallery offers it (hierarchies centred,
    /// compact trees stepping right of a vertical and down of a horizontal growth)
    pub fn new(style: Style, dir: Dir) -> Place {
        let dir = if matches!(style, Style::Radial | Style::Circular) { Dir::Down } else { dir };
        Place { style, dir, then: dir.across(), align: Align::Centre }
    }

    /// PlaceStyle / ShapePlaceStyle -> placement (None: 0 default, 15 parent default, unknown)
    pub fn from_code(code: i64) -> Option<Place> {
        use Dir::*;
        let p = |style, dir: Dir, then: Dir, align| Some(Place { style, dir, then, align });
        match code {
            1 => p(Style::Flowchart, Down, Right, Align::Centre),
            2 => p(Style::Flowchart, Right, Down, Align::Centre),
            3 => p(Style::Radial, Down, Right, Align::Centre),
            4 => p(Style::Flowchart, Up, Right, Align::Centre),
            5 => p(Style::Flowchart, Left, Down, Align::Centre),
            6 => p(Style::Circular, Down, Right, Align::Centre),
            7 => p(Style::CompactTree, Down, Right, Align::Centre),
            8 => p(Style::CompactTree, Right, Down, Align::Centre),
            9 => p(Style::CompactTree, Right, Up, Align::Centre),
            10 => p(Style::CompactTree, Up, Right, Align::Centre),
            11 => p(Style::CompactTree, Up, Left, Align::Centre),
            12 => p(Style::CompactTree, Left, Up, Align::Centre),
            13 => p(Style::CompactTree, Left, Down, Align::Centre),
            14 => p(Style::CompactTree, Down, Left, Align::Centre),
            16..=27 => {
                let k = (code - 16) as usize;
                let dir = [Down, Up, Right, Left][k / 3];
                p(Style::Hierarchy, dir, dir.across(), [Align::Start, Align::Centre, Align::End][k % 3])
            }
            _ => None,
        }
    }

    /// placement -> PlaceStyle
    pub fn code(&self) -> i64 {
        use Dir::*;
        match self.style {
            Style::Flowchart => [1, 4, 2, 5][self.dir.index()],
            Style::Radial => 3,
            Style::Circular => 6,
            Style::Hierarchy => {
                let a = match self.align {
                    Align::Start => 0,
                    Align::Centre => 1,
                    Align::End => 2,
                };
                16 + 3 * self.dir.index() as i64 + a
            }
            Style::CompactTree => match (self.dir, self.side()) {
                (Down, Right) => 7,
                (Right, Down) => 8,
                (Right, Up) => 9,
                (Up, Right) => 10,
                (Up, Left) => 11,
                (Left, Up) => 12,
                (Left, Down) => 13,
                _ => 14,
            },
        }
    }

    /// the side across the flow (`then`, mended when it runs along the flow)
    pub fn side(&self) -> Dir {
        if self.then.vertical() == self.dir.vertical() { self.dir.across() } else { self.then }
    }

    // the frame a layout is computed in: u across the flow (towards the side), v along it
    fn frame(&self) -> Frame {
        match self.style {
            Style::Radial | Style::Circular => Frame { u: [1.0, 0.0], v: [0.0, -1.0] },
            _ => Frame { u: self.side().vector(), v: self.dir.vector() },
        }
    }
}

/// what a layout needs besides the drawing
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Opts {
    pub place: Place,
    /// the space between shapes, x and y (AvenueSizeX/Y), drawing units
    pub spacing: [f64; 2],
    pub depth: Depth,
    /// the layout grid the middles snap to (BlockSizeX/Y), drawing units; None: no grid
    pub grid: Option<[f64; 2]>,
}

impl Opts {
    /// a placement with Visio's metric defaults (7.5 mm apart, medium depth, no grid)
    pub fn new(place: Place) -> Opts {
        Opts { place, spacing: [AVENUE; 2], depth: Depth::Medium, grid: None }
    }

    /// the page's cells (lengths on paper, turned into drawing units): PlaceStyle (0: a
    /// flowchart top to bottom), PlaceDepth, AvenueSizeX/Y, BlockSizeX/Y as the grid
    pub fn from_page(doc: &Document, page: PageRef) -> Opts {
        let s = SheetRef::Page(page);
        let k = 1.0 / crate::pagesetup::scale(doc, page);
        let place = num(doc, s, "PlaceStyle").and_then(|c| Place::from_code(c as i64)).unwrap_or(Place::new(Style::Flowchart, Dir::Down));
        let len = |c: &str, d: f64| num(doc, s, c).filter(|v| *v >= 0.0).unwrap_or(d) * k;
        let block = [len("BlockSizeX", BLOCK), len("BlockSizeY", BLOCK)];
        Opts {
            place,
            spacing: [len("AvenueSizeX", AVENUE), len("AvenueSizeY", AVENUE)],
            depth: Depth::from_code(num(doc, s, "PlaceDepth").unwrap_or(0.0) as i64),
            grid: (block[0] > EPS && block[1] > EPS).then_some(block),
        }
    }
}

// the frame of a layout: page = u * U + v * V (U, V orthonormal)
#[derive(Copy, Clone, Debug)]
struct Frame {
    u: [f64; 2],
    v: [f64; 2],
}

impl Frame {
    fn to_page(&self, p: [f64; 2]) -> [f64; 2] {
        [p[0] * self.u[0] + p[1] * self.v[0], p[0] * self.u[1] + p[1] * self.v[1]]
    }

    fn from_page(&self, p: [f64; 2]) -> [f64; 2] {
        [p[0] * self.u[0] + p[1] * self.u[1], p[0] * self.v[0] + p[1] * self.v[1]]
    }

    // a box on the page -> its box in the frame
    fn boxed(&self, b: [f64; 4]) -> [f64; 4] {
        let (p, q) = (self.from_page([b[0], b[1]]), self.from_page([b[2], b[3]]));
        [p[0].min(q[0]), p[1].min(q[1]), p[0].max(q[0]), p[1].max(q[1])]
    }

    // a length along x / y of the page -> along u / v
    fn uv(&self, xy: [f64; 2]) -> [f64; 2] {
        if self.u[0].abs() > 0.5 { xy } else { [xy[1], xy[0]] }
    }
}

// ---------------------------------------------------------------- reading the drawing

fn num(doc: &Document, s: SheetRef, cell: &str) -> Option<f64> {
    let c = doc.sym(cell)?;
    doc.resolve(s, &CellAddr::single(c)).num().filter(|v| v.is_finite())
}

// the top-level shape a shape belongs to
fn top(doc: &Document, s: ShapeRef) -> ShapeRef {
    let mut cur = s;
    for _ in 0..64 {
        match doc.shape(cur).and_then(|x| x.parent()) {
            Some(p) => cur = p,
            None => break,
        }
    }
    cur
}

// the outermost container around a shape (itself when it is in none)
fn outermost(doc: &Document, s: ShapeRef) -> ShapeRef {
    let mut cur = s;
    for _ in 0..64 {
        match structure::containers_of(doc, cur).into_iter().filter(|&c| c != cur && doc.shape(c).is_some()).min() {
            Some(c) => cur = c,
            None => break,
        }
    }
    cur
}

// the shape a glued end stands for in a layout: the top-level shape, its outermost container
fn end_shape(doc: &Document, t: Target) -> Option<ShapeRef> {
    match t {
        Target::Point { shape, .. } | Target::Shape(shape) => Some(outermost(doc, top(doc, shape))),
        Target::None => None,
    }
}

// a top-level 2-D shape a layout may look at: no guide, no 1-D shape, not deleted, no callout
// (it moves with its target)
fn plain_2d(doc: &Document, s: ShapeRef) -> bool {
    let Some(sh) = doc.shape(s) else { return false };
    !sh.is_deleted() && sh.header().kind != ShapeKind::Guide && !glue::is_one_d(doc, s) && structure::kind(doc, s) != Some(structure::Kind::Callout)
}

/// the shape stays where a layout or an adjustment leaves it: ShapeFixedCode &H1, &H2 or &H4,
/// LockMoveX or LockMoveY set
pub fn fixed(doc: &Document, s: ShapeRef) -> bool {
    let sh = SheetRef::Shape(s);
    num(doc, sh, "ShapeFixedCode").is_some_and(|v| (v as i64) & 7 != 0)
        || num(doc, sh, "LockMoveX").is_some_and(|v| v != 0.0)
        || num(doc, sh, "LockMoveY").is_some_and(|v| v != 0.0)
}

// how a top-level 2-D shape takes part
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Part {
    /// ObjType 4: neither laid out nor kept clear
    Never,
    /// not placeable or fixed: stays, kept clear
    Stays,
    /// placeable and free
    Node,
}

fn part(doc: &Document, s: ShapeRef, glued: &BTreeSet<ShapeRef>) -> Part {
    let obj = num(doc, SheetRef::Shape(s), "ObjType").unwrap_or(0.0) as i64;
    if obj & 4 != 0 {
        return Part::Never;
    }
    let placeable = obj & 1 != 0 || (obj & 3 == 0 && glued.contains(&s));
    if !placeable || fixed(doc, s) { Part::Stays } else { Part::Node }
}

/// what a layout works on
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagram {
    /// the shapes laid out (placeable, free; a container stands for its members), page order
    pub nodes: Vec<ShapeRef>,
    /// their boxes on the page
    pub boxes: Vec<[f64; 4]>,
    /// the connections Begin -> End between nodes (indices into `nodes`), each once, no loops
    pub edges: Vec<(usize, usize)>,
    /// the boxes of what stays and is kept clear: shapes not placeable, fixed ones and (a
    /// selection) the placeable ones outside it
    pub obstacles: Vec<[f64; 4]>,
}

// the connections of a page: (Begin, End) as the shapes they stand for, and every shape glued
fn links(doc: &Document, owner: Owner) -> (Vec<(ShapeRef, ShapeRef)>, BTreeSet<ShapeRef>) {
    let mut glued: BTreeSet<ShapeRef> = BTreeSet::new();
    let mut out: Vec<(ShapeRef, ShapeRef)> = Vec::new();
    for c in doc.top_shapes(owner) {
        if doc.shape(c).is_none_or(|x| x.is_deleted()) || !glue::is_one_d(doc, c) {
            continue;
        }
        let [a, b] = glue::glued(doc, c);
        let (a, b) = (end_shape(doc, a), end_shape(doc, b));
        glued.extend(a);
        glued.extend(b);
        if let (Some(a), Some(b)) = (a, b) {
            out.push((a, b));
        }
    }
    (out, glued)
}

/// the shapes a layout of `owner` moves and what it keeps clear; `only` (not empty): a
/// selection, the others stay (a member selected stands for its outermost container)
pub fn diagram(doc: &Document, owner: Owner, only: &[ShapeRef]) -> Diagram {
    let (links, glued) = links(doc, owner);
    let sel: BTreeSet<ShapeRef> = only.iter().map(|&s| outermost(doc, top(doc, s))).collect();
    let mut d = Diagram::default();
    for s in doc.top_shapes(owner) {
        if !plain_2d(doc, s) || outermost(doc, s) != s {
            continue;
        }
        let Some(b) = ops::bounds(doc, &[s]) else { continue };
        match part(doc, s, &glued) {
            Part::Never => {}
            Part::Node if sel.is_empty() || sel.contains(&s) => {
                d.nodes.push(s);
                d.boxes.push(b);
            }
            _ => d.obstacles.push(b),
        }
    }
    for (a, b) in links {
        let (Some(i), Some(j)) = (d.nodes.iter().position(|&x| x == a), d.nodes.iter().position(|&x| x == b)) else { continue };
        if i != j && !d.edges.contains(&(i, j)) {
            d.edges.push((i, j));
        }
    }
    d
}

// ---------------------------------------------------------------- arranging

// a part of the drawing to lay out (layered.rs, radial.rs): its nodes' extents and original
// middles in the frame (u across, v along the flow), its connections (indices into them)
pub(super) struct Comp<'a> {
    pub ext: &'a [[f64; 2]],
    pub orig: &'a [[f64; 2]],
    pub edges: &'a [(usize, usize)],
    /// the spacing across and along the flow
    pub gap: [f64; 2],
    /// the grid steps across and along (0: none)
    pub step: [f64; 2],
    pub depth: Depth,
}

/// the smallest multiple of `step` at least `v` (`step` 0: v)
pub(super) fn ceil_to(v: f64, step: f64) -> f64 {
    if step > EPS { math::ceil(v / step - 1e-9) * step } else { v }
}

fn round_to(v: f64, step: f64) -> f64 {
    if step > EPS { math::floor(v / step + 0.5) * step } else { v }
}

// the parts of a graph: node lists in index order, the parts ordered by their first node
pub(super) fn components(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    fn find(r: &mut [usize], mut x: usize) -> usize {
        while r[x] != x {
            r[x] = r[r[x]];
            x = r[x];
        }
        x
    }
    let mut root: Vec<usize> = (0..n).collect();
    for &(a, b) in edges {
        if a >= n || b >= n {
            continue;
        }
        let (ra, rb) = (find(&mut root, a), find(&mut root, b));
        if ra != rb {
            let (lo, hi) = (ra.min(rb), ra.max(rb));
            root[hi] = lo;
        }
    }
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut slot: Vec<Option<usize>> = alloc::vec![None; n];
    for i in 0..n {
        let r = find(&mut root, i);
        match slot[r] {
            Some(k) => out[k].push(i),
            None => {
                slot[r] = Some(out.len());
                out.push(alloc::vec![i]);
            }
        }
    }
    out
}

fn grow(m: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [m[0].min(b[0]), m[1].min(b[1]), m[2].max(b[2]), m[3].max(b[3])]
}

const EMPTY: [f64; 4] = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];

/// where the middles of the diagram's nodes go (page coordinates; see the file header): the
/// parts laid out in the style, one after the other across the flow (a compact tree's along
/// it), the start corner of the nodes' box kept, the obstacles kept clear
pub fn arrange(d: &Diagram, opts: &Opts) -> Vec<[f64; 2]> {
    let n = d.nodes.len().min(d.boxes.len());
    if n == 0 {
        return Vec::new();
    }
    let fr = opts.place.frame();
    let gap = fr.uv(opts.spacing).map(|g| if g.is_finite() { g.max(0.0) } else { 0.0 });
    let step = opts.grid.map_or([0.0; 2], |g| fr.uv(g).map(|s| if s.is_finite() && s > EPS { s } else { 0.0 }));
    let fb: Vec<[f64; 4]> = d.boxes[..n].iter().map(|&b| fr.boxed(b)).collect();
    let ext: Vec<[f64; 2]> = fb.iter().map(|b| [(b[2] - b[0]).max(0.0), (b[3] - b[1]).max(0.0)]).collect();
    let orig: Vec<[f64; 2]> = fb.iter().map(|b| [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5]).collect();
    let edges: Vec<(usize, usize)> = d.edges.iter().copied().filter(|&(a, b)| a < n && b < n && a != b).collect();
    // the start corner of everything moved
    let all = fb.iter().fold(EMPTY, |m, &b| grow(m, b));
    // the obstacles in the frame, grown by the spacing; a frame around everything moved is none
    let obst: Vec<[f64; 4]> = d
        .obstacles
        .iter()
        .map(|&b| fr.boxed(b))
        .filter(|o| !(o[0] <= all[0] + EPS && o[1] <= all[1] + EPS && o[2] >= all[2] - EPS && o[3] >= all[3] - EPS))
        .map(|o| [o[0] - gap[0], o[1] - gap[1], o[2] + gap[0], o[3] + gap[1]])
        .collect();
    // each part laid out in the frame
    let mut parts: Vec<(Vec<usize>, Vec<[f64; 2]>, [f64; 4])> = Vec::new();
    for comp in components(n, &edges) {
        let idx_of = |g: usize| comp.iter().position(|&x| x == g);
        let ce: Vec<[f64; 2]> = comp.iter().map(|&g| ext[g]).collect();
        let co: Vec<[f64; 2]> = comp.iter().map(|&g| orig[g]).collect();
        let cedges: Vec<(usize, usize)> = edges.iter().filter_map(|&(a, b)| Some((idx_of(a)?, idx_of(b)?))).collect();
        let c = Comp { ext: &ce, orig: &co, edges: &cedges, gap, step, depth: opts.depth };
        let pos = match opts.place.style {
            Style::Flowchart => layered::layout(&c, None),
            Style::Hierarchy => layered::layout(&c, Some(opts.place.align)),
            Style::CompactTree => radial::compact(&c),
            Style::Radial => radial::radial(&c),
            Style::Circular => radial::circular(&c),
        };
        let bx = pos.iter().zip(&ce).fold(EMPTY, |m, (p, e)| grow(m, [p[0] - e[0] * 0.5, p[1] - e[1] * 0.5, p[0] + e[0] * 0.5, p[1] + e[1] * 0.5]));
        parts.push((comp, pos, bx));
    }
    // the parts in the order they had along the packing axis
    let pk = if opts.place.style == Style::CompactTree { 1 } else { 0 };
    let first = |comp: &[usize]| comp.iter().map(|&g| fb[g][pk]).fold(f64::INFINITY, f64::min);
    parts.sort_by(|a, b| first(&a.0).total_cmp(&first(&b.0)).then(a.0[0].cmp(&b.0[0])));
    // one after the other, clear of the obstacles
    let mut out = alloc::vec![[0.0; 2]; n];
    let cross = 1 - pk;
    let mut cursor = [all[0], all[1]][pk];
    let pack_gap = gap[pk] + step[pk];
    for (comp, pos, bx) in &parts {
        let mut off = [0.0; 2];
        off[cross] = [all[0], all[1]][cross] - bx[cross];
        off[pk] = cursor - bx[pk];
        for _ in 0..obst.len() + 2 {
            // the farthest edge of an obstacle a node of the part runs into
            let mut push: Option<f64> = None;
            for (&g, p) in comp.iter().zip(pos) {
                let e = ext[g];
                let b = [p[0] + off[0] - e[0] * 0.5, p[1] + off[1] - e[1] * 0.5, p[0] + off[0] + e[0] * 0.5, p[1] + off[1] + e[1] * 0.5];
                for o in &obst {
                    if b[0] < o[2] - EPS && b[2] > o[0] + EPS && b[1] < o[3] - EPS && b[3] > o[1] + EPS {
                        // the part moves on until this node's start passes the obstacle's end
                        let to = off[pk] + (o[2 + pk] - b[pk]);
                        push = Some(push.map_or(to, |m: f64| m.max(to)));
                    }
                }
            }
            match push {
                Some(v) if v > off[pk] + EPS => off[pk] = v,
                _ => break,
            }
        }
        for (&g, p) in comp.iter().zip(pos) {
            out[g] = [p[0] + off[0], p[1] + off[1]];
        }
        cursor = bx[2 + pk] + off[pk] + pack_gap;
    }
    // back onto the page, on the grid
    let gp = opts.grid.unwrap_or([0.0; 2]);
    out.iter()
        .map(|&p| {
            let q = fr.to_page(p);
            [round_to(q[0], gp[0]), round_to(q[1], gp[1])]
        })
        .collect()
}

/// the moves of a layout: each node by how far its middle goes
pub fn plan(doc: &Document, owner: Owner, only: &[ShapeRef], opts: &Opts) -> Vec<(ShapeRef, [f64; 2])> {
    let d = diagram(doc, owner, only);
    let to = arrange(&d, opts);
    d.nodes
        .iter()
        .zip(&d.boxes)
        .zip(&to)
        .map(|((&s, b), t)| (s, [t[0] - (b[0] + b[2]) * 0.5, t[1] - (b[1] + b[3]) * 0.5]))
        .filter(|(_, m)| m[0].abs() > EPS || m[1].abs() > EPS)
        .collect()
}

// ---------------------------------------------------------------- writing

/// how connectors follow a layout
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Routing {
    /// they follow the moved shapes (glue::update: ConFixedCode keeps a fixed way)
    #[default]
    Follow,
    /// Routing-Stil auf Verbinder anwenden: ShapeRouteStyle `route` and ConLineRouteExt `ext`
    /// (1 straight, 2 curved) on the connectors, routed anew
    Apply {
        /// ShapeRouteStyle
        route: i64,
        /// ConLineRouteExt
        ext: i64,
    },
}

/// move the nodes with what they carry (members, callouts), each shape once; then their
/// containers settle (lists laid out, sizes; no membership changes) -> every shape moved
pub fn move_nodes(tx: &mut Transaction, eng: &mut Engine, owner: Owner, moves: &[(ShapeRef, [f64; 2])]) -> Result<Vec<ShapeRef>, EditError> {
    let mut done: BTreeSet<ShapeRef> = BTreeSet::new();
    let mut moved: Vec<ShapeRef> = Vec::new();
    for &(s, d) in moves {
        let group: Vec<ShapeRef> = structure::carried(tx.doc(), &[s]).into_iter().filter(|x| !done.contains(x)).collect();
        done.extend(group.iter().copied());
        if (d[0].abs() <= EPS && d[1].abs() <= EPS) || group.is_empty() {
            continue;
        }
        ops::move_shapes(tx, eng, &group, d)?;
        moved.extend(group);
    }
    if moved.is_empty() {
        return Ok(moved);
    }
    let nodes: Vec<ShapeRef> = moves.iter().map(|m| m.0).filter(|s| tx.doc().shape(*s).is_some()).collect();
    for s in structure::settle(tx, eng, owner, &nodes, false)? {
        if !moved.contains(&s) {
            moved.push(s);
        }
    }
    Ok(moved)
}

/// the connectors after shapes moved: the ones glued to them and the ones whose way now runs
/// through them follow; Routing::Apply gives its style to those (`all`: to every connector of
/// `owner`) and routes them anew
pub fn reroute(tx: &mut Transaction, eng: &mut Engine, owner: Owner, moved: &[ShapeRef], routing: Routing, all: bool) -> Result<(), EditError> {
    let apply = matches!(routing, Routing::Apply { .. });
    let conns = {
        let doc = tx.doc();
        let mut conns = glue::dependents(doc, owner, moved);
        let boxes: Vec<[f64; 4]> = moved.iter().filter(|&&s| !glue::is_one_d(doc, s)).filter_map(|&s| ops::bounds(doc, &[s])).collect();
        for c in doc.top_shapes(owner) {
            if conns.contains(&c) || !glue::is_connector(doc, c) {
                continue;
            }
            if (all && apply) || glue::path(doc, c).windows(2).any(|w| boxes.iter().any(|b| crate::route::hits_box(w[0], w[1], b))) {
                conns.push(c);
            }
        }
        conns
    };
    match routing {
        Routing::Follow => glue::update(tx, eng, &conns),
        Routing::Apply { route, ext } => {
            let (a, b) = (tx.single("ShapeRouteStyle"), tx.single("ConLineRouteExt"));
            for &c in &conns {
                if !glue::is_connector(tx.doc(), c) {
                    continue;
                }
                let sheet = SheetRef::Shape(c);
                tx.set_constant(sheet, &a, Value::Num(route as f64), None)?;
                tx.set_constant(sheet, &b, Value::Num(ext as f64), None)?;
            }
            glue::reset(tx, eng, &conns)
        }
    }
}

/// Seitenlayout neu anordnen: the nodes of `owner` (`only`: a selection) laid out by `opts`,
/// what they carry along, the containers settled, the connectors routed (Routing::Apply: the
/// style on every connector of the page, or on the selection's) -> the shapes moved
pub fn relayout(tx: &mut Transaction, eng: &mut Engine, owner: Owner, only: &[ShapeRef], opts: &Opts, routing: Routing) -> Result<Vec<ShapeRef>, EditError> {
    let moves = plan(tx.doc(), owner, only, opts);
    let moved = move_nodes(tx, eng, owner, &moves)?;
    if moved.is_empty() && routing == Routing::Follow {
        return Ok(moved);
    }
    // a selection's nodes that stayed count for the style on its connectors
    let mut around = moved.clone();
    if !only.is_empty() {
        for s in diagram(tx.doc(), owner, only).nodes {
            if !around.contains(&s) {
                around.push(s);
            }
        }
    }
    reroute(tx, eng, owner, &around, routing, only.is_empty())?;
    Ok(moved)
}

// a number into a page cell unless it holds it already; a length in the unit it shows
fn put(tx: &mut Transaction, page: PageRef, cell: &str, v: f64, length: bool) -> Result<bool, EditError> {
    let s = SheetRef::Page(page);
    if num(tx.doc(), s, cell).is_some_and(|now| (now - v).abs() <= 1e-9) {
        return Ok(false);
    }
    let unit = if length {
        let doc = tx.doc();
        Some(doc.sym(cell).and_then(|c| doc.resolve(s, &CellAddr::single(c)).unit()).filter(|u| u.dimension() == Dimension::Length).unwrap_or(Unit::Inches))
    } else {
        None
    };
    let a = tx.single(cell);
    tx.set_constant(s, &a, Value::Num(v), unit)?;
    Ok(true)
}

/// the page's layout cells as the dialog sets them: PlaceStyle, PlaceDepth (the page default
/// 0 stays where it means the same), AvenueSizeX/Y (on paper); RouteStyle and LineRouteExt
/// when `route` is given; ResizePage when `resize` is -> whether anything changed
pub fn store(tx: &mut Transaction, eng: &mut Engine, page: PageRef, opts: &Opts, route: Option<(i64, i64)>, resize: Option<bool>) -> Result<bool, EditError> {
    tx.doc().page(page).ok_or(EditError::NoPage)?;
    let s = SheetRef::Page(page);
    let k = crate::pagesetup::scale(tx.doc(), page);
    let mut changed = put(tx, page, "PlaceStyle", opts.place.code() as f64, false)?;
    let depth_now = num(tx.doc(), s, "PlaceDepth").unwrap_or(0.0) as i64;
    if Depth::from_code(depth_now) != opts.depth {
        changed |= put(tx, page, "PlaceDepth", opts.depth.code() as f64, false)?;
    }
    changed |= put(tx, page, "AvenueSizeX", opts.spacing[0] * k, true)?;
    changed |= put(tx, page, "AvenueSizeY", opts.spacing[1] * k, true)?;
    if let Some((r, e)) = route {
        changed |= put(tx, page, "RouteStyle", r as f64, false)?;
        changed |= put(tx, page, "LineRouteExt", e as f64, false)?;
    }
    if let Some(on) = resize {
        changed |= put(tx, page, "ResizePage", if on { 1.0 } else { 0.0 }, false)?;
    }
    if changed {
        eng.recalc(tx, &[s])?;
    }
    Ok(changed)
}

/// Seite vergrößern (ResizePage): the page grows around its drawing with `margin` (drawing
/// units) where the drawing juts out, never shrinks; a drawing past the left or bottom edge
/// moves with the page's origin -> the move of the drawing (None: the page holds it already)
pub fn enlarge_page(tx: &mut Transaction, eng: &mut Engine, page: PageRef, margin: f64) -> Result<Option<[f64; 2]>, EditError> {
    tx.doc().page(page).ok_or(EditError::NoPage)?;
    let Some(b) = crate::pagesetup::content(tx.doc(), page) else { return Ok(None) };
    let [w, h] = crate::pagesetup::size(tx.doc(), page);
    let m = margin.max(0.0);
    let (lx, ly) = ((b[0] - m).min(0.0), (b[1] - m).min(0.0));
    let (hx, hy) = ((b[2] + m).max(w), (b[3] + m).max(h));
    if lx >= -EPS && ly >= -EPS && hx <= w + EPS && hy <= h + EPS {
        return Ok(None);
    }
    let d = [-lx, -ly];
    if d[0] > EPS || d[1] > EPS {
        crate::pagesetup::shift(tx, eng, page, d)?;
    }
    put(tx, page, "PageWidth", hx - lx, true)?;
    put(tx, page, "PageHeight", hy - ly, true)?;
    eng.recalc(tx, &[SheetRef::Page(page)])?;
    Ok(Some(d))
}
