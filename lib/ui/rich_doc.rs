// rich_doc -- the document of ui.richtext: paragraphs of formatted runs, the selection and the
// undo history (ui::History); JSON, HTML (rich_html.rs) and plain text
//
//   RichDoc::new() / from_plain(s) / from_paras(v) / from_json(s) / from_html(s)
//   doc.to_json() / to_html() / to_plain() (list markers) / text() (paragraphs joined by '\n')
//   doc.paras() / para(i) / selection() / set_selection(sel) / revision()
//   editing at the selection (each one undoable step): insert_text(s), insert_paras(frag)
//   (paste), delete(a, b), split() (Enter), toggle(RF_*), set_color / set_bg / set_size /
//   set_font / set_link / clear_format, set_align(RA_*), toggle_list(LS_*), indent(delta),
//   set_style(PS_*); with an empty selection the character formats apply to the text typed
//   next (current_fmt shows them)
//   doc.undo() / redo() / can_undo() / can_redo() / undo_label() / is_dirty() / mark_saved()
//
//   Fmt { flags RF_BOLD | RF_ITALIC | RF_UNDER | RF_STRIKE, color, bg, size (1/10 pt),
//         font (family), link }      0 / "" = the editor's default
//   Para { align RA_*, list LS_*, level 0..8, style PS_* } + its text and runs
//
// JSON ("jfrich" 1):
//   {"jfrich":1,"font":"","size":11,"paras":[{"align":"center","list":"bullet","level":1,
//    "style":"h1","runs":[{"t":"Hallo ","b":1},{"t":"Welt","i":1,"u":1,"s":1,"color":"#c00000",
//    "bg":"#ffff00","size":14.5,"font":"DejaVu Serif","link":"https://example.org"}],
//    "mark":{"b":1}}]}
// (attributes at their default are left out; "mark" is the format of an empty paragraph).
//
// Positions are TextPos { line: paragraph, byte: UTF-8 offset into the paragraph's text };
// a paragraph holds no '\n'.  An edit replaces a range of whole paragraphs (ParaEdit: old and
// new paragraphs, the selection before and after), so undo restores text, formats and
// paragraph attributes alike; typing and repeated Backspace / Delete merge into one step.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use agent_core::json::{self, Limits, Value};

use crate::ed_core::{TextPos, TextSel};
use crate::history::{Edit, History};
use crate::i18n::tr;

/// Fmt::flags: bold
pub const RF_BOLD: u8 = 1;
/// Fmt::flags: italic
pub const RF_ITALIC: u8 = 2;
/// Fmt::flags: underlined
pub const RF_UNDER: u8 = 4;
/// Fmt::flags: struck through
pub const RF_STRIKE: u8 = 8;
/// every flag
pub const RF_ALL: u8 = 15;

/// Para::align: start (left)
pub const RA_LEFT: u8 = 0;
/// Para::align: centred
pub const RA_CENTER: u8 = 1;
/// Para::align: end (right)
pub const RA_RIGHT: u8 = 2;
/// both edges (the last row of a paragraph stays left)
pub const RA_JUSTIFY: u8 = 3;

/// Para::list: no list
pub const LS_NONE: u8 = 0;
/// a bulleted list item (•, ◦, ▪ by level)
pub const LS_BULLET: u8 = 1;
/// a numbered list item (1., a., i. by level)
pub const LS_NUMBER: u8 = 2;

/// Para::style: body text
pub const PS_NORMAL: u8 = 0;
/// headings (bold, larger)
pub const PS_H1: u8 = 1;
/// Para::style: heading 2
pub const PS_H2: u8 = 2;
/// Para::style: heading 3
pub const PS_H3: u8 = 3;
/// a quotation (indented, a bar on the left, the secondary text colour)
pub const PS_QUOTE: u8 = 4;

/// the deepest Para::level
pub const LEVEL_MAX: u8 = 8;

/// the format of a run of characters; zero / empty fields take the editor's defaults
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fmt {
    /// RF_BOLD | RF_ITALIC | RF_UNDER | RF_STRIKE
    pub flags: u8,
    /// text colour 0xAARRGGBB (0: the theme's text colour)
    pub color: u32,
    /// highlight colour behind the text (0: none)
    pub bg: u32,
    /// size in tenths of a point (0: the document's size)
    pub size: u16,
    /// font family ("": the document's font)
    pub font: String,
    /// link target ("": not a link)
    pub link: String,
}

impl Fmt {
    /// the default format with these flags
    pub fn flags(flags: u8) -> Fmt {
        Fmt { flags, ..Fmt::default() }
    }
    /// bold
    pub fn bold() -> Fmt {
        Fmt::flags(RF_BOLD)
    }
    /// italic
    pub fn italic() -> Fmt {
        Fmt::flags(RF_ITALIC)
    }
    /// a link to url
    pub fn link(url: &str) -> Fmt {
        Fmt { link: String::from(url), ..Fmt::default() }
    }
}

static NEXT_VER: AtomicU32 = AtomicU32::new(1);

fn next_ver() -> u32 {
    NEXT_VER.fetch_add(1, Ordering::Relaxed)
}

/// a paragraph: its text (no '\n') with runs of formats, and its attributes.  The attribute
/// fields may be set directly while building a document (not undoable; RichDoc's setters
/// are).
#[derive(Clone, Debug)]
pub struct Para {
    text: String,
    // (end, format) ascending; the last ends at text.len(); empty for an empty paragraph
    runs: Vec<(usize, Fmt)>,
    /// the format of the paragraph's end: typing into the empty paragraph takes it
    pub mark: Fmt,
    /// RA_*
    pub align: u8,
    /// LS_*
    pub list: u8,
    /// indent / list level 0..LEVEL_MAX
    pub level: u8,
    /// PS_*
    pub style: u8,
    // a new number with every change (layout caches)
    ver: u32,
}

