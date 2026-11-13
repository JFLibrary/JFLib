# jflib-img-all

One call that gives a [JFLib](https://jflib.org) program every raster image format the library
can read and write. Each codec of JFLib is a crate of its own, and an image becomes readable only
once its codec is entered into the format table of
[`jflib-img-core`](https://crates.io/crates/jflib-img-core). This crate does that for all of
them, and it holds a reader and writer for Photoshop files.

## Registering the codecs

`img_all::use_all_codecs()` registers PNG, JPEG, BMP, GIF, TIFF, TGA, DDS, WebP, AVIF, HEIC,
JPEG XL, JPEG XR, Paint.NET and Photoshop. From then on the functions of `img_core::io` open and
save those files, and the file dialog's list of formats (`io::filter_list`) shows them. Call it
once at start-up, before other threads decode images; in an app with a window, `App::init` is the
place:

```rust
fn init(&mut self, _ui: &mut Ui) -> bool {
    jflib::img_all::use_all_codecs();
    true
}
```

`img_all::use_core_codecs()` registers only PNG, JPEG, BMP and GIF. The runtime of an app with a
window calls it at start-up, together with SVG's registration, so those formats work without a
line of code. A command-line program calls one of the two itself and also gives img_core its file
functions (`img_core::io::set_file_io`, see that crate's page). SVG is not part of either call:
`img_svg::register()` of [`jflib-img-svg`](https://crates.io/crates/jflib-img-svg) enters it.

## Photoshop files

`img_all::psd` reads PSD files and PSB, Photoshop's format for large documents: RGB and greyscale
images with 8, 16 or 32 bits per channel, uncompressed, PackBits or ZIP-compressed with or without
prediction. Through the format table it gives the merged image Photoshop stores beside the layers,
converted to 8 bits per channel, together with the resolution and, on request, the ICC profile,
EXIF and XMP. Saving writes an 8-bit RGB image with alpha, its resolution and, on request, the
same metadata.

For an editor that works with layers, `img_all::psd::document` goes further. `decode` returns the
raster layers (or `None` for a file without any), bottom layer first, each at the size of the canvas
with its name, opacity, visibility and blend mode; layer masks are applied to the alpha. Whatever it
had to convert to get there (masks, pixels outside the canvas, a higher bit depth, PSB, blocks it
does not know) sets the document's `converted` flag, so the app can warn before it overwrites the
original. Layer groups, vector masks, layer effects, clipping masks and blend modes without an exact
counterpart are refused with `ERR_UNSUPPORTED`; the merged image remains readable then. `encode`
writes an 8-bit layered PSD with a merged image for other programs, and it refuses a blend mode
Photoshop cannot show exactly. The modes that pass are normal, multiply, additive, colour burn,
colour dodge, overlay, difference, lighten, darken and screen.

Like the other decoders, the reader treats a file as untrusted: sizes are checked against
img_core's pixel and memory budgets before anything is allocated, at most 4,096 layers are read,
and a long decode can be cancelled.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_all`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. Some of the codecs it registers did change: the TIFF, HEIC, JPEG XL, JPEG XR and
Paint.NET decoders refuse more kinds of hostile files, as their own changelogs describe. This
crate's [changelog](https://docs.rs/crate/jflib-img-all/latest/source/CHANGELOG.md) lists no
changes of its own.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_all), also on
  [docs.rs](https://docs.rs/jflib-img-all)

## License

MIT or Apache-2.0, at your choice.
