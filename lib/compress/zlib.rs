// zlib -- the zlib format (RFC 1950): a two-byte header, DEFLATE data and an Adler-32
//
//   compress(data, level) -> Vec<u8>         levels as deflate::compress; the header names the
//                                            level as zlib does (78 01 / 5E / 9C / DA)
//   decompress(data, max_out) -> Option<Vec<u8>>   the whole input must be one zlib stream;
//                                            None on corruption, a wrong Adler-32, data after
//                                            the stream or more than max_out bytes
//   compress_with(state, data, level) / compress_into(state, data, level, dst)
//   decompress_into(state, data, dst) -> Ok((bytes, consumed))   consumed = header, DEFLATE
//                                            data and Adler-32; data after it is not looked at
//
// A preset dictionary (FDICT) is refused as Error::Header.

use alloc::vec::Vec;

use crate::deflate::{dec, enc, grow, DeflateState, Error, InflateState};

/// a zlib stream of `data` at `level`
pub fn compress(data: &[u8], level: i32) -> Vec<u8> {
    enc::zlib_deflate(data, level)
}

/// `compress` with a reusable state (None = a temporary one)
pub fn compress_with(state: Option<&mut DeflateState>, data: &[u8], level: i32) -> Vec<u8> {
    enc::zlib_deflate_st(state, data, level)
}

/// `compress` into `dst`, the number of bytes written; `Error::Overflow` when it does not fit
/// with 8 bytes to spare (`deflate::bound` always suffices)
pub fn compress_into(state: Option<&mut DeflateState>, data: &[u8], level: i32, dst: &mut [u8]) -> Result<usize, Error> {
    enc::zlib_deflate_to(state, data, level, dst)
}

/// the data of the zlib stream that fills `data` completely, at most `max_out` bytes
pub fn decompress(data: &[u8], max_out: usize) -> Option<Vec<u8>> {
    let mut st = InflateState::new();
    grow(data.len(), max_out, |dst| match dec::zlib_inflate_ex(Some(&mut st), data, dst)? {
        (n, used) if used == data.len() => Ok(n),
        _ => Err(Error::Block),
    })
}

/// the zlib stream at the start of `data` into `dst`: (bytes written, input bytes of the
/// stream including its Adler-32)
pub fn decompress_into(state: Option<&mut InflateState>, data: &[u8], dst: &mut [u8]) -> Result<(usize, usize), Error> {
    dec::zlib_inflate_ex(state, data, dst)
}
