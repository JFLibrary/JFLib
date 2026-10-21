// Structured clipboard ranges, independent of platform clipboard ownership and file codecs.
use crate::{
    Document, Edit, Editor, Error, Id, Inline, Paragraph, ParagraphStyle, ParagraphText, TextColor,
};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};
#[path="table_fragment.rs"] mod tables;

/// A self-contained rich selection. Paragraph boundaries distinguish hard paragraph marks
/// from soft line breaks; sources and anchored notes travel with their native fields.
/// The first and last paragraph can be partial. IDs are local to this fragment and are
/// remapped when inserted. Public codec data must pass document.validate() before use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocumentFragment {
    pub document: Document,
}

// Partition one paragraph without ever splitting the generated display of a field.
// Zero-width fields are included when the whole paragraph is selected with a paragraph mark.
fn partition(
    p: &Paragraph,
    projection: &ParagraphText,
    start: usize,
    end: usize,
    complete: bool,
) -> Result<(Vec<Inline>, Vec<Inline>, Vec<Inline>), Error> {
    projection.text.get(start..end).ok_or(Error::Invalid)?;
    let (mut before, mut selected, mut after) = (Vec::new(), Vec::new(), Vec::new());
    for span in &projection.spans {
        let Some(node) = p.content.get(span.inline) else {
            continue;
        };
        if let Some(text) = node.text() {
            let a = start.saturating_sub(span.start).min(text.len());
            let b = end.saturating_sub(span.start).min(text.len());
            if a > 0 {
                before.push(Inline::with_format(String::from(&text[..a]), node.format()));
            }
            if a < b {
                selected.push(Inline::with_format(
                    String::from(&text[a..b]),
                    node.format(),
                ));
            }
            if b < text.len() {
                after.push(Inline::with_format(String::from(&text[b..]), node.format()));
            }
            if text.is_empty() {
                if span.start < start {
                    before.push(node.clone());
                } else if complete || start < span.start && span.start < end {
                    selected.push(node.clone());
                } else {
                    after.push(node.clone());
                }
            }
        } else if span.start == span.end {
            if (complete || start < span.start && span.start < end)
                && start <= span.start
                && span.start <= end
            {
                selected.push(node.clone());
            } else if span.start < start {
                before.push(node.clone());
            } else {
                after.push(node.clone());
            }
        } else if span.start < start && start < span.end || span.start < end && end < span.end {
            return Err(Error::InUse);
        } else if span.end <= start {
            before.push(node.clone());
        } else if span.start >= end {
            after.push(node.clone());
        } else {
            selected.push(node.clone());
        }
    }
    Ok((before, selected, after))
}

fn source_ids(nodes: &[Inline], sources: &mut BTreeSet<Id>) {
    for node in nodes {
        match node {
            Inline::Citation { source, .. } => {
                sources.insert(*source);
            }
            Inline::CitationCluster(items) => sources.extend(items.iter().map(|i| i.source)),
            _ => (),
        }
    }
}

fn remap(
    nodes: &mut [Inline],
    sources: &BTreeMap<Id, Id>,
    notes: &BTreeMap<Id, Id>,
) -> Result<(), Error> {
    for node in nodes {
        match node {
            Inline::Citation { source, .. } => {
                *source = *sources.get(source).ok_or(Error::Missing)?
            }
            Inline::CitationCluster(items) => {
                for item in items {
                    item.source = *sources.get(&item.source).ok_or(Error::Missing)?;
                }
            }
            Inline::NoteReference(id) => *id = *notes.get(id).ok_or(Error::Missing)?,
            _ => (),
        }
    }
    Ok(())
}

