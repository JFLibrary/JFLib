// md_dom -- the elements of an HTML document of the viewer (MdDoc::html, md_doc.rs): a tree whose
// nodes keep their identity across changes, queries by simple selectors (also the selectors of
// the style sheets, rich_html.rs), changes of text, attributes, classes and structure; the
// blocks follow at MdDoc::update (md_view.rs does it every frame)
//
//   doc.root() -> MdNode                 the document: its children are the HTML's top level
//   doc.by_id(id) -> Option<MdNode>      the first element in the document with this id (an
//                                        index: O(log n))
//   doc.select(sel) / select_first(sel)  the elements of the document a selector list matches,
//                                        in document order
//   doc.select_in(n, sel) / matches(n, sel)   among n's descendants / n itself
//   doc.parent(n) / children(n) / next_sibling(n) / prev_sibling(n)
//   doc.tag(n) ("" for a text) / is_text(n) / alive(n) / attached(n) (in the document)
//   doc.attr(n, name) / attrs(n) / has_class(n, c) / text_of(n) (all its text, textContent) /
//   inner_html(n)
//   changes (true / a node: done; false / MdNode::NONE: a node that is gone, a Markdown
//   document, a name that is none, the cap of nodes):
//   doc.set_text(n, s) (an element's children become one text; they are freed) /
//   set_attr(n, name, value) / remove_attr(n, name) / add_class(n, c) / remove_class(n, c) /
//   toggle_class(n, c) -> on / create_element(tag) / create_text(s) (not in the document yet) /
//   insert(parent, before, n) (before: None = at the end; a node in the document moves) /
//   remove(n) (out of the document, kept: it may go in again) / free(n) (gone: its handles and
//   those of its descendants refer to nothing) / set_inner_html(n, html)
//   MdNode::raw() / MdNode::from_raw(v): a handle as a number (the objects of a script host)
//
// The tree: HTML read the way browsers do, simplified -- void elements (br, img, input ...),
// raw text (script, style, textarea, title, template, noscript, svg), a <p> closed by a block,
// <li> by <li>, <dd> / <dt>, <td> / <th>, <tr>, <option> by their like, <head> by the first
// element of the body; an end tag closes the elements above its own, but not across a table
// cell or a button, and an inline element's end tag does not close a block (it is dropped); a
// stray </p> is an empty paragraph; comments go.  Depth (DEPTH_MAX open elements: deeper ones
// stay empty, their content follows them) and nodes (NODES_MAX: the rest is cut, truncated)
// are capped, so parsing stays linear.
//
// Selectors: a list (a, b) of compounds -- a tag or *, #id, .class, [attr], [attr=v] (also ~=
// |= ^= $= *=, values in quotes or not) -- joined by descendant (blank) and child (>)
// combinators; tags and attribute names ignore case, ids, classes and values do not.  Anything
// else (pseudo-classes, sibling combinators, escapes) makes the selector match nothing.  A
// match costs O(depth x compounds) per element (each run of child combinators is matched at
// the nearest ancestor that takes it), a query at most a budget proportional to the tree.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::md_doc::MdDoc;
use crate::rich_html::{attrs, decode, find_ci, tag_end};

pub(crate) const NIL: u32 = u32::MAX;
// live nodes, open elements at most
const NODES_MAX: usize = 1 << 20;
const DEPTH_MAX: usize = 256;
// compounds of a selector, selectors of a list, bytes of a selector list
const SEL_PARTS: usize = 16;
const SEL_LIST: usize = 64;
const SEL_BYTES: usize = 4096;

pub(crate) const K_ROOT: u8 = 0;
pub(crate) const K_ELEM: u8 = 1;
pub(crate) const K_TEXT: u8 = 2;

/// a node of an HTML document's element tree (an element or a text): the same across changes
/// of the document; a freed node's handle refers to nothing (MdDoc::alive)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MdNode {
    i: u32,
    g: u32,
}

impl MdNode {
    /// no node
    pub const NONE: MdNode = MdNode { i: 0, g: 0 };

    /// it is MdNode::NONE
    pub fn is_none(self) -> bool {
        self.g == 0
    }

    /// the handle as a number (0: none), e.g. for a script host's objects
    pub fn raw(self) -> u64 {
        (self.g as u64) << 32 | self.i as u64
    }

