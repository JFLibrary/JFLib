// xml -- a report definition's XML (VisioReportDefinition): read and written
//
//   parse(text) -> Result<ReportDef, ReportError>   the text of a visPropRptN cell or a .vrd file
//   to_xml(&def) -> String                          the same, as Visio writes it: the declaration,
//                                                   the root with its attributes, Name, Title, the
//                                                   fields, criteria, the group, the sort keys
//
// The one place of the module that reads and escapes XML: the reader is base::xml (bounded, no
// DTD, no entities but the five, no processing instructions, comments or mixed content; an
// error, never a panic), which re-exports formats::xml after the merge with main -- switching to
// formats::xml is the `use` line below.  Bounds: MAX_BYTES of text, 4096 elements, 16 levels,
// 512 fields, criteria and sort keys each.  The text may start with a BOM; a .vrd file in
// UTF-16 is decoded by the caller.  Attributes are read without regard to their order; unknown
// ones (in no namespace) and unknown child elements of the root are kept, an unknown child of a
// field is not.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::xml::{self, Limits, Node};

use super::{Condition, Field, Filter, Grouping, Property, ReportDef, Scope, Sort, NS};

/// the longest definition read (bytes)
pub const MAX_BYTES: usize = 1 << 20;
const MAX_ITEMS: usize = 512;
const LIMITS: Limits = Limits { bytes: MAX_BYTES, nodes: 4096, depth: 16 };

/// why a text is no report definition
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReportError {
    /// not well-formed XML
    Syntax,
    /// too long, too many elements or too deep
    Limit,
    /// a DOCTYPE, an entity, a comment or another construct the reader does not take
    Unsupported,
    /// well-formed, but no VisioReportDefinition
    NotReport,
}

impl From<xml::Error> for ReportError {
    fn from(e: xml::Error) -> ReportError {
        match e {
            xml::Error::Limit => ReportError::Limit,
            xml::Error::Unsupported => ReportError::Unsupported,
            _ => ReportError::Syntax,
        }
    }
}

fn ours(n: &Node) -> bool {
    n.name.ns == NS || n.name.ns.is_empty()
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

// the text of a child element (Name, Title, DisplayName)
fn child_text(n: &Node, local: &str) -> Option<String> {
    n.children.iter().find(|c| ours(c) && c.name.local == local).map(|c| c.text.clone())
}

// the attributes in no namespace: (name, value)
fn plain_attrs(n: &Node) -> impl Iterator<Item = (&str, &str)> {
    n.attrs.iter().filter(|(a, _)| a.ns.is_empty()).map(|(a, v)| (a.local.as_str(), v.as_str()))
}

/// read a definition
pub fn parse(text: &str) -> Result<ReportDef, ReportError> {
    if text.len() > MAX_BYTES {
        return Err(ReportError::Limit);
    }
    let root = xml::parse_with(text.trim_start_matches('\u{feff}'), LIMITS)?;
    if !ours(&root) || root.name.local != "VisioReportDefinition" {
        return Err(ReportError::NotReport);
    }
    let mut d = ReportDef::new("");
    d.title.clear();
    d.grouping = None;
    d.show_units = false;
    d.precision = 0;
    d.exists = false;
    for (k, v) in plain_attrs(&root) {
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
    for c in &root.children {
        if !ours(c) {
            push_extra(&mut d, c, &root.name.ns)?;
            continue;
        }
        match c.name.local.as_str() {
            "Name" => d.name = c.text.clone(),
            "Title" => d.title = c.text.clone(),
            "VisioRptDefField" => {
                if d.fields.len() >= MAX_ITEMS {
                    return Err(ReportError::Limit);
                }
                d.fields.push(field(c));
            }
            "VisioRptDefFilter" => {
                if d.filters.len() >= MAX_ITEMS {
                    return Err(ReportError::Limit);
                }
                d.filters.push(filter(c));
            }
            "VisioRptDefGroup" if d.grouping.is_none() => d.grouping = Some(group(c)),
            "VisioRptDefSort" => {
                if d.sorts.len() >= MAX_ITEMS {
                    return Err(ReportError::Limit);
                }
                d.sorts.push(sort(c));
            }
            _ => push_extra(&mut d, c, &root.name.ns)?,
        }
    }
    Ok(d)
}

fn push_extra(d: &mut ReportDef, c: &Node, ns: &str) -> Result<(), ReportError> {
    if d.extra.len() >= MAX_ITEMS {
        return Err(ReportError::Limit);
    }
    let mut s = String::new();
    node_xml(c, ns, &mut s);
    d.extra.push(s);
    Ok(())
}

fn field(n: &Node) -> Field {
    let mut f = Field::new(0, Property::Data(String::new()), "");
    let mut tag = 2;
    f.display = false;
    f.order = 0;
    for (k, v) in plain_attrs(n) {
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
    f.property = Property::of(tag, &child_text(n, "Name").unwrap_or_default());
    f.display_name = child_text(n, "DisplayName").unwrap_or_default();
    f
}

fn filter(n: &Node) -> Filter {
    let mut f = Filter::new(0, Property::Data(String::new()), Condition::Other(0), "");
    let (mut tag, mut key) = (2, String::new());
    f.and = false;
    for (k, v) in plain_attrs(n) {
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

fn group(n: &Node) -> Grouping {
    let mut g = Grouping { id: 0, field: 0, show_items: false, grand_summary: false, exclude_duplicates: false, attrs: Vec::new() };
    for (k, v) in plain_attrs(n) {
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

fn sort(n: &Node) -> Sort {
    let mut s = Sort::new(0, 0, false);
    for (k, v) in plain_attrs(n) {
        match k {
            "ID" => s.id = num(v),
            "SortField" => s.field = num(v),
            "Ascending" => s.ascending = flag(v),
            "CaseSensitive" => s.case_sensitive = flag(v),
            _ => s.attrs.push((String::from(k), String::from(v))),
        }
    }
    s
}

// ---------------------------------------------------------------- writing

// a text for an attribute value or element content: what XML 1.0 cannot carry is left out
fn esc(s: &str) -> String {
    let clean: String = s.chars().filter(|&c| xml::valid_char(c)).collect();
    xml::escape(&clean).unwrap_or_default()
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
        if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) && k.as_bytes()[0].is_ascii_alphabetic() {
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

// an element kept as it was read: its name (with a default namespace declaration when it differs
// from its parent's), its attributes in no namespace, its text or children
fn node_xml(n: &Node, parent_ns: &str, o: &mut String) {
    let name = &n.name.local;
    o.push('<');
    o.push_str(name);
    if n.name.ns != parent_ns {
        attr(o, "xmlns", &n.name.ns);
    }
    for (k, v) in plain_attrs(n) {
        attr(o, k, v);
    }
    if n.children.is_empty() && n.text.is_empty() {
        o.push_str("/>");
        return;
    }
    o.push('>');
    if n.children.is_empty() {
        o.push_str(&esc(&n.text));
    }
    for c in &n.children {
        node_xml(c, &n.name.ns, o);
    }
    o.push_str("</");
    o.push_str(name);
    o.push('>');
}
