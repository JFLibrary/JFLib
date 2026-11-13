// win_touch -- fingers on Windows: WM_POINTER* / WM_NCPOINTER* of touch pointers (Windows 8+)
//
//   pointer(hwnd, msg, id, w) -> handled    (win_pen.rs, for a pointer of type PT_TOUCH): a finger
//   nc_pointer(hwnd, msg, wp, w) -> handled (window procedure) a finger over the non-client area
//   activate(hwnd, wp, w) -> the answer to WM_POINTERACTIVATE, None: the system's
//   begin_drag(w, edges) -> true: a finger is down on window w and the window follows it from now on
//        (win_begin_move / win_begin_resize: the system's move loop follows only a mouse)
//   mouse_is_stale() -> true: this WM_MOUSEMOVE is no mouse (window procedure)
//   feedback_off(hwnd)      no system visuals for a touch of this window
//   touch_api_set(Some(api))   (test aid) the touch calls through the given functions (plattest's
//        fakes, like pen_api_set); None = user32's again
//   jfgui_TOUCH_LOG=1          stderr: where each finger came down and whose it became (for a device
//        nobody at the desk has)
//   win_touch_primary() -> a tablet in slate posture    win_touch_keyboard(w, show) the on-screen keyboard
//
// Every finger goes to plat::finger: no gesture is decided here (ui/touch.rs).  A handled pointer
// message never reaches DefWindowProc, and that is the point: DefWindowProc turns a finger into mouse
// messages, a pinch into Ctrl + wheel (preset steps), a two-finger pan into nothing at all and a
// press and hold into a right click.  EnableMouseInPointer stays off: the mouse keeps its messages.
//
// Which window a finger belongs to: a contact is wide and the system gives it to a window frame it
// touches (measured with InjectTouchInput, a contact 60 px wide): a finger on JFPainter's zoom slider
// arrived as WM_NCPOINTERDOWN HTBOTTOM on the tool window above it, whose invisible resize border ends
// at the status bar, and resized that window.  So a finger over a border belongs to the window whose
// client area is under its centre (z-order, from the window that got it), also a contact the system
// gives to a client area its centre is not in.  With no window there the main window's resize is the
// system's; a tool window's frame never is, its resize band for a finger lies inside it (ui edge_grips).
// The position of such a contact is not where the finger is: the system moves it onto the frame (17 px off
// on the slider above, also ptPixelLocationRaw) and it stays off by that while the contact lives; only the
// contact rectangle tells (Slot::adj).
// HTCAPTION: a tool window's title bar is the UI's (press and hold, then begin_drag); the main
// window's goes to the system (moves, snaps, double tap) unless a small widget is close (a finger that
// misses the gear by a hair).
//
// Coordinates come from the pointer's screen position, so the window that was moved under a finger or a
// popup window (win_popup.rs: its input belongs to the owner, in the owner's client coordinates) need
// nothing else.  The API's results are not trusted: counts are clamped, a finger without a free slot is
// nobody's.  A palm is the recogniser's business (ui/touch.rs: the pen comes first); what the system
// itself cancels comes through as FingerPhase::Cancel.

use core::cell::Cell;
use core::sync::atomic::{AtomicUsize, Ordering};

use super::{ClientToScreen, GetClientRect, GetSystemMetrics, GetWindowRect, IsWindowVisible, Point, Rect, ScreenToClient, SetWindowPos,
            UiOnly, WWF_TOP, dll_proc, rec, st, ui_hit_at, win_mods, win_work_area};
use crate::{EDGE_BOTTOM, EDGE_LEFT, EDGE_RIGHT, EDGE_TOP, FingerPhase, HB_BLOCK, HB_DRAG, HB_TEXT, PW_MAX, PWF_NOACTIVATE, PWF_TOOL, evq_push,
            finger_event, reach_clamp};

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindow(h: isize, cmd: u32) -> isize;
    fn GetCursorPos(p: *mut Point) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetTickCount() -> u32;
}

#[link(name = "dwmapi", kind = "raw-dylib")]
unsafe extern "system" {
    fn DwmGetWindowAttribute(h: isize, attr: u32, value: *mut u32, size: u32) -> i32;
}

