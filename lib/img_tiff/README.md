# jflib-img-tiff

TIFF reading and writing for [JFLib](https://jflib.org), in plain Rust, without the standard
library and without a line of `unsafe` code.

## What it reads

Classic TIFF and BigTIFF in either byte order, stored in strips or tiles, with the samples of a
pixel together or in separate planes. The colour models are grey (either polarity), RGB, palette,
CMYK, YCbCr with any of the usual subsamplings and CIELab; samples may be integers of 1 to 32 bits,
signed or unsigned, or floating point with 16, 24 or 32 bits. The compressions are none, LZW
(including the old bit-reversed variant), Deflate, PackBits, JPEG and the fax codings CCITT RLE,
Group 3 and Group 4, with the horizontal and floating-point predictors. Alpha may be associated or
unassociated. Whatever the source holds, the result has 8 bits per channel: deeper integers are
scaled down, floats are taken as linear light and encoded as sRGB.

A file with several pages reports their number in `ImgMeta::frames`, and `ImgDec::frame` picks the
page to decode. The orientation, the resolution, the ICC profile and XMP are read, and the EXIF and
GPS directories come out as a standalone EXIF block.

Large images with several strips or tiles, from a megapixel on, are decoded in parallel on the
platform's worker pool. The result never depends on the number of threads, not even which error a
damaged file reports.

TIFF lets a file describe almost anything, so the reader trusts none of it unchecked. Every
directory entry must lie inside the file, directory cycles are detected, decompressed sizes come
from the image's dimensions and never from the byte counts the file states, the tiles together may
not dwarf the image, buffers stay within img_core's memory budget, and a long decode can be
cancelled between strips. Fax data that ends early is decoded as far as it goes, and the rest of the
strip stays white.

## What it writes

Little-endian TIFF with one page: 32-bit RGBA with unassociated alpha, 24-bit RGB with transparent
parts composited over white, or an 8-bit palette image. Left to itself the encoder takes 32 bits for
an image with transparency, an exact palette for an opaque one with at most 256 colours and 24 bits
otherwise. It compresses with LZW unless you choose Deflate, PackBits or none, and 24 and 32-bit
images compressed with LZW or Deflate get the horizontal predictor. The resolution is written (96
dpi by default), the ICC profile and XMP on request. A file that would pass classic TIFF's limit of
4 GB is refused with `ERR_LIMIT`.

## How it is used

TIFF is not among the formats JFLib registers at start-up. Call `img_all::use_all_codecs()` of
[`jflib-img-all`](https://crates.io/crates/jflib-img-all) once (in `App::init`), and the image
functions of [`jflib-img-core`](https://crates.io/crates/jflib-img-core) open and save TIFF files
like any other format. `ImgOpt::tiffcomp` chooses the compression (`TIFC_NONE`, `TIFC_LZW`,
`TIFC_DEFLATE`, `TIFC_PACKBITS`) and `ImgOpt::bitdepth` 32, 24 or 8 bits. A page other than the
first:

```rust
use jflib::img_core::{io, ImgDec, ImgMeta};

let mut meta = ImgMeta::default();
io::probe(&data, Some(&mut meta));                   // meta.frames: the number of pages
let dec = ImgDec { frame: 2, ..ImgDec::default() };  // the third page
let page = io::decode_ex(&data, Some(&dec), None);
```

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_tiff`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

The decoder refuses files whose tiles are far larger than the image, more than 16 times its pixels
plus 2^20 in all, with `ERR_LIMIT`; a 1 x 1 image with huge tiles could ask for 4 GB before. The
[changelog](https://docs.rs/crate/jflib-img-tiff/latest/source/CHANGELOG.md) has the details.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_tiff), also on
  [docs.rs](https://docs.rs/jflib-img-tiff)

## License

MIT or Apache-2.0, at your choice. The YCbCr-to-RGB conversion is translated from libtiff and
also stands under the libtiff license, whose text is in the crate's
[`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/img_tiff/licenses) folder.
