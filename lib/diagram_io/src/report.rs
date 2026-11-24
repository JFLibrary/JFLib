// report -- (JD19, DA-09) shape report definitions as Visio keeps them: the XML of
// VisioReportDefinition (a .vrd file holds the same) and the document sheet's User rows
// visPropRpt1, visPropRpt2 ... (Value: that XML as a string, U="STR")
//
//   parse(text) -> Result<ReportDef, ReportError>   a definition read (diagram_core::report's
//                                       model; a BOM is skipped, a UTF-16 .vrd is decoded
//                                       by the caller, e.g. with text_of)
//   to_xml(&def) -> String              the same as Visio writes it: the declaration, the root
//                                       with its attributes, Name, Title, the fields,
//                                       criteria, the group, the sort keys, then the elements
//                                       not known here as they were read
//   stored(doc) -> Vec<Stored>          every visPropRptN row in the order of its number, read
//   find(doc, name) -> Option<Stored>   the one with this name (case ignored)
//   add(tx, &def) -> row name           a new row: the number after the largest one
//   replace(tx, row, &def)              the row's definition anew
//   rename(tx, row, name)               only the name (and the title, when it was the name)
//   remove(tx, row)                     the row goes; the definitions after it move up a row
//                                       (visPropRpt3 into visPropRpt2 ...), so the numbers
//                                       stay without gaps (Annahme: Visio reads them in order)
//
// Where Visio keeps them ([MS-VSDX] says nothing; two files Visio 16 saved, bvisual.net): the
// root VisioReportDefinition in the namespace NS with ReportExists, SelectDrawingDataMethod,
// Description, Precision, ShowUnits, CaseSensitive, UserCells, HiddenProperties,
// AdvancedFilter, FilterExpression, Version and the children Name, Title, VisioRptDefField (ID,
// Display, DisplayOrder, Type, SummaryTypes, Tag; Name, DisplayName), VisioRptDefFilter (ID,
// Property, Condition, AndOr, PropertyType, PropertyDataType, Value, Unit), VisioRptDefGroup (ID,
// GroupingField, ItemDisplay, GrandSummary, ExcludeDuplicates), VisioRptDefSort (ID, SortField,
// Ascending, CaseSensitive).  The codes and what is assumed about them: diagram_core::report.
//
// Reading: the crate's own pull reader (xml.rs: bounded, no DTD, no entity but the five, an
// error and never a panic), at most 16 levels and 64 attributes per element, MAX_BYTES of text,
// 512 fields, criteria, sort keys and unknown elements each.  Elements are taken by their local
// name in the default namespace (NS or none); an unknown child of the root is kept as the text
// it was (one with a namespace prefix is left out: its declaration may be the root's), an
// unknown child of a field is not.  Attributes in no namespace that are not known are kept.
// Writing: double quotes as Visio writes them; esc() is the one place that escapes, and leaves
// out what XML 1.0 cannot carry.  A row whose text is no definition stays as it is (its
// Stored::def is the error); replace and remove take it like any other.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::report::{Condition, Field, Filter, Grouping, Property, ReportDef, Scope, Sort};
use diagram_core::{Document, EditError, RowKey, SecKey, SheetRef, Transaction, Unit, Value};

use crate::xml::{is_blank, Event, Ns, Reader, Start, XmlError};

/// the namespace of VisioReportDefinition
pub const NS: &str = "urn:schemas-microsoft-com:office:visio:reportdefinition";
/// the longest definition read (bytes)
pub const MAX_BYTES: usize = 1 << 20;
/// the row names' prefix in the document sheet's User section
pub const ROW_PREFIX: &str = "visPropRpt";
const MAX_ITEMS: usize = 512;
const DEPTH: usize = 16;
const ATTRS: usize = 64;

/// why a text is no report definition
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReportError {
    /// not well-formed XML
    Syntax,
    /// too long, too deep, too many attributes or elements
    Limit,
    /// a DOCTYPE or an encoding the reader does not take
    Unsupported,
    /// well-formed, but no VisioReportDefinition
    NotReport,
}

