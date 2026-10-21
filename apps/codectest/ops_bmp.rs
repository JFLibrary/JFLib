// ops_bmp -- codectest ops 280..299: BMP  [apps/codectest/src/bmp_ops.s]
//
//   op 280 bmp_decode_fl  arg = JDF_* flags, data = BMP -> surface_out record
//   op 281 bmp_encode_cfg data = {u32 depth, qcfg, ppmx, ppmy} + surface -> result, file

use alloc::vec::Vec;

use crate::{Ct, surface_in};
use gfx::Surface;
use img_bmp::BmpCfg;

/// run op (one record via ct.record); false = unknown op
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        280 => {
            let mut sf = Surface::default();
            let e = img_bmp::decode_fl(data, &mut sf, arg);
            ct.surface_out(e, &sf);
        }
        281 => {
            let sf = if data.len() < 16 { None } else { surface_in(&data[16..]) };
            let Some(sf) = sf else {
                ct.record(-100, &[]);
                return true;
            };
            let u = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
            let cfg = BmpCfg { depth: u(0), qcfg: u(4), ppmx: u(8), ppmy: u(12), cancel: None };
            let mut out = Vec::new();
            let e = img_bmp::encode_cfg(&sf, &cfg, &mut out);
            ct.record(e, if e == 0 { &out } else { &[] });
        }
        _ => return false,
    }
    true
}
