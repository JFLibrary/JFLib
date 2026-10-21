// artistic -- effect group "artistic" (Künstlerisch): ink_sketch, oil_painting, pencil_sketch
//                                                                        [src/fx/fx_artistic.s]
//
//   FXTAB            the group table
//
//   oil_painting   brush size b (1..8), coarseness L (3..255): paint.net 3's oil painting: the
//                  (2b+1)^2 square window bucketed by intensity FastScale(I, L); the fullest
//                  bucket (lowest index on ties) gives the pixel: alpha = sum A / n, colour =
//                  sum (c A) / sum A
//   pencil_sketch  tip t (1..20), colour range c: D = desaturate(invert(BrightnessContrast(
//                  GaussianBlur(src, t), c, -c))); dst = ColorDodge(desaturate(src), D)
//   ink_sketch     outline k (0..99), colouring c: Glow (radius 6, b = c = -(c - 50) * 2)
//                  screened over the source, then a 5x5 edge kernel (centre 24, others -1) on
//                  the premultiplied source darkens it: d = min(255, I (k + 5) >> 6)

use gfx::blendmode::{bm_row, bm_row_inplace, BM_COLORDODGE, BM_OPAC_SHIFT, BM_SCREEN};

use crate::conv::*;
use crate::lochist::*;
use crate::*;

fn oil_px(o: &Oil, levels: u32) -> u32 {
    let (mut best, mut bc) = (0usize, 0u32);
    let mut b = 0u32;
    while b <= levels && b <= 255 {
        let c = o.cnt[b as usize];
        if c > bc {
            bc = c;
            best = b as usize;
        }
        b += 1;
    }
    if bc == 0 {
        return 0;
    }
    let s = &o.sum[best];
    let sa = s[3];
    if sa == 0 {
        return 0;
    }
    (sa / bc) << 24 | (s[2] / sa) << 16 | (s[1] / sa) << 8 | s[0] / sa
}

fn oil_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let b = pint(a, 0, 1, 8);
    let l = pint(a, 1, 3, 255);
    lh_run_oil(a, dst, b, LHS_SQUARE, l, |o, _, _| oil_px(o, l as u32))
}

fn pencil_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let t = pint(a, 0, 1, 20);
    let c = pint(a, 1, -20, 20);
    blurbac(a, dst, t, c, -c, |dst, y, x0, row| {
        let n = (row.len() / 4) as i32;
        for i in 0..n as usize {
            put(row, i, px(row, i) ^ 0x00FF_FFFF);
        }
        desat_inplace(row);
        let s = &a.src_row(y)[x0 as usize * 4..(x0 + n) as usize * 4];
        let d = dst_px(dst, y, x0, n);
        desat_row(d, s);
        bm_row_inplace(d, row, BM_COLORDODGE | 255 << BM_OPAC_SHIFT);
    })
}

/// premultiplied colour round(c a / 255) per channel ((c a + 128) 257 >> 16)
#[inline(always)]
fn pm(p: u32) -> [i32; 3] {
    let a = p >> 24;
    core::array::from_fn(|c| ((((p >> (8 * c) & 255) * a + 128) * 257) >> 16) as i32)
}

fn ink_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let c = pint(a, 1, 0, 100);
    let bc = -((c - 50) * 2);
    let k = (pint(a, 0, 0, 99) + 5) as u32;
    let (w, h) = (a.src.w, a.src.h);
    let mut colsum: alloc::vec::Vec<[i32; 3]> = alloc::vec::Vec::new();
    blurbac(a, dst, 6, bc, bc, |dst, y, x0, row| {
        let n = (row.len() / 4) as i32;
        let s = &a.src_row(y)[x0 as usize * 4..(x0 + n) as usize * 4];
        let d = dst_px(dst, y, x0, n);
        bm_row(d, s, row, BM_SCREEN | 255 << BM_OPAC_SHIFT);
        let rows: [&[u8]; 5] = core::array::from_fn(|r| a.src_row((y + r as i32 - 2).clamp(0, h - 1)));
        // column sums of the 5 x 5 window (the sum is an integer: any order is exact)
        let nc = n as usize + 4;
        if colsum.len() < nc {
            colsum.resize(nc, [0i32; 3]);
        }
        for (j, cs) in colsum.iter_mut().take(nc).enumerate() {
            let xc = (x0 - 2 + j as i32).clamp(0, w - 1) as usize;
            let mut t = [0i32; 3];
            for rr in &rows {
                let q = pm(px(rr, xc));
                for ch in 0..3 {
                    t[ch] += q[ch];
                }
            }
            *cs = t;
        }
        for i in 0..n as usize {
            let x = x0 + i as i32;
            let q = pm(px(rows[2], x as usize));
            let mut sum = [0i32; 3];
            for ch in 0..3 {
                sum[ch] = q[ch] * 25 - (colsum[i][ch] + colsum[i + 1][ch] + colsum[i + 2][ch] + colsum[i + 3][ch] + colsum[i + 4][ch]);
            }
            let e: [u32; 3] = core::array::from_fn(|ch| sum[ch].clamp(0, 255) as u32);
            let i8 = (e[0] * 7471 + e[1] * 38470 + e[2] * 19595) >> 16;
            let dd = ((i8 * k) >> 6).min(255);
            let inv = 255 - dd;
            let g = px(d, i);
            let mut o = g & 0xFF00_0000;
            for sh in [0, 8, 16] {
                o |= ((((g >> sh & 255) * inv + 128) * 257) >> 16) << sh;
            }
            put(d, i, o);
        }
    })
}

static P_INK: [FxProp; 2] = [
    FxProp::new(FP_INT, 0, 0, 0, "ink_outline", 0.0, 99.0, 50.0),
    FxProp::new(FP_INT, 0, 1, 0, "coloring", 0.0, 100.0, 50.0),
];
static P_OIL: [FxProp; 2] = [
    FxProp::new(FP_INT, 0, 0, 0, "brush_size", 1.0, 8.0, 3.0),
    FxProp::new(FP_INT, 0, 1, 0, "coarseness", 3.0, 255.0, 50.0),
];
static P_PENCIL: [FxProp; 2] = [
    FxProp::new(FP_INT, 0, 0, 0, "pencil_tip_size", 1.0, 20.0, 2.0),
    FxProp::new(FP_INT, 0, 1, 0, "color_range", -20.0, 20.0, 0.0),
];

const fn desc(id: u32, name: &'static str, render: RenderFn, props: &'static [FxProp]) -> FxDesc {
    FxDesc { id, flags: 0, name, prepare: None, render, props, category: FXC_ARTISTIC, user: 0 }
}

/// the group table  [fxtab_artistic]
pub static FXTAB: [FxDesc; 3] = [
    desc(FX_INK_SKETCH, "ink_sketch", ink_render, &P_INK),
    desc(FX_OIL_PAINTING, "oil_painting", oil_render, &P_OIL),
    desc(FX_PENCIL_SKETCH, "pencil_sketch", pencil_render, &P_PENCIL),
];
