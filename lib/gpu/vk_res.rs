// vk_res -- Vulkan resources, bindings and pipelines
//
//   device.create_buffer(&BufferDesc) -> Buffer        copies to and from it always allowed;
//        Upload / Readback buffers are mapped for their lifetime: write(off, &[T]) /
//        read(off, &mut [T]) (non-coherent memory flushed / invalidated by atom)
//   device.create_texture(&TextureDesc) -> Texture     optimal tiling, starts in Layout::
//        Undefined; its default view covers everything (2D, 2D array, cube, cube array, 3D);
//        texture.create_view(&ViewDesc) -> TextureView a range of mips and layers
//   device.create_sampler(&SamplerDesc) -> Sampler
//   ShaderModule::from_spirv(&device, &[u32])          SPIR-V up to 1.3 (Vulkan 1.1), 1.5
//        (1.2), 1.6 (1.3); the header and the instruction lengths are checked, the code is the
//        driver's; entry points and specialization constants are read (shader::reflect)
//   device.create_bind_group_layout(&[Binding])        descriptor set layout (one descriptor
//        per binding)
//   device.create_pipeline_layout(&PipelineLayoutDesc) set g = groups[g]; push constants as one
//        range for every stage (VK_SHADER_STAGE_ALL)
//   device.create_bind_group(&BindGroupDesc)           a descriptor set from the device's pools
//        (freed individually); it keeps its resources alive
//   device.create_bindless(&BindlessDesc) -> Bindless  (Voll) one update-after-bind,
//        partially bound set: binding 0 sampled textures, 1 samplers, 2 storage buffers,
//        3 storage textures; table.group() binds it, table.layout() goes into pipeline layouts
//   device.create_compute_pipeline / create_render_pipeline   (dynamic rendering: the target
//        formats are part of the pipeline; viewport and scissor are dynamic); per stage: the
//        entry point must exist with the stage's model, its pipeline constants become a
//        VkSpecializationInfo (types and ids checked against the module), a compute workgroup
//        size (LocalSize, LocalSizeId or the WorkgroupSize built-in, after specialization)
//        must fit the limits, LocalSizeId needs maintenance4
//   device.create_timestamps(n) -> Timestamps          read_ticks(first, n) / read_ms(first, n)
//        (Err(NotReady) until every one was written and finished)
//
// Every object holds the device (Arc) and is destroyed through the device's garbage once the
// GPU has finished the submissions made before it was dropped (vk_dev.rs).

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;

use rt::sync::SpinLock;

use crate::vk_dev::{Device, Garbage, Shared};
use crate::vk_ffi as vk;
use crate::vk_mem::Alloc;
use crate::*;

// ---------------------------------------------------------------- formats

pub(crate) fn vk_format(f: Format) -> u32 {
    use Format::*;
    match f {
        R8Unorm => vk::FORMAT_R8_UNORM,
        Rg8Unorm => vk::FORMAT_R8G8_UNORM,
        Rgba8Unorm => vk::FORMAT_R8G8B8A8_UNORM,
        Rgba8Srgb => vk::FORMAT_R8G8B8A8_SRGB,
        Bgra8Unorm => vk::FORMAT_B8G8R8A8_UNORM,
        Bgra8Srgb => vk::FORMAT_B8G8R8A8_SRGB,
        Rgb10a2Unorm => vk::FORMAT_A2B10G10R10_UNORM_PACK32,
        Rg11b10Float => vk::FORMAT_B10G11R11_UFLOAT_PACK32,
        R16Float => vk::FORMAT_R16_SFLOAT,
        Rg16Float => vk::FORMAT_R16G16_SFLOAT,
        Rgba16Float => vk::FORMAT_R16G16B16A16_SFLOAT,
        R32Uint => vk::FORMAT_R32_UINT,
        R32Sint => vk::FORMAT_R32_SINT,
        R32Float => vk::FORMAT_R32_SFLOAT,
        Rg32Uint => vk::FORMAT_R32G32_UINT,
        Rg32Float => vk::FORMAT_R32G32_SFLOAT,
        Rgba32Uint => vk::FORMAT_R32G32B32A32_UINT,
        Rgba32Float => vk::FORMAT_R32G32B32A32_SFLOAT,
        Depth16 => vk::FORMAT_D16_UNORM,
        Depth32Float => vk::FORMAT_D32_SFLOAT,
        Depth24Stencil8 => vk::FORMAT_D24_UNORM_S8_UINT,
        Depth32FloatStencil8 => vk::FORMAT_D32_SFLOAT_S8_UINT,
        Bc1Unorm => vk::FORMAT_BC1_RGBA_UNORM_BLOCK,
        Bc1Srgb => vk::FORMAT_BC1_RGBA_SRGB_BLOCK,
        Bc3Unorm => vk::FORMAT_BC3_UNORM_BLOCK,
        Bc3Srgb => vk::FORMAT_BC3_SRGB_BLOCK,
        Bc4Unorm => vk::FORMAT_BC4_UNORM_BLOCK,
        Bc5Unorm => vk::FORMAT_BC5_UNORM_BLOCK,
        Bc7Unorm => vk::FORMAT_BC7_UNORM_BLOCK,
        Bc7Srgb => vk::FORMAT_BC7_SRGB_BLOCK,
    }
}

/// every aspect of a format (barriers) and the one of copies and views
fn aspects(f: Format) -> (u32, u32) {
    if f.has_stencil() {
        (vk::IMAGE_ASPECT_DEPTH_BIT | vk::IMAGE_ASPECT_STENCIL_BIT, vk::IMAGE_ASPECT_DEPTH_BIT)
    } else if f.is_depth() {
        (vk::IMAGE_ASPECT_DEPTH_BIT, vk::IMAGE_ASPECT_DEPTH_BIT)
    } else {
        (vk::IMAGE_ASPECT_COLOR_BIT, vk::IMAGE_ASPECT_COLOR_BIT)
    }
}

pub(crate) fn compare_op(c: Compare) -> u32 {
    match c {
        Compare::Never => vk::COMPARE_OP_NEVER,
        Compare::Less => vk::COMPARE_OP_LESS,
        Compare::Equal => vk::COMPARE_OP_EQUAL,
        Compare::LessEqual => vk::COMPARE_OP_LESS_OR_EQUAL,
        Compare::Greater => vk::COMPARE_OP_GREATER,
        Compare::NotEqual => vk::COMPARE_OP_NOT_EQUAL,
        Compare::GreaterEqual => vk::COMPARE_OP_GREATER_OR_EQUAL,
        Compare::Always => vk::COMPARE_OP_ALWAYS,
    }
}

fn stage_bits(s: Stages) -> u32 {
    let mut v = 0;
    if s.intersects(Stages::VERTEX) {
        v |= vk::SHADER_STAGE_VERTEX_BIT;
    }
    if s.intersects(Stages::FRAGMENT) {
        v |= vk::SHADER_STAGE_FRAGMENT_BIT;
    }
    if s.intersects(Stages::COMPUTE) {
        v |= vk::SHADER_STAGE_COMPUTE_BIT;
    }
    v
}

fn descriptor_type(k: BindingKind) -> u32 {
    match k {
        BindingKind::UniformBuffer => vk::DESCRIPTOR_TYPE_UNIFORM_BUFFER,
        BindingKind::StorageBuffer | BindingKind::StorageBufferRead => vk::DESCRIPTOR_TYPE_STORAGE_BUFFER,
        BindingKind::Texture => vk::DESCRIPTOR_TYPE_SAMPLED_IMAGE,
        BindingKind::StorageTexture => vk::DESCRIPTOR_TYPE_STORAGE_IMAGE,
        BindingKind::Sampler => vk::DESCRIPTOR_TYPE_SAMPLER,
    }
}

fn nul_name(s: &str) -> Vec<u8> {
    let mut v: Vec<u8> = s.bytes().filter(|&b| b != 0).collect();
    v.push(0);
    v
}

// ---------------------------------------------------------------- buffers

pub(crate) struct BufInner {
    sh: Arc<Shared>,
    h: vk::Handle,
    a: Alloc,
    size: u64,
    usage: BufferUsage,
    mem: MemoryKind,
}

impl Drop for BufInner {
    fn drop(&mut self) {
        self.sh.defer(Garbage::Buffer(self.h, self.a));
    }
}

/// GPU memory for data: vertices, indices, storage, uniforms, indirect arguments
#[derive(Clone)]
pub struct Buffer(Arc<BufInner>);

impl Buffer {
    pub fn size(&self) -> u64 {
        self.0.size
    }
    pub fn usage(&self) -> BufferUsage {
        self.0.usage
    }
    pub fn memory(&self) -> MemoryKind {
        self.0.mem
    }
    pub(crate) fn handle(&self) -> vk::Handle {
        self.0.h
    }

    fn mapped_range(&self, offset: u64, n: u64) -> Result<usize> {
        let b = &self.0;
        // by its kind, not by where it lies: on an integrated or CPU adapter all memory may be
        // host-visible, and a program that wrote there would fail on the next GPU
        if b.mem == MemoryKind::Device || b.a.ptr == 0 {
            return Err(GpuError::Invalid("buffer not host-visible (MemoryKind::Device)"));
        }
        if offset.checked_add(n).is_none_or(|e| e > b.size) {
            return Err(GpuError::Invalid("range outside the buffer"));
        }
        Ok(b.a.ptr + offset as usize)
    }

    /// the memory range of [offset, offset + n) widened to the non-coherent atom
    fn atom_range(&self, offset: u64, n: u64) -> vk::MappedMemoryRange {
        let b = &self.0;
        let atom = b.sh.raw.non_coherent_atom_size.max(1);
        let start = (b.a.offset + offset) / atom * atom;
        let end = (b.a.offset + offset + n).div_ceil(atom) * atom;
        let block_end = b.a.offset + b.a.size; // the allocation ends inside its block
        vk::MappedMemoryRange {
            s_type: vk::STRUCTURE_TYPE_MAPPED_MEMORY_RANGE,
            memory: b.a.mem,
            offset: start,
            size: if end > block_end { vk::WHOLE_SIZE } else { end - start },
            ..vk::Zero::zero()
        }
    }

