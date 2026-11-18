// bridge -- the group "bridge": lib/game_render3d against render3d offscreen.  The reference is a scene
// built from scratch out of the world (every MeshRef at its interpolated Pose, no dirty tracking); the
// bridge's scene, kept up to date frame by frame, must draw the same image (0 pixels different) and must
// write only what changed.  Without Vulkan or a usable GPU: SKIP.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use game::{Entity, Pose, Quat, Transform, Vec3, View, World};
use game_render3d::{Bridge, MeshRef, transform};
use gpu::{Access, Buffer, BufferDesc, BufferUsage, ColorAttachment, DataLayout, DepthAttachment, Device, DeviceDesc, Format, GpuError, Layout, Level, Load,
          MemoryKind, Texture, TextureDesc, TextureRegion, TextureUsage};
use render3d::{Camera, Material, MaterialId, MeshData, MeshId, Renderer, RendererDesc, Scene};

use crate::{Ctx, Rng, out};

const W: u32 = 320;
const H: u32 = 180;
/// a far place for the origin test: a multiple of the origin's step (512), near 10^7
const FAR: f64 = 9_999_360.0;

/// an offscreen target with the read-back of its image (as render3dtest's)
struct Off {
    stride: u32,
    color: Texture,
    depth: Texture,
    rb: Buffer,
    used: bool,
}

impl Off {
    fn new(dev: &Device) -> gpu::Result<Off> {
        let stride = (W * 4).div_ceil(256) * 256;
        Ok(Off {
            stride,
            color: dev.create_texture(&TextureDesc::d2(W, H, Format::Rgba8Unorm, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC))?,
            depth: dev.create_texture(&TextureDesc::d2(W, H, Format::Depth32Float, TextureUsage::DEPTH_TARGET | TextureUsage::SAMPLED))?,
            rb: dev.create_buffer(&BufferDesc { size: stride as u64 * H as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?,
            used: false,
        })
    }

    /// one frame of the scene, waited for and read back (rows without padding)
    fn shot(&mut self, r: &mut Renderer, s: &mut Scene) -> gpu::Result<Vec<u8>> {
        s.prepare(r)?;
        let dev = r.device().clone();
        let mut e = dev.encoder()?;
        let color = ColorAttachment { view: self.color.view(), load: Load::Clear, clear: [0.2, 0.4, 0.6, 1.0], store: true,
                                      before: if self.used { Layout::CopySrc } else { Layout::Undefined }, after: Layout::CopySrc };
        let depth = DepthAttachment { view: self.depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: true,
                                      before: if self.used { Layout::Sampled } else { Layout::Undefined }, after: Layout::Sampled };
        r.record(&mut e, s, color, Some(depth))?;
        e.copy_texture_to_buffer(TextureRegion::mip(&self.color, 0), &self.rb, DataLayout { offset: 0, bytes_per_row: self.stride, rows_per_image: 0 });
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        let id = dev.queue().submit([e.finish()?])?;
        s.submitted(id);
        self.used = true;
        if !dev.wait(id, 10_000_000_000)? {
            return Err(GpuError::Invalid("the GPU did not finish within 10 s"));
        }
        let mut raw = vec![0u8; self.rb.size() as usize];
        self.rb.read(0, &mut raw)?;
        let mut img = Vec::with_capacity((W * H * 4) as usize);
        for y in 0..H as usize {
            img.extend_from_slice(&raw[y * self.stride as usize..y * self.stride as usize + (W * 4) as usize]);
        }
        Ok(img)
    }
}

/// pixels that differ
fn diff(a: &[u8], b: &[u8]) -> u32 {
    if a.len() != b.len() {
        return u32::MAX;
    }
    a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(p, q)| p != q).count() as u32
}

/// the cube -1 ..= 1, four vertices per face (render3dtest's)
fn cube() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    let (mut p, mut n, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (f, (nn, u, v)) in faces.iter().enumerate() {
        for (a, b) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            p.push([nn[0] + a * u[0] + b * v[0], nn[1] + a * u[1] + b * v[1], nn[2] + a * u[2] + b * v[2]]);
            n.push(*nn);
            uv.push([(a + 1.0) * 0.5, (b + 1.0) * 0.5]);
        }
        let f = f as u32 * 4;
        idx.extend_from_slice(&[f, f + 1, f + 2, f, f + 2, f + 3]);
    }
    (p, n, uv, idx)
}

