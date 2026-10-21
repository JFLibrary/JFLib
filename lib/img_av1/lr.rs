// lr -- AV1 superres upscaling and loop restoration (specification 7.16, 7.17)
//                                                                    [src/img/av1_lr.s]
//
//   superres(h, f) -> 0 / ERR_NOMEM   upscaling process of CdefFrame and CurrFrame into the
//                                     upscaled planes (stride Frame::ups)
//   lr(h, f) -> 0 / ERR_NOMEM         loop restoration process: LrFrame = a copy of
//                                     UpscaledCdefFrame, then Wiener / self guided filtering per
//                                     restoration unit ((plane, stripe) pairs in parallel)
//
// The filters run on whole stripe x unit areas (the specification notes that larger blocks
// give the same result within one unit; StripeStartY / StripeEndY only depend on the stripe).
// The source samples of an area (get_source_sample, 3 samples around it) are gathered into a
// padded i32 block first.  Each worker slot (plat::thr_self) has its scratch block; each
// (plane, stripe) item owns the LrFrame rows of its stripe.

use alloc::vec::Vec;

use img_core::ERR_NOMEM;
use rt::sync::SpinLock;

use crate::dec::{zvec, Frame, Hdr, Lru};
use crate::tab::*;

const LR_PW: usize = 2 * 256 + 8; // padded row length (a last unit may be < 2 units)
const LR_PH: usize = 64 + 8;
const LR_N: usize = LR_PH * LR_PW;
const RESTORE_WIENER: u8 = 1;
const RESTORE_SGRPROJ: u8 = 2;

// ------------------------------------------------------------------ superres

/// av1_superres
pub(crate) fn superres(h: &Hdr, f: &mut Frame) -> i32 {
    for p in 0..h.np as usize {
        let (sx, sy) = if p > 0 { (h.ssx, h.ssy) } else { (0, 0) };
        let down = (h.fw + sx) >> sx; // downscaledPlaneW = Round2( FrameWidth, subX )
        let up = (h.upw + sx) >> sx; // upscaledPlaneW
        let ph = (h.fh + sy) >> sy; // planeH
        let maxx = (((h.micols >> sx) << 2) as i32).wrapping_sub(1);
        // stepX = ( ( down << 14 ) + up / 2 ) / up
        let step = (down.wrapping_shl(14).wrapping_add(up >> 1)) / up;
        // err = up * stepX - ( down << 14 )
        let err = up.wrapping_mul(step).wrapping_sub(down.wrapping_shl(14)) as i32;
        // initialSubpelX = ( -( ( up - down ) << 13 ) + up / 2 ) / up + ( 1 << 7 ) - err / 2
        let a = (up.wrapping_sub(down).wrapping_shl(13) as i32).wrapping_neg().wrapping_add((up >> 1) as i32);
        let init = (a / up as i32).wrapping_add(128).wrapping_sub(err / 2) & 0x3FFF;
        // destination stride, planes
        let ups = (up + 63) & !63;
        f.ups[p] = ups;
        let n = ups as usize * f.ph[p] as usize;
        let Ok(uc) = zvec::<u16>(n) else { return ERR_NOMEM };
        f.ucpl[p] = uc;
        let Ok(u) = zvec::<u16>(n) else { return ERR_NOMEM };
        f.upl[p] = u;
        let maxv = (1i32 << h.bd) - 1;
        let sst = f.ps[p] as usize;
        let dst_st = ups as usize;
        // both frames
        for pass in 0..2 {
            let Frame { pl, cpl, cdef_own, upl, ucpl, .. } = &mut *f;
            let (src, dst): (&[u16], &mut [u16]) = if pass == 0 {
                (if *cdef_own { &cpl[p] } else { &pl[p] }, &mut ucpl[p])
            } else {
                (&pl[p], &mut upl[p])
            };
            for y in 0..ph as usize {
                let srow = &src[y * sst..];
                let drow = &mut dst[y * dst_st..];
                for x in 0..up {
                    let sxp = x.wrapping_mul(step).wrapping_add(init as u32).wrapping_sub(1 << 14) as i32; // srcX
                    let px = sxp >> 14; // srcXPx
                    let sub = ((sxp & 0x3FFF) >> 8) as usize; // srcXSubpel
                    let flt = &UPSCALE_FILTER[sub * 8..sub * 8 + 8];
                    let mut sum = 0i32;
                    for (k, &fk) in flt.iter().enumerate() {
                        let q = (px + k as i32 - 3).max(0).min(maxx);
                        sum = sum.wrapping_add((srow[q as usize] as i32).wrapping_mul(fk as i32));
                    }
                    drow[x as usize] = ((sum.wrapping_add(64) >> 7).max(0).min(maxv)) as u16;
                }
            }
        }
    }
    0
}

