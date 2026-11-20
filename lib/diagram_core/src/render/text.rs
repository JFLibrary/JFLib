// text -- a shape's text laid out in its text block (JD4): runs of characters, paragraphs,
// wrapping, alignment, the text block's transform and margins
//
//   Measure                         what the layout asks of fonts: the advance of a text at a
//                                   font size of 1 inch, ascent and descent; the drawing side
//                                   (diagram_view) measures with the fonts it draws with
//   ApproxMeasure                   half an em per character (tests, no fonts at hand)
//   TextItem { xform, runs }        lines of runs in the text frame (paper inches, y up) and
//                                   the frame's map to the page; decorations and the text
//                                   background come as their own stroke and fill items
//   measure_text(&mut m, &face, size, text, max_width) -> (w, h)   (JD7) a text in one format
//                                   wrapped as a shape's text wraps: the widest line and the
//                                   lines' height (the formula engine's TEXTWIDTH/TEXTHEIGHT
//                                   through diagram_view's Metrics)
//
// The text comes as the file keeps it (Text: characters between cp, pp, tp and fld marks,
// JD3): a cp mark selects the Character row for what follows, a pp mark the Paragraph row; a
// field Visio wrote with its text inside shows that text, one without text inside (a field of
// JFDiagrammer) its Field row's value in its format (field.rs); LF ends a paragraph,
// U+2028 breaks a line inside one, the last LF of a text is Visio's end mark and makes no empty
// paragraph.  Character cells: Font, Size, Color, ColorTrans, Style (1 bold, 2 italic,
// 4 underline, 8 small caps), Case (1 all caps, 2 initial caps), Pos (1 superscript,
// 2 subscript), Strikethru, DblUnderline, Overline, FontScale, Letterspace.  Paragraph cells:
// IndFirst, IndLeft, IndRight, SpLine (below 0: a multiple of the font size, above 0: inches,
// 0: 120 %), SpBefore, SpAfter, HorzAlign (0 left, 1 centre, 2 right, 3 justified and 4
// distributed drawn left), Bullet and BulletStr.  Text block cells: the four margins,
// VerticalAlign (0 top, 1 middle, 2 bottom), TextBkgnd (0 none, else a colour; a number is a
// colour index plus one), TextDirection (1: vertical), DefaultTabStop.  Lines wrap at spaces
// (a word wider than the line at characters); text that does not fit overflows the block
// the way VerticalAlign says.

use alloc::string::String;
use alloc::vec::Vec;

use super::{Builder, Item, ItemKind, Paint, Stroke};
use crate::cell::Value;
use crate::document::{ShapeRef, SheetRef};
use crate::geometry::{Path, Seg, Sub, Xform};
use crate::names;
use crate::sheet::{CellAddr, RowKey, SecKey};
use crate::shape::TextItem as Mark;

/// a font as the text asks for it: the family the file names, bold, italic
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Face<'a> {
    pub family: &'a str,
    pub bold: bool,
    pub italic: bool,
}

/// what the layout needs of fonts
pub trait Measure {
    /// the advance of `text` (one line, no control characters) at a font size of 1 inch, in
    /// inches
    fn advance(&mut self, face: &Face, text: &str) -> f64;
    /// ascent and descent in em, both positive
    fn metrics(&mut self, _face: &Face) -> (f64, f64) {
        (0.9, 0.25)
    }
}

/// half an em per character, a fifth for a space
#[derive(Copy, Clone, Debug, Default)]
pub struct ApproxMeasure;

impl Measure for ApproxMeasure {
    fn advance(&mut self, _face: &Face, text: &str) -> f64 {
        text.chars().map(|c| if c == ' ' { 0.25 } else { 0.5 }).sum()
    }
}

/// a shape's text, laid out
#[derive(Clone, Debug, PartialEq)]
pub struct TextItem {
    /// the text frame (paper inches, y up, its origin at the block's bottom left corner) to the
    /// page: a turn and a move, never a mirror (a flipped shape's text stays readable)
    pub xform: Xform,
    pub runs: Vec<TextRun>,
}

