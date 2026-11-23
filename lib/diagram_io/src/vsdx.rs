// vsdx -- the VSDX family (.vsdx .vsdm .vstx .vstm .vssx .vssm) and diagram_core's model
//
//   vsdx::read(&bytes, &Limits::VSDX) -> Result<Loaded, Error>
//   Loaded { doc, format, warnings, more_warnings }
//   vsdx::write(&doc, &WriteOptions::DEFAULT) -> Result<Vec<u8>, WriteError>
//   WriteOptions { format, level, zip }    another format (EX-11), the DEFLATE level, ZIP bounds
//   vsdx::new_document(kind, Measure)      a new drawing, template or stencil (blank.rs)
//   Format::from_content_type(ct) / from_extension("vsdx") / extension() / content_type()
//   Limits::VSDX                 the bounds of a file: ZIP (archive::Limits), XML nesting and
//                                attributes, counts of shapes, cells, rows and sections, the
//                                length of one formula and of one shape text
//   Error / Warning              why a file was refused / what was odd but kept
//
// What lands where (MS-VSDX; FEATURES.md 1.1 and 1.6):
//   document.xml   DocumentSettings, Colors, FaceNames, StyleSheets, DocumentSheet; EventList,
//                  HeaderFooter, PublishSettings and everything else unknown in doc.extras
//   pages.xml      the pages (PageSheet, the Rel to the page part), the page parts' shapes and
//                  Connect entries
//   masters.xml    the masters (PageSheet, Icon, Rel), the master parts' shapes and connects
//   themes         every part a theme relationship names, raw (doc.themes); a page part's own
//                  theme relationship sets Page::theme (Visio files pick a page's theme by its
//                  ThemeIndex cell: JD4 reads that)
//   everything else  windows.xml, docProps (core, app, custom), the thumbnail, media, comments,
//                  data connections and recordsets, validation, solutions, the macro project,
//                  unknown parts: doc.parts as bytes, in package order
//   relationships  every one as read in doc.rels (also the external ones, never followed)
// Unknown attributes, elements and text go into the Extras of the element they stood in
// (diagram_core's sheet.rs says how).  Nothing is computed on opening: values stay the cached
// values of the file, connectors are not rerouted, Cell::stale stays false.
//
// The macro project of a macro-enabled file (vbaProject.bin and its relationship) is kept as a
// part and never run; Document::macros says the file has the right for one.  A file whose main
// content type is not macro-enabled but that carries a macro project is refused (Error::Macros).
//
// Writing (pack.rs names the parts and keeps the relationships, write.rs makes the XML) puts
// every part of the model back: document.xml, pages.xml and masters.xml with their parts, the
// themes and every part kept as bytes (unknown packages pass through), the unknown items of
// every element at their place, the relationships with their ids (raw XML may name them), the
// main part's relationship of Visio's own type, deleted subshapes with ID 4294967295, children
// in [MS-VSDX]'s order, numbers in the shortest form that reads back the same.  The same model
// always gives the same bytes, and a file read and written again reads as the same model and
// writes the same bytes once more.  A document without windows.xml gets one of its own: desktop
// Visio refuses a file without it (error 271), although [MS-VSDX] calls the part unused.  A
// macro project is written back unchanged into a format
// that may carry it (never run); into one that may not it is WriteError::Macros.  The writer
// does not compute anything either: a cell whose formula changed (Cell::stale) keeps the value
// it has until the formula engine sets one.  It does not recurse and never panics; XML the
// model only holds as written must be well-formed where it stands, or the part is refused
// (WriteError::Xml) rather than written broken.
//
// Every file is untrusted input.  The ZIP and OPC rules come from archive::opc (names, content
// types, relationships, sizes, the ratio of compressed parts); this module adds the XML bounds,
// the counts, the structure (root elements, required attributes, IDs as numbers, r:id that name a
// relationship of the right type, a part used once), and refuses cycles: styles based on each
// other, masters that contain instances of each other, background pages of each other.  A
// reference that names nothing (a master, master shape, style, background page, connected shape)
// is kept as it is and reported as a Warning.  No panic, and no recursion: deep groups cost heap,
// never stack.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use archive::opc::{self, Package, Rules};
use diagram_core::{DocKind, Document, Owner, Part, Relationship, StyleRefs, Sym, Theme};

use crate::read::{self, Counts, RelInfo, RelMap};
pub use crate::xml::XmlError;

// ---- formats

/// one of the six files of the VSDX family: drawing, template or stencil, with or without
/// the right to carry macros
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Format {
    /// drawing (.vsdx), template (.vstx) or stencil (.vssx)
    pub kind: DocKind,
    /// macro-enabled (.vsdm, .vstm, .vssm)
    pub macros: bool,
}

