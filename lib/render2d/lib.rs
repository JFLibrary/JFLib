// render2d -- the core of the 2D renderer for CAD on lib/gpu (G6a, G6b; docs/RENDER2D.md): a retained scene of
// entities with stable ids (lines, polylines, arcs and circles; Bézier curves, ellipses and splines flattened per zoom
// band on the CPU, curve.rs; fills of rings, triangulated on the CPU, fill.rs; blocks inserted many times, their records
// once on the GPU and each insert an instance with its transform, G6b2), layers and styles (transparency per layer and
// entity, by block); strokes as quads whose
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
//   scene.add_block(&Block) -> BlockId, insert_in(block, style, &Shape); Shape::Insert(Insert) places a block on a
//        layer or in another block (moved by update, arrays as DXF's MINSERT)
//   scene.set_camera(&Camera)                                   centre (f64), scale (units per pixel), rotation
//   dxf::load(&mut scene, &bytes, &dxf::Options) -> Loaded      (G6e2) a DXF drawing into the scene (dxf.rs, dxftext.rs;
//        the reader is the crate jflib-dxf)
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

mod atlas;
mod block;
pub mod curve;
pub mod dxf;
mod dxftext;
pub mod fill;
mod geo;
mod index;
pub mod math;
pub mod msdf;
mod scene;
pub mod text;

pub use scene::{Block, BlockId, Camera, Cap, EntityId, FrameStats, HatchLine, Insert, InstanceInfo, Join, Layer, LayerId, Scene, Shape, Style, StyleId, Transparency, Unit, UploadInfo, ARC_STEP,
                BLOCK_DEPTH, BY_BLOCK_COLOR, BY_BLOCK_DASH, BY_BLOCK_WIDTH, DASH_MAX, HATCH_DASH_MAX, HATCH_MAX, TRANSPARENCY_MAX};
// (G6d) picking, area selection, object snaps; highlighting; the overlay
pub use scene::{box_is_window, Grip, HighlightInfo, Hit, IndexInfo, Overlay, OverlayInfo, Region, SelectMode, Snap, SnapKind, SnapOptions, Space, Target, CROSSING_EDGE, CROSSING_FILL, GRIP_COLD, GRIP_EDGE,
                GRIP_HOT, GRIP_HOVER, GRIP_PX, HL_ALPHA, HL_HOVER, HL_HOVER_RGB, HL_SELECTED, HL_SELECTED_RGB, MARKER_COLOR, MARKER_PX, MARKER_WIDTH, SNAP_ALL, SNAP_CEN, SNAP_END, SNAP_INS, SNAP_INT,
                SNAP_MID, SNAP_NEA, SNAP_PER, SNAP_QUA, SNAP_TAN, WINDOW_EDGE, WINDOW_FILL};
pub use scene::{TextInfo, TEXT_BAR_BELOW, TEXT_FILL_FROM, TEXT_LARGE_FROM};
// (G6e1) the CPU path: Scene::new_cpu, attach, detach, draw_cpu
pub use scene::CpuInfo;
pub use text::{layout_mtext, layout_text, Attach, Bar as TextBar, FontId, FontMetrics, Fonts, GlyphShape, HAlign, Layout as TextLayout, Line as TextLine, MText, Placed as PlacedGlyph, Text, TextStyle, TextStyleId,
               VAlign, FALLBACK_MAX, LINE_SPACING};

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
pub(crate) const VIEW_BYTES: u64 = 80;
/// an instance of a block (cull.wgsl Inst, scene.rs inst_words) and its space of a frame (block.wgsl Space)
pub(crate) const INST_WORDS: usize = 40;
pub(crate) const INST_BYTES: u64 = 160;
pub(crate) const SPACE_BYTES: u64 = 80;
/// an entry of the visibility (cull.wgsl einfo): page, instance + 1, first | count << 16, epx offset
pub(crate) const ENTRY_BYTES: u64 = 16;
pub(crate) const WG_INST: u32 = 64;
/// a hatched style's entry of the hatch table (fill.wgsl Hatch): 32 vec4f
pub(crate) const HATCH_WORDS: usize = 128;
pub(crate) const HATCH_BYTES: u64 = 512;
pub(crate) const DRAW_BYTES: u64 = 20;
/// timestamps per frame: start, after the uploads, after the visibility, after the draw
pub(crate) const STAMPS: u32 = 4;
pub(crate) const WG_SCATTER: u32 = 64;