/// a piece of one line in one format
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    /// where its baseline starts (frame)
    pub at: [f64; 2],
    pub text: String,
    /// the font family as the file names it (the drawing side substitutes)
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    /// the font size (em) in inches
    pub size: f64,
    /// 0xAARRGGBB
    pub color: u32,
    /// FontScale: how much wider the glyphs are (1: as they are)
    pub scale_x: f64,
    /// Letterspace: added after every character (inches)
    pub spacing: f64,
    /// the measured advance with scale and spacing (inches)
    pub width: f64,
}

#[derive(Clone)]
struct CharFmt {
    family: String,
    bold: bool,
    italic: bool,
    underline: bool,
    double_underline: bool,
    overline: bool,
    strike: bool,
    small_caps: bool,
    case: u32,
    pos: u32,
    size: f64,
    color: u32,
    scale_x: f64,
    spacing: f64,
}

#[derive(Clone, Copy)]
struct ParaFmt {
    ind_first: f64,
    ind_left: f64,
    ind_right: f64,
    sp_line: f64,
    sp_before: f64,
    sp_after: f64,
    align: u32,
    bullet: bool,
}

struct Para {
    pp: u32,
    /// (cp, text); '\u{2028}' inside breaks the line
    spans: Vec<(u32, String)>,
}

/// one measured piece: a word, its spaces, a tab or a line break
struct Tok {
    span: usize,
    text: String,
    width: f64,
    space: bool,
    brk: bool,
}

struct Line {
    toks: Vec<Tok>,
    width: f64,
    height: f64,
    ascent: f64,
    first: bool,
}

