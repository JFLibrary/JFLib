// read -- DirectDraw Surface reader                                     [src/img/dds.s CD]
//
//   probe(data, meta) -> recognised        dds_probe: the header is valid (the pixel format
//                                          may still be unsupported); meta filled
//   decode(data, sf, dec, meta) -> 0 / ERR_*   dds_decode: mip dec.frame
//
// Headers: "DDS " + DDS_HEADER (size 124, pixel format size 32), optional DX10 header (FourCC
// "DX10": DXGI format, dimension 1D / 2D / 3D, cube flag, array size, alpha mode).  Legacy
// pixel formats (DirectXTex order): DDPF_RGB with any contiguous masks of 8 / 16 / 24 / 32
// bits, DDPF_LUMINANCE (-> grey), DDPF_ALPHA (A8), DDPF_BUMPDUDV (signed), FourCC DXT1..DXT5
// (DXT2 / DXT4 premultiplied), ATI1 / BC4U / BC4S, ATI2 / BC5U / BC5S, D3DFMT numbers 36,
// 110..116.  DX10 formats: the table FMTS (BC1..BC7, 8-bit RGBA / BGRA / BGRX, SNORM, 10:10:10:2,
// R11G11B10 / R9G9B9E5, 16-bit UNORM / SNORM / FLOAT, 32-bit FLOAT, R8G8, R8, A8, 5:6:5,
// 5:5:5:1, 4:4:4:4).  Everything else: ERR_UNSUPPORTED.
//
// Conversion to 8-bit straight BGRA: UNORM n-bit v -> round(255 v / (2^n - 1)); SNORM ->
// round(255 (v' + M) / 2M) with M = 2^(n-1) - 1, v' = max(v, -M); half / float / R11G11B10 /
// RGB9E5 -> round(clamp(v, 0, 1) * 255) (NaN 0); sRGB formats keep their bytes.  One channel
// -> grey; two channels -> (R, G, 0, 255); A8 -> (0, 0, 0, A); missing alpha -> 255.
// Premultiplied files (DXT2, DXT4, DX10 alpha mode 2) return FMT_PARGB (colours clamped to
// alpha), everything else FMT_ARGB.
//
// dec.frame = mip level (meta.frames = mip count; counts beyond log2(max side) + 1 are
// clamped).  Cube maps: the faces of the chosen mip form a 4 x 3 horizontal cross
// (TXF_CROSS; cells without a face are transparent):
//          +Y
//      -X  +Z  +X  -Z
//          -Y
// Texture arrays -> element 0, volume textures -> slice 0 of the chosen mip.  Rows are
// packed; only the data of the decoded mip / faces must be present (ERR_TRUNC otherwise,
// checked before the surface is allocated).

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use gfx::{FMT_ARGB, FMT_PARGB, Surface};
use img_core::meta::{dims_check, mem_check, row_f32_to_u8, row_half_to_u8, surf_alloc};
use img_core::*;

use crate::dec;
use crate::{BCF_BC1, BCF_BC2, BCF_BC3, BCF_BC4, BCF_BC4S, BCF_BC5, BCF_BC5S, BCF_BC6H, BCF_BC6HS, BCF_BC7};

pub(crate) const DDS_MAGIC: u32 = 0x2053_4444; // "DDS "
pub(crate) const FCC_DX10: u32 = 0x3031_5844;
const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_ALPHA: u32 = 0x2;
pub(crate) const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
pub(crate) const DDPF_LUMINANCE: u32 = 0x20000;
const DDPF_BUMPDUDV: u32 = 0x80000;
const DDSCAPS2_CUBEMAP: u32 = 0x200;
const DDSCAPS2_VOLUME: u32 = 0x20_0000;
const DDS_MAXDEPTH: u32 = 65535;

// pixel kinds
const K_BC: u8 = 1; // BCn blocks
const K_INT: u8 = 2; // integer fields (shift / width / signed / role), LUTs
const K_F16: u8 = 3; // halves R [G [B [A]]]
const K_F32: u8 = 4; // floats R [G [B [A]]]
const K_R11: u8 = 5; // R11G11B10_FLOAT
const K_E5: u8 = 6; // R9G9B9E5_SHAREDEXP
const K_BGRA8: u8 = 7;
const K_RGBA8: u8 = 8;
const K_BGRX8: u8 = 9;
const K_RGBX8: u8 = 10;

// channel roles (output byte, or grey)
const CR_B: u8 = 0;
const CR_G: u8 = 1;
const CR_R: u8 = 2;
const CR_A: u8 = 3;
const CR_Y: u8 = 4;

// format flags
const FDF_ALPHA: u8 = 1;
const FDF_SRGB: u8 = 2;
const FDF_GRAY: u8 = 4;
const FDF_FLOAT: u8 = 8;
const FDF_HDR: u8 = 16;
const FDF_LOSSY: u8 = 32;

/// cube faces +X -X +Y -Y +Z -Z -> (column, row) in the 4 x 3 cross
pub(crate) static CROSS: [[u8; 2]; 6] = [[2, 1], [0, 1], [1, 0], [1, 2], [1, 1], [3, 1]];