const FORMATS: [(&str, &str, DocKind, bool); 6] = [
    ("vsdx", "application/vnd.ms-visio.drawing.main+xml", DocKind::Drawing, false),
    ("vstx", "application/vnd.ms-visio.template.main+xml", DocKind::Template, false),
    ("vssx", "application/vnd.ms-visio.stencil.main+xml", DocKind::Stencil, false),
    ("vsdm", "application/vnd.ms-visio.drawing.macroEnabled.main+xml", DocKind::Drawing, true),
    ("vstm", "application/vnd.ms-visio.template.macroEnabled.main+xml", DocKind::Template, true),
    ("vssm", "application/vnd.ms-visio.stencil.macroEnabled.main+xml", DocKind::Stencil, true),
];

impl Format {
    /// the format of a main part's content type (exact, as [MS-VSDX] names them)
    pub fn from_content_type(content_type: &str) -> Option<Format> {
        FORMATS.iter().find(|f| f.1 == content_type).map(|f| Format { kind: f.2, macros: f.3 })
    }

    /// the format of a file extension without the dot, ASCII case ignored ("VSDX", "vssm")
    pub fn from_extension(ext: &str) -> Option<Format> {
        FORMATS.iter().find(|f| f.0.eq_ignore_ascii_case(ext)).map(|f| Format { kind: f.2, macros: f.3 })
    }

    fn entry(self) -> &'static (&'static str, &'static str, DocKind, bool) {
        FORMATS.iter().find(|f| f.2 == self.kind && f.3 == self.macros).unwrap_or(&FORMATS[0])
    }

    /// the extension without the dot ("vsdx")
    pub fn extension(self) -> &'static str {
        self.entry().0
    }

    /// the content type of the main part
    pub fn content_type(self) -> &'static str {
        self.entry().1
    }
}

// ---- limits

/// the bounds of one file; Limits::VSDX is the default
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Limits {
    /// the ZIP container: parts, sizes, the ratio of a compressed part (archive::Limits)
    pub zip: archive::Limits,
    /// most open elements in one XML part (a group in a group costs two levels); the reader
    /// does not recurse, so this bounds memory and time, never the stack
    pub depth: usize,
    /// most attributes on one element
    pub attrs: usize,
    /// most shapes in the document (pages and masters, all levels)
    pub shapes: usize,
    /// most cells in the document (every sheet)
    pub cells: usize,
    /// most rows in the document
    pub rows: usize,
    /// most sections in the document
    pub sections: usize,
    /// most pages, masters and styles together
    pub objects: usize,
    /// most other things kept: connects, triggers, text marks and runs, colours, fonts,
    /// settings, relationships and unknown items (attributes, elements, text)
    pub items: usize,
    /// most bytes of one formula (F) or text value (V)
    pub formula_bytes: usize,
    /// most bytes of one shape text
    pub text_bytes: usize,
    /// most warnings kept in Loaded::warnings (the others are only counted)
    pub warnings: usize,
}

impl Limits {
    /// large drawings: 100000 parts, 512 MiB a part, 2 GiB in all, archives up to 1 GiB, no part
    /// denser than 200:1 (the proposal of archive::Limits); 256 levels of XML, 256 attributes, a
    /// million shapes, 16 million cells, 4 million rows, 2 million sections, formulas and values
    /// up to 1 MiB, shape texts up to 4 MiB, 100000 pages, masters and styles, 4 million other
    /// items
    pub const VSDX: Limits = Limits {
        zip: archive::Limits { entries: 100_000, part_bytes: 512 << 20, total_bytes: 2 << 30, archive_bytes: 1 << 30, ratio: 200 },
        depth: 256,
        attrs: 256,
        shapes: 1_000_000,
        cells: 16_000_000,
        rows: 4_000_000,
        sections: 2_000_000,
        objects: 100_000,
        items: 4_000_000,
        formula_bytes: 1 << 20,
        text_bytes: 4 << 20,
        warnings: 100,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Limits::VSDX
    }
}

// ---- errors and warnings

