// vk_dev -- Vulkan device and queue: creation, the timeline, deferred destruction, command
// buffer slots, staged uploads, submission
//
//   adapter.create_device(&DeviceDesc) -> Result<Device>
//        one queue (the first family with graphics + compute, presentation checked per
//        surface); level Voll when the adapter has it and desc.max_level allows it.  Enabled:
//        timeline semaphores, dynamic rendering (core 1.3 or the KHR extensions on 1.1 / 1.2),
//        VK_KHR_swapchain when the instance has surfaces, the optional features of Features,
//        the Vulkan memory model only together with its device scope (else Device-scope
//        atomics, as in the benchmark's cull, would be invalid); on Voll also descriptor
//        indexing (update after bind, partially bound, non-uniform indexing) and
//        draw-indirect-count; maintenance4 on Vulkan 1.3 (LocalSizeId); for shaders compiled
//        from WGSL (lib/wgsl): f16 in uniform buffers and push constants, the standard uniform
//        buffer layout, demote-to-helper (discard on Vulkan 1.3), clip distances, the
//        primitive index (geometryShader), sample-rate shading, dual-source blending in
//        shaders, the extended storage image formats, storage reads without format.  Not
//        enabled: robust buffer access (speed; lib/wgsl's back end clamps indices instead).
//   device.queue() -> &Queue        queue.submit([cmd, ..]) -> Result<SubmitId>, submit_after(&[id], [cmd, ..])
//   device.compute_queue() -> Option<&ComputeQueue>    (G5d) a queue of its own for compute and copies when
//        DeviceOptions::compute_queue asked for it (Adapter::create_device_with): a family without graphics where the
//        device has one (jfgui_GPU_COMPUTE=0: none); encoder(), submit, submit_after
//   queue.write_buffer(&buf, off, &[T])            staged into an upload command buffer that
//   queue.write_texture(&region, &[u8], layout, before, after)   runs first in the next submit
//   queue.write_texture_rects(&tex, &rects, &[u8], stride, before, after)   rectangles of a frame
//   queue.write_texture_rects_at(&tex, &rects, origin, &[u8], stride, before, after)   of a larger image
//   queue.read_buffer(&buf, off, &mut [T])          copies, submits and waits (slow path)
//   queue.flush() -> Result<SubmitId>               submit the pending uploads now
//   device.wait(id, timeout_ns) -> Result<bool>     false: timed out
//   device.wait_idle(), poll() (frees what the GPU has finished with), is_lost(),
//   test_lose() (a test aid: the device behaves as lost from now on)
//
// The timeline: one timeline semaphore per device; every submit signals the next value
// (SubmitId).  completed caches the last value seen reached (vkGetSemaphoreCounterValue at
// submits, polls and waits).  A dropped resource goes to the garbage with the value of the
// latest submit and is destroyed once that value is reached -- earlier if no submit is
// pending; a buffer or image, which the open upload command buffer may name, with the value of
// the next submit, which runs it.  Command buffers come from slots (a transient command pool with one buffer each),
// reused once their submission completed.  Uploads: the data goes into the staging ring, the
// copies into one upload command buffer that ends with a global barrier (copy writes -> every
// later access) and is put before the caller's command buffers in the next vkQueueSubmit.
// The staging ring: 256 KiB chunks of Upload memory, filled front to back across submissions;
// a full chunk is reused once the GPU has passed the last submission that reads from it (two
// are kept free, more go back).  An upload above half a chunk gets a buffer of its own, reused
// by the next one that fits and released when none came for a second (Device::poll): a frame of
// small rectangles costs what they hold, not a full frame or a new allocation.
// Device lost: VK_ERROR_DEVICE_LOST from any call sets the device's lost flag; every later
// call returns GpuError::DeviceLost (objects are still destroyed normally).
// (G5d) The compute queue comes only when asked for (DeviceOptions; without it a device is what it was in 0.1.0: one
// queue, every resource exclusive).  It has a timeline of its own (two queues signalling one timeline semaphore could reach its
// values out of order); its SubmitIds carry COMPUTE_ID.  A submission waits for the other queue's values through
// the timeline (submit_after, at all commands).  A dropped resource waits for the latest submission of both queues;
// command slots come from a pool of each family.  Resources are shared between the families concurrently: every
// buffer, and every texture that is no render target (render targets stay exclusive to the device's queue: render
// passes run there only, and some GPUs compress exclusive targets) -- no ownership transfers in the caller's code.
// Pending uploads run on the device's queue: a compute submission flushes them first and waits for them.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rt::sync::SpinLock;

use crate::vk_ffi as vk;
use crate::vk_inst::{Adapter, InstShared, err};
use crate::vk_mem::{Alloc, Allocator};
use crate::{AdapterInfo, DataLayout, DeviceDesc, DeviceOptions, Features, GpuError, Layout, Level, Limits, MemoryKind, Pod, Result,
            SubmitId, TextureRegion};

/// what waits for the GPU before it is destroyed
pub(crate) enum Garbage {
    Buffer(vk::Handle, Alloc),
    Image(vk::Handle, Option<Alloc>),
    View(vk::Handle),
    Sampler(vk::Handle),
    Shader(vk::Handle),
    SetLayout(vk::Handle),
    PipeLayout(vk::Handle),
    Pipeline(vk::Handle),
    Set(vk::Handle, vk::Handle),
    Pool(vk::Handle),
    Query(vk::Handle),
    #[cfg_attr(jfgui_glibc, allow(dead_code))] // (vk_swap.rs)
    Semaphore(vk::Handle),
    #[cfg_attr(jfgui_glibc, allow(dead_code))]
    Swapchain(vk::Handle),
    #[cfg_attr(not(jfgui_glibc), allow(dead_code))] // (vk_dmabuf.rs: an image's dedicated allocation)
    Memory(vk::Handle),
}

/// a command pool with its one command buffer (of the compute queue's family, or the device queue's)
pub(crate) struct Slot {
    pub pool: vk::Handle,
    pub cb: vk::CommandBuffer,
    pub compute: bool,
}

/// (G5d) the bit of a SubmitId of the compute queue (its timeline's value below it)
pub(crate) const COMPUTE_ID: u64 = 1 << 63;

/// (G5d) the compute queue: its family, its timeline, what was submitted and is known done
pub(crate) struct Lane {
    pub queue: vk::Queue,
    pub family: u32,
    pub timeline: vk::Handle,
    pub ts_bits: u32,
    submitted: AtomicU64,
    completed: AtomicU64,
    qlock: SpinLock<()>,
}

/// a buffer of the staging ring
struct Chunk {
    buf: vk::Handle,
    a: Alloc,
    used: u64, // bytes used, front to back
}

/// the pending staged uploads: the command buffer, the chunks full since the last submission
/// that it reads from
struct Upload {
    slot: Slot,
    retired: Vec<Chunk>,
}

/// the staging ring
#[derive(Default)]
struct Ring {
    cur: Option<Chunk>, // the chunk being filled (what is behind `used` stays free across submissions)
    cur_value: u64,     // the last submission that reads from it (0: none)
    cur_open: bool,     // the open upload command buffer reads from it
    busy: Vec<(u64, Chunk)>, // retired: free once the GPU reached the value
    free: Vec<Chunk>,
    big_at: u64,        // plat::time_us of the last upload above half a chunk
    big_size: u64,      // its bytes
}

pub(crate) struct State {
    pub mem: Allocator,
    /// (the device queue's value, the compute queue's)
    garbage: Vec<(u64, u64, Garbage)>,
    free: Vec<Slot>,
    /// (G5d) free slots of the compute queue's family
    cfree: Vec<Slot>,
    /// (a SubmitId's value: COMPUTE_ID for the compute queue)
    busy: Vec<(u64, Slot)>,
    pools: Vec<vk::Handle>, // descriptor pools of the bind groups (the last one is filled)
    upload: Option<Upload>,
    ring: Ring,
}

/// a chunk of the staging ring
const CHUNK: u64 = 256 << 10;
/// a large chunk stays while large uploads need at least this part of it
const BIG_KEEP_RATIO: u64 = 8;
/// free chunks kept beyond the one being filled
const CHUNKS_FREE: usize = 2;
/// an upload above half a chunk keeps its buffer this long after the last one (us)
const BIG_KEEP_US: u64 = 1_000_000;

pub(crate) struct Shared {
    pub inst: Arc<InstShared>,
    pub pd: vk::PhysicalDevice,
    pub dev: vk::Device,
    pub f: vk::DeviceFns,
    pub queue: vk::Queue,
    pub family: u32,
    pub timeline: vk::Handle,
    pub info: AdapterInfo,
    pub features: Features,
    pub limits: Limits,
    pub level: Level,
    pub raw: vk::PhysicalDeviceLimits,
    pub ts_mask: u64,
    #[cfg_attr(jfgui_glibc, allow(dead_code))] // (vk_swap.rs)
    pub swapchain: bool,
    /// images for a window go to VK_QUEUE_FAMILY_FOREIGN_EXT (vk_dmabuf.rs)
    #[cfg_attr(not(jfgui_glibc), allow(dead_code))]
    pub queue_foreign: bool,
    /// the first timestamp tick read on the device: Timestamps::read_ms counts from it
    pub ts_base: AtomicU64,
    /// vkResetQueryPool from the host (hostQueryReset enabled)
    pub host_query_reset: bool,
    /// maintenance4 enabled (Vulkan 1.3): shaders may set their workgroup size with LocalSizeId
    pub maintenance4: bool,
    /// (G5d) the compute queue
    pub lane: Option<Lane>,
    lost: AtomicBool,
    submitted: AtomicU64,
    completed: AtomicU64,
    qlock: SpinLock<()>,
    pub st: SpinLock<State>,
}

