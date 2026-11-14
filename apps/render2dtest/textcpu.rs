// textcpu -- what the text shader computes, on the CPU (G6c): the coverage of a pixel from a glyph's MSDF tile as text.wgsl
// computes it (bilinear over the tile's four texels, the median, its gradient through the em -> pixel map, the box filter
// of that edge), and the reference: the glyph's outline flattened within 1/256 px and filled by lib/gfx (non-zero, exact
// area).  `render2dtest --text-error` measures the edge error of size classes against it from 8 to 4000 px per em, and
// what a tile costs (docs/RENDER2D.md "Entscheidungen in G6c").

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gfx::path::Path;
use gfx::raster::Raster;
use render2d::msdf::{self, Params, Seg, Tile};
use render2d::math::sqrt;
use render2d::Fonts;

use crate::tests::out;

pub use crate::gallery::{SANS, SERIF};

/// the share of a unit square around the centre inside the half plane whose edge lies s from it (s > 0: outside), m its
/// normal (stroke.wgsl cov)
pub fn cov(s: f64, m: [f64; 2]) -> f64 {
    let a = m[0].abs().min(m[1].abs());
    let b = m[0].abs().max(m[1].abs());
    let x = -s;
    let outer = 0.5 * (a + b);
    if x >= outer {
        return 1.0;
    }
    if x <= -outer {
        return 0.0;
    }
    if a < 1e-4 {
        return (0.5 + x / b).clamp(0.0, 1.0);
    }
    let inner = 0.5 * (b - a);
    if x < -inner {
        let t = x + outer;
        return t * t / (2.0 * a * b);
    }
    if x > inner {
        let t = outer - x;
        return 1.0 - t * t / (2.0 * a * b);
    }
    0.5 + x / b
}

fn hyp(x: f64, y: f64) -> f64 {
    sqrt(x * x + y * y)
}