const PTI_SIZE: usize = 144; // POINTER_TOUCH_INFO (x64)
pub const TOUCH_HIST: usize = 16; // samples per WM_POINTERUPDATE
const SLOTS: usize = 10; // fingers at once: the slot is the finger's id
const PFL_INCONTACT: u32 = 0x4;
const PFL_CANCELED: u32 = 0x8000;
const WM_NCPOINTERUPDATE: u32 = 0x0241;
const WM_NCPOINTERDOWN: u32 = 0x0242;
const WM_NCPOINTERUP: u32 = 0x0243;
const WM_POINTERUPDATE: u32 = 0x0245;
const WM_POINTERDOWN: u32 = 0x0246;
const WM_POINTERUP: u32 = 0x0247;
const WM_POINTERCAPTURECHANGED: u32 = 0x024C;
const PA_ACTIVATE: isize = 1;
const PA_NOACTIVATE: isize = 3;
const HTCAPTION: u32 = 2;
const HTMINBUTTON: u32 = 8;
const HTMAXBUTTON: u32 = 9;
const HTLEFT: u32 = 10;
const HTBOTTOMRIGHT: u32 = 17;
const HTCLOSE: u32 = 20;
const GW_HWNDNEXT: u32 = 2;
const GWL_EXSTYLE: i32 = -20;
const WS_EX_TRANSPARENT: isize = 0x20;
const DWMWA_CLOAKED: u32 = 14;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
/// the system's visuals for a touch: tap, double tap, press and hold, right tap, press and tap
const FEEDBACKS: [u32; 5] = [7, 8, 9, 10, 11];

/// one sample of the pointer API: the POINTER_TOUCH_INFO fields the finger path reads
#[derive(Clone, Copy, Default)]
pub struct TouchInfo {
    pub flags: u32, // POINTER_INFO.pointerFlags
    pub x: i32,     // ptPixelLocation (screen px)
    pub y: i32,
    pub time: u32,  // dwTime (ms, 0: none)
    pub middle: Option<[i32; 2]>, // the middle of rcContact, if it has one (screen px)
}

/// the touch part of the pointer API (functions that are missing are None)
#[derive(Clone, Copy)]
pub struct TouchApi {
    /// GetPointerTouchInfo(id) -> the newest sample
    pub info: Option<fn(u32) -> Option<TouchInfo>>,
    /// GetPointerTouchInfoHistory(id, &count, out): newest first, at most *count records; *count =
    /// the samples available -> false: failed
    pub hist: Option<fn(u32, &mut u32, &mut [TouchInfo]) -> bool>,
    /// the clock of TouchInfo::time (GetTickCount)
    pub now: Option<fn() -> u32>,
}

const NO_API: TouchApi = TouchApi { info: None, hist: None, now: None };

#[derive(Clone, Copy, Default)]
struct Slot {
    used: bool,
    ptr: u32,    // the pointer id
    win: i32,    // the window it belongs to
    screen: [i32; 2],
    adj: [i32; 2], // what the position is off by (a contact given to a frame)
}

// a window that follows a finger
#[derive(Clone, Copy, Default)]
struct Drag {
    on: bool,
    slot: usize,
    win: i32,
    edges: u32,
    /// the finger, the window's rectangle and client origin when it started (screen px)
    start: [i32; 2],
    rect: [i32; 4],
    org: [i32; 2],
}

struct Touch {
    loaded: Cell<bool>,
    api: Cell<TouchApi>,
    slots: Cell<[Slot; SLOTS]>,
    drag: Cell<Drag>,
    /// a finger was the last pointer, the mouse cursor stood here when it came
    finger_mode: Cell<bool>,
    cursor: Cell<(i32, i32)>,
}

