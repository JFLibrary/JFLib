// read -- the XML parts of a VSDX package into diagram_core's model (vsdx.rs drives it)
//
//   document(..)                    document.xml: settings, colours, fonts, styles, the
//                                   document sheet; unknown children in doc.extras
//   pages(..) / masters(..)         pages.xml / masters.xml: the elements with their sheets and
//                                   the r:id of their contents
//   page_contents(..) / master_contents(..)   a page or master part: shapes and connects
//
// Shared rules: an attribute the model names becomes a field, every other attribute goes where
// the model keeps "the other attributes" (header.attrs) or, where it has none, into the extras
// of the element (Place::Element); an unknown child element is kept as the file wrote it after
// the known children before it (Place::AfterChild), text outside shape text likewise when it is
// not white space; what stood inside a cell goes to Place::InCell, inside Shapes / Connects to
// Place::InShapes / InConnects.  A missing IX counts the position (ix_written false).  Nothing
// recurses: groups at any depth go through a stack of their own (shapes), bounded by
// Limits::depth through the XML reader.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{
    Cell, ColorEntry, Connect, Extra, ExtraItem, Extras, FaceName, ForeignData, ForeignPayload, Formula, Master,
    MasterHeader, Page, PageHeader, Place, Row, RowKey, SecKey, Section, ShapeData, ShapeHeader, ShapeKind, ShapeTree,
    Sheet, Style, StyleHeader, StyleRefs, Sym, Text, TextItem, Trigger, Unit, Value,
};

use crate::vsdx::{Error, Loader, Warning};
use crate::xml::{self, Attr, Event, Ns, Reader, Start, XmlError};

/// one relationship of a part, resolved
#[derive(Clone, Debug)]
pub(crate) struct RelInfo {
    pub kind: String,
    /// the stored name of an internal target
    pub part: Option<String>,
    pub external: bool,
}

/// the relationships of one part by id
pub(crate) type RelMap = BTreeMap<String, RelInfo>;

/// what the document holds so far (against Limits)
#[derive(Default, Debug)]
pub(crate) struct Counts {
    shapes: usize,
    cells: usize,
    rows: usize,
    sections: usize,
    objects: usize,
    items: usize,
}

/// what to count
#[derive(Copy, Clone)]
pub(crate) enum Count {
    Shape,
    Cell,
    Row,
    Section,
    /// a page, master or style
    Object,
    /// anything else kept: a connect, trigger, text mark, colour, font, setting, relationship,
    /// unknown item
    Item,
}

impl Counts {
    /// one more against the limits
    pub(crate) fn add(&mut self, what: Count, lim: &crate::vsdx::Limits) -> Result<(), Error> {
        let (n, max, name) = match what {
            Count::Shape => (&mut self.shapes, lim.shapes, "shapes"),
            Count::Cell => (&mut self.cells, lim.cells, "cells"),
            Count::Row => (&mut self.rows, lim.rows, "rows"),
            Count::Section => (&mut self.sections, lim.sections, "sections"),
            Count::Object => (&mut self.objects, lim.objects, "objects"),
            Count::Item => (&mut self.items, lim.items, "items"),
        };
        *n += 1;
        if *n > max {
            return Err(Error::Limit(name));
        }
        Ok(())
    }
}

// the reading context of one part
struct Cx<'a, 'l, 'p> {
    ld: &'l mut Loader<'p>,
    r: Reader<'a>,
    part: &'l str,
    rels: &'l RelMap,
}

type R<T> = Result<T, Error>;

fn line_of(src: &str, pos: usize) -> u32 {
    let b = src.as_bytes();
    1 + b[..pos.min(b.len())].iter().filter(|&&c| c == b'\n').count() as u32
}

fn plain(a: &Attr, name: &str) -> bool {
    a.ns == Ns::None && a.local == name
}

// an element's attributes without its namespace declarations (<Rel xmlns:r='..' r:id='..'/> is
// a Rel with r:id alone)
fn own<'b, 'a>(s: &'b Start<'a>) -> Vec<&'b Attr<'a>> {
    s.attrs.iter().filter(|a| a.ns != Ns::Decl).collect()
}

// whether an element read with skip had nothing but white space inside
fn inner_blank(s: &Start, raw: &str) -> bool {
    s.empty || raw.rfind("</").is_some_and(|e| e >= s.end - s.at && xml::is_blank(&raw[s.end - s.at..e]))
}

impl<'a, 'l, 'p> Cx<'a, 'l, 'p> {
    fn xml_err(&self, (error, pos): (XmlError, usize)) -> Error {
        Error::Xml { part: alloc::format!("/{}", self.part), line: line_of(self.r.src(), pos), error }
    }

    fn bad(&self, what: &'static str) -> Error {
        Error::Structure { part: alloc::format!("/{}", self.part), line: line_of(self.r.src(), self.r.pos()), what }
    }

