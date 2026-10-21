// ed_core -- the editing core of the editors with a text model of their own: richtext
// (rich_edit.rs) and code_editor (code_edit.rs)
//
//   TextPos { line, byte }               a place in a document of lines (code) or paragraphs
//                                        (rich text): the line and a UTF-8 byte offset in it
//   TextSel { anchor, caret }            a selection (anchor == caret: the caret alone)
//   (crate) EdView                       what the caret moves need of a widget: the lines'
//                                        texts and their visual rows (byte ranges, caret x, y)
//   (crate) ed_key(view, sel, prefx, key, page, tab) -> EdAct   what an editing key does:
//                                        the caret moves (grapheme clusters, words, rows with
//                                        a preferred x, pages, the document; Shift selects),
//                                        the range Backspace / Delete remove (+Ctrl: words), Enter,
//                                        Tab, clipboard and undo chords; None: not an editing key
//   (crate) word_range(text, byte)       the word a double click selects (a run of blanks or of
//                                        punctuation as one)
//   (crate) Ui::ed_key_first / ed_key_take / ed_caret_on   the key queue past the reservation
//                                        of the focused editor, the caret blink
//
// Both widgets take the edit state of the text boxes while they have the focus (edit.rs
// ed_hold / ed_drop): the key filter of app-level key() then hides their editing keys,
// textbox_selection / textbox_select and a screen reader's selection (Act::Select) work on the
// text of their K_TEXTBOX record, the caret reaches magnifiers and the IME through ime_pos.
// Keys follow the text boxes: Left / Right (+Ctrl words, +Shift select), Up / Down at the
// preferred x, PgUp / PgDn by the view height, Home / End on the visual row (Ctrl: the
// document), Backspace / Delete (+Ctrl words), Ctrl+A / C / X / V, Ctrl+Z undo, Ctrl+Y /
// Ctrl+Shift+Z redo, Shift+Delete cut, Ctrl+Insert copy, Shift+Insert paste.  Right-to-left the
// arrows follow the text (rtl_text_key, applied by the caller).

use text::segment;

use crate::consts::*;
use crate::keys::*;
use crate::Ui;

/// a place in a document: the line (code) or paragraph (rich text) and a UTF-8 byte offset
/// into its text
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextPos {
    pub line: usize,
    pub byte: usize,
}

impl TextPos {
    /// the place `byte` of line `line`
    pub const fn new(line: usize, byte: usize) -> TextPos {
        TextPos { line, byte }
    }
}

/// a selection: the anchor stays where it began, the caret moves (equal: no selection)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextSel {
    pub anchor: TextPos,
    pub caret: TextPos,
}

impl TextSel {
    /// the caret alone at p
    pub const fn at(p: TextPos) -> TextSel {
        TextSel { anchor: p, caret: p }
    }
    /// from anchor to caret
    pub const fn new(anchor: TextPos, caret: TextPos) -> TextSel {
        TextSel { anchor, caret }
    }
    /// anchor == caret: no text selected
    pub fn is_empty(&self) -> bool {
        self.anchor == self.caret
    }
    /// the earlier end
    pub fn start(&self) -> TextPos {
        self.anchor.min(self.caret)
    }
    /// the later end
    pub fn end(&self) -> TextPos {
        self.anchor.max(self.caret)
    }
    /// (start, end)
    pub fn range(&self) -> (TextPos, TextPos) {
        (self.start(), self.end())
    }
}

/// what the caret moves need of an editor: its lines and their visual rows.  Coordinates are
/// content pixels (x from the text's left edge, y from the document's top).
pub(crate) trait EdView {
    fn lines(&self) -> usize;
    fn text(&self, line: usize) -> &str;
    /// the visual rows of a line (at least 1)
    fn rows(&self, line: usize) -> usize;
    /// the byte range of a row; a soft break's index is the next row's start
    fn row_range(&self, line: usize, row: usize) -> (usize, usize);
    /// the caret x of `byte` inside that row
    fn x_at(&self, line: usize, row: usize, byte: usize) -> i32;
    /// the caret stop of that row nearest to x (a grapheme boundary)
    fn hit_x(&self, line: usize, row: usize, x: i32) -> usize;
    /// top and height of a row
    fn row_y(&self, line: usize, row: usize) -> (i32, i32);
    /// the row at y (clamped to the document)
    fn row_at(&self, y: i32) -> (usize, usize);
}