// Preserve visible text when two differently styled paragraph fragments are joined.
fn preserve_style(d: &Document, nodes: &mut [Inline], original: ParagraphStyle, destination: ParagraphStyle) {
    if original == destination {
        return;
    }
    for node in nodes {
        if let Some(text) = node.text() {
            let mut format = node.format();
            let old = d.resolve_text(format, original);
            let new = d.resolve_text(format, destination);
            if old.font_family != new.font_family { format.font_family = Some(old.font_family); }
            if old.italic != new.italic { format.italic = Some(old.italic); }
            if old.underline != new.underline { format.underline = Some(old.underline); }
            if old.strike != new.strike { format.strike = Some(old.strike); }
            if old.highlight != new.highlight { format.highlight = Some(old.highlight); }
            if old.positioning != new.positioning { format.positioning = Some(old.positioning); }
            if old.bold != new.bold {
                format.bold = Some(old.bold);
            }
            if old.size_half_points != new.size_half_points {
                format.size_half_points = Some(old.size_half_points);
            }
            if old.color != new.color {
                format.color = Some(TextColor::Rgb(old.color & 0xffffff));
            }
            *node = Inline::with_format(String::from(text), format);
        }
    }
}

// Generated fields are indivisible, but their surrounding seams remain editable even
// when a copied selection consisted solely of fields and carried no empty text run.
fn field_carets(content: &mut Vec<Inline>) {
    let mut out: Vec<Inline> = Vec::new();
    for node in core::mem::take(content) {
        if node.text().is_none() && out.last().is_none_or(|i| i.text().is_none()) {
            out.push(Inline::Text(String::new()));
        }
        out.push(node);
    }
    if out.last().is_none_or(|i| i.text().is_none()) {
        out.push(Inline::Text(String::new()));
    }
    *content = out;
}

impl Document {
    /// Copy a story range with native formatting and complete fields. Selecting only part
    /// of a generated field fails; no display text is silently substituted for its identity.
    pub fn copy_story_range(
        &self,
        paragraph: Id,
        start: usize,
        end: usize,
    ) -> Result<DocumentFragment, Error> {
        self.validate()?;
        let story = self.story_projection(paragraph)?;
        story.text.get(start..end).ok_or(Error::Invalid)?;
        let (first, lo) = story.position(start)?;
        let (last, hi) = story.position(end)?;
        let mut paragraphs = Vec::new();
        let mut active = false;
        for projected in &story.paragraphs {
            let id = projected.projection.paragraph;
            if id == first {
                active = true;
            }
            if !active {
                continue;
            }
            let mut p = self.paragraph(id).ok_or(Error::Missing)?.clone();
            let from = if id == first { lo } else { 0 };
            let to = if id == last {
                hi
            } else {
                projected.projection.text.len()
            };
            let complete = start < end && from == 0 && to == projected.projection.text.len();
            p.content = partition(&p, &projected.projection, from, to, complete)?.1;
            if self.table_location(id).is_some_and(|(t,_,_)| self.table(t).is_some_and(|t|t.style.is_some())) {
                for node in &mut p.content {
                    if let Some(text)=node.text() {
                        let f=self.resolve_paragraph_text(self.paragraph(id).unwrap(),node.format()).explicit();
                        *node=Inline::with_format(String::from(text),f);
                    }
                }
            }
            paragraphs.push(p);
            if id == last {
                break;
            }
        }
        let mut wanted_sources = BTreeSet::new();
        let mut wanted_notes = BTreeSet::new();
        let mut bibliography = false;
        for p in &paragraphs {
            source_ids(&p.content, &mut wanted_sources);
            for node in &p.content {
                match node {
                    Inline::NoteReference(id) => {
                        wanted_notes.insert(*id);
                    }
                    Inline::Bibliography => bibliography = true,
                    _ => (),
                }
            }
        }
        let notes: Vec<_> = self
            .notes
            .iter()
            .filter(|n| wanted_notes.contains(&n.id))
            .cloned()
            .collect();
        for note in &notes {
            for p in &note.paragraphs {
                source_ids(&p.content, &mut wanted_sources);
            }
        }
        if bibliography {
            wanted_sources.extend(self.cited_source_ids());
        }
        let document = Document {
            tables: Vec::new(), table_styles: Vec::new(),
            styles: self.styles.clone(),
            paragraphs,
            sources: self
                .sources
                .iter()
                .filter(|s| wanted_sources.contains(&s.id))
                .cloned()
                .collect(),
            notes,
            page: self.page,
        };
        document.validate()?;
        Ok(DocumentFragment { document })
    }

