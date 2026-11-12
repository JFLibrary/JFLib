// render2d -- the core of the 2D renderer for CAD on lib/gpu (G6a, G6b1; docs/RENDER2D.md): a retained scene of
// entities with stable ids (lines, polylines, arcs and circles; Bézier curves, ellipses and splines flattened per zoom
// band on the CPU, curve.rs; fills of rings, triangulated on the CPU, fill.rs), layers and styles; strokes as quads whose
// coverage the fragment shader computes from the geometry (no MSAA), with exact butt, square and round caps and miter,
// bevel and round joins, dash patterns in pixels or drawing units; fills with their edge's exact coverage, every pixel
// blended once, and hatches from the camera's place in f64; f64 coordinates, a pair of f32 per axis on the GPU with the
// camera subtracted first; visibility in compute shaders that keep the painter's order and write the indirect draws.  Hosted
// like render3d as the content of a ui.gpu_view (the app glues it with a GpuContent; lib/ui knows nothing of it).  The
// same WGSL on every path: compiled ahead (shaders_spv.rs) on Vulkan, handed to the browser as text on WebGPU.
//
//   Renderer::new(&device, &RendererDesc) -> Result<Renderer>   the pipelines of one colour format
//   Scene::new(&renderer) -> Result<Scene>
//   scene.add_style(&Style) -> StyleId, set_style; add_layer(&Layer) -> LayerId, set_layer, remove_layer
//   scene.insert(layer, style, &Shape) -> EntityId, update(id, &Shape), set_entity_style(id, style), remove(id)
//   scene.set_camera(&Camera)                                   centre (f64), scale (units per pixel), rotation
//   a frame:
//        scene.prepare(&mut renderer, (w, h))?;         waits for this slot's last frame, uploads what changed
//        renderer.record(&mut encoder, &scene, color)?;
//        let id = queue.submit([encoder.finish()?])?;   (or swapchain.present)
//        scene.submitted(id);
//
// Draw order: layers by Layer::order (then by when they were added), within a layer the order of insertion (an update
// keeps an entity's place); colours are sRGB codes with straight alpha, blended in code space as lib/gfx blends
// (the target holds codes: a Unorm format such as ui's Bgra8Unorm, no sRGB format).  The camera's y axis points up
// on the screen.  Errors are gpu's: Invalid for ids and values this crate can judge, the rest from the backend.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;

use gpu::*;

pub mod curve;
pub mod fill;
pub mod math;
mod scene;

pub use scene::{Camera, Cap, EntityId, FrameStats, HatchLine, Join, Layer, LayerId, Scene, Shape, Style, StyleId, Unit, UploadInfo, ARC_STEP, DASH_MAX, HATCH_DASH_MAX, HATCH_MAX};

/// the most frames the CPU may record ahead of the GPU
pub const FRAMES_MAX: usize = 3;
/// records per page: the unit of the pool's regions and of a workgroup of culling (common.wgsl PAGE)
pub const PAGE: u32 = 256;
/// quads per instance of a draw (stroke.wgsl BATCH): the u16 indices of one instance reach 4 * BATCH corners
pub const BATCH: u32 = 16384;
/// the most visible layers (stroke.wgsl: first_instance = layer << 16 on Voll)
pub const LAYERS_MAX: u32 = 65535;

pub(crate) const RECORD_WORDS: usize = 16;
pub(crate) const RECORD_BYTES: u64 = 64;
pub(crate) const STYLE_WORDS: usize = 32;
pub(crate) const STYLE_BYTES: u64 = 128;
/// a row of the scatter shader: the place, 3 words of padding, the record
pub(crate) const CHANGE_WORDS: usize = 20;
pub(crate) const VIEW_BYTES: u64 = 64;
/// a hatched style's entry of the hatch table (fill.wgsl Hatch): 32 vec4f
pub(crate) const HATCH_WORDS: usize = 128;
pub(crate) const HATCH_BYTES: u64 = 512;
pub(crate) const DRAW_BYTES: u64 = 20;
/// timestamps per frame: start, after the uploads, after the visibility, after the draw
pub(crate) const STAMPS: u32 = 4;
pub(crate) const WG_SCATTER: u32 = 64;

