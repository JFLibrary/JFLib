// mtl_dev -- Metal on macOS and iOS (AP6, docs/APPLE.md): instance, adapters, device and queue
//
//   Instance::new() -> Result<Instance>     Err(Disabled) with jfgui_GPU=0, Err(NoAdapter) without
//        a Metal device; jfgui_GPU_VALIDATE=1 turns Metal's API validation on (MTL_DEBUG_LAYER)
//   instance.adapters()                     macOS: MTLCopyAllDevices, iOS: the system's device
//   adapter.create_device(&DeviceDesc)      a command queue and a timeline (MTLSharedEvent); level
//        Basis (Voll: AP6g)
//   device.queue() -> &Queue                submit([cmd, ..]) -> SubmitId (the event's value),
//        write_buffer / write_texture / write_texture_rects(_at) (staged in shared buffers, copied by
//        a blit encoder before the next submission), read_buffer (submits and waits)
//   device.wait(id, timeout) / is_done / completed / wait_idle   on the event
//
// Lifetimes: command buffers retain what they use (retained references), so a dropped resource
// lives as long as a submission needs it -- no deferred freeing here.  Synchronization: resources
// with Metal's hazard tracking; barriers end the current encoder (mtl_cmd.rs).
// A command buffer that ends in an error (MTLCommandBufferStatusError) loses the device: every
// later call gives DeviceLost, as on Vulkan after a driver reset.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rt::sync::SpinLock;

use crate::mtl_objc::{self as objc, Id, MTLOrigin, MTLSize, Obj, Pool, msg};
use crate::*;

unsafe extern "C" {
    fn setenv(name: *const u8, value: *const u8, overwrite: i32) -> i32;
}

pub(crate) fn env_on(name: &str) -> bool {
    plat::env(name).is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("on") || v.eq_ignore_ascii_case("true"))
}

/// Metal's GPU families (MTLGPUFamily)
pub(crate) mod family {
    pub const APPLE3: isize = 1003;
    pub const APPLE4: isize = 1004;
    pub const APPLE6: isize = 1006;
    pub const APPLE7: isize = 1007;
    pub const APPLE8: isize = 1008;
    pub const APPLE9: isize = 1009;
    pub const MAC2: isize = 2002;
    pub const METAL3: isize = 5001;
}

// ---------------------------------------------------------------- instance

pub(crate) struct InstShared {
    pub validate: bool,
}

/// the system's GPU API (Metal)
#[derive(Clone)]
pub struct Instance {
    pub(crate) sh: Arc<InstShared>,
}

/// every Metal device of the system (+1 each)
fn all_devices() -> Vec<Obj> {
    let _pool = Pool::new();
    let mut out = Vec::new();
    #[cfg(target_os = "macos")]
    {
        // SAFETY: MTLCopyAllDevices returns an NSArray (+1) of devices; count / objectAtIndex: read it.
        unsafe {
            let arr = objc::MTLCopyAllDevices();
            if !arr.is_null() {
                let n: usize = msg!(arr, c"count"; usize);
                for i in 0..n {
                    let d: Id = msg!(arr, c"objectAtIndex:", i => usize; Id);
                    if let Some(o) = Obj::retained(d) {
                        out.push(o);
                    }
                }
                msg!(arr, c"release"; ());
            }
        }
    }
    if out.is_empty() {
        // SAFETY: the Create rule: the device comes retained.
        if let Some(d) = Obj::new(unsafe { objc::MTLCreateSystemDefaultDevice() }) {
            out.push(d);
        }
    }
    out
}

impl Instance {
    pub fn new() -> Result<Instance> {
        Instance::with(&InstanceDesc::DEFAULT)
    }

    pub fn with(desc: &InstanceDesc) -> Result<Instance> {
        if plat::env("jfgui_GPU").is_some_and(|v| v == "0" || v.eq_ignore_ascii_case("off")) {
            return Err(GpuError::Disabled);
        }
        let validate = desc.validate || env_on("jfgui_GPU_VALIDATE");
        if validate && plat::env("MTL_DEBUG_LAYER").is_none() {
            // Metal reads it when the first device is made (before that in this process)
            // SAFETY: NUL-terminated strings; setenv copies them.
            unsafe { setenv(b"MTL_DEBUG_LAYER\0".as_ptr(), b"1\0".as_ptr(), 1) };
        }
        if all_devices().is_empty() {
            return Err(GpuError::NoAdapter);
        }
        Ok(Instance { sh: Arc::new(InstShared { validate }) })
    }

