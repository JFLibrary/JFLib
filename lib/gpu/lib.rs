// gpu -- GPU access for the UI, 2D CAD and 3D games: an explicit API in the manner of Vulkan
// and WebGPU over one backend per target (docs/GPU.md).  Backend now: Vulkan on Windows, on Linux,
// there in programs linked against glibc, and on Android (vulkan-1.dll, libvulkan.so.1, libvulkan.so
// loaded at run time; vk_*.rs; on Linux the windows go through dma-bufs, vk_dmabuf.rs).  Planned: WebGPU
// in the browser, Metal; they provide the same types and methods.
//
//   Instance::new() / with(&InstanceDesc) -> Result<Instance>   the system's GPU API;
//                    Err(Disabled) with jfgui_GPU=0, Err(NoLoader) without a loader
//   instance.adapters() -> Result<Vec<Adapter>>   every GPU, discrete ones first
//   instance.adapter() -> Result<Adapter>         the first usable one (jfgui_GPU_ADAPTER: an
//                    index into adapters() or a part of the name)
//   instance.adapters_low_power(display)          the GPUs for showing windows, frugal ones first
//   adapter.info() / features() / limits() / level() / create_device(&DeviceDesc)
//   instance.validating(), validation_messages() -> (errors, warnings)   jfgui_GPU_VALIDATE
//   device.queue() -> &Queue; features(), limits(), info(), level(), is_lost(), poll(),
//          wait(id, timeout_ns), wait_idle(), memory_reserved()
//   device.create_buffer(&BufferDesc)               Buffer: size(), write(off, &[T]),
//                                                   read(off, &mut [T]) (Upload / Readback)
//   device.create_texture(&TextureDesc)             Texture: view(), create_view(&ViewDesc),
//                                                   width(), height(), format(), mips(), layers()
//   device.create_sampler(&SamplerDesc)             Sampler
//   ShaderModule::from_spirv(&device, &[u32])       SPIR-V words (1.3 on Vulkan 1.1, up to 1.6);
//                                                   entry_points(), constants() (its overrides)
//   device.create_bind_group_layout(&[Binding])     BindGroupLayout
//   device.create_pipeline_layout(&PipelineLayoutDesc)   PipelineLayout
//   device.create_bind_group(&BindGroupDesc)        BindGroup
//   device.create_bindless(&BindlessDesc)           Bindless (level Voll): set_texture /
//                                                   set_sampler / set_buffer / set_storage_texture
//   device.create_compute_pipeline(&ComputePipelineDesc)  ComputePipeline
//   device.create_render_pipeline(&RenderPipelineDesc)    RenderPipeline
//                    each ShaderStage with its pipeline constants (WGSL override, SPIR-V
//                    SpecId): &[Constant { id, value: f64 }], converted to the override's type
//                    as WebGPU does; the workgroup size checked against the limits
//   device.create_timestamps(n)                     Timestamps: read_ms(first, n) (since the
//                                                   device's first read), read_ticks
//   device.encoder() -> CommandEncoder              copies, fill, barriers, compute, render
//                    passes, draws, indirect draws, timestamps; finish() -> CommandBuffer
//   queue.submit([cmd, ..]) -> SubmitId             timeline semaphore value
//   queue.write_buffer / write_texture / write_texture_rects / write_texture_rects_at (staged in
//                    a ring, before the next submit), read_buffer (waits)
//   Surface::new(&instance, plat_window)            a plat window (Windows: HWND; Linux: what its
//                                                   display accepts, plat::win_gpu_child)
//   Surface::native(&instance, handles, plat_window)  a window given by its handles (a child);
//                    track_size(): the size for a rebuild on a worker (Linux)
//   device.create_swapchain(&surface, &SwapchainDesc) -> Swapchain: acquire() -> Option<Frame>,
//                    view(&frame), texture(&frame), present(frame, [cmd, ..]), resize(),
//                    try_acquire() (never waits or rebuilds), rebuild(), and for the same code on
//                    every target flush(), busy(), next_look(retry) (a frame reaches a Linux window
//                    after the GPU has finished it: vk_dmabuf.rs), damage(rects)
//
// Conventions shared by every backend (and the WGSL compiler): normalized device coordinates
// with y up and depth 0 ..= 1, framebuffer and texture origin top left (Vulkan: the viewport
// is flipped); front faces counter-clockwise in framebuffer coordinates by default; group g /
// binding b of a pipeline layout = WGSL @group(g) @binding(b) = SPIR-V DescriptorSet g /
// Binding b; push constants: one block from offset 0 visible to every stage, at most
// PUSH_CONSTANTS_MAX bytes; vertex input is optional (vertex pulling from storage buffers).
//
// Feature levels: Basis = what WebGPU and Vulkan 1.1 do (compute, storage buffers, single
// indirect draws and dispatches, texture arrays, timestamps where present); Voll = Vulkan 1.3
// with descriptor indexing (Bindless), draw_indexed_indirect_count, multi-draw indirect, draw
// parameters and subgroup operations.  Mesh shaders and ray tracing are reported in Features,
// not enabled.  Renderers ask device.level() / features().
//
// Errors: every call returns Result; misuse the backend can detect (sizes, alignments, states,
// a draw without pipeline, a Voll command on Basis) is GpuError::Invalid -- recorded commands
// report it from finish().  A lost device (driver reset, GPU removed) is GpuError::DeviceLost
// on every later call; drop the Device and create a new one from the adapter.
//
// Lifetimes: resources are reference-counted handles; a command encoder borrows what it
// records, and a dropped resource is destroyed once the GPU has finished every submission made
// before the drop (the device's timeline).  Nothing waits on the CPU unless asked (wait,
// read_buffer, Timestamps::read_*).
//
// Environment: jfgui_GPU=0 (off), jfgui_GPU_DLL (the loader's file name or path),
// jfgui_GPU_ADAPTER (index or name part), jfgui_GPU_PRESENT=fifo|mailbox|immediate,
// jfgui_GPU_VALIDATE=1 (the Khronos validation layer with synchronization validation when it is
// installed, messages on stderr), jfgui_GPU_DEBUG=1 (the driver's debug messages).
#![no_std]

