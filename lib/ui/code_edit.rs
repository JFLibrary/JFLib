// code_edit -- ui.code_editor: a code editor over a CodeDoc (code_doc.rs) with syntax
// highlighting (highlight.rs)
//
//   ui.code_editor(id, &mut doc, &hl, CE_*) -> EV_*   the editor, filling the place it gets
//        (natural 560 x 320 DIP; next_size(-1, -1) fills): line numbers, the monospace font
//        of mono_font_set (else the system's), tabs to tab_width, the current line, the
//        bracket pair at the caret, a find / replace bar, scroll bars.  Only the visible lines
//        are laid out and highlighted (the editor keeps each line's start state and follows
//        the document's change journal), so a file of 100 000 lines scrolls as fast as a
//        short one.  EV_ACT: the text changed; EV_FOCUS while focused; EV_COMMIT: the focus
//        left.  Ids id .. id + 15 (id + 1 / + 2 the scroll bars, + 3 .. 13 the find bar).
//        CE_READONLY: select, copy and find only; CE_NO_GUTTER: no line numbers;
//        CE_NO_FRAME: no box; CE_NO_FIND: no find bar; CE_NO_LINE: no current-line highlight;
//        CE_MARKS: the gutter keeps its column of diagnostic markers while there are none
//   ui.code_find(id, query, replace)   open the find bar (replace: with the replace row) with
//        a query; the editor selects the first match from the caret on
//   ui.code_status(id) -> CodeStatus   line and column of the caret (from 1), the matches
//        and the current one, the lines tokenized in the last frame, the document's
//        diagnostics per severity and the severity of the one at the caret
//   ui.code_diag(id) -> Option<Diag>   the (most severe) diagnostic at the caret, its message
//   ui.code_theme_set(Some(theme))     the colours of every code editor (None: code_theme())
//   ui.code_font_size(pt)              the size of the code editors' font (default 10 pt)
//
// Diagnostics (CodeDoc::set_diagnostics; they follow the edits, code_doc.rs) are wavy
// underlines below their text in the colour of their severity (infos below warnings below
// errors; a point or the end of a line: one cell) and a marker in the gutter's first column on
// the line where one starts (the most severe: a filled circle with a cross, a triangle, an
// "i" -- shapes, not only colours, also in high contrast).  The column is there while the
// document has diagnostics (CE_MARKS: always, so the text does not move).  The pointer over an
// underline or a marker shows the messages as a tooltip ("Fehler: ...", most severe first);
// the ones at the caret are in code_status / code_diag and a K_TEXT record at the caret's line
// for screen readers (after the editor's record: the app's post-modifiers stay the editor's).
// Only the diagnostics of the visible lines are looked at (binary search by position), an
// underline is one command however long it is.
//
// Keys (besides those of ed_core.rs): Tab / Shift+Tab indent / unindent (several lines: the
// block), Enter a line break with the indentation (one level more after an opener), a closer
// typed on a blank line goes one level out, Backspace in leading blanks back to the previous
// tab stop, Home toggles between the first non-blank and column 0, Ctrl+C / Ctrl+X without a
// selection the whole line, Ctrl+/ line comments, Alt+Up / Alt+Down move the lines, Ctrl+Up /
// Ctrl+Down scroll, Ctrl+F find, Ctrl+H replace, F3 / Shift+F3 (Enter / Shift+Enter in the
// find box) the next / previous match, F8 / Shift+F8 the caret to the start of the next /
// previous diagnostic (round the end; without diagnostics F8 stays the window's key for the
// splitters), Esc closes the find bar (else drops the focus),
// Ctrl+Tab (read-only: Tab) moves the focus on.  Mouse: click, drag, Shift+click, double
// click a word, a click or drag in the gutter selects lines, the wheel (Shift: sideways).
// Accessibility: a K_TEXTBOX record with the text (beyond 64 KB: the visible lines) and the
// caret / anchor as byte offsets into it, ime_pos for IME and magnifiers, the edit state of
// the text boxes while focused (ed_hold).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::{CAP_ROUND, JOIN_ROUND};
use text::segment;
use text::FS_SYN_ITALIC;

use crate::cmd::{Draw, DrawRef, Gfx};
use crate::code_doc::*;
use crate::consts::*;
use crate::ed_core::*;
use crate::highlight::*;
use crate::i18n::tr;
use crate::keys::*;
use crate::{Sp, Ui};

/// code_editor flags: select, copy and find only
pub const CE_READONLY: u32 = 1;
/// no line numbers
pub const CE_NO_GUTTER: u32 = 2;
/// no text box frame
pub const CE_NO_FRAME: u32 = 4;
/// no find bar
pub const CE_NO_FIND: u32 = 8;
/// no highlight of the current line
pub const CE_NO_LINE: u32 = 16;
/// the gutter keeps its column of diagnostic markers while the document has none
pub const CE_MARKS: u32 = 32;

const NAT_W: i32 = 560;
const NAT_H: i32 = 320;
const LABEL_MAX: usize = 64 * 1024; // beyond: the record holds the visible lines
const BRACKET_LINES: usize = 3000; // how far bracket matching looks
const UNKNOWN: u32 = u32::MAX;

/// what code_status reports
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CodeStatus {
    /// the caret's line and column (from 1; columns with tabs expanded)
    pub line: usize,
    pub col: usize,
    /// the find bar's matches (0: none or closed) and the current one (from 1; 0: none)
    pub matches: usize,
    pub current: usize,
    /// the lines tokenized for highlighting states in the editor's last frame (the visible
    /// lines are tokenized again to draw them, not counted)
    pub hl_lines: usize,
    /// the number of lines
    pub lines: usize,
    /// the document's diagnostics: errors, warnings, infos (DIAG_* order)
    pub diags: [usize; 3],
    /// the severity (DIAG_*) of the most severe diagnostic at the caret (ui.code_diag: the
    /// diagnostic with its message)
    pub diag: Option<u8>,
}

// the highlighting states: states[i] = the state line i starts in
#[derive(Default)]
struct Hl {
    grammar: usize,
    uid: u32,
    rev: u64,
    states: Vec<u32>,
    // states[..=valid] are right; states up to `known` were right before the last changes
    // and from `dirty` on (past the changed lines) a state that agrees again proves the rest
    valid: usize,
    known: usize,
    dirty: usize,
    // lines tokenized this frame
    count: usize,
}

#[derive(Default)]
struct Find {
    open: bool,
    replace: bool,
    query: String,
    rep: String,
    case: i32,
    word: i32,
    matches: Vec<(TextPos, TextPos)>,
    // the query, options, revision and document the matches belong to
    key: (String, u8, u64, u32),
    current: Option<usize>,
    // the caret when the search began (incremental search starts there)
    from: TextPos,
}

impl Find {
    fn opts(&self) -> u8 {
        (if self.case != 0 { FIND_CASE } else { 0 }) | (if self.word != 0 { FIND_WORD } else { 0 })
    }
}

#[derive(Default)]
struct View {
    seen: u32,
    sx: i32,
    sy: i32,
    t0: u32,
    prefx: i32,
    // 0 none, 1 characters, 2 words, 3 lines (gutter)
    drag: u8,
    word: (TextPos, TextPos),
    held: Option<(usize, usize)>,
    // the record's text: (document, revision, first line, last line, text)
    label: (u32, u64, usize, usize, String),
    last_sel: TextSel,
    last_rev: u64,
    hl: Hl,
    find: Find,
    // the widest line and all lines (bytes) by document and revision
    max_cols: (u32, u64, usize, usize),
    bracket: Option<(u32, u64, TextPos, Option<(TextPos, TextPos)>)>,
    status: CodeStatus,
    // the lines tokenized for states by the last pass that had something to do
    hl_last: usize,
    // the most severe diagnostic at the caret
    caret_diag: Option<Diag>,
}

/// shared state (Ui::m.code)
#[derive(Default)]
pub(crate) struct S {
    views: BTreeMap<u32, View>,
    theme: Option<CodeTheme>,
    size_pt: f32,
    // a find request: id, query, replace row
    find_req: Option<(u32, String, bool)>,
    // the drawings of wavy underlines: colour, size key (the same object every frame)
    waves: Vec<(u32, u32, DrawRef)>,
}

