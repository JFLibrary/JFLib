// common -- what the native tests and the browser's share: geometry, an offscreen target, a seeded
// generator, the scene every path must draw alike (gallery)

use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{Camera, Instance, InstanceId, Light, LightKind, Material, MaterialId, MeshData, MeshId, Renderer, Scene, Sun, Transform};

use crate::cpu;

/// a mesh's data, owned
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

/// the galleries that came with G5c, by name, after the five above: every path must draw them alike
pub const LATER: [&str; 3] = ["pbr", "shadow", "lights"];

/// one of LATER in a fresh renderer of its own
pub fn later_gallery(name: &str, r: &mut Renderer) -> Result<Scene> {
    match name {
        "pbr" => pbr_gallery(r),
        "shadow" => shadow_gallery(r),
        "lights" => lights_gallery(r),
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
    sc.set_shadows(Some(&SHADOWS))?;
    let mut cam = shadow_camera([0.0; 3]);
    cam.aspect = GALLERY_W as f64 / GALLERY_H as f64;
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
