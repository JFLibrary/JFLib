// ops_jxl -- codectest batch ops 180..199: JPEG XL  [apps/codectest/src/jxl_ops.s]
//
//   180 OP_JXINFO   data = .jxl file: records the image header and every frame header / TOC:
//                   {u32 w, h, bits, float, nec, xyb, colour space, tf, orientation,
//                   have_preview, preview w, h, have_animation, ICC length, frame count;
//                   per frame (preview first) 28 u32: type, modular, flags, colour transform,
//                   upsampling, passes, w, h, xsize_blocks, ysize_blocks, groups, dc groups,
//                   TOC entries, first section (bytes), frame end (bytes), is_last, duration,
//                   save_as_reference, save_before_ct, gaborish, epf iterations, x0, y0,
//                   custom size, blend mode, blend source, group dim, TOC size sum}
//   181 OP_JXMOD    data = .jxl file, arg = frame index (preview not counted): decodes that
//                   frame and records its modular image: {u32 channels; per channel u32 w, h,
//                   hshift, vshift; then the samples of every channel (i32)}
//   184 OP_JXVDC    data = .jxl file, arg = frame index: decodes the DC groups of that VarDCT
//                   frame and records {u32 xb, yb; f32 DC planes X, Y, B; u8 strategy; i32
//                   quant field; u8 sharpness; f32 inverse sigma; u8 DC bucket; u32 tiles cw,
//                   ch; i32 ytox, ytob}
//   185 OP_JXVPX    data = .jxl file, arg = frame | flags << 16: decodes that VarDCT frame and
//                   records the reconstructed pixel planes before rendering: {u32 w, h of
//                   planes X, Y, B; f32 samples of each plane}
//   186 OP_JXRND    data = .jxl file, arg = frame | flags << 16: decodes that frame and runs
//                   the render pipeline; records {u32 planes, alpha plane (-1 none); per plane
//                   u32 w, h; f32 samples of each plane, row by row}
//   187 OP_JXENC    data = surface record [+ u32 EXIF length, EXIF, u32 XMP length, XMP],
//                   arg = IO_FLAGS | quality << 16: jxl encode, records the file
//   188 OP_JXEH     data = u32 nctx, then (u32 ctx, u32 value) tokens: the writer's histogram
//                   set and token stream; records {u32 bytes, the bytes, u16 slots of cluster
//                   0 (4096)}
//   183 OP_JXDEC    data = .jxl file, arg = IMGD flags | frame << 16: decode into a surface,
//                   records the surface (ct_surface_out)
//   189 OP_JXENC2   (jxl-plus) data = {u32 IO_FLAGS, quality, effort} + surface record + [u32
//                   ICC length, ICC, u32 EXIF length, EXIF, u32 XMP length, XMP]: the codec's
//                   encoder (the new writer unless IOF_JXL_COMPAT), records the file
//   190 OP_JXTIME   (jxl-plus) as 189, arg = repetitions: records {u32 file size, u64 best
//                   time in ns}
//   197 OP_JXSPLINE (jxl-tools) data = {u32 IO_FLAGS, quality, effort} + the spline
//                   description of img_jxl_enc::espline::from_bytes + surface record [+ the blobs of
//                   op 189]: the new writer with those splines in the frame (test-only), records
//                   the file
//   198 OP_JXFEAT   (jxl-tools) data = .jxl file: decodes every frame and records per frame 16
//                   u32: type, flags, frame w, h, patch positions, distinct patch rectangles,
//                   patch pixels (sum of the rectangles' areas), spline segments, then the 8 f32
//                   noise LUT values; arg 1: then per patch position 7 u32 (reference frame,
//                   x0, y0, w, h of the rectangle, x, y)
//   199 OP_JXBA     (jxl-tools) data = {u32 w, h, f32 intensity target, hf asymmetry, xmul} +
//                   two w x h RGBA8 images (sRGB): Butteraugli of the second against the first:
//                   records {f32 score (the largest value), f64 3-norm, f32 distmap w * h, u64
//                   microseconds (reference + comparison)}; arg 1 = one thread

