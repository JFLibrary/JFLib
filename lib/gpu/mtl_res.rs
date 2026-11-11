// mtl_res -- Metal resources (AP6, docs/APPLE.md): buffers, textures and views, samplers, shader
// modules (SPIR-V translated to MSL by shader::msl), bind group and pipeline layouts, bind
// groups, compute and render pipelines.
//
// Buffers: MemoryKind::Device private storage, Upload and Readback shared (Apple GPUs have one
// memory; the kinds keep the API's rules: only Upload / Readback are mapped).  Textures: private,
// views are MTLTexture views (newTextureViewWithPixelFormat:textureType:levels:slices:).
// Shader modules: ShaderModule::from_spirv translates every entry point with the module's default
// constants and compiles the MSL once (MTLCompileOptions without fast math): a misuse or a gap of
// the translation shows at once, as on Vulkan; a pipeline with constants (or a point list)
// translates and compiles its own.  Bindings are flat: each pipeline keeps, per stage, which
// (group, binding) goes to which buffer / texture / sampler slot (shader::msl), and the command
// encoder sets them before each dispatch or draw (mtl_cmd.rs).  Bindless and timestamps are not
// built yet (level Basis; Features says so).

use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use crate::mtl_dev::{Device, Shared};
use crate::mtl_objc::{self as objc, Id, MTLOrigin, MTLSize, NSRange, Obj, Pool, msg};
use crate::*;

/// MTLPixelFormat of a format (Depth24Stencil8: Apple GPUs have only Depth32Float_Stencil8)
pub(crate) fn mtl_format(f: Format) -> usize {
    use Format::*;
    match f {
        R8Unorm => 10,
        Rg8Unorm => 30,
        Rgba8Unorm => 70,
        Rgba8Srgb => 71,
        Bgra8Unorm => 80,
        Bgra8Srgb => 81,
        Rgb10a2Unorm => 90,
        Rg11b10Float => 92,
        R16Float => 25,
        Rg16Float => 65,
        Rgba16Float => 115,
        R32Uint => 53,
        R32Sint => 54,
        R32Float => 55,
        Rg32Uint => 103,
        Rg32Float => 105,
        Rgba32Uint => 123,
        Rgba32Float => 125,
        Depth16 => 250,
        Depth32Float => 252,
        Depth24Stencil8 | Depth32FloatStencil8 => 260,
        Bc1Unorm => 130,
        Bc1Srgb => 131,
        Bc3Unorm => 134,
        Bc3Srgb => 135,
        Bc4Unorm => 140,
        Bc5Unorm => 142,
        Bc7Unorm => 152,
        Bc7Srgb => 153,
    }
}

/// MTLCompareFunction
pub(crate) fn mtl_compare(c: Compare) -> usize {
    match c {
        Compare::Never => 0,
        Compare::Less => 1,
        Compare::Equal => 2,
        Compare::LessEqual => 3,
        Compare::Greater => 4,
        Compare::NotEqual => 5,
        Compare::GreaterEqual => 6,
        Compare::Always => 7,
    }
}

// ---------------------------------------------------------------- buffers

pub(crate) struct BufInner {
    sh: Arc<Shared>,
    pub h: Obj,
    size: u64,
    usage: BufferUsage,
    mem: MemoryKind,
    ptr: usize,
}

impl Drop for BufInner {
    fn drop(&mut self) {
        self.sh.reserved.fetch_sub(self.size, Ordering::Relaxed);
    }
}

/// GPU memory for data: vertices, indices, storage, uniforms, indirect arguments
#[derive(Clone)]
pub struct Buffer(pub(crate) Arc<BufInner>);

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

    fn mapped_range(&self, offset: u64, n: u64) -> Result<usize> {
        let b = &self.0;
        if b.mem == MemoryKind::Device || b.ptr == 0 {
            return Err(GpuError::Invalid("buffer not host-visible (MemoryKind::Device)"));
        }
        if offset.checked_add(n).is_none_or(|e| e > b.size) {
            return Err(GpuError::Invalid("range outside the buffer"));
        }
        Ok(b.ptr + offset as usize)
    }

    /// copy data into a mapped buffer (Upload / Readback) at offset; the GPU sees it in the next
    /// submission -- do not write what a pending submission still reads
    pub fn write<T: Pod>(&self, offset: u64, data: &[T]) -> Result<()> {
        let bytes = bytes_of(data);
        let p = self.mapped_range(offset, bytes.len() as u64)?;
        // SAFETY: p addresses bytes.len() bytes inside the shared buffer's contents.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len()) };
        Ok(())
    }

    /// copy a mapped buffer's contents at offset into out (wait for the submission that wrote them)
    pub fn read<T: Pod>(&self, offset: u64, out: &mut [T]) -> Result<()> {
        let n = core::mem::size_of_val(out);
        let p = self.mapped_range(offset, n as u64)?;
        // SAFETY: p addresses n bytes of the contents; T is Pod, so any bytes are a valid T.
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
        // SAFETY: maxBufferLength of the live device.
        let max: usize = unsafe { msg!(sh.dev.id(), c"maxBufferLength"; usize) };
        if desc.size > max as u64 {
            return Err(GpuError::OutOfMemory);
        }
        let opts = match desc.memory {
            MemoryKind::Device => objc::STORAGE_PRIVATE,
            MemoryKind::Upload => objc::STORAGE_SHARED | objc::CPU_CACHE_WRITE_COMBINED,
            MemoryKind::Readback => objc::STORAGE_SHARED,
        };
        // a whole number of words (fill_buffer's kernel writes words)
        let n = desc.size.div_ceil(4) * 4;
        // SAFETY: newBufferWithLength:options: returns a new buffer (+1) or nil.
        let h = Obj::new(unsafe { msg!(sh.dev.id(), c"newBufferWithLength:options:", n as usize => usize, opts => usize; Id) })
            .ok_or(GpuError::OutOfMemory)?;
        let ptr = if desc.memory == MemoryKind::Device {
            0
        } else {
            // SAFETY: contents of a shared buffer: its bytes, mapped for its lifetime.
            unsafe { msg!(h.id(), c"contents"; *mut u8) as usize }
        };
        sh.reserved.fetch_add(desc.size, Ordering::Relaxed);
        Ok(Buffer(Arc::new(BufInner { sh: sh.clone(), h, size: desc.size, usage: desc.usage, mem: desc.memory, ptr })))
    }
}