impl Builder<'_, '_> {
    /// the text of shape `r` (local -> paper transform `m`) as items: background, runs,
    /// decorations
    pub(super) fn text(&mut self, r: ShapeRef, m: &Xform, w: f64, h: f64, layer_color: Option<u32>) {
        let doc = self.doc;
        let Some(shape) = doc.shape(r) else { return };
        let Some(text) = shape.text() else { return };
        let sheet = SheetRef::Shape(r);
        if self.num(sheet, "HideText").is_some_and(|v| v != 0.0) {
            return;
        }
        // paragraphs of spans
        let mut paras = Vec::new();
        let (mut cp, mut pp) = (0u32, 0u32);
        let mut cur = Para { pp: 0, spans: Vec::new() };
        let push_text = |cur: &mut Para, paras: &mut Vec<Para>, cp: u32, pp: &mut u32, s: &str| {
            let mut first = true;
            for part in s.split('\n') {
                if !first {
                    let done = core::mem::replace(cur, Para { pp: *pp, spans: Vec::new() });
                    paras.push(done);
                }
                first = false;
                if !part.is_empty() {
                    let clean: String = part.chars().filter(|c| *c == '\t' || *c == '\u{2028}' || !c.is_control()).collect();
                    cur.spans.push((cp, clean));
                }
            }
        };
        for item in &text.items {
            match item {
                Mark::Chars(s) => push_text(&mut cur, &mut paras, cp, &mut pp, s),
                Mark::Cp(ix) => cp = *ix,
                Mark::Pp(ix) => {
                    pp = *ix;
                    if cur.spans.is_empty() {
                        cur.pp = pp;
                    }
                }
                Mark::Tp(_) => {}
                Mark::Field(ix) => {
                    let v = self.field_text(sheet, *ix);
                    push_text(&mut cur, &mut paras, cp, &mut pp, &v);
                }
                Mark::Other(xml) => {
                    if let Some(v) = crate::field::cached(doc.str(*xml)) {
                        push_text(&mut cur, &mut paras, cp, &mut pp, &v);
                    }
                }
            }
        }
        paras.push(cur);
        // Visio's end mark: the text's last LF makes no paragraph of its own
        if paras.len() > 1 && paras.last().is_some_and(|p| p.spans.is_empty()) {
            paras.pop();
        }
        if paras.iter().all(|p| p.spans.iter().all(|(_, s)| s.trim().is_empty())) {
            return;
        }
        // the block: TxtPin, TxtWidth/Height, TxtLocPin, TxtAngle (defaults: the shape's box)
        let n = |c, d: f64| self.num_sym(sheet, c).unwrap_or(d);
        let tw = n(names::TXT_WIDTH, w);
        let th = n(names::TXT_HEIGHT, h);
        let tpx = n(names::TXT_PIN_X, w * 0.5);
        let tpy = n(names::TXT_PIN_Y, h * 0.5);
        let tlx = n(names::TXT_LOC_PIN_X, tw * 0.5);
        let tly = n(names::TXT_LOC_PIN_Y, th * 0.5);
        let ta = n(names::TXT_ANGLE, 0.0);
        let s = self.scale;
        // frame (paper inches) -> block (drawing inches) -> shape local -> paper
        let mut block = Xform::translate(-tlx, -tly);
        if ta != 0.0 && ta.is_finite() {
            block = block.then(&Xform::rotate(ta));
        }
        block = block.then(&Xform::translate(tpx, tpy));
        let (mut fw, mut fh) = (tw * s, th * s);
        let mut frame = Xform::scale(1.0 / s, 1.0 / s).then(&block).then(m);
        // vertical text: the lines run downwards, the frame turned a quarter clockwise
        if self.num(sheet, "TextDirection").is_some_and(|v| v == 1.0) {
            frame = Xform::rotate(-core::f64::consts::FRAC_PI_2).then(&Xform::translate(0.0, fh)).then(&frame);
            core::mem::swap(&mut fw, &mut fh);
        }
        // a mirrored frame: mirror the text back inside its block so that it reads
        if frame.det() < 0.0 {
            frame = Xform { a: -1.0, b: 0.0, c: 0.0, d: 1.0, e: fw, f: 0.0 }.then(&frame);
        }
        let margin = |c| self.num_sym(sheet, c).unwrap_or(0.0).max(0.0);
        let (ml, mr, mt, mb) = (margin(names::LEFT_MARGIN), margin(names::RIGHT_MARGIN), margin(names::TOP_MARGIN), margin(names::BOTTOM_MARGIN));
        let valign = self.num_sym(sheet, names::VERTICAL_ALIGN).unwrap_or(1.0) as u32;
        let tab = self.num_sym(sheet, names::DEFAULT_TAB_STOP).filter(|t| *t > 0.01).unwrap_or(0.5);
        let inner_w = (fw - ml - mr).max(0.0);

        // formats
        let mut chars: Vec<CharFmt> = Vec::new();
        let mut char_ix: Vec<u32> = Vec::new();
        // lines
        // (format, lines, bullet: its format, text, x and width)
        let mut lines: Vec<(ParaFmt, Vec<Line>, Option<(usize, String, f64, f64)>)> = Vec::new();
        let mut span_fmt: Vec<usize> = Vec::new();
        let mut span_text: Vec<String> = Vec::new();
        for para in &paras {
            let mut pf = self.para_fmt(sheet, para.pp);
            // tokens of the paragraph
            let mut toks: Vec<Tok> = Vec::new();
            for (cp, text) in &para.spans {
                let fi = self.fmt_index(&mut chars, &mut char_ix, sheet, *cp, layer_color);
                let span = span_fmt.len();
                span_fmt.push(fi);
                let shown = cased(text, &chars[fi]);
                span_text.push(shown.clone());
                tokens(self.measure, &shown, span, &chars[fi], &mut toks);
            }
            // a bullet sits at the first line's indent (IndLeft + IndFirst, a hanging indent when
            // IndFirst is negative); the first line's text starts after it, at IndLeft at least
            let bullet = if pf.bullet {
                let b = self.text_cell(sheet, SecKey::one(names::PARAGRAPH), RowKey::Index(para.pp), "BulletStr").filter(|s| !s.is_empty()).unwrap_or_else(|| String::from("\u{2022}"));
                let cp = para.spans.first().map_or(0, |(cp, _)| *cp);
                let fi = self.fmt_index(&mut chars, &mut char_ix, sheet, cp, layer_color);
                let bw = self.measure_run(&chars[fi], &b);
                let bx = (pf.ind_left + pf.ind_first).max(0.0);
                let first = pf.ind_left.max(bx + bw + chars[fi].size * 0.25);
                pf.ind_first = first - pf.ind_left;
                Some((fi, b, bx, bw))
            } else {
                None
            };
            let ls = wrap(self.measure, toks, &pf, inner_w, tab, &chars, &span_fmt);
            lines.push((pf, ls, bullet));
        }
        // heights
        let mut total = 0.0;
        for (i, (pf, ls, _)) in lines.iter().enumerate() {
            if i > 0 || pf.sp_before > 0.0 {
                total += pf.sp_before;
            }
            for l in ls {
                total += l.height;
            }
            total += pf.sp_after;
        }
        let inner_h = fh - mt - mb;
        let top = match valign {
            0 => fh - mt,
            2 => mb + total,
            _ => mb + (inner_h + total) * 0.5,
        };
        // runs
        let mut runs = Vec::new();
        let mut decos: Vec<(Sub, u32, f64)> = Vec::new();
        let mut ext: Option<[f64; 4]> = None;
        let mut y = top;
        for (pf, ls, bullet) in &lines {
            y -= pf.sp_before;
            for l in ls {
                let base = y - l.ascent;
                let indent = if l.first { pf.ind_left + pf.ind_first } else { pf.ind_left };
                let avail = (inner_w - pf.ind_left - pf.ind_right - if l.first { pf.ind_first } else { 0.0 }).max(0.0);
                let x0 = ml + indent + match pf.align {
                    1 => (avail - l.width) * 0.5,
                    2 => avail - l.width,
                    _ => 0.0,
                };
                if let (true, Some((fi, b, bx, bw))) = (l.first, bullet) {
                    runs.push(make_run(&chars[*fi], [ml + bx, base], b.clone(), *bw));
                }
                let mut x = x0;
                let mut k = 0;
                while k < l.toks.len() {
                    // a run: tokens of the same span next to each other
                    let span = l.toks[k].span;
                    let mut text = String::new();
                    let mut width = 0.0;
                    while k < l.toks.len() && l.toks[k].span == span {
                        if !l.toks[k].brk {
                            text.push_str(&l.toks[k].text);
                            width += l.toks[k].width;
                        }
                        k += 1;
                    }
                    let f = &chars[span_fmt[span]];
                    let (size_k, shift) = match f.pos {
                        1 => (0.67, 0.33),
                        2 => (0.67, -0.12),
                        _ => (1.0, 0.0),
                    };
                    let mut run = make_run(f, [x, base + shift * f.size], text.replace('\t', " "), width);
                    run.size *= size_k;
                    let trimmed = text.trim_end().is_empty();
                    if !trimmed {
                        let deco_w = (f.size * 0.06).max(0.003);
                        let mut line = |dy: f64| decos.push((Sub { start: [x, base + dy], segs: alloc::vec![Seg::Line([x + width, base + dy])], closed: false }, f.color, deco_w));
                        if f.underline || f.double_underline {
                            line(-0.11 * f.size);
                        }
                        if f.double_underline {
                            line(-0.20 * f.size);
                        }
                        if f.strike {
                            line(0.28 * f.size);
                        }
                        if f.overline {
                            line(0.82 * f.size);
                        }
                        let b = [x, base - f.size * 0.25, x + width, base + f.size * 0.9];
                        ext = Some(ext.map_or(b, |e| [e[0].min(b[0]), e[1].min(b[1]), e[2].max(b[2]), e[3].max(b[3])]));
                        runs.push(run);
                    }
                    x += width;
                }
                y -= l.height;
            }
            y -= pf.sp_after;
        }
        if runs.is_empty() {
            return;
        }
        // the text background behind the text and its margins
        if let (Some(bg), Some(e)) = (self.text_background(sheet), ext) {
            let q = [e[0] - ml, e[1] - mb, e[2] + mr, e[3] + mt];
            let sub = Sub { start: [q[0], q[1]], segs: alloc::vec![Seg::Line([q[2], q[1]]), Seg::Line([q[2], q[3]]), Seg::Line([q[0], q[3]]), Seg::Line([q[0], q[1]])], closed: true };
            let path = Path { subs: alloc::vec![sub] }.map(&frame);
            self.push_fill(r, super::Role::Text, path, Paint::Solid(bg));
        }
        let corners = [[0.0, 0.0], [fw, 0.0], [fw, fh], [0.0, fh]];
        let mut bb = super::bbox_of(&corners.map(|p| frame.apply(p)));
        if let Some(e) = ext {
            let q = super::bbox_of(&[[e[0], e[1]], [e[2], e[1]], [e[2], e[3]], [e[0], e[3]]].map(|p| frame.apply(p)));
            bb = super::union(bb, q);
        }
        for run in &runs {
            self.fonts.insert(run.family.clone());
        }
        self.items.push(Item { shape: r, top: self.top, bbox: bb, role: super::Role::Text, kind: ItemKind::Text(alloc::boxed::Box::new(TextItem { xform: frame, runs })) });
        for (sub, color, width) in decos {
            let path = Path { subs: alloc::vec![sub] }.map(&frame);
            let stroke = Stroke { color, width, dash: Vec::new(), cap: super::Cap::Butt, join: super::Join::Miter };
            self.push_stroke(r, super::Role::Text, path, stroke);
        }
    }

