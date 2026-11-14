// web_cmd -- command recording in the browser: CommandEncoder -> CommandBuffer
//
// The commands become words here and a GPUCommandEncoder in web/gpu.js, all in the one call of
// Queue::submit (the op numbers: GpuHost.submit there): a frame costs one crossing into JavaScript, not
// one per command, and nothing of WebGPU lives across calls.  What the API lets go on anywhere WebGPU
// wants in a pass: the host opens a compute pass at the first dispatch and ends it before the next
// thing that is no dispatch.  Barriers and layouts mean nothing here (WebGPU's usage scopes do that);
// their calls stay so that the same code runs on every target.
//
// Every command checks what can be known here (the pipeline and pass state, bind group layouts, buffer
// usages, ranges and alignments, row lengths of 256 bytes on copies); a failed check skips the
// command and finish() returns it as GpuError::Invalid.  The rest is the browser's, later (web_dev.rs).
// The level Voll's commands are Invalid.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::marker::PhantomData;

use crate::web_dev::{DevInner, Device};
use crate::*;

/// the row length WebGPU wants of copies between buffers and textures
const COPY_ROW: u32 = 256;

const OP_COPY_BUFFER: u32 = 1;
const OP_FILL: u32 = 2;
const OP_BUFFER_TO_TEXTURE: u32 = 3;
const OP_TEXTURE_TO_BUFFER: u32 = 4;
const OP_COMPUTE_PIPELINE: u32 = 5;
const OP_BIND_GROUP: u32 = 6;
const OP_PUSH: u32 = 7;
const OP_DISPATCH: u32 = 8;
const OP_DISPATCH_INDIRECT: u32 = 9;
const OP_BEGIN_RENDER: u32 = 10;
const OP_END_RENDER: u32 = 11;
const OP_RENDER_PIPELINE: u32 = 12;
const OP_VIEWPORT: u32 = 13;
const OP_SCISSOR: u32 = 14;
const OP_VERTEX: u32 = 15;
const OP_INDEX: u32 = 16;
const OP_DRAW: u32 = 17;
const OP_DRAW_INDEXED: u32 = 18;
const OP_DRAW_INDIRECT: u32 = 19;
const OP_DRAW_INDEXED_INDIRECT: u32 = 20;

/// records commands; finish() makes a CommandBuffer for Queue::submit
pub struct CommandEncoder<'a> {
    #[allow(dead_code)]
    d: Rc<DevInner>,
    w: Vec<u32>,
    err: Option<GpuError>,
    rendering: bool,
    compute: Option<&'a ComputePipeline>,
    render: Option<&'a RenderPipeline>,
    // the running render pass's formats
    colors: Vec<Format>,
    depth: Option<Format>,
}

/// recorded commands, ready to submit (dropped unsubmitted: discarded)
pub struct CommandBuffer<'a> {
    pub(crate) words: Vec<u32>,
    _p: PhantomData<&'a ()>,
}

impl Device {
    /// a command encoder
    pub fn encoder(&self) -> Result<CommandEncoder<'_>> {
        self.d.check()?;
        Ok(CommandEncoder { d: self.d.clone(), w: Vec::with_capacity(64), err: None, rendering: false, compute: None, render: None, colors: Vec::new(), depth: None })
    }
}

fn split(v: u64) -> [u32; 2] {
    [v as u32, (v >> 32) as u32]
}

impl<'a> CommandEncoder<'a> {
    fn fail(&mut self, what: &'static str) {
        if self.err.is_none() {
            self.err = Some(GpuError::Invalid(what));
        }
    }

    // not in a render pass (else the misuse)
    fn outside(&mut self, what: &'static str) -> bool {
        if self.rendering {
            self.fail(what);
        }
        !self.rendering
    }

    fn inside(&mut self, what: &'static str) -> bool {
        if !self.rendering {
            self.fail(what);
        }
        self.rendering
    }

    pub fn copy_buffer(&mut self, src: &'a Buffer, src_off: u64, dst: &'a Buffer, dst_off: u64, size: u64) {
        if !self.outside("copy_buffer in a render pass") {
            return;
        }
        if src_off % 4 != 0 || dst_off % 4 != 0 || size % 4 != 0 || src_off.checked_add(size).is_none_or(|e| e > src.size())
            || dst_off.checked_add(size).is_none_or(|e| e > dst.size()) {
            return self.fail("copy_buffer: outside the buffers, or not in multiples of 4");
        }
        let (a, b, c) = (split(src_off), split(dst_off), split(size));
        self.w.extend_from_slice(&[OP_COPY_BUFFER, src.handle(), a[0], a[1], dst.handle(), b[0], b[1], c[0], c[1]]);
    }

