// Conservative DOCX interchange for the initial document model. New native library code.
// Unrepresentable content is rejected before exposing an editable document. No VBA support.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
use alloc::{collections::BTreeSet, format, string::String, vec, vec::Vec};
use archive::{Entry, Limits};
use base::{
    encoding,
    xml::{self, Node},
};
use doc_core::{
    Document, Id, Inline, NoteKind, PageSettings, Paragraph, ParagraphFormat, ParagraphStyle,
    TextFormat,
};
mod bibliography;
mod bibtex;
mod citations;
mod clipboard;
mod import_blocks;
mod html_import;
pub use html_import::clipboard_html_decode;
pub use clipboard::{clipboard_html_encode, clipboard_rtf_decode, clipboard_rtf_encode};
mod complex_fields;
mod formulas;
mod fragment;
pub use fragment::{clipboard_display_fragment, decode_document_fragment, encode_document_fragment};
mod exchange;
mod format;
mod styles;
mod tables;
mod table_position;
mod cell_borders;
mod table_styles;
mod library;
mod notes;
mod package;
mod preservation;
mod preserved_text;
mod ris;
pub use bibliography::{decode_sources, encode_sources, load_sources, save_sources};
pub use bibtex::{decode_bibtex, encode_bibtex};
pub use exchange::{
    BibliographyFormat, decode_bibliography, encode_bibliography, load_bibliography,
    save_bibliography,
};
pub use library::{
    LibraryRecovery, LibraryStore, LibraryStoreError, decode_source_library, encode_source_library,
};
use package::*;
pub use preservation::{DocxFile, DocxMode, DocxReport};
pub use preserved_text::DocxTextRun;
mod preserved_stories;
pub use preserved_stories::DocxStory;
pub use ris::{decode_ris, encode_ris};

