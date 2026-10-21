// Structured multi-source citation fields, rendering and source-usage lookup.
// New shared document functionality, independent of external citation processors.
use crate::{check_text, Document, Error, Id, Inline};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::String,
    vec,
    vec::Vec,
};

/// One source reference in an ordered citation field. Affixes are kept verbatim.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CitationItem {
    /// Stable document-local source ID; zero is only an unbound draft value.
    pub source: Id,
    /// Page or other location, currently rendered with the basic style's page prefix.
    pub locator: String,
    /// Literal text before this reference, including any desired whitespace.
    pub prefix: String,
    /// Literal text after this reference, including any desired whitespace.
    pub suffix: String,
    /// Hide personal or corporate authors in this reference.
    pub suppress_author: bool,
    /// Hide the publication year in this reference.
    pub suppress_year: bool,
    /// Hide the title fallback when this source has no authors.
    pub suppress_title: bool,
}
impl CitationItem {
    /// Validate field-code-safe text and the source ID range; references require a document.
    pub fn validate(&self) -> Result<(), Error> {
        if self.source == 0 || self.source >= 0x7fff_ffff {
            return Err(Error::Invalid);
        }
        for text in [&self.locator, &self.prefix, &self.suffix] {
            check_text(text)?;
            if text.len() > 128 || text.contains(['"', '\\', '\n', '\r']) {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
    fn simple(&self) -> bool {
        self.prefix.is_empty()
            && self.suffix.is_empty()
            && !self.suppress_author
            && !self.suppress_year
            && !self.suppress_title
    }
    pub(crate) fn byte_size(&self) -> usize {
        core::mem::size_of::<Self>() + self.locator.len() + self.prefix.len() + self.suffix.len()
    }
}

/// Location of one citation field, even when that field cites a source more than once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CitationLocation {
    /// Stable paragraph ID containing the field.
    pub paragraph: Id,
    /// Zero-based inline index of the field within its paragraph.
    pub inline: usize,
}

pub(crate) fn validate_items(items: &[CitationItem]) -> Result<(), Error> {
    if items.is_empty() {
        return Err(Error::Invalid);
    }
    if items.len() > 32 {
        return Err(Error::Limit);
    }
    for item in items {
        item.validate()?;
    }
    Ok(())
}

impl Inline {
    /// Create a bounded citation field, keeping simple single references in the legacy form.
    /// Source existence is checked when validating or applying the containing document.
    pub fn citation(items: Vec<CitationItem>) -> Result<Self, Error> {
        validate_items(&items)?;
        if items.len() == 1 && items[0].simple() {
            let mut items = items;
            let item = items.remove(0);
            Ok(Self::Citation {
                source: item.source,
                locator: item.locator,
            })
        } else {
            Ok(Self::CitationCluster(items))
        }
    }
    /// Owned edit draft for either citation representation; other inline nodes return None.
    pub fn citation_items(&self) -> Option<Vec<CitationItem>> {
        match self {
            Self::Citation { source, locator } => Some(vec![CitationItem {
                source: *source,
                locator: locator.clone(),
                ..Default::default()
            }]),
            Self::CitationCluster(items) => Some(items.clone()),
            _ => None,
        }
    }
}

impl Document {
    // Retained model size alone does not bound repeated source expansions. Count the exact
    // plain-text byte size without materializing a whole paragraph/document. Temporary
    // strings are restricted to one source's author and bibliography labels at a time.
    pub(crate) fn validate_expanded_text(&self) -> Result<(), Error> {
        let mut cited = BTreeMap::new();
        let mut has_bibliography = false;
        for node in self.all_paragraphs().flat_map(|p| &p.content) {
            match node {
                Inline::Citation { source, .. } => {
                    cited.insert(*source, ExpandedSource::default());
                }
                Inline::CitationCluster(items) => {
                    for item in items {
                        cited.insert(item.source, ExpandedSource::default());
                    }
                }
                Inline::Bibliography => has_bibliography = true,
                _ => (),
            }
        }
        let mut bibliography = cited.len().saturating_sub(1);
        for (id, lengths) in &mut cited {
            let source = self.source(*id).ok_or(Error::Missing)?;
            lengths.author = source.citation_author().len();
            lengths.year = source.year.len();
            lengths.anonymous = source.authors.is_empty() && source.corporate.is_empty();
            if has_bibliography {
                add_expanded(&mut bibliography, source.bibliography_text().len())?;
            }
        }
        let mut total = self.all_paragraphs().count().saturating_sub(1);
        for note in &self.notes {
            add_expanded(&mut total, self.note_marker(note.id).len() + 1)?;
        }
        for node in self.all_paragraphs().flat_map(|p| &p.content) {
            match node {
                Inline::Text(text) | Inline::StyledText { text, .. } => {
                    add_expanded(&mut total, text.len())?;
                }
                Inline::Citation { source, locator } => {
                    let lengths = cited.get(source).ok_or(Error::Missing)?;
                    add_expanded(&mut total, 2 + lengths.author)?;
                    if lengths.year != 0 {
                        add_expanded(&mut total, 2 + lengths.year)?;
                    }
                    if !locator.is_empty() {
                        add_expanded(&mut total, 5 + locator.len())?;
                    }
                }
                Inline::CitationCluster(items) => {
                    add_expanded(&mut total, 2 + 2 * items.len().saturating_sub(1))?;
                    for item in items {
                        let lengths = cited.get(&item.source).ok_or(Error::Missing)?;
                        add_expanded(&mut total, item.prefix.len() + item.suffix.len())?;
                        let author = if lengths.anonymous {
                            !item.suppress_title
                        } else {
                            !item.suppress_author
                        };
                        let year = !item.suppress_year && lengths.year != 0;
                        let locator = !item.locator.is_empty();
                        let parts = usize::from(author) + usize::from(year) + usize::from(locator);
                        add_expanded(&mut total, 2 * parts.saturating_sub(1))?;
                        if author {
                            add_expanded(&mut total, lengths.author)?;
                        }
                        if year {
                            add_expanded(&mut total, lengths.year)?;
                        }
                        if locator {
                            add_expanded(&mut total, 3 + item.locator.len())?;
                        }
                    }
                }
                Inline::Bibliography => add_expanded(&mut total, bibliography)?,
                Inline::NoteReference(id) => add_expanded(&mut total, self.note_marker(*id).len())?,
                Inline::Formula(_) => add_expanded(&mut total,64)?,
            }
        }
        Ok(())
    }

    /// Basic author/year rendering of an ordered multi-source field; this is not CSL/APA.
    /// Prefixes/suffixes are literal. Author suppression hides people/corporations; title
    /// suppression hides the title fallback of anonymous sources. Locators use "S. ".
    pub fn citation_cluster_text(&self, items: &[CitationItem]) -> String {
        if let [one] = items {
            if one.simple() {
                return self.citation_text(one.source, &one.locator);
            }
        }
        let rendered = items
            .iter()
            .map(|item| {
                let mut parts = Vec::new();
                if let Some(source) = self.source(item.source) {
                    let anonymous = source.authors.is_empty() && source.corporate.is_empty();
                    if if anonymous {
                        !item.suppress_title
                    } else {
                        !item.suppress_author
                    } {
                        parts.push(source.citation_author());
                    }
                    if !item.suppress_year && !source.year.is_empty() {
                        parts.push(source.year.clone());
                    }
                    if !item.locator.is_empty() {
                        parts.push(format!("S. {}", item.locator));
                    }
                } else {
                    parts.push(String::from("[Quelle fehlt]"));
                }
                format!("{}{}{}", item.prefix, parts.join(", "), item.suffix)
            })
            .collect::<Vec<_>>();
        format!("({})", rendered.join("; "))
    }

    /// Distinct cited source IDs in first-use order, scanning the document only once.
    /// Bibliography fields and unreferenced sources do not add IDs. Validate the document
    /// at trust boundaries; for valid documents at most 1,024 IDs are returned.
    pub fn cited_source_ids(&self) -> Vec<Id> {
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        self.visit_inlines(|_, _, node| {
            let mut add = |id| {
                if seen.insert(id) {
                    ordered.push(id);
                }
            };
            match node {
                Inline::Citation { source, .. } => add(*source),
                Inline::CitationCluster(items) => {
                    for item in items {
                        add(item.source);
                    }
                }
                _ => (),
            }
        });
        ordered
    }

    /// Citation fields using a source, once per field in document order.
    pub fn source_uses(&self, source: Id) -> Vec<CitationLocation> {
        let mut uses = Vec::new();
        self.visit_inlines(|paragraph, inline, node| {
            let contains = match node {
                Inline::Citation { source: id, .. } => *id == source,
                Inline::CitationCluster(items) => items.iter().any(|i| i.source == source),
                _ => false,
            };
            if contains {
                uses.push(CitationLocation { paragraph, inline });
            }
        });
        uses
    }
}

#[derive(Default)]
struct ExpandedSource {
    author: usize,
    year: usize,
    anonymous: bool,
}
fn add_expanded(total: &mut usize, amount: usize) -> Result<(), Error> {
    *total = total.saturating_add(amount);
    if *total > 4 << 20 {
        Err(Error::Limit)
    } else {
        Ok(())
    }
}
