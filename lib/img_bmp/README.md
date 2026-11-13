# jflib-img-bmp

BMP reading and writing for [JFLib](https://jflib.org), in plain Rust, without the standard
library and without a line of `unsafe` code.

The decoder reads the Windows bitmap in all its common forms: the old OS/2 core header and every
info header from version 1 to 5, palette images with 1, 4 or 8 bits, 16-bit pixels in 5-5-5 or
with bit fields, 24-bit and 32-bit pixels, the latter with any channel masks and an alpha mask,
rows stored bottom-up or top-down. Run-length encoded files (RLE8 and RLE4) are decoded with
every escape; pixels the stream skips stay transparent, and runs that would leave the image are
clipped. A BMP that wraps a PNG or a JPEG is decoded by JFLib's codecs for those formats. Files
beyond 32,768 pixels on a side or 268 million pixels in all are refused, and so are the rare OS/2
compressions, RLE24 and CMYK bitmaps, as unsupported. The resolution, the colour space of the
newer headers and an embedded ICC profile are read as metadata.

The encoder picks the form by itself. An opaque image with at most 256 colours becomes an 8-bit
palette image without losing anything, another opaque image a 24-bit one, and an image with
transparency a 32-bit bitmap with a version 4 header and an alpha mask. You can also ask for 32,
24 or 8 bits; with 8 bits a photo is reduced to 256 colours with dithering, composited over white
because BMP palettes have no alpha. The resolution comes from the image's metadata, 72 dpi without
it.

## How it is used

Most apps never call this crate by name. JFLib registers BMP at start-up, so the image functions
of [`jflib-img-core`](https://crates.io/crates/jflib-img-core) open and save BMP files like any
other format (`img_core::io::load_ex`, `save_opt`; `ImgOpt::bitdepth` chooses 32, 24 or 8 bits).
`img_bmp::encode_cfg` with a `BmpCfg` gives the same choices directly, including the resolution
in pixels per metre, and `img_bmp::decode` reads a file already in memory.

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_bmp`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-img-bmp/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_bmp), also on
  [docs.rs](https://docs.rs/jflib-img-bmp)

## License

MIT or Apache-2.0, at your choice.
