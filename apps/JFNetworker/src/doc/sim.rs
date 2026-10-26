// sim -- the simulator behind the editor: a jfn_sim engine that follows the project, its
// clock, the snapshot the canvas and the inspector read, and the one place where commands go
// and the engine's reports arrive
//
//   Sim::new(&project)            an engine for the project (Engine::from_project; when the
//                                 engine refuses a part it runs without it, `errors` says why)
//   start()                       realtime from now on the task clock (a script run's virtual
//                                 clock: tests repeat exactly)
//   sync(&project)                the engine follows the project: what differs from what it
//                                 was told becomes Actions -- devices come and go (a removed one
//                                 keeps its running configuration for an undo), names, power;
//                                 cables come and go, kind, plug, speed, delay, jitter, loss; a
//                                 device whose model, configuration or services changed starts
//                                 again with them (keeping the project's name whatever its
//                                 configuration's "hostname" says)
//   adopt(&device)                the engine runs this device as the project has it already (a
//                                 setting the inspector applied itself; later the console's
//                                 "copy running-config startup-config")
//   send(Cmd) -> Result<Tick>     every command of the UI goes here (U3: Cli, Interrupt; U4: the
//                                 modes and steps); console output and changes collect for
//                                 take_reports()
//   arm() -> ms / tick()          the timer (UIC_TIMER): the clock follows the wall clock
//   view / leds / ver             the snapshot after the last change for the canvas and the
//                                 inspector; ver changes with it, led_key with the LEDs
//   problems()                    Engine::problems(), kept until the engine changes
//   engine()                      the engine for questions (interfaces, routes, configuration)
//   keep(&device)                 the engine takes the device's startup configuration, which
//                                 of its services run and its scripts without a restart (a
//                                 saved configuration, a switched service, the script editor's
//                                 steps; also their undo): keep_steps(dev, old, new, has)
//                                 makes the Actions, a script's new source keeps its disk;
//                                 its files too (Action::Files, also in sync: files never
//                                 restart a device)
//   mode() / view_at() / view_time() / with_engine(f)   the simulation mode: the driver's
//                                 mode, the moment the view shows, a change of the engine
//                                 that is no command (PDUs: a ping job, a service, a lookup)
//   moved(&project)               WLAN: the positions of a drag in progress go to the engine
//                                 at once (Action::Move), so stations come and go live
//   wlan / wlan_key               the WLAN associations of the last snapshot (the canvas draws
//                                 them, its version follows wlan_key); sync also sends the
//                                 devices' WLAN settings (Action::Wlan, no restart), their
//                                 positions and the project's scale (Action::Scale)
//
// The physical view (P1): sync tells the engine every cable's length as the project resolves it
// (jfn_model::phys::length: by hand or from the places; Action::CableLen, and Connect carries it
// in Link::len_m), the delay that follows from it (phys::delay_ns; Action::Impair) and the
// optics of the devices' cages (Action::Optic, no restart).
//
// Doc calls sync after every change of the project (edits, undo, redo), so undo and redo send
// the opposite actions and project and engine never part.  Everything runs on the UI thread
// (jfn_sim::Inline); the clock is plat::task::now_us, which a script run drives with `wait`.
//
// Rewinding (K1, jfn_sim::keyframe): the driver keeps keyframes within Budget::default() (64 MiB,
// 256 keyframes) and records what changes the engine (with_engine names its change and runs it
// again in every replay).  While the view stands in the past the devices show their state at
// that moment:
//   state()                       the engine whose state the window shows: the past one once it
//                                 is restored, else the present (the canvas' view, LEDs and
//                                 WLAN, `view` / `leds` / `wlan`, follow it; engine() stays the
//                                 present for the recording, PDUs, faults and edits)
//   past() / restoring() / past_gone()   the moment of the state shown; a replay on its way (per
//                                 mille); a moment before the earliest keyframe (the present
//                                 is shown)
//   past_line(dev, line)          a console line in the past: it runs on a copy of the state
//                                 shown and changes nothing; what the copy printed
//   resume(&project)              "Von hier fortsetzen": the state shown becomes the present,
//                                 the recording and inputs after it go, the engine is told the
//                                 project as it is now (what the engine was told then comes back
//                                 first, so a change of the project after that moment takes
//                                 effect at it); the project and its undo list stay
// A short replay runs at once (RESTORE_NOW events), a longer one in slices of RESTORE_TICK per
// timer tick, meanwhile the state shown before stays.  A change that goes to the engine (an
// edit, a setting, a console line of a form, a fault, a PDU) brings the view back to the
// present first; moving devices does not.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::{CableKind, Device, Link, PortRef, Project, ServiceDef};
use jfn_sim::{ActErr, Action, Cmd, Engine, Inline, Led, Mode, Ns, Problem, Report, Tick, View, SEC};
use jfn_sim::{Budget, Restore};

/// reports kept for the terminals at most (the oldest go)
const REPORTS_MAX: usize = 4096;
/// the timer while something changes (ms), and at most between two ticks
const TICK_MS: u32 = 16;
const TICK_MAX_MS: u32 = 250;
/// while only the clock moves, a new snapshot at most this often (µs)
const SHOT_US: u64 = 50_000;
/// events a replay to a past moment runs at once, and per timer tick after that (K1)
const RESTORE_NOW: usize = 50_000;
const RESTORE_TICK: usize = 20_000;