    fn next(&mut self) -> R<Event<'a>> {
        self.r.next().map_err(|e| self.xml_err(e))
    }

    fn skip(&mut self, s: &Start<'a>) -> R<&'a str> {
        self.r.skip(s).map_err(|e| self.xml_err(e))
    }

    fn sym(&mut self, s: &str) -> Sym {
        self.ld.doc.intern(s)
    }

    // the root element of the part, which must be the Visio element `name`
    fn root(&mut self, name: &str) -> R<Start<'a>> {
        match self.next()? {
            Event::Start(s) if s.is(name) => Ok(s),
            _ => Err(self.bad("another root element")),
        }
    }

    // after the root's End: only the end of the text may follow
    fn finish(&mut self) -> R<()> {
        match self.next()? {
            Event::Eof => Ok(()),
            _ => Err(self.bad("content after the root element")),
        }
    }

    fn attr_extra(&mut self, a: &Attr, extras: &mut Extras, place: Place) -> R<()> {
        let item = ExtraItem::Attribute { name: self.sym(a.qname), value: self.sym(&a.value) };
        self.push(extras, place, item)
    }

    // one unknown item, counted
    fn push(&mut self, extras: &mut Extras, place: Place, item: ExtraItem) -> R<()> {
        self.count(Count::Item)?;
        extras.push(Extra { place, item });
        Ok(())
    }

    fn pair(&mut self, a: &Attr) -> (Sym, Sym) {
        (self.sym(a.qname), self.sym(&a.value))
    }

    // an unknown element, kept as written
    fn unknown(&mut self, s: &Start<'a>, extras: &mut Extras, place: Place) -> R<()> {
        let raw = self.skip(s)?;
        let item = ExtraItem::Element(self.sym(raw));
        self.push(extras, place, item)
    }

    // text where the model expects elements: kept unless it is white space
    fn text_extra(&mut self, t: &str, extras: &mut Extras, place: Place) -> R<()> {
        if !xml::is_blank(t) {
            let item = ExtraItem::Text(self.sym(t));
            self.push(extras, place, item)?;
        }
        Ok(())
    }

    fn uint(&self, v: &str, what: &'static str) -> R<u32> {
        if v.is_empty() || v.len() > 10 || !v.bytes().all(|c| c.is_ascii_digit()) {
            return Err(self.bad(what));
        }
        v.parse::<u32>().map_err(|_| self.bad(what))
    }

    fn int(&self, v: &str, what: &'static str) -> R<i32> {
        let digits = v.strip_prefix('-').unwrap_or(v);
        if digits.is_empty() || digits.len() > 10 || !digits.bytes().all(|c| c.is_ascii_digit()) {
            return Err(self.bad(what));
        }
        v.parse::<i32>().map_err(|_| self.bad(what))
    }

    fn flag(&self, v: &str, what: &'static str) -> R<bool> {
        match v {
            "1" | "true" => Ok(true),
            "0" | "false" => Ok(false),
            _ => Err(self.bad(what)),
        }
    }

    // the text of an element without attributes and child elements; Err(raw) otherwise (the
    // element as written, read to its end either way)
    fn simple_text(&mut self, s: &Start<'a>) -> R<Result<String, &'a str>> {
        if !s.attrs.is_empty() {
            return Ok(Err(self.skip(s)?));
        }
        let mut buf = String::new();
        let mut nested = false;
        let mut depth = 0usize;
        loop {
            match self.next()? {
                Event::Text(t) => buf.push_str(&t),
                Event::Start(_) => {
                    nested = true;
                    depth += 1;
                }
                Event::End { end, .. } => {
                    if depth == 0 {
                        if nested {
                            return Ok(Err(&self.r.src()[s.at..end]));
                        }
                        return Ok(Ok(buf));
                    }
                    depth -= 1;
                }
                Event::Eof => return Err(self.bad("an element without end")),
            }
        }
    }

    fn count(&mut self, what: Count) -> R<()> {
        let lim = self.ld.lim;
        self.ld.counts.add(what, lim)
    }

    // ---- sheets

    fn cell(&mut self, s: &Start<'a>, pos: u32, extras: &mut Extras) -> R<Cell> {
        self.count(Count::Cell)?;
        let (mut n, mut v, mut f, mut u, mut e) = (None, None, None, None, None);
        for a in &s.attrs {
            let slot = if a.ns != Ns::None {
                None
            } else {
                match a.local {
                    "N" => Some(&mut n),
                    "V" => Some(&mut v),
                    "F" => Some(&mut f),
                    "U" => Some(&mut u),
                    "E" => Some(&mut e),
                    _ => None,
                }
            };
            match slot {
                Some(x) => *x = Some(a.value.clone()),
                None => self.attr_extra(a, extras, Place::InCell(pos))?,
            }
        }
        let Some(n) = n else { return Err(self.bad("a Cell without N")) };
        let max = self.ld.lim.formula_bytes;
        if f.as_ref().is_some_and(|x| x.len() > max) || v.as_ref().is_some_and(|x| x.len() > max) {
            return Err(Error::Limit("formula"));
        }
        let unit = match u.as_deref() {
            Some(code) => Some(Unit::parse(code).unwrap_or_else(|| Unit::Other(self.ld.doc.intern(code)))),
            None => None,
        };
        let name = self.sym(&n);
        let strings = self.ld.doc.strings_mut();
        let value = v.as_deref().map_or(Value::Absent, |v| Value::parse(v, unit, strings));
        let formula = f.as_deref().map_or(Formula::Absent, |f| Formula::parse(f, strings));
        let error = e.as_deref().map(|e| strings.intern(e));
        let mut cell = Cell { name, value, formula, unit, error, stale: false };
        // (JD19) Visio writes a long string with line breaks as the element's text instead of V
        // (<Cell N='Value' U='STR'>...</Cell>, the report definitions of User.visPropRptN): a
        // Cell without V whose only content is text takes that text as its value
        let mut text: Option<String> = None;
        let mut child = false;
        loop {
            match self.next()? {
                Event::Start(c) => {
                    // mixed content is no value: the text stays where it stood, as before
                    if let Some(t) = text.take() {
                        self.text_extra(&t, extras, Place::InCell(pos))?
                    }
                    child = true;
                    self.unknown(&c, extras, Place::InCell(pos))?
                }
                Event::Text(t) => {
                    if v.is_none() && !child && text.is_none() && !t.trim().is_empty() {
                        if t.len() > max {
                            return Err(Error::Limit("formula"));
                        }
                        text = Some(String::from(&*t));
                    } else {
                        self.text_extra(&t, extras, Place::InCell(pos))?
                    }
                }
                Event::End { .. } => {
                    if let Some(t) = text {
                        let strings = self.ld.doc.strings_mut();
                        cell.value = Value::parse(&t, unit, strings);
                    }
                    return Ok(cell);
                }
                Event::Eof => return Err(self.bad("a Cell without end")),
            }
        }
    }

    // a Trigger with N alone: its children as written; anything else is an unknown element
    fn trigger(&mut self, s: &Start<'a>, triggers: &mut Vec<Trigger>, extras: &mut Extras, place: Place) -> R<()> {
        let name = match own(s).as_slice() {
            [a] if plain(a, "N") => a.value.clone(),
            _ => return self.unknown(s, extras, place),
        };
        let raw = self.skip(s)?;
        let inner = if s.empty { "" } else { raw.rfind("</").map_or("", |e| &raw[s.end - s.at..e]) };
        let t = Trigger { name: self.sym(&name), inner: if inner.is_empty() { Sym::EMPTY } else { self.sym(inner) } };
        self.count(Count::Item)?;
        triggers.push(t);
        Ok(())
    }

    fn row(&mut self, s: &Start<'a>, position: u32) -> R<Row> {
        self.count(Count::Row)?;
        let mut row = Row::new(RowKey::Index(position));
        row.ix_written = false;
        let (mut named, mut ix, mut ix_raw) = (None, None, None);
        for a in &s.attrs {
            match a.local {
                "N" if a.ns == Ns::None => named = Some(self.sym(&a.value)),
                "IX" if a.ns == Ns::None => {
                    ix = Some(self.uint(&a.value, "a Row IX that is no number")?);
                    ix_raw = Some(a.value.clone());
                }
                "T" if a.ns == Ns::None => row.kind = Some(self.sym(&a.value)),
                "Del" if a.ns == Ns::None => row.del = self.flag(&a.value, "a Row Del that is no flag")?,
                "LocalName" if a.ns == Ns::None => row.local_name = Some(self.sym(&a.value)),
                _ => self.attr_extra(a, &mut row.extras, Place::Element)?,
            }
        }
        if let Some(n) = named {
            row.key = RowKey::Named(n);
            if let Some(raw) = ix_raw {
                // both: the name is the key, IX stays as written
                let item = ExtraItem::Attribute { name: self.sym("IX"), value: self.sym(&raw) };
                self.push(&mut row.extras, Place::Element, item)?;
            }
        } else if let Some(i) = ix {
            row.key = RowKey::Index(i);
            row.ix_written = true;
        }
        let mut known = 0u32;
        loop {
            match self.next()? {
                Event::Start(c) if c.is("Cell") => {
                    let pos = row.cells.len() as u32;
                    let cell = self.cell(&c, pos, &mut row.extras)?;
                    row.cells.push(cell);
                    known += 1;
                }
                Event::Start(c) if c.is("Trigger") => {
                    let before = row.triggers.len();
                    self.trigger(&c, &mut row.triggers, &mut row.extras, Place::AfterChild(known))?;
                    if row.triggers.len() > before {
                        known += 1;
                    }
                }
                Event::Start(c) => self.unknown(&c, &mut row.extras, Place::AfterChild(known))?,
                Event::Text(t) => self.text_extra(&t, &mut row.extras, Place::AfterChild(known))?,
                Event::End { .. } => return Ok(row),
                Event::Eof => return Err(self.bad("a Row without end")),
            }
        }
    }

    fn section(&mut self, s: &Start<'a>, geometry: &mut u32) -> R<Section> {
        self.count(Count::Section)?;
        let (mut name, mut ix, mut del) = (None, None, false);
        let mut extras = Extras::new();
        for a in &s.attrs {
            match a.local {
                "N" if a.ns == Ns::None => name = Some(a.value.clone()),
                "IX" if a.ns == Ns::None => ix = Some(self.uint(&a.value, "a Section IX that is no number")?),
                "Del" if a.ns == Ns::None => del = self.flag(&a.value, "a Section Del that is no flag")?,
                _ => self.attr_extra(a, &mut extras, Place::Element)?,
            }
        }
        let Some(name) = name else { return Err(self.bad("a Section without N")) };
        let is_geometry = name == "Geometry";
        let index = match ix {
            Some(i) => i,
            None if is_geometry => *geometry,
            None => 0,
        };
        if is_geometry {
            *geometry = geometry.saturating_add(1);
        }
        let mut sec = Section::new(SecKey { name: self.sym(&name), index });
        sec.ix_written = ix.is_some();
        sec.del = del;
        sec.extras = extras;
        let mut known = 0u32;
        loop {
            match self.next()? {
                Event::Start(c) if c.is("Cell") => {
                    let pos = sec.cells.len() as u32;
                    let cell = self.cell(&c, pos, &mut sec.extras)?;
                    sec.cells.push(cell);
                    known += 1;
                }
                Event::Start(c) if c.is("Row") => {
                    let row = self.row(&c, sec.rows.len() as u32)?;
                    sec.rows.push(row);
                    known += 1;
                }
                Event::Start(c) if c.is("Trigger") => {
                    let before = sec.triggers.len();
                    self.trigger(&c, &mut sec.triggers, &mut sec.extras, Place::AfterChild(known))?;
                    if sec.triggers.len() > before {
                        known += 1;
                    }
                }
                Event::Start(c) => self.unknown(&c, &mut sec.extras, Place::AfterChild(known))?,
                Event::Text(t) => self.text_extra(&t, &mut sec.extras, Place::AfterChild(known))?,
                Event::End { .. } => return Ok(sec),
                Event::Eof => return Err(self.bad("a Section without end")),
            }
        }
    }

    // a Cell, Section or Trigger of a sheet element -> true when it was one of them
    fn sheet_child(&mut self, s: &Start<'a>, sheet: &mut Sheet, extras: &mut Extras, geometry: &mut u32, known: &mut u32) -> R<bool> {
        if s.is("Cell") {
            let pos = sheet.cells.len() as u32;
            let cell = self.cell(s, pos, extras)?;
            sheet.cells.push(cell);
        } else if s.is("Section") {
            let sec = self.section(s, geometry)?;
            sheet.sections.push(sec);
        } else if s.is("Trigger") {
            let before = sheet.triggers.len();
            self.trigger(s, &mut sheet.triggers, extras, Place::AfterChild(*known))?;
            if sheet.triggers.len() == before {
                return Ok(true); // kept as an unknown element
            }
        } else {
            return Ok(false);
        }
        *known += 1;
        Ok(true)
    }

    // a sheet element of its own (PageSheet, DocumentSheet): its style attributes, the rest of
    // its attributes in the sheet's extras or in `attrs`
    fn sheet_element(&mut self, s: &Start<'a>, mut attrs: Option<&mut Vec<(Sym, Sym)>>) -> R<(Sheet, StyleRefs)> {
        let mut sheet = Sheet::new();
        let mut extras = Extras::new();
        let mut styles = StyleRefs::default();
        for a in &s.attrs {
            match a.local {
                "LineStyle" if a.ns == Ns::None => styles.line = Some(self.uint(&a.value, "a LineStyle that is no number")?),
                "FillStyle" if a.ns == Ns::None => styles.fill = Some(self.uint(&a.value, "a FillStyle that is no number")?),
                "TextStyle" if a.ns == Ns::None => styles.text = Some(self.uint(&a.value, "a TextStyle that is no number")?),
                _ => match attrs.as_deref_mut() {
                    Some(v) => {
                        let p = self.pair(a);
                        v.push(p);
                    }
                    None => self.attr_extra(a, &mut extras, Place::Element)?,
                },
            }
        }
        let (mut geometry, mut known) = (0u32, 0u32);
        loop {
            match self.next()? {
                Event::Start(c) => {
                    if !self.sheet_child(&c, &mut sheet, &mut extras, &mut geometry, &mut known)? {
                        self.unknown(&c, &mut extras, Place::AfterChild(known))?;
                    }
                }
                Event::Text(t) => self.text_extra(&t, &mut extras, Place::AfterChild(known))?,
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("a sheet without end")),
            }
        }
        sheet.extras = extras;
        Ok((sheet, styles))
    }

    // ---- shapes

    fn text(&mut self, s: &Start<'a>) -> R<Text> {
        let mut text = Text::default();
        for a in &s.attrs {
            self.attr_extra(a, &mut text.extras, Place::Element)?;
        }
        let max = self.ld.lim.text_bytes;
        let mut size = 0usize;
        let mut buf = String::new();
        loop {
            match self.next()? {
                Event::Text(t) => {
                    size += t.len();
                    if size > max {
                        return Err(Error::Limit("text"));
                    }
                    buf.push_str(&t);
                }
                Event::Start(c) => {
                    if !buf.is_empty() {
                        self.count(Count::Item)?;
                        text.items.push(TextItem::Chars(Box::from(core::mem::take(&mut buf).as_str())));
                    }
                    let mark = match (c.ns, c.local) {
                        (Ns::Visio, "cp" | "pp" | "tp" | "fld") => match own(&c).as_slice() {
                            [a] if plain(a, "IX") => self.uint(&a.value, "an IX that is no number").ok().map(|ix| (c.local, ix)),
                            _ => None,
                        },
                        _ => None,
                    };
                    let raw = self.skip(&c)?;
                    let inner_empty = inner_blank(&c, raw);
                    size += raw.len();
                    if size > max {
                        return Err(Error::Limit("text"));
                    }
                    let item = match mark {
                        Some(("cp", ix)) if inner_empty => TextItem::Cp(ix),
                        Some(("pp", ix)) if inner_empty => TextItem::Pp(ix),
                        Some(("tp", ix)) if inner_empty => TextItem::Tp(ix),
                        Some(("fld", ix)) if inner_empty => TextItem::Field(ix),
                        _ => TextItem::Other(self.sym(raw)),
                    };
                    self.count(Count::Item)?;
                    text.items.push(item);
                }
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("a Text without end")),
            }
        }
        if !buf.is_empty() {
            self.count(Count::Item)?;
            text.items.push(TextItem::Chars(Box::from(buf.as_str())));
        }
        Ok(text)
    }

    fn foreign(&mut self, s: &Start<'a>, shape: u32) -> R<ForeignData> {
        let mut fd = ForeignData::default();
        for a in &s.attrs {
            let p = self.pair(a);
            fd.attrs.push(p);
        }
        let mut inline = String::new();
        let mut known = 0u32;
        loop {
            match self.next()? {
                Event::Start(c) if c.is("Rel") && matches!(own(&c).as_slice(), [a] if a.ns == Ns::Rel && a.local == "id") => {
                    let rid = own(&c)[0].value.clone();
                    let raw = self.skip(&c)?;
                    if !inner_blank(&c, raw) {
                        let item = ExtraItem::Element(self.sym(raw));
                        self.push(&mut fd.extras, Place::AfterChild(known), item)?;
                        continue;
                    }
                    let info = self.rels.get(rid.as_ref()).cloned().ok_or_else(|| self.bad("an r:id that names no relationship"))?;
                    match info.part {
                        Some(p) if !info.external && fd.payload == ForeignPayload::None => {
                            fd.payload = ForeignPayload::Part(self.sym(&alloc::format!("/{p}")));
                            known += 1;
                        }
                        _ => {
                            if info.external {
                                self.ld.warn(Warning::ExternalPicture { part: alloc::format!("/{}", self.part), shape });
                            }
                            let item = ExtraItem::Element(self.sym(raw));
                            self.count(Count::Item)?;
                            fd.extras.push(Extra { place: Place::AfterChild(known), item });
                        }
                    }
                }
                Event::Start(c) => self.unknown(&c, &mut fd.extras, Place::AfterChild(known))?,
                Event::Text(t) => {
                    if inline.len() + t.len() > self.ld.lim.zip.part_bytes {
                        return Err(Error::Limit("text"));
                    }
                    inline.push_str(&t);
                }
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("a ForeignData without end")),
            }
        }
        if !xml::is_blank(&inline) {
            if fd.payload == ForeignPayload::None {
                fd.payload = ForeignPayload::Inline(Box::from(inline.as_str()));
            } else {
                let item = ExtraItem::Text(self.sym(&inline));
                self.count(Count::Item)?;
                fd.extras.push(Extra { place: Place::AfterChild(known), item });
            }
        }
        Ok(fd)
    }

    // a Shape's start tag: its attributes into a ShapeData, its ID
    fn shape_start(&mut self, s: &Start<'a>) -> R<(u32, ShapeData)> {
        self.count(Count::Shape)?;
        let mut data = ShapeData::new(ShapeKind::Unspecified);
        let mut id = None;
        let h: &mut ShapeHeader = &mut data.header;
        for a in &s.attrs {
            if a.ns != Ns::None {
                let item = ExtraItem::Attribute { name: self.ld.doc.intern(a.qname), value: self.ld.doc.intern(&a.value) };
                self.count(Count::Item)?;
                data.extras.push(Extra { place: Place::Element, item });
                continue;
            }
            match a.local {
                "ID" => id = Some(self.uint(&a.value, "a Shape ID that is no number")?),
                "Type" => h.kind = ShapeKind::parse(&a.value, self.ld.doc.strings_mut()),
                "Name" => h.name = Some(self.ld.doc.intern(&a.value)),
                "NameU" => h.name_u = Some(self.ld.doc.intern(&a.value)),
                "IsCustomName" => h.is_custom_name = Some(self.flag(&a.value, "an IsCustomName that is no flag")?),
                "IsCustomNameU" => h.is_custom_name_u = Some(self.flag(&a.value, "an IsCustomNameU that is no flag")?),
                "Master" => h.master = Some(self.uint(&a.value, "a Master that is no number")?),
                "MasterShape" => h.master_shape = Some(self.uint(&a.value, "a MasterShape that is no number")?),
                "LineStyle" => h.styles.line = Some(self.uint(&a.value, "a LineStyle that is no number")?),
                "FillStyle" => h.styles.fill = Some(self.uint(&a.value, "a FillStyle that is no number")?),
                "TextStyle" => h.styles.text = Some(self.uint(&a.value, "a TextStyle that is no number")?),
                "Del" => h.del = self.flag(&a.value, "a Shape Del that is no flag")?,
                "OriginalID" => h.original_id = Some(self.uint(&a.value, "an OriginalID that is no number")?),
                "UniqueID" => h.unique_id = Some(self.ld.doc.intern(&a.value)),
                _ => {
                    let item = ExtraItem::Attribute { name: self.ld.doc.intern(a.qname), value: self.ld.doc.intern(&a.value) };
                    self.count(Count::Item)?;
                    data.extras.push(Extra { place: Place::Element, item });
                }
            }
        }
        let Some(id) = id else { return Err(self.bad("a Shape without ID")) };
        Ok((id, data))
    }

    // a child of a Shape other than its Shapes list: a sheet child, Text, ForeignData, Data1-3
    // or an unknown element (known += 1 for the known ones)
    fn shape_child(&mut self, c: &Start<'a>, f: &mut ShapeFrame) -> R<()> {
        let data = &mut f.data;
        if self.sheet_child(c, &mut data.sheet, &mut data.extras, &mut f.geometry, &mut f.known)? {
            return Ok(());
        }
        if c.is("Text") && data.text.is_none() {
            data.text = Some(Box::new(self.text(c)?));
        } else if c.is("ForeignData") && data.foreign.is_none() {
            data.foreign = Some(Box::new(self.foreign(c, f.id)?));
        } else if let Some(i) = ["Data1", "Data2", "Data3"].iter().position(|n| c.is(n)).filter(|&i| data.data[i].is_none()) {
            match self.simple_text(c)? {
                Ok(t) => data.data[i] = Some(self.sym(&t)),
                Err(raw) => {
                    let item = ExtraItem::Element(self.sym(raw));
                    self.count(Count::Item)?;
                    data.extras.push(Extra { place: Place::AfterChild(f.known), item });
                    return Ok(());
                }
            }
        } else {
            self.unknown(c, &mut data.extras, Place::AfterChild(f.known))?;
            return Ok(());
        }
        f.known += 1;
        Ok(())
    }

    // a Shapes list into `list` (pre-order, parents first), groups at any depth without
    // recursion: a stack of the open shapes and lists; unknown items of the outer list in
    // `extras`, of a group's list in the group's extras
    fn shapes(&mut self, s: &Start<'a>, list: &mut Vec<Pending>, extras: &mut Extras) -> R<()> {
        for a in &s.attrs {
            self.attr_extra(a, extras, Place::InShapes(0))?;
        }
        // the open elements: lists (the shapes read in them) and shapes, alternating
        let mut stack: Vec<Frame> = alloc::vec![Frame::List(0)];
        loop {
            let ev = self.next()?;
            let depth = stack.len();
            match stack.last_mut() {
                Some(Frame::List(n)) => {
                    let place = Place::InShapes(*n);
                    match ev {
                        Event::Start(c) if c.is("Shape") => {
                            *n += 1;
                            // the group whose list this is: the frame below (none for the outer list)
                            let parent = match depth.checked_sub(2).and_then(|i| stack.get(i)) {
                                Some(Frame::Shape(g)) => Some(g.idx),
                                _ => None,
                            };
                            let (id, data) = self.shape_start(&c)?;
                            let idx = list.len();
                            list.push(Pending { parent, id, data: ShapeData::default() });
                            stack.push(Frame::Shape(Box::new(ShapeFrame { idx, id, data, geometry: 0, known: 0, listed: false })));
                        }
                        Event::Start(c) => {
                            let ex = list_extras(&mut stack, extras);
                            self.unknown(&c, ex, place)?;
                        }
                        Event::Text(t) => {
                            let ex = list_extras(&mut stack, extras);
                            self.text_extra(&t, ex, place)?;
                        }
                        Event::End { .. } => {
                            stack.pop();
                            match stack.last_mut() {
                                Some(Frame::Shape(g)) => g.known += 1,
                                _ => return Ok(()),
                            }
                        }
                        Event::Eof => return Err(self.bad("a Shapes list without end")),
                    }
                }
                Some(Frame::Shape(f)) => match ev {
                    Event::Start(c) if c.is("Shapes") && !f.listed => {
                        f.listed = true;
                        for a in &c.attrs {
                            self.attr_extra(a, &mut f.data.extras, Place::InShapes(0))?;
                        }
                        stack.push(Frame::List(0));
                    }
                    Event::Start(c) => self.shape_child(&c, f)?,
                    Event::Text(t) => self.text_extra(&t, &mut f.data.extras, Place::AfterChild(f.known))?,
                    Event::End { .. } => {
                        if let Some(Frame::Shape(f)) = stack.pop() {
                            let f = *f;
                            list[f.idx].data = f.data;
                        }
                    }
                    Event::Eof => return Err(self.bad("a Shape without end")),
                },
                None => return Err(self.bad("a Shapes list without end")),
            }
        }
    }

    fn connect(&mut self, s: &Start<'a>) -> R<Connect> {
        self.count(Count::Item)?;
        let mut c = Connect { from_sheet: 0, from_cell: None, from_part: None, to_sheet: 0, to_cell: None, to_part: None, extras: Extras::new() };
        let (mut from, mut to) = (None, None);
        for a in &s.attrs {
            match a.local {
                "FromSheet" if a.ns == Ns::None => from = Some(self.uint(&a.value, "a FromSheet that is no number")?),
                "ToSheet" if a.ns == Ns::None => to = Some(self.uint(&a.value, "a ToSheet that is no number")?),
                "FromCell" if a.ns == Ns::None => c.from_cell = Some(self.sym(&a.value)),
                "ToCell" if a.ns == Ns::None => c.to_cell = Some(self.sym(&a.value)),
                "FromPart" if a.ns == Ns::None => c.from_part = Some(self.int(&a.value, "a FromPart that is no number")?),
                "ToPart" if a.ns == Ns::None => c.to_part = Some(self.int(&a.value, "a ToPart that is no number")?),
                _ => self.attr_extra(a, &mut c.extras, Place::Element)?,
            }
        }
        match (from, to) {
            (Some(f), Some(t)) => {
                c.from_sheet = f;
                c.to_sheet = t;
            }
            _ => return Err(self.bad("a Connect without FromSheet or ToSheet")),
        }
        loop {
            match self.next()? {
                Event::Start(x) => self.unknown(&x, &mut c.extras, Place::AfterChild(0))?,
                Event::Text(t) => self.text_extra(&t, &mut c.extras, Place::AfterChild(0))?,
                Event::End { .. } => return Ok(c),
                Event::Eof => return Err(self.bad("a Connect without end")),
            }
        }
    }

    fn connects(&mut self, s: &Start<'a>, out: &mut Vec<Connect>, extras: &mut Extras) -> R<()> {
        for a in &s.attrs {
            self.attr_extra(a, extras, Place::InConnects(0))?;
        }
        let mut n = 0u32;
        loop {
            match self.next()? {
                Event::Start(c) if c.is("Connect") => {
                    let x = self.connect(&c)?;
                    out.push(x);
                    n += 1;
                }
                Event::Start(c) => self.unknown(&c, extras, Place::InConnects(n))?,
                Event::Text(t) => self.text_extra(&t, extras, Place::InConnects(n))?,
                Event::End { .. } => return Ok(()),
                Event::Eof => return Err(self.bad("a Connects list without end")),
            }
        }
    }

    // PageContents / MasterContents
    fn contents(&mut self, root: &str, tree: &mut ShapeTree, connects: &mut Vec<Connect>, extras: &mut Extras) -> R<()> {
        let s = self.root(root)?;
        for a in &s.attrs {
            self.attr_extra(a, extras, Place::Element)?;
        }
        let mut list = Vec::new();
        let mut known = 0u32;
        let (mut had_shapes, mut had_connects) = (false, false);
        loop {
            match self.next()? {
                Event::Start(c) if c.is("Shapes") && !had_shapes => {
                    had_shapes = true;
                    self.shapes(&c, &mut list, extras)?;
                    known += 1;
                }
                Event::Start(c) if c.is("Connects") && !had_connects => {
                    had_connects = true;
                    self.connects(&c, connects, extras)?;
                    known += 1;
                }
                Event::Start(c) => self.unknown(&c, extras, Place::AfterChild(known))?,
                Event::Text(t) => self.text_extra(&t, extras, Place::AfterChild(known))?,
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("a contents part without end")),
            }
        }
        self.finish()?;
        let mut slots: Vec<u32> = Vec::with_capacity(list.len());
        for p in list {
            let parent = p.parent.and_then(|i| slots.get(i).copied());
            let slot = tree.insert(parent, p.id, p.data).map_err(|_| self.bad("two shapes with one ID"))?;
            slots.push(slot);
        }
        Ok(())
    }
}

