// env -- the environment's light (G5c; docs/RENDER3D.md "Umgebungslicht"): diffuse through 9 spherical harmonics
// (the irradiance's coefficients, in the lights' uniform block: no texture slot), specular through a cube texture
// prefiltered per roughness (level k holds roughness k / (levels - 1)) read with the split-sum's table (dfg.rs).
// The source is a procedural sky or the caller's six faces of linear radiance (RGBA f32); levels down to 4 x 4.  Everything is made on
// the CPU in f64 when it is set: level 0 is the source, every further level a GGX convolution with N = V = R
// (importance sampling of the half vector, 128 points of the Hammersley set; a face source is read at the level
// of its own box-filtered chain the sample's solid angle asks for), the harmonics from the source's texels
// weighted by their solid angle (a sky through a 32 x 32 grid per face), times the cosine lobe's factors
// (Ramamoorthi and Hanrahan).  Uploaded as Rgba16Float with every level.
//
// Faces in Vulkan's order: +x, -x, +y, -y, +z, -z; texel (i, j) of face f looks along the direction Vulkan's
// face selection maps to it (env_dir).

use alloc::vec;
use alloc::vec::Vec;

use gpu::*;

use crate::math::{self, V3};

/// texels a side of a sky's level 0
pub const SKY_SIZE: u32 = 64;
/// samples of a prefiltered texel
const SAMPLES: u32 = 128;

/// a procedural sky: linear radiance; smooth everywhere (a glow, no sharp sun disc)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sky {
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    pub ground: [f32; 3],
    /// towards the sun's glow (any length), its colour at the centre, falling off as cos^8
    pub sun: [f64; 3],
    pub glow: [f32; 3],
}

impl Sky {
    pub const DEFAULT: Sky = Sky { zenith: [0.25, 0.45, 0.9], horizon: [0.8, 0.85, 0.9], ground: [0.3, 0.27, 0.22], sun: [0.3, 0.8, 0.5], glow: [2.0, 1.8, 1.4] };

    /// the radiance along a unit vector
    pub fn radiance(&self, d: V3) -> [f64; 3] {
        let up = d[1];
        let lerp = |a: [f32; 3], b: [f32; 3], t: f64| [a[0] as f64 + (b[0] as f64 - a[0] as f64) * t, a[1] as f64 + (b[1] as f64 - a[1] as f64) * t, a[2] as f64 + (b[2] as f64 - a[2] as f64) * t];
        let base = if up >= 0.0 {
            let x = 1.0 - up;
            lerp(self.zenith, self.horizon, x * x * x)
        } else {
            lerp(self.horizon, self.ground, (-up * 4.0).min(1.0))
        };
        let s = math::norm(self.sun);
        let c = math::dot(d, s).max(0.0);
        let c4 = c * c * c * c;
        let g = c4 * c4;
        [base[0] + self.glow[0] as f64 * g, base[1] + self.glow[1] as f64 * g, base[2] + self.glow[2] as f64 * g]
    }
}

