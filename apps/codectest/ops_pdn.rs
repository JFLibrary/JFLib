// ops_pdn -- codectest ops 220..239: PDN, NRBF (img_pdn)     [apps/codectest/src/pdn_ops.s]
//
//   220 decode_ex      data = .pdn file, arg = PDND_* flags     -> doc record
//   221 encode_ex      data = doc record, arg = PDNE_* | level << 8 -> .pdn file
//   222 round trip     data = .pdn file, arg as 221             -> decode + encode_ex
//   223 Nrbf::parse    data = NRBF stream, arg = maxmem (0 none) -> {u32 objects, i32 root, u64 end}
//   224 flatten        data = .pdn file                         -> surface record
//   225 bench decode   data = .pdn file, arg = repetitions      -> {u64 us per decode, u32 layers}
//   226 bench encode   data = {u32 w, h, layers, seed}, arg = level | 256: decode + compare
//                      | 512: time flatten + resize only -> {u64 us, u64 file bytes / resize
//                      us, u64 decode us}
//   227 bench gzip     data = bytes, arg = level | reps << 8    -> {u64 us per gzip_encode, u64 bytes}
//   228 bench alloc    data = {u64 size}, arg = reps            -> {u64 us per alloc + touch + free}
//
// Doc record (little endian): header 64 bytes {u32 w, h, n, unit; f64 dpux, dpuy; u32 nvclen,
// xmllen, icclen, 0; 16 zero bytes}, nvc, xml, icc bytes, then per layer 32 bytes {u32
// namelen; u8 opacity, visible, blend, isbg; u32 metalen, w, h, fmt; 8 zero bytes}, name,
// meta, packed pixel rows (4 bytes per pixel, 1 for FMT_A8).

use alloc::vec::Vec;

use crate::{Ct, u32_at};
use gfx::{FMT_A8, FMT_ARGB, FMT_PARGB, Surface};
use img_core::gzip::gzip_encode;
use img_core::resample::{RS_FANT, resize};
use img_core::{ERR_CHECKSUM, ImgMeta, ImgOpt};
use img_pdn::nrbf::Nrbf;
use img_pdn::{PdnDoc, PdnLayer, decode_ex, encode_ex, flatten};

const DR_HDR: usize = 64;
const DRL_HDR: usize = 32;
const R_BAD: i32 = -100;

/// run op (one record via ct.record); false = unknown op  [pdn_op]
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        220 => match decode_ex(data, None, arg) {
            Ok(doc) => ct.record(0, &doc_rec(&doc)),
            Err(e) => ct.record(e, &[]),
        },
        221 => match get_doc(data) {
            Some(doc) => enc_doc(ct, &doc, arg),
            None => ct.record(R_BAD, &[]),
        },
        222 => match decode_ex(data, None, 0) {
            Ok(doc) => enc_doc(ct, &doc, arg),
            Err(e) => ct.record(e, &[]),
        },
        223 => match Nrbf::parse(data, arg as u64) {
            Ok(nb) => {
                let mut r = Vec::with_capacity(16);
                r.extend_from_slice(&nb.nobj().to_le_bytes());
                r.extend_from_slice(&nb.root().to_le_bytes());
                r.extend_from_slice(&(nb.end() as u64).to_le_bytes());
                ct.record(0, &r);
            }
            Err(e) => ct.record(e, &[]),
        },
        224 => match decode_ex(data, None, 0).and_then(|doc| flatten(&doc.view())) {
            Ok(sf) => ct.surface_out(0, &sf),
            Err(e) => ct.record(e, &[]),
        },
        225 => bench_dec(ct, data, arg),
        226 => bench_enc(ct, data, arg),
        227 => {
            let level = (arg & 255) as i32;
            let reps = match arg >> 8 {
                0 => 1,
                n => n,
            };
            let t0 = plat::time_us();
            let mut n = 0u64;
            for _ in 0..reps {
                n = gzip_encode(data, level).len() as u64;
            }
            let us = (plat::time_us() - t0) / reps as u64;
            let mut r = Vec::with_capacity(16);
            r.extend_from_slice(&us.to_le_bytes());
            r.extend_from_slice(&n.to_le_bytes());
            ct.record(0, &r);
        }
        228 => {
            let mut b = [0u8; 8];
            let k = data.len().min(8);
            b[..k].copy_from_slice(&data[..k]);
            let size = u64::from_le_bytes(b) as usize;
            let reps = if arg == 0 { 1 } else { arg };
            let t0 = plat::time_us();
            for _ in 0..reps {
                let mut v: Vec<u8> = Vec::new();
                if v.try_reserve_exact(size).is_err() {
                    break;
                }
                v.resize(size, 0);
                let mut i = 0;
                while i < size {
                    v[i] = 1;
                    i += 4096;
                }
                drop(v);
            }
            let us = (plat::time_us() - t0) / reps as u64;
            ct.record(0, &us.to_le_bytes());
        }
        _ => return false,
    }
    true
}

