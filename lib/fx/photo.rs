// photo -- effect group "photo" (Foto): glow, red_eye_removal, sharpen, soften_portrait,
// straighten, vignette                                                      [src/fx/fx_photo.s]
//
//   FXTAB            the group table
//
//   glow             paint.net 3's Glow: G = BrightnessContrast(GaussianBlur(src, r)), dst =
//                    Screen(lhs = src, rhs = G)
//   soften_portrait  G = BrightnessContrast(GaussianBlur(src, 3 softness), lighting,
//                    -lighting / 2); L = warm grey of src; dst = Overlay(L, G), alpha = src
//   sharpen          local median m of the disc (radius = amount): clamp((3 s - m) >> 1)
//   red_eye_removal  paint.net 3's RedEyeRemove op on every ROI pixel
//   straighten       rotation about the image centre (img_core::xform bicubic); zoom scales
//                    by cos|a| + max(w / h, h / w) sin|a| (edges clamped), else transparent
//                    corners
//   vignette         smoothstep darkening towards a colour in linear light

use alloc::vec;

use gfx::blendmode::{bm_row, bm_row_inplace, BM_OPAC_SHIFT, BM_OVERLAY, BM_SCREEN};
use gfx::{Surface, FMT_ARGB};
use img_core::ops::Band;
use img_core::resample::{from_work, to_work, RS_BICUBIC, RS_GAMMA};
use img_core::xform::{rotate, scale, translate, Affine, Matrix, XF_CLAMP, XF_EDGE_SHIFT, XF_TRANSPARENT};
use rt::fx::{cvt, cvtt, cvttd, maxsd, minsd};

use crate::blur::pct4;
use crate::conv::*;
use crate::lochist::*;
use crate::*;

const DEG2RAD: f64 = 0.017453292519943295;
// straighten: rows per resampling call (the asm: 8; img_core::xform allocates its scratch per call)
const ST_ROWS: i32 = 64;

fn glow_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let r = pint(a, 0, 1, 20);
    let b = pint(a, 1, -100, 100);
    let c = pint(a, 2, -100, 100);
    blurbac(a, dst, r, b, c, |dst, y, x0, row| {
        let n = (row.len() / 4) as i32;
        let s = &a.src_row(y)[x0 as usize * 4..(x0 + n) as usize * 4];
        bm_row(dst_px(dst, y, x0, n), s, row, BM_SCREEN | 255 << BM_OPAC_SHIFT);
    })
}

fn soften_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let soft = pint(a, 0, 0, 10) * 3;
    let light = pint(a, 1, -20, 20);
    let wf = pint(a, 2, 0, 20) as f32 / 100.0;
    let (red, blue) = (1.0 + wf, 1.0 - wf);
    blurbac(a, dst, soft, light, -light / 2, |dst, y, x0, row| {
        let n = (row.len() / 4) as i32;
        let s = &a.src_row(y)[x0 as usize * 4..(x0 + n) as usize * 4];
        let d = dst_px(dst, y, x0, n);
        for i in 0..n as usize {
            let p = px(s, i);
            let iv = crate::lut::luma(p);
            let fi = iv as f32;
            let r = cvtt(fi * red).min(255) as u32;
            let b = cvtt(fi * blue).max(0) as u32;
            put(d, i, p & 0xFF00_0000 | iv << 8 | r << 16 | b);
        }
        bm_row_inplace(d, row, BM_OVERLAY | 255 << BM_OPAC_SHIFT);
        for i in 0..n as usize {
            put(d, i, px(d, i) & 0x00FF_FFFF | px(s, i) & 0xFF00_0000);
        }
    })
}

fn sharpen_px(h: &Hist) -> u32 {
    let m = pct4(h, 50);
    let s = h.src;
    let mut o = 0u32;
    for sh in [0, 8, 16, 24] {
        let v = (((s >> sh & 255) * 3) as i32 - (m >> sh & 255) as i32) >> 1;
        o |= (v.clamp(0, 255) as u32) << sh;
    }
    o
}

fn sharpen_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let r = pint(a, 0, 1, 20);
    lh_run(a, dst, r, LHS_DISC, true, |h, _, _| sharpen_px(h))
}

