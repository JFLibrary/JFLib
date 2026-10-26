// phys -- the physical side of a project (P1): places -- sites, buildings, floors, rooms and in
// rooms racks and desks --, where every device stands, the length of every cable and the limit
// of its medium
//
//   PlaceKind, Place                 a place: kind, name, parent, position and size in
//                                    decimetres, a rack's height in rack units, a floor's storey
//   Spot                             where a device stands (Device::phys)
//   Tree::of(&project)               the places as a tree, the default one when the project has
//                                    none (default_places: Standort › Gebäude › Erdgeschoss ›
//                                    Raum, ids 1 to 4); get / children / path / up / abs / size /
//                                    level / units / default_room / is_default
//   tree.whereis(&device) -> Where   its room, rack (with the unit) or desk, its spot
//   tree.devices_in(p, place)        the devices standing in a place or anywhere below it
//   tree.rack_free(p, rack, unit, h, except) / rack_slot(p, rack, h)   free rack units
//   length(p, &tree, link) -> Len    a cable's length in metres: by hand (Link::len_m) or
//                                    automatic from the places of its ends (the rule below)
//   delay_ns(p, &tree, link)         the delay the simulator runs a cable with
//   reach(kind, kbps, optic)         the longest cable of a medium at a speed (Reach: metres and
//                                    the name of the standard); link_reach(p, link) for a cable
//                                    of the project; optic_fault(kind, a, b): optics that do not
//                                    fit each other or the fibre
//   issues(p)                        what does not fit: places, spots, rack units
//   ensure / add_place / rename_place / remove_place / set_place / place_device / set_len /
//   set_optic                        the edits of the physical view (the app makes undo steps
//                                    of them); PhysErr::msg says why one was refused
//
// The tree: sites (a campus, a town, positions on the map of all sites) hold buildings (on
// their site), buildings floors (storeys on top of each other, a building's footprint), floors
// rooms (on the floor plan), rooms racks and desks (on the room's plan).  Positions are the top
// left corner in the parent in decimetres (x to the east, y to the south), sizes width and
// depth in decimetres.  Place ids count on their own (not the project's next_id, so devices
// and cables keep their numbers).  A project without places has the default tree; the first
// edit of the physical view stores it (ensure).
//
// A device's Spot names its room, rack or desk (place 0, or a place that is none of these: the
// default room, the first room of the tree).  In a rack it takes `rack_units(model)` units
// from `unit` up (1 is the lowest; 0: none yet); in a room it stands at (x_dm, y_dm) when
// `placed` (else somewhere in the room: the middle, for the lengths), on a desk anywhere on it.
//
// Automatic length (metres, rounded up): an end stands at the middle of its rack, its desk, its
// spot or (without one) its room, at a height above its floor -- in a rack 0.1 m plus 44.45 mm
// per unit below it, on a desk 0.75 m, on the ceiling 2.8 m, else on the floor.  Two ends in
// one rack: a patch cable of 0.5 m plus their difference in height; on one desk: 2 m;
// otherwise the cable runs up to the trays under the ceiling (2.8 m), at right angles over the
// plans (|dx| + |dy| in the coordinates of the site or of all sites), 3.5 m per storey between
// floors (between buildings down to the ground floor and up again) and down to the other end,
// plus 10 % slack.  The provider's cloud stands nowhere: a cable to it is 3 m.  Radio has no
// cable (0).  So every cable of an older project, both ends in the default room without a spot,
// is 7 m long, 4 m to a device on the ceiling (an access point, a camera), 3 m to the cloud.
//
// Limits (reach, after the reach tables of IEEE 802.3; twisted pair as Cat6A, multimode fibre
// as OM3): copper 100 m from 10BASE-T to 10GBASE-T (30 m for 25G and 40G); multimode (SR
// optics) 1000BASE-SX 550 m, 10GBASE-SR 300 m, 25GBASE-SR 70 m, 40GBASE-SR4 100 m,
// 100GBASE-SR4 70 m; single-mode (LR optics) 1000BASE-LX 5 km, 10GBASE-LR, 25GBASE-LR,
// 40GBASE-LR4 and 100GBASE-LR4 10 km; console (RS-232 at 9600 bit/s) 15 m; serial (V.35 / EIA-530)
// 1250 m at 2.4 kbit/s down to 51 m at 64 kbit/s and 15 m above (T1 / E1); WAN lines and radio
// none.  A cable over its limit, or a fibre whose optics do not fit, stays down (jfn_sim phys.rs).
//
// Delay: a copper or fibre cable whose delay is the default of its kind (cable::default_delay_ns)
// and whose length comes from the plan -- set by hand, or both ends placed in the physical view
// (a place of their own or a spot) -- runs with 5 ns per metre; every other cable keeps its
// delay, so older projects run exactly as before.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::{record, wire_enum};

use crate::cable::{self, CableKind};
use crate::catalog::{self, Form, Optic};
use crate::msg::Msg;
use crate::msg;
use crate::project::{Device, Link, PortRef, Project};

wire_enum! {
    /// the kind of a place (the order is the depth in the tree)
    pub enum PlaceKind {
        Site = "site",
        Building = "building",
        Floor = "floor",
        Room = "room",
        Rack = "rack",
        Desk = "desk",
    }
}

