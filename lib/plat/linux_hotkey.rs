// linux_hotkey -- (hotkey.rs) global shortcuts on Linux: XGrabKey on X11 (linux_x11_screen.rs),
// the GlobalShortcuts portal in Wayland sessions and in Flatpak
//
//   available() / register(id, chord, description) / unregister(id)
//
// The portal (org.freedesktop.portal.GlobalShortcuts) runs on a thread of its own with its own
// session-bus connection, started by the first registration: CreateSession ({handle_token,
// session_handle_token}) -> Response {session_handle}; BindShortcuts (session, [(id, {
// description, preferred_trigger})], "", {handle_token}) with every registered shortcut ->
// Response {shortcuts}: the desktop asks the user, who may bind another chord or none; an id
// missing from the answer (or a cancelled dialog) is Refused(id, Dismissed); Activated
// (session, id, timestamp, options) -> Pressed(id).  A change (register, unregister) closes
// the session (Session.Close) and binds the whole set in a new one within 100 ms; with
// nothing left the thread ends.  register answers Ok at once on this path: the verdict comes
// later as Refused.  jfgui_HOTKEYS=portal | x11 picks the way (tests).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use rt::sync::SpinLock;

use crate::hotkey::{self, Chord, HotkeyError, HotkeyEvent};
use crate::linux_dbus::{Conn, Msg, T_SIGNAL, Value};
use crate::linux_desk::{PORTAL, PortalError, in_flatpak, no_reply, portal_request, portal_version, result, token};
use crate::{BK_WAYLAND, BK_X11, disp_backend};

const IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
/// how long the portal may take (its dialog: the user decides)
const WAIT_MS: u64 = 120_000;

fn portal_way() -> bool {
    match crate::env("jfgui_HOTKEYS").as_deref() {
        Some("portal") => return true,
        Some("x11") => return false,
        _ => {}
    }
    in_flatpak() || disp_backend() == BK_WAYLAND || (disp_backend() != BK_X11 && crate::linux_dbus::session_available())
}

// the portal's version + 1 (0 = not asked yet)
static VERSION: AtomicU32 = AtomicU32::new(0);

fn version() -> u32 {
    let v = VERSION.load(Ordering::Relaxed);
    if v != 0 {
        return v - 1;
    }
    let v = Conn::session().map_or(0, |mut c| portal_version(&mut c, IFACE));
    VERSION.store(v + 1, Ordering::Relaxed);
    v
}

pub fn available() -> bool {
    if portal_way() { version() >= 1 } else { crate::x11scr::ready() }
}

pub fn register(id: u32, chord: Chord, _description: &str) -> Result<(), HotkeyError> {
    if !portal_way() {
        if !crate::x11scr::ready() {
            return Err(HotkeyError::Unavailable);
        }
        return crate::x11scr::hotkey_grab(id, chord);
    }
    if version() == 0 {
        return Err(HotkeyError::Unavailable);
    }
    // hotkey.rs records it right after this returns: the thread binds the set it reads then
    changed();
    Ok(())
}

pub fn unregister(id: u32) {
    if portal_way() {
        changed();
    } else {
        crate::x11scr::hotkey_ungrab(id);
    }
}

// ---------------------------------------------------------------- the portal thread

struct Svc {
    /// the thread runs
    on: bool,
    /// the set changed: bind it again
    dirty: bool,
    thread: Option<crate::Thread>,
}

static SVC: SpinLock<Svc> = SpinLock::new(Svc { on: false, dirty: false, thread: None });

// the set changed: the thread (started when needed) binds it again
fn changed() {
    let mut s = SVC.lock();
    s.dirty = true;
    if s.on {
        return;
    }
    if let Some(t) = s.thread.take() {
        drop(s);
        t.join(); // ended (on is false): its stack goes
        s = SVC.lock();
    }
    s.on = true;
    s.thread = crate::thread_spawn(0, Box::new(service));
    if s.thread.is_none() {
        s.on = false;
    }
}

fn refuse_all(ids: &[u32], why: HotkeyError) {
    for &id in ids {
        if hotkey::registered(id) {
            hotkey::refused(id, why.clone());
        }
    }
}

fn why_of(e: PortalError) -> HotkeyError {
    match e {
        PortalError::Missing => HotkeyError::Unavailable,
        PortalError::Timeout => HotkeyError::Dismissed,
        PortalError::Failed(s) if s.is_empty() => HotkeyError::Failed(String::from("the session bus closed the connection")),
        PortalError::Failed(s) => HotkeyError::Failed(s),
    }
}

