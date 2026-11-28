// The text of a page (ISO 32000-1, 9.4): the content stream is interpreted for its text
// state (fonts, sizes, the text and line matrices, character and word spacing, horizontal
// scaling, rise, the current transformation matrix with q / Q and cm, form XObjects with their
// own matrix and resources), and every glyph shown becomes a TextItem with its text, origin and
// advance in page space.  PageText::text lays them out as Poppler's pdftotext -raw does: in the
// order the page draws them, a new line where the baseline moves by more than half the font
// size, a space where a gap is wider than 0.15 of it (0.08 after a space glyph; a space glyph
// that leaves no gap is no space; a glyph without text leaves a gap), a glyph drawn again on
// one of the last 64 (fake bold, a word printed twice) once, a run of Hebrew or Arabic drawn
// left to right in reading order.

use super::content::Ops;
use super::font::Font;
use super::object::{Dict, Object, Ref};
use super::{Document, Error};
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

/// one glyph shown on a page
#[derive(Clone, Debug, PartialEq)]
pub struct TextItem {
    /// the characters the glyph stands for (usually one; empty when the font does not say)
    pub text: String,
    /// where the glyph starts on its baseline, in page space (points, y up, before /Rotate)
    pub x: f64,
    pub y: f64,
    /// how far the glyph moves the pen, in page space (along the baseline; with character and
    /// word spacing)
    pub dx: f64,
    pub dy: f64,
    /// the glyph's own width along the baseline in page space (without the spacing)
    pub width: f64,
    /// the font size in page space
    pub size: f64,
    /// which of PageText::fonts
    pub font: u16,
    /// render mode 3: drawn invisibly (the text layer of a scan)
    pub invisible: bool,
}

/// the glyphs of a page and the names of their fonts
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageText {
    pub items: Vec<TextItem>,
    pub fonts: Vec<String>,
}

type M = [f64; 6];
const ID: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// a × b (a first, then b)
fn mul(a: &M, b: &M) -> M {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

/// a square root for lengths (no std here): Newton's method from the halved exponent
fn sqrt(x: f64) -> f64 {
    if x.is_nan() || x <= 0.0 || x.is_infinite() {
        return if x.is_infinite() && x > 0.0 { x } else { 0.0 };
    }
    let mut r = f64::from_bits((x.to_bits() >> 1) + 0x1FF8_0000_0000_0000);
    for _ in 0..6 {
        r = 0.5 * (r + x / r);
    }
    r
}

fn finite(m: M) -> M {
    if m.iter().all(|v| v.is_finite()) { m } else { ID }
}

#[derive(Clone)]
struct GState {
    ctm: M,
    font: Option<(Rc<Font>, u16)>,
    size: f64,
    tc: f64,
    tw: f64,
    th: f64,
    tl: f64,
    rise: f64,
    mode: i64,
}

/// form XObjects inside each other
const MAX_FORMS: usize = 12;
/// q without Q
const MAX_STACK: usize = 256;
/// the glyphs of one page
const MAX_ITEMS: usize = 1 << 22;
/// the operators run for one page, form XObjects counted each time they are drawn (forms
/// that draw each other many times would otherwise take time without end)
const MAX_OPS: u64 = 1 << 23;
/// a content stream costs an operator for this many of its bytes each time it is read
const BYTES_PER_OP: usize = 64;

/// the resources a content stream uses here, resolved once
struct Res {
    fonts: Dict,
    xobjects: Dict,
}

impl Res {
    fn of(doc: &Document, res: &Dict) -> Res {
        let get = |k: &str| res.get(k).map(|o| doc.dict(o)).unwrap_or_default();
        Res { fonts: get("Font"), xobjects: get("XObject") }
    }
}

/// a form XObject: its content, resources and matrix, decoded once per page
struct Form {
    data: Vec<u8>,
    res: Option<Rc<Res>>,
    matrix: Option<M>,
}

struct Run<'d> {
    doc: &'d Document,
    fonts: BTreeMap<Ref, (Rc<Font>, u16)>,
    out: PageText,
    forms: Vec<Ref>,
    /// the form XObjects drawn so far (None: not a form, or not to be decoded)
    form_cache: BTreeMap<Ref, Option<Rc<Form>>>,
    ops: u64,
    codes: Vec<(u32, u8)>,
}

fn nums(ops: &[Object]) -> Vec<f64> {
    ops.iter().map(|o| o.as_f64().unwrap_or(0.0)).collect()
}

