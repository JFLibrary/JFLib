// yuv -- YUV -> RGB conversion for HEIC / AVIF (u16 sample planes)       [src/img/yuv.s]
//
//   coef(yp) -> YuvCoef / ERR_UNSUPPORTED      the fixed-point gains (yuv_coef)
//   to_argb(yp, sf, dx, dy) -> 0 / ERR_*       YUV image into sf at (dx, dy) (clipped), opaque
//   alpha(yp, sf, dx, dy) -> 0                 replace the alpha bytes with yp's Y plane
//   prem_clamp(sf)                             limit R, G, B of every pixel to its alpha
//   argb_to_yuv() -> ERR_UNSUPPORTED           the inverse (not implemented, as in the assembly)
//
// Conversion (tools/heic_check.py has the same model): the range maps Y to Y' = (Y - yoff) *
// 255 / ymax and chroma to C' = (C - coff) * 255 / cmax (full range: yoff 0, ymax = cmax =
// 2^bits - 1, coff 2^(bits - 1); limited: yoff 16 << (bits - 8), ymax 219 << (bits - 8), coff
// 128 << (bits - 8), cmax 224 << (bits - 8); identity uses the luma range for all three).
// YCbCr: R = Y' + (2 - 2Kr) Cr', G = Y' - 2Kb(1 - Kb) / Kg Cb' - 2Kr(1 - Kr) / Kg Cr', B = Y'
// + (2 - 2Kb) Cb'; YCgCo: R = Y' - Cg' + Co', G = Y' + Cg', B = Y' - Cg' - Co'; identity:
// G = Y', B = U', R = V'.  The gains are computed in double precision (fixed order of
// operations) and rounded once: Ky = floor(255 / ymax * 2^32 + 0.5), Kc = +-floor(255 / cmax
// * coef * 2^28 + 0.5).
//
// Chroma upsampling (linear, weights summing to 4 per direction, so upsampled chroma carries
// a factor 16): horizontally co-sited (location types 0 / 2 / 4: even x = 4 c[x/2], odd x =
// 2 c[(x-1)/2] + 2 c[(x+1)/2]) or centred (1 / 3 / 5: 3 c[k] + c[k -+ 1], k = x >> 1);
// vertically centred (types 0 / 1 / 4 / 5) or co-sited (2 / 3), the same way; neighbours
// clamp at the plane edge.  YPF_NEAREST: c[x / subw][y / subh].
//
// The assembly converts bands of 32 rows on the worker pool; img_core has no thread pool
// (no plat dependency), so the bands run one after the other here -- every band depends only
// on its rows, the bytes are the same.

use alloc::vec;

use alloc::vec::Vec;
use rt::sync::SpinLock;

use crate::{Surface, ERR_UNSUPPORTED};

pub const YPF_NEAREST: u32 = 1; // nearest-neighbour chroma upsampling (default: linear)

const UM_FULL: u32 = 0; // upsampling modes per direction
const UM_NEAR: u32 = 1;
const UM_COS: u32 = 2;
const UM_CEN: u32 = 3;

const BAND: u32 = 32; // rows per band (worker item of the assembly)

/// a YUV image (YP_*): u16 samples (also for 8-bit content), strides in samples; the chroma
/// planes are ceil(w / subw) x ceil(h / subh) samples
#[derive(Clone, Copy, Default)]
pub struct YuvPlanes<'a> {
    pub pl: [&'a [u16]; 3], // Y, U (Cb / Cg / B), V (Cr / Co / R); U empty: monochrome
    pub st: [u32; 3],
    pub w: u32,
    pub h: u32,
    pub subw: u32, // horizontal chroma subsampling 1 / 2
    pub subh: u32, // vertical chroma subsampling 1 / 2
    pub bits: u32, // luma bit depth 8..16
    pub bitsc: u32, // chroma bit depth 8..16
    pub cicp: u32, // primaries | transfer << 8 | matrix << 16 | full range << 24
    pub flags: u32, // YPF_*
    pub chloc: u32, // chroma sample location type 0..5 (HEVC VUI / H.273; 0 default)
}

