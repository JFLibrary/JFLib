// eicc -- ICC profile encoding of the image header (libjxl 0.11.1 enc_icc_codec.cc PredictICC,
// WriteICC; icc_codec_common.cc); the inverse of icc.rs                [jxl-plus: no asm]
//
//   predict_icc(icc) -> bytes      PredictICC: header predicted, tag list commands, typed
//                                  element commands (curves / mluc / XYZ / LUT grids predicted
//                                  and shuffled), the rest inserted
//   write_icc(jw, icc)             U64 size + the predicted bytes entropy coded with the 41
//                                  ICC contexts (prefix codes, LZ77)

use alloc::vec::Vec;

use crate::eans::{self, HParams, Lz, Token};
use crate::ewr::Jw;
use crate::tab;

const HDR: usize = 128;
const LIMIT: usize = (u32::MAX >> 2) as usize;

fn tag(b: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*b)
}

fn kw(icc: &[u8], pos: usize) -> u32 {
    if pos + 4 > icc.len() {
        return 0;
    }
    u32::from_le_bytes([icc[pos], icc[pos + 1], icc[pos + 2], icc[pos + 3]])
}

fn be32(icc: &[u8], pos: usize) -> u32 {
    if pos + 4 > icc.len() {
        return 0;
    }
    u32::from_be_bytes([icc[pos], icc[pos + 1], icc[pos + 2], icc[pos + 3]])
}

