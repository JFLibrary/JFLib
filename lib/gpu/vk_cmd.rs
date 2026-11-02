// vk_cmd -- command recording: CommandEncoder -> CommandBuffer
//
//   device.encoder() -> Result<CommandEncoder>      a command buffer slot, begun
//   copies (outside render passes): copy_buffer, fill_buffer, copy_buffer_to_texture (the
//        region in Layout::CopyDst), copy_texture_to_buffer (Layout::CopySrc)
//   barrier(before: Access, after: Access)          a global memory barrier
//   texture_barrier(tex, before, after) / texture_barrier_range(.., mips, layers, ..)
//   set_compute_pipeline, set_bind_group(g, &group), push_constants(off, &[T]),
//        dispatch(x, y, z), dispatch_indirect(&buf, off)
//   begin_render(&RenderPassDesc) .. end_render()   dynamic rendering; the attachments go from
//        `before` to ColorTarget / DepthTarget at the start and to `after` at the end; viewport
//        and scissor = the whole target
//   set_render_pipeline, set_viewport, set_scissor, set_vertex_buffer, set_index_buffer,
//        draw, draw_indexed, draw_indirect, draw_indexed_indirect (Basis), and on Voll
//        draw_indexed_indirect_multi and draw_indexed_indirect_count
//   reset_timestamps(&ts, first, n) (outside passes), timestamp(&ts, i)
//   finish() -> Result<CommandBuffer>               the first misuse recorded, if any
//
// Bind groups and push constants go to the pipeline set last: the compute pipeline outside
// render passes, the render pipeline inside (set the pipeline first).  Every command checks
// what the backend can know cheaply (the pipeline and pass state, bind group / pipeline layout
// compatibility, target formats, buffer usages, ranges and alignments, the Voll features); a
// failed check skips the command and finish() returns it as GpuError::Invalid.
//
// Stages and accesses per Layout (texture_barrier, attachments, write_texture), as (source;
// destination): Undefined (colour output + copy + compute, no access -- the stages the
// swapchain acquisition waits at; not a destination), General (all commands, memory read /
// write), CopySrc / CopyDst (transfer), Sampled (vertex + fragment + compute shaders), ColorTarget
// (colour output), DepthTarget (early + late fragment tests), DepthRead (fragment tests and
// shaders), Present (as Undefined; bottom of pipe).
// Viewport: y flipped (origin top left, NDC y up: lib.rs conventions).

use alloc::vec::Vec;
use core::marker::PhantomData;

use crate::vk_dev::{Device, Shared, Slot};
use crate::vk_ffi as vk;
use crate::vk_res::{PipeInner, copy_region, subresources};
use crate::*;

/// what an image handed to the window is in: Linux has no VK_KHR_swapchain, vk_dmabuf.rs hands
/// dma-bufs over in GENERAL
const LAYOUT_PRESENT: u32 = if cfg!(jfgui_glibc) { vk::IMAGE_LAYOUT_GENERAL } else { vk::IMAGE_LAYOUT_PRESENT_SRC_KHR };

/// (vk layout, stages, access) of a layout as the source of a barrier
fn src_of(l: Layout) -> (u32, u32, u32) {
    let shaders = vk::PIPELINE_STAGE_VERTEX_SHADER_BIT | vk::PIPELINE_STAGE_FRAGMENT_SHADER_BIT | vk::PIPELINE_STAGE_COMPUTE_SHADER_BIT;
    let tests = vk::PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | vk::PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT;
    let acquire = vk::PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT | vk::PIPELINE_STAGE_TRANSFER_BIT | vk::PIPELINE_STAGE_COMPUTE_SHADER_BIT;
    match l {
        Layout::Undefined => (vk::IMAGE_LAYOUT_UNDEFINED, acquire, 0),
        Layout::General => (vk::IMAGE_LAYOUT_GENERAL, vk::PIPELINE_STAGE_ALL_COMMANDS_BIT, vk::ACCESS_MEMORY_WRITE_BIT),
        Layout::CopySrc => (vk::IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, vk::PIPELINE_STAGE_TRANSFER_BIT, 0),
        Layout::CopyDst => (vk::IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, vk::PIPELINE_STAGE_TRANSFER_BIT, vk::ACCESS_TRANSFER_WRITE_BIT),
        Layout::Sampled => (vk::IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, shaders, 0),
        Layout::ColorTarget => (vk::IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, vk::PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                                vk::ACCESS_COLOR_ATTACHMENT_WRITE_BIT),
        Layout::DepthTarget => (vk::IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL, tests, vk::ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT),
        Layout::DepthRead => (vk::IMAGE_LAYOUT_DEPTH_STENCIL_READ_ONLY_OPTIMAL, tests | shaders, 0),
        Layout::Present => (LAYOUT_PRESENT, acquire, 0),
    }
}

