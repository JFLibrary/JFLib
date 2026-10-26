// md_doc -- the document of the Markdown / HTML viewer (md_view.rs): blocks of formatted text,
// read from Markdown (CommonMark with GitHub's tables, task lists, strikethrough and bare
// links) or from HTML (the reader of the rich-text editor, rich_html.rs)
//
//   MdDoc::markdown(s) / MdDoc::html(s)  parse; never panics, takes linear time: nesting, input
//                                        size and the inline parser's work are capped
//   doc.blocks() -> &[MdBlock]           the blocks in reading order (see MdBlock)
//   doc.links() / images() / tables()    link targets (MdSpan::link - 1), images (MdSpan::image
//                                        - 1: src, alt, title, size), table column alignments
//   doc.plain() / doc.to_html()          everything as text (list markers, cells by tabs) / HTML
//   doc.plain_range(a, b) / html_range(a, b)   a part: TextPos { line: block, byte }
//   doc.link_spots()                     every link in reading order (block, bytes, link)
//   doc.forms() / fields() / field_spots()   the forms of HTML, their fields (MdSpan::field - 1;
//                                        hidden ones have no character; their first values:
//                                        md_view keeps the current ones) and where the visible
//                                        ones stand in reading order; field.caption(): a
//                                        button's caption
//   doc.truncated()                      a limit cut the input
//
// The model: a block is a paragraph, a heading (level 1..6), a code block (its text, the
// language of the fence), a rule or a table row (cells: byte ranges of the text, '\t' between
// them).  Its place: `quote` (block quotes around it), `list` (lists around it), `marker` /
// `number` / `task` (it starts a list item), `space` (the gap before it: 0 none, 1 a tight
// list's, 2 a paragraph's).  Its text holds the visible characters (an image is one U+FFFC,
// a hard break '\n'); spans give ranges their style (MS_*), link, image and colours.
//
// Markdown: ATX and setext headings, paragraphs with lazy continuation lines, block quotes,
// bullet (- + *) and ordered (1. 1)) lists with start numbers, tight / loose spacing and task
// items ([ ] / [x]), fenced (``` ~~~ with an info string) and indented code, thematic breaks,
// GFM tables (alignment row, escaped pipes), HTML blocks (through the HTML reader), link
// reference definitions; inline: backslash escapes, entities, code spans, emphasis and strong
// emphasis (the delimiter algorithm with the rule of three), ~~strikethrough~~, links and
// images (inline, full / collapsed / shortcut references, titles), autolinks (<...>, bare
// http:// https:// www.), hard breaks (two spaces, backslash) and soft breaks, a little inline
// HTML (br, b / strong, i / em, s / del, u / ins, code / kbd, a href, img).  Other tags are
// dropped, their text stays.  Tabs expand to 4 columns.
//
// Forms (HTML, also in Markdown's HTML blocks): <form action method> (forms do not nest),
// <input> of type text (and the other text types: email, search, url, tel, number ...),
// password, checkbox, radio, hidden, submit, image (a submit button), reset and button,
// <textarea> (its text read raw, one leading line end dropped), <select> with <option value
// selected> (one choice), <button type> (its text the caption), <label> (around a field or
// for= its id: the field's name; aria-label wins, title is the last resort) and form= (a field
// outside its form).  A field is one U+FFFC in the text with a span of its own (MdSpan::field);
// a disabled check box at the start of a list item is that item's task box (as Markdown's
// "[x]" is written out by html_range).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::ed_core::TextPos;
use crate::i18n::tr;
use crate::rich_doc::*;
use crate::rich_html::{attrs, entity, html_read, HExt, HRead};

/// MdBlock::kind: a paragraph
pub const MB_PARA: u8 = 0;
/// a heading (MdBlock::level 1 ..= 6)
pub const MB_HEADING: u8 = 1;
/// a code block (MdBlock::lang: the fence's language)
pub const MB_CODE: u8 = 2;
/// a thematic break (no text)
pub const MB_RULE: u8 = 3;
/// a table row (MdBlock::cells, table, head)
pub const MB_ROW: u8 = 4;

/// MdSpan::style: strong
pub const MS_BOLD: u16 = 1;
/// emphasis
pub const MS_ITALIC: u16 = 2;
/// struck through
pub const MS_STRIKE: u16 = 4;
/// code (monospace)
pub const MS_CODE: u16 = 8;
/// underlined (HTML)
pub const MS_UNDER: u16 = 16;

/// MdBlock::marker: no list item starts here
pub const MK_NONE: u8 = 0;
/// a bullet item
pub const MK_BULLET: u8 = 1;
/// a numbered item (MdBlock::number, delim)
pub const MK_NUMBER: u8 = 2;

/// MdTable::aligns: not given
pub const MA_NONE: u8 = 0;
/// left (start)
pub const MA_LEFT: u8 = 1;
/// centred
pub const MA_CENTER: u8 = 2;
/// right (end)
pub const MA_RIGHT: u8 = 3;

/// MdField::kind: a one-line text field (<input> of type text, email, search, url, tel,
/// number and every type not named below)
pub const MF_TEXT: u8 = 0;
/// a password field (its text never leaves the view but in the form's data)
pub const MF_PASSWORD: u8 = 1;
/// a check box
pub const MF_CHECKBOX: u8 = 2;
/// a radio button: one of the radios of its form with its name
pub const MF_RADIO: u8 = 3;
/// a hidden field (no character in the text)
pub const MF_HIDDEN: u8 = 4;
/// a text area (<textarea>: several lines)
pub const MF_TEXTAREA: u8 = 5;
/// a drop-down list (<select>)
pub const MF_SELECT: u8 = 6;
/// a submit button (<button>, <input type=submit> / image)
pub const MF_SUBMIT: u8 = 7;
/// a reset button: the form's fields get their first values back
pub const MF_RESET: u8 = 8;
/// a button without an action of its own (<button type=button>, <input type=button>)
pub const MF_BUTTON: u8 = 9;

const INPUT_MAX: usize = 16 << 20; // bytes read (the rest is cut)
const NEST_MAX: usize = 32; // block quotes and lists inside each other
const BLOCKS_MAX: usize = 1 << 20;
const LABEL_MAX: usize = 999;
const PARENS_MAX: usize = 32;
const HTML_STACK: usize = 64;
const COLS_MAX: usize = 128;
const CELLS_MAX: usize = 1 << 21;
const FIELDS_MAX: usize = 0xFFFE; // form fields of a document (the view's ids: md_field_id)

/// a range of a block's text with its style
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MdSpan {
    /// bytes a .. b of the block's text
    pub a: u32,
    pub b: u32,
    /// MS_*
    pub style: u16,
    /// a link: index + 1 into MdDoc::links (0: none)
    pub link: u32,
    /// an image (the text is U+FFFC): index + 1 into MdDoc::images (0: none)
    pub image: u32,
    /// HTML colours 0xAARRGGBB (0: the theme's)
    pub color: u32,
    pub bg: u32,
    /// a form field (the text is U+FFFC): index + 1 into MdDoc::fields (0: none)
    pub field: u32,
}

/// a block of the document (see the module header)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MdBlock {
    /// MB_*
    pub kind: u8,
    /// headings: 1 ..= 6
    pub level: u8,
    /// the visible text (U+FFFC an image, '\n' a hard break or a code line's end, '\t' between
    /// cells)
    pub text: String,
    /// styled ranges, ascending, not overlapping (the text between them is plain)
    pub spans: Vec<MdSpan>,
    /// block quotes around it
    pub quote: u8,
    /// lists around it (0: none)
    pub list: u8,
    /// the containers from the outside in: bit i set = the i-th is a block quote, clear = a
    /// list
    pub path: u32,
    /// MK_*: a list item starts with this block
    pub marker: u8,
    /// MK_NUMBER: the item's number and its delimiter ('.' or ')')
    pub number: u32,
    pub delim: u8,
    /// a task item: 0 none, 1 open, 2 done
    pub task: u8,
    /// the gap before it: 0 none, 1 a tight list's, 2 a paragraph's
    pub space: u8,
    /// code blocks: the language of the fence ("" none)
    pub lang: String,
    /// table rows: the cells' byte ranges
    pub cells: Vec<(u32, u32)>,
    /// table rows: index into MdDoc::tables, a header row
    pub table: u32,
    pub head: bool,
    /// paragraphs: 0 start, 1 centre, 2 end (HTML)
    pub align: u8,
}

/// an image of the document
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MdImage {
    pub src: String,
    pub alt: String,
    pub title: String,
    /// size given by HTML attributes (px, 0: the picture's)
    pub w: u32,
    pub h: u32,
}

/// a table: its columns' alignment (MA_*)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MdTable {
    pub aligns: Vec<u8>,
}

/// a link in reading order: block, bytes a .. b of its text, the link (index + 1)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MdLinkSpot {
    pub block: usize,
    pub a: u32,
    pub b: u32,
    pub link: u32,
}

/// a form of HTML
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MdForm {
    /// where its data goes (as written: the app resolves it) and how ("get" / "post": lower
    /// case, "get" when not given)
    pub action: String,
    pub method: String,
    pub id: String,
    pub name: String,
}

/// a choice of a drop-down list: the value sent, the text shown
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MdOption {
    pub value: String,
    pub label: String,
}

/// a field of a form (its first value: the view keeps the current one, MdViewState)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MdField {
    /// MF_*
    pub kind: u8,
    /// its form: index + 1 into MdDoc::forms (0: none -- it sends nothing)
    pub form: u32,
    pub name: String,
    pub id: String,
    /// text fields: the text; check boxes, radios, buttons: the value sent ("on" for check
    /// boxes and radios without one)
    pub value: String,
    /// the name screen readers say (aria-label, the <label>, title); buttons: the caption
    pub label: String,
    pub placeholder: String,
    /// check boxes and radios: checked at first
    pub checked: bool,
    /// drop-down lists: the choices, the one chosen at first (-1: none)
    pub options: Vec<MdOption>,
    pub selected: i32,
    /// text fields: the width in characters (size; text areas: cols; 0: the default), text
    /// areas: the lines (rows)
    pub size: u32,
    pub rows: u32,
    /// at most this many characters (0: no limit)
    pub maxlength: u32,
    pub disabled: bool,
    pub readonly: bool,
}

impl MdField {
    /// a button's caption: its own, else "Senden" / "Zurücksetzen" (submit / reset, in the UI
    /// language); "" for the other kinds
    pub fn caption(&self) -> &str {
        match self.kind {
            MF_SUBMIT | MF_RESET | MF_BUTTON if !self.label.is_empty() => &self.label,
            MF_SUBMIT => tr("Senden"),
            MF_RESET => tr("Zurücksetzen"),
            _ => "",
        }
    }
}

