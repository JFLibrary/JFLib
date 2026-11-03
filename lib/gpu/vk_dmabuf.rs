// vk_dmabuf -- the swapchain of a Linux program: images without a VkSurface (programs against glibc;
// docs/GPU.md "Ausgabe unter Linux")
//
// Vulkan's Wayland surface wants a pointer from libwayland-client, the X11 one from libxcb.  JFLib
// speaks both protocols itself and has neither.  So the GPU draws into images of its own, exported as
// dma-bufs, and plat hands them to the window (lib/plat/linux_gpuout.rs).  For the caller this is the
// same as vk_swap.rs: Surface, create_swapchain, try_acquire, view, present, rebuild.
//
//   Surface::native(&instance, plat::win_gpu_child(w), w)   what the display of window w accepts
//   surface.track_size()              (the window loop's thread) the window's size for a rebuild on a
//                                     worker: only that thread may ask plat
//   device.create_swapchain(&surface, &SwapchainDesc)       any thread
//        min_images images (0: 3), XRGB8888 (else XBGR8888, ARGB8888, ABGR8888) as B8G8R8A8 /
//        R8G8B8A8, single-plane modifiers the device can export and the display accepts: the driver
//        picks (VkImageDrmFormatModifierListCreateInfoEXT), a dedicated allocation, dma-buf
//   sc.try_acquire()                  never waits, never rebuilds; registers the images with plat at
//        its first call; looks at the frame before (flush); then NotReady while one is on its way or
//        the display has not answered the last commit (the frame stays in the caller's texture and
//        its newest state is drawn then: nothing queues up) or no image is free
//   sc.present(frame, [cmd, ..])      submits; the image goes to the window when the GPU has finished
//   sc.flush() -> still on its way    that look at the timeline: the frame goes to the window (the
//        window loop calls it, a frame is rarely done before present returns)
//   sc.busy()                         a frame is on its way: flush soon
//   sc.next_look(retry)               when a frame that found NotReady should try again
//   sc.damage(rects)                  what the next presented image changes against the last one (None:
//        all; without any call: all).  What the compositor has to draw again, nothing else
//   sc.acquire()                      waits (pumping the display, events stay queued) and rebuilds
//   sc.rebuild()                      any thread, without waiting: the old images die with the frames
//        that use them
//
// Why the image can never be wrong on the screen: linux_gpuout.rs.  Here: an image goes to plat only
// after its submission reached the timeline; an image is drawn into only when plat says FREE and a
// poll of its dma-buf for POLLOUT says no fence of another reader is left.  Queue ownership goes to
// VK_QUEUE_FAMILY_FOREIGN_EXT at the end of every frame and comes back at the start of the next.
// Compression (modifiers with more than one plane) is left out: the window system does not gain
// enough from it for the checks each driver wants at the hand-over.
//
// Threads: plat's display calls only on the thread of the window loop (linux_disp.rs).  So a worker's
// rebuild and any drop only note the buffers that go (RETIRED), try_acquire and flush free them.

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, Ordering};

use rt::sync::SpinLock;

use crate::vk_dev::{Device, Garbage, Shared};
use crate::vk_ffi as vk;
use crate::vk_inst::Adapter;
use crate::*;

const DRM_FORMAT_MOD_INVALID: u64 = 0x00FF_FFFF_FFFF_FFFF;
const FORMAT_FEATURE_TRANSFER_SRC_BIT: u32 = 0x4000;
const F_DUPFD_CLOEXEC: usize = 1030;
const POLLOUT: u16 = 4;
/// images of a swapchain without SwapchainDesc::min_images
const IMAGES_DEFAULT: u32 = 3;
/// while a frame is on its way: the next look at the timeline (us)
const GPU_LOOK_US: u64 = 1_000;
const WAIT_US: u64 = 10_000_000;

/// the DRM formats, as Vulkan calls them; the images are opaque (the fourth byte is never shown)
fn candidates(srgb: bool) -> [(u32, u32, Format); 4] {
    let (bgra, bf, rgba, rf) = if srgb {
        (vk::FORMAT_B8G8R8A8_SRGB, Format::Bgra8Srgb, vk::FORMAT_R8G8B8A8_SRGB, Format::Rgba8Srgb)
    } else {
        (vk::FORMAT_B8G8R8A8_UNORM, Format::Bgra8Unorm, vk::FORMAT_R8G8B8A8_UNORM, Format::Rgba8Unorm)
    };
    [(0x3432_5258, bgra, bf), (0x3432_4258, rgba, rf), (0x3432_5241, bgra, bf), (0x3432_4241, rgba, rf)]
}