// a shape read but not yet in its tree: its parent's index in the list
struct Pending {
    parent: Option<usize>,
    id: u32,
    data: ShapeData,
}

// a shape being read: its index in the list, what it has so far
struct ShapeFrame {
    idx: usize,
    id: u32,
    data: ShapeData,
    geometry: u32,
    known: u32,
    // its Shapes list was read (a second one is an unknown element)
    listed: bool,
}

// an open element of a Shapes list being read
enum Frame {
    // a Shapes list: how many shapes it had so far
    List(u32),
    // a Shape
    Shape(Box<ShapeFrame>),
}

// the extras unknown items of the list on top of the stack go to: its group's, or the outer ones
fn list_extras<'e>(stack: &'e mut [Frame], outer: &'e mut Extras) -> &'e mut Extras {
    let n = stack.len();
    match n.checked_sub(2).and_then(|i| stack.get_mut(i)) {
        Some(Frame::Shape(g)) => &mut g.data.extras,
        _ => outer,
    }
}

// the text of a part; the reader over it
fn decode<'b>(part: &str, bytes: &'b [u8]) -> R<Cow<'b, str>> {
    xml::text_of(bytes).ok_or_else(|| Error::Xml { part: alloc::format!("/{part}"), line: 1, error: XmlError::Unsupported })
}