/// the gains of coef (YK): out = clamp((y' * Ky + u' * Ku + v' * Kv + 2^31) >> 32) with
/// y' = Y - yoff (Ky Q32) and u' / v' = 16 * upsampled chroma - coff (Q28)
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct YuvCoef {
    pub r: [i64; 3], // Ky, Ku, Kv of red
    pub g: [i64; 3],
    pub b: [i64; 3],
    pub yoff: i64, // luma offset
    pub coff: i64, // 16 * chroma offset
    pub ky: i64,   // Q32 luma gain (monochrome: R = G = B)
}

// Kr, Kb per matrix_coefficients 0..15 (0 / 8 / 12 / 13 handled elsewhere; unknown: BT.601)
static MTAB: [[f64; 2]; 16] = [
    [0.299, 0.114],   // 0 identity (unused)
    [0.2126, 0.0722], // 1 BT.709
    [0.299, 0.114],   // 2 unspecified
    [0.299, 0.114],   // 3 reserved
    [0.30, 0.11],     // 4 FCC
    [0.299, 0.114],   // 5 BT.470BG
    [0.299, 0.114],   // 6 SMPTE 170M
    [0.212, 0.087],   // 7 SMPTE 240M
    [0.299, 0.114],   // 8 YCgCo (unused)
    [0.2627, 0.0593], // 9 BT.2020 NCL
    [0.2627, 0.0593], // 10 BT.2020 CL (as NCL)
    [0.299, 0.114],   // 11 SMPTE 2085
    [0.299, 0.114],   // 12 (primaries)
    [0.299, 0.114],   // 13 (primaries)
    [0.299, 0.114],   // 14 ICtCp
    [0.299, 0.114],   // 15
];

// Kr, Kb derived from colour_primaries 0..22 (H.273 equations; 0 = not tabulated)
static PTAB: [[f64; 2]; 23] = [
    [0.0, 0.0],                                   // 0
    [0.2126390058715103, 0.07219231536073373],    // 1 BT.709
    [0.0, 0.0],                                   // 2
    [0.0, 0.0],                                   // 3
    [0.2989666181247899, 0.1146121717422266],     // 4 BT.470M
    [0.22200430999823087, 0.07134092407648639],   // 5 BT.470BG
    [0.2123763607050675, 0.08656378236920956],    // 6 SMPTE 170M
    [0.2123763607050675, 0.08656378236920956],    // 7 SMPTE 240M
    [0.2535853634337347, 0.06807886086929262],    // 8 film
    [0.262700212011267, 0.05930171646986194],     // 9 BT.2020
    [0.0, 0.0],                                   // 10 XYZ
    [0.20949167791273057, 0.06891306792622581],   // 11 SMPTE 431
    [0.22897456406974873, 0.079286914093745],     // 12 SMPTE 432 (Display P3)
    [0.0, 0.0],                                   // 13
    [0.0, 0.0],                                   // 14
    [0.0, 0.0],                                   // 15
    [0.0, 0.0],                                   // 16
    [0.0, 0.0],                                   // 17
    [0.0, 0.0],                                   // 18
    [0.0, 0.0],                                   // 19
    [0.0, 0.0],                                   // 20
    [0.0, 0.0],                                   // 21
    [0.23175054567210923, 0.09599868152322844],   // 22 EBU 3213
];

const C2P32: f64 = 4294967296.0;
const C2P28: f64 = 268435456.0;

/// yv_round28: floor(gain * 2^28 + 0.5)
fn round28(g: f64) -> i64 {
    rt::fx::cvttd64(g * C2P28 + 0.5)
}

/// yv_krkb: Kr, Kb of a YCbCr matrix (12 / 13: from the primaries; unknown: BT.601)
fn krkb(matrix: u32, prim: u32) -> (f64, f64) {
    if matrix < 16 && matrix != 12 && matrix != 13 {
        return (MTAB[matrix as usize][0], MTAB[matrix as usize][1]);
    }
    if matrix < 16 && prim < 23 {
        let [kr, kb] = PTAB[prim as usize];
        if kr != 0.0 {
            return (kr, kb);
        }
    }
    (MTAB[6][0], MTAB[6][1])
}

