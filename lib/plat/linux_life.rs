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
// SET_LOCALE (region); other keys are ignored.  Without a bus the thread ends quietly;
// jfgui_LIFE=0 turns it off.

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, Ordering};

use rt::os::syscall6;

use crate::linux_dbus::{Conn, Msg, T_SIGNAL, Value};
use crate::{E_POWER, E_SESSION_END, E_SETTINGS, Event, SET_CONTRAST, SET_FONTS, SET_LOCALE, SET_THEME};

const SYS_RT_SIGACTION: usize = 13;
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
    loop {
        for c in [sys.as_mut(), ses.as_mut()].into_iter().flatten() {
            while let Some(m) = c.read(0) {
                handle(&m);
            }
        }
        if sys.as_ref().is_some_and(|c| c.broken) {
            sys = None;
        }
        if ses.as_ref().is_some_and(|c| c.broken) {
            ses = None;
        }
        let fds = [sys.as_ref().map_or(-1, |c| c.fd()), ses.as_ref().map_or(-1, |c| c.fd())];
        if fds == [-1, -1] {
            return;
        }
        let mut pfd = [fds[0] as u32 as u64 | (1u64 << 32), fds[1] as u32 as u64 | (1u64 << 32)];
        // SAFETY: two pollfd records (a negative fd is skipped).
        unsafe { syscall6(7, pfd.as_mut_ptr() as usize, 2, usize::MAX, 0, 0, 0) };
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
        _ => 0,
    }
}