/// a logical GPU device: creates resources, records commands, owns the queue
#[derive(Clone)]
pub struct Device {
    pub(crate) sh: Arc<Shared>,
    queue: Queue,
    compute: Option<ComputeQueue>,
}

/// (G5d) a queue of its own for compute shaders and copies, beside the device's queue (Device::compute_queue): its
/// encoders record no render passes, its submissions run in parallel with the device queue's unless one waits for the
/// other (submit_after)
#[derive(Clone)]
pub struct ComputeQueue {
    pub(crate) sh: Arc<Shared>,
}

/// the device's queue (graphics, compute, copies, presentation)
#[derive(Clone)]
pub struct Queue {
    pub(crate) sh: Arc<Shared>,
}

// features enabled when present (indices into the VkBool32 arrays)
const E10: &[usize] = &[
    vk::F10_FULL_DRAW_INDEX_UINT32,
    vk::F10_IMAGE_CUBE_ARRAY,
    vk::F10_INDEPENDENT_BLEND,
    vk::F10_GEOMETRY_SHADER,
    vk::F10_SAMPLE_RATE_SHADING,
    vk::F10_DUAL_SRC_BLEND,
    vk::F10_MULTI_DRAW_INDIRECT,
    vk::F10_DRAW_INDIRECT_FIRST_INSTANCE,
    vk::F10_DEPTH_CLAMP,
    vk::F10_FILL_MODE_NON_SOLID,
    vk::F10_SAMPLER_ANISOTROPY,
    vk::F10_TEXTURE_COMPRESSION_BC,
    vk::F10_VERTEX_PIPELINE_STORES_AND_ATOMICS,
    vk::F10_FRAGMENT_STORES_AND_ATOMICS,
    vk::F10_SHADER_STORAGE_IMAGE_EXTENDED_FORMATS,
    vk::F10_SHADER_STORAGE_IMAGE_READ_WITHOUT_FORMAT,
    vk::F10_SHADER_STORAGE_IMAGE_WRITE_WITHOUT_FORMAT,
    vk::F10_SHADER_UNIFORM_BUFFER_ARRAY_DYNAMIC_INDEXING,
    vk::F10_SHADER_SAMPLED_IMAGE_ARRAY_DYNAMIC_INDEXING,
    vk::F10_SHADER_STORAGE_BUFFER_ARRAY_DYNAMIC_INDEXING,
    vk::F10_SHADER_STORAGE_IMAGE_ARRAY_DYNAMIC_INDEXING,
    vk::F10_SHADER_CLIP_DISTANCE,
    vk::F10_SHADER_FLOAT64,
    vk::F10_SHADER_INT64,
    vk::F10_SHADER_INT16,
];
const E11: &[usize] = &[
    vk::F11_STORAGE_BUFFER_16BIT_ACCESS,
    vk::F11_UNIFORM_AND_STORAGE_BUFFER_16BIT_ACCESS,
    vk::F11_STORAGE_PUSH_CONSTANT16,
    vk::F11_SHADER_DRAW_PARAMETERS,
];
const E12: &[usize] = &[
    vk::F12_STORAGE_BUFFER_8BIT_ACCESS,
    vk::F12_SHADER_FLOAT16,
    vk::F12_SHADER_INT8,
    vk::F12_SCALAR_BLOCK_LAYOUT,
    vk::F12_UNIFORM_BUFFER_STANDARD_LAYOUT,
    vk::F12_SHADER_SUBGROUP_EXTENDED_TYPES,
    vk::F12_HOST_QUERY_RESET,
    vk::F12_TIMELINE_SEMAPHORE,
    vk::F12_BUFFER_DEVICE_ADDRESS,
    vk::F12_VULKAN_MEMORY_MODEL,
    vk::F12_VULKAN_MEMORY_MODEL_DEVICE_SCOPE,
];
const E12_VOLL: &[usize] = &[
    vk::F12_DRAW_INDIRECT_COUNT,
    vk::F12_DESCRIPTOR_INDEXING,
    vk::F12_SHADER_SAMPLED_IMAGE_ARRAY_NON_UNIFORM_INDEXING,
    vk::F12_SHADER_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
    vk::F12_SHADER_STORAGE_IMAGE_ARRAY_NON_UNIFORM_INDEXING,
    vk::F12_DESCRIPTOR_BINDING_SAMPLED_IMAGE_UPDATE_AFTER_BIND,
    vk::F12_DESCRIPTOR_BINDING_STORAGE_IMAGE_UPDATE_AFTER_BIND,
    vk::F12_DESCRIPTOR_BINDING_STORAGE_BUFFER_UPDATE_AFTER_BIND,
    vk::F12_DESCRIPTOR_BINDING_UPDATE_UNUSED_WHILE_PENDING,
    vk::F12_DESCRIPTOR_BINDING_PARTIALLY_BOUND,
    vk::F12_RUNTIME_DESCRIPTOR_ARRAY,
];
const E13: &[usize] = &[
    vk::F13_SHADER_DEMOTE_TO_HELPER_INVOCATION,
    vk::F13_SUBGROUP_SIZE_CONTROL,
    vk::F13_SYNCHRONIZATION2,
    vk::F13_DYNAMIC_RENDERING,
    vk::F13_MAINTENANCE4,
];

fn pick<const N: usize>(have: &[u32; N], want: &[usize]) -> [u32; N] {
    let mut out = [0u32; N];
    for &i in want {
        out[i] = have[i];
    }
    out
}

impl Adapter {
    /// a device of this adapter (level: the highest up to desc.max_level), with one queue
    pub fn create_device(&self, desc: &DeviceDesc) -> Result<Device> {
        self.create_device_with(desc, &DeviceOptions::DEFAULT)
    }