    /// the index of Character row cp's format in `chars` (made on first use)
    fn fmt_index(&mut self, chars: &mut Vec<CharFmt>, char_ix: &mut Vec<u32>, sheet: SheetRef, cp: u32, layer_color: Option<u32>) -> usize {
        if let Some(i) = char_ix.iter().position(|&c| c == cp) {
            return i;
        }
        let f = self.char_fmt(sheet, cp, layer_color);
        char_ix.push(cp);
        chars.push(f);
        chars.len() - 1
    }

    fn measure_run(&mut self, f: &CharFmt, text: &str) -> f64 {
        run_width(self.measure, f, text)
    }

    fn char_fmt(&mut self, sheet: SheetRef, cp: u32, layer_color: Option<u32>) -> CharFmt {
        let sec = SecKey::one(names::CHARACTER);
        let key = RowKey::Index(cp);
        let num = |b: &Self, c: &str| b.row_num(sheet, sec, key, c);
        let style = num(self, "Style").unwrap_or(0.0) as u32;
        let family = self.font_name(sheet, sec, key);
        let mut color = self.color_cell(sheet, Some((sec, key)), "Color", 0, 0x000000);
        if let Some(c) = layer_color {
            color = c;
        }
        let trans = num(self, "ColorTrans").unwrap_or(0.0);
        let size = num(self, "Size").filter(|s| *s > 0.0 && s.is_finite()).unwrap_or(12.0 / 72.0);
        CharFmt {
            family,
            bold: style & 1 != 0,
            italic: style & 2 != 0,
            underline: style & 4 != 0,
            small_caps: style & 8 != 0,
            double_underline: num(self, "DblUnderline").is_some_and(|v| v != 0.0),
            overline: num(self, "Overline").is_some_and(|v| v != 0.0),
            strike: num(self, "Strikethru").is_some_and(|v| v != 0.0) || num(self, "DoubleStrikethrough").is_some_and(|v| v != 0.0),
            case: num(self, "Case").unwrap_or(0.0) as u32,
            pos: num(self, "Pos").unwrap_or(0.0) as u32,
            size,
            color: super::with_alpha(color, trans),
            scale_x: num(self, "FontScale").filter(|s| *s > 0.01 && s.is_finite()).unwrap_or(1.0),
            spacing: num(self, "Letterspace").filter(|s| s.is_finite()).unwrap_or(0.0),
        }
    }

