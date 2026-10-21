// Bounded OPC inspection and original-byte preservation for foreign Word documents.
// New native library code. Unrepresented content is never exposed as an editable model.
use crate::{Error, decode_docx, encode_docx, find, parse, read_file, unsupported};
use alloc::{collections::BTreeSet, format, string::String, vec::Vec};
use archive::{Entry, Limits};
use base::xml::Node;
use doc_core::Document;

const REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const MAIN: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const STRICT_W: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
const PREVIEW_BYTES: usize = 1 << 20;
const PREVIEW_PARAGRAPHS: usize = 4096;

/// Whether the entire document is represented by the editable model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocxMode {
    /// Every supported package component has passed the strict editable codec.
    Editable,
    /// The original package is retained; the text excerpt is not an editable conversion.
    Preserved,
}

/// Deterministic inspection results for a bounded, structurally checked DOCX package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocxReport {
    /// Editing capability; preservation does not imply rendering or editing compatibility.
    pub mode: DocxMode,
    /// Main document part resolved through the package relationship.
    pub main_part: String,
    /// Number of ZIP parts, including content types and relationships.
    pub part_count: usize,
    /// Human-readable reasons and feature descriptions, capped at 64 distinct entries.
    pub diagnostics: Vec<String>,
    /// The reading excerpt reached its independent paragraph or text limit.
    pub preview_truncated: bool,
}

/// A DOCX session retaining its exact original archive alongside an optional full model.
/// Unknown features remain intact; individually proven text leaves may be edited without
/// model conversion. Saving without edits returns the original bytes, including compression,
/// ordering, metadata, images, fields and unknown extension parts.
pub struct DocxFile {
    original: Vec<u8>,
    document: Option<Document>,
    preview: Vec<String>,
    report: DocxReport,
    text: crate::preserved_text::TextSession,
}