extern crate alloc;

use alloc::string::String;

mod vk_cmd;
mod vk_dev;
mod vk_ffi;
mod vk_inst;
mod vk_load;
mod vk_mem;
mod vk_res;
#[cfg(not(jfgui_glibc))]
mod vk_swap;
// Linux: images without a VkSurface, handed to the window by plat as dma-bufs; the same types
#[cfg(jfgui_glibc)]
#[path = "vk_dmabuf.rs"]
mod vk_swap;

pub use vk_cmd::{CommandBuffer, CommandEncoder};
pub use vk_dev::{Device, Queue};
pub use vk_inst::{Adapter, Instance, low_power_rank, validation_messages};
pub use vk_res::{BindGroup, BindGroupLayout, Bindless, Buffer, ComputePipeline, PipelineLayout, RenderPipeline, Sampler,
                 ShaderModule, Texture, TextureView, Timestamps};
pub use vk_swap::{Frame, Surface, Swapchain, TryAcquire};

pub type Result<T> = core::result::Result<T, GpuError>;

/// the most push-constant bytes a pipeline layout may declare (every backend guarantees it)
pub const PUSH_CONSTANTS_MAX: u32 = 128;
/// the most bind groups of a pipeline layout
pub const BIND_GROUPS_MAX: u32 = 4;
/// the most colour targets of a render pass
pub const COLOR_TARGETS_MAX: usize = 8;

/// what went wrong
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuError {
    /// the GPU is switched off (jfgui_GPU=0)
    Disabled,
    /// the GPU API is not installed (no vulkan-1.dll or libvulkan.so.1, jfgui_GPU_DLL names a
    /// missing file, or a Linux program not linked against glibc)
    NoLoader,
    /// no usable GPU (or jfgui_GPU_ADAPTER matched none)
    NoAdapter,
    /// the adapter or this backend lacks something (a feature, a format, the level)
    Unsupported(&'static str),
    /// a call the API does not allow (sizes, alignments, states, order)
    Invalid(&'static str),
    /// host or device memory exhausted
    OutOfMemory,
    /// the device was lost (driver reset, GPU removed): recreate it
    DeviceLost,
    /// the window's surface is gone
    SurfaceLost,
    /// a wait ran out of time
    Timeout,
    /// results are not available yet (timestamps not written or not finished)
    NotReady,
    /// another failure of the GPU API (its code: a VkResult)
    Failed(i32),
}

impl core::fmt::Display for GpuError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            GpuError::Disabled => f.write_str("GPU disabled (jfgui_GPU=0)"),
            GpuError::NoLoader => f.write_str("no GPU API installed (Vulkan loader missing)"),
            GpuError::NoAdapter => f.write_str("no usable GPU"),
            GpuError::Unsupported(w) => write!(f, "not supported: {w}"),
            GpuError::Invalid(w) => write!(f, "invalid use: {w}"),
            GpuError::OutOfMemory => f.write_str("out of GPU or host memory"),
            GpuError::DeviceLost => f.write_str("GPU device lost"),
            GpuError::SurfaceLost => f.write_str("window surface lost"),
            GpuError::Timeout => f.write_str("GPU wait timed out"),
            GpuError::NotReady => f.write_str("GPU results not ready"),
            GpuError::Failed(c) => write!(f, "GPU call failed ({c})"),
        }
    }
}