    /// Metal's API validation is on (jfgui_GPU_VALIDATE=1: MTL_DEBUG_LAYER; it stops the process
    /// at a misuse instead of counting messages)
    pub fn validating(&self) -> bool {
        self.sh.validate
    }

    /// every GPU of the system, discrete ones first
    pub fn adapters(&self) -> Result<Vec<Adapter>> {
        let mut list: Vec<Adapter> = all_devices().into_iter().map(|d| probe(&self.sh, d)).collect();
        list.sort_by_key(|a| match a.info.kind {
            AdapterKind::Discrete => 0,
            AdapterKind::Integrated => 1,
            _ => 2,
        });
        Ok(list)
    }

    /// (the browser: adapter and device come later; here they are there)
    pub fn prepare(&self) -> Result<bool> {
        Ok(true)
    }

    /// the adapter to use: jfgui_GPU_ADAPTER (index or name part), else the first usable one
    pub fn adapter(&self) -> Result<Adapter> {
        let list = self.adapters()?;
        if let Some(sel) = plat::env("jfgui_GPU_ADAPTER").filter(|s| !s.is_empty()) {
            let pick = match sel.parse::<usize>() {
                Ok(i) => list.into_iter().nth(i),
                Err(_) => {
                    let want = sel.to_ascii_lowercase();
                    list.into_iter().find(|a| a.info.name.to_ascii_lowercase().contains(&want))
                }
            };
            let a = pick.ok_or(GpuError::NoAdapter)?;
            return match a.info.missing {
                Some(why) => Err(GpuError::Unsupported(why)),
                None => Ok(a),
            };
        }
        list.into_iter().find(|a| a.info.missing.is_none()).ok_or(GpuError::NoAdapter)
    }

    /// the adapters for showing windows, frugal ones first (integrated before discrete)
    pub fn adapters_low_power(&self, _display: Option<(u32, u32)>) -> Result<Vec<Adapter>> {
        if plat::env("jfgui_GPU_ADAPTER").is_some_and(|s| !s.is_empty()) {
            return self.adapter().map(|a| alloc::vec![a]);
        }
        let mut list: Vec<(u32, Adapter)> = self
            .adapters()?
            .into_iter()
            .filter(|a| a.info.missing.is_none())
            .filter_map(|a| low_power_rank(a.info.kind, false).map(|r| (r, a)))
            .collect();
        if list.is_empty() {
            return Err(GpuError::NoAdapter);
        }
        list.sort_by_key(|x| x.0);
        Ok(list.into_iter().map(|x| x.1).collect())
    }
}

/// the order of adapters for showing windows, lower first (None: not for this)
pub fn low_power_rank(kind: AdapterKind, drives_display: bool) -> Option<u32> {
    let k = match kind {
        AdapterKind::Integrated => 0,
        AdapterKind::Discrete => 1,
        _ => return None,
    };
    Some(if drives_display { k } else { 2 + k })
}

/// (errors, warnings) of the validation: Metal's validation stops the process instead
pub fn validation_messages() -> (u32, u32) {
    (0, 0)
}

// ---------------------------------------------------------------- adapters

/// a GPU
#[derive(Clone)]
pub struct Adapter {
    pub(crate) inst: Arc<InstShared>,
    pub(crate) dev: Obj,
    pub(crate) info: AdapterInfo,
    pub(crate) features: Features,
    pub(crate) limits: Limits,
}

fn supports(dev: Id, f: isize) -> bool {
    // SAFETY: supportsFamily: (macOS 10.15, iOS 13) on a live device.
    unsafe { msg!(dev, c"supportsFamily:", f => isize; bool) }
}