    /// (G5d) a device of this adapter with options: a compute queue of its own (Device::compute_queue) where the
    /// adapter has a family without graphics and jfgui_GPU_COMPUTE is not 0
    pub fn create_device_with(&self, desc: &DeviceDesc, opts: &DeviceOptions) -> Result<Device> {
        if let Some(why) = self.info.missing {
            return Err(GpuError::Unsupported(why));
        }
        let inst = &self.inst;
        let level = if self.voll && desc.max_level == Level::Voll { Level::Voll } else { Level::Basis };
        let voll = level == Level::Voll;
        let v12 = self.api >= vk::API_VERSION_1_2;
        let v13 = self.api >= vk::API_VERSION_1_3;
        let mut e10 = pick(&self.f10, E10);
        if !voll {
            e10[vk::F10_MULTI_DRAW_INDIRECT] = 0;
        }
        let e11 = pick(&self.f11, E11);
        let mut e12 = pick(&self.f12, E12);
        // with the Vulkan memory model on, Device-scope atomics (any shader's, also under the
        // GLSL450 model) need its device scope (VUID-RuntimeSpirv-vulkanMemoryModel-06265)
        if e12[vk::F12_VULKAN_MEMORY_MODEL_DEVICE_SCOPE] == 0 {
            e12[vk::F12_VULKAN_MEMORY_MODEL] = 0;
        }
        if voll {
            for &i in E12_VOLL {
                e12[i] = self.f12[i];
            }
        }
        let e13 = pick(&self.f13, E13);
        let mut f13: vk::PhysicalDeviceVulkan13Features = vk::Zero::zero();
        f13.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES;
        f13.f = e13;
        let mut f12: vk::PhysicalDeviceVulkan12Features = vk::Zero::zero();
        f12.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES;
        f12.f = e12;
        let mut f11: vk::PhysicalDeviceVulkan11Features = vk::Zero::zero();
        f11.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES;
        f11.f = e11;
        let mut dynr: vk::PhysicalDeviceOneFeature = vk::Zero::zero();
        dynr.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_DYNAMIC_RENDERING_FEATURES;
        dynr.f = 1;
        let mut tl: vk::PhysicalDeviceOneFeature = vk::Zero::zero();
        tl.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_FEATURES;
        tl.f = 1;
        let mut f2: vk::PhysicalDeviceFeatures2 = vk::Zero::zero();
        f2.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2;
        f2.f = e10;
        let mut names: Vec<*const u8> = Vec::new();
        // the chain is linked from its tail: no structure is written after a pointer to it exists
        if v12 {
            if v13 {
                f12.p_next = &mut f13 as *mut _ as *mut c_void;
            } else {
                f12.p_next = &mut dynr as *mut _ as *mut c_void;
                names.push(b"VK_KHR_dynamic_rendering\0".as_ptr());
            }
            f11.p_next = &mut f12 as *mut _ as *mut c_void;
            f2.p_next = &mut f11 as *mut _ as *mut c_void;
        } else {
            // Vulkan 1.1: the extensions and their dependencies
            tl.p_next = &mut dynr as *mut _ as *mut c_void;
            f2.p_next = &mut tl as *mut _ as *mut c_void;
            names.push(b"VK_KHR_timeline_semaphore\0".as_ptr());
            names.push(b"VK_KHR_create_renderpass2\0".as_ptr());
            names.push(b"VK_KHR_depth_stencil_resolve\0".as_ptr());
            names.push(b"VK_KHR_dynamic_rendering\0".as_ptr());
        }
        let swapchain = self.exts.swapchain;
        if swapchain {
            names.push(b"VK_KHR_swapchain\0".as_ptr());
        }
        let dmabuf = self.exts.dmabuf;
        if dmabuf {
            names.push(b"VK_KHR_external_memory_fd\0".as_ptr());
            names.push(b"VK_EXT_external_memory_dma_buf\0".as_ptr());
            names.push(b"VK_EXT_image_drm_format_modifier\0".as_ptr());
            if !v12 {
                names.push(b"VK_KHR_image_format_list\0".as_ptr());
            }
        }
        let queue_foreign = dmabuf && self.exts.queue_foreign;
        if queue_foreign {
            names.push(b"VK_EXT_queue_family_foreign\0".as_ptr());
        }
        let prio = 1.0f32;
        let qi = vk::DeviceQueueCreateInfo {
            s_type: vk::STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
            queue_family_index: self.family,
            queue_count: 1,
            p_queue_priorities: &prio,
            ..vk::Zero::zero()
        };
        // (G5d) the compute queue: a family of its own when asked for, unless jfgui_GPU_COMPUTE=0
        let off = plat::env("jfgui_GPU_COMPUTE").is_some_and(|v| v == "0" || v.eq_ignore_ascii_case("off"));
        let cfam = self.compute_family.filter(|_| opts.compute_queue && !off);
        let qis = [qi, vk::DeviceQueueCreateInfo { queue_family_index: cfam.map_or(0, |c| c.0), ..qi }];
        let info = vk::DeviceCreateInfo {
            s_type: vk::STRUCTURE_TYPE_DEVICE_CREATE_INFO,
            p_next: &f2 as *const _ as *const c_void,
            queue_create_info_count: 1 + cfam.is_some() as u32,
            p_queue_create_infos: qis.as_ptr(),
            enabled_extension_count: names.len() as u32,
            pp_enabled_extension_names: names.as_ptr(),
            ..vk::Zero::zero()
        };
        let mut dev: vk::Device = 0;
        // SAFETY: the chain and the arrays live until the call returns.
        let r = unsafe { (inst.f.create_device)(self.pd, &info, core::ptr::null(), &mut dev) };
        if r != vk::SUCCESS {
            return Err(err(r));
        }
        let gdpa = inst.f.get_device_proc_addr;
        // SAFETY: names are NUL-terminated; dev is the device just created.
        let f = match vk::DeviceFns::load(&|n| unsafe { gdpa(dev, n.as_ptr()) }) {
            Ok(f) => f,
            Err(name) => {
                // SAFETY: vkDestroyDevice of the fresh device (nothing created from it).
                let d = unsafe { gdpa(dev, b"vkDestroyDevice\0".as_ptr()) };
                if d != 0 {
                    // SAFETY: vkDestroyDevice's prototype.
                    unsafe {
                        core::mem::transmute::<usize, unsafe extern "system" fn(vk::Device, *const c_void)>(d)(
                            dev,
                            core::ptr::null(),
                        )
                    };
                }
                return Err(GpuError::Unsupported(name));
            }
        };
        let mut queue: vk::Queue = 0;
        // SAFETY: family / index 0 were requested above.
        unsafe { (f.get_device_queue)(dev, self.family, 0, &mut queue) };
        let ti = vk::SemaphoreTypeCreateInfo {
            s_type: vk::STRUCTURE_TYPE_SEMAPHORE_TYPE_CREATE_INFO,
            semaphore_type: vk::SEMAPHORE_TYPE_TIMELINE,
            initial_value: 0,
            ..vk::Zero::zero()
        };
        let si = vk::SemaphoreCreateInfo {
            s_type: vk::STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO,
            p_next: &ti as *const _ as *const c_void,
            flags: 0,
        };
        let mut timeline = 0;
        // SAFETY: valid create info.
        let r = unsafe { (f.create_semaphore)(dev, &si, core::ptr::null(), &mut timeline) };
        if r != vk::SUCCESS {
            // SAFETY: nothing else was created from the device.
            unsafe { (f.destroy_device)(dev, core::ptr::null()) };
            return Err(err(r));
        }
        let mut lane = None;
        if let Some((family, ts_bits)) = cfam {
            let mut cq: vk::Queue = 0;
            let mut ct = 0;
            // SAFETY: the family / index 0 requested above; a valid create info.
            let r = unsafe {
                (f.get_device_queue)(dev, family, 0, &mut cq);
                (f.create_semaphore)(dev, &si, core::ptr::null(), &mut ct)
            };
            if r != vk::SUCCESS {
                // SAFETY: only the timeline was created from the device.
                unsafe {
                    (f.destroy_semaphore)(dev, timeline, core::ptr::null());
                    (f.destroy_device)(dev, core::ptr::null());
                }
                return Err(err(r));
            }
            lane = Some(Lane { queue: cq, family, timeline: ct, ts_bits, submitted: AtomicU64::new(0), completed: AtomicU64::new(0), qlock: SpinLock::new(()) });
        }
        let mut features = self.features;
        if !voll {
            features.bindless = false;
            features.draw_indirect_count = false;
            features.multi_draw_indirect = false;
        }
        let mut limits = self.limits;
        if !voll {
            limits.max_bindless_textures = 0;
            limits.max_bindless_samplers = 0;
            limits.max_bindless_buffers = 0;
            limits.max_bindless_storage_textures = 0;
        }
        let ts_mask = match self.ts_bits {
            0 => 0,
            64.. => !0,
            b => (1u64 << b) - 1,
        };
        let sh = Arc::new(Shared {
            inst: inst.clone(),
            pd: self.pd,
            dev,
            f,
            queue,
            family: self.family,
            timeline,
            info: self.info.clone(),
            features,
            limits,
            level,
            raw: self.raw,
            ts_mask,
            swapchain,
            queue_foreign,
            ts_base: AtomicU64::new(0),
            host_query_reset: v12 && e12[vk::F12_HOST_QUERY_RESET] != 0,
            maintenance4: v13 && e13[vk::F13_MAINTENANCE4] != 0,
            lane,
            lost: AtomicBool::new(false),
            submitted: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            qlock: SpinLock::new(()),
            st: SpinLock::new(State {
                mem: Allocator::new(self.mem),
                garbage: Vec::new(),
                free: Vec::new(),
                cfree: Vec::new(),
                busy: Vec::new(),
                pools: Vec::new(),
                upload: None,
                ring: Ring::default(),
            }),
        });
        let compute = sh.lane.as_ref().map(|_| ComputeQueue { sh: sh.clone() });
        Ok(Device { queue: Queue { sh: sh.clone() }, compute, sh })
    }
}

impl Shared {
    /// Err(DeviceLost) once the device is lost
    #[inline]
    pub fn check(&self) -> Result<()> {
        if self.lost.load(Ordering::Relaxed) { Err(GpuError::DeviceLost) } else { Ok(()) }
    }

    /// a VkResult as Result (DEVICE_LOST marks the device)
    pub fn vk(&self, r: vk::VkResult) -> Result<()> {
        if r == vk::SUCCESS {
            return Ok(());
        }
        if r == vk::ERROR_DEVICE_LOST {
            self.lost.store(true, Ordering::Relaxed);
        }
        Err(err(r))
    }

    /// run f with the queue held (vkQueuePresentKHR and vkQueueSubmit need it alone)
    #[cfg_attr(jfgui_glibc, allow(dead_code))]
    pub fn with_queue<R>(&self, f: impl FnOnce(vk::Queue) -> R) -> R {
        let _q = self.qlock.lock();
        f(self.queue)
    }

    /// wait until the queue has finished everything, presents included (vkQueueWaitIdle;
    /// also on a lost device, where it returns at once)
    #[cfg_attr(jfgui_glibc, allow(dead_code))]
    pub fn queue_idle(&self) -> Result<()> {
        // the value is read while the queue is held: nothing is submitted until the wait ends
        // SAFETY: the queue is held while the driver waits on it.
        let (r, v) = self.with_queue(|q| (unsafe { (self.f.queue_wait_idle)(q) }, self.submitted()));
        self.vk(r)?;
        self.completed.fetch_max(v, Ordering::AcqRel);
        let mut st = self.st.lock();
        self.collect(&mut st);
        Ok(())
    }

    pub fn mark_lost(&self) {
        self.lost.store(true, Ordering::Relaxed);
    }

    pub fn submitted(&self) -> u64 {
        self.submitted.load(Ordering::Acquire)
    }

    /// (G5d) the compute queue's last submitted value (0 without one)
    fn lane_submitted(&self) -> u64 {
        self.lane.as_ref().map_or(0, |l| l.submitted.load(Ordering::Acquire))
    }