/// why a file was refused
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// the ZIP container or the OPC package: structure, names, content types, relationships,
    /// sizes and ratios (archive::opc)
    Package(opc::Error),
    /// no main part of the VSDX family: not a Visio file (another OPC format, or no main part)
    NotVisio,
    /// a macro project in a file whose main content type is not macro-enabled
    Macros,
    /// a part is no XML this reader takes (`line` counts from 1)
    Xml {
        /// the part (`/visio/pages/page1.xml`)
        part: String,
        /// the line of the problem
        line: u32,
        /// what is wrong
        error: XmlError,
    },
    /// a part breaks the structure of the format: another root element, a missing or malformed
    /// attribute (an ID that is no number), an r:id that names no relationship of the right type,
    /// a part used twice, a duplicate ID
    Structure {
        /// the part
        part: String,
        /// the line
        line: u32,
        /// what is wrong
        what: &'static str,
    },
    /// the document needs more than Limits allows: "shapes", "cells", "rows", "sections",
    /// "objects", "items", "formula", "text"
    Limit(&'static str),
    /// a cycle: "style" (styles based on each other), "master" (masters containing instances of
    /// each other), "background" (background pages of each other); the ID where it was found
    Cycle {
        /// what kind of reference
        what: &'static str,
        /// a style, master or page ID on the cycle
        id: u32,
    },
    /// (vss.rs) a binary file of Visio 2003-2010 (.vss, .vsd, .vst) breaks its structure: the
    /// compound file, the VisioDocument stream or a stream or chunk in it; `offset` is where in
    /// the stream (0 for the compound file), `what` says how ("version 6 (Visio 2000/2002)" for a
    /// version this reader does not take)
    Binary {
        /// the byte offset in the stream the problem was found at
        offset: u64,
        /// what is wrong
        what: &'static str,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Package(e) => write!(f, "package: {e:?}"),
            Error::NotVisio => write!(f, "not a Visio file of the VSDX family"),
            Error::Macros => write!(f, "a macro project in a file that may not carry macros"),
            Error::Xml { part, line, error } => write!(f, "{part}, line {line}: XML {error:?}"),
            Error::Structure { part, line, what } => write!(f, "{part}, line {line}: {what}"),
            Error::Limit(what) => write!(f, "too large: {what}"),
            Error::Cycle { what, id } => write!(f, "cycle of {what} references at ID {id}"),
            Error::Binary { offset, what } => write!(f, "binary file, offset {offset}: {what}"),
        }
    }
}

/// something odd that was kept as it is
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Warning {
    /// a shape names a master the document does not have
    MissingMaster {
        /// the part of the shape
        part: String,
        /// the shape ID
        shape: u32,
        /// the master ID it names
        master: u32,
    },
    /// a subshape of an instance names a shape its master does not have
    MissingMasterShape {
        /// the part of the shape
        part: String,
        /// the shape ID
        shape: u32,
        /// the MasterShape ID it names
        master_shape: u32,
    },
    /// a sheet or a setting names a style the document does not have
    MissingStyle {
        /// the part
        part: String,
        /// the style ID
        style: u32,
    },
    /// a page names a background page that does not exist or is no background page
    MissingBackPage {
        /// the page ID
        page: u32,
        /// the BackPage ID
        back_page: u32,
    },
    /// a Connect entry names a shape the page or master does not have
    MissingShape {
        /// the part
        part: String,
        /// the shape ID
        shape: u32,
    },
    /// a page or master without a Rel to its contents: it has no shapes
    NoContents {
        /// "page" or "master"
        what: &'static str,
        /// its ID
        id: u32,
    },
    /// a picture whose relationship points outside the package: kept, never fetched
    ExternalPicture {
        /// the part
        part: String,
        /// the shape ID
        shape: u32,
    },
    /// a page or master part that no Page or Master element names: kept as a plain part
    UnusedPart {
        /// the part
        part: String,
    },
    /// (vdx.rs) something of the XML formats of 2003-2010 that has no place in a file of the
    /// VSDX family, left out: tab stops, solution XML, data connections, a preview picture, a
    /// picture of an unknown kind, a macro project that is no OLE file, elements of other
    /// namespaces
    LeftOut {
        /// the element's name (Tab, SolutionXML, DataConnections ...)
        what: String,
    },
}

/// a file read into the model
#[derive(Clone, Debug)]
pub struct Loaded {
    /// the document
    pub doc: Document,
    /// which of the six formats the main content type says
    pub format: Format,
    /// what was odd, at most Limits::warnings of them
    pub warnings: Vec<Warning>,
    /// how many warnings more there were
    pub more_warnings: usize,
}

// ---- writing

/// how vsdx::write writes a file
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct WriteOptions {
    /// the format to write; None: the document's own (Document::kind, Document::macros).  A
    /// drawing can be written as a template and the other way round (EX-11); the content stays
    /// as it is, only the main part's content type changes
    pub format: Option<Format>,
    /// DEFLATE level 1 (fastest) to 9 (smallest), 0: stored; 6 by default
    pub level: u8,
    /// the bounds of the ZIP container written (Limits::VSDX.zip by default): a document that
    /// needs more is refused instead of written into a file the reader would refuse
    pub zip: archive::Limits,
}

impl WriteOptions {
    /// the document's own format, DEFLATE level 6, the reader's ZIP bounds
    pub const DEFAULT: WriteOptions = WriteOptions { format: None, level: 6, zip: Limits::VSDX.zip };
}

impl Default for WriteOptions {
    fn default() -> Self {
        WriteOptions::DEFAULT
    }
}

