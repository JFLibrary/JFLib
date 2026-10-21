// linux_wl_tablet -- (PN1) Wayland pen tablets: tablet-unstable-v2 (zwp_tablet_manager_v2)
//                                                                 [src/plat/linux/wl_tablet.s]
//
//   Wl::tablet_init()       zwp_tablet_manager_v2.get_tablet_seat(seat) when the compositor
//                           offers the manager (after the globals); nothing otherwise
//   Wl::pen_devices()       tablets announced (tablet_added - removed), -1 = the compositor
//                           has no tablet protocol
//   wlh_tab_*, wlh_tool_*   event handlers
//
// A bound tablet seat makes the compositor send tablet tool events instead of emulated
// pointer events.  Each tool (TOOL_MAX slots) collects its events until tool.frame, then
// pushes, in this order: E_MOVE (motion), E_DOWN (down: BTN_LEFT, BTN_RIGHT with BTN_STYLUS
// held, BTN_MIDDLE with BTN_STYLUS2), E_UP (up), E_LEAVE (proximity_out, no button held).
// Pen data (evq_pen_data): pressure 0..65535 (capability pressure, else none), tool type
// eraser -> PTR_ERASER (every other type PTR_PEN), tilt in degrees.  Positions are surface
// coordinates of the window the tool is in (proximity_in), converted like the pointer's
// (fixed * scale120 / 30720).  Over a popup surface (menus) the events are the owner's, moved
// by the popup's offset (like the pointer, popup_ptr); a surface that is not ours is ignored
// until the tool leaves.  With wp_cursor_shape_v1 the tool shows the window's cursor shape.
// Server-sent values are not trusted: slots, types and buttons are range checked.

use super::cursor::shape_of;
use super::proto::*;
use super::wire::{Args, A};
use super::window::WLWF_USED;
use super::Wl;
use crate::{
    BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, CUR_ARROW, CUR_COUNT, CUR_CROSS, CUR_USER0, CUR_USER_MAX, E_DOWN, E_LEAVE, E_MOVE,
    E_UP, PTR_ERASER, PTR_PEN, PW_MAX,
};

const TOOL_MAX: usize = 8;
const FR_MOTION: u32 = 1;
const FR_DOWN: u32 = 2;
const FR_UP: u32 = 4;
const FR_IN: u32 = 8;
const FR_OUT: u32 = 16;
const CAP_TILT: u32 = 1; // zwp_tablet_tool_v2.capability values
const CAP_PRESSURE: u32 = 2;
const TYPE_ERASER: u32 = 0x141; // zwp_tablet_tool_v2.type
const BTN_STYLUS: u32 = 0x14B; // linux/input-event-codes.h

/// a tool (id 0 = free slot)
#[derive(Clone, Copy, Default)]
struct Tool {
    id: u32,
    typ: u32,  // PTR_PEN / PTR_ERASER
    caps: u32, // bit 0 pressure, bit 1 tilt
    win: i32,  // window index (-1 = not over one of ours)
    x: i32,    // 24.8 surface position
    y: i32,
    press: i32, // 0..65535
    tx: i32,    // tilt degrees
    ty: i32,
    stylus: u32, // bit 0 BTN_STYLUS held, bit 1 BTN_STYLUS2 held
    frame: u32,  // FR_* pending until frame
    btn: u8,     // BTN_* of the tip contact (0 = up)
    serial: u32, // serial of proximity_in / down
    surf: u32,   // surface of proximity_in
    csd: u32,    // wp_cursor_shape_device_v1 of the tool (0 = none)
    offx: i32,   // over a popup: its offset in the owner window (px)
    offy: i32,
}

#[derive(Default)]
pub(crate) struct Tablet {
    tools: [Tool; TOOL_MAX],
    seat: u32, // zwp_tablet_seat_v2
    ntab: i32, // tablets present
}

impl Wl {
    /// the tablet seat of our seat  [wl_tablet_init]
    pub(crate) fn tablet_init(&mut self) {
        if self.g.tabm == 0 || self.tab.seat != 0 || self.g.seat == 0 {
            return;
        }
        self.tab.seat = self.w.send(self.g.tabm, &TABLET_MANAGER_GET_TABLET_SEAT,
                                    &[A::N(&IFACE_TABLET_SEAT), A::O(self.g.seat)]);
    }

    /// tablets present, -1 = no tablet protocol  [wl_pen_devices]
    pub(crate) fn pen_devices(&self) -> i32 {
        if self.tab.seat == 0 { -1 } else { self.tab.ntab }
    }

    /// the tool slot of a handler's udata (an existing tool)  [TOOLREC]
    fn tool(&self, u: u64) -> Option<usize> {
        let k = u as u32 as usize;
        if k < TOOL_MAX && self.tab.tools[k].id != 0 { Some(k) } else { None }
    }

