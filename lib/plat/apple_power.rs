// apple_power -- (power.rs) keep a Mac awake: IOKit power assertions
//
//   hold(reason, what) -> Some(Held)   INHIBIT_SUSPEND: PreventUserIdleSystemSleep; INHIBIT_IDLE:
//                                      PreventUserIdleDisplaySleep as well (the screen stays on,
//                                      no screen saver).  The reason is what `pmset -g assertions`
//                                      and Activity Monitor show ("" names the app).
//   release(held)                      IOPMAssertionRelease of each
//
// macOS only (iOS keeps power.rs's None until AP5: UIApplication's idle timer).

use alloc::string::String;

use crate::{INHIBIT_IDLE, INHIBIT_SUSPEND};

type CfRef = *const core::ffi::c_void;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(ty: CfRef, level: u32, name: CfRef, id: *mut u32) -> i32;
    fn IOPMAssertionRelease(id: u32) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithBytes(alloc: CfRef, bytes: *const u8, n: isize, encoding: u32, external: u8) -> CfRef;
    fn CFRelease(r: CfRef);
}

const LEVEL_ON: u32 = 255; // kIOPMAssertionLevelOn
const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8

/// the assertions one guard holds (0: none)
pub struct Held {
    system: u32,
    display: u32,
}

fn cfstr(s: &str) -> CfRef {
    // SAFETY: the bytes of s, copied into a new CFString (released by the caller).
    unsafe { CFStringCreateWithBytes(core::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0) }
}

fn assert_one(ty: &str, reason: &str) -> u32 {
    let (t, r) = (cfstr(ty), cfstr(reason));
    let mut id = 0u32;
    // SAFETY: two CFStrings of ours (released after) and an out id.
    let ok = unsafe {
        let ok = !t.is_null() && !r.is_null() && IOPMAssertionCreateWithName(t, LEVEL_ON, r, &mut id) == 0;
        if !t.is_null() {
            CFRelease(t);
        }
        if !r.is_null() {
            CFRelease(r);
        }
        ok
    };
    if ok { id } else { 0 }
}

pub fn hold(reason: &str, what: u32) -> Option<Held> {
    if what & (INHIBIT_SUSPEND | INHIBIT_IDLE) == 0 {
        return None;
    }
    let reason = if reason.is_empty() { crate::vfs::app_id() } else { String::from(reason) };
    let system = assert_one("PreventUserIdleSystemSleep", &reason);
    if system == 0 {
        return None;
    }
    let display = if what & INHIBIT_IDLE != 0 { assert_one("PreventUserIdleDisplaySleep", &reason) } else { 0 };
    if what & INHIBIT_IDLE != 0 && display == 0 {
        // SAFETY: the assertion made above.
        unsafe { IOPMAssertionRelease(system) };
        return None;
    }
    Some(Held { system, display })
}

pub fn release(h: &mut Held) {
    for id in [core::mem::take(&mut h.system), core::mem::take(&mut h.display)] {
        if id != 0 {
            // SAFETY: an assertion this guard made.
            unsafe { IOPMAssertionRelease(id) };
        }
    }
}