/// (G6e3) the scatter's changes in one dispatch: as many as its workgroups (of WG_SCATTER) and a storage binding reach, a
/// piece's bytes a multiple of the binding offsets' alignment (the change buffer is bound per piece)
fn scatter_chunk(l: &Limits) -> u32 {
    let bytes = CHANGE_WORDS as u64 * 4;
    let align = (l.min_storage_offset_alignment as u64).max(1);
    let mut n = (l.max_compute_workgroups[0] as u64 * WG_SCATTER as u64).min(l.max_storage_binding as u64 / bytes).min(u32::MAX as u64 / 2).max(1);
    while n > 1 && (n * bytes) % align != 0 {
        n -= 1;
    }
    n as u32
}

/// the shaders, compiled from the *.wgsl beside this file with common.wgsl in front (render2dtest --test shaders
/// compares them with what wgsl::compile makes of them, --gen-shaders writes them again); the browser takes the WGSL
#[cfg(not(target_arch = "wasm32"))]
mod spv {
    include!("shaders_spv.rs");
}

/// the WGSL sources as the device gets them (common.wgsl in front of each, block.wgsl behind it in those that place records;
/// stroke.wgsl with fs_full from full_body.wgsl, the same body as the function full_body, and fill.wgsl)
pub const COMMON_WGSL: &str = include_str!("common.wgsl");
pub const BLOCK_WGSL: &str = include_str!("block.wgsl");
pub const CULL_WGSL: &str = concat!(include_str!("common.wgsl"), include_str!("block.wgsl"), include_str!("textc.wgsl"), include_str!("cull.wgsl"));
pub const SCATTER_WGSL: &str = concat!(include_str!("common.wgsl"), include_str!("scatter.wgsl"));
pub const STROKE_WGSL: &str = concat!(include_str!("common.wgsl"), include_str!("block.wgsl"), include_str!("stroke.wgsl"), "\n@fragment\nfn fs_full(i: VsOut) -> @location(0) vec4f {\n",
                                      include_str!("full_body.wgsl"), "}\n\nfn full_body(i: VsOut) -> vec4f {\n", include_str!("full_body.wgsl"), "}\n", include_str!("fill.wgsl"));
/// text (G6c): STROKE_WGSL with textc.wgsl and text.wgsl behind it (vs_text / fs_text for layers of text alone, vs_text_all
/// / fs_text_all for text among anything else)
pub const TEXT_WGSL: &str = concat!(include_str!("common.wgsl"), include_str!("block.wgsl"), include_str!("stroke.wgsl"), "\n@fragment\nfn fs_full(i: VsOut) -> @location(0) vec4f {\n",
                                    include_str!("full_body.wgsl"), "}\n\nfn full_body(i: VsOut) -> vec4f {\n", include_str!("full_body.wgsl"), "}\n", include_str!("fill.wgsl"), include_str!("textc.wgsl"),
                                    include_str!("text.wgsl"));
pub const TEXTC_WGSL: &str = include_str!("textc.wgsl");
/// (G6d) the highlight's look (behind the stroke and text shaders: hl_wgsl) and the write pass with highlights (behind
/// CULL_WGSL: write_hl_wgsl)
pub const HL_WGSL: &str = include_str!("hl.wgsl");
pub const HL_WRITE_WGSL: &str = include_str!("hl_write.wgsl");

/// (G6d) the vertex shaders that read the highlight: STROKE_WGSL or TEXT_WGSL with every visible list's word through
/// hl_take, ink through hl_ink, half_width through hl_wider, and hl.wgsl behind them (the entries keep their names: vs,
/// vs_full, vs_fill, vs_text, vs_text_all).  The plain sources and their shaders stay as they are
pub fn hl_wgsl(base: &str) -> alloc::string::String {
    let mut s = base.replace("item_of(visible[l.x + j], l.x + j)", "item_of(hl_take(visible[l.x + j]), l.x + j)");
    s = s.replace("fn ink(color: u32, t: u32, layer_t: u32) -> u32 {",
                  "fn ink(color: u32, t: u32, layer_t: u32) -> u32 {\n    return hl_ink(ink_own(color, t, layer_t));\n}\n\nfn ink_own(color: u32, t: u32, layer_t: u32) -> u32 {");
    s = s.replace("fn half_width(st: Style, view: View) -> f32 {",
                  "fn half_width(st: Style, view: View) -> f32 {\n    return half_width_own(st, view) + hl_wider();\n}\n\nfn half_width_own(st: Style, view: View) -> f32 {");
    s.push_str(HL_WGSL);
    s
}