impl DocxFile {
    /// Inspect a conventional, unencrypted DOCX with bounded ZIP/XML processing.
    /// Macro-enabled main parts or VBA payloads are rejected. External relationships are
    /// retained as data only; this library never follows links or executes embedded content.
    pub fn open(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
            return Err(unsupported("Verschlüsselte DOCX- oder binäre DOC-Datei"));
        }
        let entries = archive::read(bytes, Limits::default()).map_err(Error::Archive)?;
        let content_types = content_types(&entries)?;
        let main_part = relationships(&entries, &content_types)?;
        if content_type(&content_types, &main_part) != Some(MAIN) {
            return Err(unsupported("Kein makrofreies DOCX-Hauptdokument"));
        }
        let root = parse(find(&entries, &main_part)?)?;
        if !(root.is(crate::package::W, "document") || root.is(STRICT_W, "document"))
            || !root.text.is_empty()
            || root.children.iter().filter(|n| word(n, "body")).count() != 1
        {
            return Err(Error::Invalid);
        }
        let mut report = DocxReport {
            mode: DocxMode::Editable,
            main_part,
            part_count: entries.len(),
            diagnostics: Vec::new(),
            preview_truncated: false,
        };
        let document = match decode_docx(bytes) {
            Ok(document) => Some(document),
            Err(error) => {
                report.mode = DocxMode::Preserved;
                issue(
                    &mut report,
                    String::from(
                        "Originaldatei bleibt vollständig erhalten. Der Textauszug bildet Seitenlayout, Felder und Objekte nicht vollständig ab.",
                    ),
                );
                issue(
                    &mut report,
                    format!(
                        "Bearbeitbares Profil nicht vollständig erfüllt: {}",
                        reason(error)
                    ),
                );
                inspect(&root, &mut report);
                for entry in &entries {
                    inspect_part(entry, &mut report);
                }
                None
            }
        };
        let mut preview = Vec::new();
        let mut remaining = PREVIEW_BYTES;
        if let Some(body) = root.children.iter().find(|n| word(n, "body")) {
            paragraphs(
                body,
                &mut preview,
                &mut remaining,
                &mut report.preview_truncated,
            );
        }
        let text = crate::preserved_text::TextSession::new(
            &entries,
            &report.main_part,
            &root,
            document.is_none(),
            &content_types,
        )?;
        if document.is_none() && text.runs().is_empty() {
            issue(
                &mut report,
                String::from("Keine sicher einzeln bearbeitbaren Textbereiche vorhanden."),
            );
        }
        Ok(Self {
            original: bytes.to_vec(),
            document,
            preview,
            report,
            text,
        })
    }

    /// Read and inspect a file, refusing archives over 32 MiB before allocation.
    pub fn load(path: &str) -> Result<Self, Error> {
        Self::open(&read_file(path, 32 << 20)?)
    }

    /// Entire editable document, or `None` when any unsupported content must be preserved.
    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }

    /// Main-story plain-text excerpt, excluding separate headers, footers and note parts.
    /// This deliberately does not claim layout fidelity.
    /// Field instructions, deleted revisions and unsupported alternate representations
    /// are omitted; cached visible field text and table-cell paragraphs may be included.
    pub fn preview(&self) -> &[String] {
        &self.preview
    }

    /// Original package's capability and compatibility diagnostics.
    pub fn report(&self) -> &DocxReport {
        &self.report
    }

    /// Safe individual text leaves in the preserved main story and related headers, footers,
    /// footnotes and endnotes. Parts resolve through OPC relationships and content types.
    /// Signed/protected packages, fields, revisions and content controls are not editable;
    /// a complex field or tracked revision disables its whole containing story part.
    pub fn text_runs(&self) -> &[crate::DocxTextRun] {
        self.text.runs()
    }

    /// Replace exactly one eligible text leaf, verifying its current text first.
    /// Paragraph/run structure and every other package payload remain unchanged.
    pub fn replace_text(
        &mut self,
        index: usize,
        expected: &str,
        replacement: &str,
    ) -> Result<(), Error> {
        self.text.replace(index, expected, replacement)?;
        self.refresh_preview();
        Ok(())
    }

    /// Whether preserved text differs from the last saved checkpoint.
    pub fn dirty(&self) -> bool {
        self.text.dirty()
    }
    /// Monotonic preserved-text revision, including successful undo and redo operations.
    /// Saving and replacing text with identical text leave the revision unchanged.
    pub fn revision(&self) -> u64 { self.text.revision() }

    /// Establish the saved checkpoint after a successful save, retaining undo history.
    pub fn mark_saved(&mut self) {
        self.text.mark_saved();
    }

    /// Undo the most recent preserved text edit, within the bounded history.
    pub fn undo(&mut self) -> bool {
        let changed = self.text.undo();
        if changed {
            self.refresh_preview();
        }
        changed
    }

    /// Redo the next preserved text edit.
    pub fn redo(&mut self) -> bool {
        let changed = self.text.redo();
        if changed {
            self.refresh_preview();
        }
        changed
    }

    /// Whether an earlier preserved text edit is available.
    pub fn can_undo(&self) -> bool {
        self.text.can_undo()
    }

    /// Whether an undone preserved text edit is available.
    pub fn can_redo(&self) -> bool {
        self.text.can_redo()
    }

    fn refresh_preview(&mut self) {
        if let Some(root) = self.text.current_root() {
            self.preview.clear();
            self.report.preview_truncated = false;
            let mut remaining = PREVIEW_BYTES;
            if let Some(body) = root.children.iter().find(|n| word(n, "body")) {
                paragraphs(
                    body,
                    &mut self.preview,
                    &mut remaining,
                    &mut self.report.preview_truncated,
                );
            }
        }
    }

    /// Produce original bytes for unchanged content, or serialize a changed fully
    /// represented model. Passing a model to a preservation-only session is an error.
    /// None saves preserved text edits if present, otherwise copies the original exactly.
    pub fn encode(&self, document: Option<&Document>) -> Result<Vec<u8>, Error> {
        match (self.document.as_ref(), document) {
            (_, None) => self.text.encode(&self.original),
            (Some(before), Some(after)) if before == after => Ok(self.original.clone()),
            (Some(_), Some(after)) => encode_docx(after),
            (None, Some(_)) => Err(unsupported(
                "Originalansicht ist schreibgeschützt; keine verlustbehaftete Konvertierung",
            )),
        }
    }

    /// Encode before atomic temporary-file/rename writing; a failed edit cannot overwrite
    /// a destination. Copying a preserved document retains every original archive byte.
    pub fn save(&self, path: &str, document: Option<&Document>) -> Result<(), Error> {
        if path.is_empty() || path.contains('\0') {
            return Err(Error::Invalid);
        }
        let bytes = self.encode(document)?;
        if plat::write_file(path, &bytes) {
            Ok(())
        } else {
            Err(Error::Io)
        }
    }
}

