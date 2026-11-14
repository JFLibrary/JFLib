// tests -- render3dtest's tests on Vulkan (main.rs lists them)

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{Camera, InstanceId, Material, MaterialId, MeshId, Renderer, RendererDesc, Scene, Sun, Transform};

use crate::common::*;
use crate::cpu::{self, RefCamera, RefScene};

/// the test state
pub struct T {
    pub fails: u32,
    pub validate: bool,
}

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
    plat::write_stdout(b"\n");
}

impl T {
    pub fn ok(&mut self, cond: bool, what: &str) -> bool {
        if cond {
            out(&format!("  ok   {what}"));
        } else {
            self.fails += 1;
            out(&format!("FAIL {what}"));
        }
        cond
    }

    pub fn res<V>(&mut self, r: gpu::Result<V>, what: &str) -> Option<V> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.ok(false, &format!("{what}: {e}"));
                None
            }
        }
    }
}

/// instance and adapter of a run
pub struct Gpu {
    pub inst: Instance,
    pub adapter: Adapter,
}

/// None (SKIP, not a failure) without Vulkan or a usable GPU
pub fn setup(t: &mut T) -> Option<Gpu> {
    let inst = match Instance::with(&InstanceDesc { app_name: "render3dtest", validate: t.validate }) {
        Ok(i) => i,
        Err(e @ (GpuError::NoLoader | GpuError::NoAdapter | GpuError::Disabled)) => {
            out(&format!("SKIP: {e}"));
            return None;
        }
        Err(e) => {
            t.ok(false, &format!("Instance::with: {e}"));
            return None;
        }
    };
    if t.validate && !inst.validating() {
        out("note: validation layer not available, running without it");
    }
    let adapter = match inst.adapter() {
        Ok(a) => a,
        Err(e @ (GpuError::NoAdapter | GpuError::Unsupported(_))) => {
            out(&format!("SKIP: {e}"));
            return None;
        }
        Err(e) => {
            t.ok(false, &format!("Instance::adapter: {e}"));
            return None;
        }
    };
    Some(Gpu { inst, adapter })
}

/// a test on the adapter's own level and on a device limited to Basis
pub fn on_levels(t: &mut T, f: &mut dyn FnMut(&mut T, &Device, &str)) {
    let Some(g) = setup(t) else { return };
    if g.adapter.level() == Level::Voll {
        if let Some(d) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") {
            f(t, &d, "Voll");
        }
    }
    if let Some(d) = t.res(g.adapter.create_device(&DeviceDesc { max_level: Level::Basis }), "Basis device") {
        f(t, &d, "Basis");
    }
}

/// submit, wait, read the image
pub fn shot(t: &mut T, dev: &Device, off: &mut Off, r: &mut Renderer, s: &mut Scene, what: &str) -> Option<Vec<u8>> {
    let id = t.res(off.submit(r, s), what)?;
    if !t.res(dev.wait(id, 10_000_000_000), what)? {
        t.ok(false, &format!("{what}: the GPU did not finish within 10 s"));
        return None;
    }
    t.res(off.read(), what)
}

/// the depth buffer through a small compute shader (lib/gpu refuses copies of depth textures)
pub fn read_depth(t: &mut T, dev: &Device, off: &Off) -> Option<Vec<f32>> {
    const SRC: &str = "
        @group(0) @binding(0) var tex: texture_depth_2d;
        @group(0) @binding(1) var<storage, read_write> out: array<f32>;
        @compute @workgroup_size(16, 16)
        fn main(@builtin(global_invocation_id) id: vec3u) {
            let d = textureDimensions(tex);
            if id.x < d.x && id.y < d.y {
                out[id.y * d.x + id.x] = textureLoad(tex, id.xy, 0);
            }
        }";
    let opt = wgsl::CompileOptions { robust: false, ..wgsl::CompileOptions::new(shader::BASIS) };
    let words = match wgsl::compile(SRC, &opt) {
        Ok(o) => o.entries.into_iter().next()?.words,
        Err(e) => {
            t.ok(false, &format!("depth reader: {:?}", e.first().map(|x| format!("{x}"))));
            return None;
        }
    };
    let m = t.res(ShaderModule::from_spirv(dev, &words), "depth reader")?;
    let bgl = t.res(dev.create_bind_group_layout(&[
        Binding { binding: 0, kind: BindingKind::Texture, stages: Stages::COMPUTE },
        Binding { binding: 1, kind: BindingKind::StorageBuffer, stages: Stages::COMPUTE },
    ]), "depth reader")?;
    let pl = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 }), "depth reader")?;
    let pipe = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pl, shader: ShaderStage::new(&m, "main") }), "depth reader")?;
    let size = (off.w * off.h * 4) as u64;
    let buf = t.res(dev.create_buffer(&BufferDesc { size, usage: BufferUsage::STORAGE, memory: MemoryKind::Device }), "depth reader")?;
    let rb = t.res(dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "depth reader")?;
    let group = t.res(dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[
        BindEntry { binding: 0, resource: Resource::Texture(off.depth.view()) },
        BindEntry { binding: 1, resource: Resource::Buffer { buffer: &buf, offset: 0, size: 0 } },
    ] }), "depth reader")?;
    let mut e = t.res(dev.encoder(), "depth reader")?;
    e.set_compute_pipeline(&pipe);
    e.set_bind_group(0, &group);
    e.dispatch(off.w.div_ceil(16), off.h.div_ceil(16), 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    e.copy_buffer(&buf, 0, &rb, 0, size);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    let cmd = t.res(e.finish(), "depth reader")?;
    let id = t.res(dev.queue().submit([cmd]), "depth reader")?;
    t.res(dev.wait(id, 10_000_000_000), "depth reader")?;
    let mut d = vec![0f32; (off.w * off.h) as usize];
    t.res(rb.read(0, &mut d), "depth reader")?;
    Some(d)
}

pub fn sun_of() -> Sun {
    // (0.7 pi lux: a white matte face towards the sun shows 0.7 + 0.3 before the tone mapper)
    Sun { direction: [0.3, 0.8, 0.5], color: [0.7 * core::f32::consts::PI; 3], ambient: [0.3, 0.3, 0.3] }
}

/// meshes, materials (one colour per spec), a scene and its reference
struct Built {
    r: Renderer,
    s: Scene,
    rs: RefScene,
}

fn build(t: &mut T, dev: &Device, geos: &[Geo], specs: &[Spec], colors: &[[f32; 3]], cam: &Camera) -> Option<Built> {
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    for g in geos {
        t.res(r.add_mesh(&g.data()), "add_mesh")?;
    }
    for c in colors {
        t.res(r.add_material(&Material::color(c[0], c[1], c[2])), "add_material")?;
    }
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let sun = sun_of();
    s.set_sun(&sun);
    s.set_camera(cam);
    let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
    for spec in specs {
        t.res(s.add(spec.instance()), "Scene::add")?;
        rs.add(&geos[spec.mesh as usize], spec, colors[spec.material as usize]);
    }
    Some(Built { r, s, rs })
}

pub fn ref_camera(c: &Camera) -> RefCamera {
    RefCamera::new(c.eye, c.forward, c.up, c.fov_y, c.aspect, c.near, c.far)
}

fn check_image(t: &mut T, b: &Built, cam: &Camera, off: &Off, img: &[u8], depth: Option<&[f32]>, what: &str) {
    let c = cpu::compare(&b.rs, &ref_camera(cam), off.w, off.h, CLEAR8, img, depth);
    t.ok(c.fine(), &format!("{what}: {}", c.text()));
    if let Some((x, y, _, _)) = c.first {
        let rc = ref_camera(cam);
        for yy in y.saturating_sub(2)..(y + 3).min(off.h) {
            let mut line = String::new();
            for xx in x.saturating_sub(3)..(x + 4).min(off.w) {
                let hit = b.rs.trace(rc.eye, rc.ray(xx as f64 + 0.5, yy as f64 + 0.5, off.w, off.h), rc.near, rc.far);
                line.push_str(&format!("{:?}/{:?}  ", pixel(img, off.w, xx, yy), hit.map(|v| v.rgb)));
            }
            out(&format!("    {line}"));
        }
    }
    if depth.is_some() {
        t.ok(c.depth_bad == 0, &format!("{what}: depth buffer, largest relative error {:.2e} at {} pixels", c.depth_err, c.solid_hit));
    }
}

pub fn pixel_of(img: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    pixel(img, w, x, y)
}

fn pixel(img: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let a = ((y * w + x) * 4) as usize;
    [img[a], img[a + 1], img[a + 2]]
}

fn pixels(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = pixels_on(t, dev, level); });
}