    /// copy data into a mapped buffer (Upload / Readback) at offset; the GPU sees it in the
    /// next submission -- do not write what a pending submission still reads
    pub fn write<T: Pod>(&self, offset: u64, data: &[T]) -> Result<()> {
        let bytes = bytes_of(data);
        let p = self.mapped_range(offset, bytes.len() as u64)?;
        // SAFETY: p addresses bytes.len() mapped bytes inside the buffer's allocation.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len()) };
        if !self.0.a.coherent && !bytes.is_empty() {
            let r = self.atom_range(offset, bytes.len() as u64);
            // SAFETY: a range of the mapped memory of this allocation.
            self.0.sh.vk(unsafe { (self.0.sh.f.flush_mapped_memory_ranges)(self.0.sh.dev, 1, &r) })?;
        }
        Ok(())
    }

    /// copy a mapped buffer's contents at offset into out (wait for the submission that wrote
    /// them first)
    pub fn read<T: Pod>(&self, offset: u64, out: &mut [T]) -> Result<()> {
        let n = core::mem::size_of_val(out);
        let p = self.mapped_range(offset, n as u64)?;
        if !self.0.a.coherent && n > 0 {
            let r = self.atom_range(offset, n as u64);
            // SAFETY: a range of the mapped memory of this allocation.
            self.0.sh.vk(unsafe { (self.0.sh.f.invalidate_mapped_memory_ranges)(self.0.sh.dev, 1, &r) })?;
        }
        // SAFETY: p addresses n mapped bytes; T is Pod, so any bytes are a valid T.
        unsafe { core::ptr::copy_nonoverlapping(p as *const u8, out.as_mut_ptr() as *mut u8, n) };
        Ok(())
    }
}

impl Device {
    pub fn create_buffer(&self, desc: &BufferDesc) -> Result<Buffer> {
        let sh = &self.sh;
        sh.check()?;
        if desc.size == 0 {
            return Err(GpuError::Invalid("buffer size 0"));
        }
        let u = desc.usage;
        let mut usage = vk::BUFFER_USAGE_TRANSFER_SRC_BIT | vk::BUFFER_USAGE_TRANSFER_DST_BIT;
        for (b, v) in [
            (BufferUsage::UNIFORM, vk::BUFFER_USAGE_UNIFORM_BUFFER_BIT),
            (BufferUsage::STORAGE, vk::BUFFER_USAGE_STORAGE_BUFFER_BIT),
            (BufferUsage::INDEX, vk::BUFFER_USAGE_INDEX_BUFFER_BIT),
            (BufferUsage::VERTEX, vk::BUFFER_USAGE_VERTEX_BUFFER_BIT),
            (BufferUsage::INDIRECT, vk::BUFFER_USAGE_INDIRECT_BUFFER_BIT),
        ] {
            if u.contains(b) {
                usage |= v;
            }
        }
        let mut st = sh.st.lock();
        let (h, a) = sh.raw_buffer(&mut st, desc.size, usage, desc.memory)?;
        drop(st);
        Ok(Buffer(Arc::new(BufInner { sh: sh.clone(), h, a, size: desc.size, usage: u, mem: desc.memory })))
    }
}

// ---------------------------------------------------------------- textures

pub(crate) struct TexInner {
    sh: Arc<Shared>,
    pub image: vk::Handle,
    a: Option<Alloc>,
    owned: bool,
    pub desc: TextureDesc,
    pub aspect_all: u32,
    pub aspect_copy: u32,
}

impl Drop for TexInner {
    fn drop(&mut self) {
        if self.owned {
            self.sh.defer(Garbage::Image(self.image, self.a));
        }
    }
}

pub(crate) struct ViewInner {
    sh: Arc<Shared>,
    pub h: vk::Handle,
    pub tex: Arc<TexInner>,
    pub base_mip: u32,
    pub mips: u32,
    pub base_layer: u32,
    pub layers: u32,
}

impl Drop for ViewInner {
    fn drop(&mut self) {
        self.sh.defer(Garbage::View(self.h));
    }
}

/// a range of a texture's mips and layers as shaders and render passes see it
#[derive(Clone)]
pub struct TextureView(pub(crate) Arc<ViewInner>);

impl TextureView {
    pub fn format(&self) -> Format {
        self.0.tex.desc.format
    }
    /// width and height of the view's first mip
    pub fn size(&self) -> (u32, u32) {
        let d = &self.0.tex.desc;
        ((d.width >> self.0.base_mip).max(1), (d.height >> self.0.base_mip).max(1))
    }
    /// what its texture may be used for
    pub fn usage(&self) -> TextureUsage {
        self.0.tex.desc.usage
    }
    /// the same view (it or a clone of it), not merely one of the same range: a cache's key
    pub fn same(&self, other: &TextureView) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// an image on the GPU: 2D, 2D array, cube (array) or 3D, with mips
#[derive(Clone)]
pub struct Texture {
    pub(crate) t: Arc<TexInner>,
    view: TextureView,
}

impl Texture {
    /// the view of everything (its natural dimension)
    pub fn view(&self) -> &TextureView {
        &self.view
    }
    pub fn width(&self) -> u32 {
        self.t.desc.width
    }
    pub fn height(&self) -> u32 {
        self.t.desc.height
    }
    pub fn depth(&self) -> u32 {
        self.t.desc.depth
    }
    pub fn layers(&self) -> u32 {
        self.t.desc.layers
    }
    pub fn mips(&self) -> u32 {
        self.t.desc.mips
    }
    pub fn format(&self) -> Format {
        self.t.desc.format
    }
    pub fn usage(&self) -> TextureUsage {
        self.t.desc.usage
    }
    pub(crate) fn image(&self) -> vk::Handle {
        self.t.image
    }

