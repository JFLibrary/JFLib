// xml -- a bounded pull reader for the XML parts of a workbook (after lib/diagram_io/src/xml.rs)
//
//   Reader::new(text, depth)           a part's text (text_of decodes UTF-8 or UTF-16)
//   r.next() -> Event                  Start, End, Text, Eof; an empty element <a/> gives Start
//                                      and then End
//   r.skip()                           the rest of the element just started, unread
//   r.text(limit)                      the text of the element just started
//
// Why not base::xml: it builds a tree of every element, and a worksheet of a million cells would
// be a tree of gigabytes; it also refuses comments, CDATA and processing instructions, which
// writers other than Excel put into their parts.  This reader hands out one event at a time and
// keeps only the names of the open elements.
//
// What it reads: elements, attributes in ' or ", the five predefined entities and character
// references, CDATA, comments and processing instructions (both skipped), namespaces (each name
// is resolved to the few namespaces the reader knows, Ns).  Refused: a DOCTYPE (no entity
// declarations, nothing external, so no entity expansion at all), an undefined entity, a
// character XML does not allow, an encoding other than UTF-8 / UTF-16 in the declaration,
// mismatched or unclosed tags, an unbound prefix, a duplicate attribute, more nesting than
// `depth`, more than MAX_ATTRS attributes on one element.  Line ends are normalised as XML says:
// CR LF and CR become LF in text; in attribute values a literal tab, line end or CR becomes a
// space.  Nothing recurses: the open elements are a Vec bounded by `depth`.

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

/// SpreadsheetML, transitional
pub(crate) const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// SpreadsheetML, strict
pub(crate) const NS_MAIN_STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
/// the namespace of r:id, transitional
pub(crate) const NS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// the namespace of r:id, strict
pub(crate) const NS_REL_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const NS_XML: &str = "http://www.w3.org/XML/1998/namespace";
const NS_XMLNS: &str = "http://www.w3.org/2000/xmlns/";
/// the most namespace declarations in scope at a time
const MAX_BINDINGS: usize = 64;
/// the most attributes on one element
const MAX_ATTRS: usize = 128;

/// what is wrong with a part's XML
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum XmlError {
    /// not well-formed
    Syntax,
    /// deeper nesting than the limit
    Depth,
    /// more attributes on one element than MAX_ATTRS, or too many namespace declarations
    Attributes,
    /// a DOCTYPE, or an encoding other than UTF-8 and UTF-16
    Unsupported,
}

impl XmlError {
    pub(crate) fn text(self) -> &'static str {
        match self {
            XmlError::Syntax => "not well-formed",
            XmlError::Depth => "nested deeper than the limit",
            XmlError::Attributes => "too many attributes or namespace declarations on an element",
            XmlError::Unsupported => "a DOCTYPE or an encoding other than UTF-8 and UTF-16",
        }
    }
}

/// the namespace of a name, as far as the reader needs to know it
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum Ns {
    /// no namespace (an unprefixed attribute, an element without default namespace)
    None,
    /// SpreadsheetML (transitional or strict)
    Main,
    /// the relationships namespace of r:id (transitional or strict)
    Rel,
    /// the xml prefix (xml:space)
    Xml,
    /// a namespace declaration itself (xmlns, xmlns:p)
    Decl,
    /// any other namespace
    Other,
}

fn ns_of(uri: &str) -> Ns {
    match uri {
        "" => Ns::None,
        NS_MAIN | NS_MAIN_STRICT => Ns::Main,
        NS_REL | NS_REL_STRICT => Ns::Rel,
        NS_XML => Ns::Xml,
        _ => Ns::Other,
    }
}

/// one attribute
pub(crate) struct Attr<'a> {
    qname: &'a str,
    /// the name without prefix
    pub local: &'a str,
    pub ns: Ns,
    /// the value with entities and character references replaced
    pub value: Cow<'a, str>,
}

/// a start tag
pub(crate) struct Start<'a> {
    pub local: &'a str,
    pub ns: Ns,
    pub attrs: Vec<Attr<'a>>,
}