/// a visible field in reading order: block, byte of its U+FFFC, the field (index + 1)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MdFieldSpot {
    pub block: usize,
    pub a: u32,
    pub field: u32,
}

/// the document (see the module header)
#[derive(Clone, Debug, Default)]
pub struct MdDoc {
    blocks: Vec<MdBlock>,
    links: Vec<String>,
    images: Vec<MdImage>,
    tables: Vec<MdTable>,
    spots: Vec<MdLinkSpot>,
    forms: Vec<MdForm>,
    fields: Vec<MdField>,
    fspots: Vec<MdFieldSpot>,
    truncated: bool,
}

// ------------------------------------------------------------------ small helpers

fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || (!c.is_alphanumeric() && !c.is_whitespace() && !c.is_control() && c as u32 > 127)
}

fn blank(s: &str) -> bool {
    s.bytes().all(|b| b == b' ' || b == b'\t')
}

fn indent_of(s: &str) -> usize {
    s.bytes().take_while(|&b| b == b' ').count()
}

// a line with its tabs expanded to 4 columns
fn expand_tabs(s: &str) -> String {
    if !s.contains('\t') {
        return String::from(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            let n = 4 - col % 4;
            for _ in 0..n {
                out.push(' ');
            }
            col += n;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}

// the label of a reference, compared case-insensitively with blanks collapsed
fn norm_label(s: &str) -> String {
    let mut out = String::new();
    let mut sp = false;
    for c in s.trim().chars() {
        if c.is_whitespace() {
            sp = true;
            continue;
        }
        if sp && !out.is_empty() {
            out.push(' ');
        }
        sp = false;
        for l in c.to_lowercase() {
            out.push(l);
        }
    }
    out
}

// backslash escapes and entities of a link destination or title
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() && b[i + 1].is_ascii_punctuation() {
            out.push(b[i + 1] as char);
            i += 2;
            continue;
        }
        if b[i] == b'&' {
            if let Some((c, n)) = entity_at(&s[i..]) {
                out.push(c);
                i += n;
                continue;
            }
        }
        let c = s[i..].chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    out
}

// an entity at the start of s -> (character, bytes)
fn entity_at(s: &str) -> Option<(char, usize)> {
    let b = s.as_bytes();
    let end = b.iter().take(34).position(|&c| c == b';')?;
    if end < 2 {
        return None;
    }
    let name = &s[1..end];
    if !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'#') {
        return None;
    }
    entity(name).map(|c| (c, end + 1))
}

// ------------------------------------------------------------------ the block parser

// a container: a block quote or a list item
#[derive(Clone)]
struct Cont {
    quote: bool,
    // list items: the content column, the marker, whether its first block was filed
    indent: usize,
    ordered: bool,
    number: u32,
    delim: u8,
    task: u8,
    filed: bool,
    // the list item began with a blank line (it may then hold one more)
    empty: bool,
}

// a leaf being read
enum Leaf {
    None,
    Para(Vec<String>),
    Fence { ch: u8, len: usize, indent: usize, info: String, lines: Vec<String> },
    Code(Vec<String>),
    Table { aligns: Vec<u8>, rows: Vec<(Vec<String>, bool)> },
    Html(Vec<String>),
}

// a block before its inline content is parsed
#[derive(Default)]
struct Raw {
    kind: u8,
    level: u8,
    src: String,
    cells: Vec<String>,
    html: bool,
    quote: u8,
    list: u8,
    path: u32,
    marker: u8,
    number: u32,
    delim: u8,
    task: u8,
    space: u8,
    lang: String,
    table: u32,
    head: bool,
}

struct BlockParser {
    conts: Vec<Cont>,
    leaf: Leaf,
    raws: Vec<Raw>,
    defs: BTreeMap<String, (String, String)>,
    tables: Vec<MdTable>,
    // a blank line since the last block (the next one gets a paragraph's gap)
    blank: bool,
    truncated: bool,
    // the cells of all tables so far (a row always costs the table's width)
    table_cells: usize,
}

// a list marker at the start of s (after up to 3 blanks) -> (ordered, bullet / delim, number,
// bytes of indent + marker)
fn list_marker(s: &str) -> Option<(bool, u8, u32, usize)> {
    let b = s.as_bytes();
    let sp = indent_of(s);
    if sp > 3 || sp >= b.len() {
        return None;
    }
    let c = b[sp];
    let after = |k: usize| k >= b.len() || b[k] == b' ';
    if matches!(c, b'-' | b'+' | b'*') {
        return after(sp + 1).then_some((false, c, 0, sp + 1));
    }
    let digits = b[sp..].iter().take_while(|c| c.is_ascii_digit()).count();
    if (1..=9).contains(&digits) {
        let k = sp + digits;
        if k < b.len() && (b[k] == b'.' || b[k] == b')') && after(k + 1) {
            let n = s[sp..k].parse::<u32>().unwrap_or(0);
            return Some((true, b[k], n, k + 1));
        }
    }
    None
}

// a thematic break: 3 or more of * - _ (blanks between them)
fn is_rule(s: &str) -> bool {
    if indent_of(s) > 3 {
        return false;
    }
    let t = s.trim();
    let Some(c) = t.bytes().next() else { return false };
    if !matches!(c, b'*' | b'-' | b'_') {
        return false;
    }
    let mut n = 0;
    for b in t.bytes() {
        if b == c {
            n += 1;
        } else if b != b' ' {
            return false;
        }
    }
    n >= 3
}

// an ATX heading -> (level, content)
fn atx(s: &str) -> Option<(u8, &str)> {
    if indent_of(s) > 3 {
        return None;
    }
    let t = s.trim_start();
    let n = t.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&n) {
        return None;
    }
    let rest = &t[n..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let mut c = rest.trim();
    // a closing sequence of #s (after a blank, or the whole content)
    let k = c.trim_end_matches('#');
    if k.is_empty() || k.ends_with(' ') {
        c = k.trim_end();
    }
    Some((n as u8, c))
}

// a fence opening -> (char, length, info)
fn fence_open(s: &str) -> Option<(u8, usize, String)> {
    let sp = indent_of(s);
    if sp > 3 {
        return None;
    }
    let t = &s[sp..];
    let c = *t.as_bytes().first()?;
    if c != b'`' && c != b'~' {
        return None;
    }
    let n = t.bytes().take_while(|&b| b == c).count();
    if n < 3 {
        return None;
    }
    let info = t[n..].trim();
    if c == b'`' && info.contains('`') {
        return None;
    }
    Some((c, n, unescape(info)))
}

// the cells of a table row (outer pipes optional, \| is no separator)
fn split_cells(s: &str) -> Vec<String> {
    let mut t = s.trim();
    if let Some(r) = t.strip_prefix('|') {
        t = r;
    }
    if t.ends_with('|') && !t.ends_with("\\|") {
        t = &t[..t.len() - 1];
    }
    let b = t.as_bytes();
    let mut out = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            i += 2;
            continue;
        }
        if b[i] == b'|' {
            out.push(t[start..i].trim().replace("\\|", "|"));
            start = i + 1;
            if out.len() >= COLS_MAX {
                return out;
            }
        }
        i += 1;
    }
    // (an escaped pipe is a pipe, also inside code spans)
    out.push(t[start..].trim().replace("\\|", "|"));
    out
}

// a delimiter row of a table -> the alignments
fn delim_row(s: &str) -> Option<Vec<u8>> {
    if indent_of(s) > 3 || !s.contains('-') {
        return None;
    }
    let t = s.trim();
    if !t.contains('|') && !t.starts_with(':') && !t.starts_with('-') {
        return None;
    }
    let cells = split_cells(s);
    let mut v = Vec::new();
    for c in cells {
        let l = c.starts_with(':');
        let r = c.ends_with(':');
        let core = c.trim_matches(':');
        if core.is_empty() || !core.bytes().all(|b| b == b'-') {
            return None;
        }
        v.push(match (l, r) {
            (true, true) => MA_CENTER,
            (true, false) => MA_LEFT,
            (false, true) => MA_RIGHT,
            _ => MA_NONE,
        });
    }
    // a single "---" without pipes is a setext underline or a rule
    if v.len() == 1 && !t.contains('|') {
        return None;
    }
    Some(v)
}

// an HTML block starts (a block-level tag or a comment)
fn html_start(s: &str) -> bool {
    if indent_of(s) > 3 {
        return false;
    }
    let t = s.trim_start();
    if t.starts_with("<!--") {
        return true;
    }
    let Some(r) = t.strip_prefix('<') else { return false };
    let r = r.strip_prefix('/').unwrap_or(r);
    let n = r.bytes().take_while(|b| b.is_ascii_alphanumeric()).count();
    if n == 0 {
        return false;
    }
    let name = r[..n].to_ascii_lowercase();
    let end = r.as_bytes().get(n).copied();
    if !matches!(end, None | Some(b' ') | Some(b'>') | Some(b'/')) {
        return false;
    }
    matches!(
        name.as_str(),
        "address" | "article" | "aside" | "blockquote" | "center" | "dd" | "details" | "dialog" | "div" | "dl" | "dt" | "fieldset"
            | "figcaption" | "figure" | "footer" | "form" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "header" | "hr" | "legend" | "li"
            | "main" | "nav" | "ol" | "p" | "pre" | "section" | "summary" | "table" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
            | "ul" | "img" | "picture"
    )
}

// link reference definitions at the start of a paragraph's text -> the rest
fn take_defs(src: &str, defs: &mut BTreeMap<String, (String, String)>) -> String {
    let mut rest = src;
    while rest.starts_with('[') {
        let Some((label, dest, title, after)) = parse_def(rest) else { break };
        let key = norm_label(&label);
        if !key.is_empty() {
            defs.entry(key).or_insert((dest, title));
        }
        rest = after;
    }
    String::from(rest)
}

// one definition: [label]: dest "title" (to the end of its line)
fn parse_def(s: &str) -> Option<(String, String, String, &str)> {
    let b = s.as_bytes();
    let mut i = 1;
    while i < b.len() && b[i] != b']' {
        if b[i] == b'[' || i > LABEL_MAX {
            return None;
        }
        if b[i] == b'\\' {
            i += 1;
        }
        i += 1;
    }
    if i >= b.len() || b.get(i + 1) != Some(&b':') {
        return None;
    }
    let label = &s[1..i];
    if label.trim().is_empty() {
        return None;
    }
    let mut k = i + 2;
    let ws = |k: &mut usize, nl: bool| {
        let mut seen = false;
        while *k < b.len() && (b[*k] == b' ' || (nl && !seen && b[*k] == b'\n')) {
            if b[*k] == b'\n' {
                seen = true;
            }
            *k += 1;
        }
    };
    ws(&mut k, true);
    let (dest, mut k) = parse_dest(s, k)?;
    let line_end = |k: usize| -> Option<usize> {
        let mut j = k;
        while j < b.len() && b[j] == b' ' {
            j += 1;
        }
        (j >= b.len() || b[j] == b'\n').then_some((j + 1).min(b.len()))
    };
    let before = k;
    // a title on the same line or the next
    let mut t = k;
    ws(&mut t, true);
    if t > before && t < b.len() && matches!(b[t], b'"' | b'\'' | b'(') {
        if let Some((title, e)) = parse_title(s, t) {
            if let Some(end) = line_end(e) {
                return Some((String::from(label), dest, title, &s[end..]));
            }
        }
    }
    k = line_end(k)?;
    Some((String::from(label), dest, String::new(), &s[k..]))
}

