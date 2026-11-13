# jflib-doc-layout

Paragraph flow and pagination for [JFLib](https://jflib.org)'s documents. It takes a document of
[`jflib-doc-core`](https://crates.io/crates/jflib-doc-core) and breaks it into lines and pages,
with every position in twips on the page. It measures no text itself: the caller supplies the
widths and heights of its fonts, so the same document gives the same pages at any zoom, on any
screen and in a test without a window. It is plain Rust, without the standard library and
without a line of `unsafe` code.

## How it is used

You implement `Measure`, two functions that give the width of a piece of text and the metrics
of a font, both in twips for a resolved format, and call `paginate`:

```rust
use jflib::doc_core::{Document, ResolvedFormat};
use jflib::doc_layout::{paginate, Measure, Metrics};

struct Fixed; // every character six tenths of the font size wide
impl Measure for Fixed {
    fn width(&mut self, text: &str, f: ResolvedFormat) -> i32 {
        text.chars().count() as i32 * i32::from(f.display_size_half_points()) * 6
    }
    fn metrics(&mut self, f: ResolvedFormat) -> Metrics {
        let em = i32::from(f.display_size_half_points()) * 10;   // half points to twips
        Metrics { ascent: em * 4 / 5, descent: em / 5, height: em * 6 / 5 }
    }
}

let layout = paginate(&doc, &mut Fixed)?;
for page in &layout.pages {
    for line in &page.lines { /* line.paragraph, line.top, line.fragments: text at x, baseline */ }
}
```

A `Layout` holds the page settings and the pages. Each `Line` names its paragraph and the UTF-8
range of the paragraph's text it shows, and each `Fragment` on it carries its text, its resolved
format and its place, so a renderer draws it and an editor maps a click back to a caret in the
model. Table cells come as `CellBox` rectangles with their fills and resolved borders, and pages
with footnotes have the rule above them. In a JFLib app,
[`jflib-doc-view`](https://crates.io/crates/jflib-doc-view) does the measuring with real fonts
and draws the pages.

## What it lays out

Formatting runs, spaces and tabs (left, centre, right, decimal and bar tabs, with leaders), line
and page breaks, paragraph spacing, indents and alignment, and headings kept with the paragraph
that follows when that fits on a page. Footnotes reserve room at the bottom of the page of their
anchor and continue on the next page when they are long; endnotes follow the main text. A note is
never left out: when an anchor and the start of its note cannot share a page, `paginate` returns
`Error::Limit`.

Tables flow by rows. Header rows repeat after a page break, a row that should not split moves to
the next page when it can, and a row taller than a page still breaks between lines without losing
text. Floating tables stand at their position and the text flows around them. `table_fit`
computes column widths from the contents, from the page width or in equal parts, with the same
measurements as the pages.

Everything is bounded: a document that would expand beyond the crate's budgets for text,
fragments or pages ends in `Error::Limit` instead of using unbounded memory. Text shaping,
the bidirectional algorithm and page breaks identical to Word's are not part of it.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::doc_layout`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-doc-layout/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [Document notes](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images)
  and [editing rich text over a drawn page](https://docs.rs/jflib/latest/jflib/guide/index.html#5-widgets),
  in JFLib's guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#doc_layout): every
  public item of `doc_layout`, also on [docs.rs](https://docs.rs/jflib-doc-layout)

## License

MIT or Apache-2.0, at your choice.