    /// a view of some mips / layers (counts 0: to the end)
    pub fn create_view(&self, desc: &ViewDesc) -> Result<TextureView> {
        let d = &self.t.desc;
        let mips = if desc.mips == 0 { d.mips.saturating_sub(desc.base_mip) } else { desc.mips };
        let layers = if desc.layers == 0 { d.layers.saturating_sub(desc.base_layer) } else { desc.layers };
        if mips == 0 || desc.base_mip.checked_add(mips).is_none_or(|e| e > d.mips) || layers == 0
            || desc.base_layer.checked_add(layers).is_none_or(|e| e > d.layers) {
            return Err(GpuError::Invalid("view range outside the texture"));
        }
        let ok = match desc.dim {
            ViewDim::D2 => d.depth == 1 && layers == 1,
            ViewDim::D2Array => d.depth == 1,
            ViewDim::Cube => d.cube && layers == 6,
            ViewDim::CubeArray => d.cube && layers % 6 == 0,
            ViewDim::D3 => d.depth > 1,
        };
        if !ok {
            return Err(GpuError::Invalid("view dimension does not fit the texture"));
        }
        make_view(&self.t, desc.dim, desc.base_mip, mips, desc.base_layer, layers)
    }
}

fn make_view(t: &Arc<TexInner>, dim: ViewDim, base_mip: u32, mips: u32, base_layer: u32, layers: u32) -> Result<TextureView> {
    let sh = &t.sh;
    let vi = vk::ImageViewCreateInfo {
        s_type: vk::STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
        image: t.image,
        view_type: match dim {
            ViewDim::D2 => vk::IMAGE_VIEW_TYPE_2D,
            ViewDim::D2Array => vk::IMAGE_VIEW_TYPE_2D_ARRAY,
            ViewDim::Cube => vk::IMAGE_VIEW_TYPE_CUBE,
            ViewDim::CubeArray => vk::IMAGE_VIEW_TYPE_CUBE_ARRAY,
            ViewDim::D3 => vk::IMAGE_VIEW_TYPE_3D,
        },
        format: vk_format(t.desc.format),
        subresource_range: vk::ImageSubresourceRange {
            aspect_mask: t.aspect_copy,
            base_mip_level: base_mip,
            level_count: mips,
            base_array_layer: base_layer,
            layer_count: layers,
        },
        ..vk::Zero::zero()
    };
    let mut h = 0;
    // SAFETY: a valid create info for an image of this device.
    sh.vk(unsafe { (sh.f.create_image_view)(sh.dev, &vi, core::ptr::null(), &mut h) })?;
    Ok(TextureView(Arc::new(ViewInner { sh: sh.clone(), h, tex: t.clone(), base_mip, mips, base_layer, layers })))
}

/// the natural view dimension of a texture
fn default_dim(d: &TextureDesc) -> ViewDim {
    if d.depth > 1 {
        ViewDim::D3
    } else if d.cube {
        if d.layers == 6 { ViewDim::Cube } else { ViewDim::CubeArray }
    } else if d.layers > 1 {
        ViewDim::D2Array
    } else {
        ViewDim::D2
    }
}

/// a swapchain image as a Texture (not owned: the swapchain destroys it)
pub(crate) fn wrap_image(sh: &Arc<Shared>, image: vk::Handle, desc: TextureDesc) -> Result<Texture> {
    let (aspect_all, aspect_copy) = aspects(desc.format);
    let t = Arc::new(TexInner { sh: sh.clone(), image, a: None, owned: false, desc, aspect_all, aspect_copy });
    let view = make_view(&t, ViewDim::D2, 0, 1, 0, 1)?;
    Ok(Texture { t, view })
}

impl Device {
    pub fn create_texture(&self, desc: &TextureDesc) -> Result<Texture> {
        let sh = &self.sh;
        sh.check()?;
        let d = *desc;
        let lim = &sh.limits;
        let max_dim = if d.depth > 1 { lim.max_texture_3d } else { lim.max_texture_2d };
        if d.width == 0 || d.height == 0 || d.depth == 0 || d.layers == 0 || d.mips == 0 {
            return Err(GpuError::Invalid("texture size, layers or mips 0"));
        }
        if d.width > max_dim || d.height > max_dim || d.depth > max_dim || d.layers > lim.max_texture_layers {
            return Err(GpuError::Invalid("texture larger than the device's limits"));
        }
        let longest = d.width.max(d.height).max(d.depth);
        if d.mips > 32 - longest.leading_zeros() {
            return Err(GpuError::Invalid("more mips than the size has"));
        }
        if d.depth > 1 && (d.layers > 1 || d.cube || d.format.is_depth()) {
            return Err(GpuError::Invalid("3D textures have one layer and a colour format"));
        }
        if d.cube && (d.layers % 6 != 0 || d.width != d.height) {
            return Err(GpuError::Invalid("cube textures are square with 6 * n layers"));
        }
        if d.usage == TextureUsage::NONE {
            return Err(GpuError::Invalid("texture without usage"));
        }
        if d.format.is_compressed() && !sh.features.texture_bc {
            return Err(GpuError::Unsupported("BC compressed textures"));
        }
        let fmt = vk_format(d.format);
        let mut fp: vk::FormatProperties = vk::Zero::zero();
        // SAFETY: plain out-parameter.
        unsafe { (sh.inst.f.get_physical_device_format_properties)(sh.pd, fmt, &mut fp) };
        let of = fp.optimal_tiling_features;
        let mut usage = 0;
        for (u, v, need) in [
            (TextureUsage::COPY_SRC, vk::IMAGE_USAGE_TRANSFER_SRC_BIT, 0),
            (TextureUsage::COPY_DST, vk::IMAGE_USAGE_TRANSFER_DST_BIT, 0),
            (TextureUsage::SAMPLED, vk::IMAGE_USAGE_SAMPLED_BIT, vk::FORMAT_FEATURE_SAMPLED_IMAGE_BIT),
            (TextureUsage::STORAGE, vk::IMAGE_USAGE_STORAGE_BIT, vk::FORMAT_FEATURE_STORAGE_IMAGE_BIT),
            (TextureUsage::COLOR_TARGET, vk::IMAGE_USAGE_COLOR_ATTACHMENT_BIT, vk::FORMAT_FEATURE_COLOR_ATTACHMENT_BIT),
            (TextureUsage::DEPTH_TARGET, vk::IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT,
             vk::FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT),
        ] {
            if d.usage.contains(u) {
                if of & need != need {
                    return Err(GpuError::Unsupported("format with this usage"));
                }
                usage |= v;
            }
        }
        if d.usage.contains(TextureUsage::COLOR_TARGET) && d.format.is_depth()
            || d.usage.contains(TextureUsage::DEPTH_TARGET) && !d.format.is_depth()
        {
            return Err(GpuError::Invalid("colour targets need colour formats, depth targets depth formats"));
        }
        // (G5d) shared with the compute queue's family, render targets excepted (they stay the device queue's)
        let fams = sh.sharing().filter(|_| !d.usage.intersects(TextureUsage::COLOR_TARGET | TextureUsage::DEPTH_TARGET));
        let ii = vk::ImageCreateInfo {
            s_type: vk::STRUCTURE_TYPE_IMAGE_CREATE_INFO,
            queue_family_index_count: if fams.is_some() { 2 } else { 0 },
            p_queue_family_indices: fams.as_ref().map_or(core::ptr::null(), |f| f.as_ptr()),
            flags: if d.cube { vk::IMAGE_CREATE_CUBE_COMPATIBLE_BIT } else { 0 },
            image_type: if d.depth > 1 { vk::IMAGE_TYPE_3D } else { vk::IMAGE_TYPE_2D },
            format: fmt,
            extent: vk::Extent3D { width: d.width, height: d.height, depth: d.depth },
            mip_levels: d.mips,
            array_layers: d.layers,
            samples: vk::SAMPLE_COUNT_1_BIT,
            tiling: vk::IMAGE_TILING_OPTIMAL,
            usage,
            sharing_mode: if fams.is_some() { vk::SHARING_MODE_CONCURRENT } else { vk::SHARING_MODE_EXCLUSIVE },
            initial_layout: vk::IMAGE_LAYOUT_UNDEFINED,
            ..vk::Zero::zero()
        };
        let mut image = 0;
        // SAFETY: a valid create info (checked above).
        sh.vk(unsafe { (sh.f.create_image)(sh.dev, &ii, core::ptr::null(), &mut image) })?;
        let mut req: vk::MemoryRequirements = vk::Zero::zero();
        // SAFETY: the image just created.
        unsafe { (sh.f.get_image_memory_requirements)(sh.dev, image, &mut req) };
        let mut st = sh.st.lock();
        let a = match st.mem.alloc(&sh.f, sh.dev, &req, MemoryKind::Device, false) {
            Ok(a) => a,
            Err(e) => {
                // SAFETY: an unbound, unused image.
                unsafe { (sh.f.destroy_image)(sh.dev, image, core::ptr::null()) };
                return Err(e);
            }
        };
        drop(st);
        // SAFETY: the allocation satisfies the image's requirements.
        if let Err(e) = sh.vk(unsafe { (sh.f.bind_image_memory)(sh.dev, image, a.mem, a.offset) }) {
            sh.defer(Garbage::Image(image, Some(a)));
            return Err(e);
        }
        let (aspect_all, aspect_copy) = aspects(d.format);
        let t = Arc::new(TexInner { sh: sh.clone(), image, a: Some(a), owned: true, desc: d, aspect_all, aspect_copy });
        let view = make_view(&t, default_dim(&d), 0, d.mips, 0, d.layers)?;
        Ok(Texture { t, view })
    }
}

/// the subresource range of mips / layers of a texture (every aspect)
pub(crate) fn subresources(t: &Texture, base_mip: u32, mips: u32, base_layer: u32, layers: u32) -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: t.t.aspect_all,
        base_mip_level: base_mip,
        level_count: mips,
        base_array_layer: base_layer,
        layer_count: layers,
    }
}

/// a buffer <-> texture copy: the region checked against the texture, the data layout against
/// the format -> (the copy with buffer_offset = layout.offset, bytes from layout.offset on);
/// data_len: the bytes behind the layout (buffer size or slice length)
pub(crate) fn copy_region(r: &TextureRegion, l: &DataLayout, data_len: u64) -> Result<(vk::BufferImageCopy, u64)> {
    let t = r.texture;
    let d = &t.t.desc;
    if r.mip >= d.mips || r.layers == 0 || r.base_layer.checked_add(r.layers).is_none_or(|e| e > d.layers) {
        return Err(GpuError::Invalid("copy: mip or layers outside the texture"));
    }
    let (mw, mh, md) = ((d.width >> r.mip).max(1), (d.height >> r.mip).max(1), (d.depth >> r.mip).max(1));
    let [x, y, z] = r.origin;
    let [w, h, dd] = r.size;
    let past = |a: u32, n: u32, m: u32| a.checked_add(n).is_none_or(|e| e > m);
    if w == 0 || h == 0 || dd == 0 || past(x, w, mw) || past(y, h, mh) || past(z, dd, md) {
        return Err(GpuError::Invalid("copy: region outside the mip"));
    }
    let f = d.format;
    let (bw, bb) = (f.block_dim(), f.block_bytes());
    if bw > 1 && (x % bw != 0 || y % bw != 0 || (w % bw != 0 && x + w != mw) || (h % bw != 0 && y + h != mh)) {
        return Err(GpuError::Invalid("copy: compressed regions are whole blocks"));
    }
    if f.has_stencil() {
        return Err(GpuError::Unsupported("copies of depth-stencil formats"));
    }
    let bx = w.div_ceil(bw) as u64;
    let by = h.div_ceil(bw) as u64;
    let row = bx * bb as u64;
    let bpr = if l.bytes_per_row == 0 { row } else { l.bytes_per_row as u64 };
    let rows = if l.rows_per_image == 0 { by } else { l.rows_per_image as u64 };
    if bpr < row || bpr % bb as u64 != 0 || rows < by {
        return Err(GpuError::Invalid("copy: bytes_per_row / rows_per_image too small or not whole texels"));
    }
    let align = if f.is_depth() { 4 } else { bb as u64 };
    if l.offset % align != 0 {
        return Err(GpuError::Invalid("copy: data offset not a multiple of the texel size"));
    }
    let slices = dd as u64 * r.layers as u64;
    // (checked: bytes_per_row and rows_per_image near u32::MAX must not wrap the span small)
    let span = bpr.checked_mul(rows).and_then(|s| s.checked_mul(slices - 1))
        .and_then(|s| s.checked_add(bpr * (by - 1) + row))
        .filter(|&s| l.offset.checked_add(s).is_some_and(|e| e <= data_len));
    let Some(span) = span else {
        return Err(GpuError::Invalid("copy: data smaller than the region"));
    };
    let copy = vk::BufferImageCopy {
        buffer_offset: l.offset,
        buffer_row_length: (bpr / bb as u64 * bw as u64) as u32,
        buffer_image_height: (rows * bw as u64) as u32,
        image_subresource: vk::ImageSubresourceLayers {
            aspect_mask: t.t.aspect_copy,
            mip_level: r.mip,
            base_array_layer: r.base_layer,
            layer_count: r.layers,
        },
        image_offset: vk::Offset3D { x: x as i32, y: y as i32, z: z as i32 },
        image_extent: vk::Extent3D { width: w, height: h, depth: dd },
    };
    Ok((copy, span))
}

// ---------------------------------------------------------------- samplers, shaders

pub(crate) struct SamplerInner {
    sh: Arc<Shared>,
    pub h: vk::Handle,
}

impl Drop for SamplerInner {
    fn drop(&mut self) {
        self.sh.defer(Garbage::Sampler(self.h));
    }
}

/// how textures are filtered and addressed
#[derive(Clone)]
pub struct Sampler(pub(crate) Arc<SamplerInner>);

