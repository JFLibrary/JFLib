// mtl_cmd -- command recording on Metal (AP6, docs/APPLE.md): CommandEncoder -> CommandBuffer
//
// An encoder records into one MTLCommandBuffer through Metal's encoders: copies and fills in a
// blit encoder, dispatches in a compute encoder, a render pass in a render encoder; switching
// kinds ends the current one.  barrier / texture_barrier end the current encoder too: Metal
// orders encoders that touch the same (hazard-tracked) resources itself, and the layouts mean
// nothing here.  Bind groups and push constants are remembered and set before the next dispatch
// or draw, at the slots the pipeline's translation gave each (group, binding) of each stage
// (shader::msl: buffers, textures, samplers; push constants at PUSH_SLOT, the bound buffer sizes
// at SIZES_SLOT).  fill_buffer runs a small kernel of its own (a blit fill takes one byte).
// The checks are those of the Vulkan backend (vk_cmd.rs); finish() returns the first misuse.

use core::marker::PhantomData;

use crate::mtl_dev::{Device, Shared};
use crate::mtl_objc::{self as objc, Id, MTLClearColor, MTLScissorRect, MTLSize, MTLViewport, Obj, Pool, msg};
use crate::mtl_res::{PipeInner, Res, StageInfo, copy_region, encode_buffer_to_texture, encode_texture_to_buffer};
use crate::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blit,
    Compute,
    Render,
}

struct Pass {
    colors: [Option<Format>; COLOR_TARGETS_MAX],
    n: usize,
    depth: Option<Format>,
    size: (u32, u32),
}

/// records commands; finish() makes a CommandBuffer for Queue::submit
pub struct CommandEncoder<'a> {
    sh: &'a Shared,
    cb: Option<Obj>,
    enc: Option<(Obj, Kind)>,
    err: Option<GpuError>,
    pass: Option<Pass>,
    compute_pipe: Option<&'a PipeInner>,
    graphics: Option<&'a PipeInner>,
    groups: [Option<&'a BindGroup>; BIND_GROUPS_MAX as usize],
    push: [u8; PUSH_CONSTANTS_MAX as usize],
    /// the bindings must be set again before the next dispatch / draw
    dirty: bool,
    /// index buffer, its offset, 4-byte indices
    index: Option<(Id, u64, bool)>,
    _borrow: PhantomData<&'a ()>,
}

/// recorded commands, ready to submit (dropped unsubmitted: discarded)
pub struct CommandBuffer<'a> {
    cb: Obj,
    _borrow: PhantomData<&'a ()>,
}

impl CommandBuffer<'_> {
    pub(crate) fn into_obj(self) -> Obj {
        self.cb
    }
}

impl Device {
    /// a command encoder of the device's queue
    pub fn encoder(&self) -> Result<CommandEncoder<'_>> {
        let cb = self.sh.command_buffer()?;
        Ok(CommandEncoder {
            sh: &self.sh,
            cb: Some(cb),
            enc: None,
            err: None,
            pass: None,
            compute_pipe: None,
            graphics: None,
            groups: [None; BIND_GROUPS_MAX as usize],
            push: [0; PUSH_CONSTANTS_MAX as usize],
            dirty: true,
            index: None,
            _borrow: PhantomData,
        })
    }
}

impl Drop for CommandEncoder<'_> {
    fn drop(&mut self) {
        // an encoder left open would make Metal complain when the buffer goes
        self.end_encoder();
    }
}

impl<'a> CommandEncoder<'a> {
    #[inline]
    fn ok(&mut self, cond: bool, what: &'static str) -> Option<()> {
        if self.err.is_some() {
            return None;
        }
        if !cond {
            self.err = Some(GpuError::Invalid(what));
            return None;
        }
        Some(())
    }

    #[inline]
    fn outside(&mut self, what: &'static str) -> Option<()> {
        let c = self.pass.is_none();
        self.ok(c, what)
    }

    fn buf_range(&mut self, b: &Buffer, off: u64, n: u64, what: &'static str) -> Option<()> {
        let c = off.checked_add(n).is_some_and(|e| e <= b.size());
        self.ok(c, what)
    }

