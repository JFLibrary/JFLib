// vk_swap -- window surfaces and swapchains
//
//   Surface::new(&instance, window) -> Result<Surface>   a plat window (Windows: its HWND and
//        HINSTANCE through plat::win_native, VK_KHR_win32_surface); other targets: Unsupported
//   device.create_swapchain(&surface, &SwapchainDesc) -> Result<Swapchain>
//        B8G8R8A8 (or R8G8B8A8) UNORM / SRGB, sRGB non-linear colour space; the present mode of
//        the desc or jfgui_GPU_PRESENT=fifo|mailbox|immediate, FIFO when the surface lacks it;
//        image count: the surface's minimum + 1; copy_src adds TRANSFER_SRC (read-back)
//   sc.acquire() -> Result<Option<Frame>>    None while the window is minimised or has no area;
//        rebuilds the swapchain first when the window's size changed, the last present said
//        out-of-date / suboptimal, resize() or set_present_mode() asked for it, or the last frame
//        was never presented; waits until the frame slot's previous submission is done
//        (frames_in_flight)
//   sc.view(&frame) / texture(&frame)        the image: Layout::Undefined at the start of the
//        frame's commands, Layout::Present at their end
//   sc.present(frame, [cmd, ..]) -> Result<SubmitId>   submit (waiting for the image at the
//        colour-output, copy and compute stages, signalling the image's render-done semaphore),
//        then vkQueuePresentKHR
//   sc.size(), format(), present_mode(), resize(), set_present_mode(m)
//
// Rebuilding (and dropping) waits for the queue to be idle -- vkQueueWaitIdle covers the
// presents too, the timeline does not see a present's semaphore wait -- then creates the new
// swapchain with the old one as oldSwapchain and destroys the old one with its views and
// semaphores (resizes are rare; a frame never waits for this).  Semaphores: one acquire
// semaphore per frame slot (reused once the slot's submission completed), one render-done
// semaphore per image (the present that waited on it has finished when the image is acquired
// again).
// The window must not be presented by the CPU (plat::win_present) at the same time: a plat
// window that never presents keeps nothing of its own to paint (win_api.rs WM_PAINT).

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::Cell;

use crate::vk_dev::{Device, Garbage, Shared};
use crate::vk_ffi as vk;
use crate::vk_inst::InstShared;
use crate::*;

pub(crate) struct SurfShared {
    inst: Arc<InstShared>,
    pub h: vk::Handle,
    pub window: i32,
}

impl Drop for SurfShared {
    fn drop(&mut self) {
        if let Some(d) = self.inst.f.destroy_surface_khr {
            // SAFETY: the swapchains of the surface hold it, so none is left.
            unsafe { d(self.inst.inst, self.h, core::ptr::null()) };
        }
    }
}

/// a window as a target for swapchains
#[derive(Clone)]
pub struct Surface(pub(crate) Arc<SurfShared>);

impl Surface {
    /// the surface of plat window `window` (an index of plat::win_create)
    pub fn new(instance: &Instance, window: i32) -> Result<Surface> {
        let sh = &instance.sh;
        if !sh.surface {
            return Err(GpuError::Unsupported("window surfaces"));
        }
        #[cfg(windows)]
        {
            let nw = plat::win_native(window).ok_or(GpuError::Invalid("no such window"))?;
            let create = sh.f.create_win32_surface_khr.ok_or(GpuError::Unsupported("VK_KHR_win32_surface"))?;
            let ci = vk::Win32SurfaceCreateInfoKHR {
                s_type: vk::STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR,
                hinstance: nw.hinstance,
                hwnd: nw.hwnd,
                ..vk::Zero::zero()
            };
            let mut h = 0;
            // SAFETY: a live window of this process and its module.
            let r = unsafe { create(sh.inst, &ci, core::ptr::null(), &mut h) };
            if r != vk::SUCCESS {
                return Err(crate::vk_inst::err(r));
            }
            Ok(Surface(Arc::new(SurfShared { inst: sh.clone(), h, window })))
        }
        #[cfg(not(windows))]
        {
            let _ = window;
            Err(GpuError::Unsupported("window surfaces on this target"))
        }
    }
}