    /// the handle of raw()
    pub fn from_raw(v: u64) -> MdNode {
        MdNode { i: v as u32, g: (v >> 32) as u32 }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    // the generation: 0 a free slot (a handle with another one is stale)
    pub(crate) serial: u32,
    pub(crate) kind: u8,
    pub(crate) parent: u32,
    pub(crate) first: u32,
    pub(crate) last: u32,
    pub(crate) prev: u32,
    pub(crate) next: u32,
    // an element's tag (lower case), a text's text
    pub(crate) name: String,
    pub(crate) attrs: Vec<(String, String)>,
    // derived (MdDoc::update): the one block this block element shows by itself (NIL: none)
    pub(crate) blk: u32,
}

/// the element tree of an HTML document (see the module header)
#[derive(Clone, Debug, Default)]
pub(crate) struct Dom {
    pub(crate) nodes: Vec<Node>,
    // free slots and the generation their next node gets
    free: Vec<(u32, u32)>,
    // elements with an id (in the document or not)
    ids: BTreeMap<String, Vec<u32>>,
    // the tree changed since the blocks were derived; the elements whose content (false) or
    // own attributes (true) changed -- None: too many, every block again
    pub(crate) changed: bool,
    pub(crate) touched: Option<Vec<(u32, bool)>>,
    // the cap of nodes cut the input
    pub(crate) truncated: bool,
}

// how many touched elements a change may name before it counts as everything
const TOUCHED_MAX: usize = 64;

// ------------------------------------------------------------------ tags

// a tag's part in building the tree
const C_INLINE: u8 = 0;
const C_BLOCK: u8 = 1;
const C_P: u8 = 2;
const C_LI: u8 = 3;
const C_DDT: u8 = 4;
const C_TCELL: u8 = 5;
const C_TR: u8 = 6;
const C_TSECT: u8 = 7;
const C_TABLE: u8 = 8;
const C_LIST: u8 = 9;
const C_DL: u8 = 10;
const C_OPTION: u8 = 11;
const C_OPTGROUP: u8 = 12;
const C_HEAD: u8 = 13;
const C_HEADING: u8 = 14;
const C_BUTTON: u8 = 15;
const C_SELECT: u8 = 16;

fn code(t: &str) -> u8 {
    match t {
        "p" => C_P,
        "li" => C_LI,
        "dd" | "dt" => C_DDT,
        "td" | "th" => C_TCELL,
        "tr" => C_TR,
        "thead" | "tbody" | "tfoot" => C_TSECT,
        "table" => C_TABLE,
        "ul" | "ol" | "menu" | "dir" => C_LIST,
        "dl" => C_DL,
        "option" => C_OPTION,
        "optgroup" => C_OPTGROUP,
        "head" => C_HEAD,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => C_HEADING,
        "button" => C_BUTTON,
        "select" => C_SELECT,
        "div" | "blockquote" | "pre" | "section" | "article" | "header" | "footer" | "address" | "figure" | "figcaption" | "center" | "nav"
        | "aside" | "main" | "form" | "fieldset" | "legend" | "details" | "summary" | "dialog" | "hgroup" | "caption" | "html" | "body"
        | "colgroup" | "listing" | "xmp" | "plaintext" => C_BLOCK,
        _ => C_INLINE,
    }
}

/// elements without content
pub(crate) fn void(t: &str) -> bool {
    matches!(t, "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "param" | "source" | "track" | "wbr" | "keygen")
}

// elements whose content is text up to their end tag
fn raw(t: &str) -> bool {
    matches!(t, "script" | "style" | "textarea" | "title" | "template" | "noscript" | "svg" | "xmp")
}

// elements of the head (others end it)
fn head_tag(t: &str) -> bool {
    matches!(t, "base" | "link" | "meta" | "title" | "style" | "script" | "noscript" | "template")
}

// a start tag that ends an open <p>
fn closes_p(t: &str) -> bool {
    matches!(
        t,
        "address" | "article" | "aside" | "blockquote" | "center" | "details" | "dialog" | "dir" | "div" | "dl" | "fieldset" | "figcaption"
            | "figure" | "footer" | "form" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "header" | "hgroup" | "hr" | "main" | "menu" | "nav"
            | "ol" | "p" | "pre" | "section" | "summary" | "table" | "ul" | "li" | "dd" | "dt" | "listing" | "xmp" | "plaintext"
    )
}

// an element that may lie inside a paragraph read again by itself (Dom::leaf_of): it neither
// starts blocks or paragraphs nor adds links, images, fields or lists
fn leaf_inside(t: &str) -> bool {
    !matches!(
        t,
        "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "blockquote" | "pre" | "tr" | "dt" | "dd" | "section" | "article" | "header"
            | "footer" | "table" | "address" | "figure" | "center" | "thead" | "tbody" | "tfoot" | "td" | "th" | "caption" | "ul" | "ol" | "dl"
            | "br" | "hr" | "img" | "input" | "select" | "textarea" | "button" | "label" | "form" | "option" | "optgroup" | "a" | "script"
            | "style" | "head" | "title" | "template" | "noscript" | "svg" | "meta" | "link" | "html" | "body" | "xmp" | "listing" | "plaintext"
    )
}

// a name of a tag or attribute that can be written into HTML
fn name_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b > b' ' && !matches!(b, b'"' | b'\'' | b'>' | b'/' | b'=' | b'<' | 0x7F))
}

// ------------------------------------------------------------------ the tree

impl Dom {
    /// an empty tree: the root alone
    pub(crate) fn new() -> Dom {
        let mut d = Dom { touched: Some(Vec::new()), ..Dom::default() };
        d.nodes.push(Node { serial: 1, kind: K_ROOT, parent: NIL, first: NIL, last: NIL, prev: NIL, next: NIL, name: String::from("#document"), attrs: Vec::new(), blk: NIL });
        d
    }

    /// the tree of an HTML text
    pub(crate) fn parse(s: &str) -> Dom {
        let mut d = Dom::new();
        build(&mut d, 0, s);
        d
    }

    /// a tree of HTML (a document of the viewer has one: the root exists)
    pub(crate) fn is_html(&self) -> bool {
        !self.nodes.is_empty()
    }

    pub(crate) fn handle(&self, i: u32) -> MdNode {
        match self.nodes.get(i as usize) {
            Some(n) if n.serial != 0 => MdNode { i, g: n.serial },
            _ => MdNode::NONE,
        }
    }

    /// the index of a live node
    pub(crate) fn get(&self, n: MdNode) -> Option<u32> {
        (n.g != 0 && self.nodes.get(n.i as usize).is_some_and(|x| x.serial == n.g)).then_some(n.i)
    }

    fn live(&self) -> usize {
        self.nodes.len() - self.free.len()
    }

    // a new node (not in the tree) -> None: the cap
    fn alloc(&mut self, kind: u8, name: String, attrs: Vec<(String, String)>) -> Option<u32> {
        if self.live() >= NODES_MAX {
            return None;
        }
        let i = match self.free.pop() {
            Some((i, g)) => {
                self.nodes[i as usize] = Node { serial: g, kind, parent: NIL, first: NIL, last: NIL, prev: NIL, next: NIL, name, attrs, blk: NIL };
                i
            }
            None => {
                self.nodes.push(Node { serial: 1, kind, parent: NIL, first: NIL, last: NIL, prev: NIL, next: NIL, name, attrs, blk: NIL });
                self.nodes.len() as u32 - 1
            }
        };
        if kind == K_ELEM {
            if let Some(id) = self.id_of(i) {
                let id = String::from(id);
                self.ids.entry(id).or_default().push(i);
            }
        }
        Some(i)
    }

    pub(crate) fn attr(&self, i: u32, k: &str) -> Option<&str> {
        self.nodes[i as usize].attrs.iter().find(|a| a.0 == k).map(|a| a.1.as_str())
    }

    fn id_of(&self, i: u32) -> Option<&str> {
        self.attr(i, "id").filter(|v| !v.is_empty())
    }

