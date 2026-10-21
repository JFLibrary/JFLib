// power -- keep the computer awake while the app works: no standby, no screen saver
//
//   inhibit(reason, what) -> Option<Inhibit>   ask the system not to go to sleep on its own
//                                    (INHIBIT_SUSPEND: downloads, backups, long exports) or
//                                    also to keep the screen on and the session active
//                                    (INHIBIT_IDLE: presentations, videos; implies
//                                    INHIBIT_SUSPEND).  The guard releases it when dropped.
//                                    None: not possible here (no service, web, Android)
//   inhibited() -> u32               the INHIBIT_* bits the live guards of this process hold
//
// reason is shown by the system (`powercfg /requests`, the desktop's list of inhibitors);
// "" names the app.  The user can still put the computer to sleep, and the opposite
// direction already reaches the app: UIC_SUSPEND (the system goes to sleep / woke up) and
// UIC_SESSION_END (logout, shutdown) come from win_api.rs and linux_life.rs.
//
// Windows (win_power.rs): a power request with the reason as its text (PowerCreateRequest,
// PowerSetRequest: INHIBIT_SUSPEND -> PowerRequestSystemRequired, INHIBIT_IDLE -> also
// PowerRequestDisplayRequired).  Linux (linux_power.rs): the Inhibit portal
// (org.freedesktop.portal.Inhibit, flags suspend 4 / idle 8; also inside Flatpak; a refusal
// of its backend arrives up to 300 ms after the answer and is waited for; INHIBIT_IDLE falls
// back to idle alone where a backend only inhibits idle), without it
// org.freedesktop.ScreenSaver (idle) and org.freedesktop.PowerManagement.Inhibit (suspend;
// only best effort beside idle) -- all on a session-bus connection of plat that stays open
// while a guard lives (the desktop ends inhibitions with their connection; it is closed when
// the last guard goes).  A backend may still drop an inhibition later; the guard does not
// notice.  logind's Inhibit is not used: it answers with a file descriptor, which plat's
// D-Bus client does not receive.

use alloc::string::String;
use core::sync::atomic::{AtomicU32, Ordering};

/// keep the system from going to sleep on its own (the screen may still turn off)
pub const INHIBIT_SUSPEND: u32 = 1;
/// keep the screen on and the session active (no screen saver, no lock); implies
/// INHIBIT_SUSPEND
pub const INHIBIT_IDLE: u32 = 2;

// live guards per bit (inhibited)
static LIVE: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];

/// an inhibition; dropping it lets the system sleep again
pub struct Inhibit {
    what: u32,
    held: native::Held,
}

impl Inhibit {
    /// the INHIBIT_* bits this guard holds (INHIBIT_IDLE with INHIBIT_SUSPEND)
    pub fn what(&self) -> u32 {
        self.what
    }
}

impl Drop for Inhibit {
    fn drop(&mut self) {
        native::release(&mut self.held);
        for (k, live) in LIVE.iter().enumerate() {
            if self.what & (1 << k) != 0 {
                live.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }
}

impl core::fmt::Debug for Inhibit {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "Inhibit({})", self.what)
    }
}

/// keep the system awake (INHIBIT_SUSPEND) or also the screen on (INHIBIT_IDLE) until the
/// guard is dropped -> None: not possible here
pub fn inhibit(reason: &str, what: u32) -> Option<Inhibit> {
    if what == 0 || what & !(INHIBIT_SUSPEND | INHIBIT_IDLE) != 0 {
        return None;
    }
    let what = if what & INHIBIT_IDLE != 0 { what | INHIBIT_SUSPEND } else { what };
    let reason: String = if reason.is_empty() { crate::vfs::app_id() } else { reason.chars().filter(|&c| c != '\0').take(256).collect() };
    let held = native::hold(&reason, what)?;
    for (k, live) in LIVE.iter().enumerate() {
        if what & (1 << k) != 0 {
            live.fetch_add(1, Ordering::SeqCst);
        }
    }
    Some(Inhibit { what, held })
}

/// the INHIBIT_* bits the live guards of this process hold
pub fn inhibited() -> u32 {
    LIVE.iter().enumerate().filter(|(_, l)| l.load(Ordering::SeqCst) != 0).fold(0, |m, (k, _)| m | 1 << k)
}

#[cfg(windows)]
#[path = "win_power.rs"]
mod native;

#[cfg(target_os = "linux")]
#[path = "linux_power.rs"]
mod native;

#[cfg(not(any(windows, target_os = "linux")))]
mod native {
    pub struct Held;
    pub fn hold(_reason: &str, _what: u32) -> Option<Held> {
        None
    }
    pub fn release(_h: &mut Held) {}
}
