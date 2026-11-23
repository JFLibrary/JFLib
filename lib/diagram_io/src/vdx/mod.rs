// vdx -- the XML formats of Visio 2003-2010 ("DatadiagramML": .vdx drawing, .vsx stencil,
// .vtx template) read into diagram_core's model (DF-20, DF-21, DF-22; JD16)
//
//   vdx::read(&bytes, DocKind::Drawing, &vsdx::Limits::VSDX) -> Result<vsdx::Loaded, vsdx::Error>
//                       one file of the format; the caller says by its extension what it is
//                       (kind_of): the XML itself does not
//   vdx::kind_of("vsx") -> Some(DocKind::Stencil)     the kind of an extension (vdx vsx vtx)
//   vdx::sniff(&bytes) -> bool         whether the bytes are such a file (root VisioDocument in
//                                      the namespace http://schemas.microsoft.com/visio/2003/core)
//
// The model is the one vsdx.rs fills, so a file read here is written as a .vsdx, .vssx or .vstx
// (a .vsdm ... with a macro project) by vsdx::write.  Microsoft's public "XML for Visio"
// schema reference describes the format; nothing of other implementations is used.  What goes
// where:
//   cells          an element per cell (<PinX F='..' Unit='MM' Err='..'>1.5</PinX>): the text is
//                  the value in internal units, F the formula ("Inh", "No Formula" as in VSDX),
//                  Unit the unit (VSDX's U), Err the error (E).  Formula and value stay a pair,
//                  nothing is computed (V1)
//   single-row sections (XForm, Line, Fill, TextBlock, Misc, PageProps ...)   flattened into the
//                  sheet's cells, the names are VSDX's (Char.Locale and Char.Perpendicular, gone
//                  since Visio 2003, are left out)
//   rows           Char -> Character, Para -> Paragraph, Prop -> Property, Act -> Actions,
//                  SmartTagDef -> ActionTag; User, Control, Connection, Scratch, Field, Layer,
//                  Hyperlink, Reviewer, Annotation by their names.  Named rows take NameU, else
//                  Name, else Row_<ID> (as Visio names them); Connection rows get T='Connection'
//                  (Visio's previewer hangs on rows without it, JD13)
//   Geom           a Geometry section per element (IX = its index), NoFill .. NoQuickDrag its
//                  cells, MoveTo, LineTo ... its rows (T = the element, IX as written)
//   Tabs           the row stays, its tab stops are left out (Warning::LeftOut "Tab": VSDX
//                  names their cells differently, and the model draws no tabs yet)
//   fonts          the Font cells of Character rows (Font, AsianFont, ComplexScriptFont) and
//                  Paragraph's BulletFont hold a FaceName ID in this format and a font name in
//                  VSDX: the ID becomes the name; FaceName Name / Panos become NameU / Panose
//   pictures       ForeignData's base64 becomes a part /visio/media/imageN.<ext> by what the bytes
//                  are (PNG, JPEG, GIF, TIFF, BMP, EMF, WMF; a bitmap without file header gets
//                  one); anything else is left out with a warning
//   masters, pages their elements with sheet, icon, shapes and connects; the writer gives them
//                  parts of their own
//   DocumentProperties   docProps core.xml, app.xml and custom.xml (props.rs); Visio's own
//                  entries (AlternateNames, BuildNumber*, TimeEdited, the root's metric) as the
//                  custom properties Visio writes in VSDX
//   Windows        windows.xml as VSDX has it (same elements), EventList and HeaderFooter the
//                  document element's own children as in VSDX
//   VBProjectData  base64 of the macro project: when it is an OLE file it becomes
//                  /visio/vbaProject.bin and Document::macros is set (written as .vsdm, never run);
//                  anything else is left out with a warning
//   everything else  (SolutionXML, DataConnections, DataRecordsets, the preview picture, elements
//                  of other namespaces) has no place in a VSDX file: left out, Warning::LeftOut
// Unknown attributes are dropped: this reader converts, it does not keep the file's XML (DF-23,
// writing .vdx, is not planned).
//
// Every file is untrusted input, bounded like a VSDX file (vsdx::Limits): the file's size by
// Limits::zip.part_bytes, nesting and attributes by the XML reader, counts of shapes, cells,
// rows, sections, objects and items, the length of formulas and texts, each picture by the
// file's size.  Cycles of styles, masters and background pages are refused as in vsdx.rs, a
// reference that names nothing is a warning.  No DOCTYPE, no external anything, no recursion
// (groups at any depth go through a stack of their own), never a panic.