/// an acquired swapchain image (present it with Swapchain::present)
#[must_use]
pub struct Frame {
    image: u32,
    slot: u32,
    generation: u32,
    size: (u32, u32),
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

/// the images a window shows, presented in turn
pub struct Swapchain {
    sh: Arc<Shared>,
    surf: Surface,
    desc: SwapchainDesc,
    h: vk::Handle,
    format: Format,
    vk_format: u32,
    color_space: u32,
    mode: PresentMode,
    extent: (u32, u32),
    images: Vec<Texture>,
    acquire: Vec<vk::Handle>,
    done: Vec<vk::Handle>,
    slot_value: Vec<Cell<u64>>,
    slot: Cell<u32>,
    generation: u32,
    stale: Cell<bool>,
    outstanding: Cell<bool>,
}

fn present_mode_vk(m: PresentMode) -> u32 {
    match m {
        PresentMode::Fifo => vk::PRESENT_MODE_FIFO_KHR,
        PresentMode::Mailbox => vk::PRESENT_MODE_MAILBOX_KHR,
        PresentMode::Immediate => vk::PRESENT_MODE_IMMEDIATE_KHR,
    }
}

impl Device {
    pub fn create_swapchain(&self, surface: &Surface, desc: &SwapchainDesc) -> Result<Swapchain> {
        let sh = &self.sh;
        sh.check()?;
        if !sh.swapchain {
            return Err(GpuError::Unsupported("swapchains (VK_KHR_swapchain)"));
        }
        let f = &sh.inst.f;
        let (Some(support), Some(formats)) = (f.get_physical_device_surface_support_khr, f.get_physical_device_surface_formats_khr)
        else {
            return Err(GpuError::Unsupported("surface queries"));
        };
        let mut ok = 0;
        // SAFETY: plain query.
        sh.vk(unsafe { support(sh.pd, sh.family, surface.0.h, &mut ok) })?;
        if ok == 0 {
            return Err(GpuError::Unsupported("presentation from this queue"));
        }
        let mut n = 0u32;
        // SAFETY: the two-call enumeration.
        sh.vk(unsafe { formats(sh.pd, surface.0.h, &mut n, core::ptr::null_mut()) })?;
        let mut fl: Vec<vk::SurfaceFormatKHR> = vec![vk::Zero::zero(); n as usize];
        // SAFETY: fl holds n entries.
        let r = unsafe { formats(sh.pd, surface.0.h, &mut n, fl.as_mut_ptr()) };
        if r != vk::INCOMPLETE {
            sh.vk(r)?;
        }
        fl.truncate(n as usize);
        let want: [(u32, Format); 2] = if desc.srgb {
            [(vk::FORMAT_B8G8R8A8_SRGB, Format::Bgra8Srgb), (vk::FORMAT_R8G8B8A8_SRGB, Format::Rgba8Srgb)]
        } else {
            [(vk::FORMAT_B8G8R8A8_UNORM, Format::Bgra8Unorm), (vk::FORMAT_R8G8B8A8_UNORM, Format::Rgba8Unorm)]
        };
        let pick = want.iter().find_map(|&(vf, ff)| {
            fl.iter().find(|s| s.format == vf && s.color_space == vk::COLOR_SPACE_SRGB_NONLINEAR_KHR).map(|s| (vf, ff, s.color_space))
        });
        let Some((vk_format, format, color_space)) = pick else {
            return Err(GpuError::Unsupported("an 8-bit RGBA / BGRA surface format"));
        };
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
            h: 0,
            format,
            vk_format,
            color_space,
            mode: PresentMode::Fifo,
            extent: (0, 0),
            images: Vec::new(),
            acquire: Vec::new(),
            done: Vec::new(),
            slot_value: (0..d.frames_in_flight).map(|_| Cell::new(0)).collect(),
            slot: Cell::new(0),
            generation: 0,
            stale: Cell::new(false),
            outstanding: Cell::new(false),
        };
        sc.build()?;
        Ok(sc)
    }
}

