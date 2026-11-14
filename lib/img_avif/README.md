# jflib-img-avif

AVIF reading and writing for [JFLib](https://jflib.org), in plain Rust, without the standard
library and without a line of `unsafe` code. An AVIF file is an AV1 still picture in a HEIF
container, so most of the work happens in two other crates: the HEIF reader of
[`jflib-img-heif`](https://crates.io/crates/jflib-img-heif) reads it, and the AV1 decoder and
encoder of [`jflib-img-av1`](https://crates.io/crates/jflib-img-av1) handle the picture. This
crate is AVIF's entry in JFLib's format table and brings the AVIF writer.

## What it reads

`img_core::io` tells AVIF from HEIC by the brand inside the file and hands AVIF files to this
codec, whose reader is img_heif's. It understands what AVIF files carry: grids of tiles, an alpha
plane (premultiplied when the file says so), the crop, rotation and mirroring of the container,
the colour description from the `nclx` box or from the AV1 sequence header, and ICC profiles,
EXIF and XMP. The AV1 picture may have 8, 10 or 12 bits and any chroma format, and film grain is
applied. Pixels come out as JFLib's 8-bit surfaces.

The reader treats every file as untrusted. Before a decoder allocates anything it checks the
picture each AV1 item declares against the pixel budget (`ImgDec::maxpx`, by default 268 million
pixels), keeps the tiles of a grid within about four times the image, and refuses the rest with
`ERR_LIMIT`.

## What it writes

The writer stores the whole image as one AV1 key frame, up to 134 million pixels, in BT.709 with
full range. Chroma is 4:2:0 by default; `chroma` 444 keeps it at full resolution, and 400 leaves
it out for a grey picture (422 is written as 4:2:0). The quality from 1 to 100 picks the
quantizer on libavif's scale of 0 to 63; 0 stands for 95, the default `io::opt_init` sets. With `IOF_LOSSLESS` the picture is 4:4:4 without colour
conversion and without quantization, so the RGB values come back exactly. An image with
transparent pixels gets its alpha as a second, grey picture. An ICC profile, EXIF (with the
orientation set to 1) and XMP go along when the options ask for them. The samples are 8 bits, and
the AV1 encoder works on the worker pool.

## How it is used

Most apps never call this crate by name. Once the codecs are registered, the image functions of
`img_core` open and save AVIF files like any other format:

```rust
use jflib::{img_all, img_core::io};

img_all::use_all_codecs();                        // once, for example in App::init

match io::load_ex("photo.avif", None, None) {
    Ok(surface) => {
        let mut opt = io::opt_init(io::format_from_path("copy.avif"));  // quality 95, 4:2:0
        opt.quality = 80;
        opt.chroma = 444;                         // chroma at full resolution
        io::save_opt("copy.avif", &surface, Some(&opt));
    }
    Err(code) => { /* an ERR_* code of img_core */ }
}
```

The crate's own items are the codec's parts: `img_avif::CODEC`, `probe` and `decode` (img_heif's
reader) and `enc::encode`, the writer.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_avif`. JFLib registers PNG, JPEG, BMP,
GIF and SVG at start-up by itself; AVIF needs `img_all::use_all_codecs()` once. All of JFLib's
crates share one version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

Reading AVIF files is safer against hostile files. A grid image can no longer make the reader
decode thousands of large tiles from a small file: every tile is checked before it is decoded,
and the tiles together must stay within about four times the image. The pixel budget now also
applies to the picture an AV1 item declares in its stream, not only to the image's size. Both
fail with `ERR_LIMIT` before anything is allocated. The writer takes quality 0 as 95, like the
other writers (it was 85). Everything is in the
[changelog](https://docs.rs/crate/jflib-img-avif/latest/source/CHANGELOG.md).

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_avif), also on
  [docs.rs](https://docs.rs/jflib-img-avif)

## License

MIT or Apache-2.0, at your choice.