/// (G6d) CULL_WGSL with the write pass that puts the highlight into the visible list (entry write_hl)
pub fn write_hl_wgsl() -> alloc::string::String {
    let mut s = alloc::string::String::from(CULL_WGSL);
    s.push_str(HL_WRITE_WGSL);
    s
}
#[cfg(not(target_arch = "wasm32"))]
pub const COUNT_SPV: &[u32] = spv::COUNT;
#[cfg(not(target_arch = "wasm32"))]
pub const SCAN_SPV: &[u32] = spv::SCAN;
#[cfg(not(target_arch = "wasm32"))]
pub const WRITE_SPV: &[u32] = spv::WRITE;
#[cfg(not(target_arch = "wasm32"))]
pub const INSTANCES_SPV: &[u32] = spv::INSTANCES;
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
#[cfg(not(target_arch = "wasm32"))]
pub const VS_TEXT_SPV: &[u32] = spv::VS_TEXT;
#[cfg(not(target_arch = "wasm32"))]
pub const FS_TEXT_SPV: &[u32] = spv::FS_TEXT;
#[cfg(not(target_arch = "wasm32"))]
pub const VS_TEXT_ALL_SPV: &[u32] = spv::VS_TEXT_ALL;
#[cfg(not(target_arch = "wasm32"))]
pub const FS_TEXT_ALL_SPV: &[u32] = spv::FS_TEXT_ALL;
/// (G6d) the vertex shaders of hl_wgsl and write_hl
#[cfg(not(target_arch = "wasm32"))]
pub const VS_HL_SPV: &[u32] = spv::VS_HL;
#[cfg(not(target_arch = "wasm32"))]
pub const VS_FULL_HL_SPV: &[u32] = spv::VS_FULL_HL;
#[cfg(not(target_arch = "wasm32"))]
pub const VS_FILL_HL_SPV: &[u32] = spv::VS_FILL_HL;
#[cfg(not(target_arch = "wasm32"))]
pub const VS_TEXT_HL_SPV: &[u32] = spv::VS_TEXT_HL;
#[cfg(not(target_arch = "wasm32"))]
pub const VS_TEXT_ALL_HL_SPV: &[u32] = spv::VS_TEXT_ALL_HL;
#[cfg(not(target_arch = "wasm32"))]
pub const WRITE_HL_SPV: &[u32] = spv::WRITE_HL;

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
    /// (G6e3) the changes one dispatch of the scatter shader reaches: its workgroups and its binding (each piece's offset
    /// aligned for a storage binding); more in one frame go up in pieces
    pub(crate) scatter_chunk: u32,
    pub(crate) tag: Arc<Tag>,
    count: ComputePipeline,
    scan: ComputePipeline,
    write: ComputePipeline,
    scatter: ComputePipeline,
    /// the spaces of a frame's instances of blocks (cull.wgsl instances)
    instances: ComputePipeline,
    /// solid segments (stroke.wgsl fs), and everything (fs_full: arcs, dash patterns) for the layers that hold arcs or
    /// dashed strokes; fills and hatches too (fs_fill) for the layers that hold a fill
    stroke: RenderPipeline,
    stroke_full: RenderPipeline,
    /// fills, hatches and everything fs_full draws, for the layers that hold a fill
    stroke_fill: RenderPipeline,
    /// text (G6c): layers of text alone (fs_text), text among anything else (fs_text_all: fs_fill's work too)
    text: RenderPipeline,
    text_all: RenderPipeline,
    pub(crate) text_layout: BindGroupLayout,
    pub(crate) count_layout: BindGroupLayout,
    pub(crate) scan_layout: BindGroupLayout,
    pub(crate) write_layout: BindGroupLayout,
    pub(crate) scatter_layout: BindGroupLayout,
    pub(crate) inst_layout: BindGroupLayout,
    pub(crate) draw_layout: BindGroupLayout,
    /// u16, 0 1 2 2 1 3 per quad, BATCH quads (written once, with the first submission)
    quads: Buffer,
    /// (G6d) the pipelines that read the highlight, made with the first scene that highlights something
    hl: Option<HlPipes>,
}

