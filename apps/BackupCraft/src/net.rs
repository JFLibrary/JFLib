// net -- the console's connection: requests and their answers, periodic refresh of what the
// visible page shows, the typed data the pages read, and events for toasts
//
//   Net::fixture(now) / Net::live(url, trust) / Net::offline()
//   want(path, period_ms, now)        keep `path` fresh (GET at most every period); pages call
//                                     it every frame for what they show
//   get(path)                         one GET now
//   action(method, path, body, label, refresh, after) -> id
//                                     a change; on success the paths in `refresh` are
//                                     fetched again, `after` says what the app does next
//   pump(now) -> wake soon            send due polls, take the answers into `data`, collect
//                                     events (finished actions, failures, lost login)
//   data                              Data: summary, jobs, hosts, repositories, sessions,
//                                     operations, users, ... decoded once per answer
//
// Every request carries the session token; a 401 answer ends the session (event Unauthorized).

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::Method;
use bc_proto::j::{self, from_list, Json, Value};
use bc_proto::model::*;

use crate::fixture::Fixture;
use crate::live::Live;

/// one answer of the backend
pub struct Response {
    pub id: u64,
    pub status: u16,
    pub body: Value,
}

/// where requests go
pub enum Backend {
    /// the in-process demo server
    Fixture(Box<Fixture>),
    /// a BackupCraft server over HTTP(S)
    Live(Live),
    /// nothing yet (login screen)
    Offline,
}

/// what the app does after an action succeeded
#[derive(Clone, Debug, PartialEq, Default)]
pub enum After {
    #[default]
    Nothing,
    /// a login answer: store the token
    Login,
    /// an operation started: follow it in the activity bar
    Operation,
    /// a pairing code arrived
    Pairing,
    /// the recovery kit's private key arrived (show it once)
    RecoveryKit,
    /// an SSH host key arrived
    SshFingerprint,
    /// close the open wizard
    CloseWizard,
    /// the first setup of a server is done: sign in
    Setup,
}

/// what happened (the app turns it into toasts and follow-ups)
#[derive(Clone, Debug)]
pub enum Event {
    Done { label: String, body: Value, after: After },
    Failed { label: String, code: String, message: String },
    /// a followed operation finished
    OpFinished(Operation),
    /// the login is no longer valid
    Unauthorized,
}

#[derive(Clone, Default)]
struct Poll {
    period: u32,
    last: u32,
    inflight: bool,
    seen: u32,
}

#[derive(Clone)]
enum Pending {
    Get(String),
    Action { label: String, refresh: Vec<String>, after: After },
}

/// the typed data of the answers
#[derive(Default)]
pub struct Data {
    pub info: Option<ServerInfo>,
    pub me: Option<User>,
    pub summary: Option<Summary>,
    pub jobs: Vec<Job>,
    pub hosts: Vec<Host>,
    pub repos: Vec<Repository>,
    pub sessions: Vec<Session>,
    pub ops: Vec<Operation>,
    pub users: Vec<User>,
    pub creds: Vec<Credential>,
    pub audit: Vec<AuditEntry>,
    pub settings: Vec<Setting>,
    pub caps: Vec<Capability>,
    pub config_backups: Vec<ConfigBackup>,
    pub recovery: Option<RecoveryStatus>,
    pub notify: Option<NotifyStatus>,
    pub update: Option<UpdateStatus>,
    pub replicas: Vec<Replica>,
    pub plans: Vec<FailoverPlan>,
    pub cdp: Vec<CdpStatus>,
    pub fleet: Vec<FleetServer>,
    pub fleet_tokens: Vec<FleetToken>,
    pub tape: Option<TapeStatus>,
    pub tapes: Vec<Cartridge>,
    pub pools: Vec<MediaPool>,
    pub tape_sets: Vec<TapeSet>,
    pub hypervisors: Vec<Hypervisor>,
    pub vms: Vec<Vm>,
    pub instant: Vec<InstantSession>,
    pub boot_checks: Vec<BootCheckResult>,
    /// the console text of instantly started VMs by session id
    pub consoles: BTreeMap<String, String>,
    /// volumes by host id
    pub volumes: BTreeMap<String, Vec<Volume>>,
    /// restore points by job id
    pub points: BTreeMap<String, Vec<RestorePoint>>,
    /// listings by request path
    pub listings: BTreeMap<String, Listing>,
    /// session logs by session id
    pub logs: BTreeMap<String, Vec<LogLine>>,
    /// paths answered at least once
    pub loaded: BTreeSet<String>,
    /// the last error per path
    pub errors: BTreeMap<String, String>,
    /// bumped on every change
    pub ver: u64,
}

