// web_res -- WebGPU resources, bindings and pipelines in the browser (through web/gpu.js)
//
//   device.create_buffer(&BufferDesc)      copies to and from it allowed.  Upload: Buffer::write is
//        queue.writeBuffer (before the next submission, as mapped memory would be there).  Readback:
//        only copies may write it (WebGPU: MAP_READ goes with COPY_DST alone); after a submission that
//        copied into it the browser maps it, Buffer::read says NotReady until then
//   device.create_texture, texture.create_view, device.create_sampler   as WebGPU's
//   ShaderModule::from_wgsl(&device, &str) the browser takes the WGSL as it is, its validator is the
//        only one (lib/wgsl is not in the bundle; scan below reads names, overrides and bindings for
//        what the API promises).  from_spirv: Unsupported
//   Push constants: WGSL `var<immediate> x: T;` becomes `@group(3) @binding(0) var<uniform> x: T;`
//        (one block, a uniform layout), group 3 of a pipeline layout with push constants: at most
//        three groups of its own then.  web/gpu.js puts them into a ring behind a dynamic offset.
//   Pipeline constants: WebGPU's `constants`, the key the id as text; an override without @id (ids from
//        0x10000: docs/GPU.md "Pipeline-Konstanten") by its name.  Unknown or doubled ids are Invalid.
//   device.create_bind_group_layout(&[Binding])   WebGPU wants more than Binding says (a texture's view
//        dimension and sample type, a storage texture's format, a sampler's kind).  So the layout is made
//        late: from the first bind group's resources, or from the WGSL of the first pipeline that uses
//        it, whichever comes first
//
// A browser's errors come later than the call (web_dev.rs); what can be known here is Invalid at once.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::web_dev::{call, create, json_f64, DevInner, Device, Handle, CALL_PIPELINE_LAYOUT, CALL_READ_BUFFER, CALL_SHADER, CALL_WRITE_BUFFER};
use crate::*;

const METHOD_BUFFER: u32 = 0;
const METHOD_TEXTURE: u32 = 1;
const METHOD_SAMPLER: u32 = 2;
const METHOD_BIND_GROUP_LAYOUT: u32 = 3;
const METHOD_BIND_GROUP: u32 = 4;
const METHOD_COMPUTE_PIPELINE: u32 = 5;
const METHOD_RENDER_PIPELINE: u32 = 6;
const METHOD_VIEW: u32 = 100;

// GPUBufferUsage
const BU_MAP_READ: u32 = 1;
const BU_COPY_SRC: u32 = 4;
const BU_COPY_DST: u32 = 8;
const BU_INDEX: u32 = 16;
const BU_VERTEX: u32 = 32;
const BU_UNIFORM: u32 = 64;
const BU_STORAGE: u32 = 128;
const BU_INDIRECT: u32 = 256;

pub(crate) struct BufInner {
    h: Handle,
    d: Rc<DevInner>,
    size: u64,
    usage: BufferUsage,
    memory: MemoryKind,
}

impl Drop for BufInner {
    fn drop(&mut self) {
        self.d.reserved.set(self.d.reserved.get().saturating_sub(self.size));
    }
}

/// GPU memory for data: vertices, indices, storage, uniforms, indirect arguments
#[derive(Clone)]
pub struct Buffer(pub(crate) Rc<BufInner>);

impl Buffer {
    pub fn size(&self) -> u64 {
        self.0.size
    }
    pub fn usage(&self) -> BufferUsage {
        self.0.usage
    }
    pub fn memory(&self) -> MemoryKind {
        self.0.memory
    }
    pub(crate) fn handle(&self) -> u32 {
        self.0.h.h
    }

    /// data into an Upload buffer at offset: the GPU sees it in the next submission (queue.writeBuffer)
    pub fn write<T: Pod>(&self, offset: u64, data: &[T]) -> Result<()> {
        if self.0.memory == MemoryKind::Device {
            return Err(GpuError::Invalid("Buffer::write: an Upload buffer"));
        }
        if self.0.memory == MemoryKind::Readback {
            return Err(GpuError::Unsupported("Buffer::write on a Readback buffer: WebGPU maps it for reading only"));
        }
        let b = bytes_of(data);
        if offset % 4 != 0 || b.len() % 4 != 0 || offset.checked_add(b.len() as u64).is_none_or(|e| e > self.0.size) {
            return Err(GpuError::Invalid("Buffer::write: outside the buffer, or not in multiples of 4"));
        }
        call(CALL_WRITE_BUFFER, &[self.0.d.h, self.handle(), offset as u32, (offset >> 32) as u32, b.as_ptr() as u32, b.len() as u32]);
        Ok(())
    }

    /// a Readback buffer's contents at offset into out: Err(NotReady) until the submission that copied
    /// into it has finished and the browser has mapped it
    pub fn read<T: Pod>(&self, offset: u64, out: &mut [T]) -> Result<()> {
        if self.0.memory != MemoryKind::Readback {
            return Err(GpuError::Unsupported("Buffer::read: only a Readback buffer in the browser"));
        }
        let n = core::mem::size_of_val(out);
        if offset.checked_add(n as u64).is_none_or(|e| e > self.0.size) {
            return Err(GpuError::Invalid("Buffer::read: outside the buffer"));
        }
        self.0.d.check()?;
        match call(CALL_READ_BUFFER, &[self.handle(), offset as u32, (offset >> 32) as u32, out.as_mut_ptr() as u32, n as u32]) {
            0 => Err(GpuError::NotReady),
            _ => Ok(()),
        }
    }
}

