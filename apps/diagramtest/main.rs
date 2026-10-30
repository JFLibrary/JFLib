// diagramtest -- the node / edge canvas (ui/diagram.rs): 30 boxes with four ports each, edges
// with every route (straight, curved, orthogonal, with waypoints, between whole nodes), the
// four tools, adding and deleting, the minimap and a status line for tests/*.uis
//
//   build/win/diagramtest.exe --script apps/diagramtest/tests/move.uis --verify-dirty
//   build/win/diagramtest.exe --shot shot.png --theme dark
//
// Keys: nodes 1001.., ports node * 10 + side (1 left, 2 right, 3 top, 4 bottom), edges
// 1000001...  The app keeps nodes and edges (with their own waypoint lists) and flattens them
// into the DNode / DPort / DEdge arrays of the DModel after every change (Scene::rebuild,
// which also bumps the model version).  Painter draws edges and boxes at render time;
// selection, ports, handles and the rest come from the widget.
// Commands for scripts: `cmd 900 F` sets DiagramState::test_keys (1 Space held, 2 Ctrl held).
//
// --frames adds a group and an area (the frames of G1, tests/frames.uis): the frame "G1" around
// N8, N9, N14 and N15 (DN_FRAME | DN_TOGGLE | DN_STEP, key 500: hit on its title bar and border,
// a drag moves its members by whole grid steps, its button collapses it into the cluster "G1
// (4)" at their middle, which a double click opens again; edges into it end at the cluster,
// the one inside is hidden) and the area "Fläche" under N25 and N26 (DN_RESIZE | DN_OUTLINE,
// key 600: its handle resizes it).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use gfx::dash::{DASH_DASH, DASH_SOLID, LCAP_ARROW_FILLED, LCAP_FLAT};
use ui::*;

const DESC: AppDesc = AppDesc { title: "Diagramm-Test", flags: AF_RESIZABLE, w: 1100, h: 760, min_w: 640, min_h: 480, ..AppDesc::NONE };

const ID_DIAGRAM: u32 = 100;
const ID_MINI: u32 = 101;
const ID_TOOL: u32 = 110; // + DG_*
const ID_ROUTE: u32 = 120; // + DR_*
const ID_ADD: u32 = 130;
const ID_DEL: u32 = 131;
const ID_FIT: u32 = 132;
const ID_SNAP: u32 = 133;
const CMD_TEST_KEYS: u32 = 900;

const TOOLS: [&str; 4] = ["Auswahl", "Hand", "Verbinden", "Platzieren"];
const ROUTES: [&str; 3] = ["Gerade", "Kurve", "Rechtwinklig"];
const NW: f32 = 96.0; // node size
const NH: f32 = 48.0;
const NODE0: u64 = 1000;
const EDGE0: u64 = 1_000_000;
const FRAME0: u64 = 500;
const AREA0: u64 = 600;
// the frame's padding around its members and its title bar (world units)
const FRAME_PAD: f32 = 16.0;
const FRAME_BAR: f32 = 24.0;
const KIND_TOKENS: [u32; 4] = [T_ACCENT, T_OK, T_WARN, T_INFO];

struct Node {
    key: u64,
    label: String,
    rect: [f32; 4],
    kind: usize,
}

struct Edge {
    key: u64,
    a: u64,
    b: u64,
    route: u8,
    wps: Vec<[f32; 2]>,
    dashed: bool,
}

// a group of nodes: a frame around them, or collapsed one cluster node at their middle
struct Frame {
    key: u64,
    label: String,
    members: Vec<u64>,
    collapsed: bool,
    // its box as built last
    rect: [f32; 4],
}

// an area under the nodes: selectable where they are not, resized by its handle
struct Area {
    key: u64,
    label: String,
    rect: [f32; 4],
}

