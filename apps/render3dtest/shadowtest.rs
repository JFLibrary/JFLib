// shadowtest -- render3dtest's tests of the sun's cascaded shadows (G5c), on Voll and on a device limited to
// Basis, against exact shadow rays of the ray caster (cpu.rs compare_shadows): a ground with cubes, a ball, a
// pole, a mirrored cube and a MASK plate whose holes let the sun through, across all four cascades; the same at
// 1e7 from the origin; MASK casting full shadows when the shadow pass has no fragment shader; one cascade
// against four; the cascades on the CPU (radius independent of the camera's turn, centres on the texel grid)
// and the map itself read back: a camera moved by a part of a texel draws the same map, by whole texels the
// same map shifted.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{Camera, Renderer, Scene, Shadows};

use crate::common::*;
use crate::cpu::{self, RefScene};
use crate::pbr::{Lit, build_lit};
use crate::tests::{T, on_levels, out, ref_camera, shot};
use crate::tex::Tex;

pub fn shadows(t: &mut T) {
    cascades_on_cpu(t);
    on_levels(t, &mut |t, dev, level| { let _ = shadows_on(t, dev, level); });
}

/// the scene with shadows and its reference (exact shadow rays)
fn build_shadowed(t: &mut T, dev: &Device, at: [f64; 3], shadows: &Shadows) -> Option<(Lit, Camera)> {
    let (geos, pics, items) = shadow_scene(at);
    let texs: Vec<Tex> = pics.into_iter().map(|(n, px, srgb)| Tex::exact(n, px, srgb)).collect();
    let cam = shadow_camera(at);
    let mut l = build_lit(t, dev, Format::Rgba8Unorm, &shadow_sun(), &geos, &texs, &items, &cam)?;
    l.s.set_origin([at[0], at[1], at[2]]);
    t.res(l.s.set_shadows(Some(shadows)), "set_shadows")?;
    l.rs.shadows = true;
    Some((l, cam))
}