/// the buffers that go: (window, plat's handle)
static RETIRED: SpinLock<Vec<(i32, u32)>> = SpinLock::new(Vec::new());

fn free_retired() {
    let list = core::mem::take(&mut *RETIRED.lock());
    for (w, h) in list {
        plat::gpu_buf_free(w, h);
    }
}

fn fd_close(fd: i32) {
    // SAFETY: a descriptor of ours.
    unsafe { rt::os::syscall3(rt::os::SYS_CLOSE, fd as usize, 0, 0) };
}

pub(crate) struct SurfShared {
    pub window: i32,
    caps: plat::GpuCaps,
    size: AtomicU64,
}

/// a window as a target for swapchains
#[derive(Clone)]
pub struct Surface(pub(crate) Arc<SurfShared>);

impl Surface {
    /// the surface of plat window `window`
    pub fn new(instance: &Instance, window: i32) -> Result<Surface> {
        let nw = plat::win_gpu_child(window).ok_or(GpuError::Unsupported("the display takes no GPU buffers"))?;
        Surface::native(instance, nw, window)
    }

    /// the surface from what plat::win_gpu_child said about plat window `window` (any thread)
    pub fn native(_instance: &Instance, nw: plat::NativeWindow, window: i32) -> Result<Surface> {
        let (w, h) = nw.size;
        Ok(Surface(Arc::new(SurfShared { window, caps: nw.caps, size: AtomicU64::new(pack(w, h)) })))
    }

    /// (the window loop's thread) the window's size now: what a rebuild on a worker makes
    pub fn track_size(&self) {
        let (w, h) = plat::win_size(self.0.window);
        self.0.size.store(pack(w, h), Ordering::Relaxed);
        free_retired();
    }

    fn size(&self) -> (u32, u32) {
        let v = self.0.size.load(Ordering::Relaxed);
        ((v >> 32) as u32, v as u32)
    }
}

fn pack(w: i32, h: i32) -> u64 {
    (w.max(0) as u64) << 32 | h.max(0) as u64
}

/// an acquired image (present it with Swapchain::present)
#[must_use]
pub struct Frame {
    image: u32,
    generation: u32,
    size: (u32, u32),
}

/// what Swapchain::try_acquire found
#[must_use]
pub enum TryAcquire {
    Frame(Frame),
    /// no image is free yet, or the display has not answered the last frame: try again later
    NotReady,
    /// the swapchain does not fit the window any more: Swapchain::rebuild first
    Stale,
    /// the window is minimised or has no area
    NoArea,
}

impl Frame {
    /// the image's index in the swapchain
    pub fn index(&self) -> u32 {
        self.image
    }
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
}

struct Img {
    tex: Texture,
    image: vk::Handle,
    mem: vk::Handle,
    fd: i32,     // the dma-buf (poll; a copy goes to plat)
    buf: u32,    // plat's handle (0: not registered yet)
    offset: u32,
    stride: u32,
}

/// the images a window shows, presented in turn
pub struct Swapchain {
    sh: Arc<Shared>,
    surf: Surface,
    desc: SwapchainDesc,
    format: Format,
    vk_format: u32,
    fourcc: u32,
    mods: Vec<u64>,
    modifier: u64,
    mode: PresentMode,
    extent: (u32, u32),
    images: Vec<Img>,
    generation: u32,
    next: Cell<u32>,
    stale: Cell<bool>,
    outstanding: Cell<bool>,
    /// the frame on its way: image, timeline value, its damage (None: all)
    fly: RefCell<Option<(u32, u64, Option<Vec<[i32; 4]>>)>>,
    dmg: RefCell<Vec<[i32; 4]>>,
    dmg_full: Cell<bool>,
    dmg_known: Cell<bool>,
    /// the last try_acquire found the display not ready
    waiting: Cell<bool>,
    /// the first frame after prepare does not wait for the display's answer to the CPU path's last
    /// frame: with a moving window there always is one on its way
    first: Cell<bool>,
}

fn present_mode_of(m: PresentMode) -> PresentMode {
    // one image per answer of the display, always the newest: what MAILBOX means to a window; a
    // client of the window system has no way to tear, and FIFO's queue would only add latency
    match m {
        PresentMode::Fifo => PresentMode::Fifo,
        _ => PresentMode::Mailbox,
    }
}