fn floor(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

/// how the shader takes the distance's direction: the median channel's gradient (G6c), or the true distance's (alpha)
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Grad {
    Median,
    Alpha,
}

/// the coverage at texel coordinates t of the tile (texel i's centre at i + 0.5) and the map j from pixels to texels
/// (dt = j dq, [j00, j01, j10, j11]), as text.wgsl's glyph_cov computes it
pub fn tile_cov(tile: &Tile, p: &Params, t: [f64; 2], j: [f64; 4], grad: Grad) -> f64 {
    let (w, h) = (tile.w as i64, tile.h as i64);
    let tx = t[0] - 0.5;
    let ty = t[1] - 0.5;
    let (fx0, fy0) = (floor(tx), floor(ty));
    let (fx, fy) = (tx - fx0, ty - fy0);
    let at = |i: i64, k: i64| -> [f64; 4] {
        let (i, k) = (i.clamp(0, w - 1), k.clamp(0, h - 1));
        let o = ((k * w + i) * 4) as usize;
        [tile.px[o] as f64 / 255.0, tile.px[o + 1] as f64 / 255.0, tile.px[o + 2] as f64 / 255.0, tile.px[o + 3] as f64 / 255.0]
    };
    let (i0, k0) = (fx0 as i64, fy0 as i64);
    let (c00, c10, c01, c11) = (at(i0, k0), at(i0 + 1, k0), at(i0, k0 + 1), at(i0 + 1, k0 + 1));
    let mut v = [0.0; 4];
    let mut gx = [0.0; 4];
    let mut gy = [0.0; 4];
    for c in 0..4 {
        let a = c00[c] + (c10[c] - c00[c]) * fx;
        let b = c01[c] + (c11[c] - c01[c]) * fx;
        v[c] = a + (b - a) * fy;
        gx[c] = (c10[c] - c00[c]) + ((c11[c] - c01[c]) - (c10[c] - c00[c])) * fy;
        gy[c] = b - a;
    }
    // the median channel
    let (r, g, b) = (v[0], v[1], v[2]);
    let m = if (r <= g && g <= b) || (b <= g && g <= r) {
        1
    } else if (g <= r && r <= b) || (b <= r && r <= g) {
        0
    } else {
        2
    };
    let d = (v[m] - 0.5) * p.range;
    let k = if grad == Grad::Median { m } else { 3 };
    let gt = [gx[k] * p.range, gy[k] * p.range];
    // (dd/dq = J^T dd/dt)
    let gq = [j[0] * gt[0] + j[2] * gt[1], j[1] * gt[0] + j[3] * gt[1]];
    let l = sqrt(gq[0] * gq[0] + gq[1] * gq[1]);
    if !(l > 1e-9) {
        return if d > 0.0 { 1.0 } else { 0.0 };
    }
    let s = d / l;
    cov(-s, [gq[0] / l, gq[1] / l])
}

/// the outline in pixels as lines (y down: x0 + k x, y0 - k y), each piece within tol px
fn outline_px(contours: &[Vec<Seg>], k: f64, o: [f64; 2], tol: f64, path: &mut Path) {
    let px = |p: [f64; 2]| [o[0] + k * p[0], o[1] - k * p[1]];
    for c in contours {
        let mut first = true;
        for s in c {
            let (pts, n): (Vec<[f64; 2]>, usize) = match *s {
                Seg::Line(a, b) => (vec![px(a), px(b)], 1),
                Seg::Quad(a, b, cc) => {
                    let (a, b, cc) = (px(a), px(b), px(cc));
                    let d = hyp(a[0] - 2.0 * b[0] + cc[0], a[1] - 2.0 * b[1] + cc[1]);
                    (vec![a, b, cc], (sqrt(d / (4.0 * tol)) as usize + 1).max(1))
                }
                Seg::Cubic(a, b, cc, dd) => {
                    let (a, b, cc, dd) = (px(a), px(b), px(cc), px(dd));
                    let m1 = hyp(a[0] - 2.0 * b[0] + cc[0], a[1] - 2.0 * b[1] + cc[1]);
                    let m2 = hyp(b[0] - 2.0 * cc[0] + dd[0], b[1] - 2.0 * cc[1] + dd[1]);
                    (vec![a, b, cc, dd], (sqrt(6.0 * m1.max(m2) / (8.0 * tol)) as usize + 1).max(1))
                }
            };
            if first {
                path.move_f(pts[0][0] as f32, pts[0][1] as f32);
                first = false;
            }
            for i in 1..=n {
                let t = i as f64 / n as f64;
                let q = match pts.len() {
                    2 => pts[1],
                    3 => {
                        let u = 1.0 - t;
                        [u * u * pts[0][0] + 2.0 * u * t * pts[1][0] + t * t * pts[2][0], u * u * pts[0][1] + 2.0 * u * t * pts[1][1] + t * t * pts[2][1]]
                    }
                    _ => {
                        let u = 1.0 - t;
                        let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                        [b0 * pts[0][0] + b1 * pts[1][0] + b2 * pts[2][0] + b3 * pts[3][0], b0 * pts[0][1] + b1 * pts[1][1] + b2 * pts[2][1] + b3 * pts[3][1]]
                    }
                };
                path.line_f(q[0] as f32, q[1] as f32);
            }
        }
        path.close();
    }
}

/// a glyph's exact coverage (lib/gfx, non-zero) in a w x h image, em -> px: (o.x + k x, o.y - k y)
pub fn reference(contours: &[Vec<Seg>], k: f64, o: [f64; 2], w: i32, h: i32) -> Vec<u8> {
    let mut path = Path::default();
    outline_px(contours, k, o, 1.0 / 256.0, &mut path);
    let mut ras = Raster::default();
    let mut m = vec![0u8; (w * h) as usize];
    path.fill_lines(&mut ras);
    ras.fill_rows([0, 0, w, h], gfx::RULE_NONZERO, &mut |y, x, c| {
        let o = y as usize * w as usize + x as usize;
        for (q, v) in c.iter().enumerate() {
            if o + q < m.len() {
                m[o + q] = m[o + q].max(*v);
            }
        }
    });
    m
}

/// the edge error of one glyph at one size: (pixels compared, max |error| codes, sum of |error| codes, pixels above 1/16
/// px = 16 codes, above 1/4 px = 64 codes, the ink's sum of both)
#[derive(Clone, Copy, Default)]
pub struct Err {
    pub n: u64,
    pub max: u32,
    pub sum: u64,
    pub over16: u64,
    pub over64: u64,
    pub ink_ref: u64,
    pub ink_gpu: u64,
}

impl Err {
    pub fn add(&mut self, o: &Err) {
        self.n += o.n;
        self.max = self.max.max(o.max);
        self.sum += o.sum;
        self.over16 += o.over16;
        self.over64 += o.over64;
        self.ink_ref += o.ink_ref;
        self.ink_gpu += o.ink_gpu;
    }
}

/// the error of a tile drawn at k px per em against the outline (pixels within 2 px of the reference's edge)
pub fn glyph_error(contours: &[Vec<Seg>], tile: &Tile, p: &Params, k: f64, grad: Grad) -> Err {
    let Some(b) = msdf::bounds(contours) else { return Err::default() };
    let w = ((b[2] - b[0]) * k + 8.0) as i32;
    let h = ((b[3] - b[1]) * k + 8.0) as i32;
    // (a quarter pixel off the grid, so that edges do not all fall on pixel borders)
    let o = [4.0 - b[0] * k + 0.25, 4.0 + b[3] * k + 0.375];
    let rf = reference(contours, k, o, w, h);
    let mut e = Err::default();
    let (wu, hu) = (w as usize, h as usize);
    // edge pixels: the reference's partial ones, and 2 px around them
    let mut near = vec![false; wu * hu];
    for y in 0..hu {
        for x in 0..wu {
            let v = rf[y * wu + x];
            if v != 0 && v != 255 {
                for dy in -2i64..=2 {
                    for dx in -2i64..=2 {
                        let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                        if xx >= 0 && yy >= 0 && (xx as usize) < wu && (yy as usize) < hu {
                            near[yy as usize * wu + xx as usize] = true;
                        }
                    }
                }
            }
        }
    }
    // pixels -> texels: t = E e - (x0, y0), e = ((q.x - o.x) / k, (o.y - q.y) / k)
    let j = [p.em / k, 0.0, 0.0, -p.em / k];
    for y in 0..hu {
        for x in 0..wu {
            if !near[y * wu + x] {
                continue;
            }
            let q = [x as f64 + 0.5, y as f64 + 0.5];
            let em = [(q[0] - o[0]) / k, (o[1] - q[1]) / k];
            let t = [em[0] * p.em - tile.x0 as f64, em[1] * p.em - tile.y0 as f64];
            let c = tile_cov(tile, p, t, j, grad);
            let g = (c * 255.0 + 0.5) as i64;
            let r = rf[y * wu + x] as i64;
            let d = (g - r).unsigned_abs() as u32;
            e.n += 1;
            e.max = e.max.max(d);
            e.sum += d as u64;
            e.over16 += (d > 16) as u64;
            e.over64 += (d > 64) as u64;
            e.ink_ref += r as u64;
            e.ink_gpu += g as u64;
        }
    }
    e
}

fn write_img(path: &str, w: u32, h: u32, px: &[u8]) {
    let mut v = Vec::with_capacity(px.len() + 8);
    v.extend_from_slice(&w.to_le_bytes());
    v.extend_from_slice(&h.to_le_bytes());
    v.extend_from_slice(px);
    let _ = plat::write_file(path, &v);
}

/// render2dtest --text-dump DIR --char C --em E --class E2 --range R: a glyph's tile (DIR/tile.rgba, its alpha
/// DIR/alpha.gray, rows flipped to y down), the reference and the shader's coverage (DIR/ref.gray, DIR/gpu.gray) at E px
/// per em
pub fn text_dump(dir: &str) -> i32 {
    let mut fonts = Fonts::new();
    let f = if plat::args().iter().any(|a| a == "--sans") { fonts.add(SANS, 0) } else { fonts.add(SERIF, 0) };
    let Ok(f) = f else { return 1 };
    let c = plat::arg_value("--char").and_then(|s| s.chars().next()).unwrap_or('a');
    let k: f64 = plat::arg_value("--em").and_then(|v| v.parse().ok()).unwrap_or(64.0);
    let em: f64 = plat::arg_value("--class").and_then(|v| v.parse().ok()).unwrap_or(32.0);
    let range: f64 = plat::arg_value("--range").and_then(|v| v.parse().ok()).unwrap_or(4.0);
    let grad = if plat::args().iter().any(|a| a == "--alpha") { Grad::Alpha } else { Grad::Median };
    let p = Params { em, range, pad: (range * 0.5).max(1.0) as u32 + 1 };
    let g = fonts.glyph_of(f, c);
    let Some(sh) = fonts.shape(f, g) else { return 1 };
    let contours = sh.contours.clone();
    for (i, cn) in contours.iter().enumerate() {
        out(&format!("contour {i}: {} segments, starts {:?}", cn.len(), cn[0].start()));
    }
    let Some(tile) = msdf::tile(&contours, &p) else { return 1 };
    let (tw, th) = (tile.w as usize, tile.h as usize);
    let mut rgba = Vec::new();
    let mut alpha = Vec::new();
    for j in (0..th).rev() {
        for i in 0..tw {
            let o = (j * tw + i) * 4;
            rgba.extend_from_slice(&tile.px[o..o + 4]);
            alpha.push(tile.px[o + 3]);
        }
    }
    write_img(&format!("{dir}/tile.rgba"), tile.w, tile.h, &rgba);
    write_img(&format!("{dir}/alpha.gray"), tile.w, tile.h, &alpha);
    let Some(b) = msdf::bounds(&contours) else { return 1 };
    let w = ((b[2] - b[0]) * k + 8.0) as i32;
    let h = ((b[3] - b[1]) * k + 8.0) as i32;
    let o = [4.0 - b[0] * k + 0.25, 4.0 + b[3] * k + 0.375];
    let rf = reference(&contours, k, o, w, h);
    let jm = [p.em / k, 0.0, 0.0, -p.em / k];
    let mut gp = vec![0u8; rf.len()];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let q = [x as f64 + 0.5, y as f64 + 0.5];
            let e = [(q[0] - o[0]) / k, (o[1] - q[1]) / k];
            let t = [e[0] * p.em - tile.x0 as f64, e[1] * p.em - tile.y0 as f64];
            gp[y * w as usize + x] = (tile_cov(&tile, &p, t, jm, grad) * 255.0 + 0.5) as u8;
        }
    }
    write_img(&format!("{dir}/ref.gray"), w as u32, h as u32, &rf);
    write_img(&format!("{dir}/gpu.gray"), w as u32, h as u32, &gp);
    let e = glyph_error(&contours, &tile, &p, k, grad);
    out(&format!("tile {}x{} at ({}, {}); {} px, max {} mean {:.2} ink {:+.3}%", tile.w, tile.h, tile.x0, tile.y0, e.n, e.max, e.sum as f64 / e.n.max(1) as f64,
                 100.0 * (e.ink_gpu as f64 / e.ink_ref.max(1) as f64 - 1.0)));
    0
}