/// the shaders, compiled from the *.wgsl beside this file with common.wgsl in front (render2dtest --test shaders
/// compares them with what wgsl::compile makes of them, --gen-shaders writes them again); the browser takes the WGSL
#[cfg(not(target_arch = "wasm32"))]
mod spv {
    include!("shaders_spv.rs");
}

/// the WGSL sources as the device gets them (common.wgsl in front of each; stroke.wgsl with fs_full from full_body.wgsl, the same body
/// as the function full_body, and fill.wgsl)
pub const COMMON_WGSL: &str = include_str!("common.wgsl");
pub const CULL_WGSL: &str = concat!(include_str!("common.wgsl"), include_str!("cull.wgsl"));
pub const SCATTER_WGSL: &str = concat!(include_str!("common.wgsl"), include_str!("scatter.wgsl"));
pub const STROKE_WGSL: &str = concat!(include_str!("common.wgsl"), include_str!("stroke.wgsl"), "\n@fragment\nfn fs_full(i: VsOut) -> @location(0) vec4f {\n", include_str!("full_body.wgsl"),
                                      "}\n\nfn full_body(i: VsOut) -> vec4f {\n", include_str!("full_body.wgsl"), "}\n", include_str!("fill.wgsl"));
#[cfg(not(target_arch = "wasm32"))]
pub const COUNT_SPV: &[u32] = spv::COUNT;
#[cfg(not(target_arch = "wasm32"))]
pub const SCAN_SPV: &[u32] = spv::SCAN;
#[cfg(not(target_arch = "wasm32"))]
pub const WRITE_SPV: &[u32] = spv::WRITE;
#[cfg(not(target_arch = "wasm32"))]
pub const SCATTER_SPV: &[u32] = spv::SCATTER;
#[cfg(not(target_arch = "wasm32"))]
pub const VS_SPV: &[u32] = spv::VS;
#[cfg(not(target_arch = "wasm32"))]
pub const VS_FULL_SPV: &[u32] = spv::VS_FULL;
#[cfg(not(target_arch = "wasm32"))]
pub const FS_SPV: &[u32] = spv::FS;
#[cfg(not(target_arch = "wasm32"))]
pub const FS_FULL_SPV: &[u32] = spv::FS_FULL;
#[cfg(not(target_arch = "wasm32"))]
pub const VS_FILL_SPV: &[u32] = spv::VS_FILL;
#[cfg(not(target_arch = "wasm32"))]
pub const FS_FILL_SPV: &[u32] = spv::FS_FILL;

/// Renderer::new.  Made with `new` and changed field by field (it may grow)
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct RendererDesc {
    /// the format of the colour targets: one without sRGB (the codes are blended as they are)
    pub color: Format,
    /// frames the CPU may record ahead of the GPU (1 ..= FRAMES_MAX)
    pub frames_in_flight: usize,
}

impl RendererDesc {
    pub const fn new(color: Format) -> RendererDesc {
        RendererDesc { color, frames_in_flight: 2 }
    }
}

/// who a scene belongs to
pub(crate) struct Tag;

