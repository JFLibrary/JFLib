// linux_power -- (power.rs) inhibitions on Linux, on a session-bus connection of their own
// (linux_desk.rs Link; open while a guard lives)
//
//   hold(reason, what) -> Option<Held>
//     1. org.freedesktop.portal.Inhibit.Inhibit("", flags (4 suspend, 8 idle), {handle_token,
//        reason}) -> the request's path.  The portal answers before its backend did: a failing
//        backend sends Response(2) on that path right after (xdg-desktop-portal-gtk, for one,
//        only inhibits idle), so the answer is awaited SETTLE_MS (AddMatch on the path first).
//     2. INHIBIT_IDLE refused: the portal again with idle alone (8; an idle session does not
//        suspend on its own), the power manager below as best effort for suspend.
//     3. without the portal: org.freedesktop.ScreenSaver.Inhibit(app, reason) for idle and
//        org.freedesktop.PowerManagement.Inhibit(app, reason) for suspend (required when only
//        suspend was asked, else best effort).
//   release(&mut Held)   Request.Close / UnInhibit(cookie); the connection closes with the
//                        last guard (which ends every inhibition of it as well)

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use super::{INHIBIT_IDLE, INHIBIT_SUSPEND};
use crate::linux_dbus::{Conn, Msg, T_ERROR, T_RETURN, T_SIGNAL, Value};
use crate::linux_desk::{Link, call, drain, no_reply};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SAVER: &str = "org.freedesktop.ScreenSaver";
const SAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
const PM: &str = "org.freedesktop.PowerManagement";
const PM_PATH: &str = "/org/freedesktop/PowerManagement/Inhibit";
const PM_IFACE: &str = "org.freedesktop.PowerManagement.Inhibit";
const FLAG_SUSPEND: u32 = 4;
const FLAG_IDLE: u32 = 8;
// ms for an answer (the portal may have to be started first: 2.6 s seen in WSL), ms its
// backend's Response may take after it (0 = held, else refused; older portals send none on
// success)
const PORTAL_MS: u64 = 5000;
const SETTLE_MS: u64 = 300;
const CALL_MS: i32 = 1000;

static LINK: Link = Link::new();
// live guards (the connection closes at 0) and the handle tokens
static HELD: AtomicU32 = AtomicU32::new(0);
static TOKEN: AtomicU32 = AtomicU32::new(0);

enum Part {
    /// the portal's request object
    Portal(String),
    /// a cookie of org.freedesktop.ScreenSaver
    Saver(u32),
    /// a cookie of org.freedesktop.PowerManagement.Inhibit
    Pm(u32),
}

/// what one guard holds
pub struct Held(Vec<Part>);

fn cookie(r: Option<Msg>) -> Option<u32> {
    r?.body.first()?.as_u32()
}