use alloc::vec::Vec;

use img_core::{ImgDec, Surface, ERR_HDR};
use img_jxl::bits::Jb;
use img_jxl::dec::jx_frame;
use img_jxl::frm::{jf_header, jf_toc, Jf};
use img_jxl::noise::jx_count;
use img_jxl::rnd::jx_render;

use crate::Ct;

const OP_JXINFO: u32 = 180;
const OP_JXMOD: u32 = 181;
const OP_JXDEC: u32 = 183;
const OP_JXVDC: u32 = 184;
const OP_JXVPX: u32 = 185;
const OP_JXRND: u32 = 186;
const OP_JXENC: u32 = 187;
const OP_JXEH: u32 = 188;
const OP_JXENC2: u32 = 189;
const OP_JXTIME: u32 = 190;
const OP_JXSELF: u32 = 191;
const OP_JXDTIME: u32 = 194;
const OP_BROTLI: u32 = 195;
const OP_JPEGRECON: u32 = 196;
const OP_JXSPLINE: u32 = 197; // jxl-tools
const OP_JXFEAT: u32 = 198; // jxl-tools
const OP_JXBA: u32 = 199; // jxl-tools

const JXI_HEAD: usize = 15;
const JXI_FRAME: usize = 28;
const JXI_MAXF: u32 = 64;

fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn putf(v: &mut Vec<u8>, d: &[f32]) {
    for x in d {
        v.extend_from_slice(&x.to_bits().to_le_bytes());
    }
}

/// run op (one record via ct.record); false = unknown op
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        OP_JXINFO => info(ct, data),
        OP_JXMOD => walk(ct, arg, data, 0),
        OP_JXVDC => walk(ct, arg | 2 << 16, data, 1),
        OP_JXVPX => walk(ct, arg, data, 2),
        OP_JXRND => walk(ct, arg, data, 3),
        OP_JXDEC => {
            let dec = ImgDec { flags: arg & 0xFFFF, frame: arg >> 16, ..Default::default() };
            let mut sf = Surface::default();
            let r = img_jxl::decode_opt(data, &mut sf, &dec, None);
            if r != 0 {
                ct.record(r, &[]);
            } else {
                ct.surface_out(0, &sf);
            }
        }
        OP_JXENC => enc(ct, arg, data),
        OP_JXEH => eh(ct, data),
        OP_JXENC2 => enc2(ct, data, 0),
        OP_JXTIME => enc2(ct, data, arg.max(1)),
        OP_JXDTIME => dtime(ct, arg & 0xFFFF, arg >> 16, data),
        OP_BROTLI => match img_jxl::brotli::decompress(data, 1 << 28) {
            // op 195 (jxl-plus): Brotli decompression -> {u32 input bytes used} + output
            Ok((v, used)) => {
                let mut o = Vec::with_capacity(v.len() + 4);
                put32(&mut o, used as u32);
                o.extend_from_slice(&v);
                ct.record(0, &o);
            }
            Err(e) => ct.record(e, &[]),
        },
        OP_JPEGRECON => match img_jxl::jpeg::reconstruct(data) {
            // op 196 (jxl-plus): JPEG reconstruction -> the JPEG file
            Ok(v) => ct.record(0, &v),
            Err(e) => ct.record(e, &[]),
        },
        OP_JXSELF => selftest(ct),
        OP_JXSPLINE => spline(ct, data), // jxl-tools
        OP_JXFEAT => feat(ct, data, arg), // jxl-tools
        OP_JXBA => butteraugli(ct, data, arg),  // jxl-tools
        _ => return false,
    }
    true
}

