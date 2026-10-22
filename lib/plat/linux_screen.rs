// linux_screen -- (screen.rs) the monitors, pictures of the screen and the colour pick on Linux
//
//   monitors()          the display backend's: X11 RandR (linux_x11_screen.rs), Wayland's
//                       outputs (linux_wl_window.rs); nothing before the first window
//   pick_* / capture_*  X11 (GetImage, a pointer grab) when the backend is X11 outside
//                       Flatpak; the screenshot portal in Wayland sessions, in Flatpak and
//                       without a window: org.freedesktop.portal.Screenshot.PickColor ("",
//                       {handle_token}) -> Response {color: (ddd)}, .Screenshot ("",
//                       {handle_token, modal, interactive: false}) -> Response {uri}: the file
//                       is read (it stays where the portal put it) -- each on a thread of its
//                       own, the answer queued (screen.rs push_color / push_shot)
//   jfgui_SCREEN=portal | x11 picks the way (tests).  The portal's availability is its version
//   property (Screenshot >= 1 for pictures, >= 2 for PickColor), asked once.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use rt::sync::SpinLock;

use crate::linux_dbus::{Conn, Value};
use crate::linux_desk::{PortalError, in_flatpak, portal_request, portal_version, result};
use crate::screen::{Monitor, ScreenError, Shot, push_color, push_shot};
use crate::{BK_WAYLAND, BK_X11, disp_backend};

/// how long the portal may take (its dialog: the user decides)
const WAIT_MS: u64 = 120_000;
const IFACE: &str = "org.freedesktop.portal.Screenshot";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Way {
    X11,
    Portal,
    None,
}

fn way() -> Way {
    match crate::env("jfgui_SCREEN").as_deref() {
        Some("portal") => return Way::Portal,
        Some("x11") => return Way::X11,
        _ => {}
    }
    if in_flatpak() {
        return Way::Portal;
    }
    match disp_backend() {
        BK_X11 => Way::X11,
        BK_WAYLAND => Way::Portal,
        _ if crate::linux_dbus::session_available() => Way::Portal,
        _ => Way::None,
    }
}

pub fn monitors() -> Vec<Monitor> {
    match disp_backend() {
        BK_X11 => crate::x11scr::monitors(),
        BK_WAYLAND => crate::wl_monitors(),
        _ => Vec::new(),
    }
}

// the Screenshot portal's version + 1 (0 = not asked yet)
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

pub fn pick_available() -> bool {
    match way() {
        Way::X11 => crate::x11scr::ready(),
        Way::Portal => version() >= 2,
        Way::None => false,
    }
}

pub fn capture_available() -> bool {
    match way() {
        Way::X11 => crate::x11scr::ready(),
        Way::Portal => version() >= 1,
        Way::None => false,
    }
}

// the portal threads (joined before the next one starts: their stacks are freed then)
static PICKER: SpinLock<Option<crate::Thread>> = SpinLock::new(None);
static SHOOTER: SpinLock<Option<crate::Thread>> = SpinLock::new(None);

fn spawn(slot: &SpinLock<Option<crate::Thread>>, f: Box<dyn FnOnce() + Send>) -> bool {
    if let Some(t) = slot.lock().take() {
        t.join();
    }
    match crate::thread_spawn(0, f) {
        Some(t) => {
            *slot.lock() = Some(t);
            true
        }
        None => false,
    }
}

fn portal_err(e: PortalError) -> ScreenError {
    match e {
        PortalError::Missing => ScreenError::Unavailable,
        PortalError::Timeout => ScreenError::Cancelled,
        PortalError::Failed(s) if s.is_empty() => ScreenError::Failed(String::from("the session bus closed the connection")),
        PortalError::Failed(s) => ScreenError::Failed(s),
    }
}

fn code_err(code: u32) -> ScreenError {
    if code == 1 { ScreenError::Cancelled } else { ScreenError::Denied }
}

// PickColor on the session bus
fn portal_pick() -> Result<u32, ScreenError> {
    let mut c = Conn::session().ok_or(ScreenError::Unavailable)?;
    let (code, res) = portal_request(&mut c, IFACE, "PickColor", alloc::vec![Value::str("")], Vec::new(), "pick", WAIT_MS, &mut |_| {})
        .map_err(portal_err)?;
    if code != 0 {
        return Err(code_err(code));
    }
    let rgb = match result(&res, "color") {
        Some(Value::R(v)) if v.len() == 3 => v.iter().map(|c| if let Value::D(d) = c { (d.clamp(0.0, 1.0) * 255.0 + 0.5) as u32 } else { 0 }).fold(0, |a, c| a << 8 | c),
        _ => return Err(ScreenError::Failed(String::from("the portal answered without a colour"))),
    };
    Ok(rgb)
}

// a file:// URI -> the path (%XX decoded)
fn uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..]; // skip a host name
    let b = path.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(core::str::from_utf8(&b[i + 1..i + 3]).ok()?, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

// Screenshot on the session bus: the file it wrote
fn portal_shot() -> Result<Shot, ScreenError> {
    let mut c = Conn::session().ok_or(ScreenError::Unavailable)?;
    let opts = alloc::vec![("modal", Value::B(true)), ("interactive", Value::B(false))];
    let (code, res) = portal_request(&mut c, IFACE, "Screenshot", alloc::vec![Value::str("")], opts, "shot", WAIT_MS, &mut |_| {})
        .map_err(portal_err)?;
    if code != 0 {
        return Err(code_err(code));
    }
    let uri = result(&res, "uri").and_then(|v| v.as_str()).ok_or(ScreenError::Failed(String::from("the portal answered without a file")))?;
    let path = uri_path(uri).ok_or_else(|| ScreenError::Failed(alloc::format!("not a local file: {uri}")))?;
    // the portal's own file for this request: read without the app's file rules (the
    // capture itself was allowed)
    let data = crate::read_file_raw(&path).ok_or_else(|| ScreenError::Failed(alloc::format!("cannot read {path}")))?;
    // a PNG's size from its IHDR (the UI decodes it)
    let (w, h) = if data.len() >= 24 && data.starts_with(b"\x89PNG\r\n\x1a\n") {
        (u32::from_be_bytes([data[16], data[17], data[18], data[19]]) as i32, u32::from_be_bytes([data[20], data[21], data[22], data[23]]) as i32)
    } else {
        (0, 0)
    };
    Ok(Shot { rect: [0, 0, w, h], w, h, px: Vec::new(), file: Some((path, data)) })
}

pub fn pick_start(test: Option<(i32, i32)>) -> Result<(), ScreenError> {
    match way() {
        Way::X11 if crate::x11scr::ready() => crate::x11scr::pick_start(test),
        Way::Portal => {
            if !spawn(&PICKER, Box::new(|| push_color(portal_pick()))) {
                return Err(ScreenError::Failed(String::from("no thread for the portal")));
            }
            Ok(())
        }
        _ => Err(ScreenError::Unavailable),
    }
}

pub fn capture_start(rect: Option<[i32; 4]>) -> Result<(), ScreenError> {
    match way() {
        Way::X11 if crate::x11scr::ready() => {
            push_shot(crate::x11scr::capture(rect));
            Ok(())
        }
        Way::Portal => {
            if !spawn(&SHOOTER, Box::new(|| push_shot(portal_shot()))) {
                return Err(ScreenError::Failed(String::from("no thread for the portal")));
            }
            Ok(())
        }
        _ => Err(ScreenError::Unavailable),
    }
}