    fn para_fmt(&mut self, sheet: SheetRef, pp: u32) -> ParaFmt {
        let sec = SecKey::one(names::PARAGRAPH);
        let key = RowKey::Index(pp);
        let num = |b: &Self, c: &str| b.row_num(sheet, sec, key, c).filter(|v| v.is_finite());
        ParaFmt {
            ind_first: num(self, "IndFirst").unwrap_or(0.0),
            ind_left: num(self, "IndLeft").unwrap_or(0.0).max(0.0),
            ind_right: num(self, "IndRight").unwrap_or(0.0).max(0.0),
            sp_line: num(self, "SpLine").unwrap_or(-1.2),
            sp_before: num(self, "SpBefore").unwrap_or(0.0).max(0.0),
            sp_after: num(self, "SpAfter").unwrap_or(0.0).max(0.0),
            align: num(self, "HorzAlign").unwrap_or(1.0) as u32,
            bullet: num(self, "Bullet").is_some_and(|v| v != 0.0),
        }
    }

    /// the Font cell of a Character row: a name, a FaceNames ID, or "Themed" (the theme's minor
    /// font)
    fn font_name(&self, sheet: SheetRef, sec: SecKey, key: RowKey) -> String {
        let doc = self.doc;
        let minor = || self.theme.as_ref().map(|t| t.minor_font.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| String::from("Calibri"));
        match self.row_val(sheet, sec, key, "Font") {
            Value::Text(s) => {
                let t = doc.str(s);
                if t.eq_ignore_ascii_case("Themed") {
                    match self.themed_value(sheet, Some((sec, key)), "Font") {
                        Some(super::themed::Tv::Text(f)) if !f.is_empty() => f,
                        _ => minor(),
                    }
                } else if t.is_empty() {
                    minor()
                } else {
                    String::from(t)
                }
            }
            Value::Num(id) => {
                let id = id as i64;
                let name_sym = doc.sym("Name");
                let id_sym = doc.sym("ID");
                doc.faces
                    .iter()
                    .find(|f| f.attrs.iter().any(|(k, v)| Some(*k) == id_sym && doc.str(*v).parse::<i64>().ok() == Some(id)))
                    .and_then(|f| f.attrs.iter().find(|(k, _)| Some(*k) == name_sym || doc.str(*k) == "NameU").map(|(_, v)| String::from(doc.str(*v))))
                    .unwrap_or_else(minor)
            }
            Value::Absent => minor(),
        }
    }

