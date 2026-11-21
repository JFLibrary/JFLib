// xml -- the XML parts of the VSDX family: formats::xml's strict pull reader with the namespaces it knows
//
//   Reader::new(text, depth, attrs)    a part's text (UTF-8 or UTF-16 decoded by text_of)
//   r.next() -> Event                  Start, End, Text, Eof; an empty element <a/> gives Start
//                                      (empty: true) and then End
//   r.skip(&start) -> &str             the rest of an element, returned as the file wrote it
//                                      (an unknown element kept for DF-03)
//
// The reader is formats::xml::Reader (strict: well-formed XML 1.0 with namespaces; mixed content,
// which shape text is: <Text>ab<cp IX='0'/>cd</Text>; positions for the raw text of unknown
// elements).  This file maps its namespace URIs to the few the VSDX reader knows (Ns), its errors to
// XmlError, and skips comments and processing instructions; CDATA is text.  Refused, as there: a
// DOCTYPE (no entities, no external anything), an undefined entity, a character XML does not allow,
// an encoding other than UTF-8 / UTF-16 in the declaration, mismatched or unclosed tags, an unbound
// prefix, a duplicate attribute, more nesting than `depth` and more attributes on one element than
// `attrs`.  Nothing recurses.

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

use formats::xml;

/// the Visio namespace of the VSDX parts
pub(crate) const NS_VISIO: &str = "http://schemas.microsoft.com/office/visio/2012/main";
/// the namespace of r:id
pub(crate) const NS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// the namespace of r:id in strict OOXML
pub(crate) const NS_REL_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";

/// what is wrong with a part's XML
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum XmlError {
    /// not well-formed: a broken tag, attribute, entity or character, tags that do not match,
    /// text outside the root element, an unbound namespace prefix, a duplicate attribute
    Syntax,
    /// deeper nesting than Limits::depth
    Depth,
    /// more attributes on one element than Limits::attrs, or more than 64 namespace
    /// declarations in scope
    Attributes,
    /// a DOCTYPE, or an encoding other than UTF-8 and UTF-16 (declared or as bytes)
    Unsupported,
}

/// the namespace of a name, as far as the reader needs to know it
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum Ns {
    /// no namespace (an unprefixed attribute, an element without default namespace)
    None,
    /// the Visio namespace
    Visio,
    /// the relationships namespace (r:id)
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
        NS_VISIO => Ns::Visio,
        NS_REL | NS_REL_STRICT => Ns::Rel,
        xml::NS_XML => Ns::Xml,
        _ => Ns::Other,
    }
}

/// one attribute
pub(crate) struct Attr<'a> {
    /// the name as written (with its prefix)
    pub qname: &'a str,
    /// the name without prefix
    pub local: &'a str,
    pub ns: Ns,
    /// the value with entities and character references replaced
    pub value: Cow<'a, str>,
}

/// a start tag
pub(crate) struct Start<'a> {
    pub qname: &'a str,
    pub local: &'a str,
    pub ns: Ns,
    pub attrs: Vec<Attr<'a>>,
    /// <a/>: an End follows right away
    pub empty: bool,
    /// the offset of '<'
    pub at: usize,
    /// the offset after '>'
    pub end: usize,
}

impl Start<'_> {
    /// whether this is the Visio element `local`
    pub fn is(&self, local: &str) -> bool {
        self.ns == Ns::Visio && self.local == local
    }
}

/// what the reader found next
pub(crate) enum Event<'a> {
    Start(Start<'a>),
    /// an end tag: the offset after its '>' (the start tag's end for an empty element)
    End { end: usize },
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

/// whether a text holds nothing but XML white space
pub(crate) fn is_blank(s: &str) -> bool {
    s.bytes().all(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r'))
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
    Attr { qname: a.qname, local: a.local, ns, value: a.value }
}

impl<'a> Reader<'a> {
    /// a reader of `s` with at most `depth` open elements and `attrs` attributes per element
    pub fn new(s: &'a str, depth: usize, attrs: usize) -> Self {
        Reader { r: xml::Reader::new(s, depth, attrs) }
    }

    /// the offset the reader is at
    pub fn pos(&self) -> usize {
        self.r.pos()
    }

    /// the source text
    pub fn src(&self) -> &'a str {
        self.r.src()
    }

    /// the next event (comments and processing instructions are skipped, CDATA is text)
    pub fn next(&mut self) -> XmlResult<Event<'a>> {
        loop {
            return Ok(match self.r.next().map_err(kind)? {
                xml::Event::Start(s) => Event::Start(Start {
                    qname: s.qname,
                    local: s.local,
                    ns: ns_of(&s.ns),
                    attrs: s.attrs.into_iter().map(attr).collect(),
                    empty: s.empty,
                    at: s.at,
                    end: s.end,
                }),
                xml::Event::End { end } => Event::End { end },
                xml::Event::Text(t) | xml::Event::CData(t) => Event::Text(t),
                xml::Event::Eof => Event::Eof,
                // comments, processing instructions
                _ => continue,
            });
        }
    }

    /// the rest of the element `start` (just returned by next), as the file wrote it
    pub fn skip(&mut self, start: &Start<'a>) -> XmlResult<&'a str> {
        self.r.skip(start.at).map_err(kind)
    }
}