fn reason(error: Error) -> String {
    match error {
        Error::Unsupported(reason) => reason,
        Error::Invalid => {
            String::from("Paketstruktur oder Dokumentkennungen außerhalb des Profils")
        }
        Error::Limit => String::from("Grenzen des bearbeitbaren Dokumentmodells"),
        Error::Model(_) => {
            String::from("Dokumentmodell kann diese Inhalte nicht vollständig darstellen")
        }
        Error::Xml(_) => String::from("XML-Struktur außerhalb des bearbeitbaren Profils"),
        Error::Archive(_) => String::from("ZIP-Eigenschaften außerhalb des bearbeitbaren Profils"),
        Error::Io => String::from("Dateizugriff"),
    }
}
fn issue(report: &mut DocxReport, text: String) {
    if report.diagnostics.len() < 64 && !report.diagnostics.contains(&text) {
        report.diagnostics.push(text);
    }
}
fn word(node: &Node, name: &str) -> bool {
    node.is(crate::package::W, name) || node.is(STRICT_W, name)
}
fn macro_type(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("macroenabled") || lower.contains("vbaproject") || lower.contains("vbadata")
}

pub(super) struct Types {
    defaults: Vec<(String, String)>,
    overrides: Vec<(String, String)>,
}
pub(super) fn content_type<'a>(types: &'a Types, name: &str) -> Option<&'a str> {
    types
        .overrides
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, t)| t.as_str())
        .or_else(|| {
            name.rsplit_once('.').and_then(|(_, extension)| {
                types
                    .defaults
                    .iter()
                    .find(|(e, _)| e.eq_ignore_ascii_case(extension))
                    .map(|(_, t)| t.as_str())
            })
        })
}
fn content_types(entries: &[Entry]) -> Result<Types, Error> {
    let root = parse(find(entries, "[Content_Types].xml")?)?;
    crate::container(&root, CT, "Types")?;
    let mut types = Types {
        defaults: Vec::new(),
        overrides: Vec::new(),
    };
    for node in &root.children {
        if !node.children.is_empty() || !node.text.is_empty() || node.attrs.len() != 2 {
            return Err(Error::Invalid);
        }
        let value = node.attr("", "ContentType").ok_or(Error::Invalid)?;
        if value.is_empty() || macro_type(value) {
            return Err(unsupported("Makros oder ungültiger Inhaltstyp"));
        }
        if node.is(CT, "Default") {
            let extension = node.attr("", "Extension").ok_or(Error::Invalid)?;
            if extension.is_empty()
                || extension.contains(['/', '\\', '.'])
                || types
                    .defaults
                    .iter()
                    .any(|(e, _)| e.eq_ignore_ascii_case(extension))
            {
                return Err(Error::Invalid);
            }
            types
                .defaults
                .push((String::from(extension), String::from(value)));
        } else if node.is(CT, "Override") {
            let name = node
                .attr("", "PartName")
                .and_then(|n| n.strip_prefix('/'))
                .ok_or(Error::Invalid)?;
            if resolve("", name)? != name
                || types.overrides.iter().any(|(n, _)| n == name)
                || !entries.iter().any(|e| e.name == name)
            {
                return Err(Error::Invalid);
            }
            types
                .overrides
                .push((String::from(name), String::from(value)));
        } else {
            return Err(Error::Invalid);
        }
    }
    for entry in entries {
        if entry.name == "[Content_Types].xml" {
            continue;
        }
        if content_type(&types, &entry.name).is_none() {
            return Err(Error::Invalid);
        }
        if entry.name.to_ascii_lowercase().contains("vbaproject")
            || entry.name.to_ascii_lowercase().ends_with("vbadata.xml")
        {
            return Err(unsupported("VBA-Inhalte werden nicht unterstützt"));
        }
    }
    Ok(types)
}