fn cx<'a, 'l, 'p>(ld: &'l mut Loader<'p>, part: &'l str, text: &'a str, rels: &'l RelMap) -> Cx<'a, 'l, 'p> {
    let (depth, attrs) = (ld.lim.depth, ld.lim.attrs);
    Cx { ld, r: Reader::new(text, depth, attrs), part, rels }
}

// ---- document.xml

/// document.xml into the document: settings, colours, fonts, styles, the document sheet
pub(crate) fn document(ld: &mut Loader, part: &str, bytes: &[u8], rels: &RelMap) -> R<()> {
    let text = decode(part, bytes)?;
    let mut cx = cx(ld, part, &text, rels);
    let root = cx.root("VisioDocument")?;
    for a in &root.attrs {
        let p = cx.pair(a);
        cx.ld.doc.attrs.push(p);
    }
    let mut extras = Extras::new();
    let mut known = 0u32;
    let (mut settings, mut colors, mut faces, mut styles, mut sheet) = (false, false, false, false, false);
    loop {
        let c = match cx.next()? {
            Event::Start(c) => c,
            Event::Text(t) => {
                cx.text_extra(&t, &mut extras, Place::AfterChild(known))?;
                continue;
            }
            Event::End { .. } => break,
            Event::Eof => return Err(cx.bad("VisioDocument without end")),
        };
        if c.is("DocumentSettings") && !settings {
            settings = true;
            cx.settings(&c)?;
        } else if c.is("Colors") && !colors {
            colors = true;
            cx.colors(&c)?;
        } else if c.is("FaceNames") && !faces {
            faces = true;
            cx.faces(&c)?;
        } else if c.is("StyleSheets") && !styles {
            styles = true;
            cx.styles(&c)?;
        } else if c.is("DocumentSheet") && !sheet {
            sheet = true;
            let mut attrs = Vec::new();
            let (sh, st) = cx.sheet_element(&c, Some(&mut attrs))?;
            let ds = cx.ld.doc.doc_sheet_mut();
            ds.sheet = sh;
            ds.styles = st;
            ds.attrs = attrs;
        } else {
            cx.unknown(&c, &mut extras, Place::AfterChild(known))?;
            continue;
        }
        known += 1;
    }
    cx.finish()?;
    cx.ld.doc.extras = extras;
    Ok(())
}