/// the row of a line that shows `byte` (a soft break belongs to the next row)
pub(crate) fn row_of(v: &dyn EdView, p: TextPos) -> usize {
    let n = v.rows(p.line);
    let mut r = n.saturating_sub(1);
    while r > 0 && v.row_range(p.line, r).0 > p.byte {
        r -= 1;
    }
    r
}

/// the caret x of p
pub(crate) fn x_of(v: &dyn EdView, p: TextPos) -> i32 {
    v.x_at(p.line, row_of(v, p), p.byte)
}

/// the end of the document
pub(crate) fn doc_end(v: &dyn EdView) -> TextPos {
    let l = v.lines().saturating_sub(1);
    TextPos::new(l, v.text(l).len())
}

/// p kept inside the document and on a character boundary
pub(crate) fn clamp_pos(v: &dyn EdView, p: TextPos) -> TextPos {
    let n = v.lines();
    if n == 0 {
        return TextPos::default();
    }
    let line = p.line.min(n - 1);
    let t = v.text(line);
    let mut b = p.byte.min(t.len());
    while !t.is_char_boundary(b) {
        b -= 1;
    }
    TextPos::new(line, b)
}

/// the grapheme cluster boundary before p (the end of the previous line at a line start)
pub(crate) fn prev_pos(v: &dyn EdView, p: TextPos) -> TextPos {
    if p.byte == 0 {
        if p.line == 0 {
            return p;
        }
        return TextPos::new(p.line - 1, v.text(p.line - 1).len());
    }
    TextPos::new(p.line, segment::grapheme_prev(v.text(p.line), p.byte))
}

/// the grapheme cluster boundary after p (the start of the next line at a line end)
pub(crate) fn next_pos(v: &dyn EdView, p: TextPos) -> TextPos {
    let t = v.text(p.line);
    if p.byte >= t.len() {
        if p.line + 1 >= v.lines() {
            return p;
        }
        return TextPos::new(p.line + 1, 0);
    }
    TextPos::new(p.line, segment::grapheme_next(t, p.byte))
}

/// Ctrl+Left: the start of the word before p (the end of the previous line at a line start)
pub(crate) fn word_left(v: &dyn EdView, p: TextPos) -> TextPos {
    if p.byte == 0 {
        return prev_pos(v, p);
    }
    TextPos::new(p.line, segment::word_prev(v.text(p.line), p.byte))
}

/// Ctrl+Right: the start of the next word after p (the next line's start at a line end)
pub(crate) fn word_right(v: &dyn EdView, p: TextPos) -> TextPos {
    let t = v.text(p.line);
    if p.byte >= t.len() {
        return next_pos(v, p);
    }
    TextPos::new(p.line, segment::word_next(t, p.byte))
}

// the row after (delta 1) / before (-1) a row, None at the document's ends
fn row_step(v: &dyn EdView, line: usize, row: usize, delta: i32) -> Option<(usize, usize)> {
    if delta > 0 {
        if row + 1 < v.rows(line) {
            Some((line, row + 1))
        } else if line + 1 < v.lines() {
            Some((line + 1, 0))
        } else {
            None
        }
    } else if row > 0 {
        Some((line, row - 1))
    } else if line > 0 {
        Some((line - 1, v.rows(line - 1) - 1))
    } else {
        None
    }
}

/// Up / Down: `delta` rows away at the preferred x (kept in prefx, -1: none yet); past the
/// first / last row: the document's start / end
pub(crate) fn vmove(v: &dyn EdView, p: TextPos, prefx: &mut i32, delta: i32) -> TextPos {
    let x = if *prefx < 0 { x_of(v, p) } else { *prefx };
    *prefx = x.max(0);
    let (mut line, mut row) = (p.line, row_of(v, p));
    for _ in 0..delta.unsigned_abs() {
        match row_step(v, line, row, delta.signum()) {
            Some(r) => (line, row) = r,
            None => return if delta < 0 { TextPos::default() } else { doc_end(v) },
        }
    }
    TextPos::new(line, v.hit_x(line, row, x))
}

