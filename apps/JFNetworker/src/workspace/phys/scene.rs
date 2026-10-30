// scene -- what the physical view shows of one place, flattened for ui::diagram: the nodes
// (places, devices, the ways out to other places), hidden ports that keep parallel cables
// apart, the edges (cables, or their sums between places), and what the painter draws
//
//   Scene::build(&project, &tree, &symbols, at, rev)   the view of place `at` (TOP: every site)
//   scene.model() -> DModel        borrowed by ui.diagram every frame
//   scene.item(key) / cable(key)   what a node or an edge shows; label() for screen readers
//   level_of(&tree, at)            which view a place gets (a desk: its room's)
//   place_key / exit_key / key_place / key_exit / key_device   the diagram keys
//
// The levels and their scales (world units per decimetre, K_*): every site on one map (sites);
// a site's map (its buildings); a building's floors as bands on top of each other (their rooms
// drawn small inside); a floor's plan (its rooms with their racks, desks and devices); a room's
// plan (racks, desks, devices at their spot, on a desk in a row, and those without a spot in a
// tray below the room); a rack's front (its units numbered from 1 at the bottom, the devices in
// their slots, beside it the rack devices of the room that are in no rack).  A cable whose far
// end lies elsewhere leads to a node on the right of the plan that names that place (a double
// click goes there); between sites, buildings and floors the cables are summed per pair
// ("3 Kabel · 120–152 m").  Lengths and limits come from the project (jfn_model::phys), so they
// follow a drag at once; a cable over its limit, or with optics that do not fit, is marked.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::catalog::{self, Form};
use jfn_model::phys::{self, PlaceKind, Tree, Where};
use jfn_model::{CableKind, Project};
use ui::*;

use crate::app::tr::{t, tf};
use crate::workspace::icons::{tint_of, Symbols};

/// the top of the tree: every site
pub(crate) const TOP: u32 = u32::MAX;

/// world units per decimetre of each level
pub(crate) const K_TOP: f32 = 0.1;
pub(crate) const K_SITE: f32 = 0.4;
pub(crate) const K_BUILDING: f32 = 2.0;
pub(crate) const K_FLOOR: f32 = 2.0;
pub(crate) const K_ROOM: f32 = 8.0;
/// the rack's front: one rack unit, the frame above and below the units, the units' x and width
pub(crate) const RU: f32 = 18.0;
pub(crate) const RACK_PAD: f32 = 22.0;
pub(crate) const RACK_X: f32 = 34.0;
pub(crate) const RACK_W: f32 = 260.0;
/// the tray beside a rack (x) and below a room (gap)
pub(crate) const RACK_TRAY_X: f32 = 380.0;
const TRAY_GAP: f32 = 48.0;
/// device boxes: in a room, on a desk, on a floor's plan
const DEV_ROOM: [f32; 2] = [40.0, 30.0];
const DEV_DESK: [f32; 2] = [28.0, 21.0];
const DEV_FLOOR: [f32; 2] = [20.0, 15.0];
/// the distance of parallel cables (world units)
const PARALLEL: f32 = 11.0;
/// a building's floors: the depth factor of a band, the gap between bands
const BAND_K: f32 = 0.6;
const BAND_GAP: f32 = 34.0;

/// which view a place has
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Level {
    #[default]
    Top,
    Site,
    Building,
    Floor,
    Room,
    Rack,
}

/// the view of a place and the place it shows (a desk: its room)
pub(crate) fn level_of(tree: &Tree, at: u32) -> (Level, u32) {
    let Some(x) = tree.get(at) else { return (Level::Top, TOP) };
    match x.kind {
        PlaceKind::Site => (Level::Site, at),
        PlaceKind::Building => (Level::Building, at),
        PlaceKind::Floor => (Level::Floor, at),
        PlaceKind::Room => (Level::Room, at),
        PlaceKind::Rack => (Level::Rack, at),
        PlaceKind::Desk => (Level::Room, x.parent),
    }
}

pub(crate) fn place_key(id: u32) -> u64 {
    2 << 48 | id as u64
}
pub(crate) fn exit_key(id: u32) -> u64 {
    3 << 48 | id as u64
}
fn sum_key(k: usize) -> u64 {
    4 << 48 | k as u64
}
fn port_key(link: u32, end: u64) -> u64 {
    5 << 48 | (link as u64) << 1 | end
}
pub(crate) fn key_place(k: u64) -> Option<u32> {
    if k >> 48 == 2 { Some(k as u32) } else { None }
}
pub(crate) fn key_exit(k: u64) -> Option<u32> {
    if k >> 48 == 3 { Some(k as u32) } else { None }
}
/// a device or a cable (project ids)
pub(crate) fn key_device(k: u64) -> Option<u32> {
    if k >> 32 == 0 { Some(k as u32) } else { None }
}