// legacy FourCCs: fourcc, DXGI format | TXF_PREMUL << 16
static FOURCCS: [(u32, u32); 19] = [
    (0x3154_5844, 71),                             // DXT1
    (0x3254_5844, 74 | (TXF_PREMUL as u32) << 16), // DXT2
    (0x3354_5844, 74),                             // DXT3
    (0x3454_5844, 77 | (TXF_PREMUL as u32) << 16), // DXT4
    (0x3554_5844, 77),                             // DXT5
    (0x3149_5441, 80),                             // ATI1
    (0x5534_4342, 80),                             // BC4U
    (0x5334_4342, 81),                             // BC4S
    (0x3249_5441, 83),                             // ATI2
    (0x5535_4342, 83),                             // BC5U
    (0x5335_4342, 84),                             // BC5S
    (36, 11),                                      // D3DFMT_A16B16G16R16
    (110, 13),                                     // D3DFMT_Q16W16V16U16
    (111, 54),                                     // D3DFMT_R16F
    (112, 34),                                     // D3DFMT_G16R16F
    (113, 10),                                     // D3DFMT_A16B16G16R16F
    (114, 41),                                     // D3DFMT_R32F
    (115, 16),                                     // D3DFMT_G32R32F
    (116, 2),                                      // D3DFMT_A32B32G32R32F
];

/// a format descriptor: DXGI, kind, bytes per pixel / block, aux (BCF_* / float channels),
/// IM_BITS, FDF_*, IM_CHANNELS, 4 channels {shift, width, signed, role}
struct Fd {
    dxgi: u16,
    kind: u8,
    bpp: u8,
    aux: u8,
    bits: u8,
    fl: u8,
    nch: u8,
    ch: [[u8; 4]; 4],
}

const fn fd(dxgi: u16, kind: u8, bpp: u8, aux: u8, bits: u8, fl: u8, nch: u8, ch: [[u8; 4]; 4]) -> Fd {
    Fd { dxgi, kind, bpp, aux, bits, fl, nch, ch }
}

const fn c(sh: u8, wd: u8, sg: u8, role: u8) -> [u8; 4] {
    [sh, wd, sg, role]
}
const C0: [u8; 4] = [0; 4];
const NOCH: [[u8; 4]; 4] = [C0; 4];

const DFA: u8 = FDF_ALPHA;
const DFS: u8 = FDF_SRGB;
const DFG: u8 = FDF_GRAY;
const DFF: u8 = FDF_FLOAT | FDF_HDR;
const DFH: u8 = FDF_HDR;
const DFL: u8 = FDF_LOSSY;
const BC1: u8 = BCF_BC1 as u8;
const BC2: u8 = BCF_BC2 as u8;
const BC3: u8 = BCF_BC3 as u8;
const BC4: u8 = BCF_BC4 as u8;
const BC5: u8 = BCF_BC5 as u8;
const BC6: u8 = BCF_BC6H as u8;
const BC7: u8 = BCF_BC7 as u8;
const BC4S: u8 = BCF_BC4S as u8;
const BC5S: u8 = BCF_BC5S as u8;
const BC6S: u8 = BCF_BC6HS as u8;
const C16RGBA: [[u8; 4]; 4] = [c(0, 16, 0, CR_R), c(16, 16, 0, CR_G), c(32, 16, 0, CR_B), c(48, 16, 0, CR_A)];
const S16RGBA: [[u8; 4]; 4] = [c(0, 16, 1, CR_R), c(16, 16, 1, CR_G), c(32, 16, 1, CR_B), c(48, 16, 1, CR_A)];
const C1010102: [[u8; 4]; 4] = [c(0, 10, 0, CR_R), c(10, 10, 0, CR_G), c(20, 10, 0, CR_B), c(30, 2, 0, CR_A)];
const S8RGBA: [[u8; 4]; 4] = [c(0, 8, 1, CR_R), c(8, 8, 1, CR_G), c(16, 8, 1, CR_B), c(24, 8, 1, CR_A)];

