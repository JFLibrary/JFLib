// generic -- generic region decoding (ITU-T T.88 6.2.5, arithmetic) and generic refinement region
// decoding (6.3)
//
// The context of a pixel is built from its neighbours in the order of the standard's figures (bit
// 0 the pixel to the left; the adaptive pixels at their fixed bits), because the typical-prediction
// bits (TPGDON, TPGRON) are decoded in fixed contexts of that numbering.  TPGDON: a row whose bit
// says so repeats the row above.  TPGRON: in such a row a pixel whose 3 x 3 neighbourhood in the
// reference is one colour takes that colour.  A skip bitmap (halftone regions) leaves pixels 0.

use crate::bitmap::Bmp;
use crate::mq::{Cx, Mq};
use crate::Error;
use alloc::vec::Vec;

/// the parameters of a generic region
pub(crate) struct Gb<'a> {
    pub w: usize,
    pub h: usize,
    pub template: u8,
    pub tpgdon: bool,
    /// the adaptive pixels (template 0: four, else one); a header gives -128 to 127, a pattern
    /// dictionary -HDPW (down to -255)
    pub at: [(i32, i32); 4],
    pub skip: Option<&'a Bmp>,
}

/// the context of TPGDON's bit per template
const SLTP: [usize; 4] = [0x9B25, 0x0795, 0x00E5, 0x0195];

/// the fixed neighbours of a template as windows: per row above (two rows up, one row up) the
/// leftmost and rightmost offset and the context bit of its rightmost pixel; then the width of the
/// window of the row itself (its pixels left of x, from bit 0) and the bits of the adaptive pixels
struct Layout {
    up2: Option<(i64, i64, u32)>,
    up1: (i64, i64, u32),
    own: u32,
    at_bits: &'static [u32],
}

const LAYOUTS: [Layout; 4] = [
    Layout { up2: Some((-1, 1, 12)), up1: (-2, 2, 5), own: 4, at_bits: &[4, 10, 11, 15] },
    Layout { up2: Some((-1, 2, 9)), up1: (-2, 2, 4), own: 3, at_bits: &[3] },
    Layout { up2: Some((-1, 1, 7)), up1: (-2, 1, 3), own: 2, at_bits: &[2] },
    Layout { up2: None, up1: (-3, 1, 5), own: 4, at_bits: &[4] },
];

/// a window of the pixels lo..=hi around x = 0 of a row (the rightmost the least significant bit)
#[inline(always)]
fn window(row: Option<&[u8]>, lo: i64, hi: i64) -> u32 {
    let Some(r) = row else { return 0 };
    let mut v = 0u32;
    for x in lo..=hi {
        v = (v << 1) | if x >= 0 && (x as usize) < r.len() { r[x as usize] as u32 } else { 0 };
    }
    v
}

#[inline(always)]
fn px(row: Option<&[u8]>, x: usize) -> u32 {
    match row {
        Some(r) if x < r.len() => r[x] as u32,
        _ => 0,
    }
}

pub(crate) fn generic(mq: &mut Mq, cx: &mut [Cx], p: &Gb, max: u64) -> Result<Bmp, Error> {
    let mut b = Bmp::new(p.w, p.h, 0, max)?;
    let (w, h) = (p.w, p.h);
    // no pixels: no rows to go through either (a height of 2^32 rows of width 0 is no work)
    if w == 0 || h == 0 {
        return Ok(b);
    }
    let t = (p.template & 3) as usize;
    let lay = &LAYOUTS[t];
    let at: Vec<(i64, i64, u32)> = lay.at_bits.iter().zip(p.at.iter()).map(|(&bit, &(x, y))| (x as i64, y as i64, bit)).collect();
    let mut ltp = 0u32;
    for y in 0..h {
        if p.tpgdon {
            ltp ^= mq.decode(&mut cx[SLTP[t]]);
            if ltp == 1 {
                if y > 0 {
                    b.d.copy_within((y - 1) * w..y * w, y * w);
                }
                continue;
            }
        }
        let (above, rest) = b.d.split_at_mut(y * w);
        let row = &mut rest[..w];
        let r1 = if y >= 1 { Some(&above[(y - 1) * w..y * w]) } else { None };
        let r2 = if y >= 2 { Some(&above[(y - 2) * w..(y - 1) * w]) } else { None };
        // the windows at x = 0
        let (l1, h1, s1) = lay.up1;
        let m1 = (1u32 << (h1 - l1 + 1)) - 1;
        let mut w1 = window(r1, l1, h1);
        let (l2, h2, s2) = lay.up2.unwrap_or((0, 0, 0));
        let m2 = (1u32 << (h2 - l2 + 1)) - 1;
        let mut w2 = if lay.up2.is_some() { window(r2, l2, h2) } else { 0 };
        let mo = (1u32 << lay.own) - 1;
        let mut wo = 0u32;
        let yi = y as i64;
        for x in 0..w {
            let skipped = p.skip.is_some_and(|s| s.get(x as i64, yi) == 1);
            let v = if skipped {
                0
            } else {
                let mut c = wo | (w1 << s1) | (w2 << s2);
                for &(ax, ay, bit) in &at {
                    let (xx, yy) = (x as i64 + ax, yi + ay);
                    let pv = if yy == yi {
                        if xx >= 0 && xx < x as i64 { row[xx as usize] } else { 0 }
                    } else if xx >= 0 && (xx as usize) < w && yy >= 0 && yy < yi {
                        above[yy as usize * w + xx as usize]
                    } else {
                        0
                    };
                    c |= (pv as u32) << bit;
                }
                let v = mq.decode(&mut cx[c as usize]) as u8;
                row[x] = v;
                v
            };
            // the windows one pixel to the right
            wo = ((wo << 1) | v as u32) & mo;
            w1 = ((w1 << 1) | px(r1, (x as i64 + h1 + 1) as usize)) & m1;
            if lay.up2.is_some() {
                w2 = ((w2 << 1) | px(r2, (x as i64 + h2 + 1) as usize)) & m2;
            }
        }
    }
    Ok(b)
}