impl Swapchain {
    fn semaphore(&self) -> Result<vk::Handle> {
        let sh = &self.sh;
        let si = vk::SemaphoreCreateInfo { s_type: vk::STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO, ..vk::Zero::zero() };
        let mut s = 0;
        // SAFETY: valid create info.
        sh.vk(unsafe { (sh.f.create_semaphore)(sh.dev, &si, core::ptr::null(), &mut s) })?;
        Ok(s)
    }

    /// drop the images and semaphores (the device is idle or nothing was submitted for them)
    fn release(&mut self) {
        self.images.clear();
        for s in self.acquire.drain(..).chain(self.done.drain(..)) {
            self.sh.defer(Garbage::Semaphore(s));
        }
    }

    /// (re)create the swapchain for the window's current size (extent 0: no swapchain)
    fn build(&mut self) -> Result<()> {
        let sh = self.sh.clone();
        let inst = &sh.inst.f;
        let (Some(caps_fn), Some(modes_fn)) =
            (inst.get_physical_device_surface_capabilities_khr, inst.get_physical_device_surface_present_modes_khr)
        else {
            return Err(GpuError::Unsupported("surface queries"));
        };
        let (Some(create), Some(images_fn)) = (sh.f.create_swapchain_khr, sh.f.get_swapchain_images_khr) else {
            return Err(GpuError::Unsupported("VK_KHR_swapchain"));
        };
        let surf = self.surf.0.h;
        let mut caps: vk::SurfaceCapabilitiesKHR = vk::Zero::zero();
        // SAFETY: plain query.
        let r = unsafe { caps_fn(sh.pd, surf, &mut caps) };
        sh.vk(r)?;
        let (ww, wh) = window_size(self.surf.0.window);
        let mut ext = caps.current_extent;
        if ext.width == u32::MAX {
            ext = vk::Extent2D {
                width: ww.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                height: wh.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
            };
        }
        self.stale.set(false);
        self.outstanding.set(false);
        let old = self.h;
        if ext.width == 0 || ext.height == 0 {
            // minimised / no area: keep nothing, acquire returns None until there is an area
            self.release();
            if old != 0 {
                sh.defer(Garbage::Swapchain(old));
            }
            self.h = 0;
            self.extent = (0, 0);
            return Ok(());
        }
        // the present mode: the wanted one when the surface has it, else FIFO
        let mut n = 0u32;
        // SAFETY: the two-call enumeration.
        sh.vk(unsafe { modes_fn(sh.pd, surf, &mut n, core::ptr::null_mut()) })?;
        let mut modes = vec![0u32; n as usize];
        // SAFETY: modes holds n entries.
        let r = unsafe { modes_fn(sh.pd, surf, &mut n, modes.as_mut_ptr()) };
        if r != vk::INCOMPLETE {
            sh.vk(r)?;
        }
        modes.truncate(n as usize);
        self.mode = if modes.contains(&present_mode_vk(self.desc.present)) { self.desc.present } else { PresentMode::Fifo };
        let mut count = caps.min_image_count + 1;
        if caps.max_image_count != 0 {
            count = count.min(caps.max_image_count);
        }
        let mut usage = vk::IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
        if caps.supported_usage_flags & vk::IMAGE_USAGE_TRANSFER_DST_BIT != 0 {
            usage |= vk::IMAGE_USAGE_TRANSFER_DST_BIT;
        }
        let copy_src = self.desc.copy_src && caps.supported_usage_flags & vk::IMAGE_USAGE_TRANSFER_SRC_BIT != 0;
        if copy_src {
            usage |= vk::IMAGE_USAGE_TRANSFER_SRC_BIT;
        }
        let alpha = if caps.supported_composite_alpha & vk::COMPOSITE_ALPHA_OPAQUE_BIT_KHR != 0 {
            vk::COMPOSITE_ALPHA_OPAQUE_BIT_KHR
        } else if caps.supported_composite_alpha & vk::COMPOSITE_ALPHA_INHERIT_BIT_KHR != 0 {
            vk::COMPOSITE_ALPHA_INHERIT_BIT_KHR
        } else {
            1 << caps.supported_composite_alpha.trailing_zeros().min(31)
        };
        let ci = vk::SwapchainCreateInfoKHR {
            s_type: vk::STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR,
            surface: surf,
            min_image_count: count,
            image_format: self.vk_format,
            image_color_space: self.color_space,
            image_extent: ext,
            image_array_layers: 1,
            image_usage: usage,
            image_sharing_mode: vk::SHARING_MODE_EXCLUSIVE,
            pre_transform: caps.current_transform,
            composite_alpha: alpha,
            present_mode: present_mode_vk(self.mode),
            clipped: 1,
            old_swapchain: old,
            ..vk::Zero::zero()
        };
        let mut h = 0;
        // SAFETY: valid create info; old (if any) is retired by this call.
        let r = unsafe { create(sh.dev, &ci, core::ptr::null(), &mut h) };
        self.release();
        if old != 0 {
            sh.defer(Garbage::Swapchain(old));
        }
        self.h = 0;
        sh.vk(r)?;
        self.h = h;
        self.extent = (ext.width, ext.height);
        self.generation = self.generation.wrapping_add(1);
        let mut n = 0u32;
        // SAFETY: the two-call enumeration.
        sh.vk(unsafe { images_fn(sh.dev, h, &mut n, core::ptr::null_mut()) })?;
        let mut imgs = vec![0u64; n as usize];
        // SAFETY: imgs holds n handles.
        let r = unsafe { images_fn(sh.dev, h, &mut n, imgs.as_mut_ptr()) };
        if r != vk::INCOMPLETE {
            sh.vk(r)?;
        }
        imgs.truncate(n as usize);
        let mut tu = TextureUsage::COLOR_TARGET | TextureUsage::COPY_DST;
        if copy_src {
            tu = tu | TextureUsage::COPY_SRC;
        }
        let td = TextureDesc::d2(ext.width, ext.height, self.format, tu);
        for &img in &imgs {
            self.images.push(crate::vk_res::wrap_image(&sh, img, td)?);
        }
        for _ in 0..self.desc.frames_in_flight {
            let s = self.semaphore()?;
            self.acquire.push(s);
        }
        for _ in 0..imgs.len() {
            let s = self.semaphore()?;
            self.done.push(s);
        }
        for v in &self.slot_value {
            v.set(0);
        }
        Ok(())
    }

