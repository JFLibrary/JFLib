// rnd -- JPEG XL render pipeline (render_pipeline/stage_*.cc; model: tools/jxl_ref.py
// Frame.render, modular_color, ec_planes, chroma_up_h/v, gaborish, upsample, xyb_to_linear,
// ycbcr_to_rgb, from_linear, to_rgba8)                                    [src/img/jxl_rnd.s]
//
//   Jrp / Jrn                       a plane of f32 samples (row stride s, a multiple of 4) /
//                                   the planes of a frame (3 colour + extra channels)
//   jx_render(jx, jf) -> 0 / ERR_*  planes of a decoded frame through the pipeline into
//                                   jf.rnd: 3 colour planes (output space, 0..1 nominal) and
//                                   the extra channels
//   jx_out8(jx, jf, sf) -> 0 / ERR_*   jf.rnd colour + alpha into the ARGB surface (image
//                                   size): floor( v * 255 + 0.5 ) in f64, clamped
//   jr_alloc(w, h) / jr_pad(jrp, p) / jr_mirror(i, n) / jr_ups(jrp, shift, weights)
//   jr_pad_into(jrp, p, out)        jxl-speed: jr_pad into a buffer of the right size (reused)
//
// f32 arithmetic in the model's order (the assembly's four samples per SSE vector are done
// lane by lane, no fused multiply-add).  Where the assembly writes whole vectors, the
// samples of the padding columns up to the next multiple of 4 are computed as well.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use img_core::{ERR_HDR, ERR_UNSUPPORTED};
use rt::fx::{cvttd, maxsd, maxss, minsd, minss, sqrt};

use crate::frm::{Jf, JBM_REPLACE, JCT_XYB, JCT_YCBCR, JFF_NOISE, JFT_DC, JFT_REGULAR, JFT_SKIPPROG};
use crate::hdr::{Jx, JCS_GRAY, JTF_709, JTF_DCI, JTF_HLG, JTF_LINEAR, JTF_PQ, JTF_SRGB};
use crate::modular::{zvec, Mc};
use crate::tab;
use crate::vqt::jq_powf;

/// a plane of f32 samples (JRP)
#[derive(Clone, Default)]
pub struct Jrp {
    pub d: Vec<f32>,
    pub w: u32,
    pub h: u32,
    pub s: u32, // row stride (floats)
}

/// the planes of a frame (JRN)
#[derive(Clone, Default)]
pub struct Jrn {
    pub np: u32,
    pub alpha: i32, // plane of the alpha channel or -1
    pub xyb: u32,   // reference frame saved before the colour transform
    pub p: Vec<Jrp>,
}

/// round up to a multiple of 4
#[inline]
pub fn r4(w: u32) -> u32 {
    w.wrapping_add(3) & !3
}

/// jr_mirror: Mirror( i ) into 0 .. n - 1
#[inline]
pub fn jr_mirror(i: i32, n: i32) -> i32 {
    let mut i = i;
    loop {
        if i < 0 {
            i = !i;
        } else if i >= n {
            i = n.wrapping_mul(2).wrapping_sub(1).wrapping_sub(i);
        } else {
            return i;
        }
    }
}

/// rows per band of the parallel stages
const BAND: usize = 32;

/// f(y0, rows, the planes' rows y0 .. y0 + rows) for bands of BAND rows on the worker pool
/// (every stage that uses it computes each row from inputs it does not write, so the result
/// does not depend on the threads)  [jxl-plus]
pub(crate) fn par_bands<T: Send>(planes: Vec<(&mut [T], usize)>, h: usize, f: impl Fn(usize, usize, &mut [&mut [T]]) + Sync) {
    let nb = h.div_ceil(BAND);
    if nb == 0 {
        return;
    }
    let k = planes.len();
    let mut per: Vec<Vec<&mut [T]>> = (0..nb).map(|_| Vec::with_capacity(k)).collect();
    for (d, s) in planes {
        let total = (h * s).min(d.len());
        for (b, ch) in d[..total].chunks_mut((BAND * s).max(1)).enumerate().take(nb) {
            per[b].push(ch);
        }
    }
    let cells: Vec<rt::sync::SpinLock<Vec<&mut [T]>>> = per.into_iter().map(rt::sync::SpinLock::new).collect();
    crate::par::par_map(nb, true, |b| {
        let mut g = cells[b].lock();
        if g.len() == k {
            f(b * BAND, BAND.min(h - b * BAND), &mut g[..]);
        }
    });
}

