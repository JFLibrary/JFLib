// write -- DirectDraw Surface writer                                    [src/img/dds.s CB]
//
//   encode(sf, opt, out) -> 0 / ERR_*      dds_encode
//
// Writes what DirectXTex (paint.net's DDSFileTypePlus) writes for the opt.ddsfmt format: a
// legacy header when a FourCC / mask layout exists (DXT1 / DXT3 / DXT5, BC4U / BC4S / BC5U /
// BC5S, A8R8G8B8, X8R8G8B8, A8B8G8R8, A1R5G5B5, A4R4G4B4, R5G6B5, L8, A8, R8G8B8), else the
// DX10 header (BC6H UF16 / SF16, BC7, R8G8, R8, and every sRGB twin with IOF_SRGB); header
// flags CAPS | HEIGHT | WIDTH | PIXELFORMAT | MIPMAPCOUNT and LINEARSIZE (top level bytes)
// for block formats, PITCH (row bytes) otherwise; depth 1; caps TEXTURE (| COMPLEX | MIPMAP
// with mips, | COMPLEX for cube maps), caps2 CUBEMAP | all faces; DX10: dimension 2D, misc
// TEXTURECUBE for cube maps, array size 1.
// * Pixels: straight BGRA rows of any source (get_row).  BC formats through enc::encode_to
//   (block rows on the thread pool; opt.effort 1..3 fast, 0 / 4..6 normal, 7..9 slow;
//   IOF_DDS_UNIFORM; BC1 transparency threshold opt.athresh, 0 = 128); BC4 / L8 / R8 store
//   the intensity (7471 B + 38470 G + 19595 R) >> 16, BC5 / R8G8 red and green, A8 the alpha;
//   4 / 5 / 6-bit channels round(c (2^n - 1) / 255), 1-bit alpha a >= 128; X channels 255.
//   Rows are packed.
// * IOF_MIPMAPS: levels down to 1 x 1, each resized from the previous one (opt.mipfilter:
//   0 / FANT / BOX area average, LINEAR bilinear, CUBIC bicubic, NEAREST, LANCZOS;
//   premultiplied filtering, RS_GAMMA (linear light) with IOF_SRGB), row bands on the pool.
// * IOF_CUBE_CROSS: the image is a 4 x 3 horizontal cross (w = 4 s, h = 3 s, else ERR_DIMS)
//   with the faces at the cells the reader produces; written as a cube map (six faces, each
//   with its mip chain).
// Errors: ERR_UNSUPPORTED (opt.ddsfmt unknown), ERR_DIMS, ERR_NOMEM, ERR_CANCEL (polled per
// level / block row / 64 rows).

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, Ordering};

use gfx::{FMT_A8, FMT_ARGB, FMT_PARGB, Surface};
use img_core::meta::{get_row, surf_convert};
use img_core::ops::bands;
use img_core::resample::{RS_BICUBIC, RS_BILINEAR, RS_FANT, RS_GAMMA, RS_LANCZOS3, RS_NEAREST, Resize};
use rt::sync::SpinLock;

use crate::enc::encode_to;
use crate::read::{CROSS, DDPF_FOURCC, DDPF_LUMINANCE, DDS_MAGIC, FCC_DX10};
use crate::*;

const WFK_BC: u8 = 1;
const WFK_RAW: u8 = 2;
// raw converters
const WR_BGRA8: u8 = 0;
const WR_BGRX8: u8 = 1;
const WR_RGBA8: u8 = 2;
const WR_555: u8 = 3;
const WR_4444: u8 = 4;
const WR_565: u8 = 5;
const WR_LUM: u8 = 6;
const WR_A8: u8 = 7;
const WR_RG: u8 = 8;
const WR_BGR8: u8 = 9;

/// a writer format entry: kind, BCF_* / converter, bytes per block / pixel, legacy pixel
/// format (WPF index, 0 = DX10 only), DXGI format (linear), sRGB twin (0 = none)
struct Wf {
    kind: u8,
    code: u8,
    bytes: u8,
    pf: u8,
    dxgi: u16,
    dxgis: u16,
}

const fn wf(kind: u8, code: u8, bytes: u8, pf: u8, dxgi: u16, dxgis: u16) -> Wf {
    Wf { kind, code, bytes, pf, dxgi, dxgis }
}