fn probe(inst: &Arc<InstShared>, dev: Obj) -> Adapter {
    let d = dev.id();
    let _pool = Pool::new();
    // SAFETY: property getters of a live MTLDevice (name: autoreleased, read inside the pool).
    let (name, low_power, removable, unified, max_buffer, wg, tg_mem, working, bc, d24) = unsafe {
        let name = objc::string_of(msg!(d, c"name"; Id));
        let low_power: bool = if cfg!(target_os = "macos") { msg!(d, c"isLowPower"; bool) } else { true };
        let removable: bool = if cfg!(target_os = "macos") { msg!(d, c"isRemovable"; bool) } else { false };
        let unified: bool = msg!(d, c"hasUnifiedMemory"; bool);
        let max_buffer: usize = msg!(d, c"maxBufferLength"; usize);
        let wg: MTLSize = msg!(d, c"maxThreadsPerThreadgroup"; MTLSize);
        let tg_mem: usize = msg!(d, c"maxThreadgroupMemoryLength"; usize);
        let working: u64 = if cfg!(target_os = "macos") { msg!(d, c"recommendedMaxWorkingSetSize"; u64) } else { 0 };
        let bc: bool = if msg!(d, c"respondsToSelector:", objc::sel(c"supportsBCTextureCompression") => objc::Sel; bool) {
            msg!(d, c"supportsBCTextureCompression"; bool)
        } else {
            false
        };
        let d24: bool = if cfg!(target_os = "macos") { msg!(d, c"isDepth24Stencil8PixelFormatSupported"; bool) } else { false };
        (name, low_power, removable, unified, max_buffer, wg, tg_mem, working, bc, d24)
    };
    let _ = d24;
    let apple = supports(d, family::APPLE3);
    let mac2 = supports(d, family::MAC2);
    let metal3 = supports(d, family::METAL3);
    let kind = if unified || low_power {
        AdapterKind::Integrated
    } else if removable {
        AdapterKind::Discrete
    } else {
        AdapterKind::Discrete
    };
    let family_name = [
        (family::APPLE9, "Apple9"),
        (family::APPLE8, "Apple8"),
        (family::APPLE7, "Apple7"),
        (family::APPLE6, "Apple6"),
        (family::APPLE4, "Apple4"),
        (family::APPLE3, "Apple3"),
        (family::MAC2, "Mac2"),
    ]
    .iter()
    .find(|(f, _)| supports(d, *f))
    .map_or("", |x| x.1);
    // (the simulator's device answers to none of the families; what it cannot do shows in the tests)
    let missing = if !(apple || mac2 || cfg!(target_abi = "sim")) { Some("a Metal GPU of family Apple3 / Mac2 or later") } else { None };
    let info = AdapterInfo {
        name,
        vendor_id: 0x106b,
        device_id: 0,
        vendor: "Apple",
        kind,
        driver: String::from("Metal"),
        driver_info: String::from(family_name),
        driver_version: String::new(),
        api_version: if metal3 { (3, 0, 0) } else { (2, 4, 0) },
        backend: "Metal",
        missing,
    };
    let apple7 = supports(d, family::APPLE7);
    let features = Features {
        timestamps: false,
        multi_draw_indirect: false,
        draw_indirect_first_instance: true,
        draw_indirect_count: false,
        bindless: false,
        subgroups: false,
        subgroups_webgpu: apple || mac2,
        subgroup_stages: subgroup_stage::COMPUTE | subgroup_stage::FRAGMENT,
        subgroup_ops: subgroup_op::BASIC | subgroup_op::VOTE | subgroup_op::ARITHMETIC | subgroup_op::BALLOT | subgroup_op::SHUFFLE
            | subgroup_op::SHUFFLE_RELATIVE | subgroup_op::QUAD,
        subgroup_size_control: false,
        subgroup_extended_types: true,
        draw_parameters: false,
        shader_f16: true,
        shader_f64: false,
        shader_i16: true,
        shader_i64: true,
        shader_i8: true,
        storage_16bit: true,
        uniform_16bit: true,
        push_constant_16bit: true,
        uniform_standard_layout: true,
        demote: false,
        clip_distances: true,
        primitive_index: apple7,
        sample_shading: true,
        storage_8bit: true,
        texture_bc: bc,
        anisotropy: true,
        depth_clamp: true,
        vertex_stores: true,
        fragment_stores: true,
        storage_write_without_format: false,
        buffer_address: false,
        swapchain: true,
        mesh_shader: false,
        ray_query: false,
        ray_pipeline: false,
    };
    let big = max_buffer.min(u32::MAX as usize) as u32;
    let limits = Limits {
        max_texture_2d: 16384,
        max_texture_3d: 2048,
        max_texture_layers: 2048,
        max_storage_binding: big,
        max_uniform_binding: big.min(1 << 26),
        max_push_constants: PUSH_CONSTANTS_MAX,
        max_bind_groups: BIND_GROUPS_MAX,
        max_compute_workgroup_size: [wg.w as u32, wg.h as u32, wg.d as u32],
        max_compute_invocations: 1024,
        max_compute_workgroups: [65535, 65535, 65535],
        max_compute_shared_memory: tg_mem as u32,
        min_uniform_offset_alignment: 256,
        min_storage_offset_alignment: 256,
        max_color_targets: 8,
        max_vertex_buffers: shader::msl::VERTEX_BUFFERS_MAX,
        max_vertex_attributes: 31,
        max_draw_indirect_count: 1,
        max_anisotropy: 16.0,
        timestamp_period_ns: 1.0,
        subgroup_size: 32,
        subgroup_min_size: 32,
        subgroup_max_size: 32,
        max_bindless_textures: 0,
        max_bindless_samplers: 0,
        max_bindless_buffers: 0,
        max_bindless_storage_textures: 0,
        device_memory: working,
    };
    Adapter { inst: inst.clone(), dev, info, features, limits }
}

