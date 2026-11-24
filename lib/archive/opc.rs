// Open Packaging Conventions (ECMA-376 Part 2) over this crate's ZIP reader and writer: a
// package's parts, their content types ([Content_Types].xml) and relationships (_rels/*.rels).
// DOCX (doc_io) and VSDX (diagram_io) share it. New native library code, taken out of doc_io's
// DOCX preservation. Nothing outside the package is ever resolved or fetched: external targets
// stay text.
//
//   Package::read(bytes, limits, rules) -> parts, content types, relationships, all checked
//   package.relationships(source) / package.resolve(source, target) / package.part(name)
//   package.set_part(..) / remove_part(..) / set_relationships(..), then package.write(..):
//   the parts nobody touched go back as they came (unknown parts are kept, never dropped)
//
// Part names are compared the way OPC says, ASCII case-insensitive and percent-escape aware
// ("Word/Media/a%20b.png" names the same part as "word/media/a b.png"), unless Rules::exact_names
// asks for the older profile of exact strings without escapes.

use crate::{Entry, Limits, Method};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};
use formats::{
    encoding,
    xml::{self, Node},
};

/// The ZIP item that holds the content types; it is no part of its own.
pub const CONTENT_TYPES: &str = "[Content_Types].xml";
/// The package's own relationship part.
pub const PACKAGE_RELS: &str = "_rels/.rels";
/// Content type of a relationship part.
pub const RELS_TYPE: &str = "application/vnd.openxmlformats-package.relationships+xml";
/// Namespace of `[Content_Types].xml`.
pub const NS_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
/// Namespace of relationship parts.
pub const NS_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
/// Relationship type from the package to its main part (transitional OOXML).
pub const OFFICE_DOCUMENT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
/// Relationship type from the package to its main part (strict OOXML).
pub const OFFICE_DOCUMENT_STRICT: &str =
    "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";

/// Why a package was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The ZIP container.
    Zip(crate::Error),
    /// `[Content_Types].xml` or a relationship part is not XML within `Rules::xml`.
    Xml(xml::Error),
    /// Package structure: a missing or duplicate part, content type or relationship id, a part
    /// without a content type, a relationship part without its source, a target that leaves the
    /// package or names no part, text that is not Unicode.
    Invalid,
    /// `[Content_Types].xml` or a relationship part has another root element, or attributes or
    /// text on it; the root's local name.
    Root(String),
    /// A part name or internal target this module does not resolve: empty, a backslash, `:`,
    /// `?`, `#`, a control character or a bad percent-escape (also an escaped `/` or `\`);
    /// with `Rules::exact_names` any `%`.
    Address,
    /// An empty content type, or without `Rules::macros` one of a macro project.
    ContentType,
    /// Without `Rules::macros`: a part of a macro project (`vbaProject.bin`, `vbaData.xml`),
    /// whatever its content type says.
    MacroPart,
    /// Without `Rules::macros`: a relationship of a macro project's type.
    MacroRelationship,
}

/// What a package may hold and how its names compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    /// Accept macro (VBA) projects. Off by default: content types, relationship types and part
    /// names of a macro project refuse the package.
    pub macros: bool,
    /// Compare part names and targets as exact strings and refuse percent-escapes, instead of
    /// OPC's case-insensitive, escape-aware equivalence (and refuse `%` in ZIP names). The
    /// conservative profile of jfWord's DOCX reader.
    pub exact_names: bool,
    /// Limits of `[Content_Types].xml` and of each relationship part.
    pub xml: xml::Limits,
}
impl Rules {
    /// No macros, OPC's name equivalence, the XML reader's default limits.
    pub const DEFAULT: Rules = Rules {
        macros: false,
        exact_names: false,
        xml: xml::Limits { bytes: 4 << 20, nodes: 30000, depth: 48 },
    };
    /// Resolve a relationship's target against its source part (`""` for the package) to a
    /// part name without the leading `/`. `..` may not climb out of the package. With OPC's
    /// equivalence the result is in canonical form (see `normalize`).
    pub fn resolve(&self, source: &str, target: &str) -> Result<String, Error> {
        if target.is_empty() || target.chars().any(char::is_control) {
            return Err(Error::Address);
        }
        let target = if self.exact_names {
            if target.contains(['\\', ':', '%', '?', '#']) {
                return Err(Error::Address);
            }
            String::from(target)
        } else {
            canonical(target, true)?
        };
        let mut components: Vec<&str> = if target.starts_with('/') {
            Vec::new()
        } else {
            source
                .rsplit_once('/')
                .map_or(Vec::new(), |(dir, _)| dir.split('/').collect())
        };
        for part in target.strip_prefix('/').unwrap_or(&target).split('/') {
            match part {
                "" => return Err(Error::Invalid),
                "." => {}
                ".." => {
                    if components.pop().is_none() {
                        return Err(Error::Invalid);
                    }
                }
                _ => components.push(part),
            }
        }
        if components.is_empty() {
            return Err(Error::Invalid);
        }
        let joined = components.join("/");
        if self.exact_names { Ok(joined) } else { canonical(&joined, false) }
    }
}
impl Default for Rules {
    fn default() -> Self {
        Self::DEFAULT
    }
}

