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

// the app's diagram and the flat arrays of its DModel
#[derive(Default)]
struct Scene {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
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

    // the flat model after a change (a new version)
    fn rebuild(&mut self) {
        let used: Vec<u64> = self.edges.iter().flat_map(|e| [e.a, e.b]).collect();
        self.dn.clear();
        self.dp.clear();
        self.de.clear();
        self.dw.clear();
        self.at.clear();
        for (i, n) in self.nodes.iter().enumerate() {
            let p0 = self.dp.len() as u32;
            let (w, h) = (n.rect[2], n.rect[3]);
            for (side, at) in [(1, [0.0, h * 0.5]), (2, [w, h * 0.5]), (3, [w * 0.5, 0.0]), (4, [w * 0.5, h])] {
                let key = port_key(n.key, side);
                self.dp.push(DPort { key, at, flags: if used.contains(&key) { DPT_USED } else { 0 } });
            }
            self.dn.push(DNode { key: n.key, rect: n.rect, ports: p0..self.dp.len() as u32, flags: 0 });
            self.at.insert(n.key, i);
        }
        for e in &self.edges {
            let w0 = self.dw.len() as u32;
            self.dw.extend_from_slice(&e.wps);
            self.de.push(DEdge { key: e.key, a: e.a, b: e.b, route: e.route, wps: w0..self.dw.len() as u32 });
        }
        self.ver = self.ver.wrapping_add(1);
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
        let edge_c = f.color(T_TEXT2);
        for (i, rt) in f.geo.routes.iter().enumerate() {
            if !f.route_visible(rt, 2.0) {
                continue;
            }
            let dash = if sc.edges.get(i).is_some_and(|e| e.dashed) { DASH_DASH } else { DASH_SOLID };
            dg_stroke_edge(g, &f.vp, &rt.pts, 2.0, edge_c, LCAP_FLAT, LCAP_ARROW_FILLED, dash);
        }
        let hair = ((f.vp.dpi + 24) / 96).max(1);
        // the card colour made opaque over the canvas (boxes may overlap)
        let card = f.color(T_CARD);
        let fill = color_mix(f.bg, card | 0xFF00_0000, ((card >> 24) * 256 / 255) as i32);
        let (stroke, text) = (f.color(T_CTRL_STROKE), f.color(T_TEXT));
        let (font, size) = f.font(TS_BODY);
        for n in &sc.nodes {
            if !f.visible(n.rect) {
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
        DiagramTest { sc, painter, st: DiagramState::new(), tool: DG_SELECT, route: DR_STRAIGHT, last: String::from("–") }
    }

    // delete the selection: nodes with their edges, edges
    fn delete(&mut self) {
        let sel = core::mem::take(&mut self.st.sel);
        let mut sc = self.sc.borrow_mut();
        sc.nodes.retain(|n| !sel.contains(&n.key));
        sc.edges.retain(|e| !sel.contains(&e.key) && !sel.contains(&node_of(e.a)) && !sel.contains(&node_of(e.b)));
        sc.rebuild();
        self.last = format!("Gelöscht: {}", sel.len());
    }

    fn handle(&mut self, ev: DEv) {
        match ev {
            DEv::Move { keys, d, done, .. } => {
                let mut sc = self.sc.borrow_mut();
                for k in &keys {
                    if let Some(&i) = sc.at.get(k) {
                        sc.nodes[i].rect[0] += d[0];
                        sc.nodes[i].rect[1] += d[1];
                    }
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
            DEv::Activate(h) => self.last = format!("Aktiviert: {}", self.sc.borrow().label(h.key())),
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
            if let Some(&i) = sc.at.get(&k) {
                let r = sc.nodes[i].rect;
                ui.text(TS_CAPTION | TX_SECONDARY, &format!("Position: {},{}", r[0] as i32, r[1] as i32));
            } else if let Some(e) = sc.edges.iter().find(|e| e.key == k) {
                ui.text(TS_CAPTION | TX_SECONDARY, &format!("Wegpunkte: {}", e.wps.len()));
            }
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