    fn cb(&self) -> Id {
        self.cb.as_ref().map_or(objc::NIL, |c| c.id())
    }

    fn end_encoder(&mut self) {
        if let Some((e, _)) = self.enc.take() {
            // SAFETY: the open encoder of this command buffer, ended once.
            unsafe { msg!(e.id(), c"endEncoding"; ()) };
        }
    }

    /// the open encoder of kind k (a new one after ending another kind)
    fn encoder(&mut self, k: Kind) -> Option<Id> {
        if let Some((e, kind)) = &self.enc {
            if *kind == k {
                return Some(e.id());
            }
        }
        self.end_encoder();
        let _pool = Pool::new();
        let cb = self.cb();
        // SAFETY: blitCommandEncoder / computeCommandEncoder of the command buffer (autoreleased,
        // retained inside the pool); no other encoder is open.
        let e = unsafe {
            match k {
                Kind::Blit => msg!(cb, c"blitCommandEncoder"; Id),
                _ => msg!(cb, c"computeCommandEncoder"; Id),
            }
        };
        let Some(e) = Obj::retained(e) else {
            self.err.get_or_insert(GpuError::OutOfMemory);
            return None;
        };
        let id = e.id();
        self.enc = Some((e, k));
        self.dirty = true;
        Some(id)
    }

    // ------------------------------------------------------------ copies

    pub fn copy_buffer(&mut self, src: &'a Buffer, src_off: u64, dst: &'a Buffer, dst_off: u64, size: u64) {
        if self.outside("copy_buffer inside a render pass").is_none()
            || self.buf_range(src, src_off, size, "copy_buffer: source range").is_none()
            || self.buf_range(dst, dst_off, size, "copy_buffer: destination range").is_none()
            || size == 0
        {
            return;
        }
        let Some(b) = self.encoder(Kind::Blit) else { return };
        // SAFETY: an open blit encoder; ranges checked.
        unsafe {
            msg!(b, c"copyFromBuffer:sourceOffset:toBuffer:destinationOffset:size:", src.0.h.id() => Id, src_off as usize => usize,
                 dst.0.h.id() => Id, dst_off as usize => usize, size as usize => usize; ());
        }
    }

    /// fill size bytes at offset with a 32-bit value (both multiples of 4)
    pub fn fill_buffer(&mut self, buf: &'a Buffer, offset: u64, size: u64, value: u32) {
        if self.outside("fill_buffer inside a render pass").is_none()
            || self.ok(offset % 4 == 0 && size % 4 == 0 && size > 0, "fill_buffer: offset and size multiples of 4").is_none()
            || self.buf_range(buf, offset, size, "fill_buffer: range").is_none()
        {
            return;
        }
        let bytes = value.to_le_bytes();
        if bytes.iter().all(|&b| b == bytes[0]) {
            let Some(b) = self.encoder(Kind::Blit) else { return };
            let range = objc::NSRange { location: offset as usize, length: size as usize };
            // SAFETY: an open blit encoder; range checked.
            unsafe { msg!(b, c"fillBuffer:range:value:", buf.0.h.id() => Id, range => objc::NSRange, bytes[0] => u8; ()) };
            return;
        }
        let Some(state) = self.fill_state() else { return };
        let Some(c) = self.encoder(Kind::Compute) else { return };
        let words = (size / 4) as u32;
        let args = [value, words];
        let groups = words.div_ceil(64) as usize;
        // SAFETY: an open compute encoder with the fill kernel: word i < words of the buffer from
        // offset gets the value (the buffer's length is a whole number of words).
        unsafe {
            msg!(c, c"setComputePipelineState:", state => Id; ());
            msg!(c, c"setBuffer:offset:atIndex:", buf.0.h.id() => Id, offset as usize => usize, 0usize => usize; ());
            msg!(c, c"setBytes:length:atIndex:", args.as_ptr() as *const core::ffi::c_void => *const core::ffi::c_void, 8usize => usize, 1usize => usize; ());
            msg!(c, c"dispatchThreadgroups:threadsPerThreadgroup:", MTLSize { w: groups, h: 1, d: 1 } => MTLSize,
                 MTLSize { w: 64, h: 1, d: 1 } => MTLSize; ());
        }
        self.dirty = true;
    }

