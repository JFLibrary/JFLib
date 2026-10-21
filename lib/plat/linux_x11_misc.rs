// linux_x11_misc -- X11 popup windows, pointer cursors and monitors
//                         [src/plat/linux/x11_popup.s, x11_cursor.s, x11_monitor.s, lx_popup.s]
//
//   popup(slot, owner, rect, frame)  (U3) an override-redirect window (no window manager
//        frame, never focused) at the root position of owner pixel rect.x, rect.y
//        (TranslateCoordinates, so it is right even when the cached window position is not),
//        stacked on top, shown with PutImage strips (popups are small); rect None unmaps it.
//        The pixels are copied (repaints on Expose read the copy).
//   popup_route(packet) -> owner index for pointer events of a popup (the packet's x, y are
//        moved into the owner's coordinates, so button / motion / crossing handle them as
//        events of the owner), -2 when the event was handled here (Expose), -1 not a popup
//   win_set_cursor(w, shape)         CUR_* of window w (CUR_NONE hides the pointer) or a
//        custom cursor CUR_USER0..CUR_TABLE-1.  CUR_ARROW is cursor None: the window shows
//        the root window's cursor, the desktop's themed arrow.  The other shapes are glyph
//        cursors (black on white) of the core "cursor" font, made on first use: xterm 152,
//        hand2 60, crosshair 34, fleur 52, watch 150, sb_v_double_arrow 116,
//        sb_h_double_arrow 108, bottom_right_corner 14 (NW-SE), bottom_left_corner 12
//        (NE-SW).  CUR_NONE is an empty 1x1 bitmap cursor.
//   cursor_create(px, w, h, hot_x, hot_y) -> CUR_USER0 + k / -1   an ARGB cursor through
//        RENDER >= 0.5: a depth-32 pixmap (PutImage in bands of <= 64 KB), a picture of the
//        direct A24 R16 G8 B0 format, RenderCreateCursor; picture / GC / pixmap freed at once
//   cursor_destroy(id)               windows showing it get None (the arrow) and forget
//                                    their shape (the next set applies even with the same id)
//   monitor_rect(x, y) -> the monitor containing root point x, y (else the primary / first
//        one), from RandR 1.5 GetMonitors, asked again at most every 2 s; None without RandR
//        1.5 (U3)

use alloc::format;
use alloc::vec::Vec;

use super::*;
use crate::{CUR_ARROW, CUR_MAX_PX, CUR_NONE, CUR_USER0};

const XQ_EVENTS: u32 = XM_BUTTON_PRESS | XM_BUTTON_RELEASE | XM_ENTER | XM_LEAVE | XM_POINTER_MOTION | XM_EXPOSURE;
const CFG_XYWH_STACK: u32 = 0x4F; // ConfigureWindow: x, y, width, height, stack-mode
const ZPIXMAP: u32 = 2;
const GC_FOREGROUND: u32 = 4;
const XR_FREE_PIXMAP: u32 = 54;
const XR_FREE_GC: u32 = 60;
const XR_FREE_CURSOR: u32 = 95;
const XRR_QUERY_VERSION: u32 = 0; // RENDER minor opcodes
const XRR_QUERY_PICT_FORMATS: u32 = 1;
const XRR_CREATE_PICTURE: u32 = 4;
const XRR_FREE_PICTURE: u32 = 7;
const XRR_CREATE_CURSOR: u32 = 27;
pub(super) const XM_MAX: usize = 8;
const XM_REFRESH_US: u64 = 2000000;
const RR_QUERY_VERSION: u32 = 0;
const RR_GET_MONITORS: u32 = 42;
// CUR_* -> cursor font glyph (CUR_ARROW: None)
static CURSOR_GLYPH: [u8; 11] = [68, 152, 60, 34, 52, 150, 116, 108, 14, 12, 0];

impl X11 {
    // ------------------------------------------------------------------ popups  [x11_popup.s]