// a link destination at s[k..] -> (destination, the byte after it)
fn parse_dest(s: &str, k: usize) -> Option<(String, usize)> {
    let b = s.as_bytes();
    if k < b.len() && b[k] == b'<' {
        let mut j = k + 1;
        while j < b.len() && b[j] != b'>' {
            if b[j] == b'\n' || b[j] == b'<' {
                return None;
            }
            if b[j] == b'\\' {
                j += 1;
            }
            j += 1;
        }
        if j >= b.len() {
            return None;
        }
        return Some((unescape(&s[k + 1..j]), j + 1));
    }
    let mut j = k;
    let mut depth = 0usize;
    while j < b.len() {
        let c = b[j];
        if c == b'\\' && j + 1 < b.len() && b[j + 1].is_ascii_punctuation() {
            j += 2;
            continue;
        }
        if c <= b' ' {
            break;
        }
        if c == b'(' {
            depth += 1;
            if depth > PARENS_MAX {
                return None;
            }
        } else if c == b')' {
            if depth == 0 {
                break;
            }
            depth -= 1;
        }
        j += 1;
    }
    if depth != 0 || j == k {
        return None;
    }
    Some((unescape(&s[k..j]), j))
}

// a link title at s[k..] ("..", '..', (..)) -> (title, the byte after it)
fn parse_title(s: &str, k: usize) -> Option<(String, usize)> {
    let b = s.as_bytes();
    let close = match b.get(k)? {
        b'"' => b'"',
        b'\'' => b'\'',
        b'(' => b')',
        _ => return None,
    };
    let mut j = k + 1;
    while j < b.len() && b[j] != close {
        if b[j] == b'\\' {
            j += 1;
        }
        j += 1;
    }
    if j >= b.len() {
        return None;
    }
    Some((unescape(&s[k + 1..j]), j + 1))
}

impl BlockParser {
    fn new() -> BlockParser {
        BlockParser { conts: Vec::new(), leaf: Leaf::None, raws: Vec::new(), defs: BTreeMap::new(),         tables: Vec::new(), blank: false, truncated: false, table_cells: 0 }
    }

    // a raw block in the place of the open containers
    fn file(&mut self, mut r: Raw) {
        if self.raws.len() >= BLOCKS_MAX {
            self.truncated = true;
            return;
        }
        r.quote = self.conts.iter().filter(|c| c.quote).count() as u8;
        r.list = self.conts.iter().filter(|c| !c.quote).count() as u8;
        r.path = self.conts.iter().enumerate().filter(|(_, c)| c.quote).fold(0, |p, (i, _)| p | 1u32 << (i & 31));
        let tight = r.list > 0 && self.raws.last().is_some_and(|p| p.list > 0) && !self.blank;
        r.space = if self.raws.is_empty() { 0 } else if tight { 1 } else { 2 };
        if let Some(c) = self.conts.iter_mut().rev().find(|c| !c.quote) {
            if !c.filed {
                c.filed = true;
                r.marker = if c.ordered { MK_NUMBER } else { MK_BULLET };
                r.number = c.number;
                r.delim = c.delim;
                r.task = c.task;
            }
        }
        self.blank = false;
        self.raws.push(r);
    }

    // close the open leaf
    fn close_leaf(&mut self) {
        match core::mem::replace(&mut self.leaf, Leaf::None) {
            Leaf::None => {}
            Leaf::Para(lines) => {
                let mut src = lines.join("\n");
                let t = src.trim_end().len();
                src.truncate(t);
                let rest = take_defs(&src, &mut self.defs);
                if !rest.trim().is_empty() {
                    self.file(Raw { kind: MB_PARA, src: rest, ..Raw::default() });
                }
            }
            Leaf::Fence { info, lines, .. } => {
                let lang = String::from(info.split_whitespace().next().unwrap_or(""));
                self.file(Raw { kind: MB_CODE, src: lines.join("\n"), lang, ..Raw::default() });
            }
            Leaf::Code(mut lines) => {
                while lines.last().is_some_and(|l| blank(l)) {
                    lines.pop();
                }
                if !lines.is_empty() {
                    self.file(Raw { kind: MB_CODE, src: lines.join("\n"), ..Raw::default() });
                }
            }
            Leaf::Table { aligns, rows } => {
                let t = self.tables.len() as u32;
                let n = aligns.len();
                self.tables.push(MdTable { aligns });
                for (ri, (mut cells, head)) in rows.into_iter().enumerate() {
                    cells.resize(n, String::new());
                    self.file(Raw { kind: MB_ROW, cells, table: t, head, ..Raw::default() });
                    // the rows of a table follow each other without a gap
                    if ri > 0 {
                        if let Some(x) = self.raws.last_mut() {
                            x.space = 0;
                        }
                    }
                }
            }
            Leaf::Html(lines) => {
                self.file(Raw { kind: MB_PARA, src: lines.join("\n"), html: true, ..Raw::default() });
            }
        }
    }

    // close the containers from index n on (an item without a block files an empty one)
    fn close_conts(&mut self, n: usize) {
        if self.conts.len() <= n {
            return;
        }
        self.close_leaf();
        while self.conts.len() > n {
            let last = self.conts.len() - 1;
            if !self.conts[last].quote && !self.conts[last].filed {
                self.file(Raw { kind: MB_PARA, ..Raw::default() });
            }
            self.conts.pop();
        }
    }

    fn line(&mut self, raw: &str) {
        let ln = expand_tabs(raw);
        let mut p = 0usize;
        let mut matched = 0;
        // the open containers this line continues
        for k in 0..self.conts.len() {
            let rest = &ln[p..];
            let c = &self.conts[k];
            if c.quote {
                let sp = indent_of(rest);
                if sp <= 3 && rest.as_bytes().get(sp) == Some(&b'>') {
                    p += sp + 1;
                    if ln.as_bytes().get(p) == Some(&b' ') {
                        p += 1;
                    }
                    matched += 1;
                    continue;
                }
                break;
            }
            if blank(rest) {
                if c.empty && !c.filed {
                    break; // an item may begin with one blank line only
                }
                matched += 1;
                continue;
            }
            if indent_of(rest) >= c.indent {
                p += c.indent;
                matched += 1;
                continue;
            }
            break;
        }
        let rest = String::from(&ln[p..]);
        // a fence goes on while all its containers do
        if let Leaf::Fence { ch, len, indent, lines, .. } = &mut self.leaf {
            if matched == self.conts.len() {
                let sp = indent_of(&rest);
                let t = &rest[sp..];
                let n = t.bytes().take_while(|&b| b == *ch).count();
                if sp <= 3 && n >= *len && t[n..].trim().is_empty() {
                    self.close_leaf();
                    return;
                }
                let cut = sp.min(*indent);
                lines.push(String::from(&rest[cut..]));
                return;
            }
        }
        // a lazy continuation line of a paragraph
        if matched < self.conts.len() && matches!(self.leaf, Leaf::Para(_)) && !blank(&rest) && !Self::starts_block(&rest, true) {
            if let Leaf::Para(lines) = &mut self.leaf {
                lines.push(String::from(rest.trim_start()));
            }
            return;
        }
        self.close_conts(matched);
        let mut rest = rest;
        // new containers
        loop {
            if self.conts.len() >= NEST_MAX {
                break;
            }
            let sp = indent_of(&rest);
            if sp >= 4 && !matches!(self.leaf, Leaf::Para(_)) {
                break;
            }
            if sp <= 3 && rest.as_bytes().get(sp) == Some(&b'>') {
                self.close_leaf();
                let mut q = sp + 1;
                if rest.as_bytes().get(q) == Some(&b' ') {
                    q += 1;
                }
                rest = String::from(&rest[q..]);
                self.conts.push(Cont { quote: true, indent: 0, ordered: false, number: 0, delim: 0, task: 0, filed: true, empty: false });
                continue;
            }
            if is_rule(&rest) {
                break;
            }
            if let Some((ordered, ch, number, mlen)) = list_marker(&rest) {
                let after = &rest[mlen..];
                let empty = blank(after);
                if matches!(self.leaf, Leaf::Para(_)) && (empty || (ordered && number != 1)) {
                    break;
                }
                self.close_leaf();
                let pad = indent_of(after);
                let (indent, skip) = if empty {
                    (mlen + 1, after.len())
                } else if pad > 4 {
                    (mlen + 1, 1)
                } else {
                    (mlen + pad, pad)
                };
                let mut body = String::from(&after[skip.min(after.len())..]);
                // a task item: [ ] or [x]
                let mut task = 0;
                let bb = body.as_bytes();
                if bb.len() >= 3 && bb[0] == b'[' && bb[2] == b']' && matches!(bb[1], b' ' | b'x' | b'X') && (bb.len() == 3 || bb[3] == b' ') {
                    task = if bb[1] == b' ' { 1 } else { 2 };
                    body = String::from(body[3.min(body.len())..].trim_start());
                }
                self.conts.push(Cont {
                    quote: false,
                    indent,
                    ordered,
                    number,
                    delim: if ordered { ch } else { 0 },
                    task,
                    filed: false,
                    empty,
                });
                rest = body;
                if empty {
                    break;
                }
                continue;
            }
            break;
        }
        self.leaf_line(rest);
    }

    // the line's text starts another block (it cannot be a lazy continuation)
    // (lazy: the paragraph is not in the line's innermost container, so any list item starts
    // a block; else only one that may interrupt a paragraph)
    fn starts_block(s: &str, lazy: bool) -> bool {
        let sp = indent_of(s);
        if sp > 3 {
            return false;
        }
        let t = &s[sp..];
        t.starts_with('>')
            || is_rule(s)
            || atx(s).is_some()
            || fence_open(s).is_some()
            || list_marker(s).is_some_and(|m| lazy || (!blank(&s[m.3..]) && (!m.0 || m.2 == 1)))
            || html_start(s)
    }

