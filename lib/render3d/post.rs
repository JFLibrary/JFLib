// post -- (G7f) what comes after the scene's passes (post.wgsl; docs/RENDER3D.md "Umrisse", "Kantenglättung") and the
// particles' pipelines (particles.wgsl): outlines from the depth buffer, FXAA from an image of the scene's own.  Per depth
// attachment the renderer was given, in a chain a shared borrow can grow (as hiz.rs: Renderer::record takes &self): the
// outline's bind group (that depth view) and FXAA's image (the attachment's size, the renderer's colour format) with its
// group.  Scene::prepare drops those not used lately.

use alloc::boxed::Box;
use core::cell::{Cell, OnceCell};

use gpu::*;

/// (G7f) lines on the edges of the depth buffer (Scene::set_outline): silhouettes and creases, drawn over the opaque image
/// before blended instances and particles.  Made with `new` and changed field by field (it may grow)
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Outline {
    /// 1 ..= 8 pixels: on a silhouette's near side the line is this wide, across a crease about twice
    pub width: u32,
    /// linear colour and opacity
    pub color: [f32; 4],
    /// how sharp a crease must be to get a line: the change of the surface's slope across it (tan of its angle to the
    /// view; 0.5 at first, about 30 degrees seen head on).  Larger: fewer creases; jumps of the depth always get one
    pub crease: f32,
}

impl Outline {
    /// lines `width` pixels wide in this colour, creases from 0.5
    pub const fn new(width: u32, color: [f32; 4]) -> Outline {
        Outline { width, color, crease: 0.5 }
    }

    pub(crate) fn check(&self) -> Result<()> {
        let fine = (1..=8).contains(&self.width) && self.color.iter().all(|v| v.is_finite() && *v >= 0.0) && self.crease.is_finite() && self.crease > 0.0;
        if fine { Ok(()) } else { Err(GpuError::Invalid("Outline: 1 ..= 8 pixels, colour finite and not negative, crease above 0")) }
    }
}

/// (G7f) anti-aliasing as a pass over the finished image (Scene::set_antialias)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Antialias {
    None,
    /// FXAA 3.11, quality preset 12: the scene is drawn into an image of the renderer's (the target's size and format), FXAA
    /// writes it to the target.  The target's old contents are replaced (Load::Keep becomes a clear)
    Fxaa,
}

/// bytes of post.wgsl's Outline
pub(crate) const OUTLINE_BYTES: u64 = 64;

/// the shader modules of post.wgsl's and particles.wgsl's entry points (one each on Vulkan, one module in the browser)
pub(crate) struct PostModules {
    pub vs_full: ShaderModule,
    pub fs_outline: ShaderModule,
    pub fs_fxaa: ShaderModule,
    pub cs_particles: ShaderModule,
    pub vs_particle: ShaderModule,
    pub fs_particle: ShaderModule,
}

/// the layouts and pipelines (the renderer's)
pub(crate) struct Post {
    pub outline_layout: BindGroupLayout,
    pub fxaa_layout: BindGroupLayout,
    pub outline: RenderPipeline,
    pub fxaa: RenderPipeline,
    pub sampler: Sampler,
    /// the outline's uniform block: every record copies its scene's into it
    pub ubo: Buffer,
    /// (particles.wgsl) the motion and list, and the draw
    pub particles_cs_layout: BindGroupLayout,
    pub particles_layout: BindGroupLayout,
    pub particles_cs: ComputePipeline,
    pub particles: RenderPipeline,
}

