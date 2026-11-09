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

use alloc::format;
use alloc::vec::Vec;

use crate::common::{Geo, Spec};

pub type V = [f64; 3];

fn sqrt(x: f64) -> f64 {
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

fn dot(a: V, b: V) -> f64 {
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

// ---------------------------------------------------------------- the ray caster

#[derive(Clone, Copy)]
struct Tri {
    v0: V,
    e1: V,
    e2: V,
    n: [V; 3],
    uv: [[f64; 2]; 3],
}

/// what an instance looks like: a colour, or a factor times a texture; with a cutoff, glTF's MASK
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub factor: [f64; 4],
    pub texture: Option<(usize, RefSampling)>,
    pub cutoff: Option<f64>,
}

impl Look {
    pub fn color(c: [f32; 3]) -> Look {
        Look { factor: [c[0] as f64, c[1] as f64, c[2] as f64, 1.0], texture: None, cutoff: None }
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
    pub sun: [f64; 3],
    pub ambient: [f64; 3],
}

/// what a ray found
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub z: f64,
    pub rgb: [u8; 3],
    pub inst: usize,
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
        RefScene { tris: Vec::new(), inst: Vec::new(), textures: Vec::new(), sun_dir: norm(sun_dir), sun, ambient }
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
        let Some((tex, s)) = look.texture else { return f };
        let w = 1.0 - u - v;
        let uv = [t.uv[0][0] * w + t.uv[1][0] * u + t.uv[2][0] * v, t.uv[0][1] * w + t.uv[1][1] * u + t.uv[2][1] * v];
        let (dx, dy) = match quad {
            Some(q) => {
                let (a, b, c, e) = (RefScene::uv_on_plane(t, eye, q.x0), RefScene::uv_on_plane(t, eye, q.x1), RefScene::uv_on_plane(t, eye, q.y0), RefScene::uv_on_plane(t, eye, q.y1));
                ([b[0] - a[0], b[1] - a[1]], [e[0] - c[0], e[1] - c[1]])
            }
            None => ([0.0; 2], [0.0; 2]),
        };
        let c = self.textures[tex].sample(&s, uv, dx, dy);
        [f[0] * c[0], f[1] * c[1], f[2] * c[2], f[3] * c[3]]
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
        let t = &self.tris[ti];
        let w = 1.0 - u - v;
        let n = add(add(mul(t.n[0], w), mul(t.n[1], u)), mul(t.n[2], v));
        let ndl = dot(norm(n), self.sun_dir).max(0.0);
        let c = self.base(&self.inst[k].look, t, u, v, eye, quad);
        let px = |i: usize| {
            let v = c[i] * (self.ambient[i] + self.sun[i] * ndl);
            (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        };
        Some(Hit { z, rgb: [px(0), px(1), px(2)], inst: k })
    }

    /// this colour is that of the instance under some light between the ambient and the full sun (one
    /// colour only: a textured instance relies on its neighbours' rays)
    fn could_show(&self, inst: usize, got: [u8; 3]) -> bool {
        let look = &self.inst[inst].look;
        if look.texture.is_some() {
            return false;
        }
        let c = look.factor;
        let k = (0..3).map(|i| got[i] as f64 / 255.0 / c[i].max(1e-6)).fold(0.0, f64::max);
        let top = (0..3).map(|i| self.ambient[i] + self.sun[i]).fold(0.0, f64::max) + 0.01;
        k <= top && (0..3).all(|i| ((c[i] * k * 255.0 + 0.5) as i32 - got[i] as i32).abs() <= 2)
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
                    if wide.iter().any(|h| h.is_some_and(|v| sc.could_show(v.inst, got))) {
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
                let fine = near_rgb(got, want, tol) || around.iter().any(|h| near_rgb(got, rgb(h), tol) || h.is_some_and(|v| sc.could_show(v.inst, got)))
                    || c.is_some_and(|v| sc.could_show(v.inst, got));
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
