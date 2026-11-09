// cpu -- the reference the tests judge the engine against: a ray caster and a visibility test in f64,
// written from the definitions and sharing no code with lib/render3d (its own square root, its own
// rotation by quaternion products, the planes as distances in camera space).  A pixel of the rasteriser
// is covered when its centre lies in the triangle, so a ray through the centre sees what the GPU should;
// the GPU snaps vertices to 1/256 of a pixel, so a pixel whose rays around the centre disagree counts as
// an edge and may show the colour, under some light, of what those rays see (the GPU's normal there lies a
// little beyond the triangle's end).  The snapping can also turn a triangle seen almost edge on towards
// the eye, which for the rays stays a back face: a sliver on a silhouette.  Those are counted, and may only
// be few and show the colour of an object a neighbouring pixel's ray sees.
//
// Textures (G5b): levels made by its own box filter, sRGB by the definition (its own exp and ln), sampled
// as Vulkan defines nearest, bilinear and trilinear filtering with the three wrap modes, the level from
// the derivatives the GPU's fine derivatives take: uv where the rays of the pixel's 2 x 2 quad meet the
// triangle's plane.  Where the GPU must match (nearest texels, level 0, a nearest level) it is to within 1;
// trilinear filtering has a stated tolerance.  A mirrored instance's triangles are turned round (its
// outside stays in front), its normals go through the signed scale.
//
// Shading (G5c): glTF 2.0 Appendix B in f64, written here from the specification (Lambert, GGX, the separable
// Smith visibility, Schlick's Fresnel), the uniform ambient light through the split-sum with Karis' fit,
// Khronos PBR Neutral; normal maps on the triangle's exact tangent frame (dp/du from its edges and uv, the
// bitangent towards decreasing v), which is what the GPU's derivatives give.  At an edge, a single-coloured
// instance may show any colour between those of its triangle's vertex normals and the hit's (the GPU's
// normal there lies a little beyond the triangle).

use alloc::format;
use alloc::vec::Vec;

use crate::common::{Geo, Spec};

pub type V = [f64; 3];

pub fn sqrt(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut r = if x > 1.0 { x } else { 1.0 };
    for _ in 0..80 {
        let n = 0.5 * (r + x / r);
        if n == r {
            break;
        }
        r = n;
    }
    r
}

/// sine and cosine of any angle: reduced, then their series
pub fn sincos(a: f64) -> (f64, f64) {
    let turns = a / core::f64::consts::TAU;
    let whole = turns as i64 as f64;
    let x = (turns - whole) * core::f64::consts::TAU;
    let (x2, mut s, mut c, mut ts, mut tc) = (x * x, 0.0, 0.0, x, 1.0);
    for n in 0..30 {
        let n = n as f64;
        s += ts;
        c += tc;
        ts *= -x2 / ((2.0 * n + 2.0) * (2.0 * n + 3.0));
        tc *= -x2 / ((2.0 * n + 1.0) * (2.0 * n + 2.0));
    }
    (s, c)
}

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mul(a: V, k: f64) -> V {
    [a[0] * k, a[1] * k, a[2] * k]
}

pub fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn norm(a: V) -> V {
    let l = sqrt(dot(a, a));
    if l > 0.0 { mul(a, 1.0 / l) } else { a }
}

/// v turned by the unit quaternion q: v + 2 u x (u x v + w v)
fn turn(q: [f64; 4], v: V) -> V {
    let u = [q[0], q[1], q[2]];
    add(v, mul(cross(u, add(cross(u, v), mul(v, q[3]))), 2.0))
}

fn unit_quat(q: [f32; 4]) -> [f64; 4] {
    let v = [q[0] as f64, q[1] as f64, q[2] as f64, q[3] as f64];
    let l = sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2] + v[3] * v[3]);
    [v[0] / l, v[1] / l, v[2] / l, v[3] / l]
}

/// the bounding sphere of a mesh's sphere under a transform
pub fn world_sphere(center: V, radius: f64, pos: V, rot: [f32; 4], scale: [f32; 3]) -> (V, f64) {
    let (sx, sy, sz) = (scale[0] as f64, scale[1] as f64, scale[2] as f64);
    let c = add(pos, turn(unit_quat(rot), [center[0] * sx, center[1] * sy, center[2] * sz]));
    (c, radius * sx.abs().max(sy.abs()).max(sz.abs()))
}

/// what the test of a sphere against a frustum says
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    /// not outside any plane: must be in the list
    Yes,
    /// within a hair of a plane
    Doubt,
    /// well outside a plane: must not be in the list
    No,
}

/// a perspective camera as the tests see it
#[derive(Clone, Copy, Debug)]
pub struct RefCamera {
    pub eye: V,
    pub f: V,
    pub r: V,
    pub u: V,
    pub tx: f64,
    pub ty: f64,
    pub near: f64,
    pub far: f64,
}

impl RefCamera {
    pub fn new(eye: V, forward: V, up: V, fov_y: f64, aspect: f64, near: f64, far: f64) -> RefCamera {
        let f = norm(forward);
        let r = norm(cross(f, up));
        let u = cross(r, f);
        let (s, c) = sincos(fov_y * 0.5);
        RefCamera { eye, f, r, u, tx: s / c * aspect, ty: s / c, near, far }
    }

    /// the distance of a point to each face of the frustum, in the frustum's inside direction
    pub fn distances(&self, p: V) -> [f64; 6] {
        let d = sub(p, self.eye);
        let (x, y, z) = (dot(d, self.r), dot(d, self.u), dot(d, self.f));
        let lx = sqrt(1.0 + self.tx * self.tx);
        let ly = sqrt(1.0 + self.ty * self.ty);
        [(x + z * self.tx) / lx, (z * self.tx - x) / lx, (y + z * self.ty) / ly, (z * self.ty - y) / ly, z - self.near,
         if self.far.is_finite() { self.far - z } else { f64::INFINITY }]
    }

    /// origin: what the GPU's f32 are relative to
    pub fn sphere_test(&self, c: V, radius: f64, origin: V) -> Seen {
        // the engine widens spheres and planes by what f32 rounds; well beyond that is a real "outside"
        let (rc, re) = (sub(c, origin), sub(self.eye, origin));
        let big = rc[0].abs().max(rc[1].abs()).max(rc[2].abs()).max(re[0].abs()).max(re[1].abs()).max(re[2].abs());
        let slack = radius * 1e-5 + big * 1e-5;
        let mut seen = Seen::Yes;
        for d in self.distances(c) {
            if d < -radius - slack {
                return Seen::No;
            }
            if d < -radius {
                seen = Seen::Doubt;
            }
        }
        seen
    }