    /// x11_plat_popup(slot, owner, rect, frame)
    pub(super) fn popup(&mut self, slot: u32, owner: i32, rect: Option<[i32; 4]>, f: Option<&Frame>) {
        let s = slot as usize;
        if s >= self.xq.len() {
            return;
        }
        let Some(r) = rect else {
            // hide
            let q = &mut self.xq[s];
            q.owner = -1;
            q.px = Vec::new();
            q.pw = 0;
            q.ph = 0;
            if !q.mapped {
                return;
            }
            q.mapped = false;
            let win = q.win;
            let (o, _) = self.req(XR_UNMAP_WINDOW, 8);
            wr32(&mut self.out, o, win);
            self.flush();
            return;
        };
        let Some(i) = self.rec(owner) else { return };
        let ow = self.xw[i].win;
        // root position of the owner's pixel rect.x, rect.y
        let (o, seq) = self.req(XR_TRANSLATE_COORDS, 16);
        let root = self.root;
        let b = &mut self.out;
        wr32(b, o, ow);
        wr32(b, o + 4, root);
        wr16(b, o + 8, r[0] as u32);
        wr16(b, o + 10, r[1] as u32);
        let Some(t) = self.reply(seq) else { return };
        let (rx, ry) = (rds16(&t, 12), rds16(&t, 14));
        // the window: created once (override-redirect)
        if self.xq[s].win == 0 {
            let win = self.new_id();
            self.xq[s].win = win;
            self.xq[s].mapped = false;
            let (op, bytes) = if self.cmap != 0 { (XR_CREATE_WINDOW | 24 << 8, 32 + 20) } else { (XR_CREATE_WINDOW, 32 + 16) };
            let (o, _) = self.req(op, bytes);
            let (vis, cmap) = (self.visual, self.cmap);
            let mut mask = XCW_BACK_PIXMAP | XCW_BORDER_PIXEL | XCW_OVERRIDE | XCW_EVENT_MASK;
            if cmap != 0 {
                mask |= XCW_COLORMAP;
            }
            let b = &mut self.out;
            wr32(b, o, win);
            wr32(b, o + 4, root);
            wr16(b, o + 8, rx as u32);
            wr16(b, o + 10, ry as u32);
            wr16(b, o + 12, r[2] as u32);
            wr16(b, o + 14, r[3] as u32);
            wr16(b, o + 16, 0); // border width
            wr16(b, o + 18, 1); // InputOutput
            wr32(b, o + 20, vis);
            wr32(b, o + 24, mask);
            wr32(b, o + 28, 0); // background None
            wr32(b, o + 32, 0); // border pixel
            wr32(b, o + 36, 1); // override-redirect
            wr32(b, o + 40, XQ_EVENTS);
            if cmap != 0 {
                wr32(b, o + 44, cmap);
            }
        }
        let q = &mut self.xq[s];
        q.owner = owner;
        q.dx = r[0];
        q.dy = r[1];
        q.w = r[2];
        q.h = r[3];
        match f {
            Some(f) => {
                let (pw, ph) = (f.w.max(0), f.h.max(0));
                let row = pw as usize * 4;
                q.px = Vec::with_capacity(row * ph as usize);
                for y in 0..ph as usize {
                    let o = y * f.stride as usize;
                    q.px.extend_from_slice(&f.px[o..o + row]);
                }
                q.pw = pw;
                q.ph = ph;
            }
            None => {
                q.px = Vec::new();
                q.pw = 0;
                q.ph = 0;
            }
        }
        let win = q.win;
        let (w, h) = (q.w, q.h);
        // place it (and on top)
        let (o, _) = self.req(XR_CONFIGURE_WINDOW, 12 + 5 * 4);
        let b = &mut self.out;
        wr32(b, o, win);
        wr16(b, o + 4, CFG_XYWH_STACK);
        wr32(b, o + 8, rx as u32);
        wr32(b, o + 12, ry as u32);
        wr32(b, o + 16, w as u32);
        wr32(b, o + 20, h as u32);
        wr32(b, o + 24, 0); // Above
        if !self.xq[s].mapped {
            self.xq[s].mapped = true;
            let (o, _) = self.req(XR_MAP_WINDOW, 8);
            wr32(&mut self.out, o, win);
        }
        self.xq_put(s);
        self.flush();
    }

