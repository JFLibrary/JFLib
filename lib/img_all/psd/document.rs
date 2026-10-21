//! Raster layer interchange. Unsupported Photoshop constructs are explicitly marked
//! as lossy or rejected so the application can protect the source document.
use super::{Reader, header, packbits, resources, sample, undo_prediction};
use alloc::{string::String, vec, vec::Vec};
use gfx::{Surface, FMT_ARGB};
use img_core::*;

pub struct Layer {
    pub pixels: Surface,
    pub name: String,
    pub opacity: u8,
    pub visible: bool,
    pub blend: u8,
}
pub struct Document {
    pub meta: ImgMeta,
    /// PaintCraft order: bottom layer first.
    pub layers: Vec<Layer>,
    /// Mask application, cropped off-canvas pixels, high depth or Photoshop-specific blocks.
    pub converted: bool,
}
pub struct LayerRef<'a> {
    pub pixels: &'a Surface,
    pub name: &'a str,
    pub visible: bool,
    pub opacity: u8,
    pub blend: u8,
}
#[derive(Clone, Copy)]
struct Bounds { top: i32, left: i32, bottom: i32, right: i32 }
impl Bounds {
    fn read(r: &mut Reader) -> Result<Self, i32> {
        Ok(Self { top: r.u32()? as i32, left: r.u32()? as i32, bottom: r.u32()? as i32, right: r.u32()? as i32 })
    }
    fn size(self) -> Result<(usize, usize), i32> {
        let (w, h) = (self.right as i64 - self.left as i64, self.bottom as i64 - self.top as i64);
        if w < 0 || h < 0 || w > IMG_MAXDIM as i64 || h > IMG_MAXDIM as i64 { return Err(ERR_DIMS); }
        Ok((w as usize, h as usize))
    }
}
struct Record {
    rect: Bounds,
    channels: Vec<(i16, usize)>,
    name: String,
    opacity: u8,
    visible: bool,
    blend: u8,
    mask: Option<(Bounds, u8, u8)>,
}

// Modes with a native equivalent. Other modes must use the saved composite.
const MODES: [(u8, &[u8; 4]); 10] = [
    (0,b"norm"), (1,b"mul "), (2,b"lddg"), (3,b"idiv"), (4,b"div "),
    (7,b"over"), (8,b"diff"), (10,b"lite"), (11,b"dark"), (12,b"scrn"),
];