    /// fill size bytes at offset with a 32-bit value (both multiples of 4)
    pub fn fill_buffer(&mut self, buf: &'a Buffer, offset: u64, size: u64, value: u32) {
        if !self.outside("fill_buffer in a render pass") {
            return;
        }
        if offset % 4 != 0 || size % 4 != 0 || offset.checked_add(size).is_none_or(|e| e > buf.size()) {
            return self.fail("fill_buffer: outside the buffer, or not in multiples of 4");
        }
        let (a, b) = (split(offset), split(size));
        self.w.extend_from_slice(&[OP_FILL, buf.handle(), a[0], a[1], b[0], b[1], value]);
    }

    // what a copy between a buffer and a texture needs of its layout: rows of 256 bytes -> bytes per row,
    // rows per image
    fn copy_layout(&mut self, r: &TextureRegion<'a>, l: &DataLayout) -> Option<(u32, u32)> {
        let f = r.texture.format();
        if f.is_depth() {
            self.fail("copies of depth textures");
            return None;
        }
        let (bw, bb) = (f.block_dim(), f.block_bytes());
        let tight = r.size[0].div_ceil(bw) * bb;
        let row = if l.bytes_per_row == 0 { tight } else { l.bytes_per_row };
        let rows = r.size[1].div_ceil(bw);
        if row < tight {
            self.fail("a copy's row is shorter than the region");
            return None;
        }
        // (WebGPU's rule; a tight row is fine only when it is a single row)
        if row % COPY_ROW != 0 && rows.max(r.layers).max(r.size[2]) > 1 {
            self.fail("a copy's rows must be multiples of 256 bytes in WebGPU");
            return None;
        }
        if l.offset % bb as u64 != 0 {
            self.fail("a copy's offset must be a multiple of the texel size");
            return None;
        }
        let per = if l.rows_per_image == 0 { rows } else { l.rows_per_image };
        Some((row, per))
    }

    /// buffer data -> a texture region
    pub fn copy_buffer_to_texture(&mut self, src: &'a Buffer, layout: DataLayout, dst: TextureRegion<'a>) {
        if !self.outside("copy_buffer_to_texture in a render pass") {
            return;
        }
        let Some((row, per)) = self.copy_layout(&dst, &layout) else { return };
        let o = split(layout.offset);
        let (z, d) = if dst.texture.depth() > 1 { (dst.origin[2], dst.size[2].max(1)) } else { (dst.base_layer, dst.layers.max(1)) };
        self.w.extend_from_slice(&[OP_BUFFER_TO_TEXTURE, src.handle(), o[0], o[1], row, per, dst.texture.handle(), dst.mip, dst.origin[0], dst.origin[1], z, dst.size[0], dst.size[1], d]);
    }

    /// a texture region -> buffer data
    pub fn copy_texture_to_buffer(&mut self, src: TextureRegion<'a>, dst: &'a Buffer, layout: DataLayout) {
        if !self.outside("copy_texture_to_buffer in a render pass") {
            return;
        }
        let Some((row, per)) = self.copy_layout(&src, &layout) else { return };
        let o = split(layout.offset);
        let (z, d) = if src.texture.depth() > 1 { (src.origin[2], src.size[2].max(1)) } else { (src.base_layer, src.layers.max(1)) };
        self.w.extend_from_slice(&[OP_TEXTURE_TO_BUFFER, src.texture.handle(), src.mip, src.origin[0], src.origin[1], z, src.size[0], src.size[1], d, dst.handle(), o[0], o[1], row, per]);
    }

