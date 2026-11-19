// direct -- what render2d's scene cannot hold, drawn straight into a gfx surface: gradient
// fills and pictures (the CPU path; the GPU path leaves them out, see lib.rs)
//
//   Target { dc, sf, at, size, cam }   where the view lies on the surface and its camera
//   fill_gradient(t, path, grad)       the path (even-odd) in the gradient's colours
//   picture(t, img, xform, clip)       a picture mapped by xform, cut at the shape's box;
//                                      bilinear between pixel centres
//   placeholder(t, xform, clip, name)  a grey box with a cross where a picture cannot be shown
//
// Coverage comes from lib/gfx's rasterizer (exact area coverage, the same bytes on every
// target); colours blend over the surface: XRGB and straight ARGB targets as opaque RGB,
// PARGB premultiplied.

use alloc::vec::Vec;

use diagram_core::geometry::{Path, Seg, Xform};
use diagram_core::render::Gradient;
use gfx::raster::Raster;
use gfx::{Dc, Surface, FMT_A8, FMT_PARGB, RULE_EVENODD, RULE_NONZERO};
use render2d::Camera;

use crate::image::Picture;

/// the view on a surface
pub struct Target<'a> {
    pub dc: &'a Dc,
    pub sf: &'a mut Surface,
    /// the view's top left corner in surface pixels
    pub at: [f64; 2],
    pub size: (u32, u32),
    pub cam: Camera,
}

impl Target<'_> {
    fn px(&self, p: [f64; 2]) -> [f64; 2] {
        let q = self.cam.to_screen(p, self.size);
        [q[0] + self.at[0], q[1] + self.at[1]]
    }

    /// the view's map from paper inches to surface pixels
    fn map(&self) -> Xform {
        let o = self.px([0.0, 0.0]);
        let x = self.px([1.0, 0.0]);
        let y = self.px([0.0, 1.0]);
        Xform { a: x[0] - o[0], b: x[1] - o[1], c: y[0] - o[0], d: y[1] - o[1], e: o[0], f: o[1] }
    }
}

/// a path's outline as lines in surface pixels (curves within a quarter pixel)
fn emit(ras: &mut Raster, path: &Path, m: &Xform) {
    for sub in &path.subs {
        let mut prev = m.apply(sub.start);
        let first = prev;
        for s in &sub.segs {
            match *s {
                Seg::Line(p) => {
                    let q = m.apply(p);
                    ras.emit_line(prev[0] as f32, prev[1] as f32, q[0] as f32, q[1] as f32);
                    prev = q;
                }
                Seg::Cubic(a, b, p) => {
                    let (a, b, q) = (m.apply(a), m.apply(b), m.apply(p));
                    // pieces by the control polygon's length: a chord error below ~0.25 px
                    let len = dist(prev, a) + dist(a, b) + dist(b, q);
                    let n = (crate::sqrt(len * 2.0) as usize + 1).clamp(1, 256);
                    let p0 = prev;
                    for i in 1..=n {
                        let r = diagram_core::geometry::cubic_at(p0, a, b, q, i as f64 / n as f64);
                        ras.emit_line(prev[0] as f32, prev[1] as f32, r[0] as f32, r[1] as f32);
                        prev = r;
                    }
                }
            }
        }
        // the raster closes nothing by itself
        if dist(prev, first) > 0.0 {
            ras.emit_line(prev[0] as f32, prev[1] as f32, first[0] as f32, first[1] as f32);
        }
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    crate::sqrt((a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]))
}

/// blend one straight colour at coverage `cov` (0..255) over the pixel at byte offset o
#[inline]
fn put(px: &mut [u8], o: usize, fmt: u8, c: u32, cov: u32) {
    let a = ((c >> 24) * cov + 127) / 255;
    if a == 0 {
        return;
    }
    let (r, g, b) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
    let inv = 255 - a;
    let d = &mut px[o..o + 4];
    if fmt == FMT_PARGB {
        d[0] = ((b * a + d[0] as u32 * inv + 127) / 255) as u8;
        d[1] = ((g * a + d[1] as u32 * inv + 127) / 255) as u8;
        d[2] = ((r * a + d[2] as u32 * inv + 127) / 255) as u8;
        d[3] = ((255 * a + d[3] as u32 * inv + 127) / 255) as u8;
    } else {
        d[0] = ((b * a + d[0] as u32 * inv + 127) / 255) as u8;
        d[1] = ((g * a + d[1] as u32 * inv + 127) / 255) as u8;
        d[2] = ((r * a + d[2] as u32 * inv + 127) / 255) as u8;
        d[3] = 255;
    }
}