// a wavy underline across its rectangle, drawn at render time (one command however long)
struct Wave {
    color: u32,
    // half a wave and its height (px), the line's width (24.8)
    hp: i32,
    amp: i32,
    width: i32,
}

impl Draw for Wave {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let pad = (self.width >> 9) + 1;
        let (x0, x1, top) = (r[0] + pad, r[0] + r[2] - pad, r[1] + pad);
        // only the waves near the clip (a dirty part), in the phase of the whole line
        let (c0, c1) = (g.dc.clip[0] - g.dc.ox, g.dc.clip[2] - g.dc.ox);
        let k0 = ((c0 - x0) / self.hp - 2).max(0);
        let y = |k: i32| ((top + (k & 1) * self.amp) << 8) + 128;
        let sh = g.shapes();
        sh.path.begin(sh.dc);
        let mut k = k0;
        let mut x = x0 + k * self.hp;
        sh.path.move_fx(x << 8, y(k));
        while x < x1 && x <= c1 + self.hp * 2 {
            x = (x + self.hp).min(x1);
            k += 1;
            sh.path.line_fx(x << 8, y(k));
        }
        sh.dc.set_color(self.color);
        sh.path.stroke(sh.ras, sh.dc, sh.sf, self.width, CAP_ROUND << 4 | JOIN_ROUND);
    }
}

// the font and cells of the editors
#[derive(Clone, Copy)]
struct Metrics {
    font: u32,
    italic: u32,
    size: i32,
    cw: i32,
    lh: i32,
    base: i32,
    per_cell: bool,
}

// the caret moves over lines of cells
struct CView<'a> {
    doc: &'a CodeDoc,
    cw: i32,
    lh: i32,
    tw: usize,
}

impl EdView for CView<'_> {
    fn lines(&self) -> usize {
        self.doc.line_count()
    }
    fn text(&self, line: usize) -> &str {
        self.doc.line(line)
    }
    fn rows(&self, _: usize) -> usize {
        1
    }
    fn row_range(&self, line: usize, _: usize) -> (usize, usize) {
        (0, self.doc.line(line).len())
    }
    fn x_at(&self, line: usize, _: usize, byte: usize) -> i32 {
        col_in(self.doc.line(line), byte, self.tw) as i32 * self.cw
    }
    fn hit_x(&self, line: usize, _: usize, x: i32) -> usize {
        let col = ((x.max(0) + self.cw / 2) / self.cw.max(1)) as usize;
        byte_in(self.doc.line(line), col, self.tw)
    }
    fn row_y(&self, line: usize, _: usize) -> (i32, i32) {
        (line as i32 * self.lh, self.lh)
    }
    fn row_at(&self, y: i32) -> (usize, usize) {
        (((y.max(0) / self.lh.max(1)) as usize).min(self.doc.line_count() - 1), 0)
    }
}

// an identifier or a run of the same other characters around byte (double click)
fn code_word(t: &str, byte: usize) -> (usize, usize) {
    let b = byte.min(t.len());
    let at = t[b..].chars().next().or_else(|| t[..b].chars().next_back());
    match at {
        Some(c) if is_word(c) => {
            let a = t[..b].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(b, |(i, _)| i);
            let e = t[b..].char_indices().find(|(_, c)| !is_word(*c)).map_or(t.len(), |(i, _)| b + i);
            (a, e)
        }
        _ => word_range(t, b),
    }
}

impl Ui {
    /// the colours of every code editor (None: code_theme(), following the UI's theme)
    pub fn code_theme_set(&mut self, theme: Option<CodeTheme>) {
        self.m.code.theme = theme;
        self.repass();
    }

    /// the size of the code editors' font in points (0: 10 pt)
    pub fn code_font_size(&mut self, pt: f32) {
        self.m.code.size_pt = pt;
        self.repass();
    }

    /// open the find bar of code editor id with a query (replace: with the replace row); the
    /// editor selects the first match from its caret on
    pub fn code_find(&mut self, id: u32, query: &str, replace: bool) {
        self.m.code.find_req = Some((id, String::from(query), replace));
        self.repass();
    }

    /// the caret, the find matches, the lines tokenized and the diagnostics in code editor
    /// id's last frame
    pub fn code_status(&self, id: u32) -> CodeStatus {
        self.m.code.views.get(&id).map_or(CodeStatus::default(), |v| v.status)
    }

    /// the most severe diagnostic at the caret of code editor id in its last frame (its
    /// message for a status bar)
    pub fn code_diag(&self, id: u32) -> Option<Diag> {
        self.m.code.views.get(&id).and_then(|v| v.caret_diag.clone())
    }

    // the monospace font: the cell a whole number of pixels wide (as the consoles)
    fn ce_metrics(&mut self) -> Metrics {
        let pt = if self.m.code.size_pt > 0.0 { self.m.code.size_pt } else { 10.0 };
        let (font, _) = self.mono_font(pt);
        let dpi = self.wp.rt.dpi as f32;
        let scale = self.text_scale as f32 / 100.0;
        let mut size = ((pt.clamp(4.0, 96.0) * 4.0 / 3.0 * dpi / 96.0 * scale * 256.0) as i32).max(256);
        let prev = self.text.get_font();
        self.text.set_font(font, size);
        let per_cell = self.text.measure("i") != self.text.measure("M");
        let cw = if per_cell {
            self.text.width("M").max(self.text.width("W")).max(1)
        } else {
            let adv = self.text.measure("0000000000");
            let cw = ((adv + 1280) / 2560).max(1);
            if adv > 0 {
                size = (size as i64 * (cw as i64 * 2560) / adv as i64) as i32;
                self.text.set_font(font, size);
            }
            cw
        };
        let (asc, desc) = (self.text.ascent(), self.text.descent());
        let lh = (self.text.line_h().max(asc + desc) + self.dp(3)).max(1);
        let base = ((lh - asc - desc) >> 1) + asc;
        self.text.set_font(prev.0, prev.1);
        let it = self.text.style(font, FS_SYN_ITALIC);
        Metrics { font, italic: if it >= 0 { it as u32 } else { font }, size, cw, lh, base, per_cell }
    }

    // follow the document's changes in the highlighting states
    fn ce_hl_sync(hl: &mut Hl, doc: &CodeDoc, h: &Highlighter) {
        let g = h.grammar() as *const Grammar as usize;
        let n = doc.line_count();
        let changes = if hl.grammar == g && hl.uid == doc.uid() && !hl.states.is_empty() { doc.changes_since(hl.rev).map(|c| c.to_vec()) } else { None };
        match changes {
            Some(cs) => {
                for c in cs {
                    if hl.dirty <= hl.valid {
                        hl.dirty = 0; // the earlier changes are highlighted again
                    }
                    // the start states of the lines after `first` that were replaced
                    let lo = (c.first + 1).min(hl.states.len());
                    let hi = (c.first + c.old).min(hl.states.len()).max(lo);
                    hl.states.splice(lo..hi, core::iter::repeat_n(UNKNOWN, c.new.saturating_sub(1)));
                    let d = c.new as isize - c.old as isize;
                    let shift = |x: usize| -> usize {
                        if x >= c.first + c.old {
                            (x as isize + d) as usize
                        } else {
                            x.min(c.first)
                        }
                    };
                    hl.known = shift(hl.known);
                    hl.dirty = if hl.dirty > c.first { shift(hl.dirty).max(c.first + c.new) } else { c.first + c.new };
                    hl.valid = hl.valid.min(c.first);
                }
            }
            None => {
                hl.states.clear();
                hl.states.push(0);
                hl.valid = 0;
                hl.known = 0;
                hl.dirty = 0;
                hl.grammar = g;
                hl.uid = doc.uid();
            }
        }
        hl.states.resize(n + 1, UNKNOWN);
        if hl.states[0] == UNKNOWN {
            hl.states[0] = 0;
        }
        hl.known = hl.known.min(n);
        hl.valid = hl.valid.min(n);
        hl.rev = doc.revision();
    }

    // the start states right up to line `upto`
    fn ce_hl_upto(hl: &mut Hl, doc: &CodeDoc, h: &Highlighter, upto: usize, toks: &mut Vec<Token>) {
        let upto = upto.min(doc.line_count());
        while hl.valid < upto {
            let k = hl.valid;
            let st = h.line(doc.line(k), hl.states[k], toks);
            hl.count += 1;
            let next = k + 1;
            if next >= hl.dirty && next <= hl.known && hl.states[next] == st {
                // the old states agree again: the rest up to `known` holds
                hl.valid = hl.known;
                hl.dirty = 0;
                continue;
            }
            hl.states[next] = st;
            hl.valid = next;
            if hl.valid > hl.known {
                hl.known = hl.valid;
            }
        }
    }

