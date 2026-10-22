// web_screen -- (screen.rs) the monitors the browser / Android host reports, the browser's
// EyeDropper
//
//   jfgui_screens(len) -> count      the host's monitors from the inbox: one line per monitor,
//                                    "x y w h wx wy ww wh dpi refresh_mhz primary name" (name:
//                                    the rest of the line, may be empty); sent at start and on
//                                    every change (then E_SETTINGS with SET_MONITORS).  Browser
//                                    (web/loader.js): window.screen (CSS pixels, availLeft /
//                                    availTop / availWidth / availHeight, devicePixelRatio) or,
//                                    where the page may already (permission "window-management"
//                                    granted), getScreenDetails() with every screen; Android
//                                    (android/GalleryActivity.java): DisplayManager.getDisplays()
//                                    with their bounds, density and refresh rate
//   monitors()                       that list; before it arrives one monitor of the viewport
//   pick_start()                     REQUEST_PICK_COLOR: the browser's EyeDropper (Chromium,
//                                    from the user's gesture), answered by jfgui_color(id, status,
//                                    rgb); without EyeDropper (the host's capability bit
//                                    HOST_EYEDROPPER) and on Android Unavailable
//   capture_*                        Unavailable (no picture of the screen here)

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use rt::sync::SpinLock;

use crate::screen::{Monitor, ScreenError, push_color};

/// (browser) open the EyeDropper; no payload; answered by jfgui_color
pub const REQUEST_PICK_COLOR: u32 = 21;
/// jfgui_capabilities: the host has the EyeDropper API
pub(crate) const HOST_EYEDROPPER: u32 = 1 << 17;

static SCREENS: SpinLock<Option<Vec<Monitor>>> = SpinLock::new(None);
static EYEDROPPER: AtomicBool = AtomicBool::new(false);
static PICK_ID: AtomicU32 = AtomicU32::new(0);

/// (jfgui_capabilities) the host's EyeDropper
pub(crate) fn set_eyedropper(on: bool) {
    EYEDROPPER.store(on, Ordering::Relaxed);
}

/// one line of jfgui_screens -> the monitor (index k gives the id)
fn parse_line(k: usize, line: &str) -> Option<Monitor> {
    let mut it = line.splitn(12, ' ');
    let mut n = [0i64; 11];
    for v in n.iter_mut() {
        let f = it.next()?.trim().parse::<f64>().ok()?;
        *v = (if f < 0.0 { f - 0.5 } else { f + 0.5 }) as i64; // rounded (no f64::round in core)
    }
    let c = |v: i64| v.clamp(-1_000_000, 1_000_000) as i32;
    let rect = [c(n[0]), c(n[1]), c(n[2]).max(0), c(n[3]).max(0)];
    let work = [c(n[4]), c(n[5]), c(n[6]).max(0), c(n[7]).max(0)];
    Some(Monitor {
        id: k as u64,
        name: String::from(it.next().unwrap_or("").trim()),
        rect,
        work: if work[2] > 0 && work[3] > 0 { work } else { rect },
        dpi: n[8].clamp(24, 960) as u32,
        refresh_mhz: n[9].clamp(0, 1_000_000) as u32,
        primary: n[10] != 0,
    })
}

/// The host's monitors (inbox: one line per monitor, see the module header) -> how many were
/// taken; a different list once the app runs is a change (E_SETTINGS with SET_MONITORS; the
/// first one after the start too: the app saw the viewport until then).
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_screens(len: u32) -> u32 {
    let Some(bytes) = super::input(len) else { return 0 };
    let Ok(text) = core::str::from_utf8(&bytes) else { return 0 };
    let list: Vec<Monitor> = text.lines().filter(|l| !l.trim().is_empty()).take(16).enumerate().filter_map(|(k, l)| parse_line(k, l)).collect();
    let n = list.len() as u32;
    let changed = {
        let mut g = SCREENS.lock();
        let before = g.replace(list);
        super::STARTED.load(core::sync::atomic::Ordering::Relaxed) && before != *g
    };
    if changed {
        let _ = crate::evq_push(crate::Event { typ: crate::E_SETTINGS, data: crate::SET_MONITORS, ..crate::Event::default() });
    }
    n
}

pub fn monitors() -> Vec<Monitor> {
    if let Some(v) = SCREENS.lock().as_ref().filter(|v| !v.is_empty()) {
        return v.clone();
    }
    // nothing from the host yet: the viewport
    let r = crate::win_work_area(0).unwrap_or([0, 0, 0, 0]);
    let dpi = crate::win_dpi(0).max(1) as u32;
    let css = [0, 0, (r[2] as u32 * 96 / dpi) as i32, (r[3] as u32 * 96 / dpi) as i32];
    if css[2] <= 0 {
        return Vec::new();
    }
    alloc::vec![Monitor { id: 0, name: String::new(), rect: css, work: css, dpi, refresh_mhz: 0, primary: true }]
}

pub fn pick_available() -> bool {
    !cfg!(target_os = "android") && EYEDROPPER.load(Ordering::Relaxed)
}

pub fn pick_start(_test: Option<(i32, i32)>) -> Result<(), ScreenError> {
    if !pick_available() {
        return Err(ScreenError::Unavailable);
    }
    let id = super::request(REQUEST_PICK_COLOR, &[], None).ok_or(ScreenError::Failed(String::from("no request slot")))?;
    PICK_ID.store(id, Ordering::SeqCst);
    Ok(())
}

/// The EyeDropper's answer to the pick request id: status 2 = picked (rgb 0xRRGGBB), -4
/// cancelled, -1 refused, -3 not supported, else failed.  1 = taken.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_color(id: u32, status: i32, rgb: u32) -> u32 {
    if id == 0 || PICK_ID.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return 0;
    }
    super::jfgui_complete(id, if status == 2 { 2 } else { status });
    super::forget_request(id);
    push_color(match status {
        2 => Ok(rgb & 0xFF_FFFF),
        -4 => Err(ScreenError::Cancelled),
        -1 => Err(ScreenError::Denied),
        -3 => Err(ScreenError::Unavailable),
        _ => Err(ScreenError::Failed(String::from("the browser's EyeDropper failed"))),
    });
    1
}

pub fn capture_available() -> bool {
    false
}

pub fn capture_start(_rect: Option<[i32; 4]>) -> Result<(), ScreenError> {
    Err(ScreenError::Unavailable)
}
