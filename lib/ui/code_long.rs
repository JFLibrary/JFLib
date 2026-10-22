// code_long -- what ui.code_editor (code_edit.rs) keeps of the long lines it shows: checkpoints
// of their columns and their tokens, so that a pass looks at the part of a line in view and an
// edit works again only near it
//
//   Longs::col / byte / floor     columns (code_doc.rs: col_from, byte_from, cell_floor) from
//                                 the checkpoint before the place
//   Longs::tokens                 the tokens of a long line (highlight.rs) and its end state
//
// A line of COL_LONG bytes or more gets checkpoints (byte, column) at cluster boundaries about
// every COL_STEP bytes and keeps its tokens with places outside every construct about every
// TOK_STEP bytes.  The entries follow the document's change journal (lines that moved keep
// theirs); a line changed in place keeps its text of before, and the next look at it compares:
// the columns are walked again from the checkpoint before the change to where they meet the
// old ones again (moved along after that, a tab evens out a shift of the columns), the tokens
// are scanned again from a place TOK_REACH bytes before the change (rules look ahead) to the
// first place after it where the scan is where the old scan was.  A construct that looks
// further ahead than TOK_REACH (Markdown emphasis waiting for its closer) is seen again when the
// whole line is scanned: a change of its start state, another grammar.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::code_doc::*;
use crate::highlight::*;

/// a line this long (bytes) gets checkpoints of its columns and keeps its tokens
pub(crate) const COL_LONG: usize = 4096;
// the bytes between two column checkpoints, between two token places
const COL_STEP: usize = 1024;
const TOK_STEP: usize = 1024;
// how far before a change a scan of the tokens starts again
const TOK_REACH: usize = 4096;
// long lines kept (more than a pass looks at), the bytes they take at most
const LONG_KEEP: usize = 256;
const LONG_BYTES: usize = 256 << 20;

// the checkpoints of a long line: (byte, column) at cluster boundaries about every COL_STEP
// bytes, (0, 0) first
fn col_marks(t: &str, tw: usize) -> Vec<(usize, usize)> {
    let mut out = alloc::vec![(0, 0)];
    col_walk(t, 0, 0, tw, &mut out, &mut |_, _| None);
    out
}

// walk t's clusters from boundary i at column col on, a checkpoint into out about every
// COL_STEP bytes; meet(a, b) sees the boundaries a..b of each step (an ASCII run: every byte, a
// cluster: a alone, b = a + 1), Some(d) stops at boundary a + d -> (where it stopped, the
// column there)
fn col_walk(t: &str, i: usize, col: usize, tw: usize, out: &mut Vec<(usize, usize)>, meet: &mut dyn FnMut(usize, usize) -> Option<usize>) -> Option<(usize, usize)> {
    let tw = tw.clamp(1, 16);
    let b = t.as_bytes();
    let (mut i, mut col) = (i, col);
    let mut next = out.last().map_or(0, |m| m.0) + COL_STEP;
    while i < t.len() {
        let k = ascii_run(b, i, t.len());
        let (n, w) = if k > 0 { (i + k, k) } else { cell(t, i, col, tw) };
        if let Some(d) = meet(i, if k > 0 { n } else { i + 1 }) {
            return Some((i + d, col + d));
        }
        // (every byte inside an ASCII run is a boundary)
        while k > 0 && next < n {
            out.push((next, col + (next - i)));
            next += COL_STEP;
        }
        (i, col) = (n, col + w);
        if i >= next && i < t.len() {
            out.push((i, col));
            next = i + COL_STEP;
        }
    }
    None
}