impl Device {
    pub fn create_sampler(&self, desc: &SamplerDesc) -> Result<Sampler> {
        let sh = &self.sh;
        sh.check()?;
        let filter = |f: Filter| if f == Filter::Linear { vk::FILTER_LINEAR } else { vk::FILTER_NEAREST };
        let addr = |a: AddressMode| match a {
            AddressMode::Repeat => vk::SAMPLER_ADDRESS_MODE_REPEAT,
            AddressMode::MirrorRepeat => vk::SAMPLER_ADDRESS_MODE_MIRRORED_REPEAT,
            AddressMode::ClampToEdge => vk::SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
        };
        let aniso = desc.anisotropy > 1;
        if aniso && !sh.features.anisotropy {
            return Err(GpuError::Unsupported("anisotropic filtering"));
        }
        if desc.lod_min < 0.0 || desc.lod_max < desc.lod_min {
            return Err(GpuError::Invalid("sampler lod range"));
        }
        let si = vk::SamplerCreateInfo {
            s_type: vk::STRUCTURE_TYPE_SAMPLER_CREATE_INFO,
            mag_filter: filter(desc.mag),
            min_filter: filter(desc.min),
            mipmap_mode: if desc.mip == Filter::Linear { vk::SAMPLER_MIPMAP_MODE_LINEAR } else { vk::SAMPLER_MIPMAP_MODE_NEAREST },
            address_mode_u: addr(desc.address[0]),
            address_mode_v: addr(desc.address[1]),
            address_mode_w: addr(desc.address[2]),
            anisotropy_enable: aniso as u32,
            max_anisotropy: (desc.anisotropy as f32).min(sh.limits.max_anisotropy).max(1.0),
            compare_enable: desc.compare.is_some() as u32,
            compare_op: desc.compare.map_or(vk::COMPARE_OP_NEVER, compare_op),
            min_lod: desc.lod_min,
            max_lod: desc.lod_max,
            border_color: vk::BORDER_COLOR_FLOAT_TRANSPARENT_BLACK,
            ..vk::Zero::zero()
        };
        let mut h = 0;
        // SAFETY: valid create info.
        sh.vk(unsafe { (sh.f.create_sampler)(sh.dev, &si, core::ptr::null(), &mut h) })?;
        Ok(Sampler(Arc::new(SamplerInner { sh: sh.clone(), h })))
    }
}

pub(crate) struct ShaderInner {
    sh: Arc<Shared>,
    pub h: vk::Handle,
    /// the words (for the workgroup size once the pipeline constants are known)
    words: Vec<u32>,
    entries: Vec<shader::reflect::Entry>,
    spec: Vec<shader::reflect::SpecConst>,
    local_size_id: bool,
}

impl Drop for ShaderInner {
    fn drop(&mut self) {
        self.sh.defer(Garbage::Shader(self.h));
    }
}

/// compiled shader code (SPIR-V)
#[derive(Clone)]
pub struct ShaderModule(pub(crate) Arc<ShaderInner>);

impl ShaderModule {
    /// a module from SPIR-V words: version up to 1.3 on Vulkan 1.1 devices, 1.5 on 1.2, 1.6 on
    /// 1.3 (the magic number and version are checked; the code itself is the driver's)
    pub fn from_spirv(device: &Device, code: &[u32]) -> Result<ShaderModule> {
        let sh = &device.sh;
        sh.check()?;
        if code.len() < 5 || code[0] != 0x0723_0203 {
            return Err(GpuError::Invalid("not SPIR-V (magic number)"));
        }
        let (maj, min) = (sh.info.api_version.0, sh.info.api_version.1);
        let max = match (maj, min) {
            (1, 1) => 0x0001_0300,
            (1, 2) => 0x0001_0500,
            _ => 0x0001_0600,
        };
        if code[1] > max || code[1] < 0x0001_0000 {
            return Err(GpuError::Unsupported("this SPIR-V version"));
        }
        let entries = shader::reflect::entry_points(code).map_err(|_| GpuError::Invalid("SPIR-V instructions do not fit the words"))?;
        let spec = shader::reflect::spec_constants(code).map_err(|_| GpuError::Invalid("SpecId on something else than a scalar specialization constant"))?;
        if spec.iter().enumerate().any(|(i, a)| spec[..i].iter().any(|b| b.spec_id == a.spec_id)) {
            return Err(GpuError::Invalid("a SpecId twice in the module"));
        }
        let ci = vk::ShaderModuleCreateInfo {
            s_type: vk::STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
            code_size: code.len() * 4,
            p_code: code.as_ptr(),
            ..vk::Zero::zero()
        };
        let mut h = 0;
        // SAFETY: code is a SPIR-V word array of code_size bytes.
        sh.vk(unsafe { (sh.f.create_shader_module)(sh.dev, &ci, core::ptr::null(), &mut h) })?;
        let local_size_id = shader::reflect::uses_local_size_id(code);
        Ok(ShaderModule(Arc::new(ShaderInner { sh: sh.clone(), h, words: code.to_vec(), entries, spec, local_size_id })))
    }

    /// the entry points: (model as SPIR-V numbers it: 0 vertex, 4 fragment, 5 compute, name)
    pub fn entry_points(&self) -> Vec<(u32, &str)> {
        self.0.entries.iter().map(|e| (e.model, e.name.as_str())).collect()
    }

    /// the pipeline-overridable constants the module declares: (id, kind, default value)
    pub fn constants(&self) -> Vec<(u32, shader::reflect::Kind, f64)> {
        self.0.spec.iter().map(|c| (c.spec_id, c.kind, kind_value(c.kind, c.default))).collect()
    }
}

/// the bits of a constant of kind k as f64
fn kind_value(k: shader::reflect::Kind, v: u64) -> f64 {
    use shader::reflect::Kind;
    match k {
        Kind::Bool => (v != 0) as u32 as f64,
        Kind::Int { width, signed: true } => {
            let s = 64 - width.min(64);
            (((v << s) as i64) >> s) as f64
        }
        Kind::Int { .. } => v as f64,
        Kind::Float { width: 16 } => shader::reflect::f16_to_f32(v as u16) as f64,
        Kind::Float { width: 32 } => f32::from_bits(v as u32) as f64,
        Kind::Float { .. } => f64::from_bits(v),
    }
}

/// a pipeline constant's value as the bits of kind k (WebGPU's conversions, lib.rs Constant)
fn constant_bits(k: shader::reflect::Kind, x: f64) -> Result<u64> {
    use shader::reflect::Kind;
    let bad = GpuError::Invalid("pipeline constant: the value does not fit the override's type");
    match k {
        Kind::Bool => Ok((x != 0.0 && !x.is_nan()) as u64),
        Kind::Int { width, signed } => {
            if !x.is_finite() {
                return Err(bad);
            }
            let t = x as i128; // toward 0 (saturating: out of range anyway)
            let w = width.clamp(1, 64);
            let (lo, hi) = if signed { (-(1i128 << (w - 1)), (1i128 << (w - 1)) - 1) } else { (0, (1i128 << w) - 1) };
            if t < lo || t > hi {
                return Err(bad);
            }
            let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
            Ok(t as u64 & m)
        }
        Kind::Float { width } => {
            if !x.is_finite() {
                return Err(bad);
            }
            match width {
                16 => {
                    let h = shader::reflect::f16_from_f32(x as f32);
                    if h & 0x7c00 == 0x7c00 || !(x as f32).is_finite() {
                        return Err(bad);
                    }
                    Ok(h as u64)
                }
                32 => {
                    let f = x as f32;
                    if !f.is_finite() {
                        return Err(bad);
                    }
                    Ok(f.to_bits() as u64)
                }
                _ => Ok(x.to_bits()),
            }
        }
    }
}

/// a stage's pipeline constants checked against its module -> (map entries, data,
/// (SpecId, bits) for shader::reflect)
fn specialize(m: &ShaderInner, constants: &[Constant]) -> Result<(Vec<vk::SpecializationMapEntry>, Vec<u8>, Vec<(u32, u64)>)> {
    let mut map = Vec::with_capacity(constants.len());
    let mut data = Vec::new();
    let mut set = Vec::with_capacity(constants.len());
    for (i, c) in constants.iter().enumerate() {
        if constants[..i].iter().any(|d| d.id == c.id) {
            return Err(GpuError::Invalid("pipeline constant: an id twice"));
        }
        let Some(decl) = m.spec.iter().find(|s| s.spec_id == c.id) else {
            return Err(GpuError::Invalid("pipeline constant: the module has no override with this id"));
        };
        let bits = constant_bits(decl.kind, c.value)?;
        let size = match decl.kind {
            shader::reflect::Kind::Bool => 4, // VkBool32
            shader::reflect::Kind::Int { width, .. } | shader::reflect::Kind::Float { width } => (width as usize).div_ceil(8),
        };
        map.push(vk::SpecializationMapEntry { constant_id: c.id, offset: data.len() as u32, size });
        data.extend_from_slice(&bits.to_le_bytes()[..size]);
        while data.len() % 4 != 0 {
            data.push(0);
        }
        set.push((c.id, bits));
    }
    Ok((map, data, set))
}

/// the checks of a stage (its entry point exists with model; the constants) -> what
/// VkPipelineShaderStageCreateInfo needs
fn stage_info(sh: &Shared, st: &ShaderStage, model: shader::Model) -> Result<(Vec<vk::SpecializationMapEntry>, Vec<u8>, Vec<(u32, u64)>)> {
    let m = &st.module.0;
    if !m.entries.iter().any(|e| e.name == st.entry && e.model == model as u32) {
        return Err(GpuError::Invalid("the module has no entry point of this name for the stage"));
    }
    if model == shader::Model::GLCompute && m.local_size_id && !sh.maintenance4 {
        return Err(GpuError::Unsupported("LocalSizeId (maintenance4, Vulkan 1.3)"));
    }
    specialize(m, st.constants)
}

// ---------------------------------------------------------------- bindings

