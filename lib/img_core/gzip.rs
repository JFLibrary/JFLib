// gzip -- gzip (RFC 1952) members with img_core's error codes
//
// gzip lives in jflib-compress since 0.3 (compress::gzip); these are the calls of before, with
// the same bytes and codes.  New code calls compress directly.
//
//   gzip_decode(src, dst) -> Ok((bytes, consumed)) / Err(ERR_*)   ONE member; consumed =
//                                    the member length incl. the trailer
//   gzip_decode_st(st, src, dst)     the same with an inflate state (None = temporary)
//   gzip_encode(src, level) -> Vec   one member: 1F 8B 08, FLG 0, MTIME 0, XFL (2 for level 9,
//                                    4 for level 1, else 0), OS 255, deflate, CRC32, ISIZE
//   gzip_encode_st(st, src, level) -> Vec
//   gzip_encode_to(st, src, level, dst) -> Ok(bytes) / Err(ERR_OVERFLOW)   into dst
//                                    (deflate_bound(len) always suffices)
//   gzip_isize(src) -> Ok(ISIZE) / Err(ERR_TRUNC)   the last 4 bytes of a single member

use alloc::vec::Vec;

use compress::gzip;

use crate::deflate::DeflateState;
use crate::inflate::{code, InflateState};
use crate::ERR_TRUNC;

/// decompress one gzip member into dst  [gzip_decode]
pub fn gzip_decode(src: &[u8], dst: &mut [u8]) -> Result<(usize, usize), i32> {
    gzip_decode_st(None, src, dst)
}

/// gzip_decode with a reusable inflate state (None = a temporary one)  [gzip_decode_st]
pub fn gzip_decode_st(st: Option<&mut InflateState>, src: &[u8], dst: &mut [u8]) -> Result<(usize, usize), i32> {
    gzip::decompress_member_into(st, src, dst).map_err(code)
}

/// one gzip member  [gzip_encode]
pub fn gzip_encode(src: &[u8], level: i32) -> Vec<u8> {
    gzip::compress(src, level)
}

/// gzip_encode with a reusable deflate state (None = a temporary one)  [gzip_encode_st]
pub fn gzip_encode_st(st: Option<&mut DeflateState>, src: &[u8], level: i32) -> Vec<u8> {
    gzip::compress_with(st, src, level)
}

/// the member of gzip_encode into dst  [gzip_encode_to]
pub fn gzip_encode_to(st: Option<&mut DeflateState>, src: &[u8], level: i32, dst: &mut [u8]) -> Result<usize, i32> {
    gzip::compress_into(st, src, level, dst).map_err(code)
}

/// the ISIZE field (uncompressed size mod 2^32) of a single-member stream  [gzip_isize]
pub fn gzip_isize(src: &[u8]) -> Result<u32, i32> {
    gzip::stored_size(src).ok_or(ERR_TRUNC)
}
