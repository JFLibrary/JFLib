// rich_html -- RichDoc as HTML and back
//
//   doc.to_html() -> String          a fragment: <div style="white-space:pre-wrap"> with <p>,
//                                    <h1>..<h3>, <blockquote>, nested <ul> / <ol> (<li>), and
//                                    per run <a href>, <span style="color / background-color /
//                                    font-size / font-family">, <b>, <i>, <u>, <s>
//   RichDoc::from_html(s) -> RichDoc a tolerant reader of what browsers and office programs
//                                    put on the clipboard: blocks (p, div, h1..h6, li, ul / ol
//                                    nesting, blockquote, pre, tr, br), inline formats (b /
//                                    strong, i / em, u / ins, s / strike / del, a href, font,
//                                    code, CSS in style attributes: color, background(-color),
//                                    font-weight, font-style, text-decoration, font-size in
//                                    pt / px / em / % / keywords, font-family, text-align),
//                                    entities; script, style and head are skipped
//   (crate) clip_html(paras, font, size) the clipboard's HTML: to_html plus the fragment as
//                                    JSON in a comment (<!--jfrich ...-->), so a paste into a
//                                    richtext keeps every attribute exactly
//   (crate) html_read(s) -> (paras, HExt)   the same reader for the Markdown / HTML viewer
//                                    (md_doc.rs): the paragraphs and, by paragraph, what a
//                                    viewer shows besides them -- rules, images (U+FFFC in the
//                                    text), preformatted blocks and their language, the heading
//                                    level, the quotation depth, table rows and their cells, the
//                                    numbers of list items
//
// Whitespace outside <pre> / white-space:pre* collapses as in browsers; a paragraph is a
// block or the text between two <br>.  Nesting is capped (NEST_MAX open blocks, formats and
// lists: deeper tags count as text-level), so no input costs more than linear time.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::rich_doc::*;

fn esc(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

// the CSS of a font family: quoted unless generic
fn css_family(f: &str) -> String {
    if matches!(f, "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" | "system-ui") {
        String::from(f)
    } else {
        format!("'{}'", f.replace('\'', ""))
    }
}

fn run_html(t: &str, f: &Fmt, out: &mut String) {
    let mut close: Vec<&str> = Vec::new();
    if !f.link.is_empty() {
        out.push_str("<a href=\"");
        esc(&f.link, out);
        out.push_str("\">");
        close.push("</a>");
    }
    let mut css = String::new();
    if f.color != 0 {
        css.push_str(&format!("color:{};", color_hex(f.color)));
    }
    if f.bg != 0 {
        css.push_str(&format!("background-color:{};", color_hex(f.bg)));
    }
    if f.size != 0 {
        css.push_str(&format!("font-size:{}pt;", size_text(f.size)));
    }
    if !f.font.is_empty() {
        css.push_str(&format!("font-family:{};", css_family(&f.font)));
    }
    if !css.is_empty() {
        css.pop();
        out.push_str("<span style=\"");
        esc(&css, out);
        out.push_str("\">");
        close.push("</span>");
    }
    for (bit, open, end) in [(RF_BOLD, "<b>", "</b>"), (RF_ITALIC, "<i>", "</i>"), (RF_UNDER, "<u>", "</u>"), (RF_STRIKE, "<s>", "</s>")] {
        if f.flags & bit != 0 {
            out.push_str(open);
            close.push(end);
        }
    }
    esc(t, out);
    for c in close.iter().rev() {
        out.push_str(c);
    }
}

fn para_content(p: &Para, out: &mut String) {
    if p.is_empty() {
        out.push_str("<br>");
        return;
    }
    for (t, f) in p.runs() {
        run_html(t, f, out);
    }
}

fn align_css(a: u8) -> &'static str {
    match a {
        RA_CENTER => "text-align:center",
        RA_RIGHT => "text-align:right",
        RA_JUSTIFY => "text-align:justify",
        _ => "",
    }
}

