# jflib-img-hevc

HEVC (H.265) intra coding for [JFLib](https://jflib.org): the decoder and the encoder for the
still pictures inside HEIC files, in plain Rust, without the standard library and without a line
of `unsafe` code. The crate works on NAL units and sample planes, not on files. The HEIF container
around them is read and written by [`jflib-img-heif`](https://crates.io/crates/jflib-img-heif).

## What it decodes

The decoder handles the intra-coded picture a still image consists of, in Main, Main 10, Main 12
and Main Still Picture streams: 8 to 12 bits, 4:0:0, 4:2:0, 4:2:2 and 4:4:4, with scaling lists,
transform skip, lossless blocks, PCM, sign data hiding, quantizer changes inside the picture,
deblocking and SAO, tiles, wavefront parallel processing, several slices and dependent slice
segments. It decodes the first picture of a stream; the video parameter set, SEI messages and
other NAL units are ignored.

It is no video decoder: P and B slices are refused with `ERR_UNSUPPORTED`, and so are separate
colour planes and several tools of the range extensions (cross-component prediction, chroma QP
offset lists, extended precision, implicit RDPCM, persistent Rice adaptation, bypass alignment).
A picture of more than 2^26 luma samples, almost twice what HEVC's largest level allows, is
refused with `ERR_LIMIT` before its planes are allocated, and running out of memory is
`ERR_NOMEM`, not an abort. A cancel flag is checked once per coding tree unit.

## What it encodes

The encoder writes the parameter sets and one IDR slice of a picture: 8 bits in 4:0:0, 4:2:0 or
4:4:4, coding tree units of 32 x 32, and coding blocks from 8 to 32 chosen by a rate-distortion
search. For each block it measures the intra modes, codes the most promising ones for real and
keeps the cheapest; the deblocking filter is on, SAO is not used. `HEF_LOSSLESS` codes every block
losslessly. The result is the NAL units with their headers, and the twelve bytes of profile, tier
and level that the `hvcC` box of a HEIC file needs.

## How it is used

Apps rarely call this crate by name. Once `img_all::use_all_codecs()` has registered HEIC,
`img_core::io` opens and saves HEIC files like any other format, and this crate does the HEVC part
underneath (see [`jflib-img-heif`](https://crates.io/crates/jflib-img-heif)).

The crate's own API is there for work on HEVC data itself. `Hevc::new(0, 0)` makes a decoder,
`nal(unit)` takes one NAL unit after the other (or `annexb(stream)` a whole Annex B byte stream),
and `finish()` runs the loop filters and returns the `Pic`: 16-bit sample planes with the
conformance window, the bit depths, the chroma format and the colour description of the stream.
`img_hevc::encode(&EncCfg)` goes the other way, from sample planes and a quantizer to NAL units.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_hevc`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-img-hevc/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_hevc), also on
  [docs.rs](https://docs.rs/jflib-img-hevc)

## License

MIT or Apache-2.0, at your choice.