impl PlaceKind {
    /// the German name (the app translates it)
    pub fn title(self) -> &'static str {
        // i18n
        match self {
            PlaceKind::Site => "Standort",
            PlaceKind::Building => "Gebäude",
            PlaceKind::Floor => "Etage",
            PlaceKind::Room => "Raum",
            PlaceKind::Rack => "Rack",
            PlaceKind::Desk => "Tisch",
        }
    }
    /// the kind of place it lies in (None: the top)
    pub fn parent_kind(self) -> Option<PlaceKind> {
        match self {
            PlaceKind::Site => None,
            PlaceKind::Building => Some(PlaceKind::Site),
            PlaceKind::Floor => Some(PlaceKind::Building),
            PlaceKind::Room => Some(PlaceKind::Floor),
            PlaceKind::Rack | PlaceKind::Desk => Some(PlaceKind::Room),
        }
    }
    /// the kinds of places that lie in it
    pub fn child_kinds(self) -> &'static [PlaceKind] {
        match self {
            PlaceKind::Site => &[PlaceKind::Building],
            PlaceKind::Building => &[PlaceKind::Floor],
            PlaceKind::Floor => &[PlaceKind::Room],
            PlaceKind::Room => &[PlaceKind::Rack, PlaceKind::Desk],
            PlaceKind::Rack | PlaceKind::Desk => &[],
        }
    }
    /// the size of a new place: width and depth in decimetres
    pub fn default_size(self) -> (u32, u32) {
        match self {
            PlaceKind::Site => (3000, 2000),
            PlaceKind::Building => (400, 250),
            PlaceKind::Floor => (0, 0),
            PlaceKind::Room => (100, 80),
            PlaceKind::Rack => (6, 10),
            PlaceKind::Desk => (16, 8),
        }
    }
    /// devices stand here (a room, a rack, a desk)
    pub fn holds_devices(self) -> bool {
        matches!(self, PlaceKind::Room | PlaceKind::Rack | PlaceKind::Desk)
    }
}

record! {
    /// a place of the physical view
    pub struct Place {
        pub id: u32,
        pub kind: PlaceKind,
        pub name: String,
        /// the place it lies in (0 for a site)
        pub parent: u32,
        /// the top left corner in the parent in decimetres (x to the east, y to the south);
        /// a site's on the map of all sites, floors have none
        pub x_dm: i32,
        pub y_dm: i32,
        /// width and depth in decimetres (0: the default of its kind; a floor: its building's)
        pub w_dm: u32,
        pub d_dm: u32,
        /// a rack's height in rack units (0: 42)
        pub units: u16,
        /// a floor's storey (0 the ground floor, 1 the first floor, -1 the basement)
        pub level: i32,
    }
}

record! {
    /// where a device stands in the physical view
    pub struct Spot {
        /// its room, rack or desk (0: the default room)
        pub place: u32,
        /// in a rack: the lowest rack unit it takes (1 at the bottom; 0: none yet)
        pub unit: u16,
        /// in a room: its middle in decimetres from the room's top left corner (when placed)
        pub x_dm: i32,
        pub y_dm: i32,
        pub placed: bool,
    }
}

/// the height of a rack without one of its own
pub const RACK_UNITS: u16 = 42;
/// the names of the default tree (the app passes them in its language)
pub const DEFAULT_NAMES: [&str; 4] = ["Standort", "Gebäude", "Erdgeschoss", "Raum"];
/// the id of the default room in the default tree
pub const DEFAULT_ROOM: u32 = 4;

// the rule of the automatic length, in millimetres
const TRAY_MM: i64 = 2800;
const STOREY_MM: i64 = 3500;
const DESK_MM: i64 = 750;
const RACK_BASE_MM: i64 = 100;
const PATCH_MM: i64 = 500;
const DESK_CABLE_MM: i64 = 2000;
const SLACK_PCT: i64 = 10;
const VIRTUAL_M: u32 = 3;
/// the propagation delay of copper and fibre (about two thirds of the speed of light)
pub const NS_PER_M: u64 = 5;
/// the longest length a cable may be given by hand
pub const LEN_MAX_M: u32 = 100_000;

/// a rack unit (1 U = 44.45 mm) in hundredths of a millimetre
const RU_CMM: i64 = 4445;

/// the default tree: Standort › Gebäude › Erdgeschoss › Raum (ids 1 to 4), named `names`
pub fn default_places(names: &[&str; 4]) -> Vec<Place> {
    let s = |i: usize| String::from(names[i]);
    alloc::vec![
        Place { id: 1, kind: PlaceKind::Site, name: s(0), parent: 0, x_dm: 0, y_dm: 0, w_dm: 3000, d_dm: 2000, ..Place::default() },
        Place { id: 2, kind: PlaceKind::Building, name: s(1), parent: 1, x_dm: 200, y_dm: 200, w_dm: 400, d_dm: 250, ..Place::default() },
        Place { id: 3, kind: PlaceKind::Floor, name: s(2), parent: 2, ..Place::default() },
        Place { id: DEFAULT_ROOM, kind: PlaceKind::Room, name: s(3), parent: 3, x_dm: 20, y_dm: 20, w_dm: 100, d_dm: 80, ..Place::default() },
    ]
}

