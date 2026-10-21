//! PSD/PSB RGB and grayscale composites, plus the raster-layer document API.
//! High-depth sources are converted only for the 8-bit Surface API.
//! Format: https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/
use alloc::{vec, vec::Vec};
use gfx::{FMT_ARGB, Surface};
use img_core::*;
pub mod document;

pub static CODEC: Codec = Codec {
    probe: Some(probe),
    decode: Some(decode),
    encode: Some(encode),
    caps: IFC_DECODE | IFC_ENCODE,
    iom: IOM_ICC | IOM_EXIF | IOM_XMP,
    ..Codec::EMPTY
};
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], i32> {
        let end = self.pos.checked_add(n).ok_or(ERR_OVERFLOW)?;
        let v = self.data.get(self.pos..end).ok_or(ERR_TRUNC)?;
        self.pos = end;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16, i32> {
        let d = self.take(2)?;
        Ok(u16::from_be_bytes([d[0], d[1]]))
    }
    fn u32(&mut self) -> Result<u32, i32> {
        let d = self.take(4)?;
        Ok(u32::from_be_bytes([d[0], d[1], d[2], d[3]]))
    }
    fn section(&mut self) -> Result<&'a [u8], i32> {
        let n = self.u32()?;
        self.take(n as usize)
    }
    fn u64(&mut self) -> Result<u64, i32> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn large_section(&mut self, large: bool) -> Result<&'a [u8], i32> {
        let n = if large { self.u64()? } else { self.u32()? as u64 };
        self.take(usize::try_from(n).map_err(|_| ERR_LIMIT)?)
    }
}
fn header(data: &[u8], m: &mut ImgMeta) -> Result<(usize, usize, usize, bool), i32> {
    let mut r = Reader { data, pos: 0 };
    if r.take(4)? != b"8BPS" { return Err(ERR_HDR); }
    let version = r.u16()?;
    if ![1, 2].contains(&version) || r.take(6)? != [0; 6] {
        return Err(ERR_HDR);
    }
    let channels = r.u16()? as usize;
    let h = r.u32()? as usize;
    let w = r.u32()? as usize;
    let depth = r.u16()?;
    let mode = r.u16()?;
    let limit = if version == 2 { 300000 } else { 30000 };
    if channels == 0 || channels > 56 || w == 0 || h == 0 || w > limit || h > limit {
        return Err(ERR_DIMS);
    }
    if ![8, 16, 32].contains(&depth)
        || ![1, 3].contains(&mode)
        || channels < if mode == 3 { 3 } else { 1 }
        || channels > if mode == 3 { 4 } else { 2 }
    {
        return Err(ERR_UNSUPPORTED);
    }
    m.w = w as u32;
    m.h = h as u32;
    m.bits = depth as u8;
    m.channels = channels as u8;
    m.model = if mode == 3 { IMM_RGB } else { IMM_GRAY };
    m.frames = 1;
    m.orient = 1;
    m.flags = IMF_LOSSLESS | if depth == 32 { IMF_HDR | IMF_FLOAT } else { 0 }
        | if channels > if mode == 3 { 3 } else { 1 } {
            IMF_ALPHA
        } else {
            0
        };
    Ok((w, h, channels, mode == 1))
}
pub fn probe(data: &[u8], m: &mut ImgMeta) -> bool {
    header(data, m).is_ok()
}
pub fn has_layers(data: &[u8]) -> bool {
    let mut r = Reader { data, pos: 26 };
    (|| -> Result<bool, i32> {
        r.section()?;
        r.section()?;
        let section = r.large_section(data.get(5) == Some(&2))?;
        Ok(section.len() > 4)
    })()
    .unwrap_or(false)
}
fn packbits(data: &[u8], row: &mut [u8]) -> Result<(), i32> {
    let mut r = Reader { data, pos: 0 };
    let mut x = 0;
    while r.pos < data.len() {
        let count = r.take(1)?[0] as i8;
        match count {
            -128 => {}
            0..=127 => {
                let n = count as usize + 1;
                let dst = row.get_mut(x..x + n).ok_or(ERR_OVERFLOW)?;
                dst.copy_from_slice(r.take(n)?);
                x += n;
            }
            _ => {
                let n = (1 - count as i16) as usize;
                let v = r.take(1)?[0];
                row.get_mut(x..x + n).ok_or(ERR_OVERFLOW)?.fill(v);
                x += n;
            }
        }
    }
    if x != row.len() {
        return Err(ERR_TRUNC);
    }
    Ok(())
}
fn decoded(data: &[u8], dec: &ImgDec, m: &mut ImgMeta) -> Result<Surface, i32> {
    let (w, h, channels, gray) = header(data, m)?;
    let check = meta::dims_check(w as u32, h as u32, dec);
    if check != 0 {
        return Err(check);
    }
    let mut r = Reader { data, pos: 26 };
    r.section()?;
    resources(r.section()?, dec, m)?;
    r.large_section(data[5] == 2)?;
    let compression = r.u16()?;
    let bpc = m.bits as usize / 8;
    let row_bytes = w * bpc;
    let size = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(channels))
        .and_then(|n| n.checked_mul(bpc))
        .ok_or(ERR_OVERFLOW)?;
    let check = meta::mem_check((size + w * h * 4) as u64, dec);
    if check != 0 {
        return Err(check);
    }
    if dec.cancelled() {
        return Err(ERR_CANCEL);
    }
    let plane = match compression {
        0 => r.take(size)?.to_vec(),
        1 => {
            let mut lengths = Vec::with_capacity(channels * h);
            for _ in 0..channels * h {
                lengths.push(if data[5] == 2 { r.u32()? as usize } else { r.u16()? as usize });
            }
            let mut out = vec![0; size];
            for (i, n) in lengths.into_iter().enumerate() {
                if dec.cancelled() {
                    return Err(ERR_CANCEL);
                }
                packbits(r.take(n)?, &mut out[i * row_bytes..(i + 1) * row_bytes])?;
            }
            out
        }
        2 | 3 => {
            let mut out = vec![0; size];
            if inflate::zlib_inflate(&data[r.pos..], &mut out)? != size {
                return Err(ERR_TRUNC);
            }
            if compression == 3 {
                undo_prediction(&mut out, w, m.bits)?;
            }
            out
        }
        _ => return Err(ERR_UNSUPPORTED),
    };
    let mut sf = meta::surf_alloc(w as u32, h as u32, FMT_ARGB, dec)?;
    let alpha = if gray { 1 } else { 3 };
    for y in 0..h {
        if dec.cancelled() {
            return Err(ERR_CANCEL);
        }
        for x in 0..w {
            let i = y * w + x;
            let p = y * sf.stride as usize + x * 4;
            for c in 0..3 {
                sf.px[p + c] = sample(&plane, (if gray { 0 } else { 2 - c }) * w * h + i, m.bits, false);
            }
            sf.px[p + 3] = if channels > alpha {
                sample(&plane, alpha * w * h + i, m.bits, true)
            } else {
                255
            };
        }
    }
    Ok(sf)
}

