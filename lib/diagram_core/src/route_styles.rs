// route_styles -- (JD21, CN-05, SS-40, SS-19) the routing styles of a page and of a connector with
// their directions, the ways they lay, and the shapes connectors may cross
//
//   Kind, RouteStyle { kind, dir }      a style and its direction (organization chart, flowchart,
//                                       tree and simple have one; the others None)
//   RouteStyle::from_code(c) / code()   Visio's RouteStyle / ShapeRouteStyle codes 0 .. 22
//   RouteStyle::new(kind, dir)          (a direction where the style has none is dropped, one
//                                       where it needs one defaults to top to bottom)
//   of(doc, connector) -> RouteStyle    its ShapeRouteStyle, else its page's RouteStyle (0 and
//                                       unknown codes: right angle)
//   pass(doc, shape) -> u8              route::PASS_X | PASS_Y from ShapePermeableX / Y
//   permeable_place(doc, shape)         ShapePermeablePlace: a layout may place shapes on it
//                                       (layout::diagram leaves it out of the obstacles)
//   dynamics_off(doc, owner) -> bool    the page's DynamicsOff
//   (crate) shape(doc, target) -> Option<Shape>   a dynamic end's shape: box, middle, outline
//   (crate) route(style, [shapes], &a_ports, &b_ports, ..) -> Routed   the way of a connector in its
//                                       style and whether Visio's right-angle router lays it
//   (crate) mark(tx, connector, router) ConFixedCode afterwards as Visio writes it (6 / 0)
//
// Visio's rules -- belegt (Microsoft Learn: the cells RouteStyle, ShapeRouteStyle, ConFixedCode,
// ShapePermeableX/Y/Place, DynamicsOff, CtrlAsInput, LineAdjustFrom/To, LineRouteExt; the archived
// Visio 2002 developer pages "Creating Routable and Other 1-D Connectors" and "Designing Shapes for
// Automatic Layout"; the Visio 2000 user's guide, chapter 6):
// * RouteStyle / ShapeRouteStyle: 0 default (right angle), 1 right angle, 2 straight,
//   3/4/10/11 organization chart top to bottom / left to right / bottom to top / right to left,
//   5/6/12/13 flowchart, 7/8/14/15 tree (the same four directions), 9 network, 16 centre to
//   centre, 17/18/19/20 simple (top to bottom, left to right, bottom to top, right to left),
//   21 simple horizontal-vertical, 22 simple vertical-horizontal.  ShapeRouteStyle 0 takes the
//   page's.
// * ShapePermeableX / Y: TRUE lets connectors route horizontally / vertically through the
//   (placeable) shape.  ShapePermeablePlace: shapes may be placed on top of it by a layout
//   (layout.rs).  ObjType 4: routing behaves as if the shape did not exist (glue.rs).
// * DynamicsOff: TRUE turns off "placeable shapes move and connectors reroute around other shapes
//   and connectors".  CtrlAsInput: which shape is the parent of a control handle's glue -- not
//   routing, kept for Visio.  LineAdjustFrom / LineAdjustTo: which connectors routed on top of each
//   other are spaced apart / lined up -- kept for Visio and shown in Seite einrichten (JD21v: Visio
//   16 spread connectors at one shape apart whatever the two cells said, 0 .. 2; see below).
//   LineRouteExt / ConLineRouteExt: 0 default, 1 straight, 2 curved (the appearance, glue.rs).
// * ConFixedCode: 0 reroute freely, 1 reroute as needed (manual reroute), 2 never reroute,
//   3 reroute on crossover, 4 .. 6 internal (route_edit.rs keeps a way by hand).
//
// (JD21v) The ways as a real Visio 16 lays them -- belegt by the counter-check of 2026-11-27
// (Documents\JFLib-jd21-visio-check: visio-run3 grid.txt, every style against a child in ten places
// around its parent, the tests in route_styles_visio_tests.rs; visio-route-1 / -2, Visio's own
// drawings with obstacles, permeable shapes, WalkPreference, ConFixedCode; visio-run2 [route] and
// visio-route-3, our drawing after a move: 63 of 100 ways the same; visio-obst-1 / -2, what Visio
// routes around and how it spreads connectors, then 76 of 100, the rest below under "Not here").
// A is the connector's begin shape, B its end shape (both glued dynamically); "the stub" is half
// the page's AvenueSize (route clearance, glue::opts_for: 3.75 mm by default;
// AvenueSize 0.5 in gave 0.25 in, LineToNode 0.4 in nothing); "overlap" means the two boxes share
// a stretch along an axis:
// * The direction of a style decides nothing but its axis: bottom to top lays the same ways as top
//   to bottom, right to left as left to right (the sides come from where B lies).
// * Right angle (0, 1) and network (9; not straight): B beside A (overlap across the axis): one
//   straight line in the middle of the overlap.  Else an L that leaves A vertically (top or bottom,
//   at its middle) and comes into B's side from the side (at its middle) -- for squares, tall and
//   wide shapes, far and near, whatever WalkPreference says (it has no effect on these ways), and
//   the other way round when the connector runs from the lower shape (the begin leaves
//   vertically).  Obstacles (placeable shapes): each leg of the straight line or the L runs in the
//   middle of the part of its band (the stretch of the shape's side it may leave from) that no
//   obstacle between the shapes covers -- the nearest such part, then the one towards the other
//   shape, then the higher one; no part free: the L leaving A sideways (horizontal, then vertical);
//   neither: a detour around the obstacles at the stub's distance (route.rs; Visio's detours were
//   matched in form, not always in the side they pass).  Visio writes ConFixedCode 6.  (visio-obst-1
//   [sprd]) The straight pieces of the other connectors glued to A or B that run along a leg split
//   its band as well (no clearance to them): a second connector leaving A's bottom runs in the
//   middle of the part the first leaves free (A 3.2 .. 4.8 wide, the first at 4.0: 4.4 towards C,
//   3.6 towards B), each connector routed again keeps out of the others' way; connectors between
//   other shapes do not count.  Placeable is ObjType with bit 1 (not 4): glue.rs.
// * Straight (2): B beside A: the right-angle's straight line (ConFixedCode 6); else a line from
//   A's corner nearest to B to B's corner nearest to A (0).  Obstacles do not matter.
// * Centre to centre (16): the line between the middles, from outline to outline (6).
// * Organization chart (3/4/10/11) and flowchart (5/6/12/13): B beyond A along the axis: out of A's
//   side facing B at its middle, into B's side facing A at its middle; one straight line when the
//   middles line up, else a Z whose middle piece runs one stub before B (organization chart) or one
//   stub after A (flowchart); where an obstacle lies on that piece, at the other of the two places
//   (Visio did so), where both are blocked or the straight line is, around the obstacles keeping to
//   the two sides.  B beside A (overlap along the axis): the right-angle router (ConFixedCode 6),
//   else 0.
// * Tree (7/8/14/15): one straight line from the middle of A's side facing B to the middle of B's
//   side facing A, along the axis -- across it when B lies beside A; obstacles do not matter (0).
// * Simple (17 .. 20): like the tree's sides, one line when the middles line up, else a Z bending
//   halfway between the two sides; obstacles do not matter (0).
// * Simple horizontal-vertical (21) / vertical-horizontal (22): an L out of A's side (21) or its top
//   or bottom (22), into B's top or bottom (21) or its side (22), through the middles; B beside A
//   (no L): one line when the middles line up, else a Z -- above or below A bending one stub before
//   B, to its left or right one stub after A (both styles alike); obstacles do not matter (0).
// * After routing Visio writes ConFixedCode 6 where its right-angle router laid the way and 0 where
//   a style laid its own (also over 4, 5, 6 and 0 set before); 1, 2 and 3 stay (mark).
// Angenommen: ends glued to connection points or free keep the rules from before (the sides of
// the direction, the right-angle search); a stub longer than the gap gives the middle; Visio's only
// data had the parent at the begin (organization chart before B, flowchart after A are taken as
// before the end and after the begin).  Not here: in our own drawings Visio laid the stubs of
// organization chart and flowchart larger (0.264 in for our connectors, 0.189 in for its own
// there; 0.148 in in its drawings, half the AvenueSize): Visio reads our page's AvenueSize as
// 7.5 mm, and 0.264 stayed with AvenueSize 0.375 in, LineToNode 0.4 in and without arrows
// (visio-obst-2 [stub]) -- unexplained, the drawing's theme is left; the counter-check of our
// drawing after the moves: 76 of 100 ways the same, the other 24 these stubs.  Two connectors
// between the same two shapes: when one end moves Visio left the first where it was (the second
// avoided it), here both are routed again.

