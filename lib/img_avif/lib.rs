// img_avif -- AVIF (AV1 still images in HEIF): the codec entry points and the writer
//                                                    [src/img/avif.s, avif_enc.s]
//
//   probe(data, meta) -> recognised           avif_probe = heic_probe
//   decode(data, sf, dec, meta) -> 0 / ERR_*  avif_decode = heic_decode
//   enc::encode(sf, opt, out) -> 0 / ERR_*    avif_encode (the AV1 encoder img_av1::encode)
//   CODEC                                     avif_codec
//
// Reading is the HEIF reader of img_heif (it handles av01 items, grids, alpha,
// transformations, colour and metadata the same way as for HEIC; the AV1 decoder is img_av1).
// The dispatcher picks this codec for files with an AVIF brand (img_core::io bmff kind).
// Writing: enc (avif_enc.s).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use img_core::{Codec, IFC_DECODE, IFC_ENCODE, IOM_CHROMA, IOM_EXIF, IOM_ICC, IOM_LOSSLESS, IOM_QUALITY, IOM_XMP};

pub mod enc; // r9-av1enc: the AVIF writer (avif_enc.s)

pub use img_heif::{decode, probe};

/// the encoder options the AVIF writer honours (AVIF_IOM)
pub const AVIF_IOM: u32 = IOM_QUALITY | IOM_LOSSLESS | IOM_CHROMA | IOM_ICC | IOM_EXIF | IOM_XMP;

/// the codec descriptor  [avif_codec: probe, decode, encode (avif_enc.s), IFC_DECODE | IFC_ENCODE,
/// AVIF_IOM]
pub static CODEC: Codec = Codec {
    probe: Some(img_heif::probe),
    decode: Some(img_heif::decode),
    encode: Some(enc::encode),     // r9-av1enc
    caps: IFC_DECODE | IFC_ENCODE, // r9-av1enc
    iom: AVIF_IOM,                 // r9-av1enc
    ..Codec::EMPTY
};