/// a pyramid on a square base, 1 high (a second shape: a switched mesh shows)
fn pyramid() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) {
    let apex = [0.0f32, 1.0, 0.0];
    let base = [[-1.0f32, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, -1.0, 1.0], [-1.0, -1.0, 1.0]];
    let (mut p, mut n, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for k in 0..4 {
        let (a, b) = (base[(k + 1) % 4], base[k]);
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [apex[0] - a[0], apex[1] - a[1], apex[2] - a[2]];
        let c = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let l = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]) as f64;
        let l = physics::math::sqrt(l) as f32;
        let nn = [c[0] / l, c[1] / l, c[2] / l];
        let i = p.len() as u32;
        p.extend_from_slice(&[a, b, apex]);
        n.extend_from_slice(&[nn, nn, nn]);
        uv.extend_from_slice(&[[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]]);
        idx.extend_from_slice(&[i, i + 1, i + 2]);
    }
    let i = p.len() as u32;
    p.extend_from_slice(&base);
    n.extend_from_slice(&[[0.0, -1.0, 0.0]; 4]);
    uv.extend_from_slice(&[[0.0; 2]; 4]);
    idx.extend_from_slice(&[i, i + 1, i + 2, i, i + 2, i + 3]);
    (p, n, uv, idx)
}

/// what the tests draw with
struct Kit {
    meshes: Vec<MeshId>,
    materials: Vec<MaterialId>,
}

fn kit(r: &mut Renderer) -> gpu::Result<Kit> {
    let mut meshes = Vec::new();
    for (p, n, uv, idx) in [cube(), pyramid(), cube()] {
        meshes.push(r.add_mesh(&MeshData { positions: &p, normals: &n, uvs: &uv, indices: &idx })?);
    }
    let mut materials = Vec::new();
    for c in [[0.9, 0.3, 0.2], [0.2, 0.8, 0.3], [0.3, 0.4, 0.95], [0.9, 0.85, 0.3], [0.7, 0.7, 0.7]] {
        materials.push(r.add_material(&Material::color(c[0], c[1], c[2]))?);
    }
    Ok(Kit { meshes, materials })
}

/// a position on a grid of 1/1024 m (exact in f32 and f64, near 0 and at FAR)
fn q(v: f64) -> f64 {
    (v * 1024.0) as i64 as f64 / 1024.0
}

/// n entities with a Pose and a MeshRef in a slab in front of the camera, offset by `at`; a few without a
/// mesh, one without a pose
fn fill(w: &mut World, k: &Kit, n: usize, at: Vec3, seed: u64) -> Vec<Entity> {
    let mut r = Rng::new(seed);
    let mut out = Vec::new();
    for i in 0..n {
        let pos = Vec3::new(at.x + q(r.range(-14.0, 14.0)), at.y + q(r.range(-6.0, 6.0)), at.z + q(r.range(-30.0, 2.0)));
        let rot = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), r.range(0.0, 6.0)).mul(Quat::from_axis_angle(Vec3::X, r.range(-1.0, 1.0)));
        let s = r.range(0.3, 0.9) as f32;
        let t = Transform::new(pos, rot).scaled([s, s * 1.2, s]);
        let m = MeshRef { mesh: k.meshes[i % 2], material: k.materials[r.below(4)] };
        out.push(w.spawn_with((Pose::new(t), m)));
    }
    for i in 0..n / 8 {
        w.spawn_with((Pose::at(Vec3::new(i as f64, 0.0, 0.0)),));
    }
    out
}

fn camera_at(at: Vec3) -> Camera {
    let v = View::look_at(at + Vec3::new(3.0, 4.0, 18.0), at + Vec3::new(0.0, 0.0, -10.0), Vec3::Y).expect("view");
    game_render3d::camera(&v, 1.0, W as f64 / H as f64, 0.1, f64::INFINITY)
}

/// the reference: a scene built from scratch out of the world at alpha
fn reference(r: &Renderer, w: &World, alpha: f64, cam: &Camera, origin: [f64; 3]) -> gpu::Result<Scene> {
    let mut s = Scene::new(r)?;
    if let Some(meshes) = w.column::<MeshRef>() {
        for (e, m) in meshes.iter() {
            let t = w.get::<Pose>(e).map_or(Transform::IDENTITY, |p| p.interpolate(alpha));
            s.add(render3d::Instance { mesh: m.mesh, material: m.material, transform: transform(&t) })?;
        }
    }
    s.set_camera(cam);
    s.set_origin(origin);
    Ok(s)
}

