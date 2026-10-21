// Provenance-preserving edits of individual Word text leaves, independent of the model.
// Original XML syntax, run properties and every unrelated package payload are retained.
use crate::{DocxStory, Error, find, parse, unsupported};
use alloc::{format, string::String, vec::Vec};
use archive::{Entry, Limits};
use base::{
    encoding::{self, Encoding},
    xml::Node,
};

const W: &str = crate::package::W;
const STRICT_W: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
const MAX_TEXT: usize = 64 << 10;
const MAX_HISTORY: usize = 4 << 20;

/// An individually editable text leaf in an original Word story part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocxTextRun {
    /// Stable zero-based identifier accepted by DocxFile::replace_text.
    pub index: usize,
    /// Original OPC part path; shared headers are exposed only once.
    pub part: String,
    /// The containing story, resolved from the main part's relationship and content type.
    pub story: DocxStory,
    /// OOXML note identity, not its rendered number. None for the main story and headers/footers.
    pub note_id: Option<i64>,
    /// One-based XML paragraph number within this part, including table-cell paragraphs.
    pub paragraph: usize,
    /// Current decoded text. Run properties and paragraph structure are kept separately.
    pub text: String,
}

struct Span {
    part: usize,
    start: usize,
    end: usize,
    open: String,
    close: String,
}
struct Edit {
    index: usize,
    before: String,
    after: String,
}
struct TextPart {
    path: String,
    xml: String,
    encoding: Encoding,
    story: DocxStory,
}
pub(super) struct TextSession {
    parts: Vec<TextPart>,
    spans: Vec<Span>,
    runs: Vec<DocxTextRun>,
    original: Vec<String>,
    saved: Vec<String>,
    history: Vec<Edit>,
    position: usize,
    revision: u64,
    current_root: Option<Node>,
}
impl TextSession {
    pub(super) fn new(
        entries: &[Entry],
        path: &str,
        root: &Node,
        preserved: bool,
        types: &crate::preservation::Types,
    ) -> Result<Self, Error> {
        let mut session = Self {
            parts: Vec::new(),
            spans: Vec::new(),
            runs: Vec::new(),
            original: Vec::new(),
            saved: Vec::new(),
            history: Vec::new(),
            position: 0,
            revision: 0,
            current_root: None,
        };
        if !preserved || protected(entries, types) {
            return Ok(session);
        }
        session.add_part(entries, path, root, DocxStory::Main)?;
        for (path, story) in crate::preserved_stories::related(entries, path, types)? {
            let root = parse(find(entries, &path)?)?;
            // A malformed semantic association is retained, but never exposed for edits.
            if word(&root, story.root()) {
                session.add_part(entries, &path, &root, story)?;
            }
        }
        session.original = session.runs.iter().map(|run| run.text.clone()).collect();
        session.saved = session.original.clone();
        Ok(session)
    }
    fn add_part(
        &mut self,
        entries: &[Entry],
        path: &str,
        root: &Node,
        story: DocxStory,
    ) -> Result<(), Error> {
        let (xml, encoding) =
            encoding::decode(find(entries, path)?, 4 << 20).map_err(|_| Error::Invalid)?;
        let part = TextPart {
            path: String::from(path),
            xml,
            encoding,
            story,
        };
        let index = self.parts.len();
        self.parts.push(part);
        // Complex fields and tracked changes can cross paragraph boundaries within a story.
        if ["fldChar", "ins", "del", "moveFrom", "moveTo"]
            .iter()
            .any(|name| contains(root, name))
        {
            return Ok(());
        }
        let part = &self.parts[index];
        let mut cursor = 0;
        if part.xml.starts_with('\u{feff}') {
            cursor += '\u{feff}'.len_utf8();
        }
        if part.xml[cursor..].starts_with("<?xml ") {
            cursor += part.xml[cursor..].find("?>").ok_or(Error::Invalid)? + 2;
        }
        let mut paragraph = 0;
        scan(
            part,
            index,
            &mut cursor,
            root,
            None,
            true,
            None,
            &mut paragraph,
            &mut self.spans,
            &mut self.runs,
        )?;
        Ok(())
    }
    pub(super) fn runs(&self) -> &[DocxTextRun] {
        &self.runs
    }
    pub(super) fn current_root(&self) -> Option<&Node> {
        self.current_root.as_ref()
    }
    pub(super) fn dirty(&self) -> bool {
        self.runs
            .iter()
            .zip(&self.saved)
            .any(|(run, text)| &run.text != text)
    }
    pub(super) fn mark_saved(&mut self) {
        self.saved = self.runs.iter().map(|run| run.text.clone()).collect();
    }
    pub(super) fn can_undo(&self) -> bool {
        self.position != 0
    }
    pub(super) fn revision(&self) -> u64 { self.revision }
    pub(super) fn can_redo(&self) -> bool {
        self.position < self.history.len()
    }
    pub(super) fn undo(&mut self) -> bool {
        if !self.can_undo() {
            return false;
        }
        self.position -= 1;
        self.revision = self.revision.wrapping_add(1);
        let edit = &self.history[self.position];
        self.runs[edit.index].text = edit.before.clone();
        if self.spans[edit.index].part == 0 {
            self.current_root = self.root(0).ok();
        }
        true
    }
    pub(super) fn redo(&mut self) -> bool {
        if !self.can_redo() {
            return false;
        }
        let edit = &self.history[self.position];
        self.runs[edit.index].text = edit.after.clone();
        self.position += 1;
        self.revision = self.revision.wrapping_add(1);
        if self.spans[edit.index].part == 0 {
            self.current_root = self.root(0).ok();
        }
        true
    }
    pub(super) fn replace(
        &mut self,
        index: usize,
        expected: &str,
        replacement: &str,
    ) -> Result<(), Error> {
        let current = self
            .runs
            .get(index)
            .ok_or_else(|| unsupported("Textbereich ist nicht einzeln bearbeitbar"))?;
        if current.text != expected {
            return Err(unsupported("Textbereich wurde inzwischen geändert"));
        }
        if replacement.len() > MAX_TEXT {
            return Err(Error::Limit);
        }
        if replacement.contains(['\r', '\n', '\t']) {
            return Err(unsupported(
                "Neue Absätze und Tabulatoren benötigen eine vollständige Dokumentbearbeitung",
            ));
        }
        base::xml::escape(replacement)?;
        if expected == replacement {
            return Ok(());
        }
        let before = current.text.clone();
        self.runs[index].text = String::from(replacement);
        let part = self.spans[index].part;
        let root = match self.root(part) {
            Ok(root) => root,
            Err(error) => {
                self.runs[index].text = before;
                return Err(error);
            }
        };
        self.history.truncate(self.position);
        self.history.push(Edit {
            index,
            before,
            after: String::from(replacement),
        });
        while self.history.len() > 64
            || self
                .history
                .iter()
                .map(|edit| edit.before.len() + edit.after.len())
                .sum::<usize>()
                > MAX_HISTORY
        {
            self.history.remove(0);
        }
        self.position = self.history.len();
        self.revision = self.revision.wrapping_add(1);
        if part == 0 {
            self.current_root = Some(root);
        }
        Ok(())
    }
    fn patched_xml(&self, part: usize) -> Result<String, Error> {
        let xml = &self.parts[part].xml;
        let mut out = String::new();
        let mut previous = 0;
        for (index, span) in self.spans.iter().enumerate() {
            if span.part != part || self.runs[index].text == self.original[index] {
                continue;
            }
            out.push_str(&xml[previous..span.start]);
            out.push_str(&span.open);
            out.push_str(&base::xml::escape(&self.runs[index].text)?);
            out.push_str(&span.close);
            previous = span.end;
            if out.len() > 4 << 20 {
                return Err(Error::Limit);
            }
        }
        out.push_str(&xml[previous..]);
        if out.len() > 4 << 20 {
            return Err(Error::Limit);
        }
        Ok(out)
    }
    fn root(&self, part: usize) -> Result<Node, Error> {
        parse(&self.encoded_xml(part)?)
    }
    fn encoded_xml(&self, part: usize) -> Result<Vec<u8>, Error> {
        let bytes = encoding::encode(&self.patched_xml(part)?, self.parts[part].encoding);
        if bytes.len() > 4 << 20 {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
    pub(super) fn encode(&self, original: &[u8]) -> Result<Vec<u8>, Error> {
        if self
            .runs
            .iter()
            .zip(&self.original)
            .all(|(run, text)| &run.text == text)
        {
            return Ok(original.to_vec());
        }
        let mut entries = archive::read(original, Limits::default()).map_err(Error::Archive)?;
        for (part, source) in self.parts.iter().enumerate() {
            if !self
                .spans
                .iter()
                .enumerate()
                .any(|(i, span)| span.part == part && self.runs[i].text != self.original[i])
            {
                continue;
            }
            let entry = entries
                .iter_mut()
                .find(|entry| entry.name == source.path)
                .ok_or(Error::Invalid)?;
            entry.data = self.encoded_xml(part)?;
        }
        archive::write(&entries, Limits::default()).map_err(Error::Archive)
    }
}

fn word(node: &Node, local: &str) -> bool {
    node.is(W, local) || node.is(STRICT_W, local)
}
fn contains(node: &Node, local: &str) -> bool {
    word(node, local) || node.children.iter().any(|child| contains(child, local))
}
fn protected(entries: &[Entry], types: &crate::preservation::Types) -> bool {
    let mut settings = Vec::new();
    for entry in entries {
        if crate::preservation::content_type(types, &entry.name)
            != Some("application/vnd.openxmlformats-package.relationships+xml")
        {
            continue;
        }
        let Ok(relationships) = parse(&entry.data) else {
            return true;
        };
        let Ok(source) = crate::preservation::relationship_source(&entry.name) else {
            return true;
        };
        for relationship in &relationships.children {
            if relationship
                .attr("", "Type")
                .is_some_and(|kind| kind.to_ascii_lowercase().contains("digital-signature"))
            {
                return true;
            }
            if !matches!(
                relationship.attr("", "Type"),
                Some(
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings"
                ) | Some("http://purl.oclc.org/ooxml/officeDocument/relationships/settings")
            ) {
                continue;
            }
            // No external settings are fetched; their protection state is unknown.
            if relationship.attr("", "TargetMode") == Some("External") {
                return true;
            }
            let Some(target) = relationship.attr("", "Target") else {
                return true;
            };
            let Ok(target) = crate::preservation::resolve(&source, target) else {
                return true;
            };
            settings.push(target);
        }
    }
    entries.iter().any(|entry| {
        let name = entry.name.to_ascii_lowercase();
        let content_type = crate::preservation::content_type(types, &entry.name)
            .unwrap_or("")
            .to_ascii_lowercase();
        if name.starts_with("_xmlsignatures/") || content_type.contains("digital-signature") {
            return true;
        }
        if !(name.ends_with(".xml")
            || name.ends_with(".rels")
            || content_type.ends_with("+xml")
            || matches!(content_type.as_str(), "application/xml" | "text/xml")
            || settings.contains(&entry.name))
        {
            return false;
        }
        let Ok((text, _)) = encoding::decode(&entry.data, 4 << 20) else {
            return true;
        };
        if text.to_ascii_lowercase().contains("digital-signature") {
            return true;
        }
        let Ok(node) = base::xml::parse(&text) else {
            return true;
        };
        contains(&node, "documentProtection") || contains(&node, "trackRevisions")
    })
}
fn whitespace(xml: &str, cursor: &mut usize) {
    while xml
        .as_bytes()
        .get(*cursor)
        .is_some_and(|b| b" \t\r\n".contains(b))
    {
        *cursor += 1;
    }
}
fn tag_end(xml: &str, start: usize) -> Result<usize, Error> {
    let mut quote = None;
    for (offset, byte) in xml.as_bytes()[start..].iter().copied().enumerate() {
        if let Some(end) = quote {
            if byte == end {
                quote = None;
            }
        } else if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
        } else if byte == b'>' {
            return Ok(start + offset + 1);
        }
    }
    Err(Error::Invalid)
}
// The independent XML parser has already validated expanded names, nesting, attribute
// uniqueness and entities. This walk maps that exact tree to original lexical spans;
// it does not infer semantics from prefixes, raw names or string searches.
fn scan(
    part: &TextPart,
    part_index: usize,
    cursor: &mut usize,
    node: &Node,
    parent: Option<&Node>,
    eligible: bool,
    note_id: Option<i64>,
    paragraph: &mut usize,
    spans: &mut Vec<Span>,
    runs: &mut Vec<DocxTextRun>,
) -> Result<(), Error> {
    let xml = &part.xml;
    whitespace(xml, cursor);
    let start = *cursor;
    if !xml[start..].starts_with('<') {
        return Err(Error::Invalid);
    }
    let opening_end = tag_end(xml, start)?;
    let empty = xml[..opening_end].ends_with("/>");
    let parent_is = |name: &str| parent.is_some_and(|node| word(node, name));
    let allowed = eligible
        && (node.name.ns == W || node.name.ns == STRICT_W)
        && match node.name.local.as_str() {
            "document" | "hdr" | "ftr" | "footnotes" | "endnotes" => {
                parent.is_none() && node.name.local == part.story.root()
            }
            "body" => parent_is("document"),
            "footnote" => parent_is("footnotes"),
            "endnote" => parent_is("endnotes"),
            "tbl" | "p" => ["body", "tc", "hdr", "ftr", "footnote", "endnote"]
                .iter()
                .any(|n| parent_is(n)),
            "tr" => parent_is("tbl"),
            "tc" => parent_is("tr"),
            "r" => parent_is("p"),
            "t" => parent_is("r"),
            _ => false,
        };
    let is_note = word(node, "footnote") || word(node, "endnote");
    let note_id = if is_note {
        node.attr(&node.name.ns, "id")
            .and_then(|s| s.parse::<i64>().ok())
    } else {
        note_id
    };
    let allowed = allowed
        && (!is_note
            || (note_id.is_some_and(|id| id >= 0)
                && matches!(node.attr(&node.name.ns, "type"), None | Some("normal"))));
    let allowed = allowed
        && !(word(node, "r")
            && ["vanish", "webHidden", "specVanish"]
                .iter()
                .any(|name| contains(node, name)));
    if word(node, "p") {
        *paragraph += 1;
    }
    *cursor = opening_end;
    if !empty {
        for child in &node.children {
            scan(
                part,
                part_index,
                cursor,
                child,
                Some(node),
                allowed,
                note_id,
                paragraph,
                spans,
                runs,
            )?;
        }
        if node.children.is_empty() {
            *cursor += xml[*cursor..].find('<').ok_or(Error::Invalid)?;
        }
        whitespace(xml, cursor);
        if !xml[*cursor..].starts_with("</") {
            return Err(Error::Invalid);
        }
        *cursor = tag_end(xml, *cursor)?;
    }
    if allowed
        && word(node, "t")
        && node.children.is_empty()
        && node.text.len() <= MAX_TEXT
        && node.attrs.iter().all(|(name, value)| {
            name.ns == crate::package::XML && name.local == "space" && value == "preserve"
        })
    {
        let raw_open = &xml[start..opening_end];
        let name_end = raw_open[1..]
            .find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
            .ok_or(Error::Invalid)?
            + 1;
        let name = &raw_open[1..name_end];
        let cut = raw_open.len() - if empty { 2 } else { 1 };
        let mut open = String::from(&raw_open[..cut]);
        if node.attr(crate::package::XML, "space").is_none() {
            open.push_str(" xml:space=\"preserve\"");
        }
        open.push('>');
        spans.push(Span {
            part: part_index,
            start,
            end: *cursor,
            open,
            close: format!("</{name}>"),
        });
        runs.push(DocxTextRun {
            index: runs.len(),
            part: part.path.clone(),
            story: part.story,
            note_id,
            paragraph: *paragraph,
            text: node.text.clone(),
        });
    }
    Ok(())
}