/// what a node is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum What {
    Place(PlaceKind),
    Device,
    /// the way to another place (its id)
    Exit(u32),
}

/// a node to paint
pub(crate) struct SItem {
    pub key: u64,
    pub rect: [f32; 4],
    pub what: What,
    pub name: String,
    /// the second line: size, units, model, slot
    pub sub: String,
    pub sym: Option<usize>,
    pub tint: usize,
    pub off: bool,
    /// a rack: units taken and its height
    pub fill: Option<(u16, u16)>,
    /// a cable from here is too long or its optics do not fit
    pub warn: bool,
    /// what lies inside, drawn small (a floor band's rooms)
    pub inner: Vec<([f32; 4], String)>,
    /// for screen readers and scripts
    pub sem: String,
}

/// an edge to paint (one cable, or the sum of cables between two places)
pub(crate) struct SCable {
    pub link: Option<u32>,
    pub cable: CableKind,
    pub label: String,
    /// where along the line its label sits (parallel cables stagger theirs)
    pub at: f32,
    pub bad: bool,
    pub down: bool,
    pub sem: String,
}

/// the rack's front: its height, where its units start
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RackGeo {
    pub units: u16,
}

#[derive(Default)]
pub(crate) struct Scene {
    pub nodes: Vec<DNode>,
    pub ports: Vec<DPort>,
    pub edges: Vec<DEdge>,
    pub wps: Vec<[f32; 2]>,
    pub ver: u32,
    pub items: Vec<SItem>,
    pub cables: Vec<SCable>,
    pub level: Level,
    /// the place shown (TOP: every site)
    pub at: u32,
    /// world units per decimetre
    pub k: f32,
    /// the outline of the place shown (x, y, w, h) and its name
    pub frame: Option<[f32; 4]>,
    pub frame_name: String,
    /// the tray of devices without a spot (a room) or of the rack devices of the room (a rack)
    pub tray: Option<[f32; 4]>,
    pub tray_name: String,
    pub rack: Option<RackGeo>,
    /// what it was built from: the document's rev, the place
    pub built: (u32, u32),
    idx: BTreeMap<u64, usize>,
    cidx: BTreeMap<u64, usize>,
}

/// "10 × 8 m", "0,6 × 1 m"
pub(crate) fn size_text(w: u32, d: u32) -> String {
    format!("{} \u{d7} {} m", dm_text(w as i32), dm_text(d as i32))
}

/// decimetres as metres with the language's decimal mark ("2,5", "10")
pub(crate) fn dm_text(v: i32) -> String {
    let (sign, a) = if v < 0 { ("-", -(v as i64)) } else { ("", v as i64) };
    if a % 10 == 0 {
        format!("{}{}", sign, a / 10)
    } else {
        format!("{}{}{}{}", sign, a / 10, if crate::app::tr::en() { '.' } else { ',' }, a % 10)
    }
}

/// the name of a place as the view shows it (the default tree in the window's language)
pub(crate) fn place_name(tree: &Tree, id: u32) -> String {
    match tree.get(id) {
        Some(x) if tree.is_default() && (1..=4).contains(&x.id) => String::from(t(phys::DEFAULT_NAMES[x.id as usize - 1])),
        Some(x) => x.name.clone(),
        None => String::from(t("Alle Standorte")),
    }
}

/// "Standort › Gebäude › Raum" in the view's language
pub(crate) fn path_text(tree: &Tree, id: u32) -> String {
    let parts: Vec<String> = tree.path(id).into_iter().map(|i| place_name(tree, i)).collect();
    parts.join(" \u{203a} ")
}

/// where a device stands, as a sentence: "Raum (ohne Platz)", "Rack 1, HE 12–13", "Tisch 2"
pub(crate) fn where_text(p: &Project, tree: &Tree, dev: u32) -> String {
    let Some(d) = p.device(dev) else { return String::new() };
    let w = tree.whereis(d);
    let h = p.model_of(dev).map_or(1, |m| catalog::rack_units(m).max(1)) as u16;
    if w.rack != 0 {
        let rack = place_name(tree, w.rack);
        return match (w.unit, h) {
            (0, _) => tf("{} (ohne Höheneinheit)", &[&rack]),
            (u, 1) => tf("{}, HE {}", &[&rack, &u.to_string()]),
            (u, h) => tf("{}, HE {}–{}", &[&rack, &u.to_string(), &(u + h - 1).to_string()]),
        };
    }
    if w.desk != 0 {
        return place_name(tree, w.desk);
    }
    let room = place_name(tree, w.room);
    match w.at {
        Some((x, y)) => tf("{} bei {} m Ost, {} m Süd", &[&room, &dm_text(x), &dm_text(y)]),
        None => tf("{} (ohne Platz)", &[&room]),
    }
}

