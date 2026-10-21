// blend -- JPEG XL reference frames, frame blending and patches (blending.cc,
// stage_blending.cc, dec_patch_dictionary.cc; model: tools/jxl_ref.py perform_blending,
// alpha_blend1, alpha_wadd, RefFrame, Frame.blend, read_patches, apply_patches)
//                                                                      [src/img/jxl_blend.s]
//
//   jr_saveref(jrn, xyb) -> Jrn              copy of the planes as a reference frame
//   jr_blend(jx, jf, jrn) -> 0 / ERR_*       the frame on the canvas (image size) over its
//                                            reference frames; the planes of jrn become the
//                                            canvas
//   jx_patches(jx, jf, jb) -> 0 / ERR_*      PatchDictionary::Decode into jf.patch
//   jr_patches(jx, jf, jrn) -> 0 / ERR_*     blend the patches into the planes

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use img_core::{ERR_CODE, ERR_HDR, ERR_UNSUPPORTED};
use rt::fx::{maxss, minss};

use crate::bits::Jb;
use crate::ent::{je_read, Jr};
use crate::frm::Jf;
use crate::hdr::{Jx, Jxe};
use crate::rnd::{jr_alloc, r4, Jrn, Jrp};

pub const PB_NONE: u32 = 0;
pub const PB_REPLACE: u32 = 1;
pub const PB_ADD: u32 = 2;
pub const PB_MUL: u32 = 3;
pub const PB_BLEND_ABOVE: u32 = 4;
pub const PB_BLEND_BELOW: u32 = 5;
pub const PB_AWADD_ABOVE: u32 = 6;
pub const PB_AWADD_BELOW: u32 = 7;
static F2P: [u32; 5] = [PB_REPLACE, PB_ADD, PB_BLEND_ABOVE, PB_AWADD_ABOVE, PB_MUL];

/// a patch (PT): reference frame, rectangle in it, position, blendings (colour, then the
/// extra channels: mode, alpha, clamp)
#[derive(Clone, Default)]
pub struct Patch {
    pub rf: u32,
    pub rx0: u32,
    pub ry0: u32,
    pub rw: u32,
    pub rh: u32,
    pub x: u32,
    pub y: u32,
    pub bl: Vec<[u32; 3]>,
}

/// jr_saveref: copy of the planes as a reference frame
pub fn jr_saveref(jrn: &Jrn, xyb: u32) -> Result<Box<Jrn>, i32> {
    let mut r = Box::new(Jrn { np: jrn.np, alpha: jrn.alpha, xyb, p: Vec::with_capacity(jrn.p.len()) });
    for p in &jrn.p {
        let mut d = jr_alloc(p.w, p.h)?;
        let s = r4(p.w) as usize;
        let w = p.w as usize;
        for y in 0..p.h as usize {
            d[y * s..y * s + w].copy_from_slice(&p.d[y * p.s as usize..y * p.s as usize + w]);
        }
        r.p.push(Jrp { d, w: p.w, h: p.h, s: s as u32 });
    }
    Ok(r)
}

#[inline(always)]
fn clamp01(v: f32) -> f32 {
    maxss(minss(v, 1.0), 0.0)
}

/// pb_ab1: PerformAlphaBlending of one channel; flags: 1 premultiplied, 2 clamp flag
/// (inverted as in libjxl: fga is clamped when it is clear), 4 the channel is the alpha
/// channel itself
#[inline]
fn ab1(bg: f32, bga: f32, fg: f32, fga: f32, flags: u32) -> f32 {
    let mut fa = fga;
    if flags & 2 == 0 {
        fa = clamp01(fa);
    }
    let om = 1.0 - fa;
    if flags & 4 != 0 {
        return 1.0 - om * (1.0 - bga);
    }
    if flags & 1 != 0 {
        return bg * om + fg;
    }
    let t = (1.0 - bga) * om;
    let r = (bg * bga) * om + fg * fa;
    let na = 1.0 - t;
    let inv = if na > 0.0 { 1.0 / na } else { 0.0 };
    r * inv
}

