// render3d -- the core of the 3D engine on lib/gpu (G5a to G5d; docs/RENDER3D.md): meshes in shared buffers,
// a table of glTF's PBR materials with textures, scenes of instances, a camera, a sun, visibility on the GPU and one
// instance list per mesh and side (mirrored instances turn their faces round); (G5d) the camera's occlusion test
// against a depth pyramid in two phases.  No draw per object anywhere: a frame is a few compute dispatches and one
// indirect draw per list and phase (level Basis) or one multi-draw per side and phase for all of them (level Voll).  The same WGSL on every path: compiled ahead (shaders_spv.rs) on
// Vulkan, handed to the browser as text on WebGPU.
//
//   Renderer::new(&device, &RendererDesc) -> Result<Renderer>   the pipelines and the pools of one colour format
//   renderer.add_mesh(&MeshData) -> Result<MeshId>              positions, normals, uv, indices into the pools
//   renderer.add_texture(&TextureData) -> Result<TextureId>     rgba into the array of its size class, mip levels
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
//   gltf::upload(&mut renderer, &document) -> Result<Imported>  a jflib-gltf Document: meshes, textures,
//        materials, and its instances for a Scene (gltf.rs)
//
// Conventions: reverse Z (depth 1 at the near plane, compare Greater, clear with DEPTH_CLEAR; a far plane
// of f64::INFINITY is the default), right-handed world, faces counter-clockwise from outside, light in the
// units of KHR_lights_punctual, times the scene's exposure, tone mapped in the fragment shader (Khronos PBR
// Neutral, or none), colours linear (an sRGB target encodes them).  Positions are f64 on the CPU and f32 relative to the scene's
// origin on the GPU.
//
// Errors are gpu's: Invalid for ids and values this crate can judge, the rest from the backend.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::Cell;
use core::sync::atomic::{AtomicU32, Ordering};

use gpu::*;

mod dfg;
mod env;
pub mod gltf;
mod hiz;
pub mod math;
mod scene;
mod shadow;
mod texture;

pub use dfg::{DFG, DFG_N, dfg_table};
pub use env::{Environment, EnvironmentData, SKY_SIZE, Sky, env_dir, env_face, from_half, to_half};
pub use shadow::{CASCADES_MAX, Cascade, Shadows};
pub use scene::{Camera, FrameStats, Instance, InstanceId, Light, LightKind, ListInfo, Scene, Sun, Tonemap, Transform, UploadInfo};
pub use texture::{CLASSES, CLASS_MAX, CLASS_MIN, Mip, Sampling, TextureData, TextureId, TextureInfo, TextureRef, Wrap, class_of, mip_chain};

/// what a depth attachment of this engine clears to (reverse Z: 0 is infinitely far)
pub const DEPTH_CLEAR: f32 = 0.0;
/// the most frames the CPU may record ahead of the GPU
pub const FRAMES_MAX: usize = 3;

/// a view's uniform block: cull.wgsl's View (draw.wgsl declares its first 176 bytes)
pub(crate) const VIEW_BYTES: u64 = 208;
/// (G5d) the most detail levels of a mesh
pub const LODS_MAX: usize = 8;
/// draw.wgsl's Lights: sun, ambient, exposure, tone mapper, shadows, 4 cascades of 64 bytes, the environment
/// and its 9 harmonics
pub(crate) const LIGHTS_BYTES: u64 = 480;
/// where the lights lie in a frame's staging buffer (behind the view)
pub(crate) const LIGHTS_AT: u64 = 256;
/// (G5c) where the cascades' views lie in it, and how far apart
pub(crate) const CASCADE_AT: u64 = 736;
pub(crate) const CASCADE_STRIDE: u64 = 256;
/// timestamps per frame: start, uploads, the camera's culling (phase 1), 4 cascades' culling, 4 depth passes, the
/// main pass (phase 1), (G5d) the pyramid, phase 2's culling, phase 2's pass
pub(crate) const STAMPS: u32 = 15;
pub(crate) const INSTANCE_WORDS: usize = 16;
pub(crate) const CHANGE_WORDS: usize = 24;
const MATERIAL_WORDS: usize = 20;
const MATERIAL_BYTES: u64 = MATERIAL_WORDS as u64 * 4;
pub(crate) const DRAW_BYTES: u64 = 20;
pub(crate) const WG_CULL: u32 = 256;
pub(crate) const WG_SCATTER: u32 = 64;

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
/// (G5d) the depth pyramid of the occlusion test
pub const HIZ_WGSL: &str = include_str!("hiz.wgsl");
#[cfg(not(target_arch = "wasm32"))]
pub const CULL_SPV: &[u32] = spv::CULL;
#[cfg(not(target_arch = "wasm32"))]
pub const CULL_EARLY_SPV: &[u32] = spv::CULL_EARLY;
#[cfg(not(target_arch = "wasm32"))]
pub const CULL_LATE_SPV: &[u32] = spv::CULL_LATE;
#[cfg(not(target_arch = "wasm32"))]
pub const HIZ_DEPTH_SPV: &[u32] = spv::HIZ_DEPTH;
#[cfg(not(target_arch = "wasm32"))]
pub const HIZ_DOWN_SPV: &[u32] = spv::HIZ_DOWN;
#[cfg(not(target_arch = "wasm32"))]
pub const SCATTER_SPV: &[u32] = spv::SCATTER;
#[cfg(not(target_arch = "wasm32"))]
pub const VS_SPV: &[u32] = spv::VS;
#[cfg(not(target_arch = "wasm32"))]
pub const FS_SPV: &[u32] = spv::FS;
#[cfg(not(target_arch = "wasm32"))]
pub const VS_DEPTH_SPV: &[u32] = spv::VS_DEPTH;
#[cfg(not(target_arch = "wasm32"))]
pub const VS_SHADOW_SPV: &[u32] = spv::VS_SHADOW;
#[cfg(not(target_arch = "wasm32"))]
pub const FS_SHADOW_SPV: &[u32] = spv::FS_SHADOW;

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

