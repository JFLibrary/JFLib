// Explicit source reconciliation with complete citation remapping; new native functionality.
use crate::{Document, Error, Id, Inline, Source};
use alloc::{format, string::String, vec::Vec};

/// Selectable source metadata; personal and corporate creators form one exclusive field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceField {
    Kind,
    Creators,
    Title,
    Year,
    Publisher,
    City,
    Edition,
    Journal,
    Volume,
    Issue,
    Pages,
    Website,
    Url,
    StandardNumber,
    YearAccessed,
    MonthAccessed,
    DayAccessed,
    ShortTitle,
    Comments,
    ExternalId,
    Language,
}
impl SourceField {
    /// Every reconcilable field in stable form order; source identity is never selectable.
    pub const ALL: [Self; 21] = [
        Self::Kind,
        Self::Creators,
        Self::Title,
        Self::Year,
        Self::Publisher,
        Self::City,
        Self::Edition,
        Self::Journal,
        Self::Volume,
        Self::Issue,
        Self::Pages,
        Self::Website,
        Self::Url,
        Self::StandardNumber,
        Self::YearAccessed,
        Self::MonthAccessed,
        Self::DayAccessed,
        Self::ShortTitle,
        Self::Comments,
        Self::ExternalId,
        Self::Language,
    ];
}

/// Explicit field choices: selected fields come from remove, all others remain from keep.
/// An incoming empty field deliberately clears the kept value. Creator choices are atomic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceMerge {
    /// Existing source identity retained by the transaction.
    pub keep: Id,
    /// Existing source removed after its citation references are redirected.
    pub remove: Id,
    /// Fields explicitly copied from remove, including deliberately selected empty values.
    pub incoming: Vec<SourceField>,
}

/// An advisory duplicate candidate. Similarity never authorizes an automatic merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceCandidate {
    /// Candidate's stable identity in the same document.
    pub source: Id,
    /// True when every modeled field matches, ignoring only the local source ID.
    pub identical: bool,
}

impl Source {
    fn scalar(&self, field: SourceField) -> Option<&String> {
        Some(match field {
            SourceField::Kind | SourceField::Creators => return None,
            SourceField::Title => &self.title,
            SourceField::Year => &self.year,
            SourceField::Publisher => &self.publisher,
            SourceField::City => &self.city,
            SourceField::Edition => &self.edition,
            SourceField::Journal => &self.journal,
            SourceField::Volume => &self.volume,
            SourceField::Issue => &self.issue,
            SourceField::Pages => &self.pages,
            SourceField::Website => &self.website,
            SourceField::Url => &self.url,
            SourceField::StandardNumber => &self.standard_number,
            SourceField::YearAccessed => &self.year_accessed,
            SourceField::MonthAccessed => &self.month_accessed,
            SourceField::DayAccessed => &self.day_accessed,
            SourceField::ShortTitle => &self.short_title,
            SourceField::Comments => &self.comments,
            SourceField::ExternalId => &self.external_id,
            SourceField::Language => &self.language,
        })
    }
    fn scalar_mut(&mut self, field: SourceField) -> Option<&mut String> {
        Some(match field {
            SourceField::Kind | SourceField::Creators => return None,
            SourceField::Title => &mut self.title,
            SourceField::Year => &mut self.year,
            SourceField::Publisher => &mut self.publisher,
            SourceField::City => &mut self.city,
            SourceField::Edition => &mut self.edition,
            SourceField::Journal => &mut self.journal,
            SourceField::Volume => &mut self.volume,
            SourceField::Issue => &mut self.issue,
            SourceField::Pages => &mut self.pages,
            SourceField::Website => &mut self.website,
            SourceField::Url => &mut self.url,
            SourceField::StandardNumber => &mut self.standard_number,
            SourceField::YearAccessed => &mut self.year_accessed,
            SourceField::MonthAccessed => &mut self.month_accessed,
            SourceField::DayAccessed => &mut self.day_accessed,
            SourceField::ShortTitle => &mut self.short_title,
            SourceField::Comments => &mut self.comments,
            SourceField::ExternalId => &mut self.external_id,
            SourceField::Language => &mut self.language,
        })
    }
    /// Readable comparison value; exact equality additionally preserves creator structure.
    pub fn field_text(&self, field: SourceField) -> String {
        match field {
            SourceField::Kind => format!("{:?}", self.kind),
            SourceField::Creators => self.author_text(),
            _ => self.scalar(field).cloned().unwrap_or_default(),
        }
    }
    /// Exact metadata comparison, including creator mode, components and entered order.
    pub fn field_equals(&self, other: &Self, field: SourceField) -> bool {
        match field {
            SourceField::Kind => self.kind == other.kind,
            SourceField::Creators => {
                self.authors == other.authors && self.corporate == other.corporate
            }
            _ => self.scalar(field) == other.scalar(field),
        }
    }
    /// True only for absent metadata. Kind is always present; whitespace is retained data.
    pub fn field_empty(&self, field: SourceField) -> bool {
        match field {
            SourceField::Kind => false,
            SourceField::Creators => self.authors.is_empty() && self.corporate.is_empty(),
            _ => self.scalar(field).is_some_and(String::is_empty),
        }
    }
    /// Copy explicitly selected metadata from another scope, retaining this source's ID.
    /// Creator mode and ordered people change together. Duplicate choices and invalid
    /// resulting metadata are rejected; selecting an empty value deliberately clears it.
    pub fn merge_fields(&self, incoming: &Self, fields: &[SourceField]) -> Result<Self, Error> {
        if fields.len() > SourceField::ALL.len() {
            return Err(Error::Invalid);
        }
        let mut source = self.clone();
        for (index, field) in fields.iter().enumerate() {
            if fields[..index].contains(field) {
                return Err(Error::Invalid);
            }
            match field {
                SourceField::Kind => source.kind = incoming.kind,
                SourceField::Creators => {
                    source.authors = incoming.authors.clone();
                    source.corporate = incoming.corporate.clone();
                }
                _ => {
                    if let (Some(target), Some(value)) =
                        (source.scalar_mut(*field), incoming.scalar(*field))
                    {
                        *target = value.clone();
                    }
                }
            }
        }
        source.validate()?;
        Ok(source)
    }
}

