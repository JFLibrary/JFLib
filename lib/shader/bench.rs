// bench -- the GPU benchmark scene: 100 000 to 1 000 000 instanced cubes, moved, culled and
// drawn on the GPU (docs/SHADER.md, "Benchmark-Vertrag").  The shaders as SPIR-V words for
// SPIR-V 1.3 (Basis) or 1.6 (Voll), their data contract and CPU references.
//
//   Shader::{Move, Cull, CullReset, VsInstanced, VsSingle, FsLambert}.build(minor)
//   move_spv / cull_spv / cull_reset_spv / vs_instanced_spv / vs_single_spv / fs_lambert_spv
//   Object (48 bytes), Camera (176), DrawCmd (20), CubeVertex (32), Push (20): the buffer
//   records with bytes() / from_bytes(); offsets in the *_OFF constants
//   cube_vertices() (24) / cube_indices() (36, u16, counter-clockwise from outside)
//   Camera::look_at / from_view_proj: Vulkan clip space (y down, depth 0..1), planes
//   hash, unit, floor, sincos_turns, move_ref, move_all, visible, cull_ref: the CPU references
//
// Descriptor set 0 (one layout for every pipeline, stages COMPUTE | VERTEX | FRAGMENT):
//   binding 0  B_CAMERA   uniform buffer  Camera (std140)        cull, vs_*, fs_lambert
//   binding 1  B_OBJECTS  storage buffer  Object[count] (std430)  move (writes), cull, vs_*
//   binding 2  B_VISIBLE  storage buffer  uint[count]             cull (writes), vs_instanced
//   binding 3  B_DRAW     storage buffer  DrawCmd, also the INDIRECT buffer   cull_reset, cull
//   binding 4  B_CUBE     storage buffer  CubeVertex[24]          vs_*
// Push constants (all stages, offset 0, 20 bytes): Push { time, count, seed, extent,
// object_index }.  Workgroups: move and cull 256 x 1 x 1 (dispatch ceil(count / 256)),
// cull_reset 1 x 1 x 1.  Every shader: entry point "main", capability Shader only.
//
// move and cull are exact: only add, sub, mul, floor, conversions of integers below 2^24 and
// integer ops, every float result decorated NoContraction (no fused multiply-add), in the
// order of the CPU references.  Vulkan rounds add / sub / mul correctly, so the GPU results
// equal move_ref / cull_ref bit for bit (denormals may flush to zero; the visible list comes
// in atomic order: compare it sorted).

use alloc::vec::Vec;

use crate::layout::Rule;
use crate::module::{Id, Module};
use crate::spv::{glsl, op, scope, Bi, Dec, Mode, Model, Sc};

pub const SET: u32 = 0;
pub const B_CAMERA: u32 = 0;
pub const B_OBJECTS: u32 = 1;
pub const B_VISIBLE: u32 = 2;
pub const B_DRAW: u32 = 3;
pub const B_CUBE: u32 = 4;
pub const WG_MOVE: u32 = 256;
pub const WG_CULL: u32 = 256;
pub const CUBE_VERTICES: u32 = 24;
pub const CUBE_INDICES: u32 = 36;

// Object, std430, stride 48: vec3 pos, float scale, vec4 rot (quaternion x y z w), vec3
// color, float radius (bounding sphere)
pub const OBJECT_SIZE: u32 = 48;
pub const OBJECT_OFF: [u32; 5] = [0, 12, 16, 32, 44];
// Camera, std140: mat4 view_proj (column-major, MatrixStride 16), vec4 planes[6] (stride 16),
// vec3 light_dir (towards the light, unit length), float ambient
pub const CAMERA_SIZE: u32 = 176;
pub const CAMERA_OFF: [u32; 4] = [0, 64, 160, 172];
// DrawCmd = VkDrawIndexedIndirectCommand, std430
pub const DRAW_SIZE: u32 = 20;
pub const DRAW_OFF: [u32; 5] = [0, 4, 8, 12, 16];
// CubeVertex, std430, stride 32: vec4 pos (w = 1), vec4 normal (w = 0)
pub const CUBE_VERTEX_SIZE: u32 = 32;
// Push, std430: float time, uint count, uint seed, float extent, uint object_index
pub const PUSH_SIZE: u32 = 20;
pub const PUSH_OFF: [u32; 5] = [0, 4, 8, 12, 16];

// ---- the records

