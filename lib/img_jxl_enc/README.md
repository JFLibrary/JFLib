# jflib-img-jxl-enc

The full JPEG XL encoder of [JFLib](https://jflib.org): a port of the encoder of libjxl 0.11.1,
the reference implementation, in plain Rust, without the standard library and without a line of
`unsafe` code. It is a crate of its own beside
[`jflib-img-jxl`](https://crates.io/crates/jflib-img-jxl), which holds the decoder and a compact
writer, because it is large: an app that only reads JPEG XL does not have to carry it, and a
program can even load it from a separate file when it first needs it.

## How the two crates relate

img_jxl decodes JPEG XL and registers the format. When it saves a file, it asks which writer the
app has chosen. Without a choice it uses its own compact writer, which knows only a few fixed
tools. One call at start-up, `img_jxl_enc::use_static()`, installs this crate's encoder instead,
and from then on every JPEG XL save of the app goes through it, whether it comes from
`img_core::io` or from img_jxl itself. The encoder reuses the decoder's parts, which img_jxl
exports for it, and reads its options from the same `ImgOpt` every format uses.

## What it writes

The encoder picks its tools by the effort, from 1 to 9, the way libjxl does; a higher effort gives
a smaller file and takes longer, and 7 is the default. Lossless images (quality 100, or
`IOF_LOSSLESS`) use palettes and colour transforms, the weighted predictor and learned
context trees, and from effort 8 on LZ77 and a search for the predictor's parameters. Lossy images
(quality 1 to 99, mapped to a Butteraugli distance as libjxl maps it; the default is 90) are coded
in XYB with VarDCT: adaptive quantization, a search over the transforms from 2 x 2 to 64 x 64
pixels, chroma from luma and gaborish, and at efforts 8 and 9 the Butteraugli loop, which decodes
the frame two or four times and adjusts the quantization by Butteraugli, a measure of what the eye
notices. Patches put repeated small content, such as the glyphs of text or the pieces of a user
interface, into the file once.

The options of the format are all there. `IOF_JXL_PROGRESSIVE` writes a file that shows a coarse
picture first, photon noise has the decoder add the grain of a camera sensor at a chosen ISO,
`IOF_EMBED_ICC` stores the colour profile, and EXIF and XMP go into their boxes. The work runs on
the worker pool, and the file is the same byte for byte for any number of threads
(`IOF_JXL_1THREAD` uses none). The samples are 8 bits; extra channels other than alpha, deeper
samples and animations are not written.

## How it is used

```rust
use jflib::img_core::io;
use jflib::img_jxl::writer::{IOF_JXL_NOISE_SHIFT, IOF_JXL_PROGRESSIVE};
use jflib::{img_all, img_jxl_enc};

img_all::use_all_codecs();                        // once, for example in App::init
img_jxl_enc::use_static();                        // JPEG XL saves take the full encoder

let mut opt = io::opt_init(io::format_from_path("photo.jxl"));    // quality 90, effort 7
opt.effort = 9;
opt.flags |= IOF_JXL_PROGRESSIVE | (16 << IOF_JXL_NOISE_SHIFT);   // photon noise of ISO 1600
io::save_opt("photo.jxl", &surface, Some(&opt));
```

To call the encoder without `img_core::io`, `enew::encode(&surface, &opts)` takes the options as
`img_jxl::writer::Opts::from_imgopt(&opt)` makes them and returns the file. The encoder's stages
are public modules as well.

Inside JFLib's repository the same encoder is also built as a loadable module, `jxlenc`. A program
can then leave the encoder out of its executable and load it from a file next to it on the first
JPEG XL save (`img_jxl::writer::use_module`); the file it writes is the same byte for byte. That
is why this crate also compiles with `--cfg jflib_module`, on
[`jflib-modrt`](https://crates.io/crates/jflib-modrt) and
[`jflib-modplat`](https://crates.io/crates/jflib-modplat) instead of JFLib's runtime. The module
itself is not published, and building modules for programs outside the repository is not
supported yet; jflib-modrt's README tells more.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_jxl_enc`, on every platform but the
browser. For wasm32 the facade leaves it out, because it takes about 25 minutes to compile for
that target; an app that wants the full encoder there names `jflib-img-jxl-enc` as a dependency of
its own, at the same version as `jflib`. Without it, JPEG XL files are saved by img_jxl's compact
writer. All of JFLib's crates share one version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-img-jxl-enc/latest/source/CHANGELOG.md)
says the same.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_jxl_enc), with the
  writer's options under [img_jxl](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_jxl),
  also on [docs.rs](https://docs.rs/jflib-img-jxl-enc)

## License

MIT or Apache-2.0, at your choice. The encoder is a port of libjxl 0.11.1's encoder and of libjxl
0.7's Butteraugli, which are under the BSD 3-Clause License with Google's patent grant; both texts
are in the crate's [`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/img_jxl_enc/licenses)
folder.
