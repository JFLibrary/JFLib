// xml -- a small element tree of a theme part (DrawingML with Visio's extensions)
//
// Theme parts are a few dozen kilobytes of attributes; the engine reads colours, fonts and
// style lists from them.  This reader keeps elements (local names, attributes, children) and
// drops text, comments, processing instructions and the DOCTYPE; it refuses nothing: what it
// cannot read ends the tree there.  Limits: 200 000 elements, depth 256.

use alloc::string::String;
use alloc::vec::Vec;

/// one element: its local name (prefix removed), attributes (local names) and children
#[derive(Clone, Debug, Default)]
pub struct Elem {
    /// the name without prefix
    pub name: String,
    /// the attributes without prefixes, values with entities decoded
    pub attrs: Vec<(String, String)>,
    /// the children (indices into Tree::elems)
    pub children: Vec<u32>,
}

/// a parsed part
#[derive(Clone, Debug, Default)]
pub struct Tree {
    /// all elements; 0 is the root when the part had one
    pub elems: Vec<Elem>,
}

const MAX_ELEMS: usize = 200_000;
const MAX_DEPTH: usize = 256;

fn local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, l)| l)
}

fn decode(s: &str) -> String {
    if !s.contains('&') {
        return String::from(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let Some(e) = rest.find(';') else { break };
        let ent = &rest[1..e];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                let n = if let Some(h) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                    u32::from_str_radix(h, 16).ok()
                } else {
                    ent.strip_prefix('#').and_then(|d| d.parse().ok())
                };
                n.and_then(char::from_u32)
            }
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

impl Tree {
    /// read a part (UTF-8; a BOM is skipped)
    pub fn parse(data: &[u8]) -> Tree {
        let text = core::str::from_utf8(data).unwrap_or("");
        let s = text.strip_prefix('\u{feff}').unwrap_or(text);
        let b = s.as_bytes();
        let mut t = Tree::default();
        let mut stack: Vec<u32> = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] != b'<' {
                i += 1;
                continue;
            }
            let rest = &s[i..];
            if rest.starts_with("<?") {
                i += rest.find("?>").map_or(b.len(), |p| p + 2);
                continue;
            }
            if rest.starts_with("<!--") {
                i += rest.find("-->").map_or(b.len(), |p| p + 3);
                continue;
            }
            if rest.starts_with("<![CDATA[") {
                i += rest.find("]]>").map_or(b.len(), |p| p + 3);
                continue;
            }
            if rest.starts_with("<!") {
                i += rest.find('>').map_or(b.len(), |p| p + 1);
                continue;
            }
            if rest.starts_with("</") {
                stack.pop();
                i += rest.find('>').map_or(b.len(), |p| p + 1);
                continue;
            }
            // a start tag: name, attributes, maybe "/>"
            let mut j = i + 1;
            while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
                j += 1;
            }
            let name = local(&s[i + 1..j]);
            let mut attrs = Vec::new();
            let mut empty = false;
            loop {
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= b.len() {
                    return t;
                }
                if b[j] == b'>' {
                    j += 1;
                    break;
                }
                if b[j] == b'/' {
                    empty = true;
                    j = s[j..].find('>').map_or(b.len(), |p| j + p + 1);
                    break;
                }
                let a0 = j;
                while j < b.len() && b[j] != b'=' && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                    j += 1;
                }
                let an = &s[a0..j];
                while j < b.len() && (b[j].is_ascii_whitespace() || b[j] == b'=') {
                    j += 1;
                }
                if j >= b.len() || !(b[j] == b'"' || b[j] == b'\'') {
                    return t;
                }
                let q = b[j];
                let v0 = j + 1;
                let Some(len) = b[v0..].iter().position(|&c| c == q) else { return t };
                let v = &s[v0..v0 + len];
                j = v0 + len + 1;
                if !an.starts_with("xmlns") {
                    attrs.push((String::from(local(an)), decode(v)));
                }
            }
            if t.elems.len() >= MAX_ELEMS || stack.len() >= MAX_DEPTH {
                return t;
            }
            let id = t.elems.len() as u32;
            t.elems.push(Elem { name: String::from(name), attrs, children: Vec::new() });
            if let Some(&p) = stack.last() {
                t.elems[p as usize].children.push(id);
            } else if id != 0 {
                // a second root: ignore everything after the first
                return t;
            }
            if !empty {
                stack.push(id);
            }
            i = j;
        }
        t
    }

    /// an element
    pub fn get(&self, i: u32) -> Option<&Elem> {
        self.elems.get(i as usize)
    }

    /// the first child of `i` named `name`
    pub fn child(&self, i: u32, name: &str) -> Option<u32> {
        self.get(i)?.children.iter().copied().find(|&c| self.elems[c as usize].name == name)
    }

    /// the children of `i` named `name`
    pub fn children<'a>(&'a self, i: u32, name: &'a str) -> impl Iterator<Item = u32> + 'a {
        self.get(i).into_iter().flat_map(move |e| e.children.iter().copied().filter(move |&c| self.elems[c as usize].name == name))
    }

    /// the element at a path of names below `i` (each step the first match)
    pub fn path(&self, i: u32, names: &[&str]) -> Option<u32> {
        names.iter().try_fold(i, |cur, n| self.child(cur, n))
    }

    /// the first element named `name` anywhere below `i` (depth first)
    pub fn find(&self, i: u32, name: &str) -> Option<u32> {
        let mut stack: Vec<u32> = self.get(i)?.children.iter().rev().copied().collect();
        while let Some(c) = stack.pop() {
            let e = &self.elems[c as usize];
            if e.name == name {
                return Some(c);
            }
            stack.extend(e.children.iter().rev().copied());
        }
        None
    }

    /// an attribute of `i`
    pub fn attr(&self, i: u32, name: &str) -> Option<&str> {
        self.get(i)?.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}
