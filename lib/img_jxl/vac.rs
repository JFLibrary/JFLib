// vac -- JPEG XL VarDCT AC side: quantization encodings, coefficient orders, AC histograms and
// coefficients (quant_weights.cc Decode, coeff_order.cc, dec_group.cc DecodeACVarBlock; model:
// tools/jxl_ref.py read_quant_encodings, natural_order, decode_coeff_orders,
// Frame.vardct_ac_global, Frame.vardct_ac_group)                         [src/img/jxl_vac.s]
//
//   jv_acglobal(jx, jf, jb) -> 0 / ERR_*        AC global section; also computes the
//                                               dequantization tables of the strategies in
//                                               use and allocates the pixel planes
//   jv_acgroup(jx, jf, g, pass, jb) -> 0 / ERR_*   coefficients of one pass of group g
//   jv_coefbuf(jv, g) -> index / ERR_NOMEM      coefficient buffer of group g (zeroed on
//                                               first use)
//   jv_natorder(acs, out)                       natural coefficient order

use alloc::vec;
use alloc::vec::Vec;

use img_core::{ERR_CODE, ERR_HDR};

use crate::bits::Jb;
use crate::ent::{ceillog2, je_read, Jr};
use crate::frm::Jf;
use crate::hdr::{unpack_signed, Jx};
use crate::modular::{jm_decode, zvec, Mi};
use crate::tab;
use crate::vdc::{Jv, Qe, ACSCX, ACSCY};

/// coefficients per channel of a group buffer
pub const JV_COEFSZ: usize = 65536;
const ALMOST0: u32 = 0x322bcc77; // 1e-8
static D_ORDER: [u32; 8] = [0x5F, 0, 0x13, 0, 0, 0, 0, 13];
static CORDER: [usize; 3] = [1, 0, 2];

/// |v| < 1e-8 (or NaN)
#[inline]
fn too_small(v: f32) -> bool {
    !(f32::from_bits(v.to_bits() & 0x7FFF_FFFF) >= f32::from_bits(ALMOST0))
}

/// va_dctparams: DctQuantWeightParams into bands[3][16]
fn dctparams(jb: &mut Jb, bands: &mut [f32; 48], nb: &mut u32) -> i32 {
    let n = jb.f(4) + 1;
    *nb = n;
    for c in 0..3 {
        for i in 0..n as usize {
            let mut v = jb.f16f();
            if i == 0 {
                if !(v >= f32::from_bits(ALMOST0)) {
                    return ERR_HDR;
                }
                v *= 64.0;
            }
            bands[c * 16 + i] = v;
        }
    }
    0
}