    /// the text of the field in Field row ix (fld elements without cached text): its value in
    /// its format picture (field.rs)
    fn field_text(&self, sheet: SheetRef, ix: u32) -> String {
        crate::field::shown(self.doc, sheet, ix)
    }

    fn text_background(&self, sheet: SheetRef) -> Option<u32> {
        let trans = self.num_sym(sheet, names::TEXT_BKGND_TRANS).unwrap_or(0.0);
        let c = match self.doc.resolve(sheet, &CellAddr::single(names::TEXT_BKGND)).value() {
            Value::Num(n) if n >= 1.0 => self.palette(n as u32 - 1),
            Value::Text(s) => super::parse_color(self.doc.str(s)).or_else(|| {
                // "Themed": the theme's background
                self.doc.str(s).eq_ignore_ascii_case("Themed").then(|| self.theme.as_ref().and_then(|t| t.background).unwrap_or(0xFFFFFF))
            })?,
            _ => return None,
        };
        (trans < 1.0).then(|| super::with_alpha(c, trans))
    }
}

fn make_run(f: &CharFmt, at: [f64; 2], text: String, width: f64) -> TextRun {
    TextRun { at, text, family: f.family.clone(), bold: f.bold, italic: f.italic, size: if f.small_caps { f.size * 0.8 } else { f.size }, color: f.color, scale_x: f.scale_x, spacing: f.spacing, width }
}

