// lenient -- the forgiving reader of formats::xml: what a browser or an editor would still show
//
//   Lenient::new(text) / with_limits(text, attrs, entities, expand)
//   l.next() -> Token          Start { name, attrs (raw values), empty }, End { name }, Text (raw), CData, Eof
//   l.decode(raw, &mut out)    a raw text or attribute value with its references replaced
//   l.truncated()              the input ended inside a tag, or the entities' budget was spent
//
// For files from editors, which are often sloppy (SVG, fontconfig, theme parts): nothing is an error.
// Tokens: a start tag (its name as written, at most `attrs` attributes: a repeated one counts, the first
// stays; a value in ' or ", unquoted to the next blank or '>', none for a bare name; a stray quote, '=' or
// '/' is skipped; '<' ends a broken tag), an end tag (its name; the rest up to '>' is skipped), text up to
// the next '<' (a lone '<' that starts no tag is text), CDATA as written.  Skipped: the XML declaration and
// processing instructions, comments, every `<!...>` declaration; a DOCTYPE's internal subset gives the
// entities: its `<!ENTITY name "value">` declarations (general ones; parameter and external entities are
// skipped; the first declaration of a name stays; at most `entities`, values of at most 64 KiB, with
// character references and the five predefined entities replaced, other references kept as written so no
// entity grows exponentially).  An unterminated construct ends the document there.
//
// References (decode): the five predefined entities, character references (decimal, x or X; a number that
// is no character, or 0, gives U+FFFD), the DOCTYPE's entities while `expand` bytes of their values have
// not been used up (beyond: the reference is dropped and truncated() set); an unknown entity stays as
// written, and so does an '&' that has no ';' within 63 bytes before the next '&'.  Nesting, a second
// root, mismatched end tags and limits on elements are the caller's: it builds the tree it needs.

use alloc::string::String;
use alloc::vec::Vec;

/// the default limit of attributes per start tag
pub const ATTRS: usize = 512;
/// the default limit of entity declarations in a DOCTYPE
pub const ENTITIES: usize = 256;
/// the default budget of bytes that entity references may produce in all
pub const EXPAND: usize = 32 << 20;

