// deploy -- rolling deployments (A2): files and a unit go from one host to the hosts of a list,
// one after the other, each with a health check before the next one
//
//   Action::Deploy { spec } / Engine::deploy_start(spec) -> id    starts a run on spec.src (a
//                                  refusal says why: jfn_model::unit::DeploySpec names what)
//   Action::DeployAbort { dev, id }    ends a run: the host it is at stays as it is
//   Engine::deployments(dev)       the runs a device started, oldest first (DeployRun: per host
//                                  its state and what it is doing, the log of the run)
//
// A run is a console-flagged service of the source device, so it runs on virtual time through
// the engine's own events, and the K1 recording replays it from its Action: it uses the service
// timers of its id, is no entry of Dienste or Metriken, and shows in ps as deploy.  For each host,
// in order: wait for the copy, write the files and the unit file (the same paths as on the source;
// missing folders are made; "kopiert" in the log), read the unit file, enable it, restart it, then
// the health check -- the unit is active (asked directly), a TCP connection to a port, or an HTTP
// GET with an expected status, both from the source host over the simulated network to the
// target's address -- repeated every 500 ms until it holds or the time for the check is over, a
// wait, and the next host.
//
// How files travel: the copy is at engine level, an event per host (the time: 50 ms + the bytes at
// 100 Mbit/s); it needs the target switched on, not a working network.  X1f's scp can replace
// it later.  What is copied is read from the source when the run starts, so editing meanwhile
// does not change it.
//
// The first host whose check fails stops the run, and the hosts after it are not touched.  With
// OnFail::Stop the hosts done so far keep the new files and the failed host stays as it is; with
// OnFail::Rollback the failed host and then the hosts done (last one first) get back the files
// they had (a file that is new goes), the unit as it was enabled, and it runs again if it ran.
//
// Rollback restores only what the run wrote.  The target's project keeps every file through
// Report::Files (doc/console.rs: an undo step of the project per host and write).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use jfn_model::catalog::CliStyle;
use jfn_model::msg;
use jfn_model::msg::Msg;
use jfn_model::unit::{self, DeploySpec, Health, OnFail};
use netpkt::IpAddr;

use crate::action::ActErr;
use crate::kernel::{Engine, Ns, MS};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::tcp::{SockOpts, TcpEnd};
use crate::trace::Note;
use crate::units::UnitState;

const K_STEP: u32 = 0;
/// the first look after a restart, the next tries, between two hosts of a rollback
const FIRST_CHECK: Ns = 200 * MS;
const RETRY: Ns = 500 * MS;
const ROLL_GAP: Ns = 100 * MS;
/// a host's copy: this much and the bytes at COPY_BPS
const COPY_BASE: Ns = 50 * MS;
const COPY_BPS: u64 = 100_000_000;
/// the lines a run keeps in its log
const LOG_KEEP: usize = 200;

/// how a host of a run stands
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostState {
    /// its turn did not come yet
    Pending,
    Copying,
    Checking,
    /// the check held: the wait before the next host
    Waiting,
    Done,
    Failed,
    /// the run stopped before it
    Skipped,
    /// back to its files before the run
    RolledBack,
}

impl HostState {
    /// the state in words, for the console and reports
    pub fn word(self) -> &'static str {
        match self {
            HostState::Pending => "wartet",
            HostState::Copying => "kopiert",
            HostState::Checking => "prüft",
            HostState::Waiting => "wartet nach der Prüfung",
            HostState::Done => "fertig",
            HostState::Failed => "gescheitert",
            HostState::Skipped => "nicht begonnen",
            HostState::RolledBack => "zurückgerollt",
        }
    }
}

/// a host in a run
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRun {
    pub dev: u32,
    pub name: String,
    pub state: HostState,
    pub since: Ns,
    /// what it did last or why it failed
    pub note: Msg,
}

/// how a run stands
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Running,
    Done,
    /// a host failed; the hosts before it keep the new version
    Failed,
    /// a host failed; every host has its old files again
    RolledBack,
    Aborted,
}

