# jflib-text-shape

OpenType shaping and the Unicode bidirectional algorithm for [JFLib](https://jflib.org)'s text
engine. It is a thin layer over three third-party crates, pinned to exact versions and used
without the standard library:

| Crate | Version | License | What it does here |
| --- | --- | --- | --- |
| [HarfRust](https://crates.io/crates/harfrust) | 0.14.0 | MIT | the shaping: HarfBuzz's shaper in Rust |
| [unicode-bidi](https://crates.io/crates/unicode-bidi) | 0.3.18 | MIT or Apache-2.0 | the direction of each run |
| [unicode-script](https://crates.io/crates/unicode-script) | 0.5.8 | MIT or Apache-2.0 | the script of each character |

## A building block

`jflib` brings this crate along and makes it reachable as `jflib::text_shape`, but hides it from
its documentation: it is a building block of [`jflib-text`](https://crates.io/crates/jflib-text)
and no part of what `jflib` promises (JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md#the-crates-on-cratesio)).
Its API may change in any release. An app shapes text through `jflib-text` instead, which
switches shaping on with `Text::set_shaping` and keeps measuring, drawing, hit testing and
carets in step with it.

## What it does

`Engine::new(font, face)` opens a TrueType or OpenType font (a face of a collection) from its
bytes, and `with_coords` gives the same font at the coordinates of a variable instance.
`Engine::shape(text)` shapes one line: it splits the text into runs of one direction and one
script, lets HarfRust shape each run (ligatures, combining marks, Arabic joining, Indic
reordering) and returns the glyphs in visual order, each with its glyph id, advance and offset
in font units and the byte range of the text it stands for. `shape_with_fallback` takes further
engines and shapes a run that the first font does not cover with the first one that does. A line
longer than 65,536 bytes is refused.

## How to get it

Through the one dependency [`jflib`](https://crates.io/crates/jflib), which brings it along. All
of JFLib's crates share one version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-text-shape/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [API reference](https://docs.rs/jflib-text-shape) on docs.rs
- [The text engine's API](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#text), which
  uses it

## License

MIT or Apache-2.0, at your choice. The crates it builds on keep their own licenses: `NOTICE.md`
lists them with the crates they depend on in turn, and `licenses/` holds their license texts.
