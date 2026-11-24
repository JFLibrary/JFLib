// route_styles -- (JD21, CN-05, SS-40, SS-19) the routing styles of a page and of a connector with
// their directions, and the shapes connectors may cross
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
//   plan(style, &a_ports, &b_ports, ..) (crate) the way of a style with a direction: the sides
//                                       the ends may use, the obstacles, where the bends go;
//                                       None: the plain right-angle search (glue::route_one)
//
// Visio's rules -- belegt (Microsoft Learn: the cells RouteStyle, ShapeRouteStyle, ConFixedCode,
// ShapePermeableX/Y/Place, DynamicsOff, CtrlAsInput, LineAdjustFrom/To, LineRouteExt; the archived
// Visio 2002 developer pages "Creating Routable and Other 1-D Connectors" and "Designing Shapes for
// Automatic Layout"; the Visio blog "Improved flowchart routing in Visio 2010"; the Visio 2000
// user's guide, chapter 6):
// * RouteStyle / ShapeRouteStyle: 0 default (right angle), 1 right angle, 2 straight,
//   3/4/10/11 organization chart top to bottom / left to right / bottom to top / right to left,
//   5/6/12/13 flowchart, 7/8/14/15 tree (the same four directions), 9 network, 16 centre to
//   centre, 17/18/19/20 simple (top to bottom, left to right, bottom to top, right to left),
//   21 simple horizontal-vertical, 22 simple vertical-horizontal.  ShapeRouteStyle 0 takes the
//   page's; "setting a connector's ShapeRouteStyle cell to 7 creates a routable connector that
//   always routes as if it's contained in a tree diagram in top-to-bottom orientation".
// * Flowchart top to bottom: connectors generally leave the bottom of the shape they come from and
//   reach the top of the one they go to; since Visio 2010 other sides where the two shapes are
//   close along the direction's axis; static glue keeps its points, only dynamic ends move.
// * ShapePermeableX / Y: TRUE lets connectors route horizontally / vertically through the
//   (placeable) shape.  ShapePermeablePlace: shapes may be placed on top of it by a layout
//   (layout.rs).  ObjType 4: routing behaves as if the shape did not exist (glue.rs).
// * DynamicsOff: TRUE turns off "placeable shapes move and connectors reroute around other shapes
//   and connectors".  CtrlAsInput: which shape is the parent of a control handle's glue -- not
//   routing, kept for Visio.  LineAdjustFrom (0 unrelated lines, 1 all, 2 none, 3 the routing
//   style's default) and LineAdjustTo (0 the routing style's default, 1 lines close to each
//   other, 2 none, 3 related lines): which connectors routed on top of each other are spaced
//   apart / lined up -- kept for Visio and shown in Seite einrichten, connectors here are not
//   spaced apart (open).  LineRouteExt / ConLineRouteExt: 0 default, 1 straight, 2 curved (the
//   appearance, glue.rs).
// * ConFixedCode: 0 reroute freely, 1 reroute as needed (manual reroute), 2 never reroute,
//   3 reroute on crossover, 4 .. 6 internal (route_edit.rs keeps a way by hand).  The local corpus
//   (two Visio drawings, 104 dynamic connectors): every page's RouteStyle and every
//   ShapeRouteStyle 0; ConFixedCode 0 (38, from the style), 3 (15), 4 (4), 5 (44), 6 (3), never 1
//   or 2; one style of each drawing has ShapePermeableX/Y/Place 1; so there are no stored ways of
//   the other styles to compare with (diagramiotest group route counts them).
//
// Angenommen (no source describes the ways; the user's check in Visio may correct it):
// * Straight (2), network (9) and centre to centre (16) draw one straight line, docked where the
//   line between the two shapes' middles leaves their outlines (as straight was before JD21).
// * A style with a direction names the side an end uses: the begin leaves its shape in the
//   direction (the bottom for top to bottom), the end comes into its shape against it (the top).
//   Organization chart and tree keep to these sides always (a way against the flow goes round);
//   flowchart only where the end's shape lies at least two clearances further along the
//   direction than the begin's, else the cheapest sides (Visio 2010's exception).  Static and
//   free ends keep their points whatever the style.
// * Organization chart bends halfway between the shapes (the bus in the middle); tree bends right
//   after the begin's shape (one clearance past it), so that the branches to all children share
//   the line under their parent; flowchart bends halfway.
// * Simple (17 .. 20) keeps to the sides like an organization chart but routes around nothing
//   but the two glued shapes; simple horizontal-vertical leaves the begin's shape on a left or
//   right side and comes into the end's from above or below (an L), vertical-horizontal the
//   other way round.
// * A 2-D shape's ShapeRouteStyle does not change the connectors glued to it (Visio lists the cell
//   for placeable shapes, its effect there is unknown).

