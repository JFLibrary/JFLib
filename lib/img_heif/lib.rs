// img_heif -- HEIF: the ISO BMFF reader, the HEIF reader (HEIC + AVIF items, grids, alpha,
// transformations, colour, metadata) and the HEIC writer
//                                                    [src/img/isobmff.s, heic.s, heic_enc.s]
//
//   CODEC                          the IMGCODEC descriptor (heic_codec)
//   probe(data, meta)              heic_probe (also avif_probe: the AVIF codec reads
//                                  through this reader)
//   decode(data, sf, dec, meta)    heic_decode (also avif_decode)
//   encode(sf, opt, out)           heic_encode
//
// Modules: bmff (isobmff.s: boxes, items, extents, properties, references), dec (heic.s: the
// reader; HEVC items through img_hevc, AV1 items through img_av1), enc (heic_enc.s: the
// writer; HEVC through img_hevc::encode).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod bmff;
mod dec;
mod enc;

use img_core::{Codec, IFC_DECODE, IFC_ENCODE, IOM_CHROMA, IOM_EXIF, IOM_ICC, IOM_LOSSLESS, IOM_QUALITY, IOM_XMP};

pub use dec::{decode, probe};
pub use enc::encode;

/// the encoder options the HEIC writer honours
pub const HEIC_IOM: u32 = IOM_QUALITY | IOM_LOSSLESS | IOM_CHROMA | IOM_ICC | IOM_EXIF | IOM_XMP;

/// the codec descriptor  [heic_codec]
pub static CODEC: Codec = Codec {
    probe: Some(dec::probe),
    decode: Some(dec::decode),
    encode: Some(enc::encode),
    caps: IFC_DECODE | IFC_ENCODE,
    iom: HEIC_IOM,
    ..Codec::EMPTY
};