static FMTS: [Fd; 58] = [
    fd(2, K_F32, 16, 4, 32, DFA | DFF, 4, NOCH),
    fd(6, K_F32, 12, 3, 32, DFF, 3, NOCH),
    fd(10, K_F16, 8, 4, 16, DFA | DFF, 4, NOCH),
    fd(9, K_INT, 8, 0, 16, DFA | DFH, 4, C16RGBA),
    fd(11, K_INT, 8, 0, 16, DFA | DFH, 4, C16RGBA),
    fd(13, K_INT, 8, 0, 16, DFA | DFH, 4, S16RGBA),
    fd(16, K_F32, 8, 2, 32, DFF, 2, NOCH),
    fd(23, K_INT, 4, 0, 10, DFA | DFH, 4, C1010102),
    fd(24, K_INT, 4, 0, 10, DFA | DFH, 4, C1010102),
    fd(26, K_R11, 4, 3, 16, DFF, 3, NOCH),
    fd(27, K_RGBA8, 4, 0, 8, DFA, 4, NOCH),
    fd(28, K_RGBA8, 4, 0, 8, DFA, 4, NOCH),
    fd(29, K_RGBA8, 4, 0, 8, DFA | DFS, 4, NOCH),
    fd(31, K_INT, 4, 0, 8, DFA, 4, S8RGBA),
    fd(34, K_F16, 4, 2, 16, DFF, 2, NOCH),
    fd(35, K_INT, 4, 0, 16, DFH, 2, [c(0, 16, 0, CR_R), c(16, 16, 0, CR_G), C0, C0]),
    fd(37, K_INT, 4, 0, 16, DFH, 2, [c(0, 16, 1, CR_R), c(16, 16, 1, CR_G), C0, C0]),
    fd(41, K_F32, 4, 1, 32, DFF | DFG, 1, NOCH),
    fd(49, K_INT, 2, 0, 8, 0, 2, [c(0, 8, 0, CR_R), c(8, 8, 0, CR_G), C0, C0]),
    fd(51, K_INT, 2, 0, 8, 0, 2, [c(0, 8, 1, CR_R), c(8, 8, 1, CR_G), C0, C0]),
    fd(54, K_F16, 2, 1, 16, DFF | DFG, 1, NOCH),
    fd(56, K_INT, 2, 0, 16, DFH | DFG, 1, [c(0, 16, 0, CR_Y), C0, C0, C0]),
    fd(58, K_INT, 2, 0, 16, DFH | DFG, 1, [c(0, 16, 1, CR_Y), C0, C0, C0]),
    fd(61, K_INT, 1, 0, 8, DFG, 1, [c(0, 8, 0, CR_Y), C0, C0, C0]),
    fd(63, K_INT, 1, 0, 8, DFG, 1, [c(0, 8, 1, CR_Y), C0, C0, C0]),
    fd(65, K_INT, 1, 0, 8, DFA | DFG, 1, [c(0, 8, 0, CR_A), C0, C0, C0]),
    fd(67, K_E5, 4, 3, 9, DFF, 3, NOCH),
    fd(70, K_BC, 8, BC1, 8, DFA | DFL, 4, NOCH),
    fd(71, K_BC, 8, BC1, 8, DFA | DFL, 4, NOCH),
    fd(72, K_BC, 8, BC1, 8, DFA | DFL | DFS, 4, NOCH),
    fd(73, K_BC, 16, BC2, 8, DFA | DFL, 4, NOCH),
    fd(74, K_BC, 16, BC2, 8, DFA | DFL, 4, NOCH),
    fd(75, K_BC, 16, BC2, 8, DFA | DFL | DFS, 4, NOCH),
    fd(76, K_BC, 16, BC3, 8, DFA | DFL, 4, NOCH),
    fd(77, K_BC, 16, BC3, 8, DFA | DFL, 4, NOCH),
    fd(78, K_BC, 16, BC3, 8, DFA | DFL | DFS, 4, NOCH),
    fd(79, K_BC, 8, BC4, 8, DFL | DFG, 1, NOCH),
    fd(80, K_BC, 8, BC4, 8, DFL | DFG, 1, NOCH),
    fd(81, K_BC, 8, BC4S, 8, DFL | DFG, 1, NOCH),
    fd(82, K_BC, 16, BC5, 8, DFL, 2, NOCH),
    fd(83, K_BC, 16, BC5, 8, DFL, 2, NOCH),
    fd(84, K_BC, 16, BC5S, 8, DFL, 2, NOCH),
    fd(85, K_INT, 2, 0, 6, 0, 3, [c(0, 5, 0, CR_B), c(5, 6, 0, CR_G), c(11, 5, 0, CR_R), C0]),
    fd(86, K_INT, 2, 0, 5, DFA, 4, [c(0, 5, 0, CR_B), c(5, 5, 0, CR_G), c(10, 5, 0, CR_R), c(15, 1, 0, CR_A)]),
    fd(87, K_BGRA8, 4, 0, 8, DFA, 4, NOCH),
    fd(88, K_BGRX8, 4, 0, 8, 0, 3, NOCH),
    fd(90, K_BGRA8, 4, 0, 8, DFA, 4, NOCH),
    fd(91, K_BGRA8, 4, 0, 8, DFA | DFS, 4, NOCH),
    fd(92, K_BGRX8, 4, 0, 8, 0, 3, NOCH),
    fd(93, K_BGRX8, 4, 0, 8, DFS, 3, NOCH),
    fd(94, K_BC, 16, BC6, 16, DFL | DFF, 3, NOCH),
    fd(95, K_BC, 16, BC6, 16, DFL | DFF, 3, NOCH),
    fd(96, K_BC, 16, BC6S, 16, DFL | DFF, 3, NOCH),
    fd(97, K_BC, 16, BC7, 8, DFA | DFL, 4, NOCH),
    fd(98, K_BC, 16, BC7, 8, DFA | DFL, 4, NOCH),
    fd(99, K_BC, 16, BC7, 8, DFA | DFL | DFS, 4, NOCH),
    fd(115, K_INT, 2, 0, 4, DFA, 4, [c(0, 4, 0, CR_B), c(4, 4, 0, CR_G), c(8, 4, 0, CR_R), c(12, 4, 0, CR_A)]),
    fd(191, K_INT, 2, 0, 4, DFA, 4, [c(0, 4, 0, CR_A), c(4, 4, 0, CR_B), c(8, 4, 0, CR_G), c(12, 4, 0, CR_R)]),
];

