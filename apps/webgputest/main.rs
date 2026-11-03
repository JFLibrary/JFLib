// webgputest -- lib/gpu's WebGPU backend behind its API, in the browser (lib/gpu/web_*.rs, web/gpu.js;
// docs/GPU.md "Ausgabe im Browser").  No window: the checks run on the app's worker, step by step, as
// the browser answers (nothing can wait here), and say ok / FAIL on the console; tests/api_web.mjs
// starts it and reads them.
//   adapter and device asynchronously, level Basis, what the browser cannot do said as errors
//   misuse found at once: Invalid and Unsupported, as native code gets them
//   compute: push constants through the uniform block (several values in one submission), pipeline
//   constants by number and by name, storage buffers, read back
//   copies: rows of 256 bytes, write_texture_rects(_at), fill_buffer, Upload and Readback buffers
//   render: vertex pulling, a depth test, draw, draw_indexed, draw_indirect, a sampled texture array
//   (the layout comes from the WGSL), a storage texture, the push block in a fragment shader
//   a lost device (Device::test_lose: device.destroy(), a simulation) and a new one
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use gpu::*;

const COMPUTE: &str = r#"
// f(i) = i * k + c + base: base from the push block, k by number, c by name
struct Push { base: u32, n: u32 }
var<immediate> push: Push;
@id(7) override k: u32 = 1;
override c: u32 = 100;
@group(0) @binding(0) var<storage, read_write> out: array<u32>;
@compute @workgroup_size(16)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if (id.x < push.n) { out[id.x] = id.x * k + c + push.base; }
}
"#;

const RENDER: &str = r#"
struct Place { color: vec4f, z: f32 }
var<immediate> place: Place;
@group(0) @binding(0) var<storage, read> points: array<vec2f>;
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    return vec4f(points[i], place.z, 1.0);
}
@fragment
fn fs() -> @location(0) vec4f {
    return place.color;
}
"#;

const SAMPLED: &str = r#"
@group(0) @binding(0) var tex: texture_2d_array<f32>;
@group(0) @binding(1) var smp: sampler;
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}
@fragment
fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
    return textureSampleLevel(tex, smp, p.xy / 4.0, 1, 0.0);
}
"#;

const STORAGE: &str = r#"
@group(0) @binding(0) var img: texture_storage_2d<rgba8unorm, write>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3u) {
    textureStore(img, id.xy, vec4f(f32(id.x) / 255.0, f32(id.y) / 255.0, 0.5, 1.0));
}
"#;

/// a check that needs the browser's answer: the buffer, and what to do with its bytes
struct Later {
    what: &'static str,
    rb: Buffer,
    size: usize,
    check: Box<dyn Fn(&[u8]) -> bool>,
}

struct Probe {
    step: u32,
    inst: Option<Instance>,
    dev: Option<Device>,
    later: Vec<Later>,
    keep: Vec<Texture>,
    waited: bool,
    ok: u32,
    bad: u32,
    t0: u64,
}

impl Probe {
    // the error this misuse must give: Invalid (at once), or Unsupported (what the browser cannot do)
    fn is<T>(&mut self, r: Result<T>, invalid: bool, what: &str) {
        let got = match r {
            Err(GpuError::Invalid(_)) => invalid,
            Err(GpuError::Unsupported(_)) => !invalid,
            _ => false,
        };
        self.yes(got, what);
    }

    fn yes(&mut self, ok: bool, what: &str) {
        if ok {
            self.ok += 1;
            rt::os::write_stdout(alloc::format!("ok   {what}\n").as_bytes());
        } else {
            self.bad += 1;
            rt::os::write_stdout(alloc::format!("FAIL {what}\n").as_bytes());
        }
    }

    fn end(&mut self) {
        rt::os::write_stdout(alloc::format!("webgputest: {} ok, {} failed, {:.1} ms\n", self.ok, self.bad, (plat::time_us() - self.t0) as f64 / 1000.0).as_bytes());
        self.step = u32::MAX;
    }