    /// the fill kernel's pipeline (compiled once per device)
    fn fill_state(&mut self) -> Option<Id> {
        let mut f = self.sh.fill.lock();
        if f.is_none() {
            let src = "#include <metal_stdlib>\nusing namespace metal;\n\
                       kernel void jf_fill(device uint* b [[buffer(0)]], constant uint2& a [[buffer(1)]], uint i [[thread_position_in_grid]]) {\n\
                       if (i < a.y) { b[i] = a.x; }\n}\n";
            let made = crate::mtl_res::compile(self.sh, src).ok().and_then(|lib| {
                let _pool = Pool::new();
                let name = objc::nsstring("jf_fill")?;
                // SAFETY: newFunctionWithName: / newComputePipelineStateWithFunction:error: on live
                // objects (+1 results or nil).
                unsafe {
                    let func = Obj::new(msg!(lib.id(), c"newFunctionWithName:", name.id() => Id; Id))?;
                    let mut err: Id = objc::NIL;
                    Obj::new(msg!(self.sh.dev.id(), c"newComputePipelineStateWithFunction:error:", func.id() => Id, &mut err => *mut Id; Id))
                }
            });
            *f = made;
        }
        let id = f.as_ref().map(|o| o.id());
        drop(f);
        if id.is_none() {
            self.err.get_or_insert(GpuError::Failed(0));
        }
        id
    }

    /// buffer data -> a texture region
    pub fn copy_buffer_to_texture(&mut self, src: &'a Buffer, layout: DataLayout, dst: TextureRegion<'a>) {
        if self.outside("copy inside a render pass").is_none() {
            return;
        }
        let c = copy_region(&dst, &layout, src.size());
        let ok = c.is_ok() && dst.texture.usage().contains(TextureUsage::COPY_DST);
        if self.ok(ok, "copy_buffer_to_texture: region, data layout or COPY_DST usage").is_none() {
            return;
        }
        let Ok(c) = c else { return };
        let Some(b) = self.encoder(Kind::Blit) else { return };
        encode_buffer_to_texture(b, src.0.h.id(), layout.offset, &c, &dst);
    }

    /// a texture region -> buffer data
    pub fn copy_texture_to_buffer(&mut self, src: TextureRegion<'a>, dst: &'a Buffer, layout: DataLayout) {
        if self.outside("copy inside a render pass").is_none() {
            return;
        }
        let c = copy_region(&src, &layout, dst.size());
        let ok = c.is_ok() && src.texture.usage().contains(TextureUsage::COPY_SRC);
        if self.ok(ok, "copy_texture_to_buffer: region, data layout or COPY_SRC usage").is_none() {
            return;
        }
        let Ok(c) = c else { return };
        let Some(b) = self.encoder(Kind::Blit) else { return };
        encode_texture_to_buffer(b, dst.0.h.id(), layout.offset, &c, &src);
    }

    // ------------------------------------------------------------ barriers

    /// the accesses `before` finish before the `after` ones: the next command starts a new encoder
    pub fn barrier(&mut self, _before: Access, _after: Access) {
        if self.outside("barrier inside a render pass").is_none() {
            return;
        }
        self.end_encoder();
    }

    pub fn texture_barrier(&mut self, tex: &'a Texture, before: Layout, after: Layout) {
        self.texture_barrier_range(tex, 0, tex.mips(), 0, tex.layers(), before, after);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn texture_barrier_range(&mut self, tex: &'a Texture, base_mip: u32, mips: u32, base_layer: u32, layers: u32,
                                 _before: Layout, after: Layout) {
        if self.outside("texture_barrier inside a render pass").is_none()
            || self.ok(mips > 0 && layers > 0 && base_mip + mips <= tex.mips() && base_layer + layers <= tex.layers(),
                       "texture_barrier: range outside the texture").is_none()
            || self.ok(after != Layout::Undefined, "texture_barrier: Undefined is no destination").is_none()
        {
            return;
        }
        self.end_encoder();
    }

    // ------------------------------------------------------------ compute

    pub fn set_compute_pipeline(&mut self, p: &'a ComputePipeline) {
        if self.outside("set_compute_pipeline inside a render pass").is_none() {
            return;
        }
        self.compute_pipe = Some(&p.0);
        self.dirty = true;
    }

    fn current(&mut self) -> Option<&'a PipeInner> {
        let p = if self.pass.is_some() { self.graphics } else { self.compute_pipe };
        self.ok(p.is_some(), "bind group / push constants before a pipeline")?;
        p
    }

