// textool -- the text tool: a text box on the canvas with a caret and the contextual
// ribbon group "Text"; the box is rasterized into the layer on commit
//   [apps/paint/src/textool.s]
//
//   text_input(p, ui, in) -> true canvas input while TOOL_TEXT is active: a press creates a
//                                box (dragging sets its size) or places the caret; the band
//                                around the frame moves it, the handles resize it; typed
//                                text, Backspace, Delete, arrows, Home / End, Enter (new
//                                line), Ctrl+V (paste), Ctrl+C (copy the text), Esc
//                                (commit); a press outside commits
//   text_overlay(p, ui)          the box preview scaled with the zoom, dashed frame, handles
//                                and caret; commits when the tool changed
//   text_ribbon(p, ui)           group "Text" (only while TOOL_TEXT is active): font style
//                                (JFLib Sans Normal / Halbfett / Fett), size 8..72 pt, Fett,
//                                Kursiv, Unterstrichen, Durchgestrichen, Hintergrund füllen
//                                (colour 2 behind the text)
//   text_cmd(p, ui, id, arg) -> false
//
// The box is rendered into its own PARGB surface at document resolution: glyph outlines
// through the path API (glyph_path with the gp transform), so italic is a shear of the
// flattened points around the baseline.  The preview shows that surface scaled; the
// commit blends it into the current layer between hist_begin and hist_end, so preview and
// result are the same pixels.  Sizes: 1 pt = 4/3 doc px.  Colour 1 draws the text (colour
// 2 the background).

use alloc::rc::Rc;
use alloc::vec::Vec;

use gfx::{Surface, FMT_PARGB};
use rt::fx::cvt;
use text::{Gp, FONT_BOLD};
use ui::*;

use crate::consts::*;
use crate::Kritzel;

const BTN_L_MASK: u32 = 1 << BTN_LEFT;
const TX_CAP: usize = 4096; // text bytes (UTF-8, with the asm's NUL)
const ID_TXT_FONT: u32 = 1600;
const ID_TXT_SIZE: u32 = 1601;
const ID_TXT_BOLD: u32 = 1602;
const ID_TXT_ITALIC: u32 = 1603;
const ID_TXT_UNDER: u32 = 1604;
const ID_TXT_STRIKE: u32 = 1605;
const ID_TXT_FILL: u32 = 1606;
const DR_NONE: u32 = 0; // drag modes
const DR_CREATE: u32 = 1;
const DR_MOVE: u32 = 2;
const DR_SIZE: u32 = 3;
const HM_L: u32 = 1; // handle edges
const HM_R: u32 = 2;
const HM_T: u32 = 4;
const HM_B: u32 = 8;
const ZN_OUTSIDE: u32 = 0; // tx_zone results
const ZN_INSIDE: u32 = 1;
const ZN_BORDER: u32 = 2;
const ZN_HANDLE: u32 = 3;
const MIN_BOX: i32 = 16; // smallest box side (doc px)
const F_SLANT: f32 = 0.2;
const TX_RANGE_PT: Range = Range::new(8, 72, 1, 0);
// handles: third of width, third of height (0..2 halves), edges
const TX_HANDLES: [(i32, i32, u32); 8] = [
    (0, 0, HM_L | HM_T),
    (1, 0, HM_T),
    (2, 0, HM_R | HM_T),
    (0, 1, HM_L),
    (2, 1, HM_R),
    (0, 2, HM_L | HM_B),
    (1, 2, HM_B),
    (2, 2, HM_R | HM_B),
];
// font style (JFLib Sans Regular / Semibold / Bold)
const TX_FONTS: [&str; 3] = ["Normal", "Halbfett", "Fett"];

/// the tool's state (the asm's .data / .bss)
pub(crate) struct TextTool {
    /// FONT_REG / FONT_SEMI / FONT_BOLD
    pub(crate) weight: i32,
    /// size in points
    pub(crate) pt: i32,
    pub(crate) bold: i32,
    pub(crate) italic: i32,
    pub(crate) under: i32,
    pub(crate) strike: i32,
    pub(crate) fill: i32,
    sf: Option<Rc<Surface>>, // rendered box (PARGB, doc px)
    view: [i32; 4],          // canvas widget (screen px)
    srect: [i32; 4],         // box on screen
    buf: Vec<u8>,
    open: bool,
    x: i32, // box origin (doc px)
    y: i32,
    uw: i32, // size asked for by the user (0 = fit)
    uh: i32,
    w: i32, // actual size (at least the text)
    h: i32,
    caret: usize,
    drag: u32,
    mask: u32,
    ax: i32, // drag anchor / grab offset
    ay: i32,
    dirty: bool,
    ver: u32,
    rkey: u32, // settings of the rendered surface
    rcol1: u32,
    rcol2: u32,
    font: u32, // metrics of the selected box font
    fx: i32,
    sx: f32,
    asc: i32,
    lh: i32,
    pad: i32,
    nlines: i32,
    sfont: u32, // saved UI font
    ssize: i32,
}

