// linux_desk -- what the desktop integration on Linux shares (linux_power.rs, linux_taskbar.rs,
// linux_autostart.rs)
//
//   Link                      a session-bus connection of one module, made on first use and
//                             kept (the desktop ties inhibitions and the dock's badge to it);
//                             Link::with(f) runs f with it, Link::close_if(cond) ends it
//   call(c, msg, ms) -> Option<Msg>   send a call and wait for its answer (None: error, no
//                             answer in ms, broken connection)
//   drain(c, f)               the messages that arrived meanwhile; calls to us are answered
//                             with f(msg) (None: UnknownMethod), Ping always
//   desktop_id() -> String    the name of the app's .desktop file (without ".desktop"):
//                             jfgui_DESKTOP_ID, $FLATPAK_ID, the file GLib launched this
//                             process from ($GIO_LAUNCHED_DESKTOP_FILE when its PID is ours),
//                             the app id (vfs::app_id) or its lower case when an installed
//                             .desktop file has that name (the .deb's is lower case), else
//                             the app id
//   in_flatpak() -> bool      /.flatpak-info exists

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::linux_dbus::{Conn, F_NO_REPLY, Msg, T_CALL, Value};
use crate::sync::Mutex;

/// a session-bus connection kept between calls
pub(crate) struct Link(Mutex<Option<Conn>>);

impl Link {
    pub(crate) const fn new() -> Link {
        Link(Mutex::new(None))
    }

    /// f with the connection (connected first when needed) -> None without a session bus
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut Conn) -> R) -> Option<R> {
        let mut g = self.0.lock();
        if g.as_ref().is_some_and(|c| c.broken) {
            *g = None;
        }
        if g.is_none() {
            *g = Conn::session();
        }
        Some(f(g.as_mut()?))
    }

    /// close the connection when cond() holds (checked under the lock)
    pub(crate) fn close_if(&self, cond: impl FnOnce() -> bool) {
        let mut g = self.0.lock();
        if cond() {
            *g = None;
        }
    }
}

/// send m and wait up to ms for its answer (None: an error answer, none, a broken connection)
pub(crate) fn call(c: &mut Conn, m: &Msg, ms: i32) -> Option<Msg> {
    let serial = c.send(m)?;
    c.wait_reply(serial, ms)
}

/// m as a call that wants no answer
pub(crate) fn no_reply(mut m: Msg) -> Msg {
    m.flags |= F_NO_REPLY;
    m
}

/// read what arrived; calls to us are answered: Peer.Ping, else f(call) (None: UnknownMethod)
pub(crate) fn drain(c: &mut Conn, f: impl Fn(&Msg) -> Option<Vec<Value>>) {
    while let Some(m) = c.read(0) {
        if m.typ != T_CALL || m.flags & F_NO_REPLY != 0 {
            continue;
        }
        let body = if m.iface == "org.freedesktop.DBus.Peer" && m.member == "Ping" { Some(Vec::new()) } else { f(&m) };
        c.send(&match body {
            Some(b) => Msg::reply(&m, b),
            None => Msg::error(&m, "org.freedesktop.DBus.Error.UnknownMethod", "unknown method"),
        });
    }
}

pub(crate) const PORTAL: &str = "org.freedesktop.portal.Desktop";
pub(crate) const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
static TOKEN: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// why a portal request has no answer
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PortalError {
    /// no portal, no such interface or method
    Missing,
    /// an error answer (its name), a broken connection ("")
    Failed(String),
    /// no Response in time
    Timeout,
}

/// a fresh handle token "jfgui_<what><n>"
pub(crate) fn token(what: &str) -> String {
    alloc::format!("jfgui_{what}{}", TOKEN.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1)
}

