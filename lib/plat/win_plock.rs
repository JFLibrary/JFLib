// win_plock -- (G7a) the pointer lock on Windows (the shared part: plock.rs)
//
//   win_plock_set(w, on) -> PointerLock    lock the pointer to window w / let it go
//   (window procedure, win_api.rs)
//     input(w, lparam)        WM_INPUT of the mouse: raw motion -> plock_motion
//     focus_in(w)             WM_SETFOCUS: a request that waited (Pending) takes effect
//     release(w)              WM_KILLFOCUS, minimized, WM_DESTROY, Esc: the lock goes
//     follow(w)               WM_MOVE / WM_SIZE: the clip follows the window
//     locked(w) -> bool       WM_SETCURSOR (no cursor), WM_NCHITTEST (all client: a hidden
//                             pointer never drags the title bar), win_set_cursor (kept for later)
//
// Locked: the mouse's Raw Input (usage page 1, usage 2) goes to the window (WM_INPUT, only while
// it is in the foreground: no RIDEV_INPUTSINK), the cursor is clipped to one pixel in the middle
// of the client area (ClipCursor; again after every move and size) and hidden (WM_SETCURSOR sets
// none).  The legacy mouse messages stay (no RIDEV_NOLEGACY): buttons and the wheel come as
// before.  Relative devices give counts (lLastX / lLastY); absolute ones (MOUSE_MOVE_ABSOLUTE: a
// remote desktop, a tablet, a virtual machine's pointer) positions 0..65535 of the primary
// monitor or the whole desktop (MOUSE_VIRTUAL_DESKTOP), whose differences are the motion in
// pixels.  The release unregisters the device (RIDEV_REMOVE), lifts the clip and puts the
// cursor back where it was before the lock.

use core::cell::Cell;

use super::{ClientToScreen, GetClientRect, GetSystemMetrics, IsIconic, Point, Rect, SetCursor, UiOnly, cur_handle, rec, st};
use crate::{PointerLock, plock_motion, plock_set, plock_state, plock_win, raw_abs_delta, raw_mouse_decode};

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterRawInputDevices(dev: *const RawDev, n: u32, size: u32) -> i32;
    fn GetRawInputData(h: isize, cmd: u32, data: *mut u8, size: *mut u32, hdr: u32) -> u32;
    fn ClipCursor(r: *const Rect) -> i32;
    fn GetCursorPos(p: *mut Point) -> i32;
    fn SetCursorPos(x: i32, y: i32) -> i32;
    fn GetForegroundWindow() -> isize;
}

/// RAWINPUTDEVICE
#[repr(C)]
struct RawDev {
    page: u16,
    usage: u16,
    flags: u32,
    target: isize,
}

const RIDEV_REMOVE: u32 = 0x1;
const RID_INPUT: u32 = 0x1000_0003;
const MOUSE_MOVE_ABSOLUTE: u16 = 1;
const MOUSE_VIRTUAL_DESKTOP: u16 = 2;
const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const SM_CXVIRTUALSCREEN: i32 = 78;
const SM_CYVIRTUALSCREEN: i32 = 79;

struct Wk {
    /// Raw Input registered, the cursor clipped and hidden
    on: Cell<bool>,
    /// where the cursor was before (screen px)
    saved: Cell<Point>,
    /// the last position of an absolute device (None: the next one starts)
    abs: Cell<Option<[i32; 2]>>,
}

static WK: UiOnly<Wk> = UiOnly::new(Wk { on: Cell::new(false), saved: Cell::new(Point { x: 0, y: 0 }), abs: Cell::new(None) });

/// (plock.rs) lock the pointer to window w (on) or let it go -> the state now
pub fn win_plock_set(w: i32, on: bool) -> PointerLock {
    if !on {
        release(w);
        return PointerLock::Off;
    }
    let Some(r) = rec(w) else { return PointerLock::Refused };
    let cur = plock_win();
    if cur >= 0 && cur != w {
        release(cur); // one window at a time
    }
    if cur == w && plock_state() == PointerLock::Locked {
        return PointerLock::Locked;
    }
    // SAFETY: plain queries of our window.
    let focused = unsafe { GetForegroundWindow() == r.hwnd.get() && IsIconic(r.hwnd.get()) == 0 };
    if focused {
        engage(w);
    } else {
        plock_set(w, PointerLock::Pending); // takes effect with the focus (focus_in)
    }
    plock_state()
}

/// (window procedure) window w holds the lock now
pub(super) fn locked(w: i32) -> bool {
    WK.get().on.get() && plock_win() == w
}