/// what the lenient reader found next
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Token<'a> {
    /// a start tag: its name as written and its attributes with their raw values ("" for none)
    Start { name: &'a str, attrs: Vec<(&'a str, &'a str)>, empty: bool },
    /// an end tag
    End { name: &'a str },
    /// character data as written (decode replaces the references)
    Text(&'a str),
    /// a CDATA section's content as written
    CData(&'a str),
    /// the end of the text
    Eof,
}

/// the forgiving reader (see the top of this file)
pub struct Lenient<'a> {
    s: &'a str,
    i: usize,
    ents: Vec<(String, String)>,
    expanded: usize,
    truncated: bool,
    max_attrs: usize,
    max_ents: usize,
    max_expand: usize,
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

// a repeated attribute (not well-formed): the first one stays.  Sorted once per element; a search per
// attribute would cost up to `attrs` steps for each attribute of the file
fn dedup_first(attrs: &mut Vec<(&str, &str)>) {
    if attrs.len() <= 8 {
        let mut k = 1;
        while k < attrs.len() {
            if attrs[..k].iter().any(|(x, _)| *x == attrs[k].0) {
                attrs.remove(k);
            } else {
                k += 1;
            }
        }
        return;
    }
    let mut ix: Vec<u32> = (0..attrs.len() as u32).collect();
    ix.sort_unstable_by(|&a, &b| attrs[a as usize].0.cmp(attrs[b as usize].0).then(a.cmp(&b)));
    let mut dup = alloc::vec![false; attrs.len()];
    for w in ix.windows(2) {
        if attrs[w[0] as usize].0 == attrs[w[1] as usize].0 {
            dup[w[1] as usize] = true;
        }
    }
    let mut k = 0;
    attrs.retain(|_| {
        k += 1;
        !dup[k - 1]
    });
}

impl<'a> Lenient<'a> {
    /// a reader of `s` with the default limits (ATTRS, ENTITIES, EXPAND)
    pub fn new(s: &'a str) -> Self {
        Self::with_limits(s, ATTRS, ENTITIES, EXPAND)
    }

    /// a reader with at most `attrs` attributes per start tag, `entities` entity declarations and `expand`
    /// bytes of entity values in all
    pub fn with_limits(s: &'a str, attrs: usize, entities: usize, expand: usize) -> Self {
        Lenient { s, i: 0, ents: Vec::new(), expanded: 0, truncated: false, max_attrs: attrs, max_ents: entities, max_expand: expand }
    }

    /// whether the input ended inside a tag or the entities' budget was spent
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// the offset the reader is at
    pub fn pos(&self) -> usize {
        self.i
    }

    /// a raw text or attribute value (from a token) with its references replaced, appended to `out`
    pub fn decode(&mut self, raw: &str, out: &mut String) {
        let b = raw.as_bytes();
        let mut i = 0;
        let mut start = 0;
        while i < b.len() {
            if b[i] != b'&' {
                i += 1;
                continue;
            }
            // a bare '&' must not swallow a real reference after it ("AT&T &amp; Co")
            let Some(semi) = b[i + 1..].iter().take(63).position(|&c| c == b';' || c == b'&').filter(|&k| b[i + 1 + k] == b';').map(|k| k + 1) else {
                i += 1;
                continue;
            };
            out.push_str(&raw[start..i]);
            let name = &raw[i + 1..i + semi];
            let mut done = true;
            match name {
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "amp" => out.push('&'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                _ if name.starts_with('#') => {
                    let nb = name.as_bytes();
                    let (digits, radix) = if nb.get(1).map(|c| c | 32) == Some(b'x') { (&name[2..], 16) } else { (&name[1..], 10) };
                    let v = u32::from_str_radix(digits, radix).ok();
                    out.push(v.and_then(char::from_u32).filter(|&c| c != '\0').unwrap_or('\u{FFFD}'));
                }
                _ => {
                    // sorted by name: no walk over all of them per reference
                    match self.ents.binary_search_by(|(n, _)| n.as_str().cmp(name)).ok() {
                        Some(k) if self.expanded + self.ents[k].1.len() <= self.max_expand => {
                            self.expanded += self.ents[k].1.len();
                            out.push_str(&self.ents[k].1);
                        }
                        Some(_) => self.truncated = true,
                        None => done = false,
                    }
                }
            }
            if !done {
                // an unknown entity stays as written
                out.push_str(&raw[i..i + semi + 1]);
            }
            i += semi + 1;
            start = i;
        }
        out.push_str(&raw[start..]);
    }

    // <!DOCTYPE ... [ subset ] >: the entity declarations of the subset
    fn doctype(&mut self) {
        let src: &'a str = self.s;
        let s = src.as_bytes();
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
                    self.entity(&src[i + 8..end]);
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
    fn entity(&mut self, decl: &str) {
        let d = decl.as_bytes();
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
        let name = &decl[n0..i];
        while i < d.len() && is_blank(d[i]) {
            i += 1;
        }
        let Some(&q) = d.get(i) else { return };
        if q != b'"' && q != b'\'' {
            return;
        }
        let v0 = i + 1;
        let Some(len) = d[v0..].iter().position(|&c| c == q) else { return };
        if name.is_empty() || self.ents.len() >= self.max_ents || len > 65536 {
            return;
        }
        let mut val = String::new();
        // character references inside the value; other entities are not expanded again
        let saved = core::mem::take(&mut self.ents);
        self.decode(&decl[v0..v0 + len], &mut val);
        self.ents = saved;
        // sorted by name; the first declaration of a name stays
        if let Err(at) = self.ents.binary_search_by(|(n, _)| n.as_str().cmp(name)) {
            self.ents.insert(at, (String::from(name), val));
        }
    }

    fn end_tag(&mut self) -> Token<'a> {
        let src: &'a str = self.s;
        let s = src.as_bytes();
        let n0 = self.i + 2;
        let mut j = n0;
        while j < s.len() && !name_end(s[j]) {
            j += 1;
        }
        let name = &src[n0..j];
        self.i = find(s, j, b">").map_or(s.len(), |e| e + 1);
        Token::End { name }
    }

    fn start_tag(&mut self) -> Token<'a> {
        let src: &'a str = self.s;
        let s = src.as_bytes();
        let n0 = self.i + 1;
        let mut j = n0;
        while j < s.len() && !name_end(s[j]) {
            j += 1;
        }
        let name = &src[n0..j];
        let mut attrs: Vec<(&'a str, &'a str)> = Vec::new();
        let mut empty = false;
        let mut i = j;
        loop {
            while i < s.len() && is_blank(s[i]) {
                i += 1;
            }
            if i >= s.len() {
                self.truncated = true;
                break;
            }
            match s[i] {
                b'>' => {
                    i += 1;
                    break;
                }
                b'/' => {
                    if s.get(i + 1) == Some(&b'>') {
                        empty = true;
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
            let an = &src[a0..i];
            while i < s.len() && is_blank(s[i]) {
                i += 1;
            }
            let mut val = "";
            if s.get(i) == Some(&b'=') {
                i += 1;
                while i < s.len() && is_blank(s[i]) {
                    i += 1;
                }
                val = match s.get(i) {
                    Some(&q) if q == b'"' || q == b'\'' => {
                        let v0 = i + 1;
                        let end = s[v0..].iter().position(|&c| c == q).map_or(s.len(), |k| v0 + k);
                        i = (end + 1).min(s.len());
                        &src[v0..end]
                    }
                    _ => {
                        let v0 = i;
                        while i < s.len() && !is_blank(s[i]) && s[i] != b'>' {
                            i += 1;
                        }
                        &src[v0..i]
                    }
                };
            }
            if attrs.len() < self.max_attrs {
                attrs.push((an, val));
            }
        }
        dedup_first(&mut attrs);
        self.i = i;
        Token::Start { name, attrs, empty }
    }

    /// the next token (Eof at the end, and from then on)
    pub fn next(&mut self) -> Token<'a> {
        let src: &'a str = self.s;
        let s = src.as_bytes();
        while self.i < s.len() {
            let i = self.i;
            if s[i] != b'<' {
                let end = s[i..].iter().position(|&c| c == b'<').map_or(s.len(), |k| i + k);
                self.i = end;
                return Token::Text(&src[i..end]);
            }
            let rest = &s[i..];
            if rest.starts_with(b"<?") {
                self.i = find(s, i + 2, b"?>").map_or(s.len(), |e| e + 2);
            } else if rest.starts_with(b"<!--") {
                self.i = find(s, i + 4, b"-->").map_or(s.len(), |e| e + 3);
            } else if rest.starts_with(b"<![CDATA[") {
                let end = find(s, i + 9, b"]]>").unwrap_or(s.len());
                self.i = (end + 3).min(s.len());
                return Token::CData(&src[i + 9..end]);
            } else if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case(b"<!DOCTYPE") {
                self.i = i + 9;
                self.doctype();
            } else if rest.starts_with(b"<!") {
                self.i = find(s, i + 2, b">").map_or(s.len(), |e| e + 1);
            } else if rest.starts_with(b"</") {
                return self.end_tag();
            } else if rest.len() > 1 && !name_end(rest[1]) {
                return self.start_tag();
            } else {
                self.i = i + 1;
                return Token::Text(&src[i..i + 1]); // a lone '<'
            }
        }
        Token::Eof
    }
}