// per DDSF_* 1..21
static WFMTS: [Wf; 21] = [
    wf(WFK_BC, BCF_BC1 as u8, 8, 1, 71, 72),
    wf(WFK_BC, BCF_BC2 as u8, 16, 2, 74, 75),
    wf(WFK_BC, BCF_BC3 as u8, 16, 3, 77, 78),
    wf(WFK_BC, BCF_BC4 as u8, 8, 4, 80, 0),
    wf(WFK_BC, BCF_BC5 as u8, 16, 6, 83, 0),
    wf(WFK_BC, BCF_BC6H as u8, 16, 0, 95, 0),
    wf(WFK_BC, BCF_BC7 as u8, 16, 0, 98, 99),
    wf(WFK_RAW, WR_BGRA8, 4, 8, 87, 91),
    wf(WFK_RAW, WR_BGRX8, 4, 9, 88, 93),
    wf(WFK_RAW, WR_RGBA8, 4, 10, 28, 29),
    wf(WFK_RAW, WR_555, 2, 11, 86, 0),
    wf(WFK_RAW, WR_4444, 2, 12, 115, 0),
    wf(WFK_RAW, WR_565, 2, 13, 85, 0),
    wf(WFK_RAW, WR_LUM, 1, 14, 61, 0),
    wf(WFK_RAW, WR_A8, 1, 15, 65, 0),
    wf(WFK_RAW, WR_RG, 2, 0, 49, 0),
    wf(WFK_BC, BCF_BC4S as u8, 8, 5, 81, 0),
    wf(WFK_BC, BCF_BC5S as u8, 16, 7, 84, 0),
    wf(WFK_BC, BCF_BC6HS as u8, 16, 0, 96, 0),
    wf(WFK_RAW, WR_LUM, 1, 0, 61, 0),
    wf(WFK_RAW, WR_BGR8, 3, 16, 0, 0),
];

// legacy pixel formats (DDS_PIXELFORMAT after the size): flags, FourCC, bit count, R, G, B, A
// masks (DirectXTex's DDSPF_* values)
static WPF: [[u32; 7]; 17] = [
    [0, 0, 0, 0, 0, 0, 0],
    [DDPF_FOURCC, 0x3154_5844, 0, 0, 0, 0, 0],                // 1 DXT1
    [DDPF_FOURCC, 0x3354_5844, 0, 0, 0, 0, 0],                // 2 DXT3
    [DDPF_FOURCC, 0x3554_5844, 0, 0, 0, 0, 0],                // 3 DXT5
    [DDPF_FOURCC, 0x5534_4342, 0, 0, 0, 0, 0],                // 4 BC4U
    [DDPF_FOURCC, 0x5334_4342, 0, 0, 0, 0, 0],                // 5 BC4S
    [DDPF_FOURCC, 0x5535_4342, 0, 0, 0, 0, 0],                // 6 BC5U
    [DDPF_FOURCC, 0x5335_4342, 0, 0, 0, 0, 0],                // 7 BC5S
    [0x41, 0, 32, 0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000],      // 8 A8R8G8B8
    [0x40, 0, 32, 0xFF_0000, 0xFF00, 0xFF, 0],                // 9 X8R8G8B8
    [0x41, 0, 32, 0xFF, 0xFF00, 0xFF_0000, 0xFF00_0000],      // 10 A8B8G8R8
    [0x41, 0, 16, 0x7C00, 0x3E0, 0x1F, 0x8000],               // 11 A1R5G5B5
    [0x41, 0, 16, 0xF00, 0xF0, 0xF, 0xF000],                  // 12 A4R4G4B4
    [0x40, 0, 16, 0xF800, 0x7E0, 0x1F, 0],                    // 13 R5G6B5
    [DDPF_LUMINANCE, 0, 8, 0xFF, 0, 0, 0],                    // 14 L8
    [2, 0, 8, 0, 0, 0, 0xFF],                                 // 15 A8 (DDPF_ALPHA)
    [0x40, 0, 24, 0xFF_0000, 0xFF00, 0xFF, 0],                // 16 R8G8B8
];

// opt.mipfilter 0..MIPF_LANCZOS -> resample filter
static MIPF_RS: [u32; 7] = [RS_FANT, RS_FANT, RS_BILINEAR, RS_BICUBIC, RS_NEAREST, RS_FANT, RS_LANCZOS3];

