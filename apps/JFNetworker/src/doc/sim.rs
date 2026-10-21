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
//   keep(&device)                 the engine takes the device's startup configuration and
//                                 which of its services run without a restart (a saved
//                                 configuration, a switched service; also their undo)
//   mode() / view_at() / view_time() / with_engine(f)   the simulation mode: the driver's
//                                 mode, the moment the view shows, a change of the engine
//                                 that is no command (PDUs: a ping job, a service, a lookup)
//
// Doc calls sync after every change of the project (edits, undo, redo), so undo and redo send
// the opposite actions and project and engine never part.  Everything runs on the UI thread
// (jfn_sim::Inline); the clock is plat::task::now_us, which a script run drives with `wait`.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::{CableKind, Device, Link, PortRef, Project, ServiceDef};
use jfn_sim::{ActErr, Action, Cmd, Engine, Inline, Led, Mode, Ns, Problem, Report, Tick, View, SEC};

/// reports kept for the terminals at most (the oldest go)
const REPORTS_MAX: usize = 4096;
/// the timer while something changes (ms), and at most between two ticks
const TICK_MS: u32 = 16;
const TICK_MAX_MS: u32 = 250;
/// while only the clock moves, a new snapshot at most this often (µs)
const SHOT_US: u64 = 50_000;

/// what the engine was told of a device
#[derive(Clone, PartialEq)]
struct MDev {
    name: String,
    model: String,
    off: bool,
    startup: String,
    services: Vec<ServiceDef>,
}