impl Default for TextTool {
    fn default() -> TextTool {
        TextTool {
            weight: 0,
            pt: 16,
            bold: 0,
            italic: 0,
            under: 0,
            strike: 0,
            fill: 0,
            sf: None,
            view: [0; 4],
            srect: [0; 4],
            buf: Vec::new(),
            open: false,
            x: 0,
            y: 0,
            uw: 0,
            uh: 0,
            w: 0,
            h: 0,
            caret: 0,
            drag: DR_NONE,
            mask: 0,
            ax: 0,
            ay: 0,
            dirty: false,
            ver: 0,
            rkey: 0,
            rcol1: 0,
            rcol2: 0,
            font: 0,
            fx: 0,
            sx: 0.0,
            asc: 0,
            lh: 0,
            pad: 0,
            nlines: 0,
            sfont: 0,
            ssize: 0,
        }
    }
}

// ------------------------------------------------------------------ UTF-8 (core/str.s)

// utf8_decode(s, i) -> codepoint, next index: invalid bytes, overlong forms, surrogates
// and values above U+10FFFF decode as U+FFFD and advance one byte; the end returns 0
fn utf8_decode(s: &[u8], i: usize) -> (u32, usize) {
    let Some(&b0) = s.get(i) else { return (0, i) };
    let b0 = b0 as u32;
    if b0 < 0x80 {
        return (b0, i + 1);
    }
    let bad = (0xFFFD, i + 1);
    let (n, mut cp, min) = match b0 {
        0xC2..=0xDF => (1, b0 & 0x1F, 0x80),
        0xE0..=0xEF => (2, b0 & 0x0F, 0x800),
        0xF0..=0xF4 => (3, b0 & 7, 0x10000),
        _ => return bad, // continuation byte, overlong lead, > U+10FFFF
    };
    let mut j = i + 1;
    for _ in 0..n {
        let Some(&c) = s.get(j) else { return bad };
        if c & 0xC0 != 0x80 {
            return bad;
        }
        cp = cp << 6 | (c & 0x3F) as u32;
        j += 1;
    }
    if cp < min || cp > 0x10FFFF || cp & 0xFFFF_F800 == 0xD800 {
        return bad;
    }
    (cp, j)
}

// utf8_encode(cp) -> bytes, count (1..4; above U+10FFFF: U+FFFD)
fn utf8_encode(cp: u32) -> ([u8; 4], usize) {
    let mut b = [0u8; 4];
    if cp < 0x80 {
        b[0] = cp as u8;
        return (b, 1);
    }
    if cp < 0x800 {
        b[0] = 0xC0 | (cp >> 6) as u8;
        b[1] = 0x80 | (cp & 0x3F) as u8;
        return (b, 2);
    }
    let cp = if cp >= 0x10000 && cp > 0x10FFFF { 0xFFFD } else { cp };
    if cp < 0x10000 {
        b[0] = 0xE0 | (cp >> 12) as u8;
        b[1] = 0x80 | ((cp >> 6) & 0x3F) as u8;
        b[2] = 0x80 | (cp & 0x3F) as u8;
        return (b, 3);
    }
    b[0] = 0xF0 | (cp >> 18) as u8;
    b[1] = 0x80 | ((cp >> 12) & 0x3F) as u8;
    b[2] = 0x80 | ((cp >> 6) & 0x3F) as u8;
    b[3] = 0x80 | (cp & 0x3F) as u8;
    (b, 4)
}

impl TextTool {
    // tx_prev(i) -> start of the character before byte i
    fn prev(&self, i: usize) -> usize {
        let mut j = i;
        loop {
            if j == 0 {
                return 0;
            }
            j -= 1;
            if j == 0 {
                return 0;
            }
            if self.buf[j] & 0xC0 != 0x80 {
                return j;
            }
        }
    }

    // tx_next(i) -> start of the character after byte i (i at the end: i)
    fn next(&self, i: usize) -> usize {
        if i >= self.buf.len() {
            return i;
        }
        utf8_decode(&self.buf, i).1
    }

    // tx_line_start(i) -> start of the line containing byte i
    fn line_start(&self, i: usize) -> usize {
        let mut j = i;
        while j > 0 && self.buf[j - 1] != b'\n' {
            j -= 1;
        }
        j
    }