/// the parse context (DC_*)
#[derive(Default)]
struct Ctx {
    w: u32,     // width of mip 0 (one face)
    h: u32,
    d: u32,     // depth (volumes), else 1
    mips: u32,  // mip levels
    arr: u32,   // array size (cube maps: cubes)
    facem: u32, // faces present (cube maps: bits 0..5 = +X -X +Y -Y +Z -Z)
    off: u64,   // offset of the pixel data
    kind: u8,
    bpp: u8,
    aux: u8,
    txf: u8,
    bits: u8,
    nchm: u8,
    fl: u8,
    dxgi: u32,
    ch: [[u8; 4]; 4], // {shift, width, signed, role}
    // decode state
    cmask: [u32; 4],      // value masks
    lut: [Vec<u8>; 4],    // LUTs (empty: channel absent or wider than 16 bits)
    rowh: Vec<u16>,       // row scratch of the half kinds
    rowf: Vec<f32>,       // row scratch of the float kinds
    rowb: Vec<u8>,        // byte samples
}

#[inline]
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

// ------------------------------------------------------------------ header

// the descriptor of a DXGI format  [dds_find_fmt]
fn find_fmt(dxgi: u32) -> Option<&'static Fd> {
    FMTS.iter().find(|f| f.dxgi as u32 == dxgi)
}

// copy a descriptor into the context  [dds_use_fmt]
fn use_fmt(c: &mut Ctx, f: &Fd) {
    c.dxgi = f.dxgi as u32;
    c.kind = f.kind;
    c.bpp = f.bpp;
    c.aux = f.aux;
    c.bits = f.bits;
    c.fl = f.fl;
    c.nchm = f.nch;
    c.ch = f.ch;
}

// channel k of a legacy format from a bit mask (0 = absent); non-contiguous masks are
// refused  [dds_mask_ch]
fn mask_ch(c: &mut Ctx, k: usize, mask: u32, role: u8) -> Result<(), i32> {
    if mask == 0 {
        return Ok(());
    }
    let sh = mask.trailing_zeros();
    let v = mask >> sh;
    if v & v.wrapping_add(1) != 0 {
        return Err(ERR_UNSUPPORTED);
    }
    let wd = 32 - v.leading_zeros();
    c.ch[k & 3] = [sh as u8, wd as u8, 0, role];
    Ok(())
}

// pixel format of a header without DX10 (pf = DDS_PIXELFORMAT); DirectXTex order: RGB,
// LUMINANCE, ALPHA, BUMPDUDV, FOURCC  [dds_legacy]
fn legacy(c: &mut Ctx, pf: &[u8]) -> Result<(), i32> {
    let flags = u32_at(pf, 4);
    let bc = u32_at(pf, 12);
    let (mr, mg, mb, ma) = (u32_at(pf, 16), u32_at(pf, 20), u32_at(pf, 24), u32_at(pf, 28));
    if flags & (DDPF_RGB | DDPF_LUMINANCE | DDPF_ALPHA | DDPF_BUMPDUDV) == 0 {
        if flags & DDPF_FOURCC == 0 {
            return Err(ERR_UNSUPPORTED);
        }
        let fcc = u32_at(pf, 8);
        let Some(&(_, v)) = FOURCCS.iter().find(|e| e.0 == fcc) else { return Err(ERR_UNSUPPORTED) };
        let Some(f) = find_fmt(v & 0xFFFF) else { return Err(ERR_UNSUPPORTED) };
        use_fmt(c, f);
        c.txf |= (v >> 16) as u8;
        c.txf |= TXF_LEGACY;
        return Ok(());
    }
    if !matches!(bc, 8 | 16 | 24 | 32) {
        return Err(ERR_UNSUPPORTED);
    }
    c.bpp = (bc >> 3) as u8;
    c.kind = K_INT;
    c.fl = 0;
    c.nchm = 3;
    if flags & DDPF_RGB != 0 {
        if bc == 32 && rgb_fast(c, flags, mr, mg, mb, ma) {
            c.txf |= TXF_LEGACY;
            return Ok(());
        }
        c.kind = K_INT;
        c.dxgi = 0;
        mask_ch(c, 0, mr, CR_R)?;
        mask_ch(c, 1, mg, CR_G)?;
        mask_ch(c, 2, mb, CR_B)?;
        legacy_alpha(c, flags, ma)?;
    } else if flags & DDPF_LUMINANCE != 0 {
        c.nchm = 1;
        c.fl |= FDF_GRAY;
        mask_ch(c, 0, mr, CR_Y)?;
        legacy_alpha(c, flags, ma)?;
    } else if flags & DDPF_ALPHA != 0 {
        c.nchm = 0;
        c.fl |= FDF_GRAY;
        mask_ch(c, 3, ma, CR_A)?;
        if c.ch[3][1] == 0 {
            return Err(ERR_UNSUPPORTED);
        }
        c.fl |= FDF_ALPHA;
        c.nchm = 1;
        if bc == 8 {
            c.dxgi = 65;
        }
    } else {
        // BUMPDUDV: every mask signed
        mask_ch(c, 0, mr, CR_R)?;
        mask_ch(c, 1, mg, CR_G)?;
        mask_ch(c, 2, mb, CR_B)?;
        mask_ch(c, 3, ma, CR_A)?;
        for k in 0..4 {
            let wd = c.ch[k][1];
            if wd == 0 {
                continue;
            }
            if !(2..=16).contains(&wd) {
                return Err(ERR_UNSUPPORTED); // a 1-bit signed field has no range
            }
            c.ch[k][2] = 1;
        }
        if c.ch[3][1] != 0 {
            c.fl |= FDF_ALPHA;
            c.nchm = 4;
        }
    }
    // IM_BITS = widest channel (at most 16); at least one channel
    let wmax = c.ch.iter().map(|x| x[1]).max().unwrap_or(0);
    if wmax == 0 {
        return Err(ERR_UNSUPPORTED);
    }
    if wmax > 8 {
        c.fl |= FDF_HDR;
    }
    c.bits = wmax.min(16);
    c.txf |= TXF_LEGACY;
    Ok(())
}

