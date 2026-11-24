// brotli -- Brotli decompression (RFC 7932) of the JPEG reconstruction data (jbrd box) of
// JPEG transcodes, with img_core's error codes
//
// The decoder lives in jflib-compress since 0.3 (compress::brotli); this is the call of before.
//
//   decompress(data, max) -> (bytes, consumed)   the stream at the start of data; ERR_CODE for
//                                                a damaged stream, ERR_LIMIT past max output
//                                                bytes

use alloc::vec::Vec;

use compress::brotli::{self, Error};
use img_core::{ERR_CODE, ERR_LIMIT};

/// decompress the Brotli stream at the start of data -> (output, bytes of input used)
pub fn decompress(data: &[u8], max: usize) -> Result<(Vec<u8>, usize), i32> {
    brotli::decompress_prefix(data, max).map_err(|e| match e {
        Error::Limit => ERR_LIMIT,
        _ => ERR_CODE,
    })
}
