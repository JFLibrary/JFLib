// diagram -- a node / edge canvas: zoom and pan, grid and snapping, a spatial index, hit
// testing, selection (click, marquee, keyboard), moving, connecting, waypoints, a minimap and
// one semantic record per visible node.  The app owns the model and paints it; the widget
// keeps the view, the selection and the gestures in DiagramState and reports what the user
// did as DEv events.
//
//   ui.diagram(id, &mut st, &m, tool, paint, ver) -> Vec<DEv>
//                                   the canvas in the next layout slot (size it with
//                                   next_size); tool DG_SELECT / DG_PAN / DG_CONNECT /
//                                   DG_PLACE; paint draws the content at render time
//                                   (DgPaint); ver stands for everything it shows besides the
//                                   geometry (labels, states, colours)
//   ui.diagram_minimap(id, &mut st, &m) -> bool    an overview in the bottom right corner of
//                                   the canvas; pressing / dragging there moves the view
//   ui.diagram_sems(id, &mut st, &m, &label)   one K_LISTITEM record per visible node
//                                   (scripts: click listitem:"R1"; screen readers), no hit box
//   Viewport::{scale, to_screen, to_world, world_rect, rect_to_screen, zoom_at, step, fit}
//   dg_paint_grid(g, vp, step, minor, major)   grid lines inside the clip (render time)
//   dg_edge_points(m, i, &mut out)  the route of edge i (world polyline) without a state
//   dg_stroke_edge(g, vp, pts, width, argb, cap0, cap1, dash)   stroke a route (gfx::dash
//                                   caps LCAP_* and patterns DASH_*), culled by the clip
//   dg_point_at(pts, f) -> point    the point at fraction f of a route's length (packets)
//   dg_hairline(g, vp, pts, argb)   a route as 1 px anti-aliased lines: far cheaper than
//                                   dg_stroke_edge for many thin edges (DgFrame::moving)
//   DgFrame::find(margin, edges)    the nodes or edges near the clip (the spatial buckets)
//
// Model (DModel, borrowed every frame): nodes with a world rectangle and a range of ports,
// ports at a position relative to their node's top left corner, edges from key a to key b
// (a port, or a node: then the route starts on its border) with a route DR_STRAIGHT /
// DR_BEZIER / DR_ORTHO through a range of waypoints.  Keys are non-zero and unique over
// nodes, ports and edges; later nodes lie on top.  ver changes whenever the geometry
// changes: the key maps, the routes and the spatial buckets (256 world units) are rebuilt
// then.  World units are DIP at zoom 1.
//
// Input: the hit test prefers the handles of selected edges (waypoints, then the middles of
// their stretches), then the ports of the node under the pointer, the topmost node, the
// nearest edge (max(5 DIP, half the edge width + 3 DIP)), the background.  A press + drag
// (4 DIP) on a selected node moves the selection (snapped to the grid, Alt: free), from a
// port connects, on a handle moves or inserts a waypoint, on the background draws a marquee
// (Shift adds, Ctrl toggles); middle drag, Space + drag and DG_PAN pan; the wheel scrolls
// (Shift: sideways), Ctrl + wheel zooms at the pointer.  Keys while the canvas has the focus:
// arrows nudge the selection (grid steps, Shift x10, Alt 1) or pan, Del, Ctrl+A, Esc cancels
// a gesture (else clears the selection), Tab / Shift+Tab walk the nodes (and leave after the
// last), Enter activates, Ctrl + plus / minus zoom, Ctrl+0 fits, Shift+F10 / the menu key
// ask for the context menu.  DiagramState::test_keys stands in for held keys in scripts.
// Drawing: the paint runs as one custom command whose version covers ver, the model version,
// the view, the theme and the gesture flag; selection, hover, ports, handles, marquee and rubber
// band are recorded commands on top, so hovering redraws only their cells.  The paint draws the
// whole canvas into a backing store (an XRGB surface of the DiagramState) once per version;
// every dirty rectangle copies its part out of it.  Skip what lies outside DgFrame::clip
// (DgFrame::find asks the spatial buckets).  While a gesture moves the view or nodes (a pan,
// wheel turns until MOVING_MS after the last one, a drag of nodes or a waypoint)
// DgFrame::moving asks for less detail; a full paint follows the gesture.  Content that moves
// every frame (packets) is best drawn after diagram() with ui.draw_* inside
// ui.clip_push(st.vp.rect).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::ops::Range;

use crate::cmd::{Draw, DrawRef, Gfx};
use crate::consts::*;
use crate::keys::*;
use crate::layout::dp_at;
use crate::theme::{color_mix, THEME_N};
use crate::Ui;
use gfx::dash::{dash_pattern, dash_stroke, DStyle, LCAP_FLAT};
use gfx::{Surface, FMT_XRGB};
use rt::fx::{floor_i, sqrt};

/// tools of ui.diagram
pub const DG_SELECT: u32 = 0; // select, move, connect from ports, waypoints
pub const DG_PAN: u32 = 1; // a left drag pans
pub const DG_CONNECT: u32 = 2; // a drag (or two clicks) from a node / port to another connects
pub const DG_PLACE: u32 = 3; // a click reports DEv::Click (snapped) to place something
/// edge routes (DEdge::route)
pub const DR_STRAIGHT: u8 = 0;
pub const DR_BEZIER: u8 = 1;
pub const DR_ORTHO: u8 = 2; // horizontal and vertical stretches (L / Z shapes)
/// node flags (DNode::flags)
pub const DN_LOCKED: u32 = 1; // never moved by the widget
pub const DN_NOSELECT: u32 = 2; // decoration: never hit nor selected
/// port flags (DPort::flags)
pub const DPT_HIDDEN: u32 = 1; // neither shown nor hit
pub const DPT_USED: u32 = 2; // shown filled (connected)

const CELL: f32 = 256.0; // bucket size (world units)
const CELLS_MAX: i64 = 4096; // a query over more buckets looks at everything
const SPAN_MAX: i64 = 64; // an item over more buckets goes to the always-checked list
const DRAG_DIP: i32 = 4;
const STUB: f32 = 16.0; // orthogonal routes leave a port straight for this many units
const EPS: f32 = 0.01;
/// a view gesture of wheel turns (or the minimap) lasts this long after the last one (ms)
const MOVING_MS: u32 = 160;
const ZOOMS: [f32; 16] = [0.1, 0.125, 0.167, 0.25, 0.333, 0.5, 0.667, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0];
// sides of a port / anchor
const S_NONE: u8 = 0;
const S_L: u8 = 1;
const S_R: u8 = 2;
const S_T: u8 = 3;
const S_B: u8 = 4;

// ------------------------------------------------------------------ view

/// what part of the world the canvas shows: zoom (DIP per world unit, 1 = 100 %), the world
/// point at the centre of rect; rect (absolute px) and dpi are kept by ui.diagram
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub zoom: f32,
    pub cx: f64,
    pub cy: f64,
    pub rect: [i32; 4],
    pub dpi: i32,
}

impl Default for Viewport {
    fn default() -> Viewport {
        Viewport { zoom: 1.0, cx: 0.0, cy: 0.0, rect: [0; 4], dpi: 96 }
    }
}

impl Viewport {
    pub const ZOOM_MIN: f32 = 0.1;
    pub const ZOOM_MAX: f32 = 8.0;

    /// px per world unit
    pub fn scale(&self) -> f64 {
        clampz(self.zoom) as f64 * self.dpi.max(1) as f64 / 96.0
    }

    fn mid(&self) -> (f64, f64) {
        (self.rect[0] as f64 + self.rect[2] as f64 * 0.5, self.rect[1] as f64 + self.rect[3] as f64 * 0.5)
    }

    fn world64(&self, x: f64, y: f64) -> (f64, f64) {
        let s = self.scale();
        let (mx, my) = self.mid();
        ((x - mx) / s + self.cx, (y - my) / s + self.cy)
    }

    /// a world point in absolute px
    pub fn to_screen(&self, p: [f32; 2]) -> [f32; 2] {
        let s = self.scale();
        let (mx, my) = self.mid();
        [(mx + (p[0] as f64 - self.cx) * s) as f32, (my + (p[1] as f64 - self.cy) * s) as f32]
    }

    /// an absolute px position in world units
    pub fn to_world(&self, x: f32, y: f32) -> [f32; 2] {
        let (wx, wy) = self.world64(x as f64, y as f64);
        [wx as f32, wy as f32]
    }

    /// the world rectangle x0, y0, x1, y1 the canvas shows
    pub fn world_rect(&self) -> [f32; 4] {
        let r = self.rect;
        let a = self.to_world(r[0] as f32, r[1] as f32);
        let b = self.to_world((r[0] + r[2]) as f32, (r[1] + r[3]) as f32);
        [a[0], a[1], b[0], b[1]]
    }

    /// a world rectangle x, y, w, h in px (x, y, w, h, fractional)
    pub fn rect_to_screen(&self, r: [f32; 4]) -> [f32; 4] {
        let a = self.to_screen([r[0], r[1]]);
        let s = self.scale() as f32;
        [a[0], a[1], r[2] * s, r[3] * s]
    }

    /// zoom (clamped) keeping the world point under the absolute px (sx, sy) in place
    pub fn zoom_at(&mut self, zoom: f32, sx: f32, sy: f32) {
        let (wx, wy) = self.world64(sx as f64, sy as f64);
        self.zoom = clampz(zoom);
        let s = self.scale();
        let (mx, my) = self.mid();
        self.cx = wx - (sx as f64 - mx) / s;
        self.cy = wy - (sy as f64 - my) / s;
    }

    /// the next zoom step above (dir > 0) or below around the absolute px (sx, sy)
    pub fn step(&mut self, dir: i32, sx: f32, sy: f32) {
        let z = self.zoom;
        let n = if dir > 0 {
            ZOOMS.iter().copied().find(|&p| p > z * 1.001)
        } else {
            ZOOMS.iter().rev().copied().find(|&p| p < z * 0.999)
        };
        if let Some(n) = n {
            self.zoom_at(n, sx, sy);
        }
    }

    /// show the world rectangle b (x0, y0, x1, y1) with `margin` DIP around it, at most 100 %
    pub fn fit(&mut self, b: [f32; 4], margin: i32) {
        let m = dp_at(margin, self.dpi) as f64 * 2.0;
        let (w, h) = ((b[2] - b[0]) as f64, (b[3] - b[1]) as f64);
        let (vw, vh) = (self.rect[2] as f64 - m, self.rect[3] as f64 - m);
        let d = self.dpi.max(1) as f64 / 96.0;
        let mut z = 1.0f64;
        if w > 0.0 && vw > 0.0 {
            z = z.min(vw / (w * d));
        }
        if h > 0.0 && vh > 0.0 {
            z = z.min(vh / (h * d));
        }
        self.zoom = clampz(z as f32);
        self.cx = (b[0] as f64 + b[2] as f64) * 0.5;
        self.cy = (b[1] as f64 + b[3] as f64) * 0.5;
    }
}

fn clampz(z: f32) -> f32 {
    if z > 0.0 { z.clamp(Viewport::ZOOM_MIN, Viewport::ZOOM_MAX) } else { 1.0 }
}

// ------------------------------------------------------------------ model and events

/// a node: world rectangle x, y, w, h, its ports (a range of DModel::ports), DN_* flags
#[derive(Clone, Debug, Default)]
pub struct DNode {
    pub key: u64,
    pub rect: [f32; 4],
    pub ports: Range<u32>,
    pub flags: u32,
}

/// a port: its position relative to the node's top left corner, DPT_* flags
#[derive(Clone, Copy, Debug, Default)]
pub struct DPort {
    pub key: u64,
    pub at: [f32; 2],
    pub flags: u32,
}

/// an edge from key a to key b (ports or nodes) along route DR_* through a range of
/// DModel::wps
#[derive(Clone, Debug, Default)]
pub struct DEdge {
    pub key: u64,
    pub a: u64,
    pub b: u64,
    pub route: u8,
    pub wps: Range<u32>,
}

/// the app's diagram, borrowed for one call; ver changes with every geometry change
#[derive(Clone, Copy)]
pub struct DModel<'a> {
    pub nodes: &'a [DNode],
    pub ports: &'a [DPort],
    pub edges: &'a [DEdge],
    pub wps: &'a [[f32; 2]],
    pub ver: u32,
}

/// what a point of the canvas shows
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DHit {
    #[default]
    None,
    Node(u64),
    Port(u64),
    /// an edge: seg = the stretch between its defining points seg and seg + 1 (0 = the
    /// start, then the waypoints), t = 0..1 along it
    Edge { key: u64, seg: u32, t: f32 },
    /// the handle of waypoint idx of a selected edge
    Waypoint(u64, u32),
    /// the handle in the middle of stretch idx of a selected edge (a drag inserts waypoint idx)
    Mid(u64, u32),
}

impl DHit {
    /// the node, port or edge key (0 = the background)
    pub fn key(&self) -> u64 {
        match *self {
            DHit::None => 0,
            DHit::Node(k) | DHit::Port(k) | DHit::Waypoint(k, _) | DHit::Mid(k, _) => k,
            DHit::Edge { key, .. } => key,
        }
    }
}

// the same thing (a double click needs both presses on it)
fn same(a: DHit, b: DHit) -> bool {
    core::mem::discriminant(&a) == core::mem::discriminant(&b) && a.key() == b.key()
}