// the lock takes effect: Raw Input to the window, the clip, no cursor
fn engage(w: i32) {
    let Some(r) = rec(w) else { return };
    let hwnd = r.hwnd.get();
    let dev = RawDev { page: 1, usage: 2, flags: 0, target: hwnd };
    // SAFETY: one RAWINPUTDEVICE of its own size.
    if unsafe { RegisterRawInputDevices(&dev, 1, core::mem::size_of::<RawDev>() as u32) } == 0 {
        crate::log_warn("plat", "pointer lock: RegisterRawInputDevices failed");
        plock_set(w, PointerLock::Refused);
        return;
    }
    let k = WK.get();
    let mut p = Point { x: 0, y: 0 };
    // SAFETY: a valid POINT.
    if unsafe { GetCursorPos(&mut p) } != 0 {
        k.saved.set(p);
    }
    k.on.set(true);
    k.abs.set(None);
    plock_set(w, PointerLock::Locked);
    follow(w);
    // SAFETY: no cursor while locked (WM_SETCURSOR keeps it so).
    unsafe { SetCursor(0) };
}

/// (window procedure) WM_MOVE / WM_SIZE of window w: the clip stays in the middle of its client area
pub(super) fn follow(w: i32) {
    if !locked(w) {
        return;
    }
    let Some(r) = rec(w) else { return };
    let hwnd = r.hwnd.get();
    let (mut c, mut o) = (Rect::default(), Point::default());
    // SAFETY: our window, valid out-parameters; ClipCursor reads the RECT.
    unsafe {
        GetClientRect(hwnd, &mut c);
        ClientToScreen(hwnd, &mut o);
        let (x, y) = (o.x + c.r / 2, o.y + c.b / 2);
        ClipCursor(&Rect { l: x, t: y, r: x + 1, b: y + 1 });
    }
}

/// (window procedure) WM_SETFOCUS of window w: the request that waited for it takes effect
pub(super) fn focus_in(w: i32) {
    if plock_win() == w && plock_state() == PointerLock::Pending && !WK.get().on.get() {
        let Some(r) = rec(w) else { return };
        // SAFETY: our window.
        if unsafe { IsIconic(r.hwnd.get()) } == 0 {
            engage(w);
        }
    }
}

/// (window procedure, win_plock_set) window w lets the lock go (focus lost, minimized, closed,
/// Esc, the app): Raw Input off, no clip, the cursor back where it was and shown again
pub(super) fn release(w: i32) {
    if plock_win() != w {
        return;
    }
    let k = WK.get();
    if k.on.replace(false) {
        let dev = RawDev { page: 1, usage: 2, flags: RIDEV_REMOVE, target: 0 };
        let p = k.saved.get();
        // SAFETY: RIDEV_REMOVE takes no window; a null RECT lifts the clip.
        unsafe {
            RegisterRawInputDevices(&dev, 1, core::mem::size_of::<RawDev>() as u32);
            ClipCursor(core::ptr::null());
            SetCursorPos(p.x, p.y);
        }
        if let Some(r) = rec(w) {
            if st().ptr_win.get() == w {
                // SAFETY: the window's cursor again.
                unsafe { SetCursor(cur_handle(r.cursor.get(), r.dpi.get())) };
            }
        }
    }
    let s = plock_state();
    if s != PointerLock::Off {
        plock_set(w, PointerLock::Off);
    }
}

/// (window procedure) WM_INPUT (RIM_INPUT) of window w: the mouse's raw record -> motion
pub(super) fn input(w: i32, lp: isize) {
    if !locked(w) {
        return;
    }
    let mut buf = [0u8; 64];
    let mut size = buf.len() as u32;
    // SAFETY: the record of this WM_INPUT into a buffer of `size` bytes (the header's size: x64).
    let n = unsafe { GetRawInputData(lp, RID_INPUT, buf.as_mut_ptr(), &mut size, crate::plock::RAW_HDR as u32) };
    if n == u32::MAX || n as usize > buf.len() {
        return;
    }
    let Some((flags, x, y)) = raw_mouse_decode(&buf[..n as usize]) else { return };
    let k = WK.get();
    if flags & MOUSE_MOVE_ABSOLUTE != 0 {
        if x == 0 && y == 0 {
            return; // (a button's record without a position)
        }
        // SAFETY: plain metrics.
        let span = unsafe {
            if flags & MOUSE_VIRTUAL_DESKTOP != 0 {
                [GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)]
            } else {
                [GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)]
            }
        };
        let (dx, dy) = raw_abs_delta(k.abs.replace(Some([x, y])), [x, y], span);
        plock_motion(w, dx, dy);
    } else {
        plock_motion(w, x as f64, y as f64);
    }
}
