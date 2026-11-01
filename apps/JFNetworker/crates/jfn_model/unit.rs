// unit -- the unit files of a Linux host (A2): the INI subset of /etc/systemd/system/NAME.service
// the virtual hosts understand, and the names and places around them.  Only the text: what a
// unit does at run time is jfn_sim's (units.rs)
//
//   parse(text) -> Unit             never fails: what does not fit is an Issue (Unit::issues), an
//                                   error keeps the unit from starting (Unit::bad)
//   Unit::service_def(name, file)   the service a unit's ExecStart makes (a jfn_model ServiceDef);
//                                   `file`: the text of its settings file; the refusal as a Msg
//   unit_name("web") -> "web.service", stem("web.service") -> "web", path_of(name), name_ok(name)
//   link_path(target, name)         the file that says a unit is enabled for a target
//   UNIT_DIR                        where unit files live
//
// The format, one setting per line, "#" and ";" start a comment, "\" at the end continues a line:
//
//   [Unit]
//   Description=Die Bestell-API
//   After=network.target kv.service       (order among the units a boot starts)
//   Requires=kv.service                   (starts it first; stops with it)
//   StartLimitBurst=5                     (starts in the interval before it gives up, 0: no limit)
//   StartLimitIntervalSec=10
//
//   [Service]
//   ExecStart=jfs /root/app/server.jfs --port 8080
//   Environment=MODE=test "NAME=a b"      (config.env of the script, several lines add up)
//   Restart=on-failure                    (no | on-failure | always)
//   RestartSec=2                          (5, 2s, 500ms, 1min; default 100 ms)
//   Settings={"ns_per_op": 5}             (JSON: the rest of the service's definition)
//
//   [Install]
//   WantedBy=multi-user.target            (what "systemctl enable" links it to)
//
// ExecStart names one of two programs: "jfs FILE [ARGS ...]" runs a script of the device's files
// (a script service with config {"file": FILE, "args": [ARGS], "env": {...}, "unit": NAME}), and
// "jfn-service KIND [FILE]" runs a built-in service (the kinds of a project's service definitions:
// http-server, lb, kv, cache ...; svcact.rs lists them) with its JSON settings in FILE or in
// Settings=.  Paths are absolute.  Targets in After, Requires and WantedBy mean nothing but
// themselves: a boot starts what any "*.wants" folder holds.  An empty value ("After=") empties a
// list, as in the original format.  Unknown sections and settings are warnings and count for
// nothing, a missing or unusable ExecStart is an error.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;

use crate::msg;
use crate::msg::Msg;
use crate::project::ServiceDef;

/// where unit files live
pub const UNIT_DIR: &str = "/etc/systemd/system";
/// the time before a restart when the file names none (ns)
pub const RESTART_DEFAULT: u64 = 100_000_000;
/// the start limit when the file names none: starts within the interval (ns)
pub const LIMIT_BURST: u32 = 5;
pub const LIMIT_INTERVAL: u64 = 10_000_000_000;

/// Restart=
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Restart {
    #[default]
    No,
    OnFailure,
    Always,
}

impl Restart {
    /// the word in a unit file
    pub fn as_str(self) -> &'static str {
        match self {
            Restart::No => "no",
            Restart::OnFailure => "on-failure",
            Restart::Always => "always",
        }
    }
}

/// what ExecStart runs
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Exec {
    /// no usable ExecStart
    #[default]
    None,
    /// "jfs FILE ARGS ...": a script of the device's files
    Jfs { file: String, args: Vec<String> },
    /// "jfn-service KIND [FILE]": a built-in service, its settings in FILE or in Settings=
    Builtin { kind: String, file: Option<String> },
}

/// something a unit file gets wrong
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// from 1 (0: the file as a whole)
    pub line: u32,
    /// it cannot start (else a warning: the line counts for nothing)
    pub error: bool,
    pub msg: Msg,
}

