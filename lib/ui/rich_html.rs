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
//   (crate) html_read(s) -> HRead   the same reader for the Markdown / HTML viewer
//                                    (md_doc.rs): the paragraphs and, by paragraph, what a
//                                    viewer shows besides them -- rules, images and form fields
//                                    (U+FFFC in the text), preformatted blocks and their
//                                    language, the heading level, the quotation depth, table
//                                    rows and their cells, the numbers of list items, task
//                                    boxes; the forms and their fields (md_doc.rs "Forms").
//                                    It reads the element tree of the HTML (md_dom.rs): the
//                                    tags as the tree has them, elements with the hidden
//                                    attribute or display:none left out
//   (crate) dom_read(dom) / dom_para(dom, l)   the same for a tree of a document (MdDoc::html,
//                                    its changes): the paragraphs, fields and forms know their
//                                    elements; one block element's paragraph read again
//
// Style sheets (the viewer's reader only; RichDoc::from_html takes the style attributes as
// before): the <style> elements of the tree (media screen / all, type text/css) in document
// order give rules -- selectors of md_dom.rs (tag, *, #id, .class, [attr], descendant and
// child) with declarations; comments, at-rules and rules whose selector is not in the subset
// are left out.  An element's style is the cascade: the declarations of the rules that match
// by specificity and source order, its style attribute's, then the !important ones the same
// way (of each property the last).  The text's properties are inherited as the formats are
// (colour, weight, style, decoration, size, family, alignment); a link takes the link colour
// unless a rule for it gives one.  A block element (and ul, ol, nav, form ...) whose style
// gives margins, padding, borders (width, a visible style -- drawn solid -- and colour, per
// side) or a background gets a box (md_doc.rs MdBox; its background is the box's, not the
// text's).  display:none hides.  Rules are indexed by the id, class or tag their subject needs;
// matching has a budget proportional to the tree and the sheets (then no rule matches), at
// most RULES_MAX rules.
//
// Whitespace outside <pre> / white-space:pre* collapses as in browsers; a paragraph is a
// block or the text between two <br>.  Nesting is capped (NEST_MAX open blocks, formats and
// lists: deeper tags count as text-level), so no input costs more than linear time.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::md_doc::{MdBox, MdField, MdForm, MdOption, MF_BUTTON, MF_CHECKBOX, MF_HIDDEN, MF_PASSWORD, MF_RADIO, MF_RESET, MF_SELECT, MF_SUBMIT, MF_TEXT, MF_TEXTAREA};
use crate::md_dom::{parse_sels, sel_match, void, Dom, MdNode, Sel, SelKey, K_ELEM, K_TEXT, NIL};
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
pub(crate) fn html_read(s: &str) -> HRead {
    let d = Dom::parse(s);
    let sheet = Sheet::of(&d);
    dom_read(&d, &sheet, false)
}

/// (md_doc.rs) html_read of a document's tree with its style sheets (handles: the paragraphs,
/// fields and forms name their elements, HRead::ranges the paragraphs of the block elements)
pub(crate) fn dom_read(d: &Dom, sheet: &Sheet, handles: bool) -> HRead {
    let mut r = Reader::new();
    r.ext = Some(Ext::default());
    r.css = sheet.budget(d.nodes.len());
    r.walk(d, 0, false, handles, sheet);
    r.cap_end();
    r.label_end();
    let mut e = r.ext.take().unwrap_or_default();
    // label for= and form= by id (the first element with it)
    let mut ids: BTreeMap<String, usize> = BTreeMap::new();
    for (i, f) in e.fields.iter().enumerate().rev() {
        if !f.id.is_empty() {
            ids.insert(f.id.clone(), i);
        }
    }
    for (id, text) in core::mem::take(&mut e.labels) {
        if let Some(f) = ids.get(&id).and_then(|&i| e.fields.get_mut(i)) {
            if f.label.is_empty() && !matches!(f.kind, MF_SUBMIT | MF_RESET | MF_BUTTON) {
                f.label = text;
            }
        }
    }
    ids.clear();
    for (i, f) in e.forms.iter().enumerate().rev() {
        if !f.id.is_empty() {
            ids.insert(f.id.clone(), i);
        }
    }
    for (k, id) in core::mem::take(&mut e.form_of) {
        if let (Some(&i), Some(f)) = (ids.get(&id), e.fields.get_mut(k)) {
            f.form = i as u32 + 1;
        }
    }
    for (k, t) in core::mem::take(&mut e.titles) {
        if let Some(f) = e.fields.get_mut(k).filter(|f| f.label.is_empty()) {
            f.label = t;
        }
    }
    let mut paras = r.finish();
    if paras.len() == 1 && paras[0].is_empty() && e.items.is_empty() {
        paras.clear();
    }
    HRead {
        paras,
        ext: e.items,
        forms: e.forms,
        fields: e.fields,
        nodes: e.para_nodes,
        ranges: e.ranges,
        boxes: e.boxes,
        para_boxes: e.para_boxes,
        rule_boxes: e.rule_boxes,
    }
}

