// img_png -- the PNG codec                                  [src/img/png.s, imgcore.s PNG parts]
//
//   CODEC                          the codec descriptor (probe / decode_ex / encode_opt; scan,
//                                  decode_fl, save for the classic API and BMP's embedded PNG)
//   dec::decode_fl(data, sf, jdf)  png_decode_fl (dec.rs: decoder)
//   enc::encode_cfg(sf, cfg)       png_encode_cfg (enc.rs: encoder, PngCfg, put_chunk)
//   glue::scan / probe / decode_ex / encode_opt   imgcore.s (glue.rs)
//   encode(px, w, h, stride, grey) the screenshot writer of ui (--shot): rows of 32-bit
//                                  B, G, R, x (RGB) or 8-bit grey -> png_encode's file
//   decode_pargb(data)             png_decode: a FMT_PARGB surface or None (the hook of
//                                  text::Text::png_decode, CBDT colour glyphs)
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;

use gfx::{FMT_A8, FMT_XRGB};
use img_core::{Codec, IFC_DECODE, IFC_ENCODE, PNG_IOM, Surface};

pub mod dec;
pub mod enc;
pub mod glue;

pub use dec::decode_fl;
pub use enc::{PNGD_CLASSIC, PngCfg, encode_cfg, put_chunk};

/// the PNG codec (png_codec)
pub static CODEC: Codec = Codec {
    probe: Some(glue::probe),
    decode: Some(glue::decode_ex),
    encode: Some(glue::encode_opt),
    caps: IFC_DECODE | IFC_ENCODE,
    iom: PNG_IOM,
    scan: Some(glue::scan),
    decode_fl: Some(dec::decode_fl),
    save: Some(save),
};

/// png_decode: the file as a new FMT_PARGB surface, None on any error
pub fn decode_pargb(data: &[u8]) -> Option<Surface> {
    let mut sf = Surface::default();
    if dec::decode_fl(data, &mut sf, 0) == 0 { Some(sf) } else { None }
}

// the classic encoder of img_save (png_encode, level 6)
fn save(sf: &Surface, _quality: u32) -> Option<Vec<u8>> {
    enc::encode_default(sf)
}

/// the PNG file of w x h pixels: 32-bit B, G, R, x rows (RGB) or, with `grey`, 8-bit rows;
/// `stride` = bytes per row (png_encode of an XRGB / A8 surface)
pub fn encode(px: &[u8], w: usize, h: usize, stride: usize, grey: bool) -> Vec<u8> {
    let fmt = if grey { FMT_A8 } else { FMT_XRGB };
    let mut sf = Surface::new(w as i32, h as i32, fmt);
    let rb = if grey { w } else { w * 4 };
    let ds = sf.stride as usize;
    for y in 0..h {
        sf.px[y * ds..y * ds + rb].copy_from_slice(&px[y * stride..y * stride + rb]);
    }
    enc::encode_default(&sf).unwrap_or_default()
}