fn err(e: (XmlError, usize)) -> ReportError {
    match e.0 {
        XmlError::Depth | XmlError::Attributes => ReportError::Limit,
        XmlError::Unsupported => ReportError::Unsupported,
        _ => ReportError::Syntax,
    }
}

type R<T> = Result<T, ReportError>;

// the default namespace or none: the report's elements and attributes
fn ours(ns: Ns) -> bool {
    matches!(ns, Ns::None | Ns::Other)
}

fn num(s: &str) -> u32 {
    let t = s.trim();
    match t.parse::<i64>() {
        Ok(v) => v.clamp(0, u32::MAX as i64) as u32,
        Err(_) => t.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).map_or(0, |v| v.min(u32::MAX as f64) as u32),
    }
}

fn flag(s: &str) -> bool {
    matches!(s.trim(), "1" | "-1" | "true" | "TRUE" | "True")
}

// the attributes in no namespace: (name, value)
fn plain<'s>(s: &'s Start<'_>) -> impl Iterator<Item = (&'s str, &'s str)> {
    s.attrs.iter().filter(|a| a.ns == Ns::None).map(|a| (a.local, a.value.as_ref()))
}

// the text inside an element just started (child elements in it are passed over)
fn content(r: &mut Reader) -> R<String> {
    let mut s = String::new();
    loop {
        match r.next().map_err(err)? {
            Event::Text(t) => s.push_str(&t),
            Event::Start(c) => {
                r.skip(&c).map_err(err)?;
            }
            Event::End { .. } => return Ok(s),
            Event::Eof => return Err(ReportError::Syntax),
        }
    }
}

/// read a definition
pub fn parse(text: &str) -> R<ReportDef> {
    if text.len() > MAX_BYTES {
        return Err(ReportError::Limit);
    }
    let mut r = Reader::new(text.trim_start_matches('\u{feff}'), DEPTH, ATTRS);
    let root = loop {
        match r.next().map_err(err)? {
            Event::Start(s) => break s,
            Event::Text(t) if is_blank(&t) => {}
            _ => return Err(ReportError::Syntax),
        }
    };
    if !ours(root.ns) || root.local != "VisioReportDefinition" {
        return Err(ReportError::NotReport);
    }
    let mut d = ReportDef::new("");
    d.title.clear();
    d.grouping = None;
    d.show_units = false;
    d.precision = 0;
    d.exists = false;
    for (k, v) in plain(&root) {
        match k {
            "ReportExists" => d.exists = flag(v),
            "SelectDrawingDataMethod" => d.scope = Scope::of(num(v)),
            "Description" => d.description = String::from(v),
            "Precision" => d.precision = num(v).min(15),
            "ShowUnits" => d.show_units = flag(v),
            "CaseSensitive" => d.case_sensitive = flag(v),
            "UserCells" => d.user_cells = flag(v),
            "HiddenProperties" => d.hidden_properties = flag(v),
            "AdvancedFilter" => d.advanced_filter = flag(v),
            "FilterExpression" => d.filter_expression = String::from(v),
            "Version" => d.version = String::from(v),
            _ => d.attrs.push((String::from(k), String::from(v))),
        }
    }
    let full = |n: usize| if n >= MAX_ITEMS { Err(ReportError::Limit) } else { Ok(()) };
    loop {
        let c = match r.next().map_err(err)? {
            Event::Start(c) => c,
            Event::End { .. } => break,
            Event::Text(_) => continue,
            Event::Eof => return Err(ReportError::Syntax),
        };
        match (ours(c.ns), c.local) {
            (true, "Name") => d.name = content(&mut r)?,
            (true, "Title") => d.title = content(&mut r)?,
            (true, "VisioRptDefField") => {
                full(d.fields.len())?;
                let f = field(&mut r, &c)?;
                d.fields.push(f);
            }
            (true, "VisioRptDefFilter") => {
                full(d.filters.len())?;
                d.filters.push(filter(&c));
                r.skip(&c).map_err(err)?;
            }
            (true, "VisioRptDefGroup") if d.grouping.is_none() => {
                d.grouping = Some(group(&c));
                r.skip(&c).map_err(err)?;
            }
            (true, "VisioRptDefSort") => {
                full(d.sorts.len())?;
                d.sorts.push(sort(&c));
                r.skip(&c).map_err(err)?;
            }
            _ => {
                full(d.extra.len())?;
                let raw = r.skip(&c).map_err(err)?;
                if !c.qname.contains(':') {
                    d.extra.push(String::from(raw));
                }
            }
        }
    }
    // nothing but white space after the root
    loop {
        match r.next().map_err(err)? {
            Event::Eof => return Ok(d),
            Event::Text(t) if is_blank(&t) => {}
            _ => return Err(ReportError::Syntax),
        }
    }
}