fn pixels_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (160u32, 90u32);
    let aspect = w as f64 / h as f64;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let (geos, spin) = (vec![cube(), ball(16, 24), plate()], quat(cpu::norm([1.0, 2.0, 0.5]), 0.9));
    let at = |mesh: u32, material: u32, pos: [f64; 3], rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material, pos, rot, scale };
    let none = [0.0f32, 0.0, 0.0, 1.0];
    let colors = [[1.0f32, 0.5, 0.25], [0.25, 1.0, 0.5], [0.5, 0.25, 1.0], [1.0, 1.0, 1.0]];
    let far_cam = Camera::look_at([0.0, 0.0, 6.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);

    // a turned cube: orientation, colours, lighting per face
    let cam = far_cam;
    let mut b = build(t, dev, &geos, &[at(0, 0, [0.0; 3], spin, [1.0; 3])], &colors, &cam)?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "a cube")?;
    let d = read_depth(t, dev, &off)?;
    check_image(t, &b, &cam, &off, &img, Some(&d), &format!("{level}: a turned cube"));

    // the same pair in both orders: the depth test decides, not the order of the list
    let cam = Camera::look_at([1.0, 1.0, 8.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    let pair = [at(0, 0, [-0.5, 0.0, 2.0], spin, [1.0; 3]), at(0, 1, [0.5, 0.25, 0.0], none, [1.5; 3])];
    for (name, list) in [("near one last", [pair[1], pair[0]]), ("near one first", pair)] {
        let mut b = build(t, dev, &geos, &list, &colors, &cam)?;
        let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "a pair")?;
        let d = read_depth(t, dev, &off)?;
        check_image(t, &b, &cam, &off, &img, Some(&d), &format!("{level}: two cubes, {name}"));
    }

    // a ball scaled unevenly: the normals of the inverse transpose, interpolated
    let cam = Camera::look_at([0.0, 1.0, 7.0], [0.0; 3], [0.0, 1.0, 0.0], 0.9, aspect, 0.5, 100.0);
    let mut b = build(t, dev, &geos, &[at(1, 2, [0.25, 0.0, 0.0], spin, [2.5, 1.0, 0.5])], &colors, &cam)?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "a ball")?;
    let d = read_depth(t, dev, &off)?;
    check_image(t, &b, &cam, &off, &img, Some(&d), &format!("{level}: a ball scaled 2.5 x 1 x 0.5, near 0.5, far 100"));

    // from inside a cube: its faces look away, nothing is drawn; through the far plane: cut off
    let cam = Camera::look_at([0.0; 3], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    let mut b = build(t, dev, &geos, &[at(0, 3, [0.0; 3], none, [3.0; 3])], &colors, &cam)?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "inside a cube")?;
    t.ok(img.chunks_exact(4).all(|p| p[0..3] == CLEAR8), &format!("{level}: from inside a cube: back faces culled, only the clear colour"));

    // far plane: the plate behind it stays away, the one before it shows
    let cam = Camera::look_at([0.0; 3], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, 50.0);
    let list = [at(2, 0, [0.0, 0.0, -60.0], none, [100.0; 3]), at(2, 1, [0.0, 0.0, -40.0], none, [5.0; 3])];
    let mut b = build(t, dev, &geos, &list, &colors, &cam)?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "far plane")?;
    check_image(t, &b, &cam, &off, &img, None, &format!("{level}: far plane 50: a plate at 40 shows, the one at 60 does not"));

    // reverse Z far away: plates 1 apart at 100 000 with near 0.01.  (Float depth from 0 at the near plane
    // would hold 1 - 1e-7 for both.)
    let dist = 100_000.0;
    let cam = Camera::look_at([0.0; 3], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.01, f64::INFINITY);
    let list = [at(2, 0, [0.0, 0.0, -dist], none, [20_000.0; 3]), at(2, 1, [0.0, 0.0, -(dist - 1.0)], none, [15_000.0; 3])];
    let list2 = [list[1], list[0]];
    for (name, l) in [("far one first", list), ("near one first", list2)] {
        let mut b = build(t, dev, &geos, &l, &colors, &cam)?;
        let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "reverse Z")?;
        let d = read_depth(t, dev, &off)?;
        check_image(t, &b, &cam, &off, &img, Some(&d), &format!("{level}: reverse Z: plates 1 apart at 100 000, near 0.01, {name}"));
        // the upper left corner sees only the farther plate, the middle the nearer one
        let (c, m) = (pixel(&img, w, w / 2, h / 2), pixel(&img, w, 0, 0));
        t.ok(c != m && c[1] > c[0], &format!("{level}: the nearer plate in the middle {c:?}, the farther beside it {m:?}"));
    }
    Some(())
}

// ---------------------------------------------------------------- visibility

/// the lists of the last frame: which slots they hold, against what the instance's mesh is
pub struct Seen {
    slots: Vec<u32>,
    per_list: Vec<u32>,
    wrong_list: u32,
}

pub fn read_lists(t: &mut T, dev: &Device, s: &mut Scene, off: &mut Off, r: &mut Renderer, specs: &[(u32, u32)]) -> Option<Seen> {
    let id = t.res(off.submit(r, s), "frame")?;
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let lists = s.lists();
    let (dsize, vsize) = (s.draws_buffer().size(), s.visible_buffer().size());
    let rd = t.res(dev.create_buffer(&BufferDesc { size: dsize, usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "read-back")?;
    let rv = t.res(dev.create_buffer(&BufferDesc { size: vsize, usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "read-back")?;
    let mut e = t.res(dev.encoder(), "encoder")?;
    e.barrier(Access::INDIRECT | Access::INDEX_VERTEX, Access::COPY_READ);
    e.copy_buffer(s.draws_buffer(), 0, &rd, 0, dsize);
    e.copy_buffer(s.visible_buffer(), 0, &rv, 0, vsize);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    let cmd = t.res(e.finish(), "finish")?;
    let id = t.res(dev.queue().submit([cmd]), "submit")?;
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let (mut draws, mut vis) = (vec![0u32; dsize as usize / 4], vec![0u32; vsize as usize / 4]);
    t.res(rd.read(0, &mut draws), "read draws")?;
    t.res(rv.read(0, &mut vis), "read visible")?;
    let mut out = Seen { slots: Vec::new(), per_list: Vec::new(), wrong_list: 0 };
    for l in &lists {
        let n = draws[l.record as usize * 5 + 1];
        out.per_list.push(n);
        for k in 0..n.min(l.capacity) {
            let slot = vis[(l.first + k) as usize];
            out.slots.push(slot);
            if specs.iter().find(|x| x.0 == slot).is_none_or(|x| x.1 != l.mesh.0) {
                out.wrong_list += 1;
            }
        }
    }
    Some(out)
}

fn visibility(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = visibility_on(t, dev, level); });
}

fn visibility_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (128u32, 72u32);
    let aspect = w as f64 / h as f64;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let geos = [cube(), prism(7, 0.6), ball(6, 8)];
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    for g in &geos {
        t.res(r.add_mesh(&g.data()), "add_mesh")?;
    }
    let m = t.res(r.add_material(&Material::color(0.5, 0.75, 1.0)), "add_material")?;
    // random scenes, seeds fixed: every list = the exact test on the CPU
    for (seed, n, spread) in [(1u32, 20_000u32, 200.0f64), (2, 5000, 40.0), (3, 60_000, 500.0)] {
        let mut s = t.res(Scene::new(&r), "Scene::new")?;
        let mut rng = Rng(seed);
        let cam = Camera::look_at([rng.range(-20.0, 20.0), rng.range(-20.0, 20.0), rng.range(-20.0, 20.0)],
                                  [rng.range(-200.0, 200.0), rng.range(-200.0, 200.0), rng.range(-200.0, 200.0)],
                                  [0.0, 1.0, 0.0], rng.range(0.5, 1.4), aspect, 0.25, if seed == 2 { 80.0 } else { f64::INFINITY });
        s.set_camera(&cam);
        let rc = ref_camera(&cam);
        let mut truth = Vec::new();
        let mut ids = Vec::new();
        for _ in 0..n {
            let scale = rng.range(0.1, 3.0) as f32;
            let (mesh, pos) = (rng.below(3), [rng.range(-spread, spread), rng.range(-spread, spread), rng.range(-spread, spread)]);
            let rot = quat(cpu::norm([rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), 1.0]), rng.range(0.0, 6.0));
            let sc = [scale, scale * rng.range(0.5, 1.0) as f32, scale];
            let id = t.res(s.add(render3d::Instance { mesh: MeshId(mesh), material: m, transform: Transform { pos, rot, scale: sc } }), "add")?;
            ids.push((id.slot, mesh));
            let info = r.mesh_info(MeshId(mesh))?;
            let (c, rad) = cpu::world_sphere(info.center, info.radius, pos, rot, sc);
            truth.push((id.slot, rc.sphere_test(c, rad, [0.0; 3])));
        }
        let seen = read_lists(t, dev, &mut s, &mut off, &mut r, &ids)?;
        judge(t, &format!("{level}: seed {seed}: {n} instances, 3 meshes"), &seen, &truth);
    }
    // objects exactly at each face of the frustum: touching stays, a hair beyond goes
    let cam = Camera::look_at([3.0, -2.0, 11.0], [-4.0, 5.0, -9.0], [0.0, 1.0, 0.0], 1.1, aspect, 2.0, 400.0);
    let rc = ref_camera(&cam);
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    s.set_camera(&cam);
    let info = r.mesh_info(MeshId(2))?; // the ball: its sphere is itself
    let mut truth = Vec::new();
    let mut ids = Vec::new();
    let rad = 1.5f64;
    let k = rad / info.radius;
    let lx = cpu_len(rc.tx);
    let ly = cpu_len(rc.ty);
    // inward unit normals in (right, up, forward) and a point on each face
    let z = 50.0;
    let faces: [(&str, [f64; 3], [f64; 3]); 6] = [
        ("left", [1.0 / lx, 0.0, rc.tx / lx], [-z * rc.tx, 0.0, z]),
        ("right", [-1.0 / lx, 0.0, rc.tx / lx], [z * rc.tx, 0.0, z]),
        ("bottom", [0.0, 1.0 / ly, rc.ty / ly], [0.0, -z * rc.ty, z]),
        ("top", [0.0, -1.0 / ly, rc.ty / ly], [0.0, z * rc.ty, z]),
        ("near", [0.0, 0.0, 1.0], [0.0, 0.0, 2.0]),
        ("far", [0.0, 0.0, -1.0], [0.0, 0.0, 400.0]),
    ];
    for (name, n, p) in faces {
        for (gap, want) in [(0.0, true), (0.01, false)] {
            let d = -(rad + gap);
            let camera_space = [p[0] + n[0] * d, p[1] + n[1] * d, p[2] + n[2] * d];
            let world = [rc.eye[0] + rc.r[0] * camera_space[0] + rc.u[0] * camera_space[1] + rc.f[0] * camera_space[2],
                         rc.eye[1] + rc.r[1] * camera_space[0] + rc.u[1] * camera_space[1] + rc.f[1] * camera_space[2],
                         rc.eye[2] + rc.r[2] * camera_space[0] + rc.u[2] * camera_space[1] + rc.f[2] * camera_space[2]];
            let id = t.res(s.add(render3d::Instance { mesh: MeshId(2), material: m, transform: Transform { pos: world, rot: [0.0, 0.0, 0.0, 1.0], scale: [k as f32; 3] } }), "add")?;
            ids.push((id.slot, 2));
            truth.push((id.slot, name, want));
        }
    }
    let seen = read_lists(t, dev, &mut s, &mut off, &mut r, &ids)?;
    for (slot, name, want) in &truth {
        let got = seen.slots.contains(slot);
        t.ok(got == *want, &format!("{level}: a sphere {} the {name} face: {}", if *want { "touching" } else { "0.01 beyond" }, if got { "in the list" } else { "culled" }));
    }
    // culled = not culled: nothing visible goes missing in the image
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let mut rng = Rng(9);
    let cam = Camera::look_at([0.0, 0.0, 30.0], [10.0, 5.0, 0.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    s.set_camera(&cam);
    for _ in 0..8000 {
        let sc = rng.range(0.2, 1.0) as f32;
        let pos = [rng.range(-80.0, 80.0), rng.range(-50.0, 50.0), rng.range(-120.0, 25.0)];
        t.res(s.add(render3d::Instance { mesh: MeshId(rng.below(3)), material: m, transform: Transform { pos, rot: [0.0, 0.0, 0.0, 1.0], scale: [sc; 3] } }), "add")?;
    }
    let a = shot(t, dev, &mut off, &mut r, &mut s, "culled")?;
    s.set_culling(false);
    let b = shot(t, dev, &mut off, &mut r, &mut s, "not culled")?;
    let (n, m2) = diff(&a, &b);
    t.ok(n == 0, &format!("{level}: 8000 instances, culled against all drawn: {n} pixels differ (largest {m2})"));
    Some(())
}

fn cpu_len(t: f64) -> f64 {
    // the length of (1, tan), by the reference's own square root
    let v = cpu::norm([1.0, t, 0.0]);
    1.0 / v[0]
}

pub fn judge(t: &mut T, what: &str, seen: &Seen, truth: &[(u32, cpu::Seen)]) {
    let mut in_list = vec![false; truth.iter().map(|x| x.0).max().map_or(0, |v| v as usize + 1)];
    let mut twice = 0;
    for &slot in &seen.slots {
        if slot as usize >= in_list.len() {
            twice += 1;
        } else if core::mem::replace(&mut in_list[slot as usize], true) {
            twice += 1;
        }
    }
    let (mut yes, mut doubt, mut missing, mut extra) = (0, 0, 0, 0);
    for (slot, v) in truth {
        match v {
            cpu::Seen::Yes => {
                yes += 1;
                if !in_list[*slot as usize] {
                    missing += 1;
                }
            }
            cpu::Seen::Doubt => doubt += 1,
            cpu::Seen::No => {
                if in_list[*slot as usize] {
                    extra += 1;
                }
            }
        }
    }
    let total: u32 = seen.per_list.iter().sum();
    t.ok(missing == 0 && extra == 0 && twice == 0 && seen.wrong_list == 0,
         &format!("{what}: GPU {total} visible, CPU {yes} (+{doubt} within a hair of a face): {missing} missing, {extra} too many, {twice} doubled, {} in a wrong list", seen.wrong_list));
}

// ---------------------------------------------------------------- changes

fn changes(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = changes_on(t, dev, level); });
}