/// why a document could not be written
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WriteError {
    /// the package: a part name or content type OPC does not take, a part too large for
    /// WriteOptions::zip, more parts than it allows (archive::opc)
    Package(opc::Error),
    /// the document carries a macro project (vbaProject.bin and its relationship) and the format
    /// may not: write it as .vsdm, .vstm or .vssm, or take the project out of Document::parts
    Macros,
    /// a part the model keeps as bytes (Document::parts) has a name that is no valid part name
    /// or that another part has, or no content type
    Part {
        /// the part
        part: String,
        /// what is wrong
        what: &'static str,
    },
    /// XML the model holds as written (unknown elements in the extras, unknown text marks,
    /// trigger contents) does not fit where it stands: the part would not be well-formed
    Xml {
        /// the part
        part: String,
        /// the line of the problem in the written part (from 1)
        line: u32,
    },
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::Package(e) => write!(f, "package: {e:?}"),
            WriteError::Macros => write!(f, "a macro project in a format that may not carry macros"),
            WriteError::Part { part, what } => write!(f, "{part}: {what}"),
            WriteError::Xml { part, line } => write!(f, "{part}, line {line}: XML kept from a file is not well-formed here"),
        }
    }
}

pub use crate::blank::{new_document, Measure};

/// (JD16, DF-04) take the macro project out of a document: the parts a macro-enabled format
/// carries for it (vbaProject.bin, vbaData.xml) and the relationships to them; Document::macros
/// becomes false.  For "Speichern als .vsdx" after the user said yes; true when there was one
pub fn remove_macros(doc: &mut Document) -> bool {
    let before = doc.parts.len();
    let gone: Vec<Sym> = doc.parts.iter().filter(|p| macro_name(doc.str(p.name)) || macro_type(doc.str(p.content_type))).map(|p| p.name).collect();
    doc.parts.retain(|p| !gone.contains(&p.name));
    let removed = doc.parts.len() != before;
    let keep: Vec<bool> = doc.rels.iter().map(|r| !r.part.is_some_and(|p| gone.contains(&p)) && !macro_type(doc.str(r.kind))).collect();
    let mut k = keep.iter();
    doc.rels.retain(|_| k.next().copied().unwrap_or(true));
    let had = doc.macros;
    doc.macros = false;
    removed || had
}

/// write a document as a file of the VSDX family (see the module comment); the same document
/// always gives the same bytes, and a document read from a file and written unchanged reads
/// back as the same model
pub fn write(doc: &Document, options: &WriteOptions) -> Result<Vec<u8>, WriteError> {
    crate::pack::write(doc, options)
}

// ---- relationship types and content types ([MS-VSDX] 2.1)

pub(crate) const CT_PAGES: &str = "application/vnd.ms-visio.pages+xml";
pub(crate) const CT_PAGE: &str = "application/vnd.ms-visio.page+xml";
pub(crate) const CT_MASTERS: &str = "application/vnd.ms-visio.masters+xml";
pub(crate) const CT_MASTER: &str = "application/vnd.ms-visio.master+xml";
/// the windows part: desktop Visio refuses a file without it (error 271), although [MS-VSDX]
/// calls it unused
pub(crate) const CT_WINDOWS: &str = "application/vnd.ms-visio.windows+xml";
pub(crate) const CT_THEME: &str = "application/vnd.openxmlformats-officedocument.theme+xml";

// the type namespaces of the relationships a VSDX package uses
const REL_PREFIXES: [&str; 5] = [
    "http://schemas.microsoft.com/visio/2010/relationships/",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/",
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/",
    "http://schemas.microsoft.com/office/2006/relationships/",
];

/// whether a relationship type is the one named `name` (pages, page, masters, master, theme,
/// image ...) in one of the namespaces of the format
pub(crate) fn rel_is(kind: &str, name: &str) -> bool {
    REL_PREFIXES.iter().any(|p| kind.strip_prefix(p) == Some(name))
}

pub(crate) fn macro_name(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.contains("vbaproject") || l.ends_with("vbadata.xml")
}

pub(crate) fn macro_type(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.contains("vbaproject") || l.contains("vbadata") || l.contains("macroenabled")
}

// ---- the reader

// the main part: Visio names it with its own relationship type (…/visio/2010/relationships/
// document), other OOXML formats with officeDocument; one of them, inside the package
fn main_part(pkg: &Package) -> Result<String, Error> {
    let rels = pkg.relationships("").map_err(Error::Package)?;
    let mut found: Option<String> = None;
    for r in rels.iter().filter(|r| rel_is(&r.kind, "document") || r.kind == opc::OFFICE_DOCUMENT || r.kind == opc::OFFICE_DOCUMENT_STRICT) {
        if r.external {
            return Err(structure(opc::PACKAGE_RELS, "an external main part"));
        }
        let part = String::from(pkg.resolve("", &r.target).map_err(Error::Package)?);
        if found.as_ref().is_some_and(|f| *f != part) {
            return Err(structure(opc::PACKAGE_RELS, "more than one main part"));
        }
        found = Some(part);
    }
    found.ok_or(Error::NotVisio)
}

