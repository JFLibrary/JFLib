// units -- services as units on Linux hosts (A2): unit files on the device's file system
// (jfn_model::unit) run as the services of os.rs, with restart policies in virtual time; the
// services of the project show up as generated, read-only units
//
//   Os::units (Units)              per device: the units as last read with their state, the
//                                  project services systemctl stopped, the deployments (deploy.rs)
//   Engine::unit_op(dev, name, op) / Action::Unit   start, stop, restart, enable, disable a unit
//   Engine::units_reload(ni)       daemon-reload: every unit file read again
//   Engine::units(dev) / unit(dev, name)   the units as UnitView (what systemctl shows)
//   Engine::units_of_file(dev, path)   the units that were read and run script `path`, by the
//                                  names of their services, with their states (the IDE's F5)
//   Engine::units_boot(ni)         the device starts (add_node, power on): every unit file is
//                                  read, the enabled ones start, in the order of After=
//   Engine::units_power_off(ni), units_halted(ni, id, code), unit_timer(ni, id),
//   units_ctl(ni, id, on)          what os.rs tells: the power went off, a unit's service ended
//                                  by itself, its RestartSec is over, it was switched from outside
//                                  (script start|stop, a fault, Action::ServiceRun)
//   unit_journal(os, name)         whether a line of the log belongs to a unit
//
// Where the state lives.  What survives a power cycle and saving the project are the files, as on
// a real host: /etc/systemd/system/NAME.service and, for an enabled unit, a small file
// /etc/systemd/system/TARGET.wants/NAME.service that names it (the store has no symbolic links).
// Both go through the store like every file and reach the project as Report::Files.  Everything
// else -- active or not, the exit status, the restarts -- is run-time state and starts afresh.
//
// A unit runs as one service of os.rs under the name of its stem: SvcSlot::unit says so, and that
// slot starts when the unit says (os_power leaves it out).  The slot stays through stops and
// restarts, so a script keeps its disk, numbers and time series.  A new ExecStart (after a
// daemon-reload) takes a script's slot along (ScriptSource), any other kind replaces it.
//
// Reading.  Like the original, a loaded unit keeps what it read until "systemctl daemon-reload";
// UnitView::changed tells that the file is newer.  A unit nothing has read yet is read when it
// is first used.  A unit file of the same name takes a project service over: that service then
// starts only if the unit is enabled, from the file.
//
// The project's services are generated units: they show in list-units and status, start, stop and
// restart work, enable and disable are refused (the project says it, in the device's Dienste);
// stop is not for good: a power cycle starts them again if the project says so.  Without unit
// files and without a word typed, nothing here runs and no state changes, so old projects run as
// they did.
//
// Restart policies: a script that runs from a unit ends when nothing is left to do (status 0) or
// after an error that ends it (status 1: its main program fails, its file is missing, or any
// error with "stop_on_error"); Restart=on-failure starts it again after RestartSec when the status
// is not 0, Restart=always in both cases.  Stopping by request never restarts.  Like the original,
// more than StartLimitBurst starts within StartLimitIntervalSec make it give up (failed).  The
// timer of a restart is key 3 of the unit's service while that is stopped: no fixed slot, and os.rs
// comes here for a timer of a service that does not run.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::msg;
use jfn_model::msg::Msg;
use jfn_model::project::ServiceDef;
use jfn_model::unit::{self, Exec, Issue, Restart, Unit, UNIT_DIR};

use crate::kernel::{Engine, Ns, SEC};
use crate::os::{Job, LogLine, Os, SLOT_SVC, SVC_TIMERS};
use crate::scriptsvc::{ScriptService, ScriptState};
use crate::trace::Note;

/// the timer key of a unit's restart (its service does not run then)
const K_RESTART: u32 = 3;
/// the log's tag of what the unit manager says
pub const LOG_TAG: &str = "systemd";

/// what a unit does now
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitState {
    Inactive,
    Active,
    Failed,
    /// it ended and starts again after RestartSec
    AutoRestart,
}

impl UnitState {
    /// is-active's word
    pub fn active_word(self) -> &'static str {
        match self {
            UnitState::Active => "active",
            UnitState::Inactive => "inactive",
            UnitState::Failed => "failed",
            UnitState::AutoRestart => "activating",
        }
    }
    /// the word for what it does in more detail
    pub fn sub_word(self) -> &'static str {
        match self {
            UnitState::Active => "running",
            UnitState::Inactive => "dead",
            UnitState::Failed => "failed",
            UnitState::AutoRestart => "auto-restart",
        }
    }
}

/// whether a unit starts with the device
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitEnabled {
    Enabled,
    Disabled,
    /// the file has no [Install]: nothing to enable
    Static,
    /// a service of the project, which says it (started with the device)
    Generated,
    /// a service of the project that is switched off there
    GeneratedOff,
}

