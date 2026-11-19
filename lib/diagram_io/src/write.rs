// write -- diagram_core's model as the XML parts of a VSDX package (pack.rs drives it)
//
//   document(doc) / pages(doc, ..) / masters(doc, ..) / contents(doc, ..)   one part's XML
//   check(text) -> Err(line)   a written part must be well-formed XML: the model's raw items
//                              (unknown elements, foreign text marks, trigger contents) are
//                              written as it holds them and could break it; the reader's own
//                              XML reader parses the part once to make sure
//
// The XML is Visio's: the declaration <?xml version='1.0' encoding='utf-8' ?> with CR LF,
// attributes in single quotes, no white space between elements.  Text and attribute values are
// escaped so that the reader gets them back unchanged (CR as &#13;, a tab or line end in an
// attribute as a character reference); a character XML cannot carry at all (U+0001 ...) becomes
// U+FFFD.  Numbers are written in the shortest form that reads back to the same f64: plain
// decimals, and below 1e-4 or from 1e16 on with an exponent (6.123233995736766E-17), as Visio
// writes them.
//
// The known children of an element come in the order of [MS-VSDX] and of Visio's own files: in a
// sheet Cell, Trigger, Section; in a section its cells, triggers, rows; in a shape then Text,
// Data1, Data2, Data3, ForeignData, Shapes; in a master PageSheet, Icon, Rel; in a page
// PageSheet, Rel.  The reader keeps unknown items with the number of known children before
// them, so they go back to the same place: an attribute into the start tag of its element (of
// its cell, its Shapes or its Connects list), an element or text after as many known children
// as it had (AfterChild), inside its cell (InCell), inside the Shapes or Connects list after as
// many entries (InShapes, InConnects).  An item whose place lies beyond the last child goes
// last; one whose place its element does not have, or an attribute that would repeat one the
// writer writes itself, is left out.  Groups at any depth go through a stack of their own: no
// recursion.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use diagram_core::{
    Cell, Connect, Document, Extra, ExtraItem, Extras, ForeignData, ForeignPayload, Formula, Place, Row, RowKey, Section,
    ShapeData, ShapeKind, ShapeTree, Sheet, StyleRefs, Sym, Text, TextItem, Trigger, Value,
};

use crate::xml::{Event, Reader, NS_REL, NS_REL_STRICT, NS_VISIO};

/// the XML declaration of the Visio parts
pub(crate) const DECL: &str = "<?xml version='1.0' encoding='utf-8' ?>\r\n";

/// a value V: the shortest form that reads back to the same number; an exponent below 1e-4
/// and from 1e16 on (Visio writes 6.123233995736766E-17); -0 as 0
pub(crate) fn number(x: f64) -> String {
    if x == 0.0 {
        return String::from("0");
    }
    let a = x.abs();
    if a.is_finite() && (a < 1e-4 || a >= 1e16) {
        alloc::format!("{x:E}")
    } else {
        alloc::format!("{x}")
    }
}

// characters a part must escape (or replace): markup, the quote, CR, white space that the reader
// would turn into a space in an attribute, and what XML 1.0 cannot carry
fn plain(s: &str, attr: bool) -> bool {
    !s.bytes().any(|b| matches!(b, b'&' | b'<' | b'>' | b'\'' | b'\r') || (b < 0x20 && (attr || b != b'\t' && b != b'\n')))
        && !s.contains(['\u{fffe}', '\u{ffff}'])
}

pub(crate) fn escape(out: &mut String, s: &str, attr: bool) {
    if plain(s, attr) {
        out.push_str(s);
        return;
    }
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' if attr => out.push_str("&apos;"),
            '\r' => out.push_str("&#13;"),
            '\n' if attr => out.push_str("&#10;"),
            '\t' if attr => out.push_str("&#9;"),
            '\t' | '\n' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
}

/// whether a written part is well-formed XML; Err: the line of the first problem
pub(crate) fn check(text: &str) -> Result<(), u32> {
    let mut r = Reader::new(text, usize::MAX, usize::MAX);
    loop {
        match r.next() {
            Ok(Event::Eof) => return Ok(()),
            Ok(_) => {}
            Err((_, pos)) => {
                let b = text.as_bytes();
                return Err(1 + b[..pos.min(b.len())].iter().filter(|&&c| c == b'\n').count() as u32);
            }
        }
    }
}

// ---- unknown items by place

// the items of one kind of place, sorted by their number (stable): taken from the front
struct Bucket<'e> {
    items: Vec<(u32, &'e Extra)>,
    pos: usize,
}