// the doc record of a document  [pdo_put_doc]
fn doc_rec(doc: &PdnDoc) -> Vec<u8> {
    let icc: &[u8] = doc.meta.as_ref().map_or(&[], |m| &m.icc);
    let mut r = Vec::new();
    for v in [doc.w, doc.h, doc.layers.len() as u32, doc.unit] {
        r.extend_from_slice(&v.to_le_bytes());
    }
    r.extend_from_slice(&doc.dpux.to_le_bytes());
    r.extend_from_slice(&doc.dpuy.to_le_bytes());
    for v in [doc.nvc.len() as u32, doc.xml.len() as u32, icc.len() as u32, 0] {
        r.extend_from_slice(&v.to_le_bytes());
    }
    r.resize(DR_HDR, 0);
    r.extend_from_slice(&doc.nvc);
    r.extend_from_slice(&doc.xml);
    r.extend_from_slice(icc);
    for l in &doc.layers {
        let name = cut(&l.name);
        let mut h = [0u8; DRL_HDR];
        h[..4].copy_from_slice(&(name.len() as u32).to_le_bytes());
        h[4..8].copy_from_slice(&[l.opacity, l.visible, l.blend, l.isbg]);
        h[8..12].copy_from_slice(&(l.meta.len() as u32).to_le_bytes());
        h[12..16].copy_from_slice(&(l.sf.w as u32).to_le_bytes());
        h[16..20].copy_from_slice(&(l.sf.h as u32).to_le_bytes());
        h[20..24].copy_from_slice(&(l.sf.fmt as u32).to_le_bytes());
        r.extend_from_slice(&h);
        r.extend_from_slice(name);
        r.extend_from_slice(&l.meta);
        let rb = l.sf.w.max(0) as usize * 4;
        let ss = l.sf.stride as usize;
        for y in 0..l.sf.h.max(0) as usize {
            r.extend_from_slice(&l.sf.px[y * ss..y * ss + rb]);
        }
    }
    r
}

// a NUL-terminated name's bytes
fn cut(s: &[u8]) -> &[u8] {
    match s.iter().position(|&b| b == 0) {
        Some(k) => &s[..k],
        None => s,
    }
}

// CHECK_SPAN
fn span(p: usize, n: usize, len: usize) -> bool {
    p <= len && len - p >= n
}

