// code_assist -- what a language gives ui.code_editor beyond highlighting: completion, hover
// texts and signature help (CodeAssist, through ui.code_editor_with), the requests of the
// editor that the app answers (CodeEv, ui.code_events), and the popups that show them
//
//   trait CodeAssist        complete(doc, at, explicit) -> Option<CompList>   the items for
//                           the word at the caret (explicit: Ctrl+Space; else typing a word or
//                           a trigger character of triggers(), "." by default)
//                           hover(doc, at) -> Option<String>   the text of a tooltip over the
//                           word at a position (lines by '\n'; long ones are wrapped)
//                           signature(doc, at) -> Option<SigHelp>   inside a call's parentheses
//                           Every method has a default that offers nothing.
//   CompItem { label, detail, doc, kind: CI_*, insert }   an item: what the list shows, a short
//                           line beside it (a signature), a description (shown under the list
//                           for the selected item), its kind (a letter in a badge), the text
//                           it inserts ("": the label)
//   CompList { start, items }   the items replace the word from start to the end of the word
//                           at the caret
//   SigHelp { label, params, active, doc }   "split(sep, n)", the byte ranges of the
//                           parameters in it, the one at the caret (drawn bold), a description
//   CodeEv                  Definition(pos) (Ctrl+click, F12), Rename(pos) (F2), Format
//                           (Shift+Alt+F), Breakpoint(line) (a click in the breakpoints' column)
//
// The completion popup opens while a word is typed (once per word: a word without items keeps
// it closed until it ends), after a trigger character and on Ctrl+Space; it filters as one
// types (the items whose label starts with the typed part, ignoring case, those that match
// the case first, otherwise in the order of the assist).  Up / Down / PgUp / PgDn choose, Enter
// or Tab insert (the word from the list's start to the end of the word at the caret becomes
// the item's text, one undo step, the caret after it), Esc closes, a click on an item inserts
// it; leaving the word, a character that is no part of a word, a click elsewhere or the focus
// leaving close it.  At most POP_ROWS rows show, the wheel scrolls them.  Signature help opens
// when ( or , is typed and on Ctrl+Shift+Space, follows the caret and closes outside a call or
// on Esc.  Both are drawn on the popup layer below / above the caret line within the window,
// in the code theme's colours (high contrast too), and recorded for screen readers: the list
// as a K_LIST (id + 14) with a K_LISTITEM per visible row (the chosen one SS_SELECTED, label
// and detail), signature help as a K_TEXT.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::code_doc::{is_word, CodeDoc};
use crate::consts::*;
use crate::ed_core::TextPos;
use crate::highlight::*;
use crate::i18n::tr;
use crate::Ui;

/// completion item kinds: plain text
pub const CI_TEXT: u8 = 0;
pub const CI_KEYWORD: u8 = 1;
/// a variable (let, loop, catch)
pub const CI_VARIABLE: u8 = 2;
pub const CI_PARAM: u8 = 3;
pub const CI_FUNCTION: u8 = 4;
pub const CI_METHOD: u8 = 5;
/// a module or namespace
pub const CI_MODULE: u8 = 6;
pub const CI_CONSTANT: u8 = 7;
pub const CI_FIELD: u8 = 8;
/// a value of the host (a configuration)
pub const CI_VALUE: u8 = 9;

/// the rows the completion popup shows at most
pub const POP_ROWS: usize = 10;

/// a completion item (see the module header)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompItem {
    pub label: String,
    pub detail: String,
    pub doc: String,
    pub kind: u8,
    /// the text it inserts ("": the label)
    pub insert: String,
}

impl CompItem {
    pub fn new(label: &str, kind: u8, detail: &str) -> CompItem {
        CompItem { label: String::from(label), detail: String::from(detail), doc: String::new(), kind, insert: String::new() }
    }
}

/// what complete() offers: the items replace the word from start on
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompList {
    pub start: TextPos,
    pub items: Vec<CompItem>,
}

