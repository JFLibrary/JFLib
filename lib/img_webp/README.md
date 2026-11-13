# jflib-img-webp

WebP reading and writing for [JFLib](https://jflib.org), lossy and lossless, in plain Rust,
without the standard library and without a line of `unsafe` code.

The decoder reads both kinds of WebP: lossy images (VP8 key frames) and lossless ones (VP8L), the
simple container as well as the extended one with an alpha plane, an ICC profile, EXIF and XMP.
Both bitstream decoders give the same pixels as libwebp 1.3, and the container follows libwebp's
rules for what it accepts. Of an animation it decodes the first frame onto a transparent canvas;
the metadata counts the frames and gives the loop count and the first frame's delay. Every chunk
size is checked against the file, sizes against img_core's pixel and memory budgets before
anything is allocated, and a long decode can be cancelled.

There are two encoders, both for images up to 16,383 pixels on a side. The lossless one analyses the
image the way libwebp's does: it chooses between a palette and the predictor, cross-colour and
subtract-green transforms, finds repeated pixels through hash chains and, from effort 4 on, through
a cost-model search with entropy codes clustered by region. Efforts 7 to 9 try several
configurations and keep the smallest file. The lossy encoder is modelled on libwebp's, the effort
choosing libwebp's method: segments by the complexity of each macroblock, rate-distortion mode
decisions and a loop filter per segment, with libwebp's presets for pictures, photos, drawings,
icons and text. Its alpha plane goes into the file losslessly compressed. Colours under fully
transparent pixels are kept exactly unless you allow the encoder to change them, and a sharp
RGB-to-YUV conversion is available for lossy files. Both write the ICC profile, EXIF (with the
orientation set to 1) and XMP when asked.

## How it is used

WebP is not among the formats JFLib registers at start-up. Call `img_all::use_all_codecs()` of
[`jflib-img-all`](https://crates.io/crates/jflib-img-all) once (in `App::init`), and the image
functions of [`jflib-img-core`](https://crates.io/crates/jflib-img-core) open and save WebP files
like any other format:

```rust
use jflib::img_core::{io, IMGF_WEBP, IOF_LOSSLESS};

let mut opt = io::opt_init(IMGF_WEBP);
opt.flags |= IOF_LOSSLESS;                // VP8L; without the flag lossy, at opt.quality
opt.effort = 9;                           // the smallest file, the most time
let err = io::save_opt("icon.webp", &surface, Some(&opt));   // 0, or an ERR_* code
```

For lossy files `ImgOpt::quality` sets the quality (95 unless you change it) and the field
`ImgOpt::ddsfmt` takes a preset, `img_webp::WPP_PHOTO`, `WPP_DRAWING` and so on. The flag
`img_webp::IOF_WEBP_NOEXACT` lets the encoder change colours under transparent pixels, and
`IOF_WEBP_SHARPYUV` turns on the sharp conversion.

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_webp`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-img-webp/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_webp), also on
  [docs.rs](https://docs.rs/jflib-img-webp)

## License

MIT or Apache-2.0, at your choice. Large parts of the lossy decoder and encoder and the lossless
encoder's heuristics are translated from libwebp and also stand under its BSD-3-Clause license
with Google's patent grant for the WebM codecs; both texts are in the crate's
[`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/img_webp/licenses) folder.
