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
