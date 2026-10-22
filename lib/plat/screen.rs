// screen -- the monitors, the colour of any pixel on the screen, pictures of the screen
//
//   monitors() -> Vec<Monitor>        every monitor: id, name, rectangle and work area (desktop
//                                     units), dpi, refresh rate, the primary one first; when the
//                                     set or one of them changes, E_SETTINGS with SET_MONITORS
//   monitor_at(x, y) -> Option<Monitor>   the monitor holding a desktop point, else the primary
//   pick_available() -> bool          the user can pick a colour outside the app's windows here
//   pick_start() -> Result<(), ScreenError>   the user clicks a pixel anywhere on the screen (Esc
//                                     or a right click cancels); the answer is ScreenEvent::Color
//   capture_available() -> bool       pictures of the screen can be taken here
//   capture_start(rect) -> Result<(), ScreenError>   a picture of the whole desktop (None) or
//                                     of a rectangle (desktop units); needs `screen.capture` in
//                                     the manifest (else Denied, "PERMDENY screen.capture"); the
//                                     answer is ScreenEvent::Shot
//   screen_take() -> Vec<ScreenEvent> the answers since the last call (each one calls wake())
//   pick_test_point(Some((x, y)))     (test aid) the next pick_start answers the pixel at the
//                                     desktop point x, y without asking the user (Windows,
//                                     X11; the portal and the browser ignore it)
//
// One pick and one capture at a time (else Busy).  Picking needs no right: the user chooses
// the one pixel the app gets, like a file in the file dialog.  Call pick_start and
// capture_start from the UI thread: their answers are made there (Windows, X11) or on a thread
// of their own (the portal), and arrive through the event loop.
//
// Windows (win_screen.rs): EnumDisplayMonitors + GetMonitorInfoW (device name, work area,
// primary), GetDpiForMonitor (shcore), EnumDisplaySettingsW (refresh), WM_DISPLAYCHANGE and a
// changed work area -> SET_MONITORS; capture: BitBlt (CAPTUREBLT) from the screen DC into a DIB
// section; pick: a topmost window over the virtual desktop showing a frozen picture of it with
// a magnifier at the pointer.  Linux (linux_screen.rs): X11 RandR 1.5 (GetMonitors, the CRTC's
// mode for the refresh rate, _NET_WORKAREA, RRScreenChangeNotify), GetImage of the root window,
// a pointer grab for the pick; Wayland wl_output (+ zxdg_output_v1 for logical position and
// size; no work area, no primary: the first output); in Wayland sessions and in Flatpak the
// screenshot portal (Screenshot, PickColor: the desktop asks the user).  Browser: the host's
// window.screen (getScreenDetails when the page may), the EyeDropper API (Chromium) for the
// pick, no capture.  Android: the host's DisplayManager displays, no pick, no capture.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use rt::sync::SpinLock;

/// one monitor
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Monitor {
    /// stable while the monitor stays connected (Windows: a hash of its device name, X11 the
    /// RandR monitor's name atom, Wayland the output's global name, browser / Android an index)
    pub id: u64,
    /// the system's name ("\\.\DISPLAY1", "HDMI-1", "rdp-0"); "" when unknown
    pub name: String,
    /// x, y, w, h in desktop units: physical pixels on Windows, X11 and Android; the
    /// compositor's logical units on Wayland; CSS pixels in the browser
    pub rect: [i32; 4],
    /// the part windows should use (without taskbars, docks, panels); = rect where unknown
    pub work: [i32; 4],
    /// 96 = 100 %
    pub dpi: u32,
    /// refresh rate in mHz (60000 = 60 Hz), 0 = unknown
    pub refresh_mhz: u32,
    /// the primary monitor (exactly one in a non-empty list)
    pub primary: bool,
}

/// why a pick or a capture did not happen
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScreenError {
    /// not possible here (browser without EyeDropper, Android, no X server / portal)
    Unavailable,
    /// the app's permissions lack `screen.capture`, or the system / the user refused
    Denied,
    /// the user cancelled (Esc, a right click, the portal's dialog dismissed)
    Cancelled,
    /// a pick (or capture) is still running
    Busy,
    /// the system failed: what it said
    Failed(String),
}

impl core::fmt::Display for ScreenError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            ScreenError::Unavailable => f.write_str("not available here"),
            ScreenError::Denied => f.write_str("refused"),
            ScreenError::Cancelled => f.write_str("cancelled"),
            ScreenError::Busy => f.write_str("busy"),
            ScreenError::Failed(s) => write!(f, "failed: {s}"),
        }
    }
}

/// a picture of the screen
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shot {
    /// where it was taken: x, y, w, h in desktop units ([0, 0, w, h] when the system did not
    /// say: the portal)
    pub rect: [i32; 4],
    /// size in pixels
    pub w: i32,
    pub h: i32,
    /// w x h pixels B, G, R, x (rows of w * 4 bytes); empty when `file` holds the picture
    pub px: Vec<u8>,
    /// the picture as the system wrote it (the screenshot portal's PNG): its name and bytes
    pub file: Option<(String, Vec<u8>)>,
}

