# jflib-img-jxr

JPEG XR reading and writing for [JFLib](https://jflib.org), in plain Rust, without the standard
library and without a line of `unsafe` code. JPEG XR (ITU-T T.832) is the standardised form of
Microsoft's HD Photo; its files end in `.jxr`, `.wdp`, `.hdp` or `.wmp`.

## What it reads

The decoder reads the TIFF-like container and the codestream in spatial and frequency mode, with
tiles and the overlap filters. Grey, RGB (stored as YUV 4:2:0, 4:2:2 or 4:4:4) and CMYK images
decode at every bit depth the format has, from 1 bit up to 16- and 32-bit integers and floating
point; an alpha channel may be part of the image or a separate codestream, and premultiplied
formats stay premultiplied. The samples are converted to
JFLib's 8-bit pixels the way Windows' own codec converts them. The ICC profile, EXIF, XMP, the
resolution and the orientation come along. A few rare options are refused with `ERR_UNSUPPORTED`:
images with an arbitrary number of channels, frequency mode without an index table, and hard
tiling where the overlap filters cross several tiles.

Files are treated as untrusted. The size is checked against the pixel budget (`ImgDec::maxpx`, by
default 268 million pixels), memory is reserved so that running out of it is `ERR_NOMEM`, and a
file too short for the number of macroblocks it claims fails with `ERR_TRUNC` before the planes
are allocated.

## What it writes

The writer stores 24-bit colour, or 32-bit colour with alpha when `bitdepth` is 32, or when it is
0 and a pixel is not opaque. The alpha goes into a second codestream that is always lossless. With
`IOF_LOSSLESS` the colour is lossless too. Otherwise the quality from 1 to 100 (0 means 90) picks
the quantizers with the tables Microsoft's reference software uses, the overlap filter smooths the
block edges, and `chroma` 422 or 420 subsamples the colour (anything else keeps 4:4:4). The ICC
profile, XMP and the resolution go along when the options ask for them, and a cancel flag is
checked once per row of macroblocks.

## How it is used

Most apps never call this crate by name. Once the codecs are registered, the image functions of
`img_core` open and save JPEG XR files like any other format:

```rust
use jflib::{img_all, img_core::io, img_core::IOF_LOSSLESS};

img_all::use_all_codecs();                        // once, for example in App::init

match io::load_ex("photo.jxr", None, None) {
    Ok(surface) => {
        let mut opt = io::opt_init(io::format_from_path("copy.jxr"));  // quality 90, 4:4:4
        opt.flags |= IOF_LOSSLESS;
        io::save_opt("copy.jxr", &surface, Some(&opt));
    }
    Err(code) => { /* an ERR_* code of img_core */ }
}
```

The crate's own `probe`, `decode` and `enc::encode` are the codec's entries that `img_core::io`
calls.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_jxr`. JFLib registers PNG, JPEG, BMP,
GIF and SVG at start-up by itself; JPEG XR needs `img_all::use_all_codecs()` once. All of JFLib's
crates share one version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

A file with an index table now gets the same plausibility check as one without: a file of 86 bytes
that claims 16384 x 16384 pixels fails with `ERR_TRUNC` right after its headers instead of after
allocating gigabytes. Everything is in the
[changelog](https://docs.rs/crate/jflib-img-jxr/latest/source/CHANGELOG.md).

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_jxr), also on
  [docs.rs](https://docs.rs/jflib-img-jxr)

## License

MIT or Apache-2.0, at your choice. The two tables that map the quality to quantizers come from
jxrlib, Microsoft's reference software, which is under the BSD 2-Clause License; the text is in
the crate's [`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/img_jxr/licenses)
folder.