    /// bind group g (= @group(g)) of the current pipeline's layout
    pub fn set_bind_group(&mut self, index: u32, group: &'a BindGroup) {
        let Some(p) = self.current() else { return };
        let fits = p.layout.0.groups.get(index as usize).is_some_and(|l| alloc::sync::Arc::ptr_eq(&l.0, &group.0.layout));
        if self.ok(fits, "set_bind_group: the group's layout differs from the pipeline layout's").is_none() {
            return;
        }
        self.groups[index as usize] = Some(group);
        self.dirty = true;
    }

    /// push constants at offset (bytes) for the current pipeline
    pub fn push_constants<T: Pod>(&mut self, offset: u32, data: &[T]) {
        let Some(p) = self.current() else { return };
        let bytes = bytes_of(data);
        let n = bytes.len() as u32;
        let fits = offset % 4 == 0 && n % 4 == 0 && n > 0 && offset + n <= p.layout.0.push;
        if self.ok(fits, "push_constants: range outside the layout's (multiples of 4)").is_none() {
            return;
        }
        self.push[offset as usize..(offset + n) as usize].copy_from_slice(bytes);
        self.dirty = true;
    }

    /// set a stage's bindings on the open encoder: stage 0 compute, 1 vertex, 2 fragment
    fn bind(&mut self, e: Id, info: &StageInfo, stage: u32) -> Option<()> {
        use shader::msl::SlotKind;
        let (buf, tex, smp, bytes) = match stage {
            0 => (c"setBuffer:offset:atIndex:", c"setTexture:atIndex:", c"setSamplerState:atIndex:", c"setBytes:length:atIndex:"),
            1 => (c"setVertexBuffer:offset:atIndex:", c"setVertexTexture:atIndex:", c"setVertexSamplerState:atIndex:", c"setVertexBytes:length:atIndex:"),
            _ => (c"setFragmentBuffer:offset:atIndex:", c"setFragmentTexture:atIndex:", c"setFragmentSamplerState:atIndex:", c"setFragmentBytes:length:atIndex:"),
        };
        let mut sizes = [0u32; shader::msl::FIRST_VERTEX_SLOT as usize];
        for s in &info.slots {
            let g = self.groups.get(s.set as usize).copied().flatten();
            let r = g.and_then(|g| g.0.res.iter().find(|r| r.0 == s.binding)).map(|r| r.1.clone());
            let Some(r) = r else {
                self.ok(false, "dispatch / draw: a bind group the pipeline uses is not set")?;
                return None;
            };
            // SAFETY: an open encoder of the matching kind; the objects are alive (the group holds
            // them, the command buffer retains them); the slot indices come from the translation.
            unsafe {
                match (s.kind, r) {
                    (SlotKind::Buffer, Res::Buf(b, off, range)) => {
                        let f: unsafe extern "C" fn(Id, objc::Sel, Id, usize, usize) = core::mem::transmute(objc::objc_msgSend as *const ());
                        f(e, objc::sel(buf), b.id(), off as usize, s.index as usize);
                        if let Some(x) = sizes.get_mut(s.index as usize) {
                            *x = range.min(u32::MAX as u64) as u32;
                        }
                    }
                    (SlotKind::Texture, Res::Tex(t)) => {
                        let f: unsafe extern "C" fn(Id, objc::Sel, Id, usize) = core::mem::transmute(objc::objc_msgSend as *const ());
                        f(e, objc::sel(tex), t.id(), s.index as usize);
                    }
                    (SlotKind::Sampler, Res::Smp(sm)) => {
                        let f: unsafe extern "C" fn(Id, objc::Sel, Id, usize) = core::mem::transmute(objc::objc_msgSend as *const ());
                        f(e, objc::sel(smp), sm.id(), s.index as usize);
                    }
                    _ => {
                        self.ok(false, "a bound resource's kind differs from the shader's")?;
                    }
                }
            }
        }
        let setb = |data: *const u8, n: usize, at: u32| {
            // SAFETY: setBytes copies n bytes from data at once.
            unsafe {
                let f: unsafe extern "C" fn(Id, objc::Sel, *const u8, usize, usize) = core::mem::transmute(objc::objc_msgSend as *const ());
                f(e, objc::sel(bytes), data, n, at as usize);
            }
        };
        if info.push {
            setb(self.push.as_ptr(), self.push.len(), shader::msl::PUSH_SLOT);
        }
        if info.sizes {
            setb(sizes.as_ptr() as *const u8, core::mem::size_of_val(&sizes), shader::msl::SIZES_SLOT);
        }
        Some(())
    }