    fn id_drop(&mut self, i: u32) {
        if let Some(id) = self.id_of(i) {
            let id = String::from(id);
            if let Some(v) = self.ids.get_mut(&id) {
                v.retain(|&x| x != i);
                if v.is_empty() {
                    self.ids.remove(&id);
                }
            }
        }
    }

    fn id_add(&mut self, i: u32) {
        if let Some(id) = self.id_of(i) {
            let id = String::from(id);
            self.ids.entry(id).or_default().push(i);
        }
    }

    /// c as the last child of p
    fn append(&mut self, p: u32, c: u32) {
        self.insert_before(p, c, NIL);
    }

    // c (detached) into p before `before` (NIL: at the end)
    fn insert_before(&mut self, p: u32, c: u32, before: u32) {
        let prev = if before == NIL { self.nodes[p as usize].last } else { self.nodes[before as usize].prev };
        {
            let n = &mut self.nodes[c as usize];
            n.parent = p;
            n.prev = prev;
            n.next = before;
        }
        if prev == NIL {
            self.nodes[p as usize].first = c;
        } else {
            self.nodes[prev as usize].next = c;
        }
        if before == NIL {
            self.nodes[p as usize].last = c;
        } else {
            self.nodes[before as usize].prev = c;
        }
    }

    // c out of its parent
    fn detach(&mut self, c: u32) {
        let (p, prev, next) = {
            let n = &self.nodes[c as usize];
            (n.parent, n.prev, n.next)
        };
        if p == NIL {
            return;
        }
        if prev == NIL {
            self.nodes[p as usize].first = next;
        } else {
            self.nodes[prev as usize].next = next;
        }
        if next == NIL {
            self.nodes[p as usize].last = prev;
        } else {
            self.nodes[next as usize].prev = prev;
        }
        let n = &mut self.nodes[c as usize];
        n.parent = NIL;
        n.prev = NIL;
        n.next = NIL;
    }

    /// the nodes of the subtree at i in document order (i first)
    pub(crate) fn subtree(&self, i: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut st = vec![i];
        while let Some(x) = st.pop() {
            out.push(x);
            let mut k = self.nodes[x as usize].last;
            while k != NIL {
                st.push(k);
                k = self.nodes[k as usize].prev;
            }
        }
        out
    }

    // free the subtree at i (detached first)
    fn free_tree(&mut self, i: u32) {
        self.detach(i);
        for x in self.subtree(i) {
            if self.nodes[x as usize].kind == K_ELEM {
                self.id_drop(x);
            }
            let n = &mut self.nodes[x as usize];
            // (the slot's next node gets the next generation: old handles stay stale)
            let g = n.serial.wrapping_add(1).max(1);
            *n = Node { serial: 0, kind: K_TEXT, parent: NIL, first: NIL, last: NIL, prev: NIL, next: NIL, name: String::new(), attrs: Vec::new(), blk: NIL };
            self.free.push((x, g));
        }
    }

    // the derived block of every node of the subtree at i goes (it moved or left)
    fn unblock(&mut self, i: u32) {
        for x in self.subtree(i) {
            self.nodes[x as usize].blk = NIL;
        }
    }

    /// i is in the document (the root above it)
    pub(crate) fn attached(&self, i: u32) -> bool {
        let mut x = i;
        loop {
            let n = &self.nodes[x as usize];
            if n.kind == K_ROOT {
                return true;
            }
            if n.parent == NIL {
                return false;
            }
            x = n.parent;
        }
    }

    /// i is a or below it
    fn within(&self, i: u32, a: u32) -> bool {
        let mut x = i;
        while x != NIL {
            if x == a {
                return true;
            }
            x = self.nodes[x as usize].parent;
        }
        false
    }

    /// a change: the content (own: false) or the attributes (own: true) of element i
    pub(crate) fn touch(&mut self, i: u32, own: bool) {
        self.changed = true;
        if let Some(t) = self.touched.as_mut() {
            if !t.contains(&(i, own)) {
                t.push((i, own));
            }
            if t.len() > TOUCHED_MAX {
                self.touched = None;
            }
        }
    }

    /// all the text below i
    pub(crate) fn text_of(&self, i: u32) -> String {
        let mut s = String::new();
        for x in self.subtree(i) {
            let n = &self.nodes[x as usize];
            if n.kind == K_TEXT {
                s.push_str(&n.name);
            }
        }
        s
    }

    /// the block element whose one paragraph a change of element e touches (own: e's own
    /// attributes changed) when that paragraph can be read again by itself: its content is
    /// formatted text (no links, images, fields, breaks, blocks, lists, raw text), it lies
    /// outside tables, pre, buttons, lists of choices, labels and links, not too deep for the
    /// reader's caps -> None: every block again
    pub(crate) fn leaf_of(&self, e: u32, own: bool) -> Option<u32> {
        let mut l = e;
        while self.nodes[l as usize].blk == NIL {
            l = self.nodes[l as usize].parent;
            if l == NIL {
                return None;
            }
        }
        if l == e && own {
            return None;
        }
        let mut depth = 0;
        let mut x = self.nodes[l as usize].parent;
        while x != NIL {
            let n = &self.nodes[x as usize];
            if n.kind == K_ELEM
                && matches!(
                    n.name.as_str(),
                    "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" | "caption" | "pre" | "button" | "select" | "option" | "optgroup"
                        | "textarea" | "label" | "a" | "script" | "style" | "head" | "title" | "template" | "noscript" | "svg" | "xmp" | "listing"
                        | "plaintext"
                )
            {
                return None;
            }
            depth += 1;
            x = n.parent;
        }
        // the content: depth below the caps of the reader (NEST_MAX 64 formats)
        let mut st = vec![(l, depth)];
        while let Some((x, dd)) = st.pop() {
            if dd > 48 {
                return None;
            }
            let mut k = self.nodes[x as usize].first;
            while k != NIL {
                let n = &self.nodes[k as usize];
                if n.kind == K_ELEM {
                    if !leaf_inside(&n.name) {
                        return None;
                    }
                    st.push((k, dd + 1));
                }
                k = n.next;
            }
        }
        Some(l)
    }

    /// i is hidden: the hidden attribute
    pub(crate) fn hidden_attr(&self, i: u32) -> bool {
        self.nodes[i as usize].attrs.iter().any(|a| a.0 == "hidden")
    }