    /// (G5d) the compute queue's value the GPU has reached (asks the driver; 0 without one)
    fn lane_refresh(&self) -> u64 {
        let Some(l) = &self.lane else { return 0 };
        // (nothing pending: no call; a device that never uses the queue never asks its timeline)
        let done = l.completed.load(Ordering::Acquire);
        if l.submitted.load(Ordering::Acquire) <= done {
            return done;
        }
        let mut v = 0u64;
        // SAFETY: the compute queue's timeline semaphore.
        let r = unsafe { (self.f.get_semaphore_counter_value)(self.dev, l.timeline, &mut v) };
        if r != vk::SUCCESS {
            if r == vk::ERROR_DEVICE_LOST {
                self.lost.store(true, Ordering::Relaxed);
            }
            return l.completed.load(Ordering::Acquire);
        }
        l.completed.fetch_max(v, Ordering::AcqRel);
        v
    }

    /// the timeline value the GPU has reached (asks the driver)
    pub fn refresh(&self) -> u64 {
        let mut v = 0u64;
        // SAFETY: the device's timeline semaphore.
        let r = unsafe { (self.f.get_semaphore_counter_value)(self.dev, self.timeline, &mut v) };
        if r != vk::SUCCESS {
            if r == vk::ERROR_DEVICE_LOST {
                self.lost.store(true, Ordering::Relaxed);
            }
            return self.completed.load(Ordering::Acquire);
        }
        self.completed.fetch_max(v, Ordering::AcqRel);
        v
    }

    /// destroy g once the GPU has finished every submission made so far; a buffer or image also
    /// the open upload buffer, whose copies may name it (a texture written and dropped before the
    /// next submit).  Nothing else waits for that: a swapchain must go before its surface, which
    /// its drop releases right after this
    pub fn defer(&self, g: Garbage) {
        let mut st = self.st.lock();
        let named = st.upload.is_some() && matches!(g, Garbage::Buffer(..) | Garbage::Image(..));
        let v = self.submitted() + named as u64;
        // (G5d) and the compute queue's latest submission
        let vc = self.lane_submitted();
        let lane_done = self.lane.as_ref().is_none_or(|l| vc <= l.completed.load(Ordering::Acquire));
        if (v <= self.completed.load(Ordering::Acquire) && lane_done) || self.lost.load(Ordering::Relaxed) {
            self.destroy(&mut st, g);
        } else {
            st.garbage.push((v, vc, g));
        }
    }

    fn destroy(&self, st: &mut State, g: Garbage) {
        let (f, d, n) = (&self.f, self.dev, core::ptr::null());
        // SAFETY: the GPU no longer uses the object (its timeline value is reached, or no
        // submission is pending, or the device is lost); each handle is destroyed once.
        unsafe {
            match g {
                Garbage::Buffer(h, a) => {
                    (f.destroy_buffer)(d, h, n);
                    st.mem.free(f, d, &a);
                }
                Garbage::Image(h, a) => {
                    (f.destroy_image)(d, h, n);
                    if let Some(a) = a {
                        st.mem.free(f, d, &a);
                    }
                }
                Garbage::View(h) => (f.destroy_image_view)(d, h, n),
                Garbage::Sampler(h) => (f.destroy_sampler)(d, h, n),
                Garbage::Shader(h) => (f.destroy_shader_module)(d, h, n),
                Garbage::SetLayout(h) => (f.destroy_descriptor_set_layout)(d, h, n),
                Garbage::PipeLayout(h) => (f.destroy_pipeline_layout)(d, h, n),
                Garbage::Pipeline(h) => (f.destroy_pipeline)(d, h, n),
                Garbage::Set(pool, set) => {
                    (f.free_descriptor_sets)(d, pool, 1, &set);
                }
                Garbage::Pool(h) => (f.destroy_descriptor_pool)(d, h, n),
                Garbage::Query(h) => (f.destroy_query_pool)(d, h, n),
                Garbage::Semaphore(h) => (f.destroy_semaphore)(d, h, n),
                Garbage::Swapchain(h) => {
                    if let Some(ds) = f.destroy_swapchain_khr {
                        ds(d, h, n);
                    }
                }
                Garbage::Memory(h) => (f.free_memory)(d, h, n),
            }
        }
    }

