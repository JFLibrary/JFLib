// gpu_view -- GPU content in a window: a 3D view between widgets (G5e1; docs/GPU.md "3D-Ansicht")
//
// The framebuffer stays the truth: a view is drawn into it like any canvas, its command's Draw copies the view's
// last frame there.  What is new is where that frame comes from: the app's content (GpuContent; a render3d Renderer
// and Scene, or anything else on lib/gpu: lib/ui knows nothing of render3d) draws it on the UI's own device into a
// texture of the view's size, and the frame is read back from there.
//
//   ui.gpu_view(id, &content, ver) -> &UiInput   the view in the layout's next rectangle (next_size), its input
//                                                as ui.canvas's; a new ver (or size, or device) draws a frame
//   GpuContent::attach(dev) / detach()           a device for the view: the first one, a new one after a loss /
//                                                it goes; build and drop what the frames need there
//   GpuContent::draw(e, target, size)            a frame into target (GPU_VIEW_FORMAT, the view's size)
//   GpuContent::submitted(id)                    the frame went to the queue
//   GpuContent::paint(g, r)                      what the view shows without a device
//   ui.gpu_view_info(win, id)                    what a view does now (tests, measurements)
//   AF_GPU_3D                                    the app shows views: the UI's device at level Voll on the
//                                                fastest adapter (without it, as ever: Basis, the frugal one)
//
// Where a frame shows (gpu_out.rs "views"):
//   - the window on its swapchain and the view on top in its cells: the output's render pass draws the texture
//     there (a quad, as an image's), the framebuffer lacks those pixels (Split::owed) until somebody reads it:
//     gp_settle runs the Draw, which reads the texture back.  At rest (REST after the last frame) gp_tick does
//     that once, so the framebuffer holds the picture again without anybody asking.  A layer over the view gets
//     the frame below it the same way; cells with something of the base over the view are drawn by the CPU over
//     the frame read back.
//   - anywhere else (the CPU path: before the switch, a new size, minimised, the inspector, right to left,
//     jfgui_GPU_OUTPUT=0, jfgui_GPU_VIEWS=0, a script without `gpu on`): the frame is read back at once and the
//     Draw copies it into the framebuffer, in the cells the CPU draws.
//   - no device (jfgui_GPU=0, no Vulkan or no usable adapter, a static Linux program, a lost device not back
//     yet): the last frame read back, else GpuContent::paint.
// The device is the UI output's (one for all windows); without an output in this run (not wanted, never
// possible) the views make one of their own on first use.  A new device (generation) detaches the content from
// the old one and attaches it to the new one; its next frame follows.
//
// The browser cannot wait: a read-back there is answered later, the Draw shows the last frame that came back
// (G5e2 tests it).

use alloc::rc::{Rc, Weak};
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use gfx::Surface;

use crate::canvas::ViewRec;
use crate::cmd::{Cmd, Draw, DrawRef, Gfx};
use crate::{Ui, UiInput};

#[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
use gpu::{Access, Buffer, DataLayout, Device, GpuError, Layout, Texture, TextureRegion};

/// the format of a view's target: what the output samples and what is read back byte for byte into the
/// framebuffer (B, G, R, x).  The content's colours go in as it writes them, no sRGB encoding
pub const GPU_VIEW_FORMAT: gpu::Format = gpu::Format::Bgra8Unorm;

/// what a view (ui.gpu_view) shows: content drawn on the UI's GPU device.  The app keeps it in an
/// `Rc<RefCell<..>>` and hands the same one every frame; lib/ui keeps a Weak and calls these at render time
pub trait GpuContent {
    /// a device for the view: the first, or a new one after the last was lost (what was built on an earlier one is
    /// dead).  Build what the frames need on it.  Err: the view paints until the next device
    fn attach(&mut self, dev: &gpu::Device) -> gpu::Result<()>;
    /// the device goes (lost, the output ended, another content took the view's id): drop what was built on it
    fn detach(&mut self);
    /// one frame: record into e a render pass into target (GPU_VIEW_FORMAT, size w x h px).  Keep its view, before
    /// and after (the pass may set its own load and clear: `ColorAttachment { clear: .., ..target }`); passes that
    /// split it (render3d's occlusion test) begin with its before and end with its after.  lib/ui submits e
    fn draw<'a>(&'a mut self, e: &mut gpu::CommandEncoder<'a>, target: gpu::ColorAttachment<'a>, size: (u32, u32)) -> gpu::Result<()>;
    /// the frame went to the queue as id (render3d: Scene::submitted)
    fn submitted(&mut self, id: gpu::SubmitId) {
        let _ = id;
    }
    /// what the view shows without a device (no GPU, jfgui_GPU=0, before the first frame): painted on the CPU into
    /// g, clipped to r (x, y, w, h); a dark grey unless the content says otherwise
    fn paint(&self, g: &mut Gfx, r: [i32; 4]) {
        paint_default(g, r);
    }
}