    fn first(&mut self, dev: &Device, a: &Adapter) -> Result<()> {
        let info = a.info();
        rt::os::write_stdout(alloc::format!("adapter: {} ({}), level {:?}, limits: texture {}, groups {}, uniform alignment {}\n",
            info.name, info.backend, a.level(), a.limits().max_texture_2d, a.limits().max_bind_groups, a.limits().min_uniform_offset_alignment).as_bytes());
        self.yes(info.backend == "WebGPU" && a.level() == Level::Basis && dev.level() == Level::Basis, "the backend is WebGPU, level Basis");
        self.yes(!dev.features().bindless && !dev.features().timestamps && dev.features().swapchain, "Features: no Bindless, no timestamps, a swapchain");
        self.yes(dev.limits().max_push_constants == PUSH_CONSTANTS_MAX && dev.limits().max_texture_2d >= 8192, "Limits: the push block, textures");
        Ok(())
    }

    // misuse is found at once, what the browser cannot do is said
    fn misuse(&mut self, dev: &Device) -> Result<()> {
        self.is(ShaderModule::from_spirv(dev, &[0x07230203]), false, "from_spirv: Unsupported");
        self.is(dev.wait_idle(), false, "wait_idle: Unsupported (the worker cannot wait)");
        let b = dev.create_buffer(&BufferDesc { size: 256, usage: BufferUsage::STORAGE, memory: MemoryKind::Device })?;
        let mut out = [0u32; 4];
        self.is(dev.queue().read_buffer(&b, 0, &mut out), false, "read_buffer: Unsupported");
        self.is(dev.create_timestamps(2), false, "timestamps: Unsupported");
        self.is(dev.create_bindless(&BindlessDesc { textures: 1, samplers: 1, buffers: 1, storage_textures: 1 }), false, "Bindless: Unsupported");
        self.is(dev.create_buffer(&BufferDesc { size: 16, usage: BufferUsage::STORAGE, memory: MemoryKind::Readback }), false, "a Readback buffer with another use: Unsupported");
        self.is(dev.create_buffer(&BufferDesc { size: 0, usage: BufferUsage::NONE, memory: MemoryKind::Device }), true, "a buffer of 0 bytes: Invalid");
        self.is(dev.queue().write_buffer(&b, 2, &[1u32]), true, "write_buffer at an odd offset: Invalid");
        self.is(dev.queue().write_buffer(&b, 252, &[1u32, 2]), true, "write_buffer outside: Invalid");
        self.is(b.read(0, &mut out), false, "Buffer::read of a device buffer: Unsupported");
        // recorded misuse comes from finish
        let bgl = dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageBuffer, stages: Stages::COMPUTE }])?;
        let bgl2 = dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageBuffer, stages: Stages::COMPUTE }])?;
        self.is(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl, &bgl, &bgl, &bgl], push_constants: 4 }), false, "four groups and push constants: Unsupported");
        let layout = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 8 })?;
        let module = ShaderModule::from_wgsl(dev, COMPUTE)?;
        self.yes(module.entry_points() == vec![(5, "main")], "entry_points from the WGSL");
        let cs = module.constants();
        self.yes(cs.len() == 2 && cs[0].0 == 7 && cs[0].2 == 1.0 && cs[1].0 == 0x10000 && cs[1].2 == 100.0, "constants from the WGSL (with @id, without)");
        self.is(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &layout, shader: ShaderStage::new(&module, "nothing") }), true, "a missing entry point: Invalid");
        let c = [Constant { id: 3, value: 1.0 }];
        self.is(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &layout, shader: ShaderStage { module: &module, entry: "main", constants: &c } }), true, "a constant the module lacks: Invalid");
        let c = [Constant { id: 7, value: 1.0 }, Constant { id: 7, value: 2.0 }];
        self.is(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &layout, shader: ShaderStage { module: &module, entry: "main", constants: &c } }), true, "a constant twice: Invalid");
        let pipe = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &layout, shader: ShaderStage::new(&module, "main") })?;
        let group = dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[BindEntry { binding: 0, resource: Resource::Buffer { buffer: &b, offset: 0, size: 0 } }] })?;
        let group2 = dev.create_bind_group(&BindGroupDesc { layout: &bgl2, entries: &[BindEntry { binding: 0, resource: Resource::Buffer { buffer: &b, offset: 0, size: 0 } }] })?;
        let mut e = dev.encoder()?;
        e.dispatch(1, 1, 1);
        self.is(e.finish(), true, "dispatch without a pipeline: Invalid");
        let mut e = dev.encoder()?;
        e.set_compute_pipeline(&pipe);
        e.push_constants(0, &[1u32, 2, 3]);
        self.is(e.finish(), true, "push constants beyond the block: Invalid");
        let mut e = dev.encoder()?;
        e.set_compute_pipeline(&pipe);
        e.set_bind_group(0, &group2);
        self.is(e.finish(), true, "a group of another layout: Invalid");
        let mut e = dev.encoder()?;
        e.set_compute_pipeline(&pipe);
        e.set_bind_group(0, &group);
        e.copy_buffer(&b, 2, &b, 8, 4);
        self.is(e.finish(), true, "copy_buffer at an odd offset: Invalid");
        let t = dev.create_texture(&TextureDesc::d2(16, 8, Format::Rgba8Unorm, TextureUsage::COPY_SRC | TextureUsage::COPY_DST))?;
        let rb = dev.create_buffer(&BufferDesc { size: 4096, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?;
        let mut e = dev.encoder()?;
        e.copy_texture_to_buffer(TextureRegion::mip(&t, 0), &rb, DataLayout::default());
        self.is(e.finish(), true, "a copy with rows of 64 bytes: Invalid (WebGPU wants 256)");
        let mut e = dev.encoder()?;
        e.copy_texture_to_buffer(TextureRegion::mip(&t, 0), &rb, DataLayout { offset: 0, bytes_per_row: 100, rows_per_image: 0 });
        self.is(e.finish(), true, "a copy with rows of 100 bytes: Invalid");
        let mut e = dev.encoder()?;
        e.draw_indexed_indirect_multi(&b, 0, 1, 20);
        self.is(e.finish(), true, "a Voll command: Invalid");
        let mut e = dev.encoder()?;
        e.draw(3, 1, 0, 0);
        self.is(e.finish(), true, "a draw outside a render pass: Invalid");
        self.is(dev.create_texture(&TextureDesc::d2(0, 8, Format::Rgba8Unorm, TextureUsage::COPY_DST)), true, "a texture without size: Invalid");
        let mut e = dev.encoder()?;
        e.begin_render(&RenderPassDesc { color: &[], depth: None });
        self.is(e.finish(), true, "a render pass without attachments: Invalid");
        Ok(())
    }

    // compute: three values of the push block in one submission, constants by number and by name
    fn compute(&mut self, dev: &Device) -> Result<()> {
        let bgl = dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageBuffer, stages: Stages::COMPUTE }])?;
        let layout = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 8 })?;
        let module = ShaderModule::from_wgsl(dev, COMPUTE)?;
        const N: usize = 1024;
        let buf = dev.create_buffer(&BufferDesc { size: (N * 4) as u64 * 3, usage: BufferUsage::STORAGE, memory: MemoryKind::Device })?;
        let rb = dev.create_buffer(&BufferDesc { size: (N * 4) as u64 * 3, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?;
        let group = |off: u64| dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[BindEntry { binding: 0, resource: Resource::Buffer { buffer: &buf, offset: off, size: (N * 4) as u64 } }] });
        let (g0, g1, g2) = (group(0)?, group((N * 4) as u64)?, group((N * 8) as u64)?);
        let align = dev.limits().min_storage_offset_alignment as usize;
        self.yes((N * 4) % align == 0, "the test's storage offsets are aligned");
        let defaults = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &layout, shader: ShaderStage::new(&module, "main") })?;
        let c = [Constant { id: 7, value: 3.9 }, Constant { id: 0x10000, value: 5.0 }];
        let set = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &layout, shader: ShaderStage { module: &module, entry: "main", constants: &c } })?;
        let mut e = dev.encoder()?;
        e.set_compute_pipeline(&defaults);
        e.set_bind_group(0, &g0);
        e.push_constants(0, &[1000u32, N as u32]);
        e.dispatch(N as u32 / 16, 1, 1);
        e.barrier(Access::COMPUTE_WRITE, Access::COMPUTE_WRITE);
        e.set_bind_group(0, &g1);
        e.push_constants(0, &[2000u32]); // (only base: the push block keeps n)
        e.dispatch(N as u32 / 16, 1, 1);
        e.set_compute_pipeline(&set);
        e.set_bind_group(0, &g2);
        e.push_constants(0, &[3000u32, N as u32 / 2]);
        e.dispatch(N as u32 / 16, 1, 1);
        e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
        e.copy_buffer(&buf, 0, &rb, 0, (N * 4) as u64 * 3);
        let id = dev.queue().submit([e.finish()?])?;
        self.yes(id.0 >= 1, "submit gives a number");
        self.later.push(Later {
            what: "compute: three push values, constants by number and by name",
            rb,
            size: N * 12,
            check: Box::new(|b| {
                let w = |i: usize| u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
                (0..N).all(|i| w(i) == i as u32 + 100 + 1000)
                    && (0..N).all(|i| w(N + i) == i as u32 + 100 + 2000)
                    && (0..N / 2).all(|i| w(2 * N + i) == i as u32 * 3 + 5 + 3000)
                    && (N / 2..N).all(|i| w(2 * N + i) == 0)
            }),
        });
        Ok(())
    }

    // copies: buffers, rows of 256 bytes, rectangles of a frame
    fn copies(&mut self, dev: &Device) -> Result<()> {
        let up = dev.create_buffer(&BufferDesc { size: 256 * 8, usage: BufferUsage::NONE, memory: MemoryKind::Upload })?;
        let data: Vec<u32> = (0..64 * 8).map(|i| 0x0101_0101u32.wrapping_mul(i as u32 + 1)).collect();
        up.write(0, &data)?;
        self.is(up.write(2, &[1u32]), true, "Buffer::write at an odd offset: Invalid");
        let t = dev.create_texture(&TextureDesc::d2(16, 8, Format::Rgba8Unorm, TextureUsage::COPY_SRC | TextureUsage::COPY_DST))?;
        let rb = dev.create_buffer(&BufferDesc { size: 256 * 8, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?;
        let layout = DataLayout { offset: 0, bytes_per_row: 256, rows_per_image: 0 };
        let mut e = dev.encoder()?;
        e.copy_buffer_to_texture(&up, layout, TextureRegion::mip(&t, 0));
        e.copy_texture_to_buffer(TextureRegion::mip(&t, 0), &rb, layout);
        dev.queue().submit([e.finish()?])?;
        let want = data.clone();
        self.later.push(Later {
            what: "buffer -> texture -> buffer with rows of 256 bytes",
            rb,
            size: 256 * 8,
            check: Box::new(move |b| (0..8).all(|y| (0..16).all(|x| {
                let i = y * 64 + x;
                u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]) == want[i]
            }))),
        });
        // ---- rectangles of a larger image into a texture, with and without an origin
        let (w, h) = (64i32, 32i32);
        let img: Vec<u8> = (0..(w * h * 4) as usize).map(|i| (i % 251) as u8).collect();
        let t2 = dev.create_texture(&TextureDesc::d2(32, 16, Format::Bgra8Unorm, TextureUsage::COPY_SRC | TextureUsage::COPY_DST))?;
        let q = dev.queue();
        q.write_texture_rects_at(&t2, &[[8, 8, 32, 16], [-5, -5, 3, 3], [60, 30, 100, 100]], (8, 8), &img, (w * 4) as u32, Layout::Undefined, Layout::CopySrc)?;
        let rb2 = dev.create_buffer(&BufferDesc { size: 256 * 16, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?;
        let mut e = dev.encoder()?;
        e.copy_texture_to_buffer(TextureRegion::mip(&t2, 0), &rb2, DataLayout { offset: 0, bytes_per_row: 256, rows_per_image: 0 });
        q.submit([e.finish()?])?;
        let want = img.clone();
        self.later.push(Later {
            what: "write_texture_rects_at: rectangles of a larger image, what lies outside skipped",
            rb: rb2,
            size: 256 * 16,
            check: Box::new(move |b| (0..16usize).all(|y| (0..32usize).all(|x| {
                let (s, d) = (((y + 8) * 64 + x + 8) * 4, y * 256 + x * 4);
                b[d..d + 4] == want[s..s + 4]
            }))),
        });
        // ---- fill and clear; a texture written by a whole region
        let f = dev.create_buffer(&BufferDesc { size: 64, usage: BufferUsage::STORAGE, memory: MemoryKind::Device })?;
        q.write_buffer(&f, 0, &[0xFFFF_FFFFu32; 16])?;
        let rb3 = dev.create_buffer(&BufferDesc { size: 64, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?;
        let mut e = dev.encoder()?;
        e.fill_buffer(&f, 4, 8, 0);
        e.fill_buffer(&f, 16, 8, 0x1234_5678);
        e.copy_buffer(&f, 0, &rb3, 0, 64);
        q.submit([e.finish()?])?;
        self.later.push(Later {
            what: "fill_buffer: zero and another value, the rest untouched",
            rb: rb3,
            size: 64,
            check: Box::new(|b| {
                let w = |i: usize| u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
                w(0) == !0 && w(1) == 0 && w(2) == 0 && w(3) == !0 && w(4) == 0x1234_5678 && w(5) == 0x1234_5678 && w(6) == !0
            }),
        });
        Ok(())
    }

    // render: pulling from a storage buffer, the push block in both stages, a depth test, draw kinds
    fn render(&mut self, dev: &Device) -> Result<()> {
        let bgl = dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageBufferRead, stages: Stages::VERTEX }])?;
        let layout = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 32 })?;
        let m = ShaderModule::from_wgsl(dev, RENDER)?;
        let pipe = dev.create_render_pipeline(&RenderPipelineDesc {
            layout: &layout,
            vertex: ShaderStage::new(&m, "vs"),
            fragment: Some(ShaderStage::new(&m, "fs")),
            buffers: &[],
            topology: Topology::Triangles,
            cull: Cull::Back,
            front_cw: false,
            targets: &[ColorTarget { format: Format::Rgba8Unorm, blend: None, write_mask: 0xF }],
            depth: Some(DepthState { format: Format::Depth32Float, write: true, compare: Compare::Less }),
        })?;
        const SIZE: u32 = 32;
        let target = dev.create_texture(&TextureDesc::d2(SIZE, SIZE, Format::Rgba8Unorm, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC))?;
        let depth = dev.create_texture(&TextureDesc::d2(SIZE, SIZE, Format::Depth32Float, TextureUsage::DEPTH_TARGET))?;
        // a triangle over the whole target (counter-clockwise), the same one turned round: culled
        let pts: [f32; 12] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0, -1.0, -1.0, -1.0, 3.0, 3.0, -1.0];
        let pb = dev.create_buffer(&BufferDesc { size: 48, usage: BufferUsage::STORAGE, memory: MemoryKind::Device })?;
        dev.queue().write_buffer(&pb, 0, &pts)?;
        let ib = dev.create_buffer(&BufferDesc { size: 12, usage: BufferUsage::INDEX, memory: MemoryKind::Device })?;
        dev.queue().write_buffer(&ib, 0, &[0u32, 1, 2])?;
        let ia = dev.create_buffer(&BufferDesc { size: 16, usage: BufferUsage::INDIRECT, memory: MemoryKind::Device })?;
        dev.queue().write_buffer(&ia, 0, &[3u32, 1, 0, 0])?;
        let group = dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[BindEntry { binding: 0, resource: Resource::Buffer { buffer: &pb, offset: 0, size: 0 } }] })?;
        let rb = dev.create_buffer(&BufferDesc { size: 256 * SIZE as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?;
        let mut e = dev.encoder()?;
        e.begin_render(&RenderPassDesc {
            color: &[ColorAttachment { view: target.view(), load: Load::Clear, clear: [0.0, 0.0, 0.0, 1.0], store: true, before: Layout::Undefined, after: Layout::CopySrc }],
            depth: Some(DepthAttachment { view: depth.view(), load: Load::Clear, clear: 1.0, store: false, before: Layout::Undefined, after: Layout::DepthTarget }),
        });
        e.set_render_pipeline(&pipe);
        e.set_bind_group(0, &group);
        // red at depth 0.5 in the left half
        e.set_scissor(0, 0, SIZE / 2, SIZE);
        e.push_constants(0, &[1.0f32, 0.0, 0.0, 1.0, 0.5]);
        e.draw(3, 1, 0, 0);
        // green behind it, everywhere: only the right half
        e.set_scissor(0, 0, SIZE, SIZE);
        e.push_constants(0, &[0.0f32, 1.0, 0.0, 1.0, 0.75]);
        e.set_index_buffer(&ib, 0, IndexFormat::U32);
        e.draw_indexed(3, 1, 0, 0, 0);
        // turned round: culled, it would be blue in front of all
        e.push_constants(0, &[0.0f32, 0.0, 1.0, 1.0, 0.1]);
        e.draw(3, 1, 3, 0);
        // the upper rows white in front of all, from an indirect draw
        e.set_scissor(0, 0, SIZE, 4);
        e.push_constants(0, &[1.0f32, 1.0, 1.0, 1.0, 0.2]);
        e.draw_indirect(&ia, 0);
        e.end_render();
        e.copy_texture_to_buffer(TextureRegion::mip(&target, 0), &rb, DataLayout { offset: 0, bytes_per_row: 256, rows_per_image: 0 });
        dev.queue().submit([e.finish()?])?;
        self.later.push(Later {
            what: "render: pulling, the push block in both stages, depth, culling, draw, draw_indexed, draw_indirect",
            rb,
            size: 256 * SIZE as usize,
            check: Box::new(|b| {
                let at = |x: usize, y: usize| &b[y * 256 + x * 4..y * 256 + x * 4 + 4];
                (0..SIZE as usize).all(|x| at(x, 2) == [255, 255, 255, 255])
                    && (4..SIZE as usize).all(|y| (0..16).all(|x| at(x, y) == [255, 0, 0, 255]) && (16..SIZE as usize).all(|x| at(x, y) == [0, 255, 0, 255]))
            }),
        });
        // ---- a sampled texture array: the layout comes from the WGSL (the bind group after the pipeline)
        let bgl = dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::Texture, stages: Stages::FRAGMENT }, Binding { binding: 1, kind: BindingKind::Sampler, stages: Stages::FRAGMENT }])?;
        let layout = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 })?;
        let m = ShaderModule::from_wgsl(dev, SAMPLED)?;
        let pipe = dev.create_render_pipeline(&RenderPipelineDesc {
            layout: &layout,
            vertex: ShaderStage::new(&m, "vs"),
            fragment: Some(ShaderStage::new(&m, "fs")),
            buffers: &[],
            topology: Topology::Triangles,
            cull: Cull::None,
            front_cw: false,
            targets: &[ColorTarget { format: Format::Rgba8Unorm, blend: None, write_mask: 0xF }],
            depth: None,
        })?;
        let arr = dev.create_texture(&TextureDesc { width: 4, height: 4, depth: 1, layers: 2, mips: 1, format: Format::Rgba8Unorm, usage: TextureUsage::COPY_DST | TextureUsage::SAMPLED, cube: false })?;
        let texels: Vec<u8> = (0..4 * 4 * 2).flat_map(|i: u32| if i < 16 { [255u8, 0, 0, 255] } else { [10, 20, 30 + (i - 16) as u8, 255] }).collect();
        let whole = TextureRegion { texture: &arr, mip: 0, origin: [0; 3], size: [4, 4, 1], base_layer: 0, layers: 2 };
        dev.queue().write_texture(&whole, &texels, DataLayout { offset: 0, bytes_per_row: 16, rows_per_image: 4 }, Layout::Undefined, Layout::Sampled)?;
        let smp = dev.create_sampler(&SamplerDesc::NEAREST)?;
        let group = dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[BindEntry { binding: 0, resource: Resource::Texture(arr.view()) }, BindEntry { binding: 1, resource: Resource::Sampler(&smp) }] })?;
        let t2 = dev.create_texture(&TextureDesc::d2(4, 4, Format::Rgba8Unorm, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC))?;
        let rb2 = dev.create_buffer(&BufferDesc { size: 256 * 4, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?;
        let mut e = dev.encoder()?;
        e.begin_render(&RenderPassDesc {
            color: &[ColorAttachment { view: t2.view(), load: Load::Discard, clear: [0.0; 4], store: true, before: Layout::Undefined, after: Layout::CopySrc }],
            depth: None,
        });
        e.set_render_pipeline(&pipe);
        e.set_bind_group(0, &group);
        e.draw(3, 1, 0, 0);
        e.end_render();
        e.copy_texture_to_buffer(TextureRegion::mip(&t2, 0), &rb2, DataLayout { offset: 0, bytes_per_row: 256, rows_per_image: 0 });
        dev.queue().submit([e.finish()?])?;
        self.later.push(Later {
            what: "a texture array in a fragment shader, its layout from the WGSL",
            rb: rb2,
            size: 256 * 4,
            check: Box::new(|b| (0..4usize).all(|y| (0..4usize).all(|x| b[y * 256 + x * 4..y * 256 + x * 4 + 4] == [10, 20, 30 + (y * 4 + x) as u8, 255]))),
        });
        // ---- a storage texture: its layout from the bind group (made before the pipeline)
        let bgl = dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageTexture, stages: Stages::COMPUTE }])?;
        let st = dev.create_texture(&TextureDesc::d2(4, 4, Format::Rgba8Unorm, TextureUsage::STORAGE | TextureUsage::COPY_SRC))?;
        let group = dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[BindEntry { binding: 0, resource: Resource::StorageTexture(st.view()) }] })?;
        let layout = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 })?;
        let m = ShaderModule::from_wgsl(dev, STORAGE)?;
        let pipe = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &layout, shader: ShaderStage::new(&m, "main") })?;
        let rb3 = dev.create_buffer(&BufferDesc { size: 256 * 4, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?;
        let mut e = dev.encoder()?;
        e.set_compute_pipeline(&pipe);
        e.set_bind_group(0, &group);
        e.dispatch(4, 4, 1);
        e.copy_texture_to_buffer(TextureRegion::mip(&st, 0), &rb3, DataLayout { offset: 0, bytes_per_row: 256, rows_per_image: 0 });
        dev.queue().submit([e.finish()?])?;
        self.later.push(Later {
            what: "a storage texture in a compute shader, its layout from the bind group",
            rb: rb3,
            size: 256 * 4,
            check: Box::new(|b| (0..4usize).all(|y| (0..4usize).all(|x| b[y * 256 + x * 4] == x as u8 && b[y * 256 + x * 4 + 1] == y as u8 && b[y * 256 + x * 4 + 3] == 255))),
        });
        self.keep.push(target);
        Ok(())
    }

    fn step(&mut self) -> Result<()> {
        match self.step {
            0 => {
                match Instance::new() {
                    Ok(i) => self.inst = Some(i),
                    Err(e) => {
                        rt::os::write_stdout(alloc::format!("SKIP webgputest: {e}\n").as_bytes());
                        self.step = u32::MAX;
                        return Ok(());
                    }
                }
                self.step = 1;
            }
            1 | 10 => {
                let Some(inst) = self.inst.clone() else { return Ok(()) };
                match inst.prepare() {
                    Ok(false) => {
                        if !self.waited {
                            self.waited = true;
                            let r = inst.adapter().map(|_| ());
                            self.yes(r == Err(GpuError::NotReady), "the adapter comes asynchronously: NotReady until then");
                        }
                        return Ok(());
                    }
                    Ok(true) => {}
                    Err(e) => {
                        rt::os::write_stdout(alloc::format!("SKIP webgputest: {e}\n").as_bytes());
                        self.step = u32::MAX;
                        return Ok(());
                    }
                }
                let a = inst.adapter()?;
                let dev = a.create_device(&DeviceDesc::DEFAULT)?;
                self.is(a.create_device(&DeviceDesc::DEFAULT), false, "a second device of an adapter: Unsupported");
                if self.step == 1 {
                    self.first(&dev, &a)?;
                    self.misuse(&dev)?;
                    self.compute(&dev)?;
                    self.copies(&dev)?;
                    self.render(&dev)?;
                    self.dev = Some(dev);
                    self.step = 2;
                } else {
                    // ---- the new device after the lost one
                    let b = dev.create_buffer(&BufferDesc { size: 16, usage: BufferUsage::STORAGE, memory: MemoryKind::Device })?;
                    let rb = dev.create_buffer(&BufferDesc { size: 16, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?;
                    dev.queue().write_buffer(&b, 0, &[7u32, 8, 9, 10])?;
                    let mut e = dev.encoder()?;
                    e.copy_buffer(&b, 0, &rb, 0, 16);
                    dev.queue().submit([e.finish()?])?;
                    self.later.push(Later { what: "a new device works after the lost one", rb, size: 16, check: Box::new(|x| x[0] == 7 && x[12] == 10) });
                    self.dev = Some(dev);
                    self.step = 11;
                }
            }
            2 | 11 => {
                // what the browser has answered; a read before it has is NotReady, not a wait
                while let Some(l) = self.later.first() {
                    let mut out = vec![0u8; l.size];
                    match l.rb.read(0, &mut out) {
                        Err(GpuError::NotReady) => return Ok(()),
                        r => {
                            let good = r.is_ok() && (l.check)(&out);
                            let what = String::from(l.what);
                            self.yes(good, &what);
                            self.later.remove(0);
                        }
                    }
                }
                let Some(dev) = self.dev.clone() else { return Ok(()) };
                dev.poll()?;
                self.yes(dev.poll().is_ok(), "the browser found no validation error in all of it");
                if self.step == 2 {
                    self.step = 3;
                } else {
                    self.end();
                }
            }
            3 => {
                let Some(dev) = self.dev.clone() else { return Ok(()) };
                dev.test_lose();
                self.yes(dev.is_lost(), "a lost device says so (simulated: device.destroy())");
                self.yes(matches!(dev.create_buffer(&BufferDesc { size: 16, usage: BufferUsage::STORAGE, memory: MemoryKind::Device }), Err(GpuError::DeviceLost)), "calls on it: DeviceLost");
                self.yes(matches!(dev.encoder().map(|_| ()), Err(GpuError::DeviceLost)), "an encoder on it: DeviceLost");
                self.dev = None;
                self.inst = Some(Instance::new()?);
                self.step = 10;
            }
            _ => {}
        }
        Ok(())
    }
}

impl plat::web::WebApplication for Probe {
    fn frame_delay(&self) -> u32 {
        0
    }
    fn tick(&mut self, _time_ms: u32) {
        if self.step == u32::MAX {
            return;
        }
        if plat::time_us() > self.t0 + 30_000_000 {
            self.yes(false, "the browser's answers did not come in 30 s");
            self.end();
            return;
        }
        if let Err(e) = self.step() {
            let what = alloc::format!("step {}: {e}", self.step);
            self.yes(false, &what);
            self.end();
        }
    }
    fn control(&mut self, _line: &[u8]) -> Option<String> {
        None
    }
}

fn main() -> i32 {
    let t0 = plat::time_us();
    if !plat::web::install(Box::new(Probe { step: 0, inst: None, dev: None, later: Vec::new(), keep: Vec::new(), waited: false, ok: 0, bad: 0, t0 })) {
        return 2;
    }
    0
}

rt::main!(main());
