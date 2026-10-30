// hexview -- bytes as hex and ASCII side by side, with marked ranges (a packet inspector)
//
//   hex_view(id, &mut st, data, marks) -> HexEv
//        rows "0010  45 00 00 54 ...  |E..T....|" in the system's monospace font (else the
//        UI font, a character per cell), virtualized in a vertical scroll view that fills
//        its parent unless next_size says otherwise; st.cols bytes per row (0: 16 when they
//        fit, else 8), offsets with 4 hex digits (8 beyond 64 KiB).  marks are translucent
//        fills behind their bytes in both columns (colour 0 = the chart palette colour of
//        the mark's index; an opaque colour gets a highlight's alpha), smaller ranges over
//        larger ones.  The selection (st.sel: start, length), the byte under the pointer and,
//        while the view has the focus, the caret show in the hex and the ASCII column
//        together; zero bytes and the dots of other bytes are dimmed.
//        Mouse: a click selects a byte (HexEv::clicked_key: the key of the smallest mark
//        holding it), a drag a range (the view scrolls at its edges), Shift+click extends
//        it, a double click selects the mark's range, a click on an offset its row; the byte
//        under the pointer is HexEv::hover, its offset a tooltip.  Keys (focused): arrows,
//        PgUp / PgDn, Home / End (of the row; with Ctrl of the data) move the caret (Shift
//        extends; a move reports the caret's mark like a click), Ctrl+A selects all, Ctrl+C
//        copies the selection as hex in one line ("45 00 00 54").  When the app changes
//        st.sel the view scrolls to it.
//        HexEv::ev: EV_ACT the user changed the selection, EV_DOUBLE, EV_RCLICK (the byte is
//        selected first: open a context menu), EV_HOVER, EV_FOCUS.
//
// Ids: id (the scroll view; focus; its bar part(id, 1)), part(id, 3) the rows.  Records:
// K_LIST id (value: the caret, max: the number of bytes), K_BOX part(id, 3) (value: the byte
// under the pointer or -1), per visible row K_TEXT with its offset ("0010", value: the
// offset), its hex bytes ("45 00 00 54 ...") and its characters ("E..T...."); the hover
// tooltip is a K_TEXT too.  The rows keep their left-to-right order in mirrored windows
// (ltr_begin).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::consts::*;
use crate::fontpick::FontSel;
use crate::keys::*;
use crate::list::{into_view, SCR_F_LIST};
use crate::Ui;

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// a marked byte range of a hex view (a protocol field)
#[derive(Clone, Copy, Debug, Default)]
pub struct HexMark {
    pub start: u32,
    pub len: u32,
    /// 0xAARRGGBB (an opaque colour is drawn translucent); 0 = the chart palette colour of
    /// the mark's index
    pub color: u32,
    /// the app's key of the range (HexEv::clicked_key)
    pub key: u64,
}

/// the app-owned state of a hex view: selection, bytes per row, caret
#[derive(Clone, Default, Debug)]
pub struct HexState {
    /// the selected bytes: (start, length >= 1); when the app changes it, the view scrolls to it
    pub sel: Option<(u32, u32)>,
    /// bytes per row; 0 = 16 when they fit, else 8
    pub cols: u8,
    caret: u32,
    anchor: u32,
    seen: Option<(u32, u32)>,
    drag: bool,
    moved: bool,
    press: u32,
    reveal: Option<(u32, u32)>,
    auto_t: u32,
}

impl HexState {
    /// the keyboard caret (a byte offset)
    pub fn caret(&self) -> u32 {
        self.caret
    }

    /// scroll byte `off` into view
    pub fn reveal(&mut self, off: u32) {
        self.reveal = Some((off, off));
    }
}

/// what happened in a hex view during the pass
#[derive(Clone, Copy, Default, Debug)]
pub struct HexEv {
    /// the byte under the pointer
    pub hover: Option<u32>,
    /// a byte was clicked or the caret moved onto it: the key of the smallest mark holding it
    pub clicked_key: Option<u64>,
    /// EV_ACT the selection changed, EV_DOUBLE, EV_RCLICK, EV_HOVER, EV_FOCUS
    pub ev: u32,
}