    /// xq_put(slot): the whole surface into the popup window (PutImage strips)
    fn xq_put(&mut self, s: usize) {
        let q = &self.xq[s];
        if q.px.is_empty() || q.win == 0 {
            return;
        }
        let w = q.pw.min(q.w);
        if w <= 0 {
            return;
        }
        let h = q.ph.min(q.h);
        let bpr = w as u32 * 4; // bytes per row
        let rows = ((self.maxreq.min(X11_OUT_CAP as u32) - 32) / bpr).max(1);
        let (win, gc) = (q.win, self.gc);
        let mut y = 0i32;
        while y < h {
            let n = ((h - y) as u32).min(rows);
            let (o, _) = self.req(XR_PUT_IMAGE | ZPIXMAP << 8, (n * bpr + 24) as usize);
            let q = &self.xq[s];
            let b = &mut self.out;
            wr32(b, o, win);
            wr32(b, o + 4, gc);
            wr16(b, o + 8, w as u32);
            wr16(b, o + 10, n);
            wr16(b, o + 12, 0);
            wr16(b, o + 14, y as u32);
            b[o + 16] = 0; // left pad
            b[o + 17] = 24; // depth
            wr16(b, o + 18, 0);
            let mut d = o + 20;
            for _ in 0..n {
                let src = y as usize * q.pw as usize * 4;
                b[d..d + bpr as usize].copy_from_slice(&q.px[src..src + bpr as usize]);
                d += bpr as usize;
                y += 1;
            }
        }
    }

    /// x11_popup_route(packet) -> owner index / -2 handled / -1 not a popup
    pub(super) fn popup_route(&mut self, b: &mut [u8]) -> i32 {
        let typ = b[0] & 0x7F;
        if typ == XE_EXPOSE {
            let Some(s) = self.xq_find(rd32(b, 4)) else { return -1 };
            if rd16(b, 16) == 0 {
                // the last Expose of a series
                self.xq_put(s);
                self.flush();
            }
            return -2;
        }
        if !(XE_BUTTON_PRESS..=XE_LEAVE).contains(&typ) {
            return -1;
        }
        let Some(s) = self.xq_find(rd32(b, 12)) else { return -1 };
        let q = &self.xq[s];
        if q.owner < 0 {
            return -1;
        }
        let x = (rd16(b, XEV_X) as i32).wrapping_add(q.dx);
        let y = (rd16(b, XEV_Y) as i32).wrapping_add(q.dy);
        wr16(b, XEV_X, x as u32);
        wr16(b, XEV_Y, y as u32);
        q.owner
    }

    /// xq_find(xid) -> the popup slot with that window
    fn xq_find(&self, xid: u32) -> Option<usize> {
        if xid == 0 {
            return None;
        }
        self.xq.iter().position(|q| q.win == xid)
    }

    // ------------------------------------------------------------------ cursors  [x11_cursor.s]

    /// x11_plat_win_set_cursor(w, shape): the cursor of window w (its CWCursor attribute)
    pub(super) fn win_set_cursor(&mut self, w: i32, shape: u32) {
        if shape >= CUR_COUNT && shape.wrapping_sub(CUR_USER0) >= CUR_USER_MAX {
            return;
        }
        let Some(i) = self.rec(w) else { return };
        if self.xw[i].cursor == shape as i32 {
            return;
        }
        self.xw[i].cursor = shape as i32;
        let c = self.cursor_get(shape);
        let win = self.xw[i].win;
        let (o, _) = self.req(XR_CHANGE_WINDOW_ATTR, 16);
        wr32(&mut self.out, o, win);
        wr32(&mut self.out, o + 4, XCW_CURSOR);
        wr32(&mut self.out, o + 8, c);
        self.flush();
    }