/// the modifiers (single plane) a device can draw into in this format
fn device_modifiers(a: &crate::vk_inst::InstShared, pd: vk::PhysicalDevice, format: u32, features: u32) -> Vec<u64> {
    let Some(get) = a.f.get_physical_device_format_properties2 else { return Vec::new() };
    let mut list: vk::DrmFormatModifierPropertiesListEXT = vk::Zero::zero();
    list.s_type = vk::STRUCTURE_TYPE_DRM_FORMAT_MODIFIER_PROPERTIES_LIST_EXT;
    let mut fp: vk::FormatProperties2 = vk::Zero::zero();
    fp.s_type = vk::STRUCTURE_TYPE_FORMAT_PROPERTIES_2;
    fp.p_next = &mut list as *mut _ as *mut c_void;
    // SAFETY: the two-call enumeration of a valid chain.
    unsafe { get(pd, format, &mut fp) };
    let mut v: Vec<vk::DrmFormatModifierPropertiesEXT> = vec![vk::Zero::zero(); list.count as usize];
    list.p_properties = v.as_mut_ptr();
    fp.p_next = &mut list as *mut _ as *mut c_void;
    // SAFETY: v holds list.count entries.
    unsafe { get(pd, format, &mut fp) };
    v.truncate(list.count as usize);
    v.iter().filter(|m| m.plane_count == 1 && m.tiling_features & features == features).map(|m| m.modifier).collect()
}

/// an image of this kind can be made and its memory exported
fn exportable(a: &crate::vk_inst::InstShared, pd: vk::PhysicalDevice, format: u32, usage: u32, modifier: u64) -> bool {
    let Some(get) = a.f.get_physical_device_image_format_properties2 else { return false };
    let dm = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT {
        s_type: vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_DRM_FORMAT_MODIFIER_INFO_EXT,
        modifier,
        sharing_mode: vk::SHARING_MODE_EXCLUSIVE,
        ..vk::Zero::zero()
    };
    let ei = vk::PhysicalDeviceExternalImageFormatInfo {
        s_type: vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_IMAGE_FORMAT_INFO,
        p_next: &dm as *const _ as *const c_void,
        handle_type: vk::EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
    };
    let info = vk::PhysicalDeviceImageFormatInfo2 {
        s_type: vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_FORMAT_INFO_2,
        p_next: &ei as *const _ as *const c_void,
        format,
        image_type: vk::IMAGE_TYPE_2D,
        tiling: vk::IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT,
        usage,
        flags: 0,
    };
    let mut ep: vk::ExternalImageFormatProperties = vk::Zero::zero();
    ep.s_type = vk::STRUCTURE_TYPE_EXTERNAL_IMAGE_FORMAT_PROPERTIES;
    let mut p: vk::ImageFormatProperties2 = vk::Zero::zero();
    p.s_type = vk::STRUCTURE_TYPE_IMAGE_FORMAT_PROPERTIES_2;
    p.p_next = &mut ep as *mut _ as *mut c_void;
    // SAFETY: valid chains on this stack frame.
    let r = unsafe { get(pd, &info, &mut p) };
    r == vk::SUCCESS && ep.external_memory_properties.external_memory_features & vk::EXTERNAL_MEMORY_FEATURE_EXPORTABLE_BIT != 0
}

fn image_usage(copy_src: bool) -> (u32, u32) {
    let mut usage = vk::IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
    let mut features = vk::FORMAT_FEATURE_COLOR_ATTACHMENT_BIT;
    if copy_src {
        usage |= vk::IMAGE_USAGE_TRANSFER_SRC_BIT;
        features |= FORMAT_FEATURE_TRANSFER_SRC_BIT;
    }
    (usage, features)
}

/// the first format both sides have -> (fourcc, Vulkan format, Format, the modifiers both accept)
fn pick_format(a: &crate::vk_inst::InstShared, pd: vk::PhysicalDevice, caps: &plat::GpuCaps, srgb: bool, copy_src: bool)
               -> Option<(u32, u32, Format, Vec<u64>)> {
    let (usage, features) = image_usage(copy_src);
    for (fourcc, vf, ff) in candidates(srgb) {
        if !caps.formats.iter().any(|e| e.0 == fourcc && e.1 != DRM_FORMAT_MOD_INVALID) {
            continue;
        }
        let mods: Vec<u64> = device_modifiers(a, pd, vf, features)
            .into_iter()
            .filter(|&m| caps.formats.contains(&(fourcc, m)) && exportable(a, pd, vf, usage, m))
            .collect();
        if !mods.is_empty() {
            return Some((fourcc, vf, ff, mods));
        }
    }
    None
}

