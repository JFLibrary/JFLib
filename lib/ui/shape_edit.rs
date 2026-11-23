// shape_edit -- editing shapes on a ui.diagram canvas (DG_EDIT): selection by click, marquee,
// keyboard, Shift and Ctrl; the eight handles and the rotation handle of turned and flipped
// frames, the rotation pin, yellow control points, vertices, the ends of 1D shapes with glue to
// connection points (static) or to whole shapes (dynamic); moving with the grid, the dynamic grid
// (alignment to other shapes' edges and middles) and guides; the connector and drawing tools; the
// magnifier (Ctrl + Shift + click); one semantic record per shape and per handle.  The app owns
// the model and applies what the user did, reported as SeEv events; the widget keeps the
// selection and the gesture in SeState.  It knows nothing of diagram files: an app maps its
// shapes to SeShape frames (diagram_core for JFDiagrammer).
//
//   ui.diagram(id, &mut dg, &m, DG_EDIT, paint, ver)     the view: the paint, zoom, pan
//   ui.shape_edit(id, &mut st, &mut dg, &model, tool) -> Vec<SeEv>
//                                   right after it with the same id and DiagramState: the left
//                                   button, the keys while the canvas has the focus, the cursor,
//                                   the overlays (selection, handles, glue, guides, rubber bands)
//   ui.shape_edit_sems(id, &mut st, &dg, &model, &label)   one K_LISTITEM record per visible
//                                   top-level shape (label(key)) and one K_BUTTON record per
//                                   handle of a single selection ("Größengriff unten rechts",
//                                   "Drehgriff", "Anfangspunkt", "Steuerpunkt 1" ...), per
//                                   connection arrow while they show ("Verbindungspfeil rechts",
//                                   "... unten", "... links", "... oben") and per insertion mark
//                                   while it shows ("Element einfügen", value = its tag): scripts
//                                   click and drag them, screen readers name them (a screen
//                                   reader's click on an arrow or a mark reports it as a click)
//   tools SE_POINTER, SE_CONNECT, SE_LINE, SE_RECT, SE_ELLIPSE, SE_TEXT, SE_PLACE, SE_FREEFORM,
//         SE_ARC
//
// Model (SeModel, borrowed per call; world units are the canvas's, y down):
//   SeShape { key, parent, bx, ends, pin, bbox, flags, points }
//   SeBox { o, u, v }   the frame: the point o + fx u + fy v for fx, fy in 0..1; u runs along the
//                       shape's width, v along its height (downwards on the screen for a shape
//                       that is neither turned nor flipped; a flip reverses u or v); handles at
//                       fx, fy in {0, 0.5, 1}, the rotation handle beyond the edge fy = 0
//   SePoint { at, kind, tag, dir, flags }   SP_CONTROL (yellow), SP_CONN (connection points:
//                       shown while connecting, with SeState::show_points those of every
//                       visible shape, RA-06; SPF_OUTWARD: shown, no target), SP_VERTEX (shown
//                       for a single selection when the app lists them: vertex editing)
//   SeGuide { key, vertical, at }   guides (blue, dashed, across the canvas); moves snap to them
//                       (SeState::snap and snap_guides; an app hides guides by leaving them out)
//   SeMark { at, dir, tag, owner }  the app's insertion marks (Visio's blue triangle between two
//                       items of a list): a small triangle, its tip at `at`, pointing in `dir`
//   hit(p, tol, within) -> key      the topmost shape at world point p within tol world units
//                       (within = 0: a top-level shape; a group's key: the child of that group
//                       under p, for the second click into a group), 0 = none; the app asks its
//                       display list
//
// Live gestures: Move, Resize, Rotate, Pin, End and Point report the whole change from the
// start of the drag with every step (done = false) and once more when released (done = true);
// Esc during one sends SeEv::Cancel.  So the app takes its preview back and applies the new
// total each step, and keeps the last one as the undo step.  The frames of the selection follow
// the model (the app's preview), the gesture keeps what it had at the start.  The connector and
// line tools and the drawing tools draw a rubber band and report Connect / Draw when released;
// a click with them selects like the pointer.  With Shift the rectangle and the ellipse tool draw
// a square and a circle, and a move with Shift keeps to one axis (KB-09); a drag of the selection
// that starts with Ctrl + Shift moves a copy of it (SeEv::MoveCopy, live like Move: the app
// shows a preview each step and makes the copy when released; one axis while Shift is held,
// KB-09).  The freeform tool
// records the pointer's way (a point every 2 px) and reports SeEv::Stroke when released; the arc
// tool shows a quarter ellipse from the press to the pointer, bowing the way the drag started
// (horizontally or vertically, decided once the pointer left the drag threshold), and reports
// SeEv::Arc; both start and end glued to connection points like the line tool (CN-12).
//
// Input (pointer tool): a press on a handle of the selection starts its drag; on a shape it
// selects it (Shift adds, Ctrl toggles) and a drag moves the selection (grid, dynamic grid,
// guides; Alt: free); a second, slow click on a selected group selects the child under the
// pointer; a double click edits the text (SeEv::Text); on the background a drag draws a marquee
// or, with SeState::lasso, a free outline (shapes fully inside, with SeState::partial also those
// it touches, ZF-06; Shift adds, Ctrl toggles).  Corner handles keep the aspect ratio, edge
// handles change one side (a locked aspect: both), Ctrl resizes about the middle; locked handles
// are drawn grey and do nothing.  The rotation handle turns about the pin (one shape) or the
// middle of the selection: in 15-degree steps near the shape, in single degrees farther away.
// An end or a control point that can glue snaps to a connection point under the pointer (static
// glue); over a shape a dynamic connector's end glues to the whole shape (dynamic glue, the
// shape's frame is highlighted); with Ctrl held a dynamic end swaps the two (CN-03): over a
// connection point it glues to that point's shape as a whole, over a shape to the shape's
// nearest connection point.  Connection points with SPF_OUTWARD are no target for ends (SS-29).
// Ctrl + Shift + click zooms in at the pointer (on a selected shape too), + drag zooms to the
// rectangle (from a selected shape: the copy above), + right click zooms out (ZF-22), + right
// drag pans the view (KB-04).  Keys while the canvas has the focus: Esc
// cancels the gesture, else clears the selection; Del deletes; Ctrl+A selects all; arrows nudge
// by a grid step (Shift: a pixel) or pan without a selection; Tab / Shift+Tab walk the top-level
// shapes (without a selection Tab takes the first, Shift+Tab the last, KB-06); F2 and Enter edit
// the text of the single selection, so does typing a character (the
// character is handed on).  A right click selects the shape under the pointer first and reports
// SeEv::Context; the menu itself (also from Shift+F10 and the menu key) is the app's
// ui.context_menu right after this call.
//
// Connection arrows (SeState::arrows, on by default; Ansicht › Verbindungspfeile): with the
// pointer tool and no gesture, a pointer on a top-level shape shows four small filled triangles
// in the accent colour around its box (SeShape::bbox), in the middle of each side, ARROW_GAP
// (12) DIP out from the box and ARROW_LEN (10) DIP long; half transparent, the one under the
// pointer opaque.  Directions as seen: 0 right, 1 down, 2 left, 3 up.  They stay while the
// pointer is on the shape, on one of its handles, on an arrow or in the strip between the box
// and an arrow (as wide as the arrow), and go at once elsewhere (on another shape: that
// shape's arrows).  None for 1D shapes, SEF_NOSELECT, SEF_NOARROWS (the app's choice: shapes
// whose file refuses them, ...), children of groups, shapes smaller than ARROW_MIN (16) DIP on
// the screen, nor right after a gesture or a click until the pointer moves again.  An arrow
// that would cover a rotation handle of the selection (the single shape's, or the one of the
// box around several) moves out along its direction until it clears the handle by a few DIP
// (Visio puts its top arrow beyond the rotation handle too); the strip that keeps the arrows
// grows with it.  Handles and control points of that shape (or of a selection box around it)
// stay on top and are hit first; another selected shape's handle under a hot arrow is not.  A click on an
// arrow reports SeEv::Arrow { key, dir } and leaves the selection alone (the app selects what
// it made); a drag from an arrow is the connector tool from that shape: a rubber band from
// the middle of the box's side, glue and the target's highlight as SE_CONNECT, released
// SeEv::Connect { tool: SE_CONNECT, a: that middle, ga: SeGlue::Shape(key), b, gb }; Esc
// cancels.  The arrow under the pointer is SeState::arrow_hover(), and each change of it is
// SeEv::ArrowHover { key, dir } once (dir 255: off the arrows of `key`), for a preview.
//
// Insertion marks (SeModel::marks): with the pointer tool and no gesture each mark shows while
// the pointer is on its owner (the shape under the pointer, or inside the owner's box) or
// within MARK_NEAR (16) DIP of it; a click reports SeEv::Mark { tag }, a drag does nothing.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;

use crate::cmd::{Draw, Gfx};
use crate::consts::*;
use crate::diagram::{dist, fl, fx, inside, ipx, node_sem_id, overlaps, snapv, DiagramState, ExtIn};
use crate::keys::*;
use crate::Ui;
use rt::fx::sqrt;

/// tools of ui.shape_edit
pub const SE_POINTER: u32 = 0;
/// draw a dynamic connector: its ends glue to connection points or whole shapes
pub const SE_CONNECT: u32 = 1;
/// draw a line: its ends glue to connection points only (CN-12); vertex editing when the app
/// lists vertices
pub const SE_LINE: u32 = 2;
pub const SE_RECT: u32 = 3;
pub const SE_ELLIPSE: u32 = 4;
/// a text box: drag its frame, or click for one of the app's default size
pub const SE_TEXT: u32 = 5;
/// place what the app holds (a shape from a stencil, click + click): a click anywhere, on a
/// shape too, reports Draw { tool: SE_PLACE, a, b: a } at the snapped point; no handles, a
/// drag places nothing
pub const SE_PLACE: u32 = 6;
/// draw freehand: the pointer's way, reported as SeEv::Stroke (its ends glue like a line's)
pub const SE_FREEFORM: u32 = 7;
/// draw a quarter ellipse from the press to the release, reported as SeEv::Arc (its ends glue
/// like a line's)
pub const SE_ARC: u32 = 8;

/// shape flags (SeShape::flags)
pub const SEF_1D: u32 = 1; // the two ends instead of the eight handles
pub const SEF_GROUP: u32 = 2; // a group: a second click selects the child under the pointer
pub const SEF_DYNAMIC: u32 = 4; // a dynamic connector: its ends also glue to whole shapes
pub const SEF_LOCK_W: u32 = 8;
pub const SEF_LOCK_H: u32 = 0x10;
pub const SEF_LOCK_ASPECT: u32 = 0x20;
pub const SEF_LOCK_ROTATE: u32 = 0x40;
pub const SEF_LOCK_BEGIN: u32 = 0x80;
pub const SEF_LOCK_END: u32 = 0x100;
pub const SEF_LOCK_MOVE: u32 = 0x200; // LockMoveX and LockMoveY: no move by dragging
pub const SEF_NOSELECT: u32 = 0x400; // never selected (LockSelect)
pub const SEF_NOGLUE: u32 = 0x800; // nothing glues to it
pub const SEF_BEGIN_GLUED: u32 = 0x1000; // the begin is glued to a point (static)
pub const SEF_BEGIN_DYN: u32 = 0x2000; // the begin is glued to a whole shape (dynamic)
pub const SEF_END_GLUED: u32 = 0x4000;
pub const SEF_END_DYN: u32 = 0x8000;
pub const SEF_LOCK_PIN: u32 = 0x1_0000; // no rotation pin handle
pub const SEF_NOARROWS: u32 = 0x2_0000; // no connection arrows around it (see the file header)
pub const SEF_NOSNAP: u32 = 0x4_0000; // moves do not snap to it (the dynamic grid leaves it out)

/// kinds of SePoint
pub const SP_CONTROL: u8 = 0;
pub const SP_CONN: u8 = 1;
pub const SP_VERTEX: u8 = 2;
/// point flags (SePoint::flags)
pub const SPF_GLUE: u32 = 1; // a control point that glues to connection points (SP-04)
/// a connection point that points outward only (Visio's Type 1): shown, but no end glues to it
/// (SS-29)
pub const SPF_OUTWARD: u32 = 2;

/// a frame in world units: the point o + fx u + fy v for fx, fy in 0..1
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeBox {
    pub o: [f32; 2],
    pub u: [f32; 2],
    pub v: [f32; 2],
}

impl SeBox {
    /// an axis-parallel frame x0, y0, x1, y1
    pub fn rect(r: [f32; 4]) -> SeBox {
        SeBox { o: [r[0], r[1]], u: [r[2] - r[0], 0.0], v: [0.0, r[3] - r[1]] }
    }

    /// the point at fractions fx, fy
    pub fn at(&self, fx: f32, fy: f32) -> [f32; 2] {
        [self.o[0] + fx * self.u[0] + fy * self.v[0], self.o[1] + fx * self.u[1] + fy * self.v[1]]
    }

    /// the fractions of a world point (None: a degenerate frame)
    pub fn frac(&self, p: [f32; 2]) -> Option<[f32; 2]> {
        let det = self.u[0] * self.v[1] - self.u[1] * self.v[0];
        if !(det.abs() > 1e-12) {
            return None;
        }
        let (dx, dy) = (p[0] - self.o[0], p[1] - self.o[1]);
        Some([(dx * self.v[1] - dy * self.v[0]) / det, (self.u[0] * dy - self.u[1] * dx) / det])
    }

    /// the axis-parallel box x0, y0, x1, y1 around it
    pub fn aabb(&self) -> [f32; 4] {
        let c = [self.at(0.0, 0.0), self.at(1.0, 0.0), self.at(1.0, 1.0), self.at(0.0, 1.0)];
        let mut b = [c[0][0], c[0][1], c[0][0], c[0][1]];
        for q in &c[1..] {
            b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
        }
        b
    }

    /// the length of u and v
    pub fn size(&self) -> [f32; 2] {
        [len(self.u), len(self.v)]
    }

    fn axis_parallel(&self) -> bool {
        self.u[1].abs() <= 1e-4 * len(self.u).max(1e-6) && self.v[0].abs() <= 1e-4 * len(self.v).max(1e-6)
    }
}