impl Data {
    fn apply(&mut self, path: &str, v: &Value) {
        let p = path.split('?').next().unwrap_or(path);
        let seg: Vec<&str> = p.trim_matches('/').split('/').collect();
        match seg.as_slice() {
            ["api"] => self.info = ServerInfo::from_json(v),
            ["api", "auth", "me"] => self.me = User::from_json(v),
            ["api", "summary"] => self.summary = Summary::from_json(v),
            ["api", "jobs"] => self.jobs = from_list(v),
            ["api", "jobs", id] => {
                if let Some(job) = Job::from_json(v) {
                    match self.jobs.iter_mut().find(|j| j.id == *id) {
                        Some(j) => *j = job,
                        None => self.jobs.push(job),
                    }
                }
            }
            ["api", "jobs", id, "points"] => {
                self.points.insert(String::from(*id), from_list(v));
            }
            ["api", "hosts"] => self.hosts = from_list(v),
            ["api", "repositories"] => self.repos = from_list(v),
            ["api", "sessions"] => self.sessions = from_list(v),
            ["api", "sessions", id, "log"] => {
                self.logs.insert(String::from(*id), from_list(v));
            }
            ["api", "operations"] => self.ops = from_list(v),
            ["api", "operations", id] => {
                if let Some(op) = Operation::from_json(v) {
                    match self.ops.iter_mut().find(|o| o.id == *id) {
                        Some(o) => *o = op,
                        None => self.ops.insert(0, op),
                    }
                }
            }
            ["api", "users"] => self.users = from_list(v),
            ["api", "credentials"] => self.creds = from_list(v),
            ["api", "audit"] => self.audit = from_list(v),
            ["api", "settings"] => self.settings = from_list(v),
            ["api", "capabilities"] => self.caps = from_list(v),
            ["api", "config", "backups"] => self.config_backups = from_list(v),
            ["api", "recovery"] => self.recovery = RecoveryStatus::from_json(v),
            ["api", "notify"] => self.notify = NotifyStatus::from_json(v),
            ["api", "update"] | ["api", "update", "check"] => self.update = UpdateStatus::from_json(v),
            ["api", "replicas"] => self.replicas = from_list(v),
            ["api", "failover-plans"] => self.plans = from_list(v),
            ["api", "cdp"] => self.cdp = from_list(v),
            ["api", "fleet"] => self.fleet = from_list(v),
            ["api", "fleet", "tokens"] => self.fleet_tokens = from_list(v),
            ["api", "tape"] => self.tape = TapeStatus::from_json(v),
            ["api", "tape", "tapes"] => self.tapes = from_list(v),
            ["api", "tape", "pools"] => self.pools = from_list(v),
            ["api", "tape", "sets"] => self.tape_sets = from_list(v),
            ["api", "hypervisors"] => self.hypervisors = from_list(v),
            ["api", "vms"] => self.vms = from_list(v),
            ["api", "instant-start"] => self.instant = from_list(v),
            ["api", "instant-start", id, "console"] => {
                self.consoles.insert(String::from(*id), String::from(j::str_of(v, "text")));
            }
            ["api", "boot-checks"] => self.boot_checks = from_list(v),
            ["api", "hosts", id, "volumes"] => {
                self.volumes.insert(String::from(*id), from_list(v));
            }
            ["api", "tape", "sets", _, "browse"] => {
                if let Some(l) = Listing::from_json(v) {
                    self.listings.insert(String::from(path), l);
                }
            }
            ["api", "fleet", "servers", id, "check"] => {
                if let Some(f) = FleetServer::from_json(v) {
                    if let Some(x) = self.fleet.iter_mut().find(|x| x.id == *id) {
                        *x = f;
                    }
                }
            }
            ["api", "points", _, "browse"] | ["api", "hosts", _, "browse"] => {
                if let Some(l) = Listing::from_json(v) {
                    self.listings.insert(String::from(path), l);
                }
            }
            _ => {}
        }
        self.loaded.insert(String::from(path));
        self.errors.remove(path);
        self.ver += 1;
    }

    /// a setting's value ("" when unset)
    pub fn setting(&self, key: &str) -> &str {
        self.settings.iter().find(|s| s.key == key).map_or("", |s| s.value.as_str())
    }