/// rasterize what `ras` holds and paint every covered pixel with color(x, y) (pixel centres)
fn paint(t: &mut Target, ras: &mut Raster, rule: u32, color: &mut dyn FnMut(f64, f64) -> u32) {
    if t.sf.px.is_empty() || t.sf.fmt == FMT_A8 {
        return;
    }
    let stride = t.sf.stride as usize;
    let fmt = t.sf.fmt;
    let row0 = t.dc.row0;
    let px = &mut t.sf.px;
    ras.fill_rows(t.dc.clip, rule, &mut |y, x, cov| {
        let o = (y - row0) as usize * stride;
        for (i, &c) in cov.iter().enumerate() {
            if c == 0 {
                continue;
            }
            let xi = x as usize + i;
            let col = color(xi as f64 + 0.5, y as f64 + 0.5);
            put(px, o + xi * 4, fmt, col, c as u32);
        }
    });
}

/// a gradient fill (even-odd)
pub fn fill_gradient(t: &mut Target, ras: &mut Raster, path: &Path, g: &Gradient) {
    let m = t.map();
    ras.lines_begin();
    emit(ras, path, &m);
    if !ras.has_lines() {
        return;
    }
    // 256 steps of the ramp
    let lut: Vec<u32> = (0..256).map(|i| g.at(i as f64 / 255.0)).collect();
    let (from, to) = (m.apply(g.from), m.apply(g.to));
    let d = [to[0] - from[0], to[1] - from[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    let radial = g.radial;
    let r = crate::sqrt(l2);
    paint(t, ras, RULE_EVENODD, &mut |x, y| {
        let v = [x - from[0], y - from[1]];
        let k = if !(l2 > 0.0) {
            1.0
        } else if radial {
            crate::sqrt(v[0] * v[0] + v[1] * v[1]) / r
        } else {
            (v[0] * d[0] + v[1] * d[1]) / l2
        };
        lut[(k.clamp(0.0, 1.0) * 255.0 + 0.5) as usize]
    });
}

/// a picture: its unit square mapped by xform (paper inches), cut at the quad `clip`
pub fn picture(t: &mut Target, ras: &mut Raster, img: &Picture, xform: &Xform, clip: &[[f64; 2]; 4]) {
    let m = t.map();
    let Some(inv) = xform.then(&m).inverse() else { return };
    let quad = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|p| xform.apply(p));
    let poly = clip_convex(&quad, clip);
    if poly.len() < 3 {
        return;
    }
    ras.lines_begin();
    for i in 0..poly.len() {
        let (a, b) = (m.apply(poly[i]), m.apply(poly[(i + 1) % poly.len()]));
        ras.emit_line(a[0] as f32, a[1] as f32, b[0] as f32, b[1] as f32);
    }
    let (w, h) = (img.w as f64, img.h as f64);
    paint(t, ras, RULE_NONZERO, &mut |x, y| {
        let q = inv.apply([x, y]);
        // the picture's rows run top to bottom, its square bottom to top
        let (u, v) = (q[0] * w - 0.5, (1.0 - q[1]) * h - 0.5);
        sample(img, u, v)
    });
}

/// a grey box with a cross where a picture cannot be shown
pub fn placeholder(t: &mut Target, ras: &mut Raster, xform: &Xform, clip: &[[f64; 2]; 4]) {
    let m = t.map();
    let quad = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|p| xform.apply(p));
    let poly = clip_convex(&quad, clip);
    if poly.len() < 3 {
        return;
    }
    ras.lines_begin();
    for i in 0..poly.len() {
        let (a, b) = (m.apply(poly[i]), m.apply(poly[(i + 1) % poly.len()]));
        ras.emit_line(a[0] as f32, a[1] as f32, b[0] as f32, b[1] as f32);
    }
    paint(t, ras, RULE_NONZERO, &mut |_, _| 0xFFE4_E6EA);
    // the frame and the cross, a pixel wide
    let c = quad.map(|p| m.apply(p));
    for (a, b) in [(c[0], c[1]), (c[1], c[2]), (c[2], c[3]), (c[3], c[0]), (c[0], c[2]), (c[1], c[3])] {
        ras.lines_begin();
        thin_line(ras, a, b, 1.0);
        paint(t, ras, RULE_NONZERO, &mut |_, _| 0xFF9A_A0A8);
    }
}

