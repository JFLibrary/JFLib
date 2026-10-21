// colr -- colour glyphs: COLR v0 / CPAL layers and CBDT / CBLC PNG bitmaps  [src/text/colr.s]
//
//   open(ctx)                         validate COLR + CPAL (palette 0) and CBDT + CBLC; sets
//                                     TFF_COLR / TFF_CBDT
//   layers(ctx, glyph, cap) -> {glyph | LAYER_FG, argb (straight)}   (empty = none)
//   bitmap(ctx, glyph, size_fx, png, scale) -> Bitmap   the strike's PNG decoded and scaled
//        to size_fx (area average when shrinking, bilinear when enlarging)
//
// COLR v1 paint graphs are not supported: v1 fonts that keep v0 base glyph records use
// those, otherwise the monochrome glyph is drawn.  Palette 0 only; palette index 0xFFFF =
// the current text colour.  CBDT: index subtable formats 1..5, glyph image formats 17, 18,
// 19 (PNG); strike = the smallest ppem >= the requested size, else the largest.  The PNG
// decoder and the enlarging scaler come from the image crates through hooks (Text).

use alloc::vec;
use alloc::vec::Vec;

use gfx::Surface;
use rt::fx::{cvt, cvtt, maxss, minss};

use crate::ttf::{Tb, TtFont, TFF_CBDT, TFF_COLR, TT_CBDT, TT_CBLC, TT_COLR, TT_CPAL};

pub const LAYER_FG: u32 = 0x8000_0000;
const PNG_MAX: u64 = 16 * 1024 * 1024;
const BMP_MAXSIDE: i32 = 2048;

/// decode a PNG into a PARGB surface
pub type PngDecode = fn(&[u8]) -> Option<Surface>;
/// bilinear resize of a PARGB surface to w x h (img_scale mode 1)
pub type ImgScale = fn(&Surface, i32, i32) -> Option<Surface>;

/// a colour glyph bitmap: placement relative to pen x / baseline, PARGB pixels (stride 4 w)
pub struct Bitmap {
    pub left: i32,
    pub top: i32,
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>,
}

/// validate COLR + CPAL and CBDT + CBLC  [colr_open]
pub fn open(f: &mut TtFont) {
    // ---- COLR + CPAL
    'colr: {
        let t = f.tb(TT_COLR);
        if t.len() < 14 || t.u16(0) as u32 > 1 {
            break 'colr;
        }
        let nb = t.u16(2);
        if nb <= 0 {
            break 'colr; // no v0 base glyph records
        }
        let bo = t.u32(4);
        if bo < 0 || (nb as u32).wrapping_mul(6) as u64 + bo as u64 > t.len() as u64 {
            break 'colr;
        }
        let nl = t.u16(12);
        if nl <= 0 {
            break 'colr;
        }
        let lo = t.u32(8);
        if lo < 0 || lo as u64 + nl as u64 * 4 > t.len() as u64 {
            break 'colr;
        }
        // CPAL: palette 0
        let p = f.tb(TT_CPAL);
        if p.len() < 14 {
            break 'colr;
        }
        let ne = p.u16(2);
        if ne <= 0 || p.u16(4) <= 0 {
            break 'colr;
        }
        let cro = p.u32(8);
        if cro < 0 {
            break 'colr;
        }
        let ci = p.u16(12);
        if ci < 0 {
            break 'colr;
        }
        let first = cro as u64 + ci as u64 * 4; // palette 0's first record
        if p.len() as u64 <= first {
            break 'colr;
        }
        let avail = (p.len() as u64 - first) >> 2;
        let n = (ne as u64).min(avail) as u32;
        if n == 0 {
            break 'colr;
        }
        f.cpal = (first as u32).wrapping_add(f.tab[TT_CPAL].0);
        f.cpaln = n;
        f.flags |= TFF_COLR;
    }
    // ---- CBDT + CBLC: at least one bitmap size record
    if f.tab[TT_CBDT].1 < 8 {
        return;
    }
    let t = f.tb(TT_CBLC);
    if t.len() < 8 + 48 {
        return;
    }
    let v = t.u16(0);
    if !(2..=3).contains(&v) || t.u32(4) <= 0 {
        return;
    }
    f.flags |= TFF_CBDT;
}