mod media;
mod sheet;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{DocKind, Document, Extra, ExtraItem, Part, Place, Sym};

use crate::props::{self, Custom, Props};
use crate::read::{Count, Counts};
use crate::vsdx::{Error, Format, Limits, Loaded, Warning};
use crate::xml::{self, Event, Ns, Reader, Start, XmlError};

/// the part name errors and warnings of this format name (a file has no parts)
const PART: &str = "VDX";
const CT_VBA: &str = "application/vnd.ms-office.vbaProject";
const REL_VBA: &str = "http://schemas.microsoft.com/office/2006/relationships/vbaProject";
const REL_WINDOWS: &str = "http://schemas.microsoft.com/visio/2010/relationships/windows";

/// the kind of document an extension of the format stands for ("vdx", "vsx", "vtx"; ASCII case
/// ignored)
pub fn kind_of(ext: &str) -> Option<DocKind> {
    match ext.to_ascii_lowercase().as_str() {
        "vdx" => Some(DocKind::Drawing),
        "vsx" => Some(DocKind::Stencil),
        "vtx" => Some(DocKind::Template),
        _ => None,
    }
}

/// whether `bytes` are a file of this format: XML whose root is VisioDocument in the namespace of
/// 2003 (only the start is read)
pub fn sniff(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let Some(text) = xml::text_of(head).or_else(|| {
        // a cut in the middle of a UTF-8 sequence: the valid start
        core::str::from_utf8(&head[..core::str::from_utf8(head).err().map_or(head.len(), |e| e.valid_up_to())]).ok().map(alloc::borrow::Cow::Borrowed)
    }) else {
        return false;
    };
    let mut r = Reader::new(&text, 4, 64);
    matches!(r.next(), Ok(Event::Start(s)) if s.ns == Ns::Vdx && s.local == "VisioDocument")
}

/// read a file of the XML format of 2003-2010 as a document of `kind` (see the module comment);
/// `limits` bound everything (vsdx::Limits::VSDX for real files)
pub fn read(bytes: &[u8], kind: DocKind, limits: &Limits) -> Result<Loaded, Error> {
    if bytes.len() > limits.zip.part_bytes {
        return Err(Error::Limit("file"));
    }
    let text = xml::text_of(bytes).ok_or(Error::Xml { part: String::from(PART), line: 1, error: XmlError::Unsupported })?;
    let mut vx = Vx {
        r: Reader::new(&text, limits.depth, limits.attrs),
        lim: limits,
        doc: Document::new(kind),
        counts: Counts::default(),
        warnings: Vec::new(),
        more: 0,
        media: 0,
        faces: BTreeMap::new(),
        props: Props::default(),
        metric: None,
        windows: None,
        vba: None,
    };
    vx.document()?;
    vx.finish()?;
    let Vx { doc, mut warnings, mut more, .. } = vx;
    crate::vsdx::check_doc(&doc, PART, &mut warnings, &mut more, limits.warnings)?;
    let format = Format { kind, macros: doc.macros };
    Ok(Loaded { doc, format, warnings, more_warnings: more })
}

// the reading of one file
pub(crate) struct Vx<'a, 'l> {
    r: Reader<'a>,
    lim: &'l Limits,
    doc: Document,
    counts: Counts,
    warnings: Vec<Warning>,
    more: usize,
    // the pictures made parts so far
    media: u32,
    // FaceName ID -> name
    faces: BTreeMap<u32, String>,
    // DocumentProperties
    props: Props,
    // the root's metric attribute
    metric: Option<bool>,
    // windows.xml as VSDX writes it
    windows: Option<Vec<u8>>,
    // the macro project (an OLE file)
    vba: Option<Vec<u8>>,
}

type R<T> = Result<T, Error>;

fn line_of(src: &str, pos: usize) -> u32 {
    let b = src.as_bytes();
    1 + b[..pos.min(b.len())].iter().filter(|&&c| c == b'\n').count() as u32
}