fn thin_line(ras: &mut Raster, a: [f64; 2], b: [f64; 2], w: f64) {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = dist(a, b);
    if !(l > 0.0) {
        return;
    }
    let n = [-d[1] / l * w * 0.5, d[0] / l * w * 0.5];
    let p = [[a[0] + n[0], a[1] + n[1]], [b[0] + n[0], b[1] + n[1]], [b[0] - n[0], b[1] - n[1]], [a[0] - n[0], a[1] - n[1]]];
    for i in 0..4 {
        let (s, e) = (p[i], p[(i + 1) % 4]);
        ras.emit_line(s[0] as f32, s[1] as f32, e[0] as f32, e[1] as f32);
    }
}

/// bilinear between pixel centres, edges clamped
fn sample(img: &Picture, u: f64, v: f64) -> u32 {
    let (w, h) = (img.w as i64, img.h as i64);
    let (fu, fv) = (crate::floor(u), crate::floor(v));
    let x0 = (fu as i64).clamp(0, w - 1);
    let y0 = (fv as i64).clamp(0, h - 1);
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = (u - fu).clamp(0.0, 1.0);
    let fy = (v - fv).clamp(0.0, 1.0);
    let p = |x: i64, y: i64| img.px[(y * w + x) as usize];
    let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
    let mut out = 0u32;
    // premultiplied interpolation, straight result
    let ch = |c: u32, s: u32| (((c >> s) & 255) * (c >> 24)) as f64;
    let alpha = |c: u32| (c >> 24) as f64;
    let lerp2 = |va: f64, vb: f64, vc: f64, vd: f64| (va * (1.0 - fx) + vb * fx) * (1.0 - fy) + (vc * (1.0 - fx) + vd * fx) * fy;
    let al = lerp2(alpha(a), alpha(b), alpha(c), alpha(d));
    if al < 0.5 {
        return 0;
    }
    for s in [16, 8, 0] {
        let v = lerp2(ch(a, s), ch(b, s), ch(c, s), ch(d, s)) / al;
        out |= ((v + 0.5) as u32).min(255) << s;
    }
    out | (((al + 0.5) as u32).min(255) << 24)
}

/// a convex polygon cut by another convex one (Sutherland-Hodgman); either winding
fn clip_convex(subject: &[[f64; 2]], clip: &[[f64; 2]; 4]) -> Vec<[f64; 2]> {
    let area: f64 = (0..4).map(|i| clip[i][0] * clip[(i + 1) % 4][1] - clip[(i + 1) % 4][0] * clip[i][1]).sum();
    let sign = if area < 0.0 { -1.0 } else { 1.0 };
    let mut out: Vec<[f64; 2]> = subject.to_vec();
    for i in 0..4 {
        let (a, b) = (clip[i], clip[(i + 1) % 4]);
        let inside = |p: [f64; 2]| sign * ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])) >= -1e-12;
        let input = core::mem::take(&mut out);
        if input.is_empty() {
            break;
        }
        for k in 0..input.len() {
            let (p, q) = (input[k], input[(k + 1) % input.len()]);
            let (ip, iq) = (inside(p), inside(q));
            if ip {
                out.push(p);
            }
            if ip != iq {
                // the crossing with the clip edge's line
                let d1 = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
                let d2 = (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]);
                let k = d1 / (d1 - d2);
                out.push([p[0] + (q[0] - p[0]) * k, p[1] + (q[1] - p[1]) * k]);
            }
        }
    }
    out
}
