# jflib-img-heif

HEIF reading and HEIC writing for [JFLib](https://jflib.org), in plain Rust, without the standard
library and without a line of `unsafe` code. HEIF is the container of HEIC, the format many
phones store their photos in, and of AVIF. This crate reads the container and the image items in
it; the pictures themselves are decoded by
[`jflib-img-hevc`](https://crates.io/crates/jflib-img-hevc) (HEVC, for HEIC) and
[`jflib-img-av1`](https://crates.io/crates/jflib-img-av1) (AV1, for AVIF). It writes HEIC files
with img_hevc's encoder.

## What it reads

The reader decodes the primary image of a HEIF file: one HEVC or AV1 picture, or a grid of them
placed side by side. Overlays and other derived images are refused with `ERR_UNSUPPORTED`. It
follows the container's whole description of the picture:

- the colour from the `nclx` box of the image or its tile, else from the HEVC or AV1 stream, with
  limited or full range and the matrix the file names; 10- and 12-bit pictures come out as 8-bit
  pixels, and `ImgMeta` reports their bit depth and whether they are HDR;
- an alpha plane stored as an auxiliary image, premultiplied when the file says so;
- the crop of the clean aperture, which the reader applies, and the rotation and mirroring, which
  it reports as the image's orientation so that `img_core::io` turns the picture upright (as HEIF
  demands, an EXIF orientation in the file does not count);
- the ICC profile, the EXIF block and the XMP packet, when the caller asks for them.

The tiles of a grid are decoded in parallel on the worker pool. Every file is treated as
untrusted: before a decoder allocates anything, the picture each item declares in its stream must
fit the pixel budget (`ImgDec::maxpx`, by default 268 million pixels), the tiles of a grid
together must stay within about four times the image with none larger than the first, and the
data fed to the decoders must stay in proportion to the file. Anything beyond is `ERR_LIMIT`.

## What it writes

The writer stores HEIC with HEVC pictures in BT.709 with full range, written into the container
and into the stream alike so that decoders which pick the matrix by themselves, like Windows',
show the same colours. Chroma is 4:2:0 by default and 4:4:4 with `chroma` 444 (422 is written as
4:2:0). The quality from 1 to 100 sets the quantizer; 0 means 95. With `IOF_LOSSLESS` the picture
is 4:4:4 without colour conversion and without quantization, so the RGB values come back exactly.

An image up to 512 x 512 pixels becomes one picture. Larger ones become a grid of tiles of up to
512 pixels a side, encoded in parallel; a tile is made 510 pixels wide or high where 512 would
leave a last column or row of one pixel, which Windows' decoder rejects. Transparent pixels give
an alpha plane as a second, grey image. An ICC profile, EXIF (with the orientation set to 1) and
XMP go along when the options ask for them. The samples are 8 bits.

## How it is used

Most apps never call this crate by name. Once the codecs are registered, the image functions of
`img_core` open and save HEIC files like any other format; a file named `.heif` is saved as HEIC
too. Here a photo is opened with its EXIF block:

```rust
use jflib::img_all;
use jflib::img_core::{io, ImgDec, ImgMeta, IMGD_EXIF};

img_all::use_all_codecs();                        // once, for example in App::init

let dec = ImgDec { flags: IMGD_EXIF, ..Default::default() };
let mut meta = ImgMeta::default();
match io::load_ex("IMG_0001.heic", Some(&dec), Some(&mut meta)) {
    Ok(photo) => { /* upright; meta.exif holds the camera's EXIF block */ }
    Err(code) => { /* an ERR_* code of img_core */ }
}
```

The crate's own `probe`, `decode` and `encode` are the codec's entries that `img_core::io` calls.
AVIF files take the same reader through [`jflib-img-avif`](https://crates.io/crates/jflib-img-avif),
which also brings the AVIF writer.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_heif`. JFLib registers PNG, JPEG, BMP,
GIF and SVG at start-up by itself; HEIC needs `img_all::use_all_codecs()` once. All of JFLib's
crates share one version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

Two fixes protect the reader against hostile files. A grid image can no longer make it decode
thousands of large tiles from a small file: every tile is checked before it is decoded, and the
tiles together must stay within about four times the image. And the pixel budget now also applies
to the picture an HEVC or AV1 item declares in its stream, not only to the image's size. Both fail
with `ERR_LIMIT` before anything is allocated. Everything is in the
[changelog](https://docs.rs/crate/jflib-img-heif/latest/source/CHANGELOG.md).

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_heif), also on
  [docs.rs](https://docs.rs/jflib-img-heif)

## License

MIT or Apache-2.0, at your choice.