fn varint(v: u64, out: &mut Vec<u8>) {
    let mut v = v;
    while v > 127 {
        out.push((v & 127) as u8 | 128);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Unshuffle: transpose width columns x ceil(n / width) rows
fn unshuffle(d: &mut [u8], width: usize) {
    let n = d.len();
    if n == 0 {
        return;
    }
    let height = n.div_ceil(width);
    let mut r = alloc::vec![0u8; n];
    let (mut s, mut j) = (0usize, 0usize);
    for &b in d.iter() {
        r[j] = b;
        j += height;
        if j >= n {
            s += 1;
            j = s;
        }
    }
    d.copy_from_slice(&r);
}

fn pv(order: u32, p1: u32, p2: u32, p3: u32) -> u32 {
    match order {
        0 => p1,
        1 => p1.wrapping_add(p1).wrapping_sub(p2),
        _ => p1.wrapping_mul(3).wrapping_sub(p2.wrapping_mul(3)).wrapping_add(p3),
    }
}

/// LinearPredictICCValue over the original bytes
fn lin_predict(d: &[u8], start: usize, i: usize, stride: usize, width: usize, order: u32) -> u8 {
    let pos = start + i;
    match width {
        1 => pv(order, d[pos - stride] as u32, d[pos - 2 * stride] as u32, d[pos - 3 * stride] as u32) as u8,
        2 => {
            let p = start + (i & !1);
            let r = |o: usize| (d[o] as u32) << 8 | d[o + 1] as u32;
            let v = pv(order, r(p - stride), r(p - 2 * stride), r(p - 3 * stride)) & 0xFFFF;
            if i & 1 != 0 { v as u8 } else { (v >> 8) as u8 }
        }
        _ => {
            let p = start + (i & !3);
            let r = |o: usize| if o + 4 > pos { 0 } else { be32(d, o) };
            let v = pv(order, r(p - stride), r(p - 2 * stride), r(p - 3 * stride));
            (v >> ((3 - (i & 3)) * 8)) as u8
        }
    }
}

/// PredictAndShuffle
fn predict_shuffle(stride: usize, width: usize, order: u32, num: usize, d: &[u8], pos: &mut usize, out: &mut Vec<u8>) -> bool {
    if *pos + num > d.len() || *pos == 0 || ((*pos - 1) >> 2) < stride || *pos < stride * 4 {
        return false;
    }
    let start = out.len();
    for i in 0..num {
        let p = lin_predict(d, *pos, i, stride, width, order);
        out.push(d[*pos + i].wrapping_sub(p));
    }
    *pos += num;
    if width > 1 {
        unshuffle(&mut out[start..], width);
    }
    true
}

/// PredictICC
pub fn predict_icc(icc: &[u8]) -> Vec<u8> {
    let size = icc.len();
    let mut result = Vec::with_capacity(size + 32);
    let mut commands: Vec<u8> = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    varint(size as u64, &mut result);
    // header
    let mut header = tab::ICCHEADER;
    header[0..4].copy_from_slice(&(size as u32).to_be_bytes());
    for i in 0..HDR.min(size) {
        if i == 8 && size >= 8 {
            header[80..84].copy_from_slice(&icc[4..8]);
        }
        if i == 41 && size >= 41 {
            if icc[40] == b'A' {
                header[41..44].copy_from_slice(b"PPL");
            }
            if icc[40] == b'M' {
                header[41..44].copy_from_slice(b"SFT");
            }
        }
        if i == 42 && size >= 42 && icc[40] == b'S' {
            if icc[41] == b'G' {
                header[42..44].copy_from_slice(b"I ");
            }
            if icc[41] == b'U' {
                header[42..44].copy_from_slice(b"NW");
            }
        }
        data.push(icc[i].wrapping_sub(header[i]));
    }
    if size <= HDR {
        varint(0, &mut result);
        result.extend_from_slice(&data);
        return result;
    }
    let rtrc = tag(b"rTRC");
    let gtrc = tag(b"gTRC");
    let btrc = tag(b"bTRC");
    let rxyz = tag(b"rXYZ");
    let gxyz = tag(b"gXYZ");
    let bxyz = tag(b"bXYZ");
    let mut tagstarts: Vec<usize> = Vec::new();
    let mut tagsizes: Vec<usize> = Vec::new();
    let mut tagmap: Vec<(usize, usize)> = Vec::new(); // (start, index), last wins
    let mut pos = HDR;
    if pos + 4 <= size {
        let numtags = be32(icc, pos) as u64;
        pos += 4;
        varint(numtags + 1, &mut commands);
        let mut prevstart = HDR as u64 + numtags * 12;
        let mut prevsize: u64 = 0;
        let mut i = 0u64;
        while i < numtags {
            if pos + 12 > size {
                break;
            }
            let t = kw(icc, pos);
            let tstart = be32(icc, pos + 4) as u64;
            let tsize = be32(icc, pos + 8) as u64;
            pos += 12;
            tagstarts.push(tstart as usize);
            tagsizes.push(tsize as usize);
            let idx = tagstarts.len() - 1;
            if let Some(e) = tagmap.iter_mut().find(|e| e.0 == tstart as usize) {
                e.1 = idx;
            } else {
                tagmap.push((tstart as usize, idx));
            }
            let mut code: u8 = 1;
            for (j, &s) in tab::ICCTAGS.iter().enumerate() {
                if t == s {
                    code = (j + 4) as u8;
                    break;
                }
            }
            if t == rtrc && pos + 24 < size {
                let mut ok = kw(icc, pos) == gtrc && kw(icc, pos + 12) == btrc;
                if ok {
                    for k in 0..8 {
                        if icc[pos - 8 + k] != icc[pos + 4 + k] || icc[pos - 8 + k] != icc[pos + 16 + k] {
                            ok = false;
                        }
                    }
                }
                if ok {
                    code = 2;
                    pos += 24;
                    i += 2;
                }
            }
            if t == rxyz && pos + 24 < size {
                let mut ok = kw(icc, pos) == gxyz && kw(icc, pos + 12) == bxyz;
                let og = be32(icc, pos + 4) as u64;
                let ob = be32(icc, pos + 16) as u64;
                let sg = be32(icc, pos + 8);
                let sb = be32(icc, pos + 20);
                ok &= tsize == 20 && sg == 20 && sb == 20;
                ok &= og == tstart + 20 && ob == tstart + 40;
                if ok {
                    code = 3;
                    pos += 24;
                    i += 2;
                }
            }
            let mut cmd = code;
            if prevstart + prevsize != tstart {
                cmd |= 64;
            }
            let mut psize = prevsize;
            if tab::ICCXYZ.contains(&t) {
                psize = 20;
            }
            if psize != tsize {
                cmd |= 128;
            }
            commands.push(cmd);
            if code == 1 {
                data.extend_from_slice(&t.to_le_bytes());
            }
            if cmd & 64 != 0 {
                varint(tstart, &mut commands);
            }
            if cmd & 128 != 0 {
                varint(tsize, &mut commands);
            }
            prevstart = tstart;
            prevsize = tsize;
            i += 1;
        }
    }
    commands.push(0);
    // main content
    let mluc = tag(b"mluc");
    let curv = tag(b"curv");
    let mab = tag(b"mAB ");
    let mba = tag(b"mBA ");
    let vcgt = tag(b"vcgt");
    let gbd = tag(b"gbd ");
    let xyz = tag(b"XYZ ");
    let mut t: u32 = 0;
    let mut tstart = 0usize;
    let mut tsize = 0usize;
    let mut clutstart = 0usize;
    let mut last0 = pos;
    while pos <= size {
        let mut last1 = pos;
        let mut cadd: Vec<u8> = Vec::new();
        let mut dadd: Vec<u8> = Vec::new();
        let sane = |ts: usize| ts > 8 && ts < LIMIT;
        if pos > tstart + tsize && tsize < LIMIT {
            t = 0;
        }
        if let Some(&(_, index)) = tagmap.iter().find(|e| e.0 == pos)
            && pos + 4 <= size
        {
            t = kw(icc, pos);
            tstart = tagstarts[index];
            tsize = tagsizes[index];
            if t == mluc && sane(tsize) && pos + tsize <= size && icc[pos + 4..pos + 8] == [0, 0, 0, 0] {
                let num = tsize - 8;
                cadd.push(16 + 3);
                pos += 8;
                cadd.push(2);
                varint(num as u64, &mut cadd);
                let st = dadd.len();
                dadd.extend_from_slice(&icc[pos..pos + num]);
                pos += num;
                unshuffle(&mut dadd[st..], 2);
            }
            if t == curv && sane(tsize) && pos + tsize <= size && icc[pos + 4..pos + 8] == [0, 0, 0, 0] {
                let num = tsize - 8;
                if num > 16 && num < (1 << 28) && pos + num <= size && pos > 0 {
                    let mut c2 = cadd.clone();
                    let mut d2 = dadd.clone();
                    let mut p2 = pos + 8;
                    c2.push(16 + 5);
                    c2.push(4);
                    c2.push((1 << 2) | 1);
                    varint(num as u64, &mut c2);
                    if predict_shuffle(2, 2, 1, num, icc, &mut p2, &mut d2) {
                        cadd = c2;
                        dadd = d2;
                        pos = p2;
                    }
                }
            }
        }
        if t == mab || t == mba {
            let sub = kw(icc, pos);
            if pos + 12 < size && (sub == curv || sub == vcgt) && be32(icc, pos + 4) == 0 {
                let num = be32(icc, pos + 8) as usize * 2;
                if num > 16 && num < (1 << 28) && pos + 12 + num <= size {
                    let mut p2 = pos + 12;
                    let mut c2 = cadd.clone();
                    let mut d2 = dadd.clone();
                    c2.push(4);
                    c2.push((1 << 2) | 1);
                    varint(num as u64, &mut c2);
                    if predict_shuffle(2, 2, 1, num, icc, &mut p2, &mut d2) {
                        last1 = pos + 12;
                        pos = p2;
                        cadd = c2;
                        dadd = d2;
                    }
                }
            }
            if pos == tstart + 24 && pos + 4 < size {
                clutstart = tstart + be32(icc, pos) as usize;
            }
            if pos == clutstart && clutstart + 16 < size {
                let numi = icc[tstart + 8] as usize;
                let numo = icc[tstart + 9] as usize;
                let width = icc[clutstart + 16] as usize;
                let stride = width * numo;
                let mut num = width * numo;
                for i in 0..numi {
                    if clutstart + i < size {
                        num *= icc[clutstart + i] as usize;
                    }
                }
                if (width == 1 || width == 2) && num > 64 && num < (1 << 28) && pos + num <= size && pos > stride * 4 {
                    let flags = (1u8 << 2) | (width as u8 - 1) | if stride == width { 0 } else { 16 };
                    let mut p2 = pos;
                    let mut c2 = cadd.clone();
                    let mut d2 = dadd.clone();
                    c2.push(4);
                    c2.push(flags);
                    if flags & 16 != 0 {
                        varint(stride as u64, &mut c2);
                    }
                    varint(num as u64, &mut c2);
                    if predict_shuffle(stride, width, 1, num, icc, &mut p2, &mut d2) {
                        pos = p2;
                        cadd = c2;
                        dadd = d2;
                    }
                }
            }
        }
        if cadd.is_empty() && dadd.is_empty() && t == gbd && sane(tsize) && pos == tstart + 8 && pos + tsize - 8 <= size && pos > 16 {
            let num = tsize - 8;
            let mut p2 = pos;
            let mut c2 = Vec::new();
            let mut d2 = Vec::new();
            c2.push(4);
            c2.push(3);
            varint(num as u64, &mut c2);
            if predict_shuffle(4, 4, 0, num, icc, &mut p2, &mut d2) {
                pos = p2;
                cadd = c2;
                dadd = d2;
            }
        }
        if cadd.is_empty() && dadd.is_empty() && pos + 20 <= size && kw(icc, pos) == xyz && be32(icc, pos + 4) == 0 {
            cadd.push(10);
            pos += 8;
            dadd.extend_from_slice(&icc[pos..pos + 12]);
            pos += 12;
        }
        if cadd.is_empty() && dadd.is_empty() && pos + 8 <= size && be32(icc, pos + 4) == 0 {
            let sub = kw(icc, pos);
            for (i, &ty) in tab::ICCTYPES.iter().enumerate() {
                if sub == ty {
                    cadd.push(16 + i as u8);
                    pos += 8;
                    break;
                }
            }
        }
        let added = !(cadd.is_empty() && dadd.is_empty());
        if added || pos == size {
            if last0 < last1 {
                commands.push(1);
                varint((last1 - last0) as u64, &mut commands);
                data.extend_from_slice(&icc[last0..last1]);
            }
            commands.extend_from_slice(&cadd);
            data.extend_from_slice(&dadd);
            last0 = pos;
        }
        if !added {
            pos += 1;
        }
    }
    varint(commands.len() as u64, &mut result);
    result.extend_from_slice(&commands);
    result.extend_from_slice(&data);
    result
}

/// U64 field of any value (U64Coder)
pub fn write_u64(jw: &mut Jw, v: u64) {
    if v == 0 {
        jw.bits(0, 2);
    } else if v <= 16 {
        jw.bits(1, 2);
        jw.bits(v - 1, 4);
    } else if v <= 272 {
        jw.bits(2, 2);
        jw.bits(v - 17, 8);
    } else {
        jw.bits(3, 2);
        jw.bits(v & 4095, 12);
        let mut v = v >> 12;
        let mut shift = 12;
        while v > 0 && shift < 60 {
            jw.bits(1, 1);
            jw.bits(v & 255, 8);
            v >>= 8;
            shift += 8;
        }
        if v > 0 {
            jw.bits(1, 1);
            jw.bits(v & 15, 4);
        } else {
            jw.bits(0, 1);
        }
    }
}

/// WriteICC
pub fn write_icc(jw: &mut Jw, icc: &[u8]) {
    let enc = predict_icc(icc);
    write_u64(jw, enc.len() as u64);
    let mut toks = Vec::with_capacity(enc.len());
    for i in 0..enc.len() {
        let ctx = if i <= 128 {
            0
        } else {
            1 + tab::ICCKIND1[enc[i - 1] as usize] as u32 + tab::ICCKIND2[enc[i - 2] as usize] as u32 * 8
        };
        toks.push(Token::new(ctx, enc[i] as u32));
    }
    let p = HParams { lz: Lz::Lz77, force_huffman: true, ..HParams::default() };
    let mut streams = alloc::vec![toks];
    let code = eans::build(&p, 41, &mut streams, Some(jw));
    eans::write_tokens(&streams[0], &code, jw);
}

// ------------------------------------------------------------- sRGB detection  [jxl-plus]

/// the tag (offset, size) of signature sig
fn find_tag(icc: &[u8], sig: &[u8; 4]) -> Option<(usize, usize)> {
    if icc.len() < 132 {
        return None;
    }
    let n = be32(icc, 128) as usize;
    for i in 0..n.min(1000) {
        let p = 132 + i * 12;
        if p + 12 > icc.len() {
            return None;
        }
        if &icc[p..p + 4] == sig {
            let off = be32(icc, p + 4) as usize;
            let size = be32(icc, p + 8) as usize;
            if off.checked_add(size).is_some_and(|e| e <= icc.len()) {
                return Some((off, size));
            }
            return None;
        }
    }
    None
}

/// s15Fixed16
fn s15(icc: &[u8], p: usize) -> f64 {
    be32(icc, p) as i32 as f64 / 65536.0
}

/// the XYZ of an XYZType tag
fn xyz_tag(icc: &[u8], sig: &[u8; 4]) -> Option<[f64; 3]> {
    let (o, n) = find_tag(icc, sig)?;
    if n < 20 || &icc[o..o + 4] != b"XYZ " {
        return None;
    }
    Some([s15(icc, o + 8), s15(icc, o + 12), s15(icc, o + 16)])
}

/// x^e for x >= 0
fn pw(x: f64, e: f64) -> f64 {
    crate::emath::powf(x.max(0.0) as f32, e as f32) as f64
}

/// a tone curve (curv / para) at x in 0..1
fn trc_at(icc: &[u8], sig: &[u8; 4], x: f64) -> Option<f64> {
    let (o, n) = find_tag(icc, sig)?;
    if n < 12 {
        return None;
    }
    match &icc[o..o + 4] {
        b"curv" => {
            let cnt = be32(icc, o + 8) as usize;
            if cnt == 0 {
                return Some(x);
            }
            if n < 12 + 2 * cnt {
                return None;
            }
            let at = |i: usize| ((icc[o + 12 + 2 * i] as u32) << 8 | icc[o + 13 + 2 * i] as u32) as f64;
            if cnt == 1 {
                return Some(pw(x, at(0) / 256.0));
            }
            let f = x * (cnt - 1) as f64;
            let i = (f as usize).min(cnt - 2);
            let fr = f - i as f64;
            Some((at(i) * (1.0 - fr) + at(i + 1) * fr) / 65535.0)
        }
        b"para" => {
            let ty = (icc[o + 8] as usize) << 8 | icc[o + 9] as usize;
            let np = [1usize, 3, 4, 5, 7];
            if ty > 4 || n < 12 + 4 * np[ty] {
                return None;
            }
            let p: Vec<f64> = (0..np[ty]).map(|k| s15(icc, o + 12 + 4 * k)).collect();
            let g = p[0];
            Some(match ty {
                0 => pw(x, g),
                1 => {
                    if x >= -p[2] / p[1] { pw(p[1] * x + p[2], g) } else { 0.0 }
                }
                2 => {
                    if x >= -p[2] / p[1] { pw(p[1] * x + p[2], g) + p[3] } else { p[3] }
                }
                3 => {
                    if x >= p[4] { pw(p[1] * x + p[2], g) } else { p[3] * x }
                }
                _ => {
                    if x >= p[4] { pw(p[1] * x + p[2], g) + p[5] } else { p[3] * x + p[6] }
                }
            })
        }
        _ => None,
    }
}

/// whether an RGB matrix / TRC profile describes sRGB (D50 adapted primaries and the sRGB
/// tone curve within a small tolerance): such images are coded like sRGB ones (XYB)
pub fn icc_srgb_like(icc: &[u8]) -> bool {
    if icc.len() < 132 || &icc[36..40] != b"acsp" || &icc[16..20] != b"RGB " || &icc[20..24] != b"XYZ " {
        return false;
    }
    static PRIM: [(&[u8; 4], [f64; 3]); 3] = [
        (b"rXYZ", [0.43607, 0.22249, 0.01392]),
        (b"gXYZ", [0.38515, 0.71687, 0.09708]),
        (b"bXYZ", [0.14307, 0.06061, 0.71410]),
    ];
    for (sig, want) in PRIM.iter() {
        let Some(v) = xyz_tag(icc, sig) else { return false };
        for k in 0..3 {
            if (v[k] - want[k]).abs() > 0.004 {
                return false;
            }
        }
    }
    for sig in [b"rTRC", b"gTRC", b"bTRC"] {
        for x in [0.02f64, 0.05, 0.1, 0.2, 0.35, 0.5, 0.7, 0.9] {
            let want = if x <= 0.04045 { x / 12.92 } else { pw((x + 0.055) / 1.055, 2.4) };
            match trc_at(icc, sig, x) {
                Some(v) if (v - want).abs() <= 0.006 => {}
                _ => return false,
            }
        }
    }
    true
}
