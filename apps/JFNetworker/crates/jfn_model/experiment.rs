// experiment -- a parameter sweep as data (SA2): which configurations of a project to compare under
// which load, how long to measure and what counts as good enough.  jfn_sim's experiment.rs runs one;
// nothing here runs anything
//
//   Experiment::from_text(text) -> Result<_, Msg> / to_text()   a .jfnex file (indented JSON)
//   Experiment::problems(&project) -> Vec<Msg>   what keeps it from running on this project, every
//                                      finding with its axis and what would be right
//   Experiment::configs() -> Vec<Config>   the configurations in their order: the series, within
//                                      each the cross product of its axes (the first axis slowest)
//   Experiment::levels() -> Vec<Level>     the load levels, Level::text()
//   Experiment::seeds_or(project_seed)     the seeds of a run; prices_of(): the assumed prices with
//                                      what it names instead (catalog::Prices)
//   Slo::limits() -> SloLimits         the limits in ns and parts per million; held(...), text()
//   Config::label() / labels()         a configuration in words, its value per axis
//   patch_hw / patch_config / patch_load / patch_pool / patch_clones   what an axis does to a copy
//                                      of the project (jfn_sim's experiment.rs calls them)
//   SavedRun                           a run kept by its name in the project's "runs" (SA3): the name,
//                                      the date, the experiment as it ran and its results as jfn_sim's
//                                      Sweep::to_value writes them; KEEP_RUNS, KEEP_POINTS bound them
//   known_keys(kind)                   the settings of the services this checks by name
//   set_path(&mut config, path, value) a member of a service's config by its path
//
// The file:
//
//   {
//     "format": "jfnex", "version": 1, "title": "Größer oder mehr?",
//     "project": "scale_up_vs_out.jfnet",        the project, relative to this file ("" in a
//                                                project's own section "experiments")
//     "name": "vergleich",                       its name (--name; a project's section may hold many)
//     "load": {"dev": "GEN", "svc": "load", "rps": [100, 200, 400]},   or "users": [8, 16, 32]
//     "timing": {"warmup_s": 5, "settle_s": 3, "measure_s": 20, "seeds": [1, 2]},
//     "slo": {"p99_ms": 200, "errors_pct": 1},   and p50_ms, p95_ms, p999_ms
//     "axes": [...],                             axes every series has
//     "series": [{"name": "vertikal", "axes": [...]}, {"name": "horizontal", "axes": [...]}],
//     "prices": {"core": 0.05},                  €/h instead of catalog::Prices::ASSUMED, by key
//     "cost_exclude": ["SW1"]                    devices that do not count for the cost
//   }
//
// An axis has a "kind" and a list of values:
//
//   {"kind": "hw", "dev": "WEB1", "values": [{"preset": "server-m"}, {"cores": 8, "speed": 1.5,
//    "ram_gb": 32, "swap_gb": 4, "disk": "nvme", "disk_gb": 960}]}   (or "group": a group of the
//    project; figures on top of the preset, else on what the device has)
//   {"kind": "config", "dev": "WEB1", "path": "http-server.workers", "values": [1, 4, 16]}  a
//    service's kind or name, then the member of its config; "routes.0.cpu_ms" goes into arrays
//   {"kind": "backends", "dev": "LB", "svc": "lb", "pool": ["WEB1", "WEB2", "WEB3"], "counts": [1, 3]}
//    the first N hosts of the pool behind a load balancer, the others switched off
//   {"kind": "backends", "dev": "LB", "svc": "lb", "template": "WEB1", "counts": [1, 2, 4]}
//    the first N copies of a device: a copy has the next addresses and the next free port of the
//    switch the device hangs on
//
// Levels are strictly rising (the knee is the highest level up to which every level holds the SLO).
// Every run boots each configuration once per seed, warms up at "warm" of "load" (default: the first
// level), and then every level starts from that same warm state: settle_s after the change of load,
// measure_s of measuring.  Times are whole seconds, which are the slots of the latency windows.
// Texts are messages for the app's catalogue where they tell what is wrong (jfn_model::msg); a
// configuration's labels are the words of the tables, which stay German like a scenario's report.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::{Json, Value};
use agent_core::json::{self, Limits};
use agent_core::record;

use crate::catalog::{self, DiskClass, Hw, Prices};
use crate::msg;
use crate::msg::Msg;
use crate::project::{Device, PortRef, Project};
use crate::scenario::scaled;

/// the format name in every experiment file
pub const FORMAT: &str = "jfnex";
/// the newest file version this program writes and reads
pub const VERSION: u32 = 1;
/// configurations times levels times seeds of one experiment at most
pub const MAX_RUNS: usize = 100_000;
/// the latency windows of the services keep this many seconds
pub const MAX_MEASURE_S: u64 = 280;

record! {
    /// one thing that varies
    pub struct Axis {
        /// hw, config, backends
        pub kind: String,
        /// the device it acts on, or a group of the project ("group")
        pub dev: String,
        pub group: String,
        /// its column in the results (default: from the rest)
        pub name: String,
        /// hw: figures; config: the member's values
        pub values: Vec<Value>,
        /// config: "http-server.workers"
        pub path: String,
        /// backends: the load balancer's name on `dev`
        pub svc: String,
        /// backends: the hosts it takes the first N of
        pub pool: Vec<String>,
        /// backends: the device it makes N of
        pub template: String,
        pub counts: Vec<u32>,
    }
}

