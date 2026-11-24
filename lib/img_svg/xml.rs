// xml -- the element tree of an SVG file, from formats::xml's forgiving reader
//
//   parse(bytes) -> Ok(Dom) / Err(ERR_*)   the element tree (the first element is the root)
//
// The reading is formats::xml::Lenient's (SVG files from editors are often sloppy): the declaration,
// processing instructions, comments and declarations skipped; CDATA; a DOCTYPE's internal subset and
// its `<!ENTITY name "value">` declarations (values not expanded again, so no entity can grow
// exponentially; MAX_ENT declarations, MAX_EXPAND bytes produced by entity references in all); the
// five predefined entities and character references; unquoted attribute values to the next blank, an
// unterminated construct ends the document.  Bytes that are no UTF-8 count as Latin-1 (old files).
// The tree is this file's: a mismatched end tag closes up to the matching open element (or is
// ignored), deeper than MAX_DEPTH elements are dropped with their content, so is everything beyond
// MAX_ELS elements and a second root; MAX_ATTRS attributes per element (a repeated one counts; the
// first stays).  Element names "svg:x" become "x"; other prefixes stay (inkscape:..., sodipodi:...
// are foreign elements that are not drawn).  Character data is kept only where it matters (text,
// tspan, style, title, desc, textPath).

use alloc::string::String;
use alloc::vec::Vec;

use formats::xml::{Lenient, Token};
use img_core::{ERR_HDR, ERR_LIMIT, ERR_UNSUPPORTED};

pub(crate) const MAX_ELS: usize = 250_000;
pub(crate) const MAX_DEPTH: usize = 256;
const MAX_ATTRS: usize = 512;
const MAX_ENT: usize = 256;
const MAX_EXPAND: usize = 32 << 20;
pub(crate) const NONE: u32 = u32::MAX;

/// a child of an element: another element or character data
#[derive(Clone, Debug)]
pub(crate) enum Item {
    El(u32),
    Text(String),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub kids: Vec<Item>,
    pub parent: u32,
}

impl El {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

#[derive(Default)]
pub(crate) struct Dom {
    pub els: Vec<El>,
    /// the input ended early or a limit dropped content
    pub truncated: bool,
}

struct P<'a> {
    lex: Lenient<'a>,
    dom: Dom,
    stack: Vec<u32>,
    skip: usize, // > 0: inside elements dropped for depth / count (their nesting level)
}

// text of the kinds whose character data is kept
fn keeps_text(name: &str) -> bool {
    matches!(name, "text" | "tspan" | "style" | "title" | "desc" | "textPath")
}

// the bytes as text: UTF-8, each byte that is no part of it as its Latin-1 code point (old files)
fn latin1_fallback(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match core::str::from_utf8(&b[i..]) {
            Ok(s) => {
                out.push_str(s);
                break;
            }
            Err(e) => {
                let ok = e.valid_up_to();
                if let Ok(s) = core::str::from_utf8(&b[i..i + ok]) {
                    out.push_str(s);
                }
                out.push(char::from(b[i + ok]));
                i += ok + 1;
            }
        }
    }
    out
}

impl P<'_> {
    fn text(&mut self, raw: &str, cdata: bool) {
        if self.skip > 0 {
            return;
        }
        let Some(&cur) = self.stack.last() else { return };
        let keep = self.dom.els.get(cur as usize).is_some_and(|e| keeps_text(&e.name));
        if !keep {
            return;
        }
        let mut s = String::new();
        if cdata {
            s.push_str(raw);
        } else {
            self.lex.decode(raw, &mut s);
        }
        if s.is_empty() {
            return;
        }
        if let Some(e) = self.dom.els.get_mut(cur as usize) {
            if let Some(Item::Text(t)) = e.kids.last_mut() {
                t.push_str(&s);
            } else {
                e.kids.push(Item::Text(s));
            }
        }
    }

    fn end_tag(&mut self, name: &str) {
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        let local = local_name(name);
        let pos = self.stack.iter().rposition(|&k| self.dom.els.get(k as usize).is_some_and(|e| e.name == local));
        if let Some(p) = pos {
            self.stack.truncate(p);
        }
    }

    fn start_tag(&mut self, name: &str, raw: &[(&str, &str)], closed: bool) {
        if self.skip > 0 {
            if !closed {
                self.skip += 1;
            }
            return;
        }
        // the values of an element that is kept (or dropped for a limit) count against the budget
        let mut attrs = Vec::with_capacity(raw.len());
        for (n, v) in raw {
            let mut val = String::new();
            self.lex.decode(v, &mut val);
            attrs.push((String::from(*n), val));
        }
        if self.stack.len() >= MAX_DEPTH || self.dom.els.len() >= MAX_ELS {
            self.dom.truncated = true;
            if !closed {
                self.skip = 1;
            }
            return;
        }
        // a second root element: dropped
        if self.stack.is_empty() && !self.dom.els.is_empty() {
            if !closed {
                self.skip = 1;
            }
            return;
        }
        let k = self.dom.els.len() as u32;
        let parent = self.stack.last().copied().unwrap_or(NONE);
        self.dom.els.push(El { name: String::from(local_name(name)), attrs, kids: Vec::new(), parent });
        if let Some(p) = self.dom.els.get_mut(parent as usize) {
            p.kids.push(Item::El(k));
        }
        if !closed {
            self.stack.push(k);
        }
    }

    fn run(&mut self) {
        loop {
            match self.lex.next() {
                Token::Text(t) => self.text(t, false),
                Token::CData(t) => self.text(t, true),
                Token::Start { name, attrs, empty } => self.start_tag(name, &attrs, empty),
                Token::End { name } => self.end_tag(name),
                Token::Eof => break,
                _ => {}
            }
        }
        if !self.stack.is_empty() || self.lex.truncated() {
            self.dom.truncated = true;
        }
    }
}

// "svg:rect" -> "rect"; other prefixes stay
fn local_name(name: &str) -> &str {
    if name.len() > 4 && name.as_bytes()[..4].eq_ignore_ascii_case(b"svg:") { &name[4..] } else { name }
}

/// the element tree of an SVG document: Err(ERR_HDR) without any element,
/// ERR_UNSUPPORTED for UTF-16 input, ERR_LIMIT for input beyond MAX_INPUT
pub(crate) fn parse(data: &[u8]) -> Result<Dom, i32> {
    if data.len() > crate::MAX_INPUT {
        return Err(ERR_LIMIT);
    }
    let mut s = data;
    if s.starts_with(&[0xEF, 0xBB, 0xBF]) {
        s = &s[3..];
    }
    if s.starts_with(&[0xFF, 0xFE]) || s.starts_with(&[0xFE, 0xFF]) {
        return Err(ERR_UNSUPPORTED);
    }
    let text = latin1_fallback(s);
    let mut p = P { lex: Lenient::with_limits(&text, MAX_ATTRS, MAX_ENT, MAX_EXPAND), dom: Dom::default(), stack: Vec::new(), skip: 0 };
    p.run();
    if p.dom.els.is_empty() {
        return Err(ERR_HDR);
    }
    Ok(p.dom)
}
