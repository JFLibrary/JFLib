// ops_png -- codectest ops 260..279: PNG and the metadata paths of the core encoders
//                                                                 [apps/codectest/src/png_ops.s]
//
//   op 260 encode with metadata  data = IMGOPT (64 bytes) + {u32 dpix, dpiy (16.16), resunit,
//          icclen, exiflen, xmplen} + icc + exif + xmp + surface -> result, file bytes
//   op 261 decode_ex      arg = IMGD_* flags, data = file -> result, the whole IMGMETA (128
//          bytes, blob pointers zeroed) + icc + exif + xmp
//   op 262 png decode_fl  arg = JDF_* flags, data = PNG -> surface record
//   op 263 png encode_cfg data = {u32 level, depth, qcfg, 0} + surface -> result, file

use alloc::vec::Vec;

use crate::{Ct, R_UNKNOWN, surface_in, u32_at};
use gfx::Surface;
use img_core::io;
use img_core::{ImgDec, ImgMeta, ImgOpt};
use img_png::{PngCfg, encode_cfg};

const EM_HDR: usize = 64 + 24; // op 260 fixed header

/// run op (one record via ct.record); false = unknown op
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        260 => {
            if data.len() < EM_HDR {
                ct.record(R_UNKNOWN, &[]);
                return true;
            }
            let mut m = ImgMeta { dpix: u32_at(data, 64), dpiy: u32_at(data, 68), resunit: u32_at(data, 72), ..Default::default() };
            let mut p = EM_HDR;
            for (k, b) in [&mut m.icc, &mut m.exif, &mut m.xmp].into_iter().enumerate() {
                let n = u32_at(data, 76 + k * 4) as usize;
                if n > data.len() - p {
                    ct.record(R_UNKNOWN, &[]);
                    return true;
                }
                *b = data[p..p + n].to_vec();
                p += n;
            }
            let Some(sf) = surface_in(&data[p..]) else {
                ct.record(R_UNKNOWN, &[]);
                return true;
            };
            let mut opt = ImgOpt::from_bytes(&data[..64]);
            opt.meta = Some(&m);
            match io::encode(&sf, Some(&opt)) {
                Ok(f) => ct.record(0, &f),
                Err(e) => ct.record(e, &[]),
            }
        }
        261 => {
            let dec = ImgDec { flags: arg, ..Default::default() };
            let mut m = ImgMeta::default();
            let r = match io::decode_ex(data, Some(&dec), Some(&mut m)) {
                Ok(_) => 0,
                Err(e) => e,
            };
            let mut d = Vec::with_capacity(128 + m.icc.len() + m.exif.len() + m.xmp.len());
            d.extend_from_slice(&m.to_bytes());
            d.extend_from_slice(&m.icc);
            d.extend_from_slice(&m.exif);
            d.extend_from_slice(&m.xmp);
            ct.record(r, &d);
        }
        262 => {
            let mut sf = Surface::default();
            let r = img_png::decode_fl(data, &mut sf, arg);
            ct.surface_out(r, &sf);
        }
        263 => {
            if data.len() < 16 {
                ct.record(R_UNKNOWN, &[]);
                return true;
            }
            let Some(sf) = surface_in(&data[16..]) else {
                ct.record(R_UNKNOWN, &[]);
                return true;
            };
            let cfg = PngCfg { level: u32_at(data, 0), depth: u32_at(data, 4), qcfg: u32_at(data, 8), ..Default::default() };
            match encode_cfg(&sf, &cfg) {
                Ok(f) => ct.record(0, &f),
                Err(e) => ct.record(e, &[]),
            }
        }
        _ => return false,
    }
    true
}
