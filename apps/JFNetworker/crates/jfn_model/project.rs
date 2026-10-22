// project -- a JFNetworker project: devices, links, notes, the view, an optional lab task
//
//   Project::new(title, seed)                     an empty project (format "jfn", version 1)
//   add_device(model, x, y) -> id                 a new device with a free name (PC1, SW2, ...)
//   rename / move_device / remove_device -> (Device, links) / insert_device (undo, paste)
//   connect(a, b, cable) -> link id               ports by any spelling; the right cable when
//                                                 `cable` is None, else checked with cable_fits
//   disconnect(id) -> Link / insert_link (undo)    add_note / remove_note
//   suggest_ports(a, b)                           the free port pair a quick connect would take
//   ports / free_ports / link_at / find / model_of / bounds
//   check() -> Vec<Issue>                         consistency (names, ports, cables, ids)
//   to_text() / from_text(text)                   the .jfnet file: indented JSON
//   ProjErr::msg(), Issue::msg, LoadErr::msg      the German texts as a Msg (msg.rs) for the
//                                                 app's catalogue
//
// A device's configuration is its startup-config text (the simulator replays it through the
// command line); services carry their settings as JSON.  Ids come from `next_id` and are never
// reused while the project is open, so undo, the canvas and the simulator agree on them.
//
// A lab project (assets/labs of the app) has a "lab" member, a free one null (lab.rs):
//
//   "lab": {
//     "title": ["Lastverteilung", "Load balancing"],      a text: "Deutsch" or ["Deutsch",
//     "summary": ["Eine Zeile für die Liste", "..."],       "English"] (no English: the German)
//     "topic": "ds",                                      "net" or "ds": the shelf of the list
//     "intro": [["Absatz 1", "Paragraph 1"], ...],
//     "ordered": false,                                   true: a goal counts after the ones
//                                                         before it only
//     "goals": [{"text": ["WEB3 trägt mit", "WEB3 does its share"],
//                "checks": ["WEB3 svc web metric requests >= 50 over 5s"],
//                "hints": [["Erster Hinweis", "First hint"], ...]}],
//     "hints": [["Ein Hinweis zur ganzen Aufgabe", "..."]]
//   }
//
// A check is an expectation of a scenario without its "expect" (scenario.rs: output, route,
// no route, hops, metric, a metric over a span); a goal is reached when all its checks hold at
// one moment.  Members that are missing take their defaults; a file without a lab stays as it
// was ("lab": null).  An assessment adds "assess" to the lab (lab.rs, A1): graded items with
// points, the feedback, a time limit and the target network; the project is its start network,
// a lab without one has "assess": null.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use agent_core::jmap::{Json, Value};
use agent_core::json::{self, Limits};
use agent_core::{record, wire_enum};

use crate::cable::{self, CableKind, Why};
use crate::catalog::{self, Model, PortSpec, F_AP, F_ANNOTATION};
use crate::ifname;
use crate::lab::Lab;
use crate::msg::Msg;
use crate::msg;

/// the format name in every project file
pub const FORMAT: &str = "jfn";
/// the newest file version this program writes and reads
pub const VERSION: u32 = 1;

record! {
    /// one end of a link: a device and the full name of its port
    pub struct PortRef {
        pub dev: u32,
        pub port: String,
    }
}

record! {
    /// a point on the canvas (world units = DIP at 100 %)
    pub struct Point {
        pub x: i32,
        pub y: i32,
    }
}

wire_enum! {
    /// how a link is drawn
    pub enum Route {
        Straight = "straight",
        Ortho = "ortho",
        Curve = "curve",
    }
}

record! {
    /// a service running on a device (DHCP server, web server, script, ...)
    pub struct ServiceDef {
        pub kind: String,
        pub name: String,
        pub disabled: bool,
        pub config: Value,
    }
}

record! {
    /// a device on the canvas
    pub struct Device {
        pub id: u32,
        /// host name (unique, case-insensitive)
        pub name: String,
        /// catalog model id
        pub model: String,
        pub x: i32,
        pub y: i32,
        /// switched off
        pub off: bool,
        /// the configuration the device boots with (IOS or shell commands)
        pub startup_config: String,
        pub services: Vec<ServiceDef>,
        pub comment: String,
    }
}

