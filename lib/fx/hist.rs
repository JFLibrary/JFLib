// hist -- histogram helpers of the effect kernels (thread safe, explicit buffers)  [src/fx/fx_hist.s]
//
//   hist_rgba(a, hist) -> count | ERR_CANCEL    u32[4][256] (B, G, R, A) of the ROI pixels with
//        alpha != 0 and (with a selection) coverage != 0; polls the cancel flag once per row
//   hist_percentile(h, total, fraction) -> j    smallest j with h[0] + .. + h[j] > total *
//        fraction (f64 compare); 0 when there is none (paint.net HistogramRgb.GetPercentile)
//   hist_mean(h) -> f64                         f32(sum j h[j]) / f32(sum h[j]) (paint.net GetMean)
//   lochist(a, dst, rad, f) -> 0 | ERR_CANCEL   paint.net LocalHistogramEffect.RenderRect: for
//        every ROI pixel the histograms (LocHist) of the source pixels (x+u, y+v) inside the
//        image with u^2 + v^2 <= ((2 rad + 1)^2 + 2) / 4; dst = f(hist, area, srcpixel).
//        Histograms start at the ROI's left edge of every row and slide to the right (with
//        paint.net's edge loops), so the result does not depend on the ROI split.

use alloc::vec;

use alloc::vec::Vec;

use crate::{px, put, Dst, FxArgs, ERR_CANCEL};

pub const FX_LOCHIST_MAX: i32 = 1024;

/// histograms B, G, R, A and their block sums (block k = bins 16k .. 16k + 15)
#[derive(Clone)]
pub struct LocHist {
    pub h: [[u32; 256]; 4],
    pub blk: [[u32; 16]; 4],
}

impl LocHist {
    #[inline(always)]
    fn add(&mut self, p: u32) {
        for c in 0..4 {
            let v = (p >> (8 * c) & 255) as usize;
            self.h[c][v] += 1;
            self.blk[c][v >> 4] += 1;
        }
    }
    #[inline(always)]
    fn sub(&mut self, p: u32) {
        for c in 0..4 {
            let v = (p >> (8 * c) & 255) as usize;
            self.h[c][v] -= 1;
            self.blk[c][v >> 4] -= 1;
        }
    }
}

/// histograms of the ROI (alpha > 0, selection > 0) -> pixels counted | ERR_CANCEL  [fx_hist_rgba]
pub fn hist_rgba(a: &FxArgs, hist: &mut [[u32; 256]; 4]) -> i64 {
    *hist = [[0; 256]; 4];
    let mut count = 0i64;
    let mut sbuf = [0u8; 256];
    for y in a.y0..a.y1 {
        if a.cancelled() {
            return ERR_CANCEL as i64;
        }
        let row = a.src_row(y);
        let mut x = a.x0;
        while a.x1 - x > 0 {
            let n = (a.x1 - x).min(256);
            let sel = match a.sel {
                Some(m) => Some(m.row(x, y, n, &mut sbuf).0),
                None => None,
            };
            for i in 0..n as usize {
                if let Some(s) = sel {
                    if s[i] == 0 {
                        continue;
                    }
                }
                let p = px(row, x as usize + i);
                if p < 0x0100_0000 {
                    continue; // alpha 0
                }
                hist[0][(p & 255) as usize] += 1;
                hist[1][(p >> 8 & 255) as usize] += 1;
                hist[2][(p >> 16 & 255) as usize] += 1;
                hist[3][(p >> 24) as usize] += 1;
                count += 1;
            }
            x += n;
        }
    }
    count
}

/// smallest j with h[0] + .. + h[j] > total * fraction, else 0  [fx_hist_percentile]
pub fn hist_percentile(h: &[u32; 256], total: i64, fraction: f64) -> u32 {
    let target = total as f64 * fraction;
    let mut integral = 0i64;
    for (j, &c) in h.iter().enumerate() {
        integral += c as i64;
        if integral as f64 > target {
            return j as u32;
        }
    }
    0
}

/// f32(sum j h[j]) / f32(sum h[j]) as f64, 0 when empty  [fx_hist_mean]
pub fn hist_mean(h: &[u32; 256]) -> f64 {
    let (mut s, mut n) = (0i64, 0i64);
    for (j, &c) in h.iter().enumerate() {
        n += c as i64;
        s += c as i64 * j as i64;
    }
    if n == 0 {
        return 0.0;
    }
    (s as f32 / n as f32) as f64
}

