// hiz -- the depth pyramid of the camera's occlusion test (G5d; hiz.wgsl, docs/RENDER3D.md "Verdeckung"): a square
// R32Float texture of 2^(levels - 1) texels whose mip k - 1 holds level k of the depth buffer, built in every frame
// between the two phases, and its bind groups.  One per depth attachment the renderer was given, in a chain that a
// shared borrow can grow (Renderer::record takes &self, and the encoder borrows what it binds); Scene::prepare (with
// &mut) drops those not used in the last records.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::{Cell, OnceCell};

use gpu::*;

/// hiz.wgsl's workgroup: 8 x 8
const WG: u32 = 8;
/// records a pyramid stays without being used
pub(crate) const KEEP: u64 = 16;

/// the layouts and pipelines that make pyramids (the renderer's)
pub(crate) struct Maker {
    depth: BindGroupLayout,
    down: BindGroupLayout,
    pub read: BindGroupLayout,
    from_depth: ComputePipeline,
    step: ComputePipeline,
}

/// texels of level k that cover an image of n pixels
fn valid(n: u32, k: u32) -> u32 {
    n.div_ceil(1 << k)
}

impl Maker {
    pub fn new(dev: &Device, from_depth: &ShaderModule, down: &ShaderModule) -> Result<Maker> {
        let cs = Stages::COMPUTE;
        let b = |binding, kind| Binding { binding, kind, stages: cs };
        // (two layouts: the browser tells a depth texture from a float one in the layout)
        let depth = dev.create_bind_group_layout(&[b(0, BindingKind::Texture), b(1, BindingKind::StorageTexture)])?;
        let dl = dev.create_bind_group_layout(&[b(1, BindingKind::StorageTexture), b(2, BindingKind::Texture)])?;
        let read = dev.create_bind_group_layout(&[b(0, BindingKind::Texture)])?;
        let pl = |g: &BindGroupLayout| dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[g], push_constants: 12 });
        let (p1, p2) = (pl(&depth)?, pl(&dl)?);
        let from_depth = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &p1, shader: ShaderStage::new(from_depth, "from_depth") })?;
        let step = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &p2, shader: ShaderStage::new(down, "down") })?;
        Ok(Maker { depth, down: dl, read, from_depth, step })
    }
}

/// a depth buffer's pyramid
pub(crate) struct Pyramid {
    tex: Texture,
    /// levels 1 ..= levels, mips 0 .. levels
    pub levels: u32,
    /// the depth buffer's size
    pub w: u32,
    pub h: u32,
    /// level 1 from the depth buffer, then level k from level k - 1
    groups: Vec<BindGroup>,
    /// every mip, sampled: group 1 of phase 2's visibility
    pub read: BindGroup,
}

impl Pyramid {
    /// None for a depth buffer of one pixel (nothing to test)
    fn new(dev: &Device, m: &Maker, depth: &TextureView) -> Result<Option<Pyramid>> {
        let (w, h) = depth.size();
        let most = w.max(h);
        if most < 2 {
            return Ok(None);
        }
        // until one texel covers it all: ceil(log2(most))
        let levels = 32 - (most - 1).leading_zeros();
        let side = 1u32 << (levels - 1);
        let tex = dev.create_texture(&TextureDesc { width: side, height: side, depth: 1, layers: 1, mips: levels, format: Format::R32Float,
                                                    usage: TextureUsage::STORAGE | TextureUsage::SAMPLED, cube: false })?;
        let mip = |k: u32| tex.create_view(&ViewDesc { dim: ViewDim::D2, base_mip: k, mips: 1, base_layer: 0, layers: 1 });
        let mut groups = Vec::with_capacity(levels as usize);
        fn e(binding: u32, resource: Resource<'_>) -> BindEntry<'_> {
            BindEntry { binding, resource }
        }
        for k in 0..levels {
            let out = mip(k)?;
            let g = if k == 0 {
                dev.create_bind_group(&BindGroupDesc { layout: &m.depth, entries: &[e(0, Resource::Texture(depth)), e(1, Resource::StorageTexture(&out))] })?
            } else {
                let src = mip(k - 1)?;
                dev.create_bind_group(&BindGroupDesc { layout: &m.down, entries: &[e(1, Resource::StorageTexture(&out)), e(2, Resource::Texture(&src))] })?
            };
            groups.push(g);
        }
        let all = tex.create_view(&ViewDesc { dim: ViewDim::D2, base_mip: 0, mips: 0, base_layer: 0, layers: 1 })?;
        let read = dev.create_bind_group(&BindGroupDesc { layout: &m.read, entries: &[e(0, Resource::Texture(&all))] })?;
        Ok(Some(Pyramid { tex, levels, w, h, groups, read }))
    }

    /// every level, after phase 1's pass left the depth buffer in Layout::Sampled; afterwards every mip is
    /// Layout::Sampled for phase 2's visibility.  Barriers: Undefined -> General for all mips (the old contents need
    /// not stay; it waits for the compute shaders of the frame before that read them), and after each level's
    /// dispatch General -> Sampled for that mip (its writes before the next level's and phase 2's reads)
    pub fn record<'a>(&'a self, e: &mut CommandEncoder<'a>, m: &'a Maker) {
        e.texture_barrier(&self.tex, Layout::Undefined, Layout::General);
        for k in 1..=self.levels {
            e.set_compute_pipeline(if k == 1 { &m.from_depth } else { &m.step });
            e.set_bind_group(0, &self.groups[k as usize - 1]);
            e.push_constants(0, &[self.w, self.h, k]);
            e.dispatch(valid(self.w, k).div_ceil(WG), valid(self.h, k).div_ceil(WG), 1);
            e.texture_barrier_range(&self.tex, k - 1, 1, 0, 1, Layout::General, Layout::Sampled);
        }
    }

    /// (tests) the texture: mip k - 1 holds level k
    pub fn texture(&self) -> &Texture {
        &self.tex
    }
}

/// a link of the chain: the attachment's view (it keeps it alive), its pyramid, when it was last used
pub(crate) struct Node {
    depth: TextureView,
    p: Option<Pyramid>,
    used: Cell<u64>,
    next: OnceCell<Box<Node>>,
}

/// the pyramid of a depth attachment: found in the chain, or made and put at its end (None: a single pixel)
pub(crate) fn find<'a>(head: &'a OnceCell<Box<Node>>, dev: &Device, m: &Maker, depth: &TextureView, now: u64) -> Result<Option<&'a Pyramid>> {
    let mut at = head;
    loop {
        match at.get() {
            Some(n) if n.depth.same(depth) => {
                n.used.set(now);
                return Ok(n.p.as_ref());
            }
            Some(n) => at = &n.next,
            None => {
                let p = Pyramid::new(dev, m, depth)?;
                let _ = at.set(Box::new(Node { depth: depth.clone(), p, used: Cell::new(now), next: OnceCell::new() }));
                return Ok(at.get().and_then(|n| n.p.as_ref()));
            }
        }
    }
}

/// keep the pyramids used at or after `since` (the others go when the GPU is done with them)
pub(crate) fn prune(head: &mut OnceCell<Box<Node>>, since: u64) {
    let mut keep = Vec::new();
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

/// (tests) the pyramid made for a depth attachment, if there is one
pub(crate) fn of<'a>(head: &'a OnceCell<Box<Node>>, depth: &TextureView) -> Option<&'a Pyramid> {
    let mut at = head;
    while let Some(n) = at.get() {
        if n.depth.same(depth) {
            return n.p.as_ref();
        }
        at = &n.next;
    }
    None
}
