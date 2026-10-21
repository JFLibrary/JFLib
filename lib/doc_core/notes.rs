// Footnotes and endnotes with stable identities, anchored once in a body or table-cell story.
// New shared model functionality, independent of pagination and external formats.
use crate::{
    CitationLocation, Document, Error, Id, Inline, Paragraph, ParagraphFormat, ParagraphStyle,
};
use alloc::{collections::BTreeMap, format, string::String, vec, vec::Vec};

/// Separate automatically numbered note sequences, in body/table reading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    Footnote,
    Endnote,
}

/// A note body. Paragraphs support text and citations, but no nested notes or bibliography.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub id: Id,
    pub kind: NoteKind,
    pub paragraphs: Vec<Paragraph>,
}

impl Document {
    /// Lookup by stable note identity.
    pub fn note(&self, id: Id) -> Option<&Note> {
        self.notes.iter().find(|note| note.id == id)
    }

    /// Body paragraphs, note bodies and cell paragraphs in storage order.
    pub fn all_paragraphs(&self) -> impl Iterator<Item = &Paragraph> {
        self.paragraphs
            .iter()
            .chain(self.notes.iter().flat_map(|note| &note.paragraphs))
            .chain(self.tables.iter().flat_map(|t| t.cells().flat_map(|c| &c.paragraphs)))
    }

    /// Lookup a body, cell or note paragraph by its globally unique identity.
    pub fn paragraph(&self, id: Id) -> Option<&Paragraph> {
        self.all_paragraphs().find(|paragraph| paragraph.id == id)
    }

    pub(crate) fn all_paragraphs_mut(&mut self) -> impl Iterator<Item = &mut Paragraph> {
        self.paragraphs
            .iter_mut()
            .chain(self.notes.iter_mut().flat_map(|note| &mut note.paragraphs))
            .chain(self.tables.iter_mut().flat_map(|t| t.cells_mut().flat_map(|c| &mut c.paragraphs)))
    }

    pub(crate) fn paragraph_mut(&mut self, id: Id) -> Option<&mut Paragraph> {
        self.all_paragraphs_mut()
            .find(|paragraph| paragraph.id == id)
    }

    /// Note owning a paragraph, or None for a main-body or missing paragraph.
    pub fn note_for_paragraph(&self, id: Id) -> Option<&Note> {
        self.notes
            .iter()
            .find(|note| note.paragraphs.iter().any(|paragraph| paragraph.id == id))
    }

    /// Body or cell anchor position for a note.
    pub fn note_location(&self, id: Id) -> Option<CitationLocation> {
        for paragraph in self.body_paragraphs() {
            for (inline, node) in paragraph.content.iter().enumerate() {
                if matches!(node, Inline::NoteReference(note) if *note == id) {
                    return Some(CitationLocation {
                        paragraph: paragraph.id,
                        inline,
                    });
                }
            }
        }
        None
    }

    /// Live decimal number within this note's kind; identities and storage order do not affect it.
    pub fn note_number(&self, id: Id) -> Option<usize> {
        let kind = self.note(id)?.kind;
        self.ordered_notes(kind)
            .iter()
            .position(|note| note.id == id)
            .map(|index| index + 1)
    }