/// a unit file as it was read
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unit {
    pub description: String,
    /// units it starts after in a boot ("kv.service")
    pub after: Vec<String>,
    /// units it needs: they start first, and it stops with them
    pub requires: Vec<String>,
    pub exec: Exec,
    /// ExecStart as written
    pub exec_text: String,
    pub env: Vec<(String, String)>,
    pub restart: Restart,
    /// RestartSec in ns
    pub restart_ns: u64,
    /// Settings= as written (JSON)
    pub settings: String,
    /// targets of [Install] WantedBy
    pub wanted_by: Vec<String>,
    pub limit_burst: u32,
    pub limit_ns: u64,
    pub issues: Vec<Issue>,
}

impl Unit {
    /// it cannot start
    pub fn bad(&self) -> bool {
        self.issues.iter().any(|i| i.error)
    }

    /// the first error (for messages)
    pub fn first_error(&self) -> Option<&Issue> {
        self.issues.iter().find(|i| i.error)
    }

    /// the service this unit runs under name `name`; `file`: the text of the settings file of
    /// "jfn-service KIND FILE" (read by the caller)
    pub fn service_def(&self, name: &str, file: Option<&str>) -> Result<ServiceDef, Msg> {
        let settings = |text: &str| -> Result<Value, Msg> {
            match agent_core::json::parse(text.as_bytes()) {
                Ok(v) if v.as_object().is_some() => Ok(v),
                Ok(_) => Err(msg!("Die Einstellungen sind kein JSON-Objekt.")),
                Err(_) => Err(msg!("Die Einstellungen sind kein gültiges JSON.")),
            }
        };
        let mut config = agent_core::jmap::empty_object();
        if let Some(text) = file {
            config = settings(text)?;
        }
        if !self.settings.is_empty() {
            let more = settings(&self.settings)?;
            for (k, v) in more.as_object().unwrap_or(&[]) {
                config.insert(k, v.clone());
            }
        }
        let kind = match &self.exec {
            Exec::None => return Err(msg!("[Service] braucht ExecStart.")),
            Exec::Jfs { file, args } => {
                config.insert("file", Value::string(file));
                config.insert("args", Value::Array(args.iter().map(|a| Value::string(a)).collect()));
                let mut env = agent_core::jmap::empty_object();
                for (k, v) in &self.env {
                    env.insert(k, Value::string(v));
                }
                config.insert("env", env);
                config.insert("unit", Value::string(&unit_name(name)));
                String::from("script")
            }
            Exec::Builtin { kind, .. } => kind.clone(),
        };
        Ok(ServiceDef { kind, name: String::from(stem(name)), disabled: false, config })
    }
}

/// a unit's full name: "web" -> "web.service"; a target stays as it is
pub fn unit_name(s: &str) -> String {
    if s.ends_with(".service") || s.ends_with(".target") { String::from(s) } else { format!("{}.service", s) }
}

/// the name without ".service"
pub fn stem(name: &str) -> &str {
    name.strip_suffix(".service").unwrap_or(name)
}

/// the file of unit `name`
pub fn path_of(name: &str) -> String {
    format!("{}/{}", UNIT_DIR, unit_name(name))
}

/// the file that says unit `name` is enabled for `target`
pub fn link_path(target: &str, name: &str) -> String {
    format!("{}/{}.wants/{}", UNIT_DIR, target, unit_name(name))
}

/// a name a unit may have: letters, digits, "-", "_", ".", ":" and "@", not empty
pub fn name_ok(name: &str) -> bool {
    let s = stem(name);
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '@'))
}

/// "5", "2s", "500ms", "1min", "1h" -> ns
fn seconds(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        return s.parse::<u64>().ok()?.checked_mul(1_000_000_000);
    }
    crate::scenario::parse_duration(s)
}

/// the words of a value: spaces separate, "..." and '...' hold words together
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote = None;
    let mut any = false;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                any = true;
            }
            None if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(core::mem::take(&mut cur));
                }
                any = false;
            }
            None => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// the service names of a list (targets mean nothing here)
fn services(v: &str) -> Vec<String> {
    v.split_whitespace().filter(|w| !w.ends_with(".target")).map(unit_name).collect()
}