/// pb_wadd: flags 2 clamp, 4 alpha channel itself
#[inline]
fn wadd(bg: f32, fg: f32, fga: f32, flags: u32) -> f32 {
    if flags & 4 != 0 {
        return bg;
    }
    let mut a = fga;
    if flags & 2 != 0 {
        a = clamp01(a);
    }
    bg + fg * a
}

/// jr_pbrow: PerformBlending for n samples of every plane (results into the background rows)
fn pbrow(bg: &mut [&mut [f32]], fg: &[&[f32]], n: usize, ec: &[Jxe], hasa: bool, bl: &[[u32; 3]]) {
    let np = bg.len().min(260);
    let mut b = vec![0f32; np];
    let mut f = vec![0f32; np];
    let mut o = vec![0f32; np];
    for i in 0..n {
        for c in 0..np {
            b[c] = bg[c][i];
            f[c] = fg[c][i];
        }
        let mut e = 0usize;
        while e + 3 < np {
            let [mode, aa, clamp] = bl[e + 1];
            let mut a = aa as usize;
            if (aa as u64) + 3 >= np as u64 {
                a = e;
            }
            let c = e + 3;
            let ca = a + 3;
            let mut flags = 0u32;
            if ec.get(a).is_some_and(|x| x.assoc != 0) {
                flags |= 1;
            }
            if clamp != 0 {
                flags |= 2;
            }
            if e == a {
                flags |= 4;
            }
            o[c] = match mode {
                PB_ADD => b[c] + f[c],
                PB_BLEND_ABOVE => ab1(b[c], b[ca], f[c], f[ca], flags),
                PB_BLEND_BELOW => ab1(f[c], f[ca], b[c], b[ca], flags),
                PB_AWADD_ABOVE => wadd(b[c], f[c], f[ca], flags),
                PB_AWADD_BELOW => wadd(f[c], b[c], b[ca], flags),
                PB_MUL => {
                    let v = if clamp != 0 { clamp01(f[c]) } else { f[c] };
                    b[c] * v
                }
                PB_REPLACE => f[c],
                _ => b[c],
            };
            e += 1;
        }
        // colour: mode, alpha channel, clamp
        let [mode, aa, clamp] = bl[0];
        let mut a = aa as usize;
        if (aa as u64) + 3 >= np as u64 {
            a = 0;
        }
        let add = |o: &mut [f32]| {
            for c in 0..3 {
                o[c] = b[c] + f[c];
            }
        };
        match mode {
            PB_ADD => add(&mut o),
            PB_AWADD_ABOVE | PB_AWADD_BELOW => {
                if hasa {
                    let fl = if clamp != 0 { 2 } else { 0 };
                    for c in 0..3 {
                        o[c] = if mode == PB_AWADD_ABOVE {
                            wadd(b[c], f[c], f[a + 3], fl)
                        } else {
                            wadd(f[c], b[c], b[a + 3], fl)
                        };
                    }
                } else {
                    add(&mut o);
                }
            }
            PB_BLEND_ABOVE | PB_BLEND_BELOW => {
                if hasa {
                    let (lo, hi) = if mode == PB_BLEND_ABOVE { (&b, &f) } else { (&f, &b) };
                    let mut fa = hi[a + 3];
                    if clamp != 0 {
                        fa = clamp01(fa);
                    }
                    let mut fl = 2u32;
                    if ec.get(a).is_some_and(|x| x.assoc != 0) {
                        fl |= 1;
                    }
                    for c in 0..3 {
                        o[c] = ab1(lo[c], lo[a + 3], hi[c], fa, fl);
                    }
                    o[a + 3] = ab1(0.0, lo[a + 3], 0.0, fa, 2 | 4);
                } else {
                    o[..3].copy_from_slice(&f[..3]);
                }
            }
            PB_REPLACE => o[..3].copy_from_slice(&f[..3]),
            PB_MUL => {
                for c in 0..3 {
                    let v = if clamp != 0 { clamp01(f[c]) } else { f[c] };
                    o[c] = b[c] * v;
                }
            }
            _ => o[..3].copy_from_slice(&b[..3]),
        }
        for c in 0..np {
            bg[c][i] = o[c];
        }
    }
}

/// any extra channel is alpha
fn has_alpha(jx: &Jx) -> bool {
    jx.ec.iter().take(jx.nec as usize).any(|e| e.typ == 0)
}

