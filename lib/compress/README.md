# jflib-compress

Lossless compression for [JFLib](https://jflib.org): LZ4 and Zstandard in plain Rust, without the
standard library and without a line of `unsafe` code. Both formats are the real ones: frames
written here open with the `lz4` and `zstd` command-line tools, and what those tools write opens
here, as long as it needs no dictionary.

## What it does

**LZ4** is the fast one. `lz4::compress` and `lz4::decompress` work on single blocks,
`lz4::frame_compress` and `lz4::frame_decompress` on the LZ4 frame format with its xxHash32
checksums. JFLib's build packs an app's `assets/` with it.

**Zstandard** is the stronger of the two, the one for archives and backups.
`zstd::compress(data, level)` writes a frame with the content size and a checksum, at levels 1 to
19 (`zstd::LEVEL_DEFAULT` is 3). `zstd::decompress` reads any conforming frames that do not need
a dictionary, several in a row and skippable ones included, as RFC 8878 describes them.

Every decoder takes the largest output the caller accepts and treats its input as untrusted:
truncated or corrupt data, a bad checksum or more output than allowed are refused (`None`), never
a panic and never a read or write outside the buffers.

For formats that store blobs with their compression, `Codec` (`None`, `Lz4`, `Zstd`) has a byte
to store beside each blob and a name for configuration files:

```rust
use jflib::compress::{zstd, Codec};

let packed = zstd::compress(&data, zstd::LEVEL_DEFAULT);
let back = zstd::decompress(&packed, 64 << 20);   // None: damaged, or more than 64 MiB

let codec = Codec::from_name("lz4").unwrap_or(Codec::None);
let blob = codec.pack(&data, 0);                  // the level counts for zstd only
let id = codec.id();                              // the byte a file stores beside the blob
let again = Codec::from_id(id).and_then(|c| c.unpack(&blob, 64 << 20));
```

The two checksums of the formats are there on their own as well: `xxh32` and `xxh64`. They find
accidental damage and are not cryptographic.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::compress`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-compress/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#17-network-programs) on the
  crates for servers, clients and backups
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#compress): every public
  item of `compress`, also on [docs.rs](https://docs.rs/jflib-compress)

## License

MIT or Apache-2.0, at your choice.