    /// (WebGPU's usage scopes do this)
    pub fn barrier(&mut self, _before: Access, _after: Access) {}
    pub fn texture_barrier(&mut self, _tex: &'a Texture, _before: Layout, _after: Layout) {}
    pub fn texture_barrier_range(&mut self, _tex: &'a Texture, _base_mip: u32, _mips: u32, _base_layer: u32, _layers: u32, _before: Layout, _after: Layout) {}

    pub fn set_compute_pipeline(&mut self, p: &'a ComputePipeline) {
        if self.outside("set_compute_pipeline in a render pass") {
            self.compute = Some(p);
            self.w.extend_from_slice(&[OP_COMPUTE_PIPELINE, p.0.h.h]);
        }
    }

    /// bind group g (= @group(g)) of the current pipeline's layout
    pub fn set_bind_group(&mut self, index: u32, group: &'a BindGroup) {
        self.w.extend_from_slice(&[OP_BIND_GROUP, index, group.handle()]);
        let layout = if self.rendering { self.render.map(|p| &p.0.layout) } else { self.compute.map(|p| &p.0.layout) };
        let Some(layout) = layout else {
            return self.fail("set_bind_group before a pipeline");
        };
        if !layout.0.groups.get(index as usize).is_some_and(|l| group.fits(l)) {
            self.fail("set_bind_group: the group does not fit the pipeline's layout");
        }
    }

    /// push constants at offset (bytes) for the current pipeline (a uniform block in the browser)
    pub fn push_constants<T: Pod>(&mut self, offset: u32, data: &[T]) {
        let bytes = bytes_of(data);
        let layout = if self.rendering { self.render.map(|p| &p.0.layout) } else { self.compute.map(|p| &p.0.layout) };
        let Some(layout) = layout else {
            return self.fail("push_constants before a pipeline");
        };
        if offset % 4 != 0 || bytes.len() % 4 != 0 || (offset as usize).checked_add(bytes.len()).is_none_or(|e| e > layout.0.push as usize) {
            return self.fail("push_constants: beyond the layout's block, or not in multiples of 4");
        }
        self.w.extend_from_slice(&[OP_PUSH, offset, bytes.len() as u32 / 4]);
        self.w.extend(bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])));
    }

    pub fn dispatch(&mut self, x: u32, y: u32, z: u32) {
        if !self.outside("dispatch in a render pass") {
            return;
        }
        if self.compute.is_none() {
            return self.fail("dispatch without a pipeline");
        }
        self.w.extend_from_slice(&[OP_DISPATCH, x, y, z]);
    }

    /// workgroup counts (3 x u32) from buf at offset
    pub fn dispatch_indirect(&mut self, buf: &'a Buffer, offset: u64) {
        if !self.outside("dispatch_indirect in a render pass") {
            return;
        }
        if self.compute.is_none() || !buf.usage().contains(BufferUsage::INDIRECT) || offset % 4 != 0 || offset.checked_add(12).is_none_or(|e| e > buf.size()) {
            return self.fail("dispatch_indirect: no pipeline, no INDIRECT usage, or outside the buffer");
        }
        let o = split(offset);
        self.w.extend_from_slice(&[OP_DISPATCH_INDIRECT, buf.handle(), o[0], o[1]]);
    }

    pub fn begin_render(&mut self, desc: &RenderPassDesc<'_, 'a>) {
        if !self.outside("begin_render in a render pass") {
            return;
        }
        if desc.color.len() > COLOR_TARGETS_MAX || (desc.color.is_empty() && desc.depth.is_none()) {
            return self.fail("begin_render: no attachments, or too many");
        }
        self.rendering = true;
        self.render = None;
        self.colors.clear();
        self.colors.extend(desc.color.iter().map(|c| c.view.format()));
        self.depth = desc.depth.as_ref().map(|d| d.view.format());
        self.w.extend_from_slice(&[OP_BEGIN_RENDER, desc.color.len() as u32]);
        for c in desc.color {
            self.w.extend_from_slice(&[c.view.handle(), (c.load == Load::Keep) as u32, c.store as u32, c.clear[0].to_bits(), c.clear[1].to_bits(), c.clear[2].to_bits(), c.clear[3].to_bits()]);
        }
        match &desc.depth {
            Some(d) => self.w.extend_from_slice(&[d.view.handle(), (d.load == Load::Keep) as u32, d.store as u32, d.clear.to_bits(), d.view.format().has_stencil() as u32]),
            None => self.w.extend_from_slice(&[0; 5]),
        }
    }

    pub fn end_render(&mut self) {
        if self.inside("end_render outside a render pass") {
            self.rendering = false;
            self.render = None;
            self.w.push(OP_END_RENDER);
        }
    }

    pub fn set_render_pipeline(&mut self, p: &'a RenderPipeline) {
        if self.inside("set_render_pipeline outside a render pass") {
            if p.0.targets != self.colors || p.0.depth != self.depth {
                return self.fail("set_render_pipeline: the pipeline's formats are not the render pass's");
            }
            self.render = Some(p);
            self.w.extend_from_slice(&[OP_RENDER_PIPELINE, p.0.h.h]);
        }
    }

    /// the viewport in pixels from the top left (depth range min ..= max)
    pub fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32, min_depth: f32, max_depth: f32) {
        if self.inside("set_viewport outside a render pass") {
            self.w.extend_from_slice(&[OP_VIEWPORT, x.to_bits(), y.to_bits(), w.to_bits(), h.to_bits(), min_depth.to_bits(), max_depth.to_bits()]);
        }
    }

    pub fn set_scissor(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if self.inside("set_scissor outside a render pass") {
            self.w.extend_from_slice(&[OP_SCISSOR, x, y, w, h]);
        }
    }

    pub fn set_vertex_buffer(&mut self, slot: u32, buf: &'a Buffer, offset: u64) {
        if !self.inside("set_vertex_buffer outside a render pass") {
            return;
        }
        if !buf.usage().contains(BufferUsage::VERTEX) {
            return self.fail("set_vertex_buffer: no VERTEX usage");
        }
        let o = split(offset);
        self.w.extend_from_slice(&[OP_VERTEX, slot, buf.handle(), o[0], o[1]]);
    }

    pub fn set_index_buffer(&mut self, buf: &'a Buffer, offset: u64, format: IndexFormat) {
        if !self.inside("set_index_buffer outside a render pass") {
            return;
        }
        if !buf.usage().contains(BufferUsage::INDEX) {
            return self.fail("set_index_buffer: no INDEX usage");
        }
        let o = split(offset);
        self.w.extend_from_slice(&[OP_INDEX, buf.handle(), o[0], o[1], (format == IndexFormat::U32) as u32]);
    }

    fn drawing(&mut self) -> bool {
        if !self.inside("a draw outside a render pass") {
            return false;
        }
        if self.render.is_none() {
            self.fail("a draw without a pipeline");
            return false;
        }
        true
    }

    pub fn draw(&mut self, vertices: u32, instances: u32, first_vertex: u32, first_instance: u32) {
        if self.drawing() {
            self.w.extend_from_slice(&[OP_DRAW, vertices, instances, first_vertex, first_instance]);
        }
    }

    pub fn draw_indexed(&mut self, indices: u32, instances: u32, first_index: u32, base_vertex: i32, first_instance: u32) {
        if self.drawing() {
            self.w.extend_from_slice(&[OP_DRAW_INDEXED, indices, instances, first_index, base_vertex as u32, first_instance]);
        }
    }

    /// one draw: vertex count, instance count, first vertex, first instance (4 x u32)
    pub fn draw_indirect(&mut self, buf: &'a Buffer, offset: u64) {
        if !self.drawing() {
            return;
        }
        if !buf.usage().contains(BufferUsage::INDIRECT) || offset % 4 != 0 || offset.checked_add(16).is_none_or(|e| e > buf.size()) {
            return self.fail("draw_indirect: no INDIRECT usage, or outside the buffer");
        }
        let o = split(offset);
        self.w.extend_from_slice(&[OP_DRAW_INDIRECT, buf.handle(), o[0], o[1]]);
    }

    /// one indexed draw: index count, instance count, first index, base vertex (i32), first instance
    pub fn draw_indexed_indirect(&mut self, buf: &'a Buffer, offset: u64) {
        if !self.drawing() {
            return;
        }
        if !buf.usage().contains(BufferUsage::INDIRECT) || offset % 4 != 0 || offset.checked_add(20).is_none_or(|e| e > buf.size()) {
            return self.fail("draw_indexed_indirect: no INDIRECT usage, or outside the buffer");
        }
        let o = split(offset);
        self.w.extend_from_slice(&[OP_DRAW_INDEXED_INDIRECT, buf.handle(), o[0], o[1]]);
    }

    /// (level Voll)
    pub fn draw_indexed_indirect_multi(&mut self, _buf: &'a Buffer, _offset: u64, _count: u32, _stride: u32) {
        self.fail("draw_indexed_indirect_multi: level Voll");
    }

    /// (level Voll)
    pub fn draw_indexed_indirect_count(&mut self, _buf: &'a Buffer, _offset: u64, _count_buf: &'a Buffer, _count_offset: u64, _max_count: u32, _stride: u32) {
        self.fail("draw_indexed_indirect_count: level Voll");
    }

    /// (not built in the browser)
    pub fn reset_timestamps(&mut self, _ts: &'a Timestamps, _first: u32, _n: u32) {
        self.fail("timestamps in the browser");
    }

    /// (not built in the browser)
    pub fn timestamp(&mut self, _ts: &'a Timestamps, _i: u32) {
        self.fail("timestamps in the browser");
    }

    /// the recorded commands, or the first misuse
    pub fn finish(mut self) -> Result<CommandBuffer<'a>> {
        if self.rendering {
            self.fail("finish inside a render pass");
        }
        if let Some(e) = self.err {
            return Err(e);
        }
        Ok(CommandBuffer { words: core::mem::take(&mut self.w), _p: PhantomData })
    }
}