/// what a view does now (Ui::gpu_view_info; tests and measurements)
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct GpuViewInfo {
    /// its content is attached to a device: its frames come from the GPU
    pub device: bool,
    /// that device is the UI output's (else the views' own: no output in this run)
    pub output: bool,
    /// the device's level
    pub level: Option<gpu::Level>,
    /// the device's generation (a new one after a lost device; 0: none)
    pub generation: u32,
    /// the output's render pass drew it in the window's last frame (else the CPU copies the frame read back)
    pub apart: bool,
    /// the content attached and detached so far
    pub attaches: u32,
    pub detaches: u32,
    /// frames drawn, the CPU time recording and submitting them took (us)
    pub frames: u64,
    pub draw_us: u64,
    /// frames read back into the CPU's copy, the time that took (us: submit, wait, copy)
    pub reads: u64,
    pub read_us: u64,
    /// the size of its target (px)
    pub size: (u32, u32),
    /// what the last failure said
    pub error: Option<String>,
}

/// the dark grey of a view without a device
pub(crate) fn paint_default(g: &mut Gfx, r: [i32; 4]) {
    g.dc.set_color(0xFF30_3438);
    g.dc.fill(g.sf, r[0], r[1], r[2], r[3]);
}

/// a view's target on its device (gpu_out.rs makes it; the Draw reads it back)
#[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
pub(crate) struct Target {
    pub(crate) dev: Device,
    pub(crate) t: Texture,
    /// the read-back's buffer, rows of `stride` bytes (WebGPU wants multiples of 256; Vulkan takes them too).  The
    /// browser: a buffer per read-back (a mapped one may not go into a submit again), on its way with its frame
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) rb: Buffer,
    pub(crate) stride: u32,
    #[cfg(target_arch = "wasm32")]
    pub(crate) wait: Option<(Buffer, u64)>,
}

/// what the Draw of a view holds: the last frame on the CPU, the way to the newest
pub(crate) struct Shared {
    pub(crate) c: RefCell<Cpu>,
}

pub(crate) struct Cpu {
    /// the last frame read back (none: 0 x 0) and its number
    pub(crate) image: Surface,
    pub(crate) image_frame: u64,
    /// the newest frame the GPU drew into the target (0: none yet)
    pub(crate) frame: u64,
    #[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
    pub(crate) gpu: Option<Target>,
    pub(crate) content: Weak<RefCell<dyn GpuContent>>,
    pub(crate) reads: u64,
    pub(crate) read_us: u64,
    #[cfg_attr(not(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32")), allow(dead_code))]
    pub(crate) error: Option<String>,
    #[cfg_attr(not(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32")), allow(dead_code))]
    raw: Vec<u8>,
}

impl Cpu {
    fn new(content: Weak<RefCell<dyn GpuContent>>) -> Cpu {
        Cpu {
            image: Surface::default(),
            image_frame: 0,
            frame: 0,
            #[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
            gpu: None,
            content,
            reads: 0,
            read_us: 0,
            error: None,
            raw: Vec::new(),
        }
    }

    /// the newest frame into the image (waits for the GPU; the browser cannot: there it comes with a later call)
    /// -> the image holds it now
    #[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
    pub(crate) fn read(&mut self) -> bool {
        if self.image_frame == self.frame {
            return true;
        }
        let t0 = plat::time_us();
        let frame = self.frame;
        let Some(g) = self.gpu.as_mut() else { return false };
        let (w, h, stride) = (g.t.width(), g.t.height(), g.stride);
        let got = match read_back(g, frame, &mut self.raw) {
            Ok(got) => got,
            Err(e) => {
                self.error = Some(alloc::format!("{e}"));
                return false;
            }
        };
        let Some(got) = got else { return false };
        if (self.image.w, self.image.h) != (w as i32, h as i32) {
            self.image = Surface::new(w as i32, h as i32, gfx::FMT_XRGB);
        }
        let (row, to) = (w as usize * 4, self.image.stride as usize);
        for y in 0..h as usize {
            self.image.px[y * to..y * to + row].copy_from_slice(&self.raw[y * stride as usize..][..row]);
        }
        self.image_frame = got;
        self.reads += 1;
        self.read_us += plat::time_us().saturating_sub(t0);
        self.image_frame == self.frame
    }
}