/// an answer of pick_start / capture_start
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScreenEvent {
    /// the picked colour 0xRRGGBB
    Color(Result<u32, ScreenError>),
    /// the picture
    Shot(Result<Shot, ScreenError>),
}

static EVENTS: SpinLock<Vec<ScreenEvent>> = SpinLock::new(Vec::new());
static PICKING: AtomicBool = AtomicBool::new(false);
static CAPTURING: AtomicBool = AtomicBool::new(false);
static TEST_POINT: SpinLock<Option<(i32, i32)>> = SpinLock::new(None);

/// (platform) the answer of the running pick: queued, the UI thread woken
#[allow(dead_code)]
pub(crate) fn push_color(r: Result<u32, ScreenError>) {
    EVENTS.lock().push(ScreenEvent::Color(r));
    PICKING.store(false, Ordering::SeqCst);
    crate::wake();
}

/// (platform) the answer of the running capture
#[allow(dead_code)]
pub(crate) fn push_shot(r: Result<Shot, ScreenError>) {
    EVENTS.lock().push(ScreenEvent::Shot(r));
    CAPTURING.store(false, Ordering::SeqCst);
    crate::wake();
}

/// (platform) a pick is running
#[allow(dead_code)]
pub(crate) fn picking() -> bool {
    PICKING.load(Ordering::SeqCst)
}

/// the answers since the last call, oldest first
pub fn screen_take() -> Vec<ScreenEvent> {
    core::mem::take(&mut *EVENTS.lock())
}

/// every monitor, the primary one first (empty when nothing is known: Linux before the first
/// window, headless runs)
pub fn monitors() -> Vec<Monitor> {
    let mut v = native::monitors();
    sort_primary(&mut v);
    v
}

/// the primary monitor first, the others by position; exactly one primary
pub(crate) fn sort_primary(v: &mut [Monitor]) {
    if v.is_empty() {
        return;
    }
    if !v.iter().any(|m| m.primary) {
        v[0].primary = true;
    }
    let mut seen = false;
    for m in v.iter_mut() {
        if m.primary && seen {
            m.primary = false;
        }
        seen |= m.primary;
    }
    v.sort_by_key(|m| (!m.primary, m.rect[1], m.rect[0]));
}

/// the monitor holding the desktop point x, y, else the primary one
pub fn monitor_at(x: i32, y: i32) -> Option<Monitor> {
    let v = monitors();
    let inside = |m: &&Monitor| x >= m.rect[0] && y >= m.rect[1] && x < m.rect[0] + m.rect[2] && y < m.rect[1] + m.rect[3];
    v.iter().find(inside).or(v.first()).cloned()
}

/// whether the user can pick a colour anywhere on the screen here
pub fn pick_available() -> bool {
    native::pick_available()
}

/// the user picks a pixel anywhere on the screen; the answer comes as ScreenEvent::Color
pub fn pick_start() -> Result<(), ScreenError> {
    if PICKING.swap(true, Ordering::SeqCst) {
        return Err(ScreenError::Busy);
    }
    let test = TEST_POINT.lock().take();
    let r = native::pick_start(test);
    if r.is_err() {
        PICKING.store(false, Ordering::SeqCst);
    }
    r
}

/// (test aid) the next pick_start answers the pixel at the desktop point x, y at once,
/// without asking the user (Windows, X11; the portal and the browser ignore it); None: ask
/// again
pub fn pick_test_point(p: Option<(i32, i32)>) {
    *TEST_POINT.lock() = p;
}

/// whether pictures of the screen can be taken here (the permission is checked when taking)
pub fn capture_available() -> bool {
    native::capture_available()
}

/// a picture of the whole desktop (None) or of the rectangle x, y, w, h (desktop units); the
/// answer comes as ScreenEvent::Shot.  The screenshot portal takes the whole screen (or what
/// the user chooses in its dialog) whatever rect says.
pub fn capture_start(rect: Option<[i32; 4]>) -> Result<(), ScreenError> {
    if rect.is_some_and(|r| r[2] <= 0 || r[3] <= 0 || r[2] > 32768 || r[3] > 32768) {
        return Err(ScreenError::Failed(String::from("empty or too large rectangle")));
    }
    if !crate::policy::screen_capture_allowed() {
        return Err(ScreenError::Denied);
    }
    if CAPTURING.swap(true, Ordering::SeqCst) {
        return Err(ScreenError::Busy);
    }
    let r = native::capture_start(rect);
    if r.is_err() {
        CAPTURING.store(false, Ordering::SeqCst);
    }
    r
}

/// the pixel 0xRRGGBB at x, y of a shot's B, G, R, x rows (None outside)
pub fn shot_pixel(s: &Shot, x: i32, y: i32) -> Option<u32> {
    if x < 0 || y < 0 || x >= s.w || y >= s.h {
        return None;
    }
    let o = (y as usize * s.w as usize + x as usize) * 4;
    let p = s.px.get(o..o + 3)?;
    Some((p[2] as u32) << 16 | (p[1] as u32) << 8 | p[0] as u32)
}

#[cfg(windows)]
use crate::win_screen as native;

#[cfg(target_os = "linux")]
use crate::linux_screen as native;

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
use crate::web_screen as native;