fn redeye_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(rect) = roi(a) else {
        return 0;
    };
    let t = pint(a, 0, 0, 100);
    let p = pint(a, 1, 0, 100) as f64 / 100.0;
    for y in rect[1]..rect[3] {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let s = a.src_row(y);
        let d = dst.row(y);
        for x in rect[0]..rect[2] {
            let c = px(s, x as usize);
            let mut o = c;
            let (b, g, r) = ((c & 255) as i32, (c >> 8 & 255) as i32, (c >> 16 & 255) as i32);
            if r - b.max(g) > t {
                let (rf, gf, bf) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
                let mn = minsd(minsd(rf, gf), bf);
                let mx = maxsd(maxsd(rf, gf), bf);
                if mx != 0.0 {
                    let delta = mx - mn;
                    if delta != 0.0 && cvttd(delta / mx * 255.0) > 100 {
                        let iv = (b as f64 * 0.114 + g as f64 * 0.587 + r as f64 * 0.299) / 255.0 * 255.0 * p;
                        o = c & 0xFF00_FFFF | (cvttd(iv) as u32 & 255) << 16;
                    }
                }
            }
            put(d, (x - a.x0) as usize, o);
        }
    }
    0
}

fn straighten_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(rect) = roi(a) else {
        return 0;
    };
    let zoom = pint(a, 1, 0, 1);
    let ang = minsd(maxsd(pdbl(a, 0), -45.0), 45.0) * DEG2RAD;
    let (sn, cs) = gfx::xform::sincos(f64::from_bits(ang.to_bits() & !(1u64 << 63)));
    let (w, h) = (a.src.w, a.src.h);
    let mut s = 1.0f64;
    if zoom != 0 {
        let (wd, hd) = (w as f64, h as f64);
        let r = maxsd(wd / hd, hd / wd);
        s = 1.0 / (r * sn + cs);
    }
    let cx = w as f64 * 0.5;
    let cy = h as f64 * 0.5;
    let mut m = Matrix::IDENTITY;
    translate(&mut m, cx, cy);
    rotate(&mut m, f64::from_bits(ang.to_bits() ^ (1u64 << 63)), 0.0, 0.0);
    scale(&mut m, s, s);
    translate(&mut m, f64::from_bits(cx.to_bits() ^ (1u64 << 63)), f64::from_bits(cy.to_bits() ^ (1u64 << 63)));
    let edge = if zoom != 0 { XF_CLAMP } else { XF_TRANSPARENT } << XF_EDGE_SHIFT;
    let meas = Surface { px: alloc::vec::Vec::new(), w, h, stride: 0, fmt: FMT_ARGB, flags: 0, opac: 255, blend: 0 };
    let xa = match Affine::prepare(a.src, &meas, &m, RS_BICUBIC, edge, Some(rect)) {
        Ok(x) => x,
        Err(e) => return e,
    };
    // perf-fx: angle 0 gives the identity matrix: xform's integer-translation path copies the
    // source pixels of the clip unchanged (same format, all inside) -> copy them straight into
    // dst like the asm, whose rs_affine_rows writes into the destination (no band + copy)
    if m.a == 1.0 && m.e == 1.0 && m.b == 0.0 && m.d == 0.0 && m.c == 0.0 && m.f == 0.0 && a.src.fmt == FMT_ARGB {
        return copy_roi(a, dst);
    }
    // perf-fx: a band of stride = the clip width: xform writes only the clip columns x0 .. x1
    // of each band row (row k from (x0 + k tw) pixels on), so the rows lie back to back from
    // x0 on and the buffer needs x0 + rows * tw pixels (not rows * x1)
    let (x0, tw) = (rect[0] as usize, (rect[2] - rect[0]) as usize);
    let mut buf = vec![0u8; (x0 + tw * ST_ROWS as usize) * 4];
    let mut y = rect[1];
    while y < rect[3] {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let y1 = (y + ST_ROWS).min(rect[3]);
        let mut band = Band { px: &mut buf, w, h, fmt: FMT_ARGB, stride: tw * 4, y0: y, y1 };
        let e = xa.rows(&mut band, y, y1);
        if e != 0 {
            return e;
        }
        for yy in y..y1 {
            let o = (x0 + (yy - y) as usize * tw) * 4;
            dst_px(dst, yy, rect[0], tw as i32).copy_from_slice(&buf[o..o + tw * 4]);
        }
        y = y1;
    }
    0
}