/// (md_doc.rs) the paragraph block element l of a document's tree shows by itself, read again
/// with the format its ancestors give it (None: it shows none or several)
pub(crate) fn dom_para(d: &Dom, sheet: &Sheet, l: u32) -> Option<Para> {
    let mut r = Reader::new();
    r.ext = Some(Ext::default());
    r.css = sheet.budget(64);
    let mut chain = Vec::new();
    let mut x = d.nodes[l as usize].parent;
    while x != NIL {
        chain.push(x);
        x = d.nodes[x as usize].parent;
    }
    // (what the ancestors push on the reader's formats)
    let mut f = Fmt::default();
    for &a in chain.iter().rev() {
        let n = &d.nodes[a as usize];
        if n.kind == K_ELEM && pushes_fmt(&n.name) {
            let eff = sheet.attrs_of(d, a, &mut r.css);
            f = fmt_with(f, &n.name, eff.as_deref().unwrap_or(&n.attrs), true);
        }
    }
    r.fmts.push((String::from("#"), f));
    r.walk(d, l, true, true, sheet);
    if r.out.len() != 1 {
        return None;
    }
    r.out.pop()
}

// an element that puts a format on the reader's stack (in a document's tree, outside cells,
// select and button, at a depth below NEST_MAX)
fn pushes_fmt(tag: &str) -> bool {
    !matches!(
        tag,
        "ul" | "ol" | "form" | "label" | "textarea" | "select" | "button" | "br" | "img" | "hr" | "meta" | "link" | "input" | "script" | "style"
            | "head" | "title" | "template" | "noscript" | "svg"
    )
}

// display:none in a style attribute (the last display wins)
fn display_none(style: &str) -> bool {
    let mut none = false;
    for decl in style.split(';') {
        if let Some((p, v)) = decl.split_once(':') {
            if p.trim().eq_ignore_ascii_case("display") {
                none = v.trim().trim_end_matches("!important").trim().eq_ignore_ascii_case("none");
            }
        }
    }
    none
}

/// what html_read returns: the paragraphs, the items by paragraph, the forms and fields
/// (MdField::form: index + 1 into forms; HExt::Field::n: index into fields); dom_read with
/// handles: the element of every paragraph (the innermost block; the last one may be missing:
/// none) and the paragraphs of the block elements (element, first, end); the boxes of CSS, the
/// boxes around every paragraph (indices of boxes, outermost first; the last one may be
/// missing) and around the rules before paragraphs
pub(crate) struct HRead {
    pub paras: Vec<Para>,
    pub ext: Vec<(usize, HExt)>,
    pub forms: Vec<MdForm>,
    pub fields: Vec<MdField>,
    pub nodes: Vec<MdNode>,
    pub ranges: Vec<(u32, u32, u32)>,
    pub boxes: Vec<MdBox>,
    pub para_boxes: Vec<Vec<u32>>,
    pub rule_boxes: Vec<(usize, Vec<u32>)>,
}

/// what html_read reports about paragraph `at` (the index the item is filed under)
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HExt {
    /// a form field: U+FFFC at `byte` of the paragraph (None: a hidden one, no character);
    /// fields[n]
    Field { byte: Option<usize>, n: usize },
    /// the paragraph is a task item: a disabled check box at its start (checked)
    Task(bool),
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
    // forms and their fields; the open form (index)
    forms: Vec<MdForm>,
    fields: Vec<MdField>,
    form: Option<u32>,
    // the open <label>: for=, its text, the field inside it
    label: Option<(String, String, Option<usize>)>,
    // labels for= an id, form= ids and titles of fields (resolved at the end)
    labels: Vec<(String, String)>,
    form_of: Vec<(usize, String)>,
    titles: Vec<(usize, String)>,
    // the field whose content is read: a <select>'s options or a <button>'s caption (raw,
    // collapsed at its end); the option being read has a value attribute
    cap: Option<usize>,
    opt_val: bool,
    // the <textarea> whose text is read raw
    raw: Option<usize>,
    // dom_read with handles: the element of every paragraph, the paragraphs of the block
    // elements (element, first, end)
    para_nodes: Vec<MdNode>,
    ranges: Vec<(u32, u32, u32)>,
    // the boxes of CSS, the open ones (outermost first), the boxes around every paragraph and
    // around the rules (by the paragraph they come before)
    boxes: Vec<MdBox>,
    open_boxes: Vec<u32>,
    para_boxes: Vec<Vec<u32>>,
    rule_boxes: Vec<(usize, Vec<u32>)>,
}

// open blocks, formats and lists at most (deeper ones are not opened)
const NEST_MAX: usize = 64;
// forms, fields, choices of a list at most (more count as nothing)
const FORMS_MAX: usize = 4096;
const FIELDS_MAX: usize = 0xFFFE;
const OPTIONS_MAX: usize = 4096;

// the bytes of a label, caption or choice kept at most
const CAP_TEXT: usize = 8192;

// s + t, at most CAP_TEXT bytes (cut at a character)
fn push_cap(s: &mut String, t: &str) {
    let mut n = CAP_TEXT.saturating_sub(s.len()).min(t.len());
    while !t.is_char_boundary(n) {
        n -= 1;
    }
    s.push_str(&t[..n]);
}