    fn leaf_line(&mut self, rest: String) {
        if blank(&rest) {
            match &mut self.leaf {
                Leaf::Code(lines) => lines.push(String::new()),
                Leaf::None => {}
                _ => self.close_leaf(),
            }
            self.blank = true;
            return;
        }
        let sp = indent_of(&rest);
        if sp >= 4 {
            match &mut self.leaf {
                Leaf::Para(lines) => {
                    lines.push(String::from(rest.trim_start()));
                    return;
                }
                Leaf::Code(lines) => {
                    lines.push(String::from(&rest[4..]));
                    return;
                }
                Leaf::Table { .. } | Leaf::Html(_) => {}
                _ => {
                    self.close_leaf();
                    self.leaf = Leaf::Code(vec![String::from(&rest[4..])]);
                    return;
                }
            }
        }
        if matches!(self.leaf, Leaf::Code(_)) {
            self.close_leaf();
        }
        if let Leaf::Html(lines) = &mut self.leaf {
            lines.push(rest);
            return;
        }
        if let Some((ch, len, info)) = fence_open(&rest) {
            self.close_leaf();
            self.leaf = Leaf::Fence { ch, len, indent: sp, info, lines: Vec::new() };
            return;
        }
        if let Some((level, text)) = atx(&rest) {
            self.close_leaf();
            let src = String::from(text);
            self.file(Raw { kind: MB_HEADING, level, src, ..Raw::default() });
            return;
        }
        // a setext underline turns the paragraph into a heading
        if sp <= 3 {
            if let Leaf::Para(lines) = &self.leaf {
                let t = rest.trim();
                let lv = if !t.is_empty() && t.bytes().all(|b| b == b'=') {
                    1
                } else if !t.is_empty() && t.bytes().all(|b| b == b'-') {
                    2
                } else {
                    0
                };
                if lv > 0 {
                    let src = lines.join("\n");
                    let rest_src = take_defs(&src, &mut self.defs);
                    self.leaf = Leaf::None;
                    if rest_src.trim().is_empty() {
                        // only definitions: the line is text (or a rule)
                        if lv == 2 && is_rule(&rest) {
                            self.file(Raw { kind: MB_RULE, ..Raw::default() });
                        } else {
                            self.leaf = Leaf::Para(vec![String::from(rest.trim())]);
                        }
                        return;
                    }
                    self.file(Raw { kind: MB_HEADING, level: lv, src: String::from(rest_src.trim_end()), ..Raw::default() });
                    return;
                }
            }
        }
        if is_rule(&rest) {
            self.close_leaf();
            self.file(Raw { kind: MB_RULE, ..Raw::default() });
            return;
        }
        // a table: a paragraph's last line followed by its delimiter row
        if let Leaf::Para(lines) = &mut self.leaf {
            if let (Some(aligns), Some(last)) = (delim_row(&rest), lines.last()) {
                let head = split_cells(last);
                if head.len() == aligns.len() && last.contains('|') {
                    lines.pop();
                    if lines.is_empty() {
                        self.leaf = Leaf::None;
                    } else {
                        self.close_leaf();
                    }
                    self.table_cells = self.table_cells.saturating_add(aligns.len());
                    self.leaf = Leaf::Table { aligns, rows: vec![(head, true)] };
                    return;
                }
            }
        }
        if let Leaf::Table { aligns, rows } = &mut self.leaf {
            if !blank(&rest) && !Self::starts_block(&rest, false) {
                if rows.len() < BLOCKS_MAX && self.table_cells + aligns.len() <= CELLS_MAX {
                    self.table_cells += aligns.len();
                    rows.push((split_cells(&rest), false));
                } else {
                    self.truncated = true;
                }
                return;
            }
            self.close_leaf();
        }
        if html_start(&rest) {
            self.close_leaf();
            self.leaf = Leaf::Html(vec![rest]);
            return;
        }
        match &mut self.leaf {
            Leaf::Para(lines) => lines.push(String::from(rest.trim_start())),
            _ => {
                self.close_leaf();
                self.leaf = Leaf::Para(vec![String::from(rest.trim_start())]);
            }
        }
    }

    fn finish(mut self) -> (Vec<Raw>, BTreeMap<String, (String, String)>, Vec<MdTable>, bool) {
        self.close_conts(0);
        self.close_leaf();
        (self.raws, self.defs, self.tables, self.truncated)
    }
}

// ------------------------------------------------------------------ the inline parser

const N_TEXT: u8 = 0;
const N_CODE: u8 = 1;
const N_IMG: u8 = 2;
const N_SKIP: u8 = 3;

struct Node {
    t: String,
    k: u8,
    img: u32,
}

struct Delim {
    node: usize,
    ch: u8,
    n: usize,
    orig: usize,
    open: bool,
    close: bool,
}

struct Bracket {
    node: usize,
    src: usize,
    image: bool,
    // the intervals there were when it opened (those before end at or before its node)
    ivs: usize,
}

// a style or link over nodes a .. b
struct Iv {
    a: usize,
    b: usize,
    style: u16,
    link: u32,
    color: u32,
    bg: u32,
}

struct Inline<'a> {
    s: &'a str,
    b: &'a [u8],
    defs: &'a BTreeMap<String, (String, String)>,
    nodes: Vec<Node>,
    pend: String,
    delims: Vec<Delim>,
    brackets: Vec<Bracket>,
    // brackets below this index are no link openers any more (a link formed after them)
    dead: usize,
    ivs: Vec<Iv>,
    // inline HTML elements open: (tag, node, style, link)
    html: Vec<(String, usize, u16, u32)>,
    // backtick runs by length: their starts, ascending
    ticks: BTreeMap<usize, Vec<usize>>,
    // bytes the link and tag parsers may still scan (adversarial input stays linear)
    budget: usize,
    // a comment was not closed: no later one is
    no_cmt: bool,
    links: &'a mut Vec<String>,
    images: &'a mut Vec<MdImage>,
}

// the char before byte i / at byte i
fn char_before(s: &str, i: usize) -> Option<char> {
    s[..i].chars().next_back()
}

fn char_at(s: &str, i: usize) -> Option<char> {
    s.get(i..).and_then(|t| t.chars().next())
}