/// Failure with the unsupported part/element included for a useful UI explanation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Io,
    Limit,
    Invalid,
    Archive(archive::Error),
    Xml(xml::Error),
    Unsupported(String),
    Model(doc_core::Error),
}
impl From<xml::Error> for Error {
    fn from(e: xml::Error) -> Self {
        Self::Xml(e)
    }
}
impl From<doc_core::Error> for Error {
    fn from(e: doc_core::Error) -> Self {
        Self::Model(e)
    }
}
fn unsupported(s: &str) -> Error {
    Error::Unsupported(String::from(s))
}
fn esc(s: &str) -> Result<String, Error> {
    Ok(xml::escape(s)?)
}
fn parse(b: &[u8]) -> Result<Node, Error> {
    let (s, _) = encoding::decode(b, 4 << 20).map_err(|_| Error::Invalid)?;
    Ok(xml::parse(&s)?)
}
fn container(n: &Node, ns: &str, name: &str) -> Result<(), Error> {
    if !n.is(ns, name) || !n.attrs.is_empty() || !n.text.is_empty() {
        return Err(unsupported(&n.name.local));
    }
    Ok(())
}
fn leaf(n: &Node, ns: &str, name: &str) -> Result<String, Error> {
    if !n.is(ns, name) || !n.attrs.is_empty() || !n.children.is_empty() {
        return Err(unsupported(&n.name.local));
    }
    Ok(n.text.clone())
}
fn find<'a>(entries: &'a [Entry], name: &str) -> Result<&'a [u8], Error> {
    entries
        .iter()
        .find(|e| e.name == name)
        .map(|e| e.data.as_slice())
        .ok_or(Error::Invalid)
}
fn style_name(s: ParagraphStyle) -> String {
    match s {
        ParagraphStyle::Normal => String::from("Normal"),
        ParagraphStyle::Title => String::from("Title"),
        ParagraphStyle::Heading1 => String::from("Heading1"),
        ParagraphStyle::Heading2 => String::from("Heading2"),
        ParagraphStyle::Named(id) => format!("jfStyle{id}"),
    }
}
fn style(s: &str) -> Result<ParagraphStyle, Error> {
    match s {
        "Normal" => Ok(ParagraphStyle::Normal),
        "Title" => Ok(ParagraphStyle::Title),
        "Heading1" => Ok(ParagraphStyle::Heading1),
        "Heading2" => Ok(ParagraphStyle::Heading2),
        _ => {
            if s.is_empty() || s.len() > 128 || s.chars().any(char::is_control) { return Err(Error::Invalid); }
            let id = if let Some(n) = s.strip_prefix("jfStyle") { n.parse::<u32>().map_err(|_| Error::Invalid)? }
            else { s.bytes().fold(2166136261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16777619)) & 0x7fff_ffff };
            if id == 0 || id == u32::MAX { return Err(Error::Invalid); }
            Ok(ParagraphStyle::Named(id))
        },
    }
}
fn run(s: &str) -> Result<String, Error> {
    styled_run(s, TextFormat::default())
}
fn styled_run(s: &str, f: TextFormat) -> Result<String, Error> {
    let mut out = format!("<w:r>{}<w:t xml:space=\"preserve\">", format::write_run(f));
    for c in s.chars() {
        match c {
            '\n' => out.push_str("</w:t><w:br/><w:t xml:space=\"preserve\">"),
            '\t' => out.push_str("</w:t><w:tab/><w:t xml:space=\"preserve\">"),
            _ => {
                let mut buf = [0; 4];
                out.push_str(&esc(c.encode_utf8(&mut buf))?);
            }
        }
    }
    out.push_str("</w:t></w:r>");
    Ok(out)
}
fn write_paragraph(
    d: &Document,
    p: &Paragraph,
    note_mark: Option<NoteKind>,
) -> Result<String, Error> {
    let mut document = String::new();
    if p.id > 0x7fff_ffff {
        return Err(Error::Limit);
    }
    document.push_str(&format!(
        "<w:p w14:paraId=\"{:08X}\">{}",
        p.id,
        format::write_paragraph(p)
    ));
    if let Some(kind) = note_mark {
        document.push_str(&format!(
            "<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:{}Ref/></w:r>",
            notes::name(kind)
        ));
    }
    for i in &p.content {
        match i {
            Inline::Text(t) => document.push_str(&run(t)?),
            Inline::StyledText { text, format } => document.push_str(&styled_run(text, *format)?),
            Inline::Citation { source, locator } => {
                let instr = if locator.is_empty() {
                    format!(" CITATION jf{source} ")
                } else {
                    format!(" CITATION jf{source} \\p \"{locator}\" ")
                };
                document.push_str(&format!(
                    "<w:fldSimple w:instr=\"{}\">{}</w:fldSimple>",
                    esc(&instr)?,
                    run(&d.citation_text(*source, locator))?
                ));
            }
            Inline::CitationCluster(items) => document.push_str(&format!(
                "<w:fldSimple w:instr=\"{}\">{}</w:fldSimple>",
                esc(&citations::write(items))?,
                run(&d.citation_cluster_text(items))?
            )),
            Inline::Bibliography => document.push_str(&format!(
                "<w:fldSimple w:instr=\" BIBLIOGRAPHY \">{}</w:fldSimple>",
                run(&d.bibliography().join("\n"))?
            )),
            Inline::Formula(f) => document.push_str(&format!(
                "<w:fldSimple w:instr=\"{}\">{}</w:fldSimple>",
                esc(&formulas::write(f))?,run(&d.evaluate_formula(p.id,f).text)?
            )),
            Inline::NoteReference(id) => {
                let note = d.note(*id).ok_or(Error::Invalid)?;
                document.push_str(&format!("<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:{}Reference w:id=\"{id}\"/></w:r>", notes::name(note.kind)));
            }
        }
    }
    document.push_str("</w:p>");
    Ok(document)
}
/// Serialize a validated document to a deterministic DOCX package. Sources are native
/// bibliography custom XML; citations are CITATION fields with cached display text.
/// Footnotes and endnotes use their standard parts and continuous decimal numbering.
pub fn encode_docx(d: &Document) -> Result<Vec<u8>, Error> {
    d.validate()?;
    let mut document = format!(
        "{DECL}<w:document xmlns:w=\"{W}\" xmlns:w14=\"{W14}\" xmlns:mc=\"{MC}\" mc:Ignorable=\"w14\"><w:body>"
    );
    for p in &d.paragraphs {
        for t in d.tables.iter().filter(|t| t.before == p.id) { document.push_str(&tables::write(d,t)?); }
        document.push_str(&write_paragraph(d, p, None)?);
    }
    document.push_str(&notes::write_section(d));
    document.push_str("</w:body></w:document>");
    // Never save a model whose expanded fields exceed the matching XML reader's limits.
    parse(document.as_bytes())?;
    let sources = encode_sources(&d.sources)?;
    let has_notes = notes::KINDS.map(|kind| notes::present(d, kind));
    let mut entries: Vec<Entry> = FIXED
        .iter()
        .map(|(n, s)| Entry {
            name: String::from(*n),
            data: tables::metadata(n, metadata(n, s, has_notes), !d.tables.is_empty()).into_bytes(),
        })
        .collect();
    if !d.tables.is_empty() { entries.push(Entry {name:String::from(tables::PATH),data:tables::identities(d).into_bytes()}); }
    if !d.styles.is_empty() || !d.table_styles.is_empty() || d.tables.iter().any(|t|t.style.is_some()) {
        entries.iter_mut().find(|e| e.name == "word/styles.xml").unwrap().data = styles::write(d)?.into_bytes();
    }
    entries.push(Entry {
        name: String::from("word/document.xml"),
        data: document.into_bytes(),
    });
    entries.push(Entry {
        name: String::from("customXml/item1.xml"),
        data: sources,
    });
    for kind in notes::KINDS {
        if notes::present(d, kind) {
            entries.push(Entry {
                name: String::from(notes::path(kind)),
                data: notes::write(d, kind)?,
            });
        }
    }
    if !d.notes.is_empty() {
        entries.push(Entry {
            name: String::from("word/settings.xml"),
            data: notes::settings(has_notes).into_bytes(),
        });
    }
    archive::write(&entries, Limits::default()).map_err(Error::Archive)
}
fn source_id(s: &str) -> Result<Id, Error> {
    let s = s
        .strip_prefix("jf")
        .ok_or_else(|| unsupported("Quellenkennung"))?;
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Invalid);
    }
    s.parse().map_err(|_| Error::Invalid)
}
fn read_run(n: &Node) -> Result<Inline, Error> {
    container(n, W, "r")?;
    let mut s = String::new();
    let mut f = TextFormat::default();
    for (index, c) in n.children.iter().enumerate() {
        if c.is(W, "rPr") {
            if index != 0 {
                return Err(Error::Invalid);
            }
            f = format::read_run(c)?;
        } else if c.is(W, "t") {
            if !c.children.is_empty()
                || c.attrs
                    .iter()
                    .any(|(a, v)| a.ns != XML || a.local != "space" || v != "preserve")
            {
                return Err(unsupported("Textattribute"));
            }
            s.push_str(&c.text);
        } else if c.is(W, "br") || c.is(W, "tab") {
            if !c.children.is_empty() || !c.attrs.is_empty() || !c.text.is_empty() {
                return Err(unsupported(&c.name.local));
            }
            s.push(if c.name.local == "br" { '\n' } else { '\t' });
        } else {
            return Err(unsupported(&c.name.local));
        }
    }
    Ok(Inline::with_format(s, f))
}
fn read_citation(n: &Node) -> Result<Inline, Error> {
    if !n.text.is_empty() {
        return Err(unsupported("Feldattribute"));
    }
    complex_fields::field_attributes(n, "instr")?;
    let instr = n.attr(W, "instr").ok_or(Error::Invalid)?.trim();
    for c in &n.children {
        if read_run(c)?.format() != TextFormat::default() {
            return Err(unsupported("Direkte Formatierung eines Feldresultats"));
        }
    }
    if instr == "BIBLIOGRAPHY" {
        return Ok(Inline::Bibliography);
    }
    if instr.starts_with('='){return formulas::read(instr).map(Inline::Formula);}
    citations::read(instr)
}
fn paragraph_id(n: &Node, required: bool) -> Result<Id, Error> {
    if !n.is(W, "p") || !n.text.is_empty() || n.attrs.len() > 1 {
        return Err(unsupported(&n.name.local));
    }
    let Some(raw_id) = n.attr(W14, "paraId") else {
        return if required || !n.attrs.is_empty() {
            Err(Error::Invalid)
        } else {
            Ok(0)
        };
    };
    if raw_id.len() != 8 || !raw_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Invalid);
    }
    let id = Id::from_str_radix(raw_id, 16).map_err(|_| Error::Invalid)?;
    if id == 0 || id >= 0x7fff_ffff {
        return Err(Error::Invalid);
    }
    Ok(id)
}
fn read_paragraph(
    n: &Node,
    mapping: &[notes::Mapping],
    note: Option<(NoteKind, bool)>,
) -> Result<Paragraph, Error> {
    let id = paragraph_id(n, false)?;
    let mut p = Paragraph {
        id,
        style: ParagraphStyle::Normal,
        format: ParagraphFormat::default(),
        content: Vec::new(),
    };
    let mut props = false;
    let mut mark = false;
    let normalized;
    let children = if n.children.iter().any(|r| r.is(W, "r") && r.children.iter().any(|n| n.is(W, "fldChar") || n.is(W, "instrText"))) {
        normalized = complex_fields::normalize(n)?;
        normalized.as_slice()
    } else { n.children.as_slice() };
    for c in children {
        if c.is(W, "pPr") {
            if props || mark || !p.content.is_empty() {
                return Err(Error::Invalid);
            }
            props = true;
            format::read_paragraph(c, &mut p)?;
        } else if c.is(W, "r") {
            if let Some(tag) = notes::marker(c) {
                if let Some((kind, first)) = note {
                    if !first
                        || mark
                        || !p.content.is_empty()
                        || !matches!(tag, "footnoteRef" | "endnoteRef")
                    {
                        return Err(unsupported("Verschachtelte oder wiederholte Notenzeichen"));
                    }
                    notes::read_mark(c, kind)?;
                    mark = true;
                } else {
                    p.content.push(notes::read_reference(c, mapping)?);
                }
            } else {
                p.content.push(read_run(c)?);
            }
        } else if c.is(W, "fldSimple") {
            let field = read_citation(c)?;
            if note.is_some() && field == Inline::Bibliography {
                return Err(unsupported("Literaturverzeichnis in Note"));
            }
            p.content.push(field);
        } else {
            return Err(unsupported(&c.name.local));
        }
    }
    if note.is_some_and(|(_, first)| first) && !mark {
        return Err(unsupported("Fehlendes Notenzeichen"));
    }
    if p.content.is_empty() {
        p.content.push(Inline::Text(String::new()));
    }
    Ok(p)
}
/// Read the initial editable DOCX profile. Unknown package parts, layout, formatting,
/// relationships, fields and source fields are rejected, never silently discarded.
/// Full Word interoperability is deliberately not claimed by this early codec.
pub fn decode_docx(b: &[u8]) -> Result<Document, Error> {
    let entries = archive::read(b, Limits::default()).map_err(Error::Archive)?;
    for e in &entries {
        if !FIXED.iter().any(|(n, _)| *n == e.name)
            && !matches!(
                e.name.as_str(),
                "word/document.xml"
                    | "customXml/item1.xml"
                    | "word/footnotes.xml"
                    | "word/endnotes.xml"
                    | "word/settings.xml"
                    | "customXml/jfTables.xml"
            )
        {
            return Err(unsupported(&e.name));
        }
    }
    let has_notes = notes::KINDS.map(|kind| entries.iter().any(|e| e.name == notes::path(kind)));
    if has_notes.iter().any(|present| *present) {
        if parse(find(&entries, "word/settings.xml")?)?
            != parse(notes::settings(has_notes).as_bytes())?
        {
            return Err(unsupported("word/settings.xml"));
        }
    } else if entries.iter().any(|e| e.name == "word/settings.xml") {
        return Err(unsupported("word/settings.xml"));
    }
    let has_tables = entries.iter().any(|e| e.name == tables::PATH);
    for (name, expected) in FIXED {
        if *name == "word/styles.xml" { continue; }
        // Compare expanded-name trees, accepting namespace prefix and whitespace changes.
        if parse(find(&entries, name)?)? != parse(tables::metadata(name, metadata(name, expected, has_notes),has_tables).as_bytes())? {
            return Err(unsupported(name));
        }
    }
    let source_list =
        bibliography::read_sources(&parse(find(&entries, "customXml/item1.xml")?)?, true)?;
    let root = parse(find(&entries, "word/document.xml")?)?;
    if !root.is(W, "document")
        || !root.text.is_empty()
        || root.children.len() != 1
        || (!root.attrs.is_empty()
            && (root.attrs.len() != 1 || root.attr(MC, "Ignorable") != Some("w14")))
    {
        return Err(unsupported("Dokumentattribute"));
    }
    let body = &root.children[0];
    container(body, W, "body")?;
    let mut used = BTreeSet::new();
    for source in &source_list {
        if !used.insert(source.id) {
            return Err(Error::Invalid);
        }
    }
    tables::reserve_paragraphs(body, &mut used)?;
    let table_ids = tables::read_identities(&entries, &mut used)?;
    let (document_notes, mapping) = notes::read(&entries, &mut used)?;
    let mut paragraphs = Vec::new();
    let mut document_tables = Vec::new();
    let mut table_cursor = 0;
    let mut section = false;
    let mut page = PageSettings::default();
    let mut next_id = 1;
    for n in &body.children {
        if n.is(W, "sectPr") {
            if section {
                return Err(unsupported("Abschnittslayout"));
            }
            page = notes::read_section(n, has_notes)?;
            section = true;
            continue;
        }
        if section {
            return Err(unsupported(&n.name.local));
        }
        if n.is(W,"tbl") {
            let mut nested=Vec::new();
            let t = tables::read(n,&table_ids,&mut table_cursor,&mut nested,1,&mapping,&mut used,&mut next_id)?;
            document_tables.push(t);document_tables.extend(nested); continue;
        }
        let mut paragraph = read_paragraph(n, &mapping, None)?;
        if paragraph.id == 0 {
            paragraph.id = notes::allocate(&mut used, &mut next_id)?;
        }
        for t in document_tables.iter_mut().filter(|t| t.before == 0) { t.before=paragraph.id; }
        paragraphs.push(paragraph);
    }
    if !section {
        return Err(unsupported("Abschnittslayout"));
    }
    if has_tables && table_ids.len() != document_tables.len() { return Err(Error::Invalid); }
    if document_tables.iter().any(|t| t.before == 0) {
        let id=notes::allocate(&mut used,&mut next_id)?;
        for t in document_tables.iter_mut().filter(|t| t.before == 0) { t.before=id; }
        paragraphs.push(Paragraph {id,style:ParagraphStyle::Normal,format:ParagraphFormat::default(),content:vec![Inline::Text(String::new())]});
    }
    let d = Document {
        tables: document_tables,
        table_styles: table_styles::read(find(&entries, "word/styles.xml")?)?,
        styles: styles::read(find(&entries, "word/styles.xml")?)?,
        paragraphs,
        notes: document_notes,
        sources: source_list,
        page,
    };
    d.validate()?;
    Ok(d)
}
/// Bounded whole-file read; refuses oversized files before allocation, including short reads.
pub fn read_file(path: &str, limit: usize) -> Result<Vec<u8>, Error> {
    if path.is_empty() || path.contains('\0') {
        return Err(Error::Invalid);
    }
    let f = plat::File::open(path, plat::FO_READ).ok_or(Error::Io)?;
    let size = f.size().ok_or(Error::Io)?;
    if size > limit as u64 {
        return Err(Error::Limit);
    }
    let mut bytes = vec![0; size as usize];
    if f.read_at(0, &mut bytes) != Some(bytes.len()) || f.size() != Some(size) {
        return Err(Error::Io);
    }
    Ok(bytes)
}
/// Load a DOCX without altering the caller's current editing session on failure.
pub fn load_docx(path: &str) -> Result<Document, Error> {
    decode_docx(&read_file(path, 32 << 20)?)
}
/// Encode completely before the platform's temporary-file/rename write.
pub fn save_docx(path: &str, d: &Document) -> Result<(), Error> {
    if path.is_empty() || path.contains('\0') {
        return Err(Error::Invalid);
    }
    let bytes = encode_docx(d)?;
    if plat::write_file(path, &bytes) {
        Ok(())
    } else {
        Err(Error::Io)
    }
}