/// (G5d) a detail level of a mesh (Renderer::set_lods): an instance whose bounding sphere's projected diameter is less
/// than `below` of the view's height is drawn with `mesh` instead; None: not drawn at all (contribution culling)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lod {
    pub mesh: Option<MeshId>,
    pub below: f32,
}

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

/// a row of the material table: glTF's metallic-roughness material, shaded as glTF 2.0 Appendix B says (G5c;
/// draw.wgsl).  Fragments whose alpha is below alpha_cutoff are dropped.  What reaches the target is opaque
/// (alpha 1)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// linear rgb and alpha (the texture's sRGB is decoded first)
    pub base_color: [f32; 4],
    /// 0 ..= 1 (clamped in the shader, after the texture)
    pub metallic: f32,
    pub roughness: f32,
    /// linear radiance (before the exposure)
    pub emissive: [f32; 3],
    pub base_color_texture: Option<TextureRef>,
    /// linear: roughness in g, metallic in b (times the factors)
    pub metallic_roughness_texture: Option<TextureRef>,
    /// linear, glTF's tangent space (+y up in the image); the tangent frame comes from the derivatives
    /// (docs/RENDER3D.md: how that differs from MikkTSpace); scale multiplies x and y
    pub normal_texture: Option<TextureRef>,
    pub normal_scale: f32,
    /// linear, r: how much of the ambient (indirect) light reaches the surface, 1 + strength (r - 1)
    pub occlusion_texture: Option<TextureRef>,
    pub occlusion_strength: f32,
    /// sRGB, times emissive
    pub emissive_texture: Option<TextureRef>,
    /// glTF's alpha mode MASK: fragments whose alpha is below it are not drawn (None: opaque)
    pub alpha_cutoff: Option<f32>,
}

impl Material {
    /// a dull material of one colour
    pub const fn color(r: f32, g: f32, b: f32) -> Material {
        Material { base_color: [r, g, b, 1.0], metallic: 0.0, roughness: 1.0, emissive: [0.0; 3], base_color_texture: None,
                   metallic_roughness_texture: None, normal_texture: None, normal_scale: 1.0, occlusion_texture: None,
                   occlusion_strength: 1.0, emissive_texture: None, alpha_cutoff: None }
    }

    /// a dull material of a texture (white times the texture)
    pub const fn textured(t: TextureRef) -> Material {
        let mut m = Material::color(1.0, 1.0, 1.0);
        m.base_color_texture = Some(t);
        m
    }

    /// the 80 bytes of draw.wgsl's Material
    fn words(&self, t: &texture::Textures) -> Result<[u32; MATERIAL_WORDS]> {
        let b = |v: f32| v.to_bits();
        let (c, e) = (self.base_color, self.emissive);
        let cutoff = match self.alpha_cutoff {
            Some(v) if v.is_finite() && v >= 0.0 => v,
            Some(_) => return Err(GpuError::Invalid("Material: alpha_cutoff must be finite and not negative")),
            None => -1.0,
        };
        Ok([b(c[0]), b(c[1]), b(c[2]), b(c[3]), b(e[0]), b(e[1]), b(e[2]), b(self.metallic), b(self.roughness), b(self.normal_scale),
            b(self.occlusion_strength), b(cutoff), t.word(&self.base_color_texture)?, t.word(&self.metallic_roughness_texture)?,
            t.word(&self.normal_texture)?, t.word(&self.occlusion_texture)?, t.word(&self.emissive_texture)?, 0, 0, 0])
    }
}