/// read a file of the VSDX family; `limits` bound everything (Limits::VSDX for real files)
pub fn read(bytes: &[u8], limits: &Limits) -> Result<Loaded, Error> {
    // archive::opc's own XML limits for [Content_Types].xml and the relationship parts (30000
    // elements a part, enough for 29000 pages or masters): more would make its linear search of
    // the content type defaults quadratic
    let mut rules = Rules::DEFAULT;
    rules.macros = true; // checked below against the main content type; never run
    let pkg = Package::read(bytes, limits.zip, rules).map_err(Error::Package)?;
    let main = main_part(&pkg)?;
    let format = Format::from_content_type(pkg.content_type(&main).unwrap_or("")).ok_or(Error::NotVisio)?;
    let mut ld = Loader {
        pkg: &pkg,
        lim: limits,
        doc: Document::new(format.kind),
        rels: BTreeMap::new(),
        used: BTreeSet::new(),
        warnings: Vec::new(),
        more: 0,
        counts: Counts::default(),
        main: main.clone(),
        theme_of: BTreeMap::new(),
    };
    ld.doc.macros = format.macros;
    ld.relationships(format)?;
    ld.used.insert(main.clone());
    ld.document(&main)?;
    ld.themes()?;
    ld.masters(&main)?;
    ld.pages(&main)?;
    ld.parts();
    ld.check()?;
    Ok(Loaded { doc: ld.doc, format, warnings: ld.warnings, more_warnings: ld.more })
}

pub(crate) struct Loader<'p> {
    pkg: &'p Package,
    pub(crate) lim: &'p Limits,
    pub(crate) doc: Document,
    // the relationships of each source part (stored name, "" = the package)
    rels: BTreeMap<String, RelMap>,
    // the parts the model holds (document, pages, masters, their parts, themes)
    used: BTreeSet<String>,
    warnings: Vec<Warning>,
    more: usize,
    pub(crate) counts: Counts,
    // the main part (document.xml)
    main: String,
    // the theme parts read so far -> their index in doc.themes
    theme_of: BTreeMap<String, u32>,
}

fn slash(name: &str) -> String {
    format!("/{name}")
}

fn structure(part: &str, what: &'static str) -> Error {
    Error::Structure { part: slash(part), line: 0, what }
}

impl<'p> Loader<'p> {
    pub(crate) fn warn(&mut self, w: Warning) {
        if self.warnings.len() < self.lim.warnings {
            self.warnings.push(w);
        } else {
            self.more += 1;
        }
    }

    // every relationship part: doc.rels in package order, the maps by source; the macro rule
    fn relationships(&mut self, format: Format) -> Result<(), Error> {
        let pkg = self.pkg;
        let mut macros = false;
        for e in pkg.entries() {
            if e.name.eq_ignore_ascii_case(opc::CONTENT_TYPES) {
                continue;
            }
            macros |= macro_name(&e.name) || (e.name != self.main && pkg.content_type(&e.name).is_some_and(macro_type));
            if !e.name.to_ascii_lowercase().ends_with(".rels") {
                continue;
            }
            let source = opc::rels_source(&e.name.to_ascii_lowercase()).map_err(Error::Package)?;
            let source = if source.is_empty() { source } else { String::from(pkg.name_of(&source).unwrap_or(&source)) };
            let list = pkg.relationships(&source).map_err(Error::Package)?;
            let mut map = RelMap::new();
            for r in list {
                self.counts.add(read::Count::Item, self.lim)?;
                macros |= macro_type(&r.kind);
                let part = if r.external { None } else { Some(String::from(pkg.resolve(&source, &r.target).map_err(Error::Package)?)) };
                let src_sym = if source.is_empty() { Sym::EMPTY } else { self.doc.intern(&slash(&source)) };
                let rel = Relationship {
                    source: src_sym,
                    id: self.doc.intern(&r.id),
                    kind: self.doc.intern(&r.kind),
                    target: self.doc.intern(&r.target),
                    external: r.external,
                    part: part.as_deref().map(|p| self.doc.intern(&slash(p))),
                };
                self.doc.rels.push(rel);
                map.insert(r.id, RelInfo { kind: r.kind, part, external: r.external });
            }
            self.rels.insert(source, map);
        }
        if macros && !format.macros {
            return Err(Error::Macros);
        }
        Ok(())
    }

    fn rel_map(&self, source: &str) -> RelMap {
        self.rels.get(source).cloned().unwrap_or_default()
    }

    // the one target of `source`'s relationships of type `name` (None: there is none); it must
    // have content type `ct`
    fn single(&self, source: &str, name: &str, ct: &str) -> Result<Option<String>, Error> {
        let mut found = None;
        for r in self.rels.get(source).into_iter().flat_map(|m| m.values()) {
            if !rel_is(&r.kind, name) {
                continue;
            }
            let Some(p) = r.part.as_ref().filter(|_| !r.external) else {
                return Err(structure(source, "an external relationship where a part belongs"));
            };
            if found.is_some() {
                return Err(structure(source, "more than one relationship of a kind that is single"));
            }
            if self.pkg.content_type(p) != Some(ct) {
                return Err(structure(p, "a part of the wrong content type for its relationship"));
            }
            found = Some(p.clone());
        }
        Ok(found)
    }