// the checkpoints of line `new` after a change of `old` (marks: its checkpoints; p bytes at the
// start and s at the end alike): kept before the change, walked again to the first old one past
// it the walk meets, the rest moved along -- by the shift of the columns there, after the next
// tab by the shift that tab leaves (a multiple of the tab width, the same to the end)
fn col_marks_edit(old: &str, marks: &[(usize, usize)], new: &str, p: usize, s: usize, tw: usize) -> Vec<(usize, usize)> {
    let tw = tw.clamp(1, 16);
    let delta = new.len() as isize - old.len() as isize;
    // (a character or cluster at the change may join what is around it)
    let k = marks.partition_point(|m| m.0 + 16 < p).max(1);
    let mut out = marks[..k].to_vec();
    let (b0, c0) = out[k - 1];
    // (the old checkpoints past the change, where they are in new)
    let from = new.len() - s + 16;
    let at = |j: usize| (marks[j].0 as isize + delta) as usize;
    let mut j = marks.partition_point(|m| (m.0 as isize + delta) < from as isize);
    let met = col_walk(new, b0, c0, tw, &mut out, &mut |a, b| {
        while j < marks.len() && at(j) < a.max(from) {
            j += 1;
        }
        (j < marks.len() && at(j) < b).then(|| at(j) - a)
    });
    let Some((y, cy)) = met else { return out };
    out.retain(|m| m.0 < y);
    let d = cy as isize - marks[j].1 as isize;
    // the first tab from there: after it the shift it leaves
    let tab = new[y..].find('\t').map(|t| y + t);
    let d1 = tab.map_or(d, |t| {
        let a = col_from(old, marks[j].0, marks[j].1, (t as isize - delta) as usize, tw) as isize;
        let (wn, wo) = (tw as isize - (a + d).rem_euclid(tw as isize), tw as isize - a.rem_euclid(tw as isize));
        d + wn - wo
    });
    for m in &marks[j..] {
        let x = (m.0 as isize + delta) as usize;
        let sh = if tab.is_some_and(|t| x > t) { d1 } else { d };
        out.push((x, (m.1 as isize + sh) as usize));
    }
    out
}

// the tokens of a long line: from start state st with highlighter h; the state at its end;
// places outside every construct about every TOK_STEP bytes (byte, the tokens before it, the
// end of the last of them then: a later token of its kind may have joined it)
struct Toks {
    h: Highlighter,
    st: u32,
    end: u32,
    toks: Rc<Vec<Token>>,
    marks: Vec<(usize, usize, usize)>,
}

impl Toks {
    // scan a whole line
    fn scan(h: Highlighter, t: &str, st: u32) -> Toks {
        let mut toks = Vec::new();
        let mut marks = alloc::vec![(0, 0, 0)];
        let (_, end) = h.line_part(t, st, 0, &mut toks, &mut |i, out| {
            if i >= marks.last().map_or(0, |m| m.0) + TOK_STEP {
                marks.push((i, out.len(), out.last().map_or(0, |x| x.end)));
            }
            false
        });
        Toks { h, st, end: end.unwrap_or(0), toks: Rc::new(toks), marks }
    }

    // the line changed from old to new (p bytes at the start and s at the end alike): scan
    // again from a place TOK_REACH before the change to where the scan meets the old one
    fn edit(&mut self, old: &str, new: &str, p: usize, s: usize) {
        let delta = new.len() as isize - old.len() as isize;
        let k = self.marks.partition_point(|m| m.0 + TOK_REACH <= p).max(1) - 1;
        let (r, rk, rend) = self.marks[k];
        let toks = Rc::make_mut(&mut self.toks);
        // the token before r as it was there (a token from r on may join it again)
        let head = rk.saturating_sub(1);
        let mut mid: Vec<Token> = toks[head..rk].iter().map(|x| Token { end: rend, ..*x }).collect();
        let base = head;
        // the scans may meet past the change (the character before the place too) and past the
        // first non-blank of both lines (rules at the line start)
        let bol = |t: &str| t.len() - t.trim_start_matches([' ', '\t']).len();
        let from = (new.len() - s + 4).max(bol(new) + 1).max((bol(old) as isize + delta + 1).max(0) as usize);
        let om = &self.marks;
        let mut j = om.partition_point(|m| (m.0 as isize + delta) < from as isize);
        let mut marks: Vec<(usize, usize, usize)> = self.marks[..=k].to_vec();
        let mut met = None;
        let (_, end) = self.h.line_part(new, self.st, r, &mut mid, &mut |i, out| {
            if i >= from {
                while j < om.len() && (om[j].0 as isize + delta) < i as isize {
                    j += 1;
                }
                if let Some(&(_, ok, oend)) = om.get(j).filter(|m| (m.0 as isize + delta) as usize == i) {
                    // the old scan was here too, with the same token before it: the same from here
                    let same = match (out.last(), ok.checked_sub(1).map(|q| toks[q])) {
                        (Some(a), Some(b)) => a.kind == b.kind && a.end as isize == oend as isize + delta,
                        (None, None) => true,
                        _ => false,
                    };
                    if same {
                        met = Some(j);
                        return true;
                    }
                }
            }
            if i > r && i >= marks.last().map_or(0, |m| m.0) + TOK_STEP {
                marks.push((i, base + out.len(), out.last().map_or(0, |x| x.end)));
            }
            false
        });
        match met {
            Some(j) => {
                let (_, ok, _) = self.marks[j];
                // the token before the meeting place grew as the old one did
                if let (Some(a), Some(q)) = (mid.last_mut(), ok.checked_sub(1)) {
                    a.end = (toks[q].end as isize + delta) as usize;
                }
                let nk = base + mid.len();
                toks.splice(head..ok, mid);
                for x in toks[nk..].iter_mut() {
                    x.start = (x.start as isize + delta) as usize;
                    x.end = (x.end as isize + delta) as usize;
                }
                let sh = nk as isize - ok as isize;
                for m in &self.marks[j..] {
                    // (no token before it: the end stays 0)
                    let e = if m.1 > 0 { (m.2 as isize + delta) as usize } else { 0 };
                    marks.push(((m.0 as isize + delta) as usize, (m.1 as isize + sh) as usize, e));
                }
            }
            None => {
                toks.truncate(head);
                toks.extend(mid);
                self.end = end.unwrap_or(0);
            }
        }
        self.marks = marks;
    }
}

