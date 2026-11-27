# Changelog of jflib-compress

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- DEFLATE, zlib and gzip in both directions, the Brotli decoder, CRC-32 and Adler-32, moved here from img_core and img_jxl: `compress::deflate`, `compress::zlib` and `compress::gzip` with `compress(data, level)` and `decompress(data, max_out)` (strict: data after the stream is refused; `gzip::decompress` reads every member, as gunzip does), reusable states (`deflate::DeflateState`, `deflate::InflateState`) and `*_into` variants in the caller's buffer that report the input a stream took, `deflate::Error`, `deflate::bound`; `compress::brotli::{decompress, decompress_prefix, Error}` (RFC 7932, no encoder); `compress::{crc32, adler32}`. The encoder gives the same bytes as before at every level, the decoder the same errors; the output buffer of `decompress` grows with the data, up to max_out. compress still depends on nothing, so the build tools with std use it too.
- Streams: every format in pieces, with bounded memory (compress::stream). Decode::decode(input, out) and Encode::encode(input, out, flush) take input as it comes and write into the caller's buffer; Step { read, written, status } says whether a call wants more input (Status::Input), more room (Output) or is at the end (End, with read stopping at the stream's end). deflate::, zlib:: and gzip::Decoder and Encoder, zstd::Decoder and Encoder, lz4::FrameDecoder and FrameEncoder, brotli::Decoder; gzip, zstd and LZ4 read several members or frames in a row as their tools do (Decoder::single() stops after the first). The memory is the format's window: 32 KiB for DEFLATE, the frame's for zstd (refused above 128 MiB by default, with_window_max) and Brotli, 64 KiB and one block for LZ4. Flush::Sync makes everything so far decodable (an empty stored block for DEFLATE, a whole block for zstd and LZ4). Up to 256 KiB the DEFLATE, zlib and gzip encoders write the same bytes as compress(), the zstd and LZ4 encoders for any size once given it (with_size; Error::Size when the input differs). stream::decode_all(dec, read, write, limit) and encode_all(enc, read, write) pump between a reader and a writer; Xxh32 and Xxh64 hash data in pieces
- Zstandard dictionaries: zstd::Dictionary::train(samples, max_size) learns one as zstd --train does (fastCover; k and d chosen on a quarter of the samples; the Huffman and FSE tables from compressing the samples, every symbol kept usable), Dictionary::parse / to_bytes read and write the zstd tool's format (raw content without its magic, or Dictionary::raw), compress_with_dict / decompress_with_dict and dictionary_id(frame); zstd::Encoder and Decoder take them with with_dictionary (the decoder several, chosen by ID; Error::Dictionary for a frame that names one not given). The zstd tool reads our frames and dictionaries and we read its (zstd -D both ways); 600 JSON records of 225 bytes get 3.15 times smaller with a 16 KiB dictionary of ours (the tool's own: 3.22; without one about 1.2)
- A Brotli encoder: brotli::compress(data, level) at levels 0 to 11 (brotli::LEVEL_DEFAULT 5, LEVEL_MAX 11) and brotli::Encoder in pieces (stream::Encode; with_size gives the bytes of compress(), with_window a smaller window, Flush::Sync makes everything so far readable), brotli::bound. Levels 0 to 9 match over hash buckets as libbrotli does, with lazy matching; levels 10 and 11 parse near-optimally (11 in two rounds); the static dictionary with a space before a word, its first letter in upper case, 44 suffixes and up to 9 letters cut; the literals in 64 contexts (UTF-8 or signed) merged into clusters. JFLib's guide (430 KB) takes 158 KB at level 5 (libbrotli at the same level 163 KB, gzip -9 173 KB) and 147 KB at level 11 (libbrotli 145 KB); libbrotli is 1.5 to 3 times as fast at levels 0 to 9, about as fast at 10 and 11. libbrotli and the browsers read the streams
- lz4::compress_hc(data, level) and LEVEL_HC_DEFAULT (9): the LZ4 block format with the longest match along a hash chain of 2^(level-1) candidates, as lz4 -9; a few percent smaller than lz4::compress, as fast to decompress

### Changed

- zstd::compress uses the tables the decoder already has when that is cheaper: the literals with the previous block's Huffman tree (treeless), a sequence code in Repeat_Mode. Frames of several blocks get up to 4% smaller; the zstd tool reads them as before
- The Brotli decoder is 2.2 to 4.8 times as fast, brotli::decompress / decompress_prefix and brotli::Decoder alike: a bit reader with a 64-bit buffer refilled bytewise instead of single bits; prefix codes as lookup tables (8 bits, then second-level tables, as libbrotli builds them) instead of a walk bit by bit; the literals of a block in one run, without the context when all 64 contexts of a block type name one tree; copies of up to 32 bytes as one copy of a fixed size, dictionary words without an allocation. brotli::Decoder runs whole commands at once while the input surely holds them and the output has room, and goes step by step only near the ends. The same streams are accepted and refused as before, with the same errors, and brotli::Decoder returns the same Step for every call. On our streams and libbrotli's (levels 1, 5, 9, 11) of five kinds of data, the one-shot decoder now reads 270 to 800 MB/s (before 107 to 263), the stream decoder 250 to 790 (before 102 to 277); libbrotli 340 to 980

### Fixed

- Brotli: compressing text with dictionary words of 4 to 8 letters no longer subtracts past zero (a panic where overflow checks are on, as in the debug builds of apps)

## [0.2.1] - 2026-11-16

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.2.0] - 2026-11-14

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