impl Post {
    pub fn new(dev: &Device, m: &PostModules, color: Format, encode: bool) -> Result<Post> {
        let b = |binding, kind, stages| Binding { binding, kind, stages };
        let fs = Stages::FRAGMENT;
        let outline_layout = dev.create_bind_group_layout(&[b(0, BindingKind::UniformBuffer, fs), b(1, BindingKind::Texture, fs)])?;
        let fxaa_layout = dev.create_bind_group_layout(&[b(2, BindingKind::Texture, fs), b(3, BindingKind::Sampler, fs)])?;
        let ol = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&outline_layout], push_constants: 0 })?;
        let fl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&fxaa_layout], push_constants: 0 })?;
        let enc = [Constant { id: 0, value: 1.0 }];
        let encode_c: &[Constant] = if encode { &enc } else { &[] };
        // sRGB and float targets: their sampler returns linear light, FXAA's luma takes its square root
        let linear = color.is_srgb() || matches!(color, Format::Rgba16Float | Format::Rgba32Float | Format::R16Float | Format::Rg16Float | Format::R32Float
                                                     | Format::Rg32Float | Format::Rg11b10Float);
        let lin = [Constant { id: 1, value: 1.0 }];
        let linear_c: &[Constant] = if linear { &lin } else { &[] };
        let full = |layout: &PipelineLayout, module: &ShaderModule, entry: &'static str, constants: &[Constant], blend: Option<Blend>| dev.create_render_pipeline(&RenderPipelineDesc {
            layout,
            vertex: ShaderStage::new(&m.vs_full, "vs_full"),
            fragment: Some(ShaderStage { module, entry, constants }),
            buffers: &[],
            topology: Topology::Triangles,
            cull: Cull::None,
            front_cw: false,
            targets: &[ColorTarget { format: color, blend, write_mask: 0xF }],
            depth: None,
        });
        let outline = full(&ol, &m.fs_outline, "fs_outline", encode_c, Some(Blend::PREMULTIPLIED))?;
        let fxaa = full(&fl, &m.fs_fxaa, "fs_fxaa", linear_c, None)?;
        let sampler = dev.create_sampler(&SamplerDesc { mip: Filter::Nearest, ..SamplerDesc::LINEAR })?;
        let ubo = dev.create_buffer(&BufferDesc { size: OUTLINE_BYTES, usage: BufferUsage::UNIFORM, memory: MemoryKind::Device })?;
        let (cs, vs) = (Stages::COMPUTE, Stages::VERTEX);
        let particles_cs_layout = dev.create_bind_group_layout(&[b(0, BindingKind::UniformBuffer, cs), b(1, BindingKind::StorageBufferRead, cs), b(2, BindingKind::StorageBuffer, cs)])?;
        let particles_layout = dev.create_bind_group_layout(&[b(0, BindingKind::UniformBuffer, vs), b(1, BindingKind::StorageBufferRead, vs), b(3, BindingKind::UniformBuffer, vs),
                                                              b(4, BindingKind::UniformBuffer, vs), b(5, BindingKind::UniformBuffer, fs), b(6, BindingKind::StorageBufferRead, vs)])?;
        let pcl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&particles_cs_layout], push_constants: 0 })?;
        let pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&particles_layout], push_constants: 0 })?;
        let particles_cs = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pcl, shader: ShaderStage::new(&m.cs_particles, "cs_particles") })?;
        // no culling (a billboard faces the camera either way), the depth tested and not written, premultiplied alpha
        let particles = dev.create_render_pipeline(&RenderPipelineDesc {
            layout: &pl,
            vertex: ShaderStage::new(&m.vs_particle, "vs_particle"),
            fragment: Some(ShaderStage { module: &m.fs_particle, entry: "fs_particle", constants: encode_c }),
            buffers: &[],
            topology: Topology::Triangles,
            cull: Cull::None,
            front_cw: false,
            targets: &[ColorTarget { format: color, blend: Some(Blend::PREMULTIPLIED), write_mask: 0xF }],
            depth: Some(DepthState { format: Format::Depth32Float, write: false, compare: Compare::Greater }),
        })?;
        Ok(Post { outline_layout, fxaa_layout, outline, fxaa, sampler, ubo, particles_cs_layout, particles_layout, particles_cs, particles })
    }
}

/// FXAA's image of the scene: the renderer's colour format, the attachment's size, and its group
pub(crate) struct Image {
    pub tex: Texture,
    pub group: BindGroup,
}

/// a link of the chain: the depth attachment's view (it keeps it alive), the outline's group, FXAA's image, when it was
/// last used
pub(crate) struct Node {
    depth: TextureView,
    outline: OnceCell<BindGroup>,
    image: OnceCell<Image>,
    used: Cell<u64>,
    next: OnceCell<Box<Node>>,
}

impl Node {
    /// the outline's group (the attachment must be sampled: record checks it)
    pub fn outline<'a>(&'a self, dev: &Device, p: &Post) -> Result<&'a BindGroup> {
        if let Some(g) = self.outline.get() {
            return Ok(g);
        }
        let g = dev.create_bind_group(&BindGroupDesc { layout: &p.outline_layout, entries: &[BindEntry { binding: 0, resource: Resource::Buffer { buffer: &p.ubo, offset: 0, size: 0 } },
                                                                                          BindEntry { binding: 1, resource: Resource::Texture(&self.depth) }] })?;
        let _ = self.outline.set(g);
        self.outline.get().ok_or(GpuError::Invalid("post: no group"))
    }

    /// FXAA's image in this colour format
    pub fn image<'a>(&'a self, dev: &Device, p: &Post, color: Format) -> Result<&'a Image> {
        if let Some(i) = self.image.get() {
            return Ok(i);
        }
        let (w, h) = self.depth.size();
        let tex = dev.create_texture(&TextureDesc::d2(w, h, color, TextureUsage::COLOR_TARGET | TextureUsage::SAMPLED))?;
        let group = dev.create_bind_group(&BindGroupDesc { layout: &p.fxaa_layout, entries: &[BindEntry { binding: 2, resource: Resource::Texture(tex.view()) },
                                                                                       BindEntry { binding: 3, resource: Resource::Sampler(&p.sampler) }] })?;
        let _ = self.image.set(Image { tex, group });
        self.image.get().ok_or(GpuError::Invalid("post: no image"))
    }
}

/// the node of a depth attachment: found in the chain, or made and put at its end
pub(crate) fn find<'a>(head: &'a OnceCell<Box<Node>>, depth: &TextureView, now: u64) -> &'a Node {
    let mut at = head;
    loop {
        match at.get() {
            Some(n) if n.depth.same(depth) => {
                n.used.set(now);
                return n;
            }
            Some(n) => at = &n.next,
            None => {
                let _ = at.set(Box::new(Node { depth: depth.clone(), outline: OnceCell::new(), image: OnceCell::new(), used: Cell::new(now), next: OnceCell::new() }));
                if let Some(n) = at.get() {
                    return n;
                }
            }
        }
    }
}

/// keep the nodes used at or after `since` (the others go when the GPU is done with them)
pub(crate) fn prune(head: &mut OnceCell<Box<Node>>, since: u64) {
    let mut keep = alloc::vec::Vec::new();
    let mut cur = head.take();
    while let Some(mut n) = cur {
        cur = n.next.take();
        if n.used.get() >= since {
            keep.push(n);
        }
    }
    let mut next = OnceCell::new();
    while let Some(mut n) = keep.pop() {
        n.next = next;
        next = OnceCell::new();
        let _ = next.set(n);
    }
    *head = next;
}
