// reader -- the strict pull reader of formats::xml: well-formed XML 1.0 with namespaces, without a DTD
//
//   Reader::new(text, depth, attrs)   at most `depth` open elements and `attrs` attributes per element
//   r.next() -> Event                 Start, End, Text, CData, Comment, Pi, Eof; an empty element <a/> gives
//                                     Start (empty: true) and then End
//   r.skip(at) -> &str                the rest of the element whose start tag began at `at`, as written
//   r.pos() / r.src()
//
// Read: elements, attributes in ' or ", the five predefined entities and character references, CDATA,
// comments and processing instructions (as events), the XML declaration (version 1.0; encoding UTF-8 or
// UTF-16, the text is decoded already; standalone; in this order, values as written), a leading U+FEFF, and
// namespaces: every name comes with the URI its prefix is bound to (Namespaces in XML 1.0; xmlns attributes
// are in NS_XMLNS).  Refused: a DOCTYPE and every other declaration (Unsupported: no entities, nothing
// external), an undefined entity, a character XML does not allow, mismatched or unclosed tags, text or a
// second element outside the root, an unbound prefix, a name that is no QName, a duplicate attribute (also
// two prefixes of one namespace), more nesting than `depth` (Depth), more attributes on one element than
// `attrs` or more than 64 namespace bindings in scope (Attributes).  Line ends are normalised as XML says:
// CR LF and CR become LF in text; in attribute values a literal tab, line end or CR becomes a space
// (character references stay).  Nothing recurses: the stack is the reader's own Vec, bounded by `depth`.
// Text is borrowed from the input unless a reference or a CR had to be replaced.

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

use super::Error;

/// the namespace of the xml prefix (xml:lang, xml:space)
pub const NS_XML: &str = "http://www.w3.org/XML/1998/namespace";
/// the namespace of namespace declarations (xmlns, xmlns:p)
pub const NS_XMLNS: &str = "http://www.w3.org/2000/xmlns/";
/// the most namespace bindings in scope at a time
const MAX_BINDINGS: usize = 64;

/// one attribute of a start tag
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr<'a> {
    /// the name as written (with its prefix)
    pub qname: &'a str,
    /// the name without prefix (for a declaration: the prefix it binds, "" for xmlns)
    pub local: &'a str,
    /// the namespace URI ("" for an unprefixed attribute; NS_XMLNS for a declaration)
    pub ns: Cow<'a, str>,
    /// the value with references replaced and white space normalised
    pub value: Cow<'a, str>,
}

/// a start tag
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Start<'a> {
    /// the name as written (with its prefix)
    pub qname: &'a str,
    /// the name without prefix
    pub local: &'a str,
    /// the namespace URI ("" for none)
    pub ns: Cow<'a, str>,
    /// the attributes in their order, namespace declarations included
    pub attrs: Vec<Attr<'a>>,
    /// <a/>: an End follows right away
    pub empty: bool,
    /// the offset of '<'
    pub at: usize,
    /// the offset after '>'
    pub end: usize,
}

impl Start<'_> {
    /// whether this is the element `local` of the namespace `ns`
    pub fn is(&self, ns: &str, local: &str) -> bool {
        self.local == local && self.ns == ns
    }

    /// the value of the attribute `local` of the namespace `ns` ("" for an unprefixed one)
    pub fn attr(&self, ns: &str, local: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.local == local && a.ns == ns && a.ns != NS_XMLNS).map(|a| &*a.value)
    }
}

