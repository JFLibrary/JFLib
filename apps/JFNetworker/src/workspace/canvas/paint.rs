// paint -- what the topology canvas shows: the scene (the project flattened for ui::diagram)
// and its painting at render time
//
//   Scene::build(&project, &symbols)   nodes for the diagram (notes below, devices above) with
//                                      their ports, one edge per link (its route and
//                                      waypoints), what to paint; a new version
//   scene.model() -> DModel            borrowed by ui.diagram every frame
//   scene.label(key) / rect(key)       the name of a node (scripts, screen readers), its box
//   scene.port(key) / link(id)         what a port key names (device, port index), a link
//   port_key(dev, index)               the key of a port in the diagram
//   Painter                            the DgPaint: reads the scene and the simulator's LEDs
//                                      through a Weak to Net
//
// Devices are their symbol's 64 x 48 box (world units = DIP at 100 %) with the host name
// below; the widget draws selection, hover and focus around the box.  A cable leaves a device
// where the line from its centre towards the other end (or the first waypoint) crosses the box:
// that point is its (hidden) port in the diagram, so the widget routes it straight, as a curve
// or at right angles through its waypoints, and parallel cables run 7 units apart.  Each device
// also shows one free port per port group as a dot on the border while hovered (the next free
// Gi port, the next free Te port, ...): a drag from dot to dot connects at once.
//
// Cables are drawn along the widget's routes in their kind's colour and pattern, without
// relying on colour alone: copper graphite (crossover dashed), fibre muted orange, serial brick
// red with a zigzag in the middle, console light blue dotted, radio thin teal dotted, WAN a
// double line; an unplugged cable is faded.  12 units before each end sits the port's LED from
// the simulator (green up, red down, grey without power, amber blocking or learning); from
// 100 % on the short port names stand next to it.  Levels of detail: below 50 % no names and no
// LEDs, below 30 % a tinted square instead of the symbol; while a gesture moves the view or
// devices (a pan, wheel turns, a drag) and more than LOD_MIN devices and cables are in view, no
// names, LEDs and port names either, and the cables are plain 1 px lines.  The widget keeps the
// painting in a backing store and repaints it only when the scene, the view, the LEDs or the
// theme change; the painter asks the widget's buckets for what lies near the clip and culls
// that against it, with labels and line widths included.

use alloc::collections::BTreeMap;
use alloc::rc::Weak;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use gfx::dash::{DASH_DASH, DASH_DOT, DASH_SOLID, LCAP_FLAT};
use jfn_model::catalog::PortSpec;
use jfn_model::project::Route;
use jfn_model::{catalog, ifname, CableKind, Project};
use jfn_sim::Led;
use ui::*;

use crate::app::Net;
use crate::workspace::icons::{tint_of, Symbols, TINTS};

/// a device's box in world units
pub(crate) const DEV_W: f32 = 64.0;
pub(crate) const DEV_H: f32 = 48.0;
/// the gap between a device's box and its name
const LABEL_GAP: f32 = 3.0;
/// the distance of parallel links (world units)
const PARALLEL: f32 = 7.0;
/// the inner padding of a note (world units)
const NOTE_PAD: f32 = 8.0;
/// where a cable's LED sits: this far from its end along the cable, and its radius
const LED_AT: f32 = 12.0;
const LED_R: f32 = 3.0;
/// a port name: this far from the end along the cable, this far beside it
const NAME_AT: f32 = 26.0;
const NAME_SIDE: f32 = 9.0;
/// how far names and port names reach beyond a box or a cable (world units)
const REACH: f32 = 96.0;
/// devices and cables in view above which a moving view leaves out names, LEDs and port names
const LOD_MIN: usize = 150;

/// the diagram key of a port: devices, links and notes have project ids below 2^32
pub(crate) fn port_key(dev: u32, index: u16) -> u64 {
    1 << 48 | (dev as u64) << 16 | index as u64
}

/// (device, port index) of a port key
pub(crate) fn key_port(key: u64) -> Option<(u32, u16)> {
    if key >> 48 == 1 { Some(((key >> 16) as u32, key as u16)) } else { None }
}

/// a device to paint
pub(crate) struct SDev {
    pub rect: [f32; 4],
    /// the symbol (None: an unknown model, drawn as a box)
    pub sym: Option<usize>,
    pub tint: usize,
    pub name: String,
    pub off: bool,
}

/// a note to paint
pub(crate) struct SNote {
    pub rect: [f32; 4],
    pub text: String,
}