/// what a scene should hold, in the order of its slots or any other
#[derive(Clone)]
struct Want {
    spec: Spec,
    id: InstanceId,
}

fn rebuilt(t: &mut T, r: &Renderer, list: &[Want], cam: &Camera, reverse: bool) -> Option<Scene> {
    let mut s = t.res(Scene::new(r), "Scene::new")?;
    s.set_camera(cam);
    let mut order: Vec<&Want> = list.iter().collect();
    if reverse {
        order.reverse();
    }
    for w in order {
        t.res(s.add(w.spec.instance()), "add")?;
    }
    Some(s)
}

fn changes_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let (meshes, materials) = t.res(gallery_fill(&mut r), "fill")?;
    let cam = gallery_camera([0.0; 3]);
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    s.set_camera(&cam);
    // adding, over the first capacities
    let mut list = Vec::new();
    for spec in gallery_specs(meshes, materials, 2000, [0.0; 3]) {
        let id = t.res(s.add(spec.instance()), "add")?;
        list.push(Want { spec, id });
    }
    let same = |t: &mut T, off: &mut Off, r: &mut Renderer, s: &mut Scene, list: &[Want], what: &str| -> Option<()> {
        let got = shot(t, dev, off, r, s, what)?;
        let info = s.upload_info();
        let mut fresh = rebuilt(t, r, list, &cam, true)?;
        let want = shot(t, dev, off, r, &mut fresh, what)?;
        let (n, m) = diff(&got, &want);
        t.ok(n == 0, &format!("{level}: {what}: against a scene built anew: {n} pixels differ (largest {m}); sent {} changes{}, {} bytes", info.changed, if info.full { ", all" } else { "" }, info.bytes));
        Some(())
    };
    same(t, &mut off, &mut r, &mut s, &list, "2000 added")?;
    // nothing changed: only view and lights go up
    let _ = shot(t, dev, &mut off, &mut r, &mut s, "idle")?;
    let idle = s.upload_info();
    t.ok(idle.bytes == 208 + 480 && idle.changed == 0 && !idle.full, &format!("{level}: a frame without changes sends {} bytes (view and lights)", idle.bytes));
    // moving a few: k changes, 96 bytes each
    let mut rng = Rng(5);
    for _ in 0..37 {
        let k = rng.below(list.len() as u32) as usize;
        list[k].spec.pos[0] += 1.5;
        list[k].spec.pos[1] -= 0.75;
        let (id, sp) = (list[k].id, list[k].spec);
        t.res(s.set_transform(id, &sp.instance().transform), "set_transform")?;
    }
    same(t, &mut off, &mut r, &mut s, &list, "37 moved (some twice)")?;
    // removing
    let mut freed = Vec::new();
    for _ in 0..300 {
        let k = rng.below(list.len() as u32) as usize;
        let w = list.swap_remove(k);
        t.res(s.remove(w.id), "remove")?;
        freed.push(w.id);
    }
    same(t, &mut off, &mut r, &mut s, &list, "300 removed")?;
    t.ok(matches!(s.set_transform(freed[0], &Transform::at([0.0; 3])), Err(GpuError::Invalid(_))), &format!("{level}: a removed instance's id is refused"));
    // places come free and are used again; capacities stay
    let before = s.lists();
    let extra = gallery_specs(meshes, materials, 250, [15.0, 8.0, -20.0]);
    let (mut reused, slots_free): (u32, Vec<u32>) = (0, freed.iter().map(|i| i.slot).collect());
    for spec in extra {
        let id = t.res(s.add(spec.instance()), "add")?;
        reused += slots_free.contains(&id.slot) as u32;
        list.push(Want { spec, id });
    }
    t.ok(reused == 250, &format!("{level}: 250 added after 300 removed: {reused} on places that came free"));
    let stale = s.set_transform(freed[0], &Transform::at([0.0; 3]));
    t.ok(stale.is_err(), &format!("{level}: an old id does not reach the instance now on its place"));
    same(t, &mut off, &mut r, &mut s, &list, "250 added on free places")?;
    // (lists are settled by prepare: judged after the frame.  A list whose room went past its capacity grows and moves
    // those after it; the others keep their capacity, and every list its draw record)
    let after = s.lists();
    let grown = after.iter().zip(&before).filter(|(a, b)| a.capacity != b.capacity).count();
    let wrongly = after.iter().zip(&before).filter(|(a, b)| a.capacity != b.capacity && a.room <= b.capacity).count();
    let kept = grown > 0 || after.iter().zip(&before).all(|(a, b)| a.first == b.first);
    t.ok(after.len() == before.len() && wrongly == 0 && kept && after.iter().zip(&before).all(|(a, b)| a.record == b.record),
         &format!("{level}: coming and going: {grown} of {} lists grew (more instances than their capacity), {wrongly} without need; the draw records stay", after.len()));
    // within their capacity (as many instances of the same meshes come as go) every list keeps its place
    let before = s.lists();
    for _ in 0..50 {
        let k = rng.below(list.len() as u32) as usize;
        let mut w = list.swap_remove(k);
        t.res(s.remove(w.id), "remove")?;
        w.spec.pos[1] += 0.5;
        w.id = t.res(s.add(w.spec.instance()), "add")?;
        list.push(w);
    }
    same(t, &mut off, &mut r, &mut s, &list, "50 removed and added again")?;
    t.ok(s.lists().iter().zip(&before).all(|(a, b)| a.first == b.first && a.capacity == b.capacity && a.record == b.record),
         &format!("{level}: coming and going within their capacity, the lists keep their places"));
    // material and mesh of instances, a row of the table
    for k in 0..40 {
        let (id, m) = (list[k * 7].id, (k as u32 * 3) % materials);
        list[k * 7].spec.material = m;
        t.res(s.set_material(id, MaterialId(m)), "set_material")?;
    }
    for k in 0..40 {
        let (id, m) = (list[k * 11 + 1].id, (k as u32 * 5) % meshes);
        list[k * 11 + 1].spec.mesh = m;
        t.res(s.set_mesh(id, MeshId(m)), "set_mesh")?;
    }
    same(t, &mut off, &mut r, &mut s, &list, "materials and meshes of 80 instances changed")?;
    // a change of the table shows without an instance's upload
    let marked = list[0].spec.material;
    let magenta = |img: &[u8]| img.chunks_exact(4).filter(|p| p[0] > 0 && p[1] == 0 && p[2] > 0).count();
    let old = shot(t, dev, &mut off, &mut r, &mut s, "table row")?;
    t.res(r.set_material(MaterialId(marked), &Material::color(1.0, 0.0, 1.0)), "set_material")?;
    let got = shot(t, dev, &mut off, &mut r, &mut s, "table row")?;
    t.ok(s.upload_info().changed == 0, &format!("{level}: a row of the material table goes up by itself ({} bytes)", s.upload_info().bytes));
    t.ok(magenta(&old) == 0 && magenta(&got) > 0, &format!("{level}: the new colour shows: {} pixels of it, before {}", magenta(&got), magenta(&old)));
    let fresh_view = {
        let mut f = rebuilt(t, &r, &list, &cam, false)?;
        shot(t, dev, &mut off, &mut r, &mut f, "table row")?
    };
    let (n, m) = diff(&got, &fresh_view);
    t.ok(n == 0, &format!("{level}: a changed row of the table: against a scene built anew: {n} pixels differ (largest {m})"));
    // all of them at once: the other way of uploading
    for i in 0..list.len() {
        list[i].spec.pos[2] += 2.0;
        let (id, sp) = (list[i].id, list[i].spec);
        t.res(s.set_transform(id, &sp.instance().transform), "set_transform")?;
    }
    let info_all = {
        let got = shot(t, dev, &mut off, &mut r, &mut s, "all moved")?;
        let info = s.upload_info();
        let mut f = rebuilt(t, &r, &list, &cam, true)?;
        let want = shot(t, dev, &mut off, &mut r, &mut f, "all moved")?;
        let (n, m) = diff(&got, &want);
        t.ok(n == 0, &format!("{level}: all {} moved: against a scene built anew: {n} pixels differ (largest {m})", list.len()));
        info
    };
    t.ok(info_all.changed as usize == list.len() && !info_all.full, &format!("{level}: all {} moved (below the threshold): through the scatter shader, {} bytes", list.len(), info_all.bytes));
    // above the threshold (4096 changes and an eighth of the slots) all go at once
    let mut big = t.res(Scene::new(&r), "Scene::new")?;
    big.set_camera(&cam);
    let mut blist = Vec::new();
    for spec in gallery_specs(meshes, materials, 10_000, [0.0; 3]) {
        let id = t.res(big.add(spec.instance()), "add")?;
        blist.push(Want { spec, id });
    }
    let first = shot(t, dev, &mut off, &mut r, &mut big, "10000")?;
    t.ok(big.upload_info().full, &format!("{level}: 10 000 new instances go up at once ({} bytes)", big.upload_info().bytes));
    let _ = first;
    for i in 0..6000 {
        blist[i].spec.pos[1] += 1.0;
        let (id, sp) = (blist[i].id, blist[i].spec);
        t.res(big.set_transform(id, &sp.instance().transform), "set_transform")?;
    }
    let got = shot(t, dev, &mut off, &mut r, &mut big, "6000 moved")?;
    let info = big.upload_info();
    t.ok(info.full, &format!("{level}: 6000 of 10 000 moved: all at once ({} bytes)", info.bytes));
    let mut f = rebuilt(t, &r, &blist, &cam, true)?;
    let want = shot(t, dev, &mut off, &mut r, &mut f, "6000 moved")?;
    let (n, m) = diff(&got, &want);
    t.ok(n == 0, &format!("{level}: 6000 of 10 000 moved, against a scene built anew: {n} pixels differ (largest {m})"));
    for i in 0..200 {
        blist[i * 3].spec.pos[1] -= 1.0;
        let (id, sp) = (blist[i * 3].id, blist[i * 3].spec);
        t.res(big.set_transform(id, &sp.instance().transform), "set_transform")?;
    }
    let got = shot(t, dev, &mut off, &mut r, &mut big, "200 moved")?;
    let info = big.upload_info();
    t.ok(info.changed == 200 && !info.full && info.bytes == 208 + 480 + 200 * 96, &format!("{level}: 200 of 10 000 moved: {} changes, {} bytes", info.changed, info.bytes));
    let mut f = rebuilt(t, &r, &blist, &cam, false)?;
    let want = shot(t, dev, &mut off, &mut r, &mut f, "200 moved")?;
    let (n, m) = diff(&got, &want);
    t.ok(n == 0, &format!("{level}: 200 of 10 000 moved, against a scene built anew: {n} pixels differ (largest {m})"));
    // a prepare that never reached a frame loses nothing
    let (id, mut sp) = (blist[0].id, blist[0].spec);
    sp.pos[0] += 5.0;
    blist[0].spec = sp;
    t.res(big.set_transform(id, &sp.instance().transform), "set_transform")?;
    t.res(big.prepare(&mut r), "prepare")?;
    let got = shot(t, dev, &mut off, &mut r, &mut big, "after a lost prepare")?;
    let mut f = rebuilt(t, &r, &blist, &cam, true)?;
    let want = shot(t, dev, &mut off, &mut r, &mut f, "after a lost prepare")?;
    let (n, m) = diff(&got, &want);
    t.ok(n == 0, &format!("{level}: a prepare without a frame, then a frame: against a scene built anew: {n} pixels differ (largest {m})"));
    Some(())
}