pub(crate) struct BglInner {
    sh: Arc<Shared>,
    pub h: vk::Handle,
    pub bindings: Vec<Binding>, // sorted by binding (empty for a bindless layout)
    needs: Vec<vk::DescriptorPoolSize>,
}

impl Drop for BglInner {
    fn drop(&mut self) {
        self.sh.defer(Garbage::SetLayout(self.h));
    }
}

/// the bindings of one bind group (@group)
#[derive(Clone)]
pub struct BindGroupLayout(pub(crate) Arc<BglInner>);

impl Device {
    pub fn create_bind_group_layout(&self, bindings: &[Binding]) -> Result<BindGroupLayout> {
        let sh = &self.sh;
        sh.check()?;
        let mut b: Vec<Binding> = bindings.to_vec();
        b.sort_by_key(|x| x.binding);
        if b.windows(2).any(|w| w[0].binding == w[1].binding) {
            return Err(GpuError::Invalid("a binding number twice"));
        }
        for x in &b {
            if stage_bits(x.stages) == 0 {
                return Err(GpuError::Invalid("binding without stages"));
            }
            if x.kind == BindingKind::StorageBuffer && x.stages.intersects(Stages::VERTEX) && !sh.features.vertex_stores {
                return Err(GpuError::Unsupported("writable storage buffers in vertex shaders"));
            }
        }
        let vb: Vec<vk::DescriptorSetLayoutBinding> = b
            .iter()
            .map(|x| vk::DescriptorSetLayoutBinding {
                binding: x.binding,
                descriptor_type: descriptor_type(x.kind),
                descriptor_count: 1,
                stage_flags: stage_bits(x.stages),
                p_immutable_samplers: core::ptr::null(),
            })
            .collect();
        let ci = vk::DescriptorSetLayoutCreateInfo {
            s_type: vk::STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
            binding_count: vb.len() as u32,
            p_bindings: vb.as_ptr(),
            ..vk::Zero::zero()
        };
        let mut h = 0;
        // SAFETY: valid create info.
        sh.vk(unsafe { (sh.f.create_descriptor_set_layout)(sh.dev, &ci, core::ptr::null(), &mut h) })?;
        let needs = vb.iter().map(|x| vk::DescriptorPoolSize { type_: x.descriptor_type, descriptor_count: 1 }).collect();
        Ok(BindGroupLayout(Arc::new(BglInner { sh: sh.clone(), h, bindings: b, needs })))
    }
}

pub(crate) struct PlInner {
    sh: Arc<Shared>,
    pub h: vk::Handle,
    pub sets: Vec<vk::Handle>,
    pub push: u32,
    _groups: Vec<BindGroupLayout>,
}

impl Drop for PlInner {
    fn drop(&mut self) {
        self.sh.defer(Garbage::PipeLayout(self.h));
    }
}

/// the bind group layouts and push constants of pipelines
#[derive(Clone)]
pub struct PipelineLayout(pub(crate) Arc<PlInner>);

impl Device {
    pub fn create_pipeline_layout(&self, desc: &PipelineLayoutDesc) -> Result<PipelineLayout> {
        let sh = &self.sh;
        sh.check()?;
        if desc.groups.len() > BIND_GROUPS_MAX as usize || desc.groups.len() > sh.raw.max_bound_descriptor_sets as usize {
            return Err(GpuError::Invalid("too many bind groups"));
        }
        if desc.push_constants % 4 != 0 || desc.push_constants > PUSH_CONSTANTS_MAX {
            return Err(GpuError::Invalid("push constants: a multiple of 4, at most PUSH_CONSTANTS_MAX"));
        }
        let sets: Vec<vk::Handle> = desc.groups.iter().map(|g| g.0.h).collect();
        let range = vk::PushConstantRange { stage_flags: vk::SHADER_STAGE_ALL, offset: 0, size: desc.push_constants };
        let ci = vk::PipelineLayoutCreateInfo {
            s_type: vk::STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
            set_layout_count: sets.len() as u32,
            p_set_layouts: sets.as_ptr(),
            push_constant_range_count: (desc.push_constants > 0) as u32,
            p_push_constant_ranges: &range,
            ..vk::Zero::zero()
        };
        let mut h = 0;
        // SAFETY: valid create info; the set layouts are alive.
        sh.vk(unsafe { (sh.f.create_pipeline_layout)(sh.dev, &ci, core::ptr::null(), &mut h) })?;
        Ok(PipelineLayout(Arc::new(PlInner {
            sh: sh.clone(),
            h,
            sets,
            push: desc.push_constants,
            _groups: desc.groups.iter().map(|g| (*g).clone()).collect(),
        })))
    }
}

/// what a bind group keeps alive (held for their drop only)
#[derive(Clone)]
#[allow(dead_code)]
enum Keep {
    Buf(Buffer),
    View(TextureView),
    Sampler(Sampler),
}

pub(crate) struct BgInner {
    sh: Arc<Shared>,
    pub set: vk::Handle,
    pool: vk::Handle,
    own_pool: bool,
    pub layout: vk::Handle,
    _layout: BindGroupLayout,
    _keep: Vec<Keep>,
}

impl Drop for BgInner {
    fn drop(&mut self) {
        if self.own_pool {
            self.sh.defer(Garbage::Pool(self.pool));
        } else {
            self.sh.defer(Garbage::Set(self.pool, self.set));
        }
    }
}

/// resources bound for shaders (a descriptor set)
#[derive(Clone)]
pub struct BindGroup(pub(crate) Arc<BgInner>);

impl Device {
    pub fn create_bind_group(&self, desc: &BindGroupDesc) -> Result<BindGroup> {
        let sh = &self.sh;
        sh.check()?;
        let lay = &desc.layout.0;
        if desc.entries.len() != lay.bindings.len() {
            return Err(GpuError::Invalid("bind group: one entry per binding of the layout"));
        }
        let mut keep = Vec::with_capacity(desc.entries.len());
        let mut bufs: Vec<vk::DescriptorBufferInfo> = Vec::with_capacity(desc.entries.len());
        let mut imgs: Vec<vk::DescriptorImageInfo> = Vec::with_capacity(desc.entries.len());
        let mut which: Vec<(u32, u32, bool, usize)> = Vec::with_capacity(desc.entries.len()); // binding, type, buffer?, index
        for e in desc.entries {
            let Some(b) = lay.bindings.iter().find(|b| b.binding == e.binding) else {
                return Err(GpuError::Invalid("bind group: entry for a binding the layout lacks"));
            };
            if which.iter().any(|w| w.0 == e.binding) {
                return Err(GpuError::Invalid("bind group: a binding twice"));
            }
            let ty = descriptor_type(b.kind);
            match (b.kind, e.resource) {
                (BindingKind::UniformBuffer | BindingKind::StorageBuffer | BindingKind::StorageBufferRead,
                 Resource::Buffer { buffer, offset, size }) => {
                    let uniform = b.kind == BindingKind::UniformBuffer;
                    let (need, align, max) = if uniform {
                        (BufferUsage::UNIFORM, sh.limits.min_uniform_offset_alignment, sh.limits.max_uniform_binding)
                    } else {
                        (BufferUsage::STORAGE, sh.limits.min_storage_offset_alignment, sh.limits.max_storage_binding)
                    };
                    if !buffer.usage().contains(need) {
                        return Err(GpuError::Invalid("bind group: buffer lacks UNIFORM / STORAGE usage"));
                    }
                    let range = if size == 0 { buffer.size().saturating_sub(offset) } else { size };
                    if offset % align.max(1) as u64 != 0 || range == 0 || offset.checked_add(range).is_none_or(|e| e > buffer.size()) || range > max as u64 {
                        return Err(GpuError::Invalid("bind group: buffer range (alignment, size, limit)"));
                    }
                    which.push((e.binding, ty, true, bufs.len()));
                    bufs.push(vk::DescriptorBufferInfo { buffer: buffer.handle(), offset, range });
                    keep.push(Keep::Buf(buffer.clone()));
                }
                (BindingKind::Texture, Resource::Texture(v)) | (BindingKind::StorageTexture, Resource::StorageTexture(v)) => {
                    let storage = b.kind == BindingKind::StorageTexture;
                    let need = if storage { TextureUsage::STORAGE } else { TextureUsage::SAMPLED };
                    if !v.usage().contains(need) {
                        return Err(GpuError::Invalid("bind group: texture lacks SAMPLED / STORAGE usage"));
                    }
                    which.push((e.binding, ty, false, imgs.len()));
                    imgs.push(vk::DescriptorImageInfo {
                        sampler: 0,
                        image_view: v.0.h,
                        image_layout: if storage { vk::IMAGE_LAYOUT_GENERAL } else { vk::IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL },
                    });
                    keep.push(Keep::View(v.clone()));
                }
                (BindingKind::Sampler, Resource::Sampler(s)) => {
                    which.push((e.binding, ty, false, imgs.len()));
                    imgs.push(vk::DescriptorImageInfo { sampler: s.0.h, image_view: 0, image_layout: 0 });
                    keep.push(Keep::Sampler(s.clone()));
                }
                _ => return Err(GpuError::Invalid("bind group: resource does not match the binding's kind")),
            }
        }
        let (pool, set) = sh.alloc_set(lay.h, &lay.needs)?;
        let writes: Vec<vk::WriteDescriptorSet> = which
            .iter()
            .map(|&(binding, ty, is_buf, i)| vk::WriteDescriptorSet {
                s_type: vk::STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
                dst_set: set,
                dst_binding: binding,
                descriptor_count: 1,
                descriptor_type: ty,
                p_image_info: if is_buf { core::ptr::null() } else { &imgs[i] },
                p_buffer_info: if is_buf { &bufs[i] } else { core::ptr::null() },
                ..vk::Zero::zero()
            })
            .collect();
        // SAFETY: a fresh set no command buffer uses; the infos live until the call returns.
        unsafe { (sh.f.update_descriptor_sets)(sh.dev, writes.len() as u32, writes.as_ptr(), 0, core::ptr::null()) };
        Ok(BindGroup(Arc::new(BgInner {
            sh: sh.clone(),
            set,
            pool,
            own_pool: false,
            layout: lay.h,
            _layout: desc.layout.clone(),
            _keep: keep,
        })))
    }
}

