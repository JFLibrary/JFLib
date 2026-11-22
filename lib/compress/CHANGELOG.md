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

### Changed

- zstd::compress uses the tables the decoder already has when that is cheaper: the literals with the previous block's Huffman tree (treeless), a sequence code in Repeat_Mode. Frames of several blocks get up to 4% smaller; the zstd tool reads them as before

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
