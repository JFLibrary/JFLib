# jflib-img-pdn

Reading and writing paint.net documents (`.pdn`) for [JFLib](https://jflib.org), with all their
layers, in plain Rust, without the standard library and without a line of `unsafe` code. paint.net
is an image editor for Windows, and its documents keep what a flat image loses: every layer with
its name, opacity, visibility and blend mode. The crate reads and writes them compatibly with
paint.net 5.1.

## What it reads

A paint.net document is a small XML header with the size and a thumbnail, then a .NET
BinaryFormatter object graph, then the layers' pixels in compressed chunks. The reader understands
the files of paint.net 5.1 and older ones: the gzip-wrapped layout, the bare gzip files of
paint.net 2.0 and earlier, and the way paint.net 3.x stored metadata, blend modes and pixel data.
It returns a `PdnDoc` with the size, the resolution (its unit and the dots per unit), the
document's metadata, the ICC profile stored there, and the layers from the bottom up. Each layer
has its pixels (8-bit, straight alpha), its name, opacity, visibility, its blend mode (paint.net's
fourteen, from Normal to Xor), whether it is the background, and its own metadata. Chunks are
decompressed in parallel on the worker pool.

Documents are treated as untrusted. The size must fit the pixel budget (`ImgDec::maxpx`, by default
268 million pixels), damaged or missing chunks are errors, every layer must have pixel data of its
own (`ERR_HDR` when a second layer points at the same block), and the layers' names and metadata
together stay in proportion to the file (`ERR_LIMIT`). `decode_ex` with `PDND_PROBE` reads only the
size, the resolution and the metadata.

## What it writes

The writer builds the object graph the way paint.net 5.1 does and compresses the pixels in chunks
of 256 KiB, in parallel; the effort is the deflate level, 1 by default. It adds a thumbnail to the
header and writes the resolution and the ICC profile into the metadata, keeping every other item.
It takes the layers as a borrowed `DocRef`, so an app hands over its own layer surfaces without
copying them; they may be opaque, premultiplied, straight or alpha-only.

## How it is used

Through `img_core::io`, once the codecs are registered, a `.pdn` file opens like any flat image:
loading gives the visible layers composed with paint.net's blend modes and opacity, and saving
writes a document with one background layer. An app that works with the layers calls the crate:

```rust
use jflib::{img_pdn, plat};

if let Some(data) = plat::read_file("drawing.pdn") {
    match img_pdn::decode(&data, None) {
        Ok(doc) => {
            for layer in &doc.layers {                     // the bottom layer first
                // layer.sf: its pixels; layer.name, opacity, visible, blend, meta
            }
            let picture = img_pdn::flatten(&doc.view());    // the visible layers as one image
            let file = img_pdn::encode(&doc.view(), None);  // the document as a .pdn again
        }
        Err(code) => { /* an ERR_* code of img_core */ }
    }
}
```

The .NET BinaryFormatter format the documents are built on has a reader and a writer of its own in
`img_pdn::nrbf`.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_pdn`. Its own functions work at once;
for `img_core::io` to know `.pdn`, call `img_all::use_all_codecs()` once, since JFLib registers
only PNG, JPEG, BMP, GIF and SVG by itself. All of JFLib's crates share one version and are
released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

The reader is safer against hostile documents. A block of pixels or an inline byte array that more
than one layer points at is refused with `ERR_HDR`; before, every reference expanded the same data
again. And the layers' names and metadata together are bounded by the size of the document
(`ERR_LIMIT`). Everything is in the
[changelog](https://docs.rs/crate/jflib-img-pdn/latest/source/CHANGELOG.md).

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_pdn), also on
  [docs.rs](https://docs.rs/jflib-img-pdn)

## License

MIT or Apache-2.0, at your choice.
