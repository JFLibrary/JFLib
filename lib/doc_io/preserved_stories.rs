// Related Word stories resolved by OPC identity, never guessed from ZIP file names.
use crate::{Error, parse, preservation};
use alloc::{format, string::String, vec::Vec};
use archive::Entry;

/// The Word story containing an individually editable preserved text leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocxStory {
    /// Main document, including eligible table-cell paragraphs.
    Main,
    /// A section header part; multiple sections can share the same part.
    Header,
    /// A section footer part.
    Footer,
    /// A normal footnote (separator/continuation definitions are excluded).
    Footnote,
    /// A normal endnote (separator/continuation definitions are excluded).
    Endnote,
}
impl DocxStory {
    pub(super) fn root(self) -> &'static str {
        match self {
            Self::Main => "document",
            Self::Header => "hdr",
            Self::Footer => "ftr",
            Self::Footnote => "footnotes",
            Self::Endnote => "endnotes",
        }
    }
}

pub(super) fn related(
    entries: &[Entry],
    main: &str,
    types: &preservation::Types,
) -> Result<Vec<(String, DocxStory)>, Error> {
    let mut found = Vec::new();
    for entry in entries.iter().filter(|e| e.name.ends_with(".rels")) {
        if preservation::relationship_source(&entry.name)? != main {
            continue;
        }
        let root = parse(&entry.data)?;
        for relationship in &root.children {
            if relationship.attr("", "TargetMode") == Some("External") {
                continue;
            }
            let Some(kind) = relationship.attr("", "Type").and_then(|s| {
                s.strip_prefix(
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/",
                )
                .or_else(|| {
                    s.strip_prefix("http://purl.oclc.org/ooxml/officeDocument/relationships/")
                })
            }) else {
                continue;
            };
            let story = match kind {
                "header" => DocxStory::Header,
                "footer" => DocxStory::Footer,
                "footnotes" => DocxStory::Footnote,
                "endnotes" => DocxStory::Endnote,
                _ => continue,
            };
            let target = preservation::resolve(
                main,
                relationship.attr("", "Target").ok_or(Error::Invalid)?,
            )?;
            if preservation::content_type(types, &target)
                != Some(
                    format!(
                        "application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml"
                    )
                    .as_str(),
                )
            {
                continue;
            }
            if !found.iter().any(|(path, _)| *path == target) {
                found.push((target, story));
            }
        }
    }
    Ok(found)
}
