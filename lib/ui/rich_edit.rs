// rich_edit -- ui.richtext: a rich-text editor over a RichDoc (rich_doc.rs), and its toolbar
//
//   ui.richtext(id, &mut doc, RT_*) -> EV_*   the editor, filling the place it gets (natural
//        480 x 240 DIP; next_size(-1, -1) fills): the paragraphs wrapped at the width
//        (text::segment line breaks, grapheme clusters inside a word too long), runs in their
//        fonts (bold / italic: the family's faces, else synthetic), colours, highlight,
//        underline / strike, links in the accent colour, alignment (justify spreads the
//        blanks), bulleted / numbered lists by level, headings, quotations; a scroll bar.
//        EV_ACT: the content changed this frame; EV_FOCUS while focused; EV_COMMIT: the focus
//        left.  Ids id .. id + 5 (id + 1 the scroll bar, id + 2 .. 5 the link flyout).
//        RT_READONLY: select and copy only (a click opens a link); RT_NO_FRAME: no box;
//        RT_OPEN_LINKS: the editor opens an activated link with open_url itself; RT_TAB: Tab
//        outside a list inserts a tab (else Tab moves the focus on); RT_PLAIN_PASTE: paste
//        only text.
//   ui.richtext_toolbar(id, editor, &mut doc, RTB_*) -> EV_ACT when it changed doc
//        undo / redo, paragraph style, font family (registered families: font_register),
//        size, bold / italic / underline / strike, text and highlight colour, link, alignment,
//        lists, indent, clear format; the buttons show the state at the caret and give the
//        focus back to the editor (ids id .. id + 149)
//   ui.richtext_link(id) -> Option<String>   a link the user activated (Ctrl+click, a click
//        when read-only, Ctrl+Enter on it), taken once
//   ui.richtext_link_dialog(id)              open the editor's link flyout (as Ctrl+K)
//
// Keys (besides those of ed_core.rs): Ctrl+B / I / U bold, italic, underline, Ctrl+Shift+X
// strike, Ctrl+Space clear format, Ctrl+K link, Ctrl+L / E / R / J alignment, Ctrl+Shift+8 /
// Ctrl+Shift+L bullets, Ctrl+Shift+7 numbers, Ctrl+1 / 2 / 3 headings, Ctrl+0 body text,
// Ctrl+] / Ctrl+[ size up / down, Tab / Shift+Tab a list level in / out, Enter a new
// paragraph (in an empty list item: the list ends), Backspace at the start of a list item:
// the item becomes body text, Ctrl+Shift+V paste text only, Esc drops the focus.  With no
// selection a format applies to what is typed next.
// Clipboard: CLIP_TEXT (list markers when several paragraphs are copied) and CLIP_HTML
// (rich_html.rs: HTML with the exact fragment as JSON in a comment); a paste takes HTML when
// there is some (any program's), else the text.  Undo: the document's History (rich_doc.rs).
// Accessibility: a K_TEXTBOX record (label: the text, value / max: caret / anchor as byte
// offsets into it) as text boxes have, a K_LINK record per visible link; the caret goes to
// ime_pos (IME, magnifiers); the editor takes the text boxes' edit state while focused
// (ed_hold), so a screen reader can select.  Right-to-left the text keeps its order (as in
// text boxes); bidirectional reordering inside a paragraph is not done.
//
// Layout: a paragraph is laid out once per version (Para::version, its attributes, the
// width, dpi, text scale and document font); the rows are cached per editor.  Only the
// visible paragraphs are drawn.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use text::segment;
use text::{FS_SYN_BOLD, FS_SYN_ITALIC};

use crate::cmd::{Draw, Gfx};
use crate::consts::*;
use crate::ed_core::*;
use crate::i18n::tr;
use crate::keys::*;
use crate::rich_doc::*;
use crate::rich_html::clip_html;
use crate::vicon::paint_svg_icon2;
use crate::{FontSel, Sp, Ui};

/// richtext flags: select and copy only
pub const RT_READONLY: u32 = 1;
/// no text box frame
pub const RT_NO_FRAME: u32 = 2;
/// the editor opens activated links itself (open_url)
pub const RT_OPEN_LINKS: u32 = 4;
/// Tab outside a list inserts a tab
pub const RT_TAB: u32 = 8;
/// a paste takes only the text
pub const RT_PLAIN_PASTE: u32 = 16;

/// richtext_toolbar groups: undo / redo
pub const RTB_HISTORY: u32 = 1;
/// paragraph style, font family, size
pub const RTB_STYLE: u32 = 2;
/// bold, italic, underline, strike
pub const RTB_CHARS: u32 = 4;
/// text and highlight colour
pub const RTB_COLOR: u32 = 8;
/// link
pub const RTB_LINK: u32 = 16;
/// alignment
pub const RTB_ALIGN: u32 = 32;
/// lists and indent
pub const RTB_LISTS: u32 = 64;
/// clear format
pub const RTB_CLEAR: u32 = 128;
/// every group
pub const RTB_ALL: u32 = 255;

const NAT_W: i32 = 480;
const NAT_H: i32 = 240;
const LEVEL_W: i32 = 24; // DIP per indent level
const LIST_W: i32 = 26; // DIP for the list marker
const QUOTE_W: i32 = 16; // DIP of a quotation's indent
const TAB_W: i32 = 40; // DIP between tab stops

// a piece of a row: bytes a..b of one run, at x (px from the text column's left)
#[derive(Clone)]
struct Piece {
    a: usize,
    b: usize,
    x: i32,
    w: i32,
    font: u32,
    size: i32,
    asc: i32,
    flags: u8,
    color: u32,
    bg: u32,
    link: bool,
    tab: bool,
}

// a visual row of a paragraph
struct Row {
    a: usize,
    b: usize,
    // top (from the paragraph's first row), height, baseline offset
    y: i32,
    h: i32,
    base: i32,
    // the caret stops: grapheme boundaries a..=b and their x
    stops: Vec<(usize, i32)>,
    pieces: Vec<Piece>,
}

impl Row {
    fn x(&self, byte: usize) -> i32 {
        match self.stops.binary_search_by(|s| s.0.cmp(&byte)) {
            Ok(i) => self.stops[i].1,
            Err(0) => self.stops.first().map_or(0, |s| s.1),
            Err(i) => self.stops[i - 1].1,
        }
    }
    fn hit(&self, x: i32) -> usize {
        self.stops.iter().min_by_key(|s| (s.1 - x).abs()).map_or(self.a, |s| s.0)
    }
    // the character under x (its start), None outside the row's text
    fn char_at(&self, x: i32) -> Option<usize> {
        let w = self.stops.windows(2).find(|w| x >= w[0].1 && x < w[1].1)?;
        Some(w[0].0)
    }
}

// the layout of a paragraph
struct PLay {
    // space above the first row, the rows' height, the space below
    top: i32,
    h: i32,
    rows: Vec<Row>,
    // where the text column starts (indent, list marker, quotation)
    indent: i32,
    // the marker's font and size (the first run's)
    mfont: (u32, i32, i32),
}

#[derive(Clone, PartialEq, Default)]
struct LKey {
    w: i32,
    dpi: i32,
    scale: u32,
    font: String,
    size: u16,
    ui: u32,
}

// the state of one editor between frames
#[derive(Default)]
struct View {
    seen: u32,
    scroll: i32,
    key: LKey,
    lays: BTreeMap<(u32, u8, u8, u8, u8), Rc<PLay>>,
    t0: u32,
    prefx: i32,
    // pointer drag: 0 none, 1 characters, 2 words (the word it began on)
    drag: u8,
    word: (TextPos, TextPos),
    held: Option<(usize, usize)>,
    last_sel: TextSel,
    last_rev: u64,
    // the text for the accessibility record, by document and revision
    label: (u32, u64, String),
    // the link flyout: open, its text, the caret's rectangle (anchor)
    link_open: bool,
    link_text: String,
    caret_rect: [i32; 4],
}

/// shared state (Ui::m.rich)
#[derive(Default)]
pub(crate) struct S {
    views: BTreeMap<u32, View>,
    link: Option<(u32, String)>,
    icons: BTreeMap<(usize, u32, u32), Rc<Art>>,
}

// a toolbar icon (vicon path data) in a colour and an accent colour
struct Art {
    art: &'static str,
    color: u32,
    accent: u32,
}