/// what the engine was told of a device
#[derive(Clone, PartialEq)]
struct MDev {
    name: String,
    model: String,
    off: bool,
    startup: String,
    services: Vec<ServiceDef>,
    /// WLAN: its settings and position (no restart for either)
    wlan: Option<jfn_model::Wlan>,
    pos: (i32, i32),
    /// its files (no restart either)
    files: Vec<jfn_model::files::File>,
    /// the optics in its cages (no restart)
    optics: BTreeMap<String, jfn_model::Optic>,
}

impl MDev {
    fn of(d: &Device) -> MDev {
        MDev { name: d.name.clone(), model: d.model.clone(), off: d.off, startup: d.startup_config.clone(), services: d.services.clone(), wlan: d.wlan.clone(), pos: (d.x, d.y), files: d.files.clone(), optics: d.optics.clone() }
    }

    /// the device must start again: another model, configuration or services
    fn restarts(&self, d: &Device) -> bool {
        self.model != d.model || self.startup != d.startup_config || self.services != d.services
    }
}

/// what the engine was told of a link
#[derive(Clone, PartialEq)]
struct MLink {
    a: PortRef,
    b: PortRef,
    cable: CableKind,
    kbps: u32,
    delay_ns: u64,
    jitter_ns: u64,
    loss_ppm: u32,
    down: bool,
    /// the length the project resolves (by hand or from the places; phys.rs)
    len_m: u32,
}

impl MLink {
    /// a link as the engine runs it: its resolved length and the delay that follows (phys.rs)
    fn of(l: &Link, len_m: u32, delay_ns: u64) -> MLink {
        MLink { a: l.a.clone(), b: l.b.clone(), cable: l.cable, kbps: l.kbps, delay_ns, jitter_ns: l.jitter_ns, loss_ppm: l.loss_ppm, down: l.down, len_m }
    }
}

/// what the engine was told of the project (K1: "Von hier fortsetzen" goes back to it)
#[derive(Clone, PartialEq)]
struct Told {
    devs: BTreeMap<u32, MDev>,
    links: BTreeMap<u32, MLink>,
    stash: BTreeMap<u32, String>,
    scale: u32,
}

/// a snapshot of an engine for the canvas and the inspector
struct Shot {
    view: Rc<View>,
    leds: BTreeMap<u32, [Led; 2]>,
    led_key: u32,
    wlan: Rc<Vec<jfn_sim::AssocView>>,
    wlan_key: u32,
}

/// the snapshot of engine `e` (its view given)
fn shot_of(e: &Engine, v: View) -> Shot {
    let mut leds = BTreeMap::new();
    // the cables of the project (WLAN associations have their own key)
    for l in v.links.iter().filter(|l| !jfn_sim::wlan::is_assoc_id(l.id)) {
        leds.insert(l.id, l.led);
    }
    let mut led_key: u32 = 0x811C_9DC5;
    for (id, l) in &leds {
        for x in [*id, l[0] as u32, l[1] as u32] {
            led_key = (led_key ^ x).wrapping_mul(0x0100_0193);
        }
    }
    let assocs = e.wlan_assocs();
    let mut wlan_key: u32 = 0x811C_9DC5;
    for a in &assocs {
        for x in [a.id, a.sta, a.ap, a.dbm as u32, a.kbps, a.lost as u32 | (a.authorized as u32) << 1] {
            wlan_key = (wlan_key ^ x).wrapping_mul(0x0100_0193);
        }
    }
    Shot { view: Rc::new(v), leds, led_key, wlan: Rc::new(assocs), wlan_key }
}

/// the view in the past and the devices' state then (K1)
struct Past {
    /// the moment the view shows
    at: Ns,
    /// the replay on its way to `at`
    job: Option<Restore>,
    /// the state shown (at `shown`; None before the first replay arrived)
    state: Option<jfn_sim::Past>,
    shown: Ns,
    /// `at` lies before the earliest keyframe: the present is shown
    gone: bool,
    /// the present's snapshot while the past one is shown
    live: Option<Shot>,
}

/// a project's link with the length and delay its places give it (what Action::Connect takes)
fn resolved(p: &Project, tree: &jfn_model::Tree, l: &Link) -> (Link, u32, u64) {
    let len = jfn_model::phys::length(p, tree, l).m;
    let delay = jfn_model::phys::delay_ns(p, tree, l);
    let mut x = l.clone();
    x.len_m = len;
    x.delay_ns = delay;
    (x, len, delay)
}

/// the new value when it differs from the old one
fn diff<T: PartialEq + Copy>(old: T, new: T) -> Option<T> {
    if old != new { Some(new) } else { None }
}

