// web_dev -- WebGPU in the browser: instance, adapter, device and queue, through web/gpu.js
//
//   Instance::new() -> Result<Instance>     Err(Disabled) with jfgui_GPU=0, Err(NoLoader) without
//        navigator.gpu; starts requestAdapter and requestDevice, which answer later
//   instance.prepare() -> Result<bool>      true: adapter and device are there (never waits; the
//        worker has to return to the browser's event loop for it); Err(NoAdapter), Err(Failed)
//   instance.adapter() / adapters() / adapters_low_power(display)   Err(NotReady) before that; one
//        adapter (the browser chooses, "low-power" asked); a fallback adapter (software) is Cpu and
//        not among adapters_low_power
//   adapter.create_device(&DeviceDesc)      the device that was prepared, level Basis; once per
//        instance: a new device takes a new Instance (a WebGPU adapter gives one)
//   device.queue() -> &Queue                queue.submit([cmd, ..]) -> Result<SubmitId>
//   queue.write_buffer, write_texture, write_texture_rects, write_texture_rects_at   the browser's
//        queue.writeBuffer / writeTexture: copied at the call, before the next submission, from
//        the WASM memory itself (no staging ring here)
//   device.is_done(id), completed(), poll()  never wait
//
// What the API promises and the browser cannot do, said as errors and not faked: the worker cannot
// wait for the GPU (the answer needs its event loop), so wait(id, _) answers at once (false: not done
// yet), wait_idle and queue.read_buffer are Unsupported, Buffer::read says NotReady until the
// submission has finished.  SPIR-V is Unsupported (ShaderModule::from_wgsl).  Errors the browser finds
// (validation, memory) come later than the call that caused them, as its uncaptured errors, and are
// returned by the next submit, try_acquire or poll.  Bindless, indirect count, multi-draw, timestamps
// and subgroup sizes are not built here (the level Basis; Features says so).
//
// WebGPU's objects belong to this thread: the types of this backend hold Rc, so the compiler keeps
// them from the compute workers of the threads bundle (docs/GPU.md "Ausgabe im Browser").

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use crate::*;

#[link(wasm_import_module = "jfgui")]
unsafe extern "C" {
    #[link_name = "gpu_call"]
    fn host_call(op: u32, args: *const u32, n: usize) -> u32;
}

// the calls of web/gpu.js
pub(crate) const CALL_PREPARE: u32 = 0;
pub(crate) const CALL_INFO: u32 = 1;
pub(crate) const CALL_DEVICE: u32 = 2;
pub(crate) const CALL_FREE: u32 = 3;
pub(crate) const CALL_CREATE: u32 = 4;
pub(crate) const CALL_SHADER: u32 = 5;
pub(crate) const CALL_WRITE_BUFFER: u32 = 6;
pub(crate) const CALL_READ_BUFFER: u32 = 7;
pub(crate) const CALL_WRITE_TEXTURE: u32 = 8;
pub(crate) const CALL_WRITE_RECTS: u32 = 9;
pub(crate) const CALL_CONTEXT: u32 = 10;
pub(crate) const CALL_CURRENT: u32 = 11;
pub(crate) const CALL_RESIZE: u32 = 12;
pub(crate) const CALL_SUBMIT: u32 = 13;
pub(crate) const CALL_STATE: u32 = 14;
pub(crate) const CALL_DONE: u32 = 16;
pub(crate) const CALL_LOSE: u32 = 17;
pub(crate) const CALL_RESERVE: u32 = 18;
pub(crate) const CALL_PIPELINE_LAYOUT: u32 = 19;
pub(crate) const CALL_RGBA: u32 = 20;

/// a call of web/gpu.js with its numbers
pub(crate) fn call(op: u32, args: &[u32]) -> u32 {
    // SAFETY: the host reads args during the call; addresses among them are live for it (their
    // callers hold the slices), it keeps nothing.
    unsafe { host_call(op, args.as_ptr(), args.len()) }
}

