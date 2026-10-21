// ops_tga -- codectest ops 80..99: TGA  [apps/codectest/src/tga_ops.s]
//
// No TGA-specific ops so far: tools/tga_check.py drives the codec through the generic ops 240
// (img_decode_ex), 241 (img_probe) and 252 (img_encode) of ops_meta.

use crate::Ct;

/// run op (one record via ct.record); false = unknown op
pub fn op(_ct: &mut Ct, _op: u32, _arg: u32, _data: &[u8]) -> bool {
    false
}