/// the bridge's image against the reference's
fn same(c: &mut Ctx, off: &mut Off, r: &mut Renderer, s: &mut Scene, w: &World, alpha: f64, what: &str) -> Option<Vec<u8>> {
    let cam = *s.camera();
    let origin = s.origin();
    let a = match off.shot(r, s) {
        Ok(a) => a,
        Err(e) => {
            c.check(false, || format!("{what}: {e}"));
            return None;
        }
    };
    let b = reference(r, w, alpha, &cam, origin).and_then(|mut rs| off.shot(r, &mut rs));
    match b {
        Ok(b) => {
            let d = diff(&a, &b);
            let drawn = a.chunks_exact(4).filter(|p| p[..3] != [51, 102, 153]).count();
            c.check(d == 0 && drawn > 500, || format!("{what}: {d} pixels differ from the scene built anew ({drawn} drawn)"));
            Some(a)
        }
        Err(e) => {
            c.check(false, || format!("{what}: reference: {e}"));
            None
        }
    }
}

pub fn run(c: &mut Ctx) {
    let inst = match gpu::Instance::with(&gpu::InstanceDesc { app_name: "gametest", validate: false }) {
        Ok(i) => i,
        Err(e @ (GpuError::NoLoader | GpuError::NoAdapter | GpuError::Disabled)) => {
            out(&format!("  bridge: SKIP: {e}\n"));
            return;
        }
        Err(e) => {
            c.check(false, || format!("Instance::with: {e}"));
            return;
        }
    };
    let adapter = match inst.adapter() {
        Ok(a) => a,
        Err(e @ (GpuError::NoAdapter | GpuError::Unsupported(_))) => {
            out(&format!("  bridge: SKIP: {e}\n"));
            return;
        }
        Err(e) => {
            c.check(false, || format!("Instance::adapter: {e}"));
            return;
        }
    };
    // the adapter's own level, and a device limited to Basis (render3d's other way of drawing)
    let mut descs = vec![DeviceDesc { max_level: Level::Basis }];
    if adapter.level() == Level::Voll {
        descs.insert(0, DeviceDesc::DEFAULT);
    }
    for (i, d) in descs.iter().enumerate() {
        let res = adapter.create_device(d).and_then(|dev| {
            out(&format!("  bridge: {}, level {:?}\n", adapter.info().name, dev.level()));
            tests(c, &dev)?;
            if i == 0 { scale(c, &dev) } else { Ok(()) }
        });
        if let Err(e) = res {
            c.check(false, || format!("bridge: {e}"));
        }
    }
}

/// what a sync costs at scale: 100 000 entities with a mesh (10 000 with --quick), 1 % moving per tick (no
/// images: the CPU side of the bridge and render3d's bookkeeping)
fn scale(c: &mut Ctx, dev: &Device) -> gpu::Result<()> {
    let n = c.n(100_000);
    let mut r = Renderer::new(dev, &RendererDesc::new(Format::Rgba8Unorm))?;
    let k = kit(&mut r)?;
    let mut w = World::new();
    let mut ents = Vec::with_capacity(n);
    for i in 0..n {
        let p = Vec3::new((i % 400) as f64, 0.0, -((i / 400) as f64));
        ents.push(w.spawn_with((Pose::at(p), MeshRef { mesh: k.meshes[i % 2], material: k.materials[i % 4] })));
    }
    w.begin_tick();
    let mut s = Scene::new(&r)?;
    let mut b = Bridge::new();
    let t0 = plat::time_us();
    let st = b.sync(&w, &mut s, 0.0)?;
    let t1 = plat::time_us();
    let (mut moving_us, mut moved, mut still_us) = (0u64, 0u64, 0u64);
    let frames = 20u64;
    for f in 0..frames {
        w.begin_tick();
        for e in ents.iter().skip(f as usize % 100).step_by(100) {
            if let Some(mut p) = w.get_mut::<Pose>(*e) {
                p.now.pos.y += 0.25;
            }
        }
        let a = plat::time_us();
        moved += b.sync(&w, &mut s, 0.5)?.moved as u64;
        let m = plat::time_us();
        b.sync(&w, &mut s, 0.5)?;
        moving_us += m - a;
        still_us += plat::time_us() - m;
    }
    w.begin_tick();
    w.begin_tick();
    b.sync(&w, &mut s, 0.0)?;
    let a = plat::time_us();
    let idle = b.sync(&w, &mut s, 0.0)?;
    let idle_us = plat::time_us() - a;
    let per = (n as u64).div_ceil(100);
    c.check(st.added as usize == n && moved >= frames * (per - 1) && idle.visited == 0, || format!("scale: {st:?}, {moved} moved, idle {idle:?}"));
    out(&format!("  bridge: {n} entities: first sync {} ms; a frame with 1 % moved ({} written: the new movers and the settled ones) {} us, the same alpha again {} us, nothing changed {} us\n",
                 (t1 - t0) / 1000, moved / frames, moving_us / frames, still_us / frames, idle_us));
    Ok(())
}

