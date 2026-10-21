// ops_jxr -- codectest ops 200..219: JPEG XR (img_jxr)  [apps/codectest/src/jxr_ops.s]
//
//   200 OP_JDUMP  arg = mode (0 reconstructed planes, 1 before the inverse transform, 2 DC grids,
//                 3 native output samples): records img_jxr::dbg_planes(data, mode) -> {result, blob}
//   201 OP_JTIME  arg = repetitions (low 16 bits, 0 = 1) | IMGDEC flags << 16: decodes the file
//                 that often with img_jxr's decoder (the surface freed each time) and records
//                 {result, u64 us}
//   202 OP_JDTIME arg = mode (low 8 bits, see 200) | repetitions << 8: times dbg_planes
//                 (profiling of the decoder stages) and records {result, u64 us}

use crate::Ct;
use img_core::{ImgDec, ImgMeta, Surface};

const OP_JDUMP: u32 = 200;
const OP_JTIME: u32 = 201;
const OP_JDTIME: u32 = 202;

/// run op (one record via ct.record); false = unknown op  [jxr_op]
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        OP_JDUMP => {
            let (r, b) = img_jxr::dbg_planes(data, arg);
            ct.record(r, &b);
        }
        OP_JTIME => {
            let reps = match arg & 0xFFFF {
                0 => 1,
                n => n,
            };
            // the assembly calls jxr_decode with a zeroed IMGDEC (flags set) and no IMGMETA
            let dec = ImgDec { flags: arg >> 16, ..Default::default() };
            let t0 = plat::time_us();
            let mut res = 0;
            for _ in 0..reps {
                let mut sf = Surface::default();
                let mut m = ImgMeta::default();
                res = img_jxr::decode(data, &mut sf, &dec, &mut m);
                if res != 0 {
                    break;
                }
            }
            let us = plat::time_us() - t0;
            ct.record(res, &us.to_le_bytes());
        }
        OP_JDTIME => {
            let mode = arg & 255;
            let reps = match arg >> 8 {
                0 => 1,
                n => n,
            };
            let t0 = plat::time_us();
            let mut res = 0;
            for _ in 0..reps {
                res = img_jxr::dbg_planes(data, mode).0;
                if res != 0 {
                    break;
                }
            }
            let us = plat::time_us() - t0;
            ct.record(res, &us.to_le_bytes());
        }
        _ => return false,
    }
    true
}