impl Draw for Art {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let s = r[2].min(r[3]);
        paint_svg_icon2(g, self.art, r[0] + (r[2] - s) / 2, r[1] + (r[3] - s) / 2, s << 8, self.color, self.accent, 1.75);
    }
}

// toolbar icons on a 24 x 24 grid
const ART_BOLD: &str = "S2.4M7.5 5H13A3.5 3.5 0 0 1 13 12H7.5ZM7.5 12H14A3.5 3.5 0 0 1 14 19H7.5Z";
const ART_ITALIC: &str = "M10 5H18M6 19H14M14 5L10 19";
const ART_UNDER: &str = "M7 4V11A5 5 0 0 0 17 11V4M6 20H18";
const ART_STRIKE: &str = "M16.5 7.5C15.8 5.9 14.2 5 12 5C9.3 5 7.5 6.3 7.5 8.4C7.5 10 8.6 11 10.4 11.5M4.5 12.5H19.5M8 16.2C8.7 18 10.3 19 12.3 19C15 19 16.8 17.6 16.8 15.6C16.8 14.8 16.6 14.2 16.1 13.6";
const ART_LEFT: &str = "M4 6H20M4 10H14M4 14H20M4 18H14";
const ART_CENTER: &str = "M4 6H20M7 10H17M4 14H20M7 18H17";
const ART_RIGHT: &str = "M4 6H20M10 10H20M4 14H20M10 18H20";
const ART_JUSTIFY: &str = "M4 6H20M4 10H20M4 14H20M4 18H20";
const ART_BULLETS: &str = "M9.5 6.5H20M9.5 12H20M9.5 17.5H20|FM6.4 6.5A1.2 1.2 0 1 1 4 6.5A1.2 1.2 0 1 1 6.4 6.5Z|FM6.4 12A1.2 1.2 0 1 1 4 12A1.2 1.2 0 1 1 6.4 12Z|FM6.4 17.5A1.2 1.2 0 1 1 4 17.5A1.2 1.2 0 1 1 6.4 17.5Z";
const ART_NUMBERS: &str = "M10 6.5H20M10 12H20M10 17.5H20|S1.3M4.4 5.2L5.6 4.4V8.6M4.4 8.6H6.8|S1.3M4.3 11C4.7 10.2 6.7 10.2 6.7 11.4C6.7 12.4 4.3 12.9 4.3 14.2H6.9|S1.3M4.3 16.1C4.8 15.5 6.7 15.5 6.7 16.7C6.7 17.4 6 17.7 5.4 17.7C6.1 17.7 6.9 18 6.9 18.9C6.9 20.1 4.9 20.3 4.2 19.5";
const ART_INDENT: &str = "M11 6H20M11 10H20M11 14H20M11 18H20|M4 9L7.5 12L4 15";
const ART_OUTDENT: &str = "M11 6H20M11 10H20M11 14H20M11 18H20|M7.5 9L4 12L7.5 15";
const ART_LINK: &str = "M10 14L14 10M11 7L13 5A3.5 3.5 0 0 1 19 11L17 13M13 17L11 19A3.5 3.5 0 0 1 5 13L7 11";
const ART_COLOR: &str = "M7 16L12 4.5L17 16M8.8 12H15.2|AFM4 18.5H20V21.5H4Z";
const ART_HILITE: &str = "M14.5 4.5L19.5 9.5L11 18H6V13Z|M12.7 6.3L17.7 11.3|AFM3.5 19.5H20.5V22H3.5Z";
const ART_CLEAR: &str = "M6 5H16M11 5L8.5 17|M14 14.5L19 19.5M19 14.5L14 19.5";
const ART_UNDO: &str = "M9 14L4 9L9 4|M4 9H14.5A5.5 5.5 0 0 1 14.5 20H8";
const ART_REDO: &str = "M15 14L20 9L15 4|M20 9H9.5A5.5 5.5 0 0 0 9.5 20H16";

// the colours of the colour menus (0: automatic / none)
const PALETTE: [u32; 20] = [
    0xFF000000, 0xFF595959, 0xFF8C8C8C, 0xFFC0C0C0, 0xFFFFFFFF, 0xFFC00000, 0xFFE8590C, 0xFFF2C200, 0xFF2E9E44, 0xFF0F7B8C, 0xFF1F5FBF,
    0xFF6A3FB5, 0xFFB5338A, 0xFF7F3F00, 0xFF3D6B00, 0xFFFFF27A, 0xFFB7F0A6, 0xFFA6E0F5, 0xFFF5C2E0, 0xFFFFD0A6,
];
const SIZES: [u16; 14] = [80, 90, 100, 110, 120, 140, 160, 180, 200, 240, 280, 360, 480, 720];

// the scale of a paragraph style (percent) and whether it is bold
fn style_scale(s: u8) -> (u32, bool) {
    match s {
        PS_H1 => (180, true),
        PS_H2 => (145, true),
        PS_H3 => (120, true),
        _ => (100, false),
    }
}

// the colour of c over an opaque background (alpha)
fn opaque(c: u32) -> u32 {
    if c >> 24 == 0 { c | 0xFF00_0000 } else { c }
}

// the view of the caret moves over the laid out paragraphs
struct RView<'a> {
    doc: &'a RichDoc,
    lays: &'a [Rc<PLay>],
    tops: &'a [i32],
}

impl EdView for RView<'_> {
    fn lines(&self) -> usize {
        self.doc.para_count()
    }
    fn text(&self, line: usize) -> &str {
        self.doc.para(line).text()
    }
    fn rows(&self, line: usize) -> usize {
        self.lays[line].rows.len().max(1)
    }
    fn row_range(&self, line: usize, row: usize) -> (usize, usize) {
        let r = &self.lays[line].rows[row];
        (r.a, r.b)
    }
    fn x_at(&self, line: usize, row: usize, byte: usize) -> i32 {
        self.lays[line].rows[row].x(byte)
    }
    fn hit_x(&self, line: usize, row: usize, x: i32) -> usize {
        self.lays[line].rows[row].hit(x)
    }
    fn row_y(&self, line: usize, row: usize) -> (i32, i32) {
        let l = &self.lays[line];
        let r = &l.rows[row];
        (self.tops[line] + l.top + r.y, r.h)
    }
    fn row_at(&self, y: i32) -> (usize, usize) {
        let n = self.lays.len();
        let mut line = self.tops.partition_point(|&t| t <= y).saturating_sub(1).min(n - 1);
        let l = &self.lays[line];
        let ly = y - self.tops[line] - l.top;
        if ly >= l.rows.last().map_or(0, |r| r.y + r.h) && line + 1 < n && y >= self.tops[line] + l.h {
            line += 1;
        }
        let l = &self.lays[line];
        let ly = y - self.tops[line] - l.top;
        let row = l.rows.iter().rposition(|r| r.y <= ly).unwrap_or(0);
        (line, row)
    }
}

impl RView<'_> {
    // the caret stop nearest to (x, y) (content px)
    fn hit(&self, x: i32, y: i32) -> TextPos {
        let (line, row) = self.row_at(y);
        TextPos::new(line, self.hit_x(line, row, x))
    }
    // the character under (x, y), None beside the text
    fn char_at(&self, x: i32, y: i32) -> Option<TextPos> {
        let (line, row) = self.row_at(y);
        let (ry, rh) = self.row_y(line, row);
        if y < ry || y >= ry + rh {
            return None;
        }
        self.lays[line].rows[row].char_at(x).map(|b| TextPos::new(line, b))
    }
}