/// plain data that may be copied to and from GPU memory byte for byte: no padding, no
/// pointers, every bit pattern valid
///
/// # Safety
/// Implemented here for the integer and float types and arrays of them.
pub unsafe trait Pod: Copy + 'static {}
// SAFETY: integers and floats have no padding and accept every bit pattern.
unsafe impl Pod for u8 {}
// SAFETY: as above.
unsafe impl Pod for i8 {}
// SAFETY: as above.
unsafe impl Pod for u16 {}
// SAFETY: as above.
unsafe impl Pod for i16 {}
// SAFETY: as above.
unsafe impl Pod for u32 {}
// SAFETY: as above.
unsafe impl Pod for i32 {}
// SAFETY: as above.
unsafe impl Pod for u64 {}
// SAFETY: as above.
unsafe impl Pod for i64 {}
// SAFETY: as above.
unsafe impl Pod for f32 {}
// SAFETY: as above.
unsafe impl Pod for f64 {}
// SAFETY: an array of Pod elements has no padding between them.
unsafe impl<T: Pod, const N: usize> Pod for [T; N] {}

/// the bytes of a slice of plain data
pub fn bytes_of<T: Pod>(s: &[T]) -> &[u8] {
    // SAFETY: T is Pod (no padding), the length covers exactly the slice's memory.
    unsafe { core::slice::from_raw_parts(s.as_ptr() as *const u8, core::mem::size_of_val(s)) }
}

/// the feature level of a device (Basis < Voll)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// WebGPU / Vulkan 1.1: compute, storage buffers, single indirect draws, texture arrays
    Basis,
    /// Vulkan 1.3 with bindless descriptors, indirect count, multi-draw, subgroups
    Voll,
}

/// what kind of GPU
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterKind {
    Discrete,
    Integrated,
    Virtual,
    Cpu,
    Other,
}

/// who an adapter is
#[derive(Clone, Debug)]
pub struct AdapterInfo {
    /// the GPU's name ("NVIDIA GeForce RTX 2070 SUPER")
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    /// the vendor's name for well-known vendor ids ("NVIDIA", "AMD", "Intel", ..)
    pub vendor: &'static str,
    pub kind: AdapterKind,
    /// the driver's name ("NVIDIA") and its information string (the version as the vendor
    /// writes it)
    pub driver: String,
    pub driver_info: String,
    /// the driver version decoded by the vendor's scheme
    pub driver_version: String,
    /// the API version the device supports (major, minor, patch)
    pub api_version: (u32, u32, u32),
    /// "Vulkan", later "WebGPU" / "Metal"
    pub backend: &'static str,
    /// why the backend cannot use this adapter (None: usable)
    pub missing: Option<&'static str>,
}

/// what a device can do beyond the Basis level
#[derive(Clone, Copy, Debug, Default)]
pub struct Features {
    /// GPU timestamps on the queue (Timestamps)
    pub timestamps: bool,
    /// draw_indexed_indirect_multi (several draws from one buffer)
    pub multi_draw_indirect: bool,
    /// a first instance other than 0 in indirect draws
    pub draw_indirect_first_instance: bool,
    /// draw_indexed_indirect_count
    pub draw_indirect_count: bool,
    /// Bindless tables (descriptor indexing, update after bind)
    pub bindless: bool,
    /// subgroup operations (basic, vote, ballot, arithmetic, shuffle) in compute shaders
    pub subgroups: bool,
    /// what WebGPU's `subgroups` feature needs (lib/wgsl's `enable subgroups`): the operations
    /// of `subgroups` plus relative shuffles and quad operations, in compute and fragment shaders
    pub subgroups_webgpu: bool,
    /// the stages with subgroup operations (subgroup_stage::*) and the operations
    /// (subgroup_op::*), as the device reports them
    pub subgroup_stages: u32,
    pub subgroup_ops: u32,
    /// Device::create_compute_pipeline_sized: a required subgroup size (WGSL @subgroup_size;
    /// Limits::subgroup_min_size ..= subgroup_max_size)
    pub subgroup_size_control: bool,
    /// subgroup operations on f16 values too
    pub subgroup_extended_types: bool,
    /// the base vertex / base instance / draw index builtins
    pub draw_parameters: bool,
    pub shader_f16: bool,
    pub shader_f64: bool,
    pub shader_i16: bool,
    pub shader_i64: bool,
    pub shader_i8: bool,
    /// 16 / 8 bit values in storage buffers
    pub storage_16bit: bool,
    /// 16-bit values in uniform buffers and push constants too (WGSL f16 there)
    pub uniform_16bit: bool,
    pub push_constant_16bit: bool,
    /// uniform buffers in the storage layout (uniformBufferStandardLayout: a WGSL uniform
    /// buffer whose layout is not std140, e.g. mat2x2f)
    pub uniform_standard_layout: bool,
    /// OpDemoteToHelperInvocation (lib/wgsl's discard at the level Voll)
    pub demote: bool,
    /// WGSL @builtin(clip_distances), @builtin(primitive_index) (geometryShader),
    /// @interpolate(.., sample) / @builtin(sample_index) (sampleRateShading)
    pub clip_distances: bool,
    pub primitive_index: bool,
    pub sample_shading: bool,
    pub storage_8bit: bool,
    /// BC1 .. BC7 compressed textures
    pub texture_bc: bool,
    pub anisotropy: bool,
    pub depth_clamp: bool,
    /// storage writes in vertex / fragment shaders
    pub vertex_stores: bool,
    pub fragment_stores: bool,
    /// storage textures without a format in the shader
    pub storage_write_without_format: bool,
    /// shader access to buffers by address
    pub buffer_address: bool,
    /// presentation to windows (Swapchain)
    pub swapchain: bool,
    /// present on the adapter, not enabled yet: mesh shaders, ray queries, ray-tracing pipelines
    pub mesh_shader: bool,
    pub ray_query: bool,
    pub ray_pipeline: bool,
}