    // tx_line_end(i) -> index of the newline ending the line at byte i (or the length)
    fn line_end(&self, i: usize) -> usize {
        let mut j = i;
        while j < self.buf.len() && self.buf[j] != b'\n' {
            j += 1;
        }
        j
    }

    // tx_insert(cp) -- insert a codepoint at the caret
    fn insert(&mut self, cp: u32) {
        let (b, n) = utf8_encode(cp);
        if self.buf.len() + n >= TX_CAP - 1 {
            return;
        }
        let c = self.caret;
        self.buf.splice(c..c, b[..n].iter().copied());
        self.caret += n;
        self.dirty = true;
    }

    // tx_remove(a, b) -- delete the bytes a..b-1; the caret goes to a
    fn remove(&mut self, a: usize, b: usize) {
        if a >= b {
            return;
        }
        self.buf.drain(a..b);
        self.caret = a;
        self.dirty = true;
    }

    // tx_style_key() -> all settings that change the rendering
    fn style_key(&self) -> u32 {
        (self.pt as u32) << 8
            | self.weight as u32
            | (self.bold as u32) << 2
            | (self.italic as u32) << 3
            | (self.under as u32) << 4
            | (self.strike as u32) << 5
            | (self.fill as u32) << 6
    }

    // tx_screen_rect() -- srect = the box on screen (px)
    fn screen_rect(&mut self, zoom: i32, cx: i32, cy: i32) {
        self.srect[0] = (self.x.wrapping_mul(zoom) / 1000).wrapping_add(cx);
        self.srect[1] = (self.y.wrapping_mul(zoom) / 1000).wrapping_add(cy);
        self.srect[2] = (self.w.wrapping_mul(zoom) / 1000).max(1);
        self.srect[3] = (self.h.wrapping_mul(zoom) / 1000).max(1);
    }
}

// ------------------------------------------------------------------ input

/// text_input(in) -> true
pub(crate) fn text_input(pt: &mut Kritzel, ui: &mut Ui, inp: &UiInput) -> bool {
    pt.txt.view = inp.rect;
    let mx = inp.x.wrapping_add(inp.rect[0]);
    let my = inp.y.wrapping_add(inp.rect[1]);
    let (px, py) = view_to_doc(pt, mx, my);
    if pt.txt.drag != DR_NONE {
        // dragging: create / move / resize follows the pointer
        tx_drag_to(&mut pt.txt, px, py);
        if inp.btn & BTN_L_MASK == 0 {
            tx_drag_end(&mut pt.txt);
        }
    } else if inp.pressed & BTN_L_MASK != 0 && inp.ev & EV_HOVER != 0 {
        ui.focus(0); // typing belongs to the box now
        if pt.txt.open {
            let (zone, edges) = tx_zone(pt, ui, mx, my);
            match zone {
                ZN_INSIDE => {
                    tx_font(&mut pt.txt, ui);
                    tx_layout(&mut pt.txt, ui);
                    let t = &mut pt.txt;
                    t.caret = tx_hit(t, ui, px.wrapping_sub(t.x), py.wrapping_sub(t.y));
                    tx_unfont(&pt.txt, ui);
                }
                ZN_BORDER => {
                    let t = &mut pt.txt;
                    t.drag = DR_MOVE;
                    t.ax = px.wrapping_sub(t.x);
                    t.ay = py.wrapping_sub(t.y);
                }
                ZN_HANDLE => {
                    pt.txt.drag = DR_SIZE;
                    pt.txt.mask = edges;
                }
                _ => {
                    tx_commit(pt, ui); // a press outside the box
                    return true;
                }
            }
        } else {
            // a new box at the press (inside the image)
            let x = px.min(doc_w(pt) - 1).max(0);
            let y = py.min(doc_h(pt) - 1).max(0);
            let t = &mut pt.txt;
            t.ax = x;
            t.ay = y;
            t.x = x;
            t.y = y;
            t.uw = 0;
            t.uh = 0;
            t.buf.clear();
            t.caret = 0;
            t.open = true;
            t.dirty = true;
            t.drag = DR_CREATE;
            return true;
        }
    }
    if pt.txt.open && pt.txt.drag != DR_CREATE {
        tx_keys(pt, ui);
    }
    true
}