use alloc::vec::Vec;

use crate::cell::Value;
use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::glue::{self, Target};
use crate::layout::Dir;
use crate::route::{self, Opts, Port, PASS_X, PASS_Y};
use crate::sheet::CellAddr;

/// a routing style without its direction
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// right angles around obstacles (also the page default)
    RightAngle,
    /// one straight line, corner to corner (beside each other: across in the middle)
    Straight,
    /// right angles, the bend one stub before the end's shape
    OrgChart,
    /// right angles, the bend one stub after the begin's shape
    Flowchart,
    /// one straight line between the middles of the facing sides
    Tree,
    /// right angles, as RightAngle (Visio lays the same ways)
    Network,
    /// right angles bending halfway, around nothing
    Simple,
    /// leave horizontally, arrive vertically, around nothing
    SimpleHV,
    /// leave vertically, arrive horizontally, around nothing
    SimpleVH,
    /// one straight line between the middles
    CentreToCentre,
}

impl Kind {
    /// whether the style has a direction
    pub fn directed(self) -> bool {
        matches!(self, Kind::OrgChart | Kind::Flowchart | Kind::Tree | Kind::Simple)
    }

    /// whether the style draws one straight line (straight, centre to centre)
    pub fn straight(self) -> bool {
        matches!(self, Kind::Straight | Kind::CentreToCentre)
    }
}

/// a routing style and its direction (Visio's RouteStyle / ShapeRouteStyle)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct RouteStyle {
    /// the style
    pub kind: Kind,
    /// the flow (directed kinds only; the ways use only its axis)
    pub dir: Option<Dir>,
}

// the four directions in the order of Visio's codes: NS, WE, SN, EW
const DIRS: [Dir; 4] = [Dir::Down, Dir::Right, Dir::Up, Dir::Left];

