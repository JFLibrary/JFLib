// img_hevc -- HEVC (H.265) intra decoder and encoder for HEIC still images
//                                  [src/img/hevc.s, hevc_ps.s, hevc_ctu.s, hevc_rec.s, hevc_lf.s,
//                                   hevc_enc.s, ASMLIB/inc/hevc_int.inc]
//
//   Hevc::new(flags, maxctu)          the decoder (hevc_new); set_cancel, nal (hevc_nal),
//                                     annexb (hevc_annexb), finish -> Pic (hevc_finish),
//                                     newpic (hevc_newpic)
//   encode(&EncCfg) -> EncOut         the encoder (hevc_encode): VPS, SPS, PPS, IDR slice
//
// Modules: dec (the decoder record and the driver: hevc.s), ps (NAL / RBSP reader, parameter
// sets, slice header, tiles: hevc_ps.s), ctu (CABAC, slice data: hevc_ctu.s), rec (intra
// prediction, reconstruction: hevc_rec.s), lf (deblocking, SAO: hevc_lf.s), enc (the encoder:
// hevc_enc.s).  Model of both: tools/hevc_ref.py / tools/hevc_enc_ref.py (bit-exact).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod ctu;
mod dec;
pub mod enc;
mod lf;
mod ps;
mod rec;

pub use dec::{Hevc, Pic, HVF_NODEBLOCK, HVF_NOSAO};
pub use enc::{encode, EncCfg, EncOut, HEF_LOSSLESS, HEF_NODEBLOCK};