// text with its runs of white space as one blank, none at the ends (a no-break space stays,
// as a blank)
fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for w in s.split(|c: char| c.is_whitespace() && c != '\u{A0}').filter(|w| !w.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&w.replace('\u{A0}', " "));
    }
    out
}

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
pub(crate) fn decode(s: &str) -> String {
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

// a block's attributes and text settings (node: its element in a document's tree)
#[derive(Clone)]
struct Block {
    tag: String,
    align: u8,
    style: u8,
    pre: bool,
    node: MdNode,
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
    // dom_read: the element being opened, what matching the style sheets' rules may still cost
    node: MdNode,
    css: usize,
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
            node: MdNode::NONE,
            css: 0,
        }
    }

    // html_read: the elements of a tree in document order (below `top`, or `top` itself when
    // `own`) as the stream of their tags, each with the style the sheet's cascade gives it;
    // hidden ones (the hidden attribute, display:none) are left out, raw text is skipped but a
    // text area's; a box of CSS (MdBox) around the paragraphs of a block element whose style
    // gives one.  handles: paragraphs, fields and forms name their elements, the block
    // elements' paragraphs are noted
    fn walk(&mut self, d: &Dom, top: u32, own: bool, handles: bool, sheet: &Sheet) {
        // (node, Some(paragraphs before its content, a box opened): its end)
        let mut st: Vec<(u32, Option<(usize, bool)>)> = Vec::new();
        let push_kids = |st: &mut Vec<(u32, Option<(usize, bool)>)>, x: u32| {
            let mut k = d.nodes[x as usize].last;
            while k != NIL {
                st.push((k, None));
                k = d.nodes[k as usize].prev;
            }
        };
        if own {
            st.push((top, None));
        } else {
            push_kids(&mut st, top);
        }
        while let Some((x, end)) = st.pop() {
            let n = &d.nodes[x as usize];
            if let Some((p0, boxed)) = end {
                self.close(&n.name);
                if let Some(e) = self.ext.as_mut() {
                    if boxed {
                        e.open_boxes.pop();
                    }
                    if handles && BLOCKS.contains(&n.name.as_str()) {
                        e.ranges.push((x, p0 as u32, self.out.len() as u32));
                    }
                }
                continue;
            }
            if n.kind == K_TEXT {
                self.text_dec(&n.name);
                continue;
            }
            if n.kind != K_ELEM || d.hidden_attr(x) {
                continue;
            }
            let eff = if sheet.is_empty() { None } else { sheet.attrs_of(d, x, &mut self.css) };
            let at: &[(String, String)] = eff.as_deref().unwrap_or(&n.attrs);
            let style = at.iter().find(|a| a.0 == "style").map(|a| a.1.as_str());
            if style.is_some_and(display_none) {
                continue;
            }
            self.node = if handles { d.handle(x) } else { MdNode::NONE };
            // (a box's place among the quotes and lists: those around its element)
            let depth = self.blocks.iter().filter(|b| b.tag == "blockquote").count() + self.lists.len();
            self.open(&n.name, at, false);
            if void(&n.name) {
                continue;
            }
            if self.skip.is_some() {
                // raw text: a text area's is its value, the rest is skipped
                self.raw_text_dec(&d.text_of(x));
                self.skip = None;
                self.close(&n.name);
                continue;
            }
            let mut boxed = false;
            if let (Some(e), Some(s)) = (self.ext.as_mut(), style) {
                if box_tag(&n.name) {
                    if let Some(mut b) = box_of(s) {
                        b.depth = depth.min(255) as u8;
                        b.node = self.node;
                        e.open_boxes.push(e.boxes.len() as u32);
                        e.boxes.push(b);
                        boxed = true;
                    }
                }
            }
            st.push((x, Some((self.out.len(), boxed))));
            push_kids(&mut st, x);
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
        let node = self.blocks.last().map_or(MdNode::NONE, |b| b.node);
        let Some(e) = self.ext.as_mut() else { return };
        e.para_nodes.push(node);
        e.para_boxes.push(e.open_boxes.clone());
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
        self.text_dec(&decode(raw));
    }

    // text with its entities decoded
    fn text_dec(&mut self, t: &str) {
        // html_read: inside a <select> or <button> the text is the field's; the text of an open
        // <label> names its field
        if let Some(e) = self.ext.as_mut() {
            if let Some(fd) = e.cap.and_then(|f| e.fields.get_mut(f)) {
                if fd.kind != MF_SELECT {
                    push_cap(&mut fd.label, &t);
                } else if let Some(o) = fd.options.last_mut() {
                    push_cap(&mut o.label, &t);
                }
                return;
            }
            if let Some(l) = e.label.as_mut() {
                push_cap(&mut l.1, &t);
            }
        }
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
        fmt_with(self.fmt(), tag, at, self.ext.is_some())
    }

    // html_read inside a table cell: blocks, lists and breaks are part of the cell's text
    fn in_cell(&self, tag: &str) -> bool {
        self.ext.as_ref().is_some_and(|e| e.cell) && !matches!(tag, "tr" | "td" | "th" | "table" | "thead" | "tbody" | "tfoot")
    }

    // html_read: a new field -- its U+FFFC (none for a hidden one) in the paragraph, the open
    // form's, the open label's -> its index (None: FIELDS_MAX reached)
    fn field_new(&mut self, mut fd: MdField, at: &[(String, String)]) -> Option<usize> {
        let attr = |k: &str| at.iter().find(|a| a.0 == k).map(|a| a.1.as_str());
        let n = self.ext.as_ref()?.fields.len();
        if n >= FIELDS_MAX {
            return None;
        }
        fd.name = String::from(attr("name").unwrap_or(""));
        fd.id = String::from(attr("id").unwrap_or("").trim());
        fd.node = self.node;
        fd.disabled = attr("disabled").is_some();
        fd.readonly = attr("readonly").is_some();
        if !matches!(fd.kind, MF_SUBMIT | MF_RESET | MF_BUTTON) {
            fd.label = collapse(attr("aria-label").unwrap_or(""));
        }
        let byte = if fd.kind == MF_HIDDEN {
            None
        } else {
            let f = self.fmt();
            let b = self.cur.len();
            self.cur.push("\u{FFFC}", &f);
            self.started = true;
            self.space = false;
            Some(b)
        };
        let para = self.out.len();
        let e = self.ext.as_mut()?;
        fd.form = e.form.map_or(0, |i| i + 1);
        if let Some(id) = attr("form") {
            e.form_of.push((n, String::from(id.trim())));
        }
        if let Some(t) = attr("title") {
            e.titles.push((n, collapse(t)));
        }
        if let Some(l) = e.label.as_mut() {
            if l.2.is_none() {
                l.2 = Some(n);
            }
        }
        e.fields.push(fd);
        e.items.push((para, HExt::Field { byte, n }));
        Some(n)
    }

    // html_read: the choice read last gets its text (collapsed) and, without a value
    // attribute, that text as its value
    fn option_end(fd: &mut MdField, had_value: bool) {
        if let Some(o) = fd.options.last_mut() {
            o.label = collapse(&o.label);
            if !had_value {
                o.value = o.label.clone();
            }
        }
    }

    // html_read: the end of a <select>'s or <button>'s content
    fn cap_end(&mut self) {
        let Some(e) = self.ext.as_mut() else { return };
        let Some(f) = e.cap.take() else { return };
        let had_value = e.opt_val;
        let Some(fd) = e.fields.get_mut(f) else { return };
        if fd.kind == MF_SELECT {
            Self::option_end(fd, had_value);
            if fd.selected < 0 && !fd.options.is_empty() {
                fd.selected = 0;
            }
        } else {
            fd.label = collapse(&fd.label);
        }
    }

    // html_read: the end of a <label>: its text names the field inside it, or the one with the
    // id of its for= (resolved at the end); an aria-label wins
    fn label_end(&mut self) {
        let Some(e) = self.ext.as_mut() else { return };
        let Some((to, text, field)) = e.label.take() else { return };
        let text = collapse(&text);
        if text.is_empty() {
            return;
        }
        if !to.is_empty() {
            e.labels.push((to, text));
        } else if let Some(fd) = field.and_then(|f| e.fields.get_mut(f)) {
            if fd.label.is_empty() && !matches!(fd.kind, MF_SUBMIT | MF_RESET | MF_BUTTON) {
                fd.label = text;
            }
        }
    }

    // html_read: a <textarea>'s text (raw; entities decoded, line ends as '\n', the first one
    // dropped)
    fn raw_text(&mut self, s: &str) {
        if self.ext.as_ref().is_some_and(|e| e.raw.is_some()) {
            self.raw_text_dec(&decode(s));
        }
    }

    // the same with the entities decoded
    fn raw_text_dec(&mut self, s: &str) {
        let Some(e) = self.ext.as_mut() else { return };
        let Some(fd) = e.raw.take().and_then(|f| e.fields.get_mut(f)) else { return };
        let t = s.replace("\r\n", "\n").replace('\r', "\n");
        fd.value = String::from(t.strip_prefix('\n').unwrap_or(&t));
    }

    // html_read: forms and their fields (form, label, input, textarea, select / option,
    // button) -> the tag was taken
    fn form_open(&mut self, tag: &str, at: &[(String, String)]) -> bool {
        let attr = |k: &str| at.iter().find(|a| a.0 == k).map(|a| a.1.as_str());
        let num = |k: &str| attr(k).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0).min(100_000);
        let started = self.started;
        let Some(e) = self.ext.as_mut() else { return false };
        // inside a <select> or <button>: choices, a picture's alt text; other tags count as
        // nothing, a new field ends it
        if let Some(fd) = e.cap.and_then(|f| e.fields.get_mut(f)) {
            match tag {
                "input" | "select" | "textarea" | "button" => {}
                "option" if fd.kind == MF_SELECT => {
                    if fd.options.len() < OPTIONS_MAX {
                        Self::option_end(fd, e.opt_val);
                        if attr("selected").is_some() {
                            fd.selected = fd.options.len() as i32;
                        }
                        fd.options.push(MdOption { value: String::from(attr("value").unwrap_or("")), label: String::new() });
                        e.opt_val = attr("value").is_some();
                    }
                    return true;
                }
                "img" if fd.kind != MF_SELECT => {
                    push_cap(&mut fd.label, attr("alt").unwrap_or(""));
                    return true;
                }
                _ => return true,
            }
            self.cap_end();
        }
        let Some(e) = self.ext.as_mut() else { return false };
        let field = |kind: u8| MdField { kind, selected: -1, ..MdField::default() };
        match tag {
            "form" => {
                // (forms do not nest: an inner one counts as nothing)
                if e.form.is_none() && e.forms.len() < FORMS_MAX {
                    let method = if attr("method").is_some_and(|m| m.trim().eq_ignore_ascii_case("post")) { "post" } else { "get" };
                    e.forms.push(MdForm {
                        action: String::from(attr("action").unwrap_or("").trim()),
                        method: String::from(method),
                        id: String::from(attr("id").unwrap_or("").trim()),
                        name: String::from(attr("name").unwrap_or("")),
                        node: self.node,
                    });
                    e.form = Some(e.forms.len() as u32 - 1);
                    if !self.in_cell(tag) {
                        self.flush(false);
                    }
                }
            }
            "label" => {
                self.label_end();
                if let Some(e) = self.ext.as_mut() {
                    e.label = Some((String::from(attr("for").unwrap_or("").trim()), String::new(), None));
                }
            }
            "input" => {
                let ty = attr("type").unwrap_or("text").trim().to_ascii_lowercase();
                let kind = match ty.as_str() {
                    "password" => MF_PASSWORD,
                    "checkbox" => MF_CHECKBOX,
                    "radio" => MF_RADIO,
                    "hidden" => MF_HIDDEN,
                    "submit" | "image" => MF_SUBMIT,
                    "reset" => MF_RESET,
                    "button" => MF_BUTTON,
                    _ => MF_TEXT,
                };
                let checked = attr("checked").is_some();
                // a disabled check box at the start of a list item: the item's task box
                if kind == MF_CHECKBOX && attr("disabled").is_some() && e.item.is_some() && !started {
                    e.items.push((self.out.len(), HExt::Task(checked)));
                    return true;
                }
                let value = attr("value");
                let mut fd = field(kind);
                fd.checked = checked;
                fd.value = String::from(match kind {
                    MF_CHECKBOX | MF_RADIO => value.unwrap_or("on"),
                    _ => value.unwrap_or(""),
                });
                if kind == MF_TEXT || kind == MF_PASSWORD {
                    fd.value.retain(|c| c != '\r' && c != '\n');
                }
                if matches!(kind, MF_SUBMIT | MF_RESET | MF_BUTTON) {
                    fd.label = collapse(if ty == "image" { attr("alt").unwrap_or("") } else { value.unwrap_or("") });
                }
                fd.placeholder = collapse(attr("placeholder").unwrap_or(""));
                fd.size = num("size");
                fd.maxlength = num("maxlength");
                self.field_new(fd, at);
            }
            "textarea" => {
                let mut fd = field(MF_TEXTAREA);
                fd.placeholder = collapse(attr("placeholder").unwrap_or(""));
                fd.size = num("cols");
                fd.rows = num("rows");
                fd.maxlength = num("maxlength");
                let n = self.field_new(fd, at);
                if let Some(e) = self.ext.as_mut() {
                    e.raw = n;
                }
                // (the text up to </textarea> is the field's: run reads it raw)
                self.skip = Some(String::from("textarea"));
            }
            "select" | "button" => {
                let kind = match (tag, attr("type").map(|t| t.trim().to_ascii_lowercase()).as_deref()) {
                    ("select", _) => MF_SELECT,
                    (_, Some("reset")) => MF_RESET,
                    (_, Some("button")) => MF_BUTTON,
                    _ => MF_SUBMIT,
                };
                let mut fd = field(kind);
                if kind != MF_SELECT {
                    fd.value = String::from(attr("value").unwrap_or(""));
                }
                let n = self.field_new(fd, at);
                if let Some(e) = self.ext.as_mut() {
                    e.cap = n;
                    e.opt_val = false;
                }
            }
            _ => return false,
        }
        true
    }

    // html_read: the end of a form, a label, a list or a button -> the tag was taken
    fn form_close(&mut self, tag: &str) -> bool {
        let Some(e) = self.ext.as_ref() else { return false };
        if let Some(f) = e.cap {
            let select = e.fields.get(f).is_some_and(|fd| fd.kind == MF_SELECT);
            match tag {
                "select" if select => {}
                "button" if !select => {}
                "form" => {}
                // (end tags inside a <select> or <button> count as nothing)
                _ => return true,
            }
            self.cap_end();
            if tag != "form" {
                return true;
            }
        }
        match tag {
            "label" => self.label_end(),
            "form" => {
                if self.ext.as_ref().is_some_and(|e| e.form.is_some()) {
                    if !self.in_cell(tag) {
                        self.flush(false);
                    }
                    if let Some(e) = self.ext.as_mut() {
                        e.form = None;
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn open(&mut self, tag: &str, at: &[(String, String)], selfclose: bool) {
        if self.ext.is_some() && self.form_open(tag, at) {
            return;
        }
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
                        e.rule_boxes.push((self.out.len(), e.open_boxes.clone()));
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
            self.blocks.push(Block { tag: String::from(tag), align, style, pre, node: self.node });
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
        if self.ext.is_some() && self.form_close(tag) {
            return;
        }
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
                        self.raw_text(&rest[..i]);
                        rest = &rest[i..];
                        self.skip = None;
                    }
                    None => {
                        self.raw_text(rest);
                        return;
                    }
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

// ------------------------------------------------------------------ style sheets

// rules of a document's style sheets, declarations of a rule at most (more are ignored)
const RULES_MAX: usize = 4096;
const DECLS_MAX: usize = 256;

// a declaration: the property (lower case), its value, !important
type Decl = (String, String, bool);

// a rule: its selector, specificity and place, its declarations (an index of Sheet::decls)
#[derive(Debug)]
struct Rule {
    sel: Sel,
    spec: u32,
    order: u32,
    decls: u32,
}

/// the style sheets of a document (its <style> elements in document order; media other than
/// screen / all and types other than text/css are left out): the rules by the id, class or
/// tag their subject needs
#[derive(Debug, Default)]
pub(crate) struct Sheet {
    rules: Vec<Rule>,
    decls: Vec<Vec<Decl>>,
    ids: BTreeMap<String, Vec<u32>>,
    classes: BTreeMap<String, Vec<u32>>,
    tags: BTreeMap<String, Vec<u32>>,
    any: Vec<u32>,
    // bytes of CSS read
    size: usize,
}

impl Sheet {
    /// the style sheets of a document's tree
    pub(crate) fn of(d: &Dom) -> Sheet {
        let mut s = Sheet::default();
        if !d.is_html() {
            return s;
        }
        for x in d.subtree(0) {
            let n = &d.nodes[x as usize];
            if n.kind != K_ELEM || n.name != "style" {
                continue;
            }
            let media = d.attr(x, "media").unwrap_or("").to_ascii_lowercase();
            let ty = d.attr(x, "type").unwrap_or("text/css").trim().to_ascii_lowercase();
            if (!media.trim().is_empty() && !media.contains("screen") && !media.contains("all")) || (!ty.is_empty() && ty != "text/css") {
                continue;
            }
            s.add(&d.text_of(x));
        }
        s
    }

    /// no rules
    pub(crate) fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// what matching may cost in a document of n nodes (compounds compared)
    pub(crate) fn budget(&self, n: usize) -> usize {
        n.saturating_add(self.size).saturating_mul(64).saturating_add(1 << 20)
    }

    // the rules of a sheet's text (comments, at-rules and rules with selectors this subset does
    // not take are left out)
    fn add(&mut self, css: &str) {
        self.size = self.size.saturating_add(css.len());
        let s = strip_comments(css);
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() && self.rules.len() < RULES_MAX {
            if b[i].is_ascii_whitespace() || b[i] == b';' || b[i] == b'}' {
                i += 1;
                continue;
            }
            if b[i] == b'@' {
                // an at-rule: up to its ';' or the end of its block
                i = skip_rule(b, i, true);
                continue;
            }
            let Some(open) = s[i..].find('{').map(|k| i + k) else { break };
            let end = skip_rule(b, open, false);
            let sel = s[i..open].trim();
            let body = &s[open + 1..end.saturating_sub(1).max(open + 1)];
            i = end;
            let Some(sels) = parse_sels(sel) else { continue };
            let decls = parse_decls(body);
            if decls.is_empty() {
                continue;
            }
            let di = self.decls.len() as u32;
            self.decls.push(decls);
            for sel in sels {
                if self.rules.len() >= RULES_MAX {
                    break;
                }
                let r = self.rules.len() as u32;
                match sel.key() {
                    SelKey::Id(k) => self.ids.entry(String::from(k)).or_default().push(r),
                    SelKey::Class(k) => self.classes.entry(String::from(k)).or_default().push(r),
                    SelKey::Tag(k) => self.tags.entry(String::from(k)).or_default().push(r),
                    SelKey::Any => self.any.push(r),
                }
                self.rules.push(Rule { spec: sel.spec(), sel, order: r, decls: di });
            }
        }
    }

    /// element i's attributes with its style as the cascade gives it: the declarations of the
    /// rules that match by specificity and order, its style attribute's, then the !important
    /// ones in the same order -> None: no rule matches (its own attributes stand)
    pub(crate) fn attrs_of(&self, d: &Dom, i: u32, budget: &mut usize) -> Option<Vec<(String, String)>> {
        if self.rules.is_empty() || *budget == 0 {
            return None;
        }
        let mut hits: Vec<(u32, u32, u32)> = Vec::new();
        let test = |list: &[u32], hits: &mut Vec<(u32, u32, u32)>, budget: &mut usize| {
            for &r in list {
                if *budget == 0 {
                    break;
                }
                let rule = &self.rules[r as usize];
                if sel_match(d, i, &rule.sel, budget) {
                    hits.push((rule.spec, rule.order, rule.decls));
                }
            }
        };
        test(&self.any, &mut hits, budget);
        if let Some(v) = d.attr(i, "id").and_then(|id| self.ids.get(id)) {
            test(v, &mut hits, budget);
        }
        if let Some(c) = d.attr(i, "class") {
            for w in c.split_ascii_whitespace() {
                if let Some(v) = self.classes.get(w) {
                    test(v, &mut hits, budget);
                }
            }
        }
        if let Some(v) = self.tags.get(&d.nodes[i as usize].name) {
            test(v, &mut hits, budget);
        }
        if hits.is_empty() {
            return None;
        }
        hits.sort_unstable();
        hits.dedup();
        let own = d.attr(i, "style").map(parse_decls).unwrap_or_default();
        // the declarations in the cascade's order; of each property only its last one, where
        // it stands (a shorthand and its parts keep their order)
        let mut seq: Vec<(&str, &str)> = Vec::new();
        for imp in [false, true] {
            let rules = hits.iter().flat_map(|h| self.decls[h.2 as usize].iter());
            for (p, v, im) in rules.chain(own.iter()) {
                if *im == imp {
                    seq.push((p, v));
                }
            }
        }
        let mut last: BTreeMap<&str, usize> = BTreeMap::new();
        for (k, (p, _)) in seq.iter().enumerate() {
            last.insert(p, k);
        }
        let mut css = String::new();
        for (k, (p, v)) in seq.iter().enumerate() {
            if last.get(p) == Some(&k) {
                css.push_str(p);
                css.push(':');
                css.push_str(v);
                css.push(';');
            }
        }
        let mut at = d.nodes[i as usize].attrs.clone();
        match at.iter_mut().find(|a| a.0 == "style") {
            Some(a) => a.1 = css,
            None => at.push((String::from("style"), css)),
        }
        Some(at)
    }
}

// CSS without its comments
fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        out.push(' ');
        rest = rest[i + 2..].find("*/").map_or("", |j| &rest[i + 2 + j + 2..]);
    }
    out.push_str(rest);
    out
}

// the end of a rule or at-rule at b[i] (at '{' for a rule): after its block (blocks inside
// counted, quotes respected) or, for an at-rule without a block, after its ';'
fn skip_rule(b: &[u8], mut i: usize, at: bool) -> usize {
    let (mut depth, mut q) = (0usize, 0u8);
    while i < b.len() {
        let c = b[i];
        i += 1;
        if q != 0 {
            if c == q {
                q = 0;
            }
            continue;
        }
        match c {
            b'"' | b'\'' => q = c,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i;
                }
            }
            b';' if at && depth == 0 => return i,
            _ => {}
        }
    }
    i
}

// the declarations of a block or a style attribute (property: value [!important]; ...)
fn parse_decls(body: &str) -> Vec<Decl> {
    let mut out = Vec::new();
    let b = body.as_bytes();
    let (mut start, mut q, mut paren) = (0, 0u8, 0usize);
    let push = |d: &str, out: &mut Vec<Decl>| {
        let Some((p, v)) = d.split_once(':') else { return };
        let p = p.trim().to_ascii_lowercase();
        if p.is_empty() || !p.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') || out.len() >= DECLS_MAX {
            return;
        }
        let mut v = v.trim();
        let mut imp = false;
        if let Some(k) = v.rfind('!') {
            if v[k + 1..].trim().eq_ignore_ascii_case("important") {
                imp = true;
                v = v[..k].trim();
            }
        }
        if !v.is_empty() {
            out.push((p, String::from(v), imp));
        }
    };
    for (i, &c) in b.iter().enumerate() {
        if q != 0 {
            if c == q {
                q = 0;
            }
            continue;
        }
        match c {
            b'"' | b'\'' => q = c,
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b';' if paren == 0 => {
                push(&body[start..i], &mut out);
                start = i + 1;
            }
            _ => {}
        }
    }
    push(&body[start..], &mut out);
    out
}