/// the target's frame into raw (rows of g.stride bytes) -> the number of the frame it holds, None: not yet (the
/// browser).  The target rests in Layout::Sampled between frames (the content's pass ends there): to CopySrc for
/// the copy (after the shaders that sampled it: the output's pass), back to Sampled (before the next shader reads
/// it); the copy's writes before the host reads them, which waits for the submission's value
#[cfg(any(windows, jfgui_glibc, target_os = "android"))]
fn read_back(g: &mut Target, frame: u64, raw: &mut Vec<u8>) -> gpu::Result<Option<u64>> {
    let rb = &g.rb;
    let mut e = g.dev.encoder()?;
    e.texture_barrier(&g.t, Layout::Sampled, Layout::CopySrc);
    e.copy_texture_to_buffer(TextureRegion::mip(&g.t, 0), rb, DataLayout { offset: 0, bytes_per_row: g.stride, rows_per_image: 0 });
    e.texture_barrier(&g.t, Layout::CopySrc, Layout::Sampled);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    let id = g.dev.queue().submit([e.finish()?])?;
    if !g.dev.wait(id, 5_000_000_000)? {
        return Err(GpuError::Timeout);
    }
    raw.resize(rb.size() as usize, 0);
    rb.read(0, &mut raw[..])?;
    Ok(Some(frame))
}

/// (the browser) a read-back on its way: done -> its frame; else one is sent off for this frame and comes later
#[cfg(target_arch = "wasm32")]
fn read_back(g: &mut Target, frame: u64, raw: &mut Vec<u8>) -> gpu::Result<Option<u64>> {
    use gpu::{BufferDesc, BufferUsage, MemoryKind};
    if let Some((rb, at)) = &g.wait {
        raw.resize(rb.size() as usize, 0);
        return match rb.read(0, &mut raw[..]) {
            Ok(()) => {
                let at = *at;
                g.wait = None;
                Ok(Some(at))
            }
            Err(GpuError::NotReady) => Ok(None),
            Err(e) => {
                g.wait = None;
                Err(e)
            }
        };
    }
    let rb = g.dev.create_buffer(&BufferDesc { size: g.stride as u64 * g.t.height() as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?;
    let mut e = g.dev.encoder()?;
    e.texture_barrier(&g.t, Layout::Sampled, Layout::CopySrc);
    e.copy_texture_to_buffer(TextureRegion::mip(&g.t, 0), &rb, DataLayout { offset: 0, bytes_per_row: g.stride, rows_per_image: 0 });
    e.texture_barrier(&g.t, Layout::CopySrc, Layout::Sampled);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    g.dev.queue().submit([e.finish()?])?;
    g.wait = Some((rb, frame));
    Ok(None)
}

/// a view's Draw: its last frame (the newest read back first), else the content's painting
pub(crate) struct ViewDraw(pub(crate) Rc<Shared>);

impl Draw for ViewDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        #[cfg_attr(not(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32")), allow(unused_mut))]
        let mut c = self.0.c.borrow_mut();
        #[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
        c.read();
        if c.image_frame != 0 && (c.image.w, c.image.h) == (r[2], r[3]) {
            g.dc.blit(g.sf, &c.image, r[0], r[1]);
            return;
        }
        let content = c.content.upgrade();
        drop(c);
        // (borrowed elsewhere: an app that reads the framebuffer while it holds its content)
        match content.as_ref().and_then(|x| x.try_borrow().ok()) {
            Some(x) => x.paint(g, r),
            None => paint_default(g, r),
        }
    }
}

/// a view's target on the device it is drawn with (gpu_out.rs): the texture, the output's bind group for it
#[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
pub(crate) struct ViewTex {
    pub(crate) t: Texture,
    pub(crate) group: Option<gpu::BindGroup>,
    pub(crate) w: u32,
    pub(crate) h: u32,
    /// no frame drawn into it yet (Layout::Undefined)
    pub(crate) fresh: core::cell::Cell<bool>,
}

/// a view the UI has seen: (window, id), its content, its Draw; the device side (gpu_out.rs)
#[cfg_attr(not(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32")), allow(dead_code))]
pub(crate) struct Entry {
    pub(crate) win: usize,
    pub(crate) id: u32,
    pub(crate) content: Weak<RefCell<dyn GpuContent>>,
    pub(crate) sh: Rc<Shared>,
    pub(crate) draw: DrawRef,
    /// the device generation the content is attached to (0: none), whether it is the output's, its level
    pub(crate) devgen: u32,
    pub(crate) output: bool,
    pub(crate) level: Option<gpu::Level>,
    /// a generation whose attach failed: not again on it
    pub(crate) refused: u32,
    #[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
    pub(crate) tex: Option<Rc<ViewTex>>,
    /// ver, size and generation of the last frame tried (another one draws a frame)
    pub(crate) tried: Option<(u32, (u32, u32), u32)>,
    /// a frame was drawn in this render; when its pixels go to the framebuffer at rest (0: done)
    pub(crate) moved: bool,
    pub(crate) rest_at: u64,
    pub(crate) apart: bool,
    pub(crate) info: GpuViewInfo,
}