/// (vk layout, stages, access) of a layout as the destination of a barrier
fn dst_of(l: Layout) -> (u32, u32, u32) {
    let shaders = vk::PIPELINE_STAGE_VERTEX_SHADER_BIT | vk::PIPELINE_STAGE_FRAGMENT_SHADER_BIT | vk::PIPELINE_STAGE_COMPUTE_SHADER_BIT;
    let tests = vk::PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | vk::PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT;
    match l {
        Layout::Undefined => (vk::IMAGE_LAYOUT_UNDEFINED, vk::PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, 0),
        Layout::General => (vk::IMAGE_LAYOUT_GENERAL, vk::PIPELINE_STAGE_ALL_COMMANDS_BIT,
                            vk::ACCESS_MEMORY_READ_BIT | vk::ACCESS_MEMORY_WRITE_BIT),
        Layout::CopySrc => (vk::IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, vk::PIPELINE_STAGE_TRANSFER_BIT, vk::ACCESS_TRANSFER_READ_BIT),
        Layout::CopyDst => (vk::IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, vk::PIPELINE_STAGE_TRANSFER_BIT, vk::ACCESS_TRANSFER_WRITE_BIT),
        Layout::Sampled => (vk::IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, shaders, vk::ACCESS_SHADER_READ_BIT),
        Layout::ColorTarget => (vk::IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, vk::PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                                vk::ACCESS_COLOR_ATTACHMENT_READ_BIT | vk::ACCESS_COLOR_ATTACHMENT_WRITE_BIT),
        Layout::DepthTarget => (vk::IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL, tests,
                                vk::ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT | vk::ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT),
        Layout::DepthRead => (vk::IMAGE_LAYOUT_DEPTH_STENCIL_READ_ONLY_OPTIMAL, tests | shaders,
                              vk::ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT | vk::ACCESS_SHADER_READ_BIT),
        Layout::Present => (LAYOUT_PRESENT, vk::PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, 0),
    }
}

fn image_barrier_info(image: vk::Handle, range: vk::ImageSubresourceRange, before: Layout, after: Layout)
                      -> (vk::ImageMemoryBarrier, u32, u32) {
    let (ol, ss, sa) = src_of(before);
    let (nl, ds, da) = dst_of(after);
    let b = vk::ImageMemoryBarrier {
        s_type: vk::STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
        src_access_mask: sa,
        dst_access_mask: da,
        old_layout: ol,
        new_layout: nl,
        src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
        dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
        image,
        subresource_range: range,
        ..vk::Zero::zero()
    };
    (b, ss, ds)
}

/// record one image layout transition
///
/// # Safety
/// cb is recording outside a render pass; image belongs to f's device.
pub(crate) unsafe fn image_barrier(f: &vk::DeviceFns, cb: vk::CommandBuffer, image: vk::Handle, range: vk::ImageSubresourceRange,
                                   before: Layout, after: Layout) {
    let (b, ss, ds) = image_barrier_info(image, range, before, after);
    // SAFETY: the caller's contract.
    unsafe { (f.cmd_pipeline_barrier)(cb, ss, ds, 0, 0, core::ptr::null(), 0, core::ptr::null(), 1, &b) };
}