// the app's diagram and the flat arrays of its DModel
#[derive(Default)]
struct Scene {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    frames: Vec<Frame>,
    areas: Vec<Area>,
    // the edges drawn (an edge into a collapsed frame ends at its cluster; None: hidden)
    drawn: Vec<Option<usize>>,
    dn: Vec<DNode>,
    dp: Vec<DPort>,
    de: Vec<DEdge>,
    dw: Vec<[f32; 2]>,
    at: BTreeMap<u64, usize>, // node key -> index
    ver: u32,
    next_node: u64,
    next_edge: u64,
}

fn port_key(node: u64, side: u64) -> u64 {
    node * 10 + side
}

// floor(v + 0.5)
fn rnd(v: f64) -> i64 {
    let f = v + 0.5;
    let t = f as i64;
    if (t as f64) > f { t - 1 } else { t }
}

// the node of a node or port key
fn node_of(key: u64) -> u64 {
    if (10_000..EDGE0).contains(&key) { key / 10 } else { key }
}

impl Scene {
    fn model(&self) -> DModel<'_> {
        DModel { nodes: &self.dn, ports: &self.dp, edges: &self.de, wps: &self.dw, ver: self.ver }
    }

    // the collapsed frame that hides node key
    fn hidden_by(&self, key: u64) -> Option<usize> {
        self.frames.iter().position(|f| f.collapsed && f.members.contains(&key))
    }

    // the middle of a frame's members
    fn members_box(&self, f: &Frame) -> Option<[f32; 4]> {
        let mut b: Option<[f32; 4]> = None;
        for n in self.nodes.iter().filter(|n| f.members.contains(&n.key)) {
            let r = [n.rect[0], n.rect[1], n.rect[0] + n.rect[2], n.rect[1] + n.rect[3]];
            b = Some(match b {
                None => r,
                Some(c) => [c[0].min(r[0]), c[1].min(r[1]), c[2].max(r[2]), c[3].max(r[3])],
            });
        }
        b
    }

    // the flat model after a change (a new version)
    fn rebuild(&mut self) {
        let used: Vec<u64> = self.edges.iter().flat_map(|e| [e.a, e.b]).collect();
        self.dn.clear();
        self.dp.clear();
        self.de.clear();
        self.dw.clear();
        self.at.clear();
        // areas, then frames below the nodes they hold
        for a in &self.areas {
            self.dn.push(DNode { key: a.key, rect: a.rect, ports: 0..0, flags: DN_RESIZE | DN_OUTLINE });
        }
        for k in 0..self.frames.len() {
            let Some(b) = self.members_box(&self.frames[k]) else { continue };
            let f = &mut self.frames[k];
            if f.collapsed {
                let (cx, cy) = ((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5);
                f.rect = [cx - NW * 0.5, cy - NH * 0.5, NW, NH];
                continue;
            }
            f.rect = [b[0] - FRAME_PAD, b[1] - FRAME_PAD - FRAME_BAR, b[2] - b[0] + FRAME_PAD * 2.0, b[3] - b[1] + FRAME_PAD * 2.0 + FRAME_BAR];
            self.dn.push(DNode { key: f.key, rect: f.rect, ports: 0..0, flags: DN_FRAME | DN_TOGGLE | DN_STEP });
        }
        for (i, n) in self.nodes.iter().enumerate() {
            self.at.insert(n.key, i);
            if self.frames.iter().any(|f| f.collapsed && f.members.contains(&n.key)) {
                continue;
            }
            let p0 = self.dp.len() as u32;
            let (w, h) = (n.rect[2], n.rect[3]);
            for (side, at) in [(1, [0.0, h * 0.5]), (2, [w, h * 0.5]), (3, [w * 0.5, 0.0]), (4, [w * 0.5, h])] {
                let key = port_key(n.key, side);
                self.dp.push(DPort { key, at, flags: if used.contains(&key) { DPT_USED } else { 0 } });
            }
            self.dn.push(DNode { key: n.key, rect: n.rect, ports: p0..self.dp.len() as u32, flags: 0 });
        }
        // collapsed frames: their clusters on top
        for f in self.frames.iter().filter(|f| f.collapsed && !f.members.is_empty()) {
            self.dn.push(DNode { key: f.key, rect: f.rect, ports: 0..0, flags: DN_STEP });
        }
        // an end hidden in a collapsed frame moves to its cluster; both in it: hidden
        self.drawn.clear();
        for (i, e) in self.edges.iter().enumerate() {
            let end = |k: u64| match self.hidden_by(node_of(k)) {
                Some(f) => (self.frames[f].key, true),
                None => (k, false),
            };
            let ((a, ha), (b, hb)) = (end(e.a), end(e.b));
            if (ha || hb) && node_of(a) == node_of(b) {
                self.drawn.push(None);
                continue;
            }
            let w0 = self.dw.len() as u32;
            if !ha && !hb {
                self.dw.extend_from_slice(&e.wps);
            }
            self.de.push(DEdge { key: e.key, a, b, route: if ha || hb { DR_STRAIGHT } else { e.route }, wps: w0..self.dw.len() as u32 });
            self.drawn.push(Some(i));
        }
        self.ver = self.ver.wrapping_add(1);
    }

    // move node, frame (its members) or area `key` by d; moved: the node keys already moved
    fn shift(&mut self, key: u64, d: [f32; 2], moved: &mut Vec<u64>) {
        if let Some(f) = self.frames.iter().position(|f| f.key == key) {
            let members = self.frames[f].members.clone();
            for m in members {
                self.shift(m, d, moved);
            }
        } else if let Some(a) = self.areas.iter_mut().find(|a| a.key == key) {
            a.rect[0] += d[0];
            a.rect[1] += d[1];
        } else if let Some(&i) = self.at.get(&key) {
            if !moved.contains(&key) {
                moved.push(key);
                self.nodes[i].rect[0] += d[0];
                self.nodes[i].rect[1] += d[1];
            }
        }
    }

    fn add_node(&mut self, x: f32, y: f32) -> u64 {
        self.next_node += 1;
        let key = self.next_node;
        let n = (key - NODE0) as usize;
        self.nodes.push(Node { key, label: format!("N{}", n), rect: [x, y, NW, NH], kind: (n - 1) % 4 });
        key
    }

    fn add_edge(&mut self, a: u64, b: u64, route: u8, wps: &[[f32; 2]], dashed: bool) -> u64 {
        self.next_edge += 1;
        let key = self.next_edge;
        self.edges.push(Edge { key, a, b, route, wps: wps.to_vec(), dashed });
        key
    }

    fn label(&self, key: u64) -> String {
        if let Some(f) = self.frames.iter().find(|f| f.key == key) {
            return if f.collapsed { format!("{} ({})", f.label, f.members.len()) } else { f.label.clone() };
        }
        if let Some(a) = self.areas.iter().find(|a| a.key == key) {
            return a.label.clone();
        }
        if let Some(&i) = self.at.get(&node_of(key)) {
            return self.nodes[i].label.clone();
        }
        match self.edges.iter().position(|e| e.key == key) {
            Some(i) => format!("E{}", i + 1),
            None => String::from("–"),
        }
    }

    // 30 boxes in six columns, edges of every kind
    fn demo() -> Scene {
        let mut s = Scene { next_node: NODE0, next_edge: EDGE0, ..Scene::default() };
        for r in 0..5 {
            for c in 0..6 {
                s.add_node(48.0 + c as f32 * 160.0, 48.0 + r as f32 * 112.0);
            }
        }
        let n = |i: u64| NODE0 + i; // node i (1-based)
        let p = |i: u64, side: u64| port_key(NODE0 + i, side);
        // row 1: straight from right to left ports
        s.add_edge(p(1, 2), p(2, 1), DR_STRAIGHT, &[], false);
        s.add_edge(p(3, 2), p(4, 1), DR_STRAIGHT, &[], false);
        s.add_edge(p(5, 2), p(6, 1), DR_STRAIGHT, &[], true);
        // row 2: curves
        s.add_edge(p(7, 2), p(8, 1), DR_BEZIER, &[], false);
        s.add_edge(p(8, 4), p(15, 3), DR_BEZIER, &[], false);
        s.add_edge(p(10, 2), p(11, 1), DR_BEZIER, &[[656.0, 144.0]], false);
        // row 3: orthogonal, with and without waypoints
        s.add_edge(p(13, 2), p(20, 1), DR_ORTHO, &[], false);
        s.add_edge(p(16, 4), p(23, 3), DR_ORTHO, &[], false);
        s.add_edge(p(17, 2), p(24, 3), DR_ORTHO, &[[848.0, 352.0]], false);
        // columns: straight down, one with a waypoint
        s.add_edge(p(1, 4), p(7, 3), DR_STRAIGHT, &[], false);
        s.add_edge(p(2, 4), p(9, 3), DR_ORTHO, &[], false);
        s.add_edge(p(3, 4), p(10, 1), DR_STRAIGHT, &[[496.0, 128.0]], false);
        s.add_edge(p(4, 4), p(10, 3), DR_ORTHO, &[], false);
        s.add_edge(p(19, 4), p(25, 3), DR_STRAIGHT, &[], true);
        s.add_edge(p(21, 4), p(27, 3), DR_BEZIER, &[], false);
        // whole nodes (the route starts on their borders)
        s.add_edge(n(14), n(22), DR_STRAIGHT, &[], false);
        s.add_edge(n(28), n(23), DR_ORTHO, &[], false);
        s.add_edge(p(28, 2), p(29, 1), DR_STRAIGHT, &[], false);
        s.add_edge(p(29, 2), p(30, 1), DR_BEZIER, &[], false);
        s.add_edge(p(12, 4), p(18, 3), DR_STRAIGHT, &[], false);
        s.add_edge(p(18, 4), p(24, 3), DR_ORTHO, &[], false);
        if plat::args().iter().any(|a| a == "--frames") {
            s.frames.push(Frame { key: FRAME0 + 1, label: String::from("G1"), members: alloc::vec![n(8), n(9), n(14), n(15)], collapsed: false, rect: [0.0; 4] });
            s.areas.push(Area { key: AREA0 + 1, label: String::from("Fläche"), rect: [32.0, 472.0, 304.0, 104.0] });
        }
        s.rebuild();
        s
    }
}

