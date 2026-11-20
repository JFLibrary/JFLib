// pack -- a Document as a VSDX package: part names, relationships, content types (vsdx::write)
//
// Part names: every part keeps the name it was read with (Page::part, Master::part,
// Theme::part, Part::name; the main, pages and masters parts as doc.rels names them).  A page or
// master without one gets the first free visio/pages/pageN.xml / visio/masters/masterN.xml; a
// name that is no valid part name, or that an earlier part already has, counts as none.  The
// parts kept as bytes (Part) must have valid names of their own.  A page gets a part when it
// had one, has contents, or is a page of a drawing or template (a stencil's pages have none,
// as Visio writes them); every master gets one.
//
// Relationships: every relationship of doc.rels whose source part and internal target are
// still written is kept with its id, type and target (raw XML in the extras can name the id;
// external targets are kept as text and never followed).  The ones the model owns follow the
// model: package -> main part, main -> pages and masters, pages -> each page part, masters ->
// each master part, a page part -> its theme (Page::theme); a kept one that no longer fits is
// left out.  Missing ones are added with the first free rIdN of their source: those, the
// relationships of a page or master part to the masters its shapes are instances of and to the
// pictures its ForeignData names, a theme no relationship names (from the main part), and a
// Part whose rel_type says what names it when nothing does (document properties and the
// thumbnail from the package, Visio's own kinds from the main part).  New relationships use
// Visio's types (the main part's is …/visio/2010/relationships/document).
//
// Desktop Visio refuses a file without a windows part (error 271; [MS-VSDX] calls the part
// unused): a document without one gets visio/windows.xml of its own (blank::windows_xml), named
// from the main part, written after the other parts -- reading the file keeps it as a Part.
//
// Order (deterministic): [Content_Types].xml, the relationship parts in the order doc.rels
// names their sources (new sources after), the main part, pages, page parts, masters, master
// parts, themes, then the Parts in model order.  Reading such a file gives the model back in
// the same order, so writing it again gives the same bytes.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use archive::opc::{self, Package, Rules};
use archive::Method;
use diagram_core::{DocKind, Document, ForeignPayload, MasterRef, Owner, ShapeTree, Sym};

use crate::blank;
use crate::vsdx::{self, Format, WriteError, WriteOptions, CT_MASTER, CT_MASTERS, CT_PAGE, CT_PAGES, CT_THEME, CT_WINDOWS};
use crate::write;

const VISIO_RELS: &str = "http://schemas.microsoft.com/visio/2010/relationships/";
const REL_DOCUMENT: &str = "http://schemas.microsoft.com/visio/2010/relationships/document";
const REL_PAGES: &str = "http://schemas.microsoft.com/visio/2010/relationships/pages";
const REL_PAGE: &str = "http://schemas.microsoft.com/visio/2010/relationships/page";
const REL_MASTERS: &str = "http://schemas.microsoft.com/visio/2010/relationships/masters";
const REL_MASTER: &str = "http://schemas.microsoft.com/visio/2010/relationships/master";
const REL_THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const REL_WINDOWS: &str = "http://schemas.microsoft.com/visio/2010/relationships/windows";
// relationship kinds whose source is the package itself
const PACKAGE_KINDS: [&str; 4] = [
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties",
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail",
];

// what a written part is
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Role {
    Main,
    Pages,
    Masters,
    Page(usize),
    Master(usize),
    Theme(usize),
    Part(usize),
    Windows,
}

// one relationship to write
struct Out {
    id: String,
    kind: String,
    target: String,
    external: bool,
    // the key of the target part (internal ones)
    to: Option<String>,
}

// the comparison key of a part name (OPC: canonical, ASCII case folded); None: no name a
// package takes (archive::opc refuses '?', '#', ':', '\', control characters, more than 1024
// bytes, empty, "." and ".." segments, the content types item and relationship parts)
fn key(name: &str) -> Option<String> {
    if name.len() > 1024 || bare(name).eq_ignore_ascii_case(opc::CONTENT_TYPES) || name.contains(['?', '#', ':', '\\']) || name.chars().any(char::is_control) {
        return None;
    }
    let n = opc::normalize(name).ok()?;
    let lower = n.to_ascii_lowercase();
    (!lower.ends_with(".rels") && lower != "[content_types].xml").then_some(lower)
}