static TOUCH: UiOnly<Touch> = UiOnly::new(Touch {
    loaded: Cell::new(false),
    api: Cell::new(NO_API),
    slots: Cell::new([Slot { used: false, ptr: 0, win: 0, screen: [0; 2], adj: [0; 2] }; SLOTS]),
    drag: Cell::new(Drag { on: false, slot: 0, win: 0, edges: 0, start: [0; 2], rect: [0; 4], org: [0; 2] }),
    finger_mode: Cell::new(false),
    cursor: Cell::new((0, 0)),
});

// user32's functions (0 = missing)
static P_INFO: AtomicUsize = AtomicUsize::new(0);
static P_HIST: AtomicUsize = AtomicUsize::new(0);
static P_FEEDBACK: AtomicUsize = AtomicUsize::new(usize::MAX);
static LOG: AtomicUsize = AtomicUsize::new(usize::MAX);

fn touch() -> &'static Touch {
    TOUCH.get()
}

/// (test aid) the touch calls through `api` (None: user32's again, loaded on first use)
pub fn touch_api_set(api: Option<TouchApi>) {
    let t = touch();
    match api {
        Some(a) => {
            t.api.set(a);
            t.loaded.set(true);
        }
        None => t.loaded.set(false),
    }
    t.slots.set([Slot::default(); SLOTS]);
    t.drag.set(Drag::default());
    t.finger_mode.set(false);
}

fn touch_api() -> TouchApi {
    let t = touch();
    if !t.loaded.get() {
        t.loaded.set(true);
        let u = b"user32.dll\0";
        P_INFO.store(dll_proc(u, b"GetPointerTouchInfo\0"), Ordering::Relaxed);
        P_HIST.store(dll_proc(u, b"GetPointerTouchInfoHistory\0"), Ordering::Relaxed);
        let on = |a: &AtomicUsize| a.load(Ordering::Relaxed) != 0;
        t.api.set(TouchApi {
            info: if on(&P_INFO) { Some(real_info) } else { None },
            hist: if on(&P_HIST) { Some(real_hist) } else { None },
            now: Some(real_now),
        });
    }
    t.api.get()
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// the fields of a POINTER_TOUCH_INFO record
fn pti(b: &[u8]) -> TouchInfo {
    let r = [u32_at(b, 104) as i32, u32_at(b, 108) as i32, u32_at(b, 112) as i32, u32_at(b, 116) as i32];
    // (touchMask: TOUCH_MASK_CONTACTAREA)
    let middle = (u32_at(b, 100) & 1 != 0 && r[2] > r[0] && r[3] > r[1]).then(|| [r[0] / 2 + r[2] / 2, r[1] / 2 + r[3] / 2]);
    TouchInfo { flags: u32_at(b, 12), x: u32_at(b, 32) as i32, y: u32_at(b, 36) as i32, time: u32_at(b, 64), middle }
}

fn real_info(id: u32) -> Option<TouchInfo> {
    let mut b = [0u8; PTI_SIZE];
    // SAFETY: GetPointerTouchInfo(UINT32, POINTER_TOUCH_INFO*) -> BOOL; b holds one record.
    let f: unsafe extern "system" fn(u32, *mut u8) -> i32 = unsafe { core::mem::transmute(P_INFO.load(Ordering::Relaxed)) };
    if unsafe { f(id, b.as_mut_ptr()) } != 0 { Some(pti(&b)) } else { None }
}

fn real_hist(id: u32, count: &mut u32, out: &mut [TouchInfo]) -> bool {
    let room = (*count as usize).min(out.len()).min(TOUCH_HIST);
    let mut b = [0u8; TOUCH_HIST * PTI_SIZE];
    let mut c = room as u32;
    // SAFETY: GetPointerTouchInfoHistory(UINT32, UINT32*, POINTER_TOUCH_INFO*) -> BOOL; b holds
    // `room` records, the system writes at most *count of them.
    let f: unsafe extern "system" fn(u32, *mut u32, *mut u8) -> i32 =
        unsafe { core::mem::transmute(P_HIST.load(Ordering::Relaxed)) };
    if unsafe { f(id, &mut c, b.as_mut_ptr()) } == 0 {
        return false;
    }
    for (k, o) in out.iter_mut().enumerate().take(room.min(c as usize)) {
        *o = pti(&b[k * PTI_SIZE..(k + 1) * PTI_SIZE]);
    }
    *count = c;
    true
}

fn real_now() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetTickCount() }
}

