// linux_wl_input -- Wayland seat: pointer, keyboard, key repeat, the event helper
//                                                                  [src/plat/linux/wl_input.s]
//
//   Wl::push(type, btn, x, y, data)   evq_push with the current MOD_* bits and the event's
//                                     window (inp.evwin)
//   Wl::input_service(now) -> next key-repeat deadline (u64::MAX none)   (wait)
//   Wl::release_buttons()             E_UP for every held button (move / resize grabs)
//   wlh_seat_caps, wlh_ptr_*, wlh_kb_* event handlers
//
// Pointer events belong to the window the pointer entered (ptr_win), keyboard events to the
// window with the keyboard focus (kb_win).  Pointer positions are converted from surface
// coordinates (24.8 fixed, logical) to physical pixels: px = fixed * scale120 / 30720 (the
// window's scale).  Wheel events are accumulated until wl_pointer.frame: axis_value120
// (seat >= 8) or discrete steps x 120 (WSLg) or the continuous value x 12 / 256
// (touchpads); vertical is negated so that up = positive, like Win32.
// Keys: the keymap is parsed by linux_xkb.rs; key repeat uses repeat_info (default 25/s
// after 600 ms) with its own deadline in wait (modifiers never repeat).

use rt::os::{syscall6, SYS_MMAP, SYS_MUNMAP};

use super::proto::*;
use super::wire::{sys_close, Args, A};
use super::xkb::{wl_mods, wl_mods_set, xkb_dump, xkb_is_modifier, xkb_key, xkb_mod_bit, xkb_parse, xkb_set_mods,
                 xkb_use_fallback};
use super::Wl;
use crate::{
    Event, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, BTN_X1, BTN_X2, E_DOWN, E_FOCUS_IN, E_FOCUS_OUT, E_KEY_DOWN, E_KEY_UP,
    E_LEAVE, E_MOVE, E_TEXT, E_UP, E_WHEEL, PW_MAX,
};

const BTN_CODE_LEFT: u32 = 0x110; // linux/input-event-codes.h

// evdev BTN_LEFT.. BTN_BACK -> BTN_*
static BTN_MAP: [u8; 7] = [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_X1, BTN_X2, BTN_X2, BTN_X1];

pub(crate) struct Input {
    pub ptr: u32, // wl_pointer
    pub kb: u32,  // wl_keyboard
    pub csd: u32, // wp_cursor_shape_device_v1 of the pointer
    pub enter_serial: u32,
    pub btn_serial: u32,
    pub input_serial: u32,
    pub ptr_in: bool,
    pub mx: i32,
    pub my: i32,
    btn_mask: u32,
    pub kb_focus: bool,
    pub kb_win: i32,  // window with the keyboard focus
    pub evwin: i32,   // window of the event being pushed
    pub ptr_win: i32, // window under the pointer (-1 = none)
    pub off_x: i32,   // offset of the popup under the pointer in its owner (px, wl_popup)
    pub off_y: i32,
    pub s120_test: i32, // pointer scale while there is no window (g_wl_s120, tests)
    kb_ready: bool,
    rep_key: u32, // evdev + 1 of the repeating key
    rep_next: u64,
    rep_rate: i32,
    rep_delay: i32,
    ax_val: [i32; 2], // per axis (24.8)
    ax_disc: [i32; 2],
    ax_120: [i32; 2],
    ax_flags: u32, // bit axis: discrete, bit 2+axis: value120
}

impl Input {
    pub fn new() -> Input {
        Input {
            ptr: 0,
            kb: 0,
            csd: 0,
            enter_serial: 0,
            btn_serial: 0,
            input_serial: 0,
            ptr_in: false,
            mx: 0,
            my: 0,
            btn_mask: 0,
            kb_focus: false,
            kb_win: 0,
            evwin: 0,
            ptr_win: -1,
            off_x: 0,
            off_y: 0,
            s120_test: 0,
            kb_ready: false,
            rep_key: 0,
            rep_next: 0,
            rep_rate: 25,
            rep_delay: 600,
            ax_val: [0; 2],
            ax_disc: [0; 2],
            ax_120: [0; 2],
            ax_flags: 0,
        }
    }
}

impl Wl {
    /// queue an event with the current modifiers and the window inp.evwin  [wl_push]
    pub(crate) fn push(&mut self, typ: u8, btn: u8, x: i32, y: i32, data: u32) {
        let win = (self.inp.evwin as u32 & (PW_MAX as u32 - 1)) as u8;
        crate::evq_push(Event { typ, btn, mods: wl_mods() as u8, win, x, y, data });
    }