/// the actions that take a running device's services from `old` (what the engine was told)
/// to `new` (the project) without a restart, None when that cannot be done (the next sync
/// restarts the device): switched services (ServiceRun); a script's new definition
/// (ScriptSource, which keeps its disk and numbers; one the engine does not run yet comes as
/// Service, one that does not compile goes, as when a project loads); new and removed scripts;
/// a renamed script (one gone, one new, the same definition, or the same name in another case:
/// RenameService).  `has` tells whether the engine runs a script of that name.  Other services
/// that come, go or change: None.
pub(crate) fn keep_steps(dev: u32, old: &[ServiceDef], new: &[ServiceDef], has: &dyn Fn(&str) -> bool) -> Option<Vec<Action>> {
    fn script(s: &ServiceDef) -> bool {
        s.kind == crate::doc::scripts::KIND
    }
    fn same(a: &ServiceDef, b: &ServiceDef) -> bool {
        a.name.eq_ignore_ascii_case(&b.name)
    }
    fn compiles(s: &ServiceDef) -> bool {
        jfn_sim::scriptsvc::from_def(&s.config).is_ok()
    }
    let gone: Vec<&ServiceDef> = old.iter().filter(|&a| !new.iter().any(|b| same(a, b))).collect();
    let added: Vec<&ServiceDef> = new.iter().filter(|&b| !old.iter().any(|a| same(a, b))).collect();
    if gone.iter().chain(added.iter()).any(|&s| !script(s)) {
        return None;
    }
    let mut acts = Vec::new();
    match (&gone[..], &added[..]) {
        (&[a], &[b]) if a.config == b.config => {
            acts.push(Action::RenameService { dev, name: a.name.clone(), to: b.name.clone() });
            if a.disabled != b.disabled {
                acts.push(Action::ServiceRun { dev, name: b.name.clone(), on: !b.disabled });
            }
        }
        _ => {
            for &a in &gone {
                acts.push(Action::RemoveService { dev, name: a.name.clone() });
            }
            for &b in &added {
                if compiles(b) {
                    acts.push(Action::Service { dev, def: b.clone() });
                }
            }
        }
    }
    for b in new {
        let Some(a) = old.iter().find(|&a| same(a, b)) else { continue };
        if a.kind != b.kind || (a.config != b.config && !script(b)) {
            return None;
        }
        if a.name != b.name {
            acts.push(Action::RenameService { dev, name: a.name.clone(), to: b.name.clone() });
        }
        if a.config != b.config {
            match (compiles(b), has(&a.name)) {
                (true, true) => acts.push(Action::ScriptSource { dev, svc: b.name.clone(), config: b.config.clone() }),
                (true, false) => acts.push(Action::Service { dev, def: b.clone() }),
                (false, true) => acts.push(Action::RemoveService { dev, name: b.name.clone() }),
                (false, false) => {}
            }
        }
        if a.disabled != b.disabled {
            acts.push(Action::ServiceRun { dev, name: b.name.clone(), on: !b.disabled });
        }
    }
    Some(acts)
}

pub(crate) struct Sim {
    drv: Inline,
    devs: BTreeMap<u32, MDev>,
    links: BTreeMap<u32, MLink>,
    /// the running configurations of removed devices (an undo brings them back as they ran)
    stash: BTreeMap<u32, String>,
    /// devices and links of the project the engine refused: id -> why (German)
    pub errors: BTreeMap<u32, jfn_model::Msg>,
    /// the engine's state after the last change
    pub view: Rc<View>,
    /// link id -> the LEDs at end a and end b
    pub leds: BTreeMap<u32, [Led; 2]>,
    /// changes with every snapshot
    pub ver: u32,
    /// changes with the LEDs (the canvas paints them)
    pub led_key: u32,
    /// link id -> frames on it per second: (time and frames of the last sample, the rate then,
    /// the frames at the last snapshot)
    rates: BTreeMap<u32, (Ns, u64, u32, u64)>,
    /// changes with what the engine was told (problems depend on it)
    topo: u32,
    problems: Option<(u32, Rc<Vec<Problem>>)>,
    reports: VecDeque<Report>,
    /// the timer is set
    armed: bool,
    /// tick soon: something just changed
    soon: bool,
    /// events ran since the last snapshot
    dirty: bool,
    /// the task clock at the last snapshot (µs)
    shot_at: u64,
    /// the scale the engine was told (Project::dip_per_m)
    scale: u32,
    /// the WLAN associations of the last snapshot, a key that changes with them
    pub wlan: Rc<Vec<jfn_sim::AssocView>>,
    pub wlan_key: u32,
    /// the view in the past and the state shown (K1)
    past: Option<Past>,
    /// what the engine was told, after input number n
    told: Vec<(usize, Rc<Told>)>,
}

