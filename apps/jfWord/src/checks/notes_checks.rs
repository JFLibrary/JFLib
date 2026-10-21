// Native contracts for scientific notes, citation scope, identity and atomic history.
use alloc::{format, string::String, vec};
use doc_core::{
    CitationItem, CitationLocation, Document, Edit, Editor, Error, Inline, Note, NoteKind,
    Paragraph, ParagraphFormat, ParagraphStyle, SearchOptions, Source, SourceMerge, TextFormat,
};

fn paragraph(id: u64, content: alloc::vec::Vec<Inline>) -> Paragraph {
    Paragraph {
        id,
        style: ParagraphStyle::Normal,
        format: ParagraphFormat::default(),
        content,
    }
}
fn text(value: &str) -> Inline {
    Inline::Text(String::from(value))
}
fn cite(source: u64) -> Inline {
    Inline::Citation {
        source,
        locator: String::new(),
    }
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
    let mut d = Document::default();
    d.sources = vec![
        Source {
            id: 20,
            title: String::from("Hauptquelle"),
            ..Default::default()
        },
        Source {
            id: 21,
            title: String::from("Anmerkungsquelle"),
            ..Default::default()
        },
    ];
    d.paragraphs = vec![paragraph(
        1,
        vec![
            text("Haupttext"),
            Inline::NoteReference(7),
            text(" weiter "),
            Inline::NoteReference(9),
            Inline::NoteReference(5),
            cite(20),
        ],
    )];
    d.notes = vec![
        Note {
            id: 5,
            kind: NoteKind::Footnote,
            paragraphs: vec![paragraph(6, vec![text("Zweite Fußnote")])],
        },
        Note {
            id: 9,
            kind: NoteKind::Endnote,
            paragraphs: vec![paragraph(10, vec![text("Endnote "), cite(20)])],
        },
        Note {
            id: 7,
            kind: NoteKind::Footnote,
            paragraphs: vec![paragraph(8, vec![text("Erste Fußnote "), cite(21)])],
        },
    ];
    check!(
        d.validate().is_ok(),
        "notes accept unordered storage and globally distinct identities"
    );
    check!(
        d.note_number(7) == Some(1)
            && d.note_number(5) == Some(2)
            && d.note_number(9) == Some(1)
            && d.note_number(777).is_none()
            && d.ordered_notes(NoteKind::Footnote)
                .iter()
                .map(|n| n.id)
                .collect::<alloc::vec::Vec<_>>()
                == vec![7, 5],
        "notes number independently by anchor order instead of identity or storage"
    );
    check!(
        d.note_location(7)
            == Some(CitationLocation {
                paragraph: 1,
                inline: 1
            })
            && d.note_for_paragraph(8).map(|n| n.id) == Some(7)
            && d.note_for_paragraph(1).is_none()
            && d.paragraph(10).is_some()
            && d.all_paragraphs().count() == 4,
        "note and paragraph lookup keeps body and main locations distinct"
    );
    check!(
        d.cited_source_ids() == vec![21, 20]
            && d.bibliography()
                == vec![
                    d.source(21).unwrap().bibliography_text(),
                    d.source(20).unwrap().bibliography_text()
                ]
            && d.source_uses(20)
                == vec![
                    CitationLocation {
                        paragraph: 10,
                        inline: 1
                    },
                    CitationLocation {
                        paragraph: 1,
                        inline: 5
                    }
                ],
        "note citations participate at their anchor in bibliography and usage order"
    );
    check!(d.plain_text() == "Haupttext[1] weiter [E1][2](Hauptquelle)\n[1] Erste Fußnote (Anmerkungsquelle)\n[E1] Endnote (Hauptquelle)\n[2] Zweite Fußnote",
        "plain text contains each note body once and distinguishes both numbering sequences");
    let mut search_order = d.clone();
    search_order.paragraphs[0].content[0] = text("note Haupttext");
    search_order.paragraphs.push(paragraph(
        30,
        vec![text("note danach"), Inline::NoteReference(31)],
    ));
    search_order.notes.insert(
        0,
        Note {
            id: 31,
            kind: NoteKind::Endnote,
            paragraphs: vec![paragraph(32, vec![text("note zuletzt")])],
        },
    );
    let expected_order = vec![1, 8, 10, 6, 30, 32];
    check!(
        search_order
            .find_text("note", SearchOptions::default())
            .unwrap()
            .iter()
            .map(|found| found.paragraph)
            .collect::<alloc::vec::Vec<_>>()
            == expected_order,
        "search follows each main paragraph with note bodies in anchor order, not storage order"
    );
    let mut search_editor = Editor::new(search_order.clone()).unwrap();
    check!(
        search_editor.apply(Edit::ReplaceAll {
            query: String::from("note"),
            replacement: String::from("Kommentar"),
            options: SearchOptions::default()
        }) == Ok(6)
            && search_editor
                .document()
                .find_text("note", SearchOptions::default())
                .unwrap()
                .is_empty()
            && search_editor
                .document()
                .find_text("Kommentar", SearchOptions::default())
                .unwrap()
                .iter()
                .map(|found| found.paragraph)
                .collect::<alloc::vec::Vec<_>>()
                == expected_order
            && search_editor.undo()
            && search_editor.document() == &search_order,
        "replace all handles interleaved note search order atomically with undo"
    );
    let mut e = Editor::new(d.clone()).unwrap();
    check!(
        e.apply(Edit::RemoveSource(21)) == Err(Error::InUse) && !e.dirty(),
        "citation in a note protects its source against deletion"
    );
    check!(
        e.apply(Edit::SetNoteKind {
            note: 7,
            kind: NoteKind::Endnote
        }) == Ok(7)
            && e.document().note_number(7) == Some(1)
            && e.document().note_number(9) == Some(2)
            && e.document().note_number(5) == Some(1)
            && e.undo()
            && e.document() == &d,
        "note conversion renumbers both sequences and is one undo transaction"
    );
    let revision = e.revision();
    check!(
        e.apply(Edit::SetNoteKind {
            note: 7,
            kind: NoteKind::Footnote
        }) == Ok(7)
            && e.revision() == revision
            && e.can_redo(),
        "unchanged note kind preserves redo and revision"
    );
    check!(
        e.apply(Edit::MergeSources(SourceMerge {
            keep: 20,
            remove: 21,
            incoming: vec![]
        })) == Ok(20)
            && e.document().source_uses(20).len() == 3
            && e.document().source(21).is_none()
            && e.undo()
            && e.document() == &d,
        "source reconciliation remaps citations inside notes with full undo"
    );
    check!(
        e.apply(Edit::SetText {
            paragraph: 8,
            inline: 0,
            text: String::from("Änderung ")
        }) == Ok(8)
            && e.document()
                .find_text("Änderung", SearchOptions::default())
                .unwrap()[0]
                .paragraph
                == 8
            && e.apply(Edit::ReplaceAll {
                query: String::from("Änderung"),
                replacement: String::from("Korrektur"),
                options: SearchOptions::default()
            }) == Ok(1)
            && e.document().paragraph(8).unwrap().content[0].text() == Some("Korrektur"),
        "note text supports generic editing, search and atomic replacement"
    );
    check!(
        e.apply(Edit::FormatText {
            paragraph: 8,
            inline: 0,
            start: 0,
            end: 9,
            format: TextFormat {
                italic: Some(true),
                ..Default::default()
            }
        }) == Ok(8)
            && e.apply(Edit::SetStyle {
                paragraph: 8,
                style: ParagraphStyle::Heading2
            }) == Ok(8)
            && e.document().paragraph(8).unwrap().content[0]
                .format()
                .italic
                == Some(true),
        "note paragraphs support character and paragraph styles"
    );
    check!(
        e.apply(Edit::UpdateCitation {
            paragraph: 8,
            inline: 2,
            items: vec![CitationItem {
                source: 20,
                locator: String::from("23"),
                ..Default::default()
            }]
        }) == Ok(8)
            && e.apply(Edit::ConvertCitationToText {
                paragraph: 8,
                inline: 2
            }) == Ok(8)
            && e.document().paragraph(8).unwrap().content[2].text() == Some("(Hauptquelle, S. 23)"),
        "note citations can update and convert through generic citation operations"
    );
    let new_body = e.apply(Edit::AddParagraph { after: 8 }).unwrap();
    check!(
        new_body > 21
            && e.document().note(7).unwrap().paragraphs.len() == 2
            && e.document().paragraphs.len() == 1
            && e.apply(Edit::RemoveParagraph(8)) == Ok(8)
            && e.apply(Edit::RemoveParagraph(new_body)) == Err(Error::Invalid),
        "paragraph insertion stays within note and cannot delete its final paragraph"
    );
    check!(
        e.apply(Edit::InsertBibliography { after: new_body })
            .is_err(),
        "bibliography insertion cannot create a field in a note body"
    );

    let format = TextFormat {
        bold: Some(true),
        size_half_points: Some(25),
        ..Default::default()
    };
    let mut original = Document::default();
    original.paragraphs[0].content = vec![Inline::with_format(String::from("Äpfel Ω"), format)];
    let mut insert = Editor::new(original.clone()).unwrap();
    check!(
        insert.apply(Edit::InsertNote {
            paragraph: 1,
            inline: 0,
            offset: 1,
            kind: NoteKind::Footnote,
            text: String::from("Fehler")
        }) == Err(Error::Invalid)
            && insert.document() == &original
            && !insert.dirty()
            && !insert.can_undo(),
        "note insertion rejects a split UTF-8 character atomically"
    );
    let note = insert
        .apply(Edit::InsertNote {
            paragraph: 1,
            inline: 0,
            offset: 6,
            kind: NoteKind::Footnote,
            text: String::from("Kommentar Ω"),
        })
        .unwrap();
    let body = insert.document().note(note).unwrap().paragraphs[0].id;
    check!(
        note == 2
            && body == 3
            && insert.document().paragraphs[0].content
                == vec![
                    Inline::with_format(String::from("Äpfel"), format),
                    Inline::NoteReference(note),
                    Inline::with_format(String::from(" Ω"), format)
                ],
        "note insertion splits formatted text exactly and allocates two stable identities"
    );
    let inserted = insert.document().clone();
    check!(
        insert.apply(Edit::InsertNote {
            paragraph: body,
            inline: 0,
            offset: 0,
            kind: NoteKind::Endnote,
            text: String::new()
        }) == Err(Error::Invalid)
            && insert.document() == &inserted,
        "note editor forbids nesting without changing history or data"
    );
    check!(
        insert.apply(Edit::RemoveNote(note)) == Ok(note)
            && insert.document().notes.is_empty()
            && insert.document().plain_text() == "Äpfel Ω"
            && insert.undo()
            && insert.document() == &inserted,
        "note removal removes anchor and body together and restores both with undo"
    );
    check!(
        insert.undo() && insert.document() == &original,
        "note insertion is one undo step"
    );
    let later = insert
        .apply(Edit::InsertNote {
            paragraph: 1,
            inline: 0,
            offset: 9,
            kind: NoteKind::Endnote,
            text: String::new(),
        })
        .unwrap();
    check!(
        later > body
            && insert.document().paragraphs[0]
                .content
                .last()
                .unwrap()
                .text()
                == Some("")
            && !insert.can_redo(),
        "new note IDs never reuse undone IDs and end insertion retains editable tail"
    );

    let mut anchor_only = d.clone();
    anchor_only.paragraphs[0].content = vec![Inline::NoteReference(7)];
    anchor_only.notes.retain(|note| note.id == 7);
    let mut sole_anchor = Editor::new(anchor_only.clone()).unwrap();
    check!(
        sole_anchor.apply(Edit::RemoveNote(7)) == Ok(7)
            && sole_anchor.document().paragraphs[0].content == vec![text("")]
            && sole_anchor.document().source(21).is_some()
            && sole_anchor.document().source_uses(21).is_empty()
            && sole_anchor.undo()
            && sole_anchor.document() == &anchor_only,
        "removing a sole note anchor keeps an editable empty paragraph and unreferenced sources"
    );
    let mut citation_only = anchor_only.clone();
    citation_only.notes[0].paragraphs[0].content = vec![cite(21)];
    let mut sole_citation = Editor::new(citation_only.clone()).unwrap();
    check!(sole_citation.apply(Edit::RemoveCitation { paragraph: 8, inline: 0 }) == Ok(8)
        && sole_citation.document().paragraph(8).unwrap().content == vec![text("")]
        && sole_citation.document().note(7).is_some()
        && sole_citation.document().source(21).is_some()
        && sole_citation.document().source_uses(21).is_empty()
        && sole_citation.undo() && sole_citation.document() == &citation_only
        && sole_citation.redo()
        && sole_citation.apply(Edit::SetText { paragraph: 8, inline: 0,
            text: String::from("Neue Anmerkung") }) == Ok(8),
        "removing a citation-only note field leaves editable text, preserves note and source, and undoes");
    let mut cascade = Editor::new(d.clone()).unwrap();
    let retained = cascade.apply(Edit::AddParagraph { after: 1 }).unwrap();
    let before = cascade.document().clone();
    check!(
        cascade.apply(Edit::RemoveParagraph(1)) == Ok(1)
            && cascade.document().notes.is_empty()
            && cascade.document().paragraphs[0].id == retained
            && cascade.document().source_uses(20).is_empty()
            && cascade.undo()
            && cascade.document() == &before,
        "deleting a main paragraph cascades all its notes and restores citations on undo"
    );

    let mut bad = d.clone();
    bad.paragraphs[0].content.push(Inline::NoteReference(777));
    check!(
        bad.validate() == Err(Error::Missing),
        "missing note body is rejected"
    );
    let mut bad = d.clone();
    bad.paragraphs[0].content.push(Inline::NoteReference(7));
    check!(
        bad.validate() == Err(Error::Invalid),
        "duplicate note anchor is rejected"
    );
    let mut bad = d.clone();
    bad.paragraphs[0]
        .content
        .retain(|n| !matches!(n, Inline::NoteReference(7)));
    check!(
        bad.validate() == Err(Error::Missing),
        "orphaned note body is rejected"
    );
    let mut bad = d.clone();
    bad.notes[0].paragraphs.clear();
    check!(
        bad.validate() == Err(Error::Invalid),
        "empty note body paragraph list is rejected"
    );
    let mut bad = d.clone();
    bad.notes[0].paragraphs[0]
        .content
        .push(Inline::NoteReference(7));
    check!(
        bad.validate() == Err(Error::Invalid),
        "nested note references are rejected at codec trust boundaries"
    );
    let mut bad = d.clone();
    bad.notes[0].paragraphs[0]
        .content
        .push(Inline::Bibliography);
    check!(
        bad.validate() == Err(Error::Invalid),
        "bibliography fields inside notes are rejected"
    );
    let mut bad = d.clone();
    bad.notes[0].paragraphs[0].id = 20;
    check!(
        bad.validate() == Err(Error::Invalid),
        "note paragraph IDs cannot collide with source IDs"
    );
    let mut bad = d.clone();
    bad.notes[0].id = 1;
    check!(
        bad.validate() == Err(Error::Invalid),
        "note IDs cannot collide with main paragraph IDs"
    );
    let mut bad = d.clone();
    bad.notes[0].paragraphs[0].content.push(cite(999));
    check!(
        bad.validate() == Err(Error::Missing),
        "dangling note citations are rejected"
    );
    let mut bad = d.clone();
    bad.notes[0].paragraphs[0].content = vec![text("\0")];
    check!(
        bad.validate() == Err(Error::Invalid),
        "note text obeys XML character invariants"
    );
    let mut bad = d.clone();
    bad.notes = vec![bad.notes[0].clone(); 1025];
    check!(
        bad.validate() == Err(Error::Limit),
        "note count resource cap applies before traversal"
    );
    let mut bad = d.clone();
    bad.notes[0].paragraphs = vec![bad.notes[0].paragraphs[0].clone(); 2048];
    check!(
        bad.validate() == Err(Error::Limit),
        "paragraph budget counts main and every note body"
    );
    let mut bad = d.clone();
    bad.notes[0].paragraphs[0].content = vec![Inline::Text("x".repeat(2 << 20))];
    check!(
        bad.validate() == Err(Error::Limit),
        "retained byte budget includes note text"
    );
    let mut bad = d.clone();
    bad.sources[0].corporate = "x".repeat(4096);
    bad.notes[0].paragraphs[0].content = vec![cite(20); 1100];
    check!(
        bad.byte_size() < 2 << 20 && bad.validate() == Err(Error::Limit),
        "expanded text budget includes repeated citations inside notes"
    );
    let mut boundary = Document::default();
    boundary.paragraphs[0].content.clear();
    for index in 0..1024 {
        let id = index * 2 + 2;
        boundary.paragraphs[0]
            .content
            .push(Inline::NoteReference(id));
        boundary.notes.push(Note {
            id,
            kind: if index % 2 == 0 {
                NoteKind::Footnote
            } else {
                NoteKind::Endnote
            },
            paragraphs: vec![paragraph(id + 1, vec![text("")])],
        });
    }
    check!(
        boundary.validate().is_ok()
            && boundary.note_number(2046) == Some(512)
            && boundary.note_number(2048) == Some(512),
        "exact maximum note count retains independent numbering"
    );
    for index in 0..1023 {
        boundary.notes[0]
            .paragraphs
            .push(paragraph(3000 + index, vec![text("")]));
    }
    check!(
        boundary.all_paragraphs().count() == 2048 && boundary.validate().is_ok(),
        "combined main and note paragraph budget accepts its exact boundary"
    );
    for note_body in [false, true] {
        let mut empty = Document::default();
        let paragraph_id = if note_body {
            empty.paragraphs[0].content = vec![Inline::NoteReference(2)];
            empty.notes.push(Note {
                id: 2,
                kind: NoteKind::Footnote,
                paragraphs: vec![paragraph(3, vec![])],
            });
            3
        } else {
            empty.paragraphs[0].content.clear();
            1
        };
        let mut editor = Editor::new(empty.clone()).unwrap();
        check!(
            editor.document() == &empty
                && editor.apply(Edit::SetText {
                    paragraph: paragraph_id,
                    inline: 0,
                    text: String::new()
                }) == Ok(paragraph_id)
                && editor.apply(Edit::SetText {
                    paragraph: paragraph_id,
                    inline: 1,
                    text: String::from("Ungültig")
                }) == Err(Error::Missing)
                && editor.apply(Edit::SetText {
                    paragraph: paragraph_id,
                    inline: 0,
                    text: String::from("\0")
                }) == Err(Error::Invalid)
                && editor.document() == &empty
                && !editor.dirty()
                && !editor.can_undo()
                && editor.revision() == 0,
            if note_body {
                "empty imported note body stays unchanged for no-op or invalid virtual-run writes"
            } else {
                "empty imported main paragraph stays unchanged for no-op or invalid virtual-run writes"
            }
        );
        let changed = editor.apply(Edit::SetText {
            paragraph: paragraph_id,
            inline: 0,
            text: String::from("Neu Ω"),
        });
        let edited = editor.document().clone();
        let undone = editor.undo();
        let revision = editor.revision();
        let no_op = editor.apply(Edit::SetText {
            paragraph: paragraph_id,
            inline: 0,
            text: String::new(),
        });
        check!(
            changed == Ok(paragraph_id)
                && edited.paragraph(paragraph_id).unwrap().content == vec![text("Neu Ω")]
                && undone
                && editor.document() == &empty
                && !editor.dirty()
                && no_op == Ok(paragraph_id)
                && editor.revision() == revision
                && editor.can_redo()
                && editor.redo()
                && editor.document() == &edited
                && editor.dirty(),
            if note_body {
                "typing into an empty note body creates one text run and restores exact empty structure on undo"
            } else {
                "typing into an empty main paragraph creates one text run and restores exact empty structure on undo"
            }
        );
    }
    let mut saved = Editor::new(d).unwrap();
    let note = saved
        .apply(Edit::InsertNote {
            paragraph: 1,
            inline: 0,
            offset: 0,
            kind: NoteKind::Footnote,
            text: String::new(),
        })
        .unwrap();
    saved.mark_saved();
    check!(
        note > 21
            && !saved.dirty()
            && saved.undo()
            && saved.dirty()
            && saved.redo()
            && !saved.dirty(),
        "notes respect saved checkpoint and source-aware identity allocation"
    );
    (passed, failed)
}
