// common -- what the native tests and the browser's share: geometry, an offscreen target, a seeded
// generator, the scene every path must draw alike (gallery)

use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{Camera, Instance, InstanceId, Light, LightKind, Material, MaterialId, MeshData, MeshId, Renderer, RendererDesc, Scene, Sun, Transform};

use crate::cpu;

/// a mesh's data, owned
#[derive(Clone)]
pub struct Geo {
    pub pos: Vec<[f32; 3]>,
    pub nrm: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    pub idx: Vec<u32>,
}

impl Geo {
    pub fn data(&self) -> MeshData<'_> {
        MeshData { positions: &self.pos, normals: &self.nrm, uvs: &self.uv, indices: &self.idx }
    }
}

/// the cube -1 ..= 1, four vertices per face, normals per face
pub fn cube() -> Geo {
    // (normal, u, v) with u x v = normal: corners -u-v, u-v, u+v, -u+v run counter-clockwise from outside
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    let mut g = Geo { pos: Vec::new(), nrm: Vec::new(), uv: Vec::new(), idx: Vec::new() };
    for (f, (n, u, v)) in faces.iter().enumerate() {
        for (a, b) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            g.pos.push([n[0] + a * u[0] + b * v[0], n[1] + a * u[1] + b * v[1], n[2] + a * u[2] + b * v[2]]);
            g.nrm.push(*n);
            g.uv.push([(a + 1.0) * 0.5, (b + 1.0) * 0.5]);
        }
        let f = f as u32 * 4;
        g.idx.extend_from_slice(&[f, f + 1, f + 2, f, f + 2, f + 3]);
    }
    g
}

/// a square in the xy plane, -1 ..= 1, seen from +z
pub fn plate() -> Geo {
    Geo {
        pos: vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [1.0, 1.0, 0.0], [-1.0, 1.0, 0.0]],
        nrm: vec![[0.0, 0.0, 1.0]; 4],
        uv: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        idx: vec![0, 1, 2, 0, 2, 3],
    }
}

/// a prism with k sides around the y axis, radius 1, height 2 h: a shape per k, so that a wrong first
/// index or base vertex shows
pub fn prism(k: u32, h: f32) -> Geo {
    let mut g = Geo { pos: Vec::new(), nrm: Vec::new(), uv: Vec::new(), idx: Vec::new() };
    let at = |i: u32| {
        let (s, c) = cpu::sincos(i as f64 / k as f64 * core::f64::consts::TAU);
        (s as f32, c as f32)
    };
    for i in 0..k {
        let ((s0, c0), (s1, c1)) = (at(i), at(i + 1));
        let (mid_s, mid_c) = cpu::sincos((i as f64 + 0.5) / k as f64 * core::f64::consts::TAU);
        let n = [mid_s as f32, 0.0, mid_c as f32];
        let b = g.pos.len() as u32;
        // the side, counter-clockwise from outside (the angle grows from +z towards +x)
        g.pos.extend_from_slice(&[[s0, -h, c0], [s1, -h, c1], [s1, h, c1], [s0, h, c0]]);
        g.nrm.extend_from_slice(&[n; 4]);
        g.uv.extend_from_slice(&[[0.0; 2]; 4]);
        g.idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
        // the caps as fans around the middle
        let b = g.pos.len() as u32;
        g.pos.extend_from_slice(&[[0.0, h, 0.0], [s0, h, c0], [s1, h, c1], [0.0, -h, 0.0], [s1, -h, c1], [s0, -h, c0]]);
        g.nrm.extend_from_slice(&[[0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, -1.0, 0.0], [0.0, -1.0, 0.0]]);
        g.uv.extend_from_slice(&[[0.0; 2]; 6]);
        g.idx.extend_from_slice(&[b, b + 1, b + 2, b + 3, b + 4, b + 5]);
    }
    g
}

/// a ball of radius 1 with smooth normals
pub fn ball(stacks: u32, slices: u32) -> Geo {
    let mut g = Geo { pos: Vec::new(), nrm: Vec::new(), uv: Vec::new(), idx: Vec::new() };
    for j in 0..=stacks {
        let (sj, cj) = cpu::sincos(j as f64 / stacks as f64 * core::f64::consts::PI);
        for i in 0..=slices {
            let (si, ci) = cpu::sincos(i as f64 / slices as f64 * core::f64::consts::TAU);
            let p = [(sj * si) as f32, cj as f32, (sj * ci) as f32];
            g.pos.push(p);
            g.nrm.push(p);
            g.uv.push([i as f32 / slices as f32, j as f32 / stacks as f32]);
        }
    }
    let row = slices + 1;
    for j in 0..stacks {
        for i in 0..slices {
            let (a, b, c, d) = (j * row + i, j * row + i + 1, (j + 1) * row + i, (j + 1) * row + i + 1);
            g.idx.extend_from_slice(&[a, c, d, a, d, b]);
        }
    }
    g
}

/// the same mesh with its indices beyond 65535: unused vertices in front of it (a ray caster's time stays
/// where it was)
pub fn beyond_16_bits(g: Geo) -> Geo {
    const MORE: usize = 70_000;
    let mut out = Geo { pos: vec![g.pos[0]; MORE], nrm: vec![g.nrm[0]; MORE], uv: vec![g.uv[0]; MORE], idx: Vec::new() };
    out.pos.extend_from_slice(&g.pos);
    out.nrm.extend_from_slice(&g.nrm);
    out.uv.extend_from_slice(&g.uv);
    out.idx = g.idx.iter().map(|i| i + MORE as u32).collect();
    out
}

/// a quaternion: angle radians around a unit axis
pub fn quat(axis: [f64; 3], angle: f64) -> [f32; 4] {
    let (s, c) = cpu::sincos(angle * 0.5);
    [(axis[0] * s) as f32, (axis[1] * s) as f32, (axis[2] * s) as f32, c as f32]
}

/// a small generator with a fixed seed (the same numbers on every path)
pub struct Rng(pub u32);

impl Rng {
    pub fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
        let w = ((self.0 >> ((self.0 >> 28) + 4)) ^ self.0).wrapping_mul(277_803_737);
        (w >> 22) ^ w
    }

    /// in [0, 1), 24 bits: exact on every path
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 8) as f64 / 16_777_216.0
    }

    pub fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.unit()
    }

    pub fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

/// an offscreen target in one colour format, with the read-back of its image
pub struct Off {
    pub w: u32,
    pub h: u32,
    /// bytes per pixel
    pub bpp: u32,
    pub stride: u32,
    pub color: Texture,
    pub depth: Texture,
    pub rb: Buffer,
    used: bool,
}

pub const CLEAR: [f32; 4] = [0.2, 0.4, 0.6, 1.0];
pub const CLEAR8: [u8; 3] = [51, 102, 153];

/// a renderer whose 8-bit targets get the linear light, as the ray caster compares it (G5e2: RendererDesc::new has
/// the fragment shaders encode sRGB for an 8-bit target without sRGB; the test `srgb` checks that encoding)
pub fn linear_desc(color: Format) -> RendererDesc {
    let mut d = RendererDesc::new(color);
    d.encode_srgb = false;
    d
}

/// the same with n frames in flight (Rgba8Unorm)
pub fn frames_desc(n: usize) -> RendererDesc {
    let mut d = linear_desc(Format::Rgba8Unorm);
    d.frames_in_flight = n;
    d
}