/// the size of a text in one format as a shape's text block sets it: (the widest line, the
/// height of all lines), inches.  Paragraphs end at LF (the text's last LF is Visio's end mark),
/// U+2028 breaks a line, lines wrap at `max_width` the way the layout wraps them (at spaces and
/// after hyphens, a word wider than the line at characters), tabs go to stops 0.5 in apart;
/// a line is 120 % of the size high (SpLine's default).  An empty text is one empty line.
pub fn measure_text(m: &mut dyn Measure, face: &Face, size: f64, text: &str, max_width: Option<f64>) -> (f64, f64) {
    let size = if size.is_finite() && size > 0.0 { size } else { 12.0 / 72.0 };
    let f = CharFmt {
        family: String::from(face.family),
        bold: face.bold,
        italic: face.italic,
        underline: false,
        double_underline: false,
        overline: false,
        strike: false,
        small_caps: false,
        case: 0,
        pos: 0,
        size,
        color: 0,
        scale_x: 1.0,
        spacing: 0.0,
    };
    let pf = ParaFmt { ind_first: 0.0, ind_left: 0.0, ind_right: 0.0, sp_line: -1.2, sp_before: 0.0, sp_after: 0.0, align: 0, bullet: false };
    let inner_w = max_width.filter(|w| w.is_finite()).map_or(f64::INFINITY, |w| w.max(0.0));
    let chars = [f];
    let span_fmt = [0usize];
    let mut paras: Vec<&str> = text.split('\n').collect();
    if paras.len() > 1 && paras.last().is_some_and(|p| p.is_empty()) {
        paras.pop();
    }
    let (mut w, mut h) = (0.0f64, 0.0f64);
    for para in paras {
        let clean: String = para.chars().filter(|c| *c == '\t' || *c == '\u{2028}' || !c.is_control()).collect();
        let mut toks = Vec::new();
        tokens(m, &clean, 0, &chars[0], &mut toks);
        for l in wrap(m, toks, &pf, inner_w, 0.5, &chars, &span_fmt) {
            w = w.max(l.width);
            h += l.height;
        }
    }
    (w, h)
}

/// the advance of a run's text in its format: the size, super- and subscript, FontScale and
/// Letterspace
fn run_width(m: &mut dyn Measure, f: &CharFmt, text: &str) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let k = if f.pos != 0 { 0.67 } else { 1.0 };
    let adv = m.advance(&Face { family: &f.family, bold: f.bold, italic: f.italic }, text);
    adv * f.size * k * f.scale_x + f.spacing * text.chars().count() as f64
}

/// the measured pieces of a span's text (words, spaces, tabs, line breaks)
fn tokens(m: &mut dyn Measure, shown: &str, span: usize, f: &CharFmt, toks: &mut Vec<Tok>) {
    for piece in split_words(shown) {
        let brk = piece == "\u{2028}";
        let space = !brk && piece.chars().all(|c| c == ' ' || c == '\t');
        let width = if brk || piece.contains('\t') { 0.0 } else { run_width(m, f, &piece) };
        toks.push(Tok { span, text: piece, width, space, brk });
    }
}

/// a paragraph's tokens in lines of the block's inner width
fn wrap(m: &mut dyn Measure, toks: Vec<Tok>, pf: &ParaFmt, inner_w: f64, tab: f64, chars: &[CharFmt], span_fmt: &[usize]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut cur = Line { toks: Vec::new(), width: 0.0, height: 0.0, ascent: 0.0, first: true };
    let avail = |first: bool| (inner_w - pf.ind_left - pf.ind_right - if first { pf.ind_first } else { 0.0 }).max(0.0);
    for mut t in toks {
        if t.brk {
            let done = core::mem::replace(&mut cur, Line { toks: Vec::new(), width: 0.0, height: 0.0, ascent: 0.0, first: false });
            lines.push(finish_line(m, done, pf, chars, span_fmt, Some(t.span)));
            continue;
        }
        if t.text.contains('\t') {
            // to the next tab stop of the default spacing
            let next = (crate::geometry::floor(cur.width / tab) + 1.0) * tab;
            t.width = (next - cur.width).max(0.0) + tab * (t.text.matches('\t').count() as f64 - 1.0);
            t.space = true;
        }
        let limit = avail(cur.first);
        if !t.space && cur.width + t.width > limit + 1e-9 && cur.toks.iter().any(|x| !x.space) {
            // break before the word; trailing spaces hang
            let done = core::mem::replace(&mut cur, Line { toks: Vec::new(), width: 0.0, height: 0.0, ascent: 0.0, first: false });
            lines.push(finish_line(m, done, pf, chars, span_fmt, None));
        }
        let limit = avail(cur.first);
        if !t.space && t.width > limit + 1e-9 && limit > 0.0 && t.text.chars().count() > 1 {
            // a word wider than the whole line: at characters
            let f = &chars[span_fmt[t.span]];
            let mut piece = String::new();
            let mut pw = 0.0;
            for c in t.text.chars() {
                let mut s = String::new();
                s.push(c);
                let cw = run_width(m, f, &s);
                if !piece.is_empty() && cur.width + pw + cw > avail(cur.first) + 1e-9 {
                    cur.toks.push(Tok { span: t.span, text: core::mem::take(&mut piece), width: pw, space: false, brk: false });
                    cur.width += pw;
                    pw = 0.0;
                    let done = core::mem::replace(&mut cur, Line { toks: Vec::new(), width: 0.0, height: 0.0, ascent: 0.0, first: false });
                    lines.push(finish_line(m, done, pf, chars, span_fmt, None));
                }
                piece.push(c);
                pw += cw;
            }
            t.text = piece;
            t.width = pw;
        }
        cur.width += t.width;
        cur.toks.push(t);
    }
    lines.push(finish_line(m, cur, pf, chars, span_fmt, None));
    lines
}