fn undo_prediction(data: &mut [u8], w: usize, bits: u8) -> Result<(), i32> {
    let bpc = bits as usize / 8;
    if w == 0 || ![1, 2, 4].contains(&bpc) { return Err(ERR_DIMS); }
    let mut shuffled = if bits == 32 { vec![0; w * 4] } else { Vec::new() };
    for row in data.chunks_exact_mut(w * bpc) {
        if bits == 16 {
            let mut last = 0u16;
            for item in row.chunks_exact_mut(2) {
                last = last.wrapping_add(u16::from_be_bytes([item[0], item[1]]));
                item.copy_from_slice(&last.to_be_bytes());
            }
        } else {
            for x in 1..row.len() { row[x] = row[x].wrapping_add(row[x - 1]); }
            if bits == 32 {
                shuffled.copy_from_slice(row);
                for x in 0..w { for c in 0..4 { row[x * 4 + c] = shuffled[c * w + x]; } }
            }
        }
    }
    Ok(())
}

fn sample(data: &[u8], index: usize, bits: u8, alpha: bool) -> u8 {
    match bits {
        16 => ((u16::from_be_bytes([data[index * 2], data[index * 2 + 1]]) as u32 + 128) / 257) as u8,
        32 => {
            let x = f32::from_be_bytes(data[index * 4..index * 4 + 4].try_into().unwrap()) as f64;
            let x = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
            let x = if alpha { x } else if x <= 0.0031308 { x * 12.92 } else { 1.055 * rt::x87::pow(x, 1.0 / 2.4) - 0.055 };
            (x * 255.0 + 0.5).clamp(0.0, 255.0) as u8
        },
        _ => data[index],
    }
}
pub fn decode(data: &[u8], sf: &mut Surface, dec: &ImgDec, m: &mut ImgMeta) -> i32 {
    match decoded(data, dec, m) {
        Ok(s) => {
            *sf = s;
            0
        }
        Err(e) => e,
    }
}
pub fn encode(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    if sf.w < 1 || sf.h < 1 || sf.w > 30000 || sf.h > 30000 {
        return ERR_DIMS;
    }
    let (w, h) = (sf.w as usize, sf.h as usize);
    out.clear();
    out.extend_from_slice(b"8BPS\0\x01\0\0\0\0\0\0\0\x04");
    out.extend_from_slice(&(h as u32).to_be_bytes());
    out.extend_from_slice(&(w as u32).to_be_bytes());
    out.extend_from_slice(&[0, 8, 0, 3]);
    out.extend_from_slice(&[0; 4]); // no color-mode data
    let mut resources = Vec::new();
    if let Some(m) = opt.meta {
        if m.dpix != 0 || m.dpiy != 0 {
            let mut resolution = Vec::new();
            for dpi in [m.dpix, m.dpiy] {
                resolution.extend_from_slice(&dpi.max(1).to_be_bytes());
                resolution.extend_from_slice(&[0, 1, 0, 1]); // dpi, display inches
            }
            resource(&mut resources, 1005, &resolution);
        }
        for (id, flag, data) in [
            (1039, IOF_EMBED_ICC, &m.icc),
            (1058, IOF_EMBED_EXIF, &m.exif),
            (1060, IOF_EMBED_XMP, &m.xmp),
        ] {
            if opt.flags & flag != 0 && !data.is_empty() {
                resource(&mut resources, id, data);
            }
        }
    }
    out.extend_from_slice(&(resources.len() as u32).to_be_bytes());
    out.extend_from_slice(&resources);
    out.extend_from_slice(&[0; 4]); // no layer/mask data
    out.extend_from_slice(&[0, 0]);
    let mut row = vec![0; w * 4];
    for c in [2, 1, 0, 3] {
        for y in 0..h {
            if opt.cancelled() {
                out.clear();
                return ERR_CANCEL;
            }
            meta::get_row(sf, y, &mut row, FMT_ARGB);
            out.extend(row.chunks_exact(4).map(|p| p[c]));
        }
    }
    0
}