// a document from a doc record (the XML is not used by the encoder)  [pdo_get_doc]
fn get_doc(d: &[u8]) -> Option<PdnDoc> {
    let len = d.len();
    if len < DR_HDR {
        return None;
    }
    let f64_at = |o: usize| f64::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3], d[o + 4], d[o + 5], d[o + 6], d[o + 7]]);
    let mut doc = PdnDoc { w: u32_at(d, 0), h: u32_at(d, 4), unit: u32_at(d, 12), dpux: f64_at(16), dpuy: f64_at(24), ..Default::default() };
    let mut p = DR_HDR;
    let n = u32_at(d, 32) as usize; // nvc
    if !span(p, n, len) {
        return None;
    }
    doc.nvc = d[p..p + n].to_vec();
    p += n;
    let n = u32_at(d, 36) as usize; // xml
    if !span(p, n, len) {
        return None;
    }
    p += n;
    let n = u32_at(d, 40) as usize; // icc
    if !span(p, n, len) {
        return None;
    }
    if n != 0 {
        doc.meta = Some(ImgMeta { icc: d[p..p + n].to_vec(), ..Default::default() });
        p += n;
    }
    let nl = u32_at(d, 8);
    if nl > 65535 {
        return None;
    }
    for _ in 0..nl {
        if !span(p, DRL_HDR, len) {
            return None;
        }
        let h = &d[p..p + DRL_HDR];
        p += DRL_HDR;
        let mut l = PdnLayer { opacity: h[4], visible: h[5], blend: h[6], isbg: h[7], ..Default::default() };
        let n = u32_at(h, 0) as usize; // name
        if !span(p, n, len) {
            return None;
        }
        l.name = d[p..p + n].to_vec();
        p += n;
        let n = u32_at(h, 8) as usize; // meta
        if !span(p, n, len) {
            return None;
        }
        l.meta = d[p..p + n].to_vec();
        p += n;
        // pixels: {w, h, fmt} + rows
        let (w, hh, fmt) = (u32_at(h, 12), u32_at(h, 16), u32_at(h, 20));
        if w > 65536 || hh > 65536 {
            return None; // surf_alloc fails
        }
        l.sf = Surface::new(w as i32, hh as i32, fmt as u8);
        let rb = if l.sf.fmt == FMT_A8 { 1 } else { 4 } * w as usize;
        if !span(p, rb * hh as usize, len) {
            return None;
        }
        let ss = l.sf.stride as usize;
        for y in 0..hh as usize {
            l.sf.px[y * ss..y * ss + rb].copy_from_slice(&d[p..p + rb]);
            p += rb;
        }
        doc.layers.push(l);
    }
    Some(doc)
}

// encode_ex(doc, IO_EFFORT = arg >> 8, flags arg & 255) and record the file  [pdo_enc_doc]
fn enc_doc(ct: &mut Ct, doc: &PdnDoc, arg: u32) {
    let opt = ImgOpt { effort: arg >> 8, ..Default::default() };
    match encode_ex(&doc.view(), Some(&opt), arg & 255) {
        Ok(f) => ct.record(0, &f),
        Err(e) => ct.record(e, &[]),
    }
}

// op 225  [pdo_bench_dec]
fn bench_dec(ct: &mut Ct, data: &[u8], arg: u32) {
    let reps = if arg == 0 { 1 } else { arg };
    let t0 = plat::time_us();
    let mut n = 0u32;
    for _ in 0..reps {
        match decode_ex(data, None, 0) {
            Ok(doc) => n = doc.layers.len() as u32,
            Err(e) => {
                ct.record(e, &[]);
                return;
            }
        }
    }
    let us = (plat::time_us() - t0) / reps as u64;
    let mut r = Vec::with_capacity(12);
    r.extend_from_slice(&us.to_le_bytes());
    r.extend_from_slice(&n.to_le_bytes());
    ct.record(0, &r);
}