/// jr_blend: the frame on the canvas over its reference frames
pub fn jr_blend(jx: &Jx, jf: &Jf, jrn: &mut Jrn) -> i32 {
    let np = jrn.np as usize;
    let mut cv: Vec<Jrp> = Vec::with_capacity(np);
    for c in 0..np {
        let d = match jr_alloc(jx.w, jx.h) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let mut p = Jrp { d, w: jx.w, h: jx.h, s: r4(jx.w) };
        let src = if c < 3 { jf.bsrc } else { jf.ecbl[c - 3][3] };
        if src >= 4 {
            return ERR_HDR;
        }
        if let Some(r) = jx.refs[src as usize].as_ref() {
            if r.xyb != 0 || c as u32 >= r.np {
                return ERR_UNSUPPORTED;
            }
            let rp = &r.p[c];
            if rp.w < jx.w || rp.h < jx.h {
                return ERR_UNSUPPORTED;
            }
            let w = jx.w as usize;
            for y in 0..jx.h as usize {
                let (ps, rs) = (p.s as usize, rp.s as usize);
                p.d[y * ps..y * ps + w].copy_from_slice(&rp.d[y * rs..y * rs + w]);
            }
        }
        cv.push(p);
    }
    let (fx0, fy0) = if jf.custom != 0 { (jf.x0 as i64, jf.y0 as i64) } else { (0, 0) };
    let cx0 = fx0.max(0);
    let cy0 = fy0.max(0);
    let cx1 = (jrn.p[0].w as i64 + fx0).min(jx.w as i64);
    let cy1 = (jrn.p[0].h as i64 + fy0).min(jx.h as i64);
    if cx1 > cx0 && cy1 > cy0 {
        for p in &jrn.p {
            if (p.w as i64 + fx0) < cx1 || (p.h as i64 + fy0) < cy1 {
                return ERR_UNSUPPORTED;
            }
        }
        let mut bl: Vec<[u32; 3]> = Vec::with_capacity(jx.nec as usize + 1);
        if jf.bmode >= 5 {
            return ERR_HDR;
        }
        bl.push([F2P[jf.bmode as usize], jf.balpha, jf.bclamp]);
        for e in 0..jx.nec as usize {
            let b = jf.ecbl[e];
            if b[0] >= 5 {
                return ERR_HDR;
            }
            bl.push([F2P[b[0] as usize], b[1], b[2]]);
        }
        let hasa = has_alpha(jx);
        let n = (cx1 - cx0) as usize;
        for y in cy0..cy1 {
            let mut bgr: Vec<&mut [f32]> = cv
                .iter_mut()
                .map(|p| {
                    let o = (p.s as i64 * y + cx0) as usize;
                    &mut p.d[o..]
                })
                .collect();
            let fgr: Vec<&[f32]> = jrn
                .p
                .iter()
                .map(|p| {
                    let o = (p.s as i64 * (y - fy0) + cx0 - fx0) as usize;
                    &p.d[o..]
                })
                .collect();
            pbrow(&mut bgr, &fgr, n, &jx.ec, hasa, &bl);
        }
    }
    // the canvas replaces the frame planes
    for (p, c) in jrn.p.iter_mut().zip(cv) {
        *p = c;
    }
    0
}

