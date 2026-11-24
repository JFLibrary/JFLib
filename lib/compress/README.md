# jflib-compress

Lossless compression for [JFLib](https://jflib.org): DEFLATE, zlib, gzip, Brotli, LZ4 and
Zstandard in plain Rust, without the standard library, without a line of `unsafe` code and
without a dependency. All formats are the real ones: what is written here opens with `gzip`,
`zlib`, `lz4`, `zstd`, `brotli` and every browser, and what those tools write opens here,
Zstandard with dictionaries too. Each also comes in pieces, with bounded memory, for data that
is too large to hold or that arrives over time.

## What it does

**DEFLATE, zlib and gzip** are the formats of the web, of ZIP files, PNG images and PDF streams.
`deflate`, `zlib` and `gzip` each have `compress(data, level)` at levels 0 to 9
(`deflate::LEVEL_DEFAULT` is 6, zlib's default) and `decompress(data, max_out)`. The encoder gives
the same bytes on every platform. `gzip::decompress` reads all members of a file one after the
other, as `gunzip` does. For codecs that handle many small pieces there are reusable states
(`deflate::DeflateState`, `deflate::InflateState`) and variants that work in the caller's buffer
and report how much input a stream took (`decompress_into`, `compress_into`).

**Brotli** is the `br` of HTTP and the compression of WOFF2 fonts. `brotli::compress(data,
level)` writes a stream at levels 0 to 11 (`brotli::LEVEL_DEFAULT` is 5, for the responses of a
web server; `LEVEL_MAX` for files compressed once and sent often), with the smallest window that
holds the data. It uses what makes Brotli smaller than gzip: a window of up to 16 MiB, the static
dictionary of 13 504 words and pieces of HTML in several languages with their transforms, the
context of the two bytes before each literal, and at levels 10 and 11 a near-optimal parse. On
JFLib's guide (430 KB of Markdown) level 5 makes 158 KB (gzip -9: 173 KB, libbrotli at the
same level: 163 KB), level 11 147 KB (libbrotli: 145 KB). libbrotli is 1.5 to 3 times as fast
at levels 0 to 9 and about as fast at 10 and 11. `brotli::decompress(data, max_out)` reads any
stream of RFC 7932; libbrotli decodes up to 1.5 times as fast.

```rust
use jflib::compress::brotli;

let body = brotli::compress(page.as_bytes(), brotli::LEVEL_DEFAULT);  // Content-Encoding: br
let font = brotli::compress(&tables, brotli::LEVEL_MAX);              // once, for a WOFF2 file
let back = brotli::decompress(&body, 16 << 20);
```

**LZ4** is the fast one. `lz4::compress` and `lz4::decompress` work on single blocks,
`lz4::frame_compress` and `lz4::frame_decompress` on the LZ4 frame format with its xxHash32
checksums. `lz4::compress_hc(data, level)` searches harder for the same block format, as `lz4 -9`
does: slower, a few percent smaller, as fast to decompress. JFLib's build packs an app's
`assets/` with LZ4, cargo-jflib the AppImage with `compress_hc`.

**Zstandard** is the strongest, the one for archives and backups. `zstd::compress(data, level)`
writes a frame with the content size and a checksum, at levels 1 to 19 (`zstd::LEVEL_DEFAULT` is
3). `zstd::decompress` reads any conforming frames, several in a row and skippable ones included,
as RFC 8878 describes them.

**Dictionaries** make small pieces of data compress well: records of a log, messages, rows of a
database, each too short to have much to repeat on its own. A dictionary holds what such pieces
have in common. `zstd::Dictionary::train(&samples, 16 << 10)` learns one from samples of the
data, as `zstd --train` does, and `to_bytes` / `Dictionary::parse` store and read it in the same
format, so the zstd tool takes ours (`zstd -D`) and we take its. `compress_with_dict` and
`decompress_with_dict` use it; the frame names the dictionary's ID. Bytes that are no dictionary
of that format, or `Dictionary::raw(content)`, are content only. On 600 JSON records of 225
bytes each, a 16 KiB dictionary makes them 3.15 times smaller instead of about 1.2 times without
one; the zstd tool with a dictionary of its own training reaches 3.22. The stream `Encoder` and
`Decoder` take dictionaries too (`with_dictionary`).

```rust
use jflib::compress::zstd::{self, Dictionary};

let samples: Vec<&[u8]> = records.iter().map(|r| r.as_slice()).collect();
let dict = Dictionary::train(&samples, 16 << 10).unwrap();
let packed = zstd::compress_with_dict(&record, 3, &dict);
let back = zstd::decompress_with_dict(&packed, 1 << 20, &dict);
let file = dict.to_bytes();                        // what `zstd -D file` reads
```

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

**Streams** are for data too large to hold, or that arrives in pieces: a download, a file of
gigabytes, a socket. Every format has a decoder and an encoder that take their input in pieces
and write into a buffer of the caller (`compress::stream`): `deflate::Decoder` and `Encoder`, the
same for `zlib` and `gzip`, `zstd::Decoder` and `Encoder`, `lz4::FrameDecoder` and
`FrameEncoder`, `brotli::Decoder` and `Encoder`. A call returns how much input it took, how much
it wrote, and whether it wants more input, more room or is at the end. The memory stays bounded
by the format: 32 KiB of window for DEFLATE, the frame's window for zstd and Brotli (the Brotli
encoder's `with_window` makes it smaller, for decoders with little memory), 64 KiB and one block
for LZ4. A small piece of input never makes more output than the buffer holds, so a compression
bomb is the caller's to stop, not a large allocation. Encoders can flush (`Flush::Sync`) so that
a reader can decode everything so far, as for a log or a live connection. Up to 256 KiB, the
DEFLATE, zlib and gzip encoders write the same bytes as `compress()`; the zstd, LZ4 and Brotli
encoders do for any size once they are told it (`with_size`).
`stream::decode_all` and `encode_all` pump a whole stream between a reader and a writer.

```rust
use jflib::compress::{gzip, stream, zstd};
use jflib::plat;

// a file of any size, gunzipped in pieces of 64 KiB, at most 10 GiB of output
let src = plat::File::open("server.log.gz", plat::FO_READ).unwrap();
let dst = plat::File::open("server.log", plat::FO_WRITE).unwrap();
let (mut at, mut to) = (0u64, 0u64);
let n = stream::decode_all(
    &mut gzip::Decoder::new(),
    |buf| src.read_at(at, buf).inspect(|&k| at += k as u64),
    |bytes| dst.write_at(to, bytes).inspect(|&k| to += k as u64) == Some(bytes.len()),
    10 << 30,
);

// or piece by piece, as the data comes in
let mut dec = zstd::Decoder::new();
let mut buf = [0u8; 16 << 10];
let step = dec.decode(&chunk, &mut buf)?;   // step.read, step.written, step.status
```

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
Brotli's decoder and encoder carry its static dictionary, the decoder builds its prefix-code
tables and the encoder scores matches, estimates costs and writes its prefix codes as libbrotli
does; those parts also stand under the licenses of
their sources (Zlib, MIT, and MIT or Unlicense), whose texts are in the crate's
[`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/compress/licenses) folder.