/// the writer state (DE_*)
struct W<'a, 'o> {
    opt: &'a ImgOpt<'o>,
    ent: &'static Wf,
    buf: Vec<u8>,
    pos: usize,
    fw: u32,
    fh: u32,
    levels: u32,
    nfaces: u32,
    cfg: u32,
    srgb: bool,
    hdr: usize,
}

/// the DDS file of sf (see the module header)  [dds_encode]
pub fn encode(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    // format
    let mut f = opt.ddsfmt;
    if f == 0 {
        f = DDSF_BC1;
    }
    if f.wrapping_sub(1) > DDSF_COUNT - 2 {
        return ERR_UNSUPPORTED;
    }
    let ent = &WFMTS[(f - 1) as usize];
    let srgb = opt.flags & IOF_SRGB != 0 && ent.dxgis != 0;
    // BC cfg: level from opt.effort, metric, BC1 threshold
    let mut cfg = match opt.effort {
        0 => BCQ_NORMAL,
        1..=3 => BCQ_FAST,
        4..=6 => BCQ_NORMAL,
        _ => BCQ_SLOW,
    };
    if opt.flags & IOF_DDS_UNIFORM != 0 {
        cfg |= BCQ_UNIFORM;
    }
    cfg |= opt.athresh.min(256) << BCQ_ATHR_SHIFT;
    // mip filter
    let filt = MIPF_RS[if opt.mipfilter > MIPF_LANCZOS { 0 } else { opt.mipfilter as usize }];
    let rsfl = if opt.flags & IOF_SRGB != 0 { RS_GAMMA } else { 0 };
    // faces
    let (mut fw, mut fh) = (sf.w as u32, sf.h as u32);
    let mut nfaces = 1;
    if opt.flags & IOF_CUBE_CROSS != 0 {
        if fw & 3 != 0 {
            return ERR_DIMS;
        }
        let s = fw >> 2; // s = w / 4
        if s.wrapping_mul(3) != fh {
            return ERR_DIMS; // h must be 3 s
        }
        fw = s;
        fh = s;
        nfaces = 6;
    }
    if fw as i32 <= 0 || fh as i32 <= 0 || fw > IMG_MAXDIM || fh > IMG_MAXDIM {
        return ERR_DIMS;
    }
    // mip levels
    let levels = if opt.flags & IOF_MIPMAPS != 0 { 32 - fw.max(fh).leading_zeros() } else { 1 };
    // header bytes: 128, DX10 148
    let hdr = if !srgb && ent.pf != 0 { 128 } else { 148 };
    // total = header + faces * sum of the level sizes
    let chain: u64 = (0..levels).map(|l| lsize(ent, fw, fh, l)).sum();
    let total = chain * nfaces as u64 + hdr as u64;
    // (zeroed pages from the allocator; an allocation failure ends the process instead of
    // ERR_NOMEM, as everywhere in the rebuild)
    let buf = vec![0u8; total as usize];
    let mut w = W { opt, ent, buf, pos: hdr, fw, fh, levels, nfaces, cfg, srgb, hdr };
    whdr(&mut w);
    for face in 0..nfaces {
        // face view: the whole image or the cross cell of the face
        let cell;
        let view: &Surface = if nfaces == 6 {
            let [col, row] = CROSS[(face & 7) as usize];
            cell = crop(sf, col as u32 * fw, row as u32 * fh, fw, fh);
            &cell
        } else {
            sf
        };
        // level 0 from the view
        let r = wlevel(&mut w, view);
        if r != 0 {
            return r;
        }
        if levels <= 1 {
            continue;
        }
        // mips: the previous level must be ARGB / PARGB for the resampler
        let conv;
        let mut first: &Surface = view;
        if view.fmt != FMT_PARGB && view.fmt != FMT_ARGB {
            conv = match surf_convert(view, FMT_ARGB) {
                Ok(s) => s,
                Err(e) => return e,
            };
            first = &conv;
        }
        let mut own: Option<Surface> = None; // the previous level once it is ours
        for l in 1..levels {
            if opt.cancelled() {
                return ERR_CANCEL;
            }
            let mut cur = Surface::new(((fw >> l).max(1)) as i32, ((fh >> l).max(1)) as i32, FMT_ARGB);
            let prev = own.as_ref().unwrap_or(first);
            let r = resize(prev, &mut cur, filt, rsfl);
            if r != 0 {
                return r;
            }
            let r = wlevel(&mut w, &cur);
            if r != 0 {
                return r;
            }
            own = Some(cur);
        }
    }
    *out = w.buf;
    0
}