record! {
    /// the configurations of one family: its axes across the experiment's
    pub struct Series {
        pub name: String,
        pub axes: Vec<Axis>,
    }
}

record! {
    /// the load: a generator (kind http-load) and its levels, requests per second or users
    pub struct LoadAxis {
        pub dev: String,
        pub svc: String,
        pub rps: Vec<u64>,
        pub users: Vec<u32>,
        /// the level the warm-up runs at (0: the first)
        pub warm: u64,
    }
}

record! {
    /// how long, in whole seconds of virtual time
    pub struct Timing {
        pub warmup_s: u64,
        /// after a change of load, before the measuring starts
        pub settle_s: u64,
        pub measure_s: u64,
        /// none: the project's
        pub seeds: Vec<u64>,
    }
}

record! {
    /// what counts as good enough (decimals allowed; null: no limit)
    pub struct Slo {
        pub p50_ms: Value,
        pub p95_ms: Value,
        pub p99_ms: Value,
        pub p999_ms: Value,
        pub errors_pct: Value,
    }
}

record! {
    /// a sweep (see the head of this file)
    pub struct Experiment {
        pub format: String,
        pub version: u32,
        pub name: String,
        pub title: String,
        pub project: String,
        pub load: LoadAxis,
        pub timing: Timing,
        pub slo: Slo,
        pub axes: Vec<Axis>,
        pub series: Vec<Series>,
        pub prices: BTreeMap<String, Value>,
        pub cost_exclude: Vec<String>,
    }
}

/// runs a project keeps at most, and points (configurations times levels) of one kept run: about
/// 300 bytes each in the file, so a project grows by about 10 MiB at most through the points of its runs
pub const KEEP_RUNS: usize = 16;
pub const KEEP_POINTS: usize = 2000;

record! {
    /// a run kept by its name (Project::runs)
    pub struct SavedRun {
        pub name: String,
        /// when it ran, "2026-11-03 14:05"
        pub date: String,
        /// the experiment as it ran
        pub experiment: Experiment,
        /// the results: jfn_sim's Sweep::to_value (without the details of each run)
        pub result: Value,
    }
}

/// one load level
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// requests per second (open loop)
    Rps(u64),
    /// virtual users (closed loop)
    Users(u32),
}

impl Level {
    /// the number of the level
    pub fn value(self) -> u64 {
        match self {
            Level::Rps(r) => r,
            Level::Users(u) => u as u64,
        }
    }
    /// "400/s", "32 Nutzer"
    pub fn text(self) -> String {
        match self {
            Level::Rps(r) => format!("{}/s", r),
            Level::Users(u) => format!("{} Nutzer", u),
        }
    }
}

/// one configuration: its series' name and for each axis the value it takes
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub series: String,
    /// (the axis, the index of its value)
    pub picks: Vec<(Axis, usize)>,
}

impl Config {
    /// the columns of the results: (the axis' name, the value as text)
    pub fn labels(&self) -> Vec<(String, String)> {
        self.picks.iter().map(|(a, i)| (a.column(), a.value_label(*i))).collect()
    }
    /// "größer 8 Kerne", "mehr Hosts 4" as the tables show it: an axis with one value says nothing
    pub fn label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.series.is_empty() {
            parts.push(self.series.clone());
        }
        for (a, i) in self.picks.iter().filter(|(a, _)| a.len() > 1) {
            parts.push(if a.kind == "hw" { a.value_label(*i) } else { format!("{} {}", a.column(), a.value_label(*i)) });
        }
        if parts.is_empty() { String::from("-") } else { parts.join(" ") }
    }
}

/// the limits of an SLO: latencies in ns, errors in parts per million
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SloLimits {
    pub p50: Option<u64>,
    pub p95: Option<u64>,
    pub p99: Option<u64>,
    pub p999: Option<u64>,
    pub errors_ppm: Option<u64>,
}

/// thousandths of `unit` from a JSON number or a text with a number
fn decimal_of(v: &Value, unit: u64) -> Option<u64> {
    match v {
        Value::Number(n) => scaled(n.as_str(), unit),
        Value::String(s) => scaled(s.trim(), unit),
        _ => None,
    }
}

/// an SLO's limit: None without one, Err when it is no number
fn limit(v: &Value, unit: u64) -> Result<Option<u64>, ()> {
    match v {
        Value::Null => Ok(None),
        x => decimal_of(x, unit).map(Some).ok_or(()),
    }
}

impl Slo {
    /// the limits (what is no number counts as none: problems() names it)
    pub fn limits(&self) -> SloLimits {
        let ms = |v: &Value| limit(v, 1_000_000).ok().flatten();
        SloLimits { p50: ms(&self.p50_ms), p95: ms(&self.p95_ms), p99: ms(&self.p99_ms), p999: ms(&self.p999_ms), errors_ppm: limit(&self.errors_pct, 10_000).ok().flatten() }
    }
}