impl<'a> Cx<'a, '_, '_> {
    fn settings(&mut self, s: &Start<'a>) -> R<()> {
        let mut st = core::mem::take(&mut self.ld.doc.settings);
        for a in &s.attrs {
            let v = &a.value;
            match a.local {
                "TopPage" if a.ns == Ns::None => st.top_page = Some(self.uint(v, "a TopPage that is no number")?),
                "DefaultTextStyle" if a.ns == Ns::None => st.default_text_style = Some(self.uint(v, "a DefaultTextStyle that is no number")?),
                "DefaultLineStyle" if a.ns == Ns::None => st.default_line_style = Some(self.uint(v, "a DefaultLineStyle that is no number")?),
                "DefaultFillStyle" if a.ns == Ns::None => st.default_fill_style = Some(self.uint(v, "a DefaultFillStyle that is no number")?),
                "DefaultGuideStyle" if a.ns == Ns::None => st.default_guide_style = Some(self.uint(v, "a DefaultGuideStyle that is no number")?),
                _ => {
                    let p = self.pair(a);
                    st.attrs.push(p);
                }
            }
        }
        loop {
            match self.next()? {
                Event::Start(c) if c.ns == Ns::Visio => match self.simple_text(&c)? {
                    Ok(t) => {
                        let item = (self.sym(c.qname), self.sym(&t));
                        self.count(Count::Item)?;
                        st.items.push(item);
                    }
                    Err(raw) => {
                        let item = ExtraItem::Element(self.sym(raw));
                        self.count(Count::Item)?;
                        st.extras.push(Extra { place: Place::AfterChild(st.items.len() as u32), item });
                    }
                },
                Event::Start(c) => {
                    let n = st.items.len() as u32;
                    self.unknown(&c, &mut st.extras, Place::AfterChild(n))?;
                }
                Event::Text(t) => {
                    let n = st.items.len() as u32;
                    self.text_extra(&t, &mut st.extras, Place::AfterChild(n))?;
                }
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("DocumentSettings without end")),
            }
        }
        self.ld.doc.settings = st;
        Ok(())
    }

    fn colors(&mut self, s: &Start<'a>) -> R<()> {
        let mut extras = Extras::new();
        for a in &s.attrs {
            self.attr_extra(a, &mut extras, Place::Element)?;
        }
        let mut list = Vec::new();
        loop {
            match self.next()? {
                Event::Start(c) => {
                    let attrs = own(&c);
                    let parsed = match attrs.as_slice() {
                        [x, y] if c.is("ColorEntry") && plain(x, "IX") && plain(y, "RGB") => Some((*x, *y)),
                        [y, x] if c.is("ColorEntry") && plain(x, "IX") && plain(y, "RGB") => Some((*x, *y)),
                        _ => None,
                    };
                    let ix = parsed.and_then(|(x, _)| self.uint(&x.value, "").ok());
                    let rgb = parsed.map(|(_, y)| y.value.clone());
                    let raw = self.skip(&c)?;
                    let inner_empty = c.empty || raw.rfind("</").is_some_and(|e| e == c.end - c.at);
                    match (ix, rgb) {
                        (Some(ix), Some(rgb)) if inner_empty => {
                            let rgb = self.sym(&rgb);
                            self.count(Count::Item)?;
                            list.push(ColorEntry { ix, rgb });
                        }
                        _ => {
                            let item = ExtraItem::Element(self.sym(raw));
                            self.count(Count::Item)?;
                            extras.push(Extra { place: Place::AfterChild(list.len() as u32), item });
                        }
                    }
                }
                Event::Text(t) => self.text_extra(&t, &mut extras, Place::AfterChild(list.len() as u32))?,
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("Colors without end")),
            }
        }
        self.ld.doc.colors = list;
        self.ld.doc.colors_extras = extras;
        Ok(())
    }

    fn faces(&mut self, s: &Start<'a>) -> R<()> {
        let mut extras = Extras::new();
        for a in &s.attrs {
            self.attr_extra(a, &mut extras, Place::Element)?;
        }
        let mut list = Vec::new();
        loop {
            match self.next()? {
                Event::Start(c) => {
                    let raw = self.skip(&c)?;
                    let inner_empty = c.empty || raw.rfind("</").is_some_and(|e| e == c.end - c.at);
                    if c.is("FaceName") && inner_empty {
                        let mut f = FaceName::default();
                        for a in &c.attrs {
                            let p = self.pair(a);
                            f.attrs.push(p);
                        }
                        self.count(Count::Item)?;
                        list.push(f);
                    } else {
                        let item = ExtraItem::Element(self.sym(raw));
                        self.count(Count::Item)?;
                        extras.push(Extra { place: Place::AfterChild(list.len() as u32), item });
                    }
                }
                Event::Text(t) => self.text_extra(&t, &mut extras, Place::AfterChild(list.len() as u32))?,
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("FaceNames without end")),
            }
        }
        self.ld.doc.faces = list;
        self.ld.doc.faces_extras = extras;
        Ok(())
    }

    fn styles(&mut self, s: &Start<'a>) -> R<()> {
        let mut extras = Extras::new();
        for a in &s.attrs {
            self.attr_extra(a, &mut extras, Place::Element)?;
        }
        let mut n = 0u32;
        loop {
            match self.next()? {
                Event::Start(c) if c.is("StyleSheet") => {
                    let st = self.style(&c)?;
                    self.ld.doc.insert_style(st).map_err(|_| self.bad("two styles with one ID"))?;
                    n += 1;
                }
                Event::Start(c) => self.unknown(&c, &mut extras, Place::AfterChild(n))?,
                Event::Text(t) => self.text_extra(&t, &mut extras, Place::AfterChild(n))?,
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("StyleSheets without end")),
            }
        }
        self.ld.doc.styles_extras = extras;
        Ok(())
    }

    fn style(&mut self, s: &Start<'a>) -> R<Style> {
        self.count(Count::Object)?;
        let mut h = StyleHeader::default();
        let mut id = None;
        for a in &s.attrs {
            let v = &a.value;
            match a.local {
                "ID" if a.ns == Ns::None => id = Some(self.uint(v, "a StyleSheet ID that is no number")?),
                "Name" if a.ns == Ns::None => h.name = Some(self.sym(v)),
                "NameU" if a.ns == Ns::None => h.name_u = Some(self.sym(v)),
                "IsCustomName" if a.ns == Ns::None => h.is_custom_name = Some(self.flag(v, "an IsCustomName that is no flag")?),
                "IsCustomNameU" if a.ns == Ns::None => h.is_custom_name_u = Some(self.flag(v, "an IsCustomNameU that is no flag")?),
                "LineStyle" if a.ns == Ns::None => h.based_on.line = Some(self.uint(v, "a LineStyle that is no number")?),
                "FillStyle" if a.ns == Ns::None => h.based_on.fill = Some(self.uint(v, "a FillStyle that is no number")?),
                "TextStyle" if a.ns == Ns::None => h.based_on.text = Some(self.uint(v, "a TextStyle that is no number")?),
                _ => {
                    let p = self.pair(a);
                    h.attrs.push(p);
                }
            }
        }
        h.id = id.ok_or_else(|| self.bad("a StyleSheet without ID"))?;
        let mut st = Style { header: h, ..Style::default() };
        let (mut geometry, mut known) = (0u32, 0u32);
        loop {
            match self.next()? {
                Event::Start(c) => {
                    if !self.sheet_child(&c, &mut st.sheet, &mut st.extras, &mut geometry, &mut known)? {
                        self.unknown(&c, &mut st.extras, Place::AfterChild(known))?;
                    }
                }
                Event::Text(t) => self.text_extra(&t, &mut st.extras, Place::AfterChild(known))?,
                Event::End { .. } => return Ok(st),
                Event::Eof => return Err(self.bad("a StyleSheet without end")),
            }
        }
    }
}

