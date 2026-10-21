// mtr -- JPEG XL modular inverse transforms (modular/transform/rct.cc, palette.cc,
// squeeze.cc; model: tools/jxl_ref.py ModImage.undo_transforms, inv_rct, inv_palette,
// inv_squeeze, unsqueeze, smooth_tendency, palette_value)                 [src/img/jxl_mtr.s]
//
//   jm_undo(mi, wh) -> 0 / ERR_*    undo the transforms of mi (last first), wh = WP header of
//                                   the group (palette predictor 6)

use alloc::vec::Vec;

use img_core::{ERR_HDR, ERR_LIMIT};

use crate::modular::{jm_predict, zvec, Mc, Mi, Mt, Wp, MTR_PALETTE, MTR_RCT};
use crate::tab;

/// jm_undo: undo the transforms of mi, last first
pub fn jm_undo(mi: &mut Mi, wh: &[u32; 11]) -> i32 {
    while let Some(mt) = mi.tr.last() {
        let mt = mt.clone();
        let r = match mt.id {
            MTR_RCT => jt_rct(mi, mt.begin, mt.rct),
            MTR_PALETTE => jt_palette(mi, &mt, wh),
            _ => jt_squeeze(mi, &mt),
        };
        if r != 0 {
            return r;
        }
        mi.tr.pop();
    }
    0
}

// ---------------------------------------------------------------- RCT

/// jt_rct: inverse RCT of channels m .. m + 2
fn jt_rct(mi: &mut Mi, m: u32, typ: u32) -> i32 {
    let r = mi.checkeq(m, m.wrapping_add(2));
    if r != 0 {
        return r;
    }
    if typ == 0 {
        return 0;
    }
    let perm = typ / 7;
    let custom = typ % 7;
    let m = m as usize;
    let (c0, rest) = mi.ch[m..].split_at_mut(1);
    let (c1, c2) = rest.split_at_mut(1);
    let (c0, c1, c2) = (&mut c0[0], &mut c1[0], &mut c2[0]);
    let n = c0.w as u64 * c0.h as u64;
    if custom != 0 && n != 0 {
        let n = n as usize;
        let (a, b, c) = (&mut c0.d[..n], &mut c1.d[..n], &mut c2.d[..n]);
        if custom == 6 {
            for i in 0..n {
                let y = a[i];
                let co = b[i];
                let cg = c[i];
                let tmp = y.wrapping_sub(cg >> 1);
                let g = cg.wrapping_add(tmp);
                let bb = tmp.wrapping_sub(co >> 1);
                let r = bb.wrapping_add(co);
                a[i] = r;
                b[i] = g;
                c[i] = bb;
            }
        } else {
            let second = custom >> 1;
            let third = custom & 1;
            for i in 0..n {
                let f = a[i];
                let mut t = c[i];
                if third != 0 {
                    t = t.wrapping_add(f);
                    c[i] = t;
                }
                if second == 1 {
                    b[i] = b[i].wrapping_add(f);
                } else if second == 2 {
                    b[i] = b[i].wrapping_add(t.wrapping_add(f) >> 1);
                }
            }
        }
    }
    // ch[o0], ch[o1], ch[o2] = c0, c1, c2
    let o0 = (perm % 3) as usize;
    let o1 = ((perm + 1 + perm / 3) % 3) as usize;
    let o2 = ((perm + 2 - perm / 3) % 3) as usize;
    let old = [core::mem::take(c0), core::mem::take(c1), core::mem::take(c2)];
    let [a, b, c] = old;
    let mut slots: [Option<Mc>; 3] = [None, None, None];
    slots[o0] = Some(a);
    slots[o1] = Some(b);
    slots[o2] = Some(c);
    for (k, s) in slots.into_iter().enumerate() {
        if let Some(v) = s {
            mi.ch[m + k] = v;
        }
    }
    0
}

// ---------------------------------------------------------------- palette

