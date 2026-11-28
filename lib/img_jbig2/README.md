# jflib-img-jbig2

A JBIG2 decoder for [JFLib](https://jflib.org), in plain Rust, without the standard library and
without a line of `unsafe` code. It reads the bilevel images that PDF embeds (`JBIG2Decode`, with
their `JBIG2Globals`) and JBIG2 files (`.jb2`, `.jbig2`), following ITU-T T.88 (ISO/IEC 14492). It
is written from the standard, not ported from another decoder.

## What it decodes

- **Generic regions:**
  - MMR, or arithmetic coding with templates 0 to 3, adaptive pixels and typical prediction
    (TPGDON)
  - data of unknown length, ended by its marker
- **Refinement regions:** templates 0 and 1, with typical prediction (TPGRON).
- **Symbol dictionaries:**
  - arithmetic or Huffman coding
  - refinement and aggregate coding
  - collective bitmaps, uncompressed or MMR
  - exports
  - coding contexts kept for the next dictionary
- **Text regions:**
  - arithmetic or Huffman coding
  - refined instances
  - transposed or not, every reference corner and combination operator
  - Huffman tables: the fifteen standard ones and those of table segments
- **Halftones:** pattern dictionaries, and halftone regions with skipping and grids at any angle.
- **Regions:** every one intermediate, immediate or lossless.
- **Pages:**
  - page information, end of stripe, end of page, end of file
  - striped pages whose height is unknown until their last stripe
  - the default pixel, and the combination operators OR, AND, XOR, XNOR and REPLACE
- **Files:** sequential and random-access organisation, every page.

The segments are decoded once, in their order. A damaged segment ends its page with what was
decoded before it; the next page information segment starts the next page. Limits on the pixels
and on the work of a stream (the pixels of its pages and regions, four times `max_pixels`) refuse
hostile data before it costs memory or time; `decode_file` leaves out the pages past them. A
symbol dictionary or text region whose arithmetic data has run out stops there (a bitmap's last
rows may come from the fill: encoders may leave out the bytes at the end that equal it).

Not decoded: colour (JBIG2 Amendment 3's palettes and coloured regions are ignored; the page stays
bilevel) and extension segments.

## How it is used

```rust
// PDF: the image stream's data and its JBIG2Globals stream
let page = img_jbig2::decode_embedded(&data, Some(&globals), &img_jbig2::Options::default())?;
assert!(page.get(0, 0) || true); // true = black; page.bits: rows of page.stride bytes, MSB first
// a JBIG2 file: every page
let pages = img_jbig2::decode_file(&file, &img_jbig2::Options::default())?;
```

[`jflib-pdf`](https://crates.io/crates/jflib-pdf) draws a PDF's `JBIG2Decode` images with it, and
through [`jflib-img-all`](https://crates.io/crates/jflib-img-all) `img_jbig2::CODEC` opens a JBIG2
file's first page with the image functions of
[`jflib-img-core`](https://crates.io/crates/jflib-img-core).

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_jbig2`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.1

This crate is new and not part of 0.2.1: it comes with the next release. The
[changelog](https://docs.rs/crate/jflib-img-jbig2/latest/source/CHANGELOG.md) will say so.

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_jbig2), also on
  [docs.rs](https://docs.rs/jflib-img-jbig2)

## License

MIT or Apache-2.0, at your choice.
