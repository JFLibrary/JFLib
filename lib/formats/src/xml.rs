// Bounded, namespace-aware XML subset for interchange. No DTD, entities, I/O or XInclude.
// New native functionality; deliberately rejects constructs it cannot represent losslessly.
use alloc::{string::String, vec::Vec};

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
    Syntax,
    Limit,
    Unsupported,
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
// references decoded; text: element content, where a literal "]]>" is not allowed (it is in
// an attribute value)
fn unescape(s: &str, text: bool) -> Result<String, Error> {
    if s.contains('<') || (text && s.contains("]]>")) {
        return Err(Error::Syntax);
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        let end = rest.find(';').ok_or(Error::Syntax)?;
        let e = &rest[..end];
        let c = match e {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let (digits, radix) = if let Some(d) = e.strip_prefix("#x") {
                    (d, 16)
                } else if let Some(d) = e.strip_prefix('#') {
                    (d, 10)
                } else {
                    return Err(Error::Unsupported);
                };
                if digits.is_empty()
                    || !digits.bytes().all(|b| {
                        if radix == 16 {
                            b.is_ascii_hexdigit()
                        } else {
                            b.is_ascii_digit()
                        }
                    })
                {
                    return Err(Error::Syntax);
                }
                char::from_u32(u32::from_str_radix(digits, radix).map_err(|_| Error::Syntax)?)
                    .ok_or(Error::Syntax)?
            }
        };
        if !valid_char(c) {
            return Err(Error::Syntax);
        }
        out.push(c);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}
