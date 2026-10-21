// Rich clipboard transactions: native boundaries, references, identities and atomic history.
use alloc::{format, string::String, vec, vec::Vec};
use doc_core::{
    CitationItem, Document, DocumentFragment, Edit, Editor, Inline, Note, NoteKind, Paragraph,
    ParagraphStyle, Source, TextColor, TextFormat,
};

fn paragraph(id: u64, text: &str, style: ParagraphStyle) -> Paragraph {
    Paragraph {
        id,
        style,
        format: Default::default(),
        content: vec![Inline::Text(String::from(text))],
    }
}

fn formatted() -> Document {
    let mut d = Document::default();
    d.paragraphs[0].content = vec![
        Inline::Text(String::from("Aα")),
        Inline::with_format(
            String::from("B\nC"),
            TextFormat {
                bold: Some(true),
                color: Some(TextColor::Rgb(0x963215)),
                ..Default::default()
            },
        ),
    ];
    let mut middle = paragraph(2, "中", ParagraphStyle::Heading1);
    middle.format.alignment = doc_core::Alignment::Right;
    middle.format.first_line = 240;
    d.paragraphs.push(middle);
    d.paragraphs
        .push(paragraph(3, "ΩZ", ParagraphStyle::Normal));
    d
}

fn scientific() -> Document {
    let mut d = Document::default();
    d.sources = vec![
        Source {
            id: 10,
            title: String::from("First source"),
            year: String::from("1843"),
            ..Default::default()
        },
        Source {
            id: 11,
            title: String::from("Second source"),
            ..Default::default()
        },
        Source {
            id: 12,
            title: String::from("Unused source"),
            ..Default::default()
        },
    ];
    d.paragraphs[0].content = vec![
        Inline::Text(String::from("Aα")),
        Inline::CitationCluster(vec![
            CitationItem {
                source: 10,
                ..Default::default()
            },
            CitationItem {
                source: 11,
                ..Default::default()
            },
        ]),
        Inline::Text(String::from("Z")),
        Inline::NoteReference(20),
        Inline::Text(String::from("N")),
        Inline::NoteReference(21),
    ];
    d.notes = vec![
        Note {
            id: 20,
            kind: NoteKind::Footnote,
            paragraphs: vec![
                Paragraph {
                    content: vec![
                        Inline::Text(String::from("Fußnote ")),
                        Inline::Citation {
                            source: 11,
                            locator: String::from("2"),
                        },
                    ],
                    ..paragraph(30, "", ParagraphStyle::Normal)
                },
                paragraph(31, "Mehr\nText", ParagraphStyle::Heading2),
            ],
        },
        Note {
            id: 21,
            kind: NoteKind::Endnote,
            paragraphs: vec![paragraph(32, "Endnote", ParagraphStyle::Normal)],
        },
    ];
    d.paragraphs.push(Paragraph {
        content: vec![Inline::Bibliography],
        ..paragraph(2, "", ParagraphStyle::Normal)
    });
    d
}