fn judge(t: &mut T, rs: &RefScene, cam: &Camera, img: &[u8], inside_most: f64, what: &str) -> cpu::ShadowCompared {
    let c = cpu::compare_shadows(rs, &ref_camera(cam), SHADOW_W, SHADOW_H, CLEAR8, img, 1);
    let fine = c.bad == 0 && c.edges_bad == 0 && (c.inside as f64) <= inside_most * c.on_objects as f64;
    t.ok(fine, &format!("{what}: {}", c.text()));
    if let Some((x, y, _, _)) = c.first {
        let rc = ref_camera(cam);
        for (dx, dy) in [(0.0, 0.0), (-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
            if let Some(h) = rs.trace(rc.eye, rc.ray(x as f64 + 0.5 + dx, y as f64 + 0.5 + dy, SHADOW_W, SHADOW_H), rc.near, rc.far) {
                let px = crate::tests::pixel_of(img, SHADOW_W, (x as f64 + dx) as u32, (y as f64 + dy) as u32);
                out(&format!("    at {dx}, {dy}: got {px:?}, inst {} tri {} rgb {:?} linear {:?} sun {:?} vis {}", h.inst, h.tri, h.rgb, h.linear, h.sun, h.vis));
            }
        }
    }
    c
}

fn shadows_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let mut off = t.res(Off::new(dev, SHADOW_W, SHADOW_H, Format::Rgba8Unorm), "target")?;
    // at the origin: every cascade in use
    let (mut l, cam) = build_shadowed(t, dev, [0.0; 3], &SHADOWS)?;
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "shadows")?;
    let cs = l.s.cascades().to_vec();
    out(&format!("  {level}: cascades (near .. far, radius, texel): {}", cs.iter().map(|c| format!("{:.1} .. {:.1} r {:.2} t {:.4}", c.near, c.far, c.radius, c.texel)).collect::<Vec<_>>().join("; ")));
    let base = judge(t, &l.rs, &cam, &img, 0.0005, &format!("{level}: the sun's shadows, 4 cascades of 2048 up to 90, against exact shadow rays"));
    l.rs.shadows = false;
    let c = cpu::compare_tol(&l.rs, &ref_camera(&cam), SHADOW_W, SHADOW_H, CLEAR8, &img, None, 1);
    l.rs.shadows = true;
    t.ok(c.bad > 2000, &format!("{level}: the test can tell: the same image against the scene without shadows: {} wrong", c.bad));
    // MASK without the shadow pass's fragment shader: its holes cast shadow too
    l.r.set_shadow_mask(false);
    let full = shot(t, dev, &mut off, &mut l.r, &mut l.s, "MASK, full shadows")?;
    let (n, _) = diff(&img, &full);
    let darker = img.chunks_exact(4).zip(full.chunks_exact(4)).filter(|(a, b)| (b[0] as u32 + b[1] as u32 + b[2] as u32) + 30 < a[0] as u32 + a[1] as u32 + a[2] as u32).count();
    let c = cpu::compare_shadows(&l.rs, &ref_camera(&cam), SHADOW_W, SHADOW_H, CLEAR8, &full, 1);
    t.ok(n > 0 && darker > 50 && c.at_edges + c.inside > base.at_edges + base.inside + 50,
         &format!("{level}: the shadow pass without a fragment shader: MASK's holes cast shadow ({n} pixels differ, {darker} darker; against the rays {} on the other side instead of {})",
                  c.at_edges + c.inside, base.at_edges + base.inside));
    l.r.set_shadow_mask(true);
    // one cascade over the same distance: coarser
    let one = Shadows { cascades: 1, ..SHADOWS };
    t.res(l.s.set_shadows(Some(&one)), "set_shadows")?;
    let img1 = shot(t, dev, &mut off, &mut l.r, &mut l.s, "one cascade")?;
    let c1 = cpu::compare_shadows(&l.rs, &ref_camera(&cam), SHADOW_W, SHADOW_H, CLEAR8, &img1, 1);
    t.ok(c1.bad == 0, &format!("{level}: one cascade of 2048 up to 90 (for comparison): {}", c1.text()));
    // far away: positions and camera at 1e7, the origin there
    let far = [1e7 + 0.375, 0.0, -1e7 + 0.625];
    let (mut l, cam) = build_shadowed(t, dev, far, &SHADOWS)?;
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "shadows at 1e7")?;
    judge(t, &l.rs, &cam, &img, 0.0005, &format!("{level}: the same at 1e7 from the origin (the origin there)"));
    // stable: the map of a camera moved by part of a texel along the map's x is the same; by whole texels
    // the same shifted (and the radius never changes with the camera's turn: on the CPU)
    let (mut l, cam) = build_shadowed(t, dev, [0.0; 3], &SHADOWS)?;
    let _ = shot(t, dev, &mut off, &mut l.r, &mut l.s, "stable")?;
    let c0 = l.s.cascades()[0];
    let a = read_layer(t, dev, l.r.shadow_map(), 0)?;
    let mut moved = |l: &mut Lit, texels: f64| -> Option<Vec<f32>> {
        let mut c = cam;
        // a part of a texel off the grid's lines, along the map's x
        let fx = c0.axes[0];
        let d = texels * c0.texel;
        c.eye = [cam.eye[0] + fx[0] * d, cam.eye[1] + fx[1] * d, cam.eye[2] + fx[2] * d];
        l.s.set_camera(&c);
        shot(t, dev, &mut off, &mut l.r, &mut l.s, "moved")?;
        read_layer(t, dev, l.r.shadow_map(), 0)
    };
    let b = moved(&mut l, 0.2)?;
    let snapped_b = l.s.cascades()[0];
    let c = moved(&mut l, 3.2)?;
    let snapped_c = l.s.cascades()[0];
    let size = SHADOWS.size as usize;
    let shift = |m: &[f32], k: i64| -> (u32, u32) {
        // m shifted by k texels along x against a: texels that differ, compared
        let (mut differ, mut compared) = (0u32, 0u32);
        for y in 0..size {
            for x in 0..size {
                let xs = x as i64 + k;
                if xs < 0 || xs >= size as i64 {
                    continue;
                }
                compared += 1;
                if m[y * size + xs as usize].to_bits() != a[y * size + x].to_bits() {
                    differ += 1;
                }
            }
        }
        (differ, compared)
    };
    let steps_b = cpu::floor((cpu::dot(snapped_b.center, c0.axes[0]) - cpu::dot(c0.center, c0.axes[0])) / c0.texel + 0.5);
    let steps_c = cpu::floor((cpu::dot(snapped_c.center, c0.axes[0]) - cpu::dot(c0.center, c0.axes[0])) / c0.texel + 0.5);
    let (db, nb) = shift(&b, -(steps_b as i64));
    let (dc, nc) = shift(&c, -(steps_c as i64));
    let (dn, _) = shift(&c, 0);
    // by whole texels: the matrix's constants round to other f32 values: depths may differ in their last bits
    // and a texel's centre exactly on a triangle's edge may fall to the other side
    let k = -(steps_c as i64);
    let (mut coverage, mut largest) = (0u32, 0.0f32);
    for y in 0..size {
        for x in 0..size {
            let xs = x as i64 + k;
            if xs < 0 || xs >= size as i64 {
                continue;
            }
            let (p, q) = (c[y * size + xs as usize], a[y * size + x]);
            if (p > 0.0) != (q > 0.0) {
                coverage += 1;
            } else {
                largest = largest.max((p - q).abs());
            }
        }
    }
    let covered = a.iter().filter(|v| **v > 0.0).count();
    t.ok(db == 0 && dn > 0 && covered > 1000 && coverage * 10_000 <= nc && largest < 1e-5,
         &format!("{level}: cascade 0's map ({covered} texels with a caster), camera moved by 0.2 texels along the map's x: map moved {steps_b} texels, {db} of {nb} texels differ; by 3.2: moved {steps_c}, {dc} of {nc} differ in their bits (unshifted {dn}): {coverage} covered on one side only, depths at most {largest:.1e} apart"));
    Some(())
}