type Namespaces = Vec<(String, String)>;
// an NCName of the subset: a letter or '_' first, then letters, digits, '_', '.', '-'
fn ncname(s: &str) -> bool {
    s.as_bytes().first().is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
fn expand(q: &str, namespaces: &Namespaces, attr: bool) -> Result<Name, Error> {
    let (prefix, local) = q.split_once(':').unwrap_or(("", q));
    if !ncname(local) || (!prefix.is_empty() && !ncname(prefix)) {
        return Err(Error::Syntax);
    }
    let ns = if prefix.is_empty() && attr {
        ""
    } else if prefix == "xml" {
        "http://www.w3.org/XML/1998/namespace"
    } else {
        match namespaces.iter().rev().find(|(p, _)| p == prefix) {
            Some((_, n)) => n.as_str(),
            None if prefix.is_empty() => "",
            _ => return Err(Error::Syntax),
        }
    };
    Ok(Name {
        ns: String::from(ns),
        local: String::from(local),
    })
}
struct Parser<'a> {
    s: &'a str,
    pos: usize,
    nodes: usize,
    lim: Limits,
    // bytes of the names the tree keeps: each element and attribute name holds a copy of its
    // namespace URI, so a long URI and many names would multiply a small text
    kept: usize,
}
impl<'a> Parser<'a> {
    fn keep(&mut self, n: &Name) -> Result<(), Error> {
        self.kept = self.kept.saturating_add(n.ns.len() + n.local.len());
        if self.kept > self.lim.bytes.saturating_mul(16) {
            return Err(Error::Limit);
        }
        Ok(())
    }
    fn rest(&self) -> &'a str {
        &self.s[self.pos..]
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.rest().starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }
    fn ws(&mut self) {
        while self
            .rest()
            .as_bytes()
            .first()
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.pos += 1;
        }
    }
    fn name(&mut self) -> Result<&'a str, Error> {
        let start = self.pos;
        if !self
            .rest()
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        {
            return Err(Error::Syntax);
        }
        while self
            .rest()
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b"_:.-".contains(b))
        {
            self.pos += 1;
        }
        Ok(&self.s[start..self.pos])
    }
    fn node(&mut self, ns: &Namespaces, depth: usize) -> Result<Node, Error> {
        self.nodes += 1;
        if depth > self.lim.depth || self.nodes > self.lim.nodes {
            return Err(Error::Limit);
        }
        if !self.eat("<") {
            return Err(Error::Syntax);
        }
        if self.rest().starts_with(['!', '?']) {
            return Err(Error::Unsupported);
        }
        let q = self.name()?;
        let mut scope = ns.clone();
        let mut attrs = Vec::new();
        let empty;
        loop {
            let before = self.pos;
            self.ws();
            if self.eat("/>") {
                empty = true;
                break;
            }
            if self.eat(">") {
                empty = false;
                break;
            }
            if before == self.pos || attrs.len() >= 64 {
                return Err(Error::Syntax);
            }
            let name = self.name()?;
            self.ws();
            if !self.eat("=") {
                return Err(Error::Syntax);
            }
            self.ws();
            let quote = if self.eat("\"") {
                '"'
            } else if self.eat("'") {
                '\''
            } else {
                return Err(Error::Syntax);
            };
            let end = self.rest().find(quote).ok_or(Error::Syntax)?;
            let value = unescape(&self.rest()[..end].replace(['\n', '\t'], " "), false)?;
            self.pos += end + 1;
            if attrs.iter().any(|(n, _)| *n == name) {
                return Err(Error::Syntax);
            }
            if name == "xmlns" || name.starts_with("xmlns:") {
                let prefix = name.strip_prefix("xmlns:").unwrap_or("");
                // "xmlns:" declares a prefix: one there must be (an empty one is no default
                // namespace), and a valid one
                if (name != "xmlns" && !ncname(prefix))
                    || prefix == "xmlns"
                    || (!prefix.is_empty() && value.is_empty())
                    || value == "http://www.w3.org/2000/xmlns/"
                    || (prefix == "xml") != (value == "http://www.w3.org/XML/1998/namespace")
                {
                    return Err(Error::Syntax);
                }
                scope.push((String::from(prefix), value.clone()));
                if scope.len() > 64
                    || scope.iter().map(|(p, n)| p.len() + n.len()).sum::<usize>() > 4096
                {
                    return Err(Error::Limit);
                }
            }
            attrs.push((name, value));
        }
        let mut node = Node {
            name: expand(q, &scope, false)?,
            attrs: Vec::new(),
            text: String::new(),
            children: Vec::new(),
        };
        self.keep(&node.name)?;
        for (q, v) in attrs {
            if q == "xmlns" || q.starts_with("xmlns:") {
                continue;
            }
            let n = expand(q, &scope, true)?;
            self.keep(&n)?;
            if node.attrs.iter().any(|(a, _)| *a == n) {
                return Err(Error::Syntax);
            }
            node.attrs.push((n, v));
        }
        if empty {
            return Ok(node);
        }
        loop {
            if self.eat("</") {
                let end = self.name()?;
                self.ws();
                if end != q || !self.eat(">") {
                    return Err(Error::Syntax);
                }
                break;
            }
            if self.rest().starts_with('<') {
                node.children.push(self.node(&scope, depth + 1)?);
            } else {
                let end = self.rest().find('<').ok_or(Error::Syntax)?;
                node.text.push_str(&unescape(&self.rest()[..end], true)?);
                self.pos += end;
            }
        }
        if !node.children.is_empty() {
            if !node.text.trim_matches([' ', '\t', '\r', '\n']).is_empty() {
                return Err(Error::Unsupported);
            }
            node.text.clear();
        }
        Ok(node)
    }
}
// the inside of "<?xml ... ?>": version="1.0", then optionally encoding and standalone, in this
// order, each after white space, values as written (a reference or another attribute is no
// declaration)
fn xml_decl(d: &str, lim: Limits) -> Result<(), Error> {
    let mut p = Parser { s: d, pos: 0, nodes: 0, lim, kept: 0 };
    let mut last = 0;
    loop {
        let before = p.pos;
        p.ws();
        if p.rest().is_empty() {
            break;
        }
        if before == p.pos {
            return Err(Error::Syntax); // no white space before the attribute
        }
        let name = p.name()?;
        p.ws();
        if !p.eat("=") {
            return Err(Error::Syntax);
        }
        p.ws();
        let quote = if p.eat("\"") {
            '"'
        } else if p.eat("'") {
            '\''
        } else {
            return Err(Error::Syntax);
        };
        let end = p.rest().find(quote).ok_or(Error::Syntax)?;
        let value = &p.rest()[..end];
        p.pos += end + 1;
        let order = match name {
            "version" if value == "1.0" => 1,
            "encoding" if matches!(value, "UTF-8" | "utf-8" | "UTF-16" | "utf-16" | "UTF-16LE" | "UTF-16BE") => 2,
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
/// Parse a bounded XML subset. DTDs, processing instructions, comments, CDATA and mixed
/// content are explicitly rejected. No resources are ever resolved or fetched.
pub fn parse(s: &str) -> Result<Node, Error> {
    parse_with(s, Limits::default())
}
/// `parse` within other limits than the default ones.
pub fn parse_with(s: &str, lim: Limits) -> Result<Node, Error> {
    if s.len() > lim.bytes {
        return Err(Error::Limit);
    }
    if !s.chars().all(valid_char) {
        return Err(Error::Syntax);
    }
    let normalized = s
        .strip_prefix('\u{feff}')
        .unwrap_or(s)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut p = Parser {
        s: &normalized,
        pos: 0,
        nodes: 0,
        lim,
        kept: 0,
    };
    // the declaration: "<?xml" and white space (a space, a tab, a line break), read as it is
    // written -- no references, no namespace attributes
    let r = p.rest().as_bytes();
    if r.starts_with(b"<?xml") && r.get(5).is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n')) {
        p.pos += 5;
        let end = p.rest().find("?>").ok_or(Error::Syntax)?;
        xml_decl(&p.rest()[..end], lim)?;
        p.pos += end + 2;
    }
    p.ws();
    let node = p.node(&Vec::new(), 0)?;
    p.ws();
    if !p.rest().is_empty() {
        return Err(Error::Syntax);
    }
    Ok(node)
}
