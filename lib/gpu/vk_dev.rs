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
//        draw-indirect-count.  Not enabled: robust buffer access (speed).
//   device.queue() -> &Queue        queue.submit([cmd, ..]) -> Result<SubmitId>
//   queue.write_buffer(&buf, off, &[T])            staged into an upload command buffer that
//   queue.write_texture(&region, &[u8], layout, before, after)   runs first in the next submit
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
// pending.  Command buffers come from slots (a transient command pool with one buffer each),
// reused once their submission completed.  Uploads: staging chunks of at least 4 MiB of
// Upload memory, filled front to back; their copies are recorded into one upload command
// buffer that ends with a global barrier (copy writes -> every later access) and is put
// before the caller's command buffers in the next vkQueueSubmit.
// Device lost: VK_ERROR_DEVICE_LOST from any call sets the device's lost flag; every later
// call returns GpuError::DeviceLost (objects are still destroyed normally).

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rt::sync::SpinLock;

use crate::vk_ffi as vk;
use crate::vk_inst::{Adapter, InstShared, err};
use crate::vk_mem::{Alloc, Allocator};
use crate::{AdapterInfo, DataLayout, DeviceDesc, Features, GpuError, Layout, Level, Limits, MemoryKind, Pod, Result,
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
    Semaphore(vk::Handle),
    Swapchain(vk::Handle),
}

/// a command pool with its one command buffer
pub(crate) struct Slot {
    pub pool: vk::Handle,
    pub cb: vk::CommandBuffer,
}

/// the pending staged uploads
struct Upload {
    slot: Slot,
    chunks: Vec<(vk::Handle, Alloc)>,
    used: u64, // bytes used of the last chunk
}

pub(crate) struct State {
    pub mem: Allocator,
    garbage: Vec<(u64, Garbage)>,
    free: Vec<Slot>,
    busy: Vec<(u64, Slot)>,
    pools: Vec<vk::Handle>, // descriptor pools of the bind groups (the last one is filled)
    upload: Option<Upload>,
}