// tx_drag_to(x, y) -- the pointer (doc px) during a create / move / resize drag
fn tx_drag_to(t: &mut TextTool, x: i32, y: i32) {
    match t.drag {
        DR_CREATE => {
            t.x = x.min(t.ax);
            t.uw = x.max(t.ax).wrapping_sub(t.x);
            t.y = y.min(t.ay);
            t.uh = y.max(t.ay).wrapping_sub(t.y);
        }
        DR_MOVE => {
            t.x = x.wrapping_sub(t.ax);
            t.y = y.wrapping_sub(t.ay);
            return;
        }
        DR_SIZE => {
            let m = t.mask;
            if m & HM_R != 0 {
                t.uw = x.wrapping_sub(t.x).max(MIN_BOX);
            }
            if m & HM_L != 0 {
                let right = t.x.wrapping_add(t.w); // right edge stays
                let nx = x.min(right - MIN_BOX);
                t.x = nx;
                t.uw = right.wrapping_sub(nx);
            }
            if m & HM_B != 0 {
                t.uh = y.wrapping_sub(t.y).max(MIN_BOX);
            }
            if m & HM_T != 0 {
                let bottom = t.y.wrapping_add(t.h); // bottom edge stays
                let ny = y.min(bottom - MIN_BOX);
                t.y = ny;
                t.uh = bottom.wrapping_sub(ny);
            }
        }
        _ => return,
    }
    t.dirty = true;
}

// tx_drag_end() -- button released; a click (no real drag) creates a default box
fn tx_drag_end(t: &mut TextTool) {
    if t.drag == DR_CREATE && t.uw < 8 && t.uh < 8 {
        t.x = t.ax;
        t.y = t.ay;
        t.uw = (t.pt << 4).max(120); // about 12 em wide
        t.uh = 0;
    }
    t.drag = DR_NONE;
    t.dirty = true;
}

// tx_zone(x, y) -> ZN_*, handle edges -- where a press (screen px) hits the box
fn tx_zone(pt: &mut Kritzel, ui: &Ui, x: i32, y: i32) -> (u32, u32) {
    let (zoom, cx, cy) = view(pt);
    let t = &mut pt.txt;
    t.screen_rect(zoom, cx, cy);
    let r = t.srect;
    let reach = ui.dp(6); // handle reach
    for &(hx, hy, edges) in TX_HANDLES.iter() {
        let ex = (r[2].wrapping_mul(hx) >> 1).wrapping_add(r[0]).wrapping_sub(x);
        if ex.wrapping_abs() > reach {
            continue;
        }
        let ey = (r[3].wrapping_mul(hy) >> 1).wrapping_add(r[1]).wrapping_sub(y);
        if ey.wrapping_abs() > reach {
            continue;
        }
        return (ZN_HANDLE, edges);
    }
    let dx = x.wrapping_sub(r[0]);
    let dy = y.wrapping_sub(r[1]);
    if dx >= 0 && dx < r[2] && dy >= 0 && dy < r[3] {
        return (ZN_INSIDE, 0);
    }
    // up to 8 DIP around the frame: move
    let b = ui.dp(8);
    let x0 = r[0].wrapping_sub(b);
    let y0 = r[1].wrapping_sub(b);
    if x < x0 || x >= x0.wrapping_add(r[2]).wrapping_add(b * 2) {
        return (ZN_OUTSIDE, 0);
    }
    if y < y0 || y >= y0.wrapping_add(r[3]).wrapping_add(b * 2) {
        return (ZN_OUTSIDE, 0);
    }
    (ZN_BORDER, 0)
}

// ------------------------------------------------------------------ editing

// tx_keys() -- typed text and the editing keys of the open box
fn tx_keys(pt: &mut Kritzel, ui: &mut Ui) {
    for cp in ui.text_take(64) {
        if cp >= 32 && cp != 127 {
            pt.txt.insert(cp);
        }
    }
    // Ctrl+V pastes, Ctrl+C copies the whole text; Ctrl+X / Ctrl+A are taken so the
    // selection accelerators do not act while typing
    if ui.mods() & MOD_CTRL != 0 {
        if ui.key(VK_V) {
            tx_paste(&mut pt.txt, ui);
        }
        if ui.key(VK_C) {
            ui.clip_set(CLIP_TEXT, &pt.txt.buf);
        }
        ui.key(VK_X);
        ui.key(VK_A);
    }
    let t = &mut pt.txt;
    while ui.key(VK_BACK) {
        let a = t.prev(t.caret);
        t.remove(a, t.caret);
    }
    while ui.key(VK_DELETE) {
        let b = t.next(t.caret);
        t.remove(t.caret, b);
    }
    while ui.key(VK_LEFT) {
        t.caret = t.prev(t.caret);
    }
    while ui.key(VK_RIGHT) {
        t.caret = t.next(t.caret);
    }
    if ui.key(VK_HOME) {
        t.caret = t.line_start(t.caret);
    }
    if ui.key(VK_END) {
        t.caret = t.line_end(t.caret);
    }
    while ui.key(VK_UP) {
        tx_vert(t, ui, -1);
    }
    while ui.key(VK_DOWN) {
        tx_vert(t, ui, 1);
    }
    while ui.key(VK_RETURN) {
        t.insert(10);
    }
    if ui.key(VK_ESCAPE) {
        tx_commit(pt, ui);
    }
}