record! {
    /// a cable (or a radio association) between two ports
    pub struct Link {
        pub id: u32,
        pub a: PortRef,
        pub b: PortRef,
        pub cable: CableKind,
        /// the user chose the cable (else it follows the ports: "Automatisch")
        pub cable_fixed: bool,
        pub kbps: u32,
        pub delay_ns: u64,
        pub jitter_ns: u64,
        /// loss in parts per million
        pub loss_ppm: u32,
        /// unplugged on purpose (fault injection)
        pub down: bool,
        pub route: Route,
        pub waypoints: Vec<Point>,
    }
}

record! {
    /// a text note on the canvas
    pub struct Note {
        pub id: u32,
        pub x: i32,
        pub y: i32,
        pub w: i32,
        pub h: i32,
        pub text: String,
    }
}

record! {
    /// the view of the canvas when the project was saved
    pub struct Layout {
        /// zoom in per mille (1000 = 100 %; 0 = 100 %)
        pub zoom_pm: u32,
        pub cx: i32,
        pub cy: i32,
        /// grid size in world units (0 = 20)
        pub grid: u32,
        pub no_snap: bool,
        /// hidden protocols of the event list
        pub filters: Vec<String>,
    }
}

record! {
    /// a whole project (one .jfnet file)
    pub struct Project {
        pub format: String,
        pub version: u32,
        pub title: String,
        pub description: String,
        /// the seed of the simulation's random numbers
        pub seed: u64,
        pub next_id: u32,
        /// copper links with the wrong cable stay down (no auto-MDI-X)
        pub strict_mdix: bool,
        pub devices: Vec<Device>,
        pub links: Vec<Link>,
        pub notes: Vec<Note>,
        pub layout: Layout,
        /// the task of a lab (title, goals with their checks, hints), None (null) for a free
        /// project
        pub lab: Option<Lab>,
    }
}

/// why an edit was refused (German text through Display)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjErr {
    UnknownModel(String),
    NotADevice(String),
    UnknownDevice(u32),
    UnknownPort { dev: String, port: String },
    UnknownLink(u32),
    BadName(String),
    NameTaken(String),
    IdTaken(u32),
    Cable(Why),
}

impl ProjErr {
    /// the reason as a message (Display: its German text)
    pub fn msg(&self) -> Msg {
        match self {
            ProjErr::UnknownModel(m) => msg!("Unbekanntes Gerätemodell \u{201e}{}\u{201c}.", m),
            ProjErr::NotADevice(m) => msg!("\u{201e}{}\u{201c} ist kein Gerät.", m),
            ProjErr::UnknownDevice(id) => msg!("Das Gerät {} gibt es nicht.", id),
            ProjErr::UnknownPort { dev, port } => msg!("{} hat keinen Port \u{201e}{}\u{201c}.", dev, port),
            ProjErr::UnknownLink(id) => msg!("Die Verbindung {} gibt es nicht.", id),
            ProjErr::BadName(n) => msg!("\u{201e}{}\u{201c} ist kein gültiger Gerätename: Buchstaben, Ziffern und Bindestriche, vorn ein Buchstabe, höchstens 63 Zeichen.", n),
            ProjErr::NameTaken(n) => msg!("Der Name \u{201e}{}\u{201c} ist schon vergeben.", n),
            ProjErr::IdTaken(id) => msg!("Die Nummer {} ist schon vergeben.", id),
            ProjErr::Cable(w) => w.msg(),
        }
    }
}

impl fmt::Display for ProjErr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.msg().text())
    }
}

/// a finding of check()
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// an error (the project is inconsistent) or a warning
    pub error: bool,
    pub dev: Option<u32>,
    pub link: Option<u32>,
    /// German (msg.text())
    pub text: String,
    /// what is wrong, for the app's catalogue
    pub msg: Msg,
}

impl Issue {
    fn new(error: bool, dev: Option<u32>, link: Option<u32>, msg: Msg) -> Issue {
        Issue { error, dev, link, text: msg.text(), msg }
    }
}