impl<'a> Inline<'a> {
    fn new(s: &'a str, defs: &'a BTreeMap<String, (String, String)>, links: &'a mut Vec<String>, images: &'a mut Vec<MdImage>) -> Inline<'a> {
        let b = s.as_bytes();
        let mut ticks: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'`' {
                let n = b[i..].iter().take_while(|&&c| c == b'`').count();
                ticks.entry(n).or_default().push(i);
                i += n;
            } else {
                i += 1;
            }
        }
        Inline {
            s,
            b,
            defs,
            nodes: Vec::new(),
            pend: String::new(),
            delims: Vec::new(),
            brackets: Vec::new(),
            dead: 0,
            ivs: Vec::new(),
            html: Vec::new(),
            ticks,
            budget: s.len() * 8 + 4096,
            no_cmt: false,
            links,
            images,
        }
    }

    fn flush(&mut self) {
        if !self.pend.is_empty() {
            let t = core::mem::take(&mut self.pend);
            self.nodes.push(Node { t, k: N_TEXT, img: 0 });
        }
    }

    fn push(&mut self, t: String, k: u8) -> usize {
        self.flush();
        self.nodes.push(Node { t, k, img: 0 });
        self.nodes.len() - 1
    }

    fn link_id(&mut self, url: String) -> u32 {
        self.links.push(url);
        self.links.len() as u32
    }

    fn run(&mut self) {
        let (s, b) = (self.s, self.b);
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            match c {
                b'\\' => {
                    if i + 1 < b.len() && b[i + 1].is_ascii_punctuation() {
                        self.pend.push(b[i + 1] as char);
                        i += 2;
                    } else if i + 1 < b.len() && b[i + 1] == b'\n' {
                        self.push(String::from("\n"), N_TEXT);
                        i += 2;
                        i += b[i..].iter().take_while(|&&c| c == b' ').count();
                    } else {
                        self.pend.push('\\');
                        i += 1;
                    }
                }
                b'`' => {
                    let n = b[i..].iter().take_while(|&&c| c == b'`').count();
                    let close = self.ticks.get(&n).and_then(|v| {
                        let k = v.partition_point(|&p| p <= i);
                        v.get(k).copied()
                    });
                    match close {
                        Some(j) => {
                            let mut t: String = s[i + n..j].chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
                            if t.len() >= 2 && t.starts_with(' ') && t.ends_with(' ') && !t.bytes().all(|c| c == b' ') {
                                t = String::from(&t[1..t.len() - 1]);
                            }
                            self.push(t, N_CODE);
                            i = j + n;
                        }
                        None => {
                            for _ in 0..n {
                                self.pend.push('`');
                            }
                            i += n;
                        }
                    }
                }
                b'*' | b'_' | b'~' => {
                    let n = b[i..].iter().take_while(|&&x| x == c).count();
                    let before = char_before(s, i);
                    let after = char_at(s, i + n);
                    let ws = |c: Option<char>| c.is_none_or(|c| c.is_whitespace());
                    let pu = |c: Option<char>| c.is_some_and(is_punct);
                    let left = !ws(after) && (!pu(after) || ws(before) || pu(before));
                    let right = !ws(before) && (!pu(before) || ws(after) || pu(after));
                    let (open, close) = if c == b'_' {
                        (left && (!right || pu(before)), right && (!left || pu(after)))
                    } else {
                        (left, right)
                    };
                    let node = self.push(String::from(&s[i..i + n]), N_TEXT);
                    if (open || close) && (c != b'~' || n <= 2) {
                        self.delims.push(Delim { node, ch: c, n, orig: n, open, close });
                    }
                    i += n;
                }
                b'[' => {
                    let node = self.push(String::from("["), N_TEXT);
                    self.brackets.push(Bracket { node, src: i + 1, image: false, ivs: self.ivs.len() });
                    i += 1;
                }
                b'!' if b.get(i + 1) == Some(&b'[') => {
                    let node = self.push(String::from("!["), N_TEXT);
                    self.brackets.push(Bracket { node, src: i + 2, image: true, ivs: self.ivs.len() });
                    i += 2;
                }
                b']' => i = self.close_bracket(i),
                b'<' => i = self.angle(i),
                b'&' => match entity_at(&s[i..]) {
                    Some((ch, n)) => {
                        self.pend.push(ch);
                        i += n;
                    }
                    None => {
                        self.pend.push('&');
                        i += 1;
                    }
                },
                b'\n' => {
                    let hard = self.pend.ends_with("  ");
                    let t = self.pend.trim_end_matches(' ').len();
                    self.pend.truncate(t);
                    self.push(String::from(if hard { "\n" } else { " " }), N_TEXT);
                    i += 1;
                    i += b[i..].iter().take_while(|&&c| c == b' ').count();
                }
                b'h' | b'H' | b'w' | b'W' => {
                    if let Some(n) = self.bare_link(i) {
                        i += n;
                    } else {
                        self.pend.push(c as char);
                        i += 1;
                    }
                }
                _ => {
                    // plain text up to the next special byte
                    let j = b[i..]
                        .iter()
                        .position(|&x| matches!(x, b'\\' | b'`' | b'*' | b'_' | b'~' | b'[' | b'!' | b']' | b'<' | b'&' | b'\n' | b'h' | b'H' | b'w' | b'W'))
                        .map_or(b.len(), |k| i + k.max(1));
                    let j = (j..=b.len()).find(|&k| s.is_char_boundary(k)).unwrap_or(b.len());
                    self.pend.push_str(&s[i..j]);
                    i = j;
                }
            }
        }
        self.flush();
    }

    // http:// https:// www. at i (after a blank or the start) -> bytes taken
    fn bare_link(&mut self, i: usize) -> Option<usize> {
        let (s, b) = (self.s, self.b);
        if let Some(p) = char_before(s, i) {
            if !(p.is_whitespace() || matches!(p, '(' | '*' | '_' | '~' | '"' | '\'')) {
                return None;
            }
        }
        let rest = &b[i..];
        let pre = |p: &[u8]| rest.len() > p.len() && rest[..p.len()].eq_ignore_ascii_case(p);
        let www = pre(b"www.");
        if !(www || pre(b"http://") || pre(b"https://")) {
            return None;
        }
        let mut e = i;
        while e < b.len() && !b[e].is_ascii_whitespace() && b[e] != b'<' {
            e += 1;
        }
        while !s.is_char_boundary(e) {
            e -= 1;
        }
        // trailing punctuation is no part of it, nor an unbalanced ')' (the counts are kept, the
        // trimmed punctuation holds no parentheses)
        let opens = s[i..e].matches('(').count();
        let mut closes = s[i..e].matches(')').count();
        loop {
            let t = &s[i..e];
            let Some(last) = t.chars().next_back() else { break };
            if matches!(last, '?' | '!' | '.' | ',' | ':' | '*' | '_' | '~' | '\'' | '"' | ';') {
                e -= last.len_utf8();
                continue;
            }
            if last == ')' && closes > opens {
                closes -= 1;
                e -= 1;
                continue;
            }
            break;
        }
        let t = &s[i..e];
        let host = if www { t.get(4..).unwrap_or("") } else { t.split("://").nth(1).unwrap_or("") };
        if host.is_empty() || (www && !host.contains('.')) {
            return None;
        }
        let url = if www { format!("http://{}", t) } else { String::from(t) };
        let link = self.link_id(url);
        let node = self.push(String::from(t), N_TEXT);
        self.ivs.push(Iv { a: node, b: node + 1, style: 0, link, color: 0, bg: 0 });
        Some(e - i)
    }

    // ']' at i: a link or image, or the text "]" -> the next byte
    fn close_bracket(&mut self, i: usize) -> usize {
        let Some(br) = self.brackets.pop() else {
            self.pend.push(']');
            return i + 1;
        };
        // a link formed after this bracket opened: it opens no link
        let idx = self.brackets.len();
        let dead = idx < self.dead;
        self.dead = self.dead.min(idx);
        if dead && !br.image {
            self.pend.push(']');
            return i + 1;
        }
        let (s, b) = (self.s, self.b);
        let mut found: Option<(String, String, usize)> = None;
        // inline: (dest "title")
        if b.get(i + 1) == Some(&b'(') && self.budget > 0 {
            let start = i + 2;
            let r = (|| {
                let mut k = start;
                let ws = |k: &mut usize| {
                    while *k < b.len() && (b[*k] == b' ' || b[*k] == b'\n') {
                        *k += 1;
                    }
                };
                ws(&mut k);
                let (dest, mut k) = if b.get(k) == Some(&b')') { (String::new(), k) } else { parse_dest(s, k)? };
                let before = k;
                ws(&mut k);
                let mut title = String::new();
                if k > before && k < b.len() && matches!(b[k], b'"' | b'\'' | b'(') {
                    let (t, e) = parse_title(s, k)?;
                    title = t;
                    k = e;
                    ws(&mut k);
                }
                (b.get(k) == Some(&b')')).then_some((dest, title, k + 1))
            })();
            // what the attempt scanned counts against the budget
            let used = r.as_ref().map_or(b.len() - start, |x| x.2 - start);
            self.budget = self.budget.saturating_sub(used.min(b.len()));
            found = r;
        }
        // references: [label], [], or the text itself
        if found.is_none() && i - br.src <= LABEL_MAX && !self.defs.is_empty() && self.budget > 0 {
            // (nested brackets would read the same label bytes again and again)
            self.budget = self.budget.saturating_sub(i - br.src + 1);
            let text_label = &s[br.src..i];
            let mut label = String::from(text_label);
            let mut end = i + 1;
            if b.get(i + 1) == Some(&b'[') {
                let lim = (i + 2 + LABEL_MAX + 1).min(b.len());
                if let Some(k) = b[i + 2..lim].iter().position(|&c| c == b']' || c == b'[') {
                    let k = i + 2 + k;
                    if b[k] == b']' {
                        if k > i + 2 {
                            label = String::from(&s[i + 2..k]);
                        }
                        end = k + 1;
                    }
                }
            }
            if let Some((d, t)) = self.defs.get(&norm_label(&label)) {
                found = Some((d.clone(), t.clone(), end));
            }
        }
        let Some((dest, title, end)) = found else {
            self.pend.push(']');
            return i + 1;
        };
        self.flush();
        let a = br.node + 1;
        if br.image {
            // the alt text: the plain text inside (without delimiter runs); the nodes give way
            // to one image (they go: nested images stay linear)
            let first = self.delims.partition_point(|d| d.node < a);
            let runs: Vec<usize> = self.delims[first..].iter().map(|d| d.node).collect();
            let mut alt = String::new();
            for (k, n) in self.nodes[a..].iter().enumerate() {
                if (n.k == N_TEXT || n.k == N_CODE) && runs.binary_search(&(a + k)).is_err() {
                    alt.push_str(&n.t);
                }
            }
            self.nodes.truncate(a);
            let tail = self.ivs.split_off(br.ivs.min(self.ivs.len()));
            self.ivs.extend(tail.into_iter().filter(|iv| iv.a < a).map(|mut iv| {
                iv.b = iv.b.min(a);
                iv
            }));
            self.html.retain(|h| h.1 < a);
            self.delims.truncate(first);
            self.images.push(MdImage { src: dest, alt, title, w: 0, h: 0 });
            let img = self.images.len() as u32;
            let nd = &mut self.nodes[br.node];
            nd.k = N_IMG;
            nd.img = img;
            nd.t = String::from("\u{FFFC}");
        } else {
            let link = self.link_id(dest);
            self.nodes[br.node].k = N_SKIP;
            let bn = self.nodes.len();
            self.ivs.push(Iv { a, b: bn, style: 0, link, color: 0, bg: 0 });
            // no links inside links: the brackets before it open none
            self.dead = self.brackets.len();
        }
        end
    }

    // '<' at i: an autolink, inline HTML or the text "<" -> the next byte
    fn angle(&mut self, i: usize) -> usize {
        let (s, b) = (self.s, self.b);
        let lim = (i + 2048).min(b.len());
        let end = b[i + 1..lim].iter().position(|&c| c == b'>' || c == b'<' || c == b'\n').map(|k| i + 1 + k);
        if let Some(e) = end.filter(|&e| b[e] == b'>') {
            let inner = &s[i + 1..e];
            // autolinks: scheme:... or an e-mail address
            let sch = inner.bytes().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-')).count();
            if (2..=32).contains(&sch) && inner.as_bytes()[0].is_ascii_alphabetic() && inner.as_bytes().get(sch) == Some(&b':') && !inner.contains(' ') {
                let link = self.link_id(String::from(inner));
                let node = self.push(String::from(inner), N_TEXT);
                self.ivs.push(Iv { a: node, b: node + 1, style: 0, link, color: 0, bg: 0 });
                return e + 1;
            }
            if let Some((user, host)) = inner.split_once('@') {
                if !user.is_empty() && host.contains('.') && !inner.contains(' ') && !host.contains('@') {
                    let link = self.link_id(format!("mailto:{}", inner));
                    let node = self.push(String::from(inner), N_TEXT);
                    self.ivs.push(Iv { a: node, b: node + 1, style: 0, link, color: 0, bg: 0 });
                    return e + 1;
                }
            }
        }
        // comments (once one is not closed, no later one is)
        if s[i..].starts_with("<!--") && !self.no_cmt {
            match s[i + 4..].find("-->") {
                Some(k) => return i + 4 + k + 3,
                None => self.no_cmt = true,
            }
        }
        // a tag (quotes respected; the scans count against the budget)
        if self.budget == 0 {
            self.pend.push('<');
            return i + 1;
        }
        let mut q = 0u8;
        let mut gt = None;
        let mut scanned = 0;
        for (k, &c) in b[i + 1..lim].iter().enumerate() {
            scanned = k + 1;
            if q != 0 {
                if c == q {
                    q = 0;
                }
            } else if c == b'"' || c == b'\'' {
                q = c;
            } else if c == b'>' {
                gt = Some(i + 1 + k);
                break;
            } else if c == b'<' {
                break;
            }
        }
        self.budget = self.budget.saturating_sub(scanned);
        let Some(gt) = gt else {
            self.pend.push('<');
            return i + 1;
        };
        let inner = &s[i + 1..gt];
        let closing = inner.starts_with('/');
        let body = inner.trim_start_matches('/');
        let nl = body.bytes().take_while(|c| c.is_ascii_alphanumeric()).count();
        if nl == 0 || !body.as_bytes()[0].is_ascii_alphabetic() {
            self.pend.push('<');
            return i + 1;
        }
        let name = body[..nl].to_ascii_lowercase();
        let style = match name.as_str() {
            "b" | "strong" => MS_BOLD,
            "i" | "em" | "cite" => MS_ITALIC,
            "s" | "del" | "strike" => MS_STRIKE,
            "u" | "ins" => MS_UNDER,
            "code" | "kbd" | "tt" | "samp" => MS_CODE,
            _ => 0,
        };
        match name.as_str() {
            "br" => {
                self.push(String::from("\n"), N_TEXT);
            }
            "img" if !closing => {
                let at = attrs(&body[nl..]);
                let get = |k: &str| at.iter().find(|a| a.0 == k).map(|a| a.1.clone()).unwrap_or_default();
                let num = |k: &str| get(k).trim().trim_end_matches("px").parse::<u32>().unwrap_or(0).min(100_000);
                self.images.push(MdImage { src: get("src"), alt: get("alt"), title: get("title"), w: num("width"), h: num("height") });
                let img = self.images.len() as u32;
                let node = self.push(String::from("\u{FFFC}"), N_IMG);
                self.nodes[node].img = img;
            }
            _ if style != 0 || name == "a" => {
                self.flush();
                if closing {
                    if let Some(k) = self.html.iter().rposition(|h| h.0 == name) {
                        let (_, a, st, link) = self.html.remove(k);
                        let bn = self.nodes.len();
                        if a < bn {
                            self.ivs.push(Iv { a, b: bn, style: st, link, color: 0, bg: 0 });
                        }
                    }
                } else if self.html.len() < HTML_STACK {
                    let link = if name == "a" {
                        let at = attrs(&body[nl..]);
                        match at.iter().find(|a| a.0 == "href") {
                            Some(h) => self.link_id(h.1.clone()),
                            None => 0,
                        }
                    } else {
                        0
                    };
                    let a = self.nodes.len();
                    self.html.push((name, a, style, link));
                }
            }
            _ => {}
        }
        gt + 1
    }

    // the delimiter runs become emphasis (CommonMark's "process emphasis")
    fn emphasis(&mut self) {
        let n = self.delims.len();
        if n == 0 {
            return;
        }
        const NONE: usize = usize::MAX;
        let mut prev: Vec<usize> = (0..n).map(|i| if i == 0 { NONE } else { i - 1 }).collect();
        let mut next: Vec<usize> = (0..n).map(|i| if i + 1 < n { i + 1 } else { NONE }).collect();
        let mut gone = vec![false; n];
        // the lowest opener worth looking at, by char, length % 3, closer can open
        let mut bottom = [[[0usize; 2]; 3]; 3];
        let ci = |c: u8| match c {
            b'*' => 0,
            b'_' => 1,
            _ => 2,
        };
        let unlink = |k: usize, prev: &mut Vec<usize>, next: &mut Vec<usize>, gone: &mut Vec<bool>| {
            gone[k] = true;
            let (p, x) = (prev[k], next[k]);
            if p != NONE {
                next[p] = x;
            }
            if x != NONE {
                prev[x] = p;
            }
        };
        let mut c = 0;
        while c != NONE {
            if gone[c] || !self.delims[c].close {
                c = next[c];
                continue;
            }
            let (ch, cn, corig, copen) = {
                let d = &self.delims[c];
                (d.ch, d.n, d.orig, d.open)
            };
            let key = (ci(ch), corig % 3, copen as usize);
            let lim = bottom[key.0][key.1][key.2];
            let mut o = prev[c];
            let mut found = NONE;
            while o != NONE && o >= lim {
                let d = &self.delims[o];
                if !gone[o] && d.open && d.ch == ch {
                    let three = ch != b'~' && (d.close || copen) && (d.orig + corig) % 3 == 0 && !(d.orig % 3 == 0 && corig % 3 == 0);
                    let tilde = ch == b'~' && d.n != cn;
                    if !three && !tilde {
                        found = o;
                        break;
                    }
                }
                o = prev[o];
            }
            if found == NONE {
                bottom[key.0][key.1][key.2] = c;
                let nx = next[c];
                if !copen {
                    unlink(c, &mut prev, &mut next, &mut gone);
                }
                c = nx;
                continue;
            }
            let o = found;
            let used = if ch == b'~' { cn } else if cn >= 2 && self.delims[o].n >= 2 { 2 } else { 1 };
            let style = if ch == b'~' { MS_STRIKE } else if used == 2 { MS_BOLD } else { MS_ITALIC };
            let (on, cnode) = (self.delims[o].node, self.delims[c].node);
            self.ivs.push(Iv { a: on + 1, b: cnode, style, link: 0, color: 0, bg: 0 });
            // the runs between them are text now
            let mut k = next[o];
            while k != NONE && k != c {
                let nx = next[k];
                unlink(k, &mut prev, &mut next, &mut gone);
                k = nx;
            }
            self.delims[o].n -= used;
            let t = &mut self.nodes[on].t;
            t.truncate(t.len() - used);
            self.delims[c].n -= used;
            self.nodes[cnode].t.drain(..used);
            if self.delims[o].n == 0 {
                unlink(o, &mut prev, &mut next, &mut gone);
            }
            if self.delims[c].n == 0 {
                let nx = next[c];
                unlink(c, &mut prev, &mut next, &mut gone);
                c = nx;
            }
        }
    }

    // the text and its spans
    fn output(mut self) -> (String, Vec<MdSpan>) {
        self.emphasis();
        let n = self.nodes.len();
        // events by node: (node, start?, interval)
        let mut ev: Vec<(usize, bool, usize)> = Vec::with_capacity(self.ivs.len() * 2);
        for (k, iv) in self.ivs.iter().enumerate() {
            if iv.a < iv.b && iv.a < n {
                ev.push((iv.a, true, k));
                ev.push((iv.b.min(n), false, k));
            }
        }
        ev.sort_by_key(|e| (e.0, e.1));
        let mut counts = [0u32; 16];
        let mut links: Vec<usize> = Vec::new();
        let mut text = String::new();
        let mut spans: Vec<MdSpan> = Vec::new();
        let mut ei = 0;
        for (k, node) in self.nodes.iter().enumerate() {
            while ei < ev.len() && ev[ei].0 == k {
                let (_, start, iv) = ev[ei];
                let v = &self.ivs[iv];
                for (bit, c) in counts.iter_mut().enumerate() {
                    if v.style & (1 << bit) != 0 {
                        if start {
                            *c += 1;
                        } else {
                            *c = c.saturating_sub(1);
                        }
                    }
                }
                if v.link != 0 || v.color != 0 || v.bg != 0 {
                    if start {
                        links.push(iv);
                    } else if let Some(p) = links.iter().rposition(|&x| x == iv) {
                        links.remove(p);
                    }
                }
                ei += 1;
            }
            if node.k == N_SKIP || (node.t.is_empty() && node.k != N_IMG) {
                continue;
            }
            let mut style = 0u16;
            for (bit, &c) in counts.iter().enumerate() {
                if c > 0 {
                    style |= 1 << bit;
                }
            }
            if node.k == N_CODE {
                style |= MS_CODE;
            }
            let link = links.iter().rev().map(|&i| self.ivs[i].link).find(|&l| l != 0).unwrap_or(0);
            let a = text.len() as u32;
            text.push_str(&node.t);
            let b = text.len() as u32;
            let img = if node.k == N_IMG { node.img } else { 0 };
            if style == 0 && link == 0 && img == 0 {
                continue;
            }
            match spans.last_mut() {
                Some(l) if l.b == a && l.style == style && l.link == link && img == 0 && l.image == 0 => l.b = b,
                _ => spans.push(MdSpan { a, b, style, link, image: img, color: 0, bg: 0, field: 0 }),
            }
        }
        (text, spans)
    }
}