/// jt_palval: GetPaletteValue (pal = the palette channel)
fn palval(pal: &Mc, index: i32, c: u32, bd: u32) -> i32 {
    let size = pal.w;
    if index < 0 {
        if c >= 3 {
            return 0;
        }
        let i = (!index as u32) % 143;
        let k = (((i + 1) >> 1) * 3 + c) as usize;
        let mut v = tab::DELTAPAL[k] as i32;
        if i & 1 == 0 {
            v = v.wrapping_neg();
        }
        if bd > 8 {
            v = v.wrapping_shl(bd - 8);
        }
        return v;
    }
    let index = index as u32;
    if index < size {
        let k = c as u64 * size as u64 + index as u64;
        return pal.d.get(k as usize).copied().unwrap_or(0);
    }
    if c >= 3 {
        return 0;
    }
    let mut i = index - size;
    let max = (1u32 << (bd & 31)).wrapping_sub(1);
    if i < 64 {
        i = (i >> (c * 2)) & 3;
        let s = if bd >= 3 { bd - 3 } else { 0 };
        return ((max.wrapping_mul(i) >> 2).wrapping_add(1u32 << (s & 31))) as i32;
    }
    i -= 64;
    if c >= 1 {
        i /= if c == 2 { 25 } else { 5 };
    }
    i %= 5;
    (max.wrapping_mul(i) >> 2) as i32
}

/// jt_palette: InvPalette
fn jt_palette(mi: &mut Mi, mt: &Mt, wh: &[u32; 11]) -> i32 {
    if mi.nbmeta < 1 {
        return ERR_HDR;
    }
    let nb = mi.ch[0].h;
    let c0 = mt.begin.wrapping_add(1);
    if c0 >= mi.nch() {
        return ERR_HDR;
    }
    let (w, h) = (mi.ch[c0 as usize].w, mi.ch[c0 as usize].h);
    for _ in 1..nb {
        let (hs, vs) = (mi.ch[c0 as usize].hs, mi.ch[c0 as usize].vs);
        let r = mi.insert(c0 + 1, w, h, hs, vs);
        if r != 0 {
            return r;
        }
    }
    let bd = mi.bd.min(24);
    if w != 0 && h != 0 {
        let idx: Vec<i32> = mi.ch[c0 as usize].d.clone();
        let n = w as usize * h as usize;
        let wu = w as usize;
        let mut c = nb;
        while c != 0 {
            c -= 1;
            let oc = (c0 + c) as usize;
            let mut out = core::mem::take(&mut mi.ch[oc].d);
            let pal = &mi.ch[0];
            if mt.nbdelta == 0 && mt.pred == 0 {
                for i in 0..n {
                    let mut index = idx[i];
                    if nb == 1 {
                        let lim = (pal.w as i32).wrapping_sub(1);
                        if index > lim {
                            index = lim;
                        }
                        if index < 0 {
                            index = 0;
                        }
                    }
                    out[i] = palval(pal, index, c, bd);
                }
            } else {
                let mut ws = if mt.pred == 6 {
                    match Wp::new(wh, w) {
                        Ok(v) => Some(v),
                        Err(e) => {
                            mi.ch[oc].d = out;
                            return e;
                        }
                    }
                } else {
                    None
                };
                for y in 0..h {
                    let ro = y as usize * wu;
                    for x in 0..w {
                        let xi = x as usize;
                        let i = ro + xi;
                        let index = idx[i];
                        let mut entry = palval(pal, index, c, bd);
                        if index < mt.nbdelta as i32 {
                            let has_prev = y != 0;
                            let left = if xi > 0 {
                                out[i - 1]
                            } else if has_prev {
                                out[i - wu]
                            } else {
                                0
                            };
                            let top = if has_prev { out[i - wu] } else { left };
                            let tl = if xi > 0 && has_prev { out[i - wu - 1] } else { left };
                            let tr = if xi + 1 < wu && has_prev { out[i - wu + 1] } else { top };
                            let ll = if xi > 1 { out[i - 2] } else { left };
                            let tt = if y > 1 { out[i - 2 * wu] } else { top };
                            let trr = if xi + 2 < wu && has_prev { out[i - wu + 2] } else { tr };
                            let mut wpp = 0i64;
                            if let Some(wsr) = ws.as_mut() {
                                let nbw = [top, left, tr, tl, tt];
                                wpp = wsr.predict(x, y, &nbw, None);
                            }
                            let nb7 = [left, top, tl, tr, ll, tt, trr];
                            entry = entry.wrapping_add(jm_predict(mt.pred, &nb7, wpp) as i32);
                        }
                        out[i] = entry;
                        if let Some(wsr) = ws.as_mut() {
                            wsr.update(entry, x, y);
                        }
                    }
                }
            }
            mi.ch[oc].d = out;
        }
    }
    if c0 >= mi.nbmeta {
        mi.nbmeta = mi.nbmeta.wrapping_sub(1);
    } else {
        mi.nbmeta = mi.nbmeta.wrapping_add(nb).wrapping_sub(2);
    }
    mi.delete(0, 1);
    0
}

