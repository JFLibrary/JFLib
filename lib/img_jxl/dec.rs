// dec -- JPEG XL frame decoding driver (dec_frame.cc, dec_modular.cc; model: tools/jxl_ref.py
// Frame.decode, decode_global_modular, decode_group, pass_bracket)       [src/img/jxl_dec.s]
//
//   jx_frame(jx, cs, jf) -> 0 / ERR_*   decode the sections of the frame whose header and TOC
//                                       are in jf: jf.full = modular image with the transforms
//                                       undone (modular frames), jf.vd = VarDCT state with the
//                                       pixel planes (VarDCT frames)
//   jx_framefree(jf)                    release the decoding state (not the header arrays)

use alloc::boxed::Box;
use alloc::vec::Vec;

use img_core::ERR_HDR;

use crate::bits::Jb;
use crate::ent::je_read;
use crate::frm::{ghdr_default, Jf, JCT_NONE, JCT_YCBCR, JFF_NOISE, JFF_PATCHES, JFF_SKIPDCSMOOTH, JFF_SPLINES, JFF_USEDC};
use crate::hdr::{Jx, JCS_GRAY};
use crate::modular::{jm_decode, jm_tree, Mi};

/// jf.dbg bit: decode for the JPEG reconstruction (serial, unscaled DC, the quantized
/// coefficients captured instead of pixels)  [jxl-plus]
pub const JDBG_JPEG: u32 = 32;
/// jf.dbg bit with JDBG_JPEG: the JPEG has one component  [jxl-plus]
pub const JDBG_GRAY: u32 = 64;

/// jx_framefree: release the decoding state of jf (the header arrays stay)
pub fn jx_framefree(jf: &mut Jf) {
    jf.full = None;
    jf.gcode = None;
    jf.gtree = Default::default();
    jf.vd = None;
    jf.rnd = None;
    jf.patch = Vec::new();
    jf.splines = None;
    jf.newref = None;
    jf.newdc = None;
}

/// xd_bracket: GetDownsamplingBracket -> (min shift, max shift)
fn bracket(jf: &Jf, pass: u32) -> (i32, i32) {
    let mut max: i32 = 2;
    let mut min: i32 = 3;
    let mut i = 0u32;
    loop {
        for j in 0..(jf.nds as usize).min(4) {
            if jf.lastpass[j] == i {
                min = jf.ds[j].trailing_zeros() as i32;
            }
        }
        if i == jf.passes.wrapping_sub(1) {
            min = 0;
        }
        if i == pass {
            break;
        }
        max = min - 1;
        i += 1;
        if i >= 16 {
            break;
        }
    }
    // the assembly returns max << 8 | min (low byte unsigned, the max byte signed)
    ((min & 255), (max as i8) as i32)
}

/// xd_group: a modular group: the channels larger than a group, cropped to rect = x0, y0,
/// xs, ys, decoded and copied into jf.full
fn xd_group(jx: &Jx, jf: &mut Jf, rect: [u32; 4], jb: &mut Jb, mins: i32, maxs: i32, stream: u32) -> i32 {
    match xd_group_decode(jx, jf, rect, jb, mins, maxs, stream) {
        Ok(Some((gi, wh))) => xd_group_apply(jf, &gi, &wh),
        Ok(None) => 0,
        Err(e) => e,
    }
}