/// the bits of Features::subgroup_stages
pub mod subgroup_stage {
    pub const COMPUTE: u32 = 1;
    pub const FRAGMENT: u32 = 2;
    pub const VERTEX: u32 = 4;
}

/// the bits of Features::subgroup_ops (the Vulkan feature bits)
pub mod subgroup_op {
    pub const BASIC: u32 = 1;
    pub const VOTE: u32 = 2;
    pub const ARITHMETIC: u32 = 4;
    pub const BALLOT: u32 = 8;
    pub const SHUFFLE: u32 = 0x10;
    pub const SHUFFLE_RELATIVE: u32 = 0x20;
    pub const CLUSTERED: u32 = 0x40;
    pub const QUAD: u32 = 0x80;
}

/// the device's limits
#[derive(Clone, Copy, Debug, Default)]
pub struct Limits {
    pub max_texture_2d: u32,
    pub max_texture_3d: u32,
    pub max_texture_layers: u32,
    /// the largest range of a storage / uniform buffer binding (bytes)
    pub max_storage_binding: u32,
    pub max_uniform_binding: u32,
    /// push-constant bytes of the device (the API allows PUSH_CONSTANTS_MAX)
    pub max_push_constants: u32,
    pub max_bind_groups: u32,
    pub max_compute_workgroup_size: [u32; 3],
    pub max_compute_invocations: u32,
    pub max_compute_workgroups: [u32; 3],
    pub max_compute_shared_memory: u32,
    pub min_uniform_offset_alignment: u32,
    pub min_storage_offset_alignment: u32,
    pub max_color_targets: u32,
    pub max_vertex_buffers: u32,
    pub max_vertex_attributes: u32,
    pub max_draw_indirect_count: u32,
    pub max_anisotropy: f32,
    /// nanoseconds per timestamp tick
    pub timestamp_period_ns: f32,
    /// threads per subgroup (0: unknown)
    pub subgroup_size: u32,
    /// the subgroup sizes shaders can get (without size control: subgroup_size twice)
    pub subgroup_min_size: u32,
    pub subgroup_max_size: u32,
    /// the most slots of a Bindless table (Voll; 0 on Basis)
    pub max_bindless_textures: u32,
    pub max_bindless_samplers: u32,
    pub max_bindless_buffers: u32,
    pub max_bindless_storage_textures: u32,
    /// bytes of the device-local memory heaps
    pub device_memory: u64,
}

/// Instance::with
#[derive(Clone, Copy)]
pub struct InstanceDesc<'a> {
    /// the application's name for the driver
    pub app_name: &'a str,
    /// the validation layer (as jfgui_GPU_VALIDATE=1)
    pub validate: bool,
}

impl InstanceDesc<'_> {
    pub const DEFAULT: InstanceDesc<'static> = InstanceDesc { app_name: "jfgui", validate: false };
}

/// Adapter::create_device
#[derive(Clone, Copy)]
pub struct DeviceDesc {
    /// the highest level to enable (the device gets the highest it supports up to this one)
    pub max_level: Level,
}

impl DeviceDesc {
    pub const DEFAULT: DeviceDesc = DeviceDesc { max_level: Level::Voll };
}

/// a submission's place on the device timeline (Device::wait)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SubmitId(pub u64);

