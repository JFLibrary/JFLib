# jflib-img-jpeg

JPEG reading and writing for [JFLib](https://jflib.org), in plain Rust, without the standard
library and without a line of `unsafe` code.

The decoder reads baseline, extended and progressive JPEG files with Huffman coding: grey, YCbCr and
RGB images as well as the CMYK and YCCK files of print work, with chroma subsampling factors from 1
to 4, restart intervals and 16-bit quantization tables. The EXIF orientation is applied while
decoding, so the picture comes out upright. Arithmetic coding, lossless and hierarchical JPEG and
12-bit samples are refused as unsupported, and so are images beyond 32,768 pixels on a side or 268
million pixels in all. Nearly every baseline file is decoded through a small ring of rows rather
than whole sample planes, with the pixels converted while the data is read.

The decoder is forgiving where real files are sloppy: missing Huffman tables are replaced by the
standard's, garbage between segments is skipped, a missing end marker does no harm. A damaged or
cut-off file still shows what could be decoded, as libjpeg would show it: the rest of a broken
interval stays grey and lost restart markers are found again.

The encoder writes baseline JFIF files. The quality, 1 to 100, scales the standard's quantization
tables the way libjpeg does, and the Huffman tables are built for each image in a second pass,
which makes the file smaller at no cost in quality or compatibility. Colour is stored as YCbCr
with 4:4:4, 4:2:2 or 4:2:0 subsampling, by default 4:2:0 below quality 90 and 4:4:4 from there
on, or as grey alone; an 8-bit grey surface becomes a one-component file. Transparent pixels are
composited over white. The metadata goes along: the resolution, the ICC profile (spread over as
many segments as it needs), and EXIF and XMP when each fits into one segment, the EXIF
orientation set to 1 because the pixels are already upright. When reading, the ICC profile,
EXIF, XMP, the resolution and the Adobe colour marker are picked up.

## How it is used

Most apps never call this crate by name. JFLib registers JPEG at start-up, so the image functions
of [`jflib-img-core`](https://crates.io/crates/jflib-img-core) open and save JPEG files like any
other format (`img_core::io::load_ex`, `save_opt`; `ImgOpt::quality` and `ImgOpt::chroma` choose
the quality and the subsampling). The crate's own functions are there for work closer to the
format:

```rust
use jflib::img_jpeg::{self, EncCfg};

let cfg = EncCfg { chroma: 444, unit: 1, xd: 300, yd: 300 };   // 4:4:4, 300 dpi
let file: Option<Vec<u8>> = img_jpeg::encode_ex(&surface, 90, Some(&cfg));
```

`img_jpeg::decode` turns a file in memory into a surface, and `decode_fl_max` refuses a frame
larger than the caller allows before allocating anything. Other parts of JFLib decode their
embedded JPEG data through this crate: JPEG-compressed TIFF strips and tiles, BMP files that hold
a JPEG, and the textures of glTF models.

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_jpeg`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-img-jpeg/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_jpeg), also on
  [docs.rs](https://docs.rs/jflib-img-jpeg)

## License

MIT or Apache-2.0, at your choice. The integer DCTs, the colour conversions, the upsampling and
the encoder's rules for quantization and Huffman tables are translated from IJG's libjpeg and
libjpeg-turbo, and the decoder's Huffman lookup follows stb_image; those parts also stand under
the IJG License, BSD-3-Clause and Zlib (libjpeg-turbo) and MIT or Unlicense (stb_image), whose
texts are in the crate's
[`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/img_jpeg/licenses) folder. This
software is based in part on the work of the Independent JPEG Group.