fn unreserved(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~')
}
fn pchar(c: u8) -> bool {
    unreserved(c) || b"!$&'()*+,;=:@".contains(&c)
}
fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}
fn escape(out: &mut String, c: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(HEX[(c >> 4) as usize] as char);
    out.push(HEX[(c & 15) as usize] as char);
}
// The canonical form of a name or (reference) a relative target: escapes of unreserved
// characters decoded, other escapes in upper case, every byte that is neither a pchar nor '/'
// escaped. A reference may not hold ':', '?' or '#' (a scheme, a query, a fragment).
fn canonical(s: &str, reference: bool) -> Result<String, Error> {
    if s.chars().any(char::is_control) {
        return Err(Error::Address);
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'%' {
            let v = match (b.get(i + 1).and_then(|&h| hex(h)), b.get(i + 2).and_then(|&l| hex(l))) {
                (Some(h), Some(l)) => h << 4 | l,
                _ => return Err(Error::Address),
            };
            if v == b'/' || v == b'\\' || v < 0x20 || v == 0x7f {
                return Err(Error::Address);
            }
            if unreserved(v) {
                out.push(v as char);
            } else {
                escape(&mut out, v);
            }
            i += 3;
            continue;
        }
        if c == b'\\' || (reference && matches!(c, b':' | b'?' | b'#')) {
            return Err(Error::Address);
        }
        if c == b'/' || pchar(c) {
            out.push(c as char);
        } else {
            escape(&mut out, c);
        }
        i += 1;
    }
    Ok(out)
}
/// A part name in canonical form, without the leading `/`: escapes of unreserved characters
/// decoded (`%41` is `A`), other escapes in upper case, spaces, non-ASCII and other characters
/// a URI path may not hold escaped (`ä` is `%C3%A4`). `Address` for a bad name: a bad escape,
/// an escaped `/` or `\`, a control character, an empty, `.` or `..` segment.
pub fn normalize(name: &str) -> Result<String, Error> {
    let c = canonical(name.strip_prefix('/').unwrap_or(name), false)?;
    if c.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
        return Err(Error::Address);
    }
    Ok(c)
}
/// Whether two part names name the same part (OPC: ASCII case-insensitive, after `normalize`).
pub fn same_part(a: &str, b: &str) -> bool {
    matches!((normalize(a), normalize(b)), (Ok(a), Ok(b)) if a.eq_ignore_ascii_case(&b))
}
// the comparison key of a name: exact names (and the content types' item) only fold ASCII case
fn key(name: &str, exact: bool) -> Result<String, Error> {
    if exact || name.eq_ignore_ascii_case(CONTENT_TYPES) {
        Ok(name.to_ascii_lowercase())
    } else {
        Ok(normalize(name)?.to_ascii_lowercase())
    }
}
// a name's extension (of its last segment), "" when it has none
fn extension(name: &str) -> &str {
    let last = name.rsplit('/').next().unwrap_or(name);
    last.rsplit_once('.').map_or("", |(_, e)| e)
}
fn macro_type(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("macroenabled") || lower.contains("vbaproject") || lower.contains("vbadata")
}
fn macro_part(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("vbaproject") || lower.ends_with("vbadata.xml")
}
fn parse(b: &[u8], rules: &Rules) -> Result<Node, Error> {
    let (s, _) = encoding::decode(b, rules.xml.bytes).map_err(|_| Error::Invalid)?;
    xml::parse_with(&s, rules.xml).map_err(Error::Xml)
}
fn container(n: &Node, ns: &str, name: &str) -> Result<(), Error> {
    if !n.is(ns, name) || !n.attrs.is_empty() || !n.text.is_empty() {
        return Err(Error::Root(n.name.local.clone()));
    }
    Ok(())
}
fn esc(s: &str) -> Result<String, Error> {
    xml::escape(s).map_err(Error::Xml)
}