// ------------------------------------------------------------------ loop restoration

/// LA: area description for the filters
struct La {
    x0: u32, // area in plane samples
    y0: u32,
    w: u32,
    h: u32,
    sstart: i32, // StripeStartY / StripeEndY
    send: i32,
    pex: i32, // PlaneEndX / PlaneEndY
    pey: i32,
    unit: Lru,
    stride: usize,
    maxv: i32,
}

/// the scratch block of a worker slot: P, two work arrays, two filtered outputs
struct Scr {
    p: Vec<i32>,
    w1: Vec<i32>,
    w2: Vec<i32>,
    f0: Vec<i32>,
    f1: Vec<i32>,
}

/// av1_lr
pub(crate) fn lr(h: &Hdr, f: &mut Frame) -> i32 {
    let np = h.np as usize;
    // LrFrame = copy of UpscaledCdefFrame
    for p in 0..np {
        let n = f.ups[p] as usize * f.ph[p] as usize;
        let mut v: Vec<u16> = Vec::new();
        if v.try_reserve_exact(n).is_err() {
            return ERR_NOMEM;
        }
        v.extend_from_slice(&f.upcdef_pl(p)[..n]);
        f.lrpl[p] = v;
    }
    // stripes per plane: stripeNum while Max( 0, StripeStartY ) <= PlaneEndY
    for p in 0..3 {
        f.lrns[p] = 0;
        if p >= np || h.frt[p] == 0 {
            continue;
        }
        let sy = if p > 0 { h.ssy } else { 0 };
        let pey = ((h.fh + sy) >> sy) as i32 - 1;
        let mut n = 0u32;
        while ((n as i32 * 64 - 8) >> sy).max(0) <= pey {
            n += 1;
        }
        f.lrns[p] = n;
    }
    // scratch: one block per thread slot
    let slots = plat::thr_count() as usize + 1;
    let mut scr: Vec<SpinLock<Scr>> = Vec::new();
    for _ in 0..slots {
        let mk = || zvec::<i32>(LR_N);
        let (Ok(p), Ok(w1), Ok(w2), Ok(f0), Ok(f1)) = (mk(), mk(), mk(), mk(), mk()) else { return ERR_NOMEM };
        scr.push(SpinLock::new(Scr { p, w1, w2, f0, f1 }));
    }
    let Frame { pl, cpl, cdef_own, upl, ucpl, sr, ups, lrpl, lru, lrurows, lrucols, lrns, .. } = f;
    let (sr, cdef_own) = (*sr, *cdef_own);
    // the sources of a plane: [UpscaledCdefFrame, UpscaledCurrFrame]
    let srcs: [[&[u16]; 2]; 3] = core::array::from_fn(|p| {
        let up: &[u16] = if sr { &upl[p] } else { &pl[p] };
        let upc: &[u16] = if sr {
            &ucpl[p]
        } else if cdef_own {
            &cpl[p]
        } else {
            &pl[p]
        };
        [upc, up]
    });
    // the LrFrame rows of every (plane, stripe) pair
    let mut jobs: Vec<(usize, u32, SpinLock<&mut [u16]>)> = Vec::new();
    for (p, plane) in lrpl.iter_mut().enumerate().take(np) {
        let n = lrns[p];
        let sy = if p > 0 { h.ssy } else { 0 };
        let st = ups[p] as usize;
        let pey = ((h.fh + sy) >> sy) as i32 - 1;
        let mut rest: &mut [u16] = plane;
        let mut at = 0usize; // first row of rest
        for k in 0..n {
            let ss = ((k as i32) * 64 - 8) >> sy;
            let se = ss + (64 >> sy) - 1;
            let y0 = ss.max(0) as usize;
            let y1 = (se.min(pey) + 1) as usize;
            let (_, r) = core::mem::take(&mut rest).split_at_mut((y0 - at) * st);
            let (mine, r) = r.split_at_mut((y1 - y0) * st);
            rest = r;
            at = y1;
            jobs.push((p, k, SpinLock::new(mine)));
        }
    }
    // lr_job: loop restoration of one (plane, stripe) pair
    let item = |i: usize, _: &plat::JobCtl| {
        let (p, stripe, ref dst) = jobs[i];
        let mut d = dst.lock();
        let mut sc = scr[plat::thr_self().min(slots - 1)].lock();
        let (sx, sy) = if p > 0 { (h.ssx, h.ssy) } else { (0, 0) };
        let mut la = La {
            x0: 0,
            y0: 0,
            w: 0,
            h: 0,
            sstart: 0,
            send: 0,
            pex: ((h.upw + sx) >> sx) as i32 - 1, // PlaneEndX = Round2( UpscaledWidth, subX ) - 1
            pey: ((h.fh + sy) >> sy) as i32 - 1,
            unit: Lru::default(),
            stride: ups[p] as usize,
            maxv: (1i32 << h.bd) - 1,
        };
        let us = h.lrsize[p];
        // StripeStartY = ( -8 + stripeNum * 64 ) >> subY
        la.sstart = (stripe as i32 * 64 - 8) >> sy;
        la.send = la.sstart + (64 >> sy) - 1;
        let y0 = la.sstart.max(0);
        if y0 > la.pey {
            return;
        }
        la.y0 = y0 as u32;
        la.h = (la.send.min(la.pey) - y0 + 1) as u32;
        // unitRow = Min( unitRows - 1, ( ( lumaY + 8 ) >> subY ) / unitSize ), lumaY of the stripe
        let ly = (stripe as i32 * 64 - 8).max(0) as u32;
        let urow = (((ly + 8) >> sy) / us).min(lrurows[p] - 1);
        let ucols = lrucols[p];
        for ucol in 0..ucols {
            let x0 = ucol * us;
            if x0 as i32 > la.pex {
                break;
            }
            la.x0 = x0;
            let x1 = if ucol + 1 < ucols { (la.pex as u32).min(x0 + us - 1) } else { la.pex as u32 };
            la.w = x1 - x0 + 1;
            la.unit = lru[p][(urow * ucols + ucol) as usize];
            match la.unit.typ {
                RESTORE_WIENER => {
                    gather(&la, &srcs[p], &mut sc.p);
                    wiener(h, &la, &mut sc, &mut d);
                }
                RESTORE_SGRPROJ => {
                    gather(&la, &srcs[p], &mut sc.p);
                    sgr(h, &la, &mut sc, &mut d);
                }
                _ => {}
            }
        }
    };
    let _ = plat::parallel_for(jobs.len(), &item);
    0
}