impl Adapter {
    pub fn info(&self) -> &AdapterInfo {
        &self.info
    }
    pub fn features(&self) -> &Features {
        &self.features
    }
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
    /// the highest level the adapter supports (Metal: Basis; Voll is AP6g)
    pub fn level(&self) -> Level {
        Level::Basis
    }
    /// every Metal device can show in a CAMetalLayer
    pub fn can_present(&self, _surface: &crate::Surface) -> bool {
        self.info.missing.is_none()
    }

    pub fn create_device(&self, desc: &DeviceDesc) -> Result<Device> {
        self.create_device_with(desc, &DeviceOptions::DEFAULT)
    }

    /// (G5d) DeviceOptions::compute_queue: Metal has one kind of queue; Device::compute_queue is None
    pub fn create_device_with(&self, _desc: &DeviceDesc, _opts: &DeviceOptions) -> Result<Device> {
        if let Some(why) = self.info.missing {
            return Err(GpuError::Unsupported(why));
        }
        let d = self.dev.id();
        // SAFETY: newCommandQueue / newSharedEvent return new objects (+1) of a live device.
        let (queue, event) = unsafe { (msg!(d, c"newCommandQueue"; Id), msg!(d, c"newSharedEvent"; Id)) };
        let queue = Obj::new(queue).ok_or(GpuError::OutOfMemory)?;
        let event = Obj::new(event).ok_or(GpuError::OutOfMemory)?;
        let sh = Arc::new(Shared {
            validate: self.inst.validate,
            dev: self.dev.clone(),
            queue,
            event,
            info: self.info.clone(),
            features: self.features,
            limits: self.limits,
            lost: AtomicBool::new(false),
            submitted: AtomicU64::new(0),
            reserved: AtomicU64::new(0),
            st: SpinLock::new(State { upload: None, inflight: Vec::new(), ring: Ring::default() }),
            fill: SpinLock::new(None),
        });
        Ok(Device { queue: Queue { sh: sh.clone() }, sh })
    }
}

// ---------------------------------------------------------------- device

struct Upload {
    cb: Obj,
    /// the open blit encoder (None once ended)
    blit: Option<Obj>,
}

impl Upload {
    fn end(&mut self) {
        if let Some(b) = self.blit.take() {
            // SAFETY: the open upload encoder, ended once.
            unsafe { msg!(b.id(), c"endEncoding"; ()) };
        }
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        // pending uploads never submitted (the device goes): Metal wants the encoder ended
        self.end();
    }
}

/// a chunk of the staging ring (shared memory)
struct Chunk {
    buf: Obj,
    ptr: usize,
    size: u64,
    used: u64,
    /// the last submission that reads it (OPEN: the uploads not submitted yet)
    last: u64,
}

const OPEN: u64 = u64::MAX;
/// a chunk of the staging ring
const CHUNK: u64 = 256 << 10;
/// free chunks kept beyond the one being filled
const CHUNKS_FREE: usize = 2;
/// an upload above half a chunk has a buffer of its own, kept this long after its last use (us)
const BIG_KEEP_US: u64 = 1_000_000;

/// uploads go through chunks filled front to back across submissions; a chunk is free again once
/// the GPU finished the last submission that reads it (as Vulkan's ring, vk_dev.rs)
#[derive(Default)]
struct Ring {
    cur: Option<Chunk>,
    busy: Vec<Chunk>,
    free: Vec<Chunk>,
    /// large uploads' buffers and when they were last used
    big: Vec<(Chunk, u64)>,
}

pub(crate) struct State {
    upload: Option<Upload>,
    /// submitted command buffers not yet seen complete: (timeline value, buffer)
    inflight: Vec<(u64, Obj)>,
    ring: Ring,
}

pub(crate) struct Shared {
    pub validate: bool,
    pub dev: Obj,
    pub queue: Obj,
    pub event: Obj,
    pub info: AdapterInfo,
    pub features: Features,
    pub limits: Limits,
    lost: AtomicBool,
    submitted: AtomicU64,
    /// bytes of the buffers and textures that live
    pub reserved: AtomicU64,
    st: SpinLock<State>,
    /// the pipeline of fill_buffer (made at the first use)
    pub fill: SpinLock<Option<Obj>>,
}