impl Ui {
    // the font of a run: the family's face of that weight / slant, else one of its faces with
    // synthetic styles, else the UI font
    pub(crate) fn rt_face(&mut self, family: &str, bold: bool, italic: bool) -> u32 {
        let syn = |ui: &mut Ui, f: u32, flags: u32| -> u32 {
            if flags == 0 {
                return f;
            }
            let r = ui.text.style(f, flags);
            if r >= 0 { r as u32 } else { f }
        };
        let want = (if bold { FS_SYN_BOLD } else { 0 }) | (if italic { FS_SYN_ITALIC } else { 0 });
        if !family.is_empty() {
            if self.font_is_registered(family) {
                let w = if bold { 700 } else { 400 };
                if let Some(f) = self.font_registered_face(family, w, italic) {
                    return f;
                }
                if let Some(f) = self.font_registered_face(family, w, false) {
                    return syn(self, f, want & FS_SYN_ITALIC);
                }
                if let Some(f) = self.font_registered_face(family, 400, italic) {
                    return syn(self, f, want & FS_SYN_BOLD);
                }
                if let Some(f) = self.font_registered_face(family, 400, false) {
                    return syn(self, f, want);
                }
            } else if family.eq_ignore_ascii_case("monospace") {
                let (f, _) = self.mono_font(11.0);
                return syn(self, f, want);
            } else if let Some(f) = self.font_load(&FontSel { family: String::from(family), weight: if bold { 700 } else { 400 }, italic, size: 11.0 }) {
                return f;
            }
        }
        let base = if bold { text::FONT_BOLD } else { self.style.ts_font()[TS_BODY as usize] as u32 };
        syn(self, base, want & FS_SYN_ITALIC)
    }

    // tenths of a point -> 24.8 px at this dpi and text scale
    fn rt_px(&self, pt10: u32) -> i32 {
        let px = pt10 as f32 / 10.0 * 4.0 / 3.0 * self.wp.rt.dpi as f32 / 96.0 * self.text_scale as f32 / 100.0;
        ((px * 256.0) as i32).clamp(256, 512 * 256)
    }

    // lay out one paragraph at width w (px of the text column)
    fn rt_lay(&mut self, p: &Para, w: i32, doc_font: &str, base10: u16, faces: &mut Vec<((String, bool, bool), u32)>) -> PLay {
        let (scale, sbold) = style_scale(p.style);
        let mut face = |ui: &mut Ui, f: &Fmt| -> (u32, i32) {
            let fam = if f.font.is_empty() { doc_font } else { f.font.as_str() };
            let bold = f.flags & RF_BOLD != 0 || sbold;
            let italic = f.flags & RF_ITALIC != 0;
            let k = (String::from(fam), bold, italic);
            let id = match faces.iter().find(|e| e.0 == k) {
                Some(e) => e.1,
                None => {
                    let id = ui.rt_face(fam, bold, italic);
                    faces.push((k, id));
                    id
                }
            };
            let pt = if f.size != 0 { f.size } else { base10 } as u32;
            (id, ui.rt_px(pt * scale / 100))
        };
        let indent = self.dp(LEVEL_W) * p.level as i32
            + if p.list != LS_NONE { self.dp(LIST_W) } else { 0 }
            + if p.style == PS_QUOTE { self.dp(QUOTE_W) } else { 0 };
        let avail = (w - indent).max(self.dp(40));
        let text = p.text();
        let tb = text.as_bytes();
        // the runs with their fonts
        struct Seg {
            a: usize,
            b: usize,
            font: u32,
            size: i32,
            asc: i32,
            desc: i32,
            lh: i32,
            fmt: Fmt,
        }
        let mut segs: Vec<Seg> = Vec::new();
        let metrics = |ui: &mut Ui, font: u32, size: i32| {
            ui.text.set_font(font, size);
            let (asc, desc) = (ui.text.ascent(), ui.text.descent());
            (asc, desc, ui.text.line_h().max(asc + desc))
        };
        for (a, b, f) in p.spans() {
            let (font, size) = face(self, f);
            let (asc, desc, lh) = metrics(self, font, size);
            segs.push(Seg { a, b, font, size, asc, desc, lh, fmt: f.clone() });
        }
        let (mfont, msize) = face(self, segs.first().map_or(&p.mark, |s| &s.fmt));
        let (masc, mdesc, mlh) = metrics(self, mfont, msize);
        let tab = self.dp(TAB_W).max(1) << 8;
        // the width (24.8) of a..b laid out from pen x (tabs to the next stop)
        let width = |ui: &mut Ui, a: usize, b: usize, x: i32| -> i32 {
            let mut pen = x;
            for s in segs.iter() {
                let (s0, s1) = (s.a.max(a), s.b.min(b));
                if s0 >= s1 {
                    continue;
                }
                ui.text.set_font(s.font, s.size);
                let mut i = s0;
                while i < s1 {
                    if tb[i] == b'\t' {
                        pen = (pen / tab + 1) * tab;
                        i += 1;
                        continue;
                    }
                    let j = tb[i..s1].iter().position(|&c| c == b'\t').map_or(s1, |k| i + k);
                    pen += ui.text.measure_n(&tb[i..], j - i);
                    i = j;
                }
            }
            pen - x
        };
        // break the text into rows: the last opportunity that fits, else clusters
        let mut rows_ab: Vec<(usize, usize)> = Vec::new();
        if !text.is_empty() {
            let ops: Vec<usize> = segment::line_breaks(text).into_iter().map(|o| o.0).collect();
            let vis = |e: usize, s: usize| {
                let mut v = e;
                while v > s && (tb[v - 1] == b' ' || tb[v - 1] == b'\t') {
                    v -= 1;
                }
                v
            };
            let lim = avail << 8;
            let mut start = 0;
            let mut x = 0; // pen of the row so far (24.8)
            let mut prev = 0; // the last opportunity in the row
            for &o in &ops {
                if o <= prev {
                    continue;
                }
                let atom = width(self, prev, vis(o, prev), x);
                if x + atom > lim && prev > start {
                    rows_ab.push((start, prev));
                    start = prev;
                    x = 0;
                }
                if x == 0 {
                    // an atom wider than the row: clusters while they fit
                    let mut a = start;
                    loop {
                        let w_all = width(self, a, vis(o, a), 0);
                        if w_all <= lim || a >= o {
                            break;
                        }
                        let mut e = segment::grapheme_next(text, a).min(o);
                        while e < o {
                            let n = segment::grapheme_next(text, e).min(o);
                            if width(self, a, n, 0) > lim {
                                break;
                            }
                            e = n;
                        }
                        rows_ab.push((a, e));
                        a = e;
                        start = e;
                    }
                }
                x += width(self, prev.max(start), o, x);
                prev = o;
            }
            rows_ab.push((start, text.len()));
        } else {
            rows_ab.push((0, 0));
        }
        // the rows: pieces, caret stops, metrics, alignment
        let mut rows: Vec<Row> = Vec::with_capacity(rows_ab.len());
        let mut y = 0;
        let nrows = rows_ab.len();
        for (ri, &(ra, rb)) in rows_ab.iter().enumerate() {
            let mut pieces: Vec<Piece> = Vec::new();
            let mut stops: Vec<(usize, i32)> = Vec::new();
            let (mut asc, mut desc, mut lh) = (0, 0, 0);
            let mut pen = 0; // 24.8
            let justify = p.align == RA_JUSTIFY && ri + 1 < nrows;
            for s in segs.iter() {
                let (s0, s1) = (s.a.max(ra), s.b.min(rb));
                if s0 >= s1 {
                    continue;
                }
                asc = asc.max(s.asc);
                desc = desc.max(s.desc);
                lh = lh.max(s.lh);
                self.text.set_font(s.font, s.size);
                // pieces split at tabs (and, justified, after blanks)
                let mut i = s0;
                while i < s1 {
                    if tb[i] == b'\t' {
                        let np = (pen / tab + 1) * tab;
                        stops.push((i, (pen + 128) >> 8));
                        pieces.push(Piece { a: i, b: i + 1, x: (pen + 128) >> 8, w: (np - pen + 128) >> 8, font: s.font, size: s.size, asc: s.asc, flags: s.fmt.flags, color: s.fmt.color, bg: s.fmt.bg, link: !s.fmt.link.is_empty(), tab: true });
                        pen = np;
                        i += 1;
                        continue;
                    }
                    let mut j = tb[i..s1].iter().position(|&c| c == b'\t').map_or(s1, |k| i + k);
                    if justify {
                        if let Some(k) = tb[i..j].iter().position(|&c| c == b' ') {
                            j = i + k + 1;
                        }
                    }
                    let piece = &tb[i..j];
                    let x0 = pen;
                    // a stop at every cluster boundary
                    let mut g = i;
                    while g < j {
                        stops.push((g, (x0 + self.text.measure_n(piece, g - i) + 128) >> 8));
                        g = segment::grapheme_next(text, g).min(j);
                    }
                    let w = self.text.measure_n(piece, j - i);
                    pieces.push(Piece { a: i, b: j, x: (x0 + 128) >> 8, w: (w + 255) >> 8, font: s.font, size: s.size, asc: s.asc, flags: s.fmt.flags, color: s.fmt.color, bg: s.fmt.bg, link: !s.fmt.link.is_empty(), tab: false });
                    pen = x0 + w;
                    i = j;
                }
            }
            stops.push((rb, (pen + 128) >> 8));
            if pieces.is_empty() {
                asc = masc;
                desc = mdesc;
                lh = mlh;
            }
            let h = lh.max(asc + desc).max(1);
            let base = asc + (h - asc - desc) / 2;
            // the visible width (without trailing blanks) and the alignment offset
            let mut vis_end = rb;
            while vis_end > ra && (tb[vis_end - 1] == b' ' || tb[vis_end - 1] == b'\t') {
                vis_end -= 1;
            }
            let vis_w = stops.iter().find(|s| s.0 >= vis_end).map_or(0, |s| s.1);
            let off = match p.align {
                RA_CENTER => ((avail - vis_w) / 2).max(0),
                RA_RIGHT => (avail - vis_w).max(0),
                _ => 0,
            };
            if justify {
                let blanks: Vec<usize> = (ra..vis_end).filter(|&i| tb[i] == b' ').collect();
                if !blanks.is_empty() && vis_w < avail {
                    let extra = ((avail - vis_w) as i64) << 8;
                    let n = blanks.len() as i64;
                    let shift = |b: usize| -> i32 {
                        let k = blanks.partition_point(|&s| s < b) as i64;
                        ((extra * k / n) >> 8) as i32
                    };
                    for s in stops.iter_mut() {
                        s.1 += shift(s.0);
                    }
                    for pc in pieces.iter_mut() {
                        pc.x += shift(pc.a);
                    }
                }
            }
            for s in stops.iter_mut() {
                s.1 += indent + off;
            }
            for pc in pieces.iter_mut() {
                pc.x += indent + off;
            }
            stops.dedup_by_key(|s| s.0);
            rows.push(Row { a: ra, b: rb, y, h, base, stops, pieces });
            y += h;
        }
        let top = match p.style {
            PS_H1 => self.dp(12),
            PS_H2 => self.dp(10),
            PS_H3 => self.dp(8),
            _ => 0,
        };
        let gap = if p.list != LS_NONE { self.dp(3) } else { self.dp(8) };
        PLay { top, h: top + y + gap, rows, indent, mfont: (mfont, msize, masc) }
    }

