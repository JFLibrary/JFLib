// Bounded footnote/endnote OPC profile and global-ID mapping. New native library code.
use crate::{
    Error, Node, container, find, format, package::*, parse, read_paragraph, unsupported,
    write_paragraph,
};
use alloc::{collections::BTreeSet, string::String, vec::Vec};
use archive::Entry;
use doc_core::{Document, Id, Inline, Note, NoteKind, PageSettings, Paragraph};

const JF: &str = "urn:jfword:notes:1";
const MAX_ID: Id = 0x7fff_ffff;
pub(crate) const KINDS: [NoteKind; 2] = [NoteKind::Footnote, NoteKind::Endnote];

pub(crate) fn name(kind: NoteKind) -> &'static str {
    match kind {
        NoteKind::Footnote => "footnote",
        NoteKind::Endnote => "endnote",
    }
}
pub(crate) fn path(kind: NoteKind) -> &'static str {
    match kind {
        NoteKind::Footnote => "word/footnotes.xml",
        NoteKind::Endnote => "word/endnotes.xml",
    }
}
pub(crate) fn present(d: &Document, kind: NoteKind) -> bool {
    d.notes.iter().any(|n| n.kind == kind)
}
pub(crate) fn settings(has_notes: [bool; 2]) -> String {
    let mut out = format!("{DECL}<w:settings xmlns:w=\"{W}\">");
    for (index, kind) in KINDS.into_iter().enumerate() {
        if has_notes[index] {
            let tag = name(kind);
            out.push_str(&format!(
                "<w:{tag}Pr><w:{tag} w:id=\"-1\"/><w:{tag} w:id=\"0\"/></w:{tag}Pr>"
            ));
        }
    }
    out.push_str("</w:settings>");
    out
}

/// Map separately scoped OOXML note identifiers to globally unique model identifiers.
pub(crate) struct Mapping {
    kind: NoteKind,
    raw: Id,
    id: Id,
}
struct RawNote {
    kind: NoteKind,
    raw: Id,
    order: Option<usize>,
    paragraphs: Vec<Paragraph>,
}

fn number(s: &str) -> Result<Id, Error> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Invalid);
    }
    let n = s.parse().map_err(|_| Error::Invalid)?;
    if n >= MAX_ID {
        return Err(Error::Limit);
    }
    Ok(n)
}
pub(crate) fn allocate(used: &mut BTreeSet<Id>, next: &mut Id) -> Result<Id, Error> {
    while used.contains(next) {
        *next += 1;
    }
    if *next >= MAX_ID {
        return Err(Error::Limit);
    }
    let id = *next;
    used.insert(id);
    *next += 1;
    Ok(id)
}

pub(crate) fn write(d: &Document, kind: NoteKind) -> Result<Vec<u8>, Error> {
    let tag = name(kind);
    let mut out = format!(
        "{DECL}<w:{tag}s xmlns:w=\"{W}\" xmlns:w14=\"{W14}\" xmlns:mc=\"{MC}\" xmlns:jf=\"{JF}\" mc:Ignorable=\"w14 jf\">"
    );
    for (id, separator) in [(-1, "separator"), (0, "continuationSeparator")] {
        out.push_str(&format!(
            "<w:{tag} w:type=\"{separator}\" w:id=\"{id}\"><w:p><w:r><w:{separator}/></w:r></w:p></w:{tag}>"
        ));
    }
    for (order, note) in d.notes.iter().enumerate().filter(|(_, n)| n.kind == kind) {
        out.push_str(&format!(
            "<w:{tag} w:id=\"{}\" jf:order=\"{order}\">",
            note.id
        ));
        for (index, p) in note.paragraphs.iter().enumerate() {
            out.push_str(&write_paragraph(d, p, (index == 0).then_some(kind))?);
        }
        out.push_str(&format!("</w:{tag}>"));
    }
    out.push_str(&format!("</w:{tag}s>"));
    parse(out.as_bytes())?;
    Ok(out.into_bytes())
}