/// what the reader found next
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event<'a> {
    /// a start tag
    Start(Start<'a>),
    /// an end tag: the offset after its '>' (the start tag's end for an empty element)
    End { end: usize },
    /// character data with references replaced
    Text(Cow<'a, str>),
    /// a CDATA section's content (line ends normalised)
    CData(Cow<'a, str>),
    /// a comment's content
    Comment(&'a str),
    /// a processing instruction
    Pi { target: &'a str, data: &'a str },
    /// the end of a well-formed document
    Eof,
}

/// an error and the byte offset it was found at
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadError {
    pub error: Error,
    pub at: usize,
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
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

// an NCName: no colon, not empty, not starting with a digit, '-' or '.'
fn ncname(s: &str) -> bool {
    !s.is_empty() && !s.contains(':') && !matches!(s.as_bytes()[0], b'0'..=b'9' | b'-' | b'.')
}

// replace entities and character references; `attr`: literal white space becomes a space
pub(super) fn unescape(raw: &str, attr: bool) -> Option<Cow<'_, str>> {
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
    uri: Cow<'a, str>,
    depth: usize,
}

/// the strict pull reader (see the top of this file)
pub struct Reader<'a> {
    s: &'a str,
    pos: usize,
    open: Vec<&'a str>,
    bindings: Vec<Binding<'a>>,
    max_depth: usize,
    max_attrs: usize,
    // an empty element was returned: its End comes next
    pending_end: Option<usize>,
    root_seen: bool,
}

impl<'a> Reader<'a> {
    /// a reader of `s` with at most `depth` open elements and `attrs` attributes per element; a leading
    /// U+FEFF is skipped
    pub fn new(s: &'a str, depth: usize, attrs: usize) -> Self {
        let pos = if s.starts_with('\u{feff}') { 3 } else { 0 };
        Reader { s, pos, open: Vec::new(), bindings: Vec::new(), max_depth: depth, max_attrs: attrs, pending_end: None, root_seen: false }
    }

    /// the offset the reader is at
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// the source text
    pub fn src(&self) -> &'a str {
        self.s
    }

    /// the elements open now
    pub fn depth(&self) -> usize {
        self.open.len()
    }

    fn err<T>(&self, error: Error) -> Result<T, ReadError> {
        Err(ReadError { error, at: self.pos })
    }

    fn err_at<T>(&mut self, error: Error, at: usize) -> Result<T, ReadError> {
        self.pos = at;
        Err(ReadError { error, at })
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

    // a QName: NCName or NCName:NCName
    fn name(&mut self) -> Result<&'a str, ReadError> {
        let b = self.s.as_bytes();
        let start = self.pos;
        while self.pos < b.len() && !name_end(b[self.pos]) {
            self.pos += 1;
        }
        let n = &self.s[start..self.pos];
        let name_char = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':') || c >= 0x80;
        if !n.bytes().all(name_char) || n.contains(['\u{fffe}', '\u{ffff}']) {
            return self.err(Error::Syntax);
        }
        let ok = match n.split_once(':') {
            Some((p, l)) => ncname(p) && ncname(l),
            None => ncname(n),
        };
        if !ok {
            return self.err(Error::Syntax);
        }
        Ok(n)
    }

    // the namespace bound to a prefix ("" = default)
    fn lookup(&self, prefix: &str) -> Option<Cow<'a, str>> {
        if prefix == "xml" {
            return Some(Cow::Borrowed(NS_XML));
        }
        match self.bindings.iter().rev().find(|b| b.prefix == prefix) {
            Some(b) => Some(b.uri.clone()),
            None if prefix.is_empty() => Some(Cow::Borrowed("")),
            None => None,
        }
    }

    // the inside of "<?xml ... ?>": version="1.0", then optionally encoding and standalone, in this order,
    // each after white space, values as written (a reference or another attribute is no declaration)
    fn declaration(&mut self, body: &str) -> Result<(), Error> {
        let b = body.as_bytes();
        let mut i = 0;
        let mut last = 0;
        loop {
            let before = i;
            while i < b.len() && is_space(b[i]) {
                i += 1;
            }
            if i == b.len() {
                break;
            }
            if before == i {
                return Err(Error::Syntax); // no white space before the attribute
            }
            let n0 = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b':') {
                i += 1;
            }
            let name = &body[n0..i];
            while i < b.len() && is_space(b[i]) {
                i += 1;
            }
            if b.get(i) != Some(&b'=') || name.is_empty() {
                return Err(Error::Syntax);
            }
            i += 1;
            while i < b.len() && is_space(b[i]) {
                i += 1;
            }
            let q = match b.get(i) {
                Some(&c @ (b'"' | b'\'')) => c,
                _ => return Err(Error::Syntax),
            };
            let v0 = i + 1;
            let Some(len) = b[v0..].iter().position(|&c| c == q) else { return Err(Error::Syntax) };
            let value = &body[v0..v0 + len];
            i = v0 + len + 1;
            let order = match name {
                "version" if value == "1.0" => 1,
                "encoding" if ["utf-8", "utf-16", "utf-16le", "utf-16be"].iter().any(|e| value.eq_ignore_ascii_case(e)) => 2,
                "standalone" if matches!(value, "yes" | "no") => 3,
                _ => return Err(Error::Unsupported),
            };
            if order <= last || (last == 0 && order != 1) {
                return Err(if last == 0 { Error::Unsupported } else { Error::Syntax });
            }
            last = order;
        }
        if last == 0 {
            return Err(Error::Unsupported); // no version
        }
        Ok(())
    }

    /// the next event; after an error the reader is not meant to go on
    pub fn next(&mut self) -> Result<Event<'a>, ReadError> {
        if let Some(end) = self.pending_end.take() {
            self.close();
            return Ok(Event::End { end });
        }
        loop {
            if self.pos >= self.s.len() {
                if !self.open.is_empty() || !self.root_seen {
                    return self.err(Error::Syntax);
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
                    if !raw.bytes().all(is_space) {
                        return self.err_at(Error::Syntax, start);
                    }
                    continue;
                }
                if raw.contains("]]>") {
                    return self.err_at(Error::Syntax, start);
                }
                return match unescape(raw, false) {
                    Some(t) => Ok(Event::Text(t)),
                    None => self.err_at(Error::Syntax, start),
                };
            }
            if rest.starts_with("<?") {
                let first = self.pos == 0 || (self.pos == 3 && self.s.starts_with('\u{feff}'));
                if rest.starts_with("<?xml") && rest.as_bytes().get(5).is_some_and(|&c| is_space(c)) {
                    if !first {
                        return self.err(Error::Syntax);
                    }
                    let Some(end) = rest[5..].find("?>").map(|e| e + 5) else { return self.err(Error::Syntax) };
                    if let Err(e) = self.declaration(&rest[5..end]) {
                        return self.err(e);
                    }
                    self.pos += end + 2;
                    continue;
                }
                // a processing instruction: a target that is a name and not "xml" in any case ("?>" is
                // searched after "<?": in "<?>" it would overlap the start)
                let Some(e) = rest[2..].find("?>").map(|e| e + 2) else { return self.err(Error::Syntax) };
                let body = &rest[2..e];
                let t_end = body.find(|c: char| c.is_ascii_whitespace()).unwrap_or(body.len());
                let target = &body[..t_end];
                if target.is_empty() || target.eq_ignore_ascii_case("xml") || bad_chars(body) || target.bytes().any(name_end) {
                    return self.err(Error::Syntax);
                }
                let data = body[t_end..].trim_start_matches(|c: char| c.is_ascii_whitespace());
                self.pos += e + 2;
                return Ok(Event::Pi { target, data });
            }
            if rest.starts_with("<!--") {
                let Some(e) = rest[4..].find("-->") else { return self.err(Error::Syntax) };
                let body = &rest[4..4 + e];
                if bad_chars(body) || body.contains("--") || body.ends_with('-') {
                    return self.err(Error::Syntax);
                }
                self.pos += 4 + e + 3;
                return Ok(Event::Comment(body));
            }
            if rest.starts_with("<![CDATA[") {
                if self.open.is_empty() {
                    return self.err(Error::Syntax);
                }
                let Some(e) = rest[9..].find("]]>") else { return self.err(Error::Syntax) };
                let raw = &rest[9..9 + e];
                if bad_chars(raw) {
                    return self.err(Error::Syntax);
                }
                self.pos += 9 + e + 3;
                let t = if raw.contains('\r') { Cow::Owned(raw.replace("\r\n", "\n").replace('\r', "\n")) } else { Cow::Borrowed(raw) };
                return Ok(Event::CData(t));
            }
            if rest.starts_with("<!") {
                // DOCTYPE and every other declaration: no entities, nothing external
                return self.err(Error::Unsupported);
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

    fn end_tag(&mut self) -> Result<Event<'a>, ReadError> {
        self.pos += 2;
        let name = self.name()?;
        self.skip_space();
        if !self.rest().starts_with('>') || self.open.last() != Some(&name) {
            return self.err(Error::Syntax);
        }
        self.pos += 1;
        self.close();
        Ok(Event::End { end: self.pos })
    }

    fn start_tag(&mut self) -> Result<Event<'a>, ReadError> {
        let at = self.pos;
        if self.open.is_empty() && self.root_seen {
            return self.err(Error::Syntax);
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
                return self.err(Error::Syntax);
            }
            let n = self.name()?;
            self.skip_space();
            if !self.rest().starts_with('=') {
                return self.err(Error::Syntax);
            }
            self.pos += 1;
            self.skip_space();
            let q = match self.rest().as_bytes().first() {
                Some(&c @ (b'"' | b'\'')) => c as char,
                _ => return self.err(Error::Syntax),
            };
            self.pos += 1;
            let Some(e) = self.rest().find(q) else { return self.err(Error::Syntax) };
            let v = &self.s[self.pos..self.pos + e];
            if v.contains('<') {
                return self.err(Error::Syntax);
            }
            self.pos += e + 1;
            if raw.len() >= self.max_attrs {
                return self.err(Error::Attributes);
            }
            raw.push((n, v));
        }
        let end = self.pos;
        if self.open.len() >= self.max_depth {
            return self.err_at(Error::Depth, at);
        }
        if has_duplicate(raw.iter().map(|(n, _)| *n)) {
            return self.err_at(Error::Syntax, at);
        }
        self.open.push(qname);
        self.root_seen = true;
        let depth = self.open.len();
        // the declarations first: they apply to the element's own name and attributes
        let mut attrs = Vec::with_capacity(raw.len());
        for &(n, v) in &raw {
            let Some(value) = unescape(v, true) else { return self.err_at(Error::Syntax, at) };
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
                    return self.err_at(Error::Syntax, at);
                }
                if prefix != "xml" {
                    if self.bindings.len() >= MAX_BINDINGS {
                        return self.err_at(Error::Attributes, at);
                    }
                    self.bindings.push(Binding { prefix, uri: value.clone(), depth });
                }
                attrs.push(Attr { qname: n, local: prefix, ns: Cow::Borrowed(NS_XMLNS), value });
            } else {
                attrs.push(Attr { qname: n, local: n, ns: Cow::Borrowed(""), value });
            }
        }
        for a in &mut attrs {
            if a.ns != NS_XMLNS {
                if let Some((p, l)) = a.qname.split_once(':') {
                    let Some(ns) = self.lookup(p) else { return self.err_at(Error::Syntax, at) };
                    a.ns = ns;
                    a.local = l;
                }
            }
        }
        // two prefixes of one namespace give the same attribute twice
        if attrs.iter().filter(|a| !a.ns.is_empty() && a.ns != NS_XMLNS).count() > 1 {
            let mut names: Vec<(&str, &str)> = attrs.iter().filter(|a| !a.ns.is_empty() && a.ns != NS_XMLNS).map(|a| (&*a.ns, a.local)).collect();
            names.sort_unstable();
            if names.windows(2).any(|w| w[0] == w[1]) {
                return self.err_at(Error::Syntax, at);
            }
        }
        let (prefix, local) = qname.split_once(':').unwrap_or(("", qname));
        let Some(ns) = self.lookup(prefix) else { return self.err_at(Error::Syntax, at) };
        if empty {
            self.pending_end = Some(end);
        }
        Ok(Event::Start(Start { qname, local, ns, attrs, empty, at, end }))
    }

    /// the rest of the element whose start tag began at `at` (just returned by next), as the file
    /// wrote it
    pub fn skip(&mut self, at: usize) -> Result<&'a str, ReadError> {
        let mut depth = 0usize;
        loop {
            match self.next()? {
                Event::Start(_) => depth += 1,
                Event::End { end } => {
                    if depth == 0 {
                        return Ok(&self.s[at..end]);
                    }
                    depth -= 1;
                }
                Event::Eof => return self.err(Error::Syntax),
                _ => {}
            }
        }
    }
}

// whether a name occurs twice (sorted when there are many: no quadratic time)
fn has_duplicate<'b>(names: impl Iterator<Item = &'b str> + Clone) -> bool {
    let n = names.clone().count();
    if n <= 8 {
        let v: Vec<&str> = names.collect();
        return (0..v.len()).any(|i| v[..i].contains(&v[i]));
    }
    let mut v: Vec<&str> = names.collect();
    v.sort_unstable();
    v.windows(2).any(|w| w[0] == w[1])
}