// a copy of the w x h cell of sf at (x, y), same format  [the face view of dds_encode]
fn crop(sf: &Surface, x: u32, y: u32, w: u32, h: u32) -> Surface {
    let bpp = if sf.fmt == FMT_A8 { 1 } else { 4 };
    let mut d = Surface::new(w as i32, h as i32, sf.fmt);
    let (ss, ds) = (sf.stride as usize, d.stride as usize);
    let rb = w as usize * bpp;
    for r in 0..h as usize {
        let so = (y as usize + r) * ss + x as usize * bpp;
        d.px[r * ds..r * ds + rb].copy_from_slice(&sf.px[so..so + rb]);
    }
    d
}

// bytes of one face's level  [dds_lsize]
fn lsize(ent: &Wf, w: u32, h: u32, l: u32) -> u64 {
    let mut w = (w >> l).max(1);
    let mut h = (h >> l).max(1);
    if ent.kind == WFK_BC {
        w = (w + 3) >> 2;
        h = (h + 3) >> 2;
    }
    w as u64 * h as u64 * ent.bytes as u64
}

#[inline]
fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

// header (+ DX10 header) at the start of the output  [dds_whdr]
fn whdr(w: &mut W) {
    let ent = w.ent;
    let b = &mut w.buf[..w.hdr];
    b.fill(0);
    put32(b, 0, DDS_MAGIC);
    put32(b, 4, 124);
    let bc = ent.kind == WFK_BC;
    // CAPS HEIGHT WIDTH PIXELFORMAT MIPMAPCOUNT, LINEARSIZE or PITCH
    put32(b, 8, 0x1007 | 0x20000 | if bc { 0x80000 } else { 0x8 });
    put32(b, 12, w.fh);
    put32(b, 16, w.fw);
    // pitch (row bytes) or linear size (top level bytes)
    let pl = if bc { lsize(ent, w.fw, w.fh, 0).min(0xFFFF_FFFF) as u32 } else { w.fw.wrapping_mul(ent.bytes as u32) };
    put32(b, 20, pl);
    put32(b, 24, 1); // depth
    put32(b, 28, w.levels);
    put32(b, 76, 32);
    if w.hdr != 148 {
        let pf = &WPF[if ent.pf as usize > WPF.len() - 1 { 0 } else { ent.pf as usize }];
        for (k, &v) in pf.iter().enumerate() {
            put32(b, 80 + 4 * k, v);
        }
    } else {
        put32(b, 80, DDPF_FOURCC);
        put32(b, 84, FCC_DX10);
        put32(b, 128, if w.srgb { ent.dxgis } else { ent.dxgi } as u32);
        put32(b, 132, 3); // TEXTURE2D
        put32(b, 136, if w.nfaces == 6 { 4 } else { 0 }); // TEXTURECUBE
        put32(b, 140, 1); // array size
    }
    let mut caps = 0x1000; // TEXTURE
    if w.levels > 1 {
        caps |= 0x40_0008; // COMPLEX | MIPMAP
    }
    let mut caps2 = 0;
    if w.nfaces == 6 {
        caps |= 0x8; // COMPLEX
        caps2 = 0xFE00; // CUBEMAP | all faces
    }
    put32(b, 108, caps);
    put32(b, 112, caps2);
}

// one level of one face at the write position (advanced), in the format of the entry
// [dds_wlevel]
fn wlevel(w: &mut W, sf: &Surface) -> i32 {
    if w.opt.cancelled() {
        return ERR_CANCEL;
    }
    let ent = w.ent;
    let n = lsize(ent, sf.w as u32, sf.h as u32, 0) as usize;
    let dst = &mut w.buf[w.pos..w.pos + n];
    if ent.kind == WFK_BC {
        let r = encode_to(sf, ent.code as u32, w.cfg, dst, w.opt.cancel);
        if r != 0 {
            return r;
        }
    } else {
        // row buffer: w straight BGRA pixels
        let pw = sf.w as usize;
        let mut row = vec![0u8; pw * 4];
        let rb = ent.bytes as usize * pw; // packed row bytes
        for y in 0..sf.h as usize {
            if y & 63 == 0 && w.opt.cancelled() {
                return ERR_CANCEL;
            }
            get_row(sf, y, &mut row, FMT_ARGB);
            wrow(&mut dst[y * rb..(y + 1) * rb], &row, pw, ent.code);
        }
    }
    w.pos += n;
    0
}

