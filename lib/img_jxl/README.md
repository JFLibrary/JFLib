# jflib-img-jxl

JPEG XL reading and writing for [JFLib](https://jflib.org), in plain Rust, without the standard
library and without a line of `unsafe` code. The crate holds the complete decoder and a compact
writer of its own. The full encoder, a port of libjxl's, is the separate crate
[`jflib-img-jxl-enc`](https://crates.io/crates/jflib-img-jxl-enc), which this crate uses when the
app asks for it.

## What it reads

The decoder reads JPEG XL as ISO/IEC 18181 defines it, the bare codestream as well as the box
container. Lossless (modular) and lossy (VarDCT) images decode with all their tools: reference
frames and blending, patches, splines, noise, the edge-preserving filter, upsampling and
progressive DC frames. Of an animation it decodes any shown frame (`ImgDec::frame`); `ImgMeta`
tells how many frames there are and how long each is shown. The ICC profile, EXIF and XMP come
along, Brotli-compressed metadata boxes included. Images with more than 8 bits per sample come out
as 8-bit pixels. What the decoder cannot show correctly it refuses with `ERR_UNSUPPORTED` instead
of guessing: floating-point samples outside the XYB colour space, and XYB images (the usual lossy
kind) meant for the transfer curves PQ, HLG or BT.709 or for other primaries than sRGB's.

The groups of a frame decode in parallel on the worker pool, with the same pixels for any number
of threads. Files are treated as untrusted: the image and every frame with a size of its own must
fit the pixel budget (`ImgDec::maxpx`, by default 268 million pixels), and the decoder bounds what
palettes, patches and splines may cost before it allocates or computes, with `ERR_LIMIT` or
`ERR_NOMEM` instead of a crash.

A JPEG that was converted to JPEG XL without loss can be had back: `jpeg::reconstruct(data)`
returns the original JPEG file, byte for byte as libjxl gives it.

## Two writers

Saving goes to one of two writers. Without anything further it is the compact writer in this
crate: lossless images with a fixed predictor, lossy ones with 8 x 8 DCT blocks throughout. It
honours the quality, `IOF_LOSSLESS`, EXIF and XMP; the effort and the other options mean nothing
to it.

The full encoder lives in [`jflib-img-jxl-enc`](https://crates.io/crates/jflib-img-jxl-enc). After
`img_jxl_enc::use_static()` every JPEG XL save of the app uses it, with all the options JFLib has
for the format: the effort from 1 to 9 as libjxl has it, progressive files, photon noise, embedded
ICC profiles. The same encoder can also come from a loadable module instead of being linked in
(`writer::use_module`); its README explains both ways. `IOF_JXL_COMPAT` still picks the compact
writer for a single save, and `img_jxl::writer::kind()` tells which writer is in use. The options
are listed in the API reference.

## How it is used

Most apps never call this crate by name. Once the codecs are registered, the image functions of
`img_core` open and save JPEG XL files like any other format:

```rust
use jflib::{img_all, img_core::io, img_jxl_enc};

img_all::use_all_codecs();                        // once, for example in App::init
img_jxl_enc::use_static();                        // the full encoder (not on wasm32)

match io::load_ex("photo.jxl", None, None) {
    Ok(surface) => {
        let mut opt = io::opt_init(io::format_from_path("copy.jxl"));  // quality 90, effort 7
        opt.quality = 100;                        // lossless
        io::save_opt("copy.jxl", &surface, Some(&opt));
    }
    Err(code) => { /* an ERR_* code of img_core */ }
}
```

The crate's own functions are there for work closer to the format: `img_jxl::open` and
`jx_frames` walk a file's headers and frames, `box_contents` takes a metadata box out of the
container, `jpeg::has_jbrd` and `jpeg::reconstruct` give back a converted JPEG. The decoder's
stages are public modules as well.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_jxl`. JFLib registers PNG, JPEG, BMP,
GIF and SVG at start-up by itself; JPEG XL needs `img_all::use_all_codecs()` once. All of JFLib's
crates share one version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

Five fixes protect the decoder against hostile files. A container box whose 64-bit size reaches
the end of the number range no longer crashes `probe` and `decode`. A frame with a size of its own
counts against the pixel budget like the image. And palettes, patches and the splines' cache can
no longer make the decoder allocate gigabytes or compute for minutes from a small file; such files
fail with `ERR_LIMIT` or `ERR_NOMEM`; plane sizes on 32-bit targets and the splines' area
estimate no longer wrap either. Valid files with chroma subsampling whose width is no multiple
of 16 decode with the right colours now (the edge-preserving filter read the chroma rows from
the wrong place). Everything is in the
[changelog](https://docs.rs/crate/jflib-img-jxl/latest/source/CHANGELOG.md).

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_jxl), with the
  writer's options, also on [docs.rs](https://docs.rs/jflib-img-jxl)

## License

MIT or Apache-2.0, at your choice. The decoder was written after libjxl 0.11.1's, and some parts
are ported from libjxl (the splines, the fast math, the JPEG reconstruction); libjxl is under the
BSD 3-Clause License with Google's patent grant. Brotli's static dictionary, which the metadata
boxes and the JPEG reconstruction need, is under the MIT License. The texts are in the crate's
[`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/img_jxl/licenses) folder.