/// a link to paint (along the widget's route of the edge with the same index)
pub(crate) struct SLink {
    pub id: u32,
    pub cable: CableKind,
    /// unplugged on purpose
    pub down: bool,
    /// the short port names ("Gi1/0/1")
    pub names: [String; 2],
    /// the side of each end's port name (1: left of the way out of the device, -1: right;
    /// parallel cables put them outside)
    pub sides: [f32; 2],
}

/// the project as the canvas needs it
#[derive(Default)]
pub(crate) struct Scene {
    pub nodes: Vec<DNode>,
    pub ports: Vec<DPort>,
    pub edges: Vec<DEdge>,
    pub wps: Vec<[f32; 2]>,
    /// the diagram's model version (bumped by every build)
    pub ver: u32,
    /// the Doc::rev it was built from
    pub rev: u32,
    pub devs: Vec<SDev>,
    pub notes: Vec<SNote>,
    /// in the order of the edges
    pub links: Vec<SLink>,
    /// node key -> index in nodes (the notes first, then the devices)
    at: BTreeMap<u64, usize>,
    /// the grid is shown (else the painter covers it: snapping works without it)
    pub grid: bool,
}

/// a model's ports, expanded once per build, and the index of each exact name
#[derive(Default)]
struct ModelPorts {
    ports: Vec<PortSpec>,
    exact: BTreeMap<String, u16>,
}

// the ports of a model, expanded once per build
fn ports_of<'a>(cache: &'a mut BTreeMap<&'static str, ModelPorts>, m: Option<&'static catalog::Model>) -> Option<&'a ModelPorts> {
    let m = m?;
    Some(cache.entry(m.id).or_insert_with(|| {
        let ports = m.ports();
        let exact = ports.iter().map(|p| (p.name.clone(), p.index)).collect();
        ModelPorts { ports, exact }
    }))
}

// the index of a port by any spelling (projects keep the full names: those are looked up)
fn port_index(mp: Option<&ModelPorts>, name: &str) -> Option<u16> {
    let mp = mp?;
    if let Some(&i) = mp.exact.get(name) {
        return Some(i);
    }
    let ports = &mp.ports;
    ports.iter().find(|p| p.name.eq_ignore_ascii_case(name.trim())).or_else(|| ports.iter().find(|p| ifname::same(&p.name, name))).map(|p| p.index)
}

// where the line from `from` (inside the box r: x, y, w, h) towards `to` leaves the box
fn border_hit(r: [f32; 4], from: [f32; 2], to: [f32; 2]) -> [f32; 2] {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let mut t = f32::MAX;
    if dx > 1e-4 {
        t = t.min((r[0] + r[2] - from[0]) / dx);
    } else if dx < -1e-4 {
        t = t.min((r[0] - from[0]) / dx);
    }
    if dy > 1e-4 {
        t = t.min((r[1] + r[3] - from[1]) / dy);
    } else if dy < -1e-4 {
        t = t.min((r[1] - from[1]) / dy);
    }
    if !(t < f32::MAX) || t < 0.0 {
        return [r[0] + r[2] * 0.5, r[1]];
    }
    let p = [from[0] + dx * t, from[1] + dy * t];
    [p[0].clamp(r[0], r[0] + r[2]), p[1].clamp(r[1], r[1] + r[3])]
}

// a point at distance s along the border of a w x h box, clockwise from its top left corner
fn perimeter(w: f32, h: f32, s: f32) -> [f32; 2] {
    let p = 2.0 * (w + h);
    let mut s = s;
    while s >= p {
        s -= p;
    }
    while s < 0.0 {
        s += p;
    }
    if s < w {
        [s, 0.0]
    } else if s < w + h {
        [w, s - w]
    } else if s < 2.0 * w + h {
        [w - (s - w - h), h]
    } else {
        [0.0, h - (s - 2.0 * w - h)]
    }
}