// paints edges and boxes at render time (culled by the frame's clip)
struct Painter {
    sc: Rc<RefCell<Scene>>,
}

impl DgPaint for Painter {
    fn paint(&self, g: &mut Gfx, f: &DgFrame) {
        let Ok(sc) = self.sc.try_borrow() else { return };
        let s = f.vp.scale() as f32;
        let hair = ((f.vp.dpi + 24) / 96).max(1);
        let px = |r: [f32; 4]| {
            let q = f.vp.rect_to_screen(r);
            let (x, y) = ((q[0] + 0.5) as i32, (q[1] + 0.5) as i32);
            [x, y, ((q[0] + q[2] + 0.5) as i32 - x).max(2), ((q[1] + q[3] + 0.5) as i32 - y).max(2)]
        };
        // areas and frames: a tinted fill, a stroke; a frame's title bar with its label and the
        // collapse button (a chevron) at its right end
        let accent = f.color(T_ACCENT);
        for a in sc.areas.iter().filter(|a| f.visible(a.rect)) {
            let q = px(a.rect);
            let mut sh = g.shapes();
            sh.dc.set_color(color_mix(f.bg, accent | 0xFF00_0000, 20));
            sh.fill_rrect(q[0], q[1], q[2], q[3], 0);
            sh.dc.set_color(f.color(T_STRONG_STROKE));
            sh.stroke_rrect(q[0], q[1], q[2], q[3], 0, hair);
        }
        let (font, size) = f.font(TS_BODY);
        for fr in sc.frames.iter().filter(|x| !x.collapsed && f.visible(x.rect)) {
            let q = px(fr.rect);
            let bar = (FRAME_BAR * s) as i32;
            let rad = (4.0 * s) as i32;
            {
                let mut sh = g.shapes();
                sh.dc.set_color(color_mix(f.bg, accent | 0xFF00_0000, 14));
                sh.fill_rrect(q[0], q[1], q[2], q[3], rad);
                sh.dc.set_color(color_mix(f.bg, accent | 0xFF00_0000, 40));
                sh.fill_rrect(q[0], q[1], q[2], bar, rad);
                sh.dc.set_color(accent);
                sh.stroke_rrect(q[0], q[1], q[2], q[3], rad, hair);
            }
            if size >= 6 << 8 {
                g.text.set_font(font, size);
                g.dc.set_color(f.color(T_TEXT));
                g.text.draw(g.dc, g.sf, q[0] + (8.0 * s) as i32, q[1] + (bar + g.text.ascent() - g.text.descent()) / 2, fr.label.as_bytes());
            }
            let t = px(dg_toggle_rect(&DNode { key: fr.key, rect: fr.rect, ports: 0..0, flags: DN_FRAME | DN_TOGGLE }, FRAME_BAR));
            let (cx, cy, d) = (t[0] + t[2] / 2, t[1] + t[3] / 2, (t[2] / 5).max(2));
            g.dc.set_color(f.color(T_TEXT2));
            let fxp = |v: i32| v << 8;
            let mut sh = g.shapes();
            sh.line_fx(fxp(cx - d), fxp(cy - d / 2), fxp(cx), fxp(cy + d / 2), hair << 8);
            sh.line_fx(fxp(cx), fxp(cy + d / 2), fxp(cx + d), fxp(cy - d / 2), hair << 8);
        }
        let edge_c = f.color(T_TEXT2);
        for (i, rt) in f.geo.routes.iter().enumerate() {
            if !f.route_visible(rt, 2.0) {
                continue;
            }
            let e = sc.drawn.iter().filter_map(|x| *x).nth(i).and_then(|k| sc.edges.get(k));
            let dash = if e.is_some_and(|e| e.dashed) { DASH_DASH } else { DASH_SOLID };
            dg_stroke_edge(g, &f.vp, &rt.pts, 2.0, edge_c, LCAP_FLAT, LCAP_ARROW_FILLED, dash);
        }
        // the card colour made opaque over the canvas (boxes may overlap)
        let card = f.color(T_CARD);
        let fill = color_mix(f.bg, card | 0xFF00_0000, ((card >> 24) * 256 / 255) as i32);
        let (stroke, text) = (f.color(T_CTRL_STROKE), f.color(T_TEXT));
        let (font, size) = f.font(TS_BODY);
        for n in &sc.nodes {
            if !f.visible(n.rect) || sc.hidden_by(n.key).is_some() {
                continue;
            }
            let r = f.vp.rect_to_screen(n.rect);
            let (x, y) = ((r[0] + 0.5) as i32, (r[1] + 0.5) as i32);
            let (w, h) = (((r[0] + r[2] + 0.5) as i32 - x).max(2), ((r[1] + r[3] + 0.5) as i32 - y).max(2));
            let rad = (6.0 * s) as i32;
            let mut sh = g.shapes();
            sh.dc.set_color(fill);
            sh.fill_rrect(x, y, w, h, rad);
            sh.dc.set_color(stroke);
            sh.stroke_rrect(x, y, w, h, rad, hair);
            // the kind: a coloured dot
            let d = (5.0 * s * 256.0) as i32;
            sh.dc.set_color(f.color(KIND_TOKENS[n.kind]));
            sh.fill_circle_fx(((x as f32 + 14.0 * s) * 256.0) as i32, (y << 8) + (h << 7), d);
            // the label, when it is big enough to read
            if size >= 6 << 8 {
                g.text.set_font(font, size);
                let tw = g.text.width(&n.label);
                let base = y + (h + g.text.ascent() - g.text.descent()) / 2;
                g.dc.set_color(text);
                g.text.draw(g.dc, g.sf, x + (w - tw) / 2 + (6.0 * s) as i32, base, n.label.as_bytes());
            }
        }
        // the clusters of collapsed frames: two cards behind the front one, the label
        for fr in sc.frames.iter().filter(|x| x.collapsed && f.visible(x.rect)) {
            let q = px(fr.rect);
            let (o, rad) = ((4.0 * s) as i32, (6.0 * s) as i32);
            let mut sh = g.shapes();
            for k in [2, 1] {
                sh.dc.set_color(fill);
                sh.fill_rrect(q[0] + o * k, q[1] - o * k, q[2], q[3], rad);
                sh.dc.set_color(accent);
                sh.stroke_rrect(q[0] + o * k, q[1] - o * k, q[2], q[3], rad, hair);
            }
            sh.dc.set_color(fill);
            sh.fill_rrect(q[0], q[1], q[2], q[3], rad);
            sh.dc.set_color(accent);
            sh.stroke_rrect(q[0], q[1], q[2], q[3], rad, hair);
            if size >= 6 << 8 {
                let l = sc.label(fr.key);
                g.text.set_font(font, size);
                let tw = g.text.width(&l);
                g.dc.set_color(text);
                g.text.draw(g.dc, g.sf, q[0] + (q[2] - tw) / 2, q[1] + (q[3] + g.text.ascent() - g.text.descent()) / 2, l.as_bytes());
            }
        }
    }
}