/// lr_gather: P[ r ][ c ] = get_source_sample( plane, x0 + c - 3, y0 + r - 3 ) for r < h + 6,
/// c < w + 6 (src = [UpscaledCdefFrame, UpscaledCurrFrame] of the plane)
fn gather(la: &La, src: &[&[u16]; 2], p: &mut [i32]) {
    for r in 0..(la.h + 6) as i32 {
        let mut y = (la.y0 as i32 + r - 3).max(0).min(la.pey);
        let mut s = src[0];
        if y < la.sstart {
            y = y.max(la.sstart - 2);
            s = src[1];
        } else if y > la.send {
            y = y.min(la.send + 2);
            s = src[1];
        }
        let row = &s[y as usize * la.stride..];
        let pr = &mut p[r as usize * LR_PW..];
        for c in 0..(la.w + 6) as i32 {
            let x = (la.x0 as i32 + c - 3).max(0).min(la.pex);
            pr[c as usize] = row[x as usize] as i32;
        }
    }
}

/// lr_wiener: Wiener filter process of the area (source P) into dst (the stripe's LrFrame rows)
fn wiener(h: &Hdr, la: &La, sc: &mut Scr, dst: &mut [u16]) {
    // filters from the 3 coded coefficients (pass 0 vertical, pass 1 horizontal)
    let mut vf = [0i32; 7];
    let mut hf = [0i32; 7];
    for pass in 0..2 {
        let f = if pass == 0 { &mut vf } else { &mut hf };
        f[3] = 128;
        for c in 0..3 {
            let v = la.unit.wiener[pass][c] as i32;
            f[c] = v;
            f[6 - c] = v;
            f[3] -= 2 * v;
        }
    }
    let (r0, r1) = if h.bd == 12 { (5u32, 9u32) } else { (3, 11) };
    let off = 1i32 << (h.bd + 7 - r0 - 1); // offset = 1 << ( BitDepth + 7 - InterRound0 - 1 )
    let lim = (1i32 << (h.bd + 7 - r0 + 1)) - 1; // limit = ( 1 << ( BitDepth + 1 + 7 - InterRound0 ) ) - 1
    let rnd0 = (1i32 << r0) >> 1;
    // intermediate[ r ][ c ] (W1) for r < h + 6, c < w
    let (w, hh) = (la.w as usize, la.h as usize);
    for r in 0..hh + 6 {
        let pr = &sc.p[r * LR_PW..r * LR_PW + w + 6];
        let ir = &mut sc.w1[r * LR_PW..r * LR_PW + w];
        for (c, o) in ir.iter_mut().enumerate() {
            let mut s = 0i32;
            for t in 0..7 {
                s = s.wrapping_add(pr[c + t].wrapping_mul(hf[t]));
            }
            let v = s.wrapping_add(rnd0) >> r0; // Round2( s, InterRound0 )
            *o = v.max(-off).min(lim - off);
        }
    }
    // LrFrame[ y0 + r ][ x0 + c ] = Clip1( Round2( sum vfilter[ t ] * intermediate[ r + t ][ c ], InterRound1 ) )
    let rnd1 = (1i32 << r1) >> 1;
    for r in 0..hh {
        let drow = &mut dst[r * la.stride + la.x0 as usize..];
        for (c, o) in drow[..w].iter_mut().enumerate() {
            let mut s = 0i32;
            for t in 0..7 {
                s = s.wrapping_add(sc.w1[(r + t) * LR_PW + c].wrapping_mul(vf[t]));
            }
            *o = (s.wrapping_add(rnd1) >> r1).max(0).min(la.maxv) as u16;
        }
    }
}

