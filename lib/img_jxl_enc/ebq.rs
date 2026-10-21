// ebq -- the writer's Butteraugli quantization loop at efforts 8 and 9 (libjxl 0.11.1
// enc_adaptive_quantization.cc FindBestQuantization, quantizer.cc SetQuantField): round trips
// of the frame with the current quant field, the Butteraugli distance per transform block, the
// field raised where a block is above the target distance and (first two rounds) lowered where it
// is below                                                              [jxl-tools: no asm]
//
//   set_quant_field(q, quant_dc, qf) -> raw field   global scale from the field's median
//   find_best(lin, w, h, acs, xb, yb, d, iters, threads, qf, q, rt) -> raw field
//
// libjxl's last round trip (after the last update) is not used for anything; it is skipped.

use alloc::vec;
use alloc::vec::Vec;

use crate::eaq::Quantizer;
use crate::ebutter::{Comparator, Params};
use crate::spline::pow_f64;

/// Quantizer::SetQuantField: the global scale and quant DC from the median of the field and the
/// median of the absolute deviations (nth_element: the same order statistic as a sort), the raw
/// field
pub fn set_quant_field(q: &mut Quantizer, quant_dc: f32, qf: &[f32]) -> Vec<i32> {
    let mut data = qf.to_vec();
    data.sort_unstable_by(|a, b| a.total_cmp(b));
    let med = data[data.len() / 2];
    let mut dev: Vec<f32> = data.iter().map(|&v| (v - med).abs()).collect();
    dev.sort_unstable_by(|a, b| a.total_cmp(b));
    let absd = dev[dev.len() / 2];
    q.compute(quant_dc, med, absd);
    qf.iter().map(|&v| q.raw(v)).collect()
}

/// std::lround of a float: to nearest, halves away from zero
fn lround(v: f32) -> i64 {
    let x = v as f64;
    let t = x as i64;
    let f = x - t as f64;
    if f >= 0.5 {
        t + 1
    } else if f <= -0.5 {
        t - 1
    } else {
        t
    }
}

/// TileDistMap (tile 8, no margin): per transform the 16-norm of the distances of its pixels
/// times 1.2, on every block it covers
fn tile_distmap(dm: &[f32], w: usize, h: usize, acs: &[u8], xb: usize, yb: usize) -> Vec<f32> {
    let mut out = vec![0f32; xb * yb];
    for by in 0..yb {
        for bx in 0..xb {
            let a8 = acs[by * xb + bx];
            if a8 & 1 == 0 {
                continue;
            }
            let (cx, cy) = crate::etx::cover((a8 >> 1) as usize);
            let (y0, y1) = (by * 8, (by * 8 + cy * 8).min(h));
            let (x0, x1) = (bx * 8, (bx * 8 + cx * 8).min(w));
            let mut norm = 0f32;
            let mut pix = 0f64;
            for y in y0..y1 {
                for &d in &dm[y * w + x0..y * w + x1] {
                    let mut v = d;
                    v *= v;
                    v *= v;
                    v *= v;
                    v *= v;
                    norm += v;
                    pix += 1.0;
                }
            }
            if pix == 0.0 {
                pix = 1.0;
            }
            let r = norm as f64 / pix;
            let td = if r.is_finite() { (1.2f32 as f64 * pow_f64(r, 0.0625)) as f32 } else { f32::INFINITY };
            for iy in 0..cy {
                for ix in 0..cx {
                    if by + iy < yb && bx + ix < xb {
                        out[(by + iy) * xb + bx + ix] = td;
                    }
                }
            }
        }
    }
    out
}

/// FindBestQuantization: lin = the reference (linear sRGB planes, w x h), qf = the initial
/// quant field (xb x yb, adjusted per transform), rt(q, raw field) = the decoded round trip
/// (linear sRGB planes, w x h); iters = 4 at effort 9, 2 at effort 8.  qf and q end as the
/// final field and quantizer; returns the raw field.
#[allow(clippy::too_many_arguments)]
pub fn find_best(
    lin: &[Vec<f32>; 3], w: usize, h: usize, acs: &[u8], xb: usize, yb: usize, d: f32, iters: usize, threads: bool,
    qf: &mut [f32], q: &mut Quantizer, rt: &mut dyn FnMut(&Quantizer, Vec<i32>) -> Result<[Vec<f32>; 3], i32>,
) -> Result<Vec<i32>, i32> {
    let quant_dc = crate::eaq::initial_quant_dc(d);
    let cmp = Comparator::new(lin, w, h, Params::default(), threads);
    // libjxl adjusts the (already adjusted) field once more here
    crate::eaq::adjust_quant_field(acs, xb, yb, d, qf);
    let init = qf.to_vec();
    let (mut mn, mut mx) = (init[0], init[0]);
    for &v in &init {
        mn = mn.min(v);
        mx = mx.max(v);
    }
    let dev_low = libm_sqrtf(250.0 / (mx / mn));
    let asym = if dev_low < 2.0 { dev_low } else { 2.0 };
    let lower = mn / (asym * dev_low);
    let higher = mx * (dev_low / asym);
    const POW: [f64; 8] = [0.2, 0.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    for i in 0..iters {
        let rqf = set_quant_field(q, quant_dc, qf);
        let dec = rt(q, rqf)?;
        let dm = cmp.diffmap(&dec);
        let tile = tile_distmap(&dm, w, h, acs, xb, yb);
        if i == 1 {
            // not much below the initial guess (the DC reconstruction would oscillate)
            let keep = 1.0f64 - 0.6;
            for (v, &v0) in qf.iter_mut().zip(&init) {
                let clamp = keep * *v as f64 + 0.6 * v0 as f64;
                if (*v as f64) < clamp {
                    *v = clamp as f32;
                    if *v > higher {
                        *v = higher;
                    }
                    if *v < lower {
                        *v = lower;
                    }
                }
            }
        }
        let cur_pow = if i < 7 { POW[i] } else { 0.0 };
        let (igs, scale) = (q.inv_global_scale, q.scale);
        for (v, &t) in qf.iter_mut().zip(&tile) {
            let diff = t / d;
            if diff <= 1.0 {
                if cur_pow != 0.0 {
                    *v = (*v as f64 * pow_f64(diff as f64, cur_pow)) as f32;
                }
            } else {
                let old = *v;
                *v *= diff;
                if lround(old * igs) == lround(*v * igs) {
                    *v = old + scale;
                }
            }
            if *v > higher {
                *v = higher;
            }
            if *v < lower {
                *v = lower;
            }
        }
    }
    Ok(set_quant_field(q, quant_dc, qf))
}

/// sqrtf (correctly rounded: through f64)
fn libm_sqrtf(x: f32) -> f32 {
    crate::spline::sqrt_f64(x as f64) as f32
}