/// (stages, source access, destination access) of Access flags
fn access_of(a: Access) -> (u32, u32, u32) {
    let table: [(Access, u32, u32, u32); 12] = [
        (Access::COPY_READ, vk::PIPELINE_STAGE_TRANSFER_BIT, 0, vk::ACCESS_TRANSFER_READ_BIT),
        (Access::COPY_WRITE, vk::PIPELINE_STAGE_TRANSFER_BIT, vk::ACCESS_TRANSFER_WRITE_BIT, vk::ACCESS_TRANSFER_WRITE_BIT),
        (Access::COMPUTE_READ, vk::PIPELINE_STAGE_COMPUTE_SHADER_BIT, 0, vk::ACCESS_SHADER_READ_BIT | vk::ACCESS_UNIFORM_READ_BIT),
        (Access::COMPUTE_WRITE, vk::PIPELINE_STAGE_COMPUTE_SHADER_BIT, vk::ACCESS_SHADER_WRITE_BIT,
         vk::ACCESS_SHADER_READ_BIT | vk::ACCESS_SHADER_WRITE_BIT),
        (Access::GRAPHICS_READ, vk::PIPELINE_STAGE_VERTEX_SHADER_BIT | vk::PIPELINE_STAGE_FRAGMENT_SHADER_BIT, 0,
         vk::ACCESS_SHADER_READ_BIT | vk::ACCESS_UNIFORM_READ_BIT),
        (Access::GRAPHICS_WRITE, vk::PIPELINE_STAGE_VERTEX_SHADER_BIT | vk::PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
         vk::ACCESS_SHADER_WRITE_BIT, vk::ACCESS_SHADER_READ_BIT | vk::ACCESS_SHADER_WRITE_BIT),
        (Access::INDIRECT, vk::PIPELINE_STAGE_DRAW_INDIRECT_BIT, 0, vk::ACCESS_INDIRECT_COMMAND_READ_BIT),
        (Access::INDEX_VERTEX, vk::PIPELINE_STAGE_VERTEX_INPUT_BIT, 0,
         vk::ACCESS_INDEX_READ_BIT | vk::ACCESS_VERTEX_ATTRIBUTE_READ_BIT),
        (Access::COLOR_TARGET, vk::PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, vk::ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
         vk::ACCESS_COLOR_ATTACHMENT_READ_BIT | vk::ACCESS_COLOR_ATTACHMENT_WRITE_BIT),
        (Access::DEPTH_TARGET, vk::PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | vk::PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
         vk::ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
         vk::ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT | vk::ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT),
        (Access::HOST_READ, vk::PIPELINE_STAGE_HOST_BIT, 0, vk::ACCESS_HOST_READ_BIT),
        (Access::HOST_WRITE, vk::PIPELINE_STAGE_HOST_BIT, vk::ACCESS_HOST_WRITE_BIT, vk::ACCESS_HOST_WRITE_BIT),
    ];
    let mut out = (0, 0, 0);
    for (flag, st, sa, da) in table {
        if a.intersects(flag) {
            out.0 |= st;
            out.1 |= sa;
            out.2 |= da;
        }
    }
    out
}

/// an attachment's transition at the end of a pass
struct EndBarrier {
    image: vk::Handle,
    range: vk::ImageSubresourceRange,
    from: Layout,
    to: Layout,
}

struct Pass {
    colors: [u32; COLOR_TARGETS_MAX],
    n: usize,
    depth: u32,
    end: Vec<EndBarrier>,
}

/// records commands; finish() makes a CommandBuffer for Queue::submit
pub struct CommandEncoder<'a> {
    sh: &'a Shared,
    slot: Option<Slot>,
    cb: vk::CommandBuffer,
    err: Option<GpuError>,
    pass: Option<Pass>,
    compute: Option<&'a PipeInner>,
    graphics: Option<&'a PipeInner>,
    index: bool,
    _borrow: PhantomData<&'a ()>,
}

/// recorded commands, ready to submit (dropped unsubmitted: discarded)
pub struct CommandBuffer<'a> {
    sh: &'a Shared,
    slot: Option<Slot>,
}

impl CommandBuffer<'_> {
    pub(crate) fn into_slot(mut self) -> Slot {
        self.slot.take().unwrap_or_else(|| unreachable!())
    }
}

impl Drop for CommandBuffer<'_> {
    fn drop(&mut self) {
        if let Some(s) = self.slot.take() {
            self.sh.give_slot(s);
        }
    }
}

impl Drop for CommandEncoder<'_> {
    fn drop(&mut self) {
        if let Some(s) = self.slot.take() {
            // the buffer is recording (maybe inside a pass) and never submitted: resetting its
            // pool returns it to the initial state
            self.sh.give_slot(s);
        }
    }
}

impl Device {
    /// a command encoder (a recording command buffer)
    pub fn encoder(&self) -> Result<CommandEncoder<'_>> {
        let slot = self.sh.take_slot()?;
        Ok(CommandEncoder {
            sh: &self.sh,
            cb: slot.cb,
            slot: Some(slot),
            err: None,
            pass: None,
            compute: None,
            graphics: None,
            index: false,
            _borrow: PhantomData,
        })
    }
}

impl<'a> CommandEncoder<'a> {
    /// record the first misuse; Some(()) when the command may be recorded
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

    // ------------------------------------------------------------ copies

    pub fn copy_buffer(&mut self, src: &'a Buffer, src_off: u64, dst: &'a Buffer, dst_off: u64, size: u64) {
        if self.outside("copy_buffer inside a render pass").is_none()
            || self.buf_range(src, src_off, size, "copy_buffer: source range").is_none()
            || self.buf_range(dst, dst_off, size, "copy_buffer: destination range").is_none()
            || size == 0
        {
            return;
        }
        let r = vk::BufferCopy { src_offset: src_off, dst_offset: dst_off, size };
        // SAFETY: recording; ranges checked.
        unsafe { (self.sh.f.cmd_copy_buffer)(self.cb, src.handle(), dst.handle(), 1, &r) };
    }