fn appearance(d: &Document) -> Vec<(usize, char, doc_core::ResolvedFormat)> {
    let mut at = 0;
    let mut out = Vec::new();
    for (index, p) in d.paragraphs.iter().enumerate() {
        if index > 0 {
            at += 1;
        }
        for i in &p.content {
            if let Some(text) = i.text() {
                for ch in text.chars() {
                    out.push((at, ch, i.format().resolve(p.style)));
                    at += ch.len_utf8();
                }
            }
        }
    }
    out
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
    let d = formatted();
    let story = d.story_projection(1).unwrap();
    for start in 0..=story.text.len() {
        for end in start..=story.text.len() {
            let copied = d.copy_story_range(2, start, end);
            if !story.text.is_char_boundary(start) || !story.text.is_char_boundary(end) {
                check!(copied.is_err(), "rich copy rejects non-scalar endpoints");
                continue;
            }
            let fragment = copied.unwrap();
            check!(
                fragment
                    .document
                    .story_projection(fragment.document.paragraphs[0].id)
                    .unwrap()
                    .text
                    == story.text[start..end],
                "rich copy projects exact Unicode range including soft breaks"
            );
            let paragraph_count = fragment.document.paragraphs.len();
            let mut target = Document::default();
            target.paragraphs[0].content = vec![Inline::Text(String::from("xz"))];
            let mut e = Editor::new(target.clone()).unwrap();
            let result = e.replace_story_fragment(1, 1, 1, fragment.clone());
            check!(
                result.is_ok(),
                "rich fragment insertion accepts every scalar range"
            );
            if let Ok((id, caret)) = result {
                let actual = e.document().story_projection(1).unwrap();
                check!(
                    actual.text == format!("x{}z", &story.text[start..end]),
                    "rich fragment preserves copied text and target prefix/suffix"
                );
                check!(
                    e.document().paragraphs.len() == paragraph_count,
                    "rich clipboard preserves hard versus soft breaks"
                );
                check!(
                    actual.offset(id, caret) == Some(1 + end - start),
                    "rich fragment returns actual inserted trailing caret"
                );
                check!(
                    doc_io::decode_docx(&doc_io::encode_docx(e.document()).unwrap())
                        == Ok(e.document().clone()),
                    "rich clipboard result survives native DOCX roundtrip"
                );
                if *e.document() != target {
                    let after = e.document().clone();
                    check!(
                        e.undo() && *e.document() == target && e.redo() && *e.document() == after,
                        "rich clipboard is one reversible history transaction"
                    );
                } else {
                    check!(!e.can_undo(), "empty rich paste does not invent history");
                }
            }
        }
    }
    let copied = d.copy_story_range(1, 0, story.text.len()).unwrap();
    let originals = appearance(&d);
    for from in 0..=story.text.len() {
        for to in from..=story.text.len() {
            if !story.text.is_char_boundary(from) || !story.text.is_char_boundary(to) {
                continue;
            }
            for (a, b) in [(0, story.text.len()), (4, 5), (6, 7), (7, 10)] {
                let fragment = d.copy_story_range(1, a, b).unwrap();
                let mut expected: Vec<_> = originals
                    .iter()
                    .filter(|(offset, _, _)| *offset < from)
                    .map(|(_, ch, format)| (*ch, *format))
                    .collect();
                expected.extend(
                    originals
                        .iter()
                        .filter(|(offset, _, _)| a <= *offset && *offset < b)
                        .map(|(_, ch, format)| (*ch, *format)),
                );
                expected.extend(
                    originals
                        .iter()
                        .filter(|(offset, _, _)| *offset >= to)
                        .map(|(_, ch, format)| (*ch, *format)),
                );
                let inserted_paragraphs = fragment.document.paragraphs.len();
                let mut e = Editor::new(d.clone()).unwrap();
                let (id, caret) = e.replace_story_fragment(2, from, to, fragment).unwrap();
                check!(
                    e.document().story_projection(1).unwrap().text
                        == format!(
                            "{}{}{}",
                            &story.text[..from],
                            &story.text[a..b],
                            &story.text[to..]
                        ),
                    "rich replacement matrix retains selected source and surviving target text"
                );
                check!(
                    appearance(e.document())
                        .iter()
                        .map(|(_, ch, format)| (*ch, *format))
                        .collect::<Vec<_>>()
                        == expected,
                    "rich replacement matrix retains each scalar's original resolved appearance"
                );
                let removed_marks = story
                    .paragraphs
                    .iter()
                    .skip(1)
                    .filter(|p| from < p.start && p.start <= to)
                    .count();
                check!(
                    e.document().paragraphs.len()
                        == d.paragraphs.len() - removed_marks + inserted_paragraphs - 1,
                    "rich replacement matrix retains the native paragraph-boundary count"
                );
                check!(e.document().story_projection(1).unwrap().offset(id, caret) == Some(from + b - a), "rich replacement matrix caret follows inserted content across target paragraphs");
            }
        }
    }
    let mut e = Editor::new(Document::default()).unwrap();
    e.replace_story_fragment(1, 0, 0, copied).unwrap();
    check!(
        e.document()
            .paragraphs
            .iter()
            .zip(&d.paragraphs)
            .all(|(a, b)| a.content == b.content && a.style == b.style && a.format == b.format),
        "full rich paragraphs retain direct and inherited formatting"
    );
    let heading = d
        .copy_story_range(
            2,
            story.paragraphs[1].start,
            story.paragraphs[1].start + "中".len(),
        )
        .unwrap();
    let mut target = Document::default();
    target.paragraphs[0].content = vec![Inline::Text(String::from("xy"))];
    let mut e = Editor::new(target).unwrap();
    e.replace_story_fragment(1, 1, 1, heading).unwrap();
    check!(
        e.document().paragraphs[0]
            .content
            .iter()
            .find(|i| i.text() == Some("中"))
            .is_some_and(|i| i.format().resolve(ParagraphStyle::Normal)
                == TextFormat::default().resolve(ParagraphStyle::Heading1)),
        "partial paragraph paste preserves source inherited appearance"
    );

    let scientific = scientific();
    check!(
        scientific.validate().is_ok(),
        "scientific clipboard fixture is valid"
    );
    let text = scientific.story_projection(1).unwrap().text;
    let fragment = scientific.copy_story_range(1, 0, text.len()).unwrap();
    check!(
        fragment.document.sources.len() == 2 && fragment.document.notes.len() == 2,
        "clipboard carries cited sources and complete foot/endnote bodies only"
    );
    let mut target = Document::default();
    let mut reused = scientific.sources[0].clone();
    reused.id = 99;
    target.sources.push(reused);
    target.sources.push(Source {
        id: 10,
        title: String::from("Existing different source with conflicting ID"),
        ..Default::default()
    });
    let mut e = Editor::new(target.clone()).unwrap();
    check!(
        e.replace_story_fragment(1, 0, 0, fragment.clone()).is_ok(),
        "cross-document paste remaps colliding identities"
    );
    check!(
        e.document().sources.len() == 3
            && e.document()
                .sources
                .iter()
                .filter(|s| s.title == "First source")
                .count()
                == 1,
        "rich paste reuses only exact source metadata duplicates"
    );
    check!(e.document().paragraphs[0].content.iter().any(|i| matches!(i, Inline::CitationCluster(items) if items[0].source == 99 && items[1].source > 99)), "multi-source citation references are remapped");
    check!(
        e.document()
            .notes
            .iter()
            .all(|n| n.id > 99 && n.paragraphs.iter().all(|p| p.id > 99))
            && e.document().validate().is_ok(),
        "notes and their paragraphs receive fresh globally unique identities"
    );
    check!(
        doc_io::decode_docx(&doc_io::encode_docx(e.document()).unwrap())
            == Ok(e.document().clone()),
        "rich citations notes bibliography survive DOCX roundtrip"
    );
    let pasted = e.document().clone();
    check!(
        e.undo() && *e.document() == target && e.redo() && *e.document() == pasted,
        "rich sources and note imports undo together with text"
    );
    let all = e.document().story_projection(1).unwrap().text.len();
    check!(
        e.replace_story_fragment(1, 0, all, DocumentFragment::default())
            .is_ok()
            && e.document().notes.is_empty()
            && e.document().sources.len() == 3,
        "cut of complete fields removes note bodies but retains source library records"
    );
    check!(
        e.undo() && *e.document() == pasted,
        "cut restores exact note/source identities on undo"
    );

    let projection = scientific.text_projection(1).unwrap();
    let field = &projection.spans[1];
    check!(
        scientific
            .copy_story_range(1, field.start + 1, field.end)
            .is_err(),
        "partial generated field cannot become a rich clipboard fragment"
    );
    let mut e = Editor::new(scientific.clone()).unwrap();
    e.apply(Edit::SetStyle {
        paragraph: 1,
        style: ParagraphStyle::Heading1,
    })
    .unwrap();
    e.undo();
    let state = e.state_id();
    check!(
        e.replace_story_fragment(1, field.start + 1, field.end, DocumentFragment::default())
            .is_err()
            && *e.document() == scientific
            && e.state_id() == state
            && e.can_redo(),
        "partial-field cut is atomic and preserves redo"
    );
    let field_copy = scientific
        .copy_story_range(1, field.start, field.end)
        .unwrap();
    check!(
        field_copy.document.paragraphs[0].content.len() == 1
            && matches!(&field_copy.document.paragraphs[0].content[0], Inline::CitationCluster(items) if items.len() == 2),
        "copying exact field keeps structured multi-source citation"
    );
    check!(
        e.replace_story_fragment(1, field.start, field.end, DocumentFragment::default())
            .is_ok()
            && !e.document().paragraphs[0]
                .content
                .iter()
                .any(|i| matches!(i, Inline::CitationCluster(_))),
        "complete citation is removable through rich range"
    );
    let mut e = Editor::new(scientific.clone()).unwrap();
    check!(
        e.replace_story_fragment(30, 0, 0, fragment).is_err()
            && *e.document() == scientific
            && !e.can_undo(),
        "rich paste rejects nested note anchors and bibliography atomically"
    );
    check!(
        e.replace_story_fragment(30, 0, 0, field_copy).is_ok() && e.document().validate().is_ok(),
        "citation fields can be pasted into note text"
    );

    // Existing note markers grow from one to two digits after insertion; returned caret
    // must be measured from the updated suffix, not from the old clipboard display.
    let mut target = Document::default();
    target.paragraphs[0].content.clear();
    for index in 0..9 {
        target.paragraphs[0]
            .content
            .push(Inline::NoteReference(10 + index));
        target.notes.push(Note {
            id: 10 + index,
            kind: NoteKind::Footnote,
            paragraphs: vec![paragraph(30 + index, "n", ParagraphStyle::Normal)],
        });
    }
    let note_span = &projection.spans[3];
    let note_copy = scientific
        .copy_story_range(1, note_span.start, note_span.end)
        .unwrap();
    let mut e = Editor::new(target).unwrap();
    check!(
        e.replace_story_fragment(1, 0, 0, note_copy) == Ok((1, 3))
            && e.document()
                .paragraph_text(&e.document().paragraphs[0])
                .ends_with("[10]"),
        "rich paste caret accounts for renumbered marker widths"
    );
    let mut malformed = DocumentFragment::default();
    malformed.document.paragraphs[0]
        .content
        .push(Inline::Citation {
            source: 999,
            locator: String::new(),
        });
    let before = e.document().clone();
    let state = e.state_id();
    check!(
        e.replace_story_fragment(1, 0, 0, malformed).is_err()
            && *e.document() == before
            && e.state_id() == state,
        "malformed clipboard references cannot mutate destination"
    );
    let mut oversized = DocumentFragment::default();
    for id in 2..=2049 {
        oversized
            .document
            .paragraphs
            .push(paragraph(id, "", ParagraphStyle::Normal));
    }
    check!(
        e.replace_story_fragment(1, 0, 0, oversized) == Err(doc_core::Error::Limit)
            && *e.document() == before,
        "rich clipboard enforces native paragraph resource limits"
    );
    let mut bad_format = DocumentFragment::default();
    bad_format.document.paragraphs[0].content = vec![Inline::Text(String::from("W"))];
    bad_format.document.page.width = 20000;
    bad_format.document.paragraphs[0].format.left = 12000;
    bad_format.document.sources = vec![Source {
        id: 500,
        title: String::from("Transient source"),
        ..Default::default()
    }];
    check!(
        bad_format.document.validate().is_ok(),
        "clipboard can have source page setup wider than target"
    );
    let mut e = Editor::new(Document::default()).unwrap();
    check!(
        e.replace_story_fragment(1, 0, 0, bad_format).is_err()
            && *e.document() == Document::default()
            && !e.can_undo(),
        "paste incompatible paragraph layout fails after staging without side effects"
    );
    check!(
        e.apply(Edit::PutSource(Source {
            title: String::from("Next source"),
            ..Default::default()
        })) == Ok(2),
        "failed rich paste does not consume stable identity allocator"
    );
    let mut empty_bib = Document::default();
    empty_bib.paragraphs[0].content = vec![Inline::Bibliography];
    let mut e = Editor::new(empty_bib.clone()).unwrap();
    check!(
        e.apply(Edit::RemoveField {
            paragraph: 1,
            inline: 0
        })
        .is_ok()
            && e.document().paragraphs[0].content == vec![Inline::Text(String::new())],
        "zero-width bibliography is removable by native field identity"
    );
    check!(
        e.undo() && *e.document() == empty_bib,
        "removing zero-width field has one reversible history step"
    );
    let mut e = Editor::new(scientific.clone()).unwrap();
    check!(
        e.apply(Edit::RemoveField {
            paragraph: 1,
            inline: 3
        })
        .is_ok()
            && e.document().note(20).is_none()
            && e.document().validate().is_ok(),
        "native field removal removes the anchored note body atomically"
    );
    let before = e.document().clone();
    check!(
        e.apply(Edit::RemoveField {
            paragraph: 1,
            inline: 0
        })
        .is_err()
            && *e.document() == before,
        "field removal never accepts editable text"
    );
    let field = scientific
        .copy_story_range(1, field.start, field.end)
        .unwrap();
    let mut e = Editor::new(Document::default()).unwrap();
    let (id, caret) = e.replace_story_fragment(1, 0, 0, field).unwrap();
    check!(
        e.document()
            .text_projection(id)
            .unwrap()
            .position(0)
            .is_ok()
            && e.document()
                .text_projection(id)
                .unwrap()
                .position(caret)
                .is_ok(),
        "field-only paste keeps editable carets before and after the field"
    );
    check!(
        e.apply(Edit::ReplaceStoryText {
            paragraph: id,
            start: caret,
            end: caret,
            text: String::from("weiter"),
            paragraph_breaks: true
        })
        .is_ok(),
        "typing can continue immediately after a pasted citation"
    );
    let mut styled = Document::default();
    styled.paragraphs[0].style = ParagraphStyle::Heading1;
    styled.paragraphs[0].content = vec![Inline::with_format(
        String::from("abcdef"),
        TextFormat {
            italic: Some(true),
            ..Default::default()
        },
    )];
    let mut e = Editor::new(styled.clone()).unwrap();
    e.replace_story_fragment(1, 2, 4, DocumentFragment::default())
        .unwrap();
    check!(
        e.document().paragraphs[0].content.len() == 1
            && e.document().paragraphs[0].content[0].format()
                == styled.paragraphs[0].content[0].format(),
        "rich cut does not introduce an unformatted insertion seam"
    );
    e.replace_story_fragment(1, 0, 4, DocumentFragment::default())
        .unwrap();
    check!(
        e.document().paragraphs[0].style == ParagraphStyle::Heading1
            && e.document().paragraphs[0].content[0].format()
                == styled.paragraphs[0].content[0].format(),
        "cutting whole heading retains insertion style and direct formatting"
    );
    let mut live = Document::default();
    live.sources.push(Source {
        id: 2,
        title: String::from("Übersicht mit langem Quellentitel"),
        ..Default::default()
    });
    live.paragraphs[0].content = vec![
        Inline::Bibliography,
        Inline::Text(String::from("\nIntro ")),
        Inline::Citation {
            source: 2,
            locator: String::new(),
        },
        Inline::Text(String::from(" Ende")),
        Inline::Bibliography,
    ];
    let mut e = Editor::new(live.clone()).unwrap();
    check!(
        e.remove_field(1, 2) == Ok((1, "\nIntro ".len()))
            && e.document().text_projection(1).unwrap().text == "\nIntro  Ende",
        "field removal caret follows bibliography changes before and after the removed citation"
    );
    check!(
        e.undo() && *e.document() == live && e.redo(),
        "dynamic field removal is one exact reversible transaction"
    );
    let after = e.document().clone();
    let state = e.state_id();
    check!(
        e.remove_field(1, 1).is_err() && *e.document() == after && e.state_id() == state,
        "field caret convenience rejects text without changing document or history"
    );
    let mut before = scientific.clone();
    before.paragraphs.insert(
        0,
        Paragraph {
            content: vec![Inline::Bibliography],
            ..paragraph(100, "", ParagraphStyle::Normal)
        },
    );
    let mut e = Editor::new(before).unwrap();
    let (id, caret) = e.remove_field(1, 1).unwrap();
    check!(
        caret == "Aα".len()
            && e.document().story_projection(id).unwrap().offset(id, caret)
                == Some(
                    e.document()
                        .paragraph_text(&e.document().paragraphs[0])
                        .len()
                        + 1
                        + "Aα".len()
                ),
        "native field caret remaps correctly after an earlier bibliography paragraph changes"
    );
    (passed, failed)
}