impl UnitEnabled {
    /// is-enabled's word
    pub fn word(self) -> &'static str {
        match self {
            UnitEnabled::Enabled => "enabled",
            UnitEnabled::Disabled | UnitEnabled::GeneratedOff => "disabled",
            UnitEnabled::Static => "static",
            UnitEnabled::Generated => "generated",
        }
    }
    /// is-enabled's exit status 0
    pub fn holds(self) -> bool {
        !matches!(self, UnitEnabled::Disabled | UnitEnabled::GeneratedOff)
    }
}

/// a unit a device runs
#[derive(Clone, Debug)]
pub struct UnitRt {
    pub unit: Unit,
    /// the text it was read from
    pub source: String,
    /// its service (from its first start on)
    pub svc: Option<u32>,
    pub state: UnitState,
    /// when the state began
    pub since: Ns,
    /// the status it ended with
    pub exit: Option<i32>,
    /// why it failed
    pub why: Option<String>,
    /// automatic restarts so far
    pub restarts: u32,
    /// when its restart is due
    pub due: Option<Ns>,
    /// its last starts (the start limit)
    starts: VecDeque<Ns>,
    /// the definition its service runs
    def: Option<(String, Value)>,
}

impl UnitRt {
    fn new(unit: Unit, source: String) -> UnitRt {
        UnitRt { unit, source, svc: None, state: UnitState::Inactive, since: 0, exit: None, why: None, restarts: 0, due: None, starts: VecDeque::new(), def: None }
    }
}

/// the units of a device
#[derive(Clone, Debug, Default)]
pub struct Units {
    pub(crate) map: BTreeMap<String, UnitRt>,
    /// the project services systemctl stopped (they come back with the next power on)
    pub(crate) held: Vec<u32>,
    /// the deployments this device started (deploy.rs)
    pub(crate) deploys: Vec<crate::deploy::DeployRun>,
}

impl Units {
    /// the bytes it holds (keyframe.rs)
    pub fn footprint(&self) -> usize {
        let units: usize = self.map.iter().map(|(k, u)| 256 + k.len() + u.source.len() * 2 + u.starts.len() * 8 + u.why.as_ref().map_or(0, |w| w.len())).sum();
        units + self.held.len() * 4 + self.deploys.iter().map(|d| d.footprint()).sum::<usize>()
    }
}

/// a unit as systemctl shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitView {
    /// "web.service"
    pub name: String,
    pub description: String,
    /// the unit file (None: a generated unit)
    pub file: Option<String>,
    /// the file has errors: it cannot start
    pub bad: bool,
    pub state: UnitState,
    pub enabled: UnitEnabled,
    /// ExecStart as written; a generated unit: the kind of its service
    pub exec: String,
    pub restart: Restart,
    pub restart_ns: Ns,
    pub restarts: u32,
    pub since: Ns,
    pub exit: Option<i32>,
    pub why: Option<String>,
    /// 100 + the service's id while it runs
    pub pid: Option<u32>,
    /// its file is newer than what it read (daemon-reload)
    pub changed: bool,
    pub issues: Vec<Issue>,
    pub after: Vec<String>,
    pub requires: Vec<String>,
}

/// why a unit command was refused
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitErr {
    /// there is no such unit
    NotFound(String),
    /// the unit file is wrong, or there is no such file to enable
    Bad(Msg),
    /// its start failed
    Failed(Msg),
    /// a generated unit cannot be enabled or disabled
    Generated(String),
}

impl UnitErr {
    /// the message for the console
    pub fn msg(&self) -> Msg {
        match self {
            UnitErr::NotFound(n) => msg!("Die Einheit {} gibt es nicht.", n),
            UnitErr::Bad(m) | UnitErr::Failed(m) => m.clone(),
            UnitErr::Generated(n) => msg!("{} kommt aus dem Projekt: ob der Dienst startet, steht dort (Gerätefenster, Dienste).", n),
        }
    }
}

/// what Action::Unit does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitOp {
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
}

/// does a line of the log belong to unit `name` ("web.service", its service id `svc`)
pub fn unit_journal(l: &LogLine, name: &str, svc: Option<u32>) -> bool {
    let st = unit::stem(name);
    if l.tag == LOG_TAG {
        return l.text.strip_prefix(name).is_some_and(|r| r.starts_with(':'));
    }
    (svc.is_some() && l.svc == svc.unwrap_or(0)) || l.tag.eq_ignore_ascii_case(st)
}

/// the units that are not targets among the files of `dir` (names)
fn unit_files(os: &Os, dir: &str) -> Vec<String> {
    os.files.list(dir).map(|v| v.into_iter().filter(|(n, s)| !s.dir && n.ends_with(".service")).map(|(n, _)| n).collect()).unwrap_or_default()
}