/// where a device stands, resolved
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Where {
    /// its room (0: the tree has none)
    pub room: u32,
    /// the rack it is in (0: none), with its lowest unit (0: none yet)
    pub rack: u32,
    pub unit: u16,
    /// the desk it is on (0: none)
    pub desk: u32,
    /// its spot in the room (decimetres), when it has one
    pub at: Option<(i32, i32)>,
}

impl Where {
    /// the place it is in: its rack, desk or room
    pub fn place(&self) -> u32 {
        if self.rack != 0 {
            self.rack
        } else if self.desk != 0 {
            self.desk
        } else {
            self.room
        }
    }
}

/// the places of a project as a tree
#[derive(Clone, Debug)]
pub struct Tree {
    pub places: Vec<Place>,
    default: bool,
    idx: BTreeMap<u32, usize>,
    room0: u32,
}

impl Tree {
    /// the project's places (the default tree when it has none)
    pub fn of(p: &Project) -> Tree {
        Tree::from_places(if p.places.is_empty() { default_places(&DEFAULT_NAMES) } else { p.places.clone() }, p.places.is_empty())
    }

    fn from_places(places: Vec<Place>, default: bool) -> Tree {
        let mut idx = BTreeMap::new();
        for (i, x) in places.iter().enumerate() {
            idx.entry(x.id).or_insert(i);
        }
        let mut t = Tree { places, default, idx, room0: 0 };
        t.room0 = t.places.iter().find(|x| x.kind == PlaceKind::Room && t.rooted(x.id)).map_or(0, |x| x.id);
        t
    }

    /// the project has no places of its own: this is the default tree
    pub fn is_default(&self) -> bool {
        self.default
    }
    pub fn get(&self, id: u32) -> Option<&Place> {
        self.places.get(*self.idx.get(&id)?)
    }
    /// the places in a place in their order (0: the sites)
    pub fn children(&self, id: u32) -> Vec<&Place> {
        self.places.iter().filter(|x| x.parent == id && x.id != id).collect()
    }
    /// the ids from the site down to `id` (empty: unknown; a loop in the parents ends it)
    pub fn path(&self, id: u32) -> Vec<u32> {
        let mut v = Vec::new();
        let mut at = id;
        while let Some(x) = self.get(at) {
            if v.contains(&at) || v.len() > 8 {
                break;
            }
            v.push(at);
            at = x.parent;
        }
        v.reverse();
        v
    }
    // the place hangs below a site through places of the right kinds
    fn rooted(&self, id: u32) -> bool {
        let path = self.path(id);
        let Some(first) = path.first().and_then(|&i| self.get(i)) else { return false };
        if first.kind != PlaceKind::Site || first.parent != 0 {
            return false;
        }
        path.windows(2).all(|w| match (self.get(w[0]), self.get(w[1])) {
            (Some(a), Some(b)) => b.kind.parent_kind() == Some(a.kind),
            _ => false,
        })
    }
    /// the place itself or the one above it of a kind
    pub fn up(&self, id: u32, kind: PlaceKind) -> Option<u32> {
        self.path(id).into_iter().rev().find(|&i| self.get(i).is_some_and(|x| x.kind == kind))
    }
    /// the default room: the first room of the tree (0: none)
    pub fn default_room(&self) -> u32 {
        self.room0
    }
    /// width and depth in decimetres (a floor: its building's)
    pub fn size(&self, id: u32) -> (u32, u32) {
        let Some(x) = self.get(id) else { return (0, 0) };
        if x.kind == PlaceKind::Floor {
            // its building's (a file whose floors lie in floors gets a building's default)
            return match self.get(x.parent) {
                Some(b) if b.kind != PlaceKind::Floor => self.size(b.id),
                _ => PlaceKind::Building.default_size(),
            };
        }
        let (dw, dd) = x.kind.default_size();
        (if x.w_dm == 0 { dw } else { x.w_dm }, if x.d_dm == 0 { dd } else { x.d_dm })
    }
    /// the top left corner in the coordinates of all sites, in millimetres
    pub fn abs(&self, id: u32) -> (i64, i64) {
        let mut x = 0i64;
        let mut y = 0i64;
        for i in self.path(id) {
            if let Some(p) = self.get(i)
                && p.kind != PlaceKind::Floor
            {
                x += p.x_dm as i64 * 100;
                y += p.y_dm as i64 * 100;
            }
        }
        (x, y)
    }
    /// the middle in the coordinates of all sites, in millimetres
    pub fn mid(&self, id: u32) -> (i64, i64) {
        let (x, y) = self.abs(id);
        let (w, d) = self.size(id);
        (x + w as i64 * 50, y + d as i64 * 50)
    }
    /// the storey of the floor a place is on (0 without one)
    pub fn level(&self, id: u32) -> i32 {
        self.up(id, PlaceKind::Floor).and_then(|f| self.get(f)).map_or(0, |f| f.level)
    }
    /// a rack's height in rack units
    pub fn units(&self, rack: u32) -> u16 {
        match self.get(rack) {
            Some(x) if x.units > 0 => x.units,
            _ => RACK_UNITS,
        }
    }
    /// "Standort › Gebäude › Erdgeschoss › Raum"
    pub fn name_path(&self, id: u32) -> String {
        let names: Vec<&str> = self.path(id).into_iter().filter_map(|i| self.get(i)).map(|x| x.name.as_str()).collect();
        names.join(" \u{203a} ")
    }