macro_rules! flags {
    ($(#[$m:meta])* $name:ident { $( $(#[$fm:meta])* $f:ident = $v:expr ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $name(pub u32);
        impl $name {
            pub const NONE: $name = $name(0);
            $( $(#[$fm])* pub const $f: $name = $name($v); )*
            pub const fn contains(self, o: $name) -> bool {
                self.0 & o.0 == o.0
            }
            pub const fn intersects(self, o: $name) -> bool {
                self.0 & o.0 != 0
            }
        }
        impl core::ops::BitOr for $name {
            type Output = $name;
            fn bitor(self, o: $name) -> $name {
                $name(self.0 | o.0)
            }
        }
    };
}

flags! {
    /// how a buffer is used (copies to and from it are always allowed)
    BufferUsage {
        UNIFORM = 1,
        STORAGE = 2,
        INDEX = 4,
        VERTEX = 8,
        /// indirect draw / dispatch arguments and draw counts
        INDIRECT = 16,
    }
}

flags! {
    /// how a texture is used
    TextureUsage {
        COPY_SRC = 1,
        COPY_DST = 2,
        SAMPLED = 4,
        STORAGE = 8,
        COLOR_TARGET = 16,
        DEPTH_TARGET = 32,
    }
}

flags! {
    /// shader stages that see a binding
    Stages {
        VERTEX = 1,
        FRAGMENT = 2,
        COMPUTE = 4,
        ALL = 7,
    }
}

flags! {
    /// memory accesses for CommandEncoder::barrier: what must finish (before) and what waits
    /// (after); textures change layout with texture_barrier instead
    Access {
        COPY_READ = 1,
        COPY_WRITE = 2,
        COMPUTE_READ = 4,
        COMPUTE_WRITE = 8,
        /// vertex and fragment shader reads (storage, uniform, sampled)
        GRAPHICS_READ = 16,
        GRAPHICS_WRITE = 32,
        /// indirect arguments and draw counts
        INDIRECT = 64,
        /// index and vertex buffers
        INDEX_VERTEX = 128,
        COLOR_TARGET = 256,
        DEPTH_TARGET = 512,
        /// the CPU through Buffer::read / write
        HOST_READ = 1024,
        HOST_WRITE = 2048,
    }
}

/// where a buffer's memory lives
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    /// GPU memory (fastest for the GPU; filled by copies or Queue::write_buffer)
    Device,
    /// CPU memory the GPU reads (staging, per-frame data; Buffer::write)
    Upload,
    /// CPU memory the GPU writes (results; Buffer::read)
    Readback,
}

/// Device::create_buffer
#[derive(Clone, Copy, Debug)]
pub struct BufferDesc {
    pub size: u64,
    pub usage: BufferUsage,
    pub memory: MemoryKind,
}

/// texel formats
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    R8Unorm,
    Rg8Unorm,
    Rgba8Unorm,
    Rgba8Srgb,
    Bgra8Unorm,
    Bgra8Srgb,
    Rgb10a2Unorm,
    Rg11b10Float,
    R16Float,
    Rg16Float,
    Rgba16Float,
    R32Uint,
    R32Sint,
    R32Float,
    Rg32Uint,
    Rg32Float,
    Rgba32Uint,
    Rgba32Float,
    Depth16,
    Depth32Float,
    Depth24Stencil8,
    Depth32FloatStencil8,
    Bc1Unorm,
    Bc1Srgb,
    Bc3Unorm,
    Bc3Srgb,
    Bc4Unorm,
    Bc5Unorm,
    Bc7Unorm,
    Bc7Srgb,
}

impl Format {
    /// bytes per texel block (a texel, or a 4 x 4 block of a compressed format)
    pub const fn block_bytes(self) -> u32 {
        use Format::*;
        match self {
            R8Unorm => 1,
            Rg8Unorm | R16Float | Depth16 => 2,
            Rgba8Unorm | Rgba8Srgb | Bgra8Unorm | Bgra8Srgb | Rgb10a2Unorm | Rg11b10Float | Rg16Float | R32Uint
            | R32Sint | R32Float | Depth32Float | Depth24Stencil8 => 4,
            Rgba16Float | Rg32Uint | Rg32Float | Depth32FloatStencil8 => 8,
            Rgba32Uint | Rgba32Float => 16,
            Bc1Unorm | Bc1Srgb | Bc4Unorm => 8,
            Bc3Unorm | Bc3Srgb | Bc5Unorm | Bc7Unorm | Bc7Srgb => 16,
        }
    }
    /// the block's width and height in texels (4 for compressed formats, else 1)
    pub const fn block_dim(self) -> u32 {
        if self.is_compressed() { 4 } else { 1 }
    }
    pub const fn is_compressed(self) -> bool {
        use Format::*;
        matches!(self, Bc1Unorm | Bc1Srgb | Bc3Unorm | Bc3Srgb | Bc4Unorm | Bc5Unorm | Bc7Unorm | Bc7Srgb)
    }
    pub const fn is_depth(self) -> bool {
        matches!(self, Format::Depth16 | Format::Depth32Float | Format::Depth24Stencil8 | Format::Depth32FloatStencil8)
    }
    pub const fn has_stencil(self) -> bool {
        matches!(self, Format::Depth24Stencil8 | Format::Depth32FloatStencil8)
    }
    pub const fn is_srgb(self) -> bool {
        matches!(self, Format::Rgba8Srgb | Format::Bgra8Srgb | Format::Bc1Srgb | Format::Bc3Srgb | Format::Bc7Srgb)
    }
}

/// Device::create_texture: a 2D texture (layers > 1: an array; cube: 6 * n layers) or a 3D one
/// (depth > 1)
#[derive(Clone, Copy, Debug)]
pub struct TextureDesc {
    pub width: u32,
    pub height: u32,
    /// 1 for 2D textures
    pub depth: u32,
    /// 1 for a single texture
    pub layers: u32,
    /// mip levels (1 = none)
    pub mips: u32,
    pub format: Format,
    pub usage: TextureUsage,
    /// cube faces: layers is a multiple of 6
    pub cube: bool,
}

impl TextureDesc {
    /// a single-level 2D texture
    pub const fn d2(width: u32, height: u32, format: Format, usage: TextureUsage) -> TextureDesc {
        TextureDesc { width, height, depth: 1, layers: 1, mips: 1, format, usage, cube: false }
    }
}

/// a view's dimension
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewDim {
    D2,
    D2Array,
    Cube,
    CubeArray,
    D3,
}

/// Texture::create_view: a range of mips and layers (counts 0 = to the end)
#[derive(Clone, Copy, Debug)]
pub struct ViewDesc {
    pub dim: ViewDim,
    pub base_mip: u32,
    pub mips: u32,
    pub base_layer: u32,
    pub layers: u32,
}

/// the state a texture is in (texture_barrier, render pass attachments, write_texture)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// contents discarded; waits for earlier colour, copy and compute work (and for the
    /// swapchain image's acquisition)
    Undefined,
    /// storage textures and anything at once (slower on some GPUs)
    General,
    CopySrc,
    CopyDst,
    /// sampled in shaders (also depth textures)
    Sampled,
    ColorTarget,
    DepthTarget,
    /// depth test without writes while shaders sample it
    DepthRead,
    /// handed to the window (the last layout of a swapchain image)
    Present,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressMode {
    Repeat,
    MirrorRepeat,
    ClampToEdge,
}

/// comparison functions (depth test, comparison samplers)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compare {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

/// Device::create_sampler
#[derive(Clone, Copy, Debug)]
pub struct SamplerDesc {
    pub mag: Filter,
    pub min: Filter,
    pub mip: Filter,
    /// u, v, w
    pub address: [AddressMode; 3],
    /// 1 = off, up to Limits::max_anisotropy (needs Features::anisotropy)
    pub anisotropy: u16,
    /// a comparison sampler (shadow maps)
    pub compare: Option<Compare>,
    pub lod_min: f32,
    pub lod_max: f32,
}

impl SamplerDesc {
    pub const LINEAR: SamplerDesc = SamplerDesc {
        mag: Filter::Linear,
        min: Filter::Linear,
        mip: Filter::Linear,
        address: [AddressMode::ClampToEdge; 3],
        anisotropy: 1,
        compare: None,
        lod_min: 0.0,
        lod_max: 1000.0,
    };
    pub const NEAREST: SamplerDesc =
        SamplerDesc { mag: Filter::Nearest, min: Filter::Nearest, mip: Filter::Nearest, ..SamplerDesc::LINEAR };
}

/// what a binding holds
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingKind {
    UniformBuffer,
    /// read and written by shaders
    StorageBuffer,
    /// read only (also visible to vertex shaders without Features::vertex_stores)
    StorageBufferRead,
    /// a sampled texture (Layout::Sampled)
    Texture,
    /// a storage texture (Layout::General)
    StorageTexture,
    Sampler,
}

/// one binding of a BindGroupLayout
#[derive(Clone, Copy, Debug)]
pub struct Binding {
    pub binding: u32,
    pub kind: BindingKind,
    pub stages: Stages,
}

/// Device::create_pipeline_layout
#[derive(Clone, Copy)]
pub struct PipelineLayoutDesc<'a> {
    /// bind group g = @group(g); at most BIND_GROUPS_MAX
    pub groups: &'a [&'a BindGroupLayout],
    /// push-constant bytes (a multiple of 4, at most PUSH_CONSTANTS_MAX)
    pub push_constants: u32,
}