fn resources(data: &[u8], dec: &ImgDec, m: &mut ImgMeta) -> Result<(), i32> {
    let mut r = Reader { data, pos: 0 };
    while r.pos < data.len() {
        if dec.cancelled() {
            return Err(ERR_CANCEL);
        }
        if r.take(4)? != b"8BIM" {
            return Err(ERR_HDR);
        }
        let id = r.u16()?;
        let name = r.take(1)?[0] as usize;
        r.take(name + ((name + 1) & 1))?;
        let size = r.u32()? as usize;
        let bytes = r.take(size)?;
        r.take(size & 1)?;
        match id {
            1005 if size == 16 => {
                m.dpix = u32::from_be_bytes(bytes[..4].try_into().unwrap());
                m.dpiy = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
                m.resunit = if bytes[4..6] == [0, 2] { IMR_CM } else { IMR_INCH };
            }
            1039 if dec.flags & IMGD_ICC != 0 => m.icc = bytes.to_vec(),
            1058 if dec.flags & IMGD_EXIF != 0 => m.exif = bytes.to_vec(),
            1060 if dec.flags & IMGD_XMP != 0 => m.xmp = bytes.to_vec(),
            _ => {}
        }
    }
    Ok(())
}
fn resource(out: &mut Vec<u8>, id: u16, bytes: &[u8]) {
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
    if bytes.len() & 1 != 0 {
        out.push(0);
    }
}
