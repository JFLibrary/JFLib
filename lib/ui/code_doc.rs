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
//   diagnostics (a checker's or compiler's messages, drawn by the editor):
//     set_diagnostics(&[Diag]) / clear_diagnostics() / diagnostics() -> Vec<Diag> /
//     diagnostic(i) / diagnostic_count() / diagnostic_counts() -> [errors, warnings, infos] /
//     diagnostics_at(pos) (most severe first) / next_diagnostic(pos, back) (wrapping)
//   breakpoints and a debugger's current line (the gutter column of CE_BREAKPOINTS; lines
//     from 0): set_breakpoints(&[(line, BP_ON | BP_DISABLED | BP_COND)]) / breakpoints() /
//     breakpoint(line) / set_exec_line(Some(line)) / exec_line()
//   folding (ranges from a language's services; lines from 0): set_fold_ranges(&[(first,
//     last)]) / fold_ranges() / fold_at(line) / set_folded(line, on) / toggle_fold(line) /
//     fold_all(on) / fold_around(line) / is_hidden(line) / reveal(line) / visible_lines() /
//     row_of_line(line) / line_of_row(row) / visible_line(line, down)
//   several changes as one undo step: apply_edits(&[TextEdit], label) (a rename's edits),
//     replace_text(text, label) (a formatter's result: only the lines that differ change, the
//     caret keeps its place among the non-blank characters)
//
// An edit replaces a range of whole lines (LineEdit: old and new lines, the selection before
// and after); each change is also kept in a short journal (changes_since) that the editor's
// highlighting states and the find results follow without starting again from the top.
//
// Diagnostics follow every change of the text (edits, undo, redo) until the app sets new
// ones: when lines change, the old and new lines are compared -- the same number of lines
// line by line (one line moved by Alt+Up / Down: as a move), else as one replaced part from
// the first to the last difference.  Text before a change keeps its positions, text after it
// shifts; a diagnostic whose text was replaced shrinks to what is left of it and is dropped
// when nothing is (one that marks a point stays at the change).  Typed text at a diagnostic's
// edge stays outside it.  So squiggles stay under their words while the user types, until
// the next check replaces them.  Breakpoints, the execution line and folding ranges follow
// the same way by their lines (a line counts as the point at its start); an edit inside the
// hidden lines of a folded range unfolds it.  None of them is part of the undo history.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;

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

/// Diag::severity: an error
pub const DIAG_ERROR: u8 = 0;
/// a warning
pub const DIAG_WARNING: u8 = 1;
/// a note, a hint (higher values count as this too)
pub const DIAG_INFO: u8 = 2;

/// a diagnostic of a code editor: a part of the text in lines and columns from 1 as compilers
/// report them (columns count characters, a tab is one; the end is exclusive, an end at or
/// before the start marks the one character at the start, past a line's end the end of the
/// line), a severity DIAG_* and the message (see the module header)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diag {
    /// where it starts (from 1)
    pub line: usize,
    pub col: usize,
    /// where it ends (exclusive)
    pub end_line: usize,
    pub end_col: usize,
    /// DIAG_ERROR, DIAG_WARNING, DIAG_INFO
    pub severity: u8,
    pub msg: String,
}

impl Diag {
    /// a part of one line: line:col .. line:end_col
    pub fn new(line: usize, col: usize, end_col: usize, severity: u8, msg: &str) -> Diag {
        Diag { line, col, end_line: line, end_col, severity, msg: String::from(msg) }
    }

    /// line:col .. end_line:end_col
    pub fn span(line: usize, col: usize, end_line: usize, end_col: usize, severity: u8, msg: &str) -> Diag {
        Diag { line, col, end_line, end_col, severity, msg: String::from(msg) }
    }
}

// a diagnostic in the document: a .. b (a == b: the character at a), severity, message
#[derive(Clone, Debug)]
pub(crate) struct Mark {
    pub(crate) a: TextPos,
    pub(crate) b: TextPos,
    pub(crate) sev: u8,
    pub(crate) msg: String,
}

/// a breakpoint's look (CodeDoc::set_breakpoints): set
pub const BP_ON: u8 = 0;
/// a breakpoint that is there but off (a ring)
pub const BP_DISABLED: u8 = 1;
/// a breakpoint with a condition (a bar across it)
pub const BP_COND: u8 = 2;

/// a change of a CodeDoc's text: a .. b becomes text (CodeDoc::apply_edits)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextEdit {
    pub a: TextPos,
    pub b: TextPos,
    pub text: String,
}