/// (G6d) the write pass with highlights and the vertex shaders that read them, with the plain fragment shaders
struct HlPipes {
    write: ComputePipeline,
    pub(crate) write_layout: BindGroupLayout,
    stroke: RenderPipeline,
    stroke_full: RenderPipeline,
    stroke_fill: RenderPipeline,
    text: RenderPipeline,
    text_all: RenderPipeline,
}

/// the highlight's shader modules: the write pass, the vertex shaders, the plain fragment shaders
struct HlModules {
    write: ShaderModule,
    vs: ShaderModule,
    vs_full: ShaderModule,
    vs_fill: ShaderModule,
    vs_text: ShaderModule,
    vs_text_all: ShaderModule,
    fs: ShaderModule,
    fs_full: ShaderModule,
    fs_fill: ShaderModule,
    fs_text: ShaderModule,
    fs_text_all: ShaderModule,
}

#[cfg(not(target_arch = "wasm32"))]
fn hl_modules(dev: &Device) -> Result<HlModules> {
    let m = |w: &[u32]| ShaderModule::from_spirv(dev, w);
    Ok(HlModules { write: m(WRITE_HL_SPV)?, vs: m(VS_HL_SPV)?, vs_full: m(VS_FULL_HL_SPV)?, vs_fill: m(VS_FILL_HL_SPV)?, vs_text: m(VS_TEXT_HL_SPV)?, vs_text_all: m(VS_TEXT_ALL_HL_SPV)?, fs: m(FS_SPV)?,
                   fs_full: m(FS_FULL_SPV)?, fs_fill: m(FS_FILL_SPV)?, fs_text: m(FS_TEXT_SPV)?, fs_text_all: m(FS_TEXT_ALL_SPV)? })
}

#[cfg(target_arch = "wasm32")]
fn hl_modules(dev: &Device) -> Result<HlModules> {
    let w = ShaderModule::from_wgsl(dev, &write_hl_wgsl())?;
    let s = ShaderModule::from_wgsl(dev, &hl_wgsl(STROKE_WGSL))?;
    let t = ShaderModule::from_wgsl(dev, &hl_wgsl(TEXT_WGSL))?;
    let fs = ShaderModule::from_wgsl(dev, STROKE_WGSL)?;
    let ft = ShaderModule::from_wgsl(dev, TEXT_WGSL)?;
    Ok(HlModules { write: w, vs: s.clone(), vs_full: s.clone(), vs_fill: s, vs_text: t.clone(), vs_text_all: t, fs: fs.clone(), fs_full: fs.clone(), fs_fill: fs, fs_text: ft.clone(), fs_text_all: ft })
}

struct Modules {
    count: ShaderModule,
    scan: ShaderModule,
    write: ShaderModule,
    instances: ShaderModule,
    scatter: ShaderModule,
    vs: ShaderModule,
    vs_full: ShaderModule,
    fs: ShaderModule,
    fs_full: ShaderModule,
    vs_fill: ShaderModule,
    fs_fill: ShaderModule,
    vs_text: ShaderModule,
    fs_text: ShaderModule,
    vs_text_all: ShaderModule,
    fs_text_all: ShaderModule,
}

#[cfg(not(target_arch = "wasm32"))]
fn modules(dev: &Device) -> Result<Modules> {
    let m = |w: &[u32]| ShaderModule::from_spirv(dev, w);
    Ok(Modules { count: m(COUNT_SPV)?, scan: m(SCAN_SPV)?, write: m(WRITE_SPV)?, instances: m(INSTANCES_SPV)?, scatter: m(SCATTER_SPV)?, vs: m(VS_SPV)?, vs_full: m(VS_FULL_SPV)?, fs: m(FS_SPV)?,
                 fs_full: m(FS_FULL_SPV)?, vs_fill: m(VS_FILL_SPV)?, fs_fill: m(FS_FILL_SPV)?, vs_text: m(VS_TEXT_SPV)?, fs_text: m(FS_TEXT_SPV)?, vs_text_all: m(VS_TEXT_ALL_SPV)?,
                 fs_text_all: m(FS_TEXT_ALL_SPV)? })
}