/// the views' own device: none, made (on first use when there is no output), to be tried again, never
#[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
#[derive(Default)]
pub(crate) enum Own {
    #[default]
    None,
    Ready {
        _inst: gpu::Instance,
        dev: Device,
        devgen: u32,
    },
    Retry(u64),
    Never(String),
}

/// the views of all windows
#[derive(Default)]
pub(crate) struct S {
    pub(crate) views: Vec<Entry>,
    #[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
    pub(crate) own: Own,
}

impl Ui {
    /// a view of GPU content (gpu_view.rs): the layout's next rectangle (next_size) shows what `content` draws on
    /// the UI's device, between the widgets, under popups and tooltips; its input is ui.canvas's.  `ver` stands for
    /// everything the view shows (scene, camera, an animation's time): another value draws a frame, the same one
    /// none (an animation asks for passes itself, ui.invalidate).  Hand the same content every frame
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn gpu_view<C: GpuContent + 'static>(&mut self, id: u32, content: &Rc<RefCell<C>>, ver: u32) -> &UiInput {
        let rc: Rc<RefCell<dyn GpuContent>> = content.clone();
        let win = self.uiw_cur as usize;
        let draw = self.gv_entry(win, id, &rc);
        let _ = self.canvas(id, Some(draw.clone()), 0, ver);
        let l = self.wp.cmd.layer as usize;
        let list = &self.wp.cmd.layers[l];
        // (a full layer drops the command)
        if let Some(Cmd::Custom { f: Some(f), .. }) = list.last() {
            if Rc::as_ptr(f) as *const () == Rc::as_ptr(&draw) as *const () {
                let index = list.len() as u32 - 1;
                self.wp.canv.views.push(ViewRec { layer: l as u8, index, id, ver, apart: false });
            }
        }
        &self.uiinput
    }

    /// what view `id` of window `win` (0 = main) does now; None: no such view
    pub fn gpu_view_info(&self, win: u32, id: u32) -> Option<GpuViewInfo> {
        let e = self.m.gview.views.iter().find(|e| e.win == win as usize && e.id == id)?;
        let c = e.sh.c.borrow();
        let mut i = e.info.clone();
        i.device = e.devgen != 0;
        i.output = e.devgen != 0 && e.output;
        i.level = if e.devgen != 0 { e.level } else { None };
        i.generation = e.devgen;
        i.apart = e.apart;
        i.frames = c.frame;
        i.reads = c.reads;
        i.read_us = c.read_us;
        if i.error.is_none() {
            i.error = c.error.clone();
        }
        // (why there is no device: the views' own could not be made)
        #[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
        if let (None, 0, Own::Never(why)) = (&i.error, e.devgen, &self.m.gview.own) {
            i.error = Some(why.clone());
        }
        Some(i)
    }

    /// the record of view id in window win, made the first time -> its Draw.  Another content under the id: the old
    /// one leaves its device, the new one starts afresh
    fn gv_entry(&mut self, win: usize, id: u32, rc: &Rc<RefCell<dyn GpuContent>>) -> DrawRef {
        let views = &mut self.m.gview.views;
        let at = Rc::as_ptr(rc) as *const ();
        if let Some(e) = views.iter_mut().find(|e| e.win == win && e.id == id) {
            if e.content.as_ptr() as *const () != at {
                if e.devgen != 0 {
                    if let Some(old) = e.content.upgrade() {
                        if let Ok(mut o) = old.try_borrow_mut() {
                            o.detach();
                        }
                    }
                    e.info.detaches += 1;
                }
                e.content = Rc::downgrade(rc);
                e.devgen = 0;
                e.refused = 0;
                e.tried = None;
                e.apart = false;
                e.rest_at = 0;
                #[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
                {
                    e.tex = None;
                }
                let mut c = e.sh.c.borrow_mut();
                let (frame, reads, read_us) = (c.frame, c.reads, c.read_us);
                *c = Cpu::new(Rc::downgrade(rc));
                // (the numbers go on: a new frame must not look like an old one)
                (c.frame, c.image_frame, c.reads, c.read_us) = (frame, frame, reads, read_us);
            }
            return e.draw.clone();
        }
        let sh = Rc::new(Shared { c: RefCell::new(Cpu::new(Rc::downgrade(rc))) });
        let draw: DrawRef = Rc::new(ViewDraw(sh.clone()));
        views.push(Entry {
            win,
            id,
            content: Rc::downgrade(rc),
            sh,
            draw: draw.clone(),
            devgen: 0,
            output: false,
            level: None,
            refused: 0,
            #[cfg(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32"))]
            tex: None,
            tried: None,
            moved: false,
            rest_at: 0,
            apart: false,
            info: GpuViewInfo::default(),
        });
        draw
    }
}