/// va_qenc: DequantMatrices::Decode into jv.qe
fn qenc(jf: &Jf, jv: &mut Jv, jb: &mut Jb) -> i32 {
    jv.qe = (0..17).map(Qe::library).collect();
    if jb.f(1) != 0 {
        return 0; // all library tables
    }
    for i in 0..17usize {
        let mode = jb.f(3);
        if mode == 0 {
            continue;
        }
        let sx = tab::QTSX[i] as u32;
        let sy = tab::QTSX[i + 17] as u32;
        if mode <= 5 && sx * sy != 1 {
            return ERR_HDR;
        }
        let mut q = Qe { mode, ..Default::default() };
        match mode {
            1 | 2 => {
                let n = if mode == 1 { 9 } else { 18 };
                for k in 0..n {
                    let v = jb.f16f();
                    if too_small(v) {
                        return ERR_HDR;
                    }
                    q.w[k] = v * 64.0;
                }
            }
            3 | 4 => {
                let n = if mode == 3 { 6 } else { 3 };
                for k in 0..n {
                    let v = jb.f16f();
                    if too_small(v) {
                        return ERR_HDR;
                    }
                    q.w[k] = v;
                }
                let r = dctparams(jb, &mut q.b1, &mut q.nb);
                if r != 0 {
                    return r;
                }
            }
            5 => {
                for k in 0..27 {
                    let mut v = jb.f16f();
                    if k % 9 < 6 {
                        v *= 64.0;
                    }
                    q.w[k] = v;
                }
                let r = dctparams(jb, &mut q.b1, &mut q.nb);
                if r != 0 {
                    return r;
                }
                let r = dctparams(jb, &mut q.b2, &mut q.nb2);
                if r != 0 {
                    return r;
                }
            }
            6 => {
                let r = dctparams(jb, &mut q.b1, &mut q.nb);
                if r != 0 {
                    return r;
                }
            }
            _ => {
                // raw: denominator, modular table image
                let den = jb.f16f();
                if !(den >= f32::from_bits(ALMOST0)) {
                    return ERR_HDR;
                }
                q.den = den;
                let w = sx << 3;
                let h = sy << 3;
                let mut mi = Mi::new(w, h, 8);
                while mi.nch() < 3 {
                    let n = mi.nch();
                    let r = mi.insert(n, w, h, 0, 0);
                    if r != 0 {
                        return r;
                    }
                }
                let gtree = if jf.gtree.nodes.is_empty() { None } else { Some(&jf.gtree) };
                let gcode = jf.gcode.as_deref();
                let stream = jf.ndg.wrapping_mul(3).wrapping_add(i as u32).wrapping_add(1);
                let r = jm_decode(jb, &mut mi, stream, 0xFF_FFFF, gtree, gcode, true, None);
                if r != 0 {
                    return r;
                }
                if mi.nch() != 3 {
                    return ERR_HDR;
                }
                let per = (w * h) as usize;
                let mut raw = match zvec::<i32>(per * 3) {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                for p in 0..3 {
                    let ch = &mi.ch[p];
                    if ch.w != w {
                        return ERR_HDR;
                    }
                    for s in 0..per {
                        let v = ch.d.get(s).copied().unwrap_or(0);
                        if v <= 0 {
                            return ERR_HDR;
                        }
                        raw[p * per + s] = v;
                    }
                }
                q.raw = raw;
            }
        }
        jv.qe[i] = q;
    }
    0
}

/// jv_natorder: CoeffOrderAndLut (order variant): scan position -> coefficient index
pub fn jv_natorder(acs: usize, out: &mut [u16]) {
    let mut cx = ACSCX[acs] as u32;
    let mut cy = ACSCY[acs] as u32;
    if cx < cy {
        core::mem::swap(&mut cx, &mut cy);
    }
    let xs = cx / cy;
    let xsm = xs - 1;
    let xss = ceillog2(xs);
    let mut cur = cx * cy;
    let cx8 = cx * 8;
    for i in 0..cx8 {
        for j in 0..=i {
            let (mut x, mut y) = (j, i - j);
            if i & 1 != 0 {
                core::mem::swap(&mut x, &mut y);
            }
            if y & xsm != 0 {
                continue;
            }
            y >>= xss;
            let val = y * cx8 + x;
            let pos = if x < cx && y < cy {
                y * cx + x
            } else {
                let p = cur;
                cur += 1;
                p
            };
            out[pos as usize] = val as u16;
        }
    }
    let mut ip = cx8 - 1;
    while ip != 0 {
        let i = ip - 1;
        for j in 0..=i {
            let (mut x, mut y) = (cx8 - 1 - (i - j), cx8 - 1 - j);
            if i & 1 != 0 {
                core::mem::swap(&mut x, &mut y);
            }
            if y & xsm != 0 {
                continue;
            }
            y >>= xss;
            out[cur as usize] = (y * cx8 + x) as u16;
            cur += 1;
        }
        ip -= 1;
    }
}

/// va_ctx: min(7, CoeffOrderContext token)
#[inline]
fn va_ctx(v: u32) -> u32 {
    crate::frm::tc_ctx(v)
}

/// va_perm: ReadPermutation + DecodeLehmerCode -> the permutation of size entries
fn perm(jr: &mut Jr, jb: &mut Jb, skip: u32, size: u32, code: &mut Vec<u32>, temp: &mut Vec<u32>) -> Result<Vec<u32>, i32> {
    code.clear();
    code.resize(size as usize, 0);
    let end = jr.read(jb, va_ctx(size)).wrapping_add(skip);
    if end > size {
        return Err(ERR_CODE);
    }
    let mut last = 0u32;
    for i in skip..end {
        let v = jr.read(jb, va_ctx(last));
        last = v;
        if v >= size - i {
            return Err(ERR_CODE);
        }
        code[i as usize] = v;
    }
    let log2n = ceillog2(size);
    let padded = 1u32 << log2n;
    temp.clear();
    temp.resize(padded as usize, 0);
    for (i, t) in temp.iter_mut().enumerate() {
        let k = i as u32 + 1;
        *t = k & k.wrapping_neg();
    }
    let mut out = vec![0u32; size as usize];
    for i in 0..size as usize {
        let mut rank = code[i] + 1;
        let mut bit = padded;
        let mut next = 0u32;
        for _ in 0..=log2n {
            let cand = next + bit;
            bit >>= 1;
            let t = temp.get(cand as usize - 1).copied().unwrap_or(u32::MAX);
            if t < rank {
                next = cand;
                rank -= t;
            }
        }
        out[i] = next;
        let mut k = next + 1;
        while k <= padded {
            temp[k as usize - 1] = temp[k as usize - 1].wrapping_sub(1);
            k += k & k.wrapping_neg();
        }
    }
    Ok(out)
}

/// va_orders: DecodeCoeffOrders into dst[13][3]
fn orders(jb: &mut Jb, used_orders: u32, used_acs: u32, dst: &mut [Vec<u16>]) -> i32 {
    let mut mask = 0u32;
    for c in 0..27 {
        if used_acs >> c & 1 != 0 {
            mask |= 1 << tab::STRATEGYORDER[c];
        }
    }
    let jc = if used_orders != 0 {
        match je_read(jb, 8, false) {
            Ok(c) => Some(c),
            Err(e) => return e,
        }
    } else {
        None
    };
    let mut jr = match jc.as_ref() {
        Some(c) => match Jr::new(c, jb, 0) {
            Ok(r) => Some(r),
            Err(e) => return e,
        },
        None => None,
    };
    let mut done = 0u32;
    let mut code = Vec::new();
    let mut temp = Vec::new();
    for o in 0..27usize {
        let b = tab::STRATEGYORDER[o] as u32;
        if done >> b & 1 != 0 {
            continue;
        }
        done |= 1 << b;
        let llf = ACSCX[o] as u32 * ACSCY[o] as u32;
        let size = llf * 64;
        let used = mask >> b & 1 != 0;
        let coded = used_orders >> b & 1 != 0;
        if !used && !coded {
            continue;
        }
        let mut nat = vec![0u16; size as usize];
        jv_natorder(o, &mut nat);
        if !coded {
            for c in 0..3 {
                dst[b as usize * 3 + c] = nat.clone();
            }
            continue;
        }
        let Some(r) = jr.as_mut() else { return ERR_CODE };
        for c in 0..3 {
            let p = match perm(r, jb, llf, size, &mut code, &mut temp) {
                Ok(p) => p,
                Err(e) => return e,
            };
            if used {
                dst[b as usize * 3 + c] = p.iter().map(|&k| nat[k as usize]).collect();
            }
        }
    }
    if let Some(r) = jr.as_ref()
        && !r.fin()
    {
        return ERR_CODE;
    }
    0
}

/// jv_acglobal: the AC global section
pub fn jv_acglobal(jx: &Jx, jf: &mut Jf, jb: &mut Jb) -> i32 {
    let _ = jx;
    let Some(mut jv) = jf.vd.take() else { return ERR_HDR };
    let r = acglobal(jf, &mut jv, jb);
    jf.vd = Some(jv);
    r
}

fn acglobal(jf: &Jf, jv: &mut Jv, jb: &mut Jb) -> i32 {
    let r = qenc(jf, jv, jb);
    if r != 0 {
        return r;
    }
    let bits = ceillog2(jf.ng);
    jv.nhist = jb.f(bits) + 1;
    jv.nacctx = jv.nctx * (37 + 458);
    jv.np = jf.passes;
    jv.ord = Vec::new();
    jv.ord.resize(jf.passes as usize * 39, Vec::new());
    jv.acc = Vec::new();
    for p in 0..jf.passes as usize {
        let uo = jb.u32f(&D_ORDER);
        let r = orders(jb, uo, jv.usedacs, &mut jv.ord[p * 39..p * 39 + 39]);
        if r != 0 {
            return r;
        }
        match je_read(jb, jv.nhist.wrapping_mul(jv.nacctx), false) {
            Ok(c) => jv.acc.push(c),
            Err(e) => return e,
        }
    }
    jv.xdm = tab::DM[(jf.xqm & 7) as usize];
    jv.bdm = tab::DM[(jf.bqm & 7) as usize];
    // dequantization tables of the strategies in use
    jv.qtab = Vec::new();
    jv.qtab.resize(17, Vec::new());
    for s in 0..27 {
        if jv.usedacs >> s & 1 != 0 {
            let t = tab::ACSQT[s] as usize;
            if jv.qtab[t].is_empty() {
                let r = crate::vqt::jv_qtable(jv, t);
                if r != 0 {
                    return r;
                }
            }
        }
    }
    // pixel planes
    for c in 0..3 {
        let w = (jf.xb >> (jf.hshift(c) & 31)) << 3;
        let h = (jf.yb >> (jf.vshift(c) & 31)) << 3;
        jv.pxw[c] = w;
        jv.pxh[c] = h;
        match zvec::<f32>(w as usize * h as usize + 16) {
            Ok(v) => jv.px[c] = v,
            Err(e) => return e,
        }
    }
    jv.nz = vec![0; 3 * 1024];
    jv.ncb = if jf.passes == 1 { 1 } else { jf.ng };
    jv.coef = Vec::new();
    jv.coef.resize(jv.ncb as usize, Vec::new());
    jb.check()
}

/// jv_coefbuf: index of the coefficient buffer of group g (allocated, zeroed, on first use)
pub fn jv_coefbuf(jv: &mut Jv, g: u32) -> Result<usize, i32> {
    let k = if jv.ncb == 1 { 0 } else { g as usize };
    if k >= jv.coef.len() {
        return Err(img_core::ERR_NOMEM);
    }
    if jv.coef[k].is_empty() {
        jv.coef[k] = zvec(3 * JV_COEFSZ)?;
    }
    Ok(k)
}

/// jv_acgroup: coefficients of one pass of group g
pub fn jv_acgroup(jx: &Jx, jf: &mut Jf, g: u32, pass: u32, jb: &mut Jb) -> i32 {
    let _ = jx;
    let Some(mut jv) = jf.vd.take() else { return ERR_HDR };
    let r = acgroup(jf, &mut jv, g, pass, jb);
    jf.vd = Some(jv);
    r
}

fn acgroup(jf: &Jf, jv: &mut Jv, g: u32, pass: u32, jb: &mut Jb) -> i32 {
    let bufk = match jv_coefbuf(jv, g) {
        Ok(k) => k,
        Err(e) => return e,
    };
    let mut coef = core::mem::take(&mut jv.coef[bufk]);
    let mut nz = core::mem::take(&mut jv.nz);
    let r = acgroup_into(jf, jv, g, pass, jb, &mut nz, &mut coef);
    jv.coef[bufk] = coef;
    jv.nz = nz;
    r
}

/// the coefficients of one pass of group g added to coef (3 * JV_COEFSZ), nz the 3 * 1024
/// non-zero counts (scratch)  [jxl-plus: jv read-only, for the worker threads]
pub fn acgroup_into(jf: &Jf, jv: &Jv, g: u32, pass: u32, jb: &mut Jb, nz: &mut [i32], coef: &mut [i32]) -> i32 {
    let shift = jf.shift.get(pass as usize).copied().unwrap_or(0);
    let gy = g / jf.xg;
    let gx = g % jf.xg;
    let x0 = gx << 5;
    let y0 = gy << 5;
    let w = jf.xb.wrapping_sub(x0).min(32);
    let h = jf.yb.wrapping_sub(y0).min(32);
    let mut hs = [0u32; 3];
    let mut vs = [0u32; 3];
    for c in 0..3 {
        hs[c] = jf.hshift(c) & 31;
        vs[c] = jf.vshift(c) & 31;
    }
    let mut sel = 0u32;
    if jv.nhist > 1 {
        sel = jb.f(ceillog2(jv.nhist));
    }
    if sel >= jv.nhist {
        return ERR_CODE;
    }
    let ctxoff = sel.wrapping_mul(jv.nacctx);
    let Some(acc) = jv.acc.get(pass as usize) else { return ERR_CODE };
    let mut jr = match Jr::new(acc, jb, 0) {
        Ok(r) => r,
        Err(e) => return e,
    };
    nz.fill(0);
    let xb = jf.xb;
    let nctx = jv.nctx;
    let mut off = 0usize;
    for by in 0..h {
        for bx in 0..w {
            let bi = (y0 + by).wrapping_mul(xb).wrapping_add(x0).wrapping_add(bx) as usize;
            let a8 = jv.acs[bi];
            if a8 & 1 == 0 {
                continue;
            }
            let a = (a8 >> 1) as usize;
            let log2 = tab::ACSLOG2[a.min(26)] as u32;
            let size = 64u32 << log2;
            for &c in &CORDER {
                let sbx = bx >> hs[c];
                if sbx << hs[c] != bx {
                    continue;
                }
                let sby = by >> vs[c];
                if sby << vs[c] != by {
                    continue;
                }
                // predicted non-zeros from the top / left blocks
                let nzp = c * 1024;
                let idx = (sby * 32 + sbx) as usize;
                let pred = if sbx != 0 {
                    let l = nz[nzp + idx - 1] as u32;
                    if sby != 0 { (l.wrapping_add(nz[nzp + idx - 32] as u32).wrapping_add(1)) >> 1 } else { l }
                } else if sby != 0 {
                    nz[nzp + idx - 32] as u32
                } else {
                    32
                };
                // block context
                let qi = (y0 + by).wrapping_mul(xb).wrapping_add(x0).wrapping_add(sbx) as usize;
                let qf = jv.qf[qi] as u32;
                let mut qidx = 0u32;
                for t in 0..jv.nqft as usize {
                    if qf > jv.qft[t] {
                        qidx += 1;
                    }
                }
                let ci = if c < 2 { (c ^ 1) as u32 } else { 2 };
                let ob = tab::STRATEGYORDER[a.min(26)] as u32;
                let mut k = (ci * 13 + ob).wrapping_mul(jv.nqft + 1).wrapping_add(qidx);
                k = k.wrapping_mul(jv.ndcctx).wrapping_add(jv.qdci[bi] as u32);
                let bctx = jv.ctxmap.get(k as usize).copied().unwrap_or(0) as u32;
                let oi = pass as usize * 39 + ob as usize * 3 + c;
                // non-zero count
                let mut e = pred.min(64);
                if e >= 8 {
                    e = (e >> 1) + 4;
                }
                let ctx = e.wrapping_mul(nctx).wrapping_add(bctx).wrapping_add(ctxoff);
                let nzeros = jr.read(jb, ctx);
                let cov = 1u32 << log2;
                if nzeros > size - cov {
                    return ERR_CODE;
                }
                let mut nzr = nzeros;
                let v = (nzeros + cov - 1) >> log2;
                let cy = ACSCY[a.min(26)] as u32;
                let cx = ACSCX[a.min(26)] as u32;
                for yy in 0..cy {
                    for xx in 0..cx {
                        let p = ((sby + yy) * 32 + sbx + xx) as usize;
                        if p < 1024 {
                            nz[nzp + p] = v as i32;
                        }
                    }
                }
                let hbase = bctx.wrapping_mul(458).wrapping_add(nctx * 37).wrapping_add(ctxoff);
                let order = &jv.ord[oi];
                let blk = &mut coef[c * JV_COEFSZ + off..];
                let mut kk = cov;
                let mut prev = (nzr <= size >> 4) as u32;
                while kk < size && nzr != 0 {
                    let a1 = tab::NZCTX[((nzr + cov - 1) >> log2) as usize & 63] as u32;
                    let a2 = tab::FREQCTX[(kk >> log2) as usize & 63] as u32;
                    let cx2 = (a1 + a2) * 2 + prev + hbase;
                    let u = jr.read(jb, cx2);
                    prev = (u != 0) as u32;
                    nzr -= prev;
                    let val = unpack_signed(u).wrapping_shl(shift);
                    let o = order.get(kk as usize).copied().unwrap_or(0) as usize;
                    if let Some(b) = blk.get_mut(o) {
                        *b = b.wrapping_add(val);
                    }
                    kk += 1;
                }
                if nzr != 0 {
                    return ERR_CODE;
                }
            }
            off += size as usize;
        }
    }
    if !jr.fin() {
        return ERR_CODE;
    }
    jb.check()
}