/// jr_alloc: zeroed plane samples, stride (w + 3) & -4 floats
pub fn jr_alloc(w: u32, h: u32) -> Result<Vec<f32>, i32> {
    zvec(r4(w) as usize * h as usize + 16)
}

/// jr_pad: mirrored copy with a border of p (stride s + 2 p, h + 2 p rows)
pub fn jr_pad(jp: &Jrp, p: u32) -> Result<Vec<f32>, i32> {
    let mut out = Vec::new();
    jr_pad_into(jp, p, &mut out)?;
    Ok(out)
}

/// jr_pad into out: a new zeroed vector unless out has the size already (then it held a
/// padding of a plane of the same dimensions: the samples are rewritten, the columns past
/// the border are still 0)  [jxl-speed]
pub fn jr_pad_into(jp: &Jrp, p: u32, out: &mut Vec<f32>) -> Result<(), i32> {
    let ps = (jp.s + 2 * p) as usize;
    let rows = (jp.h + 2 * p) as usize;
    if out.len() != ps * rows + 16 {
        *out = zvec(ps * rows + 16)?;
    }
    let w = jp.w as usize;
    let pu = p as usize;
    let s = jp.s as usize;
    // jxl-plus: bands of rows on the worker pool, the border columns only
    par_bands(alloc::vec![(&mut out[..], ps)], rows, |r0, nr, b| {
        for rr in 0..nr {
            let r = r0 + rr;
            let sr = jr_mirror(r as i32 - p as i32, jp.h as i32) as usize;
            let src = &jp.d[sr * s..sr * s + w];
            let dst = &mut b[0][rr * ps..rr * ps + ps];
            dst[pu..pu + w].copy_from_slice(src);
            for c in (0..pu).chain(w + pu..w + 2 * pu) {
                dst[c] = src[jr_mirror(c as i32 - p as i32, jp.w as i32) as usize];
            }
        }
    });
    Ok(())
}

/// jr_setnew: replace the samples of jp by a jr_alloc plane of w x h
fn setnew(jp: &mut Jrp, d: Vec<f32>, w: u32, h: u32) {
    jp.d = d;
    jp.w = w;
    jp.h = h;
    jp.s = r4(w);
}

/// jr_ints: plane = f32( sample (+ sample of mc2) ) * f
fn ints(jp: &mut Jrp, mc: &Mc, f: f32, mc2: Option<&Mc>) -> i32 {
    let d = match jr_alloc(mc.w, mc.h) {
        Ok(v) => v,
        Err(e) => return e,
    };
    setnew(jp, d, mc.w, mc.h);
    let w = mc.w as usize;
    let s = jp.s as usize;
    for y in 0..mc.h as usize {
        for x in 0..w {
            let k = y * w + x;
            let mut v = mc.d[k] as i64;
            if let Some(m2) = mc2 {
                v += m2.d.get(k).copied().unwrap_or(0) as i64;
            }
            jp.d[y * s + x] = v as f32 * f;
        }
    }
    0
}

/// jr_scale: f32( 1 / (2^bits - 1) )
fn scale(bits: u32) -> f32 {
    let m = (1u64 << (bits & 63)).wrapping_sub(1);
    (1.0f64 / m as i64 as f64) as f32
}