// ---------------------------------------------------------------- many

fn many(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = many_on(t, dev, level); });
}

fn many_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (240u32, 135u32);
    let aspect = w as f64 / h as f64;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    // 160 shapes, 400 materials: the pools and the table grow on the way
    let mut geos = Vec::new();
    for k in 0..160u32 {
        geos.push(prism(3 + k % 40, 0.3 + k as f32 / 160.0));
    }
    let mut rng = Rng(21);
    let colors: Vec<[f32; 3]> = (0..400).map(|_| [rng.range(0.2, 1.0) as f32, rng.range(0.2, 1.0) as f32, rng.range(0.2, 1.0) as f32]).collect();
    let mut specs = Vec::new();
    for y in 0..30 {
        for x in 0..53 {
            specs.push(Spec { mesh: (x * 7 + y * 13) % 160, material: (x * 11 + y * 29) % 400, pos: [(x as f64 - 26.0) * 2.5, (y as f64 - 14.5) * 2.5, -(((x + y) % 5) as f64)],
                              rot: quat(cpu::norm([1.0, (x % 3) as f64, (y % 3) as f64]), (x + y) as f64 * 0.3), scale: [1.0; 3] });
        }
    }
    let cam = Camera::look_at([0.0, 0.0, 38.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, aspect, 0.5, f64::INFINITY);
    let mut b = build(t, dev, &geos, &specs, &colors, &cam)?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "many")?;
    let lists = b.s.lists();
    t.ok(lists.len() == 160, &format!("{level}: {} lists", lists.len()));
    t.ok(b.r.multi_draw() == (level == "Voll"), &format!("{level}: {}", if b.r.multi_draw() { "one multi-draw" } else { "one indirect draw per list" }));
    check_image(t, &b, &cam, &off, &img, None, &format!("{level}: {} instances, 160 meshes, 400 materials", specs.len()));
    // the mesh pools hold what they got before growing
    let g = ball(20, 40);
    t.res(b.r.add_mesh(&g.data()), "add_mesh")?;
    let big = prism(2000, 0.5);
    t.res(b.r.add_mesh(&big.data()), "add_mesh")?;
    let img2 = shot(t, dev, &mut off, &mut b.r, &mut b.s, "many, grown")?;
    let (n, m) = diff(&img, &img2);
    t.ok(n == 0, &format!("{level}: after the pools grew (a mesh of {} vertices): {n} pixels differ (largest {m})", big.pos.len()));
    // more than 65536 vertices: 32-bit indices, beside and between meshes of 16 bits; a mesh of an odd number of indices
    let huge = beyond_16_bits(ball(8, 12));
    let wide = t.res(b.r.add_mesh(&huge.data()), "add_mesh")?;
    t.ok(b.r.mesh_info(wide).is_some_and(|i| i.wide) && !b.r.mesh_info(MeshId(0)).is_some_and(|i| i.wide), &format!("{level}: a mesh of {} vertices takes 32-bit indices", huge.pos.len()));
    let mut geos2 = vec![cube(), huge, prism(5, 0.5), ball(8, 8), prism(6, 0.75)];
    geos2[2].idx.truncate(9); // (9 indices: an odd number)
    let mut list = Vec::new();
    for x in 0..5u32 {
        for y in 0..5u32 {
            list.push(Spec { mesh: (x + y) % 5, material: x * 5 + y, pos: [(x as f64 - 2.0) * 3.0, (y as f64 - 2.0) * 3.0, 0.0], rot: quat(cpu::norm([1.0, 1.0, 0.0]), 0.7), scale: [1.0; 3] });
        }
    }
    let cam2 = Camera::look_at([0.0, 0.0, 17.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, aspect, 0.5, f64::INFINITY);
    let mut b2 = build(t, dev, &geos2, &list, &colors, &cam2)?;
    let img3 = shot(t, dev, &mut off, &mut b2.r, &mut b2.s, "wide indices")?;
    check_image(t, &b2, &cam2, &off, &img3, None, &format!("{level}: lists of 16 and 32 bits in turn, a mesh of 9 indices"));
    Some(())
}

// ---------------------------------------------------------------- levels, coordinates, frames

/// the gallery's images: mixed, changed, far away (G5a), textured with mirrored instances, the glTF scene (G5b),
/// then those of common::LATER (G5c)
pub const GALLERY_NAMES: [&str; 5] = ["mix", "changed", "far", "tex", "gltf"];

fn gallery_image(t: &mut T, dev: &Device) -> Option<Vec<Vec<u8>>> {
    gallery_image_in(t, dev, linear_desc(Format::Rgba8Unorm))
}

/// the gallery's images with renderers of this description, in targets of its format
pub fn gallery_image_in(t: &mut T, dev: &Device, desc: RendererDesc) -> Option<Vec<Vec<u8>>> {
    let format = desc.color;
    let mut r = t.res(Renderer::new(dev, &desc), "Renderer::new")?;
    let (meshes, materials) = t.res(gallery_fill(&mut r), "fill")?;
    let mut off = t.res(Off::new(dev, GALLERY_W, GALLERY_H, format), "target")?;
    let (mut s, mut ids) = t.res(gallery_scene(&r, meshes, materials, [0.0; 3]), "gallery")?;
    let a = shot(t, dev, &mut off, &mut r, &mut s, "gallery")?;
    t.res(gallery_change(&mut s, &mut ids, meshes, materials), "gallery change")?;
    let b = shot(t, dev, &mut off, &mut r, &mut s, "gallery, changed")?;
    let (mut far, _) = t.res(gallery_scene(&r, meshes, materials, [1e7, -3e6, 2e7]), "gallery far")?;
    let c = shot(t, dev, &mut off, &mut r, &mut far, "gallery, far")?;
    let mut rt = t.res(Renderer::new(dev, &desc), "Renderer::new")?;
    let mut st = t.res(tex_gallery(&mut rt), "textured gallery")?;
    let d = shot(t, dev, &mut off, &mut rt, &mut st, "gallery, textured")?;
    let mut rg = t.res(Renderer::new(dev, &desc), "Renderer::new")?;
    let mut sg = t.res(gltf_gallery(&mut rg), "glTF gallery")?;
    let e = shot(t, dev, &mut off, &mut rg, &mut sg, "gallery, glTF")?;
    let mut all = vec![a, b, c, d, e];
    for name in LATER {
        let mut r = t.res(Renderer::new(dev, &desc), "Renderer::new")?;
        let mut s = t.res(later_gallery_in(name, &mut r, format), name)?;
        all.push(shot(t, dev, &mut off, &mut r, &mut s, name)?);
    }
    Some(all)
}