// inline Markdown -> (text, spans)
fn inline(src: &str, defs: &BTreeMap<String, (String, String)>, links: &mut Vec<String>, images: &mut Vec<MdImage>) -> (String, Vec<MdSpan>) {
    let mut p = Inline::new(src, defs, links, images);
    p.run();
    p.output()
}

// ------------------------------------------------------------------ HTML

// paragraphs of the HTML reader -> blocks (inside `quote` / `list` containers of Markdown)
fn html_blocks(s: &str, doc: &mut MdDoc, quote: u8, list: u8, path: u32, first_space: u8) {
    let depth = (quote + list) as u32;
    // the HTML's own quotes inside the Markdown's containers, its lists inside them
    let path_of = |dq: u8| path | (((1u64 << dq.min(31)) - 1) << depth.min(31)) as u32;
    let HRead { paras, ext, forms, fields } = html_read(s);
    // the forms and fields (MdField::form and HExt::Field::n count from the ones before)
    let (form0, field0) = (doc.forms.len() as u32, doc.fields.len() as u32);
    let room = FIELDS_MAX.saturating_sub(doc.fields.len());
    doc.forms.extend(forms);
    doc.fields.extend(fields.into_iter().take(room).map(|mut f| {
        if f.form > 0 {
            f.form += form0;
        }
        f
    }));
    let mut by: BTreeMap<usize, Vec<&HExt>> = BTreeMap::new();
    for (at, e) in &ext {
        by.entry(*at).or_default().push(e);
    }
    let mut table_map: BTreeMap<u32, u32> = BTreeMap::new();
    let mut pre_open: Option<(u32, usize)> = None; // (pre block, its MdBlock index)
    let n = paras.len();
    let mut first = true;
    let space = |first: &mut bool, s: u8| -> u8 {
        if core::mem::take(first) { first_space } else { s }
    };
    for k in 0..=n {
        let items: Vec<&HExt> = by.get(&k).cloned().unwrap_or_default();
        if items.iter().any(|e| matches!(e, HExt::Rule)) {
            let sp = space(&mut first, 2);
            doc.push_block(MdBlock { kind: MB_RULE, quote, list, path, space: sp, ..MdBlock::default() });
            pre_open = None;
        }
        if k == n {
            break;
        }
        let p = &paras[k];
        let mut b = MdBlock { quote, list, path, ..MdBlock::default() };
        // the images and fields of the paragraph by byte (their U+FFFC gets a span of its own):
        // byte, index + 1, a field
        let mut imgs: Vec<(usize, u32, bool)> = Vec::new();
        for e in &items {
            match e {
                HExt::Image { byte, src, alt, w, h } => {
                    doc.images.push(MdImage { src: src.clone(), alt: alt.clone(), title: String::new(), w: *w, h: *h });
                    imgs.push((*byte, doc.images.len() as u32, false));
                }
                HExt::Field { byte: Some(byte), n } if *n < room => imgs.push((*byte, field0 + *n as u32 + 1, true)),
                _ => {}
            }
        }
        imgs.sort_unstable();
        // text and spans from the runs (one link index for the runs of one link)
        let mut ii = 0;
        let mut last_link: Option<(usize, u32)> = None;
        for (t, f) in p.runs() {
            let a0 = b.text.len();
            b.text.push_str(t);
            let e0 = b.text.len();
            let mut style = 0;
            if f.flags & RF_BOLD != 0 {
                style |= MS_BOLD;
            }
            if f.flags & RF_ITALIC != 0 {
                style |= MS_ITALIC;
            }
            if f.flags & RF_UNDER != 0 {
                style |= MS_UNDER;
            }
            if f.flags & RF_STRIKE != 0 {
                style |= MS_STRIKE;
            }
            if f.font.eq_ignore_ascii_case("monospace") {
                style |= MS_CODE;
            }
            let link = if f.link.is_empty() {
                0
            } else {
                match last_link {
                    Some((end, l)) if end == a0 && doc.link(l) == f.link => l,
                    _ => {
                        doc.links.push(f.link.clone());
                        doc.links.len() as u32
                    }
                }
            };
            last_link = (link != 0).then_some((e0, link));
            let styled = style != 0 || link != 0 || f.color != 0 || f.bg != 0;
            let span = |a: usize, e: usize, image: u32| MdSpan { a: a as u32, b: e as u32, style, link, image, color: f.color, bg: f.bg, field: 0 };
            let mut s0 = a0;
            while ii < imgs.len() && imgs[ii].0 < e0 {
                let (ib, obj, field) = imgs[ii];
                ii += 1;
                if ib >= s0 && b.text.get(ib..ib + 3) == Some("\u{FFFC}") {
                    if ib > s0 && styled {
                        b.spans.push(span(s0, ib, 0));
                    }
                    if field {
                        // (a field has no style or link of its own)
                        b.spans.push(MdSpan { a: ib as u32, b: ib as u32 + 3, field: obj, ..MdSpan::default() });
                    } else {
                        b.spans.push(span(ib, ib + 3, obj));
                    }
                    s0 = ib + 3;
                }
            }
            if s0 < e0 && styled {
                b.spans.push(span(s0, e0, 0));
            }
        }
        if p.style == PS_QUOTE {
            b.quote = quote + 1;
        }
        b.align = match p.align {
            RA_CENTER => 1,
            RA_RIGHT => 2,
            _ => 0,
        };
        if p.list != LS_NONE {
            b.list = list + p.level + 1;
        }
        let mut sp = 2;
        let mut pre = None;
        for e in &items {
            match e {
                HExt::Heading(l) => {
                    b.kind = MB_HEADING;
                    b.level = *l;
                }
                HExt::Quote(d) => b.quote = quote + d,
                HExt::Item(num) => {
                    b.marker = if *num > 0 { MK_NUMBER } else { MK_BULLET };
                    b.number = *num;
                    b.delim = b'.';
                    sp = if b.list > 0 && doc.blocks.last().is_some_and(|l| l.list > 0) { 1 } else { 2 };
                }
                HExt::Task(done) => b.task = if *done { 2 } else { 1 },
                HExt::Pre { block, lang } => pre = Some((*block, lang.clone())),
                HExt::Row { table, head, cells } => {
                    let next_t = doc.tables.len() as u32;
                    let t = *table_map.entry(*table).or_insert(next_t);
                    if t == next_t {
                        doc.tables.push(MdTable { aligns: Vec::new() });
                    }
                    b.kind = MB_ROW;
                    b.table = t;
                    b.head = *head;
                    let len = b.text.len();
                    for (ci, &(start, al)) in cells.iter().enumerate().take(COLS_MAX) {
                        let end = cells.get(ci + 1).map_or(len, |c| c.0.saturating_sub(1));
                        b.cells.push((start.min(len) as u32, end.max(start).min(len) as u32));
                        let tb = &mut doc.tables[t as usize];
                        if tb.aligns.len() <= ci {
                            tb.aligns.resize(ci + 1, MA_NONE);
                        }
                        if al != 0 && tb.aligns[ci] == MA_NONE {
                            tb.aligns[ci] = al;
                        }
                    }
                    sp = if doc.blocks.last().is_some_and(|l| l.kind == MB_ROW && l.table == t) { 0 } else { 2 };
                }
                _ => {}
            }
        }
        if b.list > 0 && b.marker == MK_NONE && p.list != LS_NONE {
            sp = 1;
        }
        b.path = path_of(b.quote.saturating_sub(quote));
        // the lines of one <pre> make one code block
        if let Some((blk, lang)) = pre {
            match pre_open {
                Some((pb, idx)) if pb == blk && idx < doc.blocks.len() => {
                    let cb = &mut doc.blocks[idx];
                    cb.text.push('\n');
                    cb.text.push_str(&expand_tabs(&b.text));
                }
                _ => {
                    b.text = expand_tabs(&b.text);
                    b.kind = MB_CODE;
                    b.lang = lang;
                    b.spans.clear();
                    b.space = space(&mut first, 2);
                    doc.push_block(b);
                    pre_open = Some((blk, doc.blocks.len() - 1));
                }
            }
            continue;
        }
        pre_open = None;
        if b.kind == MB_ROW {
            trim_cells(&mut b);
        }
        b.space = space(&mut first, sp);
        doc.push_block(b);
    }
    // a line end before </pre> shows no line
    for b in doc.blocks.iter_mut().filter(|b| b.kind == MB_CODE) {
        while b.text.ends_with('\n') {
            b.text.pop();
        }
    }
}

