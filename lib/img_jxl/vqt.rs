// vqt -- JPEG XL dequantization tables (quant_weights.cc ComputeQuantTable, GetQuantWeights;
// fast_math-inl.h FastPowf; model: tools/jxl_ref.py quant_weights, quant_table, fast_powf)
//                                                                        [src/img/jxl_vqt.s]
//
//   jv_qtable(jv, t) -> 0 / ERR_*   jv.qtab[t] = f32[3][8 sx * 8 sy] = 1 / weight, from jv.qe[t]
//   jq_powf(b, e) -> f32            FastPowf
//
// f32 arithmetic in the model's order, no fused multiply-add.

use alloc::vec::Vec;

use img_core::ERR_HDR;
use rt::fx::{cvtt, sqrt};

use crate::modular::zvec;
use crate::tab;
use crate::vdc::{Jv, Qe};

const ALMOST0: f32 = f32::from_bits(0x322bcc77); // 1e-8
const BIG: f32 = f32::from_bits(0x4cbebc20); // 1 / 1e-8 as f32
const BAD: f32 = f32::from_bits(0x453ad000); // 0xBAD
const SQRT2: f32 = f32::from_bits(0x3fb504f3);
const E1M6: f32 = f32::from_bits(0x358637bd); // 1e-6

/// jq_powf: FastPow2f( FastLog2f( b ) * e )
#[inline]
pub fn jq_powf(b: f32, e: f32) -> f32 {
    let p = &tab::FASTPOW;
    let bits = b.to_bits() as i32;
    let ex = bits.wrapping_sub(0x3f2a_aaab) >> 23;
    let m = bits.wrapping_sub(ex.wrapping_shl(23));
    let t = f32::from_bits(m as u32) - 1.0;
    let mut yp = p[2] * t;
    yp = yp + p[1];
    yp = yp * t;
    yp = yp + p[0];
    let mut yq = p[5] * t;
    yq = yq + p[4];
    yq = yq * t;
    yq = yq + p[3];
    let lg = yp / yq + ex as f32;
    let x = lg * e;
    let mut fl = cvtt(x);
    let mut flf = fl as f32;
    if flf > x {
        fl = fl.wrapping_sub(1);
        flf = flf - 1.0;
    }
    let f = x - flf;
    let p2 = f32::from_bits((fl.wrapping_add(127) as u32).wrapping_shl(23));
    let mut num = f + p[6];
    num = num * f;
    num = num + p[7];
    num = num * f;
    num = num + p[8];
    num = num * p2;
    let mut den = f * p[9];
    den = den + p[10];
    den = den * f;
    den = den + p[11];
    den = den * f;
    den = den + p[12];
    num / den
}

/// jq_bands: bands[i] = bands[i - 1] * Mult( in[i] ), all >= 1e-8
fn bands(inp: &[f32], nb: u32, out: &mut [f32; 16]) -> i32 {
    let mut v = inp[0];
    if !(v >= ALMOST0) {
        return ERR_HDR;
    }
    out[0] = v;
    for i in 1..nb as usize {
        let x = inp[i];
        let m = if x > 0.0 { x + 1.0 } else { 1.0 / (1.0 - x) };
        v = v * m;
        if !(v >= ALMOST0) {
            return ERR_HDR;
        }
        out[i] = v;
    }
    0
}

/// jq_weights: GetQuantWeights for one channel (rows x cols into out)
fn weights(inp: &[f32], nb: u32, rows: u32, cols: u32, out: &mut [f32]) -> i32 {
    let mut b = [0f32; 16];
    let r = bands(inp, nb, &mut b);
    if r != 0 {
        return r;
    }
    let scale = (nb as i32 - 1) as f32 / (SQRT2 + E1M6);
    let rcpcol = scale / (cols as i32 - 1) as f32;
    let rcprow = scale / (rows as i32 - 1) as f32;
    for y in 0..rows {
        let dy = y as i32 as f32 * rcprow;
        let dy2 = dy * dy;
        for x in 0..cols {
            let v = if nb == 1 {
                b[0]
            } else {
                let dx = x as i32 as f32 * rcpcol;
                let d = sqrt(dx * dx + dy2);
                let i = cvtt(d);
                let frac = d - i as f32;
                let lo = b.get(i as usize).copied().unwrap_or(0.0);
                let hi = b.get(i as usize + 1).copied().unwrap_or(0.0);
                jq_powf(hi / lo, frac) * lo
            };
            out[(y * cols + x) as usize] = v;
        }
    }
    0
}

/// jv_qtable: jv.qtab[t] from jv.qe[t]
pub fn jv_qtable(jv: &mut Jv, t: usize) -> i32 {
    match qweights(&jv.qe[t], t) {
        Ok(out) => {
            jv.qtab[t] = out;
            0
        }
        Err(e) => e,
    }
}