// the alpha mask of DDPF_ALPHAPIXELS  [.lg_alpha]
fn legacy_alpha(c: &mut Ctx, flags: u32, ma: u32) -> Result<(), i32> {
    if flags & DDPF_ALPHAPIXELS != 0 {
        mask_ch(c, 3, ma, CR_A)?;
        if c.ch[3][1] != 0 {
            c.fl |= FDF_ALPHA;
            c.nchm += 1;
        }
    }
    Ok(())
}

// 32-bit RGB fast paths 8:8:8(:8); false = generic masks  [.lg_rgbfast]
fn rgb_fast(c: &mut Ctx, flags: u32, mr: u32, mg: u32, mb: u32, ma: u32) -> bool {
    let a = if flags & DDPF_ALPHAPIXELS != 0 { ma } else { 0 };
    if mr == 0x00FF_0000 {
        if mg != 0xFF00 || mb != 0xFF {
            return false;
        }
        c.kind = K_BGRX8;
        c.dxgi = 88;
        if a != 0 {
            if a != 0xFF00_0000 {
                return false;
            }
            c.kind = K_BGRA8;
            c.dxgi = 87;
        }
    } else {
        if mr != 0xFF || mg != 0xFF00 || mb != 0x00FF_0000 {
            return false;
        }
        c.kind = K_RGBX8;
        if a != 0 {
            if a != 0xFF00_0000 {
                return false;
            }
            c.kind = K_RGBA8;
            c.dxgi = 28;
        }
    }
    c.bits = 8;
    if a != 0 {
        c.fl |= FDF_ALPHA;
        c.nchm = 4;
    }
    true
}

// header scan into c; with ERR_UNSUPPORTED the size / mip / layout fields are valid
// [dds_parse]
fn parse(d: &[u8], c: &mut Ctx) -> i32 {
    *c = Ctx::default();
    if d.len() < 4 {
        return ERR_TRUNC;
    }
    if u32_at(d, 0) != DDS_MAGIC {
        return ERR_HDR;
    }
    if d.len() < 128 {
        return ERR_TRUNC;
    }
    if u32_at(d, 4) != 124 || u32_at(d, 76) != 32 {
        return ERR_HDR;
    }
    c.w = u32_at(d, 16);
    c.h = u32_at(d, 12);
    c.d = 1;
    c.arr = 1;
    c.facem = 1;
    c.off = 128;
    let caps2 = u32_at(d, 112);
    let r13;
    if u32_at(d, 84) == FCC_DX10 && u32_at(d, 80) & DDPF_FOURCC != 0 {
        if d.len() < 148 {
            return ERR_TRUNC;
        }
        c.off = 148;
        let arr = u32_at(d, 140);
        if arr == 0 {
            return ERR_HDR;
        }
        c.arr = arr;
        let mut array = false;
        match u32_at(d, 132) {
            2 => {
                if c.h != 1 {
                    return ERR_HDR;
                }
                array = true;
            }
            3 => {
                if u32_at(d, 136) & 4 != 0 {
                    c.facem = 63;
                    c.txf |= TXF_CUBE | TXF_CROSS;
                } else {
                    array = true;
                }
            }
            4 => {
                if arr != 1 {
                    return ERR_HDR;
                }
                c.d = u32_at(d, 24).max(1);
                c.txf |= TXF_VOLUME;
            }
            _ => return ERR_HDR,
        }
        if array && c.arr > 1 {
            c.txf |= TXF_ARRAY;
        }
        if u32_at(d, 144) & 7 == 2 {
            c.txf |= TXF_PREMUL;
        }
        let f = u32_at(d, 128);
        match if f > 0xFFFF { None } else { find_fmt(f) } {
            Some(fd) => {
                use_fmt(c, fd);
                r13 = 0;
            }
            None => {
                c.dxgi = f;
                r13 = ERR_UNSUPPORTED;
            }
        }
    } else {
        if caps2 & DDSCAPS2_CUBEMAP != 0 {
            let faces = (caps2 >> 10) & 63;
            if faces == 0 {
                return ERR_HDR;
            }
            c.facem = faces;
            c.txf |= TXF_CUBE | TXF_CROSS;
        } else if caps2 & DDSCAPS2_VOLUME != 0 {
            c.d = u32_at(d, 24).max(1);
            c.txf |= TXF_VOLUME;
        }
        r13 = match legacy(c, &d[76..108]) {
            Ok(()) => 0,
            Err(e) => e,
        };
    }
    // sizes: w, h 1..65535, depth <= DDS_MAXDEPTH; mip count clamped
    if c.w.wrapping_sub(1) > IMG_MAXDIM - 1 || c.h.wrapping_sub(1) > IMG_MAXDIM - 1 {
        return ERR_DIMS;
    }
    if c.d > DDS_MAXDEPTH {
        return ERR_HDR;
    }
    let mx = c.w.max(c.h).max(c.d);
    let full = 32 - mx.leading_zeros(); // levels of a full chain
    c.mips = u32_at(d, 28).max(1).min(full);
    r13
}