impl MDev {
    fn of(d: &Device) -> MDev {
        MDev { name: d.name.clone(), model: d.model.clone(), off: d.off, startup: d.startup_config.clone(), services: d.services.clone() }
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
}

impl MLink {
    fn of(l: &Link) -> MLink {
        MLink { a: l.a.clone(), b: l.b.clone(), cable: l.cable, kbps: l.kbps, delay_ns: l.delay_ns, jitter_ns: l.jitter_ns, loss_ppm: l.loss_ppm, down: l.down }
    }
}

/// the new value when it differs from the old one
fn diff<T: PartialEq + Copy>(old: T, new: T) -> Option<T> {
    if old != new { Some(new) } else { None }
}

pub(crate) struct Sim {
    drv: Inline,
    devs: BTreeMap<u32, MDev>,
    links: BTreeMap<u32, MLink>,
    /// the running configurations of removed devices (an undo brings them back as they ran)
    stash: BTreeMap<u32, String>,
    /// devices and links of the project the engine refused: id -> why (German)
    pub errors: BTreeMap<u32, String>,
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
        };
        if whole {
            for d in &p.devices {
                s.devs.insert(d.id, MDev::of(d));
                s.fix_name(d);
            }
            for l in &p.links {
                s.links.insert(l.id, MLink::of(l));
            }
        } else {
            // piece by piece: what the engine takes runs, the rest is in `errors`
            s.sync(p);
        }
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
            self.devs.insert(d.id, now);
        }
        // new links; kind, plug and quality
        for l in &p.links {
            let Some(m) = self.links.get(&l.id).cloned() else {
                match self.act(Action::Connect(l.clone())) {
                    Ok(()) => {
                        self.links.insert(l.id, MLink::of(l));
                        self.errors.remove(&l.id);
                    }
                    Err(e) => {
                        self.errors.insert(l.id, e.to_string());
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
                        self.errors.insert(l.id, e.to_string());
                    }
                }
            }
            if m.down != l.down && self.act(Action::Unplug { link: l.id, down: l.down }).is_ok() {
                now.down = l.down;
            }
            // a speed of 0 is no speed (the engine keeps the ports' one)
            let kbps = if l.kbps == 0 { None } else { diff(m.kbps, l.kbps) };
            let (delay_ns, jitter_ns, loss_ppm) = (diff(m.delay_ns, l.delay_ns), diff(m.jitter_ns, l.jitter_ns), diff(m.loss_ppm, l.loss_ppm));
            if (kbps, delay_ns, jitter_ns, loss_ppm) != (None, None, None, None) && self.act(Action::Impair { link: l.id, kbps, delay_ns, jitter_ns, loss_ppm }).is_ok() {
                (now.kbps, now.delay_ns, now.jitter_ns, now.loss_ppm) = (l.kbps, l.delay_ns, l.jitter_ns, l.loss_ppm);
            }
            self.links.insert(l.id, now);
        }
        // refusals of things the project no longer has
        self.errors.retain(|id, _| p.device(*id).is_some() || p.link(*id).is_some());
        if self.topo != before {
            self.changed();
        }
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
                self.errors.insert(d.id, e.to_string());
            }
        }
    }

    // the project's name counts: a configuration may name the device otherwise ("hostname")
    fn fix_name(&mut self, d: &Device) {
        if self.drv.eng.node(d.id).is_some_and(|n| n.name != d.name) {
            let _ = self.act(Action::Hostname { dev: d.id, name: d.name.clone() });
        }
    }

    /// the engine runs this device as the project has it now: its configuration was applied
    /// by the caller (no restart), the engine's startup configuration follows
    pub(crate) fn adopt(&mut self, d: &Device) {
        let Some(m) = self.devs.get(&d.id).cloned() else { return };
        if m.startup != d.startup_config {
            let _ = self.act(Action::Startup { dev: d.id, text: d.startup_config.clone() });
        }
        self.devs.insert(d.id, MDev { startup: d.startup_config.clone(), services: d.services.clone(), ..m });
    }

    /// a command of the UI (its effect is in the next snapshot, taken at once)
    pub(crate) fn send(&mut self, c: Cmd) -> Result<Tick, ActErr> {
        self.catch_up();
        self.topo = self.topo.wrapping_add(1);
        let r = self.drv.send(c, plat::task::now_us());
        self.changed();
        r
    }

    // something changed: a snapshot now, the timer soon
    fn changed(&mut self) {
        self.snapshot();
        self.soon = true;
    }

    // the view, the LEDs, the reports
    fn snapshot(&mut self) {
        let v = self.drv.eng.view();
        let mut leds = BTreeMap::new();
        for l in &v.links {
            leds.insert(l.id, l.led);
        }
        if leds != self.leds {
            let mut h: u32 = 0x811C_9DC5;
            for (id, l) in &leds {
                for x in [*id, l[0] as u32, l[1] as u32] {
                    h = (h ^ x).wrapping_mul(0x0100_0193);
                }
            }
            self.led_key = h;
            self.leds = leds;
            // loops count the cables that are up: the problems follow the LEDs
            self.topo = self.topo.wrapping_add(1);
        }
        // frames per second on every cable: what both ends sent, sampled once a second
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
        self.rates.retain(|id, _| self.leds.contains_key(id));
        self.view = Rc::new(v);
        self.ver = self.ver.wrapping_add(1);
        self.dirty = false;
        self.shot_at = plat::task::now_us();
        self.drain();
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
        let ms = if self.soon {
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
    /// it boots with (Action::Startup) and which of its services run (Action::ServiceRun) --
    /// "copy running-config startup-config", a service switched in the device window, and
    /// the undo and redo of both (Doc::apply_kept); other changes of its services still
    /// restart it at the next sync
    pub(crate) fn keep(&mut self, d: &Device) {
        let Some(m) = self.devs.get(&d.id).cloned() else { return };
        let same = m.services.len() == d.services.len() && m.services.iter().zip(&d.services).all(|(a, b)| a.kind == b.kind && a.name == b.name && a.config == b.config);
        if same {
            for (a, b) in m.services.iter().zip(&d.services) {
                if a.disabled != b.disabled {
                    let _ = self.act(Action::ServiceRun { dev: d.id, name: b.name.clone(), on: !b.disabled });
                }
            }
        }
        if m.startup != d.startup_config {
            let _ = self.act(Action::Startup { dev: d.id, text: d.startup_config.clone() });
        }
        let services = if same { d.services.clone() } else { m.services.clone() };
        self.devs.insert(d.id, MDev { startup: d.startup_config.clone(), services, ..m });
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
    /// workspace's PDUs); like send, a snapshot follows
    pub(crate) fn with_engine<R>(&mut self, f: impl FnOnce(&mut Engine) -> R) -> R {
        self.catch_up();
        self.topo = self.topo.wrapping_add(1);
        let r = f(&mut self.drv.eng);
        self.changed();
        r
    }
}