    /// fill size bytes at offset with a 32-bit value (both multiples of 4)
    pub fn fill_buffer(&mut self, buf: &'a Buffer, offset: u64, size: u64, value: u32) {
        if self.outside("fill_buffer inside a render pass").is_none()
            || self.ok(offset % 4 == 0 && size % 4 == 0 && size > 0, "fill_buffer: offset and size multiples of 4").is_none()
            || self.buf_range(buf, offset, size, "fill_buffer: range").is_none()
        {
            return;
        }
        // SAFETY: recording; range checked.
        unsafe { (self.sh.f.cmd_fill_buffer)(self.cb, buf.handle(), offset, size, value) };
    }

    /// buffer data -> a texture region (the region in Layout::CopyDst)
    pub fn copy_buffer_to_texture(&mut self, src: &'a Buffer, layout: DataLayout, dst: TextureRegion<'a>) {
        if self.outside("copy inside a render pass").is_none() {
            return;
        }
        let c = copy_region(&dst, &layout, src.size());
        let ok = c.is_ok() && dst.texture.usage().contains(TextureUsage::COPY_DST);
        if self.ok(ok, "copy_buffer_to_texture: region, data layout or COPY_DST usage").is_none() {
            return;
        }
        let Ok((r, _)) = c else { return };
        // SAFETY: recording; region checked against texture and buffer.
        unsafe {
            (self.sh.f.cmd_copy_buffer_to_image)(self.cb, src.handle(), dst.texture.image(),
                vk::IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &r)
        };
    }

    /// a texture region -> buffer data (the region in Layout::CopySrc)
    pub fn copy_texture_to_buffer(&mut self, src: TextureRegion<'a>, dst: &'a Buffer, layout: DataLayout) {
        if self.outside("copy inside a render pass").is_none() {
            return;
        }
        let c = copy_region(&src, &layout, dst.size());
        let ok = c.is_ok() && src.texture.usage().contains(TextureUsage::COPY_SRC);
        if self.ok(ok, "copy_texture_to_buffer: region, data layout or COPY_SRC usage").is_none() {
            return;
        }
        let Ok((r, _)) = c else { return };
        // SAFETY: as above.
        unsafe {
            (self.sh.f.cmd_copy_image_to_buffer)(self.cb, src.texture.image(), vk::IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                dst.handle(), 1, &r)
        };
    }

    // ------------------------------------------------------------ barriers

    /// the accesses `before` finish (and their writes become visible) before the `after` ones
    pub fn barrier(&mut self, before: Access, after: Access) {
        if self.outside("barrier inside a render pass").is_none() {
            return;
        }
        let (ss, sa, _) = access_of(before);
        let (ds, _, da) = access_of(after);
        let mb = vk::MemoryBarrier { s_type: vk::STRUCTURE_TYPE_MEMORY_BARRIER, src_access_mask: sa, dst_access_mask: da,
                                     ..vk::Zero::zero() };
        let ss = if ss == 0 { vk::PIPELINE_STAGE_TOP_OF_PIPE_BIT } else { ss };
        let ds = if ds == 0 { vk::PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT } else { ds };
        // SAFETY: recording outside a pass.
        unsafe { (self.sh.f.cmd_pipeline_barrier)(self.cb, ss, ds, 0, 1, &mb, 0, core::ptr::null(), 0, core::ptr::null()) };
    }

    /// every mip and layer of tex from `before` to `after`
    pub fn texture_barrier(&mut self, tex: &'a Texture, before: Layout, after: Layout) {
        self.texture_barrier_range(tex, 0, tex.mips(), 0, tex.layers(), before, after);
    }

    /// mips base_mip .. +mips and layers base_layer .. +layers of tex from `before` to `after`
    #[allow(clippy::too_many_arguments)]
    pub fn texture_barrier_range(&mut self, tex: &'a Texture, base_mip: u32, mips: u32, base_layer: u32, layers: u32,
                                 before: Layout, after: Layout) {
        if self.outside("texture_barrier inside a render pass").is_none()
            || self.ok(mips > 0 && layers > 0 && base_mip + mips <= tex.mips() && base_layer + layers <= tex.layers(),
                       "texture_barrier: range outside the texture").is_none()
            || self.ok(after != Layout::Undefined, "texture_barrier: Undefined is no destination").is_none()
        {
            return;
        }
        let range = subresources(tex, base_mip, mips, base_layer, layers);
        // SAFETY: recording outside a pass.
        unsafe { image_barrier(&self.sh.f, self.cb, tex.image(), range, before, after) };
    }

    // ------------------------------------------------------------ compute