// ---------------------------------------------------------------- bindless (Voll)

/// slots of a Bindless table, by binding
struct Slots {
    keep: [Vec<Option<Keep>>; 4],
}

/// a large table of textures, samplers and buffers that shaders index (level Voll): binding 0
/// sampled textures, 1 samplers, 2 storage buffers, 3 storage textures; slots may be changed
/// while command buffers that use the table are pending, as long as those do not use the slot
pub struct Bindless {
    group: BindGroup,
    layout: BindGroupLayout,
    counts: [u32; 4],
    slots: SpinLock<Slots>,
}

impl Device {
    pub fn create_bindless(&self, desc: &BindlessDesc) -> Result<Bindless> {
        let sh = &self.sh;
        sh.check()?;
        if !sh.features.bindless {
            return Err(GpuError::Unsupported("bindless tables (level Voll)"));
        }
        let l = &sh.limits;
        let counts = [desc.textures, desc.samplers, desc.buffers, desc.storage_textures];
        let maxes = [l.max_bindless_textures, l.max_bindless_samplers, l.max_bindless_buffers, l.max_bindless_storage_textures];
        if counts.iter().zip(maxes.iter()).any(|(c, m)| c > m) {
            return Err(GpuError::Invalid("bindless: more slots than Limits::max_bindless_*"));
        }
        let types = [vk::DESCRIPTOR_TYPE_SAMPLED_IMAGE, vk::DESCRIPTOR_TYPE_SAMPLER, vk::DESCRIPTOR_TYPE_STORAGE_BUFFER,
                     vk::DESCRIPTOR_TYPE_STORAGE_IMAGE];
        let vb: Vec<vk::DescriptorSetLayoutBinding> = (0..4)
            .filter(|&i| counts[i] > 0)
            .map(|i| vk::DescriptorSetLayoutBinding {
                binding: i as u32,
                descriptor_type: types[i],
                descriptor_count: counts[i],
                stage_flags: vk::SHADER_STAGE_ALL,
                p_immutable_samplers: core::ptr::null(),
            })
            .collect();
        if vb.is_empty() {
            return Err(GpuError::Invalid("bindless table without slots"));
        }
        let flags = vec![vk::DESCRIPTOR_BINDING_UPDATE_AFTER_BIND_BIT | vk::DESCRIPTOR_BINDING_UPDATE_UNUSED_WHILE_PENDING_BIT
            | vk::DESCRIPTOR_BINDING_PARTIALLY_BOUND_BIT; vb.len()];
        let fi = vk::DescriptorSetLayoutBindingFlagsCreateInfo {
            s_type: vk::STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_BINDING_FLAGS_CREATE_INFO,
            binding_count: flags.len() as u32,
            p_binding_flags: flags.as_ptr(),
            ..vk::Zero::zero()
        };
        let ci = vk::DescriptorSetLayoutCreateInfo {
            s_type: vk::STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
            p_next: &fi as *const _ as *const c_void,
            flags: vk::DESCRIPTOR_SET_LAYOUT_CREATE_UPDATE_AFTER_BIND_POOL_BIT,
            binding_count: vb.len() as u32,
            p_bindings: vb.as_ptr(),
        };
        let mut lh = 0;
        // SAFETY: valid create info with its chain.
        sh.vk(unsafe { (sh.f.create_descriptor_set_layout)(sh.dev, &ci, core::ptr::null(), &mut lh) })?;
        let layout = BindGroupLayout(Arc::new(BglInner { sh: sh.clone(), h: lh, bindings: Vec::new(), needs: Vec::new() }));
        let sizes: Vec<vk::DescriptorPoolSize> =
            vb.iter().map(|b| vk::DescriptorPoolSize { type_: b.descriptor_type, descriptor_count: b.descriptor_count }).collect();
        let pi = vk::DescriptorPoolCreateInfo {
            s_type: vk::STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
            flags: vk::DESCRIPTOR_POOL_CREATE_UPDATE_AFTER_BIND_BIT,
            max_sets: 1,
            pool_size_count: sizes.len() as u32,
            p_pool_sizes: sizes.as_ptr(),
            ..vk::Zero::zero()
        };
        let mut pool = 0;
        // SAFETY: valid create info.
        sh.vk(unsafe { (sh.f.create_descriptor_pool)(sh.dev, &pi, core::ptr::null(), &mut pool) })?;
        let ai = vk::DescriptorSetAllocateInfo {
            s_type: vk::STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
            descriptor_pool: pool,
            descriptor_set_count: 1,
            p_set_layouts: &lh,
            ..vk::Zero::zero()
        };
        let mut set = 0;
        // SAFETY: one set of the layout from its own pool.
        if let Err(e) = sh.vk(unsafe { (sh.f.allocate_descriptor_sets)(sh.dev, &ai, &mut set) }) {
            sh.defer(Garbage::Pool(pool));
            return Err(e);
        }
        let group = BindGroup(Arc::new(BgInner {
            sh: sh.clone(),
            set,
            pool,
            own_pool: true,
            layout: lh,
            _layout: layout.clone(),
            _keep: Vec::new(),
        }));
        let slots = Slots { keep: counts.map(|n| vec![None; n as usize]) };
        Ok(Bindless { group, layout, counts, slots: SpinLock::new(slots) })
    }
}

impl Bindless {
    /// the table as a bind group (CommandEncoder::set_bind_group)
    pub fn group(&self) -> &BindGroup {
        &self.group
    }
    /// its layout (PipelineLayoutDesc::groups)
    pub fn layout(&self) -> &BindGroupLayout {
        &self.layout
    }
    /// slots per binding: textures, samplers, buffers, storage textures
    pub fn counts(&self) -> [u32; 4] {
        self.counts
    }

    fn write(&self, binding: usize, slot: u32, keep: Keep, img: Option<vk::DescriptorImageInfo>,
             buf: Option<vk::DescriptorBufferInfo>) -> Result<()> {
        let sh = &self.group.0.sh;
        sh.check()?;
        if slot >= self.counts[binding] {
            return Err(GpuError::Invalid("bindless: slot outside the table"));
        }
        let types = [vk::DESCRIPTOR_TYPE_SAMPLED_IMAGE, vk::DESCRIPTOR_TYPE_SAMPLER, vk::DESCRIPTOR_TYPE_STORAGE_BUFFER,
                     vk::DESCRIPTOR_TYPE_STORAGE_IMAGE];
        let w = vk::WriteDescriptorSet {
            s_type: vk::STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
            dst_set: self.group.0.set,
            dst_binding: binding as u32,
            dst_array_element: slot,
            descriptor_count: 1,
            descriptor_type: types[binding],
            p_image_info: img.as_ref().map_or(core::ptr::null(), |i| i as *const _),
            p_buffer_info: buf.as_ref().map_or(core::ptr::null(), |b| b as *const _),
            ..vk::Zero::zero()
        };
        let mut slots = self.slots.lock();
        // SAFETY: the set is update-after-bind; the lock serialises updates of it.
        unsafe { (sh.f.update_descriptor_sets)(sh.dev, 1, &w, 0, core::ptr::null()) };
        slots.keep[binding][slot as usize] = Some(keep); // the old resource is destroyed later
        Ok(())
    }

    /// slot of binding 0: a sampled texture (Layout::Sampled when shaders read it)
    pub fn set_texture(&self, slot: u32, view: &TextureView) -> Result<()> {
        if !view.usage().contains(TextureUsage::SAMPLED) {
            return Err(GpuError::Invalid("bindless: texture lacks SAMPLED usage"));
        }
        let i = vk::DescriptorImageInfo { sampler: 0, image_view: view.0.h, image_layout: vk::IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL };
        self.write(0, slot, Keep::View(view.clone()), Some(i), None)
    }

    /// slot of binding 1: a sampler
    pub fn set_sampler(&self, slot: u32, s: &Sampler) -> Result<()> {
        let i = vk::DescriptorImageInfo { sampler: s.0.h, image_view: 0, image_layout: 0 };
        self.write(1, slot, Keep::Sampler(s.clone()), Some(i), None)
    }

    /// slot of binding 2: a storage buffer range (size 0: to the end)
    pub fn set_buffer(&self, slot: u32, buf: &Buffer, offset: u64, size: u64) -> Result<()> {
        let sh = &self.group.0.sh;
        let range = if size == 0 { buf.size().saturating_sub(offset) } else { size };
        let align = sh.limits.min_storage_offset_alignment.max(1) as u64;
        if !buf.usage().contains(BufferUsage::STORAGE) || offset % align != 0 || range == 0 || offset.checked_add(range).is_none_or(|e| e > buf.size()) {
            return Err(GpuError::Invalid("bindless: buffer range (usage, alignment, size)"));
        }
        let b = vk::DescriptorBufferInfo { buffer: buf.handle(), offset, range };
        self.write(2, slot, Keep::Buf(buf.clone()), None, Some(b))
    }

    /// slot of binding 3: a storage texture (Layout::General when shaders use it)
    pub fn set_storage_texture(&self, slot: u32, view: &TextureView) -> Result<()> {
        if !view.usage().contains(TextureUsage::STORAGE) {
            return Err(GpuError::Invalid("bindless: texture lacks STORAGE usage"));
        }
        let i = vk::DescriptorImageInfo { sampler: 0, image_view: view.0.h, image_layout: vk::IMAGE_LAYOUT_GENERAL };
        self.write(3, slot, Keep::View(view.clone()), Some(i), None)
    }
}

// ---------------------------------------------------------------- pipelines

pub(crate) struct PipeInner {
    sh: Arc<Shared>,
    pub h: vk::Handle,
    pub layout: PipelineLayout,
    pub colors: [u32; COLOR_TARGETS_MAX],
    pub ncolor: usize,
    pub depth: u32,
}