impl<'e> Bucket<'e> {
    fn new(mut items: Vec<(u32, &'e Extra)>) -> Self {
        items.sort_by_key(|(n, _)| *n);
        Bucket { items, pos: 0 }
    }

    // the items numbered up to n not taken yet
    fn upto(&mut self, n: u32) -> &[(u32, &'e Extra)] {
        let start = self.pos;
        while self.pos < self.items.len() && self.items[self.pos].0 <= n {
            self.pos += 1;
        }
        &self.items[start..self.pos]
    }

    // the items numbered n (those below n not taken yet are dropped)
    fn at(&mut self, n: u32) -> &[(u32, &'e Extra)] {
        while self.pos < self.items.len() && self.items[self.pos].0 < n {
            self.pos += 1;
        }
        let start = self.pos;
        while self.pos < self.items.len() && self.items[self.pos].0 == n {
            self.pos += 1;
        }
        &self.items[start..self.pos]
    }

    // everything not taken yet
    fn rest(&mut self) -> &[(u32, &'e Extra)] {
        let start = self.pos;
        self.pos = self.items.len();
        &self.items[start..]
    }
}

// the unknown items of one element, by place
struct Placed<'e> {
    attrs: Vec<&'e Extra>,
    after: Bucket<'e>,
    cell: Bucket<'e>,
    shapes: Bucket<'e>,
    connects: Bucket<'e>,
}

impl<'e> Placed<'e> {
    fn new(lists: &[&'e Extras]) -> Self {
        let (mut attrs, mut after, mut cell, mut shapes, mut connects) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for e in lists.iter().flat_map(|l| l.items()) {
            let attr = matches!(e.item, ExtraItem::Attribute { .. });
            match e.place {
                Place::Element if attr => attrs.push(e),
                Place::AfterChild(n) if !attr => after.push((n, e)),
                Place::InCell(n) => cell.push((n, e)),
                Place::InShapes(n) => shapes.push((if attr { 0 } else { n }, e)),
                Place::InConnects(n) => connects.push((if attr { 0 } else { n }, e)),
                _ => {}
            }
        }
        Placed { attrs, after: Bucket::new(after), cell: Bucket::new(cell), shapes: Bucket::new(shapes), connects: Bucket::new(connects) }
    }
}

// ---- the writer

/// one part being written
pub(crate) struct Xml<'d> {
    pub(crate) s: String,
    doc: &'d Document,
    // the attribute names of the open start tag (only kept while extras are written into it)
    names: Vec<&'d str>,
}

// a shape whose Shapes list is open: what comes after the list
struct Open<'e> {
    after: Bucket<'e>,
    known: u32,
}

// a Shapes list being written
struct List<'d> {
    slots: &'d [u32],
    next: usize,
    items: Bucket<'d>,
    // the group whose list it is (None: the page's or master's own list)
    group: Option<Open<'d>>,
}

impl<'d> Xml<'d> {
    pub(crate) fn new(doc: &'d Document) -> Self {
        Xml { s: String::from(DECL), doc, names: Vec::new() }
    }

    fn str(&self, s: Sym) -> &'d str {
        self.doc.str(s)
    }

    fn open(&mut self, name: &str) {
        self.names.clear();
        self.s.push('<');
        self.s.push_str(name);
    }

    fn attr(&mut self, name: &str, value: &str) {
        self.s.push(' ');
        self.s.push_str(name);
        self.s.push_str("='");
        escape(&mut self.s, value, true);
        self.s.push('\'');
    }

    fn uint(&mut self, name: &str, v: u32) {
        let _ = write!(self.s, " {name}='{v}'");
    }

    fn int(&mut self, name: &str, v: i32) {
        let _ = write!(self.s, " {name}='{v}'");
    }

    fn flag(&mut self, name: &str, v: bool) {
        self.attr(name, if v { "1" } else { "0" });
    }

    fn opt_sym(&mut self, name: &str, v: Option<Sym>) {
        if let Some(v) = v {
            let t = self.str(v);
            self.attr(name, t);
        }
    }

    fn opt_flag(&mut self, name: &str, v: Option<bool>) {
        if let Some(v) = v {
            self.flag(name, v);
        }
    }

    fn opt_uint(&mut self, name: &str, v: Option<u32>) {
        if let Some(v) = v {
            self.uint(name, v);
        }
    }

    fn styles(&mut self, st: StyleRefs) {
        self.opt_uint("LineStyle", st.line);
        self.opt_uint("FillStyle", st.fill);
        self.opt_uint("TextStyle", st.text);
    }

    // attributes kept as pairs (header.attrs ...), but none named as one in `known` or as one
    // written into this start tag before (open() starts the list)
    fn pairs(&mut self, list: &[(Sym, Sym)], known: &[&str]) {
        for &(n, v) in list {
            let (n, v) = (self.str(n), self.str(v));
            if n.is_empty() || known.contains(&n) || self.names.contains(&n) {
                continue;
            }
            self.names.push(n);
            self.attr(n, v);
        }
    }

