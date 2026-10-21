// code_doc -- the document of ui.code_editor: lines of text, the selection, undo (ui::History),
// the indentation settings and the edits of a code editor
//
//   CodeDoc::new() / from_text(s)       ('\r\n', '\r' and '\n' end lines)
//   doc.text() / line_count() / line(i) / lines() / selection() / set_selection(sel)
//   doc.revision()                      counts content changes (edits, undo, redo)
//   edits (each one undoable step; typing and repeated Backspace / Delete merge):
//     replace(a, b, s) / insert(s) / delete(a, b) / newline(openers) (auto-indent: the line's
//     indentation, one level more after an opener, a closer right of the caret on a line of
//     its own) / type_text(s) (a closer typed on a blank line goes one level out) / indent(delta)
//     (block indent / unindent of the selected lines) / tab() / back_tab() /
//     toggle_comment(prefix) / replace_all(q, rep, FIND_*) / replace_next(q, rep, FIND_*)
//   find: find_all(q, FIND_*) / find_next(from, q, FIND_*, backwards) (FIND_CASE matches the
//     case, FIND_WORD whole words)
//   doc.undo() / redo() / can_undo() / can_redo() / undo_label() / is_dirty() / mark_saved()
//   tab_width (1..16, default 4), use_tabs (indent with tabs instead of blanks)
//   col_of(line, byte) / byte_at_col(line, col)   visual columns (tabs to the next stop,
//   wide East Asian characters two)
//
// An edit replaces a range of whole lines (LineEdit: old and new lines, the selection before
// and after); each change is also kept in a short journal (changes_since) that the editor's
// highlighting states and the find results follow without starting again from the top.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use text::segment;

use crate::ed_core::{TextPos, TextSel};
use crate::history::{Edit, History};
use crate::i18n::tr;
use crate::rich_doc::lines_of;

/// find: match the case
pub const FIND_CASE: u8 = 1;
/// find: whole words only
pub const FIND_WORD: u8 = 2;
/// find_all stops after this many matches
pub const FIND_MAX: usize = 100_000;

// merge classes of LineEdit
pub(crate) const CK_NONE: u8 = 0;
pub(crate) const CK_TYPE: u8 = 1;
pub(crate) const CK_BACK: u8 = 2;
pub(crate) const CK_DEL: u8 = 3;

/// a change of the lines: first .. first + old became first .. first + new (at revision rev)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineChange {
    pub rev: u64,
    pub first: usize,
    pub old: usize,
    pub new: usize,
}

const JOURNAL_MAX: usize = 256;

/// the content an edit changes: the lines, the selection and the change journal
pub(crate) struct CodeBody {
    lines: Vec<String>,
    sel: TextSel,
    rev: u64,
    journal: Vec<LineChange>,
}

impl CodeBody {
    fn splice(&mut self, first: usize, old: usize, new: &[String]) {
        self.lines.splice(first..first + old, new.iter().cloned());
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.rev += 1;
        if self.journal.len() >= JOURNAL_MAX {
            self.journal.drain(..JOURNAL_MAX / 2);
        }
        self.journal.push(LineChange { rev: self.rev, first, old, new: new.len() });
    }
}

struct LineEdit {
    first: usize,
    old: Vec<String>,
    new: Vec<String>,
    sel0: TextSel,
    sel1: TextSel,
    label: String,
    kind: u8,
}

impl Edit<CodeBody> for LineEdit {
    fn apply(&mut self, s: &mut CodeBody) {
        s.splice(self.first, self.old.len(), &self.new);
        s.sel = self.sel1;
    }
    fn revert(&mut self, s: &mut CodeBody) {
        s.splice(self.first, self.new.len(), &self.old);
        s.sel = self.sel0;
    }
    fn label(&self) -> &str {
        &self.label
    }
    fn merge(&mut self, next: &mut dyn Edit<CodeBody>) -> bool {
        if self.kind == CK_NONE {
            return false;
        }
        match next.value().and_then(|v| v.downcast_ref::<LineEdit>()) {
            Some(n) if n.kind == self.kind && n.first == self.first && n.sel0 == self.sel1 && n.old == self.new => {
                self.new = n.new.clone();
                self.sel1 = n.sel1;
                true
            }
            _ => false,
        }
    }
    fn value(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }
}