use alloc::vec::Vec;

use crate::cell::Value;
use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::layout::Dir;
use crate::route::{self, Opts, Port, PASS_X, PASS_Y};
use crate::sheet::CellAddr;

/// a routing style without its direction
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// right angles around obstacles (also the page default)
    RightAngle,
    /// one straight line
    Straight,
    /// right angles, the bend halfway, the sides of the direction always
    OrgChart,
    /// right angles, the sides of the direction where the shapes lie along it
    Flowchart,
    /// right angles, the bend right after the begin's shape, the sides of the direction always
    Tree,
    /// one straight line
    Network,
    /// the sides of the direction, around nothing but the glued shapes
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

    /// whether the style draws one straight line
    pub fn straight(self) -> bool {
        matches!(self, Kind::Straight | Kind::Network | Kind::CentreToCentre)
    }
}

/// a routing style and its direction (Visio's RouteStyle / ShapeRouteStyle)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct RouteStyle {
    /// the style
    pub kind: Kind,
    /// the flow (directed kinds only)
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

// a port's box (its glued shape), else its point
fn extent(p: &Port) -> [f64; 4] {
    p.own.unwrap_or([p.at[0], p.at[1], p.at[0], p.at[1]])
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

// how far a box reaches along direction d: its nearest and farthest points
fn span(b: &[f64; 4], d: [f64; 2]) -> (f64, f64) {
    let c = [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]];
    let v: Vec<f64> = c.iter().map(|p| dot(*p, d)).collect();
    (v.iter().copied().fold(f64::INFINITY, f64::min), v.iter().copied().fold(f64::NEG_INFINITY, f64::max))
}

// the indices of the ports whose side faces `want` (all of them when none does: a static or a
// free end keeps its point)
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

fn same_dir(a: [f64; 2], b: [f64; 2]) -> bool {
    dot(a, b) > 0.9
}

// (tree) the middle piece of a Z between the two ends moves to one clearance past the begin's
// shape, where that stays clear of the obstacles
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
    let (_, far) = span(begin, d);
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

/// the way of a style with a direction or fixed sides (organization chart, flowchart, tree,
/// simple): the ports `ca`, `cb` of the two ends as glue offers them (glue::candidates), the
/// obstacles with how they may be crossed -> the indices of the ports used and the way (None:
/// the plain right-angle search decides)
pub(crate) fn plan(style: RouteStyle, ca: &[Port], cb: &[Port], boxes: &[[f64; 4]], pass: &[u8], opts: &Opts) -> Option<(usize, usize, Vec<[f64; 2]>)> {
    if ca.is_empty() || cb.is_empty() {
        return None;
    }
    let horizontal = |v: [f64; 2]| v[0] != 0.0 && v[1] == 0.0;
    let vertical = |v: [f64; 2]| v[0] == 0.0 && v[1] != 0.0;
    match style.kind {
        Kind::SimpleHV | Kind::SimpleVH => {
            let hv = style.kind == Kind::SimpleHV;
            let ia = sides(ca, |v| if hv { horizontal(v) } else { vertical(v) });
            let ib = sides(cb, |v| if hv { vertical(v) } else { horizontal(v) });
            Some(among(ca, cb, &ia, &ib, &[], &[], opts))
        }
        Kind::OrgChart | Kind::Flowchart | Kind::Tree | Kind::Simple => {
            let d = style.dir.unwrap_or(Dir::Down).vector();
            let back = [-d[0], -d[1]];
            let (ba, bb) = (extent(&ca[0]), extent(&cb[0]));
            if style.kind == Kind::Flowchart {
                // only where the end's shape lies further along the flow, with room for a bend
                let (_, a_far) = span(&ba, d);
                let (b_near, _) = span(&bb, d);
                if b_near - a_far < 2.0 * opts.clearance {
                    return None;
                }
            }
            let ia = sides(ca, |v| same_dir(v, d));
            let ib = sides(cb, |v| same_dir(v, back));
            if style.kind == Kind::Simple {
                return Some(among(ca, cb, &ia, &ib, &[], &[], opts));
            }
            let (i, j, way) = among(ca, cb, &ia, &ib, boxes, pass, opts);
            let way = if style.kind == Kind::Tree { bend_early(way, d, &ba, boxes, pass, opts.clearance) } else { way };
            Some((i, j, way))
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "route_styles_tests.rs"]
pub(crate) mod tests;