fn bus_call(member: &str, rule: String) -> Msg {
    no_reply(Msg::call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", member, alloc::vec![Value::S(rule)]))
}

// Inhibit on the portal with flags -> the request, unless it was refused (an error, or
// Response != 0 within SETTLE_MS of the answer)
fn portal(c: &mut Conn, reason: &str, flags: u32) -> Option<Part> {
    let token = alloc::format!("jfgui_inhibit{}", TOKEN.fetch_add(1, Ordering::Relaxed) + 1);
    let mut handle = alloc::format!("{PORTAL_PATH}/request/{}/{token}", c.unique.trim_start_matches(':').replace('.', "_"));
    let rule = alloc::format!("type='signal',interface='org.freedesktop.portal.Request',member='Response',path='{handle}'");
    c.send(&bus_call("AddMatch", rule.clone()));
    let opts = Value::dict(alloc::vec![("handle_token", Value::S(token)), ("reason", Value::str(reason))]);
    let m = Msg::call(PORTAL, PORTAL_PATH, "org.freedesktop.portal.Inhibit", "Inhibit", alloc::vec![Value::str(""), Value::U(flags), opts]);
    let serial = c.send(&m)?;
    let (mut answered, mut ok) = (false, false);
    let mut end = crate::time_us() + PORTAL_MS * 1000;
    loop {
        let now = crate::time_us();
        if now >= end || c.broken {
            break;
        }
        let Some(r) = c.read(((end - now) / 1000) as i32 + 1) else { continue };
        match r.typ {
            T_RETURN if r.reply_to == serial => {
                if let Some(Value::O(p)) = r.body.first() {
                    handle = p.clone();
                    (answered, ok) = (true, true);
                    end = crate::time_us() + SETTLE_MS * 1000;
                } else {
                    break;
                }
            }
            T_ERROR if r.reply_to == serial => break,
            T_SIGNAL if r.member == "Response" && r.path == handle => {
                // the backend refused: the portal drops the request itself
                ok = r.body.first().and_then(|v| v.as_u32()) == Some(0) && answered;
                break;
            }
            _ => {}
        }
    }
    c.send(&bus_call("RemoveMatch", rule));
    ok.then_some(Part::Portal(handle))
}

fn power_manager(c: &mut Conn, reason: &str) -> Option<Part> {
    let args = alloc::vec![Value::S(crate::vfs::app_id()), Value::str(reason)];
    cookie(call(c, &Msg::call(PM, PM_PATH, PM_IFACE, "Inhibit", args), CALL_MS)).map(Part::Pm)
}

// without the portal: the screen saver (idle) and the power manager (suspend)
fn fallback(c: &mut Conn, reason: &str, what: u32) -> Option<Vec<Part>> {
    let mut parts = Vec::new();
    if what & INHIBIT_IDLE != 0 {
        let args = alloc::vec![Value::S(crate::vfs::app_id()), Value::str(reason)];
        let k = cookie(call(c, &Msg::call(SAVER, SAVER_PATH, SAVER, "Inhibit", args), CALL_MS))?;
        parts.push(Part::Saver(k));
    }
    if what & INHIBIT_SUSPEND != 0 {
        match power_manager(c, reason) {
            Some(p) => parts.push(p),
            // only suspend was asked: the power manager is required
            None if what & INHIBIT_IDLE == 0 => return None,
            None => {}
        }
    }
    Some(parts)
}

fn acquire(c: &mut Conn, reason: &str, what: u32) -> Option<Vec<Part>> {
    let flags = if what & INHIBIT_SUSPEND != 0 { FLAG_SUSPEND } else { 0 } | if what & INHIBIT_IDLE != 0 { FLAG_IDLE } else { 0 };
    if let Some(p) = portal(c, reason, flags) {
        return Some(alloc::vec![p]);
    }
    if what & INHIBIT_IDLE != 0 {
        if let Some(p) = portal(c, reason, FLAG_IDLE) {
            let mut parts = alloc::vec![p];
            parts.extend(power_manager(c, reason));
            return Some(parts);
        }
    }
    fallback(c, reason, what)
}

fn send_release(c: &mut Conn, p: &Part) {
    let m = match p {
        Part::Portal(path) => Msg::call(PORTAL, path, "org.freedesktop.portal.Request", "Close", Vec::new()),
        Part::Saver(k) => Msg::call(SAVER, SAVER_PATH, SAVER, "UnInhibit", alloc::vec![Value::U(*k)]),
        Part::Pm(k) => Msg::call(PM, PM_PATH, PM_IFACE, "UnInhibit", alloc::vec![Value::U(*k)]),
    };
    c.send(&no_reply(m));
}

pub fn hold(reason: &str, what: u32) -> Option<Held> {
    let parts = LINK.with(|c| {
        drain(c, |_| None);
        let parts = acquire(c, reason, what)?;
        HELD.fetch_add(1, Ordering::SeqCst);
        Some(parts)
    })
    .flatten();
    if parts.is_none() {
        LINK.close_if(|| HELD.load(Ordering::SeqCst) == 0); // nothing held: no connection kept
    }
    Some(Held(parts?))
}

pub fn release(h: &mut Held) {
    if h.0.is_empty() {
        return;
    }
    let parts = core::mem::take(&mut h.0);
    LINK.with(|c| {
        drain(c, |_| None);
        for p in &parts {
            send_release(c, p);
        }
    });
    if HELD.fetch_sub(1, Ordering::SeqCst) == 1 {
        LINK.close_if(|| HELD.load(Ordering::SeqCst) == 0);
    }
}