impl RouteStyle {
    /// right angles (RouteStyle 1)
    pub const RIGHT_ANGLE: RouteStyle = RouteStyle { kind: Kind::RightAngle, dir: None };

    /// a style in a direction (dropped where the style has none; top to bottom where it needs one)
    pub fn new(kind: Kind, dir: Option<Dir>) -> RouteStyle {
        let dir = if kind.directed() { Some(dir.unwrap_or(Dir::Down)) } else { None };
        RouteStyle { kind, dir }
    }

    /// a RouteStyle / ShapeRouteStyle code (None: 0, the default, and unknown codes)
    pub fn from_code(code: i64) -> Option<RouteStyle> {
        let d = |i: usize| Some(DIRS[i]);
        let (kind, dir) = match code {
            1 => (Kind::RightAngle, None),
            2 => (Kind::Straight, None),
            3 => (Kind::OrgChart, d(0)),
            4 => (Kind::OrgChart, d(1)),
            5 => (Kind::Flowchart, d(0)),
            6 => (Kind::Flowchart, d(1)),
            7 => (Kind::Tree, d(0)),
            8 => (Kind::Tree, d(1)),
            9 => (Kind::Network, None),
            10 => (Kind::OrgChart, d(2)),
            11 => (Kind::OrgChart, d(3)),
            12 => (Kind::Flowchart, d(2)),
            13 => (Kind::Flowchart, d(3)),
            14 => (Kind::Tree, d(2)),
            15 => (Kind::Tree, d(3)),
            16 => (Kind::CentreToCentre, None),
            17 => (Kind::Simple, d(0)),
            18 => (Kind::Simple, d(1)),
            19 => (Kind::Simple, d(2)),
            20 => (Kind::Simple, d(3)),
            21 => (Kind::SimpleHV, None),
            22 => (Kind::SimpleVH, None),
            _ => return None,
        };
        Some(RouteStyle { kind, dir })
    }

    /// its RouteStyle / ShapeRouteStyle code
    pub fn code(self) -> i64 {
        let k = DIRS.iter().position(|&d| Some(d) == self.dir).unwrap_or(0) as i64;
        match self.kind {
            Kind::RightAngle => 1,
            Kind::Straight => 2,
            Kind::OrgChart => [3, 4, 10, 11][k as usize],
            Kind::Flowchart => [5, 6, 12, 13][k as usize],
            Kind::Tree => [7, 8, 14, 15][k as usize],
            Kind::Network => 9,
            Kind::CentreToCentre => 16,
            Kind::Simple => 17 + k,
            Kind::SimpleHV => 21,
            Kind::SimpleVH => 22,
        }
    }

    // the axis of the style's direction: 0 x (left to right, right to left), 1 y
    fn axis(self) -> usize {
        match self.dir {
            Some(Dir::Right | Dir::Left) => 0,
            _ => 1,
        }
    }
}

fn num(doc: &Document, sheet: SheetRef, cell: &str) -> Option<f64> {
    let c = doc.sym(cell)?;
    doc.resolve(sheet, &CellAddr::single(c)).num().filter(|v| v.is_finite())
}

// a boolean cell (TRUE is 1 in the files; text such as "TRUE" counts too)
fn truthy(doc: &Document, sheet: SheetRef, cell: &str) -> bool {
    let Some(c) = doc.sym(cell) else { return false };
    match doc.resolve(sheet, &CellAddr::single(c)).value() {
        Value::Num(v) => v != 0.0 && v.is_finite(),
        Value::Text(t) => doc.str(t).eq_ignore_ascii_case("true"),
        Value::Absent => false,
    }
}

/// the style a connector routes in: its own ShapeRouteStyle, else its page's RouteStyle
pub fn of(doc: &Document, connector: ShapeRef) -> RouteStyle {
    let own = num(doc, SheetRef::Shape(connector), "ShapeRouteStyle").map(|v| v as i64).and_then(RouteStyle::from_code);
    let page = || match connector.owner {
        Owner::Page(p) => num(doc, SheetRef::Page(p), "RouteStyle").map(|v| v as i64).and_then(RouteStyle::from_code),
        Owner::Master(_) => None,
    };
    own.or_else(page).unwrap_or(RouteStyle::RIGHT_ANGLE)
}

/// how connectors may cross a shape: PASS_X (ShapePermeableX), PASS_Y (ShapePermeableY)
pub fn pass(doc: &Document, shape: ShapeRef) -> u8 {
    let s = SheetRef::Shape(shape);
    (if truthy(doc, s, "ShapePermeableX") { PASS_X } else { 0 }) | (if truthy(doc, s, "ShapePermeableY") { PASS_Y } else { 0 })
}

/// whether a layout may place shapes on top of the shape (ShapePermeablePlace)
pub fn permeable_place(doc: &Document, shape: ShapeRef) -> bool {
    truthy(doc, SheetRef::Shape(shape), "ShapePermeablePlace")
}

/// whether the page turns dynamics off (DynamicsOff): connectors do not reroute around shapes
/// moved onto them
pub fn dynamics_off(doc: &Document, owner: Owner) -> bool {
    match owner {
        Owner::Page(p) => truthy(doc, SheetRef::Page(p), "DynamicsOff"),
        Owner::Master(_) => false,
    }
}

