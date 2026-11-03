// cpu -- the reference the tests judge the engine against: a ray caster and a visibility test in f64,
// written from the definitions and sharing no code with lib/render3d (its own square root, its own
// rotation by quaternion products, the planes as distances in camera space).  A pixel of the rasteriser
// is covered when its centre lies in the triangle, so a ray through the centre sees what the GPU should;
// the GPU snaps vertices to 1/256 of a pixel, so a pixel whose rays around the centre disagree counts as
// an edge and may show the colour, under some light, of what those rays see (the GPU's normal there lies a
// little beyond the triangle's end).  The snapping can also turn a triangle seen almost edge on towards
// the eye, which for the rays stays a back face: a sliver on a silhouette.  Those are counted, and may only
// be few and show the colour of an object a neighbouring pixel's ray sees.

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
    (c, radius * sx.max(sy).max(sz))
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

#[derive(Clone, Copy)]
struct Tri {
    v0: V,
    e1: V,
    e2: V,
    n: [V; 3],
}

struct RefInstance {
    color: [f64; 3],
    center: V,
    r2: f64,
    first: usize,
    last: usize,
}

/// a scene for rays
pub struct RefScene {
    tris: Vec<Tri>,
    inst: Vec<RefInstance>,
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

impl RefScene {
    pub fn new(sun_dir: V, sun: [f64; 3], ambient: [f64; 3]) -> RefScene {
        RefScene { tris: Vec::new(), inst: Vec::new(), sun_dir: norm(sun_dir), sun, ambient }
    }

    /// an instance: its triangles in the world
    pub fn add(&mut self, geo: &Geo, s: &Spec, color: [f32; 3]) {
        let q = unit_quat(s.rot);
        let scale = [s.scale[0] as f64, s.scale[1] as f64, s.scale[2] as f64];
        let world = |p: [f32; 3]| add(s.pos, turn(q, [p[0] as f64 * scale[0], p[1] as f64 * scale[1], p[2] as f64 * scale[2]]));
        // the inverse transpose of rotation * scale
        let normal = |n: [f32; 3]| turn(q, [n[0] as f64 / scale[0], n[1] as f64 / scale[1], n[2] as f64 / scale[2]]);
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
        for t in geo.idx.chunks_exact(3) {
            let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
            let (v0, v1, v2) = (world(geo.pos[a]), world(geo.pos[b]), world(geo.pos[c]));
            self.tris.push(Tri { v0, e1: sub(v1, v0), e2: sub(v2, v0), n: [normal(geo.nrm[a]), normal(geo.nrm[b]), normal(geo.nrm[c])] });
        }
        self.inst.push(RefInstance { color: [color[0] as f64, color[1] as f64, color[2] as f64], center, r2: dot(half, half) * 1.0001, first, last: self.tris.len() });
    }

    /// the nearest front face along eye + z * d with z between near and far
    pub fn trace(&self, eye: V, d: V, near: f64, far: f64) -> Option<Hit> {
        let mut best: Option<(f64, usize, V)> = None;
        let dd = dot(d, d);
        for (k, inst) in self.inst.iter().enumerate() {
            // past the box's sphere?
            let oc = sub(inst.center, eye);
            let along = dot(oc, d);
            if dot(oc, oc) - along * along / dd > inst.r2 {
                continue;
            }
            for t in &self.tris[inst.first..inst.last] {
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
                let w = 1.0 - u - v;
                let n = add(add(mul(t.n[0], w), mul(t.n[1], u)), mul(t.n[2], v));
                best = Some((z, k, n));
            }
        }
        let (z, k, n) = best?;
        let ndl = dot(norm(n), self.sun_dir).max(0.0);
        let c = self.inst[k].color;
        let px = |i: usize| {
            let v = c[i] * (self.ambient[i] + self.sun[i] * ndl);
            (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        };
        Some(Hit { z, rgb: [px(0), px(1), px(2)], inst: k })
    }
}

impl RefScene {
    /// this colour is that of the instance under some light between the ambient and the full sun
    fn could_show(&self, inst: usize, got: [u8; 3]) -> bool {
        let c = self.inst[inst].color;
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
    /// solid pixels off by more than 1, edge pixels showing none of what their rays see
    pub bad: u32,
    /// the first bad pixel: x, y, got rgb, want rgb
    pub first: Option<(u32, u32, [u8; 3], [u8; 3])>,
    /// the depth buffer's largest error at solid pixels with an object, relative to the value
    pub depth_err: f64,
    pub depth_bad: u32,
    pub objects_seen: u32,
}

fn near_rgb(a: [u8; 3], b: [u8; 3]) -> bool {
    (0..3).all(|k| a[k].abs_diff(b[k]) <= 1)
}

/// the image (RGBA rows), and if there the depth buffer, against rays through the pixels
pub fn compare(sc: &RefScene, cam: &RefCamera, w: u32, h: u32, clear: [u8; 3], image: &[u8], depth: Option<&[f32]>) -> Compared {
    const NEAR_EDGE: f64 = 1.0 / 64.0;
    let mut out = Compared::default();
    let mut seen = alloc::vec![false; sc.inst.len()];
    let shoot = |x: f64, y: f64| sc.trace(cam.eye, cam.ray(x, y, w, h), cam.near, cam.far);
    let rgb = |h: &Option<Hit>| h.map_or(clear, |v| v.rgb);
    for y in 0..h {
        for x in 0..w {
            let at = ((y * w + x) * 4) as usize;
            let got = [image[at], image[at + 1], image[at + 2]];
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            let c = shoot(cx, cy);
            let around = [shoot(cx - NEAR_EDGE, cy), shoot(cx + NEAR_EDGE, cy), shoot(cx, cy - NEAR_EDGE), shoot(cx, cy + NEAR_EDGE)];
            let same = |h: &Option<Hit>| h.map(|v| v.inst) == c.map(|v| v.inst) && near_rgb(rgb(h), rgb(&c));
            let want = rgb(&c);
            if let Some(hit) = c {
                seen[hit.inst] = true;
            }
            if around.iter().all(same) {
                out.solid += 1;
                if !near_rgb(got, want) {
                    let wide = [shoot(cx - 1.0, cy), shoot(cx + 1.0, cy), shoot(cx, cy - 1.0), shoot(cx, cy + 1.0), shoot(cx - 1.0, cy - 1.0), shoot(cx + 1.0, cy - 1.0), shoot(cx - 1.0, cy + 1.0), shoot(cx + 1.0, cy + 1.0)];
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
                let fine = near_rgb(got, want) || around.iter().any(|h| near_rgb(got, rgb(h)) || h.is_some_and(|v| sc.could_show(v.inst, got)))
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
        format!("{} solid ({} on objects, {} objects seen), {} at edges, {} slivers, {} wrong{}", self.solid, self.solid_hit, self.objects_seen, self.edges, self.slivers, self.bad,
                self.first.map_or(alloc::string::String::new(), |f| format!(" (first at {}, {}: {:?}, expected {:?})", f.0, f.1, f.2, f.3)))
    }
}