// ---------------------------------------------------------------- textures

pub(crate) struct TexInner {
    sh: Arc<Shared>,
    pub h: Obj,
    pub desc: TextureDesc,
    bytes: u64,
}

impl Drop for TexInner {
    fn drop(&mut self) {
        self.sh.reserved.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

pub(crate) struct ViewInner {
    pub h: Obj,
    pub tex: Arc<TexInner>,
    pub base_mip: u32,
    pub mips: u32,
    #[allow(dead_code)] // (kept with the other bounds of the view)
    pub base_layer: u32,
    pub layers: u32,
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
    pub fn usage(&self) -> TextureUsage {
        self.0.tex.desc.usage
    }
    /// the same view (it or a clone of it): a cache's key
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

/// MTLTextureType of a view dimension
fn texture_type(dim: ViewDim) -> usize {
    match dim {
        ViewDim::D2 => 2,
        ViewDim::D2Array => 3,
        ViewDim::Cube => 5,
        ViewDim::CubeArray => 6,
        ViewDim::D3 => 7,
    }
}

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

impl Texture {
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

    /// a view of some mips / layers (counts 0: to the end)
    pub fn create_view(&self, desc: &ViewDesc) -> Result<TextureView> {
        let d = &self.t.desc;
        let mips = if desc.mips == 0 { d.mips.saturating_sub(desc.base_mip) } else { desc.mips };
        let layers = if desc.layers == 0 { d.layers.saturating_sub(desc.base_layer) } else { desc.layers };
        if mips == 0 || desc.base_mip + mips > d.mips || layers == 0 || desc.base_layer + layers > d.layers {
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
    let d = &t.desc;
    let whole = base_mip == 0 && mips == d.mips && base_layer == 0 && layers == d.layers && dim == default_dim(d);
    let h = if whole {
        t.h.clone()
    } else {
        let levels = NSRange { location: base_mip as usize, length: mips as usize };
        let slices = NSRange { location: base_layer as usize, length: layers as usize };
        // SAFETY: a view of a live texture with a range inside it (checked by the callers).
        Obj::new(unsafe {
            msg!(t.h.id(), c"newTextureViewWithPixelFormat:textureType:levels:slices:", mtl_format(d.format) => usize,
                 texture_type(dim) => usize, levels => NSRange, slices => NSRange; Id)
        })
        .ok_or(GpuError::OutOfMemory)?
    };
    Ok(TextureView(Arc::new(ViewInner { h, tex: t.clone(), base_mip, mips, base_layer, layers })))
}

/// a drawable's texture as a Texture (mtl_swap.rs)
pub(crate) fn wrap_texture(sh: &Arc<Shared>, h: Obj, desc: TextureDesc) -> Texture {
    let t = Arc::new(TexInner { sh: sh.clone(), h: h.clone(), desc, bytes: 0 });
    let view = TextureView(Arc::new(ViewInner { h, tex: t.clone(), base_mip: 0, mips: 1, base_layer: 0, layers: 1 }));
    Texture { t, view }
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
        if d.cube && d.layers > 6 && cfg!(target_abi = "sim") {
            return Err(GpuError::Unsupported("cube arrays: the simulator's Metal has none")); // (it would abort the process)
        }
        if d.usage == TextureUsage::NONE {
            return Err(GpuError::Invalid("texture without usage"));
        }
        if d.format.is_compressed() && !sh.features.texture_bc {
            return Err(GpuError::Unsupported("BC compressed textures"));
        }
        if d.usage.contains(TextureUsage::COLOR_TARGET) && d.format.is_depth()
            || d.usage.contains(TextureUsage::DEPTH_TARGET) && !d.format.is_depth()
        {
            return Err(GpuError::Invalid("colour targets need colour formats, depth targets depth formats"));
        }
        if d.usage.contains(TextureUsage::STORAGE) && (d.format.is_depth() || d.format.is_compressed() || d.format.is_srgb()) {
            return Err(GpuError::Unsupported("format with this usage"));
        }
        let dim = default_dim(&d);
        let mut usage = 0usize;
        if d.usage.contains(TextureUsage::SAMPLED) {
            usage |= 1; // ShaderRead
        }
        if d.usage.contains(TextureUsage::STORAGE) {
            usage |= 1 | 2; // ShaderRead | ShaderWrite
        }
        if d.usage.intersects(TextureUsage::COLOR_TARGET | TextureUsage::DEPTH_TARGET) {
            usage |= 4; // RenderTarget
        }
        usage |= 0x10; // PixelFormatView: views of other ranges
        let _pool = Pool::new();
        // SAFETY: a texture descriptor set up with values checked above; newTextureWithDescriptor:
        // returns a new texture (+1) or nil.
        let h = unsafe {
            let td: Id = msg!(objc::class(c"MTLTextureDescriptor"), c"new"; Id);
            let td = Obj::new(td).ok_or(GpuError::OutOfMemory)?;
            let t = td.id();
            msg!(t, c"setTextureType:", texture_type(dim) => usize; ());
            msg!(t, c"setPixelFormat:", mtl_format(d.format) => usize; ());
            msg!(t, c"setWidth:", d.width as usize => usize; ());
            msg!(t, c"setHeight:", d.height as usize => usize; ());
            msg!(t, c"setDepth:", d.depth as usize => usize; ());
            msg!(t, c"setMipmapLevelCount:", d.mips as usize => usize; ());
            let arr = if d.cube { d.layers / 6 } else { d.layers };
            msg!(t, c"setArrayLength:", arr as usize => usize; ());
            msg!(t, c"setUsage:", usage => usize; ());
            msg!(t, c"setStorageMode:", 2usize => usize; ());
            Obj::new(msg!(sh.dev.id(), c"newTextureWithDescriptor:", t => Id; Id)).ok_or(GpuError::OutOfMemory)?
        };
        // SAFETY: allocatedSize of the live texture.
        let bytes: usize = unsafe { msg!(h.id(), c"allocatedSize"; usize) };
        sh.reserved.fetch_add(bytes as u64, Ordering::Relaxed);
        let t = Arc::new(TexInner { sh: sh.clone(), h, desc: d, bytes: bytes as u64 });
        let view = make_view(&t, dim, 0, d.mips, 0, d.layers)?;
        Ok(Texture { t, view })
    }
}

/// a checked buffer <-> texture copy
pub(crate) struct Copy {
    /// bytes per row and per image (layer / slice) in the data
    pub bpr: u64,
    pub bpi: u64,
    /// bytes from the layout's offset on
    pub span: u64,
}

/// a buffer <-> texture copy: the region checked against the texture, the data layout against the
/// format (data_len: the bytes behind the layout)
pub(crate) fn copy_region(r: &TextureRegion, l: &DataLayout, data_len: u64) -> Result<Copy> {
    let t = r.texture;
    let d = &t.t.desc;
    if r.mip >= d.mips || r.layers == 0 || r.base_layer + r.layers > d.layers {
        return Err(GpuError::Invalid("copy: mip or layers outside the texture"));
    }
    let (mw, mh, md) = ((d.width >> r.mip).max(1), (d.height >> r.mip).max(1), (d.depth >> r.mip).max(1));
    let [x, y, z] = r.origin;
    let [w, h, dd] = r.size;
    if w == 0 || h == 0 || dd == 0 || x + w > mw || y + h > mh || z + dd > md {
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
    let slices = (dd * r.layers) as u64;
    let span = bpr * rows * (slices - 1) + bpr * (by - 1) + row;
    if l.offset.checked_add(span).is_none_or(|e| e > data_len) {
        return Err(GpuError::Invalid("copy: data smaller than the region"));
    }
    Ok(Copy { bpr, bpi: bpr * rows, span })
}

/// encode buffer -> texture for a checked copy (one copy per layer; a 3D region in one)
pub(crate) fn encode_buffer_to_texture(blit: Id, buf: Id, off: u64, c: &Copy, r: &TextureRegion) {
    let t = r.texture;
    let size = MTLSize { w: r.size[0] as usize, h: r.size[1] as usize, d: r.size[2] as usize };
    let origin = MTLOrigin { x: r.origin[0] as usize, y: r.origin[1] as usize, z: r.origin[2] as usize };
    for k in 0..r.layers {
        // SAFETY: an open blit encoder; the region lies in the texture, the bytes in the buffer
        // (copy_region checked both).
        unsafe {
            msg!(blit, c"copyFromBuffer:sourceOffset:sourceBytesPerRow:sourceBytesPerImage:sourceSize:toTexture:destinationSlice:destinationLevel:destinationOrigin:",
                 buf => Id, (off + c.bpi * r.size[2] as u64 * k as u64) as usize => usize, c.bpr as usize => usize, c.bpi as usize => usize,
                 size => MTLSize, t.t.h.id() => Id, (r.base_layer + k) as usize => usize, r.mip as usize => usize, origin => MTLOrigin; ());
        }
    }
}

/// encode texture -> buffer for a checked copy
pub(crate) fn encode_texture_to_buffer(blit: Id, buf: Id, off: u64, c: &Copy, r: &TextureRegion) {
    let t = r.texture;
    let size = MTLSize { w: r.size[0] as usize, h: r.size[1] as usize, d: r.size[2] as usize };
    let origin = MTLOrigin { x: r.origin[0] as usize, y: r.origin[1] as usize, z: r.origin[2] as usize };
    for k in 0..r.layers {
        // SAFETY: as in encode_buffer_to_texture.
        unsafe {
            msg!(blit, c"copyFromTexture:sourceSlice:sourceLevel:sourceOrigin:sourceSize:toBuffer:destinationOffset:destinationBytesPerRow:destinationBytesPerImage:",
                 t.t.h.id() => Id, (r.base_layer + k) as usize => usize, r.mip as usize => usize, origin => MTLOrigin, size => MTLSize,
                 buf => Id, (off + c.bpi * r.size[2] as u64 * k as u64) as usize => usize, c.bpr as usize => usize, c.bpi as usize => usize; ());
        }
    }
}

// ---------------------------------------------------------------- samplers

pub(crate) struct SamplerInner {
    pub h: Obj,
}

/// how textures are filtered and addressed
#[derive(Clone)]
pub struct Sampler(pub(crate) Arc<SamplerInner>);

impl Device {
    pub fn create_sampler(&self, desc: &SamplerDesc) -> Result<Sampler> {
        let sh = &self.sh;
        sh.check()?;
        if desc.anisotropy > 1 && !sh.features.anisotropy {
            return Err(GpuError::Unsupported("anisotropic filtering"));
        }
        if desc.lod_min < 0.0 || desc.lod_max < desc.lod_min {
            return Err(GpuError::Invalid("sampler lod range"));
        }
        let filter = |f: Filter| if f == Filter::Linear { 1usize } else { 0 };
        let addr = |a: AddressMode| match a {
            AddressMode::ClampToEdge => 0usize,
            AddressMode::Repeat => 2,
            AddressMode::MirrorRepeat => 3,
        };
        let _pool = Pool::new();
        // SAFETY: a sampler descriptor set up and turned into a state (+1).
        let h = unsafe {
            let sd = Obj::new(msg!(objc::class(c"MTLSamplerDescriptor"), c"new"; Id)).ok_or(GpuError::OutOfMemory)?;
            let s = sd.id();
            msg!(s, c"setMinFilter:", filter(desc.min) => usize; ());
            msg!(s, c"setMagFilter:", filter(desc.mag) => usize; ());
            msg!(s, c"setMipFilter:", if desc.mip == Filter::Linear { 2usize } else { 1 } => usize; ());
            msg!(s, c"setSAddressMode:", addr(desc.address[0]) => usize; ());
            msg!(s, c"setTAddressMode:", addr(desc.address[1]) => usize; ());
            msg!(s, c"setRAddressMode:", addr(desc.address[2]) => usize; ());
            msg!(s, c"setMaxAnisotropy:", (desc.anisotropy.clamp(1, 16)) as usize => usize; ());
            if let Some(c) = desc.compare {
                msg!(s, c"setCompareFunction:", mtl_compare(c) => usize; ());
            }
            msg!(s, c"setLodMinClamp:", desc.lod_min => f32; ());
            msg!(s, c"setLodMaxClamp:", desc.lod_max => f32; ());
            Obj::new(msg!(sh.dev.id(), c"newSamplerStateWithDescriptor:", s => Id; Id)).ok_or(GpuError::OutOfMemory)?
        };
        Ok(Sampler(Arc::new(SamplerInner { h })))
    }
}

// ---------------------------------------------------------------- shader modules

/// a translation of a module, compiled
pub(crate) struct Compiled {
    pub lib: Obj,
    pub msl: shader::msl::Msl,
}

pub(crate) struct ShaderInner {
    sh: Arc<Shared>,
    pub words: Vec<u32>,
    entries: Vec<shader::reflect::Entry>,
    spec: Vec<shader::reflect::SpecConst>,
    base: Compiled,
}

/// compiled shader code (SPIR-V, translated to MSL)
#[derive(Clone)]
pub struct ShaderModule(pub(crate) Arc<ShaderInner>);

/// compile MSL (no fast math: NaN, infinities and signed zeros as SPIR-V and WGSL have them)
pub(crate) fn compile(sh: &Shared, src: &str) -> Result<Obj> {
    let _pool = Pool::new();
    let s = objc::nsstring(src).ok_or(GpuError::OutOfMemory)?;
    // SAFETY: MTLCompileOptions set up (mathMode where it exists, macOS 15 / iOS 18, else
    // fastMathEnabled), newLibraryWithSource:options:error: returns a new library (+1) or nil
    // with an autoreleased error.
    unsafe {
        let o = Obj::new(msg!(objc::class(c"MTLCompileOptions"), c"new"; Id)).ok_or(GpuError::OutOfMemory)?;
        if msg!(o.id(), c"respondsToSelector:", objc::sel(c"setMathMode:") => objc::Sel; bool) {
            msg!(o.id(), c"setMathMode:", 0isize => isize; ()); // MTLMathModeSafe
        } else {
            msg!(o.id(), c"setFastMathEnabled:", false => bool; ());
        }
        msg!(o.id(), c"setLanguageVersion:", (2usize << 16) | 4 => usize; ()); // MSL 2.4
        let mut err: Id = objc::NIL;
        let lib = msg!(sh.dev.id(), c"newLibraryWithSource:options:error:", s.id() => Id, o.id() => Id, &mut err => *mut Id; Id);
        match Obj::new(lib) {
            Some(l) => Ok(l),
            None => {
                let text = objc::error_text(err);
                plat::write_stderr(format!("gpu: Metal rejected the translated shader: {}\n", text.lines().take(6).collect::<Vec<_>>().join(" | ")).as_bytes());
                if crate::mtl_dev::env_on("jfgui_GPU_DEBUG") {
                    plat::write_stderr(src.as_bytes());
                }
                Err(GpuError::Invalid("the Metal compiler rejected the translated shader (stderr says why)"))
            }
        }
    }
}

/// translate a module with options and compile it
fn translate(sh: &Shared, words: &[u32], opt: &shader::msl::Options) -> Result<Compiled> {
    let msl = shader::msl::translate(words, opt).map_err(|e| {
        plat::write_stderr(format!("gpu: the SPIR-V cannot be translated to MSL: {}\n", e).as_bytes());
        GpuError::Unsupported("SPIR-V the MSL translation does not cover (stderr says what)")
    })?;
    let lib = compile(sh, &msl.source)?;
    Ok(Compiled { lib, msl })
}

impl ShaderModule {
    /// a module from SPIR-V words (1.0 ..= 1.6), translated to MSL and compiled
    pub fn from_spirv(device: &Device, code: &[u32]) -> Result<ShaderModule> {
        let sh = &device.sh;
        sh.check()?;
        if code.len() < 5 || code[0] != 0x0723_0203 {
            return Err(GpuError::Invalid("not SPIR-V (magic number)"));
        }
        if code[1] > 0x0001_0600 || code[1] < 0x0001_0000 {
            return Err(GpuError::Unsupported("this SPIR-V version"));
        }
        let entries = shader::reflect::entry_points(code).map_err(|_| GpuError::Invalid("SPIR-V instructions do not fit the words"))?;
        let spec = shader::reflect::spec_constants(code).map_err(|_| GpuError::Invalid("SpecId on something else than a scalar specialization constant"))?;
        if spec.iter().enumerate().any(|(i, a)| spec[..i].iter().any(|b| b.spec_id == a.spec_id)) {
            return Err(GpuError::Invalid("a SpecId twice in the module"));
        }
        let base = translate(sh, code, &shader::msl::Options::DEFAULT)?;
        Ok(ShaderModule(Arc::new(ShaderInner { sh: sh.clone(), words: code.to_vec(), entries, spec, base })))
    }

    /// (the browser) WGSL text: Metal takes SPIR-V like Vulkan
    pub fn from_wgsl(_device: &Device, _source: &str) -> Result<ShaderModule> {
        Err(GpuError::Unsupported("WGSL text (Metal takes SPIR-V from lib/wgsl)"))
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
            let t = x as i128;
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

/// what a pipeline stage binds where (the translation's entry)
#[derive(Clone)]
pub(crate) struct StageInfo {
    pub slots: Vec<shader::msl::Slot>,
    pub push: bool,
    pub sizes: bool,
    /// it reads DrawIndex (shader::msl::DRAW_INDEX_AT of the sizes)
    pub draw_index: bool,
    pub workgroup: [u32; 3],
}

/// a stage's function: its module's base translation, or one of its own for constants or a point
/// list -> (function (+1), what it binds)
fn stage_function(st: &ShaderStage, model: shader::Model, point_size: bool) -> Result<(Obj, StageInfo)> {
    let m = &st.module.0;
    if !m.entries.iter().any(|e| e.name == st.entry && e.model == model as u32) {
        return Err(GpuError::Invalid("the module has no entry point of this name for the stage"));
    }
    let mut set = Vec::with_capacity(st.constants.len());
    for (i, c) in st.constants.iter().enumerate() {
        if st.constants[..i].iter().any(|d| d.id == c.id) {
            return Err(GpuError::Invalid("pipeline constant: an id twice"));
        }
        let Some(decl) = m.spec.iter().find(|s| s.spec_id == c.id) else {
            return Err(GpuError::Invalid("pipeline constant: the module has no override with this id"));
        };
        set.push((c.id, constant_bits(decl.kind, c.value)?));
    }
    let own;
    let comp = if set.is_empty() && !point_size {
        &m.base
    } else {
        shader::reflect::constant_values(&m.words, &set)
            .map_err(|_| GpuError::Invalid("pipeline constants: a specialization constant is undefined (division by 0, shift)"))?;
        own = translate(&m.sh, &m.words, &shader::msl::Options { spec: &set, entry: Some(st.entry), point_size })?;
        &own
    };
    let e = comp.msl.entries.iter().find(|e| e.name == st.entry && e.model == model as u32)
        .ok_or(GpuError::Invalid("the module has no entry point of this name for the stage"))?;
    let _pool = Pool::new();
    let name = objc::nsstring(&e.function).ok_or(GpuError::OutOfMemory)?;
    // SAFETY: newFunctionWithName: of a live library returns a new function (+1) or nil.
    let f = Obj::new(unsafe { msg!(comp.lib.id(), c"newFunctionWithName:", name.id() => Id; Id) })
        .ok_or(GpuError::Invalid("the translated entry point is missing in the library"))?;
    let info = StageInfo { slots: e.slots.clone(), push: e.push_constants, sizes: e.sizes, draw_index: e.draw_index, workgroup: e.workgroup };
    Ok((f, info))
}

// ---------------------------------------------------------------- bindings

pub(crate) struct BglInner {
    pub bindings: Vec<Binding>, // sorted by binding
}

/// the bindings of one bind group (@group)
#[derive(Clone)]
pub struct BindGroupLayout(pub(crate) Arc<BglInner>);

pub(crate) struct PlInner {
    pub groups: Vec<BindGroupLayout>,
    pub push: u32,
}

/// the bind group layouts and push constants of pipelines
#[derive(Clone)]
pub struct PipelineLayout(pub(crate) Arc<PlInner>);

/// a bound resource
#[derive(Clone)]
pub(crate) enum Res {
    /// buffer, offset, range
    Buf(Obj, u64, u64),
    Tex(Obj),
    Smp(Obj),
}

pub(crate) struct BgInner {
    pub layout: Arc<BglInner>,
    /// (binding, resource), sorted by binding
    pub res: Vec<(u32, Res)>,
}

/// resources bound for shaders
#[derive(Clone)]
pub struct BindGroup(pub(crate) Arc<BgInner>);

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
            if x.stages.0 & Stages::ALL.0 == 0 {
                return Err(GpuError::Invalid("binding without stages"));
            }
        }
        Ok(BindGroupLayout(Arc::new(BglInner { bindings: b })))
    }

    pub fn create_pipeline_layout(&self, desc: &PipelineLayoutDesc) -> Result<PipelineLayout> {
        self.sh.check()?;
        if desc.groups.len() > BIND_GROUPS_MAX as usize {
            return Err(GpuError::Invalid("too many bind groups"));
        }
        if desc.push_constants % 4 != 0 || desc.push_constants > PUSH_CONSTANTS_MAX {
            return Err(GpuError::Invalid("push constants: a multiple of 4, at most PUSH_CONSTANTS_MAX"));
        }
        Ok(PipelineLayout(Arc::new(PlInner { groups: desc.groups.iter().map(|g| (*g).clone()).collect(), push: desc.push_constants })))
    }

    pub fn create_bind_group(&self, desc: &BindGroupDesc) -> Result<BindGroup> {
        let sh = &self.sh;
        sh.check()?;
        let lay = &desc.layout.0;
        if desc.entries.len() != lay.bindings.len() {
            return Err(GpuError::Invalid("bind group: one entry per binding of the layout"));
        }
        let mut res: Vec<(u32, Res)> = Vec::with_capacity(desc.entries.len());
        for e in desc.entries {
            let Some(b) = lay.bindings.iter().find(|b| b.binding == e.binding) else {
                return Err(GpuError::Invalid("bind group: entry for a binding the layout lacks"));
            };
            if res.iter().any(|r| r.0 == e.binding) {
                return Err(GpuError::Invalid("bind group: a binding twice"));
            }
            let r = match (b.kind, e.resource) {
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
                    if offset % align.max(1) as u64 != 0 || range == 0 || offset + range > buffer.size() || range > max as u64 {
                        return Err(GpuError::Invalid("bind group: buffer range (alignment, size, limit)"));
                    }
                    Res::Buf(buffer.0.h.clone(), offset, range)
                }
                (BindingKind::Texture, Resource::Texture(v)) | (BindingKind::StorageTexture, Resource::StorageTexture(v)) => {
                    let need = if b.kind == BindingKind::StorageTexture { TextureUsage::STORAGE } else { TextureUsage::SAMPLED };
                    if !v.usage().contains(need) {
                        return Err(GpuError::Invalid("bind group: texture lacks SAMPLED / STORAGE usage"));
                    }
                    Res::Tex(v.0.h.clone())
                }
                (BindingKind::Sampler, Resource::Sampler(s)) => Res::Smp(s.0.h.clone()),
                _ => return Err(GpuError::Invalid("bind group: resource does not match the binding's kind")),
            };
            res.push((e.binding, r));
        }
        res.sort_by_key(|r| r.0);
        Ok(BindGroup(Arc::new(BgInner { layout: lay.clone(), res })))
    }

    /// (level Voll) a Bindless table: not on Metal yet (AP6g)
    pub fn create_bindless(&self, _desc: &BindlessDesc) -> Result<Bindless> {
        Err(GpuError::Unsupported("Bindless tables (level Voll; Metal: AP6g)"))
    }
}

/// a large table of textures, samplers and buffers that shaders index (level Voll; not on Metal yet)
pub struct Bindless {
    group: BindGroup,
    layout: BindGroupLayout,
}

impl Bindless {
    pub fn group(&self) -> &BindGroup {
        &self.group
    }
    pub fn layout(&self) -> &BindGroupLayout {
        &self.layout
    }
    pub fn counts(&self) -> [u32; 4] {
        [0; 4]
    }
    pub fn set_texture(&self, _slot: u32, _view: &TextureView) -> Result<()> {
        Err(GpuError::Unsupported("Bindless tables"))
    }
    pub fn set_sampler(&self, _slot: u32, _s: &Sampler) -> Result<()> {
        Err(GpuError::Unsupported("Bindless tables"))
    }
    pub fn set_buffer(&self, _slot: u32, _buf: &Buffer, _offset: u64, _size: u64) -> Result<()> {
        Err(GpuError::Unsupported("Bindless tables"))
    }
    pub fn set_storage_texture(&self, _slot: u32, _view: &TextureView) -> Result<()> {
        Err(GpuError::Unsupported("Bindless tables"))
    }
}

// ---------------------------------------------------------------- pipelines

pub(crate) struct PipeInner {
    pub state: Obj,
    pub layout: PipelineLayout,
    /// compute or vertex stage, fragment stage
    pub stages: [Option<StageInfo>; 2],
    pub depth_state: Option<Obj>,
    pub colors: [Option<Format>; COLOR_TARGETS_MAX],
    pub ncolor: usize,
    pub depth: Option<Format>,
    /// MTLPrimitiveType, MTLCullMode, MTLWinding, MTLDepthClipMode
    pub prim: usize,
    pub cull: usize,
    pub winding: usize,
}

/// a compute shader with its layout
#[derive(Clone)]
pub struct ComputePipeline(pub(crate) Arc<PipeInner>);

/// vertex + fragment shaders with their fixed state and target formats
#[derive(Clone)]
pub struct RenderPipeline(pub(crate) Arc<PipeInner>);

/// a stage's slots must find their groups and bindings in the layout, of the right kind
fn check_slots(info: &StageInfo, layout: &PipelineLayout) -> Result<()> {
    use shader::msl::SlotKind;
    for s in &info.slots {
        let Some(g) = layout.0.groups.get(s.set as usize) else {
            return Err(GpuError::Invalid("the shader uses a bind group the pipeline layout lacks"));
        };
        let Some(b) = g.0.bindings.iter().find(|b| b.binding == s.binding) else {
            return Err(GpuError::Invalid("the shader uses a binding the bind group layout lacks"));
        };
        let fits = match s.kind {
            SlotKind::Buffer => matches!(b.kind, BindingKind::UniformBuffer | BindingKind::StorageBuffer | BindingKind::StorageBufferRead),
            SlotKind::Texture => matches!(b.kind, BindingKind::Texture | BindingKind::StorageTexture),
            SlotKind::Sampler => b.kind == BindingKind::Sampler,
        };
        if !fits {
            return Err(GpuError::Invalid("a binding's kind differs from what the shader declares"));
        }
    }
    Ok(())
}

impl Device {
    pub fn create_compute_pipeline(&self, desc: &ComputePipelineDesc) -> Result<ComputePipeline> {
        self.compute_pipeline(desc)
    }

    /// create_compute_pipeline with a required subgroup size: Metal's SIMD groups have one size
    /// (32), so only 0 (the driver's choice) and that size work
    pub fn create_compute_pipeline_sized(&self, desc: &ComputePipelineDesc, size: u32) -> Result<ComputePipeline> {
        if size != 0 && size != self.sh.limits.subgroup_size {
            return Err(GpuError::Unsupported("subgroup size control"));
        }
        self.compute_pipeline(desc)
    }

    fn compute_pipeline(&self, desc: &ComputePipelineDesc) -> Result<ComputePipeline> {
        let sh = &self.sh;
        sh.check()?;
        // the workgroup size once specialized, before anything is translated
        let m = &desc.shader.module.0;
        let mut set = Vec::new();
        for c in desc.shader.constants {
            if let Some(decl) = m.spec.iter().find(|s| s.spec_id == c.id) {
                set.push((c.id, constant_bits(decl.kind, c.value)?));
            }
        }
        if let Ok(Some(w)) = shader::reflect::workgroup_size(&m.words, desc.shader.entry, &set) {
            let lim = &sh.limits;
            let count = w.iter().map(|&x| x as u64).product::<u64>();
            if w.contains(&0) || (0..3).any(|i| w[i] > lim.max_compute_workgroup_size[i]) || count > lim.max_compute_invocations as u64 {
                return Err(GpuError::Invalid("workgroup size 0 or beyond Limits::max_compute_workgroup_size / max_compute_invocations"));
            }
        }
        let (f, info) = stage_function(&desc.shader, shader::Model::GLCompute, false)?;
        let wg = info.workgroup;
        let lim = &sh.limits;
        let count = wg.iter().map(|&x| x as u64).product::<u64>();
        if wg.contains(&0) || (0..3).any(|i| wg[i] > lim.max_compute_workgroup_size[i]) || count > lim.max_compute_invocations as u64 {
            return Err(GpuError::Invalid("workgroup size 0 or beyond Limits::max_compute_workgroup_size / max_compute_invocations"));
        }
        check_slots(&info, desc.layout)?;
        let _pool = Pool::new();
        let mut err: Id = objc::NIL;
        // SAFETY: newComputePipelineStateWithFunction:error: returns a new state (+1) or nil.
        let state = unsafe { msg!(sh.dev.id(), c"newComputePipelineStateWithFunction:error:", f.id() => Id, &mut err => *mut Id; Id) };
        let state = Obj::new(state).ok_or_else(|| {
            plat::write_stderr(format!("gpu: Metal refused the compute pipeline: {}\n", objc::error_text(err)).as_bytes());
            GpuError::Invalid("Metal refused the compute pipeline (stderr says why)")
        })?;
        // SAFETY: maxTotalThreadsPerThreadgroup of the live state.
        let max: usize = unsafe { msg!(state.id(), c"maxTotalThreadsPerThreadgroup"; usize) };
        if count > max as u64 {
            return Err(GpuError::Unsupported("a workgroup larger than this pipeline allows on the device"));
        }
        Ok(ComputePipeline(Arc::new(PipeInner {
            state,
            layout: desc.layout.clone(),
            stages: [Some(info), None],
            depth_state: None,
            colors: [None; COLOR_TARGETS_MAX],
            ncolor: 0,
            depth: None,
            prim: 0,
            cull: 0,
            winding: 0,
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
        if desc.buffers.len() > sh.limits.max_vertex_buffers as usize
            || desc.buffers.iter().map(|b| b.attributes.len()).sum::<usize>() > sh.limits.max_vertex_attributes as usize
        {
            return Err(GpuError::Invalid("too many vertex buffers or attributes"));
        }
        let points = desc.topology == Topology::Points;
        let (vf, vinfo) = stage_function(&desc.vertex, shader::Model::Vertex, points)?;
        let frag = match &desc.fragment {
            Some(fs) => Some(stage_function(fs, shader::Model::Fragment, false)?),
            None => None,
        };
        check_slots(&vinfo, desc.layout)?;
        if let Some((_, fi)) = &frag {
            check_slots(fi, desc.layout)?;
        }
        let factor = |f: BlendFactor| -> usize {
            match f {
                BlendFactor::Zero => 0,
                BlendFactor::One => 1,
                BlendFactor::Src => 2,
                BlendFactor::OneMinusSrc => 3,
                BlendFactor::SrcAlpha => 4,
                BlendFactor::OneMinusSrcAlpha => 5,
                BlendFactor::Dst => 6,
                BlendFactor::OneMinusDst => 7,
                BlendFactor::DstAlpha => 8,
                BlendFactor::OneMinusDstAlpha => 9,
            }
        };
        let op = |o: BlendOp| -> usize {
            match o {
                BlendOp::Add => 0,
                BlendOp::Subtract => 1,
                BlendOp::ReverseSubtract => 2,
                BlendOp::Min => 3,
                BlendOp::Max => 4,
            }
        };
        let vfmt = |f: VertexFormat| -> usize {
            match f {
                VertexFormat::F32 => 28,
                VertexFormat::F32x2 => 29,
                VertexFormat::F32x3 => 30,
                VertexFormat::F32x4 => 31,
                VertexFormat::U32 => 36,
                VertexFormat::U32x2 => 37,
                VertexFormat::U32x4 => 39,
                VertexFormat::Unorm8x4 => 9,
            }
        };
        let _pool = Pool::new();
        // SAFETY: a render pipeline descriptor set up from checked values; its sub-descriptors are
        // owned by it (autoreleased getters inside the pool); newRenderPipelineStateWithDescriptor:
        // error: returns a new state (+1) or nil.
        let state = unsafe {
            let pd = Obj::new(msg!(objc::class(c"MTLRenderPipelineDescriptor"), c"new"; Id)).ok_or(GpuError::OutOfMemory)?;
            let p = pd.id();
            msg!(p, c"setVertexFunction:", vf.id() => Id; ());
            if let Some((ff, _)) = &frag {
                msg!(p, c"setFragmentFunction:", ff.id() => Id; ());
            }
            let cas: Id = msg!(p, c"colorAttachments"; Id);
            for (i, t) in desc.targets.iter().enumerate() {
                let ca: Id = msg!(cas, c"objectAtIndexedSubscript:", i => usize; Id);
                msg!(ca, c"setPixelFormat:", mtl_format(t.format) => usize; ());
                // MTLColorWriteMask: red 8, green 4, blue 2, alpha 1
                let m = t.write_mask;
                let wm = ((m & 1) << 3) | ((m & 2) << 1) | ((m & 4) >> 1) | ((m & 8) >> 3);
                msg!(ca, c"setWriteMask:", wm as usize => usize; ());
                if let Some(b) = t.blend {
                    msg!(ca, c"setBlendingEnabled:", true => bool; ());
                    msg!(ca, c"setSourceRGBBlendFactor:", factor(b.color.src) => usize; ());
                    msg!(ca, c"setDestinationRGBBlendFactor:", factor(b.color.dst) => usize; ());
                    msg!(ca, c"setRgbBlendOperation:", op(b.color.op) => usize; ());
                    msg!(ca, c"setSourceAlphaBlendFactor:", factor(b.alpha.src) => usize; ());
                    msg!(ca, c"setDestinationAlphaBlendFactor:", factor(b.alpha.dst) => usize; ());
                    msg!(ca, c"setAlphaBlendOperation:", op(b.alpha.op) => usize; ());
                }
            }
            if let Some(d) = desc.depth {
                msg!(p, c"setDepthAttachmentPixelFormat:", mtl_format(d.format) => usize; ());
                if d.format.has_stencil() {
                    msg!(p, c"setStencilAttachmentPixelFormat:", mtl_format(d.format) => usize; ());
                }
            }
            if !desc.buffers.is_empty() {
                let vd: Id = msg!(objc::class(c"MTLVertexDescriptor"), c"vertexDescriptor"; Id);
                let attrs: Id = msg!(vd, c"attributes"; Id);
                let layouts: Id = msg!(vd, c"layouts"; Id);
                for (i, b) in desc.buffers.iter().enumerate() {
                    let slot = shader::msl::vertex_slot(i as u32) as usize;
                    let l: Id = msg!(layouts, c"objectAtIndexedSubscript:", slot => usize; Id);
                    msg!(l, c"setStride:", b.stride as usize => usize; ());
                    // MTLVertexStepFunctionPerVertex 1, PerInstance 2
                    msg!(l, c"setStepFunction:", if b.per_instance { 2usize } else { 1 } => usize; ());
                    for a in b.attributes {
                        let at: Id = msg!(attrs, c"objectAtIndexedSubscript:", a.location as usize => usize; Id);
                        msg!(at, c"setFormat:", vfmt(a.format) => usize; ());
                        msg!(at, c"setOffset:", a.offset as usize => usize; ());
                        msg!(at, c"setBufferIndex:", slot => usize; ());
                    }
                }
                msg!(p, c"setVertexDescriptor:", vd => Id; ());
            }
            // MTLPrimitiveTopologyClass: point 1, line 2, triangle 3
            let class = match desc.topology {
                Topology::Points => 1usize,
                Topology::Lines | Topology::LineStrip => 2,
                _ => 3,
            };
            msg!(p, c"setInputPrimitiveTopology:", class => usize; ());
            let mut err: Id = objc::NIL;
            let s = msg!(sh.dev.id(), c"newRenderPipelineStateWithDescriptor:error:", p => Id, &mut err => *mut Id; Id);
            Obj::new(s).ok_or_else(|| {
                plat::write_stderr(format!("gpu: Metal refused the render pipeline: {}\n", objc::error_text(err)).as_bytes());
                GpuError::Invalid("Metal refused the render pipeline (stderr says why)")
            })?
        };
        // SAFETY: a depth-stencil descriptor turned into a state (+1).
        let depth_state = unsafe {
            let dd = Obj::new(msg!(objc::class(c"MTLDepthStencilDescriptor"), c"new"; Id)).ok_or(GpuError::OutOfMemory)?;
            let (cmp, write) = match desc.depth {
                Some(d) => (mtl_compare(d.compare), d.write),
                None => (7, false),
            };
            msg!(dd.id(), c"setDepthCompareFunction:", cmp => usize; ());
            msg!(dd.id(), c"setDepthWriteEnabled:", write => bool; ());
            Obj::new(msg!(sh.dev.id(), c"newDepthStencilStateWithDescriptor:", dd.id() => Id; Id))
        };
        let mut colors = [None; COLOR_TARGETS_MAX];
        for (i, t) in desc.targets.iter().enumerate() {
            colors[i] = Some(t.format);
        }
        // MTLPrimitiveType: point 0, line 1, line strip 2, triangle 3, triangle strip 4
        let prim = match desc.topology {
            Topology::Points => 0,
            Topology::Lines => 1,
            Topology::LineStrip => 2,
            Topology::Triangles => 3,
            Topology::TriangleStrip => 4,
        };
        let cull = match desc.cull {
            Cull::None => 0,
            Cull::Front => 1,
            Cull::Back => 2,
        };
        Ok(RenderPipeline(Arc::new(PipeInner {
            state,
            layout: desc.layout.clone(),
            stages: [Some(vinfo), frag.map(|f| f.1)],
            depth_state,
            colors,
            ncolor: desc.targets.len(),
            depth: desc.depth.map(|d| d.format),
            prim,
            cull,
            winding: if desc.front_cw { 0 } else { 1 }, // MTLWindingClockwise 0, CounterClockwise 1
        })))
    }
}

// ---------------------------------------------------------------- timestamps

/// GPU timestamps (not on Metal yet: Features::timestamps is false)
pub struct Timestamps {
    count: u32,
}

impl Device {
    pub fn create_timestamps(&self, _count: u32) -> Result<Timestamps> {
        self.sh.check()?;
        Err(GpuError::Unsupported("timestamps (Metal: not built yet)"))
    }
}

impl Timestamps {
    pub fn count(&self) -> u32 {
        self.count
    }
    pub fn read_ticks(&self, _first: u32, _n: u32) -> Result<Vec<u64>> {
        Err(GpuError::Unsupported("timestamps"))
    }
    pub fn read_ms(&self, _first: u32, _n: u32) -> Result<Vec<f64>> {
        Err(GpuError::Unsupported("timestamps"))
    }
}