    pub(crate) fn replace_story_fragment(
        &mut self,
        paragraph: Id,
        start: usize,
        end: usize,
        fragment: DocumentFragment,
        allocate: &mut impl FnMut() -> Result<Id, Error>,
    ) -> Result<Id, Error> {
        fragment.document.validate()?;
        let story = self.story_projection(paragraph)?;
        story.text.get(start..end).ok_or(Error::Invalid)?;
        let (left, lo) = story.position(start)?;
        let (right, hi) = story.position(end)?;
        let first = story
            .paragraphs
            .iter()
            .position(|p| p.projection.paragraph == left)
            .unwrap();
        let last = story
            .paragraphs
            .iter()
            .position(|p| p.projection.paragraph == right)
            .unwrap();
        let left_p = self.paragraph(left).ok_or(Error::Missing)?.clone();
        let right_p = self.paragraph(right).ok_or(Error::Missing)?.clone();
        let mut prefix = Vec::new();
        let mut suffix = Vec::new();
        for (index, projected) in story
            .paragraphs
            .iter()
            .enumerate()
            .take(last + 1)
            .skip(first)
        {
            let p = self
                .paragraph(projected.projection.paragraph)
                .ok_or(Error::Missing)?;
            let from = if index == first { lo } else { 0 };
            let to = if index == last {
                hi
            } else {
                projected.projection.text.len()
            };
            let complete = start < end && from == 0 && to == projected.projection.text.len();
            let (before, _, after) = partition(p, &projected.projection, from, to, complete)?;
            if index == first {
                prefix = before;
            }
            if index == last {
                suffix = after;
            }
        }
        let mut incoming = fragment.document;
        if !incoming.tables.is_empty() {
            return self.insert_table_fragment(left,right,left_p,right_p,prefix,suffix,incoming,allocate);
        }
        if (self.note_for_paragraph(paragraph).is_some() && !incoming.notes.is_empty())
            || ((self.note_for_paragraph(paragraph).is_some() || self.cell_for_paragraph(paragraph).is_some())
                && incoming
                    .paragraphs
                    .iter()
                    .flat_map(|p| &p.content)
                    .any(|i| matches!(i, Inline::Bibliography)))
        {
            return Err(Error::Invalid);
        }
        let empty = incoming.paragraphs.len() == 1
            && incoming.paragraphs[0]
                .content
                .iter()
                .all(|i| i.text() == Some(""));
        if empty && start == end {
            return Ok(left);
        }
        if !empty { self.import_fragment_styles(&mut incoming)?; }
        let mut sources = BTreeMap::new();
        for mut source in incoming.sources {
            let old = source.id;
            let id = if let Some(existing) = self.sources.iter().find(|s| s.same_content(&source)) {
                existing.id
            } else {
                source.id = allocate()?;
                let id = source.id;
                self.sources.push(source);
                id
            };
            sources.insert(old, id);
        }
        let mut notes = BTreeMap::new();
        for note in &incoming.notes {
            notes.insert(note.id, allocate()?);
        }
        for mut note in incoming.notes {
            note.id = *notes.get(&note.id).ok_or(Error::Missing)?;
            for p in &mut note.paragraphs {
                p.id = allocate()?;
                remap(&mut p.content, &sources, &notes)?;
                field_carets(&mut p.content);
            }
            self.notes.push(note);
        }
        for (index, p) in incoming.paragraphs.iter_mut().enumerate() {
            p.id = if index == 0 { left } else { allocate()? };
            remap(&mut p.content, &sources, &notes)?;
        }
        let first_p = &mut incoming.paragraphs[0];
        if empty {
            // A cut must not leave an unformatted zero-width run between two formatted
            // survivors. Keep the selected run's insertion style only if nothing survives.
            first_p.content.clear();
            if prefix.is_empty() && suffix.is_empty() {
                let format = story.paragraphs[first]
                    .projection
                    .position(lo)
                    .ok()
                    .and_then(|at| left_p.content.get(at.inline))
                    .map_or_else(crate::TextFormat::default, Inline::format);
                first_p
                    .content
                    .push(Inline::with_format(String::new(), format));
            }
        }
        if lo > 0 || empty {
            if !empty {
                preserve_style(self, &mut first_p.content, first_p.style, left_p.style);
            }
            first_p.style = left_p.style;
            first_p.format = left_p.format;
        }
        prefix.append(&mut first_p.content);
        first_p.content = prefix;
        let last_p = incoming.paragraphs.last_mut().ok_or(Error::Invalid)?;
        preserve_style(self, &mut suffix, right_p.style, last_p.style);
        last_p.content.extend(suffix);
        let affected = last_p.id;
        for p in &mut incoming.paragraphs {
            crate::text_range::coalesce(&mut p.content);
            field_carets(&mut p.content);
        }
        let group = self.paragraph_group_mut(left).ok_or(Error::Missing)?;
        let base = group.iter().position(|p| p.id == left).ok_or(Error::Missing)?;
        group.splice(base..=base + last - first, incoming.paragraphs);
        self.prune_notes();
        Ok(affected)
    }
}