    /// the ray of a point of the image (pixels; y down): its parameter is the distance along forward
    pub fn ray(&self, x: f64, y: f64, w: u32, h: u32) -> V {
        let nx = 2.0 * x / w as f64 - 1.0;
        let ny = 1.0 - 2.0 * y / h as f64;
        add(add(self.f, mul(self.r, nx * self.tx)), mul(self.u, ny * self.ty))
    }

    /// reverse Z: the depth buffer's value at distance z along forward
    pub fn depth(&self, z: f64) -> f64 {
        if self.far.is_finite() { self.near * (self.far - z) / (z * (self.far - self.near)) } else { self.near / z }
    }
}


// ---------------------------------------------------------------- textures (G5b)

/// e^x: halved until small, its series, squared back (another way than lib/render3d's)
pub fn exp(x: f64) -> f64 {
    let mut k = 0;
    let mut y = x;
    while y.abs() > 0.125 {
        y *= 0.5;
        k += 1;
    }
    let (mut term, mut sum) = (1.0, 1.0);
    for n in 1..20 {
        term *= y / n as f64;
        sum += term;
    }
    for _ in 0..k {
        sum *= sum;
    }
    sum
}

/// ln x by Halley's iteration on e^y = x, from the binary exponent
pub fn ln(x: f64) -> f64 {
    if x <= 0.0 {
        return f64::NEG_INFINITY;
    }
    let e = ((x.to_bits() >> 52) & 0x7FF) as i64 - 1023;
    let mut y = e as f64 * core::f64::consts::LN_2;
    for _ in 0..60 {
        let ey = exp(y);
        let n = y + 2.0 * (x - ey) / (x + ey);
        if n == y {
            break;
        }
        y = n;
    }
    y
}

pub fn log2(x: f64) -> f64 {
    ln(x) / core::f64::consts::LN_2
}

pub fn floor(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

/// an sRGB code (0 ..= 1) in linear light, by the definition
pub fn srgb_decode(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { exp(2.4 * ln((c + 0.055) / 1.055)) }
}

/// linear light as an 8-bit sRGB code, rounded
pub fn srgb_encode(v: f64) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let c = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * exp(ln(v) / 2.4) - 0.055 };
    (c * 255.0 + 0.5) as u8
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RWrap {
    Repeat,
    Clamp,
    Mirror,
}

/// how the reference samples: as Vulkan defines the filters (mip: 0 none, 1 nearest, 2 linear)
#[derive(Clone, Copy, Debug)]
pub struct RefSampling {
    pub mag_nearest: bool,
    pub min_nearest: bool,
    pub mip: u32,
    pub wrap: [RWrap; 2],
}

/// a texture as the GPU holds it: a power of two square with 8-bit levels made by a box filter of its own
/// (sRGB averaged in linear light by the definition, rounded to codes again)
pub struct RefTexture {
    pub size: u32,
    pub levels: Vec<Vec<[u8; 4]>>,
    lin: [f64; 256],
}

impl RefTexture {
    pub fn new(size: u32, rgba: &[u8], srgb: bool) -> RefTexture {
        let mut lin = [0.0; 256];
        for (k, l) in lin.iter_mut().enumerate() {
            *l = if srgb { srgb_decode(k as f64 / 255.0) } else { k as f64 / 255.0 };
        }
        let first: Vec<[u8; 4]> = rgba.chunks_exact(4).take((size * size) as usize).map(|p| [p[0], p[1], p[2], p[3]]).collect();
        let mut levels = alloc::vec![first];
        let mut n = size as usize;
        while n > 1 {
            let m = n / 2;
            let prev = &levels[levels.len() - 1];
            let mut next = Vec::with_capacity(m * m);
            for y in 0..m {
                for x in 0..m {
                    let four = [prev[2 * y * n + 2 * x], prev[2 * y * n + 2 * x + 1], prev[(2 * y + 1) * n + 2 * x], prev[(2 * y + 1) * n + 2 * x + 1]];
                    let mut px = [0u8; 4];
                    for k in 0..4 {
                        if srgb && k < 3 {
                            px[k] = srgb_encode(four.iter().map(|p| lin[p[k] as usize]).sum::<f64>() / 4.0);
                        } else {
                            px[k] = ((four.iter().map(|p| p[k] as u32).sum::<u32>() as f64 / 4.0) + 0.5) as u8;
                        }
                    }
                    next.push(px);
                }
            }
            levels.push(next);
            n = m;
        }
        RefTexture { size, levels, lin }
    }

    /// texel (i, j) of a level, wrapped, in linear light
    fn texel(&self, level: usize, i: i64, j: i64, wrap: [RWrap; 2]) -> [f64; 4] {
        let n = (self.size as i64 >> level).max(1);
        let w = |i: i64, m: RWrap| -> i64 {
            match m {
                RWrap::Repeat => i.rem_euclid(n),
                RWrap::Clamp => i.clamp(0, n - 1),
                RWrap::Mirror => {
                    let k = i.rem_euclid(2 * n);
                    if k < n { k } else { 2 * n - 1 - k }
                }
            }
        };
        let p = self.levels[level][(w(j, wrap[1]) * n + w(i, wrap[0])) as usize];
        [self.lin[p[0] as usize], self.lin[p[1] as usize], self.lin[p[2] as usize], p[3] as f64 / 255.0]
    }

    fn at_level(&self, level: usize, uv: [f64; 2], nearest: bool, wrap: [RWrap; 2]) -> [f64; 4] {
        let n = (self.size >> level).max(1) as f64;
        if nearest {
            return self.texel(level, floor(uv[0] * n) as i64, floor(uv[1] * n) as i64, wrap);
        }
        let (x, y) = (uv[0] * n - 0.5, uv[1] * n - 0.5);
        let (i, j) = (floor(x), floor(y));
        let (a, b) = (x - i, y - j);
        let (i, j) = (i as i64, j as i64);
        let (t00, t10, t01, t11) = (self.texel(level, i, j, wrap), self.texel(level, i + 1, j, wrap), self.texel(level, i, j + 1, wrap), self.texel(level, i + 1, j + 1, wrap));
        let mut out = [0.0; 4];
        for k in 0..4 {
            out[k] = (t00[k] * (1.0 - a) + t10[k] * a) * (1.0 - b) + (t01[k] * (1.0 - a) + t11[k] * a) * b;
        }
        out
    }