/// an object of web/gpu.js's table: freed when the last Rc goes; buffers and textures destroyed (what
/// was submitted before finishes first: WebGPU's rule)
pub(crate) struct Handle {
    pub h: u32,
    pub destroy: bool,
}

impl Drop for Handle {
    fn drop(&mut self) {
        call(CALL_FREE, &[self.h, self.destroy as u32]);
    }
}

/// an object from a descriptor (JSON; {"$":n} names object n): `method` of the device, or 100, a view
/// of texture `parent` -> its number
pub(crate) fn create(parent: u32, method: u32, json: &str) -> Result<u32> {
    match call(CALL_CREATE, &[parent, method, json.as_ptr() as u32, json.len() as u32]) {
        0 => Err(GpuError::Failed(0)),
        h => Ok(h),
    }
}

const STATE_LOST: u32 = 1;
const STATE_VALIDATION: u32 = 2;
const STATE_MEMORY: u32 = 4;
const STATE_INTERNAL: u32 = 8;

pub(crate) struct DevInner {
    pub h: u32,
    pub info: AdapterInfo,
    pub features: Features,
    pub limits: Limits,
    /// a test said so (the browser's answer comes later)
    lost: Cell<bool>,
    last: Cell<u64>,
    done: Cell<u64>,
    /// what the buffers and textures asked for (the browser tells nothing)
    pub reserved: Cell<u64>,
    /// the words of finished command buffers are thrown away: this keeps one allocation
    pub spare: RefCell<Vec<u32>>,
}

impl DevInner {
    /// the browser's word on this device since the last look: lost, or an error of an earlier call
    pub fn check(&self) -> Result<()> {
        if self.lost.get() {
            return Err(GpuError::DeviceLost);
        }
        let s = call(CALL_STATE, &[self.h]);
        if s & STATE_LOST != 0 {
            self.lost.set(true);
            return Err(GpuError::DeviceLost);
        }
        if s & STATE_MEMORY != 0 {
            return Err(GpuError::OutOfMemory);
        }
        if s & STATE_VALIDATION != 0 {
            return Err(GpuError::Invalid("the browser's WebGPU refused a call (the console says which)"));
        }
        if s & STATE_INTERNAL != 0 {
            return Err(GpuError::Failed(1));
        }
        Ok(())
    }
}

/// the browser's WebGPU; adapter and device come asynchronously (prepare)
#[derive(Clone)]
pub struct Instance {
    adapter: Rc<RefCell<Option<Adapter>>>,
}

/// the names of the Features WebGPU has, and what they stand for here
fn features_of(list: &str) -> Features {
    let has = |n: &str| list.split(',').any(|f| f == n);
    Features {
        draw_indirect_first_instance: has("indirect-first-instance"),
        subgroups_webgpu: has("subgroups"),
        shader_f16: has("shader-f16"),
        texture_bc: has("texture-compression-bc"),
        anisotropy: true,
        depth_clamp: has("depth-clip-control"),
        fragment_stores: true,
        clip_distances: has("clip-distances"),
        primitive_index: has("primitive-index"),
        swapchain: true,
        ..Features::default()
    }
}

fn vendor_name(v: &str) -> &'static str {
    match v {
        "nvidia" => "NVIDIA",
        "amd" | "ati" => "AMD",
        "intel" => "Intel",
        "apple" => "Apple",
        "arm" => "ARM",
        "qualcomm" => "Qualcomm",
        "imagination" | "img-tec" => "Imagination",
        "samsung" => "Samsung",
        "google" => "Google",
        "microsoft" => "Microsoft",
        "mesa" => "Mesa",
        _ => "",
    }
}

