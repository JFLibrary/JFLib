// Atomic structure, history identities and DOCX round trips. New native editor contracts.
use alloc::{format, string::String, vec};
use doc_core::{Document, Edit, Editor, Inline, Paragraph, ParagraphStyle, TextFormat};

fn simple(text: &str) -> Document {
    let mut d = Document::default();
    d.paragraphs[0].content = vec![Inline::Text(String::from(text))];
    d
}
fn split(paragraph: u64, inline: usize, start: usize, end: usize) -> Edit {
    Edit::SplitParagraph {
        paragraph,
        inline,
        start,
        end,
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
    let text = "Aα𝄞中Z";
    for offset in 0..=text.len() {
        let original = simple(text);
        let mut e = Editor::new(original.clone()).unwrap();
        let result = e.apply(split(1, 0, offset, offset));
        if text.is_char_boundary(offset) {
            let id = result.unwrap();
            check!(
                e.document().paragraph(1).unwrap().content[0].text() == Some(&text[..offset])
                    && e.document().paragraph(id).unwrap().content[0].text()
                        == Some(&text[offset..]),
                "split preserves every Unicode scalar boundary"
            );
            let seam = e.document().join_boundary(1, id).unwrap();
            check!(
                seam.paragraph == 1 && seam.inline == 0 && seam.offset == offset,
                "join seam is the original UTF-8 byte offset"
            );
            e.apply(Edit::JoinParagraphs {
                first: 1,
                second: id,
            })
            .unwrap();
            check!(
                *e.document() == original,
                "split/join restores exact plain document"
            );
        } else {
            check!(
                result.is_err() && *e.document() == original && e.state_id() == 0 && !e.can_undo(),
                "split rejects partial scalar atomically"
            );
            check!(
                e.apply(split(1, 0, 0, 0)) == Ok(2),
                "failed split does not allocate identity"
            );
        }
    }
    let original = simple(text);
    let mut e = Editor::new(original.clone()).unwrap();
    let id = e.apply(split(1, 0, 1, text.len() - 1)).unwrap();
    let after = e.document().clone();
    let state = e.state_id();
    check!(
        after.paragraphs[0].content[0].text() == Some("A")
            && after.paragraphs[1].content[0].text() == Some("Z"),
        "split replaces selected Unicode range"
    );
    check!(
        e.undo() && *e.document() == original && e.state_id() == 0,
        "selection split is one undo transaction"
    );
    for edit in [
        split(1, 0, 6, 2),
        split(1, 99, 0, 0),
        split(999, 0, 0, 0),
        Edit::JoinParagraphs {
            first: 1,
            second: id,
        },
    ] {
        check!(
            e.apply(edit).is_err()
                && e.state_id() == 0
                && e.can_redo()
                && *e.document() == original,
            "invalid structural edit preserves redo and document"
        );
    }
    e.apply(Edit::SetText {
        paragraph: 1,
        inline: 0,
        text: String::from(text),
    })
    .unwrap();
    check!(
        e.state_id() == 0 && e.can_redo(),
        "no-op retains history identity and redo"
    );
    check!(
        e.redo() && *e.document() == after && e.state_id() == state,
        "redo restores structural state identity"
    );
    e.undo();
    e.apply(split(1, 0, 0, 0)).unwrap();
    check!(
        e.state_id() != state && !e.can_redo(),
        "branch uses fresh state identity"
    );

    let mut heading = simple("Heading");
    heading.paragraphs[0].style = ParagraphStyle::Heading1;
    heading.paragraphs[0].format.page_break_before = true;
    let mut e = Editor::new(heading.clone()).unwrap();
    let id = e.apply(split(1, 0, 3, 3)).unwrap();
    check!(
        e.document().paragraph(id).unwrap().style == ParagraphStyle::Heading1
            && !e.document().paragraph(id).unwrap().format.page_break_before
            && e.document().paragraph(1).unwrap().format.page_break_before,
        "heading middle continues style but not page break"
    );
    let mut e = Editor::new(heading).unwrap();
    let id = e.apply(split(1, 0, 7, 7)).unwrap();
    check!(
        e.document().paragraph(id).unwrap().style == ParagraphStyle::Normal,
        "heading end continues as body text"
    );
    let mut empty = simple("");
    empty.paragraphs[0].content.clear();
    let mut e = Editor::new(empty).unwrap();
    check!(
        e.apply(split(1, 0, 0, 0)).is_ok() && e.document().paragraphs.len() == 2,
        "virtual empty run can split"
    );

    let original = crate::notes_io_checks::sample();
    let mut e = Editor::new(original.clone()).unwrap();
    let id = e.apply(split(1, 2, 4, 4)).unwrap();
    check!(
        e.document().paragraph(1).unwrap().content[1] == Inline::NoteReference(30)
            && e.document().paragraph(id).unwrap().content[1] == Inline::NoteReference(20)
            && e.document().notes == original.notes
            && e.document().sources == original.sources,
        "split carries note anchors without changing note identities or bibliography"
    );
    check!(
        doc_io::decode_docx(&doc_io::encode_docx(e.document()).unwrap())
            == Ok(e.document().clone()),
        "split document and scientific references round trip through DOCX"
    );
    e.apply(Edit::JoinParagraphs {
        first: 1,
        second: id,
    })
    .unwrap();
    check!(
        *e.document() == original,
        "join merges matching seam runs without textifying fields"
    );
    let id = e.apply(split(31, 0, 3, 3)).unwrap();
    check!(
        e.document().note_for_paragraph(id).unwrap().id == 30
            && e.document().paragraph(id).unwrap().content[0].format()
                == original.paragraph(31).unwrap().content[0].format()
            && e.document().paragraph(id).unwrap().content[1]
                == original.paragraph(31).unwrap().content[1],
        "note-body split preserves rich text and citation cluster"
    );
    e.apply(Edit::JoinParagraphs {
        first: 31,
        second: id,
    })
    .unwrap();
    check!(
        *e.document() == original,
        "note-body split/join retains complete scientific model"
    );
    for (first, second) in [(1, 21), (11, 21), (12, 11), (1, 1), (3, 1), (999, 1)] {
        check!(
            e.document().join_boundary(first, second).is_err()
                && e.apply(Edit::JoinParagraphs { first, second }).is_err()
                && *e.document() == original,
            "join rejects cross-story and nonadjacent paragraphs"
        );
    }
    check!(
        e.apply(split(1, 1, 0, 0)).is_err(),
        "cannot split a note anchor as text"
    );
    // Field-to-field joins need an editable seam; heterogeneous run formats must survive.
    for (left, right) in [
        (vec![], vec![]),
        (vec![Inline::Bibliography], vec![Inline::Bibliography]),
        (
            vec![Inline::Bibliography],
            vec![Inline::Text(String::from("right"))],
        ),
        (
            vec![Inline::Text(String::from("left"))],
            vec![Inline::Bibliography],
        ),
        (
            vec![Inline::with_format(
                String::from("α"),
                TextFormat {
                    bold: Some(true),
                    ..Default::default()
                },
            )],
            vec![Inline::with_format(
                String::from("β"),
                TextFormat {
                    italic: Some(true),
                    ..Default::default()
                },
            )],
        ),
    ] {
        let mut d = simple("");
        d.paragraphs[0].content = left;
        d.paragraphs.push(Paragraph {
            id: 2,
            content: right,
            style: ParagraphStyle::Normal,
            format: Default::default(),
        });
        let mut e = Editor::new(d.clone()).unwrap();
        let seam = e.document().join_boundary(1, 2).unwrap();
        e.apply(Edit::JoinParagraphs {
            first: 1,
            second: 2,
        })
        .unwrap();
        check!(
            e.document().paragraph(1).unwrap().content[seam.inline]
                .text()
                .is_some_and(|s| s.is_char_boundary(seam.offset)),
            "join always yields an editable valid seam"
        );
        check!(
            doc_io::decode_docx(&doc_io::encode_docx(e.document()).unwrap())
                == Ok(e.document().clone()),
            "field and mixed-format joins round trip through DOCX"
        );
        check!(
            e.undo() && *e.document() == d,
            "heterogeneous join undo restores both paragraphs exactly"
        );
    }
    let mut large = simple("");
    for id in 2..=2048 {
        large.paragraphs.push(Paragraph {
            id,
            content: vec![],
            style: ParagraphStyle::Normal,
            format: Default::default(),
        });
    }
    let mut e = Editor::new(large.clone()).unwrap();
    check!(
        e.apply(split(1, 0, 0, 0)).is_err()
            && *e.document() == large
            && !e.can_undo()
            && e.state_id() == 0,
        "paragraph budget failure is atomic"
    );
    e.apply(Edit::JoinParagraphs {
        first: 1,
        second: 2,
    })
    .unwrap();
    check!(
        e.apply(split(1, 0, 0, 0)) == Ok(2049),
        "budget rejection does not consume IDs"
    );
    // Trimmed history must keep caret-state identities aligned with document snapshots.
    let mut e = Editor::new(simple("")).unwrap();
    let mut states = vec![e.state_id()];
    for n in 1..=70 {
        e.apply(Edit::SetText {
            paragraph: 1,
            inline: 0,
            text: format!("{n}"),
        })
        .unwrap();
        states.push(e.state_id());
    }
    for n in (6..70).rev() {
        check!(
            e.undo()
                && e.state_id() == states[n]
                && e.document().paragraph(1).unwrap().content[0].text()
                    == Some(format!("{n}").as_str()),
            "trimmed undo snapshot and state identities agree"
        );
    }
    check!(!e.undo(), "history retains exactly 64 transactions");
    for expected in &states[7..] {
        check!(
            e.redo() && e.state_id() == *expected,
            "trimmed redo restores exact identity"
        );
    }
    check!(!e.redo(), "redo reaches final transaction");
    (passed, failed)
}
