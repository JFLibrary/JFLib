// Import-review contracts and deterministic exchange fixtures for native GUI tests.
use alloc::{format, string::String, vec};
use doc_core::{Document, Edit, Editor, Person, Source, SourceKind, SourceMatch, SourceTarget};

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
    let source = Source {
        id: 90,
        title: String::from("Methoden"),
        year: String::from("2026"),
        publisher: String::from("Alpha"),
        authors: vec![Person {
            given: String::from("Grace"),
            family: String::from("Hopper"),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut same = source.clone();
    same.id = 99;
    let mut similar = source.clone();
    similar.id = 100;
    similar.publisher = String::from("Beta");
    let batch = vec![source.clone(), same.clone(), similar.clone()];
    let mut editor = Editor::new(Document::default()).unwrap();
    check!(
        editor.document().review_sources(&batch)
            == Ok(vec![
                SourceMatch::New,
                SourceMatch::Identical(SourceTarget::Import(0)),
                SourceMatch::Similar(SourceTarget::Import(0))
            ]),
        "review detects exact and possible duplicates within an import"
    );
    check!(
        !editor.dirty() && !editor.can_undo() && editor.revision() == 0,
        "review has no document side effects"
    );
    editor
        .apply(Edit::ImportSources(vec![source.clone()]))
        .unwrap();
    let id = editor.document().sources[0].id;
    check!(
        editor.document().review_sources(&batch)
            == Ok(vec![
                SourceMatch::Identical(SourceTarget::Document(id)),
                SourceMatch::Identical(SourceTarget::Document(id)),
                SourceMatch::Similar(SourceTarget::Document(id))
            ]),
        "document matches take priority over import rows"
    );
    check!(
        source.same_content(&same) && !source.same_content(&similar),
        "source identity excluded from exact comparison"
    );
    check!(
        source.matches_query("  grace   HOPPER ")
            && source.matches_query("alpha")
            && !source.matches_query("unknown"),
        "source filtering covers people and metadata"
    );
    let mut unicode = source.clone();
    unicode.title = String::from("Äpfel und Öfen");
    check!(
        unicode.matches_query("äPFEL"),
        "source filtering uses Unicode lowercase"
    );
    let mut url = source.clone();
    url.url = String::from("https://example.org/paper");
    let mut other = url.clone();
    other.title = String::from("Other title");
    check!(
        Document::default().review_sources(&[url, other])
            == Ok(vec![
                SourceMatch::New,
                SourceMatch::Similar(SourceTarget::Import(0))
            ]),
        "shared URL is advisory rather than an automatic merge"
    );
    editor
        .apply(Edit::ImportSources(vec![similar.clone()]))
        .unwrap();
    check!(
        editor.document().sources.len() == 2,
        "similar source kept separate"
    );
    editor.undo();
    let before = editor.document().clone();
    let revision = editor.revision();
    let mut bad = source.clone();
    bad.title.clear();
    check!(
        editor
            .document()
            .review_sources(&[source.clone(), bad])
            .is_err()
            && *editor.document() == before
            && editor.revision() == revision
            && editor.can_redo(),
        "invalid review preserves document and redo"
    );
    check!(
        editor
            .document()
            .review_sources(&vec![source.clone(); 1025])
            .is_err(),
        "review source-count bound"
    );
    editor.apply(Edit::ImportSources(vec![same])).unwrap();
    check!(
        editor.revision() == revision && editor.can_redo(),
        "identical import is not an undo step"
    );
    editor.apply(Edit::ImportSources(vec![similar])).unwrap();
    editor.undo();
    check!(
        *editor.document() == before,
        "selected import remains one reversible transaction"
    );
    check!(
        doc_io::BibliographyFormat::from_path("x.BIBTEX") == Ok(doc_io::BibliographyFormat::Bibtex)
            && doc_io::BibliographyFormat::from_path("x.RIS")
                == Ok(doc_io::BibliographyFormat::Ris)
            && doc_io::BibliographyFormat::from_path("x.csv").is_err(),
        "exchange selection uses explicit extension"
    );
    if base::Args(plat::args()).value("--fixture").is_some() {
        check!(
            doc_io::save_bibliography("build/jfword-exchange.ris", &batch).is_ok(),
            "RIS GUI import fixture"
        );
        let website = Source {
            id: 120,
            kind: SourceKind::Website,
            title: String::from("Offene Forschung"),
            corporate: String::from("Institut"),
            url: String::from("https://example.org"),
            ..Default::default()
        };
        check!(
            doc_io::save_bibliography("build/jfword-exchange.bib", &[source, website]).is_ok(),
            "BibTeX GUI import fixture"
        );
    }
    (passed, failed)
}
