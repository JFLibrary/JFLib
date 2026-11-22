// gzip -- the gzip format (RFC 1952): members of a header, DEFLATE data, CRC-32 and size
// (until 0.2 img_core::gzip)                                               [src/img/gzip.s]
//
//   compress(data, level) -> Vec<u8>   one member: 1F 8B 08, FLG 0, MTIME 0, XFL (2 for level
//                                      9, 4 for level 1, else 0), OS 255, deflate, CRC32, ISIZE
//   decompress(data, max_out) -> Option<Vec<u8>>   every member of data, one after the other
//                                      as gunzip reads them; None on corruption, a wrong
//                                      CRC-32 or size, bytes that are no member, or more than
//                                      max_out bytes
//   compress_with(state, data, level) / compress_into(state, data, level, dst) -> Ok(bytes) /
//                                      Err(Error::Overflow)   into dst (deflate::bound(len)
//                                      always suffices)
//   decompress_member_into(state, data, dst) -> Ok((bytes, consumed))   ONE member; consumed
//                                      = the member's length including the trailer
//   stored_size(data) -> Option<u32>   ISIZE, the last 4 bytes of a single member (the size
//                                      mod 2^32, as the member claims it)
//   Decoder / Encoder                  in pieces (stream::Decode / Encode); Decoder reads every
//                                      member, Decoder::single() the first
//
// Decoding: ID1 ID2, CM 8, FLG (reserved bits 5..7 must be 0), MTIME, XFL, OS, then FEXTRA,
// FNAME, FCOMMENT, FHCRC (low 16 bits of the CRC-32 of the header bytes), the deflate data,
// CRC32 and ISIZE.  Errors: Error::Truncated, Error::Header, Error::Checksum and the inflate
// errors.

use alloc::vec::Vec;

use crate::checksum::crc32;
use crate::deflate::enc::{DeflateState, deflate_raw_ex, deflate_raw_to};
use crate::deflate::dec::{InflateState, inflate_raw_st};
use crate::deflate::{grow, Error};
use crate::deflate::senc;
use crate::stream::{gather, Decode, Encode, Error as SError, Flush, Status, Step};

const GZ_FHCRC: u8 = 2;
const GZ_FEXTRA: u8 = 4;
const GZ_FNAME: u8 = 8;
const GZ_FCOMMENT: u8 = 16;

/// the data of all members that fill `data` completely, at most `max_out` bytes
pub fn decompress(data: &[u8], max_out: usize) -> Option<Vec<u8>> {
    if data.is_empty() {
        return None;
    }
    let mut st = InflateState::new();
    grow(data.len(), max_out, |dst| {
        let (mut p, mut o) = (0, 0);
        while p < data.len() {
            let (n, used) = decompress_member_into(Some(&mut st), &data[p..], &mut dst[o..])?;
            p += used;
            o += n;
        }
        Ok(o)
    })
}

// gz_skipz: the position after the NUL that ends the field at pos
fn skipz(src: &[u8], pos: usize) -> Option<usize> {
    src.get(pos..)?.iter().position(|&b| b == 0).map(|k| pos + k + 1)
}

/// the first member of `data` into `dst` with a reusable state (None = a temporary one):
/// (bytes written, input bytes of the member)  [gzip_decode_st]
pub fn decompress_member_into(st: Option<&mut InflateState>, src: &[u8], dst: &mut [u8]) -> Result<(usize, usize), Error> {
    let len = src.len();
    if len < 10 {
        return Err(Error::Truncated);
    }
    if src[0] != 0x1F || src[1] != 0x8B || src[2] != 8 {
        return Err(Error::Header);
    }
    let flg = src[3];
    if flg & 0xE0 != 0 {
        return Err(Error::Header);
    }
    let mut p = 10usize;
    if flg & GZ_FEXTRA != 0 {
        if len - p < 2 {
            return Err(Error::Truncated);
        }
        let x = u16::from_le_bytes([src[p], src[p + 1]]) as usize;
        p += 2;
        if len - p < x {
            return Err(Error::Truncated);
        }
        p += x;
    }
    if flg & GZ_FNAME != 0 {
        p = skipz(src, p).ok_or(Error::Truncated)?;
    }
    if flg & GZ_FCOMMENT != 0 {
        p = skipz(src, p).ok_or(Error::Truncated)?;
    }
    if flg & GZ_FHCRC != 0 {
        if len - p < 2 {
            return Err(Error::Truncated);
        }
        let c = crc32(0, &src[..p]) as u16;
        if c != u16::from_le_bytes([src[p], src[p + 1]]) {
            return Err(Error::Checksum);
        }
        p += 2;
    }
    let (n, used) = inflate_raw_st(st, &src[p..], dst)?;
    p += used;
    if len - p < 8 {
        return Err(Error::Truncated);
    }
    let crc = u32::from_le_bytes([src[p], src[p + 1], src[p + 2], src[p + 3]]);
    let isize = u32::from_le_bytes([src[p + 4], src[p + 5], src[p + 6], src[p + 7]]);
    if crc32(0, &dst[..n]) != crc || n as u32 != isize {
        return Err(Error::Checksum);
    }
    Ok((n, p + 8))
}

