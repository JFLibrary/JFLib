// props -- the document properties of a drawing (DF-09): docProps/core.xml, app.xml, custom.xml
//
//   props::read(&doc) -> Props          what the three parts say ("" where an entry is missing)
//   props::write(&mut doc, &props)      the parts made to say it: a part is only touched when one
//                                       of its own entries changed, so an unchanged file keeps its
//                                       bytes; a missing part is created, a custom part without
//                                       entries is removed
//   Custom { name, kind, value }        a custom property: kind is the vt element ("lpwstr",
//                                       "i4", "r8", "bool", "filetime" ...), value its text
//
// The parts are found by their content type (OPC names them freely).  core.xml is written anew
// from Props in the order Office writes it; app.xml keeps everything it had (Visio's page and
// master lists, the version) and only gets the four entries Props names (Template, Company,
// Manager, HyperlinkBase) replaced or added; custom.xml is written anew from the list.  Dates
// stay the W3CDTF text the file had ("2026-11-14T18:04:59Z"); nothing here reads a clock.
// Reading is bounded like every part (vsdx::Limits::VSDX's XML bounds) and never fails: a part
// that is no XML reads as empty.

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{Document, Part};

use crate::xml::{self, Event, Reader};

/// the content type of docProps/core.xml
pub const CT_CORE: &str = "application/vnd.openxmlformats-package.core-properties+xml";
/// the content type of docProps/app.xml
pub const CT_APP: &str = "application/vnd.openxmlformats-officedocument.extended-properties+xml";
/// the content type of docProps/custom.xml
pub const CT_CUSTOM: &str = "application/vnd.openxmlformats-officedocument.custom-properties+xml";
const REL_CORE: &str = "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
const REL_APP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";
const REL_CUSTOM: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties";
const NS_CORE: &str = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties";
const NS_APP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties";
const NS_CUSTOM: &str = "http://schemas.openxmlformats.org/officeDocument/2006/custom-properties";
const NS_VT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes";
/// the format id Office gives every custom property
const FMTID: &str = "{D5CDD505-2E9C-101B-9397-08002B2CF9AE}";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";

/// one custom property
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Custom {
    /// its name
    pub name: String,
    /// the vt element of its value: "lpwstr", "i4", "r8", "bool", "filetime", "date" ...
    pub kind: String,
    /// the value as text
    pub value: String,
}

/// the document properties; "" where the file has none
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Props {
    /// Titel (dc:title)
    pub title: String,
    /// Thema (dc:subject)
    pub subject: String,
    /// Autor (dc:creator)
    pub creator: String,
    /// Stichwörter (cp:keywords)
    pub keywords: String,
    /// Kommentare (dc:description)
    pub description: String,
    /// Kategorie (cp:category)
    pub category: String,
    /// Sprache (dc:language)
    pub language: String,
    /// zuletzt geändert von (cp:lastModifiedBy)
    pub last_modified_by: String,
    /// Status (cp:contentStatus)
    pub content_status: String,
    /// Revision (cp:revision)
    pub revision: String,
    /// erstellt (dcterms:created, W3CDTF)
    pub created: String,
    /// geändert (dcterms:modified, W3CDTF)
    pub modified: String,
    /// zuletzt gedruckt (cp:lastPrinted, W3CDTF)
    pub last_printed: String,
    /// Vorlage (app.xml Template)
    pub template: String,
    /// Firma (app.xml Company)
    pub company: String,
    /// Manager (app.xml Manager)
    pub manager: String,
    /// Hyperlinkbasis (app.xml HyperlinkBase)
    pub hyperlink_base: String,
    /// the program that wrote the file (app.xml Application; read only)
    pub application: String,
    /// the custom properties in file order
    pub custom: Vec<Custom>,
}

// core.xml: (field, prefix:name) in the order Office writes them
type Get = fn(&Props) -> &String;
type Set = fn(&mut Props) -> &mut String;
const CORE: [(&str, &str, Get, Set); 13] = [
    ("dc", "title", |p| &p.title, |p| &mut p.title),
    ("dc", "subject", |p| &p.subject, |p| &mut p.subject),
    ("dc", "creator", |p| &p.creator, |p| &mut p.creator),
    ("cp", "keywords", |p| &p.keywords, |p| &mut p.keywords),
    ("dc", "description", |p| &p.description, |p| &mut p.description),
    ("cp", "lastModifiedBy", |p| &p.last_modified_by, |p| &mut p.last_modified_by),
    ("cp", "revision", |p| &p.revision, |p| &mut p.revision),
    ("cp", "lastPrinted", |p| &p.last_printed, |p| &mut p.last_printed),
    ("dcterms", "created", |p| &p.created, |p| &mut p.created),
    ("dcterms", "modified", |p| &p.modified, |p| &mut p.modified),
    ("cp", "category", |p| &p.category, |p| &mut p.category),
    ("cp", "contentStatus", |p| &p.content_status, |p| &mut p.content_status),
    ("dc", "language", |p| &p.language, |p| &mut p.language),
];

