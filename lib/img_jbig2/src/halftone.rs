// halftone -- pattern dictionaries (ITU-T T.88 6.7, 7.4.4) and halftone regions (6.6, 7.4.5)
//
// A pattern dictionary is one collective bitmap (generic region, arithmetic or MMR) cut into
// GRAYMAX + 1 patterns of HDPW x HDPH.  A halftone region decodes a grey-scale image of HGW x HGH
// cells as HBPP bit planes (most significant first, Gray-coded: each plane is XORed with the one
// above it), optionally skipping cells whose pattern would lie outside the region, and draws each
// cell's pattern on the grid (HGX, HGY) + m (HRY, HRX) + n (HRX, -HRY), in 1/256 pixels.

use crate::bitmap::Bmp;
use crate::generic::{self, Gb};
use crate::mmr;
use crate::mq::{Cx, Mq};
use crate::Error;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;

fn be32(d: &[u8], i: usize) -> Result<u32, Error> {
    d.get(i..i + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).ok_or(Error::Truncated)
}

/// a pattern dictionary segment -> its patterns
pub(crate) fn patterns(d: &[u8], work: &mut u64, limit: u64, max: u64) -> Result<Vec<Rc<Bmp>>, Error> {
    if d.len() < 7 {
        return Err(Error::Truncated);
    }
    let flags = d[0];
    let mmr_on = flags & 1 != 0;
    let template = (flags >> 1) & 3;
    let (pw, ph) = (d[1] as usize, d[2] as usize);
    let graymax = be32(d, 3)? as usize;
    if pw == 0 || ph == 0 || graymax >= 1 << 16 {
        return Err(Error::Corrupt);
    }
    let data = &d[7..];
    let w = (graymax + 1) * pw;
    *work = work.saturating_add((w * ph) as u64);
    if *work > limit {
        return Err(Error::Limit);
    }
    let collective = if mmr_on {
        mmr::decode(data, w, ph, max)?.0
    } else {
        let at = [(-(pw as i32).min(127) as i8, 0), (-3, -1), (2, -2), (-2, -2)];
        let mut mq = Mq::new(data);
        let mut cx = vec![Cx::default(); 1 << 16];
        generic::generic(&mut mq, &mut cx, &Gb { w, h: ph, template, tpgdon: false, at, skip: None }, max)?
    };
    (0..=graymax).map(|g| collective.sub((g * pw) as i64, 0, pw, ph).map(Rc::new)).collect()
}

/// a halftone region (its data after the region segment information) of w x h with the patterns
pub(crate) fn region(d: &[u8], w: usize, h: usize, pats: &[Rc<Bmp>], work: &mut u64, limit: u64, max: u64) -> Result<Bmp, Error> {
    if d.len() < 21 {
        return Err(Error::Truncated);
    }
    let flags = d[0];
    let mmr_on = flags & 1 != 0;
    let template = (flags >> 1) & 3;
    let enable_skip = flags & 8 != 0;
    let combop = (flags >> 4) & 7;
    let defpixel = flags >> 7;
    let gw = be32(d, 1)? as usize;
    let gh = be32(d, 5)? as usize;
    let gx = be32(d, 9)? as i32 as i64;
    let gy = be32(d, 13)? as i32 as i64;
    let rx = u16::from_be_bytes([d[17], d[18]]) as i64;
    let ry = u16::from_be_bytes([d[19], d[20]]) as i64;
    let data = &d[21..];
    let mut out = Bmp::new(w, h, defpixel, max)?;
    if pats.is_empty() {
        return Err(Error::Corrupt);
    }
    if (gw as u64).saturating_mul(gh as u64) > max {
        return Err(Error::Limit);
    }
    let (pw, ph) = (pats[0].w as i64, pats[0].h as i64);
    let bpp = if pats.len() <= 1 { 0 } else { 64 - ((pats.len() - 1) as u64).leading_zeros() };
    // the planes and the patterns drawn
    let cells = (gw as u64).saturating_mul(gh as u64);
    *work = work.saturating_add(cells.saturating_mul(bpp as u64 + (pw * ph) as u64));
    if *work > limit {
        return Err(Error::Limit);
    }
    // no cells: nothing drawn (and no rows of empty planes to go through)
    if cells == 0 {
        return Ok(out);
    }
    // the cells whose pattern lies outside the region
    let cell = |m: usize, n: usize| -> (i64, i64) { ((gx + m as i64 * ry + n as i64 * rx) >> 8, (gy + m as i64 * rx - n as i64 * ry) >> 8) };
    let skip = if enable_skip {
        let mut s = Bmp::new(gw, gh, 0, max)?;
        for m in 0..gh {
            for n in 0..gw {
                let (x, y) = cell(m, n);
                if x + pw <= 0 || x >= w as i64 || y + ph <= 0 || y >= h as i64 {
                    s.d[m * gw + n] = 1;
                }
            }
        }
        Some(s)
    } else {
        None
    };
    // the grey-scale image (C.5): the planes from the most significant one
    let mut planes: Vec<Bmp> = Vec::with_capacity(bpp as usize);
    let at = match template {
        0 => [(3, -1), (-3, -1), (2, -2), (-2, -2)],
        1 => [(3, -1), (0, 0), (0, 0), (0, 0)],
        _ => [(2, -1), (0, 0), (0, 0), (0, 0)],
    };
    let mut mq = Mq::new(data);
    let mut cx = vec![Cx::default(); 1 << 16];
    let mut pos = 0usize;
    for _ in 0..bpp {
        let plane = if mmr_on {
            let (b, used) = mmr::decode(data.get(pos..).unwrap_or(&[]), gw, gh, max)?;
            pos += used;
            b
        } else {
            generic::generic(&mut mq, &mut cx, &Gb { w: gw, h: gh, template, tpgdon: false, at, skip: skip.as_ref() }, max)?
        };
        planes.push(plane);
    }
    // Gray code: each plane XOR the one above it (planes[0] is the most significant)
    for j in 1..planes.len() {
        let (above, rest) = planes.split_at_mut(j);
        for (a, b) in rest[0].d.iter_mut().zip(&above[j - 1].d) {
            *a ^= b;
        }
    }
    for m in 0..gh {
        for n in 0..gw {
            let mut g = 0usize;
            for p in &planes {
                g = (g << 1) | p.d[m * gw + n] as usize;
            }
            let (x, y) = cell(m, n);
            out.compose(&pats[g.min(pats.len() - 1)], x, y, combop);
        }
    }
    Ok(out)
}