impl RunState {
    /// the word of "expect DEV deploy WORD" and of a report
    pub fn word(self) -> &'static str {
        match self {
            RunState::Running => "running",
            RunState::Done => "done",
            RunState::Failed => "failed",
            RunState::RolledBack => "rolledback",
            RunState::Aborted => "aborted",
        }
    }
}

/// a line of a run's log
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeployLine {
    pub at: Ns,
    /// the host it is about (0: the run)
    pub dev: u32,
    pub msg: Msg,
}

/// a deployment as the window and the reports see it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeployRun {
    pub id: u32,
    pub src: u32,
    pub unit: String,
    pub state: RunState,
    pub start: Ns,
    pub end: Option<Ns>,
    pub hosts: Vec<HostRun>,
    pub log: Vec<DeployLine>,
    /// the lines before the first one kept
    pub dropped: usize,
}

impl DeployRun {
    pub fn footprint(&self) -> usize {
        256 + self.hosts.len() * 128 + self.log.len() * 96
    }
    /// hosts that are through
    pub fn done(&self) -> usize {
        self.hosts.iter().filter(|h| h.state == HostState::Done).count()
    }
}

/// what a host had before the run
#[derive(Clone)]
struct Backup {
    host: usize,
    files: Vec<(String, Option<Vec<u8>>)>,
    enabled: bool,
    active: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Copy,
    Check,
    Wait,
    Roll,
    End,
}

/// a deployment under way (the service on the source device)
#[derive(Clone)]
pub struct Deployer {
    spec: DeploySpec,
    id: u32,
    /// the files as the run began: path, bytes (folders: None)
    payload: Vec<(String, Option<Vec<u8>>)>,
    bytes: u64,
    cur: usize,
    phase: Phase,
    deadline: Ns,
    /// the check's connection and what came on it
    sock: Option<u32>,
    got: Vec<u8>,
    last_why: Msg,
    backups: Vec<Backup>,
}

impl Deployer {
    fn with_run(&self, os: &mut OsApi, f: impl FnOnce(&mut DeployRun)) {
        let (ni, id) = (os.ni, self.id);
        if let Some(r) = os.eng.os_mut(ni).and_then(|o| o.units.deploys.iter_mut().find(|d| d.id == id)) {
            f(r);
        }
    }

    fn say(&self, os: &mut OsApi, dev: u32, m: Msg) {
        let at = os.now();
        let id = self.id;
        let n = os.dev();
        os.eng.note(n, None, Note::Deploy { id, what: m.clone() });
        self.with_run(os, |r| {
            if r.log.len() >= LOG_KEEP {
                r.log.remove(0);
                r.dropped += 1;
            }
            r.log.push(DeployLine { at, dev, msg: m });
        });
    }

    fn host_state(&self, os: &mut OsApi, h: usize, st: HostState, note: Msg) {
        let at = os.now();
        self.with_run(os, |r| {
            if let Some(x) = r.hosts.get_mut(h) {
                x.state = st;
                x.since = at;
                x.note = note;
            }
        });
    }

    /// the next host, or the end
    fn next_host(&mut self, os: &mut OsApi) {
        let Some(&dev) = self.spec.hosts.get(self.cur) else {
            self.finish(os, RunState::Done);
            return;
        };
        let name = os.eng.node(dev).map(|n| n.name.clone()).unwrap_or_default();
        self.host_state(os, self.cur, HostState::Copying, msg!("kopiert"));
        self.say(os, dev, msg!("{}: kopiert {} Dateien", name, self.payload.iter().filter(|p| p.1.is_some()).count()));
        self.phase = Phase::Copy;
        let t = os.now() + COPY_BASE + self.bytes.saturating_mul(8).saturating_mul(1_000_000_000) / COPY_BPS;
        os.timer(K_STEP, t);
    }