impl Shared {
    pub fn check(&self) -> Result<()> {
        if self.lost.load(Ordering::Acquire) { Err(GpuError::DeviceLost) } else { Ok(()) }
    }

    pub fn mark_lost(&self) {
        self.lost.store(true, Ordering::Release);
    }

    /// a new command buffer of the queue (+1)
    pub fn command_buffer(&self) -> Result<Obj> {
        self.check()?;
        let _pool = Pool::new();
        // SAFETY: commandBuffer returns an autoreleased buffer, retained here inside the pool.
        Obj::retained(unsafe { msg!(self.queue.id(), c"commandBuffer"; Id) }).ok_or(GpuError::OutOfMemory)
    }

    /// the blit encoder of the pending uploads (made on demand)
    fn upload_blit(&self, st: &mut State) -> Result<Id> {
        if st.upload.is_none() {
            let cb = self.command_buffer()?;
            let _pool = Pool::new();
            // SAFETY: a fresh command buffer; the encoder is autoreleased, retained inside the pool.
            let blit = Obj::retained(unsafe { msg!(cb.id(), c"blitCommandEncoder"; Id) }).ok_or(GpuError::OutOfMemory)?;
            st.upload = Some(Upload { cb, blit: Some(blit) });
        }
        Ok(st.upload.as_ref().and_then(|u| u.blit.as_ref()).map_or(objc::NIL, |b| b.id()))
    }

    fn new_chunk(&self, size: u64) -> Result<Chunk> {
        // SAFETY: newBufferWithLength:options: returns a new shared buffer (+1); contents maps it.
        let (buf, ptr) = unsafe {
            let b = Obj::new(msg!(self.dev.id(), c"newBufferWithLength:options:", size as usize => usize,
                                  objc::STORAGE_SHARED | objc::CPU_CACHE_WRITE_COMBINED => usize; Id))
                .ok_or(GpuError::OutOfMemory)?;
            let p: *mut u8 = msg!(b.id(), c"contents"; *mut u8);
            (b, p as usize)
        };
        self.reserved.fetch_add(size, Ordering::Relaxed);
        Ok(Chunk { buf, ptr, size, used: 0, last: 0 })
    }

    fn drop_chunk(&self, c: Chunk) {
        self.reserved.fetch_sub(c.size, Ordering::Relaxed);
    }

    /// copy bytes into the staging ring for the open upload -> (buffer, offset)
    fn stage(&self, st: &mut State, bytes: &[u8]) -> Result<(Id, u64)> {
        let n = (bytes.len() as u64).max(4);
        let now = plat::time_us();
        let done = self.completed();
        let r = &mut st.ring;
        let (buf, off, ptr) = if n > CHUNK / 2 {
            let i = match r.big.iter().position(|(c, _)| c.size >= n && c.last <= done) {
                Some(i) => i,
                None => {
                    let c = self.new_chunk(n.div_ceil(1 << 20) << 20)?;
                    r.big.push((c, now));
                    r.big.len() - 1
                }
            };
            let (c, t) = &mut r.big[i];
            c.last = OPEN;
            *t = now;
            (c.buf.id(), 0, c.ptr)
        } else {
            let room = r.cur.as_ref().is_some_and(|c| c.used.div_ceil(16) * 16 + n <= c.size);
            if !room {
                if let Some(c) = r.cur.take() {
                    r.busy.push(c);
                }
                let mut k = 0;
                while k < r.busy.len() {
                    if r.busy[k].last <= done {
                        let c = r.busy.remove(k);
                        r.free.push(c);
                    } else {
                        k += 1;
                    }
                }
                let mut c = match r.free.pop() {
                    Some(c) => c,
                    None => self.new_chunk(CHUNK)?,
                };
                c.used = 0;
                r.cur = Some(c);
            }
            let Some(c) = r.cur.as_mut() else { return Err(GpuError::OutOfMemory) };
            let off = c.used.div_ceil(16) * 16;
            c.used = off + n;
            c.last = OPEN;
            (c.buf.id(), off, c.ptr)
        };
        // SAFETY: the n bytes from off lie in the chunk (checked above); the GPU reads them only
        // after this submission, and the chunk is not handed out again before that finished.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), (ptr + off as usize) as *mut u8, bytes.len()) };
        Ok((buf, off))
    }

    /// the chunks the open upload used are read by submission v
    fn mark_ring(st: &mut State, v: u64) {
        let r = &mut st.ring;
        for c in r.cur.iter_mut().chain(r.busy.iter_mut()).chain(r.big.iter_mut().map(|(c, _)| c)) {
            if c.last == OPEN {
                c.last = v;
            }
        }
    }