/// reads a unit file
pub fn parse(text: &str) -> Unit {
    let mut u = Unit { restart_ns: RESTART_DEFAULT, limit_burst: LIMIT_BURST, limit_ns: LIMIT_INTERVAL, ..Unit::default() };
    let mut section = String::new();
    let mut seen_exec = false;
    let mut it = text.lines().enumerate();
    while let Some((i, raw)) = it.next() {
        let line = (i + 1) as u32;
        let mut l = String::from(raw.trim());
        if l.starts_with('#') || l.starts_with(';') || l.is_empty() {
            continue;
        }
        // a backslash at the end continues the line
        while l.ends_with('\\') {
            l.pop();
            match it.next() {
                Some((_, next)) => {
                    l.push(' ');
                    l.push_str(next.trim());
                }
                None => break,
            }
        }
        if l.starts_with('[') {
            let Some(name) = l.strip_prefix('[').and_then(|x| x.strip_suffix(']')) else {
                u.issues.push(Issue { line, error: false, msg: msg!("Zeile {}: {} ist kein Abschnitt.", line, l.clone()) });
                continue;
            };
            section = String::from(name);
            if !matches!(name, "Unit" | "Service" | "Install") {
                u.issues.push(Issue { line, error: false, msg: msg!("Zeile {}: den Abschnitt [{}] kenne ich nicht, er zählt nicht.", line, name) });
            }
            continue;
        }
        let Some((k, v)) = l.split_once('=') else {
            u.issues.push(Issue { line, error: false, msg: msg!("Zeile {}: ohne \u{201e}=\u{201c} zählt die Zeile nicht.", line) });
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        match (section.as_str(), k) {
            ("Unit", "Description") => u.description = String::from(v),
            ("Unit", "Documentation") => {}
            ("Unit", "After") if v.is_empty() => u.after.clear(),
            ("Unit", "After") => u.after.extend(services(v)),
            ("Unit", "Requires" | "Wants") if v.is_empty() => u.requires.clear(),
            ("Unit", "Requires" | "Wants") => {
                let list = services(v);
                // a unit it needs also starts before it
                u.after.extend(list.iter().cloned());
                u.requires.extend(list);
            }
            ("Unit", "StartLimitBurst") => match v.parse::<u32>() {
                Ok(n) => u.limit_burst = n,
                Err(_) => u.issues.push(Issue { line, error: false, msg: msg!("Zeile {}: {} ist keine Zahl, es bleibt bei {}.", line, String::from(v), u.limit_burst) }),
            },
            ("Unit", "StartLimitIntervalSec") => match seconds(v) {
                Some(n) => u.limit_ns = n,
                None => u.issues.push(Issue { line, error: false, msg: msg!("Zeile {}: {} ist keine Zeit.", line, String::from(v)) }),
            },
            ("Service", "ExecStart") => {
                seen_exec = true;
                if v.is_empty() {
                    u.exec = Exec::None;
                    u.exec_text.clear();
                    seen_exec = false;
                    continue;
                }
                u.exec_text = String::from(v);
                let w = words(v);
                let args = &w[1..];
                match w.first().map(String::as_str) {
                    Some("jfs" | "/usr/bin/jfs") => match args.first() {
                        Some(f) if f.starts_with('/') => u.exec = Exec::Jfs { file: f.clone(), args: args[1..].to_vec() },
                        Some(f) => u.issues.push(Issue { line, error: true, msg: msg!("Zeile {}: ExecStart braucht einen absoluten Pfad, nicht {}.", line, f.clone()) }),
                        None => u.issues.push(Issue { line, error: true, msg: msg!("Zeile {}: jfs braucht eine Datei.", line) }),
                    },
                    Some("jfn-service" | "/usr/bin/jfn-service") => match args {
                        [kind] => u.exec = Exec::Builtin { kind: kind.clone(), file: None },
                        [kind, f] if f.starts_with('/') => u.exec = Exec::Builtin { kind: kind.clone(), file: Some(f.clone()) },
                        [_, f] => u.issues.push(Issue { line, error: true, msg: msg!("Zeile {}: ExecStart braucht einen absoluten Pfad, nicht {}.", line, f.clone()) }),
                        _ => u.issues.push(Issue { line, error: true, msg: msg!("Zeile {}: jfn-service braucht die Art des Dienstes, dahinter höchstens eine Datei mit seinen Einstellungen.", line) }),
                    },
                    Some(p) => u.issues.push(Issue { line, error: true, msg: msg!("Zeile {}: ExecStart startet hier nur jfs oder jfn-service, nicht {}.", line, String::from(p)) }),
                    None => {}
                }
            }
            ("Service", "Type") => {
                if v != "simple" {
                    u.issues.push(Issue { line, error: false, msg: msg!("Zeile {}: Type={} gibt es hier nicht, der Dienst läuft als simple.", line, String::from(v)) });
                }
            }
            ("Service", "Environment") if v.is_empty() => u.env.clear(),
            ("Service", "Environment") => {
                for w in words(v) {
                    match w.split_once('=') {
                        Some((a, b)) if !a.is_empty() => {
                            u.env.retain(|x| x.0 != a);
                            u.env.push((String::from(a), String::from(b)));
                        }
                        _ => u.issues.push(Issue { line, error: false, msg: msg!("Zeile {}: {} ist nicht NAME=WERT.", line, w.clone()) }),
                    }
                }
            }
            ("Service", "Restart") => match v {
                "no" => u.restart = Restart::No,
                "on-failure" => u.restart = Restart::OnFailure,
                "always" => u.restart = Restart::Always,
                _ => u.issues.push(Issue { line, error: false, msg: msg!("Zeile {}: Restart={} gibt es hier nicht (no, on-failure, always).", line, String::from(v)) }),
            },
            ("Service", "RestartSec") => match seconds(v) {
                Some(n) => u.restart_ns = n,
                None => u.issues.push(Issue { line, error: false, msg: msg!("Zeile {}: {} ist keine Zeit.", line, String::from(v)) }),
            },
            ("Service", "Settings") => u.settings = String::from(v),
            ("Install", "WantedBy") if v.is_empty() => u.wanted_by.clear(),
            ("Install", "WantedBy") => u.wanted_by.extend(v.split_whitespace().map(String::from)),
            ("", _) => u.issues.push(Issue { line, error: false, msg: msg!("Zeile {}: {} steht vor jedem Abschnitt und zählt nicht.", line, String::from(k)) }),
            ("Unit" | "Service" | "Install", _) => u.issues.push(Issue { line, error: false, msg: msg!("Zeile {}: {} gibt es hier nicht, die Zeile zählt nicht.", line, String::from(k)) }),
            _ => {}
        }
    }
    if !seen_exec && !u.issues.iter().any(|i| i.error) {
        u.issues.push(Issue { line: 0, error: true, msg: msg!("[Service] braucht ExecStart.") });
    }
    u
}

// ---------------------------------------------------------------- deployments

/// how a deployment checks a host after restarting its unit
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Health {
    /// the unit is active
    #[default]
    Active,
    /// a TCP connection to this port opens
    Tcp { port: u16 },
    /// a GET of `path` on `port` answers `status`
    Http { port: u16, path: String, status: u16 },
}

/// what a deployment does when a host fails
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OnFail {
    /// it stops: the hosts done so far keep the new files
    #[default]
    Stop,
    /// it stops and every host it changed gets its old files back
    Rollback,
}

/// a rolling deployment (jfn_sim deploy.rs runs it)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploySpec {
    /// the host the files come from
    pub src: u32,
    /// the hosts, in order
    pub hosts: Vec<u32>,
    /// files and folders of the source, absolute
    pub files: Vec<String>,
    /// the unit to install, enable and restart
    pub unit: String,
    pub health: Health,
    /// after a host's check held, before the next host (ns)
    pub wait: u64,
    /// how long a host's check may take (ns)
    pub timeout: u64,
    pub on_fail: OnFail,
}