// Resolve OPC part-relative paths without touching a filesystem or following a URL.
pub(super) fn resolve(source: &str, target: &str) -> Result<String, Error> {
    if target.is_empty()
        || target.contains(['\\', ':', '%', '?', '#'])
        || target.chars().any(char::is_control)
    {
        return Err(unsupported("Nicht unterstützte interne Paketadresse"));
    }
    let mut components: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        source
            .rsplit_once('/')
            .map_or(Vec::new(), |(dir, _)| dir.split('/').collect())
    };
    for part in target.strip_prefix('/').unwrap_or(target).split('/') {
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
    Ok(components.join("/"))
}
pub(super) fn relationship_source(name: &str) -> Result<String, Error> {
    if name == "_rels/.rels" {
        return Ok(String::new());
    }
    let name = name.strip_suffix(".rels").ok_or(Error::Invalid)?;
    if let Some((dir, file)) = name.rsplit_once("/_rels/") {
        if file.is_empty() || file.contains('/') {
            return Err(Error::Invalid);
        }
        Ok(format!("{dir}/{file}"))
    } else if let Some(file) = name.strip_prefix("_rels/") {
        if file.is_empty() || file.contains('/') {
            return Err(Error::Invalid);
        }
        Ok(String::from(file))
    } else {
        Err(Error::Invalid)
    }
}
fn relationships(entries: &[Entry], types: &Types) -> Result<String, Error> {
    find(entries, "_rels/.rels")?;
    let mut main = None;
    for entry in entries.iter().filter(|e| e.name.ends_with(".rels")) {
        if content_type(types, &entry.name)
            != Some("application/vnd.openxmlformats-package.relationships+xml")
        {
            return Err(Error::Invalid);
        }
        let source = relationship_source(&entry.name)?;
        if !source.is_empty() && !entries.iter().any(|e| e.name == source) {
            return Err(Error::Invalid);
        }
        let root = parse(&entry.data)?;
        crate::container(&root, REL, "Relationships")?;
        let mut ids = BTreeSet::new();
        for node in &root.children {
            if !node.is(REL, "Relationship")
                || !node.text.is_empty()
                || !node.children.is_empty()
                || node.attrs.len() < 3
                || node.attrs.len() > 4
                || node.attrs.iter().any(|(n, _)| {
                    !n.ns.is_empty()
                        || !matches!(n.local.as_str(), "Id" | "Type" | "Target" | "TargetMode")
                })
            {
                return Err(Error::Invalid);
            }
            let id = node.attr("", "Id").ok_or(Error::Invalid)?;
            let kind = node.attr("", "Type").ok_or(Error::Invalid)?;
            let target = node.attr("", "Target").ok_or(Error::Invalid)?;
            if id.is_empty() || kind.is_empty() || target.is_empty() || !ids.insert(id) {
                return Err(Error::Invalid);
            }
            if macro_type(kind) {
                return Err(unsupported("VBA-Verknüpfung"));
            }
            let external = match node.attr("", "TargetMode") {
                None | Some("Internal") => false,
                Some("External") => true,
                _ => return Err(Error::Invalid),
            };
            let office = matches!(
                kind,
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"
                    | "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument"
            );
            if office && source.is_empty() {
                if external || main.is_some() {
                    return Err(Error::Invalid);
                }
                main = Some(resolve(&source, target)?);
            }
            if !external {
                let part = resolve(&source, target)?;
                if !entries.iter().any(|e| e.name == part) {
                    return Err(Error::Invalid);
                }
            }
        }
    }
    main.ok_or(Error::Invalid)
}

