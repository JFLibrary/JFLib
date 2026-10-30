// linux_flatpak -- (pkg.rs) the updates of a Flatpak app through org.freedesktop.portal.Flatpak
//
//   FlatpakMonitor::create()   CreateUpdateMonitor({handle_token}) on the session bus; the
//                              handle /org/freedesktop/portal/Flatpak/update_monitor/SENDER/TOKEN
//                              is known before the call, so the signals are matched first
//   poll(ms)                   the newest UpdateAvailable({running-commit, local-commit,
//                              remote-commit}) that arrived within ms
//   update(progress, cancel)   Update("", {}) and its Progress signals ({n_ops, op, progress,
//                              status: 0 running, 1 nothing to do, 2 done, 3 failed; error,
//                              error_message}) until the end
//   drop                       Close()
//   spawn_latest(args)         Spawn(cwd, argv, {}, {}, LATEST_VERSION, {}): the app's newest
//                              installed version with the same program and these arguments
//
// The portal (flatpak-portal, outside the sandbox) checks the app's remote every few minutes
// (its --poll-timeout, 300 s by default) while a monitor exists; Update only installs a
// version that asks for no more permissions than the running one.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use super::FlatpakUpdate;
use crate::linux_dbus::{Conn, Msg, T_ERROR, T_RETURN, T_SIGNAL, Value};

const PORTAL: &str = "org.freedesktop.portal.Flatpak";
const PORTAL_PATH: &str = "/org/freedesktop/portal/Flatpak";
const MONITOR: &str = "org.freedesktop.portal.Flatpak.UpdateMonitor";
const CALL_MS: i32 = 20_000;
// Spawn flags
const SPAWN_LATEST_VERSION: u32 = 2;
// Progress status
const PS_RUNNING: u32 = 0;
const PS_EMPTY: u32 = 1;
const PS_DONE: u32 = 2;
static TOKEN: AtomicU32 = AtomicU32::new(0);

/// the value of key in an a{sv}
fn field<'a>(dict: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    let Some(Value::A(_, kv)) = dict else { return None };
    kv.iter().find_map(|e| match e {
        Value::E(k, v) if k.as_str() == Some(key) => Some(match &**v {
            Value::V(inner) => &**inner,
            other => other,
        }),
        _ => None,
    })
}

fn text(dict: Option<&Value>, key: &str) -> String {
    field(dict, key).and_then(Value::as_str).map(String::from).unwrap_or_default()
}

fn number(dict: Option<&Value>, key: &str) -> Option<u32> {
    field(dict, key).and_then(Value::as_u32)
}

// an error answer -> its text (the bus name and the message)
fn error_text(m: &Msg) -> String {
    let msg = m.body.first().and_then(Value::as_str).unwrap_or("");
    if msg.is_empty() { m.error.clone() } else { format!("{}: {}", m.error, msg) }
}

// call and wait for its answer (signals meanwhile go to `signal`) -> the return or the error text
fn call(c: &mut Conn, m: &Msg, signal: &mut dyn FnMut(&Msg)) -> Result<Msg, String> {
    let serial = c.send(m).ok_or_else(|| String::from("the session bus closed the connection"))?;
    let end = crate::time_us() + CALL_MS as u64 * 1000;
    loop {
        let now = crate::time_us();
        if now >= end {
            return Err(format!("{} did not answer {}", PORTAL, m.member));
        }
        let Some(r) = c.read(((end - now) / 1000) as i32 + 1) else {
            if c.broken {
                return Err(String::from("the session bus closed the connection"));
            }
            continue;
        };
        match r.typ {
            T_RETURN if r.reply_to == serial => return Ok(r),
            T_ERROR if r.reply_to == serial => return Err(error_text(&r)),
            T_SIGNAL => signal(&r),
            _ => {}
        }
    }
}

/// the update monitor of this app (pkg::FlatpakMonitor)
pub struct FlatpakMonitor {
    c: Conn,
    handle: String,
    // an UpdateAvailable that arrived while waiting for something else
    seen: Option<FlatpakUpdate>,
}

