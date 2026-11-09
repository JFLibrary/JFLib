// occl -- (G5d) render3dtest --test occlusion: the camera's occlusion test against a depth pyramid in two phases
// (docs/RENDER3D.md "Verdeckung").  A scene of walls with 2400 small instances among and behind them (a third
// mirrored) is flown through twice, at the origin and at 1e7, by two scenes of one renderer: one with the test, one
// without.  Every frame the two images must be the same; on the way the camera jumps, instances come and go, the
// origin moves and the target changes its size.  At three frames the image is judged against the ray caster, and
// what the GPU drew against what the rays see: nothing seen may be missing, and fewer must be drawn than the view
// holds.  The pyramid of a first frame is read back against the depth buffer's minima on the CPU; frames through
// the renderer's own depth buffer (set_size, a new size too) give the same images.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{InstanceId, Material, Renderer, Scene};

use crate::common::*;
use crate::cpu::{self, RefScene};
use crate::tests::{T, on_levels, read_depth, ref_camera, shot, sun_of};

pub fn occlusion(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        for at in [[0.0; 3], [1e7, 0.0, 1e7]] {
            let _ = fly(t, dev, level, at);
        }
        let _ = pyramid(t, dev, level);
    });
}

/// slot -> spec of what a scene holds
struct Held {
    ids: Vec<(InstanceId, InstanceId)>,
    specs: Vec<Spec>,
}

impl Held {
    fn add(&mut self, t: &mut T, a: &mut Scene, b: &mut Scene, list: &[Spec]) -> Option<()> {
        for sp in list {
            let ia = t.res(a.add(sp.instance()), "add")?;
            let ib = t.res(b.add(sp.instance()), "add")?;
            self.ids.push((ia, ib));
            self.specs.push(*sp);
        }
        Some(())
    }

    fn remove(&mut self, t: &mut T, a: &mut Scene, b: &mut Scene, k: usize) -> Option<()> {
        let (ia, ib) = self.ids.swap_remove(k);
        self.specs.swap_remove(k);
        t.res(a.remove(ia), "remove")?;
        t.res(b.remove(ib), "remove")
    }

    fn reference(&self, geos: &[Geo], colors: &[[f32; 3]]) -> RefScene {
        let sun = sun_of();
        let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
        for sp in &self.specs {
            rs.add(&geos[sp.mesh as usize], sp, colors[sp.material as usize]);
        }
        rs
    }
}