    // the bracket at the caret (before or after it) and its partner, outside strings and
    // comments
    fn ce_bracket(v: &mut View, doc: &CodeDoc, h: &Highlighter) -> Option<(TextPos, TextPos)> {
        let caret = doc.selection().caret;
        if let Some((uid, rev, at, r)) = v.bracket {
            if uid == doc.uid() && rev == doc.revision() && at == caret {
                return r;
            }
        }
        let pairs: Vec<(u8, u8)> = h.grammar().pairs.as_bytes().chunks(2).filter(|c| c.len() == 2).map(|c| (c[0], c[1])).collect();
        let mut toks = Vec::new();
        let code_at = |hl: &mut Hl, toks: &mut Vec<Token>, l: usize, b: usize| -> bool {
            Ui::ce_hl_upto(hl, doc, h, l, toks);
            h.line(doc.line(l), hl.states[l], toks);
            !toks.iter().any(|t| t.start <= b && b < t.end && matches!(t.kind, SY_STRING | SY_COMMENT | SY_CODE | SY_ESCAPE))
        };
        let line = doc.line(caret.line).as_bytes();
        let mut found = None;
        for at in [caret.byte, caret.byte.wrapping_sub(1)] {
            let Some(&c) = line.get(at) else { continue };
            let Some(&(o, cl)) = pairs.iter().find(|p| p.0 == c || p.1 == c) else { continue };
            if !code_at(&mut v.hl, &mut toks, caret.line, at) {
                continue;
            }
            let fwd = c == o;
            let (want, other) = if fwd { (cl, o) } else { (o, cl) };
            let mut depth = 0;
            let mut l = caret.line;
            let mut b = at as isize;
            let mut steps = 0;
            'scan: loop {
                let t = doc.line(l).as_bytes();
                loop {
                    b += if fwd { 1 } else { -1 };
                    if b < 0 || b as usize >= t.len() {
                        break;
                    }
                    let x = t[b as usize];
                    if x != want && x != other {
                        continue;
                    }
                    if !code_at(&mut v.hl, &mut toks, l, b as usize) {
                        continue;
                    }
                    if x == other {
                        depth += 1;
                    } else if depth == 0 {
                        found = Some((TextPos::new(caret.line, at), TextPos::new(l, b as usize)));
                        break 'scan;
                    } else {
                        depth -= 1;
                    }
                }
                steps += 1;
                if steps > BRACKET_LINES {
                    break;
                }
                if fwd {
                    l += 1;
                    if l >= doc.line_count() {
                        break;
                    }
                    b = -1;
                } else {
                    if l == 0 {
                        break;
                    }
                    l -= 1;
                    b = doc.line(l).len() as isize;
                }
            }
            if found.is_some() {
                break;
            }
        }
        v.bracket = Some((doc.uid(), doc.revision(), caret, found));
        found
    }

    // the find matches for the current query (kept up with the change journal)
    fn ce_find_update(f: &mut Find, doc: &CodeDoc) {
        let key = (f.query.clone(), f.opts(), doc.revision(), doc.uid());
        if f.key == key {
            return;
        }
        let same_query = f.key.0 == key.0 && f.key.1 == key.1 && f.key.3 == key.3;
        let changes = if same_query { doc.changes_since(f.key.2).map(|c| c.to_vec()) } else { None };
        match changes {
            Some(cs) => {
                for c in cs {
                    let lo = f.matches.partition_point(|m| m.0.line < c.first);
                    let hi = f.matches.partition_point(|m| m.0.line < c.first + c.old);
                    let d = c.new as isize - c.old as isize;
                    for m in f.matches[hi..].iter_mut() {
                        m.0.line = (m.0.line as isize + d) as usize;
                        m.1.line = (m.1.line as isize + d) as usize;
                    }
                    let fresh = doc_find_range(doc, &f.query, f.opts(), c.first, c.new);
                    f.matches.splice(lo..hi, fresh);
                }
                f.matches.truncate(FIND_MAX);
            }
            None => f.matches = doc.find_all(&f.query, f.opts()),
        }
        f.key = key;
    }

    /// ui_code_editor(id, doc, hl, CE_*) -> EV_* (see the module header)
    pub fn code_editor(&mut self, id: u32, doc: &mut CodeDoc, h: &Highlighter, flags: u32) -> u32 {
        let ro = flags & CE_READONLY != 0;
        let stflags = self.take_flags();
        let r = self.place(self.dp(NAT_W), self.dp(NAT_H));
        let hair = self.hair();
        let st = self.style;
        let frame = flags & CE_NO_FRAME == 0;
        let m = self.ce_metrics();
        let theme = self.m.code.theme.clone().unwrap_or_else(|| self.code_theme());
        let mut v = self.m.code.views.remove(&id).unwrap_or_default();
        v.seen = self.frame;
        v.hl.count = 0;
        let view0 = (v.sx, v.sy);
        let rev0 = doc.revision();
        let sel0 = doc.selection();
        let tw = doc.tab_width.clamp(1, 16) as usize;
        // a find request
        if let Some((rid, q, rep)) = self.m.code.find_req.take() {
            if rid == id && flags & CE_NO_FIND == 0 {
                v.find.open = true;
                v.find.replace = rep;
                v.find.query = q;
                v.find.from = doc.selection().start();
                v.find.key.2 = u64::MAX;
                self.focus(id.wrapping_add(3));
            } else {
                self.m.code.find_req = Some((rid, q, rep));
            }
        }
        // ---- geometry
        let sbw = st.scrollbar_thickness(self);
        let n = doc.line_count();
        let digits = format!("{}", n).len().max(2) as i32;
        // the gutter: the markers' column (while there are diagnostics), the line numbers
        let mk = if flags & CE_MARKS != 0 || doc.diagnostic_count() > 0 { m.lh } else { 0 };
        let nums = flags & CE_NO_GUTTER == 0;
        let gw = mk + if nums { digits * m.cw + self.dp(18) } else { 0 };
        let bar_h = if v.find.open { self.dp(if v.find.replace { 76 } else { 40 }) } else { 0 };
        let inner = [r[0] + hair, r[1] + hair, (r[2] - hair * 2).max(1), (r[3] - hair * 2).max(1)];
        let ty = inner[1] + bar_h;
        let tx = inner[0] + gw + self.dp(6);
        if v.max_cols.0 != doc.uid() || v.max_cols.1 != doc.revision() || v.max_cols.2 == 0 {
            v.max_cols = line_sizes(doc);
        }
        let content_w = (v.max_cols.2 as i32 + 2).saturating_mul(m.cw);
        let tw_px0 = (inner[0] + inner[2] - sbw - tx).max(m.cw);
        let hbar = content_w > tw_px0;
        let th = (inner[1] + inner[3] - ty - if hbar { sbw } else { 0 }).max(m.lh);
        let tw_px = tw_px0;
        let total_h = n as i32 * m.lh;
        let page = th;
        // ---- behave
        let mut hb = HB_FOCUS | HB_TEXT | CUR_IBEAM << HB_CURSOR_SHIFT;
        if stflags & ST_DISABLED != 0 {
            hb |= HB_DISABLED;
        }
        if doc.diagnostic_count() > 0 {
            hb |= HB_MOVES; // (the tooltip follows the pointer from one diagnostic to the next)
        }
        self.wp.ctx.last_rect = [inner[0], ty, inner[2], inner[1] + inner[3] - ty];
        let ev = self.behave(id, hb);
        let disabled = self.wp.ctx.last_state == SS_DISABLED as u32;
        let focused = !disabled && self.ed_focused(id);
        let mut out = ev & (EV_HOVER | EV_HELD | EV_DOUBLE | EV_RCLICK | EV_DRAG);
        if focused {
            if let (Some(sel), Some(held)) = (self.textbox_selection(id), v.held) {
                if sel != held {
                    // (a screen reader's selection, in the record's text)
                    let base = doc.flat(TextPos::new(v.label.2, 0));
                    let (a, c) = (doc.unflat(sel.0 + base), doc.unflat(sel.1 + base));
                    doc.set_selection(TextSel::new(a, c));
                }
            }
            if let Some(t) = self.a11y_text(id) {
                if !ro {
                    let e = doc.end();
                    doc.replace(TextPos::default(), e, &t);
                }
            }
        } else if v.held.is_some() {
            v.held = None;
            v.drag = 0;
            self.ed_drop(id);
            out |= EV_COMMIT;
        }
        // ---- pointer
        let (mx, my) = (self.rtl_map(self.wp.ctx.mx, inner[0], inner[2]), self.wp.ctx.my);
        let (cx, cy) = (mx - tx + v.sx, my - ty + v.sy);
        let in_gutter = mx < tx && gw > 0;
        let c = &self.wp.ctx;
        let press = c.pressed & (1 << BTN_LEFT) != 0 && ev & EV_HOVER != 0 && c.active == id;
        if press && !disabled {
            let view = CView { doc: &*doc, cw: m.cw, lh: m.lh, tw };
            let p = clamp_pos(&view, TextPos::new(view.row_at(cy).0, view.hit_x(view.row_at(cy).0, 0, cx)));
            v.prefx = -1;
            v.t0 = self.now_ms;
            if in_gutter {
                let l = view.row_at(cy).0;
                let e = if l + 1 < n { TextPos::new(l + 1, 0) } else { TextPos::new(l, doc.line(l).len()) };
                v.word = (TextPos::new(l, 0), e);
                if self.mods & MOD_SHIFT != 0 {
                    let a = doc.selection().anchor;
                    doc.set_selection(TextSel::new(a, e));
                } else {
                    doc.set_selection(TextSel::new(v.word.0, e));
                }
                v.drag = 3;
            } else if ev & EV_DOUBLE != 0 {
                let (a, b) = code_word(doc.line(p.line), p.byte);
                v.word = (TextPos::new(p.line, a), TextPos::new(p.line, b));
                doc.set_selection(TextSel::new(v.word.0, v.word.1));
                v.drag = 2;
            } else if self.mods & MOD_SHIFT != 0 {
                let a = doc.selection().anchor;
                doc.set_selection(TextSel::new(a, p));
                v.drag = 1;
            } else {
                doc.set_selection(TextSel::at(p));
                v.drag = 1;
            }
        } else if ev & EV_HELD != 0 && v.drag != 0 {
            if my < ty {
                v.sy -= m.lh;
            } else if my >= ty + th {
                v.sy += m.lh;
            }
            if mx >= tx + tw_px {
                v.sx += m.cw * 2;
            } else if mx < tx && v.drag != 3 {
                v.sx -= m.cw * 2;
            }
            v.sy = v.sy.clamp(0, (total_h - th).max(0));
            v.sx = v.sx.clamp(0, (content_w - tw_px).max(0));
            let (cx, cy) = (mx - tx + v.sx, my - ty + v.sy);
            let view = CView { doc: &*doc, cw: m.cw, lh: m.lh, tw };
            let l = view.row_at(cy).0;
            let p = TextPos::new(l, view.hit_x(l, 0, cx));
            match v.drag {
                3 => {
                    let e = if l + 1 < n { TextPos::new(l + 1, 0) } else { TextPos::new(l, doc.line(l).len()) };
                    if TextPos::new(l, 0) < v.word.0 {
                        doc.set_selection(TextSel::new(v.word.1, TextPos::new(l, 0)));
                    } else {
                        doc.set_selection(TextSel::new(v.word.0, e));
                    }
                }
                2 => {
                    let (a, b) = code_word(doc.line(l), p.byte);
                    let (wa, wb) = (TextPos::new(l, a), TextPos::new(l, b));
                    if wa < v.word.0 {
                        doc.set_selection(TextSel::new(v.word.1, wa));
                    } else {
                        doc.set_selection(TextSel::new(v.word.0, wb.max(v.word.1)));
                    }
                }
                _ => {
                    let a = doc.selection().anchor;
                    doc.set_selection(TextSel::new(a, p));
                }
            }
        } else if ev & EV_HELD == 0 {
            v.drag = 0;
        }
        // ---- keys and typed text
        if focused {
            loop {
                let k0 = self.ed_key_first();
                if k0 == 0 {
                    break;
                }
                let k = self.rtl_text_key(k0);
                if self.ce_key(id, doc, &mut v, h, k, flags, &m) {
                    self.ed_key_take();
                    v.t0 = self.now_ms;
                    continue;
                }
                let act = {
                    let view = CView { doc: &*doc, cw: m.cw, lh: m.lh, tw };
                    ed_key(&view, doc.selection(), &mut v.prefx, k, page, false)
                };
                let Some(act) = act else { break };
                self.ed_key_take();
                v.t0 = self.now_ms;
                match act {
                    EdAct::Sel(s) => doc.set_selection(s),
                    EdAct::Del(a, b, _) if !ro => {
                        let back = k & 0xFFFF == VK_BACK;
                        let empty = doc.selection().is_empty();
                        let (a, b) = if back && empty && !doc.use_tabs { soft_back(doc, a, b) } else { (a, b) };
                        doc.delete_kind(a, b, if !empty { CK_NONE } else if back { CK_BACK } else { CK_DEL });
                    }
                    EdAct::Enter if !ro => {
                        doc.newline(h.grammar().openers);
                    }
                    EdAct::Copy => self.ce_copy(doc, false, ro),
                    EdAct::Cut => self.ce_copy(doc, !ro, ro),
                    EdAct::Paste if !ro => {
                        if let Some(t) = self.clip_get(CLIP_TEXT) {
                            let s = String::from_utf8_lossy(&t).replace('\0', "");
                            if !s.is_empty() {
                                doc.insert(&s);
                            }
                        }
                    }
                    EdAct::Undo if !ro => {
                        doc.undo();
                    }
                    EdAct::Redo if !ro => {
                        doc.redo();
                    }
                    _ => {}
                }
            }
            let cps = self.text_take(super::ctx::TEXTQ_MAX);
            if !ro {
                let s = typed_text(&cps, false);
                if !s.is_empty() {
                    doc.type_text(&s, h.grammar().closers);
                    v.t0 = self.now_ms;
                    v.prefx = -1;
                }
            }
        }
        let changed = doc.revision() != rev0;
        if changed {
            out |= EV_ACT;
            v.max_cols = line_sizes(doc);
        }
        if changed || doc.selection() != sel0 {
            self.repass();
        }
        let n = doc.line_count();
        let total_h = n as i32 * m.lh;
        // ---- the find bar's matches
        if v.find.open {
            Ui::ce_find_update(&mut v.find, doc);
        }
        // ---- wheel, the caret into view
        let wy = self.wp.ctx.wheel_y;
        let wx = self.wp.ctx.wheel_x;
        let over = ev & EV_HOVER != 0 || self.wp.ctx.hot == id.wrapping_add(1) || self.wp.ctx.hot == id.wrapping_add(2);
        if over && (wy != 0 || wx != 0) {
            let (dx, dy) = if self.mods & MOD_SHIFT != 0 { (wy, 0) } else { (wx, wy) };
            let sy = (v.sy - dy * m.lh * 3 / 120).clamp(0, (total_h - th).max(0));
            let sx = (v.sx - dx * m.cw * 6 / 120).clamp(0, (content_w - tw_px).max(0));
            if sy != v.sy || sx != v.sx {
                v.sy = sy;
                v.sx = sx;
                self.wp.ctx.wheel_y = 0;
                self.wp.ctx.wheel_x = 0;
            }
        }
        let caret = doc.clamp(doc.selection().caret);
        let ccol = doc.col_of(caret.line, caret.byte) as i32;
        if (focused || v.find.open) && (doc.selection() != v.last_sel || doc.revision() != v.last_rev) {
            let cy = caret.line as i32 * m.lh;
            if cy < v.sy {
                v.sy = cy;
            }
            if cy + m.lh > v.sy + th {
                v.sy = cy + m.lh - th;
            }
            let cxp = ccol * m.cw;
            if cxp < v.sx + m.cw * 2 {
                v.sx = (cxp - m.cw * 4).max(0);
            }
            if cxp > v.sx + tw_px - m.cw * 2 {
                v.sx = cxp - tw_px + m.cw * 4;
            }
        }
        v.last_sel = doc.selection();
        v.last_rev = doc.revision();
        v.sy = v.sy.clamp(0, (total_h - th).max(0));
        v.sx = v.sx.clamp(0, (content_w - tw_px).max(0));
        // ---- highlighting states up to the last visible line
        let first = (v.sy / m.lh) as usize;
        let last = (((v.sy + th) / m.lh) as usize + 1).min(n);
        let mut toks = Vec::new();
        Ui::ce_hl_sync(&mut v.hl, doc, h);
        Ui::ce_hl_upto(&mut v.hl, doc, h, last, &mut toks);
        let bracket = if focused { Ui::ce_bracket(&mut v, doc, h) } else { None };

        // ---- frame and gutter
        if frame {
            let mut sp = Sp { value: NAT_W, value2: m.lh, flags: 1, ..Sp::default() };
            let _ = st.textbox_metrics(self, &mut sp);
            sp.place(r);
            sp.ev = ev;
            sp.value = focused as i32;
            sp.value2 = disabled as i32;
            st.textbox_frame(self, &sp);
        }
        self.clip_push(inner[0], inner[1], inner[2], inner[3]);
        self.draw_color(theme.bg);
        self.draw_rect(inner[0], ty, inner[2], inner[1] + inner[3] - ty);
        if gw > 0 {
            self.draw_color(theme.gutter_bg);
            self.draw_rect(inner[0], ty, gw, inner[1] + inner[3] - ty);
        }
        self.ltr_begin(inner[0], inner[2]);
        let (sa, sb) = doc.selection().range();
        let sel_col = if focused { theme.sel } else { theme.sel_inactive };
        let ital = |k: u8| theme.italic >> k & 1 != 0;
        let bold = |k: u8| theme.bold >> k & 1 != 0;
        let vis_matches: Vec<(TextPos, TextPos, bool)> = if v.find.open && !v.find.matches.is_empty() {
            let lo = v.find.matches.partition_point(|x| x.0.line < first);
            let hi = v.find.matches.partition_point(|x| x.0.line < last);
            let cur = v.find.current;
            (lo..hi).map(|i| (v.find.matches[i].0, v.find.matches[i].1, Some(i) == cur)).collect()
        } else {
            Vec::new()
        };
        let col0 = (v.sx / m.cw.max(1)) as usize;
        let cols = (tw_px / m.cw.max(1)) as usize + 2;
        let tx0 = tx - v.sx;
        // the line numbers
        if nums {
            for l in first..last {
                let y = ty + l as i32 * m.lh - v.sy;
                let num = format!("{}", l + 1);
                let nw = num.len() as i32 * m.cw;
                let c = if l == caret.line { theme.gutter_cur } else { theme.gutter_fg };
                self.ce_cells(&m, inner[0] + gw - self.dp(10) - nw, y + m.base, num.as_bytes(), c, false, false);
            }
        }
        // the diagnostics of the visible lines: per line the most severe one starting there
        // marks the gutter; underlines (severity, x0, x1, line top) are drawn after the text
        let dvis: Vec<usize> = doc.mark_range(first, last).filter(|&i| last_line(&doc.marks()[i]) >= first).collect();
        if mk > 0 {
            let mut best: Vec<(usize, u8)> = Vec::new();
            for &i in &dvis {
                let d = &doc.marks()[i];
                match best.last_mut() {
                    Some(b) if b.0 == d.a.line => b.1 = b.1.min(d.sev),
                    _ if d.a.line >= first => best.push((d.a.line, d.sev)),
                    _ => {}
                }
            }
            for (l, sev) in best {
                let y = ty + l as i32 * m.lh - v.sy;
                self.ce_marker(sev, inner[0], y, mk, m.lh, theme.diag[sev as usize]);
            }
        }
        let mut waves: Vec<(u8, i32, i32, i32)> = Vec::new();
        for &i in &dvis {
            let d = &doc.marks()[i];
            for l in d.a.line.max(first)..=d.b.line.min(last - 1) {
                if !touches(d, l) {
                    continue;
                }
                let (c0, c1) = mark_cols(d, l, doc.line(l), tw);
                waves.push((d.sev, tx0 + c0 as i32 * m.cw, tx0 + c1 as i32 * m.cw, ty + l as i32 * m.lh - v.sy));
            }
        }
        // the diagnostics under the pointer (an underline, a marker)
        let mut hov: Vec<usize> = Vec::new();
        if ev & EV_HOVER != 0 && v.drag == 0 && !dvis.is_empty() && my >= ty && my < ty + th {
            let l = ((my - ty + v.sy) / m.lh.max(1)) as usize;
            let col = ((mx - tx0).max(0) / m.cw.max(1)) as usize;
            let on_mark = mk > 0 && mx >= inner[0] && mx < inner[0] + mk;
            let on_text = mx >= tx && mx < tx + tw_px;
            hov = dvis
                .iter()
                .copied()
                .filter(|&i| {
                    let d = &doc.marks()[i];
                    if on_mark {
                        d.a.line == l
                    } else {
                        on_text && touches(d, l) && {
                            let (c0, c1) = mark_cols(d, l, doc.line(l), tw);
                            col >= c0 && col < c1
                        }
                    }
                })
                .collect();
            hov.sort_by_key(|&i| doc.marks()[i].sev);
        }
        let area = [tx - self.dp(6), ty, inner[0] + inner[2] - tx + self.dp(6), th];
        self.clip_push(area[0], area[1], area[2], area[3]);
        for l in first..last {
            let y = ty + l as i32 * m.lh - v.sy;
            let t = doc.line(l);
            // the current line, the selection, the find matches, the bracket pair
            if l == caret.line && flags & CE_NO_LINE == 0 && (focused || v.find.open) && sa == sb {
                self.draw_color(theme.line_hl);
                self.draw_rect(area[0], y, area[2], m.lh);
            }
            let (ls, le) = (TextPos::new(l, 0), TextPos::new(l, t.len()));
            if sa < sb && sa <= le && sb >= ls {
                let (lo, hi) = (ls.max(sa), le.min(sb));
                let x0 = col_in(t, lo.byte, tw) as i32 * m.cw;
                let x1 = col_in(t, hi.byte, tw) as i32 * m.cw + if sb > le { m.cw / 2 } else { 0 };
                if x1 > x0 {
                    self.draw_color(sel_col);
                    self.draw_rect(tx0 + x0, y, x1 - x0, m.lh);
                }
            }
            for (a, b, cur) in vis_matches.iter().filter(|x| x.0.line == l) {
                let x0 = col_in(t, a.byte, tw) as i32 * m.cw;
                let x1 = col_in(t, b.byte, tw) as i32 * m.cw;
                self.draw_color(if *cur { theme.find_cur } else { theme.find });
                if theme.find_frame {
                    self.draw_frame(tx0 + x0, y, (x1 - x0).max(m.cw / 2), m.lh, 0, if *cur { hair * 2 } else { hair });
                } else {
                    self.draw_rect(tx0 + x0, y, (x1 - x0).max(m.cw / 2), m.lh);
                }
            }
            if let Some((p, q)) = bracket {
                for at in [p, q] {
                    if at.line == l {
                        let x0 = col_in(t, at.byte, tw) as i32 * m.cw;
                        self.draw_color(theme.bracket);
                        self.draw_frame(tx0 + x0, y, m.cw, m.lh, 0, hair);
                    }
                }
            }
            // the text by tokens (the plain text between them: SY_TEXT), visible cells only
            h.line(t, v.hl.states[l], &mut toks);
            let mut runs: Vec<(usize, usize, u8)> = Vec::with_capacity(toks.len() * 2 + 1);
            let mut at = 0;
            for tk in &toks {
                if tk.start > at {
                    runs.push((at, tk.start, SY_TEXT));
                }
                runs.push((tk.start, tk.end, tk.kind));
                at = tk.end;
            }
            if at < t.len() {
                runs.push((at, t.len(), SY_TEXT));
            }
            let mut col = 0usize;
            let mut i = 0;
            let tb = t.as_bytes();
            for (a, b, kind) in runs {
                if col > col0 + cols {
                    break;
                }
                let inside = sa < sb && TextPos::new(l, a) >= sa && TextPos::new(l, b) <= sb;
                let color = if theme.sel_text != 0 && inside { theme.sel_text } else { theme.tokens[kind as usize % SY_COUNT] };
                // cells of a .. b: ASCII runs at once, other clusters one by one
                i = i.max(a);
                while i < b {
                    if tb[i] == b'\t' {
                        col += tw - col % tw;
                        i += 1;
                        continue;
                    }
                    if tb[i] < 0x80 && !m.per_cell {
                        let mut j = i;
                        while j < b && tb[j] < 0x80 && tb[j] != b'\t' {
                            j += 1;
                        }
                        // only the visible part
                        let s0 = i + col0.saturating_sub(col).min(j - i);
                        let c0 = col + (s0 - i);
                        let s1 = (s0 + (col0 + cols).saturating_sub(c0)).min(j);
                        if s1 > s0 && tb[s0..s1].iter().any(|&c| c != b' ') {
                            self.ce_cells(&m, tx0 + c0 as i32 * m.cw, y + m.base, &tb[s0..s1], color, bold(kind), ital(kind));
                        }
                        col += j - i;
                        i = j;
                        continue;
                    }
                    let e = segment::grapheme_next(t, i).min(b.max(i + 1));
                    let ch = t[i..].chars().next().unwrap_or(' ');
                    let w = char_cols(ch);
                    if col + w > col0 && col < col0 + cols && ch != ' ' {
                        self.ce_cells(&m, tx0 + col as i32 * m.cw, y + m.base, &tb[i..e], color, bold(kind), ital(kind));
                    }
                    col += w;
                    i = e;
                }
            }
        }
        // the underlines: infos first, errors on top; only their visible part (whole waves)
        waves.sort_by_key(|w| core::cmp::Reverse(w.0));
        let (lim0, lim1) = (tx - m.cw * 2, tx + tw_px + m.cw * 2);
        let period = self.dp(2).max(2) * 2;
        for (sev, x0, x1, y) in waves {
            let x0 = if x0 < lim0 { x0 + (lim0 - x0) / period * period } else { x0 };
            let x1 = x1.min(lim1);
            if x1 > x0 {
                self.ce_wave(x0, x1, y, &m, theme.diag[sev as usize]);
            }
        }
        self.clip_pop();
        // the caret
        let cx_px = tx0 + ccol * m.cw;
        let cy_px = ty + caret.line as i32 * m.lh - v.sy;
        if focused {
            let pe = self.ed_preedit();
            if !pe.text.is_empty() {
                self.ed_preedit_draw(&pe, cx_px, cy_px, m.lh, cy_px + m.base, (m.font, m.size), theme.fg, theme.bg);
            } else if doc.selection().is_empty() && self.ed_caret_on(v.t0) && cy_px + m.lh > ty && cy_px < ty + th && cx_px >= tx - 1 {
                self.draw_color(theme.caret);
                self.draw_rect(cx_px, cy_px, self.dp(2).max(1), m.lh);
            }
            self.ime_pos(cx_px, cy_px, m.lh);
        }
        self.ltr_end();
        self.clip_pop();
        // ---- scroll bars
        if total_h > th {
            let mut pos = v.sy;
            self.next_rect_px(inner[0] + inner[2] - sbw, ty, sbw, th);
            if self.scrollbar(id.wrapping_add(1), &mut pos, total_h, th, true) & EV_ACT != 0 {
                v.sy = pos.clamp(0, (total_h - th).max(0));
            }
        }
        if hbar {
            let mut pos = v.sx;
            self.next_rect_px(tx, ty + th, tw_px, sbw);
            if self.scrollbar(id.wrapping_add(2), &mut pos, content_w, tw_px, false) & EV_ACT != 0 {
                v.sx = pos.clamp(0, (content_w - tw_px).max(0));
            }
        }
        // ---- the find bar
        if v.find.open {
            self.ce_find_bar(id, doc, &mut v, [inner[0], inner[1], inner[2], bar_h], ro);
        }
        // ---- the record: the text (or the visible lines), caret and anchor as offsets
        let big = v.max_cols.3 > LABEL_MAX;
        let (l0, l1) = if big { (first, last) } else { (0, n) };
        if v.label.0 != doc.uid() || v.label.1 != doc.revision() || v.label.2 != l0 || v.label.3 != l1 {
            let mut s = String::new();
            for (k, l) in doc.lines()[l0..l1].iter().enumerate() {
                if k > 0 {
                    s.push('\n');
                }
                if s.len() + l.len() > LABEL_MAX {
                    break;
                }
                s.push_str(l);
            }
            v.label = (doc.uid(), doc.revision(), l0, l1, s);
        }
        let rel = |p: TextPos| -> usize {
            if p.line < l0 || p.line >= l1 {
                0
            } else {
                doc.lines()[l0..p.line].iter().map(|l| l.len() + 1).sum::<usize>() + p.byte
            }
        };
        let (fa, fc) = if big { (rel(doc.selection().anchor), rel(doc.selection().caret)) } else { (doc.flat(doc.selection().anchor), doc.flat(doc.selection().caret)) };
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = if focused { SS_FOCUSED as u32 } else { 0 } | if disabled { SS_DISABLED as u32 } else { 0 };
        let label = core::mem::take(&mut v.label.4);
        {
            let kind = (TB_MULTI | if ro { TB_READONLY } else { 0 }) >> 24;
            let s = self.sem(K_TEXTBOX, id, Some(&label), if focused { fc as i32 } else { 0 });
            s.max = if focused { fa as i32 } else { 0 };
            s.min = kind as i32;
        }
        v.label.4 = label;
        // the messages under the pointer: the tooltip of the record above (part: the first
        // diagnostic, in the gutter or the text; only while the pointer is here, so the part
        // tooltips of other widgets keep theirs)
        if ev & EV_HOVER != 0 && !dvis.is_empty() {
            if hov.is_empty() {
                self.tooltip_part("", 0);
            } else {
                let part = 1 + hov[0] as u64 + if mx < tx { 1 << 40 } else { 0 };
                self.tooltip_part(&diag_text(doc, &hov), part);
            }
        }
        // the diagnostics at the caret: the status, a record at the caret's line (after the
        // editor's own, so an app's tooltip or context menu stays the editor's)
        let at = doc.marks_at(caret);
        v.caret_diag = at.first().map(|&i| doc.diag_of(&doc.marks()[i]));
        if !at.is_empty() {
            let state = self.wp.ctx.last_state;
            let cy = (ty + caret.line as i32 * m.lh - v.sy).clamp(ty, (ty + th - m.lh).max(ty));
            self.wp.ctx.last_rect = [inner[0], cy, inner[2], m.lh];
            self.wp.ctx.last_state = 0;
            self.sem(K_TEXT, 0, Some(&diag_text(doc, &at)), doc.marks()[at[0]].sev as i32);
            self.wp.ctx.last_rect = r;
            self.wp.ctx.last_state = state;
        }
        if focused {
            self.ed_hold(id, fa, fc);
            v.held = Some((fa, fc));
            out |= EV_FOCUS;
        }
        // (passes that only repeat a frame keep the count of the one that did the work)
        if v.hl.count > 0 || doc.revision() != rev0 || doc.selection() != sel0 || (v.sx, v.sy) != view0 {
            v.hl_last = v.hl.count;
        }
        v.status = CodeStatus {
            line: caret.line + 1,
            col: ccol as usize + 1,
            matches: if v.find.open { v.find.matches.len() } else { 0 },
            current: if v.find.open { v.find.current.map_or(0, |c| c + 1) } else { 0 },
            hl_lines: v.hl_last,
            lines: n,
            diags: doc.diagnostic_counts(),
            diag: at.first().map(|&i| doc.marks()[i].sev),
        };
        self.m.code.views.insert(id, v);
        let now = self.frame;
        if self.m.code.views.len() > 8 {
            self.m.code.views.retain(|_, v| now.wrapping_sub(v.seen) < 600);
        }
        self.wp.ctx.last_rect = r;
        out
    }

    // the cells of s at x (the font's advance is the cell; a proportional fallback cell by cell)
    #[allow(clippy::too_many_arguments)]
    fn ce_cells(&mut self, m: &Metrics, x: i32, y: i32, s: &[u8], c: u32, bold: bool, italic: bool) {
        let font = if italic { m.italic } else { m.font };
        let passes = if bold { 2 } else { 1 };
        for p in 0..passes {
            let dx = p * ((m.size >> 8) / 16 + 1);
            if m.per_cell {
                let mut i = 0;
                let mut k = 0;
                while i < s.len() {
                    let n = match s[i] {
                        0..=0x7F => 1,
                        0x80..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        _ => 4,
                    }
                    .min(s.len() - i);
                    if s[i] != b' ' {
                        self.cmd_text(x + k * m.cw + dx, y, &s[i..i + n], font, m.size, c, 0);
                    }
                    i += n;
                    k += 1;
                }
            } else {
                self.cmd_text(x + dx, y, s, font, m.size, c, 0);
            }
        }
    }

    // a wavy underline from x0 to x1 at the foot of the line whose top is y (one command: a
    // Wave of this colour and size, the same object every frame)
    fn ce_wave(&mut self, x0: i32, x1: i32, y: i32, m: &Metrics, color: u32) {
        let hp = self.dp(2).max(2);
        let amp = self.dp(2).max(2);
        let width = self.dp(1).max(1) << 8;
        let top = (y + m.base + self.dp(1)).min(y + m.lh - amp - 1);
        let key = (hp as u32) << 16 | (amp as u32) << 8 | (width >> 8) as u32;
        let w = &mut self.m.code.waves;
        let d = match w.iter().find(|e| e.0 == color && e.1 == key) {
            Some(e) => e.2.clone(),
            None => {
                if w.len() >= 24 {
                    w.clear();
                }
                let d: DrawRef = Rc::new(Wave { color, hp, amp, width });
                w.push((color, key, d.clone()));
                d
            }
        };
        let pad = (width >> 9) + 1;
        self.cmd_custom(x0 - pad, top - pad, x1 - x0 + pad * 2, amp + pad * 2, Some(d), key as usize, color);
    }

    // the gutter's marker of a diagnostic, centred in the cell (x, y, w: cell, h: lh): a filled
    // circle with a cross (error), a triangle (warning), an "i" (info)
    fn ce_marker(&mut self, sev: u8, x: i32, y: i32, cell: i32, lh: i32, color: u32) {
        let cp = match sev {
            DIAG_ERROR => IC_ERROR_FILL,
            DIAG_WARNING => IC_WARNING,
            _ => IC_INFO,
        };
        let px = (lh * 2 / 3).max(8);
        self.draw_color(color);
        self.draw_icon(cp, x + (cell - px) / 2, y + (lh - px) / 2, px << 8);
    }

    // a key of the editor itself -> consumed
    #[allow(clippy::too_many_arguments)]
    fn ce_key(&mut self, id: u32, doc: &mut CodeDoc, v: &mut View, h: &Highlighter, k: u32, flags: u32, m: &Metrics) -> bool {
        let ro = flags & CE_READONLY != 0;
        let vk = k & 0xFFFF;
        let mods = k >> 16;
        let altgr = mods & MOD_ALTGR != 0 || mods & (MOD_CTRL | MOD_ALT) == MOD_CTRL | MOD_ALT;
        if altgr {
            return false;
        }
        let ctrl = mods & MOD_CTRL != 0;
        let shift = mods & MOD_SHIFT != 0;
        let alt = mods & MOD_ALT != 0;
        let sel = doc.selection();
        if alt {
            return match vk {
                VK_UP | VK_DOWN if !ro && !ctrl && !shift => {
                    doc.move_lines(if vk == VK_UP { -1 } else { 1 });
                    true
                }
                _ => false,
            };
        }
        if ctrl {
            return match (vk, shift) {
                (0x46, false) | (0x48, false) if flags & CE_NO_FIND == 0 => {
                    let q = if sel.is_empty() || sel.start().line != sel.end().line { v.find.query.clone() } else { doc.selected_text() };
                    v.find.open = true;
                    v.find.replace = vk == 0x48 && !ro;
                    v.find.query = q;
                    v.find.from = sel.start();
                    v.find.key.2 = u64::MAX;
                    self.focus(id.wrapping_add(3));
                    true
                }
                (VK_OEM_2, false) | (VK_DIVIDE, false) => {
                    if !ro {
                        doc.toggle_comment(h.grammar().comment);
                    }
                    true
                }
                (VK_UP, false) | (VK_DOWN, false) => {
                    v.sy += if vk == VK_UP { -m.lh } else { m.lh };
                    v.sy = v.sy.max(0);
                    true
                }
                _ => false,
            };
        }
        match vk {
            // (read-only: Tab moves the focus on)
            VK_TAB if !ro => {
                if shift {
                    doc.back_tab();
                } else {
                    doc.tab();
                }
                true
            }
            VK_F3 => {
                self.ce_find_step(doc, v, shift);
                true
            }
            // (without diagnostics F8 is the window's: the splitters)
            VK_F8 if doc.diagnostic_count() > 0 => {
                if let Some(p) = doc.next_diagnostic(sel.caret, shift) {
                    doc.set_selection(TextSel::at(p));
                }
                v.prefx = -1;
                true
            }
            VK_HOME => {
                // the first non-blank, else column 0
                let t = doc.line(sel.caret.line);
                let ind = t.len() - t.trim_start_matches([' ', '\t']).len();
                let to = if sel.caret.byte == ind { 0 } else { ind };
                let p = TextPos::new(sel.caret.line, to);
                doc.set_selection(if shift { TextSel::new(sel.anchor, p) } else { TextSel::at(p) });
                v.prefx = -1;
                true
            }
            VK_ESCAPE => {
                if v.find.open {
                    v.find.open = false;
                } else {
                    self.wp.ctx.focus = 0;
                }
                self.repass();
                true
            }
            _ => false,
        }
    }

    // copy (cut) the selection; without one the caret's whole line
    fn ce_copy(&mut self, doc: &mut CodeDoc, cut: bool, ro: bool) {
        let sel = doc.selection();
        if sel.is_empty() {
            let l = sel.caret.line;
            let mut s = String::from(doc.line(l));
            s.push('\n');
            self.clip_set(CLIP_TEXT, s.as_bytes());
            if cut && !ro {
                let (a, b) = if l + 1 < doc.line_count() {
                    (TextPos::new(l, 0), TextPos::new(l + 1, 0))
                } else if l > 0 {
                    (TextPos::new(l - 1, doc.line(l - 1).len()), TextPos::new(l, doc.line(l).len()))
                } else {
                    (TextPos::new(0, 0), TextPos::new(0, doc.line(0).len()))
                };
                doc.delete(a, b);
            }
            return;
        }
        let s = doc.selected_text();
        self.clip_set(CLIP_TEXT, s.as_bytes());
        if cut {
            doc.delete(sel.start(), sel.end());
        }
    }

    // the next / previous match after the selection; it becomes the selection
    fn ce_find_step(&mut self, doc: &mut CodeDoc, v: &mut View, back: bool) {
        if v.find.query.is_empty() {
            return;
        }
        Ui::ce_find_update(&mut v.find, doc);
        let sel = doc.selection();
        let ms = &v.find.matches;
        if ms.is_empty() {
            v.find.current = None;
            return;
        }
        let i = if back {
            let k = ms.partition_point(|x| x.0 < sel.start());
            if k == 0 { ms.len() - 1 } else { k - 1 }
        } else {
            let k = ms.partition_point(|x| x.0 < sel.end() || (x.0 == sel.start() && x.1 == sel.end() && !sel.is_empty()));
            if k >= ms.len() { 0 } else { k }
        };
        v.find.current = Some(i);
        doc.set_selection(TextSel::new(ms[i].0, ms[i].1));
        v.find.from = ms[i].0;
        self.repass();
    }

    // the find bar: query, case, whole word, previous / next, matches, close; replace row
    fn ce_find_bar(&mut self, id: u32, doc: &mut CodeDoc, v: &mut View, r: [i32; 4], ro: bool) {
        let ids = |k: u32| id.wrapping_add(k);
        self.draw_color(self.color(T_LAYER) | 0xFF00_0000);
        self.draw_rect(r[0], r[1], r[2], r[3]);
        self.draw_color(self.color(T_DIVIDER));
        self.draw_rect(r[0], r[1] + r[3] - self.hair(), r[2], self.hair());
        let pad = self.dp(6);
        let row_h = self.dp(32);
        let bw = self.dp(32);
        let (fq, fr) = (ids(3), ids(4));
        // Esc / F3 / Enter in the boxes
        let in_bar = self.wp.ctx.focus == fq || self.wp.ctx.focus == fr;
        if in_bar {
            if self.ed_key_chord(VK_ESCAPE, 0) {
                v.find.open = false;
                self.ed_focus_quiet(id);
                return;
            }
            if self.ed_key_chord(VK_F3, 0) {
                self.ce_find_step(doc, v, false);
            }
            if self.ed_key_chord(VK_F3, MOD_SHIFT) {
                self.ce_find_step(doc, v, true);
            }
        }
        let shift_enter = self.wp.ctx.focus == fq && self.ed_key_chord(VK_RETURN, MOD_SHIFT);
        let x0 = r[0] + pad;
        let y0 = r[1] + (self.dp(40) - row_h) / 2;
        let count_w = self.dp(90);
        let fw = (r[2] - pad * 3 - bw * 5 - count_w).max(self.dp(80));
        self.next_rect_px(x0, y0, fw, row_h);
        let before = v.find.query.clone();
        // Enter taken before the box (its EV_COMMIT also comes from a focus change)
        let enter = self.wp.ctx.focus == fq && self.ed_key_chord(VK_RETURN, 0);
        self.textbox(fq, Some(tr("Suchen")), &mut v.find.query, 1024);
        if v.find.query != before {
            // incremental: the first match from where the search began
            Ui::ce_find_update(&mut v.find, doc);
            let from = v.find.from;
            let ms = &v.find.matches;
            let k = ms.partition_point(|x| x.0 < from);
            v.find.current = if ms.is_empty() { None } else { Some(if k >= ms.len() { 0 } else { k }) };
            if let Some(i) = v.find.current {
                doc.set_selection(TextSel::new(ms[i].0, ms[i].1));
            }
            self.repass();
        }
        if shift_enter {
            self.ce_find_step(doc, v, true);
        } else if enter {
            self.ce_find_step(doc, v, false);
        }
        let mut x = x0 + fw + pad;
        let tool = |ui: &mut Ui, kid: u32, x: i32, label: &str, tip: &str, on: bool| -> bool {
            ui.next_rect_px(x, y0, bw, row_h);
            ui.next_flags(ST_SUBTLE | if on { ST_CHECKED } else { 0 });
            let e = ui.button(kid, label);
            ui.tooltip(tip);
            e & EV_ACT != 0
        };
        if tool(self, ids(5), x, "Aa", tr("Groß-/Kleinschreibung"), v.find.case != 0) {
            v.find.case ^= 1;
            self.repass();
        }
        x += bw;
        if tool(self, ids(6), x, "W", tr("Ganzes Wort"), v.find.word != 0) {
            v.find.word ^= 1;
            self.repass();
        }
        x += bw;
        self.next_rect_px(x, y0, bw, row_h);
        if self.icon_button(ids(7), IC_CHEV_UP, tr("Vorheriger Treffer")) & EV_ACT != 0 {
            self.ce_find_step(doc, v, true);
        }
        x += bw;
        self.next_rect_px(x, y0, bw, row_h);
        if self.icon_button(ids(8), IC_CHEV_DOWN, tr("Nächster Treffer")) & EV_ACT != 0 {
            self.ce_find_step(doc, v, false);
        }
        x += bw;
        Ui::ce_find_update(&mut v.find, doc);
        // the current match: the one the selection is
        let sel = doc.selection();
        v.find.current = v.find.matches.binary_search_by(|m| m.0.cmp(&sel.start()).then(m.1.cmp(&sel.end()))).ok();
        let count = if v.find.query.is_empty() {
            String::new()
        } else if v.find.matches.is_empty() {
            String::from(tr("Keine Treffer"))
        } else {
            let more = if v.find.matches.len() >= FIND_MAX { "+" } else { "" };
            format!("{}/{}{}", v.find.current.map_or(String::from("?"), |c| format!("{}", c + 1)), v.find.matches.len(), more)
        };
        self.next_rect_px(x + pad / 2, y0, count_w, row_h);
        self.ts_font(TS_CAPTION);
        let tc = self.color(T_TEXT2);
        let (f, s) = self.text.get_font();
        let base = self.ts_base(TS_CAPTION);
        let lr = self.place(count_w, row_h);
        self.cmd_text(lr[0], lr[1] + (row_h - self.ts_line(TS_CAPTION)) / 2 + base, count.as_bytes(), f, s, tc, count_w);
        self.wp.ctx.last_state = 0;
        self.sem(K_TEXT, 0, Some(&count), 0);
        x += count_w;
        self.next_rect_px(x, y0, bw, row_h);
        if self.icon_button(ids(9), IC_CANCEL, tr("Schließen")) & EV_ACT != 0 {
            v.find.open = false;
            self.ed_focus_quiet(id);
        }
        // the replace row
        if v.find.replace && !ro {
            let y1 = y0 + self.dp(36);
            self.next_rect_px(x0, y1, fw, row_h);
            let renter = self.wp.ctx.focus == fr && self.ed_key_chord(VK_RETURN, 0);
            self.textbox(fr, Some(tr("Ersetzen")), &mut v.find.rep, 1024);
            let mut xr = x0 + fw + pad;
            let q = v.find.query.clone();
            let rep = v.find.rep.clone();
            let opts = v.find.opts();
            self.next_rect_px(xr, y1, self.dp(96), row_h);
            if self.button(ids(10), tr("Ersetzen")) & EV_ACT != 0 || renter {
                doc.replace_next(&q, &rep, opts);
                self.repass();
            }
            xr += self.dp(100);
            self.next_rect_px(xr, y1, self.dp(110), row_h);
            if self.button(ids(11), tr("Alle ersetzen")) & EV_ACT != 0 {
                doc.replace_all(&q, &rep, opts);
            }
        }
    }
}