impl Run<'_> {
    fn font(&mut self, fonts: &Dict, name: &[u8]) -> Option<(Rc<Font>, u16)> {
        let o = fonts.get_bytes(name)?;
        if let Some(r) = o.as_ref() {
            if let Some(f) = self.fonts.get(&r) {
                return Some(f.clone());
            }
        }
        let d = self.doc.dict(o);
        if d.is_empty() {
            return None;
        }
        let f = Rc::new(Font::load(self.doc, &d));
        let idx = self.out.fonts.len().min(u16::MAX as usize) as u16;
        self.out.fonts.push(f.name.clone());
        let entry = (f, idx);
        if let Some(r) = o.as_ref() {
            self.fonts.insert(r, entry.clone());
        }
        Some(entry)
    }

    fn show(&mut self, gs: &GState, tm: &mut M, s: &[u8]) {
        let Some((font, fi)) = gs.font.clone() else { return };
        let mut codes = core::mem::take(&mut self.codes);
        font.codes(s, &mut codes);
        for &(code, len) in &codes {
            let w0 = font.width(code);
            let trm = mul(&[gs.size * gs.th, 0.0, 0.0, gs.size, 0.0, gs.rise], &mul(tm, &gs.ctm));
            let space = if len == 1 && code == 32 { gs.tw } else { 0.0 };
            let (tx, ty) = if font.vertical { (0.0, w0 * gs.size + gs.tc + space) } else { ((w0 * gs.size + gs.tc + space) * gs.th, 0.0) };
            let m = mul(tm, &gs.ctm);
            let (dx, dy) = (tx * m[0] + ty * m[2], tx * m[1] + ty * m[3]);
            let own = if font.vertical { w0 * gs.size * sqrt(m[2] * m[2] + m[3] * m[3]) } else { w0 * gs.size * gs.th * sqrt(m[0] * m[0] + m[1] * m[1]) };
            let size = (gs.size * sqrt(m[2] * m[2] + m[3] * m[3])).abs();
            if self.out.items.len() < MAX_ITEMS && size.is_finite() {
                self.out.items.push(TextItem {
                    text: font.text(code).unwrap_or_default(),
                    x: trm[4],
                    y: trm[5],
                    dx,
                    dy,
                    width: own.abs(),
                    size,
                    font: fi,
                    invisible: gs.mode == 3 || gs.mode == 7,
                });
            }
            *tm = finite(mul(&[1.0, 0.0, 0.0, 1.0, tx, ty], tm));
        }
        self.codes = codes;
    }

    /// a form XObject by its reference (or a direct one), from the cache
    fn form(&mut self, o: &Object) -> Option<Rc<Form>> {
        let r = o.as_ref();
        if let Some(f) = r.and_then(|r| self.form_cache.get(&r)) {
            return f.clone();
        }
        let built = match self.doc.resolve(o) {
            Object::Stream(s) if s.dict.is("Subtype", "Form") => self.doc.decode(&s).ok().map(|data| {
                let res = s.dict.get("Resources").map(|x| Rc::new(Res::of(self.doc, &self.doc.dict(x))));
                let matrix = self.doc.key(&s.dict, "Matrix").as_array().map(nums).filter(|n| n.len() >= 6).map(|n| [n[0], n[1], n[2], n[3], n[4], n[5]]);
                Rc::new(Form { data, res, matrix })
            }),
            _ => None,
        };
        if let Some(r) = r {
            self.form_cache.insert(r, built.clone());
        }
        built
    }

    fn run(&mut self, content: &[u8], res: &Rc<Res>, gs0: GState) {
        // the bytes read count too: a form of a megabyte of white space drawn a million times
        // has no operators but takes as long to read
        self.ops += (content.len() / BYTES_PER_OP) as u64;
        if self.ops > MAX_OPS {
            return;
        }
        let mut gs = gs0;
        let mut stack: Vec<GState> = Vec::new();
        let mut extra_q = 0usize;
        let (mut tm, mut tlm) = (ID, ID);
        for op in Ops::new(content) {
            self.ops += 1;
            if self.ops > MAX_OPS {
                return;
            }
            let a = &op.operands;
            match op.operator {
                b"q" => {
                    if stack.len() < MAX_STACK {
                        stack.push(gs.clone());
                    } else {
                        extra_q += 1;
                    }
                }
                b"Q" => {
                    if extra_q > 0 {
                        extra_q -= 1;
                    } else if let Some(g) = stack.pop() {
                        gs = g;
                    }
                }
                b"cm" if a.len() >= 6 => {
                    let n = nums(&a[a.len() - 6..]);
                    gs.ctm = finite(mul(&[n[0], n[1], n[2], n[3], n[4], n[5]], &gs.ctm));
                }
                b"BT" => {
                    tm = ID;
                    tlm = ID;
                }
                b"Tf" if a.len() >= 2 => {
                    if let Some(n) = a[a.len() - 2].as_name() {
                        let n = n.to_vec();
                        gs.font = self.font(&res.fonts, &n);
                    }
                    gs.size = a[a.len() - 1].as_f64().unwrap_or(0.0);
                }
                b"Tc" if !a.is_empty() => gs.tc = a[0].as_f64().unwrap_or(0.0),
                b"Tw" if !a.is_empty() => gs.tw = a[0].as_f64().unwrap_or(0.0),
                b"Tz" if !a.is_empty() => gs.th = a[0].as_f64().unwrap_or(100.0) / 100.0,
                b"TL" if !a.is_empty() => gs.tl = a[0].as_f64().unwrap_or(0.0),
                b"Ts" if !a.is_empty() => gs.rise = a[0].as_f64().unwrap_or(0.0),
                b"Tr" if !a.is_empty() => gs.mode = a[0].as_int().unwrap_or(0),
                b"Td" | b"TD" if a.len() >= 2 => {
                    let (x, y) = (a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0));
                    if op.operator == b"TD" {
                        gs.tl = -y;
                    }
                    tlm = finite(mul(&[1.0, 0.0, 0.0, 1.0, x, y], &tlm));
                    tm = tlm;
                }
                b"Tm" if a.len() >= 6 => {
                    let n = nums(&a[a.len() - 6..]);
                    tlm = finite([n[0], n[1], n[2], n[3], n[4], n[5]]);
                    tm = tlm;
                }
                b"T*" => {
                    tlm = finite(mul(&[1.0, 0.0, 0.0, 1.0, 0.0, -gs.tl], &tlm));
                    tm = tlm;
                }
                b"Tj" => {
                    if let Some(s) = a.last().and_then(Object::as_bytes) {
                        self.show(&gs, &mut tm, s);
                    }
                }
                b"'" | b"\"" => {
                    if op.operator == b"\"" && a.len() >= 3 {
                        gs.tw = a[0].as_f64().unwrap_or(0.0);
                        gs.tc = a[1].as_f64().unwrap_or(0.0);
                    }
                    tlm = finite(mul(&[1.0, 0.0, 0.0, 1.0, 0.0, -gs.tl], &tlm));
                    tm = tlm;
                    if let Some(s) = a.last().and_then(Object::as_bytes) {
                        self.show(&gs, &mut tm, s);
                    }
                }
                b"TJ" => {
                    let Some(arr) = a.last().and_then(Object::as_array) else { continue };
                    let vertical = gs.font.as_ref().is_some_and(|f| f.0.vertical);
                    for e in arr {
                        match e {
                            Object::String(s) => self.show(&gs, &mut tm, s),
                            o => {
                                if let Some(n) = o.as_f64() {
                                    let adj = -n / 1000.0 * gs.size;
                                    let t = if vertical { [1.0, 0.0, 0.0, 1.0, 0.0, adj] } else { [1.0, 0.0, 0.0, 1.0, adj * gs.th, 0.0] };
                                    tm = finite(mul(&t, &tm));
                                }
                            }
                        }
                    }
                }
                b"Do" if !a.is_empty() => {
                    let Some(o) = a[0].as_name().and_then(|n| res.xobjects.get_bytes(n)).cloned() else { continue };
                    let r = o.as_ref();
                    if self.forms.len() >= MAX_FORMS || r.is_some_and(|r| self.forms.contains(&r)) {
                        continue;
                    }
                    let Some(form) = self.form(&o) else { continue };
                    let mut g = gs.clone();
                    if let Some(m) = form.matrix {
                        g.ctm = finite(mul(&m, &g.ctm));
                    }
                    let fres = form.res.clone().unwrap_or_else(|| res.clone());
                    self.forms.push(r.unwrap_or(Ref { num: u32::MAX, generation: 0 }));
                    self.run(&form.data, &fres, g);
                    self.forms.pop();
                }
                _ => {}
            }
        }
    }
}