    /// free the garbage and the command slots whose submissions completed
    fn collect(&self, st: &mut State) {
        if st.garbage.is_empty() && st.busy.is_empty() && st.ring.busy.is_empty() && st.ring.free.is_empty() {
            return;
        }
        let lost = self.lost.load(Ordering::Relaxed);
        let done = if lost { u64::MAX } else { self.refresh() };
        let done_c = if lost { u64::MAX } else { self.lane_refresh() };
        let mut i = 0;
        while i < st.ring.busy.len() {
            if st.ring.busy[i].0 <= done {
                let (_, mut c) = st.ring.busy.swap_remove(i);
                c.used = 0;
                st.ring.free.push(c);
            } else {
                i += 1;
            }
        }
        self.ring_trim(st);
        let mut i = 0;
        while i < st.busy.len() {
            let v = st.busy[i].0;
            let finished = if v & COMPUTE_ID != 0 { v & !COMPUTE_ID <= done_c } else { v <= done };
            if finished {
                let (_, s) = st.busy.swap_remove(i);
                // SAFETY: the pool's command buffer finished executing.
                unsafe { (self.f.reset_command_pool)(self.dev, s.pool, 0) };
                if s.compute { st.cfree.push(s) } else { st.free.push(s) }
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        while i < st.garbage.len() {
            if st.garbage[i].0 <= done && st.garbage[i].1 <= done_c {
                let (_, _, g) = st.garbage.swap_remove(i);
                self.destroy(st, g);
            } else {
                i += 1;
            }
        }
    }

    /// a command buffer slot (reused or new), begun for one submission
    #[cfg_attr(not(jfgui_glibc), allow(dead_code))] // (vk_dmabuf.rs)
    pub fn take_slot(&self) -> Result<Slot> {
        self.take_slot_for(false)
    }

    /// (G5d) a slot of the compute queue's family (compute) or the device queue's
    pub fn take_slot_for(&self, compute: bool) -> Result<Slot> {
        self.check()?;
        if compute && self.lane.is_none() {
            return Err(GpuError::Unsupported("a compute queue"));
        }
        let slot = {
            let mut st = self.st.lock();
            let empty = if compute { st.cfree.is_empty() } else { st.free.is_empty() };
            if empty {
                self.collect(&mut st);
            }
            if compute { st.cfree.pop() } else { st.free.pop() }
        };
        let slot = match slot {
            Some(s) => s,
            None => self.new_slot(compute)?,
        };
        let bi = vk::CommandBufferBeginInfo {
            s_type: vk::STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            flags: vk::COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            ..vk::Zero::zero()
        };
        // SAFETY: the slot's buffer is in the initial state (new or its pool reset).
        let r = unsafe { (self.f.begin_command_buffer)(slot.cb, &bi) };
        if let Err(e) = self.vk(r) {
            self.give_slot(slot);
            return Err(e);
        }
        Ok(slot)
    }

    fn new_slot(&self, compute: bool) -> Result<Slot> {
        let family = match (&self.lane, compute) {
            (Some(l), true) => l.family,
            _ => self.family,
        };
        let pi = vk::CommandPoolCreateInfo {
            s_type: vk::STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
            flags: vk::COMMAND_POOL_CREATE_TRANSIENT_BIT,
            queue_family_index: family,
            ..vk::Zero::zero()
        };
        let mut pool = 0;
        // SAFETY: valid create info.
        self.vk(unsafe { (self.f.create_command_pool)(self.dev, &pi, core::ptr::null(), &mut pool) })?;
        let ai = vk::CommandBufferAllocateInfo {
            s_type: vk::STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
            command_pool: pool,
            level: vk::COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: 1,
            ..vk::Zero::zero()
        };
        let mut cb = 0;
        // SAFETY: one buffer from the new pool.
        let r = unsafe { (self.f.allocate_command_buffers)(self.dev, &ai, &mut cb) };
        if let Err(e) = self.vk(r) {
            // SAFETY: the pool is unused.
            unsafe { (self.f.destroy_command_pool)(self.dev, pool, core::ptr::null()) };
            return Err(e);
        }
        Ok(Slot { pool, cb, compute })
    }

    /// a slot that was never submitted: reset, back to the free list
    pub fn give_slot(&self, s: Slot) {
        // SAFETY: the buffer is not pending (never submitted).
        unsafe { (self.f.reset_command_pool)(self.dev, s.pool, 0) };
        let mut st = self.st.lock();
        if s.compute { st.cfree.push(s) } else { st.free.push(s) }
    }

    /// (G5d) the families a resource is shared by (concurrently), when there is a compute queue
    pub fn sharing(&self) -> Option<[u32; 2]> {
        self.lane.as_ref().map(|l| [self.family, l.family])
    }

    /// the free chunks beyond what the ring keeps go back: ordinary ones above CHUNKS_FREE,
    /// large ones a second after the last large upload (all but the largest at once; the largest
    /// too when the last large upload was far smaller: one big image's buffer does not stay
    /// around for the frames after it)
    fn ring_trim(&self, st: &mut State) {
        let now = plat::time_us();
        let keep_big = now.saturating_sub(st.ring.big_at) < BIG_KEEP_US;
        let largest = st.ring.free.iter().map(|c| c.a.size).max().unwrap_or(0);
        let mut small = 0;
        let mut i = 0;
        while i < st.ring.free.len() {
            let size = st.ring.free[i].a.size;
            let keep = if size >= CHUNK * 2 {
                keep_big && size == largest && size <= st.ring.big_size.saturating_mul(BIG_KEEP_RATIO).max(CHUNK * 2)
            } else {
                small += 1;
                small <= CHUNKS_FREE
            };
            if keep {
                i += 1;
            } else {
                let c = st.ring.free.swap_remove(i);
                self.destroy(st, Garbage::Buffer(c.buf, c.a));
            }
        }
    }

    /// a chunk of at least size bytes, reused or new (its `used` is 0); the ring's own chunks
    /// (size = CHUNK) are never large ones, those would stay around
    fn ring_chunk(&self, st: &mut State, size: u64) -> Result<Chunk> {
        let top = if size == CHUNK { CHUNK * 2 } else { u64::MAX };
        let best = st.ring.free.iter().enumerate().filter(|(_, c)| c.a.size >= size && c.a.size < top).min_by_key(|(_, c)| c.a.size).map(|(i, _)| i);
        if let Some(i) = best {
            return Ok(st.ring.free.swap_remove(i));
        }
        let (buf, a) = self.raw_buffer(st, size, vk::BUFFER_USAGE_TRANSFER_SRC_BIT, MemoryKind::Upload)?;
        Ok(Chunk { buf, a, used: 0 })
    }

    /// the upload command buffer (begun when there is none): it runs first in the next submit
    pub fn upload_cb(&self, st: &mut State) -> Result<vk::CommandBuffer> {
        self.stage(st, 0)?;
        Ok(st.upload.as_ref().map_or(0, |u| u.slot.cb))
    }

    /// staging space of n bytes for the upload command buffer -> (buffer, offset, address)
    /// (n = 0: only the command buffer)
    fn stage(&self, st: &mut State, n: u64) -> Result<(vk::Handle, u64, usize)> {
        if st.upload.is_none() {
            // the upload buffer's slot: from the free list or new (the lock is held: no collect)
            let slot = match st.free.pop() {
                Some(s) => s,
                None => self.new_slot(false)?,
            };
            let bi = vk::CommandBufferBeginInfo {
                s_type: vk::STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                flags: vk::COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
                ..vk::Zero::zero()
            };
            // SAFETY: a slot in the initial state.
            let r = unsafe { (self.f.begin_command_buffer)(slot.cb, &bi) };
            if let Err(e) = self.vk(r) {
                st.free.push(slot);
                return Err(e);
            }
            st.upload = Some(Upload { slot, retired: Vec::new() });
        }
        if n == 0 {
            return Ok((0, 0, 0));
        }
        if n > CHUNK / 2 {
            // a buffer of its own: free once the next submission is done
            st.ring.big_at = plat::time_us();
            st.ring.big_size = n;
            let size = (n + 0xFFFF) & !0xFFFF;
            let mut c = self.ring_chunk(st, size.max(CHUNK * 2))?;
            c.used = n;
            let (buf, ptr) = (c.buf, c.a.ptr);
            st.upload.as_mut().unwrap_or_else(|| unreachable!()).retired.push(c);
            return Ok((buf, 0, ptr));
        }
        let room = st.ring.cur.as_ref().is_some_and(|c| c.used + n <= c.a.size);
        if !room {
            if let Some(c) = st.ring.cur.take() {
                if st.ring.cur_open {
                    st.upload.as_mut().unwrap_or_else(|| unreachable!()).retired.push(c);
                } else {
                    let v = st.ring.cur_value;
                    st.ring.busy.push((v, c));
                }
            }
            let c = self.ring_chunk(st, CHUNK)?;
            st.ring.cur = Some(c);
            st.ring.cur_open = false;
        }
        st.ring.cur_open = true;
        let c = st.ring.cur.as_mut().unwrap_or_else(|| unreachable!());
        let off = c.used;
        c.used = (off + n + 15) & !15;
        Ok((c.buf, off, c.a.ptr + off as usize))
    }

    /// a VkBuffer with memory of kind (bound) -> (buffer, allocation)
    pub fn raw_buffer(&self, st: &mut State, size: u64, usage: u32, kind: MemoryKind) -> Result<(vk::Handle, Alloc)> {
        // (G5d) every buffer is shared with the compute queue's family, if there is one
        let fams = self.sharing();
        let bi = vk::BufferCreateInfo {
            s_type: vk::STRUCTURE_TYPE_BUFFER_CREATE_INFO,
            size,
            usage,
            sharing_mode: if fams.is_some() { vk::SHARING_MODE_CONCURRENT } else { vk::SHARING_MODE_EXCLUSIVE },
            queue_family_index_count: if fams.is_some() { 2 } else { 0 },
            p_queue_family_indices: fams.as_ref().map_or(core::ptr::null(), |f| f.as_ptr()),
            ..vk::Zero::zero()
        };
        let mut buf = 0;
        // SAFETY: valid create info.
        self.vk(unsafe { (self.f.create_buffer)(self.dev, &bi, core::ptr::null(), &mut buf) })?;
        let mut req: vk::MemoryRequirements = vk::Zero::zero();
        // SAFETY: the buffer just created.
        unsafe { (self.f.get_buffer_memory_requirements)(self.dev, buf, &mut req) };
        let a = match st.mem.alloc(&self.f, self.dev, &req, kind, true) {
            Ok(a) => a,
            Err(e) => {
                // SAFETY: unbound, unused buffer.
                unsafe { (self.f.destroy_buffer)(self.dev, buf, core::ptr::null()) };
                return Err(e);
            }
        };
        // SAFETY: the allocation satisfies the buffer's requirements.
        let r = unsafe { (self.f.bind_buffer_memory)(self.dev, buf, a.mem, a.offset) };
        if let Err(e) = self.vk(r) {
            // SAFETY: as above.
            unsafe { (self.f.destroy_buffer)(self.dev, buf, core::ptr::null()) };
            st.mem.free(&self.f, self.dev, &a);
            return Err(e);
        }
        Ok((buf, a))
    }

    /// submit command buffers (their slots), after the pending uploads; wait: a binary
    /// semaphore and its stages, signal: a binary semaphore -> the timeline value
    pub fn submit(&self, cbs: &[vk::CommandBuffer], slots: Vec<Slot>, wait: Option<(vk::Handle, u32)>,
                  signal: Option<vk::Handle>) -> Result<u64> {
        self.submit_on(false, cbs, slots, wait, &[], signal)
    }

    /// (G5d) the semaphore and value a SubmitId waits on; Invalid when it was not submitted yet
    fn wait_of(&self, id: u64) -> Result<(vk::Handle, u64)> {
        if id & COMPUTE_ID == 0 {
            return if id <= self.submitted() { Ok((self.timeline, id)) } else { Err(GpuError::Invalid("submit_after: a submission not made yet")) };
        }
        match &self.lane {
            Some(l) if id & !COMPUTE_ID <= l.submitted.load(Ordering::Acquire) => Ok((l.timeline, id & !COMPUTE_ID)),
            Some(_) => Err(GpuError::Invalid("submit_after: a submission not made yet")),
            None => Err(GpuError::Invalid("submit_after: an id of a compute queue the device does not have")),
        }
    }

    /// submit on the device's queue (compute false: after the pending uploads) or on the compute queue (the pending
    /// uploads go first on the device's queue, and it waits for them); wait: a binary semaphore and its stages,
    /// after: SubmitIds of either queue it waits for (at all commands), signal: a binary semaphore -> the SubmitId
    /// (COMPUTE_ID on the compute queue)
    pub fn submit_on(&self, compute: bool, cbs: &[vk::CommandBuffer], slots: Vec<Slot>, wait: Option<(vk::Handle, u32)>, after: &[u64],
                     signal: Option<vk::Handle>) -> Result<u64> {
        let mut slots = slots;
        let fine = self.check().and_then(|_| {
            if slots.iter().any(|s| s.compute != compute) {
                return Err(GpuError::Invalid("submit: a command buffer of the other queue's family (Device::encoder / ComputeQueue::encoder)"));
            }
            if compute && (self.lane.is_none() || wait.is_some() || signal.is_some()) {
                return Err(GpuError::Invalid("submit: no compute queue, or a semaphore on it"));
            }
            Ok(())
        });
        let mut waits: Vec<(vk::Handle, u64)> = Vec::with_capacity(after.len() + 1);
        let fine = fine.and_then(|_| {
            for &id in after {
                waits.push(self.wait_of(id)?);
            }
            Ok(())
        });
        if let Err(e) = fine {
            for s in slots {
                self.give_slot(s);
            }
            return Err(e);
        }
        if compute {
            return self.submit_lane(cbs, slots, waits);
        }
        let up = self.st.lock().upload.take();
        let mut all: Vec<vk::CommandBuffer> = Vec::with_capacity(cbs.len() + 1);
        let mut chunks: Vec<Chunk> = Vec::new();
        let uploads = up.is_some();
        if let Some(u) = up {
            let mb = vk::MemoryBarrier {
                s_type: vk::STRUCTURE_TYPE_MEMORY_BARRIER,
                src_access_mask: vk::ACCESS_TRANSFER_WRITE_BIT,
                dst_access_mask: vk::ACCESS_MEMORY_READ_BIT | vk::ACCESS_MEMORY_WRITE_BIT,
                ..vk::Zero::zero()
            };
            // SAFETY: the upload buffer is recording; its copies are done before what follows.
            unsafe {
                (self.f.cmd_pipeline_barrier)(u.slot.cb, vk::PIPELINE_STAGE_TRANSFER_BIT,
                    vk::PIPELINE_STAGE_ALL_COMMANDS_BIT, 0, 1, &mb, 0, core::ptr::null(), 0, core::ptr::null());
            }
            // SAFETY: as above.
            let r = unsafe { (self.f.end_command_buffer)(u.slot.cb) };
            all.push(u.slot.cb);
            slots.push(u.slot);
            chunks = u.retired;
            if let Err(e) = self.vk(r) {
                self.fail_submit(slots, chunks);
                return Err(e);
            }
        }
        all.extend_from_slice(cbs);
        // the binary semaphore (its value is ignored) and the timelines it waits for
        let mut ws: Vec<vk::Handle> = Vec::with_capacity(waits.len() + 1);
        let mut wv: Vec<u64> = Vec::with_capacity(waits.len() + 1);
        let mut wst: Vec<u32> = Vec::with_capacity(waits.len() + 1);
        if let Some((s, st)) = wait {
            ws.push(s);
            wv.push(0);
            wst.push(st);
        }
        for &(s, v) in &waits {
            ws.push(s);
            wv.push(v);
            wst.push(vk::PIPELINE_STAGE_ALL_COMMANDS_BIT);
        }
        let q = self.qlock.lock();
        let value = self.submitted.load(Ordering::Relaxed) + 1;
        let sems = [self.timeline, signal.unwrap_or(0)];
        let values = [value, 0];
        let nsig = 1 + signal.is_some() as u32;
        let tsi = vk::TimelineSemaphoreSubmitInfo {
            s_type: vk::STRUCTURE_TYPE_TIMELINE_SEMAPHORE_SUBMIT_INFO,
            wait_semaphore_value_count: wv.len() as u32,
            p_wait_semaphore_values: wv.as_ptr(),
            signal_semaphore_value_count: nsig,
            p_signal_semaphore_values: values.as_ptr(),
            ..vk::Zero::zero()
        };
        let si = vk::SubmitInfo {
            s_type: vk::STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: &tsi as *const _ as *const c_void,
            wait_semaphore_count: ws.len() as u32,
            p_wait_semaphores: ws.as_ptr(),
            p_wait_dst_stage_mask: wst.as_ptr(),
            command_buffer_count: all.len() as u32,
            p_command_buffers: all.as_ptr(),
            signal_semaphore_count: nsig,
            p_signal_semaphores: sems.as_ptr(),
        };
        // SAFETY: every command buffer is executable; the queue is held (qlock).
        let r = unsafe { (self.f.queue_submit)(self.queue, 1, &si, 0) };
        if r == vk::SUCCESS {
            self.submitted.store(value, Ordering::Release);
        }
        drop(q);
        if let Err(e) = self.vk(r) {
            self.fail_submit(slots, chunks);
            return Err(e);
        }
        let mut st = self.st.lock();
        for s in slots {
            st.busy.push((value, s));
        }
        for c in chunks {
            st.ring.busy.push((value, c));
        }
        if uploads && st.ring.cur_open {
            st.ring.cur_value = value;
            st.ring.cur_open = false;
        }
        self.collect(&mut st);
        Ok(value)
    }

    /// (G5d) a submission on the compute queue (checked by submit_on): the pending uploads first on the device's
    /// queue, waited for
    fn submit_lane(&self, cbs: &[vk::CommandBuffer], slots: Vec<Slot>, mut waits: Vec<(vk::Handle, u64)>) -> Result<u64> {
        let Some(l) = &self.lane else { return Err(GpuError::Invalid("no compute queue")) };
        if self.st.lock().upload.is_some() {
            match self.submit_on(false, &[], Vec::new(), None, &[], None) {
                Ok(v) => waits.push((self.timeline, v)),
                Err(e) => {
                    for s in slots {
                        self.give_slot(s);
                    }
                    return Err(e);
                }
            }
        }
        let wv: Vec<u64> = waits.iter().map(|w| w.1).collect();
        let ws: Vec<vk::Handle> = waits.iter().map(|w| w.0).collect();
        let wst: Vec<u32> = ws.iter().map(|_| vk::PIPELINE_STAGE_ALL_COMMANDS_BIT).collect();
        let q = l.qlock.lock();
        let value = l.submitted.load(Ordering::Relaxed) + 1;
        let tsi = vk::TimelineSemaphoreSubmitInfo {
            s_type: vk::STRUCTURE_TYPE_TIMELINE_SEMAPHORE_SUBMIT_INFO,
            wait_semaphore_value_count: wv.len() as u32,
            p_wait_semaphore_values: wv.as_ptr(),
            signal_semaphore_value_count: 1,
            p_signal_semaphore_values: &value,
            ..vk::Zero::zero()
        };
        let si = vk::SubmitInfo {
            s_type: vk::STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: &tsi as *const _ as *const c_void,
            wait_semaphore_count: ws.len() as u32,
            p_wait_semaphores: ws.as_ptr(),
            p_wait_dst_stage_mask: wst.as_ptr(),
            command_buffer_count: cbs.len() as u32,
            p_command_buffers: cbs.as_ptr(),
            signal_semaphore_count: 1,
            p_signal_semaphores: &l.timeline,
        };
        // SAFETY: every command buffer is executable and of the compute family; the queue is held.
        let r = unsafe { (self.f.queue_submit)(l.queue, 1, &si, 0) };
        if r == vk::SUCCESS {
            l.submitted.store(value, Ordering::Release);
        }
        drop(q);
        if let Err(e) = self.vk(r) {
            for s in slots {
                self.give_slot(s);
            }
            return Err(e);
        }
        let mut st = self.st.lock();
        for s in slots {
            st.busy.push((value | COMPUTE_ID, s));
        }
        self.collect(&mut st);
        Ok(value | COMPUTE_ID)
    }

    /// (G5d) wait for a SubmitId of either queue -> false on timeout
    pub fn wait_id(&self, id: u64, timeout_ns: u64) -> Result<bool> {
        if id & COMPUTE_ID == 0 {
            return self.wait_value(id, timeout_ns);
        }
        self.check()?;
        let Some(l) = &self.lane else { return Err(GpuError::Invalid("wait: an id of a compute queue the device does not have")) };
        let v = id & !COMPUTE_ID;
        if v <= l.completed.load(Ordering::Acquire) {
            return Ok(true);
        }
        if v > l.submitted.load(Ordering::Acquire) {
            return Err(GpuError::Invalid("wait: not submitted yet"));
        }
        let wi = vk::SemaphoreWaitInfo {
            s_type: vk::STRUCTURE_TYPE_SEMAPHORE_WAIT_INFO,
            semaphore_count: 1,
            p_semaphores: &l.timeline,
            p_values: &v,
            ..vk::Zero::zero()
        };
        // SAFETY: the compute queue's timeline semaphore and a value already submitted.
        let r = unsafe { (self.f.wait_semaphores)(self.dev, &wi, timeout_ns) };
        if r == vk::TIMEOUT {
            return Ok(false);
        }
        self.vk(r)?;
        l.completed.fetch_max(v, Ordering::AcqRel);
        let mut st = self.st.lock();
        self.collect(&mut st);
        Ok(true)
    }

    fn fail_submit(&self, slots: Vec<Slot>, chunks: Vec<Chunk>) {
        for s in slots {
            self.give_slot(s);
        }
        let mut st = self.st.lock();
        st.ring.cur_open = false; // its copies never ran
        for c in chunks {
            self.destroy(&mut st, Garbage::Buffer(c.buf, c.a));
        }
    }

    /// wait until the timeline reaches v -> false on timeout
    pub fn wait_value(&self, v: u64, timeout_ns: u64) -> Result<bool> {
        self.check()?;
        if v <= self.completed.load(Ordering::Acquire) {
            return Ok(true);
        }
        if v > self.submitted() {
            return Err(GpuError::Invalid("wait: not submitted yet"));
        }
        let wi = vk::SemaphoreWaitInfo {
            s_type: vk::STRUCTURE_TYPE_SEMAPHORE_WAIT_INFO,
            semaphore_count: 1,
            p_semaphores: &self.timeline,
            p_values: &v,
            ..vk::Zero::zero()
        };
        // SAFETY: the device's timeline semaphore and a value already submitted (or later).
        let r = unsafe { (self.f.wait_semaphores)(self.dev, &wi, timeout_ns) };
        if r == vk::TIMEOUT {
            return Ok(false);
        }
        self.vk(r)?;
        self.completed.fetch_max(v, Ordering::AcqRel);
        let mut st = self.st.lock();
        self.collect(&mut st);
        Ok(true)
    }

    /// bind-group descriptor sets: a set of layout from the pools (a new pool when all are full)
    pub fn alloc_set(&self, layout: vk::Handle, needs: &[vk::DescriptorPoolSize]) -> Result<(vk::Handle, vk::Handle)> {
        let mut st = self.st.lock();
        let alloc = |pool: vk::Handle| {
            let ai = vk::DescriptorSetAllocateInfo {
                s_type: vk::STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
                descriptor_pool: pool,
                descriptor_set_count: 1,
                p_set_layouts: &layout,
                ..vk::Zero::zero()
            };
            let mut set = 0;
            // SAFETY: the pool belongs to this device; the state lock serialises pool use.
            let r = unsafe { (self.f.allocate_descriptor_sets)(self.dev, &ai, &mut set) };
            (r, set)
        };
        for &pool in st.pools.iter().rev() {
            let (r, set) = alloc(pool);
            if r == vk::SUCCESS {
                return Ok((pool, set));
            }
            if r != vk::ERROR_OUT_OF_POOL_MEMORY && r != vk::ERROR_FRAGMENTED_POOL {
                self.vk(r)?;
            }
        }
        // a new pool, large enough for 64 groups of this layout as well
        let mut sizes: Vec<vk::DescriptorPoolSize> = [
            (vk::DESCRIPTOR_TYPE_UNIFORM_BUFFER, 256),
            (vk::DESCRIPTOR_TYPE_STORAGE_BUFFER, 512),
            (vk::DESCRIPTOR_TYPE_SAMPLED_IMAGE, 512),
            (vk::DESCRIPTOR_TYPE_STORAGE_IMAGE, 128),
            (vk::DESCRIPTOR_TYPE_SAMPLER, 256),
        ]
        .iter()
        .map(|&(t, n)| vk::DescriptorPoolSize { type_: t, descriptor_count: n })
        .collect();
        for s in sizes.iter_mut() {
            let need: u32 = needs.iter().filter(|x| x.type_ == s.type_).map(|x| x.descriptor_count).sum();
            s.descriptor_count = s.descriptor_count.max(need.saturating_mul(64));
        }
        let pi = vk::DescriptorPoolCreateInfo {
            s_type: vk::STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
            flags: vk::DESCRIPTOR_POOL_CREATE_FREE_DESCRIPTOR_SET_BIT,
            max_sets: 256,
            pool_size_count: sizes.len() as u32,
            p_pool_sizes: sizes.as_ptr(),
            ..vk::Zero::zero()
        };
        let mut pool = 0;
        // SAFETY: valid create info.
        self.vk(unsafe { (self.f.create_descriptor_pool)(self.dev, &pi, core::ptr::null(), &mut pool) })?;
        st.pools.push(pool);
        let (r, set) = alloc(pool);
        self.vk(r)?;
        Ok((pool, set))
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        let (f, d, n) = (&self.f, self.dev, core::ptr::null());
        // SAFETY: the last handle of the device is gone: wait for the GPU, then destroy every
        // object the device still owns, the device last.
        unsafe {
            (f.device_wait_idle)(d);
            let mut st = self.st.lock();
            if let Some(u) = st.upload.take() {
                for c in u.retired {
                    st.garbage.push((0, 0, Garbage::Buffer(c.buf, c.a)));
                }
                st.free.push(u.slot);
            }
            let ring = core::mem::take(&mut st.ring);
            for c in ring.cur.into_iter().chain(ring.free).chain(ring.busy.into_iter().map(|(_, c)| c)) {
                st.garbage.push((0, 0, Garbage::Buffer(c.buf, c.a)));
            }
            let garbage = core::mem::take(&mut st.garbage);
            for (_, _, g) in garbage {
                self.destroy(&mut st, g);
            }
            for (_, s) in core::mem::take(&mut st.busy) {
                st.free.push(s);
            }
            let cfree = core::mem::take(&mut st.cfree);
            for s in st.free.drain(..).chain(cfree) {
                (f.destroy_command_pool)(d, s.pool, n);
            }
            for p in st.pools.drain(..) {
                (f.destroy_descriptor_pool)(d, p, n);
            }
            st.mem.destroy_all(f, d);
            (f.destroy_semaphore)(d, self.timeline, n);
            if let Some(l) = &self.lane {
                (f.destroy_semaphore)(d, l.timeline, n);
            }
            drop(st);
            (f.destroy_device)(d, n);
        }
    }
}

impl Device {
    pub fn queue(&self) -> &Queue {
        &self.queue
    }
    /// (G5d) the queue of its own for compute and copies (a family without graphics), when DeviceOptions asked for it;
    /// None where the device has none, without asking and with jfgui_GPU_COMPUTE=0 (then everything goes through
    /// queue())
    pub fn compute_queue(&self) -> Option<&ComputeQueue> {
        self.compute.as_ref()
    }
    pub fn info(&self) -> &AdapterInfo {
        &self.sh.info
    }
    /// what this device has enabled
    pub fn features(&self) -> &Features {
        &self.sh.features
    }
    pub fn limits(&self) -> &Limits {
        &self.sh.limits
    }
    pub fn level(&self) -> Level {
        self.sh.level
    }
    /// the device was lost: every call fails with DeviceLost; create a new device
    pub fn is_lost(&self) -> bool {
        self.sh.check().is_err()
    }
    /// (test aid) behave as a lost device from now on (a lost device runs nothing any more and
    /// its resources may go at once; here the GPU may still work on the last submission: it
    /// finishes first)
    pub fn test_lose(&self) {
        let _ = self.wait_idle();
        self.sh.mark_lost();
    }
    /// the validation layer watches this device
    pub fn validating(&self) -> bool {
        self.sh.inst.validate
    }
    /// free what the GPU has finished with (also done by submit and wait)
    pub fn poll(&self) -> Result<()> {
        self.sh.check()?;
        let mut st = self.sh.st.lock();
        self.sh.collect(&mut st);
        drop(st);
        self.sh.check()
    }
    /// wait for a submission (of either queue) -> false when timeout_ns passed first
    pub fn wait(&self, id: SubmitId, timeout_ns: u64) -> Result<bool> {
        self.sh.wait_id(id.0, timeout_ns)
    }
    /// wait for every submission so far (of both queues)
    pub fn wait_idle(&self) -> Result<()> {
        let v = self.sh.submitted();
        let vc = self.sh.lane_submitted();
        if self.sh.wait_value(v, 10_000_000_000)? && (vc == 0 || self.sh.wait_id(vc | COMPUTE_ID, 10_000_000_000)?) { Ok(()) } else { Err(GpuError::Timeout) }
    }
    /// the last submission the GPU has finished
    pub fn completed(&self) -> SubmitId {
        SubmitId(self.sh.refresh())
    }
    /// the submission id (of either queue) is done on the GPU (asks the driver; never waits)
    pub fn is_done(&self, id: SubmitId) -> bool {
        if id.0 & COMPUTE_ID != 0 {
            return self.sh.lane_refresh() >= id.0 & !COMPUTE_ID;
        }
        self.sh.refresh() >= id.0
    }
    /// bytes of device and host memory the device holds in blocks
    pub fn memory_reserved(&self) -> u64 {
        self.sh.st.lock().mem.reserved
    }
}

impl Queue {
    /// run command buffers in order (after the pending uploads) -> their timeline value
    pub fn submit<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(&self, cmds: I) -> Result<SubmitId> {
        let mut cbs = Vec::new();
        let mut slots = Vec::new();
        for c in cmds {
            let s = c.into_slot();
            cbs.push(s.cb);
            slots.push(s);
        }
        self.sh.submit(&cbs, slots, None, None).map(SubmitId)
    }

    /// (G5d) run command buffers in order once the submissions `wait` (of either queue; Device::compute_queue) are
    /// done: a wait through the timeline, at all commands
    pub fn submit_after<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(&self, wait: &[SubmitId], cmds: I) -> Result<SubmitId> {
        let (cbs, slots) = split_cmds(cmds);
        let after: Vec<u64> = wait.iter().map(|w| w.0).collect();
        self.sh.submit_on(false, &cbs, slots, None, &after, None).map(SubmitId)
    }

    /// submit the pending uploads now
    pub fn flush(&self) -> Result<SubmitId> {
        self.sh.submit(&[], Vec::new(), None, None).map(SubmitId)
    }

    /// write data into a buffer at offset before the next submission (staged)
    pub fn write_buffer<T: Pod>(&self, buf: &crate::Buffer, offset: u64, data: &[T]) -> Result<()> {
        let bytes = crate::bytes_of(data);
        let n = bytes.len() as u64;
        if offset.checked_add(n).is_none_or(|e| e > buf.size()) {
            return Err(GpuError::Invalid("write_buffer: range outside the buffer"));
        }
        if n == 0 {
            return Ok(());
        }
        let sh = &self.sh;
        sh.check()?;
        let mut st = sh.st.lock();
        let (sbuf, soff, ptr) = sh.stage(&mut st, n)?;
        let cb = st.upload.as_ref().map_or(0, |u| u.slot.cb);
        // SAFETY: ptr addresses n mapped bytes of the staging chunk; the copy is recorded
        // into the upload buffer (recording, guarded by the lock).
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
            let region = vk::BufferCopy { src_offset: soff, dst_offset: offset, size: n };
            (sh.f.cmd_copy_buffer)(cb, sbuf, buf.handle(), 1, &region);
        }
        Ok(())
    }

    /// write texels into a texture region before the next submission (staged): the region
    /// goes from `before` to CopyDst, gets the data, then goes to `after`
    pub fn write_texture(&self, dst: &TextureRegion, data: &[u8], layout: DataLayout, before: Layout,
                         after: Layout) -> Result<()> {
        let t = dst.texture;
        let copy = crate::vk_res::copy_region(dst, &layout, data.len() as u64)?;
        let need = copy.1;
        let sh = &self.sh;
        sh.check()?;
        let mut st = sh.st.lock();
        let (sbuf, soff, ptr) = sh.stage(&mut st, need)?;
        let cb = st.upload.as_ref().map_or(0, |u| u.slot.cb);
        let mut region = copy.0;
        region.buffer_offset = soff;
        let range = crate::vk_res::subresources(t, dst.mip, 1, dst.base_layer, dst.layers);
        // SAFETY: as in write_buffer; data holds at least `need` bytes after layout.offset
        // (copy_region checked it).
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr().add(layout.offset as usize), ptr as *mut u8, need as usize);
            crate::vk_cmd::image_barrier(&sh.f, cb, t.image(), range, before, Layout::CopyDst);
            (sh.f.cmd_copy_buffer_to_image)(cb, sbuf, t.image(), vk::IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &region);
            crate::vk_cmd::image_barrier(&sh.f, cb, t.image(), range, Layout::CopyDst, after);
        }
        Ok(())
    }

    /// write rectangles (x, y, w, h; clipped to the texture, empty ones skipped) of mip 0, layer 0
    /// of a 2D texture from `data`, an image of the texture's size in rows of `stride` bytes (a
    /// framebuffer), before the next submission; the staged bytes are those of the rectangles.
    /// The whole level goes from `before` to CopyDst and then to `after`, once for all of them
    pub fn write_texture_rects(&self, t: &crate::Texture, rects: &[[i32; 4]], data: &[u8], stride: u32, before: Layout,
                               after: Layout) -> Result<()> {
        let f = t.format();
        let bb = f.block_bytes() as u64;
        if f.is_compressed() || f.is_depth() || t.depth() != 1 {
            return Err(GpuError::Invalid("write_texture_rects: an uncompressed 2D colour texture"));
        }
        let (tw, th) = (t.width() as i32, t.height() as i32);
        if (stride as u64) < tw as u64 * bb || (data.len() as u64) < stride as u64 * th as u64 {
            return Err(GpuError::Invalid("write_texture_rects: data smaller than the texture"));
        }
        self.write_texture_rects_at(t, rects, (0, 0), data, stride, before, after)
    }

    /// write_texture_rects from an image of another size: rectangles (x, y, w, h) of `data`, an
    /// image in rows of `stride` bytes, go to (x - origin.0, y - origin.1) of the texture; what
    /// lies outside the image or the texture is skipped
    #[allow(clippy::too_many_arguments)]
    pub fn write_texture_rects_at(&self, t: &crate::Texture, rects: &[[i32; 4]], origin: (i32, i32), data: &[u8], stride: u32, before: Layout,
                                  after: Layout) -> Result<()> {
        let f = t.format();
        let bb = f.block_bytes() as u64;
        if f.is_compressed() || f.is_depth() || t.depth() != 1 {
            return Err(GpuError::Invalid("write_texture_rects: an uncompressed 2D colour texture"));
        }
        let (tw, th) = (t.width() as i32, t.height() as i32);
        if stride == 0 {
            return Err(GpuError::Invalid("write_texture_rects: stride 0"));
        }
        // the image's size: rows that lie completely in data, as many pixels as a row holds
        let (iw, ih) = ((stride as u64 / bb).min(i32::MAX as u64) as i64, (data.len() as u64 / stride as u64).min(i32::MAX as u64) as i64);
        let mut regions: Vec<vk::BufferImageCopy> = Vec::with_capacity(rects.len());
        let mut total = 0u64;
        let sub = crate::vk_res::subresources(t, 0, 1, 0, 1);
        let (ox, oy) = (origin.0 as i64, origin.1 as i64);
        for r in rects {
            // in the texture's coordinates, cut to the texture and the image
            let x0 = (r[0] as i64 - ox).max(0).max(-ox);
            let y0 = (r[1] as i64 - oy).max(0).max(-oy);
            let x1 = (r[0] as i64 + r[2] as i64 - ox).min(tw as i64).min(iw - ox);
            let y1 = (r[1] as i64 + r[3] as i64 - oy).min(th as i64).min(ih - oy);
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let (x0, y0, x1, y1) = (x0 as i32, y0 as i32, x1 as i32, y1 as i32);
            let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
            regions.push(vk::BufferImageCopy {
                buffer_offset: total,
                buffer_row_length: w,
                buffer_image_height: h,
                image_subresource: vk::ImageSubresourceLayers { aspect_mask: sub.aspect_mask, mip_level: 0, base_array_layer: 0, layer_count: 1 },
                image_offset: vk::Offset3D { x: x0, y: y0, z: 0 },
                image_extent: vk::Extent3D { width: w, height: h, depth: 1 },
            });
            total += w as u64 * h as u64 * bb;
        }
        if regions.is_empty() {
            return Ok(());
        }
        let sh = &self.sh;
        sh.check()?;
        let mut st = sh.st.lock();
        let (sbuf, soff, ptr) = sh.stage(&mut st, total)?;
        let cb = st.upload.as_ref().map_or(0, |u| u.slot.cb);
        for g in regions.iter_mut() {
            let (x, y) = ((g.image_offset.x as i64 + ox) as usize, (g.image_offset.y as i64 + oy) as usize);
            let (w, h) = (g.image_extent.width as usize, g.image_extent.height as usize);
            let row = w * bb as usize;
            let mut dst = ptr + g.buffer_offset as usize;
            for yy in y..y + h {
                let from = yy * stride as usize + x * bb as usize;
                // SAFETY: the row lies in data (cut to the image above) and in the staged range
                // of `total` bytes, whose rectangles follow each other.
                unsafe { core::ptr::copy_nonoverlapping(data.as_ptr().add(from), dst as *mut u8, row) };
                dst += row;
            }
            g.buffer_offset += soff;
        }
        // SAFETY: the upload buffer is recording (guarded by the lock); the regions lie in the
        // texture (clipped above) and the staged range.
        unsafe {
            crate::vk_cmd::image_barrier(&sh.f, cb, t.image(), sub, before, Layout::CopyDst);
            (sh.f.cmd_copy_buffer_to_image)(cb, sbuf, t.image(), vk::IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, regions.len() as u32, regions.as_ptr());
            crate::vk_cmd::image_barrier(&sh.f, cb, t.image(), sub, Layout::CopyDst, after);
        }
        Ok(())
    }

    /// read a buffer's contents at offset (copies through a readback buffer when needed,
    /// submits and waits for the GPU: the slow path, for tools and tests)
    pub fn read_buffer<T: Pod>(&self, buf: &crate::Buffer, offset: u64, out: &mut [T]) -> Result<()> {
        let n = core::mem::size_of_val(out) as u64;
        if offset.checked_add(n).is_none_or(|e| e > buf.size()) {
            return Err(GpuError::Invalid("read_buffer: range outside the buffer"));
        }
        if buf.memory() == MemoryKind::Readback {
            let id = self.flush()?;
            self.sh.wait_value(id.0, u64::MAX)?;
            return buf.read(offset, out);
        }
        let dev = Device { sh: self.sh.clone(), queue: self.clone(), compute: None };
        let rb = dev.create_buffer(&crate::BufferDesc { size: n.max(4), usage: crate::BufferUsage::NONE, memory: MemoryKind::Readback })?;
        let mut enc = dev.encoder()?;
        enc.copy_buffer(buf, offset, &rb, 0, n);
        let id = self.submit([enc.finish()?])?;
        if !self.sh.wait_value(id.0, 10_000_000_000)? {
            return Err(GpuError::Timeout);
        }
        rb.read(0, out)
    }
}