impl<'a> Vx<'a, '_> {
    fn xml_err(&self, (error, pos): (XmlError, usize)) -> Error {
        Error::Xml { part: String::from(PART), line: line_of(self.r.src(), pos), error }
    }

    fn bad(&self, what: &'static str) -> Error {
        Error::Structure { part: String::from(PART), line: line_of(self.r.src(), self.r.pos()), what }
    }

    fn next(&mut self) -> R<Event<'a>> {
        self.r.next().map_err(|e| self.xml_err(e))
    }

    fn skip(&mut self, s: &Start<'a>) -> R<&'a str> {
        self.r.skip(s).map_err(|e| self.xml_err(e))
    }

    fn sym(&mut self, s: &str) -> Sym {
        self.doc.intern(s)
    }

    fn count(&mut self, what: Count) -> R<()> {
        let lim = self.lim;
        self.counts.add(what, lim)
    }

    fn warn(&mut self, w: Warning) {
        if self.warnings.len() < self.lim.warnings {
            self.warnings.push(w);
        } else {
            self.more += 1;
        }
    }

    // an element that has no place in the model: read to its end, a warning
    fn left_out(&mut self, s: &Start<'a>) -> R<()> {
        self.skip(s)?;
        self.warn(Warning::LeftOut { what: String::from(s.qname) });
        Ok(())
    }

    // whether `s` is the element `local` of the format's namespace
    fn is(s: &Start, local: &str) -> bool {
        s.ns == Ns::Vdx && s.local == local
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

    // the text of an element (child elements left out with a warning), at most `max` bytes
    fn text_of(&mut self, s: &Start<'a>, max: usize, what: &'static str) -> R<String> {
        let mut buf = String::new();
        if s.empty {
            self.next()?;
            return Ok(buf);
        }
        loop {
            match self.next()? {
                Event::Text(t) => {
                    if buf.len() + t.len() > max {
                        return Err(Error::Limit(what));
                    }
                    buf.push_str(&t);
                }
                Event::Start(c) => self.left_out(&c)?,
                Event::End { .. } => return Ok(buf),
                Event::Eof => return Err(self.bad("an element without end")),
            }
        }
    }

    // an element of the format's namespace and everything in it as XML text without namespace
    // prefixes (it is written into a VSDX part whose default namespace is Visio's); None, and the
    // element read to its end, when something in it is of another namespace
    fn copy(&mut self, s: &Start<'a>) -> R<Option<String>> {
        Ok(self.copy_full(s)?.0)
    }

    // copy, and whether the element had child elements
    pub(crate) fn copy_full(&mut self, s: &Start<'a>) -> R<(Option<String>, bool)> {
        let mut out = String::new();
        let mut ok = s.ns == Ns::Vdx;
        let mut nested = false;
        open_tag(&mut out, s);
        if s.empty {
            self.next()?;
            out.push_str("/>");
            return Ok((ok.then_some(out), false));
        }
        out.push('>');
        let mut names: Vec<&'a str> = alloc::vec![s.local];
        let max = self.lim.text_bytes;
        loop {
            match self.next()? {
                Event::Start(c) => {
                    nested = true;
                    ok &= c.ns == Ns::Vdx;
                    open_tag(&mut out, &c);
                    if c.empty {
                        out.push_str("/>");
                        self.next()?;
                    } else {
                        out.push('>');
                        names.push(c.local);
                    }
                }
                Event::Text(t) => props::escape(&t, &mut out),
                Event::End { .. } => {
                    let n = names.pop().unwrap_or("");
                    out.push_str("</");
                    out.push_str(n);
                    out.push('>');
                    if names.is_empty() {
                        break;
                    }
                }
                Event::Eof => return Err(self.bad("an element without end")),
            }
            if out.len() > max {
                return Err(Error::Limit("text"));
            }
        }
        Ok((ok.then_some(out), nested))
    }

    // ---- the document element

    fn document(&mut self) -> R<()> {
        let root = match self.next()? {
            Event::Start(s) if Self::is(&s, "VisioDocument") => s,
            _ => return Err(Error::NotVisio),
        };
        for a in &root.attrs {
            if a.ns == Ns::None && a.local == "metric" {
                self.metric = self.flag(&a.value, "a metric attribute that is no flag").ok();
            }
        }
        if root.empty {
            self.next()?;
            return self.end();
        }
        let mut seen: Vec<&'a str> = Vec::new();
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(_) => continue,
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("VisioDocument without end")),
            };
            // each child once; a second one is left out
            if c.ns != Ns::Vdx || seen.contains(&c.local) {
                self.left_out(&c)?;
                continue;
            }
            seen.push(c.local);
            match c.local {
                "DocumentProperties" => self.doc_props(&c)?,
                "DocumentSettings" => self.settings(&c)?,
                "Colors" => self.colors(&c)?,
                "FaceNames" => self.face_names(&c)?,
                "StyleSheets" => self.styles(&c)?,
                "DocumentSheet" => self.doc_sheet(&c)?,
                "Masters" => self.masters(&c)?,
                "Pages" => self.pages(&c)?,
                "Windows" => self.windows(&c)?,
                // the same elements in VSDX's document.xml, after its five known children
                "EventList" | "HeaderFooter" => match self.copy(&c)? {
                    Some(x) => {
                        self.count(Count::Item)?;
                        let item = ExtraItem::Element(self.sym(&x));
                        self.doc.extras.push(Extra { place: Place::AfterChild(5), item });
                    }
                    None => self.warn(Warning::LeftOut { what: String::from(c.qname) }),
                },
                "VBProjectData" => {
                    let t = self.text_of(&c, self.lim.zip.part_bytes, "file")?;
                    match media::base64(t.as_bytes(), self.lim.zip.part_bytes) {
                        Some(b) if b.starts_with(&media::OLE) => self.vba = Some(b),
                        _ => self.warn(Warning::LeftOut { what: String::from("VBProjectData") }),
                    }
                }
                _ => self.left_out(&c)?,
            }
        }
        self.end()
    }

    // after the root's end: only the end of the text
    fn end(&mut self) -> R<()> {
        match self.next()? {
            Event::Eof => Ok(()),
            _ => Err(self.bad("content after the root element")),
        }
    }

    fn doc_props(&mut self, s: &Start<'a>) -> R<()> {
        if s.empty {
            self.next()?;
            return Ok(());
        }
        let max = self.lim.formula_bytes;
        let mut own: Vec<Custom> = Vec::new();
        let mut visio: Vec<Custom> = Vec::new();
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(_) => continue,
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("DocumentProperties without end")),
            };
            if c.ns != Ns::Vdx {
                self.left_out(&c)?;
                continue;
            }
            if c.local == "CustomProps" {
                self.custom_props(&c, &mut own)?;
                continue;
            }
            if c.local == "PreviewPicture" {
                self.left_out(&c)?;
                continue;
            }
            let t = self.text_of(&c, max, "formula")?;
            let p = &mut self.props;
            match c.local {
                "Title" => p.title = t,
                "Subject" => p.subject = t,
                "Creator" => p.creator = t,
                "Manager" => p.manager = t,
                "Company" => p.company = t,
                "Category" => p.category = t,
                "Keywords" => p.keywords = t,
                "Desc" => p.description = t,
                "HyperlinkBase" => p.hyperlink_base = t,
                "Template" => p.template = t,
                "TimeCreated" => p.created = w3cdtf(&t),
                "TimeSaved" => p.modified = w3cdtf(&t),
                "TimePrinted" => p.last_printed = w3cdtf(&t),
                "AlternateNames" => visio.push(Custom { name: String::from("_VPID_ALTERNATENAMES"), kind: String::from("lpwstr"), value: t }),
                "BuildNumberCreated" => visio.push(Custom { name: String::from("BuildNumberCreated"), kind: String::from("i4"), value: t }),
                "BuildNumberEdited" => visio.push(Custom { name: String::from("BuildNumberEdited"), kind: String::from("i4"), value: t }),
                "TimeEdited" => visio.push(Custom { name: String::from("TimeEdited"), kind: String::from("filetime"), value: w3cdtf(&t) }),
                _ => self.warn(Warning::LeftOut { what: String::from(c.qname) }),
            }
        }
        // Visio's own entries first, as Visio writes them in VSDX
        self.props.custom = visio;
        self.props.custom.extend(own);
        Ok(())
    }

    fn custom_props(&mut self, s: &Start<'a>, out: &mut Vec<Custom>) -> R<()> {
        if s.empty {
            self.next()?;
            return Ok(());
        }
        let max = self.lim.formula_bytes;
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(_) => continue,
                Event::End { .. } => return Ok(()),
                Event::Eof => return Err(self.bad("CustomProps without end")),
            };
            if !Self::is(&c, "CustomProp") {
                self.left_out(&c)?;
                continue;
            }
            let mut name = String::new();
            let mut kind = "lpwstr";
            for a in &c.attrs {
                match (a.ns, a.local) {
                    (Ns::None, "Name") => name = String::from(a.value.as_ref()),
                    (Ns::None, "PropType") => {
                        kind = match a.value.as_ref() {
                            "number" => "r8",
                            "boolean" => "bool",
                            "date" => "filetime",
                            _ => "lpwstr",
                        }
                    }
                    _ => {}
                }
            }
            let mut value = self.text_of(&c, max, "formula")?;
            match kind {
                "filetime" => value = w3cdtf(&value),
                "bool" => value = String::from(if value.trim() == "1" || value.trim().eq_ignore_ascii_case("true") { "true" } else { "false" }),
                _ => {}
            }
            self.count(Count::Item)?;
            out.push(Custom { name, kind: String::from(kind), value });
        }
    }

    fn windows(&mut self, s: &Start<'a>) -> R<()> {
        let Some(x) = self.copy(s)? else {
            self.warn(Warning::LeftOut { what: String::from("Windows") });
            return Ok(());
        };
        // the root gets VSDX's namespaces (copy writes attributes quoted, so the first '>' ends
        // the start tag)
        let cut = x.find('>').map_or(x.len(), |i| if x[..i].ends_with('/') { i - 1 } else { i });
        let mut out = String::from("<?xml version='1.0' encoding='utf-8' ?>\r\n");
        out.push_str(&x[..cut]);
        out.push_str(" xmlns='");
        out.push_str(xml::NS_VISIO);
        out.push_str("' xmlns:r='");
        out.push_str(xml::NS_REL);
        out.push_str("' xml:space='preserve'");
        out.push_str(&x[cut..]);
        self.windows = Some(out.into_bytes());
        Ok(())
    }

    // ---- after reading: the parts the format keeps elsewhere

    fn finish(&mut self) -> R<()> {
        if let Some(m) = self.metric {
            self.props.custom.push(Custom { name: String::from("IsMetric"), kind: String::from("bool"), value: String::from(if m { "true" } else { "false" }) });
        }
        if self.props != Props::default() {
            let p = core::mem::take(&mut self.props);
            props::write(&mut self.doc, &p);
        }
        if let Some(w) = self.windows.take() {
            let part = Part {
                name: self.sym("/visio/windows.xml"),
                content_type: self.sym(crate::vsdx::CT_WINDOWS),
                rel_type: self.sym(REL_WINDOWS),
                data: w,
            };
            self.doc.parts.push(part);
        }
        if let Some(v) = self.vba.take() {
            let name = self.sym("/visio/vbaProject.bin");
            let part = Part { name, content_type: self.sym(CT_VBA), rel_type: self.sym(REL_VBA), data: v };
            // the writer hangs it on the main part (pack.rs)
            self.doc.parts.push(part);
            self.doc.macros = true;
        }
        Ok(())
    }
}

// a start tag without namespace prefixes: the local name and the plain attributes, quoted with '
fn open_tag(out: &mut String, s: &Start) {
    out.push('<');
    out.push_str(s.local);
    for a in s.attrs.iter().filter(|a| a.ns == Ns::None) {
        out.push(' ');
        out.push_str(a.local);
        out.push_str("='");
        props::escape(&a.value, out);
        out.push('\'');
    }
}

// a date of the format ("2000-12-20T23:00:00", local time without zone) as W3CDTF: a zone is
// added when there is none
fn w3cdtf(t: &str) -> String {
    let t = t.trim();
    let has_zone = t.ends_with('Z') || t.get(19..).is_some_and(|z| z.contains(['+', '-']));
    if t.is_empty() || has_zone || !t.contains('T') {
        String::from(t)
    } else {
        alloc::format!("{t}Z")
    }
}