impl Scene {
    pub(crate) fn model(&self) -> DModel<'_> {
        DModel { nodes: &self.nodes, ports: &self.ports, edges: &self.edges, wps: &self.wps, ver: self.ver }
    }

    /// flatten the project (after every change)
    pub(crate) fn build(&mut self, p: &Project, syms: &Symbols, rev: u32) {
        self.nodes.clear();
        self.ports.clear();
        self.edges.clear();
        self.wps.clear();
        self.devs.clear();
        self.notes.clear();
        self.links.clear();
        self.at.clear();
        for n in &p.notes {
            let rect = [n.x as f32, n.y as f32, n.w.max(40) as f32, n.h.max(24) as f32];
            self.at.insert(n.id as u64, self.nodes.len());
            self.nodes.push(DNode { key: n.id as u64, rect, ports: 0..0, flags: 0 });
            self.notes.push(SNote { rect, text: n.text.clone() });
        }
        // the devices by id with their models (looked up once, not per cable end)
        let by_id: BTreeMap<u32, (&jfn_model::Device, Option<&'static catalog::Model>)> = p.devices.iter().map(|d| (d.id, (d, catalog::model(&d.model)))).collect();
        let rect_of = |id: u32| by_id.get(&id).map(|(d, _)| [d.x as f32, d.y as f32, DEV_W, DEV_H]);
        let model_of = |id: u32| by_id.get(&id).and_then(|x| x.1);
        let centre = |r: [f32; 4]| [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5];
        // the cables: their ports where they leave the boxes (parallel ones side by side)
        let mut cache: BTreeMap<&'static str, ModelPorts> = BTreeMap::new();
        // the ports with a cable: where it leaves the box (x, y in the box)
        let mut used: BTreeMap<(u32, u16), [f32; 2]> = BTreeMap::new();
        let mut taken: alloc::collections::BTreeSet<(u32, u16)> = alloc::collections::BTreeSet::new();
        let pair = |a: u32, b: u32| if a < b { (a, b) } else { (b, a) };
        let mut count: BTreeMap<(u32, u32), (usize, usize)> = BTreeMap::new();
        for l in &p.links {
            count.entry(pair(l.a.dev, l.b.dev)).or_default().0 += 1;
        }
        for l in &p.links {
            let ia = port_index(ports_of(&mut cache, model_of(l.a.dev)), &l.a.port);
            let ib = port_index(ports_of(&mut cache, model_of(l.b.dev)), &l.b.port);
            for (d, i) in [(l.a.dev, ia), (l.b.dev, ib)] {
                if let Some(i) = i {
                    taken.insert((d, i));
                }
            }
            let wp0 = self.wps.len() as u32;
            self.wps.extend(l.waypoints.iter().map(|w| [w.x as f32, w.y as f32]));
            let wps = &self.wps[wp0 as usize..];
            let route = match l.route {
                Route::Straight => DR_STRAIGHT,
                Route::Ortho => DR_ORTHO,
                Route::Curve => DR_BEZIER,
            };
            let key = pair(l.a.dev, l.b.dev);
            let c = count.entry(key).or_default();
            let (same, nth) = (c.0, c.1);
            c.1 += 1;
            // parallel cables name their ports on their outer side: left of the way out of the
            // lower id is the pair's normal, of the other end its opposite
            let off = nth as f32 - (same - 1) as f32 * 0.5;
            let s = if off < 0.0 { -1.0 } else { 1.0 };
            let (s0, s1) = if off == 0.0 { (1.0, 1.0) } else { (s, -s) };
            let sides = if l.a.dev == key.0 { [s0, s1] } else { [s1, s0] };
            if let (Some(ra), Some(rb)) = (rect_of(l.a.dev), rect_of(l.b.dev)) {
                let (ca, cb) = (centre(ra), centre(rb));
                // the offset of parallel links: the normal of the pair's direction
                let mut o = [0.0f32; 2];
                if same > 1 {
                    let (p0, p1) = if l.a.dev == key.0 { (ca, cb) } else { (cb, ca) };
                    let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
                    let len = rt::fx::sqrt(dx * dx + dy * dy).max(0.001);
                    let off = (nth as f32 - (same - 1) as f32 * 0.5) * PARALLEL;
                    o = [-dy / len * off, dx / len * off];
                }
                let ta = wps.first().copied().unwrap_or([cb[0] + o[0], cb[1] + o[1]]);
                let tb = wps.last().copied().unwrap_or([ca[0] + o[0], ca[1] + o[1]]);
                let pa = border_hit(ra, [ca[0] + o[0], ca[1] + o[1]], ta);
                let pb = border_hit(rb, [cb[0] + o[0], cb[1] + o[1]], tb);
                if let Some(i) = ia {
                    used.entry((l.a.dev, i)).or_insert([pa[0] - ra[0], pa[1] - ra[1]]);
                }
                if let Some(i) = ib {
                    used.entry((l.b.dev, i)).or_insert([pb[0] - rb[0], pb[1] - rb[1]]);
                }
            }
            let end = |dev: u32, i: Option<u16>| i.map_or(dev as u64, |i| port_key(dev, i));
            self.edges.push(DEdge { key: l.id as u64, a: end(l.a.dev, ia), b: end(l.b.dev, ib), route, wps: wp0..self.wps.len() as u32 });
            self.links.push(SLink {
                id: l.id,
                cable: l.cable,
                down: l.down,
                names: [ifname::short(&l.a.port), ifname::short(&l.b.port)],
                sides,
            });
        }
        // the devices with their ports: the cables' ends (hidden), one free port per group
        let mut sym_of: BTreeMap<&'static str, Option<usize>> = BTreeMap::new();
        for d in &p.devices {
            let m = by_id.get(&d.id).and_then(|x| x.1);
            let rect = [d.x as f32, d.y as f32, DEV_W, DEV_H];
            let first = self.ports.len() as u32;
            for (&(dev, i), &at) in used.range((d.id, 0)..=(d.id, u16::MAX)) {
                self.ports.push(DPort { key: port_key(dev, i), at, flags: DPT_USED | DPT_HIDDEN });
            }
            if let Some(m) = m {
                let all = ports_of(&mut cache, Some(m)).map_or(&[][..], |x| &x.ports);
                let mut free = Vec::new();
                let mut k = 0usize;
                for g in m.groups {
                    let grp = &all[k..k + g.count as usize];
                    k += g.count as usize;
                    if let Some(p) = grp.iter().find(|p| !taken.contains(&(d.id, p.index))) {
                        free.push(p.index);
                    }
                }
                let n = free.len().max(1) as f32;
                let per = 2.0 * (DEV_W + DEV_H);
                for (j, i) in free.into_iter().enumerate() {
                    let at = perimeter(DEV_W, DEV_H, DEV_W * 0.5 + j as f32 * per / n);
                    self.ports.push(DPort { key: port_key(d.id, i), at, flags: 0 });
                }
            }
            self.at.insert(d.id as u64, self.nodes.len());
            self.nodes.push(DNode { key: d.id as u64, rect, ports: first..self.ports.len() as u32, flags: 0 });
            self.devs.push(SDev {
                rect,
                sym: m.and_then(|m| *sym_of.entry(m.symbol).or_insert_with(|| syms.index(m.symbol))),
                tint: m.map_or(0, |m| tint_of(m.category)),
                name: d.name.clone(),
                off: d.off,
            });
        }
        self.ver = self.ver.wrapping_add(1);
        self.rev = rev;
    }

    /// the box of a node
    pub(crate) fn rect(&self, key: u64) -> Option<[f32; 4]> {
        Some(self.nodes.get(*self.at.get(&key)?)?.rect)
    }

    /// the name of a node: a device's host name, "Notiz: " and the first line of a note
    pub(crate) fn label(&self, key: u64) -> String {
        let Some(&i) = self.at.get(&key) else { return String::from("–") };
        if let Some(n) = self.notes.get(i) {
            let first = n.text.lines().next().unwrap_or("").trim();
            return if first.is_empty() { String::from("Notiz") } else { alloc::format!("Notiz: {}", first) };
        }
        self.devs.get(i - self.notes.len()).map_or_else(|| String::from("–"), |d| d.name.clone())
    }
}