/// jr_colour: colour planes of the frame (chroma not yet upsampled)
fn colour(jx: &Jx, jf: &mut Jf, jrn: &mut Jrn) -> i32 {
    if jf.modular == 0 {
        let mut wh = [(0u32, 0u32); 3];
        for (c, v) in wh.iter_mut().enumerate() {
            let hs = jf.hshift(c) & 31;
            let vs = jf.vshift(c) & 31;
            *v = (((1u32 << hs) - 1).wrapping_add(jf.w) >> hs, ((1u32 << vs) - 1).wrapping_add(jf.h) >> vs);
        }
        let Some(jv) = jf.vd.as_mut() else { return ERR_HDR };
        for c in 0..3 {
            let p = &mut jrn.p[c];
            p.d = core::mem::take(&mut jv.px[c]);
            p.s = jv.pxw[c];
            p.w = wh[c].0;
            p.h = wh[c].1;
        }
        return 0;
    }
    let Some(full) = jf.full.as_ref() else { return ERR_HDR };
    if jf.ct == JCT_XYB {
        let ch = &full.ch;
        if ch.len() < 3 {
            return ERR_HDR;
        }
        let r = ints(&mut jrn.p[0], &ch[1], jf.dcq[0], None);
        if r != 0 {
            return r;
        }
        let r = ints(&mut jrn.p[1], &ch[0], jf.dcq[1], None);
        if r != 0 {
            return r;
        }
        return ints(&mut jrn.p[2], &ch[2], jf.dcq[2], Some(&ch[0]));
    }
    if jx.flt != 0 {
        return ERR_UNSUPPORTED;
    }
    let f = scale(jx.bits);
    for c in 0..3usize {
        let k = if jf.nbcol == 1 { 0 } else { c };
        let Some(mc) = full.ch.get(k) else { return ERR_HDR };
        let r = ints(&mut jrn.p[c], mc, f, None);
        if r != 0 {
            return r;
        }
    }
    0
}

/// jr_ecs: extra channel planes
fn ecs(jx: &Jx, jf: &Jf, jrn: &mut Jrn) -> i32 {
    let Some(full) = jf.full.as_ref() else { return ERR_HDR };
    for e in 0..jx.nec as usize {
        let ec = &jx.ec[e];
        if ec.flt != 0 {
            return ERR_UNSUPPORTED;
        }
        let f = scale(ec.bits);
        let Some(mc) = full.ch.get(jf.nbcol as usize + e) else { return ERR_HDR };
        let r = ints(&mut jrn.p[3 + e], mc, f, None);
        if r != 0 {
            return r;
        }
    }
    0
}

const C025: f32 = 0.25;
const C075: f32 = 0.75;

/// jr_chromah: out[2x] = 0.25 q[x - 1] + 0.75 q[x], out[2x + 1] = 0.25 q[x + 1] + ...
fn chromah(jp: &mut Jrp) -> i32 {
    let w = jp.w;
    let mut out = match jr_alloc(w * 2, jp.h) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let os = r4(w * 2) as usize;
    let s = jp.s as usize;
    for y in 0..jp.h as usize {
        let src = &jp.d[y * s..];
        for x in 0..w as i32 {
            let l = src[jr_mirror(x - 1, w as i32) as usize];
            let r = src[jr_mirror(x + 1, w as i32) as usize];
            let cur = src[x as usize] * C075;
            out[y * os + 2 * x as usize] = l * C025 + cur;
            out[y * os + 2 * x as usize + 1] = r * C025 + cur;
        }
    }
    let h = jp.h;
    setnew(jp, out, w * 2, h);
    0
}

/// jr_chromav: out[2y] = q[y - 1] 0.25 + 0.75 q[y], out[2y + 1] = q[y + 1] 0.25 + ...
fn chromav(jp: &mut Jrp) -> i32 {
    let w = jp.w as usize;
    let h = jp.h;
    let mut out = match jr_alloc(jp.w, h * 2) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let os = r4(jp.w) as usize;
    let s = jp.s as usize;
    for y in 0..h as i32 {
        let a = jr_mirror(y - 1, h as i32) as usize * s;
        let b = jr_mirror(y + 1, h as i32) as usize * s;
        let t = y as usize * s;
        let o0 = 2 * y as usize * os;
        let o1 = o0 + os;
        for x in 0..w {
            let cur = jp.d[t + x] * C075;
            out[o0 + x] = jp.d[a + x] * C025 + cur;
            out[o1 + x] = jp.d[b + x] * C025 + cur;
        }
    }
    let ww = jp.w;
    setnew(jp, out, ww, h * 2);
    0
}