fn read_adapter() -> Option<Adapter> {
    let mut buf = vec![0u8; 8192];
    let n = call(CALL_INFO, &[buf.as_mut_ptr() as u32, buf.len() as u32]) as usize;
    let text = core::str::from_utf8(&buf[..n]).ok()?;
    if text.is_empty() {
        return None;
    }
    let get = |k: &str| text.lines().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('='))).unwrap_or("");
    let num = |k: &str| get(k).parse::<f64>().map_or(0, |v| v.min(u32::MAX as f64) as u32);
    let (vendor, arch, device, desc) = (get("vendor"), get("arch"), get("device"), get("desc"));
    let name = if !desc.is_empty() {
        String::from(desc)
    } else {
        let mut s = String::from(vendor);
        for p in [arch, device] {
            if !p.is_empty() {
                s.push(' ');
                s.push_str(p);
            }
        }
        s
    };
    let fallback = get("fallback") == "1";
    let info = AdapterInfo {
        name,
        vendor_id: 0,
        device_id: 0,
        vendor: vendor_name(vendor),
        // (the browser does not say integrated or discrete)
        kind: if fallback { AdapterKind::Cpu } else { AdapterKind::Other },
        driver: String::from(vendor),
        driver_info: String::from(arch),
        driver_version: String::new(),
        api_version: (1, 0, 0),
        backend: "WebGPU",
        missing: None,
    };
    let limits = Limits {
        max_texture_2d: num("maxTextureDimension2D"),
        max_texture_3d: num("maxTextureDimension3D"),
        max_texture_layers: num("maxTextureArrayLayers"),
        max_storage_binding: num("maxStorageBufferBindingSize"),
        max_uniform_binding: num("maxUniformBufferBindingSize"),
        max_push_constants: PUSH_CONSTANTS_MAX,
        // (with push constants the last of them is theirs: create_pipeline_layout)
        max_bind_groups: num("maxBindGroups"),
        max_compute_workgroup_size: [num("maxComputeWorkgroupSizeX"), num("maxComputeWorkgroupSizeY"), num("maxComputeWorkgroupSizeZ")],
        max_compute_invocations: num("maxComputeInvocationsPerWorkgroup"),
        max_compute_workgroups: [num("maxComputeWorkgroupsPerDimension"); 3],
        max_compute_shared_memory: num("maxComputeWorkgroupStorageSize"),
        min_uniform_offset_alignment: num("minUniformBufferOffsetAlignment"),
        min_storage_offset_alignment: num("minStorageBufferOffsetAlignment"),
        max_color_targets: num("maxColorAttachments"),
        max_vertex_buffers: num("maxVertexBuffers"),
        max_vertex_attributes: num("maxVertexAttributes"),
        max_draw_indirect_count: 1,
        max_anisotropy: 16.0,
        timestamp_period_ns: 0.0,
        subgroup_size: 0,
        subgroup_min_size: num("minSubgroupSize"),
        subgroup_max_size: num("maxSubgroupSize"),
        ..Limits::default()
    };
    Some(Adapter(Rc::new(AdapterInner { info, features: features_of(get("features")), limits, taken: Cell::new(false) })))
}

impl Instance {
    /// the browser's WebGPU, adapter and device asked for (prepare says when they are there)
    pub fn new() -> Result<Instance> {
        Instance::with(&InstanceDesc::DEFAULT)
    }

    /// (app_name and validate mean nothing here: the browser validates every call)
    pub fn with(_desc: &InstanceDesc) -> Result<Instance> {
        if matches!(plat::env("jfgui_GPU").as_deref(), Some("0") | Some("off")) {
            return Err(GpuError::Disabled);
        }
        match call(CALL_PREPARE, &[1]) {
            0 => Err(GpuError::NoLoader),
            _ => Ok(Instance { adapter: Rc::new(RefCell::new(None)) }),
        }
    }

    /// have the browser's adapter and device arrived?  Never waits; a worker gets them by
    /// returning to the browser's event loop.  (true at once on the other targets.)
    pub fn prepare(&self) -> Result<bool> {
        if self.adapter.borrow().is_some() {
            return Ok(true);
        }
        match call(CALL_PREPARE, &[0]) {
            0 => Err(GpuError::NoLoader),
            1 => Ok(false),
            2 => {
                let a = read_adapter().ok_or(GpuError::NoAdapter)?;
                *self.adapter.borrow_mut() = Some(a);
                Ok(true)
            }
            3 => Err(GpuError::NoAdapter),
            _ => Err(GpuError::Failed(0)),
        }
    }

