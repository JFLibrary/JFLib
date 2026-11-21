// deflate -- DEFLATE (RFC 1951) / zlib (RFC 1950) compression with img_core's error codes
//
// The encoder lives in jflib-compress since 0.3 (compress::deflate, compress::zlib); these are
// the calls of before, with the same bytes.  New code calls compress directly.
//
//   zlib_deflate(src, level) -> Vec          zlib stream (level 0..9, < 0 = 6, > 9 = 9)
//   deflate_raw(src, level) -> Vec           raw RFC 1951 data
//   deflate_raw_ex(st, src, level, head, tail) -> Vec   head free bytes before the data
//                                            (gzip writes its header there), tail reserved
//                                            after it (not part of the Vec)
//   DeflateState::new() -> Box               a reusable compressor state (~330 KB; one
//                                            thread at a time); the same bytes as fresh calls
//   zlib_deflate_st(st, src, level) / deflate_raw_st(st, src, level) -> Vec
//   zlib_deflate_to(st, src, level, dst) / deflate_raw_to(...) -> Ok(bytes) /
//                                            Err(ERR_OVERFLOW): into dst (no allocation with
//                                            a state); overflow when the stream + 8 bytes do
//                                            not fit (dst contents undefined then)
//   deflate_bound(len) -> len + len / 512 + 128   a cap with which *_to never overflow

use alloc::vec::Vec;

use compress::{deflate, zlib};

pub use compress::deflate::DeflateState;

use crate::inflate::code;

/// output capacity with which the *_to functions never report ERR_OVERFLOW  [deflate_bound]
pub fn deflate_bound(len: usize) -> usize {
    deflate::bound(len)
}

/// zlib stream into dst  [zlib_deflate_to]
pub fn zlib_deflate_to(st: Option<&mut DeflateState>, src: &[u8], level: i32, dst: &mut [u8]) -> Result<usize, i32> {
    zlib::compress_into(st, src, level, dst).map_err(code)
}

/// raw deflate data into dst  [deflate_raw_to]
pub fn deflate_raw_to(st: Option<&mut DeflateState>, src: &[u8], level: i32, dst: &mut [u8]) -> Result<usize, i32> {
    deflate::compress_into(st, src, level, dst).map_err(code)
}

/// zlib stream  [zlib_deflate]
pub fn zlib_deflate(src: &[u8], level: i32) -> Vec<u8> {
    zlib::compress(src, level)
}

/// zlib stream with a reusable state (None = a temporary one)  [zlib_deflate_st]
pub fn zlib_deflate_st(st: Option<&mut DeflateState>, src: &[u8], level: i32) -> Vec<u8> {
    zlib::compress_with(st, src, level)
}

/// raw deflate data  [deflate_raw]
pub fn deflate_raw(src: &[u8], level: i32) -> Vec<u8> {
    deflate::compress(src, level)
}

/// raw deflate data with a reusable state  [deflate_raw_st without head / tail]
pub fn deflate_raw_st(st: Option<&mut DeflateState>, src: &[u8], level: i32) -> Vec<u8> {
    deflate::compress_with(st, src, level)
}

/// raw deflate data after `head` zero bytes (head, tail: 0..65536, larger values clamped);
/// the Vec holds head + the compressed bytes, with room for `tail` more  [deflate_raw_st]
pub fn deflate_raw_ex(st: Option<&mut DeflateState>, src: &[u8], level: i32, head: usize, tail: usize) -> Vec<u8> {
    deflate::compress_padded(st, src, level, head, tail)
}