struct DiagramTest {
    sc: Rc<RefCell<Scene>>,
    painter: Rc<dyn DgPaint>,
    st: DiagramState,
    tool: u32,
    route: u8,
    last: String,
}

impl DiagramTest {
    fn new() -> DiagramTest {
        let sc = Rc::new(RefCell::new(Scene::demo()));
        let painter: Rc<dyn DgPaint> = Rc::new(Painter { sc: sc.clone() });
        let mut st = DiagramState::new();
        st.frame_bar = FRAME_BAR;
        DiagramTest { sc, painter, st, tool: DG_SELECT, route: DR_STRAIGHT, last: String::from("–") }
    }

    // delete the selection: nodes with their edges, edges
    fn delete(&mut self) {
        let sel = core::mem::take(&mut self.st.sel);
        let mut sc = self.sc.borrow_mut();
        sc.nodes.retain(|n| !sel.contains(&n.key));
        sc.edges.retain(|e| !sel.contains(&e.key) && !sel.contains(&node_of(e.a)) && !sel.contains(&node_of(e.b)));
        sc.frames.retain(|f| !sel.contains(&f.key));
        sc.areas.retain(|a| !sel.contains(&a.key));
        for f in sc.frames.iter_mut() {
            f.members.retain(|m| !sel.contains(m));
        }
        sc.rebuild();
        self.last = format!("Gelöscht: {}", sel.len());
    }