fn vignette_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(rect) = roi(a) else {
        return 0;
    };
    let (w, h) = (a.src.w, a.src.h);
    let cx = (pdbl(a, 0) + 1.0) * (w as f64 * 0.5);
    let cy = (pdbl(a, 1) + 1.0) * (h as f64 * 0.5);
    let hw = w as f64 * 0.5;
    let hh = h as f64 * 0.5;
    let dg = rt::fx::sqrtd(hw * hw + hh * hh);
    let rad = minsd(maxsd(pdbl(a, 2), 0.1), 4.0);
    let inv = 1.0 / ((rad + rad) * dg);
    let dens = minsd(maxsd(pdbl(a, 3), 0.0), 1.0);
    let col = a.params.0[4] as u32;
    let amt = dens * (col >> 24) as f64 / 255.0;
    let mut cw = [0i16; 4];
    to_work(&mut cw, &(col | 0xFF00_0000).to_le_bytes(), 1, FMT_ARGB, RS_GAMMA);
    let n = (rect[2] - rect[0]) as usize;
    let mut wrow = vec![0i16; n * 4 + 8];
    for y in rect[1]..rect[3] {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        load_row(&mut wrow, a.src, y, rect[0], rect[2], FXL_GAMMA);
        let dy = y as f64 + 0.5 - cy;
        let dy2 = dy * dy;
        for i in 0..n {
            let x = rect[0] + i as i32;
            let dx = x as f64 + 0.5 - cx;
            let rho = rt::fx::sqrtd(dx * dx + dy2) * inv;
            let t = minsd(maxsd((rho - 0.4) / 0.7, 0.0), 1.0);
            let ss = t * t * (3.0 - (t + t));
            let v = (ss * amt) as f32;
            let p = &mut wrow[i * 4..i * 4 + 4];
            let pa = p[3] as u16 as u32;
            for c in 0..4 {
                let q = if c == 3 { pa } else { (cw[c] as u16 as u32 * pa + 8160) / 16320 };
                let pc = p[c] as u16 as i32;
                let r = pc as f32 + (q as i32 - pc) as f32 * v;
                p[c] = cvt(r).clamp(-32768, 32767) as i16;
            }
        }
        from_work(dst_px(dst, y, rect[0], n as i32), &wrow, n, FMT_ARGB, RS_GAMMA);
    }
    0
}

static P_GLOW: [FxProp; 3] = [
    FxProp::new(FP_INT, 0, 0, 0, "radius", 1.0, 20.0, 6.0),
    FxProp::new(FP_INT, 0, 1, 0, "brightness", -100.0, 100.0, 10.0),
    FxProp::new(FP_INT, 0, 2, 0, "contrast", -100.0, 100.0, 10.0),
];
static P_REDEYE: [FxProp; 2] = [
    FxProp::new(FP_INT, 0, 0, 0, "tolerance", 0.0, 100.0, 70.0),
    FxProp::new(FP_INT, 0, 1, 0, "saturation", 0.0, 100.0, 90.0),
];
static P_SHARPEN: [FxProp; 1] = [FxProp::new(FP_INT, 0, 0, 0, "amount", 1.0, 20.0, 10.0)];
static P_SOFTEN: [FxProp; 3] = [
    FxProp::new(FP_INT, 0, 0, 0, "softness", 0.0, 10.0, 5.0),
    FxProp::new(FP_INT, 0, 1, 0, "lighting", -20.0, 20.0, 0.0),
    FxProp::new(FP_INT, 0, 2, 0, "warmth", 0.0, 20.0, 10.0),
];
static P_STRAIGHTEN: [FxProp; 2] = [
    FxProp::new(FP_ANGLE, 2, 0, 0, "angle", -45.0, 45.0, 0.0),
    FxProp::new(FP_BOOL, 0, 1, 0, "zoom", 0.0, 1.0, 1.0),
];
static P_VIGNETTE: [FxProp; 4] = [
    FxProp::pan(0, "offset", 2, -1.0, 1.0, 0.0, 0.0),
    FxProp::new(FP_DEC, 2, 2, 0, "radius", 0.1, 4.0, 0.5),
    FxProp::new(FP_DEC, 2, 3, 0, "density", 0.0, 1.0, 1.0),
    FxProp::color(4, "color", 0xFF00_0000),
];

const fn desc(id: u32, name: &'static str, render: RenderFn, props: &'static [FxProp]) -> FxDesc {
    FxDesc { id, flags: 0, name, prepare: None, render, props, category: FXC_PHOTO, user: 0 }
}

/// the group table  [fxtab_photo]
pub static FXTAB: [FxDesc; 6] = [
    desc(FX_GLOW, "glow", glow_render, &P_GLOW),
    desc(FX_RED_EYE, "red_eye_removal", redeye_render, &P_REDEYE),
    desc(FX_SHARPEN, "sharpen", sharpen_render, &P_SHARPEN),
    desc(FX_SOFTEN_PORTRAIT, "soften_portrait", soften_render, &P_SOFTEN),
    desc(FX_STRAIGHTEN, "straighten", straighten_render, &P_STRAIGHTEN),
    desc(FX_VIGNETTE, "vignette", vignette_render, &P_VIGNETTE),
];