fn tests(c: &mut Ctx, dev: &Device) -> gpu::Result<()> {
    let mut r = Renderer::new(dev, &RendererDesc::new(Format::Rgba8Unorm))?;
    r.set_size(W, H)?;
    let k = kit(&mut r)?;
    let mut off = Off::new(dev)?;
    let mut w = World::new();
    let ents = fill(&mut w, &k, 300, Vec3::ZERO, 11);
    let lone = w.spawn_with((MeshRef { mesh: k.meshes[0], material: k.materials[4] },));
    w.begin_tick();
    let mut s = Scene::new(&r)?;
    s.set_camera(&camera_at(Vec3::ZERO));
    let mut b = Bridge::new();

    // everything added once
    let st = b.sync(&w, &mut s, 0.0)?;
    c.check(st.added == 301 && st.removed == 0 && st.moved == 0 && b.len() == 301 && s.len() == 301, || format!("first sync: {st:?}"));
    c.check(b.instance(lone).is_some() && b.instance(Entity::NONE).is_none() && b.mesh_users(k.meshes[0]) == 151, || "instances, users".into());
    same(c, &mut off, &mut r, &mut s, &w, 0.0, "first sync");
    // nothing changed: nothing written, nothing uploaded
    let st = b.sync(&w, &mut s, 0.0)?;
    c.check(st == game_render3d::SyncStats::default(), || format!("a sync without change: {st:?}"));
    let _ = off.shot(&mut r, &mut s)?;
    c.check(s.upload_info().changed == 0, || format!("a frame without change uploads no instance: {:?}", s.upload_info()));

    // a tick moves five: written at the frame's alpha, every frame while between two states
    w.begin_tick();
    let movers: Vec<Entity> = ents.iter().step_by(60).copied().collect();
    for e in &movers {
        if let Some(mut p) = w.get_mut::<Pose>(*e) {
            p.now.pos.x += 1.0;
            p.now.rot = p.now.rot.mul(Quat::from_axis_angle(Vec3::Y, 0.5));
        }
    }
    let st = b.sync(&w, &mut s, 1.0)?;
    c.check(st.moved == 5 && st.visited == 5, || format!("five moved: {st:?}"));
    same(c, &mut off, &mut r, &mut s, &w, 1.0, "five moved, alpha 1");
    c.check(s.upload_info().changed == 5, || format!("five instances uploaded: {:?}", s.upload_info()));
    let st = b.sync(&w, &mut s, 0.25)?;
    c.check(st.moved == 5 && st.visited == 5, || format!("alpha 0.25: {st:?}"));
    same(c, &mut off, &mut r, &mut s, &w, 0.25, "alpha 0.25");
    let st = b.sync(&w, &mut s, 0.25)?;
    c.check(st.moved == 0 && st.visited == 5, || format!("the same alpha again writes nothing: {st:?}"));
    // the next tick without motion: one last write (now), then nothing
    w.begin_tick();
    let st = b.sync(&w, &mut s, 0.5)?;
    c.check(st.moved == 5 && st.visited == 5, || format!("settled: {st:?}"));
    same(c, &mut off, &mut r, &mut s, &w, 0.5, "settled");
    let st = b.sync(&w, &mut s, 0.75)?;
    c.check(st.moved == 0 && st.visited == 0, || format!("standing: {st:?}"));

    // gone: despawned, the MeshRef removed; a pose removed keeps the instance where it was
    for e in &ents[1..4] {
        w.despawn(*e);
    }
    w.remove::<MeshRef>(ents[10]);
    w.remove::<MeshRef>(ents[11]);
    w.remove::<Pose>(ents[12]);
    let before = b.instance(ents[12]).map(|i| s.transform(i));
    let st = b.sync(&w, &mut s, 0.0)?;
    c.check(st.removed == 5 && b.len() == 296 && s.len() == 296, || format!("five gone: {st:?}"));
    let kept = b.instance(ents[12]).map(|i| s.transform(i));
    c.check(kept.is_some() && kept == before, || "a pose removed: the instance stays where it was".into());
    w.insert(ents[12], *w.get::<Pose>(ents[13]).unwrap());
    b.sync(&w, &mut s, 0.0)?;
    same(c, &mut off, &mut r, &mut s, &w, 0.0, "after removals");

    // switched meshes and materials
    for e in &ents[20..24] {
        if let Some(mut m) = w.get_mut::<MeshRef>(*e) {
            m.mesh = k.meshes[2];
        }
    }
    for e in &ents[30..33] {
        if let Some(mut m) = w.get_mut::<MeshRef>(*e) {
            m.material = k.materials[4];
        }
    }
    let st = b.sync(&w, &mut s, 0.0)?;
    c.check(st.switched == 7 && st.added == 0 && st.removed == 0 && b.mesh_users(k.meshes[2]) == 4, || format!("switched: {st:?}"));
    same(c, &mut off, &mut r, &mut s, &w, 0.0, "switched");

    // a slot used again between two syncs: the old instance goes, the new one comes
    let old = ents[40];
    w.despawn(old);
    let new = w.spawn_with((Pose::at(Vec3::new(0.0, 0.0, -5.0)), MeshRef { mesh: k.meshes[1], material: k.materials[3] }));
    c.check(new.index() == old.index(), || "the slot came back".into());
    let st = b.sync(&w, &mut s, 0.0)?;
    c.check(st.removed == 1 && st.added == 1 && b.instance(old).is_none() && b.instance(new).is_some(), || format!("slot reused: {st:?}"));
    same(c, &mut off, &mut r, &mut s, &w, 0.0, "slot reused");

    // retire a mesh: it stays while instances use it, goes once they are gone
    b.retire_mesh(k.meshes[2]);
    b.retire_material(k.materials[4]);
    let n0 = (r.mesh_count(), r.material_count());
    c.check(b.collect(&mut r)? == 0 && b.retired() == (1, 1), || "retired, still in use".into());
    let users: Vec<Entity> = w.column::<MeshRef>().unwrap().iter().filter(|(_, m)| m.mesh == k.meshes[2] || m.material == k.materials[4]).map(|(e, _)| e).collect();
    for e in &users {
        w.remove::<MeshRef>(*e);
    }
    b.sync(&w, &mut s, 0.0)?;
    let gone = b.collect(&mut r)?;
    c.check(gone == 2 && b.retired() == (0, 0) && r.mesh_info(k.meshes[2]).is_none() && (r.mesh_count(), r.material_count()) == (n0.0 - 1, n0.1 - 1),
            || format!("collected {gone}: meshes {} materials {}", r.mesh_count(), r.material_count()));
    same(c, &mut off, &mut r, &mut s, &w, 0.0, "after collect");

    // the device went (GpuContent::detach): reset, a new scene, everything again
    let img = off.shot(&mut r, &mut s)?;
    drop(s);
    b.reset();
    let mut s = Scene::new(&r)?;
    s.set_camera(&camera_at(Vec3::ZERO));
    let st = b.sync(&w, &mut s, 0.0)?;
    let n = w.column::<MeshRef>().unwrap().len() as u32;
    c.check(st.added == n && s.len() == n, || format!("after reset: {st:?}"));
    let img2 = off.shot(&mut r, &mut s)?;
    c.check(diff(&img, &img2) == 0, || "a new scene after reset: the same image".into());
    b.clear(&mut s)?;
    c.check(s.is_empty() && b.is_empty(), || "clear".into());
    b.sync(&w, &mut s, 0.0)?;
    c.check(s.len() == n, || "after clear the next sync adds them again".into());
    drop(s);

    origin(c, &mut r, &mut off, &k)?;
    churn(c, &mut r, &mut off, &k)?;
    Ok(())
}

