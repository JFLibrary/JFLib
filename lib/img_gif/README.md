# jflib-img-gif

GIF reading and writing for [JFLib](https://jflib.org), in plain Rust, without the standard
library and without a line of `unsafe` code.

The decoder reads GIF87a and GIF89a: global and local colour tables, interlaced images, LZW clear
codes wherever they appear, and the transparent colour of the Graphic Control Extension. Of an
animation it decodes the first frame, placed at its offset on a transparent canvas the size of the
logical screen; the metadata still counts the frames and gives the first frame's delay and the loop
count. A file that breaks off or carries corrupt LZW data after the image header still opens: the
pixels that could not be decoded stay transparent. Canvases beyond 32,768 pixels on a side or 268
million pixels in all are refused. An ICC profile stored in the usual application extension is read
on request.

The encoder writes a GIF89a file with one image. An image with at most 256 colours keeps exactly
those colours; anything richer is reduced to a palette with Wu's quantizer and Floyd-Steinberg
dithering, whose strength you can choose. Pixels below an alpha threshold become the transparent
colour. GIF knows only fully transparent and fully opaque pixels, so soft edges are decided by
that threshold.

## How it is used

Most apps never call this crate by name. JFLib registers GIF at start-up, so the image functions
of [`jflib-img-core`](https://crates.io/crates/jflib-img-core) open and save GIF files like any
other format (`img_core::io::load_ex`, `save_opt`; `ImgOpt::dither` and `ImgOpt::athresh` set the
dithering and the transparency threshold). `img_gif::encode_q` takes the same choices directly,
and `img_gif::decode` reads a file already in memory.

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_gif`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-img-gif/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_gif), also on
  [docs.rs](https://docs.rs/jflib-img-gif)

## License

MIT or Apache-2.0, at your choice.