    /// where a device stands
    pub fn whereis(&self, d: &Device) -> Where {
        let s = &d.phys;
        let place = self.get(s.place).filter(|_| self.rooted(s.place));
        let free = |room: u32| Where { room, at: if s.placed { Some((s.x_dm, s.y_dm)) } else { None }, ..Where::default() };
        match place.map(|x| x.kind) {
            Some(PlaceKind::Room) => free(s.place),
            Some(PlaceKind::Rack) => Where { room: self.up(s.place, PlaceKind::Room).unwrap_or(0), rack: s.place, unit: s.unit, ..Where::default() },
            Some(PlaceKind::Desk) => Where { room: self.up(s.place, PlaceKind::Room).unwrap_or(0), desk: s.place, ..Where::default() },
            // a site, building or floor: the first room below it
            Some(_) => {
                let r = self.places.iter().find(|x| x.kind == PlaceKind::Room && self.path(x.id).contains(&s.place)).map_or(self.room0, |x| x.id);
                Where { room: r, ..Where::default() }
            }
            None => Where { room: self.room0, ..Where::default() },
        }
    }

    /// the devices standing in a place or anywhere below it (in project order)
    pub fn devices_in<'a>(&self, p: &'a Project, place: u32) -> Vec<&'a Device> {
        p.devices.iter().filter(|d| self.path(self.whereis(d).place()).contains(&place)).collect()
    }

    /// rack units unit .. unit + h - 1 of a rack are inside it and taken by no device but
    /// `except`
    pub fn rack_free(&self, p: &Project, rack: u32, unit: u16, h: u16, except: u32) -> bool {
        let top = self.units(rack);
        if unit == 0 || h == 0 || unit as u32 + h as u32 - 1 > top as u32 {
            return false;
        }
        p.devices.iter().filter(|d| d.id != except).all(|d| {
            let w = self.whereis(d);
            if w.rack != rack || w.unit == 0 {
                return true;
            }
            let dh = p.model_of(d.id).map_or(1, |m| catalog::rack_units(m).max(1)) as u16;
            w.unit + dh <= unit || unit + h <= w.unit
        })
    }
    /// the lowest unit of a rack where `h` units are free (None: it is full)
    pub fn rack_slot(&self, p: &Project, rack: u32, h: u16, except: u32) -> Option<u16> {
        (1..=self.units(rack)).find(|&u| self.rack_free(p, rack, u, h, except))
    }
}

/// a cable's length in metres and whether it is the automatic one
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Len {
    pub m: u32,
    pub auto: bool,
}

// an end of a cable: where it is (millimetres, the coordinates of all sites), how high above
// its floor, on which storey of which building, in which rack or on which desk
struct End {
    x: i64,
    y: i64,
    z: i64,
    level: i32,
    building: u32,
    rack: u32,
    desk: u32,
    nowhere: bool,
}

fn end(t: &Tree, p: &Project, dev: u32) -> End {
    let Some(d) = p.device(dev) else {
        return End { x: 0, y: 0, z: 0, level: 0, building: 0, rack: 0, desk: 0, nowhere: true };
    };
    let form = catalog::model(&d.model).map_or(Form::Desktop, catalog::form);
    let w = t.whereis(d);
    let room = w.room;
    let base = End { x: 0, y: 0, z: 0, level: t.level(room), building: t.up(room, PlaceKind::Building).unwrap_or(0), rack: w.rack, desk: w.desk, nowhere: form == Form::Virtual || room == 0 };
    let high = if form == Form::Ceiling { TRAY_MM } else { 0 };
    if w.rack != 0 {
        let (x, y) = t.mid(w.rack);
        let z = RACK_BASE_MM + (w.unit.max(1) as i64 - 1) * RU_CMM / 100;
        return End { x, y, z, ..base };
    }
    if w.desk != 0 {
        let (x, y) = t.mid(w.desk);
        return End { x, y, z: DESK_MM, ..base };
    }
    match w.at {
        Some((sx, sy)) => {
            let (rx, ry) = t.abs(room);
            End { x: rx + sx as i64 * 100, y: ry + sy as i64 * 100, z: high, ..base }
        }
        None => {
            let (x, y) = t.mid(room);
            End { x, y, z: high, ..base }
        }
    }
}

/// the automatic length of a cable between two devices in metres (the rule above)
pub fn auto_len_m(p: &Project, t: &Tree, a: u32, b: u32) -> u32 {
    let (ea, eb) = (end(t, p, a), end(t, p, b));
    if ea.nowhere || eb.nowhere {
        return VIRTUAL_M;
    }
    let mm = if ea.rack != 0 && ea.rack == eb.rack {
        PATCH_MM + (ea.z - eb.z).abs()
    } else if ea.desk != 0 && ea.desk == eb.desk {
        DESK_CABLE_MM
    } else {
        let flat = (ea.x - eb.x).abs() + (ea.y - eb.y).abs();
        let storeys = if ea.building == eb.building { (ea.level - eb.level).abs() as i64 } else { ea.level.abs() as i64 + eb.level.abs() as i64 };
        let run = (TRAY_MM - ea.z).max(0) + (TRAY_MM - eb.z).max(0) + flat + storeys * STOREY_MM;
        run * (100 + SLACK_PCT) / 100
    };
    (((mm + 999) / 1000).max(1)).min(u32::MAX as i64) as u32
}