    pub fn set_compute_pipeline(&mut self, p: &'a ComputePipeline) {
        if self.outside("set_compute_pipeline inside a render pass").is_none() {
            return;
        }
        self.compute = Some(&p.0);
        // SAFETY: recording; the pipeline belongs to this device.
        unsafe { (self.sh.f.cmd_bind_pipeline)(self.cb, vk::PIPELINE_BIND_POINT_COMPUTE, p.0.h) };
    }

    /// the pipeline bind groups and push constants go to: the render pipeline inside a pass,
    /// else the compute pipeline
    fn current(&mut self) -> Option<(&'a PipeInner, u32)> {
        let (p, bp) = if self.pass.is_some() {
            (self.graphics, vk::PIPELINE_BIND_POINT_GRAPHICS)
        } else {
            (self.compute, vk::PIPELINE_BIND_POINT_COMPUTE)
        };
        self.ok(p.is_some(), "bind group / push constants before a pipeline")?;
        p.map(|p| (p, bp))
    }

    /// bind group g (= @group(g)) of the current pipeline's layout
    pub fn set_bind_group(&mut self, index: u32, group: &'a BindGroup) {
        let Some((p, bp)) = self.current() else { return };
        let sets = &p.layout.0.sets;
        let fits = sets.get(index as usize).is_some_and(|&l| l == group.0.layout);
        if self.ok(fits, "set_bind_group: the group's layout differs from the pipeline layout's").is_none() {
            return;
        }
        // SAFETY: recording; set and layout are compatible (checked).
        unsafe {
            (self.sh.f.cmd_bind_descriptor_sets)(self.cb, bp, p.layout.0.h, index, 1, &group.0.set, 0, core::ptr::null())
        };
    }

    /// push constants at offset (bytes) for the current pipeline
    pub fn push_constants<T: Pod>(&mut self, offset: u32, data: &[T]) {
        let Some((p, _)) = self.current() else { return };
        let bytes = bytes_of(data);
        let n = bytes.len() as u32;
        let fits = offset % 4 == 0 && n % 4 == 0 && n > 0 && offset + n <= p.layout.0.push;
        if self.ok(fits, "push_constants: range outside the layout's (multiples of 4)").is_none() {
            return;
        }
        // SAFETY: recording; the range lies within the layout's push-constant range.
        unsafe {
            (self.sh.f.cmd_push_constants)(self.cb, p.layout.0.h, vk::SHADER_STAGE_ALL, offset, n,
                bytes.as_ptr() as *const core::ffi::c_void)
        };
    }

    pub fn dispatch(&mut self, x: u32, y: u32, z: u32) {
        let m = self.sh.limits.max_compute_workgroups;
        let ok = self.pass.is_none() && self.compute.is_some() && x <= m[0] && y <= m[1] && z <= m[2];
        if self.ok(ok, "dispatch: no compute pipeline, inside a pass or too many workgroups").is_none() {
            return;
        }
        // SAFETY: recording with a compute pipeline bound.
        unsafe { (self.sh.f.cmd_dispatch)(self.cb, x, y, z) };
    }