impl Sim {
    /// an engine for the project, paused until start()
    pub(crate) fn new(p: &Project) -> Sim {
        let (eng, whole) = match Engine::from_project(p) {
            Ok(e) => (e, true),
            Err(_) => {
                let mut e = Engine::new(p.seed);
                e.strict_mdix = p.strict_mdix;
                (e, false)
            }
        };
        let mut s = Sim {
            drv: Inline::new(eng),
            devs: BTreeMap::new(),
            links: BTreeMap::new(),
            stash: BTreeMap::new(),
            errors: BTreeMap::new(),
            view: Rc::new(View { now: 0, devices: Vec::new(), links: Vec::new(), pending: 0, next_hop: 0 }),
            leds: BTreeMap::new(),
            ver: 1,
            led_key: 0,
            rates: BTreeMap::new(),
            topo: 1,
            problems: None,
            reports: VecDeque::new(),
            armed: false,
            soon: true,
            dirty: false,
            shot_at: 0,
            scale: if whole { p.dip_per_m } else { 0 },
            wlan: Rc::new(Vec::new()),
            wlan_key: 0,
            past: None,
            told: Vec::new(),
        };
        if whole {
            for d in &p.devices {
                s.devs.insert(d.id, MDev::of(d));
                s.fix_name(d);
            }
            let tree = jfn_model::Tree::of(p);
            for l in &p.links {
                let (_, len, delay) = resolved(p, &tree, l);
                s.links.insert(l.id, MLink::of(l, len, delay));
            }
        } else {
            // piece by piece: what the engine takes runs, the rest is in `errors`
            s.sync(p);
        }
        // keyframes and inputs from here on (K1)
        s.drv.rewind_on(Budget::default());
        s.note_told();
        s.snapshot();
        s
    }

    /// the clock runs in real time from now on
    pub(crate) fn start(&mut self) {
        let _ = self.drv.send(Cmd::Realtime { speed_pm: 1000 }, plat::task::now_us());
        self.soon = true;
    }

    /// the engine, for questions
    pub(crate) fn engine(&self) -> &Engine {
        &self.drv.eng
    }

    /// the LEDs of a link (end a, end b)
    pub(crate) fn led(&self, link: u32) -> Option<[Led; 2]> {
        self.leds.get(&link).copied()
    }

    /// frames per second on a link (both directions, over the last second of virtual time)
    pub(crate) fn rate(&self, link: u32) -> u32 {
        let Some(&(t, n, rate, last)) = self.rates.get(&link) else { return 0 };
        let now = self.drv.eng.now();
        if now >= t + 2 * SEC {
            // no new sample for a while: what came since the last one
            return (last.saturating_sub(n) as u128 * SEC as u128 / (now - t) as u128) as u32;
        }
        rate
    }

    // in realtime, the engine first catches up with the wall clock (a change happens now)
    fn catch_up(&mut self) {
        if let Mode::Realtime { .. } = self.drv.mode() {
            if self.drv.tick(plat::task::now_us()).events > 0 {
                self.dirty = true;
            }
        }
    }

    fn act(&mut self, a: Action) -> Result<(), ActErr> {
        if !matches!(a, Action::Move { .. } | Action::Scale { .. }) {
            self.to_live();
        }
        self.topo = self.topo.wrapping_add(1);
        self.drv.send(Cmd::Apply(a), plat::task::now_us()).map(|_| ())
    }