/// a cable's length: the one set by hand, else the automatic one (radio: none, 0)
pub fn length(p: &Project, t: &Tree, l: &Link) -> Len {
    if l.cable == CableKind::Wireless {
        return Len { m: 0, auto: l.len_m == 0 };
    }
    if l.len_m > 0 {
        return Len { m: l.len_m, auto: false };
    }
    Len { m: auto_len_m(p, t, l.a.dev, l.b.dev), auto: true }
}

/// the device was placed in the physical view: it has a place or a spot of its own
pub fn placed(d: &Device) -> bool {
    d.phys.place != 0 || d.phys.placed
}

/// the delay the simulator runs a cable with: 5 ns per metre for copper and fibre whose delay
/// is the default of its kind and whose length comes from the plan, else its own
pub fn delay_ns(p: &Project, t: &Tree, l: &Link) -> u64 {
    if !(l.cable.is_copper() || l.cable.is_fiber()) || l.delay_ns != cable::default_delay_ns(l.cable) {
        return l.delay_ns;
    }
    let both = [l.a.dev, l.b.dev].iter().all(|&d| p.device(d).is_some_and(placed));
    if l.len_m == 0 && !both {
        return l.delay_ns;
    }
    length(p, t, l).m as u64 * NS_PER_M
}

/// the longest cable of a medium at a speed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reach {
    pub max_m: u32,
    /// the standard that sets it ("1000BASE-T", "10GBASE-SR", "RS-232", "V.35")
    pub std: &'static str,
}

/// the reach of a medium: the cable kind at a speed (kbit/s) with the optics of its ends (fibre;
/// None counts as SR); None: no limit (WAN lines, radio)
pub fn reach(kind: CableKind, kbps: u32, optic: Option<Optic>) -> Option<Reach> {
    let r = |max_m: u32, std: &'static str| Some(Reach { max_m, std });
    match kind {
        CableKind::CopperStraight | CableKind::CopperCross => match kbps {
            0..=10_000 => r(100, "10BASE-T"),
            10_001..=100_000 => r(100, "100BASE-TX"),
            100_001..=1_000_000 => r(100, "1000BASE-T"),
            1_000_001..=2_500_000 => r(100, "2.5GBASE-T"),
            2_500_001..=5_000_000 => r(100, "5GBASE-T"),
            5_000_001..=10_000_000 => r(100, "10GBASE-T"),
            10_000_001..=25_000_000 => r(30, "25GBASE-T"),
            _ => r(30, "40GBASE-T"),
        },
        CableKind::FiberMm | CableKind::FiberSm => match (optic.unwrap_or(Optic::Sr), kbps) {
            (Optic::Sr, 0..=1_000_000) => r(550, "1000BASE-SX"),
            (Optic::Sr, 1_000_001..=10_000_000) => r(300, "10GBASE-SR"),
            (Optic::Sr, 10_000_001..=25_000_000) => r(70, "25GBASE-SR"),
            (Optic::Sr, 25_000_001..=40_000_000) => r(100, "40GBASE-SR4"),
            (Optic::Sr, _) => r(70, "100GBASE-SR4"),
            (Optic::Lr, 0..=1_000_000) => r(5_000, "1000BASE-LX"),
            (Optic::Lr, 1_000_001..=10_000_000) => r(10_000, "10GBASE-LR"),
            (Optic::Lr, 10_000_001..=25_000_000) => r(10_000, "25GBASE-LR"),
            (Optic::Lr, 25_000_001..=40_000_000) => r(10_000, "40GBASE-LR4"),
            (Optic::Lr, _) => r(10_000, "100GBASE-LR4"),
        },
        CableKind::Console => r(15, "RS-232"),
        CableKind::Serial => match kbps {
            0..=2 => r(1250, "V.35"),
            3..=4 => r(625, "V.35"),
            5..=9 => r(312, "V.35"),
            10..=19 => r(156, "V.35"),
            20..=38 => r(78, "V.35"),
            39..=64 => r(51, "V.35"),
            _ => r(15, "V.35"),
        },
        CableKind::Wireless | CableKind::Wan => None,
    }
}

/// why the optics of a fibre cannot carry
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpticFault {
    /// the ends have different optics (end a's, end b's)
    Mismatch(Optic, Optic),
    /// both have this optic, the fibre is the other kind
    WrongFiber(Optic),
}

/// the optics of a fibre's ends against each other and the fibre (None for other cables;
/// a port without one counts as SR)
pub fn optic_fault(kind: CableKind, a: Option<Optic>, b: Option<Optic>) -> Option<OpticFault> {
    if !kind.is_fiber() {
        return None;
    }
    let (a, b) = (a.unwrap_or(Optic::Sr), b.unwrap_or(Optic::Sr));
    if a != b {
        return Some(OpticFault::Mismatch(a, b));
    }
    if (a == Optic::Lr) != (kind == CableKind::FiberSm) {
        return Some(OpticFault::WrongFiber(a));
    }
    None
}