/// jr_gab: out = s2 w2 + (s1 w1 + p w0), weights normalized
fn gab(jf: &Jf, jrn: &mut Jrn) -> i32 {
    for c in 0..3 {
        let w1 = jf.gabw[c * 2];
        let w2 = jf.gabw[c * 2 + 1];
        let w0 = 1.0f32 / ((w1 + w2) * 4.0 + 1.0);
        let w1 = w1 * w0;
        let w2 = w2 * w0;
        let jp = &mut jrn.p[c];
        let pad = match jr_pad(jp, 1) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let ps = (jp.s + 2) as usize;
        let s = jp.s as usize;
        let wv = r4(jp.w) as usize;
        let h = jp.h as usize;
        par_bands(alloc::vec![(&mut jp.d[..], s)], h, |y0, rows, b| {
            for yy in 0..rows {
                let y = y0 + yy;
                let r0 = &pad[y * ps..];
                let r1 = &pad[(y + 1) * ps..];
                let r2 = &pad[(y + 2) * ps..];
                let o = &mut b[0][yy * s..yy * s + wv];
                for x in 0..wv {
                    let s1 = (r1[x] + r1[x + 2]) + (r0[x + 1] + r2[x + 1]);
                    let s2 = (r0[x] + r0[x + 2]) + (r2[x] + r2[x + 2]);
                    o[x] = s2 * w2 + (s1 * w1 + r1[x + 1] * w0);
                }
            }
        });
    }
    0
}

/// jr_ups: UpsamplingStage (factor 1 << shift, in place)
pub fn jr_ups(jp: &mut Jrp, shift: u32, wts: &[f32]) -> i32 {
    let n = 1usize << shift;
    let hn = n >> 1;
    let nw = (n * 5) >> 1;
    let mut k: Vec<[f32; 25]> = vec![[0.0; 25]; n * n];
    for p in 0..n * n {
        let (oy, ox) = (p / n, p % n);
        for t in 0..25usize {
            let (iy, ix) = (t / 5, t % 5);
            let (ky, a) = if oy < hn { (oy, iy) } else { (n - 1 - oy, 4 - iy) };
            let (kx, b) = if ox < hn { (ox, ix) } else { (n - 1 - ox, 4 - ix) };
            let j = 5 * ky + a;
            let i = 5 * kx + b;
            let (y, x) = if j <= i { (j, i) } else { (i, j) };
            let idx = (nw * y) as u32 - ((((y as u32).wrapping_sub(1)).wrapping_mul(y as u32)) >> 1) + (x - y) as u32;
            k[p][t] = wts.get(idx as usize).copied().unwrap_or(0.0);
        }
    }
    let pad = match jr_pad(jp, 2) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let ps = (jp.s + 4) as usize;
    let ow = jp.w * n as u32;
    let oh = jp.h * n as u32;
    let mut out = match jr_alloc(ow, oh) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let so = r4(ow) as usize;
    let w = jp.w as usize;
    let mut win = [0f32; 25];
    for y in 0..jp.h as usize {
        for x in 0..w {
            for r in 0..5 {
                let row = &pad[(y + r) * ps + x..];
                win[r * 5..r * 5 + 5].copy_from_slice(&row[..5]);
            }
            let mut mn = win[12];
            let mut mx = win[12];
            for &v in &win {
                mn = minss(v, mn);
                mx = maxss(v, mx);
            }
            for (p, kp) in k.iter().enumerate() {
                let mut acc = 0.0f32;
                for t in 0..25 {
                    acc = acc + win[t] * kp[t];
                }
                acc = maxss(acc, mn);
                acc = minss(acc, mx);
                let (oy, ox) = (p / n, p % n);
                out[(n * y + oy) * so + n * x + ox] = acc;
            }
        }
    }
    setnew(jp, out, ow, oh);
    0
}

