// compress -- lossless compression for backup data
//
//   lz4::compress(data) -> Vec<u8>                  LZ4 block (greedy, 64 KiB window)
//   lz4::decompress(block, max_out) -> Option<Vec>  bounds-checked; None on any corruption
//   lz4::frame_compress / frame_decompress          the LZ4 frame format (lz4 / zstd --format=lz4
//                                                   interoperable), xxHash32 checksums
//   zstd::compress(data, level) / decompress        Zstandard frames (RFC 8878; the zstd tool
//                                                   reads ours and we read its)
//   xxh32(data, seed), xxh64(data, seed)
//   Codec { None, Lz4, Zstd } with id(): the codec byte that formats on disk store per blob;
//   Codec::pack(data, level) / unpack(data, max_out)
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;

pub mod lz4;
mod xxh;
pub mod zstd;

pub use xxh::{xxh32, xxh64};

/// the compression of a stored blob (the byte on disk)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    None = 0,
    Lz4 = 1,
    Zstd = 2,
}

impl Codec {
    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn from_id(b: u8) -> Option<Codec> {
        match b {
            0 => Some(Codec::None),
            1 => Some(Codec::Lz4),
            2 => Some(Codec::Zstd),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Codec::None => "none",
            Codec::Lz4 => "lz4",
            Codec::Zstd => "zstd",
        }
    }

    pub fn from_name(s: &str) -> Option<Codec> {
        match s {
            "none" => Some(Codec::None),
            "lz4" => Some(Codec::Lz4),
            "zstd" => Some(Codec::Zstd),
            _ => None,
        }
    }

    /// compress with this codec (`level`: zstd only); None stores the data as it is
    pub fn pack(self, data: &[u8], level: i32) -> Vec<u8> {
        match self {
            Codec::None => Vec::from(data),
            Codec::Lz4 => lz4::compress(data),
            Codec::Zstd => zstd::compress(data, level),
        }
    }

    /// the data of pack(); None when it is damaged or longer than max_out
    pub fn unpack(self, data: &[u8], max_out: usize) -> Option<Vec<u8>> {
        match self {
            Codec::None => (data.len() <= max_out).then(|| Vec::from(data)),
            Codec::Lz4 => lz4::decompress(data, max_out),
            Codec::Zstd => zstd::decompress(data, max_out),
        }
    }
}