/// the speed a cable of the project runs at: its own and both ports' (as the simulator has it)
pub fn link_kbps(p: &Project, l: &Link) -> u32 {
    let mut k = if l.kbps == 0 { u32::MAX } else { l.kbps };
    for r in [&l.a, &l.b] {
        if let Ok(s) = p.resolve_port(r) {
            k = k.min(s.kbps.max(1));
        }
    }
    if k == u32::MAX { 0 } else { k }
}

/// the reach of a cable of the project (its speed, the optic of end a)
pub fn link_reach(p: &Project, l: &Link) -> Option<Reach> {
    let optic = p.resolve_port(&l.a).ok().and_then(|s| s.optic);
    reach(l.cable, link_kbps(p, l), optic)
}

/// what does not fit: (an error, the device, the message) -- places in no fitting place, ids
/// twice, devices in a place that is none, in a rack while they are no rack device, rack units
/// outside the rack or taken twice
pub fn issues(p: &Project) -> Vec<(bool, Option<u32>, Msg)> {
    let mut v = Vec::new();
    if p.places.is_empty() {
        return v;
    }
    let t = Tree::of(p);
    let mut ids = Vec::new();
    for x in &p.places {
        if ids.contains(&x.id) || x.id == 0 {
            v.push((true, None, msg!("Die Ortsnummer {} kommt doppelt vor.", x.id)));
        }
        ids.push(x.id);
        if !t.rooted(x.id) {
            v.push((true, None, msg!("Der Ort \u{201e}{}\u{201c} liegt in keinem passenden Ort.", x.name)));
        }
    }
    if t.default_room() == 0 {
        v.push((true, None, msg!("Der Plan hat keinen Raum: die Geräte haben keinen Ort.")));
    }
    for d in &p.devices {
        let s = &d.phys;
        if s.place != 0 && !t.get(s.place).is_some_and(|x| x.kind.holds_devices() && t.rooted(x.id)) {
            v.push((false, Some(d.id), msg!("{} steht an keinem gültigen Ort (Nummer {}); es zählt der erste Raum.", d.name, s.place)));
        }
        let w = t.whereis(d);
        if w.rack == 0 {
            continue;
        }
        let Some(m) = p.model_of(d.id) else { continue };
        let rack = t.get(w.rack).map_or_else(String::new, |x| x.name.clone());
        if catalog::form(m) != Form::Rack {
            v.push((false, Some(d.id), msg!("{} ist kein Rack-Gerät und steckt doch im Rack \u{201e}{}\u{201c}.", d.name, rack)));
            continue;
        }
        let h = catalog::rack_units(m).max(1) as u16;
        if w.unit != 0 && !t.rack_free(p, w.rack, w.unit, h, d.id) {
            v.push((false, Some(d.id), msg!("{} belegt im Rack \u{201e}{}\u{201c} Höheneinheiten, die nicht frei sind (ab HE {}).", d.name, rack, w.unit)));
        }
    }
    v
}

// ---------------------------------------------------------------- edits

/// why an edit of the physical view was refused
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhysErr {
    UnknownPlace(u32),
    UnknownDevice(u32),
    UnknownLink(u32),
    /// a place of this kind does not go there
    WrongParent(PlaceKind, PlaceKind),
    BadName(String),
    /// the last room stays (devices need one)
    LastRoom,
    /// devices stand in rooms, racks and desks
    NoDevices(PlaceKind),
    /// only rack devices go into a rack
    NotRack(String),
    /// the rack units from `unit` are taken or outside the rack
    UnitsTaken { rack: String, unit: u16, h: u16 },
    RackFull(String),
    /// a rack's devices would stick out (the lowest height that holds them)
    TooLow(u16),
    BadSize,
    BadLen(u32),
    /// the port has no optics cage
    NoCage(String),
}

impl PhysErr {
    /// the reason as a message
    pub fn msg(&self) -> Msg {
        match self {
            PhysErr::UnknownPlace(id) => msg!("Den Ort {} gibt es nicht.", id),
            PhysErr::UnknownDevice(id) => msg!("Das Gerät {} gibt es nicht.", id),
            PhysErr::UnknownLink(id) => msg!("Die Verbindung {} gibt es nicht.", id),
            PhysErr::WrongParent(k, p) => msg!("Ein Ort der Art {} gehört nicht in einen Ort der Art {}.", crate::msg::phrase(k.title()), crate::msg::phrase(p.title())),
            PhysErr::BadName(n) => msg!("\u{201e}{}\u{201c} taugt nicht als Name: 1 bis 63 Zeichen.", n),
            PhysErr::LastRoom => msg!("Der letzte Raum bleibt: jedes Gerät braucht einen Ort."),
            PhysErr::NoDevices(k) => msg!("In einem Ort der Art {} stehen keine Geräte: in einen Raum, ein Rack oder auf einen Tisch.", crate::msg::phrase(k.title())),
            PhysErr::NotRack(n) => msg!("{} ist kein Rack-Gerät.", n),
            PhysErr::UnitsTaken { rack, unit, h: 1 } => msg!("Im Rack \u{201e}{}\u{201c} ist die Höheneinheit {} nicht frei.", rack, unit),
            PhysErr::UnitsTaken { rack, unit, h } => msg!("Im Rack \u{201e}{}\u{201c} sind die Höheneinheiten {} bis {} nicht frei.", rack, unit, unit + h - 1),
            PhysErr::RackFull(n) => msg!("Im Rack \u{201e}{}\u{201c} ist kein Platz mehr.", n),
            PhysErr::TooLow(u) => msg!("Die Geräte im Rack brauchen mindestens {} Höheneinheiten.", u),
            PhysErr::BadSize => msg!("Breite und Tiefe: 0,1 bis 10 000 m."),
            PhysErr::BadLen(m) => msg!("{} m geht nicht: 0 (automatisch) bis 100 000 m.", m),
            PhysErr::NoCage(port) => msg!("{} hat keinen Schacht für eine Optik.", port),
        }
    }
}