/// yuv_coef: the fixed-point gains of yp's matrix, range and bit depths
pub fn coef(yp: &YuvPlanes) -> Result<YuvCoef, i32> {
    let mut yk = YuvCoef::default();
    if yp.bits.wrapping_sub(8) > 8 || yp.bitsc.wrapping_sub(8) > 8 {
        return Err(ERR_UNSUPPORTED);
    }
    let matrix = (yp.cicp >> 16) & 255;
    let full = (yp.cicp >> 24) & 255 != 0;
    // luma range
    let (ymax, yoff) = if full { ((1u32 << yp.bits) - 1, 0u32) } else { (219 << (yp.bits - 8), 16 << (yp.bits - 8)) };
    yk.yoff = yoff as i64;
    let ky = rt::fx::cvttd64(255.0 / ymax as i32 as f64 * C2P32 + 0.5);
    yk.ky = ky;
    // chroma range
    let (cmax, coff) = if full {
        let t = 1u32 << yp.bitsc;
        (t - 1, if matrix != 0 { t >> 1 } else { 0 }) // identity: luma range
    } else {
        let s = yp.bitsc - 8;
        if matrix != 0 { (224 << s, 128 << s) } else { (219 << s, 16 << s) }
    };
    yk.coff = (coff << 4) as i64;
    let cmul = 255.0 / cmax as i32 as f64;
    if matrix == 0 {
        let v = round28(cmul);
        yk.r[2] = v; // R = V'
        yk.g[0] = ky; // G = Y'
        yk.b[1] = v; // B = U'
    } else if matrix == 8 {
        let v = round28(cmul);
        yk.r = [ky, -v, v]; // R = Y' - Cg' + Co'
        yk.g = [ky, v, 0]; // G = Y' + Cg'
        yk.b = [ky, -v, -v]; // B = Y' - Cg' - Co'
    } else {
        let (kr, kb) = krkb(matrix, yp.cicp & 255);
        yk.r[2] = round28(cmul * (2.0 - (kr + kr)));
        yk.b[1] = round28(cmul * (2.0 - (kb + kb)));
        let kg = (1.0 - kr) - kb;
        yk.g[1] = -round28(cmul * ((kb + kb) * (1.0 - kb) / kg));
        yk.g[2] = -round28(cmul * ((kr + kr) * (1.0 - kr) / kg));
        yk.r[0] = ky;
        yk.g[0] = ky;
        yk.b[0] = ky;
    }
    Ok(yk)
}

/// clamp of a Q32 sum to 0..255 (cmp 255 unsigned; negative -> 0, large -> 255)
#[inline(always)]
fn clamp8(v: i64) -> u32 {
    if v as u64 > 255 { !(v >> 63) as u32 & 255 } else { v as u32 }
}

/// the geometry of a conversion shared by its bands
struct Geo {
    cols: u32,
    cw: u32,
    ch: u32,
    hm: u32,
    vm: u32,
}

