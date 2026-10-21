// noise -- effect group "noise" (Rauschen): add_noise, reduce_noise         [src/fx/fx_noise.s]
//
//   FXTAB            the group table
//
//   add_noise     intensity (0..100), colour saturation (0..400), coverage (0..100), seed:
//                 paint.net 3's Add Noise with per-pixel hashes h_k = hash32(x, y, seed, k):
//                 changed when h_0 mod 100 < coverage; n_k = ((sum of the 4 bytes of h_k) -
//                 510) * 1774 >> 8 for R (1), G (2), B (3); i = (4899 r + 9618 g + 1867 b) >> 14,
//                 c = i + (((c - i) sat) >> 12), sat = saturation * 4096 / 100, dev = I^2 / 4,
//                 channel' = clamp(channel + ((c dev + 32768) >> 16)); alpha kept
//   reduce_noise  radius (0..200), strength (0..1): per channel the median m of the disc,
//                 f = strength (1 - 0.75 I), v' = trunc(clamp(v + f (m - v), 0, 255))

use rt::fx::{cvttd, maxsd, minsd};

use crate::conv::*;
use crate::lochist::*;
use crate::*;

#[inline(always)]
fn noise_ch(src: u32, sh: u32, n: i32, dev: i32) -> u32 {
    let d = n.wrapping_mul(dev).wrapping_add(32768) >> 16;
    (((src >> sh & 255) as i32 + d).clamp(0, 255) as u32) << sh
}

fn add_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(rect) = roi(a) else {
        return 0;
    };
    let i = pint(a, 0, 0, 100);
    let dev = ((i * i) as u32 >> 2) as i32;
    let sat = (((pint(a, 1, 0, 400) as u32) << 12) / 100) as i32;
    let cov = pint(a, 2, 0, 100) as u32;
    let seed = a.params.0[3] as u32;
    for y in rect[1]..rect[3] {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let s = a.src_row(y);
        let d = dst.row(y);
        for x in rect[0]..rect[2] {
            let mut p = px(s, x as usize);
            if hash32(x as u32, y as u32, seed, 0) % 100 < cov {
                let mut nz = [0i32; 3];
                for (k, nv) in nz.iter_mut().enumerate() {
                    let h = hash32(x as u32, y as u32, seed, k as u32 + 1);
                    let sum = (h & 255) + (h >> 8 & 255) + (h >> 16 & 255) + (h >> 24);
                    *nv = (sum as i32 - 510).wrapping_mul(1774) >> 8;
                }
                let [r, g, b] = nz;
                let iv = (r.wrapping_mul(4899).wrapping_add(g.wrapping_mul(9618)).wrapping_add(b.wrapping_mul(1867))) >> 14;
                let r = (r - iv).wrapping_mul(sat) >> 12;
                let g = (g - iv).wrapping_mul(sat) >> 12;
                let b = (b - iv).wrapping_mul(sat) >> 12;
                p = p & 0xFF00_0000 | noise_ch(p, 16, r + iv, dev) | noise_ch(p, 8, g + iv, dev) | noise_ch(p, 0, b + iv, dev);
            }
            put(d, (x - a.x0) as usize, p);
        }
    }
    0
}

fn reduce_px(h: &Hist, strength: f64) -> u32 {
    let c = h.src;
    if c & 0xFF00_0000 == 0 || h.ncol == 0 {
        return c;
    }
    let iv = ((c & 255) as f64 * 0.114 + (c >> 8 & 255) as f64 * 0.587 + (c >> 16 & 255) as f64 * 0.299) / 255.0;
    let f = (1.0 - iv * 0.75) * strength;
    let mut o = c & 0xFF00_0000;
    for ch in 0..3 {
        let v = (c >> (8 * ch) & 255) as i32;
        let m = lh_percentile(h, ch, (h.ncol >> 1) as i32) as i32;
        let r = minsd(maxsd((m - v) as f64 * f + v as f64, 0.0), 255.0);
        o |= (cvttd(r) as u32) << (8 * ch);
    }
    o
}

fn reduce_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let r = pint(a, 0, 0, 200);
    let strength = minsd(maxsd(pdbl(a, 1), 0.0), 1.0);
    lh_run(a, dst, r, LHS_DISC, true, |h, _, _| reduce_px(h, strength))
}

static P_ADD: [FxProp; 4] = [
    FxProp::new(FP_INT, 0, 0, 0, "intensity", 0.0, 100.0, 64.0),
    FxProp::new(FP_INT, 0, 1, 0, "color_saturation", 0.0, 400.0, 100.0),
    FxProp::new(FP_INT, 0, 2, 0, "coverage", 0.0, 100.0, 100.0),
    FxProp::new(FP_SEED, 0, 3, 0, "seed", 0.0, 2147483647.0, 0.0),
];
static P_REDUCE: [FxProp; 2] = [
    FxProp::new(FP_INT, 0, 0, 0, "radius", 0.0, 200.0, 6.0),
    FxProp::new(FP_DEC, 2, 1, 0, "strength", 0.0, 1.0, 0.4),
];

/// the group table  [fxtab_noise]
pub static FXTAB: [FxDesc; 2] = [
    FxDesc { id: FX_ADD_NOISE, flags: FXF_SEED, name: "add_noise", prepare: None, render: add_render, props: &P_ADD, category: FXC_NOISE, user: 0 },
    FxDesc { id: FX_REDUCE_NOISE, flags: FXF_SLOW, name: "reduce_noise", prepare: None, render: reduce_render, props: &P_REDUCE, category: FXC_NOISE, user: 0 },
];