fn bare(name: &str) -> &str {
    name.strip_prefix('/').unwrap_or(name)
}

fn is_main_kind(kind: &str) -> bool {
    vsdx::rel_is(kind, "document") || kind == opc::OFFICE_DOCUMENT || kind == opc::OFFICE_DOCUMENT_STRICT
}

// a target relative to the source part (the package: from its root)
fn relative(source: &str, target: &str) -> String {
    let t = opc::normalize(target).unwrap_or_else(|_| String::from(target));
    let dir: Vec<&str> = match source.rsplit_once('/') {
        Some((d, _)) if !source.is_empty() => d.split('/').collect(),
        _ => Vec::new(),
    };
    let segs: Vec<&str> = t.split('/').collect();
    let folders = segs.len().saturating_sub(1);
    let mut common = 0;
    while common < dir.len() && common < folders && dir[common] == segs[common] {
        common += 1;
    }
    let mut out = String::new();
    for _ in common..dir.len() {
        out.push_str("../");
    }
    out.push_str(&segs[common..].join("/"));
    out
}

struct Plan<'d> {
    doc: &'d Document,
    kind: DocKind,
    // key -> (stored name without '/', role)
    parts: BTreeMap<String, (String, Role)>,
    main: String,
    pages: Option<String>,
    masters: Option<String>,
    page_parts: Vec<Option<String>>,
    master_parts: Vec<Option<String>>,
    theme_parts: Vec<String>,
    // a windows part of its own when the document has none
    windows: Option<String>,
    // relationships by source key ("" = the package), sources in output order
    order: Vec<String>,
    rels: BTreeMap<String, Vec<Out>>,
    used: BTreeMap<String, BTreeSet<String>>,
    next_id: BTreeMap<String, u32>,
    // (source, target) -> the positions of its relationships in rels[source]; every target
    links: BTreeMap<(String, String), Vec<usize>>,
    targets: BTreeSet<String>,
    // the masters in document order
    master_index: BTreeMap<MasterRef, usize>,
}

impl<'d> Plan<'d> {
    fn claim(&mut self, name: &str, role: Role) -> bool {
        let Some(k) = key(name) else { return false };
        if self.parts.contains_key(&k) {
            return false;
        }
        self.parts.insert(k, (String::from(name), role));
        true
    }

    // `first` when it is free, else dir/stemN.xml with the smallest free N
    fn fresh(&mut self, first: Option<&str>, dir: &str, stem: &str, role: Role) -> String {
        if let Some(f) = first {
            if self.claim(f, role) {
                return String::from(f);
            }
        }
        let mut n = 1u64;
        loop {
            let name = format!("{dir}/{stem}{n}.xml");
            if self.claim(&name, role) {
                return name;
            }
            n += 1;
        }
    }

    fn name_of(&self, k: &str) -> &str {
        if k.is_empty() {
            return "";
        }
        self.parts.get(k).map_or("", |(n, _)| n.as_str())
    }

    fn role_of(&self, k: &str) -> Option<Role> {
        self.parts.get(k).map(|(_, r)| *r)
    }

    // the part a relationship of doc.rels names as source or target (None: not written)
    fn written(&self, name: Sym) -> Option<String> {
        let k = key(bare(self.doc.str(name)))?;
        self.parts.contains_key(&k).then_some(k)
    }

    fn new_id(&mut self, src: &str) -> String {
        let used = self.used.entry(String::from(src)).or_default();
        let next = self.next_id.entry(String::from(src)).or_insert(1);
        loop {
            let id = format!("rId{next}");
            *next = next.saturating_add(1);
            if used.insert(id.clone()) {
                return id;
            }
        }
    }

    fn push(&mut self, src: &str, out: Out) {
        if !self.rels.contains_key(src) {
            self.order.push(String::from(src));
        }
        let list = self.rels.entry(String::from(src)).or_default();
        if let Some(to) = &out.to {
            self.links.entry((String::from(src), to.clone())).or_default().push(list.len());
            self.targets.insert(to.clone());
        }
        list.push(out);
    }