    fn rebuild(&mut self) -> Result<()> {
        // the old images may still be in use: wait (resizes are rare)
        self.sh.queue_idle()?;
        self.build()
    }

    /// the next image to draw into; None while the window is minimised or has no area
    pub fn acquire(&mut self) -> Result<Option<Frame>> {
        self.sh.check()?;
        let Some(acq) = self.sh.f.acquire_next_image_khr else {
            return Err(GpuError::Unsupported("VK_KHR_swapchain"));
        };
        if window_minimised(self.surf.0.window) {
            return Ok(None);
        }
        if self.outstanding.get() {
            self.stale.set(true); // the last frame was never presented
        }
        if window_size(self.surf.0.window) != self.extent {
            self.stale.set(true);
        }
        if self.stale.get() || self.h == 0 {
            self.rebuild()?;
        }
        for attempt in 0..2 {
            if self.h == 0 {
                return Ok(None);
            }
            let k = self.slot.get();
            let prev = self.slot_value[k as usize].get();
            if prev != 0 && !self.sh.wait_value(prev, 10_000_000_000)? {
                return Err(GpuError::Timeout);
            }
            let mut image = 0u32;
            // SAFETY: the slot's semaphore is unsignalled (its last wait completed); no fence.
            let r = unsafe { acq(self.sh.dev, self.h, 10_000_000_000, self.acquire[k as usize], 0, &mut image) };
            match r {
                vk::SUCCESS | vk::SUBOPTIMAL_KHR => {
                    if r == vk::SUBOPTIMAL_KHR {
                        self.stale.set(true);
                    }
                    self.outstanding.set(true);
                    return Ok(Some(Frame { image, slot: k, generation: self.generation, size: self.extent }));
                }
                vk::ERROR_OUT_OF_DATE_KHR if attempt == 0 => self.rebuild()?,
                vk::ERROR_OUT_OF_DATE_KHR => return Ok(None),
                vk::TIMEOUT | vk::NOT_READY => return Err(GpuError::Timeout),
                _ => {
                    self.sh.vk(r)?;
                }
            }
        }
        Ok(None)
    }