impl Start<'_> {
    /// whether this is the SpreadsheetML element `local`
    pub fn is(&self, local: &str) -> bool {
        self.ns == Ns::Main && self.local == local
    }
    /// an attribute without namespace
    pub fn attr(&self, local: &str) -> Option<&str> {
        self.attr_ns(Ns::None, local)
    }
    /// an attribute in a namespace
    pub fn attr_ns(&self, ns: Ns, local: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.ns == ns && a.local == local).map(|a| a.value.as_ref())
    }
    /// xml:space of this element: Some(true) for "preserve", Some(false) for "default"
    pub fn space(&self) -> Option<bool> {
        match self.attr_ns(Ns::Xml, "space") {
            Some("preserve") => Some(true),
            Some("default") => Some(false),
            _ => None,
        }
    }
}

/// what the reader found next
pub(crate) enum Event<'a> {
    Start(Start<'a>),
    End,
    /// character data, CDATA included
    Text(Cow<'a, str>),
    Eof,
}

/// the error and the byte offset it was found at
pub(crate) type XmlResult<T> = Result<T, (XmlError, usize)>;

/// a part's bytes as text: UTF-8 with or without BOM (borrowed), UTF-16 with BOM (decoded);
/// None for anything else (UTF-32, invalid sequences)
pub(crate) fn text_of(b: &[u8]) -> Option<Cow<'_, str>> {
    if let Some(rest) = b.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return core::str::from_utf8(rest).ok().map(Cow::Borrowed);
    }
    let le = b.starts_with(&[0xff, 0xfe]);
    if le || b.starts_with(&[0xfe, 0xff]) {
        if b.starts_with(&[0xff, 0xfe, 0, 0]) {
            return None;
        }
        let units = &b[2..];
        if units.len() % 2 != 0 {
            return None;
        }
        let it = units.chunks_exact(2).map(|p| if le { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) });
        let mut s = String::with_capacity(units.len() / 2);
        for c in char::decode_utf16(it) {
            s.push(c.ok()?);
        }
        return Some(Cow::Owned(s));
    }
    if b.starts_with(&[0, 0, 0xfe, 0xff]) {
        return None;
    }
    core::str::from_utf8(b).ok().map(Cow::Borrowed)
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

/// whether a text holds nothing but XML white space
pub(crate) fn is_blank(s: &str) -> bool {
    s.bytes().all(is_space)
}

/// a text without the XML white space at either end
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\n', '\r'])
}

fn name_end(c: u8) -> bool {
    is_space(c) || matches!(c, b'/' | b'>' | b'=' | b'<' | b'"' | b'\'' | b'&' | b'!' | b'?')
}

// characters XML 1.0 does not allow (the bytes below 0x20 but tab, LF, CR; U+FFFE, U+FFFF)
fn bad_chars(s: &str) -> bool {
    s.bytes().any(|c| c < 0x20 && !matches!(c, b'\t' | b'\n' | b'\r')) || s.contains(['\u{fffe}', '\u{ffff}'])
}

fn char_ok(c: u32) -> bool {
    matches!(c, 9 | 10 | 13 | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)
}

// replace entities and character references; `attr`: literal white space becomes a space
fn unescape(raw: &str, attr: bool) -> Option<Cow<'_, str>> {
    if bad_chars(raw) {
        return None;
    }
    let plain = !raw.contains('&') && !raw.contains('\r') && !(attr && raw.contains(['\t', '\n']));
    if plain {
        return Some(Cow::Borrowed(raw));
    }
    let mut out = String::with_capacity(raw.len());
    let b = raw.as_bytes();
    let mut i = 0;
    let mut run = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'&' {
            out.push_str(&raw[run..i]);
            let semi = raw[i..].find(';')? + i;
            let name = &raw[i + 1..semi];
            match name {
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "amp" => out.push('&'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                _ => {
                    // leading zeros are allowed, the rest must fit a code point
                    let v = if let Some(h) = name.strip_prefix("#x") {
                        let t = h.trim_start_matches('0');
                        if h.is_empty() || t.len() > 8 || !h.bytes().all(|d| d.is_ascii_hexdigit()) {
                            return None;
                        }
                        if t.is_empty() { 0 } else { u32::from_str_radix(t, 16).ok()? }
                    } else if let Some(d) = name.strip_prefix('#') {
                        let t = d.trim_start_matches('0');
                        if d.is_empty() || t.len() > 10 || !d.bytes().all(|x| x.is_ascii_digit()) {
                            return None;
                        }
                        if t.is_empty() { 0 } else { t.parse::<u32>().ok()? }
                    } else {
                        return None;
                    };
                    if !char_ok(v) {
                        return None;
                    }
                    out.push(char::from_u32(v)?);
                }
            }
            i = semi + 1;
            run = i;
        } else if c == b'\r' {
            out.push_str(&raw[run..i]);
            out.push(if attr { ' ' } else { '\n' });
            i += if b.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
            run = i;
        } else if attr && (c == b'\t' || c == b'\n') {
            out.push_str(&raw[run..i]);
            out.push(' ');
            i += 1;
            run = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&raw[run..]);
    Some(Cow::Owned(out))
}

