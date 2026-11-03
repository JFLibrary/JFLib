// render3d -- the core of the 3D engine on lib/gpu (G5a; docs/RENDER3D.md): meshes in shared buffers, a
// table of materials, scenes of instances, a camera, a sun, visibility on the GPU and one instance list
// per mesh.  No draw per object anywhere: a frame is one compute dispatch and one indirect draw per
// mesh (level Basis) or one multi-draw for all of them (level Voll).  The same WGSL on every path:
// compiled ahead (shaders_spv.rs) on Vulkan, handed to the browser as text on WebGPU.
//
//   Renderer::new(&device, &RendererDesc) -> Result<Renderer>   the pipelines and the pools of one colour format
//   renderer.add_mesh(&MeshData) -> Result<MeshId>              positions, normals, uv, indices into the pools
//   renderer.add_material(&Material) -> Result<MaterialId>, set_material(id, &Material)
//   renderer.set_size(w, h)                                     a depth buffer the renderer keeps (or bring your own)
//   Scene::new(&renderer) -> Result<Scene>
//   scene.add(Instance { mesh, material, transform }) -> Result<InstanceId>, remove(id), set_transform(id, t),
//        set_material(id, m), set_mesh(id, m); set_camera(&Camera), set_sun(&Sun), set_origin(pos)
//   a frame:
//        scene.prepare(&mut renderer)?;                 waits for this slot's last frame, uploads what changed
//        renderer.record(&mut encoder, &scene, color, depth)?;      depth None: the renderer's own
//        let id = queue.submit([encoder.finish()?])?;   (or swapchain.present)
//        scene.submitted(id);
//
// Conventions: reverse Z (depth 1 at the near plane, compare Greater, clear with DEPTH_CLEAR; a far plane
// of f64::INFINITY is the default), right-handed world, faces counter-clockwise from outside, colours
// linear (an sRGB target encodes them).  Positions are f64 on the CPU and f32 relative to the scene's
// origin on the GPU.
//
// Errors are gpu's: Invalid for ids and values this crate can judge, the rest from the backend.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;
use core::cell::Cell;

use gpu::*;

pub mod math;
mod scene;

pub use scene::{Camera, FrameStats, Instance, InstanceId, ListInfo, Scene, Sun, Transform, UploadInfo};

/// what a depth attachment of this engine clears to (reverse Z: 0 is infinitely far)
pub const DEPTH_CLEAR: f32 = 0.0;
/// the most frames the CPU may record ahead of the GPU
pub const FRAMES_MAX: usize = 3;

pub(crate) const VIEW_BYTES: u64 = 176;
pub(crate) const LIGHTS_BYTES: u64 = 48;
/// where the lights lie in a frame's staging buffer (behind the view)
pub(crate) const LIGHTS_AT: u64 = 256;
pub(crate) const INSTANCE_WORDS: usize = 16;
pub(crate) const CHANGE_WORDS: usize = 24;
const MATERIAL_WORDS: usize = 16;
pub(crate) const DRAW_BYTES: u64 = 20;
pub(crate) const WG_CULL: u32 = 256;
pub(crate) const WG_SCATTER: u32 = 64;
/// what a texture slot of a material holds when it has none (G5b)
const NO_TEXTURE: u32 = u32::MAX;

/// the shaders, compiled from the *.wgsl beside this file (render3dtest --test shaders compares them with
/// what wgsl::compile makes of them, --gen-shaders writes them again); the browser takes the WGSL itself
#[cfg(not(target_arch = "wasm32"))]
mod spv {
    include!("shaders_spv.rs");
}

/// the WGSL sources and their embedded words (render3dtest: the embedding check)
pub const CULL_WGSL: &str = include_str!("cull.wgsl");
pub const SCATTER_WGSL: &str = include_str!("scatter.wgsl");
pub const DRAW_WGSL: &str = include_str!("draw.wgsl");
#[cfg(not(target_arch = "wasm32"))]
pub const CULL_SPV: &[u32] = spv::CULL;
#[cfg(not(target_arch = "wasm32"))]
pub const SCATTER_SPV: &[u32] = spv::SCATTER;
#[cfg(not(target_arch = "wasm32"))]
pub const VS_SPV: &[u32] = spv::VS;
#[cfg(not(target_arch = "wasm32"))]
pub const FS_SPV: &[u32] = spv::FS;