    /// the engine follows the project (after every change of it)
    pub(crate) fn sync(&mut self, p: &Project) {
        self.catch_up();
        let before = self.topo;
        // devices that are gone, or start again with another model, configuration or services
        let out: Vec<(u32, bool)> = self
            .devs
            .iter()
            .filter_map(|(&id, m)| match p.device(id) {
                None => Some((id, false)),
                Some(d) if m.restarts(d) => Some((id, true)),
                Some(_) => None,
            })
            .collect();
        for (id, again) in out {
            if !again {
                let run = self.drv.eng.running_config(id);
                self.stash.insert(id, run);
            }
            let _ = self.act(Action::RemoveDevice(id));
            self.devs.remove(&id);
            self.links.retain(|_, l| l.a.dev != id && l.b.dev != id);
        }
        // links that are gone or join other ports now
        let out: Vec<u32> = self.links.iter().filter(|(id, m)| p.link(**id).is_none_or(|l| l.a != m.a || l.b != m.b)).map(|(&id, _)| id).collect();
        for id in out {
            let _ = self.act(Action::Disconnect(id));
            self.links.remove(&id);
        }
        // new devices; names and power
        for d in &p.devices {
            let Some(m) = self.devs.get(&d.id).cloned() else {
                self.add_device(d);
                continue;
            };
            let mut now = m.clone();
            if m.name != d.name && self.act(Action::Hostname { dev: d.id, name: d.name.clone() }).is_ok() {
                now.name = d.name.clone();
            }
            if m.off != d.off && self.act(Action::Power { dev: d.id, on: !d.off }).is_ok() {
                now.off = d.off;
            }
            // WLAN: settings and position (no restart)
            if m.wlan != d.wlan && self.act(Action::Wlan { dev: d.id, wlan: d.wlan.clone() }).is_ok() {
                now.wlan = d.wlan.clone();
            }
            if m.pos != (d.x, d.y) && self.act(Action::Move { dev: d.id, x: d.x, y: d.y }).is_ok() {
                now.pos = (d.x, d.y);
            }
            // files: the store takes the project's (no restart)
            if m.files != d.files && self.act(Action::Files { dev: d.id, files: d.files.clone() }).is_ok() {
                now.files = d.files.clone();
            }
            // the optics in its cages (phys.rs, no restart)
            if m.optics != d.optics {
                let ports: alloc::collections::BTreeSet<String> = m.optics.keys().chain(d.optics.keys()).cloned().collect();
                for port in ports {
                    let optic = d.optics.get(&port).copied().unwrap_or(jfn_model::Optic::Sr);
                    if m.optics.get(&port).copied().unwrap_or(jfn_model::Optic::Sr) != optic {
                        let _ = self.act(Action::Optic { dev: d.id, port, optic });
                    }
                }
                now.optics = d.optics.clone();
            }
            self.devs.insert(d.id, now);
        }
        // the scale of the canvas (the ranges of the radios)
        if self.scale != p.dip_per_m && self.act(Action::Scale { dip_per_m: p.dip_per_m }).is_ok() {
            self.scale = p.dip_per_m;
        }
        // new links; kind, plug and quality; the lengths and delays of the places (phys.rs)
        let tree = jfn_model::Tree::of(p);
        for l in &p.links {
            let (x, len, delay) = resolved(p, &tree, l);
            let Some(m) = self.links.get(&l.id).cloned() else {
                match self.act(Action::Connect(x)) {
                    Ok(()) => {
                        self.links.insert(l.id, MLink::of(l, len, delay));
                        self.errors.remove(&l.id);
                    }
                    Err(e) => {
                        self.errors.insert(l.id, e.msg());
                    }
                }
                continue;
            };
            let mut now = m.clone();
            if m.cable != l.cable {
                match self.act(Action::SetCable { link: l.id, cable: l.cable }) {
                    Ok(()) => {
                        now.cable = l.cable;
                        self.errors.remove(&l.id);
                    }
                    Err(e) => {
                        self.errors.insert(l.id, e.msg());
                    }
                }
            }
            if m.down != l.down && self.act(Action::Unplug { link: l.id, down: l.down }).is_ok() {
                now.down = l.down;
            }
            // a speed of 0 is no speed (the engine keeps the ports' one)
            let kbps = if l.kbps == 0 { None } else { diff(m.kbps, l.kbps) };
            let (delay_ns, jitter_ns, loss_ppm) = (diff(m.delay_ns, delay), diff(m.jitter_ns, l.jitter_ns), diff(m.loss_ppm, l.loss_ppm));
            if (kbps, delay_ns, jitter_ns, loss_ppm) != (None, None, None, None) && self.act(Action::Impair { link: l.id, kbps, delay_ns, jitter_ns, loss_ppm }).is_ok() {
                (now.kbps, now.delay_ns, now.jitter_ns, now.loss_ppm) = (l.kbps, delay, l.jitter_ns, l.loss_ppm);
            }
            if m.len_m != len && self.act(Action::CableLen { link: l.id, len_m: len }).is_ok() {
                now.len_m = len;
            }
            self.links.insert(l.id, now);
        }
        // refusals of things the project no longer has
        self.errors.retain(|id, _| p.device(*id).is_some() || p.link(*id).is_some());
        if self.topo != before {
            self.changed();
        }
        self.note_told();
    }

    // a device of the project starts running (as it ran when an undo brings it back)
    fn add_device(&mut self, d: &Device) {
        let mut boot = d.clone();
        let ran = self.stash.remove(&d.id);
        if let Some(run) = &ran {
            boot.startup_config = run.clone();
        }
        match self.act(Action::AddDevice(boot)) {
            Ok(()) => {
                if ran.is_some() {
                    // it runs as it ran, it boots with the project's configuration
                    let _ = self.act(Action::Startup { dev: d.id, text: d.startup_config.clone() });
                }
                self.fix_name(d);
                self.devs.insert(d.id, MDev::of(d));
                self.errors.remove(&d.id);
            }
            Err(e) => {
                self.errors.insert(d.id, e.msg());
            }
        }
    }

    // the project's name counts: a configuration may name the device otherwise ("hostname")
    fn fix_name(&mut self, d: &Device) {
        if self.drv.eng.node(d.id).is_some_and(|n| n.name != d.name) {
            let _ = self.act(Action::Hostname { dev: d.id, name: d.name.clone() });
        }
    }

    /// a drag in progress: the positions of the devices with a radio go to the engine at once
    /// (WLAN stations come and go live); a snapshot only when WLAN changed (the step at the
    /// drag's end brings the rest)
    pub(crate) fn moved(&mut self, p: &Project) {
        let moved: Vec<(u32, i32, i32)> = p
            .devices
            .iter()
            .filter(|d| self.devs.get(&d.id).is_some_and(|m| m.pos != (d.x, d.y)))
            .filter(|d| jfn_model::catalog::model(&d.model).is_some_and(|m| jfn_model::wlan::role(m) != jfn_model::wlan::Role::None))
            .map(|d| (d.id, d.x, d.y))
            .collect();
        if moved.is_empty() {
            return;
        }
        self.catch_up();
        let ver = self.drv.eng.wlan_ver();
        for (id, x, y) in moved {
            if self.drv.send(Cmd::Apply(Action::Move { dev: id, x, y }), plat::task::now_us()).is_ok()
                && let Some(m) = self.devs.get_mut(&id)
            {
                m.pos = (x, y);
            }
        }
        if self.drv.eng.wlan_ver() != ver {
            self.topo = self.topo.wrapping_add(1);
            self.changed();
        }
        self.note_told();
    }