    /// events of the pointer: its window  [ptr_evwin]
    fn ptr_evwin(&mut self) {
        self.inp.evwin = self.inp.ptr_win.max(0);
    }

    /// fixed -> physical pixels at the scale of the pointer's window (without a window
    /// s120_test, set by synthetic tests)  [ptr_px]
    fn ptr_px(&self, fixed: i32) -> i32 {
        let w = self.inp.ptr_win.max(0) as usize & (PW_MAX - 1);
        let r = &self.win[w];
        let s = if r.flags & super::window::WLWF_USED != 0 { r.s120 } else { self.inp.s120_test };
        (fixed as i64 * s as i64 / 30720) as i32
    }

    /// the left button is held (the implicit grab of a drag, dnd.rs)
    pub(crate) fn left_held(&self) -> bool {
        self.inp.btn_mask & 1 << BTN_LEFT != 0
    }

    /// E_UP for every held button (at the last position)  [wl_release_buttons]
    pub(crate) fn release_buttons(&mut self) {
        self.ptr_evwin();
        for b in 1..=5u8 {
            if self.inp.btn_mask & 1 << b != 0 {
                self.inp.btn_mask &= !(1 << b);
                let (x, y) = (self.inp.mx, self.inp.my);
                self.push(E_UP, b, x, y, 0);
            }
        }
    }

    /// key repeat -> the next repeat deadline (u64::MAX = none)  [wl_input_service]
    pub(crate) fn input_service(&mut self, now: u64) -> u64 {
        if self.inp.rep_key == 0 {
            return u64::MAX;
        }
        if !self.inp.kb_focus {
            self.inp.rep_key = 0;
            return u64::MAX;
        }
        if now < self.inp.rep_next {
            return self.inp.rep_next;
        }
        self.inp.evwin = self.inp.kb_win;
        let (vk, text) = xkb_key(self.inp.rep_key - 1);
        if vk != 0 {
            self.push(E_KEY_DOWN, 0, 0, 0, vk | 0x8000_0000);
        }
        if text != 0 {
            self.push(E_TEXT, 0, 0, 0, text);
        }
        if self.inp.rep_rate <= 0 {
            self.inp.rep_key = 0;
            return u64::MAX;
        }
        let iv = (1_000_000 / self.inp.rep_rate as u32) as u64;
        let mut next = self.inp.rep_next + iv;
        if next <= now {
            next = now + iv; // fell behind: no bursts
        }
        self.inp.rep_next = next;
        next
    }
}

// ------------------------------------------------------------------ seat

/// wl_seat.capabilities: create / drop pointer + keyboard  [wlh_seat_caps]
pub(crate) fn wlh_seat_caps(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    let caps = a.u(0);
    if caps & 1 != 0 {
        if wl.inp.ptr == 0 {
            let p = wl.w.send(wl.g.seat, &SEAT_GET_POINTER, &[A::N(&IFACE_WL_POINTER)]);
            wl.inp.ptr = p;
            if wl.g.csm != 0 {
                wl.inp.csd = wl.w.send(wl.g.csm, &CSM_GET_POINTER, &[A::N(&IFACE_WP_CSD), A::O(p)]);
            }
        }
    } else if wl.inp.ptr != 0 {
        let p = wl.inp.ptr;
        wl.inp.ptr = 0;
        wl.inp.ptr_in = false;
        if wl.g.seat_ver >= 3 {
            wl.w.send(p, &POINTER_RELEASE, &[]);
        }
    }
    if caps & 2 != 0 {
        if wl.inp.kb == 0 {
            wl.inp.kb = wl.w.send(wl.g.seat, &SEAT_GET_KEYBOARD, &[A::N(&IFACE_WL_KEYBOARD)]);
        }
    } else if wl.inp.kb != 0 {
        let k = wl.inp.kb;
        wl.inp.kb = 0;
        wl.inp.rep_key = 0;
        if wl.g.seat_ver >= 3 {
            wl.w.send(k, &KEYBOARD_RELEASE, &[]);
        }
    }
}

// ------------------------------------------------------------------ pointer