impl Off {
    pub fn new(dev: &Device, w: u32, h: u32, format: Format) -> Result<Off> {
        let bpp = format.block_bytes();
        let stride = (w * bpp).div_ceil(256) * 256;
        Ok(Off {
            w,
            h,
            bpp,
            stride,
            color: dev.create_texture(&TextureDesc::d2(w, h, format, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC))?,
            depth: dev.create_texture(&TextureDesc::d2(w, h, Format::Depth32Float, TextureUsage::DEPTH_TARGET | TextureUsage::SAMPLED))?,
            rb: dev.create_buffer(&BufferDesc { size: stride as u64 * h as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?,
            used: false,
        })
    }

    /// one frame of the scene, with the image copied to the read-back
    pub fn submit(&mut self, r: &mut Renderer, s: &mut Scene) -> Result<SubmitId> {
        s.prepare(r)?;
        let dev = r.device().clone();
        let mut e = dev.encoder()?;
        let color = ColorAttachment { view: self.color.view(), load: Load::Clear, clear: CLEAR, store: true,
                                      before: if self.used { Layout::CopySrc } else { Layout::Undefined }, after: Layout::CopySrc };
        let depth = DepthAttachment { view: self.depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: true,
                                      before: if self.used { Layout::Sampled } else { Layout::Undefined }, after: Layout::Sampled };
        r.record(&mut e, s, color, Some(depth))?;
        e.copy_texture_to_buffer(TextureRegion::mip(&self.color, 0), &self.rb, DataLayout { offset: 0, bytes_per_row: self.stride, rows_per_image: 0 });
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        let id = dev.queue().submit([e.finish()?])?;
        s.submitted(id);
        self.used = true;
        Ok(id)
    }

    /// the image as rows without padding (RGBA8: 4 bytes a pixel; the browser: Err(NotReady) until it has answered)
    pub fn read(&self) -> Result<Vec<u8>> {
        let mut raw = vec![0u8; self.rb.size() as usize];
        self.rb.read(0, &mut raw)?;
        let mut out = Vec::with_capacity((self.w * self.h * self.bpp) as usize);
        for y in 0..self.h as usize {
            let at = y * self.stride as usize;
            out.extend_from_slice(&raw[at..at + (self.w * self.bpp) as usize]);
        }
        Ok(out)
    }
}

/// (pixels that differ, the largest difference of a channel)
pub fn diff(a: &[u8], b: &[u8]) -> (u32, u8) {
    let (mut n, mut m) = (0, 0u8);
    if a.len() != b.len() {
        return (u32::MAX, 255);
    }
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        if p != q {
            n += 1;
            for k in 0..4 {
                m = m.max(p[k].abs_diff(q[k]));
            }
        }
    }
    (n, m)
}

// ---------------------------------------------------------------- the gallery

/// Apple's GPUs (Metal): a derivative depends on the quad's invisible lanes, which the GPU fills differently from run to
/// run (uv and position bit-identical, dpdx/dpdy not; docs/APPLE.md AP6).  What picks levels and texels from the
/// derivatives (the textured gallery, normal maps) is held to a looser bound there
pub const DERIVATIVES_VARY: bool = cfg!(target_vendor = "apple");

/// (DERIVATIVES_VARY) pixels of a 320 x 180 image that may differ by more than the usual bound: 0.2 %
pub const DERIVATIVE_PIXELS: u32 = 320 * 180 / 500;

pub const GALLERY_W: u32 = 320;
pub const GALLERY_H: u32 = 180;
pub const GALLERY_N: u32 = 3000;

/// an instance as the tests describe it: for the engine and for the CPU's reference
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub mesh: u32,
    pub material: u32,
    pub pos: [f64; 3],
    pub rot: [f32; 4],
    pub scale: [f32; 3],
}

impl Spec {
    pub fn instance(&self) -> Instance {
        Instance { mesh: MeshId(self.mesh), material: MaterialId(self.material),
                   transform: Transform { pos: self.pos, rot: self.rot, scale: self.scale } }
    }
}

/// the meshes of the gallery: a cube, a ball, prisms of 3 to 48 sides, among them a mesh with 32-bit indices
pub fn gallery_geos() -> Vec<Geo> {
    let mut v = vec![cube(), ball(8, 12)];
    for k in 3..=48 {
        v.push(if k == 20 { beyond_16_bits(prism(k, 0.5 + k as f32 / 64.0)) } else { prism(k, 0.5 + k as f32 / 64.0) });
    }
    v
}

/// the colours of the gallery's materials
pub fn gallery_colors(n: u32) -> Vec<[f32; 3]> {
    let mut r = Rng(77);
    (0..n).map(|_| [r.range(0.25, 1.0) as f32, r.range(0.25, 1.0) as f32, r.range(0.25, 1.0) as f32]).collect()
}

/// n instances in a slab in front of the gallery's camera, at world position `at`
pub fn gallery_specs(meshes: u32, materials: u32, n: u32, at: [f64; 3]) -> Vec<Spec> {
    let mut r = Rng(4242);
    let mut v = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let axis = cpu::norm([r.range(-1.0, 1.0), r.range(-1.0, 1.0), r.range(-1.0, 1.0) + 0.01]);
        let s = r.range(0.2, 0.9) as f32;
        // (multiples of 1/1024: exact in f32 and f64, wherever the scene lies)
        let q = |r: &mut Rng, a: f64, b: f64| (r.range(a, b) * 1024.0) as i64 as f64 / 1024.0;
        v.push(Spec {
            mesh: r.below(meshes),
            material: r.below(materials),
            pos: [at[0] + q(&mut r, -60.0, 60.0), at[1] + q(&mut r, -30.0, 30.0), at[2] + q(&mut r, -70.0, 40.0)],
            rot: quat(axis, r.range(0.0, 6.0)),
            scale: [s, s * r.range(0.7, 1.3) as f32, s],
        });
    }
    v
}

pub fn gallery_camera(at: [f64; 3]) -> Camera {
    Camera::look_at([at[0] + 7.0, at[1] + 6.0, at[2] + 60.0], [at[0], at[1], at[2] - 10.0], [0.0, 1.0, 0.0], 1.0, GALLERY_W as f64 / GALLERY_H as f64, 0.1, f64::INFINITY)
}

/// the gallery's renderer contents
pub fn gallery_fill(r: &mut Renderer) -> Result<(u32, u32)> {
    let geos = gallery_geos();
    for g in &geos {
        r.add_mesh(&g.data())?;
    }
    let colors = gallery_colors(96);
    for c in &colors {
        r.add_material(&Material::color(c[0], c[1], c[2]))?;
    }
    Ok((geos.len() as u32, colors.len() as u32))
}

/// the gallery's scene at world position `at`, its origin there
pub fn gallery_scene(r: &Renderer, meshes: u32, materials: u32, at: [f64; 3]) -> Result<(Scene, Vec<InstanceId>)> {
    let mut s = Scene::new(r)?;
    s.set_origin(at);
    s.set_camera(&gallery_camera(at));
    s.set_sun(&Sun::DEFAULT);
    let mut ids = Vec::new();
    for spec in gallery_specs(meshes, materials, GALLERY_N, at) {
        ids.push(s.add(spec.instance())?);
    }
    Ok((s, ids))
}

/// the gallery's changes between its first and second image: moved, removed, added (their places used again)
pub fn gallery_change(s: &mut Scene, ids: &mut Vec<InstanceId>, meshes: u32, materials: u32) -> Result<()> {
    let mut r = Rng(99);
    for _ in 0..200 {
        let id = ids[r.below(ids.len() as u32) as usize];
        let mut t = s.transform(id)?;
        t.pos[0] += r.range(-3.0, 3.0);
        t.pos[1] += r.range(-3.0, 3.0);
        s.set_transform(id, &t)?;
    }
    for _ in 0..100 {
        let k = r.below(ids.len() as u32) as usize;
        s.remove(ids.swap_remove(k))?;
    }
    let extra = gallery_specs(meshes, materials, 50, [20.0, -5.0, 10.0]);
    for spec in extra {
        ids.push(s.add(spec.instance())?);
    }
    Ok(())
}

// ---------------------------------------------------------------- the textured gallery (G5b)

/// random texels
pub fn noise_rgba(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut r = Rng(seed);
    (0..w * h).flat_map(|_| { let v = r.next(); [v as u8, (v >> 8) as u8, (v >> 16) as u8, 255] }).collect()
}

/// a scene every path must draw alike with textures: seven textures of four classes (sRGB and linear, a
/// 4 x 4 one repeated, one of 100 x 60 resampled), every sampling mode and MASK, 600 instances of which a
/// third are mirrored
pub fn tex_gallery(r: &mut Renderer) -> Result<Scene> {
    use render3d::{Mip, Sampling, TextureData, TextureRef, Wrap};
    let geos = [cube(), ball(10, 16), prism(5, 0.6), plate()];
    let mut meshes = Vec::new();
    for g in &geos {
        meshes.push(r.add_mesh(&g.data())?);
    }
    let pics: [(u32, u32, Vec<u8>, bool, bool); 7] = [
        (128, 128, crate::gltfgen::picture(128, 128, 1, &|_, _| 255), true, false),
        (256, 256, crate::gltfgen::picture(256, 256, 2, &|_, _| 255), true, false),
        (512, 512, noise_rgba(512, 512, 3), false, false),
        (4, 4, noise_rgba(4, 4, 4), true, true),
        (100, 60, crate::gltfgen::picture(100, 60, 5, &|_, _| 255), true, false),
        (128, 128, crate::gltfgen::picture(128, 128, 6, &|x, y| if ((x / 8) + (y / 8)) % 3 == 0 { 30 } else { 230 }), true, true),
        (1024, 1024, crate::gltfgen::picture(1024, 1024, 7, &|_, _| 255), true, false),
    ];
    let mut tex = Vec::new();
    for p in &pics {
        tex.push(r.add_texture(&TextureData { width: p.0, height: p.1, rgba: &p.2, srgb: p.3, nearest: p.4 })?);
    }
    let s = |mag_nearest, min_nearest, mip, wrap_u, wrap_v| Sampling { mag_nearest, min_nearest, mip, wrap_u, wrap_v };
    let samplings = [Sampling::LINEAR, Sampling::NEAREST, s(false, false, Mip::Linear, Wrap::Clamp, Wrap::Mirror), s(true, false, Mip::Nearest, Wrap::Mirror, Wrap::Repeat),
                     s(false, true, Mip::None, Wrap::Repeat, Wrap::Clamp)];
    let colors = gallery_colors(12);
    let mut mats = Vec::new();
    for (i, c) in colors.iter().enumerate() {
        let mut m = Material::color(c[0] * 0.5 + 0.5, c[1] * 0.5 + 0.5, c[2] * 0.5 + 0.5);
        if i % 6 != 5 {
            m.base_color_texture = Some(TextureRef { texture: tex[i % 7], sampling: samplings[i % 5] });
        }
        if i % 7 == 5 {
            m.alpha_cutoff = Some(0.5);
        }
        mats.push(r.add_material(&m)?);
    }
    let mut sc = Scene::new(r)?;
    sc.set_camera(&gallery_camera([0.0; 3]));
    sc.set_sun(&Sun::DEFAULT);
    // close to the camera and large enough that the textures show
    let mut rng = Rng(555);
    for _ in 0..600 {
        let axis = cpu::norm([rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0) + 0.01]);
        let s = rng.range(0.8, 2.4) as f32;
        let q = |r: &mut Rng, a: f64, b: f64| (r.range(a, b) * 1024.0) as i64 as f64 / 1024.0;
        let pos = [q(&mut rng, -30.0, 30.0), q(&mut rng, -16.0, 20.0), q(&mut rng, -40.0, 25.0)];
        let mut scale = [s, s * rng.range(0.7, 1.3) as f32, s];
        if rng.below(3) == 0 {
            scale[rng.below(3) as usize] *= -1.0;
        }
        let (mesh, material) = (meshes[rng.below(geos.len() as u32) as usize], mats[rng.below(mats.len() as u32) as usize]);
        sc.add(Instance { mesh, material, transform: render3d::Transform { pos, rot: quat(axis, rng.range(0.0, 6.0)), scale } })?;
    }
    Ok(sc)
}