fn read_part(root: &Node, kind: NoteKind) -> Result<Vec<RawNote>, Error> {
    let tag = name(kind);
    if !root.is(W, &format!("{tag}s"))
        || !root.text.is_empty()
        || root.attrs.iter().any(|(a, v)| {
            a.ns != MC || a.local != "Ignorable" || !matches!(v.as_str(), "w14" | "w14 jf")
        })
    {
        return Err(unsupported("Notenattribute"));
    }
    let mut out = Vec::new();
    let mut ids = BTreeSet::new();
    let mut separators = BTreeSet::new();
    for n in &root.children {
        if !n.is(W, tag) || !n.text.is_empty() {
            return Err(unsupported(&n.name.local));
        }
        let raw = n.attr(W, "id").ok_or(Error::Invalid)?;
        // Signed decimal identity comparison rejects aliases such as 00 and 0.
        let signed = raw.parse::<i32>().map_err(|_| Error::Invalid)?;
        if !ids.insert(signed) {
            return Err(Error::Invalid);
        }
        let kind_attr = n.attr(W, "type");
        if matches!(kind_attr, Some("separator" | "continuationSeparator")) {
            let separator = kind_attr.unwrap();
            if n.attrs.len() != 2
                || !separators.insert(separator)
                || signed != if separator == "separator" { -1 } else { 0 }
                || n.children.len() != 1
            {
                return Err(unsupported("Notentrennlinie"));
            }
            let p = &n.children[0];
            container(p, W, "p")?;
            if p.children.len() != 1 {
                return Err(unsupported("Notentrennlinie"));
            }
            let r = &p.children[0];
            container(r, W, "r")?;
            if r.children.len() != 1 {
                return Err(unsupported("Notentrennlinie"));
            }
            container(&r.children[0], W, separator)?;
            if !r.children[0].children.is_empty() {
                return Err(unsupported("Notentrennlinie"));
            }
            continue;
        }
        if kind_attr.is_some() && kind_attr != Some("normal") {
            return Err(unsupported("Notentyp"));
        }
        if n.attrs.iter().any(|(a, _)| {
            !(a.ns == W && matches!(a.local.as_str(), "id" | "type")
                || a.ns == JF && a.local == "order")
        }) {
            return Err(unsupported("Notenattribute"));
        }
        let raw = number(raw)?;
        let order = n
            .attr(JF, "order")
            .map(number)
            .transpose()?
            .map(|n| n as usize);
        if order.is_some() && root.attr(MC, "Ignorable") != Some("w14 jf") {
            return Err(unsupported("Notenreihenfolge"));
        }
        if n.children.is_empty() {
            return Err(Error::Invalid);
        }
        let mut paragraphs = Vec::new();
        for (i, p) in n.children.iter().enumerate() {
            paragraphs.push(read_paragraph(p, &[], Some((kind, i == 0)))?);
        }
        out.push(RawNote {
            kind,
            raw,
            order,
            paragraphs,
        });
    }
    if out.is_empty() || separators.len() != 2 {
        return Err(Error::Invalid);
    }
    Ok(out)
}

pub(crate) fn read(
    entries: &[Entry],
    used: &mut BTreeSet<Id>,
) -> Result<(Vec<Note>, Vec<Mapping>), Error> {
    let mut raw_notes = Vec::new();
    for kind in KINDS {
        if entries.iter().any(|e| e.name == path(kind)) {
            raw_notes.extend(read_part(&parse(find(entries, path(kind))?)?, kind)?);
        }
    }
    if raw_notes.iter().any(|n| n.order.is_some()) {
        raw_notes.sort_by_key(|n| n.order);
        if raw_notes
            .iter()
            .enumerate()
            .any(|(i, n)| n.order != Some(i))
        {
            return Err(unsupported("Notenreihenfolge"));
        }
    }
    for note in &raw_notes {
        for p in &note.paragraphs {
            if p.id != 0 && !used.insert(p.id) {
                return Err(Error::Invalid);
            }
        }
    }
    let mut mapping = Vec::new();
    let mut remap = Vec::new();
    // Reserve every noncolliding foreign note ID before allocating replacements.
    for note in &raw_notes {
        let id = if note.raw != 0 && used.insert(note.raw) {
            note.raw
        } else {
            0
        };
        remap.push(id);
    }
    let mut next = 1;
    let mut notes = Vec::new();
    for (mut note, id) in raw_notes.into_iter().zip(remap) {
        let id = if id == 0 {
            allocate(used, &mut next)?
        } else {
            id
        };
        for p in &mut note.paragraphs {
            if p.id == 0 {
                p.id = allocate(used, &mut next)?;
            }
        }
        mapping.push(Mapping {
            kind: note.kind,
            raw: note.raw,
            id,
        });
        notes.push(Note {
            id,
            kind: note.kind,
            paragraphs: note.paragraphs,
        });
    }
    Ok((notes, mapping))
}