    /// cursor_get(shape) -> cursor id (made on first use; 0 = None; a custom cursor's id)
    fn cursor_get(&mut self, shape: u32) -> u32 {
        let k = shape.wrapping_sub(CUR_USER0);
        if k < CUR_USER_MAX {
            return self.xcust[k as usize];
        }
        if shape >= CUR_COUNT {
            return 0;
        }
        let c = self.cursors[shape as usize];
        if c != 0 || shape == CUR_ARROW {
            return c;
        }
        let cid;
        if shape == CUR_NONE {
            // 1x1 depth-1 pixmap cleared to 0: source = mask = nothing visible
            let pm = self.new_id();
            let (o, _) = self.req(XR_CREATE_PIXMAP | 1 << 8, 16);
            let win0 = self.xw[0].win;
            wr32(&mut self.out, o, pm);
            wr32(&mut self.out, o + 4, win0);
            wr32(&mut self.out, o + 8, 0x00010001); // 1 x 1
            let gc = self.new_id();
            let (o, _) = self.req(XR_CREATE_GC, 20);
            wr32(&mut self.out, o, gc);
            wr32(&mut self.out, o + 4, pm);
            wr32(&mut self.out, o + 8, GC_FOREGROUND);
            wr32(&mut self.out, o + 12, 0);
            let (o, _) = self.req(XR_POLY_FILL_RECTANGLE, 20);
            wr32(&mut self.out, o, pm);
            wr32(&mut self.out, o + 4, gc);
            wr32(&mut self.out, o + 8, 0); // x, y
            wr32(&mut self.out, o + 12, 0x00010001);
            cid = self.new_id();
            let (o, _) = self.req(XR_CREATE_CURSOR, 32);
            wr32(&mut self.out, o, cid);
            wr32(&mut self.out, o + 4, pm); // source
            wr32(&mut self.out, o + 8, pm); // mask (colours and hot spot stay 0)
        } else {
            if self.cur_font == 0 {
                self.cur_font = self.new_id();
                let (o, _) = self.req(XR_OPEN_FONT, 12 + 8); // "cursor" padded to 8
                let font = self.cur_font;
                wr32(&mut self.out, o, font);
                wr16(&mut self.out, o + 4, 6);
                self.out[o + 8..o + 14].copy_from_slice(b"cursor");
            }
            cid = self.new_id();
            let (o, _) = self.req(XR_CREATE_GLYPH_CURSOR, 32);
            let font = self.cur_font;
            let g = CURSOR_GLYPH[shape as usize] as u32;
            let b = &mut self.out;
            wr32(b, o, cid);
            wr32(b, o + 4, font); // source font
            wr32(b, o + 8, font); // mask font
            wr16(b, o + 12, g); // source char
            wr16(b, o + 14, g + 1); // mask char
            wr32(b, o + 16, 0); // foreground black
            wr16(b, o + 20, 0);
            wr32(b, o + 22, 0xFFFFFFFF); // background white
            wr16(b, o + 26, 0xFFFF);
        }
        self.cursors[shape as usize] = cid;
        cid
    }