    /// the class attribute of i holds c
    pub(crate) fn has_class(&self, i: u32, c: &str) -> bool {
        self.attr(i, "class").is_some_and(|v| v.split_ascii_whitespace().any(|w| w == c))
    }

    // the HTML of the children of i
    fn inner_html(&self, i: u32, out: &mut String) {
        // (node, exit?)
        let mut st: Vec<(u32, bool)> = Vec::new();
        let mut k = self.nodes[i as usize].last;
        while k != NIL {
            st.push((k, false));
            k = self.nodes[k as usize].prev;
        }
        let rawp = |d: &Dom, x: u32| {
            let p = d.nodes[x as usize].parent;
            p != NIL && matches!(d.nodes[p as usize].name.as_str(), "script" | "style" | "template" | "noscript" | "svg" | "xmp")
        };
        while let Some((x, exit)) = st.pop() {
            let n = &self.nodes[x as usize];
            if exit {
                out.push_str("</");
                out.push_str(&n.name);
                out.push('>');
                continue;
            }
            if n.kind == K_TEXT {
                if rawp(self, x) {
                    out.push_str(&n.name);
                } else {
                    esc_html(&n.name, false, out);
                }
                continue;
            }
            out.push('<');
            out.push_str(&n.name);
            for (k, v) in &n.attrs {
                out.push(' ');
                out.push_str(k);
                if !v.is_empty() {
                    out.push_str("=\"");
                    esc_html(v, true, out);
                    out.push('"');
                }
            }
            out.push('>');
            if void(&n.name) {
                continue;
            }
            st.push((x, true));
            let mut k = n.last;
            while k != NIL {
                st.push((k, false));
                k = self.nodes[k as usize].prev;
            }
        }
    }
}

fn esc_html(s: &str, attr: bool, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\u{A0}' => out.push_str("&nbsp;"),
            _ => out.push(c),
        }
    }
}

// ------------------------------------------------------------------ building the tree

struct Builder<'a> {
    d: &'a mut Dom,
    // the open elements and their part (C_*); st[0] is where the HTML goes (never closed)
    st: Vec<(u32, u8)>,
}

impl Builder<'_> {
    fn top(&self) -> (u32, u8) {
        self.st[self.st.len() - 1]
    }

    // the nearest open element `want` takes before one `stop` takes -> its place in the stack
    fn find(&self, want: impl Fn(&Dom, u32, u8) -> bool, stop: impl Fn(&Dom, u32, u8) -> bool) -> Option<usize> {
        for k in (1..self.st.len()).rev() {
            let (n, c) = self.st[k];
            if want(self.d, n, c) {
                return Some(k);
            }
            if stop(self.d, n, c) {
                return None;
            }
        }
        None
    }

    // text into the open element (in <head> text that is more than blanks ends it)
    fn text(&mut self, t: &str) -> bool {
        if t.is_empty() {
            return true;
        }
        if self.st.len() > 1 && self.top().1 == C_HEAD && t.chars().any(|c| !c.is_whitespace()) {
            self.st.pop();
        }
        let p = self.top().0;
        self.text_into(p, t)
    }

    fn text_into(&mut self, p: u32, t: &str) -> bool {
        let last = self.d.nodes[p as usize].last;
        if last != NIL && self.d.nodes[last as usize].kind == K_TEXT {
            self.d.nodes[last as usize].name.push_str(t);
            return true;
        }
        let Some(x) = self.d.alloc(K_TEXT, String::from(t), Vec::new()) else { return false };
        self.d.append(p, x);
        true
    }

    // a start tag -> the element (None: the cap of nodes) and whether it reads raw text
    fn open(&mut self, name: &str, at: Vec<(String, String)>, selfclose: bool) -> Option<(u32, bool)> {
        let c = code(name);
        let boundary = |_: &Dom, _: u32, c: u8| matches!(c, C_TABLE | C_TCELL | C_BUTTON | C_SELECT);
        // the head ends at an element of the body
        if self.st.len() > 1 && self.top().1 == C_HEAD && !head_tag(name) {
            self.st.pop();
        }
        if c == C_LI || c == C_DDT {
            let k = self.find(
                |_, _, e| e == c,
                |d, n, e| matches!(e, C_LIST | C_DL | C_TABLE | C_TCELL | C_BUTTON | C_SELECT) || (e == C_BLOCK && !matches!(d.nodes[n as usize].name.as_str(), "div" | "address")),
            );
            if let Some(k) = k {
                self.st.truncate(k);
            }
        }
        if closes_p(name) {
            if let Some(k) = self.find(|_, _, e| e == C_P, boundary) {
                self.st.truncate(k);
            }
        }
        let tc = self.top().1;
        if self.st.len() > 1 && ((c == C_HEADING && tc == C_HEADING) || (matches!(c, C_OPTION | C_OPTGROUP) && tc == C_OPTION)) {
            self.st.pop();
        }
        if c == C_OPTGROUP && self.st.len() > 1 && self.top().1 == C_OPTGROUP {
            self.st.pop();
        }
        let table_part = match c {
            C_TR => Some(self.find(|_, _, e| e == C_TR, |_, _, e| matches!(e, C_TABLE | C_TSECT))),
            C_TCELL => Some(self.find(|_, _, e| e == C_TCELL, |_, _, e| matches!(e, C_TR | C_TABLE | C_TSECT))),
            C_TSECT => Some(self.find(|_, _, e| e == C_TSECT, |_, _, e| e == C_TABLE)),
            _ => None,
        };
        if let Some(Some(k)) = table_part {
            self.st.truncate(k);
        }
        let x = self.d.alloc(K_ELEM, String::from(name), at)?;
        let p = self.top().0;
        self.d.append(p, x);
        if void(name) || selfclose {
            return Some((x, false));
        }
        if self.st.len() < DEPTH_MAX {
            self.st.push((x, c));
        }
        Some((x, raw(name)))
    }

    // an end tag
    fn close(&mut self, name: &str) {
        if void(name) || name == "body" || name == "html" {
            return;
        }
        let c = code(name);
        let boundary = |_: &Dom, _: u32, e: u8| matches!(e, C_TABLE | C_TCELL | C_BUTTON | C_SELECT);
        if c == C_P && self.find(|_, _, e| e == C_P, boundary).is_none() {
            // a stray </p>: an empty paragraph
            if let Some(x) = self.d.alloc(K_ELEM, String::from("p"), Vec::new()) {
                let p = self.top().0;
                self.d.append(p, x);
            }
            return;
        }
        let k = self.find(
            |d, n, _| d.nodes[n as usize].name == name,
            |_, _, e| {
                if c == C_INLINE {
                    // (an inline element's end tag does not close a block)
                    e != C_INLINE
                } else {
                    match e {
                        C_TCELL => !matches!(c, C_TR | C_TSECT | C_TABLE),
                        C_TABLE | C_BUTTON | C_SELECT => true,
                        _ => false,
                    }
                }
            },
        );
        if let Some(k) = k {
            self.st.truncate(k);
        }
    }
}

