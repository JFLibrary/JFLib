// inflate -- DEFLATE (RFC 1951) and zlib (RFC 1950) decompression with img_core's error codes
//
// The decoder lives in jflib-compress since 0.3 (compress::deflate, compress::zlib); these are
// the calls of before, for the codecs and for code written against them.  New code calls
// compress directly.
//
//   zlib_inflate(src, dst) -> Ok(bytes written) / Err(ERR_*)
//   inflate_raw(src, dst) -> Ok(bytes) / Err(ERR_*)
//   inflate_raw_ex(src, dst) -> Ok((bytes, consumed)) / Err(ERR_*)   consumed = input bytes
//                                     of the deflate data (up to the end of the final block)
//   InflateState::new() -> Box      reusable decoder state (one thread at a time; an error
//                                     never affects later calls)
//   zlib_inflate_st(st, src, dst) / inflate_raw_st(st, src, dst)   the same with a state
//                                     (None = a temporary one, like the classic calls)
//
// Errors: ERR_OVERFLOW (the output does not fit), ERR_DIST, ERR_TRUNC, ERR_HDR (zlib header),
// ERR_CHECKSUM (Adler-32), ERR_BLOCK, ERR_CODE -- the same codes for the same input as before.

use compress::deflate::{self, Error};
use compress::zlib;

pub use compress::deflate::InflateState;

use crate::{ERR_BLOCK, ERR_CHECKSUM, ERR_CODE, ERR_DIST, ERR_FAIL, ERR_HDR, ERR_OVERFLOW, ERR_TRUNC};

/// img_core's code of a DEFLATE, zlib or gzip error
pub(crate) fn code(e: Error) -> i32 {
    match e {
        Error::Truncated => ERR_TRUNC,
        Error::Header => ERR_HDR,
        Error::Block => ERR_BLOCK,
        Error::Code => ERR_CODE,
        Error::Distance => ERR_DIST,
        Error::Overflow => ERR_OVERFLOW,
        Error::Checksum => ERR_CHECKSUM,
        _ => ERR_FAIL,
    }
}

/// zlib stream src -> dst  [zlib_inflate]
pub fn zlib_inflate(src: &[u8], dst: &mut [u8]) -> Result<usize, i32> {
    zlib_inflate_st(None, src, dst)
}

/// zlib_inflate with a reusable state (None = a temporary one)  [zlib_inflate_st]
pub fn zlib_inflate_st(st: Option<&mut InflateState>, src: &[u8], dst: &mut [u8]) -> Result<usize, i32> {
    zlib::decompress_into(st, src, dst).map(|(n, _)| n).map_err(code)
}

/// raw deflate data src -> dst  [inflate_raw]
pub fn inflate_raw(src: &[u8], dst: &mut [u8]) -> Result<usize, i32> {
    inflate_raw_st(None, src, dst).map(|(n, _)| n)
}

/// inflate_raw that also gives the input bytes the deflate data occupied  [inflate_raw_ex]
pub fn inflate_raw_ex(src: &[u8], dst: &mut [u8]) -> Result<(usize, usize), i32> {
    inflate_raw_st(None, src, dst)
}

/// inflate_raw_ex with a reusable state (None = a temporary one)  [inflate_raw_st]
pub fn inflate_raw_st(st: Option<&mut InflateState>, src: &[u8], dst: &mut [u8]) -> Result<(usize, usize), i32> {
    deflate::decompress_into(st, src, dst).map_err(code)
}
