// Model transactions over paragraph marks, independently of the page editor.
use alloc::{format, string::String, vec};
use doc_core::{Document, Edit, Editor, Inline, Paragraph, ParagraphStyle, TextFormat};

fn sample() -> Document {
    let mut d = Document::default();
    d.paragraphs[0].content = vec![
        Inline::Text(String::from("Aα")),
        Inline::with_format(
            String::from("B"),
            TextFormat {
                bold: Some(true),
                ..Default::default()
            },
        ),
    ];
    d.paragraphs.push(Paragraph {
        id: 2,
        style: ParagraphStyle::Heading2,
        format: Default::default(),
        content: vec![Inline::Text(String::from("中Z"))],
    });
    d.paragraphs.push(Paragraph {
        id: 3,
        style: ParagraphStyle::Normal,
        format: Default::default(),
        content: vec![Inline::with_format(
            String::from("ΩQ"),
            TextFormat {
                italic: Some(true),
                ..Default::default()
            },
        )],
    });
    d
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
    let text = "AαB\n中Z\nΩQ";
    for start in 0..=text.len() {
        for end in start..=text.len() {
            for replacement in ["", "δ", "x\ny\n"] {
                let mut e = Editor::new(d.clone()).unwrap();
                let result = e.apply(Edit::ReplaceStoryText {
                    paragraph: 2,
                    start,
                    end,
                    text: String::from(replacement),
                    paragraph_breaks: true,
                });
                if text.is_char_boundary(start) && text.is_char_boundary(end) {
                    let expected = format!("{}{replacement}{}", &text[..start], &text[end..]);
                    check!(
                        result.is_ok()
                            && e.document().story_projection(1).unwrap().text == expected,
                        "story replacement across all scalar boundaries"
                    );
                    check!(
                        e.document().paragraphs.len()
                            == expected.bytes().filter(|b| *b == b'\n').count() + 1,
                        "paragraph marks remain native structure"
                    );
                    check!(
                        doc_io::decode_docx(&doc_io::encode_docx(e.document()).unwrap())
                            == Ok(e.document().clone()),
                        "cross-paragraph edit DOCX round trip"
                    );
                    if *e.document() != d {
                        check!(
                            e.undo() && *e.document() == d && e.redo(),
                            "one atomic story history step"
                        );
                    }
                } else {
                    check!(
                        result.is_err() && *e.document() == d && !e.can_undo(),
                        "invalid story scalar leaves history untouched"
                    );
                }
            }
        }
    }
    let mut e = Editor::new(d.clone()).unwrap();
    let f = TextFormat {
        underline: Some(true),
        ..Default::default()
    };
    check!(
        e.apply(Edit::FormatStoryText {
            paragraph: 3,
            start: 1,
            end: text.len() - 1,
            format: f
        })
        .is_ok(),
        "format several paragraphs"
    );
    check!(
        e.document().paragraphs[0]
            .content
            .last()
            .unwrap()
            .format()
            .bold
            == Some(true)
            && e.document().paragraphs[1].style == ParagraphStyle::Heading2
            && e.document().paragraphs[2]
                .content
                .first()
                .unwrap()
                .format()
                .italic
                == Some(true)
            && e.document().paragraphs[2]
                .content
                .last()
                .unwrap()
                .format()
                .underline
                .is_none(),
        "format patch preserves styles and unselected suffix"
    );
    check!(
        e.undo() && *e.document() == d,
        "format across paragraphs is one undo"
    );
    e.apply(Edit::ReplaceStoryText {
        paragraph: 1,
        start: 0,
        end: 0,
        text: String::from("x\ny"),
        paragraph_breaks: false,
    })
    .unwrap();
    check!(
        e.document().paragraphs.len() == 3
            && e.document()
                .text_projection(1)
                .unwrap()
                .text
                .starts_with("x\ny"),
        "soft newline keeps paragraph"
    );
    e.undo();
    e.apply(Edit::ReplaceStoryText {
        paragraph: 1,
        start: 0,
        end: 0,
        text: String::from("x\r\ny\rz"),
        paragraph_breaks: true,
    })
    .unwrap();
    check!(
        e.document().paragraphs.len() == 5
            && e.document()
                .story_projection(1)
                .unwrap()
                .text
                .starts_with("x\ny\nz"),
        "clipboard CRLF and CR become paragraph marks"
    );

    let mut d = sample();
    d.paragraphs.truncate(2);
    let bold = TextFormat {
        bold: Some(true),
        ..Default::default()
    };
    let italic = TextFormat {
        italic: Some(true),
        ..Default::default()
    };
    d.paragraphs[0].content = vec![
        Inline::Text(String::from("A")),
        Inline::with_format(String::from("B"), bold),
    ];
    d.paragraphs[1].content = vec![Inline::with_format(String::from("CD"), italic)];
    let mut e = Editor::new(d.clone()).unwrap();
    e.apply(Edit::ReplaceStoryText {
        paragraph: 1,
        start: 1,
        end: 4,
        text: String::from("x\ny"),
        paragraph_breaks: true,
    })
    .unwrap();
    check!(
        e.document().paragraphs[0].content
            == vec![
                Inline::Text(String::from("A")),
                Inline::with_format(String::from("x"), bold)
            ]
            && e.document().paragraphs[1].content
                == vec![
                    Inline::with_format(String::from("y"), bold),
                    Inline::with_format(String::from("D"), italic)
                ],
        "inserted paragraphs inherit selected run format while suffix retains its format"
    );
    check!(
        e.undo() && *e.document() == d,
        "formatted multi-paragraph paste undo restores complete model"
    );
    e.apply(Edit::ReplaceStoryText {
        paragraph: 1,
        start: 1,
        end: 2,
        text: String::from("Ω"),
        paragraph_breaks: true,
    })
    .unwrap();
    check!(
        e.document().paragraphs[0].content.last().unwrap().format() == bold,
        "replacement at a formatting seam inherits selected run"
    );
    let mut d = sample();
    d.paragraphs[1].content.clear();
    let mut e = Editor::new(d.clone()).unwrap();
    let length = d.story_projection(1).unwrap().text.len();
    e.apply(Edit::FormatStoryText {
        paragraph: 1,
        start: 0,
        end: length,
        format: bold,
    })
    .unwrap();
    e.apply(Edit::ReplaceParagraphText {
        paragraph: 2,
        start: 0,
        end: 0,
        text: String::from("inserted"),
    })
    .unwrap();
    check!(
        e.document().paragraphs[1].content[0].format().bold == Some(true),
        "selected empty paragraph retains typing format"
    );
    e.undo();
    e.undo();
    check!(*e.document() == d, "empty paragraph formatting history");
    let story = d.story_projection(1).unwrap();
    let end = story.offset(3, 0).unwrap();
    e.apply(Edit::FormatStoryParagraphs {
        paragraph: 1,
        start: 1,
        end,
        change: doc_core::ParagraphChange::Alignment(doc_core::Alignment::Right),
    })
    .unwrap();
    check!(
        e.document().paragraphs[..2]
            .iter()
            .all(|p| p.format.alignment == doc_core::Alignment::Right)
            && e.document().paragraphs[2].format == d.paragraphs[2].format
            && e.document().paragraphs[1].style == d.paragraphs[1].style,
        "paragraph command excludes end-at-next-start and preserves unrelated properties"
    );
    check!(
        e.undo() && *e.document() == d,
        "multi-paragraph alignment atomic undo"
    );
    let before = e.state_id();
    let mut invalid = doc_core::ParagraphFormat::default();
    invalid.first_line = 30000;
    check!(
        e.apply(Edit::FormatStoryParagraphs {
            paragraph: 1,
            start: 0,
            end: length,
            change: doc_core::ParagraphChange::Format(invalid)
        })
        .is_err()
            && *e.document() == d
            && e.state_id() == before
            && e.can_redo(),
        "invalid paragraph selection format leaves document and redo untouched"
    );

    let d = crate::checks::sample();
    let mut e = Editor::new(d.clone()).unwrap();
    let length = d.story_projection(1).unwrap().text.len();
    check!(
        e.apply(Edit::ReplaceStoryText {
            paragraph: 1,
            start: 0,
            end: length,
            text: String::from("lost"),
            paragraph_breaks: true
        })
        .is_err()
            && *e.document() == d
            && !e.can_undo(),
        "field and bibliography protection across paragraphs is atomic"
    );
    check!(
        e.apply(Edit::FormatStoryText {
            paragraph: 1,
            start: 0,
            end: length,
            format: f
        })
        .is_err()
            && *e.document() == d,
        "format field intersection leaves earlier paragraphs unchanged"
    );

    let mut e = Editor::new(sample()).unwrap();
    let note = e
        .apply(Edit::InsertNote {
            paragraph: 1,
            inline: 0,
            offset: 1,
            kind: doc_core::NoteKind::Footnote,
            text: String::from("note"),
        })
        .unwrap();
    let body = e.document().note(note).unwrap().paragraphs[0].id;
    let main = e.document().paragraphs.clone();
    e.apply(Edit::ReplaceStoryText {
        paragraph: body,
        start: 1,
        end: 3,
        text: String::from("α\nβ"),
        paragraph_breaks: true,
    })
    .unwrap();
    check!(
        e.document().story_projection(body).unwrap().text == "nα\nβe"
            && e.document().paragraphs == main
            && e.document().note(note).unwrap().paragraphs.len() == 2,
        "note story edits never consume main body paragraphs"
    );
    check!(
        doc_io::decode_docx(&doc_io::encode_docx(e.document()).unwrap())
            == Ok(e.document().clone()),
        "multi-paragraph note DOCX round trip"
    );
    (passed, failed)
}