// the HTML s into node `parent` of d (its children follow the ones it has)
fn build(d: &mut Dom, parent: u32, s: &str) {
    let mut b = Builder { d, st: vec![(parent, C_BLOCK)] };
    let mut rest = s;
    // a raw element just opened: its text up to its end tag
    let mut raw_of: Option<u32> = None;
    loop {
        if let Some(x) = raw_of.take() {
            let tag = b.d.nodes[x as usize].name.clone();
            let close = format!("</{}", tag);
            let (content, after) = match find_ci(rest, &close) {
                Some(i) => (&rest[..i], &rest[i..]),
                None => (rest, ""),
            };
            let ok = if tag == "textarea" || tag == "title" { b.text_into(x, &decode(content)) } else { content.is_empty() || b.text_into(x, content) };
            if !ok {
                b.d.truncated = true;
                return;
            }
            rest = after;
        }
        if rest.is_empty() {
            return;
        }
        let Some(lt) = rest.find('<') else {
            if !b.text(&decode(rest)) {
                b.d.truncated = true;
            }
            return;
        };
        if lt > 0 && !b.text(&decode(&rest[..lt])) {
            b.d.truncated = true;
            return;
        }
        rest = &rest[lt..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |j| &rest[j + 3..]);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            rest = rest.find('>').map_or("", |j| &rest[j + 1..]);
            continue;
        }
        let Some(gt) = tag_end(rest) else {
            if !b.text(&decode(rest)) {
                b.d.truncated = true;
            }
            return;
        };
        let inner = &rest[1..gt];
        rest = &rest[gt + 1..];
        if let Some(name) = inner.strip_prefix('/') {
            let name = name.trim().split_whitespace().next().unwrap_or("").to_ascii_lowercase();
            b.close(&name);
            continue;
        }
        let name_end = inner.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(inner.len());
        let name = inner[..name_end].to_ascii_lowercase();
        if name.is_empty() || !name.as_bytes()[0].is_ascii_alphabetic() {
            if !b.text(&format!("<{}>", decode(inner))) {
                b.d.truncated = true;
                return;
            }
            continue;
        }
        let selfclose = inner.trim_end().ends_with('/');
        let at = attrs(&inner[name_end..]);
        match b.open(&name, at, selfclose) {
            None => {
                b.d.truncated = true;
                return;
            }
            Some((x, true)) => raw_of = Some(x),
            Some(_) => {}
        }
    }
}

// ------------------------------------------------------------------ selectors

// a compound: a tag ("" any), an id, classes, attributes (name, operator, value)
#[derive(Clone, Debug, Default)]
pub(crate) struct Compound {
    tag: String,
    id: String,
    classes: Vec<String>,
    attrs: Vec<(String, u8, String)>,
}

/// a selector: its compounds from the subject leftwards; child[i]: compound i + 1 is the parent
/// (else an ancestor) of compound i's element
#[derive(Clone, Debug, Default)]
pub(crate) struct Sel {
    parts: Vec<Compound>,
    child: Vec<bool>,
}

fn ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || c as u32 > 127
}

// an identifier at s[i..] -> (it, the byte after it)
fn ident(s: &str, i: usize) -> Option<(&str, usize)> {
    let n = s[i..].char_indices().find(|&(_, c)| !ident_char(c)).map_or(s.len() - i, |(k, _)| k);
    (n > 0).then(|| (&s[i..i + n], i + n))
}

/// a selector list -> None: something it does not support (it matches nothing)
pub(crate) fn parse_sels(s: &str) -> Option<Vec<Sel>> {
    if s.len() > SEL_BYTES {
        return None;
    }
    let mut out = Vec::new();
    for part in split_list(s)? {
        if out.len() >= SEL_LIST {
            return None;
        }
        out.push(parse_sel(part.trim())?);
    }
    (!out.is_empty()).then_some(out)
}

// the selectors of a list (commas outside brackets and quotes)
fn split_list(s: &str) -> Option<Vec<&str>> {
    let mut out = Vec::new();
    let (mut start, mut q, mut br) = (0, 0u8, false);
    for (i, b) in s.bytes().enumerate() {
        if q != 0 {
            if b == q {
                q = 0;
            }
            continue;
        }
        match b {
            b'"' | b'\'' => q = b,
            b'[' => br = true,
            b']' => br = false,
            b',' if !br => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if q != 0 || br {
        return None;
    }
    out.push(&s[start..]);
    Some(out)
}

fn parse_sel(s: &str) -> Option<Sel> {
    let b = s.as_bytes();
    let mut parts: Vec<Compound> = Vec::new();
    let mut child: Vec<bool> = Vec::new();
    let mut i = 0;
    // the combinator before the next compound: 0 none yet, 1 descendant, 2 child
    let mut comb = 0u8;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            if comb == 0 && !parts.is_empty() {
                comb = 1;
            }
            i += 1;
            continue;
        }
        if c == b'>' {
            if parts.is_empty() || comb == 2 {
                return None;
            }
            comb = 2;
            i += 1;
            continue;
        }
        if !parts.is_empty() && comb == 0 {
            return None;
        }
        if parts.len() >= SEL_PARTS {
            return None;
        }
        let (cp, e) = parse_compound(s, i)?;
        if !parts.is_empty() {
            child.push(comb == 2);
        }
        parts.push(cp);
        comb = 0;
        i = e;
    }
    if parts.is_empty() || comb == 2 {
        return None;
    }
    parts.reverse();
    child.reverse();
    Some(Sel { parts, child })
}

