// plock -- (G7a) the pointer lock of a window and the raw relative motion while it holds it,
// the part every platform shares
//
//   win_pointer_lock(w, on) -> PointerLock    lock the pointer to window w (on) or let it go;
//                                             the state right after the call
//   win_pointer_lock_state(w) -> PointerLock  the state now
//   win_raw_motion_take(w) -> (dx, dy)        the raw motion since the last call (0, 0 when none
//                                             or w does not hold the lock)
//   E_POINTER_LOCK                            the state changed (data 1: locked now, 0: not)
//   E_RAW_MOTION                              raw motion waits (one event until it is taken)
//
// While a window holds the lock the pointer is hidden, stays where it is (or inside the window)
// and its motion comes raw: without the system's acceleration where the platform has it, in the
// device's counts.  Per platform:
//   Windows  Raw Input (WM_INPUT of usage page 1, usage 2), the cursor clipped to the middle of
//            the window (ClipCursor) and hidden; motion in mouse counts; absolute devices (a
//            remote desktop, a tablet, some virtual machines) as differences of their positions
//            in pixels (win_plock.rs)
//   Wayland  zwp_pointer_constraints_v1.lock_pointer and zwp_relative_pointer_v1's
//            unaccelerated motion; Unsupported when the compositor lacks one of them
//            (linux_wl_lock.rs)
//   X11      XInput 2.1 raw motion (XI_RawMotion: the device's values before acceleration) and a
//            pointer grab confined to the window with an invisible cursor; Unsupported without
//            XInput 2, Refused when another client holds a grab (linux_x11_lock.rs)
//   macOS    CGAssociateMouseAndMouseCursorPosition(false), the cursor hidden; motion from the
//            events' deltaX / deltaY (points, after the system's acceleration) (apple_mac.rs)
//   browser  requestPointerLock on the canvas (unadjustedMovement where the browser has it:
//            raw; else the browser's accelerated CSS pixels); the browser grants it only from a
//            user's gesture: Pending until the next press in the page (web_windows.rs, loader.js)
//   Android, iOS  Unsupported (touch controls are a later stage)
//
// The lock always lets go: Esc (the key goes to the lock, not to the app; the browser owns Esc
// itself), the window losing the keyboard focus, being minimized or closed, and
// win_pointer_lock(w, false).  It is never taken back by itself: after a release the app asks
// again (a click "to play").  A request while the window has no focus waits (Pending) and takes
// effect when it gets the focus.  One window holds the lock at a time (there is one pointer):
// asking for another lets the first go.

use rt::sync::SpinLock;

use crate::{E_POINTER_LOCK, E_RAW_MOTION, Event, evq_push};

/// (G7a) the state of a window's pointer lock (win_pointer_lock, ui.pointer_lock)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PointerLock {
    /// not locked: never asked for, or let go (by the app, Esc, the focus going elsewhere, the
    /// window minimized or closed)
    #[default]
    Off,
    /// asked for, not granted yet: the window has no focus yet, the browser waits for the next
    /// press in the page, the Wayland compositor has not confirmed it
    Pending,
    /// locked: the pointer is hidden and stays put, win_raw_motion_take has its motion
    Locked,
    /// this platform or display cannot lock the pointer: Android, iOS; a Wayland compositor
    /// without zwp_pointer_constraints_v1 or zwp_relative_pointer_manager_v1, an X server without
    /// XInput 2.1
    Unsupported,
    /// the system said no: the browser's pointerlockerror, another X11 client holds a grab, no
    /// such window
    Refused,
}

struct Lk {
    /// the window that asked (-1: none)
    win: i32,
    state: PointerLock,
    motion: [f64; 2],
    /// an E_RAW_MOTION waits for win_raw_motion_take
    queued: bool,
    /// the Esc that let the lock go: its up goes to the lock too
    esc_up: bool,
}

static LK: SpinLock<Lk> = SpinLock::new(Lk { win: -1, state: PointerLock::Off, motion: [0.0; 2], queued: false, esc_up: false });

/// (G7a) lock the pointer to window w (on) or let it go (off) -> the state right after the call:
/// Locked at once where the platform grants it, Pending while it waits (see PointerLock), Off
/// after a release.  E_POINTER_LOCK follows every change.  The pointer is hidden while locked and
/// its motion comes through win_raw_motion_take; Esc, the focus going elsewhere, minimizing and
/// closing the window always let it go.  A window that is not the one holding (or waiting for)
/// the lock lets nothing go with off
pub fn win_pointer_lock(w: i32, on: bool) -> PointerLock {
    if !on && plock_win() != w {
        return win_pointer_lock_state(w);
    }
    #[cfg(windows)]
    return crate::win_plock_set(w, on);
    #[cfg(target_os = "linux")]
    return crate::disp_pointer_lock(w, on);
    #[cfg(target_os = "macos")]
    return crate::mac_pointer_lock(w, on);
    #[cfg(target_arch = "wasm32")]
    return crate::web::web_pointer_lock(w, on);
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = w;
        return if on { PointerLock::Unsupported } else { PointerLock::Off };
    }
}