/// jx_patches: PatchDictionary::Decode into jf.patch
pub fn jx_patches(jx: &Jx, jf: &mut Jf, jb: &mut Jb) -> i32 {
    jf.patch = Vec::new();
    let jc = match je_read(jb, 10, false) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let mut jr = match Jr::new(&jc, jb, 0) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let nref = jr.read(jb, 0);
    if nref as u64 > ((jf.xb as u64 * jf.yb as u64) << 4) + 1024 {
        return ERR_HDR;
    }
    let nec = jx.nec;
    for _ in 0..nref {
        let rf = jr.read(jb, 1);
        if rf >= 4 {
            return ERR_HDR;
        }
        let Some(r) = jx.refs[rf as usize].as_ref() else { return ERR_HDR };
        if r.xyb == 0 {
            return ERR_HDR;
        }
        let rx0 = jr.read(jb, 3);
        let ry0 = jr.read(jb, 3);
        let rw = jr.read(jb, 2).wrapping_add(1);
        if rw == 0 {
            return ERR_HDR;
        }
        let rh = jr.read(jb, 2).wrapping_add(1);
        if rh == 0 {
            return ERR_HDR;
        }
        if rx0 as u64 + rw as u64 > r.p[0].w as u64 || ry0 as u64 + rh as u64 > r.p[0].h as u64 {
            return ERR_HDR;
        }
        let cnt = jr.read(jb, 7).wrapping_add(1);
        if cnt == 0 {
            return ERR_HDR;
        }
        let (mut x, mut y) = (0i64, 0i64);
        for k in 0..cnt {
            if k == 0 {
                x = jr.read(jb, 4) as i64;
                y = jr.read(jb, 4) as i64;
            } else {
                let u = jr.read(jb, 6);
                x = x.wrapping_add(((u >> 1) as i64) ^ (-((u & 1) as i64)));
                let u = jr.read(jb, 6);
                y = y.wrapping_add(((u >> 1) as i64) ^ (-((u & 1) as i64)));
                if x < 0 || y < 0 {
                    return ERR_HDR;
                }
            }
            if x + rw as i64 > (jf.xb as i64) << 3 || y + rh as i64 > (jf.yb as i64) << 3 {
                return ERR_HDR;
            }
            let mut p = Patch { rf, rx0, ry0, rw, rh, x: x as u32, y: y as u32, bl: Vec::with_capacity(nec as usize + 1) };
            for _ in 0..=nec {
                let mode = jr.read(jb, 5);
                if mode >= 8 {
                    return ERR_HDR;
                }
                let mut alpha = 0u32;
                if mode >= PB_BLEND_ABOVE && nec > 1 {
                    alpha = jr.read(jb, 8);
                    if alpha >= nec {
                        return ERR_HDR;
                    }
                }
                let mut clamp = 0u32;
                if mode >= PB_MUL {
                    clamp = (jr.read(jb, 9) != 0) as u32;
                }
                p.bl.push([mode, alpha, clamp]);
            }
            jf.patch.push(p);
        }
    }
    if !jr.fin() {
        return ERR_CODE;
    }
    0
}

/// jr_patches: blend the patches into the planes
pub fn jr_patches(jx: &Jx, jf: &Jf, jrn: &mut Jrn) -> i32 {
    let hasa = has_alpha(jx);
    for pt in &jf.patch {
        let (w0, h0) = (jrn.p[0].w, jrn.p[0].h);
        if pt.x >= w0 || pt.y >= h0 {
            continue;
        }
        let x1 = pt.x.wrapping_add(pt.rw).min(w0);
        let y1 = pt.y.wrapping_add(pt.rh).min(h0);
        let Some(r) = jx.refs[pt.rf as usize & 3].as_ref() else { return ERR_HDR };
        if jrn.np > r.np {
            return ERR_UNSUPPORTED;
        }
        for c in 0..jrn.np as usize {
            let p = &jrn.p[c];
            let rp = &r.p[c];
            if x1 > p.w || y1 > p.h {
                return ERR_UNSUPPORTED;
            }
            if (x1 - pt.x).wrapping_add(pt.rx0) > rp.w || (y1 - pt.y).wrapping_add(pt.ry0) > rp.h {
                return ERR_UNSUPPORTED;
            }
        }
        let n = (x1 - pt.x) as usize;
        let mut yy = 0u32;
        while pt.y + yy < y1 {
            let mut bgr: Vec<&mut [f32]> = jrn
                .p
                .iter_mut()
                .map(|p| {
                    let o = (pt.y + yy) as usize * p.s as usize + pt.x as usize;
                    &mut p.d[o..]
                })
                .collect();
            let fgr: Vec<&[f32]> = r
                .p
                .iter()
                .take(jrn.np as usize)
                .map(|p| {
                    let o = (pt.ry0 + yy) as usize * p.s as usize + pt.rx0 as usize;
                    &p.d[o..]
                })
                .collect();
            pbrow(&mut bgr, &fgr, n, &jx.ec, hasa, &pt.bl);
            yy += 1;
        }
    }
    0
}