impl SloLimits {
    /// the quantiles (ns) and errors (ppm) of a point hold it; a quantile without any answers
    /// does not
    pub fn held(&self, p50: Option<u64>, p95: Option<u64>, p99: Option<u64>, p999: Option<u64>, errors_ppm: u64) -> bool {
        let under = |lim: Option<u64>, v: Option<u64>| lim.is_none_or(|l| v.is_some_and(|x| x <= l));
        under(self.p50, p50) && under(self.p95, p95) && under(self.p99, p99) && under(self.p999, p999) && self.errors_ppm.is_none_or(|l| errors_ppm <= l)
    }
    /// "p99 bis 200 ms, Fehler bis 1 %"
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        let ms = |ns: u64| fixed(ns, 1_000_000);
        for (name, lim) in [("p50", self.p50), ("p95", self.p95), ("p99", self.p99), ("p99.9", self.p999)] {
            if let Some(l) = lim {
                parts.push(format!("{} bis {} ms", name, ms(l)));
            }
        }
        if let Some(e) = self.errors_ppm {
            parts.push(format!("Fehler bis {} %", fixed(e, 10_000)));
        }
        if parts.is_empty() { String::from("kein SLO") } else { parts.join(", ") }
    }
}

/// a number in `unit`s with its fraction as far as it has one: (1500000, 1000000) -> "1.5"
pub fn fixed(v: u64, unit: u64) -> String {
    let (int, frac) = (v / unit, v % unit);
    if frac == 0 {
        return format!("{}", int);
    }
    let width = format!("{}", unit - 1).len();
    let f = format!("{:0w$}", frac, w = width);
    format!("{}.{}", int, f.trim_end_matches('0'))
}

/// the settings of the services whose names are checked: an unknown one would only give a curve
/// that does not move (a config's reader takes what it knows and ignores the rest)
pub fn known_keys(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "http-server" => &["port", "root", "workers", "queue", "time_ms", "jitter_ms", "error_rate", "error_status", "keepalive_ms", "max_requests", "header_timeout_ms", "max_body", "backlog", "server", "routes", "kv", "db", "cpu_ms", "serial_us", "usl", "mem_mb", "conn_kb", "data_mb", "tls"],
        "http-load" => &["target", "targets", "mode", "users", "think_ms", "think", "rps", "start_rps", "arrival", "ramp_ms", "delay_ms", "duration_ms", "requests", "timeout_ms", "connect_timeout_ms", "keepalive", "max_conns", "balance", "mix"],
        "lb" => &["mode", "port", "algorithm", "hash", "vnodes", "backends", "health", "retries", "timeout_ms", "connect_timeout_ms", "drain_ms", "max_conns", "keepalive", "tag", "cpu_us", "mem_mb", "conn_kb", "data_mb"],
        "db" => &["port", "tables", "seed", "pool_mb", "page_kb", "fanout", "max_conns", "durability", "flush_ms", "isolation", "lock_timeout_ms", "txn_idle_ms", "checkpoint_s", "touched_max", "stmt_us", "page_us", "row_us", "write_us", "redo_mbps", "max_rows", "write_rows_max", "wal_tail", "mem_mb", "conn_kb", "data_mb", "disk"],
        "kv" => &["nodes", "port", "hb_port", "cpu_us", "mem_mb", "conn_kb", "data_mb", "vnodes", "rf", "w", "r", "consistency", "read_repair", "timeout_ms", "heartbeat_ms", "down_ms", "disk"],
        "cache" => &["backend", "port", "entries", "max_kb", "ttl_ms", "cache_control", "hit_us", "collapse", "timeout_ms", "connect_timeout_ms", "mem_mb", "conn_kb", "data_mb"],
        "worker" => &["queue", "topic", "concurrency", "time_ms", "cpu_ms", "mem_mb", "conn_kb", "data_mb", "jitter_ms", "error_rate", "wait_ms", "idle_ms", "timeout_ms"],
        "mq" => &["port", "visibility_ms", "max_deliveries", "max_depth", "durable", "topics"],
        _ => return None,
    })
}

/// the path's value in a config: `path` is "routes.0.cpu_ms"; objects on the way are made, an array
/// must have that element
pub fn set_path(v: &mut Value, path: &str, value: Value) -> Result<(), Msg> {
    let (head, rest) = match path.split_once('.') {
        Some((h, r)) => (h, Some(r)),
        None => (path, None),
    };
    if head.is_empty() {
        return Err(msg!("Im Pfad fehlt ein Teil: {}", path));
    }
    match v {
        Value::Object(_) | Value::Null => {
            if matches!(v, Value::Null) {
                *v = Value::Object(Vec::new());
            }
            let Value::Object(o) = v else {
                return Ok(());
            };
            let i = match o.iter().position(|(k, _)| k == head) {
                Some(i) => i,
                None => {
                    o.push((String::from(head), Value::Null));
                    o.len() - 1
                }
            };
            match rest {
                Some(r) => set_path(&mut o[i].1, r, value),
                None => {
                    o[i].1 = value;
                    Ok(())
                }
            }
        }
        Value::Array(a) => {
            let Some(x) = head.parse::<usize>().ok().and_then(|n| a.get_mut(n)) else {
                return Err(msg!("Im Pfad gibt es kein Element {}.", head));
            };
            match rest {
                Some(r) => set_path(x, r, value),
                None => {
                    *x = value;
                    Ok(())
                }
            }
        }
        _ => Err(msg!("\u{201e}{}\u{201c} liegt nicht in einem Objekt.", head)),
    }
}

