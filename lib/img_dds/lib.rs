// img_dds -- DirectDraw Surface reader / writer and the BC1..BC7 block codecs
//                         [src/img/dds.s, bc_dec.s, bc_enc.s, bc6h_enc.s, bc7_enc.s]
//
// Modules:
//   read     DDS reader: probe, decode (headers, DX10, legacy pixel formats, mipmaps,
//            cube maps as a 4 x 3 cross, arrays, volumes)                     [dds.s CD]
//   write    DDS writer: encode (legacy / DX10 headers, BC and raw formats, mip chains,
//            cube crosses)                                                    [dds.s CB]
//   dec      BC1..BC7 block decoders, decode_to / decode                      [bc_dec.s]
//   enc      BC1..BC5 block encoders, the surface driver (block rows on the thread pool)
//            and the float pixel-set helpers of the BC6H / BC7 encoders       [bc_enc.s]
//   bc6h     BC6H block encoder                                               [bc6h_enc.s]
//   bc7      BC7 block encoder                                                [bc7_enc.s]
//   tables   literal tables of the BC codecs
//
//   CODEC    the codec descriptor (dds_codec): probe, decode, encode
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use img_core::*;

pub mod bc6h;
pub mod bc7;
pub mod dec;
pub mod enc;
pub mod read;
pub mod tables;
pub mod write;

pub use dec::{bc6h_decode_half, block_bytes, decode, decode_block, decode_to};
pub use enc::{encode as bc_encode, encode_block, encode_to, encoded_size};
pub use read::{decode as dds_decode, probe as dds_probe};
pub use write::encode as dds_encode;

// ------------------------------------------------------------------ img.inc (BC / DDS parts)

/// block formats of the BC codecs (BCF_*); 1..7 equal DDSF_BC1..BC7
pub const BCF_BC1: u32 = 1;
pub const BCF_BC2: u32 = 2;
pub const BCF_BC3: u32 = 3;
pub const BCF_BC4: u32 = 4; // BC4 unsigned (ATI1 / BC4U)
pub const BCF_BC5: u32 = 5; // BC5 unsigned (ATI2 / BC5U)
pub const BCF_BC6H: u32 = 6; // BC6H UF16
pub const BCF_BC7: u32 = 7;
pub const BCF_BC4S: u32 = 8; // BC4 signed
pub const BCF_BC5S: u32 = 9; // BC5 signed
pub const BCF_BC6HS: u32 = 10; // BC6H SF16
pub const BCF_COUNT: u32 = 11;

/// encoder configuration (cfg of encode_block / encode_to)
pub const BCQ_LEVEL: u32 = 3; // mask: 0 default (= normal), BCQ_FAST, BCQ_NORMAL, BCQ_SLOW
pub const BCQ_FAST: u32 = 1; // BC7: mode 6 only; BC1..5: one refinement
pub const BCQ_NORMAL: u32 = 2; // BC7: modes 1, 3, 4, 5, 6, 7
pub const BCQ_SLOW: u32 = 3; // BC7: all modes / rotations / p-bits
pub const BCQ_UNIFORM: u32 = 4; // BC1..BC3 colour: uniform RGB error (else perceptual)
pub const BCQ_ATHR_SHIFT: u32 = 8; // BC1: bits 8..16 = alpha threshold + 1 (0 = default 128)

/// more ImgOpt::ddsfmt values (DDSF_BC1 .. DDSF_R8G8 are in img_core)
pub const DDSF_BC4S: u32 = 17; // BC4 signed (legacy FourCC "BC4S")
pub const DDSF_BC5S: u32 = 18; // BC5 signed ("BC5S")
pub const DDSF_BC6HS: u32 = 19; // BC6H SF16 (DX10)
pub const DDSF_R8: u32 = 20; // DXGI R8_UNORM (grey, DX10)
pub const DDSF_BGR8: u32 = 21; // legacy 24-bit R8G8B8
pub const DDSF_COUNT: u32 = 22;
/// more ImgOpt::mipfilter values (0 = MIPF_FANT)
pub const MIPF_FANT: u32 = 5; // area average
pub const MIPF_LANCZOS: u32 = 6; // Lanczos-3
/// ImgOpt::flags: uniform error metric for BC1..BC3 colour
pub const IOF_DDS_UNIFORM: u32 = 0x10000;

/// the codec descriptor  [dds_codec]
pub static CODEC: Codec = Codec {
    probe: Some(read::probe),
    decode: Some(read::decode),
    encode: Some(write::encode),
    caps: IFC_DECODE | IFC_ENCODE,
    iom: IOM_DDSFMT | IOM_MIPMAPS | IOM_MIPFILTER | IOM_EFFORT | IOM_SRGB | IOM_CUBE,
    ..Codec::EMPTY
};
