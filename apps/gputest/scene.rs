// scene -- the benchmark scene of G1c on lib/gpu: n instanced cubes moved, culled and drawn on
// the GPU with the shaders and the data contract of lib/shader/bench.rs (docs/SHADER.md
// "Benchmark-Vertrag"; the method and the numbers: docs/GPU.md "Messszene")
//
//   Scene::new(t, dev, n, color, slots) -> Option<Scene>   pipelines for one colour format,
//        the buffers for n objects (created once on the GPU: move at time 0), per frame slot a
//        camera staging buffer, a DrawCmd read-back and four timestamps
//   scene.set_camera(slot, &cam)      the slot's camera, before its frame is recorded
//   scene.record(e, slot, mode, time, color, depth)   one frame (below)
//   scene.gpu_ms(slot) / scene.visible(slot)   the four timestamps and the instance count of the
//        slot's last frame (after its submission finished)
//   bench_camera(time, extent, aspect)  the moving camera of the measurements
//
// One frame (Mode::Indirect), as docs/SHADER.md "Ablauf je Frame":
//   timestamp 0; barrier (every access of the frame before -> copy / compute writes)
//   camera staging[slot] -> camera; move, ceil(n / 256) workgroups; timestamp 1
//   draw.instance_count = 0 (fill_buffer); barrier; cull; timestamp 2; barrier (-> indirect,
//   vertex, copy); DrawCmd -> read-back[slot]; render pass: one draw_indexed_indirect;
//   timestamp 3 (GPU times: move = 1 - 0, cull = 2 - 1, draw = 3 - 2)
// Mode::Single: no cull; in the pass one push_constants(object_index) + draw_indexed(36) per
// object.  time None: no move (the objects as uploaded: the image tests).
//
// Two bind group layouts with the contract's bindings: the compute one (0 camera, 1 objects,
// 2 visible, 3 draw; 1..3 writable) and the render one (0, 1, 2, 4, read only), so no vertex
// shader needs a writable binding (Features::vertex_stores; WebGPU allows none).  The objects,
// the visible list and the DrawCmd are single buffers: the GPU runs the frames one after the
// other, the CPU records up to `slots` frames ahead (the caller waits for a slot's previous
// frame before it records into the slot again).

use alloc::format;
use alloc::vec::Vec;

use gpu::*;
use shader::bench::{self, Camera, DrawCmd, Push, Shader};

use crate::T;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// move, cull, one indirect draw of the visible cubes (vs_instanced)
    Indirect,
    /// move, no cull, one draw per object (vs_single, the index as push constant)
    Single,
}

/// timestamps per frame slot
pub const TS: u32 = 4;

pub struct Scene {
    pub n: u32,
    pub seed: u32,
    pub extent: f32,
    mv: ComputePipeline,
    cull: ComputePipeline,
    inst: RenderPipeline,
    single: RenderPipeline,
    cg: BindGroup,
    rg: BindGroup,
    /// n + 1 records: the last one is a guard the shaders never touch
    pub objects: Buffer,
    pub visible: Buffer,
    pub draw: Buffer,
    pub camera: Buffer,
    index: Buffer,
    _cube: Buffer,
    stage: Vec<Buffer>,
    drawrb: Vec<Buffer>,
    pub ts: Option<Timestamps>,
}

/// the push constants of a frame as words (Push: time, count, seed, extent, object_index)
fn push_words(p: &Push) -> [u32; 5] {
    [p.time.to_bits(), p.count, p.seed, p.extent.to_bits(), p.object_index]
}