    /// free chunks the GPU is done with; large buffers a second after their last use
    fn collect_ring(&self, st: &mut State) {
        let done = self.completed();
        let now = plat::time_us();
        let r = &mut st.ring;
        let mut k = 0;
        while k < r.busy.len() {
            if r.busy[k].last <= done {
                let c = r.busy.remove(k);
                r.free.push(c);
            } else {
                k += 1;
            }
        }
        while r.free.len() > CHUNKS_FREE {
            if let Some(c) = r.free.pop() {
                self.drop_chunk(c);
            }
        }
        let mut k = 0;
        while k < r.big.len() {
            let (c, t) = &r.big[k];
            if c.last <= done && now.saturating_sub(*t) > BIG_KEEP_US {
                let (c, _) = r.big.remove(k);
                self.drop_chunk(c);
            } else {
                k += 1;
            }
        }
    }

    pub fn submitted(&self) -> u64 {
        self.submitted.load(Ordering::Acquire)
    }

    /// the timeline's value: the last submission the GPU finished
    pub fn completed(&self) -> u64 {
        // SAFETY: signaledValue of the live event.
        unsafe { msg!(self.event.id(), c"signaledValue"; u64) }
    }

    /// command buffers that ended: an error loses the device
    fn collect(&self, st: &mut State) {
        let done = self.completed();
        let mut failed = false;
        st.inflight.retain(|(v, cb)| {
            // SAFETY: status of a live command buffer (MTLCommandBufferStatusError = 5).
            let status: usize = unsafe { msg!(cb.id(), c"status"; usize) };
            if status == 5 {
                failed = true;
            }
            *v > done && status < 4
        });
        if failed {
            plat::write_stderr(b"gpu: a Metal command buffer ended in an error: the device is lost\n");
            self.mark_lost();
        }
        self.collect_ring(st);
    }

    /// commit command buffers (after the pending uploads); the last one signals the timeline ->
    /// its value
    pub fn submit(&self, cbs: Vec<Obj>) -> Result<u64> {
        self.submit_present(cbs, None)
    }

    /// submit, the last command buffer also presenting a drawable (mtl_swap.rs)
    pub fn submit_present(&self, cbs: Vec<Obj>, drawable: Option<Obj>) -> Result<u64> {
        self.check()?;
        let mut st = self.st.lock();
        self.collect(&mut st);
        self.check()?;
        let mut all: Vec<Obj> = Vec::with_capacity(cbs.len() + 1);
        if let Some(mut u) = st.upload.take() {
            u.end();
            all.push(u.cb.clone());
        }
        all.extend(cbs);
        if all.is_empty() {
            all.push(self.command_buffer()?);
        }
        let v = self.submitted.load(Ordering::Acquire) + 1;
        let last = all.len() - 1;
        for (i, cb) in all.iter().enumerate() {
            // SAFETY: command buffers whose encoders have all ended; the event outlives them.
            unsafe {
                if i == last {
                    if let Some(d) = &drawable {
                        msg!(cb.id(), c"presentDrawable:", d.id() => Id; ());
                    }
                    msg!(cb.id(), c"encodeSignalEvent:value:", self.event.id() => Id, v => u64; ());
                }
                msg!(cb.id(), c"commit"; ());
            }
        }
        self.submitted.store(v, Ordering::Release);
        Shared::mark_ring(&mut st, v);
        for cb in all {
            st.inflight.push((v, cb));
        }
        let extra = st.inflight.len().saturating_sub(64);
        st.inflight.drain(..extra);
        Ok(v)
    }

    /// wait until the timeline reaches v -> false on timeout
    pub fn wait_value(&self, v: u64, timeout_ns: u64) -> Result<bool> {
        self.check()?;
        if self.completed() >= v {
            return Ok(true);
        }
        if v > self.submitted() {
            return Err(GpuError::Invalid("wait for a submission that was not made"));
        }
        let ms = timeout_ns.div_ceil(1_000_000);
        // SAFETY: waitUntilSignaledValue:timeoutMS: (macOS 12, iOS 15) on the live event.
        let ok: bool = unsafe { msg!(self.event.id(), c"waitUntilSignaledValue:timeoutMS:", v => u64, ms => u64; bool) };
        let mut st = self.st.lock();
        self.collect(&mut st);
        drop(st);
        self.check()?;
        Ok(ok || self.completed() >= v)
    }
}