// tx_paste() -- insert the clipboard text at the caret (line breaks kept, CR dropped)
fn tx_paste(t: &mut TextTool, ui: &mut Ui) {
    let Some(data) = ui.clip_get(CLIP_TEXT) else { return };
    let mut i = 0;
    loop {
        let (cp, next) = utf8_decode(&data, i);
        if cp == 0 {
            break;
        }
        i = next;
        if cp == 10 || (cp >= 32 && cp != 127) {
            t.insert(cp);
        }
    }
}

// tx_vert(dir) -- caret one line up (-1) or down (+1) near the same x
fn tx_vert(t: &mut TextTool, ui: &mut Ui, dir: i32) {
    tx_font(t, ui);
    let ls = t.line_start(t.caret);
    let cx = (ui.text.measure_n(&t.buf[ls..], t.caret - ls) + 128) >> 8; // caret x (px)
    let caret = 'v: {
        let start = if dir >= 0 {
            let le = t.line_end(t.caret);
            if le >= t.buf.len() {
                break 'v le; // last line: to its end
            }
            le + 1
        } else {
            if ls == 0 {
                break 'v 0; // first line: to the start
            }
            t.line_start(ls - 1)
        };
        let le = t.line_end(start);
        start + ui.text.hit(&t.buf[start..], Some(le - start), cx)
    };
    t.caret = caret;
    tx_unfont(t, ui);
}

// tx_hit(x, y) -> byte index of the caret closest to x, y (doc px in the box; the box font
// must be selected and the layout current)
fn tx_hit(t: &TextTool, ui: &mut Ui, x: i32, y: i32) -> usize {
    let lh = if t.lh == 0 { 1 } else { t.lh };
    let mut line = (y.wrapping_sub(t.pad) / lh).max(0);
    line = line.min(t.nlines - 1);
    let mut i = 0;
    while line > 0 {
        let le = t.line_end(i);
        if le >= t.buf.len() {
            break;
        }
        i = le + 1;
        line -= 1;
    }
    let le = t.line_end(i);
    i + ui.text.hit(&t.buf[i..], Some(le - i), x.wrapping_sub(t.pad))
}

// tx_caret_pos() -> x (doc px in the box), line of the caret (font selected)
fn tx_caret_pos(t: &TextTool, ui: &mut Ui) -> (i32, i32) {
    let ls = t.line_start(t.caret);
    let line = t.buf[..ls].iter().filter(|&&b| b == b'\n').count() as i32;
    let x = ((ui.text.measure_n(&t.buf[ls..], t.caret - ls) + 128) >> 8) + t.pad;
    (x, line)
}

// ------------------------------------------------------------------ layout and rendering

// tx_font() -- select the box font (the UI font is saved) and compute the metrics
fn tx_font(t: &mut TextTool, ui: &mut Ui) {
    let (f, s) = ui.text.get_font();
    t.sfont = f;
    t.ssize = s;
    let mut font = t.weight as u32;
    if font > FONT_BOLD {
        font = 0;
    }
    if t.bold != 0 {
        font = FONT_BOLD;
    }
    t.font = font;
    let p = t.pt.clamp(8, 72);
    t.fx = (((p as u32) << 10) / 3) as i32; // pt * 4/3 px in 24.8
    ui.text.set_font(t.font, t.fx);
    t.sx = ui.text.gc_scale(t.font, t.fx).0;
    t.asc = ui.text.ascent();
    t.lh = ui.text.line_h();
    t.pad = ((t.lh as u32) >> 2) as i32 + 2; // padding: a quarter line + 2 px
}

// tx_unfont() -- restore the UI font
fn tx_unfont(t: &TextTool, ui: &mut Ui) {
    ui.text.set_font(t.sfont, t.ssize);
}

// tx_layout() -- number of lines and the box size (doc px) from text and user size
fn tx_layout(t: &mut TextTool, ui: &mut Ui) {
    let mut widest = 0; // 24.8
    let mut lines = 0;
    let mut ls = 0;
    loop {
        let le = t.line_end(ls);
        let w = ui.text.measure_n(&t.buf[ls..], le - ls);
        widest = widest.max(w);
        lines += 1;
        if le >= t.buf.len() {
            break;
        }
        ls = le + 1;
    }
    t.nlines = lines;
    let mut w = (widest.wrapping_add(255) >> 8) + t.pad * 2 + 2;
    if t.italic != 0 {
        w += (((t.asc.wrapping_mul(52)) as u32 >> 8) as i32) + 1; // slant 0.2 of the ascent
    }
    t.w = w.max(t.uw);
    t.h = (t.lh.wrapping_mul(lines) + t.pad * 2).max(t.uh);
}

