// paint -- the pixel side of the renderer: targets with an offset (layers), clip masks,
// shaders (solid colours, gradients through a 256-entry lookup table) and the span and
// layer compositing ("over" on premultiplied pixels, exact div255 rounding)

use alloc::vec;
use alloc::vec::Vec;

use gfx::blend;
use gfx::xform::Matrix;
use gfx::Surface;
use rt::fx::{div255, sqrtd};

use crate::values::floor;

/// a drawing target: a surface whose pixel (0, 0) lies at device (ox, oy); clip = the device
/// rectangle x0, y0, x1, y1 (exclusive) that may be touched (inside the surface)
pub(crate) struct Tgt<'s> {
    pub sf: &'s mut Surface,
    pub ox: i32,
    pub oy: i32,
    pub clip: [i32; 4],
}

/// an 8-bit clip mask over the device rectangle r (rows of r[2] - r[0] bytes)
pub(crate) struct Mask {
    pub px: Vec<u8>,
    pub r: [i32; 4],
}

impl Mask {
    pub fn new(r: [i32; 4]) -> Mask {
        let w = (r[2] - r[0]).max(0) as usize;
        let h = (r[3] - r[1]).max(0) as usize;
        Mask { px: vec![0; w * h], r }
    }

    fn w(&self) -> usize {
        (self.r[2] - self.r[0]).max(0) as usize
    }

    /// coverage *= mask for the span (x, y) .. (x + n, y)
    pub fn apply(&self, y: i32, x: i32, cov: &mut [u8]) {
        let r = self.r;
        if y < r[1] || y >= r[3] {
            cov.fill(0);
            return;
        }
        let w = self.w();
        let row = (y - r[1]) as usize * w;
        for (i, c) in cov.iter_mut().enumerate() {
            let px = x + i as i32;
            if px < r[0] || px >= r[2] {
                *c = 0;
            } else if *c != 0 {
                let m = self.px.get(row + (px - r[0]) as usize).copied().unwrap_or(0) as u32;
                *c = div255(*c as u32 * m) as u8;
            }
        }
    }

    /// union with coverage (a8 over) at (x, y)
    pub fn add(&mut self, y: i32, x: i32, cov: &[u8]) {
        let r = self.r;
        if y < r[1] || y >= r[3] {
            return;
        }
        let w = self.w();
        let row = (y - r[1]) as usize * w;
        for (i, &c) in cov.iter().enumerate() {
            let px = x + i as i32;
            if px < r[0] || px >= r[2] || c == 0 {
                continue;
            }
            if let Some(d) = self.px.get_mut(row + (px - r[0]) as usize) {
                let v = c as u32 + div255(*d as u32 * (255 - c as u32));
                *d = v.min(255) as u8;
            }
        }
    }

    /// self *= other (outside other: 0)
    pub fn intersect(&mut self, other: &Mask) {
        let w = self.w();
        let r = self.r;
        let mut row = vec![0u8; w];
        for y in r[1]..r[3] {
            let o = (y - r[1]) as usize * w;
            if let Some(src) = self.px.get(o..o + w) {
                row.copy_from_slice(src);
            }
            other.apply(y, r[0], &mut row);
            if let Some(dst) = self.px.get_mut(o..o + w) {
                dst.copy_from_slice(&row);
            }
        }
    }
}

/// a gradient shader: device pixel -> position t -> premultiplied colour
pub(crate) struct Grad {
    pub radial: bool,
    /// linear: t = lin[0] * xc + lin[1] * yc + lin[2] (xc, yc = device pixel centre)
    pub lin: [f64; 3],
    /// radial: device -> gradient space
    pub inv: Matrix,
    pub f: (f64, f64),
    pub fr: f64,
    pub cd: (f64, f64),
    pub dr: f64,
    pub a: f64,
    /// 0 pad, 1 reflect, 2 repeat
    pub spread: u8,
    pub lut: [u32; 256],
}