    // the first internal relationship from `src` to `to` (of kind `name` when given)
    fn find(&self, src: &str, to: &str, name: Option<&str>) -> Option<&Out> {
        let list = self.rels.get(src)?;
        let at = self.links.get(&(String::from(src), String::from(to)))?;
        at.iter().filter_map(|&i| list.get(i)).find(|r| name.is_none_or(|n| vsdx::rel_is(&r.kind, n)))
    }

    fn has(&self, src: &str, to: &str, name: Option<&str>) -> bool {
        self.find(src, to, name).is_some()
    }

    fn targeted(&self, to: &str) -> bool {
        self.targets.contains(to)
    }

    // a relationship from `src` to the part `to` unless one of kind `name` (any kind: None) is there
    fn ensure(&mut self, src: &str, to: &str, name: Option<&str>, kind: &str) {
        if self.has(src, to, name) {
            return;
        }
        let id = self.new_id(src);
        let target = relative(self.name_of(src), self.name_of(to));
        self.push(src, Out { id, kind: String::from(kind), target, external: false, to: Some(String::from(to)) });
    }

    // the names of every part
    fn names(&mut self, format: Format) -> Result<(), WriteError> {
        let doc = self.doc;
        // the main part, the pages and masters parts as doc.rels names them
        let main_rel = doc.rels.iter().find(|r| r.source == Sym::EMPTY && !r.external && is_main_kind(doc.str(r.kind)));
        let main_name = main_rel.and_then(|r| r.part).map(|p| String::from(bare(doc.str(p))));
        self.main = self.fresh(Some(main_name.as_deref().unwrap_or("visio/document.xml")), "visio", "document", Role::Main);
        let main_key = key(&self.main).unwrap_or_default();
        let listed = |name: &str| {
            doc.rels
                .iter()
                .find(|r| !r.external && vsdx::rel_is(doc.str(r.kind), name) && key(bare(doc.str(r.source))).as_deref() == Some(main_key.as_str()))
                .and_then(|r| r.part)
                .map(|p| String::from(bare(doc.str(p))))
        };
        // the parts kept as bytes: their own names or nothing (before the pages and masters
        // lists, whose names doc.rels gives and a stale relationship could give wrong)
        for (i, p) in doc.parts.iter().enumerate() {
            let name = bare(doc.str(p.name));
            if !self.claim(name, Role::Part(i)) {
                return Err(WriteError::Part { part: format!("/{name}"), what: "a part name that is not valid or taken twice" });
            }
            if doc.str(p.content_type).is_empty() {
                return Err(WriteError::Part { part: format!("/{name}"), what: "a part without content type" });
            }
        }
        if doc.page_count() > 0 || !doc.pages_extras.is_empty() {
            let free = |n: &String| key(n).is_some_and(|k| !self.parts.contains_key(&k));
            let n = listed("pages").filter(free).unwrap_or_else(|| String::from("visio/pages/pages.xml"));
            self.pages = Some(self.fresh(Some(&n), "visio/pages", "pages", Role::Pages));
        }
        if doc.masters().next().is_some() || !doc.masters_extras.is_empty() {
            let free = |n: &String| key(n).is_some_and(|k| !self.parts.contains_key(&k));
            let n = listed("masters").filter(free).unwrap_or_else(|| String::from("visio/masters/masters.xml"));
            self.masters = Some(self.fresh(Some(&n), "visio/masters", "masters", Role::Masters));
        }
        for (i, t) in doc.themes.iter().enumerate() {
            let name = String::from(bare(doc.str(t.part)));
            let n = self.fresh(Some(&name), "visio/theme", "theme", Role::Theme(i));
            self.theme_parts.push(n);
        }
        // pages and masters: the names they were read with first, then new ones
        let pages: Vec<_> = doc.pages().filter_map(|p| doc.page(p)).collect();
        let stencil = format.kind == DocKind::Stencil;
        let need: Vec<bool> = pages
            .iter()
            .map(|p| p.part.is_some() || !stencil || !p.shapes.is_empty() || !p.connects.is_empty() || !p.contents_extras.is_empty())
            .collect();
        self.page_parts = alloc::vec![None; pages.len()];
        for (i, p) in pages.iter().enumerate() {
            if let (true, Some(part)) = (need[i], p.part) {
                let n = bare(doc.str(part));
                if self.claim(n, Role::Page(i)) {
                    self.page_parts[i] = Some(String::from(n));
                }
            }
        }
        for i in 0..pages.len() {
            if need[i] && self.page_parts[i].is_none() {
                self.page_parts[i] = Some(self.fresh(None, "visio/pages", "page", Role::Page(i)));
            }
        }
        let masters: Vec<_> = doc.masters().filter_map(|m| doc.master(m)).collect();
        self.master_parts = alloc::vec![None; masters.len()];
        for (i, m) in masters.iter().enumerate() {
            if let Some(part) = m.part {
                let n = bare(doc.str(part));
                if self.claim(n, Role::Master(i)) {
                    self.master_parts[i] = Some(String::from(n));
                }
            }
        }
        for i in 0..masters.len() {
            if self.master_parts[i].is_none() {
                self.master_parts[i] = Some(self.fresh(None, "visio/masters", "master", Role::Master(i)));
            }
        }
        self.master_index = doc.masters().enumerate().map(|(i, m)| (m, i)).collect();
        // desktop Visio opens no file without a windows part: one of its own when there is none
        if !doc.parts.iter().any(|p| doc.str(p.content_type) == CT_WINDOWS) {
            self.windows = Some(self.fresh(Some("visio/windows.xml"), "visio", "windows", Role::Windows));
        }
        Ok(())
    }