/// the decoded part of xd_group: the group image and its channels (full channel, x0, y0, w,
/// h), None when the group has no channel in the bracket
fn xd_group_decode(jx: &Jx, jf: &Jf, rect: [u32; 4], jb: &mut Jb, mins: i32, maxs: i32, stream: u32) -> Result<Option<(Mi, Vec<[u32; 5]>)>, i32> {
    let _ = jx;
    let gdim = jf.gdim;
    let Some(full) = jf.full.as_ref() else { return Ok(None) };
    let mut first = full.nbmeta as usize;
    while first < full.ch.len() {
        let c = &full.ch[first];
        if c.w > gdim || c.h > gdim {
            break;
        }
        first += 1;
    }
    if first >= full.ch.len() {
        return Ok(None);
    }
    let mut wh: Vec<[u32; 5]> = Vec::new();
    let mut gi = Mi::new(rect[2], rect[3], full.bd);
    for i in first..full.ch.len() {
        let c = &full.ch[i];
        let m = if c.hs > c.vs { c.vs } else { c.hs };
        if m > maxs || m < mins {
            continue;
        }
        let hs = (c.hs & 31) as u32;
        let vs = (c.vs & 31) as u32;
        let x0 = ((rect[0] as i32) >> hs) as u32;
        let xs = ((rect[2] as i32) >> hs) as u32;
        let y0 = ((rect[1] as i32) >> vs) as u32;
        let ys = ((rect[3] as i32) >> vs) as u32;
        let dw = c.w.wrapping_sub(x0) as i32;
        let w = if dw <= 0 { 0 } else { (dw as u32).min(xs) };
        let dh = c.h.wrapping_sub(y0) as i32;
        let h = if dh <= 0 { 0 } else { (dh as u32).min(ys) };
        if w == 0 || h == 0 {
            continue;
        }
        let n = wh.len() as u32;
        wh.push([i as u32, x0, y0, w, h]);
        let r = gi.insert(n, w, h, c.hs, c.vs);
        if r != 0 {
            return Err(r);
        }
    }
    if wh.is_empty() {
        return Ok(None);
    }
    let gtree = if jf.gtree.nodes.is_empty() { None } else { Some(&jf.gtree) };
    let gcode = jf.gcode.as_deref();
    let r = jm_decode(jb, &mut gi, stream, 0xFF_FFFF, gtree, gcode, true, None);
    if r != 0 {
        return Err(r);
    }
    if gi.ch.len() != wh.len() {
        return Err(ERR_HDR);
    }
    Ok(Some((gi, wh)))
}

/// the rest of xd_group: the group's channels into jf.full
fn xd_group_apply(jf: &mut Jf, gi: &Mi, wh: &[[u32; 5]]) -> i32 {
    let Some(full) = jf.full.as_mut() else { return 0 };
    for (k, w) in wh.iter().enumerate() {
        let g = &gi.ch[k];
        if g.w != w[3] || g.h != w[4] {
            return ERR_HDR;
        }
        let f = &mut full.ch[w[0] as usize];
        let fw = f.w as usize;
        let n = w[3] as usize;
        for row in 0..w[4] as usize {
            let o = (w[2] as usize + row) * fw + w[1] as usize;
            let s = row * g.w as usize;
            f.d[o..o + n].copy_from_slice(&g.d[s..s + n]);
        }
    }
    0
}

/// xd_global: global modular info: tree, full image, global channels
fn xd_global(jx: &Jx, jf: &mut Jf, jb: &mut Jb) -> i32 {
    let mut nb = 3u32;
    if jx.cspace == JCS_GRAY && jf.ct == JCT_NONE {
        nb = 1;
    }
    if jf.modular == 0 {
        nb = 0;
    }
    jf.nbcol = nb;
    if jb.f(1) != 0 {
        let mut lim = (jf.w as u64 * jf.h as u64).wrapping_mul(jx.nec.wrapping_add(nb) as u64) >> 4;
        lim = lim.wrapping_add(1024).min(1 << 22);
        match jm_tree(jb, lim) {
            Ok(m) => jf.gtree = m,
            Err(e) => return e,
        }
        match je_read(jb, (jf.gtree.nodes.len() as u32 + 1) >> 1, false) {
            Ok(c) => jf.gcode = Some(Box::new(c)),
            Err(e) => return e,
        }
    }
    let mut full = Box::new(Mi::new(jf.w, jf.h, jx.bits));
    for c in 0..nb {
        let (mut w, mut h, mut hs, mut vs) = (jf.w, jf.h, 0u32, 0u32);
        if jf.ct == JCT_YCBCR {
            hs = jf.hshift(c as usize);
            w = jf.w.wrapping_add((1u32 << (hs & 31)) - 1) >> (hs & 31);
            vs = jf.vshift(c as usize);
            h = jf.h.wrapping_add((1u32 << (vs & 31)) - 1) >> (vs & 31);
        }
        let r = full.insert(c, w, h, hs as i32, vs as i32);
        if r != 0 {
            return r;
        }
    }
    for e in 0..jx.nec as usize {
        let ups = jf.ecups[e];
        let tu = ups.trailing_zeros() & 31;
        let s = tu.wrapping_sub(jf.ups.trailing_zeros()) as i32;
        let w = (ups.wrapping_sub(1).wrapping_add(jf.wup)) >> tu;
        let h = (ups.wrapping_sub(1).wrapping_add(jf.hup)) >> tu;
        let n = full.nch();
        let r = full.insert(n, w, h, s, s);
        if r != 0 {
            return r;
        }
    }
    let gtree = if jf.gtree.nodes.is_empty() { None } else { Some(&jf.gtree) };
    let gcode = jf.gcode.as_deref();
    let r = jm_decode(jb, &mut full, 0, jf.gdim, gtree, gcode, false, Some(&mut jf.ghdr));
    jf.full = Some(full);
    r
}