pub fn decode(data: &[u8], dec: &ImgDec) -> Result<Option<Document>, i32> {
    let mut meta = ImgMeta { fmt: IMGF_PSD, ..ImgMeta::default() };
    let (w, h, _, gray) = header(data, &mut meta)?;
    let error = meta::dims_check(w as u32, h as u32, dec);
    if error != 0 { return Err(error); }
    let large = data[5] == 2;
    let mut r = Reader { data, pos: 26 };
    r.section()?;
    resources(r.section()?, dec, &mut meta)?;
    let section = r.large_section(large)?;
    if section.is_empty() { return Ok(None); }
    let mut r = Reader { data: section, pos: 0 };
    let mut layers = r.large_section(large)?;
    // Photoshop stores high-depth layer info in a tagged block after the global mask.
    if meta.bits != 8 {
        if r.pos < section.len() { r.section()?; }
        while r.pos + 12 <= section.len() {
            if ![b"8BIM".as_slice(), b"8B64"].contains(&r.take(4)?) { return Err(ERR_HDR); }
            let key = r.take(4)?;
            let long = large && [b"LMsk".as_slice(),b"Lr16",b"Lr32",b"Layr",b"Mt16",b"Mt32",b"Mtrn",b"Alph",b"FMsk",b"lnk2",b"FEid",b"FXid",b"PxSD"].contains(&key);
            let block = r.large_section(long)?;
            if key == if meta.bits == 16 { b"Lr16" } else { b"Lr32" } { layers = block; }
            if block.len() % 2 != 0 { r.take(1)?; }
        }
    }
    if layers.is_empty() { return Ok(None); }
    let mut r = Reader { data: layers, pos: 0 };
    let count = (r.u16()? as i16 as i32).unsigned_abs() as usize;
    if count == 0 { return Ok(None); }
    if count > 4096 || (w as u64) * (h as u64) * (count as u64) > dec.max_pixels() { return Err(ERR_LIMIT); }
    let surface_bytes=(w as u64) * (h as u64) * count as u64 * 4;
    let error = meta::mem_check(surface_bytes, dec);
    if error != 0 { return Err(error); }
    let mut converted = meta.bits != 8 || large; // the writer emits PSD, not PSB
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        if dec.cancelled() { return Err(ERR_CANCEL); }
        let rect = Bounds::read(&mut r)?; rect.size()?;
        let count = r.u16()? as usize;
        if count > 56 { return Err(ERR_HDR); }
        let mut channels = Vec::with_capacity(count);
        for _ in 0..count {
            let id = r.u16()? as i16;
            let size = if large { r.u64()? } else { r.u32()? as u64 };
            if channels.iter().any(|&(other, _)| other == id) { return Err(ERR_HDR); }
            channels.push((id, usize::try_from(size).map_err(|_| ERR_LIMIT)?));
        }
        if r.take(4)? != b"8BIM" { return Err(ERR_HDR); }
        let key = r.take(4)?;
        let blend = MODES.iter().find(|(_, k)| k.as_slice() == key).ok_or(ERR_UNSUPPORTED)?.0;
        let props = r.take(4)?;
        if props[1] != 0 || props[2] & 16 != 0 { return Err(ERR_UNSUPPORTED); }
        let extra = r.section()?;
        let mut e = Reader { data: extra, pos: 0 };
        let mask_data = e.section()?;
        let mask = if !mask_data.is_empty() {
            converted = true;
            let mut mr = Reader { data: mask_data, pos: 0 };
            let mut bounds = Bounds::read(&mut mr)?;
            let flags = mr.take(2)?;
            if flags[1] & 1 != 0 {
                bounds.left = bounds.left.checked_add(rect.left).ok_or(ERR_DIMS)?;
                bounds.right = bounds.right.checked_add(rect.left).ok_or(ERR_DIMS)?;
                bounds.top = bounds.top.checked_add(rect.top).ok_or(ERR_DIMS)?;
                bounds.bottom = bounds.bottom.checked_add(rect.top).ok_or(ERR_DIMS)?;
            }
            bounds.size()?;
            if flags[1] & 16 != 0 || mask_data.len() > 20 { return Err(ERR_UNSUPPORTED); }
            Some((bounds, flags[0], flags[1]))
        } else { None };
        let ranges = e.section()?;
        if !ranges.is_empty() && ranges.chunks_exact(8).any(|v| v != [0,0,255,255,0,0,255,255]) { return Err(ERR_UNSUPPORTED); }
        let n = e.take(1)?[0] as usize;
        let mut name: String = e.take(n)?.iter().map(|&c| c as char).collect();
        e.take((4 - (n + 1) % 4) % 4)?;
        while e.pos + 12 <= extra.len() {
            if ![b"8BIM".as_slice(),b"8B64"].contains(&e.take(4)?) { return Err(ERR_HDR); }
            let key = e.take(4)?;
            let block = e.section()?;
            if block.len() & 1 != 0 { e.take(1)?; }
            match key {
                b"luni" => {
                    let mut u = Reader { data: block, pos: 0 };
                    let count = u.u32()? as usize;
                    if count > 65535 { return Err(ERR_LIMIT); }
                    let bytes = u.take(count * 2)?;
                    name = core::char::decode_utf16(bytes.chunks_exact(2).map(|v| u16::from_be_bytes([v[0],v[1]]))).map(|c| c.unwrap_or('\u{fffd}')).collect();
                },
                b"lsct" | b"lsdk" | b"vmsk" | b"vsms" | b"lrFX" | b"lfx2" => return Err(ERR_UNSUPPORTED),
                b"lyid" | b"lspf" | b"clbl" | b"infx" | b"lnsr" => {},
                _ => converted = true,
            }
        }
        converted |= rect.left < 0 || rect.top < 0 || rect.right as i64 > w as i64 || rect.bottom as i64 > h as i64;
        records.push(Record { rect, channels, name, opacity: props[0], visible: props[2] & 2 == 0, blend, mask });
    }
    let mut output = Vec::with_capacity(count);
    for record in records {
        if dec.cancelled() { return Err(ERR_CANCEL); }
        let (lw, lh) = record.rect.size()?;
        if lw as u64 * lh as u64 > dec.max_pixels() { return Err(ERR_LIMIT); }
        let mut planes: Vec<(i16, Vec<u8>)> = Vec::new();
        let mut plane_bytes=0u64;
        for &(id, n) in &record.channels {
            let bytes = r.take(n)?;
            let (cw, ch) = if id == -2 { record.mask.ok_or(ERR_HDR)?.0.size()? } else { (lw, lh) };
            if id == -3 { return Err(ERR_UNSUPPORTED); }
            let meaningful = id == -1 || id == -2 || (0..if gray { 1 } else { 3 }).contains(&id);
            if !meaningful { converted = true; continue; }
            plane_bytes=plane_bytes.checked_add(cw as u64 * ch as u64 * (meta.bits as u64 / 8)).ok_or(ERR_OVERFLOW)?;
            let error=meta::mem_check(surface_bytes.checked_add(plane_bytes).ok_or(ERR_OVERFLOW)?,dec);
            if error!=0 { return Err(error); }
            planes.push((id, channel(bytes, cw, ch, meta.bits, large, dec)?));
        }
        let mut sf = meta::surf_alloc(w as u32, h as u32, FMT_ARGB, dec)?;
        if lw != 0 && lh != 0 && !(0..if gray {1} else {3}).all(|id| planes.iter().any(|p| p.0 == id)) { return Err(ERR_HDR); }
        for y in record.rect.top.max(0)..record.rect.bottom.min(h as i32) {
            if dec.cancelled() { return Err(ERR_CANCEL); }
            for x in record.rect.left.max(0)..record.rect.right.min(w as i32) {
                let index = (y as i64 - record.rect.top as i64) as usize * lw + (x as i64 - record.rect.left as i64) as usize;
                let p = y as usize * sf.stride as usize + x as usize * 4;
                for c in 0..3 {
                    let id = if gray { 0 } else { 2 - c as i16 };
                    sf.px[p+c] = sample(&planes.iter().find(|p| p.0 == id).unwrap().1, index, meta.bits, false);
                }
                let mut alpha = planes.iter().find(|p| p.0 == -1).map_or(255, |p| sample(&p.1, index, meta.bits, true));
                if let Some((rect, default, flags)) = record.mask {
                    if flags & 2 == 0 {
                        let (mw, _) = rect.size()?;
                        let mut value = if x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom {
                            let plane = planes.iter().find(|p| p.0 == -2).ok_or(ERR_HDR)?;
                            sample(&plane.1, (y as i64 - rect.top as i64) as usize * mw + (x as i64 - rect.left as i64) as usize, meta.bits, true)
                        } else { default };
                        if flags & 4 != 0 { value = 255 - value; }
                        alpha = ((alpha as u32 * value as u32 + 127) / 255) as u8;
                    }
                }
                sf.px[p+3] = alpha;
            }
        }
        output.push(Layer { pixels: sf, name: record.name, opacity: record.opacity, visible: record.visible, blend: record.blend });
    }
    output.reverse();
    Ok(Some(Document { meta, layers: output, converted }))
}