/// The relationship part of a source part: `_rels/.rels` for the package (`""`),
/// `dir/_rels/name.rels` for `dir/name`.
pub fn rels_name(source: &str) -> String {
    let source = source.strip_prefix('/').unwrap_or(source);
    match source.rsplit_once('/') {
        _ if source.is_empty() => String::from(PACKAGE_RELS),
        Some((dir, file)) => alloc::format!("{dir}/_rels/{file}.rels"),
        None => alloc::format!("_rels/{source}.rels"),
    }
}
/// The source part of a relationship part (`""` for `_rels/.rels`); `Invalid` for a name
/// that is no relationship part's.
pub fn rels_source(name: &str) -> Result<String, Error> {
    if name == PACKAGE_RELS {
        return Ok(String::new());
    }
    let name = name.strip_suffix(".rels").ok_or(Error::Invalid)?;
    if let Some((dir, file)) = name.rsplit_once("/_rels/") {
        if file.is_empty() || file.contains('/') {
            return Err(Error::Invalid);
        }
        Ok(alloc::format!("{dir}/{file}"))
    } else if let Some(file) = name.strip_prefix("_rels/") {
        if file.is_empty() || file.contains('/') {
            return Err(Error::Invalid);
        }
        Ok(String::from(file))
    } else {
        Err(Error::Invalid)
    }
}

/// `[Content_Types].xml`: content types by extension (defaults) and by part (overrides).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentTypes {
    // (extension as written, its key, content type)
    defaults: Vec<(String, String, String)>,
    // (part name as written without '/', content type) and key -> index
    overrides: Vec<(String, String)>,
    index: BTreeMap<String, usize>,
    exact: bool,
}
impl ContentTypes {
    /// Empty: no defaults, no overrides, OPC's name equivalence.
    pub fn new() -> Self {
        Self::default()
    }
    /// The content type of a part: its override, else the default for its extension
    /// (case-insensitive); None when it has neither.
    pub fn get(&self, part: &str) -> Option<&str> {
        let k = key(part, self.exact).ok()?;
        if let Some(&i) = self.index.get(&k) {
            return Some(self.overrides[i].1.as_str());
        }
        self.default_of(&k)
    }
    // the default for the extension of a part's key
    fn default_of(&self, k: &str) -> Option<&str> {
        let ext = extension(k);
        self.defaults
            .iter()
            .find(|(_, e, _)| !ext.is_empty() && *e == ext)
            .map(|(_, _, t)| t.as_str())
    }
    /// The defaults in file order: (extension, content type).
    pub fn defaults(&self) -> impl Iterator<Item = (&str, &str)> {
        self.defaults.iter().map(|(e, _, t)| (e.as_str(), t.as_str()))
    }
    /// The overrides in file order: (part name without the leading `/`, content type).
    pub fn overrides(&self) -> impl Iterator<Item = (&str, &str)> {
        self.overrides.iter().map(|(n, t)| (n.as_str(), t.as_str()))
    }
    /// Set the default of an extension (no `.`, `/` or `\`), replacing an earlier one.
    pub fn set_default(&mut self, extension: &str, content_type: &str) -> Result<(), Error> {
        let k = self.ext_key(extension)?;
        if content_type.is_empty() {
            return Err(Error::ContentType);
        }
        match self.defaults.iter_mut().find(|(_, e, _)| *e == k) {
            Some(d) => d.2 = String::from(content_type),
            None => self.defaults.push((String::from(extension), k, String::from(content_type))),
        }
        Ok(())
    }
    /// Set the override of a part, replacing an earlier one.
    pub fn set_override(&mut self, part: &str, content_type: &str) -> Result<(), Error> {
        let part = part.strip_prefix('/').unwrap_or(part);
        let k = self.name_key(part)?;
        if content_type.is_empty() {
            return Err(Error::ContentType);
        }
        match self.index.get(&k) {
            Some(&i) => self.overrides[i].1 = String::from(content_type),
            None => {
                self.index.insert(k, self.overrides.len());
                self.overrides.push((String::from(part), String::from(content_type)));
            }
        }
        Ok(())
    }
    /// Remove the override of a part; false when it had none.
    pub fn remove_override(&mut self, part: &str) -> bool {
        let Ok(k) = key(part, self.exact) else { return false };
        let Some(i) = self.index.remove(&k) else { return false };
        self.overrides.remove(i);
        for v in self.index.values_mut() {
            if *v > i {
                *v -= 1;
            }
        }
        true
    }
    /// `[Content_Types].xml` with an XML declaration: the defaults, then the overrides.
    pub fn to_xml(&self) -> Result<String, Error> {
        let mut s = alloc::format!("{DECL}<Types xmlns=\"{NS_TYPES}\">");
        for (e, _, t) in &self.defaults {
            s.push_str(&alloc::format!("<Default Extension=\"{}\" ContentType=\"{}\"/>", esc(e)?, esc(t)?));
        }
        for (n, t) in &self.overrides {
            s.push_str(&alloc::format!("<Override PartName=\"/{}\" ContentType=\"{}\"/>", esc(n)?, esc(t)?));
        }
        s.push_str("</Types>");
        Ok(s)
    }
    fn ext_key(&self, extension: &str) -> Result<String, Error> {
        if extension.is_empty() || extension.contains(['/', '\\', '.']) {
            return Err(Error::Invalid);
        }
        key(extension, self.exact).map_err(|_| Error::Invalid)
    }
    fn name_key(&self, part: &str) -> Result<String, Error> {
        if self.exact {
            if Rules::DEFAULT_EXACT.resolve("", part)? != part {
                return Err(Error::Invalid);
            }
            key(part, true)
        } else {
            if part.contains(['?', '#', ':']) {
                return Err(Error::Address);
            }
            key(part, false).map_err(|e| if part.split('/').any(|s| s.is_empty() || s == "." || s == "..") { Error::Invalid } else { e })
        }
    }
}
impl Rules {
    const DEFAULT_EXACT: Rules = Rules { exact_names: true, ..Rules::DEFAULT };
}