/// jr_ycc: (cb, y, cr) -> (r, g, b)
fn ycc(jrn: &mut Jrn) {
    let k128 = 128.0f32 / 255.0;
    let c1402 = tab::YCC[0];
    let c1772 = tab::YCC[1];
    let cgcb = (tab::YCC[2] * tab::YCC[1]) / tab::YCC[3];
    let cgcr = (tab::YCC[4] * tab::YCC[0]) / tab::YCC[3];
    let (w, h) = (r4(jrn.p[0].w) as usize, jrn.p[0].h as usize);
    let (s0, s1, s2) = (jrn.p[0].s as usize, jrn.p[1].s as usize, jrn.p[2].s as usize);
    let [p0, p1, p2, ..] = &mut jrn.p[..] else { return };
    for y in 0..h {
        for x in 0..w {
            let cb = p0.d[y * s0 + x];
            let yv = p1.d[y * s1 + x] + k128;
            let cr = p2.d[y * s2 + x];
            let r = c1402 * cr + yv;
            let g = cgcr * cr + (cgcb * cb + yv);
            let b = cb * c1772 + yv;
            p0.d[y * s0 + x] = r;
            p1.d[y * s1 + x] = g;
            p2.d[y * s2 + x] = b;
        }
    }
}

/// jr_cbrt: cube root (Newton from the f32 estimate 2^(e / 3)), a > 0
fn cbrt(a: f64) -> f64 {
    let e = (((a as f32).to_bits() >> 23) as i32).wrapping_sub(127);
    let q = e / 3;
    let mut x = f64::from_bits(((q.wrapping_add(1023)) as u64) << 52);
    for _ in 0..60 {
        let x2 = x * x;
        let t = a / x2;
        x = ((x + x) + t) * tab::RNDD[7];
    }
    x
}

/// jr_xyb: XybToRgb into linear sRGB
fn xyb(jx: &Jx, jrn: &mut Jrn) {
    let f = (255.0f32 / f32::from_bits(jx.itarget)) as f64;
    let mut row = [0f64; 3];
    let gray = jx.cspace == JCS_GRAY;
    if gray {
        for i in 0..3 {
            row[i] = (jx.invop[i] as f64 * tab::RNDD[2] + jx.invop[3 + i] as f64 * tab::RNDD[3])
                + jx.invop[6 + i] as f64 * tab::RNDD[4];
        }
    }
    let mut m = [0f32; 9];
    for i in 0..9 {
        let v = if gray { row[i % 3] } else { jx.invop[i] as f64 };
        m[i] = (v * f) as f32;
    }
    let mut nb = [0f32; 3];
    let mut cb = [0f32; 3];
    for i in 0..3 {
        let b = jx.opbias[i];
        nb[i] = b;
        let d = b as f64;
        let ad = f64::from_bits(d.to_bits() & 0x7FFF_FFFF_FFFF_FFFF);
        cb[i] = if ad == 0.0 || ad.is_nan() {
            0.0
        } else {
            f64::from_bits(cbrt(ad).to_bits() | (d.to_bits() & 0x8000_0000_0000_0000)) as f32
        };
    }
    let (w, h) = (r4(jrn.p[0].w) as usize, jrn.p[0].h as usize);
    let (s0, s1, s2) = (jrn.p[0].s as usize, jrn.p[1].s as usize, jrn.p[2].s as usize);
    let [p0, p1, p2, ..] = &mut jrn.p[..] else { return };
    let planes = alloc::vec![(&mut p0.d[..], s0), (&mut p1.d[..], s1), (&mut p2.d[..], s2)];
    par_bands(planes, h, |_, rows, b| {
        let [q0, q1, q2] = b else { return };
        for y in 0..rows {
            for x in 0..w {
                let xv = q0[y * s0 + x];
                let yv = q1[y * s1 + x];
                let bv = q2[y * s2 + x];
                let g0 = (yv + xv) - cb[0];
                let g1 = (yv - xv) - cb[1];
                let g2 = bv - cb[2];
                let m0 = (g0 * g0) * g0 + nb[0];
                let m1 = (g1 * g1) * g1 + nb[1];
                let m2 = (g2 * g2) * g2 + nb[2];
                q0[y * s0 + x] = (m[0] * m0 + m[1] * m1) + m[2] * m2;
                q1[y * s1 + x] = (m[3] * m0 + m[4] * m1) + m[5] * m2;
                q2[y * s2 + x] = (m[6] * m0 + m[7] * m1) + m[8] * m2;
            }
        }
    });
}