/// a resource bound into a BindGroup
#[derive(Clone, Copy)]
pub enum Resource<'a> {
    /// a buffer range (size 0: to the end); offsets aligned to Limits::min_*_offset_alignment
    Buffer { buffer: &'a Buffer, offset: u64, size: u64 },
    Texture(&'a TextureView),
    StorageTexture(&'a TextureView),
    Sampler(&'a Sampler),
}

#[derive(Clone, Copy)]
pub struct BindEntry<'a> {
    pub binding: u32,
    pub resource: Resource<'a>,
}

/// Device::create_bind_group: one entry per binding of the layout
#[derive(Clone, Copy)]
pub struct BindGroupDesc<'a> {
    pub layout: &'a BindGroupLayout,
    pub entries: &'a [BindEntry<'a>],
}

/// Device::create_bindless: the slots of each kind (bindings 0 textures, 1 samplers, 2 storage
/// buffers, 3 storage textures of one group)
#[derive(Clone, Copy, Debug)]
pub struct BindlessDesc {
    pub textures: u32,
    pub samplers: u32,
    pub buffers: u32,
    pub storage_textures: u32,
}

/// a pipeline-overridable constant: WGSL `@id(id) override`, SPIR-V `SpecId id`.  The value
/// is an f64 as in WebGPU's `GPUProgrammableStage.constants` and is converted to the
/// override's type the way WebGPU converts it: bool true unless 0 or NaN; i32 / u32 (and the
/// other integers) truncated toward 0, Invalid when not finite or out of range; f32 / f16 /
/// f64 rounded to nearest, Invalid when not finite or out of range.  An id the module does not
/// declare, or the same id twice, is Invalid (Vulkan would ignore it; WebGPU does not).
/// Overrides the pipeline leaves out keep the module's default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Constant {
    pub id: u32,
    pub value: f64,
}

