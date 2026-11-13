# jflib-doc-view

The page view of [JFLib](https://jflib.org)'s documents. It brings together the document model
of [`jflib-doc-core`](https://crates.io/crates/jflib-doc-core), the pagination of
[`jflib-doc-layout`](https://crates.io/crates/jflib-doc-layout) and JFLib's text engine and
renderer, so that an app shows paper pages, previews and thumbnails with one implementation on
every platform. On top come the pieces a word processor builds its editing around: rulers, table
handles and the property editors for tables, tab stops and formulas. It is plain Rust, without
the standard library and without a line of `unsafe` code.

## Fonts that belong to the document

A document's look must not depend on the app's theme. `Fonts` registers the document faces once
per text engine, independently of the fonts the GUI style uses, and keeps them across document
changes. DejaVu Sans is always there; DejaVu Serif and DejaVu Sans Mono are built in and loaded
when a document names them, each in regular, bold and synthetic italic and bold italic. Outside
the browser the fonts installed on the system can be used as well; in the browser the three
built-in families need no access to the user's fonts. A family that is not available keeps its
name in the document and is drawn with a built-in substitute, chosen by its name (a "mono" family
gets the monospaced face, a "serif" one the serif face), and `substituted` tells the app so. Up to
64 further families are kept in the cache.

## Pages on the screen

`doc_view::paginate` measures a document with these fonts and returns the `doc_layout::Layout`;
the GUI's own font is restored afterwards. A `PageDraw` then holds one page of it. It implements
the UI's `Draw`, so it goes onto a canvas as a custom drawing command, and `thumbnail` renders the
same page into a picture of its own, at most 2048 by 4096 pixels:

```rust
use alloc::rc::Rc;
use jflib::{doc_core::Document, doc_view::{self, Fonts, PageDraw}, gfx::Surface, ui::Ui};

// Fonts::new(&mut ui.text) once, then the same Fonts for every document
fn first_page(ui: &mut Ui, fonts: &Fonts, doc: &Document) -> Option<(PageDraw, Surface)> {
    let layout = Rc::new(doc_view::paginate(doc, &mut ui.text, fonts).ok()?);
    let page = PageDraw::new(layout, 0, fonts.clone()).ok()?;
    let thumb = page.thumbnail(&mut ui.text, 240).ok()?;     // 240 pixels wide
    Some((page, thumb))
}
```

The colours on a page are the document's. The app's theme, dark mode, the screen's scale and the
zoom change neither the formatting nor the line breaks. `PageTransform` converts between twips on
the page and pixels on the screen with one scale for drawing, pointer input and thumbnails,
rejects sizes that make no sense and saturates extreme coordinates; `zoom_width` gives a page's
width at 10 to 500 percent. `apply_paginated` commits an edit of a `doc_core::Editor` only when
the result can still be laid out with the real fonts, so a failed layout never leaves a document
that cannot be shown.

## Editing around the page

`RulerDraw` paints the horizontal and vertical rulers in the page's own scale, with the margins,
the indents and the tab stops of the selected paragraph, and `RulerController` turns drags on them
into new indents, margins and tab stops (snapped to a sixteenth of an inch; Alt drags freely,
Escape cancels). `TableController` resizes columns and rows and moves floating tables with the
mouse and returns one `doc_core::TableChange` when the button is released. `TableProperties`,
`CellBorderEditor`, `TableStyleEditor`, `TablePositionEditor`, `TabEditor` and `FormulaEditor`
draw the contents of the matching dialogs with JFLib's widgets and hand back what to apply. None
of them changes the document itself: the model and its undo history stay in the app's hands.

What the crate does not do yet: a fallback font for single characters that the chosen face lacks,
and complex text shaping. A substitute face can have other metrics than the font it stands for,
so the same line breaks need the same families on both computers.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::doc_view`. All of JFLib's crates share one
version and are released together. It runs wherever JFLib's UI runs.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-doc-view/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [Editing rich text over a drawn page](https://docs.rs/jflib/latest/jflib/guide/index.html#5-widgets),
  in JFLib's guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#doc_view): every public
  item of `doc_view`, also on [docs.rs](https://docs.rs/jflib-doc-view)

## License

MIT or Apache-2.0, at your choice. The crate bundles the DejaVu fonts, DejaVu Sans 2.37 converted
to the embedded font format of [`jflib-text`](https://crates.io/crates/jflib-text) and DejaVu Serif
and DejaVu Sans Mono 2.37 unchanged; they stand under the Bitstream Vera licence, with DejaVu's
own changes in the public domain, as
[DejaVu-LICENSE.txt](https://github.com/JFLibrary/JFLib/blob/main/lib/doc_view/assets/DejaVu-LICENSE.txt)
says.