/// the reader of section i (frames with one section share theirs)  [jx_section]
fn section<'a>(cs: &'a [u8], jf: &Jf, i: u32) -> Jb<'a> {
    let i = i as usize;
    let start = jf.tocoff.get(i).copied().unwrap_or(0).wrapping_add(jf.base);
    let size = jf.tocsz.get(i).copied().unwrap_or(0) as u64;
    Jb::section(cs, start, size)
}

/// jx_frame: decode the sections of the frame (see the module header)
pub fn jx_frame(jx: &Jx, cs: &[u8], jf: &mut Jf) -> i32 {
    jf.ghdr = ghdr_default();
    let one = jf.ntoc == 1;
    let mut sec = if one {
        Jb::section(cs, jf.base, jf.tocsz.first().copied().unwrap_or(0) as u64)
    } else {
        section(cs, jf, 0)
    };
    if jf.flags & JFF_PATCHES != 0 {
        let r = crate::blend::jx_patches(jx, jf, &mut sec);
        if r != 0 {
            return r;
        }
    }
    // jxl-tools: splines (the draw cache needs the colour correlation of the DC global info)
    let mut splq = None;
    if jf.flags & JFF_SPLINES != 0 {
        match crate::spline::decode(&mut sec, jf.w as u64 * jf.h as u64) {
            Ok(q) => splq = Some(q),
            Err(e) => return e,
        }
    }
    if jf.flags & JFF_NOISE != 0 {
        for k in 0..8 {
            jf.nlut[k] = sec.f(10) as i32 as f32 / 1024.0;
        }
    }
    jf.dcq = [f32::from_bits(0x3980_0000), f32::from_bits(0x3b00_0000), f32::from_bits(0x3b80_0000)];
    if sec.f(1) == 0 {
        for k in 0..3 {
            let v = sec.f16f() * f32::from_bits(0x3c00_0000);
            jf.dcq[k] = v;
            if !(v >= f32::from_bits(0x322b_cc77)) {
                return ERR_HDR;
            }
        }
    }
    if jf.modular == 0 {
        let r = crate::vdc::jv_global(jx, jf, &mut sec);
        if r != 0 {
            return r;
        }
        if jf.dbg & JDBG_JPEG != 0 {
            // jxl-plus, JPEG reconstruction: the DC unscaled (ClearDCMul)
            if let Some(jv) = jf.vd.as_mut() {
                jv.muldc = [1.0; 3];
            }
        }
    }
    if let Some(q) = splq {
        let (bx, bb) = match jf.vd.as_ref() {
            Some(v) if jf.modular == 0 => (v.basex, v.baseb),
            _ => (0.0, 1.0),
        };
        match crate::spline::Splines::new(&q, jf.wup, jf.hup, bx, bb) {
            Ok(s) => jf.splines = Some(Box::new(s)),
            Err(e) => return e,
        }
    }
    let r = xd_global(jx, jf, &mut sec);
    if r != 0 {
        return r;
    }
    if !one {
        let r = sec.check();
        if r != 0 {
            return r;
        }
    }
    // DC groups (jxl-plus: on the worker threads when they are sections of their own)
    let mt = jf.dbg & JDBG_JPEG == 0;
    if !one && jf.ndg > 1 && mt {
        let r = dc_groups_mt(jx, cs, jf);
        if r != 0 {
            return r;
        }
    }
    for g in 0..if !one && jf.ndg > 1 && mt { 0 } else { jf.ndg } {
        let gy = g / jf.xdg;
        let gx = g % jf.xdg;
        let dim = jf.gdim << 3;
        let rect = [gx.wrapping_mul(dim), gy.wrapping_mul(dim), dim, dim];
        if !one {
            sec = section(cs, jf, g + 1);
        }
        if jf.modular == 0 {
            let r = if jf.flags & JFF_USEDC != 0 {
                crate::vdc::jv_dcframe(jx, jf, g) // jxl-plus: DC from the DC frame
            } else {
                crate::vdc::jv_dcgroup(jx, jf, g, &mut sec)
            };
            if r != 0 {
                return r;
            }
        }
        let r = xd_group(jx, jf, rect, &mut sec, 3, 1000, g.wrapping_add(jf.ndg).wrapping_add(1));
        if r != 0 {
            return r;
        }
        if jf.modular == 0 {
            let r = crate::vdc::jv_acmeta(jx, jf, g, &mut sec);
            if r != 0 {
                return r;
            }
        }
        if !one {
            let r = sec.check();
            if r != 0 {
                return r;
            }
        }
    }
    if jf.modular == 0 {
        if jf.flags & (JFF_SKIPDCSMOOTH | JFF_USEDC) == 0 {
            let r = crate::vdc::jv_smooth(jf);
            if r != 0 {
                return r;
            }
        }
        if jf.dbg & 2 != 0 {
            return 0; // test hook: stop after the DC groups
        }
        if !one {
            sec = section(cs, jf, jf.ndg + 1);
        }
        let r = crate::vac::jv_acglobal(jx, jf, &mut sec);
        if r != 0 {
            return r;
        }
        if !one {
            let r = sec.check();
            if r != 0 {
                return r;
            }
        }
    }
    // AC groups (jxl-plus: on the worker threads when they are sections of their own)
    for pass in 0..jf.passes {
        let (mins, maxs) = bracket(jf, pass);
        if !one && jf.ng > 1 && mt {
            let r = ac_groups_mt(jx, cs, jf, pass, mins, maxs);
            if r != 0 {
                return r;
            }
            continue;
        }
        for g in 0..jf.ng {
            let gy = g / jf.xg;
            let gx = g % jf.xg;
            let gd = jf.gdim;
            let rect = [gx.wrapping_mul(gd), gy.wrapping_mul(gd), gd, gd];
            if !one {
                let k = pass.wrapping_mul(jf.ng).wrapping_add(g).wrapping_add(jf.ndg).wrapping_add(2);
                sec = section(cs, jf, k);
            }
            if jf.modular == 0 {
                let r = crate::vac::jv_acgroup(jx, jf, g, pass, &mut sec);
                if r != 0 {
                    return r;
                }
            }
            let stream = pass.wrapping_mul(jf.ng).wrapping_add(g).wrapping_add(jf.ndg.wrapping_mul(3)).wrapping_add(18);
            let r = xd_group(jx, jf, rect, &mut sec, mins, maxs, stream);
            if r != 0 {
                return r;
            }
            if jf.modular == 0 && pass + 1 >= jf.passes {
                let r = if mt { crate::vrc::jv_recon(jx, jf, g) } else { crate::jpeg::capture(jf, g) };
                if r != 0 {
                    return r;
                }
            }
            if !one {
                let r = sec.check();
                if r != 0 {
                    return r;
                }
            }
        }
    }
    if one {
        let r = sec.check();
        if r != 0 {
            return r;
        }
    }
    if jf.dbg & 1 != 0 {
        return 0;
    }
    let wh = jf.ghdr.wp;
    match jf.full.as_mut() {
        Some(full) => crate::mtr::jm_undo(full, &wh),
        None => 0,
    }
}