impl Document {
    /// the glyphs of a page, in the order the page draws them
    pub fn page_text(&self, page: usize) -> Result<PageText, Error> {
        let p = self.page(page)?;
        let content = self.contents(page)?;
        let res = Rc::new(Res::of(self, &p.resources));
        let mut run = Run { doc: self, fonts: BTreeMap::new(), out: PageText::default(), forms: Vec::new(), form_cache: BTreeMap::new(), ops: 0, codes: Vec::new() };
        let gs = GState { ctm: ID, font: None, size: 0.0, tc: 0.0, tw: 0.0, th: 1.0, tl: 0.0, rise: 0.0, mode: 0 };
        run.run(&content, &res, gs);
        Ok(run.out)
    }

    /// the text of a page in reading order (PageText::text)
    pub fn text(&self, page: usize) -> Result<String, Error> {
        Ok(self.page_text(page)?.text())
    }
}

/// a letter of a right-to-left script (Hebrew, Arabic, Syriac, Thaana, N'Ko and their
/// presentation forms)
fn rtl(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

/// a piece of a line: text and where it starts along the line
struct Piece {
    text: String,
    pos: f64,
}

fn piece_rtl(p: &Piece) -> bool {
    p.text.chars().next().is_some_and(rtl)
}

/// right-to-left runs drawn left to right (the visual order) turned into reading order; a
/// number or other neutral run inside keeps its own order
fn reorder(line: &mut [Piece]) {
    let is_ltr = |p: &Piece| p.text.chars().any(|c| c.is_alphabetic() && !rtl(c));
    let mut i = 0;
    while i < line.len() {
        if !piece_rtl(&line[i]) {
            i += 1;
            continue;
        }
        // the run: from this RTL piece to the last RTL piece before an LTR letter
        let start = i;
        let mut end = i;
        let mut j = i + 1;
        while j < line.len() && !is_ltr(&line[j]) {
            if piece_rtl(&line[j]) {
                end = j;
            }
            j += 1;
        }
        let run = &mut line[start..=end];
        if run.len() > 1 && run[0].pos < run[run.len() - 1].pos {
            // units: single RTL pieces, and blocks of the others in their own order
            let mut units: Vec<Vec<Piece>> = Vec::new();
            for p in run.iter_mut().map(|p| Piece { text: core::mem::take(&mut p.text), pos: p.pos }) {
                let r = piece_rtl(&p);
                match units.last_mut() {
                    Some(u) if !r && !piece_rtl(&u[0]) => u.push(p),
                    _ => units.push(alloc::vec![p]),
                }
            }
            units.reverse();
            for (slot, p) in run.iter_mut().zip(units.into_iter().flatten()) {
                *slot = p;
            }
        }
        i = end + 1;
    }
}

impl PageText {
    /// the text: lines in the order the page draws them, words apart where the glyphs are,
    /// right-to-left runs in reading order
    pub fn text(&self) -> String {
        let mut lines: Vec<Vec<Piece>> = Vec::new();
        let mut cur: Vec<Piece> = Vec::new();
        let mut prev: Option<&TextItem> = None;
        // where the line starts and its direction
        let mut origin = (0.0, 0.0, 1.0, 0.0);
        // a space glyph since the last glyph: a word ends if it made a gap at all
        let mut spaced = false;
        // the last glyphs with text, for glyphs drawn twice
        let mut recent: Vec<&TextItem> = Vec::new();
        for it in &self.items {
            if it.text.is_empty() {
                // a glyph the font gives no text for leaves a gap, as in Poppler
                continue;
            }
            if it.text.chars().all(|c| c == ' ' || c == '\u{A0}') {
                spaced = true;
                continue;
            }
            let tol = 0.1 * it.size.max(1e-6);
            if recent.iter().any(|r| r.text == it.text && (r.x - it.x).abs() < tol && (r.y - it.y).abs() < tol) {
                continue;
            }
            if recent.len() == 64 {
                recent.remove(0);
            }
            recent.push(it);
            let mut new_line = prev.is_none();
            if let Some(p) = prev {
                let len = sqrt(p.dx * p.dx + p.dy * p.dy);
                let (ux, uy) = if len > 1e-9 { (p.dx / len, p.dy / len) } else { (1.0, 0.0) };
                let (rx, ry) = (it.x - (p.x + ux * p.width), it.y - (p.y + uy * p.width));
                let along = rx * ux + ry * uy;
                let across = ux * ry - uy * rx;
                let size = p.size.max(it.size).max(1e-6);
                let ilen = sqrt(it.dx * it.dx + it.dy * it.dy);
                let same_dir = ilen < 1e-9 || len < 1e-9 || (it.dx * ux + it.dy * uy) / ilen > 0.9;
                if !same_dir || across.abs() > 0.5 * size || along < -size * 1.5 {
                    new_line = true;
                } else if along > if spaced { 0.08 } else { 0.15 } * size && !cur.last().is_some_and(|l| l.text.ends_with(' ')) && !it.text.starts_with(' ') {
                    let pos = (it.x - origin.0) * origin.2 + (it.y - origin.1) * origin.3;
                    cur.push(Piece { text: String::from(" "), pos: pos - 1e-3 });
                }
            }
            if new_line {
                if !cur.is_empty() {
                    lines.push(core::mem::take(&mut cur));
                }
                let len = sqrt(it.dx * it.dx + it.dy * it.dy);
                let (ux, uy) = if len > 1e-9 { (it.dx / len, it.dy / len) } else { (1.0, 0.0) };
                origin = (it.x, it.y, ux, uy);
            }
            let pos = (it.x - origin.0) * origin.2 + (it.y - origin.1) * origin.3;
            cur.push(Piece { text: it.text.clone(), pos });
            prev = Some(it);
            spaced = false;
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        let mut out = String::new();
        for mut l in lines {
            if l.iter().any(|p| p.text.chars().any(rtl)) {
                reorder(&mut l);
            }
            for p in l {
                out.push_str(&p.text);
            }
            out.push('\n');
        }
        out
    }
}