    /// the copy is over: the files, the unit, the restart
    fn after_copy(&mut self, os: &mut OsApi) {
        let dev = self.spec.hosts[self.cur];
        let name = os.eng.node(dev).map(|n| n.name.clone()).unwrap_or_default();
        let Some(tni) = os.eng.node_index(dev) else {
            self.fail_host(os, msg!("das Gerät gibt es nicht mehr"));
            return;
        };
        if !os.eng.nodes.get(tni).is_some_and(|n| n.as_ref().is_some_and(|n| n.power)) {
            self.fail_host(os, msg!("das Gerät ist ausgeschaltet"));
            return;
        }
        let name_u = unit::unit_name(&self.spec.unit);
        let active = os.eng.unit(dev, &name_u).is_some_and(|u| u.state == UnitState::Active);
        let enabled = os.eng.unit_enabled(tni, &name_u).is_some_and(|e| e == crate::units::UnitEnabled::Enabled);
        let files = match os.eng.deploy_write(tni, &self.payload) {
            Ok(b) => b,
            Err(m) => {
                self.fail_host(os, msg!("die Dateien kamen nicht an: {}", m));
                return;
            }
        };
        self.backups.push(Backup { host: self.cur, files, enabled, active });
        // the unit file is new: read, enabled, restarted
        os.eng.unit_reread(tni, &name_u);
        let bad = os.eng.os(tni).and_then(|o| o.units.map.get(&name_u)).and_then(|u| u.unit.first_error().map(|i| i.msg.clone()));
        if let Some(m) = bad {
            self.fail_host(os, msg!("die Einheit ist falsch: {}", m));
            return;
        }
        if let Err(e) = os.eng.unit_enable(tni, &name_u) {
            self.fail_host(os, e.msg());
            return;
        }
        if let Err(e) = os.eng.unit_op(dev, &name_u, crate::units::UnitOp::Restart) {
            self.fail_host(os, e.msg());
            return;
        }
        self.say(os, dev, msg!("{}: {} neu gestartet", name, name_u.clone()));
        self.host_state(os, self.cur, HostState::Checking, msg!("prüft"));
        self.phase = Phase::Check;
        self.deadline = os.now() + self.spec.timeout;
        self.last_why = msg!("keine Antwort");
        os.timer(K_STEP, os.now() + FIRST_CHECK);
    }

    /// a try of the health check
    fn attempt(&mut self, os: &mut OsApi) {
        let dev = self.spec.hosts[self.cur];
        match self.spec.health.clone() {
            Health::Active => {
                let u = os.eng.unit(dev, &self.spec.unit);
                match u.as_ref().map(|u| u.state) {
                    Some(UnitState::Active) => self.check_ok(os),
                    s => {
                        self.last_why = msg!("die Einheit ist {}", s.map_or("?", |x| x.active_word()));
                        self.retry(os);
                    }
                }
            }
            Health::Tcp { port } | Health::Http { port, .. } => {
                if let Some(s) = self.sock.take() {
                    os.tcp_abort(s);
                }
                self.got.clear();
                let Some(addr) = os.eng.deploy_addr(dev) else {
                    self.last_why = msg!("das Gerät hat keine Adresse");
                    self.retry(os);
                    return;
                };
                match os.tcp_connect_msg(addr, port, SockOpts::default()) {
                    Ok(s) => {
                        self.sock = Some(s);
                        // the try ends at the next timer
                        os.timer(K_STEP, os.now() + RETRY);
                    }
                    Err(m) => {
                        self.last_why = m;
                        self.retry(os);
                    }
                }
            }
        }
    }

    /// after a failed try: the next one, or the host failed
    fn retry(&mut self, os: &mut OsApi) {
        if let Some(s) = self.sock.take() {
            os.tcp_abort(s);
        }
        if os.now() >= self.deadline {
            let why = self.last_why.clone();
            self.fail_host(os, msg!("die Prüfung hielt nicht: {}", why));
            return;
        }
        os.timer(K_STEP, os.now() + RETRY);
    }

    /// the check holds: the wait, then the next host
    fn check_ok(&mut self, os: &mut OsApi) {
        if let Some(s) = self.sock.take() {
            os.tcp_close(s);
        }
        let dev = self.spec.hosts[self.cur];
        let name = os.eng.node(dev).map(|n| n.name.clone()).unwrap_or_default();
        self.say(os, dev, msg!("{}: Prüfung hielt", name));
        self.host_state(os, self.cur, HostState::Waiting, msg!("wartet"));
        self.phase = Phase::Wait;
        os.timer(K_STEP, os.now() + self.spec.wait);
    }