fn channel(data: &[u8], w: usize, h: usize, depth: u8, large: bool, dec: &ImgDec) -> Result<Vec<u8>, i32> {
    if w as u64 * h as u64 > dec.max_pixels() { return Err(ERR_LIMIT); }
    let row = w.checked_mul(depth as usize / 8).ok_or(ERR_OVERFLOW)?;
    let size = row.checked_mul(h).ok_or(ERR_OVERFLOW)?;
    let error = meta::mem_check(size as u64, dec); if error != 0 { return Err(error); }
    let mut r = Reader { data, pos: 0 };
    let compression = r.u16()?;
    if size == 0 { return Ok(Vec::new()); }
    match compression {
        0 => Ok(r.take(size)?.to_vec()),
        1 => {
            let mut lengths = Vec::with_capacity(h);
            for _ in 0..h { lengths.push(if large {r.u32()? as usize} else {r.u16()? as usize}); }
            let mut output = vec![0;size];
            for (y, n) in lengths.into_iter().enumerate() {
                if dec.cancelled() { return Err(ERR_CANCEL); }
                packbits(r.take(n)?, &mut output[y*row..(y+1)*row])?;
            }
            Ok(output)
        },
        2 | 3 => {
            let mut output = vec![0;size];
            if inflate::zlib_inflate(&data[r.pos..], &mut output)? != size { return Err(ERR_TRUNC); }
            if compression == 3 { undo_prediction(&mut output, w, depth)?; }
            Ok(output)
        },
        _ => Err(ERR_UNSUPPORTED),
    }
}

