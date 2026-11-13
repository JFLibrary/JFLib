# jflib-img-tga

TGA (Truevision Targa) reading and writing for [JFLib](https://jflib.org), in plain Rust, without
the standard library and without a line of `unsafe` code.

The decoder reads uncompressed and run-length encoded files of all three kinds: colour-mapped with
8- or 16-bit indices into a map of 15, 16, 24 or 32-bit entries, true colour with 15, 16, 24 or 32
bits, and grey with 8 bits or with an alpha byte beside it. All four origins are understood, and
run-length packets may cross rows. Whether the alpha channel means anything is decided as the
format intends: a TGA 2.0 extension area says straight or premultiplied alpha or none, and without
one an alpha channel that is zero everywhere counts as opaque. The pixel data's minimum size is
checked before any memory is allocated, so a file that claims more than it holds is refused at
once.

TGA has no signature, so a file is taken for TGA only when its name ends in `.tga` or a strict
check of its header accepts it.

The encoder writes 32-bit pixels with straight alpha, or 24-bit pixels with transparent parts
composited over white, optionally run-length encoded with packets that never cross a row, and adds
the TGA 2.0 extension area and footer that tell other programs how to read the alpha.

## How it is used

TGA is not among the formats JFLib registers at start-up. Call `img_all::use_all_codecs()` of
[`jflib-img-all`](https://crates.io/crates/jflib-img-all) once (in `App::init`), and the image
functions of [`jflib-img-core`](https://crates.io/crates/jflib-img-core) open and save TGA files
like any other format (`img_core::io::load_ex`, `save_opt`; `ImgOpt::bitdepth` chooses 32 or 24
bits, the flag `IOF_RLE` compression). The defaults of `img_core::io::opt_init` are 32 bits with
run-length encoding.

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_tga`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-img-tga/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_tga), also on
  [docs.rs](https://docs.rs/jflib-img-tga)

## License

MIT or Apache-2.0, at your choice.