impl Scene {
    pub fn new(t: &mut T, dev: &Device, n: u32, color: Format, slots: usize) -> Option<Scene> {
        let minor = if dev.level() == Level::Voll { shader::VOLL } else { shader::BASIS };
        let mut module = |s: Shader| t.res(ShaderModule::from_spirv(dev, &s.build(minor)), &format!("shader {}", s.name()));
        let (ms, cs, vi, vsg, fs) = (module(Shader::Move)?, module(Shader::Cull)?, module(Shader::VsInstanced)?,
                                     module(Shader::VsSingle)?, module(Shader::FsLambert)?);
        let b = |binding, kind, stages| Binding { binding, kind, stages };
        let cl = t.res(dev.create_bind_group_layout(&[
            b(bench::B_CAMERA, BindingKind::UniformBuffer, Stages::COMPUTE),
            b(bench::B_OBJECTS, BindingKind::StorageBuffer, Stages::COMPUTE),
            b(bench::B_VISIBLE, BindingKind::StorageBuffer, Stages::COMPUTE),
            b(bench::B_DRAW, BindingKind::StorageBuffer, Stages::COMPUTE),
        ]), "compute bind group layout")?;
        let rl = t.res(dev.create_bind_group_layout(&[
            b(bench::B_CAMERA, BindingKind::UniformBuffer, Stages::VERTEX | Stages::FRAGMENT),
            b(bench::B_OBJECTS, BindingKind::StorageBufferRead, Stages::VERTEX),
            b(bench::B_VISIBLE, BindingKind::StorageBufferRead, Stages::VERTEX),
            b(bench::B_CUBE, BindingKind::StorageBufferRead, Stages::VERTEX),
        ]), "render bind group layout")?;
        let cpl = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&cl], push_constants: bench::PUSH_SIZE }),
                        "compute pipeline layout")?;
        let rpl = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&rl], push_constants: bench::PUSH_SIZE }),
                        "render pipeline layout")?;
        let compute = |t: &mut T, m: &ShaderModule, what: &str| {
            t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &cpl, shader: ShaderStage::new(m, "main") }),
                  what)
        };
        let mv = compute(t, &ms, "move pipeline")?;
        let cull = compute(t, &cs, "cull pipeline")?;
        let render = |t: &mut T, vs: &ShaderModule, what: &str| {
            t.res(dev.create_render_pipeline(&RenderPipelineDesc {
                layout: &rpl,
                vertex: ShaderStage::new(vs, "main"),
                fragment: Some(ShaderStage::new(&fs, "main")),
                buffers: &[],
                topology: Topology::Triangles,
                cull: Cull::Back,
                front_cw: false,
                targets: &[ColorTarget { format: color, blend: None, write_mask: 0xF }],
                depth: Some(DepthState { format: Format::Depth32Float, write: true, compare: Compare::Less }),
            }), what)
        };
        let inst = render(t, &vi, "vs_instanced pipeline")?;
        let single = render(t, &vsg, "vs_single pipeline")?;
        let buf = |t: &mut T, size: u64, usage: BufferUsage, memory: MemoryKind, what: &str| {
            t.res(dev.create_buffer(&BufferDesc { size, usage, memory }), what)
        };
        let objects = buf(t, (n as u64 + 1) * bench::OBJECT_SIZE as u64, BufferUsage::STORAGE, MemoryKind::Device, "objects")?;
        let visible = buf(t, (n as u64 + 1) * 4, BufferUsage::STORAGE, MemoryKind::Device, "visible list")?;
        let draw = buf(t, bench::DRAW_SIZE as u64, BufferUsage::STORAGE | BufferUsage::INDIRECT, MemoryKind::Device, "draw command")?;
        let camera = buf(t, bench::CAMERA_SIZE as u64, BufferUsage::UNIFORM, MemoryKind::Device, "camera")?;
        let cube = buf(t, bench::CUBE_VERTEX_SIZE as u64 * 24, BufferUsage::STORAGE, MemoryKind::Device, "cube vertices")?;
        let index = buf(t, bench::CUBE_INDICES as u64 * 2, BufferUsage::INDEX, MemoryKind::Device, "cube indices")?;
        let q = dev.queue();
        t.res(q.write_buffer(&cube, 0, &bench::cube_bytes()), "write cube")?;
        t.res(q.write_buffer(&index, 0, &bench::cube_indices()), "write indices")?;
        t.res(q.write_buffer(&draw, 0, &DrawCmd::reset().bytes()), "write draw command")?;
        let mut stage = Vec::new();
        let mut drawrb = Vec::new();
        for _ in 0..slots {
            stage.push(buf(t, bench::CAMERA_SIZE as u64, BufferUsage::NONE, MemoryKind::Upload, "camera staging")?);
            drawrb.push(buf(t, bench::DRAW_SIZE as u64, BufferUsage::NONE, MemoryKind::Readback, "draw read-back")?);
        }
        let ts = if dev.features().timestamps { t.res(dev.create_timestamps(TS * slots as u32), "timestamps") } else { None };
        fn entry(binding: u32, buffer: &Buffer) -> BindEntry<'_> {
            BindEntry { binding, resource: Resource::Buffer { buffer, offset: 0, size: 0 } }
        }
        let cg = t.res(dev.create_bind_group(&BindGroupDesc { layout: &cl, entries: &[
            entry(bench::B_CAMERA, &camera), entry(bench::B_OBJECTS, &objects), entry(bench::B_VISIBLE, &visible),
            entry(bench::B_DRAW, &draw),
        ] }), "compute bind group")?;
        let rg = t.res(dev.create_bind_group(&BindGroupDesc { layout: &rl, entries: &[
            entry(bench::B_CAMERA, &camera), entry(bench::B_OBJECTS, &objects), entry(bench::B_VISIBLE, &visible),
            entry(bench::B_CUBE, &cube),
        ] }), "render bind group")?;
        let s = Scene { n, seed: 1, extent: bench::extent_for(n), mv, cull, inst, single, cg, rg, objects, visible, draw, camera,
                        index, _cube: cube, stage, drawrb, ts };
        // the initial state: move at time 0, once (the frames move the objects from there)
        let mut e = t.res(dev.encoder(), "encoder")?;
        s.move_at(&mut e, 0.0);
        e.barrier(Access::COMPUTE_WRITE, Access::COMPUTE_READ | Access::COMPUTE_WRITE | Access::GRAPHICS_READ);
        let cmd = t.res(e.finish(), "initial move")?;
        t.res(q.submit([cmd]), "submit initial move")?;
        Some(s)
    }

    pub fn push(&self, time: f32) -> Push {
        Push { time, count: self.n, seed: self.seed, extent: self.extent, object_index: 0 }
    }

    /// move at `time` (outside a pass; leaves the move pipeline and the compute group bound)
    pub fn move_at<'a>(&'a self, e: &mut CommandEncoder<'a>, time: f32) {
        e.set_compute_pipeline(&self.mv);
        e.set_bind_group(0, &self.cg);
        e.push_constants(0, &push_words(&self.push(time)));
        e.dispatch(self.n.div_ceil(bench::WG_MOVE), 1, 1);
    }

    /// reset the instance count and cull (the camera must be in place)
    pub fn cull_now<'a>(&'a self, e: &mut CommandEncoder<'a>, time: f32) {
        e.fill_buffer(&self.draw, 4, 4, 0);
        e.barrier(Access::COPY_WRITE | Access::COMPUTE_WRITE, Access::COMPUTE_READ | Access::COMPUTE_WRITE);
        e.set_compute_pipeline(&self.cull);
        e.set_bind_group(0, &self.cg);
        e.push_constants(0, &push_words(&self.push(time)));
        e.dispatch(self.n.div_ceil(bench::WG_CULL), 1, 1);
    }

    /// the camera of slot's next frame
    pub fn set_camera(&self, slot: usize, cam: &Camera) -> gpu::Result<()> {
        self.stage[slot].write(0, &cam.bytes())
    }

    fn stamp<'a>(&'a self, e: &mut CommandEncoder<'a>, slot: usize, k: u32) {
        if let Some(ts) = &self.ts {
            e.timestamp(ts, slot as u32 * TS + k);
        }
    }

    /// one frame into e (see the file header); time None: the objects stay as they are
    pub fn record<'a>(&'a self, e: &mut CommandEncoder<'a>, slot: usize, mode: Mode, time: Option<f32>,
                      color: ColorAttachment<'a>, depth: DepthAttachment<'a>) {
        if let Some(ts) = &self.ts {
            e.reset_timestamps(ts, slot as u32 * TS, TS);
        }
        self.stamp(e, slot, 0);
        // the frame before read and wrote everything this one writes: camera, draw, objects,
        // visible list (write after read and write after write)
        e.barrier(Access::COPY_READ | Access::COPY_WRITE | Access::COMPUTE_READ | Access::COMPUTE_WRITE | Access::INDIRECT
                      | Access::GRAPHICS_READ,
                  Access::COPY_WRITE | Access::COMPUTE_WRITE);
        e.copy_buffer(&self.stage[slot], 0, &self.camera, 0, bench::CAMERA_SIZE as u64);
        let t = time.unwrap_or(0.0);
        if let Some(t) = time {
            self.move_at(e, t);
        }
        self.stamp(e, slot, 1);
        if mode == Mode::Indirect {
            self.cull_now(e, t);
            self.stamp(e, slot, 2);
            e.barrier(Access::COMPUTE_WRITE | Access::COPY_WRITE, Access::INDIRECT | Access::GRAPHICS_READ | Access::COPY_READ);
            e.copy_buffer(&self.draw, 0, &self.drawrb[slot], 0, bench::DRAW_SIZE as u64);
        } else {
            self.stamp(e, slot, 2);
            e.barrier(Access::COMPUTE_WRITE | Access::COPY_WRITE, Access::GRAPHICS_READ);
        }
        e.begin_render(&RenderPassDesc { color: &[color], depth: Some(depth) });
        e.set_index_buffer(&self.index, 0, IndexFormat::U16);
        match mode {
            Mode::Indirect => {
                e.set_render_pipeline(&self.inst);
                e.set_bind_group(0, &self.rg);
                e.draw_indexed_indirect(&self.draw, 0);
            }
            Mode::Single => {
                e.set_render_pipeline(&self.single);
                e.set_bind_group(0, &self.rg);
                e.push_constants(0, &push_words(&self.push(t)));
                for i in 0..self.n {
                    e.push_constants(16, &[i]);
                    e.draw_indexed(bench::CUBE_INDICES, 1, 0, 0, 0);
                }
            }
        }
        e.end_render();
        self.stamp(e, slot, 3);
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    }

    /// the four timestamps (ms on the GPU's clock) of the slot's last frame: start, after move,
    /// after cull, after the render pass; None without timestamps
    pub fn gpu_ms(&self, slot: usize) -> Option<[f64; 4]> {
        let ms = self.ts.as_ref()?.read_ms(slot as u32 * TS, TS).ok()?;
        Some([ms[0], ms[1], ms[2], ms[3]])
    }

    /// the DrawCmd of the slot's last Indirect frame (after cull)
    pub fn draw_cmd(&self, slot: usize) -> DrawCmd {
        let mut b = [0u8; 20];
        if self.drawrb[slot].read(0, &mut b).is_err() {
            return DrawCmd::default();
        }
        DrawCmd::from_bytes(&b)
    }

    /// the instance count of the slot's last Indirect frame
    pub fn visible(&self, slot: usize) -> u32 {
        self.draw_cmd(slot).instance_count
    }
}

/// the camera of the measurements at `time` (seconds): inside the disc of cubes, on a circle of
/// half its radius just above the middle, looking 30 degrees beside the centre; it goes round
/// once in 50 s.  About half of the cubes are in view (bounding spheres: a little more).
pub fn bench_camera(time: f32, extent: f32, aspect: f32) -> Camera {
    let (s, c) = bench::sincos_turns(time * 0.02);
    let (r, h) = (extent * 0.5, extent * 0.08);
    let eye = [r * c, h, r * s];
    // inwards (-c, -s) turned by 30 degrees towards the direction of travel (-s, c)
    let (s30, c30) = (0.5f32, 0.866_025_4f32);
    let dir = [-c * c30 - s * s30, 0.0, -s * c30 + c * s30];
    let target = [eye[0] + dir[0] * extent, 0.0, eye[2] + dir[2] * extent];
    Camera::look_at(eye, target, 1.047_197_6, aspect, 0.1, extent * 3.0)
}
