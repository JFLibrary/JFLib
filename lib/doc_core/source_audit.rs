// Advisory bibliography completeness checks; new native functionality without a reference.
use crate::{Document, Id, SourceKind};
use alloc::vec::Vec;

/// Missing bibliographic metadata, independent of any particular citation style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceIssueKind {
    /// Neither personal authors nor a corporate author were supplied.
    MissingCreators,
    /// The publication year is blank.
    MissingYear,
    /// A book has no publisher.
    MissingPublisher,
    /// A journal article has no journal name.
    MissingJournal,
    /// A website has no URL.
    MissingUrl,
    /// Access-date components skip a preceding year or month component.
    IncompleteAccessDate,
}

/// A completeness hint associated with a stable source identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceIssue {
    /// Source containing the incomplete metadata.
    pub source: Id,
    /// Missing field or incomplete component relationship.
    pub kind: SourceIssueKind,
}

impl Document {
    /// Inspect all sources, including uncited ones, without changing document or history.
    /// Hints never block source saving, mark the document dirty or claim style compliance.
    /// Results follow source order, then creators, year, type-specific field and access date.
    /// Whitespace-only fields are blank. Access dates are checked only for missing preceding
    /// components, not calendar validity; year-only and year/month dates are accepted.
    /// A valid document has at most 1,024 sources and fewer than 6,144 returned hints.
    pub fn audit_sources(&self) -> Vec<SourceIssue> {
        let mut issues = Vec::new();
        for source in &self.sources {
            let mut issue = |kind| {
                issues.push(SourceIssue {
                    source: source.id,
                    kind,
                });
            };
            if source.authors.is_empty() && source.corporate.trim().is_empty() {
                issue(SourceIssueKind::MissingCreators);
            }
            if source.year.trim().is_empty() {
                issue(SourceIssueKind::MissingYear);
            }
            match source.kind {
                SourceKind::Book if source.publisher.trim().is_empty() => {
                    issue(SourceIssueKind::MissingPublisher);
                }
                SourceKind::JournalArticle if source.journal.trim().is_empty() => {
                    issue(SourceIssueKind::MissingJournal);
                }
                SourceKind::Website if source.url.trim().is_empty() => {
                    issue(SourceIssueKind::MissingUrl);
                }
                _ => (),
            }
            let year = !source.year_accessed.trim().is_empty();
            let month = !source.month_accessed.trim().is_empty();
            let day = !source.day_accessed.trim().is_empty();
            if (month && !year) || (day && (!year || !month)) {
                issue(SourceIssueKind::IncompleteAccessDate);
            }
        }
        issues
    }
}