    // a part's text, decoded
    fn bytes(&self, part: &str) -> &'p [u8] {
        self.pkg.part(part).unwrap_or(&[])
    }

    // mark a part as held by the model; a second use is an error
    fn take(&mut self, part: &str) -> Result<(), Error> {
        if !self.used.insert(String::from(part)) {
            return Err(structure(part, "a part used twice"));
        }
        Ok(())
    }

    fn document(&mut self, main: &str) -> Result<(), Error> {
        let rels = self.rel_map(main);
        let bytes = self.bytes(main);
        read::document(self, main, bytes, &rels)
    }

    fn masters(&mut self, main: &str) -> Result<(), Error> {
        let Some(list) = self.single(main, "masters", CT_MASTERS)? else { return Ok(()) };
        self.take(&list)?;
        let rels = self.rel_map(&list);
        let bytes = self.bytes(&list);
        let masters = read::masters(self, &list, bytes, &rels)?;
        for (mut master, rid) in masters {
            let id = master.header.id;
            match rid {
                Some(rid) => {
                    let part = self.content_part(&list, &rels, &rid, "master", CT_MASTER)?;
                    let prels = self.rel_map(&part);
                    master.part = Some(self.doc.intern(&slash(&part)));
                    let bytes = self.bytes(&part);
                    read::master_contents(self, &part, bytes, &prels, &mut master)?;
                }
                None => self.warn(Warning::NoContents { what: "master", id }),
            }
            self.doc.insert_master(master).map_err(|_| structure(&list, "two masters with one ID"))?;
        }
        Ok(())
    }

    fn pages(&mut self, main: &str) -> Result<(), Error> {
        let Some(list) = self.single(main, "pages", CT_PAGES)? else { return Ok(()) };
        self.take(&list)?;
        let rels = self.rel_map(&list);
        let bytes = self.bytes(&list);
        let pages = read::pages(self, &list, bytes, &rels)?;
        for (mut page, rid) in pages {
            let id = page.header.id;
            match rid {
                Some(rid) => {
                    let part = self.content_part(&list, &rels, &rid, "page", CT_PAGE)?;
                    let prels = self.rel_map(&part);
                    page.part = Some(self.doc.intern(&slash(&part)));
                    let bytes = self.bytes(&part);
                    read::page_contents(self, &part, bytes, &prels, &mut page)?;
                    // a theme of the page's own (MS-VSDX allows one; Visio itself picks a page's
                    // theme by its ThemeIndex cell)
                    if let Some(t) = prels.values().find(|r| rel_is(&r.kind, "theme") && !r.external) {
                        if let Some(p) = &t.part {
                            page.theme = Some(self.theme_index(p)?);
                        }
                    }
                }
                // a stencil's page has no contents (Visio writes them so)
                None if self.doc.kind == DocKind::Stencil => {}
                None => self.warn(Warning::NoContents { what: "page", id }),
            }
            self.doc.insert_page(page).map_err(|_| structure(&list, "two pages with one ID"))?;
        }
        Ok(())
    }

    // the part of a Page or Master element's Rel: a relationship of type `kind` to a part of
    // content type `ct`, not used before
    fn content_part(&mut self, list: &str, rels: &RelMap, rid: &str, kind: &str, ct: &str) -> Result<String, Error> {
        let r = rels.get(rid).ok_or_else(|| structure(list, "an r:id that names no relationship"))?;
        if !rel_is(&r.kind, kind) || r.external {
            return Err(structure(list, "an r:id that names a relationship of another kind"));
        }
        let part = r.part.clone().ok_or_else(|| structure(list, "an r:id that names no part"))?;
        if self.pkg.content_type(&part) != Some(ct) {
            return Err(structure(&part, "a part of the wrong content type for its relationship"));
        }
        self.take(&part)?;
        Ok(part)
    }

    // the index of a theme part in doc.themes, added when new
    fn theme_index(&mut self, part: &str) -> Result<u32, Error> {
        if self.pkg.content_type(part) != Some(CT_THEME) {
            return Err(structure(part, "a theme relationship to a part that is no theme"));
        }
        if let Some(&i) = self.theme_of.get(part) {
            return Ok(i);
        }
        let sym = self.doc.intern(&slash(part));
        self.doc.themes.push(Theme { part: sym, data: self.bytes(part).to_vec() });
        self.used.insert(String::from(part));
        let i = self.doc.themes.len() as u32 - 1;
        self.theme_of.insert(String::from(part), i);
        Ok(i)
    }

    // every part a theme relationship names (from any source), in package order
    fn themes(&mut self) -> Result<(), Error> {
        let mut targets = BTreeSet::new();
        for m in self.rels.values() {
            for r in m.values() {
                if rel_is(&r.kind, "theme") && !r.external {
                    if let Some(p) = &r.part {
                        targets.insert(p.clone());
                    }
                }
            }
        }
        for e in self.pkg.entries() {
            if targets.contains(&e.name) {
                self.theme_index(&e.name)?;
            }
        }
        Ok(())
    }

    // the parts the model does not hold, as bytes in package order
    fn parts(&mut self) {
        let mut rel_of: BTreeMap<&str, &str> = BTreeMap::new();
        for m in self.rels.values() {
            for r in m.values() {
                if let Some(p) = &r.part {
                    rel_of.entry(p.as_str()).or_insert(r.kind.as_str());
                }
            }
        }
        let mut unused = Vec::new();
        let mut parts = Vec::new();
        for e in self.pkg.entries() {
            let lower = e.name.to_ascii_lowercase();
            if self.used.contains(&e.name) || lower.ends_with(".rels") || e.name.eq_ignore_ascii_case(opc::CONTENT_TYPES) {
                continue;
            }
            let ct = self.pkg.content_type(&e.name).unwrap_or("");
            if ct == CT_PAGE || ct == CT_MASTER {
                unused.push(slash(&e.name));
            }
            let rel = rel_of.get(e.name.as_str()).copied().unwrap_or("");
            parts.push((slash(&e.name), String::from(ct), String::from(rel), e.data.clone()));
        }
        for (name, ct, rel, data) in parts {
            let part = Part { name: self.doc.intern(&name), content_type: self.doc.intern(&ct), rel_type: self.doc.intern(&rel), data };
            self.doc.parts.push(part);
        }
        for part in unused {
            self.warn(Warning::UnusedPart { part });
        }
    }

    // cycles (errors) and references that name nothing (warnings)
    fn check(&mut self) -> Result<(), Error> {
        let main = slash(&self.main);
        check_doc(&self.doc, &main, &mut self.warnings, &mut self.more, self.lim.warnings)
    }
}