impl PartialEq for Para {
    fn eq(&self, o: &Para) -> bool {
        self.text == o.text
            && self.runs == o.runs
            && self.mark == o.mark
            && self.align == o.align
            && self.list == o.list
            && self.level == o.level
            && self.style == o.style
    }
}

impl Default for Para {
    fn default() -> Para {
        Para::new()
    }
}

impl Para {
    /// an empty body paragraph
    pub fn new() -> Para {
        Para { text: String::new(), runs: Vec::new(), mark: Fmt::default(), align: RA_LEFT, list: LS_NONE, level: 0, style: PS_NORMAL, ver: next_ver() }
    }

    /// a paragraph of unformatted text ('\n' and '\r' are dropped)
    pub fn plain(text: &str) -> Para {
        let mut p = Para::new();
        p.push(text, &Fmt::default());
        p
    }

    /// builder: text in a format appended
    pub fn with(mut self, text: &str, fmt: Fmt) -> Para {
        self.push(text, &fmt);
        self
    }

    /// builder: the attributes
    pub fn attrs(mut self, align: u8, list: u8, level: u8, style: u8) -> Para {
        self.align = align.min(RA_JUSTIFY);
        self.list = list.min(LS_NUMBER);
        self.level = level.min(LEVEL_MAX);
        self.style = style.min(PS_QUOTE);
        self
    }

    /// append text in a format ('\n' and '\r' are dropped)
    pub fn push(&mut self, text: &str, fmt: &Fmt) {
        let clean: String;
        let text = if text.contains(['\n', '\r']) {
            clean = text.chars().filter(|&c| c != '\n' && c != '\r').collect();
            &clean
        } else {
            text
        };
        if text.is_empty() {
            return;
        }
        self.text.push_str(text);
        let end = self.text.len();
        match self.runs.last_mut() {
            Some(r) if r.1 == *fmt => r.0 = end,
            _ => self.runs.push((end, fmt.clone())),
        }
        self.ver = next_ver();
    }

    /// the paragraph's text (no '\n')
    pub fn text(&self) -> &str {
        &self.text
    }

    /// the text's length in bytes
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// no text
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// the runs: (text, format)
    pub fn runs(&self) -> impl Iterator<Item = (&str, &Fmt)> + '_ {
        let mut start = 0;
        self.runs.iter().map(move |(end, f)| {
            let s = &self.text[start..*end];
            start = *end;
            (s, f)
        })
    }

    /// the runs as (start, end, format)
    pub fn spans(&self) -> impl Iterator<Item = (usize, usize, &Fmt)> + '_ {
        let mut start = 0;
        self.runs.iter().map(move |(end, f)| {
            let s = start;
            start = *end;
            (s, *end, f)
        })
    }

    /// changes with every edit of the paragraph (layout caches)
    pub fn version(&self) -> u32 {
        self.ver
    }

    /// the format of the character at byte (the last one at the end; the mark when empty)
    pub fn fmt_at(&self, byte: usize) -> &Fmt {
        for (end, f) in &self.runs {
            if byte < *end {
                return f;
            }
        }
        self.runs.last().map_or(&self.mark, |r| &r.1)
    }

    /// the format typing at byte takes: the character before it (at the start: the first
    /// character; empty: the mark); a link ends at its end
    pub fn typing_fmt(&self, byte: usize) -> Fmt {
        if self.text.is_empty() {
            return self.mark.clone();
        }
        let mut f = if byte == 0 { self.fmt_at(0).clone() } else { self.fmt_at(byte - 1).clone() };
        if !f.link.is_empty() && (byte == 0 || byte >= self.text.len() || self.fmt_at(byte).link != f.link) {
            f.link.clear();
        }
        f
    }

    // the part a..b with this paragraph's attributes (mark: the format at b)
    fn slice(&self, a: usize, b: usize) -> Para {
        let mut p = Para {
            text: String::new(),
            runs: Vec::new(),
            mark: self.mark.clone(),
            align: self.align,
            list: self.list,
            level: self.level,
            style: self.style,
            ver: 0,
        };
        for (s, e, f) in self.spans() {
            let (s, e) = (s.max(a), e.min(b));
            if s < e {
                p.push(&self.text[s..e], f);
            }
        }
        if b < self.text.len() || self.text.is_empty() {
            p.mark = if self.text.is_empty() { self.mark.clone() } else { self.typing_fmt(b) };
        }
        p.ver = next_ver();
        p
    }

    // append another paragraph's text and runs
    fn append(&mut self, o: &Para) {
        for (s, f) in o.runs() {
            self.push(s, f);
        }
    }

    // change the formats of a..b
    fn map_fmt(&mut self, a: usize, b: usize, f: &dyn Fn(&mut Fmt)) {
        if a >= b {
            return;
        }
        let old = core::mem::take(&mut self.runs);
        let text = core::mem::take(&mut self.text);
        let mut start = 0;
        for (end, fmt) in old {
            for (s, e, inside) in [(start, end.min(a).max(start), false), (start.max(a), end.min(b), true), (b.max(start), end, false)] {
                if s < e {
                    let mut g = fmt.clone();
                    if inside {
                        f(&mut g);
                    }
                    self.push(&text[s..e], &g);
                }
            }
            start = end;
        }
        self.ver = next_ver();
    }

    fn touch(&mut self) {
        self.ver = next_ver();
    }

    // the same attributes as o
    fn attrs_of(&mut self, o: &Para) {
        self.align = o.align;
        self.list = o.list;
        self.level = o.level;
        self.style = o.style;
    }
}

// ------------------------------------------------------------------ the undoable state

/// the content an edit changes: the paragraphs and the selection
#[derive(Clone)]
pub(crate) struct RichBody {
    paras: Vec<Para>,
    sel: TextSel,
}