    /// x11_plat_cursor_create(px, w, h, hot) -> CUR_USER0 + k / -1: an ARGB cursor
    pub(super) fn cursor_create(&mut self, px: &[u8], w: i32, h: i32, hot_x: i32, hot_y: i32) -> i32 {
        if (w as u32).wrapping_sub(1) >= CUR_MAX_PX as u32 || (h as u32).wrapping_sub(1) >= CUR_MAX_PX as u32 {
            return -1;
        }
        if self.fd <= 0 || px.len() < w as usize * h as usize * 4 {
            return -1;
        }
        let Some(k) = self.xcust.iter().position(|&c| c == 0) else { return -1 };
        if !self.xr_init() {
            return -1;
        }
        // depth-32 pixmap + a GC for it
        let pm = self.new_id();
        let (o, _) = self.req(XR_CREATE_PIXMAP | 32 << 8, 16);
        let root = self.root;
        wr32(&mut self.out, o, pm);
        wr32(&mut self.out, o + 4, root);
        wr16(&mut self.out, o + 8, w as u32);
        wr16(&mut self.out, o + 10, h as u32);
        let gc = self.new_id();
        let (o, _) = self.req(XR_CREATE_GC, 16);
        wr32(&mut self.out, o, gc);
        wr32(&mut self.out, o + 4, pm);
        wr32(&mut self.out, o + 8, 0); // no values
        // PutImage ZPixmap in bands of at most 64 KB
        let band = (65536 / (w as u32 * 4)).max(1);
        let mut row = 0u32;
        while row < h as u32 {
            let n = (h as u32 - row).min(band);
            let bytes = (w as u32 * n * 4) as usize;
            let (o, _) = self.req(XR_PUT_IMAGE | ZPIXMAP << 8, bytes + 24);
            let b = &mut self.out;
            wr32(b, o, pm);
            wr32(b, o + 4, gc);
            wr16(b, o + 8, w as u32);
            wr16(b, o + 10, n);
            wr16(b, o + 12, 0);
            wr16(b, o + 14, row);
            b[o + 16] = 0; // left pad
            b[o + 17] = 32; // depth
            wr16(b, o + 18, 0);
            let s = (row * w as u32 * 4) as usize;
            b[o + 20..o + 20 + bytes].copy_from_slice(&px[s..s + bytes]);
            row += n;
        }
        let pic = self.new_id();
        let (op, fmt) = (self.xr_op, self.xr_fmt);
        let (o, _) = self.req(op | XRR_CREATE_PICTURE << 8, 20);
        wr32(&mut self.out, o, pic);
        wr32(&mut self.out, o + 4, pm);
        wr32(&mut self.out, o + 8, fmt);
        wr32(&mut self.out, o + 12, 0); // value mask
        let cid = self.new_id();
        let (o, _) = self.req(op | XRR_CREATE_CURSOR << 8, 16);
        wr32(&mut self.out, o, cid);
        wr32(&mut self.out, o + 4, pic);
        wr16(&mut self.out, o + 8, hot_x as u32);
        wr16(&mut self.out, o + 10, hot_y as u32);
        // the cursor keeps its own copy: free the picture, the GC and the pixmap
        let (o, _) = self.req(op | XRR_FREE_PICTURE << 8, 8);
        wr32(&mut self.out, o, pic);
        let (o, _) = self.req(XR_FREE_GC, 8);
        wr32(&mut self.out, o, gc);
        let (o, _) = self.req(XR_FREE_PIXMAP, 8);
        wr32(&mut self.out, o, pm);
        self.flush();
        self.xcust[k] = cid;
        (CUR_USER0 + k as u32) as i32
    }

    /// x11_plat_cursor_destroy(id): FreeCursor; windows showing it get None (the arrow) and
    /// forget their shape
    pub(super) fn cursor_destroy(&mut self, id: i32) {
        let k = (id as u32).wrapping_sub(CUR_USER0);
        if k >= CUR_USER_MAX {
            return;
        }
        let cid = self.xcust[k as usize];
        if cid == 0 {
            return;
        }
        self.xcust[k as usize] = 0;
        for j in 0..PW_MAX {
            if self.xw[j].win == 0 || self.xw[j].cursor != id {
                continue;
            }
            self.xw[j].cursor = -1;
            let win = self.xw[j].win;
            let (o, _) = self.req(XR_CHANGE_WINDOW_ATTR, 16);
            wr32(&mut self.out, o, win);
            wr32(&mut self.out, o + 4, XCW_CURSOR);
            wr32(&mut self.out, o + 8, 0); // None: the parent's cursor
        }
        let (o, _) = self.req(XR_FREE_CURSOR, 8);
        wr32(&mut self.out, o, cid);
        self.flush();
    }

    /// xr_init -> RENDER >= 0.5 with an ARGB32 format (xr_op, xr_fmt), asked once
    fn xr_init(&mut self) -> bool {
        if self.xr_state == 0 {
            self.xr_state = 2;
            self.xr_query();
        }
        self.xr_state == 1
    }