/// One relationship of a source part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    /// Unique among the source's relationships (`rId1` …).
    pub id: String,
    /// The relationship type, a URI (`Type`).
    pub kind: String,
    /// The target as written: relative to the source part, or a URI when `external`.
    pub target: String,
    /// `TargetMode="External"`: the target is outside the package and never followed here.
    pub external: bool,
}
// one <Relationship>, checked against the ids before it
fn relationship(node: &Node, ids: &mut BTreeSet<String>, rules: &Rules) -> Result<Relationship, Error> {
    if !node.is(NS_RELS, "Relationship")
        || !node.text.is_empty()
        || !node.children.is_empty()
        || node.attrs.len() < 3
        || node.attrs.len() > 4
        || node.attrs.iter().any(|(n, _)| {
            !n.ns.is_empty() || !matches!(n.local.as_str(), "Id" | "Type" | "Target" | "TargetMode")
        })
    {
        return Err(Error::Invalid);
    }
    let id = node.attr("", "Id").ok_or(Error::Invalid)?;
    let kind = node.attr("", "Type").ok_or(Error::Invalid)?;
    let target = node.attr("", "Target").ok_or(Error::Invalid)?;
    if id.is_empty() || kind.is_empty() || target.is_empty() || !ids.insert(String::from(id)) {
        return Err(Error::Invalid);
    }
    if !rules.macros && macro_type(kind) {
        return Err(Error::MacroRelationship);
    }
    let external = match node.attr("", "TargetMode") {
        None | Some("Internal") => false,
        Some("External") => true,
        _ => return Err(Error::Invalid),
    };
    Ok(Relationship {
        id: String::from(id),
        kind: String::from(kind),
        target: String::from(target),
        external,
    })
}
/// The relationships of one relationship part, checked but not resolved: the root element,
/// each relationship's attributes, unique ids, the target mode, macro types.
pub fn parse_relationships(xml: &[u8], rules: Rules) -> Result<Vec<Relationship>, Error> {
    let root = parse(xml, &rules)?;
    container(&root, NS_RELS, "Relationships")?;
    let mut ids = BTreeSet::new();
    root.children.iter().map(|n| relationship(n, &mut ids, &rules)).collect()
}
/// A relationship part with an XML declaration.
pub fn relationships_xml(list: &[Relationship]) -> Result<String, Error> {
    let mut s = alloc::format!("{DECL}<Relationships xmlns=\"{NS_RELS}\">");
    for r in list {
        s.push_str(&alloc::format!(
            "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"{}/>",
            esc(&r.id)?,
            esc(&r.kind)?,
            esc(&r.target)?,
            if r.external { " TargetMode=\"External\"" } else { "" }
        ));
    }
    s.push_str("</Relationships>");
    Ok(s)
}