/// what Renderer::set_environment takes
#[derive(Clone, Copy, Debug)]
pub enum EnvironmentData<'a> {
    Sky(Sky),
    /// six faces of size x size texels (a power of two, 4 ..= 1024), rows from the top, RGBA f32 linear radiance
    /// (alpha unused; RGBE, half floats and the like converted by the caller)
    Faces { size: u32, rgba: [&'a [f32]; 6] },
}

/// the environment as the GPU holds it (tests, tools): each level's six faces, RGBA as f32 of the half floats
/// uploaded, and the irradiance's harmonics (divided by pi: what a white matte surface shows)
#[derive(Clone, Debug)]
pub struct Environment {
    pub size: u32,
    pub levels: Vec<Vec<[f32; 4]>>,
    pub sh: [[f32; 3]; 9],
}

/// the unit vector of a point (s, t) in 0 ..= 1 of face f (Vulkan's face selection, inverted)
pub fn env_dir(f: usize, s: f64, t: f64) -> V3 {
    let (sc, tc) = (2.0 * s - 1.0, 2.0 * t - 1.0);
    let d = match f {
        0 => [1.0, -tc, -sc],
        1 => [-1.0, -tc, sc],
        2 => [sc, 1.0, tc],
        3 => [sc, -1.0, -tc],
        4 => [sc, -tc, 1.0],
        _ => [-sc, -tc, -1.0],
    };
    math::norm(d)
}

/// the face and (s, t) a unit vector falls on (Vulkan's face selection)
pub fn env_face(d: V3) -> (usize, f64, f64) {
    let (ax, ay, az) = (d[0].abs(), d[1].abs(), d[2].abs());
    let (f, sc, tc, ma) = if ax >= ay && ax >= az {
        if d[0] >= 0.0 { (0, -d[2], -d[1], ax) } else { (1, d[2], -d[1], ax) }
    } else if ay >= az {
        if d[1] >= 0.0 { (2, d[0], d[2], ay) } else { (3, d[0], -d[2], ay) }
    } else if d[2] >= 0.0 {
        (4, d[0], -d[1], az)
    } else {
        (5, -d[0], -d[1], az)
    };
    (f, 0.5 * (sc / ma + 1.0), 0.5 * (tc / ma + 1.0))
}

/// the 9 real spherical harmonics at a unit vector
fn sh9(d: V3) -> [f64; 9] {
    let (x, y, z) = (d[0], d[1], d[2]);
    [0.282_095, 0.488_603 * y, 0.488_603 * z, 0.488_603 * x, 1.092_548 * x * y, 1.092_548 * y * z, 0.315_392 * (3.0 * z * z - 1.0), 1.092_548 * x * z, 0.546_274 * (x * x - y * y)]
}

/// a face source with its box-filtered levels: [level][face] rows of rgb
struct Source {
    size: usize,
    levels: Vec<[Vec<[f64; 3]>; 6]>,
}

impl Source {
    fn texel(&self, l: usize, f: usize, i: i64, j: i64) -> [f64; 3] {
        let n = (self.size >> l).max(1) as i64;
        self.levels[l][f][(j.clamp(0, n - 1) * n + i.clamp(0, n - 1)) as usize]
    }

    /// bilinear within the face at a level (edges clamped: the prefilter's blur hides the seams)
    fn sample(&self, d: V3, l: usize) -> [f64; 3] {
        let (f, s, t) = env_face(d);
        let n = (self.size >> l).max(1) as f64;
        let (u, v) = (s * n - 0.5, t * n - 0.5);
        let (i, j) = (math::floor(u), math::floor(v));
        let (a, b) = (u - i, v - j);
        let (i, j) = (i as i64, j as i64);
        let (t00, t10, t01, t11) = (self.texel(l, f, i, j), self.texel(l, f, i + 1, j), self.texel(l, f, i, j + 1), self.texel(l, f, i + 1, j + 1));
        let mut out = [0.0; 3];
        for k in 0..3 {
            out[k] = (t00[k] * (1.0 - a) + t10[k] * a) * (1.0 - b) + (t01[k] * (1.0 - a) + t11[k] * a) * b;
        }
        out
    }
}


/// what gets uploaded: levels of rgb per face, the harmonics
pub(crate) fn build(d: &EnvironmentData) -> Result<(u32, Vec<[Vec<[f64; 3]>; 6]>, [[f64; 3]; 9])> {
    let (size, source): (u32, Option<Source>) = match d {
        EnvironmentData::Sky(_) => (SKY_SIZE, None),
        EnvironmentData::Faces { size, rgba } => {
            let n = *size;
            if !(4..=1024).contains(&n) || !n.is_power_of_two() {
                return Err(GpuError::Invalid("set_environment: faces of 4 ..= 1024 texels, a power of two"));
            }
            let need = (n * n * 4) as usize;
            if rgba.iter().any(|f| f.len() < need || f[..need].iter().any(|v| !v.is_finite() || *v < 0.0)) {
                return Err(GpuError::Invalid("set_environment: six faces of size x size x 4 finite values, not negative"));
            }
            // the source's own levels by 2 x 2 boxes
            let first: [Vec<[f64; 3]>; 6] = core::array::from_fn(|f| rgba[f][..need].chunks_exact(4).map(|p| [p[0] as f64, p[1] as f64, p[2] as f64]).collect());
            let mut levels = vec![first];
            let mut m = n as usize;
            while m > 1 {
                let h = m / 2;
                let prev = &levels[levels.len() - 1];
                let next: [Vec<[f64; 3]>; 6] = core::array::from_fn(|f| {
                    let p = &prev[f];
                    (0..h * h).map(|k| {
                        let (x, y) = (k % h, k / h);
                        let at = |dx: usize, dy: usize| p[(2 * y + dy) * m + 2 * x + dx];
                        let (a, b, c, e) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
                        [(a[0] + b[0] + c[0] + e[0]) * 0.25, (a[1] + b[1] + c[1] + e[1]) * 0.25, (a[2] + b[2] + c[2] + e[2]) * 0.25]
                    }).collect()
                });
                levels.push(next);
                m = h;
            }
            (n, Some(Source { size: n as usize, levels }))
        }
    };
    let sky = match d {
        EnvironmentData::Sky(s) => Some(*s),
        _ => None,
    };
    let radiance = |dir: V3, lod: f64| -> [f64; 3] {
        match (&sky, &source) {
            (Some(s), _) => s.radiance(dir),
            (_, Some(src)) => {
                let top = (src.levels.len() - 1) as f64;
                let l = lod.clamp(0.0, top);
                let l0 = math::floor(l);
                let a = src.sample(dir, l0 as usize);
                if l0 >= top {
                    return a;
                }
                let b = src.sample(dir, l0 as usize + 1);
                let t = l - l0;
                [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
            }
            _ => [0.0; 3],
        }
    };
    // (down to 4 x 4: the widest lobe needs no fewer texels, and seamless filtering of a 1 x 1 face is all corners)
    let mips = (size.trailing_zeros() as usize).saturating_sub(1).max(1);
    let mut levels: Vec<[Vec<[f64; 3]>; 6]> = Vec::with_capacity(mips);
    for k in 0..mips {
        let n = (size >> k).max(1) as usize;
        let rough = if mips > 1 { k as f64 / (mips - 1) as f64 } else { 0.0 };
        let alpha = (rough * rough).max(0.001);
        let a2 = alpha * alpha;
        let face = |f: usize| -> Vec<[f64; 3]> {
            (0..n * n).map(|t| {
                let (i, j) = (t % n, t / n);
                let nd = env_dir(f, (i as f64 + 0.5) / n as f64, (j as f64 + 0.5) / n as f64);
                if k == 0 {
                    return match &source {
                        Some(src) => src.levels[0][f][t],
                        None => radiance(nd, 0.0),
                    };
                }
                // a basis around the direction
                let up = if nd[1].abs() < 0.99 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
                let tx = math::norm(math::cross(up, nd));
                let ty = math::cross(nd, tx);
                let (mut sum, mut w) = ([0.0; 3], 0.0);
                for s in 0..SAMPLES {
                    let e1 = s as f64 / SAMPLES as f64;
                    let e2 = s.reverse_bits() as f64 / 4_294_967_296.0;
                    let cos_t = math::sqrt((1.0 - e2) / (1.0 + (a2 - 1.0) * e2));
                    let sin_t = math::sqrt(1.0 - cos_t * cos_t);
                    let (sp, cp) = math::sincos(e1 * core::f64::consts::TAU);
                    let h = math::add(math::add(math::mul(tx, sin_t * cp), math::mul(ty, sin_t * sp)), math::mul(nd, cos_t));
                    // N = V = R: l = 2 (n.h) h - n
                    let nh = cos_t;
                    let l = math::sub(math::mul(h, 2.0 * nh), nd);
                    let nl = math::dot(nd, l);
                    if nl <= 0.0 {
                        continue;
                    }
                    // the source's level for the sample's solid angle: pdf = D / 4 (n.h = v.h)
                    let lod = if source.is_some() {
                        let t = nh * nh * (a2 - 1.0) + 1.0;
                        let pdf = a2 / (core::f64::consts::PI * t * t) * 0.25;
                        let omega_s = 1.0 / (SAMPLES as f64 * pdf);
                        let omega_p = 4.0 * core::f64::consts::PI / (6.0 * (size as f64) * (size as f64));
                        (0.5 * math::ln(omega_s / omega_p) / core::f64::consts::LN_2 + 1.0).max(0.0)
                    } else {
                        0.0
                    };
                    let c = radiance(l, lod);
                    for q in 0..3 {
                        sum[q] += c[q] * nl;
                    }
                    w += nl;
                }
                if w > 0.0 { [sum[0] / w, sum[1] / w, sum[2] / w] } else { [0.0; 3] }
            }).collect()
        };
        levels.push(core::array::from_fn(face));
    }
    // the harmonics: over the source's texels (a sky over a 32 x 32 grid per face), by solid angle
    let grid = if source.is_some() { size as usize } else { 32 };
    let mut sh = [[0.0f64; 3]; 9];
    // (the texels' solid angles at their centres, scaled to add up to 4 pi: a coarse grid's midpoint rule
    // would miss a constant by some thousandths)
    let mut total = 0.0;
    for t in 0..grid * grid {
        let (u, v) = (2.0 * ((t % grid) as f64 + 0.5) / grid as f64 - 1.0, 2.0 * ((t / grid) as f64 + 0.5) / grid as f64 - 1.0);
        let r2 = 1.0 + u * u + v * v;
        total += 6.0 * 4.0 / (r2 * math::sqrt(r2)) / (grid * grid) as f64;
    }
    let scale = 4.0 * core::f64::consts::PI / total;
    for f in 0..6 {
        for t in 0..grid * grid {
            let (i, j) = (t % grid, t / grid);
            let (s, tt) = ((i as f64 + 0.5) / grid as f64, (j as f64 + 0.5) / grid as f64);
            let (u, v) = (2.0 * s - 1.0, 2.0 * tt - 1.0);
            let r2 = 1.0 + u * u + v * v;
            let d_omega = 4.0 / (r2 * math::sqrt(r2)) / (grid * grid) as f64 * scale;
            let dir = env_dir(f, s, tt);
            let c = match &source {
                Some(src) => src.levels[0][f][t],
                None => radiance(dir, 0.0),
            };
            let y = sh9(dir);
            for (k, coeff) in sh.iter_mut().enumerate() {
                for q in 0..3 {
                    coeff[q] += c[q] * y[k] * d_omega;
                }
            }
        }
    }
    // irradiance / pi: the cosine lobe's factors pi, 2 pi / 3, pi / 4, over pi
    let band = [1.0, 2.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, 0.25, 0.25, 0.25, 0.25, 0.25];
    for (k, c) in sh.iter_mut().enumerate() {
        for v in c.iter_mut() {
            *v *= band[k];
        }
    }
    Ok((size, levels, sh))
}

/// an f32 as IEEE half (nearest, ties to even; large values to infinity, which radiance never reaches here)
pub fn to_half(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let e = ((b >> 23) & 0xFF) as i32 - 127 + 15;
    let m = b & 0x7F_FFFF;
    if e >= 31 {
        return sign | 0x7C00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = m | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let rest = m & ((1 << shift) - 1);
        let mut r = m >> shift;
        if rest > half || (rest == half && r & 1 == 1) {
            r += 1;
        }
        return sign | r as u16;
    }
    let mut r = ((e as u32) << 10) | (m >> 13);
    let rest = m & 0x1FFF;
    if rest > 0x1000 || (rest == 0x1000 && r & 1 == 1) {
        r += 1;
    }
    sign | r as u16
}

/// a half as f32
pub fn from_half(h: u16) -> f32 {
    let (s, e, m) = ((h >> 15) as u32, ((h >> 10) & 31) as u32, (h & 1023) as u32);
    let v = if e == 0 {
        m as f32 * (1.0 / 16_777_216.0)
    } else if e == 31 {
        f32::INFINITY
    } else {
        f32::from_bits(((e + 112) << 23) | (m << 13))
    };
    if s == 1 { -v } else { v }
}

/// the cube texture of the levels (Rgba16Float, left in Layout::Sampled) and what it holds as f32
pub(crate) fn upload(dev: &Device, size: u32, levels: &[[Vec<[f64; 3]>; 6]]) -> Result<(Texture, Vec<Vec<[f32; 4]>>)> {
    let mips = levels.len() as u32;
    let tex = dev.create_texture(&TextureDesc { width: size, height: size, depth: 1, layers: 6, mips, format: Format::Rgba16Float,
                                                usage: TextureUsage::SAMPLED | TextureUsage::COPY_DST, cube: true })?;
    let mut held = Vec::with_capacity(levels.len());
    for (k, faces) in levels.iter().enumerate() {
        let n = (size >> k).max(1);
        let mut words: Vec<u16> = Vec::with_capacity((n * n * 4 * 6) as usize);
        let mut f32s = Vec::with_capacity((n * n * 6) as usize);
        for face in faces {
            for c in face {
                let h = [to_half(c[0] as f32), to_half(c[1] as f32), to_half(c[2] as f32), to_half(1.0)];
                words.extend_from_slice(&h);
                f32s.push([from_half(h[0]), from_half(h[1]), from_half(h[2]), 1.0]);
            }
        }
        let region = TextureRegion { texture: &tex, mip: k as u32, origin: [0; 3], size: [n, n, 1], base_layer: 0, layers: 6 };
        dev.queue().write_texture(&region, bytes_of(&words), DataLayout::default(), Layout::Undefined, Layout::Sampled)?;
        held.push(f32s);
    }
    Ok((tex, held))
}

/// a white 1 x 1 cube (what group 2 binds before any environment)
pub(crate) fn placeholder(dev: &Device) -> Result<Texture> {
    let white: [Vec<[f64; 3]>; 6] = core::array::from_fn(|_| vec![[1.0; 3]]);
    Ok(upload(dev, 1, &[white])?.0)
}