/// Write the app's complete 8-bit raster stack and a compatibility composite.
/// Return an error for blend modes that Photoshop cannot represent exactly.
pub fn encode(composite: &Surface, layers: &[LayerRef], opt: &ImgOpt) -> Result<Vec<u8>, i32> {
    if layers.is_empty() || layers.len() > 32767 { return Err(ERR_LIMIT); }
    let mut flat = Vec::new();
    let error = super::encode(composite, opt, &mut flat);
    if error != 0 { return Err(error); }
    let (w,h) = (composite.w as usize, composite.h as usize);
    let size = w.checked_mul(h).ok_or(ERR_OVERFLOW)?;
    if size > (u32::MAX as usize - 2) { return Err(ERR_LIMIT); }
    let mut info = Vec::new();
    info.extend_from_slice(&(-(layers.len() as i16)).to_be_bytes());
    for layer in layers.iter().rev() {
        if layer.pixels.w != composite.w || layer.pixels.h != composite.h { return Err(ERR_DIMS); }
        let key = MODES.iter().find(|(m,_)| *m == layer.blend).ok_or(ERR_UNSUPPORTED)?.1;
        for v in [0u32,0,h as u32,w as u32] { info.extend_from_slice(&v.to_be_bytes()); }
        info.extend_from_slice(&4u16.to_be_bytes());
        for id in [-1i16,0,1,2] { info.extend_from_slice(&id.to_be_bytes()); info.extend_from_slice(&((size+2) as u32).to_be_bytes()); }
        info.extend_from_slice(b"8BIM"); info.extend_from_slice(key);
        info.extend_from_slice(&[layer.opacity,0,if layer.visible {8} else {10},0]);
        let mut extra = vec![0;8]; // no mask or special blending ranges
        let name: Vec<u8> = layer.name.chars().take(255).map(|c| if c.is_ascii() {c as u8} else {b'?'}).collect();
        extra.push(name.len() as u8); extra.extend_from_slice(&name);
        while extra.len() % 4 != 0 { extra.push(0); }
        let unicode: Vec<u16> = layer.name.encode_utf16().collect();
        if unicode.len() > 65535 { return Err(ERR_LIMIT); }
        extra.extend_from_slice(b"8BIMluni"); extra.extend_from_slice(&((4+unicode.len()*2) as u32).to_be_bytes());
        extra.extend_from_slice(&(unicode.len() as u32).to_be_bytes());
        for ch in unicode { extra.extend_from_slice(&ch.to_be_bytes()); }
        info.extend_from_slice(&(extra.len() as u32).to_be_bytes()); info.extend_from_slice(&extra);
    }
    let mut row = vec![0; w*4];
    for layer in layers.iter().rev() {
        for c in [3,2,1,0] {
            info.extend_from_slice(&[0,0]);
            for y in 0..h {
                if opt.cancelled() { return Err(ERR_CANCEL); }
                meta::get_row(layer.pixels, y, &mut row, FMT_ARGB);
                info.extend(row.chunks_exact(4).map(|p| p[c]));
            }
        }
    }
    if info.len() & 1 != 0 { info.push(0); }
    if info.len() > u32::MAX as usize - 8 { return Err(ERR_LIMIT); }
    let mut reader = Reader { data: &flat, pos: 26 };
    reader.section()?; reader.section()?;
    let split = reader.pos;
    let mut output = Vec::with_capacity(flat.len()+info.len()+8);
    output.extend_from_slice(&flat[..split]);
    output.extend_from_slice(&((info.len()+8) as u32).to_be_bytes());
    output.extend_from_slice(&(info.len() as u32).to_be_bytes()); output.extend_from_slice(&info);
    output.extend_from_slice(&[0;4]); // global layer mask
    output.extend_from_slice(&flat[split+4..]);
    Ok(output)
}
