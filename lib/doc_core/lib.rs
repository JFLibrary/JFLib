// Structured document and source model, with atomic edits and bounded undo/redo.
// New native functionality, independent of UI, file formats and operating system.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
use alloc::{format, string::String, vec, vec::Vec};
mod citation;
mod format;
mod tabs;
mod tables;
mod table_tree;
mod table_position;
pub use table_position::{TablePosition,TableHorizontal,TableVertical};
pub use table_tree::MAX_TABLE_DEPTH;
mod table_styles;
mod cell_borders;
pub use cell_borders::{BorderSide,BorderStyle,CellBorder,CellBorders};
pub use table_styles::{TableLook, TableRegion, TableRegionKind, TableStyle};
pub use tables::{Table, TableRow, TableCell, TableChange, CellAlignment, RowHeightRule};
mod character;
#[path="formula/mod.rs"] mod formula;
pub use formula::{Formula,FormulaError,FormulaResult};
pub use character::{Highlight, TextPositioning};
mod styles;
pub use styles::{FontFamily, StyleDefinition};
mod fragment;
pub use fragment::DocumentFragment;
mod notes;
mod paragraph_edit;
mod text_range;
pub use text_range::{ParagraphText, TextSpan};
mod story_range;
pub use story_range::{StoryParagraph, StoryText};
mod search;
mod source;
mod source_audit;
mod source_import;
mod source_library;
mod source_merge;
pub use citation::{CitationItem, CitationLocation};
pub use format::{Alignment, PageSettings, ParagraphFormat, ResolvedFormat, TextColor, TextFormat};
pub use tabs::{TabAlignment, TabLeader, TabStop, TabStops};
pub use notes::{Note, NoteKind};
pub use paragraph_edit::TextPosition;
pub use search::{SearchOptions, TextMatch};
pub use source::{Person, Source, SourceKind};
pub use source_audit::{SourceIssue, SourceIssueKind};
pub use source_import::{SourceMatch, SourceTarget};
pub use source_library::{LibraryEdit, SourceLibrary, SourceLibraryEditor};
pub use source_merge::{SourceCandidate, SourceField, SourceMerge};