fn part(id: u32, k: u32) -> u32 {
    id.wrapping_mul(8).wrapping_add(k) | 0x8000_0000
}

// the smallest mark holding byte b (the first of equal ones)
fn mark_at(marks: &[HexMark], b: u32) -> Option<&HexMark> {
    let mut best: Option<&HexMark> = None;
    for m in marks {
        if m.len > 0 && b >= m.start && b - m.start < m.len && best.is_none_or(|x| m.len < x.len) {
            best = Some(m);
        }
    }
    best
}

// the range from byte a to byte b (either order) as (start, length)
fn range(a: u32, b: u32) -> Option<(u32, u32)> {
    Some((a.min(b), a.max(b) - a.min(b) + 1))
}

// the bytes as hex: "45 00 00 54"
fn hex_text(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 3);
    for (i, &x) in b.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push(HEX[(x >> 4) as usize] as char);
        s.push(HEX[(x & 15) as usize] as char);
    }
    s
}

// the cells of a row, in characters from its left padding: the offset (od digits), two
// blanks, the bytes (two digits and a blank; one more blank after every 8 when the row has
// more), two blanks, '|', the characters, '|'.  adv: a cell's width (24.8 px, the font's
// advance: text runs place their glyphs at fractional positions)
#[derive(Clone, Copy)]
struct Cells {
    od: i32,
    cols: i32,
    adv: i32,
}

impl Cells {
    // px from the padding to the left edge of cell c
    fn px(&self, c: i32) -> i32 {
        ((c as i64 * self.adv as i64 + 128) >> 8) as i32
    }
    // the cell at px dx from the padding
    fn at(&self, dx: i32) -> i32 {
        ((dx as i64) << 8).div_euclid(self.adv.max(1) as i64) as i32
    }
    // the first digit of byte j
    fn hx(&self, j: i32) -> i32 {
        self.od + 2 + 3 * j + if self.cols > 8 { j / 8 } else { 0 }
    }
    // the character of byte 0 (the '|' is before it)
    fn ax(&self) -> i32 {
        self.hx(self.cols - 1) + 5
    }
    fn total(&self) -> i32 {
        self.ax() + self.cols + 1
    }
    // the column at cell c (0 offset, 1 hex, 2 ASCII) and the byte of the row there
    fn pick(&self, c: i32) -> (u8, i32) {
        if c >= self.ax() - 1 {
            return (2, (c - self.ax()).clamp(0, self.cols - 1));
        }
        if c < self.hx(0) - 1 {
            return (0, 0);
        }
        let mut j = 0;
        while j + 1 < self.cols && self.hx(j + 1) <= c {
            j += 1;
        }
        (1, j)
    }
}