// gz_header: the 10 header bytes of our members
fn header(h: &mut [u8], level: i32) {
    h[..4].copy_from_slice(&[0x1F, 0x8B, 8, 0]);
    h[4..8].fill(0);
    h[8] = match level {
        9 => 2,
        1 => 4,
        _ => 0,
    };
    h[9] = 255;
}

fn norm_level(level: i32) -> i32 {
    if level < 0 { 6 } else { level.min(9) }
}

/// one gzip member of `data` at `level`  [gzip_encode]
pub fn compress(src: &[u8], level: i32) -> Vec<u8> {
    compress_with(None, src, level)
}

/// `compress` with a reusable state (None = a temporary one)  [gzip_encode_st]
pub fn compress_with(st: Option<&mut DeflateState>, src: &[u8], level: i32) -> Vec<u8> {
    let lv = norm_level(level);
    let mut buf = deflate_raw_ex(st, src, lv, 10, 8);
    header(&mut buf[..10], lv);
    buf.extend_from_slice(&crc32(0, src).to_le_bytes());
    buf.extend_from_slice(&(src.len() as u32).to_le_bytes());
    buf
}

/// the member of `compress` into `dst`, the number of bytes written  [gzip_encode_to]
pub fn compress_into(st: Option<&mut DeflateState>, src: &[u8], level: i32, dst: &mut [u8]) -> Result<usize, Error> {
    let lv = norm_level(level);
    if dst.len() < 18 {
        return Err(Error::Overflow);
    }
    // deflate's 8 bytes of slack hold the trailer
    let n = deflate_raw_to(st, src, lv, &mut dst[10..])?;
    header(&mut dst[..10], lv);
    dst[10 + n..14 + n].copy_from_slice(&crc32(0, src).to_le_bytes());
    dst[14 + n..18 + n].copy_from_slice(&(src.len() as u32).to_le_bytes());
    Ok(n + 18)
}

/// the ISIZE field (uncompressed size mod 2^32) of a single-member stream; None for fewer than
/// 18 bytes  [gzip_isize]
pub fn stored_size(src: &[u8]) -> Option<u32> {
    let n = src.len();
    if n < 18 {
        return None;
    }
    Some(u32::from_le_bytes([src[n - 4], src[n - 3], src[n - 2], src[n - 1]]))
}

/// a gzip decoder that takes its input in pieces (stream::Decode): every member, one after
/// the other as gunzip reads them (`single()`: only the first, and the input after it is not
/// taken); the header with its optional fields, the DEFLATE data through deflate::Decoder,
/// the CRC-32 and the size of each member
pub struct Decoder {
    raw: crate::deflate::Decoder,
    buf: Vec<u8>,
    phase: u8,
    flg: u8,
    skip: usize,  // FEXTRA bytes left
    hcrc: u32,    // CRC-32 of the header bytes so far (FHCRC)
    crc: u32,     // of the member's output
    size: u32,    // its length mod 2^32
    members: u64, // members ended
    single: bool,
}

// phases of Decoder
const P_HEAD: u8 = 0;
const P_XLEN: u8 = 1;
const P_EXTRA: u8 = 2;
const P_NAME: u8 = 3;
const P_COMMENT: u8 = 4;
const P_HCRC: u8 = 5;
const P_DATA: u8 = 6;
const P_TRAIL: u8 = 7;
const P_END: u8 = 8;

impl Decoder {
    /// a decoder of all members
    pub fn new() -> Decoder {
        Decoder {
            raw: crate::deflate::Decoder::new(),
            buf: Vec::new(),
            phase: P_HEAD,
            flg: 0,
            skip: 0,
            hcrc: 0,
            crc: 0,
            size: 0,
            members: 0,
            single: false,
        }
    }

    /// a decoder of one member: Status::End after it, input behind it is not taken
    pub fn single() -> Decoder {
        Decoder { single: true, ..Decoder::new() }
    }

    /// start anew (the memory stays)
    pub fn reset(&mut self) {
        self.raw.reset();
        self.buf.clear();
        self.phase = P_HEAD;
        self.members = 0;
    }

