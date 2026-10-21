// outline -- glyph outline buffer, path emitter and the operations of synthetic styles
//                                                                       [src/text/outline.s]
//
//   Outline::reset / point / reserve / close / contour / npts / bbox
//   emit(path, gp, zones)       append the buffer to the path with the gp transform (TrueType
//                               contours exactly like glyph_path; cubic contours flattened like
//                               path_cubic); zones = FS_SHARP grid fitting (fit_zones)
//   embolden(strength)          FreeType FT_Outline_EmboldenXY(strength, strength)
//   shear(k), transform(from, a, b, c, d), translate(from, dx, dy)
//   owner_set / owner_is        "prepared" tracking: a box query leaves the untransformed
//                               outline of (font context, glyph) in the buffer
//
// Points f32 x, y (font units, y up) + u8 tags; contour ends u32 (last point index).
// TrueType contours (OLT_ON / OLT_OFF) follow the implied-midpoint rules; cubic contours
// (CFF) start with an on-curve point, two OLT_CUBIC points between on-curve points.

use alloc::vec::Vec;

use gfx::path::Path;
use rt::fx::{cvt, cvtt, maxss, minss, sqrt};

use crate::font::Gp;

pub const OLT_OFF: u8 = 0; // quadratic control point
pub const OLT_ON: u8 = 1; // on-curve point
pub const OLT_CUBIC: u8 = 2; // cubic control point
pub const OL_MAX_PTS: usize = 1 << 20;
pub const OL_MAX_CONT: usize = 1 << 18;
const ZONE_MAX: usize = 8;
const F_DMIN: f32 = -0.9375; // shift only for turns below ~160 degrees
pub const F_TAN12: f32 = 0.21255656;
pub const F_INV30: f32 = 0.033333335;

/// FS_SHARP zones: {z (font units), p (px)} ascending
#[derive(Clone, Copy, Default)]
pub struct Zones {
    pub n: usize,
    pub z: [(f32, f32); ZONE_MAX],
}

impl Zones {
    /// descender, 0, x-height, cap height, ascender snapped with sy  [ol_fit_zones]
    pub fn fit(desc: i32, xh: i32, caph: i32, asc: i32, sy: f32) -> Zones {
        let mut zn = Zones::default();
        for v in [desc, 0, xh, caph, asc] {
            let f = v as f32;
            if zn.n != 0 && !(f > zn.z[zn.n - 1].0) {
                continue; // values <= the previous one are skipped
            }
            if zn.n >= ZONE_MAX {
                break;
            }
            zn.z[zn.n] = (f, cvt(f * sy) as f32);
            zn.n += 1;
        }
        zn
    }
}

#[derive(Default)]
pub struct Outline {
    pub pts: Vec<(f32, f32)>,
    pub tags: Vec<u8>,
    pub ends: Vec<u32>,
    cstart: u32,
    pub err: bool,
    owner: (u64, u32), // (font context serial, glyph); serial 0 = none
}

impl Outline {
    /// empty the buffer (owner cleared)  [ol_reset]
    pub fn reset(&mut self) {
        self.pts.clear();
        self.tags.clear();
        self.ends.clear();
        self.cstart = 0;
        self.err = false;
        self.owner = (0, u32::MAX);
    }

    pub fn npts(&self) -> usize {
        self.pts.len()
    }

    /// append a point (font units, OLT_*)  [ol_point]
    pub fn point(&mut self, x: f32, y: f32, tag: u8) -> bool {
        if self.pts.len() + 1 > OL_MAX_PTS {
            self.err = true;
            return false;
        }
        self.pts.push((x, y));
        self.tags.push(tag);
        true
    }

    /// append n points (0, 0, OLT_OFF) to be filled in; the index of the first  [ol_reserve]
    pub fn reserve(&mut self, n: usize) -> Option<usize> {
        let first = self.pts.len();
        if first + n > OL_MAX_PTS {
            self.err = true;
            return None;
        }
        self.pts.resize(first + n, (0.0, 0.0));
        self.tags.resize(first + n, 0);
        Some(first)
    }