/// (G7a) the state of window w's pointer lock (Off for a window that neither holds nor waits for
/// it; Unsupported / Refused stay until the next request)
pub fn win_pointer_lock_state(w: i32) -> PointerLock {
    let l = LK.lock();
    if l.win == w { l.state } else { PointerLock::Off }
}

/// (G7a) the raw relative motion of window w's pointer since the last call, while w holds the
/// lock: (dx, dy), right and down positive, in the device's counts where the platform gives them
/// (Windows' Raw Input, Wayland's unaccelerated motion, X11's raw values, the browser with
/// unadjustedMovement), else in its accelerated pixels (macOS' deltas, the browser without it);
/// (0, 0) when there is none.  E_RAW_MOTION says that some waits
pub fn win_raw_motion_take(w: i32) -> (f64, f64) {
    let mut l = LK.lock();
    if l.win != w {
        return (0.0, 0.0);
    }
    l.queued = false;
    let m = core::mem::take(&mut l.motion);
    (m[0], m[1])
}

// ------------------------------------------------------------------ the backends' side

/// (backends) the window that holds or waits for the lock (-1: none)
pub(crate) fn plock_win() -> i32 {
    LK.lock().win
}

/// (backends) the state of the lock (whichever window)
#[allow(dead_code)]
pub(crate) fn plock_state() -> PointerLock {
    LK.lock().state
}

/// (backends) the lock of window w is in state s now: E_POINTER_LOCK when it changed; the motion
/// that waited goes when the lock is not held any more
#[allow(dead_code)]
pub(crate) fn plock_set(w: i32, s: PointerLock) {
    let mut l = LK.lock();
    if l.win == w && l.state == s {
        return;
    }
    let old = if l.win >= 0 { Some((l.win, l.state)) } else { None };
    l.win = w;
    l.state = s;
    if s != PointerLock::Locked {
        l.motion = [0.0; 2];
        l.queued = false;
    }
    drop(l);
    // another window that held it: its change first
    if let Some((ow, os)) = old {
        if ow != w && os != PointerLock::Off {
            evq_push(Event { typ: E_POINTER_LOCK, win: (ow & 7) as u8, ..Event::default() });
        }
    }
    evq_push(Event { typ: E_POINTER_LOCK, win: (w & 7) as u8, data: (s == PointerLock::Locked) as u32, ..Event::default() });
}

/// (backends) raw motion of window w's pointer (right and down positive): added while w holds the
/// lock, an E_RAW_MOTION for the first since the last take
#[allow(dead_code)]
pub(crate) fn plock_motion(w: i32, dx: f64, dy: f64) {
    if !(dx.is_finite() && dy.is_finite()) || (dx == 0.0 && dy == 0.0) {
        return;
    }
    let mut l = LK.lock();
    if l.win != w || l.state != PointerLock::Locked {
        return;
    }
    l.motion[0] += dx;
    l.motion[1] += dy;
    if l.queued {
        return;
    }
    l.queued = true;
    drop(l);
    evq_push(Event { typ: E_RAW_MOTION, win: (w & 7) as u8, ..Event::default() });
}

/// (backends) a key of any of the app's windows goes down or up (vk: VK_*) -> true: it belongs to
/// the lock and must not reach the app.  Esc going down while a window holds or waits for the
/// lock: the backend lets the lock go (its release, which says Off); its up is the lock's too
#[allow(dead_code)]
pub(crate) fn plock_key(vk: u32, down: bool) -> bool {
    if vk & 0xFF != 0x1B {
        return false;
    }
    let mut l = LK.lock();
    if !down {
        return core::mem::take(&mut l.esc_up);
    }
    // (the auto-repeat of the Esc that let it go: the lock's too)
    if l.esc_up || (l.win >= 0 && matches!(l.state, PointerLock::Locked | PointerLock::Pending)) {
        l.esc_up = true;
        return true;
    }
    false
}

/// (test aid, plattest --test input) the shared state without a platform: window w's lock is in
/// state s now, as a backend says it (E_POINTER_LOCK on a change)
pub fn pointer_lock_test(w: i32, s: PointerLock) {
    plock_set(w, s)
}