// ---- pages.xml, masters.xml

/// pages.xml -> the pages with the r:id of their contents
pub(crate) fn pages(ld: &mut Loader, part: &str, bytes: &[u8], rels: &RelMap) -> R<Vec<(Page, Option<String>)>> {
    let text = decode(part, bytes)?;
    let mut cx = cx(ld, part, &text, rels);
    let root = cx.root("Pages")?;
    let mut extras = Extras::new();
    for a in &root.attrs {
        cx.attr_extra(a, &mut extras, Place::Element)?;
    }
    let mut out = Vec::new();
    loop {
        match cx.next()? {
            Event::Start(c) if c.is("Page") => {
                let p = cx.page(&c)?;
                out.push(p);
            }
            Event::Start(c) => cx.unknown(&c, &mut extras, Place::AfterChild(out.len() as u32))?,
            Event::Text(t) => cx.text_extra(&t, &mut extras, Place::AfterChild(out.len() as u32))?,
            Event::End { .. } => break,
            Event::Eof => return Err(cx.bad("Pages without end")),
        }
    }
    cx.finish()?;
    cx.ld.doc.pages_extras = extras;
    Ok(out)
}

/// masters.xml -> the masters with the r:id of their contents
pub(crate) fn masters(ld: &mut Loader, part: &str, bytes: &[u8], rels: &RelMap) -> R<Vec<(Master, Option<String>)>> {
    let text = decode(part, bytes)?;
    let mut cx = cx(ld, part, &text, rels);
    let root = cx.root("Masters")?;
    let mut extras = Extras::new();
    for a in &root.attrs {
        cx.attr_extra(a, &mut extras, Place::Element)?;
    }
    let mut out = Vec::new();
    loop {
        match cx.next()? {
            Event::Start(c) if c.is("Master") => {
                let m = cx.master(&c)?;
                out.push(m);
            }
            Event::Start(c) => cx.unknown(&c, &mut extras, Place::AfterChild(out.len() as u32))?,
            Event::Text(t) => cx.text_extra(&t, &mut extras, Place::AfterChild(out.len() as u32))?,
            Event::End { .. } => break,
            Event::Eof => return Err(cx.bad("Masters without end")),
        }
    }
    cx.finish()?;
    cx.ld.doc.masters_extras = extras;
    Ok(out)
}