    /// the host failed: the rest stays untouched; a rollback goes back
    fn fail_host(&mut self, os: &mut OsApi, why: Msg) {
        if let Some(s) = self.sock.take() {
            os.tcp_abort(s);
        }
        let dev = self.spec.hosts[self.cur];
        let name = os.eng.node(dev).map(|n| n.name.clone()).unwrap_or_default();
        self.say(os, dev, msg!("{}: gescheitert: {}", name, why));
        self.host_state(os, self.cur, HostState::Failed, why);
        for h in self.cur + 1..self.spec.hosts.len() {
            self.host_state(os, h, HostState::Skipped, msg!("nicht begonnen"));
        }
        if self.spec.on_fail == OnFail::Rollback && !self.backups.is_empty() {
            self.phase = Phase::Roll;
            os.timer(K_STEP, os.now() + ROLL_GAP);
        } else {
            self.finish(os, RunState::Failed);
        }
    }

    /// the next host of a rollback, last one first
    fn roll_next(&mut self, os: &mut OsApi) {
        let Some(b) = self.backups.pop() else {
            self.finish(os, RunState::RolledBack);
            return;
        };
        let dev = self.spec.hosts[b.host];
        let name = os.eng.node(dev).map(|n| n.name.clone()).unwrap_or_default();
        if let Some(tni) = os.eng.node_index(dev) {
            let name_u = unit::unit_name(&self.spec.unit);
            os.eng.deploy_restore(tni, &b.files);
            let _ = os.eng.unit_op(dev, &name_u, crate::units::UnitOp::Stop);
            if !b.enabled {
                let _ = os.eng.unit_disable(tni, &name_u);
            }
            os.eng.unit_reread(tni, &name_u);
            // a unit that came with this run goes with its service
            if !os.eng.unit_is_file(tni, &name_u) {
                let _ = os.eng.remove_service(dev, unit::stem(&name_u));
            }
            if b.active {
                let _ = os.eng.unit_op(dev, &name_u, crate::units::UnitOp::Start);
            }
        }
        self.say(os, dev, msg!("{}: zurückgerollt", name));
        self.host_state(os, b.host, HostState::RolledBack, msg!("zurückgerollt"));
        os.timer(K_STEP, os.now() + ROLL_GAP);
    }

    /// the run is over
    fn finish(&mut self, os: &mut OsApi, st: RunState) {
        let end = os.now();
        self.phase = Phase::End;
        self.with_run(os, |r| {
            r.state = st;
            r.end = Some(end);
        });
        let unit = unit::unit_name(&self.spec.unit);
        let m = match st {
            RunState::Done => msg!("{} ist auf allen Hosts bereitgestellt", unit),
            RunState::Failed => msg!("Bereitstellung von {} gescheitert", unit),
            RunState::RolledBack => msg!("Bereitstellung von {} gescheitert, alle Hosts zurückgerollt", unit),
            _ => msg!("Bereitstellung von {} abgebrochen", unit),
        };
        self.say(os, 0, m);
        os.halt_code(0);
    }

    /// Action::DeployAbort: the run ends where it is
    pub(crate) fn abort(&mut self, os: &mut OsApi) {
        if self.phase == Phase::End {
            return;
        }
        if let Some(s) = self.sock.take() {
            os.tcp_abort(s);
        }
        let n = self.spec.hosts.len();
        for h in self.cur..n {
            let what = if h == self.cur && matches!(self.phase, Phase::Copy | Phase::Check | Phase::Wait) { msg!("abgebrochen") } else { msg!("nicht begonnen") };
            let st = if h == self.cur && matches!(self.phase, Phase::Copy | Phase::Check | Phase::Wait) { HostState::Failed } else { HostState::Skipped };
            self.host_state(os, h, st, what);
        }
        self.finish(os, RunState::Aborted);
    }
}