impl Ui {
    /// ui_hex_view(id, &mut st, data, marks) -> HexEv: bytes as hex and ASCII rows with
    /// marked ranges, a selection and a caret (see the module header of hexview.rs)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn hex_view(&mut self, id: u32, st: &mut HexState, data: &[u8], marks: &[HexMark]) -> HexEv {
        let _src = self.src_scope();
        let mut out = HexEv::default();
        let len = data.len().min(u32::MAX as usize) as u32;
        // the font: the system's monospace at the size of body text, else the UI font
        self.ts_font(TS_BODY);
        let (ui_font, size) = self.text.get_font();
        let mono_id = self.font_load(&FontSel::new("monospace", 10.0));
        let font = mono_id.unwrap_or(ui_font);
        self.text.set_font(font, size);
        let w0 = self.text.measure("0");
        let mono = mono_id.is_some() && ["W", "i", ".", " ", "|"].iter().all(|s| self.text.measure(s) == w0);
        let adv = if mono { w0 } else { w0.max(self.text.measure("W")) }.max(256);
        let (asc, desc) = (self.text.ascent(), self.text.descent());
        self.text.set_font(ui_font, size);
        let rh = self.ts_line(TS_BODY).max(asc + desc + 2);
        let od = if len > 0x1_0000 { 8 } else { 4 };
        // the scroll view (fills the parent unless next_size says otherwise)
        if self.wp.lay.next_w == 0 && self.wp.lay.next_h == 0 && self.wp.lay.next_rect_set == 0 {
            self.next_size(-1, -1);
        }
        let vev = self.scr_open(id, SC_V | SCR_F_LIST, HB_FOCUS);
        let (_, vr) = self.scr_view();
        let pad = self.dp(8);
        let wide = Cells { od, cols: 16, adv };
        let cols = if st.cols > 0 {
            st.cols as i32
        } else if wide.px(wide.total()) <= vr[2] - pad * 2 {
            16
        } else {
            8
        };
        let cl = Cells { od, cols, adv };
        let cu = cols as u32;
        let nrows = (len.div_ceil(cu) as i32).min(i32::MAX / rh - 1);
        // the selection as the app left it: kept inside the data; a new one is scrolled to
        let caret_in = st.caret;
        st.sel = st.sel.and_then(|(s, l)| (l > 0 && s < len).then(|| (s, l.min(len - s))));
        if st.sel != st.seen {
            if let Some((s, l)) = st.sel {
                st.anchor = s;
                st.caret = s + l - 1;
                st.reveal = Some((s, s + l - 1));
            }
        }
        let last = len.saturating_sub(1);
        st.caret = st.caret.min(last);
        st.anchor = st.anchor.min(last);
        let mut sel = st.sel;
        // keys: the caret, Ctrl+A, Ctrl+C
        if vev & EV_FOCUS != 0 && len > 0 {
            let ctrl = self.mods & MOD_CTRL != 0;
            let shift = self.mods & MOD_SHIFT != 0;
            let swap = self.rtl && self.mods & (MOD_CTRL | MOD_ALT) == 0; // the arrows as drawn
            let (kl, kr) = if swap { (VK_RIGHT, VK_LEFT) } else { (VK_LEFT, VK_RIGHT) };
            let (cur, c, n) = (st.caret as i64, cols as i64, len as i64);
            let page = (vr[3] / rh).max(1) as i64 * c;
            let mut new: Option<i64> = None;
            if self.key(kl) {
                new = Some(cur - 1);
            }
            if self.key(kr) {
                new = Some(cur + 1);
            }
            if self.key(VK_UP) {
                new = Some(if cur >= c { cur - c } else { cur });
            }
            if self.key(VK_DOWN) {
                new = Some(if cur + c < n { cur + c } else { cur });
            }
            if self.key(VK_PRIOR) {
                new = Some(if cur >= page { cur - page } else { cur % c });
            }
            if self.key(VK_NEXT) {
                new = Some(if cur + page < n { cur + page } else { n - 1 });
            }
            if self.key(VK_HOME) {
                new = Some(if ctrl { 0 } else { cur - cur % c });
            }
            if self.key(VK_END) {
                new = Some(if ctrl { n - 1 } else { cur - cur % c + c - 1 });
            }
            if ctrl && self.key(b'A' as u32) {
                sel = Some((0, len));
                st.anchor = 0;
                st.caret = last;
            }
            if ctrl && self.key(b'C' as u32) {
                if let Some((s, l)) = sel {
                    let t = hex_text(&data[s as usize..(s + l) as usize]);
                    self.clip_set(CLIP_TEXT, t.as_bytes());
                }
            }
            if let Some(b) = new {
                let b = b.clamp(0, n - 1) as u32;
                st.caret = b;
                if shift {
                    sel = range(st.anchor, b);
                } else {
                    st.anchor = b;
                    sel = Some((b, 1));
                    out.clicked_key = mark_at(marks, b).map(|m| m.key);
                }
                st.reveal = Some((b, b));
                self.wp.ctx.focus_visible = true;
            }
        }
        // a byte (range) to show: scrolled into view at once (once the view has a size)
        if vr[3] > 0 {
            if let Some((a, b)) = st.reveal.take() {
                let (ra, rb) = ((a / cu) as i32, (b / cu) as i32);
                let s = self.slot(id);
                let d = into_view(ra * rh, (rb - ra + 1) * rh, s.scrolly, vr[3]);
                s.scrolly += d;
                s.adur = s.scrolly as u32;
            }
        }
        self.scr_offset();
        let y0 = self.box_rect()[1];
        let k = self.wp.ctx.uiclip;
        let first = (k[1].wrapping_sub(y0).max(0) / rh).min(nrows);
        let end = ((k[3].wrapping_sub(y0).max(0) + rh - 1) / rh).min(nrows).max(first);
        // the rows' hit box (the visible part of them); the pointer as the rows read it
        let cid = part(id, 3);
        let (ix, iw) = (vr[0], vr[2]);
        self.ltr_begin(ix, iw);
        let rtl = self.rtl;
        let lx = |x: i32, w: i32| if rtl { ix + ix + iw - x - w } else { x }; // logical x of a span drawn LTR
        let x0 = vr[0] + pad;
        let ch = (y0 + nrows * rh - vr[1]).clamp(0, vr[3]);
        let cev = self.hit_rect(cid, vr[0], vr[1], vr[2], ch, HB_MOVES);
        let (mx, my) = (self.ltr_x(self.wp.ctx.mx), self.wp.ctx.my);
        let (region, j) = cl.pick(cl.at(mx - x0));
        let row = (my - y0).div_euclid(rh);
        let prev_press = st.press;
        if cev & EV_HOVER != 0 && region != 0 && row >= 0 && row < nrows {
            let b = row as u32 * cu + j as u32;
            if b < len {
                out.hover = Some(b);
            }
        }
        // the byte at the pointer, kept inside the data (presses, drags)
        let at = |j: i32| (row.clamp(0, (nrows - 1).max(0)) as u32 * cu + j as u32).min(last);
        let pressed = self.wp.ctx.active == cid && self.wp.ctx.pressed & (1 << BTN_LEFT) != 0;
        if pressed && len > 0 {
            let c = &mut self.wp.ctx;
            c.focus = id;
            c.focus_visible = false;
            let shift = self.mods & MOD_SHIFT != 0;
            if region == 0 && !shift {
                // an offset: its row
                let (a, b) = (at(0), at(cols - 1));
                st.anchor = a;
                st.caret = b;
                sel = range(a, b);
                st.drag = false;
            } else {
                let b = at(j);
                if shift {
                    sel = range(st.anchor, b);
                } else {
                    st.anchor = b;
                    sel = Some((b, 1));
                }
                st.caret = b;
                st.press = b;
                st.drag = true;
                st.moved = false;
            }
        }
        // a double click (on the byte of the first click): the mark's range
        if cev & EV_DOUBLE != 0 && len > 0 && at(j) == prev_press {
            if let Some(m) = mark_at(marks, at(j)) {
                let s = m.start.min(last);
                let l = m.len.min(len - s);
                sel = Some((s, l));
                st.anchor = s;
                st.caret = s + l - 1;
            }
            st.drag = false;
            out.ev |= EV_DOUBLE;
        }
        if cev & EV_HELD != 0 && st.drag && len > 0 {
            let b = at(j);
            if b != st.press {
                st.moved = true;
            }
            if st.moved {
                st.caret = b;
                sel = range(st.anchor, b);
            }
            // above or below the view: scroll a row every 50 ms
            let dir = if my < vr[1] { -1 } else if my >= vr[1] + vr[3] { 1 } else { 0 };
            if dir != 0 {
                let now = self.now_ms;
                if now.wrapping_sub(st.auto_t) >= 50 {
                    st.auto_t = now;
                    let s = self.slot(id);
                    s.scrolly += dir * rh;
                    s.adur = s.scrolly as u32;
                    self.repass();
                }
                self.animating = true;
            }
        }
        if cev & EV_COMMIT != 0 && st.drag {
            st.drag = false;
            if !st.moved {
                out.clicked_key = mark_at(marks, st.press).map(|m| m.key);
            }
        }
        if cev & EV_HELD == 0 && self.wp.ctx.active != cid {
            st.drag = false;
        }
        if cev & EV_RCLICK != 0 && len > 0 {
            let b = at(j);
            if !sel.is_some_and(|(s, l)| b >= s && b - s < l) {
                st.anchor = b;
                st.caret = b;
                sel = Some((b, 1));
            }
            out.ev |= EV_RCLICK;
        }
        if sel != st.sel {
            st.sel = sel;
            out.ev |= EV_ACT;
            self.repass();
        } else if st.caret != caret_in {
            self.repass();
        }
        st.seen = st.sel;
        // the rows' record (the tooltip belongs to it)
        self.wp.ctx.last_rect = [vr[0], vr[1], vr[2], ch];
        self.wp.ctx.last_state = 0;
        self.sem(K_BOX, cid, None, out.hover.map_or(-1, |b| b as i32));
        // (every pass but over the offsets: the tooltip keeps track of the pointer)
        if out.hover.is_some() || cev & EV_HOVER == 0 {
            let b = out.hover.unwrap_or(0);
            self.tooltip(&format!("0x{:0w$X} ({})", b, b, w = od as usize));
        }