// ---------------------------------------------------------------- the ways of the styles

/// (JD21v) the shape of a dynamically glued end: its box and middle on the page, its outline
/// (where the end docks)
pub(crate) struct Shape {
    bx: [f64; 4],
    mid: [f64; 2],
    segs: Vec<[[f64; 2]; 2]>,
}

impl Shape {
    // where the end docks on side `d` (an axis direction, outward) at coordinate t across it
    fn side(&self, d: [f64; 2], t: f64) -> [f64; 2] {
        let mut o = self.mid;
        o[if d[0] != 0.0 { 1 } else { 0 }] = t;
        glue::dock(&self.segs, o, d, Some(self.bx))
    }

    // where the line from the middle towards p leaves the outline
    fn toward(&self, p: [f64; 2]) -> [f64; 2] {
        glue::dock(&self.segs, self.mid, [p[0] - self.mid[0], p[1] - self.mid[1]], Some(self.bx))
    }
}

/// (JD21v) the shape of an end glued to a whole shape (None: glued to a point, or free)
pub(crate) fn shape(doc: &Document, t: Target) -> Option<Shape> {
    match t {
        Target::Shape(s) => Some(Shape { bx: glue::page_box(doc, s)?, mid: glue::centre(doc, s), segs: glue::outline(doc, s) }),
        _ => None,
    }
}

/// (JD21v) a connector's way and who lays it
pub(crate) struct Routed {
    /// the way on the page, Begin first
    pub(crate) way: Vec<[f64; 2]>,
    /// Visio's right-angle router lays it (ConFixedCode 6 afterwards), not the style itself (0)
    pub(crate) router: bool,
}

fn routed(way: Vec<[f64; 2]>, router: bool) -> Routed {
    Routed { way, router }
}

/// (JD21v) ConFixedCode after routing, as Visio writes it: 6 where its right-angle router laid the
/// way, 0 where the style laid its own; 1, 2 (kept ways) and 3 (reroute on crossover) stay
pub(crate) fn mark(tx: &mut Transaction, c: ShapeRef, router: bool) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(c);
    let now = num(tx.doc(), sheet, "ConFixedCode").map_or(0, |v| v as i64);
    let want = if router { 6 } else { 0 };
    if matches!(now, 1..=3) || now == want {
        return Ok(());
    }
    let a = tx.single("ConFixedCode");
    tx.set_constant(sheet, &a, Value::Num(want as f64), None)
}

// a tolerance for coordinates of this size
fn tol(v: f64) -> f64 {
    1e-9 * (1.0 + v.abs())
}

// the unit vector along axis k with sign s
fn unit(k: usize, s: f64) -> [f64; 2] {
    if k == 0 { [s, 0.0] } else { [0.0, s] }
}

fn neg(d: [f64; 2]) -> [f64; 2] {
    [-d[0], -d[1]]
}

// the side of box b facing direction s along axis k
fn edge(b: &[f64; 4], k: usize, s: f64) -> f64 {
    if s > 0.0 { b[k + 2] } else { b[k] }
}

// the stretch two boxes share along axis k (None: none of positive length)
fn overlap(a: &[f64; 4], b: &[f64; 4], k: usize) -> Option<(f64, f64)> {
    let (l, h) = (a[k].max(b[k]), a[k + 2].min(b[k + 2]));
    (h - l > tol(h)).then_some((l, h))
}

// +1 when b's middle lies further along axis k than a's, else -1
fn toward(a: &Shape, b: &Shape, k: usize) -> f64 {
    if b.mid[k] < a.mid[k] { -1.0 } else { 1.0 }
}

// a box from a stretch on axis k and one on the other axis
fn rect(k: usize, on: (f64, f64), across: (f64, f64)) -> [f64; 4] {
    let (l0, h0) = (on.0.min(on.1), on.0.max(on.1));
    let (l1, h1) = (across.0.min(across.1), across.0.max(across.1));
    if k == 0 { [l0, l1, h0, h1] } else { [l1, l0, h1, h0] }
}

// whether two boxes share an area
fn meets(o: &[f64; 4], r: &[f64; 4]) -> bool {
    let t = tol(r[2].abs().max(r[3].abs()));
    o[0] < r[2] - t && r[0] < o[2] - t && o[1] < r[3] - t && r[1] < o[3] - t
}

fn holds(b: &[f64; 4], p: [f64; 2]) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
}

// the obstacles that count between two shapes: not the shapes, not what holds them (a container)
fn others(a: &Shape, b: &Shape, boxes: &[[f64; 4]], pass: &[u8]) -> (Vec<[f64; 4]>, Vec<u8>) {
    let keep: Vec<usize> = (0..boxes.len()).filter(|&n| !holds(&boxes[n], a.mid) && !holds(&boxes[n], b.mid)).collect();
    (keep.iter().map(|&n| boxes[n]).collect(), keep.iter().map(|&n| pass.get(n).copied().unwrap_or(0)).collect())
}

