// Source-list review and filtering, shared by bibliography UIs and headless consumers.
use crate::{Document, Error, Id, Source};
use alloc::{string::String, vec::Vec};

/// Reference to an existing document source or an earlier zero-based import row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceTarget {
    Document(Id),
    Import(usize),
}
/// Advisory import classification. Similar records are never automatically merged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceMatch {
    New,
    Identical(SourceTarget),
    Similar(SourceTarget),
}
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
impl Source {
    /// Compare all source metadata, ignoring the container-local source ID.
    pub fn same_content(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.authors == other.authors
            && self.text_fields() == other.text_fields()
    }
    /// Search title, people and metadata with case-insensitive, normalized whitespace.
    pub fn matches_query(&self, query: &str) -> bool {
        let query = normalized(query);
        query.is_empty()
            || self
                .text_fields()
                .iter()
                .any(|s| normalized(s).contains(&query))
            || self.authors.iter().any(|p| {
                normalized(&p.display()).contains(&query)
                    || normalized(&alloc::format!("{} {} {}", p.given, p.middle, p.family))
                        .contains(&query)
            })
    }
    pub(crate) fn similar_to(&self, other: &Self) -> bool {
        (!self.url.is_empty() && self.url.trim() == other.url.trim())
            || (self.kind == other.kind
                && normalized(&self.title) == normalized(&other.title)
                && normalized(&self.year) == normalized(&other.year))
    }
}
impl Document {
    /// Validate and classify an import without changing the document. Identical rows are safe
    /// to skip; a similar title/year or exact URL is only a hint requiring human review.
    /// Existing records take precedence over references to earlier rows in the incoming list.
    pub fn review_sources(&self, sources: &[Source]) -> Result<Vec<SourceMatch>, Error> {
        if sources.len() > 1024 || sources.iter().map(Source::byte_size).sum::<usize>() > 2 << 20 {
            return Err(Error::Limit);
        }
        let mut out = Vec::with_capacity(sources.len());
        for (index, source) in sources.iter().enumerate() {
            source.validate()?;
            let candidates = || {
                self.sources
                    .iter()
                    .map(|s| (s, SourceTarget::Document(s.id)))
                    .chain(
                        sources[..index]
                            .iter()
                            .enumerate()
                            .map(|(i, s)| (s, SourceTarget::Import(i))),
                    )
            };
            let classification =
                if let Some((_, target)) = candidates().find(|(s, _)| source.same_content(s)) {
                    SourceMatch::Identical(target)
                } else if let Some((_, target)) = candidates().find(|(s, _)| source.similar_to(s)) {
                    SourceMatch::Similar(target)
                } else {
                    SourceMatch::New
                };
            out.push(classification);
        }
        Ok(out)
    }
}