    /// the colour at uv with the derivatives of uv along x and y (Vulkan's rho and lambda, no anisotropy)
    pub fn sample(&self, s: &RefSampling, uv: [f64; 2], dx: [f64; 2], dy: [f64; 2]) -> [f64; 4] {
        let n = self.size as f64;
        let rho = (sqrt(dx[0] * dx[0] + dx[1] * dx[1]) * n).max(sqrt(dy[0] * dy[0] + dy[1] * dy[1]) * n);
        let lod = if rho > 0.0 { log2(rho) } else { f64::NEG_INFINITY };
        let top = (self.levels.len() - 1) as f64;
        if lod <= 0.0 {
            return self.at_level(0, uv, s.mag_nearest, s.wrap);
        }
        match s.mip {
            0 => self.at_level(0, uv, s.min_nearest, s.wrap),
            1 => self.at_level(floor(lod + 0.5).min(top) as usize, uv, s.min_nearest, s.wrap),
            _ => {
                let d = lod.min(top);
                let l0 = floor(d);
                let a = self.at_level(l0 as usize, uv, s.min_nearest, s.wrap);
                if l0 >= top {
                    return a;
                }
                let b = self.at_level(l0 as usize + 1, uv, s.min_nearest, s.wrap);
                let f = d - l0;
                [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f, a[3] + (b[3] - a[3]) * f]
            }
        }
    }
}

// ---------------------------------------------------------------- shading (G5c)

const PI: f64 = core::f64::consts::PI;
/// the least alpha the engine's BRDF takes (draw.wgsl ALPHA_MIN)
const ALPHA_MIN: f64 = 0.001;

/// glTF 2.0 Appendix B for one light from l, times n.l: (1 - F) c_diff / pi + F D V, V the separable Smith
/// term G / (4 |n.l| |n.v|), a2 = alpha squared
pub fn brdf(n: V, v: V, l: V, c_diff: [f64; 3], f0: [f64; 3], a2: f64) -> [f64; 3] {
    let nl = dot(n, l);
    if !(nl > 0.0) {
        return [0.0; 3];
    }
    let nv = dot(n, v).clamp(0.0, 1.0);
    let h = norm(add(l, v));
    let nh = dot(n, h).clamp(0.0, 1.0);
    let vh = dot(v, h).clamp(0.0, 1.0);
    let x = 1.0 - vh;
    let x5 = x * x * x * x * x;
    let t = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * t * t);
    let vis = 1.0 / ((nl + sqrt(a2 + (1.0 - a2) * nl * nl)) * (nv + sqrt(a2 + (1.0 - a2) * nv * nv)));
    let mut out = [0.0; 3];
    for k in 0..3 {
        let f = f0[k] + (1.0 - f0[k]) * x5;
        out[k] = ((1.0 - f) * c_diff[k] / PI + f * d * vis) * nl;
    }
    out
}

/// 2^x by the reference's own exp
pub fn exp2(x: f64) -> f64 {
    exp(x * core::f64::consts::LN_2)
}

/// Karis' analytic fit of the split-sum's second sum (measured against the integral; the engine has a table)
pub fn karis_ab(roughness: f64, nv: f64) -> (f64, f64) {
    let c0 = [-1.0, -0.0275, -0.572, 0.022];
    let c1 = [1.0, 0.0425, 1.04, -0.04];
    let r = [roughness * c0[0] + c1[0], roughness * c0[1] + c1[1], roughness * c0[2] + c1[2], roughness * c0[3] + c1[3]];
    let a004 = (r[0] * r[0]).min(exp2(-9.28 * nv)) * r[0] + r[1];
    (-1.04 * a004 + r[2], 1.04 * a004 + r[3])
}

/// the split-sum's second sum (scale, bias of f0): the engine's table (render3d::DFG, data like a texture's
/// texels; checked against the integral by the pbr test), read bilinearly
pub fn env_ab(roughness: f64, nv: f64) -> (f64, f64) {
    let n = render3d::DFG_N;
    let last = (n - 1) as f64;
    let (x, y) = (nv.clamp(0.0, 1.0) * last, roughness.clamp(0.0, 1.0) * last);
    let (i, j) = ((x as usize).min(n - 2), (y as usize).min(n - 2));
    let (fx, fy) = (x - i as f64, y - j as f64);
    let at = |i: usize, j: usize, c: usize| render3d::DFG[j * n + i][c] as f64;
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let get = |c: usize| lerp(lerp(at(i, j, c), at(i + 1, j, c), fx), lerp(at(i, j + 1, c), at(i + 1, j + 1, c), fx), fy);
    (get(0), get(1))
}

/// Khronos PBR Neutral, from its specification
pub fn neutral(c: [f64; 3]) -> [f64; 3] {
    let start = 0.8 - 0.04;
    let desaturation = 0.15;
    let x = c[0].min(c[1]).min(c[2]);
    let offset = if x < 0.08 { x - 6.25 * x * x } else { 0.04 };
    let mut col = [c[0] - offset, c[1] - offset, c[2] - offset];
    let peak = col[0].max(col[1]).max(col[2]);
    if peak < start {
        return col;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    for v in &mut col {
        *v *= new_peak / peak;
    }
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    [col[0] + (new_peak - col[0]) * g, col[1] + (new_peak - col[1]) * g, col[2] + (new_peak - col[2]) * g]
}

/// what a material's inputs are at a point: linear base colour (alpha), metallic, roughness, the shading
/// normal, occlusion, emission
#[derive(Clone, Copy, Debug)]
pub struct Surface {
    pub base: [f64; 4],
    pub metallic: f64,
    pub roughness: f64,
    pub n: V,
    pub ao: f64,
    pub emissive: [f64; 3],
}

// ---------------------------------------------------------------- the ray caster

#[derive(Clone, Copy)]
struct Tri {
    v0: V,
    e1: V,
    e2: V,
    n: [V; 3],
    uv: [[f64; 2]; 3],
}

/// what an instance looks like: glTF's metallic-roughness material, its textures as indices into
/// RefScene::textures (each as the GPU holds it: sRGB for base colour and emission, linear for the others);
/// with a cutoff, glTF's MASK
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub factor: [f64; 4],
    pub texture: Option<(usize, RefSampling)>,
    pub cutoff: Option<f64>,
    pub metallic: f64,
    pub roughness: f64,
    pub mr_texture: Option<(usize, RefSampling)>,
    pub normal_texture: Option<(usize, RefSampling)>,
    pub normal_scale: f64,
    pub occlusion_texture: Option<(usize, RefSampling)>,
    pub occlusion: f64,
    pub emissive: [f64; 3],
    pub emissive_texture: Option<(usize, RefSampling)>,
}

impl Look {
    /// render3d's Material::color: dull (metallic 0, roughness 1)
    pub const DULL: Look = Look { factor: [1.0; 4], texture: None, cutoff: None, metallic: 0.0, roughness: 1.0, mr_texture: None, normal_texture: None,
                                  normal_scale: 1.0, occlusion_texture: None, occlusion: 1.0, emissive: [0.0; 3], emissive_texture: None };

