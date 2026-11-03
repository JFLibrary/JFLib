// web_swap -- a canvas as a swapchain in the browser (through web/gpu.js)
//
//   Surface::native(&instance, nw, window)   the OffscreenCanvas plat::win_gpu_child brought from the
//        page (lib/plat/web_gpu.rs); Surface::new asks for it, NotReady until it is here
//   device.create_swapchain(&surface, &SwapchainDesc)   getContext("webgpu"), configure: the format the
//        browser prefers (bgra8unorm or rgba8unorm), opaque, sRGB; copy_src adds COPY_SRC; srgb a view in
//        the -srgb format.  The present mode and the image count are not ours: the browser composes the
//        newest image at its next frame, present_mode() says Mailbox
//   sc.try_acquire()   getCurrentTexture.  NotReady never: the image is there at once, and a second
//        frame in one turn of the worker's event loop draws over the first, which nobody saw
//   sc.present(frame, [cmd, ..])   queue.submit.  There is no present: the browser takes the image when
//        the worker returns to its event loop (OffscreenCanvas of a worker: no main thread involved)
//
// A swapchain cannot be read back into this turn: Buffer::read later (web_res.rs).

use alloc::rc::Rc;
use core::cell::Cell;

use crate::web_dev::{call, Device, Handle, CALL_CONTEXT, CALL_CURRENT, CALL_RESERVE, CALL_RESIZE, CALL_RGBA};
use crate::*;

pub(crate) struct SurfInner {
    canvas: Handle,
    window: i32,
}

/// a canvas as a target for swapchains
#[derive(Clone)]
pub struct Surface(pub(crate) Rc<SurfInner>);

impl Surface {
    /// (Linux: the window's size for a rebuild on a worker; the browser knows it)
    pub fn track_size(&self) {}

    /// the canvas of plat window `window` (NotReady: it is on its way from the page)
    pub fn new(instance: &Instance, window: i32) -> Result<Surface> {
        let nw = plat::win_gpu_child(window).ok_or(GpuError::NotReady)?;
        Surface::native(instance, nw, window)
    }

    /// the canvas of plat::win_gpu_child, whose size is that of plat window `window`
    pub fn native(_instance: &Instance, nw: plat::NativeWindow, window: i32) -> Result<Surface> {
        Ok(Surface(Rc::new(SurfInner { canvas: Handle { h: nw.take(), destroy: false }, window })))
    }
}

/// a frame being drawn
pub struct Frame {
    size: (u32, u32),
}

/// what Swapchain::try_acquire found
#[must_use]
pub enum TryAcquire {
    Frame(Frame),
    /// (never in the browser)
    NotReady,
    /// the swapchain does not fit the window any more: Swapchain::rebuild first
    Stale,
    /// the window has no area
    NoArea,
}

impl Frame {
    /// (the browser has one current image)
    pub fn index(&self) -> u32 {
        0
    }
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
}

/// images shown on a canvas
pub struct Swapchain {
    dev: Device,
    surf: Surface,
    ctx: Handle,
    size: Cell<(u32, u32)>,
    format: Format,
    texture: Texture,
    view: TextureView,
    stale: Cell<bool>,
}

impl Device {
    pub fn create_swapchain(&self, surface: &Surface, desc: &SwapchainDesc) -> Result<Swapchain> {
        self.d.check()?;
        let (w, h) = plat::win_size(surface.0.window);
        let (w, h) = (w.max(0) as u32, h.max(0) as u32);
        if w == 0 || h == 0 {
            return Err(GpuError::NotReady);
        }
        let ctx = call(CALL_CONTEXT, &[self.d.h, surface.0.canvas.h, w, h, desc.copy_src as u32, desc.srgb as u32]);
        if ctx == 0 {
            return Err(GpuError::SurfaceLost);
        }
        let rgba = call(CALL_RGBA, &[]) != 0;
        let format = match (rgba, desc.srgb) {
            (false, false) => Format::Bgra8Unorm,
            (false, true) => Format::Bgra8Srgb,
            (true, false) => Format::Rgba8Unorm,
            (true, true) => Format::Rgba8Srgb,
        };
        let usage = TextureUsage::COLOR_TARGET | if desc.copy_src { TextureUsage::COPY_SRC } else { TextureUsage::NONE };
        let view = TextureView::for_frame(call(CALL_RESERVE, &[]), format, w, h);
        let texture = Texture::for_frame(call(CALL_RESERVE, &[]), view.clone(), format, w, h, usage);
        Ok(Swapchain { dev: self.clone(), surf: surface.clone(), ctx: Handle { h: ctx, destroy: false }, size: Cell::new((w, h)), format, texture, view, stale: Cell::new(false) })
    }
}