/// layers of a COLR base glyph (empty = none), at most cap  [colr_layers]
pub fn layers(f: &TtFont, glyph: u32, cap: usize, out: &mut Vec<(u32, u32)>) {
    out.clear();
    if f.flags & TFF_COLR == 0 {
        return;
    }
    let t = f.tb(TT_COLR);
    let d = t.0;
    let nb = t.u16(2) as u32; // base glyph records (checked by open)
    let bo = t.u32(4) as u32;
    let (mut lo, mut hi) = (0u32, nb);
    let mut found = None;
    while lo < hi {
        let mid = (lo + hi) >> 1;
        let r = (mid * 6 + bo) as usize;
        let g = u16::from_be_bytes([d[r], d[r + 1]]) as u32;
        if glyph < g {
            hi = mid;
        } else if glyph > g {
            lo = mid + 1;
        } else {
            let first = u16::from_be_bytes([d[r + 2], d[r + 3]]) as u32;
            let n = u16::from_be_bytes([d[r + 4], d[r + 5]]) as u32;
            found = Some((first, n));
            break;
        }
    }
    let Some((first, n)) = found else { return };
    let n = n.min(cap as u32);
    let nl = t.u16(12) as u32; // numLayerRecords (checked by open)
    let lo = t.u32(8) as u32;
    for k in 0..n {
        let li = first + k;
        if li >= nl {
            break;
        }
        let r = (lo + li * 4) as usize;
        let lg = u16::from_be_bytes([d[r], d[r + 1]]) as u32;
        let pi = u16::from_be_bytes([d[r + 2], d[r + 3]]) as u32;
        if pi >= f.cpaln {
            out.push((lg | LAYER_FG, 0)); // 0xFFFF (and anything invalid): text colour
        } else {
            // bytes B G R A = 0xAARRGGBB
            let o = (f.cpal + pi * 4) as usize;
            let c = u32::from_le_bytes([f.data[o], f.data[o + 1], f.data[o + 2], f.data[o + 3]]);
            out.push((lg, c));
        }
    }
}

// the image of glyph in the best strike  [cbdt_find]
struct Found {
    off: u32, // image data offset in CBDT
    len: u32,
    fmt: u32,
    ppem: u32,
    metrics: [u8; 5], // height, width, bearingX, bearingY, advance (from the index, formats 2 / 5)
}

// BigGlyphMetrics at a CBLC offset (bytes past the end read as 0)  [cb_bigmetrics]
fn bigmetrics(t: Tb, off: u32, m: &mut [u8; 5]) {
    for (i, v) in m.iter_mut().enumerate() {
        let b = t.u8(off.wrapping_add(i as u32));
        *v = if b < 0 { 0 } else { b as u8 };
    }
}