/// a shader entry point and its pipeline constants
#[derive(Clone, Copy)]
pub struct ShaderStage<'a> {
    pub module: &'a ShaderModule,
    pub entry: &'a str,
    /// pipeline-overridable constants of this stage (empty: the module's defaults)
    pub constants: &'a [Constant],
}

impl<'a> ShaderStage<'a> {
    /// an entry point without pipeline constants
    pub const fn new(module: &'a ShaderModule, entry: &'a str) -> ShaderStage<'a> {
        ShaderStage { module, entry, constants: &[] }
    }
}

/// Device::create_compute_pipeline
#[derive(Clone, Copy)]
pub struct ComputePipelineDesc<'a> {
    pub layout: &'a PipelineLayout,
    pub shader: ShaderStage<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topology {
    Points,
    Lines,
    LineStrip,
    Triangles,
    TriangleStrip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cull {
    None,
    Front,
    Back,
}

/// vertex attribute formats
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VertexFormat {
    F32,
    F32x2,
    F32x3,
    F32x4,
    U32,
    U32x2,
    U32x4,
    /// four bytes as 0 ..= 1
    Unorm8x4,
}

#[derive(Clone, Copy, Debug)]
pub struct VertexAttribute {
    pub location: u32,
    pub format: VertexFormat,
    pub offset: u32,
}

/// a vertex buffer slot (empty list: vertex pulling)
#[derive(Clone, Copy)]
pub struct VertexBufferLayout<'a> {
    pub stride: u32,
    pub per_instance: bool,
    pub attributes: &'a [VertexAttribute],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendFactor {
    Zero,
    One,
    Src,
    OneMinusSrc,
    Dst,
    OneMinusDst,
    SrcAlpha,
    OneMinusSrcAlpha,
    DstAlpha,
    OneMinusDstAlpha,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendOp {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlendComponent {
    pub src: BlendFactor,
    pub dst: BlendFactor,
    pub op: BlendOp,
}

/// colour and alpha blending of a colour target
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blend {
    pub color: BlendComponent,
    pub alpha: BlendComponent,
}

impl Blend {
    /// premultiplied alpha: src + dst * (1 - src.a)
    pub const PREMULTIPLIED: Blend = Blend {
        color: BlendComponent { src: BlendFactor::One, dst: BlendFactor::OneMinusSrcAlpha, op: BlendOp::Add },
        alpha: BlendComponent { src: BlendFactor::One, dst: BlendFactor::OneMinusSrcAlpha, op: BlendOp::Add },
    };
    /// straight alpha: src * src.a + dst * (1 - src.a)
    pub const ALPHA: Blend = Blend {
        color: BlendComponent { src: BlendFactor::SrcAlpha, dst: BlendFactor::OneMinusSrcAlpha, op: BlendOp::Add },
        alpha: BlendComponent { src: BlendFactor::One, dst: BlendFactor::OneMinusSrcAlpha, op: BlendOp::Add },
    };
    /// additive: src + dst
    pub const ADD: Blend = Blend {
        color: BlendComponent { src: BlendFactor::One, dst: BlendFactor::One, op: BlendOp::Add },
        alpha: BlendComponent { src: BlendFactor::One, dst: BlendFactor::One, op: BlendOp::Add },
    };
}

/// a colour target of a render pipeline
#[derive(Clone, Copy, Debug)]
pub struct ColorTarget {
    pub format: Format,
    pub blend: Option<Blend>,
    /// bits 0..3 = r, g, b, a (0xF: all)
    pub write_mask: u32,
}

/// the depth test of a render pipeline
#[derive(Clone, Copy, Debug)]
pub struct DepthState {
    pub format: Format,
    pub write: bool,
    pub compare: Compare,
}

/// Device::create_render_pipeline
#[derive(Clone, Copy)]
pub struct RenderPipelineDesc<'a> {
    pub layout: &'a PipelineLayout,
    pub vertex: ShaderStage<'a>,
    pub fragment: Option<ShaderStage<'a>>,
    /// vertex buffer slots (empty: the vertex shader pulls its data)
    pub buffers: &'a [VertexBufferLayout<'a>],
    pub topology: Topology,
    pub cull: Cull,
    /// front faces are clockwise (default counter-clockwise)
    pub front_cw: bool,
    pub targets: &'a [ColorTarget],
    pub depth: Option<DepthState>,
}

