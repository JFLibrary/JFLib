// Native scientific source records and the deliberately simple author/year formatter.
use crate::{Error, Id, check_text};
use alloc::{format, string::String, vec::Vec};

/// Source categories supported by the current editor and bibliography codecs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceKind {
    #[default]
    Book,
    JournalArticle,
    Website,
}
/// A person's name, kept in separate components without guessing name boundaries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Person {
    pub given: String,
    pub middle: String,
    pub family: String,
}
impl Person {
    /// Family name, or given/middle names when no family name was supplied.
    pub fn short_name(&self) -> String {
        if self.family.is_empty() {
            self.given_names()
        } else {
            self.family.clone()
        }
    }
    fn given_names(&self) -> String {
        [self.given.as_str(), self.middle.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
    /// Display name in family, given order, preserving Unicode and entered spelling.
    pub fn display(&self) -> String {
        [self.family.clone(), self.given_names()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
/// A source independent of its file format. Authors are ordered people OR one corporation.
/// id=0 is accepted by source validation and allocated by an editing transaction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    pub id: Id,
    pub kind: SourceKind,
    pub authors: Vec<Person>,
    pub corporate: String,
    pub title: String,
    pub year: String,
    pub publisher: String,
    pub city: String,
    pub edition: String,
    pub journal: String,
    pub volume: String,
    pub issue: String,
    pub pages: String,
    pub website: String,
    pub url: String,
    pub standard_number: String,
    pub year_accessed: String,
    pub month_accessed: String,
    pub day_accessed: String,
    pub short_title: String,
    pub comments: String,
    pub external_id: String,
    pub language: String,
}
impl Source {
    /// Bibliographic scalar fields, for consistent validation and resource accounting.
    pub fn text_fields(&self) -> [&str; 20] {
        [
            &self.corporate,
            &self.title,
            &self.year,
            &self.publisher,
            &self.city,
            &self.edition,
            &self.journal,
            &self.volume,
            &self.issue,
            &self.pages,
            &self.website,
            &self.url,
            &self.standard_number,
            &self.year_accessed,
            &self.month_accessed,
            &self.day_accessed,
            &self.short_title,
            &self.comments,
            &self.external_id,
            &self.language,
        ]
    }
    /// Validate required title, exclusive author modes, field sizes and XML-compatible text.
    pub fn validate(&self) -> Result<(), Error> {
        if self.title.trim().is_empty() || (!self.authors.is_empty() && !self.corporate.is_empty())
        {
            return Err(Error::Invalid);
        }
        if self.authors.len() > 64 {
            return Err(Error::Limit);
        }
        for t in self.text_fields().into_iter().chain(
            self.authors
                .iter()
                .flat_map(|p| [p.given.as_str(), p.middle.as_str(), p.family.as_str()]),
        ) {
            check_text(t)?;
            if t.len() > 4096 {
                return Err(Error::Limit);
            }
        }
        if self.authors.iter().any(|p| p.display().trim().is_empty()) {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    /// Approximate retained source size, including author/string storage.
    pub fn byte_size(&self) -> usize {
        core::mem::size_of::<Self>()
            + self.text_fields().iter().map(|s| s.len()).sum::<usize>()
            + self
                .authors
                .iter()
                .map(|p| {
                    core::mem::size_of::<Person>() + p.given.len() + p.middle.len() + p.family.len()
                })
                .sum::<usize>()
    }
    /// All authors in entered order; corporate spelling is kept verbatim.
    pub fn author_text(&self) -> String {
        if !self.corporate.is_empty() {
            self.corporate.clone()
        } else {
            self.authors
                .iter()
                .map(Person::display)
                .collect::<Vec<_>>()
                .join("; ")
        }
    }
    /// Abbreviated authors for the basic citation style (not a CSL implementation).
    pub fn citation_author(&self) -> String {
        if !self.corporate.is_empty() {
            return self.corporate.clone();
        }
        match self.authors.as_slice() {
            [] => self.title.clone(),
            [one] => one.short_name(),
            [one, two] => format!("{} & {}", one.short_name(), two.short_name()),
            [one, ..] => format!("{} et al.", one.short_name()),
        }
    }
    /// Full entry in the simple author/year style, including available publication details.
    pub fn bibliography_text(&self) -> String {
        let author = self.author_text();
        let mut row = if author.is_empty() {
            String::new()
        } else {
            format!("{author} ")
        };
        row.push_str(&format!(
            "({}). {}.",
            if self.year.is_empty() {
                "o. J."
            } else {
                &self.year
            },
            self.title
        ));
        let mut details = Vec::new();
        if !self.edition.is_empty() {
            details.push(format!("{}. Aufl.", self.edition));
        }
        for s in [&self.journal, &self.website] {
            if !s.is_empty() {
                details.push(s.clone());
            }
        }
        if !self.volume.is_empty() || !self.issue.is_empty() {
            details.push(format!(
                "{}{}",
                self.volume,
                if self.issue.is_empty() {
                    String::new()
                } else {
                    format!("({})", self.issue)
                }
            ));
        }
        if !self.pages.is_empty() {
            details.push(format!("S. {}", self.pages));
        }
        let publisher = [&self.city, &self.publisher]
            .into_iter()
            .filter(|s| !s.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(": ");
        if !publisher.is_empty() {
            details.push(publisher);
        }
        if !details.is_empty() {
            row.push_str(&format!(" {}.", details.join(", ")));
        }
        if !self.standard_number.is_empty() {
            row.push_str(&format!(" {}.", self.standard_number));
        }
        if !self.url.is_empty() {
            row.push_str(&format!(" {}", self.url));
        }
        let accessed = [
            &self.year_accessed,
            &self.month_accessed,
            &self.day_accessed,
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("-");
        if !accessed.is_empty() {
            row.push_str(&format!(" (Zugriff: {accessed})"));
        }
        row
    }
}
