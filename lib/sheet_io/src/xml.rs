// xml -- the XML parts of a workbook: formats::xml's strict pull reader with the namespaces it knows
//
//   Reader::new(text, depth)           a part's text (text_of decodes UTF-8 or UTF-16)
//   r.next() -> Event                  Start, End, Text, Eof; an empty element <a/> gives Start
//                                      and then End
//   r.skip()                           the rest of the element just started, unread
//   r.text(limit)                      the text of the element just started
//
// Why not formats::xml::parse: it builds a tree of every element, and a worksheet of a million
// cells would be a tree of gigabytes.  The pull reader hands out one event at a time and keeps only
// the names of the open elements.
//
// The reader is formats::xml::Reader (strict: well-formed XML 1.0 with namespaces), as in
// diagram_io.  This file maps its namespace URIs to the few the workbook reader knows (Ns), its
// errors to XmlError, and skips comments and processing instructions; CDATA is text.  Refused, as
// there: a DOCTYPE (no entity declarations, nothing external, so no entity expansion at all), an
// undefined entity, a character XML does not allow, an encoding other than UTF-8 / UTF-16 in the
// declaration, mismatched or unclosed tags, an unbound prefix, a duplicate attribute, more nesting
// than `depth`, more than MAX_ATTRS attributes on one element, more than 64 namespace declarations
// in scope.  Line ends and attribute values are normalised as XML says.  Nothing recurses.

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

use formats::xml;

/// SpreadsheetML, transitional
pub(crate) const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// SpreadsheetML, strict
pub(crate) const NS_MAIN_STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
/// the namespace of r:id, transitional
pub(crate) const NS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// the namespace of r:id, strict
pub(crate) const NS_REL_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
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
        xml::NS_XML => Ns::Xml,
        _ => Ns::Other,
    }
}

/// one attribute
pub(crate) struct Attr<'a> {
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

/// a text without the XML white space at either end
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\n', '\r'])
}

/// the pull reader of one part: formats::xml's strict Reader, its namespaces as Ns
pub(crate) struct Reader<'a> {
    r: xml::Reader<'a>,
}

fn kind(e: xml::ReadError) -> (XmlError, usize) {
    let k = match e.error {
        xml::Error::Depth => XmlError::Depth,
        xml::Error::Attributes => XmlError::Attributes,
        xml::Error::Unsupported => XmlError::Unsupported,
        _ => XmlError::Syntax,
    };
    (k, e.at)
}

fn attr<'a>(a: xml::Attr<'a>) -> Attr<'a> {
    let ns = if a.ns == xml::NS_XMLNS { Ns::Decl } else { ns_of(&a.ns) };
    Attr { local: a.local, ns, value: a.value }
}

impl<'a> Reader<'a> {
    /// a reader of `s` with at most `depth` open elements
    pub fn new(s: &'a str, depth: usize) -> Self {
        Reader { r: xml::Reader::new(s, depth.max(1), MAX_ATTRS) }
    }

    /// the next event (comments and processing instructions are skipped, CDATA is text)
    pub fn next(&mut self) -> XmlResult<Event<'a>> {
        loop {
            return Ok(match self.r.next().map_err(kind)? {
                xml::Event::Start(s) => Event::Start(Start { local: s.local, ns: ns_of(&s.ns), attrs: s.attrs.into_iter().map(attr).collect() }),
                xml::Event::End { .. } => Event::End,
                xml::Event::Text(t) | xml::Event::CData(t) => Event::Text(t),
                xml::Event::Eof => Event::Eof,
                // comments, processing instructions
                _ => continue,
            });
        }
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
                Event::Eof => return Err((XmlError::Syntax, self.r.pos())),
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
                Event::Eof => return Err((XmlError::Syntax, self.r.pos())),
            }
        }
    }
}