// the parts of the band [lo, hi] on axis k that no obstacle reaching into `region` covers; an
// obstacle with the flag `through` lets the way cross it (PASS_Y for a vertical way).  (JD21v)
// The pieces of related connectors that run along the way (across k) split the band where they
// lie in it (Visio spreads connectors leaving one side so)
fn free(band: (f64, f64), k: usize, region: [f64; 4], ob: &[[f64; 4]], op: &[u8], through: u8, lines: &[[[f64; 2]; 2]]) -> Vec<(f64, f64)> {
    let mut parts = alloc::vec![band];
    let along = lines.iter().filter(|l| (l[0][k] - l[1][k]).abs() <= tol(l[0][k])).map(|l| rect(k, (l[0][k], l[1][k]), (l[0][1 - k], l[1][1 - k])));
    let cover: Vec<([f64; 4], u8)> = ob.iter().enumerate().map(|(n, o)| (*o, op.get(n).copied().unwrap_or(0))).chain(along.map(|b| (b, 0))).collect();
    for (o, p) in &cover {
        if p & through != 0 || !meets(o, &region) {
            continue;
        }
        let (c0, c1) = (o[k], o[k + 2]);
        let mut next = Vec::with_capacity(parts.len() + 1);
        for (l, h) in parts {
            if c1 <= l || c0 >= h {
                next.push((l, h));
                continue;
            }
            if c0 > l {
                next.push((l, c0));
            }
            if c1 < h {
                next.push((c1, h));
            }
        }
        parts = next;
    }
    parts.retain(|&(l, h)| h - l > tol(h));
    parts
}

// the middle of the free part nearest to `def`, then nearest to `other`, then the higher one
fn pick(parts: &[(f64, f64)], def: f64, other: f64) -> Option<f64> {
    let dist = |p: &(f64, f64), x: f64| if x < p.0 { p.0 - x } else if x > p.1 { x - p.1 } else { 0.0 };
    let key = |p: &(f64, f64)| [dist(p, def), ((p.0 + p.1) * 0.5 - other).abs(), -(p.0 + p.1)];
    // distances the same but for rounding count as the same
    let cmp = |a: f64, b: f64| if (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs())) { core::cmp::Ordering::Equal } else { a.total_cmp(&b) };
    parts
        .iter()
        .min_by(|x, y| {
            let (a, b) = (key(x), key(y));
            cmp(a[0], b[0]).then(cmp(a[1], b[1])).then(a[2].total_cmp(&b[2]))
        })
        .map(|p| (p.0 + p.1) * 0.5)
}

// whether a way runs through an obstacle where it may not
fn blocked(way: &[[f64; 2]], ob: &[[f64; 4]], op: &[u8]) -> bool {
    way.windows(2).any(|s| ob.iter().enumerate().any(|(n, o)| route::hits_pass(s[0], s[1], o, op.get(n).copied().unwrap_or(0))))
}

// through the two shapes as one would pass them: whether their boxes overlap both ways
fn tangled(a: &Shape, b: &Shape) -> bool {
    overlap(&a.bx, &b.bx, 0).is_some() && overlap(&a.bx, &b.bx, 1).is_some()
}

// the ports of the full lists that face d (all when none does: a static or free end)
fn sides(ports: &[Port], want: impl Fn([f64; 2]) -> bool) -> Vec<usize> {
    let some: Vec<usize> = (0..ports.len()).filter(|&k| ports[k].dir.is_some_and(&want)).collect();
    if some.is_empty() {
        (0..ports.len()).collect()
    } else {
        some
    }
}