/// what the user did (ui.diagram's result, in order)
#[derive(Clone, Debug, PartialEq)]
pub enum DEv {
    /// st.sel changed
    Select,
    /// move nodes `keys` by d (world units); the d of one gesture add up to total; done = the
    /// gesture ended (released, a key nudge; Esc sends d = minus everything so far)
    Move { keys: Vec<u64>, d: [f32; 2], total: [f32; 2], done: bool },
    /// connect from -> to (port or node keys); quick = dragged out of a port with DG_SELECT
    Connect { from: u64, to: u64, quick: bool },
    /// a connection is being drawn: the pointer at `at`, over port / node `over` (0 = none)
    ConnectDrag { from: u64, at: [f32; 2], over: u64 },
    /// the connection ended without a target (released elsewhere, Esc)
    ConnectCancel,
    /// waypoint idx of edge goes to at; insert = add it at idx first (the first event of a
    /// drag from a middle handle); done = released
    Waypoint { edge: u64, idx: u32, at: [f32; 2], insert: bool, done: bool },
    /// remove waypoint idx of edge (double click on its handle, Esc after an insert)
    WaypointDelete { edge: u64, idx: u32 },
    /// double click on a node / port / edge, Enter on the focused node
    Activate(DHit),
    /// right click, Shift+F10 or the menu key: at = window px (logical, for popups)
    Context(DHit, [i32; 2]),
    /// Del with a selection: delete st.sel
    Delete,
    /// a click on the background, or anywhere with DG_PLACE (snapped then); world units
    Click { at: [f32; 2] },
    /// zoom or centre changed
    ViewChanged,
}

// ------------------------------------------------------------------ geometry for painting

/// an edge's route in world units: the polyline, the indices of its defining points (a, the
/// waypoints, b) in it, its bounding box x0, y0, x1, y1 (empty pts: an end is unknown)
#[derive(Clone, Debug, Default)]
pub struct DgRoute {
    pub key: u64,
    pub pts: Vec<[f32; 2]>,
    pub marks: Vec<u32>,
    pub bb: [f32; 4],
}

/// the routes of every edge in model order and the spatial buckets of nodes and edges, built
/// once per model version (DgFrame::find asks the buckets)
#[derive(Default)]
pub struct DgGeo {
    pub routes: Vec<DgRoute>,
    // (row, column) -> (nodes, edges); what spans too many buckets; the numbers of nodes and
    // edges
    cells: BTreeMap<(i32, i32), (Vec<u32>, Vec<u32>)>,
    big: (Vec<u32>, Vec<u32>),
    counts: (usize, usize),
}

/// what a DgPaint gets at render time
pub struct DgFrame<'a> {
    pub vp: Viewport,
    /// the world rectangle x0, y0, x1, y1 that needs pixels now: skip what lies outside
    pub clip: [f32; 4],
    pub geo: &'a DgGeo,
    /// the theme's colours (color(T_*)), the canvas background (opaque)
    pub theme: &'a [u32],
    pub bg: u32,
    /// font and size (24.8 px at zoom 1) of every TS_* text style
    pub fonts: [(u32, i32); 7],
    pub dark: bool,
    /// a gesture moves the view or nodes (a pan, wheel turns, a drag of nodes or a waypoint):
    /// leave out what costs and nobody reads in passing (small labels; plain shapes,
    /// dg_hairline); the canvas is painted in full after it
    pub moving: bool,
}

impl DgFrame<'_> {
    /// 0xAARRGGBB of a theme token
    pub fn color(&self, tok: u32) -> u32 {
        self.theme.get(((tok & 252) / 4) as usize).copied().unwrap_or(0xFF00_0000)
    }

    /// the world rectangle x, y, w, h (with 2 px for anti-aliasing) touches the clip
    pub fn visible(&self, r: [f32; 4]) -> bool {
        let g = 2.0 / self.vp.scale() as f32;
        overlaps([r[0] - g, r[1] - g, r[0] + r[2] + g, r[1] + r[3] + g], self.clip)
    }

    /// a route stroked `width` world units wide (dg_stroke_edge, arrow heads included)
    /// touches the clip
    pub fn route_visible(&self, rt: &DgRoute, width: f32) -> bool {
        let s = self.vp.scale() as f32;
        let g = ((width * s).max(1.0) * 4.0 + 6.0) / s;
        rt.pts.len() >= 2 && overlaps([rt.bb[0] - g, rt.bb[1] - g, rt.bb[2] + g, rt.bb[3] + g], self.clip)
    }

    /// the nodes (edges = false: DModel::nodes) or the edges whose spatial buckets touch the
    /// clip grown by `margin` px: ascending indices, a superset of what can need paint (cheaper
    /// than looking at all of them)
    pub fn find(&self, margin: f32, edges: bool) -> Vec<u32> {
        let g = margin / (self.vp.scale() as f32).max(1e-6);
        let c = self.clip;
        let mut v = Vec::new();
        self.geo.find([c[0] - g, c[1] - g, c[2] + g, c[3] + g], edges, &mut v);
        v
    }

    /// font and size (24.8 px) of text style ts at the current zoom
    pub fn font(&self, ts: u32) -> (u32, i32) {
        let (f, s) = self.fonts[((ts & 7) as usize).min(6)];
        (f, (s as f32 * clampz(self.vp.zoom)) as i32)
    }
}

/// the app's painting of a diagram (render time): nodes, edges (DgFrame::geo), labels; it
/// runs once per dirty rectangle, so skip what DgFrame::visible / route_visible reject
pub trait DgPaint {
    fn paint(&self, g: &mut Gfx, f: &DgFrame);
}

// ------------------------------------------------------------------ state

#[derive(Clone, Default)]
enum Gesture {
    #[default]
    Idle,
    /// a left press below the drag threshold: what it hit, where (px, world), the modifiers,
    /// whether the hit was selected before
    Press { hit: DHit, px: [i32; 2], at: [f32; 2], mods: u32, was: bool },
    /// ignore the pointer until the left button is up
    Hold,
    Pan { btn: u32, px: [i32; 2], cx: f64, cy: f64 },
    Move { keys: Vec<u64>, start: [f32; 2], anchor: [f32; 2], sent: [f32; 2] },
    Marquee { start: [f32; 2], cur: [f32; 2], mods: u32, base: BTreeSet<u64> },
    /// armed: started by a click, a second click ends it
    Connect { from: u64, quick: bool, armed: bool, at: [f32; 2], over: u64 },
    Waypoint { edge: u64, idx: u32, orig: [f32; 2], insert: bool, started: bool, at: [f32; 2] },
}

// the key maps, the routes and the spatial buckets of one model version
#[derive(Default)]
struct Index {
    built: bool,
    ver: u32,
    counts: [usize; 4],
    nodes: BTreeMap<u64, u32>,
    ports: BTreeMap<u64, (u32, u32)>,
    edges: BTreeMap<u64, u32>,
    geo: Rc<DgGeo>,
    bounds: Option<[f32; 4]>,
}

/// the widget's state, kept by the app: the view, the selection (node and edge keys), the
/// focused node, grid and snapping, the hover, the gesture in progress and the index
pub struct DiagramState {
    pub vp: Viewport,
    pub sel: BTreeSet<u64>,
    /// the node with the keyboard focus (0 = none): Tab walks, Enter activates it
    pub focus: u64,
    /// grid step in world units (0 = no grid)
    pub grid: f32,
    /// moves, waypoints and DG_PLACE clicks snap to the grid (Alt held: free)
    pub snap: bool,
    /// what the pointer is over (after the last diagram call)
    pub hover: DHit,
    /// the stroke width of the edges in world units (hit testing, the highlight)
    pub edge_width: f32,
    /// scripts cannot hold keys: 1 = as if Space were held, 2 = Ctrl
    pub test_keys: u32,
    fit_req: bool,
    pending_sel: bool,
    g: Gesture,
    ix: Index,
    last_hit: DHit,
    last_ms: u32,
    last_px: [i32; 2],
    visible: Vec<u32>,
    paint_obj: Option<(u64, DrawRef)>,
    mini_obj: Option<(u64, DrawRef)>,
    mini_ext: Option<[f32; 4]>,
    /// the canvas as painted last (Wrap)
    store: Rc<RefCell<Backing>>,
    /// the view moves until this ms (wheel turns, the minimap)
    moving_until: u32,
}

impl Default for DiagramState {
    fn default() -> DiagramState {
        DiagramState::new()
    }
}

impl DiagramState {
    /// grid 16, snapping on, edges 2 units wide; the first diagram call fits the content
    pub fn new() -> DiagramState {
        DiagramState {
            vp: Viewport::default(),
            sel: BTreeSet::new(),
            focus: 0,
            grid: 16.0,
            snap: true,
            hover: DHit::None,
            edge_width: 2.0,
            test_keys: 0,
            fit_req: true,
            pending_sel: false,
            g: Gesture::Idle,
            ix: Index::default(),
            last_hit: DHit::None,
            last_ms: 0,
            last_px: [0; 2],
            visible: Vec::new(),
            paint_obj: None,
            mini_obj: None,
            mini_ext: None,
            store: Rc::new(RefCell::new(Backing::default())),
            moving_until: 0,
        }
    }

    /// fit the whole content into the view with the next diagram call
    pub fn request_fit(&mut self) {
        self.fit_req = true;
    }

    /// a gesture (drag, marquee, connection, pan) is in progress
    pub fn busy(&self) -> bool {
        !matches!(self.g, Gesture::Idle | Gesture::Hold)
    }

    /// the route of edge `key` (as of the last diagram call)
    pub fn route(&self, key: u64) -> Option<&DgRoute> {
        let i = *self.ix.edges.get(&key)?;
        self.ix.geo.routes.get(i as usize)
    }

    /// every route (model order), shared with the paint
    pub fn geo(&self) -> &Rc<DgGeo> {
        &self.ix.geo
    }

    /// the bounding box x0, y0, x1, y1 of nodes and routes (None: empty)
    pub fn bounds(&self) -> Option<[f32; 4]> {
        self.ix.bounds
    }

    /// the indices of the nodes the canvas showed in the last diagram call
    pub fn visible(&self) -> &[u32] {
        &self.visible
    }

    fn snaps(&self, mods: u32) -> bool {
        self.snap && self.grid > 0.0 && mods & MOD_ALT == 0
    }

    fn snap_pt(&self, p: [f32; 2], mods: u32) -> [f32; 2] {
        if self.snaps(mods) { [snapv(p[0], self.grid), snapv(p[1], self.grid)] } else { p }
    }

    // the node index of a node or port key
    fn node_index(&self, key: u64) -> Option<u32> {
        self.ix.ports.get(&key).map(|p| p.0).or_else(|| self.ix.nodes.get(&key).copied())
    }