/// the device the display renders with is this adapter's (not known: yes)
fn same_device(adapter: &Adapter, caps: &plat::GpuCaps) -> bool {
    let (Some(d), true) = (caps.device, adapter.exts.physical_drm) else { return true };
    let major = ((d >> 32) & 0xFFFF_F000 | (d >> 8) & 0xFFF) as i64;
    let minor = ((d >> 12) & 0xFFFF_FF00 | d & 0xFF) as i64;
    let mut drm: vk::PhysicalDeviceDrmPropertiesEXT = vk::Zero::zero();
    drm.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_DRM_PROPERTIES_EXT;
    let mut p2: vk::PhysicalDeviceProperties2 = vk::Zero::zero();
    p2.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2;
    p2.p_next = &mut drm as *mut _ as *mut c_void;
    // SAFETY: a valid chain on this stack frame.
    unsafe { (adapter.inst.f.get_physical_device_properties2)(adapter.pd, &mut p2) };
    (drm.has_primary != 0 && (drm.primary_major, drm.primary_minor) == (major, minor))
        || (drm.has_render != 0 && (drm.render_major, drm.render_minor) == (major, minor))
}

/// (Adapter::can_present) a device of this adapter can show images on the surface
pub(crate) fn can_present(adapter: &Adapter, surface: &Surface) -> bool {
    adapter.exts.dmabuf
        && adapter.info.kind != AdapterKind::Cpu
        && same_device(adapter, &surface.0.caps)
        && pick_format(&adapter.inst, adapter.pd, &surface.0.caps, false, false).is_some()
}

impl Device {
    pub fn create_swapchain(&self, surface: &Surface, desc: &SwapchainDesc) -> Result<Swapchain> {
        let sh = &self.sh;
        sh.check()?;
        if !sh.features.swapchain {
            return Err(GpuError::Unsupported("images for a window (VK_EXT_image_drm_format_modifier, not on a CPU adapter)"));
        }
        let (fourcc, vk_format, format, mods) = pick_format(&sh.inst, sh.pd, &surface.0.caps, desc.srgb, desc.copy_src)
            .ok_or(GpuError::Unsupported("a format the display and the device share"))?;
        let mut d = *desc;
        d.frames_in_flight = d.frames_in_flight.clamp(1, 3);
        match plat::env("jfgui_GPU_PRESENT").as_deref() {
            Some("fifo") => d.present = PresentMode::Fifo,
            Some("mailbox") => d.present = PresentMode::Mailbox,
            Some("immediate") => d.present = PresentMode::Immediate,
            _ => {}
        }
        let mut sc = Swapchain {
            sh: sh.clone(),
            surf: surface.clone(),
            desc: d,
            format,
            vk_format,
            fourcc,
            mods,
            modifier: 0,
            mode: present_mode_of(d.present),
            extent: (0, 0),
            images: Vec::new(),
            generation: 0,
            next: Cell::new(0),
            stale: Cell::new(false),
            outstanding: Cell::new(false),
            fly: RefCell::new(None),
            dmg: RefCell::new(Vec::new()),
            dmg_full: Cell::new(false),
            dmg_known: Cell::new(false),
            waiting: Cell::new(false),
            first: Cell::new(false),
        };
        sc.build()?;
        Ok(sc)
    }
}

impl Swapchain {
    /// the images and what hangs on them go: when the GPU has finished with them
    fn release(&mut self) {
        let window = self.surf.0.window;
        for m in self.images.drain(..) {
            if m.buf != 0 {
                RETIRED.lock().push((window, m.buf));
            }
            fd_close(m.fd);
            drop(m.tex);
            self.sh.defer(Garbage::Image(m.image, None));
            self.sh.defer(Garbage::Memory(m.mem));
        }
        *self.fly.borrow_mut() = None;
    }

