// stylize -- Stilisieren: Edge Detect, Emboss, Outline, Relief           [src/fx/fx_stylize.s]
//
//   FXTAB            the group table
//
// Edge Detect, Emboss and Relief are paint.net 3.x ColorDifferenceEffect kernels: a 3 x 3
// weight matrix rotated by the angle (r = angle * 2 * pi / 360, dr = pi / 4):
//       cos(r + dr)  cos(r + 2dr)  cos(r + 3dr)
//       cos(r)       CENTER        cos(r + 4dr)
//       cos(r - dr)  cos(r - 2dr)  cos(r - 3dr)
// Weights are quantised to wq = floor(w * 16384 + 0.5) (integer sums, bit-exact, tile
// independent); taps outside the image count as 0.  Per channel v = trunc(sum wq * c / 16384)
// (towards zero) + OFFSET, clamped to 0..255; alpha = the centre pixel's alpha.
//   edge_detect  CENTER 0, OFFSET 128, channels B, G, R        (angle default 45)
//   relief       CENTER 1, OFFSET 0, channels B, G, R          (angle default 45)
//   emboss       CENTER 0, OFFSET 128, on the intensity, output B = G = R = v (angle default 0)
//   outline      paint.net 3.x OutlineEffect (hist::lochist): per channel over the circle of
//                radius `thickness`: skip empty bins, count up to area (100 - intensity) / 200
//                -> c1, on up to area (100 + intensity) / 200 -> c2, c = 255 - (c2 - c1);
//                alpha = a2 (the alpha scan skips on the BLUE histogram like paint.net)

use crate::hist::{lochist, LocHist};
use crate::lut::{cos, luma};
use simd::{Simd, Sse2};
use crate::*;

const CD_CHUNK: i32 = 256;
static STZ_DR: [i32; 9] = [1, 2, 3, 0, 0, 4, -1, -2, -3]; // tap angle = r + k dr (tap 4: centre)

/// the angle slot 0 in degrees, NaN -> 0, clamped to +-3600  [stz_angle]
fn angle(a: &FxArgs) -> f64 {
    let v = a.params.f(0);
    let v = if v.is_nan() { 0.0 } else { v };
    rt::fx::minsd(rt::fx::maxsd(v, -3600.0), 3600.0)
}

/// quantised weights, row major  [stz_weights]
fn weights(ang: f64, center: f64) -> [i32; 9] {
    let r = (ang + ang) * 3.141592653589793 / 360.0;
    let mut w = [0i32; 9];
    for (i, wv) in w.iter_mut().enumerate() {
        let v = if i == 4 {
            center
        } else if STZ_DR[i] == 0 {
            cos(r)
        } else {
            cos(STZ_DR[i] as f64 * 0.7853981633974483 + r)
        };
        *wv = rt::fx::cvttd(v * 16384.0 + 32768.5) - 32768;
    }
    w
}

/// acc + pmaddwd(pixel bytes widened to i32 lanes, {w, 0})
#[inline(always)]
fn tap(acc: <Sse2 as Simd>::V, p: u32, w: <Sse2 as Simd>::V) -> <Sse2 as Simd>::V {
    let x = Sse2;
    let z = x.zero();
    let v = x.unpacklo16(x.unpacklo8(x.from_i32(p as i32), z), z);
    x.add32(acc, x.madd16(v, w))
}