impl Service for Deployer {
    fn kind(&self) -> &'static str {
        "deploy"
    }
    fn start(&mut self, os: &mut OsApi) {
        let hosts: Vec<String> = self.spec.hosts.iter().map(|d| os.eng.node(*d).map(|n| n.name.clone()).unwrap_or_default()).collect();
        self.say(os, 0, msg!("Bereitstellung von {} auf {}", unit::unit_name(&self.spec.unit), hosts.join(", ")));
        self.next_host(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Timer(K_STEP) => match self.phase {
                Phase::Copy => self.after_copy(os),
                Phase::Check => {
                    // a try that is still open got no answer in time
                    if self.sock.is_some() {
                        self.last_why = msg!("keine Antwort");
                    }
                    if os.now() >= self.deadline {
                        self.retry(os);
                    } else {
                        self.attempt(os);
                    }
                }
                Phase::Wait => {
                    let dev = self.spec.hosts[self.cur];
                    let name = os.eng.node(dev).map(|n| n.name.clone()).unwrap_or_default();
                    self.say(os, dev, msg!("{}: fertig", name));
                    self.host_state(os, self.cur, HostState::Done, msg!("fertig"));
                    self.cur += 1;
                    self.next_host(os);
                }
                Phase::Roll => self.roll_next(os),
                Phase::End => {}
            },
            SvcEvent::TcpConnected { sock, .. } if self.sock == Some(sock) => match self.spec.health.clone() {
                Health::Http { path, .. } => {
                    let host = os.eng.deploy_addr(self.spec.hosts[self.cur]).map(|a| format!("{}", a)).unwrap_or_default();
                    let req = format!("GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: jfn-deploy\r\nConnection: close\r\n\r\n", path, host);
                    os.tcp_send(sock, req.as_bytes());
                }
                _ => self.check_ok(os),
            },
            SvcEvent::TcpData { sock, .. } if self.sock == Some(sock) => {
                let b = os.tcp_recv(sock, 4096);
                self.got.extend_from_slice(&b);
                if let Health::Http { status, .. } = self.spec.health.clone()
                    && let Some(i) = self.got.iter().position(|x| *x == b'\n')
                {
                    let line = String::from_utf8_lossy(&self.got[..i]).into_owned();
                    let code = line.split_whitespace().nth(1).and_then(|c| c.parse::<u16>().ok()).unwrap_or(0);
                    if code == status {
                        self.check_ok(os);
                    } else {
                        self.last_why = msg!("Status {} statt {}", code, status);
                        self.retry(os);
                    }
                }
            }
            SvcEvent::TcpClosed { sock, end } if self.sock == Some(sock) => {
                self.sock = None;
                os.tcp_close(sock);
                self.last_why = match end {
                    TcpEnd::Refused => msg!("niemand lauscht auf dem Port"),
                    TcpEnd::Timeout => msg!("keine Antwort"),
                    TcpEnd::Unreachable => msg!("das Ziel ist nicht erreichbar"),
                    _ => msg!("die Verbindung endete ohne Antwort"),
                };
                self.retry(os);
            }
            SvcEvent::TcpEof { sock, .. } if self.sock == Some(sock) => {
                self.last_why = msg!("die Verbindung endete ohne Antwort");
                self.retry(os);
            }
            _ => {}
        }
    }
    fn stop(&mut self, os: &mut OsApi) {
        // the source device went off: the run is over where it is
        if self.phase == Phase::End {
            return;
        }
        self.phase = Phase::End;
        let end = os.now();
        self.with_run(os, |r| {
            r.state = RunState::Aborted;
            r.end = Some(end);
            for h in r.hosts.iter_mut().filter(|h| !matches!(h.state, HostState::Done | HostState::Failed | HostState::RolledBack)) {
                h.state = HostState::Skipped;
                h.since = end;
                h.note = msg!("nicht begonnen");
            }
        });
    }
    fn metrics(&self) -> Vec<Metric> {
        Vec::new()
    }
    fn status(&self) -> String {
        String::from(unit::stem(&self.spec.unit))
    }
    fn footprint(&self) -> usize {
        1024 + self.bytes as usize * (1 + self.backups.len())
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Engine {
    /// the address a host is reached at: its first interface with an address that is up
    pub fn deploy_addr(&self, dev: u32) -> Option<IpAddr> {
        let ni = self.node_index(dev)?;
        let st = self.stack(ni)?;
        (0..st.ifaces.len() as u16).find_map(|i| {
            let f = &st.ifaces[i as usize];
            if f.hidden || f.is_loopback() || !self.iface_up(ni, i) {
                return None;
            }
            f.primary().map(IpAddr::V4)
        })
    }

    /// writes files on a device -> what those paths held before; the refusal in words
    pub(crate) fn deploy_write(&mut self, ni: usize, payload: &[(String, Option<Vec<u8>>)]) -> Result<Vec<(String, Option<Vec<u8>>)>, Msg> {
        let t = self.file_time(ni);
        let Some(os) = self.os_mut(ni) else { return Err(msg!("das Gerät hat keine Dateien")) };
        let mut old = Vec::new();
        let mut first = String::new();
        for (path, data) in payload {
            match data {
                None => {
                    if !os.files.exists(path) {
                        os.files.mkdir(path, t, true).map_err(|e| msg!("{}: {}", path.clone(), e.errno_text()))?;
                    }
                }
                Some(b) => {
                    let parent = os.files.parent(path);
                    if !os.files.exists(&parent) {
                        os.files.mkdir(&parent, t, true).map_err(|e| msg!("{}: {}", parent.clone(), e.errno_text()))?;
                    }
                    let before = os.files.read(path).ok().map(Vec::from);
                    os.files.write(path, b, t).map_err(|e| msg!("{}: {}", path.clone(), e.errno_text()))?;
                    old.push((path.clone(), before));
                    first = path.clone();
                }
            }
        }
        self.files_changed(ni, &first);
        Ok(old)
    }

    /// puts back what deploy_write replaced (a file that was new goes)
    pub(crate) fn deploy_restore(&mut self, ni: usize, old: &[(String, Option<Vec<u8>>)]) {
        let t = self.file_time(ni);
        let Some(os) = self.os_mut(ni) else { return };
        let mut first = String::new();
        for (path, before) in old {
            let _ = match before {
                Some(b) => os.files.write(path, b, t),
                None => os.files.remove(path),
            };
            if first.is_empty() {
                first = path.clone();
            }
        }
        self.files_changed(ni, &first);
    }

    /// starts a deployment -> its number
    pub fn deploy_start(&mut self, spec: DeploySpec) -> Result<u32, ActErr> {
        let ni = self.node_index(spec.src).ok_or(ActErr::UnknownDevice(spec.src))?;
        let refuse = |m: Msg| ActErr::Refused(m);
        let src_name = self.node(spec.src).map(|n| n.name.clone()).unwrap_or_default();
        let linux = |e: &Engine, n: usize| e.cdev(n).is_some_and(|c| c.style == CliStyle::Linux) && e.os(n).is_some();
        if !linux(self, ni) {
            return Err(refuse(msg!("{} hat keine Linux-Shell: von dort lässt sich nichts bereitstellen.", src_name.clone())));
        }
        if !self.nodes.get(ni).is_some_and(|n| n.as_ref().is_some_and(|n| n.power)) {
            return Err(refuse(msg!("{} ist ausgeschaltet.", src_name.clone())));
        }
        if self.os(ni).is_some_and(|o| o.units.deploys.iter().any(|d| d.state == RunState::Running)) {
            return Err(refuse(msg!("Auf {} läuft schon eine Bereitstellung.", src_name.clone())));
        }
        let mut hosts: Vec<u32> = Vec::new();
        for &h in &spec.hosts {
            if !hosts.contains(&h) {
                hosts.push(h);
            }
        }
        if hosts.is_empty() {
            return Err(refuse(msg!("Eine Bereitstellung braucht mindestens einen Host.")));
        }
        for &h in &hosts {
            let n = self.node_index(h).ok_or(ActErr::UnknownDevice(h))?;
            if !linux(self, n) {
                let name = self.node(h).map(|n| n.name.clone()).unwrap_or_default();
                return Err(refuse(msg!("{} hat keine Linux-Shell und nimmt keine Einheit.", name)));
            }
        }
        if !unit::name_ok(&spec.unit) {
            return Err(refuse(msg!("\u{201e}{}\u{201c} taugt nicht als Name einer Einheit.", spec.unit.clone())));
        }
        let name_u = unit::unit_name(&spec.unit);
        // what goes: the files and folders as they are now
        let os = self.os(ni).ok_or(ActErr::NoIp(spec.src))?;
        let mut payload: Vec<(String, Option<Vec<u8>>)> = Vec::new();
        let mut unit_text: Option<Vec<u8>> = None;
        let target_unit = unit::path_of(&name_u);
        for f in &spec.files {
            let p = os.files.norm(f).map_err(|_| refuse(msg!("Den Pfad {} gibt es auf {} nicht.", f.clone(), src_name.clone())))?;
            if !os.files.exists(&p) {
                return Err(refuse(msg!("Den Pfad {} gibt es auf {} nicht.", f.clone(), src_name.clone())));
            }
            let prefix = format!("{}/", p);
            for r in os.files.records() {
                if r.path == p || r.path.starts_with(&prefix) {
                    if r.dir {
                        payload.push((r.path.clone(), None));
                    } else {
                        // the unit file may lie among them
                        if r.path.rsplit('/').next() == Some(name_u.as_str()) && unit_text.is_none() {
                            unit_text = Some(r.data.clone());
                        }
                        payload.push((r.path.clone(), Some(r.data.clone())));
                    }
                }
            }
        }
        // the unit file: in /etc/systemd/system of the source, else among what goes
        if let Ok(b) = os.files.read(&target_unit) {
            unit_text = Some(Vec::from(b));
        }
        let Some(text) = unit_text else {
            return Err(refuse(msg!("Die Einheit {} gibt es auf {} nicht: weder in {} noch unter den Dateien.", name_u.clone(), src_name.clone(), String::from(unit::UNIT_DIR))));
        };
        if !payload.iter().any(|p| p.0 == target_unit) {
            payload.push((target_unit.clone(), Some(text)));
        }
        let bytes: u64 = payload.iter().map(|p| p.1.as_ref().map_or(0, |b| b.len() as u64)).sum();
        // the run
        let now = self.now;
        let host_runs: Vec<HostRun> = hosts.iter().map(|&d| HostRun { dev: d, name: self.node(d).map(|n| n.name.clone()).unwrap_or_default(), state: HostState::Pending, since: now, note: msg!("wartet") }).collect();
        let id = self.os(ni).map_or(0, |o| o.units.deploys.len() as u32) + 1;
        if let Some(os) = self.os_mut(ni) {
            os.units.deploys.push(DeployRun { id, src: spec.src, unit: name_u.clone(), state: RunState::Running, start: now, end: None, hosts: host_runs, log: Vec::new(), dropped: 0 });
        }
        let mut spec = spec;
        spec.hosts = hosts;
        spec.unit = name_u;
        let svc = Deployer { spec, id, payload, bytes, cur: 0, phase: Phase::Copy, deadline: 0, sock: None, got: Vec::new(), last_why: msg!("keine Antwort"), backups: Vec::new() };
        let dev = self.dev_of(ni);
        let name = format!("deploy-{}", id);
        self.add_service(dev, &name, Box::new(svc), true)?;
        if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.name == name)) {
            s.console = true;
        }
        Ok(id)
    }

    /// Action::DeployAbort
    pub(crate) fn deploy_abort(&mut self, dev: u32, id: u32) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let name = format!("deploy-{}", id);
        let slot = self.os(ni).and_then(|o| o.svc(&name)).filter(|s| s.running).map(|s| s.id);
        let Some(sid) = slot else {
            return Err(ActErr::Refused(msg!("Diese Bereitstellung läuft nicht.")));
        };
        self.svc_call(ni, sid, None, |s, api| {
            if let Some(d) = s.as_any_mut().downcast_mut::<Deployer>() {
                d.abort(api);
            }
        });
        Ok(())
    }

    /// the deployments a device started, oldest first
    pub fn deployments(&self, dev: u32) -> &[DeployRun] {
        self.node_index(dev).and_then(|ni| self.os(ni)).map_or(&[], |o| &o.units.deploys)
    }

    /// every deployment of every device
    pub fn deployments_all(&self) -> Vec<&DeployRun> {
        self.devices().flat_map(|d| self.deployments(d).iter()).collect()
    }
}
