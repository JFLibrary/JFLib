// img_tiff -- TIFF decoder and encoder (+ CCITT)          [src/img/tiff.s, tiff_ccitt.s]
//
//   CODEC                      the IMGCODEC descriptor (probe, decode, encode, caps, iom)
//   dec::probe / dec::decode   tiff_probe / tiff_decode (classic + BigTIFF, strips / tiles,
//                              none / LZW / Deflate / PackBits / JPEG / CCITT, predictors,
//                              1..32-bit integers and floats, every colour model of the
//                              assembly, pages, metadata; parallel for big images)
//   enc::encode                tiff_encode (none / LZW / Deflate / PackBits, 32 / 24 / 8 bit)
//
// Modules: dec (the reader; IFDs through img_core::exif::Tifr), lzw (tc_lzw), ccitt
// (tiff_ccitt.s), enc (the writer).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use img_core::{Codec, IFC_DECODE, IFC_ENCODE, IOM_BITDEPTH, IOM_DITHER, IOM_ICC, IOM_TIFFCOMP, IOM_XMP};

mod ccitt;
pub mod dec;
pub mod enc;
mod lzw;

/// the codec descriptor  [tiff_codec]
pub static CODEC: Codec = Codec {
    probe: Some(dec::probe),
    decode: Some(dec::decode),
    encode: Some(enc::encode),
    caps: IFC_DECODE | IFC_ENCODE,
    iom: IOM_BITDEPTH | IOM_DITHER | IOM_TIFFCOMP | IOM_ICC | IOM_XMP,
    ..Codec::EMPTY
};
