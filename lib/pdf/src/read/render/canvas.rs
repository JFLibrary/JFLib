// The pixels a page is drawn into (ISO 32000-1, 11.3): layers of premultiplied RGBA over a
// rectangle of the device, 8-bit masks (clipping paths and soft masks), and the compositing of
// a source colour into a layer with coverage, alpha and a blend mode (the separable modes of
// 11.3.5.2 and the non-separable ones of 11.3.5.3).

use alloc::rc::Rc;
use alloc::vec::Vec;
use gfx::raster::Raster;

/// a device rectangle: x0, y0 inclusive, x1, y1 exclusive
pub(super) type Rect = [i32; 4];

pub(super) fn intersect(a: Rect, b: Rect) -> Rect {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    if r[0] >= r[2] || r[1] >= r[3] { [0, 0, 0, 0] } else { r }
}

pub(super) fn is_empty(r: Rect) -> bool {
    r[0] >= r[2] || r[1] >= r[3]
}

/// zeroed memory of n bytes, None when it is not there
pub(super) fn zeroed(n: usize) -> Option<Vec<u8>> {
    gfx::try_zeroed(n)
}

/// premultiplied RGBA pixels over a rectangle of the device
pub(super) struct Layer {
    pub rect: Rect,
    pub px: Vec<u8>,
}

impl Layer {
    pub fn new(rect: Rect) -> Option<Layer> {
        let (w, h) = ((rect[2] - rect[0]).max(0) as usize, (rect[3] - rect[1]).max(0) as usize);
        Some(Layer { rect, px: zeroed(w.checked_mul(h)?.checked_mul(4)?)? })
    }

    pub fn width(&self) -> usize {
        (self.rect[2] - self.rect[0]).max(0) as usize
    }

    /// the byte offset of device pixel (x, y), which must be inside
    #[inline]
    pub fn at(&self, x: i32, y: i32) -> usize {
        ((y - self.rect[1]) as usize * self.width() + (x - self.rect[0]) as usize) * 4
    }

    pub fn fill(&mut self, c: [u8; 4]) {
        for p in self.px.chunks_exact_mut(4) {
            p.copy_from_slice(&c);
        }
    }
}

/// an 8-bit mask over a rectangle; `outside` everywhere else
pub(super) struct Mask {
    pub rect: Rect,
    pub a: Vec<u8>,
    pub outside: u8,
}

impl Mask {
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> u8 {
        if x < self.rect[0] || y < self.rect[1] || x >= self.rect[2] || y >= self.rect[3] {
            return self.outside;
        }
        self.a[(y - self.rect[1]) as usize * (self.rect[2] - self.rect[0]) as usize + (x - self.rect[0]) as usize]
    }

    /// the values of row y from x on
    pub fn row(&self, y: i32, x: i32, out: &mut [u8]) {
        let w = self.rect[2] - self.rect[0];
        if y < self.rect[1] || y >= self.rect[3] {
            out.fill(self.outside);
            return;
        }
        let base = (y - self.rect[1]) as usize * w as usize;
        for (i, o) in out.iter_mut().enumerate() {
            let xx = x + i as i32;
            *o = if xx < self.rect[0] || xx >= self.rect[2] { self.outside } else { self.a[base + (xx - self.rect[0]) as usize] };
        }
    }
}

/// the clipping path of a graphics state: a rectangle, and a mask inside it when the path is
/// not a rectangle of whole pixels
#[derive(Clone)]
pub(super) struct Clip {
    pub rect: Rect,
    pub mask: Option<Rc<Mask>>,
}

impl Clip {
    /// the pixels of the mask and_path would make (the clip's rectangle and the lines' box)
    pub fn path_area(&self, ras: &Raster) -> u64 {
        if !ras.has_lines() {
            return 0;
        }
        let r = self.path_rect(ras);
        if is_empty(r) {
            return 0;
        }
        (r[2] - r[0]) as u64 * (r[3] - r[1]) as u64
    }

    fn path_rect(&self, ras: &Raster) -> Rect {
        let b = ras.lbox();
        intersect(self.rect, [floor(b[0]), floor(b[1]), ceil(b[2]) + 1, ceil(b[3]) + 1])
    }