/// (G5d) the command buffers' handles and slots
fn split_cmds<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(cmds: I) -> (Vec<vk::CommandBuffer>, Vec<Slot>) {
    let mut cbs = Vec::new();
    let mut slots = Vec::new();
    for c in cmds {
        let s = c.into_slot();
        cbs.push(s.cb);
        slots.push(s);
    }
    (cbs, slots)
}

impl ComputeQueue {
    /// (G5d) an encoder of this queue's family: copies, fills, barriers, compute and timestamps (where the family has
    /// them); a render pass is Invalid (from finish)
    pub fn encoder(&self) -> Result<crate::CommandEncoder<'_>> {
        crate::vk_cmd::CommandEncoder::of(&self.sh, true)
    }

    /// (G5d) run this queue's command buffers in order -> their SubmitId (Device::wait takes it); the pending uploads go
    /// first, on the device's queue
    pub fn submit<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(&self, cmds: I) -> Result<SubmitId> {
        self.submit_after(&[], cmds)
    }

    /// (G5d) the same once the submissions `wait` (of either queue) are done: a wait through the timeline, at all
    /// commands
    pub fn submit_after<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(&self, wait: &[SubmitId], cmds: I) -> Result<SubmitId> {
        let (cbs, slots) = split_cmds(cmds);
        let after: Vec<u64> = wait.iter().map(|w| w.0).collect();
        self.sh.submit_on(true, &cbs, slots, None, &after, None).map(SubmitId)
    }
}