/// jr_srgb: TF_SRGB EncodedFromDisplay of every sample
fn srgb(jp: &mut Jrp) {
    let t = &tab::SRGB;
    let (thr, slope) = (t[10], t[11]);
    let w = r4(jp.w) as usize;
    let s = jp.s as usize;
    let h = jp.h as usize;
    par_bands(alloc::vec![(&mut jp.d[..], s)], h, |_, rows, b| {
        for y in 0..rows {
            for v in b[0][y * s..y * s + w].iter_mut() {
                let x = *v;
                let a = f32::from_bits(x.to_bits() & 0x7FFF_FFFF);
                let sq = sqrt(a);
                let mut yp = t[4];
                let mut yq = t[9];
                for k in [3usize, 2, 1, 0] {
                    yp = yp * sq + t[k];
                    yq = yq * sq + t[5 + k];
                }
                let mag = if thr < a { yp / yq } else { a * slope };
                let neg = if x < 0.0 { 0x8000_0000 } else { 0 };
                *v = f32::from_bits(mag.to_bits() ^ neg);
            }
        }
    });
}

/// jr_gamma: v = p <= 1e-5 ? 0 : FastPowf( p, ig )
fn gamma(jp: &mut Jrp, ig: f32) {
    let lo = tab::SRGB[12];
    let s = jp.s as usize;
    let (w, h) = (jp.w as usize, jp.h as usize);
    par_bands(alloc::vec![(&mut jp.d[..], s)], h, |_, rows, b| {
        for y in 0..rows {
            for v in b[0][y * s..y * s + w].iter_mut() {
                let x = *v;
                *v = if x > lo { jq_powf(x, ig) } else { 0.0 };
            }
        }
    });
}

/// jr_fromlin: FromLinearStage for the image colour encoding
fn fromlin(jx: &Jx, jrn: &mut Jrn) -> i32 {
    let ig;
    if jx.havegamma != 0 {
        ig = (jx.gamma as f64 * tab::RNDD[1]) as f32;
    } else if jx.tf == JTF_DCI {
        ig = 1.0f32 / tab::SRGB[13];
    } else {
        if jx.tf == JTF_LINEAR {
            return 0;
        }
        if jx.tf != JTF_SRGB {
            return ERR_UNSUPPORTED;
        }
        for c in 0..3 {
            srgb(&mut jrn.p[c]);
        }
        return 0;
    }
    for c in 0..3 {
        gamma(&mut jrn.p[c], ig);
    }
    0
}

/// jr_tolin: linear planes to the output encoding when to_output; afterwards they are not
/// linear
fn tolin(jx: &Jx, jrn: &mut Jrn, lin: &mut bool, to_output: bool) -> i32 {
    if !*lin {
        return 0;
    }
    *lin = false;
    if to_output { fromlin(jx, jrn) } else { 0 }
}

/// jr_upsw: jr_ups with the image's weights of that factor
fn upsw(jx: &Jx, jp: &mut Jrp, shift: u32) -> i32 {
    let w: &[f32] = match shift {
        1 => &jx.up[0..15],
        2 => &jx.up[15..70],
        3 => &jx.up[70..280],
        _ => return ERR_HDR,
    };
    jr_ups(jp, shift, w)
}

/// jx_rndfree
pub fn jx_rndfree(jf: &mut Jf) {
    jf.rnd = None;
}

/// jf.dbg bit: an XYB frame's output stays linear (the writer's Butteraugli round trips)
/// [jxl-tools]
pub const JDBG_LINEAR: u32 = 128;

