// noise -- JPEG XL noise synthesis (dec_noise.cc, stage_noise.cc; model: tools/jxl_ref.py
// Xorshift128Plus, random_rect, noise_planes, convolve_noise, noise_strength, add_noise)
//                                                                      [src/img/jxl_noise.s]
//
//   jx_count(jx, jf)                   visible / invisible frame counters for a parsed frame
//                                      header (every frame in stream order, the preview too)
//   jr_noise(jx, jf, jrn) -> 0 / ERR_* AddNoise on the colour planes (upsampled frame size)

use alloc::vec::Vec;

use rt::fx::{cvtt, maxss, minss};

use crate::frm::{Jf, JFT_REGULAR, JFT_SKIPPROG};
use crate::hdr::Jx;
use crate::rnd::{jr_alloc, jr_pad, r4, Jrn, Jrp};
use crate::tab;

/// jx_count: visible / invisible frame counters into jf.vis / jf.nonvis
pub fn jx_count(jx: &mut Jx, jf: &mut Jf) {
    let vis = jf.preview == 0
        && (jf.typ == JFT_REGULAR || jf.typ == JFT_SKIPPROG)
        && (jf.islast != 0 || jf.duration != 0);
    if vis {
        jx.vis = jx.vis.wrapping_add(1);
        jx.nonvis = 0;
    } else {
        jx.nonvis = jx.nonvis.wrapping_add(1);
    }
    jf.vis = jx.vis;
    jf.nonvis = jx.nonvis;
}

/// ns_split: SplitMix64 finalizer
#[inline]
fn split(z: u64) -> u64 {
    let mut a = (z >> 30) ^ z;
    a = a.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    a ^= a >> 27;
    a = a.wrapping_mul(0x94D0_49BB_1331_11EB);
    a ^ (a >> 31)
}

/// xorshift128+ generators (8 lanes)
struct Ns {
    s0: [u64; 8],
    s1: [u64; 8],
    buf: [u32; 16],
}

impl Ns {
    /// ns_seed
    fn seed(a: u32, b: u32, c: u32, d: u32) -> Ns {
        const G: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut n = Ns { s0: [0; 8], s1: [0; 8], buf: [0; 16] };
        n.s0[0] = split(((a as u64) << 32).wrapping_add(b as u64).wrapping_add(G));
        n.s1[0] = split(((c as u64) << 32).wrapping_add(d as u64).wrapping_add(G));
        for i in 1..8 {
            n.s0[i] = split(n.s0[i - 1]);
            n.s1[i] = split(n.s1[i - 1]);
        }
        n
    }

    /// ns_fill: 16 random u32
    fn fill(&mut self) {
        for i in 0..8 {
            let mut s1 = self.s0[i];
            let s0 = self.s1[i];
            let bits = s1.wrapping_add(s0);
            self.s0[i] = s0;
            s1 ^= s1 << 23;
            s1 ^= s0 ^ (s1 >> 18) ^ (s0 >> 5);
            self.s1[i] = s1;
            self.buf[2 * i] = bits as u32;
            self.buf[2 * i + 1] = (bits >> 32) as u32;
        }
    }

    /// ns_rect: floats in [1, 2) from the bits, each row starts a new batch
    fn rect(&mut self, jp: &mut Jrp, x0: u32, y0: u32, rw: u32, rh: u32) {
        let s = jp.s as usize;
        for y in 0..rh as usize {
            let row = (y0 as usize + y) * s + x0 as usize;
            let mut x = 0u32;
            while x < rw {
                self.fill();
                for k in 0..16u32 {
                    if x + k >= rw {
                        break;
                    }
                    jp.d[row + (x + k) as usize] = f32::from_bits((self.buf[k as usize] >> 9) | 0x3F80_0000);
                }
                x += 16;
            }
        }
    }
}

/// ns_conv: v = (sum of the 24 neighbours) * 0.16 + p * -3.84 (in place)
fn conv(jp: &mut Jrp) -> i32 {
    let pad = match jr_pad(jp, 2) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let (c016, cm384) = (tab::NOISE[0], tab::NOISE[1]);
    let ps = (jp.s + 4) as usize;
    let s = jp.s as usize;
    let wv = r4(jp.w) as usize;
    for y in 0..jp.h as usize {
        let r0 = &pad[y * ps..];
        let r1 = &pad[(y + 1) * ps..];
        let r2 = &pad[(y + 2) * ps..];
        let r3 = &pad[(y + 3) * ps..];
        let r4r = &pad[(y + 4) * ps..];
        for x in 0..wv {
            let mut acc = 0.0f32;
            for i in 0..5 {
                acc = acc + r0[x + i];
                acc = acc + r1[x + i];
                acc = acc + r3[x + i];
                acc = acc + r4r[x + i];
            }
            for i in [0usize, 1, 3, 4] {
                acc = acc + r2[x + i];
            }
            jp.d[y * s + x] = acc * c016 + r2[x + 2] * cm384;
        }
    }
    0
}