    /// whether the server offers a staged feature
    pub fn capable(&self, name: &str) -> bool {
        self.caps.iter().any(|c| c.name == name && c.available)
    }

    pub fn job(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }
    pub fn host(&self, id: &str) -> Option<&Host> {
        self.hosts.iter().find(|h| h.id == id)
    }
    pub fn repo(&self, id: &str) -> Option<&Repository> {
        self.repos.iter().find(|r| r.id == id)
    }
    /// the running operations
    pub fn running_ops(&self) -> impl Iterator<Item = &Operation> {
        self.ops.iter().filter(|o| !o.status.finished())
    }
}

/// the connection state shown in the title bar
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Link {
    Offline,
    Connecting,
    Online,
    Lost,
}

pub struct Net {
    backend: Backend,
    next: u64,
    pending: BTreeMap<u64, Pending>,
    polls: BTreeMap<String, Poll>,
    follow: BTreeSet<String>,
    pub data: Data,
    pub events: Vec<Event>,
    pub token: String,
    pub link: Link,
    pub server: String,
    frame: u32,
}

impl Net {
    fn with(backend: Backend, server: &str) -> Net {
        Net {
            backend,
            next: 1,
            pending: BTreeMap::new(),
            polls: BTreeMap::new(),
            follow: BTreeSet::new(),
            data: Data::default(),
            events: Vec::new(),
            token: String::new(),
            link: Link::Offline,
            server: String::from(server),
            frame: 0,
        }
    }

    /// the demo server
    pub fn fixture(now: i64) -> Net {
        Net::with(Backend::Fixture(Box::new(Fixture::new(now))), "demo.backupcraft.local")
    }

    /// a BackupCraft server
    pub fn live(url: &str, trust: netio::trust::ServerTrust) -> Net {
        let live = Live::new(url, trust);
        // the address as the console understood it ("http://localhost:7642")
        let shown = live.url().map_or(String::from(url), |u| u.origin());
        let mut n = Net::with(Backend::Live(live), &shown);
        n.link = Link::Connecting;
        n
    }

    pub fn offline() -> Net {
        Net::with(Backend::Offline, "")
    }

    /// the CA a server presented that the console does not trust (yet)
    pub fn presented(&self) -> Option<netio::trust::Presented> {
        match &self.backend {
            Backend::Live(l) => l.presented(),
            _ => None,
        }
    }

    fn send(&mut self, method: Method, path: &str, body: Option<Value>, p: Pending) -> u64 {
        let id = self.next;
        self.next += 1;
        self.pending.insert(id, p);
        match &mut self.backend {
            Backend::Fixture(f) => f.request(id, method, path, body, &self.token),
            Backend::Live(l) => l.request(id, method, path, body, &self.token),
            Backend::Offline => {}
        }
        id
    }

    /// keep a path fresh: fetched now if never or more than `period` ms ago
    pub fn want(&mut self, path: &str, period: u32, now: u32) {
        let frame = self.frame;
        let due = {
            let p = self.polls.entry(String::from(path)).or_default();
            p.period = period;
            p.seen = frame;
            !p.inflight && (p.last == 0 || now.wrapping_sub(p.last) >= period)
        };
        if due {
            self.fetch(path, now);
        }
    }

    fn fetch(&mut self, path: &str, now: u32) {
        let p = self.polls.entry(String::from(path)).or_default();
        p.inflight = true;
        p.last = now.max(1);
        self.send(Method::Get, path, None, Pending::Get(String::from(path)));
    }

    /// fetch a path once now (also when it is fresh)
    pub fn get(&mut self, path: &str, now: u32) {
        if self.polls.get(path).is_some_and(|p| p.inflight) {
            return;
        }
        self.fetch(path, now);
    }

    /// whether answers wait (the demo server answers at once: a frame that asked for
    /// something takes them before it is shown, so screenshots and scripts see the data)
    pub fn answered(&self) -> bool {
        matches!(&self.backend, Backend::Fixture(f) if f.has_answers())
    }

    /// whether a path's answer arrived at least once
    pub fn loaded(&self, path: &str) -> bool {
        self.data.loaded.contains(path)
    }

    /// a change; `refresh` is fetched again on success
    pub fn action(&mut self, method: Method, path: &str, body: Option<Value>, label: &str, refresh: &[&str], after: After) -> u64 {
        self.send(method, path, body, Pending::Action {
            label: String::from(label),
            refresh: refresh.iter().map(|s| String::from(*s)).collect(),
            after,
        })
    }