// ImgMeta fields from a parsed header  [dds_meta]
fn meta_of(c: &Ctx, m: &mut ImgMeta) {
    let (mut w, mut h) = (c.w, c.h);
    if c.txf & TXF_CROSS != 0 {
        w = w.wrapping_shl(2); // 4 x 3 cross
        h = h.wrapping_mul(3);
    }
    m.w = w;
    m.h = h;
    m.frames = c.mips;
    m.bits = c.bits;
    m.channels = c.nchm;
    m.dxgi = c.dxgi as u16;
    m.texfl = c.txf;
    m.texn = if c.txf & TXF_VOLUME != 0 { c.d } else { c.arr };
    m.model = if c.fl & FDF_GRAY != 0 { IMM_GRAY } else { IMM_RGB };
    let mut f = 0;
    if c.fl & FDF_ALPHA != 0 {
        f |= IMF_ALPHA;
    }
    if c.fl & FDF_SRGB != 0 {
        f |= IMF_SRGB;
    }
    if c.fl & FDF_FLOAT != 0 {
        f |= IMF_FLOAT;
    }
    if c.fl & FDF_HDR != 0 {
        f |= IMF_HDR;
    }
    if c.fl & FDF_LOSSY == 0 && c.kind != 0 {
        f |= IMF_LOSSLESS;
    }
    if c.txf & TXF_PREMUL != 0 {
        f |= IMF_SRC_PREMUL;
    }
    m.flags |= f;
}

// bytes of mip l of one face / element (all its slices), its width and height  [dds_level]
fn level(c: &Ctx, l: u32) -> (u64, u32, u32) {
    let w = (c.w >> l).max(1);
    let h = (c.h >> l).max(1);
    let d = (c.d >> l).max(1);
    let (mut bw, mut bh) = (w, h);
    if c.kind == K_BC {
        bw = (bw + 3) >> 2;
        bh = (bh + 3) >> 2;
    }
    (bw as u64 * bh as u64 * c.bpp as u64 * d as u64, w, h)
}

// ------------------------------------------------------------------ probe / decode

/// recognised (meta: size, depth, model, alpha, mips): the header is valid (the pixel
/// format may still be unsupported)  [dds_probe]
pub fn probe(data: &[u8], meta: &mut ImgMeta) -> bool {
    let mut c = Ctx::default();
    let r = parse(data, &mut c);
    if r == 0 || r == ERR_UNSUPPORTED {
        meta_of(&c, meta);
        true
    } else {
        false
    }
}

// CHECK_SPAN: off .. off + len inside the data
#[inline]
fn span_ok(off: u64, len: u64, end: u64) -> bool {
    end >= off && end - off >= len
}

/// decode mip dec.frame (see the module header)  [dds_decode]
pub fn decode(data: &[u8], sf: &mut Surface, dec: &ImgDec, meta: &mut ImgMeta) -> i32 {
    let mut c = Ctx::default();
    let r = parse(data, &mut c);
    if r != 0 && r != ERR_UNSUPPORTED {
        return r;
    }
    meta_of(&c, meta);
    if r != 0 {
        return r;
    }
    if dec.flags & IMGD_PROBE != 0 {
        return 0;
    }
    // frame = mip level
    let fr = dec.frame;
    if fr >= c.mips {
        return ERR_LIMIT;
    }
    // chain size, offset of the mip, its size
    let (mut chain, mut mipoff) = (0u64, 0u64);
    let (mut lb, mut wl, mut hl) = (0u64, 0u32, 0u32);
    for l in 0..c.mips {
        if l == fr {
            mipoff = chain;
        }
        let (b, w, h) = level(&c, l);
        if l == fr {
            (lb, wl, hl) = (b, w, h);
        }
        chain = chain.wrapping_add(b);
    }
    // the decoded bytes of one face: a volume needs slice 0 only
    if c.txf & TXF_VOLUME != 0 {
        lb /= (c.d >> fr).max(1) as u64;
    }
    // output size limits, then every face present in the file -- before allocating
    let (mut ow, mut oh) = (wl, hl);
    if c.txf & TXF_CROSS != 0 {
        ow = wl.wrapping_shl(2);
        oh = hl.wrapping_mul(3);
    }
    let r = dims_check(ow, oh, dec);
    if r != 0 {
        return r;
    }
    let len = data.len() as u64;
    let base = c.off;
    let face_off = |nth: u64| nth.wrapping_mul(chain).wrapping_add(mipoff).wrapping_add(base);
    let mut nth = 0u64;
    for face in 0..6 {
        if c.facem >> face & 1 != 0 {
            if !span_ok(face_off(nth), lb, len) {
                return ERR_TRUNC;
            }
            nth += 1;
        }
    }
    *sf = match surf_alloc(ow, oh, FMT_ARGB, dec) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let r = setup(&mut c, dec);
    if r != 0 {
        *sf = Surface::default();
        return r;
    }
    let mut nth = 0u64;
    for face in 0..6 {
        if c.facem >> face & 1 != 0 {
            let o = face_off(nth);
            if !span_ok(o, lb, len) {
                *sf = Surface::default();
                return ERR_TRUNC;
            }
            let (mut rx, mut ry) = (0, 0);
            if c.txf & TXF_CROSS != 0 {
                let cell = CROSS[face & 7];
                rx = cell[0] as u32 * wl;
                ry = cell[1] as u32 * hl;
            }
            let r = region(&mut c, &data[o as usize..(o + lb) as usize], sf, rx, ry, wl, hl, dec);
            if r != 0 {
                *sf = Surface::default();
                return r;
            }
            nth += 1;
        }
        if c.txf & TXF_CROSS == 0 {
            break;
        }
    }
    if c.txf & TXF_PREMUL != 0 {
        premul_fix(sf);
    }
    0
}