    pub fn validating(&self) -> bool {
        false
    }

    /// the one adapter the browser offers
    pub fn adapters(&self) -> Result<Vec<Adapter>> {
        Ok(vec![self.adapter()?])
    }

    pub fn adapter(&self) -> Result<Adapter> {
        self.prepare()?;
        self.adapter.borrow().clone().ok_or(GpuError::NotReady)
    }

    /// the adapter for showing windows: not a software one, which is no gain over the CPU path
    pub fn adapters_low_power(&self, _display: Option<(u32, u32)>) -> Result<Vec<Adapter>> {
        let a = self.adapter()?;
        Ok(if a.info().kind == AdapterKind::Cpu { Vec::new() } else { vec![a] })
    }
}

/// the order of adapters for showing windows (the browser has one; see adapters_low_power)
pub fn low_power_rank(kind: AdapterKind, _drives_display: bool) -> Option<u32> {
    (kind != AdapterKind::Cpu).then_some(0)
}

/// nothing counts them here: the browser's console shows what its validation says
pub fn validation_messages() -> (u32, u32) {
    (0, 0)
}

struct AdapterInner {
    info: AdapterInfo,
    features: Features,
    limits: Limits,
    taken: Cell<bool>,
}

/// the GPU the browser offers
#[derive(Clone)]
pub struct Adapter(Rc<AdapterInner>);

impl Adapter {
    pub fn info(&self) -> &AdapterInfo {
        &self.0.info
    }
    pub fn features(&self) -> &Features {
        &self.0.features
    }
    pub fn limits(&self) -> &Limits {
        &self.0.limits
    }
    /// always Basis in the browser
    pub fn level(&self) -> Level {
        Level::Basis
    }
    pub fn can_present(&self, _surface: &crate::Surface) -> bool {
        true
    }

    /// the device of this adapter, which prepare got (WebGPU gives one per adapter: a second one
    /// takes a new Instance)
    pub fn create_device(&self, _desc: &DeviceDesc) -> Result<Device> {
        if self.0.taken.replace(true) {
            return Err(GpuError::Unsupported("one device per Instance in the browser"));
        }
        let h = call(CALL_DEVICE, &[]);
        if h == 0 {
            return Err(GpuError::NotReady);
        }
        let d = Rc::new(DevInner {
            h,
            info: self.0.info.clone(),
            features: self.0.features,
            limits: self.0.limits,
            lost: Cell::new(false),
            last: Cell::new(0),
            done: Cell::new(0),
            reserved: Cell::new(0),
            spare: RefCell::new(Vec::new()),
        });
        Ok(Device { d: d.clone(), queue: Queue { d } })
    }
}

/// a logical GPU device: creates resources, records commands, owns the queue
#[derive(Clone)]
pub struct Device {
    pub(crate) d: Rc<DevInner>,
    queue: Queue,
}

/// the device's queue
#[derive(Clone)]
pub struct Queue {
    pub(crate) d: Rc<DevInner>,
}

/// (G5d) a queue of its own for compute: WebGPU has one queue, so Device::compute_queue is None here and there is no
/// value of this type
#[derive(Clone)]
pub struct ComputeQueue {
    _none: (),
}

impl ComputeQueue {
    pub fn encoder(&self) -> Result<crate::CommandEncoder<'_>> {
        Err(GpuError::Unsupported("a compute queue in the browser"))
    }
    pub fn submit<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(&self, _cmds: I) -> Result<SubmitId> {
        Err(GpuError::Unsupported("a compute queue in the browser"))
    }
    pub fn submit_after<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(&self, _wait: &[SubmitId], _cmds: I) -> Result<SubmitId> {
        Err(GpuError::Unsupported("a compute queue in the browser"))
    }
}