// app.xml entries Props writes
const APP: [(&str, Get, Set); 4] = [
    ("Template", |p| &p.template, |p| &mut p.template),
    ("Manager", |p| &p.manager, |p| &mut p.manager),
    ("Company", |p| &p.company, |p| &mut p.company),
    ("HyperlinkBase", |p| &p.hyperlink_base, |p| &mut p.hyperlink_base),
];

fn part_of(doc: &Document, ct: &str) -> Option<usize> {
    doc.parts.iter().position(|p| doc.str(p.content_type) == ct)
}

// the children of a part's root: (local name, text, first child element's local name, the
// offsets of the child element [start, end)); None when the part is no XML this reads
struct Child {
    local: String,
    text: String,
    attrs: Vec<(String, String)>,
    inner: Option<(String, String)>,
    at: usize,
    end: usize,
}

fn children(data: &[u8]) -> Option<(String, Vec<Child>)> {
    let text = xml::text_of(data)?;
    let lim = crate::vsdx::Limits::VSDX;
    let mut r = Reader::new(&text, lim.depth, lim.attrs);
    let root = match r.next().ok()? {
        Event::Start(s) => s,
        _ => return None,
    };
    let mut out = Vec::new();
    if !root.empty {
        loop {
            match r.next().ok()? {
                Event::Start(c) => {
                    let mut ch = Child { local: String::from(c.local), text: String::new(), attrs: Vec::new(), inner: None, at: c.at, end: c.end };
                    for a in &c.attrs {
                        if a.ns != xml::Ns::Decl {
                            ch.attrs.push((String::from(a.local), String::from(a.value.as_ref())));
                        }
                    }
                    let mut depth = 0usize;
                    loop {
                        match r.next().ok()? {
                            Event::Start(g) => {
                                if depth == 0 && ch.inner.is_none() {
                                    ch.inner = Some((String::from(g.local), String::new()));
                                }
                                depth += 1;
                            }
                            Event::Text(t) => match (&mut ch.inner, depth) {
                                (_, 0) => ch.text.push_str(&t),
                                (Some(i), 1) => i.1.push_str(&t),
                                _ => {}
                            },
                            Event::End { end } => {
                                if depth == 0 {
                                    ch.end = end;
                                    break;
                                }
                                depth -= 1;
                            }
                            Event::Eof => return None,
                        }
                    }
                    out.push(ch);
                }
                Event::Text(_) => {}
                Event::End { .. } => break,
                Event::Eof => return None,
            }
        }
    }
    Some((String::from(root.local), out))
}

/// what the document's property parts say
pub fn read(doc: &Document) -> Props {
    let mut p = Props::default();
    if let Some((_, list)) = part_of(doc, CT_CORE).and_then(|i| children(&doc.parts[i].data)) {
        for c in list {
            if let Some(e) = CORE.iter().find(|e| e.1 == c.local) {
                *(e.3)(&mut p) = c.text;
            }
        }
    }
    if let Some((_, list)) = part_of(doc, CT_APP).and_then(|i| children(&doc.parts[i].data)) {
        for c in list {
            if let Some(e) = APP.iter().find(|e| e.0 == c.local) {
                *(e.2)(&mut p) = c.text;
            } else if c.local == "Application" {
                p.application = c.text;
            }
        }
    }
    if let Some((_, list)) = part_of(doc, CT_CUSTOM).and_then(|i| children(&doc.parts[i].data)) {
        for c in list.into_iter().filter(|c| c.local == "property") {
            let name = c.attrs.iter().find(|a| a.0 == "name").map(|a| a.1.clone()).unwrap_or_default();
            let (kind, value) = c.inner.unwrap_or_default();
            p.custom.push(Custom { name, kind, value });
        }
    }
    p
}

/// XML text with &, <, > and quotes escaped (and the characters XML does not allow left out)
pub(crate) fn escape(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\r' => out.push_str("&#13;"),
            '\t' | '\n' => out.push(ch),
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => {}
            c => out.push(c),
        }
    }
}