/// render2dtest --text-error: the classes' edge error from 8 to 4000 px per em, the tiles' size and time
pub fn text_error() -> i32 {
    let mut fonts = Fonts::new();
    let fs = [("DejaVu Serif", fonts.add(SERIF, 0)), ("Inter", fonts.add(SANS, 0))];
    let glyphs = "agRe&S@5%";
    let classes: [(f64, f64); 8] = [(16.0, 4.0), (16.0, 6.0), (24.0, 4.0), (32.0, 4.0), (32.0, 8.0), (48.0, 4.0), (64.0, 4.0), (128.0, 4.0)];
    let sizes = [8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0, 96.0, 128.0, 192.0, 256.0, 384.0, 512.0, 1000.0, 2000.0, 4000.0];
    let only: Option<f64> = plat::arg_value("--em").and_then(|v| v.parse().ok());
    let grad = if plat::args().iter().any(|a| a == "--alpha") { Grad::Alpha } else { Grad::Median };
    for (name, f) in fs {
        let Ok(f) = f else {
            out(&format!("FAIL {name}: the font did not load"));
            return 1;
        };
        out(&format!("== {name}: glyphs {glyphs}, error against lib/gfx in codes of pixels within 2 px of the edge (mean / p max; >16: share above 1/16 px, >64: above 1/4 px); ink: gpu / ref - 1"));
        for &(em, range) in &classes {
            if only.is_some_and(|v| v != em) {
                continue;
            }
            let p = Params { em, range, pad: (range * 0.5).max(1.0) as u32 + 1 };
            let mut tiles = Vec::new();
            let t0 = plat::time_us();
            let (mut texels, mut bytes) = (0u64, 0u64);
            for c in glyphs.chars() {
                let g = fonts.glyph_of(f, c);
                let Some(sh) = fonts.shape(f, g) else { continue };
                let contours = sh.contours.clone();
                if let Some(t) = msdf::tile(&contours, &p) {
                    texels += (t.w * t.h) as u64;
                    bytes += t.px.len() as u64;
                    tiles.push((contours, t));
                }
            }
            let ms = (plat::time_us() - t0) as f64 / 1000.0;
            out(&format!("-- class {em} texels/em, range {range}: {} tiles, {texels} texels ({:.1} KB, {:.0} texels a glyph), {:.2} ms a tile", tiles.len(), bytes as f64 / 1024.0,
                         texels as f64 / tiles.len().max(1) as f64, ms / tiles.len().max(1) as f64));
            let mut row = alloc::string::String::new();
            for &k in &sizes {
                // (minified more than range / 2 texels a pixel holds: beyond the class)
                if k * range < 1.4 * em * 0.999 && k < 64.0 {
                    row.push_str(&format!("  {k:>4}: -"));
                    continue;
                }
                let mut e = Err::default();
                for (c, t) in &tiles {
                    e.add(&glyph_error(c, t, &p, k, grad));
                }
                let mean = e.sum as f64 / e.n.max(1) as f64;
                let ink = e.ink_gpu as f64 / e.ink_ref.max(1) as f64 - 1.0;
                row.push_str(&format!("  {k:>4}: {mean:.2}/{} >16 {:.2}% >64 {:.3}% ink {:+.3}%", e.max, 100.0 * e.over16 as f64 / e.n.max(1) as f64, 100.0 * e.over64 as f64 / e.n.max(1) as f64, 100.0 * ink));
                out(&row);
                row.clear();
            }
        }
    }
    0
}
