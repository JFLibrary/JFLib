// xml -- a small element tree of a theme part (DrawingML with Visio's extensions)
//
// Theme parts are a few dozen kilobytes of attributes; the engine reads colours, fonts and
// style lists from them.  The reading is formats::xml::Lenient's (nothing is an error: a broken
// tag or reference is read as far as it goes); this tree keeps elements (local names, attributes
// without the xmlns declarations, children) and drops text, comments, processing instructions and
// the DOCTYPE; an end tag closes the element open last.  Limits: 200 000 elements, depth 256; a
// second root element or a limit ends the tree there.

use alloc::string::String;
use alloc::vec::Vec;

use formats::xml::{Lenient, Token};

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

impl Tree {
    /// read a part (UTF-8; a BOM is skipped)
    pub fn parse(data: &[u8]) -> Tree {
        let text = core::str::from_utf8(data).unwrap_or("");
        let s = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut lex = Lenient::new(s);
        let mut t = Tree::default();
        let mut stack: Vec<u32> = Vec::new();
        loop {
            match lex.next() {
                Token::Start { name, attrs, empty } => {
                    if t.elems.len() >= MAX_ELEMS || stack.len() >= MAX_DEPTH {
                        return t;
                    }
                    let mut kept = Vec::with_capacity(attrs.len());
                    for (n, v) in attrs.iter().filter(|(n, _)| !n.starts_with("xmlns")) {
                        let mut value = String::new();
                        lex.decode(v, &mut value);
                        kept.push((String::from(local(n)), value));
                    }
                    let id = t.elems.len() as u32;
                    t.elems.push(Elem { name: String::from(local(name)), attrs: kept, children: Vec::new() });
                    if let Some(&p) = stack.last() {
                        t.elems[p as usize].children.push(id);
                    } else if id != 0 {
                        // a second root: ignore everything after the first
                        return t;
                    }
                    if !empty {
                        stack.push(id);
                    }
                }
                Token::End { .. } => {
                    stack.pop();
                }
                Token::Eof => return t,
                // text and CDATA: not kept
                _ => {}
            }
        }
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
