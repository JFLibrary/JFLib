// cursor -- custom pointer cursors drawn by the app, at the dpi of the window
//                                                                     [src/ui/ui_cursor.s]
//
//   cursor_new(f, w, h, hot) -> handle 1..CUR_USER_MAX (0 = no slot left)
//        a cursor of w x h DIP with the hot spot hot = x | y << 16 (DIP); f(g, dpi) draws it
//        at `dpi` into g (FMT_PARGB, transparent; physical px: ceil(w * dpi / 96) x
//        ceil(h * dpi / 96)) -- called again for every new dpi
//   cursor_custom(handle)      the cursor that hit boxes with CUR_CUSTOM show (their
//                              next_flags(CUR_CUSTOM << HB_CURSOR_SHIFT)); set it each
//                              frame before the widget; 0 = the crosshair
//   cursor_render(handle, dpi) -> Some(surface)   the cursor drawn into a new surface
//                              (FMT_PARGB; tests, previews)
//   cursor_current() -> handle the handle CUR_CUSTOM stands for (0 = none)
// Internal: cursor_resolve(shape) -> the plat shape for a hit box cursor (CUR_CUSTOM: the
// custom cursor made for the current window's dpi with plat::cursor_create, rebuilt when
// that dpi changes; the crosshair when there is none or the platform has no custom
// cursors, e.g. headless runs).

use alloc::rc::Rc;
use alloc::vec::Vec;

use super::cmd::Gfx;
use super::consts::*;
use super::Ui;
use gfx::{Surface, FMT_PARGB};

/// a cursor's draw function: f(g, dpi) draws into the cursor surface
pub type CursorDraw = Rc<dyn Fn(&mut Gfx, i32)>;

// one custom cursor slot  [UC_*]
#[derive(Clone)]
struct UCur {
    f: CursorDraw,
    w: u32,
    h: u32,
    hot: u32,
    /// plat shape (-1 none, -2 could not be made at dpi)
    plat: i32,
    dpi: u32,
}

/// the cursor slots and the current custom cursor
#[derive(Default)]
pub(crate) struct S {
    slots: Vec<Option<UCur>>,
    cur: u32,
}

// uc_px(dip, dpi) -> ceil(dip * dpi / 96)
fn uc_px(dip: u32, dpi: u32) -> u32 {
    dip.wrapping_mul(dpi).wrapping_add(95) / 96
}

impl Ui {
    /// ui_cursor_new(f, w, h, hot) -> handle (0 = none)
    pub fn cursor_new(&mut self, f: CursorDraw, w: u32, h: u32, hot: u32) -> u32 {
        let s = &mut self.ucur.slots;
        if s.is_empty() {
            s.resize(CUR_USER_MAX as usize, None);
        }
        let Some(i) = s.iter().position(|x| x.is_none()) else { return 0 };
        s[i] = Some(UCur { f, w, h, hot, plat: -1, dpi: 0 });
        i as u32 + 1
    }

    /// ui_cursor_custom(handle): the cursor of CUR_CUSTOM hit boxes
    pub fn cursor_custom(&mut self, h: u32) {
        self.ucur.cur = h;
    }

    /// ui_cursor_current -> handle
    pub fn cursor_current(&self) -> u32 {
        self.ucur.cur
    }

    // uc_rec(handle): the slot of a live handle
    fn uc_rec(&self, h: u32) -> Option<&UCur> {
        let i = h.wrapping_sub(1);
        if i >= CUR_USER_MAX {
            return None;
        }
        self.ucur.slots.get(i as usize)?.as_ref()
    }

    /// ui_cursor_render(handle, dpi) -> the cursor drawn at dpi into a new surface
    pub fn cursor_render(&mut self, h: u32, dpi: i32) -> Option<Surface> {
        let r = self.uc_rec(h)?.clone();
        if (dpi as u32).wrapping_sub(24) > 960 - 24 {
            return None; // 25 % .. 1000 %
        }
        let w = uc_px(r.w, dpi as u32);
        let hh = uc_px(r.h, dpi as u32);
        if w.wrapping_sub(1) >= CUR_MAX_PX as u32 || hh.wrapping_sub(1) >= CUR_MAX_PX as u32 {
            return None;
        }
        let mut sf = Surface::new(w as i32, hh as i32, FMT_PARGB);
        if sf.px.is_empty() {
            return None;
        }
        self.dc.begin(&sf);
        {
            let mut g = Gfx { dc: &mut self.dc, sf: &mut sf, text: &mut self.text };
            (r.f)(&mut g, dpi);
        }
        self.dc.end();
        Some(sf)
    }

    /// ui_cursor_resolve(shape) -> the plat shape: CUR_CUSTOM becomes the custom cursor made
    /// for the current window's dpi (the crosshair without one)
    pub(crate) fn cursor_resolve(&mut self, shape: u32) -> u32 {
        if shape != CUR_CUSTOM {
            return shape;
        }
        let h = self.ucur.cur;
        let Some(r) = self.uc_rec(h).cloned() else { return CUR_CROSS };
        let dpi = plat::cursor_dpi(self.wp.rt.dpi) as u32; // dpi to draw for
        if r.dpi == dpi {
            if r.plat >= 0 {
                return r.plat as u32;
            }
            if r.plat == -2 {
                return CUR_CROSS; // failed at this dpi already
            }
        }
        // a new image: forget the old cursor (the platform shows the arrow meanwhile)
        if r.plat >= 0 {
            plat::cursor_destroy(r.plat);
        }
        let i = (h - 1) as usize;
        if let Some(s) = self.ucur.slots[i].as_mut() {
            s.plat = -2;
            s.dpi = dpi;
        }
        let Some(sf) = self.cursor_render(h, dpi as i32) else { return CUR_CROSS };
        // rows of exactly w * 4 bytes for the platform
        let (w, hh) = (sf.w as usize, sf.h as usize);
        let mut px = Vec::with_capacity(w * hh * 4);
        for y in 0..hh {
            let o = y * sf.stride as usize;
            px.extend_from_slice(&sf.px[o..o + w * 4]);
        }
        // hot spot in px
        let hx = ((r.hot & 0xFFFF).wrapping_mul(dpi).wrapping_add(48) / 96) as i32;
        let hy = ((r.hot >> 16).wrapping_mul(dpi).wrapping_add(48) / 96) as i32;
        let id = plat::cursor_create(&px, w as i32, hh as i32, hx, hy, dpi as i32);
        if id >= 0 {
            if let Some(s) = self.ucur.slots[i].as_mut() {
                s.plat = id;
            }
            return id as u32;
        }
        CUR_CROSS
    }
}