    // doc.rels kept where they still fit
    fn keep(&mut self) {
        let doc = self.doc;
        let page_theme: Vec<Option<u32>> = doc.pages().map(|p| doc.page(p).and_then(|x| x.theme)).collect();
        // (source, single kind) pairs already kept, and (source, target) of page/master rels
        let mut singles: BTreeSet<(String, &str)> = BTreeSet::new();
        let mut contents: BTreeSet<(String, String)> = BTreeSet::new();
        let mut redo: Vec<(String, usize)> = Vec::new();
        for r in &doc.rels {
            let src = if r.source == Sym::EMPTY {
                String::new()
            } else {
                match self.written(r.source) {
                    Some(k) => k,
                    None => continue,
                }
            };
            let (kind, target) = (doc.str(r.kind), doc.str(r.target));
            if kind.is_empty() || target.is_empty() {
                continue;
            }
            let src_role = if src.is_empty() { None } else { self.role_of(&src) };
            let mut out = Out { id: String::from(doc.str(r.id)), kind: String::from(kind), target: String::from(target), external: r.external, to: None };
            if !r.external {
                let Some(to) = r.part.and_then(|p| self.written(p)) else { continue };
                let to_role = self.role_of(&to);
                // the relationships the model owns must fit it
                let fits = match (src_role, to_role) {
                    (None, t) if is_main_kind(kind) => {
                        t == Some(Role::Main) && singles.insert((src.clone(), "main"))
                    }
                    (Some(Role::Main), t) if vsdx::rel_is(kind, "pages") => t == Some(Role::Pages) && singles.insert((src.clone(), "pages")),
                    (Some(Role::Main), t) if vsdx::rel_is(kind, "masters") => {
                        t == Some(Role::Masters) && singles.insert((src.clone(), "masters"))
                    }
                    (Some(Role::Pages), t) if vsdx::rel_is(kind, "page") => {
                        matches!(t, Some(Role::Page(_))) && contents.insert((src.clone(), to.clone()))
                    }
                    (Some(Role::Masters), t) if vsdx::rel_is(kind, "master") => {
                        matches!(t, Some(Role::Master(_))) && contents.insert((src.clone(), to.clone()))
                    }
                    (Some(Role::Page(i)), t) if vsdx::rel_is(kind, "theme") => match t {
                        Some(Role::Theme(n)) => page_theme.get(i).copied().flatten() == Some(n as u32) && singles.insert((src.clone(), "theme")),
                        _ => false,
                    },
                    _ => true,
                };
                if !fits {
                    continue;
                }
                // a target that no longer resolves to its part is written anew
                let resolves = Rules::DEFAULT.resolve(self.name_of(&src), target).ok().and_then(|n| key(&n)).as_deref() == Some(to.as_str());
                if !resolves {
                    out.target = relative(self.name_of(&src), self.name_of(&to));
                }
                out.to = Some(to);
            }
            // ids unique per source; a repeated or empty one is replaced below
            let used = self.used.entry(src.clone()).or_default();
            if out.id.is_empty() || !used.insert(out.id.clone()) {
                let n = self.rels.get(&src).map_or(0, Vec::len);
                redo.push((src.clone(), n));
            }
            self.push(&src, out);
        }
        for (src, i) in redo {
            let id = self.new_id(&src);
            if let Some(o) = self.rels.get_mut(&src).and_then(|l| l.get_mut(i)) {
                o.id = id;
            }
        }
    }