/// enter(serial, surface, x, y): the pointer is over one of our windows  [wlh_ptr_enter]
pub(crate) fn wlh_ptr_enter(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    wl.inp.enter_serial = a.u(0);
    wl.inp.off_x = 0;
    wl.inp.off_y = 0;
    wl.popup_ptr_reset();
    let surf = a.u(1);
    let mut w = wl.find(surf);
    if w < 0 {
        w = wl.popup_ptr(surf); // (U3) a popup window: the events are its owner's, moved by its offset
        if w < 0 {
            return;
        }
    }
    wl.inp.ptr_win = w;
    wl.inp.evwin = w;
    wl.inp.ptr_in = true;
    wl.inp.mx = wl.ptr_px(a.i(2)) + wl.inp.off_x;
    wl.inp.my = wl.ptr_px(a.i(3)) + wl.inp.off_y;
    wl.cursor_apply();
    let (x, y) = (wl.inp.mx, wl.inp.my);
    wl.push(E_MOVE, 0, x, y, 0);
}

/// [wlh_ptr_leave]
pub(crate) fn wlh_ptr_leave(wl: &mut Wl, _id: u32, _a: &Args, _u: u64) {
    if !wl.inp.ptr_in {
        return;
    }
    wl.inp.ptr_in = false;
    wl.release_buttons();
    wl.ptr_evwin();
    wl.push(E_LEAVE, 0, 0, 0, 0);
    wl.inp.ptr_win = -1;
}

/// motion(time, x, y)  [wlh_ptr_motion]
pub(crate) fn wlh_ptr_motion(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    wl.inp.mx = wl.ptr_px(a.i(1)) + wl.inp.off_x; // (U3) over a popup window
    wl.inp.my = wl.ptr_px(a.i(2)) + wl.inp.off_y;
    wl.ptr_evwin();
    let (x, y) = (wl.inp.mx, wl.inp.my);
    wl.push(E_MOVE, 0, x, y, 0);
}

/// button(serial, time, button, state)  [wlh_ptr_button]
pub(crate) fn wlh_ptr_button(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    wl.inp.btn_serial = a.u(0);
    wl.inp.input_serial = a.u(0);
    let b = a.u(2).wrapping_sub(BTN_CODE_LEFT);
    if b > 6 {
        return;
    }
    let btn = BTN_MAP[b as usize];
    let typ = if a.u(3) != 0 {
        wl.inp.btn_mask |= 1 << btn;
        E_DOWN
    } else {
        if wl.inp.btn_mask & 1 << btn == 0 {
            return; // not pressed inside our window
        }
        wl.inp.btn_mask &= !(1 << btn);
        E_UP
    };
    wl.ptr_evwin();
    let (x, y) = (wl.inp.mx, wl.inp.my);
    wl.push(typ, btn, x, y, 0);
}

/// axis(time, axis, value)  [wlh_ptr_axis]
pub(crate) fn wlh_ptr_axis(wl: &mut Wl, id: u32, a: &Args, u: u64) {
    let ax = a.u(1);
    if ax > 1 {
        return;
    }
    wl.inp.ax_val[ax as usize] = wl.inp.ax_val[ax as usize].wrapping_add(a.i(2));
    if wl.g.seat_ver < 5 {
        wlh_ptr_frame(wl, id, a, u); // no frame events before v5
    }
}

/// axis_discrete(axis, steps)  [wlh_ptr_discrete]
pub(crate) fn wlh_ptr_discrete(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    let ax = a.u(0);
    if ax > 1 {
        return;
    }
    wl.inp.ax_disc[ax as usize] = wl.inp.ax_disc[ax as usize].wrapping_add(a.i(1));
    wl.inp.ax_flags |= 1 << ax;
}

/// axis_value120(axis, value120)  [wlh_ptr_value120]
pub(crate) fn wlh_ptr_value120(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    let ax = a.u(0);
    if ax > 1 {
        return;
    }
    wl.inp.ax_120[ax as usize] = wl.inp.ax_120[ax as usize].wrapping_add(a.i(1));
    wl.inp.ax_flags |= 1 << (ax + 2);
}

/// end of a pointer event group: flush the wheel  [wlh_ptr_frame]
pub(crate) fn wlh_ptr_frame(wl: &mut Wl, _id: u32, _a: &Args, _u: u64) {
    let mut d = [0i32; 2]; // dy, dx
    for ax in 0..2 {
        let f = wl.inp.ax_flags;
        let v = if f & 1 << (ax + 2) != 0 {
            wl.inp.ax_120[ax]
        } else if f & 1 << ax != 0 {
            wl.inp.ax_disc[ax].wrapping_mul(120)
        } else {
            ((wl.inp.ax_val[ax] as i64 * 12) >> 8) as i32
        };
        let v = v.clamp(-32767, 32767);
        d[ax] = if ax == 0 { -v } else { v }; // Wayland: positive = down
    }
    wl.inp.ax_flags = 0;
    wl.inp.ax_val = [0; 2];
    wl.inp.ax_disc = [0; 2];
    wl.inp.ax_120 = [0; 2];
    if d[0] | d[1] == 0 {
        return;
    }
    let data = (d[0] as u32 & 0xFFFF) | (d[1] as u32) << 16;
    wl.ptr_evwin();
    let (x, y) = (wl.inp.mx, wl.inp.my);
    wl.push(E_WHEEL, 0, x, y, data);
}