// merge classes of ParaEdit
const K_NONE: u8 = 0;
const K_TYPE: u8 = 1;
const K_BACK: u8 = 2;
const K_DEL: u8 = 3;

// paragraphs first .. first + old.len() become new
struct ParaEdit {
    first: usize,
    old: Vec<Para>,
    new: Vec<Para>,
    sel0: TextSel,
    sel1: TextSel,
    label: String,
    kind: u8,
}

impl Edit<RichBody> for ParaEdit {
    fn apply(&mut self, s: &mut RichBody) {
        s.paras.splice(self.first..self.first + self.old.len(), self.new.iter().cloned());
        s.sel = self.sel1;
    }
    fn revert(&mut self, s: &mut RichBody) {
        s.paras.splice(self.first..self.first + self.new.len(), self.old.iter().cloned());
        s.sel = self.sel0;
    }
    fn label(&self) -> &str {
        &self.label
    }
    fn merge(&mut self, next: &mut dyn Edit<RichBody>) -> bool {
        if self.kind == K_NONE {
            return false;
        }
        match next.value().and_then(|v| v.downcast_ref::<ParaEdit>()) {
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

/// the document of ui.richtext (see the module header)
pub struct RichDoc {
    body: RichBody,
    hist: History<RichBody>,
    rev: u64,
    // the format typed next at this caret (a format chosen without a selection)
    typing: Option<(Fmt, TextPos)>,
    // tells documents apart (editor caches)
    uid: u32,
    /// the document's font family ("": the UI font)
    pub font: String,
    /// the document's size in tenths of a point (0: 11 pt)
    pub size: u16,
}

impl Default for RichDoc {
    fn default() -> RichDoc {
        RichDoc::new()
    }
}

impl Clone for RichDoc {
    /// the content and the selection (not the undo history)
    fn clone(&self) -> RichDoc {
        let mut d = RichDoc::from_paras(self.body.paras.clone());
        d.body.sel = self.body.sel;
        d.font = self.font.clone();
        d.size = self.size;
        d
    }
}

const HIST_LIMIT: usize = 500;

impl RichDoc {
    /// one empty paragraph
    pub fn new() -> RichDoc {
        RichDoc::from_paras(Vec::new())
    }

    /// paragraphs (an empty list: one empty paragraph); the caret at the start
    pub fn from_paras(mut paras: Vec<Para>) -> RichDoc {
        if paras.is_empty() {
            paras.push(Para::new());
        }
        RichDoc { body: RichBody { paras, sel: TextSel::default() }, hist: History::new(HIST_LIMIT), rev: 0, typing: None, uid: next_ver(), font: String::new(), size: 0 }
    }

    /// unformatted text: a paragraph per line
    pub fn from_plain(s: &str) -> RichDoc {
        RichDoc::from_paras(lines_of(s).map(Para::plain).collect())
    }

    /// the content (no history)
    pub fn paras(&self) -> &[Para] {
        &self.body.paras
    }

    /// paragraph i (clamped to the last one)
    pub fn para(&self, i: usize) -> &Para {
        &self.body.paras[i.min(self.body.paras.len() - 1)]
    }

    /// the number of paragraphs (at least 1)
    pub fn para_count(&self) -> usize {
        self.body.paras.len()
    }

    /// counts the changes of the content (edits, undo, redo; not the selection)
    pub fn revision(&self) -> u64 {
        self.rev
    }

    // (rich_edit.rs) a number of this document instance
    pub(crate) fn uid(&self) -> u32 {
        self.uid
    }

    /// the selection (the caret: anchor == caret)
    pub fn selection(&self) -> TextSel {
        self.body.sel
    }

    /// move the selection (clamped to the text, not undoable)
    pub fn set_selection(&mut self, sel: TextSel) {
        let sel = TextSel::new(self.clamp(sel.anchor), self.clamp(sel.caret));
        self.body.sel = sel;
    }

    /// p inside the document, on a character boundary
    pub fn clamp(&self, p: TextPos) -> TextPos {
        let line = p.line.min(self.body.paras.len() - 1);
        let t = &self.body.paras[line].text;
        let mut b = p.byte.min(t.len());
        while !t.is_char_boundary(b) {
            b -= 1;
        }
        TextPos::new(line, b)
    }

    /// the end of the document
    pub fn end(&self) -> TextPos {
        let l = self.body.paras.len() - 1;
        TextPos::new(l, self.body.paras[l].len())
    }

    /// the text: the paragraphs joined by '\n' (no list markers)
    pub fn text(&self) -> String {
        let mut s = String::new();
        for (i, p) in self.body.paras.iter().enumerate() {
            if i > 0 {
                s.push('\n');
            }
            s.push_str(&p.text);
        }
        s
    }

    /// the byte offset of p in text()
    pub fn flat(&self, p: TextPos) -> usize {
        let p = self.clamp(p);
        self.body.paras[..p.line].iter().map(|q| q.text.len() + 1).sum::<usize>() + p.byte
    }

    /// the position of byte offset f of text()
    pub fn unflat(&self, f: usize) -> TextPos {
        let mut f = f;
        for (i, p) in self.body.paras.iter().enumerate() {
            if f <= p.text.len() {
                return self.clamp(TextPos::new(i, f));
            }
            f -= p.text.len() + 1;
        }
        self.end()
    }

    /// the text between a and b (paragraphs joined by '\n')
    pub fn text_range(&self, a: TextPos, b: TextPos) -> String {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        let mut s = String::new();
        for l in a.line..=b.line {
            let t = &self.body.paras[l].text;
            let s0 = if l == a.line { a.byte } else { 0 };
            let s1 = if l == b.line { b.byte } else { t.len() };
            if l > a.line {
                s.push('\n');
            }
            s.push_str(&t[s0..s1]);
        }
        s
    }

    /// the paragraphs between a and b (the first and the last cut there), with attributes
    pub fn paras_range(&self, a: TextPos, b: TextPos) -> Vec<Para> {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        (a.line..=b.line)
            .map(|l| {
                let p = &self.body.paras[l];
                p.slice(if l == a.line { a.byte } else { 0 }, if l == b.line { b.byte } else { p.len() })
            })
            .collect()
    }

    /// the selected text
    pub fn selected_text(&self) -> String {
        let (a, b) = self.body.sel.range();
        self.text_range(a, b)
    }

    /// the selected paragraphs (a fragment for the clipboard)
    pub fn selected_paras(&self) -> Vec<Para> {
        let (a, b) = self.body.sel.range();
        self.paras_range(a, b)
    }

    /// the paragraph with the caret
    pub fn current_para(&self) -> &Para {
        &self.body.paras[self.body.sel.caret.line]
    }

    /// the format typing takes now (a format chosen without a selection, else the text's at
    /// the caret); with a selection: its first character's, the flags all of it has
    pub fn current_fmt(&self) -> Fmt {
        let sel = self.body.sel;
        if sel.is_empty() {
            if let Some((f, at)) = &self.typing {
                if *at == sel.caret {
                    return f.clone();
                }
            }
            return self.body.paras[sel.caret.line].typing_fmt(sel.caret.byte);
        }
        let (a, b) = sel.range();
        let mut first: Option<Fmt> = None;
        let mut flags = RF_ALL;
        for l in a.line..=b.line {
            let p = &self.body.paras[l];
            let (s0, s1) = (if l == a.line { a.byte } else { 0 }, if l == b.line { b.byte } else { p.len() });
            for (s, e, f) in p.spans() {
                if s.max(s0) < e.min(s1) {
                    flags &= f.flags;
                    if first.is_none() {
                        first = Some(f.clone());
                    }
                }
            }
        }
        let mut f = first.unwrap_or_else(|| self.body.paras[a.line].typing_fmt(a.byte));
        f.flags &= flags;
        f
    }

    /// the link target at p (None: no link there)
    pub fn link_at(&self, p: TextPos) -> Option<&str> {
        let p = self.clamp(p);
        let para = &self.body.paras[p.line];
        if p.byte >= para.len() {
            return None;
        }
        let l = &para.fmt_at(p.byte).link;
        (!l.is_empty()).then_some(l.as_str())
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
    /// the label of the step undo takes back ("Fett", "Schreiben" ...)
    pub fn undo_label(&self) -> Option<&str> {
        self.hist.undo_label()
    }
    /// the label of the step redo does again
    pub fn redo_label(&self) -> Option<&str> {
        self.hist.redo_label()
    }
    /// take the last step back -> false: nothing to undo
    pub fn undo(&mut self) -> bool {
        let r = self.hist.undo(&mut self.body);
        if r {
            self.rev += 1;
            self.typing = None;
        }
        r
    }
    /// do the last undone step again -> false: nothing to redo
    pub fn redo(&mut self) -> bool {
        let r = self.hist.redo(&mut self.body);
        if r {
            self.rev += 1;
            self.typing = None;
        }
        r
    }
    /// the content differs from the state of the last mark_saved
    pub fn is_dirty(&self) -> bool {
        self.hist.dirty()
    }
    /// the content is saved now (is_dirty false until the next change)
    pub fn mark_saved(&mut self) {
        self.hist.mark_saved();
    }
    /// forget the undo steps
    pub fn clear_history(&mut self) {
        self.hist.clear();
    }
    /// the next edits are one undo step (until group_end)
    pub fn group_begin(&mut self, label: &str) {
        self.hist.group_begin(label);
    }
    /// close the step of group_begin
    pub fn group_end(&mut self) {
        self.hist.group_end();
    }

    // record paragraphs first .. first + n becoming `new`, the selection then sel
    fn edit(&mut self, first: usize, n: usize, mut new: Vec<Para>, sel: TextSel, label: &str, kind: u8) -> bool {
        if new.is_empty() {
            new.push(Para::new());
        }
        let old: Vec<Para> = self.body.paras[first..first + n].to_vec();
        if old == new {
            self.body.sel = sel;
            return false;
        }
        let e = ParaEdit { first, old, new, sel0: self.body.sel, sel1: sel, label: String::from(label), kind };
        self.hist.apply(&mut self.body, Box::new(e));
        self.rev += 1;
        self.typing = None;
        true
    }

    // the paragraphs that replace a..b by frag: frag[0] continues a's paragraph, the last
    // one takes what follows b.  attrs: frag's paragraphs keep their attributes (a paste of
    // whole paragraphs), else they take the host's.  fill: the mark of empty results (text
    // edits).  -> (new paragraphs, the position after the inserted text)
    fn spliced(&self, a: TextPos, b: TextPos, frag: &[Para], attrs: bool, fill: Option<&Fmt>) -> (Vec<Para>, TextPos) {
        let host = &self.body.paras[a.line];
        let last = &self.body.paras[b.line];
        let k = frag.len().max(1) - 1;
        let empty = Para::new();
        let f = |i: usize| frag.get(i).unwrap_or(&empty);
        let mut out: Vec<Para> = Vec::with_capacity(k + 1);
        let mut p0 = host.slice(0, a.byte);
        p0.append(f(0));
        if attrs && k > 0 && a.byte == 0 {
            p0.attrs_of(f(0));
        }
        p0.mark = if k == 0 { last.mark.clone() } else { f(0).mark.clone() };
        let end;
        if k == 0 {
            end = TextPos::new(a.line, p0.len());
            p0.append(&last.slice(b.byte, last.len()));
            out.push(p0);
        } else {
            out.push(p0);
            for i in 1..k {
                let mut p = f(i).clone();
                if !attrs {
                    p.attrs_of(host);
                }
                p.touch();
                out.push(p);
            }
            let mut pk = f(k).clone();
            if !attrs {
                pk.attrs_of(host);
            }
            end = TextPos::new(a.line + k, pk.len());
            let tail = last.slice(b.byte, last.len());
            pk.append(&tail);
            pk.mark = tail.mark;
            pk.touch();
            out.push(pk);
        }
        if let Some(fm) = fill {
            for p in out.iter_mut().filter(|p| p.is_empty()) {
                p.mark = fm.clone();
            }
        }
        (out, end)
    }

    // ------------------------------------------------------------------ text edits

    /// replace the selection by text (a paragraph per line) in the current format
    pub fn insert_text(&mut self, s: &str) -> bool {
        self.insert_text_kind(s, if s.contains(['\n', '\r']) { K_NONE } else { K_TYPE })
    }

    // (rich_edit.rs) typing merges into one step; a paste does not
    pub(crate) fn insert_text_kind(&mut self, s: &str, kind: u8) -> bool {
        let sel = self.body.sel;
        let fmt = match &self.typing {
            Some((f, at)) if *at == sel.caret => f.clone(),
            _ if sel.is_empty() => self.current_fmt(),
            _ => {
                // typing over a selection takes the format of its first character
                let a = sel.start();
                let p = &self.body.paras[a.line];
                if a.byte < p.len() { p.fmt_at(a.byte).clone() } else { p.typing_fmt(a.byte) }
            }
        };
        let frag: Vec<Para> = lines_of(s)
            .map(|l| {
                let mut p = Para::new();
                p.push(l, &fmt);
                p
            })
            .collect();
        let (a, b) = self.body.sel.range();
        let (new, end) = self.spliced(a, b, &frag, false, Some(&fmt));
        let label = if kind == K_TYPE { tr("Schreiben") } else { tr("Einfügen") };
        self.edit(a.line, b.line - a.line + 1, new, TextSel::at(end), label, kind)
    }

    /// replace the selection by paragraphs (a paste): several keep their attributes, one
    /// is inserted into the paragraph at the caret
    pub fn insert_paras(&mut self, frag: &[Para]) -> bool {
        if frag.is_empty() {
            return self.delete_selection();
        }
        let (a, b) = self.body.sel.range();
        let (new, end) = self.spliced(a, b, frag, frag.len() > 1, None);
        self.edit(a.line, b.line - a.line + 1, new, TextSel::at(end), tr("Einfügen"), K_NONE)
    }

    /// remove the selection
    pub fn delete_selection(&mut self) -> bool {
        let (a, b) = self.body.sel.range();
        self.delete(a, b)
    }

    /// remove a..b (the caret goes to a)
    pub fn delete(&mut self, a: TextPos, b: TextPos) -> bool {
        self.delete_kind(a, b, K_NONE)
    }

    // (rich_edit.rs) Backspace / Delete merge with the previous one
    pub(crate) fn delete_kind(&mut self, a: TextPos, b: TextPos, kind: u8) -> bool {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        if a == b {
            return false;
        }
        let fill = self.body.paras[a.line].typing_fmt(a.byte);
        let (new, end) = self.spliced(a, b, &[], false, Some(&fill));
        // joining into an empty paragraph keeps the attributes of the one with text
        let mut new = new;
        if a.byte == 0 && self.body.paras[a.line].is_empty() && b.line > a.line {
            let src = &self.body.paras[b.line];
            new[0].attrs_of(src);
        }
        self.edit(a.line, b.line - a.line + 1, new, TextSel::at(end), tr("Löschen"), kind)
    }

    /// Enter: split the paragraph at the caret (the selection is replaced).  In an empty
    /// list item the list ends instead (a nested one moves out a level); after a heading the
    /// new paragraph is body text.
    pub fn split(&mut self) -> bool {
        let (a, b) = self.body.sel.range();
        let host = &self.body.paras[a.line];
        if a == b && host.is_empty() && host.list != LS_NONE {
            let mut p = host.clone();
            if p.level > 0 {
                p.level -= 1;
            } else {
                p.list = LS_NONE;
            }
            p.touch();
            return self.edit(a.line, 1, alloc::vec![p], self.body.sel, tr("Liste"), K_NONE);
        }
        let fmt = self.current_fmt();
        let mut e1 = Para::new();
        e1.mark = fmt.clone();
        let mut e2 = Para::new();
        e2.mark = fmt.clone();
        let at_end = b.byte >= self.body.paras[b.line].len();
        let heading = host.style != PS_NORMAL && host.style != PS_QUOTE;
        let (mut new, end) = self.spliced(a, b, &[e1, e2], false, Some(&fmt));
        if heading && at_end {
            if let Some(p) = new.last_mut() {
                p.style = PS_NORMAL;
            }
        }
        self.edit(a.line, b.line - a.line + 1, new, TextSel::at(end), tr("Absatz"), K_NONE)
    }

    // the range a format applies to: the selection, or (link edits) the link at the caret
    fn fmt_range(&self, link_word: bool) -> Option<(TextPos, TextPos)> {
        let sel = self.body.sel;
        if !sel.is_empty() {
            return Some(sel.range());
        }
        if link_word {
            let p = self.body.paras.get(sel.caret.line)?;
            let at = sel.caret.byte;
            let link = if at < p.len() && !p.fmt_at(at).link.is_empty() {
                p.fmt_at(at).link.clone()
            } else if at > 0 && !p.fmt_at(at - 1).link.is_empty() {
                p.fmt_at(at - 1).link.clone()
            } else {
                return None;
            };
            // the run of characters with that link around the caret
            let spans: Vec<(usize, usize, bool)> = p.spans().map(|(s, e, f)| (s, e, f.link == link)).collect();
            let i = spans.iter().position(|&(s, e, m)| m && s <= at && (at < e || (at == e && at > 0)))?;
            let mut s = spans[i].0;
            let mut e = spans[i].1;
            let mut j = i;
            while j > 0 && spans[j - 1].2 {
                j -= 1;
                s = spans[j].0;
            }
            let mut j = i;
            while j + 1 < spans.len() && spans[j + 1].2 {
                j += 1;
                e = spans[j].1;
            }
            return Some((TextPos::new(sel.caret.line, s), TextPos::new(sel.caret.line, e)));
        }
        None
    }

    // change the character formats of the selection (without one: the format typed next)
    fn map_sel(&mut self, label: &str, link_word: bool, f: &dyn Fn(&mut Fmt)) -> bool {
        let Some((a, b)) = self.fmt_range(link_word) else {
            let mut t = self.current_fmt();
            f(&mut t);
            let caret = self.body.sel.caret;
            // an empty paragraph keeps the format as its mark
            if self.body.paras[caret.line].is_empty() {
                let mut p = self.body.paras[caret.line].clone();
                p.mark = t.clone();
                p.touch();
                let sel = self.body.sel;
                self.edit(caret.line, 1, alloc::vec![p], sel, label, K_NONE);
            }
            self.typing = Some((t, caret));
            return true;
        };
        let new: Vec<Para> = (a.line..=b.line)
            .map(|l| {
                let mut p = self.body.paras[l].clone();
                let s0 = if l == a.line { a.byte } else { 0 };
                let s1 = if l == b.line { b.byte } else { p.len() };
                p.map_fmt(s0, s1, f);
                if s1 >= p.len() || p.is_empty() {
                    f(&mut p.mark);
                }
                p
            })
            .collect();
        let sel = self.body.sel;
        self.edit(a.line, b.line - a.line + 1, new, sel, label, K_NONE)
    }

    /// RF_BOLD / RF_ITALIC / RF_UNDER / RF_STRIKE on (when not all of the selection has it)
    /// or off
    pub fn toggle(&mut self, flag: u8) -> bool {
        let on = self.current_fmt().flags & flag != flag;
        let label = match flag {
            RF_BOLD => tr("Fett"),
            RF_ITALIC => tr("Kursiv"),
            RF_UNDER => tr("Unterstrichen"),
            _ => tr("Durchgestrichen"),
        };
        self.map_sel(label, false, &|f: &mut Fmt| if on { f.flags |= flag } else { f.flags &= !flag })
    }

    /// set (on) or clear flags
    pub fn set_flags(&mut self, flags: u8, on: bool) -> bool {
        self.map_sel(tr("Format"), false, &|f: &mut Fmt| if on { f.flags |= flags } else { f.flags &= !flags })
    }

    /// the text colour (0: the theme's)
    pub fn set_color(&mut self, argb: u32) -> bool {
        self.map_sel(tr("Textfarbe"), false, &|f: &mut Fmt| f.color = argb)
    }

    /// the highlight colour (0: none)
    pub fn set_bg(&mut self, argb: u32) -> bool {
        self.map_sel(tr("Hervorhebung"), false, &|f: &mut Fmt| f.bg = argb)
    }

    /// the size in tenths of a point (0: the document's)
    pub fn set_size(&mut self, tenths: u16) -> bool {
        let t = if tenths == 0 { 0 } else { tenths.clamp(10, 4000) };
        self.map_sel(tr("Schriftgröße"), false, &|f: &mut Fmt| f.size = t)
    }

    /// the font family ("": the document's)
    pub fn set_font(&mut self, family: &str) -> bool {
        let fam = String::from(family);
        self.map_sel(tr("Schriftart"), false, &|f: &mut Fmt| f.font = fam.clone())
    }

    /// a link on the selection (without one: on the link at the caret); "" removes it.  A
    /// collapsed caret outside a link inserts the url as linked text.
    pub fn set_link(&mut self, url: &str) -> bool {
        let u = String::from(url);
        if self.body.sel.is_empty() && self.fmt_range(true).is_none() {
            if url.is_empty() {
                return false;
            }
            let mut f = self.current_fmt();
            f.link = u.clone();
            self.typing = Some((f, self.body.sel.caret));
            return self.insert_text_kind(url, K_NONE);
        }
        self.map_sel(tr("Link"), true, &|f: &mut Fmt| f.link = u.clone())
    }

    /// back to the default format (links stay)
    pub fn clear_format(&mut self) -> bool {
        self.map_sel(tr("Format löschen"), false, &|f: &mut Fmt| *f = Fmt { link: core::mem::take(&mut f.link), ..Fmt::default() })
    }

    // ------------------------------------------------------------------ paragraph edits

    // change the attributes of the selected paragraphs
    fn map_paras(&mut self, label: &str, f: &dyn Fn(&mut Para)) -> bool {
        let (a, b) = self.body.sel.range();
        let new: Vec<Para> = (a.line..=b.line)
            .map(|l| {
                let mut p = self.body.paras[l].clone();
                f(&mut p);
                p.touch();
                p
            })
            .collect();
        let sel = self.body.sel;
        self.edit(a.line, b.line - a.line + 1, new, sel, label, K_NONE)
    }

    /// RA_* for the selected paragraphs
    pub fn set_align(&mut self, align: u8) -> bool {
        let a = align.min(RA_JUSTIFY);
        self.map_paras(tr("Ausrichtung"), &|p: &mut Para| p.align = a)
    }

    /// LS_BULLET / LS_NUMBER: the selected paragraphs become such list items, or plain
    /// paragraphs when all of them already are
    pub fn toggle_list(&mut self, kind: u8) -> bool {
        let (a, b) = self.body.sel.range();
        let all = (a.line..=b.line).all(|l| self.body.paras[l].list == kind);
        let k = if all { LS_NONE } else { kind.min(LS_NUMBER) };
        self.map_paras(tr("Liste"), &|p: &mut Para| p.list = k)
    }

    /// LS_* without toggling
    pub fn set_list(&mut self, kind: u8) -> bool {
        let k = kind.min(LS_NUMBER);
        self.map_paras(tr("Liste"), &|p: &mut Para| p.list = k)
    }

    /// one level deeper (delta 1) or out (-1), 0..LEVEL_MAX
    pub fn indent(&mut self, delta: i32) -> bool {
        self.map_paras(tr("Einzug"), &|p: &mut Para| p.level = (p.level as i32).saturating_add(delta).clamp(0, LEVEL_MAX as i32) as u8)
    }

    /// PS_* for the selected paragraphs
    pub fn set_style(&mut self, style: u8) -> bool {
        let s = style.min(PS_QUOTE);
        self.map_paras(tr("Formatvorlage"), &|p: &mut Para| p.style = s)
    }

    /// replace the whole content (one undo step)
    pub fn set_paras(&mut self, paras: Vec<Para>) -> bool {
        let n = self.body.paras.len();
        self.edit(0, n, paras, TextSel::default(), tr("Ersetzen"), K_NONE)
    }

    // ------------------------------------------------------------------ JSON

    /// the document as JSON (see the module header)
    pub fn to_json(&self) -> String {
        let mut paras = Vec::new();
        for p in &self.body.paras {
            let mut o = Value::Object(Vec::new());
            if p.align != RA_LEFT {
                o.insert("align", Value::string(["left", "center", "right", "justify"][p.align as usize & 3]));
            }
            if p.list != LS_NONE {
                o.insert("list", Value::string(if p.list == LS_BULLET { "bullet" } else { "number" }));
            }
            if p.level != 0 {
                o.insert("level", Value::u64(p.level as u64));
            }
            if p.style != PS_NORMAL {
                o.insert("style", Value::string(STYLE_NAMES[p.style as usize % STYLE_NAMES.len()]));
            }
            let runs: Vec<Value> = p
                .runs()
                .map(|(t, f)| {
                    let mut r = fmt_json(f);
                    if let Value::Object(v) = &mut r {
                        v.insert(0, (String::from("t"), Value::string(t)));
                    }
                    r
                })
                .collect();
            o.insert("runs", Value::Array(runs));
            if p.mark != Fmt::default() && p.is_empty() {
                o.insert("mark", fmt_json(&p.mark));
            }
            paras.push(o);
        }
        let mut root = Value::object([("jfrich", Value::u64(1))]);
        if !self.font.is_empty() {
            root.insert("font", Value::string(self.font.clone()));
        }
        if self.size != 0 {
            root.insert("size", size_json(self.size));
        }
        root.insert("paras", Value::Array(paras));
        json::stringify_with_limits(&root, big_limits()).unwrap_or_default()
    }

    /// a document from to_json's JSON (None: not such a document)
    pub fn from_json(s: &str) -> Option<RichDoc> {
        let v = json::parse_with_limits(s.as_bytes(), big_limits()).ok()?;
        v.get("jfrich")?.as_u64().filter(|&n| n == 1)?;
        let mut paras = Vec::new();
        for pv in v.get("paras")?.as_array()? {
            let mut p = Para::new();
            let s = |k: &str| pv.get(k).and_then(|x| x.as_str());
            p.align = match s("align") {
                Some("center") => RA_CENTER,
                Some("right") => RA_RIGHT,
                Some("justify") => RA_JUSTIFY,
                _ => RA_LEFT,
            };
            p.list = match s("list") {
                Some("bullet") => LS_BULLET,
                Some("number") => LS_NUMBER,
                _ => LS_NONE,
            };
            p.level = pv.get("level").and_then(|x| x.as_u64()).unwrap_or(0).min(LEVEL_MAX as u64) as u8;
            p.style = s("style").and_then(|n| STYLE_NAMES.iter().position(|&m| m == n)).unwrap_or(0) as u8;
            if let Some(runs) = pv.get("runs").and_then(|x| x.as_array()) {
                for r in runs {
                    let t = r.get("t").and_then(|x| x.as_str()).unwrap_or("");
                    p.push(t, &fmt_of_json(r));
                }
            }
            if let Some(m) = pv.get("mark") {
                p.mark = fmt_of_json(m);
            }
            paras.push(p);
        }
        let mut d = RichDoc::from_paras(paras);
        d.font = v.get("font").and_then(|x| x.as_str()).unwrap_or("").into();
        d.size = v.get("size").and_then(size_of_json).unwrap_or(0);
        Some(d)
    }

    // ------------------------------------------------------------------ plain text

    /// the text with list markers ("• ", "1. ") and two blanks of indent per level
    pub fn to_plain(&self) -> String {
        let marks = list_marks(&self.body.paras);
        let mut s = String::new();
        for (i, p) in self.body.paras.iter().enumerate() {
            if i > 0 {
                s.push('\n');
            }
            for _ in 0..p.level {
                s.push_str("  ");
            }
            if let Some(m) = &marks[i] {
                s.push_str(m);
                s.push(' ');
            }
            s.push_str(&p.text);
        }
        s
    }
}

/// the list marker of every paragraph ("•", "◦", "▪", "1.", "a.", "i." by level; None: no
/// list item).  Numbers count the items of a level; a paragraph outside a list or an item
/// of another kind starts again.
pub fn list_marks(paras: &[Para]) -> Vec<Option<String>> {
    let mut count = [0u32; LEVEL_MAX as usize + 1];
    let mut kind = [LS_NONE; LEVEL_MAX as usize + 1];
    paras
        .iter()
        .map(|p| {
            if p.list == LS_NONE {
                count = [0; LEVEL_MAX as usize + 1];
                kind = [LS_NONE; LEVEL_MAX as usize + 1];
                return None;
            }
            let l = (p.level as usize).min(LEVEL_MAX as usize);
            for d in l + 1..=LEVEL_MAX as usize {
                count[d] = 0;
                kind[d] = LS_NONE;
            }
            if kind[l] != p.list {
                count[l] = 0;
                kind[l] = p.list;
            }
            count[l] += 1;
            Some(if p.list == LS_BULLET {
                String::from(["•", "◦", "▪"][l % 3])
            } else {
                match l % 3 {
                    0 => format!("{}.", count[l]),
                    1 => format!("{}.", alpha(count[l])),
                    _ => format!("{}.", roman(count[l])),
                }
            })
        })
        .collect()
}

// a, b, .. z, aa, ab ..
fn alpha(n: u32) -> String {
    let mut n = n.max(1);
    let mut v = Vec::new();
    while n > 0 {
        n -= 1;
        v.push(b'a' + (n % 26) as u8);
        n /= 26;
    }
    v.reverse();
    String::from_utf8(v).unwrap_or_default()
}

// i, ii, iii, iv ..
fn roman(n: u32) -> String {
    const T: [(u32, &str); 13] =
        [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
    let mut n = n.clamp(1, 3999);
    let mut s = String::new();
    for (v, t) in T {
        while n >= v {
            s.push_str(t);
            n -= v;
        }
    }
    s
}

const STYLE_NAMES: [&str; 5] = ["normal", "h1", "h2", "h3", "quote"];

// the lines of a text (\r\n, \r and \n end a line)
pub(crate) fn lines_of(s: &str) -> impl Iterator<Item = &str> {
    s.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).flat_map(|l| l.split('\r'))
}

fn big_limits() -> Limits {
    Limits { bytes: 64 << 20, depth: 8, values: 4 << 20, string_bytes: 16 << 20, members: 1 << 20 }
}

/// #rrggbb (#rrggbbaa when not opaque)
pub(crate) fn color_hex(c: u32) -> String {
    let a = c >> 24;
    if a == 0xFF || a == 0 {
        format!("#{:06x}", c & 0xFF_FFFF)
    } else {
        format!("#{:06x}{:02x}", c & 0xFF_FFFF, a)
    }
}

/// #rgb, #rrggbb, #rrggbbaa -> 0xAARRGGBB
pub(crate) fn parse_hex_color(s: &str) -> Option<u32> {
    let h = s.trim().strip_prefix('#')?;
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    match h.len() {
        3 => Some(0xFF00_0000 | (v >> 8 & 15) * 0x11_0000 | (v >> 4 & 15) * 0x1100 | (v & 15) * 0x11),
        6 => Some(0xFF00_0000 | v),
        8 => Some((v & 0xFF) << 24 | v >> 8),
        _ => None,
    }
}

// tenths of a point as JSON: 11, 14.5
fn size_json(t: u16) -> Value {
    match json::Number::parse(&size_text(t)) {
        Some(n) => Value::Number(n),
        None => Value::u64((t / 10) as u64),
    }
}

/// tenths of a point as text: "11", "14.5"
pub(crate) fn size_text(t: u16) -> String {
    if t % 10 == 0 { format!("{}", t / 10) } else { format!("{}.{}", t / 10, t % 10) }
}

/// "14.5" -> 145 (tenths)
pub(crate) fn parse_tenths(s: &str) -> Option<u16> {
    let s = s.trim();
    let (i, f) = s.split_once('.').unwrap_or((s, ""));
    let i: u32 = i.parse().ok()?;
    let f: u32 = f.chars().next().map_or(Some(0), |c| c.to_digit(10))?;
    Some((i.min(u16::MAX as u32) * 10 + f).min(u16::MAX as u32) as u16)
}

fn size_of_json(v: &Value) -> Option<u16> {
    match v {
        Value::Number(n) => parse_tenths(n.as_str()),
        _ => None,
    }
}

fn fmt_json(f: &Fmt) -> Value {
    let mut o = Value::Object(Vec::new());
    for (bit, k) in [(RF_BOLD, "b"), (RF_ITALIC, "i"), (RF_UNDER, "u"), (RF_STRIKE, "s")] {
        if f.flags & bit != 0 {
            o.insert(k, Value::u64(1));
        }
    }
    if f.color != 0 {
        o.insert("color", Value::string(color_hex(f.color)));
    }
    if f.bg != 0 {
        o.insert("bg", Value::string(color_hex(f.bg)));
    }
    if f.size != 0 {
        o.insert("size", size_json(f.size));
    }
    if !f.font.is_empty() {
        o.insert("font", Value::string(f.font.clone()));
    }
    if !f.link.is_empty() {
        o.insert("link", Value::string(f.link.clone()));
    }
    o
}

fn fmt_of_json(v: &Value) -> Fmt {
    let mut f = Fmt::default();
    for (bit, k) in [(RF_BOLD, "b"), (RF_ITALIC, "i"), (RF_UNDER, "u"), (RF_STRIKE, "s")] {
        let on = match v.get(k) {
            Some(Value::Bool(b)) => *b,
            Some(x) => x.as_u64().is_some_and(|n| n != 0),
            None => false,
        };
        if on {
            f.flags |= bit;
        }
    }
    let s = |k: &str| v.get(k).and_then(|x| x.as_str());
    f.color = s("color").and_then(parse_hex_color).unwrap_or(0);
    f.bg = s("bg").and_then(parse_hex_color).unwrap_or(0);
    f.size = v.get("size").and_then(size_of_json).unwrap_or(0);
    f.font = s("font").unwrap_or("").into();
    f.link = s("link").unwrap_or("").into();
    f
}

// (rich_edit.rs) the merge classes
pub(crate) const RK_TYPE: u8 = K_TYPE;
pub(crate) const RK_BACK: u8 = K_BACK;
pub(crate) const RK_DEL: u8 = K_DEL;