    pub fn color(c: [f32; 3]) -> Look {
        Look { factor: [c[0] as f64, c[1] as f64, c[2] as f64, 1.0], ..Look::DULL }
    }

    fn textured(&self) -> bool {
        self.texture.is_some() || self.mr_texture.is_some() || self.normal_texture.is_some() || self.occlusion_texture.is_some() || self.emissive_texture.is_some()
    }
}

struct RefInstance {
    look: Look,
    center: V,
    r2: f64,
    first: usize,
    last: usize,
}

/// a scene for rays
pub struct RefScene {
    tris: Vec<Tri>,
    inst: Vec<RefInstance>,
    pub textures: Vec<RefTexture>,
    pub sun_dir: V,
    /// illuminance (lux) and the uniform ambient radiance, as render3d::Sun
    pub sun: [f64; 3],
    pub ambient: [f64; 3],
    pub exposure: f64,
    /// Khronos PBR Neutral (else none)
    pub tonemap: bool,
    /// (G5c) the sun casts shadows: exact shadow rays (both sides of every triangle block, MASK's holes not)
    pub shadows: bool,
    /// (G5c) point and spot lights, no shadows
    pub lights: Vec<RefLight>,
}

/// (G5c) a point or spot light as KHR_lights_punctual defines it
#[derive(Clone, Copy, Debug)]
pub struct RefLight {
    pub pos: V,
    /// colour times intensity (candela)
    pub color: [f64; 3],
    pub range: Option<f64>,
    /// a spot's axis (unit) and cone angles
    pub spot: Option<(V, f64, f64)>,
}

impl RefLight {
    /// what reaches a point p (light per unit of BRDF times n.l) and from where (unit vector towards it)
    pub fn at(&self, p: V) -> Option<(f64, V)> {
        let d = sub(self.pos, p);
        let dist2 = dot(d, d);
        if dist2 <= 0.0 {
            return None;
        }
        let mut att = 1.0 / dist2;
        if let Some(r) = self.range {
            let q = (dist2 / (r * r)) * (dist2 / (r * r));
            att *= (1.0 - q).clamp(0.0, 1.0);
        }
        let l = mul(d, 1.0 / sqrt(dist2));
        if let Some((axis, inner, outer)) = self.spot {
            let (ci, co) = (sincos(inner).1, sincos(outer).1);
            let scale = 1.0 / (ci - co).max(0.001);
            let t = (dot(axis, mul(l, -1.0)) * scale - co * scale).clamp(0.0, 1.0);
            att *= t * t;
        }
        Some((att, l))
    }
}

/// what a ray found
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub z: f64,
    pub rgb: [u8; 3],
    pub inst: usize,
    /// the triangle and the barycentric coordinates of the hit
    pub tri: usize,
    pub u: f64,
    pub v: f64,
    /// the light leaving the point towards the eye, times the exposure, before the tone mapper
    pub linear: [f64; 3],
    /// (G5c) of it, the sun's part unshadowed, and how much of the sun reaches the point (0 or 1)
    pub sun: [f64; 3],
    pub vis: f64,
}

/// the rays through the centres of the pixel's 2 x 2 quad that the GPU's fine derivatives take: along x
/// (left and right pixel of its row), along y (upper and lower of its column)
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    pub x0: V,
    pub x1: V,
    pub y0: V,
    pub y1: V,
}

impl RefScene {
    pub fn new(sun_dir: V, sun: [f64; 3], ambient: [f64; 3]) -> RefScene {
        RefScene { tris: Vec::new(), inst: Vec::new(), textures: Vec::new(), sun_dir: norm(sun_dir), sun, ambient, exposure: 1.0, tonemap: true, shadows: false, lights: Vec::new() }
    }

    /// an instance of one colour: its triangles in the world
    pub fn add(&mut self, geo: &Geo, s: &Spec, color: [f32; 3]) {
        self.add_look(geo, s, Look::color(color));
    }