/// op 180: header and frame records
fn info(ct: &mut Ct, data: &[u8]) {
    let d = match img_jxl::open(data) {
        Ok(d) => d,
        Err(e) => {
            ct.record(e, &[]);
            return;
        }
    };
    let jx = &d.jx;
    let mut out = alloc::vec![0u8; (JXI_HEAD + JXI_FRAME * JXI_MAXF as usize) * 4];
    let set = |o: &mut Vec<u8>, i: usize, v: u32| o[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    let head = [
        jx.w,
        jx.h,
        jx.bits,
        jx.flt as u32,
        jx.nec,
        jx.xyb as u32,
        jx.cspace as u32,
        jx.tf as u32,
        jx.orient as u32,
        jx.haveprev as u32,
        jx.pw,
        jx.ph,
        jx.haveanim as u32,
        jx.icc.len() as u32,
    ];
    for (i, &v) in head.iter().enumerate() {
        set(&mut out, i, v);
    }
    let mut br = Jb::init(&d.cs, jx.cslen);
    br.pos = jx.hdrend;
    br.end = jx.cslen << 3;
    let mut preview = jx.haveprev != 0;
    let mut n = 0u32;
    let mut fin: u32 = 0;
    let mut failed = false;
    while n < JXI_MAXF {
        let mut jf = Jf::default();
        let mut r = jf_header(jx, &mut br, &mut jf, preview);
        if r == 0 {
            r = jf_toc(&mut br, &mut jf);
        }
        if r != 0 {
            fin = ((r.wrapping_neg() as u32) << 16) | n;
            failed = true;
            break;
        }
        let toc: u32 = jf.tocsz.iter().fold(0u32, |a, &v| a.wrapping_add(v));
        let f = [
            jf.typ,
            jf.modular,
            jf.flags as u32,
            jf.ct,
            jf.ups,
            jf.passes,
            jf.w,
            jf.h,
            jf.xb,
            jf.yb,
            jf.ng,
            jf.ndg,
            jf.ntoc,
            jf.base as u32,
            jf.end as u32,
            jf.islast,
            jf.duration,
            jf.saveref,
            jf.savebct,
            jf.gab,
            jf.epfit,
            jf.x0 as u32,
            jf.y0 as u32,
            jf.custom,
            jf.bmode,
            jf.bsrc,
            jf.gdim,
            toc,
        ];
        let base = JXI_HEAD + n as usize * JXI_FRAME;
        for (i, &v) in f.iter().enumerate() {
            set(&mut out, base + i, v);
        }
        n += 1;
        if !preview && jf.islast != 0 {
            break;
        }
        preview = false;
        if jf.end >= jx.cslen {
            break;
        }
        br.pos = jf.end << 3;
    }
    if !failed {
        fin = n;
    }
    set(&mut out, 14, fin);
    let len = JXI_HEAD * 4 + (fin & 0xFF) as usize * JXI_FRAME * 4;
    ct.record(0, &out[..len]);
}

/// ops 181 / 184 / 185 / 186: decode frame arg & 0xFFFF and record one of its stages
fn walk(ct: &mut Ct, arg: u32, data: &[u8], mode: u32) {
    let d = match img_jxl::open(data) {
        Ok(d) => d,
        Err(e) => {
            ct.record(e, &[]);
            return;
        }
    };
    let cs = d.cs;
    let mut jx = d.jx;
    let mut br = Jb::init(&cs, jx.cslen);
    br.pos = jx.hdrend;
    br.end = jx.cslen << 3;
    let mut preview = jx.haveprev != 0;
    let flags = arg >> 16;
    let target = arg & 0xFFFF;
    let mut k = 0u32;
    let res: Result<Vec<u8>, i32> = 'w: loop {
        let mut jf = Jf::default();
        let r = jf_header(&jx, &mut br, &mut jf, preview);
        if r != 0 {
            break 'w Err(r);
        }
        jx_count(&mut jx, &mut jf);
        let r = jf_toc(&mut br, &mut jf);
        if r != 0 {
            break 'w Err(r);
        }
        if !preview {
            if k == target {
                jf.dbg = flags;
                let r = jx_frame(&jx, &cs, &mut jf);
                if r != 0 {
                    break 'w Err(r);
                }
                break 'w record(&mut jx, &mut jf, mode);
            }
            if jf.islast != 0 {
                break 'w Err(ERR_HDR);
            }
            k += 1;
            let r = jx_frame(&jx, &cs, &mut jf);
            if r != 0 {
                break 'w Err(r);
            }
            let r = jx_render(&mut jx, &mut jf);
            if r != 0 {
                break 'w Err(r);
            }
        }
        preview = false;
        if jf.end >= jx.cslen {
            break 'w Err(ERR_HDR);
        }
        br.pos = jf.end << 3;
    };
    match res {
        Ok(v) => ct.record(0, &v),
        Err(e) => ct.record(e, &[]),
    }
}