/// the parameters of a refinement
pub(crate) struct Gr<'a> {
    pub w: usize,
    pub h: usize,
    pub template: u8,
    pub reference: &'a Bmp,
    pub dx: i32,
    pub dy: i32,
    pub tpgron: bool,
    pub at: [(i8, i8); 2],
}

/// the context of TPGRON's bit per template: every pixel 0 but the reference's pixel under the one
/// decoded
pub(crate) const SLTP_REF: [usize; 2] = [0x0100, 0x0080];

pub(crate) fn refine(mq: &mut Mq, cx: &mut [Cx], p: &Gr, max: u64) -> Result<Bmp, Error> {
    let mut b = Bmp::new(p.w, p.h, 0, max)?;
    let (w, h) = (p.w, p.h);
    if w == 0 || h == 0 {
        return Ok(b);
    }
    let t1 = p.template & 1 == 1;
    let r = p.reference;
    let a = p.at.map(|(x, y)| (x as i64, y as i64));
    let (dx, dy) = (p.dx as i64, p.dy as i64);
    let rrow = |yy: i64| -> Option<&[u8]> {
        if yy >= 0 && yy < r.h as i64 {
            Some(&r.d[yy as usize * r.w..(yy as usize + 1) * r.w])
        } else {
            None
        }
    };
    let pxi = |row: Option<&[u8]>, x: i64| -> u32 {
        match row {
            Some(rw) if x >= 0 && (x as usize) < rw.len() => rw[x as usize] as u32,
            _ => 0,
        }
    };
    let mut ltp = 0u32;
    for y in 0..h {
        if p.tpgron {
            ltp ^= mq.decode(&mut cx[SLTP_REF[t1 as usize]]);
        }
        let (above, rest) = b.d.split_at_mut(y * w);
        let row = &mut rest[..w];
        let c1 = if y >= 1 { Some(&above[(y - 1) * w..y * w]) } else { None };
        let yi = y as i64;
        let ry = yi - dy;
        let (q0, q1, q2) = (rrow(ry - 1), rrow(ry), rrow(ry + 1));
        // windows of three pixels (the rightmost the least significant bit): the row above around x,
        // the reference's three rows around rx = x - dx
        let rx0 = -dx;
        let mut wc = window(c1, -1, 1);
        let (mut w0, mut w1, mut w2) = (window(q0, rx0 - 1, rx0 + 1), window(q1, rx0 - 1, rx0 + 1), window(q2, rx0 - 1, rx0 + 1));
        let mut prev = 0u32;
        for x in 0..w {
            let xi = x as i64;
            let rx = xi - dx;
            let v = if ltp == 1 && ((w0 | w1 | w2) == 0 || (w0 & w1 & w2) == 7) {
                // a pixel whose 3 x 3 neighbourhood in the reference is one colour takes it
                w1 >> 1 & 1
            } else {
                let c = if !t1 {
                    let (ax, ay) = (xi + a[0].0, yi + a[0].1);
                    let at0 = if ay == yi {
                        if ax >= 0 && ax < xi { row[ax as usize] as u32 } else { 0 }
                    } else if ax >= 0 && (ax as usize) < w && ay >= 0 && ay < yi {
                        above[ay as usize * w + ax as usize] as u32
                    } else {
                        0
                    };
                    let at1 = r.get(rx + a[1].0, ry + a[1].1) as u32;
                    prev | (wc & 3) << 1 | at0 << 3 | w2 << 4 | w1 << 7 | (w0 & 3) << 10 | at1 << 12
                } else {
                    prev | wc << 1 | (w2 & 3) << 4 | w1 << 6 | (w0 >> 1 & 1) << 9
                };
                mq.decode(&mut cx[c as usize])
            };
            row[x] = v as u8;
            prev = v;
            wc = ((wc << 1) | pxi(c1, xi + 2)) & 7;
            w0 = ((w0 << 1) | pxi(q0, rx + 2)) & 7;
            w1 = ((w1 << 1) | pxi(q1, rx + 2)) & 7;
            w2 = ((w2 << 1) | pxi(q2, rx + 2)) & 7;
        }
    }
    Ok(b)
}