/// PgUp / PgDn: the row `dy` pixels above / below the caret's row at the preferred x
pub(crate) fn page_move(v: &dyn EdView, p: TextPos, prefx: &mut i32, dy: i32) -> TextPos {
    let x = if *prefx < 0 { x_of(v, p) } else { *prefx };
    *prefx = x.max(0);
    let row = row_of(v, p);
    let (y, h) = v.row_y(p.line, row);
    let target = y + h / 2 + dy;
    if target < 0 {
        return TextPos::default();
    }
    let (line, r) = v.row_at(target);
    if (line, r) == (p.line, row) {
        // the view holds less than a row: one row at least
        return vmove(v, p, prefx, dy.signum());
    }
    TextPos::new(line, v.hit_x(line, r, x))
}

/// Home: the start of the caret's visual row
pub(crate) fn row_home(v: &dyn EdView, p: TextPos) -> TextPos {
    TextPos::new(p.line, v.row_range(p.line, row_of(v, p)).0)
}

/// End: the end of the caret's visual row (a soft-wrapped row: before its last cluster, the
/// blank the row broke after, so the caret stays on that row)
pub(crate) fn row_end(v: &dyn EdView, p: TextPos) -> TextPos {
    let row = row_of(v, p);
    let (a, b) = v.row_range(p.line, row);
    if row + 1 < v.rows(p.line) && b > a {
        return TextPos::new(p.line, segment::grapheme_prev(v.text(p.line), b).max(a));
    }
    TextPos::new(p.line, b)
}

/// what Backspace removes before p: the word (ctrl), an emoji sequence whole, of another
/// cluster its last code point (an accent can be typed again); at a line start the break
pub(crate) fn back_range(v: &dyn EdView, p: TextPos, ctrl: bool) -> (TextPos, TextPos) {
    if p.byte == 0 {
        return (prev_pos(v, p), p);
    }
    let t = v.text(p.line);
    let a = if ctrl {
        segment::word_prev(t, p.byte)
    } else {
        let g = segment::grapheme_prev(t, p.byte);
        if segment::pictographic_cluster(t, g, p.byte) {
            g
        } else {
            let mut c = p.byte - 1;
            while !t.is_char_boundary(c) {
                c -= 1;
            }
            c.max(g)
        }
    };
    (TextPos::new(p.line, a), p)
}

/// what Delete removes after p: the cluster or the word (ctrl); at a line end the break
pub(crate) fn del_range(v: &dyn EdView, p: TextPos, ctrl: bool) -> (TextPos, TextPos) {
    let t = v.text(p.line);
    if p.byte >= t.len() {
        return (p, next_pos(v, p));
    }
    let b = if ctrl { segment::word_next(t, p.byte) } else { segment::grapheme_next(t, p.byte) };
    (p, TextPos::new(p.line, b))
}

/// the word a double click at `byte` selects: a word, or a run of blanks / of punctuation
pub(crate) fn word_range(t: &str, byte: usize) -> (usize, usize) {
    if t.is_empty() {
        return (0, 0);
    }
    let (mut a, mut b) = segment::word_at(t, byte.min(t.len()));
    let kind = segment::word_kind(t, a, b);
    if kind != segment::WK_WORD {
        while a > 0 {
            let (pa, pb) = segment::word_at(t, a - 1);
            if segment::word_kind(t, pa, pb) != kind {
                break;
            }
            a = pa;
        }
        while b < t.len() {
            let (na, nb) = segment::word_at(t, b);
            if segment::word_kind(t, na, nb) != kind {
                break;
            }
            b = nb;
        }
    }
    (a, b)
}

/// what an editing key does (ed_key)
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EdAct {
    /// the selection moved (navigation, Ctrl+A)
    Sel(TextSel),
    /// remove a..b (Backspace / Delete; with a selection: the selection); word: Ctrl held
    Del(TextPos, TextPos, bool),
    /// Enter (no Ctrl / Alt)
    Enter,
    /// Tab / Shift+Tab (only when the editor takes Tab)
    Tab(bool),
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    /// a key whose text arrives as typed text (letters, digits, AltGr chords): consumed
    Char,
}