/// the record of the decoded frame
fn record(jx: &mut img_jxl::hdr::Jx, jf: &mut Jf, mode: u32) -> Result<Vec<u8>, i32> {
    let mut o: Vec<u8> = Vec::new();
    match mode {
        0 => {
            let Some(full) = jf.full.as_ref() else { return Err(ERR_HDR) };
            put32(&mut o, full.nch());
            for c in &full.ch {
                put32(&mut o, c.w);
                put32(&mut o, c.h);
                put32(&mut o, c.hs as u32);
                put32(&mut o, c.vs as u32);
            }
            for c in &full.ch {
                let n = c.w as usize * c.h as usize;
                for v in &c.d[..n] {
                    o.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        1 => {
            let Some(jv) = jf.vd.as_ref() else { return Err(ERR_HDR) };
            let n = jf.xb as usize * jf.yb as usize;
            let nt = jv.cw as usize * jv.ch as usize;
            put32(&mut o, jf.xb);
            put32(&mut o, jf.yb);
            for c in 0..3 {
                putf(&mut o, &jv.dc[c][..n]);
            }
            o.extend_from_slice(&jv.acs[..n]);
            for v in &jv.qf[..n] {
                o.extend_from_slice(&v.to_le_bytes());
            }
            o.extend_from_slice(&jv.sharp[..n]);
            putf(&mut o, &jv.isig[..n]);
            o.extend_from_slice(&jv.qdci[..n]);
            put32(&mut o, jv.cw);
            put32(&mut o, jv.ch);
            for v in &jv.ytox[..nt] {
                o.extend_from_slice(&v.to_le_bytes());
            }
            for v in &jv.ytob[..nt] {
                o.extend_from_slice(&v.to_le_bytes());
            }
        }
        2 => {
            let Some(jv) = jf.vd.as_ref() else { return Err(ERR_HDR) };
            for c in 0..3 {
                put32(&mut o, jv.pxw[c]);
                put32(&mut o, jv.pxh[c]);
            }
            for c in 0..3 {
                let n = jv.pxw[c] as usize * jv.pxh[c] as usize;
                putf(&mut o, &jv.px[c][..n]);
            }
        }
        _ => {
            let r = jx_render(jx, jf);
            if r != 0 {
                return Err(r);
            }
            let Some(jrn) = jf.rnd.as_ref() else { return Err(ERR_HDR) };
            put32(&mut o, jrn.np);
            put32(&mut o, jrn.alpha as u32);
            for p in &jrn.p {
                put32(&mut o, p.w);
                put32(&mut o, p.h);
            }
            for p in &jrn.p {
                for y in 0..p.h as usize {
                    let s = y * p.s as usize;
                    putf(&mut o, &p.d[s..s + p.w as usize]);
                }
            }
        }
    }
    Ok(o)
}

/// op 187: jxl encode of a surface record with an optional EXIF / XMP trailer
fn enc(ct: &mut Ct, arg: u32, data: &[u8]) {
    let Some(sf) = crate::surface_in(data) else {
        ct.record(img_core::ERR_HDR, &[]);
        return;
    };
    let u32at = |o: usize| -> u32 { if o + 4 <= data.len() { crate::u32_at(data, o) } else { 0 } };
    let slice = |o: usize, n: usize| -> &[u8] {
        let a = o.min(data.len());
        let b = o.saturating_add(n).min(data.len());
        &data[a..b]
    };
    let mut exif: Option<&[u8]> = None;
    let mut xmp: Option<&[u8]> = None;
    let rows = (crate::u32_at(data, 0).wrapping_mul(crate::u32_at(data, 4)) as usize) * 4 + 12;
    if rows + 8 <= data.len() {
        let en = u32at(rows) as usize;
        let pos = rows + en + 4;
        if pos + 4 <= data.len() {
            let xn = u32at(pos) as usize;
            if en != 0 {
                exif = Some(slice(rows + 4, en));
            }
            xmp = Some(slice(pos + 4, xn));
        }
    }
    let p = img_jxl::enc::EncParams { flags: arg & 0xFFFF, quality: arg >> 16, exif, xmp, cancel: None };
    match img_jxl::enc::encode_ex(&sf, &p) {
        Ok(v) => ct.record(0, &v),
        Err(e) => ct.record(e, &[]),
    }
}

/// op 188: the writer's histogram set and token stream of (ctx, value) tokens
fn eh(ct: &mut Ct, data: &[u8]) {
    use img_jxl::ewr::{Eh, Es, Jw};
    let nctx = if data.len() >= 4 { crate::u32_at(data, 0) } else { 0 };
    let ntok = data.len().wrapping_sub(4) >> 3;
    let tok = |i: usize| (crate::u32_at(data, 4 + i * 8), crate::u32_at(data, 8 + i * 8));
    let mut jw = Jw::new();
    let mut e = Eh::new(nctx, None);
    for i in 0..ntok {
        let (c, v) = tok(i);
        e.add(c, v);
    }
    let _ = e.finish();
    let _ = e.write(&mut jw);
    let mut es = Es::new();
    for i in 0..ntok {
        let (c, v) = tok(i);
        es.put(&e, &mut jw, c, v);
    }
    es.flush(&e, &mut jw);
    jw.pad();
    let mut o: Vec<u8> = Vec::with_capacity(jw.len() + 4 + 8192);
    put32(&mut o, jw.len() as u32);
    o.extend_from_slice(&jw.buf);
    for &s in e.slot[0].iter() {
        o.extend_from_slice(&s.to_le_bytes());
    }
    ct.record(0, &o);
}

/// the input of ops 189 / 190 / 197: (surface, metadata blobs) of a surface record + blobs
fn enc_input(sd: &[u8]) -> Option<(Surface, img_core::ImgMeta)> {
    let sf = crate::surface_in(sd)?;
    let rows = (crate::u32_at(sd, 0).wrapping_mul(crate::u32_at(sd, 4)) as usize) * 4 + 12;
    let mut meta = img_core::ImgMeta::default();
    let mut pos = rows;
    let blob = |pos: &mut usize| -> Vec<u8> {
        if *pos + 4 > sd.len() {
            return Vec::new();
        }
        let n = crate::u32_at(sd, *pos) as usize;
        let a = (*pos + 4).min(sd.len());
        let b = (*pos + 4).saturating_add(n).min(sd.len());
        *pos = b;
        sd[a..b].to_vec()
    };
    meta.icc = blob(&mut pos);
    meta.exif = blob(&mut pos);
    meta.xmp = blob(&mut pos);
    Some((sf, meta))
}

/// ops 189 / 190: the codec's encoder with explicit ImgOpt fields (reps 0 = record the file)
fn enc2(ct: &mut Ct, data: &[u8], reps: u32) {
    if data.len() < 24 {
        ct.record(img_core::ERR_HDR, &[]);
        return;
    }
    let flags = crate::u32_at(data, 0);
    let quality = crate::u32_at(data, 4);
    let effort = crate::u32_at(data, 8);
    let Some((sf, meta)) = enc_input(&data[12..]) else {
        ct.record(img_core::ERR_HDR, &[]);
        return;
    };
    let opt = img_core::ImgOpt { quality, flags, effort, meta: Some(&meta), ..img_core::ImgOpt::from_bytes(&[]) };
    if reps == 0 {
        let mut out = Vec::new();
        let r = img_jxl::enc::encode(&sf, &opt, &mut out);
        ct.record(r, if r == 0 { &out } else { &[] });
        return;
    }
    let mut best = u64::MAX;
    let mut size = 0u32;
    for _ in 0..reps {
        let t0 = plat::time_us();
        let mut out = Vec::new();
        let r = img_jxl::enc::encode(&sf, &opt, &mut out);
        let t = plat::time_us().wrapping_sub(t0);
        if r != 0 {
            ct.record(r, &[]);
            return;
        }
        size = out.len() as u32;
        best = best.min(t);
    }
    let mut o = Vec::with_capacity(12);
    put32(&mut o, size);
    o.extend_from_slice(&best.to_le_bytes());
    ct.record(0, &o);
}

/// op 199 (jxl-tools): Butteraugli of two sRGB images
fn butteraugli(ct: &mut Ct, data: &[u8], arg: u32) {
    if data.len() < 20 {
        ct.record(ERR_HDR, &[]);
        return;
    }
    let w = crate::u32_at(data, 0) as usize;
    let h = crate::u32_at(data, 4) as usize;
    let f = |o: usize| f32::from_bits(crate::u32_at(data, o));
    let n = w * h;
    // tests: a u32 after the two images: debug bits (1 no half-size level, 2 the parts at the
    // centre pixel instead of the distance)
    let dbg = if data.len() >= 24 + 8 * n { crate::u32_at(data, 20 + 8 * n) } else { 0 };
    let p = img_jxl_enc::ebutter::Params { intensity: f(8), hf_asym: f(12), xmul: f(16), dbg };
    if data.len() < 20 + 8 * n {
        ct.record(ERR_HDR, &[]);
        return;
    }
    // tests: dbg 4 = the sRGB EOTF applied twice, 8 = linear input (v / 255)
    let eotf = |v: f32| -> f32 {
        if v <= 0.04045 {
            v / 12.92
        } else {
            img_jxl::spline::pow_f64(((v + 0.055) / 1.055) as f64, 2.4) as f32
        }
    };
    let lin = |o: usize| -> [Vec<f32>; 3] {
        core::array::from_fn(|c| {
            (0..n)
                .map(|i| {
                    let v = data[o + i * 4 + c];
                    if dbg & 8 != 0 {
                        v as f32 / 255.0
                    } else if dbg & 4 != 0 {
                        eotf(img_jxl::tab::SRGBLIN[v as usize])
                    } else {
                        img_jxl::tab::SRGBLIN[v as usize]
                    }
                })
                .collect()
        })
    };
    let a = lin(20);
    let b = lin(20 + 4 * n);
    let t0 = plat::time_us();
    let cmp = img_jxl_enc::ebutter::Comparator::new(&a, w, h, p, arg & 1 == 0);
    if dbg & 2 != 0 {
        let mut o = Vec::new();
        putf(&mut o, &cmp.parts(&b, (h / 2) * w + w / 2));
        ct.record(0, &o);
        return;
    }
    let dm = cmp.diffmap(&b);
    let us = plat::time_us().wrapping_sub(t0);
    let mut o = Vec::with_capacity(20 + 4 * n);
    o.extend_from_slice(&img_jxl_enc::ebutter::score(&dm).to_bits().to_le_bytes());
    o.extend_from_slice(&img_jxl_enc::ebutter::pnorm(&dm, 3.0).to_bits().to_le_bytes());
    putf(&mut o, &dm);
    o.extend_from_slice(&us.to_le_bytes());
    ct.record(0, &o);
}

/// op 198 (jxl-tools): the features of every frame
fn feat(ct: &mut Ct, data: &[u8], arg: u32) {
    let d = match img_jxl::open(data) {
        Ok(d) => d,
        Err(e) => {
            ct.record(e, &[]);
            return;
        }
    };
    let cs = d.cs;
    let mut jx = d.jx;
    let mut br = Jb::init(&cs, jx.cslen);
    br.pos = jx.hdrend;
    br.end = jx.cslen << 3;
    let mut preview = jx.haveprev != 0;
    let mut o = Vec::new();
    let res: Result<(), i32> = loop {
        let mut jf = Jf::default();
        let r = jf_header(&jx, &mut br, &mut jf, preview);
        if r != 0 {
            break Err(r);
        }
        jx_count(&mut jx, &mut jf);
        let r = jf_toc(&mut br, &mut jf);
        if r != 0 {
            break Err(r);
        }
        if !preview {
            let r = jx_frame(&jx, &cs, &mut jf);
            if r != 0 {
                break Err(r);
            }
            let mut rects: Vec<(u32, u32, u32, u32, u32)> = jf.patch.iter().map(|p| (p.rf, p.rx0, p.ry0, p.rw, p.rh)).collect();
            rects.sort_unstable();
            rects.dedup();
            let px: u32 = rects.iter().map(|r| r.3 * r.4).sum();
            let segs = jf.splines.as_ref().map_or(0, |s| s.segments()) as u32;
            for v in [jf.typ, jf.flags as u32, jf.w, jf.h, jf.patch.len() as u32, rects.len() as u32, px, segs] {
                put32(&mut o, v);
            }
            putf(&mut o, &jf.nlut);
            if arg == 1 {
                for p in &jf.patch {
                    for v in [p.rf, p.rx0, p.ry0, p.rw, p.rh, p.x, p.y] {
                        put32(&mut o, v);
                    }
                }
            }
            let r = jx_render(&mut jx, &mut jf);
            if r != 0 {
                break Err(r);
            }
        }
        if !preview && jf.islast != 0 {
            break Ok(());
        }
        preview = false;
        if jf.end >= jx.cslen {
            break Ok(());
        }
        br.pos = jf.end << 3;
    };
    match res {
        Ok(()) => ct.record(0, &o),
        Err(e) => ct.record(e, &o),
    }
}

/// op 197 (jxl-tools): the writer with test-only splines
fn spline(ct: &mut Ct, data: &[u8]) {
    if data.len() < 12 {
        ct.record(ERR_HDR, &[]);
        return;
    }
    let flags = crate::u32_at(data, 0);
    let quality = crate::u32_at(data, 4);
    let effort = crate::u32_at(data, 8);
    let Some((q, used)) = img_jxl_enc::espline::from_bytes(&data[12..]) else {
        ct.record(ERR_HDR, &[]);
        return;
    };
    let Some((sf, meta)) = enc_input(&data[12 + used..]) else {
        ct.record(ERR_HDR, &[]);
        return;
    };
    let opt = img_core::ImgOpt { quality, flags, effort, meta: Some(&meta), ..img_core::ImgOpt::from_bytes(&[]) };
    match img_jxl_enc::enew::encode_with_splines(&sf, &opt, &q) {
        Ok(v) => ct.record(0, &v),
        Err(e) => ct.record(e, &[]),
    }
}

/// op 194 (jxl-plus): decode timing: arg = reps | IMGD flags << 16 -> {u64 best us, u32 w, h,
/// u32 pixel checksum}
fn dtime(ct: &mut Ct, reps: u32, flags: u32, data: &[u8]) {
    let mut best = u64::MAX;
    let mut sum = 0u32;
    let (mut w, mut h) = (0u32, 0u32);
    for _ in 0..reps.max(1) {
        let dec = ImgDec { flags, ..Default::default() };
        let mut sf = Surface::default();
        let t0 = plat::time_us();
        let r = img_jxl::decode_opt(data, &mut sf, &dec, None);
        let t = plat::time_us().wrapping_sub(t0);
        if r != 0 {
            ct.record(r, &[]);
            return;
        }
        best = best.min(t);
        w = sf.w as u32;
        h = sf.h as u32;
        sum = 0;
        let mut row = alloc::vec![0u8; w as usize * 4];
        for y in 0..sf.h as usize {
            img_core::meta::get_row(&sf, y, &mut row, gfx::FMT_ARGB);
            for (i, &b) in row.iter().enumerate() {
                sum = sum.wrapping_mul(31).wrapping_add(b as u32 ^ i as u32);
            }
        }
    }
    let mut o = Vec::new();
    o.extend_from_slice(&best.to_le_bytes());
    put32(&mut o, w);
    put32(&mut o, h);
    put32(&mut o, sum);
    ct.record(0, &o);
}

/// op 191 (jxl-plus): transform self test: per strategy (0..20) the largest round-trip error
/// of TransformFromPixels / TransformToPixels, of the DC (block means) and the largest
/// difference of the writer's fast inverse from the decoder's, as f32 triples
fn selftest(ct: &mut Ct) {
    use img_jxl_enc::etx;
    let mut tx = etx::Tx::new();
    let mut o = Vec::new();
    let mut seed = 12345u32;
    for acs in 0..21usize {
        let (cx, cy) = etx::cover(acs);
        let (w, h) = (cx * 8, cy * 8);
        let mut px = alloc::vec![0f32; w * h];
        for v in px.iter_mut() {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            *v = ((seed >> 16) & 1023) as f32 / 1023.0;
        }
        let mut c = alloc::vec![0f32; 64 * cx * cy];
        etx::from_pixels(&mut tx, acs, &px, w, &mut c);
        let mut dc = alloc::vec![0f32; cx * cy];
        etx::dc_from_llf(&mut tx, acs, &c, &mut dc, cx);
        let mut dce = 0f32;
        for by in 0..cy {
            for bx in 0..cx {
                let mut m = 0f32;
                for y in 0..8 {
                    for x in 0..8 {
                        m += px[(by * 8 + y) * w + bx * 8 + x];
                    }
                }
                m /= 64.0;
                let e = (dc[by * cx + bx] - m).abs();
                if e > dce {
                    dce = e;
                }
            }
        }
        if cx * cy > 1 {
            // the decoder's LLF of this DC must give the coefficients back
            let (mn, mx) = (cx.min(cy), cx.max(cy));
            let mut llf = alloc::vec![0f32; 64 * cx * cy];
            img_jxl::dct::jd_llf(acs as u32, &dc, cx, &mut llf, &mut tx.scr);
            dce = 0.0;
            for i in 0..mn {
                for j in 0..mx {
                    let k = i * 8 * mx + j;
                    let e = (llf[k] - c[k]).abs();
                    if e > dce {
                        dce = e;
                    }
                }
            }
        }
        let mut out = alloc::vec![0f32; w * h];
        let mut c2 = c.clone();
        etx::to_pixels(&mut tx, acs, &mut c, &mut out, w);
        let mut e = 0f32;
        for i in 0..w * h {
            let d = (out[i] - px[i]).abs();
            if d > e {
                e = d;
            }
        }
        // the writer's fast inverse against the decoder's
        let mut out2 = alloc::vec![0f32; w * h];
        etx::to_pixels_fast(&mut tx, acs, &mut c2, &mut out2, w);
        let mut ef = 0f32;
        for i in 0..w * h {
            let d = (out2[i] - out[i]).abs();
            if d > ef {
                ef = d;
            }
        }
        putf(&mut o, &[e, dce, ef]);
    }
    ct.record(0, &o);
}