/// no system visuals for a touch of this window: the UI answers a finger itself
pub(super) fn feedback_off(hwnd: isize) {
    let mut p = P_FEEDBACK.load(Ordering::Relaxed);
    if p == usize::MAX {
        p = dll_proc(b"user32.dll\0", b"SetWindowFeedbackSetting\0");
        P_FEEDBACK.store(p, Ordering::Relaxed);
    }
    if p == 0 {
        return;
    }
    let off = 0i32;
    // SAFETY: SetWindowFeedbackSetting(HWND, FEEDBACK_TYPE, DWORD, UINT32, const VOID*) -> BOOL.
    let f: unsafe extern "system" fn(isize, u32, u32, u32, *const i32) -> i32 = unsafe { core::mem::transmute(p) };
    for k in FEEDBACKS {
        unsafe { f(hwnd, k, 0, 4, &off) };
    }
}

fn find(id: u32) -> Option<usize> {
    touch().slots.get().iter().position(|s| s.used && s.ptr == id)
}

// a finger's event for its window
fn push(win: i32, phase: FingerPhase, slot: usize, x: i32, y: i32, age: u32) {
    let mut e = finger_event((win as u32 & (PW_MAX as u32 - 1)) as u8, phase, slot as u8, x, y, age);
    e.mods = win_mods();
    evq_push(e);
}

// how old a sample is now (ms; 0: unknown)
fn age_of(i: &TouchInfo) -> u32 {
    if i.time == 0 {
        return 0;
    }
    let now = touch_api().now.map_or(0, |f| f());
    let a = now.wrapping_sub(i.time);
    if a > 5000 { 0 } else { a }
}

// a screen point -> client x, y of window w
fn client(win: i32, sx: i32, sy: i32) -> (i32, i32) {
    let mut p = Point { x: sx, y: sy };
    if let Some(r) = rec(win) {
        // SAFETY: our window and a valid POINT.
        unsafe { ScreenToClient(r.hwnd.get(), &mut p) };
    }
    (p.x, p.y)
}

// jfgui_TOUCH_LOG=1: a finger came down on window `from` (ht: the hit test of the non-client message, 0
// in the client area) and goes to `to` (None: the system's)
fn log_down(id: u32, ht: u32, from: i32, to: Option<i32>, i: &TouchInfo) {
    let mut on = LOG.load(Ordering::Relaxed);
    if on == usize::MAX {
        on = crate::getenv("jfgui_TOUCH_LOG").is_some_and(|v| v.starts_with('1')) as usize;
        LOG.store(on, Ordering::Relaxed);
    }
    if on != 0 {
        let (x, y) = client(from, i.x, i.y);
        // the record as the system gave it: the position, the raw one, the contact, the raw contact
        let mut b = [0u8; PTI_SIZE];
        let f = P_INFO.load(Ordering::Relaxed);
        if f != 0 {
            // SAFETY: GetPointerTouchInfo(UINT32, POINTER_TOUCH_INFO*) -> BOOL; b holds one record.
            let f: unsafe extern "system" fn(u32, *mut u8) -> i32 = unsafe { core::mem::transmute(f) };
            unsafe { f(id, b.as_mut_ptr()) };
        }
        let v: alloc::vec::Vec<i32> = [32, 36, 48, 52, 104, 108, 112, 116, 120, 124, 128, 132].iter().map(|&o| u32_at(&b, o) as i32).collect();
        crate::write_stderr(alloc::format!("touch: down ht={ht} screen {},{} window {from} at {x},{y} -> {to:?} record {v:?}\n", i.x, i.y).as_bytes());
    }
}

