# jflib-compress

Lossless compression for [JFLib](https://jflib.org): DEFLATE, zlib, gzip, Brotli, LZ4 and
Zstandard in plain Rust, without the standard library, without a line of `unsafe` code and
without a dependency. All formats are the real ones: what is written here opens with `gzip`,
`zlib`, `lz4` and `zstd`, and what those tools write opens here (Zstandard as long as it needs no
dictionary).

## What it does

**DEFLATE, zlib and gzip** are the formats of the web, of ZIP files, PNG images and PDF streams.
`deflate`, `zlib` and `gzip` each have `compress(data, level)` at levels 0 to 9
(`deflate::LEVEL_DEFAULT` is 6, zlib's default) and `decompress(data, max_out)`. The encoder gives
the same bytes on every platform. `gzip::decompress` reads all members of a file one after the
other, as `gunzip` does. For codecs that handle many small pieces there are reusable states
(`deflate::DeflateState`, `deflate::InflateState`) and variants that work in the caller's buffer
and report how much input a stream took (`decompress_into`, `compress_into`).

**Brotli** is the `br` of HTTP and the compression of WOFF2 fonts. `brotli::decompress(data,
max_out)` reads any stream of RFC 7932 with its static dictionary; there is no Brotli encoder.

**LZ4** is the fast one. `lz4::compress` and `lz4::decompress` work on single blocks,
`lz4::frame_compress` and `lz4::frame_decompress` on the LZ4 frame format with its xxHash32
checksums. JFLib's build packs an app's `assets/` with it.

**Zstandard** is the strongest, the one for archives and backups. `zstd::compress(data, level)`
writes a frame with the content size and a checksum, at levels 1 to 19 (`zstd::LEVEL_DEFAULT` is
3). `zstd::decompress` reads any conforming frames that do not need a dictionary, several in a row
and skippable ones included, as RFC 8878 describes them.

Every decoder takes the largest output the caller accepts and treats its input as untrusted:
truncated or corrupt data, a bad checksum, bytes after the stream or more output than allowed are
refused (`None`), never a panic and never a read or write outside the buffers. A decoder's buffer
grows with the output it actually produces, so a small file that claims a huge size costs nothing.

```rust
use jflib::compress::{gzip, zstd, Codec};

let packed = gzip::compress(&data, 6);
let back = gzip::decompress(&packed, 64 << 20);   // None: damaged, or more than 64 MiB

let packed = zstd::compress(&data, zstd::LEVEL_DEFAULT);
let back = zstd::decompress(&packed, 64 << 20);

let codec = Codec::from_name("lz4").unwrap_or(Codec::None);
let blob = codec.pack(&data, 0);                  // the level counts for zstd only
let id = codec.id();                              // the byte a file stores beside the blob
let again = Codec::from_id(id).and_then(|c| c.unpack(&blob, 64 << 20));
```

For formats that store blobs with their compression, `Codec` (`None`, `Lz4`, `Zstd`) has a byte
to store beside each blob and a name for configuration files.

The checksums of the formats are there on their own as well: `crc32` (gzip, PNG, ZIP), `adler32`
(zlib), `xxh32` and `xxh64`. They find accidental damage and are not cryptographic.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::compress`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.1

Nothing in this crate. It is released again as 0.2.1 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-compress/latest/source/CHANGELOG.md) says the
same.

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#17-network-programs) on the
  crates for servers, clients and backups
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#compress): every public
  item of `compress`, also on [docs.rs](https://docs.rs/jflib-compress)

## License

MIT or Apache-2.0, at your choice. The DEFLATE encoder follows zlib's compression levels and
miniz's way of limiting code lengths, the decoder builds its Huffman tables as stb_image does, and
the Brotli decoder carries Brotli's static dictionary; those parts also stand under the licenses
of their sources (Zlib, MIT, and MIT or Unlicense), whose texts are in the crate's
[`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/compress/licenses) folder.