    /// an exported image; the driver picks from mods
    fn make_image(&self, w: u32, h: u32, mods: &[u64]) -> Result<(Img, u64)> {
        let sh = self.sh.clone();
        let (Some(get_mod), Some(get_layout), Some(get_fd)) =
            (sh.f.get_image_drm_format_modifier_properties_ext, sh.f.get_image_subresource_layout, sh.f.get_memory_fd_khr)
        else {
            return Err(GpuError::Unsupported("VK_EXT_image_drm_format_modifier"));
        };
        let (usage, _) = image_usage(self.desc.copy_src);
        let list = vk::ImageDrmFormatModifierListCreateInfoEXT {
            s_type: vk::STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_LIST_CREATE_INFO_EXT,
            p_next: core::ptr::null(),
            count: mods.len() as u32,
            p_modifiers: mods.as_ptr(),
        };
        let ext = vk::ExternalMemoryImageCreateInfo {
            s_type: vk::STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO,
            p_next: &list as *const _ as *const c_void,
            handle_types: vk::EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
        };
        let ii = vk::ImageCreateInfo {
            s_type: vk::STRUCTURE_TYPE_IMAGE_CREATE_INFO,
            p_next: &ext as *const _ as *const c_void,
            image_type: vk::IMAGE_TYPE_2D,
            format: self.vk_format,
            extent: vk::Extent3D { width: w, height: h, depth: 1 },
            mip_levels: 1,
            array_layers: 1,
            samples: vk::SAMPLE_COUNT_1_BIT,
            tiling: vk::IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT,
            usage,
            sharing_mode: vk::SHARING_MODE_EXCLUSIVE,
            initial_layout: vk::IMAGE_LAYOUT_UNDEFINED,
            ..vk::Zero::zero()
        };
        let mut image = 0;
        // SAFETY: valid chains on this stack frame.
        sh.vk(unsafe { (sh.f.create_image)(sh.dev, &ii, core::ptr::null(), &mut image) })?;
        let mut mem = 0;
        let mut fd = -1;
        let made = (|| {
            let mut mp: vk::ImageDrmFormatModifierPropertiesEXT = vk::Zero::zero();
            mp.s_type = vk::STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_PROPERTIES_EXT;
            // SAFETY: the image just created with a modifier.
            sh.vk(unsafe { get_mod(sh.dev, image, &mut mp) })?;
            let mut req: vk::MemoryRequirements = vk::Zero::zero();
            // SAFETY: the image just created.
            unsafe { (sh.f.get_image_memory_requirements)(sh.dev, image, &mut req) };
            let mut props: vk::PhysicalDeviceMemoryProperties = vk::Zero::zero();
            // SAFETY: plain out-parameter.
            unsafe { (sh.inst.f.get_physical_device_memory_properties)(sh.pd, &mut props) };
            let n = props.memory_type_count.min(32);
            let ty = (0..n)
                .find(|&k| req.memory_type_bits & 1 << k != 0 && props.memory_types[k as usize].property_flags & vk::MEMORY_PROPERTY_DEVICE_LOCAL_BIT != 0)
                .or_else(|| (0..n).find(|&k| req.memory_type_bits & 1 << k != 0))
                .ok_or(GpuError::Unsupported("memory for an exported image"))?;
            let ded = vk::MemoryDedicatedAllocateInfo {
                s_type: vk::STRUCTURE_TYPE_MEMORY_DEDICATED_ALLOCATE_INFO,
                image,
                ..vk::Zero::zero()
            };
            let exp = vk::ExportMemoryAllocateInfo {
                s_type: vk::STRUCTURE_TYPE_EXPORT_MEMORY_ALLOCATE_INFO,
                p_next: &ded as *const _ as *const c_void,
                handle_types: vk::EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
            };
            let ai = vk::MemoryAllocateInfo {
                s_type: vk::STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                p_next: &exp as *const _ as *const c_void,
                allocation_size: req.size,
                memory_type_index: ty,
            };
            // SAFETY: valid chains on this stack frame.
            sh.vk(unsafe { (sh.f.allocate_memory)(sh.dev, &ai, core::ptr::null(), &mut mem) })?;
            // SAFETY: the dedicated allocation of this image.
            sh.vk(unsafe { (sh.f.bind_image_memory)(sh.dev, image, mem, 0) })?;
            let gi = vk::MemoryGetFdInfoKHR {
                s_type: vk::STRUCTURE_TYPE_MEMORY_GET_FD_INFO_KHR,
                p_next: core::ptr::null(),
                memory: mem,
                handle_type: vk::EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
            };
            // SAFETY: memory allocated for export.
            sh.vk(unsafe { get_fd(sh.dev, &gi, &mut fd) })?;
            let sub = vk::ImageSubresource { aspect_mask: vk::IMAGE_ASPECT_MEMORY_PLANE_0_BIT_EXT, mip_level: 0, array_layer: 0 };
            let mut lay: vk::SubresourceLayout = vk::Zero::zero();
            // SAFETY: plane 0 of an image with a single-plane modifier.
            unsafe { get_layout(sh.dev, image, &sub, &mut lay) };
            let mut tu = TextureUsage::COLOR_TARGET;
            if self.desc.copy_src {
                tu = tu | TextureUsage::COPY_SRC;
            }
            let tex = crate::vk_res::wrap_image(&sh, image, TextureDesc::d2(w, h, self.format, tu))?;
            Ok((tex, mp.modifier, lay.offset as u32, lay.row_pitch as u32))
        })();
        match made {
            Ok((tex, modifier, offset, stride)) => Ok((Img { tex, image, mem, fd, buf: 0, offset, stride }, modifier)),
            Err(e) => {
                if fd >= 0 {
                    fd_close(fd);
                }
                sh.defer(Garbage::Image(image, None));
                if mem != 0 {
                    sh.defer(Garbage::Memory(mem));
                }
                Err(e)
            }
        }
    }