/// a logical GPU device: creates resources, records commands, owns the queue
#[derive(Clone)]
pub struct Device {
    pub(crate) sh: Arc<Shared>,
    queue: Queue,
}

/// the device's queue
#[derive(Clone)]
pub struct Queue {
    pub(crate) sh: Arc<Shared>,
}

/// (G5d) a queue of its own for compute: Metal has none apart (Device::compute_queue is None)
pub struct ComputeQueue {
    _private: (),
}

impl ComputeQueue {
    pub fn encoder(&self) -> Result<crate::CommandEncoder<'_>> {
        Err(GpuError::Unsupported("a compute queue of its own (Metal)"))
    }
    pub fn submit<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(&self, _cmds: I) -> Result<SubmitId> {
        Err(GpuError::Unsupported("a compute queue of its own (Metal)"))
    }
    pub fn submit_after<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(&self, _wait: &[SubmitId], _cmds: I) -> Result<SubmitId> {
        Err(GpuError::Unsupported("a compute queue of its own (Metal)"))
    }
}

impl Device {
    pub fn queue(&self) -> &Queue {
        &self.queue
    }
    /// (G5d) None: Metal has one kind of queue
    pub fn compute_queue(&self) -> Option<&ComputeQueue> {
        None
    }
    pub fn info(&self) -> &AdapterInfo {
        &self.sh.info
    }
    pub fn features(&self) -> &Features {
        &self.sh.features
    }
    pub fn limits(&self) -> &Limits {
        &self.sh.limits
    }
    pub fn level(&self) -> Level {
        Level::Basis
    }
    pub fn is_lost(&self) -> bool {
        self.sh.check().is_err()
    }
    /// (test aid) behave as a lost device from now on (the last submission finishes first)
    pub fn test_lose(&self) {
        let _ = self.wait_idle();
        self.sh.mark_lost();
    }
    pub fn validating(&self) -> bool {
        self.sh.validate
    }
    /// look at what the GPU finished (also done by submit and wait)
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
    pub fn completed(&self) -> SubmitId {
        SubmitId(self.sh.completed())
    }
    pub fn is_done(&self, id: SubmitId) -> bool {
        self.sh.completed() >= id.0
    }
    /// bytes of the buffers and textures the device holds
    pub fn memory_reserved(&self) -> u64 {
        self.sh.reserved.load(Ordering::Relaxed)
    }
}

/// the MTLCommandBuffers of finished CommandBuffers
fn take_cmds<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(cmds: I) -> Vec<Obj> {
    cmds.into_iter().map(|c| c.into_obj()).collect()
}

impl Queue {
    /// run command buffers in order (after the pending uploads) -> their timeline value
    pub fn submit<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(&self, cmds: I) -> Result<SubmitId> {
        self.sh.submit(take_cmds(cmds)).map(SubmitId)
    }

    /// (G5d) run command buffers after the submissions `wait`: one queue keeps the order anyway
    pub fn submit_after<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(&self, wait: &[SubmitId], cmds: I) -> Result<SubmitId> {
        if wait.iter().any(|w| w.0 > self.sh.submitted()) {
            return Err(GpuError::Invalid("submit_after: a submission that was not made"));
        }
        self.submit(cmds)
    }

    /// submit the pending uploads now
    pub fn flush(&self) -> Result<SubmitId> {
        self.sh.submit(Vec::new()).map(SubmitId)
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
        let (sbuf, soff) = sh.stage(&mut st, bytes)?;
        let blit = sh.upload_blit(&mut st)?;
        // SAFETY: the upload encoder is open (guarded by the lock); both ranges lie in their buffers.
        unsafe {
            msg!(blit, c"copyFromBuffer:sourceOffset:toBuffer:destinationOffset:size:", sbuf => Id, soff as usize => usize,
                 buf.0.h.id() => Id, offset as usize => usize, n as usize => usize; ());
        }
        Ok(())
    }

    /// write texels into a texture region before the next submission (staged)
    pub fn write_texture(&self, dst: &TextureRegion, data: &[u8], layout: DataLayout, _before: Layout, _after: Layout) -> Result<()> {
        let c = crate::mtl_res::copy_region(dst, &layout, data.len() as u64)?;
        let sh = &self.sh;
        sh.check()?;
        let from = layout.offset as usize;
        let mut st = sh.st.lock();
        let (sbuf, soff) = sh.stage(&mut st, &data[from..from + c.span as usize])?;
        let blit = sh.upload_blit(&mut st)?;
        crate::mtl_res::encode_buffer_to_texture(blit, sbuf, soff, &c, dst);
        Ok(())
    }