// round(c (2^n - 1) / 255) ((x + 127) * 0x8081 >> 23 = x / 255, exact below 2^16)  [WRQ]
#[inline(always)]
fn q(c: u32, m: u32) -> u32 {
    ((c * m + 127) * 0x8081) >> 23
}

// n straight BGRA pixels -> packed pixels of converter conv (one loop per converter)
// [dds_wrow]
fn wrow(dst: &mut [u8], src: &[u8], n: usize, conv: u8) {
    let px = src[..n * 4].chunks_exact(4);
    match conv {
        WR_BGRA8 => dst[..n * 4].copy_from_slice(&src[..n * 4]),
        WR_BGRX8 => {
            for (d, p) in dst.chunks_exact_mut(4).zip(px) {
                d.copy_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        WR_RGBA8 => {
            for (d, p) in dst.chunks_exact_mut(4).zip(px) {
                d.copy_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
        WR_555 => {
            for (d, p) in dst.chunks_exact_mut(2).zip(px) {
                let (b, g, r, a) = (p[0] as u32, p[1] as u32, p[2] as u32, p[3] as u32);
                let mut x = q(b, 31) | q(g, 31) << 5 | q(r, 31) << 10;
                if a >= 128 {
                    x |= 0x8000;
                }
                d.copy_from_slice(&(x as u16).to_le_bytes());
            }
        }
        WR_4444 => {
            for (d, p) in dst.chunks_exact_mut(2).zip(px) {
                let (b, g, r, a) = (p[0] as u32, p[1] as u32, p[2] as u32, p[3] as u32);
                let x = q(b, 15) | q(g, 15) << 4 | q(r, 15) << 8 | q(a, 15) << 12;
                d.copy_from_slice(&(x as u16).to_le_bytes());
            }
        }
        WR_565 => {
            for (d, p) in dst.chunks_exact_mut(2).zip(px) {
                let (b, g, r) = (p[0] as u32, p[1] as u32, p[2] as u32);
                let x = q(b, 31) | q(r, 31) << 11 | q(g, 63) << 5;
                d.copy_from_slice(&(x as u16).to_le_bytes());
            }
        }
        WR_LUM => {
            for (d, p) in dst.iter_mut().zip(px) {
                *d = ((p[0] as u32 * 7471 + p[1] as u32 * 38470 + p[2] as u32 * 19595) >> 16) as u8;
            }
        }
        WR_A8 => {
            for (d, p) in dst.iter_mut().zip(px) {
                *d = p[3];
            }
        }
        WR_RG => {
            for (d, p) in dst.chunks_exact_mut(2).zip(px) {
                d.copy_from_slice(&[p[2], p[1]]);
            }
        }
        _ => {
            // BGR8: 3 bytes
            for (d, p) in dst.chunks_exact_mut(3).zip(px) {
                d.copy_from_slice(&p[..3]);
            }
        }
    }
}

// resize with the destination rows in bands on the thread pool  [dds_resize]
fn resize(src: &Surface, dst: &mut Surface, filter: u32, flags: u32) -> i32 {
    let c = match Resize::prepare(src, dst, filter, flags) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let nb = ((dst.h as u32 + 31) >> 5) as usize; // bands of 32 rows
    let bs: Vec<SpinLock<img_core::ops::Band>> = bands(dst, nb).into_iter().map(SpinLock::new).collect();
    let err = AtomicI32::new(0);
    let item = |i: usize, _: &plat::JobCtl| {
        let mut b = bs[i].lock();
        let (y0, y1) = (b.y0, b.y1);
        let r = c.rows(&mut b, y0, y1);
        if r != 0 {
            err.store(r, Ordering::Relaxed);
        }
    };
    let _ = plat::parallel_for(nb, &item);
    err.load(Ordering::Relaxed)
}