    /// an instance under a column-major 4 x 4 matrix (glTF's world matrices): positions through it,
    /// normals through the inverse transpose of its 3 x 3 part; a negative determinant turns the
    /// triangles round
    pub fn add_matrix(&mut self, geo: &Geo, m: &[f64; 16], look: Look) {
        let a = |r: usize, c: usize| m[c * 4 + r];
        let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1)) - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0)) + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
        // the inverse transpose: cofactors over the determinant
        let cof = |r: usize, c: usize| {
            let (r1, r2, c1, c2) = ((r + 1) % 3, (r + 2) % 3, (c + 1) % 3, (c + 2) % 3);
            (a(r1, c1) * a(r2, c2) - a(r1, c2) * a(r2, c1)) / det
        };
        let world = |p: [f32; 3]| {
            let p = [p[0] as f64, p[1] as f64, p[2] as f64];
            [a(0, 0) * p[0] + a(0, 1) * p[1] + a(0, 2) * p[2] + a(0, 3), a(1, 0) * p[0] + a(1, 1) * p[1] + a(1, 2) * p[2] + a(1, 3),
             a(2, 0) * p[0] + a(2, 1) * p[1] + a(2, 2) * p[2] + a(2, 3)]
        };
        let normal = |n: [f32; 3]| {
            let n = [n[0] as f64, n[1] as f64, n[2] as f64];
            [cof(0, 0) * n[0] + cof(0, 1) * n[1] + cof(0, 2) * n[2], cof(1, 0) * n[0] + cof(1, 1) * n[1] + cof(1, 2) * n[2],
             cof(2, 0) * n[0] + cof(2, 1) * n[1] + cof(2, 2) * n[2]]
        };
        self.add_with(geo, &world, &normal, det < 0.0, look);
    }

    /// an instance with a look.  A mirrored transform (the product of the scales negative) keeps the
    /// mesh's outside in front: its triangles are turned round
    pub fn add_look(&mut self, geo: &Geo, s: &Spec, look: Look) {
        let q = unit_quat(s.rot);
        let scale = [s.scale[0] as f64, s.scale[1] as f64, s.scale[2] as f64];
        let mirrored = scale[0] * scale[1] * scale[2] < 0.0;
        let world = |p: [f32; 3]| add(s.pos, turn(q, [p[0] as f64 * scale[0], p[1] as f64 * scale[1], p[2] as f64 * scale[2]]));
        // the inverse transpose of rotation * scale (signs included)
        let normal = |n: [f32; 3]| turn(q, [n[0] as f64 / scale[0], n[1] as f64 / scale[1], n[2] as f64 / scale[2]]);
        self.add_with(geo, &world, &normal, mirrored, look);
    }

    fn add_with(&mut self, geo: &Geo, world: &dyn Fn([f32; 3]) -> V, normal: &dyn Fn([f32; 3]) -> V, mirrored: bool, look: Look) {
        let first = self.tris.len();
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in &geo.pos {
            let w = world(*p);
            for k in 0..3 {
                lo[k] = lo[k].min(w[k]);
                hi[k] = hi[k].max(w[k]);
            }
        }
        let center = mul(add(lo, hi), 0.5);
        let half = mul(sub(hi, lo), 0.5);
        let uv = |i: usize| [geo.uv[i][0] as f64, geo.uv[i][1] as f64];
        for t in geo.idx.chunks_exact(3) {
            let (a, b, c) = if mirrored { (t[0] as usize, t[2] as usize, t[1] as usize) } else { (t[0] as usize, t[1] as usize, t[2] as usize) };
            let (v0, v1, v2) = (world(geo.pos[a]), world(geo.pos[b]), world(geo.pos[c]));
            self.tris.push(Tri { v0, e1: sub(v1, v0), e2: sub(v2, v0), n: [normal(geo.nrm[a]), normal(geo.nrm[b]), normal(geo.nrm[c])], uv: [uv(a), uv(b), uv(c)] });
        }
        self.inst.push(RefInstance { look, center, r2: dot(half, half) * 1.0001, first, last: self.tris.len() });
    }

    /// uv where a ray meets the plane of a triangle (inside or not), by its barycentric coordinates
    fn uv_on_plane(t: &Tri, eye: V, d: V) -> [f64; 2] {
        let p = cross(d, t.e2);
        let det = dot(t.e1, p);
        let o = sub(eye, t.v0);
        let u = dot(o, p) / det;
        let v = dot(d, cross(o, t.e1)) / det;
        let w = 1.0 - u - v;
        [t.uv[0][0] * w + t.uv[1][0] * u + t.uv[2][0] * v, t.uv[0][1] * w + t.uv[1][1] * u + t.uv[2][1] * v]
    }

    /// the look's colour (linear rgba) at a point of a triangle
    fn base(&self, look: &Look, t: &Tri, u: f64, v: f64, eye: V, quad: Option<&Quad>) -> [f64; 4] {
        let f = look.factor;
        let Some(c) = self.tex_at(look.texture, t, u, v, eye, quad) else { return f };
        [f[0] * c[0], f[1] * c[1], f[2] * c[2], f[3] * c[3]]
    }

    /// a texture of a look at a point of a triangle (None: the look has none there)
    fn tex_at(&self, which: Option<(usize, RefSampling)>, t: &Tri, u: f64, v: f64, eye: V, quad: Option<&Quad>) -> Option<[f64; 4]> {
        let (tex, s) = which?;
        let w = 1.0 - u - v;
        let uv = [t.uv[0][0] * w + t.uv[1][0] * u + t.uv[2][0] * v, t.uv[0][1] * w + t.uv[1][1] * u + t.uv[2][1] * v];
        let (dx, dy) = match quad {
            Some(q) => {
                let (a, b, c, e) = (RefScene::uv_on_plane(t, eye, q.x0), RefScene::uv_on_plane(t, eye, q.x1), RefScene::uv_on_plane(t, eye, q.y0), RefScene::uv_on_plane(t, eye, q.y1));
                ([b[0] - a[0], b[1] - a[1]], [e[0] - c[0], e[1] - c[1]])
            }
            None => ([0.0; 2], [0.0; 2]),
        };
        Some(self.textures[tex].sample(&s, uv, dx, dy))
    }

    /// the normal map's normal on the triangle's tangent frame: the tangent dp/du from the edges and their
    /// uv, made orthogonal to ng, the bitangent cross(ng, tangent) turned towards decreasing v
    fn bumped(t: &Tri, ng: V, tn: [f64; 3]) -> V {
        let (du1, dv1) = (t.uv[1][0] - t.uv[0][0], t.uv[1][1] - t.uv[0][1]);
        let (du2, dv2) = (t.uv[2][0] - t.uv[0][0], t.uv[2][1] - t.uv[0][1]);
        let det = du1 * dv2 - du2 * dv1;
        if det == 0.0 {
            return ng;
        }
        let dpdu = mul(sub(mul(t.e1, dv2), mul(t.e2, dv1)), 1.0 / det);
        let dpdv = mul(sub(mul(t.e2, du1), mul(t.e1, du2)), 1.0 / det);
        let tt = sub(dpdu, mul(ng, dot(ng, dpdu)));
        if !(dot(tt, tt) > 0.0) {
            return ng;
        }
        let tg = norm(tt);
        let mut b = cross(ng, tg);
        if dot(b, dpdv) > 0.0 {
            b = mul(b, -1.0);
        }
        norm(add(add(mul(tg, tn[0]), mul(b, tn[1])), mul(ng, tn[2])))
    }

    /// the material's inputs at a point; normal: the interpolated normal (or one given in its place)
    fn surface(&self, look: &Look, t: &Tri, u: f64, v: f64, eye: V, quad: Option<&Quad>, normal: Option<V>) -> Surface {
        let base = self.base(look, t, u, v, eye, quad);
        let (mut metallic, mut roughness) = (look.metallic, look.roughness);
        if let Some(mr) = self.tex_at(look.mr_texture, t, u, v, eye, quad) {
            roughness *= mr[1];
            metallic *= mr[2];
        }
        let w = 1.0 - u - v;
        let ng = norm(normal.unwrap_or_else(|| add(add(mul(t.n[0], w), mul(t.n[1], u)), mul(t.n[2], v))));
        let mut n = ng;
        if let Some(c) = self.tex_at(look.normal_texture, t, u, v, eye, quad) {
            let k = look.normal_scale;
            n = RefScene::bumped(t, ng, [(c[0] * 2.0 - 1.0) * k, (c[1] * 2.0 - 1.0) * k, c[2] * 2.0 - 1.0]);
        }
        let ao = self.tex_at(look.occlusion_texture, t, u, v, eye, quad).map_or(1.0, |c| 1.0 + look.occlusion * (c[0] - 1.0));
        let mut emissive = look.emissive;
        if let Some(c) = self.tex_at(look.emissive_texture, t, u, v, eye, quad) {
            emissive = [emissive[0] * c[0], emissive[1] * c[1], emissive[2] * c[2]];
        }
        Surface { base, metallic: metallic.clamp(0.0, 1.0), roughness: roughness.clamp(0.0, 1.0), n, ao, emissive }
    }

    /// the light a surface at p sends towards the eye, times the exposure (before the tone mapper), the sun
    /// shadowed by vis; and the sun's part unshadowed
    pub fn light(&self, sf: &Surface, p: V, eye: V, vis: f64) -> ([f64; 3], [f64; 3]) {
        let v = norm(sub(eye, p));
        let m = sf.metallic;
        let c_diff = [sf.base[0] * (1.0 - m), sf.base[1] * (1.0 - m), sf.base[2] * (1.0 - m)];
        let f0 = [0.04 + (sf.base[0] - 0.04) * m, 0.04 + (sf.base[1] - 0.04) * m, 0.04 + (sf.base[2] - 0.04) * m];
        let a = (sf.roughness * sf.roughness).max(ALPHA_MIN);
        let sun = brdf(sf.n, v, self.sun_dir, c_diff, f0, a * a);
        let (sa, sb) = env_ab(sf.roughness, dot(sf.n, v).clamp(0.0, 1.0));
        let mut punctual = [0.0; 3];
        for light in &self.lights {
            if let Some((att, l)) = light.at(p) {
                let f = brdf(sf.n, v, l, c_diff, f0, a * a);
                for k in 0..3 {
                    punctual[k] += f[k] * light.color[k] * att;
                }
            }
        }
        let (mut out, mut sun_part) = ([0.0; 3], [0.0; 3]);
        for k in 0..3 {
            let spec = f0[k] * sa + sb;
            sun_part[k] = sun[k] * self.sun[k] * self.exposure;
            out[k] = (self.ambient[k] * (spec + c_diff[k] * (1.0 - spec)) * sf.ao + sf.emissive[k] + punctual[k]) * self.exposure + sun_part[k] * vis;
        }
        (out, sun_part)
    }

    /// (G5c) something lies between p and the sun: any triangle, either side, beyond a hair of p; MASK's
    /// holes (base alpha at level 0) let it through
    pub fn occluded(&self, p: V, dir: V) -> bool {
        let dd = dot(dir, dir);
        for inst in &self.inst {
            let oc = sub(inst.center, p);
            let along = dot(oc, dir);
            if along < -sqrt(inst.r2) || dot(oc, oc) - along * along / dd > inst.r2 {
                continue;
            }
            for t in &self.tris[inst.first..inst.last] {
                let q = cross(dir, t.e2);
                let det = dot(t.e1, q);
                if det == 0.0 {
                    continue;
                }
                let o = sub(p, t.v0);
                let u = dot(o, q) / det;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let r = cross(o, t.e1);
                let v = dot(dir, r) / det;
                if v < 0.0 || u + v > 1.0 {
                    continue;
                }
                let z = dot(t.e2, r) / det;
                if z <= 1e-9 {
                    continue;
                }
                if let Some(cut) = inst.look.cutoff {
                    if self.base(&inst.look, t, u, v, p, None)[3] < cut {
                        continue;
                    }
                }
                return true;
            }
        }
        false
    }

    /// (G5c) how much of the sun reaches a point of a triangle with geometric normal ng (shadow rays from a
    /// hair above the surface)
    fn sun_vis(&self, p: V, ng: V) -> f64 {
        if !self.shadows || dot(ng, self.sun_dir) <= 0.0 {
            return 1.0;
        }
        let big = p[0].abs().max(p[1].abs()).max(p[2].abs()).max(1.0);
        let from = add(p, mul(ng, big * 1e-9 + 1e-6));
        if self.occluded(from, self.sun_dir) { 0.0 } else { 1.0 }
    }

    /// light (times the exposure) as the target's 8-bit value, through the tone mapper
    pub fn to_rgb8(&self, c: [f64; 3]) -> [u8; 3] {
        let c = if self.tonemap { neutral(c) } else { c };
        let px = |v: f64| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        [px(c[0]), px(c[1]), px(c[2])]
    }

    /// the light of instance k's triangle ti at (u, v), seen from eye; the sun's part unshadowed; the sun's reach
    fn shade_parts(&self, k: usize, ti: usize, u: f64, v: f64, eye: V, quad: Option<&Quad>, normal: Option<V>) -> ([f64; 3], [f64; 3], f64) {
        let t = &self.tris[ti];
        let sf = self.surface(&self.inst[k].look, t, u, v, eye, quad, normal);
        let p = add(add(t.v0, mul(t.e1, u)), mul(t.e2, v));
        let w = 1.0 - u - v;
        let ng = norm(add(add(mul(t.n[0], w), mul(t.n[1], u)), mul(t.n[2], v)));
        let vis = self.sun_vis(p, ng);
        let (c, sun) = self.light(&sf, p, eye, vis);
        (c, sun, vis)
    }

    /// the light of instance k's triangle ti at (u, v), seen from eye
    fn shade(&self, k: usize, ti: usize, u: f64, v: f64, eye: V, quad: Option<&Quad>, normal: Option<V>) -> [f64; 3] {
        self.shade_parts(k, ti, u, v, eye, quad, normal).0
    }

    /// (diagnostics) the uv of a hit
    pub fn uv_of(&self, h: &Hit) -> [f64; 2] {
        let t = &self.tris[h.tri];
        let w = 1.0 - h.u - h.v;
        [t.uv[0][0] * w + t.uv[1][0] * h.u + t.uv[2][0] * h.v, t.uv[0][1] * w + t.uv[1][1] * h.u + t.uv[2][1] * h.v]
    }

    /// the nearest front face along eye + z * d with z between near and far
    pub fn trace(&self, eye: V, d: V, near: f64, far: f64) -> Option<Hit> {
        self.trace_quad(eye, d, near, far, None)
    }

    /// trace with the rays of the pixel's quad for the texture's level (and a MASK's alpha)
    pub fn trace_quad(&self, eye: V, d: V, near: f64, far: f64, quad: Option<&Quad>) -> Option<Hit> {
        let mut best: Option<(f64, usize, usize, f64, f64)> = None;
        let dd = dot(d, d);
        for (k, inst) in self.inst.iter().enumerate() {
            // past the box's sphere?
            let oc = sub(inst.center, eye);
            let along = dot(oc, d);
            if dot(oc, oc) - along * along / dd > inst.r2 {
                continue;
            }
            for ti in inst.first..inst.last {
                let t = &self.tris[ti];
                let p = cross(d, t.e2);
                let det = dot(t.e1, p);
                if det <= 0.0 {
                    continue; // a back face
                }
                let o = sub(eye, t.v0);
                let u = dot(o, p) / det;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let q = cross(o, t.e1);
                let v = dot(d, q) / det;
                if v < 0.0 || u + v > 1.0 {
                    continue;
                }
                let z = dot(t.e2, q) / det;
                if z < near || z > far || best.as_ref().is_some_and(|b| b.0 <= z) {
                    continue;
                }
                if let Some(cut) = inst.look.cutoff {
                    if self.base(&inst.look, t, u, v, eye, quad)[3] < cut {
                        continue; // not drawn: MASK
                    }
                }
                best = Some((z, k, ti, u, v));
            }
        }
        let (z, k, ti, u, v) = best?;
        let (linear, sun, vis) = self.shade_parts(k, ti, u, v, eye, quad, None);
        Some(Hit { z, rgb: self.to_rgb8(linear), inst: k, tri: ti, u, v, linear, sun, vis })
    }

    /// this colour is one the hit's instance shows near the hit: between the colours of the hit and of its
    /// triangle's vertex normals there (one colour only: a textured instance relies on its neighbours' rays)
    fn could_show(&self, hit: &Hit, eye: V, got: [u8; 3]) -> bool {
        if self.inst[hit.inst].look.textured() {
            return false;
        }
        let t = &self.tris[hit.tri];
        let mut lo = hit.rgb;
        let mut hi = hit.rgb;
        for n in t.n {
            let c = self.to_rgb8(self.shade(hit.inst, hit.tri, hit.u, hit.v, eye, None, Some(n)));
            for k in 0..3 {
                lo[k] = lo[k].min(c[k]);
                hi[k] = hi[k].max(c[k]);
            }
        }
        (0..3).all(|k| got[k] as i32 >= lo[k] as i32 - 2 && got[k] as i32 <= hi[k] as i32 + 2)
    }
}