    fn node<'m>(&self, m: &DModel<'m>, key: u64) -> Option<&'m DNode> {
        let i = *self.ix.nodes.get(&key)?;
        m.nodes.get(i as usize)
    }

    // forget selected / focused keys the model no longer has
    fn prune(&mut self) -> bool {
        let ix = &self.ix;
        let n = self.sel.len();
        self.sel.retain(|k| ix.nodes.contains_key(k) || ix.edges.contains_key(k));
        if self.focus != 0 && !ix.nodes.contains_key(&self.focus) {
            self.focus = 0;
        }
        n != self.sel.len()
    }

    // ---- hit testing

    fn hit(&self, m: &DModel, p: [f32; 2], tool: u32, dpi: i32) -> DHit {
        let s = self.vp.scale() as f32;
        if tool == DG_PAN || !(s > 0.0) {
            return DHit::None;
        }
        let w = |dip: i32| dp_at(dip, dpi) as f32 / s; // DIP -> world units
        // the handles of selected edges: waypoints first, then the middles
        if tool == DG_SELECT {
            let hr = w(6);
            let mut best: Option<(f32, DHit)> = None;
            for pass in 0..2 {
                for &k in &self.sel {
                    let Some(&ei) = self.ix.edges.get(&k) else { continue };
                    let (Some(e), Some(rt)) = (m.edges.get(ei as usize), self.ix.geo.routes.get(ei as usize)) else { continue };
                    if pass == 0 {
                        for (j, q) in m.wps[range_of(&e.wps, m.wps.len())].iter().enumerate() {
                            let d = dist(p, *q);
                            if d <= hr && best.is_none_or(|b| d < b.0) {
                                best = Some((d, DHit::Waypoint(k, j as u32)));
                            }
                        }
                    } else {
                        for j in 0..rt.marks.len().saturating_sub(1) {
                            let Some(q) = mid_of(rt, j) else { continue };
                            let d = dist(p, q);
                            if d <= hr && best.is_none_or(|b| d < b.0) {
                                best = Some((d, DHit::Mid(k, j as u32)));
                            }
                        }
                    }
                }
                if let Some((_, h)) = best {
                    return h;
                }
            }
        }
        // the ports of the node under the pointer, the topmost node
        let pr = w(6);
        let mut cand = Vec::new();
        self.ix.query([p[0] - pr, p[1] - pr, p[0] + pr, p[1] + pr], false, m.nodes.len(), &mut cand);
        for &i in cand.iter().rev() {
            let Some(n) = m.nodes.get(i as usize) else { continue };
            if n.flags & DN_NOSELECT != 0 {
                continue;
            }
            let r = n.rect;
            let within = |g: f32| p[0] >= r[0] - g && p[1] >= r[1] - g && p[0] < r[0] + r[2] + g && p[1] < r[1] + r[3] + g;
            if !within(pr) {
                continue;
            }
            if tool != DG_PLACE {
                let mut best: Option<(f32, u64)> = None;
                for q in &m.ports[range_of(&n.ports, m.ports.len())] {
                    if q.flags & DPT_HIDDEN != 0 {
                        continue;
                    }
                    let d = dist(p, [r[0] + q.at[0], r[1] + q.at[1]]);
                    if d <= pr && best.is_none_or(|b| d < b.0) {
                        best = Some((d, q.key));
                    }
                }
                if let Some((_, k)) = best {
                    return DHit::Port(k);
                }
            }
            if within(0.0) {
                return DHit::Node(n.key);
            }
        }
        // the nearest edge
        if tool == DG_SELECT {
            let tol = w(5).max((self.edge_width * s * 0.5 + dp_at(3, dpi) as f32) / s);
            self.ix.query([p[0] - tol, p[1] - tol, p[0] + tol, p[1] + tol], true, m.edges.len(), &mut cand);
            let mut best: Option<(f32, usize, usize, f32)> = None;
            for &i in &cand {
                let Some(rt) = self.ix.geo.routes.get(i as usize) else { continue };
                if rt.pts.len() < 2 || !overlaps(rt.bb, [p[0] - tol, p[1] - tol, p[0] + tol, p[1] + tol]) {
                    continue;
                }
                let (d2, j, u) = near_route(rt, p);
                if d2 <= tol * tol && best.is_none_or(|b| d2 <= b.0) {
                    best = Some((d2, i as usize, j, u));
                }
            }
            if let Some((_, i, j, u)) = best {
                let rt = &self.ix.geo.routes[i];
                let (seg, t) = seg_t(rt, j, u);
                return DHit::Edge { key: rt.key, seg, t };
            }
        }
        DHit::None
    }

    fn cursor(&self, m: &DModel, tool: u32, space: bool) -> u32 {
        match self.g {
            Gesture::Pan { .. } => return CUR_HAND,
            Gesture::Move { .. } | Gesture::Waypoint { .. } => return CUR_MOVE,
            Gesture::Connect { .. } => return CUR_CROSS,
            _ => {}
        }
        if space || tool == DG_PAN {
            return CUR_HAND;
        }
        match (tool, self.hover) {
            (DG_PLACE, _) => CUR_CROSS,
            (_, DHit::Port(_)) => CUR_CROSS,
            (DG_CONNECT, DHit::Node(_)) => CUR_CROSS,
            (_, DHit::Waypoint(..) | DHit::Mid(..)) => CUR_MOVE,
            (_, DHit::Node(k)) if self.node(m, k).is_some_and(|n| n.flags & DN_LOCKED == 0) => CUR_MOVE,
            _ => CUR_ARROW,
        }
    }

    // ---- gestures

    fn wheel(&mut self, wx: i32, wy: i32, mods: u32, at: [i32; 2], notch: i32) {
        if wy != 0 && mods & MOD_CTRL != 0 {
            let n = wy.unsigned_abs().div_ceil(120).min(16);
            for _ in 0..n {
                self.vp.step(if wy > 0 { 1 } else { -1 }, at[0] as f32 + 0.5, at[1] as f32 + 0.5);
            }
            return;
        }
        // wheel up shows the upper part (Shift: the left part)
        let k = notch as f64 / 120.0 / self.vp.scale();
        let (dx, dy) = if mods & MOD_SHIFT != 0 { (wx - wy, 0) } else { (wx, -wy) };
        self.vp.cx += dx as f64 * k;
        self.vp.cy += dy as f64 * k;
    }

    fn step(&mut self, m: &DModel, i: &In, out: &mut Vec<DEv>) {
        let g = core::mem::take(&mut self.g);
        self.g = match g {
            Gesture::Idle => self.idle(i, out),
            Gesture::Press { hit, px, at, mods, was } => {
                let far = (i.px[0] - px[0]).abs() > i.lim || (i.px[1] - px[1]).abs() > i.lim;
                if i.btn & (1 << BTN_LEFT) == 0 {
                    self.click(m, i.tool, hit, at, mods, was, out)
                } else if far {
                    self.last_hit = DHit::None; // a drag is no first click
                    self.drag(m, i.tool, hit, at, mods)
                } else {
                    Gesture::Press { hit, px, at, mods, was }
                }
            }
            g => g,
        };
        let g = core::mem::take(&mut self.g);
        self.g = self.follow(m, i, g, out);
    }

    // a press with nothing in progress
    fn idle(&mut self, i: &In, out: &mut Vec<DEv>) -> Gesture {
        let hov = i.ev & EV_HOVER != 0;
        let lpress = hov && i.pressed & (1 << BTN_LEFT) != 0 && i.mine;
        let mpress = hov && i.pressed & (1 << BTN_MIDDLE) != 0;
        if mpress || (lpress && (i.space || i.tool == DG_PAN)) {
            let btn = if mpress { BTN_MIDDLE } else { BTN_LEFT };
            return Gesture::Pan { btn, px: i.px, cx: self.vp.cx, cy: self.vp.cy };
        }
        if !lpress {
            return Gesture::Idle;
        }
        // a double click: the second press on the same thing, in time and near the first
        // (behave's EV_DOUBLE also counts a press after a drag)
        let hit = self.hover;
        let near = (i.px[0] - self.last_px[0]).abs() <= i.lim * 2 && (i.px[1] - self.last_px[1]).abs() <= i.lim * 2;
        if same(hit, self.last_hit) && near && i.now.wrapping_sub(self.last_ms) <= i.dbl {
            self.last_hit = DHit::None;
            match hit {
                DHit::Waypoint(e, k) => {
                    out.push(DEv::WaypointDelete { edge: e, idx: k });
                    return Gesture::Hold;
                }
                DHit::None | DHit::Mid(..) => {}
                h => {
                    out.push(DEv::Activate(h));
                    return Gesture::Hold;
                }
            }
        } else {
            self.last_hit = hit;
            self.last_ms = i.now;
            self.last_px = i.px;
        }
        let mut was = false;
        if i.tool == DG_SELECT {
            let k = match hit {
                DHit::Node(k) => k,
                DHit::Edge { key, .. } => key,
                _ => 0,
            };
            if k != 0 {
                was = self.sel.contains(&k);
                let changed = if i.mods & MOD_CTRL != 0 {
                    if was {
                        self.sel.remove(&k);
                    } else {
                        self.sel.insert(k);
                    }
                    true
                } else if i.mods & MOD_SHIFT != 0 {
                    self.sel.insert(k)
                } else if !was {
                    self.sel.clear();
                    self.sel.insert(k);
                    true
                } else {
                    false
                };
                if changed {
                    out.push(DEv::Select);
                }
                if matches!(hit, DHit::Node(_)) {
                    self.focus = k;
                }
            }
        }
        Gesture::Press { hit, px: i.px, at: i.p, mods: i.mods, was }
    }

    // released below the drag threshold
    #[allow(clippy::too_many_arguments)]
    fn click(&mut self, m: &DModel, tool: u32, hit: DHit, at: [f32; 2], mods: u32, was: bool, out: &mut Vec<DEv>) -> Gesture {
        match tool {
            DG_PLACE => {
                out.push(DEv::Click { at: self.snap_pt(at, mods) });
                Gesture::Idle
            }
            DG_CONNECT => match hit {
                DHit::Node(k) | DHit::Port(k) => Gesture::Connect { from: k, quick: false, armed: true, at, over: 0 },
                _ => {
                    out.push(DEv::Click { at });
                    Gesture::Idle
                }
            },
            _ => {
                match hit {
                    // a click on one of several selected things selects it alone
                    DHit::Node(k) | DHit::Edge { key: k, .. } => {
                        if mods & (MOD_SHIFT | MOD_CTRL) == 0 && was && self.sel.len() > 1 {
                            self.sel.clear();
                            self.sel.insert(k);
                            out.push(DEv::Select);
                        }
                    }
                    // a click on a port selects its node
                    DHit::Port(k) => {
                        let nk = self.node_index(k).and_then(|n| m.nodes.get(n as usize)).map_or(0, |n| n.key);
                        if nk != 0 {
                            let before = self.sel.clone();
                            if mods & MOD_CTRL != 0 {
                                if !self.sel.remove(&nk) {
                                    self.sel.insert(nk);
                                }
                            } else {
                                if mods & MOD_SHIFT == 0 {
                                    self.sel.clear();
                                }
                                self.sel.insert(nk);
                            }
                            if before != self.sel {
                                out.push(DEv::Select);
                            }
                            self.focus = nk;
                        }
                    }
                    DHit::None => {
                        if mods & (MOD_SHIFT | MOD_CTRL) == 0 && !self.sel.is_empty() {
                            self.sel.clear();
                            out.push(DEv::Select);
                        }
                        out.push(DEv::Click { at });
                    }
                    _ => {}
                }
                Gesture::Idle
            }
        }
    }

    // moved past the drag threshold: the gesture the press starts
    fn drag(&mut self, m: &DModel, tool: u32, hit: DHit, at: [f32; 2], mods: u32) -> Gesture {
        match tool {
            DG_PLACE => Gesture::Hold,
            DG_CONNECT => match hit {
                DHit::Node(k) | DHit::Port(k) => Gesture::Connect { from: k, quick: false, armed: false, at, over: 0 },
                _ => Gesture::Hold,
            },
            _ => match hit {
                DHit::Node(k) if self.sel.contains(&k) => {
                    let movable = |key: u64| self.node(m, key).is_some_and(|n| n.flags & DN_LOCKED == 0);
                    let keys: Vec<u64> = self.sel.iter().copied().filter(|&s| movable(s)).collect();
                    let Some(&first) = keys.first() else { return Gesture::Hold };
                    let a = if movable(k) { k } else { first };
                    let anchor = self.node(m, a).map_or([0.0; 2], |n| [n.rect[0], n.rect[1]]);
                    Gesture::Move { keys, start: at, anchor, sent: [0.0; 2] }
                }
                DHit::Port(k) => Gesture::Connect { from: k, quick: true, armed: false, at, over: 0 },
                DHit::Waypoint(e, idx) => {
                    let orig = self.waypoint(m, e, idx).unwrap_or(at);
                    Gesture::Waypoint { edge: e, idx, orig, insert: false, started: false, at: orig }
                }
                DHit::Mid(e, idx) => Gesture::Waypoint { edge: e, idx, orig: at, insert: true, started: false, at },
                DHit::None => {
                    let base = if mods & (MOD_SHIFT | MOD_CTRL) != 0 { self.sel.clone() } else { BTreeSet::new() };
                    Gesture::Marquee { start: at, cur: at, mods, base }
                }
                _ => Gesture::Hold,
            },
        }
    }

    fn waypoint(&self, m: &DModel, edge: u64, idx: u32) -> Option<[f32; 2]> {
        let e = m.edges.get(*self.ix.edges.get(&edge)? as usize)?;
        m.wps[range_of(&e.wps, m.wps.len())].get(idx as usize).copied()
    }

    // the connection target under the pointer: a port of another node, another node
    fn target(&self, from: u64) -> u64 {
        let src = self.node_index(from);
        match self.hover {
            DHit::Port(k) if k != from && self.node_index(k) != src => k,
            DHit::Node(k) if self.node_index(k) != src => k,
            _ => 0,
        }
    }

    // a running gesture follows the pointer
    fn follow(&mut self, m: &DModel, i: &In, g: Gesture, out: &mut Vec<DEv>) -> Gesture {
        let held = |b: u32| i.btn & (1 << b) != 0;
        match g {
            Gesture::Pan { btn, px, cx, cy } => {
                if !held(btn) {
                    return Gesture::Idle;
                }
                let s = self.vp.scale();
                self.vp.cx = cx - (i.px[0] - px[0]) as f64 / s;
                self.vp.cy = cy - (i.px[1] - px[1]) as f64 / s;
                Gesture::Pan { btn, px, cx, cy }
            }
            Gesture::Hold => {
                if held(BTN_LEFT) {
                    Gesture::Hold
                } else {
                    Gesture::Idle
                }
            }
            Gesture::Move { keys, start, anchor, sent } => {
                let raw = [i.p[0] - start[0], i.p[1] - start[1]];
                let tot = if self.snaps(i.mods) {
                    [snapv(anchor[0] + raw[0], self.grid) - anchor[0], snapv(anchor[1] + raw[1], self.grid) - anchor[1]]
                } else {
                    raw
                };
                let done = !held(BTN_LEFT);
                let d = [tot[0] - sent[0], tot[1] - sent[1]];
                if d != [0.0, 0.0] || done {
                    out.push(DEv::Move { keys: keys.clone(), d, total: tot, done });
                }
                if done { Gesture::Idle } else { Gesture::Move { keys, start, anchor, sent: tot } }
            }
            Gesture::Marquee { start, mods, base, .. } => {
                let cur = i.p;
                let rr = [start[0].min(cur[0]), start[1].min(cur[1]), start[0].max(cur[0]), start[1].max(cur[1])];
                let toggle = mods & MOD_CTRL != 0;
                let mut sel = base.clone();
                let mut flip = |k: u64| {
                    if toggle && sel.contains(&k) {
                        sel.remove(&k);
                    } else {
                        sel.insert(k);
                    }
                };
                let mut v = Vec::new();
                self.ix.query(rr, false, m.nodes.len(), &mut v);
                for &n in &v {
                    if let Some(n) = m.nodes.get(n as usize) {
                        if n.flags & DN_NOSELECT == 0 && inside(xyxy(n.rect), rr) {
                            flip(n.key);
                        }
                    }
                }
                self.ix.query(rr, true, m.edges.len(), &mut v);
                for &e in &v {
                    if let Some(rt) = self.ix.geo.routes.get(e as usize) {
                        if rt.pts.len() >= 2 && inside(rt.bb, rr) {
                            flip(rt.key);
                        }
                    }
                }
                if sel != self.sel {
                    self.sel = sel;
                    out.push(DEv::Select);
                }
                if held(BTN_LEFT) { Gesture::Marquee { start, cur, mods, base } } else { Gesture::Idle }
            }
            Gesture::Connect { from, quick, armed, at, over } => {
                let o = self.target(from);
                let a = i.p;
                if a != at || o != over {
                    out.push(DEv::ConnectDrag { from, at: a, over: o });
                }
                let lpress = i.pressed & (1 << BTN_LEFT) != 0;
                let end = if armed { lpress } else { !held(BTN_LEFT) };
                if end {
                    // armed: a press outside the canvas only cancels
                    let ours = !armed || (i.mine && i.ev & EV_HOVER != 0);
                    if o != 0 && ours {
                        out.push(DEv::Connect { from, to: o, quick });
                    } else {
                        out.push(DEv::ConnectCancel);
                    }
                    return if armed { Gesture::Hold } else { Gesture::Idle };
                }
                Gesture::Connect { from, quick, armed, at: a, over: o }
            }
            Gesture::Waypoint { edge, idx, orig, insert, started, at } => {
                let a = self.snap_pt(i.p, i.mods);
                let done = !held(BTN_LEFT);
                if a != at || !started || done {
                    if started || done || insert || a != orig {
                        out.push(DEv::Waypoint { edge, idx, at: a, insert: insert && !started, done });
                    } else {
                        return Gesture::Waypoint { edge, idx, orig, insert, started, at };
                    }
                }
                if done { Gesture::Idle } else { Gesture::Waypoint { edge, idx, orig, insert, started: true, at: a } }
            }
            g => g,
        }
    }

    // Esc: take the gesture back -> handled
    fn cancel(&mut self, out: &mut Vec<DEv>) -> bool {
        let g = core::mem::take(&mut self.g);
        self.g = match g {
            Gesture::Idle => return false,
            Gesture::Press { .. } | Gesture::Hold => Gesture::Hold,
            Gesture::Pan { cx, cy, .. } => {
                self.vp.cx = cx;
                self.vp.cy = cy;
                Gesture::Idle
            }
            Gesture::Move { keys, sent, .. } => {
                if sent != [0.0, 0.0] {
                    out.push(DEv::Move { keys, d: [-sent[0], -sent[1]], total: [0.0; 2], done: true });
                }
                Gesture::Hold
            }
            Gesture::Marquee { base, .. } => {
                if self.sel != base {
                    self.sel = base;
                    out.push(DEv::Select);
                }
                Gesture::Hold
            }
            Gesture::Connect { armed, .. } => {
                out.push(DEv::ConnectCancel);
                if armed { Gesture::Idle } else { Gesture::Hold }
            }
            Gesture::Waypoint { edge, idx, orig, insert, started, .. } => {
                if started && insert {
                    out.push(DEv::WaypointDelete { edge, idx });
                } else if started {
                    out.push(DEv::Waypoint { edge, idx, at: orig, insert: false, done: true });
                }
                Gesture::Hold
            }
        };
        true
    }

    // a right click / the context key on `hit` (selects an unselected node or edge first)
    fn context(&mut self, hit: DHit, at: [i32; 2], out: &mut Vec<DEv>) {
        if let DHit::Node(k) | DHit::Edge { key: k, .. } = hit {
            if !self.sel.contains(&k) {
                self.sel.clear();
                self.sel.insert(k);
                out.push(DEv::Select);
            }
            if matches!(hit, DHit::Node(_)) {
                self.focus = k;
            }
        }
        out.push(DEv::Context(hit, at));
    }

    // the selectable node keys in model order
    fn walk_keys(&self, m: &DModel) -> Vec<u64> {
        m.nodes.iter().filter(|n| n.flags & DN_NOSELECT == 0).map(|n| n.key).collect()
    }

    // centre the view on a node that is not fully shown
    fn reveal(&mut self, m: &DModel, key: u64) {
        let Some(n) = self.node(m, key) else { return };
        let r = xyxy(n.rect);
        if !inside(r, self.vp.world_rect()) {
            self.vp.cx = (r[0] as f64 + r[2] as f64) * 0.5;
            self.vp.cy = (r[1] as f64 + r[3] as f64) * 0.5;
        }
    }

    fn movable_sel(&self, m: &DModel) -> Vec<u64> {
        self.sel.iter().copied().filter(|&k| self.node(m, k).is_some_and(|n| n.flags & DN_LOCKED == 0)).collect()
    }
}