// tx_units(units) -> round(units * sx * 256) -- as text_measure computes advances
fn tx_units(sx: f32, u: i32) -> i32 {
    cvt(u as f32 * sx * 256.0)
}

// tx_draw_line(s, x, baseline) -- fill the glyph outlines of one line at x (px) with the
// current colour; italic shears the points by 0.2 around the baseline
fn tx_draw_line(t: &TextTool, ui: &mut Ui, sf: &mut Surface, s: &[u8], x: i32, baseline: i32) {
    let Ui { dc, text, .. } = ui;
    let mut pen = x << 8; // 24.8
    let base = baseline as f32;
    let font = t.font;
    let mut prev: i32 = -1;
    text.path.begin(dc);
    let mut i = 0;
    while i < s.len() {
        let (cp, next) = utf8_decode(s, i);
        i = next;
        if cp == 0 || cp == 10 {
            break;
        }
        let g = text.glyph(font, cp);
        if prev >= 0 {
            let k = text.kern(font, prev as u32, g);
            if k != 0 {
                pen = pen.wrapping_add(tx_units(t.sx, k));
            }
        }
        text.gp = Gp { x: pen as f32 * 0.00390625, y: base, sx: t.sx, sy: t.sx, dx: 0, dy: 0 };
        text.glyph_path(font, g);
        let adv = text.adv(font, g);
        pen = pen.wrapping_add(tx_units(t.sx, adv));
        prev = g as i32;
    }
    if t.italic != 0 {
        for p in text.path.pts_mut() {
            p.0 = (base - p.1) * F_SLANT + p.0;
        }
    }
    text.path.fill(&mut text.ras, dc, sf, 0);
}

// tx_render() -- background, lines and decorations into sf (font selected)
fn tx_render(pt: &mut Kritzel, ui: &mut Ui) {
    let (col1, col2) = colors(pt);
    let t = &mut pt.txt;
    // tx_surf_ensure: a PARGB surface of the box size (its pixels are all cleared below)
    let mut sf = match t.sf.take().and_then(|r| Rc::try_unwrap(r).ok()) {
        Some(s) if s.w == t.w && s.h == t.h && !s.px.is_empty() => s,
        _ => Surface::new(t.w, t.h, FMT_PARGB),
    };
    if sf.w != t.w || sf.h != t.h {
        return;
    }
    ui.dc.begin(&sf);
    ui.dc.set_opacity(255);
    let bg = if t.fill != 0 { premul(col2) } else { 0 };
    ui.dc.clear(&mut sf, bg);
    ui.dc.set_color(col1);
    // decoration thickness max(1, px / 16)
    let thick = ((((t.fx as u32).wrapping_add(8 << 8)) >> 12) as i32).max(1);
    let mut ls = 0;
    let mut base = t.pad + t.asc; // baseline
    loop {
        let le = t.line_end(ls);
        let line = t.buf[ls..le].to_vec();
        tx_draw_line(t, ui, &mut sf, &line, t.pad, base);
        let lw = (ui.text.measure_n(&line, line.len()).wrapping_add(255)) >> 8; // line width px
        if lw != 0 {
            if t.under != 0 {
                let d = ((((t.fx as u32).wrapping_mul(20)) >> 16) as i32).max(1); // 0.08 px below
                ui.dc.fill(&mut sf, t.pad, base + d, lw, thick);
            }
            if t.strike != 0 {
                let d = (((t.fx as u32).wrapping_mul(77)) >> 16) as i32; // 0.3 px above
                ui.dc.fill(&mut sf, t.pad, base - d, lw, thick);
            }
        }
        if le >= t.buf.len() {
            break;
        }
        ls = le + 1;
        base += t.lh;
    }
    ui.dc.end();
    t.ver = t.ver.wrapping_add(1);
    t.sf = Some(Rc::new(sf));
}

// tx_update() -- layout and render the box now
fn tx_update(pt: &mut Kritzel, ui: &mut Ui) {
    tx_font(&mut pt.txt, ui);
    tx_layout(&mut pt.txt, ui);
    tx_render(pt, ui);
    tx_unfont(&pt.txt, ui);
    let (col1, col2) = colors(pt);
    let t = &mut pt.txt;
    t.rkey = t.style_key();
    t.rcol1 = col1;
    t.rcol2 = col2;
    t.dirty = false;
}