/// how many meshes and materials a Renderer has: its scenes judge ids by them at once, and know by this
/// object whose they are
pub(crate) struct Counts {
    pub(crate) meshes: AtomicU32,
    pub(crate) materials: AtomicU32,
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
            e.barrier(Access::COPY_WRITE, Access::COPY_READ | Access::COPY_WRITE | Access::INDEX_VERTEX | Access::GRAPHICS_READ);
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
    /// (G5d) the occlusion test's two phases (cull.wgsl) and the pyramid between them (hiz.rs)
    cull_early: ComputePipeline,
    cull_late: ComputePipeline,
    hiz_maker: hiz::Maker,
    /// a pyramid per depth attachment seen lately, and the records counted (when each was last used)
    hiz: core::cell::OnceCell<alloc::boxed::Box<hiz::Node>>,
    hiz_clock: Cell<u64>,
    scatter: ComputePipeline,
    /// front faces counter-clockwise; clockwise for the lists of mirrored instances
    draw: RenderPipeline,
    draw_mirrored: RenderPipeline,
    /// (G5c) the shadow passes: depth only (MASK casts full shadows), or with fs_shadow (its holes)
    shadow_depth: RenderPipeline,
    shadow_mask: RenderPipeline,
    mask_in_shadow: bool,
    pub(crate) shadow: shadow::ShadowMap,
    /// (G5c) the environment as the GPU holds it, None before set_environment
    pub(crate) env: Option<Environment>,
    pub(crate) textures: texture::Textures,
    pub(crate) cull_layout: BindGroupLayout,
    pub(crate) scatter_layout: BindGroupLayout,
    pub(crate) draw_layout: BindGroupLayout,
    pub(crate) vertices: Pool,
    /// 16-bit indices as long as a mesh's vertices allow it (they are relative to its base vertex): on the
    /// RTX 2070 SUPER a draw of 32-bit indices takes a third to a half longer (docs/RENDER3D.md)
    indices16: Pool,
    indices32: Pool,
    pub(crate) meshes: Vec<MeshInfo>,
    /// (G5d) per mesh: its detail levels, the sphere culling takes (its own, grown to hold its levels), and a count of
    /// set_lods (scenes send their instances again)
    pub(crate) lods: Vec<Vec<Lod>>,
    pub(crate) bounds: Vec<([f64; 3], f64)>,
    pub(crate) lod_gen: u32,
    materials: Vec<[u32; MATERIAL_WORDS]>,
    pub(crate) counts: Arc<Counts>,
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

struct Modules {
    cull: ShaderModule,
    cull_early: ShaderModule,
    cull_late: ShaderModule,
    hiz_depth: ShaderModule,
    hiz_down: ShaderModule,
    scatter: ShaderModule,
    vs: ShaderModule,
    fs: ShaderModule,
    vs_depth: ShaderModule,
    vs_shadow: ShaderModule,
    fs_shadow: ShaderModule,
}

#[cfg(not(target_arch = "wasm32"))]
fn modules(dev: &Device) -> Result<Modules> {
    let m = |w: &[u32]| ShaderModule::from_spirv(dev, w);
    Ok(Modules { cull: m(CULL_SPV)?, cull_early: m(CULL_EARLY_SPV)?, cull_late: m(CULL_LATE_SPV)?, hiz_depth: m(HIZ_DEPTH_SPV)?, hiz_down: m(HIZ_DOWN_SPV)?,
                 scatter: m(SCATTER_SPV)?, vs: m(VS_SPV)?, fs: m(FS_SPV)?, vs_depth: m(VS_DEPTH_SPV)?, vs_shadow: m(VS_SHADOW_SPV)?, fs_shadow: m(FS_SHADOW_SPV)? })
}

#[cfg(target_arch = "wasm32")]
fn modules(dev: &Device) -> Result<Modules> {
    let d = ShaderModule::from_wgsl(dev, DRAW_WGSL)?;
    let c = ShaderModule::from_wgsl(dev, CULL_WGSL)?;
    let h = ShaderModule::from_wgsl(dev, HIZ_WGSL)?;
    Ok(Modules { cull: c.clone(), cull_early: c.clone(), cull_late: c, hiz_depth: h.clone(), hiz_down: h, scatter: ShaderModule::from_wgsl(dev, SCATTER_WGSL)?, vs: d.clone(),
                 fs: d.clone(), vs_depth: d.clone(), vs_shadow: d.clone(), fs_shadow: d })
}

impl Renderer {
    pub fn new(dev: &Device, desc: &RendererDesc) -> Result<Renderer> {
        if !(1..=FRAMES_MAX).contains(&desc.frames_in_flight) {
            return Err(GpuError::Invalid("frames_in_flight: 1 ..= 3"));
        }
        let md = modules(dev)?;
        let b = |binding, kind, stages| Binding { binding, kind, stages };
        let (cs, vf) = (Stages::COMPUTE, Stages::VERTEX);
        let cull_layout = dev.create_bind_group_layout(&[
            b(0, BindingKind::UniformBuffer, cs),
            b(1, BindingKind::StorageBufferRead, cs),
            b(2, BindingKind::StorageBufferRead, cs),
            b(3, BindingKind::StorageBufferRead, cs),
            b(4, BindingKind::StorageBuffer, cs),
            b(5, BindingKind::StorageBuffer, cs),
            // (G5d) the slots phase 2 found hidden
            b(6, BindingKind::StorageBuffer, cs),
        ])?;
        let scatter_layout = dev.create_bind_group_layout(&[
            b(0, BindingKind::StorageBufferRead, cs),
            b(1, BindingKind::StorageBuffer, cs),
            b(2, BindingKind::StorageBuffer, cs),
            b(3, BindingKind::StorageBuffer, cs),
            b(4, BindingKind::StorageBuffer, cs),
        ])?;
        let draw_layout = dev.create_bind_group_layout(&[
            b(0, BindingKind::UniformBuffer, vf | Stages::FRAGMENT),
            b(1, BindingKind::StorageBufferRead, vf),
            b(2, BindingKind::UniformBuffer, Stages::FRAGMENT),
            b(3, BindingKind::StorageBufferRead, Stages::FRAGMENT),
            b(4, BindingKind::StorageBufferRead, vf),
            b(5, BindingKind::StorageBufferRead, vf),
            // (G5c) the point and spot lights
            b(6, BindingKind::StorageBufferRead, Stages::FRAGMENT),
        ])?;
        let hiz_maker = hiz::Maker::new(dev, &md.hiz_depth, &md.hiz_down)?;
        let cull_pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&cull_layout], push_constants: 20 })?;
        let late_pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&cull_layout, &hiz_maker.read], push_constants: 20 })?;
        let scatter_pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&scatter_layout], push_constants: 4 })?;
        let textures = texture::Textures::new(dev)?;
        let shadow = shadow::ShadowMap::new(dev)?;
        // group 2 (the shadow map) is the main pass's alone: the shadow passes write it
        let draw_pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&draw_layout, &textures.layout, &shadow.layout], push_constants: 4 })?;
        let shadow_pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&draw_layout, &textures.layout], push_constants: 4 })?;
        let cull = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &cull_pl, shader: ShaderStage::new(&md.cull, "main") })?;
        let cull_early = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &cull_pl, shader: ShaderStage::new(&md.cull_early, "early") })?;
        let cull_late = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &late_pl, shader: ShaderStage::new(&md.cull_late, "late") })?;
        let scatter = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &scatter_pl, shader: ShaderStage::new(&md.scatter, "main") })?;
        let pipeline = |front_cw| dev.create_render_pipeline(&RenderPipelineDesc {
            layout: &draw_pl,
            vertex: ShaderStage::new(&md.vs, "vs"),
            fragment: Some(ShaderStage::new(&md.fs, "fs")),
            buffers: &[],
            topology: Topology::Triangles,
            cull: Cull::Back,
            front_cw,
            targets: &[ColorTarget { format: desc.color, blend: None, write_mask: 0xF }],
            depth: Some(DepthState { format: Format::Depth32Float, write: true, compare: Compare::Greater }),
        });
        let (draw, draw_mirrored) = (pipeline(false)?, pipeline(true)?);
        // both sides of every triangle cast (glTF's single-sided plates too)
        let depth_only = |vertex: ShaderStage, fragment: Option<ShaderStage>| dev.create_render_pipeline(&RenderPipelineDesc {
            layout: &shadow_pl,
            vertex,
            fragment,
            buffers: &[],
            topology: Topology::Triangles,
            cull: Cull::None,
            front_cw: false,
            targets: &[],
            depth: Some(DepthState { format: Format::Depth32Float, write: true, compare: Compare::Greater }),
        });
        let shadow_depth = depth_only(ShaderStage::new(&md.vs_depth, "vs_depth"), None)?;
        let shadow_mask = depth_only(ShaderStage::new(&md.vs_shadow, "vs_shadow"), Some(ShaderStage::new(&md.fs_shadow, "fs_shadow")))?;
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
            lods: Vec::new(),
            bounds: Vec::new(),
            lod_gen: 0,
            materials: Vec::new(),
            counts: Arc::new(Counts { meshes: AtomicU32::new(0), materials: AtomicU32::new(0) }),
            material_buf: dev.create_buffer(&BufferDesc { size: MATERIAL_BYTES * 64, usage: BufferUsage::STORAGE, memory: MemoryKind::Device })?,
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
            cull_early,
            cull_late,
            hiz_maker,
            hiz: core::cell::OnceCell::new(),
            hiz_clock: Cell::new(0),
            scatter,
            draw,
            draw_mirrored,
            shadow_depth,
            shadow_mask,
            mask_in_shadow: true,
            shadow,
            env: None,
            textures,
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

    /// (G5c) false: the shadow passes run without a fragment shader and MASK materials cast full shadows
    /// (what that saves: docs/RENDER3D.md); true at first: their holes let the sun through
    pub fn set_shadow_mask(&mut self, on: bool) {
        self.mask_in_shadow = on;
    }

    /// (G5c) the environment's light: a procedural sky or six faces, prefiltered on the CPU (the levels of a
    /// cube texture per roughness, 9 harmonics for the diffuse part); scenes take it with
    /// Scene::set_environment.  The upload goes with the next submission
    pub fn set_environment(&mut self, d: &EnvironmentData) -> Result<()> {
        let (size, levels, sh) = env::build(d)?;
        let (tex, held) = env::upload(&self.dev, size, &levels)?;
        self.shadow.set_env(&self.dev, tex)?;
        let sh = sh.map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]);
        self.env = Some(Environment { size, levels: held, sh });
        Ok(())
    }

    /// (G5c; tests, tools) the environment as the GPU holds it
    pub fn environment(&self) -> Option<&Environment> {
        self.env.as_ref()
    }

    /// (G5c; tests) the sun's shadow map: CASCADES_MAX layers of Depth32Float, in Layout::Sampled between frames
    pub fn shadow_map(&self) -> &Texture {
        &self.shadow.tex
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
        self.lods.push(Vec::new());
        self.bounds.push((center, math::sqrt(r2)));
        self.counts.meshes.store(self.meshes.len() as u32, Ordering::Relaxed);
        Ok(MeshId(self.meshes.len() as u32 - 1))
    }

    pub fn mesh_info(&self, id: MeshId) -> Option<MeshInfo> {
        self.meshes.get(id.0 as usize).copied()
    }

    /// (G5d) a mesh's detail levels: other meshes (or none) for its instances below a projected size, thresholds
    /// falling, at most LODS_MAX; empty: none.  The visibility picks per instance from the camera's distance (the shadow
    /// cascades take the camera's pick, so a shadow matches its caster); the sphere culling takes for the mesh's instances
    /// grows to hold every level (cull_sphere).  Scenes send all their instances again with their next frame.  Invalid
    /// for meshes the renderer does not have and thresholds not finite, not above 0 or not falling
    pub fn set_lods(&mut self, mesh: MeshId, lods: &[Lod]) -> Result<()> {
        let m = mesh.0 as usize;
        let n = self.meshes.len();
        if m >= n || lods.iter().any(|l| l.mesh.is_some_and(|x| x.0 as usize >= n)) {
            return Err(GpuError::Invalid("set_lods: a mesh the renderer does not have"));
        }
        let falling = lods.windows(2).all(|w| w[1].below < w[0].below);
        if lods.len() > LODS_MAX || !falling || lods.iter().any(|l| !(l.below.is_finite() && l.below > 0.0)) {
            return Err(GpuError::Invalid("set_lods: at most LODS_MAX levels, thresholds finite, above 0 and falling"));
        }
        let base = self.meshes[m];
        let mut radius = base.radius;
        for o in lods.iter().filter_map(|l| l.mesh) {
            let o = self.meshes[o.0 as usize];
            radius = radius.max(math::len(math::sub(o.center, base.center)) + o.radius);
        }
        self.bounds[m] = (base.center, radius);
        self.lods[m] = lods.to_vec();
        self.lod_gen += 1;
        Ok(())
    }

    /// (G5d) a mesh's detail levels (empty: none)
    pub fn lods(&self, mesh: MeshId) -> &[Lod] {
        self.lods.get(mesh.0 as usize).map_or(&[], |v| &v[..])
    }

    /// (G5d) the sphere (centre, radius in the mesh's coordinates) the visibility takes for a mesh's instances: its own,
    /// grown to hold its detail levels
    pub fn cull_sphere(&self, mesh: MeshId) -> Option<([f64; 3], f64)> {
        self.bounds.get(mesh.0 as usize).copied()
    }

    /// a texture into the array of its size class with all its mip levels (texture.rs: resampled when it is
    /// not 128, 256, 512, 1024 or 2048 square).  The upload goes with the next submission
    pub fn add_texture(&mut self, d: &TextureData) -> Result<TextureId> {
        let dev = self.dev.clone();
        self.textures.add(&dev, d)
    }

    pub fn texture_info(&self, id: TextureId) -> Option<TextureInfo> {
        self.textures.info.get(id.0 as usize).copied()
    }

    pub fn texture_count(&self) -> u32 {
        self.textures.info.len() as u32
    }

    /// bytes of GPU memory the texture arrays take
    pub fn texture_bytes(&self) -> u64 {
        self.textures.bytes()
    }

    /// (tests) the array of a size class and colour space (layers: TextureInfo::layer), None without a texture
    pub fn texture_array(&self, class: u32, srgb: bool) -> Option<&Texture> {
        self.textures.array(class, srgb)
    }

    /// a row of the table; its textures must be this renderer's
    pub fn add_material(&mut self, m: &Material) -> Result<MaterialId> {
        let i = self.materials.len() as u32;
        let words = m.words(&self.textures)?;
        self.materials.push(words);
        self.mark_material(i);
        self.counts.materials.store(i + 1, Ordering::Relaxed);
        Ok(MaterialId(i))
    }

    /// a row of the table; the next frame shows it
    pub fn set_material(&mut self, id: MaterialId, m: &Material) -> Result<()> {
        let words = m.words(&self.textures)?;
        let Some(row) = self.materials.get_mut(id.0 as usize) else { return Err(GpuError::Invalid("set_material: no such material")) };
        *row = words;
        self.mark_material(id.0);
        Ok(())
    }

    fn mark_material(&mut self, i: u32) {
        self.material_dirty = Some(self.material_dirty.map_or((i, i + 1), |(a, b)| (a.min(i), b.max(i + 1))));
    }

    /// (Scene::prepare) what the last prepare sent may not have reached a frame: the whole table again
    pub(crate) fn resend_materials(&mut self) {
        if !self.materials.is_empty() {
            self.material_dirty = Some((0, self.materials.len() as u32));
        }
    }

    /// a depth buffer the renderer keeps, for frames recorded with depth None (sampled too: the occlusion test builds
    /// its pyramid from it)
    pub fn set_size(&mut self, w: u32, h: u32) -> Result<()> {
        if self.depth.as_ref().is_some_and(|d| (d.width(), d.height()) == (w, h)) {
            return Ok(());
        }
        self.depth = Some(self.dev.create_texture(&TextureDesc::d2(w, h, Format::Depth32Float, TextureUsage::DEPTH_TARGET | TextureUsage::SAMPLED))?);
        self.depth_used.set(false);
        Ok(())
    }

    /// (G5d; tests) the renderer's own depth buffer (set_size)
    pub fn depth_texture(&self) -> Option<&Texture> {
        self.depth.as_ref()
    }

    /// (G5d; tests) the occlusion test's pyramid of a depth attachment as the last frame with it built it: mip k - 1
    /// holds level k (the smallest depth of 2^k x 2^k pixels), in Layout::Sampled; None when no record used it lately
    pub fn pyramid(&self, depth: &TextureView) -> Option<&Texture> {
        hiz::of(&self.hiz, depth).map(|p| p.texture())
    }

    /// (Scene::prepare) the table's changes for the next frame's commands
    pub(crate) fn prepare_materials(&mut self) -> Result<u64> {
        // (G5d) the pyramids of depth attachments no record used lately go
        hiz::prune(&mut self.hiz, self.hiz_clock.get().saturating_sub(hiz::KEEP));
        // the last ones never reached a frame: all of it again
        if !self.material_up_done.get() {
            self.material_dirty = Some((0, self.materials.len() as u32));
        }
        let n = self.materials.len() as u32;
        if n > self.material_cap {
            self.material_cap = n.max(self.material_cap * 2);
            self.material_buf = self.dev.create_buffer(&BufferDesc { size: MATERIAL_BYTES * self.material_cap as u64, usage: BufferUsage::STORAGE, memory: MemoryKind::Device })?;
            self.bindings_gen += 1;
            self.material_dirty = Some((0, n));
        }
        let Some((a, b)) = self.material_dirty.take() else { return Ok(0) };
        let size = (b - a) as u64 * MATERIAL_BYTES;
        let up = self.dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Upload })?;
        up.write(0, &self.materials[a as usize..b as usize])?;
        self.material_up = Some((up, a as u64 * MATERIAL_BYTES, size));
        self.material_up_done.set(false);
        Ok(size)
    }

    /// one frame of a prepared scene into e: its uploads, its culling, its render pass with a clear of
    /// colour and depth as the attachments say.  depth None: the renderer's own (set_size), cleared.
    /// (G5d) With the occlusion test (Scene::set_occlusion, and a depth attachment with TextureUsage::SAMPLED) the
    /// pass is two: phase 1 with the attachments' load and their `before`, phase 2 that keeps what phase 1 drew and
    /// ends in their `store` and `after`
    pub fn record<'a>(&'a self, e: &mut CommandEncoder<'a>, s: &'a Scene, color: ColorAttachment<'a>, depth: Option<DepthAttachment<'a>>) -> Result<()> {
        if !Arc::ptr_eq(&s.counts, &self.counts) {
            return Err(GpuError::Invalid("record: a scene of another Renderer"));
        }
        if !s.prepared {
            return Err(GpuError::Invalid("record: Scene::prepare comes first"));
        }
        // (what went up does not hold them: the visibility shader would read and write beyond its buffers)
        if s.stale {
            return Err(GpuError::Invalid("record: instances came or changed their mesh after Scene::prepare"));
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
        let ts = match &s.ts {
            Some(ts) if s.stats_on => Some(ts),
            _ => None,
        };
        let base = s.slot as u32 * STAMPS;
        if let Some(ts) = ts {
            e.reset_timestamps(ts, base, STAMPS);
            e.timestamp(ts, base);
        }
        let cascades = s.cascades().len();
        let n = s.slots.len() as u32;
        let lists = s.lists.len() as u64;
        // (G5d) the occlusion test needs a depth buffer it can read; its pyramid is the attachment's
        let now = self.hiz_clock.get() + 1;
        self.hiz_clock.set(now);
        let pyramid = if s.occlusion && s.culling && n > 0 && lists > 0 && depth.view.usage().contains(TextureUsage::SAMPLED) {
            hiz::find(&self.hiz, &self.dev, &self.hiz_maker, depth.view, now)?
        } else {
            None
        };
        // the frame before read and wrote everything this one writes (the shadow map: its passes' own layout
        // transitions, Sampled -> DepthTarget -> Sampled)
        e.barrier(Access::COPY_READ | Access::COPY_WRITE | Access::COMPUTE_READ | Access::COMPUTE_WRITE | Access::INDIRECT | Access::INDEX_VERTEX | Access::GRAPHICS_READ,
                  Access::COPY_READ | Access::COPY_WRITE | Access::COMPUTE_READ | Access::COMPUTE_WRITE);
        // (G5d) every slot counts as seen (a new buffer, every slot sent anew); before scatter clears the bits of changed
        // slots, after the frame before's phase 2 wrote them (the barrier above)
        if s.reset_hidden {
            e.fill_buffer(&s.hidden, 0, s.hidden.size(), 0);
            if s.changed > 0 {
                e.barrier(Access::COPY_WRITE, Access::COMPUTE_READ | Access::COMPUTE_WRITE);
            }
        }
        e.copy_buffer(&f.stage, 0, &s.view.ubo, 0, VIEW_BYTES);
        e.copy_buffer(&f.stage, LIGHTS_AT, &s.lights, 0, LIGHTS_BYTES);
        for k in 0..cascades {
            e.copy_buffer(&f.stage, CASCADE_AT + k as u64 * CASCADE_STRIDE, &s.cascades[k].ubo, 0, VIEW_BYTES);
        }
        // (G5c) the point and spot lights, when they changed (the barrier below makes them visible to the
        // fragment shader; the one above waited for the frame before's reads)
        if let (n @ 1.., Some(up)) = (s.punctual_up, &f.punctual) {
            e.copy_buffer(up, 0, &s.punctual_buf, 0, n as u64 * scene::PUNCTUAL_BYTES);
        }

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
            let info = s.info_len as u64 * 16;
            e.copy_buffer(up, 0, &s.list_first, 0, info);
            e.copy_buffer(up, info, &s.template, 0, lists * DRAW_BYTES);
            e.barrier(Access::COPY_WRITE, Access::COPY_READ);
        }
        if lists > 0 {
            e.copy_buffer(&s.template, 0, &s.view.draws, 0, lists * DRAW_BYTES);
            e.copy_buffer(&s.template, 0, &s.late.draws, 0, lists * DRAW_BYTES);
            for v in &s.cascades[..cascades] {
                e.copy_buffer(&s.template, 0, &v.draws, 0, lists * DRAW_BYTES);
            }
        }
        // the copies (views, lights, draw records from the template, changes) and scatter's writes before culling
        // reads them and before the vertex and fragment shaders read views, lights, instances and materials (and
        // before the stats copy phase 2's untouched records when it does not run)
        e.barrier(Access::COPY_WRITE | Access::COMPUTE_WRITE, Access::COMPUTE_READ | Access::COMPUTE_WRITE | Access::GRAPHICS_READ | Access::COPY_READ);
        if let Some(ts) = ts {
            e.timestamp(ts, base + 1);
        }
        // (G5d) with the occlusion test phase 1: what phase 2 of the frame before saw
        let all = !s.culling as u32;
        self.cull(e, s, &s.view, if pyramid.is_some() { &self.cull_early } else { &self.cull }, &[n, all], None);
        if let Some(ts) = ts {
            e.timestamp(ts, base + 2);
        }
        // (G5c) each cascade's own visibility: its planes, its counts and lists, nothing shared written
        for k in 0..CASCADES_MAX {
            if k < cascades {
                self.cull(e, s, &s.cascades[k], &self.cull, &[n, all], None);
            }
            if let Some(ts) = ts {
                e.timestamp(ts, base + 3 + k as u32);
            }
        }
        // every view's counts and lists before the indirect draws and the vertex shaders read them
        e.barrier(Access::COMPUTE_WRITE, Access::INDIRECT | Access::GRAPHICS_READ | Access::COPY_READ);
        if let (true, Some(rb), true) = (s.stats_on, &f.readback, lists > 0) {
            e.copy_buffer(&s.view.draws, 0, rb, 0, lists * DRAW_BYTES);
            if pyramid.is_none() {
                e.copy_buffer(&s.late.draws, 0, rb, rb.size() / 2, lists * DRAW_BYTES);
            }
        }
        // (G5c) a depth pass per cascade into its layer; the pass's barriers: Sampled -> DepthTarget waits for
        // the shaders that read the map before (the last frame's main pass), DepthTarget -> Sampled makes the
        // depth writes visible to the shaders after (this frame's main pass)
        for k in 0..CASCADES_MAX {
            if k < cascades {
                let layer = DepthAttachment { view: &self.shadow.layers[k], load: Load::Clear, clear: DEPTH_CLEAR, store: true, before: Layout::Sampled, after: Layout::Sampled };
                e.begin_render(&RenderPassDesc { color: &[], depth: Some(layer) });
                self.draw_shadow(e, s, &s.cascades[k]);
                e.end_render();
            }
            if let Some(ts) = ts {
                e.timestamp(ts, base + 7 + k as u32);
            }
        }
        // (G5d) phase 1 keeps colour and depth for phase 2 and leaves the depth buffer to the pyramid's compute shader:
        // its pass ends with DepthTarget -> Sampled (late fragment tests and depth writes before the shaders' reads),
        // phase 2's begins with Sampled -> DepthTarget (the pyramid's reads before the depth test writes again)
        let (c1, d1) = match pyramid {
            Some(_) => (ColorAttachment { store: true, after: Layout::ColorTarget, ..color }, DepthAttachment { store: true, after: Layout::Sampled, ..depth }),
            None => (color, depth),
        };
        e.begin_render(&RenderPassDesc { color: &[c1], depth: Some(d1) });
        self.draw_lists(e, s, &s.view);
        e.end_render();
        if let Some(ts) = ts {
            e.timestamp(ts, base + 11);
        }
        if let Some(p) = pyramid {
            p.record(e, &self.hiz_maker);
            if let Some(ts) = ts {
                e.timestamp(ts, base + 12);
            }
            // phase 2: every instance in the view against the pyramid (its mips Layout::Sampled since the pyramid's
            // barriers); it reads and writes the bits phase 1 read (the barriers since have waited for phase 1)
            self.cull(e, s, &s.late, &self.cull_late, &[n, 0, p.w, p.h, p.levels], Some(&p.read));
            if let Some(ts) = ts {
                e.timestamp(ts, base + 13);
            }
            // phase 2's counts and lists before its indirect draws and vertex shaders read them
            e.barrier(Access::COMPUTE_WRITE, Access::INDIRECT | Access::GRAPHICS_READ | Access::COPY_READ);
            if let (true, Some(rb)) = (s.stats_on, &f.readback) {
                e.copy_buffer(&s.late.draws, 0, rb, rb.size() / 2, lists * DRAW_BYTES);
            }
            let c2 = ColorAttachment { load: Load::Keep, before: Layout::ColorTarget, ..color };
            let d2 = DepthAttachment { load: Load::Keep, before: Layout::Sampled, ..depth };
            e.begin_render(&RenderPassDesc { color: &[c2], depth: Some(d2) });
            self.draw_lists(e, s, &s.late);
            e.end_render();
        } else if let Some(ts) = ts {
            e.timestamp(ts, base + 12);
            e.timestamp(ts, base + 13);
        }
        if let Some(ts) = ts {
            e.timestamp(ts, base + 14);
        }
        if s.stats_on {
            e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        }
        Ok(())
    }

    /// (G5c) one cascade's lists into its depth pass: one pipeline for both sides (no culling of faces)
    fn draw_shadow<'a>(&'a self, e: &mut CommandEncoder<'a>, s: &'a Scene, v: &'a scene::ViewLists) {
        if !s.lists.iter().any(|l| l.room > 0) {
            return;
        }
        e.set_render_pipeline(if self.mask_in_shadow { &self.shadow_mask } else { &self.shadow_depth });
        e.set_bind_group(0, &v.render);
        e.set_bind_group(1, &self.textures.group);
        if self.multi {
            e.push_constants(0, &[0u32]);
            for &(first, count, wide, _) in &s.runs {
                self.bind_indices(e, wide);
                e.draw_indexed_indirect_multi(&v.draws, first as u64 * DRAW_BYTES, count, DRAW_BYTES as u32);
            }
        } else {
            let mut bound = None;
            for (i, l) in s.lists.iter().enumerate() {
                if l.room > 0 {
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

    /// one view's visibility (outside a pass): the camera's (G5d: phase 1 or 2 of the occlusion test), a shadow
    /// cascade's (G5c), each a ViewLists of its own; push: cull.wgsl's Push, hiz: group 1 of phase 2
    fn cull<'a>(&'a self, e: &mut CommandEncoder<'a>, s: &'a Scene, v: &'a scene::ViewLists, pipe: &'a ComputePipeline, push: &[u32], hiz: Option<&'a BindGroup>) {
        let n = s.slots.len() as u32;
        if n == 0 {
            return;
        }
        e.set_compute_pipeline(pipe);
        e.set_bind_group(0, &v.group);
        if let Some(g) = hiz {
            e.set_bind_group(1, g);
        }
        e.push_constants(0, push);
        e.dispatch(n.div_ceil(WG_CULL), 1, 1);
    }

    fn bind_indices<'a>(&'a self, e: &mut CommandEncoder<'a>, wide: bool) {
        if wide {
            e.set_index_buffer(&self.indices32.buf, 0, IndexFormat::U32);
        } else {
            e.set_index_buffer(&self.indices16.buf, 0, IndexFormat::U16);
        }
    }

    /// one view's lists into the running pass (a depth-only pipeline can take the same lists): those of
    /// instances as they are, then those of mirrored ones with the pipeline whose front faces are clockwise
    fn draw_lists<'a>(&'a self, e: &mut CommandEncoder<'a>, s: &'a Scene, v: &'a scene::ViewLists) {
        for mirrored in [false, true] {
            if !s.lists.iter().any(|l| l.mirrored == mirrored && l.room > 0) {
                continue;
            }
            e.set_render_pipeline(if mirrored { &self.draw_mirrored } else { &self.draw });
            e.set_bind_group(0, &v.render);
            e.set_bind_group(1, &self.textures.group);
            e.set_bind_group(2, &self.shadow.group);
            if self.multi {
                // lists side by side with the same width of indices and side are one multi-draw (usually all)
                e.push_constants(0, &[0u32]);
                for &(first, count, wide, m) in &s.runs {
                    if m == mirrored {
                        self.bind_indices(e, wide);
                        e.draw_indexed_indirect_multi(&v.draws, first as u64 * DRAW_BYTES, count, DRAW_BYTES as u32);
                    }
                }
            } else {
                // WebGPU and Vulkan 1.1: no first instance is sure to work; a list's start comes as a push constant
                let mut bound = None;
                for (i, l) in s.lists.iter().enumerate() {
                    if l.room > 0 && l.mirrored == mirrored {
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
}