// a new finger for window `win`; adj: what its position is off by
fn down(id: u32, win: i32, i: &TouchInfo, adj: [i32; 2]) -> bool {
    let t = touch();
    let mut slots = t.slots.get();
    if let Some(k) = slots.iter().position(|s| s.used && s.ptr == id) {
        // (it never lifted)
        slots[k].used = false;
        t.slots.set(slots);
        let (x, y) = client(slots[k].win, slots[k].screen[0], slots[k].screen[1]);
        push(slots[k].win, FingerPhase::Cancel, k, x, y, 0);
    }
    let Some(k) = slots.iter().position(|s| !s.used) else { return true }; // no room: nobody's
    let (sx, sy) = (i.x + adj[0], i.y + adj[1]);
    slots[k] = Slot { used: true, ptr: id, win, screen: [sx, sy], adj };
    t.slots.set(slots);
    // the mouse cursor stays where it is: what comes from there is no mouse
    t.finger_mode.set(true);
    let mut p = Point::default();
    // SAFETY: a valid POINT.
    if unsafe { GetCursorPos(&mut p) } != 0 {
        t.cursor.set((p.x, p.y));
    }
    let (x, y) = client(win, sx, sy);
    push(win, FingerPhase::Down, k, x, y, age_of(i));
    true
}

// a finger is gone
fn gone(k: usize, phase: FingerPhase, sample: Option<&TouchInfo>) {
    let t = touch();
    let mut slots = t.slots.get();
    let s = slots[k];
    slots[k].used = false;
    t.slots.set(slots);
    let mut d = t.drag.get();
    if d.on && d.slot == k {
        d.on = false;
        t.drag.set(d);
    }
    let (sx, sy) = sample.map_or((s.screen[0], s.screen[1]), |i| (i.x + s.adj[0], i.y + s.adj[1]));
    let (x, y) = client(s.win, sx, sy);
    push(s.win, phase, k, x, y, sample.map_or(0, age_of));
}

// a sample of a finger that is down
fn moved(k: usize, i: &TouchInfo) {
    let t = touch();
    let mut slots = t.slots.get();
    let (sx, sy) = (i.x + slots[k].adj[0], i.y + slots[k].adj[1]);
    slots[k].screen = [sx, sy];
    t.slots.set(slots);
    let d = t.drag.get();
    if d.on && d.slot == k {
        drag_to(&d, sx, sy);
        return;
    }
    let (x, y) = client(slots[k].win, sx, sy);
    push(slots[k].win, FingerPhase::Move, k, x, y, age_of(i));
}

/// a pointer message of a finger (PT_TOUCH) of window w -> handled
pub(super) fn pointer(hwnd: isize, msg: u32, id: u32, w: i32) -> bool {
    let api = touch_api();
    match msg {
        WM_POINTERDOWN => {
            let Some(i) = api.info.and_then(|f| f(id)) else { return false };
            let to = if client_has(hwnd, i.x, i.y) { w } else { window_under(hwnd, i.x, i.y).unwrap_or(w) };
            log_down(id, 0, w, Some(to), &i);
            down(id, to, &i, [0, 0])
        }
        WM_POINTERUPDATE | WM_NCPOINTERUPDATE => {
            let Some(k) = find(id) else { return false }; // (it began on the system's frame)
            update(&api, id, k);
            true
        }
        WM_POINTERUP | WM_NCPOINTERUP => {
            let Some(k) = find(id) else { return false };
            let i = api.info.and_then(|f| f(id));
            let cancel = i.is_some_and(|i| i.flags & PFL_CANCELED != 0);
            gone(k, if cancel { FingerPhase::Cancel } else { FingerPhase::Up }, i.as_ref());
            true
        }
        WM_POINTERCAPTURECHANGED => {
            let Some(k) = find(id) else { return false }; // (the system's: a move of the main window)
            gone(k, FingerPhase::Cancel, None);
            true
        }
        _ => true, // WM_POINTERENTER, WM_POINTERLEAVE: a finger has no hover
    }
}