/// jx_render: the planes of a decoded frame through the pipeline into jf.rnd
pub fn jx_render(jx: &mut Jx, jf: &mut Jf) -> i32 {
    jf.rnd = None;
    let np = jx.nec + 3;
    let mut jrn = Box::new(Jrn { np, alpha: -1, xyb: 0, p: vec![Jrp::default(); np as usize] });
    let r = render(jx, jf, &mut jrn);
    jf.rnd = Some(jrn);
    if r != 0 {
        return r;
    }
    // jxl-plus: a DC frame's output for the frames using it
    if let Some(d) = jf.newdc.take() {
        jx.dcf[((jf.dclevel.wrapping_sub(1)) & 3) as usize] = Some(d);
    }
    // store the new reference frame
    if let Some(nr) = jf.newref.take() {
        let k = (jf.saveref & 3) as usize;
        jx.refs[k] = Some(nr);
    }
    0
}

fn render(jx: &Jx, jf: &mut Jf, jrn: &mut Jrn) -> i32 {
    let r = colour(jx, jf, jrn);
    if r != 0 {
        return r;
    }
    // chroma upsampling, crop to the frame size
    for c in 0..3 {
        if jf.hshift(c) != 0 {
            let r = chromah(&mut jrn.p[c]);
            if r != 0 {
                return r;
            }
        }
        if jf.vshift(c) != 0 {
            let r = chromav(&mut jrn.p[c]);
            if r != 0 {
                return r;
            }
        }
        let p = &mut jrn.p[c];
        if jf.w > p.w || jf.h > p.h {
            return ERR_HDR;
        }
        p.w = jf.w;
        p.h = jf.h;
    }
    if jf.gab != 0 {
        let r = gab(jf, jrn);
        if r != 0 {
            return r;
        }
    }
    if jf.dbg & 8 != 0 {
        return 0; // jxl-plus: the writer's reconstruction before the EPF
    }
    if jf.epfit != 0 {
        let r = crate::epf::jr_epf(jx, jf, jrn);
        if r != 0 {
            return r;
        }
    }
    let r = ecs(jx, jf, jrn);
    if r != 0 {
        return r;
    }
    // late: upsampling != 1 and every EC factor equal
    let mut late = false;
    if jf.ups != 1 {
        late = true;
        for e in 0..jx.nec as usize {
            if jf.ecups[e] != jf.ups {
                late = false;
            }
        }
    }
    if !late {
        for e in 0..jx.nec as usize {
            let u = jf.ecups[e];
            if u == 1 {
                continue;
            }
            let r = upsw(jx, &mut jrn.p[3 + e], u.trailing_zeros());
            if r != 0 {
                return r;
            }
        }
    }
    if !jf.patch.is_empty() {
        let r = crate::blend::jr_patches(jx, jf, jrn);
        if r != 0 {
            return r;
        }
    }
    if let Some(sp) = jf.splines.as_ref() {
        sp.add_to(&mut jrn.p); // jxl-tools: splines after the patches, before upsampling
    }
    if jf.ups != 1 {
        let sh = jf.ups.trailing_zeros();
        let n = if late { 3 + jx.nec } else { 3 };
        for c in 0..n as usize {
            let r = upsw(jx, &mut jrn.p[c], sh);
            if r != 0 {
                return r;
            }
        }
    }
    // every plane: crop to the upsampled frame size
    for p in jrn.p.iter_mut() {
        p.w = jf.wup.min(p.w);
        p.h = jf.hup.min(p.h);
    }
    if jf.flags & JFF_NOISE != 0 {
        let r = crate::noise::jr_noise(jx, jf, jrn);
        if r != 0 {
            return r;
        }
    }
    if jf.typ == JFT_DC {
        // jxl-plus: DC frames end here (XYB / the frame's colour space, not displayed)
        match crate::blend::jr_saveref(jrn, 1) {
            Ok(s) => jf.newdc = Some(s),
            Err(e) => return e,
        }
        return 0;
    }
    // can_be_referenced: not last, not a DC frame, and duration 0 or saved
    let canref = jf.islast == 0 && jf.typ != JFT_DC && (jf.duration == 0 || jf.saveref != 0);
    if canref && jf.savebct != 0 {
        // reference frame before the colour transform
        match crate::blend::jr_saveref(jrn, 1) {
            Ok(s) => jf.newref = Some(s),
            Err(e) => return e,
        }
    }
    let mut lin = false;
    let mut toout = false;
    if jf.ct == JCT_YCBCR {
        ycc(jrn);
    } else if jf.ct == JCT_XYB {
        // to_output: no ICC, (not gray or D65), gamma or a supported transfer function
        let mut to = false;
        if jx.wanticc == 0 && (jx.cspace != JCS_GRAY || jx.white == 1) {
            let tf = jx.tf;
            if jx.havegamma != 0
                || tf == JTF_709
                || tf == JTF_LINEAR
                || tf == JTF_SRGB
                || tf == JTF_PQ
                || tf == JTF_DCI
                || tf == JTF_HLG
            {
                to = true;
            }
        }
        if to {
            toout = true;
            if jx.cspace != JCS_GRAY && (jx.prim != 1 || jx.white != 1) {
                return ERR_UNSUPPORTED; // XYB to other primaries / white points
            }
        }
        xyb(jx, jrn);
        lin = true;
    }
    // blending: regular / skip-progressive frames with a custom size or a non-replace mode
    if jf.typ == JFT_REGULAR || jf.typ == JFT_SKIPPROG {
        let mut blend = jf.custom != 0 || jf.bmode != JBM_REPLACE;
        for e in 0..jx.nec as usize {
            if jf.ecbl[e][0] != JBM_REPLACE {
                blend = true;
            }
        }
        if blend {
            let r = tolin(jx, jrn, &mut lin, toout);
            if r != 0 {
                return r;
            }
            let r = crate::blend::jr_blend(jx, jf, jrn);
            if r != 0 {
                return r;
            }
        }
    }
    if canref && jf.savebct == 0 {
        let r = tolin(jx, jrn, &mut lin, toout);
        if r != 0 {
            return r;
        }
        // reference frame after the colour transform
        match crate::blend::jr_saveref(jrn, 0) {
            Ok(s) => jf.newref = Some(s),
            Err(e) => return e,
        }
    }
    // jxl-tools: the writer's Butteraugli round trips keep the linear planes
    let r = tolin(jx, jrn, &mut lin, toout && jf.dbg & JDBG_LINEAR == 0);
    if r != 0 {
        return r;
    }
    // first alpha channel
    for e in 0..jx.nec as usize {
        if jx.ec[e].typ == 0 {
            jrn.alpha = 3 + e as i32;
            break;
        }
    }
    0
}

