// t_render -- gputest's graphics and indirect tests (main.rs lists them)
//
// The scene (NDC, y up): a red triangle at depth 0.25 over the lower left, a green one at depth
// 0.75 over the lower right, both counter-clockwise, drawn red first: with the depth test the
// overlap stays red.  Probes in a 64 x 64 target: (12, 48) red only, (32, 48) the overlap (red),
// (51, 48) green only, (32, 6) the clear colour (blue) -- a missing viewport flip, depth test
// or a wrong front face changes one of them.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;

use crate::shaders::{CS_ARGS, FS_COLOR, VS_PULL};
use crate::t_compute::{ADD, Fill, GUARD, MUL, TAIL, readback, storage, verify_fill};
use crate::{T, out, run_wait, setup};

pub const SIZE: u32 = 64;
pub const RED: [u8; 4] = [255, 0, 0, 255];
pub const GREEN: [u8; 4] = [0, 255, 0, 255];
pub const BLUE: [u8; 4] = [0, 0, 255, 255];
pub const CLEAR: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// the triangles' vertices: position, colour (vec4 each)
const VERTS: [[f32; 4]; 12] = [
    [-0.9, -0.9, 0.25, 1.0], [1.0, 0.0, 0.0, 1.0],
    [0.5, -0.9, 0.25, 1.0], [1.0, 0.0, 0.0, 1.0],
    [-0.9, 0.9, 0.25, 1.0], [1.0, 0.0, 0.0, 1.0],
    [-0.5, -0.9, 0.75, 1.0], [0.0, 1.0, 0.0, 1.0],
    [0.9, -0.9, 0.75, 1.0], [0.0, 1.0, 0.0, 1.0],
    [0.9, 0.9, 0.75, 1.0], [0.0, 1.0, 0.0, 1.0],
];

/// the scene's pipeline and buffers for a colour format (and a depth format)
pub struct Scene {
    pub pipe: RenderPipeline,
    pub group: BindGroup,
    pub index: Buffer,
    _verts: Buffer,
}

impl Scene {
    pub fn new(t: &mut T, dev: &Device, color: Format, depth: Option<Format>) -> Option<Scene> {
        let vs = t.res(ShaderModule::from_spirv(dev, VS_PULL), "vertex shader")?;
        let fs = t.res(ShaderModule::from_spirv(dev, FS_COLOR), "fragment shader")?;
        let bgl = t.res(dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageBufferRead,
                                                                 stages: Stages::VERTEX }]), "layout")?;
        let pl = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 }), "pipeline layout")?;
        let pipe = t.res(dev.create_render_pipeline(&RenderPipelineDesc {
            layout: &pl,
            vertex: ShaderStage::new(&vs, "main"),
            fragment: Some(ShaderStage::new(&fs, "main")),
            buffers: &[],
            topology: Topology::Triangles,
            cull: Cull::Back,
            front_cw: false,
            targets: &[ColorTarget { format: color, blend: None, write_mask: 0xF }],
            depth: depth.map(|f| DepthState { format: f, write: true, compare: Compare::Less }),
        }), "render pipeline")?;
        let verts = t.res(dev.create_buffer(&BufferDesc { size: core::mem::size_of_val(&VERTS) as u64, usage: BufferUsage::STORAGE,
                                                          memory: MemoryKind::Device }), "vertex buffer")?;
        t.res(dev.queue().write_buffer(&verts, 0, &VERTS), "write vertices")?;
        let index = t.res(dev.create_buffer(&BufferDesc { size: 24, usage: BufferUsage::INDEX, memory: MemoryKind::Device }),
                          "index buffer")?;
        t.res(dev.queue().write_buffer(&index, 0, &[0u32, 1, 2, 3, 4, 5]), "write indices")?;
        let group = t.res(dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[BindEntry { binding: 0,
            resource: Resource::Buffer { buffer: &verts, offset: 0, size: 0 } }] }), "bind group")?;
        Some(Scene { pipe, group, index, _verts: verts })
    }

    /// set pipeline, vertices and indices inside a render pass
    pub fn bind<'a>(&'a self, e: &mut CommandEncoder<'a>) {
        e.set_render_pipeline(&self.pipe);
        e.set_bind_group(0, &self.group);
        e.set_index_buffer(&self.index, 0, IndexFormat::U32);
    }
}