// what is kept of one long line; text: the line these are of (checked: it is the line's at the
// cache's revision)
struct LongLine {
    line: usize,
    text: String,
    checked: bool,
    marks: Option<Vec<(usize, usize)>>,
    toks: Option<Toks>,
}

impl LongLine {
    // the bytes it takes (about)
    fn size(&self) -> usize {
        let t = self.toks.as_ref().map_or(0, |k| k.toks.len() * core::mem::size_of::<Token>() + k.marks.len() * 24);
        self.text.len() + self.marks.as_ref().map_or(0, |m| m.len() * 16) + t
    }
}

#[derive(Default)]
struct Long {
    uid: u32,
    rev: u64,
    tw: usize,
    lines: Vec<LongLine>,
}

/// the long lines of a code editor (see the module header)
#[derive(Default)]
pub(crate) struct Longs(RefCell<Long>);

// the bytes a and b have alike at the start, and then at the end (not into the start)
fn alike(a: &str, b: &str) -> (usize, usize) {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    let m = x.len().min(y.len());
    // (blocks first: slices compare fast)
    let mut p = 0;
    while p + 64 <= m && x[p..p + 64] == y[p..p + 64] {
        p += 64;
    }
    while p < m && x[p] == y[p] {
        p += 1;
    }
    let mut s = 0;
    while s + 64 <= m - p && x[x.len() - s - 64..x.len() - s] == y[y.len() - s - 64..y.len() - s] {
        s += 64;
    }
    while s < m - p && x[x.len() - s - 1] == y[y.len() - s - 1] {
        s += 1;
    }
    (p, s)
}