    // unknown attributes, but none named as one in `known` (the writer's own) or written into
    // this start tag before
    fn extra_attrs<'x>(&mut self, items: impl Iterator<Item = &'x Extra>, known: &[&str]) {
        for e in items {
            if let ExtraItem::Attribute { name, value } = e.item {
                let (n, v) = (self.str(name), self.str(value));
                if n.is_empty() || known.contains(&n) || self.names.contains(&n) {
                    continue;
                }
                self.names.push(n);
                self.attr(n, v);
            }
        }
    }

    // '>' of a start tag; the position after it (finish() makes <a/> of an element left empty)
    fn gt(&mut self) -> usize {
        self.s.push('>');
        self.s.len()
    }

    fn finish(&mut self, name: &str, mark: usize) {
        if self.s.len() == mark {
            self.s.pop();
            self.s.push_str("/>");
        } else {
            self.s.push_str("</");
            self.s.push_str(name);
            self.s.push('>');
        }
    }

    fn text(&mut self, t: &str) {
        escape(&mut self.s, t, false);
    }

    // an unknown element or text (attributes are written into start tags elsewhere)
    fn item(&mut self, e: &Extra) {
        match e.item {
            ExtraItem::Element(x) => {
                let raw = self.str(x);
                self.s.push_str(raw);
            }
            ExtraItem::Text(t) => {
                let t = self.str(t);
                self.text(t);
            }
            ExtraItem::Attribute { .. } => {}
        }
    }

    fn items(&mut self, list: &[(u32, &Extra)]) {
        for (_, e) in list {
            self.item(e);
        }
    }

    // the unknown children before known child number `n`
    fn before(&mut self, after: &mut Bucket, n: u32) {
        let list = after.upto(n);
        for (_, e) in list {
            self.item(e);
        }
    }

    // the root element's start tag: its attributes as kept, the Visio namespace as the default
    // one, r bound to the relationships namespace, xml:space='preserve' (each added in front
    // when missing)
    fn root(&mut self, name: &str, attrs: &[(&'d str, &'d str)]) {
        self.open(name);
        let has = |n: &str| attrs.iter().any(|(a, _)| *a == n);
        if !has("xmlns") {
            self.attr("xmlns", NS_VISIO);
        }
        if !has("xmlns:r") {
            self.attr("xmlns:r", NS_REL);
        }
        if !has("xml:space") {
            self.attr("xml:space", "preserve");
        }
        let mut seen: Vec<&str> = Vec::new();
        for &(n, v) in attrs {
            if n.is_empty() || seen.contains(&n) {
                continue;
            }
            seen.push(n);
            let v = match n {
                "xmlns" => NS_VISIO,
                "xmlns:r" if v != NS_REL && v != NS_REL_STRICT => NS_REL,
                _ => v,
            };
            self.attr(n, v);
        }
        self.s.push('>');
    }

    // the attributes of an Extras list placed on the element itself, as pairs of text
    fn root_pairs(&self, extras: &'d Extras) -> Vec<(&'d str, &'d str)> {
        extras
            .items()
            .iter()
            .filter(|e| e.place == Place::Element)
            .filter_map(|e| match e.item {
                ExtraItem::Attribute { name, value } => Some((self.doc.str(name), self.doc.str(value))),
                _ => None,
            })
            .collect()
    }

    // ---- sheets

    fn cell(&mut self, c: &Cell, items: &[(u32, &Extra)]) {
        self.open("Cell");
        let n = self.str(c.name);
        self.attr("N", n);
        match c.value {
            Value::Absent => {}
            Value::Num(x) => {
                let t = number(x);
                self.attr("V", &t);
            }
            Value::Text(t) => {
                let t = self.str(t);
                self.attr("V", t);
            }
        }
        if let Some(u) = c.unit {
            let code = match u.code() {
                Some(code) => code,
                None => match u {
                    diagram_core::Unit::Other(s) => self.str(s),
                    _ => "",
                },
            };
            if !code.is_empty() {
                self.attr("U", code);
            }
        }
        match c.formula {
            Formula::Absent => {}
            Formula::Inherited => self.attr("F", "Inh"),
            Formula::NoFormula => self.attr("F", "No Formula"),
            Formula::Expr(f) => {
                let f = self.str(f);
                self.attr("F", f);
            }
        }
        self.opt_sym("E", c.error);
        self.extra_attrs(items.iter().map(|(_, e)| *e), &["N", "V", "U", "F", "E"]);
        let mark = self.gt();
        self.items(items);
        self.finish("Cell", mark);
    }

    fn trigger(&mut self, t: &Trigger) {
        self.open("Trigger");
        let n = self.str(t.name);
        self.attr("N", n);
        let mark = self.gt();
        let inner = self.str(t.inner);
        self.s.push_str(inner);
        self.finish("Trigger", mark);
    }

    // the cells (with what stood inside them) and triggers of a sheet, a section or a row
    fn cells(&mut self, cells: &[Cell], triggers: &[Trigger], p: &mut Placed, known: &mut u32) {
        for (i, c) in cells.iter().enumerate() {
            self.before(&mut p.after, *known);
            let items = p.cell.at(i as u32);
            self.cell(c, items);
            *known += 1;
        }
        for t in triggers {
            self.before(&mut p.after, *known);
            self.trigger(t);
            *known += 1;
        }
    }

    fn row(&mut self, r: &Row, position: u32) {
        let mut p = Placed::new(&[&r.extras]);
        self.open("Row");
        self.opt_sym("T", r.kind);
        let known: &[&str] = match r.key {
            RowKey::Index(i) => {
                if r.ix_written || i != position {
                    self.uint("IX", i);
                }
                &["T", "IX", "N", "LocalName", "Del"]
            }
            RowKey::Named(n) => {
                let n = self.str(n);
                self.attr("N", n);
                &["T", "N", "LocalName", "Del"]
            }
        };
        self.opt_sym("LocalName", r.local_name);
        if r.del {
            self.attr("Del", "1");
        }
        let attrs = core::mem::take(&mut p.attrs);
        self.extra_attrs(attrs.into_iter(), known);
        let mark = self.gt();
        let mut n = 0;
        self.cells(&r.cells, &r.triggers, &mut p, &mut n);
        let rest = p.after.rest();
        for (_, e) in rest {
            self.item(e);
        }
        self.finish("Row", mark);
    }

    fn section(&mut self, s: &Section, geometry: &mut u32) {
        let mut p = Placed::new(&[&s.extras]);
        let name = self.str(s.name);
        let natural = if name == "Geometry" {
            *geometry = geometry.saturating_add(1);
            *geometry - 1
        } else {
            0
        };
        self.open("Section");
        self.attr("N", name);
        if s.ix_written || s.index != natural {
            self.uint("IX", s.index);
        }
        if s.del {
            self.attr("Del", "1");
        }
        let attrs = core::mem::take(&mut p.attrs);
        self.extra_attrs(attrs.into_iter(), &["N", "IX", "Del"]);
        let mark = self.gt();
        let mut known = 0;
        self.cells(&s.cells, &s.triggers, &mut p, &mut known);
        for (i, r) in s.rows.iter().enumerate() {
            self.before(&mut p.after, known);
            self.row(r, i as u32);
            known += 1;
        }
        let rest = p.after.rest();
        for (_, e) in rest {
            self.item(e);
        }
        self.finish("Section", mark);
    }

    // the cells, triggers and sections of a sheet; `known` counts the children written
    fn sheet(&mut self, sheet: &Sheet, p: &mut Placed, known: &mut u32) {
        self.cells(&sheet.cells, &sheet.triggers, p, known);
        let mut geometry = 0;
        for s in &sheet.sections {
            self.before(&mut p.after, *known);
            self.section(s, &mut geometry);
            *known += 1;
        }
    }

    // a sheet element of its own (PageSheet, DocumentSheet, StyleSheet): `head` writes its
    // attributes; the unknown items are in `lists`
    fn sheet_element(&mut self, name: &str, sheet: &Sheet, lists: &[&'d Extras], head: impl FnOnce(&mut Self), known_attrs: &[&str]) {
        let mut p = Placed::new(lists);
        self.open(name);
        head(self);
        let attrs = core::mem::take(&mut p.attrs);
        self.extra_attrs(attrs.into_iter(), known_attrs);
        let mark = self.gt();
        let mut known = 0;
        self.sheet(sheet, &mut p, &mut known);
        let rest = p.after.rest();
        for (_, e) in rest {
            self.item(e);
        }
        self.finish(name, mark);
    }

    // ---- shapes

    fn shape_text(&mut self, t: &Text) {
        self.open("Text");
        let p = Placed::new(&[&t.extras]);
        self.extra_attrs(p.attrs.into_iter(), &[]);
        let mark = self.gt();
        for item in &t.items {
            match item {
                TextItem::Chars(c) => self.text(c),
                TextItem::Cp(ix) => {
                    let _ = write!(self.s, "<cp IX='{ix}'/>");
                }
                TextItem::Pp(ix) => {
                    let _ = write!(self.s, "<pp IX='{ix}'/>");
                }
                TextItem::Tp(ix) => {
                    let _ = write!(self.s, "<tp IX='{ix}'/>");
                }
                TextItem::Field(ix) => {
                    let _ = write!(self.s, "<fld IX='{ix}'/>");
                }
                TextItem::Other(raw) => {
                    let raw = self.str(*raw);
                    self.s.push_str(raw);
                }
            }
        }
        self.finish("Text", mark);
    }

    fn foreign(&mut self, f: &ForeignData, pictures: &BTreeMap<Sym, String>) {
        let mut p = Placed::new(&[&f.extras]);
        self.open("ForeignData");
        self.pairs(&f.attrs, &[]);
        let mark = self.gt();
        match &f.payload {
            ForeignPayload::None => {}
            ForeignPayload::Inline(b64) => {
                self.before(&mut p.after, 0);
                self.text(b64);
            }
            ForeignPayload::Part(part) => {
                // a picture whose part is gone is written without its data
                if let Some(id) = pictures.get(part) {
                    self.before(&mut p.after, 0);
                    self.s.push_str("<Rel r:id='");
                    escape(&mut self.s, id, true);
                    self.s.push_str("'/>");
                }
            }
        }
        let rest = p.after.rest();
        for (_, e) in rest {
            self.item(e);
        }
        self.finish("ForeignData", mark);
    }

    // a shape's start tag and everything before its Shapes list; the unknown children still to
    // come and how many known children were written (None: the slot holds no shape)
    fn shape_head(&mut self, tree: &'d ShapeTree, slot: u32, pictures: &BTreeMap<Sym, String>) -> Option<(Placed<'d>, u32)> {
        let d: &'d ShapeData = tree.get(slot)?;
        let id = tree.id(slot).unwrap_or(diagram_core::NO_ID);
        let mut p = Placed::new(&[&d.extras, &d.sheet.extras]);
        let h = &d.header;
        self.open("Shape");
        self.uint("ID", id);
        self.opt_sym("NameU", h.name_u);
        self.opt_flag("IsCustomNameU", h.is_custom_name_u);
        self.opt_sym("Name", h.name);
        self.opt_flag("IsCustomName", h.is_custom_name);
        match h.kind {
            ShapeKind::Other(t) => {
                let t = self.str(t);
                self.attr("Type", t);
            }
            k => {
                if let Some(t) = k.as_str() {
                    self.attr("Type", t);
                }
            }
        }
        self.opt_uint("OriginalID", h.original_id);
        self.opt_uint("Master", h.master);
        self.opt_uint("MasterShape", h.master_shape);
        self.styles(h.styles);
        self.opt_sym("UniqueID", h.unique_id);
        if h.del {
            self.attr("Del", "1");
        }
        let attrs = core::mem::take(&mut p.attrs);
        self.extra_attrs(
            attrs.into_iter(),
            &[
                "ID", "NameU", "IsCustomNameU", "Name", "IsCustomName", "Type", "OriginalID", "Master", "MasterShape", "LineStyle",
                "FillStyle", "TextStyle", "UniqueID", "Del",
            ],
        );
        self.s.push('>');
        let mut known = 0;
        self.sheet(&d.sheet, &mut p, &mut known);
        if let Some(t) = &d.text {
            self.before(&mut p.after, known);
            self.shape_text(t);
            known += 1;
        }
        for (i, data) in d.data.iter().enumerate() {
            if let Some(v) = data {
                self.before(&mut p.after, known);
                let name = ["Data1", "Data2", "Data3"][i];
                let _ = write!(self.s, "<{name}>");
                let v = self.str(*v);
                self.text(v);
                let _ = write!(self.s, "</{name}>");
                known += 1;
            }
        }
        if let Some(f) = &d.foreign {
            self.before(&mut p.after, known);
            self.foreign(f, pictures);
            known += 1;
        }
        Some((p, known))
    }

    // a Shapes start tag with its own attributes
    fn shapes_open(&mut self, items: &mut Bucket) {
        self.open("Shapes");
        let list: Vec<&Extra> = items.items[items.pos..]
            .iter()
            .filter(|(_, e)| matches!(e.item, ExtraItem::Attribute { .. }))
            .map(|(_, e)| *e)
            .collect();
        self.extra_attrs(list.into_iter(), &[]);
        self.s.push('>');
    }

    // the elements and text of a list's items numbered up to n (its attributes are in its tag)
    fn list_items(&mut self, items: &mut Bucket, n: Option<u32>) {
        let list = match n {
            Some(n) => items.upto(n),
            None => items.rest(),
        };
        for (_, e) in list {
            if !matches!(e.item, ExtraItem::Attribute { .. }) {
                self.item(e);
            }
        }
    }

    // a page's or master's shapes: the outer Shapes list with every group's list inside it,
    // without recursion
    fn shape_tree(&mut self, tree: &'d ShapeTree, outer: Bucket<'d>, pictures: &BTreeMap<Sym, String>) {
        let mut outer = outer;
        self.shapes_open(&mut outer);
        let mut stack: Vec<List<'d>> = alloc::vec![List { slots: tree.top(), next: 0, items: outer, group: None }];
        loop {
            let Some(top) = stack.last_mut() else { break };
            if top.next < top.slots.len() {
                let slot = top.slots[top.next];
                let i = top.next as u32;
                top.next += 1;
                self.list_items(&mut top.items, Some(i));
                let Some((mut p, known)) = self.shape_head(tree, slot, pictures) else { continue };
                let kids = tree.children(slot);
                if !kids.is_empty() || !p.shapes.items.is_empty() {
                    self.before(&mut p.after, known);
                    let mut list = core::mem::replace(&mut p.shapes, Bucket::new(Vec::new()));
                    self.shapes_open(&mut list);
                    stack.push(List { slots: kids, next: 0, items: list, group: Some(Open { after: p.after, known }) });
                } else {
                    let rest = p.after.rest();
                    for (_, e) in rest {
                        self.item(e);
                    }
                    self.s.push_str("</Shape>");
                }
            } else {
                let Some(mut done) = stack.pop() else { break };
                self.list_items(&mut done.items, None);
                self.s.push_str("</Shapes>");
                match done.group {
                    Some(mut g) => {
                        g.known += 1;
                        let rest = g.after.rest();
                        for (_, e) in rest {
                            self.item(e);
                        }
                        self.s.push_str("</Shape>");
                    }
                    None => break,
                }
            }
        }
    }

    fn connect(&mut self, c: &Connect) {
        let mut p = Placed::new(&[&c.extras]);
        self.open("Connect");
        self.uint("FromSheet", c.from_sheet);
        self.opt_sym("FromCell", c.from_cell);
        if let Some(v) = c.from_part {
            self.int("FromPart", v);
        }
        self.uint("ToSheet", c.to_sheet);
        self.opt_sym("ToCell", c.to_cell);
        if let Some(v) = c.to_part {
            self.int("ToPart", v);
        }
        let attrs = core::mem::take(&mut p.attrs);
        self.extra_attrs(attrs.into_iter(), &["FromSheet", "FromCell", "FromPart", "ToSheet", "ToCell", "ToPart"]);
        let mark = self.gt();
        let rest = p.after.rest();
        for (_, e) in rest {
            self.item(e);
        }
        self.finish("Connect", mark);
    }
}

