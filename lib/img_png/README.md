# jflib-img-png

PNG reading and writing for [JFLib](https://jflib.org), in plain Rust, without the standard library
and without a line of `unsafe` code.

The decoder reads every kind of PNG: grey, colour and palette images, each with or without
transparency, bit depths from 1 to 16, and interlaced images. It checks the checksums of the parts
that matter and refuses images beyond 32,768 pixels on a side or 268 million pixels in all, so a
hostile file cannot exhaust memory. Pixels come out premultiplied, the way JFLib draws them, or
with straight alpha if you ask for it. Of an animated PNG it reads the still image that programs
without animation support show.

The encoder picks the smallest fitting form by itself. An image with at most 256 colours becomes a
palette image without losing anything, an opaque one is stored without an alpha channel, and each
row gets the filter that promises the best compression. You can also choose the form yourself,
or reduce a photo to 256 colours with dithering. Metadata travels along: the resolution, the ICC
colour profile, EXIF and XMP are read and can be written back.

## How it is used

Most apps never call this crate by name. JFLib registers PNG at start-up, so the image functions
of `img_core` open and save PNG files like any other format:

```rust
use jflib::img_core::io;

match io::load_ex("photo.png", None, None) {
    Ok(surface) => {
        let mut opt = io::opt_init(io::format_from_path("copy.png"));
        opt.effort = 9;                           // the strongest compression
        io::save_opt("copy.png", &surface, Some(&opt));
    }
    Err(code) => { /* an ERR_* code of img_core */ }
}
```

The crate's own functions are there for work closer to the format: `img_png::encode_cfg` with a
`PngCfg` for full control over depth, compression and extra chunks, `img_png::decode_fl` for a
file already in memory.

Like all of JFLib, the crate comes through the one dependency [`jflib`](https://crates.io/crates/jflib),
as `jflib::img_png`. All of JFLib's crates share one version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-img-png/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_png), also on
  [docs.rs](https://docs.rs/jflib-img-png)

## License

MIT or Apache-2.0, at your choice.