    /// the members ended so far
    pub fn members(&self) -> u64 {
        self.members
    }

    // the phase after the header field `from`
    fn next_field(&self, from: u8) -> u8 {
        let f = self.flg;
        if from < P_XLEN && f & GZ_FEXTRA != 0 {
            P_XLEN
        } else if from < P_NAME && f & GZ_FNAME != 0 {
            P_NAME
        } else if from < P_COMMENT && f & GZ_FCOMMENT != 0 {
            P_COMMENT
        } else if from < P_HCRC && f & GZ_FHCRC != 0 {
            P_HCRC
        } else {
            P_DATA
        }
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
                P_HEAD => {
                    let have = gather(&mut self.buf, 10, input, &mut read);
                    // the identification bytes as soon as they are there: bytes after a member
                    // that start no other one are an error at once
                    if self.buf.iter().zip([0x1F, 0x8B, 8]).any(|(&b, w)| b != w) {
                        return Err(SError::Corrupt);
                    }
                    if !have {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    let h = &self.buf;
                    if h[0] != 0x1F || h[1] != 0x8B || h[2] != 8 || h[3] & 0xE0 != 0 {
                        return Err(SError::Corrupt);
                    }
                    self.flg = h[3];
                    self.hcrc = crc32(0, h);
                    self.crc = 0;
                    self.size = 0;
                    self.buf.clear();
                    self.phase = self.next_field(P_HEAD);
                }
                P_XLEN => {
                    if !gather(&mut self.buf, 2, input, &mut read) {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    self.hcrc = crc32(self.hcrc, &self.buf);
                    self.skip = u16::from_le_bytes([self.buf[0], self.buf[1]]) as usize;
                    self.buf.clear();
                    self.phase = P_EXTRA;
                }
                P_EXTRA => {
                    let k = self.skip.min(input.len() - read);
                    self.hcrc = crc32(self.hcrc, &input[read..read + k]);
                    read += k;
                    self.skip -= k;
                    if self.skip > 0 {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    self.phase = self.next_field(P_EXTRA);
                }
                P_NAME | P_COMMENT => {
                    let rest = &input[read..];
                    match rest.iter().position(|&b| b == 0) {
                        Some(k) => {
                            self.hcrc = crc32(self.hcrc, &rest[..k + 1]);
                            read += k + 1;
                            self.phase = self.next_field(self.phase);
                        }
                        None => {
                            self.hcrc = crc32(self.hcrc, rest);
                            return Ok(Step { read: input.len(), written, status: Status::Input });
                        }
                    }
                }
                P_HCRC => {
                    if !gather(&mut self.buf, 2, input, &mut read) {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    if self.hcrc as u16 != u16::from_le_bytes([self.buf[0], self.buf[1]]) {
                        return Err(SError::Checksum);
                    }
                    self.buf.clear();
                    self.phase = P_DATA;
                }
                P_DATA => {
                    let s = self.raw.decode(&input[read..], &mut out[written..])?;
                    let o = &out[written..written + s.written];
                    self.crc = crc32(self.crc, o);
                    self.size = self.size.wrapping_add(o.len() as u32);
                    read += s.read;
                    written += s.written;
                    if s.status != Status::End {
                        return Ok(Step { read, written, status: s.status });
                    }
                    self.phase = P_TRAIL;
                }
                P_TRAIL => {
                    if !gather(&mut self.buf, 8, input, &mut read) {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    let b = &self.buf;
                    let crc = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                    let size = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                    if crc != self.crc || size != self.size {
                        return Err(SError::Checksum);
                    }
                    self.buf.clear();
                    self.members += 1;
                    self.phase = P_END;
                }
                _ => {
                    if self.single || read == input.len() {
                        return Ok(Step { read, written, status: Status::End });
                    }
                    // another member follows
                    self.raw.reset();
                    self.phase = P_HEAD;
                }
            }
        }
    }

    fn finish(&self) -> Result<(), SError> {
        if self.phase == P_END { Ok(()) } else { Err(SError::Truncated) }
    }
}

/// a gzip encoder that takes its input in pieces (stream::Encode): one member with the header of
/// compress(), DEFLATE data as deflate::Encoder writes it, the CRC-32 and size; the same bytes as
/// compress() for a stream of at most 256 KiB
pub struct Encoder {
    core: senc::Core,
}

impl Encoder {
    /// an encoder at `level` (as compress: 0..=9, < 0 = 6)
    pub fn new(level: i32) -> Encoder {
        Encoder { core: senc::Core::new(level, senc::Wrap::Gzip) }
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