// the input of one diagram call
struct In {
    tool: u32,
    mods: u32,
    space: bool,
    ev: u32,
    px: [i32; 2],
    p: [f32; 2],
    btn: u32,
    pressed: u32,
    mine: bool,
    lim: i32,
    now: u32,
    dbl: u32,
}

// ------------------------------------------------------------------ index and routes

fn cells_of(r: [f32; 4]) -> (i32, i32, i32, i32) {
    (fl(r[0] / CELL), fl(r[1] / CELL), fl(r[2] / CELL), fl(r[3] / CELL))
}

impl Index {
    // rebuild for another model version -> rebuilt
    fn update(&mut self, m: &DModel) -> bool {
        let counts = [m.nodes.len(), m.ports.len(), m.edges.len(), m.wps.len()];
        if self.built && self.ver == m.ver && self.counts == counts {
            return false;
        }
        self.built = true;
        self.ver = m.ver;
        self.counts = counts;
        self.nodes.clear();
        self.ports.clear();
        self.edges.clear();
        for (i, n) in m.nodes.iter().enumerate() {
            self.nodes.insert(n.key, i as u32);
            for p in range_of(&n.ports, m.ports.len()) {
                self.ports.insert(m.ports[p].key, (i as u32, p as u32));
            }
        }
        for (i, e) in m.edges.iter().enumerate() {
            self.edges.insert(e.key, i as u32);
        }
        let mut geo = DgGeo { routes: Vec::with_capacity(m.edges.len()), counts: (m.nodes.len(), m.edges.len()), ..DgGeo::default() };
        for e in m.edges {
            let mut r = DgRoute::default();
            route(m, Some(self), e, &mut r);
            geo.routes.push(r);
        }
        let mut b = None;
        for (i, n) in m.nodes.iter().enumerate() {
            let r = xyxy(n.rect);
            geo.put(r, i as u32, false);
            b = union(b, r);
        }
        for i in 0..geo.routes.len() {
            let rt = &geo.routes[i];
            if rt.pts.len() >= 2 {
                let bb = rt.bb;
                geo.put(bb, i as u32, true);
                b = union(b, bb);
            }
        }
        self.bounds = b;
        self.geo = Rc::new(geo);
        true
    }

    // the node (or edge) indices in the buckets touching r (x0, y0, x1, y1), ascending; all
    // of them when r spans too many buckets
    fn query(&self, r: [f32; 4], edge: bool, all: usize, out: &mut Vec<u32>) {
        self.geo.find_in(r, edge, all, out);
    }
}

impl DgGeo {
    fn put(&mut self, r: [f32; 4], i: u32, edge: bool) {
        let (c0, r0, c1, r1) = cells_of(r);
        let n = (c1 as i64 - c0 as i64 + 1) * (r1 as i64 - r0 as i64 + 1);
        if !(1..=SPAN_MAX).contains(&n) {
            if edge { self.big.1.push(i) } else { self.big.0.push(i) }
            return;
        }
        for y in r0..=r1 {
            for x in c0..=c1 {
                let c = self.cells.entry((y, x)).or_default();
                if edge { c.1.push(i) } else { c.0.push(i) }
            }
        }
    }

    /// the indices of the nodes (edges = false) or edges whose buckets touch r (world x0, y0,
    /// x1, y1), ascending: a superset of what overlaps it (all when r spans too many buckets)
    pub fn find(&self, r: [f32; 4], edges: bool, out: &mut Vec<u32>) {
        let all = if edges { self.counts.1 } else { self.counts.0 };
        self.find_in(r, edges, all, out);
    }

    fn find_in(&self, r: [f32; 4], edge: bool, all: usize, out: &mut Vec<u32>) {
        out.clear();
        let (c0, r0, c1, r1) = cells_of(r);
        let n = (c1 as i64 - c0 as i64 + 1) * (r1 as i64 - r0 as i64 + 1);
        if !(1..=CELLS_MAX).contains(&n) {
            out.extend(0..all as u32);
            return;
        }
        for y in r0..=r1 {
            for (_, c) in self.cells.range((y, c0)..=(y, c1)) {
                out.extend_from_slice(if edge { &c.1 } else { &c.0 });
            }
        }
        out.extend_from_slice(if edge { &self.big.1 } else { &self.big.0 });
        out.sort_unstable();
        out.dedup();
    }
}

// an end of an edge: a port (point, side, node) or a whole node (rect, node)
#[derive(Clone, Copy)]
enum Ep {
    Port([f32; 2], u8),
    Node([f32; 4]),
}

fn ep_of(m: &DModel, ix: Option<&Index>, key: u64) -> Option<Ep> {
    let (ni, pi) = match ix {
        Some(ix) => match ix.ports.get(&key) {
            Some(&(n, p)) => (n as usize, Some(p as usize)),
            None => (*ix.nodes.get(&key)? as usize, None),
        },
        None => {
            let mut f = None;
            for (i, n) in m.nodes.iter().enumerate() {
                if n.key == key {
                    f = Some((i, None));
                    break;
                }
                if let Some(p) = range_of(&n.ports, m.ports.len()).find(|&p| m.ports[p].key == key) {
                    f = Some((i, Some(p)));
                    break;
                }
            }
            f?
        }
    };
    let n = m.nodes.get(ni)?;
    match pi {
        Some(p) => {
            let q = m.ports.get(p)?;
            Some(Ep::Port([n.rect[0] + q.at[0], n.rect[1] + q.at[1]], port_side(n.rect, q.at)))
        }
        None => Some(Ep::Node(n.rect)),
    }
}

// the side of the node a port sits on (the nearest edge of the rectangle)
fn port_side(r: [f32; 4], at: [f32; 2]) -> u8 {
    let d = [fabs(at[0]), fabs(r[2] - at[0]), fabs(at[1]), fabs(r[3] - at[1])];
    let mut s = 0;
    for k in 1..4 {
        if d[k] < d[s] {
            s = k;
        }
    }
    [S_L, S_R, S_T, S_B][s]
}

fn side_vec(s: u8) -> [f32; 2] {
    match s {
        S_L => [-1.0, 0.0],
        S_R => [1.0, 0.0],
        S_T => [0.0, -1.0],
        S_B => [0.0, 1.0],
        _ => [0.0, 0.0],
    }
}

fn ep_point(e: Ep) -> [f32; 2] {
    match e {
        Ep::Port(p, _) => p,
        Ep::Node(r) => [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5],
    }
}

// where a route leaves an end towards `toward`: a port's point and side; a node's border
// (orthogonal: the middle of the side facing it)
fn anchor(e: Ep, toward: [f32; 2], ortho: bool) -> ([f32; 2], u8) {
    match e {
        Ep::Port(p, s) => (p, s),
        Ep::Node(r) => {
            let c = [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5];
            let (hw, hh) = (r[2] * 0.5, r[3] * 0.5);
            let v = [toward[0] - c[0], toward[1] - c[1]];
            if fabs(v[0]) < EPS && fabs(v[1]) < EPS {
                return (c, S_NONE);
            }
            let horiz = fabs(v[0]) * hh >= fabs(v[1]) * hw;
            let s = if horiz {
                if v[0] < 0.0 { S_L } else { S_R }
            } else if v[1] < 0.0 {
                S_T
            } else {
                S_B
            };
            if ortho {
                let d = side_vec(s);
                return ([c[0] + d[0] * hw, c[1] + d[1] * hh], s);
            }
            let kx = if fabs(v[0]) > EPS { hw / fabs(v[0]) } else { f32::MAX };
            let ky = if fabs(v[1]) > EPS { hh / fabs(v[1]) } else { f32::MAX };
            let k = kx.min(ky);
            ([c[0] + v[0] * k, c[1] + v[1] * k], s)
        }
    }
}

// the route of edge e into out (empty when an end is unknown)
fn route(m: &DModel, ix: Option<&Index>, e: &DEdge, out: &mut DgRoute) {
    out.key = e.key;
    out.pts.clear();
    out.marks.clear();
    out.bb = [0.0; 4];
    let (Some(ea), Some(eb)) = (ep_of(m, ix, e.a), ep_of(m, ix, e.b)) else { return };
    let wps = &m.wps[range_of(&e.wps, m.wps.len())];
    let ta = wps.first().copied().unwrap_or(ep_point(eb));
    let tb = wps.last().copied().unwrap_or(ep_point(ea));
    let ortho = e.route == DR_ORTHO;
    let (pa, sa) = anchor(ea, ta, ortho);
    let (pb, sb) = anchor(eb, tb, ortho);
    match e.route {
        DR_BEZIER => bezier(pa, sa, wps, pb, sb, out),
        DR_ORTHO => ortho_route(pa, sa, wps, pb, sb, out),
        _ => {
            out.pts.push(pa);
            out.pts.extend_from_slice(wps);
            out.pts.push(pb);
            out.marks.extend(0..out.pts.len() as u32);
        }
    }
    let mut bb = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in &out.pts {
        bb = [bb[0].min(p[0]), bb[1].min(p[1]), bb[2].max(p[0]), bb[3].max(p[1])];
    }
    out.bb = bb;
}

