// mtl_swap -- swapchains on Metal (AP6c, docs/APPLE.md): a CAMetalLayer on a plat window
//
//   Surface::new(&instance, window)        macOS: the window's GPU layer, shown (plat::win_native)
//   Surface::native(&instance, nw, window) the UI output's layer (plat::win_gpu_child, hidden until
//                                          the output shows it)
//   device.create_swapchain(&surface, &desc)   the layer gets the device, the format (BGRA8, sRGB
//        when asked), framebufferOnly unless copy_src, two or three drawables, the window's size
//   try_acquire() never waits: nextDrawable only while fewer drawables are in use than the layer
//        has (in use: taken and not yet on the screen, plus the one on the screen; the drawables'
//        presented handlers count what reached it); NotReady otherwise
//   present(frame, cmds)                   the last command buffer presents the drawable
//
// Present modes: Fifo only.  Without displaySyncEnabled a window on macOS still shows at the rate of
// the compositor (measured: 8.3 ms per frame either way at 120 Hz), so Immediate and Mailbox are Fifo
// here, as the API allows for a missing mode.  The layer's colour space is sRGB, as the CGImage of the CPU path (plat's
// apple_mac.rs): both show the same pixels.  Layer properties change inside a Core Animation
// transaction, so a swapchain may live on another thread than the window (the UI's output job).

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicI64, Ordering};

use rt::sync::SpinLock;

use crate::mtl_dev::{Device, Instance};
use crate::mtl_objc::{self as objc, CGSize, Id, Obj, Pool, msg};
use crate::*;

type CfRef = *const c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceSRGB: CfRef;
    fn CGColorSpaceCreateWithName(name: CfRef) -> CfRef;
    fn CGColorSpaceRelease(cs: CfRef);
}

unsafe extern "C" {
    static _NSConcreteGlobalBlock: c_void;
}

#[repr(C)]
struct BlockDesc {
    reserved: usize,
    size: usize,
}

#[repr(C)]
struct Block {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: unsafe extern "C" fn(*const Block, Id),
    desc: *const BlockDesc,
}

// SAFETY: a global block is immutable after it is made; the runtime only reads it.
unsafe impl Sync for Block {}

static BLOCK_DESC: BlockDesc = BlockDesc { reserved: 0, size: core::mem::size_of::<Block>() };

/// how many drawables of each layer reached the screen: (layer, count)
static SHOWN: SpinLock<Vec<(usize, i64)>> = SpinLock::new(Vec::new());

/// the presented handler: one more drawable of its layer reached the screen
unsafe extern "C" fn presented(_b: *const Block, drawable: Id) {
    if drawable.is_null() {
        return;
    }
    // SAFETY: a CAMetalDrawable the system hands to its presented handler; layer is its property.
    let layer = unsafe { msg!(drawable, c"layer"; Id) } as usize;
    let mut s = SHOWN.lock();
    match s.iter_mut().find(|x| x.0 == layer) {
        Some(x) => x.1 += 1,
        None => s.push((layer, 1)),
    }
}

/// the global block of the presented handler (made once, kept for the process)
fn presented_block() -> *const c_void {
    static BLK: SpinLock<usize> = SpinLock::new(0);
    let mut b = BLK.lock();
    if *b == 0 {
        let isa = core::ptr::addr_of!(_NSConcreteGlobalBlock);
        let blk = alloc::boxed::Box::new(Block { isa, flags: 1 << 28, reserved: 0, invoke: presented, desc: &BLOCK_DESC }); // BLOCK_IS_GLOBAL
        *b = alloc::boxed::Box::into_raw(blk) as usize;
    }
    *b as *const c_void
}

fn shown(layer: usize) -> i64 {
    SHOWN.lock().iter().find(|x| x.0 == layer).map_or(0, |x| x.1)
}

/// run f in a Core Animation transaction without animation (any thread)
fn transaction(f: impl FnOnce()) {
    let _pool = Pool::new();
    let tx = objc::class(c"CATransaction");
    // SAFETY: begin / setDisableActions: / commit of the class.
    unsafe {
        msg!(tx, c"begin"; ());
        msg!(tx, c"setDisableActions:", true => bool; ());
    }
    f();
    // SAFETY: the transaction begun above.
    unsafe { msg!(tx, c"commit"; ()) };
}

pub(crate) struct SurfInner {
    pub layer: Obj,
    window: i32,
}

impl Drop for SurfInner {
    fn drop(&mut self) {
        let l = self.layer.id() as usize;
        SHOWN.lock().retain(|x| x.0 != l);
    }
}

/// a window as a target for swapchains
#[derive(Clone)]
pub struct Surface(pub(crate) Arc<SurfInner>);

impl Surface {
    /// (Linux: the window's size for a rebuild on a worker; here plat knows it)
    pub fn track_size(&self) {}