// ---- the parts

/// document.xml: DocumentSettings, Colors, FaceNames, StyleSheets, DocumentSheet, then the
/// unknown children (EventList, HeaderFooter ...) where they stood
pub(crate) fn document(doc: &Document) -> String {
    let mut x = Xml::new(doc);
    let attrs: Vec<(&str, &str)> = doc.attrs.iter().map(|&(n, v)| (doc.str(n), doc.str(v))).collect();
    x.root("VisioDocument", &attrs);
    let mut p = Placed::new(&[&doc.extras]);
    let mut known = 0;
    // DocumentSettings
    x.before(&mut p.after, known);
    let st = &doc.settings;
    x.open("DocumentSettings");
    x.opt_uint("TopPage", st.top_page);
    x.opt_uint("DefaultTextStyle", st.default_text_style);
    x.opt_uint("DefaultLineStyle", st.default_line_style);
    x.opt_uint("DefaultFillStyle", st.default_fill_style);
    x.opt_uint("DefaultGuideStyle", st.default_guide_style);
    x.pairs(&st.attrs, &["TopPage", "DefaultTextStyle", "DefaultLineStyle", "DefaultFillStyle", "DefaultGuideStyle"]);
    let mark = x.gt();
    let mut sp = Placed::new(&[&st.extras]);
    for (i, &(name, text)) in st.items.iter().enumerate() {
        x.before(&mut sp.after, i as u32);
        let (n, t) = (doc.str(name), doc.str(text));
        if n.is_empty() {
            continue;
        }
        x.open(n);
        let m = x.gt();
        x.text(t);
        x.finish(n, m);
    }
    let rest = sp.after.rest();
    x.items(rest);
    x.finish("DocumentSettings", mark);
    known += 1;
    // Colors and FaceNames only with entries (MS-VSDX: at least one each) or kept items
    if !doc.colors.is_empty() || !doc.colors_extras.is_empty() {
        colors(&mut x, doc, &mut p, &mut known);
    }
    if !doc.faces.is_empty() || !doc.faces_extras.is_empty() {
        faces(&mut x, doc, &mut p, &mut known);
    }
    // StyleSheets
    x.before(&mut p.after, known);
    let mut sp = Placed::new(&[&doc.styles_extras]);
    x.open("StyleSheets");
    let attrs = core::mem::take(&mut sp.attrs);
    x.extra_attrs(attrs.into_iter(), &[]);
    let mark = x.gt();
    for (i, r) in doc.styles().enumerate() {
        x.before(&mut sp.after, i as u32);
        let Some(st) = doc.style(r) else { continue };
        let h = &st.header;
        x.sheet_element(
            "StyleSheet",
            &st.sheet,
            &[&st.extras, &st.sheet.extras],
            |x| {
                x.uint("ID", h.id);
                x.opt_sym("NameU", h.name_u);
                x.opt_flag("IsCustomNameU", h.is_custom_name_u);
                x.opt_sym("Name", h.name);
                x.opt_flag("IsCustomName", h.is_custom_name);
                x.styles(h.based_on);
                x.pairs(&h.attrs, &["ID", "NameU", "IsCustomNameU", "Name", "IsCustomName", "LineStyle", "FillStyle", "TextStyle"]);
            },
            &["ID", "NameU", "IsCustomNameU", "Name", "IsCustomName", "LineStyle", "FillStyle", "TextStyle"],
        );
    }
    let rest = sp.after.rest();
    x.items(rest);
    x.finish("StyleSheets", mark);
    known += 1;
    // DocumentSheet
    x.before(&mut p.after, known);
    let ds = doc.doc_sheet();
    x.sheet_element(
        "DocumentSheet",
        &ds.sheet,
        &[&ds.sheet.extras],
        |x| {
            x.pairs(&ds.attrs, &["LineStyle", "FillStyle", "TextStyle"]);
            x.styles(ds.styles);
        },
        &["LineStyle", "FillStyle", "TextStyle"],
    );
    let rest = p.after.rest();
    x.items(rest);
    x.s.push_str("</VisioDocument>\r\n");
    x.s
}