/// the origin follows the camera: the same world at 0 and near 10^7 draws the same image; without the
/// origin following, f32 cannot hold it
fn origin(c: &mut Ctx, r: &mut Renderer, off: &mut Off, k: &Kit) -> gpu::Result<()> {
    let mut b = Bridge::new();
    let mut s = Scene::new(r)?;
    c.check(!b.follow(&mut s, [100.0, 0.0, -300.0]) && s.origin() == [0.0; 3], || "within a step: no move".into());
    c.check(b.follow(&mut s, [600.0, 0.0, 0.0]) && s.origin() == [512.0, 0.0, 0.0], || format!("a step out: {:?}", s.origin()));
    c.check(b.follow(&mut s, [-1000.0, 3.0, 2000.0]) && s.origin() == [-1024.0, 0.0, 2048.0], || format!("snapped: {:?}", s.origin()));
    b.set_origin_step(0.5);
    c.check(b.origin_step() == 512.0, || "a step below 1 is refused".into());
    drop(s);
    let mut imgs = Vec::new();
    for (at, follow) in [(Vec3::ZERO, true), (Vec3::new(FAR, 0.0, FAR), true), (Vec3::new(FAR, 0.0, FAR), false)] {
        let mut w = World::new();
        fill(&mut w, k, 200, at, 23);
        w.begin_tick();
        let mut s = Scene::new(r)?;
        let cam = camera_at(at);
        s.set_camera(&cam);
        let mut b = Bridge::new();
        if follow {
            b.follow(&mut s, cam.eye);
        }
        b.sync(&w, &mut s, 0.0)?;
        imgs.push(off.shot(r, &mut s)?);
    }
    let (d1, d2) = (diff(&imgs[0], &imgs[1]), diff(&imgs[0], &imgs[2]));
    c.check(d1 == 0 && d2 > 1000, || format!("at 10^7 with the origin following: {d1} pixels differ; without: {d2}"));
    Ok(())
}