impl Swapchain {
    /// the canvas takes the window's size now
    pub fn rebuild(&mut self) -> Result<()> {
        let (w, h) = plat::win_size(self.surf.0.window);
        let (w, h) = (w.max(0) as u32, h.max(0) as u32);
        if w != 0 && h != 0 {
            call(CALL_RESIZE, &[self.ctx.h, w, h]);
            let usage = self.texture.usage();
            let view = TextureView::for_frame(call(CALL_RESERVE, &[]), self.format, w, h);
            self.texture = Texture::for_frame(call(CALL_RESERVE, &[]), view.clone(), self.format, w, h, usage);
            self.view = view;
        }
        self.size.set((w, h));
        self.stale.set(false);
        Ok(())
    }

    /// the image to draw into (the same one until the worker returns to its event loop)
    pub fn try_acquire(&mut self) -> Result<TryAcquire> {
        self.dev.d.check()?;
        let (w, h) = plat::win_size(self.surf.0.window);
        if w <= 0 || h <= 0 {
            return Ok(TryAcquire::NoArea);
        }
        if self.stale.get() || self.size.get() != (w as u32, h as u32) {
            return Ok(TryAcquire::Stale);
        }
        match call(CALL_CURRENT, &[self.ctx.h, self.texture.handle(), self.view.handle()]) {
            0 => Err(GpuError::SurfaceLost),
            _ => Ok(TryAcquire::Frame(Frame { size: self.size.get() })),
        }
    }

    /// the image to draw into; None while the window has no area
    pub fn acquire(&mut self) -> Result<Option<Frame>> {
        match self.try_acquire()? {
            TryAcquire::Frame(f) => Ok(Some(f)),
            TryAcquire::Stale => {
                self.rebuild()?;
                match self.try_acquire()? {
                    TryAcquire::Frame(f) => Ok(Some(f)),
                    _ => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    /// the frame's image as a render target
    pub fn view(&self, _frame: &Frame) -> &TextureView {
        &self.view
    }

    /// the frame's image (copies: SwapchainDesc::copy_src)
    pub fn texture(&self, _frame: &Frame) -> &Texture {
        &self.texture
    }

    /// submit the frame's commands; the browser shows the image when the worker's turn ends
    pub fn present<'a, I: IntoIterator<Item = CommandBuffer<'a>>>(&self, _frame: Frame, cmds: I) -> Result<SubmitId> {
        self.dev.queue().submit(cmds)
    }

    /// (Linux: the display has answered for the images; here nothing to wait for)
    pub fn prepare(&mut self) -> Result<bool> {
        Ok(true)
    }
    /// (Linux: a frame reaches the window after the GPU finished it; here the browser does that)
    pub fn flush(&self) -> Result<bool> {
        Ok(true)
    }
    /// (Linux: a frame is on its way to the window; here never)
    pub fn busy(&self) -> bool {
        false
    }
    /// when a frame that found NotReady should try again: retry (the caller's time)
    pub fn next_look(&self, retry: u64) -> u64 {
        retry
    }
    /// (Linux: what the next image changes, for the compositor; here the whole image counts)
    pub fn damage(&self, _rects: Option<&[[i32; 4]]>) {}
    /// the next acquire takes the window's size
    pub fn resize(&mut self) {
        self.stale.set(true);
    }
    /// (the browser's: no present modes)
    pub fn set_present_mode(&mut self, _m: PresentMode) {}
    pub fn present_mode(&self) -> PresentMode {
        PresentMode::Mailbox
    }
    pub fn format(&self) -> Format {
        self.format
    }
    /// the images' size
    pub fn size(&self) -> (u32, u32) {
        self.size.get()
    }
    pub fn image_count(&self) -> usize {
        1
    }
}