/// a layer of a depth array texture (Layout::Sampled) through a small compute shader
pub fn read_layer(t: &mut T, dev: &Device, tex: &Texture, layer: u32) -> Option<Vec<f32>> {
    let src = format!("
        @group(0) @binding(0) var tex: texture_depth_2d_array;
        @group(0) @binding(1) var<storage, read_write> out: array<f32>;
        @compute @workgroup_size(16, 16)
        fn main(@builtin(global_invocation_id) id: vec3u) {{
            let d = textureDimensions(tex);
            if id.x < d.x && id.y < d.y {{
                out[id.y * d.x + id.x] = textureLoad(tex, id.xy, {layer}, 0);
            }}
        }}");
    let opt = wgsl::CompileOptions { robust: false, ..wgsl::CompileOptions::new(shader::BASIS) };
    let words = match wgsl::compile(&src, &opt) {
        Ok(o) => o.entries.into_iter().next()?.words,
        Err(e) => {
            t.ok(false, &format!("layer reader: {:?}", e.first().map(|x| format!("{x}"))));
            return None;
        }
    };
    let m = t.res(ShaderModule::from_spirv(dev, &words), "layer reader")?;
    let bgl = t.res(dev.create_bind_group_layout(&[
        Binding { binding: 0, kind: BindingKind::Texture, stages: Stages::COMPUTE },
        Binding { binding: 1, kind: BindingKind::StorageBuffer, stages: Stages::COMPUTE },
    ]), "layer reader")?;
    let pl = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 }), "layer reader")?;
    let pipe = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pl, shader: ShaderStage::new(&m, "main") }), "layer reader")?;
    let (w, h) = (tex.width(), tex.height());
    let size = (w * h * 4) as u64;
    let buf = t.res(dev.create_buffer(&BufferDesc { size, usage: BufferUsage::STORAGE, memory: MemoryKind::Device }), "layer reader")?;
    let rb = t.res(dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "layer reader")?;
    let view = t.res(tex.create_view(&ViewDesc { dim: ViewDim::D2Array, base_mip: 0, mips: 1, base_layer: 0, layers: 0 }), "layer view")?;
    let group = t.res(dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[
        BindEntry { binding: 0, resource: Resource::Texture(&view) },
        BindEntry { binding: 1, resource: Resource::Buffer { buffer: &buf, offset: 0, size: 0 } },
    ] }), "layer reader")?;
    let mut e = t.res(dev.encoder(), "layer reader")?;
    e.set_compute_pipeline(&pipe);
    e.set_bind_group(0, &group);
    e.dispatch(w.div_ceil(16), h.div_ceil(16), 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    e.copy_buffer(&buf, 0, &rb, 0, size);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    let cmd = t.res(e.finish(), "layer reader")?;
    let id = t.res(dev.queue().submit([cmd]), "layer reader")?;
    t.res(dev.wait(id, 10_000_000_000), "layer reader")?;
    let mut d = vec![0f32; (w * h) as usize];
    t.res(rb.read(0, &mut d), "layer reader")?;
    Some(d)
}

