// Headless contract checks against the real no_std library code on both target platforms.
use alloc::{format, string::String, vec};
use base::{
    encoding::{self, Encoding},
    xml,
};
use doc_core::{Document, Edit, Editor, Inline, ParagraphStyle, Person, Source};

pub(crate) fn sample() -> Document {
    let mut e = Editor::new(Document::default()).unwrap();
    e.apply(Edit::SetText {
        paragraph: 1,
        inline: 0,
        text: String::from("Grüße & Forschung <2026>\nα β\t𝄞"),
    })
    .unwrap();
    e.apply(Edit::SetStyle {
        paragraph: 1,
        style: ParagraphStyle::Heading1,
    })
    .unwrap();
    let s = e
        .apply(Edit::PutSource(Source {
            id: 0,
            authors: vec![Person {
                given: String::from("Ada"),
                family: String::from("Lovelace"),
                ..Person::default()
            }],
            title: String::from("Notizen & Methoden"),
            year: String::from("1843"),
            publisher: String::from("London"),
            ..Source::default()
        }))
        .unwrap();
    e.apply(Edit::Cite {
        paragraph: 1,
        source: s,
        locator: String::from("42–44"),
    })
    .unwrap();
    e.apply(Edit::SetText {
        paragraph: 1,
        inline: 2,
        text: String::from(" Danach."),
    })
    .unwrap();
    e.apply(Edit::InsertBibliography { after: 1 }).unwrap();
    e.document().clone()
}
pub(crate) fn run() -> i32 {
    let (mut passed, mut failed) = crate::science_checks::run();
    let (p, f) = crate::editor_checks::run();
    passed += p;
    failed += f;
    for (p, f) in [
        super::table_checks::run(),
        super::table_notes_checks::run(),
        super::formula_checks::run(),
        crate::ris_checks::run(),
        crate::bibtex_checks::run(),
        crate::workflow_checks::run(),
        crate::citation_checks::run(),
        crate::merge_checks::run(),
        crate::audit_checks::run(),
        crate::library_checks::run(),
        crate::library_io_checks::run(),
        crate::notes_checks::run(),
        crate::notes_io_checks::run(),
        crate::notes_layout_checks::run(),
        crate::compat_checks::run(),
        crate::ribbon_checks::run(),
        crate::clipboard_codec_checks::run(),
        crate::paragraph_checks::run(),
        crate::story_checks::run(),
        crate::rich_checks::run(),
        crate::story_edit_checks::run(),
        crate::indent_checks::run(),
        crate::fragment_checks::run(),
        crate::view_checks::run(),
        crate::format_transfer_checks::run(),
        crate::style_checks::run(),
        crate::character_checks::run_checks(),
        crate::checks::tab_checks::run(),
    ] {
        passed += p;
        failed += f;
    }
    macro_rules! check {
        ($ok:expr,$name:expr) => {
            if $ok {
                passed += 1;
            } else {
                failed += 1;
                plat::write_stdout(format!("FAIL {}\n", $name).as_bytes());
            }
        };
    }
    let text = "Grüße\r\nαβ 𝄞";
    for enc in [
        Encoding::Utf8,
        Encoding::Utf8Bom,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        check!(
            encoding::decode(&encoding::encode(text, enc), 256) == Ok((String::from(text), enc)),
            "Unicode round trip"
        );
    }
    for b in [&[0xc0, 0xaf][..], &[0xff, 0xfe, 0, 0xd8], &[0xff, 0xfe, 1]] {
        check!(
            encoding::decode(b, 256).is_err(),
            "malformed Unicode rejected"
        );
    }
    check!(encoding::decode(b"abcd", 3).is_err(), "Unicode byte budget");
    check!(
        encoding::decode(&[0xff, 0xfe, 0, 0, 65, 0, 0, 0], 256).is_err(),
        "UTF-32 rejected"
    );
    let a = xml::parse("<a:r xmlns:a=\"urn:test\" a:x=\"&amp;&#x1D11E;\"><a:t>Grüße</a:t></a:r>")
        .unwrap();
    let b = xml::parse("<b:r xmlns:b=\"urn:test\" b:x=\"&amp;𝄞\"><b:t>Grüße</b:t></b:r>").unwrap();
    check!(a == b, "XML namespace identity");
    for bad in [
        "<!DOCTYPE r [<!ENTITY x SYSTEM 'file:///x'>]><r>&x;</r>",
        "<r a='1' a='2'/>",
        "<x:r/>",
        "<r>&#0;</r>",
        "<r>&unknown;</r>",
        "<r><a/></x>",
        "<r/>tail",
        "<r>a<b/>c</r>",
        "<r xmlns:a='x' xmlns:b='x' a:c='1' b:c='2'/>",
    ] {
        check!(
            xml::parse(bad).is_err(),
            "XML rejects unsupported or invalid input"
        );
    }
    let escaped = xml::escape("a\r\n\t<&\"'").unwrap();
    check!(
        xml::parse(&format!("<r>{escaped}</r>")).unwrap().text == "a\r\n\t<&\"'",
        "XML scalar and whitespace preservation"
    );
    check!(
        xml::parse(&format!("{}{}", "<r>".repeat(51), "</r>".repeat(51))).is_err(),
        "XML depth limit"
    );
    let mut e = Editor::new(Document::default()).unwrap();
    let unchanged = e.document().clone();
    check!(
        e.apply(Edit::Cite {
            paragraph: 1,
            source: 999,
            locator: String::new()
        })
        .is_err()
            && *e.document() == unchanged
            && !e.can_undo(),
        "invalid edit is atomic"
    );
    let sid = e
        .apply(Edit::PutSource(Source {
            title: String::from("Test"),
            authors: vec![Person {
                family: String::from("Müller"),
                ..Person::default()
            }],
            year: String::from("2020"),
            ..Source::default()
        }))
        .unwrap();
    e.apply(Edit::Cite {
        paragraph: 1,
        source: sid,
        locator: String::from("7"),
    })
    .unwrap();
    check!(
        e.apply(Edit::RemoveSource(sid)) == Err(doc_core::Error::InUse),
        "cited sources cannot dangle"
    );
    let mut changed = e.document().source(sid).unwrap().clone();
    changed.year = String::from("2026");
    e.apply(Edit::PutSource(changed)).unwrap();
    check!(
        e.document().plain_text().contains("2026"),
        "source changes refresh citation text"
    );
    e.mark_saved();
    check!(!e.dirty(), "saved checkpoint");
    e.undo();
    check!(e.dirty(), "undo across checkpoint is dirty");
    e.redo();
    check!(!e.dirty(), "redo reaches checkpoint");
    e.undo();
    check!(
        e.apply(Edit::RemoveParagraph(999)).is_err() && e.can_redo(),
        "failed edit retains redo"
    );
    let id = e.apply(Edit::AddParagraph { after: 1 }).unwrap();
    e.undo();
    let newer = e.apply(Edit::AddParagraph { after: 1 }).unwrap();
    check!(
        newer > id && !e.can_redo(),
        "identity not reused after divergent undo"
    );
    for i in 0..90 {
        e.apply(Edit::SetText {
            paragraph: 1,
            inline: 0,
            text: format!("{i}"),
        })
        .unwrap();
    }
    let mut steps = 0;
    while e.undo() {
        steps += 1;
    }
    check!(steps == 64, "bounded history");
    let sample = sample();
    let bytes = doc_io::encode_docx(&sample).unwrap();
    check!(
        doc_io::decode_docx(&bytes) == Ok(sample.clone()),
        "DOCX model/source/citation round trip"
    );
    check!(
        doc_io::encode_docx(&sample).unwrap() == bytes,
        "deterministic DOCX output"
    );
    let entries = archive::read(&bytes, archive::Limits::default()).unwrap();
    check!(entries.len() == 8, "OPC package parts");
    for at in [0, 1, 4, 30, bytes.len() / 2, bytes.len() - 1] {
        check!(
            archive::read(&bytes[..at], archive::Limits::default()).is_err(),
            "truncated ZIP rejected"
        );
    }
    let mut corrupt = bytes.clone();
    corrupt[60] ^= 1;
    check!(
        archive::read(&corrupt, archive::Limits::default()).is_err(),
        "ZIP integrity"
    );
    check!(
        archive::read(
            &bytes,
            archive::Limits {
                total_bytes: 10,
                ..Default::default()
            }
        )
        .is_err(),
        "decompression budget"
    );
    check!(
        archive::write(
            &[archive::Entry {
                name: String::from("../x"),
                data: vec![]
            }],
            Default::default()
        )
        .is_err(),
        "traversal names rejected"
    );
    let mut duplicate = entries.clone();
    duplicate.push(entries[0].clone());
    check!(
        archive::write(&duplicate, Default::default()).is_err(),
        "duplicate ZIP names rejected"
    );
    let mut foreign = entries.clone();
    foreign.push(archive::Entry {
        name: String::from("word/vbaProject.bin"),
        data: vec![1, 2, 3],
    });
    let foreign = archive::write(&foreign, Default::default()).unwrap();
    check!(
        matches!(
            doc_io::decode_docx(&foreign),
            Err(doc_io::Error::Unsupported(_))
        ),
        "macro/unknown parts rejected before editing"
    );
    let mut bad = sample.clone();
    bad.paragraphs[0].content.push(Inline::Citation {
        source: 999,
        locator: String::new(),
    });
    check!(
        doc_io::encode_docx(&bad).is_err(),
        "writer rejects invalid graph"
    );
    if let Some(path) = base::Args(plat::args()).value("--fixture") {
        for (path, document) in [
            (
                "build/jfword-rich-pages.docx",
                crate::rich_checks::pages_sample(),
            ),
            ("build/jfword-rich-citations.docx", sample.clone()),
            ("build/jfword-rich-ui.docx", Document::default()),
            ("build/jfword-rich-story.docx", Document::default()),
            ("build/jfword-rich-pages-edited.docx", Document::default()),
        ] {
            check!(
                doc_io::save_docx(path, &document).is_ok(),
                "rich page editor fixture"
            );
        }
        check!(
            plat::write_file("build/jfword-stories.docx", &crate::story_checks::sample()),
            "preserved story editor fixture"
        );
        check!(
            doc_io::save_docx("build/jfword-paragraphs.docx", &Document::default()).is_ok(),
            "paragraph editor fixture"
        );
        check!(
            plat::write_file("build/jfword-compat.docx", &crate::compat_checks::sample()),
            "foreign DOCX fixture"
        );
        let mut notes_ui = Document::default();
        notes_ui.sources = crate::notes_io_checks::sample().sources;
        notes_ui.paragraphs[0].content = vec![Inline::Text(String::from("Haupttext α mit Beleg."))];
        check!(
            doc_io::save_docx("build/jfword-notes-ui.docx", &notes_ui).is_ok(),
            "note editor fixture"
        );
        let mut empty_note = crate::notes_io_checks::sample();
        empty_note.notes[0].paragraphs[0].content.clear();
        empty_note.paragraphs[0].content.clear();
        empty_note.paragraphs[0]
            .content
            .push(Inline::NoteReference(20));
        empty_note.notes.truncate(1);
        check!(
            doc_io::save_docx("build/jfword-empty-note-ui.docx", &empty_note).is_ok(),
            "empty imported note editor fixture"
        );
        check!(doc_io::save_docx(path, &sample).is_ok(), "fixture save");
        check!(
            doc_io::load_docx(path) == Ok(sample),
            "platform file round trip"
        );
    }
    plat::write_stdout(format!("jfWord core: {passed} passed, {failed} failed\n").as_bytes());
    if failed == 0 { 0 } else { 1 }
}
pub(crate) fn probe(path: &str, output: Option<&str>, reject: bool) -> i32 {
    match doc_io::load_docx(path) {
        Ok(d) => {
            if reject {
                plat::write_stdout(b"FAIL unexpected editable document\n");
                return 1;
            }
            if let Some(p) = output {
                if let Err(e) = doc_io::save_docx(p, &d) {
                    plat::write_stdout(format!("FAIL {e:?}\n").as_bytes());
                    return 1;
                }
            }
            plat::write_stdout(b"PASS supported DOCX\n");
            0
        }
        Err(e) => {
            plat::write_stdout(
                format!("{} {e:?}\n", if reject { "PASS rejected" } else { "FAIL" }).as_bytes(),
            );
            if reject { 0 } else { 1 }
        }
    }
}
