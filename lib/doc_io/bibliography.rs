// Shared Word bibliography XML codec: used by DOCX and standalone source lists.
use crate::package::{B, DECL};
use crate::{Error, Node, container, esc, leaf, parse, source_id, unsupported};
use alloc::{collections::BTreeSet, format, string::String, vec::Vec};
use doc_core::{Person, Source, SourceKind};

// A single table drives read and write, so optional supported metadata cannot disappear.
macro_rules! fields {
    ($s:ident, $visit:ident) => {
        $visit!("Title", $s.title);
        $visit!("Year", $s.year);
        $visit!("Publisher", $s.publisher);
        $visit!("City", $s.city);
        $visit!("Edition", $s.edition);
        $visit!("JournalName", $s.journal);
        $visit!("Volume", $s.volume);
        $visit!("Issue", $s.issue);
        $visit!("Pages", $s.pages);
        $visit!("InternetSiteTitle", $s.website);
        $visit!("URL", $s.url);
        $visit!("StandardNumber", $s.standard_number);
        $visit!("YearAccessed", $s.year_accessed);
        $visit!("MonthAccessed", $s.month_accessed);
        $visit!("DayAccessed", $s.day_accessed);
        $visit!("ShortTitle", $s.short_title);
        $visit!("Comments", $s.comments);
        $visit!("Guid", $s.external_id);
        $visit!("LCID", $s.language);
    };
}
fn element(out: &mut String, tag: &str, value: &str) -> Result<(), Error> {
    if !value.is_empty() {
        out.push_str(&format!("<b:{tag}>{}</b:{tag}>", esc(value)?));
    }
    Ok(())
}
/// Encode supported sources as Word Sources.xml, with document-local jf<ID> tags.
/// Input IDs must be unique and nonzero. Output is UTF-8, without external resources.
pub fn encode_sources(sources: &[Source]) -> Result<Vec<u8>, Error> {
    validate_list(sources)?;
    let mut ids = BTreeSet::new();
    let mut out = format!("{DECL}<b:Sources xmlns:b=\"{B}\">");
    for s in sources {
        if s.id == 0 || s.id >= 0x7fff_ffff || !ids.insert(s.id) {
            return Err(Error::Invalid);
        }
        out.push_str("<b:Source>");
        element(&mut out, "Tag", &format!("jf{}", s.id))?;
        element(
            &mut out,
            "SourceType",
            match s.kind {
                SourceKind::Book => "Book",
                SourceKind::JournalArticle => "JournalArticle",
                SourceKind::Website => "InternetSite",
            },
        )?;
        if !s.authors.is_empty() || !s.corporate.is_empty() {
            out.push_str("<b:Author><b:Author>");
            if !s.corporate.is_empty() {
                element(&mut out, "Corporate", &s.corporate)?;
            } else {
                out.push_str("<b:NameList>");
                for person in &s.authors {
                    out.push_str("<b:Person>");
                    element(&mut out, "Last", &person.family)?;
                    element(&mut out, "First", &person.given)?;
                    element(&mut out, "Middle", &person.middle)?;
                    out.push_str("</b:Person>");
                }
                out.push_str("</b:NameList>");
            }
            out.push_str("</b:Author></b:Author>");
        }
        macro_rules! write_field {
            ($name:literal, $field:expr) => {
                element(&mut out, $name, &$field)?;
            };
        }
        fields!(s, write_field);
        out.push_str("</b:Source>");
    }
    out.push_str("</b:Sources>");
    // Escaping and many authors can exceed XML limits even within the model budget.
    parse(out.as_bytes())?;
    Ok(out.into_bytes())
}
fn validate_list(sources: &[Source]) -> Result<(), Error> {
    if sources.len() > 1024 || sources.iter().map(Source::byte_size).sum::<usize>() > 2 << 20 {
        return Err(Error::Limit);
    }
    for source in sources {
        source.validate()?;
    }
    Ok(())
}
fn only<'a>(n: &'a Node, name: &str) -> Result<&'a Node, Error> {
    container(n, B, name)?;
    if n.children.len() != 1 {
        return Err(unsupported(name));
    }
    Ok(&n.children[0])
}
fn authors(n: &Node, s: &mut Source) -> Result<(), Error> {
    let a = only(n, "Author")?;
    let list = only(a, "Author")?;
    if list.is(B, "Corporate") {
        s.corporate = leaf(list, B, "Corporate")?;
        return Ok(());
    }
    container(list, B, "NameList")?;
    if list.children.len() > 64 {
        return Err(Error::Limit);
    }
    for n in &list.children {
        container(n, B, "Person")?;
        let mut person = Person::default();
        let mut seen = BTreeSet::new();
        for c in &n.children {
            if !seen.insert(&c.name.local) {
                return Err(Error::Invalid);
            }
            let text = leaf(c, B, &c.name.local)?;
            match c.name.local.as_str() {
                "First" => person.given = text,
                "Middle" => person.middle = text,
                "Last" => person.family = text,
                _ => return Err(unsupported(&c.name.local)),
            }
        }
        // The initial jfWord version wrote one empty Person for anonymous books.
        if !person.given.is_empty() || !person.middle.is_empty() || !person.family.is_empty() {
            s.authors.push(person);
        }
    }
    Ok(())
}
pub(crate) fn read_sources(root: &Node, document_ids: bool) -> Result<Vec<Source>, Error> {
    if !root.is(B, "Sources") || !root.text.is_empty() {
        return Err(unsupported("Sources"));
    }
    for (name, _) in &root.attrs {
        if !name.ns.is_empty()
            || !matches!(name.local.as_str(), "SelectedStyle" | "StyleName" | "URI")
        {
            return Err(unsupported(&name.local));
        }
    }
    if root.children.len() > 1024 {
        return Err(Error::Limit);
    }
    let mut tags = BTreeSet::new();
    let mut out = Vec::new();
    for n in &root.children {
        container(n, B, "Source")?;
        let mut s = Source::default();
        let mut seen = BTreeSet::new();
        let mut tag = String::new();
        for c in &n.children {
            if !seen.insert(c.name.local.as_str()) {
                return Err(Error::Invalid);
            }
            if c.is(B, "Author") {
                authors(c, &mut s)?;
                continue;
            }
            let text = leaf(c, B, &c.name.local)?;
            match c.name.local.as_str() {
                "Tag" => tag = text,
                "SourceType" => {
                    s.kind = match text.as_str() {
                        "Book" => SourceKind::Book,
                        "JournalArticle" => SourceKind::JournalArticle,
                        "InternetSite" => SourceKind::Website,
                        _ => return Err(unsupported(&text)),
                    }
                }
                // Reference order and list style belong to the originating document, not the source.
                "RefOrder" if text.parse::<u32>().is_ok() => (),
                _ => {
                    let mut known = false;
                    macro_rules! read_field {
                        ($name:literal, $field:expr) => {
                            if c.name.local == $name {
                                $field = text.clone();
                                known = true;
                            }
                        };
                    }
                    fields!(s, read_field);
                    if !known {
                        return Err(unsupported(&c.name.local));
                    }
                }
            }
        }
        if tag.trim().is_empty()
            || tag.len() > 4096
            || !tags.insert(tag.clone())
            || !seen.contains("SourceType")
        {
            return Err(Error::Invalid);
        }
        s.id = if document_ids {
            source_id(&tag)?
        } else {
            out.len() as u64 + 1
        };
        out.push(s);
    }
    validate_list(&out)?;
    Ok(out)
}
/// Decode a complete standalone Word source list. External tags are validated but remapped
/// to sequential IDs; use Edit::ImportSources to allocate identities in an existing document.
/// Unrepresented fields/types abort the whole import; no partial or silent content import.
pub fn decode_sources(bytes: &[u8]) -> Result<Vec<Source>, Error> {
    read_sources(&parse(bytes)?, false)
}
/// Load a bounded standalone Word bibliography XML file.
pub fn load_sources(path: &str) -> Result<Vec<Source>, Error> {
    decode_sources(&crate::read_file(path, 4 << 20)?)
}
/// Export a source list using the platform's temporary-file/rename write.
pub fn save_sources(path: &str, sources: &[Source]) -> Result<(), Error> {
    if path.is_empty() || path.contains('\0') {
        return Err(Error::Invalid);
    }
    let bytes = encode_sources(sources)?;
    if plat::write_file(path, &bytes) {
        Ok(())
    } else {
        Err(Error::Io)
    }
}