fn field<'a>(r: &mut Reader<'a>, s: &Start<'a>) -> R<Field> {
    let mut f = Field::new(0, Property::Data(String::new()), "");
    let mut tag = 2;
    f.display = false;
    f.order = 0;
    for (k, v) in plain(s) {
        match k {
            "ID" => f.id = num(v),
            "Display" => f.display = flag(v),
            "DisplayOrder" => f.order = num(v),
            "Type" => f.data_type = num(v).min(u16::MAX as u32) as u16,
            "SummaryTypes" => f.summary = num(v),
            "Tag" => tag = num(v),
            _ => f.attrs.push((String::from(k), String::from(v))),
        }
    }
    let mut name = String::new();
    if !s.empty {
        loop {
            match r.next().map_err(err)? {
                Event::Start(c) if ours(c.ns) && c.local == "Name" => name = content(r)?,
                Event::Start(c) if ours(c.ns) && c.local == "DisplayName" => f.display_name = content(r)?,
                Event::Start(c) => {
                    r.skip(&c).map_err(err)?;
                }
                Event::End { .. } => break,
                Event::Text(_) => {}
                Event::Eof => return Err(ReportError::Syntax),
            }
        }
    } else {
        r.skip(s).map_err(err)?;
    }
    f.property = Property::of(tag, &name);
    Ok(f)
}

fn filter(s: &Start) -> Filter {
    let mut f = Filter::new(0, Property::Data(String::new()), Condition::Other(0), "");
    let (mut tag, mut key) = (2, String::new());
    f.and = false;
    for (k, v) in plain(s) {
        match k {
            "ID" => f.id = num(v),
            "Property" => key = String::from(v),
            "Condition" => f.condition = Condition::of(num(v)),
            "AndOr" => f.and = flag(v),
            "PropertyType" => tag = num(v),
            "PropertyDataType" => f.data_type = num(v).min(u16::MAX as u32) as u16,
            "Value" => f.value = String::from(v),
            "Unit" => f.unit = String::from(v),
            _ => f.attrs.push((String::from(k), String::from(v))),
        }
    }
    f.property = Property::of(tag, &key);
    f
}

fn group(s: &Start) -> Grouping {
    let mut g = Grouping { id: 0, field: 0, show_items: false, grand_summary: false, exclude_duplicates: false, attrs: Vec::new() };
    for (k, v) in plain(s) {
        match k {
            "ID" => g.id = num(v),
            "GroupingField" => g.field = num(v),
            "ItemDisplay" => g.show_items = flag(v),
            "GrandSummary" => g.grand_summary = flag(v),
            "ExcludeDuplicates" => g.exclude_duplicates = flag(v),
            _ => g.attrs.push((String::from(k), String::from(v))),
        }
    }
    g
}

fn sort(s: &Start) -> Sort {
    let mut o = Sort::new(0, 0, false);
    for (k, v) in plain(s) {
        match k {
            "ID" => o.id = num(v),
            "SortField" => o.field = num(v),
            "Ascending" => o.ascending = flag(v),
            "CaseSensitive" => o.case_sensitive = flag(v),
            _ => o.attrs.push((String::from(k), String::from(v))),
        }
    }
    o
}

// ---------------------------------------------------------------- writing

// the one place that escapes: markup, both quotes, CR and the line ends and tabs an attribute
// would lose; what XML 1.0 cannot carry is left out
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            '\r' => o.push_str("&#13;"),
            '\n' => o.push_str("&#10;"),
            '\t' => o.push_str("&#9;"),
            c if matches!(c as u32, 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff) => o.push(c),
            _ => {}
        }
    }
    o
}