/// the devices an axis acts on: its device or the members of its group
fn targets(a: &Axis, p: &Project) -> Vec<u32> {
    if !a.group.is_empty() {
        return match p.groups.iter().find(|g| g.name.eq_ignore_ascii_case(&a.group)) {
            Some(g) => crate::group::Groups::of(p).devices(p, g.id),
            None => Vec::new(),
        };
    }
    p.find(&a.dev).map(|d| alloc::vec![d.id]).unwrap_or_default()
}

/// the service a config path names first: by name, else by kind
fn service_of<'a>(d: &'a Device, first: &str) -> Option<&'a crate::project::ServiceDef> {
    d.services.iter().find(|s| s.name.eq_ignore_ascii_case(first)).or_else(|| d.services.iter().find(|s| s.kind == first))
}

impl Axis {
    /// its column in the results
    pub fn column(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        let who = if self.group.is_empty() { self.dev.as_str() } else { self.group.as_str() };
        match self.kind.as_str() {
            "hw" => format!("hw {}", who),
            "config" => format!("{} {}", who, self.path),
            "backends" => format!("backends {}", who),
            k => String::from(k),
        }
    }
    /// how many values it has
    pub fn len(&self) -> usize {
        if self.kind == "backends" { self.counts.len() } else { self.values.len() }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// a value as text in the results
    pub fn value_label(&self, i: usize) -> String {
        match self.kind.as_str() {
            "backends" => self.counts.get(i).map(|c| format!("{}", c)).unwrap_or_default(),
            "hw" => self.values.get(i).map(hw_label).unwrap_or_default(),
            _ => self.values.get(i).map(value_text).unwrap_or_default(),
        }
    }
}

/// a JSON value as a word
fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        x => json::stringify(x).unwrap_or_default(),
    }
}

/// figures of a hw axis as words: "server-m", "8 Kerne", "8 Kerne 150 %"
fn hw_label(v: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = v.get("preset").and_then(|x| x.as_str()) {
        parts.push(String::from(s));
    }
    if let Some(c) = v.get("cores").and_then(|x| x.as_u64()) {
        parts.push(format!("{} Kerne", c));
    }
    if let Some(s) = v.get("speed").and_then(|x| decimal_of(x, 100)) {
        parts.push(format!("{} %", s));
    }
    if let Some(g) = v.get("ram_gb").and_then(|x| x.as_u64()) {
        parts.push(format!("{} GiB", g));
    }
    if let Some(g) = v.get("swap_gb").and_then(|x| x.as_u64()) {
        parts.push(format!("{} GiB Swap", g));
    }
    if let Some(s) = v.get("disk").and_then(|x| x.as_str()) {
        parts.push(String::from(s));
    }
    if let Some(g) = v.get("disk_gb").and_then(|x| x.as_u64()) {
        parts.push(format!("{} GiB Platte", g));
    }
    parts.join(" ")
}

/// the figures a value of a hw axis means on a device
fn hw_value(v: &Value, base: Option<Hw>) -> Result<Hw, Msg> {
    if v.as_object().is_none() {
        return Err(msg!("Hardware steht als Objekt da, etwa mit \u{201e}preset\u{201c} oder \u{201e}cores\u{201c}."));
    }
    for (k, _) in v.as_object().unwrap_or(&[]) {
        if !matches!(k.as_str(), "preset" | "cores" | "speed" | "ram_gb" | "swap_gb" | "disk" | "disk_gb") {
            return Err(msg!("\u{201e}{}\u{201c} gibt es bei Hardware nicht (preset, cores, speed, ram_gb, swap_gb, disk, disk_gb).", k));
        }
    }
    let mut hw = match v.get("preset") {
        Some(p) => {
            let id = p.as_str().unwrap_or("");
            catalog::hw_of(id).ok_or_else(|| msg!("Das Modell \u{201e}{}\u{201c} hat keine Hardware-Angaben.", id))?
        }
        None => base.unwrap_or(Hw { cores: 1, speed_pm: 1000, ram_mb: 1024, swap_mb: 1024, disk: DiskClass::Sata, disk_gb: 240 }),
    };
    if let Some(c) = v.get("cores") {
        hw.cores = c.as_u64().filter(|&n| (1..=1024).contains(&n)).ok_or_else(|| msg!("\u{201e}cores\u{201c} ist eine Zahl von 1 bis 1024."))? as u32;
    }
    if let Some(c) = v.get("speed") {
        hw.speed_pm = decimal_of(c, 1000).filter(|&n| (1..=100_000).contains(&n)).ok_or_else(|| msg!("\u{201e}speed\u{201c} ist das Tempo eines Kerns gegen den Referenzkern, etwa 1.5."))? as u32;
    }
    if let Some(c) = v.get("ram_gb") {
        hw.ram_mb = decimal_of(c, 1024).filter(|&n| n > 0).ok_or_else(|| msg!("\u{201e}ram_gb\u{201c} ist eine Größe in GiB."))?;
    }
    if let Some(c) = v.get("swap_gb") {
        hw.swap_mb = decimal_of(c, 1024).ok_or_else(|| msg!("\u{201e}swap_gb\u{201c} ist eine Größe in GiB."))?;
    }
    if let Some(c) = v.get("disk") {
        let s = c.as_str().unwrap_or("");
        hw.disk = DiskClass::parse(s).ok_or_else(|| msg!("Die Plattenart \u{201e}{}\u{201c} gibt es nicht (sata, nvme, nvme-fast, hdd).", s))?;
    }
    if let Some(c) = v.get("disk_gb") {
        hw.disk_gb = c.as_u64().ok_or_else(|| msg!("\u{201e}disk_gb\u{201c} ist eine Größe in GiB."))?;
    }
    Ok(hw.sane())
}