    // the layout of every paragraph (cached by version) -> (layouts, tops, height)
    fn rt_layout(&mut self, v: &mut View, doc: &RichDoc, w: i32) -> (Vec<Rc<PLay>>, Vec<i32>, i32) {
        let key = LKey { w, dpi: self.wp.rt.dpi, scale: self.layout_scale(), font: doc.font.clone(), size: doc.size, ui: self.style.ts_font()[TS_BODY as usize] as u32 };
        if key != v.key {
            v.lays.clear();
            v.key = key;
        }
        let base10 = if doc.size == 0 { 110 } else { doc.size };
        let prev = self.text.get_font();
        let mut faces = Vec::new();
        let mut keep = BTreeMap::new();
        let mut lays = Vec::with_capacity(doc.para_count());
        let mut tops = Vec::with_capacity(doc.para_count());
        let mut y = 0;
        for p in doc.paras() {
            let k = (p.version(), p.align, p.list, p.level, p.style);
            let l = match v.lays.get(&k) {
                Some(l) => l.clone(),
                None => Rc::new(self.rt_lay(p, w, &doc.font, base10, &mut faces)),
            };
            tops.push(y);
            y += l.h;
            keep.insert(k, l.clone());
            lays.push(l);
        }
        v.lays = keep;
        self.text.set_font(prev.0, prev.1);
        (lays, tops, y)
    }

