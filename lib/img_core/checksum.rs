// checksum -- CRC-32 (zlib / PNG polynomial) and Adler-32
//
// Both live in jflib-compress since 0.3 (compress::crc32, compress::adler32); img_core keeps
// the names of before.
//
//   crc32(crc, data) -> u32        start with crc = 0 (like zlib's crc32())
//   adler32(adler, data) -> u32    start with adler = 1

pub use compress::{adler32, crc32};