/// the project keeps its places: a project without them gets the default tree (named `names`)
pub fn ensure(p: &mut Project, names: &[&str; 4]) {
    if p.places.is_empty() {
        p.places = default_places(names);
    }
}

/// the next free place id
pub fn next_id(p: &Project) -> u32 {
    p.places.iter().map(|x| x.id).max().map_or(DEFAULT_ROOM + 1, |m| m + 1)
}

/// a new place of a kind in `parent` (0 for a site), named `prefix` and the lowest free number
/// among its siblings ("Raum 2"), right of its last sibling -> its id (the default tree is kept
/// first: `names`)
pub fn add_place(p: &mut Project, kind: PlaceKind, parent: u32, prefix: &str, names: &[&str; 4]) -> Result<u32, PhysErr> {
    ensure(p, names);
    let t = Tree::of(p);
    match (kind.parent_kind(), t.get(parent)) {
        (None, _) if parent == 0 => {}
        (Some(k), Some(x)) if x.kind == k => {}
        (_, Some(x)) => return Err(PhysErr::WrongParent(kind, x.kind)),
        _ => return Err(PhysErr::UnknownPlace(parent)),
    }
    let sibs = t.children(parent);
    let mut k = 1u32;
    let name = loop {
        let c = format!("{} {}", prefix, k);
        if !sibs.iter().any(|x| x.name.eq_ignore_ascii_case(&c)) {
            break c;
        }
        k += 1;
    };
    let (w, d) = kind.default_size();
    // right of the last sibling with a gap; a floor goes on top of the others
    let (x, y) = match sibs.iter().filter(|s| s.kind == kind).last() {
        Some(s) => {
            let (sw, _) = t.size(s.id);
            (s.x_dm + sw as i32 + if matches!(kind, PlaceKind::Rack) { 0 } else { (w as i32 / 5).max(2) }, s.y_dm)
        }
        None => match kind {
            PlaceKind::Site | PlaceKind::Floor => (0, 0),
            _ => ((w as i32 / 5).max(2), (d as i32 / 5).max(2)),
        },
    };
    let level = sibs.iter().filter(|s| s.kind == PlaceKind::Floor).map(|s| s.level + 1).max().unwrap_or(0);
    let id = next_id(p);
    p.places.push(Place { id, kind, name, parent, x_dm: x, y_dm: y, w_dm: w, d_dm: d, units: if kind == PlaceKind::Rack { RACK_UNITS } else { 0 }, level: if kind == PlaceKind::Floor { level } else { 0 } });
    Ok(id)
}

/// a place's new name (1 to 63 characters)
pub fn rename_place(p: &mut Project, id: u32, name: &str, names: &[&str; 4]) -> Result<(), PhysErr> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 63 || name.chars().any(|c| c.is_control()) {
        return Err(PhysErr::BadName(String::from(name)));
    }
    ensure(p, names);
    let x = p.places.iter_mut().find(|x| x.id == id).ok_or(PhysErr::UnknownPlace(id))?;
    x.name = String::from(name);
    Ok(())
}

/// removes a place with everything in it; its devices go to the default room without a spot
/// (refused when no room would be left)
pub fn remove_place(p: &mut Project, id: u32, names: &[&str; 4]) -> Result<(), PhysErr> {
    ensure(p, names);
    let t = Tree::of(p);
    if t.get(id).is_none() {
        return Err(PhysErr::UnknownPlace(id));
    }
    let gone: Vec<u32> = p.places.iter().filter(|x| t.path(x.id).contains(&id)).map(|x| x.id).collect();
    let left = p.places.iter().filter(|x| !gone.contains(&x.id)).filter(|x| x.kind == PlaceKind::Room).any(|x| t.rooted(x.id) && !t.path(x.id).iter().any(|i| gone.contains(i)));
    if !left {
        return Err(PhysErr::LastRoom);
    }
    for d in p.devices.iter_mut() {
        let at = t.whereis(d).place();
        if t.path(at).iter().any(|i| gone.contains(i)) || gone.contains(&d.phys.place) {
            d.phys = Spot::default();
        }
    }
    p.places.retain(|x| !gone.contains(&x.id));
    Ok(())
}