// ---------------------------------------------------------------- squeeze

/// jt_tendency: SmoothTendency (64 bit)
#[inline]
fn tendency(b: i64, a: i64, n: i64) -> i64 {
    let up = if b >= a {
        if a >= n {
            true
        } else if b != a {
            return 0;
        } else {
            false
        }
    } else {
        if a > n {
            return 0;
        }
        false
    };
    if up {
        let mut d = (b.wrapping_mul(4).wrapping_sub(n.wrapping_mul(3)).wrapping_sub(a).wrapping_add(6)) / 12;
        let ba2 = b.wrapping_sub(a).wrapping_mul(2);
        if d.wrapping_sub(d & 1) > ba2 {
            d = ba2.wrapping_add(1);
        }
        let an2 = a.wrapping_sub(n).wrapping_mul(2);
        if d.wrapping_add(d & 1) > an2 {
            return an2;
        }
        d
    } else {
        let mut d = (b.wrapping_mul(4).wrapping_sub(n.wrapping_mul(3)).wrapping_sub(a).wrapping_sub(6)) / 12;
        let ba2 = b.wrapping_sub(a).wrapping_mul(2);
        if d.wrapping_add(d & 1) < ba2 {
            d = ba2.wrapping_sub(1);
        }
        let an2 = a.wrapping_sub(n).wrapping_mul(2);
        if d.wrapping_sub(d & 1) < an2 {
            return an2;
        }
        d
    }
}

/// the truncating half of diff plus avg, and the second sample (low 32 bits each)
#[inline(always)]
fn unsq_pair(avg: i64, diff: i64) -> (i32, i32) {
    let a = ((diff.wrapping_add((diff as u64 >> 63) as i64)) >> 1).wrapping_add(avg);
    (a as i32, a.wrapping_sub(diff) as i32)
}

