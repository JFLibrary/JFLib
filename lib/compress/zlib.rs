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
//   Decoder / Encoder                        in pieces (stream::Decode / Encode)
//
// A preset dictionary (FDICT) is refused as Error::Header.

use alloc::vec::Vec;

use crate::checksum::adler32;
use crate::deflate::{dec, enc, grow, DeflateState, Error, InflateState};
use crate::deflate::senc;
use crate::stream::{gather, Decode, Encode, Error as SError, Flush, Status, Step};

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

/// a zlib decoder that takes its input in pieces (stream::Decode): the header, the DEFLATE
/// data through deflate::Decoder, the Adler-32 of the output
pub struct Decoder {
    raw: crate::deflate::Decoder,
    buf: Vec<u8>,
    phase: u8, // 0 header, 1 data, 2 Adler-32, 3 done
    adler: u32,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder { raw: crate::deflate::Decoder::new(), buf: Vec::new(), phase: 0, adler: 1 }
    }

    /// start a new stream (the memory stays)
    pub fn reset(&mut self) {
        self.raw.reset();
        self.buf.clear();
        self.phase = 0;
        self.adler = 1;
    }
}

impl Default for Decoder {
    fn default() -> Decoder {
        Decoder::new()
    }
}

impl Decode for Decoder {
    fn decode(&mut self, input: &[u8], out: &mut [u8]) -> Result<Step, SError> {
        let (mut read, mut written) = (0, 0);
        loop {
            match self.phase {
                0 => {
                    if !gather(&mut self.buf, 2, input, &mut read) {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    let (b0, b1) = (self.buf[0] as u32, self.buf[1] as u32);
                    if b0 & 15 != 8 || b0 >> 4 > 7 || (b0 << 8 | b1) % 31 != 0 {
                        return Err(SError::Corrupt);
                    }
                    if b1 & 0x20 != 0 {
                        return Err(SError::Unsupported); // a preset dictionary
                    }
                    self.buf.clear();
                    self.phase = 1;
                }
                1 => {
                    let s = self.raw.decode(&input[read..], &mut out[written..])?;
                    self.adler = adler32(self.adler, &out[written..written + s.written]);
                    read += s.read;
                    written += s.written;
                    if s.status != Status::End {
                        return Ok(Step { read, written, status: s.status });
                    }
                    self.phase = 2;
                }
                2 => {
                    if !gather(&mut self.buf, 4, input, &mut read) {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    if u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) != self.adler {
                        return Err(SError::Checksum);
                    }
                    self.phase = 3;
                }
                _ => return Ok(Step { read, written, status: Status::End }),
            }
        }
    }

    fn finish(&self) -> Result<(), SError> {
        if self.phase == 3 { Ok(()) } else { Err(SError::Truncated) }
    }
}

/// a zlib encoder that takes its input in pieces (stream::Encode): the header, DEFLATE data as
/// deflate::Encoder writes it, the Adler-32; the same bytes as compress() for a stream of at
/// most 256 KiB
pub struct Encoder {
    core: senc::Core,
}

impl Encoder {
    /// an encoder at `level` (as compress: 0..=9, < 0 = 6)
    pub fn new(level: i32) -> Encoder {
        Encoder { core: senc::Core::new(level, senc::Wrap::Zlib) }
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