    // the relationships the model needs that are missing
    fn derive(&mut self) {
        let doc = self.doc;
        let main = key(&self.main).unwrap_or_default();
        if !self.rels.get("").is_some_and(|l| l.iter().any(|r| r.to.as_deref() == Some(main.as_str()) && is_main_kind(&r.kind))) {
            self.ensure("", &main, None, REL_DOCUMENT);
        }
        if let Some(p) = self.pages.as_deref().and_then(key) {
            self.ensure(&main, &p, Some("pages"), REL_PAGES);
            for part in self.page_parts.clone().into_iter().flatten() {
                if let Some(k) = key(&part) {
                    self.ensure(&p, &k, Some("page"), REL_PAGE);
                }
            }
        }
        if let Some(m) = self.masters.as_deref().and_then(key) {
            self.ensure(&main, &m, Some("masters"), REL_MASTERS);
            for part in self.master_parts.clone().into_iter().flatten() {
                if let Some(k) = key(&part) {
                    self.ensure(&m, &k, Some("master"), REL_MASTER);
                }
            }
        }
        // page parts: their theme, the masters of their instances, their pictures
        let pages: Vec<_> = doc.pages().filter_map(|p| doc.page(p)).collect();
        for (i, pg) in pages.iter().enumerate() {
            let Some(src) = self.page_parts.get(i).cloned().flatten().and_then(|n| key(&n)) else { continue };
            if let Some(t) = pg.theme.and_then(|t| self.theme_parts.get(t as usize)).and_then(|n| key(n)) {
                self.ensure(&src, &t, Some("theme"), REL_THEME);
            }
            self.tree_rels(&src, &pg.shapes);
        }
        let masters: Vec<_> = doc.masters().filter_map(|m| doc.master(m)).collect();
        for (i, m) in masters.iter().enumerate() {
            let Some(src) = self.master_parts.get(i).cloned().flatten().and_then(|n| key(&n)) else { continue };
            self.tree_rels(&src, &m.shapes);
        }
        // themes nothing names: from the main part
        for t in self.theme_parts.clone() {
            if let Some(k) = key(&t) {
                if !self.targeted(&k) {
                    self.ensure(&main, &k, Some("theme"), REL_THEME);
                }
            }
        }
        if let Some(w) = self.windows.as_deref().and_then(key) {
            self.ensure(&main, &w, Some("windows"), REL_WINDOWS);
        }
        // parts that say what names them
        for p in &doc.parts {
            let kind = doc.str(p.rel_type);
            let Some(k) = key(bare(doc.str(p.name))) else { continue };
            if kind.is_empty() || self.targeted(&k) {
                continue;
            }
            if PACKAGE_KINDS.contains(&kind) {
                self.ensure("", &k, None, kind);
            } else if kind.starts_with(VISIO_RELS) && !["page", "pages", "master", "masters", "document"].iter().any(|n| vsdx::rel_is(kind, n)) {
                self.ensure(&main, &k, None, kind);
            }
        }
    }