/// the pipelines of one colour format on one device
pub struct Renderer {
    pub(crate) dev: Device,
    pub(crate) desc: RendererDesc,
    /// level Voll: one multi-draw for all visible layers; else one indirect draw per layer
    pub(crate) multi: bool,
    pub(crate) max_records: u32,
    pub(crate) max_entries: u32,
    pub(crate) tag: Arc<Tag>,
    count: ComputePipeline,
    scan: ComputePipeline,
    write: ComputePipeline,
    scatter: ComputePipeline,
    /// solid segments (stroke.wgsl fs), and everything (fs_full: arcs, dash patterns) for the layers that hold arcs or
    /// dashed strokes; fills and hatches too (fs_fill) for the layers that hold a fill
    stroke: RenderPipeline,
    stroke_full: RenderPipeline,
    /// fills, hatches and everything fs_full draws, for the layers that hold a fill
    stroke_fill: RenderPipeline,
    pub(crate) count_layout: BindGroupLayout,
    pub(crate) scan_layout: BindGroupLayout,
    pub(crate) write_layout: BindGroupLayout,
    pub(crate) scatter_layout: BindGroupLayout,
    pub(crate) draw_layout: BindGroupLayout,
    /// u16, 0 1 2 2 1 3 per quad, BATCH quads (written once, with the first submission)
    quads: Buffer,
}

struct Modules {
    count: ShaderModule,
    scan: ShaderModule,
    write: ShaderModule,
    scatter: ShaderModule,
    vs: ShaderModule,
    vs_full: ShaderModule,
    fs: ShaderModule,
    fs_full: ShaderModule,
    vs_fill: ShaderModule,
    fs_fill: ShaderModule,
}

#[cfg(not(target_arch = "wasm32"))]
fn modules(dev: &Device) -> Result<Modules> {
    let m = |w: &[u32]| ShaderModule::from_spirv(dev, w);
    Ok(Modules { count: m(COUNT_SPV)?, scan: m(SCAN_SPV)?, write: m(WRITE_SPV)?, scatter: m(SCATTER_SPV)?, vs: m(VS_SPV)?, vs_full: m(VS_FULL_SPV)?, fs: m(FS_SPV)?, fs_full: m(FS_FULL_SPV)?,
                 vs_fill: m(VS_FILL_SPV)?, fs_fill: m(FS_FILL_SPV)? })
}

#[cfg(target_arch = "wasm32")]
fn modules(dev: &Device) -> Result<Modules> {
    let c = ShaderModule::from_wgsl(dev, CULL_WGSL)?;
    let s = ShaderModule::from_wgsl(dev, STROKE_WGSL)?;
    Ok(Modules { count: c.clone(), scan: c.clone(), write: c, scatter: ShaderModule::from_wgsl(dev, SCATTER_WGSL)?, vs: s.clone(), vs_full: s.clone(), fs: s.clone(), fs_full: s.clone(),
                 vs_fill: s.clone(), fs_fill: s })
}