/// what a device has now, for figures that go on top of it
fn hw_base(d: &Device) -> Option<Hw> {
    d.hw.clone().or_else(|| catalog::hw_of(&d.model))
}

/// a hw axis: this value on its devices
pub fn patch_hw(p: &mut Project, a: &Axis, i: usize) -> Result<(), Msg> {
    let v = a.values.get(i).ok_or_else(|| msg!("Diesen Wert gibt es nicht."))?;
    for id in targets(a, p) {
        let Some(d) = p.device_mut(id) else { continue };
        d.hw = Some(hw_value(v, hw_base(d))?);
    }
    Ok(())
}

/// a config axis: this value at its path, on the service of each of its devices
pub fn patch_config(p: &mut Project, a: &Axis, i: usize) -> Result<(), Msg> {
    let v = a.values.get(i).ok_or_else(|| msg!("Diesen Wert gibt es nicht."))?;
    let (first, rest) = a.path.split_once('.').ok_or_else(|| msg!("Der Pfad \u{201e}{}\u{201c} nennt einen Dienst und seine Einstellung, etwa http-server.workers.", a.path))?;
    for id in targets(a, p) {
        let Some(d) = p.device_mut(id) else { continue };
        let name = service_of(d, first).map(|s| s.name.clone()).ok_or_else(|| msg!("{} hat keinen Dienst \u{201e}{}\u{201c}.", d.name.clone(), first))?;
        if let Some(s) = d.services.iter_mut().find(|s| s.name == name) {
            set_path(&mut s.config, rest, v.clone())?;
        }
    }
    Ok(())
}

/// the load axis: the generator's mode and the warm level, without what would end its run
pub fn patch_load(p: &mut Project, l: &LoadAxis) -> Result<(), Msg> {
    let dev = p.find(&l.dev).map(|d| d.id).ok_or_else(|| msg!("Das Gerät \u{201e}{}\u{201c} gibt es nicht.", l.dev))?;
    let d = p.device_mut(dev).ok_or_else(|| msg!("Das Gerät \u{201e}{}\u{201c} gibt es nicht.", l.dev))?;
    let name = d.name.clone();
    let s = d.services.iter_mut().find(|s| s.name.eq_ignore_ascii_case(&l.svc)).ok_or_else(|| msg!("{} hat keinen Dienst \u{201e}{}\u{201c}.", name, l.svc))?;
    if let Value::Object(o) = &mut s.config {
        o.retain(|(k, _)| !matches!(k.as_str(), "duration_ms" | "requests" | "ramp_ms" | "start_rps"));
    }
    let open = !l.rps.is_empty();
    s.config.insert("mode", Value::string(if open { "open" } else { "closed" }));
    if open {
        let warm = if l.warm > 0 { l.warm } else { l.rps.first().copied().unwrap_or(1) };
        s.config.insert("rps", Value::u64(warm));
    } else {
        let warm = if l.warm > 0 { l.warm } else { l.users.first().copied().unwrap_or(1) as u64 };
        s.config.insert("users", Value::u64(warm));
    }
    Ok(())
}

/// the backends of a load balancer in place: (name, address, port)
fn set_backends(p: &mut Project, a: &Axis, list: &[(String, String, u64)]) -> Result<(), Msg> {
    let d = p.find(&a.dev).map(|d| d.id).and_then(|id| p.device_mut(id)).ok_or_else(|| msg!("Das Gerät \u{201e}{}\u{201c} gibt es nicht.", a.dev))?;
    let name = d.name.clone();
    let s = d.services.iter_mut().find(|s| s.name.eq_ignore_ascii_case(&a.svc)).ok_or_else(|| msg!("{} hat keinen Dienst \u{201e}{}\u{201c}.", name, a.svc))?;
    let items: Vec<Value> = list.iter().map(|(n, addr, port)| Value::object([("name", Value::string(n.as_str())), ("addr", Value::string(addr.as_str())), ("port", Value::u64(*port))])).collect();
    s.config.insert("backends", Value::Array(items));
    Ok(())
}

/// the port a load balancer's backends listen on, as its config says (80 without)
fn backend_port(p: &Project, a: &Axis) -> u64 {
    p.find(&a.dev).and_then(|d| d.services.iter().find(|s| s.name.eq_ignore_ascii_case(&a.svc))).and_then(|s| s.config.get("backends")).and_then(|b| b.as_array()).and_then(|b| b.first()).and_then(|b| b.get("port")).and_then(|v| v.as_u64()).unwrap_or(80)
}

/// a backends axis with a pool: the first N hosts behind the load balancer, the others off; `addr`
/// gives a host's address, as the engine found it
pub fn patch_pool(p: &mut Project, a: &Axis, i: usize, addr: &dyn Fn(&str) -> Option<String>) -> Result<(), Msg> {
    let n = a.counts.get(i).copied().ok_or_else(|| msg!("Diesen Wert gibt es nicht."))? as usize;
    let port = backend_port(p, a);
    let mut list = Vec::new();
    for host in a.pool.iter().take(n) {
        let ip = addr(host).ok_or_else(|| msg!("{} hat keine IPv4-Adresse.", host.clone()))?;
        list.push((host.clone(), ip, port));
    }
    set_backends(p, a, &list)?;
    for (k, host) in a.pool.iter().enumerate() {
        let id = p.find(host).map(|d| d.id);
        if let Some(d) = id.and_then(|id| p.device_mut(id)) {
            d.off = k >= n;
        }
    }
    Ok(())
}