// ------------------------------------------------------------------ keyboard

/// keymap(format, fd, size): parse it (MAP_PRIVATE)  [wlh_kb_keymap]
pub(crate) fn wlh_kb_keymap(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    let fmt = a.u(0);
    let fd = a.u(1) as i32;
    let size = a.u(2) as usize;
    wl.inp.kb_ready = true;
    let mut ok = false;
    if fmt == 1 && size != 0 {
        // XKB_V1
        // SAFETY: a private read-only mapping of the keymap fd, unmapped below.
        let p = unsafe { syscall6(SYS_MMAP, 0, size, 1, 2, fd as u32 as usize, 0) };
        if !(-4095..0).contains(&p) {
            // SAFETY: the mapping holds size bytes.
            let text = unsafe { core::slice::from_raw_parts(p as *const u8, size) };
            ok = xkb_parse(text) == 0;
            // SAFETY: the mapping made above.
            unsafe { syscall6(SYS_MUNMAP, p as usize, size, 0, 0, 0, 0) };
        }
    }
    if !ok {
        crate::write_stderr(b"wayland: keymap not usable, using the built-in US layout\n");
        xkb_use_fallback();
    }
    sys_close(fd);
    if crate::env("jfgui_XKBDUMP").is_some() {
        xkb_dump();
    }
}

/// enter(serial, surface, keys): one of our windows has the keyboard focus  [wlh_kb_enter]
pub(crate) fn wlh_kb_enter(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    wl.inp.input_serial = a.u(0);
    let w = wl.find(a.u(1));
    if w < 0 {
        return;
    }
    wl.inp.kb_win = w;
    wl.inp.evwin = w;
    wl.inp.kb_focus = true;
    wl.push(E_FOCUS_IN, 0, 0, 0, 0);
}

/// [wlh_kb_leave]
pub(crate) fn wlh_kb_leave(wl: &mut Wl, _id: u32, _a: &Args, _u: u64) {
    wl.inp.kb_focus = false;
    wl.inp.rep_key = 0;
    wl.inp.evwin = wl.inp.kb_win;
    wl.push(E_FOCUS_OUT, 0, 0, 0, 0);
}

/// key(serial, time, key, state)  [wlh_kb_key]
pub(crate) fn wlh_kb_key(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    wl.inp.input_serial = a.u(0);
    let key = a.u(2); // evdev code
    let down = a.u(3) != 0;
    if key >= 256 {
        return;
    }
    if !wl.inp.kb_ready {
        wl.inp.kb_ready = true;
        xkb_use_fallback();
    }
    // like Win32, a modifier's own key event already carries its new state
    let mb = xkb_mod_bit(key);
    if mb != 0 {
        wl_mods_set(if down { wl_mods() | mb } else { wl_mods() & !mb });
    }
    wl.inp.evwin = wl.inp.kb_win;
    let (vk, text) = xkb_key(key);
    if down {
        if vk != 0 {
            wl.push(E_KEY_DOWN, 0, 0, 0, vk);
        }
        if text != 0 {
            wl.push(E_TEXT, 0, 0, 0, text);
        }
        if !xkb_is_modifier(key) && wl.inp.rep_rate > 0 {
            wl.inp.rep_key = key + 1;
            wl.inp.rep_next = crate::time_us() + (wl.inp.rep_delay as u32).wrapping_mul(1000) as u64;
        }
    } else {
        if key + 1 == wl.inp.rep_key {
            wl.inp.rep_key = 0;
        }
        if vk != 0 {
            wl.push(E_KEY_UP, 0, 0, 0, vk);
        }
    }
}

/// modifiers(serial, depressed, latched, locked, group)  [wlh_kb_modifiers]
pub(crate) fn wlh_kb_modifiers(_wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    xkb_set_mods(a.u(1), a.u(2), a.u(3), a.u(4));
}

/// repeat_info(rate per s, delay ms)  [wlh_kb_repeat]
pub(crate) fn wlh_kb_repeat(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    wl.inp.rep_rate = a.i(0);
    wl.inp.rep_delay = a.i(1);
}