/// a shape as the editor sees it
#[derive(Clone, Debug, Default)]
pub struct SeShape {
    pub key: u64,
    /// the group it belongs to (0: on the page; such a shape is what a click selects)
    pub parent: u64,
    /// its frame (2D shapes)
    pub bx: SeBox,
    /// begin and end (SEF_1D)
    pub ends: [[f32; 2]; 2],
    /// the rotation pin
    pub pin: [f32; 2],
    /// the box x0, y0, x1, y1 around what is drawn (marquee, alignment)
    pub bbox: [f32; 4],
    pub flags: u32,
    /// its control points, connection points and vertices (a range of SeModel::points)
    pub points: Range<u32>,
}

/// a point of a shape: a control point, a connection point or a vertex
#[derive(Clone, Copy, Debug, Default)]
pub struct SePoint {
    pub at: [f32; 2],
    pub kind: u8,
    /// the app's name for it (a row)
    pub tag: u32,
    /// a connection point's direction (world, unit; 0 0 = none)
    pub dir: [f32; 2],
    pub flags: u32,
}

/// a guide: a vertical (x = at) or horizontal (y = at) line
#[derive(Clone, Copy, Debug, Default)]
pub struct SeGuide {
    pub key: u64,
    pub vertical: bool,
    pub at: f32,
}

/// an insertion mark of the app (Visio's blue triangle between two items of a list): a small
/// triangle with its tip at `at` (world) pointing in `dir` (0 right, 1 down, 2 left, 3 up, as
/// seen); it shows with the pointer tool while the pointer is on the shape `owner` or near the
/// mark, and a click on it reports SeEv::Mark { tag }
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeMark {
    pub at: [f32; 2],
    pub dir: u8,
    pub tag: u32,
    pub owner: u64,
}

/// the app's shapes, borrowed for one call
#[derive(Clone, Copy)]
pub struct SeModel<'a> {
    pub shapes: &'a [SeShape],
    pub points: &'a [SePoint],
    pub guides: &'a [SeGuide],
    /// insertion marks (lists: between two items)
    pub marks: &'a [SeMark],
    /// changes whenever shapes are added, removed or reordered
    pub ver: u32,
    /// the topmost shape at a world point (see the file header)
    pub hit: &'a dyn Fn([f32; 2], f32, u64) -> u64,
}

/// what an end or a control point is glued to
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum SeGlue {
    #[default]
    None,
    /// a connection point (its tag) of a shape: static glue
    Point { key: u64, tag: u32 },
    /// a whole shape: dynamic glue
    Shape(u64),
}

/// what the user did (ui.shape_edit's result, in order)
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SeEv {
    /// st.sel changed
    Select,
    /// move `keys` by total (world units, from the start of the drag; a key nudge: done at once)
    Move { keys: Vec<u64>, total: [f32; 2], done: bool },
    /// a drag with Ctrl + Shift: a copy of `keys` goes `total` from them (live like Move: a
    /// preview each step, done: the copy, one undo step; KB-09)
    MoveCopy { keys: Vec<u64>, total: [f32; 2], done: bool },
    /// resize: the frame `from` (one shape: its frame; several: the box around them) became `to`
    Resize { keys: Vec<u64>, from: SeBox, to: SeBox, done: bool },
    /// turn `keys` about `centre` by `angle` radians (counter-clockwise as seen)
    Rotate { keys: Vec<u64>, centre: [f32; 2], angle: f32, done: bool },
    /// the rotation pin of a shape goes to `at`
    Pin { key: u64, at: [f32; 2], done: bool },
    /// end 0 (begin) or 1 of a 1D shape goes to `at`, glued to `glue`
    End { key: u64, end: u8, at: [f32; 2], glue: SeGlue, done: bool },
    /// a control point or a vertex (index `idx` of SeModel::points, the app's tag) goes to `at`
    Point { key: u64, idx: u32, tag: u32, at: [f32; 2], glue: SeGlue, done: bool },
    /// Ctrl + click on the outline of a shape with vertices listed: add a vertex at `at`
    PointInsert { key: u64, at: [f32; 2] },
    /// Del with a vertex chosen: remove it
    PointDelete { key: u64, tag: u32 },
    /// Esc during a live gesture: take the preview back
    Cancel,
    /// the connector or line tool drew from a to b
    Connect { tool: u32, a: [f32; 2], ga: SeGlue, b: [f32; 2], gb: SeGlue },
    /// a drawing tool drew the box from a to b (a == b: a click)
    Draw { tool: u32, a: [f32; 2], b: [f32; 2] },
    /// the freeform tool drew this way (world points, at least two); its begin glued to `ga`,
    /// its end to `gb` (connection points only)
    Stroke { pts: Vec<[f32; 2]>, ga: SeGlue, gb: SeGlue },
    /// the arc tool drew a quarter ellipse from a to b that leaves a horizontally
    /// (horizontal) or vertically; glued like a line's ends
    Arc { a: [f32; 2], ga: SeGlue, b: [f32; 2], gb: SeGlue, horizontal: bool },
    /// edit the text of a shape (double click, F2, Enter, or a typed character)
    Text { key: u64, ch: Option<char> },
    /// right click, Shift+F10 or the menu key on a shape (0: the page); at = window px
    Context { key: u64, at: [i32; 2] },
    /// Del with a selection
    Delete,
    /// a click on the connection arrow `dir` (0 right, 1 down, 2 left, 3 up, as seen) of the
    /// top-level shape `key`; the selection stays as it was
    Arrow { key: u64, dir: u8 },
    /// the pointer went onto the connection arrow `dir` of shape `key`, or off the arrows of
    /// `key` (dir 255); once per change
    ArrowHover { key: u64, dir: u8 },
    /// a click on the insertion mark with this tag (SeModel::marks)
    Mark { tag: u32 },
}

// what a point of the canvas shows
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum SHit {
    #[default]
    None,
    Shape(u64),
    /// handle 0 .. 7 of a single shape (HANDLES order)
    Handle(u64, u8),
    /// handle 0 .. 7 of the box around a multiple selection
    SelHandle(u8),
    Rotate(u64),
    SelRotate,
    Pin(u64),
    End(u64, u8),
    /// a control point or vertex: index into SeModel::points
    Point(u64, u32),
    /// a connection arrow of a shape: its direction
    Arrow(u64, u8),
    /// an insertion mark: index into SeModel::marks, its tag
    Mark(u32, u32),
}

// handle positions: top left, top, top right, right, bottom right, bottom, bottom left, left
const HANDLES: [[f32; 2]; 8] = [[0.0, 0.0], [0.5, 0.0], [1.0, 0.0], [1.0, 0.5], [1.0, 1.0], [0.5, 1.0], [0.0, 1.0], [0.0, 0.5]];
// i18n (library texts, i18n.rs LIB_TEXTS)
const HANDLE_NAMES: [&str; 8] = [
    "Größengriff oben links", "Größengriff oben", "Größengriff oben rechts", "Größengriff rechts",
    "Größengriff unten rechts", "Größengriff unten", "Größengriff unten links", "Größengriff links",
];
const ROT_DIP: i32 = 22; // the rotation handle's distance from the frame
const HANDLE_DIP: i32 = 4; // half the side of a handle
const GLUE_DIP: i32 = 8; // the reach of a connection point
const ALIGN_DIP: i32 = 6; // the reach of the dynamic grid and of guides
// i18n (library texts, i18n.rs LIB_TEXTS): the connection arrows by direction
const ARROW_NAMES: [&str; 4] = ["Verbindungspfeil rechts", "Verbindungspfeil unten", "Verbindungspfeil links", "Verbindungspfeil oben"];
const MARK_NAME: &str = "Element einfügen"; // an insertion mark
const ARROW_GAP: i32 = 12; // from a shape's box to the base of a connection arrow
const ARROW_LEN: i32 = 10; // a connection arrow from its base to its tip
const ARROW_HALF: i32 = 6; // half the base of a connection arrow
const ARROW_MIN: i32 = 16; // no arrows around a shape smaller than this on the screen
const MARK_LEN: i32 = 9; // an insertion mark from its base to its tip
const MARK_HALF: i32 = 5; // half its base
const MARK_NEAR: i32 = 16; // a mark shows while the pointer is this near to it
// the directions 0 right, 1 down, 2 left, 3 up (y down)
const DIRS: [[f32; 2]; 4] = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]];

// the connection arrows around a shape (canvas px, the content's LTR coordinates): its box
// x0, y0, x1, y1 and per direction the distance from the box's side to the arrow's base
#[derive(Clone, Copy)]
struct ArrowSet {
    b: [f32; 4],
    base: [f32; 4],
    len: f32,
    half: f32,
}

impl ArrowSet {
    // the middle of the box's side in direction d
    fn edge(&self, d: usize) -> [f32; 2] {
        let b = self.b;
        let (cx, cy) = ((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5);
        match d & 3 {
            0 => [b[2], cy],
            1 => [cx, b[3]],
            2 => [b[0], cy],
            _ => [cx, b[1]],
        }
    }

    // p seen from the side d: outwards from it, across it
    fn local(&self, d: usize, p: [f32; 2]) -> (f32, f32) {
        let e = self.edge(d);
        let v = DIRS[d & 3];
        let (dx, dy) = (p[0] - e[0], p[1] - e[1]);
        (dx * v[0] + dy * v[1], dy * v[0] - dx * v[1])
    }

    fn tri(&self, d: usize) -> [[f32; 2]; 3] {
        let e = self.edge(d);
        let v = DIRS[d & 3];
        let out = self.base[d & 3] + self.len;
        tri_at([e[0] + v[0] * out, e[1] + v[1] * out], d, self.len, self.half)
    }

    // the arrow at p (tol px around it)
    fn hit(&self, p: [f32; 2], tol: f32) -> Option<u8> {
        (0..4).find(|&d| {
            let (t, l) = self.local(d, p);
            t >= self.base[d] - tol && t <= self.base[d] + self.len + tol && l.abs() <= self.half + tol
        })
        .map(|d| d as u8)
    }

    // p on the box, on an arrow or in the strip between them
    fn zone(&self, p: [f32; 2], tol: f32) -> bool {
        let b = self.b;
        if p[0] >= b[0] - tol && p[0] <= b[2] + tol && p[1] >= b[1] - tol && p[1] <= b[3] + tol {
            return true;
        }
        (0..4).any(|d| {
            let (t, l) = self.local(d, p);
            t >= -tol && t <= self.base[d] + self.len + tol && l.abs() <= self.half + tol
        })
    }
}

// a triangle with its tip at `tip` pointing in direction d: len long, its base 2 half wide
fn tri_at(tip: [f32; 2], d: usize, len: f32, half: f32) -> [[f32; 2]; 3] {
    let v = DIRS[d & 3];
    let n = [-v[1], v[0]];
    let b = [tip[0] - v[0] * len, tip[1] - v[1] * len];
    [tip, [b[0] + n[0] * half, b[1] + n[1] * half], [b[0] - n[0] * half, b[1] - n[1] * half]]
}

// the px box x, y, w, h around a triangle (a pixel more on each side)
fn tri_box(t: &[[f32; 2]; 3]) -> [i32; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (t[0][0], t[0][1], t[0][0], t[0][1]);
    for p in &t[1..] {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    let (a, b) = (fl(x0) - 1, fl(y0) - 1);
    [a, b, (fl(x1) + 2 - a).max(1), (fl(y1) + 2 - b).max(1)]
}

// a filled triangle, anti-aliased (a custom draw: the path fills it); its points relative to
// the command's rectangle, which a mirrored window moves
struct Tri {
    pts: [[f32; 2]; 3],
    c: u32,
}

impl Draw for Tri {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let path = &mut g.text.path;
        path.reset();
        path.set_origin(0, 0);
        for (k, p) in self.pts.iter().enumerate() {
            let (x, y) = (r[0] as f32 + p[0], r[1] as f32 + p[1]);
            if k == 0 {
                path.move_f(x, y);
            } else {
                path.line_f(x, y);
            }
        }
        path.close();
        g.dc.set_color(self.c);
        path.fill(&mut g.text.ras, g.dc, g.sf, gfx::RULE_NONZERO);
    }
}

#[derive(Clone, Default)]
enum G {
    #[default]
    Idle,
    /// a left press below the drag threshold: what it hit, where, the modifiers, whether the
    /// shape was selected before
    Press { hit: SHit, px: [i32; 2], at: [f32; 2], mods: u32, was: bool },
    Hold,
    /// copy: a copy moves (Ctrl + Shift at the press, KB-09)
    Move { keys: Vec<u64>, start: [f32; 2], bb: [f32; 4], total: [f32; 2], copy: bool },
    Resize { keys: Vec<u64>, h: u8, from: SeBox, to: SeBox, aspect: bool },
    Rotate { keys: Vec<u64>, centre: [f32; 2], last: f32, turn: f32, angle: f32 },
    Pin { key: u64, at: [f32; 2] },
    End { key: u64, end: u8, at: [f32; 2], glue: SeGlue, dynamic: bool },
    Point { key: u64, idx: u32, tag: u32, at: [f32; 2], glue: SeGlue, glues: bool },
    Marquee { start: [f32; 2], cur: [f32; 2], mods: u32, base: BTreeSet<u64> },
    /// the lasso: the pointer's way so far
    Lasso { pts: Vec<[f32; 2]>, mods: u32, base: BTreeSet<u64> },
    Zoom { start: [f32; 2], cur: [f32; 2] },
    Connect { tool: u32, a: [f32; 2], ga: SeGlue, b: [f32; 2] },
    Draw { tool: u32, a: [f32; 2], b: [f32; 2] },
    /// the freeform tool: the way so far, the begin's glue
    Stroke { pts: Vec<[f32; 2]>, ga: SeGlue },
    /// the arc tool: from a to b, the direction it leaves a (None: not decided yet)
    Arc { a: [f32; 2], ga: SeGlue, b: [f32; 2], h: Option<bool> },
}

// a pan with the right button (Ctrl + Shift held at the press)
#[derive(Clone, Copy)]
struct RPan {
    px: [i32; 2],
    cx: f64,
    cy: f64,
    moved: bool,
}

// an alignment line of the dynamic grid: from a to b (world)
#[derive(Clone, Copy)]
struct Align {
    a: [f32; 2],
    b: [f32; 2],
}

/// the editor's state, kept by the app: the selection, the focused shape, the options, the
/// gesture in progress
pub struct SeState {
    /// the selected shapes (keys)
    pub sel: BTreeSet<u64>,
    /// the anchor of the selection (the first one selected; drawn thicker, ZF-06)
    pub primary: u64,
    /// the shape with the keyboard focus (Tab walks)
    pub focus: u64,
    /// the dynamic grid: moves snap to other shapes' edges and middles (ZF-05)
    pub dyn_grid: bool,
    /// ends glue (Ansicht › Ausrichten und Kleben)
    pub glue: bool,
    /// show the connection points of every visible shape, not only while connecting (CN-01,
    /// RA-06; also while glue is off)
    pub show_points: bool,
    /// a drag on the background draws a lasso (a free outline) instead of a rectangle (ZF-06)
    pub lasso: bool,
    /// the marquee and the lasso also take the shapes they only touch (ZF-06; off: only those
    /// wholly inside)
    pub partial: bool,
    /// moves snap to guides and to the dynamic grid at all (off: a page that inhibits snapping,
    /// SS-39; the grid is DiagramState::snap)
    pub snap: bool,
    /// moves snap to guides (Ausrichten und Kleben › Führungslinien)
    pub snap_guides: bool,
    /// the vertex chosen with a click (Del removes it): its shape and tag
    pub vertex: Option<(u64, u32)>,
    /// connection arrows around the shape under the pointer (Ansicht › Verbindungspfeile; see
    /// the file header)
    pub arrows: bool,
    // the shape whose connection arrows show (0: none), the arrow under the pointer (255: none)
    // and what SeEv::ArrowHover said last (key, dir)
    arrow_key: u64,
    arrow_hot: u8,
    arrow_said: (u64, u8),
    // the pointer (world) while insertion marks may show, the mark under it (index, tag)
    mark_p: Option<[f32; 2]>,
    mark_hot: Option<(u32, u32)>,
    // where the last gesture or click ended: no arrows or marks until the pointer leaves it
    quiet: Option<[i32; 2]>,
    // clicks of a screen reader on arrows and marks (shape_edit_sems), reported next call
    pend: Vec<SeEv>,
    // the shape whose rotation pin shows (after the pointer rested on its rotation handle, as
    // in Visio: a press in a shape's middle moves the shape, not its pin)
    pin_shown: u64,
    // Ctrl + Shift + right button held: where it went down, the view's middle then, whether it
    // left the drag threshold (a pan, KB-04)
    rpan: Option<RPan>,
    // this call's right release was the magnifier's or a pan's (no context menu)
    rtaken: bool,
    g: G,
    hover: SHit,
    // what the glue would be now (drawing)
    glue_now: SeGlue,
    aligns: Vec<Align>,
    // the shape under the pointer for the connection points
    hover_shape: u64,
    last_hit: Option<SHit>,
    last_ms: u32,
    last_px: [i32; 2],
    pending_sel: bool,
    ver: u32,
    ix: BTreeMap<u64, usize>,
}

impl Default for SeState {
    fn default() -> SeState {
        SeState::new()
    }
}

impl SeState {
    /// nothing selected; dynamic grid, glue, snapping and connection arrows on
    pub fn new() -> SeState {
        SeState {
            sel: BTreeSet::new(),
            primary: 0,
            focus: 0,
            dyn_grid: true,
            glue: true,
            show_points: false,
            lasso: false,
            partial: false,
            snap: true,
            snap_guides: true,
            vertex: None,
            arrows: true,
            arrow_key: 0,
            arrow_hot: 255,
            arrow_said: (0, 255),
            mark_p: None,
            mark_hot: None,
            quiet: None,
            pend: Vec::new(),
            pin_shown: 0,
            rpan: None,
            rtaken: false,
            g: G::Idle,
            hover: SHit::None,
            glue_now: SeGlue::None,
            aligns: Vec::new(),
            hover_shape: 0,
            last_hit: None,
            last_ms: 0,
            last_px: [0; 2],
            pending_sel: false,
            ver: u32::MAX,
            ix: BTreeMap::new(),
        }
    }