// the cheapest way between the chosen ports -> indices into the full lists
fn among(ca: &[Port], cb: &[Port], ia: &[usize], ib: &[usize], boxes: &[[f64; 4]], pass: &[u8], opts: &Opts) -> (usize, usize, Vec<[f64; 2]>) {
    let sa: Vec<Port> = ia.iter().map(|&k| ca[k]).collect();
    let sb: Vec<Port> = ib.iter().map(|&k| cb[k]).collect();
    let (i, j, way) = route::best_through(&sa, &sb, boxes, pass, opts);
    (ia.get(i).copied().unwrap_or(0), ib.get(j).copied().unwrap_or(0), way)
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn same_dir(a: [f64; 2], b: [f64; 2]) -> bool {
    dot(a, b) > 0.9
}

// the way from the middle of A's side facing along d to the middle of B's side facing back,
// around the obstacles
#[allow(clippy::too_many_arguments)]
fn kept_sides(d: [f64; 2], a: &Shape, b: &Shape, ca: &[Port], cb: &[Port], boxes: &[[f64; 4]], pass: &[u8], opts: &Opts) -> Vec<[f64; 2]> {
    let ia: Vec<usize> = middles(ca, a).into_iter().filter(|&i| ca[i].dir.is_some_and(|v| same_dir(v, d))).collect();
    let ib: Vec<usize> = middles(cb, b).into_iter().filter(|&i| cb[i].dir.is_some_and(|v| same_dir(v, neg(d)))).collect();
    let (ia, ib) = (if ia.is_empty() { sides(ca, |v| same_dir(v, d)) } else { ia }, if ib.is_empty() { sides(cb, |v| same_dir(v, neg(d))) } else { ib });
    among(ca, cb, &ia, &ib, boxes, pass, opts).2
}

// a Z from p0 to p3 along axis k whose middle piece runs at t (one line when they line up)
fn zed(p0: [f64; 2], p3: [f64; 2], k: usize, t: f64) -> Vec<[f64; 2]> {
    let j = 1 - k;
    if (p0[j] - p3[j]).abs() <= tol(p0[j]) {
        return alloc::vec![p0, p3];
    }
    let (mut p1, mut p2) = (p0, p3);
    p1[k] = t;
    p2[k] = t;
    alloc::vec![p0, p1, p2, p3]
}

// a bend at t when it lies strictly between the two ends along k, else halfway
fn between(t: f64, from: f64, to: f64) -> f64 {
    let (l, h) = (from.min(to), from.max(to));
    if t > l + tol(l) && t < h - tol(h) { t } else { (from + to) * 0.5 }
}

// Visio's right-angle router without obstacles in the way: the straight line in the middle of the
// free part of the overlap
fn straight_channel(a: &Shape, b: &Shape, ob: &[[f64; 4]], op: &[u8], lines: &[[[f64; 2]; 2]]) -> Option<Vec<[f64; 2]>> {
    for k in 0..2 {
        let Some((l, h)) = overlap(&a.bx, &b.bx, k) else { continue };
        let j = 1 - k;
        let s = toward(a, b, j);
        let (from, to) = (edge(&a.bx, j, s), edge(&b.bx, j, -s));
        if (to - from) * s <= tol(to) {
            continue;
        }
        let parts = free((l, h), k, rect(k, (l, h), (from, to)), ob, op, if j == 0 { PASS_X } else { PASS_Y }, lines);
        let t = pick(&parts, (l + h) * 0.5, b.mid[k])?;
        let d = unit(j, s);
        return Some(alloc::vec![a.side(d, t), b.side(neg(d), t)]);
    }
    None
}

// Visio's L: A's leg along axis v (1: vertical first) leaves the side facing B, B's leg along the
// other axis comes into B's side facing A; each leg in the middle of the free part of its band
fn l_channel(a: &Shape, b: &Shape, v: usize, ob: &[[f64; 4]], op: &[u8], lines: &[[[f64; 2]; 2]]) -> Option<Vec<[f64; 2]>> {
    let h = 1 - v;
    let (sv, sh) = (toward(a, b, v), toward(a, b, h));
    let a_side = edge(&a.bx, v, sv);
    let b_side = edge(&b.bx, h, -sh);
    // A's leg: a coordinate on h within A, short of B's side; B's leg: on v within B, past A's side
    let band_a = if sh > 0.0 { (a.bx[h], a.bx[h + 2].min(b_side)) } else { (a.bx[h].max(b_side), a.bx[h + 2]) };
    let band_b = if sv > 0.0 { (b.bx[v].max(a_side), b.bx[v + 2]) } else { (b.bx[v], b.bx[v + 2].min(a_side)) };
    if band_a.1 - band_a.0 <= tol(band_a.1) || band_b.1 - band_b.0 <= tol(band_b.1) {
        return None;
    }
    // the stretch each leg may run over: to the far side of the other shape
    let region_a = rect(h, band_a, (a_side, edge(&b.bx, v, sv)));
    let region_b = rect(v, band_b, (edge(&a.bx, h, -sh), b_side));
    let flag = |axis: usize| if axis == 0 { PASS_X } else { PASS_Y };
    let x = pick(&free(band_a, h, region_a, ob, op, flag(v), lines), a.mid[h], b.mid[h])?;
    let y = pick(&free(band_b, v, region_b, ob, op, flag(h), lines), b.mid[v], a.mid[v])?;
    let p0 = a.side(unit(v, sv), x);
    let p2 = b.side(unit(h, -sh), y);
    let mut p1 = [0.0; 2];
    p1[h] = x;
    p1[v] = y;
    Some(route::simplify(alloc::vec![p0, p1, p2]))
}

// Visio's right-angle router between two dynamic ends; `both`: a detour between shapes beside each
// other comes into B's facing side too (a style's fallback), else only A's is kept
#[allow(clippy::too_many_arguments)]
fn right_angle(a: &Shape, b: &Shape, ob: &[[f64; 4]], op: &[u8], lines: &[[[f64; 2]; 2]], ca: &[Port], cb: &[Port], boxes: &[[f64; 4]], pass: &[u8], opts: &Opts, both: bool) -> Routed {
    if tangled(a, b) {
        return routed(route::best_through(ca, cb, boxes, pass, opts).2, true);
    }
    if let Some(w) = straight_channel(a, b, ob, op, lines).or_else(|| l_channel(a, b, 1, ob, op, lines)).or_else(|| l_channel(a, b, 0, ob, op, lines)) {
        return routed(w, true);
    }
    // a detour from and to the middles of the sides (Visio's detours did); shapes beside each other
    // leave A's side facing B
    let (ma, mb) = (middles(ca, a), middles(cb, b));
    for k in 0..2 {
        if overlap(&a.bx, &b.bx, k).is_some() {
            let d = unit(1 - k, toward(a, b, 1 - k));
            let ia: Vec<usize> = ma.iter().copied().filter(|&i| ca[i].dir.is_some_and(|v| same_dir(v, d))).collect();
            let ib: Vec<usize> = if both { mb.iter().copied().filter(|&i| cb[i].dir.is_some_and(|v| same_dir(v, neg(d)))).collect() } else { mb.clone() };
            if !ia.is_empty() && !ib.is_empty() {
                return routed(among(ca, cb, &ia, &ib, boxes, pass, opts).2, true);
            }
        }
    }
    routed(among(ca, cb, &ma, &mb, boxes, pass, opts).2, true)
}

// the ports in the middle of a shape's sides (glue offers more where the other end lines up)
fn middles(ports: &[Port], s: &Shape) -> Vec<usize> {
    let some: Vec<usize> = (0..ports.len())
        .filter(|&i| match ports[i].dir {
            Some(d) => {
                let k = if d[0] != 0.0 { 1 } else { 0 };
                (ports[i].at[k] - s.mid[k]).abs() <= tol(s.mid[k])
            }
            None => false,
        })
        .collect();
    if some.is_empty() {
        (0..ports.len()).collect()
    } else {
        some
    }
}

/// (JD21v) the way of a connector in its style: `shapes` the shapes its two ends are glued to as a
/// whole (shape()), `ca`, `cb` the ports glue offers for them (glue::candidates), the obstacles with
/// how they may be crossed, `lines` the straight pieces of the other connectors glued to the same
/// shapes (the right-angle router lays its legs between them)
#[allow(clippy::too_many_arguments)]
pub(crate) fn route(style: RouteStyle, shapes: [Option<&Shape>; 2], ca: &[Port], cb: &[Port], boxes: &[[f64; 4]], pass: &[u8], lines: &[[[f64; 2]; 2]], opts: &Opts) -> Routed {
    if ca.is_empty() || cb.is_empty() {
        return routed(Vec::new(), true);
    }
    let [Some(a), Some(b)] = shapes else { return by_ports(style, ca, cb, boxes, pass, opts) };
    if tangled(a, b) {
        return routed(route::best_through(ca, cb, boxes, pass, opts).2, true);
    }
    let (ob, op) = others(a, b, boxes, pass);
    let stub = opts.clearance;
    match style.kind {
        Kind::RightAngle | Kind::Network => right_angle(a, b, &ob, &op, lines, ca, cb, boxes, pass, opts, false),
        Kind::CentreToCentre => routed(alloc::vec![a.toward(b.mid), b.toward(a.mid)], true),
        Kind::Straight => {
            for k in 0..2 {
                if let Some((l, h)) = overlap(&a.bx, &b.bx, k) {
                    let d = unit(1 - k, toward(a, b, 1 - k));
                    let t = (l + h) * 0.5;
                    return routed(alloc::vec![a.side(d, t), b.side(neg(d), t)], true);
                }
            }
            let corner = |s: &Shape, o: &Shape| [edge(&s.bx, 0, toward(s, o, 0)), edge(&s.bx, 1, toward(s, o, 1))];
            routed(alloc::vec![a.toward(corner(a, b)), b.toward(corner(b, a))], false)
        }
        Kind::OrgChart | Kind::Flowchart => {
            let k = style.axis();
            if overlap(&a.bx, &b.bx, k).is_some() {
                return right_angle(a, b, &ob, &op, lines, ca, cb, boxes, pass, opts, true);
            }
            let (j, s) = (1 - k, toward(a, b, k));
            let d = unit(k, s);
            let (p0, p3) = (a.side(d, a.mid[j]), b.side(neg(d), b.mid[j]));
            let (near_b, near_a) = (between(p3[k] - s * stub, p0[k], p3[k]), between(p0[k] + s * stub, p0[k], p3[k]));
            let order = if style.kind == Kind::OrgChart { [near_b, near_a] } else { [near_a, near_b] };
            for t in order {
                let w = zed(p0, p3, k, t);
                if !blocked(&w, &ob, &op) {
                    return routed(w, false);
                }
            }
            routed(kept_sides(d, a, b, ca, cb, boxes, pass, opts), false)
        }
        Kind::Tree | Kind::Simple => {
            let k0 = style.axis();
            let k = if overlap(&a.bx, &b.bx, k0).is_some() { 1 - k0 } else { k0 };
            let (j, s) = (1 - k, toward(a, b, k));
            let d = unit(k, s);
            let (p0, p3) = (a.side(d, a.mid[j]), b.side(neg(d), b.mid[j]));
            let w = if style.kind == Kind::Tree { alloc::vec![p0, p3] } else { zed(p0, p3, k, (p0[k] + p3[k]) * 0.5) };
            routed(w, false)
        }
        Kind::SimpleHV | Kind::SimpleVH => {
            let (sx, sy) = (toward(a, b, 0), toward(a, b, 1));
            let w = if overlap(&a.bx, &b.bx, 0).is_some() {
                // above or below: bending one stub before B
                let (p0, p3) = (a.side(unit(1, sy), a.mid[0]), b.side(unit(1, -sy), b.mid[0]));
                zed(p0, p3, 1, between(p3[1] - sy * stub, p0[1], p3[1]))
            } else if overlap(&a.bx, &b.bx, 1).is_some() {
                // to the left or right: bending one stub after A
                let (p0, p3) = (a.side(unit(0, sx), a.mid[1]), b.side(unit(0, -sx), b.mid[1]));
                zed(p0, p3, 0, between(p0[0] + sx * stub, p0[0], p3[0]))
            } else if style.kind == Kind::SimpleHV {
                alloc::vec![a.side(unit(0, sx), a.mid[1]), [b.mid[0], a.mid[1]], b.side(unit(1, -sy), b.mid[0])]
            } else {
                alloc::vec![a.side(unit(1, sy), a.mid[0]), [a.mid[0], b.mid[1]], b.side(unit(0, -sx), b.mid[1])]
            };
            routed(w, false)
        }
    }
}

// (tree, ends not both glued to whole shapes) the middle piece of a Z between the two ends moves to
// one clearance past the begin's shape, where that stays clear of the obstacles
fn bend_early(way: Vec<[f64; 2]>, d: [f64; 2], begin: &[f64; 4], boxes: &[[f64; 4]], pass: &[u8], c: f64) -> Vec<[f64; 2]> {
    if way.len() != 4 {
        return way;
    }
    let k = if d[0] != 0.0 { 0 } else { 1 };
    let (p0, p1, p2, p3) = (way[0], way[1], way[2], way[3]);
    // the first and last pieces run along d, the middle one across it
    let t = 1e-9 * (1.0 + p1[0].abs().max(p1[1].abs()));
    if (p1[1 - k] - p0[1 - k]).abs() > t || (p3[1 - k] - p2[1 - k]).abs() > t || (p1[k] - p2[k]).abs() > t {
        return way;
    }
    let far = [[begin[0], begin[1]], [begin[2], begin[1]], [begin[0], begin[3]], [begin[2], begin[3]]].iter().map(|p| dot(*p, d)).fold(f64::NEG_INFINITY, f64::max);
    let at = (far + c) * if d[k] < 0.0 { -1.0 } else { 1.0 };
    // still between the two ends along d
    let (a0, a3) = (p0[k], p3[k]);
    if !((a0 <= at && at <= a3) || (a3 <= at && at <= a0)) || (at - p1[k]).abs() <= t {
        return way;
    }
    let (mut q1, mut q2) = (p1, p2);
    q1[k] = at;
    q2[k] = at;
    let clear = [(p0, q1), (q1, q2), (q2, p3)].iter().all(|&(s, e)| boxes.iter().enumerate().all(|(n, b)| !route::hits_pass(s, e, b, pass.get(n).copied().unwrap_or(0))));
    if clear {
        alloc::vec![p0, q1, q2, p3]
    } else {
        way
    }
}

// an end glued to a connection point or free (no Visio data): the right-angle search; a style with
// a direction keeps to the sides of its direction (organization chart and tree always, flowchart
// where the end's shape lies two clearances further along it), simple around nothing; straight
// styles one line
fn by_ports(style: RouteStyle, ca: &[Port], cb: &[Port], boxes: &[[f64; 4]], pass: &[u8], opts: &Opts) -> Routed {
    let horizontal = |v: [f64; 2]| v[0] != 0.0 && v[1] == 0.0;
    let vertical = |v: [f64; 2]| v[0] == 0.0 && v[1] != 0.0;
    let extent = |p: &Port| p.own.unwrap_or([p.at[0], p.at[1], p.at[0], p.at[1]]);
    let span = |b: &[f64; 4], d: [f64; 2]| {
        let v = [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]].map(|p| dot(p, d));
        (v.iter().copied().fold(f64::INFINITY, f64::min), v.iter().copied().fold(f64::NEG_INFINITY, f64::max))
    };
    match style.kind {
        Kind::Straight | Kind::CentreToCentre => {
            let w = route::straight(&ca[0], &cb[0]);
            let level = w.len() == 2 && ((w[0][0] - w[1][0]).abs() <= tol(w[0][0]) || (w[0][1] - w[1][1]).abs() <= tol(w[0][1]));
            routed(w, style.kind == Kind::CentreToCentre || level)
        }
        Kind::SimpleHV | Kind::SimpleVH => {
            let hv = style.kind == Kind::SimpleHV;
            let ia = sides(ca, |v| if hv { horizontal(v) } else { vertical(v) });
            let ib = sides(cb, |v| if hv { vertical(v) } else { horizontal(v) });
            routed(among(ca, cb, &ia, &ib, &[], &[], opts).2, false)
        }
        Kind::OrgChart | Kind::Flowchart | Kind::Tree | Kind::Simple => {
            let d = style.dir.unwrap_or(Dir::Down).vector();
            let (ba, bb) = (extent(&ca[0]), extent(&cb[0]));
            if style.kind == Kind::Flowchart {
                let (_, a_far) = span(&ba, d);
                let (b_near, _) = span(&bb, d);
                if b_near - a_far < 2.0 * opts.clearance {
                    return routed(route::best_through(ca, cb, boxes, pass, opts).2, true);
                }
            }
            let ia = sides(ca, |v| same_dir(v, d));
            let ib = sides(cb, |v| same_dir(v, neg(d)));
            if style.kind == Kind::Simple {
                return routed(among(ca, cb, &ia, &ib, &[], &[], opts).2, false);
            }
            let (_, _, way) = among(ca, cb, &ia, &ib, boxes, pass, opts);
            let way = if style.kind == Kind::Tree { bend_early(way, d, &ba, boxes, pass, opts.clearance) } else { way };
            routed(way, false)
        }
        Kind::RightAngle | Kind::Network => routed(route::best_through(ca, cb, boxes, pass, opts).2, true),
    }
}

#[cfg(test)]
#[path = "route_styles_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "route_styles_visio_tests.rs"]
mod visio_tests;