// the cells of a row without the blanks around them
fn trim_cells(b: &mut MdBlock) {
    for c in b.cells.iter_mut() {
        let t = &b.text[c.0 as usize..c.1 as usize];
        let lead = t.len() - t.trim_start().len();
        let trail = t.len() - t.trim_end().len();
        c.0 += lead as u32;
        c.1 = c.1.saturating_sub(trail as u32).max(c.0);
    }
}

// ------------------------------------------------------------------ the document

impl MdDoc {
    /// a document from Markdown (see the module header)
    pub fn markdown(s: &str) -> MdDoc {
        let mut doc = MdDoc::default();
        let mut s = s;
        if s.len() > INPUT_MAX {
            let mut k = INPUT_MAX;
            while !s.is_char_boundary(k) {
                k -= 1;
            }
            s = &s[..k];
            doc.truncated = true;
        }
        let s = s.strip_prefix('\u{FEFF}').unwrap_or(s);
        let mut bp = BlockParser::new();
        let mut rest = s;
        while !rest.is_empty() {
            let (line, next) = match rest.find(['\n', '\r']) {
                Some(i) => {
                    let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
                    (&rest[..i], &rest[i + skip..])
                }
                None => (rest, ""),
            };
            if line.contains('\0') {
                bp.line(&line.replace('\0', "\u{FFFD}"));
            } else {
                bp.line(line);
            }
            rest = next;
        }
        let (raws, defs, tables, trunc) = bp.finish();
        doc.truncated |= trunc;
        doc.tables = tables;
        for r in raws {
            if r.html {
                html_blocks(&r.src, &mut doc, r.quote, r.list, r.path, r.space);
                continue;
            }
            let mut b = MdBlock {
                kind: r.kind,
                level: r.level,
                quote: r.quote,
                list: r.list,
                path: r.path,
                marker: r.marker,
                number: r.number,
                delim: r.delim,
                task: r.task,
                space: r.space,
                lang: r.lang,
                table: r.table,
                head: r.head,
                ..MdBlock::default()
            };
            match r.kind {
                MB_CODE => b.text = r.src,
                MB_RULE => {}
                MB_ROW => {
                    for (ci, c) in r.cells.iter().enumerate() {
                        if ci > 0 {
                            b.text.push('\t');
                        }
                        let (t, spans) = inline(c, &defs, &mut doc.links, &mut doc.images);
                        let off = b.text.len() as u32;
                        let t = t.replace('\t', " ").replace('\n', " ");
                        b.text.push_str(&t);
                        b.cells.push((off, b.text.len() as u32));
                        b.spans.extend(spans.into_iter().map(|mut s| {
                            s.a += off;
                            s.b += off;
                            s
                        }));
                    }
                }
                _ => {
                    let (t, spans) = inline(&r.src, &defs, &mut doc.links, &mut doc.images);
                    b.text = t;
                    b.spans = spans;
                }
            }
            doc.push_block(b);
        }
        doc.finish();
        doc
    }

    /// a document from HTML: what the rich-text editor reads (paragraphs, headings h1 .. h6,
    /// lists, block quotes, formats, links, colours) and rules, images, preformatted blocks
    /// (<pre><code class="language-x">), tables and forms (see the module header)
    pub fn html(s: &str) -> MdDoc {
        let mut doc = MdDoc::default();
        let mut s = s;
        if s.len() > INPUT_MAX {
            let mut k = INPUT_MAX;
            while !s.is_char_boundary(k) {
                k -= 1;
            }
            s = &s[..k];
            doc.truncated = true;
        }
        html_blocks(s, &mut doc, 0, 0, 0, 0);
        doc.finish();
        doc
    }

    fn push_block(&mut self, b: MdBlock) {
        if self.blocks.len() < BLOCKS_MAX {
            self.blocks.push(b);
        } else {
            self.truncated = true;
        }
    }

    // the links and fields in reading order
    fn finish(&mut self) {
        if let Some(b) = self.blocks.first_mut() {
            b.space = 0;
        }
        self.spots.clear();
        self.fspots.clear();
        for (k, b) in self.blocks.iter().enumerate() {
            for s in &b.spans {
                if s.field != 0 && (s.field as usize) <= self.fields.len() {
                    self.fspots.push(MdFieldSpot { block: k, a: s.a, field: s.field });
                }
                if s.link == 0 {
                    continue;
                }
                match self.spots.last_mut() {
                    Some(l) if l.block == k && l.link == s.link && l.b == s.a => l.b = s.b,
                    _ => self.spots.push(MdLinkSpot { block: k, a: s.a, b: s.b, link: s.link }),
                }
            }
        }
    }

    /// the blocks in reading order
    pub fn blocks(&self) -> &[MdBlock] {
        &self.blocks
    }

    /// the link targets (MdSpan::link - 1)
    pub fn links(&self) -> &[String] {
        &self.links
    }

    /// the target of MdSpan::link (index + 1); "" for 0
    pub fn link(&self, link: u32) -> &str {
        if link == 0 { "" } else { self.links.get(link as usize - 1).map_or("", |s| s.as_str()) }
    }

    /// the images (MdSpan::image - 1)
    pub fn images(&self) -> &[MdImage] {
        &self.images
    }

    /// the tables (MdBlock::table)
    pub fn tables(&self) -> &[MdTable] {
        &self.tables
    }

    /// every link in reading order (adjacent spans of one link joined)
    pub fn link_spots(&self) -> &[MdLinkSpot] {
        &self.spots
    }

    /// the forms (MdField::form - 1)
    pub fn forms(&self) -> &[MdForm] {
        &self.forms
    }

    /// the fields of the forms in document order (MdSpan::field - 1)
    pub fn fields(&self) -> &[MdField] {
        &self.fields
    }

    /// every visible field in reading order
    pub fn field_spots(&self) -> &[MdFieldSpot] {
        &self.fspots
    }

    /// a limit cut the input (size, blocks)
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// the place after the last character
    pub fn end(&self) -> TextPos {
        let n = self.blocks.len();
        if n == 0 { TextPos::default() } else { TextPos::new(n - 1, self.blocks[n - 1].text.len()) }
    }