impl Renderer {
    pub fn new(dev: &Device, desc: &RendererDesc) -> Result<Renderer> {
        if !(1..=FRAMES_MAX).contains(&desc.frames_in_flight) {
            return Err(GpuError::Invalid("frames_in_flight: 1 ..= 3"));
        }
        if desc.color.is_srgb() || desc.color.is_depth() || desc.color.is_compressed() {
            return Err(GpuError::Invalid("RendererDesc: a colour format without sRGB (render2d blends sRGB codes as they are)"));
        }
        let md = modules(dev)?;
        let b = |binding, kind, stages| Binding { binding, kind, stages };
        let (cs, vs) = (Stages::COMPUTE, Stages::VERTEX);
        // (the fragment shader reads the view and the styles: the dash patterns)
        let vfs = Stages::VERTEX | Stages::FRAGMENT;
        let (read, rw) = (BindingKind::StorageBufferRead, BindingKind::StorageBuffer);
        // (cull.wgsl's bindings: each pass names those it uses, none more than 5 storage buffers)
        // (11: the fill edges' segments in pixels, written by count)
        let count_layout = dev.create_bind_group_layout(&[b(0, BindingKind::UniformBuffer, cs), b(1, read, cs), b(2, read, cs), b(3, read, cs), b(4, rw, cs), b(5, rw, cs), b(11, rw, cs)])?;
        let scan_layout = dev.create_bind_group_layout(&[b(5, rw, cs), b(6, read, cs), b(7, rw, cs), b(8, rw, cs), b(9, rw, cs)])?;
        let write_layout = dev.create_bind_group_layout(&[b(3, read, cs), b(4, rw, cs), b(7, rw, cs), b(10, rw, cs)])?;
        let scatter_layout = dev.create_bind_group_layout(&[b(0, read, cs), b(1, rw, cs)])?;
        // (5: the hatch table, 6: the fill edges' segments in pixels; fs_fill reads the pool too)
        let draw_layout = dev.create_bind_group_layout(&[b(0, BindingKind::UniformBuffer, vfs), b(1, read, vfs), b(2, read, vfs), b(3, read, vs), b(4, read, vs), b(5, read, Stages::FRAGMENT),
                                                         b(6, read, vfs)])?;
        let pl = |g: &BindGroupLayout, push: u32| dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[g], push_constants: push });
        // (only scan, scatter and the draw read a push constant; a layout that says more than its pass sets is an error in the browser, where the
        // values are a bind group)
        let (count_pl, scan_pl, write_pl, scatter_pl, draw_pl) = (pl(&count_layout, 0)?, pl(&scan_layout, 16)?, pl(&write_layout, 0)?, pl(&scatter_layout, 4)?, pl(&draw_layout, 4)?);
        let compute = |layout: &PipelineLayout, m: &ShaderModule, entry: &str| dev.create_compute_pipeline(&ComputePipelineDesc { layout, shader: ShaderStage::new(m, entry) });
        let count = compute(&count_pl, &md.count, "count")?;
        let scan = compute(&scan_pl, &md.scan, "scan")?;
        let write = compute(&write_pl, &md.write, "write")?;
        let scatter = compute(&scatter_pl, &md.scatter, "main")?;
        // premultiplied blending of sRGB codes; no depth (the overlap at joins: stroke.wgsl)
        let stroke_with = |vs: &ShaderModule, ventry: &str, fs: &ShaderModule, fentry: &str| {
            dev.create_render_pipeline(&RenderPipelineDesc {
                layout: &draw_pl,
                vertex: ShaderStage::new(vs, ventry),
                fragment: Some(ShaderStage::new(fs, fentry)),
                buffers: &[],
                topology: Topology::Triangles,
                cull: Cull::None,
                front_cw: false,
                targets: &[ColorTarget { format: desc.color, blend: Some(Blend::PREMULTIPLIED), write_mask: 0xF }],
                depth: None,
            })
        };
        let stroke = stroke_with(&md.vs, "vs", &md.fs, "fs")?;
        let stroke_full = stroke_with(&md.vs_full, "vs_full", &md.fs_full, "fs_full")?;
        let stroke_fill = stroke_with(&md.vs_fill, "vs_fill", &md.fs_fill, "fs_fill")?;
        let mut idx = alloc::vec::Vec::with_capacity(BATCH as usize * 6);
        for q in 0..BATCH {
            let v = (q * 4) as u16;
            idx.extend_from_slice(&[v, v + 1, v + 2, v + 2, v + 1, v + 3]);
        }
        let quads = dev.create_buffer(&BufferDesc { size: idx.len() as u64 * 2, usage: BufferUsage::INDEX, memory: MemoryKind::Device })?;
        dev.queue().write_buffer(&quads, 0, &idx)?;
        let (f, l) = (dev.features(), dev.limits());
        let multi = dev.level() == Level::Voll && f.multi_draw_indirect && f.draw_indirect_first_instance;
        // what the bindings and one dispatch reach: the pool and the visible list, a workgroup per page
        let by_binding = l.max_storage_binding as u64 / RECORD_BYTES;
        let max_entries = l.max_compute_workgroups[0].min(1 << 24);
        let by_dispatch = max_entries as u64 * PAGE as u64;
        let max_records = (by_binding / PAGE as u64 * PAGE as u64).min(by_dispatch).min(1 << 31) as u32;
        Ok(Renderer {
            dev: dev.clone(),
            desc: *desc,
            multi,
            max_records,
            max_entries,
            tag: Arc::new(Tag),
            count,
            scan,
            write,
            scatter,
            stroke,
            stroke_full,
            stroke_fill,
            count_layout,
            scan_layout,
            write_layout,
            scatter_layout,
            draw_layout,
            quads,
        })
    }

    pub fn device(&self) -> &Device {
        &self.dev
    }

    pub fn desc(&self) -> &RendererDesc {
        &self.desc
    }

    /// this device draws the visible layers with one multi-draw (level Voll)
    pub fn multi_draw(&self) -> bool {
        self.multi
    }

    /// the most records (segments, free places included) a scene's pool can hold on this device
    pub fn max_records(&self) -> u32 {
        self.max_records
    }

    /// one frame of a prepared scene into e: its uploads, its visibility, its render pass (the colour attachment's load
    /// and clear as it says, sRGB codes)
    pub fn record<'a>(&'a self, e: &mut CommandEncoder<'a>, s: &'a Scene, color: ColorAttachment<'a>) -> Result<()> {
        if !Arc::ptr_eq(&s.tag, &self.tag) {
            return Err(GpuError::Invalid("record: a scene of another Renderer"));
        }
        if !s.prepared {
            return Err(GpuError::Invalid("record: Scene::prepare comes first"));
        }
        if s.stale {
            return Err(GpuError::Invalid("record: the scene changed after Scene::prepare"));
        }
        if color.view.format() != self.desc.color {
            return Err(GpuError::Invalid("record: the colour target is not in the renderer's format"));
        }
        if color.view.size() != s.size {
            return Err(GpuError::Invalid("record: the colour target is not of the size Scene::prepare was given"));
        }
        let g = s.groups.as_ref().ok_or(GpuError::Invalid("record: Scene::prepare comes first"))?;
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
        // the frame before read and wrote everything this one writes: copies, the visibility's buffers, the indirect
        // draws, the vertex shader's reads
        e.barrier(Access::COPY_READ | Access::COPY_WRITE | Access::COMPUTE_READ | Access::COMPUTE_WRITE | Access::INDIRECT | Access::INDEX_VERTEX | Access::GRAPHICS_READ,
                  Access::COPY_READ | Access::COPY_WRITE | Access::COMPUTE_READ | Access::COMPUTE_WRITE);
        // a new pool: cleared (zeros are free records), then what stays copied over from the old one
        if let Some((old, moves)) = &s.up.rebuild {
            e.fill_buffer(&s.pool, 0, s.pool.size(), 0);
            if !moves.is_empty() {
                e.barrier(Access::COPY_WRITE, Access::COPY_WRITE);
                for &(src, dst, size) in moves {
                    e.copy_buffer(old, src, &s.pool, dst, size);
                }
            }
            // (the records sent below overwrite places of both)
            e.barrier(Access::COPY_WRITE, Access::COPY_WRITE | Access::COMPUTE_WRITE);
        }
        e.copy_buffer(&f.stage, 0, &s.view_ubo, 0, VIEW_BYTES);
        if let Some((up, size)) = &s.up.styles {
            e.copy_buffer(up, 0, &s.styles_buf, 0, *size);
        }
        // (the hatch table: covered by the same barriers as the styles, the one above against the frame before's fragment
        // reads, the one below before this frame's)
        if let Some((up, size)) = &s.up.hatch {
            e.copy_buffer(up, 0, &s.hatch_buf, 0, *size);
        }
        if let Some((up, order, layers)) = &s.up.tables {
            if *order > 0 {
                e.copy_buffer(up, 0, &s.order_buf, 0, *order);
            }
            if *layers > 0 {
                e.copy_buffer(up, *order, &s.layers_buf, 0, *layers);
            }
        }
        if let Some((up, runs)) = &s.up.runs {
            for &(src, dst, size) in runs {
                e.copy_buffer(up, src, &s.pool, dst, size);
            }
        }
        if s.up.scatter > 0 {
            let Some(group) = &f.scatter_group else { return Err(GpuError::Invalid("record: Scene::prepare comes first")) };
            e.set_compute_pipeline(&self.scatter);
            e.set_bind_group(0, group);
            e.push_constants(0, &[s.up.scatter]);
            e.dispatch(s.up.scatter.div_ceil(WG_SCATTER), 1, 1);
        }
        // the uploads (copies, scatter) before the visibility and the vertex shader read them
        e.barrier(Access::COPY_WRITE | Access::COMPUTE_WRITE, Access::COMPUTE_READ | Access::GRAPHICS_READ);
        if let Some(ts) = ts {
            e.timestamp(ts, base + 1);
        }
        let layers = s.vis_layers;
        if layers > 0 {
            if s.entries > 0 {
                e.set_compute_pipeline(&self.count);
                e.set_bind_group(0, &g.count);
                e.dispatch(s.entries, 1, 1);
                // the counts and masks before the scan and the write read them
                e.barrier(Access::COMPUTE_WRITE, Access::COMPUTE_READ);
            }
            e.set_compute_pipeline(&self.scan);
            e.set_bind_group(0, &g.scan);
            e.push_constants(0, &[s.entries, layers, self.multi as u32, 0]);
            e.dispatch(1, 1, 1);
            if s.entries > 0 {
                // the offsets before the write reads them
                e.barrier(Access::COMPUTE_WRITE, Access::COMPUTE_READ);
                e.set_compute_pipeline(&self.write);
                e.set_bind_group(0, &g.write);
                e.dispatch(s.entries, 1, 1);
            }
            // the lists, the draws (scan) and the visible list (write) before the indirect draws and the vertex shader
            e.barrier(Access::COMPUTE_WRITE, Access::INDIRECT | Access::GRAPHICS_READ | Access::COPY_READ);
            if let (true, Some(rb)) = (s.stats_on, &f.readback) {
                e.copy_buffer(&s.lists_buf, 0, rb, 0, layers as u64 * 16);
            }
        }
        if let Some(ts) = ts {
            e.timestamp(ts, base + 2);
        }
        e.begin_render(&RenderPassDesc { color: &[color], depth: None });
        if layers > 0 {
            // runs of visible layers that take the same pipeline: those with fills fs_fill, with arcs or dash patterns
            // fs_full, the others fs
            let class = |k: u32| s.full.get(k as usize).copied().unwrap_or(scene::CLASS_PLAIN);
            let mut k = 0;
            while k < layers {
                let mut n = 1;
                while k + n < layers && class(k + n) == class(k) {
                    n += 1;
                }
                e.set_render_pipeline(match class(k) {
                    scene::CLASS_FILL => &self.stroke_fill,
                    scene::CLASS_FULL => &self.stroke_full,
                    _ => &self.stroke,
                });
                e.set_bind_group(0, &g.draw);
                e.set_index_buffer(&self.quads, 0, IndexFormat::U16);
                if self.multi {
                    e.push_constants(0, &[0u32]);
                    e.draw_indexed_indirect_multi(&s.draws_buf, k as u64 * DRAW_BYTES, n, DRAW_BYTES as u32);
                } else {
                    // WebGPU and Vulkan 1.1: no first instance is sure to work; the layer comes as a push constant
                    for j in k..k + n {
                        e.push_constants(0, &[j]);
                        e.draw_indexed_indirect(&s.draws_buf, j as u64 * DRAW_BYTES);
                    }
                }
                k += n;
            }
        }
        e.end_render();
        if let Some(ts) = ts {
            e.timestamp(ts, base + 3);
        }
        if s.stats_on && layers > 0 {
            e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        }
        Ok(())
    }
}