impl Editor {
    /// Remove a native field and return its surviving insertion caret. Citation and note
    /// removal can change bibliography or marker lengths before and after that caret;
    /// the returned paragraph-local UTF-8 offset is measured in the updated document.
    pub fn remove_field(&mut self, paragraph: Id, inline: usize) -> Result<(Id, usize), Error> {
        let p = self.document().paragraph(paragraph).ok_or(Error::Missing)?;
        p.content.get(inline).ok_or(Error::Missing)?;
        let mut suffix = p.clone();
        suffix.content = p.content[inline + 1..].to_vec();
        self.apply(Edit::RemoveField { paragraph, inline })?;
        let suffix_len = self.document().paragraph_text(&suffix).len();
        let total = self.document().text_projection(paragraph)?.text.len();
        Ok((paragraph, total.saturating_sub(suffix_len)))
    }

    /// Insert a rich fragment as one undo transaction and return its trailing caret as
    /// (paragraph identity, paragraph-local UTF-8 offset). Live field lengths are measured
    /// after insertion, so source reuse and note renumbering cannot misplace the caret.
    pub fn replace_story_fragment(
        &mut self,
        paragraph: Id,
        start: usize,
        end: usize,
        fragment: DocumentFragment,
    ) -> Result<(Id, usize), Error> {
        self.replace_story_fragment_checked(paragraph,start,end,fragment,|_|Ok(()))
    }
    /// Insert a fragment with an additional pre-commit constraint, preserving its exact caret.
    /// Rejection leaves the document, identity allocation and undo/redo history unchanged.
    pub fn replace_story_fragment_checked(
        &mut self, paragraph:Id, start:usize, end:usize, fragment:DocumentFragment,
        check:impl FnOnce(&Document)->Result<(),Error>,
    )->Result<(Id,usize),Error>{
        let story = self.document().story_projection(paragraph)?;
        story.text.get(start..end).ok_or(Error::Invalid)?;
        let (left, lo) = story.position(start)?;
        let (right, hi) = story.position(end)?;
        let projection = self.document().text_projection(right)?;
        let p = self.document().paragraph(right).ok_or(Error::Missing)?;
        let mut suffix = p.clone();
        let from = if left == right { lo } else { 0 };
        let complete = start < end && from == 0 && hi == projection.text.len();
        suffix.content = partition(p, &projection, from, hi, complete)?.2;
        let affected = self.apply_checked(Edit::ReplaceStoryFragment {
            paragraph,
            start,
            end,
            fragment,
        },check)?;
        let suffix_len = self.document().paragraph_text(&suffix).len();
        let total = self.document().text_projection(affected)?.text.len();
        Ok((affected, total.saturating_sub(suffix_len)))
    }
}