/// a place's position, size, rack height or storey (None keeps it); a rack is never lower than
/// its devices need
pub fn set_place(p: &mut Project, id: u32, at: Option<(i32, i32)>, size: Option<(u32, u32)>, units: Option<u16>, level: Option<i32>, names: &[&str; 4]) -> Result<(), PhysErr> {
    ensure(p, names);
    let t = Tree::of(p);
    if t.get(id).is_none() {
        return Err(PhysErr::UnknownPlace(id));
    }
    if let Some((w, d)) = size
        && (w == 0 || d == 0 || w > 100_000 || d > 100_000)
    {
        return Err(PhysErr::BadSize);
    }
    if let Some(u) = units {
        let need = p
            .devices
            .iter()
            .filter_map(|d| {
                let w = t.whereis(d);
                if w.rack != id || w.unit == 0 {
                    return None;
                }
                Some(w.unit + p.model_of(d.id).map_or(1, |m| catalog::rack_units(m).max(1)) as u16 - 1)
            })
            .max()
            .unwrap_or(1);
        if u < need || u == 0 || u > 60 {
            return Err(PhysErr::TooLow(need.max(1)));
        }
    }
    let x = p.places.iter_mut().find(|x| x.id == id).ok_or(PhysErr::UnknownPlace(id))?;
    if let Some((ax, ay)) = at {
        x.x_dm = ax;
        x.y_dm = ay;
    }
    if let Some((w, d)) = size {
        x.w_dm = w;
        x.d_dm = d;
    }
    if let Some(u) = units {
        x.units = u;
    }
    if let Some(l) = level {
        x.level = l;
    }
    Ok(())
}

/// a device goes to a place: a room (at a spot, or without one), a rack (at a unit; 0: the
/// lowest free one) or a desk
pub fn place_device(p: &mut Project, dev: u32, place: u32, unit: u16, at: Option<(i32, i32)>, names: &[&str; 4]) -> Result<(), PhysErr> {
    let d = p.device(dev).ok_or(PhysErr::UnknownDevice(dev))?;
    let name = d.name.clone();
    let m = p.model_of(dev);
    ensure(p, names);
    let t = Tree::of(p);
    let x = t.get(place).ok_or(PhysErr::UnknownPlace(place))?;
    let spot = match x.kind {
        PlaceKind::Room => Spot { place, unit: 0, x_dm: at.map_or(0, |a| a.0), y_dm: at.map_or(0, |a| a.1), placed: at.is_some() },
        PlaceKind::Desk => Spot { place, ..Spot::default() },
        PlaceKind::Rack => {
            let Some(m) = m.filter(|m| catalog::form(m) == Form::Rack) else {
                return Err(PhysErr::NotRack(name));
            };
            let h = catalog::rack_units(m).max(1) as u16;
            let u = if unit == 0 { t.rack_slot(p, place, h, dev).ok_or_else(|| PhysErr::RackFull(x.name.clone()))? } else { unit };
            if !t.rack_free(p, place, u, h, dev) {
                return Err(PhysErr::UnitsTaken { rack: x.name.clone(), unit: u, h });
            }
            Spot { place, unit: u, ..Spot::default() }
        }
        k => return Err(PhysErr::NoDevices(k)),
    };
    if let Some(d) = p.device_mut(dev) {
        d.phys = spot;
    }
    Ok(())
}

/// a cable's length set by hand (0: automatic again)
pub fn set_len(p: &mut Project, link: u32, m: u32) -> Result<(), PhysErr> {
    if m > LEN_MAX_M {
        return Err(PhysErr::BadLen(m));
    }
    let l = p.link_mut(link).ok_or(PhysErr::UnknownLink(link))?;
    l.len_m = m;
    Ok(())
}

/// the optic in a port's cage; a cable on it whose kind is automatic follows (single-mode with
/// LR optics at both ends, else multimode)
pub fn set_optic(p: &mut Project, dev: u32, port: &str, optic: Optic) -> Result<(), PhysErr> {
    let d = p.device(dev).ok_or(PhysErr::UnknownDevice(dev))?;
    let spec = catalog::model(&d.model).and_then(|m| m.port(port)).ok_or_else(|| PhysErr::NoCage(String::from(port)))?;
    if spec.optic.is_none() {
        return Err(PhysErr::NoCage(spec.name));
    }
    let full = spec.name.clone();
    if let Some(d) = p.device_mut(dev) {
        d.optics.retain(|k, _| !crate::ifname::same(k, &full));
        if optic != Optic::Sr {
            d.optics.insert(full.clone(), optic);
        }
    }
    let link = p.link_at(dev, &full).map(|l| (l.id, l.a.clone(), l.b.clone(), l.cable_fixed));
    if let Some((id, a, b, false)) = link
        && let (Ok(pa), Ok(pb)) = (p.resolve_port(&a), p.resolve_port(&b))
        && let Ok(k) = cable::cable_kind(&pa, &pb)
        && let Some(l) = p.link_mut(id)
    {
        l.cable = k;
    }
    Ok(())
}

/// the port of a project's device by any spelling (scenarios, the inspector)
pub fn port_ref(p: &Project, dev: u32, port: &str) -> Option<PortRef> {
    let spec = p.model_of(dev)?.port(port)?;
    Some(PortRef { dev, port: spec.name })
}