// a smooth curve through the ends and the waypoints (Catmull-Rom; the ends leave along the
// ports' sides), flattened
fn bezier(pa: [f32; 2], sa: u8, wps: &[[f32; 2]], pb: [f32; 2], sb: u8, out: &mut DgRoute) {
    let mut p = Vec::with_capacity(wps.len() + 2);
    p.push(pa);
    p.extend_from_slice(wps);
    p.push(pb);
    let n = p.len();
    // the control points of every stretch
    let tan = |i: usize| -> [f32; 2] {
        let (a, b) = (p[i.saturating_sub(1)], p[(i + 1).min(n - 1)]);
        let k = if i == 0 || i == n - 1 { 1.0 / 3.0 } else { 1.0 / 6.0 };
        [(b[0] - a[0]) * k, (b[1] - a[1]) * k]
    };
    out.pts.push(pa);
    out.marks.push(0);
    for i in 0..n - 1 {
        let (p0, p3) = (p[i], p[i + 1]);
        let reach = (dist(p0, p3) * 0.5).clamp(12.0, 200.0);
        let c1 = if i == 0 && sa != S_NONE {
            let v = side_vec(sa);
            [p0[0] + v[0] * reach, p0[1] + v[1] * reach]
        } else {
            let t = tan(i);
            [p0[0] + t[0], p0[1] + t[1]]
        };
        let c2 = if i == n - 2 && sb != S_NONE {
            let v = side_vec(sb);
            [p3[0] + v[0] * reach, p3[1] + v[1] * reach]
        } else {
            let t = tan(i + 1);
            [p3[0] - t[0], p3[1] - t[1]]
        };
        let len = dist(p0, c1) + dist(c1, c2) + dist(c2, p3);
        let steps = ((len / 6.0) as i32).clamp(4, 64);
        for s in 1..=steps {
            let t = s as f32 / steps as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            out.pts.push([
                a * p0[0] + b * c1[0] + c * c2[0] + d * p3[0],
                a * p0[1] + b * c1[1] + c * c2[1] + d * p3[1],
            ]);
        }
        out.marks.push(out.pts.len() as u32 - 1);
    }
}

const AX_H: u8 = 0;
const AX_V: u8 = 1;

fn axis_of(s: u8) -> u8 {
    if s == S_T || s == S_B { AX_V } else { AX_H }
}

// horizontal and vertical stretches: out of the ports (STUB units), L shapes through the
// waypoints (each one a corner), an L or Z shape into the far port
fn ortho_route(pa: [f32; 2], sa: u8, wps: &[[f32; 2]], pb: [f32; 2], sb: u8, out: &mut DgRoute) {
    let va = side_vec(sa);
    let vb = side_vec(sb);
    let a1 = [pa[0] + va[0] * STUB, pa[1] + va[1] * STUB];
    let b1 = [pb[0] + vb[0] * STUB, pb[1] + vb[1] * STUB];
    let mut def = Vec::with_capacity(wps.len() + 2); // the points the route passes, a1 .. b1
    def.push(a1);
    def.extend_from_slice(wps);
    def.push(b1);
    // (point, defining?) -- simplified at the end
    let mut v: Vec<([f32; 2], bool)> = Vec::with_capacity(def.len() * 3 + 2);
    v.push((pa, true));
    v.push((a1, false));
    // the first stretch leaves along the port's side unless that turns back over the stub
    let q = def[1];
    let mut ax = if sa != S_NONE {
        let back = (q[0] - a1[0]) * va[0] + (q[1] - a1[1]) * va[1] < 0.0;
        if back { axis_of(sa) ^ 1 } else { axis_of(sa) }
    } else if fabs(q[0] - a1[0]) >= fabs(q[1] - a1[1]) {
        AX_H
    } else {
        AX_V
    };
    for i in 0..def.len() - 1 {
        let (p, q) = (def[i], def[i + 1]);
        let last = i + 2 == def.len();
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        if fabs(dx) < EPS || fabs(dy) < EPS {
            ax = if fabs(dx) < EPS { AX_H } else { AX_V };
        } else if last && sb != S_NONE {
            // arrive along the far port's side, coming from outside its stub
            let mut e = axis_of(sb);
            if (p[0] - b1[0]) * vb[0] + (p[1] - b1[1]) * vb[1] < 0.0 {
                e ^= 1;
            }
            if ax != e {
                v.push((if ax == AX_H { [q[0], p[1]] } else { [p[0], q[1]] }, false));
            } else if ax == AX_H {
                let mx = (p[0] + q[0]) * 0.5;
                v.push(([mx, p[1]], false));
                v.push(([mx, q[1]], false));
            } else {
                let my = (p[1] + q[1]) * 0.5;
                v.push(([p[0], my], false));
                v.push(([q[0], my], false));
            }
        } else {
            v.push((if ax == AX_H { [q[0], p[1]] } else { [p[0], q[1]] }, false));
        }
        v.push((q, !last));
    }
    v.push((pb, true));
    // drop repeated points and corners on a straight line (defining points stay)
    let mut s: Vec<([f32; 2], bool)> = Vec::with_capacity(v.len());
    for (p, d) in v {
        if let Some(l) = s.last_mut() {
            if fabs(l.0[0] - p[0]) < EPS && fabs(l.0[1] - p[1]) < EPS {
                l.1 |= d;
                continue;
            }
        }
        if s.len() >= 2 {
            let (a, b) = (s[s.len() - 2].0, s[s.len() - 1]);
            let straight = (fabs(a[0] - b.0[0]) < EPS && fabs(b.0[0] - p[0]) < EPS) || (fabs(a[1] - b.0[1]) < EPS && fabs(b.0[1] - p[1]) < EPS);
            if straight && !b.1 {
                s.pop();
            }
        }
        s.push((p, d));
    }
    for (i, (p, d)) in s.iter().enumerate() {
        out.pts.push(*p);
        if *d || i == 0 || i + 1 == s.len() {
            out.marks.push(i as u32);
        }
    }
    out.marks.dedup();
}

// the nearest point of a route: (distance^2, segment, 0..1 along it)
fn near_route(r: &DgRoute, p: [f32; 2]) -> (f32, usize, f32) {
    let mut best = (f32::MAX, 0, 0.0);
    for j in 0..r.pts.len().saturating_sub(1) {
        let (d2, u) = seg_dist2(p, r.pts[j], r.pts[j + 1]);
        if d2 < best.0 {
            best = (d2, j, u);
        }
    }
    best
}