/// lr_box: box filter process of one pass into f (i32, row length LR_PW); false = r is 0 (no
/// output)
fn boxf(h: &Hdr, la: &La, pass: usize, p: &[i32], a: &mut [i32], b: &mut [i32], f: &mut [i32]) -> bool {
    let set = (la.unit.set & 15) as usize;
    let r = SGR_PARAMS[set * 4 + pass * 2] as i32;
    let eps = SGR_PARAMS[set * 4 + pass * 2 + 1] as u32;
    if r == 0 {
        return false;
    }
    let n = ((2 * r + 1) * (2 * r + 1)) as u32;
    let n2e = n.wrapping_mul(n).wrapping_mul(eps);
    let s = ((1u32 << 20) + (n2e >> 1)) / n2e;
    let one = ((1u32 << 12) + (n >> 1)) / n; // oneOverN
    let bd8 = h.bd - 8;
    let (w, hh) = (la.w as usize, la.h as usize);
    // A and B for i = -1 .. h, j = -1 .. w at [ i + 1 ][ j + 1 ]
    for i1 in 0..hh + 2 {
        for j1 in 0..w + 2 {
            // sums over dy, dx = -r .. r of P[ i + dy + 3 ][ j + dx + 3 ]
            let mut sa = 0u32;
            let mut sb = 0u32;
            for dy in -r..=r {
                let row = &p[(i1 as i32 + dy + 2) as usize * LR_PW..];
                for dx in -r..=r {
                    let c = row[(j1 as i32 + dx + 2) as usize] as u32;
                    sb = sb.wrapping_add(c);
                    sa = sa.wrapping_add(c.wrapping_mul(c));
                }
            }
            // a = Round2( a, 2 * ( BitDepth - 8 ) ), d = Round2( b, BitDepth - 8 )
            let sa = sa.wrapping_add((1u32 << (2 * bd8)) >> 1) >> (2 * bd8);
            let d = sb.wrapping_add((1u32 << bd8) >> 1) >> bd8;
            let pp = (sa.wrapping_mul(n) as i32).wrapping_sub(d.wrapping_mul(d) as i32).max(0) as u32;
            let z = ((pp as u64) * (s as u64) + (1 << 19)) >> 20;
            let a2 = if z >= 255 {
                256u32
            } else if z == 0 {
                1
            } else {
                let z = z as u32;
                ((z << 8) + (z >> 1)) / (z + 1)
            };
            a[i1 * LR_PW + j1] = a2 as i32;
            let bb = ((((256 - a2).wrapping_mul(sb)) as u64 * one as u64 + (1 << 11)) >> 12) as i32;
            b[i1 * LR_PW + j1] = bb;
        }
    }
    // F[ i ][ j ] from the 3x3 neighbourhood of A, B
    for i in 0..hh {
        let shift = if pass == 0 && i & 1 != 0 { 4 } else { 5 };
        for j in 0..w {
            let mut sa = 0i32;
            let mut sb = 0i32;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let wt = if pass == 0 {
                        if (i as i32 + dy) & 1 != 0 {
                            if dx == 0 { 6 } else { 5 }
                        } else {
                            0
                        }
                    } else if dy == 0 || dx == 0 {
                        4
                    } else {
                        3
                    };
                    let q = (i as i32 + dy + 1) as usize * LR_PW + (j as i32 + dx + 1) as usize;
                    sa = sa.wrapping_add(a[q].wrapping_mul(wt));
                    sb = sb.wrapping_add(b[q].wrapping_mul(wt));
                }
            }
            // v = a * UpscaledCdefFrame[ y ][ x ] + b (= P[ i + 3 ][ j + 3 ])
            let v = sa.wrapping_mul(p[(i + 3) * LR_PW + j + 3]).wrapping_add(sb);
            let sh = shift + 4; // SGRPROJ_SGR_BITS + shift - SGRPROJ_RST_BITS
            f[i * LR_PW + j] = v.wrapping_add((1i32 << sh) >> 1) >> sh;
        }
    }
    true
}