/// a colour target, a depth target and a read-back buffer of SIZE x SIZE
pub struct Target {
    pub color: Texture,
    pub depth: Texture,
    pub rb: Buffer,
}

impl Target {
    pub fn new(t: &mut T, dev: &Device) -> Option<Target> {
        let color = t.res(dev.create_texture(&TextureDesc::d2(SIZE, SIZE, Format::Rgba8Unorm,
                                                              TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC)), "colour target")?;
        let depth = t.res(dev.create_texture(&TextureDesc::d2(SIZE, SIZE, Format::Depth32Float, TextureUsage::DEPTH_TARGET)),
                          "depth target")?;
        let rb = readback(t, dev, (SIZE * SIZE * 4) as u64)?;
        Some(Target { color, depth, rb })
    }

    pub fn begin<'a>(&'a self, e: &mut CommandEncoder<'a>) {
        e.begin_render(&RenderPassDesc {
            color: &[ColorAttachment { view: self.color.view(), load: Load::Clear, clear: CLEAR, store: true,
                                       before: Layout::Undefined, after: Layout::CopySrc }],
            depth: Some(DepthAttachment { view: self.depth.view(), load: Load::Clear, clear: 1.0, store: false,
                                          before: Layout::Undefined, after: Layout::DepthTarget }),
        });
    }

    /// after end_render: the colour target into the read-back buffer
    pub fn read_back<'a>(&'a self, e: &mut CommandEncoder<'a>) {
        e.copy_texture_to_buffer(TextureRegion::mip(&self.color, 0), &self.rb, DataLayout::default());
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    }

    pub fn pixels(&self, t: &mut T) -> Option<Vec<u8>> {
        let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
        t.res(self.rb.read(0, &mut px), "read pixels")?;
        Some(px)
    }
}

pub fn px(data: &[u8], stride: u32, x: u32, y: u32) -> [u8; 4] {
    let o = (y * stride + x * 4) as usize;
    [data[o], data[o + 1], data[o + 2], data[o + 3]]
}

/// the probes against the expected colours (red only, overlap, green only, clear)
fn probes(t: &mut T, px_: &[u8], want: [[u8; 4]; 4], what: &str) {
    let pts = [(12, 48), (32, 48), (51, 48), (32, 6)];
    let got: Vec<[u8; 4]> = pts.iter().map(|&(x, y)| px(px_, SIZE * 4, x, y)).collect();
    let ok = got.iter().zip(want.iter()).all(|(g, w)| g == w);
    t.ok(ok, &format!("{what}: probes {pts:?} = {got:?}, expected {want:?}"));
}

pub fn graphics(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let dev = &g.dev;
    let Some(scene) = Scene::new(t, dev, Format::Rgba8Unorm, Some(Format::Depth32Float)) else { return };
    let Some(tg) = Target::new(t, dev) else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    tg.begin(&mut e);
    scene.bind(&mut e);
    e.draw_indexed(6, 1, 0, 0, 0);
    e.end_render();
    tg.read_back(&mut e);
    if run_wait(t, dev, e.finish(), "triangles").is_none() {
        return;
    }
    if let Some(p) = tg.pixels(t) {
        probes(t, &p, [RED, RED, GREEN, BLUE], "two triangles, depth test, back-face culling");
        let reds = p.chunks(4).filter(|c| *c == RED).count();
        let greens = p.chunks(4).filter(|c| *c == GREEN).count();
        out(&format!("  {reds} red, {greens} green of {} pixels", SIZE * SIZE));
        t.ok(reds > 900 && greens > 600, "both triangles cover their area");
    }
    // a pipeline of another target format must not fit the pass
    if let Some(other) = Scene::new(t, dev, Format::Bgra8Unorm, Some(Format::Depth32Float)) {
        if let Some(mut e) = t.res(dev.encoder(), "encoder") {
            tg.begin(&mut e);
            e.set_render_pipeline(&other.pipe);
            e.end_render();
            t.ok(matches!(e.finish(), Err(GpuError::Invalid(_))), "pipeline of another colour format: Invalid");
        }
    }
}