    /// end the current contour  [ol_close]
    pub fn close(&mut self) {
        let n = self.pts.len() as u32;
        if n > self.cstart {
            self.contour(n - 1);
        }
    }

    /// end a contour at point index last (last < cstart: empty, ignored)  [ol_contour]
    pub fn contour(&mut self, last: u32) -> bool {
        if last as usize >= self.pts.len() {
            self.err = true;
            return false;
        }
        if last < self.cstart {
            return true;
        }
        if self.ends.len() >= OL_MAX_CONT {
            self.err = true;
            return false;
        }
        self.ends.push(last);
        self.cstart = last + 1;
        true
    }

    pub fn owner_set(&mut self, serial: u64, glyph: u32) {
        self.owner = (serial, glyph);
    }

    pub fn owner_is(&self, serial: u64, glyph: u32) -> bool {
        self.owner == (serial, glyph)
    }

    /// control box {xmin, ymin, xmax, ymax} (floor / ceil, clamped to +-1e6)  [ol_box]
    pub fn bbox(&self) -> Option<[i32; 4]> {
        let first = *self.pts.first()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
        for &(x, y) in &self.pts {
            x0 = minss(x0, x);
            x1 = maxss(x1, x);
            y0 = minss(y0, y);
            y1 = maxss(y1, y);
        }
        let mut b = [0i32; 4];
        for (i, v) in [x0, y0, x1, y1].into_iter().enumerate() {
            let v = minss(maxss(v, -1_000_000.0), 1_000_000.0);
            let mut t = cvtt(v);
            let tf = t as f32;
            if i < 2 {
                if tf > v {
                    t -= 1; // floor
                }
            } else if !(tf >= v) {
                t += 1; // ceil
            }
            b[i] = t;
        }
        Some(b)
    }

    /// x += y * k  [ol_shear]
    pub fn shear(&mut self, k: f32) {
        for p in self.pts.iter_mut() {
            p.0 = p.1 * k + p.0;
        }
    }

    /// points from..n: (x, y) -> (a x + c y, b x + d y)  [ol_transform]
    pub fn transform(&mut self, from: usize, a: f32, b: f32, c: f32, d: f32) {
        let from = from.min(self.pts.len());
        for p in self.pts[from..].iter_mut() {
            let (x, y) = *p;
            *p = (x * a + y * c, x * b + y * d);
        }
    }

    /// points from..n += (dx, dy)  [ol_translate]
    pub fn translate(&mut self, from: usize, dx: f32, dy: f32) {
        let from = from.min(self.pts.len());
        for p in self.pts[from..].iter_mut() {
            p.0 = p.0 + dx;
            p.1 = p.1 + dy;
        }
    }

