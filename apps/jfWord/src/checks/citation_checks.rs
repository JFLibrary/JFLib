// Native contracts for shared citation editing, usage tracking and generated text.
use alloc::{format, string::String, vec};
use doc_core::{
    CitationItem, CitationLocation, Document, Edit, Editor, Error, Inline, Paragraph,
    ParagraphFormat, ParagraphStyle, Person, SearchOptions, Source, TextFormat,
};

fn item(source: u64) -> CitationItem {
    CitationItem {
        source,
        ..Default::default()
    }
}
fn document() -> Document {
    let mut document = Document::default();
    document.sources = vec![
        Source {
            id: 2,
            authors: vec![Person {
                family: String::from("Müller"),
                ..Default::default()
            }],
            title: String::from("Methoden"),
            year: String::from("2024"),
            ..Default::default()
        },
        Source {
            id: 3,
            corporate: String::from("Institut"),
            title: String::from("Forschungsbericht"),
            year: String::from("2025"),
            ..Default::default()
        },
        Source {
            id: 4,
            title: String::from("Handbuch"),
            year: String::from("2026"),
            ..Default::default()
        },
    ];
    document
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
    let simple = item(2);
    let old = Inline::Citation {
        source: 2,
        locator: String::new(),
    };
    check!(
        Inline::citation(vec![simple.clone()]) == Ok(old.clone())
            && old.citation_items() == Some(vec![simple.clone()]),
        "citation canonical legacy representation"
    );
    check!(
        Inline::Text(String::new()).citation_items().is_none()
            && Inline::Bibliography.citation_items().is_none(),
        "only citation nodes expose items"
    );
    check!(
        Inline::citation(vec![]) == Err(Error::Invalid)
            && Inline::citation(vec![simple.clone(); 33]) == Err(Error::Limit)
            && Inline::citation(vec![simple.clone(); 32]).is_ok(),
        "citation item count limits"
    );
    check!(
        ["bad\"", "bad\\", "bad\n", "bad\r", "\0"]
            .iter()
            .all(|bad| {
                let mut invalid = simple.clone();
                invalid.prefix = String::from(*bad);
                Inline::citation(vec![invalid]).is_err()
            }),
        "citation affixes reject unsafe field-code characters"
    );
    check!(
        Inline::citation(vec![CitationItem {
            locator: "ü".repeat(64),
            ..simple.clone()
        }])
        .is_ok()
            && Inline::citation(vec![CitationItem {
                locator: "ü".repeat(65),
                ..simple.clone()
            }])
            .is_err()
            && Inline::citation(vec![CitationItem {
                suffix: "x".repeat(129),
                ..simple.clone()
            }])
            .is_err()
            && Inline::citation(vec![item(0)]).is_err(),
        "citation text uses UTF-8 byte budgets and stable identities"
    );
    let d = document();
    check!(
        d.citation_cluster_text(&[simple.clone()]) == d.citation_text(2, "")
            && d.citation_cluster_text(&[CitationItem {
                locator: String::from("12–15"),
                ..simple.clone()
            }]) == d.citation_text(2, "12–15"),
        "simple single citation preserves existing rendering"
    );
    let complex = vec![
        CitationItem {
            prefix: String::from("vgl. "),
            locator: String::from("12–15"),
            suffix: String::from(", Hervorhebung ergänzt"),
            ..simple.clone()
        },
        CitationItem {
            suppress_author: true,
            ..item(3)
        },
        CitationItem {
            suppress_year: true,
            ..item(4)
        },
    ];
    check!(
        d.citation_cluster_text(&complex)
            == "(vgl. Müller, 2024, S. 12–15, Hervorhebung ergänzt; 2025; Handbuch)",
        "multi-source citation preserves order, affixes, locator and suppression"
    );
    check!(
        d.citation_cluster_text(&[
            CitationItem {
                suppress_title: true,
                ..item(4)
            },
            CitationItem {
                suppress_author: true,
                suppress_year: true,
                locator: String::from("7"),
                ..simple.clone()
            },
            CitationItem {
                suppress_title: true,
                ..item(3)
            },
        ]) == "(2026; S. 7; Institut, 2025)",
        "suppression keeps punctuation valid and only hides anonymous title fallback"
    );
    let style = TextFormat {
        bold: Some(true),
        italic: Some(true),
        ..Default::default()
    };
    let mut d = d;
    d.paragraphs[0].content = vec![Inline::with_format(String::from("Äpfel Ω"), style)];
    let mut e = Editor::new(d).unwrap();
    let before = e.document().clone();
    check!(
        e.apply(Edit::InsertCitation {
            paragraph: 1,
            inline: 0,
            offset: 1,
            items: complex.clone()
        }) == Err(Error::Invalid)
            && *e.document() == before
            && !e.dirty()
            && e.revision() == 0,
        "citation insertion rejects mid-codepoint positions atomically"
    );
    e.apply(Edit::InsertCitation {
        paragraph: 1,
        inline: 0,
        offset: 7,
        items: complex.clone(),
    })
    .unwrap();
    let nodes = &e.document().paragraphs[0].content;
    check!(
        nodes.len() == 3
            && nodes[0] == Inline::with_format(String::from("Äpfel "), style)
            && nodes[1].citation_items() == Some(complex.clone())
            && nodes[2] == Inline::with_format(String::from("Ω"), style),
        "caret insertion preserves styled UTF-8 prefix and suffix"
    );
    check!(
        e.document().paragraph_text(&e.document().paragraphs[0])
            == "Äpfel (vgl. Müller, 2024, S. 12–15, Hervorhebung ergänzt; 2025; Handbuch)Ω",
        "plain-text view expands clustered citation"
    );
    check!(
        e.undo() && *e.document() == before && e.redo(),
        "citation insertion is one complete undo transaction"
    );
    let snapshot = e.document().clone();
    e.apply(Edit::UpdateCitation {
        paragraph: 1,
        inline: 1,
        items: vec![simple.clone(), simple.clone(), item(3)],
    })
    .unwrap();
    check!(
        e.document().bibliography().len() == 2
            && e.document().bibliography()[0].starts_with("Müller")
            && e.document().bibliography()[1].starts_with("Institut"),
        "bibliography deduplicates cluster sources in first-use order"
    );
    check!(
        e.document().source_uses(2)
            == vec![CitationLocation {
                paragraph: 1,
                inline: 1
            }]
            && e.document().source_uses(4).is_empty(),
        "usage lookup reports one field per repeated source"
    );
    e.undo();
    let rev = e.revision();
    check!(
        e.apply(Edit::UpdateCitation {
            paragraph: 1,
            inline: 1,
            items: vec![item(99)]
        }) == Err(Error::Missing)
            && *e.document() == snapshot
            && e.can_redo()
            && e.revision() == rev,
        "dangling citation update preserves document and redo"
    );
    check!(
        e.apply(Edit::UpdateCitation {
            paragraph: 1,
            inline: 0,
            items: vec![simple.clone()]
        }) == Err(Error::Invalid)
            && *e.document() == snapshot,
        "citation update cannot replace editable text"
    );
    check!(
        e.apply(Edit::InsertCitation {
            paragraph: 1,
            inline: 1,
            offset: 0,
            items: vec![simple.clone()]
        }) == Err(Error::Invalid)
            && *e.document() == snapshot,
        "citation insertion cannot split a generated field"
    );
    check!(
        e.apply(Edit::RemoveSource(3)) == Err(Error::InUse)
            && *e.document() == snapshot
            && e.can_redo(),
        "clustered source deletion is protected and atomic"
    );
    let mut updated_source = e.document().source(2).unwrap().clone();
    updated_source.year = String::from("2030");
    e.apply(Edit::PutSource(updated_source)).unwrap();
    check!(
        e.document().plain_text().contains("Müller, 2030") && e.undo() && *e.document() == snapshot,
        "source edits refresh cluster labels and undo restores them"
    );
    e.apply(Edit::ConvertCitationToText {
        paragraph: 1,
        inline: 1,
    })
    .unwrap();
    check!(
        e.document().paragraphs[0].content[1].text()
            == Some(snapshot.citation_cluster_text(&complex).as_str())
            && e.document().source_uses(2).is_empty()
            && e.document().bibliography().is_empty(),
        "conversion freezes field text and releases source references"
    );
    e.apply(Edit::RemoveSource(2)).unwrap();
    check!(
        e.document().plain_text().contains("Müller")
            && e.undo()
            && e.undo()
            && *e.document() == snapshot,
        "frozen citation survives source deletion and transactions undo independently"
    );
    e.apply(Edit::RemoveCitation {
        paragraph: 1,
        inline: 1,
    })
    .unwrap();
    check!(
        e.document().plain_text() == "Äpfel Ω" && e.undo() && *e.document() == snapshot,
        "remove clustered citation preserves surrounding text and undoes"
    );
    check!(
        snapshot
            .find_text("Müller", SearchOptions::default())
            .unwrap()
            .is_empty()
            && snapshot
                .find_text("Äpfel Ω", SearchOptions::default())
                .unwrap()
                .is_empty(),
        "clustered citations remain opaque search barriers"
    );
    let mut d = document();
    d.paragraphs[0].content = vec![Inline::Text(String::from("Ende"))];
    let mut tail = Editor::new(d).unwrap();
    tail.apply(Edit::InsertCitation {
        paragraph: 1,
        inline: 0,
        offset: 4,
        items: vec![simple.clone()],
    })
    .unwrap();
    check!(
        tail.document().paragraphs[0].content[2].text() == Some(""),
        "citation inserted at run end keeps an editable empty suffix"
    );
    tail.apply(Edit::InsertCitation {
        paragraph: 1,
        inline: 0,
        offset: 0,
        items: vec![item(3)],
    })
    .unwrap();
    check!(
        tail.document().paragraphs[0].content[0].citation_items() == Some(vec![item(3)])
            && tail.document().paragraphs[0].content[1].text() == Some("Ende"),
        "citation insertion at run start preserves the entire suffix"
    );
    let mut d = document();
    d.paragraphs[0].content = vec![
        old.clone(),
        Inline::CitationCluster(vec![item(3), simple.clone()]),
    ];
    d.paragraphs.push(Paragraph {
        id: 5,
        style: ParagraphStyle::Normal,
        format: ParagraphFormat::default(),
        content: vec![old],
    });
    check!(
        d.source_uses(2)
            == vec![
                CitationLocation {
                    paragraph: 1,
                    inline: 0
                },
                CitationLocation {
                    paragraph: 1,
                    inline: 1
                },
                CitationLocation {
                    paragraph: 5,
                    inline: 0
                }
            ],
        "usage positions retain paragraph and inline document order"
    );
    let size = d.byte_size();
    d.paragraphs[0].content[1] = Inline::CitationCluster(vec![
        CitationItem {
            prefix: "p".repeat(128),
            suffix: "s".repeat(128),
            ..simple.clone()
        },
        item(3),
    ]);
    check!(
        d.byte_size() == size + 256,
        "citation affixes are included in document and history budgets"
    );
    d.paragraphs[0].content[1] = Inline::CitationCluster(vec![]);
    check!(
        d.validate() == Err(Error::Invalid),
        "public raw cluster validation rejects empty fields"
    );
    let mut boundary = Document::default();
    boundary.sources = vec![Source {
        id: 2,
        title: "t".repeat(4094),
        ..Default::default()
    }];
    let repeated = vec![item(2); 32];
    boundary.paragraphs[0].content = vec![Inline::CitationCluster(repeated.clone()); 32];
    boundary.paragraphs[0]
        .content
        .push(Inline::Text(String::new()));
    check!(
        boundary.citation_cluster_text(&repeated).len() == 131072
            && boundary.byte_size() < 100_000
            && boundary.validate().is_ok(),
        "compact clustered citations accept exactly four MiB of expanded document text"
    );
    let mut limit = Editor::new(boundary.clone()).unwrap();
    check!(
        limit.apply(Edit::SetText {
            paragraph: 1,
            inline: 32,
            text: String::from("x")
        }) == Err(Error::Limit)
            && *limit.document() == boundary
            && !limit.dirty()
            && limit.revision() == 0,
        "expanded text beyond four MiB is rejected without a partial edit"
    );
    let mut bibliography_boundary = Document::default();
    bibliography_boundary.sources = boundary.sources;
    bibliography_boundary.paragraphs[0].content = vec![Inline::Citation {
        source: 2,
        locator: String::new(),
    }];
    let row = bibliography_boundary.sources[0].bibliography_text().len();
    let available = (4 << 20) - 4096;
    let copies = available / row;
    bibliography_boundary.paragraphs[0]
        .content
        .extend(vec![Inline::Bibliography; copies]);
    bibliography_boundary.paragraphs[0]
        .content
        .push(Inline::Text("x".repeat(available % row)));
    check!(
        bibliography_boundary.validate().is_ok(),
        "repeated bibliography fields count their complete expansion at the byte boundary"
    );
    bibliography_boundary.paragraphs[0]
        .content
        .push(Inline::Text(String::from("x")));
    check!(
        bibliography_boundary.validate() == Err(Error::Limit),
        "bibliography expansion cannot bypass document display budgets"
    );
    (passed, failed)
}