/// signature help inside a call
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SigHelp {
    /// "split(sep, n)"
    pub label: String,
    /// the bytes of each parameter in label
    pub params: Vec<(usize, usize)>,
    /// the parameter at the caret (params.len(): none)
    pub active: usize,
    pub doc: String,
}

/// a language's services for ui.code_editor_with (see the module header)
pub trait CodeAssist {
    /// the items for the word at the caret `at` (explicit: Ctrl+Space)
    fn complete(&mut self, doc: &CodeDoc, at: TextPos, explicit: bool) -> Option<CompList> {
        let _ = (doc, at, explicit);
        None
    }
    /// the tooltip text for the word at `at`
    fn hover(&mut self, doc: &CodeDoc, at: TextPos) -> Option<String> {
        let _ = (doc, at);
        None
    }
    /// signature help at the caret
    fn signature(&mut self, doc: &CodeDoc, at: TextPos) -> Option<SigHelp> {
        let _ = (doc, at);
        None
    }
    /// characters after which completion opens
    fn triggers(&self) -> &str {
        "."
    }
}

/// what the user asked of a code editor in its last pass (ui.code_events)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeEv {
    /// go to the definition of the word at the position (Ctrl+click, F12)
    Definition(TextPos),
    /// rename the word at the position (F2)
    Rename(TextPos),
    /// format the document (Shift+Alt+F)
    Format,
    /// a click in the breakpoints' column of line (from 0)
    Breakpoint(usize),
}

// ---------------------------------------------------------------- the popups' state

/// the completion popup of an editor
#[derive(Default)]
pub(crate) struct Pop {
    pub(crate) open: bool,
    pub(crate) start: TextPos,
    pub(crate) items: Vec<CompItem>,
    /// the items that match the typed part, in the order shown
    pub(crate) shown: Vec<usize>,
    pub(crate) sel: usize,
    pub(crate) top: usize,
    /// the typed part `shown` belongs to
    pub(crate) typed: Option<String>,
    /// a word that had no items: (line, its start), auto-opening stays off there
    pub(crate) quiet: Option<(usize, usize)>,
}

impl Pop {
    pub(crate) fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.shown.clear();
        self.typed = None;
    }

    /// the list of an assist at the caret (None or no items: closed; a word typed without
    /// items stays quiet)
    pub(crate) fn request(&mut self, doc: &CodeDoc, a: &mut (dyn CodeAssist + '_), explicit: bool) {
        let caret = doc.selection().caret;
        match a.complete(doc, caret, explicit) {
            Some(l) if !l.items.is_empty() && l.start.line == caret.line && l.start.byte <= caret.byte => {
                self.open = true;
                self.start = l.start;
                self.items = l.items;
                self.typed = None;
                self.quiet = None;
                if !self.filter(doc) {
                    self.quiet = Some((caret.line, l.start.byte));
                }
            }
            _ => {
                self.close();
                let (a, _) = word_around(doc.line(caret.line), caret.byte);
                self.quiet = Some((caret.line, a));
            }
        }
    }

    /// the items for the typed part again -> still open (the caret left the word, or no item
    /// matches: closed)
    pub(crate) fn filter(&mut self, doc: &CodeDoc) -> bool {
        if !self.open {
            return false;
        }
        let c = doc.selection().caret;
        let t = doc.line(c.line);
        if c.line != self.start.line || c.byte < self.start.byte || !doc.selection().is_empty() || self.start.byte > t.len() {
            self.close();
            return false;
        }
        let typed = &t[self.start.byte..c.byte];
        if !typed.chars().all(is_word) {
            self.close();
            return false;
        }
        if self.typed.as_deref() == Some(typed) {
            return true;
        }
        let low = typed.to_lowercase();
        let mut exact: Vec<usize> = Vec::new();
        let mut other: Vec<usize> = Vec::new();
        for (i, it) in self.items.iter().enumerate() {
            if it.label.starts_with(typed) {
                exact.push(i);
            } else if it.label.to_lowercase().starts_with(&low) {
                other.push(i);
            }
        }
        exact.extend(other);
        self.shown = exact;
        if self.shown.is_empty() {
            self.close();
            return false;
        }
        // another typed part: the best item first
        self.typed = Some(String::from(typed));
        self.sel = 0;
        self.top = 0;
        true
    }

    pub(crate) fn scroll_to_sel(&mut self) {
        if self.sel < self.top {
            self.top = self.sel;
        }
        if self.sel >= self.top + POP_ROWS {
            self.top = self.sel + 1 - POP_ROWS;
        }
        self.top = self.top.min(self.shown.len().saturating_sub(POP_ROWS));
    }

    /// move the choice by delta rows (clamped)
    pub(crate) fn step(&mut self, delta: isize) {
        let n = self.shown.len() as isize;
        if n == 0 {
            return;
        }
        self.sel = (self.sel as isize + delta).clamp(0, n - 1) as usize;
        self.scroll_to_sel();
    }

    /// insert the chosen item (one undo step) and close
    pub(crate) fn accept(&mut self, doc: &mut CodeDoc) {
        let Some(it) = self.shown.get(self.sel).and_then(|&i| self.items.get(i)) else { return self.close() };
        let text = if it.insert.is_empty() { it.label.clone() } else { it.insert.clone() };
        let c = doc.selection().caret;
        let t = doc.line(c.line);
        let end = c.byte + t[c.byte.min(t.len())..].chars().take_while(|&ch| is_word(ch)).map(char::len_utf8).sum::<usize>();
        let (a, b) = (self.start, TextPos::new(c.line, end));
        let _ = doc.replace_kind(a, b, &text, crate::code_doc::CK_NONE, tr("Vervollständigen"));
        self.close();
    }
}