// WM_POINTERUPDATE: every coalesced sample, oldest first
fn update(api: &TouchApi, id: u32, k: usize) {
    let mut buf = [TouchInfo::default(); TOUCH_HIST];
    let mut count = TOUCH_HIST as u32;
    let have = api.hist.is_some_and(|h| h(id, &mut count, &mut buf));
    if !have {
        let Some(i) = api.info.and_then(|f| f(id)) else { return };
        buf[0] = i;
        count = 1;
    }
    let n = (count as usize).min(TOUCH_HIST); // untrusted count
    if n == 0 {
        return;
    }
    if buf[0].flags & PFL_CANCELED != 0 {
        gone(k, FingerPhase::Cancel, Some(&buf[0]));
        return;
    }
    for q in (0..n).rev() {
        // (a screen that sees a finger above it: only contact counts)
        if buf[q].flags & PFL_INCONTACT != 0 {
            moved(k, &buf[q]);
        }
    }
}

/// a finger over the non-client area of window w -> handled (else on to DefWindowProc)
pub(super) fn nc_pointer(hwnd: isize, msg: u32, wp: usize, w: i32) -> bool {
    let id = wp as u16 as u32;
    let Some(ptype) = super::pen::pointer_type(id) else { return false };
    if ptype != super::pen::WIN_PT_TOUCH {
        return false;
    }
    if msg != WM_NCPOINTERDOWN {
        return pointer(hwnd, msg, id, w);
    }
    let api = touch_api();
    let Some(i) = api.info.and_then(|f| f(id)) else { return false };
    let ht = (wp >> 16) as u16 as u32;
    let adj = i.middle.map_or([0, 0], |m| [m[0] - i.x, m[1] - i.y]);
    let to = route(hwnd, w, ht, i.x + adj[0], i.y + adj[1]);
    log_down(id, ht, w, to, &i);
    match to {
        Some(to) => down(id, to, &i, adj),
        None => false,
    }
}

// the window a finger belongs to that came down on the non-client area (hit test code ht) of window w;
// None: the system's (it moves or resizes the window)
fn route(hwnd: isize, w: i32, ht: u32, sx: i32, sy: i32) -> Option<i32> {
    let top = rec(w).is_some_and(|r| r.flags.get() & WWF_TOP != 0);
    match ht {
        HTMINBUTTON | HTMAXBUTTON | HTCLOSE => Some(w),
        HTCAPTION => (!top || widget_near(w, sx, sy)).then_some(w),
        HTLEFT..=HTBOTTOMRIGHT => {
            let under = window_under(hwnd, sx, sy);
            if !top {
                return Some(under.unwrap_or(w));
            }
            let to = under?;
            // the main window's own band: what it is for, unless a small widget is close
            (to != w || widget_near(w, sx, sy)).then_some(to)
        }
        _ => None,
    }
}

// a widget (no title bar, no empty panel) right around a screen point of window w
fn widget_near(w: i32, sx: i32, sy: i32) -> bool {
    let (x, y) = client(w, sx, sy);
    let d = rec(w).map_or(8, |r| 8 * r.dpi.get().max(96) / 96);
    [(0, 0), (-d, 0), (d, 0), (0, -d), (0, d), (-d, -d), (d, -d), (-d, d), (d, d)].iter().any(|&(ox, oy)| {
        let f = ui_hit_at(w, x + ox, y + oy);
        f != 0 && f & (HB_DRAG | HB_BLOCK) == 0
    })
}

// a screen point lies in the client area of window h
fn client_has(h: isize, sx: i32, sy: i32) -> bool {
    let (mut o, mut c) = (Point::default(), Rect::default());
    // SAFETY: any handle is tolerated, valid out-parameters.
    unsafe {
        ClientToScreen(h, &mut o);
        GetClientRect(h, &mut c);
    }
    sx >= o.x && sx < o.x + c.r && sy >= o.y && sy < o.y + c.b
}

// another program's window that counts as visible and takes no touch: cloaked (a suspended store app, the
// input host) or WS_EX_TRANSPARENT
fn see_through(h: isize) -> bool {
    let mut cloaked = 0u32;
    // SAFETY: any handle is tolerated, a valid out-parameter of the size given.
    unsafe {
        DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked, 4) == 0 && cloaked != 0
            || super::GetWindowLongPtrW(h, GWL_EXSTYLE) & WS_EX_TRANSPARENT != 0
    }
}