// a compound at s[i..] -> (it, the byte after it)
fn parse_compound(s: &str, mut i: usize) -> Option<(Compound, usize)> {
    let b = s.as_bytes();
    let mut c = Compound::default();
    let start = i;
    if b[i] == b'*' {
        i += 1;
    } else if let Some((t, e)) = ident(s, i) {
        c.tag = t.to_ascii_lowercase();
        i = e;
    }
    while i < b.len() {
        match b[i] {
            b'#' => {
                let (t, e) = ident(s, i + 1)?;
                if !c.id.is_empty() && c.id != t {
                    // (two ids: nothing matches; kept as a class that never occurs)
                    c.classes.push(String::from(" "));
                }
                c.id = String::from(t);
                i = e;
            }
            b'.' => {
                let (t, e) = ident(s, i + 1)?;
                c.classes.push(String::from(t));
                i = e;
            }
            b'[' => {
                let close = s[i..].find(']')? + i;
                let inner = s[i + 1..close].trim();
                let ops = ["~=", "|=", "^=", "$=", "*=", "="];
                let mut found = None;
                for (k, op) in ops.iter().enumerate() {
                    if let Some(p) = inner.find(op) {
                        found = Some((p, k as u8 + 1, op.len()));
                        break;
                    }
                }
                let (name, op, val) = match found {
                    Some((p, k, l)) => {
                        let v = inner[p + l..].trim();
                        let v = if v.len() >= 2 && (v.starts_with('"') && v.ends_with('"') || v.starts_with('\'') && v.ends_with('\'')) { &v[1..v.len() - 1] } else { v };
                        (inner[..p].trim(), k, String::from(v))
                    }
                    None => (inner, 0, String::new()),
                };
                if !name_ok(name) || !name.chars().all(ident_char) {
                    return None;
                }
                c.attrs.push((name.to_ascii_lowercase(), op, val));
                i = close + 1;
            }
            b' ' | b'\t' | b'\n' | b'\r' | b'\x0C' | b'>' => break,
            _ => return None,
        }
    }
    (i > start).then_some((c, i))
}

// element i takes compound c
fn takes(d: &Dom, i: u32, c: &Compound) -> bool {
    let n = &d.nodes[i as usize];
    if n.kind != K_ELEM || (!c.tag.is_empty() && c.tag != n.name) {
        return false;
    }
    if !c.id.is_empty() && d.attr(i, "id") != Some(c.id.as_str()) {
        return false;
    }
    if !c.classes.iter().all(|k| d.has_class(i, k)) {
        return false;
    }
    c.attrs.iter().all(|(k, op, v)| {
        let Some(a) = d.attr(i, k) else { return false };
        match op {
            0 => true,
            1 => a.split_ascii_whitespace().any(|w| w == v),
            2 => a == v || (a.starts_with(v.as_str()) && a.as_bytes().get(v.len()) == Some(&b'-')),
            3 => !v.is_empty() && a.starts_with(v.as_str()),
            4 => !v.is_empty() && a.ends_with(v.as_str()),
            5 => !v.is_empty() && a.contains(v.as_str()),
            _ => a == v,
        }
    })
}

/// element i matches selector s (budget: compounds compared at most; 0 left: no match)
pub(crate) fn sel_match(d: &Dom, i: u32, s: &Sel, budget: &mut usize) -> bool {
    let n = s.parts.len();
    if n == 0 {
        return false;
    }
    // the run of child combinators from compound k on -> its last compound
    let run = |k: usize| -> usize {
        let mut e = k;
        while e + 1 < n && s.child[e] {
            e += 1;
        }
        e
    };
    // compounds k ..= e taken by x, its parent ... -> the element of compound e
    let chain = |x: u32, k: usize, e: usize, budget: &mut usize| -> Option<u32> {
        let mut y = x;
        for j in k..=e {
            if j > k {
                y = d.nodes[y as usize].parent;
            }
            if y == NIL || *budget == 0 {
                return None;
            }
            *budget -= 1;
            if !takes(d, y, &s.parts[j]) {
                return None;
            }
        }
        Some(y)
    };
    // the first run at i itself
    let e = run(0);
    let Some(mut at) = chain(i, 0, e, budget) else { return false };
    let mut k = e + 1;
    // each further run: at the nearest ancestor that takes it
    while k < n {
        let e = run(k);
        let mut x = d.nodes[at as usize].parent;
        let mut found = None;
        while x != NIL && *budget > 0 {
            if let Some(y) = chain(x, k, e, budget) {
                found = Some(y);
                break;
            }
            x = d.nodes[x as usize].parent;
        }
        match found {
            Some(y) => at = y,
            None => return false,
        }
        k = e + 1;
    }
    true
}

// the elements below `top` (the root: the document) a list matches, in document order
fn select_below(d: &Dom, top: u32, sels: &[Sel], first: bool) -> Vec<u32> {
    let mut out = Vec::new();
    let mut budget = d.nodes.len().saturating_mul(64).saturating_add(1 << 16);
    let all = d.subtree(top);
    for &x in all.iter().skip(1) {
        if d.nodes[x as usize].kind != K_ELEM {
            continue;
        }
        if sels.iter().any(|s| sel_match(d, x, s, &mut budget)) {
            out.push(x);
            if first {
                break;
            }
        }
        if budget == 0 {
            break;
        }
    }
    out
}

// ------------------------------------------------------------------ the API of MdDoc

impl MdDoc {
    fn node(&self, n: MdNode) -> Option<u32> {
        self.dom.get(n)
    }

    fn elem(&self, n: MdNode) -> Option<u32> {
        self.node(n).filter(|&i| self.dom.nodes[i as usize].kind != K_TEXT)
    }

    /// the document's root (MdNode::NONE for a document from Markdown): its children are the
    /// top level of the HTML
    pub fn root(&self) -> MdNode {
        self.dom.handle(0)
    }

