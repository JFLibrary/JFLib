// ops_tiff -- codectest ops 60..79: TIFF (img_tiff)  [apps/codectest/src/tiff_ops.s]
//
//   op 60 time img_decode_ex   data = TIFF file, arg = repetitions | IMGD_* flags << 16
//                              -> {u32 microseconds per call, u32 w, u32 h} (result: error code)
//   op 61 img_encode + meta    data = IMGOPT (64 bytes, pointers ignored) + {u32 dpix (16.16),
//                              u32 dpiy, u32 resunit, u32 icclen, u32 xmplen, u32 exiflen} + ICC
//                              + XMP + EXIF bytes + {u32 w, h, fmt, pixels} -> file bytes
//                              (opt.meta holds these values)
//   op 62 time img_encode      data = IMGOPT + surface, arg = repetitions
//                              -> {u32 microseconds per call, u32 file bytes}

use alloc::vec::Vec;

use crate::{Ct, surface_in, u32_at};
use img_core::io::{decode_ex, encode};
use img_core::{ImgDec, ImgMeta, ImgOpt};

const OP_TDEC: u32 = 60;
const OP_ENCMETA: u32 = 61;
const OP_TENC: u32 = 62;
const IO_SIZE: usize = 64;
const TM_HDR: usize = 24; // op 61 metadata header
const R_BAD: i32 = -100;

/// run op (one record via ct.record); false = unknown op  [tiff_op]
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        OP_TDEC => {
            let reps = match arg & 0xFFFF {
                0 => 1,
                n => n,
            };
            let dec = ImgDec { flags: arg >> 16, ..Default::default() };
            let t0 = plat::time_us();
            let (mut w, mut h) = (0u32, 0u32);
            for _ in 0..reps {
                match decode_ex(data, Some(&dec), None) {
                    Ok(sf) => (w, h) = (sf.w as u32, sf.h as u32),
                    Err(e) => {
                        ct.record(e, &[]);
                        return true;
                    }
                }
            }
            let us = ((plat::time_us() - t0) / reps as u64) as u32;
            let mut rec = Vec::with_capacity(12);
            for v in [us, w, h] {
                rec.extend_from_slice(&v.to_le_bytes());
            }
            ct.record(0, &rec);
        }
        OP_ENCMETA => {
            if data.len() < IO_SIZE + TM_HDR {
                ct.record(R_BAD, &[]);
                return true;
            }
            let mut opt = ImgOpt::from_bytes(&data[..IO_SIZE]);
            let hd = &data[IO_SIZE..IO_SIZE + TM_HDR];
            let mut meta = ImgMeta { dpix: u32_at(hd, 0), dpiy: u32_at(hd, 4), resunit: u32_at(hd, 8), ..Default::default() };
            let mut rest = &data[IO_SIZE + TM_HDR..];
            let mut blob = |n: u32| -> Option<Vec<u8>> {
                let n = n as usize;
                if n > rest.len() {
                    return None;
                }
                let b = rest[..n].to_vec();
                rest = &rest[n..];
                Some(b)
            };
            let (Some(icc), Some(xmp), Some(exif)) = (blob(u32_at(hd, 12)), blob(u32_at(hd, 16)), blob(u32_at(hd, 20))) else {
                ct.record(R_BAD, &[]);
                return true;
            };
            meta.icc = icc;
            meta.xmp = xmp;
            meta.exif = exif;
            let Some(sf) = surface_in(rest) else {
                ct.record(R_BAD, &[]);
                return true;
            };
            opt.meta = Some(&meta);
            match encode(&sf, Some(&opt)) {
                Ok(file) => ct.record(0, &file),
                Err(e) => ct.record(e, &[]),
            }
        }
        OP_TENC => {
            if data.len() < IO_SIZE {
                ct.record(R_BAD, &[]);
                return true;
            }
            let opt = ImgOpt::from_bytes(&data[..IO_SIZE]);
            let Some(sf) = surface_in(&data[IO_SIZE..]) else {
                ct.record(R_BAD, &[]);
                return true;
            };
            let reps = match arg & 0xFFFF {
                0 => 1,
                n => n,
            };
            let t0 = plat::time_us();
            let mut bytes = 0u32;
            for _ in 0..reps {
                match encode(&sf, Some(&opt)) {
                    Ok(file) => bytes = file.len() as u32,
                    Err(e) => {
                        ct.record(e, &[]);
                        return true;
                    }
                }
            }
            let us = ((plat::time_us() - t0) / reps as u64) as u32;
            let mut rec = Vec::with_capacity(8);
            rec.extend_from_slice(&us.to_le_bytes());
            rec.extend_from_slice(&bytes.to_le_bytes());
            ct.record(0, &rec);
        }
        _ => return false,
    }
    true
}
