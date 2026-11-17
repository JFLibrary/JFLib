// linux_x11_lock -- (G7a) the pointer lock on X11 (the shared part: plock.rs)
//
//   X11::pointer_lock(w, on) -> PointerLock   the Display method (linux_disp.rs)
//   X11::lock_try()           a request that waited: FocusIn of its window
//   X11::lock_release(w)      let it go (the app, Esc, FocusOut, minimized, destroyed)
//   X11::lock_raw(ev)         an XI_RawMotion (linux_x11_xi.rs xi_event)
//
// Locked: GrabPointer on the window (owner events, confined to it, with the invisible cursor of
// CUR_NONE as the grab's cursor) and XI_RawMotion of the master devices selected on the root
// window (XInput 2: raw events are only sent there).  A raw event carries the device's values
// before the server's acceleration (axisvalues_raw); valuators 0 and 1 are x and y.  For a
// relative device (a mouse, a touchpad) they are the motion; an absolute one (a tablet, the
// pointer of a virtual machine: valuator mode Absolute in XIQueryDevice) gives positions, whose
// differences scaled from the valuator's range to the screen are the motion in pixels.  A raw
// event that repeats the last one (same time, same values: a device's event for both its master
// and itself) counts once.  The release ungrabs, deselects the raw events and warps the pointer
// back to where it was in the window when the lock began.  Without XInput 2 the request is
// Unsupported; a grab another client holds makes it Refused.

use super::*;
use crate::{PointerLock, plock_motion, plock_set, plock_state, plock_win, xi_raw_decode};

const XR_GRAB_POINTER: u32 = 26;
const XR_WARP_POINTER: u32 = 41;
const XI_SELECT_EVENTS: u32 = 46;
const XI_QUERY_DEVICE: u32 = 48;
pub(super) const XI_RAW_MOTION: u32 = 17;
const XI_ALL_MASTER: u32 = 1;
const ABS_MAX: usize = 8;

/// an absolute device: its id and the ranges of valuators 0 and 1
#[derive(Clone, Copy, Default)]
struct Abs {
    id: u16,
    range: [[i32; 2]; 2],
    last: Option<[f64; 2]>,
}

#[derive(Default)]
pub(super) struct XLock {
    /// grabbed, the raw events selected
    on: bool,
    /// the window that holds it
    win: i32,
    /// where the pointer was in the window when it began (None: outside)
    at: Option<[i32; 2]>,
    /// the last raw event (time, x, y): a repeat counts once
    last: (u32, f64, f64),
    abs: [Abs; ABS_MAX],
    nabs: usize,
}

impl X11 {
    /// (Display::pointer_lock) lock the pointer to window w or let it go -> the state now
    pub(super) fn pointer_lock(&mut self, w: i32, on: bool) -> PointerLock {
        if !on {
            self.lock_release(w);
            return PointerLock::Off;
        }
        if self.rec(w).is_none() {
            return PointerLock::Refused;
        }
        if self.xi_op == 0 {
            crate::log_warn("plat", "pointer lock: the X server has no XInput 2 (raw motion)");
            plock_set(w, PointerLock::Unsupported);
            return PointerLock::Unsupported;
        }
        let cur = plock_win();
        if cur >= 0 && cur != w {
            self.lock_release(cur); // one window at a time
        }
        if cur == w && self.lk.on {
            return PointerLock::Locked;
        }
        plock_set(w, PointerLock::Pending);
        self.lock_try();
        plock_state()
    }

    /// a request that waits: the grab once its window has the focus
    pub(super) fn lock_try(&mut self) {
        let w = plock_win();
        if w < 0 || plock_state() != PointerLock::Pending || self.lk.on {
            return;
        }
        let Some(i) = self.rec(w) else { return };
        if !self.xw[i].focus || !self.xw[i].mapped || self.fd < 0 {
            return;
        }
        let win = self.xw[i].win;
        let cursor = self.cursor_get(crate::CUR_NONE);
        let (o, seq) = self.req(XR_GRAB_POINTER | 1 << 8, 24); // owner_events 1
        wr32(&mut self.out, o, win);
        wr16(&mut self.out, o + 4, XM_BUTTON_PRESS | XM_BUTTON_RELEASE | XM_POINTER_MOTION);
        self.out[o + 6] = 1; // pointer mode async
        self.out[o + 7] = 1; // keyboard mode async
        wr32(&mut self.out, o + 8, win); // confined to the window
        wr32(&mut self.out, o + 12, cursor);
        wr32(&mut self.out, o + 16, 0); // CurrentTime
        let st = self.reply(seq).map_or(255, |r| r[1]);
        if st != 0 {
            // AlreadyGrabbed 1, InvalidTime 2, NotViewable 3, Frozen 4
            crate::log!(crate::LOG_WARN, "plat", "pointer lock: GrabPointer refused (status {st})");
            plock_set(w, PointerLock::Refused);
            return;
        }
        self.lock_abs_devices();
        let root = self.root;
        self.lock_select(root, 1 << XI_RAW_MOTION);
        self.flush();
        let inside = self.mx >= 0 && self.my >= 0 && self.mx < self.xw[i].w && self.my < self.xw[i].h;
        self.lk.at = if inside { Some([self.mx, self.my]) } else { None };
        self.lk.on = true;
        self.lk.win = w;
        self.lk.last = (0, f64::NAN, f64::NAN);
        plock_set(w, PointerLock::Locked);
    }