/// ya_band: rows y0 .. y1 of the conversion; dst = the destination row of y0 (stride bytes
/// per row, 4 bytes per pixel)
fn band(yp: &YuvPlanes, yk: &YuvCoef, g: &Geo, y0: u32, y1: u32, dst: &mut [u8], stride: usize) {
    let cols = g.cols as usize;
    if yp.pl[1].is_empty() {
        // monochrome: R = G = B = Y'
        for y in y0..y1 {
            let yr = &yp.pl[0][(y * yp.st[0]) as usize..];
            let d = &mut dst[(y - y0) as usize * stride..];
            for x in 0..cols {
                let v = clamp8(((yr[x] as i64 - yk.yoff).wrapping_mul(yk.ky)).wrapping_add(0x8000_0000) >> 32);
                let p = 0xFF00_0000 | v * 0x01_0101;
                d[x * 4..x * 4 + 4].copy_from_slice(&p.to_le_bytes());
            }
        }
        return;
    }
    let w = yp.w as usize;
    let cw = g.cw as usize;
    let mut tu = vec![0u32; w];
    let mut tv = vec![0u32; w];
    let mut vt = vec![0u32; cw];
    for y in y0..y1 {
        // vertical taps
        let (mut j0, mut j1, mut w0, mut w1) = (y, y, 4u32, 0u32);
        if g.vm != UM_FULL {
            let k = y >> 1;
            j0 = k;
            j1 = k;
            if g.vm == UM_COS {
                if y & 1 != 0 {
                    j1 = (k + 1).min(g.ch - 1); // odd row between k and k + 1
                    w0 = 2;
                    w1 = 2;
                }
            } else if g.vm == UM_CEN {
                w0 = 3;
                w1 = 1;
                j1 = if y & 1 != 0 { (k + 1).min(g.ch - 1) } else { k.saturating_sub(1) };
            }
        }
        for c in 1..=2 {
            // vertical filter of component c into vt
            let pl = yp.pl[c];
            let r0 = &pl[(j0 * yp.st[c]) as usize..];
            let r1 = &pl[(j1 * yp.st[c]) as usize..];
            for x in 0..cw {
                vt[x] = (r0[x] as u32 * w0).wrapping_add(r1[x] as u32 * w1);
            }
            // horizontal filter vt -> tu / tv
            let t = if c == 1 { &mut tu } else { &mut tv };
            let last = cw - 1;
            match g.hm {
                UM_NEAR => {
                    for x in 0..cols {
                        t[x] = vt[x >> 1] << 2;
                    }
                }
                UM_COS => {
                    for x in 0..cols {
                        let k = x >> 1;
                        t[x] = if x & 1 == 0 { vt[k] << 2 } else { (vt[k] + vt[(k + 1).min(last)]) * 2 };
                    }
                }
                UM_CEN => {
                    for x in 0..cols {
                        let k = x >> 1;
                        let n = if x & 1 != 0 { (k + 1).min(last) } else { k.saturating_sub(1) };
                        t[x] = vt[k] * 3 + vt[n];
                    }
                }
                _ => {
                    for x in 0..cols {
                        t[x] = vt[x] << 2;
                    }
                }
            }
        }
        // pixels
        let yr = &yp.pl[0][(y * yp.st[0]) as usize..];
        let d = &mut dst[(y - y0) as usize * stride..];
        let rnd = 0x8000_0000i64;
        for x in 0..cols {
            let ly = yr[x] as i64 - yk.yoff;
            let u = tu[x] as i64 - yk.coff;
            let v = tv[x] as i64 - yk.coff;
            let ch = |k: &[i64; 3]| {
                clamp8(
                    ly.wrapping_mul(k[0]).wrapping_add(u.wrapping_mul(k[1])).wrapping_add(v.wrapping_mul(k[2])).wrapping_add(rnd)
                        >> 32,
                )
            };
            let p = 0xFF00_0000 | ch(&yk.r) << 16 | ch(&yk.g) << 8 | ch(&yk.b);
            d[x * 4..x * 4 + 4].copy_from_slice(&p.to_le_bytes());
        }
    }
}

/// the clipped size of yp at (dx, dy) in sf: (columns, rows), None = nothing to do
fn clip(yp: &YuvPlanes, sf: &Surface, dx: u32, dy: u32) -> Option<(u32, u32)> {
    let cols = sf.w.wrapping_sub(dx as i32);
    if cols <= 0 {
        return None;
    }
    let rows = sf.h.wrapping_sub(dy as i32);
    if rows <= 0 {
        return None;
    }
    Some(((cols as u32).min(yp.w), (rows as u32).min(yp.h)))
}