    /// (re)create the images for the window's size as the window loop last said it (no area: none)
    fn build(&mut self) -> Result<()> {
        self.release();
        let (w, h) = self.surf.size();
        self.stale.set(false);
        self.outstanding.set(false);
        self.extent = (0, 0);
        if w == 0 || h == 0 {
            return Ok(());
        }
        let n = if self.desc.min_images != 0 { self.desc.min_images } else { IMAGES_DEFAULT }.clamp(2, 8);
        let mut mods = self.mods.clone();
        for _ in 0..n {
            let (img, m) = self.make_image(w, h, &mods)?;
            // the first image's choice for the others: one layout per swapchain
            mods = vec![m];
            self.modifier = m;
            self.images.push(img);
        }
        self.extent = (w, h);
        self.generation = self.generation.wrapping_add(1);
        self.dmg_full.set(true);
        Ok(())
    }

    /// the images for the size the window has now, without waiting (any thread)
    pub fn rebuild(&mut self) -> Result<()> {
        self.sh.check()?;
        self.build()
    }

    /// the images to plat (the window loop's thread)
    fn register(&mut self) -> Result<()> {
        let (w, h) = self.extent;
        for k in 0..self.images.len() {
            if self.images[k].buf != 0 {
                continue;
            }
            // SAFETY: a descriptor of ours, duplicated.
            let dup = unsafe { rt::os::syscall3(rt::os::SYS_FCNTL, self.images[k].fd as usize, F_DUPFD_CLOEXEC, 3) };
            if dup < 0 {
                return Err(GpuError::Unsupported("no descriptor for a buffer"));
            }
            let m = &self.images[k];
            let d = plat::GpuBufDesc {
                w: w as i32,
                h: h as i32,
                fourcc: self.fourcc,
                modifier: self.modifier,
                planes: vec![plat::GpuPlane { fd: dup as i32, offset: m.offset, stride: m.stride }],
            };
            let buf = plat::gpu_buf_new(self.surf.0.window, &d);
            if buf == 0 {
                return Err(GpuError::Unsupported("the display refuses the buffer"));
            }
            self.images[k].buf = buf;
        }
        Ok(())
    }

    /// nobody reads the image: no fence of another reader on its dma-buf
    fn unread(fd: i32) -> bool {
        #[repr(C)]
        struct PollFd {
            fd: i32,
            events: u16,
            revents: u16,
        }
        let mut p = PollFd { fd, events: POLLOUT, revents: 0 };
        // SAFETY: one pollfd, no wait.
        let r = unsafe { rt::os::poll(&mut p as *mut PollFd as usize, 1, 0) };
        r == 1
    }

    /// the frame on its way: finished by the GPU -> it goes to the window; -> still on its way
    pub fn flush(&self) -> Result<bool> {
        free_retired();
        let Some(&(k, v, _)) = self.fly.borrow().as_ref() else { return Ok(false) };
        if v > self.sh.refresh() {
            self.sh.check()?;
            return Ok(true);
        }
        let Some((_, _, rects)) = self.fly.borrow_mut().take() else { return Ok(false) };
        if let Some(m) = self.images.get(k as usize) {
            plat::gpu_show(self.surf.0.window, m.buf, rects.as_deref());
        }
        Ok(false)
    }

    /// a frame is on its way to the window: flush soon
    pub fn busy(&self) -> bool {
        self.fly.borrow().is_some()
    }

    /// when a frame that found NotReady should try again: soon when the GPU is drawing; when the
    /// display's answer is overdue if it has not given it (the answer itself ends the wait earlier);
    /// else retry, the caller's time
    pub fn next_look(&self, retry: u64) -> u64 {
        if self.busy() {
            return plat::time_us() + GPU_LOOK_US;
        }
        if !self.waiting.get() {
            return retry;
        }
        let w = self.surf.0.window;
        if plat::gpu_can_show(w) && self.images.iter().any(|m| plat::gpu_buf_state(w, m.buf) == plat::GPU_BUF_FREE) {
            return retry;
        }
        plat::gpu_next_look(w)
    }

