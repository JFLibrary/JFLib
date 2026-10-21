// icc -- JPEG XL ICC profile decoding (icc_codec.cc, icc_codec_common.cc; model:
// tools/jxl_ref.py read_icc, unpredict_icc)                              [src/img/jxl_icc.s]
//
//   jx_icc(jx, br) -> 0 / ERR_*    entropy coded ICC stream from br, unpredicted into jx.icc

use alloc::vec::Vec;

use img_core::{ERR_CODE, ERR_NOMEM};

use crate::bits::Jb;
use crate::ent::{je_read, Jr};
use crate::hdr::Jx;
use crate::tab;

const ICC_MAXSIZE: u64 = 1 << 28;

/// jx_icc: read the encoded profile and unpredict it into jx.icc
pub fn jx_icc(jx: &mut Jx, br: &mut Jb) -> i32 {
    let size = br.u64f();
    if size > ICC_MAXSIZE {
        return ERR_CODE;
    }
    let jc = match je_read(br, 41, false) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let mut jr = match Jr::new(&jc, br, 0) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let n = size as usize;
    let mut enc: Vec<u8> = Vec::new();
    if enc.try_reserve_exact(n + 16).is_err() {
        return ERR_NOMEM;
    }
    enc.resize(n + 16, 0);
    for i in 0..n {
        let ctx = if i <= 128 {
            0
        } else {
            let k1 = tab::ICCKIND1[enc[i - 1] as usize] as u32;
            let k2 = tab::ICCKIND2[enc[i - 2] as usize] as u32;
            k1 + k2 * 8 + 1
        };
        enc[i] = jr.read(br, ctx) as u8;
        if br.err != 0 {
            return br.err;
        }
    }
    if !jr.fin() {
        return ERR_CODE;
    }
    match unpredict(&enc, size) {
        Ok(v) => {
            jx.icc = v;
            0
        }
        Err(e) => e,
    }
}

/// ic_varint: DecodeVarInt at *pos (at most 10 bytes, bounded by size)
fn varint(enc: &[u8], size: u64, pos: &mut u64) -> u64 {
    let p = *pos;
    let mut v: u64 = 0;
    let mut i: u64 = 0;
    loop {
        let q = p.wrapping_add(i);
        if q >= size || i >= 10 {
            break;
        }
        let b = enc[q as usize] as u64;
        v |= (b & 127) << (i * 7);
        if b & 128 == 0 {
            break;
        }
        i += 1;
    }
    *pos = p.wrapping_add(i).wrapping_add(1);
    v
}

/// ic_shuffle: dst[i] = src[j], j stepping by ceil(num / width)
fn shuffle(src: &[u8], num: usize, width: usize, dst: &mut [u8]) {
    if num == 0 {
        return;
    }
    let height = (num + width - 1) / width;
    let (mut s, mut j) = (0usize, 0usize);
    for d in dst.iter_mut().take(num) {
        *d = src[j];
        j += height;
        if j >= num {
            s += 1;
            j = s;
        }
    }
}

/// the predictor of order o from p1, p2, p3 (32-bit wrap)
#[inline]
fn pv(order: u32, p1: u32, p2: u32, p3: u32) -> u32 {
    match order {
        0 => p1,
        1 => p1.wrapping_add(p1).wrapping_sub(p2),
        _ => p1.wrapping_mul(3).wrapping_sub(p2.wrapping_mul(3)).wrapping_add(p3),
    }
}

/// ic_predict: LinearPredictICCValue
fn predict(out: &[u8], start: u64, i: u64, stride: u64, flags: u32) -> u32 {
    let width = (flags & 3) + 1;
    let order = (flags >> 2) & 3;
    let pos = start.wrapping_add(i);
    let at = |k: u64| -> u32 { out.get(k as usize).copied().unwrap_or(0) as u32 };
    match width {
        1 => {
            let a = pos.wrapping_sub(stride);
            let b = a.wrapping_sub(stride);
            let c = b.wrapping_sub(stride);
            pv(order, at(a), at(b), at(c)) & 255
        }
        2 => {
            let mut p = start.wrapping_add(i & !1).wrapping_sub(stride);
            let p1 = at(p) << 8 | at(p.wrapping_add(1));
            p = p.wrapping_sub(stride);
            let p2 = at(p) << 8 | at(p.wrapping_add(1));
            p = p.wrapping_sub(stride);
            let p3 = at(p) << 8 | at(p.wrapping_add(1));
            let mut v = pv(order, p1, p2, p3) & 0xFFFF;
            if i & 1 == 0 {
                v >>= 8;
            }
            v & 255
        }
        _ => {
            let be32 = |p: u64| -> u32 {
                if p.wrapping_add(4) > pos {
                    0
                } else {
                    at(p) << 24 | at(p + 1) << 16 | at(p + 2) << 8 | at(p + 3)
                }
            };
            let mut p = start.wrapping_add(i & !3).wrapping_sub(stride);
            let p1 = be32(p);
            p = p.wrapping_sub(stride);
            let p2 = be32(p);
            p = p.wrapping_sub(stride);
            let p3 = be32(p);
            let sh = (3 - (i as u32 & 3)) * 8;
            (pv(order, p1, p2, p3) >> sh) & 255
        }
    }
}

