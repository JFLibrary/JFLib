// evq -- the platform event ring (shared by every platform and the script runner)
//                                                                        [src/plat/evq.s]
//
//   evq_push(ev) -> false when the ring is full (event dropped)
//   evq_pop() -> Some(event)        evq_count()        evq_clear()
//   evq_pen_data(pressure, type, tilt_x, tilt_y) -> u32   (PN1) Event::data of a pointer
//        event: pressure -1 = the device has none (-> 0), else clamped to 1..PRESSURE_ONE;
//        type PTR_*; tilts in degrees, clamped to -63..63.  0 for PTR_MOUSE and while pen
//        input is off.
//   pen_enable(on)    pen input on (default) / off: off = pens report plain mouse events
//   pen_enabled() -> bool
//
// The OS layer pushes from its message handlers, the UI core pops between frames (one
// thread; the lock only makes the static safe).

use core::sync::atomic::{AtomicBool, Ordering};

use rt::sync::SpinLock;

use crate::{Event, EVQ_CAP};

pub const PD_PRESSURE: u32 = 0xFFFF; // bits 0..15 pressure 1..PRESSURE_ONE, 0 = none (full)
pub const PD_TYPE_SHIFT: u32 = 16; // bits 16..17 PTR_*
pub const PD_TILTX_SHIFT: u32 = 18; // bits 18..24 i7 tilt x in degrees (-63..63, + = right)
pub const PD_TILTY_SHIFT: u32 = 25; // bits 25..31 i7 tilt y in degrees (+ = towards the user)
pub const PRESSURE_ONE: i32 = 65535;

struct Ring {
    ev: [Event; EVQ_CAP],
    r: u32,
    w: u32,
}

static RING: SpinLock<Ring> = SpinLock::new(Ring {
    ev: [Event { typ: 0, btn: 0, mods: 0, win: 0, x: 0, y: 0, data: 0 }; EVQ_CAP],
    r: 0,
    w: 0,
});
static PEN_OFF: AtomicBool = AtomicBool::new(false);

/// append an event; false when the ring is full (the event is dropped)
pub fn evq_push(e: Event) -> bool {
    let mut q = RING.lock();
    if q.w.wrapping_sub(q.r) as usize >= EVQ_CAP {
        return false;
    }
    let i = q.w as usize & (EVQ_CAP - 1);
    q.ev[i] = e;
    q.w = q.w.wrapping_add(1);
    true
}

/// the oldest queued event
pub fn evq_pop() -> Option<Event> {
    let mut q = RING.lock();
    if q.r == q.w {
        return None;
    }
    let e = q.ev[q.r as usize & (EVQ_CAP - 1)];
    q.r = q.r.wrapping_add(1);
    Some(e)
}

pub fn evq_count() -> u32 {
    let q = RING.lock();
    q.w.wrapping_sub(q.r)
}

pub fn evq_clear() {
    let mut q = RING.lock();
    q.r = q.w;
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