    /// workgroup counts (3 x u32) from buf at offset
    pub fn dispatch_indirect(&mut self, buf: &'a Buffer, offset: u64) {
        let ok = self.pass.is_none() && self.compute.is_some() && buf.usage().contains(BufferUsage::INDIRECT) && offset % 4 == 0
            && offset + 12 <= buf.size();
        if self.ok(ok, "dispatch_indirect: pipeline, INDIRECT usage, offset or range").is_none() {
            return;
        }
        // SAFETY: as above; range checked.
        unsafe { (self.sh.f.cmd_dispatch_indirect)(self.cb, buf.handle(), offset) };
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
        let mut barriers: Vec<vk::ImageMemoryBarrier> = Vec::with_capacity(n + 1);
        let (mut ss, mut ds) = (0, 0);
        let mut end = Vec::new();
        let mut atts: Vec<vk::RenderingAttachmentInfo> = Vec::with_capacity(n);
        let mut colors = [0u32; COLOR_TARGETS_MAX];
        let load = |l: Load| match l {
            Load::Keep => vk::ATTACHMENT_LOAD_OP_LOAD,
            Load::Clear => vk::ATTACHMENT_LOAD_OP_CLEAR,
            Load::Discard => vk::ATTACHMENT_LOAD_OP_DONT_CARE,
        };
        let store = |s: bool| if s { vk::ATTACHMENT_STORE_OP_STORE } else { vk::ATTACHMENT_STORE_OP_DONT_CARE };
        for (i, c) in desc.color.iter().enumerate() {
            let v = &c.view.0;
            let range = vk::ImageSubresourceRange { aspect_mask: v.tex.aspect_all, base_mip_level: v.base_mip, level_count: 1,
                                                    base_array_layer: v.base_layer, layer_count: 1 };
            let (b, s, d) = image_barrier_info(v.tex.image, range, c.before, Layout::ColorTarget);
            barriers.push(b);
            ss |= s;
            ds |= d;
            if c.after != Layout::ColorTarget {
                end.push(EndBarrier { image: v.tex.image, range, from: Layout::ColorTarget, to: c.after });
            }
            colors[i] = crate::vk_res::vk_format(c.view.format());
            atts.push(vk::RenderingAttachmentInfo {
                s_type: vk::STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO,
                image_view: v.h,
                image_layout: vk::IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                load_op: load(c.load),
                store_op: store(c.store),
                clear_value: c.clear.map(f32::to_bits),
                ..vk::Zero::zero()
            });
        }
        let mut datt: vk::RenderingAttachmentInfo = vk::Zero::zero();
        let mut depth = vk::FORMAT_UNDEFINED;
        if let Some(d) = desc.depth {
            let v = &d.view.0;
            let range = vk::ImageSubresourceRange { aspect_mask: v.tex.aspect_all, base_mip_level: v.base_mip, level_count: 1,
                                                    base_array_layer: v.base_layer, layer_count: 1 };
            let (b, s, dd) = image_barrier_info(v.tex.image, range, d.before, Layout::DepthTarget);
            barriers.push(b);
            ss |= s;
            ds |= dd;
            if d.after != Layout::DepthTarget {
                end.push(EndBarrier { image: v.tex.image, range, from: Layout::DepthTarget, to: d.after });
            }
            depth = crate::vk_res::vk_format(d.view.format());
            datt = vk::RenderingAttachmentInfo {
                s_type: vk::STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO,
                image_view: v.h,
                image_layout: vk::IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
                load_op: load(d.load),
                store_op: store(d.store),
                clear_value: [d.clear.to_bits(), 0, 0, 0],
                ..vk::Zero::zero()
            };
        }
        let has_stencil = desc.depth.is_some_and(|d| d.view.format().has_stencil());
        let ri = vk::RenderingInfo {
            s_type: vk::STRUCTURE_TYPE_RENDERING_INFO,
            render_area: vk::Rect2D { offset: vk::Offset2D { x: 0, y: 0 }, extent: vk::Extent2D { width: w, height: h } },
            layer_count: 1,
            color_attachment_count: n as u32,
            p_color_attachments: atts.as_ptr(),
            p_depth_attachment: if desc.depth.is_some() { &datt } else { core::ptr::null() },
            p_stencil_attachment: if has_stencil { &datt } else { core::ptr::null() },
            ..vk::Zero::zero()
        };
        let vp = vk::Viewport { x: 0.0, y: h as f32, width: w as f32, height: -(h as f32), min_depth: 0.0, max_depth: 1.0 };
        let sc = vk::Rect2D { offset: vk::Offset2D { x: 0, y: 0 }, extent: vk::Extent2D { width: w, height: h } };
        // SAFETY: recording outside a pass; the attachments were checked; the structures live
        // until the calls return.
        unsafe {
            (self.sh.f.cmd_pipeline_barrier)(self.cb, ss, ds, 0, 0, core::ptr::null(), 0, core::ptr::null(),
                barriers.len() as u32, barriers.as_ptr());
            (self.sh.f.cmd_begin_rendering)(self.cb, &ri);
            (self.sh.f.cmd_set_viewport)(self.cb, 0, 1, &vp);
            (self.sh.f.cmd_set_scissor)(self.cb, 0, 1, &sc);
        }
        self.pass = Some(Pass { colors, n, depth, end });
        self.graphics = None;
        self.index = false;
    }

    pub fn end_render(&mut self) {
        let Some(pass) = self.pass.take() else {
            self.ok(false, "end_render without begin_render");
            return;
        };
        // SAFETY: recording inside the pass begun by begin_render.
        unsafe { (self.sh.f.cmd_end_rendering)(self.cb) };
        if pass.end.is_empty() {
            return;
        }
        let (mut ss, mut ds) = (0, 0);
        let barriers: Vec<vk::ImageMemoryBarrier> = pass
            .end
            .iter()
            .map(|e| {
                let (b, s, d) = image_barrier_info(e.image, e.range, e.from, e.to);
                ss |= s;
                ds |= d;
                b
            })
            .collect();
        // SAFETY: recording outside a pass.
        unsafe {
            (self.sh.f.cmd_pipeline_barrier)(self.cb, ss, ds, 0, 0, core::ptr::null(), 0, core::ptr::null(),
                barriers.len() as u32, barriers.as_ptr())
        };
    }