/// what comparing an image with the rays found
#[derive(Clone, Copy, Debug, Default)]
pub struct Compared {
    /// pixels away from every edge, of them the ones with an object
    pub solid: u32,
    pub solid_hit: u32,
    /// pixels at an edge
    pub edges: u32,
    /// solid pixels that show an object a neighbouring pixel's ray sees: slivers
    pub slivers: u32,
    /// solid pixels off by more than the tolerance, edge pixels showing none of what their rays see
    pub bad: u32,
    /// the first bad pixel: x, y, got rgb, want rgb
    pub first: Option<(u32, u32, [u8; 3], [u8; 3])>,
    /// the depth buffer's largest error at solid pixels with an object, relative to the value
    pub depth_err: f64,
    pub depth_bad: u32,
    pub objects_seen: u32,
    /// the largest difference of a channel at a solid pixel, and how many solid pixels differ at all
    pub largest: u8,
    pub differ: u32,
}

fn near_rgb(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    (0..3).all(|k| a[k].abs_diff(b[k]) <= tol)
}

/// the image (RGBA rows), and if there the depth buffer, against rays through the pixels: solid pixels to
/// within 1 of a channel
pub fn compare(sc: &RefScene, cam: &RefCamera, w: u32, h: u32, clear: [u8; 3], image: &[u8], depth: Option<&[f32]>) -> Compared {
    compare_tol(sc, cam, w, h, clear, image, depth, 1)
}