/// signature help of an editor: open, what it shows, the revision and caret it belongs to
#[derive(Default)]
pub(crate) struct Sig {
    pub(crate) open: bool,
    pub(crate) help: Option<SigHelp>,
    pub(crate) key: (u64, TextPos),
}

/// the hover text of an editor: (document, revision, line, word start) -> the text
#[derive(Default)]
pub(crate) struct Hov {
    pub(crate) key: (u32, u64, usize, usize),
    pub(crate) text: Option<String>,
}

/// the word around byte b of t: (start, end) (b itself when there is none)
pub(crate) fn word_around(t: &str, b: usize) -> (usize, usize) {
    let b = b.min(t.len());
    let a = t[..b].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(b, |(i, _)| i);
    let e = t[b..].char_indices().find(|(_, c)| !is_word(*c)).map_or(t.len(), |(i, _)| b + i);
    (a, e)
}

/// lines of at most max characters (at blanks; a longer word stays whole)
pub(crate) fn wrap(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for (k, line) in s.split('\n').enumerate() {
        if k > 0 {
            out.push('\n');
        }
        let mut n = 0;
        for (j, w) in line.split(' ').enumerate() {
            let wn = w.chars().count();
            if j > 0 && n + 1 + wn > max {
                out.push('\n');
                n = 0;
            } else if j > 0 {
                out.push(' ');
                n += 1;
            }
            out.push_str(w);
            n += wn;
        }
    }
    out
}

/// the font and cells of the editor (code_edit.rs Metrics)
pub(crate) struct PopFont {
    pub(crate) font: u32,
    pub(crate) size: i32,
    pub(crate) cw: i32,
    pub(crate) lh: i32,
    pub(crate) base: i32,
}