/// the document of ui.code_editor (see the module header)
pub struct CodeDoc {
    body: CodeBody,
    hist: History<CodeBody>,
    // tells documents apart (editor caches)
    uid: u32,
    /// columns between tab stops (1..16)
    pub tab_width: u8,
    /// indent with tabs (else with blanks)
    pub use_tabs: bool,
}

impl Default for CodeDoc {
    fn default() -> CodeDoc {
        CodeDoc::new()
    }
}

// a character of a word (whole-word search, double click)
pub(crate) fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

// a simple case fold
pub(crate) fn fold(c: char) -> char {
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// the columns a character takes: 2 for wide East Asian characters, 0 for marks inside a
/// cluster (the caller counts clusters), else 1
pub(crate) fn char_cols(c: char) -> usize {
    // text::segment's packed properties: bit 16 East Asian wide / full width
    if segment::char_props(c) & (1 << 16) != 0 { 2 } else { 1 }
}

impl CodeDoc {
    /// one empty line
    pub fn new() -> CodeDoc {
        CodeDoc::from_lines(Vec::new())
    }

    /// a text: a line per '\n' ('\r\n' and '\r' too)
    pub fn from_text(s: &str) -> CodeDoc {
        CodeDoc::from_lines(lines_of(s).map(String::from).collect())
    }

    /// lines (an empty list: one empty line); they must not hold '\n'
    pub fn from_lines(mut lines: Vec<String>) -> CodeDoc {
        if lines.is_empty() {
            lines.push(String::new());
        }
        for l in lines.iter_mut() {
            if l.contains(['\n', '\r']) {
                *l = l.replace(['\n', '\r'], "");
            }
        }
        static NEXT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1);
        let uid = NEXT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        CodeDoc { body: CodeBody { lines, sel: TextSel::default(), rev: 0, journal: Vec::new() }, hist: History::new(1000), uid, tab_width: 4, use_tabs: false }
    }

    /// the text: the lines joined by '\n'
    pub fn text(&self) -> String {
        let n: usize = self.body.lines.iter().map(|l| l.len() + 1).sum();
        let mut s = String::with_capacity(n);
        for (i, l) in self.body.lines.iter().enumerate() {
            if i > 0 {
                s.push('\n');
            }
            s.push_str(l);
        }
        s
    }

    /// the number of lines (at least 1)
    pub fn line_count(&self) -> usize {
        self.body.lines.len()
    }

    /// line i (clamped to the last one), without its '\n'
    pub fn line(&self, i: usize) -> &str {
        &self.body.lines[i.min(self.body.lines.len() - 1)]
    }

    /// every line
    pub fn lines(&self) -> &[String] {
        &self.body.lines
    }

    /// counts the content changes (edits, undo, redo)
    pub fn revision(&self) -> u64 {
        self.body.rev
    }

    // (code_edit.rs) a number of this document instance
    pub(crate) fn uid(&self) -> u32 {
        self.uid
    }

    /// the changes after revision rev, oldest first (None: the journal no longer reaches back
    /// that far -- start again)
    pub fn changes_since(&self, rev: u64) -> Option<&[LineChange]> {
        if rev == self.body.rev {
            return Some(&[]);
        }
        let j = &self.body.journal;
        let i = j.iter().position(|c| c.rev > rev)?;
        (j[i].rev == rev + 1).then_some(&j[i..])
    }

    /// the selection (the caret: anchor == caret)
    pub fn selection(&self) -> TextSel {
        self.body.sel
    }

    /// move the selection (clamped, not undoable)
    pub fn set_selection(&mut self, sel: TextSel) {
        self.body.sel = TextSel::new(self.clamp(sel.anchor), self.clamp(sel.caret));
    }

    /// p inside the document, on a character boundary
    pub fn clamp(&self, p: TextPos) -> TextPos {
        let line = p.line.min(self.body.lines.len() - 1);
        let t = &self.body.lines[line];
        let mut b = p.byte.min(t.len());
        while !t.is_char_boundary(b) {
            b -= 1;
        }
        TextPos::new(line, b)
    }

    /// the end of the document
    pub fn end(&self) -> TextPos {
        let l = self.body.lines.len() - 1;
        TextPos::new(l, self.body.lines[l].len())
    }

    /// the text between a and b
    pub fn text_range(&self, a: TextPos, b: TextPos) -> String {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        let mut s = String::new();
        for l in a.line..=b.line {
            let t = &self.body.lines[l];
            if l > a.line {
                s.push('\n');
            }
            s.push_str(&t[if l == a.line { a.byte } else { 0 }..if l == b.line { b.byte } else { t.len() }]);
        }
        s
    }

    /// the selected text (lines joined by '\n')
    pub fn selected_text(&self) -> String {
        let (a, b) = self.body.sel.range();
        self.text_range(a, b)
    }

    /// the byte offset of p in text()
    pub fn flat(&self, p: TextPos) -> usize {
        let p = self.clamp(p);
        self.body.lines[..p.line].iter().map(|l| l.len() + 1).sum::<usize>() + p.byte
    }

    /// the position of byte offset f of text()
    pub fn unflat(&self, f: usize) -> TextPos {
        let mut f = f;
        for (i, l) in self.body.lines.iter().enumerate() {
            if f <= l.len() {
                return self.clamp(TextPos::new(i, f));
            }
            f -= l.len() + 1;
        }
        self.end()
    }

    // ------------------------------------------------------------------ columns

    /// the visual column of byte in a line (tabs to the next stop, wide characters 2)
    pub fn col_of(&self, line: usize, byte: usize) -> usize {
        col_in(self.line(line), byte, self.tab_width as usize)
    }

    /// the byte of the cluster boundary nearest to column col
    pub fn byte_at_col(&self, line: usize, col: usize) -> usize {
        byte_in(self.line(line), col, self.tab_width as usize)
    }

    // ------------------------------------------------------------------ history

    /// there is a step to undo
    pub fn can_undo(&self) -> bool {
        self.hist.can_undo()
    }
    /// there is a step to redo
    pub fn can_redo(&self) -> bool {
        self.hist.can_redo()
    }
    /// the label of the step undo takes back
    pub fn undo_label(&self) -> Option<&str> {
        self.hist.undo_label()
    }
    /// the label of the step redo does again
    pub fn redo_label(&self) -> Option<&str> {
        self.hist.redo_label()
    }
    /// take the last step back -> false: nothing to undo
    pub fn undo(&mut self) -> bool {
        self.hist.undo(&mut self.body)
    }
    /// do the last undone step again -> false: nothing to redo
    pub fn redo(&mut self) -> bool {
        self.hist.redo(&mut self.body)
    }
    /// the content differs from the state of the last mark_saved
    pub fn is_dirty(&self) -> bool {
        self.hist.dirty()
    }
    /// the content is saved now
    pub fn mark_saved(&mut self) {
        self.hist.mark_saved();
    }
    /// forget the undo steps
    pub fn clear_history(&mut self) {
        self.hist.clear();
    }

    // lines first .. first + n become new; the selection then sel
    fn edit(&mut self, first: usize, n: usize, mut new: Vec<String>, sel: TextSel, label: &str, kind: u8) -> bool {
        if new.is_empty() && n == self.body.lines.len() {
            new.push(String::new());
        }
        if self.body.lines[first..first + n] == new[..] {
            self.body.sel = sel;
            return false;
        }
        let old = self.body.lines[first..first + n].to_vec();
        let e = LineEdit { first, old, new, sel0: self.body.sel, sel1: sel, label: String::from(label), kind };
        self.hist.apply(&mut self.body, Box::new(e));
        true
    }

    // the lines that replace a..b by s -> (lines, the end of s)
    fn spliced(&self, a: TextPos, b: TextPos, s: &str) -> (Vec<String>, TextPos) {
        let head = &self.body.lines[a.line][..a.byte];
        let tail = &self.body.lines[b.line][b.byte..];
        let mut out: Vec<String> = Vec::new();
        let mut first = true;
        for l in lines_of(s) {
            if first {
                let mut h = String::with_capacity(head.len() + l.len() + tail.len());
                h.push_str(head);
                h.push_str(l);
                out.push(h);
                first = false;
            } else {
                out.push(String::from(l));
            }
        }
        let k = out.len() - 1;
        let end = TextPos::new(a.line + k, out[k].len());
        out[k].push_str(tail);
        (out, end)
    }

    // ------------------------------------------------------------------ edits

    /// replace a..b by s; the caret after it -> the end of s
    pub fn replace(&mut self, a: TextPos, b: TextPos, s: &str) -> TextPos {
        self.replace_kind(a, b, s, CK_NONE, tr("Ersetzen"))
    }

    pub(crate) fn replace_kind(&mut self, a: TextPos, b: TextPos, s: &str, kind: u8, label: &str) -> TextPos {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        let (new, end) = self.spliced(a, b, s);
        self.edit(a.line, b.line - a.line + 1, new, TextSel::at(end), label, kind);
        end
    }

    /// replace the selection by s (a paste)
    pub fn insert(&mut self, s: &str) -> TextPos {
        let (a, b) = self.body.sel.range();
        self.replace_kind(a, b, s, CK_NONE, tr("Einfügen"))
    }

    /// remove a..b
    pub fn delete(&mut self, a: TextPos, b: TextPos) -> bool {
        self.delete_kind(a, b, CK_NONE)
    }

    pub(crate) fn delete_kind(&mut self, a: TextPos, b: TextPos, kind: u8) -> bool {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        if a == b {
            return false;
        }
        let rev = self.body.rev;
        self.replace_kind(a, b, "", kind, tr("Löschen"));
        self.body.rev != rev
    }

    /// the leading blanks of a line
    pub fn indentation(&self, line: usize) -> &str {
        let t = self.line(line);
        &t[..t.len() - t.trim_start_matches([' ', '\t']).len()]
    }

    /// one level of indentation (a tab or tab_width blanks)
    pub fn indent_unit(&self) -> String {
        if self.use_tabs { String::from("\t") } else { " ".repeat(self.tab_width.clamp(1, 16) as usize) }
    }

    /// typed text replaces the selection; a closer of `closers` typed as the first character
    /// of a blank line takes one level of indentation away
    pub fn type_text(&mut self, s: &str, closers: &str) -> bool {
        let (a, b) = self.body.sel.range();
        let rev = self.body.rev;
        let line = self.line(a.line);
        let mut chars = s.chars();
        let one = chars.next().filter(|_| chars.next().is_none());
        if let Some(c) = one.filter(|c| closers.contains(*c)) {
            if a.line == b.line && line[..a.byte].trim().is_empty() && !line[..a.byte].is_empty() {
                // the line is blank up to the caret: out by one level
                let ind = &line[..a.byte];
                let w = col_in(ind, ind.len(), self.tab_width as usize);
                let tw = self.tab_width.max(1) as usize;
                let target = if w % tw == 0 { w.saturating_sub(tw) } else { w - w % tw };
                let keep = byte_in(ind, target, self.tab_width as usize);
                let mut rep = String::from(&ind[..keep]);
                rep.push(c);
                self.replace_kind(TextPos::new(a.line, 0), b, &rep, CK_NONE, tr("Schreiben"));
                return self.body.rev != rev;
            }
        }
        self.replace_kind(a, b, s, CK_TYPE, tr("Schreiben"));
        self.body.rev != rev
    }

    /// Enter: a line break with the line's indentation; after an opener of `openers` (the
    /// last non-blank character before the caret) one level more, and when its closer follows
    /// the caret, that goes to a line of its own
    pub fn newline(&mut self, openers: &str) -> bool {
        let (a, b) = self.body.sel.range();
        let rev = self.body.rev;
        let line = self.line(a.line);
        let ind = String::from(self.indentation(a.line).get(..a.byte.min(self.indentation(a.line).len())).unwrap_or(""));
        let before = line[..a.byte].trim_end();
        let after = self.line(b.line)[b.byte..].trim_start();
        let opener = before.chars().last().filter(|c| openers.contains(*c));
        let mut s = String::from("\n");
        s.push_str(&ind);
        let mut caret_off = s.len();
        if let Some(o) = opener {
            s.push_str(&self.indent_unit());
            caret_off = s.len();
            let close = match o {
                '{' => Some('}'),
                '[' => Some(']'),
                '(' => Some(')'),
                _ => None,
            };
            if close.is_some() && after.starts_with(close.unwrap()) {
                s.push('\n');
                s.push_str(&ind);
            }
        }
        // blanks right of the caret go (the new line starts at its indentation)
        let trail = self.line(b.line)[b.byte..].len() - after.len();
        let b2 = TextPos::new(b.line, b.byte + trail);
        let (new, _) = self.spliced(a, b2, &s);
        let caret = TextPos::new(a.line + 1, caret_off - 1);
        self.edit(a.line, b2.line - a.line + 1, new, TextSel::at(caret), tr("Zeilenumbruch"), CK_NONE);
        self.body.rev != rev
    }

    /// Tab: the selected lines one level in (several lines selected), else the indentation
    /// up to the next tab stop at the caret (replacing the selection)
    pub fn tab(&mut self) -> bool {
        let (a, b) = self.body.sel.range();
        if a.line != b.line {
            return self.indent(1);
        }
        let rev = self.body.rev;
        let s = if self.use_tabs {
            String::from("\t")
        } else {
            let col = self.col_of(a.line, a.byte);
            let tw = self.tab_width.clamp(1, 16) as usize;
            " ".repeat(tw - col % tw)
        };
        self.replace_kind(a, b, &s, CK_TYPE, tr("Einzug"));
        self.body.rev != rev
    }

    /// Shift+Tab: the selected lines (or the caret's) one level out
    pub fn back_tab(&mut self) -> bool {
        self.indent(-1)
    }

    /// the selected lines (a selection ending at a line start leaves that line out) one level
    /// in (delta > 0) or out; the selection follows its text
    pub fn indent(&mut self, delta: i32) -> bool {
        let sel = self.body.sel;
        let (a, b) = sel.range();
        let last = if b.line > a.line && b.byte == 0 { b.line - 1 } else { b.line };
        let unit = self.indent_unit();
        let tw = self.tab_width.clamp(1, 16) as usize;
        let mut new = Vec::with_capacity(last - a.line + 1);
        let mut shift: Vec<isize> = Vec::with_capacity(last - a.line + 1);
        for l in a.line..=last {
            let t = &self.body.lines[l];
            if delta > 0 {
                if t.trim().is_empty() && last > a.line {
                    new.push(t.clone());
                    shift.push(0);
                } else {
                    let mut s = unit.clone();
                    s.push_str(t);
                    new.push(s);
                    shift.push(unit.len() as isize);
                }
            } else {
                // remove up to one level: a tab, or blanks up to tab_width
                let n = if t.starts_with('\t') { 1 } else { t.bytes().take(tw).take_while(|&c| c == b' ').count() };
                new.push(String::from(&t[n..]));
                shift.push(-(n as isize));
            }
        }
        // (a selection of whole lines keeps its start at column 0)
        let mv = |p: TextPos| -> TextPos {
            if p.line < a.line || p.line > last {
                return p;
            }
            let d = shift[p.line - a.line];
            if d >= 0 {
                if p.byte == 0 && !sel.is_empty() { p } else { TextPos::new(p.line, p.byte + d as usize) }
            } else {
                TextPos::new(p.line, p.byte.saturating_sub((-d) as usize))
            }
        };
        let to = TextSel::new(mv(sel.anchor), mv(sel.caret));
        self.edit(a.line, last - a.line + 1, new, to, tr("Einzug"), CK_NONE)
    }

    /// line comments on (when not every selected non-blank line has one) or off
    pub fn toggle_comment(&mut self, prefix: &str) -> bool {
        if prefix.is_empty() {
            return false;
        }
        let sel = self.body.sel;
        let (a, b) = sel.range();
        let last = if b.line > a.line && b.byte == 0 { b.line - 1 } else { b.line };
        let lines = &self.body.lines[a.line..=last];
        let all = lines.iter().filter(|t| !t.trim().is_empty()).all(|t| t.trim_start().starts_with(prefix));
        let min_ind = lines.iter().filter(|t| !t.trim().is_empty()).map(|t| t.len() - t.trim_start().len()).min().unwrap_or(0);
        let mut new = Vec::new();
        let mut shift = Vec::new();
        for t in lines {
            if t.trim().is_empty() {
                new.push(t.clone());
                shift.push((0usize, 0isize));
                continue;
            }
            if all {
                let i = t.len() - t.trim_start().len();
                let mut n = prefix.len();
                if t[i + n..].starts_with(' ') {
                    n += 1;
                }
                let mut s = String::from(&t[..i]);
                s.push_str(&t[i + n..]);
                new.push(s);
                shift.push((i, -(n as isize)));
            } else {
                let mut s = String::from(&t[..min_ind]);
                s.push_str(prefix);
                s.push(' ');
                s.push_str(&t[min_ind..]);
                new.push(s);
                shift.push((min_ind, prefix.len() as isize + 1));
            }
        }
        let mv = |p: TextPos| -> TextPos {
            if p.line < a.line || p.line > last {
                return p;
            }
            let (at, d) = shift[p.line - a.line];
            if p.byte < at {
                return p;
            }
            TextPos::new(p.line, if d >= 0 { p.byte + d as usize } else { p.byte.saturating_sub((-d) as usize).max(at) })
        };
        let to = TextSel::new(mv(sel.anchor), mv(sel.caret));
        self.edit(a.line, last - a.line + 1, new, to, tr("Kommentar"), CK_NONE)
    }

    /// Alt+Up / Alt+Down: the selected lines move one line up (delta < 0) or down
    pub fn move_lines(&mut self, delta: i32) -> bool {
        let sel = self.body.sel;
        let (a, b) = sel.range();
        let last = if b.line > a.line && b.byte == 0 { b.line - 1 } else { b.line };
        let n = self.body.lines.len();
        if (delta < 0 && a.line == 0) || (delta > 0 && last + 1 >= n) {
            return false;
        }
        let (first, cnt) = if delta < 0 { (a.line - 1, last - a.line + 2) } else { (a.line, last - a.line + 2) };
        let mut new: Vec<String> = self.body.lines[first..first + cnt].to_vec();
        if delta < 0 {
            new.rotate_left(1);
        } else {
            new.rotate_right(1);
        }
        let d = if delta < 0 { -1isize } else { 1 };
        let mv = |p: TextPos| TextPos::new((p.line as isize + d) as usize, p.byte);
        let to = TextSel::new(mv(sel.anchor), mv(sel.caret));
        self.edit(first, cnt, new, to, tr("Zeilen verschieben"), CK_NONE)
    }

    // ------------------------------------------------------------------ find and replace

    /// every match of q (FIND_CASE, FIND_WORD), at most FIND_MAX
    pub fn find_all(&self, q: &str, opts: u8) -> Vec<(TextPos, TextPos)> {
        let mut out = Vec::new();
        if q.is_empty() || q.contains('\n') {
            return out;
        }
        for (l, t) in self.body.lines.iter().enumerate() {
            find_in(t, q, opts, &mut |a, b| {
                out.push((TextPos::new(l, a), TextPos::new(l, b)));
                out.len() < FIND_MAX
            });
            if out.len() >= FIND_MAX {
                break;
            }
        }
        out
    }

    /// the matches of q in lines first .. first + n
    pub fn find_lines(&self, q: &str, opts: u8, first: usize, n: usize) -> Vec<(TextPos, TextPos)> {
        let mut out = Vec::new();
        if q.is_empty() || q.contains('\n') {
            return out;
        }
        let end = (first + n).min(self.body.lines.len());
        for l in first.min(end)..end {
            find_in(&self.body.lines[l], q, opts, &mut |a, b| {
                out.push((TextPos::new(l, a), TextPos::new(l, b)));
                true
            });
        }
        out
    }

    /// the next match after `from` (backwards: the last one before it), wrapping around
    pub fn find_next(&self, from: TextPos, q: &str, opts: u8, backwards: bool) -> Option<(TextPos, TextPos)> {
        if q.is_empty() || q.contains('\n') {
            return None;
        }
        let n = self.body.lines.len();
        let from = self.clamp(from);
        for k in 0..=n {
            let l = if backwards { (from.line + n * 2 - k) % n } else { (from.line + k) % n };
            let mut hits: Vec<(usize, usize)> = Vec::new();
            find_in(&self.body.lines[l], q, opts, &mut |a, b| {
                hits.push((a, b));
                true
            });
            let pick = if backwards {
                hits.into_iter().rev().find(|h| k > 0 || h.0 < from.byte)
            } else {
                hits.into_iter().find(|h| k > 0 || h.0 >= from.byte)
            };
            if let Some((a, b)) = pick {
                return Some((TextPos::new(l, a), TextPos::new(l, b)));
            }
        }
        None
    }

    /// the selection is a match of q
    pub fn sel_is_match(&self, q: &str, opts: u8) -> bool {
        let (a, b) = self.body.sel.range();
        if a.line != b.line || a == b {
            return false;
        }
        let mut ok = false;
        find_in(&self.body.lines[a.line][..], q, opts, &mut |x, y| {
            if x == a.byte && y == b.byte {
                ok = true;
                return false;
            }
            true
        });
        ok
    }

    /// replace the selection when it is a match, then select the next match -> replaced
    pub fn replace_next(&mut self, q: &str, rep: &str, opts: u8) -> bool {
        let mut did = false;
        if self.sel_is_match(q, opts) {
            let (a, b) = self.body.sel.range();
            self.replace_kind(a, b, rep, CK_NONE, tr("Ersetzen"));
            did = true;
        }
        let from = self.body.sel.end();
        if let Some((a, b)) = self.find_next(from, q, opts, false) {
            self.body.sel = TextSel::new(a, b);
        }
        did
    }

    /// replace every match (one undo step) -> how many
    pub fn replace_all(&mut self, q: &str, rep: &str, opts: u8) -> usize {
        if q.is_empty() || q.contains('\n') {
            return 0;
        }
        let mut first = usize::MAX;
        let mut last = 0;
        let mut count = 0;
        let mut new_lines: Vec<(usize, String)> = Vec::new();
        for (l, t) in self.body.lines.iter().enumerate() {
            let mut hits = Vec::new();
            find_in(t, q, opts, &mut |a, b| {
                hits.push((a, b));
                true
            });
            if hits.is_empty() {
                continue;
            }
            let mut s = String::with_capacity(t.len());
            let mut at = 0;
            for (a, b) in &hits {
                s.push_str(&t[at..*a]);
                s.push_str(rep);
                at = *b;
            }
            s.push_str(&t[at..]);
            count += hits.len();
            first = first.min(l);
            last = l;
            new_lines.push((l, s));
        }
        if count == 0 {
            return 0;
        }
        let mut new: Vec<String> = self.body.lines[first..=last].to_vec();
        for (l, s) in new_lines {
            new[l - first] = s;
        }
        let sel = TextSel::at(self.clamp(self.body.sel.caret));
        let label = tr("Alle ersetzen");
        self.edit(first, last - first + 1, new, sel, label, CK_NONE);
        let c = self.clamp(self.body.sel.caret);
        self.body.sel = TextSel::at(c);
        count
    }
}