// one ZIP item while a package is checked
struct View<'a> {
    name: &'a str,
    data: &'a [u8],
}
// what a check finds out: part keys -> index, the content types
struct Checked {
    index: BTreeMap<String, usize>,
    types: ContentTypes,
}
// Every rule a package has to meet, in the order doc_io checked DOCX packages: the names, the
// content types (each part has one), the relationship parts (each source exists, each internal
// target names a part, at most one main part).
fn check(items: &[View], rules: &Rules) -> Result<Checked, Error> {
    let exact = rules.exact_names;
    let mut index = BTreeMap::new();
    for (i, item) in items.iter().enumerate() {
        if exact && item.name.contains('%') {
            return Err(Error::Address);
        }
        if index.insert(key(item.name, exact)?, i).is_some() {
            return Err(Error::Invalid);
        }
    }
    let is_types = |name: &str| name == CONTENT_TYPES || (!exact && name.eq_ignore_ascii_case(CONTENT_TYPES));
    // the index of the part a name refers to (exact: the very name)
    let find = |name: &str| -> Option<usize> {
        let i = *index.get(&key(name, exact).ok()?)?;
        (!exact || items[i].name == name).then_some(i)
    };
    let types_item = items.iter().find(|v| is_types(v.name)).ok_or(Error::Invalid)?;
    let root = parse(types_item.data, rules)?;
    container(&root, NS_TYPES, "Types")?;
    let mut types = ContentTypes { exact, ..ContentTypes::default() };
    for node in &root.children {
        if !node.children.is_empty() || !node.text.is_empty() || node.attrs.len() != 2 {
            return Err(Error::Invalid);
        }
        let value = node.attr("", "ContentType").ok_or(Error::Invalid)?;
        if value.is_empty() || (!rules.macros && macro_type(value)) {
            return Err(Error::ContentType);
        }
        if node.is(NS_TYPES, "Default") {
            let ext = node.attr("", "Extension").ok_or(Error::Invalid)?;
            let k = types.ext_key(ext)?;
            if types.defaults.iter().any(|(_, e, _)| *e == k) {
                return Err(Error::Invalid);
            }
            types.defaults.push((String::from(ext), k, String::from(value)));
        } else if node.is(NS_TYPES, "Override") {
            let name = node
                .attr("", "PartName")
                .and_then(|n| n.strip_prefix('/'))
                .ok_or(Error::Invalid)?;
            let k = types.name_key(name)?;
            if types.index.contains_key(&k) || find(name).is_none() {
                return Err(Error::Invalid);
            }
            types.index.insert(k, types.overrides.len());
            types.overrides.push((String::from(name), String::from(value)));
        } else {
            return Err(Error::Invalid);
        }
    }
    for item in items {
        if is_types(item.name) {
            continue;
        }
        if types.get(item.name).is_none() {
            return Err(Error::Invalid);
        }
        if !rules.macros && macro_part(item.name) {
            return Err(Error::MacroPart);
        }
    }
    find(PACKAGE_RELS).ok_or(Error::Invalid)?;
    let mut main = false;
    for item in items {
        let k = if is_types(item.name) { continue } else { key(item.name, exact)? };
        let named = if exact { item.name } else { k.as_str() };
        if !named.ends_with(".rels") {
            continue;
        }
        if types.get(item.name) != Some(RELS_TYPE) {
            return Err(Error::Invalid);
        }
        let source = rels_source(named)?;
        if !source.is_empty() && find(&source).is_none() {
            return Err(Error::Invalid);
        }
        let root = parse(item.data, rules)?;
        container(&root, NS_RELS, "Relationships")?;
        let mut ids = BTreeSet::new();
        for node in &root.children {
            let r = relationship(node, &mut ids, rules)?;
            let office = r.kind == OFFICE_DOCUMENT || r.kind == OFFICE_DOCUMENT_STRICT;
            // at most one main part, inside the package
            if office && source.is_empty() {
                if r.external || main {
                    return Err(Error::Invalid);
                }
                main = true;
            }
            if !r.external {
                let part = rules.resolve(&source, &r.target)?;
                find(&part).ok_or(Error::Invalid)?;
            }
        }
    }
    Ok(Checked { index, types })
}