impl Device {
    pub fn create_buffer(&self, desc: &BufferDesc) -> Result<Buffer> {
        self.d.check()?;
        if desc.size == 0 {
            return Err(GpuError::Invalid("a buffer of 0 bytes"));
        }
        let u = desc.usage;
        let mut bits = 0;
        for (f, b) in [(BufferUsage::UNIFORM, BU_UNIFORM), (BufferUsage::STORAGE, BU_STORAGE), (BufferUsage::INDEX, BU_INDEX), (BufferUsage::VERTEX, BU_VERTEX), (BufferUsage::INDIRECT, BU_INDIRECT)] {
            if u.contains(f) {
                bits |= b;
            }
        }
        let usage = if desc.memory == MemoryKind::Readback {
            if bits != 0 {
                return Err(GpuError::Unsupported("a Readback buffer takes copies only in WebGPU"));
            }
            BU_MAP_READ | BU_COPY_DST
        } else {
            bits | BU_COPY_SRC | BU_COPY_DST
        };
        let h = create(self.d.h, METHOD_BUFFER, &format!("{{\"size\":{},\"usage\":{usage}}}", desc.size))?;
        self.d.reserved.set(self.d.reserved.get() + desc.size);
        Ok(Buffer(Rc::new(BufInner { h: Handle { h, destroy: true }, d: self.d.clone(), size: desc.size, usage: desc.usage, memory: desc.memory })))
    }
}

/// the name WebGPU has for a format
pub(crate) fn format_name(f: Format) -> &'static str {
    use Format::*;
    match f {
        R8Unorm => "r8unorm",
        Rg8Unorm => "rg8unorm",
        Rgba8Unorm => "rgba8unorm",
        Rgba8Srgb => "rgba8unorm-srgb",
        Bgra8Unorm => "bgra8unorm",
        Bgra8Srgb => "bgra8unorm-srgb",
        Rgb10a2Unorm => "rgb10a2unorm",
        Rg11b10Float => "rg11b10ufloat",
        R16Float => "r16float",
        Rg16Float => "rg16float",
        Rgba16Float => "rgba16float",
        R32Uint => "r32uint",
        R32Sint => "r32sint",
        R32Float => "r32float",
        Rg32Uint => "rg32uint",
        Rg32Float => "rg32float",
        Rgba32Uint => "rgba32uint",
        Rgba32Float => "rgba32float",
        Depth16 => "depth16unorm",
        Depth32Float => "depth32float",
        Depth24Stencil8 => "depth24plus-stencil8",
        Depth32FloatStencil8 => "depth32float-stencil8",
        Bc1Unorm => "bc1-rgba-unorm",
        Bc1Srgb => "bc1-rgba-unorm-srgb",
        Bc3Unorm => "bc3-rgba-unorm",
        Bc3Srgb => "bc3-rgba-unorm-srgb",
        Bc4Unorm => "bc4-r-unorm",
        Bc5Unorm => "bc5-rg-unorm",
        Bc7Unorm => "bc7-rgba-unorm",
        Bc7Srgb => "bc7-rgba-unorm-srgb",
    }
}

fn dim_name(d: ViewDim) -> &'static str {
    match d {
        ViewDim::D2 => "2d",
        ViewDim::D2Array => "2d-array",
        ViewDim::Cube => "cube",
        ViewDim::CubeArray => "cube-array",
        ViewDim::D3 => "3d",
    }
}

pub(crate) struct ViewInner {
    pub h: Handle,
    format: Format,
    w: u32,
    hh: u32,
    dim: ViewDim,
    _tex: Option<Rc<TexInner>>,
}

/// a range of a texture's mips and layers as shaders and render passes see it
#[derive(Clone)]
pub struct TextureView(pub(crate) Rc<ViewInner>);

impl TextureView {
    pub fn format(&self) -> Format {
        self.0.format
    }
    /// width and height of the view's first mip
    pub fn size(&self) -> (u32, u32) {
        (self.0.w, self.0.hh)
    }
    pub(crate) fn handle(&self) -> u32 {
        self.0.h.h
    }
    /// what its texture may be used for (a swapchain frame's: a colour target)
    pub fn usage(&self) -> TextureUsage {
        self.0._tex.as_ref().map_or(TextureUsage::COLOR_TARGET, |t| t.desc.usage)
    }
    /// the same view (it or a clone of it), not merely one of the same range: a cache's key
    pub fn same(&self, other: &TextureView) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
    /// (the swapchain's image, web_swap.rs: its object changes with every frame)
    pub(crate) fn for_frame(h: u32, format: Format, w: u32, hh: u32) -> TextureView {
        TextureView(Rc::new(ViewInner { h: Handle { h, destroy: false }, format, w, hh, dim: ViewDim::D2, _tex: None }))
    }
}

pub(crate) struct TexInner {
    h: Handle,
    d: Option<Rc<DevInner>>,
    desc: TextureDesc,
    bytes: u64,
}

impl Drop for TexInner {
    fn drop(&mut self) {
        if let Some(d) = &self.d {
            d.reserved.set(d.reserved.get().saturating_sub(self.bytes));
        }
    }
}

/// an image on the GPU: 2D, 2D array, cube (array) or 3D, with mips
#[derive(Clone)]
pub struct Texture {
    t: Rc<TexInner>,
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
    pub(crate) fn handle(&self) -> u32 {
        self.t.h.h
    }
    /// (the swapchain's image, web_swap.rs: its object changes with every frame)
    pub(crate) fn for_frame(h: u32, view: TextureView, format: Format, width: u32, height: u32, usage: TextureUsage) -> Texture {
        let desc = TextureDesc::d2(width, height, format, usage);
        Texture { t: Rc::new(TexInner { h: Handle { h, destroy: false }, d: None, desc, bytes: 0 }), view }
    }

