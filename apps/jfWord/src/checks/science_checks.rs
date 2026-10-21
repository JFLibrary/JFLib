// Scientific-source regression checks against native model and codecs, without a test runtime.
use alloc::{format, string::String, vec, vec::Vec};
use doc_core::{Document, Edit, Editor, Person, Source, SourceKind};

pub(crate) fn sources() -> Vec<Source> {
    vec![
        Source {
            id: 2,
            title: String::from("Methoden & Modelle"),
            year: String::from("2026"),
            authors: vec![
                Person {
                    given: String::from("Ada"),
                    family: String::from("Lovelace"),
                    ..Default::default()
                },
                Person {
                    given: String::from("Grace"),
                    middle: String::from("B."),
                    family: String::from("Hopper"),
                },
            ],
            publisher: String::from("Forschungsverlag"),
            city: String::from("Berlin"),
            edition: String::from("2"),
            standard_number: String::from("978-1-23456-789-0"),
            ..Default::default()
        },
        Source {
            id: 3,
            kind: SourceKind::JournalArticle,
            title: String::from("Reproduzierbare Analyse"),
            authors: vec![
                Person {
                    family: String::from("Müller"),
                    ..Default::default()
                },
                Person {
                    family: String::from("李"),
                    ..Default::default()
                },
                Person {
                    family: String::from("García"),
                    ..Default::default()
                },
            ],
            year: String::from("2025"),
            journal: String::from("Journal of Methods"),
            volume: String::from("12"),
            issue: String::from("3"),
            pages: String::from("41–58"),
            url: String::from("https://doi.org/10.0000/example"),
            external_id: String::from("{12345678-1234-1234-1234-123456789ABC}"),
            language: String::from("1031"),
            short_title: String::from("Analyse"),
            comments: String::from("Unabhängige Replikation\nα β"),
            ..Default::default()
        },
        Source {
            id: 4,
            kind: SourceKind::Website,
            corporate: String::from("Institut für offene Forschung"),
            title: String::from("Offene Daten"),
            website: String::from("Forschungsportal"),
            year: String::from("2024"),
            url: String::from("https://example.org/data?a=1&b=2"),
            year_accessed: String::from("2026"),
            month_accessed: String::from("10"),
            day_accessed: String::from("10"),
            ..Default::default()
        },
    ]
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
    let sources = sources();
    let mut e = Editor::new(Document::default()).unwrap();
    check!(
        e.apply(Edit::ImportSources(sources.clone())).is_ok(),
        "atomic source import"
    );
    check!(
        e.document().sources == sources,
        "source IDs allocated from document"
    );
    e.mark_saved();
    check!(
        e.apply(Edit::ImportSources(sources.clone())) == Ok(0) && !e.dirty(),
        "repeat import is a no-op"
    );
    for source in &sources {
        e.apply(Edit::Cite {
            paragraph: 1,
            source: source.id,
            locator: String::new(),
        })
        .unwrap();
    }
    e.apply(Edit::InsertBibliography { after: 1 }).unwrap();
    let d = e.document().clone();
    check!(
        d.citation_text(2, "9") == "(Lovelace & Hopper, 2026, S. 9)",
        "two-author citation order"
    );
    check!(
        d.citation_text(3, "") == "(Müller et al., 2025)",
        "multi-author short citation"
    );
    check!(
        d.citation_text(4, "") == "(Institut für offene Forschung, 2024)",
        "corporate citation"
    );
    check!(
        d.bibliography()[1].contains("12(3), S. 41–58") && d.bibliography()[1].contains("李"),
        "journal metadata and all authors displayed"
    );
    check!(
        d.bibliography()[2].contains("Zugriff: 2026-10-10"),
        "access date displayed"
    );
    let bytes = doc_io::encode_docx(&d).unwrap();
    check!(
        doc_io::decode_docx(&bytes) == Ok(d.clone()),
        "all source kinds metadata and fields DOCX round trip"
    );
    let list = doc_io::encode_sources(&sources).unwrap();
    let imported = doc_io::decode_sources(&list).unwrap();
    let mut other = Editor::new(Document::default()).unwrap();
    other.apply(Edit::ImportSources(imported)).unwrap();
    check!(
        other.document().sources == sources,
        "standalone sources preserve content"
    );
    check!(
        other.undo() && other.document().sources.is_empty() && !other.can_undo(),
        "whole import undone in one step"
    );
    let mut invalid = sources.clone();
    invalid[2].title.clear();
    check!(
        other.apply(Edit::ImportSources(invalid)).is_err()
            && other.document().sources.is_empty()
            && other.can_redo(),
        "failed batch preserves document and redo"
    );
    check!(
        other.redo() && other.document().sources == sources,
        "redo full source import"
    );
    let mut changed = sources[0].clone();
    changed.authors.swap(0, 1);
    e.apply(Edit::PutSource(changed)).unwrap();
    check!(
        e.document().citation_text(2, "") == "(Hopper & Lovelace, 2026)",
        "author reorder refreshes fields"
    );
    e.undo();
    check!(
        *e.document() == d,
        "author reorder undo preserves full document"
    );
    let mut mixed = sources[0].clone();
    mixed.corporate = String::from("Mixed");
    check!(
        mixed.validate().is_err(),
        "mixed corporation and persons rejected"
    );
    let mut large = sources[0].clone();
    large.authors = vec![large.authors[0].clone(); 65];
    check!(large.validate().is_err(), "author limit");
    let mut long = sources[0].clone();
    long.url = "x".repeat(4097);
    check!(long.validate().is_err(), "metadata field limit");
    let mut controls = sources[0].clone();
    controls.comments.push('\0');
    check!(
        controls.validate().is_err(),
        "metadata XML characters validated"
    );
    let mut duplicate = sources.clone();
    duplicate[1].id = duplicate[0].id;
    check!(
        doc_io::encode_sources(&duplicate).is_err(),
        "duplicate export identity rejected"
    );
    let before = other.document().clone();
    let mut many = sources[0].clone();
    many.id = 1;
    let many = (0..1024)
        .map(|n| {
            let mut s = many.clone();
            s.title = format!("New {n}");
            s
        })
        .collect();
    check!(
        other.apply(Edit::ImportSources(many)).is_err() && *other.document() == before,
        "overfull batch rejected atomically"
    );
    if base::Args(plat::args()).value("--fixture").is_some() {
        check!(
            doc_io::save_sources("build/jfword-ui-sources.xml", &sources).is_ok(),
            "source GUI fixture"
        );
        check!(
            doc_io::save_docx("build/jfword-science.docx", &d).is_ok(),
            "science DOCX fixture"
        );
        check!(
            plat::write_file("build/jfword-invalid-sources.xml", b"<invalid/>"),
            "invalid source GUI fixture"
        );
    }
    (passed, failed)
}

