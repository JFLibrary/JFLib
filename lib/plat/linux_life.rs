// linux_life -- lifecycle events on Linux
//
//   life_start()     (the first window) the SIGTERM handler and the watcher thread, once
//   life_poll()      (wait) a SIGTERM that arrived -> E_SESSION_END
//
// SIGTERM (logout, shutdown, `kill`) sets a flag and wakes the loop: the app gets
// E_SESSION_END (UIC_SESSION_END), saves, and ends.  The watcher thread has connections of
// its own (linux_dbus.rs): logind's PrepareForSleep(b) on the system bus -> E_POWER (1
// sleep, 0 woke up); the settings portal's SettingChanged(namespace, key, value) on the
// session bus -> E_SETTINGS with SET_THEME (color-scheme, accent-color, gtk-theme),
// SET_CONTRAST (contrast, high-contrast), SET_FONTS (font names, text-scaling-factor),
// SET_LOCALE (region), SET_OTHER (org.gnome.desktop.wm.preferences button-layout,
// action-*-titlebar: the library's title bar); other keys are ignored.  The settings files
// without the portal (linux_settings.rs: GTK's gtk-4.0 / gtk-3.0 settings.ini, KDE's kdeglobals)
// through inotify on $XDG_CONFIG_HOME and its gtk-4.0 / gtk-3.0 folders (not recursive; the
// folders are watched once created): a write, rename or removal of one of those files ->
// E_SETTINGS SET_THEME | SET_OTHER, once per batch of events, and only while the last read of
// the settings used the files (the portal said everything: its signals are enough).  Without
// a bus and without inotify the thread ends quietly; jfgui_LIFE=0 turns it off.

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, Ordering};

use alloc::string::String;
use rt::os::{syscall3, syscall6, SYS_INOTIFY_ADD_WATCH, SYS_INOTIFY_INIT1, SYS_READ, SYS_RT_SIGACTION};

use crate::linux_dbus::{Conn, Msg, T_SIGNAL, Value};
use crate::{E_POWER, E_SESSION_END, E_SETTINGS, Event, SET_CONTRAST, SET_FONTS, SET_LOCALE, SET_OTHER, SET_THEME};

const SIGTERM: usize = 15;
const SA_RESTORER: usize = 0x0400_0000;
const SA_RESTART: usize = 0x1000_0000;

static STARTED: AtomicBool = AtomicBool::new(false);
static TERM: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn jfgui_rs_sigret();
}

// SIGTERM: only async-signal-safe work (an atomic, the wake-up write)
extern "C" fn on_term(_sig: i32) {
    TERM.store(true, Ordering::SeqCst);
    crate::wake();
}

/// the SIGTERM handler and the watcher thread (once)
pub fn life_start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let sa = [on_term as *const () as usize, SA_RESTORER | SA_RESTART, jfgui_rs_sigret as *const () as usize, 0];
    // SAFETY: a kernel sigaction record with our handler and the shared restorer.
    unsafe { syscall6(SYS_RT_SIGACTION, SIGTERM, sa.as_ptr() as usize, 0, 8, 0, 0) };
    if crate::env("jfgui_LIFE").is_some_and(|v| v.starts_with('0')) {
        return;
    }
    let _ = crate::thread_spawn(128 << 10, Box::new(watch));
}

/// (wait) a SIGTERM since the last call -> E_SESSION_END queued
pub fn life_poll() {
    if TERM.swap(false, Ordering::SeqCst) {
        crate::evq_push(Event { typ: E_SESSION_END, ..Default::default() });
    }
}

fn push(typ: u8, data: u32) {
    crate::evq_push(Event { typ, data, ..Default::default() });
    crate::wake();
}

fn add_match(c: &mut Conn, rule: &str) {
    c.send(&Msg::call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "AddMatch", alloc::vec![Value::str(rule)]));
}

fn watch() {
    let mut sys = Conn::system();
    let mut ses = Conn::session();
    if let Some(c) = sys.as_mut() {
        add_match(c, "type='signal',interface='org.freedesktop.login1.Manager',member='PrepareForSleep'");
    }
    if let Some(c) = ses.as_mut() {
        add_match(c, "type='signal',interface='org.freedesktop.portal.Settings',member='SettingChanged'");
    }
    let mut files = Files::new();
    loop {
        for c in [sys.as_mut(), ses.as_mut()].into_iter().flatten() {
            while let Some(m) = c.read(0) {
                handle(&m);
            }
        }
        if let Some(f) = files.as_mut() {
            if f.changed() && crate::linux_settings::files_used() {
                push(E_SETTINGS, SET_THEME | SET_OTHER);
            }
        }
        if sys.as_ref().is_some_and(|c| c.broken) {
            sys = None;
        }
        if ses.as_ref().is_some_and(|c| c.broken) {
            ses = None;
        }
        let fds = [sys.as_ref().map_or(-1, |c| c.fd()), ses.as_ref().map_or(-1, |c| c.fd()), files.as_ref().map_or(-1, |f| f.fd as i32)];
        if fds == [-1, -1, -1] {
            return;
        }
        let mut pfd = fds.map(|fd| fd as u32 as u64 | (1u64 << 32));
        // SAFETY: three pollfd records (a negative fd is skipped).
        unsafe { rt::os::poll(pfd.as_mut_ptr() as usize, 3, -1) };
    }
}