        // ---- the visible rows
        let pal = self.style.data().chart_palette(self);
        let alpha: u32 = if self.theme_dark() { 0x5C } else { 0x40 };
        let mcol: Vec<u32> = marks
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let c = if m.color == 0 { pal[i % 8] } else { m.color };
                if c >> 24 == 0xFF { (c & 0x00FF_FFFF) | alpha << 24 } else { c }
            })
            .collect();
        let mut order: Vec<usize> = (0..marks.len()).filter(|&i| marks[i].len > 0).collect();
        order.sort_by(|&a, &b| marks[b].len.cmp(&marks[a].len));
        let acc = self.color(T_ACCENT);
        let selc = (acc & 0x00FF_FFFF) | 0x7000_0000;
        let (t1, t2, t3) = (self.color(T_TEXT), self.color(T_TEXT2), self.color(T_TEXT3));
        let focused = vev & EV_FOCUS != 0;
        let rad = self.dp(3);
        let hair = self.hair();
        let fs = if mono { font } else { ui_font };
        let half = ((adv >> 1) + 128) >> 8; // half a cell (px): marks reach into the blanks
        if first > 0 {
            self.place(0, first * rh);
        }
        for i in first..end {
            let rr = self.place(0, rh);
            let rb = i as u32 * cu;
            let n = (len - rb).min(cu) as i32;
            let base = rr[1] + ((rh - asc - desc) >> 1) + asc;
            // the cells of bytes a ..= b of the row in the hex and the ASCII column
            let span = |a: i32, b: i32| {
                let (h0, h1) = (x0 + cl.px(cl.hx(a)) - half, x0 + cl.px(cl.hx(b) + 2) + half);
                let (a0, a1) = (x0 + cl.px(cl.ax() + a), x0 + cl.px(cl.ax() + b + 1));
                ([h0, rr[1] + 1, h1 - h0, rh - 2], [a0, rr[1] + 1, a1 - a0, rh - 2])
            };
            let row_end = rb as u64 + n as u64 - 1;
            // marks (larger first), the selection
            for &mi in &order {
                let m = &marks[mi];
                let (a, b) = (m.start.max(rb) as u64, (m.start as u64 + m.len as u64 - 1).min(row_end));
                if a > b {
                    continue;
                }
                let (h, s) = span((a - rb as u64) as i32, (b - rb as u64) as i32);
                for q in [h, s] {
                    self.cmd_rrect(q[0], q[1], q[2], q[3], rad, 0, mcol[mi], 0, 0);
                }
            }
            if let Some((ss, sl)) = st.sel {
                let (a, b) = ((ss as u64).max(rb as u64), (ss as u64 + sl as u64 - 1).min(row_end));
                if a <= b {
                    let (h, s) = span((a - rb as u64) as i32, (b - rb as u64) as i32);
                    for q in [h, s] {
                        self.cmd_rrect(q[0], q[1], q[2], q[3], rad, 0, selc, 0, 0);
                    }
                }
            }
            // the byte under the pointer, the caret
            for (b, c, t) in [(out.hover, t3, hair), (focused.then_some(st.caret), acc, hair * 2)] {
                if let Some(b) = b.filter(|&b| b >= rb && b - rb < n as u32) {
                    let (h, s) = span((b - rb) as i32, (b - rb) as i32);
                    for q in [h, s] {
                        self.cmd_rrect(q[0], q[1], q[2], q[3], rad, t, 0, c, c);
                    }
                }
            }
            // the texts: offset, hex digits (zeros dimmed), characters (dots dimmed)
            let off = format!("{:0w$X}", rb, w = od as usize);
            let bytes = &data[rb as usize..(rb + n as u32) as usize];
            let (h0, a0) = (cl.hx(0), cl.ax() - 1);
            if mono {
                let hl = (cl.hx(cols - 1) + 2 - h0) as usize;
                let (mut hv, mut hz) = (alloc::vec![b' '; hl], alloc::vec![b' '; hl]);
                let (mut av, mut ad) = (alloc::vec![b' '; cols as usize + 2], alloc::vec![b' '; cols as usize + 2]);
                ad[0] = b'|';
                ad[n as usize + 1] = b'|';
                for (j, &x) in bytes.iter().enumerate() {
                    let p = (cl.hx(j as i32) - h0) as usize;
                    let d = if x == 0 { &mut hz } else { &mut hv };
                    d[p] = HEX[(x >> 4) as usize];
                    d[p + 1] = HEX[(x & 15) as usize];
                    if (0x20..0x7F).contains(&x) {
                        av[j + 1] = x;
                    } else {
                        ad[j + 1] = b'.';
                    }
                }
                self.cmd_text(x0, base, off.as_bytes(), fs, size, t2, 0);
                for (s, x, c) in [(&hv, h0, t1), (&hz, h0, t3), (&av, a0, t1), (&ad, a0, t3)] {
                    let e = s.iter().rposition(|&b| b != b' ').map_or(0, |p| p + 1);
                    if e > 0 {
                        self.cmd_text(x0 + cl.px(x), base, &s[..e], fs, size, c, 0);
                    }
                }
            } else {
                self.cmd_text(x0, base, off.as_bytes(), fs, size, t2, 0);
                for (j, &x) in bytes.iter().enumerate() {
                    let j = j as i32;
                    let hs = [HEX[(x >> 4) as usize], HEX[(x & 15) as usize]];
                    self.cmd_text(x0 + cl.px(cl.hx(j)), base, &hs, fs, size, if x == 0 { t3 } else { t1 }, 0);
                    let (a, c) = if (0x20..0x7F).contains(&x) { (x, t1) } else { (b'.', t3) };
                    self.cmd_text(x0 + cl.px(cl.ax() + j), base, &[a], fs, size, c, 0);
                }
                self.cmd_text(x0 + cl.px(a0), base, b"|", fs, size, t3, 0);
                self.cmd_text(x0 + cl.px(cl.ax() + n), base, b"|", fs, size, t3, 0);
            }
            // records: offset, hex, characters
            let (hx0, hw) = (x0 + cl.px(h0), cl.px(cl.hx(n - 1) + 2) - cl.px(h0));
            let (ax0, aw) = (x0 + cl.px(a0), cl.px(a0 + 1 + n) - cl.px(a0)); // '|' and the characters
            let ow = cl.px(od);
            let asc_s: String = bytes.iter().map(|&x| if (0x20..0x7F).contains(&x) { x as char } else { '.' }).collect();
            self.wp.ctx.last_state = 0;
            self.wp.ctx.last_rect = [lx(x0, ow), rr[1], ow, rh];
            self.sem(K_TEXT, 0, Some(&off), rb as i32);
            self.wp.ctx.last_rect = [lx(hx0, hw), rr[1], hw, rh];
            self.sem(K_TEXT, 0, Some(&hex_text(bytes)), n);
            self.wp.ctx.last_rect = [lx(ax0, aw), rr[1], aw, rh];
            self.sem(K_TEXT, 0, Some(&asc_s), n);
        }
        if nrows > end {
            self.place(0, (nrows - end) * rh);
        }
        self.ltr_end();
        self.scr_close();
        // the view's record and focus visual
        let mut ss = 0u32;
        if (vev | cev) & EV_HOVER != 0 {
            ss |= SS_HOVER as u32;
        }
        if focused {
            ss |= SS_FOCUSED as u32;
        }
        self.wp.ctx.last_state = ss;
        self.sem(K_LIST, id, None, st.caret as i32).max = len.min(i32::MAX as u32) as i32;
        if focused && self.wp.ctx.focus_visible {
            let s = self.style;
            s.focus_rect(self, vr[0], vr[1], vr[2], vr[3]);
        }
        out.ev |= ((vev | cev) & EV_HOVER) | (vev & EV_FOCUS);
        out
    }
}