    fn handle(&mut self, ev: DEv) {
        match ev {
            DEv::Move { keys, d, done, .. } => {
                let mut sc = self.sc.borrow_mut();
                let mut moved = Vec::new();
                for k in &keys {
                    sc.shift(*k, d, &mut moved);
                }
                sc.rebuild();
                if done {
                    self.last = format!("Verschoben: {}", keys.len());
                }
            }
            DEv::Connect { from, to, quick } => {
                let mut sc = self.sc.borrow_mut();
                sc.add_edge(from, to, self.route, &[], false);
                sc.rebuild();
                self.last = format!("Verbunden: {}-{}{}", sc.label(from), sc.label(to), if quick { " (schnell)" } else { "" });
            }
            DEv::ConnectDrag { over, .. } => {
                let sc = self.sc.borrow();
                self.last = if over != 0 { format!("Verbinden mit {}", sc.label(over)) } else { String::from("Verbinden …") };
            }
            DEv::ConnectCancel => self.last = String::from("Verbinden abgebrochen"),
            DEv::Waypoint { edge, idx, at, insert, done } => {
                let mut sc = self.sc.borrow_mut();
                if let Some(e) = sc.edges.iter_mut().find(|e| e.key == edge) {
                    let i = (idx as usize).min(e.wps.len());
                    if insert {
                        e.wps.insert(i, at);
                    } else if let Some(w) = e.wps.get_mut(i) {
                        *w = at;
                    }
                }
                sc.rebuild();
                if done {
                    self.last = format!("Wegpunkt {}: {},{}", idx + 1, at[0] as i32, at[1] as i32);
                }
            }
            DEv::WaypointDelete { edge, idx } => {
                let mut sc = self.sc.borrow_mut();
                if let Some(e) = sc.edges.iter_mut().find(|e| e.key == edge) {
                    if (idx as usize) < e.wps.len() {
                        e.wps.remove(idx as usize);
                    }
                }
                sc.rebuild();
                self.last = format!("Wegpunkt {} entfernt", idx + 1);
            }
            DEv::Delete => self.delete(),
            DEv::Click { at } => {
                if self.tool == DG_PLACE {
                    let mut sc = self.sc.borrow_mut();
                    let k = sc.add_node(at[0] - NW * 0.5, at[1] - NH * 0.5);
                    sc.rebuild();
                    self.last = format!("Platziert: {}", sc.label(k));
                }
            }
            DEv::Activate(h) => {
                // a cluster opens again
                let mut sc = self.sc.borrow_mut();
                self.last = format!("Aktiviert: {}", sc.label(h.key()));
                if let Some(f) = sc.frames.iter_mut().find(|f| f.key == h.key() && f.collapsed) {
                    f.collapsed = false;
                    sc.rebuild();
                }
            }
            DEv::Toggle(k) => {
                let mut sc = self.sc.borrow_mut();
                if let Some(f) = sc.frames.iter_mut().find(|f| f.key == k) {
                    f.collapsed = !f.collapsed;
                }
                sc.rebuild();
                self.last = format!("Umgeschaltet: {}", sc.label(k));
            }
            DEv::Resize { key, rect, done } => {
                let mut sc = self.sc.borrow_mut();
                if let Some(a) = sc.areas.iter_mut().find(|a| a.key == key) {
                    a.rect = rect;
                }
                sc.rebuild();
                if done {
                    self.last = format!("Größe: {}x{}", rect[2] as i32, rect[3] as i32);
                }
            }
            DEv::Context(h, _) => {
                let what = if h == DHit::None { String::from("Hintergrund") } else { self.sc.borrow().label(h.key()) };
                self.last = format!("Kontext: {}", what);
            }
            DEv::Select | DEv::ViewChanged => {}
        }
    }