/// random frames: entities come and go, move, switch; after every sync the instances' transforms are the
/// world's, every 15 frames the image is the reference's
fn churn(c: &mut Ctx, r: &mut Renderer, off: &mut Off, k: &Kit) -> gpu::Result<()> {
    let mut w = World::new();
    let mut ents = fill(&mut w, k, c.n(3000).max(300), Vec3::ZERO, 31);
    let mut rng = Rng::new(99);
    let mut s = Scene::new(r)?;
    s.set_camera(&camera_at(Vec3::ZERO));
    let mut b = Bridge::new();
    let (mut bad, mut moved, mut visited, mut frames) = (0u32, 0u64, 0u64, 0u32);
    for f in 0..60 {
        if f % 2 == 0 {
            w.begin_tick();
            for _ in 0..rng.below(40) {
                let e = ents[rng.below(ents.len())];
                if let Some(mut p) = w.get_mut::<Pose>(e) {
                    p.now.pos.y += q(rng.range(-1.0, 1.0));
                }
            }
        }
        for _ in 0..rng.below(6) {
            let e = ents[rng.below(ents.len())];
            match rng.below(4) {
                0 => {
                    w.despawn(e);
                }
                1 => {
                    w.remove::<MeshRef>(e);
                }
                2 => {
                    if let Some(mut m) = w.get_mut::<MeshRef>(e) {
                        m.material = k.materials[rng.below(4)];
                    }
                }
                _ => {
                    let n = w.spawn_with((Pose::at(Vec3::new(q(rng.range(-10.0, 10.0)), 0.0, q(rng.range(-25.0, 0.0)))),
                                          MeshRef { mesh: k.meshes[rng.below(2)], material: k.materials[rng.below(4)] }));
                    ents.push(n);
                }
            }
        }
        let alpha = (f % 4) as f64 / 4.0;
        let st = b.sync(&w, &mut s, alpha)?;
        moved += st.moved as u64;
        visited += st.visited as u64;
        let meshes = w.column::<MeshRef>().unwrap();
        if b.len() as usize != meshes.len() || s.len() as usize != meshes.len() {
            bad += 1;
        }
        for (e, _) in meshes.iter() {
            let want = transform(&w.get::<Pose>(e).map_or(Transform::IDENTITY, |p| p.interpolate(alpha)));
            if b.instance(e).map(|i| s.transform(i)) != Some(Ok(want)) {
                bad += 1;
            }
        }
        if f % 15 == 14 {
            same(c, off, r, &mut s, &w, alpha, &format!("churn frame {f}"));
            frames += 1;
        }
    }
    let n = w.column::<Pose>().unwrap().len();
    c.check(bad == 0, || format!("churn: {bad} instances off"));
    out(&format!("  bridge: churn over 60 frames, {} poses, {moved} transforms written, {visited} poses looked at, {frames} images compared\n", n));
    Ok(())
}