    // the relationships of a page or master part to the masters and pictures its shapes use
    fn tree_rels(&mut self, src: &str, tree: &ShapeTree) {
        let doc = self.doc;
        for slot in tree.iter() {
            let Some(d) = tree.get(slot) else { continue };
            if let Some(id) = d.header.master {
                let part = doc.master_by_id(id).and_then(|m| self.master_index.get(&m)).and_then(|&i| self.master_parts.get(i).cloned().flatten());
                if let Some(k) = part.as_deref().and_then(key) {
                    self.ensure(src, &k, Some("master"), REL_MASTER);
                }
            }
            if let Some(ForeignPayload::Part(p)) = d.foreign.as_ref().map(|f| &f.payload) {
                if let Some(k) = self.written(*p) {
                    self.ensure(src, &k, None, REL_IMAGE);
                }
            }
        }
    }

    // the id of the first relationship from `src` to `to` (of kind `name` when given)
    fn id_of(&self, src: &str, to: &str, name: Option<&str>) -> Option<String> {
        self.find(src, to, name).map(|r| r.id.clone())
    }

    // the r:id of each picture part a tree names, from its part `src`
    fn pictures(&self, src: &str, tree: &ShapeTree) -> BTreeMap<Sym, String> {
        let mut out = BTreeMap::new();
        for slot in tree.iter() {
            if let Some(ForeignPayload::Part(p)) = tree.get(slot).and_then(|d| d.foreign.as_ref()).map(|f| &f.payload) {
                if let Some(id) = self.written(*p).and_then(|k| self.id_of(src, &k, None)) {
                    out.insert(*p, id);
                }
            }
        }
        out
    }
}

fn xml_part(name: &str, text: String) -> Result<Vec<u8>, WriteError> {
    write::check(&text).map_err(|line| WriteError::Xml { part: format!("/{name}"), line })?;
    Ok(text.into_bytes())
}