    /// a gesture is in progress
    pub fn busy(&self) -> bool {
        !matches!(self.g, G::Idle | G::Hold)
    }

    /// a live gesture is in progress (the app shows a preview of it)
    pub fn live(&self) -> bool {
        matches!(self.g, G::Move { .. } | G::Resize { .. } | G::Rotate { .. } | G::Pin { .. } | G::End { .. } | G::Point { .. })
    }

    /// the right button's release in the last call of ui.shape_edit belonged to Ctrl + Shift
    /// (the magnifier zooming out, a pan): the app opens no context menu for it
    pub fn right_taken(&self) -> bool {
        self.rtaken
    }

    /// the connection arrow under the pointer: the shape's key and the direction (0 right,
    /// 1 down, 2 left, 3 up, as seen)
    pub fn arrow_hover(&self) -> Option<(u64, u8)> {
        (self.arrow_key != 0 && self.arrow_hot < 4).then_some((self.arrow_key, self.arrow_hot))
    }

    // no arrows, no marks
    fn extras_off(&mut self, out: &mut Vec<SeEv>) {
        self.arrow_key = 0;
        self.arrow_hot = 255;
        self.mark_p = None;
        self.mark_hot = None;
        self.say_arrow(out);
    }

    // SeEv::ArrowHover when the arrow under the pointer changed
    fn say_arrow(&mut self, out: &mut Vec<SeEv>) {
        let now = self.arrow_hover().unwrap_or((0, 255));
        if now != self.arrow_said {
            let key = if now.1 == 255 { self.arrow_said.0 } else { now.0 };
            out.push(SeEv::ArrowHover { key, dir: now.1 });
            self.arrow_said = now;
        }
    }

    /// select exactly these shapes (the app's commands: paste, group, a new shape)
    pub fn select(&mut self, keys: &[u64]) {
        self.sel = keys.iter().copied().collect();
        self.primary = keys.first().copied().unwrap_or(0);
        if let Some(&k) = keys.first() {
            self.focus = k;
        }
        self.vertex = None;
    }

    fn shape<'m>(&self, m: &SeModel<'m>, key: u64) -> Option<&'m SeShape> {
        self.ix.get(&key).and_then(|&i| m.shapes.get(i)).filter(|s| s.key == key)
    }

    fn update_index(&mut self, m: &SeModel) -> bool {
        if self.ver == m.ver && self.ix.len() <= m.shapes.len() {
            return false;
        }
        self.ver = m.ver;
        self.ix = m.shapes.iter().enumerate().map(|(i, s)| (s.key, i)).collect();
        let n = self.sel.len();
        let ix = &self.ix;
        self.sel.retain(|k| ix.contains_key(k));
        if !self.ix.contains_key(&self.primary) {
            self.primary = self.sel.iter().next().copied().unwrap_or(0);
        }
        if !self.ix.contains_key(&self.focus) {
            self.focus = 0;
        }
        if self.vertex.is_some_and(|(k, _)| !self.ix.contains_key(&k)) {
            self.vertex = None;
        }
        n != self.sel.len()
    }

    // the selected shapes that a drag moves (not locked)
    fn movable(&self, m: &SeModel) -> Vec<u64> {
        self.sel.iter().copied().filter(|&k| self.shape(m, k).is_some_and(|s| s.flags & SEF_LOCK_MOVE == 0)).collect()
    }

    // the box x0, y0, x1, y1 around the selected shapes
    fn sel_bbox(&self, m: &SeModel) -> Option<[f32; 4]> {
        let mut b: Option<[f32; 4]> = None;
        for &k in &self.sel {
            if let Some(s) = self.shape(m, k) {
                let r = s.bbox;
                b = Some(match b {
                    None => r,
                    Some(a) => [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])],
                });
            }
        }
        b
    }

    // the single selected shape
    fn single<'m>(&self, m: &SeModel<'m>) -> Option<&'m SeShape> {
        if self.sel.len() != 1 {
            return None;
        }
        self.sel.iter().next().and_then(|&k| self.shape(m, k))
    }

    // the frame whose handles a multiple selection shows (None: one shape or none)
    fn multi_box(&self, m: &SeModel) -> Option<SeBox> {
        if self.sel.len() < 2 {
            return None;
        }
        self.sel_bbox(m).map(SeBox::rect)
    }
}

// ------------------------------------------------------------------ small geometry

fn len(v: [f32; 2]) -> f32 {
    sqrt(v[0] * v[0] + v[1] * v[1])
}

fn unit(v: [f32; 2]) -> [f32; 2] {
    let l = len(v);
    if l > 1e-9 { [v[0] / l, v[1] / l] } else { [0.0, 0.0] }
}

// the angle of a world vector, counter-clockwise as seen (y down)
fn seen_angle(v: [f32; 2]) -> f32 {
    rt::x87::atan2(-v[1] as f64, v[0] as f64) as f32
}

// the rotation handle of a frame: beyond the middle of the edge fy = 0, `d` world units out
fn rot_point(b: &SeBox, d: f32) -> [f32; 2] {
    let top = b.at(0.5, 0.0);
    let n = unit(b.v);
    // v points into the frame from its top edge; a degenerate v: straight up
    let n = if n == [0.0, 0.0] { [0.0, 1.0] } else { n };
    [top[0] - n[0] * d, top[1] - n[1] * d]
}

// the size cursor for a handle in direction dir (world, from the middle of the frame)
fn size_cursor(dir: [f32; 2]) -> u32 {
    // 0 .. 180 (atan2 gives -180 .. 180)
    let mut a = seen_angle(dir).to_degrees();
    if a < 0.0 {
        a += 180.0;
    }
    if a >= 180.0 {
        a -= 180.0;
    }
    if !(22.5..157.5).contains(&a) {
        CUR_SIZE_WE
    } else if a < 67.5 {
        CUR_SIZE_NESW
    } else if a < 112.5 {
        CUR_SIZE_NS
    } else {
        CUR_SIZE_NWSE
    }
}

// the frame after dragging handle h of `from` to fractions f of it
fn resized(from: &SeBox, h: u8, f: [f32; 2], aspect: bool, centred: bool, min: f32) -> SeBox {
    let hp = HANDLES[h as usize & 7];
    let (mut x0, mut x1, mut y0, mut y1) = (0.0f32, 1.0f32, 0.0f32, 1.0f32);
    if hp[0] == 0.0 {
        x0 = f[0];
    } else if hp[0] == 1.0 {
        x1 = f[0];
    }
    if hp[1] == 0.0 {
        y0 = f[1];
    } else if hp[1] == 1.0 {
        y1 = f[1];
    }
    if centred {
        if hp[0] == 0.0 {
            x1 = 1.0 - x0;
        } else if hp[0] == 1.0 {
            x0 = 1.0 - x1;
        }
        if hp[1] == 0.0 {
            y1 = 1.0 - y0;
        } else if hp[1] == 1.0 {
            y0 = 1.0 - y1;
        }
    }
    let sz = from.size();
    let (mx, my) = (if sz[0] > 0.0 { min / sz[0] } else { 0.0 }, if sz[1] > 0.0 { min / sz[1] } else { 0.0 });
    // no flip through the opposite side
    let fixw = |a: &mut f32, b: &mut f32, moving_a: bool, m: f32| {
        if *b - *a < m {
            if moving_a {
                *a = *b - m;
            } else {
                *b = *a + m;
            }
        }
    };
    if centred {
        let cx = 0.5;
        if x1 - x0 < mx {
            x0 = cx - mx * 0.5;
            x1 = cx + mx * 0.5;
        }
        if y1 - y0 < my {
            y0 = 0.5 - my * 0.5;
            y1 = 0.5 + my * 0.5;
        }
    } else {
        fixw(&mut x0, &mut x1, hp[0] == 0.0, mx);
        fixw(&mut y0, &mut y1, hp[1] == 0.0, my);
    }
    if aspect {
        // one factor for both sides: the one that changed more; the opposite corner (or the
        // middle) stays
        let (kx, ky) = (x1 - x0, y1 - y0);
        let k = if hp[0] == 0.5 {
            ky
        } else if hp[1] == 0.5 {
            kx
        } else if (kx - 1.0).abs() >= (ky - 1.0).abs() {
            kx
        } else {
            ky
        };
        let k = k.max(mx.max(my));
        let place = |a: &mut f32, b: &mut f32, side: f32| {
            if centred || side == 0.5 {
                let c = (*a + *b) * 0.5;
                *a = c - k * 0.5;
                *b = c + k * 0.5;
            } else if side == 0.0 {
                *a = *b - k;
            } else {
                *b = *a + k;
            }
        };
        place(&mut x0, &mut x1, hp[0]);
        place(&mut y0, &mut y1, hp[1]);
    }
    SeBox { o: from.at(x0, y0), u: [from.u[0] * (x1 - x0), from.u[1] * (x1 - x0)], v: [from.v[0] * (y1 - y0), from.v[1] * (y1 - y0)] }
}

// a handle is usable: its sides are not locked
fn handle_free(flags: u32, h: u8) -> bool {
    let hp = HANDLES[h as usize & 7];
    (hp[0] == 0.5 || flags & SEF_LOCK_W == 0) && (hp[1] == 0.5 || flags & SEF_LOCK_H == 0)
}

