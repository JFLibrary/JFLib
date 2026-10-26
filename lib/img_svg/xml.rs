// xml -- a small, forgiving XML reader for SVG files: elements, attributes, character data
//
//   parse(bytes) -> Ok(Dom) / Err(ERR_*)   the element tree (the first element is the root)
//
// Handled: the UTF-8 byte order mark, <?..?>, comments, CDATA, a DOCTYPE with an internal
// subset (its `<!ENTITY name "value">` declarations are used: their values are not expanded
// again, so no entity can grow exponentially), the five predefined entities and character
// references.  Forgiving where browsers are strict (SVG files from editors are often sloppy):
// a mismatched end tag closes up to the matching open element (or is ignored), unquoted
// attribute values run to the next blank, an unterminated construct ends the document.
// Limits: MAX_ELS elements, MAX_DEPTH nesting (deeper elements are dropped with their
// content), MAX_ATTRS attributes per element, MAX_ENT entity declarations, MAX_EXPAND bytes
// produced by entity references in all.  Element names "svg:x" become "x"; other prefixes
// stay (inkscape:..., sodipodi:... are foreign elements that are not drawn).  Character data
// is kept only where it matters (text, tspan, style, title, desc, textPath).

use alloc::string::String;
use alloc::vec::Vec;

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
    s: &'a [u8],
    i: usize,
    ents: Vec<(String, String)>,
    expanded: usize,
    dom: Dom,
    stack: Vec<u32>,
    skip: usize, // > 0: inside elements dropped for depth / count (their nesting level)
}

fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n')
}

fn name_end(c: u8) -> bool {
    is_blank(c) || matches!(c, b'/' | b'>' | b'=' | b'<' | b'"' | b'\'')
}

fn find(s: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    if pat.is_empty() || from >= s.len() {
        return None;
    }
    let first = pat[0];
    let mut i = from;
    while i + pat.len() <= s.len() {
        if s[i] == first && &s[i..i + pat.len()] == pat {
            return Some(i);
        }
        i += 1;
    }
    None
}

// text of the kinds whose character data is kept
fn keeps_text(name: &str) -> bool {
    matches!(name, "text" | "tspan" | "style" | "title" | "desc" | "textPath")
}

fn push_utf8(out: &mut String, b: &[u8]) {
    match core::str::from_utf8(b) {
        Ok(s) => out.push_str(s),
        Err(_) => {
            // Latin-1 fallback for stray bytes (old files): each invalid byte as its code point
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
                        let bad = b[i + ok];
                        out.push(char::from(bad));
                        i += ok + 1;
                    }
                }
            }
        }
    }
}