/// the ".wants" folders below the unit folder (their paths)
fn wants_dirs(os: &Os) -> Vec<String> {
    os.files.list(UNIT_DIR).map(|v| v.into_iter().filter(|(n, s)| s.dir && n.ends_with(".wants")).map(|(n, _)| format!("{}/{}", UNIT_DIR, n)).collect()).unwrap_or_default()
}

impl Engine {
    fn unit_text(&self, ni: usize, name: &str) -> Option<String> {
        let b = self.os(ni)?.files.read(&unit::path_of(name)).ok()?;
        core::str::from_utf8(b).ok().map(String::from)
    }

    /// the unit in the table, read from its file when nothing has read it yet
    fn unit_load(&mut self, ni: usize, name: &str) -> bool {
        if self.os(ni).is_some_and(|o| o.units.map.contains_key(name)) {
            return true;
        }
        let Some(text) = self.unit_text(ni, name) else { return false };
        let u = unit::parse(&text);
        self.unit_issues(ni, name, &u);
        if let Some(os) = self.os_mut(ni) {
            os.units.map.insert(String::from(name), UnitRt::new(u, text));
            return true;
        }
        false
    }

    /// what a unit file gets wrong goes into the log, as the original logs it
    fn unit_issues(&mut self, ni: usize, name: &str, u: &Unit) {
        for i in &u.issues {
            let sev = if i.error { 3 } else { 4 };
            self.os_log(ni, sev, LOG_TAG, &format!("{}: {}", name, i.msg.text()), None);
        }
    }

    fn unit_say(&mut self, ni: usize, sev: u8, name: &str, what: Msg) {
        self.os_log(ni, sev, LOG_TAG, &format!("{}: {}", name, what.text()), None);
        let dev = self.dev_of(ni);
        self.note(dev, None, Note::Unit { unit: String::from(name), what });
    }

    /// daemon-reload: every unit file is read again; a unit whose file went stays only while it
    /// runs
    pub(crate) fn units_reload(&mut self, ni: usize) {
        let names = match self.os(ni) {
            Some(os) => unit_files(os, UNIT_DIR),
            None => return,
        };
        for name in &names {
            let Some(text) = self.unit_text(ni, name) else { continue };
            let same = self.os(ni).and_then(|o| o.units.map.get(name)).is_some_and(|u| u.source == text);
            if same {
                continue;
            }
            let u = unit::parse(&text);
            self.unit_issues(ni, name, &u);
            let Some(os) = self.os_mut(ni) else { return };
            match os.units.map.get_mut(name) {
                Some(rt) => {
                    rt.unit = u;
                    rt.source = text;
                }
                None => {
                    os.units.map.insert(name.clone(), UnitRt::new(u, text));
                }
            }
        }
        if let Some(os) = self.os_mut(ni) {
            os.units.map.retain(|n, u| names.contains(n) || matches!(u.state, UnitState::Active | UnitState::AutoRestart));
        }
    }

    /// reads one unit's file again (a deployment installs it, a rollback may take it away:
    /// then the unit goes, unless it runs)
    pub(crate) fn unit_reread(&mut self, ni: usize, name: &str) {
        let Some(text) = self.unit_text(ni, name) else {
            if let Some(os) = self.os_mut(ni)
                && os.units.map.get(name).is_some_and(|u| !matches!(u.state, UnitState::Active | UnitState::AutoRestart))
            {
                os.units.map.remove(name);
            }
            return;
        };
        let u = unit::parse(&text);
        let Some(os) = self.os_mut(ni) else { return };
        match os.units.map.get_mut(name) {
            Some(rt) => {
                rt.unit = u;
                rt.source = text;
            }
            None => {
                os.units.map.insert(String::from(name), UnitRt::new(u, text));
            }
        }
    }

    /// the unit names enabled by a file in a ".wants" folder
    fn units_wanted(&self, ni: usize) -> Vec<String> {
        let Some(os) = self.os(ni) else { return Vec::new() };
        let mut v: Vec<String> = wants_dirs(os).iter().flat_map(|d| unit_files(os, d)).collect();
        v.sort();
        v.dedup();
        v
    }

    /// the device starts: the units are read, a project service of the same name goes under its
    /// unit, the enabled units start in the order of After=
    pub(crate) fn units_boot(&mut self, ni: usize) {
        let on = matches!(self.nodes.get(ni), Some(Some(n)) if n.power);
        if !on || !self.os(ni).is_some_and(|o| !o.ios && o.files.is_dir(UNIT_DIR)) {
            return;
        }
        self.units_reload(ni);
        let wanted = self.units_wanted(ni);
        // the project's services under a unit of the same name
        let names: Vec<String> = self.os(ni).map(|o| o.units.map.keys().cloned().collect()).unwrap_or_default();
        for name in &names {
            let st = unit::stem(name);
            let id = self.os(ni).and_then(|o| o.svc(st)).filter(|s| !s.console && !s.unit).map(|s| s.id);
            if let (Some(id), Some(os)) = (id, self.os_mut(ni)) {
                if let Some(s) = os.svcs.iter_mut().find(|s| s.id == id) {
                    s.unit = true;
                    s.enabled = false;
                }
                os.inbox.retain(|j| !matches!(j, Job::Start(i) if *i == id));
                if let Some(rt) = os.units.map.get_mut(name) {
                    rt.svc = Some(id);
                }
            }
        }
        // in the order of After= among those that start, names breaking ties and cycles
        let mut left: Vec<String> = wanted.into_iter().filter(|n| names.contains(n)).collect();
        while !left.is_empty() {
            let next = {
                let waits = |n: &String| self.os(ni).and_then(|o| o.units.map.get(n)).is_some_and(|u| u.unit.after.iter().any(|a| left.contains(a) && a != n));
                left.iter().position(|n| !waits(n)).unwrap_or(0)
            };
            let name = left.remove(next);
            let _ = self.unit_start(ni, &name);
        }
    }