    /// the first element in the document with this id
    pub fn by_id(&self, id: &str) -> Option<MdNode> {
        let v = self.dom.ids.get(id)?;
        let live: Vec<u32> = v.iter().copied().filter(|&i| self.dom.attached(i)).collect();
        match live.len() {
            0 => None,
            1 => Some(self.dom.handle(live[0])),
            // (several: the first in document order)
            _ => self.dom.subtree(0).into_iter().find(|x| live.contains(x)).map(|i| self.dom.handle(i)),
        }
    }

    /// the elements of the document a selector list matches, in document order (see md_dom.rs;
    /// a selector it does not support matches nothing)
    pub fn select(&self, sel: &str) -> Vec<MdNode> {
        self.select_in(self.root(), sel)
    }

    /// the first element select(sel) would give
    pub fn select_first(&self, sel: &str) -> Option<MdNode> {
        let (Some(top), Some(s)) = (self.node(self.root()), parse_sels(sel)) else { return None };
        select_below(&self.dom, top, &s, true).first().map(|&i| self.dom.handle(i))
    }

    /// the elements below n a selector list matches, in document order
    pub fn select_in(&self, n: MdNode, sel: &str) -> Vec<MdNode> {
        let (Some(top), Some(s)) = (self.node(n), parse_sels(sel)) else { return Vec::new() };
        select_below(&self.dom, top, &s, false).into_iter().map(|i| self.dom.handle(i)).collect()
    }

    /// element n matches a selector list
    pub fn matches(&self, n: MdNode, sel: &str) -> bool {
        let (Some(i), Some(s)) = (self.elem(n), parse_sels(sel)) else { return false };
        let mut budget = 1 << 16;
        s.iter().any(|x| sel_match(&self.dom, i, x, &mut budget))
    }

    /// n's parent (None: the root, a node out of the document, a node that is gone)
    pub fn parent(&self, n: MdNode) -> Option<MdNode> {
        let i = self.node(n)?;
        let p = self.dom.nodes[i as usize].parent;
        (p != NIL).then(|| self.dom.handle(p))
    }

    /// n's children (elements and texts) in order
    pub fn children(&self, n: MdNode) -> Vec<MdNode> {
        let mut out = Vec::new();
        if let Some(i) = self.node(n) {
            let mut k = self.dom.nodes[i as usize].first;
            while k != NIL {
                out.push(self.dom.handle(k));
                k = self.dom.nodes[k as usize].next;
            }
        }
        out
    }

    /// the node after n under the same parent
    pub fn next_sibling(&self, n: MdNode) -> Option<MdNode> {
        let i = self.node(n)?;
        let k = self.dom.nodes[i as usize].next;
        (k != NIL).then(|| self.dom.handle(k))
    }

    /// the node before n under the same parent
    pub fn prev_sibling(&self, n: MdNode) -> Option<MdNode> {
        let i = self.node(n)?;
        let k = self.dom.nodes[i as usize].prev;
        (k != NIL).then(|| self.dom.handle(k))
    }

    /// an element's tag in lower case ("" for a text, "#document" for the root)
    pub fn tag(&self, n: MdNode) -> &str {
        match self.node(n) {
            Some(i) if self.dom.nodes[i as usize].kind != K_TEXT => &self.dom.nodes[i as usize].name,
            _ => "",
        }
    }

    /// n is a text
    pub fn is_text(&self, n: MdNode) -> bool {
        self.node(n).is_some_and(|i| self.dom.nodes[i as usize].kind == K_TEXT)
    }

    /// n exists (it was not freed)
    pub fn alive(&self, n: MdNode) -> bool {
        self.node(n).is_some()
    }

    /// n is in the document
    pub fn attached(&self, n: MdNode) -> bool {
        self.node(n).is_some_and(|i| self.dom.attached(i))
    }

    /// an attribute of element n (names in lower case)
    pub fn attr(&self, n: MdNode, name: &str) -> Option<&str> {
        let i = self.elem(n)?;
        self.dom.attr(i, &name.to_ascii_lowercase())
    }

    /// the attributes of element n in order (names in lower case, values decoded)
    pub fn attrs(&self, n: MdNode) -> &[(String, String)] {
        match self.elem(n) {
            Some(i) => &self.dom.nodes[i as usize].attrs,
            None => &[],
        }
    }

    /// element n has class c
    pub fn has_class(&self, n: MdNode, c: &str) -> bool {
        self.elem(n).is_some_and(|i| self.dom.has_class(i, c))
    }

    /// all the text of n (a text: its own; an element: that of its descendants, as textContent)
    pub fn text_of(&self, n: MdNode) -> String {
        self.node(n).map_or(String::new(), |i| self.dom.text_of(i))
    }

    /// the HTML of n's children
    pub fn inner_html(&self, n: MdNode) -> String {
        let mut s = String::new();
        if let Some(i) = self.elem(n) {
            self.dom.inner_html(i, &mut s);
        }
        s
    }

    /// the tree changed since the blocks were derived (update() derives them)
    pub fn changed(&self) -> bool {
        self.dom.changed
    }

    // ---- changes

    /// set n's text: a text's own; an element's children (freed) become one text
    pub fn set_text(&mut self, n: MdNode, s: &str) -> bool {
        let Some(i) = self.node(n) else { return false };
        if self.dom.nodes[i as usize].kind == K_TEXT {
            if self.dom.nodes[i as usize].name != s {
                self.dom.nodes[i as usize].name = String::from(s);
                let p = self.dom.nodes[i as usize].parent;
                if p != NIL {
                    self.dom.touch(p, false);
                }
            }
            return true;
        }
        // (one text child with this text already: nothing changes)
        let f = self.dom.nodes[i as usize].first;
        if f != NIL && f == self.dom.nodes[i as usize].last && self.dom.nodes[f as usize].kind == K_TEXT && self.dom.nodes[f as usize].name == s {
            return true;
        }
        let mut k = self.dom.nodes[i as usize].first;
        while k != NIL {
            let next = self.dom.nodes[k as usize].next;
            self.dom.free_tree(k);
            k = next;
        }
        if !s.is_empty() {
            match self.dom.alloc(K_TEXT, String::from(s), Vec::new()) {
                Some(t) => self.dom.append(i, t),
                None => {
                    self.dom.touch(i, false);
                    return false;
                }
            }
        }
        self.dom.touch(i, false);
        true
    }

