// zstd -- the Zstandard format (RFC 8878), interoperable with the zstd tool both ways
//
//   compress(data, level) -> frame        levels 1..=19 (LEVEL_DEFAULT 3); content size and
//                                         checksum in the frame; window up to 8 MiB
//   decompress(frames, max_out)           any conforming frames without a dictionary,
//                                         concatenated and skippable ones too; None on
//                                         corruption, a checksum mismatch or > max_out bytes
//   content_size(frame)                   the size a frame header declares
//   bound(n)                              the largest frame compress() makes from n bytes

mod bits;
mod dec;
mod enc;
mod fse;
mod huf;
mod seq;

pub use dec::{content_size, decompress};
pub use enc::{bound, compress};

pub const LEVEL_DEFAULT: i32 = 3;
pub const LEVEL_MAX: i32 = 19;