impl FlatpakMonitor {
    pub fn create() -> Result<FlatpakMonitor, String> {
        let mut c = Conn::session().ok_or_else(|| String::from("no session bus"))?;
        let token = format!("jfgui_upd{}", TOKEN.fetch_add(1, Ordering::Relaxed) + 1);
        let sender = c.unique.trim_start_matches(':').replace('.', "_");
        let handle = format!("{PORTAL_PATH}/update_monitor/{sender}/{token}");
        let rule = format!("type='signal',interface='{MONITOR}',path='{handle}'");
        c.send(&Msg::call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "AddMatch", alloc::vec![Value::S(rule)]));
        let m = Msg::call(PORTAL, PORTAL_PATH, PORTAL, "CreateUpdateMonitor", alloc::vec![Value::dict(alloc::vec![("handle_token", Value::S(token))])]);
        let mut early = None;
        let r = call(&mut c, &m, &mut |s| {
            if s.member == "UpdateAvailable" && s.path == handle {
                early = Some(update_of(s));
            }
        })?;
        let got = r.body.first().and_then(Value::as_str).unwrap_or("");
        if got != handle {
            return Err(format!("CreateUpdateMonitor answered {got}, not {handle}"));
        }
        Ok(FlatpakMonitor { c, handle, seen: early })
    }

    /// the newest UpdateAvailable within ms (0: only what has arrived)
    pub fn poll(&mut self, ms: i32) -> Option<FlatpakUpdate> {
        let end = crate::time_us() + ms.max(0) as u64 * 1000;
        let mut got = self.seen.take();
        loop {
            let now = crate::time_us();
            let left = if now >= end { 0 } else { ((end - now) / 1000) as i32 + 1 };
            let Some(m) = self.c.read(left) else { return got };
            if m.typ == T_SIGNAL && m.member == "UpdateAvailable" && m.path == self.handle {
                got = Some(update_of(&m));
            }
        }
    }

    /// install the update: progress(op, n_ops, percent) for every Progress signal, cancel()
    /// asked between them (the portal's transaction itself cannot be stopped: Err after it)
    pub fn update(&mut self, progress: &mut dyn FnMut(u32, u32, u32), cancel: &dyn Fn() -> bool) -> Result<(), String> {
        let handle = self.handle.clone();
        let mut end_state: Option<Result<(), String>> = None;
        let mut on_signal = |s: &Msg, seen: &mut Option<FlatpakUpdate>, end_state: &mut Option<Result<(), String>>| {
            if s.path != handle {
                return;
            }
            if s.member == "UpdateAvailable" {
                *seen = Some(update_of(s));
                return;
            }
            if s.member != "Progress" {
                return;
            }
            let d = s.body.first();
            let status = number(d, "status").unwrap_or(PS_RUNNING);
            progress(number(d, "op").unwrap_or(0), number(d, "n_ops").unwrap_or(0), number(d, "progress").unwrap_or(0).min(100));
            match status {
                PS_EMPTY | PS_DONE => *end_state = Some(Ok(())),
                PS_RUNNING => {}
                _ => {
                    let msg = text(d, "error_message");
                    *end_state = Some(Err(if msg.is_empty() { text(d, "error") } else { msg }));
                }
            }
        };
        let m = Msg::call(PORTAL, &handle, MONITOR, "Update", alloc::vec![Value::str(""), Value::dict(Vec::new())]);
        let mut seen = self.seen.take();
        call(&mut self.c, &m, &mut |s| on_signal(s, &mut seen, &mut end_state))?;
        loop {
            if let Some(r) = end_state.take() {
                self.seen = seen;
                return r;
            }
            if cancel() {
                self.seen = seen;
                return Err(String::from("cancelled"));
            }
            match self.c.read(500) {
                Some(s) if s.typ == T_SIGNAL => on_signal(&s, &mut seen, &mut end_state),
                Some(_) => {}
                None if self.c.broken => return Err(String::from("the session bus closed the connection")),
                None => {}
            }
        }
    }
}

impl Drop for FlatpakMonitor {
    fn drop(&mut self) {
        let m = Msg { flags: crate::linux_dbus::F_NO_REPLY, ..Msg::call(PORTAL, &self.handle, MONITOR, "Close", Vec::new()) };
        self.c.send(&m);
    }
}

fn update_of(m: &Msg) -> FlatpakUpdate {
    let d = m.body.first();
    FlatpakUpdate { running: text(d, "running-commit"), local: text(d, "local-commit"), remote: text(d, "remote-commit") }
}

// a bytestring ("ay" with its NUL)
fn bytes(s: &str) -> Value {
    Value::A(String::from("y"), s.bytes().chain(core::iter::once(0)).map(Value::Y).collect())
}

/// Spawn(cwd, [program, args..], {}, {}, LATEST_VERSION, {}) -> the new process's pid
pub fn spawn_latest(args: &[&str]) -> Result<u32, String> {
    let mut c = Conn::session().ok_or_else(|| String::from("no session bus"))?;
    let program = crate::current_exe().ok_or_else(|| String::from("no path of the program"))?;
    let program = String::from(program.strip_suffix(" (deleted)").unwrap_or(&program));
    let cwd = Some(crate::cwd()).filter(|c| c.starts_with('/')).unwrap_or_else(|| String::from("/"));
    let argv: Vec<Value> = core::iter::once(program.as_str()).chain(args.iter().copied()).map(bytes).collect();
    let m = Msg::call(PORTAL, PORTAL_PATH, PORTAL, "Spawn", alloc::vec![
        bytes(&cwd),
        Value::A(String::from("ay"), argv),
        Value::A(String::from("{uh}"), Vec::new()),
        Value::A(String::from("{ss}"), Vec::new()),
        Value::U(SPAWN_LATEST_VERSION),
        Value::dict(Vec::new()),
    ]);
    let r = call(&mut c, &m, &mut |_| {})?;
    r.body.first().and_then(Value::as_u32).ok_or_else(|| String::from("Spawn answered no pid"))
}