    /// E_UP of a held tip (tool left / removed)  [tool_release]
    fn tool_release(&mut self, k: usize) {
        let t = self.tab.tools[k];
        if t.btn == 0 || t.win < 0 {
            return;
        }
        self.inp.evwin = t.win & (PW_MAX as i32 - 1);
        self.tab.tools[k].btn = 0;
        let (x, y) = (self.inp.mx, self.inp.my);
        self.push(E_UP, t.btn, x, y, 0);
    }

    /// physical pixels at the window's scale (no window record: s120_test)  [tool_px]
    fn tool_px(&self, w: i32, fixed: i32) -> i32 {
        let r = &self.win[w as usize & (PW_MAX - 1)];
        let s = if r.flags & WLWF_USED != 0 { r.s120 } else { self.inp.s120_test };
        (fixed as i64 * s as i64 / 30720) as i32
    }

    /// the window's cursor shape for the tool (wp_cursor_shape_v1 only; without it the
    /// compositor shows its default tool cursor)  [tool_cursor]
    fn tool_cursor(&mut self, k: usize) {
        if self.g.csm == 0 {
            return;
        }
        if self.tab.tools[k].csd == 0 {
            let id = self.tab.tools[k].id;
            self.tab.tools[k].csd = self.w.send(self.g.csm, &CSM_GET_TABLET_TOOL, &[A::N(&IFACE_WP_CSD), A::O(id)]);
        }
        let t = self.tab.tools[k];
        let mut shape = self.win[t.win as usize & (PW_MAX - 1)].cursor;
        if shape >= CUR_COUNT {
            // a custom cursor: the closest shape
            shape = if shape.wrapping_sub(CUR_USER0) < CUR_USER_MAX { CUR_CROSS } else { CUR_ARROW };
        }
        self.w.send(t.csd, &CSD_SET_SHAPE, &[A::U(t.serial), A::U(shape_of(shape))]);
    }
}

/// tablet_seat.tablet_added(new id): count it  [wlh_tab_added]
pub(crate) fn wlh_tab_added(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    wl.w.srv_add(a.u(0), &IFACE_TABLET, 0);
    wl.tab.ntab += 1;
}

/// tablet.removed: destroy it  [wlh_tab_removed]
pub(crate) fn wlh_tab_removed(wl: &mut Wl, id: u32, _a: &Args, _u: u64) {
    if wl.tab.ntab != 0 {
        wl.tab.ntab -= 1;
    }
    wl.w.send(id, &TABLET_DESTROY, &[]);
}

/// tablet_seat.tool_added(new id): a free slot (else the tool's events are dropped)
/// [wlh_tool_added]
pub(crate) fn wlh_tool_added(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    let id = a.u(0);
    let Some(k) = wl.tab.tools.iter().position(|t| t.id == 0) else { return };
    wl.tab.tools[k] = Tool { id, typ: PTR_PEN, win: -1, ..Tool::default() };
    wl.w.srv_add(id, &IFACE_TABLET_TOOL, k as u64);
}

/// type(tool_type)  [wlh_tool_type]
pub(crate) fn wlh_tool_type(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    if let Some(k) = wl.tool(u) {
        wl.tab.tools[k].typ = if a.u(0) == TYPE_ERASER { PTR_ERASER } else { PTR_PEN };
    }
}

/// capability(cap)  [wlh_tool_cap]
pub(crate) fn wlh_tool_cap(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    if let Some(k) = wl.tool(u) {
        match a.u(0) {
            CAP_PRESSURE => wl.tab.tools[k].caps |= 1,
            CAP_TILT => wl.tab.tools[k].caps |= 2,
            _ => {}
        }
    }
}

/// the tool is gone: release, destroy, free the slot  [wlh_tool_removed]
pub(crate) fn wlh_tool_removed(wl: &mut Wl, id: u32, _a: &Args, u: u64) {
    if let Some(k) = wl.tool(u) {
        wl.tool_release(k);
        let csd = wl.tab.tools[k].csd;
        if csd != 0 {
            wl.w.send(csd, &CSD_DESTROY, &[]);
        }
        wl.tab.tools[k].id = 0;
    }
    wl.w.send(id, &TABLET_TOOL_DESTROY, &[]);
}

/// proximity_in(serial, tablet, surface)  [wlh_tool_in]
pub(crate) fn wlh_tool_in(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    if let Some(k) = wl.tool(u) {
        let t = &mut wl.tab.tools[k];
        t.serial = a.u(0);
        t.surf = a.u(2);
        t.frame = (t.frame | FR_IN) & !FR_OUT;
    }
}

/// proximity_out  [wlh_tool_out]
pub(crate) fn wlh_tool_out(wl: &mut Wl, _id: u32, _a: &Args, u: u64) {
    if let Some(k) = wl.tool(u) {
        wl.tab.tools[k].frame |= FR_OUT;
    }
}