impl Drop for PipeInner {
    fn drop(&mut self) {
        self.sh.defer(Garbage::Pipeline(self.h));
    }
}

/// a compute shader with its layout
#[derive(Clone)]
pub struct ComputePipeline(pub(crate) Arc<PipeInner>);

/// vertex + fragment shaders with their fixed state and target formats
#[derive(Clone)]
pub struct RenderPipeline(pub(crate) Arc<PipeInner>);

impl Device {
    pub fn create_compute_pipeline(&self, desc: &ComputePipelineDesc) -> Result<ComputePipeline> {
        self.compute_pipeline(desc, 0)
    }

    /// create_compute_pipeline with a required subgroup size (WGSL @subgroup_size, what
    /// wgsl::Output::subgroup_size returns): Features::subgroup_size_control, a power of two in
    /// Limits::subgroup_min_size ..= subgroup_max_size; size 0: the driver's choice
    pub fn create_compute_pipeline_sized(&self, desc: &ComputePipelineDesc, size: u32) -> Result<ComputePipeline> {
        if size != 0 {
            if !self.sh.features.subgroup_size_control {
                return Err(GpuError::Unsupported("subgroup size control"));
            }
            let lim = &self.sh.limits;
            if !size.is_power_of_two() || size < lim.subgroup_min_size || size > lim.subgroup_max_size {
                return Err(GpuError::Invalid("the required subgroup size is not a power of two within Limits::subgroup_min_size ..= subgroup_max_size"));
            }
        }
        self.compute_pipeline(desc, size)
    }

    fn compute_pipeline(&self, desc: &ComputePipelineDesc, size: u32) -> Result<ComputePipeline> {
        let sh = &self.sh;
        sh.check()?;
        let (map, data, set) = stage_info(sh, &desc.shader, shader::Model::GLCompute)?;
        // the workgroup size once specialized: at least 1, within the limits (WebGPU checks it here too)
        let wg = shader::reflect::workgroup_size(&desc.shader.module.0.words, desc.shader.entry, &set)
            .map_err(|_| GpuError::Invalid("pipeline constants: a specialization constant is undefined (division by 0, shift)"))?;
        let Some(wg) = wg else { return Err(GpuError::Invalid("compute entry point without a workgroup size")) };
        let lim = &sh.limits;
        let count = wg.iter().map(|&x| x as u64).product::<u64>();
        if wg.contains(&0) || (0..3).any(|i| wg[i] > lim.max_compute_workgroup_size[i]) || count > lim.max_compute_invocations as u64 {
            return Err(GpuError::Invalid("workgroup size 0 or beyond Limits::max_compute_workgroup_size / max_compute_invocations"));
        }
        let si = vk::SpecializationInfo {
            map_entry_count: map.len() as u32,
            p_map_entries: map.as_ptr(),
            data_size: data.len(),
            p_data: data.as_ptr() as *const c_void,
        };
        let name = nul_name(desc.shader.entry);
        if size != 0 && wg[0] % size != 0 {
            return Err(GpuError::Invalid("the workgroup size in x is not a multiple of the required subgroup size"));
        }
        let required = vk::PipelineShaderStageRequiredSubgroupSizeCreateInfo {
            s_type: vk::STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_REQUIRED_SUBGROUP_SIZE_CREATE_INFO,
            p_next: core::ptr::null_mut(),
            required_subgroup_size: size,
        };
        let ci = vk::ComputePipelineCreateInfo {
            s_type: vk::STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO,
            stage: vk::PipelineShaderStageCreateInfo {
                s_type: vk::STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: if size != 0 { &required as *const _ as *const c_void } else { core::ptr::null() },
                stage: vk::SHADER_STAGE_COMPUTE_BIT,
                module: desc.shader.module.0.h,
                p_name: name.as_ptr(),
                p_specialization_info: if map.is_empty() { core::ptr::null() } else { &si as *const _ as *const c_void },
                ..vk::Zero::zero()
            },
            layout: desc.layout.0.h,
            base_pipeline_index: -1,
            ..vk::Zero::zero()
        };
        let mut h = 0;
        // SAFETY: valid create info; module and layout are alive.
        sh.vk(unsafe { (sh.f.create_compute_pipelines)(sh.dev, 0, 1, &ci, core::ptr::null(), &mut h) })?;
        Ok(ComputePipeline(Arc::new(PipeInner {
            sh: sh.clone(),
            h,
            layout: desc.layout.clone(),
            colors: [0; COLOR_TARGETS_MAX],
            ncolor: 0,
            depth: 0,
        })))
    }