/// the package of a document (vsdx::write)
pub(crate) fn write(doc: &Document, options: &WriteOptions) -> Result<Vec<u8>, WriteError> {
    let format = options.format.unwrap_or(Format { kind: doc.kind, macros: doc.macros });
    if !format.macros
        && (doc.parts.iter().any(|p| vsdx::macro_name(doc.str(p.name)) || vsdx::macro_type(doc.str(p.content_type)))
            || doc.rels.iter().any(|r| vsdx::macro_type(doc.str(r.kind))))
    {
        return Err(WriteError::Macros);
    }
    let mut plan = Plan {
        doc,
        kind: format.kind,
        parts: BTreeMap::new(),
        main: String::new(),
        pages: None,
        masters: None,
        page_parts: Vec::new(),
        master_parts: Vec::new(),
        theme_parts: Vec::new(),
        windows: None,
        order: Vec::new(),
        rels: BTreeMap::new(),
        used: BTreeMap::new(),
        next_id: BTreeMap::new(),
        links: BTreeMap::new(),
        targets: BTreeSet::new(),
        master_index: BTreeMap::new(),
    };
    plan.names(format)?;
    plan.keep();
    plan.derive();
    let _ = plan.kind;

    let rules = Rules { macros: format.macros, ..Rules::DEFAULT };
    let mut pkg = Package::new(rules);
    // content types: a default for every media extension whose parts agree on one, rels, xml
    let mut exts: BTreeMap<String, Option<String>> = BTreeMap::new();
    for p in &doc.parts {
        let name = bare(doc.str(p.name));
        let last = name.rsplit('/').next().unwrap_or(name);
        let Some((_, ext)) = last.rsplit_once('.') else { continue };
        let ext = ext.to_ascii_lowercase();
        if ext.is_empty() || ext == "xml" || ext == "rels" || ext.contains(['/', '\\']) {
            continue;
        }
        let ct = doc.str(p.content_type);
        let e = exts.entry(ext).or_insert_with(|| Some(String::from(ct)));
        if !ct.starts_with("image/") || e.as_deref() != Some(ct) {
            *e = None;
        }
    }
    let mut defaults: Vec<(String, String)> = exts.into_iter().filter_map(|(e, c)| c.map(|c| (e, c))).collect();
    defaults.push((String::from("rels"), String::from(opc::RELS_TYPE)));
    defaults.push((String::from("xml"), String::from("application/xml")));
    defaults.sort();
    for (e, c) in &defaults {
        pkg.content_types_mut().set_default(e, c).map_err(WriteError::Package)?;
    }
    // the relationship parts
    for src in plan.order.clone() {
        let list: Vec<opc::Relationship> = plan.rels.get(&src).map_or(Vec::new(), |l| {
            l.iter().map(|r| opc::Relationship { id: r.id.clone(), kind: r.kind.clone(), target: r.target.clone(), external: r.external }).collect()
        });
        let name = plan.name_of(&src).to_string();
        pkg.set_relationships(&name, &list).map_err(WriteError::Package)?;
    }
    // the parts the model owns
    let main_key = key(&plan.main).unwrap_or_default();
    pkg.set_part(&plan.main, format.content_type(), xml_part(&plan.main, write::document(doc))?).map_err(WriteError::Package)?;
    let pages: Vec<_> = doc.pages().filter_map(|p| doc.page(p).map(|x| (p, x))).collect();
    if let Some(list) = plan.pages.clone() {
        let lk = key(&list).unwrap_or_default();
        let ids: Vec<Option<String>> = plan.page_parts.iter().map(|p| p.as_deref().and_then(key).and_then(|k| plan.id_of(&lk, &k, Some("page")))).collect();
        pkg.set_part(&list, CT_PAGES, xml_part(&list, write::pages(doc, &ids))?).map_err(WriteError::Package)?;
        for (i, (pref, pg)) in pages.iter().enumerate() {
            let Some(name) = plan.page_parts.get(i).cloned().flatten() else { continue };
            let pics = plan.pictures(&key(&name).unwrap_or_default(), &pg.shapes);
            let text = write::contents(doc, Some(Owner::Page(*pref)), "PageContents", &pg.shapes, &pg.connects, &pg.contents_extras, &pics);
            pkg.set_part(&name, CT_PAGE, xml_part(&name, text)?).map_err(WriteError::Package)?;
        }
    }
    let _ = main_key;
    if let Some(list) = plan.masters.clone() {
        let lk = key(&list).unwrap_or_default();
        let ids: Vec<Option<String>> = plan.master_parts.iter().map(|p| p.as_deref().and_then(key).and_then(|k| plan.id_of(&lk, &k, Some("master")))).collect();
        pkg.set_part(&list, CT_MASTERS, xml_part(&list, write::masters(doc, &ids))?).map_err(WriteError::Package)?;
        let masters: Vec<_> = doc.masters().filter_map(|m| doc.master(m).map(|x| (m, x))).collect();
        for (i, (mref, m)) in masters.iter().enumerate() {
            let Some(name) = plan.master_parts.get(i).cloned().flatten() else { continue };
            let pics = plan.pictures(&key(&name).unwrap_or_default(), &m.shapes);
            let text = write::contents(doc, Some(Owner::Master(*mref)), "MasterContents", &m.shapes, &m.connects, &m.contents_extras, &pics);
            pkg.set_part(&name, CT_MASTER, xml_part(&name, text)?).map_err(WriteError::Package)?;
        }
    }
    for (i, t) in doc.themes.iter().enumerate() {
        let Some(name) = plan.theme_parts.get(i) else { continue };
        pkg.set_part(name, CT_THEME, t.data.clone()).map_err(WriteError::Package)?;
    }
    for p in &doc.parts {
        pkg.set_part(bare(doc.str(p.name)), doc.str(p.content_type), p.data.clone()).map_err(WriteError::Package)?;
    }
    if let Some(w) = &plan.windows {
        pkg.set_part(w, CT_WINDOWS, blank::windows_xml()).map_err(WriteError::Package)?;
    }
    let method = if options.level == 0 { Method::Stored } else { Method::Deflate(options.level) };
    pkg.write(options.zip, method).map_err(WriteError::Package)
}