// ------------------------------------------------------------------ threads  [jxl-plus]
//
// The groups of a frame whose sections are separate are entropy decoded (and VarDCT groups
// reconstructed into tiles of their own) on the worker pool; what they write into the shared
// frame state (DC, metadata, modular channels, pixels) is applied afterwards in group order,
// with the errors in the order of the serial decoder, so the result does not depend on the
// threads.

/// the entropy decoded parts of one DC group
struct DcOut {
    dc: Option<Result<(u32, Mi), i32>>,
    grp: Option<Result<Option<(Mi, Vec<[u32; 5]>)>, i32>>,
    meta: Option<Result<(u32, Mi), i32>>,
    chk: i32,
}

/// the DC groups (sections 1 ..= ndg) on the worker pool
fn dc_groups_mt(jx: &Jx, cs: &[u8], jf: &mut Jf) -> i32 {
    let vardct = jf.modular == 0;
    let usedc = jf.flags & JFF_USEDC != 0;
    let jfr: &Jf = jf;
    let outs: Vec<DcOut> = crate::par::par_map(jfr.ndg as usize, true, |gi| {
        let g = gi as u32;
        let mut sec = section(cs, jfr, g + 1);
        let mut o = DcOut { dc: None, grp: None, meta: None, chk: 0 };
        if vardct && !usedc {
            let r = crate::vdc::dc_decode(jx, jfr, g, &mut sec);
            let bad = r.is_err();
            o.dc = Some(r);
            if bad {
                return o;
            }
        }
        let dim = jfr.gdim << 3;
        let rect = [(g % jfr.xdg).wrapping_mul(dim), (g / jfr.xdg).wrapping_mul(dim), dim, dim];
        let r = xd_group_decode(jx, jfr, rect, &mut sec, 3, 1000, g.wrapping_add(jfr.ndg).wrapping_add(1));
        let bad = r.is_err();
        o.grp = Some(r);
        if bad {
            return o;
        }
        if vardct {
            let r = crate::vdc::acmeta_decode(jx, jfr, g, &mut sec);
            let bad = r.is_err();
            o.meta = Some(r);
            if bad {
                return o;
            }
        }
        o.chk = sec.check();
        o
    });
    for (gi, o) in outs.into_iter().enumerate() {
        let g = gi as u32;
        if vardct {
            let r = if usedc {
                crate::vdc::jv_dcframe(jx, jf, g)
            } else {
                match o.dc {
                    Some(Ok((eb, mi))) => crate::vdc::dc_apply(jf, g, eb, &mi),
                    Some(Err(e)) => e,
                    None => ERR_HDR,
                }
            };
            if r != 0 {
                return r;
            }
        }
        match o.grp {
            Some(Ok(Some((gi, wh)))) => {
                let r = xd_group_apply(jf, &gi, &wh);
                if r != 0 {
                    return r;
                }
            }
            Some(Ok(None)) => {}
            Some(Err(e)) => return e,
            None => return ERR_HDR,
        }
        if vardct {
            let r = match o.meta {
                Some(Ok((cnt, mi))) => crate::vdc::acmeta_apply(jf, g, cnt, &mi),
                Some(Err(e)) => e,
                None => ERR_HDR,
            };
            if r != 0 {
                return r;
            }
        }
        if o.chk != 0 {
            return o.chk;
        }
    }
    0
}