    /// the engine runs this device as the project has it now: its configuration was applied
    /// by the caller (no restart), the engine's startup configuration follows
    pub(crate) fn adopt(&mut self, d: &Device) {
        let Some(m) = self.devs.get(&d.id).cloned() else { return };
        if m.startup != d.startup_config {
            let _ = self.act(Action::Startup { dev: d.id, text: d.startup_config.clone() });
        }
        self.devs.insert(d.id, MDev { startup: d.startup_config.clone(), services: d.services.clone(), ..m });
        self.note_told();
    }

    /// a command of the UI (its effect is in the next snapshot, taken at once)
    pub(crate) fn send(&mut self, c: Cmd) -> Result<Tick, ActErr> {
        self.catch_up();
        let edit = match &c {
            Cmd::Apply(a) => !matches!(a, Action::Move { .. } | Action::Scale { .. }),
            Cmd::Cli { .. } | Cmd::Interrupt { .. } => true,
            _ => false,
        };
        if edit {
            self.to_live();
        }
        self.topo = self.topo.wrapping_add(1);
        let r = self.drv.send(c, plat::task::now_us());
        self.changed();
        self.follow();
        r
    }

    // something changed: a snapshot now, the timer soon
    fn changed(&mut self) {
        self.snapshot();
        self.soon = true;
    }

    // the view, the LEDs, the reports (while a past state is shown, the present's snapshot is
    // kept aside for when the view comes back)
    fn snapshot(&mut self) {
        let s = shot_of(&self.drv.eng, self.drv.eng.view());
        let shown = self.past.as_ref().and_then(|p| p.live.as_ref());
        if &s.leds != shown.map_or(&self.leds, |l| &l.leds) {
            // loops count the cables that are up: the problems follow the LEDs
            self.topo = self.topo.wrapping_add(1);
        }
        // frames per second on every cable: what both ends sent, sampled once a second
        let v = &s.view;
        let mut sent: BTreeMap<(u32, u16), u64> = BTreeMap::new();
        for d in &v.devices {
            for p in d.ports.iter().filter(|p| p.link.is_some()) {
                sent.insert((d.dev, p.index), p.counters.tx_frames);
            }
        }
        let now = v.now;
        for l in &v.links {
            let total = sent.get(&l.ends[0]).copied().unwrap_or(0) + sent.get(&l.ends[1]).copied().unwrap_or(0);
            let r = self.rates.entry(l.id).or_insert((now, total, 0, total));
            r.3 = total;
            if now >= r.0 + SEC {
                r.2 = (total.saturating_sub(r.1) as u128 * SEC as u128 / (now - r.0) as u128) as u32;
                (r.0, r.1) = (now, total);
            }
        }
        self.rates.retain(|id, _| s.leds.contains_key(id));
        match self.past.as_mut().filter(|p| p.live.is_some()) {
            Some(p) => p.live = Some(s),
            None => self.install(s),
        }
        self.dirty = false;
        self.shot_at = plat::task::now_us();
        self.drain();
    }

    // a snapshot becomes the one the window shows
    fn install(&mut self, s: Shot) {
        self.view = s.view;
        self.leds = s.leds;
        self.led_key = s.led_key;
        if s.wlan_key != self.wlan_key || s.wlan.len() != self.wlan.len() {
            self.wlan_key = s.wlan_key;
            self.wlan = s.wlan;
        }
        self.ver = self.ver.wrapping_add(1);
    }

    // the snapshot shown now
    fn current(&self) -> Shot {
        Shot { view: self.view.clone(), leds: self.leds.clone(), led_key: self.led_key, wlan: self.wlan.clone(), wlan_key: self.wlan_key }
    }

    // the engine's reports into the bounded queue
    fn drain(&mut self) {
        for r in self.drv.eng.take_reports() {
            if self.reports.len() == REPORTS_MAX {
                self.reports.pop_front();
            }
            self.reports.push_back(r);
        }
    }

    /// console output and console changes since the last call (doc/console.rs distributes
    /// them: consoles, saved configurations, names, the log)
    pub(crate) fn take_reports(&mut self) -> Vec<Report> {
        self.drain();
        self.reports.drain(..).collect()
    }

    /// the timer to set (ms), None while it is set or nothing will happen
    pub(crate) fn arm(&mut self) -> Option<u32> {
        if self.armed {
            return None;
        }
        let ms = if self.soon || self.restoring().is_some() {
            TICK_MS
        } else if let Mode::Realtime { speed_pm } = self.drv.mode() {
            match self.drv.eng.next_time() {
                // when the next event is due on the wall clock
                Some(t) => {
                    let d = t.saturating_sub(self.drv.eng.now()) / 1_000_000 * 1000 / speed_pm.max(1) as u64;
                    (d.min(TICK_MAX_MS as u64) as u32).max(TICK_MS)
                }
                None if self.dirty => TICK_MS,
                None => return None,
            }
        } else if self.dirty {
            TICK_MS
        } else {
            return None;
        };
        self.armed = true;
        self.soon = false;
        Some(ms)
    }