/// cycles (errors) and references that name nothing (warnings, at most `cap` kept) of a document
/// read from a file; `main` names the part of the document element (vdx.rs shares it)
pub(crate) fn check_doc(doc: &Document, main: &str, list: &mut Vec<Warning>, more: &mut usize, cap: usize) -> Result<(), Error> {
    {
        let mut warnings = Ws { list: core::mem::take(list), more: *more, cap };
        // styles: per category the based-on chain must end (a style naming itself ends it)
        let main = String::from(main);
        let styles: Vec<(u32, StyleRefs)> = doc.styles().filter_map(|s| doc.style(s)).map(|s| (s.header.id, s.header.based_on)).collect();
        let by_id: BTreeMap<u32, StyleRefs> = styles.iter().copied().collect();
        let getters: [fn(&StyleRefs) -> Option<u32>; 3] = [|r| r.line, |r| r.fill, |r| r.text];
        for get in getters {
            let next: BTreeMap<u32, u32> =
                styles.iter().filter_map(|(id, r)| get(r).filter(|n| n != id && by_id.contains_key(n)).map(|n| (*id, n))).collect();
            if let Some(id) = functional_cycle(&next) {
                return Err(Error::Cycle { what: "style", id });
            }
        }
        let style_known = |id: Option<u32>| id.is_none_or(|i| by_id.contains_key(&i));
        let style_refs = |part: &str, refs: StyleRefs, warnings: &mut Ws| {
            for id in [refs.line, refs.fill, refs.text] {
                if !style_known(id) {
                    warnings.add(|| Warning::MissingStyle { part: String::from(part), style: id.unwrap_or(0) });
                    break;
                }
            }
        };
        for (_, r) in &styles {
            style_refs(&main, *r, &mut warnings);
        }
        style_refs(&main, doc.doc_sheet().styles, &mut warnings);
        let st = &doc.settings;
        for id in [st.default_text_style, st.default_line_style, st.default_fill_style, st.default_guide_style] {
            if !style_known(id) {
                warnings.add(|| Warning::MissingStyle { part: main.clone(), style: id.unwrap_or(0) });
            }
        }
        // masters: the graph of masters through the instances in their shapes
        let mut edges: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        for m in doc.masters() {
            let Some(ms) = doc.master(m) else { continue };
            let set = edges.entry(ms.header.id).or_default();
            for s in ms.shapes.iter() {
                if let Some(t) = ms.shapes.get(s).and_then(|d| d.header.master) {
                    set.insert(t);
                }
            }
        }
        if let Some(id) = graph_cycle(&edges) {
            return Err(Error::Cycle { what: "master", id });
        }
        // background pages
        let back: BTreeMap<u32, u32> = doc.pages().filter_map(|p| doc.page(p)).filter_map(|p| p.header.back_page.map(|b| (p.header.id, b))).collect();
        if let Some(id) = functional_cycle(&back) {
            return Err(Error::Cycle { what: "background", id });
        }
        for p in doc.pages() {
            let Some(pg) = doc.page(p) else { continue };
            if let Some(b) = pg.header.back_page {
                if !doc.page_by_id(b).and_then(|r| doc.page(r)).is_some_and(|x| x.header.background) {
                    warnings.add(|| Warning::MissingBackPage { page: pg.header.id, back_page: b });
                }
            }
        }
        // shapes: masters, master shapes, styles, connects
        let owners: Vec<Owner> = doc.masters().map(Owner::Master).chain(doc.pages().map(Owner::Page)).collect();
        for owner in owners {
            let (part, sheet_styles) = match owner {
                Owner::Master(m) => doc.master(m).map(|x| (x.part, x.header.sheet_styles)).unwrap_or_default(),
                Owner::Page(p) => doc.page(p).map(|x| (x.part, x.header.sheet_styles)).unwrap_or_default(),
            };
            let part = part.map_or_else(String::new, |s| String::from(doc.str(s)));
            style_refs(&part, sheet_styles, &mut warnings);
            let Some(tree) = doc.tree(owner) else { continue };
            for slot in tree.iter() {
                let Some(d) = tree.get(slot) else { continue };
                let id = tree.id(slot).unwrap_or(0);
                if let Some(m) = d.header.master {
                    if doc.master_by_id(m).is_none() {
                        warnings.add(|| Warning::MissingMaster { part: part.clone(), shape: id, master: m });
                    }
                } else if let Some(ms) = d.header.master_shape {
                    let r = diagram_core::ShapeRef { owner, slot };
                    let has_master = doc.master_of(r).is_some();
                    if has_master && doc.master_shape_of(r).is_none() {
                        warnings.add(|| Warning::MissingMasterShape { part: part.clone(), shape: id, master_shape: ms });
                    }
                }
                style_refs(&part, d.header.styles, &mut warnings);
            }
            let connects = match owner {
                Owner::Master(m) => doc.master(m).map(|x| x.connects.as_slice()),
                Owner::Page(p) => doc.page(p).map(|x| x.connects.as_slice()),
            }
            .unwrap_or(&[]);
            for c in connects {
                for s in [c.from_sheet, c.to_sheet] {
                    if tree.by_id(s).is_none() {
                        warnings.add(|| Warning::MissingShape { part: part.clone(), shape: s });
                    }
                }
            }
        }
        *list = warnings.list;
        *more = warnings.more;
        Ok(())
    }
}