    pub fn set_render_pipeline(&mut self, p: &'a RenderPipeline) {
        let fits = self.pass.as_ref().is_some_and(|pass| {
            pass.n == p.0.ncolor && pass.colors[..pass.n] == p.0.colors[..pass.n] && pass.depth == p.0.depth
        });
        if self.ok(fits, "set_render_pipeline: outside a pass or target formats differ").is_none() {
            return;
        }
        self.graphics = Some(&p.0);
        // SAFETY: recording inside a pass.
        unsafe { (self.sh.f.cmd_bind_pipeline)(self.cb, vk::PIPELINE_BIND_POINT_GRAPHICS, p.0.h) };
    }

    /// the viewport in pixels from the top left (depth range min ..= max)
    pub fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32, min_depth: f32, max_depth: f32) {
        if self.ok(self.pass.is_some() && w > 0.0 && h > 0.0, "set_viewport: outside a pass or empty").is_none() {
            return;
        }
        let vp = vk::Viewport { x, y: y + h, width: w, height: -h, min_depth, max_depth };
        // SAFETY: recording inside a pass.
        unsafe { (self.sh.f.cmd_set_viewport)(self.cb, 0, 1, &vp) };
    }

    pub fn set_scissor(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if self.ok(self.pass.is_some(), "set_scissor outside a pass").is_none() {
            return;
        }
        let r = vk::Rect2D { offset: vk::Offset2D { x: x as i32, y: y as i32 }, extent: vk::Extent2D { width: w, height: h } };
        // SAFETY: recording inside a pass.
        unsafe { (self.sh.f.cmd_set_scissor)(self.cb, 0, 1, &r) };
    }

    pub fn set_vertex_buffer(&mut self, slot: u32, buf: &'a Buffer, offset: u64) {
        let ok = self.pass.is_some() && buf.usage().contains(BufferUsage::VERTEX) && offset < buf.size()
            && slot < self.sh.limits.max_vertex_buffers;
        if self.ok(ok, "set_vertex_buffer: outside a pass, VERTEX usage or offset").is_none() {
            return;
        }
        // SAFETY: recording inside a pass.
        unsafe { (self.sh.f.cmd_bind_vertex_buffers)(self.cb, slot, 1, &buf.handle(), &offset) };
    }

    pub fn set_index_buffer(&mut self, buf: &'a Buffer, offset: u64, format: IndexFormat) {
        let size = if format == IndexFormat::U16 { 2 } else { 4 };
        let ok = self.pass.is_some() && buf.usage().contains(BufferUsage::INDEX) && offset % size == 0 && offset < buf.size();
        if self.ok(ok, "set_index_buffer: outside a pass, INDEX usage or offset").is_none() {
            return;
        }
        let t = if format == IndexFormat::U16 { vk::INDEX_TYPE_UINT16 } else { vk::INDEX_TYPE_UINT32 };
        // SAFETY: recording inside a pass.
        unsafe { (self.sh.f.cmd_bind_index_buffer)(self.cb, buf.handle(), offset, t) };
        self.index = true;
    }

    #[inline]
    fn can_draw(&mut self, indexed: bool) -> Option<()> {
        let ok = self.pass.is_some() && self.graphics.is_some() && (!indexed || self.index);
        self.ok(ok, "draw: outside a pass, without a render pipeline or index buffer")
    }

    pub fn draw(&mut self, vertices: u32, instances: u32, first_vertex: u32, first_instance: u32) {
        if self.can_draw(false).is_some() {
            // SAFETY: recording with a render pipeline inside a pass.
            unsafe { (self.sh.f.cmd_draw)(self.cb, vertices, instances, first_vertex, first_instance) };
        }
    }

    pub fn draw_indexed(&mut self, indices: u32, instances: u32, first_index: u32, base_vertex: i32, first_instance: u32) {
        if self.can_draw(true).is_some() {
            // SAFETY: as above, with an index buffer.
            unsafe { (self.sh.f.cmd_draw_indexed)(self.cb, indices, instances, first_index, base_vertex, first_instance) };
        }
    }

    fn indirect_ok(&mut self, buf: &Buffer, offset: u64, bytes: u64) -> Option<()> {
        let ok = buf.usage().contains(BufferUsage::INDIRECT) && offset % 4 == 0 && offset.checked_add(bytes).is_some_and(|e| e <= buf.size());
        self.ok(ok, "indirect: INDIRECT usage, offset (multiple of 4) or range")
    }

    /// one draw: vertex count, instance count, first vertex, first instance (4 x u32)
    pub fn draw_indirect(&mut self, buf: &'a Buffer, offset: u64) {
        if self.can_draw(false).is_some() && self.indirect_ok(buf, offset, 16).is_some() {
            // SAFETY: recording; range checked.
            unsafe { (self.sh.f.cmd_draw_indirect)(self.cb, buf.handle(), offset, 1, 16) };
        }
    }

    /// one indexed draw: index count, instance count, first index, base vertex (i32), first
    /// instance (5 x 32 bits)
    pub fn draw_indexed_indirect(&mut self, buf: &'a Buffer, offset: u64) {
        if self.can_draw(true).is_some() && self.indirect_ok(buf, offset, 20).is_some() {
            // SAFETY: recording; range checked.
            unsafe { (self.sh.f.cmd_draw_indexed_indirect)(self.cb, buf.handle(), offset, 1, 20) };
        }
    }

    /// count indexed draws, stride bytes apart (Features::multi_draw_indirect)
    pub fn draw_indexed_indirect_multi(&mut self, buf: &'a Buffer, offset: u64, count: u32, stride: u32) {
        let ok = self.sh.features.multi_draw_indirect && stride >= 20 && stride % 4 == 0 && count > 0
            && count <= self.sh.limits.max_draw_indirect_count;
        if self.ok(ok, "draw_indexed_indirect_multi: needs multi_draw_indirect, stride >= 20").is_none()
            || self.can_draw(true).is_none()
            || self.indirect_ok(buf, offset, (count as u64 - 1) * stride as u64 + 20).is_none()
        {
            return;
        }
        // SAFETY: recording; range checked.
        unsafe { (self.sh.f.cmd_draw_indexed_indirect)(self.cb, buf.handle(), offset, count, stride) };
    }

    /// up to max_count indexed draws, their number (u32) read from count_buf at count_offset
    /// (level Voll: Features::draw_indirect_count)
    pub fn draw_indexed_indirect_count(&mut self, buf: &'a Buffer, offset: u64, count_buf: &'a Buffer, count_offset: u64,
                                       max_count: u32, stride: u32) {
        let f = self.sh.f.cmd_draw_indexed_indirect_count;
        let ok = self.sh.features.draw_indirect_count && f.is_some() && stride >= 20 && stride % 4 == 0 && max_count > 0;
        if self.ok(ok, "draw_indexed_indirect_count: needs level Voll, stride >= 20").is_none()
            || self.can_draw(true).is_none()
            || self.indirect_ok(buf, offset, (max_count as u64 - 1) * stride as u64 + 20).is_none()
            || self.indirect_ok(count_buf, count_offset, 4).is_none()
        {
            return;
        }
        let Some(f) = f else { return };
        // SAFETY: recording; ranges checked; the feature is enabled.
        unsafe { f(self.cb, buf.handle(), offset, count_buf.handle(), count_offset, max_count, stride) };
    }

    // ------------------------------------------------------------ timestamps

    /// timestamps first .. first + n may be written again (outside render passes)
    pub fn reset_timestamps(&mut self, ts: &'a Timestamps, first: u32, n: u32) {
        let ok = self.pass.is_none() && n > 0 && first.checked_add(n).is_some_and(|e| e <= ts.0.count);
        if self.ok(ok, "reset_timestamps: inside a pass or range").is_none() {
            return;
        }
        // SAFETY: recording outside a pass; range checked.
        unsafe { (self.sh.f.cmd_reset_query_pool)(self.cb, ts.0.pool, first, n) };
    }

    /// write timestamp i once every earlier command has finished (reset it first)
    pub fn timestamp(&mut self, ts: &'a Timestamps, i: u32) {
        if self.ok(i < ts.0.count, "timestamp: index outside the set").is_none() {
            return;
        }
        // SAFETY: recording; index checked.
        unsafe { (self.sh.f.cmd_write_timestamp)(self.cb, vk::PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, ts.0.pool, i) };
    }

    // ------------------------------------------------------------ end

    /// the recorded commands, or the first misuse
    pub fn finish(mut self) -> Result<CommandBuffer<'a>> {
        if self.pass.is_some() {
            self.ok(false, "finish inside a render pass");
        }
        if let Some(e) = self.err {
            return Err(e); // Drop gives the slot back
        }
        let slot = self.slot.take().unwrap_or_else(|| unreachable!());
        // SAFETY: the buffer is recording.
        let r = unsafe { (self.sh.f.end_command_buffer)(slot.cb) };
        if let Err(e) = self.sh.vk(r) {
            self.sh.give_slot(slot);
            return Err(e);
        }
        Ok(CommandBuffer { sh: self.sh, slot: Some(slot) })
    }
}