/// the colour (light theme), width (world units) and dash pattern of a cable
fn cable_style(k: CableKind) -> (u32, f32, u32) {
    match k {
        CableKind::CopperStraight => (0xFF5A_6169, 2.0, DASH_SOLID),
        CableKind::CopperCross => (0xFF5A_6169, 2.0, DASH_DASH),
        CableKind::FiberMm | CableKind::FiberSm => (0xFFC9_792B, 2.0, DASH_SOLID),
        CableKind::Serial => (0xFFB0_453F, 2.0, DASH_SOLID),
        CableKind::Console => (0xFF4F_86C6, 1.5, DASH_DOT),
        CableKind::Wireless => (0xFF3D_8C84, 1.0, DASH_DOT),
        CableKind::Wan => (0xFF5A_6169, 1.2, DASH_SOLID),
    }
}

/// the colour of a cable as drawn (theme, unplugged)
pub(crate) fn cable_color(k: CableKind, dark: bool, down: bool, bg: u32) -> u32 {
    let argb = cable_style(k).0;
    let argb = if dark { color_mix(argb, 0xFFFF_FFFF, 72) } else { argb };
    if down { color_mix(argb, bg | 0xFF00_0000, 150) } else { argb }
}

/// floor for drawing positions
fn fl(v: f32) -> i32 {
    let t = v as i32;
    if (t as f32) > v { t - 1 } else { t }
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    rt::fx::sqrt(dx * dx + dy * dy)
}