    fn prepare_dispatch(&mut self) -> Option<(Id, [u32; 3])> {
        let p = self.compute_pipe?;
        let c = self.encoder(Kind::Compute)?;
        if self.dirty {
            // SAFETY: an open compute encoder; the state is alive (the pipeline is borrowed).
            unsafe { msg!(c, c"setComputePipelineState:", p.state.id() => Id; ()) };
            let info = p.stages[0].as_ref()?;
            self.bind(c, info, 0)?;
            self.dirty = false;
        }
        Some((c, p.stages[0].as_ref()?.workgroup))
    }

    pub fn dispatch(&mut self, x: u32, y: u32, z: u32) {
        let m = self.sh.limits.max_compute_workgroups;
        let ok = self.pass.is_none() && self.compute_pipe.is_some() && x <= m[0] && y <= m[1] && z <= m[2];
        if self.ok(ok, "dispatch: no compute pipeline, inside a pass or too many workgroups").is_none() {
            return;
        }
        if x == 0 || y == 0 || z == 0 {
            return;
        }
        let Some((c, wg)) = self.prepare_dispatch() else { return };
        // SAFETY: an open compute encoder with pipeline and bindings set.
        unsafe {
            msg!(c, c"dispatchThreadgroups:threadsPerThreadgroup:", MTLSize { w: x as usize, h: y as usize, d: z as usize } => MTLSize,
                 MTLSize { w: wg[0] as usize, h: wg[1] as usize, d: wg[2] as usize } => MTLSize; ());
        }
    }

    /// workgroup counts (3 x u32) from buf at offset
    pub fn dispatch_indirect(&mut self, buf: &'a Buffer, offset: u64) {
        let ok = self.pass.is_none() && self.compute_pipe.is_some() && buf.usage().contains(BufferUsage::INDIRECT) && offset % 4 == 0
            && offset + 12 <= buf.size();
        if self.ok(ok, "dispatch_indirect: pipeline, INDIRECT usage, offset or range").is_none() {
            return;
        }
        let Some((c, wg)) = self.prepare_dispatch() else { return };
        // SAFETY: as in dispatch; range checked.
        unsafe {
            msg!(c, c"dispatchThreadgroupsWithIndirectBuffer:indirectBufferOffset:threadsPerThreadgroup:", buf.0.h.id() => Id,
                 offset as usize => usize, MTLSize { w: wg[0] as usize, h: wg[1] as usize, d: wg[2] as usize } => MTLSize; ());
        }
    }

    // ------------------------------------------------------------ render passes