    /// the frame's image as a render target (Undefined at the start, Present at the end)
    pub fn view(&self, frame: &Frame) -> &TextureView {
        self.images[frame.image as usize].view()
    }

    /// the frame's image (copies: SwapchainDesc::copy_src)
    pub fn texture(&self, frame: &Frame) -> &Texture {
        &self.images[frame.image as usize]
    }

    /// submit the frame's commands (they leave the image in Layout::Present) and show it
    pub fn present<'a, I: IntoIterator<Item = CommandBuffer<'a>>>(&self, frame: Frame, cmds: I) -> Result<SubmitId> {
        let sh = &self.sh;
        let mut cbs = Vec::new();
        let mut slots = Vec::new();
        for c in cmds {
            let s = c.into_slot();
            cbs.push(s.cb);
            slots.push(s);
        }
        if frame.generation != self.generation || !self.outstanding.get() || self.h == 0 {
            for s in slots {
                sh.give_slot(s);
            }
            return Err(GpuError::Invalid("present: a frame of an older swapchain"));
        }
        let k = frame.slot as usize;
        let wait = vk::PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT | vk::PIPELINE_STAGE_TRANSFER_BIT | vk::PIPELINE_STAGE_COMPUTE_SHADER_BIT;
        let done = self.done[frame.image as usize];
        let value = sh.submit(&cbs, slots, Some((self.acquire[k], wait)), Some(done))?;
        self.slot_value[k].set(value);
        self.slot.set((frame.slot + 1) % self.desc.frames_in_flight);
        self.outstanding.set(false);
        let Some(qp) = sh.f.queue_present_khr else {
            return Err(GpuError::Unsupported("VK_KHR_swapchain"));
        };
        let pi = vk::PresentInfoKHR {
            s_type: vk::STRUCTURE_TYPE_PRESENT_INFO_KHR,
            wait_semaphore_count: 1,
            p_wait_semaphores: &done,
            swapchain_count: 1,
            p_swapchains: &self.h,
            p_image_indices: &frame.image,
            ..vk::Zero::zero()
        };
        // SAFETY: the image was acquired from this swapchain; the queue is held.
        let r = sh.with_queue(|q| unsafe { qp(q, &pi) });
        match r {
            vk::SUCCESS => {}
            vk::SUBOPTIMAL_KHR | vk::ERROR_OUT_OF_DATE_KHR => self.stale.set(true),
            _ => sh.vk(r)?,
        }
        Ok(SubmitId(value))
    }

    /// rebuild at the next acquire (the window's size is also checked at every acquire)
    pub fn resize(&mut self) {
        self.stale.set(true);
    }

    /// another present mode from the next acquire on (FIFO when the surface lacks it)
    pub fn set_present_mode(&mut self, m: PresentMode) {
        self.desc.present = m;
        self.stale.set(true);
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
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        let _ = self.sh.queue_idle(); // presents included: their semaphores and images are free
        self.release();
        if self.h != 0 {
            self.sh.defer(Garbage::Swapchain(self.h));
        }
    }
}

#[cfg(windows)]
fn window_size(w: i32) -> (u32, u32) {
    let (x, y) = plat::win_size(w);
    (x.max(0) as u32, y.max(0) as u32)
}

#[cfg(windows)]
fn window_minimised(w: i32) -> bool {
    plat::win_state(w) & plat::WST_MINIMIZED != 0
}

#[cfg(not(windows))]
fn window_size(_w: i32) -> (u32, u32) {
    (0, 0)
}

#[cfg(not(windows))]
fn window_minimised(_w: i32) -> bool {
    true
}