/// per thread scratch of the AC group workers
struct Gs {
    nz: Vec<i32>,
    coef: Vec<i32>,
    rw: Vec<f32>,
    scr: Box<crate::dct::Scr>,
    tile: [Vec<f32>; 3],
}

const TILE: usize = 256;

/// the decoded parts of one AC group: errors of the steps, modular channels, pixels of the
/// group (VarDCT, last pass: per channel w, h and the rows)
struct AcOut {
    ac: i32,
    grp: Option<Result<Option<(Mi, Vec<[u32; 5]>)>, i32>>,
    rec: i32,
    px: Option<[(usize, usize, Vec<f32>); 3]>,
    chk: i32,
}

/// the AC groups of one pass on the worker pool
fn ac_groups_mt(jx: &Jx, cs: &[u8], jf: &mut Jf, pass: u32, mins: i32, maxs: i32) -> i32 {
    use crate::vac::JV_COEFSZ;
    use rt::sync::SpinLock;
    let vardct = jf.modular == 0;
    let last = pass + 1 >= jf.passes;
    let ng = jf.ng as usize;
    // coefficient buffers: per group when the passes add up, else per thread
    let multi = vardct && jf.passes > 1;
    let mut cells: Vec<SpinLock<Vec<i32>>> = Vec::new();
    if multi {
        let Some(jv) = jf.vd.as_mut() else { return ERR_HDR };
        for g in 0..ng {
            match crate::vac::jv_coefbuf(jv, g as u32) {
                Ok(k) => cells.push(SpinLock::new(core::mem::take(&mut jv.coef[k]))),
                Err(e) => return e,
            }
        }
    }
    let nslots = plat::thr_count() as usize + 1;
    let slots: Vec<SpinLock<Option<Gs>>> = (0..nslots).map(|_| SpinLock::new(None)).collect();
    let jfr: &Jf = jf;
    let mut outs: Vec<AcOut> = crate::par::par_map(ng, true, |gi| {
        let g = gi as u32;
        let mut o = AcOut { ac: 0, grp: None, rec: 0, px: None, chk: 0 };
        let k = pass.wrapping_mul(jfr.ng).wrapping_add(g).wrapping_add(jfr.ndg).wrapping_add(2);
        let mut sec = section(cs, jfr, k);
        let gd = jfr.gdim;
        let rect = [(g % jfr.xg).wrapping_mul(gd), (g / jfr.xg).wrapping_mul(gd), gd, gd];
        let stream = pass.wrapping_mul(jfr.ng).wrapping_add(g).wrapping_add(jfr.ndg.wrapping_mul(3)).wrapping_add(18);
        let Some(jv) = jfr.vd.as_deref().filter(|_| vardct) else {
            if vardct {
                o.ac = ERR_HDR;
                return o;
            }
            // modular frame: the group's channels only
            let r = xd_group_decode(jx, jfr, rect, &mut sec, mins, maxs, stream);
            let bad = r.is_err();
            o.grp = Some(r);
            if !bad {
                o.chk = sec.check();
            }
            return o;
        };
        let mut slot = slots[plat::thr_self().min(nslots - 1)].lock();
        let gs = slot.get_or_insert_with(|| Gs {
            nz: alloc::vec![0; 3 * 1024],
            coef: if multi { Vec::new() } else { alloc::vec![0; 3 * JV_COEFSZ] },
            rw: Vec::new(),
            scr: Box::default(),
            tile: [alloc::vec![0.0; TILE * TILE], alloc::vec![0.0; TILE * TILE], alloc::vec![0.0; TILE * TILE]],
        });
        let mut cell = if multi { Some(cells[gi].lock()) } else { None };
        let coef: &mut Vec<i32> = match cell.as_mut() {
            Some(c) => c,
            None => &mut gs.coef,
        };
        o.ac = crate::vac::acgroup_into(jfr, jv, g, pass, &mut sec, &mut gs.nz, coef);
        if o.ac != 0 {
            return o;
        }
        let r = xd_group_decode(jx, jfr, rect, &mut sec, mins, maxs, stream);
        let bad = r.is_err();
        o.grp = Some(r);
        if bad {
            return o;
        }
        if last {
            if gs.rw.is_empty() {
                gs.rw = alloc::vec![0.0; 3 * JV_COEFSZ];
            }
            for t in gs.tile.iter_mut() {
                t.fill(0.0);
            }
            o.rec = crate::vrc::recon_into(jx, jfr, jv, g, coef, &mut gs.rw, &mut gs.scr, &mut gs.tile, [TILE; 3], true);
            if o.rec != 0 {
                return o;
            }
            // the group's pixels of every channel
            let (x0, y0) = ((g % jfr.xg) << 5, (g / jfr.xg) << 5);
            let w = jfr.xb.wrapping_sub(x0).min(32);
            let h = jfr.yb.wrapping_sub(y0).min(32);
            let mut px: [(usize, usize, Vec<f32>); 3] = [(0, 0, Vec::new()), (0, 0, Vec::new()), (0, 0, Vec::new())];
            for c in 0..3 {
                let (hs, vs) = (jfr.hshift(c) & 31, jfr.vshift(c) & 31);
                let pw = jv.pxw[c] as usize;
                let ph = jv.pxh[c] as usize;
                let ox = ((x0 >> hs) as usize) * 8;
                let oy = ((y0 >> vs) as usize) * 8;
                let cw = ((((w + (1 << hs) - 1) >> hs) as usize) * 8).min(pw.saturating_sub(ox)).min(TILE);
                let ch = ((((h + (1 << vs) - 1) >> vs) as usize) * 8).min(ph.saturating_sub(oy)).min(TILE);
                let mut v = Vec::with_capacity(cw * ch);
                for r in 0..ch {
                    v.extend_from_slice(&gs.tile[c][r * TILE..r * TILE + cw]);
                }
                px[c] = (cw, ch, v);
            }
            o.px = Some(px);
        }
        o.chk = sec.check();
        o
    });
    drop(slots);
    // per group coefficient buffers back
    if multi {
        let Some(jv) = jf.vd.as_mut() else { return ERR_HDR };
        for (g, c) in cells.into_iter().enumerate() {
            jv.coef[g] = core::mem::take(&mut *c.lock());
        }
    }
    // in group order: the modular parts and the first error; the pixels of the groups before
    // it (and of the failing group itself when only its section check failed) are copied as
    // the serial decoder would have, in parallel bands of group rows (jxl-speed)
    let mut err = 0;
    let mut upto = outs.len(); // groups whose pixels are copied
    for (gi, o) in outs.iter_mut().enumerate() {
        if o.ac != 0 {
            (err, upto) = (o.ac, gi);
            break;
        }
        match o.grp.take() {
            Some(Ok(Some((gim, wh)))) => {
                let r = xd_group_apply(jf, &gim, &wh);
                if r != 0 {
                    (err, upto) = (r, gi);
                    break;
                }
            }
            Some(Ok(None)) => {}
            Some(Err(e)) => {
                (err, upto) = (e, gi);
                break;
            }
            None => {
                (err, upto) = (ERR_HDR, gi);
                break;
            }
        }
        if o.rec != 0 {
            (err, upto) = (o.rec, gi);
            break;
        }
        if o.chk != 0 {
            (err, upto) = (o.chk, gi + 1);
            break;
        }
    }
    let (xg, ng32) = (jf.xg, jf.ng);
    let hsv: [(u32, u32); 3] = [(jf.hshift(0), jf.vshift(0)), (jf.hshift(1), jf.vshift(1)), (jf.hshift(2), jf.vshift(2))];
    if let Some(jv) = jf.vd.as_mut() {
        for c in 0..3 {
            let (hs, vs) = (hsv[c].0 & 31, hsv[c].1 & 31);
            let pw = jv.pxw[c] as usize;
            if pw == 0 {
                continue;
            }
            let rows = jv.px[c].len() / pw;
            let band = ((32u32 >> vs) as usize * 8).max(1);
            let outs_r = &outs;
            crate::par::par_rows(alloc::vec![(&mut jv.px[c][..], pw)], rows, band, true, |y0, _n, b| {
                // the groups of group row y0 / band
                let gy = (y0 / band) as u32;
                for gx in 0..xg {
                    let g = gy * xg + gx;
                    if g >= ng32 || g as usize >= upto {
                        continue;
                    }
                    let Some(px) = outs_r[g as usize].px.as_ref() else { continue };
                    let (cw, ch, v) = &px[c];
                    let (x0, gy0) = (gx << 5, gy << 5);
                    let ox = ((x0 >> hs) as usize) * 8;
                    let oy = ((gy0 >> vs) as usize) * 8 - y0;
                    for r in 0..*ch {
                        let o = (oy + r) * pw + ox;
                        b[0][o..o + cw].copy_from_slice(&v[r * cw..r * cw + cw]);
                    }
                }
            });
        }
    } else if outs.iter().take(upto).any(|o| o.px.is_some()) {
        return ERR_HDR;
    }
    err
}