impl P<'_> {
    // decode entity / character references of raw text into out
    fn decode(&mut self, raw: &[u8], out: &mut String) {
        let mut i = 0;
        let mut start = 0;
        while i < raw.len() {
            if raw[i] != b'&' {
                i += 1;
                continue;
            }
            // a bare '&' must not swallow a real reference after it ("AT&T &amp; Co")
            let Some(semi) = raw[i + 1..].iter().take(63).position(|&c| c == b';' || c == b'&').filter(|&k| raw[i + 1 + k] == b';').map(|k| k + 1) else {
                i += 1;
                continue;
            };
            push_utf8(out, &raw[start..i]);
            let name = &raw[i + 1..i + semi];
            let mut done = true;
            match name {
                b"lt" => out.push('<'),
                b"gt" => out.push('>'),
                b"amp" => out.push('&'),
                b"quot" => out.push('"'),
                b"apos" => out.push('\''),
                _ if name.first() == Some(&b'#') => {
                    let (digits, radix) = if name.get(1).map(|c| c | 32) == Some(b'x') { (&name[2..], 16) } else { (&name[1..], 10) };
                    let v = core::str::from_utf8(digits).ok().and_then(|d| u32::from_str_radix(d, radix).ok());
                    out.push(v.and_then(char::from_u32).filter(|&c| c != '\0').unwrap_or('\u{FFFD}'));
                }
                _ => {
                    let found = self.ents.iter().position(|(n, _)| n.as_bytes() == name);
                    match found {
                        Some(k) if self.expanded + self.ents[k].1.len() <= MAX_EXPAND => {
                            self.expanded += self.ents[k].1.len();
                            let v = self.ents[k].1.clone();
                            out.push_str(&v);
                        }
                        Some(_) => self.dom.truncated = true,
                        None => done = false,
                    }
                }
            }
            if done {
                i += semi + 1;
            } else {
                // an unknown entity stays as written
                push_utf8(out, &raw[i..i + semi + 1]);
                i += semi + 1;
            }
            start = i;
        }
        push_utf8(out, &raw[start..]);
    }

    fn text(&mut self, raw: &[u8], cdata: bool) {
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
            push_utf8(&mut s, raw);
        } else {
            self.decode(raw, &mut s);
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

    // <!DOCTYPE ... [ subset ] >: the entity declarations of the subset
    fn doctype(&mut self) {
        let s = self.s;
        let mut i = self.i;
        let mut depth = 0;
        while i < s.len() {
            match s[i] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                b'"' | b'\'' => {
                    let q = s[i];
                    i += 1;
                    while i < s.len() && s[i] != q {
                        i += 1;
                    }
                }
                b'<' if depth > 0 && s[i..].starts_with(b"<!ENTITY") => {
                    // the declaration ends at the first ">" outside quotes
                    let mut end = i + 8;
                    let mut q = 0u8;
                    while end < s.len() && (q != 0 || s[end] != b'>') {
                        if q == 0 && (s[end] == b'"' || s[end] == b'\'') {
                            q = s[end];
                        } else if s[end] == q {
                            q = 0;
                        }
                        end += 1;
                    }
                    self.entity(&s[i + 8..end]);
                    i = end;
                }
                b'<' if depth > 0 && s[i..].starts_with(b"<!--") => {
                    i = find(s, i + 4, b"-->").map_or(s.len(), |e| e + 2);
                }
                b'>' if depth <= 0 => {
                    self.i = i + 1;
                    return;
                }
                _ => {}
            }
            i += 1;
        }
        self.i = s.len();
    }

    // name "value" (general entities only; parameter entities and external ones are skipped)
    fn entity(&mut self, d: &[u8]) {
        let mut i = 0;
        while i < d.len() && is_blank(d[i]) {
            i += 1;
        }
        if d.get(i) == Some(&b'%') {
            return;
        }
        let n0 = i;
        while i < d.len() && !is_blank(d[i]) {
            i += 1;
        }
        let name = &d[n0..i];
        while i < d.len() && is_blank(d[i]) {
            i += 1;
        }
        let Some(&q) = d.get(i) else { return };
        if q != b'"' && q != b'\'' {
            return;
        }
        let v0 = i + 1;
        let Some(len) = d[v0..].iter().position(|&c| c == q) else { return };
        if name.is_empty() || self.ents.len() >= MAX_ENT || len > 65536 {
            return;
        }
        let mut val = String::new();
        // character references inside the value; other entities are not expanded again
        let saved = core::mem::take(&mut self.ents);
        self.decode(&d[v0..v0 + len], &mut val);
        self.ents = saved;
        let mut nm = String::new();
        push_utf8(&mut nm, name);
        if !self.ents.iter().any(|(n, _)| *n == nm) {
            self.ents.push((nm, val));
        }
    }

    fn end_tag(&mut self) {
        let s = self.s;
        let n0 = self.i + 2;
        let mut j = n0;
        while j < s.len() && !name_end(s[j]) {
            j += 1;
        }
        let name = &s[n0..j];
        self.i = find(s, j, b">").map_or(s.len(), |e| e + 1);
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        let local = local_name(name);
        let pos = self.stack.iter().rposition(|&k| self.dom.els.get(k as usize).is_some_and(|e| e.name.as_bytes() == local));
        if let Some(p) = pos {
            self.stack.truncate(p);
        }
    }

    fn start_tag(&mut self) {
        let s = self.s;
        let n0 = self.i + 1;
        let mut j = n0;
        while j < s.len() && !name_end(s[j]) {
            j += 1;
        }
        let name = &s[n0..j];
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut closed = false;
        let mut i = j;
        // attributes
        loop {
            while i < s.len() && is_blank(s[i]) {
                i += 1;
            }
            if i >= s.len() {
                self.dom.truncated = true;
                break;
            }
            match s[i] {
                b'>' => {
                    i += 1;
                    break;
                }
                b'/' => {
                    if s.get(i + 1) == Some(&b'>') {
                        closed = true;
                        i += 2;
                        break;
                    }
                    i += 1;
                    continue;
                }
                b'<' => break, // a broken tag: the next one starts here
                _ => {}
            }
            let a0 = i;
            while i < s.len() && !name_end(s[i]) {
                i += 1;
            }
            if i == a0 {
                i += 1; // a stray quote or '='
                continue;
            }
            let an = &s[a0..i];
            while i < s.len() && is_blank(s[i]) {
                i += 1;
            }
            let mut val = String::new();
            if s.get(i) == Some(&b'=') {
                i += 1;
                while i < s.len() && is_blank(s[i]) {
                    i += 1;
                }
                let raw = match s.get(i) {
                    Some(&q) if q == b'"' || q == b'\'' => {
                        let v0 = i + 1;
                        let end = s[v0..].iter().position(|&c| c == q).map_or(s.len(), |k| v0 + k);
                        i = (end + 1).min(s.len());
                        &s[v0..end]
                    }
                    _ => {
                        let v0 = i;
                        while i < s.len() && !is_blank(s[i]) && s[i] != b'>' {
                            i += 1;
                        }
                        &s[v0..i]
                    }
                };
                if self.skip == 0 {
                    self.decode(raw, &mut val);
                }
            }
            if self.skip == 0 && attrs.len() < MAX_ATTRS {
                let mut n = String::new();
                push_utf8(&mut n, an);
                if !attrs.iter().any(|(x, _)| *x == n) {
                    attrs.push((n, val));
                }
            }
        }
        self.i = i;
        if self.skip > 0 {
            if !closed {
                self.skip += 1;
            }
            return;
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
        let mut nm = String::new();
        push_utf8(&mut nm, local_name(name));
        self.dom.els.push(El { name: nm, attrs, kids: Vec::new(), parent });
        if let Some(p) = self.dom.els.get_mut(parent as usize) {
            p.kids.push(Item::El(k));
        }
        if !closed {
            self.stack.push(k);
        }
    }

    fn run(&mut self) {
        let s = self.s;
        while self.i < s.len() {
            let i = self.i;
            if s[i] != b'<' {
                let end = s[i..].iter().position(|&c| c == b'<').map_or(s.len(), |k| i + k);
                self.text(&s[i..end], false);
                self.i = end;
                continue;
            }
            let rest = &s[i..];
            if rest.starts_with(b"<?") {
                self.i = find(s, i + 2, b"?>").map_or(s.len(), |e| e + 2);
            } else if rest.starts_with(b"<!--") {
                self.i = find(s, i + 4, b"-->").map_or(s.len(), |e| e + 3);
            } else if rest.starts_with(b"<![CDATA[") {
                let end = find(s, i + 9, b"]]>").unwrap_or(s.len());
                self.text(&s[i + 9..end], true);
                self.i = (end + 3).min(s.len());
            } else if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case(b"<!DOCTYPE") {
                self.i = i + 9;
                self.doctype();
            } else if rest.starts_with(b"<!") {
                self.i = find(s, i + 2, b">").map_or(s.len(), |e| e + 1);
            } else if rest.starts_with(b"</") {
                self.end_tag();
            } else if rest.len() > 1 && !name_end(rest[1]) {
                self.start_tag();
            } else {
                self.text(&s[i..i + 1], false); // a lone '<'
                self.i = i + 1;
            }
        }
        if !self.stack.is_empty() {
            self.dom.truncated = true;
        }
    }
}

// "svg:rect" -> "rect"; other prefixes stay
fn local_name(name: &[u8]) -> &[u8] {
    if name.len() > 4 && name[..4].eq_ignore_ascii_case(b"svg:") { &name[4..] } else { name }
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
    let mut p = P { s, i: 0, ents: Vec::new(), expanded: 0, dom: Dom::default(), stack: Vec::new(), skip: 0 };
    p.run();
    if p.dom.els.is_empty() {
        return Err(ERR_HDR);
    }
    Ok(p.dom)
}