pub(crate) enum Shader {
    /// premultiplied B, G, R, A (gfx::blend lanes)
    Solid([u32; 4]),
    Grad(Grad),
}

/// the premultiplied lanes of a straight colour times an opacity (0..1)
pub(crate) fn solid(argb: u32, op: f64) -> Option<Shader> {
    let a = ((argb >> 24) as f64 * op.clamp(0.0, 1.0) + 0.5) as u32;
    if a == 0 {
        return None;
    }
    let mut p = [0u32; 4];
    for (ch, v) in p.iter_mut().take(3).enumerate() {
        *v = div255(((argb >> (ch * 8)) & 255) * a);
    }
    p[3] = a;
    Some(Shader::Solid(p))
}

/// the lookup table of sorted stops (offset 0..1, straight colour), opacity applied;
/// interpolated in premultiplied space
pub(crate) fn lut(stops: &[(f64, u32)], op: f64) -> [u32; 256] {
    let pm = |c: u32| -> [f64; 4] {
        let a = (c >> 24) as f64 * op / 255.0;
        [((c >> 16) & 255) as f64 * a, ((c >> 8) & 255) as f64 * a, (c & 255) as f64 * a, a * 255.0]
    };
    let mut out = [0u32; 256];
    let mut k = 0;
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f64 / 255.0;
        while k + 1 < stops.len() && stops[k + 1].0 <= t {
            k += 1;
        }
        let c = if t <= stops[0].0 {
            pm(stops[0].1)
        } else if k + 1 >= stops.len() {
            pm(stops[stops.len() - 1].1)
        } else {
            let (o0, c0) = stops[k];
            let (o1, c1) = stops[k + 1];
            let f = if o1 > o0 { (t - o0) / (o1 - o0) } else { 1.0 };
            let (a, b) = (pm(c0), pm(c1));
            [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f, a[3] + (b[3] - a[3]) * f]
        };
        let q = |v: f64| (v + 0.5).clamp(0.0, 255.0) as u32;
        let a = q(c[3]);
        *o = a << 24 | q(c[0]).min(a) << 16 | q(c[1]).min(a) << 8 | q(c[2]).min(a);
    }
    out
}

impl Grad {
    // the position at the device pixel centre (x + 0.5, y + 0.5); None: not painted
    #[inline]
    fn t_at(&self, x: f64, y: f64) -> Option<f64> {
        if !self.radial {
            return Some(self.lin[0] * x + self.lin[1] * y + self.lin[2]);
        }
        let m = &self.inv;
        let (qx, qy) = (m.a * x + m.b * y + m.c, m.d * x + m.e * y + m.f);
        let (px, py) = (qx - self.f.0, qy - self.f.1);
        let b = px * self.cd.0 + py * self.cd.1 + self.fr * self.dr;
        let c = px * px + py * py - self.fr * self.fr;
        if self.a.abs() < 1e-12 {
            if b.abs() < 1e-12 {
                return None;
            }
            let t = c / (2.0 * b);
            return if self.fr + t * self.dr >= 0.0 { Some(t) } else { None };
        }
        let disc = b * b - self.a * c;
        if disc < 0.0 {
            return None;
        }
        let sq = sqrtd(disc);
        let (t1, t2) = ((b + sq) / self.a, (b - sq) / self.a);
        let (hi, lo) = if t1 > t2 { (t1, t2) } else { (t2, t1) };
        if self.fr + hi * self.dr >= 0.0 {
            Some(hi)
        } else if self.fr + lo * self.dr >= 0.0 {
            Some(lo)
        } else {
            None
        }
    }

    #[inline]
    fn color(&self, t: f64) -> u32 {
        let t = match self.spread {
            1 => {
                let u = t - 2.0 * floor(t * 0.5);
                if u > 1.0 { 2.0 - u } else { u }
            }
            2 => t - floor(t),
            _ => t,
        };
        let i = if t > 0.0 { (t * 255.0 + 0.5).min(255.0) as usize } else { 0 };
        self.lut[i]
    }