    /// follow an operation (activity bar; an event when it finishes)
    pub fn follow(&mut self, op_id: &str) {
        self.follow.insert(String::from(op_id));
    }

    /// forget the session (logout)
    pub fn reset_session(&mut self) {
        self.token.clear();
        self.data = Data::default();
        self.polls.clear();
        self.follow.clear();
    }

    /// advance: the demo server's simulation, due polls of watched paths, the answers ->
    /// true when something may change soon (keep the refresh timer short)
    pub fn pump(&mut self, now: u32, epoch: i64) -> bool {
        self.frame = self.frame.wrapping_add(1);
        let mut busy = false;
        if let Backend::Fixture(f) = &mut self.backend {
            busy |= f.tick(epoch);
        }
        // followed operations
        let follow: Vec<String> = self.follow.iter().cloned().collect();
        for id in follow {
            let path = alloc::format!("/api/operations/{}", id);
            self.want(&path, 1000, now);
            busy = true;
        }
        // polls nobody asked for in a while stop
        let frame = self.frame;
        self.polls.retain(|_, p| p.inflight || frame.wrapping_sub(p.seen) < 600);
        let due: Vec<String> = self
            .polls
            .iter()
            .filter(|(_, p)| !p.inflight && p.last != 0 && now.wrapping_sub(p.last) >= p.period)
            .map(|(k, _)| k.clone())
            .collect();
        for path in due {
            self.fetch(&path, now);
        }
        let answers = match &mut self.backend {
            Backend::Fixture(f) => f.take(),
            Backend::Live(l) => {
                let a = l.take();
                if let Some(state) = l.link_state() {
                    self.link = state;
                }
                a
            }
            Backend::Offline => Vec::new(),
        };
        if !answers.is_empty() {
            busy = true;
        }
        for r in answers {
            self.answer(r, now);
        }
        busy || !self.pending.is_empty()
    }

    fn answer(&mut self, r: Response, now: u32) {
        let Some(p) = self.pending.remove(&r.id) else { return };
        if r.status == 401 && !matches!(&p, Pending::Action { after: After::Login, .. }) {
            if !self.token.is_empty() {
                self.events.push(Event::Unauthorized);
            }
            if let Pending::Get(path) = &p {
                if let Some(q) = self.polls.get_mut(path) {
                    q.inflight = false;
                }
            }
            return;
        }
        if r.status != 0 && self.link != Link::Online && !self.token.is_empty() {
            self.link = Link::Online;
        }
        match p {
            Pending::Get(path) => {
                if let Some(q) = self.polls.get_mut(&path) {
                    q.inflight = false;
                    q.last = now.max(1);
                }
                if (200..300).contains(&r.status) {
                    self.data.apply(&path, &r.body);
                    // a followed operation that finished
                    if let Some(id) = path.strip_prefix("/api/operations/") {
                        if self.follow.contains(id) {
                            if let Some(op) = Operation::from_json(&r.body) {
                                if op.status.finished() {
                                    self.follow.remove(id);
                                    self.events.push(Event::OpFinished(op));
                                    // what the operation changed
                                    for p in ["/api/jobs", "/api/sessions", "/api/summary", "/api/operations", "/api/repositories", "/api/hosts"] {
                                        self.get(p, now);
                                    }
                                }
                            }
                        }
                    }
                } else {
                    let msg = String::from(j::str_of(&r.body, "message"));
                    self.data.errors.insert(path, if msg.is_empty() { alloc::format!("HTTP {}", r.status) } else { msg });
                    self.data.ver += 1;
                }
            }
            Pending::Action { label, refresh, after } => {
                if (200..300).contains(&r.status) {
                    if r.status == 202 || after == After::Operation {
                        let id = j::str_of(&r.body, "operation_id");
                        if !id.is_empty() {
                            let id = String::from(id);
                            self.follow(&id);
                            self.get("/api/operations", now);
                        }
                    }
                    for p in &refresh {
                        self.get(p, now);
                    }
                    self.events.push(Event::Done { label, body: r.body, after });
                } else {
                    let code = String::from(j::str_of(&r.body, "code"));
                    let mut message = String::from(j::str_of(&r.body, "message"));
                    if message.is_empty() {
                        message = if r.status == 0 {
                            String::from(crate::tr::t("Keine Verbindung zum Server"))
                        } else {
                            alloc::format!("HTTP {}", r.status)
                        };
                    }
                    self.events.push(Event::Failed { label, code, message });
                }
            }
        }
    }
}
