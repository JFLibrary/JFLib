# jflib-img-core

The shared part of [JFLib](https://jflib.org)'s image codecs and the place an app goes to open and
save pictures. It holds the format table that every codec registers in, the functions that load,
detect and save files of any registered format, the records for metadata and options, and the
building blocks the codecs share: EXIF and ICC handling, colour management, resampling, colour
quantization and DEFLATE. It is plain Rust, without the standard library and without a line of
`unsafe` code.

## Loading and saving

`img_core::io` is what apps call. `io::load_ex` reads a file of any registered format, recognised by
its content (the name only helps a format without a signature, such as TGA), and returns a
`gfx::Surface` with premultiplied pixels (or straight alpha if you ask for it), already turned
upright when the file carries an EXIF orientation. `io::save_opt` writes a surface in the format the
file name names, and writes it atomically, so a failed save never leaves half a file behind.
`io::opt_init` gives the usual defaults of a format, which you then adjust: quality, effort, bit
depth, chroma subsampling, compression and the metadata to embed.

```rust
use jflib::img_core::{io, ImgDec, ImgMeta, IMGD_EXIF, IMGD_ICC, IMGD_XMP};
use jflib::img_core::{IOF_EMBED_EXIF, IOF_EMBED_ICC, IOF_EMBED_XMP};

let dec = ImgDec { flags: IMGD_ICC | IMGD_EXIF | IMGD_XMP, ..ImgDec::default() };
let mut meta = ImgMeta::default();
match io::load_ex("photo.jpg", Some(&dec), Some(&mut meta)) {
    Ok(surface) => {
        let mut opt = io::opt_init(io::format_from_path("photo.png"));
        opt.flags |= IOF_EMBED_ICC | IOF_EMBED_EXIF | IOF_EMBED_XMP;
        opt.meta = Some(&meta);                   // the profile, EXIF and XMP travel along
        if io::save_opt("photo.png", &surface, Some(&opt)) != 0 { /* an ERR_* code */ }
    }
    Err(code) => { /* ERR_IO, ERR_TRUNC, ERR_LIMIT, ... */ }
}
```

`ImgDec` chooses the page or frame of a file with several, a pixel budget, a memory budget for
the codec's scratch buffers and an atomic flag that cancels a long decode from another thread.
`ImgMeta` receives what the file says about itself: size, bit depth, colour model, alpha,
whether it is lossless, animated, interlaced or HDR, the number of frames, the resolution, and on
request the ICC profile, the EXIF block and the XMP packet. `io::probe` reads only the header,
`io::detect` only the format. The simpler `io::load`, `io::save` and `io::decode` cover PNG,
JPEG, BMP and GIF without options.

Files are treated as untrusted input. The decoders check sizes before they allocate, and an
image beyond the pixel budget, 268 million pixels unless you set another, is refused with
`ERR_LIMIT` before its pixels exist. Errors are the `ERR_*` codes of this crate.

## The format table

Each codec crate exports a descriptor, `CODEC`, and enters it into the table with
`io::register`. The table has an entry for PNG, JPEG, BMP, GIF, TIFF, TGA, DDS, WebP, AVIF, HEIC,
JPEG XL, JPEG XR, Paint.NET and Photoshop, each with its extensions, MIME type and what it can
hold (alpha, layers, pages, animation, HDR, ICC, EXIF, XMP); SVG takes a slot of its own when
[`jflib-img-svg`](https://crates.io/crates/jflib-img-svg) registers it. A plugin can add a format
at run time (`io::register_dyn`), and `io::filter_list` turns the table into the list a file
dialog shows.

## Metadata and colour

`img_core::exif` reads TIFF directory structures, classic and BigTIFF, with every value checked
to lie inside the data and cycles detected; it finds an EXIF block's orientation, resolution and
colour space and can set the orientation to 1 once the pixels have been turned. `img_core::icc`
validates ICC profiles and splits and joins them across a JPEG's APP2 segments.

`img_core::icc_xform::IccXform` converts pixels between two ICC profiles: RGB and grey, version 2
and 4, matrix-and-curve profiles as well as LUT-based ones, in all four rendering intents, with
black point compensation if you want it. Two matrix-and-curve profiles take a vectorised path;
everything else goes through a 33 x 33 x 33 grid with tetrahedral interpolation. Profiles of CMYK,
Lab and other device spaces are refused with `ERR_UNSUPPORTED`. Five profiles are built in (sRGB, an
A98-compatible RGB, Display P3, ROMM RGB and grey with gamma 2.2), and `plat::monitor_icc` of
[`jflib-plat`](https://crates.io/crates/jflib-plat) supplies the profile of the monitor a window is
on.

## Image operations

`img_core::resample` resizes with nearest neighbour, bilinear, Catmull-Rom, Mitchell, Lanczos-3,
an exact area average or an adaptive choice per axis, in premultiplied space so edges against
transparency stay clean, and in linear light if you ask (`RS_GAMMA`). `img_core::xform` draws an
image through an affine or a projective transform with transparent, clamped, wrapped or mirrored
edges. `img_core::ops` flips, rotates, crops, scales and skews, `img_core::mip` keeps a chain of
half-size levels for zoomed-out display, and `img_core::quantize` reduces an image to a palette of
up to 256 colours: exact when the image has no more, otherwise with Wu's quantizer and optional
Floyd-Steinberg dithering. Resizing and transforms work on bands of rows that several threads
may fill at once, and the result does not depend on how the rows were split.

## Compression

DEFLATE, zlib and gzip in both directions (`inflate`, `deflate`, `gzip`), with CRC-32 and Adler-32
(`checksum`). The decoders write into a buffer of fixed size and check every write and every back
reference against it, so corrupt data ends in an error code. The compressor has the familiar
levels 0 to 9, and both directions can keep a reusable state for callers that handle many small
pieces.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_core`. All of JFLib's crates share one
version and are released together.

An app with a window needs no set-up: its runtime connects the file functions and registers PNG,
JPEG, BMP, GIF and SVG at start-up. For TIFF, TGA, WebP, AVIF, HEIC, JPEG XL, JPEG XR, DDS,
Paint.NET and Photoshop files, call `img_all::use_all_codecs()` of
[`jflib-img-all`](https://crates.io/crates/jflib-img-all) once in `App::init`. img_core does not
depend on the platform layer, so a command-line program connects it itself:

```rust
use jflib::{img_all, img_core::io, plat};

io::set_file_io(io::FileIo { read: plat::read_file, write: plat::write_file });
img_all::use_all_codecs();                        // or use_core_codecs(): PNG, JPEG, BMP, GIF
```

`img_core::set_parallel` hands img_core a loop for its parallel parts, such as the grids of colour
transforms; an app's runtime passes the platform's worker pool. Without one everything runs on
the calling thread and gives the same bytes.

## What's new in 0.2.0

A small correction: the format table gives Paint.NET files the MIME type `image/x-paint.net`; it
was `image/x-PaintCraft`, a leftover of a renaming. The
[changelog](https://docs.rs/crate/jflib-img-core/latest/source/CHANGELOG.md) says the same.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_core): every public
  item of `img_core`, also on [docs.rs](https://docs.rs/jflib-img-core)

## License

MIT or Apache-2.0, at your choice. The compressor follows zlib's compression levels and miniz's way
of limiting code lengths, and the decompressor builds its Huffman tables as stb_image does; those
parts also stand under the licenses of their sources (Zlib, MIT, and MIT or Unlicense), whose texts
are in the crate's [`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/img_core/licenses)
folder.
