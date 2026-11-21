// xml -- XML for JFLib, one reader in two kinds, and a bounded tree on top. New native functionality;
// no DTD, no I/O, no XInclude, nothing is ever fetched.
//
//   Reader (reader.rs)     the strict pull reader: well-formed XML 1.0 with namespaces; events,
//                          positions, the raw text of an element (skip); DOCTYPE refused
//   Lenient (lenient.rs)   the forgiving reader of files from editors (SVG, fontconfig, theme parts):
//                          tokens, nothing is an error, a DOCTYPE's internal entities with a budget
//   parse(text) -> Node    a tree of the strict subset for interchange formats: element-only or
//                          text-only nodes, names expanded to their namespaces; comments, processing
//                          instructions, CDATA and mixed content refused (Unsupported); bounded
//                          (Limits: bytes, elements, nesting; the names it keeps)
//   escape(text), valid_char(c)   for writers
//
// Every reader of the library is one of these two (img_svg, diagram_io, diagram_core's theme parts,
// text's fontconfig files): one place to fuzz, one set of limits per caller.
use alloc::{string::String, vec::Vec};

pub mod lenient;
pub mod reader;

pub use lenient::{Lenient, Token};
pub use reader::{Attr, Event, ReadError, Reader, Start, NS_XML, NS_XMLNS};

/// Expanded XML name (prefixes are not identities).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub ns: String,
    pub local: String,
}
/// Element-only or text-only node; mixed content is rejected by this subset reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub name: Name,
    pub attrs: Vec<(Name, String)>,
    pub text: String,
    pub children: Vec<Node>,
}
impl Node {
    /// Test an expanded name.
    pub fn is(&self, ns: &str, local: &str) -> bool {
        self.name.ns == ns && self.name.local == local
    }
    /// Attribute lookup; the default namespace never applies to unprefixed attributes.
    pub fn attr(&self, ns: &str, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n.ns == ns && n.local == local)
            .map(|(_, v)| v.as_str())
    }
}
/// Syntax error, resource limit, or an intentionally unsupported XML construct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// not well-formed (or, for the tree, a name it cannot keep)
    Syntax,
    /// a limit of the tree: bytes, elements, nesting, attributes, the names it keeps
    Limit,
    /// a construct the reader does not take: a DOCTYPE or other declaration, an encoding other
    /// than UTF-8 and UTF-16, XML other than 1.0; for the tree also comments, processing
    /// instructions, CDATA and mixed content
    Unsupported,
    /// (Reader) more open elements than its depth
    Depth,
    /// (Reader) more attributes on one element than allowed, or more than 64 namespace bindings
    /// in scope
    Attributes,
}
/// What one parse may take: the text's bytes, its elements, their nesting. The default
/// (4 MiB, 30000 elements, 48 levels) is `parse`'s; a reader whose own model bounds its
/// documents otherwise names what that model can expand to (`parse_with`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub bytes: usize,
    pub nodes: usize,
    pub depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self { bytes: 4 * 1024 * 1024, nodes: 30000, depth: 48 }
    }
}
/// XML 1.0 character validity (also used before emitting document text).
pub fn valid_char(c: char) -> bool {
    matches!(c as u32, 9 | 10 | 13 | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)
}
/// Escape text/attribute content, including CR so it survives XML newline normalization.
pub fn escape(s: &str) -> Result<String, Error> {
    let mut out = String::new();
    for c in s.chars() {
        if !valid_char(c) {
            return Err(Error::Syntax);
        }
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\r' => out.push_str("&#13;"),
            '\n' => out.push_str("&#10;"),
            '\t' => out.push_str("&#9;"),
            _ => out.push(c),
        }
    }
    Ok(out)
}
/// Parse a bounded XML subset into a tree. DTDs, processing instructions, comments, CDATA and
/// mixed content are explicitly rejected (Unsupported). No resources are ever resolved or fetched.
pub fn parse(s: &str) -> Result<Node, Error> {
    parse_with(s, Limits::default())
}

// the bindings' bytes one element may have in scope (prefixes and URIs)
const SCOPE_BYTES: usize = 4096;
// the attributes of one element, namespace declarations included
const TREE_ATTRS: usize = 64;

/// `parse` within other limits than the default ones. The tree is built from the strict Reader
/// without recursion; a limit of the reader (nesting, attributes, bindings) is Limit here.
pub fn parse_with(s: &str, lim: Limits) -> Result<Node, Error> {
    if s.len() > lim.bytes {
        return Err(Error::Limit);
    }
    let mut r = Reader::new(s, lim.depth.saturating_add(1), TREE_ATTRS);
    // the open elements and the bytes of the bindings each one made
    let mut stack: Vec<(Node, usize)> = Vec::new();
    let (mut nodes, mut kept, mut scope) = (0usize, 0usize, 0usize);
    // every name holds a copy of its namespace URI, so a long URI and many names would multiply a
    // small text: the names the tree keeps are bounded too
    let kept_max = lim.bytes.saturating_mul(16);
    let mut root = None;
    loop {
        let ev = r.next().map_err(|e| match e.error {
            Error::Depth | Error::Attributes => Error::Limit,
            e => e,
        })?;
        match ev {
            Event::Start(st) => {
                nodes += 1;
                if nodes > lim.nodes {
                    return Err(Error::Limit);
                }
                let own: usize = st.attrs.iter().filter(|a| a.ns == NS_XMLNS).map(|a| a.local.len() + a.value.len()).sum();
                scope += own;
                if scope > SCOPE_BYTES {
                    return Err(Error::Limit);
                }
                let name = Name { ns: String::from(&*st.ns), local: String::from(st.local) };
                kept = kept.saturating_add(name.ns.len() + name.local.len());
                let mut attrs = Vec::with_capacity(st.attrs.len());
                for a in st.attrs.iter().filter(|a| a.ns != NS_XMLNS) {
                    kept = kept.saturating_add(a.ns.len() + a.local.len());
                    attrs.push((Name { ns: String::from(&*a.ns), local: String::from(a.local) }, String::from(&*a.value)));
                }
                if kept > kept_max {
                    return Err(Error::Limit);
                }
                stack.push((Node { name, attrs, text: String::new(), children: Vec::new() }, own));
            }
            Event::End { .. } => {
                let Some((mut node, own)) = stack.pop() else { return Err(Error::Syntax) };
                scope -= own;
                if !node.children.is_empty() {
                    if !node.text.trim_matches([' ', '\t', '\r', '\n']).is_empty() {
                        return Err(Error::Unsupported);
                    }
                    node.text.clear();
                }
                match stack.last_mut() {
                    Some((parent, _)) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            Event::Text(t) => {
                if let Some((node, _)) = stack.last_mut() {
                    node.text.push_str(&t);
                }
            }
            Event::CData(_) | Event::Comment(_) | Event::Pi { .. } => return Err(Error::Unsupported),
            Event::Eof => return root.ok_or(Error::Syntax),
        }
    }
}