// Backspace in leading blanks: back to the previous tab stop
fn soft_back(doc: &CodeDoc, a: TextPos, b: TextPos) -> (TextPos, TextPos) {
    if a.line != b.line || b.byte == 0 {
        return (a, b);
    }
    let t = doc.line(b.line);
    if !t[..b.byte].bytes().all(|c| c == b' ') {
        return (a, b);
    }
    let tw = doc.tab_width.clamp(1, 16) as usize;
    let col = b.byte;
    let target = (col - 1) / tw * tw;
    (TextPos::new(b.line, target), b)
}

// the last line a diagnostic marks something on (a part over lines ending at a line's start
// ends on the line before)
fn last_line(d: &Mark) -> usize {
    if d.b.byte == 0 && d.b.line > d.a.line { d.b.line - 1 } else { d.b.line }
}

// a diagnostic marks something on line l
fn touches(d: &Mark, l: usize) -> bool {
    d.a.line <= l && l <= last_line(d)
}

// the columns a diagnostic underlines on line l (its text t): over lines to the line's end and
// one cell for its break; at least one cell
fn mark_cols(d: &Mark, l: usize, t: &str, tw: usize) -> (usize, usize) {
    let c0 = if d.a.line == l { col_in(t, d.a.byte, tw) } else { 0 };
    let c1 = if d.b.line == l { col_in(t, d.b.byte, tw) } else { col_in(t, t.len(), tw) + 1 };
    (c0, c1.max(c0 + 1))
}

// the messages of diagnostics (indexes of doc.marks()): "Fehler: ...", joined by " · "
fn diag_text(doc: &CodeDoc, ix: &[usize]) -> String {
    let mut s = String::new();
    for (k, &i) in ix.iter().enumerate() {
        let d = &doc.marks()[i];
        if k > 0 {
            s.push_str(" · ");
        }
        s.push_str(tr(["Fehler", "Warnung", "Hinweis"][d.sev.min(DIAG_INFO) as usize]));
        s.push_str(": ");
        s.push_str(&d.msg);
    }
    s
}

// the matches of lines first .. first + n
fn doc_find_range(doc: &CodeDoc, q: &str, opts: u8, first: usize, n: usize) -> Vec<(TextPos, TextPos)> {
    doc.find_lines(q, opts, first, n)
}

// the widest line and the size of all lines (bytes) at the document's revision
fn line_sizes(doc: &CodeDoc) -> (u32, u64, usize, usize) {
    let (mut w, mut t) = (0, 0);
    for l in doc.lines() {
        w = w.max(l.len());
        t += l.len() + 1;
    }
    (doc.uid(), doc.revision(), w + 1, t)
}
