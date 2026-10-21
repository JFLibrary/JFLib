// Native source reconciliation contracts, including history, expansion and DOCX persistence.
use alloc::{format, string::String, vec};
use doc_core::{
    CitationItem, Document, Edit, Editor, Error, Inline, Paragraph, ParagraphFormat,
    ParagraphStyle, Person, Source, SourceCandidate, SourceField, SourceKind, SourceMatch,
    SourceMerge, SourceTarget,
};

fn person() -> Person {
    Person {
        given: String::from("Ada"),
        middle: String::from("Augusta"),
        family: String::from("Lovelace"),
    }
}
fn item(source: u64) -> CitationItem {
    CitationItem {
        source,
        ..Default::default()
    }
}
fn incoming() -> Source {
    Source {
        id: 3,
        kind: SourceKind::JournalArticle,
        authors: vec![person()],
        corporate: String::new(),
        title: String::from("Neue Methoden"),
        year: String::from("2026"),
        publisher: String::from("Forschungsverlag"),
        city: String::from("Köln"),
        edition: String::from("2"),
        journal: String::from("Methodenhefte"),
        volume: String::from("8"),
        issue: String::from("3"),
        pages: String::from("40–57"),
        website: String::from("Forschungsportal"),
        url: String::from("https://example.org/methoden"),
        standard_number: String::from("ISSN 0000-0000"),
        year_accessed: String::from("2026"),
        month_accessed: String::from("10"),
        day_accessed: String::from("10"),
        short_title: String::from("Methoden"),
        comments: String::new(),
        external_id: String::from("methods-2026"),
        language: String::from("de"),
    }
}
fn document() -> Document {
    let mut d = Document::default();
    let kept = Source {
        id: 2,
        corporate: String::from("Institut"),
        title: String::from("Methoden"),
        year: String::from("2024"),
        url: String::from("https://example.org/methoden"),
        comments: String::from("Eigene Anmerkung"),
        ..Default::default()
    };
    let similar = Source {
        id: 4,
        corporate: String::from("Andere Autoren"),
        title: String::from("  METHODEN  "),
        year: String::from(" 2024 "),
        ..Default::default()
    };
    let mut identical = kept.clone();
    identical.id = 5;
    d.sources = vec![kept, incoming(), similar, identical];
    d.paragraphs[0].content = vec![
        Inline::Text(String::from("Vorher ")),
        Inline::Citation {
            source: 3,
            locator: String::from("4"),
        },
        Inline::CitationCluster(vec![
            CitationItem {
                locator: String::from("8–10"),
                prefix: String::from("vgl. "),
                suffix: String::from("; ergänzt"),
                suppress_author: true,
                ..item(3)
            },
            item(2),
            CitationItem {
                suppress_year: true,
                suppress_title: true,
                ..item(3)
            },
            item(4),
        ]),
        Inline::Text(String::from(" Nachher")),
    ];
    d.paragraphs.push(Paragraph {
        id: 10,
        style: ParagraphStyle::Heading1,
        format: ParagraphFormat::default(),
        content: vec![Inline::Citation {
            source: 3,
            locator: String::from("12"),
        }],
    });
    d.paragraphs.push(Paragraph {
        id: 11,
        style: ParagraphStyle::Normal,
        format: ParagraphFormat::default(),
        content: vec![Inline::Bibliography],
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
    let original = document();
    check!(
        original.cited_source_ids() == vec![3, 2, 4]
            && Document::default().cited_source_ids().is_empty(),
        "cited IDs preserve first use and exclude repeated or uncited sources"
    );
    let fields = SourceField::ALL;
    check!(
        fields.len() == 21
            && fields
                .iter()
                .enumerate()
                .all(|(i, field)| !fields[..i].contains(field)),
        "merge selectable fields are complete and unique"
    );
    check!(
        original.source_candidates(2)
            == Ok(vec![
                SourceCandidate {
                    source: 3,
                    identical: false
                },
                SourceCandidate {
                    source: 4,
                    identical: false
                },
                SourceCandidate {
                    source: 5,
                    identical: true
                },
            ])
            && original.source_candidates(99) == Err(Error::Missing),
        "duplicate candidates preserve document order and distinguish exact from similar"
    );
    let mut unrelated = original.clone();
    unrelated.sources.push(Source {
        id: 6,
        kind: SourceKind::Website,
        title: String::from("Methoden"),
        year: String::from("2024"),
        ..Default::default()
    });
    check!(
        unrelated.source_candidates(2) == original.source_candidates(2),
        "matching title and year alone do not equate different source categories"
    );
    let plan = original.plan_source_merge(2, 3).unwrap();
    check!(
        plan.incoming
            == fields
                .into_iter()
                .filter(|field| {
                    original.sources[0].field_empty(*field)
                        && !original.sources[1].field_empty(*field)
                })
                .collect::<alloc::vec::Vec<_>>()
            && !plan.incoming.contains(&SourceField::Kind)
            && !plan.incoming.contains(&SourceField::Creators)
            && !plan.incoming.contains(&SourceField::Comments),
        "merge suggestions fill only absent metadata and retain explicit conflicts"
    );
    let preview = original.preview_source_merge(&plan).unwrap();
    check!(
        preview.id == 2
            && preview.kind == SourceKind::Book
            && preview.corporate == "Institut"
            && preview.authors.is_empty()
            && preview.title == "Methoden"
            && preview.year == "2024"
            && preview.publisher == "Forschungsverlag"
            && preview.comments == "Eigene Anmerkung"
            && original == document(),
        "merge preview retains selected identity and never mutates the document"
    );
    let all = SourceMerge {
        keep: 2,
        remove: 3,
        incoming: fields.to_vec(),
    };
    let mut expected = incoming();
    expected.id = 2;
    check!(
        original.preview_source_merge(&all) == Ok(expected.clone())
            && fields
                .iter()
                .all(|field| expected.field_equals(&incoming(), *field)),
        "all source fields can be explicitly replaced without changing kept ID"
    );
    let creators = SourceMerge {
        keep: 2,
        remove: 3,
        incoming: vec![SourceField::Creators],
    };
    let replaced = original.preview_source_merge(&creators).unwrap();
    check!(
        replaced.authors == vec![person()]
            && replaced.corporate.is_empty()
            && replaced.field_text(SourceField::Creators) == "Lovelace, Ada Augusta"
            && replaced.field_text(SourceField::Kind) == "Book",
        "creator merge replaces corporate and ordered personal modes atomically"
    );
    let reverse = original
        .preview_source_merge(&SourceMerge {
            keep: 3,
            remove: 2,
            incoming: vec![SourceField::Creators, SourceField::Publisher],
        })
        .unwrap();
    check!(
        reverse.authors.is_empty()
            && reverse.corporate == "Institut"
            && reverse.publisher.is_empty(),
        "reverse creator merge and explicitly selected empty metadata are preserved"
    );
    let mut anonymous = original.clone();
    anonymous.sources[1].authors.clear();
    let anonymous_preview = anonymous.preview_source_merge(&creators).unwrap();
    check!(
        anonymous_preview.authors.is_empty() && anonymous_preview.corporate.is_empty(),
        "explicitly empty creators clear both creator modes together"
    );
    let mut display_equal = replaced.clone();
    display_equal.authors.clear();
    display_equal.corporate = replaced.author_text();
    check!(
        display_equal.field_text(SourceField::Creators)
            == replaced.field_text(SourceField::Creators)
            && !display_equal.field_equals(&replaced, SourceField::Creators),
        "creator comparison distinguishes identical display text with different structure"
    );
    let mut blank = Source::default();
    blank.title = String::from(" ");
    check!(
        !blank.field_empty(SourceField::Kind)
            && blank.field_empty(SourceField::Creators)
            && !blank.field_empty(SourceField::Title)
            && blank.field_empty(SourceField::Year),
        "merge absence detection retains intentional whitespace data"
    );
    let mut e = Editor::new(original.clone()).unwrap();
    check!(
        e.apply(Edit::MergeSources(all.clone())) == Ok(2),
        "merge returns kept source ID"
    );
    let merged = e.document().clone();
    check!(
        merged.sources
            == vec![
                expected.clone(),
                original.sources[2].clone(),
                original.sources[3].clone()
            ]
            && merged.source(3).is_none()
            && merged.source_uses(3).is_empty()
            && merged.source_uses(2).len() == 3,
        "merge removes only the redundant record and redirects every citation field"
    );
    let mut remapped = original.paragraphs.clone();
    remapped[0].content[1] = Inline::Citation {
        source: 2,
        locator: String::from("4"),
    };
    if let Inline::CitationCluster(items) = &mut remapped[0].content[2] {
        items[0].source = 2;
        items[2].source = 2;
    }
    remapped[1].content[0] = Inline::Citation {
        source: 2,
        locator: String::from("12"),
    };
    check!(
        merged.paragraphs == remapped,
        "source remapping preserves duplicate items, order, locator, affixes and suppression"
    );
    check!(
        merged.bibliography()
            == vec![
                expected.bibliography_text(),
                original.sources[2].bibliography_text()
            ],
        "merged bibliography has one row per cited source in first-use order"
    );
    check!(
        e.dirty()
            && e.undo()
            && !e.dirty()
            && *e.document() == original
            && e.redo()
            && *e.document() == merged,
        "merge undo restores complete original sources, references and saved checkpoint"
    );
    e.mark_saved();
    check!(
        !e.dirty() && e.undo() && e.dirty() && e.redo() && !e.dirty(),
        "merge history respects a checkpoint saved after merging"
    );
    e.undo();
    let revision = e.revision();
    for invalid in [
        SourceMerge {
            keep: 2,
            remove: 2,
            incoming: vec![],
        },
        SourceMerge {
            keep: 2,
            remove: 99,
            incoming: vec![],
        },
        SourceMerge {
            keep: 99,
            remove: 3,
            incoming: vec![],
        },
        SourceMerge {
            keep: 2,
            remove: 3,
            incoming: vec![SourceField::Year, SourceField::Year],
        },
        SourceMerge {
            keep: 2,
            remove: 3,
            incoming: vec![SourceField::Year; 22],
        },
    ] {
        check!(
            original.preview_source_merge(&invalid).is_err()
                && e.apply(Edit::MergeSources(invalid)).is_err()
                && *e.document() == original
                && e.revision() == revision
                && e.can_redo()
                && e.dirty(),
            "invalid merge remains atomic and preserves redo and saved state"
        );
    }
    check!(
        original.plan_source_merge(2, 2) == Err(Error::Invalid)
            && original.plan_source_merge(2, 99) == Err(Error::Missing),
        "merge planning rejects equal and missing source identities"
    );
    e.redo();
    check!(
        e.apply(Edit::PutSource(Source {
            title: String::from("Weitere Quelle"),
            ..Default::default()
        })) == Ok(12)
            && e.undo()
            && e.apply(Edit::PutSource(Source {
                title: String::from("Noch eine Quelle"),
                ..Default::default()
            })) == Ok(13),
        "merge and undo never recycle allocated source identities"
    );
    let mut backwards = Editor::new(original.clone()).unwrap();
    backwards
        .apply(Edit::MergeSources(SourceMerge {
            keep: 3,
            remove: 2,
            incoming: vec![],
        }))
        .unwrap();
    check!(
        backwards.document().sources == original.sources[1..]
            && backwards.document().source_uses(2).is_empty()
            && backwards.document().source_uses(3).len() == 3,
        "merge retains source order when the removed record precedes the kept record"
    );
    check!(
        original.review_sources(&[original.sources[0].clone()])
            == Ok(vec![SourceMatch::Identical(SourceTarget::Document(2))]),
        "explicit source reconciliation leaves duplicate import classification unchanged"
    );
    let encoded = doc_io::encode_docx(&merged).unwrap();
    check!(
        doc_io::decode_docx(&encoded) == Ok(merged.clone()),
        "merged metadata and repeated clustered citations survive DOCX roundtrip"
    );
    let mut crowded = Document::default();
    crowded.sources = (2..=1025)
        .map(|id| Source {
            id,
            title: String::from("Gleich"),
            ..Default::default()
        })
        .collect();
    check!(
        crowded.source_candidates(2).unwrap().len() == 1023,
        "candidate enumeration accepts the bounded document source maximum"
    );
    crowded.sources.push(Source {
        id: 1026,
        title: String::from("Zu viel"),
        ..Default::default()
    });
    check!(
        crowded.source_candidates(2) == Err(Error::Limit),
        "candidate enumeration rejects an over-budget public document"
    );
    let mut boundary = Document::default();
    boundary.sources = vec![
        Source {
            id: 2,
            title: String::from("Kurz"),
            ..Default::default()
        },
        Source {
            id: 3,
            title: "x".repeat(4094),
            ..Default::default()
        },
    ];
    boundary.paragraphs[0].content = vec![Inline::CitationCluster(vec![item(2); 32]); 32];
    boundary.paragraphs[0].content.push(Inline::Citation {
        source: 2,
        locator: String::new(),
    });
    let overflow = SourceMerge {
        keep: 2,
        remove: 3,
        incoming: vec![SourceField::Title],
    };
    let mut bounded = Editor::new(boundary.clone()).unwrap();
    check!(
        boundary.preview_source_merge(&overflow) == Err(Error::Limit)
            && bounded.apply(Edit::MergeSources(overflow)) == Err(Error::Limit)
            && *bounded.document() == boundary
            && bounded.revision() == 0
            && !bounded.dirty()
            && !bounded.can_undo(),
        "merge expansion overflow is rejected in preview and transaction without mutations"
    );
    if base::Args(plat::args()).value("--fixture").is_some() {
        check!(
            doc_io::save_docx("build/jfword-library-input.docx", &original).is_ok(),
            "library GUI input fixture"
        );
        let library_test_path = "build/jfword-library-ui/sources.jflib";
        if plat::file_exists("build/jfword-library-copy.docx") == 1 {
            check!(
                plat::delete("build/jfword-library-copy.docx"),
                "reset isolated library document fixture"
            );
        }
        for suffix in ["", ".bak", ".damaged"] {
            let path = format!("{library_test_path}{suffix}");
            if plat::file_exists(&path) == 1 {
                check!(plat::delete(&path), "reset isolated library GUI fixture");
            }
        }
        check!(
            doc_io::save_docx("build/jfword-merged.docx", &merged).is_ok(),
            "merged source DOCX independent fixture"
        );
        check!(
            doc_io::save_docx("build/jfword-merge-input.docx", &original).is_ok(),
            "source reconciliation GUI input fixture"
        );
    }
    (passed, failed)
}