/// the a{sv} of a message body as (key, value) pairs, the variants unwrapped
pub(crate) fn results(v: Option<&Value>) -> Vec<(String, Value)> {
    match v {
        Some(Value::A(_, kv)) => kv
            .iter()
            .filter_map(|e| match e {
                Value::E(k, v) => {
                    let v = match &**v {
                        Value::V(inner) => (**inner).clone(),
                        other => other.clone(),
                    };
                    Some((String::from(k.as_str()?), v))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// the value of key in results
pub(crate) fn result<'a>(r: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    r.iter().find(|e| e.0 == key).map(|e| &e.1)
}

/// a portal request: iface.member(args.., options with handle_token = token(what)) on the
/// portal, then its Request's Response, waited for up to ms -> (code: 0 granted, 1 cancelled,
/// 2 other, results).  Other messages meanwhile go to other(msg).
pub(crate) fn portal_request(
    c: &mut Conn,
    iface: &str,
    member: &str,
    mut args: Vec<Value>,
    mut opts: Vec<(&str, Value)>,
    what: &str,
    ms: u64,
    other: &mut dyn FnMut(&Msg),
) -> Result<(u32, Vec<(String, Value)>), PortalError> {
    let tok = token(what);
    let sender = c.unique.trim_start_matches(':').replace('.', "_");
    let mut handle = alloc::format!("{PORTAL_PATH}/request/{sender}/{tok}");
    let rule = alloc::format!("type='signal',interface='org.freedesktop.portal.Request',member='Response',path='{handle}'");
    c.send(&Msg::call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "AddMatch", alloc::vec![Value::S(rule)]));
    opts.insert(0, ("handle_token", Value::S(tok)));
    args.push(Value::dict(opts));
    let serial = c.send(&Msg::call(PORTAL, PORTAL_PATH, iface, member, args)).ok_or(PortalError::Failed(String::new()))?;
    let end = crate::time_us() + ms * 1000;
    loop {
        let now = crate::time_us();
        if now >= end {
            return Err(PortalError::Timeout);
        }
        let Some(m) = c.read((((end - now) / 1000) as i32 + 1).min(250)) else {
            if c.broken {
                return Err(PortalError::Failed(String::new()));
            }
            continue;
        };
        match m.typ {
            crate::linux_dbus::T_ERROR if m.reply_to == serial => {
                let missing = m.error.ends_with(".ServiceUnknown") || m.error.ends_with(".UnknownMethod") || m.error.ends_with(".UnknownInterface");
                return Err(if missing { PortalError::Missing } else { PortalError::Failed(m.error) });
            }
            crate::linux_dbus::T_RETURN if m.reply_to == serial => {
                if let Some(p) = m.body.first().and_then(|v| v.as_str()) {
                    handle = String::from(p); // older portals choose the path themselves
                }
            }
            crate::linux_dbus::T_SIGNAL if m.member == "Response" && m.path == handle => {
                let code = m.body.first().and_then(|v| v.as_u32()).unwrap_or(2);
                return Ok((code, results(m.body.get(1))));
            }
            _ => other(&m),
        }
    }
}

/// the portal's version of iface (Properties.Get "version"; 0 = no such portal), None when the
/// portal did not answer (still starting: ask again later)
pub(crate) fn portal_version_answer(c: &mut Conn, iface: &str) -> Option<u32> {
    let m = Msg::call(PORTAL, PORTAL_PATH, "org.freedesktop.DBus.Properties", "Get", alloc::vec![Value::str(iface), Value::str("version")]);
    let r = call(c, &m, 3000)?;
    Some(r.body.first().and_then(|v| v.as_u32()).unwrap_or(0))
}

/// a portal's version, asked once -- a portal that did not answer (it may still be starting
/// with the session) is asked again after 30 s; cell: version + 1 (0 = not known), retry: when
/// to ask again (time_us)
pub(crate) fn portal_version_cached(cell: &AtomicU32, retry: &AtomicU64, iface: &str) -> u32 {
    let v = cell.load(Ordering::Relaxed);
    if v != 0 {
        return v - 1;
    }
    let now = crate::time_us();
    if now < retry.load(Ordering::Relaxed) {
        return 0;
    }
    match Conn::session().map(|mut c| portal_version_answer(&mut c, iface)) {
        Some(Some(v)) => {
            cell.store(v + 1, Ordering::Relaxed);
            v
        }
        Some(None) => {
            retry.store(now + 30_000_000, Ordering::Relaxed);
            0
        }
        None => {
            cell.store(1, Ordering::Relaxed); // no session bus: no portal
            0
        }
    }
}

/// whether this process runs in a Flatpak sandbox
pub(crate) fn in_flatpak() -> bool {
    crate::file_exists_raw("/.flatpak-info") == 1
}

// a usable desktop id: not empty, no '/', no ".desktop" at the end
fn clean(s: &str) -> Option<String> {
    let s = s.strip_suffix(".desktop").unwrap_or(s);
    (!s.is_empty() && !s.contains('/') && !s.contains('\0')).then(|| String::from(s))
}

// the folders that hold installed .desktop files: $XDG_DATA_HOME, then $XDG_DATA_DIRS
fn app_dirs() -> Vec<String> {
    let mut v = Vec::new();
    if let Some(d) = crate::data_dir() {
        v.push(alloc::format!("{d}/applications"));
    }
    let dirs = crate::env("XDG_DATA_DIRS").filter(|s| !s.is_empty()).unwrap_or_else(|| String::from("/usr/local/share:/usr/share"));
    for d in dirs.split(':').filter(|d| d.starts_with('/')) {
        v.push(alloc::format!("{}/applications", d.trim_end_matches('/')));
    }
    v
}

/// the name of the app's .desktop file without ".desktop"
pub(crate) fn desktop_id() -> String {
    if let Some(id) = crate::env("jfgui_DESKTOP_ID").as_deref().and_then(clean) {
        return id;
    }
    if let Some(id) = crate::env("FLATPAK_ID").as_deref().and_then(clean) {
        return id;
    }
    // set by GLib for the process it launched (children inherit it: the PID tells them apart)
    if let (Some(file), Some(pid)) = (crate::env("GIO_LAUNCHED_DESKTOP_FILE"), crate::env("GIO_LAUNCHED_DESKTOP_FILE_PID")) {
        // SAFETY: getpid has no arguments.
        let me = unsafe { rt::os::syscall6(39, 0, 0, 0, 0, 0, 0) };
        if pid.trim().parse::<isize>().ok() == Some(me) {
            if let Some(id) = clean(&file[file.rfind('/').map_or(0, |k| k + 1)..]) {
                return id;
            }
        }
    }
    let app = crate::vfs::app_id();
    let lower = app.to_ascii_lowercase();
    let dirs = app_dirs();
    for cand in [&app, &lower] {
        if dirs.iter().any(|d| crate::file_exists_raw(&alloc::format!("{d}/{cand}.desktop")) == 1) {
            return cand.clone();
        }
    }
    app
}