/// an element whose CSS may give a box around its blocks (md_doc.rs MdBox)
pub(crate) fn box_tag(t: &str) -> bool {
    BLOCKS.contains(&t)
        || matches!(t, "ul" | "ol" | "dl" | "nav" | "main" | "aside" | "form" | "fieldset" | "details" | "summary" | "body" | "figcaption" | "hgroup" | "dialog" | "menu")
}

// a CSS length in px at 100 % text (em, rem: 14 px, the body text; % and other units: none)
fn css_len(v: &str) -> Option<i32> {
    let v = v.trim().to_ascii_lowercase();
    let num = |s: &str| s.trim().parse::<f32>().ok();
    let px = if v == "0" || v == "auto" {
        0.0
    } else if let Some(n) = v.strip_suffix("px") {
        num(n)?
    } else if let Some(n) = v.strip_suffix("pt") {
        num(n)? * 4.0 / 3.0
    } else if let Some(n) = v.strip_suffix("rem").or_else(|| v.strip_suffix("em")) {
        num(n)? * 14.0
    } else {
        match v.as_str() {
            "thin" => 1.0,
            "medium" => 3.0,
            "thick" => 5.0,
            _ => return None,
        }
    };
    Some((px.clamp(-2000.0, 2000.0) + if px < 0.0 { -0.5 } else { 0.5 }) as i32)
}

