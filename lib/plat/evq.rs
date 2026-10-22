// evq -- the platform event ring (shared by every platform and the script runner)
//                                                                        [src/plat/evq.s]
//
//   evq_push(ev) -> false when the event was dropped (ring and overflow full)
//   evq_pop() -> Some(event)        evq_count()        evq_clear()
//   evq_dropped() -> events dropped so far (tests)
//   evq_pen_data(pressure, type, tilt_x, tilt_y) -> u32   (PN1) Event::data of a pointer
//        event: pressure -1 = the device has none (-> 0), else clamped to 1..PRESSURE_ONE;
//        type PTR_*; tilts in degrees, clamped to -63..63.  0 for PTR_MOUSE and while pen
//        input is off.
//   pen_enable(on)    pen input on (default) / off: off = pens report plain mouse events
//   pen_enabled() -> bool
//
// The OS layer pushes from its message handlers, the UI core pops between frames (one
// thread; the lock only makes the static safe).
//
// A burst larger than the ring (one socket read of synthetic keys on X11 / Wayland, a client
// flooding input: their dispatch must read on to an awaited reply, so it cannot stop at a
// full ring) waits in a second-level queue on the heap, in order; the ring takes from it as
// it drains.  Only beyond EVQ_MORE_MAX is an event dropped: counted (evq_dropped) and
// logged once per burst.  Windows keeps WM_CHAR bursts in its own message queue instead
// (wait() stops pumping at EVQ_CAP - 64).

use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicBool, Ordering};

use rt::sync::SpinLock;

use crate::{Event, EVQ_CAP};

/// events waiting behind a full ring at most (16 bytes each: 1 MiB)
const EVQ_MORE_MAX: usize = 1 << 16;

pub const PD_PRESSURE: u32 = 0xFFFF; // bits 0..15 pressure 1..PRESSURE_ONE, 0 = none (full)
pub const PD_TYPE_SHIFT: u32 = 16; // bits 16..17 PTR_*
pub const PD_TILTX_SHIFT: u32 = 18; // bits 18..24 i7 tilt x in degrees (-63..63, + = right)
pub const PD_TILTY_SHIFT: u32 = 25; // bits 25..31 i7 tilt y in degrees (+ = towards the user)
pub const PRESSURE_ONE: i32 = 65535;

struct Ring {
    ev: [Event; EVQ_CAP],
    r: u32,
    w: u32,
    /// the newer events of a burst while the ring is full (never used otherwise)
    more: VecDeque<Event>,
    /// events dropped beyond EVQ_MORE_MAX; this burst was logged
    dropped: u32,
    logged: bool,
}

static RING: SpinLock<Ring> = SpinLock::new(Ring {
    ev: [Event { typ: 0, btn: 0, mods: 0, win: 0, x: 0, y: 0, data: 0 }; EVQ_CAP],
    r: 0,
    w: 0,
    more: VecDeque::new(),
    dropped: 0,
    logged: false,
});
static PEN_OFF: AtomicBool = AtomicBool::new(false);

/// append an event (behind a full ring: the overflow queue) -> false when it was dropped
/// (more than EVQ_MORE_MAX events wait)
pub fn evq_push(e: Event) -> bool {
    let mut q = RING.lock();
    if q.more.is_empty() && (q.w.wrapping_sub(q.r) as usize) < EVQ_CAP {
        let i = q.w as usize & (EVQ_CAP - 1);
        q.ev[i] = e;
        q.w = q.w.wrapping_add(1);
        return true;
    }
    if q.more.len() < EVQ_MORE_MAX {
        q.more.push_back(e);
        return true;
    }
    q.dropped = q.dropped.wrapping_add(1);
    let first = !q.logged;
    q.logged = true;
    drop(q);
    if first {
        crate::log_warn("plat", "event queue full: more than 65536 events wait, input is dropped");
    }
    false
}

/// the oldest queued event
pub fn evq_pop() -> Option<Event> {
    let mut q = RING.lock();
    if q.r == q.w {
        return None; // (the overflow queue only fills behind a full ring)
    }
    let e = q.ev[q.r as usize & (EVQ_CAP - 1)];
    q.r = q.r.wrapping_add(1);
    if let Some(m) = q.more.pop_front() {
        // the oldest waiting event takes the free place
        let i = q.w as usize & (EVQ_CAP - 1);
        q.ev[i] = m;
        q.w = q.w.wrapping_add(1);
        if q.more.is_empty() {
            q.logged = false;
            if q.more.capacity() > EVQ_CAP {
                q.more = VecDeque::new(); // a big burst's memory goes with it
            }
        }
    }
    Some(e)
}

/// events queued (the ring and the overflow queue)
pub fn evq_count() -> u32 {
    let q = RING.lock();
    q.w.wrapping_sub(q.r).wrapping_add(q.more.len() as u32)
}

pub fn evq_clear() {
    let mut q = RING.lock();
    q.r = q.w;
    q.more = VecDeque::new();
    q.logged = false;
}

/// events dropped since the start because more than EVQ_MORE_MAX waited (tests)
pub fn evq_dropped() -> u32 {
    RING.lock().dropped
}

/// Event::data of a pen pointer event (0 for a mouse or while pen input is off)
pub fn evq_pen_data(pressure: i32, typ: u32, tilt_x: i32, tilt_y: i32) -> u32 {
    if PEN_OFF.load(Ordering::Relaxed) {
        return 0;
    }
    let t = typ & 3;
    if t == 0 {
        return 0; // PTR_MOUSE: no data
    }
    let mut v = 0u32;
    if pressure >= 0 {
        v = pressure.clamp(1, PRESSURE_ONE) as u32;
    }
    v |= t << PD_TYPE_SHIFT;
    v |= ((tilt_x.clamp(-63, 63) & 127) as u32) << PD_TILTX_SHIFT;
    v |= ((tilt_y.clamp(-63, 63) & 127) as u32) << PD_TILTY_SHIFT;
    v
}

/// pen input on (default) / off (pens become plain mouse events)
pub fn pen_enable(on: bool) {
    PEN_OFF.store(!on, Ordering::Relaxed);
}

pub fn pen_enabled() -> bool {
    !PEN_OFF.load(Ordering::Relaxed)
}