// the box a shape aligns and snaps by: its frame (2D) or its ends (1D), not what its line width
// adds
fn align_box(sh: &SeShape) -> [f32; 4] {
    if sh.flags & SEF_1D != 0 {
        let (a, b) = (sh.ends[0], sh.ends[1]);
        [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
    } else {
        sh.bx.aabb()
    }
}

// a point inside a closed outline (even-odd)
fn in_polygon(pts: &[[f32; 2]], p: [f32; 2]) -> bool {
    let n = pts.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (pts[i], pts[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

// the quarter ellipse from a to b with axes along x and y, leaving a horizontally or vertically
// (the arc tool's rubber band; diagram_core::ops::quarter_arc makes the shape)
fn arc_points(a: [f32; 2], b: [f32; 2], horizontal: bool) -> Vec<[f32; 2]> {
    let n = 24;
    (0..=n)
        .map(|k| {
            let (sn, cs) = rt::x87::sincos(k as f64 * core::f64::consts::FRAC_PI_2 / n as f64);
            let (sn, cs) = (sn as f32, cs as f32);
            if horizontal {
                [a[0] + (b[0] - a[0]) * sn, b[1] + (a[1] - b[1]) * cs]
            } else {
                [b[0] + (a[0] - b[0]) * cs, a[1] + (b[1] - a[1]) * sn]
            }
        })
        .collect()
}

fn snap_angle(a: f32, step_deg: f32) -> f32 {
    let s = step_deg.to_radians();
    fl(a / s + 0.5) as f32 * s
}

// ------------------------------------------------------------------ the widget

impl Ui {
    /// ui_shape_edit(id, st, dg, m, tool) -> what the user did: the edit layer of the canvas
    /// `id`, called right after ui.diagram(id, dg, .., DG_EDIT, ..) (see the file header)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn shape_edit(&mut self, id: u32, st: &mut SeState, dg: &mut DiagramState, m: &SeModel, tool: u32) -> Vec<SeEv> {
        let _src = self.src_scope();
        let mut out = Vec::new();
        if st.update_index(m) {
            out.push(SeEv::Select);
        }
        if st.pending_sel {
            st.pending_sel = false;
            out.push(SeEv::Select);
        }
        out.append(&mut st.pend);
        let i = dg.ext;
        let r = dg.vp.rect;
        let scale = dg.vp.scale() as f32;
        if !(scale > 0.0) || r[2] <= 0 || r[3] <= 0 {
            return out;
        }
        let w = |dip: i32| self.dp(dip) as f32 / scale; // DIP -> world units
        // hover (the pointer over the canvas, nothing else running)
        let idle = matches!(st.g, G::Idle | G::Hold);
        st.hover = if i.track && !i.space && !i.panning { self.se_hit(st, dg, m, i.p, tool) } else { SHit::None };
        st.hover_shape = if i.track && !i.panning { (m.hit)(i.p, w(3), 0) } else { 0 };
        // the pin shows from the rotation handle on, while the pointer stays on the shape
        match st.hover {
            SHit::Rotate(k) => st.pin_shown = k,
            SHit::Pin(_) | SHit::Shape(_) | SHit::Handle(..) => {}
            _ if matches!(st.g, G::Rotate { .. } | G::Pin { .. }) => {}
            _ => st.pin_shown = 0,
        }
        if st.single(m).is_none_or(|s| s.key != st.pin_shown) {
            st.pin_shown = 0;
        }
        // connection arrows and insertion marks (they may take the hover)
        self.se_extras(st, dg, m, &i, tool, &mut out);
        let was_idle = matches!(st.g, G::Idle);
        // the gesture
        let g = core::mem::take(&mut st.g);
        st.g = match g {
            G::Idle => self.se_idle(st, dg, m, &i, tool, &mut out),
            G::Press { hit, px, at, mods, was } => {
                let far = (i.px[0] - px[0]).abs() > i.lim || (i.px[1] - px[1]).abs() > i.lim;
                if i.btn & (1 << BTN_LEFT) == 0 {
                    self.se_click(st, dg, m, tool, hit, at, mods, was, &mut out)
                } else if far {
                    st.last_hit = None;
                    self.se_drag(st, dg, m, tool, hit, at, mods)
                } else {
                    G::Press { hit, px, at, mods, was }
                }
            }
            g => g,
        };
        let g = core::mem::take(&mut st.g);
        st.g = self.se_follow(st, dg, m, &i, g, &mut out);
        // Ctrl + Shift + right drag pans (KB-04); released without a drag it is the click below
        let right = 1 << BTN_RIGHT;
        let both = i.mods & (MOD_CTRL | MOD_SHIFT) == MOD_CTRL | MOD_SHIFT;
        if st.rpan.is_none() && idle && both && i.pressed & right != 0 && i.ev & EV_HOVER != 0 && !i.space && !i.panning {
            st.rpan = Some(RPan { px: i.px, cx: dg.vp.cx, cy: dg.vp.cy, moved: false });
        }
        let mut panned = false;
        if let Some(mut rp) = st.rpan.take() {
            if (i.px[0] - rp.px[0]).abs() > i.lim || (i.px[1] - rp.px[1]).abs() > i.lim {
                rp.moved = true;
            }
            if rp.moved {
                let sc = dg.vp.scale();
                let (cx, cy) = (rp.cx - (i.px[0] - rp.px[0]) as f64 / sc, rp.cy - (i.px[1] - rp.px[1]) as f64 / sc);
                if (cx, cy) != (dg.vp.cx, dg.vp.cy) {
                    dg.vp.cx = cx;
                    dg.vp.cy = cy;
                    self.repass();
                }
            }
            if i.btn & right != 0 {
                st.rpan = Some(rp);
            } else {
                panned = rp.moved;
            }
        }
        // right click (not with Ctrl + Shift: the magnifier zooms out; nothing after a pan)
        st.rtaken = panned || (i.ev & EV_RCLICK != 0 && idle && both);
        if i.ev & EV_RCLICK != 0 && idle && !panned {
            if both {
                dg.vp.step(-1, i.px[0] as f32 + 0.5, i.px[1] as f32 + 0.5);
            } else {
                let k = match st.hover {
                    SHit::Shape(k) | SHit::Handle(k, _) | SHit::Rotate(k) | SHit::Pin(k) | SHit::End(k, _) | SHit::Point(k, _) | SHit::Arrow(k, _) => k,
                    // a mark lies on a shape: that one
                    SHit::Mark(..) => st.shape(m, st.hover_shape).filter(|s| s.flags & SEF_NOSELECT == 0).map_or(0, |s| s.key),
                    _ => 0,
                };
                if k != 0 && !st.sel.contains(&k) {
                    st.select(&[k]);
                    out.push(SeEv::Select);
                }
                out.push(SeEv::Context { key: k, at: [i.wx, i.px[1]] });
            }
        }
        // the keys while the canvas has the focus and no popup (a menu keeps its own keys)
        if self.wp.ctx.focus == id && self.popup_count() == 0 {
            self.se_keys(st, dg, m, &mut out);
        }
        // a gesture or a click that ended: the arrows wait until the pointer moves; a gesture
        // hides them (a press on an arrow or a mark keeps them)
        if !was_idle && matches!(st.g, G::Idle) {
            st.quiet = Some(i.px);
        }
        if st.quiet.is_some() || !matches!(st.g, G::Idle | G::Press { hit: SHit::Arrow(..) | SHit::Mark(..), .. }) {
            st.extras_off(&mut out);
        }
        // the cursor for the next call of ui.diagram
        dg.ext_cursor = self.se_cursor(st, m, tool);
        dg.ext_moving = st.live() || st.rpan.is_some_and(|r| r.moved) || matches!(st.g, G::Marquee { .. } | G::Lasso { .. } | G::Connect { .. } | G::Draw { .. } | G::Zoom { .. } | G::Stroke { .. } | G::Arc { .. });
        // (pointer moves bring the frames of a gesture: no pass is asked for while one runs)
        if !out.is_empty() {
            self.repass();
        }
        // drawing
        let (last_rect, last_state) = (self.wp.ctx.last_rect, self.wp.ctx.last_state);
        self.ltr_begin(r[0], r[2]);
        self.clip_push(r[0], r[1], r[2], r[3]);
        self.se_overlay(st, dg, m, tool);
        self.clip_pop();
        self.ltr_end();
        self.wp.ctx.last_rect = last_rect;
        self.wp.ctx.last_state = last_state;
        out
    }

    // ---- hit testing

    fn se_hit(&self, st: &SeState, dg: &DiagramState, m: &SeModel, p: [f32; 2], tool: u32) -> SHit {
        let s = dg.vp.scale() as f32;
        let w = |dip: i32| self.dp(dip) as f32 / s;
        let hr = w(HANDLE_DIP + 2);
        // the drawing tools keep the handles of the shape just drawn (Visio's)
        let frame_tool = matches!(tool, SE_POINTER | SE_RECT | SE_ELLIPSE | SE_TEXT);
        if frame_tool || matches!(tool, SE_LINE | SE_FREEFORM | SE_ARC) {
            if let Some(sh) = st.single(m) {
                // points first (they lie on the frame), then ends, rotation, pin, handles
                let mut best: Option<(f32, u32)> = None;
                for k in range_of(&sh.points, m.points.len()) {
                    let q = &m.points[k];
                    if q.kind == SP_CONN {
                        continue;
                    }
                    let d = dist(p, q.at);
                    if d <= hr && best.is_none_or(|b| d < b.0) {
                        best = Some((d, k as u32));
                    }
                }
                if let Some((_, k)) = best {
                    return SHit::Point(sh.key, k);
                }
                if sh.flags & SEF_1D != 0 {
                    for e in 0..2u8 {
                        if dist(p, sh.ends[e as usize]) <= hr {
                            return SHit::End(sh.key, e);
                        }
                    }
                } else if frame_tool {
                    if sh.flags & SEF_LOCK_ROTATE == 0 && dist(p, rot_point(&sh.bx, w(ROT_DIP))) <= hr {
                        return SHit::Rotate(sh.key);
                    }
                    for (h, hp) in HANDLES.iter().enumerate() {
                        if dist(p, sh.bx.at(hp[0], hp[1])) <= hr {
                            return SHit::Handle(sh.key, h as u8);
                        }
                    }
                    if st.pin_shown == sh.key && sh.flags & (SEF_LOCK_PIN | SEF_LOCK_ROTATE) == 0 && dist(p, sh.pin) <= w(4) {
                        return SHit::Pin(sh.key);
                    }
                }
            } else if let (Some(b), SE_POINTER) = (st.multi_box(m), tool) {
                if dist(p, rot_point(&b, w(ROT_DIP))) <= hr {
                    return SHit::SelRotate;
                }
                for (h, hp) in HANDLES.iter().enumerate() {
                    if dist(p, b.at(hp[0], hp[1])) <= hr {
                        return SHit::SelHandle(h as u8);
                    }
                }
            }
        }
        // a shape: inside the group of a selected child first (sub-selection)
        let within = st.single(m).map_or(0, |s| s.parent);
        let tol = w(3);
        let k = if within != 0 { (m.hit)(p, tol, within) } else { 0 };
        let k = if k != 0 { k } else { (m.hit)(p, tol, 0) };
        match st.shape(m, k) {
            Some(s) if s.flags & SEF_NOSELECT == 0 => SHit::Shape(k),
            _ => SHit::None,
        }
    }

    // the connection arrows of shape `key` (None: it gets none)
    fn se_arrow_set(&self, st: &SeState, dg: &DiagramState, m: &SeModel, key: u64) -> Option<ArrowSet> {
        let sh = st.shape(m, key)?;
        if sh.parent != 0 || sh.flags & (SEF_1D | SEF_NOSELECT | SEF_NOARROWS) != 0 {
            return None;
        }
        let vp = dg.vp;
        let (a, z) = (vp.to_screen([sh.bbox[0], sh.bbox[1]]), vp.to_screen([sh.bbox[2], sh.bbox[3]]));
        let b = [a[0].min(z[0]), a[1].min(z[1]), a[0].max(z[0]), a[1].max(z[1])];
        if !b.iter().all(|v| v.is_finite()) || !((b[2] - b[0]).max(b[3] - b[1]) >= self.dp(ARROW_MIN) as f32) {
            return None;
        }
        let mut set = ArrowSet { b, base: [self.dp(ARROW_GAP) as f32; 4], len: self.dp(ARROW_LEN) as f32, half: self.dp(ARROW_HALF) as f32 };
        // the rotation handle of the selection: an arrow that would cover it goes beyond it
        let rot_d = self.dp(ROT_DIP) as f32 / vp.scale() as f32;
        let handle = match st.single(m) {
            Some(one) if one.flags & (SEF_1D | SEF_LOCK_ROTATE) == 0 => Some(rot_point(&one.bx, rot_d)),
            Some(_) => None,
            None => st.multi_box(m).map(|bx| rot_point(&bx, rot_d)),
        };
        if let Some(q) = handle.map(|q| vp.to_screen(q)) {
            let rr = (self.dp(HANDLE_DIP + 3) + self.hair().max(1)) as f32;
            for d in 0..4 {
                let (t, l) = set.local(d, q);
                if l.abs() < set.half + rr && t + rr > set.base[d] && t - rr < set.base[d] + set.len {
                    set.base[d] = t + rr;
                }
            }
        }
        Some(set)
    }

    // the triangle of an insertion mark (canvas px)
    fn se_mark_tri(&self, dg: &DiagramState, mk: &SeMark) -> [[f32; 2]; 3] {
        tri_at(dg.vp.to_screen(mk.at), mk.dir as usize, self.dp(MARK_LEN) as f32, self.dp(MARK_HALF) as f32)
    }

    // does the insertion mark show: the pointer on its owner or near it
    fn se_mark_shown(&self, st: &SeState, dg: &DiagramState, m: &SeModel, mk: &SeMark) -> bool {
        let Some(p) = st.mark_p else { return false };
        if mk.owner != 0 && st.hover_shape == mk.owner {
            return true;
        }
        let tol = self.dp(3) as f32 / dg.vp.scale() as f32;
        if mk.owner != 0 && st.shape(m, mk.owner).is_some_and(|s| inside([p[0], p[1], p[0], p[1]], [s.bbox[0] - tol, s.bbox[1] - tol, s.bbox[2] + tol, s.bbox[3] + tol])) {
            return true;
        }
        let t = self.se_mark_tri(dg, mk);
        let c = [(t[0][0] + t[1][0] + t[2][0]) / 3.0, (t[0][1] + t[1][1] + t[2][1]) / 3.0];
        dist(dg.vp.to_screen(p), c) <= self.dp(MARK_NEAR) as f32
    }

    // the arrows and marks while the pointer rests on the canvas with the pointer tool (no
    // gesture, no button but a press now): which show, which one is under the pointer; that one
    // becomes the hover unless a handle is there
    fn se_extras(&mut self, st: &mut SeState, dg: &DiagramState, m: &SeModel, i: &ExtIn, tool: u32, out: &mut Vec<SeEv>) {
        if st.quiet.is_some_and(|q| q != i.px) {
            st.quiet = None;
        }
        if let G::Press { hit: hit @ (SHit::Arrow(..) | SHit::Mark(..)), .. } = st.g {
            // held on one: it stays as it was
            st.hover = hit;
            return;
        }
        let left = 1 << BTN_LEFT;
        let rest = matches!(st.g, G::Idle) && (i.btn & left == 0 || i.pressed & left != 0);
        if tool != SE_POINTER || !rest || !i.track || i.space || i.panning || st.quiet.is_some() {
            st.extras_off(out);
            return;
        }
        let p = dg.vp.to_screen(i.p);
        let tol = self.dp(2) as f32;
        // the arrows: the current shape's while the pointer is on one of them, on one of its
        // handles or in their zone; else those of the top-level shape under the pointer
        let (mut key, mut hot) = (0u64, 255u8);
        if st.arrows {
            let cur = if st.arrow_key != 0 { self.se_arrow_set(st, dg, m, st.arrow_key) } else { None };
            let on_handle = matches!(st.hover, SHit::Handle(k, _) | SHit::Rotate(k) | SHit::Pin(k) | SHit::Point(k, _) if k == st.arrow_key);
            if let Some(d) = cur.and_then(|a| a.hit(p, tol)) {
                (key, hot) = (st.arrow_key, d);
            } else if cur.is_some() && on_handle {
                key = st.arrow_key;
            } else if st.hover_shape != 0 && self.se_arrow_set(st, dg, m, st.hover_shape).is_some() {
                key = st.hover_shape;
            } else if cur.is_some_and(|a| a.zone(p, tol)) {
                key = st.arrow_key;
            }
        }
        st.arrow_key = key;
        st.arrow_hot = hot;
        // the marks
        st.mark_p = (!m.marks.is_empty()).then_some(i.p);
        st.mark_hot = None;
        let mut best = f32::MAX;
        for (k, mk) in m.marks.iter().enumerate() {
            if !self.se_mark_shown(st, dg, m, mk) {
                continue;
            }
            let t = self.se_mark_tri(dg, mk);
            let v = DIRS[mk.dir as usize & 3];
            let (dx, dy) = (t[0][0] - p[0], t[0][1] - p[1]);
            // back from the tip, across
            let (along, across) = (dx * v[0] + dy * v[1], dy * v[0] - dx * v[1]);
            let (len, half) = (self.dp(MARK_LEN) as f32, self.dp(MARK_HALF) as f32);
            if along >= -tol && along <= len + tol && across.abs() <= half + tol {
                let d = (along - len * 0.5).abs() + across.abs();
                if d < best {
                    best = d;
                    st.mark_hot = Some((k as u32, mk.tag));
                }
            }
        }
        // the hover: handles first (they lie on top), then a mark, then an arrow.  Only the
        // handles of the shape whose arrows show (or of a selection box around it) beat its arrow:
        // a selected neighbour's handle under the arrow (the shape a first arrow click just made
        // lies the page's spacing away, its side handle under the arrow) is not what the pointer,
        // coming from the arrows' shape, is on -- the arrow shows hot and takes the click
        let handle = match st.hover {
            SHit::Handle(k, _) | SHit::Rotate(k) | SHit::Pin(k) | SHit::End(k, _) | SHit::Point(k, _) => hot == 255 || k == key,
            SHit::SelHandle(_) | SHit::SelRotate => hot == 255 || st.sel.contains(&key),
            _ => false,
        };
        if handle {
            st.arrow_hot = 255;
            st.mark_hot = None;
        } else if let Some((k, tag)) = st.mark_hot {
            st.arrow_hot = 255;
            st.hover = SHit::Mark(k, tag);
        } else if hot != 255 {
            st.hover = SHit::Arrow(key, hot);
        }
        st.say_arrow(out);
    }

    // what an end or a control point at p would glue to; the point it would go to.  `swap`
    // (Ctrl held, a dynamic end): a connection point under p gives its shape as a whole, a shape
    // under p its nearest connection point (CN-03)
    #[allow(clippy::too_many_arguments)]
    fn se_glue(&self, st: &SeState, dg: &DiagramState, m: &SeModel, p: [f32; 2], exclude: u64, dynamic: bool, swap: bool) -> (SeGlue, [f32; 2]) {
        if !st.glue {
            return (SeGlue::None, p);
        }
        let s = dg.vp.scale() as f32;
        let reach = self.dp(GLUE_DIP) as f32 / s;
        let swap = swap && dynamic;
        let whole = |k: u64| k != 0 && k != exclude && st.shape(m, k).is_some_and(|sh| sh.flags & (SEF_NOGLUE | SEF_1D) == 0);
        // connection points of the shapes near p (those pointing outward only are no target)
        let target = |q: &SePoint| q.kind == SP_CONN && q.flags & SPF_OUTWARD == 0;
        let mut best: Option<(f32, u64, u32, [f32; 2])> = None;
        for sh in m.shapes {
            if sh.key == exclude || sh.flags & SEF_NOGLUE != 0 || sh.points.is_empty() {
                continue;
            }
            let b = sh.bbox;
            if !overlaps([b[0] - reach, b[1] - reach, b[2] + reach, b[3] + reach], [p[0], p[1], p[0], p[1]]) {
                continue;
            }
            for k in range_of(&sh.points, m.points.len()) {
                let q = &m.points[k];
                if !target(q) {
                    continue;
                }
                let d = dist(p, q.at);
                if d <= reach && best.is_none_or(|bb| d < bb.0) {
                    best = Some((d, sh.key, q.tag, q.at));
                }
            }
        }
        if let Some((_, key, tag, at)) = best {
            if swap && whole(key) {
                return (SeGlue::Shape(key), p);
            }
            return (SeGlue::Point { key, tag }, at);
        }
        if dynamic {
            let k = (m.hit)(p, self.dp(2) as f32 / s, 0);
            if whole(k) {
                if swap {
                    // the shape's nearest connection point
                    let near = st.shape(m, k).and_then(|sh| {
                        range_of(&sh.points, m.points.len()).map(|j| &m.points[j]).filter(|q| target(q)).min_by(|a, b| dist(p, a.at).total_cmp(&dist(p, b.at)))
                    });
                    if let Some(q) = near {
                        return (SeGlue::Point { key: k, tag: q.tag }, q.at);
                    }
                }
                return (SeGlue::Shape(k), p);
            }
        }
        (SeGlue::None, p)
    }

    fn se_snap_pt(&self, dg: &DiagramState, p: [f32; 2], mods: u32) -> [f32; 2] {
        if dg.snap && dg.grid > 0.0 && mods & MOD_ALT == 0 { [snapv(p[0], dg.grid), snapv(p[1], dg.grid)] } else { p }
    }

    fn se_cursor(&self, st: &SeState, m: &SeModel, tool: u32) -> u32 {
        if st.rpan.is_some_and(|r| r.moved) {
            return CUR_HAND;
        }
        match st.g {
            G::Move { .. } => return CUR_MOVE,
            G::Resize { h, ref to, .. } => return size_cursor([to.at(HANDLES[h as usize][0], HANDLES[h as usize][1])[0] - to.at(0.5, 0.5)[0], to.at(HANDLES[h as usize][0], HANDLES[h as usize][1])[1] - to.at(0.5, 0.5)[1]]),
            G::Rotate { .. } | G::Pin { .. } | G::End { .. } | G::Point { .. } | G::Connect { .. } | G::Draw { .. } | G::Zoom { .. } | G::Stroke { .. } | G::Arc { .. } => return CUR_CROSS,
            _ => {}
        }
        let frame_cursor = |b: &SeBox, h: u8| {
            let q = b.at(HANDLES[h as usize][0], HANDLES[h as usize][1]);
            let c = b.at(0.5, 0.5);
            size_cursor([q[0] - c[0], q[1] - c[1]])
        };
        match st.hover {
            SHit::Handle(k, h) => match st.shape(m, k) {
                Some(s) if handle_free(s.flags, h) => frame_cursor(&s.bx, h),
                _ => CUR_ARROW,
            },
            SHit::SelHandle(h) => st.multi_box(m).map_or(CUR_ARROW, |b| frame_cursor(&b, h)),
            SHit::Rotate(_) | SHit::SelRotate | SHit::Pin(_) | SHit::End(..) | SHit::Point(..) => CUR_CROSS,
            SHit::Shape(_) if tool == SE_POINTER => CUR_MOVE,
            _ if tool != SE_POINTER => CUR_CROSS,
            _ => CUR_ARROW,
        }
    }

    // ---- gestures

    fn se_idle(&mut self, st: &mut SeState, _dg: &mut DiagramState, m: &SeModel, i: &ExtIn, tool: u32, out: &mut Vec<SeEv>) -> G {
        let lpress = i.ev & EV_HOVER != 0 && i.pressed & (1 << BTN_LEFT) != 0 && i.mine && !i.space && !i.panning;
        if !lpress {
            return G::Idle;
        }
        // the magnifier; on a selected shape a press that becomes a drag copies the selection
        // (KB-09), a click zooms in there too (se_click)
        if i.mods & (MOD_CTRL | MOD_SHIFT) == MOD_CTRL | MOD_SHIFT {
            if let (SHit::Shape(k), SE_POINTER) = (st.hover, tool) {
                if st.sel.contains(&k) {
                    st.last_hit = None;
                    return G::Press { hit: st.hover, px: i.px, at: i.p, mods: i.mods, was: true };
                }
            }
            return G::Zoom { start: i.p, cur: i.p };
        }
        let hit = st.hover;
        // a double click on a shape: edit its text
        let near = (i.px[0] - st.last_px[0]).abs() <= i.lim * 2 && (i.px[1] - st.last_px[1]).abs() <= i.lim * 2;
        if st.last_hit == Some(hit) && near && i.now.wrapping_sub(st.last_ms) <= i.dbl {
            st.last_hit = None;
            if let SHit::Shape(k) = hit {
                out.push(SeEv::Text { key: k, ch: None });
                return G::Hold;
            }
        } else {
            st.last_hit = Some(hit);
            st.last_ms = i.now;
            st.last_px = i.px;
        }
        let mut was = false;
        if let SHit::Shape(k) = hit {
            was = st.sel.contains(&k);
            let changed = if i.mods & MOD_CTRL != 0 {
                if was {
                    st.sel.remove(&k);
                } else {
                    st.sel.insert(k);
                }
                true
            } else if i.mods & MOD_SHIFT != 0 {
                st.sel.insert(k)
            } else if !was {
                st.select(&[k]);
                true
            } else {
                false
            };
            if changed {
                if !st.sel.contains(&st.primary) {
                    st.primary = st.sel.iter().next().copied().unwrap_or(0);
                }
                if st.primary == 0 || st.sel.len() == 1 {
                    st.primary = k;
                }
                st.vertex = None;
                out.push(SeEv::Select);
            }
            st.focus = k;
        }
        // a vertex: chosen (Del removes it)
        if let SHit::Point(k, idx) = hit {
            if let Some(q) = m.points.get(idx as usize).filter(|q| q.kind == SP_VERTEX) {
                st.vertex = Some((k, q.tag));
            }
        }
        // Ctrl + click on a selected shape with vertices: a new vertex
        if let (SHit::Shape(k), true) = (hit, i.mods & MOD_CTRL != 0 && tool == SE_LINE) {
            if st.shape(m, k).is_some_and(|s| range_of(&s.points, m.points.len()).any(|j| m.points[j].kind == SP_VERTEX)) {
                st.sel.insert(k);
                out.push(SeEv::PointInsert { key: k, at: i.p });
                return G::Hold;
            }
        }
        G::Press { hit, px: i.px, at: i.p, mods: i.mods, was }
    }

    #[allow(clippy::too_many_arguments)]
    fn se_click(&mut self, st: &mut SeState, dg: &mut DiagramState, m: &SeModel, tool: u32, hit: SHit, at: [f32; 2], mods: u32, was: bool, out: &mut Vec<SeEv>) -> G {
        if (tool == SE_TEXT && !matches!(hit, SHit::Shape(_))) || tool == SE_PLACE {
            let a = self.se_snap_pt(dg, at, mods);
            out.push(SeEv::Draw { tool, a, b: a });
            return G::Idle;
        }
        // Ctrl + Shift + click on a selected shape: the magnifier as anywhere else
        if mods & (MOD_CTRL | MOD_SHIFT) == MOD_CTRL | MOD_SHIFT {
            let p = dg.vp.to_screen(at);
            dg.vp.step(1, p[0], p[1]);
            return G::Idle;
        }
        match hit {
            SHit::Shape(k) => {
                if mods & (MOD_SHIFT | MOD_CTRL) == 0 && was {
                    // a second click on a selected group: the child under the pointer
                    let group = st.shape(m, k).is_some_and(|s| s.flags & SEF_GROUP != 0);
                    let child = if group && st.sel.len() == 1 { (m.hit)(at, self.dp(3) as f32 / dg.vp.scale() as f32, k) } else { 0 };
                    if child != 0 && child != k && st.shape(m, child).is_some_and(|s| s.flags & SEF_NOSELECT == 0) {
                        st.select(&[child]);
                        out.push(SeEv::Select);
                    } else if st.sel.len() > 1 {
                        st.select(&[k]);
                        out.push(SeEv::Select);
                    }
                }
            }
            SHit::None => {
                if mods & (MOD_SHIFT | MOD_CTRL) == 0 && (!st.sel.is_empty() || st.vertex.is_some()) {
                    st.sel.clear();
                    st.primary = 0;
                    st.vertex = None;
                    out.push(SeEv::Select);
                }
            }
            // the selection stays: the app selects what it makes
            SHit::Arrow(key, dir) => out.push(SeEv::Arrow { key, dir }),
            SHit::Mark(_, tag) => out.push(SeEv::Mark { tag }),
            _ => {}
        }
        G::Idle
    }

    fn se_drag(&mut self, st: &mut SeState, dg: &mut DiagramState, m: &SeModel, tool: u32, hit: SHit, at: [f32; 2], mods: u32) -> G {
        // the drawing tools draw wherever the press was (on a shape too), but on a handle
        let on_handle = matches!(hit, SHit::Handle(..) | SHit::Rotate(_) | SHit::Pin(_) | SHit::Point(..) | SHit::End(..));
        match hit {
            // a drag from a connection arrow: the connector tool from the middle of that side
            SHit::Arrow(k, d) => {
                let Some(sh) = st.shape(m, k) else { return G::Hold };
                let b = sh.bbox;
                let (cx, cy) = ((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5);
                let a = match d & 3 {
                    0 => [b[2], cy],
                    1 => [cx, b[3]],
                    2 => [b[0], cy],
                    _ => [cx, b[1]],
                };
                return G::Connect { tool: SE_CONNECT, a, ga: SeGlue::Shape(k), b: a };
            }
            // a mark is for clicks only
            SHit::Mark(..) => return G::Hold,
            _ => {}
        }
        if tool == SE_PLACE {
            return G::Idle;
        }
        if matches!(tool, SE_RECT | SE_ELLIPSE | SE_TEXT) && !on_handle {
            let a = self.se_snap_pt(dg, at, mods);
            return G::Draw { tool, a, b: a };
        }
        // the freeform and the arc tool start glued like a line (CN-12); the stroke starts where
        // the press was, the arc on the grid
        if matches!(tool, SE_FREEFORM | SE_ARC) && !matches!(hit, SHit::Point(..) | SHit::End(..)) {
            let (ga, a) = self.se_glue(st, dg, m, at, 0, false, false);
            if tool == SE_FREEFORM {
                return G::Stroke { pts: alloc::vec![a], ga };
            }
            let a = if ga == SeGlue::None { self.se_snap_pt(dg, a, mods) } else { a };
            return G::Arc { a, ga, b: a, h: None };
        }
        if tool == SE_CONNECT || (tool == SE_LINE && !matches!(hit, SHit::Point(..) | SHit::End(..))) {
            let dynamic = tool == SE_CONNECT;
            let (ga, a) = self.se_glue(st, dg, m, at, 0, dynamic, mods & MOD_CTRL != 0);
            let a = if ga == SeGlue::None { self.se_snap_pt(dg, a, mods) } else { a };
            return G::Connect { tool, a, ga, b: a };
        }
        match hit {
            SHit::Shape(k) if st.sel.contains(&k) => {
                // Ctrl + Shift: a copy of the whole selection moves (KB-09)
                let copy = mods & (MOD_CTRL | MOD_SHIFT) == MOD_CTRL | MOD_SHIFT;
                let keys = if copy { st.sel.iter().copied().collect() } else { st.movable(m) };
                if keys.is_empty() {
                    return G::Hold;
                }
                let mut bb: Option<[f32; 4]> = None;
                for &key in &keys {
                    if let Some(sh) = st.shape(m, key) {
                        let r = align_box(sh);
                        bb = Some(match bb {
                            None => r,
                            Some(b) => [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[2]), b[3].max(r[3])],
                        });
                    }
                }
                G::Move { keys, start: at, bb: bb.unwrap_or([at[0], at[1], at[0], at[1]]), total: [0.0; 2], copy }
            }
            SHit::Handle(k, h) => match st.shape(m, k) {
                Some(sh) if handle_free(sh.flags, h) => {
                    let corner = HANDLES[h as usize][0] != 0.5 && HANDLES[h as usize][1] != 0.5;
                    G::Resize { keys: alloc::vec![k], h, from: sh.bx, to: sh.bx, aspect: corner || sh.flags & SEF_LOCK_ASPECT != 0 }
                }
                _ => G::Hold,
            },
            SHit::SelHandle(h) => match st.multi_box(m) {
                Some(b) => {
                    let corner = HANDLES[h as usize][0] != 0.5 && HANDLES[h as usize][1] != 0.5;
                    G::Resize { keys: st.movable(m), h, from: b, to: b, aspect: corner }
                }
                None => G::Hold,
            },
            SHit::Rotate(k) => match st.shape(m, k) {
                Some(sh) => {
                    let c = sh.pin;
                    let a0 = seen_angle([at[0] - c[0], at[1] - c[1]]);
                    G::Rotate { keys: alloc::vec![k], centre: c, last: a0, turn: 0.0, angle: 0.0 }
                }
                None => G::Hold,
            },
            SHit::SelRotate => match st.sel_bbox(m) {
                Some(b) => {
                    let c = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
                    let a0 = seen_angle([at[0] - c[0], at[1] - c[1]]);
                    let keys: Vec<u64> = st.sel.iter().copied().filter(|&k| st.shape(m, k).is_some_and(|x| x.flags & SEF_LOCK_ROTATE == 0)).collect();
                    G::Rotate { keys, centre: c, last: a0, turn: 0.0, angle: 0.0 }
                }
                None => G::Hold,
            },
            SHit::Pin(k) => G::Pin { key: k, at },
            SHit::End(k, e) => match st.shape(m, k) {
                Some(sh) if sh.flags & if e == 0 { SEF_LOCK_BEGIN } else { SEF_LOCK_END } == 0 => {
                    G::End { key: k, end: e, at: sh.ends[e as usize], glue: SeGlue::None, dynamic: sh.flags & SEF_DYNAMIC != 0 }
                }
                _ => G::Hold,
            },
            SHit::Point(k, idx) => match m.points.get(idx as usize) {
                Some(q) => G::Point { key: k, idx, tag: q.tag, at: q.at, glue: SeGlue::None, glues: q.kind == SP_CONTROL && q.flags & SPF_GLUE != 0 },
                None => G::Hold,
            },
            SHit::None if tool == SE_POINTER => {
                let base = if mods & (MOD_SHIFT | MOD_CTRL) != 0 { st.sel.clone() } else { BTreeSet::new() };
                if st.lasso {
                    G::Lasso { pts: alloc::vec![at], mods, base }
                } else {
                    G::Marquee { start: at, cur: at, mods, base }
                }
            }
            _ => G::Hold,
        }
    }

    // a running gesture follows the pointer
    fn se_follow(&mut self, st: &mut SeState, dg: &mut DiagramState, m: &SeModel, i: &ExtIn, g: G, out: &mut Vec<SeEv>) -> G {
        let held = i.btn & (1 << BTN_LEFT) != 0;
        let done = !held;
        let s = dg.vp.scale() as f32;
        let w = |dip: i32| self.dp(dip) as f32 / s;
        st.aligns.clear();
        st.glue_now = SeGlue::None;
        match g {
            G::Hold => {
                if held {
                    G::Hold
                } else {
                    G::Idle
                }
            }
            G::Move { keys, start, bb, total, copy } => {
                let raw = [i.p[0] - start[0], i.p[1] - start[1]];
                // Shift: along one axis, the one the pointer went farther on (KB-09)
                let raw = if i.mods & MOD_SHIFT != 0 { if raw[0].abs() >= raw[1].abs() { [raw[0], 0.0] } else { [0.0, raw[1]] } } else { raw };
                let t = self.se_snap_move(st, dg, m, &keys, bb, raw, i.mods);
                if t != total || done {
                    let keys = keys.clone();
                    out.push(if copy { SeEv::MoveCopy { keys, total: t, done } } else { SeEv::Move { keys, total: t, done } });
                }
                if done { G::Idle } else { G::Move { keys, start, bb, total: t, copy } }
            }
            G::Resize { keys, h, from, to, aspect } => {
                let p = if from.axis_parallel() { self.se_snap_pt(dg, i.p, i.mods) } else { i.p };
                let n = match from.frac(p) {
                    Some(f) => resized(&from, h, f, aspect, i.mods & MOD_CTRL != 0, w(2)),
                    None => to,
                };
                if n != to || done {
                    out.push(SeEv::Resize { keys: keys.clone(), from, to: n, done });
                }
                if done { G::Idle } else { G::Resize { keys, h, from, to: n, aspect } }
            }
            G::Rotate { keys, centre, last, turn, angle } => {
                let v = [i.p[0] - centre[0], i.p[1] - centre[1]];
                let a = seen_angle(v);
                // unwrap: the turn adds up across the half turn
                let mut d = a - last;
                let pi = core::f32::consts::PI;
                if d > pi {
                    d -= 2.0 * pi;
                } else if d < -pi {
                    d += 2.0 * pi;
                }
                let turn = turn + d;
                let raw = turn;
                // near the shape 15-degree steps, farther away single degrees (Alt: free)
                let px = len(v) * s;
                let step = if px < self.dp(120) as f32 { 15.0 } else if px < self.dp(240) as f32 { 5.0 } else { 1.0 };
                let ang = if i.mods & MOD_ALT != 0 { raw } else { snap_angle(raw, step) };
                if ang != angle || done {
                    out.push(SeEv::Rotate { keys: keys.clone(), centre, angle: ang, done });
                }
                if done { G::Idle } else { G::Rotate { keys, centre, last: a, turn, angle: ang } }
            }
            G::Pin { key, at } => {
                let a = self.se_snap_pt(dg, i.p, i.mods);
                if a != at || done {
                    out.push(SeEv::Pin { key, at: a, done });
                }
                if done { G::Idle } else { G::Pin { key, at: a } }
            }
            G::End { key, end, at, glue, dynamic } => {
                let (gl, p) = self.se_glue(st, dg, m, i.p, key, dynamic, i.mods & MOD_CTRL != 0);
                let a = if gl == SeGlue::None { self.se_snap_pt(dg, p, i.mods) } else { p };
                st.glue_now = gl;
                if a != at || gl != glue || done {
                    out.push(SeEv::End { key, end, at: a, glue: gl, done });
                }
                if done { G::Idle } else { G::End { key, end, at: a, glue: gl, dynamic } }
            }
            G::Point { key, idx, tag, at, glue, glues } => {
                let (gl, p) = if glues { self.se_glue(st, dg, m, i.p, key, false, false) } else { (SeGlue::None, i.p) };
                let a = if gl == SeGlue::None { self.se_snap_pt(dg, p, i.mods) } else { p };
                st.glue_now = gl;
                if a != at || gl != glue || done {
                    out.push(SeEv::Point { key, idx, tag, at: a, glue: gl, done });
                }
                if done { G::Idle } else { G::Point { key, idx, tag, at: a, glue: gl, glues } }
            }
            G::Marquee { start, mods, base, .. } => {
                let cur = i.p;
                let rr = [start[0].min(cur[0]), start[1].min(cur[1]), start[0].max(cur[0]), start[1].max(cur[1])];
                let toggle = mods & MOD_CTRL != 0;
                let mut sel = base.clone();
                for sh in m.shapes {
                    // wholly inside, or (SeState::partial) touched
                    let taken = if st.partial { overlaps(sh.bbox, rr) } else { inside(sh.bbox, rr) };
                    if sh.parent != 0 || sh.flags & SEF_NOSELECT != 0 || !taken {
                        continue;
                    }
                    if toggle && sel.contains(&sh.key) {
                        sel.remove(&sh.key);
                    } else {
                        sel.insert(sh.key);
                    }
                }
                if sel != st.sel {
                    st.sel = sel;
                    if !st.sel.contains(&st.primary) {
                        st.primary = st.sel.iter().next().copied().unwrap_or(0);
                    }
                    out.push(SeEv::Select);
                }
                if held { G::Marquee { start, cur, mods, base } } else { G::Idle }
            }
            G::Lasso { mut pts, mods, base } => {
                if pts.last().is_none_or(|q| dist(*q, i.p) * s >= 2.0) && pts.len() < 4096 {
                    pts.push(i.p);
                }
                let toggle = mods & MOD_CTRL != 0;
                let mut sel = base.clone();
                for sh in m.shapes {
                    if sh.parent != 0 || sh.flags & SEF_NOSELECT != 0 {
                        continue;
                    }
                    // fully inside: the corners of its box inside the outline; (SeState::partial)
                    // touched: a corner inside, or the outline's way through its box
                    let b = sh.bbox;
                    let corners = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]];
                    let taken = if st.partial {
                        corners.iter().any(|&c| in_polygon(&pts, c)) || pts.iter().any(|q| inside([q[0], q[1], q[0], q[1]], b))
                    } else {
                        corners.iter().all(|&c| in_polygon(&pts, c))
                    };
                    if !taken {
                        continue;
                    }
                    if toggle && sel.contains(&sh.key) {
                        sel.remove(&sh.key);
                    } else {
                        sel.insert(sh.key);
                    }
                }
                if sel != st.sel {
                    st.sel = sel;
                    if !st.sel.contains(&st.primary) {
                        st.primary = st.sel.iter().next().copied().unwrap_or(0);
                    }
                    out.push(SeEv::Select);
                }
                if held { G::Lasso { pts, mods, base } } else { G::Idle }
            }
            G::Zoom { start, .. } => {
                let cur = i.p;
                if done {
                    let px = (dist(start, cur) * s) as i32;
                    if px > i.lim {
                        dg.vp.fit([start[0].min(cur[0]), start[1].min(cur[1]), start[0].max(cur[0]), start[1].max(cur[1])], 0);
                    } else {
                        dg.vp.step(1, i.px[0] as f32 + 0.5, i.px[1] as f32 + 0.5);
                    }
                    G::Idle
                } else {
                    G::Zoom { start, cur }
                }
            }
            G::Connect { tool, a, ga, .. } => {
                let (gb, p) = self.se_glue(st, dg, m, i.p, 0, tool == SE_CONNECT, i.mods & MOD_CTRL != 0);
                // an end does not glue to the shape the other end glues to as a whole
                let gb = match (ga, gb) {
                    (SeGlue::Shape(x), SeGlue::Shape(y)) if x == y => SeGlue::None,
                    _ => gb,
                };
                let b = if gb == SeGlue::None { self.se_snap_pt(dg, p, i.mods) } else { p };
                st.glue_now = gb;
                if done {
                    if dist(a, b) * s > i.lim as f32 {
                        out.push(SeEv::Connect { tool, a, ga, b, gb });
                    }
                    G::Idle
                } else {
                    G::Connect { tool, a, ga, b }
                }
            }
            G::Draw { tool, a, .. } => {
                let b = self.se_snap_pt(dg, i.p, i.mods);
                // Shift: a square, a circle (KB-09)
                let b = if i.mods & MOD_SHIFT != 0 && matches!(tool, SE_RECT | SE_ELLIPSE) {
                    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                    let d = dx.abs().max(dy.abs());
                    [a[0] + if dx < 0.0 { -d } else { d }, a[1] + if dy < 0.0 { -d } else { d }]
                } else {
                    b
                };
                if done {
                    out.push(SeEv::Draw { tool, a, b });
                    G::Idle
                } else {
                    G::Draw { tool, a, b }
                }
            }
            G::Stroke { mut pts, ga } => {
                if pts.last().is_none_or(|q| dist(*q, i.p) * s >= 2.0) && pts.len() < 8192 {
                    pts.push(i.p);
                }
                let (gb, p) = self.se_glue(st, dg, m, i.p, 0, false, false);
                st.glue_now = gb;
                if done {
                    st.glue_now = SeGlue::None;
                    if gb != SeGlue::None {
                        pts.push(p);
                    }
                    let first = pts[0];
                    if pts.len() >= 2 && pts.iter().any(|q| dist(first, *q) * s > i.lim as f32) {
                        out.push(SeEv::Stroke { pts, ga, gb });
                    }
                    G::Idle
                } else {
                    G::Stroke { pts, ga }
                }
            }
            G::Arc { a, ga, h, .. } => {
                let (gb, p) = self.se_glue(st, dg, m, i.p, 0, false, false);
                let b = if gb == SeGlue::None { self.se_snap_pt(dg, p, i.mods) } else { p };
                // the bow: the axis the pointer left the threshold along
                let raw = [(i.p[0] - a[0]) * s, (i.p[1] - a[1]) * s];
                let h = h.or_else(|| (raw[0].abs().max(raw[1].abs()) > i.lim as f32).then_some(raw[0].abs() >= raw[1].abs()));
                st.glue_now = gb;
                if done {
                    if dist(a, b) * s > i.lim as f32 {
                        out.push(SeEv::Arc { a, ga, b, gb, horizontal: h.unwrap_or(true) });
                    }
                    G::Idle
                } else {
                    G::Arc { a, ga, b, h }
                }
            }
            g => g,
        }
    }

    // the total of a move: the grid, then per axis the nearest guide or else the nearest edge or
    // middle of another shape (the dynamic grid) within reach
    #[allow(clippy::too_many_arguments)]
    fn se_snap_move(&self, st: &mut SeState, dg: &DiagramState, m: &SeModel, keys: &[u64], bb: [f32; 4], raw: [f32; 2], mods: u32) -> [f32; 2] {
        if mods & MOD_ALT != 0 {
            return raw;
        }
        let s = dg.vp.scale() as f32;
        let reach = self.dp(ALIGN_DIP) as f32 / s;
        let mut t = raw;
        if dg.snap && dg.grid > 0.0 {
            t = [snapv(bb[0] + raw[0], dg.grid) - bb[0], snapv(bb[1] + raw[1], dg.grid) - bb[1]];
        }
        let mv = [bb[0] + raw[0], bb[1] + raw[1], bb[2] + raw[0], bb[3] + raw[1]];
        let page = dg.vp.world_rect();
        // per axis: (priority: 0 guide, 1 shape; distance; correction; target value; target box)
        let mut best: [Option<(u8, f32, f32, f32, [f32; 4])>; 2] = [None, None];
        let mut offer = |axis: usize, prio: u8, d: f32, v: f32, b: [f32; 4]| {
            if d.abs() <= reach && best[axis].is_none_or(|x| (prio, d.abs()) < (x.0, x.1)) {
                best[axis] = Some((prio, d.abs(), d, v, b));
            }
        };
        let guides: &[SeGuide] = if st.snap && st.snap_guides { m.guides } else { &[] };
        for axis in 0..2 {
            let ours = [mv[axis], (mv[axis] + mv[axis + 2]) * 0.5, mv[axis + 2]];
            for gd in guides.iter().filter(|g| g.vertical == (axis == 0)) {
                let b = if axis == 0 { [gd.at, page[1], gd.at, page[3]] } else { [page[0], gd.at, page[2], gd.at] };
                for &x in &ours {
                    offer(axis, 0, gd.at - x, gd.at, b);
                }
            }
            if st.snap && st.dyn_grid {
                for sh in m.shapes {
                    if sh.parent != 0 || keys.contains(&sh.key) || sh.flags & (SEF_1D | SEF_NOSNAP) != 0 || !overlaps(sh.bbox, page) {
                        continue;
                    }
                    let b = align_box(sh);
                    for &tv in &[b[axis], (b[axis] + b[axis + 2]) * 0.5, b[axis + 2]] {
                        for &x in &ours {
                            offer(axis, 1, tv - x, tv, b);
                        }
                    }
                }
            }
        }
        for axis in 0..2 {
            if let Some((_, _, d, v, b)) = best[axis] {
                t[axis] = raw[axis] + d;
                let o = 1 - axis;
                let (lo, hi) = (mv[o].min(b[o]), mv[o + 2].max(b[o + 2]));
                let (a, z) = if axis == 0 { ([v, lo], [v, hi]) } else { ([lo, v], [hi, v]) };
                st.aligns.push(Align { a, b: z });
            }
        }
        t
    }

    // the keys of a focused canvas
    fn se_keys(&mut self, st: &mut SeState, dg: &mut DiagramState, m: &SeModel, out: &mut Vec<SeEv>) {
        let r = dg.vp.rect;
        // Esc: the gesture, else the selection
        if let Some((i, _)) = self.dg_key_find(VK_ESCAPE, 0, 0) {
            let live = st.live();
            let g = core::mem::take(&mut st.g);
            let handled = match g {
                G::Idle => {
                    let any = !st.sel.is_empty();
                    if any {
                        st.sel.clear();
                        st.primary = 0;
                        st.vertex = None;
                        out.push(SeEv::Select);
                    }
                    st.g = G::Idle;
                    any
                }
                G::Marquee { base, .. } | G::Lasso { base, .. } => {
                    if st.sel != base {
                        st.sel = base;
                        out.push(SeEv::Select);
                    }
                    st.g = G::Hold;
                    true
                }
                _ => {
                    if live {
                        out.push(SeEv::Cancel);
                    }
                    st.g = G::Hold;
                    true
                }
            };
            if handled {
                self.dg_key_take(i);
            }
        }
        if !matches!(st.g, G::Idle) {
            return;
        }
        if let Some((i, _)) = self.dg_key_find(VK_DELETE, 0, MOD_CTRL | MOD_ALT) {
            if let Some((k, tag)) = st.vertex {
                self.dg_key_take(i);
                out.push(SeEv::PointDelete { key: k, tag });
                st.vertex = None;
            } else if !st.sel.is_empty() {
                self.dg_key_take(i);
                out.push(SeEv::Delete);
            }
        }
        if let Some((i, _)) = self.dg_key_find(VK_A, MOD_CTRL, MOD_CTRL | MOD_ALT) {
            self.dg_key_take(i);
            let all: BTreeSet<u64> = m.shapes.iter().filter(|s| s.parent == 0 && s.flags & SEF_NOSELECT == 0).map(|s| s.key).collect();
            if all != st.sel {
                st.primary = m.shapes.iter().find(|s| all.contains(&s.key)).map_or(0, |s| s.key);
                st.sel = all;
                out.push(SeEv::Select);
            }
        }
        // arrows: nudge the selection by a grid step (Shift: a pixel), or pan
        for (vk, dx, dy) in [(VK_LEFT, -1.0f32, 0.0f32), (VK_RIGHT, 1.0, 0.0), (VK_UP, 0.0, -1.0), (VK_DOWN, 0.0, 1.0)] {
            let Some((i, km)) = self.dg_key_find(vk, 0, MOD_CTRL | MOD_ALT) else { continue };
            self.dg_key_take(i);
            let keys = st.movable(m);
            let sc = dg.vp.scale();
            if keys.is_empty() {
                dg.vp.cx += dx as f64 * r[2] as f64 / 8.0 / sc;
                dg.vp.cy += dy as f64 * r[3] as f64 / 8.0 / sc;
                continue;
            }
            let k = if km & MOD_SHIFT != 0 { 1.0 / sc as f32 } else if dg.grid > 0.0 { dg.grid } else { 8.0 };
            out.push(SeEv::Move { keys, total: [dx * k, dy * k], done: true });
        }
        // Tab / Shift+Tab: the next top-level shape (after the last one the focus leaves);
        // without a selection the first (Shift: the last) one, wherever the focus was (KB-06)
        if let Some((i, km)) = self.dg_key_find(VK_TAB, 0, MOD_CTRL | MOD_ALT) {
            let keys: Vec<u64> = m.shapes.iter().filter(|s| s.parent == 0 && s.flags & SEF_NOSELECT == 0).map(|s| s.key).collect();
            let pos = if st.sel.is_empty() { None } else { keys.iter().position(|&k| k == st.focus) };
            let next = match (pos, km & MOD_SHIFT != 0) {
                (None, false) => keys.first().copied(),
                (None, true) => keys.last().copied(),
                (Some(p), false) => keys.get(p + 1).copied(),
                (Some(p), true) => p.checked_sub(1).and_then(|p| keys.get(p).copied()),
            };
            if let Some(k) = next {
                self.dg_key_take(i);
                self.wp.ctx.focus_visible = true;
                st.focus = k;
                if st.sel.len() != 1 || !st.sel.contains(&k) {
                    st.select(&[k]);
                    out.push(SeEv::Select);
                }
                // show it
                if let Some(sh) = st.shape(m, k) {
                    if !inside(sh.bbox, dg.vp.world_rect()) {
                        dg.vp.cx = ((sh.bbox[0] + sh.bbox[2]) * 0.5) as f64;
                        dg.vp.cy = ((sh.bbox[1] + sh.bbox[3]) * 0.5) as f64;
                    }
                }
            }
        }
        let single = st.single(m).map(|s| s.key);
        // F2 / Enter: the text of the single selection
        for (vk, need) in [(VK_F2, 0), (VK_RETURN, 0)] {
            if let (Some((i, _)), Some(k)) = (self.dg_key_find(vk, need, MOD_CTRL | MOD_ALT | MOD_SHIFT), single) {
                self.dg_key_take(i);
                out.push(SeEv::Text { key: k, ch: None });
            }
        }
        // (the menu key and Shift+F10 stay for ui.context_menu after this call: it opens the
        // menu the app picks for the selection)
        // typing starts the text of the single selection with the character
        if let Some(k) = single {
            if self.mods & (MOD_CTRL | MOD_ALT) == 0 && !self.wp.ctx.textq.is_empty() {
                let typed = self.text_take(1);
                if let Some(c) = typed.first().and_then(|&c| char::from_u32(c)).filter(|c| !c.is_control()) {
                    out.push(SeEv::Text { key: k, ch: Some(c) });
                }
            }
        }
    }

    // ---- drawing

    fn se_overlay(&mut self, st: &SeState, dg: &DiagramState, m: &SeModel, tool: u32) {
        let vp = dg.vp;
        let s = vp.scale() as f32;
        let wr = vp.world_rect();
        let accent = self.color(T_ACCENT);
        let bg = 0xFFFF_FFFF;
        let hair = self.hair().max(1);
        let hairf = hair as f32;
        let rot_d = self.dp(ROT_DIP) as f32 / s;
        // guides
        let guide = 0xFF3D8BFD;
        for gd in m.guides {
            let (a, b) = if gd.vertical { ([gd.at, wr[1]], [gd.at, wr[3]]) } else { ([wr[0], gd.at], [wr[2], gd.at]) };
            self.dg_dashed(vp.to_screen(a), vp.to_screen(b), hairf, guide);
        }
        // selection outlines
        let anchor_t = (hairf * 2.0).max(2.0);
        for &k in &st.sel {
            let Some(sh) = st.shape(m, k) else { continue };
            if !overlaps(sh.bbox, wr) {
                continue;
            }
            let t = if k == st.primary && st.sel.len() > 1 { anchor_t } else { hairf };
            if sh.flags & SEF_1D != 0 {
                let pts = [sh.ends[0], sh.ends[1]];
                if st.sel.len() > 1 {
                    self.dg_poly(&vp, &pts, t, (accent & 0x00FF_FFFF) | 0x8000_0000);
                }
            } else {
                let b = &sh.bx;
                let c = [b.at(0.0, 0.0), b.at(1.0, 0.0), b.at(1.0, 1.0), b.at(0.0, 1.0)];
                for e in 0..4 {
                    self.dg_dashed(vp.to_screen(c[e]), vp.to_screen(c[(e + 1) % 4]), t, accent);
                }
            }
        }
        // connection arrows and insertion marks (under the handles): half transparent, the one
        // under the pointer opaque
        if tool == SE_POINTER {
            let (soft, full) = ((accent & 0x00FF_FFFF) | 0xA000_0000, accent | 0xFF00_0000);
            if let Some(a) = (st.arrow_key != 0).then(|| self.se_arrow_set(st, dg, m, st.arrow_key)).flatten() {
                for d in 0..4 {
                    self.se_tri(&a.tri(d), if st.arrow_hot == d as u8 { full } else { soft });
                }
            }
            for (k, mk) in m.marks.iter().enumerate() {
                if self.se_mark_shown(st, dg, m, mk) {
                    let hot = st.mark_hot.is_some_and(|h| h.0 == k as u32);
                    let t = self.se_mark_tri(dg, mk);
                    self.se_tri(&t, if hot { full } else { soft });
                }
            }
        }
        // handles of a single selection
        if let Some(sh) = st.single(m) {
            let hs = self.dp(HANDLE_DIP);
            let locked = self.color(T_SUBTLE_HOVER) | 0xFF00_0000;
            if sh.flags & SEF_1D != 0 {
                for e in 0..2usize {
                    let glued = sh.flags & if e == 0 { SEF_BEGIN_GLUED | SEF_BEGIN_DYN } else { SEF_END_GLUED | SEF_END_DYN } != 0;
                    let dynamic = sh.flags & if e == 0 { SEF_BEGIN_DYN } else { SEF_END_DYN } != 0;
                    let fill = if glued { 0xFF2E_9E44 } else { bg };
                    let h = if dynamic { hs + self.dp(1) } else { hs };
                    let p = vp.to_screen(sh.ends[e]);
                    let (x, y) = (fl(p[0] + 0.5) - h, fl(p[1] + 0.5) - h);
                    self.cmd_rrect(x, y, h * 2, h * 2, 0, hair, fill, accent, accent);
                    if e == 1 {
                        // the end: a dot in the middle tells it from the begin
                        let c = if glued { bg } else { accent };
                        self.dg_dot(p, self.dp(1), 0, c, c);
                    }
                }
            } else if matches!(tool, SE_POINTER | SE_RECT | SE_ELLIPSE | SE_TEXT) {
                let b = sh.bx;
                // the rotation handle
                if sh.flags & SEF_LOCK_ROTATE == 0 {
                    let rp = rot_point(&b, rot_d);
                    self.dg_poly(&vp, &[b.at(0.5, 0.0), rp], hairf, accent);
                    let hot = st.hover == SHit::Rotate(sh.key);
                    self.dg_dot(vp.to_screen(rp), hs + if hot { hair } else { 0 }, hair, bg, accent);
                }
                // the pin (once the rotation handle was touched)
                if st.pin_shown == sh.key && sh.flags & (SEF_LOCK_PIN | SEF_LOCK_ROTATE) == 0 {
                    let p = vp.to_screen(sh.pin);
                    self.dg_dot(p, self.dp(3), hair, bg, accent);
                    self.dg_dot(p, self.dp(1), 0, accent, accent);
                }
                for (h, hp) in HANDLES.iter().enumerate() {
                    let p = vp.to_screen(b.at(hp[0], hp[1]));
                    let free = handle_free(sh.flags, h as u8);
                    let hot = st.hover == SHit::Handle(sh.key, h as u8) && free;
                    let e = hs + if hot { hair } else { 0 };
                    let (x, y) = (fl(p[0] + 0.5) - e, fl(p[1] + 0.5) - e);
                    self.cmd_rrect(x, y, e * 2, e * 2, 0, hair, if free { bg } else { locked }, accent, accent);
                }
            }
            // control points (yellow diamonds as small squares turned) and vertices
            for k in range_of(&sh.points, m.points.len()) {
                let q = &m.points[k];
                let p = vp.to_screen(q.at);
                match q.kind {
                    SP_CONTROL => {
                        let e = hs as f32 + if st.hover == SHit::Point(sh.key, k as u32) { hairf } else { 0.0 };
                        self.se_diamond(p, e, 0xFFFF_D21F, 0xFF5A_4A00, hairf);
                    }
                    SP_VERTEX => {
                        let chosen = st.vertex == Some((sh.key, q.tag));
                        let e = self.dp(3) + if st.hover == SHit::Point(sh.key, k as u32) { hair } else { 0 };
                        let (x, y) = (fl(p[0] + 0.5) - e, fl(p[1] + 0.5) - e);
                        self.cmd_rrect(x, y, e * 2, e * 2, 0, hair, if chosen { accent } else { bg }, accent, accent);
                    }
                    _ => {}
                }
            }
        } else if let (Some(b), SE_POINTER) = (st.multi_box(m), tool) {
            // the handles of the box around a multiple selection
            let hs = self.dp(HANDLE_DIP);
            let c = [b.at(0.0, 0.0), b.at(1.0, 0.0), b.at(1.0, 1.0), b.at(0.0, 1.0)];
            for e in 0..4 {
                self.dg_dashed(vp.to_screen(c[e]), vp.to_screen(c[(e + 1) % 4]), hairf, (accent & 0x00FF_FFFF) | 0x9000_0000);
            }
            let rp = rot_point(&b, rot_d);
            self.dg_poly(&vp, &[b.at(0.5, 0.0), rp], hairf, accent);
            self.dg_dot(vp.to_screen(rp), hs, hair, bg, accent);
            for (h, hp) in HANDLES.iter().enumerate() {
                let p = vp.to_screen(b.at(hp[0], hp[1]));
                let hot = st.hover == SHit::SelHandle(h as u8);
                let e = hs + if hot { hair } else { 0 };
                let (x, y) = (fl(p[0] + 0.5) - e, fl(p[1] + 0.5) - e);
                self.cmd_rrect(x, y, e * 2, e * 2, 0, hair, bg, accent, accent);
            }
        }
        // connection points: while connecting or moving an end, with the connector tool (the
        // shapes under the pointer and glued to), or always those of every visible shape (RA-06)
        let connecting = matches!(st.g, G::Connect { .. } | G::End { .. } | G::Stroke { .. } | G::Arc { .. }) || matches!(st.g, G::Point { glues: true, .. }) || matches!(tool, SE_CONNECT | SE_LINE | SE_FREEFORM | SE_ARC);
        if connecting && st.glue || st.show_points {
            let mut keys: Vec<u64> = Vec::new();
            if st.show_points {
                keys.extend(m.shapes.iter().filter(|s| !s.points.is_empty() && overlaps(s.bbox, wr)).map(|s| s.key));
            }
            if st.hover_shape != 0 {
                keys.push(st.hover_shape);
            }
            if let SeGlue::Point { key, .. } | SeGlue::Shape(key) = st.glue_now {
                keys.push(key);
            }
            if let G::Connect { ga: SeGlue::Point { key, .. } | SeGlue::Shape(key), .. } = st.g {
                keys.push(key);
            }
            keys.sort_unstable();
            keys.dedup();
            let ok = 0xFF2E_9E44;
            for k in keys {
                let Some(sh) = st.shape(m, k) else { continue };
                if sh.flags & SEF_NOGLUE != 0 {
                    continue;
                }
                // dynamic glue: the whole shape lights up
                if st.glue_now == SeGlue::Shape(k) {
                    let b = &sh.bx;
                    let c = [b.at(0.0, 0.0), b.at(1.0, 0.0), b.at(1.0, 1.0), b.at(0.0, 1.0)];
                    self.dg_poly(&vp, &[c[0], c[1], c[2], c[3], c[0]], (hairf * 3.0).max(3.0), (ok & 0x00FF_FFFF) | 0xA000_0000);
                }
                for j in range_of(&sh.points, m.points.len()) {
                    let q = &m.points[j];
                    if q.kind != SP_CONN {
                        continue;
                    }
                    let p = vp.to_screen(q.at);
                    let target = st.glue_now == SeGlue::Point { key: k, tag: q.tag };
                    if target {
                        let e = self.dp(5);
                        let (x, y) = (fl(p[0] + 0.5) - e, fl(p[1] + 0.5) - e);
                        self.cmd_rrect(x, y, e * 2, e * 2, 0, hair * 2, 0, ok, ok);
                    }
                    let e = self.dp(3) as f32;
                    self.draw_color(if target { ok } else { 0xFF3D_6FB5 });
                    let t = fx(hairf.max(1.0) * 1.5);
                    self.draw_line(fx(p[0] - e), fx(p[1] - e), fx(p[0] + e), fx(p[1] + e), t);
                    self.draw_line(fx(p[0] - e), fx(p[1] + e), fx(p[0] + e), fx(p[1] - e), t);
                }
            }
        }
        // alignment lines of the dynamic grid
        for al in &st.aligns {
            self.dg_dashed(vp.to_screen(al.a), vp.to_screen(al.b), hairf, 0xFFE0_4F8A);
        }
        // rubber bands
        let band = (accent & 0x00FF_FFFF) | 0xC000_0000;
        match st.g {
            G::Marquee { start, cur, .. } | G::Zoom { start, cur } => {
                let a = vp.to_screen(start);
                let b = vp.to_screen(cur);
                let q = ipx([a[0].min(b[0]), a[1].min(b[1]), (a[0] - b[0]).abs(), (a[1] - b[1]).abs()]);
                self.cmd_rrect(q[0], q[1], q[2], q[3], 0, hair, (accent & 0x00FF_FFFF) | 0x1800_0000, band, band);
            }
            G::Connect { a, b, .. } => {
                self.dg_poly(&vp, &[a, b], (hairf * 1.5).max(1.5), band);
            }
            G::Lasso { ref pts, .. } => {
                let mut way = pts.clone();
                if let Some(&first) = pts.first() {
                    way.push(first);
                }
                self.dg_poly(&vp, &way, hairf, band);
            }
            G::Stroke { ref pts, .. } => {
                self.dg_poly(&vp, pts, (hairf * 1.5).max(1.5), band);
            }
            G::Arc { a, b, h, .. } => {
                let pts = arc_points(a, b, h.unwrap_or(true));
                self.dg_poly(&vp, &pts, (hairf * 1.5).max(1.5), band);
            }
            G::Draw { tool, a, b } => {
                let r = [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])];
                if tool == SE_ELLIPSE {
                    let (cx, cy, rx, ry) = ((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5, (r[2] - r[0]) * 0.5, (r[3] - r[1]) * 0.5);
                    let mut pts = Vec::with_capacity(41);
                    for k in 0..=40 {
                        let (cs, sn) = rt::x87::sincos(k as f64 * core::f64::consts::TAU / 40.0);
                        pts.push([cx + rx * cs as f32, cy + ry * sn as f32]);
                    }
                    self.dg_poly(&vp, &pts, hairf, band);
                } else {
                    let c = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]], [r[0], r[1]]];
                    self.dg_poly(&vp, &c, hairf, band);
                }
            }
            _ => {}
        }
    }

    // a filled triangle (canvas px) in colour c
    fn se_tri(&mut self, t: &[[f32; 2]; 3], c: u32) {
        if !t.iter().all(|p| p[0].is_finite() && p[1].is_finite()) {
            return;
        }
        let b = tri_box(t);
        let pts = t.map(|p| [p[0] - b[0] as f32, p[1] - b[1] as f32]);
        self.cmd_custom(b[0], b[1], b[2], b[3], Some(Rc::new(Tri { pts, c })), 0, 0);
    }

    // a diamond (a square turned by 45 degrees) of half-diagonal e px at p
    fn se_diamond(&mut self, p: [f32; 2], e: f32, fill: u32, stroke: u32, t: f32) {
        // filled with lines across, then the outline
        self.draw_color(fill);
        let n = (e as i32).max(1);
        for k in -n..=n {
            let dy = k as f32;
            let dx = e - dy.abs();
            self.draw_line(fx(p[0] - dx), fx(p[1] + dy), fx(p[0] + dx), fx(p[1] + dy), fx(1.2));
        }
        self.draw_color(stroke);
        let c = [[p[0], p[1] - e], [p[0] + e, p[1]], [p[0], p[1] + e], [p[0] - e, p[1]]];
        for k in 0..4 {
            let (a, b) = (c[k], c[(k + 1) % 4]);
            self.draw_line(fx(a[0]), fx(a[1]), fx(b[0]), fx(b[1]), fx(t));
        }
    }

    /// ui_shape_edit_sems(id, st, dg, m, label): one K_LISTITEM record per visible top-level
    /// shape (label(key); value = its index in the model; selected / focused states) and one
    /// K_BUTTON record per handle of a single selection; a screen reader's click on a shape
    /// selects it
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn shape_edit_sems(&mut self, id: u32, st: &mut SeState, dg: &DiagramState, m: &SeModel, label: &dyn Fn(u64) -> String) {
        let _src = self.src_scope();
        let r = dg.vp.rect;
        if r[2] <= 0 || r[3] <= 0 {
            return;
        }
        let vp = dg.vp;
        let wr = vp.world_rect();
        let focused = self.wp.ctx.focus == id;
        let clip = |q: [i32; 4]| -> Option<[i32; 4]> {
            let (x0, y0) = (q[0].max(r[0]), q[1].max(r[1]));
            let (x1, y1) = ((q[0] + q[2]).min(r[0] + r[2]), (q[1] + q[3]).min(r[1] + r[3]));
            (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
        };
        for (i, sh) in m.shapes.iter().enumerate() {
            if sh.parent != 0 && !st.sel.contains(&sh.key) || sh.flags & SEF_NOSELECT != 0 || !overlaps(sh.bbox, wr) {
                continue;
            }
            let b = sh.bbox;
            let q = ipx(vp.rect_to_screen([b[0], b[1], (b[2] - b[0]).max(0.5), (b[3] - b[1]).max(0.5)]));
            let Some(mut rect) = clip(q) else { continue };
            if self.rtl {
                rect[0] = self.rtl_map(rect[0] + rect[2] - 1, r[0], r[2]);
            }
            let mut state = 0;
            if st.sel.contains(&sh.key) {
                state |= SS_SELECTED;
            }
            if focused && st.focus == sh.key {
                state |= SS_FOCUSED;
            }
            let sid = node_sem_id(id, sh.key);
            if self.m.a11y.pending() && self.a11y_click(sid) {
                st.select(&[sh.key]);
                st.pending_sel = true;
                self.repass();
            }
            self.last_set(rect, state as u32);
            let l = label(sh.key);
            self.sem(K_LISTITEM, sid, Some(&l), i as i32);
        }
        // the connection arrows and the insertion marks while they show (a screen reader's
        // click: the click's event with the next call of ui.shape_edit)
        let mut extra: Vec<([[f32; 2]; 3], u32, &'static str, i32, SeEv)> = Vec::new();
        if let Some(a) = (st.arrow_key != 0).then(|| self.se_arrow_set(st, dg, m, st.arrow_key)).flatten() {
            let k = st.arrow_key;
            for d in 0..4usize {
                let sid = node_sem_id(id ^ 0xA770, k.wrapping_mul(4).wrapping_add(d as u64));
                extra.push((a.tri(d), sid, ARROW_NAMES[d], 0, SeEv::Arrow { key: k, dir: d as u8 }));
            }
        }
        for mk in m.marks {
            if self.se_mark_shown(st, dg, m, mk) {
                let sid = node_sem_id(id ^ 0x4D4B, mk.owner.rotate_left(32) ^ mk.tag as u64);
                extra.push((self.se_mark_tri(dg, mk), sid, MARK_NAME, mk.tag as i32, SeEv::Mark { tag: mk.tag }));
            }
        }
        for (t, sid, name, value, ev) in extra {
            let Some(mut rect) = clip(tri_box(&t)) else { continue };
            if self.rtl {
                rect[0] = self.rtl_map(rect[0] + rect[2] - 1, r[0], r[2]);
            }
            if self.m.a11y.pending() && self.a11y_click(sid) {
                st.pend.push(ev);
                self.repass();
            }
            self.last_set(rect, 0);
            self.sem(K_BUTTON, sid, Some(crate::i18n::tr(name)), value);
        }
        // the handles of a single selection
        let Some(sh) = st.single(m) else { return };
        let s = vp.scale() as f32;
        let half = self.dp(HANDLE_DIP + 1);
        let mut recs: Vec<(String, [f32; 2], u32)> = Vec::new();
        if sh.flags & SEF_1D != 0 {
            recs.push((String::from(crate::i18n::tr("Anfangspunkt")), sh.ends[0], 1));
            recs.push((String::from(crate::i18n::tr("Endpunkt")), sh.ends[1], 2));
        } else {
            for (h, hp) in HANDLES.iter().enumerate() {
                if handle_free(sh.flags, h as u8) {
                    recs.push((String::from(crate::i18n::tr(HANDLE_NAMES[h])), sh.bx.at(hp[0], hp[1]), 3 + h as u32));
                }
            }
            if sh.flags & SEF_LOCK_ROTATE == 0 {
                recs.push((String::from(crate::i18n::tr("Drehgriff")), rot_point(&sh.bx, self.dp(ROT_DIP) as f32 / s), 11));
            }
        }
        let mut nc = 0;
        for k in range_of(&sh.points, m.points.len()) {
            let q = &m.points[k];
            let name = match q.kind {
                SP_CONTROL => crate::i18n::tr("Steuerpunkt"),
                SP_VERTEX => crate::i18n::tr("Scheitelpunkt"),
                _ => continue,
            };
            nc += 1;
            recs.push((format!("{name} {nc}"), q.at, 16 + k as u32));
        }
        for (name, at, n) in recs {
            let p = vp.to_screen(at);
            let (cx, cy) = (fl(p[0] + 0.5), fl(p[1] + 0.5));
            let Some(mut rect) = clip([cx - half, cy - half, half * 2, half * 2]) else { continue };
            if self.rtl {
                rect[0] = self.rtl_map(rect[0] + rect[2] - 1, r[0], r[2]);
            }
            let sid = node_sem_id(id ^ 0x5E5E, sh.key.wrapping_mul(64).wrapping_add(n as u64));
            self.last_set(rect, 0);
            self.sem(K_BUTTON, sid, Some(&name), 0);
        }
    }
}

fn range_of(r: &Range<u32>, n: usize) -> Range<usize> {
    let a = (r.start as usize).min(n);
    let b = (r.end as usize).min(n).max(a);
    a..b
}