    fn xr_query(&mut self) {
        let seq = self.query_ext_send(b"RENDER");
        let Some(r) = self.reply(seq) else { return };
        if r[8] == 0 || r[9] == 0 {
            return;
        }
        let op = r[9] as u32;
        // RenderQueryVersion 0.11 (CreateCursor needs 0.5)
        let (o, seq) = self.req(op | XRR_QUERY_VERSION << 8, 12);
        wr32(&mut self.out, o, 0);
        wr32(&mut self.out, o + 4, 11);
        let Some(r) = self.reply(seq) else { return };
        if rd32(&r, 8) == 0 && rd32(&r, 12) < 5 {
            return;
        }
        // RenderQueryPictFormats: a direct depth-32 format A 24 / R 16 / G 8 / B 0, 8 bits each
        let (_, seq) = self.req(op | XRR_QUERY_PICT_FORMATS << 8, 4);
        let Some(r) = self.reply(seq) else { return };
        let words = (rd32(&r, 4) as usize).min((X11_IN_CAP - 32) / 4);
        let n = (rd32(&r, 8) as usize).min(words * 4 / 28);
        for k in 0..n {
            let f = 32 + k * 28;
            if f + 28 > r.len() {
                break;
            }
            if r[f + 4] == 1
                && r[f + 5] == 32
                && rd32(&r, f + 8) == 0x00FF0010
                && rd32(&r, f + 12) == 0x00FF0008
                && rd32(&r, f + 16) == 0x00FF0000
                && rd32(&r, f + 20) == 0x00FF0018
            {
                self.xr_fmt = rd32(&r, f);
                self.xr_op = op;
                self.xr_state = 1;
                return;
            }
        }
    }

    // ------------------------------------------------------------------ monitors  [x11_monitor.s]

    /// xm_refresh: the monitor list (xm_n; -1 = RandR 1.5 missing)
    fn xm_refresh(&mut self) {
        if self.xm_n == -1 {
            return;
        }
        let now = crate::time_us();
        if self.xm_valid && now.wrapping_sub(self.xm_t) < XM_REFRESH_US {
            return;
        }
        self.xm_t = now;
        self.xm_valid = true;
        if self.xm_op == 0 {
            // QueryExtension "RANDR", QueryVersion 1.5
            let seq = self.query_ext_send(b"RANDR");
            match self.reply(seq) {
                Some(r) if r[8] != 0 => self.xm_op = r[9] as u32,
                _ => {
                    self.xm_n = -1;
                    return;
                }
            }
            let (o, seq) = self.req(self.xm_op | RR_QUERY_VERSION << 8, 12);
            wr32(&mut self.out, o, 1);
            wr32(&mut self.out, o + 4, 5);
            match self.reply(seq) {
                Some(r) if rd32(&r, 8) > 1 || rd32(&r, 12) >= 5 => {}
                _ => {
                    self.xm_n = -1;
                    return;
                }
            }
        }
        let (o, seq) = self.req(self.xm_op | RR_GET_MONITORS << 8, 12);
        let root = self.root;
        wr32(&mut self.out, o, root);
        wr32(&mut self.out, o + 4, 1); // active monitors only
        let Some(r) = self.reply(seq) else {
            self.xm_n = -1;
            return;
        };
        let count = rd32(&r, 12) as usize;
        let mut p = 32;
        let mut k = 0;
        while k < count && k < XM_MAX && p + 24 <= r.len() {
            self.xm[k] = [rds16(&r, p + 8), rds16(&r, p + 10), rd16(&r, p + 12) as i32, rd16(&r, p + 14) as i32];
            if r[p + 4] != 0 {
                self.xm_primary = k as u32; // primary: remembered
            }
            p += rd16(&r, p + 6) as usize * 4 + 24; // outputs
            k += 1;
        }
        self.xm_n = k as i32;
        if self.debug {
            for (j, m) in self.xm[..k].iter().enumerate() {
                eprint(&format!("x11: monitor {} at {},{} size {}x{}\n", j, m[0], m[1], m[2], m[3]));
            }
        }
    }

    /// x11_monitor_rect(x, y) -> the monitor at root point x, y (else the primary one)
    pub(super) fn monitor_rect(&mut self, x: i32, y: i32) -> Option<[i32; 4]> {
        self.xm_refresh();
        let n = self.xm_n;
        if n <= 0 {
            return None;
        }
        let n = n as usize;
        for m in &self.xm[..n] {
            if x >= m[0] && y >= m[1] && x < m[0] + m[2] && y < m[1] + m[3] {
                return Some(*m);
            }
        }
        // not on a monitor: the primary one
        let p = if (self.xm_primary as usize) < n { self.xm_primary as usize } else { 0 };
        Some(self.xm[p])
    }
}