    /// the surface of plat window `window`: its GPU layer, shown
    pub fn new(instance: &Instance, window: i32) -> Result<Surface> {
        #[cfg(target_os = "macos")]
        {
            let nw = plat::win_native(window).ok_or(GpuError::Invalid("no such window"))?;
            Surface::native(instance, nw, window)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (instance, window);
            Err(GpuError::Unsupported("window surfaces on iOS (AP6e)"))
        }
    }

    /// the surface of a window given by its GPU layer (plat::win_gpu_child) whose size and state are
    /// those of plat window `window`
    #[cfg(target_os = "macos")]
    pub fn native(_instance: &Instance, nw: plat::NativeWindow, window: i32) -> Result<Surface> {
        let layer = Obj::retained(nw.layer as Id).ok_or(GpuError::SurfaceLost)?;
        drop(nw);
        Ok(Surface(Arc::new(SurfInner { layer, window })))
    }
}

/// a frame being drawn
pub struct Frame {
    drawable: Option<Obj>,
    size: (u32, u32),
    taken: Arc<AtomicI64>,
}

impl Drop for Frame {
    fn drop(&mut self) {
        if self.drawable.take().is_some() {
            // never presented: it never reaches the screen either
            self.taken.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// what Swapchain::try_acquire found
#[must_use]
pub enum TryAcquire {
    Frame(Frame),
    /// no drawable free yet (try again with the next look)
    NotReady,
    /// the swapchain does not fit the window any more: Swapchain::rebuild first
    Stale,
    /// the window has no area (minimised, hidden or gone)
    NoArea,
}

impl Frame {
    pub fn index(&self) -> u32 {
        0
    }
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
}

/// images shown in a window through its CAMetalLayer
pub struct Swapchain {
    dev: Device,
    surf: Surface,
    format: Format,
    copy_src: bool,
    count: i64,
    size: (u32, u32),
    stale: bool,
    taken: Arc<AtomicI64>,
    /// the texture of the frame acquired last (before the first: a 1 x 1 stand-in)
    current: Texture,
    /// since when try_acquire found every drawable in use (us; 0: not)
    full_since: u64,
}

impl Device {
    pub fn create_swapchain(&self, surface: &Surface, desc: &SwapchainDesc) -> Result<Swapchain> {
        self.sh.check()?;
        let (w, h) = plat::win_size(surface.0.window);
        if w <= 0 || h <= 0 {
            return Err(GpuError::NotReady);
        }
        let format = if desc.srgb { Format::Bgra8Srgb } else { Format::Bgra8Unorm };
        let count = if desc.min_images >= 3 || desc.frames_in_flight >= 3 { 3 } else { 2 };
        let l = surface.0.layer.id();
        let dev = self.sh.dev.id();
        transaction(|| {
            // SAFETY: property setters of a live CAMetalLayer (inside the transaction); the colour
            // space is retained by the layer and released here.
            unsafe {
                msg!(l, c"setDevice:", dev => Id; ());
                msg!(l, c"setPixelFormat:", crate::mtl_res::mtl_format(format) => usize; ());
                msg!(l, c"setFramebufferOnly:", !desc.copy_src => bool; ());
                msg!(l, c"setMaximumDrawableCount:", count as usize => usize; ());
                msg!(l, c"setAllowsNextDrawableTimeout:", true => bool; ());
                msg!(l, c"setDrawableSize:", CGSize { w: w as f64, h: h as f64 } => CGSize; ());
                if msg!(l, c"respondsToSelector:", objc::sel(c"setColorspace:") => objc::Sel; bool) {
                    let cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
                    if !cs.is_null() {
                        msg!(l, c"setColorspace:", cs => CfRef; ());
                        CGColorSpaceRelease(cs);
                    }
                }
            }
        });
        let current = self.create_texture(&TextureDesc::d2(1, 1, format, TextureUsage::COLOR_TARGET))?;
        Ok(Swapchain {
            dev: self.clone(),
            surf: surface.clone(),
            format,
            copy_src: desc.copy_src,
            count,
            size: (w as u32, h as u32),
            stale: false,
            taken: Arc::new(AtomicI64::new(0)),
            current,
            full_since: 0,
        })
    }
}

impl Swapchain {
    fn layer(&self) -> Id {
        self.surf.0.layer.id()
    }

    /// drawables in use: taken and not yet on the screen, plus the one on it
    fn in_use(&self) -> i64 {
        let s = shown(self.layer() as usize);
        (self.taken.load(Ordering::Acquire) - s).max(0) + (s > 0) as i64
    }

    fn full(&self) -> bool {
        self.in_use() >= self.count
    }

    /// the drawables take the window's size now
    pub fn rebuild(&mut self) -> Result<()> {
        self.dev.sh.check()?;
        let (w, h) = plat::win_size(self.surf.0.window);
        if w > 0 && h > 0 {
            let l = self.layer();
            transaction(|| {
                // SAFETY: drawableSize of the live layer, inside the transaction.
                unsafe { msg!(l, c"setDrawableSize:", CGSize { w: w as f64, h: h as f64 } => CGSize; ()) };
            });
            self.size = (w as u32, h as u32);
        }
        self.stale = false;
        Ok(())
    }

    fn area(&self) -> Option<(u32, u32)> {
        let w = self.surf.0.window;
        let (cw, ch) = plat::win_size(w);
        if cw <= 0 || ch <= 0 || plat::win_state(w) & plat::WST_MINIMIZED != 0 {
            return None;
        }
        Some((cw as u32, ch as u32))
    }

    /// the next drawable (+1), blocking at most the layer's timeout (one second)
    fn next(&mut self) -> Result<Option<Frame>> {
        let _pool = Pool::new();
        // SAFETY: nextDrawable of the live layer (autoreleased, retained inside the pool); texture is
        // the drawable's property.
        let (d, tex) = unsafe {
            let d = msg!(self.layer(), c"nextDrawable"; Id);
            let Some(d) = Obj::retained(d) else { return Ok(None) };
            let t: Id = msg!(d.id(), c"texture"; Id);
            let Some(t) = Obj::retained(t) else { return Ok(None) };
            (d, t)
        };
        // SAFETY: width / height of the drawable's texture.
        let (tw, th) = unsafe { (msg!(tex.id(), c"width"; usize) as u32, msg!(tex.id(), c"height"; usize) as u32) };
        let usage = TextureUsage::COLOR_TARGET | if self.copy_src { TextureUsage::COPY_SRC } else { TextureUsage::NONE };
        let desc = TextureDesc::d2(tw, th, self.format, usage);
        self.current = crate::mtl_res::wrap_texture(&self.dev.sh, tex, desc);
        self.taken.fetch_add(1, Ordering::AcqRel);
        Ok(Some(Frame { drawable: Some(d), size: (tw, th), taken: self.taken.clone() }))
    }

    /// the image to draw into, never waiting
    pub fn try_acquire(&mut self) -> Result<TryAcquire> {
        self.dev.sh.check()?;
        let Some(size) = self.area() else { return Ok(TryAcquire::NoArea) };
        if self.stale || size != self.size {
            return Ok(TryAcquire::Stale);
        }
        if self.full() {
            let now = plat::time_us();
            if self.full_since == 0 {
                self.full_since = now;
            }
            // (presented handlers that never come must not stop the window for good)
            if now - self.full_since < 1_000_000 {
                return Ok(TryAcquire::NotReady);
            }
        }
        self.full_since = 0;
        match self.next()? {
            Some(f) => Ok(TryAcquire::Frame(f)),
            None => Ok(TryAcquire::NotReady),
        }
    }

    /// the image to draw into (waits for a drawable); None while the window has no area
    pub fn acquire(&mut self) -> Result<Option<Frame>> {
        self.dev.sh.check()?;
        let Some(size) = self.area() else { return Ok(None) };
        if self.stale || size != self.size {
            self.rebuild()?;
        }
        self.next()
    }

    /// the frame's image as a render target
    pub fn view(&self, _frame: &Frame) -> &TextureView {
        self.current.view()
    }

    /// the frame's image (copies: SwapchainDesc::copy_src)
    pub fn texture(&self, _frame: &Frame) -> &Texture {
        &self.current
    }

    /// submit the frame's commands; the last command buffer presents its drawable
    pub fn present<'a, I: IntoIterator<Item = CommandBuffer<'a>>>(&self, mut frame: Frame, cmds: I) -> Result<SubmitId> {
        let Some(d) = frame.drawable.take() else { return Err(GpuError::Invalid("present: a frame presented twice")) };
        // SAFETY: addPresentedHandler: (macOS 10.15.4, iOS 10.3) with a global block.
        unsafe { msg!(d.id(), c"addPresentedHandler:", presented_block() => *const c_void; ()) };
        let cbs: Vec<Obj> = cmds.into_iter().map(|c| c.into_obj()).collect();
        self.dev.sh.submit_present(cbs, Some(d)).map(SubmitId)
    }

    /// (Linux: the display has answered for the images; here nothing to wait for)
    pub fn prepare(&mut self) -> Result<bool> {
        Ok(true)
    }
    /// (Linux: a frame reaches the window after the GPU finished it; Metal does that)
    pub fn flush(&self) -> Result<bool> {
        Ok(true)
    }
    /// a frame is on its way to the screen
    pub fn busy(&self) -> bool {
        self.full()
    }
    /// when a frame that found NotReady should try again
    pub fn next_look(&self, retry: u64) -> u64 {
        retry
    }
    /// (Linux: what the next image changes, for the compositor; here the whole image counts)
    pub fn damage(&self, _rects: Option<&[[i32; 4]]>) {}
    /// the next acquire takes the window's size
    pub fn resize(&mut self) {
        self.stale = true;
    }
    /// (Fifo only: see the top)
    pub fn set_present_mode(&mut self, _m: PresentMode) {}
    pub fn present_mode(&self) -> PresentMode {
        PresentMode::Fifo
    }
    pub fn format(&self) -> Format {
        self.format
    }
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
    pub fn image_count(&self) -> usize {
        self.count as usize
    }
}