    fn status(&self, ui: &mut Ui) {
        let sc = self.sc.borrow();
        let st = &self.st;
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_LAYER | pad(6), 16);
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("Auswahl: {}", st.sel.len()));
        let z = (st.vp.zoom * 100.0 + 0.5) as i32;
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("Zoom: {} %", z));
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("Mitte: {},{}", rnd(st.vp.cx), rnd(st.vp.cy)));
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("Knoten: {}", sc.nodes.len()));
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("Kanten: {}", sc.edges.len()));
        let focus = if st.focus != 0 { sc.label(st.focus) } else { String::from("–") };
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("Fokus: {}", focus));
        if st.sel.len() == 1 {
            let k = *st.sel.iter().next().unwrap_or(&0);
            if let Some(fr) = sc.frames.iter().find(|f| f.key == k) {
                ui.text(TS_CAPTION | TX_SECONDARY, &format!("Rahmen: {},{} {}x{}", fr.rect[0] as i32, fr.rect[1] as i32, fr.rect[2] as i32, fr.rect[3] as i32));
            } else if let Some(a) = sc.areas.iter().find(|a| a.key == k) {
                ui.text(TS_CAPTION | TX_SECONDARY, &format!("Fläche: {},{} {}x{}", a.rect[0] as i32, a.rect[1] as i32, a.rect[2] as i32, a.rect[3] as i32));
            } else if let Some(&i) = sc.at.get(&k) {
                let r = sc.nodes[i].rect;
                ui.text(TS_CAPTION | TX_SECONDARY, &format!("Position: {},{}", r[0] as i32, r[1] as i32));
            } else if let Some(e) = sc.edges.iter().find(|e| e.key == k) {
                ui.text(TS_CAPTION | TX_SECONDARY, &format!("Wegpunkte: {}", e.wps.len()));
            }
        }
        if let Some(fr) = sc.frames.first() {
            ui.text(TS_CAPTION | TX_SECONDARY, &format!("Gruppe: {}", if fr.collapsed { "zugeklappt" } else { "aufgeklappt" }));
        }
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("Ereignis: {}", self.last));
        ui.box_end();
    }
}

