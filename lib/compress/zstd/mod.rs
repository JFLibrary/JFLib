// zstd -- the Zstandard format (RFC 8878), interoperable with the zstd tool both ways
//
//   compress(data, level) -> frame        levels 1..=19 (LEVEL_DEFAULT 3); content size and
//                                         checksum in the frame; window up to 8 MiB
//   decompress(frames, max_out)           any conforming frames without a dictionary,
//                                         concatenated and skippable ones too; None on
//                                         corruption, a checksum mismatch or > max_out bytes
//   content_size(frame)                   the size a frame header declares
//   bound(n)                              the largest frame compress() makes from n bytes
//   Decoder / Encoder                     in pieces (stream::Decode / Encode; sdec.rs, senc.rs):
//                                         windows up to 128 MiB (with_window_max), with_size
//                                         for the bytes of compress()

mod bits;
mod dec;
mod enc;
mod fse;
mod huf;
mod sdec;
mod senc;
mod seq;

pub use dec::{content_size, decompress};
pub use enc::{bound, compress};
pub use sdec::Decoder;
pub use senc::Encoder;

pub const LEVEL_DEFAULT: i32 = 3;
pub const LEVEL_MAX: i32 = 19;