/// (test aid) raw motion of window w as a backend reports it
pub fn raw_motion_test(w: i32, dx: f64, dy: f64) {
    plock_motion(w, dx, dy)
}

/// (test aid) a key as a backend asks the lock about it -> true: the key is the lock's (Esc)
pub fn pointer_lock_key_test(vk: u32, down: bool) -> bool {
    plock_key(vk, down)
}

// ------------------------------------------------------------------ decoding (every target: tests)

/// (G7a) the motion of a Windows RAWINPUT record of a mouse (the bytes GetRawInputData fills: the
/// 64-bit RAWINPUTHEADER, then RAWMOUSE) -> (usFlags, lLastX, lLastY); None for another device or
/// a short record.  MOUSE_MOVE_ABSOLUTE (usFlags bit 0) gives positions 0..65535 instead of
/// motion (MOUSE_VIRTUAL_DESKTOP, bit 1: of the whole desktop)
pub fn raw_mouse_decode(rec: &[u8]) -> Option<(u16, i32, i32)> {
    let u32_at = |o: usize| rec.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if u32_at(0)? != 0 {
        return None; // dwType: RIM_TYPEMOUSE
    }
    if (u32_at(4)? as usize) < RAW_HDR + 24 || rec.len() < RAW_HDR + 24 {
        return None;
    }
    let m = &rec[RAW_HDR..];
    let flags = u16::from_le_bytes([m[0], m[1]]);
    let x = i32::from_le_bytes([m[12], m[13], m[14], m[15]]);
    let y = i32::from_le_bytes([m[16], m[17], m[18], m[19]]);
    Some((flags, x, y))
}

/// RAWINPUTHEADER on 64-bit Windows: dwType, dwSize, hDevice, wParam
pub(crate) const RAW_HDR: usize = 24;

/// (G7a) the motion of an absolute pointer (a remote desktop, a tablet, a virtual machine) between
/// two of its positions (0..65535 across the screen or the desktop of `span` pixels) -> pixels;
/// the first position after a pause (prev None) moves nothing
pub fn raw_abs_delta(prev: Option<[i32; 2]>, now: [i32; 2], span: [i32; 2]) -> (f64, f64) {
    match prev {
        None => (0.0, 0.0),
        Some(p) => (
            (now[0] - p[0]) as f64 * span[0].max(1) as f64 / 65535.0,
            (now[1] - p[1]) as f64 * span[1].max(1) as f64 / 65535.0,
        ),
    }
}

/// (G7a) the raw motion of an XInput 2 raw event (XI_RawMotion, the whole GenericEvent as the
/// server sends it) -> (deviceid, sourceid, time, raw value of valuator 0, of valuator 1, which
/// of the two it carries: bit 0 / bit 1); None for a short or malformed event.  The raw values
/// (FP3232, before acceleration) follow the mask and the transformed values
pub fn xi_raw_decode(ev: &[u8]) -> Option<(u16, u16, u32, f64, f64, u32)> {
    if ev.len() < 32 {
        return None;
    }
    let rd16 = |o: usize| u16::from_le_bytes([ev[o], ev[o + 1]]);
    let rd32 = |o: usize| u32::from_le_bytes([ev[o], ev[o + 1], ev[o + 2], ev[o + 3]]);
    let extra = rd32(4) as usize;
    if extra > 1 << 16 {
        return None;
    }
    let end = (32 + extra * 4).min(ev.len());
    // xXIRawEvent: evtype 8, deviceid 10, time 12, detail 16, sourceid 20, valuators_len 22, flags 24
    let (dev, src, time) = (rd16(10), rd16(20), rd32(12));
    let mlen = rd16(22) as usize; // valuators_len: mask words
    let vals = 32 + mlen * 4;
    if vals > end {
        return None;
    }
    let mut n = 0usize; // set bits
    for w in 0..mlen {
        n += rd32(32 + w * 4).count_ones() as usize;
    }
    let raw = vals + n * 8; // after the transformed values
    if raw + n * 8 > end {
        return None;
    }
    let (mut v, mut have, mut k) = ([0.0f64; 2], 0u32, 0usize);
    for bit in 0..(mlen * 32).min(64) {
        if rd32(32 + (bit >> 5) * 4) & 1 << (bit & 31) == 0 {
            continue;
        }
        if bit < 2 {
            let o = raw + k * 8;
            v[bit] = rd32(o) as i32 as f64 + rd32(o + 4) as f64 / 4_294_967_296.0;
            have |= 1 << bit;
        }
        k += 1;
    }
    Some((dev, src, time, v[0], v[1], have))
}

/// (G7a) Wayland's fixed-point value (24.8) as a number
pub fn wl_fixed(v: i32) -> f64 {
    v as f64 / 256.0
}