    pub fn begin_render(&mut self, desc: &RenderPassDesc<'_, 'a>) {
        if self.outside("begin_render inside a render pass").is_none() {
            return;
        }
        let n = desc.color.len();
        if self.ok(n <= self.sh.limits.max_color_targets as usize && (n > 0 || desc.depth.is_some()),
                   "begin_render: no attachments or too many").is_none()
        {
            return;
        }
        let first = desc.color.first().map(|c| c.view).or(desc.depth.map(|d| d.view));
        let Some(first) = first else { return };
        let (w, h) = first.size();
        let single = |v: &TextureView| v.0.mips == 1 && v.0.layers == 1 && v.size() == (w, h);
        let colors_ok = desc.color.iter().all(|c| {
            single(c.view) && c.view.usage().contains(TextureUsage::COLOR_TARGET) && c.after != Layout::Undefined
        });
        let depth_ok = desc.depth.is_none_or(|d| {
            single(d.view) && d.view.usage().contains(TextureUsage::DEPTH_TARGET) && d.after != Layout::Undefined
        });
        if self.ok(colors_ok && depth_ok, "begin_render: attachments (one mip and layer, equal sizes, usage)").is_none() {
            return;
        }
        self.end_encoder();
        let load = |l: Load| -> usize {
            match l {
                Load::Keep => 1,
                Load::Clear => 2,
                Load::Discard => 0,
            }
        };
        let mut colors = [None; COLOR_TARGETS_MAX];
        let _pool = Pool::new();
        let cb = self.cb();
        // SAFETY: a render pass descriptor (autoreleased, inside the pool) set up with live views;
        // the render encoder is retained.
        let e = unsafe {
            let rp: Id = msg!(objc::class(c"MTLRenderPassDescriptor"), c"renderPassDescriptor"; Id);
            let cas: Id = msg!(rp, c"colorAttachments"; Id);
            for (i, c) in desc.color.iter().enumerate() {
                let ca: Id = msg!(cas, c"objectAtIndexedSubscript:", i => usize; Id);
                msg!(ca, c"setTexture:", c.view.0.h.id() => Id; ());
                msg!(ca, c"setLoadAction:", load(c.load) => usize; ());
                msg!(ca, c"setStoreAction:", if c.store { 1usize } else { 0 } => usize; ());
                let cc = MTLClearColor { r: c.clear[0] as f64, g: c.clear[1] as f64, b: c.clear[2] as f64, a: c.clear[3] as f64 };
                msg!(ca, c"setClearColor:", cc => MTLClearColor; ());
                colors[i] = Some(c.view.format());
            }
            if let Some(d) = desc.depth {
                let da: Id = msg!(rp, c"depthAttachment"; Id);
                msg!(da, c"setTexture:", d.view.0.h.id() => Id; ());
                msg!(da, c"setLoadAction:", load(d.load) => usize; ());
                msg!(da, c"setStoreAction:", if d.store { 1usize } else { 0 } => usize; ());
                msg!(da, c"setClearDepth:", d.clear as f64 => f64; ());
                if d.view.format().has_stencil() {
                    let sa: Id = msg!(rp, c"stencilAttachment"; Id);
                    msg!(sa, c"setTexture:", d.view.0.h.id() => Id; ());
                    msg!(sa, c"setLoadAction:", if d.load == Load::Keep { 1usize } else { 2 } => usize; ());
                    msg!(sa, c"setStoreAction:", if d.store { 1usize } else { 0 } => usize; ());
                }
            }
            Obj::retained(msg!(cb, c"renderCommandEncoderWithDescriptor:", rp => Id; Id))
        };
        let Some(e) = e else {
            self.err.get_or_insert(GpuError::OutOfMemory);
            return;
        };
        self.enc = Some((e, Kind::Render));
        self.pass = Some(Pass { colors, n, depth: desc.depth.map(|d| d.view.format()), size: (w, h) });
        self.graphics = None;
        self.index = None;
        self.dirty = true;
    }

    pub fn end_render(&mut self) {
        if self.pass.take().is_none() {
            self.ok(false, "end_render without begin_render");
            return;
        }
        self.end_encoder();
    }

    fn render_enc(&self) -> Id {
        match &self.enc {
            Some((e, Kind::Render)) => e.id(),
            _ => objc::NIL,
        }
    }

    pub fn set_render_pipeline(&mut self, p: &'a RenderPipeline) {
        let fits = self.pass.as_ref().is_some_and(|pass| {
            pass.n == p.0.ncolor && pass.colors[..pass.n] == p.0.colors[..pass.n] && pass.depth == p.0.depth
        });
        if self.ok(fits, "set_render_pipeline: outside a pass or target formats differ").is_none() {
            return;
        }
        self.graphics = Some(&p.0);
        let e = self.render_enc();
        // SAFETY: the open render encoder; the states are alive (the pipeline is borrowed).
        unsafe {
            msg!(e, c"setRenderPipelineState:", p.0.state.id() => Id; ());
            if let Some(ds) = &p.0.depth_state {
                msg!(e, c"setDepthStencilState:", ds.id() => Id; ());
            }
            msg!(e, c"setCullMode:", p.0.cull => usize; ());
            msg!(e, c"setFrontFacingWinding:", p.0.winding => usize; ());
        }
        self.dirty = true;
    }