fn levels(t: &mut T) {
    let Some(g) = setup(t) else { return };
    if g.adapter.level() != Level::Voll {
        out("SKIP: this adapter has only the level Basis");
        return;
    }
    let Some(voll) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let Some(a) = gallery_image(t, &voll) else { return };
    let Some(basis) = t.res(g.adapter.create_device(&DeviceDesc { max_level: Level::Basis }), "Basis device") else { return };
    let Some(b) = gallery_image(t, &basis) else { return };
    let mut names = vec!["3000 instances, 48 meshes, 96 materials", "changed", "far away", "7 textures in 4 classes, 600 instances, a third mirrored, MASK", "the glTF scene"];
    names.extend_from_slice(&LATER);
    for (i, name) in names.iter().enumerate() {
        let (n, m) = diff(&a[i], &b[i]);
        // (Apple's GPUs: the textured gallery picks levels and texels from derivatives that vary from run to run,
        // DERIVATIVES_VARY in common.rs)
        let loose = DERIVATIVES_VARY && i == 3;
        let note = if loose { format!(" (Apple's GPUs: at most {DERIVATIVE_PIXELS})") } else { String::new() };
        t.ok(n == 0 || loose && n <= DERIVATIVE_PIXELS, &format!("Voll against Basis, {name}: {n} of {} pixels differ (largest {m}){note}",
                                                           GALLERY_W * GALLERY_H));
    }
}

fn coords(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = coords_on(t, dev, level); });
}

fn coords_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let (meshes, materials) = t.res(gallery_fill(&mut r), "fill")?;
    let place = |t: &mut T, r: &mut Renderer, off: &mut Off, at: [f64; 3], origin: [f64; 3]| -> Option<Vec<u8>> {
        let mut s = t.res(Scene::new(r), "Scene::new")?;
        s.set_origin(origin);
        s.set_camera(&gallery_camera(at));
        for spec in gallery_specs(meshes, materials, GALLERY_N, at) {
            t.res(s.add(spec.instance()), "add")?;
        }
        shot(t, dev, off, r, &mut s, "scene")
    };
    let home = place(t, &mut r, &mut off, [0.0; 3], [0.0; 3])?;
    let far = [1e7, 1e7, 1e7];
    let away = place(t, &mut r, &mut off, far, far)?;
    let (n, m) = diff(&home, &away);
    t.ok(n == 0, &format!("{level}: at 0 and at 1e7, the origin there, positions exact in f64: {n} of {} pixels differ (largest {m})", w * h));
    // positions f64 rounds at 1e7 (2e-9 apart): f32 hardly ever sees it
    let jitter = |t: &mut T, r: &mut Renderer, off: &mut Off, at: [f64; 3]| -> Option<Vec<u8>> {
        let mut s = t.res(Scene::new(r), "Scene::new")?;
        s.set_origin(at);
        s.set_camera(&gallery_camera(at));
        let mut rng = Rng(31);
        for spec in gallery_specs(meshes, materials, GALLERY_N, [0.0; 3]) {
            let mut sp = spec;
            for k in 0..3 {
                sp.pos[k] += at[k] + rng.unit() * 1e-3 / 3.0;
            }
            t.res(s.add(sp.instance()), "add")?;
        }
        shot(t, dev, off, r, &mut s, "scene")
    };
    let (a, b) = (jitter(t, &mut r, &mut off, [0.0; 3])?, jitter(t, &mut r, &mut off, far)?);
    let (n, m) = diff(&a, &b);
    t.ok(n * 100 <= w * h, &format!("{level}: the same with positions f64 must round at 1e7: {n} of {} pixels differ (largest {m})", w * h));
    // moving the origin at run time: every instance again, the same image
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    s.set_camera(&gallery_camera(far));
    for spec in gallery_specs(meshes, materials, GALLERY_N, far) {
        t.res(s.add(spec.instance()), "add")?;
    }
    s.set_origin(far);
    let moved = shot(t, dev, &mut off, &mut r, &mut s, "scene")?;
    let (n, m) = diff(&home, &moved);
    t.ok(n == 0, &format!("{level}: the origin set after the instances: {n} pixels differ (largest {m})"));
    s.set_origin([far[0] + 16.0, far[1] - 32.0, far[2] + 8.0]);
    let _ = shot(t, dev, &mut off, &mut r, &mut s, "scene")?;
    let info = s.upload_info();
    t.ok(info.full, &format!("{level}: a new origin sends everything ({} bytes)", info.bytes));
    // what the origin is for: without it, f32 at 1e7 steps by 1 and the image falls apart
    let naive = place(t, &mut r, &mut off, far, [0.0; 3])?;
    let (n, m) = diff(&home, &naive);
    t.ok(n * 4 > w * h, &format!("{level}: without an origin, at 1e7 (the test can tell): {n} of {} pixels differ (largest {m})", w * h));
    Some(())
}

fn frames(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = frames_on(t, dev, level); });
}