/// the length of a cable as the view writes it: "7 m", "132 m (von Hand)"
pub(crate) fn len_text(len: jfn_model::Len) -> String {
    if len.auto { tf("{} m", &[&len.m.to_string()]) } else { tf("{} m (von Hand)", &[&len.m.to_string()]) }
}

/// a cable's length, limit and whether it is too long or its optics do not fit, from the project
pub(crate) struct CableFacts {
    pub len: jfn_model::Len,
    pub reach: Option<jfn_model::Reach>,
    pub too_long: bool,
    pub optics: Option<phys::OpticFault>,
}

pub(crate) fn facts(p: &Project, tree: &Tree, l: &jfn_model::Link) -> CableFacts {
    let len = phys::length(p, tree, l);
    let reach = phys::link_reach(p, l);
    let oa = p.resolve_port(&l.a).ok().and_then(|s| s.optic);
    let ob = p.resolve_port(&l.b).ok().and_then(|s| s.optic);
    let optics = phys::optic_fault(l.cable, oa, ob);
    CableFacts { len, reach, too_long: reach.is_some_and(|r| len.m > r.max_m), optics }
}

impl Scene {
    pub(crate) fn model(&self) -> DModel<'_> {
        DModel { nodes: &self.nodes, ports: &self.ports, edges: &self.edges, wps: &self.wps, ver: self.ver }
    }
    pub(crate) fn item(&self, key: u64) -> Option<&SItem> {
        self.items.get(*self.idx.get(&key)?)
    }
    pub(crate) fn cable(&self, key: u64) -> Option<&SCable> {
        self.cables.get(*self.cidx.get(&key)?)
    }
    /// the label of a node for screen readers and scripts
    pub(crate) fn label(&self, key: u64) -> String {
        self.item(key).map_or_else(|| String::from("–"), |x| x.sem.clone())
    }
    pub(crate) fn edge_label(&self, key: u64) -> String {
        self.cable(key).map_or_else(|| String::from("–"), |x| x.sem.clone())
    }

    fn push(&mut self, it: SItem, flags: u32) {
        self.idx.insert(it.key, self.items.len());
        self.nodes.push(DNode { key: it.key, rect: it.rect, ports: 0..0, flags });
        self.items.push(it);
    }

    /// the view of place `at` (TOP: every site)
    pub(crate) fn build(&mut self, p: &Project, tree: &Tree, syms: &Symbols, at: u32, rev: u32) {
        self.nodes.clear();
        self.ports.clear();
        self.edges.clear();
        self.wps.clear();
        self.items.clear();
        self.cables.clear();
        self.idx.clear();
        self.cidx.clear();
        self.frame = None;
        self.tray = None;
        self.rack = None;
        self.frame_name.clear();
        self.tray_name.clear();
        let (level, shown) = level_of(tree, at);
        self.level = level;
        self.at = shown;
        let wh: BTreeMap<u32, Where> = p.devices.iter().map(|d| (d.id, tree.whereis(d))).collect();
        match level {
            Level::Top => self.places_map(p, tree, &wh, 0, K_TOP),
            Level::Site => self.places_map(p, tree, &wh, shown, K_SITE),
            Level::Building => self.building(p, tree, &wh, shown),
            Level::Floor => self.floor(p, tree, syms, &wh, shown),
            Level::Room => self.room(p, tree, syms, &wh, shown),
            Level::Rack => self.rack_front(p, tree, syms, &wh, shown),
        }
        self.cables_of(p, tree, &wh);
        self.ver = self.ver.wrapping_add(1);
        self.built = (rev, at);
    }

    // a device node
    fn device(&mut self, p: &Project, tree: &Tree, syms: &Symbols, dev: u32, centre: [f32; 2], size: [f32; 2]) {
        let Some(d) = p.device(dev) else { return };
        let m = catalog::model(&d.model);
        let rect = [centre[0] - size[0] * 0.5, centre[1] - size[1] * 0.5, size[0], size[1]];
        let sub = m.map_or_else(String::new, |m| String::from(t(m.name)));
        let sem = tf("{}, {}", &[&d.name, &where_text(p, tree, dev)]);
        self.push(
            SItem {
                key: dev as u64,
                rect,
                what: What::Device,
                name: d.name.clone(),
                sub,
                sym: m.and_then(|m| syms.index(m.symbol)),
                tint: m.map_or(0, |m| tint_of(m.category)),
                off: d.off,
                fill: None,
                warn: false,
                inner: Vec::new(),
                sem,
            },
            0,
        );
    }

    // a place node
    fn place(&mut self, p: &Project, tree: &Tree, wh: &BTreeMap<u32, Where>, id: u32, rect: [f32; 4], flags: u32) {
        let Some(x) = tree.get(id) else { return };
        let name = place_name(tree, id);
        let devs = wh.values().filter(|w| tree.path(w.place()).contains(&id)).count();
        let (w, d) = tree.size(id);
        let (sub, fill) = match x.kind {
            PlaceKind::Rack => {
                let units = tree.units(id);
                let used: u16 = p
                    .devices
                    .iter()
                    .filter(|d| wh.get(&d.id).is_some_and(|w| w.rack == id && w.unit > 0))
                    .map(|d| p.model_of(d.id).map_or(1, |m| catalog::rack_units(m).max(1)) as u16)
                    .sum();
                (tf("{} von {} HE", &[&used.to_string(), &units.to_string()]), Some((used, units)))
            }
            PlaceKind::Floor => (tf("Etage {}", &[&x.level.to_string()]), None),
            _ => (size_text(w, d), None),
        };
        let count = if devs == 1 { String::from(t("1 Gerät")) } else { tf("{} Geräte", &[&devs.to_string()]) };
        let sem = tf("{} {}, {}", &[t(x.kind.title()), &name, &count]);
        self.push(SItem { key: place_key(id), rect, what: What::Place(x.kind), name, sub, sym: None, tint: 0, off: false, fill, warn: false, inner: Vec::new(), sem }, flags);
    }

    // sites on the map of all sites (parent 0) or the buildings of a site
    fn places_map(&mut self, p: &Project, tree: &Tree, wh: &BTreeMap<u32, Where>, parent: u32, k: f32) {
        self.k = k;
        if parent != 0 {
            let (w, d) = tree.size(parent);
            self.frame = Some([0.0, 0.0, w as f32 * k, d as f32 * k]);
            self.frame_name = place_name(tree, parent);
        }
        let kids: Vec<(u32, i32, i32)> = tree.children(parent).iter().map(|x| (x.id, x.x_dm, x.y_dm)).collect();
        for (id, x, y) in kids {
            let (w, d) = tree.size(id);
            self.place(p, tree, wh, id, [x as f32 * k, y as f32 * k, (w as f32 * k).max(24.0), (d as f32 * k).max(18.0)], 0);
        }
    }

    // a building's floors as bands, the highest on top, their rooms small inside
    fn building(&mut self, p: &Project, tree: &Tree, wh: &BTreeMap<u32, Where>, b: u32) {
        self.k = K_BUILDING;
        let (w, d) = tree.size(b);
        let bw = w as f32 * K_BUILDING;
        let bh = (d as f32 * BAND_K).max(80.0);
        let mut floors: Vec<(u32, i32)> = tree.children(b).iter().filter(|x| x.kind == PlaceKind::Floor).map(|x| (x.id, x.level)).collect();
        floors.sort_by_key(|f| -f.1);
        for (i, (f, _)) in floors.iter().enumerate() {
            let y = i as f32 * (bh + BAND_GAP);
            self.place(p, tree, wh, *f, [0.0, y, bw, bh], DN_LOCKED);
            let rooms: Vec<([f32; 4], String)> = tree
                .children(*f)
                .iter()
                .map(|r| {
                    let (rw, rd) = tree.size(r.id);
                    ([r.x_dm as f32 * K_BUILDING, y + r.y_dm as f32 * BAND_K, rw as f32 * K_BUILDING, rd as f32 * BAND_K], place_name(tree, r.id))
                })
                .collect();
            if let Some(it) = self.items.last_mut() {
                it.inner = rooms;
            }
        }
        self.frame_name = place_name(tree, b);
    }

    // a floor's plan: rooms, their racks and desks, the devices standing free or without a spot
    fn floor(&mut self, p: &Project, tree: &Tree, syms: &Symbols, wh: &BTreeMap<u32, Where>, f: u32) {
        let k = K_FLOOR;
        self.k = k;
        let (w, d) = tree.size(f);
        self.frame = Some([0.0, 0.0, w as f32 * k, d as f32 * k]);
        self.frame_name = place_name(tree, f);
        let rooms: Vec<u32> = tree.children(f).iter().filter(|x| x.kind == PlaceKind::Room).map(|x| x.id).collect();
        for &r in &rooms {
            let x = tree.get(r).map_or((0, 0), |x| (x.x_dm, x.y_dm));
            let (rw, rd) = tree.size(r);
            self.place(p, tree, wh, r, [x.0 as f32 * k, x.1 as f32 * k, rw as f32 * k, rd as f32 * k], 0);
        }
        for &r in &rooms {
            let o = tree.get(r).map_or((0, 0), |x| (x.x_dm, x.y_dm));
            let kids: Vec<(u32, i32, i32)> = tree.children(r).iter().map(|x| (x.id, x.x_dm, x.y_dm)).collect();
            for (id, x, y) in kids {
                let (cw, cd) = tree.size(id);
                self.place(p, tree, wh, id, [(o.0 + x) as f32 * k, (o.1 + y) as f32 * k, cw as f32 * k, cd as f32 * k], DN_LOCKED);
            }
            // devices free in the room (at their spot) or without one (a row from the top left)
            let mut row = 0usize;
            for dv in &p.devices {
                let Some(w) = wh.get(&dv.id).filter(|w| w.room == r && w.rack == 0 && w.desk == 0) else { continue };
                let c = match w.at {
                    Some((x, y)) => [(o.0 + x) as f32 * k, (o.1 + y) as f32 * k],
                    None => {
                        let (rw, _) = tree.size(r);
                        let per = ((rw as f32 * k - 8.0) / (DEV_FLOOR[0] + 6.0)).max(1.0) as usize;
                        let c = [o.0 as f32 * k + 6.0 + DEV_FLOOR[0] * 0.5 + (row % per) as f32 * (DEV_FLOOR[0] + 6.0), o.1 as f32 * k + 18.0 + DEV_FLOOR[1] * 0.5 + (row / per) as f32 * (DEV_FLOOR[1] + 6.0)];
                        row += 1;
                        c
                    }
                };
                self.device(p, tree, syms, dv.id, c, DEV_FLOOR);
            }
        }
    }

    // a room's plan: racks, desks with their devices in a row, devices at their spot, the tray
    fn room(&mut self, p: &Project, tree: &Tree, syms: &Symbols, wh: &BTreeMap<u32, Where>, r: u32) {
        let k = K_ROOM;
        self.k = k;
        let (w, d) = tree.size(r);
        let frame = [0.0, 0.0, w as f32 * k, d as f32 * k];
        self.frame = Some(frame);
        self.frame_name = place_name(tree, r);
        let kids: Vec<(u32, i32, i32)> = tree.children(r).iter().map(|x| (x.id, x.x_dm, x.y_dm)).collect();
        for &(id, x, y) in &kids {
            let (cw, cd) = tree.size(id);
            self.place(p, tree, wh, id, [x as f32 * k, y as f32 * k, cw as f32 * k, cd as f32 * k], 0);
        }
        // the devices on each desk, in a row inside it
        for &(id, x, y) in &kids {
            if tree.get(id).is_none_or(|x| x.kind != PlaceKind::Desk) {
                continue;
            }
            let (cw, _) = tree.size(id);
            let on: Vec<u32> = p.devices.iter().filter(|dv| wh.get(&dv.id).is_some_and(|w| w.desk == id)).map(|dv| dv.id).collect();
            let per = ((cw as f32 * k - 6.0) / (DEV_DESK[0] + 4.0)).max(1.0) as usize;
            for (i, dev) in on.into_iter().enumerate() {
                let c = [x as f32 * k + 4.0 + DEV_DESK[0] * 0.5 + (i % per) as f32 * (DEV_DESK[0] + 4.0), y as f32 * k + 4.0 + DEV_DESK[1] * 0.5 + (i / per) as f32 * (DEV_DESK[1] + 4.0)];
                self.device(p, tree, syms, dev, c, DEV_DESK);
            }
        }
        // free devices at their spot; the others in the tray below the room
        let tray: Vec<u32> = p.devices.iter().filter(|dv| wh.get(&dv.id).is_some_and(|w| w.room == r && w.rack == 0 && w.desk == 0 && w.at.is_none())).map(|dv| dv.id).collect();
        for dv in &p.devices {
            if let Some(&Where { at: Some((x, y)), rack: 0, desk: 0, room, .. }) = wh.get(&dv.id)
                && room == r
            {
                self.device(p, tree, syms, dv.id, [x as f32 * k, y as f32 * k], DEV_ROOM);
            }
        }
        let per = ((frame[2] - 8.0) / (DEV_ROOM[0] + 12.0)).max(1.0) as usize;
        let rows = tray.len().div_ceil(per).max(1);
        let ty = frame[3] + TRAY_GAP;
        self.tray = Some([0.0, ty, frame[2], rows as f32 * (DEV_ROOM[1] + 22.0) + 26.0]);
        self.tray_name = String::from(t("Im Raum, ohne Platz"));
        for (i, dev) in tray.into_iter().enumerate() {
            let c = [8.0 + DEV_ROOM[0] * 0.5 + (i % per) as f32 * (DEV_ROOM[0] + 12.0), ty + 24.0 + DEV_ROOM[1] * 0.5 + (i / per) as f32 * (DEV_ROOM[1] + 22.0)];
            self.device(p, tree, syms, dev, c, DEV_ROOM);
        }
    }

    // a rack's front: its devices in their slots, the rack devices of the room in no rack beside it
    fn rack_front(&mut self, p: &Project, tree: &Tree, syms: &Symbols, wh: &BTreeMap<u32, Where>, rack: u32) {
        self.k = 1.0;
        let units = tree.units(rack);
        self.rack = Some(RackGeo { units });
        let h = units as f32 * RU + 2.0 * RACK_PAD;
        self.frame = Some([0.0, 0.0, RACK_X * 2.0 + RACK_W, h]);
        self.frame_name = place_name(tree, rack);
        let room = tree.up(rack, PlaceKind::Room).unwrap_or(0);
        for dv in &p.devices {
            let Some(w) = wh.get(&dv.id).filter(|w| w.rack == rack) else { continue };
            let hu = p.model_of(dv.id).map_or(1, |m| catalog::rack_units(m).max(1)) as u16;
            let unit = if w.unit == 0 { 1 } else { w.unit };
            let top = unit + hu - 1;
            let y = RACK_PAD + (units as f32 - top as f32) * RU;
            self.device(p, tree, syms, dv.id, [RACK_X + RACK_W * 0.5, y + hu as f32 * RU * 0.5], [RACK_W, hu as f32 * RU]);
        }
        // the room's rack devices that are in no rack
        let free: Vec<u32> = p
            .devices
            .iter()
            .filter(|dv| wh.get(&dv.id).is_some_and(|w| w.room == room && w.rack == 0) && p.model_of(dv.id).is_some_and(|m| catalog::form(m) == Form::Rack))
            .map(|dv| dv.id)
            .collect();
        let rows = free.len().max(1);
        self.tray = Some([RACK_TRAY_X, 0.0, 240.0, rows as f32 * (RU + 8.0) + 34.0]);
        self.tray_name = String::from(t("Im Raum, in keinem Rack"));
        for (i, dev) in free.into_iter().enumerate() {
            let hu = p.model_of(dev).map_or(1, |m| catalog::rack_units(m).max(1)) as f32;
            self.device(p, tree, syms, dev, [RACK_TRAY_X + 120.0, 30.0 + i as f32 * (RU + 8.0) + RU * hu * 0.5], [220.0, RU * hu]);
        }
    }

    // the node a device's cables end at in this view, or the way out to where it is
    fn anchor(&self, tree: &Tree, w: &Where, dev: u32) -> Option<u64> {
        let here = self.at;
        let target = w.place();
        let path = tree.path(target);
        let visible = |k: u64| self.idx.contains_key(&k);
        match self.level {
            Level::Rack => None,
            Level::Room | Level::Floor => {
                if visible(dev as u64) {
                    return Some(dev as u64);
                }
                if w.rack != 0 && visible(place_key(w.rack)) {
                    return Some(place_key(w.rack));
                }
                if w.desk != 0 && visible(place_key(w.desk)) {
                    return Some(place_key(w.desk));
                }
                if self.level == Level::Floor && visible(place_key(w.room)) {
                    return Some(place_key(w.room));
                }
                self.remote(tree, &path, here)
            }
            Level::Building | Level::Site | Level::Top => {
                let inside = path.iter().find(|&&i| visible(place_key(i)));
                match inside {
                    Some(&i) => Some(place_key(i)),
                    None if self.level == Level::Top => None,
                    None => self.remote(tree, &path, here),
                }
            }
        }
    }

    // the way out towards a place elsewhere: the first place on its path that is not on the
    // path of the place shown
    fn remote(&self, tree: &Tree, path: &[u32], here: u32) -> Option<u64> {
        let mine = tree.path(here);
        path.iter().find(|i| !mine.contains(i)).map(|&i| exit_key(i))
    }

    // the cables: one edge each on plans of floors and rooms, summed per pair above; the ways out
    fn cables_of(&mut self, p: &Project, tree: &Tree, wh: &BTreeMap<u32, Where>) {
        if self.level == Level::Rack {
            return;
        }
        let summed = matches!(self.level, Level::Top | Level::Site | Level::Building);
        let mut pairs: BTreeMap<(u64, u64), Vec<(u32, CableFacts)>> = BTreeMap::new();
        let mut each: Vec<(u64, u64, u32, CableFacts)> = Vec::new();
        for l in &p.links {
            if l.cable == CableKind::Wireless {
                continue;
            }
            let (Some(wa), Some(wb)) = (wh.get(&l.a.dev), wh.get(&l.b.dev)) else { continue };
            let (Some(a), Some(b)) = (self.anchor(tree, wa, l.a.dev), self.anchor(tree, wb, l.b.dev)) else { continue };
            if a == b || (key_exit(a).is_some() && key_exit(b).is_some()) {
                continue;
            }
            let f = facts(p, tree, l);
            if summed {
                pairs.entry(if a < b { (a, b) } else { (b, a) }).or_default().push((l.id, f));
            } else {
                each.push((a, b, l.id, f));
            }
        }
        // the ways out: a column right of the plan
        let mut exits: Vec<u32> = Vec::new();
        for k in pairs.keys().flat_map(|&(a, b)| [a, b]).chain(each.iter().flat_map(|e| [e.0, e.1])) {
            if let Some(x) = key_exit(k)
                && !exits.contains(&x)
            {
                exits.push(x);
            }
        }
        let fr = self.frame.unwrap_or_else(|| self.bounds());
        let (ew, eh) = (180.0, 44.0);
        for (i, x) in exits.iter().enumerate() {
            let rect = [fr[0] + fr[2] + 70.0, fr[1] + i as f32 * (eh + 14.0), ew, eh];
            let name = place_name(tree, *x);
            let kind = tree.get(*x).map_or(PlaceKind::Room, |x| x.kind);
            let sub = path_text(tree, tree.get(*x).map_or(0, |x| x.parent));
            let sem = tf("Weg nach {} {}", &[t(kind.title()), &name]);
            self.push(SItem { key: exit_key(*x), rect, what: What::Exit(*x), name, sub, sym: None, tint: 0, off: false, fill: None, warn: false, inner: Vec::new(), sem }, DN_LOCKED);
        }
        let label_of = |p: &Project, id: u32| -> String { p.link(id).map_or_else(String::new, |l| crate::workspace::canvas::cables::ends_text(p, l)) };
        if summed {
            for (k, ((a, b), list)) in pairs.into_iter().enumerate() {
                let lens: Vec<u32> = list.iter().map(|x| x.1.len.m).collect();
                let (lo, hi) = (lens.iter().copied().min().unwrap_or(0), lens.iter().copied().max().unwrap_or(0));
                let bad = list.iter().filter(|x| x.1.too_long || x.1.optics.is_some()).count();
                let n = list.len();
                let span = if lo == hi { tf("{} m", &[&lo.to_string()]) } else { tf("{}–{} m", &[&lo.to_string(), &hi.to_string()]) };
                let mut label = if n == 1 { tf("1 Kabel · {}", &[&span]) } else { tf("{} Kabel · {}", &[&n.to_string(), &span]) };
                if bad > 0 {
                    label = tf("{} · {} zu lang oder falsch", &[&label, &bad.to_string()]);
                }
                let name = |key: u64| self.item(key).map_or_else(String::new, |x| x.name.clone());
                let sem = tf("Kabel zwischen {} und {}: {}", &[&name(a), &name(b), &label]);
                let cable = list.first().and_then(|x| p.link(x.0)).map_or(CableKind::CopperStraight, |l| l.cable);
                self.cidx.insert(sum_key(k + 1), self.cables.len());
                self.edges.push(DEdge { key: sum_key(k + 1), a, b, route: DR_STRAIGHT, wps: 0..0 });
                self.cables.push(SCable { link: None, cable, label, at: 0.5, bad: bad > 0, down: false, sem });
            }
            return;
        }
        // one edge per cable, parallel ones apart (hidden ports at the nodes' middles)
        let mut count: BTreeMap<(u64, u64), (usize, usize)> = BTreeMap::new();
        for e in &each {
            count.entry(if e.0 < e.1 { (e.0, e.1) } else { (e.1, e.0) }).or_default().0 += 1;
        }
        let mut ports: BTreeMap<u64, Vec<DPort>> = BTreeMap::new();
        let mut edges: Vec<DEdge> = Vec::new();
        for (a, b, id, f) in each {
            let Some(l) = p.link(id) else { continue };
            let pair = if a < b { (a, b) } else { (b, a) };
            let c = count.entry(pair).or_default();
            let nth = c.1 as f32 - (c.0 - 1) as f32 * 0.5;
            let off = nth * PARALLEL;
            let at = (0.5 + nth * 0.16).clamp(0.18, 0.82);
            c.1 += 1;
            let (ra, rb) = (self.item(a).map(|x| x.rect), self.item(b).map(|x| x.rect));
            let (Some(ra), Some(rb)) = (ra, rb) else { continue };
            let mid = |r: [f32; 4]| [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5];
            let (ma, mb) = (mid(ra), mid(rb));
            let (dx, dy) = (mb[0] - ma[0], mb[1] - ma[1]);
            let len = rt::fx::sqrt(dx * dx + dy * dy).max(0.001);
            let o = [-dy / len * off, dx / len * off];
            let pa = port_key(id, 0);
            let pb = port_key(id, 1);
            ports.entry(a).or_default().push(DPort { key: pa, at: [ra[2] * 0.5 + o[0], ra[3] * 0.5 + o[1]], flags: DPT_HIDDEN | DPT_USED });
            ports.entry(b).or_default().push(DPort { key: pb, at: [rb[2] * 0.5 + o[0], rb[3] * 0.5 + o[1]], flags: DPT_HIDDEN | DPT_USED });
            edges.push(DEdge { key: id as u64, a: pa, b: pb, route: DR_STRAIGHT, wps: 0..0 });
            let bad = f.too_long || f.optics.is_some();
            let mut label = len_text(f.len);
            if f.too_long
                && let Some(r) = f.reach
            {
                label = tf("{} – zu lang, erlaubt {} m", &[&label, &r.max_m.to_string()]);
            } else if f.optics.is_some() {
                label = tf("{} – Optik passt nicht", &[&label]);
            }
            let limit = match f.reach {
                Some(r) => tf("erlaubt {} m ({})", &[&r.max_m.to_string(), r.std]),
                None => String::from(t("ohne Grenze")),
            };
            let mut sem = tf("Kabel {}, {}, {}", &[&label_of(p, id), &len_text(f.len), &limit]);
            if f.too_long {
                sem = tf("{}, zu lang", &[&sem]);
            } else if f.optics.is_some() {
                sem = tf("{}, Optik passt nicht", &[&sem]);
            }
            self.cables.push(SCable { link: Some(id), cable: l.cable, label, at, bad, down: l.down, sem });
            if bad {
                for k in [a, b] {
                    if let Some(&i) = self.idx.get(&k) {
                        self.items[i].warn = true;
                    }
                }
            }
        }
        // the hidden ports: rebuild the nodes' port ranges (nodes are in self.nodes order)
        for n in self.nodes.iter_mut() {
            let first = self.ports.len() as u32;
            if let Some(v) = ports.remove(&n.key) {
                self.ports.extend(v);
            }
            n.ports = first..self.ports.len() as u32;
        }
        // the edges in the order of their cables (the painter pairs them by index)
        for e in edges {
            self.cidx.insert(e.key, self.edges.len());
            self.edges.push(e);
        }
    }

    /// what the view shows when it fits: the outline, the tray and every node (x0, y0, x1, y1)
    pub(crate) fn extent(&self) -> [f32; 4] {
        let mut b: Option<[f32; 4]> = None;
        let rects = self.frame.iter().chain(self.tray.iter()).copied().chain(self.nodes.iter().map(|n| n.rect));
        for r in rects {
            b = Some(match b {
                None => [r[0], r[1], r[0] + r[2], r[1] + r[3]],
                Some(c) => [c[0].min(r[0]), c[1].min(r[1] - 16.0), c[2].max(r[0] + r[2]), c[3].max(r[1] + r[3] + 16.0)],
            });
        }
        b.unwrap_or([0.0, 0.0, 400.0, 300.0])
    }

    // the box around every node (no frame)
    fn bounds(&self) -> [f32; 4] {
        let mut b: Option<[f32; 4]> = None;
        for n in &self.nodes {
            let r = n.rect;
            b = Some(match b {
                None => [r[0], r[1], r[0] + r[2], r[1] + r[3]],
                Some(c) => [c[0].min(r[0]), c[1].min(r[1]), c[2].max(r[0] + r[2]), c[3].max(r[1] + r[3])],
            });
        }
        let b = b.unwrap_or([0.0; 4]);
        [b[0], b[1], b[2] - b[0], b[3] - b[1]]
    }

    /// the place node whose rectangle holds a point, of the kinds given (the topmost)
    pub(crate) fn place_at(&self, pt: [f32; 2], kinds: &[PlaceKind]) -> Option<u32> {
        self.items.iter().rev().find_map(|it| match it.what {
            What::Place(k) if kinds.contains(&k) && inside(it.rect, pt) => key_place(it.key),
            _ => None,
        })
    }
}

/// a point inside a rectangle x, y, w, h
pub(crate) fn inside(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
}