// tx_commit() -- blend the box into the current layer (undoable) and close it
fn tx_commit(pt: &mut Kritzel, ui: &mut Ui) {
    let t = &mut pt.txt;
    if !t.open {
        return;
    }
    t.open = false;
    t.drag = DR_NONE;
    if t.buf.is_empty() && t.fill == 0 {
        return;
    }
    tx_update(pt, ui);
    let Some(sf) = pt.txt.sf.clone() else { return };
    let layer = cur_layer(pt);
    hist_begin(pt, layer);
    let (x, y, w, h) = (pt.txt.x, pt.txt.y, pt.txt.w, pt.txt.h);
    {
        let dc = &mut ui.dc;
        let dst = layer_mut(pt, layer);
        dc.begin(dst);
        dc.set_opacity(255);
        dc.blit(dst, &sf, x, y);
        dc.end();
    }
    let (x1, y1) = (x.wrapping_add(w), y.wrapping_add(h));
    doc_mark(pt, x, y, x1, y1);
    hist_end(pt, x, y, x1, y1);
}

// ------------------------------------------------------------------ overlay

/// text_overlay() -- the open box: preview, dashed frame, handles, caret
pub(crate) fn text_overlay(pt: &mut Kritzel, ui: &mut Ui) {
    if !pt.txt.open {
        return;
    }
    if tool(pt) != TOOL_TEXT {
        tx_commit(pt, ui); // another tool was chosen
        return;
    }
    let (col1, col2) = colors(pt);
    let t = &pt.txt;
    if t.style_key() != t.rkey || col1 != t.rcol1 || col2 != t.rcol2 || t.dirty {
        tx_update(pt, ui);
    }
    let (zoom, cx, cy) = view(pt);
    let t = &mut pt.txt;
    t.screen_rect(zoom, cx, cy);
    let v = t.view;
    ui.draw_clip(v[0], v[1], v[2], v[3]);
    let r = t.srect;
    if let Some(sf) = &t.sf {
        ui.draw_image(sf, t.ver, r[0], r[1], r[2], r[3]);
    }
    tx_draw_frame(t, ui);
    if t.drag == DR_NONE {
        // caret
        tx_font(t, ui);
        let (x, line) = tx_caret_pos(t, ui);
        let top = line.wrapping_mul(t.lh) + t.pad; // caret top (doc px in the box)
        let lh = t.lh;
        tx_unfont(t, ui);
        let cxp = (x.wrapping_mul(zoom) / 1000).wrapping_add(r[0]);
        let cyp = (top.wrapping_mul(zoom) / 1000).wrapping_add(r[1]);
        let ch = lh.wrapping_mul(zoom) / 1000;
        let cw = (zoom / 1000).max(ui.hair()); // caret width: 1 doc px
        ui.draw_color(0xFF00_0000);
        ui.draw_rect(cxp, cyp, cw, ch);
    }
    ui.draw_unclip();
}

// tx_dash_range(a, b, view_start, view_len, period) -> first dash start, end -- the dashes
// of the edge a..b that can be visible
fn tx_dash_range(a: i32, b: i32, vs: i32, vl: i32, period: i32) -> (i32, i32) {
    let end = b.min(vs.wrapping_add(vl));
    let mut start = a;
    if vs > a {
        // skip whole periods before the view
        let n = (vs.wrapping_sub(a) as u32) / (period as u32);
        start = (n.wrapping_mul(period as u32) as i32).wrapping_add(a);
    }
    (start, end)
}

// tx_draw_frame() -- dashed outline just outside srect and the eight handles (dashes only
// inside the visible canvas)
fn tx_draw_frame(t: &TextTool, ui: &mut Ui) {
    ui.draw_color(0xFF5C_5C5C);
    let th = ui.hair(); // thickness
    let dash = ui.dp(4);
    let period = ui.dp(3) + dash;
    let r = t.srect;
    let x0 = r[0] - th;
    let y0 = r[1] - th;
    let x1 = r[0] + r[2];
    let y1 = r[1] + r[3];
    // horizontal edges
    let (mut s, e) = tx_dash_range(x0, x1 + th, t.view[0], t.view[2], period);
    while s < e {
        let l = (e - s).min(dash);
        ui.draw_rect(s, y0, l, th);
        ui.draw_rect(s, y1, l, th);
        s += period;
    }
    // vertical edges
    let (mut s, e) = tx_dash_range(y0, y1 + th, t.view[1], t.view[3], period);
    while s < e {
        let l = (e - s).min(dash);
        ui.draw_rect(x0, s, th, l);
        ui.draw_rect(x1, s, th, l);
        s += period;
    }
    // handles
    let hs = ui.dp(7); // handle size
    let half = ((hs as u32) >> 1) as i32;
    for &(hx, hy, _) in TX_HANDLES.iter() {
        let x = (r[2].wrapping_mul(hx) >> 1) + r[0] - half;
        let y = (r[3].wrapping_mul(hy) >> 1) + r[1] - half;
        ui.draw_color(0xFF8A_8A8A);
        ui.draw_rect(x, y, hs, hs);
        ui.draw_color(0xFFFF_FFFF);
        ui.draw_rect(x + 1, y + 1, hs - 2, hs - 2);
    }
}