fn frames_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let mut r = t.res(Renderer::new(dev, &frames_desc(3)), "Renderer::new")?;
    let (meshes, materials) = t.res(gallery_fill(&mut r), "fill")?;
    let mut targets = Vec::new();
    for _ in 0..7 {
        targets.push(t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?);
    }
    let cams: Vec<Camera> = (0..7).map(|i| {
        let mut c = gallery_camera([0.0; 3]);
        c.eye[0] += i as f64 * 2.0;
        c
    }).collect();
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let mut ids = Vec::new();
    let mut specs = gallery_specs(meshes, materials, 1500, [0.0; 3]);
    for sp in &specs {
        ids.push(t.res(s.add(sp.instance()), "add")?);
    }
    // seven frames, none waited for by hand: each with its camera and a change of its own
    let mut states = Vec::new();
    let mut last = None;
    for i in 0..7 {
        specs[i * 10].pos[1] += 6.0;
        let sp = specs[i * 10];
        t.res(s.set_transform(ids[i * 10], &sp.instance().transform), "set_transform")?;
        s.set_camera(&cams[i]);
        last = Some(t.res(targets[i].submit(&mut r, &mut s), "frame")?);
        states.push(specs.clone());
    }
    t.res(dev.wait(last?, 10_000_000_000), "wait")?;
    t.res(dev.wait_idle(), "wait_idle")?;
    for i in 0..7 {
        let got = t.res(targets[i].read(), "read")?;
        let mut fresh = t.res(Scene::new(&r), "Scene::new")?;
        fresh.set_camera(&cams[i]);
        for sp in &states[i] {
            t.res(fresh.add(sp.instance()), "add")?;
        }
        let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
        let want = shot(t, dev, &mut off, &mut r, &mut fresh, "alone")?;
        let (n, m) = diff(&got, &want);
        t.ok(n == 0, &format!("{level}: frame {i} of 7 in flight (3 slots), against it drawn alone: {n} pixels differ (largest {m})"));
    }
    // the renderer's own depth buffer
    t.res(r.set_size(w, h), "set_size")?;
    let color = t.res(dev.create_texture(&TextureDesc::d2(w, h, Format::Rgba8Unorm, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC)), "texture")?;
    let rb = t.res(dev.create_buffer(&BufferDesc { size: 1280 * h as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "read-back")?;
    s.set_camera(&cams[6]);
    let mut id = None;
    for k in 0..2 {
        t.res(s.prepare(&mut r), "prepare")?;
        let mut e = t.res(dev.encoder(), "encoder")?;
        let ca = ColorAttachment { view: color.view(), load: Load::Clear, clear: CLEAR, store: true, before: if k == 0 { Layout::Undefined } else { Layout::CopySrc }, after: Layout::CopySrc };
        t.res(r.record(&mut e, &s, ca, None), "record")?;
        e.copy_texture_to_buffer(TextureRegion::mip(&color, 0), &rb, DataLayout { offset: 0, bytes_per_row: 1280, rows_per_image: 0 });
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        let cmd = t.res(e.finish(), "finish")?;
        let v = t.res(dev.queue().submit([cmd]), "submit")?;
        s.submitted(v);
        id = Some(v);
    }
    t.res(dev.wait(id?, 10_000_000_000), "wait")?;
    let mut got = vec![0u8; (w * h * 4) as usize];
    t.res(rb.read(0, &mut got), "read")?;
    let want = t.res(targets[6].read(), "read")?;
    let (n, m) = diff(&got, &want);
    t.ok(n == 0, &format!("{level}: the renderer's own depth buffer, twice: against the caller's: {n} pixels differ (largest {m})"));
    Some(())
}

// ---------------------------------------------------------------- errors, shaders, window

fn errors(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let inv = |r: gpu::Result<()>| matches!(r, Err(GpuError::Invalid(_)));
    t.ok(inv(Renderer::new(&dev, &frames_desc(0)).map(|_| ())), "0 frames in flight: Invalid");
    t.ok(inv(Renderer::new(&dev, &frames_desc(4)).map(|_| ())), "4 frames in flight: Invalid");
    let Some(mut r) = t.res(Renderer::new(&dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new") else { return };
    let c = cube();
    t.ok(inv(r.add_mesh(&render3d::MeshData { positions: &c.pos, normals: &c.nrm[1..], uvs: &c.uv, indices: &c.idx }).map(|_| ())), "add_mesh: too few normals: Invalid");
    t.ok(inv(r.add_mesh(&render3d::MeshData { positions: &c.pos, normals: &c.nrm, uvs: &c.uv, indices: &c.idx[1..] }).map(|_| ())), "add_mesh: indices not in threes: Invalid");
    t.ok(inv(r.add_mesh(&render3d::MeshData { positions: &c.pos, normals: &c.nrm, uvs: &c.uv, indices: &[0, 1, 24] }).map(|_| ())), "add_mesh: an index out of range: Invalid");
    let nan = [[f32::NAN, 0.0, 0.0]; 3];
    let (un, uu) = ([[0.0f32; 3]; 3], [[0.0f32; 2]; 3]);
    t.ok(inv(r.add_mesh(&render3d::MeshData { positions: &nan, normals: &un, uvs: &uu, indices: &[0, 1, 2] }).map(|_| ())), "add_mesh: a position not finite: Invalid");
    let Some(mesh) = t.res(r.add_mesh(&c.data()), "add_mesh") else { return };
    let Some(mat) = t.res(r.add_material(&Material::color(1.0, 1.0, 1.0)), "add_material") else { return };
    t.ok(inv(r.set_material(MaterialId(5), &Material::color(1.0, 1.0, 1.0))), "set_material of a material not there: Invalid");
    let Some(mut s) = t.res(Scene::new(&r), "Scene::new") else { return };
    let mirrored = Transform { pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0], scale: [-1.0, 1.0, 1.0] };
    t.ok(s.add(render3d::Instance { mesh, material: mat, transform: mirrored }).is_ok() && s.lists().is_empty(), "a negative scale: accepted (its list comes with prepare)");
    let infinite = Transform { scale: [f32::INFINITY, 1.0, 1.0], ..mirrored };
    t.ok(inv(s.add(render3d::Instance { mesh, material: mat, transform: infinite }).map(|_| ())), "a scale not finite: Invalid");
    let no_rot = Transform { pos: [0.0; 3], rot: [0.0; 4], scale: [1.0; 3] };
    t.ok(inv(s.add(render3d::Instance { mesh, material: mat, transform: no_rot }).map(|_| ())), "a quaternion of length 0: Invalid");
    let nowhere = Transform::at([f64::NAN, 0.0, 0.0]);
    t.ok(inv(s.add(render3d::Instance { mesh, material: mat, transform: nowhere }).map(|_| ())), "a position not finite: Invalid");
    let Some(mut off) = t.res(Off::new(&dev, 16, 16, Format::Rgba8Unorm), "target") else { return };
    // recording before prepare, a wrong clear, no depth at all
    if let Some(mut e) = t.res(dev.encoder(), "encoder") {
        let ca = ColorAttachment { view: off.color.view(), load: Load::Clear, clear: CLEAR, store: true, before: Layout::Undefined, after: Layout::CopySrc };
        t.ok(inv(r.record(&mut e, &s, ca, None)), "record before prepare: Invalid");
    }
    let Some(other) = t.res(dev.create_texture(&TextureDesc::d2(16, 16, Format::Bgra8Unorm, TextureUsage::COLOR_TARGET)), "texture") else { return };
    if t.res(s.prepare(&mut r), "prepare").is_some() {
        t.ok(s.lists().iter().any(|l| l.mirrored), "a negative scale: after prepare a mirrored instance in a list of its own (G5b)");
        if let Some(mut e) = t.res(dev.encoder(), "encoder") {
            let ca = ColorAttachment { view: off.color.view(), load: Load::Clear, clear: CLEAR, store: true, before: Layout::Undefined, after: Layout::CopySrc };
            t.ok(inv(r.record(&mut e, &s, ca, None)), "no depth attachment and no set_size: Invalid");
            let da = DepthAttachment { view: off.depth.view(), load: Load::Clear, clear: 1.0, store: true, before: Layout::Undefined, after: Layout::Sampled };
            t.ok(inv(r.record(&mut e, &s, ca, Some(da))), "a depth clear of 1 (forward Z): Invalid");
            let cb = ColorAttachment { view: other.view(), load: Load::Clear, clear: CLEAR, store: true, before: Layout::Undefined, after: Layout::ColorTarget };
            let da = DepthAttachment { clear: render3d::DEPTH_CLEAR, ..da };
            t.ok(inv(r.record(&mut e, &s, cb, Some(da))), "a colour target in another format: Invalid");
        }
    }
    // a mesh and a material the renderer does not have: at once, and nothing stays behind
    let Some(mut s2) = t.res(Scene::new(&r), "Scene::new") else { return };
    let here = Transform::at([0.0; 3]);
    t.ok(inv(s2.add(render3d::Instance { mesh: MeshId(99), material: mat, transform: here }).map(|_| ())), "an instance of a mesh not there: Invalid");
    t.ok(inv(s2.add(render3d::Instance { mesh: MeshId(u32::MAX), material: mat, transform: here }).map(|_| ())), "the largest mesh id: Invalid, nothing allocated for it");
    t.ok(inv(s2.add(render3d::Instance { mesh, material: MaterialId(99), transform: here }).map(|_| ())), "an instance of a material not there: Invalid");
    t.ok(s2.is_empty() && s2.lists().is_empty(), "refused instances leave nothing in the scene");
    let Some(id) = t.res(s2.add(render3d::Instance { mesh, material: mat, transform: here }), "add") else { return };
    t.ok(inv(s2.set_mesh(id, MeshId(99))), "set_mesh to a mesh not there: Invalid");
    t.ok(inv(s2.set_material(id, MaterialId(99))), "set_material to a material not there: Invalid");
    // a mesh and a material that come after the scene count
    let Some(later) = t.res(r.add_mesh(&c.data()), "add_mesh") else { return };
    let Some(later_m) = t.res(r.add_material(&Material::color(0.5, 0.5, 0.5)), "add_material") else { return };
    t.res(s2.set_mesh(id, later), "set_mesh to a later mesh");
    t.res(s2.set_material(id, later_m), "set_material to a later material");
    // a scene is its renderer's
    let Some(mut r2) = t.res(Renderer::new(&dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new") else { return };
    t.ok(inv(s2.prepare(&mut r2)), "prepare with another renderer: Invalid");
    if t.res(s2.prepare(&mut r), "prepare with its own renderer").is_some() {
        if let Some(mut e) = t.res(dev.encoder(), "encoder") {
            let ca = ColorAttachment { view: off.color.view(), load: Load::Clear, clear: CLEAR, store: true, before: Layout::Undefined, after: Layout::CopySrc };
            let da = DepthAttachment { view: off.depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: true, before: Layout::Undefined, after: Layout::Sampled };
            t.ok(inv(r2.record(&mut e, &s2, ca, Some(da))), "record with another renderer: Invalid");
        }
    }
    // an empty scene: a clear, nothing else; also with an origin, and after a prepare without a frame
    let Some(mut s4) = t.res(Scene::new(&r), "Scene::new") else { return };
    if let Some(img) = shot(t, &dev, &mut off, &mut r, &mut s4, "an empty scene") {
        t.ok(img.chunks_exact(4).all(|p| p[0..3] == CLEAR8), "an empty scene: the clear colour");
    }
    s4.set_origin([1e7, 0.0, 0.0]);
    if let Some(img) = shot(t, &dev, &mut off, &mut r, &mut s4, "an empty scene with an origin") {
        t.ok(img.chunks_exact(4).all(|p| p[0..3] == CLEAR8), "an empty scene with an origin: the clear colour");
    }
    t.res(s4.prepare(&mut r), "prepare of an empty scene");
    if let Some(img) = shot(t, &dev, &mut off, &mut r, &mut s4, "an empty scene after a prepare without a frame") {
        t.ok(img.chunks_exact(4).all(|p| p[0..3] == CLEAR8), "an empty scene after a prepare without a frame: the clear colour");
    }
    // instances that come after prepare: what went up does not hold them, the next prepare takes them
    let cam = Camera::look_at([0.0, 0.0, 6.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, 1.0, 0.1, f64::INFINITY);
    let Some(mut s5) = t.res(Scene::new(&r), "Scene::new") else { return };
    s5.set_camera(&cam);
    let Some(one) = t.res(s5.add(render3d::Instance { mesh, material: mat, transform: Transform::at([-2.0, 0.0, 0.0]) }), "add") else { return };
    let Some(_) = shot(t, &dev, &mut off, &mut r, &mut s5, "before") else { return };
    let refused = |t: &mut T, r: &Renderer, s: &Scene, off: &Off, what: &str| {
        if let Some(mut e) = t.res(dev.encoder(), "encoder") {
            let ca = ColorAttachment { view: off.color.view(), load: Load::Clear, clear: CLEAR, store: true, before: Layout::CopySrc, after: Layout::CopySrc };
            let da = DepthAttachment { view: off.depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: true, before: Layout::Sampled, after: Layout::Sampled };
            t.ok(inv(r.record(&mut e, s, ca, Some(da))), what);
        }
    };
    t.res(s5.prepare(&mut r), "prepare");
    t.res(s5.add(render3d::Instance { mesh: later, material: mat, transform: Transform::at([2.0, 0.0, 0.0]) }), "add after prepare");
    refused(t, &r, &s5, &off, "an instance added after prepare: record = Invalid");
    t.res(s5.prepare(&mut r), "prepare");
    t.res(s5.set_mesh(one, later), "set_mesh after prepare");
    refused(t, &r, &s5, &off, "a mesh changed after prepare: record = Invalid");
    let Some(got) = shot(t, &dev, &mut off, &mut r, &mut s5, "after them") else { return };
    let Some(mut fresh) = t.res(Scene::new(&r), "Scene::new") else { return };
    fresh.set_camera(&cam);
    for x in [-2.0, 2.0] {
        t.res(fresh.add(render3d::Instance { mesh: later, material: mat, transform: Transform::at([x, 0.0, 0.0]) }), "add");
    }
    if let Some(want) = shot(t, &dev, &mut off, &mut r, &mut fresh, "built anew") {
        let (n, m) = diff(&got, &want);
        t.ok(n == 0 && want.chunks_exact(4).any(|p| p[0..3] != CLEAR8), &format!("the next prepare takes them: against a scene built anew: {n} pixels differ (largest {m})"));
    }
    // a material row that a frame recorded and never sent is sent again
    let Some(mut s6) = t.res(Scene::new(&r), "Scene::new") else { return };
    s6.set_camera(&cam);
    t.res(s6.add(render3d::Instance { mesh, material: mat, transform: Transform::at([0.0; 3]) }), "add");
    let Some(_) = shot(t, &dev, &mut off, &mut r, &mut s6, "before the row") else { return };
    t.res(r.set_material(mat, &Material::color(0.0, 1.0, 0.0)), "set_material");
    t.res(s6.prepare(&mut r), "prepare");
    if let Some(mut e) = t.res(dev.encoder(), "encoder") {
        let ca = ColorAttachment { view: off.color.view(), load: Load::Clear, clear: CLEAR, store: true, before: Layout::CopySrc, after: Layout::CopySrc };
        let da = DepthAttachment { view: off.depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: true, before: Layout::Sampled, after: Layout::Sampled };
        t.res(r.record(&mut e, &s6, ca, Some(da)), "a frame recorded and dropped");
    }
    if let Some(img) = shot(t, &dev, &mut off, &mut r, &mut s6, "after the dropped frame") {
        t.ok(img.chunks_exact(4).any(|p| p[1] > 0 && p[0] == 0 && p[2] == 0), "a row of the table in a frame never submitted: it still arrives");
    }
    // lost: every call says so
    dev.test_lose();
    t.ok(matches!(off.submit(&mut r, &mut s4), Err(GpuError::DeviceLost)), "a lost device: a frame = DeviceLost");
}

fn compile_shaders() -> Option<Vec<(&'static str, Vec<u32>)>> {
    let opt = wgsl::CompileOptions { robust: false, ..wgsl::CompileOptions::new(shader::BASIS) };
    let mut outv = Vec::new();
    for (src, list) in [(render3d::CULL_WGSL, &[("CULL", "main"), ("CULL_EARLY", "early"), ("CULL_LATE", "late")][..]), (render3d::SCATTER_WGSL, &[("SCATTER", "main")][..]),
                        (render3d::DRAW_WGSL, &[("VS", "vs"), ("FS", "fs"), ("FS_MASK", "fs_mask"), ("VS_DEPTH", "vs_depth"), ("VS_SHADOW", "vs_shadow"), ("FS_SHADOW", "fs_shadow")][..]),
                        (render3d::HIZ_WGSL, &[("HIZ_DEPTH", "from_depth"), ("HIZ_DOWN", "down")][..])] {
        match wgsl::compile(src, &opt) {
            Ok(o) => {
                for (name, entry) in list {
                    outv.push((*name, o.entry(entry)?.words.clone()));
                }
            }
            Err(d) => {
                out(&format!("FAIL {}: {:?}", list[0].0, d.first().map(|x| format!("{x}"))));
                return None;
            }
        }
    }
    Some(outv)
}

fn gen_shaders(path: &str) -> i32 {
    let Some(v) = compile_shaders() else { return 1 };
    let mut s = String::from("// written by render3dtest --gen-shaders from cull.wgsl, scatter.wgsl, draw.wgsl, hiz.wgsl (shader::BASIS, no robustness); do not edit\n");
    for (name, w) in &v {
        s.push_str(&format!("pub const {name}: &[u32] = &[\n"));
        for row in w.chunks(10) {
            s.push_str("    ");
            for x in row {
                s.push_str(&format!("{x:#x}, "));
            }
            s.truncate(s.trim_end().len());
            s.push('\n');
        }
        s.push_str("];\n");
    }
    if plat::write_file(path, s.as_bytes()) {
        out(&format!("{path}: {}", v.iter().map(|x| format!("{} {} words", x.0, x.1.len())).collect::<Vec<_>>().join(", ")));
        0
    } else {
        out(&format!("FAIL cannot write {path}"));
        1
    }
}

fn shaders(t: &mut T) {
    let Some(v) = compile_shaders() else {
        t.ok(false, "the WGSL compiles");
        return;
    };
    let table: [(&str, &[u32]); 12] = [("CULL", render3d::CULL_SPV), ("CULL_EARLY", render3d::CULL_EARLY_SPV), ("CULL_LATE", render3d::CULL_LATE_SPV),
                                       ("SCATTER", render3d::SCATTER_SPV), ("VS", render3d::VS_SPV), ("FS", render3d::FS_SPV), ("FS_MASK", render3d::FS_MASK_SPV),
                                       ("VS_DEPTH", render3d::VS_DEPTH_SPV),
                                       ("VS_SHADOW", render3d::VS_SHADOW_SPV), ("FS_SHADOW", render3d::FS_SHADOW_SPV), ("HIZ_DEPTH", render3d::HIZ_DEPTH_SPV),
                                       ("HIZ_DOWN", render3d::HIZ_DOWN_SPV)];
    for (name, embedded) in table {
        let made = v.iter().find(|x| x.0 == name).map_or(&[][..], |x| &x.1[..]);
        t.ok(made == embedded, &format!("{name}: {} words embedded, as wgsl::compile makes them (if not: render3dtest --gen-shaders lib/render3d/shaders_spv.rs)", embedded.len()));
    }
    // (G5c) the split-sum's table
    let fresh = render3d::dfg_table();
    let same = fresh.len() == render3d::DFG.len() && fresh.iter().zip(render3d::DFG.iter()).all(|(a, b)| a[0].to_bits() == b[0].to_bits() && a[1].to_bits() == b[1].to_bits());
    t.ok(same, &format!("DFG: {} entries embedded, as dfg_table makes them (if not: render3dtest --gen-dfg lib/render3d/dfg_table.rs)", render3d::DFG.len()));
}

fn gen_dfg(path: &str) -> i32 {
    let v = render3d::dfg_table();
    let mut s = String::from("// written by render3dtest --gen-dfg from render3d::dfg_table (dfg.rs): (A, B) as f32 bits, n.v along a row; do not edit\n");
    s.push_str(&format!("pub const DFG_BITS: [u32; {}] = [\n", v.len() * 2));
    for row in v.chunks(4) {
        s.push_str("    ");
        for e in row {
            s.push_str(&format!("{:#010x}, {:#010x}, ", e[0].to_bits(), e[1].to_bits()));
        }
        s.truncate(s.trim_end().len());
        s.push('\n');
    }
    s.push_str("];\n");
    if plat::write_file(path, s.as_bytes()) {
        out(&format!("{path}: {} entries", v.len()));
        0
    } else {
        out(&format!("FAIL cannot write {path}"));
        1
    }
}

/// a real window, a few frames with the swapchain's format, the presented image against the offscreen one
fn window(t: &mut T) {
    if cfg!(target_os = "ios") {
        out("SKIP: a plain program has no window on iOS (UIKit's host: uigputest)");
        return;
    }
    let Some(g) = setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    if !dev.features().swapchain {
        out("SKIP: no swapchains on this device");
        return;
    }
    let w = plat::win_create("render3dtest", 320, 180, plat::PWF_SYSFRAME, plat::PW_NOOWNER, plat::PW_DEFAULT, plat::PW_DEFAULT);
    if !t.ok(w >= 0, "plat window") {
        return;
    }
    plat::win_show(w, if plat::env("jfgui_TEST_NOACTIVATE").is_some() { plat::PWS_SHOWNA } else { plat::PWS_SHOW });
    pump(150);
    window_checks(t, &g, &dev, w);
    plat::win_destroy(w);
    pump(50);
}

fn pump(ms: u64) {
    let dl = plat::time_us() + ms * 1000;
    loop {
        plat::wait(if ms == 0 { 0 } else { dl });
        while plat::evq_pop().is_some() {}
        if plat::time_us() >= dl {
            return;
        }
    }
}

fn window_checks(t: &mut T, g: &Gpu, dev: &Device, w: i32) -> Option<()> {
    let surf = t.res(Surface::new(&g.inst, w), "Surface::new")?;
    let desc = SwapchainDesc { present: PresentMode::Fifo, copy_src: true, ..SwapchainDesc::DEFAULT };
    let mut sc = t.res(dev.create_swapchain(&surf, &desc), "create_swapchain")?;
    let (sw, sh) = sc.size();
    let format = sc.format();
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(format)), "Renderer::new")?;
    let (meshes, materials) = t.res(gallery_fill(&mut r), "fill")?;
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let mut cam = gallery_camera([0.0; 3]);
    cam.aspect = sw as f64 / sh as f64;
    s.set_camera(&cam);
    for spec in gallery_specs(meshes, materials, GALLERY_N, [0.0; 3]) {
        t.res(s.add(spec.instance()), "add")?;
    }
    t.res(r.set_size(sw, sh), "set_size")?;
    let stride = (sw * 4).div_ceil(256) * 256;
    let rb = t.res(dev.create_buffer(&BufferDesc { size: stride as u64 * sh as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "read-back")?;
    let mut id = None;
    // a few frames as a program would draw them, the last one read back
    for k in 0..6 {
        let frame = t.res(sc.acquire(), "acquire")?;
        let Some(frame) = frame else {
            t.ok(false, "acquire: no image");
            return None;
        };
        t.res(s.prepare(&mut r), "prepare")?;
        let mut e = t.res(dev.encoder(), "encoder")?;
        let last = k == 5;
        let color = ColorAttachment { view: sc.view(&frame), load: Load::Clear, clear: CLEAR, store: true, before: Layout::Undefined,
                                      after: if last { Layout::CopySrc } else { Layout::Present } };
        t.res(r.record(&mut e, &s, color, None), "record")?;
        if last {
            e.copy_texture_to_buffer(TextureRegion::mip(sc.texture(&frame), 0), &rb, DataLayout { offset: 0, bytes_per_row: stride, rows_per_image: 0 });
            e.texture_barrier(sc.texture(&frame), Layout::CopySrc, Layout::Present);
            e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        }
        let cmd = t.res(e.finish(), "finish")?;
        let v = t.res(sc.present(frame, [cmd]), "present")?;
        s.submitted(v);
        id = Some(v);
        pump(0);
    }
    t.res(dev.wait(id?, 10_000_000_000), "wait")?;
    let mut raw = vec![0u8; stride as usize * sh as usize];
    t.res(rb.read(0, &mut raw), "read")?;
    // the same frame offscreen, in the same format
    let mut off = t.res(Off::new(dev, sw, sh, format), "target")?;
    let want = shot(t, dev, &mut off, &mut r, &mut s, "offscreen")?;
    let mut got = Vec::with_capacity(want.len());
    for y in 0..sh as usize {
        got.extend_from_slice(&raw[y * stride as usize..y * stride as usize + sw as usize * 4]);
    }
    let (n, m) = diff(&got, &want);
    t.ok(n == 0, &format!("the presented image ({sw} x {sh}, {format:?}) against the same frame offscreen: {n} pixels differ (largest {m})"));
    // (G5e2) the right colours: the same frame in Rgba32Float, encoded on the CPU by the definition
    if RendererDesc::new(format).encodes_srgb() {
        let mut rf = t.res(Renderer::new(dev, &RendererDesc::new(Format::Rgba32Float)), "Renderer::new")?;
        let (meshes, materials) = t.res(gallery_fill(&mut rf), "fill")?;
        let mut sf = t.res(Scene::new(&rf), "Scene::new")?;
        sf.set_camera(&cam);
        for spec in gallery_specs(meshes, materials, GALLERY_N, [0.0; 3]) {
            t.res(sf.add(spec.instance()), "add")?;
        }
        let mut offf = t.res(Off::new(dev, sw, sh, Format::Rgba32Float), "target")?;
        let f = shot(t, dev, &mut offf, &mut rf, &mut sf, "offscreen, float")?;
        let bgra = matches!(format, Format::Bgra8Unorm);
        let (mut n, mut m) = (0u32, 0u8);
        for (p, c) in got.chunks_exact(4).zip(f.chunks_exact(16)) {
            let v = |k: usize| f32::from_le_bytes([c[k * 4], c[k * 4 + 1], c[k * 4 + 2], c[k * 4 + 3]]) as f64;
            let want = [cpu::srgb_encode(v(0)), cpu::srgb_encode(v(1)), cpu::srgb_encode(v(2))];
            let have = if bgra { [p[2], p[1], p[0]] } else { [p[0], p[1], p[2]] };
            let d = (0..3).map(|k| have[k].abs_diff(want[k])).max().unwrap_or(0);
            n += (d > 0) as u32;
            m = m.max(d);
        }
        t.ok(m <= 1, &format!("the presented image in sRGB codes (the shader encodes for {format:?}) against the float frame encoded on the CPU: {n} pixels differ, largest {m}"));
    } else {
        out(&format!("  note: the swapchain's format {format:?}: the shader does not encode"));
    }
    t.res(dev.wait_idle(), "wait_idle")?;
    Some(())
}

// ---------------------------------------------------------------- the gallery for the browser's test

/// the formats --dump --format knows (the gallery in other targets: sRGB, float; docs/RENDER3D.md "sRGB")
const DUMP_FORMATS: [(&str, Format); 5] = [("rgba8", Format::Rgba8Unorm), ("bgra8", Format::Bgra8Unorm), ("rgba8srgb", Format::Rgba8Srgb),
                                           ("rgba16f", Format::Rgba16Float), ("rgba32f", Format::Rgba32Float)];

fn dump(dir: &str) -> i32 {
    let mut t = T { fails: 0, validate: false };
    let Some(g) = setup(&mut t) else { return 0 };
    let fmt = plat::arg_value("--format");
    let Some(&(ext, format)) = (match &fmt {
        None => Some(&("rgba", Format::Rgba8Unorm)),
        Some(f) => DUMP_FORMATS.iter().find(|x| x.0 == f.as_str()),
    }) else {
        out(&format!("FAIL --format: one of {}", DUMP_FORMATS.map(|x| x.0).join(", ")));
        return 2;
    };
    // (the tests' images: linear; --format: as RendererDesc::new has it, --linear: without the shader's sRGB)
    let mut desc = RendererDesc::new(format);
    desc.encode_srgb = fmt.is_some() && !plat::args().iter().any(|a| a == "--linear");
    let mut names = Vec::new();
    let voll = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device");
    let basis = t.res(g.adapter.create_device(&DeviceDesc { max_level: Level::Basis }), "Basis device");
    for (level, dev) in [("voll", voll), ("basis", basis)] {
        let Some(dev) = dev else { continue };
        let Some(imgs) = gallery_image_in(&mut t, &dev, desc) else { continue };
        for (i, name) in GALLERY_NAMES.iter().chain(LATER.iter()).enumerate() {
            let file = format!("{dir}/{level}_{name}.{ext}");
            t.ok(plat::write_file(&file, &imgs[i]), &format!("write {file}"));
            names.push(file);
        }
    }
    out(&format!("adapter: {}, {} files", g.adapter.info().name, names.len()));
    (t.fails > 0) as i32
}

/// (G5e3) the shadow gallery on a device limited to Basis, offscreen, sRGB codes: once with Shadows::DEFAULT (4 x 2048,
/// what Voll takes) and once with Shadows::for_level(Basis) (2 x 1024): what the Basis default looks like.  Writes
/// DIR/basis_4x2048.rgba and DIR/basis_2x1024.rgba, rows of R, G, B, A; --res WxH (1920 x 1080)
fn shadow_shots(dir: &str) -> i32 {
    let mut t = T { fails: 0, validate: false };
    let Some(g) = setup(&mut t) else { return 0 };
    let (w, h) = plat::arg_value("--res")
        .and_then(|r| {
            let mut p = r.split('x').filter_map(|x| x.parse::<u32>().ok());
            Some((p.next()?, p.next()?))
        })
        .unwrap_or((1920, 1080));
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc { max_level: Level::Basis }), "Basis device") else { return 1 };
    for (name, sh) in [("basis_4x2048", render3d::Shadows::DEFAULT), ("basis_2x1024", render3d::Shadows::for_level(dev.level()))] {
        let Some(mut r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new") else { return 1 };
        let Some(mut s) = t.res(shadow_gallery_with(&mut r, &sh, w as f64 / h as f64), "the shadow scene") else { return 1 };
        let Some(mut off) = t.res(Off::new(&dev, w, h, Format::Rgba8Unorm), "target") else { return 1 };
        let Some(px) = shot(&mut t, &dev, &mut off, &mut r, &mut s, name) else { return 1 };
        let file = format!("{dir}/{name}.rgba");
        t.ok(plat::write_file(&file, &px), &format!("write {file}: {w} x {h}, {} cascades of {} up to {}", sh.cascades, sh.size, sh.distance));
        let cs: Vec<String> = s.cascades().iter().map(|c| format!("{:.2} .. {:.2}: texel {:.4}", c.near, c.far, c.texel)).collect();
        out(&format!("  cascades (along the view; a texel in world units): {}", cs.join("; ")));
    }
    out(&format!("adapter: {}, level {:?}", g.adapter.info().name, dev.level()));
    (t.fails > 0) as i32
}

pub fn run() -> i32 {
    let args = plat::args();
    let validate = args.iter().any(|a| a == "--validate");
    if validate {
        plat::setenv("jfgui_GPU_VALIDATE", Some("1"));
    }
    if let Some(p) = plat::arg_value("--gen-shaders") {
        return gen_shaders(&p);
    }
    if let Some(p) = plat::arg_value("--gen-dfg") {
        return gen_dfg(&p);
    }
    if let Some(d) = plat::arg_value("--dump") {
        return dump(&d);
    }
    if let Some(d) = plat::arg_value("--shadow-shots") {
        return shadow_shots(&d);
    }
    if args.iter().any(|a| a == "--bench-upload") {
        return crate::bench::bench_upload();
    }
    if args.iter().any(|a| a == "--bench-gltf") {
        return crate::bench::bench_gltf();
    }
    if let Some(n) = plat::arg_value("--bench").and_then(|v| v.parse::<u32>().ok()).filter(|&n| n > 0) {
        return crate::bench::bench(n, validate);
    }
    let Some(test) = plat::arg_value("--test") else {
        plat::write_stderr(b"render3dtest: --test NAME [--validate], --bench N, --gen-shaders FILE, --dump DIR, --shadow-shots DIR (see apps/render3dtest/main.rs)\n");
        return 2;
    };
    let mut t = T { fails: 0, validate };
    let list: [(&str, fn(&mut T)); 20] = [("shaders", shaders), ("pixels", pixels), ("visibility", visibility), ("changes", changes), ("many", many),
                                          ("levels", levels), ("coords", coords), ("frames", frames), ("errors", errors), ("textures", crate::tex::textures),
                                          ("mirror", crate::tex::mirror), ("gltf", crate::gltftest::gltf), ("pbr", crate::pbr::pbr),
                                          ("shadows", crate::shadowtest::shadows), ("lights", crate::lighttest::lights), ("env", crate::envtest::env),
                                          ("occlusion", crate::occl::occlusion), ("lods", crate::lodtest::lods), ("alpha", crate::alphatest::alpha),
                                          ("srgb", crate::srgbtest::srgb)];
    // --test A,B,...: those tests in the list's order; --except A,B,...: not those (for `all`: the suite runs the
    // validation in parts side by side, the rest of `all` in one, so a new test is in it by itself)
    let names: Vec<&str> = test.split(',').collect();
    let except_arg = plat::arg_value("--except").unwrap_or_default();
    let except: Vec<&str> = except_arg.split(',').filter(|n| !n.is_empty()).collect();
    let known = |n: &str| n == "window" || list.iter().any(|(m, _)| *m == n);
    if let Some(bad) = names.iter().chain(except.iter()).find(|n| !(known(n) || **n == "all")) {
        plat::write_stderr(format!("render3dtest: no test {bad}\n").as_bytes());
        return 2;
    }
    let chosen = |n: &str| (names.contains(&n) || (n != "window" && names.contains(&"all"))) && !except.contains(&n);
    let mut ran = false;
    for (name, f) in list {
        if chosen(name) {
            out(&format!("== {name}{}", if validate { " (validation)" } else { "" }));
            f(&mut t);
            ran = true;
        }
    }
    if chosen("window") {
        out(&format!("== window{}", if validate { " (validation)" } else { "" }));
        window(&mut t);
        ran = true;
    }
    if !ran {
        plat::write_stderr(format!("render3dtest: no test in {test}\n").as_bytes());
        return 2;
    }
    if validate {
        let (e, w) = gpu::validation_messages();
        t.ok(e == 0, &format!("validation layer: {e} errors, {w} warnings"));
    }
    if t.fails > 0 {
        out(&format!("{} check(s) failed", t.fails));
        1
    } else {
        out("all checks passed");
        0
    }
}