/// the 3 x 3 colour difference kernel over the ROI; mode bit 0 = +128 offset, bit 1 =
/// intensity input (emboss)  [cd_render]
fn cd_render(a: &FxArgs, dst: &mut Dst, w: &[i32; 9], mode: u32) -> i32 {
    let x = Sse2;
    let z = x.zero();
    // pmaddwd weight vectors {w, 0} (the low 16 bits of w)
    let wv: [<Sse2 as Simd>::V; 9] = core::array::from_fn(|i| x.splat32(w[i] as u16 as u32));
    let off = x.splat32(if mode & 1 != 0 { 128 } else { 0 });
    let c16383 = x.splat32(16383);
    let (sw, sh) = (a.src.w, a.src.h);
    let mut bufs = [[0u32; (CD_CHUNK + 2) as usize]; 3];
    for y in a.y0..a.y1 {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let mut xs = a.x0;
        while a.x1 - xs > 0 {
            let n = (a.x1 - xs).min(CD_CHUNK);
            // three padded source rows y-1, y, y+1: pixels xs-1 .. xs+n (0 outside the image)
            for (r, buf) in bufs.iter_mut().enumerate() {
                let sy = y + r as i32 - 1;
                let buf = &mut buf[..(n + 2) as usize];
                if (sy as u32) >= sh as u32 {
                    buf.fill(0);
                    continue;
                }
                let row = a.src_row(sy);
                let k0 = (1 - xs).max(0) as usize; // first k with x inside
                let k1 = ((sw - xs + 1).min(n + 2)).max(k0 as i32) as usize;
                buf[..k0].fill(0);
                buf[k1..].fill(0);
                let srcx = (xs - 1 + k0 as i32) as usize;
                let src = &row[srcx * 4..(srcx + k1 - k0) * 4];
                if mode & 2 != 0 {
                    for (b, c) in buf[k0..k1].iter_mut().zip(src.chunks_exact(4)) {
                        *b = luma(u32::from_le_bytes([c[0], c[1], c[2], c[3]])) * 0x010101;
                    }
                } else {
                    for (b, c) in buf[k0..k1].iter_mut().zip(src.chunks_exact(4)) {
                        *b = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                    }
                }
            }
            let srow = &a.src_row(y)[xs as usize * 4..(xs + n) as usize * 4];
            let d = &mut dst.row(y)[(xs - a.x0) as usize * 4..(xs - a.x0 + n) as usize * 4];
            let nn = n as usize + 2;
            let (b0, b1, b2) = (&bufs[0][..nn], &bufs[1][..nn], &bufs[2][..nn]);
            let rows = b0.windows(3).zip(b1.windows(3)).zip(b2.windows(3));
            for ((((t0, t1), t2), o), sp) in rows.zip(d.chunks_exact_mut(4)).zip(srow.chunks_exact(4)) {
                let mut acc = tap(z, t0[0], wv[0]);
                acc = tap(acc, t0[1], wv[1]);
                acc = tap(acc, t0[2], wv[2]);
                acc = tap(acc, t1[0], wv[3]);
                acc = tap(acc, t1[1], wv[4]);
                acc = tap(acc, t1[2], wv[5]);
                acc = tap(acc, t2[0], wv[6]);
                acc = tap(acc, t2[1], wv[7]);
                acc = tap(acc, t2[2], wv[8]);
                // trunc(acc / 16384) towards zero, + offset, saturated to 0..255
                let r = x.add32(acc, x.and(x.sra32::<31>(acc), c16383));
                let r = x.add32(x.sra32::<14>(r), off);
                let r = x.packs32(r, r);
                let v = x.to_i32(x.packus16(r, r)) as u32 & 0x00FF_FFFF;
                o.copy_from_slice(&(v | (sp[3] as u32) << 24).to_le_bytes());
            }
            xs += n;
        }
    }
    0
}

/// weights from the angle, then cd_render  [stz_run]
fn stz_run(a: &FxArgs, dst: &mut Dst, center: f64, mode: u32) -> i32 {
    let w = weights(angle(a), center);
    cd_render(a, dst, &w, mode)
}

fn edge_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    stz_run(a, dst, 0.0, 1)
}

fn relief_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    stz_run(a, dst, 1.0, 0)
}

fn emboss_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    stz_run(a, dst, 0.0, 3)
}

// ---------------------------------------------------------------- outline