/// (tests) line `text` changed by `edits` one after the other (bytes a..b of the line then
/// replaced by s): the column checkpoints and the tokens a code editor keeps of a long line,
/// brought along through each change, against a fresh look at the line -> what differs (empty:
/// nothing)
pub fn code_long_check(text: &str, h: &Highlighter, tw: u8, edits: &[(usize, usize, &str)]) -> Vec<String> {
    let tw = tw.clamp(1, 16) as usize;
    let mut t = String::from(text);
    let mut marks = col_marks(&t, tw);
    let mut tk = Toks::scan(*h, &t, 0);
    let mut out = Vec::new();
    // (a place inside a character: its start)
    let floor = |t: &str, mut i: usize| {
        i = i.min(t.len());
        while !t.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    for (k, &(a, b, s)) in edits.iter().enumerate() {
        let a = floor(&t, a);
        let b = floor(&t, b).max(a);
        let n = alloc::format!("{}{}{}", &t[..a], s, &t[b..]);
        let (p, q) = alike(&t, &n);
        marks = col_marks_edit(&t, &marks, &n, p, q, tw);
        tk.edit(&t, &n, p, q);
        t = n;
        // every checkpoint a cluster boundary with its column, in order
        let bounds = text::segment::graphemes(&t);
        if let Some(m) = marks.iter().find(|m| bounds.binary_search(&m.0).is_err() || col_in(&t, m.0, tw) != m.1) {
            out.push(alloc::format!("edit {k}: checkpoint {m:?}: column {}", col_in(&t, m.0, tw)));
        }
        if marks.windows(2).any(|w| w[0].0 >= w[1].0) {
            out.push(alloc::format!("edit {k}: checkpoints out of order"));
        }
        // the tokens and end state of a whole scan; every place kept one that scan passes
        let mut seen = Vec::new();
        let mut fresh = Vec::new();
        let (_, end) = h.line_part(&t, 0, 0, &mut fresh, &mut |i, o| {
            seen.push((i, o.len(), o.last().map_or(0, |x| x.end)));
            false
        });
        if *tk.toks != fresh || tk.end != end.unwrap_or(0) {
            let at = tk.toks.iter().zip(&fresh).position(|(x, y)| x != y).unwrap_or(fresh.len().min(tk.toks.len()));
            out.push(alloc::format!("edit {k}: tokens differ from {at}: {:?} / {:?}", tk.toks.get(at), fresh.get(at)));
        }
        if let Some(m) = tk.marks.iter().skip(1).find(|m| seen.binary_search(m).is_err()) {
            out.push(alloc::format!("edit {k}: token place {m:?} not one of a whole scan"));
        }
    }
    out
}

impl Longs {
    // f with the entry of a long line, brought up to the document's revision
    fn with<R>(&self, doc: &CodeDoc, line: usize, tw: usize, f: impl FnOnce(&mut LongLine, &str) -> R) -> R {
        let mut s = self.0.borrow_mut();
        let same = s.uid == doc.uid() && s.tw == tw;
        if !same || s.rev != doc.revision() {
            match doc.changes_since(s.rev).filter(|_| same) {
                Some(cs) => {
                    for c in cs {
                        // lines changed in place keep their entries (compared at the next look)
                        s.lines.retain_mut(|e| {
                            if e.line >= c.first + c.old {
                                e.line = e.line - c.old + c.new;
                                true
                            } else if e.line >= c.first && c.old == c.new {
                                e.checked = false;
                                true
                            } else {
                                e.line < c.first
                            }
                        });
                    }
                }
                None => s.lines.clear(),
            }
            (s.uid, s.rev, s.tw) = (doc.uid(), doc.revision(), tw);
        }
        let t = doc.line(line);
        let k = match s.lines.iter().position(|e| e.line == line) {
            Some(k) => k,
            None => {
                // (the oldest go)
                let mut bytes: usize = s.lines.iter().map(|e| e.size()).sum::<usize>() + t.len();
                let mut k = 0;
                while k < s.lines.len() && (s.lines.len() - k >= LONG_KEEP || bytes > LONG_BYTES) {
                    bytes -= s.lines[k].size();
                    k += 1;
                }
                s.lines.drain(..k);
                s.lines.push(LongLine { line, text: String::from(t), checked: true, marks: None, toks: None });
                s.lines.len() - 1
            }
        };
        let e = &mut s.lines[k];
        if !e.checked {
            e.checked = true;
            if e.text != t {
                let (p, q) = alike(&e.text, t);
                if let Some(m) = e.marks.take() {
                    e.marks = Some(col_marks_edit(&e.text, &m, t, p, q, tw));
                }
                if let Some(tk) = e.toks.as_mut() {
                    tk.edit(&e.text, t, p, q);
                }
                e.text.clear();
                e.text.push_str(t);
            }
        }
        f(e, t)
    }

    // the checkpoint of a long line at or before byte x (by_col: column x)
    fn mark(&self, doc: &CodeDoc, line: usize, tw: usize, x: usize, by_col: bool) -> (usize, usize) {
        if doc.line(line).len() < COL_LONG {
            return (0, 0);
        }
        self.with(doc, line, tw, |e, t| {
            let m = e.marks.get_or_insert_with(|| col_marks(t, tw));
            m[m.partition_point(|p| if by_col { p.1 <= x } else { p.0 <= x }).saturating_sub(1)]
        })
    }

    /// the column of byte on a line
    pub(crate) fn col(&self, doc: &CodeDoc, line: usize, byte: usize, tw: usize) -> usize {
        let (b, c) = self.mark(doc, line, tw, byte, false);
        col_from(doc.line(line), b, c, byte, tw)
    }

    /// the cluster boundary nearest to column col
    pub(crate) fn byte(&self, doc: &CodeDoc, line: usize, col: usize, tw: usize) -> usize {
        let (b, c) = self.mark(doc, line, tw, col, true);
        byte_from(doc.line(line), b, c, col, tw)
    }

    /// the cluster boundary at or before column col and its column
    pub(crate) fn floor(&self, doc: &CodeDoc, line: usize, col: usize, tw: usize) -> (usize, usize) {
        let (b, c) = self.mark(doc, line, tw, col, true);
        cell_floor(doc.line(line), b, c, col, tw)
    }

    /// the tokens of a long line from start state st -> (the state at its end, the tokens)
    pub(crate) fn tokens(&self, doc: &CodeDoc, line: usize, tw: usize, h: &Highlighter, st: u32) -> (u32, Rc<Vec<Token>>) {
        self.with(doc, line, tw, |e, t| {
            if !e.toks.as_ref().is_some_and(|k| k.st == st && core::ptr::eq(k.h.grammar(), h.grammar())) {
                e.toks = Some(Toks::scan(*h, t, st));
            }
            match &e.toks {
                Some(k) => (k.end, k.toks.clone()),
                None => (0, Rc::default()),
            }
        })
    }
}