/// An OPC package: every ZIP item in archive order with its bytes, the content types and the
/// main part. Parts this crate does not understand are kept as they are; `write` puts them back
/// unchanged, `[Content_Types].xml` too unless the content types changed.
#[derive(Clone, Debug)]
pub struct Package {
    entries: Vec<Entry>,
    index: BTreeMap<String, usize>,
    types: ContentTypes,
    types_changed: bool,
    rules: Rules,
}

impl Package {
    /// An empty package to fill: only `[Content_Types].xml`. `write` needs the package's
    /// relationships (`set_relationships("", ..)`) and a content type for every part.
    pub fn new(rules: Rules) -> Package {
        let mut index = BTreeMap::new();
        index.insert(CONTENT_TYPES.to_ascii_lowercase(), 0);
        Package {
            entries: alloc::vec![Entry { name: String::from(CONTENT_TYPES), data: Vec::new() }],
            index,
            types: ContentTypes { exact: rules.exact_names, ..ContentTypes::default() },
            types_changed: true,
            rules,
        }
    }
    /// Read and check a package within the ZIP `limits` (percent-escapes in ZIP names only
    /// without `Rules::exact_names`).
    pub fn read(bytes: &[u8], limits: Limits, rules: Rules) -> Result<Package, Error> {
        let (entries, _) = crate::read_parts(bytes, limits, !rules.exact_names).map_err(Error::Zip)?;
        Self::from_entries(entries, rules)
    }
    /// Check the ZIP items of a package already read.
    pub fn from_entries(entries: Vec<Entry>, rules: Rules) -> Result<Package, Error> {
        let views: Vec<View> = entries.iter().map(|e| View { name: &e.name, data: &e.data }).collect();
        let c = check(&views, &rules)?;
        Ok(Package { entries, index: c.index, types: c.types, types_changed: false, rules })
    }
    /// Every ZIP item in archive order, `[Content_Types].xml` included as it was read (or last
    /// written); parts added later at the end.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    /// The rules the package was read with.
    pub fn rules(&self) -> Rules {
        self.rules
    }
    /// The stored name of the part the package's internal `officeDocument` relationship names
    /// (its main part, `word/document.xml` or `visio/document.xml`).
    pub fn main_part(&self) -> Option<&str> {
        self.relationships("")
            .ok()?
            .iter()
            .find(|r| !r.external && (r.kind == OFFICE_DOCUMENT || r.kind == OFFICE_DOCUMENT_STRICT))
            .and_then(|r| self.resolve("", &r.target).ok())
    }
    /// The content types.
    pub fn content_types(&self) -> &ContentTypes {
        &self.types
    }
    /// The content types to change; `write` checks the result.
    pub fn content_types_mut(&mut self) -> &mut ContentTypes {
        self.types_changed = true;
        &mut self.types
    }
    /// The content type of a part.
    pub fn content_type(&self, name: &str) -> Option<&str> {
        self.types.get(name)
    }
    fn find(&self, name: &str) -> Option<usize> {
        let exact = self.rules.exact_names;
        let i = *self.index.get(&key(name, exact).ok()?)?;
        (!exact || self.entries[i].name == name).then_some(i)
    }
    /// The stored name of the part a name refers to.
    pub fn name_of(&self, name: &str) -> Option<&str> {
        self.find(name).map(|i| self.entries[i].name.as_str())
    }
    /// The bytes of a part (any equivalent name finds it).
    pub fn part(&self, name: &str) -> Option<&[u8]> {
        self.find(name).map(|i| self.entries[i].data.as_slice())
    }
    /// The relationships of a part (`""` for the package's own); empty when it has none.
    pub fn relationships(&self, source: &str) -> Result<Vec<Relationship>, Error> {
        match self.part(&rels_name(source)) {
            Some(xml) => parse_relationships(xml, self.rules),
            None => Ok(Vec::new()),
        }
    }
    /// The stored name of the part an internal target of `source` names.
    pub fn resolve(&self, source: &str, target: &str) -> Result<&str, Error> {
        let part = self.rules.resolve(source, target)?;
        self.name_of(&part).ok_or(Error::Invalid)
    }
    /// Add a part or replace its bytes (an existing part keeps its stored name), and make
    /// `content_type` its content type: the default of its extension when that matches,
    /// otherwise an override.
    pub fn set_part(&mut self, name: &str, content_type: &str, data: Vec<u8>) -> Result<(), Error> {
        let name = name.strip_prefix('/').unwrap_or(name);
        if content_type.is_empty() {
            return Err(Error::ContentType);
        }
        if name.eq_ignore_ascii_case(CONTENT_TYPES) || !crate::name_ok(name, !self.rules.exact_names) {
            return Err(Error::Address);
        }
        let k = self.types.name_key(name)?;
        match self.find(name) {
            Some(i) => self.entries[i].data = data,
            None => {
                if self.index.contains_key(&k) {
                    return Err(Error::Invalid);
                }
                self.index.insert(k.clone(), self.entries.len());
                self.entries.push(Entry { name: String::from(name), data });
            }
        }
        if self.types.get(name) != Some(content_type) {
            if self.types.default_of(&k) == Some(content_type) {
                self.types.remove_override(name);
            } else {
                self.types.set_override(name, content_type)?;
            }
            self.types_changed = true;
        }
        Ok(())
    }
    /// Replace the bytes of an existing part, keeping its content type.
    pub fn replace(&mut self, name: &str, data: Vec<u8>) -> Result<(), Error> {
        let i = self.find(name).ok_or(Error::Invalid)?;
        if i == self.types_index() {
            return Err(Error::Address);
        }
        self.entries[i].data = data;
        Ok(())
    }
    /// Remove a part with its override and its own relationship part; false when there is no
    /// such part. Relationships of other parts that name it are the caller's to change.
    pub fn remove_part(&mut self, name: &str) -> bool {
        let Some(i) = self.find(name) else { return false };
        if i == self.types_index() {
            return false;
        }
        let stored = self.entries[i].name.clone();
        self.entries.remove(i);
        if self.types.remove_override(&stored) {
            self.types_changed = true;
        }
        self.reindex();
        if !stored.ends_with(".rels") {
            self.remove_part(&rels_name(&stored));
        }
        true
    }
    /// Replace the relationships of a part (`""`: the package's); an empty list removes its
    /// relationship part. Adds the default for `.rels` when there is none.
    pub fn set_relationships(&mut self, source: &str, list: &[Relationship]) -> Result<(), Error> {
        let name = rels_name(source);
        if list.is_empty() {
            self.remove_part(&name);
        } else {
            if self.types.defaults().all(|(e, _)| !e.eq_ignore_ascii_case("rels")) {
                self.types.set_default("rels", RELS_TYPE)?;
                self.types_changed = true;
            }
            self.set_part(&name, RELS_TYPE, relationships_xml(list)?.into_bytes())?;
        }
        Ok(())
    }
    fn types_index(&self) -> usize {
        let exact = self.rules.exact_names;
        self.entries
            .iter()
            .position(|e| e.name == CONTENT_TYPES || (!exact && e.name.eq_ignore_ascii_case(CONTENT_TYPES)))
            .unwrap_or(usize::MAX)
    }
    fn reindex(&mut self) {
        let exact = self.rules.exact_names;
        self.index = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| key(&e.name, exact).ok().map(|k| (k, i)))
            .collect();
    }
    /// The package as a ZIP archive, after checking it as `read` would: every part in its
    /// order, `[Content_Types].xml` new when the content types changed. Deterministic: the same
    /// package always gives the same bytes.
    pub fn write(&self, limits: Limits, method: Method) -> Result<Vec<u8>, Error> {
        let types_xml = if self.types_changed { Some(self.types.to_xml()?.into_bytes()) } else { None };
        let t = self.types_index();
        let views: Vec<View> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| View {
                name: &e.name,
                data: if i == t { types_xml.as_deref().unwrap_or(&e.data) } else { &e.data },
            })
            .collect();
        check(&views, &self.rules)?;
        let parts: Vec<(&str, &[u8])> = views.iter().map(|v| (v.name, v.data)).collect();
        crate::write_parts(&parts, limits, method, !self.rules.exact_names, 0xFFFF_FFFF).map_err(Error::Zip)
    }
}