// a binding of a prefix ("" = the default namespace) made by the element at `depth`
struct Binding<'a> {
    prefix: &'a str,
    ns: Ns,
    depth: usize,
}

/// the pull reader of one part
pub(crate) struct Reader<'a> {
    s: &'a str,
    pos: usize,
    open: Vec<&'a str>,
    bindings: Vec<Binding<'a>>,
    max_depth: usize,
    // an empty element was returned: its End comes next
    pending_end: bool,
    root_seen: bool,
}

impl<'a> Reader<'a> {
    /// a reader of `s` with at most `depth` open elements
    pub fn new(s: &'a str, depth: usize) -> Self {
        Reader { s, pos: 0, open: Vec::new(), bindings: Vec::new(), max_depth: depth.max(1), pending_end: false, root_seen: false }
    }

    fn err<T>(&self, e: XmlError) -> XmlResult<T> {
        Err((e, self.pos))
    }

    fn rest(&self) -> &'a str {
        &self.s[self.pos..]
    }

    fn skip_space(&mut self) {
        let b = self.s.as_bytes();
        while self.pos < b.len() && is_space(b[self.pos]) {
            self.pos += 1;
        }
    }

    fn name(&mut self) -> XmlResult<&'a str> {
        let b = self.s.as_bytes();
        let start = self.pos;
        while self.pos < b.len() && !name_end(b[self.pos]) {
            self.pos += 1;
        }
        let n = &self.s[start..self.pos];
        let name_char = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':') || c >= 0x80;
        if !n.bytes().all(name_char) || n.contains(['\u{fffe}', '\u{ffff}']) {
            return self.err(XmlError::Syntax);
        }
        let first = n.as_bytes().first().copied();
        if n.is_empty() || matches!(first, Some(b'0'..=b'9' | b'-' | b'.' | b':')) || n.ends_with(':') || n.matches(':').count() > 1 {
            return self.err(XmlError::Syntax);
        }
        Ok(n)
    }

    // the namespace bound to a prefix ("" = default)
    fn lookup(&self, prefix: &str) -> Option<Ns> {
        if prefix == "xml" {
            return Some(Ns::Xml);
        }
        match self.bindings.iter().rev().find(|b| b.prefix == prefix) {
            Some(b) => Some(b.ns),
            None if prefix.is_empty() => Some(Ns::None),
            None => None,
        }
    }

    // the XML declaration: version, encoding (UTF-8 or UTF-16 only), standalone
    fn declaration(&mut self) -> XmlResult<()> {
        let Some(end) = self.rest().find("?>") else { return self.err(XmlError::Syntax) };
        let body = &self.s[self.pos + 5..self.pos + end];
        if let Some(i) = body.find("encoding") {
            let v = body[i + 8..].trim_start().strip_prefix('=').map(str::trim_start).unwrap_or("");
            let q = v.as_bytes().first().copied();
            let val = match q {
                Some(c @ (b'"' | b'\'')) => v[1..].split(c as char).next().unwrap_or(""),
                _ => return self.err(XmlError::Syntax),
            };
            if !val.eq_ignore_ascii_case("utf-8") && !val.eq_ignore_ascii_case("utf-16") {
                return self.err(XmlError::Unsupported);
            }
        }
        self.pos += end + 2;
        Ok(())
    }

    /// the next event
    pub fn next(&mut self) -> XmlResult<Event<'a>> {
        if self.pending_end {
            self.pending_end = false;
            self.close();
            return Ok(Event::End);
        }
        loop {
            if self.pos >= self.s.len() {
                if !self.open.is_empty() || !self.root_seen {
                    return self.err(XmlError::Syntax);
                }
                return Ok(Event::Eof);
            }
            let rest = self.rest();
            if !rest.starts_with('<') {
                let start = self.pos;
                let n = rest.find('<').unwrap_or(rest.len());
                self.pos += n;
                let raw = &self.s[start..self.pos];
                if self.open.is_empty() {
                    if !is_blank(raw) {
                        self.pos = start;
                        return self.err(XmlError::Syntax);
                    }
                    continue;
                }
                if raw.contains("]]>") {
                    self.pos = start;
                    return self.err(XmlError::Syntax);
                }
                return match unescape(raw, false) {
                    Some(t) => Ok(Event::Text(t)),
                    None => {
                        self.pos = start;
                        self.err(XmlError::Syntax)
                    }
                };
            }
            if rest.starts_with("<?") {
                if rest.starts_with("<?xml") && rest.as_bytes().get(5).is_some_and(|&c| is_space(c)) {
                    if self.pos != 0 {
                        return self.err(XmlError::Syntax);
                    }
                    self.declaration()?;
                    continue;
                }
                // a processing instruction: a target that is a name and not "xml" in any case
                let Some(e) = rest.find("?>") else { return self.err(XmlError::Syntax) };
                let body = &rest[2..e];
                let target = body.split(|c: char| c.is_ascii_whitespace()).next().unwrap_or("");
                if target.is_empty() || target.eq_ignore_ascii_case("xml") || bad_chars(body) {
                    return self.err(XmlError::Syntax);
                }
                self.pos += e + 2;
                continue;
            }
            if rest.starts_with("<!--") {
                let Some(e) = rest[4..].find("-->") else { return self.err(XmlError::Syntax) };
                let body = &rest[4..4 + e];
                if bad_chars(body) || body.contains("--") || body.ends_with('-') {
                    return self.err(XmlError::Syntax);
                }
                self.pos += 4 + e + 3;
                continue;
            }
            if rest.starts_with("<![CDATA[") {
                if self.open.is_empty() {
                    return self.err(XmlError::Syntax);
                }
                let Some(e) = rest[9..].find("]]>") else { return self.err(XmlError::Syntax) };
                let raw = &rest[9..9 + e];
                if bad_chars(raw) {
                    return self.err(XmlError::Syntax);
                }
                self.pos += 9 + e + 3;
                let t = if raw.contains('\r') { Cow::Owned(raw.replace("\r\n", "\n").replace('\r', "\n")) } else { Cow::Borrowed(raw) };
                return Ok(Event::Text(t));
            }
            if rest.starts_with("<!") {
                // DOCTYPE and every other declaration: no entities, nothing external
                return self.err(XmlError::Unsupported);
            }
            if rest.starts_with("</") {
                return self.end_tag();
            }
            return self.start_tag();
        }
    }

    fn close(&mut self) {
        let depth = self.open.len();
        while self.bindings.last().is_some_and(|b| b.depth == depth) {
            self.bindings.pop();
        }
        self.open.pop();
    }

    fn end_tag(&mut self) -> XmlResult<Event<'a>> {
        self.pos += 2;
        let name = self.name()?;
        self.skip_space();
        if !self.rest().starts_with('>') || self.open.last() != Some(&name) {
            return self.err(XmlError::Syntax);
        }
        self.pos += 1;
        self.close();
        Ok(Event::End)
    }

    fn start_tag(&mut self) -> XmlResult<Event<'a>> {
        let at = self.pos;
        if self.open.is_empty() && self.root_seen {
            return self.err(XmlError::Syntax);
        }
        self.pos += 1;
        let qname = self.name()?;
        let mut raw: Vec<(&'a str, &'a str)> = Vec::new();
        let empty;
        loop {
            let before = self.pos;
            self.skip_space();
            let rest = self.rest();
            if rest.starts_with("/>") {
                self.pos += 2;
                empty = true;
                break;
            }
            if rest.starts_with('>') {
                self.pos += 1;
                empty = false;
                break;
            }
            if self.pos == before || rest.is_empty() {
                return self.err(XmlError::Syntax);
            }
            let n = self.name()?;
            self.skip_space();
            if !self.rest().starts_with('=') {
                return self.err(XmlError::Syntax);
            }
            self.pos += 1;
            self.skip_space();
            let q = match self.rest().as_bytes().first() {
                Some(&c @ (b'"' | b'\'')) => c as char,
                _ => return self.err(XmlError::Syntax),
            };
            self.pos += 1;
            let Some(e) = self.rest().find(q) else { return self.err(XmlError::Syntax) };
            let v = &self.s[self.pos..self.pos + e];
            if v.contains('<') {
                return self.err(XmlError::Syntax);
            }
            self.pos += e + 1;
            if raw.len() >= MAX_ATTRS {
                return self.err(XmlError::Attributes);
            }
            raw.push((n, v));
        }
        if self.open.len() >= self.max_depth {
            self.pos = at;
            return self.err(XmlError::Depth);
        }
        // duplicate attributes (sorted when there are many: no quadratic time)
        if raw.len() <= 8 {
            for i in 0..raw.len() {
                if raw[..i].iter().any(|(n, _)| *n == raw[i].0) {
                    self.pos = at;
                    return self.err(XmlError::Syntax);
                }
            }
        } else {
            let mut names: Vec<&str> = raw.iter().map(|(n, _)| *n).collect();
            names.sort_unstable();
            if names.windows(2).any(|w| w[0] == w[1]) {
                self.pos = at;
                return self.err(XmlError::Syntax);
            }
        }
        self.open.push(qname);
        self.root_seen = true;
        let depth = self.open.len();
        // the declarations first: they apply to the element's own name and attributes
        let mut attrs = Vec::with_capacity(raw.len());
        for &(n, v) in &raw {
            let Some(value) = unescape(v, true) else {
                self.pos = at;
                return self.err(XmlError::Syntax);
            };
            if n == "xmlns" || n.starts_with("xmlns:") {
                let prefix = n.strip_prefix("xmlns:").unwrap_or("");
                // xml only to its own namespace, xmlns never; no other prefix to those two
                let reserved = value == NS_XML || value == NS_XMLNS;
                let bad = match prefix {
                    "xml" => value != NS_XML,
                    "xmlns" => true,
                    "" => reserved,
                    _ => reserved || value.is_empty(),
                };
                if bad {
                    self.pos = at;
                    return self.err(XmlError::Syntax);
                }
                if prefix != "xml" {
                    if self.bindings.len() >= MAX_BINDINGS {
                        self.pos = at;
                        return self.err(XmlError::Attributes);
                    }
                    self.bindings.push(Binding { prefix, ns: ns_of(&value), depth });
                }
            }
            attrs.push(Attr { qname: n, local: n, ns: Ns::None, value });
        }
        for a in &mut attrs {
            if a.qname == "xmlns" || a.qname.starts_with("xmlns:") {
                a.ns = Ns::Decl;
                a.local = a.qname.strip_prefix("xmlns:").unwrap_or("");
            } else if let Some((p, l)) = a.qname.split_once(':') {
                let Some(ns) = self.lookup(p) else {
                    self.pos = at;
                    return self.err(XmlError::Syntax);
                };
                a.ns = ns;
                a.local = l;
            }
        }
        let (prefix, local) = qname.split_once(':').unwrap_or(("", qname));
        let Some(ns) = self.lookup(prefix) else {
            self.pos = at;
            return self.err(XmlError::Syntax);
        };
        self.pending_end = empty;
        Ok(Event::Start(Start { local, ns, attrs }))
    }

    /// the rest of the element just started (its End included), unread
    pub fn skip(&mut self) -> XmlResult<()> {
        let mut depth = 0usize;
        loop {
            match self.next()? {
                Event::Start(_) => depth += 1,
                Event::End => {
                    if depth == 0 {
                        return Ok(());
                    }
                    depth -= 1;
                }
                Event::Text(..) => {}
                Event::Eof => return self.err(XmlError::Syntax),
            }
        }
    }

    /// the text of the element just started up to its End (CDATA included, nested elements
    /// skipped); None when it is longer than `limit` bytes (the rest is read, not kept)
    pub fn text(&mut self, limit: usize) -> XmlResult<Option<String>> {
        let mut out = String::new();
        let mut over = false;
        loop {
            match self.next()? {
                Event::Start(_) => self.skip()?,
                Event::End => return Ok(if over { None } else { Some(out) }),
                Event::Text(t) => {
                    if over || out.len() + t.len() > limit {
                        over = true;
                    } else {
                        out.push_str(&t);
                    }
                }
                Event::Eof => return self.err(XmlError::Syntax),
            }
        }
    }
}