/// the address `k` further on in the last place: ("10.0.2.11", 3) -> "10.0.2.14"
fn next_addr(ip: &str, k: u32) -> Option<String> {
    let mut parts: Vec<u32> = ip.split('.').filter_map(|x| x.parse().ok()).collect();
    if parts.len() != 4 {
        return None;
    }
    let last = parts[3] + k;
    if last > 254 {
        return None;
    }
    parts[3] = last;
    Some(format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[3]))
}

/// a backends axis with a template: N copies of a device in all, each with the next address and the
/// next free port of the switch the original hangs on, behind the load balancer; `addr` gives a
/// host's address, as the engine found it
pub fn patch_clones(p: &mut Project, a: &Axis, i: usize, addr: &dyn Fn(&str) -> Option<String>) -> Result<(), Msg> {
    let n = a.counts.get(i).copied().ok_or_else(|| msg!("Diesen Wert gibt es nicht."))?;
    let tpl = p.find(&a.template).cloned().ok_or_else(|| msg!("Das Gerät \u{201e}{}\u{201c} gibt es nicht.", a.template))?;
    let ip = addr(&a.template).ok_or_else(|| msg!("{} hat keine IPv4-Adresse.", a.template.clone()))?;
    let cables: Vec<_> = p.links.iter().filter(|l| l.a.dev == tpl.id || l.b.dev == tpl.id).cloned().collect();
    let [cable] = &cables[..] else {
        return Err(msg!("{} hängt an {} Kabeln; eine Vorlage hängt an genau einem.", tpl.name.clone(), cables.len()));
    };
    let (mine, other) = if cable.a.dev == tpl.id { (cable.a.clone(), cable.b.clone()) } else { (cable.b.clone(), cable.a.clone()) };
    let port = backend_port(p, a);
    let mut list = alloc::vec![(tpl.name.clone(), ip.clone(), port)];
    for k in 1..n {
        let new_ip = next_addr(&ip, k).ok_or_else(|| msg!("Für die Kopie {} gibt es keine Adresse mehr nach {}.", k + 1, ip.clone()))?;
        if p.devices.iter().any(|d| d.startup_config.contains(&format!("{}/", new_ip)) || d.startup_config.contains(&format!("{} ", new_ip))) {
            return Err(msg!("Die Adresse {} der Kopie {} hat schon ein anderes Gerät.", new_ip, k + 1));
        }
        let name = format!("{}-{}", tpl.name, k + 1);
        let id = p.next_id.max(p.max_id() + 1);
        p.next_id = id + 1;
        let mut d = tpl.clone();
        d.id = id;
        d.name = name.clone();
        d.y += 60 * k as i32;
        d.startup_config = tpl.startup_config.replace(&ip, &new_ip).replace(&format!("hostname {}", tpl.name), &format!("hostname {}", name));
        p.devices.push(d);
        // the next free port of the switch for the copy
        let m = p.device(other.dev).and_then(|d| catalog::model(&d.model)).ok_or_else(|| msg!("Das Modell des Geräts {} gibt es nicht.", other.dev))?;
        let media = m.port(&other.port).map(|x| x.media);
        let free = m.ports().iter().find(|x| Some(x.media) == media && p.link_at(other.dev, &x.name).is_none()).map(|x| x.name.clone());
        let Some(free) = free else {
            return Err(msg!("Am Gerät {} ist für die Kopie {} kein Port mehr frei.", p.device(other.dev).map_or(String::new(), |d| d.name.clone()), k + 1));
        };
        let at = PortRef { dev: id, port: mine.port.clone() };
        let to = PortRef { dev: other.dev, port: free };
        let cable_kind = cable.cable;
        p.connect(&at, &to, Some(cable_kind)).map_err(|e| e.msg())?;
        list.push((name, new_ip, port));
    }
    set_backends(p, a, &list)
}