/// compare with a tolerance per channel at solid pixels (trilinear filtering: the hardware's weights and
/// level differ from the definition by a little)
#[allow(clippy::too_many_arguments)]
pub fn compare_tol(sc: &RefScene, cam: &RefCamera, w: u32, h: u32, clear: [u8; 3], image: &[u8], depth: Option<&[f32]>, tol: u8) -> Compared {
    const NEAR_EDGE: f64 = 1.0 / 64.0;
    let mut out = Compared::default();
    let mut seen = alloc::vec![false; sc.inst.len()];
    let shoot = |x: f64, y: f64, q: &Quad| sc.trace_quad(cam.eye, cam.ray(x, y, w, h), cam.near, cam.far, Some(q));
    let rgb = |h: &Option<Hit>| h.map_or(clear, |v| v.rgb);
    for y in 0..h {
        for x in 0..w {
            let at = ((y * w + x) * 4) as usize;
            let got = [image[at], image[at + 1], image[at + 2]];
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            let (qx, qy) = ((x & !1) as f64 + 0.5, (y & !1) as f64 + 0.5);
            let q = Quad { x0: cam.ray(qx, cy, w, h), x1: cam.ray(qx + 1.0, cy, w, h), y0: cam.ray(cx, qy, w, h), y1: cam.ray(cx, qy + 1.0, w, h) };
            let c = shoot(cx, cy, &q);
            let around = [shoot(cx - NEAR_EDGE, cy, &q), shoot(cx + NEAR_EDGE, cy, &q), shoot(cx, cy - NEAR_EDGE, &q), shoot(cx, cy + NEAR_EDGE, &q)];
            let same = |h: &Option<Hit>| h.map(|v| v.inst) == c.map(|v| v.inst) && near_rgb(rgb(h), rgb(&c), 1);
            let want = rgb(&c);
            if let Some(hit) = c {
                seen[hit.inst] = true;
            }
            if around.iter().all(same) {
                out.solid += 1;
                let d = (0..3).map(|k| got[k].abs_diff(want[k])).max().unwrap_or(0);
                out.largest = out.largest.max(d);
                out.differ += (d > 0) as u32;
                if !near_rgb(got, want, tol) {
                    let wide = [shoot(cx - 1.0, cy, &q), shoot(cx + 1.0, cy, &q), shoot(cx, cy - 1.0, &q), shoot(cx, cy + 1.0, &q), shoot(cx - 1.0, cy - 1.0, &q),
                                shoot(cx + 1.0, cy - 1.0, &q), shoot(cx - 1.0, cy + 1.0, &q), shoot(cx + 1.0, cy + 1.0, &q)];
                    if wide.iter().any(|h| h.is_some_and(|v| sc.could_show(&v, cam.eye, got))) {
                        out.slivers += 1;
                    } else {
                        out.bad += 1;
                        out.first.get_or_insert((x, y, got, want));
                    }
                    continue;
                }
                if let (Some(hit), Some(d)) = (c, depth) {
                    out.solid_hit += 1;
                    let (v, exact) = (d[(y * w + x) as usize] as f64, cam.depth(hit.z));
                    let e = (v - exact).abs() / exact.abs().max(1e-30);
                    if e > out.depth_err {
                        out.depth_err = e;
                    }
                    if e > 1e-3 {
                        out.depth_bad += 1;
                    }
                } else if c.is_some() {
                    out.solid_hit += 1;
                }
            } else {
                out.edges += 1;
                let fine = near_rgb(got, want, tol) || around.iter().any(|h| near_rgb(got, rgb(h), tol) || h.is_some_and(|v| sc.could_show(&v, cam.eye, got)))
                    || c.is_some_and(|v| sc.could_show(&v, cam.eye, got));
                if !fine {
                    out.bad += 1;
                    out.first.get_or_insert((x, y, got, want));
                }
            }
        }
    }
    out.objects_seen = seen.iter().filter(|v| **v).count() as u32;
    out
}