    /// FreeType FT_Outline_EmboldenXY(strength, strength): every edge moves outwards by
    /// strength / 2, the glyph grows up / right by strength  [ol_embolden]
    pub fn embolden(&mut self, strength: f32) {
        let xs = strength * 0.5;
        if xs == 0.0 {
            return;
        }
        // orientation: area = sum (y_cur - y_prev) * (x_cur + x_prev)
        let mut area = 0.0f32;
        let mut first = 0usize;
        for &e in &self.ends {
            let last = e as usize;
            let (mut px, mut py) = self.pts[last];
            for i in first..=last {
                let (x, y) = self.pts[i];
                area = area + (y - py) * (px + x);
                px = x;
                py = y;
            }
            first = last + 1;
        }
        if !(area < 0.0 || area > 0.0) {
            return; // no orientation (or NaN): leave it
        }
        let tt = !(area >= 0.0); // area < 0: TrueType (clockwise)
        let pts = &mut self.pts;
        let mut first = 0usize;
        for &e in &self.ends {
            let last = e as usize;
            let (mut in_x, mut in_y, mut l_in) = (0.0f32, 0.0f32, 0.0f32);
            let (mut anc_x, mut anc_y, mut l_anc) = (0.0f32, 0.0f32, 0.0f32);
            let mut i = last;
            let mut j = first;
            let mut k: isize = -1;
            while j != i && i as isize != k {
                let (out_x, out_y, l_out);
                if j as isize != k {
                    let ox = pts[j].0 - pts[i].0;
                    let oy = pts[j].1 - pts[i].1;
                    let l = sqrt(ox * ox + oy * oy);
                    if !(l > 0.0) {
                        // zero length: next j
                        j = if j < last { j + 1 } else { first };
                        continue;
                    }
                    out_x = ox / l;
                    out_y = oy / l;
                    l_out = l;
                } else {
                    out_x = anc_x;
                    out_y = anc_y;
                    l_out = l_anc;
                }
                if !(l_in < 0.0 || l_in > 0.0) {
                    // first segment (ucomiss + je: zero or unordered)
                    i = j;
                } else {
                    if k < 0 {
                        k = i as isize;
                        anc_x = in_x;
                        anc_y = in_y;
                        l_anc = l_in;
                    }
                    let d0 = in_x * out_x + in_y * out_y;
                    let (mut sx, mut sy) = (0.0f32, 0.0f32);
                    if d0 > F_DMIN {
                        let d = d0 + 1.0;
                        sx = in_y + out_y;
                        sy = in_x + out_x;
                        let mut q = out_x * in_y - out_y * in_x;
                        if tt {
                            sx = f32::from_bits(sx.to_bits() ^ 0x8000_0000);
                            q = f32::from_bits(q.to_bits() ^ 0x8000_0000);
                        } else {
                            sy = f32::from_bits(sy.to_bits() ^ 0x8000_0000);
                        }
                        let l = minss(l_in, l_out);
                        if !(xs * q > l * d) {
                            sx = sx * xs / d;
                            sy = sy * xs / d;
                        } else {
                            sx = sx * l / q;
                            sy = sy * l / q;
                        }
                    }
                    sx = sx + xs;
                    sy = sy + xs;
                    while i != j {
                        pts[i].0 = pts[i].0 + sx;
                        pts[i].1 = pts[i].1 + sy;
                        i = if i < last { i + 1 } else { first };
                    }
                }
                in_x = out_x;
                in_y = out_y;
                l_in = l_out;
                j = if j < last { j + 1 } else { first };
            }
            first = last + 1;
        }
    }

    /// append every contour to the path (gp transform, composite offset dx / dy honoured;
    /// zones: FS_SHARP vertical fitting)  [ol_emit]
    pub fn emit(&self, path: &mut Path, gp: &Gp, zones: Option<&Zones>) {
        let dxf = gp.dx as f32;
        let dyf = gp.dy as f32;
        let load = |i: usize| -> (f32, f32) {
            let (x, y) = self.pts[i];
            let x = (x + dxf) * gp.sx + gp.x;
            let y = y + dyf;
            let yp = match zones {
                None => y * gp.sy,
                Some(zn) => fit_y(zn, y, gp.sy),
            };
            (x, gp.y - yp)
        };
        let tags = &self.tags;
        let mut first = 0usize;
        for &e in &self.ends {
            let last = e as usize;
            if last + 1 <= first {
                first = last + 1;
                continue;
            }
            let n = last - first + 1;
            if tags[first..=last].contains(&OLT_CUBIC) {
                self.emit_cubic(path, &load, first, last, n);
                first = last + 1;
                continue;
            }
            // TrueType contour (the glyph_path walk)
            let (start, mut next, mut left);
            if tags[first] & OLT_ON != 0 {
                start = load(first);
                next = first + 1;
                left = n - 1;
            } else if tags[last] & OLT_ON != 0 {
                start = load(last);
                next = first;
                left = n - 1;
            } else {
                let a = load(last);
                let b = load(first);
                start = ((b.0 + a.0) * 0.5, (b.1 + a.1) * 0.5);
                next = first;
                left = n;
            }
            path.move_f(start.0, start.1);
            let mut pending: Option<(f32, f32)> = None;
            while left > 0 {
                left -= 1;
                let i = next;
                next += 1;
                let p = load(i);
                if tags[i] & OLT_ON != 0 {
                    match pending.take() {
                        Some(c) => path.quad_f(c.0, c.1, p.0, p.1),
                        None => path.line_f(p.0, p.1),
                    }
                } else {
                    if let Some(c) = pending {
                        path.quad_f(c.0, c.1, (p.0 + c.0) * 0.5, (p.1 + c.1) * 0.5);
                    }
                    pending = Some(p);
                }
            }
            if let Some(c) = pending {
                path.quad_f(c.0, c.1, start.0, start.1);
            }
            path.close();
            first = last + 1;
        }
    }