// the letter and the token colour of an item kind's badge
fn badge(kind: u8, t: &CodeTheme) -> (&'static str, u32) {
    match kind {
        CI_KEYWORD => ("k", t.tokens[SY_KEYWORD as usize]),
        CI_VARIABLE => ("v", t.tokens[SY_NUMBER as usize]),
        CI_PARAM => ("p", t.tokens[SY_NUMBER as usize]),
        CI_FUNCTION => ("f", t.tokens[SY_FUNC as usize]),
        CI_METHOD => ("m", t.tokens[SY_FUNC as usize]),
        CI_MODULE => ("M", t.tokens[SY_TYPE as usize]),
        CI_CONSTANT => ("c", t.tokens[SY_CONST as usize]),
        CI_FIELD => ("F", t.tokens[SY_KEY as usize]),
        CI_VALUE => ("w", t.tokens[SY_KEY as usize]),
        _ => ("t", t.fg),
    }
}

impl Ui {
    // the window's rectangle (px) for popups, the hit clip set to it; -> the old clip
    fn ca_layer_begin(&mut self) -> ([i32; 4], u32, [i32; 4], u32) {
        let (w, h) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        let prev = self.layer_set(L_POPUP);
        let clip = self.wp.ctx.uiclip;
        self.wp.ctx.uiclip = [0, 0, w, h];
        ([0, 0, w, h], prev, clip, self.wp.ctx.last_state)
    }

    fn ca_layer_end(&mut self, prev: u32, clip: [i32; 4], lr: [i32; 4], ls: u32) {
        self.wp.ctx.uiclip = clip;
        self.layer_set(prev);
        self.wp.ctx.last_rect = lr;
        self.wp.ctx.last_state = ls;
    }

