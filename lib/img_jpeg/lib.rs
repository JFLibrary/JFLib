// img_jpeg -- JPEG decoder and encoder  [src/img/jpeg_dec.s, jpeg_enc.s, jpeg_dsp.s,
//                                        imgcore.s (JPEG parts)]
//
//   decode(data) -> Surface / ERR_*                        jpeg_decode (FMT_PARGB, opaque)
//   decode_fl(data, flags) -> Surface / ERR_*              jpeg_decode_fl, JDF_* flags
//   decode_abbrev(tables, data, flags) -> Surface / ERR_*  jpeg_decode_abbrev (JPEG-in-TIFF)
//   encode(sf, quality) -> file / None                     jpeg_encode
//   encode_ex(sf, quality, cfg) -> file / None             jpeg_encode_ex (EncCfg = JENCCFG)
//   meta(data, m, flags) -> 0 / ERR_*                      jpeg_meta (also used for BMP's
//                                                          embedded JPEG)
//   probe / decode_ex / encode_opt                         the codec interface (CODEC)
//
// Modules: dec (jpeg_dec.s), enc (jpeg_enc.s), dsp (jpeg_dsp.s: IDCT / FDCT / colour
// conversion / resampling kernels and the tables), glue (imgcore.s: metadata scan, IMGDEC /
// IMGOPT glue).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use img_core::{Codec, Surface, IFC_DECODE, IFC_ENCODE};

pub mod dec;
pub mod dsp;
pub mod enc;
pub mod glue;

pub use dec::{decode_abbrev, decode_fl};
pub use enc::{encode, encode_ex, EncCfg};
pub use glue::{decode_ex, encode_opt, meta, probe};
// decode flags (img.inc JDF_*) and the encoder options this codec honours
pub use img_core::{JDF_CMYK, JDF_NO_ORIENT, JDF_RGB, JDF_STRAIGHT, JDF_YCC, JPEG_IOM};

/// jpeg_decode: decode a JPEG file into a new FMT_PARGB surface
pub fn decode(data: &[u8]) -> Result<Surface, i32> {
    decode_fl(data, 0)
}

/// the codec descriptor (jpeg_codec); scan / decode_fl / save serve img_core::io's classic
/// paths (img_decode, img_save) and the BMP / TIFF embeddings
pub static CODEC: Codec = Codec {
    probe: Some(glue::probe),
    decode: Some(glue::decode_ex),
    encode: Some(glue::encode_opt),
    caps: IFC_DECODE | IFC_ENCODE,
    iom: JPEG_IOM,
    scan: Some(glue::meta),
    decode_fl: Some(glue::decode_fl_codec),
    save: Some(glue::save_codec),
};