/// the point at arc length s of a polyline and the unit direction there
pub(crate) fn at_len(pts: &[[f32; 2]], s: f32) -> ([f32; 2], [f32; 2]) {
    let mut left = s.max(0.0);
    for w in pts.windows(2) {
        let l = dist(w[0], w[1]);
        if l <= 0.0 {
            continue;
        }
        let u = [(w[1][0] - w[0][0]) / l, (w[1][1] - w[0][1]) / l];
        if left <= l {
            return ([w[0][0] + u[0] * left, w[0][1] + u[1] * left], u);
        }
        left -= l;
    }
    let n = pts.len();
    if n >= 2 {
        let l = dist(pts[n - 2], pts[n - 1]).max(1e-6);
        return (pts[n - 1], [(pts[n - 1][0] - pts[n - 2][0]) / l, (pts[n - 1][1] - pts[n - 2][1]) / l]);
    }
    (pts.first().copied().unwrap_or([0.0; 2]), [1.0, 0.0])
}

/// at_len along the polyline walked from its last point (without reversing it)
pub(crate) fn at_len_back(pts: &[[f32; 2]], s: f32) -> ([f32; 2], [f32; 2]) {
    let mut left = s.max(0.0);
    for i in (1..pts.len()).rev() {
        let (a, b) = (pts[i], pts[i - 1]);
        let l = dist(a, b);
        if l <= 0.0 {
            continue;
        }
        let u = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
        if left <= l {
            return ([a[0] + u[0] * left, a[1] + u[1] * left], u);
        }
        left -= l;
    }
    if pts.len() >= 2 {
        let l = dist(pts[1], pts[0]).max(1e-6);
        return (pts[0], [(pts[0][0] - pts[1][0]) / l, (pts[0][1] - pts[1][1]) / l]);
    }
    (pts.first().copied().unwrap_or([0.0; 2]), [1.0, 0.0])
}

// the part of a polyline between arc lengths s0 and s1
fn cut(pts: &[[f32; 2]], s0: f32, s1: f32) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    let mut at = 0.0;
    out.push(at_len(pts, s0).0);
    for w in pts.windows(2) {
        let l = dist(w[0], w[1]);
        let next = at + l;
        if next > s0 && next < s1 {
            out.push(w[1]);
        }
        at = next;
    }
    out.push(at_len(pts, s1).0);
    out
}

// a polyline moved d units to its left (mitred corners, limited)
fn offset(pts: &[[f32; 2]], d: f32) -> Vec<[f32; 2]> {
    let n = pts.len();
    let normal = |a: [f32; 2], b: [f32; 2]| {
        let l = dist(a, b).max(1e-6);
        [-(b[1] - a[1]) / l, (b[0] - a[0]) / l]
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let na = if i > 0 { normal(pts[i - 1], pts[i]) } else { normal(pts[0], pts[1.min(n - 1)]) };
        let nb = if i + 1 < n { normal(pts[i], pts[i + 1]) } else { na };
        // the mitre: (na + nb) * 2 / |na + nb|^2 (1 on a straight line), limited at sharp turns
        let m = [na[0] + nb[0], na[1] + nb[1]];
        let ml = rt::fx::sqrt(m[0] * m[0] + m[1] * m[1]);
        let k = if ml > 1e-3 { (2.0 / (ml * ml)).min(4.0) } else { 1.0 };
        let v = if ml > 1e-3 { [m[0] * k, m[1] * k] } else { na };
        out.push([pts[i][0] + v[0] * d, pts[i][1] + v[1] * d]);
    }
    out
}

/// where the LED of the end at pts[0] sits (and the way out there); len: the route's length
pub(crate) fn led_at(pts: &[[f32; 2]], len: f32) -> ([f32; 2], [f32; 2]) {
    at_len(pts, LED_AT.min(len / 3.0))
}

/// the LED's radius in px at a scale
pub(crate) fn led_radius(scale: f32) -> i32 {
    let r = (LED_R * scale).max(1.5);
    fl(r + 0.5).max(2)
}

/// the LED colour of a port end: green up, red down, amber blocking or learning, grey without
/// power (theme colours through color, bg the canvas)
pub(crate) fn led_color(led: Led, color: &dyn Fn(u32) -> u32, bg: u32) -> u32 {
    match led {
        Led::Up => color(T_OK),
        Led::Down => color(T_CRIT),
        Led::Blocking | Led::Learning => color(T_WARN),
        Led::NoPower => color_mix(color(T_TEXT3) | 0xFF00_0000, bg | 0xFF00_0000, 96),
    }
}

/// the ring around an LED
pub(crate) fn led_ring(text: u32, bg: u32, dark: bool) -> u32 {
    color_mix(bg | 0xFF00_0000, text | 0xFF00_0000, if dark { 96 } else { 140 })
}

/// the DgPaint of the topology canvas
pub(crate) struct Painter {
    pub net: Weak<RefCell<Net>>,
}