    /// a view of some mips / layers (counts 0: to the end)
    pub fn create_view(&self, d: &ViewDesc) -> Result<TextureView> {
        let t = &self.t.desc;
        let mips = if d.mips == 0 { t.mips.saturating_sub(d.base_mip) } else { d.mips };
        let total = if t.depth > 1 { 1 } else { t.layers };
        let layers = if d.layers == 0 { total.saturating_sub(d.base_layer) } else { d.layers };
        if mips == 0 || d.base_mip.checked_add(mips).is_none_or(|e| e > t.mips) || layers == 0
            || d.base_layer.checked_add(layers).is_none_or(|e| e > total) {
            return Err(GpuError::Invalid("create_view: outside the texture"));
        }
        make_view(&self.t, d.dim, d.base_mip, mips, d.base_layer, layers)
    }
}

fn make_view(t: &Rc<TexInner>, dim: ViewDim, base_mip: u32, mips: u32, base_layer: u32, layers: u32) -> Result<TextureView> {
    let json = format!(
        "{{\"dimension\":\"{}\",\"baseMipLevel\":{base_mip},\"mipLevelCount\":{mips},\"baseArrayLayer\":{base_layer},\"arrayLayerCount\":{layers}}}",
        dim_name(dim)
    );
    let h = create(t.h.h, METHOD_VIEW, &json)?;
    let (w, hh) = ((t.desc.width >> base_mip).max(1), (t.desc.height >> base_mip).max(1));
    Ok(TextureView(Rc::new(ViewInner { h: Handle { h, destroy: false }, format: t.desc.format, w, hh, dim, _tex: Some(t.clone()) })))
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

impl Device {
    pub fn create_texture(&self, desc: &TextureDesc) -> Result<Texture> {
        self.d.check()?;
        if desc.width == 0 || desc.height == 0 || desc.depth == 0 || desc.layers == 0 || desc.mips == 0 {
            return Err(GpuError::Invalid("a texture without size"));
        }
        let lim = self.d.limits.max_texture_2d;
        if desc.width > lim || desc.height > lim {
            return Err(GpuError::Unsupported("the texture is larger than this device's limit"));
        }
        if desc.format.is_compressed() && !self.d.features.texture_bc {
            return Err(GpuError::Unsupported("BC textures"));
        }
        let u = desc.usage.0;
        // (GPUTextureUsage has the first four bits; RENDER_ATTACHMENT for both kinds of target)
        let usage = (u & 0xF) | if u & 0x30 != 0 { 16 } else { 0 };
        let (third, dimension) = if desc.depth > 1 { (desc.depth, "3d") } else { (desc.layers, "2d") };
        let json = format!(
            "{{\"size\":[{},{},{third}],\"mipLevelCount\":{},\"format\":\"{}\",\"usage\":{usage},\"dimension\":\"{dimension}\"}}",
            desc.width, desc.height, desc.mips, format_name(desc.format)
        );
        let h = create(self.d.h, METHOD_TEXTURE, &json)?;
        let dim = desc.width as u64 / desc.format.block_dim() as u64 * (desc.height / desc.format.block_dim()) as u64;
        let mut bytes = dim * desc.format.block_bytes() as u64 * third as u64;
        if desc.mips > 1 {
            bytes += bytes / 3;
        }
        self.d.reserved.set(self.d.reserved.get() + bytes);
        let t = Rc::new(TexInner { h: Handle { h, destroy: true }, d: Some(self.d.clone()), desc: *desc, bytes });
        let layers = if desc.depth > 1 { 1 } else { desc.layers };
        let view = make_view(&t, default_dim(desc), 0, desc.mips, 0, layers)?;
        Ok(Texture { t, view })
    }

    pub fn create_sampler(&self, d: &SamplerDesc) -> Result<Sampler> {
        self.d.check()?;
        let address = |m: AddressMode| match m {
            AddressMode::Repeat => "repeat",
            AddressMode::MirrorRepeat => "mirror-repeat",
            AddressMode::ClampToEdge => "clamp-to-edge",
        };
        let filter = |f: Filter| if f == Filter::Linear { "linear" } else { "nearest" };
        let compare = d.compare.map_or(String::new(), |c| format!(",\"compare\":\"{}\"", compare_name(c)));
        let json = format!(
            "{{\"addressModeU\":\"{}\",\"addressModeV\":\"{}\",\"addressModeW\":\"{}\",\"magFilter\":\"{}\",\"minFilter\":\"{}\",\"mipmapFilter\":\"{}\",\"lodMinClamp\":{},\"lodMaxClamp\":{},\"maxAnisotropy\":{}{compare}}}",
            address(d.address[0]), address(d.address[1]), address(d.address[2]), filter(d.mag), filter(d.min), filter(d.mip),
            json_f64(d.lod_min as f64), json_f64(d.lod_max as f64), d.anisotropy.max(1),
        );
        let h = create(self.d.h, METHOD_SAMPLER, &json)?;
        let filtering = d.mag == Filter::Linear || d.min == Filter::Linear || d.mip == Filter::Linear;
        Ok(Sampler(Rc::new(SamplerInner { h: Handle { h, destroy: false }, comparison: d.compare.is_some(), filtering })))
    }
}

fn compare_name(c: Compare) -> &'static str {
    match c {
        Compare::Never => "never",
        Compare::Less => "less",
        Compare::Equal => "equal",
        Compare::LessEqual => "less-equal",
        Compare::Greater => "greater",
        Compare::NotEqual => "not-equal",
        Compare::GreaterEqual => "greater-equal",
        Compare::Always => "always",
    }
}