    /// the timer fired: the clock moves on -> a new snapshot was taken
    pub(crate) fn tick(&mut self) -> bool {
        self.armed = false;
        if self.restoring().is_some() {
            // the replay to the moment the view shows goes on
            self.soon = true;
            return self.work(RESTORE_TICK);
        }
        let now = plat::task::now_us();
        let t = self.drv.tick(now);
        if t.events > 0 || t.new_hops {
            self.dirty = true;
        }
        if self.dirty && now.saturating_sub(self.shot_at) >= SHOT_US {
            self.snapshot();
            return true;
        }
        self.drain();
        false
    }

    /// what is wrong with the set-up (Engine::problems, kept until the engine changes)
    pub(crate) fn problems(&mut self) -> Rc<Vec<Problem>> {
        if let Some((k, p)) = &self.problems {
            if *k == self.topo {
                return p.clone();
            }
        }
        let p = Rc::new(self.drv.eng.problems());
        self.problems = Some((self.topo, p.clone()));
        p
    }

    /// the engine runs device `d` as the project has it, without a restart: the configuration
    /// it boots with (Action::Startup), which of its services run (Action::ServiceRun) and
    /// its scripts (keep_steps: a new source keeps the script's disk and numbers, new,
    /// renamed and removed scripts) -- "copy running-config startup-config", a service
    /// switched in the device window, the script editor, and the undo and redo of them
    /// (Doc::apply_kept); other changes of its services still restart it at the next sync
    pub(crate) fn keep(&mut self, d: &Device) {
        let Some(m) = self.devs.get(&d.id).cloned() else { return };
        let eng = &self.drv.eng;
        let steps = keep_steps(d.id, &m.services, &d.services, &|name: &str| eng.service::<jfn_sim::ScriptService>(d.id, name).is_some());
        for a in steps.iter().flatten() {
            let _ = self.act(a.clone());
        }
        if m.startup != d.startup_config {
            let _ = self.act(Action::Startup { dev: d.id, text: d.startup_config.clone() });
        }
        // the files as the project has them now (a transfer's step, its undo)
        if m.files != d.files {
            let _ = self.act(Action::Files { dev: d.id, files: d.files.clone() });
        }
        let services = if steps.is_some() { d.services.clone() } else { m.services.clone() };
        self.devs.insert(d.id, MDev { startup: d.startup_config.clone(), services, files: d.files.clone(), ..m });
        self.note_told();
    }
}

// ---------------------------------------------------------------- the simulation mode (U4)

impl Sim {
    /// real time, paused or the simulation mode
    pub(crate) fn mode(&self) -> Mode {
        self.drv.mode()
    }

    /// the moment the view shows while it stands in the past (None: the present)
    pub(crate) fn view_at(&self) -> Option<Ns> {
        self.drv.view()
    }

    /// the moment the view shows (the engine's clock at the present)
    pub(crate) fn view_time(&self) -> Ns {
        self.drv.view_time()
    }

    /// a change of the engine that is no command: a ping job, a PDU's service, a lookup (the
    /// workspace's PDUs); like send, a snapshot follows.  `f` runs again in every replay of
    /// that moment (K1), so it takes what it needs by value; `what` names it in the inputs
    pub(crate) fn with_engine<R>(&mut self, what: &'static str, f: impl Fn(&mut Engine) -> R + Send + Sync + 'static) -> R {
        self.catch_up();
        self.to_live();
        self.topo = self.topo.wrapping_add(1);
        let r = self.drv.call(what, f);
        self.changed();
        r
    }
}

// ---------------------------------------------------------------- the state in the past (K1)

impl Sim {
    /// the engine whose state the window shows: the past one while the view stands in the past
    /// and its state is restored, else the present
    pub(crate) fn state(&self) -> &Engine {
        match self.past.as_ref() {
            Some(p) if p.live.is_some() => p.state.as_ref().map_or(&self.drv.eng, |s| &s.eng),
            _ => &self.drv.eng,
        }
    }

    /// the moment whose state of the devices is shown (None: the present)
    pub(crate) fn past(&self) -> Option<Ns> {
        self.past.as_ref().filter(|p| p.live.is_some() && p.state.is_some()).map(|p| p.shown)
    }

    /// a replay to the moment the view shows is on its way: how far (per mille)
    pub(crate) fn restoring(&self) -> Option<u32> {
        self.past.as_ref().and_then(|p| p.job.as_ref()).map(|j| j.progress())
    }

    /// the view shows a moment before the earliest keyframe (the devices show the present)
    pub(crate) fn past_gone(&self) -> bool {
        self.past.as_ref().is_some_and(|p| p.gone)
    }

    /// the present's view, also while a past one is shown (what changes the present looks at)
    pub(crate) fn live_view(&self) -> Rc<View> {
        match self.past.as_ref().and_then(|p| p.live.as_ref()) {
            Some(l) => l.view.clone(),
            None => self.view.clone(),
        }
    }

    /// the keyframes and inputs of the run
    pub(crate) fn rewind(&self) -> Option<&jfn_sim::Rewind> {
        self.drv.rewind()
    }