// an Activated signal of our session -> Pressed
fn activated(m: &Msg, session: &str, bound: &[u32]) {
    if m.typ != T_SIGNAL || m.iface != IFACE || m.member != "Activated" {
        return;
    }
    if m.body.first().and_then(|v| v.as_str()) != Some(session) {
        return;
    }
    if let Some(id) = m.body.get(1).and_then(|v| v.as_str()).and_then(|s| s.parse::<u32>().ok()) {
        if bound.contains(&id) {
            hotkey::push(HotkeyEvent::Pressed(id));
        }
    }
}

fn close(c: &mut Conn, session: &mut Option<String>) {
    if let Some(s) = session.take() {
        c.send(&no_reply(Msg::call(PORTAL, &s, "org.freedesktop.portal.Session", "Close", Vec::new())));
    }
}

// the thread: binds the set whenever it changed, reports Activated
fn service() {
    let Some(mut c) = Conn::session() else {
        let ids: Vec<u32> = hotkey::hotkey_entries().iter().map(|e| e.0).collect();
        refuse_all(&ids, HotkeyError::Unavailable);
        SVC.lock().on = false;
        return;
    };
    let rule = alloc::format!("type='signal',interface='{IFACE}',member='Activated'");
    c.send(&Msg::call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "AddMatch", alloc::vec![Value::S(rule)]));
    let mut session: Option<String> = None;
    let mut bound: Vec<u32> = Vec::new();
    loop {
        let dirty = {
            let mut s = SVC.lock();
            core::mem::replace(&mut s.dirty, false)
        };
        if dirty {
            close(&mut c, &mut session);
            bound.clear();
            let want = hotkey::hotkey_entries();
            if want.is_empty() {
                let mut s = SVC.lock();
                if !s.dirty {
                    s.on = false; // nothing to bind: the thread ends (the connection with it)
                    return;
                }
                continue;
            }
            let ids: Vec<u32> = want.iter().map(|e| e.0).collect();
            match bind(&mut c, &want, &mut session) {
                Ok(got) => {
                    for &id in &ids {
                        if !got.contains(&id) {
                            hotkey::refused(id, HotkeyError::Dismissed);
                        }
                    }
                    bound = got;
                }
                Err(e) => refuse_all(&ids, e),
            }
        }
        if let Some(m) = c.read(100) {
            if let Some(s) = session.as_deref() {
                activated(&m, s, &bound);
            }
        } else if c.broken {
            let ids: Vec<u32> = hotkey::hotkey_entries().iter().map(|e| e.0).collect();
            refuse_all(&ids, HotkeyError::Failed(String::from("the session bus closed the connection")));
            SVC.lock().on = false;
            return;
        }
    }
}

// a session and BindShortcuts with the set -> the ids bound
fn bind(c: &mut Conn, want: &[(u32, Chord, String)], session: &mut Option<String>) -> Result<Vec<u32>, HotkeyError> {
    let opts = alloc::vec![("session_handle_token", Value::S(token("hks")))];
    let (code, res) = portal_request(c, IFACE, "CreateSession", Vec::new(), opts, "hk", WAIT_MS, &mut |_| {}).map_err(why_of)?;
    if code != 0 {
        return Err(if code == 1 { HotkeyError::Dismissed } else { HotkeyError::Failed(String::from("the portal made no session")) });
    }
    let s = result(&res, "session_handle").and_then(|v| v.as_str()).map(String::from).ok_or(HotkeyError::Failed(String::from("no session handle")))?;
    *session = Some(s.clone());
    let list: Vec<Value> = want
        .iter()
        .map(|(id, chord, desc)| {
            let mut props = alloc::vec![("description", Value::S(desc.clone()))];
            if let Some(t) = chord.trigger() {
                props.push(("preferred_trigger", Value::S(t)));
            }
            Value::R(alloc::vec![Value::S(alloc::format!("{id}")), Value::dict(props)])
        })
        .collect();
    let args = alloc::vec![Value::O(s.clone()), Value::A(String::from("(sa{sv})"), list), Value::str("")];
    let bound_so_far: Vec<u32> = Vec::new();
    let (code, res) = portal_request(c, IFACE, "BindShortcuts", args, Vec::new(), "hk", WAIT_MS, &mut |m| activated(m, &s, &bound_so_far)).map_err(why_of)?;
    if code != 0 {
        return Err(if code == 1 { HotkeyError::Dismissed } else { HotkeyError::Failed(String::from("the portal refused the shortcuts")) });
    }
    let mut got = Vec::new();
    if let Some(Value::A(_, items)) = result(&res, "shortcuts") {
        for it in items {
            if let Value::R(f) = it {
                if let Some(id) = f.first().and_then(|v| v.as_str()).and_then(|s| s.parse::<u32>().ok()) {
                    got.push(id);
                }
            }
        }
    }
    Ok(got)
}