fn find(f: &TtFont, glyph: u32, want: u32, metrics: [u8; 5]) -> Option<Found> {
    let t = f.tb(TT_CBLC);
    let ns = t.u32(4);
    if ns <= 0 {
        return None;
    }
    let ns = (ns as u32).min(256);
    // strike: smallest ppemY >= wanted containing the glyph, else the largest
    let (mut best, mut bppem) = (u32::MAX, 0u32);
    for i in 0..ns {
        let r = i * 48 + 8; // BitmapSize record
        let s = t.u16(r + 40);
        if s < 0 || glyph < s as u32 {
            continue;
        }
        if glyph > t.u16(r + 42) as u32 {
            continue;
        }
        let pp = t.u8(r + 45);
        if pp <= 0 {
            continue;
        }
        let pp = pp as u32;
        let take = if best == u32::MAX {
            true
        } else if bppem >= want {
            // prefer: >= wanted and smaller
            pp >= want && pp < bppem
        } else {
            // both below: larger
            pp >= want || pp > bppem
        };
        if take {
            best = r;
            bppem = pp;
        }
    }
    if best == u32::MAX {
        return None;
    }
    let r = best;
    let mut m = metrics;
    let sa = t.u32(r);
    if sa < 0 {
        return None;
    }
    let sa = sa as u32;
    let nsub = t.u32(r + 8);
    if nsub <= 0 {
        return None;
    }
    let nsub = (nsub as u32).min(4096);
    // index subtable containing the glyph
    let mut sub = None;
    for i in 0..nsub {
        let e = (i << 3).wrapping_add(sa);
        let first = t.u16(e);
        if first < 0 {
            return None;
        }
        if glyph < first as u32 {
            continue;
        }
        if glyph <= t.u16(e.wrapping_add(2)) as u32 {
            sub = Some((e, first as u32));
            break;
        }
    }
    let (e, first) = sub?;
    let ao = t.u32(e.wrapping_add(4));
    if ao < 0 {
        return None;
    }
    let st = (ao as u32).wrapping_add(sa); // index subtable
    let ifmt = t.u16(st);
    let fmt = t.u16(st.wrapping_add(2)) as u32;
    let ido = t.u32(st.wrapping_add(4));
    if ido < 0 {
        return None;
    }
    let mut ido = ido as u32; // image data offset (CBDT)
    let k = glyph - first;
    // offsets of the glyph's data in CBDT: (start, end) relative to ido
    let span = |a: i64, b: i64| -> Option<(u32, u32)> {
        if b <= a {
            return None; // no data for this glyph
        }
        Some(((a as u32).wrapping_add(ido), (b - a) as u32))
    };
    let (off, len) = match ifmt {
        1 => {
            let a = t.u32(st.wrapping_add(k.wrapping_mul(4)).wrapping_add(8));
            if a < 0 {
                return None;
            }
            let b = t.u32(st.wrapping_add(k.wrapping_mul(4)).wrapping_add(12));
            if b < 0 {
                return None;
            }
            span(a, b)?
        }
        3 => {
            let a = t.u16(st.wrapping_add(k.wrapping_mul(2)).wrapping_add(8));
            if a < 0 {
                return None;
            }
            let b = t.u16(st.wrapping_add(k.wrapping_mul(2)).wrapping_add(10));
            if b < 0 {
                return None;
            }
            span(a as i64, b as i64)?
        }
        2 => {
            let size = t.u32(st.wrapping_add(8)); // imageSize
            if size <= 0 {
                return None;
            }
            let size = size as u32;
            ido = size.wrapping_mul(k).wrapping_add(ido);
            bigmetrics(t, st.wrapping_add(12), &mut m);
            (ido, size)
        }
        5 => {
            let n = t.u32(st.wrapping_add(20)); // numGlyphs
            if n <= 0 {
                return None;
            }
            let mut idx = None;
            for i in 0..n as u32 {
                let g = t.u16(st.wrapping_add(i * 2).wrapping_add(24));
                if g < 0 {
                    return None;
                }
                if g as u32 == glyph {
                    idx = Some(i);
                    break;
                }
            }
            let i = idx?;
            let size = t.u32(st.wrapping_add(8));
            if size <= 0 {
                return None;
            }
            let size = size as u32;
            ido = size.wrapping_mul(i).wrapping_add(ido);
            bigmetrics(t, st.wrapping_add(12), &mut m);
            (ido, size)
        }
        4 => {
            let n = t.u32(st.wrapping_add(8)); // numGlyphs
            if n <= 0 {
                return None;
            }
            let mut idx = None;
            for i in 0..n as u32 {
                let g = t.u16(st.wrapping_add(i * 4).wrapping_add(12));
                if g < 0 {
                    return None;
                }
                if g as u32 == glyph {
                    idx = Some(i);
                    break;
                }
            }
            let i = idx?;
            let a = t.u16(st.wrapping_add(i * 4).wrapping_add(14));
            let b = t.u16(st.wrapping_add(i * 4).wrapping_add(18));
            if b < 0 {
                return None;
            }
            span(a as u32 as i64, b as i64)?
        }
        _ => return None,
    };
    if off == 0 {
        return None;
    }
    Some(Found { off, len, fmt, ppem: bppem, metrics: m })
}