/// the output of ic_unpredict
struct Out {
    d: Vec<u8>,
    len: u64,
    osize: u64,
}

impl Out {
    fn put(&mut self, b: u8) -> Result<(), i32> {
        if self.len >= self.osize {
            return Err(ERR_CODE);
        }
        self.d[self.len as usize] = b;
        self.len += 1;
        Ok(())
    }
    fn put32(&mut self, v: u32) -> Result<(), i32> {
        if self.len + 4 > self.osize {
            return Err(ERR_CODE);
        }
        let o = self.len as usize;
        self.d[o..o + 4].copy_from_slice(&v.to_be_bytes());
        self.len += 4;
        Ok(())
    }
    fn puttag(&mut self, v: u32) -> Result<(), i32> {
        self.put32(v.swap_bytes())
    }
}

/// ic_unpredict: UnpredictICC
fn unpredict(enc: &[u8], size: u64) -> Result<Vec<u8>, i32> {
    let mut vp: u64 = 0;
    let osize = varint(enc, size, &mut vp);
    if osize > ICC_MAXSIZE {
        return Err(ERR_CODE);
    }
    let csize = varint(enc, size, &mut vp);
    let mut cpos = vp;
    let cend = cpos.checked_add(csize).ok_or(ERR_CODE)?;
    if cend > size {
        return Err(ERR_CODE);
    }
    let mut dpos = cend;
    let mut d: Vec<u8> = Vec::new();
    if d.try_reserve_exact(osize as usize + 16).is_err() {
        return Err(ERR_NOMEM);
    }
    d.resize(osize as usize + 16, 0);
    let mut o = Out { d, len: 0, osize };
    macro_rules! data {
        () => {{
            if dpos >= size {
                return Err(ERR_CODE);
            }
            let b = enc[dpos as usize];
            dpos += 1;
            b
        }};
    }
    macro_rules! cmd {
        () => {{
            if cpos >= cend {
                return Err(ERR_CODE);
            }
            let b = enc[cpos as usize] as u32;
            cpos += 1;
            b
        }};
    }
    macro_rules! room {
        ($n:expr) => {{
            let n: u64 = $n;
            match dpos.checked_add(n) {
                Some(e) if e <= size => {}
                _ => return Err(ERR_CODE),
            }
            match o.len.checked_add(n) {
                Some(e) if e <= osize => {}
                _ => return Err(ERR_CODE),
            }
        }};
    }
    // ---- header (128 bytes predicted)
    let mut hdr = tab::ICCHEADER;
    hdr[0..4].copy_from_slice(&(osize as u32).to_be_bytes());
    let mut i: u64 = 0;
    loop {
        if o.len == osize {
            o.d.truncate(o.len as usize);
            return Ok(o.d);
        }
        if i as u32 >= 128 {
            break;
        }
        if i == 8 {
            let v = [o.d[4], o.d[5], o.d[6], o.d[7]];
            hdr[80..84].copy_from_slice(&v);
        }
        if i == 41 {
            match o.d[40] {
                b'A' => hdr[41..44].copy_from_slice(b"PPL"),
                b'M' => hdr[41..44].copy_from_slice(b"SFT"),
                _ => {}
            }
        }
        if i == 42 && o.d[40] == b'S' {
            match o.d[41] {
                b'G' => hdr[42..44].copy_from_slice(b"I "),
                b'U' => hdr[42..44].copy_from_slice(b"NW"),
                _ => {}
            }
        }
        let b = data!();
        o.put(b.wrapping_add(hdr[i as usize]))?;
        i += 1;
    }
    // ---- tag table
    let v = varint(enc, size, &mut cpos);
    if v != 0 {
        let ntags = v - 1;
        o.put32(ntags as u32)?;
        let mut prevstart = ntags.wrapping_mul(12).wrapping_add(128);
        let mut prevsize: u64 = 0;
        while cpos < cend {
            let c = cmd!();
            let tc = c & 63;
            if tc == 0 {
                break;
            }
            let tag = match tc {
                1 => {
                    if dpos + 4 > size {
                        return Err(ERR_CODE);
                    }
                    let p = dpos as usize;
                    dpos += 4;
                    u32::from_le_bytes([enc[p], enc[p + 1], enc[p + 2], enc[p + 3]])
                }
                2 => tab::ICCTAGS[7],
                3 => tab::ICCTAGS[3],
                _ => {
                    if tc - 4 >= 17 {
                        return Err(ERR_CODE);
                    }
                    tab::ICCTAGS[(tc - 4) as usize]
                }
            };
            o.puttag(tag)?;
            let mut tsize = prevsize;
            if tab::ICCXYZ.contains(&tag) {
                tsize = 20;
            }
            let tstart = if c & 64 != 0 { varint(enc, size, &mut cpos) } else { prevstart.wrapping_add(prevsize) };
            o.put32(tstart as u32)?;
            if c & 128 != 0 {
                tsize = varint(enc, size, &mut cpos);
            }
            o.put32(tsize as u32)?;
            prevstart = tstart;
            prevsize = tsize;
            if tc == 2 {
                o.puttag(tab::ICCTAGS[8])?;
                o.put32(tstart as u32)?;
                o.put32(tsize as u32)?;
                o.puttag(tab::ICCTAGS[9])?;
                o.put32(tstart as u32)?;
                o.put32(tsize as u32)?;
            } else if tc == 3 {
                o.puttag(tab::ICCTAGS[4])?;
                o.put32(tstart.wrapping_add(tsize) as u32)?;
                o.put32(tsize as u32)?;
                o.puttag(tab::ICCTAGS[5])?;
                o.put32(tsize.wrapping_add(tsize).wrapping_add(tstart) as u32)?;
                o.put32(tsize as u32)?;
            }
        }
    }
    // ---- main commands
    let mut tmp: Vec<u8> = Vec::new();
    while cpos < cend {
        let c = cmd!();
        match c {
            1 => {
                let n = varint(enc, size, &mut cpos);
                room!(n);
                let (s, d) = (dpos as usize, o.len as usize);
                o.d[d..d + n as usize].copy_from_slice(&enc[s..s + n as usize]);
                dpos += n;
                o.len += n;
            }
            2 | 3 => {
                let n = varint(enc, size, &mut cpos);
                room!(n);
                let (s, d) = (dpos as usize, o.len as usize);
                let width = ((c - 1) * 2) as usize;
                shuffle(&enc[s..], n as usize, width, &mut o.d[d..]);
                dpos += n;
                o.len += n;
            }
            4 => {
                let flags = cmd!();
                let width = (flags & 3) + 1;
                if width == 3 || flags & 12 == 12 {
                    return Err(ERR_CODE);
                }
                let mut stride = width as u64;
                if flags & 16 != 0 {
                    let s = varint(enc, size, &mut cpos);
                    if s < width as u64 {
                        return Err(ERR_CODE);
                    }
                    stride = s;
                }
                if o.len == 0 || (o.len - 1) >> 2 < stride {
                    return Err(ERR_CODE);
                }
                let num = varint(enc, size, &mut cpos);
                room!(num);
                tmp.clear();
                if tmp.try_reserve_exact(num as usize + 16).is_err() {
                    return Err(ERR_NOMEM);
                }
                tmp.resize(num as usize + 16, 0);
                let s = dpos as usize;
                if width == 1 {
                    tmp[..num as usize].copy_from_slice(&enc[s..s + num as usize]);
                } else {
                    shuffle(&enc[s..], num as usize, width as usize, &mut tmp);
                }
                dpos += num;
                let start = o.len;
                for k in 0..num {
                    let p = predict(&o.d, start, k, stride, flags) as u8;
                    o.put(p.wrapping_add(tmp[k as usize]))?;
                }
            }
            10 => {
                o.puttag(tab::ICCTYPES[0])?;
                o.puttag(0)?;
                room!(12);
                let (s, d) = (dpos as usize, o.len as usize);
                o.d[d..d + 12].copy_from_slice(&enc[s..s + 12]);
                dpos += 12;
                o.len += 12;
            }
            16..=23 => {
                o.puttag(tab::ICCTYPES[(c - 16) as usize])?;
                o.puttag(0)?;
            }
            _ => return Err(ERR_CODE),
        }
    }
    if o.len != osize {
        return Err(ERR_CODE);
    }
    o.d.truncate(o.len as usize);
    Ok(o.d)
}