/// Stable identity inside a document. IDs are never reused by an editing session.
pub type Id = u64;
/// A paragraph command applied atomically to a selection. Individual properties preserve
/// each paragraph's other overrides; Format replaces the complete set of overrides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParagraphChange {
    Style(ParagraphStyle),
    Format(ParagraphFormat),
    Properties(ParagraphFormat),
    /// Replace the effective tab list, preserving explicit suppression of inherited stops.
    Tabs(TabStops),
    Alignment(Alignment),
    PageBreakBefore(bool),
}
/// Stable paragraph-style identity. Definitions and inheritance belong to the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ParagraphStyle {
    #[default]
    Normal,
    Title,
    Heading1,
    Heading2,
    Named(u32),
}
/// Text or a structured citation; a citation remains linked to a source after edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    StyledText {
        text: String,
        format: TextFormat,
    },
    Citation {
        source: Id,
        locator: String,
    },
    CitationCluster(Vec<CitationItem>),
    /// Bounded live arithmetic, using the containing table for cell references.
    Formula(Formula),
    Bibliography,
    /// Stable note identity; a note has exactly one anchor, in the main body.
    NoteReference(Id),
}
impl Inline {
    /// Editable text, excluding generated citation and bibliography fields.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(t) | Self::StyledText { text: t, .. } => Some(t),
            _ => None,
        }
    }
    /// Character overrides; plain text and fields inherit the paragraph style.
    pub fn format(&self) -> TextFormat {
        match self {
            Self::StyledText { format, .. } => *format,
            _ => TextFormat::default(),
        }
    }
    /// Canonical text node: inherited formatting uses the compact plain-text variant.
    pub fn with_format(text: String, format: TextFormat) -> Self {
        if format == TextFormat::default() {
            Self::Text(text)
        } else {
            Self::StyledText { text, format }
        }
    }
}
/// A paragraph with stable identity and structured inline content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paragraph {
    pub id: Id,
    pub style: ParagraphStyle,
    pub format: ParagraphFormat,
    pub content: Vec<Inline>,
}
/// Document data. Public for codecs; validate() is mandatory at trust boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub tables: Vec<Table>,
    pub table_styles: Vec<TableStyle>,
    pub styles: Vec<StyleDefinition>,
    pub paragraphs: Vec<Paragraph>,
    pub sources: Vec<Source>,
    pub notes: Vec<Note>,
    pub page: PageSettings,
}
/// Invalid structure, dangling citation, source in use, or a resource budget exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Missing,
    InUse,
    Limit,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            tables: Vec::new(),
            table_styles: Vec::new(),
            styles: Vec::new(),
            paragraphs: vec![Paragraph {
                id: 1,
                style: ParagraphStyle::Normal,
                format: ParagraphFormat::default(),
                content: vec![Inline::Text(String::new())],
            }],
            sources: Vec::new(),
            notes: Vec::new(),
            page: PageSettings::default(),
        }
    }
}
impl Document {
    /// Lookup by stable source identity.
    pub fn source(&self, id: Id) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }
    /// Validate identities, source references, XML-compatible text and conservative budgets.
    pub fn validate(&self) -> Result<(), Error> {
        self.page.validate()?;
        self.validate_styles()?;
        self.validate_table_styles()?;
        if self.paragraphs.is_empty()
            || self.all_paragraphs().count() > 2048
            || self.notes.len() > 1024
            || self.sources.len() > 1024
            || self.all_paragraphs().flat_map(|p|&p.content).filter(|n|matches!(n,Inline::Formula(_))).count()>128
            || self.byte_size() > 2 << 20
        {
            return Err(Error::Limit);
        }
        let mut ids = alloc::collections::BTreeSet::new();
        for s in &self.sources {
            if s.id == 0 || s.id >= 0x7fff_ffff || !ids.insert(s.id) {
                return Err(Error::Invalid);
            }
            s.validate()?;
        }
        for note in &self.notes {
            if note.id == 0
                || note.id >= 0x7fff_ffff
                || !ids.insert(note.id)
                || note.paragraphs.is_empty()
            {
                return Err(Error::Invalid);
            }
            if note
                .paragraphs
                .iter()
                .flat_map(|p| &p.content)
                .any(|node| matches!(node, Inline::NoteReference(_) | Inline::Bibliography))
            {
                return Err(Error::Invalid);
            }
        }
        self.validate_tables(&mut ids)?;
        let mut anchored = alloc::collections::BTreeSet::new();
        for p in self.all_paragraphs() {
            self.style_definition(p.style).ok_or(Error::Missing)?;
            p.format.validate_overrides()?;
            self.resolve_paragraph(p).validate(self.page)?;
            if p.id == 0 || p.id >= 0x7fff_ffff || !ids.insert(p.id) || p.content.len() > 8192 {
                return Err(Error::Invalid);
            }
            for i in &p.content {
                match i {
                    Inline::Text(t) => check_text(t)?,
                    Inline::StyledText { text, format } => {
                        check_text(text)?;
                        format.validate()?;
                    }
                    Inline::Citation { source, locator } => {
                        if self.source(*source).is_none() {
                            return Err(Error::Missing);
                        }
                        check_text(locator)?;
                        if locator.len() > 128 || locator.contains(['"', '\\', '\n', '\r']) {
                            return Err(Error::Invalid);
                        }
                    }
                    Inline::CitationCluster(items) => {
                        citation::validate_items(items)?;
                        if items.iter().any(|item| self.source(item.source).is_none()) {
                            return Err(Error::Missing);
                        }
                    }
                    Inline::Bibliography => (),
                    Inline::Formula(f) => { f.validate().map_err(|_|Error::Invalid)?; },
                    Inline::NoteReference(id) => {
                        if self.note(*id).is_none() {
                            return Err(Error::Missing);
                        }
                        if !anchored.insert(*id) {
                            return Err(Error::Invalid);
                        }
                    }
                }
            }
        }
        if anchored.len() != self.notes.len() {
            return Err(Error::Missing);
        }
        self.validate_expanded_text()?;
        Ok(())
    }
    /// Approximate retained payload size for history budgets (including node overhead).
    pub fn byte_size(&self) -> usize {
        self.all_paragraphs()
            .map(|p| {
                64 + p
                    .content
                    .iter()
                    .map(|i| {
                        core::mem::size_of::<Inline>().max(64) + match i {
                            Inline::Text(t) | Inline::StyledText { text: t, .. } => t.len(),
                            Inline::Citation { locator, .. } => locator.len(),
                            Inline::CitationCluster(items) => {
                                items.iter().map(CitationItem::byte_size).sum()
                            }
                            Inline::Bibliography | Inline::NoteReference(_) => 0,
                            Inline::Formula(f) => f.expression.len()+f.number_format.len(),
                        }
                    })
                    .sum::<usize>()
            })
            .sum::<usize>()
            + self.sources.iter().map(Source::byte_size).sum::<usize>()
            + self.tables.iter().map(|t| 128 + t.columns.len()*4 + t.cells().count()*64).sum::<usize>()
            + self.notes.len() * core::mem::size_of::<Note>()
            + self.styles.iter().map(|s| core::mem::size_of::<StyleDefinition>() + s.name.len()).sum::<usize>()
    }
    /// Cached display text for a citation; this basic author/year rendering is not CSL/APA.
    pub fn citation_text(&self, source: Id, locator: &str) -> String {
        match self.source(source) {
            Some(s) => format!(
                "({}{}{}{})",
                s.citation_author(),
                if s.year.is_empty() { "" } else { ", " },
                s.year,
                if locator.is_empty() {
                    String::new()
                } else {
                    format!(", S. {locator}")
                }
            ),
            None => String::from("[Quelle fehlt]"),
        }
    }
    /// Paragraph text including live citation labels.
    pub fn paragraph_text(&self, p: &Paragraph) -> String {
        let mut s = String::new();
        for i in &p.content {
            match i {
                Inline::Text(t) | Inline::StyledText { text: t, .. } => s.push_str(t),
                Inline::Citation { source, locator } => {
                    s.push_str(&self.citation_text(*source, locator))
                }
                Inline::CitationCluster(items) => s.push_str(&self.citation_cluster_text(items)),
                Inline::Bibliography => s.push_str(&self.bibliography().join("\n")),
                Inline::NoteReference(id) => s.push_str(&self.note_marker(*id)),
                Inline::Formula(f) => s.push_str(&self.evaluate_formula(p.id,f).text),
            }
        }
        s
    }
    /// Plain-text view used for counts and explicit text exports.
    pub fn plain_text(&self) -> String {
        let mut text = self
            .body_paragraphs()
            .map(|p| self.paragraph_text(p))
            .collect::<Vec<_>>()
            .join("\n");
        for node in self.body_paragraphs().flat_map(|p| &p.content) {
            if let Inline::NoteReference(id) = node {
                if let Some(note) = self.note(*id) {
                    text.push('\n');
                    text.push_str(&self.note_marker(*id));
                    text.push(' ');
                    text.push_str(
                        &note
                            .paragraphs
                            .iter()
                            .map(|p| self.paragraph_text(p))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
            }
        }
        text
    }
    /// Deterministic basic list of cited sources, including notes at their anchor position.
    pub fn bibliography(&self) -> Vec<String> {
        self.cited_source_ids()
            .into_iter()
            .filter_map(|id| self.source(id))
            .map(Source::bibliography_text)
            .collect()
    }
}
fn check_text(t: &str) -> Result<(), Error> {
    if t.chars().all(base::xml::valid_char) {
        Ok(())
    } else {
        Err(Error::Invalid)
    }
}
/// One atomic edit; invalid edits leave document, redo history and dirty state untouched.
pub enum Edit {
    /// Insert a live formula at a UTF-8 caret, retaining adjacent text and formatting.
    InsertFormula { paragraph:Id, inline:usize, offset:usize, formula:Formula },
    /// Replace a calculation field without touching its neighbours, as one undo step.
    SetFormula { paragraph:Id, inline:usize, formula:Formula },
    InsertTable { paragraph: Id, offset: usize, rows: usize, columns: usize },
    ChangeTable { table: Id, change: TableChange },
    SetTableStyle { style: TableStyle, table: Option<Id> },
    /// Create or replace one definition; validation rejects cycles and dangling bases atomically.
    SetStyleDefinition(StyleDefinition),
    /// Remove one generated field by native position, including zero-width bibliography.
    /// Removing a note anchor removes its body in the same undo transaction.
    RemoveField {
        paragraph: Id,
        inline: usize,
    },
    /// Replace a rich selection, including complete fields, in one atomic transaction.
    /// Imported identities are remapped and exactly identical sources are reused.
    ReplaceStoryFragment {
        paragraph: Id,
        start: usize,
        end: usize,
        fragment: DocumentFragment,
    },
    /// Change all paragraphs touched by a story selection without changing their inline
    /// objects. An end caret at the beginning of the next paragraph excludes that paragraph.
    FormatStoryParagraphs {
        paragraph: Id,
        start: usize,
        end: usize,
        change: ParagraphChange,
    },
    /// Replace a UTF-8 selection in one complete story (main body or one note). Newlines
    /// optionally create paragraphs; false keeps them as soft line breaks inside a run.
    /// Crossing generated fields is rejected atomically, including the paragraph structure.
    ReplaceStoryText {
        paragraph: Id,
        start: usize,
        end: usize,
        text: String,
        paragraph_breaks: bool,
    },
    /// Apply character overrides over several paragraphs as a single history step.
    FormatStoryText {
        paragraph: Id,
        start: usize,
        end: usize,
        format: TextFormat,
    },
    /// Replace all character overrides in a story range as one undoable edit.
    /// Default formatting clears overrides and restores paragraph-style inheritance.
    /// Text, paragraph styles and generated fields retain their original structure;
    /// selections intersecting generated fields are rejected before committing.
    ReplaceStoryFormat {
        paragraph: Id,
        start: usize,
        end: usize,
        format: TextFormat,
    },
    /// Replace a UTF-8 range across adjacent formatted runs; generated fields are protected.
    ReplaceParagraphText {
        paragraph: Id,
        start: usize,
        end: usize,
        text: String,
    },
    /// Apply explicit character overrides across adjacent formatted runs as one transaction.
    FormatParagraphText {
        paragraph: Id,
        start: usize,
        end: usize,
        format: TextFormat,
    },
    /// Split a selection across adjacent formatted runs into a new paragraph atomically.
    SplitParagraphText {
        paragraph: Id,
        start: usize,
        end: usize,
    },
    /// Replace a selection in one text run with a paragraph boundary. Formatting and all
    /// following fields/notes move intact; returns the new paragraph identity. A heading
    /// split at its end continues in Normal style. The new half never repeats a page break.
    SplitParagraph {
        paragraph: Id,
        inline: usize,
        start: usize,
        end: usize,
    },
    /// Join adjacent paragraphs in one story, retaining the first identity/style and every
    /// inline object. Matching character overrides at the seam coalesce into one text run.
    JoinParagraphs {
        first: Id,
        second: Id,
    },
    /// Insert one note at a main-body UTF-8 text caret, preserving both text formatting and suffix.
    InsertNote {
        paragraph: Id,
        inline: usize,
        offset: usize,
        kind: NoteKind,
        text: String,
    },
    /// Convert a note without changing its identity, anchor or body.
    SetNoteKind {
        note: Id,
        kind: NoteKind,
    },
    /// Remove a note and its sole anchor in one undo transaction.
    RemoveNote(Id),
    /// Format a UTF-8 range in one text run, preserving its prefix/suffix and field neighbors.
    FormatText {
        paragraph: Id,
        inline: usize,
        start: usize,
        end: usize,
        format: TextFormat,
    },
    SetParagraphFormat {
        paragraph: Id,
        format: ParagraphFormat,
    },
    SetPage(PageSettings),
    /// Replace a previously found range only if its text still matches the expected value.
    ReplaceMatch {
        found: TextMatch,
        expected: String,
        replacement: String,
    },
    /// One undo transaction for all matching text, excluding generated fields.
    ReplaceAll {
        query: String,
        replacement: String,
        options: SearchOptions,
    },
    /// Replace an editable run. An empty paragraph exposes virtual run zero; writing a
    /// nonempty value creates its first text run, while an empty value preserves the structure.
    SetText {
        paragraph: Id,
        inline: usize,
        text: String,
    },
    SetStyle {
        paragraph: Id,
        style: ParagraphStyle,
    },
    AddParagraph {
        after: Id,
    },
    InsertBibliography {
        after: Id,
    },
    RemoveParagraph(Id),
    PutSource(Source),
    /// Import a complete list atomically, allocating IDs and skipping exact content duplicates.
    ImportSources(Vec<Source>),
    /// Merge two existing sources and redirect all citation occurrences in one transaction.
    MergeSources(SourceMerge),
    RemoveSource(Id),
    Cite {
        paragraph: Id,
        source: Id,
        locator: String,
    },
    /// Insert a citation at a UTF-8 caret inside one editable run, preserving its formatting.
    /// The suffix is always retained as editable text, including insertion at the run end.
    InsertCitation {
        paragraph: Id,
        inline: usize,
        offset: usize,
        items: Vec<CitationItem>,
    },
    /// Replace an existing citation's items in a single undo transaction.
    UpdateCitation {
        paragraph: Id,
        inline: usize,
        items: Vec<CitationItem>,
    },
    /// Replace one citation field by its current display text, disconnecting its sources.
    ConvertCitationToText {
        paragraph: Id,
        inline: usize,
    },
    RemoveCitation {
        paragraph: Id,
        inline: usize,
    },
}
/// Editing session with a saved checkpoint and bounded snapshot history.
pub struct Editor {
    doc: Document,
    saved: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
    next: Id,
    revision: u64,
    state: u64,
    undo_states: Vec<u64>,
    redo_states: Vec<u64>,
}
impl Editor {
    /// Create a validated editing session; loading starts with an empty history.
    pub fn new(doc: Document) -> Result<Self, Error> {
        doc.validate()?;
        let next = doc
            .all_paragraphs()
            .map(|p| p.id)
            .chain(doc.sources.iter().map(|s| s.id))
            .chain(doc.notes.iter().map(|note| note.id))
            .chain(doc.tables.iter().map(|t| t.id))
            .chain(doc.tables.iter().flat_map(|t| t.cells().map(|c| c.id)))
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(Error::Limit)?;
        Ok(Self {
            saved: doc.clone(),
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
            next,
            revision: 0,
            state: 0,
            undo_states: Vec::new(),
            redo_states: Vec::new(),
        })
    }
    /// Current immutable document.
    pub fn document(&self) -> &Document {
        &self.doc
    }
    /// Changes on every successful content edit/undo/redo; suitable for a layout cache.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Identity of this history state. Unlike revision(), undo/redo restore this identity,
    /// allowing a view to restore transaction-associated caret positions without model data.
    /// Identities are local to one Editor session; no-op/failed edits do not change them.
    pub fn state_id(&self) -> u64 {
        self.state
    }
    /// Content differs from the last successfully saved checkpoint.
    pub fn dirty(&self) -> bool {
        self.doc != self.saved
    }
    /// Call only after a successful durable write.
    pub fn mark_saved(&mut self) {
        self.saved = self.doc.clone();
    }
    /// Whether an undo step exists.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    /// Whether a redo step exists.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// Undo a complete model edit, including source/citation changes.
    pub fn undo(&mut self) -> bool {
        if let Some(d) = self.undo.pop() {
            self.redo_states.push(self.state);
            self.state = self.undo_states.pop().unwrap();
            self.redo.push(core::mem::replace(&mut self.doc, d));
            self.revision = self.revision.wrapping_add(1);
            true
        } else {
            false
        }
    }
    /// Redo a complete model edit.
    pub fn redo(&mut self) -> bool {
        if let Some(d) = self.redo.pop() {
            self.undo_states.push(self.state);
            self.state = self.redo_states.pop().unwrap();
            self.undo.push(core::mem::replace(&mut self.doc, d));
            self.revision = self.revision.wrapping_add(1);
            true
        } else {
            false
        }
    }
    /// Apply an edit; returns the created/affected identity. New sources use id=0.
    pub fn apply(&mut self, edit: Edit) -> Result<Id, Error> {
        self.apply_checked(edit, |_|Ok(()))
    }
    /// Validate an additional shared constraint before committing a transaction.
    /// A rejected edit leaves identities, history, saved state and revision untouched.
    pub fn apply_checked(&mut self, edit:Edit,check:impl FnOnce(&Document)->Result<(),Error>)->Result<Id,Error>{
        let mut d = self.doc.clone();
        let mut next = self.next;
        let mut allocate = || {
            let id = next;
            next = next.checked_add(1).ok_or(Error::Limit)?;
            Ok(id)
        };
        let id = match edit {
            Edit::InsertFormula{paragraph,inline,offset,formula}=>{d.insert_formula(paragraph,inline,offset,formula)?;paragraph},
            Edit::SetFormula{paragraph,inline,formula}=>{
                formula.validate().map_err(|_|Error::Invalid)?;
                let node=d.paragraph_mut(paragraph).ok_or(Error::Missing)?.content.get_mut(inline).ok_or(Error::Missing)?;
                if !matches!(node,Inline::Formula(_)){return Err(Error::Invalid);}*node=Inline::Formula(formula);paragraph
            },
            Edit::InsertTable {paragraph,offset,rows,columns} => d.insert_table(paragraph,offset,rows,columns,&mut allocate)?,
            Edit::ChangeTable {table,change} => { d.change_table(table,change,&mut allocate)?; table },
            Edit::SetTableStyle {style,table} => {
                let style_id=style.id.clone(); d.set_table_style(style)?;
                if let Some(id)=table {let look=d.table(id).ok_or(Error::Missing)?.look; d.change_table(id,TableChange::Style{style:Some(style_id),look},&mut allocate)?;}
                table.unwrap_or(0)
            },
            Edit::RemoveField { paragraph, inline } => {
                let p = d.paragraph_mut(paragraph).ok_or(Error::Missing)?;
                let node = p.content.get(inline).ok_or(Error::Missing)?;
                if node.text().is_some() { return Err(Error::Invalid); }
                let removed = p.content.remove(inline);
                let format = inline.checked_sub(1).and_then(|i| p.content.get(i)).map_or_else(TextFormat::default, Inline::format);
                p.content.insert(inline, Inline::with_format(String::new(), format));
                crate::text_range::coalesce(&mut p.content);
                if p.content.is_empty() { p.content.push(Inline::Text(String::new())); }
                if let Inline::NoteReference(note) = removed {
                    d.notes.retain(|n| n.id != note);
                }
                paragraph
            }
            Edit::ReplaceStoryFragment { paragraph, start, end, fragment } => {
                d.replace_story_fragment(paragraph, start, end, fragment, &mut allocate)?
            }
            Edit::FormatStoryParagraphs {
                paragraph, start, end, change,
            } => {
                d.change_story_paragraphs(paragraph, start, end, change)?;
                paragraph
            }
            Edit::ReplaceStoryText {
                paragraph, start, end, text, paragraph_breaks,
            } => {
                d.replace_story_range(paragraph, start, end, &text, paragraph_breaks, &mut allocate)?
            }
            Edit::FormatStoryText {
                paragraph, start, end, format,
            } => {
                d.format_story_range(paragraph, start, end, format, false)?;
                paragraph
            }
            Edit::ReplaceStoryFormat {
                paragraph, start, end, format,
            } => {
                d.format_story_range(paragraph, start, end, format, true)?;
                paragraph
            }
            Edit::ReplaceParagraphText {
                paragraph,
                start,
                end,
                text,
            } => {
                d.replace_paragraph_range(paragraph, start, end, &text)?;
                paragraph
            }
            Edit::FormatParagraphText {
                paragraph,
                start,
                end,
                format,
            } => {
                d.format_paragraph_range(paragraph, start, end, format, false)?;
                paragraph
            }
            Edit::SplitParagraphText {
                paragraph,
                start,
                end,
            } => {
                d.replace_paragraph_range(paragraph, start, end, "")?;
                let at = d.text_projection(paragraph)?.position(start)?;
                let created = allocate()?;
                d.split_paragraph(paragraph, at.inline, at.offset, at.offset, created)?;
                created
            }
            Edit::SplitParagraph {
                paragraph,
                inline,
                start,
                end,
            } => {
                let created = allocate()?;
                d.split_paragraph(paragraph, inline, start, end, created)?;
                created
            }
            Edit::JoinParagraphs { first, second } => {
                d.join_paragraphs(first, second)?;
                first
            }
            Edit::InsertNote {
                paragraph,
                inline,
                offset,
                kind,
                text,
            } => {
                let note = allocate()?;
                let body = allocate()?;
                d.insert_note(
                    CitationLocation { paragraph, inline },
                    offset,
                    note,
                    body,
                    kind,
                    text,
                )?;
                note
            }
            Edit::SetNoteKind { note, kind } => {
                d.notes
                    .iter_mut()
                    .find(|existing| existing.id == note)
                    .ok_or(Error::Missing)?
                    .kind = kind;
                note
            }
            Edit::RemoveNote(note) => {
                d.remove_note(note)?;
                note
            }
            Edit::SetPage(page) => {
                d.page = page;
                0
            }
            Edit::SetParagraphFormat { paragraph, format } => {
                d.paragraph_mut(paragraph).ok_or(Error::Missing)?.format = format;
                paragraph
            }
            Edit::FormatText {
                paragraph,
                inline,
                start,
                end,
                format,
            } => {
                format.validate()?;
                let p = d.paragraph_mut(paragraph).ok_or(Error::Missing)?;
                let node = p.content.get(inline).ok_or(Error::Missing)?;
                let text = node.text().ok_or(Error::Invalid)?;
                let selected = text.get(start..end).ok_or(Error::Invalid)?;
                if node.format() == format {
                    return Ok(paragraph);
                }
                let mut parts = Vec::new();
                if start > 0 {
                    parts.push(Inline::with_format(
                        String::from(&text[..start]),
                        node.format(),
                    ));
                }
                parts.push(Inline::with_format(String::from(selected), format));
                if end < text.len() {
                    parts.push(Inline::with_format(
                        String::from(&text[end..]),
                        node.format(),
                    ));
                }
                p.content.splice(inline..=inline, parts);
                paragraph
            }
            Edit::ReplaceMatch {
                found,
                expected,
                replacement,
            } => {
                let p = d.paragraph_mut(found.paragraph).ok_or(Error::Missing)?;
                search::replace(p, &found, Some(&expected), &replacement)?;
                found.paragraph
            }
            Edit::ReplaceAll {
                query,
                replacement,
                options,
            } => {
                check_text(&replacement)?;
                if replacement.len() > 4096 {
                    return Err(Error::Limit);
                }
                let matches = d.find_text(&query, options)?;
                for found in matches.iter().rev() {
                    let p = d.paragraph_mut(found.paragraph).ok_or(Error::Missing)?;
                    search::replace(p, found, None, &replacement)?;
                    if d.byte_size() > 2 << 20 {
                        return Err(Error::Limit);
                    }
                }
                matches.len() as Id
            }
            Edit::SetText {
                paragraph,
                inline,
                text,
            } => {
                let p = d.paragraph_mut(paragraph).ok_or(Error::Missing)?;
                if p.content.is_empty() && inline == 0 {
                    if !text.is_empty() {
                        p.content.push(Inline::Text(text));
                    }
                } else {
                    let i = p.content.get_mut(inline).ok_or(Error::Missing)?;
                    if i.text().is_none() {
                        return Err(Error::Invalid);
                    }
                    *i = Inline::with_format(text, i.format());
                }
                paragraph
            }
            Edit::SetStyle { paragraph, style } => {
                d.paragraph_mut(paragraph).ok_or(Error::Missing)?.style = style;
                paragraph
            }
            Edit::SetStyleDefinition(style) => {
                if let Some(existing) = d.styles.iter_mut().find(|s| s.id == style.id) { *existing = style; }
                else { d.styles.push(style); }
                0
            }
            Edit::AddParagraph { after } => {
                let group = d.paragraph_group_mut(after).ok_or(Error::Missing)?;
                let at = group
                    .iter()
                    .position(|p| p.id == after)
                    .ok_or(Error::Missing)?;
                let id = allocate()?;
                group.insert(
                    at + 1,
                    Paragraph {
                        id,
                        style: ParagraphStyle::Normal,
                        format: ParagraphFormat::default(),
                        content: vec![Inline::Text(String::new())],
                    },
                );
                id
            }
            Edit::InsertBibliography { after } => {
                let at = d
                    .paragraphs
                    .iter()
                    .position(|p| p.id == after)
                    .ok_or(Error::Missing)?;
                let id = allocate()?;
                d.paragraphs.insert(
                    at + 1,
                    Paragraph {
                        id,
                        style: ParagraphStyle::Normal,
                        format: ParagraphFormat::default(),
                        content: vec![Inline::Bibliography],
                    },
                );
                id
            }
            Edit::RemoveParagraph(id) => {
                d.remove_paragraph(id)?;
                id
            }
            Edit::PutSource(mut s) => {
                if s.id == 0 {
                    s.id = allocate()?;
                    d.sources.push(s.clone());
                } else {
                    *d.sources
                        .iter_mut()
                        .find(|p| p.id == s.id)
                        .ok_or(Error::Missing)? = s.clone();
                }
                s.id
            }
            Edit::ImportSources(sources) => {
                if sources.len() > 1024 {
                    return Err(Error::Limit);
                }
                let mut affected = 0;
                for mut s in sources {
                    s.validate()?;
                    // Incoming IDs are scoped to the external list, never to this document.
                    let duplicate = d.sources.iter().any(|existing| existing.same_content(&s));
                    if !duplicate {
                        s.id = allocate()?;
                        affected = s.id;
                        d.sources.push(s);
                        if d.sources.len() > 1024 {
                            return Err(Error::Limit);
                        }
                    }
                }
                affected
            }
            Edit::MergeSources(plan) => source_merge::apply(&mut d, &plan)?,
            Edit::RemoveSource(id) => {
                if !d.source_uses(id).is_empty() {
                    return Err(Error::InUse);
                }
                let at = d
                    .sources
                    .iter()
                    .position(|s| s.id == id)
                    .ok_or(Error::Missing)?;
                d.sources.remove(at);
                id
            }
            Edit::Cite {
                paragraph,
                source,
                locator,
            } => {
                let p = d.paragraph_mut(paragraph).ok_or(Error::Missing)?;
                p.content.push(Inline::Citation { source, locator });
                p.content.push(Inline::Text(String::new()));
                paragraph
            }
            Edit::InsertCitation {
                paragraph,
                inline,
                offset,
                items,
            } => {
                let field = Inline::citation(items)?;
                let p = d.paragraph_mut(paragraph).ok_or(Error::Missing)?;
                let node = p.content.get(inline).ok_or(Error::Missing)?;
                let text = node.text().ok_or(Error::Invalid)?;
                let prefix = text.get(..offset).ok_or(Error::Invalid)?;
                let suffix = text.get(offset..).ok_or(Error::Invalid)?;
                let mut parts = Vec::new();
                if !prefix.is_empty() {
                    parts.push(Inline::with_format(String::from(prefix), node.format()));
                }
                parts.push(field);
                parts.push(Inline::with_format(String::from(suffix), node.format()));
                p.content.splice(inline..=inline, parts);
                paragraph
            }
            Edit::UpdateCitation {
                paragraph,
                inline,
                items,
            } => {
                let field = Inline::citation(items)?;
                let node = d
                    .paragraph_mut(paragraph)
                    .ok_or(Error::Missing)?
                    .content
                    .get_mut(inline)
                    .ok_or(Error::Missing)?;
                if !matches!(node, Inline::Citation { .. } | Inline::CitationCluster(_)) {
                    return Err(Error::Invalid);
                }
                *node = field;
                paragraph
            }
            Edit::ConvertCitationToText { paragraph, inline } => {
                let node = d
                    .paragraph(paragraph)
                    .ok_or(Error::Missing)?
                    .content
                    .get(inline)
                    .ok_or(Error::Missing)?;
                let text = match node {
                    Inline::Citation { source, locator } => d.citation_text(*source, locator),
                    Inline::CitationCluster(items) => d.citation_cluster_text(items),
                    _ => return Err(Error::Invalid),
                };
                d.paragraph_mut(paragraph).ok_or(Error::Missing)?.content[inline] =
                    Inline::Text(text);
                paragraph
            }
            Edit::RemoveCitation { paragraph, inline } => {
                let p = d.paragraph_mut(paragraph).ok_or(Error::Missing)?;
                if !matches!(
                    p.content.get(inline),
                    Some(Inline::Citation { .. } | Inline::CitationCluster(_))
                ) {
                    return Err(Error::Missing);
                }
                p.content.remove(inline);
                if p.content.is_empty() {
                    p.content.push(Inline::Text(String::new()));
                }
                paragraph
            }
        };
        d.validate()?;
        check(&d)?;
        if d != self.doc {
            self.undo.push(core::mem::replace(&mut self.doc, d));
            self.undo_states.push(self.state);
            self.redo.clear();
            self.redo_states.clear();
            self.next = next;
            self.revision = self.revision.wrapping_add(1);
            self.state = self.revision;
            while self.undo.len() > 64
                || self.undo.iter().map(Document::byte_size).sum::<usize>() > 8 << 20
            {
                self.undo.remove(0);
                self.undo_states.remove(0);
            }
        }
        Ok(id)
    }
}