/// c1 and c2 of one channel: hs / bs = bins / block sums for the skip of empty bins, hc / bc =
/// bins / block sums counted.  Same result as paint.net's bin-by-bin loops: whole 16-bin
/// blocks below bin 240 are skipped or added only when the loop would do so bin by bin
/// [OL_SCAN]
#[inline(always)]
fn ol_scan(hs: &[u32; 256], bs: &[u32; 16], hc: &[u32; 256], bc: &[u32; 16], min1: i32, min2: i32) -> (u32, u32) {
    let mut i = 0usize;
    while i < 240 && bs[i >> 4] == 0 {
        i += 16;
    }
    while i < 255 && hs[i] == 0 {
        i += 1;
    }
    let mut count = 0i32;
    while i < 255 && count < min1 {
        if i & 15 == 0 && i < 240 {
            let t = (bc[i >> 4] as i32).wrapping_add(count);
            if t < min1 {
                count = t;
                i += 16;
                continue;
            }
        }
        count = count.wrapping_add(hc[i] as i32);
        i += 1;
    }
    let mut j = i;
    while j < 255 && count < min2 {
        if j & 15 == 0 && j < 240 {
            let t = (bc[j >> 4] as i32).wrapping_add(count);
            if t < min2 {
                count = t;
                j += 16;
                continue;
            }
        }
        count = count.wrapping_add(hc[j] as i32);
        j += 1;
    }
    (i as u32, j as u32)
}

/// the Outline pixel function for lochist  [ol_px]
#[inline]
fn ol_px(intensity: u32, h: &LocHist, area: u32) -> u32 {
    let min1 = ((100 - intensity).wrapping_mul(area) / 200) as i32;
    let min2 = ((intensity + 100).wrapping_mul(area) / 200) as i32;
    let mut r = 0u32;
    for (c, sh) in [(2usize, 16u32), (1, 8), (0, 0)] {
        let (c1, c2) = ol_scan(&h.h[c], &h.blk[c], &h.h[c], &h.blk[c], min1, min2);
        r |= 255u32.wrapping_sub(c2).wrapping_add(c1) << sh;
    }
    let (_, a2) = ol_scan(&h.h[0], &h.blk[0], &h.h[3], &h.blk[3], min1, min2);
    r | a2 << 24
}

fn outline_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let t = a.params.i(0).clamp(1, 200) as i32;
    let inten = a.params.i(1).clamp(0, 100) as u32;
    lochist(a, dst, t, |h, area, _| ol_px(inten, h, area))
}

static PROPS_ANGLE45: [FxProp; 1] = [FxProp::new(FP_ANGLE, 2, 0, 0, "angle", -180.0, 180.0, 45.0)];
static PROPS_ANGLE0: [FxProp; 1] = [FxProp::new(FP_ANGLE, 2, 0, 0, "angle", -180.0, 180.0, 0.0)];
static PROPS_OUTLINE: [FxProp; 2] = [
    FxProp::new(FP_INT, 0, 0, 0, "thickness", 1.0, 200.0, 3.0),
    FxProp::new(FP_INT, 0, 1, 0, "intensity", 0.0, 100.0, 50.0),
];

const fn desc(id: u32, flags: u32, name: &'static str, render: RenderFn, props: &'static [FxProp]) -> FxDesc {
    FxDesc { id, flags, name, prepare: None, render, props, category: FXC_STYLIZE, user: 0 }
}

/// the group table  [fxtab_stylize]
pub static FXTAB: [FxDesc; 4] = [
    desc(FX_EDGE_DETECT, 0, "edge_detect", edge_render, &PROPS_ANGLE45),
    desc(FX_EMBOSS, 0, "emboss", emboss_render, &PROPS_ANGLE0),
    desc(FX_OUTLINE, FXF_SLOW, "outline", outline_render, &PROPS_OUTLINE),
    desc(FX_RELIEF, 0, "relief", relief_render, &PROPS_ANGLE45),
];
