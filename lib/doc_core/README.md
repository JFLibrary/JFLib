# jflib-doc-core

The document model of [JFLib](https://jflib.org): paragraphs with styles and character
formatting, tables, footnotes and endnotes, sources with citations and a bibliography, and an
editor that changes all of it in atomic steps with undo and redo. It is the part of a word
processor that knows nothing of pixels, fonts, files or the operating system, so the same
document can be edited by a GUI, a test, a command-line tool or an agent and always obeys the
same rules. It is plain Rust, without the standard library and without a line of `unsafe` code.

## The document and the editor

A `Document` holds paragraphs, each with a stable `Id`, a paragraph style (`Normal`, `Title`,
`Heading1`, `Heading2` or one of the document's named styles), paragraph formatting and its
content: text runs with character overrides (font family, bold, italic, underline, strike-through,
colour, size, highlight, superscript and subscript), and generated fields such as citations, the
bibliography, note references and formulas. Next to the paragraphs live the tables, the style
definitions with their inheritance, the sources, the notes and the page settings. Every length is
in typographic units such as twips and half points, never in pixels.

You change a document through an `Editor`. Each `Edit` is one transaction: it is checked against
the whole document before it commits, and an edit that would break anything leaves the document,
the history and the saved state exactly as they were. The editor keeps up to 64 undo steps within
8 MiB, tells whether the document differs from what was last saved, and has a revision that moves
with every change, the natural key for a layout cache.

```rust
use jflib::doc_core::{Document, Edit, Editor, TextFormat};

let mut ed = Editor::new(Document::default())?;            // one empty paragraph
let p = ed.document().paragraphs[0].id;
ed.apply(Edit::ReplaceParagraphText { paragraph: p, start: 0, end: 0, text: "Hello world".into() })?;
let bold = TextFormat { bold: Some(true), ..TextFormat::default() };
ed.apply(Edit::FormatParagraphText { paragraph: p, start: 0, end: 5, format: bold })?;
ed.undo();                                                  // "Hello" is plain again
assert!(ed.dirty());                                        // until mark_saved() after a save
```

Text positions are UTF-8 offsets in a paragraph's displayed text, or in a whole story (the main
body, one note or one table cell) for edits that span paragraphs. Generated fields are protected:
an edit that would cut through a citation or a formula is refused instead of turning it into
plain text. `Document::validate` checks identities, references and budgets (2048 paragraphs,
1024 sources, 1024 notes, 2 MiB of content); call it on every document that comes from outside.

## What a document can hold

**Tables** have rich paragraphs in their cells, merged cells, fixed or minimum row heights, header
rows that repeat after a page break, cell fills, alignment and borders, named table styles with
first-row, last-row and banded regions, and an optional floating position. Tables can nest up to
eight levels deep. Structural changes (insert or remove rows and columns, merge, split, resize)
are each one undoable `TableChange`.

**Formulas** are live calculation fields with Word's function names, evaluated in decimal
fixed-point arithmetic over the cells of the table they stand in. A formula that cannot be
computed shows a stable error code such as `#DIV/0!` or `#REF!` instead of failing the edit.

**Notes** are footnotes or endnotes with their own rich paragraphs, anchored exactly once in the
body or in a table cell. Their numbers are derived from the order of the anchors, so they never
go stale.

**Sources and citations** describe books, journal articles and websites with their authors (people
or one corporation) and publication details. A citation stays linked to its sources through
every edit, a bibliography field lists what the document cites, and sources can be imported with
duplicate detection, merged with every citation redirected, and checked for missing details.
The display is a simple author and year style, not a CSL processor. `SourceLibrary` with its
`SourceLibraryEditor` keeps a bibliography of its own, outside any document, with the same atomic
edits and undo.

Literal search and replace work across formatting runs and leave generated fields alone, and a
`DocumentFragment` carries a rich selection, with its sources and note bodies, from one document
into another.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::doc_core`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

The model has three companions:
[`jflib-doc-io`](https://crates.io/crates/jflib-doc-io) reads and writes it as DOCX, RTF, HTML and
bibliography files, [`jflib-doc-layout`](https://crates.io/crates/jflib-doc-layout) breaks it into
lines and pages, and [`jflib-doc-view`](https://crates.io/crates/jflib-doc-view) draws those pages
in a JFLib app. The word processor
[jfWord](https://github.com/JFLibrary/JFLib/tree/main/apps/jfWord) in JFLib's repository uses all
four.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-doc-core/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [Document sources and notes](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images)
  and [editing rich text over a drawn page](https://docs.rs/jflib/latest/jflib/guide/index.html#5-widgets),
  in JFLib's guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#doc_core): every public
  item of `doc_core`, also on [docs.rs](https://docs.rs/jflib-doc-core)

## License

MIT or Apache-2.0, at your choice.