    /// the power went off: nothing runs, nothing is due; what systemctl stopped comes back
    pub(crate) fn units_power_off(&mut self, ni: usize) {
        let now = self.now;
        let Some(os) = self.os_mut(ni) else { return };
        for id in core::mem::take(&mut os.units.held) {
            if let Some(s) = os.svcs.iter_mut().find(|s| s.id == id) {
                s.enabled = true;
            }
        }
        for u in os.units.map.values_mut() {
            if u.state != UnitState::Inactive {
                u.state = UnitState::Inactive;
                u.since = now;
            }
            u.due = None;
        }
    }

    // ---------------------------------------------------------------- start and stop

    /// what ExecStart makes (its file is there and compiles); why not
    fn unit_def(&self, ni: usize, name: &str) -> Result<ServiceDef, Msg> {
        let os = self.os(ni).ok_or_else(|| msg!("Dieses Gerät hat keine Dateien."))?;
        let rt = os.units.map.get(name).ok_or_else(|| msg!("Die Einheit {} gibt es nicht.", name))?;
        if let Some(i) = rt.unit.first_error() {
            return Err(i.msg.clone());
        }
        let read = |f: &str| -> Result<String, Msg> {
            let b = os.files.read(f).map_err(|_| msg!("die Datei {} gibt es nicht", f))?;
            core::str::from_utf8(b).map(String::from).map_err(|_| msg!("die Datei {} ist kein Text", f))
        };
        let settings = match &rt.unit.exec {
            Exec::Jfs { file, .. } => {
                let text = read(file)?;
                if let Some(d) = script::check(&text).into_iter().find(|d| d.error) {
                    return Err(msg!("{}, Zeile {}, Spalte {}: {}", file.clone(), d.line, d.col, d.msg));
                }
                None
            }
            Exec::Builtin { file: Some(f), .. } => Some(read(f)?),
            _ => None,
        };
        rt.unit.service_def(name, settings.as_deref())
    }

    /// starts a unit of a file (what it needs first)
    pub(crate) fn unit_start(&mut self, ni: usize, name: &str) -> Result<(), UnitErr> {
        self.unit_start_at(ni, name, 0)
    }

    fn unit_start_at(&mut self, ni: usize, name: &str, depth: u32) -> Result<(), UnitErr> {
        if !self.unit_load(ni, name) {
            return Err(UnitErr::NotFound(String::from(name)));
        }
        let now = self.now;
        let Some(rt) = self.os(ni).and_then(|o| o.units.map.get(name)) else {
            return Err(UnitErr::NotFound(String::from(name)));
        };
        if rt.state == UnitState::Active && rt.svc.is_some_and(|id| self.os(ni).is_some_and(|o| o.svcs.iter().any(|s| s.id == id))) {
            return Ok(());
        }
        // a start by hand while it waits for its restart: that is off
        if let (Some(id), Some(_)) = (rt.svc, rt.due) {
            self.cancel_timer(ni, SLOT_SVC + id * SVC_TIMERS + K_RESTART);
        }
        let Some(rt) = self.os(ni).and_then(|o| o.units.map.get(name)) else {
            return Err(UnitErr::NotFound(String::from(name)));
        };
        // what it needs starts first (a project service counts too)
        let requires = rt.unit.requires.clone();
        if depth < 16 {
            for r in requires {
                if self.unit_is_file(ni, &r) {
                    if let Err(e) = self.unit_start_at(ni, &r, depth + 1) {
                        let m = msg!("Die Einheit {} fehlt oder startet nicht: {}", r.clone(), e.msg());
                        self.unit_failed(ni, name, m.clone());
                        return Err(UnitErr::Failed(m));
                    }
                } else if let Some(id) = self.unit_generated(ni, &r) {
                    if !self.os(ni).is_some_and(|o| o.running(id)) {
                        let dev = self.dev_of(ni);
                        let _ = self.set_service_running(dev, unit::stem(&r), true);
                    }
                } else {
                    let m = msg!("Die Einheit {} fehlt oder startet nicht: {}", r.clone(), UnitErr::NotFound(r.clone()).msg());
                    self.unit_failed(ni, name, m.clone());
                    return Err(UnitErr::Failed(m));
                }
            }
        }
        let def = match self.unit_def(ni, name) {
            Ok(d) => d,
            Err(m) => {
                self.unit_failed(ni, name, m.clone());
                return Err(UnitErr::Failed(m));
            }
        };
        if let Err(e) = self.unit_slot(ni, name, def) {
            let m = e.msg();
            self.unit_failed(ni, name, m.clone());
            return Err(UnitErr::Failed(m));
        }
        let Some(rt) = self.os_mut(ni).and_then(|o| o.units.map.get_mut(name)) else {
            return Ok(());
        };
        rt.state = UnitState::Active;
        rt.since = now;
        rt.exit = None;
        rt.why = None;
        rt.due = None;
        rt.starts.push_back(now);
        while rt.starts.len() > 64 {
            rt.starts.pop_front();
        }
        let text = rt.unit.exec_text.clone();
        self.unit_say(ni, 6, name, msg!("gestartet: {}", text));
        Ok(())
    }