// ------------------------------------------------------------------ ribbon

/// text_ribbon() -- the contextual group "Text" while the text tool is active
pub(crate) fn text_ribbon(pt: &mut Kritzel, ui: &mut Ui) {
    if tool(pt) != TOOL_TEXT {
        return;
    }
    ui.box_begin(0, BOX_V | BOX_XCENTER, 2);
    ui.next_size(0, 64);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 2);
    ui.box_begin(0, BOX_V | BOX_CENTER, 2);
    // font and size
    ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
    let t = &mut pt.txt;
    if ui.combobox(ID_TXT_FONT, &mut t.weight, &TX_FONTS) & EV_ACT != 0 {
        tx_refocus(t, ui);
    }
    ui.next_size(44, 0);
    if ui.number_box(ID_TXT_SIZE, &mut t.pt, &TX_RANGE_PT) & EV_COMMIT != 0 {
        tx_refocus(t, ui);
    }
    ui.box_end();
    // style toggles
    ui.box_begin(0, BOX_H | BOX_XCENTER, 2);
    tx_toggle(ui, ID_TXT_BOLD, IC_BOLD, "Fett", &mut t.bold);
    tx_toggle(ui, ID_TXT_ITALIC, IC_ITALIC, "Kursiv", &mut t.italic);
    tx_toggle(ui, ID_TXT_UNDER, IC_UNDERLINE, "Unterstrichen", &mut t.under);
    tx_toggle(ui, ID_TXT_STRIKE, IC_STRIKETHROUGH, "Durchgestrichen", &mut t.strike);
    tx_toggle(ui, ID_TXT_FILL, IC_DROP, "Hintergrund füllen", &mut t.fill);
    ui.box_end();
    ui.box_end();
    // caption and divider (like the other ribbon groups)
    ui.box_end();
    ui.text(TS_CAPTION | TX_SECONDARY, "Text");
    ui.box_end();
    ui.separator();
}

// tx_toggle(id, icon, label, &v) -- compact icon toggle that never takes the focus
fn tx_toggle(ui: &mut Ui, id: u32, icon: u32, label: &str, v: &mut i32) {
    ui.next_flags(ST_SUBTLE | ST_ICON_ONLY | ST_NOFOCUS | ST_COMPACT);
    ui.next_icon(icon);
    ui.toggle_button(id, label, v);
}

// tx_refocus() -- after the font / size was chosen, typing goes back to the open box
fn tx_refocus(t: &TextTool, ui: &mut Ui) {
    if t.open {
        ui.focus(0);
    }
}

/// text_cmd(id, arg) -> false (no commands of its own)
pub(crate) fn text_cmd(_pt: &mut Kritzel, _ui: &mut Ui, _id: u32, _arg: u32) -> bool {
    false
}

// ------------------------------------------------------------------ paint-core glue

fn colors(pt: &Kritzel) -> (u32, u32) {
    (pt.tools.col1, pt.tools.col2)
}

fn view(pt: &Kritzel) -> (i32, i32, i32) {
    (pt.view.zoom, pt.view.cx, pt.view.cy)
}

fn tool(pt: &Kritzel) -> i32 {
    pt.tools.tool
}

fn premul(argb: u32) -> u32 {
    crate::tools::premul(argb)
}

fn view_to_doc(pt: &Kritzel, x: i32, y: i32) -> (i32, i32) {
    crate::view::view_to_doc(&pt.view, x, y)
}

fn doc_w(pt: &Kritzel) -> i32 {
    crate::doc::doc_w(&pt.doc)
}

fn doc_h(pt: &Kritzel) -> i32 {
    crate::doc::doc_h(&pt.doc)
}

fn cur_layer(pt: &Kritzel) -> i32 {
    pt.doc.cur_layer
}

fn layer_mut(pt: &mut Kritzel, i: i32) -> &mut Surface {
    crate::doc::doc_layer(&mut pt.doc, i)
}

fn doc_mark(pt: &mut Kritzel, x0: i32, y0: i32, x1: i32, y1: i32) {
    crate::doc::doc_mark(&mut pt.doc, x0, y0, x1, y1);
}

fn hist_begin(pt: &mut Kritzel, layer: i32) -> i32 {
    crate::history::hist_begin(pt, layer)
}

fn hist_end(pt: &mut Kritzel, x0: i32, y0: i32, x1: i32, y1: i32) {
    crate::history::hist_end(pt, x0, y0, x1, y1);
}