/// the dequantization table 1 / weight of encoding q for table t (jxl-plus: shared with the
/// encoder)
pub fn qweights(q: &Qe, t: usize) -> Result<Vec<f32>, i32> {
    let rows = (tab::QTSX[t] as u32) << 3;
    let cols = (tab::QTSX[t + 17] as u32) << 3;
    let num = (rows * cols) as usize;
    let mut out: Vec<f32> = zvec(num * 3 + 16)?;
    match q.mode {
        6 => {
            for c in 0..3 {
                let r = weights(&q.b1[c * 16..], q.nb, rows, cols, &mut out[c * num..c * num + num]);
                if r != 0 {
                    return Err(r);
                }
            }
        }
        1 => {
            for c in 0..3 {
                let (w0, w1, w2) = (q.w[c * 3], q.w[c * 3 + 1], q.w[c * 3 + 2]);
                let m = &mut out[c * 64..c * 64 + 64];
                m.fill(w0);
                m[1] = w1;
                m[8] = w1;
                m[9] = w2;
            }
        }
        2 => {
            for c in 0..3 {
                let d = &q.w[c * 6..c * 6 + 6];
                for y in 0..8u32 {
                    for x in 0..8u32 {
                        let mx = y.max(x);
                        let mn = y.min(x);
                        let v = if mx < 2 {
                            if mx == 0 {
                                BAD
                            } else if mn == 0 {
                                d[0]
                            } else {
                                d[1]
                            }
                        } else if mx < 4 {
                            if mn < 2 { d[2] } else { d[3] }
                        } else if mn < 4 {
                            d[4]
                        } else {
                            d[5]
                        };
                        out[c * 64 + (y * 8 + x) as usize] = v;
                    }
                }
            }
        }
        3 => {
            for c in 0..3 {
                let mut w44 = [0f32; 16];
                let r = weights(&q.b1[c * 16..], q.nb, 4, 4, &mut w44);
                if r != 0 {
                    return Err(r);
                }
                let m = &mut out[c * 64..c * 64 + 64];
                for y in 0..8usize {
                    for x in 0..8usize {
                        m[y * 8 + x] = w44[(y >> 1) * 4 + (x >> 1)];
                    }
                }
                let (m0, m1) = (q.w[c * 2], q.w[c * 2 + 1]);
                m[1] = m[1] / m0;
                m[8] = m[8] / m0;
                m[9] = m[9] / m1;
            }
        }
        4 => {
            for c in 0..3 {
                let mut w48 = [0f32; 32];
                let r = weights(&q.b1[c * 16..], q.nb, 4, 8, &mut w48);
                if r != 0 {
                    return Err(r);
                }
                let m = &mut out[c * 64..c * 64 + 64];
                for y in 0..8usize {
                    for x in 0..8usize {
                        m[y * 8 + x] = w48[(y >> 1) * 8 + x];
                    }
                }
                m[8] = m[8] / q.w[c];
            }
        }
        7 => {
            if q.raw.is_empty() {
                return Err(ERR_HDR);
            }
            for i in 0..num * 3 {
                out[i] = 1.0 / (q.raw[i] as f32 * q.den);
            }
        }
        5 => {
            for c in 0..3 {
                let mut w48 = [0f32; 32];
                let r = weights(&q.b1[c * 16..], q.nb, 4, 8, &mut w48);
                if r != 0 {
                    return Err(r);
                }
                let mut w44 = [0f32; 16];
                let r = weights(&q.b2[c * 16..], q.nb2, 4, 4, &mut w44);
                if r != 0 {
                    return Err(r);
                }
                let aw = &q.w[c * 9..c * 9 + 9];
                let mut bb = [0f32; 16];
                let r = bands(&aw[5..9], 4, &mut bb);
                if r != 0 {
                    return Err(r);
                }
                let m = &mut out[c * 64..c * 64 + 64];
                m.fill(0.0);
                m[0] = 1.0;
                m[8] = aw[0];
                m[1] = aw[1];
                m[16] = aw[2];
                m[2] = aw[3];
                m[18] = aw[4];
                let (lo, hi) = (tab::AFV[0], tab::AFV[1]);
                for f in 0..16usize {
                    let x = f & 3;
                    let y = f >> 2;
                    if x < 2 && y < 2 {
                        continue;
                    }
                    let pos = tab::AFV[2 + f] - lo;
                    let sp = (pos * 3.0) / hi;
                    let i = cvtt(sp);
                    let frac = sp - i as f32;
                    let l = bb.get(i as usize).copied().unwrap_or(0.0);
                    let h = bb.get(i as usize + 1).copied().unwrap_or(0.0);
                    m[16 * y + 2 * x] = jq_powf(h / l, frac) * l;
                }
                for y in 0..4usize {
                    for x in 0..8usize {
                        if y == 0 && x == 0 {
                            continue;
                        }
                        m[(2 * y + 1) * 8 + x] = w48[y * 8 + x];
                        if x < 4 {
                            m[16 * y + 2 * x + 1] = w44[y * 4 + x];
                        }
                    }
                }
            }
        }
        _ => return Err(ERR_HDR),
    }
    // range check and 1 / w
    for v in out.iter_mut().take(num * 3) {
        let x = *v;
        if !(x < BIG) || !(x >= ALMOST0) {
            return Err(ERR_HDR);
        }
        *v = 1.0 / x;
    }
    Ok(out)
}