#[cfg(target_arch = "wasm32")]
fn modules(dev: &Device) -> Result<Modules> {
    let c = ShaderModule::from_wgsl(dev, CULL_WGSL)?;
    let s = ShaderModule::from_wgsl(dev, STROKE_WGSL)?;
    let t = ShaderModule::from_wgsl(dev, TEXT_WGSL)?;
    Ok(Modules { count: c.clone(), scan: c.clone(), write: c.clone(), instances: c, scatter: ShaderModule::from_wgsl(dev, SCATTER_WGSL)?, vs: s.clone(), vs_full: s.clone(), fs: s.clone(),
                 fs_full: s.clone(), vs_fill: s.clone(), fs_fill: s, vs_text: t.clone(), fs_text: t.clone(), vs_text_all: t.clone(), fs_text_all: t })
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
        // (cull.wgsl's bindings: each pass names those it uses, none more than 7 storage buffers: WebGPU's least is 8 a stage)
        // (11: the fill edges' segments in pixels, written by count; 12: the instances' spaces, written by instances, read
        // by count; 13: the instances' transforms)
        let count_layout = dev.create_bind_group_layout(&[b(0, BindingKind::UniformBuffer, cs), b(1, read, cs), b(2, read, cs), b(3, read, cs), b(4, rw, cs), b(5, rw, cs), b(11, rw, cs),
                                                          b(12, rw, cs)])?;
        let scan_layout = dev.create_bind_group_layout(&[b(5, rw, cs), b(6, read, cs), b(7, rw, cs), b(8, rw, cs), b(9, rw, cs)])?;
        let write_layout = dev.create_bind_group_layout(&[b(3, read, cs), b(4, rw, cs), b(7, rw, cs), b(10, rw, cs), b(14, rw, cs)])?;
        let inst_layout = dev.create_bind_group_layout(&[b(0, BindingKind::UniformBuffer, cs), b(12, rw, cs), b(13, read, cs)])?;
        let scatter_layout = dev.create_bind_group_layout(&[b(0, read, cs), b(1, rw, cs)])?;
        // (5: the hatch table, 6: the fill edges' segments in pixels; fs_fill reads the pool too; 7: the instance of each visible
        // record of an insert, 8: the instances' spaces)
        let draw_layout = dev.create_bind_group_layout(&[b(0, BindingKind::UniformBuffer, vfs), b(1, read, vfs), b(2, read, vfs), b(3, read, vs), b(4, read, vs), b(5, read, Stages::FRAGMENT),
                                                         b(6, read, vfs), b(7, read, vs), b(8, read, vs)])?;
        // (G6c: the text pipelines' own layout, the draw's and 9: the atlas, 10: the glyph table; the others keep theirs)
        let text_layout = dev.create_bind_group_layout(&[b(0, BindingKind::UniformBuffer, vfs), b(1, read, vfs), b(2, read, vfs), b(3, read, vs), b(4, read, vs), b(5, read, Stages::FRAGMENT),
                                                         b(6, read, vfs), b(7, read, vs), b(8, read, vs), b(9, BindingKind::Texture, Stages::FRAGMENT), b(10, read, vfs)])?;
        let pl = |g: &BindGroupLayout, push: u32| dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[g], push_constants: push });
        // (only scan, scatter and the draw read a push constant; a layout that says more than its pass sets is an error in the browser, where the
        // values are a bind group)
        let (count_pl, scan_pl, write_pl, scatter_pl, draw_pl) = (pl(&count_layout, 0)?, pl(&scan_layout, 16)?, pl(&write_layout, 0)?, pl(&scatter_layout, 4)?, pl(&draw_layout, 4)?);
        let inst_pl = pl(&inst_layout, 16)?;
        let text_pl = pl(&text_layout, 4)?;
        let compute = |layout: &PipelineLayout, m: &ShaderModule, entry: &str| dev.create_compute_pipeline(&ComputePipelineDesc { layout, shader: ShaderStage::new(m, entry) });
        let count = compute(&count_pl, &md.count, "count")?;
        let scan = compute(&scan_pl, &md.scan, "scan")?;
        let write = compute(&write_pl, &md.write, "write")?;
        let scatter = compute(&scatter_pl, &md.scatter, "main")?;
        let instances = compute(&inst_pl, &md.instances, "instances")?;
        // premultiplied blending of sRGB codes; no depth (the overlap at joins: stroke.wgsl)
        let stroke_with = |vs: &ShaderModule, ventry: &str, fs: &ShaderModule, fentry: &str, layout: &PipelineLayout| {
            dev.create_render_pipeline(&RenderPipelineDesc {
                layout,
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
        let stroke = stroke_with(&md.vs, "vs", &md.fs, "fs", &draw_pl)?;
        let stroke_full = stroke_with(&md.vs_full, "vs_full", &md.fs_full, "fs_full", &draw_pl)?;
        let stroke_fill = stroke_with(&md.vs_fill, "vs_fill", &md.fs_fill, "fs_fill", &draw_pl)?;
        let text = stroke_with(&md.vs_text, "vs_text", &md.fs_text, "fs_text", &text_pl)?;
        let text_all = stroke_with(&md.vs_text_all, "vs_text_all", &md.fs_text_all, "fs_text_all", &text_pl)?;
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
        // (below 2^29: bits 29 and 30 of a visible list's word carry the highlight, G6d)
        let max_records = (by_binding / PAGE as u64 * PAGE as u64).min(by_dispatch).min(1 << 29) as u32;
        let scatter_chunk = scatter_chunk(&l);
        Ok(Renderer {
            dev: dev.clone(),
            desc: *desc,
            multi,
            max_records,
            max_entries,
            scatter_chunk,
            tag: Arc::new(Tag),
            count,
            scan,
            write,
            scatter,
            instances,
            stroke,
            stroke_full,
            stroke_fill,
            text,
            text_all,
            text_layout,
            count_layout,
            scan_layout,
            write_layout,
            scatter_layout,
            inst_layout,
            draw_layout,
            quads,
            hl: None,
        })
    }

    /// (G6d) the highlight's pipelines, made once (Scene::prepare, when a scene highlights something)
    pub(crate) fn ensure_hl(&mut self) -> Result<()> {
        if self.hl.is_some() {
            return Ok(());
        }
        let dev = self.dev.clone();
        let md = hl_modules(&dev)?;
        let b = |binding, kind, stages| Binding { binding, kind, stages };
        let (cs, rw, read) = (Stages::COMPUTE, BindingKind::StorageBuffer, BindingKind::StorageBufferRead);
        let write_layout = dev.create_bind_group_layout(&[b(3, read, cs), b(4, rw, cs), b(7, rw, cs), b(10, rw, cs), b(14, rw, cs), b(15, read, cs)])?;
        let write_pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&write_layout], push_constants: 0 })?;
        let write = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &write_pl, shader: ShaderStage::new(&md.write, "write_hl") })?;
        let draw_pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&self.draw_layout], push_constants: 4 })?;
        let text_pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&self.text_layout], push_constants: 4 })?;
        let color = self.desc.color;
        let pipe = |vs: &ShaderModule, ventry: &str, fs: &ShaderModule, fentry: &str, layout: &PipelineLayout| {
            dev.create_render_pipeline(&RenderPipelineDesc {
                layout,
                vertex: ShaderStage::new(vs, ventry),
                fragment: Some(ShaderStage::new(fs, fentry)),
                buffers: &[],
                topology: Topology::Triangles,
                cull: Cull::None,
                front_cw: false,
                targets: &[ColorTarget { format: color, blend: Some(Blend::PREMULTIPLIED), write_mask: 0xF }],
                depth: None,
            })
        };
        self.hl = Some(HlPipes {
            write,
            write_layout,
            stroke: pipe(&md.vs, "vs", &md.fs, "fs", &draw_pl)?,
            stroke_full: pipe(&md.vs_full, "vs_full", &md.fs_full, "fs_full", &draw_pl)?,
            stroke_fill: pipe(&md.vs_fill, "vs_fill", &md.fs_fill, "fs_fill", &draw_pl)?,
            text: pipe(&md.vs_text, "vs_text", &md.fs_text, "fs_text", &text_pl)?,
            text_all: pipe(&md.vs_text_all, "vs_text_all", &md.fs_text_all, "fs_text_all", &text_pl)?,
        });
        Ok(())
    }

    /// (G6d) the write pass's bind group layout with the highlight (None until the first highlight)
    pub(crate) fn hl_write_layout(&self) -> Option<&BindGroupLayout> {
        self.hl.as_ref().map(|h| &h.write_layout)
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
        if !s.tag.as_ref().is_some_and(|t| Arc::ptr_eq(t, &self.tag)) {
            return Err(GpuError::Invalid("record: a scene of another Renderer, or one without a device"));
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
        // (G6c: the glyph table's rows, a copy beside the styles under the same two barriers: the one above against the frame
        // before's vertex and fragment reads, the one below before this frame's)
        if let Some((up, dst, size)) = &s.up.gtab {
            e.copy_buffer(up, 0, &s.gtab_buf, *dst, *size);
        }
        // (G6c: new tiles of the atlas.  The texture from sampled (the frame before's fragment shaders read it: their reads
        // finish first) or, new, from nothing to a copy's target, the tiles' rectangles, back to sampled before any shader of
        // this frame reads it)
        if let (Some(at), Some(tex)) = (&s.up.atlas, &s.atlas_tex) {
            e.texture_barrier(tex, if at.fresh { Layout::Undefined } else { Layout::Sampled }, Layout::CopyDst);
            for c in &at.copies {
                e.copy_buffer_to_texture(&at.buf, DataLayout { offset: c.offset, bytes_per_row: c.row, rows_per_image: 0 },
                                         TextureRegion { texture: tex, mip: 0, origin: [c.x, c.y, 0], size: [c.w, c.h, 1], base_layer: c.page, layers: 1 });
            }
            e.texture_barrier(tex, Layout::CopyDst, Layout::Sampled);
        }
        if let Some((up, order, layers)) = &s.up.tables {
            if *order > 0 {
                e.copy_buffer(up, 0, &s.einfo_buf, 0, *order);
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
        // (the instances' transforms: under the same two barriers as the records, the one above against the frame before's
        // instance pass, the one below before this frame's)
        if let Some((up, runs)) = &s.up.insts {
            for &(src, dst, size) in runs {
                e.copy_buffer(up, src, &s.inst_buf, dst, size);
            }
        }
        // (G6d: the highlight mask's changed words, read by write_hl; the overlay's records, styles, lists and fill edges, read
        // by the vertex and fragment shaders: under the same two barriers as the records, the one above against the frame
        // before's reads, the one below before this frame's)
        if let (Some((up, runs)), Some(hb)) = (&s.up.hl, &s.hl.buf) {
            for &(src, dst, size) in runs {
                e.copy_buffer(up, src, hb, dst, size);
            }
        }
        if let (Some(ov), Some(g)) = (&s.up.ov, &s.ov_gpu) {
            for &(src, dst, size) in &ov.copies {
                let to = match dst {
                    0 => &g.view,
                    1 => &g.pool,
                    2 => &g.styles,
                    3 => &g.visible,
                    4 => &g.lists,
                    _ => &g.epx,
                };
                e.copy_buffer(&ov.buf, src, to, 0, size);
            }
        }
        if s.up.scatter > 0 {
            // (G6e3) in pieces of what one dispatch reaches, each with its binding of the change buffer: one piece on most
            // desktop GPUs, more on Metal, in the browser and on phones (65 535 workgroups) for millions of records
            if f.scatter_groups.len() < s.up.scatter.div_ceil(self.scatter_chunk) as usize {
                return Err(GpuError::Invalid("record: Scene::prepare comes first"));
            }
            e.set_compute_pipeline(&self.scatter);
            let mut left = s.up.scatter;
            for (k, group) in f.scatter_groups.iter().enumerate() {
                if left == 0 {
                    break;
                }
                let n = left.min(self.scatter_chunk);
                if k > 0 {
                    // (the pieces write other records of the pool; the barrier keeps the validation's view of one buffer
                    // written by two dispatches quiet)
                    e.barrier(Access::COMPUTE_WRITE, Access::COMPUTE_WRITE);
                }
                e.set_bind_group(0, group);
                e.push_constants(0, &[n]);
                e.dispatch(n.div_ceil(WG_SCATTER), 1, 1);
                left -= n;
            }
        }
        // the uploads (copies, scatter) before the visibility and the vertex shader read them
        e.barrier(Access::COPY_WRITE | Access::COMPUTE_WRITE, Access::COMPUTE_READ | Access::GRAPHICS_READ);
        if let Some(ts) = ts {
            e.timestamp(ts, base + 1);
        }
        let layers = s.vis_layers;
        if layers > 0 {
            if s.inst_count > 0 {
                // the instances' spaces of this frame (G6b2), before count reads them (the vertex shader reads them behind
                // the barrier after write, which takes every compute write before it)
                e.set_compute_pipeline(&self.instances);
                e.set_bind_group(0, &g.inst);
                e.push_constants(0, &[s.inst_count, 0, 0, 0]);
                e.dispatch(s.inst_count.div_ceil(WG_INST), 1, 1);
                e.barrier(Access::COMPUTE_WRITE, Access::COMPUTE_READ);
            }
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
                // (G6d: with a highlight the write that puts it into the visible list: the same buffers and the mask)
                match (&self.hl, &s.hl_group, s.hl_on) {
                    (Some(h), Some(hg), true) => {
                        e.set_compute_pipeline(&h.write);
                        e.set_bind_group(0, hg);
                    }
                    _ => {
                        e.set_compute_pipeline(&self.write);
                        e.set_bind_group(0, &g.write);
                    }
                }
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
            // fs_full, the others fs; text alone fs_text, text among others fs_text_all (G6c: their own bind group)
            // (G6d: a layer holding a highlighted entity takes the same class's pipeline with the highlight's vertex shader)
            let hl = |k: u32| s.hl_on && self.hl.is_some() && s.hl_drawn.get(k as usize).copied().unwrap_or(false);
            let class = |k: u32| (s.full.get(k as usize).copied().unwrap_or(scene::CLASS_PLAIN), hl(k));
            let mut k = 0;
            while k < layers {
                let mut n = 1;
                while k + n < layers && class(k + n) == class(k) {
                    n += 1;
                }
                let (cls, lit) = class(k);
                let (pipe, group) = match (self.hl.as_ref().filter(|_| lit), cls) {
                    (Some(h), scene::CLASS_TEXT) => (&h.text, g.text.as_ref()),
                    (Some(h), scene::CLASS_TEXT_ALL) => (&h.text_all, g.text.as_ref()),
                    (Some(h), scene::CLASS_FILL) => (&h.stroke_fill, Some(&g.draw)),
                    (Some(h), scene::CLASS_FULL) => (&h.stroke_full, Some(&g.draw)),
                    (Some(h), _) => (&h.stroke, Some(&g.draw)),
                    (None, scene::CLASS_TEXT) => (&self.text, g.text.as_ref()),
                    (None, scene::CLASS_TEXT_ALL) => (&self.text_all, g.text.as_ref()),
                    (None, scene::CLASS_FILL) => (&self.stroke_fill, Some(&g.draw)),
                    (None, scene::CLASS_FULL) => (&self.stroke_full, Some(&g.draw)),
                    (None, _) => (&self.stroke, Some(&g.draw)),
                };
                let Some(group) = group else {
                    k += n;
                    continue;
                };
                e.set_render_pipeline(pipe);
                e.set_bind_group(0, group);
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
        // (G6d) the overlay over everything: its own records, one draw of fs_fill (strokes, arcs, dashes, fills)
        if let (Some(ov), Some(gg)) = (&s.up.ov, &s.ov_gpu) {
            if let (true, Some(group)) = (ov.quads > 0, &gg.group) {
                e.set_render_pipeline(&self.stroke_fill);
                e.set_bind_group(0, group);
                e.set_index_buffer(&self.quads, 0, IndexFormat::U16);
                e.push_constants(0, &[0u32]);
                e.draw_indexed(6 * ov.quads.min(BATCH), ov.quads.div_ceil(BATCH), 0, 0, 0);
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