/// the paragraphs as an HTML fragment (font: the document's family, size in tenths of pt)
pub(crate) fn html_of(paras: &[Para], font: &str, size: u16) -> String {
    let mut out = String::from("<div style=\"white-space:pre-wrap");
    if !font.is_empty() {
        out.push_str(";font-family:");
        esc(&css_family(font), &mut out);
    }
    if size != 0 {
        out.push_str(&format!(";font-size:{}pt", size_text(size)));
    }
    out.push_str("\">");
    // open lists: (kind, an <li> open)
    let mut lists: Vec<(u8, bool)> = Vec::new();
    let close_to = |lists: &mut Vec<(u8, bool)>, n: usize, out: &mut String| {
        while lists.len() > n {
            let (k, li) = lists.pop().unwrap();
            if li {
                out.push_str("</li>");
            }
            out.push_str(if k == LS_NUMBER { "</ol>" } else { "</ul>" });
        }
    };
    for p in paras {
        if p.list == LS_NONE {
            close_to(&mut lists, 0, &mut out);
            let (tag, quote) = match p.style {
                PS_H1 => ("h1", false),
                PS_H2 => ("h2", false),
                PS_H3 => ("h3", false),
                PS_QUOTE => ("p", true),
                _ => ("p", false),
            };
            if quote {
                out.push_str("<blockquote>");
            }
            let mut css = String::from(align_css(p.align));
            if p.level > 0 {
                if !css.is_empty() {
                    css.push(';');
                }
                css.push_str(&format!("margin-left:{}em", p.level as u32 * 2));
            }
            out.push('<');
            out.push_str(tag);
            if !css.is_empty() {
                out.push_str(" style=\"");
                out.push_str(&css);
                out.push('"');
            }
            out.push('>');
            para_content(p, &mut out);
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
            if quote {
                out.push_str("</blockquote>");
            }
            continue;
        }
        let depth = p.level as usize + 1;
        close_to(&mut lists, depth, &mut out);
        if lists.len() == depth && lists[depth - 1].0 != p.list {
            close_to(&mut lists, depth - 1, &mut out);
        }
        if lists.len() == depth {
            if lists[depth - 1].1 {
                out.push_str("</li>");
                lists[depth - 1].1 = false;
            }
        }
        while lists.len() < depth {
            let d = lists.len();
            let kind = if d + 1 == depth { p.list } else { LS_BULLET };
            out.push_str(if kind == LS_NUMBER {
                match d % 3 {
                    0 => "<ol>",
                    1 => "<ol type=\"a\">",
                    _ => "<ol type=\"i\">",
                }
            } else {
                "<ul>"
            });
            lists.push((kind, false));
            if d + 1 < depth {
                // a deeper item without one above it: an empty item holds the list
                out.push_str("<li>");
                lists[d].1 = true;
            }
        }
        out.push_str("<li");
        let css = align_css(p.align);
        if !css.is_empty() {
            out.push_str(" style=\"");
            out.push_str(css);
            out.push('"');
        }
        out.push('>');
        para_content(p, &mut out);
        lists[depth - 1].1 = true;
    }
    close_to(&mut lists, 0, &mut out);
    out.push_str("</div>");
    out
}

/// the clipboard's HTML: html_of with the exact fragment as JSON in a leading comment
pub(crate) fn clip_html(paras: &[Para], font: &str, size: u16) -> String {
    let mut d = RichDoc::from_paras(paras.to_vec());
    d.font = String::from(font);
    d.size = size;
    // a comment cannot hold "--": inside JSON strings "--" is the same text
    let json = d.to_json().replace("--", "-\\u002d");
    format!("<!--jfrich {}-->{}", json, html_of(paras, font, size))
}

impl RichDoc {
    /// the document as an HTML fragment (see rich_html.rs)
    pub fn to_html(&self) -> String {
        html_of(self.paras(), &self.font, self.size)
    }

    /// a document from HTML (see rich_html.rs); a fragment of a richtext's clipboard is
    /// taken exactly
    pub fn from_html(s: &str) -> RichDoc {
        if let Some(d) = embedded(s) {
            return d;
        }
        let mut r = Reader::new();
        r.run(s);
        RichDoc::from_paras(r.finish())
    }
}

// the JSON of clip_html's comment
fn embedded(s: &str) -> Option<RichDoc> {
    let i = s.find("<!--jfrich ")?;
    let rest = &s[i + 11..];
    let j = rest.find("-->")?;
    RichDoc::from_json(&rest[..j])
}