// the Colors element
fn colors<'d>(x: &mut Xml<'d>, doc: &'d Document, p: &mut Placed<'d>, known: &mut u32) {
    x.before(&mut p.after, *known);
    let mut cp = Placed::new(&[&doc.colors_extras]);
    x.open("Colors");
    let attrs = core::mem::take(&mut cp.attrs);
    x.extra_attrs(attrs.into_iter(), &[]);
    let mark = x.gt();
    for (i, c) in doc.colors.iter().enumerate() {
        x.before(&mut cp.after, i as u32);
        x.open("ColorEntry");
        x.uint("IX", c.ix);
        let rgb = doc.str(c.rgb);
        x.attr("RGB", rgb);
        x.s.push_str("/>");
    }
    let rest = cp.after.rest();
    x.items(rest);
    x.finish("Colors", mark);
    *known += 1;
}

// the FaceNames element
fn faces<'d>(x: &mut Xml<'d>, doc: &'d Document, p: &mut Placed<'d>, known: &mut u32) {
    x.before(&mut p.after, *known);
    let mut fp = Placed::new(&[&doc.faces_extras]);
    x.open("FaceNames");
    let attrs = core::mem::take(&mut fp.attrs);
    x.extra_attrs(attrs.into_iter(), &[]);
    let mark = x.gt();
    for (i, f) in doc.faces.iter().enumerate() {
        x.before(&mut fp.after, i as u32);
        x.open("FaceName");
        x.pairs(&f.attrs, &[]);
        x.s.push_str("/>");
    }
    let rest = fp.after.rest();
    x.items(rest);
    x.finish("FaceNames", mark);
    *known += 1;
}

