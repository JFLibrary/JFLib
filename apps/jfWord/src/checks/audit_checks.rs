// Native tests for advisory bibliography completeness; new functionality without a reference.
use alloc::{format, string::String, vec, vec::Vec};
use doc_core::{Document, Edit, Editor, Person, Source, SourceIssue, SourceIssueKind, SourceKind};

fn complete(kind: SourceKind) -> Source {
    Source {
        id: 2,
        kind,
        authors: vec![Person {
            family: String::from("Müller"),
            ..Default::default()
        }],
        title: String::from("Methoden"),
        year: String::from("2026"),
        publisher: String::from("Verlag"),
        journal: String::from("Fachzeitschrift"),
        url: String::from("https://example.org/methoden"),
        ..Default::default()
    }
}
fn document(sources: Vec<Source>) -> Document {
    Document {
        sources,
        ..Default::default()
    }
}
fn issue(source: u64, kind: SourceIssueKind) -> SourceIssue {
    SourceIssue { source, kind }
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
    use SourceIssueKind::{
        IncompleteAccessDate, MissingCreators, MissingJournal, MissingPublisher, MissingUrl,
        MissingYear,
    };
    check!(
        Document::default().audit_sources().is_empty(),
        "audit empty source collection"
    );
    for kind in [
        SourceKind::Book,
        SourceKind::JournalArticle,
        SourceKind::Website,
    ] {
        let source = complete(kind);
        check!(
            document(vec![source.clone()]).audit_sources().is_empty(),
            "audit complete source type"
        );
        let mut missing = source;
        missing.authors.clear();
        missing.year.clear();
        missing.publisher.clear();
        missing.journal.clear();
        missing.url.clear();
        let kind_issue = match kind {
            SourceKind::Book => MissingPublisher,
            SourceKind::JournalArticle => MissingJournal,
            SourceKind::Website => MissingUrl,
        };
        let doc = document(vec![missing]);
        check!(
            doc.validate().is_ok()
                && doc.audit_sources()
                    == vec![
                        issue(2, MissingCreators),
                        issue(2, MissingYear),
                        issue(2, kind_issue),
                    ],
            "audit type-specific missing fields remain valid source data"
        );
    }
    let mut corporate = complete(SourceKind::Book);
    corporate.authors.clear();
    corporate.corporate = String::from(" Institut ");
    check!(
        document(vec![corporate.clone()]).audit_sources().is_empty(),
        "audit accepts a corporate author"
    );
    corporate.corporate = String::from(" \t\n\u{2003}");
    corporate.year = String::from(" \t\u{2003}");
    corporate.publisher = String::from(" \n");
    check!(
        document(vec![corporate.clone()]).audit_sources()
            == vec![
                issue(2, MissingCreators),
                issue(2, MissingYear),
                issue(2, MissingPublisher),
            ],
        "audit treats Unicode whitespace-only fields as blank"
    );
    for (year, month, day, incomplete) in [
        ("", "", "", false),
        ("2026", "", "", false),
        ("2026", "10", "", false),
        ("2026", "10", "01", false),
        ("", "10", "", true),
        ("", "", "01", true),
        ("2026", "", "01", true),
        ("", "10", "01", true),
        (" \t", "10", "", true),
        ("2026", "\u{2003}", "01", true),
        (" ", "\t", "\n", false),
        ("year", "month", "day", false),
    ] {
        let mut source = complete(SourceKind::Website);
        source.year_accessed = String::from(year);
        source.month_accessed = String::from(month);
        source.day_accessed = String::from(day);
        let expected = if incomplete {
            vec![issue(2, IncompleteAccessDate)]
        } else {
            Vec::new()
        };
        check!(
            document(vec![source]).audit_sources() == expected,
            "audit checks access-date component relationships only"
        );
    }
    let mut journal = complete(SourceKind::JournalArticle);
    journal.id = 90;
    journal.journal.clear();
    journal.publisher.clear();
    journal.url.clear();
    journal.month_accessed = String::from("10");
    corporate.id = 5;
    let mut website = complete(SourceKind::Website);
    website.id = 70;
    website.url = String::from(" \u{2003}");
    website.journal.clear();
    website.publisher.clear();
    let uncited = document(vec![journal, corporate.clone(), website]);
    check!(
        uncited.validate().is_ok()
            && uncited.audit_sources()
                == vec![
                    issue(90, MissingJournal),
                    issue(90, IncompleteAccessDate),
                    issue(5, MissingCreators),
                    issue(5, MissingYear),
                    issue(5, MissingPublisher),
                    issue(70, MissingUrl),
                ],
        "audit includes uncited sources in document order with stable IDs"
    );
    let before = uncited.clone();
    let first = uncited.audit_sources();
    check!(
        first == uncited.audit_sources() && uncited == before,
        "audit is deterministic and leaves document metadata unchanged"
    );
    let mut editor = Editor::new(uncited).unwrap();
    let initial_revision = editor.revision();
    let _ = editor.document().audit_sources();
    check!(
        !editor.dirty()
            && !editor.can_undo()
            && !editor.can_redo()
            && editor.revision() == initial_revision,
        "audit leaves a clean editor and its empty history unchanged"
    );
    editor
        .apply(Edit::SetText {
            paragraph: 1,
            inline: 0,
            text: String::from("Erster Text"),
        })
        .unwrap();
    editor
        .apply(Edit::SetText {
            paragraph: 1,
            inline: 0,
            text: String::from("Zweiter Text"),
        })
        .unwrap();
    editor.undo();
    let revision = editor.revision();
    let state = editor.document().clone();
    let audit = editor.document().audit_sources();
    check!(
        audit == first
            && *editor.document() == state
            && editor.revision() == revision
            && editor.dirty()
            && editor.can_undo()
            && editor.can_redo(),
        "audit preserves dirty state, revision and both history directions"
    );
    check!(
        editor.redo()
            && editor.document().plain_text() == "Zweiter Text"
            && editor.undo()
            && *editor.document() == state,
        "audit preserves undo and redo payloads"
    );
    editor.mark_saved();
    let revision = editor.revision();
    let _ = editor.document().audit_sources();
    check!(
        !editor.dirty() && editor.revision() == revision && editor.can_redo(),
        "audit preserves save point and pending redo"
    );
    corporate.id = 0;
    check!(
        editor.apply(Edit::PutSource(corporate)).is_ok(),
        "completeness hints do not prevent saving a source"
    );
    let max = document(
        (2..1026)
            .map(|id| Source {
                id,
                title: String::from("Titel"),
                month_accessed: String::from("10"),
                ..Default::default()
            })
            .collect(),
    );
    check!(
        max.validate().is_ok()
            && max.audit_sources().len() == 4096
            && max.audit_sources().len() <= 6 * 1024,
        "audit result count stays bounded for a full valid source collection"
    );
    (passed, failed)
}