// op 226: a synthetic w x h document with n layers (layer 0 opaque noisy gradient, the others
// mostly transparent strokes), encode_ex timed; arg & 256: decode the file again and compare
// every pixel (ERR_CHECKSUM when different)  [pdo_bench_enc]
fn bench_enc(ct: &mut Ct, data: &[u8], arg: u32) {
    if data.len() < 16 {
        ct.record(R_BAD, &[]);
        return;
    }
    let (w, h, n, seed) = (u32_at(data, 0), u32_at(data, 4), u32_at(data, 8), u32_at(data, 12));
    if n > 64 || n == 0 {
        ct.record(R_BAD, &[]);
        return;
    }
    let mut doc = PdnDoc { w, h, ..Default::default() };
    for i in 0..n {
        if w > 65536 || h > 65536 {
            return; // surf_alloc fails: the assembly records nothing here
        }
        let mut l = PdnLayer { opacity: 255, visible: 1, name: b"Layer".to_vec(), ..Default::default() };
        l.sf = Surface::new(w as i32, h as i32, FMT_ARGB);
        fill(&mut l.sf, i, seed);
        doc.layers.push(l);
    }
    let mut rec = [0u64; 3];
    if arg & 512 != 0 {
        // time flatten + resize only
        let t0 = plat::time_us();
        let comp = flatten(&doc.view()).unwrap_or_default();
        rec[0] = plat::time_us() - t0;
        let t0 = plat::time_us();
        let mut t = Surface::new(256, 192, FMT_PARGB);
        let _ = resize(&comp, &mut t, RS_FANT, 0);
        rec[1] = plat::time_us() - t0;
    } else {
        let opt = ImgOpt { effort: arg & 255, ..Default::default() };
        let t0 = plat::time_us();
        let file = match encode_ex(&doc.view(), Some(&opt), 0) {
            Ok(f) => f,
            Err(e) => {
                ct.record(e, &[]);
                return;
            }
        };
        rec[0] = plat::time_us() - t0;
        rec[1] = file.len() as u64;
        if arg & 256 != 0 {
            let t0 = plat::time_us();
            let back = match decode_ex(&file, None, 0) {
                Ok(d) => d,
                Err(e) => {
                    ct.record(e, &[]);
                    return;
                }
            };
            rec[2] = plat::time_us() - t0;
            let e = cmp_docs(&doc, &back);
            if e != 0 {
                ct.record(e, &[]);
                return;
            }
        }
    }
    let mut r = Vec::with_capacity(24);
    for v in rec {
        r.extend_from_slice(&v.to_le_bytes());
    }
    ct.record(0, &r);
}

// synthetic content: layer 0 an opaque gradient with noise, others transparent with diagonal
// semi-transparent bands  [pdo_fill]
fn fill(sf: &mut Surface, layer: u32, seed: u32) {
    let mut s = seed.wrapping_mul(0x9E37_79B1).wrapping_add(layer) | 1; // xorshift state
    let ss = sf.stride as usize;
    for y in 0..sf.h as u32 {
        let row = &mut sf.px[y as usize * ss..];
        for x in 0..sf.w as u32 {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let v = if layer == 0 {
                // B = x, G = y, R = x + y, + noise 0..7
                let nz = s & 7;
                let b = x.wrapping_add(nz) & 255;
                let g = y.wrapping_add(nz) & 255;
                let r = (b + g) & 255;
                0xFF00_0000 | r << 16 | g << 8 | b
            } else if (x.wrapping_add(y).wrapping_add(layer.wrapping_mul(37)) & 127) < 12 {
                (s & 0x00FF_FFFF) | 0x8000_0000
            } else {
                0
            };
            row[x as usize * 4..x as usize * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
}

// 0 when b has a's size, layers, properties and pixels, else ERR_CHECKSUM  [pdo_cmp_docs]
fn cmp_docs(a: &PdnDoc, b: &PdnDoc) -> i32 {
    if a.w != b.w || a.h != b.h || a.layers.len() != b.layers.len() {
        return ERR_CHECKSUM;
    }
    for (x, y) in a.layers.iter().zip(&b.layers) {
        if [x.opacity, x.visible, x.blend, x.isbg] != [y.opacity, y.visible, y.blend, y.isbg] {
            return ERR_CHECKSUM;
        }
        if cut(&x.name) != cut(&y.name) {
            return ERR_CHECKSUM;
        }
        let rb = x.sf.w as usize * 4;
        let (sa, sb) = (x.sf.stride as usize, y.sf.stride as usize);
        for r in 0..x.sf.h as usize {
            if x.sf.px[r * sa..r * sa + rb] != y.sf.px[r * sb..r * sb + rb] {
                return ERR_CHECKSUM;
            }
        }
    }
    0
}