/// lr_sgr: self guided filter process of the area into dst
fn sgr(h: &Hdr, la: &La, sc: &mut Scr, dst: &mut [u16]) {
    let Scr { p, w1, w2, f0, f1 } = sc;
    let r0 = boxf(h, la, 0, p, w1, w2, f0);
    let r1 = boxf(h, la, 1, p, w1, w2, f1);
    let w0 = la.unit.xqd[0] as i32;
    let wq1 = la.unit.xqd[1] as i32;
    let wq2 = 128 - w0 - wq1;
    for i in 0..la.h as usize {
        let drow = &mut dst[i * la.stride + la.x0 as usize..];
        for (j, o) in drow[..la.w as usize].iter_mut().enumerate() {
            let u = p[(i + 3) * LR_PW + j + 3].wrapping_shl(4);
            let mut v = wq1.wrapping_mul(u);
            let a = if r0 { f0[i * LR_PW + j] } else { u };
            v = v.wrapping_add(a.wrapping_mul(w0));
            let b = if r1 { f1[i * LR_PW + j] } else { u };
            v = v.wrapping_add(b.wrapping_mul(wq2));
            // Round2( v, SGRPROJ_RST_BITS + SGRPROJ_PRJ_BITS )
            *o = (v.wrapping_add(1 << 10) >> 11).max(0).min(la.maxv) as u16;
        }
    }
}