fn merged_source(document: &Document, plan: &SourceMerge) -> Result<Source, Error> {
    if plan.keep == plan.remove || plan.incoming.len() > SourceField::ALL.len() {
        return Err(Error::Invalid);
    }
    let source = document.source(plan.keep).ok_or(Error::Missing)?;
    let incoming = document.source(plan.remove).ok_or(Error::Missing)?;
    source.merge_fields(incoming, &plan.incoming)
}

pub(crate) fn apply(document: &mut Document, plan: &SourceMerge) -> Result<Id, Error> {
    let source = merged_source(document, plan)?;
    let kept = document
        .sources
        .iter_mut()
        .find(|source| source.id == plan.keep)
        .ok_or(Error::Missing)?;
    *kept = source;
    document.sources.retain(|source| source.id != plan.remove);
    for paragraph in document.all_paragraphs_mut() {
        for inline in &mut paragraph.content {
            match inline {
                Inline::Citation { source, .. } => {
                    if *source == plan.remove {
                        *source = plan.keep;
                    }
                }
                Inline::CitationCluster(items) => {
                    for item in items {
                        if item.source == plan.remove {
                            item.source = plan.keep;
                        }
                    }
                }
                _ => (),
            }
        }
    }
    Ok(plan.keep)
}

impl Document {
    /// Prepare explicit reconciliation, suggesting incoming data only for absent kept fields.
    /// Both IDs must exist and differ. Populated fields, including Kind, retain their values.
    pub fn plan_source_merge(&self, keep: Id, remove: Id) -> Result<SourceMerge, Error> {
        self.validate()?;
        if keep == remove {
            return Err(Error::Invalid);
        }
        let retained = self.source(keep).ok_or(Error::Missing)?;
        let incoming = self.source(remove).ok_or(Error::Missing)?;
        Ok(SourceMerge {
            keep,
            remove,
            incoming: SourceField::ALL
                .into_iter()
                .filter(|field| retained.field_empty(*field) && !incoming.field_empty(*field))
                .collect(),
        })
    }
    /// Preview without mutations, validating the full remapped document and expansion budget.
    /// Duplicate selected fields, invalid identities or excessive generated text are errors.
    pub fn preview_source_merge(&self, plan: &SourceMerge) -> Result<Source, Error> {
        self.validate()?;
        let mut preview = self.clone();
        apply(&mut preview, plan)?;
        preview.validate()?;
        preview.source(plan.keep).cloned().ok_or(Error::Missing)
    }
    /// List identical or similar sources in document order, excluding the requested source.
    /// The validated document contains at most 1024 sources, so at most 1023 are returned.
    pub fn source_candidates(&self, source: Id) -> Result<Vec<SourceCandidate>, Error> {
        self.validate()?;
        let source = self.source(source).ok_or(Error::Missing)?;
        Ok(self
            .sources
            .iter()
            .filter(|candidate| candidate.id != source.id)
            .filter_map(|candidate| {
                let identical = source.same_content(candidate);
                (identical || source.similar_to(candidate)).then_some(SourceCandidate {
                    source: candidate.id,
                    identical,
                })
            })
            .collect())
    }
}