    /// (code_edit.rs) the completion popup of editor id: below the caret line (x: the word's
    /// start, below: the line's foot, above: its top), above it when there is no room ->
    /// accepted (a click on an item)
    pub(crate) fn ca_complete_popup(&mut self, id: u32, pop: &mut Pop, doc: &mut CodeDoc, x: i32, below: i32, above: i32, f: &PopFont, t: &CodeTheme) -> bool {
        if !pop.open || pop.shown.is_empty() {
            return false;
        }
        let lr = self.wp.ctx.last_rect;
        let (win, prev, clip, ls) = self.ca_layer_begin();
        let pad = self.dp(4);
        let rh = f.lh.max(self.dp(22));
        let rows = pop.shown.len().min(POP_ROWS);
        let first = pop.top.min(pop.shown.len().saturating_sub(rows));
        // the columns: a badge, the labels (code font), the details (caption font)
        let lchars = pop.shown.iter().map(|&i| pop.items[i].label.chars().count()).max().unwrap_or(1).clamp(6, 40) as i32;
        self.ts_font(TS_CAPTION);
        let (cf, cs) = self.text.get_font();
        let cap_line = self.ts_line(TS_CAPTION);
        let cap_base = self.ts_base(TS_CAPTION);
        let max_d = self.dp(300);
        let mut dw = 0;
        for &i in &pop.shown[first..first + rows] {
            dw = dw.max(self.text.width(&pop.items[i].detail).min(max_d));
        }
        let badge_w = rh;
        let w = (badge_w + pad + lchars * f.cw + if dw > 0 { self.dp(16) + dw } else { 0 } + pad * 2).max(self.dp(180)).min(win[2] - self.dp(8));
        // the selected item's description under the rows
        let docs = pop.shown.get(pop.sel).map(|&i| wrap(&pop.items[i].doc, ((w - pad * 4) / (cap_line / 2).max(1)).max(20) as usize)).unwrap_or_default();
        let doc_lines = if docs.is_empty() { 0 } else { docs.split('\n').count().min(4) as i32 };
        let foot = if doc_lines > 0 { doc_lines * cap_line + pad * 2 + self.hair() } else { 0 };
        let h = rows as i32 * rh + pad * 2 + foot;
        let mut y = below;
        if y + h > win[3] && above - h >= 0 {
            y = above - h;
        }
        // (the labels start below the word)
        let x = (x - pad * 2 - badge_w).min(win[2] - w - self.dp(4)).max(self.dp(4));
        // the surface
        let hair = self.hair();
        let bg = self.color(T_FLYOUT) | 0xFF00_0000;
        self.cmd_rrect(x, y, w, h, self.dp(4), hair, bg, self.color(T_FLYOUT_STROKE), self.color(T_FLYOUT_STROKE));
        let ry0 = y + pad;
        for (k, &i) in pop.shown[first..first + rows].iter().enumerate() {
            let it = &pop.items[i];
            let ry = ry0 + k as i32 * rh;
            let chosen = first + k == pop.sel;
            let fg = if chosen && t.sel_text != 0 { t.sel_text } else { t.fg };
            if chosen {
                self.draw_color(t.sel);
                self.draw_rrect(x + pad, ry, w - pad * 2, rh, self.dp(3));
            }
            let (letter, bc) = badge(it.kind, t);
            let bc = if chosen && t.sel_text != 0 { t.sel_text } else { bc };
            let bs = (rh * 2 / 3).max(8);
            let (bx, by) = (x + pad + (badge_w - bs) / 2, ry + (rh - bs) / 2);
            self.draw_color(bc);
            self.draw_frame(bx, by, bs, bs, self.dp(2), hair);
            let lw = self.text.width(letter);
            self.cmd_text(bx + (bs - lw) / 2, by + (bs - cap_line) / 2 + cap_base, letter.as_bytes(), cf, cs, bc, 0);
            let lx = x + pad + badge_w + pad;
            let ly = ry + (rh - f.lh) / 2 + f.base;
            self.cmd_text(lx, ly, it.label.as_bytes(), f.font, f.size, fg, lchars * f.cw);
            if !it.detail.is_empty() {
                let dx = x + w - pad * 2 - dw;
                let dc = if chosen && t.sel_text != 0 { t.sel_text } else { t.gutter_fg };
                self.cmd_text(dx.max(lx + lchars * f.cw + self.dp(8)), ry + (rh - cap_line) / 2 + cap_base, it.detail.as_bytes(), cf, cs, dc, dw);
            }
        }
        if doc_lines > 0 {
            let fy = ry0 + rows as i32 * rh + pad;
            self.draw_color(self.color(T_DIVIDER));
            self.draw_rect(x + pad, fy, w - pad * 2, hair);
            for (k, l) in docs.split('\n').take(4).enumerate() {
                self.cmd_text(x + pad * 2, fy + pad + k as i32 * cap_line + cap_base, l.as_bytes(), cf, cs, t.fg, w - pad * 4);
            }
        }
        // the pointer: a click inserts, the wheel scrolls
        self.wp.ctx.last_rect = [x, y, w, h];
        let ev = self.behave(id, 0);
        let mut accepted = false;
        if ev & EV_HOVER != 0 {
            let wy = self.wp.ctx.wheel_y;
            if wy != 0 && pop.shown.len() > POP_ROWS {
                let d = if wy > 0 { -3 } else { 3 };
                pop.top = (pop.top as isize + d).clamp(0, (pop.shown.len() - POP_ROWS) as isize) as usize;
                self.wp.ctx.wheel_y = 0;
                self.repass();
            }
        }
        if ev & EV_ACT != 0 {
            let my = self.wp.ctx.my;
            let k = ((my - ry0) / rh.max(1)).clamp(0, rows as i32 - 1) as usize;
            if my >= ry0 && my < ry0 + rows as i32 * rh {
                pop.sel = first + k;
                pop.accept(doc);
                accepted = true;
            }
        }
        // the records: the list and its visible rows
        self.wp.ctx.last_rect = [x, y, w, h];
        self.wp.ctx.last_state = 0;
        self.sem(K_LIST, id, Some(tr("Vorschläge")), pop.sel as i32);
        if !accepted {
            for k in 0..rows {
                let it = &pop.items[pop.shown[first + k]];
                let label = if it.detail.is_empty() { it.label.clone() } else { format!("{} – {}", it.label, it.detail) };
                self.wp.ctx.last_rect = [x + pad, ry0 + k as i32 * rh, w - pad * 2, rh];
                self.wp.ctx.last_state = if first + k == pop.sel { SS_SELECTED as u32 } else { 0 };
                self.sem(K_LISTITEM, 0, Some(&label), (first + k) as i32);
            }
            if doc_lines > 0 {
                self.wp.ctx.last_rect = [x, ry0 + rows as i32 * rh, w, foot];
                self.wp.ctx.last_state = 0;
                self.sem(K_TEXT, 0, Some(&docs.replace('\n', " ")), 0);
            }
        }
        self.ca_layer_end(prev, clip, lr, ls);
        accepted
    }