    /// set an attribute of element n (the name in lower case; "id" updates the index)
    pub fn set_attr(&mut self, n: MdNode, name: &str, value: &str) -> bool {
        let Some(i) = self.elem(n) else { return false };
        if self.dom.nodes[i as usize].kind == K_ROOT || !name_ok(name) {
            return false;
        }
        let name = name.to_ascii_lowercase();
        if self.dom.attr(i, &name) == Some(value) {
            return true;
        }
        if name == "id" {
            self.dom.id_drop(i);
        }
        let at = &mut self.dom.nodes[i as usize].attrs;
        match at.iter_mut().find(|a| a.0 == name) {
            Some(a) => a.1 = String::from(value),
            None => at.push((name.clone(), String::from(value))),
        }
        if name == "id" {
            self.dom.id_add(i);
        }
        self.dom.touch(i, true);
        true
    }

    /// remove an attribute of element n -> it had one
    pub fn remove_attr(&mut self, n: MdNode, name: &str) -> bool {
        let Some(i) = self.elem(n) else { return false };
        let name = name.to_ascii_lowercase();
        if self.dom.attr(i, &name).is_none() {
            return false;
        }
        if name == "id" {
            self.dom.id_drop(i);
        }
        self.dom.nodes[i as usize].attrs.retain(|a| a.0 != name);
        self.dom.touch(i, true);
        true
    }

    /// add class c to element n
    pub fn add_class(&mut self, n: MdNode, c: &str) -> bool {
        let Some(i) = self.elem(n) else { return false };
        if c.is_empty() || c.contains(|x: char| x.is_ascii_whitespace()) {
            return false;
        }
        if self.dom.has_class(i, c) {
            return true;
        }
        let v = match self.dom.attr(i, "class") {
            Some(v) if !v.trim().is_empty() => format!("{} {}", v.trim(), c),
            _ => String::from(c),
        };
        self.set_attr(n, "class", &v)
    }

    /// remove class c from element n -> it had it
    pub fn remove_class(&mut self, n: MdNode, c: &str) -> bool {
        let Some(i) = self.elem(n) else { return false };
        if !self.dom.has_class(i, c) {
            return false;
        }
        let v: Vec<&str> = self.dom.attr(i, "class").unwrap_or("").split_ascii_whitespace().filter(|w| *w != c).collect();
        let v = v.join(" ");
        self.set_attr(n, "class", &v)
    }

    /// class c on element n off when it has it, else on -> on
    pub fn toggle_class(&mut self, n: MdNode, c: &str) -> bool {
        if self.has_class(n, c) {
            self.remove_class(n, c);
            false
        } else {
            self.add_class(n, c)
        }
    }

    /// a new element (not in the document: insert puts it there) -> MdNode::NONE: a Markdown
    /// document, a tag that is none, the cap of nodes
    pub fn create_element(&mut self, tag: &str) -> MdNode {
        if !self.dom.is_html() || !name_ok(tag) || !tag.as_bytes()[0].is_ascii_alphabetic() {
            return MdNode::NONE;
        }
        match self.dom.alloc(K_ELEM, tag.to_ascii_lowercase(), Vec::new()) {
            Some(i) => self.dom.handle(i),
            None => MdNode::NONE,
        }
    }

    /// a new text (not in the document)
    pub fn create_text(&mut self, s: &str) -> MdNode {
        if !self.dom.is_html() {
            return MdNode::NONE;
        }
        match self.dom.alloc(K_TEXT, String::from(s), Vec::new()) {
            Some(i) => self.dom.handle(i),
            None => MdNode::NONE,
        }
    }

    /// put n under element `parent` before its child `before` (None: at the end); a node with a
    /// parent moves -> false: a node that is gone, the root, n above parent, before no child of
    /// parent
    pub fn insert(&mut self, parent: MdNode, before: Option<MdNode>, n: MdNode) -> bool {
        let (Some(p), Some(c)) = (self.elem(parent), self.node(n)) else { return false };
        if self.dom.nodes[c as usize].kind == K_ROOT || self.dom.within(p, c) {
            return false;
        }
        let b = match before {
            Some(b) => match self.node(b) {
                Some(b) if self.dom.nodes[b as usize].parent == p && b != c => b,
                _ => return false,
            },
            None => NIL,
        };
        let old = self.dom.nodes[c as usize].parent;
        if old != NIL {
            self.dom.detach(c);
            self.dom.touch(old, false);
        }
        self.dom.unblock(c);
        self.dom.insert_before(p, c, b);
        self.dom.touch(p, false);
        true
    }

    /// take n out of the document (it stays: insert may put it back; free drops it)
    pub fn remove(&mut self, n: MdNode) -> bool {
        let Some(c) = self.node(n) else { return false };
        let p = self.dom.nodes[c as usize].parent;
        if p == NIL {
            return false;
        }
        self.dom.detach(c);
        self.dom.unblock(c);
        self.dom.touch(p, false);
        true
    }

    /// drop n and its descendants (out of the document first): their handles refer to nothing
    pub fn free(&mut self, n: MdNode) -> bool {
        let Some(c) = self.node(n) else { return false };
        if self.dom.nodes[c as usize].kind == K_ROOT {
            return false;
        }
        let p = self.dom.nodes[c as usize].parent;
        self.dom.free_tree(c);
        if p != NIL {
            self.dom.touch(p, false);
        }
        true
    }

    /// element n's children (freed) become the nodes of an HTML text (raw text elements: the
    /// text)
    pub fn set_inner_html(&mut self, n: MdNode, html: &str) -> bool {
        let Some(i) = self.elem(n) else { return false };
        if raw(&self.dom.nodes[i as usize].name) {
            return self.set_text(n, html);
        }
        let mut k = self.dom.nodes[i as usize].first;
        while k != NIL {
            let next = self.dom.nodes[k as usize].next;
            self.dom.free_tree(k);
            k = next;
        }
        let t = self.dom.truncated;
        build(&mut self.dom, i, html);
        let cut = self.dom.truncated && !t;
        self.dom.truncated = t;
        self.dom.touch(i, false);
        !cut
    }
}