/// the slots a frame drew: phase 1's lists and phase 2's (wait for the frame first)
pub fn drawn(t: &mut T, dev: &Device, s: &Scene) -> Option<(Vec<u32>, u32, u32)> {
    let lists = s.lists();
    let (ld, lv) = s.late_buffers();
    let bufs = [s.draws_buffer(), s.visible_buffer(), ld, lv];
    let mut rbs = Vec::new();
    let mut e = t.res(dev.encoder(), "encoder")?;
    e.barrier(Access::INDIRECT | Access::INDEX_VERTEX | Access::GRAPHICS_READ, Access::COPY_READ);
    for b in bufs {
        let rb = t.res(dev.create_buffer(&BufferDesc { size: b.size(), usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "read-back")?;
        rbs.push(rb);
    }
    for (b, rb) in bufs.iter().zip(&rbs) {
        e.copy_buffer(b, 0, rb, 0, b.size());
    }
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    let cmd = t.res(e.finish(), "finish")?;
    let id = t.res(dev.queue().submit([cmd]), "submit")?;
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let mut words: Vec<Vec<u32>> = Vec::new();
    for rb in &rbs {
        let mut w = vec![0u32; rb.size() as usize / 4];
        t.res(rb.read(0, &mut w), "read")?;
        words.push(w);
    }
    let (mut slots, mut early, mut late) = (Vec::new(), 0, 0);
    for (phase, (d, v)) in [(&words[0], &words[1]), (&words[2], &words[3])].into_iter().enumerate() {
        for l in &lists {
            let c = d[l.record as usize * 5 + 1].min(l.capacity);
            if phase == 0 { early += c } else { late += c }
            slots.extend_from_slice(&v[l.first as usize..(l.first + c) as usize]);
        }
    }
    Some((slots, early, late))
}

fn fly(t: &mut T, dev: &Device, level: &str, at: [f64; 3]) -> Option<()> {
    let geos = occl_geos();
    let colors = gallery_colors(24);
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    for g in &geos {
        t.res(r.add_mesh(&g.data()), "add_mesh")?;
    }
    for c in &colors {
        t.res(r.add_material(&Material::color(c[0], c[1], c[2])), "add_material")?;
    }
    let (mut a, mut b) = (t.res(Scene::new(&r), "Scene::new")?, t.res(Scene::new(&r), "Scene::new")?);
    b.set_occlusion(false);
    for s in [&mut a, &mut b] {
        s.set_sun(&sun_of());
        s.set_origin(at);
    }
    let mut held = Held { ids: Vec::new(), specs: Vec::new() };
    held.add(t, &mut a, &mut b, &occl_world(at, 2400, 77))?;
    let big = t.res(Off::new(dev, GALLERY_W, GALLERY_H, Format::Rgba8Unorm), "target")?;
    let small_off = t.res(Off::new(dev, 256, 144, Format::Rgba8Unorm), "target")?;
    let mut offs = [big, small_off];
    let mut rng = Rng(4711);
    let (mut same, mut frames, mut late_sum) = (0, 0, 0u32);
    let place = if at[0] == 0.0 { "at 0" } else { "at 1e7" };
    for f in 0..12usize {
        // on the way: instances come (4) and go (7), the origin moves (8), the target shrinks (9..)
        if f == 4 {
            let more = occl_small(at, 150, 99);
            held.add(t, &mut a, &mut b, &more)?;
        }
        if f == 7 {
            for _ in 0..300 {
                let k = 1 + rng.below(held.specs.len() as u32 - 1) as usize;
                held.remove(t, &mut a, &mut b, k)?;
            }
        }
        if f == 8 {
            for s in [&mut a, &mut b] {
                s.set_origin([at[0] + 16.0, at[1], at[2] - 8.0]);
            }
        }
        let o = if f >= 9 { 1 } else { 0 };
        let aspect = offs[o].w as f64 / offs[o].h as f64;
        let cam = occl_camera(at, f, aspect);
        a.set_camera(&cam);
        b.set_camera(&cam);
        let ia = shot(t, dev, &mut offs[o], &mut r, &mut a, "with the occlusion test")?;
        let (slots, early, late) = drawn(t, dev, &a)?;
        let ib = shot(t, dev, &mut offs[o], &mut r, &mut b, "without")?;
        let (frustum, _, _) = drawn(t, dev, &b)?;
        let (n, m) = diff(&ia, &ib);
        same += (n == 0) as u32;
        frames += 1;
        late_sum += late;
        if n != 0 {
            t.ok(false, &format!("{level}, {place}: frame {f}: with the occlusion test against without: {n} pixels differ (largest {m})"));
        }
        if [3, 6, 10].contains(&f) {
            let rs = held.reference(&geos, &colors);
            let rc = ref_camera(&cam);
            let (w, h) = (offs[o].w, offs[o].h);
            let c = cpu::compare(&rs, &rc, w, h, CLEAR8, &ia, None);
            t.ok(c.fine(), &format!("{level}, {place}: frame {f} ({w} x {h}) against the ray caster: {}", c.text()));
            // what the rays see through the pixels' centres must have been drawn
            let mut seen = vec![false; held.specs.len()];
            for y in 0..h {
                for x in 0..w {
                    if let Some(hit) = rs.trace(rc.eye, rc.ray(x as f64 + 0.5, y as f64 + 0.5, w, h), rc.near, rc.far) {
                        seen[hit.inst] = true;
                    }
                }
            }
            let mut slot_of = vec![usize::MAX; held.ids.iter().map(|x| x.0.slot as usize + 1).max().unwrap_or(0)];
            for (k, (ia, _)) in held.ids.iter().enumerate() {
                slot_of[ia.slot as usize] = k;
            }
            let mut got = vec![false; held.specs.len()];
            for &s in &slots {
                if let Some(&k) = slot_of.get(s as usize).filter(|&&k| k != usize::MAX) {
                    got[k] = true;
                }
            }
            let visible = seen.iter().filter(|&&x| x).count();
            let missing = seen.iter().zip(&got).filter(|(s, g)| **s && !**g).count();
            t.ok(missing == 0 && slots.len() < frustum.len(),
                 &format!("{level}, {place}: frame {f}: drawn {} (phase 1 {early}, phase 2 {late}) of {} in the view, {visible} seen by the rays, {missing} of them not drawn",
                          slots.len(), frustum.len()));
        }
    }
    t.ok(same == frames, &format!("{level}, {place}: {frames} frames (a jump, 150 instances added, 300 removed, a new origin, a new size): {same} the same as without the occlusion test"));
    t.ok(late_sum > 0, &format!("{level}, {place}: phase 2 drew {late_sum} instances over the frames"));
    // the renderer's own depth buffer: the same images, also after a new size
    for (k, o) in [0usize, 0, 1].into_iter().enumerate() {
        let (w, h) = (offs[o].w, offs[o].h);
        let cam = occl_camera(at, 11 - k, w as f64 / h as f64);
        a.set_camera(&cam);
        b.set_camera(&cam);
        let want = shot(t, dev, &mut offs[o], &mut r, &mut b, "without")?;
        let got = own(t, dev, &mut r, &mut a, w, h)?;
        let (n, m) = diff(&got, &want);
        t.ok(n == 0, &format!("{level}, {place}: the renderer's own depth buffer ({w} x {h}), frame {k}: {n} pixels differ from without the occlusion test (largest {m})"));
    }
    Some(())
}

/// a frame with the renderer's own depth buffer (set_size), the image read back
fn own(t: &mut T, dev: &Device, r: &mut Renderer, s: &mut Scene, w: u32, h: u32) -> Option<Vec<u8>> {
    t.res(r.set_size(w, h), "set_size")?;
    let color = t.res(dev.create_texture(&TextureDesc::d2(w, h, Format::Rgba8Unorm, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC)), "texture")?;
    let stride = (w * 4).div_ceil(256) * 256;
    let rb = t.res(dev.create_buffer(&BufferDesc { size: stride as u64 * h as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "read-back")?;
    t.res(s.prepare(r), "prepare")?;
    let mut e = t.res(dev.encoder(), "encoder")?;
    let ca = ColorAttachment { view: color.view(), load: Load::Clear, clear: CLEAR, store: true, before: Layout::Undefined, after: Layout::CopySrc };
    t.res(r.record(&mut e, s, ca, None), "record")?;
    e.copy_texture_to_buffer(TextureRegion::mip(&color, 0), &rb, DataLayout { offset: 0, bytes_per_row: stride, rows_per_image: 0 });
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    let cmd = t.res(e.finish(), "finish")?;
    let id = t.res(dev.queue().submit([cmd]), "submit")?;
    s.submitted(id);
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let mut raw = vec![0u8; (stride * h) as usize];
    t.res(rb.read(0, &mut raw), "read")?;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        out.extend_from_slice(&raw[y * stride as usize..y * stride as usize + w as usize * 4]);
    }
    Some(out)
}

/// an R32Float view (one mip) through a small compute shader
fn read_r32(t: &mut T, dev: &Device, view: &TextureView, w: u32, h: u32) -> Option<Vec<f32>> {
    const SRC: &str = "
        @group(0) @binding(0) var tex: texture_2d<f32>;
        @group(0) @binding(1) var<storage, read_write> out: array<f32>;
        @compute @workgroup_size(8, 8)
        fn main(@builtin(global_invocation_id) id: vec3u) {
            let d = textureDimensions(tex);
            if id.x < d.x && id.y < d.y {
                out[id.y * d.x + id.x] = textureLoad(tex, id.xy, 0).x;
            }
        }";
    let opt = wgsl::CompileOptions { robust: false, ..wgsl::CompileOptions::new(shader::BASIS) };
    let words = wgsl::compile(SRC, &opt).ok()?.entries.into_iter().next()?.words;
    let m = t.res(ShaderModule::from_spirv(dev, &words), "reader")?;
    let bgl = t.res(dev.create_bind_group_layout(&[
        Binding { binding: 0, kind: BindingKind::Texture, stages: Stages::COMPUTE },
        Binding { binding: 1, kind: BindingKind::StorageBuffer, stages: Stages::COMPUTE },
    ]), "reader")?;
    let pl = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 }), "reader")?;
    let pipe = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pl, shader: ShaderStage::new(&m, "main") }), "reader")?;
    let size = (w * h * 4) as u64;
    let buf = t.res(dev.create_buffer(&BufferDesc { size, usage: BufferUsage::STORAGE, memory: MemoryKind::Device }), "reader")?;
    let rb = t.res(dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "reader")?;
    let group = t.res(dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[
        BindEntry { binding: 0, resource: Resource::Texture(view) },
        BindEntry { binding: 1, resource: Resource::Buffer { buffer: &buf, offset: 0, size: 0 } },
    ] }), "reader")?;
    let mut e = t.res(dev.encoder(), "reader")?;
    e.set_compute_pipeline(&pipe);
    e.set_bind_group(0, &group);
    e.dispatch(w.div_ceil(8), h.div_ceil(8), 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    e.copy_buffer(&buf, 0, &rb, 0, size);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    let cmd = t.res(e.finish(), "reader")?;
    let id = t.res(dev.queue().submit([cmd]), "reader")?;
    t.res(dev.wait(id, 10_000_000_000), "reader")?;
    let mut d = vec![0f32; (w * h) as usize];
    t.res(rb.read(0, &mut d), "reader")?;
    Some(d)
}