/// jx_out8: jf.rnd colour + alpha into the ARGB surface (image size)
pub fn jx_out8(jf: &Jf, sf: &mut img_core::Surface) -> i32 {
    let Some(jrn) = jf.rnd.as_ref() else { return ERR_HDR };
    let (sw, sh) = (sf.w as u32, sf.h as u32);
    for c in 0..3 {
        let p = &jrn.p[c];
        if p.w < sw || p.h < sh {
            return ERR_HDR;
        }
    }
    let mut ap: Option<&Jrp> = None;
    if jrn.alpha >= 0 && (jrn.alpha as u32) < jrn.np {
        let p = &jrn.p[jrn.alpha as usize];
        if p.w < sw || p.h < sh {
            return ERR_HDR;
        }
        ap = Some(p);
    }
    let q8 = |v: f32| -> u32 {
        let d = v as f64 * 255.0 + 0.5;
        cvttd(minsd(maxsd(d, 0.0), 255.0)) as u32
    };
    let stride = sf.stride as usize;
    par_bands(alloc::vec![(&mut sf.px[..], stride)], sh as usize, |y0, rows, b| {
        for yy in 0..rows {
            let y = y0 + yy;
            let row = &mut b[0][yy * stride..yy * stride + sw as usize * 4];
            for x in 0..sw as usize {
                let mut px = 0u32;
                for c in 0..3usize {
                    let p = &jrn.p[c];
                    px |= q8(p.d[y * p.s as usize + x]) << ((2 - c) * 8);
                }
                px |= match ap {
                    Some(p) => q8(p.d[y * p.s as usize + x]) << 24,
                    None => 0xFF00_0000,
                };
                row[x * 4..x * 4 + 4].copy_from_slice(&px.to_le_bytes());
            }
        }
    });
    0
}