// ---------------------------------------------------------------- the galleries of G5c

/// the galleries that came with G5c (and G5d's "occl"), by name, after the five above: every path must draw them alike
pub const LATER: [&str; 5] = ["pbr", "shadow", "lights", "env", "occl"];

/// one of LATER in a fresh renderer of its own, of this colour format (frames drawn before the image go into a
/// target of it)
pub fn later_gallery_in(name: &str, r: &mut Renderer, format: Format) -> Result<Scene> {
    match name {
        "pbr" => pbr_gallery(r),
        "shadow" => shadow_gallery(r),
        "lights" => lights_gallery(r),
        "env" => env_gallery(r),
        "occl" => occl_gallery(r, format),
        "sides" => sides_gallery(r),
        "blend" => blend_gallery(r),
        "churn" => churn_gallery(r, format),
        "ortho" | "toon" | "outline" | "fxaa" | "particles" | "viewmodel" => g7f2_gallery(name, r, format),
        _ => Err(GpuError::Invalid("no such gallery")),
    }
}

/// a normal map from a height field of bumps: rgb = (n + 1) / 2 in glTF's tangent space (+x along u, +y up
/// in the image, i.e. towards decreasing v)
pub fn bump_map(size: u32, seed: u32) -> Vec<u8> {
    let mut r = Rng(seed);
    let bumps: Vec<(f64, f64, f64, f64)> = (0..12).map(|_| (r.range(0.0, 1.0), r.range(0.0, 1.0), r.range(0.05, 0.2), r.range(-0.6, 0.6))).collect();
    let height = |u: f64, v: f64| -> f64 {
        bumps.iter().map(|&(cu, cv, rad, k)| {
            let d2 = ((u - cu) * (u - cu) + (v - cv) * (v - cv)) / (rad * rad);
            if d2 < 1.0 { k * (1.0 - d2) * (1.0 - d2) * rad } else { 0.0 }
        }).sum()
    };
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let e = 1.0 / size as f64;
    for y in 0..size {
        for x in 0..size {
            let (u, v) = ((x as f64 + 0.5) * e, (y as f64 + 0.5) * e);
            let du = (height(u + e, v) - height(u - e, v)) / (2.0 * e);
            let dv = (height(u, v + e) - height(u, v - e)) / (2.0 * e);
            // up in the image is -v
            let n = cpu::norm([-du, dv, 1.0]);
            let c = |x: f64| ((x + 1.0) * 0.5 * 255.0 + 0.5) as u8;
            out.extend_from_slice(&[c(n[0]), c(n[1]), c(n[2]), 255]);
        }
    }
    out
}