impl TextEdit {
    pub fn new(a: TextPos, b: TextPos, text: &str) -> TextEdit {
        TextEdit { a, b, text: String::from(text) }
    }
}

// a folding range: line `first` stays, first + 1 ..= last hide while folded
#[derive(Clone, Copy, Debug, PartialEq)]
struct FoldRange {
    first: usize,
    last: usize,
    folded: bool,
}

/// the content an edit changes: the lines, the selection, the change journal and the
/// diagnostics, breakpoints, execution line and folding ranges that follow the changes
pub(crate) struct CodeBody {
    lines: Vec<String>,
    sel: TextSel,
    rev: u64,
    journal: Vec<LineChange>,
    // sorted by position; the most lines one of them spans; how many per severity
    marks: Vec<Mark>,
    span: usize,
    counts: [usize; 3],
    // (line, BP_*) by line; the line of the current execution
    bps: Vec<(usize, u8)>,
    exec: Option<usize>,
    // by first line; the hidden lines of the folded ones (merged, inclusive), how many
    folds: Vec<FoldRange>,
    hidden: Vec<(usize, usize)>,
    hidden_n: usize,
}

impl CodeBody {
    fn splice(&mut self, first: usize, old: usize, new: &[String]) {
        if !self.marks.is_empty() || !self.bps.is_empty() || self.exec.is_some() || !self.folds.is_empty() {
            self.follow(first, old, new);
        }
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

    // the diagnostics follow lines first .. first + old becoming `new` (the lines still hold
    // the old text; see the module header)
    fn follow(&mut self, first: usize, old: usize, new: &[String]) {
        let o = &self.lines[first..first + old];
        let c = o.len();
        let f = if o.is_empty() || new.is_empty() {
            None
        } else if c == new.len() && c > 1 && new[..c - 1] == o[1..] && new[c - 1] == o[0] {
            Some(Follow::Turn(false))
        } else if c == new.len() && c > 1 && new[1..] == o[..c - 1] && new[0] == o[c - 1] {
            Some(Follow::Turn(true))
        } else if c == new.len() {
            let each = o.iter().zip(new).map(|(a, b)| (a != b).then(|| moved(core::slice::from_ref(a), core::slice::from_ref(b))));
            Some(Follow::Lines(each.collect()))
        } else {
            Some(Follow::One(moved(o, new)))
        };
        let d = new.len() as isize - c as isize;
        let marks = core::mem::take(&mut self.marks);
        self.marks = marks
            .into_iter()
            .filter_map(|mut m| {
                let point = m.a == m.b;
                match &f {
                    Some(f) => {
                        m.a = f.pos(first, c, m.a, if point { AT_POINT } else { AT_START });
                        m.b = if point { m.a } else { f.pos(first, c, m.b, AT_END) };
                    }
                    None => {
                        // (no lines on one side) the block's diagnostics go, later ones move
                        if [m.a, m.b].iter().any(|p| p.line >= first && p.line < first + c) {
                            return None;
                        }
                        for p in [&mut m.a, &mut m.b] {
                            if p.line >= first + c {
                                p.line = (p.line as isize + d) as usize;
                            }
                        }
                    }
                }
                (point || m.a < m.b).then_some(m)
            })
            .collect();
        self.marks.sort_by(|x, y| (x.a, x.b).cmp(&(y.a, y.b)));
        self.recount();
        // breakpoints, the execution line and the folds by their lines (a line as the point at
        // its start: Enter at a line's start takes it down with its text)
        let line_of = |l: usize| -> Option<usize> {
            match &f {
                Some(f) => Some(f.pos(first, c, TextPos::new(l, 0), AT_POINT).line),
                None if l >= first + c => Some((l as isize + d) as usize),
                None if l >= first => None,
                None => Some(l),
            }
        };
        if !self.bps.is_empty() {
            let mut v: Vec<(usize, u8)> = self.bps.iter().filter_map(|&(l, k)| line_of(l).map(|l| (l, k))).collect();
            v.sort_by_key(|x| x.0);
            v.dedup_by_key(|x| x.0);
            self.bps = v;
        }
        self.exec = self.exec.and_then(line_of);
        if !self.folds.is_empty() {
            // an edit in the hidden lines of a fold opens it
            let folds = core::mem::take(&mut self.folds);
            self.folds = folds
                .into_iter()
                .filter_map(|r| {
                    let inside = first + c > r.first + 1 && first <= r.last;
                    let (a, z) = (line_of(r.first)?, line_of(r.last)?);
                    (z > a).then_some(FoldRange { first: a, last: z, folded: r.folded && !inside })
                })
                .collect();
            self.folds.sort_by_key(|r| r.first);
            self.folds.dedup_by_key(|r| r.first);
            self.rehide();
        }
    }

    // the hidden lines after the folds changed
    fn rehide(&mut self) {
        self.hidden.clear();
        for r in self.folds.iter().filter(|r| r.folded) {
            let (a, z) = (r.first + 1, r.last.min(self.lines.len().saturating_sub(1)));
            if z < a {
                continue;
            }
            match self.hidden.last_mut() {
                Some(h) if a <= h.1 + 1 => h.1 = h.1.max(z),
                _ => self.hidden.push((a, z)),
            }
        }
        self.hidden_n = self.hidden.iter().map(|h| h.1 - h.0 + 1).sum();
    }

    // the counts and the longest span after the diagnostics changed
    fn recount(&mut self) {
        self.counts = [0; 3];
        self.span = 0;
        for m in &self.marks {
            self.counts[m.sev.min(DIAG_INFO) as usize] += 1;
            self.span = self.span.max(m.b.line - m.a.line);
        }
    }

    // the marks that may touch lines first .. last: those starting from first - span on
    fn mark_range(&self, first: usize, last: usize) -> Range<usize> {
        let lo = self.marks.partition_point(|m| m.a.line + self.span < first);
        let hi = self.marks.partition_point(|m| m.a.line < last);
        lo..hi.max(lo)
    }
}

// what a position of a diagnostic is when it follows a change: its start, its end, a point
const AT_START: u8 = 0;
const AT_END: u8 = 1;
const AT_POINT: u8 = 2;

// a change of a block of lines as the positions see it (block-relative: line 0 is the first
// changed line)
enum Follow {
    // one replaced part
    One(Moved),
    // as many lines as before, each changed by itself (None: unchanged)
    Lines(Vec<Option<Moved>>),
    // the lines turned by one (Alt+Up / Down): the last to the front (true), the first to the end
    Turn(bool),
}

impl Follow {
    // where position q (absolute) goes; c: the lines of the block before
    fn pos(&self, first: usize, c: usize, q: TextPos, at: u8) -> TextPos {
        if q.line < first {
            return q;
        }
        let k = q.line - first;
        match self {
            Follow::One(m) => {
                let r = m.at(TextPos::new(k, q.byte), at);
                TextPos::new(r.line + first, r.byte)
            }
            Follow::Lines(v) => match v.get(k) {
                Some(Some(m)) => TextPos::new(q.line, m.at(TextPos::new(0, q.byte), at).byte),
                _ => q,
            },
            Follow::Turn(down) if k < c => TextPos::new(first + if *down { (k + 1) % c } else { (k + c - 1) % c }, q.byte),
            Follow::Turn(_) => q,
        }
    }
}

// one replaced part: old [p, os) became new [p, ns) (block-relative)
#[derive(Clone, Copy)]
struct Moved {
    p: TextPos,
    os: TextPos,
    ns: TextPos,
}

impl Moved {
    // before the part a position stays, after it it shifts; inside it a start goes behind the
    // new text, an end and a point to its front
    fn at(&self, q: TextPos, at: u8) -> TextPos {
        match at {
            AT_END if q <= self.p => q,
            AT_END if q <= self.os => self.p,
            _ if q < self.p => q,
            AT_START if q < self.os => self.ns,
            AT_POINT if q < self.os => self.p,
            _ => self.shift(q),
        }
    }