const IN_NONBLOCK: usize = 0o4000;
const IN_CLOEXEC: usize = 0o2000000;
const IN_CLOSE_WRITE: u32 = 0x8;
const IN_MOVED_FROM: u32 = 0x40;
const IN_MOVED_TO: u32 = 0x80;
const IN_CREATE: u32 = 0x100;
const IN_DELETE: u32 = 0x200;
const IN_ISDIR: u32 = 0x4000_0000;
const IN_MASK: u32 = IN_CLOSE_WRITE | IN_MOVED_FROM | IN_MOVED_TO | IN_CREATE | IN_DELETE;

/// the settings files without the portal: inotify on the config folder and GTK's folders
struct Files {
    fd: usize,
    home: String,
    /// the watch descriptors of $XDG_CONFIG_HOME, gtk-4.0, gtk-3.0 (-1: none yet)
    wd: [i32; 3],
}

impl Files {
    fn new() -> Option<Files> {
        let home = crate::config_dir()?;
        // SAFETY: no pointers.
        let fd = unsafe { syscall3(SYS_INOTIFY_INIT1, IN_NONBLOCK | IN_CLOEXEC, 0, 0) };
        if fd < 0 {
            return None;
        }
        let mut f = Files { fd: fd as usize, home, wd: [-1; 3] };
        f.add(0);
        f.add(1);
        f.add(2);
        Some(f)
    }

    // watch folder k (0 the config home, 1 gtk-4.0, 2 gtk-3.0) when it exists
    fn add(&mut self, k: usize) {
        let dir = match k {
            0 => self.home.clone(),
            1 => alloc::format!("{}/gtk-4.0", self.home),
            _ => alloc::format!("{}/gtk-3.0", self.home),
        };
        let z: alloc::vec::Vec<u8> = dir.bytes().chain(core::iter::once(0)).collect();
        // SAFETY: a NUL-terminated path.
        let wd = unsafe { syscall3(SYS_INOTIFY_ADD_WATCH, self.fd, z.as_ptr() as usize, IN_MASK as usize) };
        self.wd[k] = if wd < 0 { -1 } else { wd as i32 };
    }

    /// read the queued events -> one of the settings files changed
    fn changed(&mut self) -> bool {
        let mut buf = [0u8; 4096];
        let mut hit = false;
        loop {
            // SAFETY: the buffer is writable for its length.
            let n = unsafe { syscall3(SYS_READ, self.fd, buf.as_mut_ptr() as usize, buf.len()) };
            if n <= 0 {
                return hit;
            }
            let (n, mut off) = (n as usize, 0usize);
            while off + 16 <= n {
                let rd = |i: usize| u32::from_ne_bytes([buf[off + i], buf[off + i + 1], buf[off + i + 2], buf[off + i + 3]]);
                let (wd, mask, len) = (rd(0) as i32, rd(4), rd(12) as usize);
                let name = &buf[off + 16..(off + 16 + len).min(n)];
                let name = &name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())];
                off += 16 + len;
                if wd == self.wd[0] {
                    if mask & IN_ISDIR != 0 && mask & (IN_CREATE | IN_MOVED_TO) != 0 {
                        // GTK's folder made after the start: watch it, its file may be new too
                        match name {
                            b"gtk-4.0" => self.add(1),
                            b"gtk-3.0" => self.add(2),
                            _ => continue,
                        }
                        hit = true;
                    } else if name == b"kdeglobals" {
                        hit = true;
                    }
                } else if (wd == self.wd[1] || wd == self.wd[2]) && name == b"settings.ini" {
                    hit = true;
                }
            }
        }
    }
}

fn handle(m: &Msg) {
    if m.typ != T_SIGNAL {
        return;
    }
    match (m.iface.as_str(), m.member.as_str()) {
        ("org.freedesktop.login1.Manager", "PrepareForSleep") => {
            if let Some(Value::B(sleep)) = m.body.first() {
                push(E_POWER, *sleep as u32);
            }
        }
        ("org.freedesktop.portal.Settings", "SettingChanged") => {
            let ns = m.body.first().and_then(|v| v.as_str()).unwrap_or("");
            let key = m.body.get(1).and_then(|v| v.as_str()).unwrap_or("");
            let bits = setting_bits(ns, key);
            if bits != 0 {
                push(E_SETTINGS, bits);
            }
        }
        _ => {}
    }
}

/// the SET_* bits of a portal setting (0: none the UI follows)
pub(crate) fn setting_bits(ns: &str, key: &str) -> u32 {
    match key {
        "color-scheme" | "accent-color" | "gtk-theme" => SET_THEME,
        "contrast" | "high-contrast" => SET_CONTRAST,
        "font-name" | "monospace-font-name" | "document-font-name" | "text-scaling-factor" | "font-antialiasing"
        | "font-hinting" | "font-rgba-order" => SET_FONTS,
        "region" if ns.contains("locale") => SET_LOCALE,
        "button-layout" | "action-double-click-titlebar" | "action-middle-click-titlebar" | "action-right-click-titlebar"
            if ns == "org.gnome.desktop.wm.preferences" =>
        {
            SET_OTHER
        }
        _ => 0,
    }
}