    /// the colours of n pixels of row y from x, over the row through coverage
    fn span(&self, row: &mut [u8], x: i32, y: i32, cov: &[u8]) {
        let yc = y as f64 + 0.5;
        for (i, &k) in cov.iter().enumerate() {
            if k == 0 {
                continue;
            }
            let xc = (x + i as i32) as f64 + 0.5;
            let Some(t) = self.t_at(xc, yc) else { continue };
            over_px(row, i, self.color(t), k as u32);
        }
    }
}

// premultiplied colour c scaled by coverage k (0..255) over pixel i of row
#[inline]
fn over_px(row: &mut [u8], i: usize, c: u32, k: u32) {
    let Some(d) = row.get_mut(i * 4..i * 4 + 4) else { return };
    let s = if k >= 255 {
        c
    } else {
        let mut s = 0;
        for ch in 0..4 {
            s |= div255(((c >> (ch * 8)) & 255) * k) << (ch * 8);
        }
        s
    };
    let ia = 255 - (s >> 24);
    if ia == 255 {
        return;
    }
    for (ch, b) in d.iter_mut().enumerate() {
        let v = ((s >> (ch * 8)) & 255) + div255(*b as u32 * ia);
        *b = v.min(255) as u8;
    }
}

/// paint one coverage span of the rasterizer into the target (through the mask)
pub(crate) fn span(t: &mut Tgt, y: i32, x: i32, cov: &[u8], sh: &Shader, mask: Option<&Mask>, buf: &mut Vec<u8>) {
    let n = cov.len();
    let c: &[u8] = match mask {
        None => cov,
        Some(m) => {
            buf.clear();
            buf.extend_from_slice(cov);
            m.apply(y, x, buf);
            buf
        }
    };
    let stride = t.sf.stride as usize;
    let (ry, rx) = (y - t.oy, x - t.ox);
    if ry < 0 || rx < 0 {
        return;
    }
    let o = ry as usize * stride + rx as usize * 4;
    let Some(row) = t.sf.px.get_mut(o..o + n * 4) else { return };
    match sh {
        Shader::Solid(pc) => blend::mask_over(row, c, pc),
        Shader::Grad(g) => g.span(row, x, y, c),
    }
}

/// composite a premultiplied layer whose pixel (0, 0) is at device (lx, ly) into the target
/// with opacity op (0..255) and an optional mask
pub(crate) fn composite(t: &mut Tgt, layer: &Surface, lx: i32, ly: i32, op: u32, mask: Option<&Mask>) {
    let x0 = t.clip[0].max(lx);
    let y0 = t.clip[1].max(ly);
    let x1 = t.clip[2].min(lx + layer.w);
    let y1 = t.clip[3].min(ly + layer.h);
    if x1 <= x0 || y1 <= y0 || op == 0 {
        return;
    }
    let n = (x1 - x0) as usize;
    let mut m = vec![0u8; if mask.is_some() { n } else { 0 }];
    for y in y0..y1 {
        let so = (y - ly) as usize * layer.stride as usize + (x0 - lx) as usize * 4;
        let Some(src) = layer.px.get(so..so + n * 4) else { continue };
        let (ry, rx) = (y - t.oy, x0 - t.ox);
        if ry < 0 || rx < 0 {
            continue;
        }
        let d = ry as usize * t.sf.stride as usize + rx as usize * 4;
        let Some(dst) = t.sf.px.get_mut(d..d + n * 4) else { continue };
        match mask {
            None => blend::blit_over(dst, src, op),
            Some(mk) => {
                m.fill(255);
                mk.apply(y, x0, &mut m);
                for i in 0..n {
                    let k = div255(m[i] as u32 * op);
                    if k == 0 {
                        continue;
                    }
                    let c = u32::from_le_bytes([src[i * 4], src[i * 4 + 1], src[i * 4 + 2], src[i * 4 + 3]]);
                    if c != 0 {
                        over_px(dst, i, c, k);
                    }
                }
            }
        }
    }
}