    /// another budget for the keyframes: the recording starts afresh from now (tests)
    #[allow(dead_code)]
    pub(crate) fn set_budget(&mut self, cfg: Budget) {
        self.drv.rewind_on(cfg);
        self.told.clear();
        self.note_told();
    }

    /// a line typed on a console while the view stands in the past: it runs on a copy of the
    /// state shown, which then goes, so nothing changes (a command that ran then, or a question
    /// that waited, is given up on the copy first); what the copy printed (None: no past state
    /// is shown)
    pub(crate) fn past_line(&self, dev: u32, line: &str) -> Option<Result<String, ActErr>> {
        let p = self.past.as_ref().filter(|p| p.live.is_some())?;
        let mut e = p.state.as_ref()?.eng.clone();
        if e.cli_busy(dev) || e.node(dev).is_some_and(|x| x.cli.ask.is_some()) {
            let _ = e.cli_interrupt(dev);
        }
        let _ = e.take_reports();
        if let Err(x) = e.cli_line(dev, line) {
            return Some(Err(x));
        }
        let mut s = String::new();
        for r in e.take_reports() {
            if let Report::CliOut { dev: d, bytes, .. } = r
                && d == dev
            {
                s.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        Some(Ok(s))
    }

    /// "Von hier fortsetzen": the state shown becomes the present -- the recording and what came
    /// from outside after its moment go; what the engine was told then comes back and the
    /// project as it is now follows (sync) -> false when no past state is shown
    pub(crate) fn resume(&mut self, proj: &Project) -> bool {
        if self.past().is_none() || self.restoring().is_some() {
            return false;
        }
        let Some(state) = self.past.take().and_then(|p| p.state) else { return false };
        let k = state.pos.inputs;
        self.drv.resume(state);
        if let Some(i) = self.told.iter().rposition(|(n, _)| *n <= k) {
            let t = self.told[i].1.clone();
            self.told.truncate(i + 1);
            self.devs = t.devs.clone();
            self.links = t.links.clone();
            self.stash = t.stash.clone();
            self.scale = t.scale;
        }
        self.reports.clear();
        self.rates.clear();
        self.problems = None;
        self.topo = self.topo.wrapping_add(1);
        self.sync(proj);
        self.changed();
        true
    }

    // the view goes back to the present before a change of the engine
    fn to_live(&mut self) {
        if self.drv.view().is_some() {
            let _ = self.drv.send(Cmd::Live, plat::task::now_us());
        }
        self.follow();
    }

    // the view moved: the devices' state at its moment -- from a keyframe, or from the state
    // shown when the view moved forward; a short replay at once, a longer one in slices
    fn follow(&mut self) {
        let Some(t) = self.drv.view() else {
            if let Some(l) = self.past.take().and_then(|p| p.live) {
                self.install(l);
            }
            return;
        };
        if self.past.as_ref().is_some_and(|p| p.at == t) {
            return;
        }
        let job = {
            let prev = self.past.as_ref().and_then(|p| p.state.as_ref());
            self.drv.restore(t, prev)
        };
        let p = self.past.get_or_insert_with(|| Past { at: t, job: None, state: None, shown: 0, gone: false, live: None });
        p.at = t;
        p.gone = job.is_none();
        p.job = job;
        // before the earliest keyframe there is nothing to show but the present
        let back = if p.gone { p.state = None; p.live.take() } else { None };
        if let Some(l) = back {
            self.install(l);
        }
        self.work(RESTORE_NOW);
        self.soon = true;
    }

    // the replay goes on (at most `budget` events) -> it arrived and its state is shown
    fn work(&mut self, budget: usize) -> bool {
        let shot = {
            let (Some(p), Some(rw)) = (self.past.as_mut(), self.drv.rewind()) else { return false };
            let Some(job) = p.job.as_mut() else { return false };
            if !job.run(rw, budget) {
                return false;
            }
            p.state = p.job.take().map(|j| j.finish());
            p.shown = p.at;
            match p.state.as_ref() {
                Some(s) => shot_of(&s.eng, s.eng.view()),
                None => return false,
            }
        };
        if self.past.as_ref().is_some_and(|p| p.live.is_none()) {
            let live = self.current();
            if let Some(p) = self.past.as_mut() {
                p.live = Some(live);
            }
        }
        self.install(shot);
        true
    }

    // what the engine was told, when it changed (resume goes back to it)
    fn note_told(&mut self) {
        if self.told.last().is_some_and(|(_, t)| t.devs == self.devs && t.links == self.links && t.stash == self.stash && t.scale == self.scale) {
            return;
        }
        let k = self.drv.pos().inputs;
        // what was told before the earliest keyframe can no longer come back
        let first = self.drv.rewind().map_or(0, |r| r.inputs().0);
        if let Some(i) = self.told.iter().rposition(|(n, _)| *n <= first) {
            self.told.drain(..i);
        }
        self.told.push((k, Rc::new(Told { devs: self.devs.clone(), links: self.links.clone(), stash: self.stash.clone(), scale: self.scale })));
    }
}