    /// ui_richtext(id, doc, RT_*) -> EV_* (see the module header)
    pub fn richtext(&mut self, id: u32, doc: &mut RichDoc, flags: u32) -> u32 {
        let ro = flags & RT_READONLY != 0;
        let stflags = self.take_flags();
        let r = self.place(self.dp(NAT_W), self.dp(NAT_H));
        let hair = self.hair();
        let st = self.style;
        let frame = flags & RT_NO_FRAME == 0;
        let pad = if frame { self.dp(10) } else { self.dp(2) };
        let sbw = st.scrollbar_thickness(self);
        let (tx, ty) = (r[0] + pad, r[1] + pad);
        let tw = (r[2] - pad * 2 - sbw).max(self.dp(40));
        let th = (r[3] - pad * 2).max(1);
        let mut v = self.m.rich.views.remove(&id).unwrap_or_default();
        v.seen = self.frame;
        let rev0 = doc.revision();
        let sel0 = doc.selection();
        let (mut lays, mut tops, mut total) = self.rt_layout(&mut v, doc, tw);
        // behave: one box for the pointer and the focus
        let mut hb = HB_FOCUS | HB_TEXT | CUR_IBEAM << HB_CURSOR_SHIFT;
        if stflags & ST_DISABLED != 0 {
            hb |= HB_DISABLED;
        }
        self.wp.ctx.last_rect = r;
        let ev = self.behave(id, hb);
        let disabled = self.wp.ctx.last_state == SS_DISABLED as u32;
        let focused = !disabled && self.ed_focused(id);
        let mut out = ev & (EV_HOVER | EV_HELD | EV_DOUBLE | EV_RCLICK | EV_DRAG);
        if focused {
            // a selection set from outside (a screen reader) since the last frame
            if let (Some(sel), Some(held)) = (self.textbox_selection(id), v.held) {
                if sel != held {
                    doc.set_selection(TextSel::new(doc.unflat(sel.0), doc.unflat(sel.1)));
                }
            }
            if let Some(t) = self.a11y_text(id) {
                if !ro {
                    doc.set_paras(RichDoc::from_plain(&t).paras().to_vec());
                    let e = doc.end();
                    doc.set_selection(TextSel::at(e));
                }
            }
        } else if v.held.is_some() {
            v.held = None;
            v.drag = 0;
            self.ed_drop(id);
            out |= EV_COMMIT;
        }
        if doc.revision() != rev0 {
            (lays, tops, total) = self.rt_layout(&mut v, doc, tw);
        }
        let mut laid = doc.revision();
        // ---- pointer
        let (mx, my) = (self.rtl_map(self.wp.ctx.mx, tx, tw), self.wp.ctx.my);
        let (cx, cy) = (mx - tx, my - ty + v.scroll);
        let c = &self.wp.ctx;
        let press = c.pressed & (1 << BTN_LEFT) != 0 && ev & EV_HOVER != 0 && c.active == id;
        if press && !disabled {
            let view = RView { doc: &*doc, lays: &lays, tops: &tops };
            let p = view.hit(cx, cy);
            let link = view.char_at(cx, cy).and_then(|q| doc.link_at(q).map(String::from));
            v.prefx = -1;
            v.t0 = self.now_ms;
            if let Some(url) = link.filter(|_| ro || self.mods & MOD_CTRL != 0) {
                self.rt_link_activate(id, &url, flags);
                v.drag = 0;
            } else if ev & EV_DOUBLE != 0 {
                let (a, b) = word_range(doc.para(p.line).text(), p.byte);
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
            let lh = lays.first().and_then(|l| l.rows.first()).map_or(self.dp(20), |r| r.h);
            if my < ty {
                v.scroll -= lh;
            } else if my >= ty + th {
                v.scroll += lh;
            }
            v.scroll = v.scroll.clamp(0, (total - th).max(0));
            let cy = my - ty + v.scroll;
            let view = RView { doc: &*doc, lays: &lays, tops: &tops };
            let p = view.hit(cx, cy);
            if v.drag == 2 {
                let (a, b) = word_range(doc.para(p.line).text(), p.byte);
                let (wa, wb) = (TextPos::new(p.line, a), TextPos::new(p.line, b));
                if wa < v.word.0 {
                    doc.set_selection(TextSel::new(v.word.1, wa));
                } else {
                    doc.set_selection(TextSel::new(v.word.0, wb.max(v.word.1)));
                }
            } else {
                let a = doc.selection().anchor;
                doc.set_selection(TextSel::new(a, p));
            }
        } else if ev & EV_HELD == 0 {
            v.drag = 0;
        }
        // ---- keys and typed text
        if focused && !v.link_open {
            loop {
                if doc.revision() != laid {
                    (lays, tops, total) = self.rt_layout(&mut v, doc, tw);
                    laid = doc.revision();
                }
                let k0 = self.ed_key_first();
                if k0 == 0 {
                    break;
                }
                let k = self.rtl_text_key(k0);
                if self.rt_key(id, doc, &mut v, k, flags) {
                    self.ed_key_take();
                    v.t0 = self.now_ms;
                    if v.link_open {
                        break;
                    }
                    continue;
                }
                let act = {
                    let view = RView { doc: &*doc, lays: &lays, tops: &tops };
                    ed_key(&view, doc.selection(), &mut v.prefx, k, th, false)
                };
                let Some(act) = act else { break };
                self.ed_key_take();
                v.t0 = self.now_ms;
                match act {
                    EdAct::Sel(s) => doc.set_selection(s),
                    EdAct::Del(a, b, _) if !ro => {
                        let back = k & 0xFFFF == VK_BACK;
                        doc.delete_kind(a, b, if !doc.selection().is_empty() { 0 } else if back { RK_BACK } else { RK_DEL });
                    }
                    EdAct::Enter if !ro => {
                        doc.split();
                    }
                    EdAct::Copy => self.rt_copy(doc),
                    EdAct::Cut => {
                        self.rt_copy(doc);
                        if !ro {
                            doc.delete_selection();
                        }
                    }
                    EdAct::Paste if !ro => self.rt_paste(doc, flags & RT_PLAIN_PASTE != 0),
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
                    doc.insert_text_kind(&s, RK_TYPE);
                    v.t0 = self.now_ms;
                    v.prefx = -1;
                }
            }
        }
        if doc.revision() != laid || lays.len() != doc.para_count() {
            (lays, tops, total) = self.rt_layout(&mut v, doc, tw);
        }
        let changed = doc.revision() != rev0;
        let moved = doc.selection() != sel0;
        if changed {
            out |= EV_ACT;
        }
        if changed || moved {
            self.repass();
        }
        // ---- wheel, the caret into view
        let lh = lays.first().and_then(|l| l.rows.first()).map_or(self.dp(20), |r| r.h);
        let wy = self.wp.ctx.wheel_y;
        if (ev & EV_HOVER != 0 || self.wp.ctx.hot == id.wrapping_add(1)) && wy != 0 {
            let s = (v.scroll - wy * lh * 3 / 120).clamp(0, (total - th).max(0));
            if s != v.scroll {
                v.scroll = s;
                self.wp.ctx.wheel_y = 0;
            }
        }
        let caret = doc.clamp(doc.selection().caret);
        let (crow_y, crow_h, cxp) = {
            let view = RView { doc: &*doc, lays: &lays, tops: &tops };
            let row = row_of(&view, caret);
            let (y, h) = view.row_y(caret.line, row);
            (y, h, view.x_at(caret.line, row, caret.byte))
        };
        if focused && (doc.selection() != v.last_sel || doc.revision() != v.last_rev) {
            if crow_y < v.scroll {
                v.scroll = crow_y;
            }
            if crow_y + crow_h > v.scroll + th {
                v.scroll = crow_y + crow_h - th;
            }
        }
        v.last_sel = doc.selection();
        v.last_rev = doc.revision();
        v.scroll = v.scroll.clamp(0, (total - th).max(0));

        // ---- frame
        if frame {
            let mut sp = Sp { value: NAT_W, value2: lh, flags: 1, ..Sp::default() };
            let _ = st.textbox_metrics(self, &mut sp);
            sp.place(r);
            sp.ev = ev;
            sp.value = focused as i32;
            sp.value2 = disabled as i32;
            st.textbox_frame(self, &sp);
        }
        self.clip_push(r[0] + hair, r[1] + hair, (r[2] - hair * 2).max(0), (r[3] - hair * 2).max(0));
        self.ltr_begin(tx, tw);
        let marks = list_marks(doc.paras());
        let (sa, sb) = doc.selection().range();
        let sel_col = if focused {
            (self.color(T_ACCENT) & 0x00FF_FFFF) | 0x60 << 24
        } else {
            (self.color(T_TEXT3) & 0x00FF_FFFF) | 0x38 << 24
        };
        let tcol = self.color(if disabled { T_TEXT_DISABLED } else { T_TEXT });
        let qcol = self.color(T_TEXT2);
        let lcol = self.color(T_ACCENT_TEXT);
        let hc = self.high_contrast;
        let first = tops.partition_point(|&t| t <= v.scroll).saturating_sub(1);
        let mut links: Vec<([i32; 4], String)> = Vec::new();
        for line in first..lays.len() {
            let top = ty + tops[line] - v.scroll;
            if top > ty + th {
                break;
            }
            let l = &lays[line];
            let p = doc.para(line);
            let ptop = top + l.top;
            // quotation bar, list marker
            if p.style == PS_QUOTE {
                let x = tx + self.dp(LEVEL_W) * p.level as i32 + self.dp(4);
                let hgt = l.rows.last().map_or(0, |r| r.y + r.h);
                self.draw_color(self.color(T_CTRL_STROKE2) | 0xFF00_0000);
                self.draw_rect(x, ptop, self.dp(3), hgt);
            }
            if let (Some(m), Some(row)) = (&marks[line], l.rows.first()) {
                let (mf, ms, masc) = l.mfont;
                let c = match p.runs().next() {
                    Some((_, f)) if f.color != 0 && !hc => opaque(f.color),
                    _ => tcol,
                };
                let base = ptop + row.y + row.base;
                if p.list == LS_BULLET {
                    // bullets as shapes (fonts may lack ◦ and ▪): disc, ring, square by level
                    let d = ((masc as f32 * 0.36) as i32).max(self.dp(4)) & !1;
                    let cx = tx + l.indent - self.dp(9) - d / 2;
                    let cy = base - (masc as f32 * 0.34) as i32;
                    self.draw_color(c);
                    match p.level % 3 {
                        0 => self.draw_circle(cx << 8, cy << 8, (d << 8) / 2, 0),
                        1 => self.draw_circle(cx << 8, cy << 8, (d << 8) / 2 - 128, self.hair().max(1) << 8),
                        _ => self.draw_rect(cx - d / 2, cy - d / 2, d, d),
                    }
                } else {
                    self.text.set_font(mf, ms);
                    let mw = self.text.width(m);
                    let x = tx + l.indent - self.dp(6) - mw;
                    self.cmd_text(x, base.max(ptop + row.y + masc), m.as_bytes(), mf, ms, c, 0);
                }
            }
            for row in &l.rows {
                let ry = ptop + row.y;
                if ry > ty + th || ry + row.h < ty {
                    continue;
                }
                // the selection under the text (the paragraph break: a small block after the
                // last row)
                let (rs, re) = (TextPos::new(line, row.a), TextPos::new(line, row.b));
                if sa < sb && sa <= re && sb >= rs {
                    let (lo, hi) = (sa.max(rs), sb.min(re));
                    let brk = sb > re && core::ptr::eq(row, l.rows.last().unwrap());
                    if lo < hi || brk {
                        let x0 = row.x(lo.byte);
                        let x1 = row.x(hi.byte) + if brk { self.dp(5) } else { 0 };
                        self.draw_color(sel_col);
                        self.draw_rect(tx + x0, ry, (x1 - x0).max(1), row.h);
                    }
                }
                for pc in &row.pieces {
                    let x = tx + pc.x;
                    // high contrast: the document's colours give way to the scheme's (a
                    // highlight becomes a frame)
                    let (pcolor, pbg) = if hc { (0, 0) } else { (pc.color, pc.bg) };
                    if pbg != 0 {
                        self.draw_color(opaque(pbg));
                        self.draw_rect(x, ry, pc.w, row.h);
                    } else if hc && pc.bg != 0 {
                        self.draw_color(tcol);
                        self.draw_frame(x, ry, pc.w, row.h, 0, hair.max(1));
                    }
                    if pc.tab {
                        continue;
                    }
                    let col = if disabled {
                        tcol
                    } else if pcolor != 0 {
                        opaque(pcolor)
                    } else if pbg != 0 {
                        // the theme's text colour may not show on the highlight
                        let c = opaque(pbg);
                        let luma = ((c >> 16 & 255) * 299 + (c >> 8 & 255) * 587 + (c & 255) * 114) / 1000;
                        if luma > 140 { 0xFF1A1A1A } else { 0xFFF2F2F2 }
                    } else if pc.link {
                        lcol
                    } else if p.style == PS_QUOTE {
                        qcol
                    } else {
                        tcol
                    };
                    let base = ry + row.base;
                    self.cmd_text(x, base, &p.text().as_bytes()[pc.a..pc.b], pc.font, pc.size, col, 0);
                    let px = pc.size >> 8;
                    let th_line = (px / 14).max(1);
                    if pc.flags & RF_UNDER != 0 || pc.link {
                        self.draw_color(col);
                        self.draw_rect(x, base + (px / 9).max(1), pc.w, th_line);
                    }
                    if pc.flags & RF_STRIKE != 0 {
                        self.draw_color(col);
                        self.draw_rect(x, base - pc.asc * 3 / 10, pc.w, th_line);
                    }
                    if pc.link && links.len() < 64 {
                        let t = String::from(&p.text()[pc.a..pc.b]);
                        match links.last_mut() {
                            Some((lr, lt)) if lr[1] == ry && lr[0] + lr[2] == x => {
                                lr[2] += pc.w;
                                lt.push_str(&t);
                            }
                            _ => links.push(([x, ry, pc.w, row.h], t)),
                        }
                    }
                }
            }
        }
        // the caret
        let (crx, cry) = (tx + cxp, ty + crow_y - v.scroll);
        v.caret_rect = [crx, cry, hair.max(1), crow_h];
        if focused {
            let pe = self.ed_preedit();
            if !pe.text.is_empty() {
                // the composition in the font at the caret
                let l = &lays[caret.line];
                let row = &l.rows[row_of(&RView { doc: &*doc, lays: &lays, tops: &tops }, caret)];
                let pc = row.pieces.iter().find(|pc| pc.a <= caret.byte && caret.byte <= pc.b && !pc.tab);
                let font = pc.map_or((l.mfont.0, l.mfont.1), |pc| (pc.font, pc.size));
                let bg = if frame { self.color(if focused { T_INPUT_ACTIVE } else { T_CTRL }) } else { self.color(T_BG) };
                let a = (bg >> 24) as i32;
                let bg = crate::theme::color_mix(self.color(T_BG) | 0xFF00_0000, bg | 0xFF00_0000, a + (a >> 7));
                self.ed_preedit_draw(&pe, crx, cry, crow_h, cry + row.base, font, tcol, bg);
            } else if doc.selection().is_empty() && self.ed_caret_on(v.t0) && cry + crow_h > ty && cry < ty + th {
                self.draw_color(tcol);
                self.draw_rect(crx, cry, hair.max(1), crow_h);
            }
            self.ime_pos(crx, cry, crow_h);
        }
        self.ltr_end();
        // links: a hand over them, a record each
        for (lr, label) in &links {
            self.wp.ctx.last_rect = *lr;
            let before = self.wp.ctx.hits[self.wp.ctx.hit_cur].len();
            self.hit(id, hb & ((1 << HB_CURSOR_SHIFT) - 1) | CUR_HAND << HB_CURSOR_SHIFT);
            if let Some(h) = self.wp.ctx.hits[self.wp.ctx.hit_cur].get_mut(before) {
                h.continuation = true;
            }
            self.wp.ctx.last_state = 0;
            self.sem(K_LINK, 0, Some(label), 0);
        }
        self.clip_pop();
        // ---- the scroll bar
        if total > th {
            let mut pos = v.scroll;
            self.next_rect_px(r[0] + r[2] - sbw - hair, r[1] + hair, sbw, r[3] - hair * 2);
            if self.scrollbar(id.wrapping_add(1), &mut pos, total, th, true) & EV_ACT != 0 {
                v.scroll = pos.clamp(0, (total - th).max(0));
            }
        }
        // ---- the record: the text, caret and anchor as offsets into it
        if v.label.0 != doc.uid() || v.label.1 != doc.revision() {
            v.label = (doc.uid(), doc.revision(), doc.text());
        }
        let (fa, fc) = (doc.flat(doc.selection().anchor), doc.flat(doc.selection().caret));
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = if focused { SS_FOCUSED as u32 } else { 0 } | if disabled { SS_DISABLED as u32 } else { 0 };
        let kind = (TB_MULTI | if ro { TB_READONLY } else { 0 }) >> 24;
        let label = core::mem::take(&mut v.label.2);
        {
            let s = self.sem(K_TEXTBOX, id, Some(&label), if focused { fc as i32 } else { 0 });
            s.max = if focused { fa as i32 } else { 0 };
            s.min = kind as i32;
        }
        v.label.2 = label;
        if focused {
            self.ed_hold(id, fa, fc);
            v.held = Some((fa, fc));
            out |= EV_FOCUS;
        }
        // ---- the link flyout
        if v.link_open {
            self.rt_link_flyout(id, doc, &mut v);
        }
        self.m.rich.views.insert(id, v);
        // views of editors no longer drawn go
        let frame_now = self.frame;
        if self.m.rich.views.len() > 8 {
            self.m.rich.views.retain(|_, v| frame_now.wrapping_sub(v.seen) < 600);
        }
        self.wp.ctx.last_rect = r;
        out
    }

    // a key of the editor itself -> consumed
    fn rt_key(&mut self, id: u32, doc: &mut RichDoc, v: &mut View, k: u32, flags: u32) -> bool {
        let ro = flags & RT_READONLY != 0;
        let vk = k & 0xFFFF;
        let m = k >> 16;
        let altgr = m & MOD_ALTGR != 0 || m & (MOD_CTRL | MOD_ALT) == MOD_CTRL | MOD_ALT;
        if altgr || m & MOD_ALT != 0 {
            return false;
        }
        let ctrl = m & MOD_CTRL != 0;
        let shift = m & MOD_SHIFT != 0;
        let sel = doc.selection();
        if ctrl {
            let edit = |doc: &mut RichDoc, f: &dyn Fn(&mut RichDoc) -> bool| {
                if !ro {
                    f(doc);
                }
                true
            };
            return match (vk, shift) {
                (0x42, false) => edit(doc, &|d| d.toggle(RF_BOLD)),
                (0x49, false) => edit(doc, &|d| d.toggle(RF_ITALIC)),
                (0x55, false) => edit(doc, &|d| d.toggle(RF_UNDER)),
                (0x58, true) => edit(doc, &|d| d.toggle(RF_STRIKE)),
                (VK_SPACE, false) => edit(doc, &|d| d.clear_format()),
                (0x4C, false) => edit(doc, &|d| d.set_align(RA_LEFT)),
                (0x45, false) => edit(doc, &|d| d.set_align(RA_CENTER)),
                (0x52, false) => edit(doc, &|d| d.set_align(RA_RIGHT)),
                (0x4A, false) => edit(doc, &|d| d.set_align(RA_JUSTIFY)),
                (0x38, true) | (0x4C, true) => edit(doc, &|d| d.toggle_list(LS_BULLET)),
                (0x37, true) => edit(doc, &|d| d.toggle_list(LS_NUMBER)),
                (0x31, false) => edit(doc, &|d| d.set_style(PS_H1)),
                (0x32, false) => edit(doc, &|d| d.set_style(PS_H2)),
                (0x33, false) => edit(doc, &|d| d.set_style(PS_H3)),
                (0x30, false) => edit(doc, &|d| d.set_style(PS_NORMAL)),
                (VK_OEM_6, false) | (VK_OEM_4, false) => {
                    let up = vk == VK_OEM_6;
                    let base = if doc.size == 0 { 110 } else { doc.size };
                    let cur = doc.current_fmt().size;
                    let cur = if cur == 0 { base } else { cur };
                    let next = if up { SIZES.iter().copied().find(|&s| s > cur).unwrap_or(cur) } else { SIZES.iter().rev().copied().find(|&s| s < cur).unwrap_or(cur) };
                    edit(doc, &|d| d.set_size(next))
                }
                (0x4B, false) => {
                                if !ro {
                        self.rt_link_open(id, doc, v);
                    }
                    true
                }
                (0x56, true) => {
                    if !ro {
                        self.rt_paste(doc, true);
                    }
                    true
                }
                (VK_RETURN, false) => {
                    // Ctrl+Enter on a link opens it
                    if let Some(url) = doc.link_at(sel.caret).or_else(|| if sel.caret.byte > 0 { doc.link_at(TextPos::new(sel.caret.line, sel.caret.byte - 1)) } else { None }).map(String::from) {
                        self.rt_link_activate(id, &url, flags);
                        return true;
                    }
                    false
                }
                _ => false,
            };
        }
        match vk {
            VK_TAB => {
                let para = doc.current_para();
                let in_list = para.list != LS_NONE || !sel.is_empty() && (sel.start().line != sel.end().line);
                if in_list || (shift && para.level > 0) {
                    if !ro {
                        doc.indent(if shift { -1 } else { 1 });
                    }
                    return true;
                }
                if flags & RT_TAB != 0 && !shift {
                    if !ro {
                        doc.insert_text_kind("\t", RK_TYPE);
                    }
                    return true;
                }
                false
            }
            VK_BACK if sel.is_empty() && sel.caret.byte == 0 && !ro => {
                let para = doc.current_para();
                if para.list != LS_NONE {
                    doc.set_list(LS_NONE);
                    return true;
                }
                if para.level > 0 {
                    doc.indent(-1);
                    return true;
                }
                false
            }
            VK_ESCAPE => {
                self.wp.ctx.focus = 0;
                self.repass();
                true
            }
            _ => false,
        }
    }

    // the selection to the clipboard: text and HTML
    fn rt_copy(&mut self, doc: &RichDoc) {
        let sel = doc.selection();
        if sel.is_empty() {
            return;
        }
        let frag = doc.selected_paras();
        let text = if frag.len() > 1 { RichDoc::from_paras(frag.clone()).to_plain() } else { doc.selected_text() };
        let html = clip_html(&frag, &doc.font, doc.size);
        self.clip_set_many(&[(CLIP_TEXT, text.as_bytes()), (CLIP_HTML, html.as_bytes())]);
    }

    // paste: HTML when there is some (and rich pastes are allowed), else the text
    fn rt_paste(&mut self, doc: &mut RichDoc, plain: bool) {
        if !plain {
            if let Some(h) = self.clip_get(CLIP_HTML) {
                let html = String::from_utf8_lossy(&h).into_owned();
                let d = RichDoc::from_html(&html);
                if d.paras().iter().any(|p| !p.is_empty()) || d.para_count() > 1 {
                    doc.insert_paras(d.paras());
                    return;
                }
            }
        }
        if let Some(t) = self.clip_get(CLIP_TEXT) {
            let s = String::from_utf8_lossy(&t).replace('\0', "");
            if !s.is_empty() {
                doc.insert_text_kind(&s, 0);
            }
        }
    }

    fn rt_link_activate(&mut self, id: u32, url: &str, flags: u32) {
        self.m.rich.link = Some((id, String::from(url)));
        if flags & RT_OPEN_LINKS != 0 {
            self.open_url(url);
        }
        self.repass();
    }

    /// a link the user activated in richtext `id` (Ctrl+click, a click when read-only,
    /// Ctrl+Enter), taken once
    pub fn richtext_link(&mut self, id: u32) -> Option<String> {
        match &self.m.rich.link {
            Some((i, _)) if *i == id => self.m.rich.link.take().map(|l| l.1),
            _ => None,
        }
    }

    // open the link flyout of editor id (the link at the caret, else "https://")
    fn rt_link_open(&mut self, id: u32, doc: &RichDoc, v: &mut View) {
        let sel = doc.selection();
        let at = if sel.is_empty() { sel.caret } else { sel.start() };
        let cur = doc.link_at(at).or_else(|| if at.byte > 0 { doc.link_at(TextPos::new(at.line, at.byte - 1)) } else { None });
        v.link_text = String::from(cur.unwrap_or("https://"));
        v.link_open = true;
        self.popup_open(id.wrapping_add(2));
        self.wp.ctx.focus = id.wrapping_add(3);
        self.wp.ctx.focus_visible = false;
        self.repass();
    }

    /// open the link flyout of richtext `id` (as Ctrl+K does): the url of the link at the
    /// caret or "https://"; OK sets it on the selection (without one: inserts it as linked
    /// text), "Entfernen" removes the link
    pub fn richtext_link_dialog(&mut self, id: u32, doc: &RichDoc) {
        let mut v = self.m.rich.views.remove(&id).unwrap_or_default();
        self.rt_link_open(id, doc, &mut v);
        self.m.rich.views.insert(id, v);
    }

    fn rt_link_flyout(&mut self, id: u32, doc: &mut RichDoc, v: &mut View) {
        let pid = id.wrapping_add(2);
        // Esc in the url box closes the flyout (the box would only drop its focus)
        if self.wp.ctx.focus == id.wrapping_add(3) && self.ed_key_chord(VK_ESCAPE, 0) {
            v.link_open = false;
            self.popup_close();
            self.ed_focus_quiet(id);
            return;
        }
        self.wp.ctx.last_rect = v.caret_rect;
        if !self.popup_is_open(pid) {
            self.popup_open(pid);
        }
        let mut done = None;
        if self.flyout_begin(pid, 0) {
            self.text(TS_BODY, tr("Link-Adresse"));
            self.next_size(self.dp(280), 0);
            // Enter taken before the box (its EV_COMMIT also comes from a focus change)
            if self.wp.ctx.focus == id.wrapping_add(3) && self.ed_key_chord(VK_RETURN, 0) {
                done = Some(true);
            }
            self.textbox(id.wrapping_add(3), Some("https://"), &mut v.link_text, 2048);
            self.box_begin(0, BOX_H, 6);
            if self.accent_button(id.wrapping_add(4), tr("OK")) & EV_ACT != 0 {
                done = Some(true);
            }
            if self.button(id.wrapping_add(5), tr("Link entfernen")) & EV_ACT != 0 {
                done = Some(false);
            }
            self.box_end();
        } else if self.wp.ctx.focus != id.wrapping_add(3) {
            // closed (Esc, a click outside)
            v.link_open = false;
        }
        self.flyout_end();
        if let Some(set) = done {
            let url = if set { String::from(v.link_text.trim()) } else { String::new() };
            doc.set_link(&url);
            v.link_open = false;
            self.popup_close();
            self.ed_focus_quiet(id);
        }
    }

    // ------------------------------------------------------------------ the toolbar

    // a toolbar icon (cached so that its identity stays)
    fn rt_art(&mut self, art: &'static str, color: u32, accent: u32) -> Rc<Art> {
        let k = (art.as_ptr() as usize, color, accent);
        self.m.rich.icons.entry(k).or_insert_with(|| Rc::new(Art { art, color, accent })).clone()
    }

    // a square tool button with an icon -> clicked
    fn rt_tool(&mut self, id: u32, art: &'static str, label: &str, checked: bool, enabled: bool, accent: u32) -> bool {
        let st = self.style;
        let mut flags = ST_SUBTLE | ST_ICON_ONLY;
        if checked {
            flags |= ST_CHECKED;
        }
        if !enabled {
            flags |= ST_DISABLED;
        }
        let s = self.dp(32);
        let r = self.place(s, s);
        let ev = self.behave(id, if enabled { HB_FOCUS } else { HB_DISABLED });
        let mut sp = Sp { flags, label: Some(label), ..Sp::default() };
        self.sp_placed(&mut sp, ev);
        st.button(self, &sp);
        let c = self.color(if enabled { T_TEXT } else { T_TEXT_DISABLED });
        let a = if accent != 0 { opaque(accent) } else { c };
        let art = self.rt_art(art, c, a);
        let is = self.dp(18);
        self.cmd_custom(r[0] + (r[2] - is) / 2, r[1] + (r[3] - is) / 2, is, is, Some(art), 0, 0);
        self.focus_ring_draw(ev);
        if checked {
            self.wp.ctx.last_state |= SS_CHECKED as u32;
        }
        self.sem(K_BUTTON, id, Some(label), 0);
        self.tooltip(label);
        ev & EV_ACT != 0
    }

    // a drop-down button of the toolbar (opens popup pid) -> the popup is open
    fn rt_drop(&mut self, id: u32, pid: u32, label: &str, w: i32, tip: &str) -> bool {
        self.next_size(w, 0);
        self.next_flags(ST_SUBTLE);
        let ev = self.button(id, label);
        self.tooltip(tip);
        if ev & EV_ACT != 0 {
            self.popup_open(pid);
        }
        self.flyout_begin(pid, 0)
    }

    /// ui_richtext_toolbar(id, editor, doc, RTB_*) -> EV_ACT when it changed doc (see the
    /// module header)
    pub fn richtext_toolbar(&mut self, id: u32, editor: u32, doc: &mut RichDoc, groups: u32) -> u32 {
        let rev = doc.revision();
        let fmt = doc.current_fmt();
        let para = doc.current_para().clone();
        let mut acted = false;
        let id = |k: u32| id.wrapping_add(k);
        self.box_begin(0, BOX_H, 2);
        let sep = |ui: &mut Ui| {
            ui.next_size(ui.dp(1), ui.dp(24));
            let r = ui.place(ui.dp(1), ui.dp(24));
            ui.draw_color(ui.color(T_DIVIDER));
            ui.draw_rect(r[0], r[1] + ui.dp(4), r[2].max(1), r[3] - ui.dp(8));
        };
        if groups & RTB_HISTORY != 0 {
            if self.rt_tool(id(0), ART_UNDO, tr("Rückgängig"), false, doc.can_undo(), 0) {
                doc.undo();
                acted = true;
            }
            if self.rt_tool(id(1), ART_REDO, tr("Wiederholen"), false, doc.can_redo(), 0) {
                doc.redo();
                acted = true;
            }
            sep(self);
        }
        if groups & RTB_STYLE != 0 {
            let names = [tr("Fließtext"), tr("Überschrift 1"), tr("Überschrift 2"), tr("Überschrift 3"), tr("Zitat")];
            if self.rt_drop(id(2), id(3), names[para.style.min(4) as usize], self.dp(118), tr("Formatvorlage")) {
                for (i, n) in names.iter().enumerate() {
                    if para.style == i as u8 {
                        self.next_flags(ST_CHECKED);
                    }
                    if self.menu_item(id(10 + i as u32), n) & EV_ACT != 0 {
                        doc.set_style(i as u8);
                        acted = true;
                    }
                }
            }
            self.flyout_end();
            let mut fams: Vec<String> = alloc::vec![String::new()];
            fams.extend(self.font_registered());
            if !fams.iter().any(|f| f.eq_ignore_ascii_case("monospace")) {
                fams.push(String::from("monospace"));
            }
            let fam_label = if fmt.font.is_empty() { String::from(tr("Standardschrift")) } else { fmt.font.clone() };
            if self.rt_drop(id(4), id(5), &fam_label, self.dp(130), tr("Schriftart")) {
                for (i, f) in fams.iter().enumerate().take(16) {
                    if *f == fmt.font {
                        self.next_flags(ST_CHECKED);
                    }
                    let l = if f.is_empty() { String::from(tr("Standardschrift")) } else { f.clone() };
                    if self.menu_item(id(60 + i as u32), &l) & EV_ACT != 0 {
                        doc.set_font(f);
                        acted = true;
                    }
                }
            }
            self.flyout_end();
            let size = if fmt.size == 0 { if doc.size == 0 { 110 } else { doc.size } } else { fmt.size };
            if self.rt_drop(id(6), id(7), &size_text(size), self.dp(52), tr("Schriftgröße")) {
                for (i, s) in SIZES.iter().enumerate() {
                    if *s == size {
                        self.next_flags(ST_CHECKED);
                    }
                    let l = size_text(*s);
                    if self.menu_item(id(80 + i as u32), &l) & EV_ACT != 0 {
                        doc.set_size(*s);
                        acted = true;
                    }
                }
            }
            self.flyout_end();
            sep(self);
        }
        if groups & RTB_CHARS != 0 {
            for (k, flag, art, label) in [
                (8, RF_BOLD, ART_BOLD, tr("Fett")),
                (9, RF_ITALIC, ART_ITALIC, tr("Kursiv")),
                (15, RF_UNDER, ART_UNDER, tr("Unterstrichen")),
                (16, RF_STRIKE, ART_STRIKE, tr("Durchgestrichen")),
            ] {
                if self.rt_tool(id(k), art, label, fmt.flags & flag != 0, true, 0) {
                    doc.toggle(flag);
                    acted = true;
                }
            }
            sep(self);
        }
        if groups & RTB_COLOR != 0 {
            // swatch ids: text colours id + 100 .., highlights id + 130 .., "none" id + 99 / 129
            for (k, pid, base, art, label, cur, is_bg) in [
                (17u32, 18u32, 100u32, ART_COLOR, tr("Textfarbe"), fmt.color, false),
                (19, 36, 130, ART_HILITE, tr("Hervorhebung"), fmt.bg, true),
            ] {
                let shown = if cur != 0 { cur } else if is_bg { 0xFFFFE36B } else { self.color(T_TEXT) };
                if self.rt_tool(id(k), art, label, false, true, shown) {
                    self.popup_open(id(pid));
                }
                if self.flyout_begin(id(pid), 0) {
                    let auto = if is_bg { tr("Keine") } else { tr("Automatisch") };
                    if self.menu_item(id(base - 1), auto) & EV_ACT != 0 {
                        if is_bg { doc.set_bg(0) } else { doc.set_color(0) };
                        acted = true;
                    }
                    let cols: &[u32] = if is_bg { &PALETTE[4..] } else { &PALETTE[..15] };
                    for (r, row) in cols.chunks(5).enumerate() {
                        self.box_begin(0, BOX_H, 4);
                        for (k, &c) in row.iter().enumerate() {
                            if self.swatch(id(base + (r * 5 + k) as u32), c, c == cur) & EV_ACT != 0 {
                                if is_bg { doc.set_bg(c) } else { doc.set_color(c) };
                                acted = true;
                                self.popup_close();
                            }
                        }
                        self.box_end();
                    }
                }
                self.flyout_end();
            }
            sep(self);
        }
        if groups & RTB_LINK != 0 {
            let on = !fmt.link.is_empty();
            if self.rt_tool(id(21), ART_LINK, tr("Link"), on, true, 0) {
                let mut v = self.m.rich.views.remove(&editor).unwrap_or_default();
                self.rt_link_open(editor, doc, &mut v);
                self.m.rich.views.insert(editor, v);
            }
            sep(self);
        }
        if groups & RTB_ALIGN != 0 {
            for (k, a, art, label) in [
                (22, RA_LEFT, ART_LEFT, tr("Linksbündig")),
                (23, RA_CENTER, ART_CENTER, tr("Zentriert")),
                (24, RA_RIGHT, ART_RIGHT, tr("Rechtsbündig")),
                (25, RA_JUSTIFY, ART_JUSTIFY, tr("Blocksatz")),
            ] {
                if self.rt_tool(id(k), art, label, para.align == a, true, 0) {
                    doc.set_align(a);
                    acted = true;
                }
            }
            sep(self);
        }
        if groups & RTB_LISTS != 0 {
            if self.rt_tool(id(26), ART_BULLETS, tr("Aufzählung"), para.list == LS_BULLET, true, 0) {
                doc.toggle_list(LS_BULLET);
                acted = true;
            }
            if self.rt_tool(id(27), ART_NUMBERS, tr("Nummerierung"), para.list == LS_NUMBER, true, 0) {
                doc.toggle_list(LS_NUMBER);
                acted = true;
            }
            if self.rt_tool(id(28), ART_OUTDENT, tr("Einzug verkleinern"), false, para.level > 0, 0) {
                doc.indent(-1);
                acted = true;
            }
            if self.rt_tool(id(29), ART_INDENT, tr("Einzug vergrößern"), false, para.level < LEVEL_MAX, 0) {
                doc.indent(1);
                acted = true;
            }
            sep(self);
        }
        if groups & RTB_CLEAR != 0 && self.rt_tool(id(30), ART_CLEAR, tr("Format löschen"), false, true, 0) {
            doc.clear_format();
            acted = true;
        }
        self.box_end();
        if acted {
            self.ed_focus_quiet(editor);
            self.repass();
        }
        if doc.revision() != rev { EV_ACT } else { 0 }
    }
}