/// Recognize marker-only runs without allowing hidden text or unsupported mark formatting.
pub(crate) fn marker(n: &Node) -> Option<&str> {
    n.children
        .iter()
        .find(|c| {
            c.name.ns == W
                && matches!(
                    c.name.local.as_str(),
                    "footnoteReference" | "endnoteReference" | "footnoteRef" | "endnoteRef"
                )
        })
        .map(|n| n.name.local.as_str())
}
fn marker_node<'a>(n: &'a Node, tag: &str) -> Result<&'a Node, Error> {
    container(n, W, "r")?;
    let children = if n.children.first().is_some_and(|c| c.is(W, "rPr")) {
        let props = &n.children[0];
        container(props, W, "rPr")?;
        if props.children.len() != 1 {
            return Err(unsupported("Notenzeichenformat"));
        }
        for c in &props.children {
            if c.name.ns != W || !c.children.is_empty() || !c.text.is_empty() || c.attrs.len() != 1
            {
                return Err(unsupported("Notenzeichenformat"));
            }
            if !c.is(W, "vertAlign") || c.attr(W, "val") != Some("superscript") {
                return Err(unsupported("Notenzeichenformat"));
            }
        }
        &n.children[1..]
    } else {
        return Err(unsupported("Notenzeichenformat"));
    };
    if children.len() != 1
        || !children[0].is(W, tag)
        || !children[0].children.is_empty()
        || !children[0].text.is_empty()
    {
        return Err(unsupported("Notenzeichen"));
    }
    Ok(&children[0])
}
pub(crate) fn read_mark(n: &Node, kind: NoteKind) -> Result<(), Error> {
    let tag = format!("{}Ref", name(kind));
    if !marker_node(n, &tag)?.attrs.is_empty() {
        return Err(unsupported("Notenzeichen"));
    }
    Ok(())
}
pub(crate) fn read_reference(n: &Node, mapping: &[Mapping]) -> Result<Inline, Error> {
    let kind = match marker(n) {
        Some("footnoteReference") => NoteKind::Footnote,
        Some("endnoteReference") => NoteKind::Endnote,
        _ => return Err(unsupported("Notenverweis")),
    };
    let tag = format!("{}Reference", name(kind));
    let mark = marker_node(n, &tag)?;
    if mark.attrs.len() != 1 {
        return Err(unsupported("Notenverweisattribute"));
    }
    let raw = number(mark.attr(W, "id").ok_or(Error::Invalid)?)?;
    let id = mapping
        .iter()
        .find(|m| m.kind == kind && m.raw == raw)
        .ok_or(Error::Invalid)?
        .id;
    Ok(Inline::NoteReference(id))
}

pub(crate) fn write_section(d: &Document) -> String {
    let mut properties = String::new();
    for kind in KINDS {
        if present(d, kind) {
            let tag = name(kind);
            let pos = if kind == NoteKind::Footnote {
                "pageBottom"
            } else {
                "docEnd"
            };
            properties.push_str(&format!("<w:{tag}Pr><w:pos w:val=\"{pos}\"/><w:numFmt w:val=\"decimal\"/><w:numStart w:val=\"1\"/><w:numRestart w:val=\"continuous\"/></w:{tag}Pr>"));
        }
    }
    format::write_section(d.page).replacen("<w:sectPr>", &format!("<w:sectPr>{properties}"), 1)
}
pub(crate) fn read_section(n: &Node, has_notes: [bool; 2]) -> Result<PageSettings, Error> {
    let mut plain = n.clone();
    let mut seen = BTreeSet::new();
    let mut decimal_end = false;
    for c in &n.children {
        let kind = if c.is(W, "footnotePr") {
            NoteKind::Footnote
        } else if c.is(W, "endnotePr") {
            NoteKind::Endnote
        } else {
            continue;
        };
        let index = if kind == NoteKind::Footnote { 0 } else { 1 };
        if !has_notes[index] || !seen.insert(index) {
            return Err(unsupported("Notenlayout"));
        }
        container(c, W, &format!("{}Pr", name(kind)))?;
        let mut properties = BTreeSet::new();
        for p in &c.children {
            if p.name.ns != W
                || !p.children.is_empty()
                || !p.text.is_empty()
                || p.attrs.len() != 1
                || !properties.insert(&p.name.local)
            {
                return Err(unsupported("Notenlayout"));
            }
            let expected = match p.name.local.as_str() {
                "numFmt" => "decimal",
                "numStart" => "1",
                "numRestart" => "continuous",
                "pos" => {
                    if kind == NoteKind::Footnote {
                        "pageBottom"
                    } else {
                        "docEnd"
                    }
                }
                _ => return Err(unsupported("Notenlayout")),
            };
            if p.attr(W, "val") != Some(expected) {
                return Err(unsupported("Notennummerierung oder -position"));
            }
            if kind == NoteKind::Endnote && p.is(W, "numFmt") {
                decimal_end = true;
            }
        }
    }
    // Word's implicit endnote format differs from ECMA's decimal default.
    if has_notes[1] && !decimal_end {
        return Err(unsupported(
            "Endnoten benötigen explizit dezimale Nummerierung",
        ));
    }
    plain
        .children
        .retain(|c| !c.is(W, "footnotePr") && !c.is(W, "endnotePr"));
    format::read_section(&plain)
}