    pub fn create_render_pipeline(&self, desc: &RenderPipelineDesc) -> Result<RenderPipeline> {
        let sh = &self.sh;
        sh.check()?;
        if desc.targets.len() > sh.limits.max_color_targets as usize {
            return Err(GpuError::Invalid("too many colour targets"));
        }
        if desc.targets.iter().any(|t| t.format.is_depth() || t.format.is_compressed()) {
            return Err(GpuError::Invalid("colour target with a depth or compressed format"));
        }
        if desc.depth.is_some_and(|d| !d.format.is_depth()) {
            return Err(GpuError::Invalid("depth state with a colour format"));
        }
        let (vmap, vdata, _) = stage_info(sh, &desc.vertex, shader::Model::Vertex)?;
        let (fmap, fdata, _) = match &desc.fragment {
            Some(fs) => stage_info(sh, fs, shader::Model::Fragment)?,
            None => (Vec::new(), Vec::new(), Vec::new()),
        };
        let spec = |map: &Vec<vk::SpecializationMapEntry>, data: &Vec<u8>| vk::SpecializationInfo {
            map_entry_count: map.len() as u32,
            p_map_entries: map.as_ptr(),
            data_size: data.len(),
            p_data: data.as_ptr() as *const c_void,
        };
        let (vsi, fsi) = (spec(&vmap, &vdata), spec(&fmap, &fdata));
        let ptr = |map: &Vec<vk::SpecializationMapEntry>, si: &vk::SpecializationInfo| {
            if map.is_empty() { core::ptr::null() } else { si as *const _ as *const c_void }
        };
        let vname = nul_name(desc.vertex.entry);
        let fname = nul_name(desc.fragment.map_or("", |f| f.entry));
        let mut stages = vec![vk::PipelineShaderStageCreateInfo {
            s_type: vk::STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
            stage: vk::SHADER_STAGE_VERTEX_BIT,
            module: desc.vertex.module.0.h,
            p_name: vname.as_ptr(),
            p_specialization_info: ptr(&vmap, &vsi),
            ..vk::Zero::zero()
        }];
        if let Some(fs) = desc.fragment {
            stages.push(vk::PipelineShaderStageCreateInfo {
                s_type: vk::STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                stage: vk::SHADER_STAGE_FRAGMENT_BIT,
                module: fs.module.0.h,
                p_name: fname.as_ptr(),
                p_specialization_info: ptr(&fmap, &fsi),
                ..vk::Zero::zero()
            });
        }
        let mut vbind = Vec::new();
        let mut vattr = Vec::new();
        for (i, b) in desc.buffers.iter().enumerate() {
            vbind.push(vk::VertexInputBindingDescription {
                binding: i as u32,
                stride: b.stride,
                input_rate: if b.per_instance { vk::VERTEX_INPUT_RATE_INSTANCE } else { vk::VERTEX_INPUT_RATE_VERTEX },
            });
            for a in b.attributes {
                vattr.push(vk::VertexInputAttributeDescription {
                    location: a.location,
                    binding: i as u32,
                    format: match a.format {
                        VertexFormat::F32 => vk::FORMAT_R32_SFLOAT,
                        VertexFormat::F32x2 => vk::FORMAT_R32G32_SFLOAT,
                        VertexFormat::F32x3 => vk::FORMAT_R32G32B32_SFLOAT,
                        VertexFormat::F32x4 => vk::FORMAT_R32G32B32A32_SFLOAT,
                        VertexFormat::U32 => vk::FORMAT_R32_UINT,
                        VertexFormat::U32x2 => vk::FORMAT_R32G32_UINT,
                        VertexFormat::U32x4 => vk::FORMAT_R32G32B32A32_UINT,
                        VertexFormat::Unorm8x4 => vk::FORMAT_R8G8B8A8_UNORM,
                    },
                    offset: a.offset,
                });
            }
        }
        if vbind.len() > sh.limits.max_vertex_buffers as usize || vattr.len() > sh.limits.max_vertex_attributes as usize {
            return Err(GpuError::Invalid("too many vertex buffers or attributes"));
        }
        let vi = vk::PipelineVertexInputStateCreateInfo {
            s_type: vk::STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
            vertex_binding_description_count: vbind.len() as u32,
            p_vertex_binding_descriptions: vbind.as_ptr(),
            vertex_attribute_description_count: vattr.len() as u32,
            p_vertex_attribute_descriptions: vattr.as_ptr(),
            ..vk::Zero::zero()
        };
        let ia = vk::PipelineInputAssemblyStateCreateInfo {
            s_type: vk::STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
            topology: match desc.topology {
                Topology::Points => vk::PRIMITIVE_TOPOLOGY_POINT_LIST,
                Topology::Lines => vk::PRIMITIVE_TOPOLOGY_LINE_LIST,
                Topology::LineStrip => vk::PRIMITIVE_TOPOLOGY_LINE_STRIP,
                Topology::Triangles => vk::PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
                Topology::TriangleStrip => vk::PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP,
            },
            ..vk::Zero::zero()
        };
        let vp = vk::PipelineViewportStateCreateInfo {
            s_type: vk::STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
            viewport_count: 1,
            scissor_count: 1,
            ..vk::Zero::zero()
        };
        let rs = vk::PipelineRasterizationStateCreateInfo {
            s_type: vk::STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
            polygon_mode: vk::POLYGON_MODE_FILL,
            cull_mode: match desc.cull {
                Cull::None => vk::CULL_MODE_NONE,
                Cull::Front => vk::CULL_MODE_FRONT_BIT,
                Cull::Back => vk::CULL_MODE_BACK_BIT,
            },
            front_face: if desc.front_cw { vk::FRONT_FACE_CLOCKWISE } else { vk::FRONT_FACE_COUNTER_CLOCKWISE },
            line_width: 1.0,
            ..vk::Zero::zero()
        };
        let ms = vk::PipelineMultisampleStateCreateInfo {
            s_type: vk::STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
            rasterization_samples: vk::SAMPLE_COUNT_1_BIT,
            min_sample_shading: 1.0,
            ..vk::Zero::zero()
        };
        let ds = vk::PipelineDepthStencilStateCreateInfo {
            s_type: vk::STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO,
            depth_test_enable: desc.depth.is_some() as u32,
            depth_write_enable: desc.depth.is_some_and(|d| d.write) as u32,
            depth_compare_op: desc.depth.map_or(vk::COMPARE_OP_ALWAYS, |d| compare_op(d.compare)),
            max_depth_bounds: 1.0,
            ..vk::Zero::zero()
        };
        let factor = |f: BlendFactor| match f {
            BlendFactor::Zero => vk::BLEND_FACTOR_ZERO,
            BlendFactor::One => vk::BLEND_FACTOR_ONE,
            BlendFactor::Src => vk::BLEND_FACTOR_SRC_COLOR,
            BlendFactor::OneMinusSrc => vk::BLEND_FACTOR_ONE_MINUS_SRC_COLOR,
            BlendFactor::Dst => vk::BLEND_FACTOR_DST_COLOR,
            BlendFactor::OneMinusDst => vk::BLEND_FACTOR_ONE_MINUS_DST_COLOR,
            BlendFactor::SrcAlpha => vk::BLEND_FACTOR_SRC_ALPHA,
            BlendFactor::OneMinusSrcAlpha => vk::BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
            BlendFactor::DstAlpha => vk::BLEND_FACTOR_DST_ALPHA,
            BlendFactor::OneMinusDstAlpha => vk::BLEND_FACTOR_ONE_MINUS_DST_ALPHA,
        };
        let op = |o: BlendOp| match o {
            BlendOp::Add => vk::BLEND_OP_ADD,
            BlendOp::Subtract => vk::BLEND_OP_SUBTRACT,
            BlendOp::ReverseSubtract => vk::BLEND_OP_REVERSE_SUBTRACT,
            BlendOp::Min => vk::BLEND_OP_MIN,
            BlendOp::Max => vk::BLEND_OP_MAX,
        };
        let att: Vec<vk::PipelineColorBlendAttachmentState> = desc
            .targets
            .iter()
            .map(|t| {
                let b = t.blend.unwrap_or(Blend::ADD);
                vk::PipelineColorBlendAttachmentState {
                    blend_enable: t.blend.is_some() as u32,
                    src_color_blend_factor: factor(b.color.src),
                    dst_color_blend_factor: factor(b.color.dst),
                    color_blend_op: op(b.color.op),
                    src_alpha_blend_factor: factor(b.alpha.src),
                    dst_alpha_blend_factor: factor(b.alpha.dst),
                    alpha_blend_op: op(b.alpha.op),
                    color_write_mask: t.write_mask & 0xF,
                }
            })
            .collect();
        let cb = vk::PipelineColorBlendStateCreateInfo {
            s_type: vk::STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
            attachment_count: att.len() as u32,
            p_attachments: att.as_ptr(),
            ..vk::Zero::zero()
        };
        let dyns = [vk::DYNAMIC_STATE_VIEWPORT, vk::DYNAMIC_STATE_SCISSOR];
        let dy = vk::PipelineDynamicStateCreateInfo {
            s_type: vk::STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO,
            dynamic_state_count: 2,
            p_dynamic_states: dyns.as_ptr(),
            ..vk::Zero::zero()
        };
        let mut colors = [0u32; COLOR_TARGETS_MAX];
        for (i, t) in desc.targets.iter().enumerate() {
            colors[i] = vk_format(t.format);
        }
        let depth = desc.depth.map_or(vk::FORMAT_UNDEFINED, |d| vk_format(d.format));
        let stencil = desc.depth.filter(|d| d.format.has_stencil()).map_or(vk::FORMAT_UNDEFINED, |d| vk_format(d.format));
        let ri = vk::PipelineRenderingCreateInfo {
            s_type: vk::STRUCTURE_TYPE_PIPELINE_RENDERING_CREATE_INFO,
            color_attachment_count: desc.targets.len() as u32,
            p_color_attachment_formats: colors.as_ptr(),
            depth_attachment_format: depth,
            stencil_attachment_format: stencil,
            ..vk::Zero::zero()
        };
        let gi = vk::GraphicsPipelineCreateInfo {
            s_type: vk::STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO,
            p_next: &ri as *const _ as *const c_void,
            stage_count: stages.len() as u32,
            p_stages: stages.as_ptr(),
            p_vertex_input_state: &vi,
            p_input_assembly_state: &ia,
            p_viewport_state: &vp,
            p_rasterization_state: &rs,
            p_multisample_state: &ms,
            p_depth_stencil_state: &ds,
            p_color_blend_state: &cb,
            p_dynamic_state: &dy,
            layout: desc.layout.0.h,
            base_pipeline_index: -1,
            ..vk::Zero::zero()
        };
        let mut h = 0;
        // SAFETY: every structure of the create info lives until the call returns.
        sh.vk(unsafe { (sh.f.create_graphics_pipelines)(sh.dev, 0, 1, &gi, core::ptr::null(), &mut h) })?;
        Ok(RenderPipeline(Arc::new(PipeInner {
            sh: sh.clone(),
            h,
            layout: desc.layout.clone(),
            colors,
            ncolor: desc.targets.len(),
            depth,
        })))
    }
}

// ---------------------------------------------------------------- timestamps

pub(crate) struct TsInner {
    sh: Arc<Shared>,
    pub pool: vk::Handle,
    pub count: u32,
}

impl Drop for TsInner {
    fn drop(&mut self) {
        self.sh.defer(Garbage::Query(self.pool));
    }
}

/// GPU timestamps (CommandEncoder::reset_timestamps / timestamp)
#[derive(Clone)]
pub struct Timestamps(pub(crate) Arc<TsInner>);

impl Device {
    pub fn create_timestamps(&self, count: u32) -> Result<Timestamps> {
        let sh = &self.sh;
        sh.check()?;
        if !sh.features.timestamps {
            return Err(GpuError::Unsupported("timestamps"));
        }
        if count == 0 {
            return Err(GpuError::Invalid("0 timestamps"));
        }
        let ci = vk::QueryPoolCreateInfo {
            s_type: vk::STRUCTURE_TYPE_QUERY_POOL_CREATE_INFO,
            query_type: vk::QUERY_TYPE_TIMESTAMP,
            query_count: count,
            ..vk::Zero::zero()
        };
        let mut pool = 0;
        // SAFETY: valid create info.
        sh.vk(unsafe { (sh.f.create_query_pool)(sh.dev, &ci, core::ptr::null(), &mut pool) })?;
        let ts = Timestamps(Arc::new(TsInner { sh: sh.clone(), pool, count }));
        // queries start reset (unavailable: read_* gives NotReady until they are written): from
        // the host where the device allows it, else first thing in the next submission
        match sh.f.reset_query_pool.filter(|_| sh.host_query_reset) {
            // SAFETY: a new pool no command uses yet.
            Some(reset) => unsafe { reset(sh.dev, pool, 0, count) },
            None => {
                let mut st = sh.st.lock();
                let cb = sh.upload_cb(&mut st)?;
                // SAFETY: the upload buffer is recording outside a render pass.
                unsafe { (sh.f.cmd_reset_query_pool)(cb, pool, 0, count) };
            }
        }
        Ok(ts)
    }
}

impl Timestamps {
    pub fn count(&self) -> u32 {
        self.0.count
    }

    /// the raw ticks of timestamps first .. first + n (Err(NotReady): not all written and done)
    pub fn read_ticks(&self, first: u32, n: u32) -> Result<Vec<u64>> {
        let t = &self.0;
        t.sh.check()?;
        if n == 0 || first.checked_add(n).is_none_or(|e| e > t.count) {
            return Err(GpuError::Invalid("timestamps outside the set"));
        }
        let mut raw = vec![[0u64; 2]; n as usize];
        // SAFETY: raw holds n (value, availability) pairs of 16 bytes.
        let r = unsafe {
            (t.sh.f.get_query_pool_results)(t.sh.dev, t.pool, first, n, raw.len() * 16, raw.as_mut_ptr() as *mut c_void, 16,
                vk::QUERY_RESULT_64_BIT | vk::QUERY_RESULT_WITH_AVAILABILITY_BIT)
        };
        if r == vk::NOT_READY {
            return Err(GpuError::NotReady);
        }
        t.sh.vk(r)?;
        if raw.iter().any(|p| p[1] == 0) {
            return Err(GpuError::NotReady);
        }
        Ok(raw.iter().map(|p| p[0] & t.sh.ts_mask).collect())
    }

    /// timestamps first .. first + n in milliseconds on the GPU's clock, counted from the first
    /// timestamp the device read (so f64 keeps every tick: absolute ticks reach 10^18 ns)
    pub fn read_ms(&self, first: u32, n: u32) -> Result<Vec<f64>> {
        let ticks = self.read_ticks(first, n)?;
        let sh = &self.0.sh;
        let low = ticks.iter().copied().min().unwrap_or(0).max(1);
        let base = match sh.ts_base.compare_exchange(0, low, core::sync::atomic::Ordering::AcqRel,
                                                     core::sync::atomic::Ordering::Acquire) {
            Ok(_) => low,
            Err(b) => b,
        };
        let period = sh.limits.timestamp_period_ns as f64;
        Ok(ticks.into_iter().map(|t| (t as i128 - base as i128) as f64 * period / 1.0e6).collect())
    }
}