/// Renderer::new
#[derive(Clone, Copy, Debug)]
pub struct RendererDesc {
    /// the format of the colour targets
    pub color: Format,
    /// frames the CPU may record ahead of the GPU (1 ..= FRAMES_MAX); G1 measured 2 to be enough
    pub frames_in_flight: usize,
}

impl RendererDesc {
    pub const fn new(color: Format) -> RendererDesc {
        RendererDesc { color, frames_in_flight: 2 }
    }
}

/// a mesh's data: one normal and one uv per position, three indices per triangle, counter-clockwise from
/// outside.  Copied into the pools (a vertex is 32 bytes; 16-bit indices up to 65536 vertices, of which
/// the GPU is fond); the renderer keeps no copy.
#[derive(Clone, Copy, Debug)]
pub struct MeshData<'a> {
    pub positions: &'a [[f32; 3]],
    pub normals: &'a [[f32; 3]],
    pub uvs: &'a [[f32; 2]],
    pub indices: &'a [u32],
}

/// a mesh of a Renderer
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshId(pub u32);

/// a material of a Renderer
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialId(pub u32);

/// where a mesh lies in the pools, and its bounding sphere in its own coordinates
#[derive(Clone, Copy, Debug)]
pub struct MeshInfo {
    pub first_index: u32,
    pub indices: u32,
    pub base_vertex: u32,
    pub vertices: u32,
    /// its indices are 32 bits wide: more than 65536 vertices
    pub wide: bool,
    pub center: [f64; 3],
    pub radius: f64,
}

/// a row of the material table.  G5a shows the base colour (rgb; alpha reaches the target); the other
/// values wait for PBR (G5c), the table has slots for textures (G5b)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// linear rgb and alpha
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
}

impl Material {
    /// a dull material of one colour
    pub const fn color(r: f32, g: f32, b: f32) -> Material {
        Material { base_color: [r, g, b, 1.0], metallic: 0.0, roughness: 1.0, emissive: [0.0; 3] }
    }

    fn words(&self) -> [u32; MATERIAL_WORDS] {
        let b = |v: f32| v.to_bits();
        let (c, e) = (self.base_color, self.emissive);
        [b(c[0]), b(c[1]), b(c[2]), b(c[3]), b(e[0]), b(e[1]), b(e[2]), b(self.metallic), b(self.roughness), 0, 0, 0,
         NO_TEXTURE, NO_TEXTURE, NO_TEXTURE, NO_TEXTURE]
    }
}

/// one growing buffer of a pool: a new one takes over, the GPU copies the old contents across
pub(crate) struct Pool {
    pub(crate) buf: Buffer,
    usage: BufferUsage,
    /// bytes per element
    size: u64,
    used: u32,
    cap: u32,
}

impl Pool {
    fn new(dev: &Device, usage: BufferUsage, size: u64, cap: u32) -> Result<Pool> {
        let buf = dev.create_buffer(&BufferDesc { size: size * cap as u64, usage, memory: MemoryKind::Device })?;
        Ok(Pool { buf, usage, size, used: 0, cap })
    }

    /// room for n more elements.  Uploads still on their way run first (they lead the next submission),
    /// so the copy has them; the old buffer goes when the GPU is done with it.
    fn reserve(&mut self, dev: &Device, n: u32) -> Result<()> {
        let need = self.used.checked_add(n).ok_or(GpuError::Invalid("the mesh pool is full"))?;
        if need <= self.cap {
            return Ok(());
        }
        let cap = need.max(self.cap.saturating_mul(2));
        let buf = dev.create_buffer(&BufferDesc { size: self.size * cap as u64, usage: self.usage, memory: MemoryKind::Device })?;
        if self.used > 0 {
            let mut e = dev.encoder()?;
            e.barrier(Access::COPY_WRITE, Access::COPY_READ);
            e.copy_buffer(&self.buf, 0, &buf, 0, self.size * self.used as u64);
            e.barrier(Access::COPY_WRITE, Access::COPY_READ | Access::COPY_WRITE | Access::INDEX_VERTEX);
            dev.queue().submit([e.finish()?])?;
        }
        self.buf = buf;
        self.cap = cap;
        Ok(())
    }
}