/// pages.xml: every page with its PageSheet and the Rel to its part (`rels[i]`: the r:id of
/// page i's part, None for a page without one)
pub(crate) fn pages(doc: &Document, rels: &[Option<String>]) -> String {
    let mut x = Xml::new(doc);
    let attrs = x.root_pairs(&doc.pages_extras);
    x.root("Pages", &attrs);
    let mut p = Placed::new(&[&doc.pages_extras]);
    p.attrs.clear();
    for (i, r) in doc.pages().enumerate() {
        x.before(&mut p.after, i as u32);
        let Some(pg) = doc.page(r) else { continue };
        let h = &pg.header;
        let mut ep = Placed::new(&[&pg.extras]);
        x.open("Page");
        x.uint("ID", h.id);
        x.opt_sym("NameU", h.name_u);
        x.opt_flag("IsCustomNameU", h.is_custom_name_u);
        x.opt_sym("Name", h.name);
        x.opt_flag("IsCustomName", h.is_custom_name);
        if h.background {
            x.attr("Background", "1");
        }
        x.opt_uint("BackPage", h.back_page);
        let known_attrs = ["ID", "NameU", "IsCustomNameU", "Name", "IsCustomName", "Background", "BackPage"];
        x.pairs(&h.attrs, &known_attrs);
        let attrs = core::mem::take(&mut ep.attrs);
        x.extra_attrs(attrs.into_iter(), &known_attrs);
        x.s.push('>');
        x.before(&mut ep.after, 0);
        x.sheet_element("PageSheet", &pg.sheet, &[&pg.sheet.extras], |x| x.styles(h.sheet_styles), &["LineStyle", "FillStyle", "TextStyle"]);
        if let Some(Some(id)) = rels.get(i) {
            x.before(&mut ep.after, 1);
            x.s.push_str("<Rel r:id='");
            escape(&mut x.s, id, true);
            x.s.push_str("'/>");
        }
        let rest = ep.after.rest();
        x.items(rest);
        x.s.push_str("</Page>");
    }
    let rest = p.after.rest();
    x.items(rest);
    x.s.push_str("</Pages>");
    x.s
}