/// a line's width without trailing spaces, its height and ascent; an empty line takes the
/// format of `empty_span` (or the paragraph's first)
fn finish_line(m: &mut dyn Measure, mut l: Line, pf: &ParaFmt, chars: &[CharFmt], span_fmt: &[usize], empty_span: Option<usize>) -> Line {
    let trailing: f64 = l.toks.iter().rev().take_while(|t| t.space).map(|t| t.width).sum();
    l.width -= trailing;
    let spans: Vec<usize> = if l.toks.is_empty() { empty_span.or(if span_fmt.is_empty() { None } else { Some(0) }).into_iter().collect() } else { l.toks.iter().map(|t| t.span).collect() };
    let mut size: f64 = 0.0;
    let mut asc: f64 = 0.0;
    for s in spans {
        let f = &chars[span_fmt[s]];
        size = size.max(f.size);
        let (a, d) = m.metrics(&Face { family: &f.family, bold: f.bold, italic: f.italic });
        let em = (a + d).max(0.1);
        // the em box centred in the line, the baseline at its share of ascent
        asc = asc.max(f.size * a / em);
    }
    if size <= 0.0 {
        size = 12.0 / 72.0;
        asc = size * 0.8;
    }
    let height = if pf.sp_line < 0.0 {
        -pf.sp_line * size
    } else if pf.sp_line > 0.0 {
        pf.sp_line
    } else {
        1.2 * size
    };
    l.height = height;
    l.ascent = (height - size) * 0.5 + asc;
    l
}

/// the text in the case its format says
fn cased(s: &str, f: &CharFmt) -> String {
    if f.case == 1 || f.small_caps {
        return s.to_uppercase();
    }
    if f.case == 2 {
        let mut out = String::with_capacity(s.len());
        let mut start = true;
        for c in s.chars() {
            if start && c.is_alphabetic() {
                out.extend(c.to_uppercase());
                start = false;
            } else {
                out.push(c);
                if c.is_whitespace() {
                    start = true;
                } else if c.is_alphabetic() {
                    start = false;
                }
            }
        }
        return out;
    }
    String::from(s)
}

/// words, runs of spaces and tabs, and line breaks
fn split_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut kind = 0u8; // 1 word, 2 spaces
    for c in s.chars() {
        if c == '\u{2028}' {
            if !cur.is_empty() {
                out.push(core::mem::take(&mut cur));
            }
            out.push(String::from("\u{2028}"));
            kind = 0;
            continue;
        }
        let k = if c == ' ' || c == '\t' { 2 } else { 1 };
        if k != kind && !cur.is_empty() {
            out.push(core::mem::take(&mut cur));
        }
        kind = k;
        cur.push(c);
        // a hyphen ends a word (a break may follow)
        if c == '-' && k == 1 {
            out.push(core::mem::take(&mut cur));
            kind = 0;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split() {
        assert_eq!(split_words("ab  cd-ef\u{2028}g"), alloc::vec!["ab", "  ", "cd-", "ef", "\u{2028}", "g"]);
    }
}
