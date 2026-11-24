// deflate -- raw DEFLATE data (RFC 1951), the compressed format inside zlib, gzip, ZIP and PNG
//
//   compress(data, level) -> Vec<u8>         levels 0..=9 (LEVEL_DEFAULT 6; < 0 = 6, > 9 = 9)
//   decompress(data, max_out) -> Option<Vec<u8>>   the whole input must be one DEFLATE stream;
//                                            None on corruption, data after the stream or
//                                            more than max_out bytes
//   compress_with(state, data, level) / compress_into(state, data, level, dst) /
//   compress_padded(state, data, level, head, tail)   with a reusable state (None = a
//                                            temporary one), into the caller's buffer, or with
//                                            free bytes before and room after the data
//   decompress_into(state, data, dst) -> Ok((bytes, consumed))   into the caller's buffer;
//                                            consumed = the input bytes of the stream, so data
//                                            after it is the caller's to judge
//   bound(len)                               an output size with which compress_into never
//                                            fails
//   InflateState / DeflateState              reusable decoder and encoder (~330 KB) states,
//                                            one thread at a time; a reused state gives the
//                                            same bytes and errors as a fresh one
//   Decoder / Encoder                        the same in pieces (stream::Decode / Encode):
//                                            sdec.rs, senc.rs (zlib and gzip wrap them)
//
// The decoder checks every write against the output buffer and every back reference against
// the bytes produced so far, so a corrupt or hostile stream ends with an Error, never with a
// read or write outside the buffers.  The encoder writes the same bytes on every platform
// (zlib's levels: greedy 1..3, lazy 4..9, blocks as the cheapest of stored, fixed and
// dynamic).  dec.rs and enc.rs are the engines, zlib.rs and gzip.rs the two wrappers.

use alloc::vec;
use alloc::vec::Vec;

use crate::stream::{Encode, Error as SError, Flush, Step};

pub(crate) mod dec;
pub(crate) mod enc;
mod sdec;
pub(crate) mod senc;

pub use dec::InflateState;
pub use enc::DeflateState;
pub use sdec::Decoder;

/// the level `compress` uses for a negative level, zlib's default
pub const LEVEL_DEFAULT: i32 = 6;
/// the strongest level; higher levels count as this one
pub const LEVEL_MAX: i32 = 9;

/// why a DEFLATE, zlib or gzip stream could not be read or written
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// the input ends before the stream does
    Truncated,
    /// a zlib or gzip header that is not valid
    Header,
    /// a block of an unknown type, or a stored block with a wrong length
    Block,
    /// a Huffman code that is not valid, or a symbol no code allows
    Code,
    /// a back reference to before the start of the output
    Distance,
    /// the output does not fit into the buffer
    Overflow,
    /// a checksum (Adler-32, CRC-32) or a stored size that does not match the data
    Checksum,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Error::Truncated => "the stream is truncated",
            Error::Header => "the header is not valid",
            Error::Block => "a block is not valid",
            Error::Code => "a Huffman code is not valid",
            Error::Distance => "a back reference points before the start",
            Error::Overflow => "the output does not fit",
            Error::Checksum => "the checksum does not match",
        })
    }
}

/// an output capacity with which `compress_into` (and the zlib and gzip ones) never fail
pub fn bound(len: usize) -> usize {
    enc::deflate_bound(len)
}

/// raw DEFLATE data of `data` at `level`
pub fn compress(data: &[u8], level: i32) -> Vec<u8> {
    enc::deflate_raw(data, level)
}

/// `compress` with a reusable state (None = a temporary one)
pub fn compress_with(state: Option<&mut DeflateState>, data: &[u8], level: i32) -> Vec<u8> {
    enc::deflate_raw_st(state, data, level)
}

/// `compress` into `dst`, the number of bytes written; `Error::Overflow` when the stream and 8
/// more bytes do not fit (the contents of `dst` are undefined then)
pub fn compress_into(state: Option<&mut DeflateState>, data: &[u8], level: i32, dst: &mut [u8]) -> Result<usize, Error> {
    enc::deflate_raw_to(state, data, level, dst)
}

/// `compress` after `head` zero bytes, with room for `tail` more bytes behind the data (both
/// at most 64 KiB): a container writes its header and trailer around the stream without
/// copying it.  The vector holds the head and the stream.
pub fn compress_padded(state: Option<&mut DeflateState>, data: &[u8], level: i32, head: usize, tail: usize) -> Vec<u8> {
    enc::deflate_raw_ex(state, data, level, head, tail)
}

/// the data of one DEFLATE stream that fills `data` completely, at most `max_out` bytes
pub fn decompress(data: &[u8], max_out: usize) -> Option<Vec<u8>> {
    let mut st = InflateState::new();
    grow(data.len(), max_out, |dst| match dec::inflate_raw_st(Some(&mut st), data, dst)? {
        (n, used) if used == data.len() => Ok(n),
        _ => Err(Error::Block),
    })
}

/// the stream at the start of `data` into `dst`: (bytes written, input bytes of the stream)
pub fn decompress_into(state: Option<&mut InflateState>, data: &[u8], dst: &mut [u8]) -> Result<(usize, usize), Error> {
    dec::inflate_raw_st(state, data, dst)
}

// an output buffer that grows until the data fit or max_out is reached: run decodes into a
// buffer of the given size and reports Error::Overflow when it is too small.  The first size
// is four times the input (at least 64 KiB), every further one twice the last, so all
// attempts together decode at most about twice the final output.
pub(crate) fn grow(input: usize, max_out: usize, mut run: impl FnMut(&mut [u8]) -> Result<usize, Error>) -> Option<Vec<u8>> {
    let mut cap = input.saturating_mul(4).max(64 << 10).min(max_out);
    loop {
        let mut buf = vec![0u8; cap];
        match run(&mut buf) {
            Ok(n) => {
                buf.truncate(n);
                return Some(buf);
            }
            Err(Error::Overflow) if cap < max_out => cap = cap.saturating_mul(2).min(max_out),
            Err(_) => return None,
        }
    }
}

/// a raw DEFLATE encoder that takes its input in pieces (stream::Encode), about 900 KB of
/// state: pieces of 256 KiB with the 32 KiB before each as their window; the same bytes as
/// compress() for a stream of at most 256 KiB
pub struct Encoder {
    core: senc::Core,
}

impl Encoder {
    /// an encoder at `level` (as compress: 0..=9, < 0 = 6)
    pub fn new(level: i32) -> Encoder {
        Encoder { core: senc::Core::new(level, senc::Wrap::Raw) }
    }

    /// start a new stream at the same level (the memory stays)
    pub fn reset(&mut self) {
        self.core.reset();
    }
}

impl Encode for Encoder {
    fn encode(&mut self, input: &[u8], out: &mut [u8], flush: Flush) -> Result<Step, SError> {
        self.core.encode(input, out, flush)
    }
}
