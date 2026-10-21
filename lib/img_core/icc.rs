// icc -- ICC profile container helpers (no colour conversion: that is icc_xform)
//                                                                           [src/img/icc.s]
//
//   icc_check(data) -> profile size / ERR_TRUNC / ERR_HDR / ERR_UNSUPPORTED
//   icc_jpeg_collect(data) -> (1 found / 0 none / ERR_CHUNK, profile)
//   icc_jpeg_segments(icc) -> Ok(APP2 segments) / Err(ERR_LIMIT)
//
// icc_check validates the 128-byte header and the tag table: declared size >= 132 and
// <= len, 'acsp' at byte 36, major version 2..5, the tag count fits, every tag's data lies
// inside the declared size.  JPEG embeds a profile in APP2 segments "ICC_PROFILE\0" + seq
// (1-based) + count + data; icc_jpeg_collect walks the segments before the first SOS and
// reassembles them in sequence order, rejecting (ERR_CHUNK) like libjpeg's
// read_icc_profile: a count of 0, counts that differ, a sequence number of 0 or > count, a
// duplicate, a missing one and an empty result.  icc_jpeg_segments produces the complete
// APP2 segments (FF E2, length, header, <= 65519 data bytes each); more than 255 segments
// -> ERR_LIMIT.

use alloc::vec;
use alloc::vec::Vec;

use crate::meta::{JSeg, jpg_seg_next};
use crate::{ERR_CHUNK, ERR_HDR, ERR_LIMIT, ERR_TRUNC, ERR_UNSUPPORTED};

const ICC_SEG_MAX: usize = 65519; // 65535 - 2 (length) - 14 (header)
const ICC_HDR_LEN: usize = 14; // "ICC_PROFILE\0" + seq + count
const ICC_SIG: &[u8; 12] = b"ICC_PROFILE\0";

fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// the declared profile size or ERR_TRUNC (shorter than the header / the declared size),
/// ERR_HDR (bad signature, size, tag table), ERR_UNSUPPORTED (major version not 2..5)
/// [icc_check]
pub fn icc_check(data: &[u8]) -> i64 {
    let len = data.len() as u64;
    if len < 132 {
        return ERR_TRUNC as i64;
    }
    let size = be32(data, 0) as u64;
    if size < 132 {
        return ERR_HDR as i64;
    }
    if size > len {
        return ERR_TRUNC as i64;
    }
    if &data[36..40] != b"acsp" {
        return ERR_HDR as i64;
    }
    if (data[8] as u32).wrapping_sub(2) > 3 {
        return ERR_UNSUPPORTED as i64;
    }
    let n = be32(data, 128) as u64;
    if n > (size - 132) / 12 {
        return ERR_HDR as i64;
    }
    for t in 0..n as usize {
        let off = be32(data, 136 + t * 12) as u64;
        let sz = be32(data, 140 + t * 12) as u64;
        if off > size || size - off < sz {
            return ERR_HDR as i64;
        }
    }
    size as i64
}

/// reassemble the APP2 ICC_PROFILE segments of a JPEG file -> (1 profile / 0 no
/// ICC_PROFILE segment / ERR_CHUNK inconsistent segments, profile bytes)  [icc_jpeg_collect]
pub fn icc_jpeg_collect(data: &[u8]) -> (i32, Vec<u8>) {
    if data.len() < 2 || data[0] != 0xFF || data[1] != 0xD8 {
        return (0, Vec::new());
    }
    let mut parts: Vec<Option<(usize, usize)>> = vec![None; 256]; // per sequence number
    let mut count = 0usize;
    let mut total = 0usize;
    let mut js = JSeg::new();
    loop {
        let m = jpg_seg_next(data, &mut js);
        if m == 0 || m == 0xDA || m == 0xD9 {
            break;
        }
        if m != 0xE2 || js.len < ICC_HDR_LEN || &data[js.ptr..js.ptr + 12] != ICC_SIG {
            continue;
        }
        let seq = data[js.ptr + 12] as usize;
        let cnt = data[js.ptr + 13] as usize;
        if cnt == 0 || (count != 0 && count != cnt) {
            return (crate::ERR_CHUNK, Vec::new());
        }
        count = cnt;
        if seq == 0 || seq > cnt || parts[seq].is_some() {
            return (ERR_CHUNK, Vec::new());
        }
        let n = js.len - ICC_HDR_LEN;
        parts[seq] = Some((js.ptr + ICC_HDR_LEN, n));
        total += n;
    }
    if count == 0 {
        return (0, Vec::new());
    }
    if total == 0 || (1..=count).any(|s| parts[s].is_none()) {
        return (ERR_CHUNK, Vec::new());
    }
    let mut out = Vec::with_capacity(total);
    for &(p, n) in parts[1..=count].iter().flatten() {
        out.extend_from_slice(&data[p..p + n]);
    }
    (1, out)
}

/// the complete APP2 marker segments that embed an ICC profile in a JPEG (written after
/// APP0 / APP1); empty for an empty profile  [icc_jpeg_segments]
pub fn icc_jpeg_segments(icc: &[u8]) -> Result<Vec<u8>, i32> {
    if icc.is_empty() {
        return Ok(Vec::new());
    }
    let nseg = icc.len().div_ceil(ICC_SEG_MAX);
    if nseg > 255 {
        return Err(ERR_LIMIT);
    }
    let mut out = Vec::with_capacity(icc.len() + nseg * (4 + ICC_HDR_LEN));
    for (k, chunk) in icc.chunks(ICC_SEG_MAX).enumerate() {
        out.extend_from_slice(&[0xFF, 0xE2]);
        out.extend_from_slice(&((chunk.len() + 2 + ICC_HDR_LEN) as u16).to_be_bytes());
        out.extend_from_slice(ICC_SIG);
        out.push((k + 1) as u8);
        out.push(nseg as u8);
        out.extend_from_slice(chunk);
    }
    Ok(out)
}