fn b(v: bool) -> &'static str {
    if v { "1" } else { "0" }
}

fn attr(o: &mut String, k: &str, v: &str) {
    o.push(' ');
    o.push_str(k);
    o.push_str("=\"");
    o.push_str(&esc(v));
    o.push('"');
}

fn extra_attrs(o: &mut String, attrs: &[(String, String)]) {
    for (k, v) in attrs {
        // a name the writer cannot carry is left out
        let ok = k.as_bytes().first().is_some_and(|c| c.is_ascii_alphabetic()) && k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
        if ok {
            attr(o, k, v);
        }
    }
}

fn element(o: &mut String, name: &str, text: &str) {
    o.push_str(&format!("<{name}>{}</{name}>", esc(text)));
}

/// the definition as XML (the text of a visPropRptN cell and of a .vrd file)
pub fn to_xml(d: &ReportDef) -> String {
    let mut o = String::from("<?xml version=\"1.0\"?>\n<VisioReportDefinition");
    attr(&mut o, "xmlns", NS);
    attr(&mut o, "ReportExists", b(d.exists));
    attr(&mut o, "SelectDrawingDataMethod", &format!("{}", d.scope.code()));
    attr(&mut o, "Description", &d.description);
    attr(&mut o, "Precision", &format!("{}", d.precision));
    attr(&mut o, "ShowUnits", b(d.show_units));
    attr(&mut o, "CaseSensitive", b(d.case_sensitive));
    attr(&mut o, "UserCells", b(d.user_cells));
    attr(&mut o, "HiddenProperties", b(d.hidden_properties));
    attr(&mut o, "AdvancedFilter", b(d.advanced_filter));
    attr(&mut o, "FilterExpression", &d.filter_expression);
    attr(&mut o, "Version", if d.version.is_empty() { "3" } else { &d.version });
    extra_attrs(&mut o, &d.attrs);
    o.push('>');
    element(&mut o, "Name", &d.name);
    element(&mut o, "Title", &d.title);
    for f in &d.fields {
        o.push_str("<VisioRptDefField");
        attr(&mut o, "ID", &format!("{}", f.id));
        attr(&mut o, "Display", b(f.display));
        attr(&mut o, "DisplayOrder", &format!("{}", f.order));
        attr(&mut o, "Type", &format!("{}", f.data_type));
        attr(&mut o, "SummaryTypes", &format!("{}", f.summary));
        attr(&mut o, "Tag", &format!("{}", f.property.tag()));
        extra_attrs(&mut o, &f.attrs);
        o.push('>');
        element(&mut o, "Name", f.property.key());
        element(&mut o, "DisplayName", &f.display_name);
        o.push_str("</VisioRptDefField>");
    }
    for f in &d.filters {
        o.push_str("<VisioRptDefFilter");
        attr(&mut o, "ID", &format!("{}", f.id));
        attr(&mut o, "Property", f.property.key());
        attr(&mut o, "Condition", &format!("{}", f.condition.code()));
        attr(&mut o, "AndOr", b(f.and));
        attr(&mut o, "PropertyType", &format!("{}", f.property.tag()));
        attr(&mut o, "PropertyDataType", &format!("{}", f.data_type));
        attr(&mut o, "Value", &f.value);
        attr(&mut o, "Unit", &f.unit);
        extra_attrs(&mut o, &f.attrs);
        o.push_str("/>");
    }
    if let Some(g) = &d.grouping {
        o.push_str("<VisioRptDefGroup");
        attr(&mut o, "ID", &format!("{}", g.id));
        attr(&mut o, "GroupingField", &format!("{}", g.field));
        attr(&mut o, "ItemDisplay", b(g.show_items));
        attr(&mut o, "GrandSummary", b(g.grand_summary));
        attr(&mut o, "ExcludeDuplicates", b(g.exclude_duplicates));
        extra_attrs(&mut o, &g.attrs);
        o.push_str("/>");
    }
    for s in &d.sorts {
        o.push_str("<VisioRptDefSort");
        attr(&mut o, "ID", &format!("{}", s.id));
        attr(&mut o, "SortField", &format!("{}", s.field));
        attr(&mut o, "Ascending", b(s.ascending));
        attr(&mut o, "CaseSensitive", b(s.case_sensitive));
        extra_attrs(&mut o, &s.attrs);
        o.push_str("/>");
    }
    for e in &d.extra {
        o.push_str(e);
    }
    o.push_str("</VisioReportDefinition>");
    o
}