/// what is wrong with an axis on this project
fn check_axis(a: &Axis, p: &Project) -> Result<(), Msg> {
    let dev_named = |n: &str| -> Result<(), Msg> { if p.find(n).is_some() { Ok(()) } else { Err(msg!("Das Gerät \u{201e}{}\u{201c} gibt es nicht.", n)) } };
    match a.kind.as_str() {
        "hw" | "config" => {
            if a.group.is_empty() && a.dev.is_empty() {
                return Err(msg!("Es fehlt \u{201e}dev\u{201c} oder \u{201e}group\u{201c}."));
            }
            if !a.group.is_empty() && !p.groups.iter().any(|g| g.name.eq_ignore_ascii_case(&a.group)) {
                return Err(msg!("Die Gruppe \u{201e}{}\u{201c} gibt es nicht.", a.group));
            }
            if a.group.is_empty() {
                dev_named(&a.dev)?;
            }
            if targets(a, p).is_empty() {
                return Err(msg!("Die Gruppe \u{201e}{}\u{201c} hat keine Geräte.", a.group));
            }
            if a.values.is_empty() {
                return Err(msg!("\u{201e}values\u{201c} ist leer."));
            }
        }
        "backends" => {}
        k => return Err(msg!("Die Art \u{201e}{}\u{201c} gibt es nicht (hw, config, backends).", k)),
    }
    if a.kind == "hw" {
        for id in targets(a, p) {
            let Some(d) = p.device(id) else { continue };
            for v in &a.values {
                hw_value(v, hw_base(d))?;
            }
        }
    }
    if a.kind == "config" {
        let Some((first, rest)) = a.path.split_once('.') else {
            return Err(msg!("Der Pfad \u{201e}{}\u{201c} nennt einen Dienst und seine Einstellung, etwa http-server.workers.", a.path));
        };
        for id in targets(a, p) {
            let Some(d) = p.device(id) else { continue };
            let names: Vec<&str> = d.services.iter().map(|s| s.name.as_str()).collect();
            let Some(s) = service_of(d, first) else {
                return Err(msg!("{} hat keinen Dienst \u{201e}{}\u{201c} (es gibt: {}).", d.name.clone(), first, names.join(", ")));
            };
            let key = rest.split('.').next().unwrap_or(rest);
            if let Some(known) = known_keys(&s.kind)
                && !known.contains(&key)
            {
                return Err(msg!("Ein Dienst der Art {} kennt \u{201e}{}\u{201c} nicht (es gibt: {}).", s.kind.clone(), key, known.join(", ")));
            }
            // a type that fits what stands there
            for v in &a.values {
                let mut c = s.config.clone();
                set_path(&mut c, rest, v.clone())?;
            }
        }
    }
    if a.kind == "backends" {
        dev_named(&a.dev)?;
        let d = p.find(&a.dev);
        let s = d.and_then(|d| d.services.iter().find(|s| s.name.eq_ignore_ascii_case(&a.svc)));
        match s {
            None => return Err(msg!("{} hat keinen Dienst \u{201e}{}\u{201c}.", a.dev.clone(), a.svc)),
            Some(s) if s.kind != "lb" => return Err(msg!("Der Dienst \u{201e}{}\u{201c} ist kein Load Balancer, sondern {}.", a.svc, s.kind.clone())),
            _ => {}
        }
        if a.counts.is_empty() {
            return Err(msg!("\u{201e}counts\u{201c} ist leer."));
        }
        if a.counts.contains(&0) {
            return Err(msg!("Ein Load Balancer braucht mindestens ein Backend; 0 geht nicht."));
        }
        match (a.pool.is_empty(), a.template.is_empty()) {
            (true, true) => return Err(msg!("Es fehlt \u{201e}pool\u{201c} oder \u{201e}template\u{201c}.")),
            (false, false) => return Err(msg!("\u{201e}pool\u{201c} und \u{201e}template\u{201c} schließen sich aus.")),
            (false, true) => {
                for h in &a.pool {
                    dev_named(h)?;
                }
                let most = a.counts.iter().copied().max().unwrap_or(0) as usize;
                if most > a.pool.len() {
                    return Err(msg!("\u{201e}counts\u{201c} verlangt {} Backends, der Pool hat {}.", most, a.pool.len()));
                }
            }
            (true, false) => dev_named(&a.template)?,
        }
    }
    Ok(())
}

impl Experiment {
    /// the experiment as the text of a .jfnex file (indented JSON, LF line ends)
    pub fn to_text(&self) -> String {
        let mut s = json::stringify_pretty(&self.to_json(), Limits::default(), 2).unwrap_or_default();
        s.push('\n');
        s
    }

    /// reads a .jfnex file (what it asks of a project: problems())
    pub fn from_text(text: &str) -> Result<Experiment, Msg> {
        let v = json::parse(text.as_bytes()).map_err(|e| msg!("Die Datei ist kein gültiges JSON ({} an Byte {}).", e.kind, e.offset))?;
        let x = Experiment::from_json(&v).ok_or_else(|| msg!("Die Datei enthält kein Experiment."))?;
        if x.format != FORMAT {
            return Err(msg!("Das ist keine Experiment-Datei."));
        }
        if x.version == 0 || x.version > VERSION {
            return Err(msg!("Die Datei hat die Version {}; dieses Programm liest Version 1 bis {}.", x.version, VERSION));
        }
        Ok(x)
    }

    /// the load levels
    pub fn levels(&self) -> Vec<Level> {
        if !self.load.rps.is_empty() {
            self.load.rps.iter().map(|r| Level::Rps(*r)).collect()
        } else {
            self.load.users.iter().map(|u| Level::Users(*u)).collect()
        }
    }

    /// the seeds of a run: its own, else the project's
    pub fn seeds_or(&self, project_seed: u64) -> Vec<u64> {
        if self.timing.seeds.is_empty() { alloc::vec![project_seed] } else { self.timing.seeds.clone() }
    }