// our window whose client area is under a screen point, from hwnd down the z-order; another program's
// window in between or none: None
fn window_under(hwnd: isize, sx: i32, sy: i32) -> Option<i32> {
    let s = st();
    let mut h = hwnd;
    for _ in 0..256 {
        if h == 0 {
            return None;
        }
        // SAFETY: GetWindow / IsWindowVisible / GetWindowRect tolerate any handle.
        if unsafe { IsWindowVisible(h) } != 0 {
            if let Some(k) = (0..PW_MAX).find(|&k| s.ww[k].hwnd.get() == h) {
                if client_has(h, sx, sy) {
                    return Some(k as i32);
                }
            } else if !see_through(h) {
                let mut r = Rect::default();
                // SAFETY: a valid RECT.
                unsafe { GetWindowRect(h, &mut r) };
                if sx >= r.l && sx < r.r && sy >= r.t && sy < r.b {
                    return None;
                }
            }
        }
        // SAFETY: as above.
        h = unsafe { GetWindow(h, GW_HWNDNEXT) };
    }
    None
}

/// WM_POINTERACTIVATE of a finger on window w: as WM_MOUSEACTIVATE, a tool window keeps the keyboard
/// where it is unless a text box is touched.  None: not a finger, or the system's answer
pub(super) fn activate(hwnd: isize, wp: usize, w: i32) -> Option<isize> {
    let id = wp as u16 as u32;
    if super::pen::pointer_type(id) != Some(super::pen::WIN_PT_TOUCH) {
        return None;
    }
    let f = rec(w)?.flags.get();
    if f & WWF_TOP != 0 || f & (PWF_TOOL | PWF_NOACTIVATE) == 0 {
        return None;
    }
    if f & PWF_NOACTIVATE != 0 {
        return Some(PA_NOACTIVATE);
    }
    let i = touch_api().info.and_then(|g| g(id))?;
    let mut p = Point { x: i.x, y: i.y };
    // SAFETY: our window and a valid POINT.
    unsafe { ScreenToClient(hwnd, &mut p) };
    Some(if ui_hit_at(w, p.x, p.y) & HB_TEXT != 0 { PA_ACTIVATE } else { PA_NOACTIVATE })
}

/// a finger is down on window w: from now on the window follows it (edges 0: moves, else the resize at
/// those edges) -> false: no finger, the mouse's way
pub(super) fn begin_drag(w: i32, edges: u32) -> bool {
    let t = touch();
    let Some(k) = t.slots.get().iter().position(|s| s.used && s.win == w) else { return false };
    let Some(r) = rec(w) else { return false };
    let (mut wr, mut o) = (Rect::default(), Point::default());
    // SAFETY: our window, valid out-parameters.
    unsafe {
        GetWindowRect(r.hwnd.get(), &mut wr);
        ClientToScreen(r.hwnd.get(), &mut o);
    }
    t.drag.set(Drag {
        on: true,
        slot: k,
        win: w,
        edges,
        start: t.slots.get()[k].screen,
        rect: [wr.l, wr.t, wr.r, wr.b],
        org: [o.x, o.y],
    });
    true
}