/// jt_unsqueeze: InvHSqueeze / InvVSqueeze of res into avg
fn unsqueeze(avg: &mut Mc, res: &Mc, horizontal: bool) -> i32 {
    let (ow, oh);
    if horizontal {
        if res.w == 0 {
            avg.hs = avg.hs.wrapping_sub(1);
            return 0;
        }
        ow = avg.w.wrapping_add(res.w);
        oh = avg.h;
        if oh != res.h && res.h != 0 {
            return ERR_HDR;
        }
    } else {
        if res.h == 0 {
            avg.vs = avg.vs.wrapping_sub(1);
            return 0;
        }
        ow = avg.w;
        if ow != res.w && res.w != 0 {
            return ERR_HDR;
        }
        oh = avg.h.wrapping_add(res.h);
    }
    let n = ow as u64 * oh as u64;
    let mut out: Vec<i32> = Vec::new();
    if n != 0 {
        if n > 0x7FFF_FFFF {
            return ERR_LIMIT;
        }
        out = match zvec::<i32>(n as usize) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let aw = avg.w as usize;
        let owu = ow as usize;
        // jxl-plus: large channels on the worker pool (rows / column bands are independent)
        let par = n >= 1 << 16;
        if horizontal {
            if res.h != 0 {
                let rw = res.w as usize;
                let row = |y: usize, orow: &mut [i32]| {
                    let ar = &avg.d[y * aw..y * aw + aw];
                    let rr = &res.d[y * rw..y * rw + rw];
                    for x in 0..rw {
                        let a = ar[x] as i64;
                        let nx = if x + 1 < aw { ar[x + 1] as i64 } else { a };
                        let left = if x > 0 { orow[2 * x - 1] as i64 } else { a };
                        let diff = tendency(left, a, nx).wrapping_add(rr[x] as i64);
                        let (p, q) = unsq_pair(a, diff);
                        orow[2 * x] = p;
                        orow[2 * x + 1] = q;
                    }
                    if ow & 1 != 0 {
                        orow[owu - 1] = ar[aw - 1];
                    }
                };
                if par {
                    crate::rnd::par_bands(alloc::vec![(&mut out[..], owu)], avg.h as usize, |y0, nr, b| {
                        for yy in 0..nr {
                            row(y0 + yy, &mut b[0][yy * owu..yy * owu + owu]);
                        }
                    });
                } else {
                    for y in 0..avg.h as usize {
                        row(y, &mut out[y * owu..y * owu + owu]);
                    }
                }
            }
        } else if res.w != 0 {
            let ah = avg.h as usize;
            let rh = res.h as usize;
            // the columns x0 .. x0 + bw of every output row into o (row stride bw)
            let cols = |x0: usize, bw: usize, o: &mut [i32]| {
                for y in 0..rh {
                    for x in x0..x0 + bw {
                        let a = avg.d[y * aw + x] as i64;
                        let top = if y > 0 { o[(2 * y - 1) * bw + x - x0] as i64 } else { a };
                        let nx = if y + 1 < ah { avg.d[(y + 1) * aw + x] as i64 } else { avg.d[y * aw + x] as i64 };
                        let diff = tendency(top, a, nx).wrapping_add(res.d[y * aw + x] as i64);
                        let (p, q) = unsq_pair(a, diff);
                        o[2 * y * bw + x - x0] = p;
                        o[(2 * y + 1) * bw + x - x0] = q;
                    }
                }
            };
            if par {
                const CB: usize = 64;
                let nb = aw.div_ceil(CB);
                let bands: Vec<Vec<i32>> = crate::par::par_map(nb, true, |b| {
                    let bw = CB.min(aw - b * CB);
                    let mut o = alloc::vec![0i32; bw * 2 * rh];
                    cols(b * CB, bw, &mut o);
                    o
                });
                for (b, o) in bands.iter().enumerate() {
                    let bw = CB.min(aw - b * CB);
                    for r in 0..2 * rh {
                        out[r * aw + b * CB..r * aw + b * CB + bw].copy_from_slice(&o[r * bw..r * bw + bw]);
                    }
                }
            } else {
                cols(0, aw, &mut out);
            }
            if oh & 1 != 0 {
                let s = (ah - 1) * aw;
                let d = (oh as usize - 1) * aw;
                out[d..d + aw].copy_from_slice(&avg.d[s..s + aw]);
            }
        }
    }
    avg.d = out;
    avg.w = ow;
    avg.h = oh;
    if horizontal {
        avg.hs = avg.hs.wrapping_sub(1);
    } else {
        avg.vs = avg.vs.wrapping_sub(1);
    }
    0
}

/// jt_squeeze: inverse squeeze steps, last first
fn jt_squeeze(mi: &mut Mi, mt: &Mt) -> i32 {
    for st in mt.sq.iter().rev() {
        let bc = st[2];
        let nc = st[3];
        let ec = bc.wrapping_add(nc).wrapping_sub(1);
        if ec >= mi.nch() {
            return ERR_HDR;
        }
        let offset = if st[1] != 0 {
            ec + 1
        } else {
            mi.nch().wrapping_add(bc).wrapping_sub(ec).wrapping_sub(1)
        };
        if offset.wrapping_add(nc) > mi.nch() {
            return ERR_HDR;
        }
        if bc < mi.nbmeta {
            mi.nbmeta = mi.nbmeta.wrapping_sub(nc);
        }
        let mut c = bc;
        while c <= ec {
            let rc = (c - bc + offset) as usize;
            let ci = c as usize;
            if rc >= mi.ch.len() {
                return ERR_HDR;
            }
            let r = if rc > ci {
                let (lo, hi) = mi.ch.split_at_mut(rc);
                let avg = &mut lo[ci];
                let res = &hi[0];
                if avg.w < res.w || avg.h < res.h {
                    return ERR_HDR;
                }
                unsqueeze(avg, res, st[0] != 0)
            } else {
                // (a damaged stream: the residual is the channel itself or before it)
                let res = mi.ch[rc].clone();
                let avg = &mut mi.ch[ci];
                if avg.w < res.w || avg.h < res.h {
                    return ERR_HDR;
                }
                unsqueeze(avg, &res, st[0] != 0)
            };
            if r != 0 {
                return r;
            }
            c += 1;
        }
        mi.delete(offset, nc);
    }
    0
}