/// ns_strength: piecewise linear in the LUT of 8, clamped
#[inline]
fn strength(x: f32, lut: &[f32; 8]) -> f32 {
    let sc = maxss(x * tab::NOISE[6], 0.0);
    let (mut i, frac) = if sc >= tab::NOISE[7] {
        (6i32, 1.0f32)
    } else {
        let t = cvtt(sc);
        (t, sc - t as f32)
    };
    if i as u32 > 6 {
        i = 6;
    }
    let i = i as usize;
    let v = (lut[i + 1] - lut[i]) * frac + lut[i];
    maxss(minss(v, 1.0), 0.0)
}

/// jr_noise: AddNoise on the colour planes
pub fn jr_noise(jx: &Jx, jf: &Jf, jrn: &mut Jrn) -> i32 {
    let _ = jx;
    let mut r: Vec<Jrp> = Vec::with_capacity(3);
    for _ in 0..3 {
        let d = match jr_alloc(jf.wup, jf.hup) {
            Ok(v) => v,
            Err(e) => return e,
        };
        r.push(Jrp { d, w: jf.wup, h: jf.hup, s: r4(jf.wup) });
    }
    let (ups, gd) = (jf.ups, jf.gdim);
    for g in 0..jf.ng {
        let gx = g % jf.xg;
        let gy = g / jf.xg;
        for iy in 0..ups {
            for ix in 0..ups {
                let x0 = (gx.wrapping_mul(ups).wrapping_add(ix)).wrapping_mul(gd);
                if x0 >= jf.wup {
                    continue;
                }
                let y0 = (gy.wrapping_mul(ups).wrapping_add(iy)).wrapping_mul(gd);
                if y0 >= jf.hup {
                    continue;
                }
                let ex = (gx + 1).wrapping_mul(gd).wrapping_mul(ups).min(jf.wup);
                if ex <= x0 {
                    continue;
                }
                let rw = (ex - x0).min(gd);
                let ey = (gy + 1).wrapping_mul(gd).wrapping_mul(ups).min(jf.hup);
                if ey <= y0 {
                    continue;
                }
                let rh = (ey - y0).min(gd);
                let mut ns = Ns::seed(jf.vis, jf.nonvis, x0, y0);
                for p in r.iter_mut() {
                    ns.rect(p, x0, y0, rw, rh);
                }
            }
        }
    }
    for p in r.iter_mut() {
        let e = conv(p);
        if e != 0 {
            return e;
        }
    }
    // colour correlation: modular 0 / 1, VarDCT base factors
    let (mut ytox, mut ytob) = (0.0f32, 1.0f32);
    if jf.modular == 0
        && let Some(jv) = jf.vd.as_ref()
    {
        ytox = jv.basex;
        ytob = jv.baseb;
    }
    let lut = &jf.nlut;
    let [p0, p1, p2, ..] = &mut jrn.p[..] else { return 0 };
    let (s0, s1, s2) = (p0.s as usize, p1.s as usize, p2.s as usize);
    let rs = r[0].s as usize;
    for y in 0..jf.hup.min(p0.h) as usize {
        for x in 0..jf.wup.min(p0.w) as usize {
            let vx = p0.d[y * s0 + x];
            let vy = p1.d[y * s1 + x];
            let g = strength((vy - vx) * tab::NOISE[5], lut);
            let rr = strength((vy + vx) * tab::NOISE[5], lut);
            let k = y * rs + x;
            let nr = r[0].d[k] * tab::NOISE[2];
            let ng = r[1].d[k] * tab::NOISE[2];
            let nc = (r[2].d[k] * tab::NOISE[2]) * tab::NOISE[4];
            let red = (nr * tab::NOISE[3] + nc) * rr;
            let green = (ng * tab::NOISE[3] + nc) * g;
            let rgn = red + green;
            p0.d[y * s0 + x] = (ytox * rgn + (red - green)) + vx;
            p1.d[y * s1 + x] = vy + rgn;
            p2.d[y * s2 + x] = ytob * rgn + p2.d[y * s2 + x];
        }
    }
    0
}