/// what a render pass does with an attachment's old contents
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Load {
    Keep,
    Clear,
    Discard,
}

/// a colour attachment of a render pass: the view goes from `before` to ColorTarget at the
/// start and to `after` at the end
#[derive(Clone, Copy)]
pub struct ColorAttachment<'a> {
    pub view: &'a TextureView,
    pub load: Load,
    pub clear: [f32; 4],
    pub store: bool,
    pub before: Layout,
    pub after: Layout,
}

/// the depth attachment of a render pass (DepthTarget during the pass)
#[derive(Clone, Copy)]
pub struct DepthAttachment<'a> {
    pub view: &'a TextureView,
    pub load: Load,
    pub clear: f32,
    pub store: bool,
    pub before: Layout,
    pub after: Layout,
}

/// CommandEncoder::begin_render (the slice may be a temporary: only the views are borrowed
/// for the encoder's lifetime)
#[derive(Clone, Copy)]
pub struct RenderPassDesc<'d, 'a> {
    pub color: &'d [ColorAttachment<'a>],
    pub depth: Option<DepthAttachment<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexFormat {
    U16,
    U32,
}

/// a region of one mip of a texture: origin and size in texels (z: 3D textures), layers
#[derive(Clone, Copy)]
pub struct TextureRegion<'a> {
    pub texture: &'a Texture,
    pub mip: u32,
    pub origin: [u32; 3],
    pub size: [u32; 3],
    pub base_layer: u32,
    pub layers: u32,
}

impl<'a> TextureRegion<'a> {
    /// all of mip `mip` and every layer
    pub fn mip(texture: &'a Texture, mip: u32) -> TextureRegion<'a> {
        let s = |v: u32| (v >> mip).max(1);
        TextureRegion {
            texture,
            mip,
            origin: [0; 3],
            size: [s(texture.width()), s(texture.height()), s(texture.depth())],
            base_layer: 0,
            layers: texture.layers(),
        }
    }
}

/// the layout of texel data in a buffer or slice: rows of bytes_per_row bytes (0: tightly
/// packed), rows_per_image rows per layer / slice (0: the region's height)
#[derive(Clone, Copy, Debug, Default)]
pub struct DataLayout {
    pub offset: u64,
    pub bytes_per_row: u32,
    pub rows_per_image: u32,
}

/// how frames reach the window
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentMode {
    /// vsync: one image per refresh, never tears (always available)
    Fifo,
    /// the newest image at the refresh, older ones dropped (no tearing, low latency)
    Mailbox,
    /// at once (tears; for measurements)
    Immediate,
}

/// Device::create_swapchain
#[derive(Clone, Copy, Debug)]
pub struct SwapchainDesc {
    pub present: PresentMode,
    /// an sRGB format (the GPU encodes gamma on writes)
    pub srgb: bool,
    /// the images may be copied from (read-back of the presented image)
    pub copy_src: bool,
    /// frames the CPU may record ahead of the GPU (1 ..= 3)
    pub frames_in_flight: u32,
    /// images of the swapchain at least (0: the surface's minimum + 1)
    pub min_images: u32,
}

impl SwapchainDesc {
    pub const DEFAULT: SwapchainDesc =
        SwapchainDesc { present: PresentMode::Fifo, srgb: false, copy_src: false, frames_in_flight: 2, min_images: 0 };
}