/// (G5c) what comparing a shadowed image with exact shadow rays found
#[derive(Clone, Copy, Debug, Default)]
pub struct ShadowCompared {
    /// pixels away from geometric edges, of them on objects, of them in the sun's shadow by the rays
    pub solid: u32,
    pub on_objects: u32,
    pub shadowed: u32,
    /// solid pixels whose colour lies on no mix of their point's shadowed and lit colour (tolerance): wrong
    pub bad: u32,
    /// the mix the GPU shows is nearer the other side than the rays' (more than half): within 3 pixels of a
    /// pixel whose ray says otherwise (a shadow's edge, the filter's and the bias's doing), or not (acne, leaks)
    pub at_edges: u32,
    pub inside: u32,
    /// pixels at geometric edges, and of them wrong
    pub edges: u32,
    pub edges_bad: u32,
    pub first: Option<(u32, u32, [u8; 3], [u8; 3])>,
    pub first_inside: Option<(u32, u32, f64, f64)>,
}

impl ShadowCompared {
    pub fn text(&self) -> alloc::string::String {
        let share = |n: u32| n as f64 * 100.0 / self.on_objects.max(1) as f64;
        format!("{} solid ({} on objects, {} shadowed by the rays): {} other side at a shadow's edge ({:.2} %), {} inside ({:.3} %), {} wrong; {} at geometric edges, {} wrong{}{}",
                self.solid, self.on_objects, self.shadowed, self.at_edges, share(self.at_edges), self.inside, share(self.inside), self.bad, self.edges, self.edges_bad,
                self.first.map_or(alloc::string::String::new(), |f| format!(" (first wrong at {}, {}: {:?}, expected {:?})", f.0, f.1, f.2, f.3)),
                self.first_inside.map_or(alloc::string::String::new(), |f| format!(" (first inside at {}, {}: shows {:.2}, rays {})", f.0, f.1, f.2, f.3)))
    }
}

/// (G5c) an image of a scene whose sun casts shadows against exact shadow rays: each solid pixel must show a
/// mix of its point's shadowed and lit colour (the filter's penumbra), and that mix is judged against the rays
pub fn compare_shadows(sc: &RefScene, cam: &RefCamera, w: u32, h: u32, clear: [u8; 3], image: &[u8], tol: u8) -> ShadowCompared {
    const NEAR_EDGE: f64 = 1.0 / 64.0;
    let mut out = ShadowCompared::default();
    let shoot = |x: f64, y: f64| sc.trace(cam.eye, cam.ray(x, y, w, h), cam.near, cam.far);
    // the rays' answer per pixel first: -1 for no object
    let mut vis = alloc::vec![-1.0f64; (w * h) as usize];
    let mut hits = alloc::vec![None; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let c = shoot(x as f64 + 0.5, y as f64 + 0.5);
            if let Some(hit) = c {
                vis[(y * w + x) as usize] = hit.vis;
            }
            hits[(y * w + x) as usize] = c;
        }
    }
    let mix = |hit: &Hit, t: f64| -> [u8; 3] {
        let d = [hit.linear[0] - hit.vis * hit.sun[0], hit.linear[1] - hit.vis * hit.sun[1], hit.linear[2] - hit.vis * hit.sun[2]];
        sc.to_rgb8([d[0] + t * hit.sun[0], d[1] + t * hit.sun[1], d[2] + t * hit.sun[2]])
    };
    // the mix nearest to the colour: (distance, t)
    let nearest = |hit: &Hit, got: [u8; 3]| -> (u8, f64) {
        let mut best = (255u8, 0.0);
        for k in 0..=64 {
            let t = k as f64 / 64.0;
            let c = mix(hit, t);
            let d = (0..3).map(|i| c[i].abs_diff(got[i])).max().unwrap_or(0);
            if d < best.0 {
                best = (d, t);
            }
        }
        best
    };
    for y in 0..h {
        for x in 0..w {
            let at = ((y * w + x) * 4) as usize;
            let got = [image[at], image[at + 1], image[at + 2]];
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            let c = hits[(y * w + x) as usize];
            let around = [shoot(cx - NEAR_EDGE, cy), shoot(cx + NEAR_EDGE, cy), shoot(cx, cy - NEAR_EDGE), shoot(cx, cy + NEAR_EDGE)];
            let same = around.iter().all(|h| h.map(|v| v.inst) == c.map(|v| v.inst));
            if !same {
                out.edges += 1;
                let fine = c.iter().chain(around.iter().flatten()).any(|h| nearest(h, got).0 <= tol + 1) || near_rgb(got, clear, tol)
                    || c.iter().chain(around.iter().flatten()).any(|h| sc.could_show(h, cam.eye, got));
                if !fine {
                    out.edges_bad += 1;
                }
                continue;
            }
            out.solid += 1;
            let Some(hit) = c else {
                if !near_rgb(got, clear, tol) {
                    out.bad += 1;
                    out.first.get_or_insert((x, y, got, clear));
                }
                continue;
            };
            out.on_objects += 1;
            out.shadowed += (hit.vis < 0.5) as u32;
            let (d, t) = nearest(&hit, got);
            if d > tol {
                out.bad += 1;
                out.first.get_or_insert((x, y, got, hit.rgb));
                continue;
            }
            // a sun too weak here to tell shadow from light
            let (lit, dark) = (mix(&hit, 1.0), mix(&hit, 0.0));
            if (0..3).map(|i| lit[i].abs_diff(dark[i])).max().unwrap_or(0) < 4 {
                continue;
            }
            if (t - hit.vis).abs() > 0.5 {
                let mut edge = false;
                for dy in -3i32..=3 {
                    for dx in -3i32..=3 {
                        let (xx, yy) = (x as i32 + dx, y as i32 + dy);
                        if xx >= 0 && yy >= 0 && (xx as u32) < w && (yy as u32) < h {
                            let v = vis[(yy as u32 * w + xx as u32) as usize];
                            if v >= 0.0 && (v - hit.vis).abs() > 0.5 {
                                edge = true;
                            }
                        }
                    }
                }
                if edge {
                    out.at_edges += 1;
                } else {
                    out.inside += 1;
                    out.first_inside.get_or_insert((x, y, t, hit.vis));
                }
            }
        }
    }
    out
}

impl Compared {
    /// nothing wrong, slivers few
    pub fn fine(&self) -> bool {
        self.bad == 0 && self.solid > 0 && self.slivers * 100 <= self.solid_hit + 200
    }

    pub fn text(&self) -> alloc::string::String {
        format!("{} solid ({} on objects, {} objects seen; {} differ, largest {}), {} at edges, {} slivers, {} wrong{}", self.solid, self.solid_hit, self.objects_seen,
                self.differ, self.largest, self.edges, self.slivers, self.bad,
                self.first.map_or(alloc::string::String::new(), |f| format!(" (first at {}, {}: {:?}, expected {:?})", f.0, f.1, f.2, f.3)))
    }
}
