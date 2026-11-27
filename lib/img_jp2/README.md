# jflib-img-jp2

A JPEG 2000 decoder for [JFLib](https://jflib.org), in plain Rust, without the standard library and
without a line of `unsafe` code. It reads JP2 files, the JPX files JP2 readers meet (PDF's
`JPXDecode` images among them) and raw codestreams (`.j2k`, `.j2c`), following ITU-T T.800 (ISO/IEC
15444-1). It is written from the standard, not ported from another decoder.

## What it decodes

- **Wavelets:** the reversible 5/3 wavelet (lossless) and the irreversible 9/7 wavelet (lossy), with
  any number of resolution levels and any image and tile offsets.
- **Codestream layout:**
  - tiles and tile-parts in any order
  - precincts, quality layers, every progression order and progression changes (POC)
  - packet headers in the data or gathered in PPM / PPT
  - SOP and EPH markers
- **Code-blocks:** every code-block style
  - selective arithmetic coding bypass
  - context reset after each pass
  - termination after each pass
  - vertically causal contexts
  - predictable termination
  - segmentation symbols
- **Samples:**
  - region-of-interest coding (max-shift)
  - the colour transforms RCT and ICT
  - sub-sampled components
  - 1 to 24 bits per sample, signed and unsigned
- **From the JP2 header:**
  - the colour space: sRGB, greyscale, sYCC (converted to RGB here unless
    `Options::convert_sycc` is false), CMYK, CIELab, or an ICC profile, which is handed over
  - the palette with its component mapping (applied here)
  - the channel definitions: colour channels in their order and the opacity channel, also a
    premultiplied one

`Options::reduce` decodes a lower resolution, halving width and height for each level. That makes
a preview or a small rendering of a large page much cheaper.

A file that is cut short gives what was decoded up to there, as the common readers do. Limits on
the pixels, the tiles and components, code-blocks, precincts and the packets looked at refuse a
hostile file before it costs memory or time. Every target
gets the same bits: the 9/7 wavelet runs in `f32` with each step written out.

Not decoded: Part 2 of the standard (other wavelets, multiple-component transforms beyond RCT and
ICT, trellis quantisation) and samples of more than 24 bits.

## How it is used

```rust
let img = img_jp2::decode(&bytes, &img_jp2::Options::default())?;
for c in &img.components {
    // c.width x c.height samples of c.precision bits, row by row; c.dx, c.dy: its sub-sampling
}
// img.color: the colour space; img.alpha: which component is the opacity
```

Through [`jflib-img-all`](https://crates.io/crates/jflib-img-all), `img_jp2::CODEC` opens JP2,
JPX and J2K files with the image functions of
[`jflib-img-core`](https://crates.io/crates/jflib-img-core), as 8-bit ARGB surfaces (CIELab files
are `ERR_UNSUPPORTED` there: no conversion to RGB yet; `decode` gives their components).
[`jflib-pdf`](https://crates.io/crates/jflib-pdf) draws a PDF's `JPXDecode` images with it.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_jp2`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.1

This crate is new and not part of 0.2.1: it comes with the next release. The
[changelog](https://docs.rs/crate/jflib-img-jp2/latest/source/CHANGELOG.md) will say so.

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_jp2), also on
  [docs.rs](https://docs.rs/jflib-img-jp2)

## License

MIT or Apache-2.0, at your choice.