/// the colour bitmap of glyph at size_fx (24.8 px)  [cbdt_bitmap]
pub fn bitmap(f: &TtFont, glyph: u32, size_fx: i32, png: Option<PngDecode>, scale: Option<ImgScale>,
              metrics: &mut [u8; 5]) -> Option<Bitmap> {
    if f.flags & TFF_CBDT == 0 {
        return None;
    }
    let want = ((size_fx as u32).wrapping_add(255)) >> 8; // wanted ppem (rounded up)
    let fd = find(f, glyph, want, *metrics)?;
    *metrics = fd.metrics;
    let t = f.tb(TT_CBDT);
    let s = fd.off;
    let (plen, poff): (i64, u32) = match fd.fmt {
        19 => {
            // metrics from the index, u32 length, PNG
            let l = t.u32(s);
            if l < 0 {
                return None;
            }
            (l, s.wrapping_add(4))
        }
        17 => {
            for i in 0..5u32 {
                let b = t.u8(s.wrapping_add(i));
                if b < 0 {
                    return None;
                }
                metrics[i as usize] = b as u8;
            }
            let l = t.u32(s.wrapping_add(5));
            if l < 0 {
                return None;
            }
            (l, s.wrapping_add(9))
        }
        18 => {
            bigmetrics(t, s, metrics);
            let l = t.u32(s.wrapping_add(8));
            if l < 0 {
                return None;
            }
            (l, s.wrapping_add(12))
        }
        _ => return None,
    };
    let _ = fd.len;
    if poff as u64 + plen as u64 > t.len() as u64 || plen as u64 > PNG_MAX {
        return None;
    }
    let src = (png?)(&t.0[poff as usize..poff as usize + plen as usize])?;
    // target size: bitmap * size / ppem
    let sc = size_fx as f32 * (1.0 / 256.0) / fd.ppem as i32 as f32;
    let w = cvt(src.w as f32 * sc).max(1);
    if w > BMP_MAXSIDE {
        return None;
    }
    let h = cvt(src.h as f32 * sc).max(1);
    if h > BMP_MAXSIDE {
        return None;
    }
    // pixels: shrink by area averaging, enlarge bilinearly
    let px = if w > src.w || h > src.h {
        let up = (scale?)(&src, w, h)?;
        area_scale(&up, w, h) // same size: a packed copy
    } else {
        area_scale(&src, w, h)
    };
    // placement: bearingX, -bearingY (scaled)
    let left = cvt(metrics[2] as i8 as i32 as f32 * sc);
    let top = cvt(metrics[3] as i8 as i32 as f32 * sc).wrapping_neg();
    Some(Bitmap { left, top, w, h, px })
}

// packed PARGB w x h: the area average of the source (w <= src.w, h <= src.h)  [area_scale]
fn area_scale(src: &Surface, w: i32, h: i32) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    // source pixels per target pixel
    let fx = src.w as f32 / w as f32;
    let fy = src.h as f32 / h as f32;
    let stride = src.stride as usize;
    let mut o = 0usize;
    for y in 0..h {
        for x in 0..w {
            // source rectangle [x0, x1) x [y0, y1)
            let x0 = x as f32 * fx;
            let x1 = x0 + fx;
            let y0 = y as f32 * fy;
            let y1 = y0 + fy;
            let mut acc = [0f32; 4];
            let mut wsum = 0f32;
            let mut sy = cvtt(y0);
            loop {
                let syf = sy as f32;
                if !(syf < y1) || sy as u32 >= src.h as u32 {
                    break;
                }
                // vertical weight = min(sy+1, y1) - max(sy, y0)
                let wy = minss(syf + 1.0, y1) - maxss(syf, y0);
                let row = sy as usize * stride;
                let mut sx = cvtt(x0);
                loop {
                    let sxf = sx as f32;
                    if !(sxf < x1) || sx as u32 >= src.w as u32 {
                        break;
                    }
                    let wgt = (minss(sxf + 1.0, x1) - maxss(sxf, x0)) * wy;
                    wsum = wsum + wgt;
                    let p = row + sx as usize * 4;
                    for c in 0..4 {
                        acc[c] = acc[c] + src.px[p + c] as f32 * wgt;
                    }
                    sx += 1;
                }
                sy += 1;
            }
            if wsum > 0.0 {
                for c in 0..4 {
                    // cvtps2dq (nearest), packssdw, packuswb
                    let v = cvt(acc[c] / wsum).clamp(-32768, 32767);
                    out[o + c] = v.clamp(0, 255) as u8;
                }
            }
            o += 4;
        }
    }
    out
}