impl DgPaint for Painter {
    fn paint(&self, g: &mut Gfx, f: &DgFrame) {
        let Some(net) = self.net.upgrade() else { return };
        let Ok(n) = net.try_borrow() else { return };
        let sc = &n.canvas.scene;
        let c = g.dc.clip;
        if !sc.grid {
            g.dc.set_color(f.bg);
            g.dc.fill(g.sf, c[0], c[1], c[2] - c[0], c[3] - c[1]);
        }
        // what may show: the nodes and cables in the buckets around the clip
        let margin = REACH * f.vp.scale() as f32;
        let nodes = f.find(margin, false);
        let edges = f.find(margin, true);
        let lod = f.moving && {
            let shown = nodes.iter().filter(|&&i| sc.nodes.get(i as usize).is_some_and(|x| f.visible(x.rect))).count();
            shown + edges.iter().filter(|&&i| f.geo.routes.get(i as usize).is_some_and(|rt| f.route_visible(rt, 2.0))).count() > LOD_MIN
        };
        paint_notes(g, f, sc, &nodes, lod);
        paint_links(g, f, sc, &edges, lod);
        paint_devices(g, f, sc, &n.syms, &nodes, lod);
        if !lod {
            paint_ends(g, f, sc, &n.doc.sim.leds, &edges);
        }
    }
}

// the px rectangle x0, y0, x1, y1 touches the clip (2 px for anti-aliasing)
fn touches(g: &Gfx, r: [i32; 4]) -> bool {
    let c = g.dc.clip;
    r[2] + 2 > c[0] && r[0] - 2 < c[2] && r[3] + 2 > c[1] && r[1] - 2 < c[3]
}

// px x, y, w, h of a world rectangle, rounded the same way for every clip
fn px_rect(f: &DgFrame, r: [f32; 4]) -> [i32; 4] {
    let q = f.vp.rect_to_screen(r);
    let (x0, y0) = (fl(q[0] + 0.5), fl(q[1] + 0.5));
    let (x1, y1) = (fl(q[0] + q[2] + 0.5), fl(q[1] + q[3] + 0.5));
    [x0, y0, (x1 - x0).max(1), (y1 - y0).max(1)]
}

fn paint_notes(g: &mut Gfx, f: &DgFrame, sc: &Scene, nodes: &[u32], lod: bool) {
    let s = f.vp.scale() as f32;
    let hair = ((f.vp.dpi + 24) / 96).max(1);
    let card = f.color(T_CARD);
    let fill = color_mix(f.bg, card | 0xFF00_0000, ((card >> 24) * 256 / 255) as i32);
    let fill = color_mix(fill, 0xFFF2_D46B, if f.dark { 18 } else { 40 });
    let stroke = f.color(T_CTRL_STROKE);
    let (font, size) = f.font(TS_BODY);
    // the notes are the first nodes
    for n in nodes.iter().filter_map(|&i| sc.notes.get(i as usize)) {
        let q = px_rect(f, n.rect);
        if !touches(g, [q[0], q[1], q[0] + q[2], q[1] + q[3]]) {
            continue;
        }
        let rad = ((4.0 * s) as i32).max(1);
        {
            let mut sh = g.shapes();
            sh.dc.set_color(fill);
            sh.fill_rrect(q[0], q[1], q[2], q[3], rad);
            sh.dc.set_color(stroke);
            sh.stroke_rrect(q[0], q[1], q[2], q[3], rad, hair);
        }
        if size < 5 << 8 || lod {
            continue;
        }
        let pad = (NOTE_PAD * s) as i32;
        let (iw, ih) = (q[2] - pad * 2, q[3] - pad * 2);
        if iw <= 0 || ih <= 0 {
            continue;
        }
        g.text.set_font(font, size);
        let line_h = (g.text.ascent() + g.text.descent()) * 6 / 5;
        let (text, color) = if n.text.trim().is_empty() { ("Notiz", f.color(T_TEXT3)) } else { (n.text.as_str(), f.color(T_TEXT)) };
        g.dc.clip_push(q[0] + pad, q[1] + pad, iw, ih);
        g.dc.set_color(color);
        let mut y = q[1] + pad + g.text.ascent();
        for line in wrap(g, text, iw) {
            if y - g.text.ascent() > q[1] + q[3] {
                break;
            }
            g.text.draw(g.dc, g.sf, q[0] + pad, y, line.as_bytes());
            y += line_h;
        }
        g.dc.clip_pop();
    }
}