impl<'a> Cx<'a, '_, '_> {
    // a Rel element with r:id alone -> the id; None: not such an element
    fn rel_id(c: &Start<'a>) -> Option<String> {
        match own(c).as_slice() {
            [a] if c.is("Rel") && a.ns == Ns::Rel && a.local == "id" => Some(String::from(a.value.as_ref())),
            _ => None,
        }
    }

    fn page(&mut self, s: &Start<'a>) -> R<(Page, Option<String>)> {
        self.count(Count::Object)?;
        let mut h = PageHeader::default();
        let mut id = None;
        for a in &s.attrs {
            let v = &a.value;
            match a.local {
                "ID" if a.ns == Ns::None => id = Some(self.uint(v, "a Page ID that is no number")?),
                "Name" if a.ns == Ns::None => h.name = Some(self.sym(v)),
                "NameU" if a.ns == Ns::None => h.name_u = Some(self.sym(v)),
                "IsCustomName" if a.ns == Ns::None => h.is_custom_name = Some(self.flag(v, "an IsCustomName that is no flag")?),
                "IsCustomNameU" if a.ns == Ns::None => h.is_custom_name_u = Some(self.flag(v, "an IsCustomNameU that is no flag")?),
                "Background" if a.ns == Ns::None => h.background = self.flag(v, "a Background that is no flag")?,
                "BackPage" if a.ns == Ns::None => h.back_page = Some(self.uint(v, "a BackPage that is no number")?),
                _ => {
                    let p = self.pair(a);
                    h.attrs.push(p);
                }
            }
        }
        h.id = id.ok_or_else(|| self.bad("a Page without ID"))?;
        let mut page = Page::new(h);
        let mut rid = None;
        let mut known = 0u32;
        let mut sheet = false;
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(t) => {
                    self.text_extra(&t, &mut page.extras, Place::AfterChild(known))?;
                    continue;
                }
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("a Page without end")),
            };
            if c.is("PageSheet") && !sheet {
                sheet = true;
                let (sh, st) = self.sheet_element(&c, None)?;
                page.sheet = sh;
                page.header.sheet_styles = st;
            } else if let Some(r) = Self::rel_id(&c).filter(|_| rid.is_none()) {
                let raw = self.skip(&c)?;
                if !inner_blank(&c, raw) {
                    let item = ExtraItem::Element(self.sym(raw));
                    self.push(&mut page.extras, Place::AfterChild(known), item)?;
                    continue;
                }
                rid = Some(r);
            } else {
                self.unknown(&c, &mut page.extras, Place::AfterChild(known))?;
                continue;
            }
            known += 1;
        }
        Ok((page, rid))
    }

    fn master(&mut self, s: &Start<'a>) -> R<(Master, Option<String>)> {
        self.count(Count::Object)?;
        let mut h = MasterHeader::default();
        let mut id = None;
        for a in &s.attrs {
            let v = &a.value;
            match a.local {
                "ID" if a.ns == Ns::None => id = Some(self.uint(v, "a Master ID that is no number")?),
                "Name" if a.ns == Ns::None => h.name = Some(self.sym(v)),
                "NameU" if a.ns == Ns::None => h.name_u = Some(self.sym(v)),
                "IsCustomName" if a.ns == Ns::None => h.is_custom_name = Some(self.flag(v, "an IsCustomName that is no flag")?),
                "IsCustomNameU" if a.ns == Ns::None => h.is_custom_name_u = Some(self.flag(v, "an IsCustomNameU that is no flag")?),
                "BaseID" if a.ns == Ns::None => h.base_id = Some(self.sym(v)),
                "UniqueID" if a.ns == Ns::None => h.unique_id = Some(self.sym(v)),
                _ => {
                    let p = self.pair(a);
                    h.attrs.push(p);
                }
            }
        }
        h.id = id.ok_or_else(|| self.bad("a Master without ID"))?;
        let mut m = Master::new(h);
        let mut rid = None;
        let mut known = 0u32;
        let mut sheet = false;
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(t) => {
                    self.text_extra(&t, &mut m.extras, Place::AfterChild(known))?;
                    continue;
                }
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("a Master without end")),
            };
            if c.is("PageSheet") && !sheet {
                sheet = true;
                let (sh, st) = self.sheet_element(&c, None)?;
                m.sheet = sh;
                m.header.sheet_styles = st;
            } else if let Some(r) = Self::rel_id(&c).filter(|_| rid.is_none()) {
                let raw = self.skip(&c)?;
                if !inner_blank(&c, raw) {
                    let item = ExtraItem::Element(self.sym(raw));
                    self.push(&mut m.extras, Place::AfterChild(known), item)?;
                    continue;
                }
                rid = Some(r);
            } else if c.is("Icon") && m.icon.is_none() {
                match self.simple_text(&c)? {
                    Ok(t) => m.icon = Some(self.sym(&t)),
                    Err(raw) => {
                        let item = ExtraItem::Element(self.sym(raw));
                        self.count(Count::Item)?;
                        m.extras.push(Extra { place: Place::AfterChild(known), item });
                        continue;
                    }
                }
            } else {
                self.unknown(&c, &mut m.extras, Place::AfterChild(known))?;
                continue;
            }
            known += 1;
        }
        Ok((m, rid))
    }
}

// ---- page and master parts

/// a page part (PageContents) into `page`
pub(crate) fn page_contents(ld: &mut Loader, part: &str, bytes: &[u8], rels: &RelMap, page: &mut Page) -> R<()> {
    let text = decode(part, bytes)?;
    let mut cx = cx(ld, part, &text, rels);
    let mut extras = Extras::new();
    cx.contents("PageContents", &mut page.shapes, &mut page.connects, &mut extras)?;
    page.contents_extras = extras;
    Ok(())
}

/// a master part (MasterContents) into `master`
pub(crate) fn master_contents(ld: &mut Loader, part: &str, bytes: &[u8], rels: &RelMap, master: &mut Master) -> R<()> {
    let text = decode(part, bytes)?;
    let mut cx = cx(ld, part, &text, rels);
    let mut extras = Extras::new();
    cx.contents("MasterContents", &mut master.shapes, &mut master.connects, &mut extras)?;
    master.contents_extras = extras;
    Ok(())
}