// warnings while checking: at most `cap` kept, each built only when it is kept
struct Ws {
    list: Vec<Warning>,
    more: usize,
    cap: usize,
}

impl Ws {
    fn add(&mut self, w: impl FnOnce() -> Warning) {
        if self.list.len() < self.cap {
            self.list.push(w());
        } else {
            self.more += 1;
        }
    }
}

// a node on a cycle of a graph where each node has at most one successor
fn functional_cycle(next: &BTreeMap<u32, u32>) -> Option<u32> {
    // 0 = new, 1 = on the current path, 2 = done
    let mut state: BTreeMap<u32, u8> = BTreeMap::new();
    for &start in next.keys() {
        if state.get(&start).copied().unwrap_or(0) != 0 {
            continue;
        }
        let mut path = Vec::new();
        let mut cur = Some(start);
        while let Some(n) = cur {
            match state.get(&n).copied().unwrap_or(0) {
                1 => return Some(n),
                2 => break,
                _ => {}
            }
            state.insert(n, 1);
            path.push(n);
            cur = next.get(&n).copied();
        }
        for n in path {
            state.insert(n, 2);
        }
    }
    None
}

// a node on a cycle of a directed graph (self-loops count), without recursion
fn graph_cycle(edges: &BTreeMap<u32, BTreeSet<u32>>) -> Option<u32> {
    let mut state: BTreeMap<u32, u8> = BTreeMap::new();
    for &start in edges.keys() {
        if state.get(&start).copied().unwrap_or(0) != 0 {
            continue;
        }
        // (node, its successors still to visit)
        let mut stack: Vec<(u32, Vec<u32>)> = Vec::new();
        state.insert(start, 1);
        stack.push((start, edges.get(&start).map(|s| s.iter().copied().collect()).unwrap_or_default()));
        loop {
            let Some(top) = stack.last_mut() else { break };
            let node = top.0;
            match top.1.pop() {
                Some(n) => match state.get(&n).copied().unwrap_or(0) {
                    1 => return Some(n),
                    2 => {}
                    _ => {
                        state.insert(n, 1);
                        let next = edges.get(&n).map(|s| s.iter().copied().collect()).unwrap_or_default();
                        stack.push((n, next));
                    }
                },
                None => {
                    state.insert(node, 2);
                    stack.pop();
                }
            }
        }
    }
    None
}