// the finger of a drag is at sx, sy: the window's new place, through the move hook (window_snap) as the
// system's move loop goes through WM_MOVING, and with its title bar in reach
fn drag_to(d: &Drag, sx: i32, sy: i32) {
    let Some(r) = rec(d.win) else { return };
    let h = r.hwnd.get();
    let (dx, dy) = (sx - d.start[0], sy - d.start[1]);
    let [l, t, ri, b] = d.rect;
    if d.edges != 0 {
        let (mut nl, mut nt, mut nr, mut nb) = (l, t, ri, b);
        if d.edges & EDGE_LEFT != 0 {
            nl += dx;
        }
        if d.edges & EDGE_RIGHT != 0 {
            nr += dx;
        }
        if d.edges & EDGE_TOP != 0 {
            nt += dy;
        }
        if d.edges & EDGE_BOTTOM != 0 {
            nb += dy;
        }
        // SAFETY: our window; the minimum size comes through WM_GETMINMAXINFO.
        unsafe { SetWindowPos(h, 0, nl, nt, nr - nl, nb - nt, SWP_NOZORDER | SWP_NOACTIVATE) };
        return;
    }
    let (mut x, mut y) = (d.org[0] + dx, d.org[1] + dy);
    let mut c = Rect::default();
    // SAFETY: our window, a valid RECT.
    unsafe { GetClientRect(h, &mut c) };
    if let Some(f) = st().moving.get() {
        let mut q = [x, y, c.r, c.b];
        f(d.win, &mut q);
        (x, y) = (q[0], q[1]);
    }
    if let Some(a) = win_work_area(d.win) {
        let p = reach_clamp([x - a[0], y - a[1]], c.r, [a[2], a[3]], r.dpi.get().max(96));
        (x, y) = (p[0] + a[0], p[1] + a[1]);
    }
    // the client origin -> the window's (its invisible border)
    let (wx, wy) = (l + x - d.org[0], t + y - d.org[1]);
    // SAFETY: our window.
    unsafe { SetWindowPos(h, 0, wx, wy, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
}

/// a WM_MOUSEMOVE while a finger was the last pointer: from where the cursor stood when the finger came
/// it is the system telling that a window changed under the (hidden) cursor, and it must not end the
/// finger mode.  One from anywhere else is a mouse
pub(super) fn mouse_is_stale() -> bool {
    let t = touch();
    if !t.finger_mode.get() {
        return false;
    }
    let mut p = Point::default();
    // SAFETY: a valid POINT.
    if unsafe { GetCursorPos(&mut p) } != 0 && t.cursor.get() == (p.x, p.y) {
        return true;
    }
    t.finger_mode.set(false);
    false
}

/// the main pointer of this machine is a finger right now: a tablet in slate posture
pub fn win_touch_primary() -> bool {
    const SM_MAXIMUMTOUCHES: i32 = 95;
    const SM_CONVERTIBLESLATEMODE: i32 = 0x2003;
    // SAFETY: GetSystemMetrics has no memory arguments.
    unsafe { GetSystemMetrics(SM_MAXIMUMTOUCHES) > 0 && GetSystemMetrics(SM_CONVERTIBLESLATEMODE) == 0 }
}

/// the on-screen keyboard for a text field that got a finger (show), or gone again.  InputPane through
/// its interop interface, which takes a desktop window (Windows 10 1809 and later; older ones and a
/// machine without one: nothing happens)
pub fn win_touch_keyboard(w: i32, show: bool) {
    use crate::win_winrt::{Guid, Obj, factory};
    let Some(hwnd) = rec(w).map(|r| r.hwnd.get()) else { return };
    static IID_INTEROP: Guid = Guid(0x75CF_2C57, 0x9195, 0x4931, [0x83, 0x32, 0xF0, 0xB4, 0x09, 0xE9, 0x16, 0xAF]);
    static IID_PANE2: Guid = Guid(0x8A6B_3F26, 0x7090, 0x4793, [0x94, 0x4C, 0xC3, 0xF2, 0xCD, 0xE2, 0x62, 0x76]);
    let Ok(interop) = factory("Windows.UI.ViewManagement.InputPane", &IID_INTEROP) else { return };
    let mut pane = 0usize;
    // SAFETY: IInputPaneInterop::GetForWindow(HWND, REFIID, void**) (IInspectable's slots + 0).
    let f: unsafe extern "system" fn(usize, isize, *const Guid, *mut usize) -> i32 = unsafe { core::mem::transmute(interop.vfn(6)) };
    if unsafe { f(interop.ptr(), hwnd, &IID_PANE2, &mut pane) } < 0 {
        return;
    }
    let Some(pane) = Obj::own(pane) else { return };
    let mut ok = 0u8;
    // SAFETY: IInputPane2::TryShow / TryHide(boolean*) (IInspectable's slots + 0, + 1).
    let g: unsafe extern "system" fn(usize, *mut u8) -> i32 = unsafe { core::mem::transmute(pane.vfn(if show { 6 } else { 7 })) };
    unsafe { g(pane.ptr(), &mut ok) };
}