/// the pipelines of one colour format, the mesh pools and the material table of one device
pub struct Renderer {
    pub(crate) dev: Device,
    pub(crate) desc: RendererDesc,
    /// level Voll: one multi-draw for all lists; else one indirect draw per list
    pub(crate) multi: bool,
    pub(crate) max_instances: u32,
    cull: ComputePipeline,
    scatter: ComputePipeline,
    draw: RenderPipeline,
    pub(crate) cull_layout: BindGroupLayout,
    pub(crate) scatter_layout: BindGroupLayout,
    pub(crate) draw_layout: BindGroupLayout,
    pub(crate) vertices: Pool,
    /// 16-bit indices as long as a mesh's vertices allow it (they are relative to its base vertex): on the
    /// RTX 2070 SUPER a draw of 32-bit indices takes a third to a half longer (docs/RENDER3D.md)
    indices16: Pool,
    indices32: Pool,
    pub(crate) meshes: Vec<MeshInfo>,
    materials: Vec<[u32; MATERIAL_WORDS]>,
    pub(crate) material_buf: Buffer,
    /// counts the material and vertex buffers (a scene's bindings follow)
    pub(crate) bindings_gen: u32,
    material_cap: u32,
    material_dirty: Option<(u32, u32)>,
    /// the rows that changed (buffer, first byte, bytes), for the next frame's commands
    material_up: Option<(Buffer, u64, u64)>,
    material_up_done: Cell<bool>,
    depth: Option<Texture>,
    depth_used: Cell<bool>,
}

#[cfg(not(target_arch = "wasm32"))]
fn modules(dev: &Device) -> Result<(ShaderModule, ShaderModule, ShaderModule, ShaderModule)> {
    Ok((ShaderModule::from_spirv(dev, CULL_SPV)?, ShaderModule::from_spirv(dev, SCATTER_SPV)?, ShaderModule::from_spirv(dev, VS_SPV)?,
        ShaderModule::from_spirv(dev, FS_SPV)?))
}

#[cfg(target_arch = "wasm32")]
fn modules(dev: &Device) -> Result<(ShaderModule, ShaderModule, ShaderModule, ShaderModule)> {
    let d = ShaderModule::from_wgsl(dev, DRAW_WGSL)?;
    Ok((ShaderModule::from_wgsl(dev, CULL_WGSL)?, ShaderModule::from_wgsl(dev, SCATTER_WGSL)?, d.clone(), d))
}