fn put(out: &mut Vec<u8>, v: &[f32]) {
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

fn get(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn get_u(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Object {
    pub pos: [f32; 3],
    pub scale: f32,
    pub rot: [f32; 4],
    pub color: [f32; 3],
    pub radius: f32,
}

impl Object {
    pub fn bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(48);
        put(&mut o, &self.pos);
        put(&mut o, &[self.scale]);
        put(&mut o, &self.rot);
        put(&mut o, &self.color);
        put(&mut o, &[self.radius]);
        o
    }
    /// one record from 48 bytes
    pub fn from_bytes(b: &[u8]) -> Object {
        let f = |i: usize| get(b, i * 4);
        Object { pos: [f(0), f(1), f(2)], scale: f(3), rot: [f(4), f(5), f(6), f(7)], color: [f(8), f(9), f(10)], radius: f(11) }
    }
}

/// the bytes of an Object buffer
pub fn objects_bytes(objs: &[Object]) -> Vec<u8> {
    objs.iter().flat_map(|o| o.bytes()).collect()
}

/// an Object buffer read back (len / 48 records)
pub fn objects_from_bytes(b: &[u8]) -> Vec<Object> {
    b.chunks_exact(OBJECT_SIZE as usize).map(Object::from_bytes).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Camera {
    /// clip = view_proj * vec4(world, 1), column-major (view_proj[col * 4 + row])
    pub view_proj: [f32; 16],
    /// (n, d) with |n| = 1: a point p is inside when dot(n, p) + d >= 0
    pub planes: [[f32; 4]; 6],
    pub light_dir: [f32; 3],
    pub ambient: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DrawCmd {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub vertex_offset: i32,
    pub first_instance: u32,
}

impl DrawCmd {
    /// the command cull_reset writes: 36 indices, no instances yet
    pub fn reset() -> DrawCmd {
        DrawCmd { index_count: CUBE_INDICES, ..DrawCmd::default() }
    }
    pub fn bytes(&self) -> Vec<u8> {
        [self.index_count, self.instance_count, self.first_index, self.vertex_offset as u32, self.first_instance]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect()
    }
    pub fn from_bytes(b: &[u8]) -> DrawCmd {
        DrawCmd { index_count: get_u(b, 0), instance_count: get_u(b, 4), first_index: get_u(b, 8),
                  vertex_offset: get_u(b, 12) as i32, first_instance: get_u(b, 16) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Push {
    /// seconds since the start
    pub time: f32,
    pub count: u32,
    pub seed: u32,
    /// the scene's radius: orbits between 0.1 and 1.0 times extent
    pub extent: f32,
    /// vs_single: the object drawn
    pub object_index: u32,
}

impl Push {
    pub fn bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(20);
        o.extend_from_slice(&self.time.to_le_bytes());
        o.extend_from_slice(&self.count.to_le_bytes());
        o.extend_from_slice(&self.seed.to_le_bytes());
        o.extend_from_slice(&self.extent.to_le_bytes());
        o.extend_from_slice(&self.object_index.to_le_bytes());
        o
    }
}

/// a good extent for count cubes (about 3 world units between neighbours)
pub fn extent_for(count: u32) -> f32 {
    // cube root by bisection (no libm)
    let (mut lo, mut hi) = (0.0f64, 1.0e4f64);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if mid * mid * mid < count as f64 { lo = mid } else { hi = mid }
    }
    (lo * 3.0) as f32
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CubeVertex {
    pub pos: [f32; 4],
    pub normal: [f32; 4],
}

/// the 24 vertices of the cube -1 ..= 1 (4 per face, normals per face)
pub fn cube_vertices() -> Vec<CubeVertex> {
    // (normal, u, v) with u x v = normal: corners -u-v, u-v, u+v, -u+v run counter-clockwise
    // seen from outside
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    let mut out = Vec::with_capacity(24);
    for (n, u, v) in faces {
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = |k: usize| n[k] + a * u[k] + b * v[k];
            out.push(CubeVertex { pos: [p(0), p(1), p(2), 1.0], normal: [n[0], n[1], n[2], 0.0] });
        }
    }
    out
}

/// the 36 indices (two triangles per face)
pub fn cube_indices() -> Vec<u16> {
    (0..6u16).flat_map(|f| [0, 1, 2, 0, 2, 3].map(|i| f * 4 + i)).collect()
}

pub fn cube_bytes() -> Vec<u8> {
    let mut o = Vec::new();
    for v in cube_vertices() {
        put(&mut o, &v.pos);
        put(&mut o, &v.normal);
    }
    o
}

// ---- the camera (CPU side; f64 maths, f32 results)

fn sqrt64(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut r = f64::from_bits((x.to_bits() >> 1) + (1023u64 << 51));
    for _ in 0..8 {
        r = 0.5 * (r + x / r);
    }
    r
}

fn tan64(x: f64) -> f64 {
    let (x2, mut s, mut c, mut ts, mut tc) = (x * x, 0.0, 0.0, x, 1.0);
    for n in 0..24 {
        let n = n as f64;
        s += ts;
        c += tc;
        ts *= -x2 / ((2.0 * n + 2.0) * (2.0 * n + 3.0));
        tc *= -x2 / ((2.0 * n + 1.0) * (2.0 * n + 2.0));
    }
    s / c
}

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: V3) -> V3 {
    let l = sqrt64(dot3(a, a));
    [a[0] / l, a[1] / l, a[2] / l]
}

impl Camera {
    /// a right-handed camera at eye looking at target (+y up), vertical field of view fovy
    /// (radians), Vulkan clip space: y flipped (world up is up on screen), depth 0 (near) to
    /// 1 (far).  Front faces are counter-clockwise (VK_FRONT_FACE_COUNTER_CLOCKWISE).
    pub fn look_at(eye: [f32; 3], target: [f32; 3], fovy: f32, aspect: f32, near: f32, far: f32) -> Camera {
        let e: V3 = [eye[0] as f64, eye[1] as f64, eye[2] as f64];
        let t: V3 = [target[0] as f64, target[1] as f64, target[2] as f64];
        let f = norm(sub(t, e));
        let s = norm(cross(f, [0.0, 1.0, 0.0]));
        let u = cross(s, f);
        // rows of the view matrix
        let view = [[s[0], s[1], s[2], -dot3(s, e)], [u[0], u[1], u[2], -dot3(u, e)],
                    [-f[0], -f[1], -f[2], dot3(f, e)], [0.0, 0.0, 0.0, 1.0]];
        let fy = 1.0 / tan64(fovy as f64 * 0.5);
        let (n, fa) = (near as f64, far as f64);
        let proj = [[fy / aspect as f64, 0.0, 0.0, 0.0], [0.0, -fy, 0.0, 0.0],
                    [0.0, 0.0, fa / (n - fa), n * fa / (n - fa)], [0.0, 0.0, -1.0, 0.0]];
        let mut vp = [0.0f32; 16];
        for r in 0..4 {
            for c in 0..4 {
                let mut x = 0.0;
                for k in 0..4 {
                    x += proj[r][k] * view[k][c];
                }
                vp[c * 4 + r] = x as f32;
            }
        }
        Camera::from_view_proj(vp)
    }

    /// planes of a view-projection matrix (Vulkan: -w <= x, y <= w, 0 <= z <= w), normalised;
    /// the light comes from above (0.3, 0.8, 0.5), ambient 0.25
    pub fn from_view_proj(vp: [f32; 16]) -> Camera {
        let row = |r: usize| [vp[r] as f64, vp[4 + r] as f64, vp[8 + r] as f64, vp[12 + r] as f64];
        let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
        let comb = |a: [f64; 4], b: [f64; 4], s: f64| [a[0] + s * b[0], a[1] + s * b[1], a[2] + s * b[2], a[3] + s * b[3]];
        let raw = [comb(r3, r0, 1.0), comb(r3, r0, -1.0), comb(r3, r1, 1.0), comb(r3, r1, -1.0), r2, comb(r3, r2, -1.0)];
        let mut planes = [[0.0f32; 4]; 6];
        for (k, p) in raw.iter().enumerate() {
            let l = sqrt64(p[0] * p[0] + p[1] * p[1] + p[2] * p[2]);
            planes[k] = [(p[0] / l) as f32, (p[1] / l) as f32, (p[2] / l) as f32, (p[3] / l) as f32];
        }
        let ld = norm([0.3, 0.8, 0.5]);
        Camera { view_proj: vp, planes, light_dir: [ld[0] as f32, ld[1] as f32, ld[2] as f32], ambient: 0.25 }
    }

    /// the 176 bytes of the uniform buffer (std140)
    pub fn bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(176);
        put(&mut o, &self.view_proj);
        for p in &self.planes {
            put(&mut o, p);
        }
        put(&mut o, &self.light_dir);
        put(&mut o, &[self.ambient]);
        o
    }
}

// ---- CPU references (the same float operations in the same order as the shaders)

const PI: f32 = core::f32::consts::PI;
const INV24: f32 = 1.0 / 16_777_216.0;
const INV255: f32 = 1.0 / 255.0;
const SQRT3: f32 = 1.732_050_8;
// Taylor coefficients of sin(x) and cos(x), |x| <= pi / 2 (Horner, innermost first)
const SIN_C: [f32; 6] = [-1.0 / 39_916_800.0, 1.0 / 362_880.0, -1.0 / 5040.0, 1.0 / 120.0, -1.0 / 6.0, 1.0];
const COS_C: [f32; 7] = [1.0 / 479_001_600.0, -1.0 / 3_628_800.0, 1.0 / 40_320.0, -1.0 / 720.0, 1.0 / 24.0, -0.5, 1.0];

/// PCG hash (Jarkko & Olano, "Hash Functions for GPU Rendering")
pub fn hash(x: u32) -> u32 {
    let s = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277_803_737);
    (w >> 22) ^ w
}

/// the top 24 bits as a float in [0, 1) (exact)
pub fn unit(h: u32) -> f32 {
    (h >> 8) as f32 * INV24
}

/// floor as GLSL.std.450 Floor computes it (exact)
pub fn floor(x: f32) -> f32 {
    if f32::from_bits(x.to_bits() & 0x7fff_ffff) >= 8_388_608.0 || x == 0.0 || x.is_nan() {
        return x;
    }
    let t = x as i32 as f32;
    if t > x { t - 1.0 } else { t }
}

/// (sin, cos) of 2 pi u: u reduced to [-0.5, 0.5], sin and cos of pi r by Taylor polynomials,
/// then the double-angle formulas
pub fn sincos_turns(u: f32) -> (f32, f32) {
    let r = u - floor(u + 0.5);
    let x = r * PI;
    let x2 = x * x;
    let mut p = SIN_C[0];
    for &k in &SIN_C[1..] {
        p = k + x2 * p;
    }
    let sp = x * p;
    let mut q = COS_C[0];
    for &k in &COS_C[1..] {
        q = k + x2 * q;
    }
    let cp = q;
    ((sp * cp) * 2.0, cp * cp - sp * sp)
}

/// what move writes for object i
pub fn move_ref(i: u32, p: &Push) -> Object {
    let h1 = hash(i ^ p.seed);
    let h2 = hash(h1);
    let h3 = hash(h2);
    let h4 = hash(h3);
    let h5 = hash(h4);
    let h6 = hash(h5);
    let h7 = hash(h6);
    let h8 = hash(h7);
    let (f1, f2, f3, f4, f5, f6, f7) = (unit(h1), unit(h2), unit(h3), unit(h4), unit(h5), unit(h6), unit(h7));
    let orbit = p.extent * (0.1 + 0.9 * f1);
    let height = (p.extent * 0.25) * (f2 - 0.5);
    let speed = (f3 - 0.5) * 0.2;
    let u = f4 + speed * p.time;
    let (s, c) = sincos_turns(u);
    let scale = 0.25 + 0.5 * f5;
    let ua = f6 + (f6 - 0.5) * p.time;
    let ub = f7 + (f7 - 0.5) * p.time;
    let (sy, cy) = sincos_turns(ua);
    let (sx, cx) = sincos_turns(ub);
    let byte = |sh: u32| 0.35 + 0.65 * (((h8 >> sh) & 255) as f32 * INV255);
    Object {
        pos: [orbit * c, height, orbit * s],
        scale,
        rot: [cy * sx, sy * cx, -(sy * sx), cy * cx],
        color: [byte(0), byte(8), byte(16)],
        radius: scale * SQRT3,
    }
}

/// move for objects 0 .. p.count
pub fn move_all(p: &Push) -> Vec<Object> {
    (0..p.count).map(|i| move_ref(i, p)).collect()
}

/// cull's test: the bounding sphere is not completely outside one of the planes
pub fn visible(o: &Object, cam: &Camera) -> bool {
    let (c, nr) = (o.pos, -o.radius);
    cam.planes.iter().all(|p| !((((p[0] * c[0] + p[1] * c[1]) + p[2] * c[2]) + p[3]) < nr))
}

/// the indices cull appends, ascending (the GPU's list is in atomic order: sort it first)
pub fn cull_ref(objs: &[Object], cam: &Camera) -> Vec<u32> {
    (0..objs.len() as u32).filter(|&i| visible(&objs[i as usize], cam)).collect()
}

// ---- the shaders

/// the six benchmark shaders
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shader {
    Move,
    Cull,
    CullReset,
    VsInstanced,
    VsSingle,
    FsLambert,
}

impl Shader {
    pub const ALL: [Shader; 6] =
        [Shader::Move, Shader::Cull, Shader::CullReset, Shader::VsInstanced, Shader::VsSingle, Shader::FsLambert];

    pub fn name(self) -> &'static str {
        match self {
            Shader::Move => "move",
            Shader::Cull => "cull",
            Shader::CullReset => "cull_reset",
            Shader::VsInstanced => "vs_instanced",
            Shader::VsSingle => "vs_single",
            Shader::FsLambert => "fs_lambert",
        }
    }

    pub fn model(self) -> Model {
        match self {
            Shader::Move | Shader::Cull | Shader::CullReset => Model::GLCompute,
            Shader::VsInstanced | Shader::VsSingle => Model::Vertex,
            Shader::FsLambert => Model::Fragment,
        }
    }

    /// the module for SPIR-V 1.minor (3 or 6)
    pub fn build(self, minor: u32) -> Vec<u32> {
        match self {
            Shader::Move => move_spv(minor),
            Shader::Cull => cull_spv(minor),
            Shader::CullReset => cull_reset_spv(minor),
            Shader::VsInstanced => vs_spv(minor, true),
            Shader::VsSingle => vs_spv(minor, false),
            Shader::FsLambert => fs_lambert_spv(minor),
        }
    }
}

#[derive(Clone, Copy)]
struct Ts {
    void: Id,
    f: Id,
    u: Id,
    i: Id,
    v3: Id,
    v4: Id,
    uv3: Id,
}

fn ts(m: &mut Module) -> Ts {
    let void = m.ty_void();
    let f = m.ty_f32();
    let u = m.ty_u32();
    let i = m.ty_i32();
    let v3 = m.ty_vec(f, 3);
    let v4 = m.ty_vec(f, 4);
    let uv3 = m.ty_vec(u, 3);
    Ts { void, f, u, i, v3, v4, uv3 }
}

fn objects_var(m: &mut Module, t: Ts, readonly: bool) -> Id {
    let obj = m.ty_struct_layout("Object", &[("pos", t.v3), ("scale", t.f), ("rot", t.v4), ("color", t.v3), ("radius", t.f)],
                                 Rule::Std430);
    let arr = m.ty_runtime_array(obj, Some(OBJECT_SIZE));
    let block = m.ty_struct_layout("Objects", &[("items", arr)], Rule::Std430);
    let v = m.buffer(Sc::StorageBuffer, block, SET, B_OBJECTS);
    m.name(v, "objects");
    if readonly {
        m.decorate(v, Dec::NonWritable, &[]);
    }
    v
}

fn camera_var(m: &mut Module, t: Ts) -> Id {
    let mat4 = m.ty_mat(t.v4, 4);
    let planes = m.ty_array_layout(t.v4, 6, Rule::Std140);
    let block = m.ty_struct_layout("Camera", &[("view_proj", mat4), ("planes", planes), ("light_dir", t.v3), ("ambient", t.f)],
                                   Rule::Std140);
    let v = m.buffer(Sc::Uniform, block, SET, B_CAMERA);
    m.name(v, "camera");
    v
}

fn push_var(m: &mut Module, t: Ts) -> Id {
    let block = m.ty_struct_layout("Push", &[("time", t.f), ("count", t.u), ("seed", t.u), ("extent", t.f), ("object_index", t.u)],
                                   Rule::Std430);
    let v = m.push_constant(block);
    m.name(v, "pc");
    v
}

fn visible_var(m: &mut Module, t: Ts, readonly: bool) -> Id {
    let arr = m.ty_runtime_array(t.u, Some(4));
    let block = m.ty_struct_layout("Visible", &[("index", arr)], Rule::Std430);
    let v = m.buffer(Sc::StorageBuffer, block, SET, B_VISIBLE);
    m.name(v, "visible");
    if readonly {
        m.decorate(v, Dec::NonWritable, &[]);
    }
    v
}

fn draw_var(m: &mut Module, t: Ts) -> Id {
    let block = m.ty_struct_layout("DrawCmd", &[("index_count", t.u), ("instance_count", t.u), ("first_index", t.u),
                                                ("vertex_offset", t.i), ("first_instance", t.u)], Rule::Std430);
    let v = m.buffer(Sc::StorageBuffer, block, SET, B_DRAW);
    m.name(v, "draw");
    v
}

fn cube_var(m: &mut Module, t: Ts) -> Id {
    let vert = m.ty_struct_layout("CubeVertex", &[("pos", t.v4), ("normal", t.v4)], Rule::Std430);
    let arr = m.ty_runtime_array(vert, Some(CUBE_VERTEX_SIZE));
    let block = m.ty_struct_layout("Cube", &[("vertices", arr)], Rule::Std430);
    let v = m.buffer(Sc::StorageBuffer, block, SET, B_CUBE);
    m.name(v, "cube");
    m.decorate(v, Dec::NonWritable, &[]);
    v
}

/// load a member of the push constants
fn push_field(m: &mut Module, pc: Id, k: u32) -> Id {
    let p = m.access_c(pc, &[k]);
    m.load(p)
}

/// gl_GlobalInvocationID.x
fn global_x(m: &mut Module, t: Ts) -> Id {
    let g = m.builtin(Sc::Input, t.uv3, Bi::GlobalInvocationId);
    m.name(g, "gl_GlobalInvocationID");
    let v = m.load(g);
    m.extract(v, &[0])
}

fn hash_spv(m: &mut Module, x: Id) -> Id {
    let (a, b, c) = (m.const_u32(747_796_405), m.const_u32(2_891_336_453), m.const_u32(277_803_737));
    let (k28, k4, k22) = (m.const_u32(28), m.const_u32(4), m.const_u32(22));
    let s0 = m.imul(x, a);
    let s = m.iadd(s0, b);
    let sh0 = m.shr(s, k28);
    let sh = m.iadd(sh0, k4);
    let w0 = m.shr(s, sh);
    let w1 = m.bxor(w0, s);
    let w = m.imul(w1, c);
    let r = m.shr(w, k22);
    m.bxor(r, w)
}

fn unit_spv(m: &mut Module, t: Ts, h: Id) -> Id {
    let k8 = m.const_u32(8);
    let top = m.shr(h, k8);
    let f = m.u2f(t.f, top);
    let s = m.const_f32(INV24);
    m.fmul(f, s)
}

/// a + b * c with constants a, b (the order of the CPU reference: b * c first)
fn madd_c(m: &mut Module, a: f32, b: f32, c: Id) -> Id {
    let (ka, kb) = (m.const_f32(a), m.const_f32(b));
    let p = m.fmul(kb, c);
    m.fadd(ka, p)
}

fn sincos_spv(m: &mut Module, u: Id) -> (Id, Id) {
    let half = m.const_f32(0.5);
    let t = m.fadd(u, half);
    let fl = m.glsl(glsl::Floor, &[t]);
    let r = m.fsub(u, fl);
    let pi = m.const_f32(PI);
    let x = m.fmul(r, pi);
    let x2 = m.fmul(x, x);
    let mut p = m.const_f32(SIN_C[0]);
    for &k in &SIN_C[1..] {
        let q = m.fmul(x2, p);
        let kk = m.const_f32(k);
        p = m.fadd(kk, q);
    }
    let sp = m.fmul(x, p);
    let mut q = m.const_f32(COS_C[0]);
    for &k in &COS_C[1..] {
        let r = m.fmul(x2, q);
        let kk = m.const_f32(k);
        q = m.fadd(kk, r);
    }
    let cp = q;
    let sc = m.fmul(sp, cp);
    let two = m.const_f32(2.0);
    let s = m.fmul(sc, two);
    let cc = m.fmul(cp, cp);
    let ss = m.fmul(sp, sp);
    let c = m.fsub(cc, ss);
    (s, c)
}

fn store_member(m: &mut Module, objs: Id, i: Id, k: u32, v: Id) {
    let (c0, ck) = (m.const_u32(0), m.const_u32(k));
    let p = m.access(objs, &[c0, i, ck]);
    m.store(p, v);
}

fn load_member(m: &mut Module, objs: Id, i: Id, k: u32) -> Id {
    let (c0, ck) = (m.const_u32(0), m.const_u32(k));
    let p = m.access(objs, &[c0, i, ck]);
    m.load(p)
}

fn finish_compute(mut m: Module, main: Id, wg: u32) -> Vec<u32> {
    m.entry_point(Model::GLCompute, main, "main");
    m.exec_mode(main, Mode::LocalSize, &[wg, 1, 1]);
    m.finish()
}

/// move: object gid = move_ref(gid, pc) for gid < pc.count
pub fn move_spv(minor: u32) -> Vec<u32> {
    let mut m = Module::new(minor);
    m.no_contraction = true;
    let t = ts(&mut m);
    let objs = objects_var(&mut m, t, false);
    let pc = push_var(&mut m, t);
    let (main, _) = m.begin_function(t.void, &[]);
    m.name(main, "main");
    let i = global_x(&mut m, t);
    let count = push_field(&mut m, pc, 1);
    let inside = m.ult(i, count);
    m.if_then(inside, |m| {
        let time = push_field(m, pc, 0);
        let seed = push_field(m, pc, 2);
        let extent = push_field(m, pc, 3);
        let x = m.bxor(i, seed);
        let mut h = [Id(0); 8];
        h[0] = hash_spv(m, x);
        for k in 1..8 {
            h[k] = hash_spv(m, h[k - 1]);
        }
        let f: Vec<Id> = h[..7].iter().map(|&x| unit_spv(m, t, x)).collect();
        let half = m.const_f32(0.5);
        let ob = madd_c(m, 0.1, 0.9, f[0]);
        let orbit = m.fmul(extent, ob);
        let q = m.const_f32(0.25);
        let ek = m.fmul(extent, q);
        let d2 = m.fsub(f[1], half);
        let height = m.fmul(ek, d2);
        let d3 = m.fsub(f[2], half);
        let k02 = m.const_f32(0.2);
        let speed = m.fmul(d3, k02);
        let st = m.fmul(speed, time);
        let u = m.fadd(f[3], st);
        let (s, c) = sincos_spv(m, u);
        let scale = madd_c(m, 0.25, 0.5, f[4]);
        let mut spin = [Id(0); 2];
        for (n, &fv) in [f[5], f[6]].iter().enumerate() {
            let d = m.fsub(fv, half);
            let dt = m.fmul(d, time);
            spin[n] = m.fadd(fv, dt);
        }
        let (sy, cy) = sincos_spv(m, spin[0]);
        let (sx, cx) = sincos_spv(m, spin[1]);
        let px = m.fmul(orbit, c);
        let pz = m.fmul(orbit, s);
        let pos = m.construct(t.v3, &[px, height, pz]);
        let qx = m.fmul(cy, sx);
        let qy = m.fmul(sy, cx);
        let qz0 = m.fmul(sy, sx);
        let qz = m.fneg(qz0);
        let qw = m.fmul(cy, cx);
        let rot = m.construct(t.v4, &[qx, qy, qz, qw]);
        let mut col = [Id(0); 3];
        let k255 = m.const_u32(255);
        for (n, sh) in [0u32, 8, 16].iter().enumerate() {
            let ksh = m.const_u32(*sh);
            let b0 = m.shr(h[7], ksh);
            let b = m.band(b0, k255);
            let bf = m.u2f(t.f, b);
            let inv = m.const_f32(INV255);
            let w = m.fmul(bf, inv);
            col[n] = madd_c(m, 0.35, 0.65, w);
        }
        let color = m.construct(t.v3, &col);
        let s3 = m.const_f32(SQRT3);
        let radius = m.fmul(scale, s3);
        store_member(m, objs, i, 0, pos);
        store_member(m, objs, i, 1, scale);
        store_member(m, objs, i, 2, rot);
        store_member(m, objs, i, 3, color);
        store_member(m, objs, i, 4, radius);
    });
    m.ret();
    m.end_function();
    finish_compute(m, main, WG_MOVE)
}

/// cull: for gid < pc.count, if visible(objects[gid], camera): visible[atomicAdd(
/// draw.instance_count, 1)] = gid
pub fn cull_spv(minor: u32) -> Vec<u32> {
    let mut m = Module::new(minor);
    m.no_contraction = true;
    let t = ts(&mut m);
    let objs = objects_var(&mut m, t, true);
    let cam = camera_var(&mut m, t);
    let vis = visible_var(&mut m, t, false);
    let draw = draw_var(&mut m, t);
    let pc = push_var(&mut m, t);
    let b = m.ty_bool();
    let (main, _) = m.begin_function(t.void, &[]);
    m.name(main, "main");
    let yes = m.const_bool(true);
    let zero = m.const_u32(0);
    let inside_v = m.local(b, Some(yes));
    m.name(inside_v, "inside");
    let k = m.local(t.u, Some(zero));
    m.name(k, "k");
    let i = global_x(&mut m, t);
    let count = push_field(&mut m, pc, 1);
    let in_range = m.ult(i, count);
    m.if_then(in_range, |m| {
        let c = load_member(m, objs, i, 0);
        let r = load_member(m, objs, i, 4);
        let nr = m.fneg(r);
        let (one, six) = (m.const_u32(1), m.const_u32(6));
        m.loop_(
            |m| {
                let kv = m.load(k);
                m.ult(kv, six)
            },
            |m, l| {
                let kv = m.load(k);
                let c1 = m.const_u32(1);
                let pp = m.access(cam, &[c1, kv]);
                let p = m.load(pp);
                let mut d = Id(0);
                for n in 0..3 {
                    let pn = m.extract(p, &[n]);
                    let cn = m.extract(c, &[n]);
                    let prod = m.fmul(pn, cn);
                    d = if n == 0 { prod } else { m.fadd(d, prod) };
                }
                let pw = m.extract(p, &[3]);
                let dist = m.fadd(d, pw);
                let out = m.flt(dist, nr);
                m.if_then(out, |m| {
                    let no = m.const_bool(false);
                    m.store(inside_v, no);
                    m.branch(l.merge);
                });
            },
            |m| {
                let kv = m.load(k);
                let k1 = m.iadd(kv, one);
                m.store(k, k1);
            },
        );
        let seen = m.load(inside_v);
        m.if_then(seen, |m| {
            let pcount = m.access_c(draw, &[1]);
            let slot = m.atomic(op::AtomicIAdd, pcount, scope::Device, 0, one);
            let c0 = m.const_u32(0);
            let dst = m.access(vis, &[c0, slot]);
            m.store(dst, i);
        });
    });
    m.ret();
    m.end_function();
    finish_compute(m, main, WG_CULL)
}

/// cull_reset: draw = DrawCmd::reset() (one invocation; or vkCmdFillBuffer instead)
pub fn cull_reset_spv(minor: u32) -> Vec<u32> {
    let mut m = Module::new(minor);
    let t = ts(&mut m);
    let draw = draw_var(&mut m, t);
    let (main, _) = m.begin_function(t.void, &[]);
    m.name(main, "main");
    let vals = [CUBE_INDICES, 0, 0, 0, 0];
    for (k, &v) in vals.iter().enumerate() {
        let p = m.access_c(draw, &[k as u32]);
        let c = if k == 3 { m.const_i32(v as i32) } else { m.const_u32(v) };
        m.store(p, c);
    }
    m.ret();
    m.end_function();
    finish_compute(m, main, 1)
}

/// 2 (a * b + sign * c * d), the off-diagonal terms of the rotation matrix
fn twice_sum(m: &mut Module, a: Id, b: Id, c: Id, d: Id, minus: bool) -> Id {
    let ab = m.fmul(a, b);
    let cd = m.fmul(c, d);
    let s = if minus { m.fsub(ab, cd) } else { m.fadd(ab, cd) };
    m.fadd(s, s)
}

/// 1 - 2 (a a + b b), the diagonal terms
fn one_minus(m: &mut Module, a: Id, b: Id) -> Id {
    let aa = m.fmul(a, a);
    let bb = m.fmul(b, b);
    let s = m.fadd(aa, bb);
    let s2 = m.fadd(s, s);
    let one = m.const_f32(1.0);
    m.fsub(one, s2)
}

/// vs_instanced (instanced = true) / vs_single: the cube's vertex gl_VertexIndex of object
/// visible[gl_InstanceIndex] (or pc.object_index), out: Location 0 normal, 1 colour
fn vs_spv(minor: u32, instanced: bool) -> Vec<u32> {
    let mut m = Module::new(minor);
    let t = ts(&mut m);
    let objs = objects_var(&mut m, t, true);
    let cam = camera_var(&mut m, t);
    let cube = cube_var(&mut m, t);
    let vis = if instanced { visible_var(&mut m, t, true) } else { Id(0) };
    let pc = if instanced { Id(0) } else { push_var(&mut m, t) };
    let vi_var = m.builtin(Sc::Input, t.u, Bi::VertexIndex);
    m.name(vi_var, "gl_VertexIndex");
    let ii_var = if instanced {
        let v = m.builtin(Sc::Input, t.u, Bi::InstanceIndex);
        m.name(v, "gl_InstanceIndex");
        v
    } else {
        Id(0)
    };
    let pos_out = m.builtin(Sc::Output, t.v4, Bi::Position);
    m.name(pos_out, "gl_Position");
    let n_out = m.output(t.v3, 0);
    m.name(n_out, "out_normal");
    let c_out = m.output(t.v3, 1);
    m.name(c_out, "out_color");
    let (main, _) = m.begin_function(t.void, &[]);
    m.name(main, "main");
    let vi = m.load(vi_var);
    let obj = if instanced {
        let ii = m.load(ii_var);
        let c0 = m.const_u32(0);
        let p = m.access(vis, &[c0, ii]);
        m.load(p)
    } else {
        push_field(&mut m, pc, 4)
    };
    let pos = load_member(&mut m, objs, obj, 0);
    let scale = load_member(&mut m, objs, obj, 1);
    let rot = load_member(&mut m, objs, obj, 2);
    let color = load_member(&mut m, objs, obj, 3);
    let vp4 = load_member(&mut m, cube, vi, 0);
    let vn4 = load_member(&mut m, cube, vi, 1);
    let local = m.shuffle(vp4, vp4, &[0, 1, 2]);
    let normal = m.shuffle(vn4, vn4, &[0, 1, 2]);
    let q: Vec<Id> = (0..4).map(|n| m.extract(rot, &[n])).collect();
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    // rotation matrix columns of the unit quaternion (x, y, z, w)
    let c0 = [one_minus(&mut m, y, z), twice_sum(&mut m, x, y, w, z, false), twice_sum(&mut m, x, z, w, y, true)];
    let c1 = [twice_sum(&mut m, x, y, w, z, true), one_minus(&mut m, x, z), twice_sum(&mut m, y, z, w, x, false)];
    let c2 = [twice_sum(&mut m, x, z, w, y, false), twice_sum(&mut m, y, z, w, x, true), one_minus(&mut m, x, y)];
    let cols: Vec<Id> = [c0, c1, c2].iter().map(|c| m.construct(t.v3, c)).collect();
    let zero = m.const_f32(0.0);
    let one = m.const_f32(1.0);
    let mut mc = Vec::new();
    for &c in &cols {
        let s = m.vec_scale(c, scale);
        mc.push(m.construct(t.v4, &[s, zero]));
    }
    mc.push(m.construct(t.v4, &[pos, one]));
    let mat4 = m.ty_mat(t.v4, 4);
    let model = m.construct(mat4, &mc);
    let lp = m.construct(t.v4, &[local, one]);
    let world = m.mat_vec(model, lp);
    let vpp = m.access_c(cam, &[0]);
    let vp = m.load(vpp);
    let clip = m.mat_vec(vp, world);
    m.store(pos_out, clip);
    let mat3 = m.ty_mat(t.v3, 3);
    let rm = m.construct(mat3, &cols);
    let n = m.mat_vec(rm, normal);
    m.store(n_out, n);
    m.store(c_out, color);
    m.ret();
    m.end_function();
    m.entry_point(Model::Vertex, main, "main");
    m.finish()
}

pub fn vs_instanced_spv(minor: u32) -> Vec<u32> {
    vs_spv(minor, true)
}

pub fn vs_single_spv(minor: u32) -> Vec<u32> {
    vs_spv(minor, false)
}

/// fs_lambert: colour * (ambient + (1 - ambient) * max(dot(normalize(n), light), 0)),
/// in: Location 0 normal, 1 colour (Flat); out: Location 0 vec4 (alpha 1)
pub fn fs_lambert_spv(minor: u32) -> Vec<u32> {
    let mut m = Module::new(minor);
    let t = ts(&mut m);
    let cam = camera_var(&mut m, t);
    let n_in = m.input(t.v3, 0);
    m.name(n_in, "in_normal");
    let c_in = m.input(t.v3, 1);
    m.name(c_in, "in_color");
    m.decorate(c_in, Dec::Flat, &[]);
    let out = m.output(t.v4, 0);
    m.name(out, "out_color");
    let (main, _) = m.begin_function(t.void, &[]);
    m.name(main, "main");
    let n0 = m.load(n_in);
    let n = m.glsl(glsl::Normalize, &[n0]);
    let lp = m.access_c(cam, &[2]);
    let l = m.load(lp);
    let ap = m.access_c(cam, &[3]);
    let amb = m.load(ap);
    let d = m.dot(n, l);
    let zero = m.const_f32(0.0);
    let ndl = m.glsl(glsl::FMax, &[d, zero]);
    let one = m.const_f32(1.0);
    let rest = m.fsub(one, amb);
    let lit = m.fmul(rest, ndl);
    let k = m.fadd(amb, lit);
    let c = m.load(c_in);
    let rgb = m.vec_scale(c, k);
    let o = m.construct(t.v4, &[rgb, one]);
    m.store(out, o);
    m.ret();
    m.end_function();
    m.entry_point(Model::Fragment, main, "main");
    m.exec_mode(main, Mode::OriginUpperLeft, &[]);
    m.finish()
}