const CHUNK: u64 = 4 << 20;

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
    pub swapchain: bool,
    /// the first timestamp tick read on the device: Timestamps::read_ms counts from it
    pub ts_base: AtomicU64,
    /// vkResetQueryPool from the host (hostQueryReset enabled)
    pub host_query_reset: bool,
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
    vk::F10_MULTI_DRAW_INDIRECT,
    vk::F10_DRAW_INDIRECT_FIRST_INSTANCE,
    vk::F10_DEPTH_CLAMP,
    vk::F10_FILL_MODE_NON_SOLID,
    vk::F10_SAMPLER_ANISOTROPY,
    vk::F10_TEXTURE_COMPRESSION_BC,
    vk::F10_VERTEX_PIPELINE_STORES_AND_ATOMICS,
    vk::F10_FRAGMENT_STORES_AND_ATOMICS,
    vk::F10_SHADER_STORAGE_IMAGE_WRITE_WITHOUT_FORMAT,
    vk::F10_SHADER_UNIFORM_BUFFER_ARRAY_DYNAMIC_INDEXING,
    vk::F10_SHADER_SAMPLED_IMAGE_ARRAY_DYNAMIC_INDEXING,
    vk::F10_SHADER_STORAGE_BUFFER_ARRAY_DYNAMIC_INDEXING,
    vk::F10_SHADER_STORAGE_IMAGE_ARRAY_DYNAMIC_INDEXING,
    vk::F10_SHADER_FLOAT64,
    vk::F10_SHADER_INT64,
    vk::F10_SHADER_INT16,
];
const E11: &[usize] = &[vk::F11_STORAGE_BUFFER_16BIT_ACCESS, vk::F11_SHADER_DRAW_PARAMETERS];
const E12: &[usize] = &[
    vk::F12_STORAGE_BUFFER_8BIT_ACCESS,
    vk::F12_SHADER_FLOAT16,
    vk::F12_SHADER_INT8,
    vk::F12_SCALAR_BLOCK_LAYOUT,
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
const E13: &[usize] = &[vk::F13_SYNCHRONIZATION2, vk::F13_DYNAMIC_RENDERING, vk::F13_MAINTENANCE4];

fn pick<const N: usize>(have: &[u32; N], want: &[usize]) -> [u32; N] {
    let mut out = [0u32; N];
    for &i in want {
        out[i] = have[i];
    }
    out
}

impl Adapter {
    /// a device of this adapter (level: the highest up to desc.max_level)
    pub fn create_device(&self, desc: &DeviceDesc) -> Result<Device> {
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
        let prio = 1.0f32;
        let qi = vk::DeviceQueueCreateInfo {
            s_type: vk::STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
            queue_family_index: self.family,
            queue_count: 1,
            p_queue_priorities: &prio,
            ..vk::Zero::zero()
        };
        let info = vk::DeviceCreateInfo {
            s_type: vk::STRUCTURE_TYPE_DEVICE_CREATE_INFO,
            p_next: &f2 as *const _ as *const c_void,
            queue_create_info_count: 1,
            p_queue_create_infos: &qi,
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
            ts_base: AtomicU64::new(0),
            host_query_reset: v12 && e12[vk::F12_HOST_QUERY_RESET] != 0,
            lost: AtomicBool::new(false),
            submitted: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            qlock: SpinLock::new(()),
            st: SpinLock::new(State {
                mem: Allocator::new(self.mem),
                garbage: Vec::new(),
                free: Vec::new(),
                busy: Vec::new(),
                pools: Vec::new(),
                upload: None,
            }),
        });
        Ok(Device { queue: Queue { sh: sh.clone() }, sh })
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
    pub fn with_queue<R>(&self, f: impl FnOnce(vk::Queue) -> R) -> R {
        let _q = self.qlock.lock();
        f(self.queue)
    }

    /// wait until the queue has finished everything, presents included (vkQueueWaitIdle;
    /// also on a lost device, where it returns at once)
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

    /// destroy g once the GPU has finished every submission made so far
    pub fn defer(&self, g: Garbage) {
        let mut st = self.st.lock();
        let v = self.submitted();
        if v <= self.completed.load(Ordering::Acquire) || self.lost.load(Ordering::Relaxed) {
            self.destroy(&mut st, g);
        } else {
            st.garbage.push((v, g));
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
            }
        }
    }

    /// free the garbage and the command slots whose submissions completed
    fn collect(&self, st: &mut State) {
        if st.garbage.is_empty() && st.busy.is_empty() {
            return;
        }
        let done = if self.lost.load(Ordering::Relaxed) { u64::MAX } else { self.refresh() };
        let mut i = 0;
        while i < st.busy.len() {
            if st.busy[i].0 <= done {
                let (_, s) = st.busy.swap_remove(i);
                // SAFETY: the pool's command buffer finished executing.
                unsafe { (self.f.reset_command_pool)(self.dev, s.pool, 0) };
                st.free.push(s);
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        while i < st.garbage.len() {
            if st.garbage[i].0 <= done {
                let (_, g) = st.garbage.swap_remove(i);
                self.destroy(st, g);
            } else {
                i += 1;
            }
        }
    }

    /// a command buffer slot (reused or new), begun for one submission
    pub fn take_slot(&self) -> Result<Slot> {
        self.check()?;
        let slot = {
            let mut st = self.st.lock();
            if st.free.is_empty() {
                self.collect(&mut st);
            }
            st.free.pop()
        };
        let slot = match slot {
            Some(s) => s,
            None => self.new_slot()?,
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

    fn new_slot(&self) -> Result<Slot> {
        let pi = vk::CommandPoolCreateInfo {
            s_type: vk::STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
            flags: vk::COMMAND_POOL_CREATE_TRANSIENT_BIT,
            queue_family_index: self.family,
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
        Ok(Slot { pool, cb })
    }

    /// a slot that was never submitted: reset, back to the free list
    pub fn give_slot(&self, s: Slot) {
        // SAFETY: the buffer is not pending (never submitted).
        unsafe { (self.f.reset_command_pool)(self.dev, s.pool, 0) };
        self.st.lock().free.push(s);
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
                None => self.new_slot()?,
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
            st.upload = Some(Upload { slot, chunks: Vec::new(), used: CHUNK });
        }
        if n == 0 {
            return Ok((0, 0, 0));
        }
        let room = {
            let up = st.upload.as_ref().unwrap_or_else(|| unreachable!());
            up.chunks.last().map_or(0, |(_, a)| a.size.saturating_sub(up.used))
        };
        if room < n {
            let size = n.max(CHUNK);
            let (buf, a) = self.raw_buffer(st, size, vk::BUFFER_USAGE_TRANSFER_SRC_BIT, MemoryKind::Upload)?;
            let up = st.upload.as_mut().unwrap_or_else(|| unreachable!());
            up.chunks.push((buf, a));
            up.used = 0;
        }
        let up = st.upload.as_mut().unwrap_or_else(|| unreachable!());
        let (buf, a) = *up.chunks.last().unwrap_or_else(|| unreachable!());
        let off = up.used;
        up.used = (off + n + 15) & !15;
        Ok((buf, off, a.ptr + off as usize))
    }

    /// a VkBuffer with memory of kind (bound) -> (buffer, allocation)
    pub fn raw_buffer(&self, st: &mut State, size: u64, usage: u32, kind: MemoryKind) -> Result<(vk::Handle, Alloc)> {
        let bi = vk::BufferCreateInfo {
            s_type: vk::STRUCTURE_TYPE_BUFFER_CREATE_INFO,
            size,
            usage,
            sharing_mode: vk::SHARING_MODE_EXCLUSIVE,
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
        let mut slots = slots;
        if let Err(e) = self.check() {
            for s in slots {
                self.give_slot(s);
            }
            return Err(e);
        }
        let up = self.st.lock().upload.take();
        let mut all: Vec<vk::CommandBuffer> = Vec::with_capacity(cbs.len() + 1);
        let mut chunks = Vec::new();
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
            chunks = u.chunks;
            if let Err(e) = self.vk(r) {
                self.fail_submit(slots, chunks);
                return Err(e);
            }
        }
        all.extend_from_slice(cbs);
        let q = self.qlock.lock();
        let value = self.submitted.load(Ordering::Relaxed) + 1;
        let sems = [self.timeline, signal.unwrap_or(0)];
        let values = [value, 0];
        let nsig = 1 + signal.is_some() as u32;
        let tsi = vk::TimelineSemaphoreSubmitInfo {
            s_type: vk::STRUCTURE_TYPE_TIMELINE_SEMAPHORE_SUBMIT_INFO,
            signal_semaphore_value_count: nsig,
            p_signal_semaphore_values: values.as_ptr(),
            ..vk::Zero::zero()
        };
        let (ws, wst) = wait.map_or(([0u64], [0u32]), |(s, st)| ([s], [st]));
        let si = vk::SubmitInfo {
            s_type: vk::STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: &tsi as *const _ as *const c_void,
            wait_semaphore_count: wait.is_some() as u32,
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
        for (b, a) in chunks {
            st.garbage.push((value, Garbage::Buffer(b, a)));
        }
        self.collect(&mut st);
        Ok(value)
    }

    fn fail_submit(&self, slots: Vec<Slot>, chunks: Vec<(vk::Handle, Alloc)>) {
        for s in slots {
            self.give_slot(s);
        }
        let mut st = self.st.lock();
        for (b, a) in chunks {
            self.destroy(&mut st, Garbage::Buffer(b, a));
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
                for (b, a) in u.chunks {
                    st.garbage.push((0, Garbage::Buffer(b, a)));
                }
                st.free.push(u.slot);
            }
            let garbage = core::mem::take(&mut st.garbage);
            for (_, g) in garbage {
                self.destroy(&mut st, g);
            }
            for (_, s) in core::mem::take(&mut st.busy) {
                st.free.push(s);
            }
            for s in st.free.drain(..) {
                (f.destroy_command_pool)(d, s.pool, n);
            }
            for p in st.pools.drain(..) {
                (f.destroy_descriptor_pool)(d, p, n);
            }
            st.mem.destroy_all(f, d);
            (f.destroy_semaphore)(d, self.timeline, n);
            drop(st);
            (f.destroy_device)(d, n);
        }
    }
}

impl Device {
    pub fn queue(&self) -> &Queue {
        &self.queue
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
    /// (test aid) behave as a lost device from now on
    pub fn test_lose(&self) {
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
    /// wait for a submission -> false when timeout_ns passed first
    pub fn wait(&self, id: SubmitId, timeout_ns: u64) -> Result<bool> {
        self.sh.wait_value(id.0, timeout_ns)
    }
    /// wait for every submission so far
    pub fn wait_idle(&self) -> Result<()> {
        let v = self.sh.submitted();
        if self.sh.wait_value(v, 10_000_000_000)? { Ok(()) } else { Err(GpuError::Timeout) }
    }
    /// the last submission the GPU has finished
    pub fn completed(&self) -> SubmitId {
        SubmitId(self.sh.refresh())
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
        let dev = Device { sh: self.sh.clone(), queue: self.clone() };
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
