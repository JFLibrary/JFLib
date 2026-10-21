// ops_meta -- codectest ops 240..259: codec infrastructure (io, meta, exif, icc, gzip,
// deflate_raw / inflate_raw_ex)                                  [apps/codectest/src/meta_ops.s]
//
// "meta record" (MR, 80 bytes): IMGMETA bytes 0..39, u64 icclen, exiflen, xmplen, IMGMETA
// bytes 88..103 (crate::meta_rec).
//
//   op 240 decode_ex      data = {u32 flags, hint, frame, usemeta, u64 maxpx, u32 cancel, u32 0}
//                         + file -> {u32 w, h, fmt | flags << 8, MR, icc, exif, xmp, pixels}
//   op 241 probe          arg = hint | 256 (detect only) | 512 (meta = None) -> IMGF, MR
//   op 242 format_info    arg = fmt -> "name\0exts\0mime\0" u32 caps, iom, codec != 0
//   op 243 format_from_path  data = path -> IMGF
//   op 244 gzip_decode    arg = dstcap -> bytes / err, {u64 consumed, bytes}
//   op 245 gzip_encode    arg = level -> member      op 246 deflate_raw  arg = level -> data
//   op 247 exif_read      -> result, {EXIFINFO (24), u32 exif_orientation}
//   op 248 exif_copy_orient1   op 249 icc_jpeg_collect   op 250 icc_jpeg_segments
//   op 251 icc_check      op 252 encode  data = IMGOPT (64) + surface -> file
//   op 253 convert        arg = fmt (| 256: surf_convert) data = surface -> surface
//   op 254 row converters arg = 0 u16 LE, 1 u16 BE, 2 half, 3 f32, 4 f32 -> sRGB,
//                         5 u16_to_u8 per u16, 6 half_to_f32 per u16, 7 / 8 bit readers
//   op 255 orient         op 256 get_row  arg = fmt -> all rows
//   op 257 inflate_raw_ex arg = dstcap     op 258 use_all_codecs     op 259 filter_list

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::AtomicU32;

use crate::{Ct, R_UNKNOWN, meta_rec, surface_bytes, surface_in, u32_at};
use gfx::Surface;
use img_core::bits::{BrL, BrM};
use img_core::deflate::deflate_raw;
use img_core::exif::{exif_copy_orient1, exif_orientation, exif_read};
use img_core::gzip::{gzip_decode, gzip_encode};
use img_core::icc::{icc_check, icc_jpeg_collect, icc_jpeg_segments};
use img_core::inflate::inflate_raw_ex;
use img_core::io;
use img_core::meta::{convert, get_row, half_to_f32, orient, row_f32_to_srgb8, row_f32_to_u8, row_half_to_u8,
                     row_u16_to_u8, surf_convert, u16_to_u8};
use img_core::{ImgDec, ImgMeta, ImgOpt};

/// run op (one record via ct.record); false = unknown op
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        240 => decex(ct, data),
        241 => {
            let mut m = ImgMeta::default();
            let r = if arg & 256 != 0 {
                io::detect(data, arg & 255)
            } else if arg & 512 != 0 {
                io::probe(data, None)
            } else {
                io::probe(data, Some(&mut m))
            };
            ct.record(r as i32, &meta_rec(&m));
        }
        242 => match io::format_info(arg) {
            None => ct.record(-1, &[]),
            Some(f) => {
                let mut d = Vec::new();
                for s in [f.name, f.exts, f.mime] {
                    d.extend_from_slice(s.as_bytes());
                    d.push(0);
                }
                d.extend_from_slice(&f.caps.to_le_bytes());
                d.extend_from_slice(&f.iom.to_le_bytes());
                d.extend_from_slice(&(f.codec.is_some() as u32).to_le_bytes());
                ct.record(0, &d);
            }
        },
        243 => ct.record(io::format_from_bytes(data) as i32, &[]),
        244 | 257 => {
            let mut buf = vec![0u8; arg as usize];
            let r = if op == 244 { gzip_decode(data, &mut buf) } else { inflate_raw_ex(data, &mut buf) };
            let (res, n, used) = match r {
                Ok((n, used)) => (n as i32, n, used),
                Err(e) => (e, 0, 0),
            };
            let mut d = Vec::with_capacity(8 + n);
            d.extend_from_slice(&(used as u64).to_le_bytes());
            d.extend_from_slice(&buf[..n]);
            ct.record(res, &d);
        }
        245 => ct.record(0, &gzip_encode(data, arg as i32)),
        246 => ct.record(0, &deflate_raw(data, arg as i32)),
        247 => {
            let (r, info) = exif_read(data);
            let mut d = Vec::with_capacity(28);
            d.extend_from_slice(&info.to_bytes());
            d.extend_from_slice(&exif_orientation(data).to_le_bytes());
            ct.record(r, &d);
        }
        248 => ct.record(0, &exif_copy_orient1(data)),
        249 => {
            let (r, p) = icc_jpeg_collect(data);
            ct.record(r, &p);
        }
        250 => match icc_jpeg_segments(data) {
            Ok(v) => ct.record(0, &v),
            Err(e) => ct.record(e, &[]),
        },
        251 => ct.record(icc_check(data) as i32, &[]),
        252 => {
            if data.len() < 64 {
                ct.record(R_UNKNOWN, &[]);
                return true;
            }
            let opt = ImgOpt::from_bytes(&data[..64]);
            let Some(sf) = surface_in(&data[64..]) else {
                ct.record(R_UNKNOWN, &[]);
                return true;
            };
            match io::encode(&sf, Some(&opt)) {
                Ok(f) => ct.record(0, &f),
                Err(e) => ct.record(e, &[]),
            }
        }
        253 => {
            let Some(mut sf) = surface_in(data) else {
                ct.record(R_UNKNOWN, &[]);
                return true;
            };
            if arg & 256 != 0 {
                match surf_convert(&sf, (arg & 255) as u8) {
                    Ok(s2) => ct.surface_out(0, &s2),
                    Err(e) => ct.surface_out(e, &Surface::default()),
                }
            } else {
                let r = convert(&mut sf, (arg & 255) as u8);
                ct.surface_out(r, &sf);
            }
        }
        254 => rowconv(ct, arg, data),
        255 => {
            let Some(mut sf) = surface_in(data) else {
                ct.record(R_UNKNOWN, &[]);
                return true;
            };
            let r = orient(&mut sf, arg);
            ct.surface_out(r, &sf);
        }
        256 => {
            let Some(sf) = surface_in(data) else {
                ct.record(R_UNKNOWN, &[]);
                return true;
            };
            let (w, h) = (sf.w as usize, sf.h as usize);
            let mut d = vec![0u8; w * h * 4];
            for y in 0..h {
                get_row(&sf, y, &mut d[y * w * 4..(y + 1) * w * 4], (arg & 255) as u8);
            }
            ct.record(0, &d);
        }
        258 => {
            img_all::use_all_codecs();
            ct.record(0, &[]);
        }
        259 => {
            let cap = match arg >> 16 {
                0 => 4096,
                c => c as usize,
            };
            let l = io::filter_list(arg & 0xFFFF, cap);
            ct.record(l.len() as i32, &l);
        }
        _ => return false,
    }
    true
}