    /// the images to the display, its answer there (the window loop's thread)
    pub fn prepare(&mut self) -> Result<bool> {
        self.register()?;
        let w = self.surf.0.window;
        for m in &self.images {
            match plat::gpu_buf_state(w, m.buf) {
                plat::GPU_BUF_FAILED => return Err(GpuError::Unsupported("the display refuses the buffer")),
                plat::GPU_BUF_PENDING => return Ok(false),
                _ => {}
            }
        }
        self.first.set(true);
        Ok(true)
    }

    /// what the next presented image changes against the last one: rectangles x, y, w, h (None: all)
    pub fn damage(&self, rects: Option<&[[i32; 4]]>) {
        self.dmg_known.set(true);
        match rects {
            Some(r) => self.dmg.borrow_mut().extend_from_slice(r),
            None => self.dmg_full.set(true),
        }
    }

    /// the next image to draw into, without waiting and without rebuilding (see TryAcquire)
    pub fn try_acquire(&mut self) -> Result<TryAcquire> {
        self.sh.check()?;
        free_retired();
        let window = self.surf.0.window;
        if plat::win_state(window) & plat::WST_MINIMIZED != 0 {
            return Ok(TryAcquire::NoArea);
        }
        let (ww, wh) = plat::win_size(window);
        let size = (ww.max(0) as u32, wh.max(0) as u32);
        if self.stale.get() || self.outstanding.get() || self.images.is_empty() || size != self.extent {
            self.stale.set(true);
            return Ok(TryAcquire::Stale);
        }
        self.register()?;
        self.waiting.set(true);
        if self.flush()? || (!self.first.get() && !plat::gpu_can_show(window)) {
            return Ok(TryAcquire::NotReady);
        }
        let n = self.images.len() as u32;
        let first = self.next.get();
        for j in 0..n {
            let k = (first + j) % n;
            let m = &self.images[k as usize];
            match plat::gpu_buf_state(window, m.buf) {
                plat::GPU_BUF_FAILED => return Err(GpuError::Unsupported("the display refuses the buffer")),
                plat::GPU_BUF_FREE if Self::unread(m.fd) => {
                    self.next.set((k + 1) % n);
                    self.outstanding.set(true);
                    self.waiting.set(false);
                    self.first.set(false);
                    return Ok(TryAcquire::Frame(Frame { image: k, generation: self.generation, size: self.extent }));
                }
                _ => {}
            }
        }
        Ok(TryAcquire::NotReady)
    }

    /// the next image to draw into, rebuilt when stale; waits (the display is pumped, events stay
    /// queued); None while the window is minimised or has no area
    pub fn acquire(&mut self) -> Result<Option<Frame>> {
        self.sh.check()?;
        let window = self.surf.0.window;
        if plat::win_state(window) & plat::WST_MINIMIZED != 0 {
            return Ok(None);
        }
        if self.outstanding.get() {
            self.stale.set(true); // the last frame was never presented
        }
        let dl = plat::time_us() + WAIT_US;
        loop {
            self.surf.track_size();
            let (w, h) = self.surf.size();
            if w == 0 || h == 0 {
                return Ok(None);
            }
            if self.stale.get() || self.images.is_empty() || (w, h) != self.extent {
                self.build()?;
            }
            match self.try_acquire()? {
                TryAcquire::Frame(f) => return Ok(Some(f)),
                TryAcquire::NoArea => return Ok(None),
                TryAcquire::Stale => {}
                TryAcquire::NotReady => {
                    let now = plat::time_us();
                    if now >= dl {
                        return Err(GpuError::Timeout);
                    }
                    plat::gpu_wait(self.next_look(now).min(now + 2_000));
                }
            }
        }
    }

    /// the frame's image as a render target (Undefined at the start, Present at the end)
    pub fn view(&self, frame: &Frame) -> &TextureView {
        self.images[frame.image as usize].tex.view()
    }

    /// the frame's image (copies: SwapchainDesc::copy_src)
    pub fn texture(&self, frame: &Frame) -> &Texture {
        &self.images[frame.image as usize].tex
    }