/// down(serial): tip contact  [wlh_tool_down]
pub(crate) fn wlh_tool_down(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    if let Some(k) = wl.tool(u) {
        wl.tab.tools[k].serial = a.u(0);
        wl.tab.tools[k].frame |= FR_DOWN;
    }
}

/// up  [wlh_tool_up]
pub(crate) fn wlh_tool_up(wl: &mut Wl, _id: u32, _a: &Args, u: u64) {
    if let Some(k) = wl.tool(u) {
        wl.tab.tools[k].frame |= FR_UP;
    }
}

/// motion(x, y) surface-local 24.8  [wlh_tool_motion]
pub(crate) fn wlh_tool_motion(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    if let Some(k) = wl.tool(u) {
        let t = &mut wl.tab.tools[k];
        t.x = a.i(0);
        t.y = a.i(1);
        t.frame |= FR_MOTION;
    }
}

/// pressure(0..65535)  [wlh_tool_pressure]
pub(crate) fn wlh_tool_pressure(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    if let Some(k) = wl.tool(u) {
        wl.tab.tools[k].press = a.u(0).min(65535) as i32;
    }
}

/// tilt(x, y) degrees 24.8  [wlh_tool_tilt]
pub(crate) fn wlh_tool_tilt(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    if let Some(k) = wl.tool(u) {
        wl.tab.tools[k].tx = a.i(0) >> 8;
        wl.tab.tools[k].ty = a.i(1) >> 8;
    }
}

/// button(serial, button, state): the barrel buttons  [wlh_tool_button]
pub(crate) fn wlh_tool_button(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    if let Some(k) = wl.tool(u) {
        let b = a.u(1).wrapping_sub(BTN_STYLUS);
        if b > 1 {
            return; // BTN_STYLUS (0) / BTN_STYLUS2 (1) only
        }
        if a.u(2) != 0 {
            wl.tab.tools[k].stylus |= 1 << b;
        } else {
            wl.tab.tools[k].stylus &= !(1 << b);
        }
    }
}

/// the tool's events of one frame -> E_*  [wlh_tool_frame]
pub(crate) fn wlh_tool_frame(wl: &mut Wl, _id: u32, _a: &Args, u: u64) {
    let Some(k) = wl.tool(u) else { return };
    let f = wl.tab.tools[k].frame;
    wl.tab.tools[k].frame = 0;
    if f & FR_IN != 0 {
        wl.tab.tools[k].offx = 0;
        wl.tab.tools[k].offy = 0;
        let surf = wl.tab.tools[k].surf;
        let mut w = wl.find(surf); // the window of the tool
        if w < 0 {
            // a popup: its owner and offset (the pointer's offset is kept)
            let (px, py) = (wl.inp.off_x, wl.inp.off_y);
            w = wl.popup_ptr(surf);
            wl.tab.tools[k].offx = wl.inp.off_x;
            wl.tab.tools[k].offy = wl.inp.off_y;
            wl.inp.off_x = px;
            wl.inp.off_y = py;
        }
        wl.tab.tools[k].win = w;
        if w >= 0 {
            wl.tool_cursor(k);
        }
    }
    let t = wl.tab.tools[k];
    if t.win < 0 {
        return;
    }
    wl.inp.evwin = t.win & (PW_MAX as i32 - 1);
    // the pen data of this frame
    let p = if t.caps & 1 != 0 { t.press } else { -1 };
    let data = crate::evq_pen_data(p, t.typ, t.tx, t.ty);
    if f & FR_MOTION != 0 {
        wl.inp.mx = wl.tool_px(t.win, t.x) + t.offx;
        wl.inp.my = wl.tool_px(t.win, t.y) + t.offy;
        let (x, y) = (wl.inp.mx, wl.inp.my);
        wl.push(E_MOVE, 0, x, y, data);
    }
    if f & FR_DOWN != 0 && t.btn == 0 {
        wl.inp.btn_serial = t.serial;
        wl.inp.input_serial = t.serial;
        let b = if t.stylus & 1 != 0 {
            BTN_RIGHT
        } else if t.stylus & 2 != 0 {
            BTN_MIDDLE
        } else {
            BTN_LEFT
        };
        wl.tab.tools[k].btn = b;
        let (x, y) = (wl.inp.mx, wl.inp.my);
        wl.push(E_DOWN, b, x, y, data);
    }
    let t = wl.tab.tools[k];
    if f & FR_UP != 0 && t.btn != 0 {
        wl.tab.tools[k].btn = 0;
        let (x, y) = (wl.inp.mx, wl.inp.my);
        wl.push(E_UP, t.btn, x, y, data);
    }
    if f & FR_OUT != 0 {
        wl.tool_release(k);
        wl.push(E_LEAVE, 0, 0, 0, 0);
        wl.tab.tools[k].win = -1;
    }
}