// premultiplied data: clamp every colour to its alpha, FMT_PARGB  [dds_premul_fix]
fn premul_fix(sf: &mut Surface) {
    let (w, st) = (sf.w as usize, sf.stride as usize);
    for y in 0..sf.h as usize {
        for p in sf.px[y * st..y * st + w * 4].chunks_exact_mut(4) {
            let a = p[3];
            p[0] = p[0].min(a);
            p[1] = p[1].min(a);
            p[2] = p[2].min(a);
        }
    }
    sf.fmt = FMT_PARGB;
}

// ------------------------------------------------------------------ pixel conversion

// LUTs of K_INT channels (<= 16 bits) and the row scratch of the float kinds (width of mip
// 0 is the largest row) -> 0 / ERR_LIMIT  [dds_setup]
fn setup(c: &mut Ctx, dec: &ImgDec) -> i32 {
    if c.kind == K_INT {
        // total LUT bytes: 2^width per channel of width 1..16
        let total: u32 = c.ch.iter().filter(|x| (1..=16).contains(&x[1])).map(|x| 1u32 << x[1]).sum();
        if total != 0 {
            let r = mem_check(total as u64, dec);
            if r != 0 {
                return r;
            }
        }
        for k in 0..4 {
            let wd = c.ch[k][1] as u32;
            c.cmask[k] = 0;
            c.lut[k] = Vec::new();
            if wd == 0 {
                continue;
            }
            c.cmask[k] = ((1u64 << wd) - 1) as u32;
            if wd > 16 {
                continue;
            }
            let m = c.cmask[k];
            let signed = c.ch[k][2] != 0;
            let mut t = vec![0u8; m as usize + 1];
            for (raw, o) in t.iter_mut().enumerate() {
                let raw = raw as u32;
                *o = if !signed {
                    // unsigned: (510 v + M) / 2M
                    ((raw * 510 + m) / (2 * m)) as u8
                } else {
                    // signed: v = sext(raw, width), M = 2^(w-1) - 1, v' = max(v, -M),
                    // (510 (v' + M) + 2M) / 4M
                    let s = 32 - wd;
                    let v = ((raw << s) as i32) >> s;
                    let mh = (1i32 << (wd - 1)) - 1;
                    let v = v.max(-mh) + mh;
                    ((v as u32 * 510 + 2 * mh as u32) / (4 * mh as u32)) as u8
                };
            }
            c.lut[k] = t;
        }
    } else if (K_F16..=K_E5).contains(&c.kind) {
        // row scratch: 4 bytes of samples + 12 bytes of floats per pixel
        let r = mem_check((c.w as u64) * 16 + 64, dec);
        if r != 0 {
            return r;
        }
        let n = c.w as usize * 4;
        c.rowb = vec![0u8; n];
        if c.kind == K_F16 || c.kind == K_R11 {
            c.rowh = vec![0u16; n];
        } else {
            c.rowf = vec![0f32; n];
        }
    }
    0
}

// decode rw x rh pixels from src (already checked) into sf at (rx, ry)  [dds_region]
#[allow(clippy::too_many_arguments)]
fn region(c: &mut Ctx, src: &[u8], sf: &mut Surface, rx: u32, ry: u32, rw: u32, rh: u32, dec: &ImgDec) -> i32 {
    let st = sf.stride as usize;
    let o = ry as usize * st + rx as usize * 4;
    if c.kind == K_BC {
        return dec::decode_to(src, c.aux as u32, &mut sf.px[o..], st, rw, rh, dec.cancel);
    }
    let pitch = c.bpp as usize * rw as usize; // source pitch (< 2^20)
    for y in 0..rh as usize {
        if dec.cancel.is_some_and(|f| f.load(Ordering::Relaxed) != 0) {
            return ERR_CANCEL;
        }
        let d = o + y * st;
        row(c, &src[y * pitch..(y + 1) * pitch], &mut sf.px[d..d + rw as usize * 4], rw as usize);
    }
    0
}