pub(crate) fn probe(path: &str, output: Option<&str>, docx: Option<&str>, reject: bool) -> i32 {
    let result = (|| {
        let sources = doc_io::load_bibliography(path)?;
        let mut editor = Editor::new(Document::default())?;
        editor.apply(Edit::ImportSources(sources))?;
        let output_bytes = if let Some(path) = output {
            Some(doc_io::encode_bibliography(
                &editor.document().sources,
                doc_io::BibliographyFormat::from_path(path)?,
            )?)
        } else {
            None
        };
        let docx_bytes = if docx.is_some() {
            let ids = editor
                .document()
                .sources
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>();
            for source in ids {
                editor.apply(Edit::Cite {
                    paragraph: 1,
                    source,
                    locator: String::new(),
                })?;
            }
            editor.apply(Edit::InsertBibliography { after: 1 })?;
            Some(doc_io::encode_docx(editor.document())?)
        } else {
            None
        };
        // Preflight all requested codecs before writing any output. Rejection probes are read-only.
        if !reject {
            for (path, bytes) in [(output, output_bytes.as_ref()), (docx, docx_bytes.as_ref())] {
                if let (Some(path), Some(bytes)) = (path, bytes) {
                    if !plat::write_file(path, bytes) {
                        return Err(doc_io::Error::Io);
                    }
                }
            }
        }
        Ok::<(), doc_io::Error>(())
    })();
    match result {
        Ok(()) if !reject => {
            plat::write_stdout(b"PASS source list\n");
            0
        }
        Err(e) if reject => {
            plat::write_stdout(format!("PASS rejected {e:?}\n").as_bytes());
            0
        }
        other => {
            plat::write_stdout(format!("FAIL source list {other:?}\n").as_bytes());
            1
        }
    }
}