pub(crate) struct SamplerInner {
    h: Handle,
    comparison: bool,
    filtering: bool,
}

/// how textures are filtered and addressed
#[derive(Clone)]
pub struct Sampler(pub(crate) Rc<SamplerInner>);

// ---------------------------------------------------------------- what a WGSL text declares

/// what a binding of a module looks like: the rest of a WebGPU layout entry
#[derive(Clone, PartialEq)]
struct Detail {
    binding: u32,
    group: u32,
    json: String,
}

/// names and numbers the API promises from a module; the browser's validator judges the code
#[derive(Default)]
struct Scan {
    entries: Vec<(u32, String)>,
    /// id (from 0x10000: no @id, by position), kind, default value, name
    overrides: Vec<(u32, shader::reflect::Kind, f64, String)>,
    bindings: Vec<Detail>,
}

// the text without comments
fn strip_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let (mut i, mut depth) = (0, 0);
    while i < b.len() {
        if b[i..].starts_with(b"/*") {
            depth += 1;
            i += 2;
        } else if depth > 0 {
            if b[i..].starts_with(b"*/") {
                depth -= 1;
                i += 2;
            } else {
                i += 1;
            }
        } else if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

// the tokens: words (numbers and identifiers), each other character by itself
fn tokens(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut at = None;
    for (i, c) in s.char_indices() {
        let word = c.is_alphanumeric() || c == '_' || c == '.';
        match (word, at) {
            (true, None) => at = Some(i),
            (false, Some(a)) => {
                out.push(&s[a..i]);
                at = None;
            }
            _ => {}
        }
        if !word && !c.is_whitespace() {
            out.push(&s[i..i + c.len_utf8()]);
        }
    }
    if let Some(a) = at {
        out.push(&s[a..]);
    }
    out
}

fn scan(src: &str) -> Scan {
    let text = strip_comments(src);
    let t = tokens(&text);
    let mut s = Scan::default();
    let (mut group, mut binding, mut id, mut stage) = (None, None, None, None);
    let mut nameless = 0x10000u32;
    let mut depth = 0i32;
    let mut i = 0;
    while i < t.len() {
        match t[i] {
            "{" => depth += 1,
            "}" => depth -= 1,
            "@" if depth == 0 => {
                let what = t.get(i + 1).copied().unwrap_or("");
                let arg = if t.get(i + 2) == Some(&"(") { t.get(i + 3).and_then(|v| v.trim_end_matches(['u', 'i']).parse::<u32>().ok()) } else { None };
                match what {
                    "group" => group = arg,
                    "binding" => binding = arg,
                    "id" => id = arg,
                    "vertex" => stage = Some(0),
                    "fragment" => stage = Some(4),
                    "compute" => stage = Some(5),
                    _ => {}
                }
                i += 2;
                continue;
            }
            "fn" if depth == 0 => {
                if let (Some(m), Some(name)) = (stage.take(), t.get(i + 1)) {
                    s.entries.push((m, String::from(*name)));
                }
            }
            "override" if depth == 0 => {
                let name = t.get(i + 1).copied().unwrap_or("");
                let mut k = i + 2;
                let mut kind = shader::reflect::Kind::Float { width: 32 };
                if t.get(k) == Some(&":") {
                    kind = match t.get(k + 1).copied().unwrap_or("") {
                        "bool" => shader::reflect::Kind::Bool,
                        "i32" => shader::reflect::Kind::Int { width: 32, signed: true },
                        "u32" => shader::reflect::Kind::Int { width: 32, signed: false },
                        "f16" => shader::reflect::Kind::Float { width: 16 },
                        _ => shader::reflect::Kind::Float { width: 32 },
                    };
                    k += 2;
                }
                let mut value = 0.0;
                if t.get(k) == Some(&"=") {
                    let neg = t.get(k + 1) == Some(&"-");
                    let lit = t.get(k + if neg { 2 } else { 1 }).copied().unwrap_or("");
                    value = match lit {
                        "true" => 1.0,
                        "false" => 0.0,
                        _ => lit.trim_end_matches(['u', 'i', 'f', 'h']).parse().unwrap_or(0.0),
                    };
                    if neg {
                        value = -value;
                    }
                }
                let n = id.take().unwrap_or_else(|| {
                    let v = nameless;
                    nameless += 1;
                    v
                });
                s.overrides.push((n, kind, value, String::from(name)));
            }
            "var" if depth == 0 => {
                if let (Some(g), Some(b)) = (group.take(), binding.take()) {
                    let end = (i..t.len()).find(|&k| t[k] == ";").unwrap_or(t.len());
                    if let Some(json) = binding_detail(&t[i..end]) {
                        s.bindings.push(Detail { group: g, binding: b, json });
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    s
}

// the layout entry of `var name: type` or `var<space> name: type`
fn binding_detail(t: &[&str]) -> Option<String> {
    let at = t.iter().position(|w| *w == ":")?;
    let ty = *t.get(at + 1)?;
    let args: Vec<&str> = t[at + 2..].iter().copied().filter(|w| !matches!(*w, "<" | ">" | ",")).collect();
    let dim = |ty: &str| {
        if ty.ends_with("2d_array") {
            "2d-array"
        } else if ty.ends_with("cube_array") {
            "cube-array"
        } else if ty.ends_with("cube") {
            "cube"
        } else if ty.ends_with("3d") {
            "3d"
        } else if ty.ends_with("1d") {
            "1d"
        } else {
            "2d"
        }
    };
    if ty == "sampler" {
        return Some(String::from("\"sampler\":{\"type\":\"filtering\"}"));
    }
    if ty == "sampler_comparison" {
        return Some(String::from("\"sampler\":{\"type\":\"comparison\"}"));
    }
    if ty.starts_with("texture_storage_") {
        let access = match args.get(1).copied().unwrap_or("write") {
            "read" => "read-only",
            "read_write" => "read-write",
            _ => "write-only",
        };
        return Some(format!("\"storageTexture\":{{\"access\":\"{access}\",\"format\":\"{}\",\"viewDimension\":\"{}\"}}", args.first().copied().unwrap_or("rgba8unorm"), dim(ty)));
    }
    if ty.starts_with("texture_depth_") {
        return Some(format!("\"texture\":{{\"sampleType\":\"depth\",\"viewDimension\":\"{}\"}}", dim(ty)));
    }
    if ty.starts_with("texture_") {
        let sample = match args.first().copied().unwrap_or("f32") {
            "i32" => "sint",
            "u32" => "uint",
            _ => "float",
        };
        return Some(format!("\"texture\":{{\"sampleType\":\"{sample}\",\"viewDimension\":\"{}\"}}", dim(ty)));
    }
    None
}

pub(crate) struct ShaderInner {
    h: Handle,
    scan: Scan,
}

/// shader code: WGSL, as the browser takes it
#[derive(Clone)]
pub struct ShaderModule(pub(crate) Rc<ShaderInner>);

impl ShaderModule {
    /// the browser takes WGSL: ShaderModule::from_wgsl
    pub fn from_spirv(_device: &Device, _code: &[u32]) -> Result<ShaderModule> {
        Err(GpuError::Unsupported("SPIR-V in the browser: ShaderModule::from_wgsl"))
    }

    /// a module from WGSL source, which the browser compiles and validates (its errors come later:
    /// web_dev.rs); `var<immediate>` becomes the uniform block behind push constants
    pub fn from_wgsl(device: &Device, source: &str) -> Result<ShaderModule> {
        device.d.check()?;
        let code = source.replace("var<immediate>", "@group(3) @binding(0) var<uniform>");
        let h = call(CALL_SHADER, &[device.d.h, code.as_ptr() as u32, code.len() as u32]);
        if h == 0 {
            return Err(GpuError::Failed(0));
        }
        Ok(ShaderModule(Rc::new(ShaderInner { h: Handle { h, destroy: false }, scan: scan(source) })))
    }

    /// the entry points: (model as SPIR-V numbers it: 0 vertex, 4 fragment, 5 compute, name)
    pub fn entry_points(&self) -> Vec<(u32, &str)> {
        self.0.scan.entries.iter().map(|(m, n)| (*m, n.as_str())).collect()
    }

    /// the pipeline-overridable constants the module declares: (id, kind, default value)
    pub fn constants(&self) -> Vec<(u32, shader::reflect::Kind, f64)> {
        self.0.scan.overrides.iter().map(|(i, k, v, _)| (*i, *k, *v)).collect()
    }
}

// ---------------------------------------------------------------- bindings

pub(crate) struct BglInner {
    d: Rc<DevInner>,
    bindings: Vec<Binding>,
    /// made when its details are known (the header)
    h: RefCell<Option<Handle>>,
}

/// the bindings of one bind group (@group)
#[derive(Clone)]
pub struct BindGroupLayout(pub(crate) Rc<BglInner>);

impl BindGroupLayout {
    // the WebGPU layout, from what `detail` says of each binding (None: its kind's simplest form)
    fn make(&self, detail: impl Fn(&Binding) -> Option<String>) -> Result<u32> {
        if let Some(h) = &*self.0.h.borrow() {
            return Ok(h.h);
        }
        let mut entries = Vec::new();
        for b in &self.0.bindings {
            let kind = match detail(b) {
                Some(j) => j,
                None => String::from(match b.kind {
                    BindingKind::UniformBuffer => "\"buffer\":{\"type\":\"uniform\"}",
                    BindingKind::StorageBuffer => "\"buffer\":{\"type\":\"storage\"}",
                    BindingKind::StorageBufferRead => "\"buffer\":{\"type\":\"read-only-storage\"}",
                    BindingKind::Texture => "\"texture\":{\"sampleType\":\"float\",\"viewDimension\":\"2d\"}",
                    BindingKind::StorageTexture => return Err(GpuError::Invalid("a storage texture binding: neither a bind group nor WGSL says its format yet")),
                    BindingKind::Sampler => "\"sampler\":{\"type\":\"filtering\"}",
                }),
            };
            entries.push(format!("{{\"binding\":{},\"visibility\":{},{kind}}}", b.binding, b.stages.0 & 7));
        }
        let json = format!("{{\"entries\":[{}]}}", entries.join(","));
        let h = create(self.0.d.h, METHOD_BIND_GROUP_LAYOUT, &json)?;
        *self.0.h.borrow_mut() = Some(Handle { h, destroy: false });
        Ok(h)
    }
}

pub(crate) struct PlInner {
    d: Rc<DevInner>,
    pub groups: Vec<BindGroupLayout>,
    pub push: u32,
    h: RefCell<Option<Handle>>,
}

/// the bind group layouts and push constants of pipelines
#[derive(Clone)]
pub struct PipelineLayout(pub(crate) Rc<PlInner>);

impl PipelineLayout {
    // the WebGPU layout; what its groups do not know yet comes from these modules' WGSL
    fn make(&self, modules: &[&ShaderModule]) -> Result<u32> {
        if let Some(h) = &*self.0.h.borrow() {
            return Ok(h.h);
        }
        let mut args = alloc::vec![self.0.d.h, self.0.push, self.0.groups.len() as u32];
        for (g, l) in self.0.groups.iter().enumerate() {
            let h = l.make(|b| {
                if matches!(b.kind, BindingKind::UniformBuffer | BindingKind::StorageBuffer | BindingKind::StorageBufferRead) {
                    return None;
                }
                modules.iter().find_map(|m| m.0.scan.bindings.iter().find(|d| d.group == g as u32 && d.binding == b.binding).map(|d| d.json.clone()))
            })?;
            args.push(h);
        }
        match call(CALL_PIPELINE_LAYOUT, &args) {
            0 => Err(GpuError::Failed(0)),
            h => {
                *self.0.h.borrow_mut() = Some(Handle { h, destroy: false });
                Ok(h)
            }
        }
    }
}

pub(crate) struct BgInner {
    pub h: Handle,
    layout: BindGroupLayout,
    _keep: Vec<Keep>,
}

// what a bind group keeps alive: destroying a buffer or a texture it names would invalidate it
#[allow(dead_code)]
enum Keep {
    Buffer(Buffer),
    View(TextureView),
    Sampler(Sampler),
}

/// resources bound for shaders
#[derive(Clone)]
pub struct BindGroup(pub(crate) Rc<BgInner>);

impl BindGroup {
    pub(crate) fn handle(&self) -> u32 {
        self.0.h.h
    }
    pub(crate) fn fits(&self, l: &BindGroupLayout) -> bool {
        Rc::ptr_eq(&self.0.layout.0, &l.0)
    }
}

impl Device {
    pub fn create_bind_group_layout(&self, bindings: &[Binding]) -> Result<BindGroupLayout> {
        let mut seen = 0u64;
        for b in bindings {
            if b.binding >= 64 || seen & (1 << b.binding) != 0 {
                return Err(GpuError::Invalid("a binding twice, or beyond 63"));
            }
            seen |= 1 << b.binding;
        }
        Ok(BindGroupLayout(Rc::new(BglInner { d: self.d.clone(), bindings: bindings.to_vec(), h: RefCell::new(None) })))
    }

    /// groups: at most BIND_GROUPS_MAX, and with push constants at most 3 (the last is theirs)
    pub fn create_pipeline_layout(&self, desc: &PipelineLayoutDesc) -> Result<PipelineLayout> {
        if desc.groups.len() > BIND_GROUPS_MAX as usize {
            return Err(GpuError::Invalid("too many bind groups"));
        }
        if desc.push_constants > PUSH_CONSTANTS_MAX || desc.push_constants % 4 != 0 {
            return Err(GpuError::Invalid("push constants: a multiple of 4 up to PUSH_CONSTANTS_MAX"));
        }
        if desc.push_constants != 0 && desc.groups.len() > 3 {
            return Err(GpuError::Unsupported("four bind groups and push constants: they take the fourth in the browser"));
        }
        Ok(PipelineLayout(Rc::new(PlInner {
            d: self.d.clone(),
            groups: desc.groups.iter().map(|g| (*g).clone()).collect(),
            push: desc.push_constants,
            h: RefCell::new(None),
        })))
    }

    pub fn create_bind_group(&self, desc: &BindGroupDesc) -> Result<BindGroup> {
        self.d.check()?;
        let bindings = &desc.layout.0.bindings;
        if desc.entries.len() != bindings.len() {
            return Err(GpuError::Invalid("a bind group needs one entry per binding"));
        }
        let entry = |b: u32| desc.entries.iter().find(|e| e.binding == b);
        let layout = desc.layout.make(|b| match (b.kind, entry(b.binding).map(|e| e.resource)) {
            (BindingKind::Texture, Some(Resource::Texture(v))) => {
                let f = v.format();
                let sample = if f.is_depth() {
                    "depth"
                } else if matches!(f, Format::R32Uint | Format::Rg32Uint | Format::Rgba32Uint) {
                    "uint"
                } else if f == Format::R32Sint {
                    "sint"
                } else if matches!(f, Format::R32Float | Format::Rg32Float | Format::Rgba32Float) {
                    "unfilterable-float"
                } else {
                    "float"
                };
                Some(format!("\"texture\":{{\"sampleType\":\"{sample}\",\"viewDimension\":\"{}\"}}", dim_name(v.0.dim)))
            }
            (BindingKind::StorageTexture, Some(Resource::StorageTexture(v))) => {
                Some(format!("\"storageTexture\":{{\"access\":\"write-only\",\"format\":\"{}\",\"viewDimension\":\"{}\"}}", format_name(v.format()), dim_name(v.0.dim)))
            }
            (BindingKind::Sampler, Some(Resource::Sampler(s))) => {
                let t = if s.0.comparison { "comparison" } else if s.0.filtering { "filtering" } else { "non-filtering" };
                Some(format!("\"sampler\":{{\"type\":\"{t}\"}}"))
            }
            _ => None,
        })?;
        let mut keep = Vec::with_capacity(desc.entries.len());
        let mut entries = Vec::with_capacity(desc.entries.len());
        for e in desc.entries {
            let Some(b) = bindings.iter().find(|b| b.binding == e.binding) else {
                return Err(GpuError::Invalid("a bind group entry for a binding the layout lacks"));
            };
            let res = match (b.kind, e.resource) {
                (BindingKind::UniformBuffer | BindingKind::StorageBuffer | BindingKind::StorageBufferRead, Resource::Buffer { buffer, offset, size }) => {
                    let need = if b.kind == BindingKind::UniformBuffer { BufferUsage::UNIFORM } else { BufferUsage::STORAGE };
                    if !buffer.usage().contains(need) {
                        return Err(GpuError::Invalid("a bind group's buffer lacks the usage"));
                    }
                    keep.push(Keep::Buffer(buffer.clone()));
                    let size = if size == 0 { buffer.size().saturating_sub(offset) } else { size };
                    format!("{{\"buffer\":{{\"$\":{}}},\"offset\":{offset},\"size\":{size}}}", buffer.handle())
                }
                (BindingKind::Texture, Resource::Texture(v)) | (BindingKind::StorageTexture, Resource::StorageTexture(v)) => {
                    keep.push(Keep::View(v.clone()));
                    format!("{{\"$\":{}}}", v.handle())
                }
                (BindingKind::Sampler, Resource::Sampler(s)) => {
                    keep.push(Keep::Sampler(s.clone()));
                    format!("{{\"$\":{}}}", s.0.h.h)
                }
                _ => return Err(GpuError::Invalid("a bind group entry of the wrong kind")),
            };
            entries.push(format!("{{\"binding\":{},\"resource\":{res}}}", e.binding));
        }
        let json = format!("{{\"layout\":{{\"$\":{layout}}},\"entries\":[{}]}}", entries.join(","));
        let h = create(self.d.h, METHOD_BIND_GROUP, &json)?;
        Ok(BindGroup(Rc::new(BgInner { h: Handle { h, destroy: false }, layout: desc.layout.clone(), _keep: keep })))
    }

    /// (level Voll)
    pub fn create_bindless(&self, _desc: &BindlessDesc) -> Result<Bindless> {
        Err(GpuError::Unsupported("Bindless: level Voll, not in the browser"))
    }
}

/// a large table of textures, samplers and buffers (level Voll: never in the browser)
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
        Err(GpuError::Unsupported("Bindless"))
    }
    pub fn set_sampler(&self, _slot: u32, _s: &Sampler) -> Result<()> {
        Err(GpuError::Unsupported("Bindless"))
    }
    pub fn set_buffer(&self, _slot: u32, _buf: &Buffer, _offset: u64, _size: u64) -> Result<()> {
        Err(GpuError::Unsupported("Bindless"))
    }
    pub fn set_storage_texture(&self, _slot: u32, _view: &TextureView) -> Result<()> {
        Err(GpuError::Unsupported("Bindless"))
    }
}

// ---------------------------------------------------------------- pipelines

// a stage's descriptor: the checks of docs/GPU.md "Pipeline-Konstanten", WebGPU's constants
fn stage_json(s: &ShaderStage, model: u32, extra: &str) -> Result<String> {
    let scan = &s.module.0.scan;
    if !scan.entries.iter().any(|(m, n)| *m == model && n == s.entry) {
        return Err(GpuError::Invalid("the module has no such entry point for this stage"));
    }
    let mut constants = Vec::with_capacity(s.constants.len());
    for (i, c) in s.constants.iter().enumerate() {
        let Some(o) = scan.overrides.iter().find(|o| o.0 == c.id) else {
            return Err(GpuError::Invalid("a pipeline constant the module does not declare"));
        };
        if s.constants[..i].iter().any(|p| p.id == c.id) {
            return Err(GpuError::Invalid("a pipeline constant twice"));
        }
        if c.value.is_nan() && o.1 != shader::reflect::Kind::Bool || c.value.is_infinite() {
            return Err(GpuError::Invalid("a pipeline constant out of range"));
        }
        let key = if c.id >= 0x10000 { o.3.clone() } else { format!("{}", c.id) };
        let value = if c.value.is_nan() { String::from("0") } else { json_f64(c.value) };
        constants.push(format!("\"{key}\":{value}"));
    }
    Ok(format!("{{\"module\":{{\"$\":{}}},\"entryPoint\":\"{}\",\"constants\":{{{}}}{extra}}}", s.module.0.h.h, s.entry, constants.join(",")))
}

pub(crate) struct PipeInner {
    pub h: Handle,
    pub layout: PipelineLayout,
    pub targets: Vec<Format>,
    pub depth: Option<Format>,
}

/// a compute shader with its layout
#[derive(Clone)]
pub struct ComputePipeline(pub(crate) Rc<PipeInner>);

/// vertex + fragment shaders with their fixed state and target formats
#[derive(Clone)]
pub struct RenderPipeline(pub(crate) Rc<PipeInner>);

fn blend_factor(f: BlendFactor) -> &'static str {
    match f {
        BlendFactor::Zero => "zero",
        BlendFactor::One => "one",
        BlendFactor::Src => "src",
        BlendFactor::OneMinusSrc => "one-minus-src",
        BlendFactor::Dst => "dst",
        BlendFactor::OneMinusDst => "one-minus-dst",
        BlendFactor::SrcAlpha => "src-alpha",
        BlendFactor::OneMinusSrcAlpha => "one-minus-src-alpha",
        BlendFactor::DstAlpha => "dst-alpha",
        BlendFactor::OneMinusDstAlpha => "one-minus-dst-alpha",
    }
}

fn blend_component(c: BlendComponent) -> String {
    let op = match c.op {
        BlendOp::Add => "add",
        BlendOp::Subtract => "subtract",
        BlendOp::ReverseSubtract => "reverse-subtract",
        BlendOp::Min => "min",
        BlendOp::Max => "max",
    };
    format!("{{\"operation\":\"{op}\",\"srcFactor\":\"{}\",\"dstFactor\":\"{}\"}}", blend_factor(c.src), blend_factor(c.dst))
}

impl Device {
    pub fn create_compute_pipeline(&self, desc: &ComputePipelineDesc) -> Result<ComputePipeline> {
        self.d.check()?;
        let layout = desc.layout.make(&[desc.shader.module])?;
        let json = format!("{{\"layout\":{{\"$\":{layout}}},\"compute\":{}}}", stage_json(&desc.shader, 5, "")?);
        let h = create(self.d.h, METHOD_COMPUTE_PIPELINE, &json)?;
        Ok(ComputePipeline(Rc::new(PipeInner { h: Handle { h, destroy: false }, layout: desc.layout.clone(), targets: Vec::new(), depth: None })))
    }

    /// (a required subgroup size: not built in the browser; size 0 is the plain pipeline)
    pub fn create_compute_pipeline_sized(&self, desc: &ComputePipelineDesc, size: u32) -> Result<ComputePipeline> {
        if size != 0 {
            return Err(GpuError::Unsupported("a required subgroup size in the browser"));
        }
        self.create_compute_pipeline(desc)
    }

    pub fn create_render_pipeline(&self, desc: &RenderPipelineDesc) -> Result<RenderPipeline> {
        self.d.check()?;
        let mut modules = alloc::vec![desc.vertex.module];
        if let Some(f) = &desc.fragment {
            modules.push(f.module);
        }
        let layout = desc.layout.make(&modules)?;
        let mut buffers = Vec::with_capacity(desc.buffers.len());
        for b in desc.buffers {
            let attrs: Vec<String> = b.attributes.iter().map(|a| {
                let format = match a.format {
                    VertexFormat::F32 => "float32",
                    VertexFormat::F32x2 => "float32x2",
                    VertexFormat::F32x3 => "float32x3",
                    VertexFormat::F32x4 => "float32x4",
                    VertexFormat::U32 => "uint32",
                    VertexFormat::U32x2 => "uint32x2",
                    VertexFormat::U32x4 => "uint32x4",
                    VertexFormat::Unorm8x4 => "unorm8x4",
                };
                format!("{{\"format\":\"{format}\",\"offset\":{},\"shaderLocation\":{}}}", a.offset, a.location)
            }).collect();
            buffers.push(format!("{{\"arrayStride\":{},\"stepMode\":\"{}\",\"attributes\":[{}]}}", b.stride, if b.per_instance { "instance" } else { "vertex" }, attrs.join(",")));
        }
        let vertex = stage_json(&desc.vertex, 0, &format!(",\"buffers\":[{}]", buffers.join(",")))?;
        let mut json = format!("{{\"layout\":{{\"$\":{layout}}},\"vertex\":{vertex}");
        if let Some(f) = &desc.fragment {
            let targets: Vec<String> = desc.targets.iter().map(|t| {
                let blend = t.blend.map_or(String::new(), |b| format!(",\"blend\":{{\"color\":{},\"alpha\":{}}}", blend_component(b.color), blend_component(b.alpha)));
                format!("{{\"format\":\"{}\",\"writeMask\":{}{blend}}}", format_name(t.format), t.write_mask & 0xF)
            }).collect();
            json.push_str(&format!(",\"fragment\":{}", stage_json(f, 4, &format!(",\"targets\":[{}]", targets.join(",")))?));
        }
        let topology = match desc.topology {
            Topology::Points => "point-list",
            Topology::Lines => "line-list",
            Topology::LineStrip => "line-strip",
            Topology::Triangles => "triangle-list",
            Topology::TriangleStrip => "triangle-strip",
        };
        let cull = match desc.cull {
            Cull::None => "none",
            Cull::Front => "front",
            Cull::Back => "back",
        };
        json.push_str(&format!(",\"primitive\":{{\"topology\":\"{topology}\",\"cullMode\":\"{cull}\",\"frontFace\":\"{}\"}}", if desc.front_cw { "cw" } else { "ccw" }));
        if let Some(d) = desc.depth {
            json.push_str(&format!(",\"depthStencil\":{{\"format\":\"{}\",\"depthWriteEnabled\":{},\"depthCompare\":\"{}\"}}", format_name(d.format), d.write, compare_name(d.compare)));
        }
        json.push('}');
        let h = create(self.d.h, METHOD_RENDER_PIPELINE, &json)?;
        Ok(RenderPipeline(Rc::new(PipeInner {
            h: Handle { h, destroy: false },
            layout: desc.layout.clone(),
            targets: desc.targets.iter().map(|t| t.format).collect(),
            depth: desc.depth.map(|d| d.format),
        })))
    }

    /// (not built in the browser: Features::timestamps is false; WebGPU has them only at the start and
    /// the end of a pass)
    pub fn create_timestamps(&self, _count: u32) -> Result<Timestamps> {
        Err(GpuError::Unsupported("timestamps in the browser"))
    }
}

/// GPU timestamps (not built in the browser)
pub struct Timestamps {
    _none: (),
}

impl Timestamps {
    pub fn count(&self) -> u32 {
        0
    }
    pub fn read_ticks(&self, _first: u32, _n: u32) -> Result<Vec<u64>> {
        Err(GpuError::Unsupported("timestamps in the browser"))
    }
    pub fn read_ms(&self, _first: u32, _n: u32) -> Result<Vec<f64>> {
        Err(GpuError::Unsupported("timestamps in the browser"))
    }
}