/// masters.xml: every master with its PageSheet, Icon and the Rel to its part (`rels[i]`)
pub(crate) fn masters(doc: &Document, rels: &[Option<String>]) -> String {
    let mut x = Xml::new(doc);
    let attrs = x.root_pairs(&doc.masters_extras);
    x.root("Masters", &attrs);
    let mut p = Placed::new(&[&doc.masters_extras]);
    p.attrs.clear();
    for (i, r) in doc.masters().enumerate() {
        x.before(&mut p.after, i as u32);
        let Some(m) = doc.master(r) else { continue };
        let h = &m.header;
        let mut ep = Placed::new(&[&m.extras]);
        x.open("Master");
        x.uint("ID", h.id);
        x.opt_sym("NameU", h.name_u);
        x.opt_flag("IsCustomNameU", h.is_custom_name_u);
        x.opt_sym("Name", h.name);
        x.opt_flag("IsCustomName", h.is_custom_name);
        let known_attrs = ["ID", "NameU", "IsCustomNameU", "Name", "IsCustomName", "UniqueID", "BaseID"];
        x.pairs(&h.attrs, &known_attrs);
        x.opt_sym("UniqueID", h.unique_id);
        x.opt_sym("BaseID", h.base_id);
        let attrs = core::mem::take(&mut ep.attrs);
        x.extra_attrs(attrs.into_iter(), &known_attrs);
        x.s.push('>');
        let mut known = 0;
        x.before(&mut ep.after, known);
        x.sheet_element("PageSheet", &m.sheet, &[&m.sheet.extras], |x| x.styles(h.sheet_styles), &["LineStyle", "FillStyle", "TextStyle"]);
        known += 1;
        if let Some(icon) = m.icon {
            x.before(&mut ep.after, known);
            x.s.push_str("<Icon>");
            let t = doc.str(icon);
            x.text(t);
            x.s.push_str("</Icon>");
            known += 1;
        }
        if let Some(Some(id)) = rels.get(i) {
            x.before(&mut ep.after, known);
            x.s.push_str("<Rel r:id='");
            escape(&mut x.s, id, true);
            x.s.push_str("'/>");
        }
        let rest = ep.after.rest();
        x.items(rest);
        x.s.push_str("</Master>");
    }
    let rest = p.after.rest();
    x.items(rest);
    x.s.push_str("</Masters>");
    x.s
}