    /// the intersection with the area of the lines in `ras` (fill rule `rule`)
    pub fn and_path(&self, ras: &mut Raster, rule: u32) -> Option<Clip> {
        if !ras.has_lines() {
            return Some(Clip { rect: [0, 0, 0, 0], mask: None });
        }
        let rect = self.path_rect(ras);
        if is_empty(rect) {
            return Some(Clip { rect, mask: None });
        }
        let w = (rect[2] - rect[0]) as usize;
        let mut a = zeroed(w.checked_mul((rect[3] - rect[1]) as usize)?)?;
        ras.fill_rows(rect, rule, &mut |y, x, cov| {
            let o = (y - rect[1]) as usize * w + (x - rect[0]) as usize;
            a[o..o + cov.len()].copy_from_slice(cov);
        });
        if let Some(m) = &self.mask {
            for y in rect[1]..rect[3] {
                let o = (y - rect[1]) as usize * w;
                for (i, v) in a[o..o + w].iter_mut().enumerate() {
                    if *v != 0 {
                        *v = mul255(*v as u32, m.get(rect[0] + i as i32, y) as u32) as u8;
                    }
                }
            }
        }
        // a rectangle of whole pixels needs no mask
        let Some(t) = tight(&a, rect) else { return Some(Clip { rect: [0, 0, 0, 0], mask: None }) };
        let whole = (t[1]..t[3]).all(|y| {
            let o = (y - rect[1]) as usize * w;
            a[o + (t[0] - rect[0]) as usize..o + (t[2] - rect[0]) as usize].iter().all(|&v| v == 255)
        });
        if whole && self.mask.is_none() {
            return Some(Clip { rect: t, mask: None });
        }
        Some(Clip { rect: t, mask: Some(Rc::new(Mask { rect, a, outside: 0 })) })
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> u8 {
        match &self.mask {
            Some(m) => m.get(x, y),
            None => 255,
        }
    }
}

/// the smallest rectangle holding every nonzero value of a mask over `r`
fn tight(a: &[u8], r: Rect) -> Option<Rect> {
    let w = (r[2] - r[0]) as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (row, line) in a.chunks_exact(w.max(1)).enumerate() {
        let Some(first) = line.iter().position(|&v| v != 0) else { continue };
        let last = line.iter().rposition(|&v| v != 0).unwrap_or(first);
        let y = r[1] + row as i32;
        y0 = y0.min(y);
        y1 = y1.max(y + 1);
        x0 = x0.min(r[0] + first as i32);
        x1 = x1.max(r[0] + last as i32 + 1);
    }
    (x0 < x1).then_some([x0, y0, x1, y1])
}

pub(super) fn floor(v: f32) -> i32 {
    if !(v > -1.0e8) {
        return -100_000_000;
    }
    if !(v < 1.0e8) {
        return 100_000_000;
    }
    let t = v as i32;
    if (t as f32) > v { t - 1 } else { t }
}

pub(super) fn ceil(v: f32) -> i32 {
    -floor(-v)
}

#[inline]
pub(super) fn mul255(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

/// the blend modes of 11.3.5
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Blend {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl Blend {
    pub fn from_name(n: &[u8]) -> Option<Blend> {
        Some(match n {
            b"Normal" | b"Compatible" => Blend::Normal,
            b"Multiply" => Blend::Multiply,
            b"Screen" => Blend::Screen,
            b"Overlay" => Blend::Overlay,
            b"Darken" => Blend::Darken,
            b"Lighten" => Blend::Lighten,
            b"ColorDodge" => Blend::ColorDodge,
            b"ColorBurn" => Blend::ColorBurn,
            b"HardLight" => Blend::HardLight,
            b"SoftLight" => Blend::SoftLight,
            b"Difference" => Blend::Difference,
            b"Exclusion" => Blend::Exclusion,
            b"Hue" => Blend::Hue,
            b"Saturation" => Blend::Saturation,
            b"Color" => Blend::Color,
            b"Luminosity" => Blend::Luminosity,
            _ => return None,
        })
    }
}

fn sep(bm: Blend, b: f32, s: f32) -> f32 {
    match bm {
        Blend::Multiply => b * s,
        Blend::Screen => b + s - b * s,
        Blend::Overlay => sep(Blend::HardLight, s, b),
        Blend::Darken => b.min(s),
        Blend::Lighten => b.max(s),
        Blend::ColorDodge => {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        Blend::ColorBurn => {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        Blend::HardLight => {
            if s <= 0.5 {
                b * 2.0 * s
            } else {
                let t = 2.0 * s - 1.0;
                b + t - b * t
            }
        }
        Blend::SoftLight => {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { rt::fx::sqrt(b) };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }
        Blend::Difference => (b - s).abs(),
        Blend::Exclusion => b + s - 2.0 * b * s,
        _ => s,
    }
}

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut c = c;
    if n < 0.0 && l - n > 1e-6 {
        for v in &mut c {
            *v = l + (*v - l) * l / (l - n);
        }
    }
    if x > 1.0 && x - l > 1e-6 {
        for v in &mut c {
            *v = l + (*v - l) * (1.0 - l) / (x - l);
        }
    }
    c
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    if mx - mn <= 1e-6 {
        return [0.0; 3];
    }
    let mut out = [0.0; 3];
    for i in 0..3 {
        out[i] = if c[i] >= mx { s } else if c[i] <= mn { 0.0 } else { (c[i] - mn) * s / (mx - mn) };
    }
    out
}

/// B(cb, cs) for one pixel (straight colours, 0..1)
fn blend3(bm: Blend, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    match bm {
        Blend::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
        Blend::Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
        Blend::Color => set_lum(s, lum(b)),
        Blend::Luminosity => set_lum(b, lum(s)),
        _ => [sep(bm, b[0], s[0]), sep(bm, b[1], s[1]), sep(bm, b[2], s[2])],
    }
}

/// composite a straight colour s with alpha a (0..255, coverage and every alpha applied) into
/// the premultiplied pixel d
#[inline]
pub(super) fn put(d: &mut [u8], s: [u8; 3], a: u32, bm: Blend) {
    if a == 0 {
        return;
    }
    if bm == Blend::Normal || d[3] == 0 {
        if a >= 255 {
            d[0] = s[0];
            d[1] = s[1];
            d[2] = s[2];
            d[3] = 255;
            return;
        }
        let ia = 255 - a;
        d[0] = (mul255(s[0] as u32, a) + mul255(d[0] as u32, ia)) as u8;
        d[1] = (mul255(s[1] as u32, a) + mul255(d[1] as u32, ia)) as u8;
        d[2] = (mul255(s[2] as u32, a) + mul255(d[2] as u32, ia)) as u8;
        d[3] = (a + mul255(d[3] as u32, ia)) as u8;
        return;
    }
    let ab = d[3] as f32 / 255.0;
    let cb = [d[0] as f32 / 255.0 / ab, d[1] as f32 / 255.0 / ab, d[2] as f32 / 255.0 / ab];
    let cb = [cb[0].min(1.0), cb[1].min(1.0), cb[2].min(1.0)];
    let cs = [s[0] as f32 / 255.0, s[1] as f32 / 255.0, s[2] as f32 / 255.0];
    let bl = blend3(bm, cb, cs);
    let as_ = a as f32 / 255.0;
    for i in 0..3 {
        let cr = (1.0 - ab) * cs[i] + ab * bl[i].clamp(0.0, 1.0);
        let v = as_ * cr + (1.0 - as_) * d[i] as f32 / 255.0;
        d[i] = (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
    let ao = as_ + ab * (1.0 - as_);
    d[3] = (ao * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
}

/// where a paint's colour comes from: one colour, or a colour per pixel
pub(super) trait Shader {
    /// straight RGBA of the pixels x .. x + out.len() of row y
    fn row(&self, y: i32, x: i32, out: &mut [[u8; 4]]);
}

pub(super) enum Src<'a> {
    Solid([u8; 3]),
    Shade(&'a dyn Shader),
}

/// how a paint goes into its layer
pub(super) struct Put<'a> {
    pub clip: &'a Clip,
    pub alpha: u8,
    pub blend: Blend,
    pub smask: Option<&'a Mask>,
}

/// the buffers of a row's compositing
#[derive(Default)]
struct Rows {
    clip: Vec<u8>,
    smask: Vec<u8>,
    shade: Vec<[u8; 4]>,
}

/// composite one row of coverage at (x, y) (inside the layer) with the paint
#[inline]
fn put_row(px: &mut [u8], w: usize, r0: Rect, y: i32, x: i32, cov: &[u8], src: &Src, p: &Put, b: &mut Rows) {
    let n = cov.len();
    if let Some(m) = &p.clip.mask {
        b.clip.resize(n, 0);
        m.row(y, x, &mut b.clip);
    }
    if let Some(m) = p.smask {
        b.smask.resize(n, 0);
        m.row(y, x, &mut b.smask);
    }
    if let Src::Shade(s) = src {
        b.shade.resize(n, [0; 4]);
        s.row(y, x, &mut b.shade);
    }
    let o = ((y - r0[1]) as usize * w + (x - r0[0]) as usize) * 4;
    for i in 0..n {
        let mut a = cov[i] as u32;
        if a == 0 {
            continue;
        }
        if p.clip.mask.is_some() {
            a = mul255(a, b.clip[i] as u32);
        }
        if p.smask.is_some() {
            a = mul255(a, b.smask[i] as u32);
        }
        if p.alpha != 255 {
            a = mul255(a, p.alpha as u32);
        }
        let s = match src {
            Src::Solid(c) => *c,
            Src::Shade(_) => {
                let c = b.shade[i];
                a = mul255(a, c[3] as u32);
                [c[0], c[1], c[2]]
            }
        };
        put(&mut px[o + i * 4..o + i * 4 + 4], s, a, p.blend);
    }
}

/// fill the area of the lines in `ras` with `src`; returns the pixels touched (for the budget)
pub(super) fn fill(layer: &mut Layer, ras: &mut Raster, rule: u32, src: &Src, p: &Put) -> u64 {
    if !ras.has_lines() || p.alpha == 0 {
        return 0;
    }
    let area = intersect(layer.rect, p.clip.rect);
    if is_empty(area) {
        return 0;
    }
    let mut work = 0u64;
    let mut b = Rows::default();
    let w = layer.width();
    let r0 = layer.rect;
    let px = &mut layer.px;
    ras.fill_rows(area, rule, &mut |y, x, cov| {
        work += cov.len() as u64;
        put_row(px, w, r0, y, x, cov, src, p, &mut b);
    });
    work
}

/// an 8-bit coverage over a rectangle (a glyph drawn once, placed many times)
/// the longest side of a glyph's coverage (larger glyphs are filled as paths)
pub(super) const COVER_SIDE: usize = 4096;

pub(super) struct Cover {
    /// where its corner is, relative to the place it is drawn at
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub a: Vec<u8>,
}

impl Cover {
    /// the coverage of the lines in `ras`, relative to (ox, oy); the lines must lie where x and y
    /// are positive (the rasterizer's rows begin there)
    /// the coverage of the lines in `ras` (None: larger than COVER_SIDE on a side)
    pub fn of(ras: &mut Raster, ox: i32, oy: i32) -> Option<Cover> {
        if !ras.has_lines() {
            return Some(Cover { x: 0, y: 0, w: 0, h: 0, a: Vec::new() });
        }
        let b = ras.lbox();
        let r = [floor(b[0]), floor(b[1]), ceil(b[2]) + 1, ceil(b[3]) + 1];
        let (w, h) = ((r[2] - r[0]).max(0) as usize, (r[3] - r[1]).max(0) as usize);
        // each side too: a glyph of 2 million by half a pixel passed, and the rasterizer's
        // buffers grew to its width (w * h also wraps on 32-bit targets)
        if w > COVER_SIDE || h > COVER_SIDE || w.checked_mul(h).is_none_or(|n| n > 1 << 22) {
            return None;
        }
        let mut a = alloc::vec![0u8; w * h];
        ras.fill_rows(r, gfx::RULE_NONZERO, &mut |y, x, cov| {
            let o = (y - r[1]) as usize * w + (x - r[0]) as usize;
            a[o..o + cov.len()].copy_from_slice(cov);
        });
        Some(Cover { x: r[0] - ox, y: r[1] - oy, w, h, a })
    }
}

/// a coverage placed with its origin at (ox, oy), filled with `src`
pub(super) fn fill_cover(layer: &mut Layer, c: &Cover, ox: i32, oy: i32, src: &Src, p: &Put) -> u64 {
    if c.w == 0 || p.alpha == 0 {
        return 0;
    }
    let (x0, y0) = (ox.saturating_add(c.x), oy.saturating_add(c.y));
    let r = [x0, y0, x0.saturating_add(c.w as i32), y0.saturating_add(c.h as i32)];
    let area = intersect(intersect(layer.rect, p.clip.rect), r);
    if is_empty(area) {
        return 0;
    }
    let mut b = Rows::default();
    let w = layer.width();
    let r0 = layer.rect;
    for y in area[1]..area[3] {
        let o = (y - y0) as usize * c.w;
        let row = &c.a[o + (area[0] - x0) as usize..o + (area[2] - x0) as usize];
        put_row(&mut layer.px, w, r0, y, area[0], row, src, p, &mut b);
    }
    ((area[2] - area[0]) as u64) * ((area[3] - area[1]) as u64)
}

/// draw the premultiplied layer `src` into `dst` through the clip, with alpha, blend mode and
/// soft mask (a transparency group)
pub(super) fn composite(dst: &mut Layer, src: &Layer, p: &Put) -> u64 {
    let area = intersect(intersect(dst.rect, src.rect), p.clip.rect);
    if is_empty(area) {
        return 0;
    }
    let sw = src.width();
    let dw = dst.width();
    for y in area[1]..area[3] {
        for x in area[0]..area[2] {
            let so = ((y - src.rect[1]) as usize * sw + (x - src.rect[0]) as usize) * 4;
            let sa = src.px[so + 3] as u32;
            if sa == 0 {
                continue;
            }
            let mut a = sa;
            a = mul255(a, p.clip.get(x, y) as u32);
            if let Some(m) = p.smask {
                a = mul255(a, m.get(x, y) as u32);
            }
            if p.alpha != 255 {
                a = mul255(a, p.alpha as u32);
            }
            let un = |v: u8| ((v as u32 * 255 + sa / 2) / sa).min(255) as u8;
            let s = [un(src.px[so]), un(src.px[so + 1]), un(src.px[so + 2])];
            let doff = ((y - dst.rect[1]) as usize * dw + (x - dst.rect[0]) as usize) * 4;
            put(&mut dst.px[doff..doff + 4], s, a, p.blend);
        }
    }
    ((area[2] - area[0]) as u64) * ((area[3] - area[1]) as u64)
}