    // a cubic contour: start at the first on-curve point, n steps around
    fn emit_cubic(&self, path: &mut Path, load: &dyn Fn(usize) -> (f32, f32), first: usize, last: usize, n: usize) {
        let tags = &self.tags;
        let Some(s) = (first..=last).find(|&i| tags[i] & OLT_ON != 0) else { return };
        let start = load(s);
        path.move_f(start.0, start.1);
        let mut cur = start;
        let mut npend = 0;
        let (mut c1, mut c2) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32));
        let mut idx = s;
        for _ in 0..n {
            idx = if idx + 1 <= last { idx + 1 } else { first };
            let p = load(idx);
            if tags[idx] & OLT_ON == 0 {
                if npend == 0 {
                    c1 = p;
                    npend = 1;
                } else {
                    c2 = p;
                    npend = 2;
                }
                continue;
            }
            match npend {
                0 => path.line_f(p.0, p.1),
                1 => path.quad_f(c1.0, c1.1, p.0, p.1),
                _ => cubic(path, [cur.0, cur.1, c1.0, c1.1, c2.0, c2.1, p.0, p.1]),
            }
            npend = 0;
            cur = p;
        }
        path.close();
    }
}

// the zone mapping of FS_SHARP: piecewise linear through the snapped zones (pixels, y up)
fn fit_y(zn: &Zones, y: f32, sy: f32) -> f32 {
    let z = &zn.z;
    if !(y > z[0].0) {
        return (y - z[0].0) * sy + z[0].1; // below the first zone: slope sy
    }
    let mut k = 0;
    let mut left = zn.n;
    loop {
        left -= 1;
        if left == 0 {
            return (y - z[k].0) * sy + z[k].1; // above the last zone
        }
        if !(y > z[k + 1].0) {
            // p0 + (y - z0) * (p1 - p0) / (z1 - z0)
            return (y - z[k].0) * (z[k + 1].1 - z[k].1) / (z[k + 1].0 - z[k].0) + z[k].1;
        }
        k += 1;
    }
}

/// flatten the cubic {p0, c1, c2, p3} (device px; p0 is the current point) into the path,
/// the subdivision of path_cubic (tolerance 0.2 px)  [ol_cubic]
pub fn cubic(path: &mut Path, p: [f32; 8]) {
    let ax = p[0] - (p[2] + p[2]) + p[4];
    let ay = p[1] - (p[3] + p[3]) + p[5];
    let m1 = sqrt(ax * ax + ay * ay);
    let bx = p[2] - (p[4] + p[4]) + p[6];
    let by = p[3] - (p[5] + p[5]) + p[7];
    let m2 = maxss(sqrt(bx * bx + by * by), m1);
    let v = minss(sqrt(m2 * 3.75), 128.0);
    let mut n = cvtt(v);
    if !(n as f32 >= v) {
        n += 1;
    }
    let n = n.max(1);
    for i in 1..=n {
        if i == n {
            path.line_f(p[6], p[7]);
            break;
        }
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let uu = u * u;
        let b0 = uu * u;
        let b1 = uu * t * 3.0;
        let tt = t * t;
        let b2 = tt * u * 3.0;
        let b3 = tt * t;
        let x = p[0] * b0 + p[2] * b1 + p[4] * b2 + p[6] * b3;
        let y = p[1] * b0 + p[3] * b1 + p[5] * b2 + p[7] * b3;
        path.line_f(x, y);
    }
}