    /// the whole document as text
    pub fn plain(&self) -> String {
        self.plain_range(TextPos::default(), self.end())
    }

    /// the whole document as HTML
    pub fn to_html(&self) -> String {
        self.html_range(TextPos::default(), self.end())
    }

    // the text of block k from a to b with images as their alt texts, buttons as their captions
    // (other fields as nothing: their values are the view's)
    fn block_text(&self, k: usize, a: usize, b: usize) -> String {
        let bl = &self.blocks[k];
        let t = &bl.text;
        let (a, b) = (clamp_char(t, a), clamp_char(t, b.max(a)));
        let mut out = String::new();
        let mut i = a;
        for s in &bl.spans {
            if (s.image == 0 && s.field == 0) || (s.b as usize) <= a || (s.a as usize) >= b {
                continue;
            }
            out.push_str(&t[i..s.a as usize]);
            if s.image != 0 {
                out.push_str(&self.images.get(s.image as usize - 1).map_or(String::new(), |m| m.alt.clone()));
            } else if let Some(f) = self.fields.get(s.field as usize - 1) {
                out.push_str(f.caption());
            }
            i = s.b as usize;
        }
        out.push_str(&t[i.min(b)..b]);
        out
    }

    /// the text of a .. b: blocks on lines of their own (a blank line before paragraphs),
    /// list markers ("- ", "1. ", "[x] "), cells by tabs, images as their alt texts, buttons
    /// as their captions (the other fields: nothing)
    pub fn plain_range(&self, a: TextPos, b: TextPos) -> String {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out = String::new();
        if self.blocks.is_empty() {
            return out;
        }
        let last = b.line.min(self.blocks.len() - 1);
        for k in a.line..=last {
            let bl = &self.blocks[k];
            let s = if k == a.line { a.byte } else { 0 };
            let e = if k == b.line { b.byte } else { bl.text.len() };
            if k > a.line {
                out.push('\n');
                if bl.space >= 2 {
                    out.push('\n');
                }
            }
            if s == 0 {
                for _ in 1..bl.list {
                    out.push_str("  ");
                }
                match bl.marker {
                    MK_BULLET => out.push_str("- "),
                    MK_NUMBER => out.push_str(&format!("{}{} ", bl.number, bl.delim as char)),
                    _ if bl.list > 0 => out.push_str("  "),
                    _ => {}
                }
                match bl.task {
                    1 => out.push_str("[ ] "),
                    2 => out.push_str("[x] "),
                    _ => {}
                }
                if bl.kind == MB_RULE && k != last {
                    out.push_str("---");
                }
            }
            out.push_str(&self.block_text(k, s, e));
        }
        out
    }

    /// the HTML of a .. b: h1 .. h6, p, ul / ol / li (task items with a disabled check box),
    /// blockquote, pre / code (class language-x), hr, table / tr / th / td (align), strong,
    /// em, del, u, code, a href, img src alt, colours as span styles; of forms only the
    /// buttons' captions (as text)
    pub fn html_range(&self, a: TextPos, b: TextPos) -> String {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out = String::new();
        if self.blocks.is_empty() {
            return out;
        }
        let last = b.line.min(self.blocks.len() - 1);
        // open containers: quotes, lists (ordered?), the open <li>, the table
        let mut quotes = 0u8;
        let mut lists: Vec<(bool, bool)> = Vec::new();
        let mut table: Option<u32> = None;
        for k in a.line..=last {
            let bl = &self.blocks[k];
            let s = if k == a.line { a.byte } else { 0 };
            let e = if k == b.line { b.byte } else { bl.text.len() };
            if bl.kind != MB_ROW || table != Some(bl.table) {
                if table.take().is_some() {
                    out.push_str("</table>");
                }
            }
            // lists deeper than this block's close (an item of the other kind closes its list)
            loop {
                let depth = bl.list as usize;
                let pop = match lists.last() {
                    Some(_) if lists.len() > depth => true,
                    Some(l) if lists.len() == depth && bl.marker != MK_NONE => l.0 != (bl.marker == MK_NUMBER),
                    _ => false,
                };
                if !pop {
                    break;
                }
                if let Some((ord, li)) = lists.pop() {
                    if li {
                        out.push_str("</li>");
                    }
                    out.push_str(if ord { "</ol>" } else { "</ul>" });
                }
            }
            while quotes > bl.quote {
                out.push_str("</blockquote>");
                quotes -= 1;
            }
            while quotes < bl.quote {
                out.push_str("<blockquote>");
                quotes += 1;
            }
            while lists.len() < bl.list as usize {
                let ord = bl.marker == MK_NUMBER && lists.len() + 1 == bl.list as usize;
                if ord && bl.number != 1 {
                    out.push_str(&format!("<ol start=\"{}\">", bl.number));
                } else {
                    out.push_str(if ord { "<ol>" } else { "<ul>" });
                }
                lists.push((ord, false));
            }
            if bl.marker != MK_NONE {
                if let Some(l) = lists.last_mut() {
                    if l.1 {
                        out.push_str("</li>");
                    }
                    out.push_str("<li>");
                    l.1 = true;
                }
                match bl.task {
                    1 => out.push_str("<input type=\"checkbox\" disabled> "),
                    2 => out.push_str("<input type=\"checkbox\" checked disabled> "),
                    _ => {}
                }
            }
            match bl.kind {
                MB_HEADING => {
                    let l = bl.level.clamp(1, 6);
                    out.push_str(&format!("<h{}>", l));
                    self.inline_html(k, s, e, &mut out);
                    out.push_str(&format!("</h{}>", l));
                }
                MB_CODE => {
                    if bl.lang.is_empty() {
                        out.push_str("<pre><code>");
                    } else {
                        out.push_str("<pre><code class=\"language-");
                        esc(&bl.lang, &mut out);
                        out.push_str("\">");
                    }
                    esc(&bl.text[clamp_char(&bl.text, s)..clamp_char(&bl.text, e)], &mut out);
                    out.push_str("</code></pre>");
                }
                MB_RULE => out.push_str("<hr>"),
                MB_ROW => {
                    if table.is_none() {
                        out.push_str("<table>");
                        table = Some(bl.table);
                    }
                    out.push_str("<tr>");
                    let aligns = self.tables.get(bl.table as usize).map(|t| t.aligns.as_slice()).unwrap_or(&[]);
                    for (ci, &(ca, cb)) in bl.cells.iter().enumerate() {
                        let (ca, cb) = (ca as usize, cb as usize);
                        if cb < s || ca > e {
                            continue;
                        }
                        let tag = if bl.head { "th" } else { "td" };
                        match aligns.get(ci).copied().unwrap_or(MA_NONE) {
                            MA_LEFT => out.push_str(&format!("<{} align=\"left\">", tag)),
                            MA_CENTER => out.push_str(&format!("<{} align=\"center\">", tag)),
                            MA_RIGHT => out.push_str(&format!("<{} align=\"right\">", tag)),
                            _ => out.push_str(&format!("<{}>", tag)),
                        }
                        self.inline_html(k, ca.max(s), cb.min(e), &mut out);
                        out.push_str(&format!("</{}>", tag));
                    }
                    out.push_str("</tr>");
                }
                _ => {
                    match bl.align {
                        1 => out.push_str("<p style=\"text-align:center\">"),
                        2 => out.push_str("<p style=\"text-align:right\">"),
                        _ => out.push_str("<p>"),
                    }
                    self.inline_html(k, s, e, &mut out);
                    out.push_str("</p>");
                }
            }
        }
        if table.is_some() {
            out.push_str("</table>");
        }
        while let Some((ord, li)) = lists.pop() {
            if li {
                out.push_str("</li>");
            }
            out.push_str(if ord { "</ol>" } else { "</ul>" });
        }
        while quotes > 0 {
            out.push_str("</blockquote>");
            quotes -= 1;
        }
        out
    }

    // the inline HTML of block k's bytes a .. b
    fn inline_html(&self, k: usize, a: usize, b: usize, out: &mut String) {
        let bl = &self.blocks[k];
        let t = &bl.text;
        let (a, b) = (clamp_char(t, a), clamp_char(t, b.max(a)));
        let mut i = a;
        let plain = |out: &mut String, s: &str| {
            for (n, part) in s.split('\n').enumerate() {
                if n > 0 {
                    out.push_str("<br>");
                }
                esc(part, out);
            }
        };
        for s in &bl.spans {
            let (sa, sb) = (s.a as usize, s.b as usize);
            if sb <= a || sa >= b {
                continue;
            }
            let (sa, sb) = (clamp_char(t, sa.max(a)), clamp_char(t, sb.min(b)));
            if sa > i {
                plain(out, &t[i..sa]);
            }
            if s.image != 0 {
                if let Some(m) = self.images.get(s.image as usize - 1) {
                    out.push_str("<img src=\"");
                    esc(&m.src, out);
                    out.push_str("\" alt=\"");
                    esc(&m.alt, out);
                    out.push_str("\">");
                }
                i = sb;
                continue;
            }
            if s.field != 0 {
                if let Some(f) = self.fields.get(s.field as usize - 1) {
                    esc(f.caption(), out);
                }
                i = sb;
                continue;
            }
            let mut close: Vec<&str> = Vec::new();
            if s.link != 0 {
                out.push_str("<a href=\"");
                esc(self.link(s.link), out);
                out.push_str("\">");
                close.push("</a>");
            }
            if s.color != 0 || s.bg != 0 {
                out.push_str("<span style=\"");
                if s.color != 0 {
                    out.push_str(&format!("color:#{:06x};", s.color & 0xFF_FFFF));
                }
                if s.bg != 0 {
                    out.push_str(&format!("background-color:#{:06x};", s.bg & 0xFF_FFFF));
                }
                out.push_str("\">");
                close.push("</span>");
            }
            for (bit, o, c) in [(MS_BOLD, "<strong>", "</strong>"), (MS_ITALIC, "<em>", "</em>"), (MS_STRIKE, "<del>", "</del>"), (MS_UNDER, "<u>", "</u>"), (MS_CODE, "<code>", "</code>")] {
                if s.style & bit != 0 {
                    out.push_str(o);
                    close.push(c);
                }
            }
            plain(out, &t[sa..sb]);
            for c in close.iter().rev() {
                out.push_str(c);
            }
            i = sb;
        }
        if i < b {
            plain(out, &t[i..b]);
        }
    }
}

fn clamp_char(t: &str, i: usize) -> usize {
    let mut i = i.min(t.len());
    while !t.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn esc(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\u{FFFC}' => {}
            _ => out.push(c),
        }
    }
}