    /// window w lets the lock go: ungrab, no raw events, the pointer back where it was
    pub(super) fn lock_release(&mut self, w: i32) {
        if w < 0 || plock_win() != w {
            return;
        }
        if core::mem::take(&mut self.lk.on) && self.fd >= 0 {
            let root = self.root;
            self.lock_select(root, 0);
            let (o, _) = self.req(XR_UNGRAB_POINTER, 8);
            wr32(&mut self.out, o, 0); // CurrentTime
            if let (Some([x, y]), Some(i)) = (self.lk.at, self.rec(w)) {
                let win = self.xw[i].win;
                let (o, _) = self.req(XR_WARP_POINTER, 24);
                wr32(&mut self.out, o, 0); // src: None
                wr32(&mut self.out, o + 4, win);
                wr32(&mut self.out, o + 8, 0); // src x, y
                wr32(&mut self.out, o + 12, 0); // src w, h
                wr16(&mut self.out, o + 16, x as u32);
                wr16(&mut self.out, o + 18, y as u32);
            }
            self.flush();
        }
        if plock_state() != PointerLock::Off {
            plock_set(w, PointerLock::Off);
        }
    }

    // XISelectEvents on the root for every master device (mask 0: none)
    fn lock_select(&mut self, root: u32, mask: u32) {
        if self.xi_op == 0 || root == 0 {
            return;
        }
        let (o, _) = self.req(self.xi_op | XI_SELECT_EVENTS << 8, 20);
        let b = &mut self.out;
        wr32(b, o, root);
        wr16(b, o + 4, 1); // num_mask
        wr16(b, o + 8, XI_ALL_MASTER);
        wr16(b, o + 10, 1); // mask_len
        wr32(b, o + 12, mask);
    }

    // the devices whose valuators 0 and 1 are absolute (XIQueryDevice)
    fn lock_abs_devices(&mut self) {
        self.lk.nabs = 0;
        let (o, seq) = self.req(self.xi_op | XI_QUERY_DEVICE << 8, 8);
        wr16(&mut self.out, o, 0); // XIAllDevices
        let Some(r) = self.reply(seq) else { return };
        let len = rd32(&r, 4) as usize;
        if len > (X11_IN_CAP - 32) / 4 {
            return;
        }
        let end = (len * 4 + 32).min(r.len());
        let mut left = rd16(&r, 8);
        let mut p = 32;
        while left > 0 && p + 12 <= end {
            left -= 1;
            let id = rd16(&r, p);
            let mut classes = rd16(&r, p + 6);
            let name_len = rd16(&r, p + 8) as usize;
            let mut c = p + 12 + ((name_len + 3) & !3);
            let mut a = Abs { id, ..Abs::default() };
            let mut have = 0u32;
            while classes > 0 && c + 4 <= end {
                classes -= 1;
                let cl = rd16(&r, c + 2) as usize * 4;
                if cl < 4 || c + cl > end {
                    return;
                }
                // XIValuatorClass: number +6, min.integral +12, max.integral +20, mode +40
                if rd16(&r, c) == 2 && cl >= 44 {
                    let num = rd16(&r, c + 6) as usize;
                    if num < 2 && r[c + 40] == 1 {
                        a.range[num] = [rd32(&r, c + 12) as i32, rd32(&r, c + 20) as i32];
                        have |= 1 << num;
                    }
                }
                c += cl;
            }
            if have == 3 && self.lk.nabs < ABS_MAX {
                self.lk.abs[self.lk.nabs] = a;
                self.lk.nabs += 1;
            }
            p = c;
        }
    }

    /// (xi_event) an XI_RawMotion: the motion of the locked pointer
    pub(super) fn lock_raw(&mut self, b: &[u8]) {
        if !self.lk.on {
            return;
        }
        let Some((_dev, src, time, x, y, have)) = xi_raw_decode(b) else { return };
        if have == 0 || (time, x, y) == self.lk.last {
            return; // nothing of x / y, or the same event again
        }
        self.lk.last = (time, x, y);
        let n = self.lk.nabs;
        let (dx, dy) = match self.lk.abs[..n].iter().position(|a| a.id == src) {
            Some(k) => {
                // an absolute device: its positions' difference, scaled to the screen
                let (sw, sh) = (self.scr_w.max(1) as f64, self.scr_h.max(1) as f64);
                let a = &mut self.lk.abs[k];
                let prev = a.last.unwrap_or([x, y]);
                let now = [if have & 1 != 0 { x } else { prev[0] }, if have & 2 != 0 { y } else { prev[1] }];
                a.last = Some(now);
                let span = |r: [i32; 2]| (r[1] as f64 - r[0] as f64).max(1.0);
                ((now[0] - prev[0]) * sw / span(a.range[0]), (now[1] - prev[1]) * sh / span(a.range[1]))
            }
            None => (if have & 1 != 0 { x } else { 0.0 }, if have & 2 != 0 { y } else { 0.0 }),
        };
        plock_motion(self.lk.win, dx, dy);
    }
}

/// (test aid, plattest --test input) play the X11 side without a server: a lock of window w held
/// (on) or none; absolute devices (id, x range, y range) for lock_raw
pub(super) fn lock_test(x: &mut X11, w: i32, on: bool, abs: &[(u16, [i32; 2], [i32; 2])]) {
    x.lk = XLock { on, win: w, last: (0, f64::NAN, f64::NAN), ..XLock::default() };
    for &(id, rx, ry) in abs.iter().take(ABS_MAX) {
        x.lk.abs[x.lk.nabs] = Abs { id, range: [rx, ry], last: None };
        x.lk.nabs += 1;
    }
    plock_set(w, if on { PointerLock::Locked } else { PointerLock::Off });
}