/// ed_key(view, sel, prefx, key, page_h, tab) -> what the key (VK | MOD_* << 16, already mapped
/// with rtl_text_key) does to an editor; None: not an editing key (it stays for the app).
/// prefx is the preferred x of Up / Down (-1 after other keys); page_h the view height (px)
pub(crate) fn ed_key(v: &dyn EdView, sel: TextSel, prefx: &mut i32, key: u32, page_h: i32, tab: bool) -> Option<EdAct> {
    let vk = key & 0xFFFF;
    let m = key >> 16;
    let shift = m & MOD_SHIFT != 0;
    let altgr = m & MOD_ALTGR != 0 || m & (MOD_CTRL | MOD_ALT) == MOD_CTRL | MOD_ALT;
    let ctrl = !altgr && m & MOD_CTRL != 0;
    let alt = !altgr && m & MOD_ALT != 0;
    if alt {
        return None;
    }
    let keep = *prefx;
    *prefx = -1;
    let to = |p: TextPos| {
        let p = clamp_pos(v, p);
        Some(EdAct::Sel(if shift { TextSel::new(sel.anchor, p) } else { TextSel::at(p) }))
    };
    let c = sel.caret;
    match vk {
        VK_LEFT => {
            if ctrl {
                to(word_left(v, c))
            } else if !shift && !sel.is_empty() {
                to(sel.start())
            } else {
                to(prev_pos(v, c))
            }
        }
        VK_RIGHT => {
            if ctrl {
                to(word_right(v, c))
            } else if !shift && !sel.is_empty() {
                to(sel.end())
            } else {
                to(next_pos(v, c))
            }
        }
        VK_UP | VK_DOWN if !ctrl => {
            *prefx = keep;
            let p = vmove(v, c, prefx, if vk == VK_UP { -1 } else { 1 });
            let px = *prefx;
            let r = to(p);
            *prefx = px;
            r
        }
        VK_PRIOR | VK_NEXT if !ctrl => {
            *prefx = keep;
            let h = page_h.max(1);
            let p = page_move(v, c, prefx, if vk == VK_PRIOR { -h } else { h });
            let px = *prefx;
            let r = to(p);
            *prefx = px;
            r
        }
        VK_HOME => to(if ctrl { TextPos::default() } else { row_home(v, c) }),
        VK_END => to(if ctrl { doc_end(v) } else { row_end(v, c) }),
        VK_BACK if !shift || !ctrl => {
            if !sel.is_empty() {
                Some(EdAct::Del(sel.start(), sel.end(), false))
            } else {
                let (a, b) = back_range(v, c, ctrl);
                Some(EdAct::Del(a, b, ctrl))
            }
        }
        VK_DELETE => {
            if shift && !ctrl {
                Some(EdAct::Cut)
            } else if !sel.is_empty() {
                Some(EdAct::Del(sel.start(), sel.end(), false))
            } else {
                let (a, b) = del_range(v, c, ctrl);
                Some(EdAct::Del(a, b, ctrl))
            }
        }
        VK_INSERT => {
            if ctrl {
                Some(EdAct::Copy)
            } else if shift {
                Some(EdAct::Paste)
            } else {
                Some(EdAct::Char)
            }
        }
        VK_RETURN if !ctrl => Some(EdAct::Enter),
        VK_TAB if tab && !ctrl => Some(EdAct::Tab(shift)),
        _ if ctrl => match vk {
            0x41 if !shift => Some(EdAct::Sel(TextSel::new(TextPos::default(), doc_end(v)))),
            0x43 if !shift => Some(EdAct::Copy),
            0x58 if !shift => Some(EdAct::Cut),
            0x56 if !shift => Some(EdAct::Paste),
            0x5A => Some(if shift { EdAct::Redo } else { EdAct::Undo }),
            0x59 if !shift => Some(EdAct::Redo),
            _ => None,
        },
        _ => {
            // keys without a text of their own stay for the app
            let own = vk.wrapping_sub(VK_SHIFT) <= VK_CAPITAL - VK_SHIFT
                || vk.wrapping_sub(VK_LWIN) <= VK_APPS - VK_LWIN
                || vk.wrapping_sub(VK_F1) <= 23
                || vk.wrapping_sub(VK_NUMLOCK) <= 1
                || vk.wrapping_sub(VK_LSHIFT) <= VK_RMENU - VK_LSHIFT
                || matches!(vk, VK_ESCAPE | VK_TAB | VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_RETURN);
            if own {
                *prefx = keep;
                None
            } else {
                *prefx = keep;
                Some(EdAct::Char)
            }
        }
    }
}