// one to four values for the four sides (top, right, bottom, left)
fn sides<T: Copy>(v: &[T]) -> Option<[T; 4]> {
    match *v {
        [a] => Some([a, a, a, a]),
        [a, b] => Some([a, b, a, b]),
        [a, b, c] => Some([a, b, c, b]),
        [a, b, c, d] => Some([a, b, c, d]),
        _ => None,
    }
}

// a border style: visible (true: drawn solid), none / hidden (false)
fn border_style(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" => Some(true),
        "none" | "hidden" => Some(false),
        _ => None,
    }
}

// the words of a value (blanks outside parentheses)
fn words(v: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut paren) = (None, 0usize);
    for (i, c) in v.char_indices() {
        match c {
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            _ => {}
        }
        if c.is_whitespace() && paren == 0 {
            if let Some(s) = start.take() {
                out.push(&v[s..i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push(&v[s..]);
    }
    out
}

/// the box an element's style gives (margins, padding, borders drawn solid, the background)
/// -> None: none
pub(crate) fn box_of(style: &str) -> Option<MdBox> {
    let mut b = MdBox::default();
    // per side: the border's width given, its style (None: not given), its colour
    let mut bw: [Option<i32>; 4] = [None; 4];
    let mut bs: [Option<bool>; 4] = [None; 4];
    let mut bc = [0u32; 4];
    let pick = |p: &str| match p {
        "top" => Some(0),
        "right" => Some(1),
        "bottom" => Some(2),
        "left" => Some(3),
        _ => None,
    };
    for decl in style.split(';') {
        let Some((p, v)) = decl.split_once(':') else { continue };
        let p = p.trim().to_ascii_lowercase();
        let v = v.trim();
        let lens = || -> Option<[i32; 4]> { sides(&words(v).iter().map(|w| css_len(w)).collect::<Option<Vec<i32>>>()?) };
        match p.as_str() {
            "margin" => {
                if let Some(s) = lens() {
                    b.margin = s.map(|x| x.clamp(0, 1000) as i16);
                    b.mset = 15;
                }
            }
            "padding" => {
                if let Some(s) = lens() {
                    b.padding = s.map(|x| x.clamp(0, 1000) as i16);
                }
            }
            "border-width" => {
                if let Some(s) = lens() {
                    bw = s.map(Some);
                }
            }
            "border-style" => {
                if let Some(s) = sides(&words(v).iter().map(|w| border_style(w)).collect::<Option<Vec<bool>>>().unwrap_or_default()) {
                    bs = s.map(Some);
                }
            }
            "border-color" => {
                if let Some(s) = sides(&words(v).iter().map(|w| css_color(w)).collect::<Option<Vec<u32>>>().unwrap_or_default()) {
                    bc = s;
                }
            }
            "background" | "background-color" => {
                if let Some(c) = words(v).into_iter().find_map(css_color) {
                    b.bg = c;
                } else if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("transparent") {
                    b.bg = 0;
                }
            }
            _ => {
                let rest = p.strip_prefix("margin-").or_else(|| p.strip_prefix("padding-"));
                if let (Some(side), Some(n)) = (rest.and_then(pick), css_len(v)) {
                    if p.starts_with('m') {
                        b.margin[side] = n.clamp(0, 1000) as i16;
                        b.mset |= 1 << side;
                    } else {
                        b.padding[side] = n.clamp(0, 1000) as i16;
                    }
                    continue;
                }
                // border, border-SIDE (width style colour in any order), border-SIDE-width / -style / -color
                let Some(rest) = p.strip_prefix("border") else { continue };
                let (which, part) = match rest.strip_prefix('-') {
                    None if rest.is_empty() => ([true; 4], ""),
                    Some(r) => match r.split_once('-') {
                        Some((s, part)) => match pick(s) {
                            Some(k) => (core::array::from_fn(|i| i == k), part),
                            None => continue,
                        },
                        None => match pick(r) {
                            Some(k) => (core::array::from_fn(|i| i == k), ""),
                            None => continue,
                        },
                    },
                    _ => continue,
                };
                for (k, on) in which.iter().enumerate() {
                    if !on {
                        continue;
                    }
                    match part {
                        "width" => bw[k] = css_len(v).or(bw[k]),
                        "style" => bs[k] = border_style(v).or(bs[k]),
                        "color" => bc[k] = css_color(v).unwrap_or(bc[k]),
                        "" => {
                            // the shorthand resets what it does not give
                            let (mut w, mut s, mut c) = (None, None, 0);
                            for t in words(v) {
                                if let Some(x) = border_style(t) {
                                    s = Some(x);
                                } else if let Some(x) = css_len(t) {
                                    w = Some(x);
                                } else if let Some(x) = css_color(t) {
                                    c = x;
                                }
                            }
                            bw[k] = w;
                            bs[k] = Some(s.unwrap_or(false));
                            bc[k] = c;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    for k in 0..4 {
        // (a border needs a style; without a width it is medium)
        b.border[k] = if bs[k] == Some(true) { bw[k].unwrap_or(3).clamp(0, 100) as u8 } else { 0 };
        b.border_color[k] = bc[k];
    }
    let any = b.margin.iter().any(|&x| x != 0) || b.padding.iter().any(|&x| x != 0) || b.border.iter().any(|&x| x != 0) || b.bg != 0 || b.mset != 0;
    any.then_some(b)
}

// the format an element's attributes give on top of f (view: the viewer's reader -- a link
// takes the link colour unless it has one of its own, a box's background is the box's)
fn fmt_with(mut f: Fmt, tag: &str, at: &[(String, String)], view: bool) -> Fmt {
    if view && tag == "a" && at.iter().any(|a| a.0 == "href") {
        f.color = 0;
    }
    let boxed = view && box_tag(tag);
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
                        "background-color" | "background" if !boxed => {
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

// the '>' that ends the tag at s[0] == '<' (quotes respected)
pub(crate) fn tag_end(s: &str) -> Option<usize> {
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

pub(crate) fn find_ci(hay: &str, needle: &str) -> Option<usize> {
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
