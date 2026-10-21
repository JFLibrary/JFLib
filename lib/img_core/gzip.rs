// gzip -- gzip (RFC 1952) members on top of inflate_raw / deflate_raw  [src/img/gzip.s]
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
//
// Decoding: ID1 ID2, CM 8, FLG (reserved bits 5..7 must be 0), MTIME, XFL, OS, then FEXTRA,
// FNAME, FCOMMENT, FHCRC (low 16 bits of the CRC-32 of the header bytes), the deflate data,
// CRC32 and ISIZE.  Errors: ERR_TRUNC, ERR_HDR, ERR_CHECKSUM and the inflate errors.

use alloc::vec::Vec;

use crate::checksum::crc32;
use crate::deflate::{DeflateState, deflate_raw_ex, deflate_raw_to};
use crate::inflate::{InflateState, inflate_raw_st};
use crate::{ERR_CHECKSUM, ERR_HDR, ERR_OVERFLOW, ERR_TRUNC};

const GZ_FHCRC: u8 = 2;
const GZ_FEXTRA: u8 = 4;
const GZ_FNAME: u8 = 8;
const GZ_FCOMMENT: u8 = 16;

/// decompress one gzip member into dst  [gzip_decode]
pub fn gzip_decode(src: &[u8], dst: &mut [u8]) -> Result<(usize, usize), i32> {
    gzip_decode_st(None, src, dst)
}

// gz_skipz: the position after the NUL that ends the field at pos
fn skipz(src: &[u8], pos: usize) -> Option<usize> {
    src.get(pos..)?.iter().position(|&b| b == 0).map(|k| pos + k + 1)
}

/// gzip_decode with a reusable inflate state (None = a temporary one)  [gzip_decode_st]
pub fn gzip_decode_st(st: Option<&mut InflateState>, src: &[u8], dst: &mut [u8]) -> Result<(usize, usize), i32> {
    let len = src.len();
    if len < 10 {
        return Err(ERR_TRUNC);
    }
    if src[0] != 0x1F || src[1] != 0x8B || src[2] != 8 {
        return Err(ERR_HDR);
    }
    let flg = src[3];
    if flg & 0xE0 != 0 {
        return Err(ERR_HDR);
    }
    let mut p = 10usize;
    if flg & GZ_FEXTRA != 0 {
        if len - p < 2 {
            return Err(ERR_TRUNC);
        }
        let x = u16::from_le_bytes([src[p], src[p + 1]]) as usize;
        p += 2;
        if len - p < x {
            return Err(ERR_TRUNC);
        }
        p += x;
    }
    if flg & GZ_FNAME != 0 {
        p = skipz(src, p).ok_or(ERR_TRUNC)?;
    }
    if flg & GZ_FCOMMENT != 0 {
        p = skipz(src, p).ok_or(ERR_TRUNC)?;
    }
    if flg & GZ_FHCRC != 0 {
        if len - p < 2 {
            return Err(ERR_TRUNC);
        }
        let c = crc32(0, &src[..p]) as u16;
        if c != u16::from_le_bytes([src[p], src[p + 1]]) {
            return Err(ERR_CHECKSUM);
        }
        p += 2;
    }
    let (n, used) = inflate_raw_st(st, &src[p..], dst)?;
    p += used;
    if len - p < 8 {
        return Err(ERR_TRUNC);
    }
    let crc = u32::from_le_bytes([src[p], src[p + 1], src[p + 2], src[p + 3]]);
    let isize = u32::from_le_bytes([src[p + 4], src[p + 5], src[p + 6], src[p + 7]]);
    if crc32(0, &dst[..n]) != crc || n as u32 != isize {
        return Err(ERR_CHECKSUM);
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

/// one gzip member  [gzip_encode]
pub fn gzip_encode(src: &[u8], level: i32) -> Vec<u8> {
    gzip_encode_st(None, src, level)
}

/// gzip_encode with a reusable deflate state (None = a temporary one)  [gzip_encode_st]
pub fn gzip_encode_st(st: Option<&mut DeflateState>, src: &[u8], level: i32) -> Vec<u8> {
    let lv = norm_level(level);
    let mut buf = deflate_raw_ex(st, src, lv, 10, 8);
    header(&mut buf[..10], lv);
    buf.extend_from_slice(&crc32(0, src).to_le_bytes());
    buf.extend_from_slice(&(src.len() as u32).to_le_bytes());
    buf
}

/// the member of gzip_encode into dst  [gzip_encode_to]
pub fn gzip_encode_to(st: Option<&mut DeflateState>, src: &[u8], level: i32, dst: &mut [u8]) -> Result<usize, i32> {
    let lv = norm_level(level);
    if dst.len() < 18 {
        return Err(ERR_OVERFLOW);
    }
    // deflate's 8 bytes of slack hold the trailer
    let n = deflate_raw_to(st, src, lv, &mut dst[10..])?;
    header(&mut dst[..10], lv);
    dst[10 + n..14 + n].copy_from_slice(&crc32(0, src).to_le_bytes());
    dst[14 + n..18 + n].copy_from_slice(&(src.len() as u32).to_le_bytes());
    Ok(n + 18)
}

/// the ISIZE field (uncompressed size mod 2^32) of a single-member stream  [gzip_isize]
pub fn gzip_isize(src: &[u8]) -> Result<u32, i32> {
    let n = src.len();
    if n < 18 {
        return Err(ERR_TRUNC);
    }
    Ok(u32::from_le_bytes([src[n - 4], src[n - 3], src[n - 2], src[n - 1]]))
}