pub fn indirect(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let dev = &g.dev;
    let voll = dev.features().draw_indirect_count;
    // the argument writer: a word per dispatch, from push constants
    let Some(m) = t.res(ShaderModule::from_spirv(dev, CS_ARGS), "args shader") else { return };
    let Some(abgl) = t.res(dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageBuffer,
                                                                    stages: Stages::COMPUTE }]), "layout") else { return };
    let Some(apl) = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&abgl], push_constants: 64 }), "layout")
    else { return };
    let Some(apipe) = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &apl, shader: ShaderStage::new(&m, "main") }), "args pipeline") else { return };
    let Some(args) = storage(t, dev, 64, BufferUsage::INDIRECT) else { return };
    let Some(abg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &abgl, entries: &[BindEntry { binding: 0,
        resource: Resource::Buffer { buffer: &args, offset: 0, size: 0 } }] }), "bind group") else { return };
    // words: dispatch (64, 1, 1) at 0; draw 0 (red: 3 indices from 0) at 16; draw 1 (green: 3 from
    // 3) at 36; the draw count 1 at 56
    let words: [u32; 16] = [64, 1, 1, 0, 3, 1, 0, 0, 0, 3, 1, 3, 0, 0, 1, 0];
    // 1) dispatch_indirect with arguments written by the GPU
    let Some(fp) = Fill::new(t, dev) else { return };
    let n = 64 * 64;
    let bytes = (n + TAIL) as u64 * 4;
    let Some(buf) = storage(t, dev, bytes, BufferUsage::NONE) else { return };
    let Some(rb) = readback(t, dev, bytes) else { return };
    let Some(fbg) = fp.group(t, dev, &buf) else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    e.fill_buffer(&buf, 0, bytes, GUARD);
    e.set_compute_pipeline(&apipe);
    e.set_bind_group(0, &abg);
    for (i, w) in words.iter().enumerate() {
        e.push_constants(0, &[i as u32, *w]);
        e.dispatch(1, 1, 1);
        e.barrier(Access::COMPUTE_WRITE, Access::COMPUTE_WRITE);
    }
    e.barrier(Access::COMPUTE_WRITE | Access::COPY_WRITE, Access::INDIRECT | Access::COMPUTE_WRITE);
    e.set_compute_pipeline(&fp.pipe);
    e.set_bind_group(0, &fbg);
    e.push_constants(0, &[n, MUL, ADD]);
    e.dispatch_indirect(&args, 0);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    e.copy_buffer(&buf, 0, &rb, 0, bytes);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    if run_wait(t, dev, e.finish(), "dispatch_indirect").is_some() {
        let mut got = vec![0u32; (n + TAIL) as usize];
        t.res(rb.read(0, &mut got), "read");
        verify_fill(t, &got, n, "dispatch_indirect (64 workgroups from a compute shader)");
    }
    // 2) draw_indexed_indirect: only the red triangle
    let Some(scene) = Scene::new(t, dev, Format::Rgba8Unorm, Some(Format::Depth32Float)) else { return };
    let Some(tg) = Target::new(t, dev) else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    tg.begin(&mut e);
    scene.bind(&mut e);
    e.draw_indexed_indirect(&args, 16);
    e.end_render();
    tg.read_back(&mut e);
    if run_wait(t, dev, e.finish(), "draw_indexed_indirect").is_some() {
        if let Some(p) = tg.pixels(t) {
            probes(t, &p, [RED, RED, BLUE, BLUE], "draw_indexed_indirect (one draw from GPU-written arguments)");
        }
    }
    // 3) Voll: two draws through draw_indexed_indirect_multi and _count
    if !voll {
        out("SKIP: draw_indexed_indirect_multi / _count need level Voll");
        return;
    }
    for multi in [true, false] {
        let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
        tg.begin(&mut e);
        scene.bind(&mut e);
        if multi {
            e.draw_indexed_indirect_multi(&args, 16, 2, 20);
        } else {
            e.draw_indexed_indirect_count(&args, 16, &args, 56, 2, 20);
        }
        e.end_render();
        tg.read_back(&mut e);
        let what = if multi { "draw_indexed_indirect_multi (2 draws)" } else { "draw_indexed_indirect_count (count 1 from the GPU, max 2)" };
        if run_wait(t, dev, e.finish(), what).is_some() {
            if let Some(p) = tg.pixels(t) {
                let want = if multi { [RED, RED, GREEN, BLUE] } else { [RED, RED, BLUE, BLUE] };
                probes(t, &p, want, what);
            }
        }
    }
}