/// the matches of q in t: found(a, b) for each, false stops
pub(crate) fn find_in(t: &str, q: &str, opts: u8, found: &mut dyn FnMut(usize, usize) -> bool) {
    if q.is_empty() {
        return;
    }
    let case = opts & FIND_CASE != 0;
    let word = opts & FIND_WORD != 0;
    let qc: Vec<char> = q.chars().map(|c| if case { c } else { fold(c) }).collect();
    let tb = t.as_bytes();
    let first_ascii = q.as_bytes()[0];
    let mut i = 0;
    while i < tb.len() {
        // a quick test of the first byte for ASCII queries
        if first_ascii < 0x80 {
            let b = tb[i];
            let ok = if case { b == first_ascii } else { b.to_ascii_lowercase() == first_ascii.to_ascii_lowercase() };
            if !ok {
                i += 1;
                continue;
            }
        } else if tb[i] & 0xC0 == 0x80 {
            i += 1;
            continue;
        }
        // compare the characters
        let mut j = i;
        let mut ok = true;
        for &c in &qc {
            let Some(tc) = t[j..].chars().next() else {
                ok = false;
                break;
            };
            let tc2 = if case { tc } else { fold(tc) };
            if tc2 != c {
                ok = false;
                break;
            }
            j += tc.len_utf8();
        }
        if ok && word {
            let before = t[..i].chars().next_back().is_some_and(is_word);
            let after = t[j..].chars().next().is_some_and(is_word);
            ok = !before && !after;
        }
        if ok {
            if !found(i, j) {
                return;
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
        while i < tb.len() && tb[i] & 0xC0 == 0x80 {
            i += 1;
        }
    }
}

/// the visual column of byte in t (tab stops every tw columns)
pub(crate) fn col_in(t: &str, byte: usize, tw: usize) -> usize {
    let tw = tw.clamp(1, 16);
    let mut col = 0;
    let mut i = 0;
    let byte = byte.min(t.len());
    while i < byte {
        let n = segment::grapheme_next(t, i);
        let c = t[i..].chars().next().unwrap_or(' ');
        col += if c == '\t' { tw - col % tw } else { char_cols(c) };
        i = n;
    }
    col
}

/// the cluster boundary of t nearest to column col
pub(crate) fn byte_in(t: &str, col: usize, tw: usize) -> usize {
    let tw = tw.clamp(1, 16);
    let mut c0 = 0;
    let mut i = 0;
    while i < t.len() {
        let n = segment::grapheme_next(t, i);
        let c = t[i..].chars().next().unwrap_or(' ');
        let w = if c == '\t' { tw - c0 % tw } else { char_cols(c) };
        if col < c0 + w {
            // nearer to the start or to the end of this cluster
            return if (col - c0) * 2 < w { i } else { n };
        }
        c0 += w;
        i = n;
    }
    t.len()
}