fn core_xml(p: &Props) -> Vec<u8> {
    let mut s = String::from(DECL);
    s.push_str("<cp:coreProperties xmlns:cp=\"");
    s.push_str(NS_CORE);
    s.push_str("\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">");
    for (prefix, name, get, _) in CORE {
        let v = get(p);
        if v.is_empty() {
            continue;
        }
        let w3c = prefix == "dcterms";
        s.push('<');
        s.push_str(prefix);
        s.push(':');
        s.push_str(name);
        if w3c {
            s.push_str(" xsi:type=\"dcterms:W3CDTF\"");
        }
        s.push('>');
        escape(v, &mut s);
        s.push_str("</");
        s.push_str(prefix);
        s.push(':');
        s.push_str(name);
        s.push('>');
    }
    s.push_str("</cp:coreProperties>");
    s.into_bytes()
}

fn custom_xml(list: &[Custom]) -> Vec<u8> {
    let mut s = String::from(DECL);
    s.push_str("<Properties xmlns=\"");
    s.push_str(NS_CUSTOM);
    s.push_str("\" xmlns:vt=\"");
    s.push_str(NS_VT);
    s.push_str("\">");
    for (k, c) in list.iter().enumerate() {
        // a kind that is no plain name is written as text
        let kind = if !c.kind.is_empty() && c.kind.bytes().all(|b| b.is_ascii_alphanumeric()) { c.kind.as_str() } else { "lpwstr" };
        s.push_str("<property fmtid=\"");
        s.push_str(FMTID);
        s.push_str(&alloc::format!("\" pid=\"{}\" name=\"", k + 2));
        escape(&c.name, &mut s);
        s.push_str("\"><vt:");
        s.push_str(kind);
        s.push('>');
        escape(&c.value, &mut s);
        s.push_str("</vt:");
        s.push_str(kind);
        s.push_str("></property>");
    }
    s.push_str("</Properties>");
    s.into_bytes()
}

// app.xml with the four entries of Props: replaced where the part has them, added before the
// root's end otherwise (an empty value removes nothing: the element stays, empty)
fn app_xml(old: Option<&[u8]>, p: &Props) -> Vec<u8> {
    let parsed = old.and_then(|d| xml::text_of(d).map(|t| String::from(t.as_ref()))).and_then(|t| children(t.as_bytes()).map(|c| (t, c)));
    let Some((text, (_, list))) = parsed else {
        let mut s = String::from(DECL);
        s.push_str("<Properties xmlns=\"");
        s.push_str(NS_APP);
        s.push_str("\" xmlns:vt=\"");
        s.push_str(NS_VT);
        s.push_str("\">");
        if !p.application.is_empty() {
            s.push_str("<Application>");
            escape(&p.application, &mut s);
            s.push_str("</Application>");
        }
        for (name, get, _) in APP {
            if !get(p).is_empty() {
                element(&mut s, name, get(p));
            }
        }
        s.push_str("</Properties>");
        return s.into_bytes();
    };
    // the edits by offset: (from, to, new text), applied back to front
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut added = String::new();
    for (name, get, _) in APP {
        let v = get(p);
        match list.iter().find(|c| c.local == name) {
            Some(c) if c.text != *v && c.inner.is_none() => {
                let mut e = String::new();
                element(&mut e, name, v);
                edits.push((c.at, c.end, e));
            }
            Some(_) => {}
            None if !v.is_empty() => element(&mut added, name, v),
            None => {}
        }
    }
    let mut out = text;
    if !added.is_empty() {
        if let Some(close) = out.rfind("</") {
            edits.push((close, close, added));
        }
    }
    edits.sort_by_key(|e| core::cmp::Reverse(e.0));
    for (from, to, new) in edits {
        if from <= to && to <= out.len() && out.is_char_boundary(from) && out.is_char_boundary(to) {
            out.replace_range(from..to, &new);
        }
    }
    out.into_bytes()
}

fn element(s: &mut String, name: &str, v: &str) {
    s.push('<');
    s.push_str(name);
    s.push('>');
    escape(v, s);
    s.push_str("</");
    s.push_str(name);
    s.push('>');
}

fn put(doc: &mut Document, ct: &str, name: &str, rel: &str, data: Vec<u8>) {
    match part_of(doc, ct) {
        Some(i) => doc.parts[i].data = data,
        None => {
            let part = Part { name: doc.intern(name), content_type: doc.intern(ct), rel_type: doc.intern(rel), data };
            doc.parts.push(part);
        }
    }
}

/// make the property parts say `p` (see the module comment); parts whose entries did not change
/// keep their bytes
pub fn write(doc: &mut Document, p: &Props) {
    let old = read(doc);
    let core_changed = CORE.iter().any(|e| (e.2)(&old) != (e.2)(p));
    if core_changed {
        put(doc, CT_CORE, "/docProps/core.xml", REL_CORE, core_xml(p));
    }
    let app_changed = APP.iter().any(|e| (e.1)(&old) != (e.1)(p));
    if app_changed {
        let data = app_xml(part_of(doc, CT_APP).map(|i| doc.parts[i].data.as_slice()), p);
        put(doc, CT_APP, "/docProps/app.xml", REL_APP, data);
    }
    if old.custom != p.custom {
        if p.custom.is_empty() {
            if let Some(i) = part_of(doc, CT_CUSTOM) {
                doc.parts.remove(i);
            }
        } else {
            put(doc, CT_CUSTOM, "/docProps/custom.xml", REL_CUSTOM, custom_xml(&p.custom));
        }
    }
}