    // q at or after the old part's end
    fn shift(&self, q: TextPos) -> TextPos {
        if q.line == self.os.line {
            TextPos::new(self.ns.line, self.ns.byte + (q.byte - self.os.byte))
        } else {
            TextPos::new(q.line - self.os.line + self.ns.line, q.byte)
        }
    }
}

// lines o became lines n: one replaced part from the first difference to the last (the common
// end does not reach into the common start)
fn moved(o: &[String], n: &[String]) -> Moved {
    let m = o.len().min(n.len());
    let mut k = 0;
    while k < m && o[k] == n[k] {
        k += 1;
    }
    let p = if k < m {
        TextPos::new(k, common_prefix(&o[k], &n[k]))
    } else if k > 0 {
        TextPos::new(k - 1, o[k - 1].len())
    } else {
        TextPos::new(0, 0)
    };
    let (mut ol, mut nl) = (o.len() - 1, n.len() - 1);
    while ol > p.line && nl > p.line && o[ol] == n[nl] {
        ol -= 1;
        nl -= 1;
    }
    let lo = if ol == p.line { p.byte } else { 0 };
    let ln = if nl == p.line { p.byte } else { 0 };
    let s = common_suffix(&o[ol], &n[nl], (o[ol].len() - lo).min(n[nl].len() - ln));
    Moved { p, os: TextPos::new(ol, o[ol].len() - s), ns: TextPos::new(nl, n[nl].len() - s) }
}

// the bytes a and b start with alike (up to a character boundary)
fn common_prefix(a: &str, b: &str) -> usize {
    let mut k = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    while !(a.is_char_boundary(k) && b.is_char_boundary(k)) {
        k -= 1;
    }
    k
}

// the bytes a and b end with alike, at most max (from a character boundary)
fn common_suffix(a: &str, b: &str, max: usize) -> usize {
    let mut k = a.bytes().rev().zip(b.bytes().rev()).take(max).take_while(|(x, y)| x == y).count();
    while !(a.is_char_boundary(a.len() - k) && b.is_char_boundary(b.len() - k)) {
        k -= 1;
    }
    k
}

// the byte of character column c (from 0) of t; past its end the end
fn byte_of_char(t: &str, c: usize) -> usize {
    t.char_indices().nth(c).map_or(t.len(), |(i, _)| i)
}

// p as a position of lines: on a line there is, at most at its end and at a character boundary
fn pos_in(lines: &[String], p: TextPos) -> TextPos {
    let Some(last) = lines.len().checked_sub(1) else { return TextPos::new(0, 0) };
    let line = p.line.min(last);
    let t = &lines[line];
    let mut b = p.byte.min(t.len());
    while !t.is_char_boundary(b) {
        b -= 1;
    }
    TextPos::new(line, b)
}

// a selection of lines (pos_in for both ends)
fn sel_in(lines: &[String], s: TextSel) -> TextSel {
    TextSel::new(pos_in(lines, s.anchor), pos_in(lines, s.caret))
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
    // (the selection a step leaves is one of the text it leaves: a caller that measured it in
    // the text before the step would otherwise hand redo a position past a line's end or
    // inside a character, and the next edit slices there)
    fn apply(&mut self, s: &mut CodeBody) {
        s.splice(self.first, self.old.len(), &self.new);
        self.sel1 = sel_in(&s.lines, self.sel1);
        s.sel = self.sel1;
    }
    fn revert(&mut self, s: &mut CodeBody) {
        s.splice(self.first, self.new.len(), &self.old);
        self.sel0 = sel_in(&s.lines, self.sel0);
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
        let body = CodeBody {
            lines,
            sel: TextSel::default(),
            rev: 0,
            journal: Vec::new(),
            marks: Vec::new(),
            span: 0,
            counts: [0; 3],
            bps: Vec::new(),
            exec: None,
            folds: Vec::new(),
            hidden: Vec::new(),
            hidden_n: 0,
        };
        CodeDoc { body, hist: History::new(1000), uid, tab_width: 4, use_tabs: false }
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
            self.body.sel = sel_in(&self.body.lines, sel);
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
        // the indentation the lines share: blanks and tabs only (one byte each, so the cut below
        // is at a character boundary of every line; a no-break space is 2 bytes, an ideographic
        // space 3, and the smallest count of another line can end inside one)
        let ind = |t: &str| t.bytes().take_while(|&c| c == b' ' || c == b'\t').count();
        let min_ind = lines.iter().filter(|t| !t.trim().is_empty()).map(|t| ind(t)).min().unwrap_or(0);
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
        // (a selection of whole lines ends at the start of the line below them; moved down to
        // the end of the text there is no such line: it ends at the end of the last one)
        let mv = |p: TextPos| {
            let l = (p.line as isize + d) as usize;
            if l >= n { TextPos::new(n - 1, new[cnt - 1].len()) } else { TextPos::new(l, p.byte) }
        };
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

    // ------------------------------------------------------------------ diagnostics

    /// replace the diagnostics (Diag positions, clamped to the text); from now on they follow
    /// the edits until the next call (see the module header)
    pub fn set_diagnostics(&mut self, d: &[Diag]) {
        let lines = &self.body.lines;
        let pos = |line: usize, col: usize| -> TextPos {
            match lines.get(line.max(1) - 1) {
                Some(t) => TextPos::new(line.max(1) - 1, byte_of_char(t, col.max(1) - 1)),
                None => TextPos::new(lines.len() - 1, lines[lines.len() - 1].len()),
            }
        };
        let mut v: Vec<Mark> = d
            .iter()
            .map(|x| {
                let a = pos(x.line, x.col);
                Mark { a, b: pos(x.end_line, x.end_col).max(a), sev: x.severity.min(DIAG_INFO), msg: x.msg.clone() }
            })
            .collect();
        v.sort_by(|x, y| (x.a, x.b).cmp(&(y.a, y.b)));
        self.body.marks = v;
        self.body.recount();
    }

    /// no diagnostics
    pub fn clear_diagnostics(&mut self) {
        self.body.marks.clear();
        self.body.recount();
    }

    /// the diagnostics where they are now, by position
    pub fn diagnostics(&self) -> Vec<Diag> {
        self.body.marks.iter().map(|m| self.diag_of(m)).collect()
    }

    /// the number of diagnostics
    pub fn diagnostic_count(&self) -> usize {
        self.body.marks.len()
    }

    /// diagnostic i (0 .. diagnostic_count(), by position) where it is now
    pub fn diagnostic(&self, i: usize) -> Option<Diag> {
        self.body.marks.get(i).map(|m| self.diag_of(m))
    }

    /// the number of errors, warnings and infos (DIAG_* order)
    pub fn diagnostic_counts(&self) -> [usize; 3] {
        self.body.counts
    }

    /// the diagnostics at p (their part holds p, its end included; a point: its character),
    /// most severe first
    pub fn diagnostics_at(&self, p: TextPos) -> Vec<Diag> {
        self.marks_at(p).into_iter().map(|i| self.diag_of(&self.body.marks[i])).collect()
    }

    /// the start of the first diagnostic after p (back: the last one before p), round the end
    /// of the text; None without diagnostics
    pub fn next_diagnostic(&self, p: TextPos, back: bool) -> Option<TextPos> {
        let m = &self.body.marks;
        let last = m.last()?;
        Some(if back {
            let k = m.partition_point(|x| x.a < p);
            if k == 0 { last.a } else { m[k - 1].a }
        } else {
            m.get(m.partition_point(|x| x.a <= p)).map_or(m[0].a, |x| x.a)
        })
    }

    // ------------------------------------------------------------------ breakpoints, execution, folds

    /// the breakpoints: (line from 0, BP_*), drawn in the gutter column of CE_BREAKPOINTS (the
    /// app keeps the real ones: a click there is CodeEv::Breakpoint); they follow the edits
    /// with their lines until the next call
    pub fn set_breakpoints(&mut self, bps: &[(usize, u8)]) {
        let n = self.body.lines.len();
        let mut v: Vec<(usize, u8)> = bps.iter().copied().filter(|b| b.0 < n).collect();
        v.sort_by_key(|b| b.0);
        v.dedup_by_key(|b| b.0);
        self.body.bps = v;
    }

    /// the breakpoints where they are now, by line
    pub fn breakpoints(&self) -> &[(usize, u8)] {
        &self.body.bps
    }

    /// the breakpoint of a line
    pub fn breakpoint(&self, line: usize) -> Option<u8> {
        self.body.bps.binary_search_by_key(&line, |b| b.0).ok().map(|i| self.body.bps[i].1)
    }

    /// the line a debugger stopped at (None: none), drawn marked with an arrow in the
    /// breakpoints' column; it follows the edits
    pub fn set_exec_line(&mut self, line: Option<usize>) {
        self.body.exec = line.filter(|&l| l < self.body.lines.len());
    }

    /// the line of set_exec_line where it is now
    pub fn exec_line(&self) -> Option<usize> {
        self.body.exec
    }

    /// the folding ranges (first, last) with lines from 0 (a language's services give them):
    /// line first stays, first + 1 ..= last fold away.  A range that starts on the line of a
    /// folded one stays folded; ranges follow the edits, and an edit inside the hidden lines
    /// of a folded range unfolds it
    pub fn set_fold_ranges(&mut self, ranges: &[(usize, usize)]) {
        let n = self.body.lines.len();
        let old = core::mem::take(&mut self.body.folds);
        let mut v: Vec<FoldRange> = ranges
            .iter()
            .filter(|r| r.0 < r.1 && r.0 < n)
            .map(|&(a, z)| FoldRange { first: a, last: z.min(n - 1), folded: old.iter().any(|o| o.first == a && o.folded) })
            .filter(|r| r.last > r.first)
            .collect();
        v.sort_by_key(|r| r.first);
        v.dedup_by_key(|r| r.first);
        self.body.folds = v;
        self.body.rehide();
    }

    /// the number of folding ranges
    pub fn fold_count(&self) -> usize {
        self.body.folds.len()
    }

    /// the folding ranges where they are now: (first, last, folded)
    pub fn fold_ranges(&self) -> Vec<(usize, usize, bool)> {
        self.body.folds.iter().map(|r| (r.first, r.last, r.folded)).collect()
    }

    /// the range that starts at line: (last, folded)
    pub fn fold_at(&self, line: usize) -> Option<(usize, bool)> {
        let f = &self.body.folds;
        f.binary_search_by_key(&line, |r| r.first).ok().map(|i| (f[i].last, f[i].folded))
    }

    /// fold (folded) or unfold the range that starts at line -> changed
    pub fn set_folded(&mut self, line: usize, folded: bool) -> bool {
        let Ok(i) = self.body.folds.binary_search_by_key(&line, |r| r.first) else { return false };
        if self.body.folds[i].folded == folded {
            return false;
        }
        self.body.folds[i].folded = folded;
        self.body.rehide();
        true
    }

    /// fold or unfold the range that starts at line -> changed
    pub fn toggle_fold(&mut self, line: usize) -> bool {
        match self.fold_at(line) {
            Some((_, f)) => self.set_folded(line, !f),
            None => false,
        }
    }

    /// every range folded (true) or unfolded
    pub fn fold_all(&mut self, folded: bool) {
        for r in self.body.folds.iter_mut() {
            r.folded = folded;
        }
        self.body.rehide();
    }

    /// the innermost range that holds line (its first line counts): (first, last)
    pub fn fold_around(&self, line: usize) -> Option<(usize, usize)> {
        self.body.folds.iter().filter(|r| r.first <= line && line <= r.last).max_by_key(|r| r.first).map(|r| (r.first, r.last))
    }

    /// line is hidden in a folded range
    pub fn is_hidden(&self, line: usize) -> bool {
        let h = &self.body.hidden;
        let k = h.partition_point(|x| x.1 < line);
        h.get(k).is_some_and(|x| x.0 <= line)
    }

    /// unfold every range that hides line -> changed
    pub fn reveal(&mut self, line: usize) -> bool {
        if !self.is_hidden(line) {
            return false;
        }
        for r in self.body.folds.iter_mut() {
            if r.folded && r.first < line && line <= r.last {
                r.folded = false;
            }
        }
        self.body.rehide();
        true
    }

    /// the lines that are not hidden
    pub fn visible_lines(&self) -> usize {
        self.body.lines.len() - self.body.hidden_n
    }

    /// the visible row of a line (counting only lines that are not hidden; a hidden line:
    /// the row of its fold's first line)
    pub fn row_of_line(&self, line: usize) -> usize {
        let mut r = line;
        for &(a, z) in &self.body.hidden {
            if z < line {
                r -= z - a + 1;
            } else {
                if a <= line {
                    r -= line - a + 1;
                }
                break;
            }
        }
        r
    }

    /// the line of a visible row
    pub fn line_of_row(&self, row: usize) -> usize {
        let mut l = row;
        for &(a, z) in &self.body.hidden {
            if a <= l {
                l += z - a + 1;
            } else {
                break;
            }
        }
        l.min(self.body.lines.len() - 1)
    }

    /// the nearest line that is not hidden from line on, downwards (down) or upwards
    pub fn visible_line(&self, line: usize, down: bool) -> usize {
        let h = &self.body.hidden;
        let k = h.partition_point(|x| x.1 < line);
        match h.get(k) {
            Some(&(a, z)) if a <= line => {
                if down && z + 1 < self.body.lines.len() {
                    z + 1
                } else {
                    a - 1
                }
            }
            _ => line,
        }
    }

    // (code_edit.rs) the hidden lines: merged ranges (first, last), by line
    pub(crate) fn hidden(&self) -> &[(usize, usize)] {
        &self.body.hidden
    }

    // ------------------------------------------------------------------ several edits as one step

    /// several changes as one undo step named `label`: the edits (positions in the text before
    /// any of them, in any order, not overlapping) replace their parts; the selection follows
    /// the text (inside a replaced part: to its new end) -> false when nothing changed or two
    /// edits overlap.  Rename and format of a language service go through here
    pub fn apply_edits(&mut self, edits: &[TextEdit], label: &str) -> bool {
        let mut ed: Vec<TextEdit> = edits
            .iter()
            .map(|e| TextEdit { a: self.clamp(e.a.min(e.b)), b: self.clamp(e.a.max(e.b)), text: e.text.replace("\r\n", "\n").replace('\r', "\n") })
            .collect();
        ed.sort_by(|x, y| (x.a, x.b).cmp(&(y.a, y.b)));
        if ed.is_empty() || ed.windows(2).any(|w| w[1].a < w[0].b) {
            return false;
        }
        let first = ed[0].a.line;
        let last = ed.iter().map(|e| e.b.line).max().unwrap_or(first);
        let block = self.text_range(TextPos::new(first, 0), TextPos::new(last, self.body.lines[last].len()));
        let base = self.flat(TextPos::new(first, 0));
        let mut out = String::with_capacity(block.len() + 16);
        let mut at = 0;
        // (old start, old end, new end) of each edit, offsets in the block
        let mut map: Vec<(usize, usize, usize)> = Vec::with_capacity(ed.len());
        for e in &ed {
            let (a, b) = (self.flat(e.a) - base, self.flat(e.b) - base);
            out.push_str(&block[at..a]);
            out.push_str(&e.text);
            map.push((a, b, out.len()));
            at = b;
        }
        out.push_str(&block[at..]);
        let new: Vec<String> = out.split('\n').map(String::from).collect();
        let dl = new.len() as isize - (last - first + 1) as isize;
        let mv = |p: TextPos| -> TextPos {
            if p.line < first {
                return p;
            }
            if p.line > last {
                return TextPos::new((p.line as isize + dl) as usize, p.byte);
            }
            let o = self.flat(p) - base;
            let mut n = o;
            for &(a, b, ne) in &map {
                if o >= b {
                    n = ne + (o - b);
                } else {
                    if o > a {
                        n = ne;
                    }
                    break;
                }
            }
            let line = out[..n].matches('\n').count();
            let ls = out[..n].rfind('\n').map_or(0, |k| k + 1);
            TextPos::new(first + line, n - ls)
        };
        let s = self.body.sel;
        let sel = TextSel::new(mv(s.anchor), mv(s.caret));
        self.edit(first, last - first + 1, new, sel, label, CK_NONE)
    }

    /// the whole text becomes `text` as one undo step; only the lines that differ change (the
    /// diagnostics, breakpoints and folds of the others stay), and the caret keeps its place
    /// among the characters that are no blanks (so after a formatter it stays at its token)
    /// -> changed
    pub fn replace_text(&mut self, text: &str, label: &str) -> bool {
        let new: Vec<String> = lines_of(text).map(String::from).collect();
        let old = &self.body.lines;
        let pre = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        if pre == old.len() && pre == new.len() {
            return false;
        }
        let room = old.len().min(new.len()) - pre;
        let post = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
        let n_old = old.len() - pre - post;
        let mid: Vec<String> = new[pre..new.len() - post].to_vec();
        let mv = |p: TextPos| -> TextPos {
            if p.line < pre {
                return p;
            }
            if p.line >= pre + n_old {
                return TextPos::new(p.line - n_old + mid.len(), p.byte);
            }
            // the same count of ink before it (no blanks, no `;`): right after the k-th one when
            // it was right after one, else at the next one
            let ink = |c: char| !c.is_whitespace() && c != ';';
            let line = &old[p.line][..p.byte.min(old[p.line].len())];
            let k: usize = old[pre..p.line].iter().map(|l| l.chars().filter(|&c| ink(c)).count()).sum::<usize>() + line.chars().filter(|&c| ink(c)).count();
            let after = line.chars().next_back().is_some_and(ink);
            let want = if after { k - 1 } else { k };
            let mut seen = 0;
            let mut end = TextPos::new(pre, 0);
            for (i, l) in mid.iter().enumerate() {
                for (b, c) in l.char_indices() {
                    if !ink(c) {
                        continue;
                    }
                    if seen == want {
                        return TextPos::new(pre + i, if after { b + c.len_utf8() } else { b });
                    }
                    seen += 1;
                    end = TextPos::new(pre + i, b + c.len_utf8());
                }
            }
            end
        };
        let s = self.body.sel;
        let clamp = |p: TextPos| -> TextPos {
            let line = p.line.min(new.len() - 1);
            let t = &new[line];
            let mut b = p.byte.min(t.len());
            while !t.is_char_boundary(b) {
                b -= 1;
            }
            TextPos::new(line, b)
        };
        let sel = TextSel::new(clamp(mv(s.anchor)), clamp(mv(s.caret)));
        self.edit(pre, n_old, mid, sel, label, CK_NONE)
    }

    // (code_edit.rs) the diagnostics by position
    pub(crate) fn marks(&self) -> &[Mark] {
        &self.body.marks
    }

    // (code_edit.rs) the indexes of the diagnostics that may touch lines first .. last
    pub(crate) fn mark_range(&self, first: usize, last: usize) -> Range<usize> {
        self.body.mark_range(first, last)
    }

    // (code_edit.rs) the indexes of the diagnostics at p, most severe first
    pub(crate) fn marks_at(&self, p: TextPos) -> Vec<usize> {
        let m = &self.body.marks;
        let mut v: Vec<usize> = self.body.mark_range(p.line, p.line + 1).filter(|&i| m[i].a <= p && p <= m[i].b).collect();
        v.sort_by_key(|&i| m[i].sev);
        v
    }

    // a mark as the app sees it
    pub(crate) fn diag_of(&self, m: &Mark) -> Diag {
        let col = |p: TextPos| {
            let p = self.clamp(p);
            self.body.lines[p.line][..p.byte].chars().count() + 1
        };
        Diag { line: m.a.line + 1, col: col(m.a), end_line: m.b.line + 1, end_col: col(m.b), severity: m.sev, msg: m.msg.clone() }
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
    col_from(t, 0, 0, byte, tw)
}

/// the cluster boundary of t nearest to column col
pub(crate) fn byte_in(t: &str, col: usize, tw: usize) -> usize {
    byte_from(t, 0, 0, col, tw)
}

// plain ASCII from i on (before lim): one cluster and one column per byte; the last one before
// a non-ASCII character is left out (a mark may join it)
pub(crate) fn ascii_run(b: &[u8], i: usize, lim: usize) -> usize {
    let mut j = i;
    while j < lim && (0x20..0x7F).contains(&b[j]) {
        j += 1;
    }
    if j > i && b.get(j).is_some_and(|&c| c >= 0x80) {
        j -= 1;
    }
    j - i
}

// the cluster of t at boundary i, at column col: (its end, its columns)
pub(crate) fn cell(t: &str, i: usize, col: usize, tw: usize) -> (usize, usize) {
    let n = segment::cluster_end(t, i);
    let c = t[i..].chars().next().unwrap_or(' ');
    (n, if c == '\t' { tw - col % tw } else { char_cols(c) })
}

/// the column of byte in t, counted from boundary i at column col (a checkpoint of a long line)
pub(crate) fn col_from(t: &str, i: usize, col: usize, byte: usize, tw: usize) -> usize {
    let tw = tw.clamp(1, 16);
    let (b, byte) = (t.as_bytes(), byte.min(t.len()));
    let (mut i, mut col) = (i, col);
    while i < byte {
        let k = ascii_run(b, i, byte);
        if k > 0 {
            (i, col) = (i + k, col + k);
            continue;
        }
        let (n, w) = cell(t, i, col, tw);
        (i, col) = (n, col + w);
    }
    col
}

/// the cluster boundary nearest to column col, counted from boundary i at column c0 <= col
pub(crate) fn byte_from(t: &str, i: usize, c0: usize, col: usize, tw: usize) -> usize {
    let (i, c0, n, w) = cell_at(t, i, c0, col, tw);
    // nearer to the start or to the end of this cluster
    if i < t.len() && (col - c0) * 2 >= w { n } else { i }
}

/// the cluster boundary at or before column col and its column, counted from boundary i at
/// column c0 <= col (where drawing a row from column col starts)
pub(crate) fn cell_floor(t: &str, i: usize, c0: usize, col: usize, tw: usize) -> (usize, usize) {
    let (i, c0, _, _) = cell_at(t, i, c0, col, tw);
    (i, c0)
}

// the cluster that holds column col, counted from boundary i at column c0 <= col -> (its start,
// its column, its end, its columns); past the end of t: (t.len(), the end's column, t.len(), 0)
fn cell_at(t: &str, i: usize, c0: usize, col: usize, tw: usize) -> (usize, usize, usize, usize) {
    let tw = tw.clamp(1, 16);
    let b = t.as_bytes();
    let (mut i, mut c0) = (i, c0);
    while i < t.len() {
        // (ASCII up to the column at most)
        let k = ascii_run(b, i, t.len().min(i + (col - c0) + 1));
        if k > 0 {
            if col < c0 + k {
                let j = i + (col - c0);
                return (j, col, j + 1, 1);
            }
            (i, c0) = (i + k, c0 + k);
            continue;
        }
        let (n, w) = cell(t, i, c0, tw);
        if col < c0 + w {
            return (i, c0, n, w);
        }
        (i, c0) = (n, c0 + w);
    }
    (t.len(), c0, t.len(), 0)
}