    // an image's queue ownership: barrier in a command buffer of its own
    fn ownership(&self, image: vk::Handle, acquire: bool) -> Result<crate::vk_dev::Slot> {
        let sh = &self.sh;
        let foreign = if sh.queue_foreign { vk::QUEUE_FAMILY_FOREIGN_EXT } else { vk::QUEUE_FAMILY_EXTERNAL };
        let slot = sh.take_slot()?;
        let range = vk::ImageSubresourceRange { aspect_mask: vk::IMAGE_ASPECT_COLOR_BIT, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 };
        let b = vk::ImageMemoryBarrier {
            s_type: vk::STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            src_access_mask: if acquire { 0 } else { vk::ACCESS_COLOR_ATTACHMENT_WRITE_BIT | vk::ACCESS_TRANSFER_READ_BIT },
            dst_access_mask: 0,
            old_layout: if acquire { vk::IMAGE_LAYOUT_UNDEFINED } else { vk::IMAGE_LAYOUT_GENERAL },
            new_layout: vk::IMAGE_LAYOUT_GENERAL,
            src_queue_family_index: if acquire { foreign } else { sh.family },
            dst_queue_family_index: if acquire { sh.family } else { foreign },
            image,
            subresource_range: range,
            ..vk::Zero::zero()
        };
        let (ss, ds) = if acquire {
            (vk::PIPELINE_STAGE_TOP_OF_PIPE_BIT, vk::PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT)
        } else {
            (vk::PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT | vk::PIPELINE_STAGE_TRANSFER_BIT, vk::PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT)
        };
        // SAFETY: a buffer in the recording state, one barrier on this stack frame.
        unsafe { (sh.f.cmd_pipeline_barrier)(slot.cb, ss, ds, 0, 0, core::ptr::null(), 0, core::ptr::null(), 1, &b) };
        // SAFETY: as above.
        if let Err(e) = sh.vk(unsafe { (sh.f.end_command_buffer)(slot.cb) }) {
            sh.give_slot(slot);
            return Err(e);
        }
        Ok(slot)
    }

    /// submit the frame's commands (they leave the image in Layout::Present); it goes to the window
    /// when the GPU has finished it (flush)
    pub fn present<'a, I: IntoIterator<Item = CommandBuffer<'a>>>(&self, frame: Frame, cmds: I) -> Result<SubmitId> {
        let sh = &self.sh;
        let mut slots = Vec::new();
        for c in cmds {
            slots.push(c.into_slot());
        }
        if frame.generation != self.generation || !self.outstanding.get() || self.images.is_empty() {
            for s in slots {
                sh.give_slot(s);
            }
            return Err(GpuError::Invalid("present: a frame of an older swapchain"));
        }
        let image = self.images[frame.image as usize].image;
        let first = match self.ownership(image, true) {
            Ok(s) => s,
            Err(e) => {
                for s in slots {
                    sh.give_slot(s);
                }
                return Err(e);
            }
        };
        let last = match self.ownership(image, false) {
            Ok(s) => s,
            Err(e) => {
                sh.give_slot(first);
                for s in slots {
                    sh.give_slot(s);
                }
                return Err(e);
            }
        };
        slots.insert(0, first);
        slots.push(last);
        let cbs: Vec<vk::CommandBuffer> = slots.iter().map(|s| s.cb).collect();
        let value = sh.submit(&cbs, slots, None, None)?;
        self.outstanding.set(false);
        let rects = if self.dmg_known.get() && !self.dmg_full.replace(false) {
            Some(core::mem::take(&mut *self.dmg.borrow_mut()))
        } else {
            self.dmg.borrow_mut().clear();
            None
        };
        *self.fly.borrow_mut() = Some((frame.image, value, rects));
        self.flush()?;
        Ok(SubmitId(value))
    }

    /// rebuild at the next acquire (the window's size is also checked at every acquire)
    pub fn resize(&mut self) {
        self.stale.set(true);
    }

    /// another present mode from the next acquire on
    pub fn set_present_mode(&mut self, m: PresentMode) {
        self.desc.present = m;
        self.mode = present_mode_of(m);
    }

    pub fn present_mode(&self) -> PresentMode {
        self.mode
    }
    pub fn format(&self) -> Format {
        self.format
    }
    /// the images' size (0, 0 while the window has no area)
    pub fn size(&self) -> (u32, u32) {
        self.extent
    }
    pub fn image_count(&self) -> usize {
        self.images.len()
    }
    /// the images' DRM format and modifier (tests, traces)
    pub fn drm_format(&self) -> (u32, u64) {
        (self.fourcc, self.modifier)
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        self.release();
    }
}