fn seg_dist2(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> (f32, f32) {
    let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = vx * vx + vy * vy;
    let u = if l2 > 0.0 { (((p[0] - a[0]) * vx + (p[1] - a[1]) * vy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (dx, dy) = (a[0] + vx * u - p[0], a[1] + vy * u - p[1]);
    (dx * dx + dy * dy, u)
}

// segment j at u -> (stretch between defining points, 0..1 along it)
fn seg_t(r: &DgRoute, j: usize, u: f32) -> (u32, f32) {
    let mut seg = 0;
    while seg + 2 < r.marks.len() && r.marks[seg + 1] as usize <= j {
        seg += 1;
    }
    let (a, b) = (r.marks.get(seg).copied().unwrap_or(0) as usize, r.marks.get(seg + 1).copied().unwrap_or(0) as usize);
    if b <= a || b >= r.pts.len() {
        return (seg as u32, 0.0);
    }
    let total = dg_length(&r.pts[a..=b]);
    let mut at = 0.0;
    for k in a..j.min(b) {
        at += dist(r.pts[k], r.pts[k + 1]);
    }
    if j < b {
        at += dist(r.pts[j], r.pts[j + 1]) * u;
    }
    (seg as u32, if total > 0.0 { (at / total).clamp(0.0, 1.0) } else { 0.0 })
}

// the middle of stretch i of a route
fn mid_of(r: &DgRoute, i: usize) -> Option<[f32; 2]> {
    let (a, b) = (*r.marks.get(i)? as usize, *r.marks.get(i + 1)? as usize);
    if b <= a || b >= r.pts.len() {
        return None;
    }
    Some(dg_point_at(&r.pts[a..=b], 0.5))
}

/// the length of a polyline
pub fn dg_length(pts: &[[f32; 2]]) -> f32 {
    pts.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// the point at fraction f (0..1) of a polyline's length (a packet travelling along an edge)
pub fn dg_point_at(pts: &[[f32; 2]], f: f32) -> [f32; 2] {
    let Some(&first) = pts.first() else { return [0.0; 2] };
    let total = dg_length(pts);
    if !(total > 0.0) {
        return first;
    }
    let mut left = total * f.clamp(0.0, 1.0);
    for w in pts.windows(2) {
        let l = dist(w[0], w[1]);
        if left <= l && l > 0.0 {
            let u = left / l;
            return [w[0][0] + (w[1][0] - w[0][0]) * u, w[0][1] + (w[1][1] - w[0][1]) * u];
        }
        left -= l;
    }
    pts[pts.len() - 1]
}

/// the route of edge i of m (world polyline) into out, without a DiagramState (ports and
/// nodes are looked up linearly; DiagramState::route has them indexed)
pub fn dg_edge_points(m: &DModel, i: usize, out: &mut Vec<[f32; 2]>) {
    out.clear();
    let Some(e) = m.edges.get(i) else { return };
    let mut r = DgRoute::default();
    route(m, None, e, &mut r);
    out.extend_from_slice(&r.pts);
}

// ------------------------------------------------------------------ painting helpers

/// grid lines every `step` world units (every fifth in `major`) inside the clip and the
/// canvas; steps closer than 8 px are widened five times
pub fn dg_paint_grid(g: &mut Gfx, vp: &Viewport, step: f32, minor: u32, major: u32) {
    if !(step > 0.0) {
        return;
    }
    let s = vp.scale() as f32;
    let mut st = step;
    let mut k = 0;
    while st * s < 8.0 && k < 12 {
        st *= 5.0;
        k += 1;
    }
    let r = vp.rect;
    let c = g.dc.clip;
    let (x0, y0) = (c[0].max(r[0]), c[1].max(r[1]));
    let (x1, y1) = (c[2].min(r[0] + r[2]), c[3].min(r[1] + r[3]));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let a = vp.to_world(x0 as f32, y0 as f32);
    let b = vp.to_world(x1 as f32, y1 as f32);
    let hair = ((vp.dpi + 24) / 96).max(1);
    let (i0, i1) = (fl(a[0] / st), fl(b[0] / st) + 1);
    let (j0, j1) = (fl(a[1] / st), fl(b[1] / st) + 1);
    if i1 as i64 - i0 as i64 > 4000 || j1 as i64 - j0 as i64 > 4000 {
        return;
    }
    for pass in 0..2 {
        // the minor lines first, the major ones over them
        let want = pass == 1;
        g.dc.set_color(if want { major } else { minor });
        for i in i0..=i1 {
            if (i.rem_euclid(5) == 0) != want {
                continue;
            }
            let x = fl(vp.to_screen([i as f32 * st, 0.0])[0]);
            g.dc.fill(g.sf, x, y0, hair, y1 - y0);
        }
        for j in j0..=j1 {
            if (j.rem_euclid(5) == 0) != want {
                continue;
            }
            let y = fl(vp.to_screen([0.0, j as f32 * st])[1]);
            g.dc.fill(g.sf, x0, y, x1 - x0, hair);
        }
    }
}

/// stroke a route (world points) `width` world units wide (at least 1 px) in argb, with
/// gfx::dash caps (LCAP_*: arrows at an end) and pattern DASH_*; nothing when it misses the
/// clip
#[allow(clippy::too_many_arguments)]
pub fn dg_stroke_edge(g: &mut Gfx, vp: &Viewport, pts: &[[f32; 2]], width: f32, argb: u32, cap0: u32, cap1: u32, dash: u32) {
    if pts.len() < 2 {
        return;
    }
    let w = (width * vp.scale() as f32).max(1.0);
    let mut bb = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    let path = &mut g.text.path;
    path.reset();
    path.set_origin(0, 0);
    for (i, p) in pts.iter().enumerate() {
        let q = vp.to_screen(*p);
        bb = [bb[0].min(q[0]), bb[1].min(q[1]), bb[2].max(q[0]), bb[3].max(q[1])];
        if i == 0 {
            path.move_f(q[0], q[1]);
        } else {
            path.line_f(q[0], q[1]);
        }
    }
    let m = w * 4.0 + 6.0; // half the width and an arrow head
    let c = g.dc.clip;
    if bb[2] + m < c[0] as f32 || bb[0] - m > c[2] as f32 || bb[3] + m < c[1] as f32 || bb[1] - m > c[3] as f32 {
        path.reset();
        return;
    }
    let st = DStyle { width: w as f64, pat: dash_pattern(dash), join: gfx::JOIN_ROUND, cap0, cap1, dcap: LCAP_FLAT, offset: 0.0, miter: 0.0 };
    dash_stroke(path, &st);
    g.dc.set_color(argb);
    path.fill(&mut g.text.ras, g.dc, g.sf, gfx::RULE_NONZERO);
}

// the canvas as painted last: Wrap paints the whole canvas into it once per paint hash and
// copies every dirty rectangle out of it (absolute px: the surface reaches from the window's
// corner to the canvas' far corner)
#[derive(Default)]
struct Backing {
    hash: u64,
    sf: Surface,
}

/// a route (world points) as 1 px anti-aliased lines in argb (Xiaolin Wu): a fraction of
/// dg_stroke_edge's cost for many thin edges (the level of detail of DgFrame::moving); nothing
/// outside the clip
pub fn dg_hairline(g: &mut Gfx, vp: &Viewport, pts: &[[f32; 2]], argb: u32) {
    if pts.len() < 2 || g.sf.fmt == gfx::FMT_A8 {
        return;
    }
    g.dc.set_color(argb);
    let mut a = vp.to_screen(pts[0]);
    for &q in &pts[1..] {
        let b = vp.to_screen(q);
        wu_line(g, a, b);
        a = b;
    }
}

// one anti-aliased line between two px points in the Dc's colour, inside its clip: per column
// (row when steep) the two pixels next to the line share its coverage (16.16 fixed point)
fn wu_line(g: &mut Gfx, a: [f32; 2], b: [f32; 2]) {
    let c = g.dc.clip;
    let (ox, oy) = (g.dc.ox as f32, g.dc.oy as f32);
    let (mut x0, mut y0, mut x1, mut y1) = (a[0] + ox, a[1] + oy, b[0] + ox, b[1] + oy);
    let lim = 1.0e6;
    if !(fabs(x0) < lim && fabs(y0) < lim && fabs(x1) < lim && fabs(y1) < lim) {
        return;
    }
    let steep = fabs(y1 - y0) > fabs(x1 - x0);
    if steep {
        core::mem::swap(&mut x0, &mut y0);
        core::mem::swap(&mut x1, &mut y1);
    }
    if x0 > x1 {
        core::mem::swap(&mut x0, &mut x1);
        core::mem::swap(&mut y0, &mut y1);
    }
    // the clip along the line's main axis and across it
    let (lo, hi, mlo, mhi) = if steep { (c[1], c[3], c[0], c[2]) } else { (c[0], c[2], c[1], c[3]) };
    let dx = x1 - x0;
    let grad = if dx > 1e-6 { (y1 - y0) / dx } else { 0.0 };
    // the columns whose centres lie on the line
    let i0 = fl(x0 + 0.499).max(lo);
    let i1 = fl(x1 - 0.5).min(hi - 1);
    if i1 < i0 {
        return;
    }
    // the line at column i0's centre, measured from the pixel centres across it
    let mut y = ((y0 + grad * (i0 as f32 + 0.5 - x0) - 0.5) * 65536.0) as i64;
    let step = (grad * 65536.0) as i64;
    let (pcol, stride, w) = (g.dc.pcol, g.sf.stride as i64, g.sf.w);
    let px = &mut g.sf.px;
    for i in i0..=i1 {
        let j = (y >> 16) as i32;
        let f = ((y >> 8) & 0xFF) as u32;
        for (jj, cov) in [(j, 255 - f), (j + 1, f)] {
            if jj < mlo || jj >= mhi || cov == 0 {
                continue;
            }
            let (x, yy) = if steep { (jj, i) } else { (i, jj) };
            if x < 0 || x >= w || yy < 0 {
                continue;
            }
            let k = (yy as i64 * stride + x as i64 * 4) as usize;
            let Some(p) = px.get_mut(k..k + 4) else { continue };
            let ia = 255 - gfx::div255(pcol[3] * cov);
            for ch in 0..4 {
                p[ch] = (gfx::div255(pcol[ch] * cov) + gfx::div255(p[ch] as u32 * ia)).min(255) as u8;
            }
        }
        y += step;
    }
}

// the render-time object of the canvas: background, grid, the app's paint
struct Wrap {
    paint: Option<Rc<dyn DgPaint>>,
    vp: Viewport,
    geo: Rc<DgGeo>,
    theme: [u32; THEME_N],
    fonts: [(u32, i32); 7],
    grid: f32,
    dark: bool,
    moving: bool,
    hash: u64,
    store: Rc<RefCell<Backing>>,
}

impl Draw for Wrap {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let k = g.dc.clip;
        if k[2] <= k[0] || k[3] <= k[1] {
            return;
        }
        let v = self.vp.rect;
        let (w, h) = (v[0].max(0) + v[2], v[1].max(0) + v[3]);
        let Ok(mut b) = self.store.try_borrow_mut() else {
            self.paint_all(g, r);
            return;
        };
        if b.hash != self.hash || b.sf.w != w || b.sf.h != h || b.sf.px.is_empty() {
            if b.sf.w != w || b.sf.h != h || b.sf.px.is_empty() {
                b.sf = Surface::new(w, h, FMT_XRGB);
            }
            if b.sf.px.is_empty() {
                // no memory for it: paint the dirty rectangle straight away
                b.hash = 0;
                drop(b);
                self.paint_all(g, r);
                return;
            }
            let Backing { hash, sf } = &mut *b;
            g.dc.begin(sf);
            g.dc.clip_push(v[0], v[1], v[2], v[3]);
            self.paint_all(&mut Gfx { dc: &mut *g.dc, sf, text: &mut *g.text }, v);
            g.dc.end();
            *hash = self.hash;
        }
        // a canvas the window moved (a mirrored window's island) shows the same pixels there
        g.dc.blit(g.sf, &b.sf, r[0] - v[0], r[1] - v[1]);
    }
}

impl Wrap {
    // background, grid and the app's paint in r (the clip says what needs pixels)
    fn paint_all(&self, g: &mut Gfx, r: [i32; 4]) {
        let k = g.dc.clip;
        let bg = canvas_bg(&self.theme);
        g.dc.set_color(bg);
        g.dc.fill(g.sf, r[0], r[1], r[2], r[3]);
        if self.grid > 0.0 {
            let div = self.theme[(T_DIVIDER / 4) as usize];
            let minor = over(bg, (div & 0x00FF_FFFF) | ((div >> 25) << 24));
            dg_paint_grid(g, &self.vp, self.grid, minor, over(bg, div));
        }
        if let Some(p) = &self.paint {
            let a = self.vp.to_world(k[0] as f32, k[1] as f32);
            let b = self.vp.to_world(k[2] as f32, k[3] as f32);
            let f = DgFrame { vp: self.vp, clip: [a[0], a[1], b[0], b[1]], geo: &self.geo, theme: &self.theme, bg, fonts: self.fonts, dark: self.dark, moving: self.moving };
            p.paint(g, &f);
        }
    }
}

// the overview: node rectangles (absolute px) in two colours
struct Mini {
    rects: Vec<([i32; 4], bool)>,
    node: u32,
    sel: u32,
}

impl Draw for Mini {
    fn draw(&self, g: &mut Gfx, _r: [i32; 4]) {
        for (r, s) in &self.rects {
            g.dc.set_color(if *s { self.sel } else { self.node });
            g.dc.fill(g.sf, r[0], r[1], r[2], r[3]);
        }
    }
}

fn canvas_bg(theme: &[u32]) -> u32 {
    theme[(T_BG / 4) as usize] | 0xFF00_0000
}

// c (straight alpha) over the opaque bg -> opaque
fn over(bg: u32, c: u32) -> u32 {
    color_mix(bg | 0xFF00_0000, c | 0xFF00_0000, ((c >> 24) * 256 / 255) as i32)
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xCBF2_9CE4_8422_2325)
    }
    fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
}

// ------------------------------------------------------------------ small maths

fn fabs(x: f32) -> f32 {
    if x < 0.0 { -x } else { x }
}

// floor for any float (non-finite: 0)
fn fl(x: f32) -> i32 {
    if x.is_finite() { floor_i(x.clamp(-1.0e9, 1.0e9)) } else { 0 }
}

fn snapv(v: f32, g: f32) -> f32 {
    fl(v / g + 0.5) as f32 * g
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
    sqrt(dx * dx + dy * dy)
}

// x, y, w, h -> x0, y0, x1, y1
fn xyxy(r: [f32; 4]) -> [f32; 4] {
    [r[0], r[1], r[0] + r[2], r[1] + r[3]]
}

fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

// a inside b (both x0, y0, x1, y1)
fn inside(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] >= b[0] && a[1] >= b[1] && a[2] <= b[2] && a[3] <= b[3]
}

fn union(a: Option<[f32; 4]>, b: [f32; 4]) -> Option<[f32; 4]> {
    Some(match a {
        None => b,
        Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
    })
}

fn range_of(r: &Range<u32>, len: usize) -> Range<usize> {
    let a = (r.start as usize).min(len);
    let b = (r.end as usize).min(len).max(a);
    a..b
}

// a px rectangle x, y, w, h (fractional) rounded to whole px
fn ipx(r: [f32; 4]) -> [i32; 4] {
    let (x0, y0) = (fl(r[0] + 0.5), fl(r[1] + 0.5));
    let (x1, y1) = (fl(r[0] + r[2] + 0.5), fl(r[1] + r[3] + 0.5));
    [x0, y0, (x1 - x0).max(1), (y1 - y0).max(1)]
}

// px -> 24.8
fn fx(v: f32) -> i32 {
    (v.clamp(-4.0e6, 4.0e6) * 256.0) as i32
}

// the semantic id of a node of diagram id
fn node_sem_id(id: u32, key: u64) -> u32 {
    let mut h = Fnv::new();
    h.u64(id as u64);
    h.u64(key);
    0x5000_0000 | (h.0 as u32 ^ (h.0 >> 32) as u32) & 0x0FFF_FFFF
}

// ------------------------------------------------------------------ the widget

impl Ui {
    /// ui_diagram(id, st, m, tool, paint, ver) -> what the user did: the node / edge canvas
    /// in the next layout slot (see the file header)
    pub fn diagram(&mut self, id: u32, st: &mut DiagramState, m: &DModel, tool: u32, paint: Option<Rc<dyn DgPaint>>, ver: u32) -> Vec<DEv> {
        let mut out = Vec::new();
        let _ = self.take_flags();
        let r = self.place(0, 0);
        let dpi = self.wp.rt.dpi;
        st.vp.rect = r;
        st.vp.dpi = dpi;
        st.vp.zoom = clampz(st.vp.zoom);
        if st.ix.update(m) && st.prune() {
            out.push(DEv::Select);
        }
        if st.fit_req && r[2] > 0 && r[3] > 0 {
            st.fit_req = false;
            match st.ix.bounds {
                Some(b) => st.vp.fit(b, 24),
                None => {
                    st.vp.cx = 0.0;
                    st.vp.cy = 0.0;
                }
            }
        }
        if st.pending_sel {
            st.pending_sel = false;
            out.push(DEv::Select);
        }
        let view0 = (st.vp.zoom, st.vp.cx, st.vp.cy);
        // the pointer (a canvas keeps its LTR content in a mirrored window)
        let c = &self.wp.ctx;
        let (hot, mine0) = (c.hot == id, c.active == id);
        let (mx, my) = (self.rtl_map(c.mx, r[0], r[2]), c.my);
        let inside_r = c.mx != -1 && mx >= r[0] && my >= r[1] && mx < r[0] + r[2] && my < r[1] + r[3];
        let mut mods = self.mods;
        if st.test_keys & 2 != 0 {
            mods |= MOD_CTRL;
        }
        let space = st.test_keys & 1 != 0 || self.key_down(VK_SPACE);
        let p = st.vp.to_world(mx as f32 + 0.5, my as f32 + 0.5);
        let track = (hot || mine0) && inside_r && !space && !matches!(st.g, Gesture::Pan { .. });
        st.hover = if track { st.hit(m, p, tool, dpi) } else { DHit::None };
        let cur = st.cursor(m, tool, space);
        // Enter activates; behave would take it as a click
        let enter = self.wp.ctx.focus == id && self.key_peek(VK_RETURN);
        let lim = self.dp(DRAG_DIP);
        let notch = self.dp(48);
        let ev = self.behave(id, HB_FOCUS | HB_MOVES | cur << HB_CURSOR_SHIFT);
        let state = self.wp.ctx.last_state;
        let c = &self.wp.ctx;
        let (now, dbl) = (self.now_ms, self.dbl_ms);
        let inp = In { tool, mods, space, ev, px: [mx, my], p, btn: c.mbtn, pressed: c.pressed, mine: c.active == id, lim, now, dbl };
        if ev & EV_HOVER != 0 && (c.wheel_x != 0 || c.wheel_y != 0) {
            let wx = if self.rtl { -c.wheel_x } else { c.wheel_x };
            st.wheel(wx, c.wheel_y, mods, [mx, my], notch);
            st.moving_until = now.wrapping_add(MOVING_MS);
        }
        st.step(m, &inp, &mut out);
        if ev & EV_RCLICK != 0 {
            let at = [self.wp.ctx.mx, my];
            st.context(st.hover, at, &mut out);
        }
        if self.wp.ctx.focus == id {
            self.dg_keys(st, m, enter, &mut out);
        }
        if (st.vp.zoom, st.vp.cx, st.vp.cy) != view0 {
            out.push(DEv::ViewChanged);
        }
        if !out.is_empty() {
            // the view or the selection moved: what the pointer is over now
            let p = st.vp.to_world(mx as f32 + 0.5, my as f32 + 0.5);
            st.hover = if track { st.hit(m, p, tool, dpi) } else { DHit::None };
            self.repass();
        }
        // drawing: the paint, then the overlays
        self.ltr_begin(r[0], r[2]);
        self.clip_push(r[0], r[1], r[2], r[3]);
        // a gesture that moves the view or nodes (a pan, a drag of nodes or a waypoint, wheel
        // turns until MOVING_MS after the last one) paints with less detail; a frame after it
        // paints everything again
        let wheel = (st.moving_until.wrapping_sub(self.now_ms) as i32) > 0;
        if wheel {
            self.animating = true;
        }
        let moving = wheel || matches!(st.g, Gesture::Pan { .. } | Gesture::Move { .. } | Gesture::Waypoint { .. });
        let (h, obj) = self.dg_paint_obj(st, m, paint, ver, moving);
        self.cmd_custom(r[0], r[1], r[2], r[3], Some(obj), 0, h as u32);
        let ring = self.wp.ctx.focus == id && self.wp.ctx.focus_visible;
        self.dg_overlay(st, m, tool, ring);
        self.clip_pop();
        self.ltr_end();
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = state;
        self.sem(K_CANVAS, id, None, st.sel.len() as i32);
        out
    }

    // an unconsumed key vk whose modifiers & mask == need -> its queue index and modifiers
    // (Left / Right as drawn: the canvas is not mirrored)
    fn dg_key_find(&self, vk: u32, need: u32, mask: u32) -> Option<(usize, u32)> {
        for (i, &k) in self.wp.ctx.keyq.iter().enumerate() {
            if (k as i32) < 0 {
                continue;
            }
            let mods = (k >> 16) & 0x7FFF;
            let mut v = k & 0xFFFF;
            if self.rtl && mods & (MOD_CTRL | MOD_ALT) == 0 {
                v = match v {
                    VK_LEFT => VK_RIGHT,
                    VK_RIGHT => VK_LEFT,
                    v => v,
                };
            }
            if v == vk && mods & mask == need {
                return Some((i, mods));
            }
        }
        None
    }

    fn dg_key_take(&mut self, i: usize) {
        if let Some(k) = self.wp.ctx.keyq.get_mut(i) {
            *k |= 0x8000_0000;
        }
    }

    // the keys of a focused diagram
    fn dg_keys(&mut self, st: &mut DiagramState, m: &DModel, enter: bool, out: &mut Vec<DEv>) {
        let r = st.vp.rect;
        let idle = matches!(st.g, Gesture::Idle);
        // Esc: the gesture, else the selection
        if let Some((i, _)) = self.dg_key_find(VK_ESCAPE, 0, 0) {
            let done = st.cancel(out) || {
                let any = !st.sel.is_empty();
                if any {
                    st.sel.clear();
                    out.push(DEv::Select);
                }
                any
            };
            if done {
                self.dg_key_take(i);
            }
        }
        if !idle {
            return;
        }
        if let Some((i, _)) = self.dg_key_find(VK_DELETE, 0, MOD_CTRL | MOD_ALT) {
            if !st.sel.is_empty() {
                self.dg_key_take(i);
                out.push(DEv::Delete);
            }
        }
        if let Some((i, _)) = self.dg_key_find(VK_A, MOD_CTRL, MOD_CTRL | MOD_ALT) {
            self.dg_key_take(i);
            let mut all: BTreeSet<u64> = st.walk_keys(m).into_iter().collect();
            all.extend(st.ix.geo.routes.iter().filter(|rt| rt.pts.len() >= 2).map(|rt| rt.key));
            if all != st.sel {
                st.sel = all;
                out.push(DEv::Select);
            }
        }
        // arrows: nudge the selection, or pan
        for (vk, dx, dy) in [(VK_LEFT, -1.0f32, 0.0f32), (VK_RIGHT, 1.0, 0.0), (VK_UP, 0.0, -1.0), (VK_DOWN, 0.0, 1.0)] {
            let Some((i, km)) = self.dg_key_find(vk, 0, MOD_CTRL) else { continue };
            self.dg_key_take(i);
            let keys = st.movable_sel(m);
            if keys.is_empty() {
                let s = st.vp.scale();
                st.vp.cx += dx as f64 * r[2] as f64 / 8.0 / s;
                st.vp.cy += dy as f64 * r[3] as f64 / 8.0 / s;
                continue;
            }
            let base = if st.grid > 0.0 { st.grid } else { 8.0 };
            let k = if km & MOD_ALT != 0 {
                1.0
            } else if km & MOD_SHIFT != 0 {
                base * 10.0
            } else {
                base
            };
            let d = [dx * k, dy * k];
            out.push(DEv::Move { keys, d, total: d, done: true });
        }
        // Tab / Shift+Tab: the next node (after the last one the focus leaves the canvas)
        if let Some((i, km)) = self.dg_key_find(VK_TAB, 0, MOD_CTRL | MOD_ALT) {
            let keys = st.walk_keys(m);
            let pos = keys.iter().position(|&k| k == st.focus);
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
                    st.sel.clear();
                    st.sel.insert(k);
                    out.push(DEv::Select);
                }
                st.reveal(m, k);
            }
        }
        // the thing Enter and the context key act on: the focused node, else a single selection
        let subject = if st.focus != 0 && st.ix.nodes.contains_key(&st.focus) {
            DHit::Node(st.focus)
        } else if st.sel.len() == 1 {
            let k = *st.sel.iter().next().unwrap_or(&0);
            if st.ix.nodes.contains_key(&k) { DHit::Node(k) } else { DHit::Edge { key: k, seg: 0, t: 0.5 } }
        } else {
            DHit::None
        };
        if enter && subject != DHit::None {
            out.push(DEv::Activate(subject));
        }
        let ctx = self.dg_key_find(VK_APPS, 0, 0).or_else(|| self.dg_key_find(VK_F10, MOD_SHIFT, MOD_SHIFT | MOD_CTRL | MOD_ALT));
        if let Some((i, _)) = ctx {
            self.dg_key_take(i);
            let at = match subject {
                DHit::Node(k) => st.node(m, k).map(|n| st.vp.to_screen([n.rect[0] + n.rect[2] * 0.5, n.rect[1] + n.rect[3] * 0.5])),
                DHit::Edge { key, .. } => st.route(key).map(|rt| st.vp.to_screen(dg_point_at(&rt.pts, 0.5))),
                _ => None,
            };
            let at = at.map_or([r[0] + r[2] / 2, r[1] + r[3] / 2], |q| [fl(q[0]), fl(q[1])]);
            let at = [self.rtl_map(at[0], r[0], r[2]), at[1]];
            st.context(subject, at, out);
        }
        // Ctrl + plus / minus / 0: zoom around the centre, fit
        let (cx, cy) = (r[0] as f32 + r[2] as f32 * 0.5, r[1] as f32 + r[3] as f32 * 0.5);
        for (vk, dir) in [(VK_OEM_PLUS, 1), (VK_ADD, 1), (VK_OEM_MINUS, -1), (VK_SUBTRACT, -1)] {
            if let Some((i, _)) = self.dg_key_find(vk, MOD_CTRL, MOD_CTRL | MOD_ALT) {
                self.dg_key_take(i);
                st.vp.step(dir, cx, cy);
            }
        }
        for vk in [VK_0, VK_NUMPAD0] {
            if let Some((i, _)) = self.dg_key_find(vk, MOD_CTRL, MOD_CTRL | MOD_ALT) {
                self.dg_key_take(i);
                if let Some(b) = st.ix.bounds {
                    st.vp.fit(b, 24);
                }
            }
        }
    }

    // the cached render-time object of the canvas (a new one when anything it shows changed)
    fn dg_paint_obj(&mut self, st: &mut DiagramState, m: &DModel, paint: Option<Rc<dyn DgPaint>>, ver: u32, moving: bool) -> (u64, DrawRef) {
        let mut fonts = [(0u32, 0i32); 7];
        for (ts, f) in fonts.iter_mut().enumerate() {
            self.ts_font(ts as u32);
            *f = self.text.get_font();
        }
        let vp = st.vp;
        let mut h = Fnv::new();
        h.u64(ver as u64 | (m.ver as u64) << 32);
        h.u64(Rc::as_ptr(&st.ix.geo) as usize as u64);
        h.u64(paint.as_ref().map_or(0, |p| Rc::as_ptr(p) as *const () as usize as u64));
        h.u64(vp.zoom.to_bits() as u64 | (vp.dpi as u64) << 32);
        h.u64(vp.cx.to_bits());
        h.u64(vp.cy.to_bits());
        h.u64(vp.rect[0] as u32 as u64 | (vp.rect[1] as u32 as u64) << 32);
        h.u64(vp.rect[2] as u32 as u64 | (vp.rect[3] as u32 as u64) << 32);
        h.u64(st.grid.to_bits() as u64 | (self.theme_dark as u64) << 32 | (moving as u64) << 33);
        for &c in self.theme.iter() {
            h.u64(c as u64);
        }
        for &(f, s) in &fonts {
            h.u64(f as u64 | (s as u32 as u64) << 32);
        }
        let hash = h.0;
        if let Some((k, d)) = &st.paint_obj {
            if *k == hash {
                return (hash, d.clone());
            }
        }
        let d: DrawRef = Rc::new(Wrap { paint, vp, geo: st.ix.geo.clone(), theme: self.theme, fonts, grid: st.grid, dark: self.theme_dark, moving, hash, store: st.store.clone() });
        st.paint_obj = Some((hash, d.clone()));
        (hash, d)
    }

    // a polyline (world) as recorded lines w px wide
    fn dg_poly(&mut self, vp: &Viewport, pts: &[[f32; 2]], w: f32, c: u32) {
        if pts.len() < 2 {
            return;
        }
        self.draw_color(c);
        let wfx = fx(w);
        let r = vp.rect;
        let g = w + 2.0;
        let (x0, y0, x1, y1) = (r[0] as f32 - g, r[1] as f32 - g, (r[0] + r[2]) as f32 + g, (r[1] + r[3]) as f32 + g);
        let mut a = vp.to_screen(pts[0]);
        for &q in &pts[1..] {
            let b = vp.to_screen(q);
            let off = (a[0] < x0 && b[0] < x0) || (a[0] > x1 && b[0] > x1) || (a[1] < y0 && b[1] < y0) || (a[1] > y1 && b[1] > y1);
            if !off {
                self.draw_line(fx(a[0]), fx(a[1]), fx(b[0]), fx(b[1]), wfx);
            }
            a = b;
        }
    }

    // a dashed line between two px points
    fn dg_dashed(&mut self, a: [f32; 2], b: [f32; 2], w: f32, c: u32) {
        let len = dist(a, b);
        if !(len > 0.5) {
            return;
        }
        self.draw_color(c);
        let (on, off) = (self.dp(6) as f32, self.dp(4) as f32);
        let n = ((len / (on + off)) as i32 + 1).min(400);
        for k in 0..n {
            let t0 = k as f32 * (on + off) / len;
            if t0 >= 1.0 {
                break;
            }
            let t1 = ((k as f32 * (on + off) + on) / len).min(1.0);
            let p = [a[0] + (b[0] - a[0]) * t0, a[1] + (b[1] - a[1]) * t0];
            let q = [a[0] + (b[0] - a[0]) * t1, a[1] + (b[1] - a[1]) * t1];
            self.draw_line(fx(p[0]), fx(p[1]), fx(q[0]), fx(q[1]), fx(w));
        }
    }

    // a round handle / port dot at px (x, y): radius r, fill, ring t px in stroke
    fn dg_dot(&mut self, p: [f32; 2], r: i32, t: i32, fill: u32, stroke: u32) {
        let (x, y) = (fl(p[0] + 0.5) - r, fl(p[1] + 0.5) - r);
        self.cmd_rrect(x, y, r * 2, r * 2, r, t, fill, stroke, stroke);
    }

    // selection, hover, focus, handles, ports, marquee and rubber band over the paint
    fn dg_overlay(&mut self, st: &mut DiagramState, m: &DModel, tool: u32, ring: bool) {
        let vp = st.vp;
        let s = vp.scale() as f32;
        let wr = vp.world_rect();
        let accent = self.color(T_ACCENT);
        let bg = self.color(T_BG) | 0xFF00_0000;
        let hair = self.hair();
        // the visible nodes (also for diagram_sems)
        let mut vis = core::mem::take(&mut st.visible);
        st.ix.query(wr, false, m.nodes.len(), &mut vis);
        vis.retain(|&i| m.nodes.get(i as usize).is_some_and(|n| overlaps(xyxy(n.rect), wr)));
        // selected and hovered edges
        let ew = (st.edge_width * s).max(1.0);
        let hov_edge = if let DHit::Edge { key, .. } = st.hover { key } else { 0 };
        let mut sel_edges = Vec::new();
        for &k in &st.sel {
            if let Some(&i) = st.ix.edges.get(&k) {
                sel_edges.push(i as usize);
            }
        }
        let geo = st.ix.geo.clone();
        for &i in &sel_edges {
            if let Some(rt) = geo.routes.get(i) {
                if overlaps(rt.bb, wr) {
                    self.dg_poly(&vp, &rt.pts, ew + self.dp(2) as f32, accent);
                }
            }
        }
        if hov_edge != 0 && !st.sel.contains(&hov_edge) && matches!(st.g, Gesture::Idle) {
            if let Some(rt) = st.route(hov_edge) {
                let c = self.color(T_ACCENT_HOVER);
                let pts = rt.pts.clone();
                self.dg_poly(&vp, &pts, ew, c);
            }
        }
        // nodes: selection frames, the hovered node, the focus ring
        let hov_node = match st.hover {
            DHit::Node(k) => k,
            DHit::Port(k) => st.node_index(k).and_then(|i| m.nodes.get(i as usize)).map_or(0, |n| n.key),
            _ => 0,
        };
        let (pad, t2, rad) = (self.dp(3), self.dp(2), self.dp(4));
        let (fo, fi) = (self.color(T_FOCUS_OUT), self.color(T_FOCUS_IN));
        for &i in &vis {
            let n = &m.nodes[i as usize];
            let q = ipx(vp.rect_to_screen(n.rect));
            if st.sel.contains(&n.key) {
                self.cmd_rrect(q[0] - pad, q[1] - pad, q[2] + pad * 2, q[3] + pad * 2, rad, t2, 0, accent, accent);
            } else if n.key == hov_node && matches!(st.g, Gesture::Idle | Gesture::Press { .. }) {
                let c = (accent & 0x00FF_FFFF) | 0x9000_0000;
                self.cmd_rrect(q[0] - pad, q[1] - pad, q[2] + pad * 2, q[3] + pad * 2, rad, hair, 0, c, c);
            }
            if ring && st.focus == n.key {
                let g = pad + t2 + self.dp(1);
                self.cmd_rrect(q[0] - g, q[1] - g, q[2] + g * 2, q[3] + g * 2, rad + t2, t2, 0, fo, fo);
                self.cmd_rrect(q[0] - g + t2, q[1] - g + t2, q[2] + (g - t2) * 2, q[3] + (g - t2) * 2, rad, hair, 0, fi, fi);
            }
        }
        // the canvas' own focus ring while no node has the focus
        if ring && !vis.iter().any(|&i| m.nodes[i as usize].key == st.focus) {
            let r = vp.rect;
            self.cmd_rrect(r[0], r[1], r[2], r[3], 0, t2, 0, fo, fo);
        }
        // the handles of selected edges
        if tool == DG_SELECT {
            let (hr, mr) = (self.dp(5), self.dp(4));
            for &i in &sel_edges {
                let (Some(e), Some(rt)) = (m.edges.get(i), geo.routes.get(i)) else { continue };
                if !overlaps(rt.bb, wr) {
                    continue;
                }
                for j in 0..rt.marks.len().saturating_sub(1) {
                    if let Some(q) = mid_of(rt, j) {
                        let hot = st.hover == DHit::Mid(e.key, j as u32);
                        self.dg_dot(vp.to_screen(q), if hot { mr + hair } else { mr }, hair.max(self.dp(1)), bg, accent);
                    }
                }
                for (j, q) in m.wps[range_of(&e.wps, m.wps.len())].iter().enumerate() {
                    let hot = st.hover == DHit::Waypoint(e.key, j as u32);
                    let p = vp.to_screen(*q);
                    let h = if hot { hr + hair } else { hr };
                    let (x, y) = (fl(p[0] + 0.5) - h, fl(p[1] + 0.5) - h);
                    self.cmd_rrect(x, y, h * 2, h * 2, self.dp(2), hair, accent, bg, bg);
                }
            }
        }
        // ports: of the hovered node; while connecting of the source and the target
        let mut port_nodes = Vec::new();
        let (mut hot_port, mut src_port) = (match st.hover {
            DHit::Port(k) => k,
            _ => 0,
        }, 0);
        if tool != DG_PAN && tool != DG_PLACE && hov_node != 0 {
            port_nodes.push(hov_node);
        }
        if let Gesture::Connect { from, over, .. } = st.g {
            src_port = from;
            hot_port = over;
            for k in [from, over] {
                if let Some(n) = st.node_index(k).and_then(|i| m.nodes.get(i as usize)) {
                    port_nodes.push(n.key);
                }
            }
        }
        port_nodes.sort_unstable();
        port_nodes.dedup();
        let (pr, pr2) = (self.dp(4), self.dp(6));
        let strong = self.color(T_STRONG_STROKE);
        for k in port_nodes {
            let Some(n) = st.node(m, k) else { continue };
            for q in &m.ports[range_of(&n.ports, m.ports.len())] {
                if q.flags & DPT_HIDDEN != 0 {
                    continue;
                }
                let p = vp.to_screen([n.rect[0] + q.at[0], n.rect[1] + q.at[1]]);
                if q.key == hot_port || q.key == src_port {
                    self.dg_dot(p, pr2, t2, if q.flags & DPT_USED != 0 { accent } else { bg }, accent);
                } else {
                    self.dg_dot(p, pr, hair.max(self.dp(1)), if q.flags & DPT_USED != 0 { accent } else { bg }, strong);
                }
            }
        }
        // the marquee
        if let Gesture::Marquee { start, cur, .. } = &st.g {
            let a = vp.to_screen(*start);
            let b = vp.to_screen(*cur);
            let q = ipx([a[0].min(b[0]), a[1].min(b[1]), fabs(b[0] - a[0]), fabs(b[1] - a[1])]);
            let fill = (accent & 0x00FF_FFFF) | 0x2400_0000;
            self.cmd_rrect(q[0], q[1], q[2], q[3], 0, hair, fill, accent, accent);
        }
        // the rubber band of a connection
        if let Gesture::Connect { from, at, over, .. } = st.g {
            let end = if over != 0 { ep_of(m, Some(&st.ix), over).map(|e| anchor(e, at, false).0).unwrap_or(at) } else { at };
            if let Some(e) = ep_of(m, Some(&st.ix), from) {
                let a = vp.to_screen(anchor(e, end, false).0);
                let b = vp.to_screen(end);
                self.dg_dashed(a, b, self.dp(2) as f32, accent);
                self.dg_dot(b, self.dp(3), 0, accent, 0);
            }
        }
        st.visible = vis;
    }

    /// ui_diagram_minimap(id, st, m) -> the view moved: an overview of the nodes and the
    /// shown part in the bottom right corner of the last diagram (nothing when the canvas is
    /// small); pressing or dragging there centres the view on that point
    pub fn diagram_minimap(&mut self, id: u32, st: &mut DiagramState, m: &DModel) -> bool {
        let r = st.vp.rect;
        let (w, h, mg) = (self.dp(176), self.dp(112), self.dp(12));
        if r[2] < w + mg * 2 + self.dp(160) || r[3] < h + mg * 2 + self.dp(120) {
            return false;
        }
        let mr = [r[0] + r[2] - w - mg, r[1] + r[3] - h - mg, w, h];
        let pad = self.dp(6);
        let inner = [mr[0] + pad, mr[1] + pad, mr[2] - pad * 2, mr[3] - pad * 2];
        // the world it shows: the content and the view (kept while the pointer holds it)
        let held = self.wp.ctx.active == id;
        let ext = match (held, st.mini_ext) {
            (true, Some(e)) => e,
            _ => {
                let v = st.vp.world_rect();
                let b = st.ix.bounds.map_or(v, |b| [b[0].min(v[0]), b[1].min(v[1]), b[2].max(v[2]), b[3].max(v[3])]);
                let (gx, gy) = ((b[2] - b[0]) * 0.04 + 1.0, (b[3] - b[1]) * 0.04 + 1.0);
                let e = [b[0] - gx, b[1] - gy, b[2] + gx, b[3] + gy];
                st.mini_ext = Some(e);
                e
            }
        };
        let (ew, eh) = (ext[2] - ext[0], ext[3] - ext[1]);
        let k = (inner[2] as f32 / ew).min(inner[3] as f32 / eh);
        let ox = inner[0] as f32 + (inner[2] as f32 - ew * k) * 0.5;
        let oy = inner[1] as f32 + (inner[3] as f32 - eh * k) * 0.5;
        let map = |p: [f32; 2]| [ox + (p[0] - ext[0]) * k, oy + (p[1] - ext[1]) * k];
        // input (the hit box where the mirrored window shows it)
        let hx = if self.rtl { self.rtl_map(mr[0] + mr[2] - 1, r[0], r[2]) } else { mr[0] };
        let ev = self.hit_rect(id, hx, mr[1], mr[2], mr[3], HB_MOVES | CUR_HAND << HB_CURSOR_SHIFT);
        let state = self.wp.ctx.last_state;
        let mut moved = false;
        if ev & EV_HELD != 0 && k > 0.0 {
            let c = &self.wp.ctx;
            let (mx, my) = (self.rtl_map(c.mx, r[0], r[2]), c.my);
            let wx = ext[0] as f64 + (mx as f64 + 0.5 - ox as f64) / k as f64;
            let wy = ext[1] as f64 + (my as f64 + 0.5 - oy as f64) / k as f64;
            if wx != st.vp.cx || wy != st.vp.cy {
                st.vp.cx = wx;
                st.vp.cy = wy;
                moved = true;
                st.moving_until = self.now_ms.wrapping_add(MOVING_MS);
                self.repass();
            }
        }
        // drawing
        let fill = over(self.color(T_BG), self.color(T_FLYOUT));
        let stroke = self.color(T_FLYOUT_STROKE);
        let (node_c, sel_c) = (over(fill, self.color(T_TEXT3)), self.color(T_ACCENT));
        self.ltr_begin(r[0], r[2]);
        self.cmd_rrect(mr[0], mr[1], mr[2], mr[3], self.dp(6), self.hair(), fill, stroke, stroke);
        let mut hh = Fnv::new();
        hh.u64(m.ver as u64 | (node_c as u64) << 32);
        hh.u64(sel_c as u64 | (mr[0] as u32 as u64) << 32);
        hh.u64(mr[1] as u32 as u64 | (mr[2] as u32 as u64) << 32);
        for v in ext {
            hh.u64(v.to_bits() as u64);
        }
        for &s in &st.sel {
            hh.u64(s);
        }
        let hash = hh.0;
        let obj = match &st.mini_obj {
            Some((h, d)) if *h == hash => d.clone(),
            _ => {
                let mut rects = Vec::with_capacity(m.nodes.len());
                for n in m.nodes {
                    let a = map([n.rect[0], n.rect[1]]);
                    let q = [fl(a[0]), fl(a[1]), fl(n.rect[2] * k).max(1), fl(n.rect[3] * k).max(1)];
                    rects.push((q, st.sel.contains(&n.key)));
                }
                let d: DrawRef = Rc::new(Mini { rects, node: node_c, sel: sel_c });
                st.mini_obj = Some((hash, d.clone()));
                d
            }
        };
        self.clip_push(inner[0], inner[1], inner[2], inner[3]);
        self.cmd_custom(inner[0], inner[1], inner[2], inner[3], Some(obj), 0, hash as u32);
        let v = st.vp.world_rect();
        let (a, b) = (map([v[0], v[1]]), map([v[2], v[3]]));
        let q = ipx([a[0], a[1], b[0] - a[0], b[1] - a[1]]);
        self.draw_color(sel_c);
        self.draw_frame(q[0], q[1], q[2], q[3], 0, self.dp(1).max(1));
        self.clip_pop();
        self.ltr_end();
        self.wp.ctx.last_rect = [hx, mr[1], mr[2], mr[3]];
        self.wp.ctx.last_state = state;
        self.sem(K_CANVAS, id, None, 0);
        moved
    }

    /// ui_diagram_sems(id, st, m, label): one K_LISTITEM record per node the last diagram
    /// call showed (label(key), value = the node's index, selected / focused / hover states),
    /// cut to the canvas; a screen reader's click on one selects it
    pub fn diagram_sems(&mut self, id: u32, st: &mut DiagramState, m: &DModel, label: &dyn Fn(u64) -> String) {
        let r = st.vp.rect;
        let focused = self.wp.ctx.focus == id;
        let hov = match st.hover {
            DHit::Node(k) => k,
            _ => 0,
        };
        let vis = core::mem::take(&mut st.visible);
        for &i in &vis {
            let Some(n) = m.nodes.get(i as usize) else { continue };
            let q = ipx(st.vp.rect_to_screen(n.rect));
            let (x0, y0) = (q[0].max(r[0]), q[1].max(r[1]));
            let (x1, y1) = ((q[0] + q[2]).min(r[0] + r[2]), (q[1] + q[3]).min(r[1] + r[3]));
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let mut rect = [x0, y0, x1 - x0, y1 - y0];
            if self.rtl {
                rect[0] = self.rtl_map(rect[0] + rect[2] - 1, r[0], r[2]);
            }
            let mut state = 0;
            if st.sel.contains(&n.key) {
                state |= SS_SELECTED;
            }
            if focused && st.focus == n.key {
                state |= SS_FOCUSED;
            }
            if hov == n.key {
                state |= SS_HOVER;
            }
            let sid = node_sem_id(id, n.key);
            if self.m.a11y.pending() && self.a11y_click(sid) {
                st.sel.clear();
                st.sel.insert(n.key);
                st.focus = n.key;
                st.pending_sel = true;
                self.repass();
            }
            self.last_set(rect, state as u32);
            let l = label(n.key);
            self.sem(K_LISTITEM, sid, Some(&l), i as i32);
        }
        st.visible = vis;
    }
}