// greedy word wrap at w px (the font of g.text), paragraphs at the line ends
fn wrap(g: &mut Gfx, text: &str, w: i32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let cand = if line.is_empty() { String::from(word) } else { alloc::format!("{} {}", line, word) };
            if !line.is_empty() && g.text.width(&cand) > w {
                out.push(core::mem::take(&mut line));
                line = String::from(word);
            } else {
                line = cand;
            }
        }
        out.push(line);
    }
    out
}

fn paint_links(g: &mut Gfx, f: &DgFrame, sc: &Scene, edges: &[u32], lod: bool) {
    for &i in edges {
        let (Some(l), Some(rt)) = (sc.links.get(i as usize), f.geo.routes.get(i as usize)) else { continue };
        let (_, w, dash) = cable_style(l.cable);
        if !f.route_visible(rt, w + 6.0) {
            continue;
        }
        let pts = &rt.pts;
        let argb = cable_color(l.cable, f.dark, l.down, f.bg);
        let len = dg_length(pts);
        if !(len > 0.5) {
            continue;
        }
        if lod {
            dg_hairline(g, &f.vp, pts, argb);
            continue;
        }
        match l.cable {
            CableKind::Wan => {
                for k in [-1.6f32, 1.6] {
                    dg_stroke_edge(g, &f.vp, &offset(pts, k), w, argb, LCAP_FLAT, LCAP_FLAT, dash);
                }
            }
            CableKind::Serial if len > 40.0 => {
                // the line with a gap in the middle, a zigzag across it
                let (mid, u) = at_len(pts, len * 0.5);
                let nrm = [-u[1], u[0]];
                let at = |t: f32, n: f32| [mid[0] + u[0] * t + nrm[0] * n, mid[1] + u[1] * t + nrm[1] * n];
                dg_stroke_edge(g, &f.vp, &cut(pts, 0.0, len * 0.5 - 6.0), w, argb, LCAP_FLAT, LCAP_FLAT, dash);
                dg_stroke_edge(g, &f.vp, &cut(pts, len * 0.5 + 6.0, len), w, argb, LCAP_FLAT, LCAP_FLAT, dash);
                let zig = [at(-6.0, 0.0), at(-2.0, 5.0), at(2.0, -5.0), at(6.0, 0.0)];
                dg_stroke_edge(g, &f.vp, &zig, w, argb, LCAP_FLAT, LCAP_FLAT, DASH_SOLID);
            }
            _ => dg_stroke_edge(g, &f.vp, pts, w, argb, LCAP_FLAT, LCAP_FLAT, dash),
        }
    }
}

fn paint_devices(g: &mut Gfx, f: &DgFrame, sc: &Scene, syms: &Symbols, nodes: &[u32], lod: bool) {
    let zoom = f.vp.zoom;
    let s = f.vp.scale() as f32;
    let (font, size) = f.font(TS_BODY);
    let named = zoom >= 0.5 && !lod;
    g.text.set_font(font, size);
    let (asc, desc) = (g.text.ascent(), g.text.descent());
    let text_c = f.color(T_TEXT);
    let plate = (f.bg & 0x00FF_FFFF) | 0xD800_0000;
    let hair = ((f.vp.dpi + 24) / 96).max(1);
    // the devices follow the notes among the nodes
    let first = sc.notes.len() as u32;
    for d in nodes.iter().filter(|&&i| i >= first).filter_map(|&i| sc.devs.get((i - first) as usize)) {
        let q = px_rect(f, d.rect);
        // the name below the box, centred
        let gap = (LABEL_GAP * s) as i32;
        let tw = if named { g.text.width(&d.name) } else { 0 };
        let pad = (3.0 * s) as i32;
        let lx = q[0] + (q[2] - tw) / 2;
        let ly = q[1] + q[3] + gap;
        let mut ext = [q[0], q[1], q[0] + q[2], q[1] + q[3]];
        if named {
            ext = [ext[0].min(lx - pad), ext[1], ext[2].max(lx + tw + pad), ext[3].max(ly + asc + desc + 1)];
        }
        if !touches(g, ext) {
            continue;
        }
        if zoom < 0.3 || d.sym.is_none() {
            // far away (or an unknown model): a square in the category colour
            let side = q[3].min(q[2]);
            let (x, y) = (q[0] + (q[2] - side) / 2, q[1] + (q[3] - side) / 2);
            let tint = TINTS[d.tint] | 0xFF00_0000;
            let fill = if f.dark { color_mix(tint, 0xFFFF_FFFF, 56) } else { tint };
            let mut sh = g.shapes();
            sh.dc.set_color(if d.off { color_mix(fill, f.bg, 140) } else { fill });
            sh.fill_rrect(x, y, side, side, (side / 6).max(1));
            if d.sym.is_none() {
                sh.dc.set_color(f.color(T_TEXT2));
                sh.stroke_rrect(x, y, side, side, (side / 6).max(1), hair);
            }
        } else if let Some(i) = d.sym {
            let w = q[2];
            let h = syms.height(i, w);
            syms.draw(g, i, q[0], q[1] + (q[3] - h) / 2, w, d.tint, d.off);
        }
        if named && size >= 4 << 8 {
            {
                let mut sh = g.shapes();
                sh.dc.set_color(plate);
                sh.fill_rrect(lx - pad, ly, tw + pad * 2, asc + desc, (2.0 * s) as i32);
            }
            g.text.set_font(font, size);
            g.dc.set_color(text_c);
            g.text.draw(g.dc, g.sf, lx, ly + asc, d.name.as_bytes());
        }
    }
}