    /// the viewport in pixels from the top left (depth range min ..= max)
    pub fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32, min_depth: f32, max_depth: f32) {
        if self.ok(self.pass.is_some() && w > 0.0 && h > 0.0, "set_viewport: outside a pass or empty").is_none() {
            return;
        }
        let vp = MTLViewport { x: x as f64, y: y as f64, w: w as f64, h: h as f64, znear: min_depth as f64, zfar: max_depth as f64 };
        // SAFETY: the open render encoder.
        unsafe { msg!(self.render_enc(), c"setViewport:", vp => MTLViewport; ()) };
    }

    pub fn set_scissor(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if self.ok(self.pass.is_some(), "set_scissor outside a pass").is_none() {
            return;
        }
        // Metal wants the rectangle inside the target (Vulkan clips it)
        let Some((tw, th)) = self.pass.as_ref().map(|p| p.size) else { return };
        let (x0, y0) = (x.min(tw), y.min(th));
        let (x1, y1) = (x.saturating_add(w).min(tw), y.saturating_add(h).min(th));
        let r = MTLScissorRect { x: x0 as usize, y: y0 as usize, w: (x1 - x0) as usize, h: (y1 - y0) as usize };
        // SAFETY: the open render encoder.
        unsafe { msg!(self.render_enc(), c"setScissorRect:", r => MTLScissorRect; ()) };
    }

    pub fn set_vertex_buffer(&mut self, slot: u32, buf: &'a Buffer, offset: u64) {
        let ok = self.pass.is_some() && buf.usage().contains(BufferUsage::VERTEX) && offset < buf.size()
            && slot < self.sh.limits.max_vertex_buffers;
        if self.ok(ok, "set_vertex_buffer: outside a pass, VERTEX usage or offset").is_none() {
            return;
        }
        // SAFETY: the open render encoder; the vertex buffer slots lie above the resources'.
        unsafe {
            msg!(self.render_enc(), c"setVertexBuffer:offset:atIndex:", buf.0.h.id() => Id, offset as usize => usize,
                 shader::msl::vertex_slot(slot) as usize => usize; ());
        }
    }

    pub fn set_index_buffer(&mut self, buf: &'a Buffer, offset: u64, format: IndexFormat) {
        let size = if format == IndexFormat::U16 { 2 } else { 4 };
        let ok = self.pass.is_some() && buf.usage().contains(BufferUsage::INDEX) && offset % size == 0 && offset < buf.size();
        if self.ok(ok, "set_index_buffer: outside a pass, INDEX usage or offset").is_none() {
            return;
        }
        self.index = Some((buf.0.h.id(), offset, format == IndexFormat::U32));
    }

    fn can_draw(&mut self, indexed: bool) -> Option<Id> {
        let ok = self.pass.is_some() && self.graphics.is_some() && (!indexed || self.index.is_some());
        self.ok(ok, "draw: outside a pass, without a render pipeline or index buffer")?;
        let e = self.render_enc();
        if self.dirty {
            let p = self.graphics?;
            if let Some(v) = &p.stages[0] {
                self.bind(e, v, 1)?;
            }
            if let Some(f) = &p.stages[1] {
                self.bind(e, f, 2)?;
            }
            self.dirty = false;
        }
        Some(e)
    }

    fn prim(&self) -> usize {
        self.graphics.map_or(3, |p| p.prim)
    }

    pub fn draw(&mut self, vertices: u32, instances: u32, first_vertex: u32, first_instance: u32) {
        let Some(e) = self.can_draw(false) else { return };
        if vertices == 0 || instances == 0 {
            return;
        }
        // SAFETY: the open render encoder with pipeline and bindings set.
        unsafe {
            msg!(e, c"drawPrimitives:vertexStart:vertexCount:instanceCount:baseInstance:", self.prim() => usize,
                 first_vertex as usize => usize, vertices as usize => usize, instances as usize => usize, first_instance as usize => usize; ());
        }
    }

    pub fn draw_indexed(&mut self, indices: u32, instances: u32, first_index: u32, base_vertex: i32, first_instance: u32) {
        let Some(e) = self.can_draw(true) else { return };
        if indices == 0 || instances == 0 {
            return;
        }
        let Some((ib, off, wide)) = self.index else { return };
        let size = if wide { 4 } else { 2 };
        // SAFETY: as in draw, with an index buffer.
        unsafe {
            msg!(e, c"drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:instanceCount:baseVertex:baseInstance:",
                 self.prim() => usize, indices as usize => usize, wide as usize => usize, ib => Id,
                 (off + first_index as u64 * size) as usize => usize, instances as usize => usize, base_vertex as isize => isize,
                 first_instance as usize => usize; ());
        }
    }

    fn indirect_ok(&mut self, buf: &Buffer, offset: u64, bytes: u64) -> Option<()> {
        let ok = buf.usage().contains(BufferUsage::INDIRECT) && offset % 4 == 0 && offset.checked_add(bytes).is_some_and(|e| e <= buf.size());
        self.ok(ok, "indirect: INDIRECT usage, offset (multiple of 4) or range")
    }

    /// one draw: vertex count, instance count, first vertex, first instance (4 x u32)
    pub fn draw_indirect(&mut self, buf: &'a Buffer, offset: u64) {
        if self.indirect_ok(buf, offset, 16).is_none() {
            return;
        }
        let Some(e) = self.can_draw(false) else { return };
        // SAFETY: as in draw; range checked (MTLDrawPrimitivesIndirectArguments is Vulkan's layout).
        unsafe {
            msg!(e, c"drawPrimitives:indirectBuffer:indirectBufferOffset:", self.prim() => usize, buf.0.h.id() => Id, offset as usize => usize; ());
        }
    }

    /// one indexed draw: index count, instance count, first index, base vertex (i32), first instance
    pub fn draw_indexed_indirect(&mut self, buf: &'a Buffer, offset: u64) {
        if self.indirect_ok(buf, offset, 20).is_none() {
            return;
        }
        let Some(e) = self.can_draw(true) else { return };
        let Some((ib, off, wide)) = self.index else { return };
        // SAFETY: as above (MTLDrawIndexedPrimitivesIndirectArguments is Vulkan's layout).
        unsafe {
            msg!(e, c"drawIndexedPrimitives:indexType:indexBuffer:indexBufferOffset:indirectBuffer:indirectBufferOffset:", self.prim() => usize,
                 wide as usize => usize, ib => Id, off as usize => usize, buf.0.h.id() => Id, offset as usize => usize; ());
        }
    }

    /// several indexed draws from one buffer (Features::multi_draw_indirect: not at Metal's level Basis)
    pub fn draw_indexed_indirect_multi(&mut self, _buf: &'a Buffer, _offset: u64, _count: u32, _stride: u32) {
        self.ok(false, "draw_indexed_indirect_multi: needs multi_draw_indirect, stride >= 20");
    }

    /// (level Voll) not on Metal yet
    pub fn draw_indexed_indirect_count(&mut self, _buf: &'a Buffer, _offset: u64, _count_buf: &'a Buffer, _count_offset: u64,
                                       _max_count: u32, _stride: u32) {
        self.ok(false, "draw_indexed_indirect_count: needs level Voll, stride >= 20");
    }

    // ------------------------------------------------------------ timestamps

    pub fn reset_timestamps(&mut self, _ts: &'a Timestamps, _first: u32, _n: u32) {
        self.ok(false, "reset_timestamps: inside a pass or range");
    }

    pub fn timestamp(&mut self, _ts: &'a Timestamps, _i: u32) {
        self.ok(false, "timestamp: index outside the set, or no timestamps on this queue");
    }

    // ------------------------------------------------------------ end

    /// the recorded commands, or the first misuse
    pub fn finish(mut self) -> Result<CommandBuffer<'a>> {
        if self.pass.is_some() {
            self.ok(false, "finish inside a render pass");
        }
        self.end_encoder();
        if let Some(e) = self.err {
            return Err(e);
        }
        let cb = self.cb.take().ok_or(GpuError::Invalid("finish twice"))?;
        Ok(CommandBuffer { cb, _borrow: PhantomData })
    }
}