/// (md_doc.rs) the paragraphs of HTML and what a viewer shows besides them (see the module
/// header)
pub(crate) fn html_read(s: &str) -> (Vec<Para>, Vec<(usize, HExt)>) {
    let mut r = Reader::new();
    r.ext = Some(Ext::default());
    r.run(s);
    let ext = r.ext.take().map(|e| e.items).unwrap_or_default();
    let mut paras = r.finish();
    if paras.len() == 1 && paras[0].is_empty() && ext.is_empty() {
        paras.clear();
    }
    (paras, ext)
}

/// what html_read reports about paragraph `at` (the index the item is filed under)
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HExt {
    /// a horizontal rule before the paragraph
    Rule,
    /// an image: U+FFFC at `byte` of the paragraph (width / height: attributes, 0 none)
    Image { byte: usize, src: String, alt: String, w: u32, h: u32 },
    /// the paragraph is a line of preformatted block `block` (language from a class
    /// "language-x" of its <code>)
    Pre { block: u32, lang: String },
    /// the paragraph is a heading of this level (1 ..= 6)
    Heading(u8),
    /// the paragraph is inside this many block quotes
    Quote(u8),
    /// the paragraph is a row of table `table` (header row); its cells start at these bytes
    /// ('\t' between them) with their alignment (0 none, 1 left, 2 centre, 3 right)
    Row { table: u32, head: bool, cells: Vec<(usize, u8)> },
    /// the paragraph begins a list item (its number in an ordered list, else 0)
    Item(u32),
}

// the reader's extra state for html_read
#[derive(Default)]
struct Ext {
    items: Vec<(usize, HExt)>,
    pre: u32,
    lang: String,
    table: u32,
    cells: Vec<(usize, u8)>,
    head: bool,
    thead: bool,
    item: Option<u32>,
    // per open list: the next number (0: bullets)
    nums: Vec<u32>,
    // inside a table cell
    cell: bool,
}

// open blocks, formats and lists at most (deeper ones are not opened)
const NEST_MAX: usize = 64;

// ------------------------------------------------------------------ the reader

pub(crate) fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let v = if let Some(h) = num.strip_prefix(['x', 'X']) { u32::from_str_radix(h, 16).ok()? } else { num.parse().ok()? };
        return char::from_u32(v).map(|c| if c == '\0' { '\u{FFFD}' } else { c });
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{A0}',
        "ensp" => '\u{2002}',
        "emsp" => '\u{2003}',
        "thinsp" => '\u{2009}',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "sbquo" => '‚',
        "ldquo" => '“',
        "rdquo" => '”',
        "bdquo" => '„',
        "laquo" => '«',
        "raquo" => '»',
        "bull" => '•',
        "middot" => '·',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "euro" => '€',
        "deg" => '°',
        "times" => '×',
        "divide" => '÷',
        "auml" => 'ä',
        "ouml" => 'ö',
        "uuml" => 'ü',
        "Auml" => 'Ä',
        "Ouml" => 'Ö',
        "Uuml" => 'Ü',
        "szlig" => 'ß',
        "eacute" => 'é',
        "egrave" => 'è',
        "agrave" => 'à',
        "ccedil" => 'ç',
        "shy" => '\u{AD}',
        _ => return None,
    })
}