// the ends of the cables: the LEDs (from 50 %), the port names (from 100 %)
fn paint_ends(g: &mut Gfx, f: &DgFrame, sc: &Scene, leds: &BTreeMap<u32, [Led; 2]>, edges: &[u32]) {
    let zoom = f.vp.zoom;
    if zoom < 0.5 {
        return;
    }
    let s = f.vp.scale() as f32;
    let hair = ((f.vp.dpi + 24) / 96).max(1);
    let named = zoom >= 1.0 - 1e-3;
    let (font, size) = f.font(TS_CAPTION);
    g.text.set_font(font, size);
    let (asc, desc) = (g.text.ascent(), g.text.descent());
    let plate = (f.bg & 0x00FF_FFFF) | 0xC800_0000;
    let text_c = f.color(T_TEXT2);
    let ring = led_ring(f.color(T_TEXT), f.bg, f.dark);
    for &i in edges {
        let (Some(l), Some(rt)) = (sc.links.get(i as usize), f.geo.routes.get(i as usize)) else { continue };
        // LEDs and names stay within REACH of the cable
        if rt.pts.len() < 2 || !f.route_visible(rt, REACH / 4.0) {
            continue;
        }
        let pts = rt.pts.as_slice();
        let len = dg_length(pts);
        let state = leds.get(&l.id).copied().unwrap_or([Led::NoPower; 2]);
        // the end at the route's start is end a; end b walks the route backwards
        for k in 0..2 {
            let along = |d: f32| if k == 0 { at_len(pts, d) } else { at_len_back(pts, d) };
            let (c, u) = along(LED_AT.min(len / 3.0));
            let ri = led_radius(s);
            let p = f.vp.to_screen(c);
            let bb = [fl(p[0]) - ri - 1, fl(p[1]) - ri - 1, fl(p[0]) + ri + 2, fl(p[1]) + ri + 2];
            if touches(g, bb) {
                let (x, y) = (fl(p[0] + 0.5) - ri, fl(p[1] + 0.5) - ri);
                let mut sh = g.shapes();
                sh.dc.set_color(led_color(state[k], &|t| f.color(t), f.bg));
                sh.fill_rrect(x, y, ri * 2, ri * 2, ri);
                sh.dc.set_color(ring);
                sh.stroke_rrect(x, y, ri * 2, ri * 2, ri, hair);
            }
            // short cables leave their ports unnamed (the names would cover each other)
            if !named || size < 4 << 8 || len < NAME_AT * 2.0 + 60.0 {
                continue;
            }
            // the port name beside the cable (further out below a device: its name is there)
            let (a, _) = along(NAME_AT + if u[1] > 0.6 { 14.0 } else { 0.0 });
            let side = [-u[1] * l.sides[k], u[0] * l.sides[k]];
            let name = &l.names[k];
            let tw = g.text.width(name);
            let th = asc + desc;
            let q = f.vp.to_screen([a[0] + side[0] * NAME_SIDE, a[1] + side[1] * NAME_SIDE]);
            // the label's box hangs off that point on the side away from the cable
            let x = if side[0] >= 0.0 { q[0] } else { q[0] - tw as f32 };
            let y = if side[1] >= 0.0 { q[1] } else { q[1] - th as f32 };
            let (x, y) = (fl(x + 0.5), fl(y + 0.5));
            let pad = (2.0 * s) as i32;
            if !touches(g, [x - pad, y, x + tw + pad, y + th]) {
                continue;
            }
            {
                let mut sh = g.shapes();
                sh.dc.set_color(plate);
                sh.fill_rrect(x - pad, y, tw + pad * 2, th, (2.0 * s) as i32);
            }
            g.text.set_font(font, size);
            g.dc.set_color(text_c);
            g.text.draw(g.dc, g.sf, x, y + asc, name.as_bytes());
        }
    }
}