    /// write rectangles (x, y, w, h; clipped, empty ones skipped) of mip 0, layer 0 of a 2D
    /// texture from `data`, an image of the texture's size in rows of `stride` bytes
    pub fn write_texture_rects(&self, t: &crate::Texture, rects: &[[i32; 4]], data: &[u8], stride: u32, before: Layout, after: Layout) -> Result<()> {
        let f = t.format();
        let bb = f.block_bytes() as u64;
        if f.is_compressed() || f.is_depth() || t.depth() != 1 {
            return Err(GpuError::Invalid("write_texture_rects: an uncompressed 2D colour texture"));
        }
        let (tw, th) = (t.width() as u64, t.height() as u64);
        if (stride as u64) < tw * bb || (data.len() as u64) < stride as u64 * th {
            return Err(GpuError::Invalid("write_texture_rects: data smaller than the texture"));
        }
        self.write_texture_rects_at(t, rects, (0, 0), data, stride, before, after)
    }

    /// write_texture_rects from an image of another size: rectangles of `data` go to (x - origin.0,
    /// y - origin.1) of the texture; what lies outside the image or the texture is skipped
    #[allow(clippy::too_many_arguments)]
    pub fn write_texture_rects_at(&self, t: &crate::Texture, rects: &[[i32; 4]], origin: (i32, i32), data: &[u8], stride: u32, _before: Layout,
                                  _after: Layout) -> Result<()> {
        let f = t.format();
        let bb = f.block_bytes() as u64;
        if f.is_compressed() || f.is_depth() || t.depth() != 1 {
            return Err(GpuError::Invalid("write_texture_rects: an uncompressed 2D colour texture"));
        }
        if stride == 0 {
            return Err(GpuError::Invalid("write_texture_rects: stride 0"));
        }
        let (tw, th) = (t.width() as i64, t.height() as i64);
        let (iw, ih) = ((stride as u64 / bb).min(i32::MAX as u64) as i64, (data.len() as u64 / stride as u64).min(i32::MAX as u64) as i64);
        let (ox, oy) = (origin.0 as i64, origin.1 as i64);
        // (x, y, w, h in the texture, offset in the staging bytes)
        let mut regions: Vec<(i64, i64, u64, u64, u64)> = Vec::with_capacity(rects.len());
        let mut total = 0u64;
        for r in rects {
            let x0 = (r[0] as i64 - ox).max(0).max(-ox);
            let y0 = (r[1] as i64 - oy).max(0).max(-oy);
            let x1 = (r[0] as i64 + r[2] as i64 - ox).min(tw).min(iw - ox);
            let y1 = (r[1] as i64 + r[3] as i64 - oy).min(th).min(ih - oy);
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let (w, h) = ((x1 - x0) as u64, (y1 - y0) as u64);
            regions.push((x0, y0, w, h, total));
            total += w * h * bb;
        }
        if regions.is_empty() {
            return Ok(());
        }
        let sh = &self.sh;
        sh.check()?;
        let mut bytes = alloc::vec![0u8; total as usize];
        for &(x0, y0, w, h, off) in &regions {
            let row = (w * bb) as usize;
            let mut at = off as usize;
            for yy in 0..h as i64 {
                let from = ((y0 + oy + yy) as usize) * stride as usize + ((x0 + ox) as usize) * bb as usize;
                bytes[at..at + row].copy_from_slice(&data[from..from + row]);
                at += row;
            }
        }
        let mut st = sh.st.lock();
        let (sbuf, soff) = sh.stage(&mut st, &bytes)?;
        let blit = sh.upload_blit(&mut st)?;
        for &(x0, y0, w, h, off) in &regions {
            // SAFETY: the upload encoder is open; the region lies in mip 0 of the texture and in the
            // staged bytes (rows of w texels, tightly packed).
            unsafe {
                msg!(blit, c"copyFromBuffer:sourceOffset:sourceBytesPerRow:sourceBytesPerImage:sourceSize:toTexture:destinationSlice:destinationLevel:destinationOrigin:",
                     sbuf => Id, (soff + off) as usize => usize, (w * bb) as usize => usize, (w * h * bb) as usize => usize,
                     MTLSize { w: w as usize, h: h as usize, d: 1 } => MTLSize, t.t.h.id() => Id, 0usize => usize, 0usize => usize,
                     MTLOrigin { x: x0 as usize, y: y0 as usize, z: 0 } => MTLOrigin; ());
            }
        }
        Ok(())
    }

    /// read a buffer's contents at offset (through a readback buffer when needed; submits and
    /// waits: the slow path, for tools and tests)
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