// text with its entities decoded
fn decode(s: &str) -> String {
    if !s.contains('&') {
        return String::from(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[1..].find(|c: char| c == ';' || c == '&' || c == '<' || c.is_whitespace()).map(|j| j + 1);
        match end {
            Some(j) if rest.as_bytes()[j] == b';' && j <= 12 => match entity(&rest[1..j]) {
                Some(c) => {
                    out.push(c);
                    rest = &rest[j + 1..];
                }
                None => {
                    out.push('&');
                    rest = &rest[1..];
                }
            },
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// the attributes of a tag: (name lower-case, value decoded)
pub(crate) fn attrs(s: &str) -> Vec<(String, String)> {
    let b = s.as_bytes();
    let mut v = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let n0 = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        if n0 == i {
            break;
        }
        let name = s[n0..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let v0 = i + 1;
                i += 1;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                val = decode(&s[v0..i.min(b.len())]);
                i += 1;
            } else {
                let v0 = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                val = decode(&s[v0..i]);
            }
        }
        v.push((name, val));
    }
    v
}

// a CSS length as tenths of a point (em / %: of `base`)
fn css_size(v: &str, base: u16) -> Option<u16> {
    let v = v.trim().to_ascii_lowercase();
    let num = |s: &str| -> Option<f32> { s.trim().parse::<f32>().ok() };
    let base = if base == 0 { 110 } else { base } as f32;
    let t = if let Some(n) = v.strip_suffix("pt") {
        num(n)? * 10.0
    } else if let Some(n) = v.strip_suffix("px") {
        num(n)? * 7.5
    } else if let Some(n) = v.strip_suffix("rem") {
        num(n)? * 110.0
    } else if let Some(n) = v.strip_suffix("em") {
        num(n)? * base
    } else if let Some(n) = v.strip_suffix('%') {
        num(n)? * base / 100.0
    } else {
        match v.as_str() {
            "xx-small" => 70.0,
            "x-small" => 75.0,
            "small" => 100.0,
            "medium" => 120.0,
            "large" => 135.0,
            "x-large" => 180.0,
            "xx-large" => 240.0,
            "smaller" => base * 0.83,
            "larger" => base * 1.2,
            _ => num(&v)? * 7.5,
        }
    };
    (t >= 10.0 && t <= 4000.0).then_some((t + 0.5) as u16)
}

// a CSS colour (#hex, rgb(), rgba(), a few names) -> 0xAARRGGBB
fn css_color(v: &str) -> Option<u32> {
    let v = v.trim().to_ascii_lowercase();
    if v.starts_with('#') {
        return parse_hex_color(&v);
    }
    if let Some(inner) = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")) {
        let inner = inner.trim_end_matches(')');
        let parts: Vec<&str> = inner.split([',', ' ', '/']).filter(|p| !p.is_empty()).collect();
        if parts.len() < 3 {
            return None;
        }
        let c = |s: &str| -> Option<u32> {
            if let Some(p) = s.strip_suffix('%') {
                Some((p.trim().parse::<f32>().ok()? * 2.55).clamp(0.0, 255.0) as u32)
            } else {
                Some(s.trim().parse::<f32>().ok()?.clamp(0.0, 255.0) as u32)
            }
        };
        let a = match parts.get(3) {
            Some(s) if s.ends_with('%') => (s.trim_end_matches('%').parse::<f32>().ok()? * 2.55) as u32,
            Some(s) => (s.parse::<f32>().ok()? * 255.0) as u32,
            None => 255,
        };
        if a == 0 {
            return None; // transparent: no colour
        }
        return Some(a.min(255) << 24 | c(parts[0])? << 16 | c(parts[1])? << 8 | c(parts[2])?);
    }
    let rgb = match v.as_str() {
        "black" => 0x000000,
        "white" => 0xFFFFFF,
        "red" => 0xFF0000,
        "green" => 0x008000,
        "blue" => 0x0000FF,
        "yellow" => 0xFFFF00,
        "orange" => 0xFFA500,
        "purple" => 0x800080,
        "gray" | "grey" => 0x808080,
        "silver" => 0xC0C0C0,
        "maroon" => 0x800000,
        "navy" => 0x000080,
        "teal" => 0x008080,
        "olive" => 0x808000,
        "lime" => 0x00FF00,
        "aqua" | "cyan" => 0x00FFFF,
        "fuchsia" | "magenta" => 0xFF00FF,
        _ => return None,
    };
    Some(0xFF00_0000 | rgb)
}

// a block's attributes and text settings
#[derive(Clone)]
struct Block {
    tag: String,
    align: u8,
    style: u8,
    pre: bool,
}

struct Reader {
    out: Vec<Para>,
    cur: Para,
    // the paragraph holds something (text or an explicit break)
    started: bool,
    // the last character added was a collapsed blank
    space: bool,
    fmts: Vec<(String, Fmt)>,
    blocks: Vec<Block>,
    // open lists: LS_*
    lists: Vec<u8>,
    // the paragraph is a list item
    item: Option<(u8, u8)>,
    skip: Option<String>,
    // html_read: what a viewer shows besides the paragraphs
    ext: Option<Ext>,
}

const BLOCKS: [&str; 22] = [
    "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "blockquote", "pre", "tr", "dt", "dd", "section", "article", "header", "footer",
    "table", "address", "figure", "center",
];

impl Reader {
    fn new() -> Reader {
        Reader {
            out: Vec::new(),
            cur: Para::new(),
            started: false,
            space: false,
            fmts: Vec::new(),
            blocks: Vec::new(),
            lists: Vec::new(),
            item: None,
            skip: None,
            ext: None,
        }
    }

    fn fmt(&self) -> Fmt {
        self.fmts.last().map(|f| f.1.clone()).unwrap_or_default()
    }

    fn pre(&self) -> bool {
        self.blocks.iter().any(|b| b.pre)
    }

    // finish the current paragraph (an empty one only when forced)
    fn flush(&mut self, force: bool) {
        if !self.started && !force {
            return;
        }
        let mut p = core::mem::take(&mut self.cur);
        // trailing collapsed blanks go
        if self.space && !p.is_empty() {
            let t: String = String::from(p.text().trim_end_matches(' '));
            let mut q = Para::new();
            for (s, e, f) in p.spans() {
                let e = e.min(t.len());
                if s < e {
                    q.push(&t[s..e], f);
                }
            }
            q.mark = p.mark.clone();
            p = q;
        }
        let b = self.blocks.last();
        p.align = self.blocks.iter().rev().map(|b| b.align).find(|&a| a != RA_LEFT).unwrap_or(RA_LEFT);
        p.style = b.map_or(PS_NORMAL, |_| self.blocks.iter().rev().map(|b| b.style).find(|&s| s != PS_NORMAL).unwrap_or(PS_NORMAL));
        if let Some((kind, level)) = self.item {
            p.list = kind;
            p.level = level;
        }
        p.mark = self.fmt();
        if self.ext.is_some() {
            self.ext_para();
        }
        self.out.push(p);
        self.cur = Para::new();
        self.started = false;
        self.space = false;
    }

    // html_read: what the paragraph about to be filed is besides its text
    fn ext_para(&mut self) {
        let at = self.out.len();
        let pre = self.pre();
        let level = self.blocks.iter().rev().find_map(|b| {
            let t = b.tag.as_bytes();
            (t.len() == 2 && t[0] == b'h' && (b'1'..=b'6').contains(&t[1])).then(|| t[1] - b'0')
        });
        let quote = self.blocks.iter().filter(|b| b.tag == "blockquote").count().min(255) as u8;
        let row = self.blocks.last().is_some_and(|b| b.tag == "tr");
        let Some(e) = self.ext.as_mut() else { return };
        if pre {
            e.items.push((at, HExt::Pre { block: e.pre, lang: e.lang.clone() }));
        }
        if let Some(l) = level {
            e.items.push((at, HExt::Heading(l)));
        }
        if quote > 0 {
            e.items.push((at, HExt::Quote(quote)));
        }
        if let Some(n) = e.item.take() {
            e.items.push((at, HExt::Item(n)));
        }
        if row && !e.cells.is_empty() {
            let cells = core::mem::take(&mut e.cells);
            e.items.push((at, HExt::Row { table: e.table, head: e.head || e.thead, cells }));
        }
        e.cells.clear();
        e.head = false;
    }

    // html_read: an element of a viewer (img, hr, cells, pre, code, lists, tables)
    fn ext_open(&mut self, tag: &str, at: &[(String, String)]) {
        let attr = |k: &str| at.iter().find(|a| a.0 == k).map(|a| a.1.as_str());
        let num = |k: &str| attr(k).and_then(|v| v.trim().trim_end_matches("px").parse::<u32>().ok()).unwrap_or(0).min(100_000);
        match tag {
            "img" => {
                let f = self.fmt();
                let byte = self.cur.len();
                self.cur.push("\u{FFFC}", &f);
                self.started = true;
                self.space = false;
                let (src, alt) = (String::from(attr("src").unwrap_or("")), String::from(attr("alt").unwrap_or("")));
                let (w, h) = (num("width"), num("height"));
                let n = self.out.len();
                if let Some(e) = self.ext.as_mut() {
                    e.items.push((n, HExt::Image { byte, src, alt, w, h }));
                }
            }
            "td" | "th" => {
                let f = self.fmt();
                let first = self.ext.as_ref().is_none_or(|e| e.cells.is_empty());
                if !first {
                    self.cur.push("\t", &f);
                }
                self.started = true;
                self.space = false;
                let byte = self.cur.len();
                let a = attr("align").or_else(|| {
                    attr("style").and_then(|v| v.split(';').filter_map(|d| d.split_once(':')).find(|(p, _)| p.trim() == "text-align").map(|(_, x)| x.trim()))
                });
                let align = match a.map(|s| s.to_ascii_lowercase()).as_deref() {
                    Some("left") | Some("start") => 1,
                    Some("center") => 2,
                    Some("right") | Some("end") => 3,
                    _ => 0,
                };
                if let Some(e) = self.ext.as_mut() {
                    e.cells.push((byte, align));
                    e.head |= tag == "th";
                    e.cell = true;
                }
            }
            _ => {
                let Some(e) = self.ext.as_mut() else { return };
                match tag {
                    "table" => {
                        e.table += 1;
                        e.thead = false;
                    }
                    "thead" => e.thead = true,
                    "tbody" | "tfoot" => e.thead = false,
                    "pre" => {
                        e.pre += 1;
                        e.lang.clear();
                    }
                    "code" => {
                        if let Some(c) = attr("class") {
                            for w in c.split_whitespace() {
                                if let Some(l) = w.strip_prefix("language-").or_else(|| w.strip_prefix("lang-")) {
                                    e.lang = String::from(l);
                                }
                            }
                        }
                    }
                    "ol" => {
                        let start = attr("start").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1);
                        e.nums.push(start.min(999_999_999));
                    }
                    "ul" => e.nums.push(0),
                    "li" => {
                        let n = match e.nums.last_mut() {
                            Some(n) if *n > 0 => {
                                let v = *n;
                                *n = n.saturating_add(1);
                                v
                            }
                            _ => 0,
                        };
                        e.item = Some(n);
                    }
                    _ => {}
                }
            }
        }
    }

    fn text(&mut self, raw: &str) {
        let t = decode(raw);
        if self.pre() {
            let f = self.fmt();
            let mut first = true;
            for line in lines_of(&t) {
                if !first {
                    self.flush(true);
                }
                first = false;
                self.cur.push(&line.replace('\u{A0}', " "), &f);
                self.started = true;
            }
            return;
        }
        let mut s = String::new();
        for c in t.chars() {
            if c.is_whitespace() && c != '\u{A0}' {
                let at_start = !self.started && s.is_empty();
                if !self.space && !at_start {
                    s.push(' ');
                    self.space = true;
                }
            } else {
                s.push(if c == '\u{A0}' { ' ' } else { c });
                self.space = false;
            }
        }
        if !s.is_empty() {
            let f = self.fmt();
            self.cur.push(&s, &f);
            self.started = true;
        }
    }

    // the format an element's attributes give (on top of the current one)
    fn fmt_for(&self, tag: &str, at: &[(String, String)]) -> Fmt {
        let mut f = self.fmt();
        match tag {
            "b" | "strong" => f.flags |= RF_BOLD,
            "i" | "em" | "cite" | "var" | "dfn" => f.flags |= RF_ITALIC,
            "u" | "ins" => f.flags |= RF_UNDER,
            "s" | "strike" | "del" => f.flags |= RF_STRIKE,
            "code" | "tt" | "kbd" | "samp" => f.font = String::from("monospace"),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "th" => {}
            _ => {}
        }
        for (k, v) in at {
            match k.as_str() {
                "href" if tag == "a" => f.link = v.clone(),
                "color" if tag == "font" => f.color = css_color(v).unwrap_or(f.color),
                "face" if tag == "font" => f.font = family_of(v),
                "size" if tag == "font" => {
                    f.size = match v.trim() {
                        "1" => 75,
                        "2" => 100,
                        "3" => 120,
                        "4" => 135,
                        "5" => 180,
                        "6" => 240,
                        "7" => 360,
                        _ => f.size,
                    }
                }
                "style" => {
                    for decl in v.split(';') {
                        let Some((p, val)) = decl.split_once(':') else { continue };
                        let val = val.trim().trim_end_matches("!important").trim();
                        match p.trim().to_ascii_lowercase().as_str() {
                            "color" => f.color = css_color(val).unwrap_or(f.color),
                            "background-color" | "background" => {
                                if let Some(c) = val.split_whitespace().find_map(css_color) {
                                    if c & 0xFF_FFFF != 0xFF_FFFF {
                                        f.bg = c;
                                    }
                                }
                            }
                            "font-weight" => {
                                let bold = matches!(val, "bold" | "bolder") || val.parse::<u32>().is_ok_and(|w| w >= 600);
                                if bold {
                                    f.flags |= RF_BOLD;
                                } else if matches!(val, "normal" | "lighter") || val.parse::<u32>().is_ok_and(|w| w < 600) {
                                    f.flags &= !RF_BOLD;
                                }
                            }
                            "font-style" => {
                                if val.starts_with("italic") || val.starts_with("oblique") {
                                    f.flags |= RF_ITALIC;
                                } else if val == "normal" {
                                    f.flags &= !RF_ITALIC;
                                }
                            }
                            "text-decoration" | "text-decoration-line" => {
                                if val.contains("underline") {
                                    f.flags |= RF_UNDER;
                                }
                                if val.contains("line-through") {
                                    f.flags |= RF_STRIKE;
                                }
                                if val == "none" {
                                    f.flags &= !(RF_UNDER | RF_STRIKE);
                                }
                            }
                            "font-size" => f.size = css_size(val, f.size).unwrap_or(f.size),
                            "font-family" => f.font = family_of(val),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        f
    }

    // html_read inside a table cell: blocks, lists and breaks are part of the cell's text
    fn in_cell(&self, tag: &str) -> bool {
        self.ext.as_ref().is_some_and(|e| e.cell) && !matches!(tag, "tr" | "td" | "th" | "table" | "thead" | "tbody" | "tfoot")
    }

    fn open(&mut self, tag: &str, at: &[(String, String)], selfclose: bool) {
        if self.in_cell(tag) && (BLOCKS.contains(&tag) || matches!(tag, "br" | "hr" | "ul" | "ol")) {
            if !self.space && self.started {
                self.text(" ");
            }
            return;
        }
        match tag {
            "script" | "style" | "head" | "title" | "template" | "noscript" | "svg" => {
                if !selfclose {
                    self.skip = Some(String::from(tag));
                }
                return;
            }
            "br" => {
                self.flush(true);
                return;
            }
            "img" | "hr" | "meta" | "link" | "input" => {
                if tag == "hr" {
                    self.flush(false);
                    if let Some(e) = self.ext.as_mut() {
                        e.items.push((self.out.len(), HExt::Rule));
                    }
                } else if tag == "img" && self.ext.is_some() {
                    self.ext_open(tag, at);
                }
                return;
            }
            "ul" | "ol" => {
                self.flush(false);
                if self.lists.len() < NEST_MAX {
                    self.lists.push(if tag == "ol" { LS_NUMBER } else { LS_BULLET });
                    if self.ext.is_some() {
                        self.ext_open(tag, at);
                    }
                }
                return;
            }
            _ => {}
        }
        // deeper than NEST_MAX: the tag counts as nothing (its text stays)
        if self.blocks.len() >= NEST_MAX || self.fmts.len() >= NEST_MAX {
            return;
        }
        let block = BLOCKS.contains(&tag);
        if self.ext.is_some() && matches!(tag, "table" | "thead" | "tbody" | "tfoot" | "code") {
            self.ext_open(tag, at);
        }
        if block {
            self.flush(false);
            let mut align = RA_LEFT;
            for (k, v) in at {
                let v = v.to_ascii_lowercase();
                let a = if k == "align" {
                    Some(v.as_str())
                } else if k == "style" {
                    v.split(';').filter_map(|d| d.split_once(':')).find(|(p, _)| p.trim() == "text-align").map(|(_, x)| x.trim())
                } else {
                    None
                };
                align = match a {
                    Some("center") => RA_CENTER,
                    Some("right") | Some("end") => RA_RIGHT,
                    Some("justify") => RA_JUSTIFY,
                    _ => align,
                };
            }
            if tag == "center" {
                align = RA_CENTER;
            }
            let style = match tag {
                "h1" => PS_H1,
                "h2" => PS_H2,
                "h3" | "h4" | "h5" | "h6" => PS_H3,
                "blockquote" => PS_QUOTE,
                _ => PS_NORMAL,
            };
            let pre = tag == "pre"
                || at.iter().any(|(k, v)| k == "style" && v.to_ascii_lowercase().replace(' ', "").contains("white-space:pre"));
            self.blocks.push(Block { tag: String::from(tag), align, style, pre });
            if tag == "li" {
                let kind = self.lists.last().copied().unwrap_or(LS_BULLET);
                self.item = Some((kind, (self.lists.len().max(1) - 1).min(LEVEL_MAX as usize) as u8));
            }
            if self.ext.is_some() && matches!(tag, "pre" | "li") {
                self.ext_open(tag, at);
            }
        }
        if self.ext.is_some() && matches!(tag, "td" | "th") {
            self.ext_open(tag, at);
        }
        if selfclose {
            if block {
                self.close(tag);
            }
            return;
        }
        let f = self.fmt_for(tag, at);
        self.fmts.push((String::from(tag), f));
    }

    fn close(&mut self, tag: &str) {
        if let Some(i) = self.fmts.iter().rposition(|f| f.0 == tag) {
            self.fmts.truncate(i);
        }
        if self.in_cell(tag) && (BLOCKS.contains(&tag) || matches!(tag, "ul" | "ol")) {
            return;
        }
        if let Some(e) = self.ext.as_mut() {
            if matches!(tag, "td" | "th" | "tr" | "table") {
                e.cell = false;
            }
        }
        match tag {
            "ul" | "ol" => {
                self.flush(false);
                self.lists.pop();
                if self.lists.is_empty() {
                    self.item = None;
                }
                if let Some(e) = self.ext.as_mut() {
                    e.nums.pop();
                }
                return;
            }
            "thead" => {
                if let Some(e) = self.ext.as_mut() {
                    e.thead = false;
                }
            }
            _ => {}
        }
        if BLOCKS.contains(&tag) {
            self.flush(false);
            if let Some(i) = self.blocks.iter().rposition(|b| b.tag == tag) {
                self.blocks.truncate(i);
            }
            if tag == "li" {
                self.item = None;
            }
        }
    }

    fn run(&mut self, s: &str) {
        let mut rest = s;
        while !rest.is_empty() {
            if let Some(tag) = &self.skip {
                let close = format!("</{}", tag);
                match find_ci(rest, &close) {
                    Some(i) => {
                        rest = &rest[i..];
                        self.skip = None;
                    }
                    None => return,
                }
            }
            let Some(lt) = rest.find('<') else {
                self.text(rest);
                return;
            };
            if lt > 0 {
                self.text(&rest[..lt]);
            }
            rest = &rest[lt..];
            if rest.starts_with("<!--") {
                rest = rest.find("-->").map_or("", |j| &rest[j + 3..]);
                continue;
            }
            if rest.starts_with("<!") || rest.starts_with("<?") {
                rest = rest.find('>').map_or("", |j| &rest[j + 1..]);
                continue;
            }
            let Some(gt) = tag_end(rest) else {
                self.text(rest);
                return;
            };
            let inner = &rest[1..gt];
            rest = &rest[gt + 1..];
            if let Some(name) = inner.strip_prefix('/') {
                let name = name.trim().split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                self.close(&name);
                continue;
            }
            let name_end = inner.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(inner.len());
            let name = inner[..name_end].to_ascii_lowercase();
            if name.is_empty() || !name.as_bytes()[0].is_ascii_alphabetic() {
                self.text("<");
                self.text(inner);
                self.text(">");
                continue;
            }
            let selfclose = inner.trim_end().ends_with('/');
            let at = attrs(&inner[name_end..]);
            self.open(&name, &at, selfclose);
        }
    }

    fn finish(mut self) -> Vec<Para> {
        self.flush(false);
        // trailing empty paragraphs of a closing <br>
        while self.out.len() > 1 && self.out.last().is_some_and(|p| p.is_empty()) {
            self.out.pop();
        }
        self.out
    }
}

// the '>' that ends the tag at s[0] == '<' (quotes respected)
fn tag_end(s: &str) -> Option<usize> {
    let mut q = 0u8;
    for (i, b) in s.bytes().enumerate().skip(1) {
        if q != 0 {
            if b == q {
                q = 0;
            }
        } else if b == b'"' || b == b'\'' {
            q = b;
        } else if b == b'>' {
            return Some(i);
        }
    }
    None
}

fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.len() > h.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

// the first family of a CSS font-family list
fn family_of(v: &str) -> String {
    let f = v.split(',').next().unwrap_or("").trim().trim_matches(['\'', '"']).trim();
    String::from(f)
}