/// yuv_to_argb: convert the YUV image into sf at (dx, dy) (clipped to sf) as opaque BGRA
/// (alpha 255; alpha replaces it) -> 0 / ERR_UNSUPPORTED
pub fn to_argb(yp: &YuvPlanes, sf: &mut Surface, dx: u32, dy: u32) -> i32 {
    let yk = match coef(yp) {
        Ok(k) => k,
        Err(e) => return e,
    };
    let Some((cols, rows)) = clip(yp, sf, dx, dy) else { return 0 };
    if cols == 0 || rows == 0 {
        return 0;
    }
    let mut g = Geo { cols, cw: 0, ch: 0, hm: UM_FULL, vm: UM_FULL };
    if !yp.pl[1].is_empty() {
        // chroma geometry and upsampling modes
        g.cw = (yp.w + yp.subw - 1) / yp.subw;
        g.ch = (yp.h + yp.subh - 1) / yp.subh;
        let loc = yp.chloc;
        g.hm = if loc & 1 != 0 { UM_CEN } else { UM_COS }; // horizontal: even types co-sited
        g.vm = if loc.wrapping_sub(2) <= 1 { UM_COS } else { UM_CEN }; // vertical: 2 / 3 co-sited
        if yp.flags & YPF_NEAREST != 0 {
            g.hm = UM_NEAR;
            g.vm = UM_NEAR;
        }
        if yp.subw == 1 {
            g.hm = UM_FULL;
        }
        if yp.subh == 1 {
            g.vm = UM_FULL;
        }
    }
    let stride = sf.stride as usize;
    let base = stride * dy as usize + dx as usize * 4;
    let nb = rows.div_ceil(BAND) as usize;
    match crate::parallel() {
        Some(par) if nb > 1 => {
            // bands of BAND rows on the worker pool, each into its own rows (ya_band)
            let bands: Vec<SpinLock<&mut [u8]>> =
                sf.px[base..].chunks_mut(BAND as usize * stride).take(nb).map(SpinLock::new).collect();
            par(nb, &|k| {
                let y0 = k as u32 * BAND;
                let mut d = bands[k].lock();
                band(yp, &yk, &g, y0, (y0 + BAND).min(rows), &mut d, stride);
            });
        }
        _ => {
            let mut y0 = 0;
            while y0 < rows {
                let y1 = (y0 + BAND).min(rows);
                band(yp, &yk, &g, y0, y1, &mut sf.px[base + y0 as usize * stride..], stride);
                y0 = y1;
            }
        }
    }
    0
}

/// yuv_alpha: replace the alpha bytes of sf at (dx, dy) (clipped) with yp's Y plane (bit
/// depth bits, always full range: a = floor(Y * 255 / (2^bits - 1) + 1/2) in Q32) -> 0
pub fn alpha(yp: &YuvPlanes, sf: &mut Surface, dx: u32, dy: u32) -> i32 {
    let Some((cols, rows)) = clip(yp, sf, dx, dy) else { return 0 };
    let stride = sf.stride as usize;
    let max = ((1u32 << yp.bits) - 1) as i32 as f64;
    let ka = rt::fx::cvttd64(255.0 / max * C2P32 + 0.5);
    for r in 0..rows as usize {
        let yr = &yp.pl[0][r * yp.st[0] as usize..];
        let d = &mut sf.px[(dy as usize + r) * stride + dx as usize * 4..];
        for x in 0..cols as usize {
            let a = ((yr[x] as i64).wrapping_mul(ka).wrapping_add(0x8000_0000) as u64 >> 32) as u32;
            d[x * 4 + 3] = a.min(255) as u8;
        }
    }
    0
}

/// yuv_prem_clamp: limit R, G, B of every pixel to its alpha (premultiplied sources: the
/// result is valid FMT_PARGB)
pub fn prem_clamp(sf: &mut Surface) {
    let (w, stride) = (sf.w.max(0) as usize, sf.stride as usize);
    for y in 0..sf.h.max(0) as usize {
        for p in sf.px[y * stride..y * stride + w * 4].chunks_exact_mut(4) {
            let a = p[3];
            p[0] = p[0].min(a);
            p[1] = p[1].min(a);
            p[2] = p[2].min(a);
        }
    }
}

/// argb_to_yuv: the inverse, for future encoders (not implemented yet)
pub fn argb_to_yuv() -> i32 {
    ERR_UNSUPPORTED
}