/// the code points of typed text an editor inserts (control characters dropped; Tab kept when
/// `tab`)
pub(crate) fn typed_text(cps: &[u32], tab: bool) -> alloc::string::String {
    let mut s = alloc::string::String::new();
    for &c in cps {
        if (c < 32 && !(tab && c == 9)) || c == 127 {
            continue;
        }
        if let Some(ch) = char::from_u32(c) {
            s.push(ch);
        }
    }
    s
}

impl Ui {
    /// the first unconsumed key of this frame (VK | MOD_* << 16), past the reservation of the
    /// focused editor; 0: none
    pub(crate) fn ed_key_first(&self) -> u32 {
        self.wp.ctx.keyq.iter().copied().find(|&k| (k as i32) >= 0).unwrap_or(0)
    }

    /// consume the key ed_key_first returned
    pub(crate) fn ed_key_take(&mut self) {
        let q = &mut self.wp.ctx.keyq;
        if let Some(i) = q.iter().position(|&k| (k as i32) >= 0) {
            q[i] |= 0x8000_0000;
        }
    }

    /// consume an unconsumed key with exactly these modifiers (Shift, Ctrl, Alt) anywhere in
    /// the queue
    pub(crate) fn ed_key_chord(&mut self, vk: u32, mods: u32) -> bool {
        let m = MOD_SHIFT | MOD_CTRL | MOD_ALT;
        let q = &mut self.wp.ctx.keyq;
        match q.iter().position(|&k| (k as i32) >= 0 && k & 0xFFFF == vk && (k >> 16) & m == mods && k >> 16 & MOD_ALTGR == 0) {
            Some(i) => {
                q[i] |= 0x8000_0000;
                true
            }
            None => false,
        }
    }

    /// the caret is drawn now (blinks with the system's time from t0; arms the timer;
    /// headless: always)
    pub(crate) fn ed_caret_on(&mut self, t0: u32) -> bool {
        if self.headless || self.blink_ms == 0 {
            return true;
        }
        let blink = self.blink_ms;
        let t = self.now_ms.wrapping_sub(t0);
        self.timer(UIT_CARET, blink - t % blink);
        (t / blink) & 1 == 0
    }

    /// the widget is the window's focused widget and the window has the keyboard
    pub(crate) fn ed_focused(&self, id: u32) -> bool {
        self.wp.ctx.focus == id && self.uiw_cur == self.m.win.active
    }

    /// the IME's composition (preedit: IBus, Wayland text input) of the live window; empty:
    /// none.  The editors draw it at the caret (the text boxes leave it to the IME's window).
    pub(crate) fn ed_preedit(&self) -> plat::Preedit {
        let pw = if self.headless { self.uiw_cur as i32 } else { self.uiw_pw(self.uiw_cur) };
        if pw < 0 {
            return plat::Preedit::default();
        }
        plat::ime_preedit(pw as u8)
    }

    /// draw a composition at x, the baseline y (row top / height for its box) in a font: a box
    /// in the background colour over the text, the text underlined, a caret at its cursor
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ed_preedit_draw(&mut self, pe: &plat::Preedit, x: i32, top: i32, h: i32, base: i32, font: (u32, i32), fg: u32, bg: u32) {
        let prev = self.text.get_font();
        self.text.set_font(font.0, font.1);
        let w = self.text.width(&pe.text).max(1);
        let cx = self.text.caret_x(pe.text.as_bytes(), pe.cursor.min(pe.text.len()));
        self.text.set_font(prev.0, prev.1);
        let hair = self.hair().max(1);
        self.draw_color(bg | 0xFF00_0000);
        self.draw_rect(x, top, w + hair, h);
        self.cmd_text(x, base, pe.text.as_bytes(), font.0, font.1, fg, 0);
        self.draw_color(fg);
        self.draw_rect(x, base + hair.max((font.1 >> 8) / 10), w, hair);
        self.draw_rect(x + cx, top, hair, h);
    }

    /// give the focus to an editor without the focus ring (a click, a tool button)
    pub(crate) fn ed_focus_quiet(&mut self, id: u32) {
        if self.wp.ctx.focus != id {
            self.wp.ctx.focus = id;
            self.wp.ctx.focus_visible = false;
            self.repass();
        }
    }
}
