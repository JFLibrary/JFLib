// Native source-library identity, isolation, transaction and history contracts.
use alloc::{format, string::String, vec, vec::Vec};
use doc_core::{
    Document, Edit, Editor, Error, LibraryEdit, Person, Source, SourceField, SourceKind,
    SourceLibrary, SourceLibraryEditor,
};

fn source(id: u64, title: &str) -> Source {
    Source {
        id,
        title: String::from(title),
        corporate: String::from("Institut"),
        year: String::from("2026"),
        ..Default::default()
    }
}
fn populated(id: u64) -> Source {
    Source {
        id,
        kind: SourceKind::Website,
        authors: vec![Person {
            given: String::from("Ada"),
            middle: String::from("Augusta"),
            family: String::from("Lovelace"),
        }],
        corporate: String::new(),
        title: String::from("Analytical notes"),
        year: String::from("1843"),
        publisher: String::from("Publisher"),
        city: String::from("London"),
        edition: String::from("2"),
        journal: String::from("Journal"),
        volume: String::from("3"),
        issue: String::from("4"),
        pages: String::from("10–20"),
        website: String::from("Archives"),
        url: String::from("https://example.org/notes"),
        standard_number: String::from("ISBN 000"),
        year_accessed: String::from("2026"),
        month_accessed: String::from("10"),
        day_accessed: String::from("11"),
        short_title: String::from("Notes"),
        comments: String::from("Independent metadata"),
        external_id: String::from("notes-1843"),
        language: String::from("en"),
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
                plat::write_stdout(format!("FAIL library: {}\n", $name).as_bytes());
            }
        };
    }
    let empty = SourceLibrary::default();
    check!(
        empty.sources.is_empty() && empty.next_id == 1 && empty.validate().is_ok(),
        "empty library has first allocation ID"
    );
    check!(empty.source(1).is_none(), "empty lookup has no source");
    for next_id in [0, 0x8000_0000, u64::MAX] {
        check!(
            SourceLibraryEditor::new(SourceLibrary {
                sources: Vec::new(),
                next_id
            })
            .is_err(),
            "invalid allocation watermark rejected"
        );
    }
    for id in [0, 2, 0x7fff_ffff, u64::MAX] {
        check!(
            SourceLibrary {
                sources: vec![source(id, "A")],
                next_id: 2
            }
            .validate()
                == Err(Error::Invalid),
            "invalid/out-of-range source identity rejected"
        );
    }
    check!(
        SourceLibrary {
            sources: vec![source(1, "A"), source(1, "B")],
            next_id: 2
        }
        .validate()
            == Err(Error::Invalid),
        "duplicate local IDs rejected"
    );
    check!(
        SourceLibrary {
            sources: vec![source(1, " ")],
            next_id: 2
        }
        .validate()
            == Err(Error::Invalid),
        "invalid source metadata rejected"
    );

    let mut editor = SourceLibraryEditor::new(empty.clone()).unwrap();
    check!(
        !editor.undo() && !editor.redo() && editor.revision() == 0,
        "empty history operations do not change revision"
    );
    check!(
        editor.apply(LibraryEdit::Import(Vec::new())) == Ok(0) && !editor.can_undo(),
        "empty import is a no-op"
    );
    check!(
        editor.apply(LibraryEdit::Import(vec![
            source(500, "A"),
            source(500, "B"),
            source(u64::MAX, "A")
        ])) == Ok(1),
        "import ignores external IDs and returns final duplicate ID"
    );
    check!(
        editor
            .library()
            .sources
            .iter()
            .map(|source| source.id)
            .collect::<Vec<_>>()
            == vec![1, 2]
            && editor.library().next_id == 3,
        "import allocates ordered library-local IDs"
    );
    check!(
        editor.revision() == 1 && editor.can_undo(),
        "batch import forms one transaction"
    );
    check!(
        editor.undo() && editor.library().sources.is_empty() && editor.library().next_id == 3,
        "undo retains identity watermark"
    );
    let revision = editor.revision();
    check!(
        editor.apply(LibraryEdit::Import(Vec::new())) == Ok(0)
            && editor.can_redo()
            && editor.revision() == revision,
        "empty import preserves redo and revision"
    );
    check!(
        editor.redo()
            && editor.library().source(1).unwrap().title == "A"
            && editor.library().source(2).unwrap().title == "B",
        "redo restores stable source identities"
    );
    let before = editor.library().clone();
    let revision = editor.revision();
    check!(
        editor.apply(LibraryEdit::Import(vec![source(0, "A"), source(0, "B")])) == Ok(2)
            && *editor.library() == before
            && editor.revision() == revision,
        "exact duplicate import returns existing ID without history edit"
    );
    check!(
        editor.apply(LibraryEdit::Put(source(0, "A"))) == Ok(3)
            && editor.library().sources.len() == 3,
        "explicit creation retains intentional duplicate"
    );
    check!(
        editor.undo() && editor.library().next_id == 4,
        "undo explicit creation does not recycle ID"
    );
    let revision = editor.revision();
    check!(
        editor.apply(LibraryEdit::Put(source(1, "A"))) == Ok(1)
            && editor.can_redo()
            && editor.revision() == revision,
        "unchanged existing source preserves redo and revision"
    );
    check!(
        editor.apply(LibraryEdit::Put(source(50, "Missing"))) == Err(Error::Missing)
            && editor.can_redo()
            && editor.revision() == revision,
        "unknown nonzero source cannot insert implicitly"
    );
    check!(
        editor.apply(LibraryEdit::Remove(50)) == Err(Error::Missing)
            && editor.can_redo()
            && editor.revision() == revision,
        "missing removal is atomic"
    );
    let before = editor.library().clone();
    check!(
        editor.apply(LibraryEdit::Import(vec![source(0, "New"), source(0, "")]))
            == Err(Error::Invalid)
            && *editor.library() == before
            && editor.can_redo(),
        "invalid import batch changes neither sources nor allocator/history"
    );
    check!(
        editor.apply(LibraryEdit::Put(source(0, "D"))) == Ok(4) && !editor.can_redo(),
        "branch after undo consumes fresh ID and clears redo"
    );
    check!(
        editor.apply(LibraryEdit::Put(source(1, "Edited A"))) == Ok(1)
            && editor.library().next_id == 5,
        "existing edit retains identity and watermark"
    );
    check!(
        editor.undo()
            && editor.library().source(1).unwrap().title == "A"
            && editor.redo()
            && editor.library().source(1).unwrap().title == "Edited A",
        "metadata edits undo and redo atomically"
    );
    check!(
        editor.apply(LibraryEdit::Remove(2)) == Ok(2)
            && editor.library().source(2).is_none()
            && editor.library().next_id == 5,
        "source removal retains watermark"
    );
    check!(
        editor.undo()
            && editor.library().source(2).is_some()
            && editor.redo()
            && editor.library().source(2).is_none(),
        "source removal supports complete history"
    );
    let mut restarted = SourceLibraryEditor::new(editor.library().clone()).unwrap();
    check!(
        restarted.revision() == 0
            && !restarted.can_undo()
            && restarted.apply(LibraryEdit::Put(source(0, "Restart"))) == Ok(5),
        "serialized model restart retains allocator but not transient history"
    );
    let mut isolated = editor.clone();
    check!(
        isolated.apply(LibraryEdit::Remove(1)).is_ok() && editor.library().source(1).is_some(),
        "editor clones are independent for prepared durable transactions"
    );

    let document_source = editor.library().source(1).unwrap().clone();
    let mut document = Editor::new(Document::default()).unwrap();
    let document_id = document
        .apply(Edit::ImportSources(vec![document_source]))
        .unwrap();
    check!(
        document_id != 1 && document.document().source(document_id).unwrap().title == "Edited A",
        "library copy into document obtains document-local identity"
    );
    check!(
        editor
            .apply(LibraryEdit::Put(source(1, "Library only")))
            .is_ok()
            && document.document().source(document_id).unwrap().title == "Edited A",
        "library edits cannot mutate document copies"
    );
    check!(
        document
            .apply(Edit::PutSource(source(document_id, "Document only")))
            .is_ok()
            && editor.library().source(1).unwrap().title == "Library only",
        "document edits cannot mutate library copies"
    );

    let mut exhausted = SourceLibraryEditor::new(SourceLibrary {
        sources: vec![source(0x7fff_fffd, "Before last")],
        next_id: 0x7fff_fffe,
    })
    .unwrap();
    check!(
        exhausted.apply(LibraryEdit::Put(source(0, "Last"))) == Ok(0x7fff_fffe)
            && exhausted.library().next_id == 0x7fff_ffff,
        "last valid ID allocates exhausted sentinel"
    );
    check!(
        exhausted.apply(LibraryEdit::Import(vec![source(0, "Last")])) == Ok(0x7fff_fffe),
        "exhausted allocator still deduplicates existing source"
    );
    check!(
        exhausted.undo() && exhausted.library().next_id == 0x7fff_ffff,
        "undo at exhaustion preserves watermark"
    );
    let before = exhausted.library().clone();
    let revision = exhausted.revision();
    check!(
        exhausted.apply(LibraryEdit::Put(source(0, "Never reused"))) == Err(Error::Limit)
            && *exhausted.library() == before
            && exhausted.revision() == revision
            && exhausted.can_redo(),
        "exhausted allocation remains atomic after undo"
    );
    check!(
        exhausted.redo()
            && exhausted.apply(LibraryEdit::Remove(0x7fff_fffe)).is_ok()
            && exhausted.library().next_id == 0x7fff_ffff,
        "exhausted libraries still support redo/removal"
    );

    let full = SourceLibrary {
        sources: (1..=1024).map(|id| source(id, &format!("S{id}"))).collect(),
        next_id: 1025,
    };
    check!(full.validate().is_ok(), "1024 sources fit count budget");
    let mut bounded = SourceLibraryEditor::new(full.clone()).unwrap();
    check!(
        bounded.apply(LibraryEdit::Import(vec![source(7000, "S1")])) == Ok(1),
        "full library can import exact duplicate"
    );
    check!(
        bounded.apply(LibraryEdit::Put(source(0, "New"))) == Err(Error::Limit)
            && *bounded.library() == full,
        "full library creation is atomic"
    );
    check!(
        bounded.apply(LibraryEdit::Remove(1024)).is_ok(),
        "removal frees count capacity"
    );
    let before = bounded.library().clone();
    check!(
        bounded.apply(LibraryEdit::Import(vec![source(0, "X"), source(0, "Y")]))
            == Err(Error::Limit)
            && *bounded.library() == before,
        "batch overflow rolls back prior allocations"
    );
    check!(
        bounded.apply(LibraryEdit::Import(vec![source(0, "S1"); 1025])) == Err(Error::Limit),
        "oversized duplicate import is bounded before deduplication"
    );
    check!(
        bounded.apply(LibraryEdit::Put(source(0, "New"))) == Ok(1025),
        "failed import consumes no IDs"
    );

    let mut history = SourceLibraryEditor::new(SourceLibrary::default()).unwrap();
    history
        .apply(LibraryEdit::Put(source(0, "History")))
        .unwrap();
    for i in 0..80 {
        history
            .apply(LibraryEdit::Put(source(1, &format!("History {i}"))))
            .unwrap();
    }
    let mut undo_count = 0;
    while history.undo() {
        undo_count += 1;
    }
    check!(
        undo_count == 64 && history.library().source(1).unwrap().title == "History 15",
        "history retains exactly the latest 64 small transactions"
    );
    let mut redo_count = 0;
    while history.redo() {
        redo_count += 1;
    }
    check!(
        redo_count == 64 && history.library().source(1).unwrap().title == "History 79",
        "bounded history redo restores latest content"
    );

    let large = SourceLibrary {
        sources: (1..=300)
            .map(|id| {
                let mut value = source(id, &format!("Large {id}"));
                value.comments = "x".repeat(4096);
                value
            })
            .collect(),
        next_id: 301,
    };
    check!(
        large.validate().is_ok(),
        "large metadata collection fits payload budget"
    );
    let mut large_editor = SourceLibraryEditor::new(large).unwrap();
    for i in 0..12 {
        let mut value = large_editor.library().source(1).unwrap().clone();
        value.year = format!("{:04}", 2000 + i);
        large_editor.apply(LibraryEdit::Put(value)).unwrap();
    }
    let retained = (8 << 20) / large_editor.library().byte_size();
    let mut undo_count = 0;
    while large_editor.undo() {
        undo_count += 1;
    }
    check!(
        undo_count == retained && retained < 12,
        "history byte budget trims old large snapshots"
    );
    let before = large_editor.library().clone();
    let more: Vec<_> = (1..=200)
        .map(|id| {
            let mut value = source(0, &format!("Incoming {id}"));
            value.comments = "y".repeat(4096);
            value
        })
        .collect();
    check!(
        large_editor.apply(LibraryEdit::Import(more)) == Err(Error::Limit)
            && *large_editor.library() == before
            && large_editor.can_redo(),
        "combined payload overflow is atomic and preserves redo"
    );
    let mut too_large = before.clone();
    let more: Vec<_> = (301..=500)
        .map(|id| {
            let mut value = source(id, &format!("Large {id}"));
            value.comments = "x".repeat(4096);
            value
        })
        .collect();
    too_large.sources.extend(more);
    too_large.next_id = 501;
    check!(
        too_large.validate() == Err(Error::Limit),
        "model validation rejects oversized persisted payload"
    );

    // Undo moves the current model into redo. A much larger current model must consume
    // budget there too, even though every snapshot previously retained in undo was small.
    let mut small = before.clone();
    small.sources.truncate(280);
    let small_bytes = small.byte_size();
    let capacity = (8 << 20) / small_bytes;
    let mut changing = SourceLibraryEditor::new(small).unwrap();
    for i in 0..capacity - 1 {
        let mut value = changing.library().source(1).unwrap().clone();
        value.year = format!("{:04}", 1900 + i);
        changing.apply(LibraryEdit::Put(value)).unwrap();
    }
    let additions = (1..=150)
        .map(|id| {
            let mut value = source(0, &format!("Additional {id}"));
            value.comments = "z".repeat(4096);
            value
        })
        .collect();
    changing.apply(LibraryEdit::Import(additions)).unwrap();
    let large_bytes = changing.library().byte_size();
    check!(
        changing.undo() && changing.library().sources.len() == 280,
        "undo a large import moves the large model into redo"
    );
    let expected_undo = ((8 << 20) - large_bytes) / small_bytes;
    let mut remaining_undo = 0;
    while changing.undo() {
        remaining_undo += 1;
    }
    check!(
        remaining_undo == expected_undo && remaining_undo < capacity - 1,
        "undo trims oldest states when larger redo payload exceeds combined budget"
    );
    while changing.redo() {}
    check!(
        changing.library().sources.len() == 430,
        "history trimmed for memory still redoes to latest imported collection"
    );

    let retained = source(42, "Retained");
    let incoming = populated(42);
    let complete = retained.merge_fields(&incoming, &SourceField::ALL).unwrap();
    check!(
        complete.id == 42 && complete.same_content(&incoming),
        "all metadata fields can transfer across equal numeric IDs"
    );
    for field in SourceField::ALL {
        let mut other_scope = incoming.clone();
        other_scope.id = 9000;
        let merged = retained.merge_fields(&other_scope, &[field]).unwrap();
        check!(
            merged.id == retained.id
                && merged.field_equals(&incoming, field)
                && SourceField::ALL
                    .iter()
                    .filter(|other| **other != field)
                    .all(|other| merged.field_equals(&retained, *other)),
            "individual metadata copy changes only selected field and retains local identity"
        );
    }
    let corporate = incoming
        .merge_fields(&retained, &[SourceField::Creators])
        .unwrap();
    check!(
        corporate.authors.is_empty() && corporate.corporate == "Institut",
        "creator transfer replaces people and corporate mode together"
    );
    let clear = incoming
        .merge_fields(&retained, &[SourceField::Comments])
        .unwrap();
    check!(
        clear.comments.is_empty() && clear.title == incoming.title,
        "explicit empty field clears selected metadata"
    );
    check!(
        retained.merge_fields(&incoming, &[]) == Ok(retained.clone()),
        "empty field transfer retains existing source"
    );
    check!(
        retained.merge_fields(&incoming, &[SourceField::Year, SourceField::Year])
            == Err(Error::Invalid),
        "duplicate field choices rejected"
    );
    let mut invalid = incoming.clone();
    invalid.title.clear();
    check!(
        retained.merge_fields(&invalid, &[SourceField::Title]) == Err(Error::Invalid),
        "selected invalid title cannot replace valid metadata"
    );
    check!(
        retained
            .merge_fields(&invalid, &[SourceField::Year])
            .unwrap()
            .title
            == "Retained",
        "unused incoming fields do not alter retained metadata"
    );
    let mut draft = retained.clone();
    draft.id = 0;
    check!(
        draft
            .merge_fields(&incoming, &[SourceField::Title])
            .unwrap()
            .id
            == 0,
        "unallocated draft identity remains unallocated"
    );
    (passed, failed)
}
