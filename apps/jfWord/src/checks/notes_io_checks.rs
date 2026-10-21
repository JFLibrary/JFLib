// Native contracts for note parts, strict loss boundaries and cross-scope DOCX identities.
use alloc::{format, string::String, vec, vec::Vec};
use archive::{Entry, Limits};
use doc_core::{
    Alignment, CitationItem, Document, Inline, Note, NoteKind, Paragraph, ParagraphFormat,
    ParagraphStyle, Source, TextFormat,
};

fn paragraph(id: u64, content: Vec<Inline>) -> Paragraph {
    Paragraph {
        id,
        style: ParagraphStyle::Normal,
        format: ParagraphFormat::default(),
        content,
    }
}
pub(crate) fn sample() -> Document {
    let mut d = Document::default();
    d.sources.push(Source {
        id: 2,
        corporate: String::from("Forschungsinstitut"),
        title: String::from("Noten & Quellen"),
        year: String::from("2026"),
        ..Default::default()
    });
    d.paragraphs[0].content = vec![
        Inline::Text(String::from("Aussage α")),
        Inline::NoteReference(30),
        Inline::Text(String::from(" mit Hintergrund")),
        Inline::NoteReference(20),
        Inline::Text(String::from(" und Erläuterung")),
        Inline::NoteReference(10),
        Inline::Text(String::from(".")),
    ];
    d.paragraphs.push(paragraph(3, vec![Inline::Bibliography]));
    d.notes = vec![
        Note {
            id: 20,
            kind: NoteKind::Endnote,
            paragraphs: vec![paragraph(
                21,
                vec![Inline::Text(String::from("Hintergrund Ω\t中文\r\n"))],
            )],
        },
        Note {
            id: 10,
            kind: NoteKind::Footnote,
            paragraphs: vec![
                paragraph(
                    11,
                    vec![
                        Inline::Text(String::from("Erläuterung ")),
                        Inline::Citation {
                            source: 2,
                            locator: String::from("12–14"),
                        },
                    ],
                ),
                paragraph(12, vec![Inline::Text(String::from("Zweiter Absatz."))]),
            ],
        },
        Note {
            id: 30,
            kind: NoteKind::Footnote,
            paragraphs: vec![paragraph(
                31,
                vec![
                    Inline::with_format(
                        String::from("Beleg "),
                        TextFormat {
                            italic: Some(true),
                            size_half_points: Some(20),
                            ..Default::default()
                        },
                    ),
                    Inline::citation(vec![CitationItem {
                        source: 2,
                        prefix: String::from("vgl. "),
                        suffix: String::from(", eigene Übersetzung"),
                        locator: String::from("9"),
                        ..Default::default()
                    }])
                    .unwrap(),
                    Inline::Text(String::new()),
                ],
            )],
        },
    ];
    d.notes[1].paragraphs[1].format.alignment = Alignment::Justify;
    d
}
fn replace(entries: &[Entry], path: &str, from: &str, to: &str) -> Vec<Entry> {
    entries
        .iter()
        .map(|e| Entry {
            name: e.name.clone(),
            data: if e.name == path {
                String::from_utf8(e.data.clone())
                    .unwrap()
                    .replace(from, to)
                    .into_bytes()
            } else {
                e.data.clone()
            },
        })
        .collect()
}
fn decode(entries: &[Entry]) -> Result<Document, doc_io::Error> {
    doc_io::decode_docx(&archive::write(entries, Limits::default()).unwrap())
}
fn text<'a>(entries: &'a [Entry], path: &str) -> &'a str {
    core::str::from_utf8(&entries.iter().find(|e| e.name == path).unwrap().data).unwrap()
}
pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($ok:expr, $name:expr) => {
            if $ok {
                passed += 1;
            } else {
                failed += 1;
                plat::write_stdout(format!("FAIL {}\n", $name).as_bytes());
            }
        };
    }
    let d = sample();
    let bytes = doc_io::encode_docx(&d).unwrap();
    let entries = archive::read(&bytes, Limits::default()).unwrap();
    check!(
        doc_io::decode_docx(&bytes) == Ok(d.clone()),
        "note DOCX preserves mixed storage order, every ID, formatting and citation metadata"
    );
    check!(
        doc_io::encode_docx(&doc_io::decode_docx(&bytes).unwrap()) == Ok(bytes.clone()),
        "note DOCX second save is byte deterministic"
    );
    check!(
        entries.len() == 11
            && text(&entries, "word/footnotes.xml").contains("w:type=\"separator\" w:id=\"-1\"")
            && text(&entries, "word/endnotes.xml").contains("<w:endnoteRef/>")
            && text(&entries, "word/document.xml").contains("<w:numFmt w:val=\"decimal\"/>"),
        "note parts, standard separators, automatic markers and explicit decimal numbering"
    );
    check!(
        text(&entries, "word/settings.xml").contains(
            "<w:footnotePr><w:footnote w:id=\"-1\"/><w:footnote w:id=\"0\"/></w:footnotePr>"
        ) && text(&entries, "word/settings.xml")
            .contains("<w:endnotePr><w:endnote w:id=\"-1\"/><w:endnote w:id=\"0\"/></w:endnotePr>"),
        "document settings declare every special footnote and endnote"
    );
    check!(
        d.cited_source_ids() == vec![2]
            && d.source_uses(2).len() == 2
            && d.bibliography().len() == 1,
        "sources cited only inside note bodies feed the main bibliography"
    );
    for kind in [NoteKind::Footnote, NoteKind::Endnote] {
        let mut one = d.clone();
        one.notes.retain(|n| n.kind == kind);
        let ids: Vec<_> = one.notes.iter().map(|n| n.id).collect();
        one.paragraphs[0]
            .content
            .retain(|i| !matches!(i, Inline::NoteReference(id) if !ids.contains(id)));
        let encoded = doc_io::encode_docx(&one).unwrap();
        check!(
            archive::read(&encoded, Limits::default()).unwrap().len() == 10
                && doc_io::decode_docx(&encoded) == Ok(one),
            "single note kind uses its body part and required document settings"
        );
    }
    let plain = doc_io::encode_docx(&Document::default()).unwrap();
    check!(
        archive::read(&plain, Limits::default()).unwrap().len() == 8
            && doc_io::decode_docx(&plain) == Ok(Document::default()),
        "documents without notes retain the original eight-part profile"
    );

    let mut collision = replace(&entries, "word/footnotes.xml", "w:id=\"30\"", "w:id=\"1\"");
    collision = replace(&collision, "word/endnotes.xml", "w:id=\"20\"", "w:id=\"1\"");
    collision = replace(
        &collision,
        "word/document.xml",
        "footnoteReference w:id=\"30\"",
        "footnoteReference w:id=\"1\"",
    );
    collision = replace(
        &collision,
        "word/document.xml",
        "endnoteReference w:id=\"20\"",
        "endnoteReference w:id=\"1\"",
    );
    let remapped = decode(&collision).unwrap();
    check!(
        remapped.validate().is_ok()
            && remapped.notes[0].id != remapped.notes[2].id
            && remapped.notes[0].id != 1
            && remapped.notes[2].id != 1
            && remapped.notes[0].paragraphs == d.notes[0].paragraphs
            && remapped.notes[2].paragraphs == d.notes[2].paragraphs
            && remapped.note_number(remapped.notes[0].id) == Some(1)
            && remapped.note_number(remapped.notes[2].id) == Some(1),
        "foreign foot/end note IDs are independently mapped around all global model IDs"
    );
    check!(
        doc_io::decode_docx(&doc_io::encode_docx(&remapped).unwrap()) == Ok(remapped),
        "remapped foreign note identities are stable on subsequent native save"
    );
    let zero = replace(
        &entries,
        "word/footnotes.xml",
        "<w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>",
        "",
    );
    let zero = replace(&zero, "word/footnotes.xml", "w:id=\"30\"", "w:id=\"0\"");
    let zero = replace(
        &zero,
        "word/document.xml",
        "footnoteReference w:id=\"30\"",
        "footnoteReference w:id=\"0\"",
    );
    check!(
        decode(&zero).is_err(),
        "normal note cannot replace the required declared system continuation separator"
    );

    let mut foreign = entries.clone();
    for path in ["word/footnotes.xml", "word/endnotes.xml"] {
        for index in 0..3 {
            foreign = replace(&foreign, path, &format!(" jf:order=\"{index}\""), "");
        }
        for id in [11, 12, 21, 31] {
            foreign = replace(&foreign, path, &format!(" w14:paraId=\"{id:08X}\""), "");
        }
    }
    let imported = decode(&foreign).unwrap();
    check!(
        imported.notes.len() == 3
            && imported.validate().is_ok()
            && imported
                .notes
                .iter()
                .flat_map(|n| &n.paragraphs)
                .all(|p| p.id != 0)
            && imported.source_uses(2).len() == 2,
        "foreign note paragraphs without optional paraId receive unique model IDs"
    );
    let no_foot_properties = replace(
        &entries,
        "word/document.xml",
        "<w:footnotePr><w:pos w:val=\"pageBottom\"/><w:numFmt w:val=\"decimal\"/><w:numStart w:val=\"1\"/><w:numRestart w:val=\"continuous\"/></w:footnotePr>",
        "",
    );
    check!(
        decode(&no_foot_properties) == Ok(d.clone()),
        "implicit footnote defaults are represented decimal continuous page-bottom"
    );
    let normal_type = replace(
        &entries,
        "word/footnotes.xml",
        "w:id=\"10\" jf:order",
        "w:type=\"normal\" w:id=\"10\" jf:order",
    );
    check!(
        decode(&normal_type) == Ok(d.clone()),
        "explicit normal note type matches the omitted default"
    );

    for (path, from, to, label) in [
        (
            "word/document.xml",
            "footnoteReference w:id=\"30\"",
            "footnoteReference w:id=\"999\"",
            "dangling note reference",
        ),
        (
            "word/document.xml",
            "footnoteReference w:id=\"30\"",
            "endnoteReference w:id=\"30\"",
            "wrong-kind note reference",
        ),
        (
            "word/document.xml",
            "footnoteReference w:id=\"30\"",
            "footnoteReference w:id=\"10\"",
            "duplicate note anchor and orphan body",
        ),
        (
            "word/document.xml",
            "footnoteReference w:id=\"30\"",
            "footnoteReference w:id=\"30\" w:customMarkFollows=\"1\"",
            "custom note marker",
        ),
        (
            "word/document.xml",
            "footnoteReference w:id=\"30\"",
            "footnoteReference w:id=\"-1\"",
            "separator cannot be a reference target",
        ),
        (
            "word/document.xml",
            "<w:numFmt w:val=\"decimal\"/>",
            "<w:numFmt w:val=\"lowerRoman\"/>",
            "unrepresented numbering format",
        ),
        (
            "word/document.xml",
            "<w:numStart w:val=\"1\"/>",
            "<w:numStart w:val=\"3\"/>",
            "nondefault numbering start",
        ),
        (
            "word/document.xml",
            "<w:numRestart w:val=\"continuous\"/>",
            "<w:numRestart w:val=\"eachPage\"/>",
            "page restart numbering",
        ),
        (
            "word/document.xml",
            "<w:pos w:val=\"pageBottom\"/>",
            "<w:pos w:val=\"beneathText\"/>",
            "nondefault note placement",
        ),
        (
            "word/document.xml",
            "<w:endnotePr><w:pos w:val=\"docEnd\"/><w:numFmt w:val=\"decimal\"/><w:numStart w:val=\"1\"/><w:numRestart w:val=\"continuous\"/></w:endnotePr>",
            "",
            "ambiguous implicit Word endnote format",
        ),
        (
            "word/footnotes.xml",
            "w:id=\"30\"",
            "w:id=\"10\"",
            "duplicate same-kind note identity",
        ),
        (
            "word/footnotes.xml",
            "w:id=\"30\"",
            "w:id=\"-2\"",
            "negative normal note identity",
        ),
        (
            "word/footnotes.xml",
            "w14:paraId=\"0000001F\"",
            "w14:paraId=\"0000000B\"",
            "duplicate note paragraph identity",
        ),
        (
            "word/footnotes.xml",
            "<w:footnoteRef/>",
            "<w:endnoteRef/>",
            "wrong-kind note body marker",
        ),
        (
            "word/footnotes.xml",
            "<w:footnoteRef/>",
            "<w:footnoteRef/><w:footnoteRef/>",
            "repeated note body marker",
        ),
        (
            "word/footnotes.xml",
            "<w:footnoteRef/>",
            "<w:t>Hidden mark text</w:t><w:footnoteRef/>",
            "mixed marker run text",
        ),
        (
            "word/footnotes.xml",
            "<w:footnoteRef/>",
            "<w:footnoteReference w:id=\"30\"/>",
            "nested note anchor",
        ),
        (
            "word/footnotes.xml",
            "<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:footnoteRef/></w:r>",
            "",
            "missing automatic note body marker",
        ),
        (
            "word/footnotes.xml",
            "<w:vertAlign w:val=\"superscript\"/>",
            "<w:b/>",
            "unrepresented mark formatting",
        ),
        (
            "word/footnotes.xml",
            " jf:order=\"1\"",
            "",
            "partially declared native note order",
        ),
        (
            "word/footnotes.xml",
            " jf:order=\"1\"",
            " jf:order=\"0\"",
            "duplicate native note order",
        ),
        (
            "word/footnotes.xml",
            "<w:separator/>",
            "<w:t>Custom separator</w:t>",
            "unrepresented separator contents",
        ),
        (
            "word/footnotes.xml",
            "<w:continuationSeparator/>",
            "<w:continuationSeparator w:val=\"x\"/>",
            "separator metadata",
        ),
        (
            "word/footnotes.xml",
            "w:type=\"separator\" w:id=\"-1\"",
            "w:type=\"separator\" w:id=\"10\"",
            "separator and note ID collision",
        ),
        (
            "word/_rels/document.xml.rels",
            "Target=\"footnotes.xml\"",
            "Target=\"endnotes.xml\"",
            "wrong note relationship target",
        ),
        (
            "word/_rels/document.xml.rels",
            "Target=\"footnotes.xml\"",
            "Target=\"https://example.org/notes\" TargetMode=\"External\"",
            "external note relationship",
        ),
        (
            "[Content_Types].xml",
            "wordprocessingml.footnotes+xml",
            "wordprocessingml.endnotes+xml",
            "wrong note content type",
        ),
    ] {
        check!(decode(&replace(&entries, path, from, to)).is_err(), label);
    }
    let mut missing_part = entries.clone();
    missing_part.retain(|e| e.name != "word/footnotes.xml");
    check!(
        decode(&missing_part).is_err(),
        "missing note part rejects before editable document exposure"
    );
    let mut missing_settings = entries.clone();
    missing_settings.retain(|e| e.name != "word/settings.xml");
    check!(
        decode(&missing_settings).is_err(),
        "notes require a document settings part declaring special notes"
    );
    for path in [
        "word/document.xml",
        "word/footnotes.xml",
        "word/endnotes.xml",
    ] {
        for replacement in [
            "",
            "<w:rPr/>",
            "<w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr>",
        ] {
            check!(
                decode(&replace(
                    &entries,
                    path,
                    "<w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr>",
                    replacement
                ))
                .is_err(),
                "marker baseline or undefined style cannot silently become superscript"
            );
        }
    }
    for (path, from, to) in [
        ("word/settings.xml", "<w:footnote w:id=\"-1\"/>", ""),
        ("word/settings.xml", "<w:endnote w:id=\"0\"/>", ""),
        (
            "word/settings.xml",
            "<w:footnote w:id=\"-1\"/>",
            "<w:footnote w:id=\"1\"/>",
        ),
        (
            "word/settings.xml",
            "<w:footnote w:id=\"-1\"/>",
            "<w:footnote w:id=\"-1\"/><w:footnote w:id=\"-1\"/>",
        ),
        (
            "word/footnotes.xml",
            "<w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>",
            "",
        ),
        (
            "word/endnotes.xml",
            "<w:endnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:endnote>",
            "",
        ),
        (
            "word/_rels/document.xml.rels",
            "Target=\"settings.xml\"",
            "Target=\"missing.xml\"",
        ),
        (
            "[Content_Types].xml",
            "wordprocessingml.settings+xml",
            "wordprocessingml.styles+xml",
        ),
    ] {
        check!(
            decode(&replace(&entries, path, from, to)).is_err(),
            "special note settings and parts must agree completely"
        );
    }
    let mut extra_part = entries.clone();
    extra_part.push(Entry {
        name: String::from("word/_rels/footnotes.xml.rels"),
        data: b"<Relationships/>".to_vec(),
    });
    check!(
        decode(&extra_part).is_err(),
        "unknown note subpart relationships remain rejected"
    );
    let mut invalid = d.clone();
    invalid.notes[0].paragraphs[0]
        .content
        .push(Inline::NoteReference(10));
    check!(
        doc_io::encode_docx(&invalid).is_err(),
        "encoder rejects nested model note before writing package"
    );
    invalid = d.clone();
    invalid.notes[0].paragraphs[0]
        .content
        .push(Inline::Bibliography);
    check!(
        doc_io::encode_docx(&invalid).is_err(),
        "encoder rejects note bibliography before writing package"
    );
    if base::Args(plat::args()).value("--fixture").is_some() {
        check!(
            doc_io::save_docx("build/jfword-notes.docx", &d).is_ok(),
            "native notes DOCX fixture for independent verification"
        );
    }
    (passed, failed)
}