/// paint.net LocalHistogramEffect.RenderRect over the ROI -> 0 | ERR_CANCEL  [fx_lochist]
pub fn lochist(a: &FxArgs, dst: &mut Dst, rad: i32, mut f: impl FnMut(&LocHist, u32, u32) -> u32) -> i32 {
    let rad = rad.clamp(0, FX_LOCHIST_MAX);
    let cutoff = ((2 * rad + 1) * (2 * rad + 1) + 2) >> 2;
    let (w, h) = (a.src.w, a.src.h);
    let stride = a.src.stride as usize;
    let sp = &a.src.px;
    let at = |x: i32, y: i32| -> u32 { px(&sp[y as usize * stride..], x as usize) };
    // leadingEdgeX[v] = largest u <= rad with u^2 + v^2 <= cutoff
    let mut edge = vec![0i32; rad as usize + 1];
    for v in 0..=rad {
        let mut e = 0;
        for u in 0..=rad {
            if u * u + v * v <= cutoff {
                e = u;
            }
        }
        edge[v as usize] = e;
    }
    let mut hb = alloc::boxed::Box::new(LocHist { h: [[0; 256]; 4], blk: [[0; 16]; 4] });
    // interior columns (rad <= x <= w - 2 - rad): every window row loses its trailing pixel and
    // gains its leading one; (trailing, leading) byte offsets of the rows less x * 4.  The counts
    // do not depend on the order of the updates; an equal pair changes nothing.
    let mut offs: Vec<(usize, usize)> = Vec::with_capacity(2 * rad as usize + 1);
    let rd = |o: usize| -> u32 {
        let q: [u8; 4] = sp[o..o + 4].try_into().unwrap();
        u32::from_le_bytes(q)
    };
    for y in a.y0..a.y1 {
        if a.x1 <= a.x0 {
            break;
        }
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let hb = &mut *hb;
        hb.h = [[0; 256]; 4];
        hb.blk = [[0; 16]; 4];
        let mut area = 0u32;
        let top = -rad.min(y);
        let bottom = (h - 1 - y).min(rad);
        let x0 = a.x0;
        let mut left = -rad.min(x0);
        let mut right = (w - 1 - x0).min(rad);
        for v in top..=bottom {
            for u in left..=right {
                if u * u + v * v <= cutoff {
                    hb.add(at(x0 + u, y + v));
                    area += 1;
                }
            }
        }
        offs.clear();
        for v in top..=bottom {
            let e = edge[v.unsigned_abs() as usize] as usize;
            let ro = (y + v) as usize * stride;
            offs.push((ro.wrapping_sub(e * 4), ro + (e + 1) * 4));
        }
        let drow = dst.row(y);
        for x in a.x0..a.x1 {
            put(drow, (x - a.x0) as usize, f(hb, area, at(x, y)));
            if x >= rad && x <= w - 2 - rad {
                let b = x as usize * 4;
                for &(ot, ol) in &offs {
                    let (pt, pl) = (rd(ot.wrapping_add(b)), rd(ol + b));
                    if pt != pl {
                        hb.sub(pt);
                        hb.add(pl);
                    }
                }
                continue;
            }
            left = -rad.min(x);
            right = (w - 1 - x).min(rad + 1);
            // trailing edge, top half
            let mut v = -1;
            while v >= top {
                if -edge[-v as usize] >= left {
                    break;
                }
                v -= 1;
            }
            while v >= top {
                hb.sub(at(x - edge[-v as usize], y + v));
                area -= 1;
                v -= 1;
            }
            // leading edge, top half
            let mut v = -1;
            while v >= top {
                if edge[-v as usize] + 1 <= right {
                    break;
                }
                v -= 1;
            }
            while v >= top {
                hb.add(at(x + edge[-v as usize] + 1, y + v));
                area += 1;
                v -= 1;
            }
            // trailing edge, bottom half
            let mut v = 0;
            while v <= bottom {
                if -edge[v as usize] >= left {
                    break;
                }
                v += 1;
            }
            while v <= bottom {
                hb.sub(at(x - edge[v as usize], y + v));
                area -= 1;
                v += 1;
            }
            // leading edge, bottom half
            let mut v = 0;
            while v <= bottom {
                if edge[v as usize] + 1 <= right {
                    break;
                }
                v += 1;
            }
            while v <= bottom {
                hb.add(at(x + edge[v as usize] + 1, y + v));
                area += 1;
                v += 1;
            }
        }
    }
    0
}
