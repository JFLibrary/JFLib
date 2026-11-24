// zstd -- the Zstandard format (RFC 8878), interoperable with the zstd tool both ways
//
//   compress(data, level) -> frame        levels 1..=19 (LEVEL_DEFAULT 3); content size and
//                                         checksum in the frame; window up to 8 MiB
//   decompress(frames, max_out)           any conforming frames, concatenated and skippable
//                                         ones too; None on corruption, a checksum mismatch,
//                                         > max_out bytes or a frame that names a dictionary
//   Dictionary::train(samples, max) / parse / raw / to_bytes   dictionaries as `zstd --train`
//                                         makes them (dict.rs, train.rs)
//   compress_with_dict / decompress_with_dict   frames with a dictionary (zstd -D)
//   content_size(frame), dictionary_id(frame)   what a frame header declares
//   bound(n)                              the largest frame compress() makes from n bytes
//   Decoder / Encoder                     in pieces (stream::Decode / Encode; sdec.rs, senc.rs):
//                                         windows up to 128 MiB (with_window_max), with_size
//                                         for the bytes of compress(), with_dictionary

mod bits;
mod dec;
mod dict;
mod enc;
mod fse;
mod huf;
mod sdec;
mod senc;
mod seq;
mod train;

pub use dec::{content_size, decompress, decompress_with_dict, dictionary_id};
pub use dict::Dictionary;
pub use enc::{bound, compress, compress_with_dict};
pub use sdec::Decoder;
pub use senc::Encoder;

pub const LEVEL_DEFAULT: i32 = 3;
pub const LEVEL_MAX: i32 = 19;
