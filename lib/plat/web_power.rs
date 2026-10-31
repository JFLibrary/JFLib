// web_power -- (power.rs) keeping the screen on in the browser and on Android
//
// The guards of the process are counted per INHIBIT_* bit; the host hears the union whenever
// it changes.  Browser (web/system.js): REQUEST_POWER, one byte (1: keep the screen on, 0: let
// it go) -> the Screen Wake Lock (navigator.wakeLock.request("screen")), which the browser drops
// while the tab is hidden: the page takes it again when the tab is visible again.  Only
// INHIBIT_IDLE is possible there (a page cannot keep the computer running with the screen
// off): INHIBIT_SUSPEND alone and a browser without the API (jfgui_system wakelock=0) give None.
// Android (android/GallerySystem.java): CALL_POWER, u32 INHIBIT_* union -> one byte 1 / 0;
// INHIBIT_IDLE is FLAG_KEEP_SCREEN_ON on the activity's window, INHIBIT_SUSPEND (which
// INHIBIT_IDLE implies) a partial WakeLock "jfgui:<app>" (the CPU keeps running with the
// screen off; the manifest needs android.permission.WAKE_LOCK, without it: None).

use rt::sync::SpinLock;

use crate::{INHIBIT_IDLE, INHIBIT_SUSPEND};

/// (browser) keep the screen on (one byte 1 / 0; no completion)
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const REQUEST_POWER: u32 = 25;
#[cfg(target_os = "android")]
const CALL_POWER: u32 = 52;

pub struct Held(u32);

// the live guards per bit (suspend, idle)
static COUNT: SpinLock<[u32; 2]> = SpinLock::new([0, 0]);

fn union(c: &[u32; 2]) -> u32 {
    (if c[0] > 0 { INHIBIT_SUSPEND } else { 0 }) | (if c[1] > 0 { INHIBIT_IDLE } else { 0 })
}

// tell the host the union -> it holds it
fn apply(bits: u32) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        crate::web::notice(REQUEST_POWER, &[(bits & INHIBIT_IDLE != 0) as u8]);
        true
    }
    #[cfg(target_os = "android")]
    {
        crate::web::android_host::call(CALL_POWER, &bits.to_le_bytes()).is_some_and(|r| r.first() == Some(&1))
    }
}

pub fn hold(_reason: &str, what: u32) -> Option<Held> {
    #[cfg(target_arch = "wasm32")]
    if what & INHIBIT_IDLE == 0 || !crate::web::system_host::wakelock() {
        return None;
    }
    let mut c = COUNT.lock();
    let before = union(&c);
    let mut next = *c;
    for (k, n) in next.iter_mut().enumerate() {
        if what & (1 << k) != 0 {
            *n += 1;
        }
    }
    let after = union(&next);
    if after != before && !apply(after) {
        return None;
    }
    *c = next;
    Some(Held(what))
}

pub fn release(h: &mut Held) {
    let mut c = COUNT.lock();
    let before = union(&c);
    for (k, n) in c.iter_mut().enumerate() {
        if h.0 & (1 << k) != 0 {
            *n = n.saturating_sub(1);
        }
    }
    let after = union(&c);
    if after != before {
        let _ = apply(after);
    }
    h.0 = 0;
}