fn inspect_part(entry: &Entry, report: &mut DocxReport) {
    let name = entry.name.as_str();
    let message = if name.contains("/media/") {
        Some("Bilder und Medien")
    } else if name.contains("/embeddings/") {
        Some("Eingebettete Dateien und Objekte")
    } else if name.contains("/charts/") {
        Some("Diagramme")
    } else if name.contains("/theme/") {
        Some("Dokumentdesign und Schriftzuordnungen")
    } else if name.ends_with("numbering.xml") {
        Some("Listen und Nummerierung")
    } else if name.contains("comments") {
        Some("Kommentare")
    } else if name.contains("/header") || name.contains("/footer") {
        Some("Kopf- und Fußzeilen")
    } else if name.starts_with("docProps/") {
        Some("Dokumenteigenschaften")
    } else if name.ends_with("footnotes.xml") || name.ends_with("endnotes.xml") {
        Some("Fußnoten und Endnoten")
    } else if name.starts_with("_xmlsignatures/") {
        Some("Digitale Signaturen (ungeprüft, unverändert aufbewahrt)")
    } else {
        None
    };
    if let Some(message) = message {
        issue(report, String::from(message));
    }
}
fn inspect(node: &Node, report: &mut DocxReport) {
    let message = if word(node, "tbl") {
        Some("Tabellen")
    } else if word(node, "drawing") || word(node, "pict") {
        Some("Grafiken und Zeichnungsobjekte")
    } else if word(node, "hyperlink") {
        Some("Hyperlinks (werden nicht aufgerufen)")
    } else if word(node, "fldSimple") || word(node, "fldChar") {
        Some("Dokumentfelder (gespeicherte Ergebnisse, keine Neuberechnung)")
    } else if word(node, "ins")
        || word(node, "del")
        || word(node, "moveFrom")
        || word(node, "moveTo")
    {
        Some("Nachverfolgte Änderungen")
    } else if word(node, "sdt") {
        Some("Inhaltssteuerelemente")
    } else if word(node, "altChunk") {
        Some("Importierte Dokumentteile")
    } else if node.name.local == "oMath" || node.name.local == "oMathPara" {
        Some("Mathematische Formeln")
    } else if node.is(crate::package::MC, "AlternateContent") {
        Some("Alternative Objektdarstellungen (nicht im Textauszug)")
    } else {
        None
    };
    if let Some(message) = message {
        issue(report, String::from(message));
    }
    for child in &node.children {
        inspect(child, report);
    }
}
fn skipped(node: &Node) -> bool {
    word(node, "del") || word(node, "moveFrom") || node.is(crate::package::MC, "AlternateContent")
}
fn paragraphs(node: &Node, result: &mut Vec<String>, remaining: &mut usize, truncated: &mut bool) {
    if skipped(node) {
        return;
    }
    if word(node, "p") {
        if result.len() >= PREVIEW_PARAGRAPHS || *remaining == 0 {
            *truncated = true;
            return;
        }
        let mut text = String::new();
        for child in &node.children {
            excerpt(child, &mut text, remaining, truncated);
        }
        result.push(text);
        // Text boxes can contain nested paragraphs; collect them once, separately.
        for child in &node.children {
            nested_paragraphs(child, result, remaining, truncated);
        }
    } else {
        for child in &node.children {
            paragraphs(child, result, remaining, truncated);
        }
    }
}
fn nested_paragraphs(
    node: &Node,
    result: &mut Vec<String>,
    remaining: &mut usize,
    truncated: &mut bool,
) {
    if skipped(node) {
        return;
    }
    if word(node, "p") {
        paragraphs(node, result, remaining, truncated);
    } else {
        for child in &node.children {
            nested_paragraphs(child, result, remaining, truncated);
        }
    }
}
fn append(text: &str, out: &mut String, remaining: &mut usize, truncated: &mut bool) {
    let mut end = text.len().min(*remaining);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    out.push_str(&text[..end]);
    if end < text.len() {
        *remaining = 0;
        *truncated = true;
    } else {
        *remaining -= end;
    }
}
fn excerpt(node: &Node, out: &mut String, remaining: &mut usize, truncated: &mut bool) {
    if skipped(node) || word(node, "p") {
        return;
    }
    if word(node, "t") {
        append(&node.text, out, remaining, truncated);
    } else if word(node, "tab") {
        append("\t", out, remaining, truncated);
    } else if word(node, "br") || word(node, "cr") {
        append("\n", out, remaining, truncated);
    } else if word(node, "noBreakHyphen") {
        append("\u{2011}", out, remaining, truncated);
    } else if word(node, "softHyphen") {
        append("\u{ad}", out, remaining, truncated);
    } else {
        for child in &node.children {
            excerpt(child, out, remaining, truncated);
        }
    }
}