    /// (code_edit.rs) signature help above the caret line (top: the line's top, foot: its
    /// foot for below), x near the caret
    pub(crate) fn ca_signature_popup(&mut self, sig: &SigHelp, x: i32, top: i32, foot: i32, f: &PopFont, t: &CodeTheme) {
        let lr = self.wp.ctx.last_rect;
        let (win, prev, clip, ls) = self.ca_layer_begin();
        let pad = self.dp(6);
        self.ts_font(TS_CAPTION);
        let (cf, cs) = self.text.get_font();
        let cap_line = self.ts_line(TS_CAPTION);
        let cap_base = self.ts_base(TS_CAPTION);
        let chars = sig.label.chars().count() as i32;
        let doc = wrap(&sig.doc, 70);
        let doc_lines = if doc.is_empty() { 0 } else { doc.split('\n').count().min(3) as i32 };
        let mut dw = 0;
        for l in doc.split('\n').take(3) {
            dw = dw.max(self.text.width(l));
        }
        let w = (chars * f.cw).max(dw) + pad * 2;
        let w = w.min(win[2] - self.dp(8));
        let h = f.lh + pad * 2 + doc_lines * cap_line;
        let mut y = top - h - self.dp(2);
        if y < 0 {
            y = foot + self.dp(2);
        }
        let x = x.min(win[2] - w - self.dp(4)).max(self.dp(4));
        let hair = self.hair();
        let bg = self.color(T_FLYOUT) | 0xFF00_0000;
        self.cmd_rrect(x, y, w, h, self.dp(4), hair, bg, self.color(T_FLYOUT_STROKE), self.color(T_FLYOUT_STROKE));
        // the label, the active parameter bold and underlined
        let lx = x + pad;
        let ly = y + pad + f.base;
        self.cmd_text(lx, ly, sig.label.as_bytes(), f.font, f.size, t.fg, w - pad * 2);
        let mut said = String::new();
        if let Some(&(a, b)) = sig.params.get(sig.active) {
            if b <= sig.label.len() && a < b {
                let c0 = sig.label[..a].chars().count() as i32;
                let c1 = sig.label[..b].chars().count() as i32;
                let px = lx + c0 * f.cw;
                let part = &sig.label[a..b];
                let c = t.tokens[SY_FUNC as usize];
                self.cmd_text(px, ly, part.as_bytes(), f.font, f.size, c, 0);
                self.cmd_text(px + (f.size >> 8) / 16 + 1, ly, part.as_bytes(), f.font, f.size, c, 0);
                self.draw_color(c);
                self.draw_rect(px, ly + self.dp(2), (c1 - c0) * f.cw, hair.max(self.dp(1)));
                said = String::from(part);
            }
        }
        for (k, l) in doc.split('\n').take(3).enumerate() {
            self.cmd_text(lx, y + pad + f.lh + k as i32 * cap_line + cap_base, l.as_bytes(), cf, cs, t.gutter_fg, w - pad * 2);
        }
        self.wp.ctx.last_rect = [x, y, w, h];
        self.wp.ctx.last_state = 0;
        let mut text = sig.label.clone();
        if !said.is_empty() {
            text.push_str(" – ");
            text.push_str(&said);
        }
        if !doc.is_empty() {
            text.push_str(" – ");
            text.push_str(&doc.replace('\n', " "));
        }
        self.sem(K_TEXT, 0, Some(&text), 0);
        self.ca_layer_end(prev, clip, lr, ls);
    }
}