/// why a file could not be read
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadErr {
    /// byte offset of a syntax error, else 0
    pub offset: usize,
    /// German (msg.text())
    pub text: String,
    /// the reason, for the app's catalogue
    pub msg: Msg,
}

impl LoadErr {
    fn new(offset: usize, msg: Msg) -> LoadErr {
        LoadErr { offset, text: msg.text(), msg }
    }
}

impl fmt::Display for LoadErr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// the limits of project files: large topologies with long configurations
pub fn file_limits() -> Limits {
    Limits { bytes: 64 << 20, depth: 32, values: 4_000_000, string_bytes: 16 << 20, members: 1_000_000 }
}

/// a host name as IOS accepts it: a letter first, letters, digits and hyphens, a letter or
/// digit last, at most 63 characters
pub fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && b[0].is_ascii_alphabetic()
        && b[b.len() - 1].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
}

impl Project {
    /// an empty project
    pub fn new(title: &str, seed: u64) -> Project {
        Project {
            format: String::from(FORMAT),
            version: VERSION,
            title: String::from(title),
            seed,
            next_id: 1,
            layout: Layout { zoom_pm: 1000, grid: 20, ..Layout::default() },
            ..Project::default()
        }
    }

    // ---------------------------------------------------------------- lookups

    pub fn device(&self, id: u32) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }
    pub fn device_mut(&mut self, id: u32) -> Option<&mut Device> {
        self.devices.iter_mut().find(|d| d.id == id)
    }
    /// a device by name (case-insensitive)
    pub fn find(&self, name: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.name.eq_ignore_ascii_case(name.trim()))
    }
    pub fn link(&self, id: u32) -> Option<&Link> {
        self.links.iter().find(|l| l.id == id)
    }
    pub fn link_mut(&mut self, id: u32) -> Option<&mut Link> {
        self.links.iter_mut().find(|l| l.id == id)
    }
    pub fn note(&self, id: u32) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }
    /// the catalog model of a device
    pub fn model_of(&self, id: u32) -> Option<&'static Model> {
        catalog::model(&self.device(id)?.model)
    }
    /// the link on a port (any spelling of the port)
    pub fn link_at(&self, dev: u32, port: &str) -> Option<&Link> {
        self.links.iter().find(|l| (l.a.dev == dev && ifname::same(&l.a.port, port)) || (l.b.dev == dev && ifname::same(&l.b.port, port)))
    }
    /// every port of a device with the id of its link
    pub fn ports(&self, dev: u32) -> Vec<(PortSpec, Option<u32>)> {
        let Some(m) = self.model_of(dev) else {
            return Vec::new();
        };
        m.ports().into_iter().map(|p| {
            let l = self.link_at(dev, &p.name).map(|l| l.id);
            (p, l)
        }).collect()
    }
    /// the ports of a device without a link
    pub fn free_ports(&self, dev: u32) -> Vec<PortSpec> {
        self.ports(dev).into_iter().filter(|(_, l)| l.is_none()).map(|(p, _)| p).collect()
    }
    /// the far end of every link of a device: (own port, other device, other port, link id)
    pub fn neighbors(&self, dev: u32) -> Vec<(String, u32, String, u32)> {
        let mut v = Vec::new();
        for l in &self.links {
            if l.a.dev == dev {
                v.push((l.a.port.clone(), l.b.dev, l.b.port.clone(), l.id));
            } else if l.b.dev == dev {
                v.push((l.b.port.clone(), l.a.dev, l.a.port.clone(), l.id));
            }
        }
        v
    }
    /// the largest id of any device, link or note
    pub fn max_id(&self) -> u32 {
        let d = self.devices.iter().map(|d| d.id).max().unwrap_or(0);
        let l = self.links.iter().map(|l| l.id).max().unwrap_or(0);
        let n = self.notes.iter().map(|n| n.id).max().unwrap_or(0);
        d.max(l).max(n)
    }
    fn alloc_id(&mut self) -> u32 {
        let id = self.next_id.max(self.max_id() + 1).max(1);
        self.next_id = id + 1;
        id
    }
    /// the canvas area of all devices and notes: [x0, y0, x1, y1]
    pub fn bounds(&self) -> Option<[i32; 4]> {
        let mut b: Option<[i32; 4]> = None;
        let mut add = |x0: i32, y0: i32, x1: i32, y1: i32| {
            b = Some(match b {
                None => [x0, y0, x1, y1],
                Some(c) => [c[0].min(x0), c[1].min(y0), c[2].max(x1), c[3].max(y1)],
            });
        };
        for d in &self.devices {
            add(d.x, d.y, d.x, d.y);
        }
        for n in &self.notes {
            add(n.x, n.y, n.x + n.w, n.y + n.h);
        }
        b
    }

    // ---------------------------------------------------------------- devices

    /// the prefix followed by the lowest free number from 1 ("SW1", "SW2", ...)
    pub fn unique_name(&self, prefix: &str) -> String {
        let mut n = 1u32;
        loop {
            let c = format!("{}{}", prefix, n);
            if self.find(&c).is_none() {
                return c;
            }
            n += 1;
        }
    }
    /// a new device of a catalog model at (x, y)
    pub fn add_device(&mut self, model_id: &str, x: i32, y: i32) -> Result<u32, ProjErr> {
        let m = catalog::model(model_id).ok_or_else(|| ProjErr::UnknownModel(String::from(model_id)))?;
        if m.has(F_ANNOTATION) {
            return Err(ProjErr::NotADevice(String::from(m.name)));
        }
        let name = self.unique_name(m.name_prefix);
        let id = self.alloc_id();
        self.devices.push(Device { id, name, model: String::from(m.id), x, y, ..Device::default() });
        Ok(id)
    }
    /// puts a device back with its own id and name (undo, paste after renaming)
    pub fn insert_device(&mut self, d: Device) -> Result<(), ProjErr> {
        if self.device(d.id).is_some() {
            return Err(ProjErr::IdTaken(d.id));
        }
        if catalog::model(&d.model).is_none() {
            return Err(ProjErr::UnknownModel(d.model));
        }
        if !valid_name(&d.name) {
            return Err(ProjErr::BadName(d.name));
        }
        if self.find(&d.name).is_some() {
            return Err(ProjErr::NameTaken(d.name));
        }
        self.next_id = self.next_id.max(d.id + 1);
        self.devices.push(d);
        Ok(())
    }
    /// gives a device a new host name
    pub fn rename(&mut self, id: u32, name: &str) -> Result<(), ProjErr> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(ProjErr::BadName(String::from(name)));
        }
        if self.find(name).is_some_and(|d| d.id != id) {
            return Err(ProjErr::NameTaken(String::from(name)));
        }
        let d = self.device_mut(id).ok_or(ProjErr::UnknownDevice(id))?;
        d.name = String::from(name);
        Ok(())
    }
    pub fn move_device(&mut self, id: u32, x: i32, y: i32) -> bool {
        match self.device_mut(id) {
            Some(d) => {
                d.x = x;
                d.y = y;
                true
            }
            None => false,
        }
    }
    /// removes a device and its links; both come back for undo
    pub fn remove_device(&mut self, id: u32) -> Option<(Device, Vec<Link>)> {
        let i = self.devices.iter().position(|d| d.id == id)?;
        let d = self.devices.remove(i);
        let mut gone = Vec::new();
        self.links.retain(|l| {
            if l.a.dev == id || l.b.dev == id {
                gone.push(l.clone());
                false
            } else {
                true
            }
        });
        Some((d, gone))
    }

    // ---------------------------------------------------------------- links

    /// the port a reference names, with its full name
    pub fn resolve_port(&self, r: &PortRef) -> Result<PortSpec, ProjErr> {
        let d = self.device(r.dev).ok_or(ProjErr::UnknownDevice(r.dev))?;
        let m = catalog::model(&d.model).ok_or_else(|| ProjErr::UnknownModel(d.model.clone()))?;
        m.port(&r.port).ok_or_else(|| ProjErr::UnknownPort { dev: d.name.clone(), port: r.port.clone() })
    }
    /// connects two ports; `cable` None takes the right cable for them
    pub fn connect(&mut self, a: &PortRef, b: &PortRef, cable: Option<CableKind>) -> Result<u32, ProjErr> {
        if a.dev == b.dev {
            return Err(ProjErr::Cable(Why::SameDevice));
        }
        let pa = self.resolve_port(a)?;
        let pb = self.resolve_port(b)?;
        for (r, p) in [(a, &pa), (b, &pb)] {
            if self.link_at(r.dev, &p.name).is_some() {
                let dn = self.device(r.dev).map(|d| d.name.clone()).unwrap_or_default();
                return Err(ProjErr::Cable(Why::PortInUse(format!("{} {}", dn, ifname::short(&p.name)))));
            }
        }
        let kind = match cable {
            Some(k) => {
                cable::cable_fits(k, &pa, &pb).map_err(ProjErr::Cable)?;
                k
            }
            None => cable::cable_kind(&pa, &pb).map_err(ProjErr::Cable)?,
        };
        let id = self.alloc_id();
        self.links.push(Link {
            id,
            a: PortRef { dev: a.dev, port: pa.name.clone() },
            b: PortRef { dev: b.dev, port: pb.name.clone() },
            cable: kind,
            cable_fixed: cable.is_some(),
            kbps: cable::link_kbps(&pa, &pb),
            delay_ns: cable::default_delay_ns(kind),
            ..Link::default()
        });
        Ok(id)
    }
    /// puts a link back with its id (undo)
    pub fn insert_link(&mut self, l: Link) -> Result<(), ProjErr> {
        if self.link(l.id).is_some() {
            return Err(ProjErr::IdTaken(l.id));
        }
        let pa = self.resolve_port(&l.a)?;
        let pb = self.resolve_port(&l.b)?;
        for (r, p) in [(&l.a, &pa), (&l.b, &pb)] {
            if self.link_at(r.dev, &p.name).is_some() {
                return Err(ProjErr::Cable(Why::PortInUse(String::from(p.name.as_str()))));
            }
        }
        self.next_id = self.next_id.max(l.id + 1);
        self.links.push(l);
        Ok(())
    }
    pub fn disconnect(&mut self, id: u32) -> Option<Link> {
        let i = self.links.iter().position(|l| l.id == id)?;
        Some(self.links.remove(i))
    }
    /// the free port pair a quick connect of two devices takes: a radio association when one
    /// of them is an access point, else the fastest data link, console cables last, the lowest
    /// port numbers on a tie
    pub fn suggest_ports(&self, a: u32, b: u32) -> Option<(String, String)> {
        if a == b {
            return None;
        }
        let ap = self.model_of(a)?.has(F_AP) || self.model_of(b)?.has(F_AP);
        let fa = self.free_ports(a);
        let fb = self.free_ports(b);
        let mut best: Option<((u8, u32, i32), &str, &str)> = None;
        for pa in &fa {
            for pb in &fb {
                let Ok(k) = cable::cable_kind(pa, pb) else {
                    continue;
                };
                let class = match k {
                    CableKind::Wireless if ap => 3,
                    CableKind::Wireless => 1,
                    CableKind::Console => 0,
                    _ => 2,
                };
                let key = (class, cable::link_kbps(pa, pb), -(pa.index as i32 + pb.index as i32));
                if best.as_ref().is_none_or(|b| key > b.0) {
                    best = Some((key, &pa.name, &pb.name));
                }
            }
        }
        best.map(|(_, x, y)| (String::from(x), String::from(y)))
    }

    // ---------------------------------------------------------------- notes

    pub fn add_note(&mut self, x: i32, y: i32, text: &str) -> u32 {
        let id = self.alloc_id();
        self.notes.push(Note { id, x, y, w: 200, h: 80, text: String::from(text) });
        id
    }
    pub fn remove_note(&mut self, id: u32) -> Option<Note> {
        let i = self.notes.iter().position(|n| n.id == id)?;
        Some(self.notes.remove(i))
    }

    // ---------------------------------------------------------------- consistency

    /// everything that does not fit together
    pub fn check(&self) -> Vec<Issue> {
        let mut v = Vec::new();
        let err = |v: &mut Vec<Issue>, dev: Option<u32>, link: Option<u32>, m: Msg| v.push(Issue::new(true, dev, link, m));
        let mut ids = Vec::new();
        for d in &self.devices {
            if ids.contains(&d.id) {
                err(&mut v, Some(d.id), None, msg!("Die Gerätenummer {} kommt doppelt vor.", d.id));
            }
            ids.push(d.id);
            match catalog::model(&d.model) {
                None => err(&mut v, Some(d.id), None, ProjErr::UnknownModel(d.model.clone()).msg()),
                Some(m) if m.has(F_ANNOTATION) => err(&mut v, Some(d.id), None, ProjErr::NotADevice(d.model.clone()).msg()),
                Some(_) => {}
            }
            if !valid_name(&d.name) {
                v.push(Issue::new(false, Some(d.id), None, ProjErr::BadName(d.name.clone()).msg()));
            }
            if self.devices.iter().any(|o| o.id != d.id && o.name.eq_ignore_ascii_case(&d.name)) {
                err(&mut v, Some(d.id), None, ProjErr::NameTaken(d.name.clone()).msg());
            }
        }
        let mut lids = Vec::new();
        let mut used: Vec<(u32, String)> = Vec::new();
        for l in &self.links {
            if lids.contains(&l.id) {
                err(&mut v, None, Some(l.id), msg!("Die Verbindungsnummer {} kommt doppelt vor.", l.id));
            }
            lids.push(l.id);
            if l.a.dev == l.b.dev {
                err(&mut v, Some(l.a.dev), Some(l.id), Why::SameDevice.msg());
                continue;
            }
            let (pa, pb) = match (self.resolve_port(&l.a), self.resolve_port(&l.b)) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(e), _) | (_, Err(e)) => {
                    err(&mut v, None, Some(l.id), e.msg());
                    continue;
                }
            };
            for (r, p) in [(&l.a, &pa), (&l.b, &pb)] {
                let key = (r.dev, p.name.clone());
                if used.contains(&key) {
                    err(&mut v, Some(r.dev), Some(l.id), Why::PortInUse(p.name.clone()).msg());
                }
                used.push(key);
            }
            if let Err(w) = cable::cable_fits(l.cable, &pa, &pb) {
                err(&mut v, None, Some(l.id), w.msg());
            }
            if l.kbps == 0 {
                v.push(Issue::new(false, None, Some(l.id), msg!("Die Verbindung hat keine Geschwindigkeit.")));
            }
        }
        if self.next_id <= self.max_id() {
            v.push(Issue::new(false, None, None, msg!("Die nächste freie Nummer ist zu klein.")));
        }
        v
    }

    // ---------------------------------------------------------------- file

    /// the project as the text of a .jfnet file (indented JSON, LF line ends)
    pub fn to_text(&self) -> String {
        let mut s = json::stringify_pretty(&self.to_json(), file_limits(), 2).unwrap_or_default();
        s.push('\n');
        s
    }
    /// reads a .jfnet file; missing members take their defaults, unknown ones are ignored
    pub fn from_text(text: &str) -> Result<Project, LoadErr> {
        let v = json::parse_with_limits(text.as_bytes(), file_limits()).map_err(|e| LoadErr::new(e.offset, msg!("Die Datei ist kein gültiges JSON ({} an Byte {}).", e.kind, e.offset)))?;
        let mut p = Project::from_json(&v).ok_or_else(|| LoadErr::new(0, msg!("Die Datei enthält kein Projekt.")))?;
        if p.format != FORMAT {
            return Err(LoadErr::new(0, msg!("Das ist keine JFNetworker-Datei.")));
        }
        if p.version == 0 || p.version > VERSION {
            return Err(LoadErr::new(0, msg!("Die Datei hat die Version {}; dieses Programm liest Version 1 bis {}.", p.version, VERSION)));
        }
        p.next_id = p.next_id.max(p.max_id() + 1);
        Ok(p)
    }
}