/// the pyramid of a first frame (phase 2 adds nothing to it) against the depth buffer's minima, every level
fn pyramid(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let geos = occl_geos();
    let colors = gallery_colors(24);
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    for g in &geos {
        t.res(r.add_mesh(&g.data()), "add_mesh")?;
    }
    for c in &colors {
        t.res(r.add_material(&Material::color(c[0], c[1], c[2])), "add_material")?;
    }
    // an odd size: the edge texels of every level cover fewer pixels
    let (w, h) = (301u32, 167u32);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    s.set_camera(&occl_camera([0.0; 3], 2, w as f64 / h as f64));
    for sp in occl_world([0.0; 3], 2400, 77) {
        t.res(s.add(sp.instance()), "add")?;
    }
    shot(t, dev, &mut off, &mut r, &mut s, "pyramid")?;
    let depth = read_depth(t, dev, &off)?;
    let Some(tex) = r.pyramid(off.depth.view()) else {
        t.ok(false, &format!("{level}: the frame made no pyramid"));
        return None;
    };
    let mut bad = 0;
    let mut texels = 0;
    for k in 1..=tex.mips() {
        let (vw, vh) = (w.div_ceil(1 << k), h.div_ceil(1 << k));
        let side = (tex.width() >> (k - 1)).max(1);
        let view = t.res(tex.create_view(&ViewDesc { dim: ViewDim::D2, base_mip: k - 1, mips: 1, base_layer: 0, layers: 1 }), "view")?;
        let got = read_r32(t, dev, &view, side, side)?;
        for y in 0..vh {
            for x in 0..vw {
                let mut want = f32::INFINITY;
                for py in (y << k)..((y + 1) << k).min(h) {
                    for px in (x << k)..((x + 1) << k).min(w) {
                        want = want.min(depth[(py * w + px) as usize]);
                    }
                }
                texels += 1;
                if got[(y * side + x) as usize].to_bits() != want.to_bits() {
                    bad += 1;
                }
            }
        }
    }
    t.ok(bad == 0, &format!("{level}: the pyramid of {w} x {h}: {} levels, {texels} texels against the depth buffer's minima on the CPU, {bad} differ", tex.mips()));

    Some(())
}