    /// Notes of one kind in body/table reading order.
    pub fn ordered_notes(&self, kind: NoteKind) -> Vec<&Note> {
        let notes: BTreeMap<_, _> = self.notes.iter().map(|note| (note.id, note)).collect();
        self.body_paragraphs()
            .flat_map(|paragraph| &paragraph.content)
            .filter_map(|node| {
                if let Inline::NoteReference(id) = node {
                    notes.get(id).copied().filter(|note| note.kind == kind)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Plain-text marker for a note. Endnotes use an E prefix to distinguish the two sequences.
    pub fn note_marker(&self, id: Id) -> String {
        match (self.note(id), self.note_number(id)) {
            (Some(note), Some(number)) => match note.kind {
                NoteKind::Footnote => format!("[{number}]"),
                NoteKind::Endnote => format!("[E{number}]"),
            },
            _ => String::from("[Anmerkung fehlt]"),
        }
    }

    // Visit each valid note exactly where its main-body anchor occurs, without recursion.
    pub(crate) fn visit_inlines(&self, mut visit: impl FnMut(Id, usize, &Inline)) {
        for paragraph in self.body_paragraphs() {
            for (inline, node) in paragraph.content.iter().enumerate() {
                visit(paragraph.id, inline, node);
                if let Inline::NoteReference(id) = node {
                    if let Some(note) = self.note(*id) {
                        for paragraph in &note.paragraphs {
                            for (inline, node) in paragraph.content.iter().enumerate() {
                                visit(paragraph.id, inline, node);
                            }
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn paragraph_group_mut(&mut self, id: Id) -> Option<&mut Vec<Paragraph>> {
        if self.paragraphs.iter().any(|paragraph| paragraph.id == id) {
            return Some(&mut self.paragraphs);
        }
        if let Some(cell) = self.tables.iter_mut().flat_map(|t| t.cells_mut()).find(|c| c.paragraphs.iter().any(|p| p.id == id)) { return Some(&mut cell.paragraphs); }
        self.notes
            .iter_mut()
            .find(|note| note.paragraphs.iter().any(|paragraph| paragraph.id == id))
            .map(|note| &mut note.paragraphs)
    }

    pub(crate) fn remove_paragraph(&mut self, id: Id) -> Result<(), Error> {
        if self.tables.iter().any(|t| t.before == id) { return Err(Error::InUse); }
        let group = self.paragraph_group_mut(id).ok_or(Error::Missing)?;
        if group.len() <= 1 {
            return Err(Error::Invalid);
        }
        let at = group
            .iter()
            .position(|paragraph| paragraph.id == id)
            .ok_or(Error::Missing)?;
        let removed = group.remove(at);
        for node in removed.content {
            if let Inline::NoteReference(note) = node {
                self.notes.retain(|existing| existing.id != note);
            }
        }
        Ok(())
    }

    pub(crate) fn insert_note(
        &mut self,
        location: CitationLocation,
        offset: usize,
        note: Id,
        body: Id,
        kind: NoteKind,
        text: String,
    ) -> Result<(), Error> {
        // Body and table-cell stories can anchor notes; note bodies cannot nest them.
        if self.note_for_paragraph(location.paragraph).is_some() { return Err(Error::Invalid); }
        let paragraph = self
            .paragraph_mut(location.paragraph)
            .ok_or(Error::Invalid)?;
        let node = paragraph
            .content
            .get(location.inline)
            .ok_or(Error::Missing)?;
        let original = node.text().ok_or(Error::Invalid)?;
        let prefix = original.get(..offset).ok_or(Error::Invalid)?;
        let suffix = original.get(offset..).ok_or(Error::Invalid)?;
        let mut parts = Vec::new();
        if !prefix.is_empty() {
            parts.push(Inline::with_format(String::from(prefix), node.format()));
        }
        parts.push(Inline::NoteReference(note));
        parts.push(Inline::with_format(String::from(suffix), node.format()));
        paragraph
            .content
            .splice(location.inline..=location.inline, parts);
        self.notes.push(Note {
            id: note,
            kind,
            paragraphs: vec![Paragraph {
                id: body,
                style: ParagraphStyle::Normal,
                format: ParagraphFormat::default(),
                content: vec![Inline::Text(text)],
            }],
        });
        Ok(())
    }

    pub(crate) fn remove_note(&mut self, id: Id) -> Result<(), Error> {
        let location = self.note_location(id).ok_or(Error::Missing)?;
        let paragraph = self
            .paragraph_mut(location.paragraph)
            .ok_or(Error::Missing)?;
        paragraph.content.remove(location.inline);
        if paragraph.content.is_empty() {
            paragraph.content.push(Inline::Text(String::new()));
        }
        self.notes.retain(|note| note.id != id);
        Ok(())
    }

    /// Collect note bodies after an edit removed their anchors, including nested tables.
    pub(crate) fn prune_notes(&mut self) {
        let anchors: alloc::collections::BTreeSet<_> = self.body_paragraphs()
            .flat_map(|p| &p.content).filter_map(|n| match n {
                Inline::NoteReference(id) => Some(*id), _ => None,
            }).collect();
        self.notes.retain(|n| anchors.contains(&n.id));
    }
}
