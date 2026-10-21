// win_power -- (power.rs) inhibitions on Windows: one power request per guard
//
//   hold(reason, what) -> Option<Held>   PowerCreateRequest with the reason as a simple string,
//                                        PowerSetRequest(PowerRequestSystemRequired, and
//                                        PowerRequestDisplayRequired for INHIBIT_IDLE)
//   release(&mut Held)                   PowerClearRequest for each, CloseHandle

use alloc::vec::Vec;

use super::INHIBIT_IDLE;

/// REASON_CONTEXT (x64, 32 bytes): version, flags, then the union (the simple string's
/// pointer first; the detailed form is 24 bytes)
#[repr(C)]
struct ReasonContext {
    version: u32,
    flags: u32,
    reason: *const u16,
    rest: [usize; 2],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn PowerCreateRequest(ctx: *const ReasonContext) -> isize;
    fn PowerSetRequest(h: isize, typ: u32) -> i32;
    fn PowerClearRequest(h: isize, typ: u32) -> i32;
    fn CloseHandle(h: isize) -> i32;
}

const POWER_REQUEST_CONTEXT_SIMPLE_STRING: u32 = 1;
const DISPLAY_REQUIRED: u32 = 0;
const SYSTEM_REQUIRED: u32 = 1;

/// an open power request and the request types set on it
pub struct Held {
    h: isize,
    set: Vec<u32>,
    _reason: Vec<u16>,
}

pub fn hold(reason: &str, what: u32) -> Option<Held> {
    let w: Vec<u16> = reason.encode_utf16().chain(core::iter::once(0)).collect();
    let ctx = ReasonContext { version: 0, flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING, reason: w.as_ptr(), rest: [0; 2] };
    // SAFETY: a valid REASON_CONTEXT whose string lives during the call (and in Held).
    let h = unsafe { PowerCreateRequest(&ctx) };
    if h == 0 || h == -1 {
        return None;
    }
    let mut held = Held { h, set: Vec::new(), _reason: w };
    let types: &[u32] = if what & INHIBIT_IDLE != 0 { &[SYSTEM_REQUIRED, DISPLAY_REQUIRED] } else { &[SYSTEM_REQUIRED] };
    for &t in types {
        // SAFETY: our request handle.
        if unsafe { PowerSetRequest(h, t) } == 0 {
            release(&mut held);
            return None;
        }
        held.set.push(t);
    }
    Some(held)
}

pub fn release(held: &mut Held) {
    if held.h == 0 {
        return;
    }
    for &t in &held.set {
        // SAFETY: a request type set on our handle.
        unsafe { PowerClearRequest(held.h, t) };
    }
    held.set.clear();
    // SAFETY: our request handle, closed once.
    unsafe { CloseHandle(held.h) };
    held.h = 0;
}