impl Device {
    pub fn queue(&self) -> &Queue {
        &self.queue
    }
    /// (G5d) WebGPU has one queue: None
    pub fn compute_queue(&self) -> Option<&ComputeQueue> {
        None
    }
    pub fn info(&self) -> &AdapterInfo {
        &self.d.info
    }
    pub fn features(&self) -> &Features {
        &self.d.features
    }
    pub fn limits(&self) -> &Limits {
        &self.d.limits
    }
    pub fn level(&self) -> Level {
        Level::Basis
    }
    /// the device was lost: every call fails with DeviceLost; create a new device
    pub fn is_lost(&self) -> bool {
        matches!(self.d.check(), Err(GpuError::DeviceLost))
    }
    /// (test aid) the device is destroyed: the browser's own way of losing it
    pub fn test_lose(&self) {
        call(CALL_LOSE, &[self.d.h]);
        self.d.lost.set(true);
    }
    pub fn validating(&self) -> bool {
        false
    }
    /// what the browser has said since the last look (nothing to free here)
    pub fn poll(&self) -> Result<()> {
        self.d.check()
    }
    /// the browser cannot wait on this thread: at once whether the submission is done
    pub fn wait(&self, id: SubmitId, _timeout_ns: u64) -> Result<bool> {
        self.d.check()?;
        Ok(self.is_done(id))
    }
    pub fn wait_idle(&self) -> Result<()> {
        Err(GpuError::Unsupported("waiting for the GPU in the browser: Device::is_done"))
    }
    pub fn completed(&self) -> SubmitId {
        SubmitId(self.d.done.get())
    }
    /// the submission id is done on the GPU (the browser's answer comes with its event loop)
    pub fn is_done(&self, id: SubmitId) -> bool {
        if self.d.done.get() >= id.0 {
            return true;
        }
        if call(CALL_DONE, &[self.d.h, id.0 as u32]) != 0 {
            self.d.done.set(id.0);
            return true;
        }
        false
    }
    /// what the buffers and textures asked for (the browser says nothing about its own memory)
    pub fn memory_reserved(&self) -> u64 {
        self.d.reserved.get()
    }
}

fn split(v: u64) -> (u32, u32) {
    (v as u32, (v >> 32) as u32)
}

impl Queue {
    /// run command buffers in order -> their number
    pub fn submit<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(&self, cmds: I) -> Result<SubmitId> {
        self.d.check()?;
        let mut words: Vec<u32> = core::mem::take(&mut *self.d.spare.borrow_mut());
        words.clear();
        for c in cmds {
            words.extend_from_slice(&c.words);
            words.push(0);
        }
        let id = call(CALL_SUBMIT, &[self.d.h, words.as_ptr() as u32, words.len() as u32]);
        *self.d.spare.borrow_mut() = words;
        self.d.last.set(id as u64);
        Ok(SubmitId(id as u64))
    }

    /// (G5d) one queue: its submissions run in order, so this is submit
    pub fn submit_after<'a, I: IntoIterator<Item = crate::CommandBuffer<'a>>>(&self, _wait: &[SubmitId], cmds: I) -> Result<SubmitId> {
        self.submit(cmds)
    }

    /// nothing waits for a submission here
    pub fn flush(&self) -> Result<SubmitId> {
        self.d.check()?;
        Ok(SubmitId(self.d.last.get()))
    }

    /// write data into a buffer at offset, before the next submission
    pub fn write_buffer<T: Pod>(&self, buf: &crate::Buffer, offset: u64, data: &[T]) -> Result<()> {
        let bytes = bytes_of(data);
        if offset % 4 != 0 || bytes.len() % 4 != 0 {
            return Err(GpuError::Invalid("write_buffer: offset and size must be multiples of 4 in WebGPU"));
        }
        if offset.checked_add(bytes.len() as u64).is_none_or(|e| e > buf.size()) {
            return Err(GpuError::Invalid("write_buffer: outside the buffer"));
        }
        let (lo, hi) = split(offset);
        call(CALL_WRITE_BUFFER, &[self.d.h, buf.handle(), lo, hi, bytes.as_ptr() as u32, bytes.len() as u32]);
        Ok(())
    }

