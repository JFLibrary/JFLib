// linux_wl_lock -- (G7a) the pointer lock on Wayland (the shared part: plock.rs)
//
//   Wl::pointer_lock(w, on) -> PointerLock   the Display method (linux_disp.rs)
//   Wl::lock_try()            a request that waits: the lock objects once window w has the
//                             keyboard (wl_keyboard.enter) and the seat a pointer
//   Wl::lock_release(w)       let it go (the app, Esc, wl_keyboard.leave, minimized, destroyed)
//   Wl::lock_hides(slot)      no cursor over that window (cursor_apply)
//   wlh_lp_locked / wlh_lp_unlocked / lock_motion   the compositor's events
//
// zwp_pointer_constraints_v1.lock_pointer (lifetime oneshot, no region: the whole surface) keeps
// the pointer where it is while the surface has the pointer focus; the compositor says when it
// holds (locked: Pending -> Locked, the cursor goes) and when it let go on its own (unlocked: its
// focus moved, its own shortcut; the objects go, Off).  The motion comes from a
// zwp_relative_pointer_v1 of the seat's pointer: the unaccelerated dx_unaccel / dy_unaccel (24.8
// fixed, in the device's units; libinput normalises them to 1000 dpi), not the pointer's
// positions.  Both globals must be there (the lock without the motion would leave the game
// blind): without one of them the request is Unsupported and logged.

use super::proto::*;
use super::wire::{Args, A};
use super::Wl;
use crate::{PointerLock, plock_motion, plock_set, plock_state, plock_win, wl_fixed};

const LIFETIME_ONESHOT: u32 = 1;

/// the lock's protocol objects
#[derive(Default)]
pub(crate) struct Lock {
    /// zwp_locked_pointer_v1 (0: none)
    pub lp: u32,
    /// zwp_relative_pointer_v1 of the lock (0: none)
    pub rp: u32,
    /// the window of lp
    pub win: i32,
    /// the compositor said locked
    pub locked: bool,
    /// the missing protocols were logged
    logged: bool,
}

impl Wl {
    /// (Display::pointer_lock) lock the pointer to window w or let it go -> the state now
    pub(crate) fn pointer_lock(&mut self, w: i32, on: bool) -> PointerLock {
        if !on {
            self.lock_release(w);
            return PointerLock::Off;
        }
        if self.rec(w).is_none() {
            return PointerLock::Refused;
        }
        if self.g.pcm == 0 || self.g.rpm == 0 {
            if !self.lock.logged {
                self.lock.logged = true;
                crate::log_warn("plat", if self.g.pcm == 0 {
                    "pointer lock: the compositor has no zwp_pointer_constraints_v1"
                } else {
                    "pointer lock: the compositor has no zwp_relative_pointer_manager_v1"
                });
            }
            plock_set(w, PointerLock::Unsupported);
            return PointerLock::Unsupported;
        }
        let cur = plock_win();
        if cur >= 0 && cur != w {
            self.lock_release(cur); // one window at a time
        }
        if cur == w && (self.lock.lp != 0 || plock_state() == PointerLock::Locked) {
            return plock_state();
        }
        plock_set(w, PointerLock::Pending);
        self.lock_try();
        plock_state()
    }

    /// a request that waits: the lock objects, once its window has the keyboard
    pub(crate) fn lock_try(&mut self) {
        let w = plock_win();
        if w < 0 || plock_state() != PointerLock::Pending || self.lock.lp != 0 {
            return;
        }
        let Some(i) = self.rec(w) else { return };
        let (surf, ptr) = (self.win[i].surf, self.inp.ptr);
        if surf == 0 || ptr == 0 || !self.inp.kb_focus || self.inp.kb_win != w || self.g.pcm == 0 || self.g.rpm == 0 {
            return;
        }
        self.lock.win = w;
        self.lock.locked = false;
        self.lock.lp = self.w.send(self.g.pcm, &PC_LOCK_POINTER,
                                   &[A::N(&IFACE_ZWP_LP), A::O(surf), A::O(ptr), A::O(0), A::U(LIFETIME_ONESHOT)]);
        self.lock.rp = self.w.send(self.g.rpm, &RPM_GET_RELATIVE_POINTER, &[A::N(&IFACE_ZWP_RP), A::O(ptr)]);
        self.w.flush();
    }

    /// window w lets the lock go: the objects go, the cursor comes back, Off
    pub(crate) fn lock_release(&mut self, w: i32) {
        if w < 0 || plock_win() != w {
            return;
        }
        let shown = self.lock.locked;
        if self.lock.rp != 0 {
            self.w.send(self.lock.rp, &RP_DESTROY, &[]);
            self.lock.rp = 0;
        }
        if self.lock.lp != 0 {
            self.w.send(self.lock.lp, &LP_DESTROY, &[]);
            self.lock.lp = 0;
        }
        self.lock.locked = false;
        if plock_state() != PointerLock::Off {
            plock_set(w, PointerLock::Off);
        }
        if shown {
            self.cursor_apply();
        }
        self.w.flush();
    }

    /// no cursor over window slot wi: its pointer is locked
    pub(crate) fn lock_hides(&self, wi: usize) -> bool {
        self.lock.locked && self.lock.lp != 0 && self.lock.win as usize == wi
    }

    /// zwp_relative_pointer_v1.relative_motion of the lock's object (utime_hi, utime_lo, dx, dy,
    /// dx_unaccel, dy_unaccel) -> true: it was the lock's
    pub(crate) fn lock_motion(&mut self, id: u32, a: &Args) -> bool {
        if id == 0 || id != self.lock.rp {
            return false;
        }
        if self.lock.locked {
            plock_motion(self.lock.win, wl_fixed(a.i(4)), wl_fixed(a.i(5)));
        }
        true
    }
}

/// zwp_locked_pointer_v1.locked: the compositor holds the pointer
pub(crate) fn wlh_lp_locked(wl: &mut Wl, id: u32, _a: &Args, _u: u64) {
    if id == 0 || id != wl.lock.lp {
        return;
    }
    wl.lock.locked = true;
    plock_set(wl.lock.win, PointerLock::Locked);
    wl.cursor_apply();
}

/// zwp_locked_pointer_v1.unlocked: the compositor let it go (a oneshot lock is over)
pub(crate) fn wlh_lp_unlocked(wl: &mut Wl, id: u32, _a: &Args, _u: u64) {
    if id == 0 || id != wl.lock.lp {
        return;
    }
    let w = wl.lock.win;
    wl.lock_release(w);
}