impl App for DiagramTest {
    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_FIXED, 0);
        // the tool bar
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_LAYER | pad(6), 6);
        for (k, name) in TOOLS.iter().enumerate() {
            ui.next_flags(if self.tool == k as u32 { ST_CHECKED } else { 0 });
            if ui.button(ID_TOOL + k as u32, name) & EV_ACT != 0 {
                self.tool = k as u32;
            }
        }
        ui.separator();
        for (k, name) in ROUTES.iter().enumerate() {
            ui.next_flags(if self.route == k as u8 { ST_CHECKED } else { 0 });
            if ui.button(ID_ROUTE + k as u32, name) & EV_ACT != 0 {
                self.route = k as u8;
            }
        }
        ui.separator();
        if ui.button(ID_ADD, "Knoten hinzufügen") & EV_ACT != 0 {
            let (cx, cy) = (self.st.vp.cx as f32, self.st.vp.cy as f32);
            let g = 16.0;
            let (x, y) = (((cx - NW * 0.5) / g + 0.5) as i32 as f32 * g, ((cy - NH * 0.5) / g + 0.5) as i32 as f32 * g);
            let mut sc = self.sc.borrow_mut();
            let k = sc.add_node(x, y);
            sc.rebuild();
            self.last = format!("Hinzugefügt: {}", sc.label(k));
        }
        ui.next_flags(if self.st.sel.is_empty() { ST_DISABLED } else { 0 });
        if ui.button(ID_DEL, "Löschen") & EV_ACT != 0 {
            self.delete();
        }
        if ui.button(ID_FIT, "Einpassen") & EV_ACT != 0 {
            self.st.request_fit();
        }
        let mut snap = self.st.snap as i32;
        ui.checkbox(ID_SNAP, "Am Raster ausrichten", &mut snap);
        self.st.snap = snap != 0;
        ui.box_end();
        // the canvas with its overview and its records
        ui.next_size(-1, -1);
        let evs = {
            let sc = self.sc.clone();
            let sc = sc.borrow();
            let m = sc.model();
            let evs = ui.diagram(ID_DIAGRAM, &mut self.st, &m, self.tool, Some(self.painter.clone()), 0);
            ui.diagram_minimap(ID_MINI, &mut self.st, &m);
            ui.diagram_sems(ID_DIAGRAM, &mut self.st, &m, &|k| sc.label(k));
            evs
        };
        for ev in evs {
            self.handle(ev);
        }
        self.status(ui);
        ui.box_end();
    }

    fn command(&mut self, _ui: &mut Ui, id: u32, arg: u32) -> bool {
        if id != CMD_TEST_KEYS {
            return false;
        }
        self.st.test_keys = arg;
        true
    }
}

ui::main!(ui::launch(&DESC, DiagramTest::new(), &style::STYLE));