    /// the configurations: the series, within each the cross product of its axes
    pub fn configs(&self) -> Vec<Config> {
        let plain = [Series::default()];
        let series: &[Series] = if self.series.is_empty() { &plain } else { &self.series };
        let mut out = Vec::new();
        for s in series {
            let axes: Vec<&Axis> = self.axes.iter().chain(s.axes.iter()).collect();
            if axes.iter().any(|a| a.is_empty()) {
                continue;
            }
            let mut idx = alloc::vec![0usize; axes.len()];
            'each: loop {
                out.push(Config { series: s.name.clone(), picks: axes.iter().zip(idx.iter()).map(|(a, i)| ((*a).clone(), *i)).collect() });
                // the last axis fastest
                let mut k = axes.len();
                loop {
                    if k == 0 {
                        break 'each;
                    }
                    k -= 1;
                    idx[k] += 1;
                    if idx[k] < axes[k].len() {
                        break;
                    }
                    idx[k] = 0;
                }
            }
        }
        out
    }

    /// what keeps it from running on this project: every finding says where and what would be right
    pub fn problems(&self, p: &Project) -> Vec<Msg> {
        let mut out = Vec::new();
        // the load
        let lv = &self.load;
        match (lv.rps.is_empty(), lv.users.is_empty()) {
            (true, true) => out.push(msg!("Last: es fehlt \u{201e}rps\u{201c} oder \u{201e}users\u{201c}.")),
            (false, false) => out.push(msg!("Last: \u{201e}rps\u{201c} und \u{201e}users\u{201c} schließen sich aus.")),
            _ => {}
        }
        let levels: Vec<u64> = self.levels().iter().map(|l| l.value()).collect();
        if levels.contains(&0) {
            out.push(msg!("Last: eine Stufe ist 0."));
        }
        if levels.windows(2).any(|w| w[0] >= w[1]) {
            out.push(msg!("Last: die Stufen müssen streng steigen, das Knie gilt sonst nichts."));
        }
        match p.find(&lv.dev) {
            None => out.push(msg!("Last: das Gerät \u{201e}{}\u{201c} gibt es nicht.", lv.dev)),
            Some(d) => match d.services.iter().find(|s| s.name.eq_ignore_ascii_case(&lv.svc)) {
                None => out.push(msg!("Last: {} hat keinen Dienst \u{201e}{}\u{201c}.", d.name.clone(), lv.svc)),
                Some(s) if s.kind != "http-load" => out.push(msg!("Last: der Dienst \u{201e}{}\u{201c} ist kein Lastgenerator (http-load), sondern {}.", lv.svc, s.kind.clone())),
                _ => {}
            },
        }
        // the time
        let t = &self.timing;
        if t.measure_s == 0 {
            out.push(msg!("Zeit: \u{201e}measure_s\u{201c} fehlt."));
        }
        if t.measure_s + t.settle_s > MAX_MEASURE_S {
            out.push(msg!("Zeit: Übergang und Messung zusammen höchstens {} s; so lange halten die Dienste ihre Latenzen.", MAX_MEASURE_S));
        }
        let mut seeds = t.seeds.clone();
        seeds.sort_unstable();
        if seeds.windows(2).any(|w| w[0] == w[1]) {
            out.push(msg!("Zeit: ein Seed steht zweimal da; er ergäbe denselben Lauf."));
        }
        // the limits
        for (name, v, unit) in [("p50_ms", &self.slo.p50_ms, 1_000_000), ("p95_ms", &self.slo.p95_ms, 1_000_000), ("p99_ms", &self.slo.p99_ms, 1_000_000), ("p999_ms", &self.slo.p999_ms, 1_000_000), ("errors_pct", &self.slo.errors_pct, 10_000)] {
            if limit(v, unit).is_err() {
                out.push(msg!("SLO: \u{201e}{}\u{201c} ist keine Zahl.", name));
            }
        }
        // the prices
        for (k, v) in &self.prices {
            if !Prices::KEYS.contains(&k.as_str()) {
                out.push(msg!("Preise: \u{201e}{}\u{201c} gibt es nicht (es gibt: {}).", k, Prices::KEYS.join(", ")));
            } else if decimal_of(v, 1_000_000).is_none() {
                out.push(msg!("Preise: \u{201e}{}\u{201c} ist keine Zahl (Euro je Stunde).", k));
            }
        }
        for n in &self.cost_exclude {
            if p.find(n).is_none() {
                out.push(msg!("Kosten: das Gerät \u{201e}{}\u{201c} gibt es nicht.", n));
            }
        }
        // the axes
        for (i, a) in self.axes.iter().enumerate() {
            if let Err(e) = check_axis(a, p) {
                out.push(msg!("Achse {}: {}", i + 1, e));
            }
        }
        let mut names: Vec<&str> = Vec::new();
        for s in &self.series {
            if s.name.is_empty() {
                out.push(msg!("Eine Reihe hat keinen Namen."));
            }
            if names.contains(&s.name.as_str()) {
                out.push(msg!("Die Reihe \u{201e}{}\u{201c} steht zweimal da.", s.name));
            }
            names.push(&s.name);
            for (i, a) in s.axes.iter().enumerate() {
                if let Err(e) = check_axis(a, p) {
                    out.push(msg!("Reihe {}, Achse {}: {}", s.name, i + 1, e));
                }
            }
        }
        if out.is_empty() {
            let runs = self.configs().len() * levels.len().max(1) * self.timing.seeds.len().max(1);
            if runs > MAX_RUNS {
                out.push(msg!("Das wären {} Läufe; höchstens {} gehen.", runs, MAX_RUNS));
            }
        }
        out
    }

    /// the prices of a run: the assumed ones and what it says instead
    pub fn prices_of(&self) -> Prices {
        let mut pr = Prices::ASSUMED;
        for (k, v) in &self.prices {
            if let Some(x) = decimal_of(v, 1_000_000) {
                pr.set(k, x);
            }
        }
        pr
    }
}