// op 240: img_decode_ex with the record's IMGDEC
fn decex(ct: &mut Ct, data: &[u8]) {
    if data.len() < 32 {
        ct.record(R_UNKNOWN, &[]);
        return;
    }
    let cancel = AtomicU32::new(u32_at(data, 24));
    let dec = ImgDec {
        flags: u32_at(data, 0),
        hint: u32_at(data, 4),
        frame: u32_at(data, 8),
        maxpx: u64::from_le_bytes(data[16..24].try_into().unwrap()),
        maxmem: 0,
        cancel: Some(&cancel),
    };
    let usemeta = u32_at(data, 12) != 0;
    let mut m = ImgMeta::default();
    let r = io::decode_ex(&data[32..], Some(&dec), if usemeta { Some(&mut m) } else { None });
    let (res, sf) = match r {
        Ok(s) => (0, s),
        Err(e) => (e, Surface::default()),
    };
    let sb = surface_bytes(&sf); // {w, h, fmt | flags << 8} + rows
    let mut d = Vec::with_capacity(sb.len() + 80 + m.icc.len() + m.exif.len() + m.xmp.len());
    d.extend_from_slice(&sb[..12]);
    d.extend_from_slice(&meta_rec(&m));
    d.extend_from_slice(&m.icc);
    d.extend_from_slice(&m.exif);
    d.extend_from_slice(&m.xmp);
    d.extend_from_slice(&sb[12..]);
    ct.record(res, &d);
}

// op 254: row converters and bit readers
fn rowconv(ct: &mut Ct, kind: u32, data: &[u8]) {
    let n2 = data.len() / 2;
    let n4 = data.len() / 4;
    let u16s = || data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<u16>>();
    let f32s = || data.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect::<Vec<f32>>();
    let out: Vec<u8> = match kind {
        0 | 1 => {
            let mut o = vec![0u8; n2];
            row_u16_to_u8(&mut o, data, kind == 1);
            o
        }
        2 => {
            let mut o = vec![0u8; n2];
            row_half_to_u8(&mut o, &u16s());
            o
        }
        3 => {
            let mut o = vec![0u8; n4];
            row_f32_to_u8(&mut o, &f32s());
            o
        }
        4 => {
            let mut o = vec![0u8; n4];
            row_f32_to_srgb8(&mut o, &f32s());
            o
        }
        5 => u16s().iter().map(|&v| u16_to_u8(v as u32)).collect(),
        6 => u16s().iter().flat_map(|&v| half_to_f32(v as u32).to_le_bytes()).collect(),
        7 | 8 => match bits(data, kind == 8) {
            Some(o) => o,
            None => {
                ct.record(R_UNKNOWN, &[]);
                return;
            }
        },
        _ => {
            ct.record(R_UNKNOWN, &[]);
            return;
        }
    };
    ct.record(0, &out);
}

// mo_bits: data = {u32 n, u8 widths[n] (1..56), bit stream}; u64 values of GET 5, PEEK 13
// (+ SKIP 13), GET 56, then n variable reads, then the overrun flag
fn bits(data: &[u8], lsb: bool) -> Option<Vec<u8>> {
    if data.len() < 4 {
        return None;
    }
    let n = u32_at(data, 0) as usize;
    if n + 4 > data.len() {
        return None;
    }
    let widths = &data[4..4 + n];
    let stream = &data[4 + n..];
    let mut v: Vec<u64> = Vec::with_capacity(n + 4);
    let over = if lsb {
        let mut r = BrL::new(stream, 0);
        r.refill();
        v.push(r.get(5));
        r.refill();
        v.push(r.peek(13));
        r.skip(13);
        r.refill();
        v.push(r.get(56));
        for &w in widths {
            r.refill();
            v.push(r.get(w as u32));
        }
        r.overrun()
    } else {
        let mut r = BrM::new(stream, 0);
        r.refill();
        v.push(r.get(5));
        r.refill();
        v.push(r.peek(13));
        r.skip(13);
        r.refill();
        v.push(r.get(56));
        for &w in widths {
            r.refill();
            v.push(r.get(w as u32));
        }
        r.overrun()
    };
    v.push(over as u64);
    Some(v.iter().flat_map(|x| x.to_le_bytes()).collect())
}
