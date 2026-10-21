// Paragraph projections, atomic mixed-run edits and native DOCX round trips.
use alloc::{format, string::String, vec};
use doc_core::{Document, Edit, Editor, Inline, TextColor, TextFormat};

struct Fixed;
pub(crate) fn pages_sample() -> Document {
    let mut document = crate::checks::sample();
    document.paragraphs.truncate(1);
    let p = &mut document.paragraphs[0];
    p.style = doc_core::ParagraphStyle::Normal;
    p.format.alignment = doc_core::Alignment::Justify;
    p.content = vec![
        Inline::with_format(
            String::from("Start α "),
            TextFormat {
                bold: Some(true),
                color: Some(TextColor::Rgb(0x2459a6)),
                ..Default::default()
            },
        ),
        Inline::Text("Forschung αβ bleibt prüfbar. ".repeat(180)),
        Inline::with_format(
            String::from("Schluss Ω"),
            TextFormat {
                strike: Some(true),
                italic: Some(true),
                ..Default::default()
            },
        ),
    ];
    document
}
impl doc_layout::Measure for Fixed {
    fn width(&mut self, text: &str, _: doc_core::ResolvedFormat) -> i32 {
        text.chars().count() as i32 * 120
    }
    fn metrics(&mut self, _: doc_core::ResolvedFormat) -> doc_layout::Metrics {
        doc_layout::Metrics {
            ascent: 180,
            descent: 60,
            height: 240,
        }
    }
}

pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
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
    let bold = TextFormat {
        bold: Some(true),
        ..Default::default()
    };
    let italic = TextFormat {
        italic: Some(true),
        ..Default::default()
    };
    let mut d = Document::default();
    d.paragraphs[0].content = vec![
        Inline::Text(String::from("Aα")),
        Inline::with_format(String::from("𝄞B"), bold),
        Inline::with_format(String::from("中Z"), italic),
    ];
    let text = "Aα𝄞B中Z";
    for start in 0..=text.len() {
        for end in start..=text.len() {
            let mut e = Editor::new(d.clone()).unwrap();
            let result = e.apply(Edit::ReplaceParagraphText {
                paragraph: 1,
                start,
                end,
                text: String::from("Ω"),
            });
            if text.is_char_boundary(start) && text.is_char_boundary(end) {
                check!(
                    result.is_ok()
                        && e.document().text_projection(1).unwrap().text
                            == format!("{}Ω{}", &text[..start], &text[end..]),
                    "mixed-run replacement at every scalar range"
                );
                check!(
                    doc_io::decode_docx(&doc_io::encode_docx(e.document()).unwrap())
                        == Ok(e.document().clone()),
                    "mixed-run replacement DOCX round trip"
                );
                check!(
                    e.undo() && *e.document() == d && e.redo(),
                    "mixed-run replacement atomic history"
                );
            } else {
                check!(
                    result.is_err() && *e.document() == d && !e.can_undo(),
                    "partial scalar range is atomic rejection"
                );
            }
        }
    }
    let mut e = Editor::new(d.clone()).unwrap();
    e.apply(Edit::FormatParagraphText {
        paragraph: 1,
        start: 1,
        end: 11,
        format: TextFormat {
            underline: Some(true),
            ..Default::default()
        },
    })
    .unwrap();
    let p = &e.document().paragraphs[0];
    check!(
        p.content[1].format().underline == Some(true)
            && p.content[2].format().bold == Some(true)
            && p.content[2].format().underline == Some(true)
            && p.content[3].format().italic == Some(true)
            && p.content[3].format().underline == Some(true)
            && p.content.last().unwrap().text() == Some("Z"),
        "format patch preserves per-run properties and selection edges"
    );
    e.undo();
    e.apply(Edit::FormatParagraphText {
        paragraph: 1,
        start: 1,
        end: 1,
        format: bold,
    })
    .unwrap();
    e.apply(Edit::ReplaceParagraphText {
        paragraph: 1,
        start: 1,
        end: 1,
        text: String::from("xx"),
    })
    .unwrap();
    check!(
        e.document().paragraphs[0].content[1].text() == Some("xx")
            && e.document().paragraphs[0].content[1].format().bold == Some(true),
        "collapsed formatting controls subsequent inserted text"
    );
    let mut e = Editor::new(d.clone()).unwrap();
    let id = e
        .apply(Edit::SplitParagraphText {
            paragraph: 1,
            start: 1,
            end: 11,
        })
        .unwrap();
    check!(
        e.document().text_projection(1).unwrap().text == "A"
            && e.document().text_projection(id).unwrap().text == "Z",
        "Enter replaces cross-run selection atomically"
    );
    check!(
        e.undo() && *e.document() == d,
        "cross-run split undo preserves exact runs"
    );
    let mut e = Editor::new(crate::checks::sample()).unwrap();
    let projection = e.document().text_projection(1).unwrap();
    let field = projection.spans.iter().find(|s| !s.editable).unwrap();
    let old = e.document().clone();
    for edit in [
        Edit::ReplaceParagraphText {
            paragraph: 1,
            start: 0,
            end: projection.text.len(),
            text: String::new(),
        },
        Edit::ReplaceParagraphText {
            paragraph: 1,
            start: field.start + 1,
            end: field.start + 1,
            text: String::from("X"),
        },
        Edit::FormatParagraphText {
            paragraph: 1,
            start: 0,
            end: projection.text.len(),
            format: bold,
        },
        Edit::SplitParagraphText {
            paragraph: 1,
            start: field.start + 1,
            end: field.end - 1,
        },
    ] {
        check!(
            e.apply(edit).is_err() && *e.document() == old && !e.can_undo(),
            "generated field intersections reject edits without mutation"
        );
    }
    e.apply(Edit::ReplaceParagraphText {
        paragraph: 1,
        start: field.end,
        end: field.end,
        text: String::from("X"),
    })
    .unwrap();
    check!(
        e.document().paragraphs[0].content[1] == old.paragraphs[0].content[1],
        "typing after citation preserves native field"
    );
    check!(
        doc_io::decode_docx(&doc_io::encode_docx(e.document()).unwrap())
            == Ok(e.document().clone()),
        "continuous citation editing retains DOCX field structure"
    );
    for content in [vec![], vec![Inline::Text(String::new())]] {
        let mut d = Document::default();
        d.paragraphs[0].content = content;
        let mut e = Editor::new(d).unwrap();
        check!(
            e.apply(Edit::ReplaceParagraphText {
                paragraph: 1,
                start: 0,
                end: 0,
                text: String::from("α")
            })
            .is_ok()
                && e.document().text_projection(1).unwrap().text == "α",
            "empty paragraph accepts projected insertion"
        );
    }
    for color in [
        None,
        Some(TextColor::Auto),
        Some(TextColor::Rgb(0)),
        Some(TextColor::Rgb(0xffffff)),
        Some(TextColor::Rgb(0xabcdef)),
    ] {
        for strike in [None, Some(true), Some(false)] {
            let mut e = Editor::new(d.clone()).unwrap();
            e.apply(Edit::FormatParagraphText {
                paragraph: 1,
                start: 0,
                end: text.len(),
                format: TextFormat {
                    color,
                    strike,
                    ..Default::default()
                },
            })
            .unwrap();
            check!(
                doc_io::decode_docx(&doc_io::encode_docx(e.document()).unwrap())
                    == Ok(e.document().clone()),
                "RGB/automatic color and strike preserve explicit values in DOCX"
            );
            e.apply(Edit::FormatParagraphText {
                paragraph: 1,
                start: 0,
                end: text.len(),
                format: TextFormat {
                    underline: Some(true),
                    ..Default::default()
                },
            })
            .unwrap();
            check!(
                e.document().paragraphs[0]
                    .content
                    .iter()
                    .all(|n| n.format().color == color && n.format().strike == strike),
                "other formatting retains RGB and strike per run"
            );
        }
    }
    check!(
        TextFormat {
            color: Some(TextColor::Rgb(0x1000000)),
            ..Default::default()
        }
        .validate()
        .is_err(),
        "RGB rejects alpha/out-of-range values"
    );
    let mut layout_doc = d.clone();
    layout_doc.paragraphs[0].content.push(Inline::Text(
        String::from("\r\n\tZeile αβ ") + &" LangerTextαβ".repeat(700) + "\n\n",
    ));
    for alignment in [
        doc_core::Alignment::Left,
        doc_core::Alignment::Center,
        doc_core::Alignment::Right,
        doc_core::Alignment::Justify,
    ] {
        layout_doc.paragraphs[0].format.alignment = alignment;
        let projection = layout_doc.text_projection(1).unwrap();
        let layout = doc_layout::paginate(&layout_doc, &mut Fixed).unwrap();
        check!(layout.pages.len() > 1, "rich layout covers multiple pages");
        let mut end = 0;
        for line in layout.pages.iter().flat_map(|p| &p.lines) {
            check!(
                line.start >= end
                    && line.end >= line.start
                    && projection.text.is_char_boundary(line.start)
                    && projection.text.is_char_boundary(line.end),
                "line ranges retain ordered scalar boundaries across page breaks"
            );
            for fragment in &line.fragments {
                let (a, b) = fragment.source.unwrap();
                check!(
                    a >= line.start
                        && b <= line.end
                        && (projection.text.get(a..b) == Some(fragment.text.as_str())
                            || projection.text.get(a..b) == Some("\t")),
                    "fragment provenance maps exactly to UTF-8 text or tab"
                );
            }
            end = line.end;
        }
        check!(
            end == projection.text.len(),
            "final empty line retains end caret"
        );
    }
    (passed, failed)
}