    /// write texels into a texture region, before the next submission (no layouts here)
    pub fn write_texture(&self, dst: &TextureRegion, data: &[u8], layout: DataLayout, _before: Layout, _after: Layout) -> Result<()> {
        let f = dst.texture.format();
        let row = layout.bytes_per_row.max(((dst.size[0] / f.block_dim()).max(1)) * f.block_bytes());
        let rows = if layout.rows_per_image != 0 { layout.rows_per_image } else { dst.size[1] / f.block_dim() };
        let z = if dst.texture.depth() > 1 { dst.origin[2] } else { dst.base_layer };
        let d = if dst.texture.depth() > 1 { dst.size[2].max(1) } else { dst.layers.max(1) };
        if layout.offset > data.len() as u64 {
            return Err(GpuError::Invalid("write_texture: the offset lies outside the data"));
        }
        let base = data.as_ptr() as u32 + layout.offset as u32;
        call(CALL_WRITE_TEXTURE, &[self.d.h, dst.texture.handle(), dst.mip, dst.origin[0], dst.origin[1], z, dst.size[0], dst.size[1], d, base, data.len() as u32, row, rows]);
        Ok(())
    }

    /// write rectangles (x, y, w, h; clipped to the texture, empty ones skipped) of mip 0, layer 0 from
    /// `data`, an image of the texture's size in rows of `stride` bytes (a framebuffer)
    pub fn write_texture_rects(&self, t: &crate::Texture, rects: &[[i32; 4]], data: &[u8], stride: u32, before: Layout, after: Layout) -> Result<()> {
        self.write_texture_rects_at(t, rects, (0, 0), data, stride, before, after)
    }

    /// write_texture_rects from an image of another size: rectangles of `data` go to (x - origin.0,
    /// y - origin.1) of the texture; what lies outside the image or the texture is skipped
    pub fn write_texture_rects_at(&self, t: &crate::Texture, rects: &[[i32; 4]], origin: (i32, i32), data: &[u8], stride: u32, _before: Layout, _after: Layout) -> Result<()> {
        let (tw, th) = (t.width() as i32, t.height() as i32);
        let bytes = t.format().block_bytes() as i32;
        if stride == 0 || bytes != 4 {
            return Err(GpuError::Invalid("write_texture_rects: a 32-bit format and a stride"));
        }
        let (img_w, img_h) = (stride as i32 / 4, (data.len() / stride as usize) as i32);
        let mut list: Vec<[i32; 4]> = Vec::with_capacity(rects.len());
        for r in rects {
            let x0 = r[0].max(origin.0).max(0);
            let y0 = r[1].max(origin.1).max(0);
            let x1 = r[0].saturating_add(r[2].max(0)).min(origin.0.saturating_add(tw)).min(img_w);
            let y1 = r[1].saturating_add(r[3].max(0)).min(origin.1.saturating_add(th)).min(img_h);
            if x0 < x1 && y0 < y1 {
                list.push([x0, y0, x1 - x0, y1 - y0]);
            }
        }
        if !list.is_empty() {
            call(CALL_WRITE_RECTS, &[self.d.h, t.handle(), list.as_ptr() as u32, list.len() as u32, data.as_ptr() as u32, data.len() as u32, stride, origin.0 as u32, origin.1 as u32]);
        }
        Ok(())
    }

    /// the worker cannot wait for the GPU: a Readback buffer and Buffer::read
    pub fn read_buffer<T: Pod>(&self, _buf: &crate::Buffer, _offset: u64, _out: &mut [T]) -> Result<()> {
        Err(GpuError::Unsupported("Queue::read_buffer waits for the GPU; in the browser a Readback buffer and Buffer::read"))
    }
}

/// a number of a JSON text
pub(crate) fn json_f64(v: f64) -> String {
    if v.is_finite() { format!("{v}") } else { String::from("0") }
}