// ---------------------------------------------------------------- the document sheet

/// a definition kept in the document
#[derive(Clone, PartialEq, Debug)]
pub struct Stored {
    /// the row's name (visPropRpt3)
    pub row: String,
    /// its number (3)
    pub index: u32,
    /// the text of its Value cell
    pub xml: String,
    /// the definition read from it
    pub def: Result<ReportDef, ReportError>,
}

fn index_of(row: &str) -> Option<u32> {
    let n = row.strip_prefix(ROW_PREFIX)?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

/// the definitions of the document sheet, by their numbers
pub fn stored(doc: &Document) -> Vec<Stored> {
    let mut v: Vec<Stored> = doc
        .user_cells(SheetRef::Document)
        .into_iter()
        .filter_map(|r| {
            let row = r.name()?;
            let index = index_of(row)?;
            let xml = match r.get("Value").value() {
                Value::Text(s) => String::from(doc.str(s)),
                _ => String::new(),
            };
            let def = parse(&xml);
            Some(Stored { row: String::from(row), index, xml, def })
        })
        .collect();
    v.sort_by_key(|s| s.index);
    v
}

/// the definition named `name` (case ignored)
pub fn find(doc: &Document, name: &str) -> Option<Stored> {
    stored(doc).into_iter().find(|s| s.def.as_ref().is_ok_and(|d| d.name.to_lowercase() == name.to_lowercase()))
}

fn write(tx: &mut Transaction, row: &str, xml: &str) -> Result<(), EditError> {
    let v = Value::Text(tx.intern(xml));
    tx.set_user(SheetRef::Document, row, v, Some(Unit::Str))
}

/// keep a new definition in the document -> its row's name
pub fn add(tx: &mut Transaction, def: &ReportDef) -> Result<String, EditError> {
    let next = stored(tx.doc()).iter().map(|s| s.index).max().unwrap_or(0) + 1;
    let row = format!("{ROW_PREFIX}{next}");
    write(tx, &row, &to_xml(def))?;
    Ok(row)
}

/// give a kept definition's row another definition
pub fn replace(tx: &mut Transaction, row: &str, def: &ReportDef) -> Result<(), EditError> {
    if !stored(tx.doc()).iter().any(|s| s.row == row) {
        return Err(EditError::NoRow);
    }
    write(tx, row, &to_xml(def))
}

/// rename a kept definition (its title too when it was its name); a row that is no
/// definition is refused (Invalid)
pub fn rename(tx: &mut Transaction, row: &str, name: &str) -> Result<(), EditError> {
    let s = stored(tx.doc()).into_iter().find(|s| s.row == row).ok_or(EditError::NoRow)?;
    let mut d = s.def.map_err(|_| EditError::Invalid)?;
    if d.title == d.name {
        d.title = String::from(name);
    }
    d.name = String::from(name);
    write(tx, row, &to_xml(&d))
}

/// remove a kept definition; the ones after it move up a row
pub fn remove(tx: &mut Transaction, row: &str) -> Result<(), EditError> {
    let all = stored(tx.doc());
    let at = all.iter().position(|s| s.row == row).ok_or(EditError::NoRow)?;
    for k in at..all.len() - 1 {
        let next = all[k + 1].xml.clone();
        write(tx, &all[k].row, &next)?;
    }
    let (user, last) = (tx.intern("User"), tx.intern(&all[all.len() - 1].row));
    tx.delete_row(SheetRef::Document, SecKey::one(user), RowKey::Named(last))
}

#[cfg(test)]
mod tests;