/// a page or master part (`root`: PageContents or MasterContents): the shapes, the Connect
/// entries, the unknown items; `pictures`: the r:id of each picture part its shapes name
pub(crate) fn contents(doc: &Document, root: &str, tree: &ShapeTree, connects: &[Connect], extras: &Extras, pictures: &BTreeMap<Sym, String>) -> String {
    let mut x = Xml::new(doc);
    let attrs = x.root_pairs(extras);
    x.root(root, &attrs);
    let mut p = Placed::new(&[extras]);
    let mut known = 0;
    if !tree.is_empty() || !p.shapes.items.is_empty() {
        x.before(&mut p.after, known);
        let outer = core::mem::replace(&mut p.shapes, Bucket::new(Vec::new()));
        x.shape_tree(tree, outer, pictures);
        known += 1;
    }
    if !connects.is_empty() || !p.connects.items.is_empty() {
        x.before(&mut p.after, known);
        x.open("Connects");
        let list: Vec<&Extra> = p.connects.items.iter().filter(|(_, e)| matches!(e.item, ExtraItem::Attribute { .. })).map(|(_, e)| *e).collect();
        x.extra_attrs(list.into_iter(), &[]);
        x.s.push('>');
        for (i, c) in connects.iter().enumerate() {
            x.list_items(&mut p.connects, Some(i as u32));
            x.connect(c);
        }
        x.list_items(&mut p.connects, None);
        x.s.push_str("</Connects>");
    }
    let rest = p.after.rest();
    x.items(rest);
    x.s.push_str("</");
    x.s.push_str(root);
    x.s.push('>');
    x.s
}