/// the cascades on the CPU: the radius does not change when the camera turns or moves; every centre lies on
/// its cascade's texel grid in the light's x and y (world coordinates), also at 1e7
fn cascades_on_cpu(t: &mut T) {
    let Some(g) = crate::tests::setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let Some(mut r) = t.res(Renderer::new(&dev, &render3d::RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new") else { return };
    let Some(mut s) = t.res(Scene::new(&r), "Scene::new") else { return };
    s.set_sun(&shadow_sun());
    if t.res(s.set_shadows(Some(&SHADOWS)), "set_shadows").is_none() {
        return;
    }
    let (mut radii_same, mut on_grid, mut count) = (true, true, 0);
    let mut worst = 0.0f64;
    let mut first: Option<Vec<f64>> = None;
    let mut rng = Rng(17);
    for k in 0..40 {
        let base = if k % 2 == 0 { [0.0; 3] } else { [1e7, 2e5, -1e7] };
        let eye = [base[0] + rng.range(-50.0, 50.0), base[1] + rng.range(1.0, 20.0), base[2] + rng.range(-50.0, 50.0)];
        let target = [eye[0] + rng.range(-10.0, 10.0), eye[1] + rng.range(-5.0, 2.0), eye[2] + rng.range(-10.0, 10.0)];
        s.set_camera(&Camera::look_at(eye, target, [0.0, 1.0, 0.0], 1.0, 16.0 / 9.0, 0.1, f64::INFINITY));
        s.set_origin(base);
        if t.res(s.prepare(&mut r), "prepare").is_none() {
            return;
        }
        let cs = s.cascades();
        let radii: Vec<f64> = cs.iter().map(|c| c.radius).collect();
        match &first {
            None => first = Some(radii),
            Some(f) => radii_same &= *f == radii,
        }
        for c in cs {
            for a in 0..2 {
                let v = cpu::dot(c.center, c.axes[a]) / c.texel;
                let off = (v - cpu::floor(v + 0.5)).abs();
                worst = worst.max(off);
                on_grid &= off < 1e-6;
                count += 1;
            }
        }
        // the frame is never recorded: prepare twice is allowed
    }
    t.ok(radii_same, &format!("CPU: 40 cameras (turned, moved, half of them at 1e7): each cascade's radius the same every time ({:?})", first.unwrap_or_default()));
    t.ok(on_grid, &format!("CPU: the centres on their texel grids in the light's x and y: {count} coordinates, at most {worst:.1e} texels off"));
}