#[inline(always)]
fn put(dst: &mut [u8], i: usize, v: u32) {
    dst[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
}

// convert n source pixels to straight BGRA  [dds_row]
fn row(c: &mut Ctx, src: &[u8], dst: &mut [u8], n: usize) {
    match c.kind {
        K_BGRA8 => dst.copy_from_slice(&src[..n * 4]),
        K_BGRX8 => {
            for i in 0..n {
                put(dst, i, u32_at(src, i * 4) | 0xFF00_0000);
            }
        }
        K_RGBA8 => {
            for i in 0..n {
                let v = u32_at(src, i * 4);
                put(dst, i, (v & 0xFF00_FF00) | (v >> 16 & 0xFF) | (v << 16 & 0x00FF_0000));
            }
        }
        K_RGBX8 => {
            for i in 0..n {
                let v = u32_at(src, i * 4);
                put(dst, i, (v & 0x0000_FF00) | (v >> 16 & 0xFF) | (v << 16 & 0x00FF_0000) | 0xFF00_0000);
            }
        }
        K_F16 => {
            let k = c.aux as usize * n;
            for (h, s) in c.rowh[..k].iter_mut().zip(src.chunks_exact(2)) {
                *h = u16::from_le_bytes([s[0], s[1]]);
            }
            row_half_to_u8(&mut c.rowb[..k], &c.rowh[..k]);
            map(c, dst, n);
        }
        K_F32 => {
            let k = c.aux as usize * n;
            for (f, s) in c.rowf[..k].iter_mut().zip(src.chunks_exact(4)) {
                *f = f32::from_bits(u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
            }
            row_f32_to_u8(&mut c.rowb[..k], &c.rowf[..k]);
            map(c, dst, n);
        }
        K_R11 => {
            // R 11 bits (5e6m), G 11 bits, B 10 bits (5e5m) -> halves e << 10 | m << 4 / 5
            for i in 0..n {
                let v = u32_at(src, i * 4);
                c.rowh[3 * i] = ((v & 0x7FF) << 4) as u16;
                c.rowh[3 * i + 1] = ((v >> 11 & 0x7FF) << 4) as u16;
                c.rowh[3 * i + 2] = ((v >> 22) << 5) as u16;
            }
            row_half_to_u8(&mut c.rowb[..3 * n], &c.rowh[..3 * n]);
            map(c, dst, n);
        }
        K_E5 => {
            // R9G9B9E5: m * 2^(e - 24) as floats
            for i in 0..n {
                let v = u32_at(src, i * 4);
                let sc = f32::from_bits(((v >> 27) + 127 - 24) << 23);
                c.rowf[3 * i] = (v & 0x1FF) as f32 * sc;
                c.rowf[3 * i + 1] = (v >> 9 & 0x1FF) as f32 * sc;
                c.rowf[3 * i + 2] = (v >> 18 & 0x1FF) as f32 * sc;
            }
            row_f32_to_u8(&mut c.rowb[..3 * n], &c.rowf[..3 * n]);
            map(c, dst, n);
        }
        K_INT => row_int(c, src, dst, n),
        _ => {}
    }
}

// samples (bytes) -> BGRA: 1 grey, 2 (R, G, 0), 3 (R, G, B), 4 (R, G, B, A)  [.dr_map]
fn map(c: &Ctx, dst: &mut [u8], n: usize) {
    let k = c.aux as usize;
    for i in 0..n {
        let s = &c.rowb[i * k..];
        let v = if k == 1 {
            s[0] as u32 * 0x010101 | 0xFF00_0000
        } else {
            let mut v = (s[0] as u32) << 16 | (s[1] as u32) << 8 | 0xFF00_0000;
            if k >= 3 {
                v |= s[2] as u32;
            }
            if k >= 4 {
                v = (v & 0x00FF_FFFF) | (s[3] as u32) << 24;
            }
            v
        };
        put(dst, i, v);
    }
}

// integer fields through the LUTs; pixels of 1..8 bytes, little endian  [.dr_int]
fn row_int(c: &Ctx, src: &[u8], dst: &mut [u8], n: usize) {
    let bpp = c.bpp as usize;
    for i in 0..n {
        let s = &src[i * bpp..];
        let px: u64 = match bpp {
            4 => u32_at(s, 0) as u64,
            2 => u16::from_le_bytes([s[0], s[1]]) as u64,
            1 => s[0] as u64,
            3 => (u16::from_le_bytes([s[0], s[1]]) as u32 | (s[2] as u32) << 16) as u64,
            8 => u32_at(s, 0) as u64 | (u32_at(s, 4) as u64) << 32,
            _ => (u16::from_le_bytes([s[4], s[5]]) as u64) << 32 | u32_at(s, 0) as u64,
        };
        let mut out: u32 = 0xFF00_0000; // alpha 255, colours 0
        for k in 0..4 {
            let ch = c.ch[k];
            if ch[1] == 0 {
                continue;
            }
            let m = c.cmask[k] as u64;
            let v = (px >> ch[0]) & m;
            let val = if !c.lut[k].is_empty() {
                c.lut[k][v as usize] as u32
            } else {
                // > 16 bits (unsigned): (510 v + M) / 2M in 64 bits
                ((v * 510 + m) / (2 * m)) as u32
            };
            if ch[3] == CR_Y {
                out = (out & 0xFF00_0000) | val.wrapping_mul(0x010101);
            } else {
                let cl = ch[3] as u32 * 8;
                out = (out & !(0xFFu32 << cl)) | val << cl;
            }
        }
        put(dst, i, out);
    }
}