impl Renderer {
    pub fn new(dev: &Device, desc: &RendererDesc) -> Result<Renderer> {
        if !(1..=FRAMES_MAX).contains(&desc.frames_in_flight) {
            return Err(GpuError::Invalid("frames_in_flight: 1 ..= 3"));
        }
        let (cull_m, scatter_m, vs, fs) = modules(dev)?;
        let b = |binding, kind, stages| Binding { binding, kind, stages };
        let (cs, vf) = (Stages::COMPUTE, Stages::VERTEX);
        let cull_layout = dev.create_bind_group_layout(&[
            b(0, BindingKind::UniformBuffer, cs),
            b(1, BindingKind::StorageBufferRead, cs),
            b(2, BindingKind::StorageBufferRead, cs),
            b(3, BindingKind::StorageBufferRead, cs),
            b(4, BindingKind::StorageBuffer, cs),
            b(5, BindingKind::StorageBuffer, cs),
        ])?;
        let scatter_layout = dev.create_bind_group_layout(&[
            b(0, BindingKind::StorageBufferRead, cs),
            b(1, BindingKind::StorageBuffer, cs),
            b(2, BindingKind::StorageBuffer, cs),
            b(3, BindingKind::StorageBuffer, cs),
        ])?;
        let draw_layout = dev.create_bind_group_layout(&[
            b(0, BindingKind::UniformBuffer, vf),
            b(1, BindingKind::StorageBufferRead, vf),
            b(2, BindingKind::UniformBuffer, Stages::FRAGMENT),
            b(3, BindingKind::StorageBufferRead, Stages::FRAGMENT),
            b(4, BindingKind::StorageBufferRead, vf),
            b(5, BindingKind::StorageBufferRead, vf),
        ])?;
        let cull_pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&cull_layout], push_constants: 8 })?;
        let scatter_pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&scatter_layout], push_constants: 4 })?;
        let draw_pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&draw_layout], push_constants: 4 })?;
        let cull = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &cull_pl, shader: ShaderStage::new(&cull_m, "main") })?;
        let scatter = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &scatter_pl, shader: ShaderStage::new(&scatter_m, "main") })?;
        let draw = dev.create_render_pipeline(&RenderPipelineDesc {
            layout: &draw_pl,
            vertex: ShaderStage::new(&vs, "vs"),
            fragment: Some(ShaderStage::new(&fs, "fs")),
            buffers: &[],
            topology: Topology::Triangles,
            cull: Cull::Back,
            front_cw: false,
            targets: &[ColorTarget { format: desc.color, blend: None, write_mask: 0xF }],
            depth: Some(DepthState { format: Format::Depth32Float, write: true, compare: Compare::Greater }),
        })?;
        let (f, l) = (dev.features(), dev.limits());
        let multi = dev.level() == Level::Voll && f.multi_draw_indirect && f.draw_indirect_first_instance;
        // what the bindings and one dispatch can reach
        let by_binding = (l.max_storage_binding / INSTANCE_WORDS as u32 / 4) as u64;
        let by_dispatch = l.max_compute_workgroups[0] as u64 * WG_CULL as u64;
        let max_instances = by_binding.min(by_dispatch).min(u32::MAX as u64) as u32;
        let vertex = BufferUsage::STORAGE;
        Ok(Renderer {
            vertices: Pool::new(dev, vertex, 32, 1 << 12)?,
            indices16: Pool::new(dev, BufferUsage::INDEX, 2, 1 << 14)?,
            indices32: Pool::new(dev, BufferUsage::INDEX, 4, 1 << 8)?,
            meshes: Vec::new(),
            materials: Vec::new(),
            material_buf: dev.create_buffer(&BufferDesc { size: 64 * 64, usage: BufferUsage::STORAGE, memory: MemoryKind::Device })?,
            bindings_gen: 0,
            material_cap: 64,
            material_dirty: None,
            material_up: None,
            material_up_done: Cell::new(true),
            depth: None,
            depth_used: Cell::new(false),
            dev: dev.clone(),
            desc: *desc,
            multi,
            max_instances,
            cull,
            scatter,
            draw,
            cull_layout,
            scatter_layout,
            draw_layout,
        })
    }

    pub fn device(&self) -> &Device {
        &self.dev
    }

    /// this device draws its lists with one multi-draw (level Voll)
    pub fn multi_draw(&self) -> bool {
        self.multi
    }

    /// the most instances a scene can hold on this device
    pub fn max_instances(&self) -> u32 {
        self.max_instances
    }

    /// a mesh into the pools (they grow; nothing waits).  The vertices must be there in equal numbers, the
    /// indices in threes and in range, positions finite.
    pub fn add_mesh(&mut self, m: &MeshData) -> Result<MeshId> {
        let n = m.positions.len();
        if n == 0 || m.normals.len() != n || m.uvs.len() != n {
            return Err(GpuError::Invalid("add_mesh: as many normals and uvs as positions, at least one"));
        }
        if m.indices.is_empty() || m.indices.len() % 3 != 0 || m.indices.iter().any(|&i| i as usize >= n) {
            return Err(GpuError::Invalid("add_mesh: indices in threes and in range"));
        }
        if m.positions.iter().any(|p| !(p[0].is_finite() && p[1].is_finite() && p[2].is_finite())) {
            return Err(GpuError::Invalid("add_mesh: positions must be finite"));
        }
        // the bounding sphere: around the middle of the box, as far as the farthest vertex
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in m.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k] as f64);
                hi[k] = hi[k].max(p[k] as f64);
            }
        }
        let center = [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1]), 0.5 * (lo[2] + hi[2])];
        let mut r2 = 0.0f64;
        for p in m.positions {
            r2 = r2.max(math::dot(math::sub([p[0] as f64, p[1] as f64, p[2] as f64], center), math::sub([p[0] as f64, p[1] as f64, p[2] as f64], center)));
        }
        let (dev, q) = (self.dev.clone(), self.dev.queue().clone());
        let (nv, ni) = (n as u32, m.indices.len() as u32);
        let cap = self.vertices.cap;
        self.vertices.reserve(&dev, nv)?;
        if cap != self.vertices.cap {
            self.bindings_gen += 1;
        }
        let base = self.vertices.used;
        let vertices: Vec<[f32; 8]> = (0..n).map(|i| {
            let (p, a, t) = (m.positions[i], m.normals[i], m.uvs[i]);
            [p[0], p[1], p[2], t[0], a[0], a[1], a[2], t[1]]
        }).collect();
        q.write_buffer(&self.vertices.buf, base as u64 * 32, &vertices)?;
        let wide = n > 1 << 16;
        let first = if wide {
            self.indices32.reserve(&dev, ni)?;
            let first = self.indices32.used;
            q.write_buffer(&self.indices32.buf, first as u64 * 4, m.indices)?;
            self.indices32.used += ni;
            first
        } else {
            // (uploads go in multiples of 4 bytes in WebGPU: an odd count is padded)
            let room = ni + (ni & 1);
            self.indices16.reserve(&dev, room)?;
            let first = self.indices16.used;
            let mut short: Vec<u16> = m.indices.iter().map(|&v| v as u16).collect();
            short.resize(room as usize, 0);
            q.write_buffer(&self.indices16.buf, first as u64 * 2, &short)?;
            self.indices16.used += room;
            first
        };
        self.vertices.used += nv;
        self.meshes.push(MeshInfo { first_index: first, indices: ni, base_vertex: base, vertices: nv, wide, center, radius: math::sqrt(r2) });
        Ok(MeshId(self.meshes.len() as u32 - 1))
    }

    pub fn mesh_info(&self, id: MeshId) -> Option<MeshInfo> {
        self.meshes.get(id.0 as usize).copied()
    }

    pub fn add_material(&mut self, m: &Material) -> Result<MaterialId> {
        let i = self.materials.len() as u32;
        self.materials.push(m.words());
        self.mark_material(i);
        Ok(MaterialId(i))
    }

    /// a row of the table; the next frame shows it
    pub fn set_material(&mut self, id: MaterialId, m: &Material) -> Result<()> {
        let Some(row) = self.materials.get_mut(id.0 as usize) else { return Err(GpuError::Invalid("set_material: no such material")) };
        *row = m.words();
        self.mark_material(id.0);
        Ok(())
    }

    fn mark_material(&mut self, i: u32) {
        self.material_dirty = Some(self.material_dirty.map_or((i, i + 1), |(a, b)| (a.min(i), b.max(i + 1))));
    }

    pub(crate) fn material_count(&self) -> usize {
        self.materials.len()
    }

    /// a depth buffer the renderer keeps, for frames recorded with depth None
    pub fn set_size(&mut self, w: u32, h: u32) -> Result<()> {
        if self.depth.as_ref().is_some_and(|d| (d.width(), d.height()) == (w, h)) {
            return Ok(());
        }
        self.depth = Some(self.dev.create_texture(&TextureDesc::d2(w, h, Format::Depth32Float, TextureUsage::DEPTH_TARGET))?);
        self.depth_used.set(false);
        Ok(())
    }

    /// (Scene::prepare) the table's changes for the next frame's commands
    pub(crate) fn prepare_materials(&mut self) -> Result<u64> {
        // the last ones never reached a frame: all of it again
        if !self.material_up_done.get() {
            self.material_dirty = Some((0, self.materials.len() as u32));
        }
        let n = self.materials.len() as u32;
        if n > self.material_cap {
            self.material_cap = n.max(self.material_cap * 2);
            self.material_buf = self.dev.create_buffer(&BufferDesc { size: 64 * self.material_cap as u64, usage: BufferUsage::STORAGE, memory: MemoryKind::Device })?;
            self.bindings_gen += 1;
            self.material_dirty = Some((0, n));
        }
        let Some((a, b)) = self.material_dirty.take() else { return Ok(0) };
        let size = (b - a) as u64 * 64;
        let up = self.dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Upload })?;
        up.write(0, &self.materials[a as usize..b as usize])?;
        self.material_up = Some((up, a as u64 * 64, size));
        self.material_up_done.set(false);
        Ok(size)
    }

    /// one frame of a prepared scene into e: its uploads, its culling, its render pass with a clear of
    /// colour and depth as the attachments say.  depth None: the renderer's own (set_size), cleared
    pub fn record<'a>(&'a self, e: &mut CommandEncoder<'a>, s: &'a Scene, color: ColorAttachment<'a>, depth: Option<DepthAttachment<'a>>) -> Result<()> {
        if !s.prepared {
            return Err(GpuError::Invalid("record: Scene::prepare comes first"));
        }
        if color.view.format() != self.desc.color {
            return Err(GpuError::Invalid("record: the colour target is not in the renderer's format"));
        }
        let depth = match depth {
            Some(d) => {
                if d.view.format() != Format::Depth32Float {
                    return Err(GpuError::Invalid("record: reverse Z wants a Depth32Float attachment"));
                }
                if d.load == Load::Clear && d.clear != DEPTH_CLEAR {
                    return Err(GpuError::Invalid("record: reverse Z: a depth attachment clears to DEPTH_CLEAR"));
                }
                d
            }
            None => {
                let Some(d) = &self.depth else { return Err(GpuError::Invalid("record: no depth attachment: Renderer::set_size") ) };
                if color.view.size() != (d.width(), d.height()) {
                    return Err(GpuError::Invalid("record: the colour target and the renderer's depth buffer differ in size"));
                }
                let before = if self.depth_used.replace(true) { Layout::DepthTarget } else { Layout::Undefined };
                DepthAttachment { view: d.view(), load: Load::Clear, clear: DEPTH_CLEAR, store: false, before, after: Layout::DepthTarget }
            }
        };
        let f = &s.frames[s.slot];
        if let Some(ts) = &s.ts {
            if s.stats_on {
                e.reset_timestamps(ts, s.slot as u32 * 4, 4);
                e.timestamp(ts, s.slot as u32 * 4);
            }
        }
        // the frame before read and wrote everything this one writes
        e.barrier(Access::COPY_READ | Access::COPY_WRITE | Access::COMPUTE_READ | Access::COMPUTE_WRITE | Access::INDIRECT | Access::INDEX_VERTEX | Access::GRAPHICS_READ,
                  Access::COPY_READ | Access::COPY_WRITE | Access::COMPUTE_READ | Access::COMPUTE_WRITE);
        e.copy_buffer(&f.stage, 0, &s.view.ubo, 0, VIEW_BYTES);
        e.copy_buffer(&f.stage, LIGHTS_AT, &s.lights, 0, LIGHTS_BYTES);
        if !self.material_up_done.replace(true) {
            if let Some((up, at, size)) = &self.material_up {
                e.copy_buffer(up, 0, &self.material_buf, *at, *size);
            }
        }
        if let Some(up) = &s.full_up {
            let n = s.slots.len() as u64;
            e.copy_buffer(up, 0, &s.spheres, 0, n * 16);
            e.copy_buffer(up, n * 16, &s.list_of, 0, n * 4);
            e.copy_buffer(up, n * 20, &s.instances, 0, n * 64);
        }
        if s.changed > 0 {
            let Some(group) = &f.scatter_group else { return Err(GpuError::Invalid("record: Scene::prepare comes first")) };
            e.set_compute_pipeline(&self.scatter);
            e.set_bind_group(0, group);
            e.push_constants(0, &[s.changed]);
            e.dispatch(s.changed.div_ceil(WG_SCATTER), 1, 1);
        }
        let lists = s.lists.len() as u64;
        if let Some(up) = &s.lists_up {
            e.copy_buffer(up, 0, &s.list_first, 0, lists * 4);
            e.copy_buffer(up, lists * 4, &s.template, 0, lists * DRAW_BYTES);
            e.barrier(Access::COPY_WRITE, Access::COPY_READ);
        }
        if lists > 0 {
            e.copy_buffer(&s.template, 0, &s.view.draws, 0, lists * DRAW_BYTES);
        }
        e.barrier(Access::COPY_WRITE | Access::COMPUTE_WRITE, Access::COMPUTE_READ | Access::COMPUTE_WRITE | Access::GRAPHICS_READ);
        if let (Some(ts), true) = (&s.ts, s.stats_on) {
            e.timestamp(ts, s.slot as u32 * 4 + 1);
        }
        self.cull(e, s, &s.view);
        if let (Some(ts), true) = (&s.ts, s.stats_on) {
            e.timestamp(ts, s.slot as u32 * 4 + 2);
        }
        e.barrier(Access::COMPUTE_WRITE, Access::INDIRECT | Access::INDEX_VERTEX | Access::COPY_READ);
        if let (true, Some(rb), true) = (s.stats_on, &f.readback, lists > 0) {
            e.copy_buffer(&s.view.draws, 0, rb, 0, lists * DRAW_BYTES);
        }
        e.begin_render(&RenderPassDesc { color: &[color], depth: Some(depth) });
        self.draw_lists(e, s, &s.view);
        e.end_render();
        if let (Some(ts), true) = (&s.ts, s.stats_on) {
            e.timestamp(ts, s.slot as u32 * 4 + 3);
        }
        if s.stats_on {
            e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        }
        Ok(())
    }

    /// one view's visibility (outside a pass).  A later view, a shadow cascade (G5c) or the second round
    /// of Hi-Z (G5d), is a ViewLists of its own through this function
    fn cull<'a>(&'a self, e: &mut CommandEncoder<'a>, s: &'a Scene, v: &'a scene::ViewLists) {
        let n = s.slots.len() as u32;
        if n == 0 {
            return;
        }
        e.set_compute_pipeline(&self.cull);
        e.set_bind_group(0, &v.group);
        e.push_constants(0, &[n, !s.culling as u32]);
        e.dispatch(n.div_ceil(WG_CULL), 1, 1);
    }

    fn bind_indices<'a>(&'a self, e: &mut CommandEncoder<'a>, wide: bool) {
        if wide {
            e.set_index_buffer(&self.indices32.buf, 0, IndexFormat::U32);
        } else {
            e.set_index_buffer(&self.indices16.buf, 0, IndexFormat::U16);
        }
    }

    /// one view's lists into the running pass (a depth-only pipeline can take the same lists)
    fn draw_lists<'a>(&'a self, e: &mut CommandEncoder<'a>, s: &'a Scene, v: &'a scene::ViewLists) {
        if s.lists.is_empty() {
            return;
        }
        e.set_render_pipeline(&self.draw);
        e.set_bind_group(0, &s.render_group);
        if self.multi {
            // lists side by side with the same width of indices are one multi-draw (usually all of them)
            e.push_constants(0, &[0u32]);
            for &(first, count, wide) in &s.runs {
                self.bind_indices(e, wide);
                e.draw_indexed_indirect_multi(&v.draws, first as u64 * DRAW_BYTES, count, DRAW_BYTES as u32);
            }
        } else {
            // WebGPU and Vulkan 1.1: no first instance is sure to work; a list's start comes as a push constant
            let mut bound = None;
            for (i, l) in s.lists.iter().enumerate() {
                if l.count > 0 {
                    if bound != Some(l.wide) {
                        self.bind_indices(e, l.wide);
                        bound = Some(l.wide);
                    }
                    e.push_constants(0, &[l.first]);
                    e.draw_indexed_indirect(&v.draws, i as u64 * DRAW_BYTES);
                }
            }
        }
    }
}