/// (G5c) materials of every kind: balls dielectric and metallic from smooth to rough, and plates with a normal
/// map (trilinear), a metallic-roughness texture, emission, occlusion; the sun, the ambient light, an
/// exposure, the neutral tone mapper
pub fn pbr_gallery(r: &mut Renderer) -> Result<Scene> {
    use render3d::{Sampling, TextureData, TextureRef};
    let ball = r.add_mesh(&ball(16, 24).data())?;
    let plate = r.add_mesh(&plate().data())?;
    let picture = crate::gltfgen::picture(128, 128, 91, &|_, _| 255);
    let mut mr = noise_rgba(128, 128, 92);
    for p in mr.chunks_exact_mut(4) {
        p[1] = 64 + (p[1] as u32 * 191 / 255) as u8;
    }
    let pics: [(Vec<u8>, bool); 4] = [(picture, true), (bump_map(128, 93), false), (mr, false), (noise_rgba(128, 128, 94), false)];
    let mut tex = Vec::new();
    for (px, srgb) in &pics {
        tex.push(r.add_texture(&TextureData { width: 128, height: 128, rgba: px, srgb: *srgb, nearest: false })?);
    }
    let t = |k: usize, s: Sampling| Some(TextureRef { texture: tex[k], sampling: s });
    let colors = gallery_colors(10);
    let mut sc = Scene::new(r)?;
    sc.set_camera(&Camera::look_at([2.0, 3.0, 16.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 1.0, GALLERY_W as f64 / GALLERY_H as f64, 0.1, f64::INFINITY));
    sc.set_sun(&Sun { direction: [0.4, 0.7, 0.5], ..Sun::DEFAULT });
    sc.set_exposure(1.2);
    for i in 0..10u32 {
        let c = colors[i as usize];
        let m = Material { metallic: (i / 5) as f32, roughness: 0.2 + 0.2 * (i % 5) as f32, ..Material::color(c[0], c[1], c[2]) };
        let id = r.add_material(&m)?;
        let pos = [(i % 5) as f64 * 2.6 - 5.2, 2.6 - (i / 5) as f64 * 2.4, 0.0];
        sc.add(Instance { mesh: ball, material: id, transform: Transform { pos, rot: [0.0, 0.0, 0.0, 1.0], scale: [1.1; 3] } })?;
    }
    let plates = [
        Material { normal_texture: t(1, Sampling::LINEAR), base_color_texture: t(0, Sampling::LINEAR), roughness: 0.4, ..Material::color(1.0, 1.0, 1.0) },
        Material { metallic: 1.0, metallic_roughness_texture: t(2, Sampling::NEAREST), ..Material::color(0.9, 0.8, 0.5) },
        Material { emissive: [1.2, 0.6, 0.3], emissive_texture: t(0, Sampling::NEAREST), ..Material::color(0.2, 0.2, 0.2) },
        Material { occlusion_texture: t(3, Sampling::LINEAR), occlusion_strength: 0.8, ..Material::color(0.8, 0.8, 0.8) },
    ];
    for (k, m) in plates.iter().enumerate() {
        let id = r.add_material(m)?;
        let pos = [k as f64 * 3.4 - 5.1, -3.1, 0.5];
        sc.add(Instance { mesh: plate, material: id, transform: Transform { pos, rot: quat([1.0, 0.0, 0.0], -0.5), scale: [1.5, 1.2, 1.0] } })?;
    }
    Ok(sc)
}

/// no turn
const NO_TURN: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
pub const SHADOW_W: u32 = 400;
pub const SHADOW_H: u32 = 225;


/// (G5c) the shadow scene's meshes, textures (size, rgba, sRGB) and items at world offset `at`: a ground, cubes,
/// a ball, a pole, a mirrored cube, a MASK plate facing the sun
pub fn shadow_scene(at: [f64; 3]) -> (Vec<Geo>, Vec<(u32, Vec<u8>, bool)>, Vec<(Spec, Material)>) {
    let geos = vec![plate(), cube(), ball(16, 24), prism(8, 1.0)];
    // MASK: square holes, nearest without levels (the shadow pass's uv derivatives are not the camera's)
    // (white: inside the opaque squares a texel's edge changes nothing, at the holes the rays see what is behind)
    let holes: Vec<u8> = (0..128u32 * 128).flat_map(|i| { let (x, y) = (i % 128, i / 128); [255u8, 255, 255, if (x / 16 + y / 16) % 2 == 0 { 30 } else { 230 }] }).collect();
    let texs = vec![(128, holes, true)];
    let mask = render3d::Sampling { mag_nearest: true, min_nearest: true, mip: render3d::Mip::None, wrap_u: render3d::Wrap::Repeat, wrap_v: render3d::Wrap::Repeat };
    let flat = quat([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2);
    let p = |x: f64, y: f64, z: f64| [at[0] + x, at[1] + y, at[2] + z];
    let s = |mesh: u32, pos: [f64; 3], rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material: 0, pos, rot, scale };
    let m = |r: f32, g: f32, b: f32, metallic: f32, roughness: f32| Material { metallic, roughness, ..Material::color(r, g, b) };
    let mut items = vec![
        (s(0, p(0.0, 0.0, -25.0), flat, [30.0, 45.0, 1.0]), m(0.8, 0.8, 0.75, 0.0, 0.9)),
        (s(1, p(-3.0, 1.0, 0.0), quat([0.0, 1.0, 0.0], 0.4), [1.0; 3]), m(0.9, 0.3, 0.2, 0.0, 0.6)),
        (s(1, p(4.0, 1.5, -8.0), quat([0.0, 1.0, 0.0], 1.1), [1.5; 3]), m(0.2, 0.6, 0.9, 0.3, 0.5)),
        (s(1, p(-6.0, 2.0, -20.0), NO_TURN, [2.0; 3]), m(0.3, 0.8, 0.3, 0.0, 0.7)),
        (s(1, p(8.0, 3.0, -40.0), quat([0.0, 1.0, 0.0], 0.7), [3.0; 3]), m(0.9, 0.8, 0.2, 1.0, 0.5)),
        (s(1, p(-10.0, 2.5, -58.0), NO_TURN, [2.5; 3]), m(0.7, 0.4, 0.9, 0.0, 0.8)),
        (s(2, p(1.0, 1.2, 6.0), NO_TURN, [1.2; 3]), m(0.95, 0.9, 0.85, 0.0, 0.4)),
        (s(3, p(-1.0, 4.0, -14.0), NO_TURN, [0.4, 4.0, 0.4]), m(0.6, 0.6, 0.6, 0.0, 0.8)),
        (s(1, p(2.5, 0.8, -2.0), quat([0.0, 1.0, 0.0], 0.3), [-0.8, 0.8, 0.8]), m(0.9, 0.5, 0.7, 0.0, 0.6)),
    ];
    let mut holed = Material { base_color_texture: Some(render3d::TextureRef { texture: render3d::TextureId(0), sampling: mask }), alpha_cutoff: Some(0.5), ..m(0.9, 0.85, 0.7, 0.0, 0.8) };
    holed.roughness = 0.8;
    // facing the sun's way (the sun is at 0.6, 0.45, 0.35): its holes' shadow on the ground
    items.push((s(0, p(6.0, 2.6, -4.0), quat([0.0, 1.0, 0.0], 1.04), [2.0, 2.0, 1.0]), holed));
    (geos, texs, items)
}

pub fn shadow_sun() -> Sun {
    Sun { direction: [0.6, 0.45, 0.35], color: [0.7 * core::f32::consts::PI; 3], ambient: [0.3; 3] }
}

pub fn shadow_camera(at: [f64; 3]) -> Camera {
    Camera::look_at([at[0], at[1] + 7.0, at[2] + 16.0], [at[0], at[1], at[2] - 15.0], [0.0, 1.0, 0.0], 1.0, SHADOW_W as f64 / SHADOW_H as f64, 0.1, f64::INFINITY)
}

pub const SHADOWS: render3d::Shadows = render3d::Shadows { cascades: 4, size: 2048, distance: 90.0, split: 0.75, reach: 50.0 };


/// (G5c) the shadow scene with the sun's four cascades
pub fn shadow_gallery(r: &mut Renderer) -> Result<Scene> {
    shadow_gallery_with(r, &SHADOWS, GALLERY_W as f64 / GALLERY_H as f64)
}

/// (G5e3) the shadow scene with these shadows, for a target of this aspect (render3dtest --shadow-shots)
pub fn shadow_gallery_with(r: &mut Renderer, shadows: &render3d::Shadows, aspect: f64) -> Result<Scene> {
    use render3d::TextureData;
    let (geos, pics, items) = shadow_scene([0.0; 3]);
    let mut meshes = Vec::new();
    for g in &geos {
        meshes.push(r.add_mesh(&g.data())?);
    }
    for (n, px, srgb) in &pics {
        r.add_texture(&TextureData { width: *n, height: *n, rgba: px, srgb: *srgb, nearest: true })?;
    }
    let mut sc = Scene::new(r)?;
    sc.set_camera(&shadow_camera([0.0; 3]));
    sc.set_sun(&shadow_sun());
    sc.set_shadows(Some(shadows))?;
    let mut cam = shadow_camera([0.0; 3]);
    cam.aspect = aspect;
    sc.set_camera(&cam);
    for (spec, m) in &items {
        let id = r.add_material(m)?;
        sc.add(Instance { material: id, ..spec.instance() })?;
    }
    Ok(sc)
}

/// the scene's lights at world offset `at`: points with and without a range, spots narrow and wide
pub fn light_set(at: [f64; 3]) -> Vec<Light> {
    let p = |x: f64, y: f64, z: f64| [at[0] + x, at[1] + y, at[2] + z];
    vec![
        Light::point(p(-3.0, 1.5, 1.0), [1.0, 0.3, 0.2], 8.0, None),
        Light::point(p(2.0, 0.8, -1.0), [0.3, 1.0, 0.4], 6.0, Some(4.0)),
        Light { kind: LightKind::Spot { inner: 0.3, outer: 0.5 }, pos: p(0.0, 5.0, 2.0), direction: [0.0, -1.0, -0.2], color: [1.0, 1.0, 0.9], intensity: 40.0, range: Some(12.0) },
        Light { kind: LightKind::Spot { inner: 0.0, outer: 0.35 }, pos: p(4.0, 3.0, -3.0), direction: [-2.0, -3.0, 0.0], color: [0.3, 0.4, 1.0], intensity: 30.0, range: None },
    ]
}

/// a ground, balls and cubes under a dim sun
pub fn light_scene(at: [f64; 3]) -> (Vec<Geo>, Vec<(Spec, Material)>) {
    let geos = vec![plate(), ball(16, 24), cube()];
    let flat = quat([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2);
    let p = |x: f64, y: f64, z: f64| [at[0] + x, at[1] + y, at[2] + z];
    let s = |mesh: u32, pos: [f64; 3], rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material: 0, pos, rot, scale };
    let m = |r: f32, g: f32, b: f32, metallic: f32, roughness: f32| Material { metallic, roughness, ..Material::color(r, g, b) };
    let items = vec![
        (s(0, p(0.0, 0.0, -2.0), flat, [9.0, 7.0, 1.0]), m(0.8, 0.8, 0.8, 0.0, 0.8)),
        (s(1, p(-2.0, 0.9, 0.0), NO_TURN, [0.9; 3]), m(0.9, 0.9, 0.9, 0.0, 0.4)),
        (s(1, p(1.5, 0.7, -2.5), NO_TURN, [0.7; 3]), m(0.95, 0.8, 0.5, 1.0, 0.35)),
        (s(2, p(3.0, 0.6, 0.5), quat([0.0, 1.0, 0.0], 0.5), [0.6; 3]), m(0.4, 0.6, 0.9, 0.0, 0.6)),
        (s(2, p(-1.0, 0.5, -4.0), quat([0.0, 1.0, 0.0], 1.0), [0.5, 0.5, 0.5]), m(0.9, 0.5, 0.3, 0.2, 0.5)),
    ];
    (geos, items)
}

pub fn light_sun() -> Sun {
    Sun { direction: [0.2, 0.9, 0.3], color: [0.1 * core::f32::consts::PI; 3], ambient: [0.04; 3] }
}

pub fn light_camera(at: [f64; 3], aspect: f64) -> Camera {
    Camera::look_at([at[0], at[1] + 6.0, at[2] + 10.0], [at[0], at[1], at[2] - 2.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY)
}

/// (G5c) the light scene: point and spot lights under a dim sun, with the sun's shadows
pub fn lights_gallery(r: &mut Renderer) -> Result<Scene> {
    let (geos, items) = light_scene([0.0; 3]);
    for g in &geos {
        r.add_mesh(&g.data())?;
    }
    let mut sc = Scene::new(r)?;
    sc.set_camera(&light_camera([0.0; 3], GALLERY_W as f64 / GALLERY_H as f64));
    sc.set_sun(&light_sun());
    sc.set_lights(&light_set([0.0; 3]))?;
    sc.set_shadows(Some(&render3d::Shadows { cascades: 2, size: 1024, distance: 30.0, ..render3d::Shadows::DEFAULT }))?;
    for (spec, m) in &items {
        let id = r.add_material(m)?;
        sc.add(Instance { material: id, ..spec.instance() })?;
    }
    Ok(sc)
}

/// (G5c) balls dielectric and metallic, smooth to rough, a ground, in the procedural sky with the sun
pub fn env_gallery(r: &mut Renderer) -> Result<Scene> {
    let ball = r.add_mesh(&ball(20, 28).data())?;
    let plate = r.add_mesh(&plate().data())?;
    r.set_environment(&render3d::EnvironmentData::Sky(render3d::Sky::DEFAULT))?;
    let mut sc = Scene::new(r)?;
    sc.set_camera(&Camera::look_at([0.0, 2.0, 11.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 1.0, GALLERY_W as f64 / GALLERY_H as f64, 0.1, f64::INFINITY));
    sc.set_sun(&Sun { direction: [0.3, 0.8, 0.5], color: [2.0; 3], ambient: [0.0; 3] });
    sc.set_environment(Some(1.0));
    for i in 0..8u32 {
        let c = [[0.9f32, 0.9, 0.9], [0.9, 0.5, 0.3], [0.3, 0.8, 0.4], [0.5, 0.6, 0.95]][(i % 4) as usize];
        let m = r.add_material(&Material { metallic: (i / 4) as f32, roughness: (i % 4) as f32 / 3.0, ..Material::color(c[0], c[1], c[2]) })?;
        sc.add(Instance { mesh: ball, material: m, transform: Transform { pos: [(i % 4) as f64 * 2.5 - 3.75, 2.4 - (i / 4) as f64 * 2.5, 0.0], rot: [0.0, 0.0, 0.0, 1.0], scale: [1.1; 3] } })?;
    }
    let ground = r.add_material(&Material { roughness: 0.7, ..Material::color(0.6, 0.6, 0.6) })?;
    sc.add(Instance { mesh: plate, material: ground, transform: Transform { pos: [0.0, -1.5, 0.0], rot: quat([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2), scale: [9.0, 5.0, 1.0] } })?;
    Ok(sc)
}

/// (G5d) the occlusion scene (occl.rs, the gallery's "occl"): its meshes, cube, ball, prism, plate
pub fn occl_geos() -> Vec<Geo> {
    vec![cube(), ball(8, 12), prism(6, 0.6), plate()]
}

/// a ground, three rows of walls and n small instances (a third mirrored) at world offset `at`
pub fn occl_world(at: [f64; 3], n: u32, seed: u32) -> Vec<Spec> {
    let p = |x: f64, y: f64, z: f64| [at[0] + x, at[1] + y, at[2] + z];
    let none = [0.0f32, 0.0, 0.0, 1.0];
    let mut v = vec![Spec { mesh: 3, material: 0, pos: p(0.0, 0.0, -25.0), rot: quat([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2), scale: [45.0, 40.0, 1.0] }];
    let mut k = 1;
    for (z, xs) in [(-8.0, &[-14.0, 0.0, 14.0][..]), (-22.0, &[-21.0, -7.0, 7.0, 21.0][..]), (-38.0, &[-14.0, 0.0, 14.0][..])] {
        for &x in xs {
            v.push(Spec { mesh: 0, material: 1 + k % 5, pos: p(x, 3.0, z), rot: none, scale: [5.5, 3.0, 0.4] });
            k += 1;
        }
    }
    v.extend(occl_small(at, n, seed));
    v
}

/// n small instances among the walls
pub fn occl_small(at: [f64; 3], n: u32, seed: u32) -> Vec<Spec> {
    let mut r = Rng(seed);
    let q = |r: &mut Rng, a: f64, b: f64| (r.range(a, b) * 1024.0) as i64 as f64 / 1024.0;
    (0..n).map(|_| {
        let axis = cpu::norm([r.range(-1.0, 1.0), r.range(-1.0, 1.0), r.range(-1.0, 1.0) + 0.01]);
        let s = r.range(0.25, 0.7) as f32;
        let mut scale = [s, s * r.range(0.7, 1.3) as f32, s];
        if r.below(3) == 0 {
            scale[r.below(3) as usize] *= -1.0;
        }
        Spec { mesh: r.below(3), material: 6 + r.below(18), pos: [at[0] + q(&mut r, -35.0, 35.0), at[1] + q(&mut r, 0.3, 5.0), at[2] + q(&mut r, -55.0, 4.0)],
               rot: quat(axis, r.range(0.0, 6.0)), scale }
    }).collect()
}

/// the camera of frame f (a jump at frame 6), for a target of aspect a
pub fn occl_camera(at: [f64; 3], f: usize, a: f64) -> Camera {
    let ff = f as f64;
    let (eye, dir) = if f < 6 {
        ([cpu::sincos(ff * 0.3).0 * 6.0, 2.5 + 0.5 * cpu::sincos(ff * 0.2).0, 14.0 - ff * 0.8], [cpu::sincos(ff * 0.1).0 * 0.4, -0.08, -1.0])
    } else {
        ([25.0 - ff, 6.0, -48.0 + ff * 0.5], [-0.8, -0.15, 1.0])
    };
    let e = [at[0] + eye[0], at[1] + eye[1], at[2] + eye[2]];
    Camera::look_at(e, [e[0] + dir[0] * 10.0, e[1] + dir[1] * 10.0, e[2] + dir[2] * 10.0], [0.0, 1.0, 0.0], 1.0, a, 0.1, f64::INFINITY)
}


/// (G5d) the occlusion scene with detail levels after five frames of its flight, at the camera of the sixth (a jump: phase 2 of the
/// occlusion test draws much of it)
pub fn occl_gallery(r: &mut Renderer, format: Format) -> Result<Scene> {
    for g in &occl_geos() {
        r.add_mesh(&g.data())?;
    }
    for c in &gallery_colors(24) {
        r.add_material(&Material::color(c[0], c[1], c[2]))?;
    }
    // (detail levels too: far balls become prisms, the farthest are left out)
    r.set_lods(MeshId(1), &[render3d::Lod { mesh: Some(MeshId(2)), below: 0.03 }, render3d::Lod { mesh: None, below: 0.006 }])?;
    let mut sc = Scene::new(r)?;
    sc.set_sun(&Sun::DEFAULT);
    for sp in occl_world([0.0; 3], 2400, 77) {
        sc.add(sp.instance())?;
    }
    let aspect = GALLERY_W as f64 / GALLERY_H as f64;
    // the frames before: into a target without a read-back (the browser maps a read-back after the frame that fills it)
    let dev = r.device().clone();
    let color = dev.create_texture(&TextureDesc::d2(GALLERY_W, GALLERY_H, format, TextureUsage::COLOR_TARGET))?;
    let depth = dev.create_texture(&TextureDesc::d2(GALLERY_W, GALLERY_H, Format::Depth32Float, TextureUsage::DEPTH_TARGET | TextureUsage::SAMPLED))?;
    for f in 0..6 {
        sc.set_camera(&occl_camera([0.0; 3], f, aspect));
        sc.prepare(r)?;
        let mut e = dev.encoder()?;
        let first = f == 0;
        let ca = ColorAttachment { view: color.view(), load: Load::Clear, clear: CLEAR, store: true, before: if first { Layout::Undefined } else { Layout::ColorTarget },
                                   after: Layout::ColorTarget };
        let da = DepthAttachment { view: depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: false,
                                   before: if first { Layout::Undefined } else { Layout::DepthTarget }, after: Layout::DepthTarget };
        r.record(&mut e, &sc, ca, Some(da))?;
        let id = dev.queue().submit([e.finish()?])?;
        sc.submitted(id);
    }
    sc.set_camera(&occl_camera([0.0; 3], 6, aspect));
    Ok(sc)
}

/// the glTF test scene (gltfgen.rs) as a .glb, read and put on the GPU: its scene and the camera that sees it
pub fn gltf_gallery(r: &mut Renderer) -> Result<Scene> {
    let (glb, _) = crate::gltfgen::test_scene().write(crate::gltfgen::Form::Glb);
    let doc = gltf::load(&glb, &mut |_| None, &gltf::Limits::DEFAULT).map_err(|_| GpuError::Invalid("the glTF gallery does not load"))?;
    let imp = render3d::gltf::upload(r, &doc, None)?;
    let mut sc = Scene::new(r)?;
    sc.set_camera(&crate::gltfgen::test_camera(GALLERY_W as f64 / GALLERY_H as f64));
    sc.set_sun(&Sun::DEFAULT);
    for i in &imp.instances {
        sc.add(*i)?;
    }
    Ok(sc)
}

// ---------------------------------------------------------------- the galleries of G7f

/// white, square holes: alpha 30 in a checker of 16 texels, 230 elsewhere (alphatest.rs's)
pub fn holes_rgba() -> Vec<u8> {
    (0..128u32 * 128).flat_map(|i| { let (x, y) = (i % 128, i / 128); [255u8, 255, 255, if (x / 16 + y / 16) % 2 == 0 { 30 } else { 230 }] }).collect()
}

/// (G7f) the galleries of G7f, after LATER: double-sided materials, blended instances, a renderer after removals.  Not in
/// LATER: the test `srgb` compares LATER's images in an 8-bit target the shader encodes with their float images, and
/// blending happens in the target's codes (docs/RENDER3D.md "Durchsichtiges")
pub const G7F: [&str; 3] = ["sides", "blend", "churn"];

/// the cube without its +z face: an open box, its inside seen from +z
pub fn cup() -> Geo {
    let mut g = cube();
    // (the faces of cube(): +x, -x, +y, -y, +z, -z, six indices each)
    g.idx.drain(24..30);
    g
}

/// a texture (srgb) of 128 texels: white, alpha rising from 20 to 235 across, a dark square in each corner (nearest)
pub fn alpha_ramp() -> Vec<u8> {
    (0..128u32 * 128).flat_map(|i| {
        let (x, y) = (i % 128, i / 128);
        let corner = (x < 16 || x >= 112) && (y < 16 || y >= 112);
        let c = if corner { 40u8 } else { 255 };
        [c, c, if corner { 200 } else { 255 }, (20 + x * 215 / 127) as u8]
    }).collect()
}

/// nearest, without levels (exact against the ray caster)
pub const NEAREST_ONLY: render3d::Sampling = render3d::Sampling { mag_nearest: true, min_nearest: true, mip: render3d::Mip::None, wrap_u: render3d::Wrap::Repeat,
                                                                  wrap_v: render3d::Wrap::Repeat };

/// what a G7f scene holds: meshes, textures (size, rgba, sRGB; exact class sizes, nearest), items (the material's
/// textures are indices into the list)
pub type Described = (Vec<Geo>, Vec<(u32, Vec<u8>, bool)>, Vec<(Spec, render3d::MaterialDesc)>);

fn desc(m: Material, blend: bool, double: bool) -> render3d::MaterialDesc {
    let mut d = render3d::MaterialDesc::new(m);
    d.blend = blend;
    d.double_sided = double;
    d
}

const TURNED: [f32; 4] = [0.0, 1.0, 0.0, 0.0];

/// (G7f) plates seen from behind (single-sided: culled; double-sided: lit from their side, also MASK, mirrored, with a
/// normal map), open boxes seen into (double-sided and not), before a plate; meshes plate, cup
pub fn sides_scene() -> Described {
    let geos = vec![plate(), cup()];
    let texs = vec![(128, holes_rgba(), true), (128, bump_map(128, 71), false)];
    let tex = |k: u32| Some(render3d::TextureRef { texture: render3d::TextureId(k), sampling: NEAREST_ONLY });
    let s = |mesh: u32, pos: [f64; 3], rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material: 0, pos, rot, scale };
    let m = |r: f32, g: f32, b: f32| Material { roughness: 0.6, ..Material::color(r, g, b) };
    let into = quat(cpu::norm([1.0, 0.4, 0.0]), 0.5);
    let items = vec![
        (s(0, [0.0, 0.0, -4.0], [0.0, 0.0, 0.0, 1.0], [9.0, 5.0, 1.0]), desc(m(0.55, 0.55, 0.6), false, false)),
        (s(0, [-4.5, 1.6, 0.0], TURNED, [1.3; 3]), desc(m(0.9, 0.3, 0.2), false, false)),
        (s(0, [-1.5, 1.6, 0.0], TURNED, [1.3; 3]), desc(m(0.3, 0.9, 0.3), false, true)),
        (s(0, [1.5, 1.6, 0.0], TURNED, [1.3; 3]), desc(Material { base_color_texture: tex(0), alpha_cutoff: Some(0.5), ..m(0.9, 0.8, 0.3) }, false, true)),
        (s(0, [4.5, 1.6, 0.0], TURNED, [-1.3, 1.3, 1.3]), desc(m(0.3, 0.4, 0.95), false, true)),
        (s(1, [-3.5, -1.6, 1.0], into, [1.1; 3]), desc(m(0.9, 0.7, 0.4), false, true)),
        (s(1, [0.0, -1.6, 1.0], into, [1.1; 3]), desc(m(0.6, 0.9, 0.8), false, false)),
        (s(0, [3.5, -1.6, 1.0], quat([0.0, 1.0, 0.0], 3.0), [1.3; 3]), desc(Material { normal_texture: tex(1), roughness: 0.4, ..m(0.9, 0.9, 0.9) }, false, true)),
    ];
    (geos, texs, items)
}

pub fn sides_camera(aspect: f64) -> Camera {
    Camera::look_at([0.5, 1.0, 10.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY)
}

/// (G7f) opaque ground, wall and a cube; blended plates at several depths, added out of their order, overlapping; a
/// blended double-sided cube (front and back faces blended in their order), a mirrored single-sided one, a plate whose
/// texture's alpha rises, one over the clear colour, one behind the wall; meshes plate, cube
pub fn blend_scene() -> Described {
    let geos = vec![plate(), cube()];
    let texs = vec![(128, alpha_ramp(), true)];
    let s = |mesh: u32, pos: [f64; 3], rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material: 0, pos, rot, scale };
    let none = [0.0f32, 0.0, 0.0, 1.0];
    let flat = quat([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2);
    let m = |r: f32, g: f32, b: f32, a: f32| Material { base_color: [r, g, b, a], roughness: 0.5, ..Material::color(r, g, b) };
    let items = vec![
        // opaque: the ground, the wall, a cube before the nearest plate
        (s(0, [0.0, -2.0, -5.0], flat, [12.0, 9.0, 1.0]), desc(m(0.7, 0.7, 0.65, 1.0), false, false)),
        (s(0, [0.0, 1.0, -12.0], none, [12.0, 3.0, 1.0]), desc(m(0.5, 0.55, 0.7, 1.0), false, false)),
        (s(1, [-1.2, -0.6, 1.5], quat([0.0, 1.0, 0.0], 0.5), [0.5; 3]), desc(m(0.9, 0.9, 0.3, 1.0), false, false)),
        // blended, out of their order
        (s(0, [0.0, 1.0, -6.0], none, [2.5, 2.0, 1.0]), desc(m(0.1, 0.2, 0.8, 0.4), true, false)),
        (s(1, [-4.5, 0.5, -1.0], quat([0.0, 1.0, 0.0], 0.5), [-1.0, 1.0, 1.0]), desc(m(0.3, 0.8, 0.8, 0.5), true, false)),
        (s(0, [-2.0, 0.0, 0.0], none, [2.0, 1.5, 1.0]), desc(m(0.8, 0.15, 0.1, 0.5), true, false)),
        (s(0, [2.6, 2.4, -1.0], quat([0.0, 1.0, 0.0], -0.3), [1.5; 3]), desc(Material { base_color_texture: Some(render3d::TextureRef { texture: render3d::TextureId(0), sampling: NEAREST_ONLY }),
                                                                                         ..m(1.0, 0.9, 0.8, 1.0) }, true, false)),
        // (small: its sphere's rectangle stays inside the wall at the level of the pyramid the test reads)
        (s(0, [0.0, 1.0, -15.0], none, [0.8, 0.6, 1.0]), desc(m(1.0, 0.2, 0.9, 0.7), true, false)),
        (s(1, [3.0, 0.3, -2.5], quat(cpu::norm([0.3, 1.0, 0.2]), 0.7), [1.0; 3]), desc(m(0.95, 0.6, 0.2, 0.35), true, true)),
        (s(0, [-1.0, 0.5, -3.0], none, [2.0, 1.5, 1.0]), desc(m(0.1, 0.65, 0.15, 0.6), true, false)),
        (s(0, [0.5, 5.2, -8.0], none, [3.5, 1.0, 1.0]), desc(m(0.95, 0.9, 0.2, 0.5), true, false)),
    ];
    (geos, texs, items)
}

pub fn blend_camera(aspect: f64) -> Camera {
    Camera::look_at([0.5, 2.0, 12.0], [0.0, 0.5, -4.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY)
}

/// a described scene into a renderer (textures, meshes, a material per item): the scene and its ids
pub fn described_in(r: &mut Renderer, d: &Described, cam: &Camera, sun: &Sun) -> Result<(Scene, Vec<InstanceId>)> {
    let (geos, texs, items) = d;
    let mut meshes = Vec::new();
    for g in geos {
        meshes.push(r.add_mesh(&g.data())?);
    }
    let mut tex = Vec::new();
    for (n, px, srgb) in texs {
        tex.push(r.add_texture(&render3d::TextureData { width: *n, height: *n, rgba: px, srgb: *srgb, nearest: true })?);
    }
    let mut sc = Scene::new(r)?;
    sc.set_camera(cam);
    sc.set_sun(sun);
    let mut ids = Vec::new();
    for (spec, m) in items {
        let mut m = *m;
        // (the items name textures by their index in the list)
        for t in [&mut m.material.base_color_texture, &mut m.material.metallic_roughness_texture, &mut m.material.normal_texture, &mut m.material.occlusion_texture,
                  &mut m.material.emissive_texture].into_iter().flatten() {
            t.texture = tex[t.texture.0 as usize];
        }
        let id = r.add_material_desc(&m)?;
        ids.push(sc.add(Instance { mesh: meshes[spec.mesh as usize], material: id, ..spec.instance() })?);
    }
    Ok((sc, ids))
}

/// (G7f) the scene of double-sided materials
pub fn sides_gallery(r: &mut Renderer) -> Result<Scene> {
    Ok(described_in(r, &sides_scene(), &sides_camera(GALLERY_W as f64 / GALLERY_H as f64), &Sun::DEFAULT)?.0)
}

/// (G7f) the scene of blended instances
pub fn blend_gallery(r: &mut Renderer) -> Result<Scene> {
    Ok(described_in(r, &blend_scene(), &blend_camera(GALLERY_W as f64 / GALLERY_H as f64), &Sun::DEFAULT)?.0)
}

/// (G7f) the churn: what a renderer holds after removals and new meshes, textures and materials in their places (two
/// frames drawn before, into a target without a read-back); the scene must look as the same contents put into a fresh
/// renderer (test `remove`): the shadow scene with its MASK plate and four textured, blended plates
pub fn churn_gallery(r: &mut Renderer, format: Format) -> Result<Scene> {
    // first: other meshes, textures and materials, and a scene with them
    let junk = [ball(6, 8), prism(7, 0.4), cube(), prism(30, 0.9)];
    let mut old_meshes = Vec::new();
    for g in &junk {
        old_meshes.push(r.add_mesh(&g.data())?);
    }
    let mut old_tex = Vec::new();
    for k in 0..3 {
        old_tex.push(r.add_texture(&render3d::TextureData { width: 128, height: 128, rgba: &noise_rgba(128, 128, 600 + k), srgb: k != 1, nearest: false })?);
    }
    let mut old_mats = Vec::new();
    for k in 0..5u32 {
        let t = render3d::TextureRef { texture: old_tex[(k % 3) as usize], sampling: render3d::Sampling::LINEAR };
        old_mats.push(r.add_material(&Material::textured(t))?);
    }
    let mut sc = Scene::new(r)?;
    sc.set_camera(&shadow_camera([0.0; 3]));
    sc.set_sun(&shadow_sun());
    let mut old = Vec::new();
    for k in 0..40u32 {
        let pos = [(k % 8) as f64 * 2.0 - 7.0, 1.0 + (k / 8) as f64, -10.0];
        old.push(sc.add(Instance { mesh: old_meshes[(k % 4) as usize], material: old_mats[(k % 5) as usize], transform: Transform::at(pos) })?);
    }
    let dev = r.device().clone();
    let color = dev.create_texture(&TextureDesc::d2(GALLERY_W, GALLERY_H, format, TextureUsage::COLOR_TARGET))?;
    let depth = dev.create_texture(&TextureDesc::d2(GALLERY_W, GALLERY_H, Format::Depth32Float, TextureUsage::DEPTH_TARGET | TextureUsage::SAMPLED))?;
    let frame = |r: &mut Renderer, sc: &mut Scene, first: bool| -> Result<()> {
        sc.prepare(r)?;
        let mut e = dev.encoder()?;
        let ca = ColorAttachment { view: color.view(), load: Load::Clear, clear: CLEAR, store: true, before: if first { Layout::Undefined } else { Layout::ColorTarget },
                                   after: Layout::ColorTarget };
        let da = DepthAttachment { view: depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: false,
                                   before: if first { Layout::Undefined } else { Layout::DepthTarget }, after: Layout::DepthTarget };
        r.record(&mut e, sc, ca, Some(da))?;
        let id = dev.queue().submit([e.finish()?])?;
        sc.submitted(id);
        Ok(())
    };
    frame(r, &mut sc, true)?;
    frame(r, &mut sc, false)?;
    // then everything of it goes: instances, materials (their textures lose them), textures, meshes; in a mixed order
    for id in old {
        sc.remove(id)?;
    }
    for &m in old_mats.iter().rev() {
        r.remove_material(m)?;
    }
    r.remove_mesh(old_meshes[1])?;
    r.remove_texture(old_tex[1])?;
    r.remove_mesh(old_meshes[3])?;
    r.remove_texture(old_tex[0])?;
    r.remove_mesh(old_meshes[0])?;
    r.remove_mesh(old_meshes[2])?;
    r.remove_texture(old_tex[2])?;
    // the shadow scene in the places they left (its texture, meshes and materials), and blended plates
    churn_final(r, &mut sc)?;
    Ok(sc)
}

/// (G7f) what the churn ends with, in a fresh renderer (test `remove`: the same image)
pub fn churn_fresh(r: &mut Renderer) -> Result<Scene> {
    let mut sc = Scene::new(r)?;
    churn_final(r, &mut sc)?;
    Ok(sc)
}

/// the churn's last contents: the shadow scene with two cascades, four blended plates (two double-sided)
fn churn_final(r: &mut Renderer, sc: &mut Scene) -> Result<()> {
    let (geos, pics, items) = shadow_scene([0.0; 3]);
    let mut tex = Vec::new();
    for (n, px, srgb) in &pics {
        tex.push(r.add_texture(&render3d::TextureData { width: *n, height: *n, rgba: px, srgb: *srgb, nearest: true })?);
    }
    let ramp = r.add_texture(&render3d::TextureData { width: 128, height: 128, rgba: &alpha_ramp(), srgb: true, nearest: true })?;
    let mut meshes = Vec::new();
    for g in &geos {
        meshes.push(r.add_mesh(&g.data())?);
    }
    sc.set_camera(&shadow_camera([0.0; 3]));
    sc.set_sun(&shadow_sun());
    sc.set_shadows(Some(&render3d::Shadows { cascades: 2, size: 1024, distance: 60.0, ..render3d::Shadows::DEFAULT }))?;
    for (spec, m) in &items {
        let mut m = *m;
        if let Some(t) = &mut m.base_color_texture {
            t.texture = tex[t.texture.0 as usize];
        }
        let id = r.add_material(&m)?;
        sc.add(Instance { mesh: meshes[spec.mesh as usize], material: id, ..spec.instance() })?;
    }
    for k in 0..4u32 {
        let mut d = render3d::MaterialDesc::new(Material { base_color: [0.4 + 0.15 * k as f32, 0.5, 0.9 - 0.2 * k as f32, 0.7],
                                                           base_color_texture: Some(render3d::TextureRef { texture: ramp, sampling: NEAREST_ONLY }),
                                                           ..Material::color(1.0, 1.0, 1.0) });
        d.blend = true;
        d.double_sided = k % 2 == 1;
        let id = r.add_material_desc(&d)?;
        let pos = [k as f64 * 2.4 - 3.6, 2.2, 3.0 - k as f64 * 1.5];
        sc.add(Instance { mesh: meshes[0], material: id, transform: Transform { pos, rot: quat([0.0, 1.0, 0.0], 0.3 * k as f64 - 0.4), scale: [1.0, 0.8, 1.0] } })?;
    }
    Ok(())
}

// ---------------------------------------------------------------- (G7f part 2) the arena

/// (G7f part 2) the galleries of G7f's second part: the arena seen orthographically, in the cartoon look, with outlines,
/// through FXAA, with particles, with a viewmodel
pub const G7F2: [&str; 6] = ["ortho", "toon", "outline", "fxaa", "particles", "viewmodel"];

/// (G7f part 2) a small arena with two figures of plain shapes: ground, walls, a crate, a ramp, a pillar, a ball, two figures
/// (body, head, legs, arms); meshes plate, cube, ball, prism; untextured.  toon: every material in that look
pub fn arena_scene(toon: Option<render3d::Toon>) -> Described {
    let geos = vec![plate(), cube(), ball(12, 16), prism(10, 1.0)];
    let s = |mesh: u32, pos: [f64; 3], rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material: 0, pos, rot, scale };
    let none = [0.0f32, 0.0, 0.0, 1.0];
    let flat = quat([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2);
    let m = |r: f32, g: f32, b: f32| {
        let mut d = render3d::MaterialDesc::new(Material { roughness: 0.7, ..Material::color(r, g, b) });
        d.toon = toon;
        d
    };
    let mut items = vec![
        (s(0, [0.0, 0.0, 0.0], flat, [9.0, 9.0, 1.0]), m(0.45, 0.65, 0.35)),
        (s(1, [0.0, 0.6, -8.5], none, [9.0, 0.6, 0.5]), m(0.7, 0.68, 0.6)),
        (s(1, [-8.5, 0.6, 0.0], none, [0.5, 0.6, 8.0]), m(0.7, 0.68, 0.6)),
        (s(1, [8.5, 0.6, 0.0], none, [0.5, 0.6, 8.0]), m(0.7, 0.68, 0.6)),
        (s(1, [3.0, 0.6, -2.5], quat([0.0, 1.0, 0.0], 0.4), [0.6; 3]), m(0.85, 0.55, 0.2)),
        (s(1, [-4.0, 0.45, -3.5], quat([1.0, 0.0, 0.0], -0.35), [1.5, 0.15, 2.0]), m(0.6, 0.6, 0.75)),
        (s(3, [5.0, 1.5, -5.0], none, [0.5, 1.5, 0.5]), m(0.62, 0.62, 0.66)),
        (s(2, [2.2, 0.5, 2.0], none, [0.5; 3]), m(0.95, 0.85, 0.2)),
    ];
    // two figures: body, head, legs, arms
    for (at, turn, color) in [([0.0, 0.0, 0.0], 0.3, [0.85, 0.2, 0.15]), ([-2.6, 0.0, 1.4], -0.8, [0.2, 0.35, 0.85])] {
        let q = quat([0.0, 1.0, 0.0], turn);
        let rot = |p: [f64; 3]| {
            let (sn, cs) = cpu::sincos(turn);
            [at[0] + p[0] * cs + p[2] * sn, at[1] + p[1], at[2] - p[0] * sn + p[2] * cs]
        };
        items.push((s(3, rot([0.0, 0.95, 0.0]), q, [0.42, 0.45, 0.3]), m(color[0], color[1], color[2])));
        items.push((s(2, rot([0.0, 1.72, 0.0]), q, [0.3; 3]), m(0.95, 0.78, 0.62)));
        for side in [-1.0, 1.0] {
            items.push((s(1, rot([0.17 * side, 0.27, 0.0]), q, [0.11, 0.25, 0.12]), m(0.2, 0.2, 0.3)));
            items.push((s(1, rot([0.55 * side, 1.0, 0.0]), q, [0.09, 0.33, 0.09]), m(color[0], color[1], color[2])));
        }
    }
    (geos, Vec::new(), items)
}

pub fn arena_camera(aspect: f64) -> Camera {
    Camera::look_at([6.5, 4.5, 9.0], [0.0, 0.9, -1.0], [0.0, 1.0, 0.0], 0.9, aspect, 0.1, f64::INFINITY)
}

/// the arena's orthographic camera from the front and above (an editor's view)
pub fn arena_ortho(aspect: f64) -> Camera {
    Camera::ortho_at([8.0, 9.0, 14.0], [0.0, 0.5, -1.0], [0.0, 1.0, 0.0], 11.0, aspect, 0.0, 40.0)
}

/// the toon variants of the comparison sheet and the gallery: (name, look, outline)
pub fn toon_variants() -> [(&'static str, render3d::Toon, Option<render3d::Outline>); 4] {
    let t = |bands: u32, softness: f32, rim: f32, rim_width: f32| {
        let mut k = render3d::Toon::new(bands);
        k.softness = softness;
        k.rim = rim;
        k.rim_width = rim_width;
        k
    };
    let o = |w: u32, c: [f32; 4], crease: f32| {
        let mut x = render3d::Outline::new(w, c);
        x.crease = crease;
        x
    };
    [("A: 2 bands, hard, no rim, 1 px dark lines", t(2, 0.0, 0.0, 0.0), Some(o(1, [0.04, 0.03, 0.03, 1.0], 1.0))),
     ("B: 3 bands, soft 0.15, rim 0.35 at 0.25, 2 px dark lines", t(3, 0.15, 0.35, 0.25), Some(o(2, [0.05, 0.04, 0.06, 1.0], 1.0))),
     ("C: 4 bands, soft 0.5, rim 0.2 at 0.35, no lines", t(4, 0.5, 0.2, 0.35), None),
     ("D: 2 bands, soft 0.05, rim 0.6 at 0.2, 2 px brown lines on silhouettes", t(2, 0.05, 0.6, 0.2), Some(o(2, [0.18, 0.09, 0.03, 1.0], 8.0)))]
}

/// (G7f part 2) an emitter of the galleries: (seed, additive, streak, life, speed, spread, size, colours, gravity, drag)
#[allow(clippy::too_many_arguments)]
pub fn emitter(seed: u32, additive: bool, streak: bool, life: [f32; 2], speed: [f32; 2], spread: f32, size: [f32; 2], color: [[f32; 4]; 2], gravity: f32,
               drag: f32) -> render3d::Emitter {
    let mut e = render3d::Emitter::new(seed);
    e.blend = if additive { render3d::ParticleBlend::Additive } else { render3d::ParticleBlend::Alpha };
    e.shape = if streak { render3d::ParticleShape::Streak } else { render3d::ParticleShape::Disc };
    e.stretch = if streak { 0.04 } else { 0.0 };
    e.life = life;
    e.speed = speed;
    e.spread = spread;
    e.size = size;
    e.color = color;
    e.gravity = gravity;
    e.drag = drag;
    e
}

/// (G7f part 2) the viewmodel of the galleries: a weapon of boxes and a barrel in the camera's space (meshes of the arena:
/// cube 1, prism 3) and their colours
pub fn weapon() -> Vec<(Spec, [f32; 3])> {
    let s = |mesh: u32, pos: [f64; 3], rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material: 0, pos, rot, scale };
    let along = quat([1.0, 0.0, 0.0], core::f64::consts::FRAC_PI_2);
    vec![(s(1, [0.22, -0.2, -0.45], [0.0, 0.0, 0.0, 1.0], [0.05, 0.06, 0.18]), [0.25, 0.25, 0.28]),
         (s(3, [0.22, -0.16, -0.75], along, [0.022, 0.16, 0.022]), [0.15, 0.15, 0.17]),
         (s(1, [0.22, -0.3, -0.38], quat([1.0, 0.0, 0.0], 0.3), [0.035, 0.09, 0.05]), [0.45, 0.28, 0.15])]
}

pub fn weapon_lens() -> render3d::Viewmodel {
    render3d::Viewmodel::new(0.95, 0.02, 4.0)
}

/// a described scene into a renderer (described_in), with the weapon as its viewmodel (materials of its own, the scene's
/// look)
pub fn arena_in(r: &mut Renderer, d: &Described, cam: &Camera, sun: &Sun, vm: bool) -> Result<(Scene, Vec<InstanceId>)> {
    let (mut sc, ids) = described_in(r, d, cam, sun)?;
    if vm {
        sc.set_viewmodel(Some(&weapon_lens()))?;
        for (spec, c) in weapon() {
            let mut m = render3d::MaterialDesc::new(Material { roughness: 0.45, metallic: 0.3, ..Material::color(c[0], c[1], c[2]) });
            m.toon = d.2[0].1.toon;
            let id = r.add_material_desc(&m)?;
            sc.add_viewmodel(Instance { mesh: MeshId(spec.mesh), material: id, ..spec.instance() })?;
        }
    }
    Ok((sc, ids))
}

/// (G7f part 2) the particles of the galleries at time t: sparks from a hit on the crate, smoke over the pillar, a muzzle
/// flash at the weapon (in the viewmodel's space when there is one)
pub fn arena_particles(sc: &mut Scene, vm: bool, t: f64) -> Result<()> {
    sc.set_particles(4096)?;
    let sparks = sc.add_emitter(&emitter(11, true, true, [0.3, 0.7], [2.0, 5.0], 1.2, [0.05, 0.02], [[6.0, 3.0, 0.8, 1.0], [2.0, 0.4, 0.05, 1.0]], 9.8, 0.5))?;
    let smoke = sc.add_emitter(&emitter(12, false, false, [1.5, 3.0], [0.3, 0.8], 0.6, [0.4, 1.6], [[0.55, 0.55, 0.58, 0.7], [0.7, 0.7, 0.72, 0.0]], -0.6, 0.8))?;
    let mut f = emitter(13, true, false, [0.08, 0.12], [0.5, 1.5], 0.5, [0.12, 0.25], [[8.0, 5.0, 1.5, 1.0], [3.0, 1.0, 0.2, 0.0]], 0.0, 2.0);
    f.viewmodel = vm;
    let flash = sc.add_emitter(&f)?;
    sc.set_time(t);
    sc.emit(sparks, [3.0, 1.2, -1.85], [0.2, 0.5, 1.0], 60)?;
    sc.emit(smoke, [5.0, 3.1, -5.0], [0.0, 1.0, 0.0], 40)?;
    sc.emit(flash, if vm { [0.22, -0.16, -0.93] } else { [0.0, 1.5, 2.0] }, [0.0, 0.0, -1.0], 12)?;
    sc.set_time(t + 0.06);
    Ok(())
}

/// the camera of the viewmodel's gallery: close to the right wall along it, the weapon (on the right) reaching into it
pub fn wall_camera(aspect: f64) -> Camera {
    Camera::look_at([7.93, 1.6, 2.0], [6.75, 1.4, -4.0], [0.0, 1.0, 0.0], 1.1, aspect, 0.05, f64::INFINITY)
}

/// one of G7F2's galleries in a fresh renderer of this format (FXAA takes no 32-bit float target: "fxaa" is drawn without it
/// there)
pub fn g7f2_gallery(name: &str, r: &mut Renderer, format: Format) -> Result<Scene> {
    let aspect = GALLERY_W as f64 / GALLERY_H as f64;
    let sun = Sun::DEFAULT;
    let toon = toon_variants()[1].1;
    let sh = render3d::Shadows { cascades: 2, size: 1024, distance: 40.0, ..render3d::Shadows::DEFAULT };
    Ok(match name {
        "ortho" => {
            let (mut sc, _) = arena_in(r, &arena_scene(None), &arena_ortho(aspect), &sun, false)?;
            sc.set_shadows(Some(&sh))?;
            sc
        }
        "toon" => {
            let (mut sc, _) = arena_in(r, &arena_scene(Some(toon)), &arena_camera(aspect), &sun, false)?;
            sc.set_shadows(Some(&sh))?;
            sc
        }
        "outline" => {
            let (mut sc, _) = arena_in(r, &arena_scene(Some(toon)), &arena_camera(aspect), &sun, false)?;
            sc.set_outline(toon_variants()[1].2.as_ref())?;
            sc
        }
        "fxaa" => {
            let (mut sc, _) = arena_in(r, &arena_scene(None), &arena_camera(aspect), &sun, false)?;
            if !matches!(format, Format::Rgba32Float | Format::Rg32Float | Format::R32Float) {
                sc.set_antialias(render3d::Antialias::Fxaa);
            }
            sc
        }
        "particles" => {
            let (mut sc, _) = arena_in(r, &arena_scene(None), &arena_camera(aspect), &sun, false)?;
            arena_particles(&mut sc, false, 1.0)?;
            sc
        }
        _ => {
            let (mut sc, _) = arena_in(r, &arena_scene(None), &wall_camera(aspect), &sun, true)?;
            arena_particles(&mut sc, true, 1.0)?;
            sc
        }
    })
}