    /// the service of a unit: the one it has, with a new definition if ExecStart changed; a new
    /// one, in place of a service of that name
    fn unit_slot(&mut self, ni: usize, name: &str, def: ServiceDef) -> Result<(), crate::action::ActErr> {
        let dev = self.dev_of(ni);
        let st = unit::stem(name);
        let want = (def.kind.clone(), def.config.clone());
        let had = self.os(ni).and_then(|o| o.units.map.get(name)).and_then(|u| u.def.clone());
        let slot = self.os(ni).and_then(|o| o.svc(st)).filter(|s| !s.console).map(|s| (s.id, s.kind, s.running));
        match slot {
            // as it was
            Some((id, _, running)) if had.as_ref() == Some(&want) => {
                if running {
                    self.svc_stop(ni, id);
                }
                if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)) {
                    s.enabled = true;
                    s.unit = true;
                }
                self.os_push(ni, Job::Start(id));
            }
            // a script keeps its disk and numbers under a new definition
            Some((id, "script", _)) if def.kind == "script" => {
                if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)) {
                    s.enabled = true;
                    s.unit = true;
                }
                self.script_source(dev, st, &def.config)?;
            }
            _ => {
                self.service_def(dev, &def)?;
                if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.name.eq_ignore_ascii_case(st))) {
                    s.unit = true;
                }
            }
        }
        let id = self.os(ni).and_then(|o| o.svc(st)).map(|s| s.id);
        if let Some(rt) = self.os_mut(ni).and_then(|o| o.units.map.get_mut(name)) {
            rt.svc = id;
            rt.def = Some(want);
        }
        Ok(())
    }

    fn unit_failed(&mut self, ni: usize, name: &str, why: Msg) {
        let now = self.now;
        let text = why.text();
        if let Some(rt) = self.os_mut(ni).and_then(|o| o.units.map.get_mut(name)) {
            rt.state = UnitState::Failed;
            rt.since = now;
            rt.why = Some(text);
            rt.due = None;
        }
        self.unit_say(ni, 3, name, msg!("Start gescheitert: {}", why));
    }

    /// stops a unit of a file and what needs it
    pub(crate) fn unit_stop(&mut self, ni: usize, name: &str) -> Result<(), UnitErr> {
        if !self.unit_load(ni, name) {
            return Err(UnitErr::NotFound(String::from(name)));
        }
        let dependents: Vec<String> = self.os(ni).map(|o| o.units.map.iter().filter(|(n, u)| n.as_str() != name && u.state == UnitState::Active && u.unit.requires.iter().any(|r| r == name)).map(|(n, _)| n.clone()).collect()).unwrap_or_default();
        for d in dependents {
            let _ = self.unit_stop(ni, &d);
        }
        let now = self.now;
        let Some(rt) = self.os(ni).and_then(|o| o.units.map.get(name)) else { return Ok(()) };
        let was = rt.state;
        let id = rt.svc;
        if let Some(id) = id {
            self.svc_stop(ni, id);
            self.cancel_timer(ni, SLOT_SVC + id * SVC_TIMERS + K_RESTART);
            if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)) {
                s.enabled = false;
            }
        }
        if let Some(rt) = self.os_mut(ni).and_then(|o| o.units.map.get_mut(name)) {
            rt.state = UnitState::Inactive;
            rt.since = now;
            rt.due = None;
        }
        if matches!(was, UnitState::Active | UnitState::AutoRestart) {
            self.unit_say(ni, 6, name, msg!("gestoppt"));
        }
        Ok(())
    }

    // ---------------------------------------------------------------- what os.rs tells

    /// the service of a unit ended by itself with a status (OsApi::halt, halt_code)
    pub(crate) fn units_halted(&mut self, ni: usize, id: u32, code: i32) {
        let Some(os) = self.os(ni) else { return };
        let Some(name) = os.units.map.iter().find(|(_, u)| u.svc == Some(id)).map(|(n, _)| n.clone()) else {
            return;
        };
        self.unit_ended(ni, &name, code, None);
    }

    /// a unit's program ended: its state, then its restart policy
    fn unit_ended(&mut self, ni: usize, name: &str, code: i32, why: Option<String>) {
        let now = self.now;
        let Some(rt) = self.os_mut(ni).and_then(|o| o.units.map.get_mut(name)) else { return };
        rt.exit = Some(code);
        rt.since = now;
        rt.why = why;
        rt.state = if code == 0 { UnitState::Inactive } else { UnitState::Failed };
        let again = match rt.unit.restart {
            Restart::No => false,
            Restart::OnFailure => code != 0,
            Restart::Always => true,
        };
        let (due, id) = (now.saturating_add(rt.unit.restart_ns), rt.svc);
        if again {
            rt.state = UnitState::AutoRestart;
            rt.due = Some(due);
        }
        let ns = rt.unit.restart_ns;
        if code == 0 {
            self.unit_say(ni, 6, name, msg!("beendet, Status 0"));
        } else {
            self.unit_say(ni, 4, name, msg!("beendet mit Status {}", code));
        }
        if again {
            self.unit_say(ni, 6, name, msg!("Neustart in {}", crate::trace::fmt_ns(ns as i64)));
            if let Some(id) = id {
                self.set_timer(ni, SLOT_SVC + id * SVC_TIMERS + K_RESTART, due);
            }
        }
    }

    /// a timer of a service that does not run: its unit's RestartSec is over (os.rs)
    pub(crate) fn unit_timer(&mut self, ni: usize, id: u32) {
        let now = self.now;
        let Some(os) = self.os_mut(ni) else { return };
        let Some((name, rt)) = os.units.map.iter_mut().find(|(_, u)| u.svc == Some(id) && u.due.is_some_and(|d| d <= now)) else {
            return;
        };
        let name = name.clone();
        rt.due = None;
        // the start limit
        let (burst, span) = (rt.unit.limit_burst as usize, rt.unit.limit_ns);
        while rt.starts.front().is_some_and(|t| now.saturating_sub(*t) >= span) {
            rt.starts.pop_front();
        }
        if burst > 0 && rt.starts.len() >= burst {
            rt.state = UnitState::Failed;
            rt.since = now;
            rt.why = Some(msg!("zu viele Starts in kurzer Zeit").text());
            self.unit_say(ni, 3, &name, msg!("zu viele Starts in kurzer Zeit: er bleibt gestoppt"));
            return;
        }
        rt.restarts += 1;
        if let Err(e) = self.unit_start(ni, &name) {
            // its file went missing or does not compile: that ends it again
            self.unit_ended(ni, &name, 1, Some(e.msg().text()));
        }
    }

    /// a unit's service was switched from outside (script start|stop, a fault, ServiceRun)
    pub(crate) fn units_ctl(&mut self, ni: usize, id: u32, on: bool) {
        let now = self.now;
        let Some(os) = self.os_mut(ni) else { return };
        let Some(rt) = os.units.map.values_mut().find(|u| u.svc == Some(id)) else { return };
        rt.state = if on { UnitState::Active } else { UnitState::Inactive };
        rt.since = now;
        rt.due = None;
    }

    // ---------------------------------------------------------------- generated units

    /// a unit file or a unit that was read
    pub(crate) fn unit_is_file(&self, ni: usize, name: &str) -> bool {
        self.os(ni).is_some_and(|o| o.units.map.contains_key(name) || o.files.exists(&unit::path_of(name)))
    }

    /// the service of the project that makes a generated unit of this name
    pub(crate) fn unit_generated(&self, ni: usize, name: &str) -> Option<u32> {
        if !name.ends_with(".service") {
            return None;
        }
        self.os(ni)?.svc(unit::stem(name)).filter(|s| !s.console && !s.unit).map(|s| s.id)
    }

    // ---------------------------------------------------------------- the operations

    /// Action::Unit, systemctl
    pub fn unit_op(&mut self, dev: u32, name: &str, op: UnitOp) -> Result<(), UnitErr> {
        let ni = self.node_index(dev).ok_or_else(|| UnitErr::NotFound(String::from(name)))?;
        let name = unit::unit_name(name);
        match op {
            UnitOp::Enable => self.unit_enable(ni, &name).map(|_| ()),
            UnitOp::Disable => self.unit_disable(ni, &name).map(|_| ()),
            UnitOp::Start | UnitOp::Stop | UnitOp::Restart => {
                if self.unit_is_file(ni, &name) {
                    return match op {
                        UnitOp::Start => self.unit_start(ni, &name),
                        UnitOp::Stop => self.unit_stop(ni, &name),
                        _ => {
                            let running = self.os(ni).and_then(|o| o.units.map.get(&name)).is_some_and(|u| u.state == UnitState::Active);
                            if running {
                                self.unit_stop_one(ni, &name);
                            }
                            self.unit_start(ni, &name)
                        }
                    };
                }
                let Some(id) = self.unit_generated(ni, &name) else {
                    return Err(UnitErr::NotFound(name));
                };
                let st = String::from(unit::stem(&name));
                let enabled = self.os(ni).and_then(|o| o.svcs.iter().find(|s| s.id == id)).is_some_and(|s| s.enabled);
                if let Some(os) = self.os_mut(ni) {
                    os.units.held.retain(|h| *h != id);
                    if op == UnitOp::Stop && enabled {
                        os.units.held.push(id);
                    }
                }
                let r = match op {
                    UnitOp::Start => self.set_service_running(dev, &st, true),
                    UnitOp::Stop => self.set_service_running(dev, &st, false),
                    _ => self.set_service_running(dev, &st, false).and_then(|_| self.set_service_running(dev, &st, true)),
                };
                r.map_err(|e| UnitErr::Failed(e.msg()))
            }
        }
    }

    /// stops a unit alone (a restart: what needs it stays)
    fn unit_stop_one(&mut self, ni: usize, name: &str) {
        let id = self.os(ni).and_then(|o| o.units.map.get(name)).and_then(|u| u.svc);
        if let Some(id) = id {
            self.svc_stop(ni, id);
        }
        if let Some(rt) = self.os_mut(ni).and_then(|o| o.units.map.get_mut(name)) {
            rt.state = UnitState::Inactive;
        }
    }

    /// enable: a file per target of [Install] -> the files it made (none: nothing to enable)
    pub(crate) fn unit_enable(&mut self, ni: usize, name: &str) -> Result<Vec<String>, UnitErr> {
        let Some(text) = self.unit_text(ni, name) else {
            return Err(if self.unit_generated(ni, name).is_some() { UnitErr::Generated(String::from(name)) } else { UnitErr::Bad(msg!("Die Datei von {} gibt es nicht.", name)) });
        };
        let u = unit::parse(&text);
        let mut made = Vec::new();
        for target in &u.wanted_by {
            let p = unit::link_path(target, name);
            if self.os(ni).is_some_and(|o| o.files.exists(&p)) {
                continue;
            }
            let dir = self.os(ni).map(|o| o.files.parent(&p)).unwrap_or_default();
            if self.file_mkdir(ni, &dir, true).is_err() {
                return Err(UnitErr::Bad(msg!("{} lässt sich nicht anlegen.", p.clone())));
            }
            if self.file_put(ni, &p, format!("{}\n", unit::path_of(name)).as_bytes(), false).is_err() {
                return Err(UnitErr::Bad(msg!("{} lässt sich nicht anlegen.", p.clone())));
            }
            made.push(p);
        }
        Ok(made)
    }

    /// disable: the files of enable go -> those that went
    pub(crate) fn unit_disable(&mut self, ni: usize, name: &str) -> Result<Vec<String>, UnitErr> {
        if self.unit_text(ni, name).is_none() && self.unit_generated(ni, name).is_some() {
            return Err(UnitErr::Generated(String::from(name)));
        }
        let links = self.unit_links(ni, name);
        if self.unit_text(ni, name).is_none() && links.is_empty() {
            return Err(UnitErr::Bad(msg!("Die Datei von {} gibt es nicht.", name)));
        }
        for p in &links {
            let _ = self.file_del(ni, p, false);
        }
        Ok(links)
    }

    /// the files that enable a unit
    fn unit_links(&self, ni: usize, name: &str) -> Vec<String> {
        let Some(os) = self.os(ni) else { return Vec::new() };
        wants_dirs(os).iter().map(|d| format!("{}/{}", d, name)).filter(|p| os.files.exists(p)).collect()
    }

    /// whether a unit starts with the device (None: there is no such unit)
    pub(crate) fn unit_enabled(&self, ni: usize, name: &str) -> Option<UnitEnabled> {
        let os = self.os(ni)?;
        if self.unit_is_file(ni, name) {
            if !self.unit_links(ni, name).is_empty() {
                return Some(UnitEnabled::Enabled);
            }
            let install = match self.unit_text(ni, name) {
                Some(t) => !unit::parse(&t).wanted_by.is_empty(),
                None => os.units.map.get(name).is_some_and(|u| !u.unit.wanted_by.is_empty()),
            };
            return Some(if install { UnitEnabled::Disabled } else { UnitEnabled::Static });
        }
        let id = self.unit_generated(ni, name)?;
        let on = os.svcs.iter().find(|s| s.id == id).is_some_and(|s| s.enabled) || os.units.held.contains(&id);
        Some(if on { UnitEnabled::Generated } else { UnitEnabled::GeneratedOff })
    }

    // ---------------------------------------------------------------- views

    /// the state of a service of the project as a unit's
    fn generated_state(s: &crate::os::SvcSlot) -> UnitState {
        if s.running {
            return UnitState::Active;
        }
        let failed = s.svc.as_ref().and_then(|x| x.as_any().downcast_ref::<ScriptService>()).is_some_and(|x| x.state() == ScriptState::Failed);
        if failed { UnitState::Failed } else { UnitState::Inactive }
    }

    fn unit_view_of(&self, ni: usize, name: &str) -> Option<UnitView> {
        let os = self.os(ni)?;
        let enabled = self.unit_enabled(ni, name)?;
        if self.unit_is_file(ni, name) {
            let text = self.unit_text(ni, name);
            let (u, rt) = match os.units.map.get(name) {
                Some(rt) => (rt.unit.clone(), Some(rt)),
                None => (unit::parse(text.as_deref().unwrap_or("")), None),
            };
            let slot = rt.and_then(|r| r.svc).and_then(|id| os.svcs.iter().find(|s| s.id == id));
            let mut state = rt.map_or(UnitState::Inactive, |r| r.state);
            // its service was removed behind it
            if state == UnitState::Active && slot.is_none() {
                state = UnitState::Inactive;
            }
            return Some(UnitView {
                name: String::from(name),
                description: u.description.clone(),
                file: text.as_ref().map(|_| unit::path_of(name)),
                bad: u.bad(),
                state,
                enabled,
                exec: u.exec_text.clone(),
                restart: u.restart,
                restart_ns: u.restart_ns,
                restarts: rt.map_or(0, |r| r.restarts),
                since: rt.map_or(0, |r| r.since),
                exit: rt.and_then(|r| r.exit),
                why: rt.and_then(|r| r.why.clone()),
                pid: slot.filter(|s| s.running).map(|s| 100 + s.id),
                changed: rt.is_some_and(|r| text.as_ref().is_some_and(|t| *t != r.source)),
                issues: u.issues.clone(),
                after: u.after.clone(),
                requires: u.requires.clone(),
            });
        }
        let id = self.unit_generated(ni, name)?;
        let s = os.svcs.iter().find(|s| s.id == id)?;
        Some(UnitView {
            name: String::from(name),
            description: String::from(s.kind),
            file: None,
            bad: false,
            state: Self::generated_state(s),
            enabled,
            exec: String::from(s.kind),
            restart: Restart::No,
            restart_ns: 0,
            restarts: 0,
            since: s.since,
            exit: None,
            why: None,
            pid: s.running.then_some(100 + s.id),
            changed: false,
            issues: Vec::new(),
            after: Vec::new(),
            requires: Vec::new(),
        })
    }

    /// the units of a device, by name: the unit files and the services of the project
    pub fn units(&self, dev: u32) -> Vec<UnitView> {
        let Some(ni) = self.node_index(dev) else { return Vec::new() };
        let Some(os) = self.os(ni) else { return Vec::new() };
        let mut names: Vec<String> = os.units.map.keys().cloned().collect();
        names.extend(unit_files(os, UNIT_DIR));
        names.extend(os.svcs.iter().filter(|s| !s.console && !s.unit).map(|s| unit::unit_name(&s.name)));
        names.sort();
        names.dedup();
        names.iter().filter_map(|n| self.unit_view_of(ni, n)).collect()
    }

    /// one unit of a device ("web" or "web.service")
    pub fn unit(&self, dev: u32, name: &str) -> Option<UnitView> {
        let ni = self.node_index(dev)?;
        self.unit_view_of(ni, &unit::unit_name(name))
    }

    /// the units that were read and run script `path` (canonical): the names of their services,
    /// their states
    pub fn units_of_file(&self, dev: u32, path: &str) -> Vec<(String, UnitState)> {
        let Some(ni) = self.node_index(dev) else { return Vec::new() };
        let Some(os) = self.os(ni) else { return Vec::new() };
        let runs = |u: &Unit| matches!(&u.exec, Exec::Jfs { file, .. } if os.files.norm(file).ok().as_deref() == Some(path));
        os.units.map.iter().filter(|(_, rt)| runs(&rt.unit)).filter_map(|(n, _)| self.unit_view_of(ni, n)).map(|v| (String::from(unit::stem(&v.name)), v.state)).collect()
    }

    /// the service id of a unit (a unit file's service, or the project's)
    pub(crate) fn unit_svc(&self, ni: usize, name: &str) -> Option<u32> {
        match self.os(ni)?.units.map.get(name) {
            Some(rt) => rt.svc,
            None => self.unit_generated(ni, name),
        }
    }
}

/// a time for messages: "2 s", "500 ms"
pub fn span_text(ns: Ns) -> String {
    if ns >= SEC && ns % SEC == 0 { format!("{} s", ns / SEC) } else { format!("{} ms", ns / 1_000_000) }
}
