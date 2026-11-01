// gpu_out -- UI windows on the screen through a swapchain (Windows; docs/GPU.md "Ausgabe der UI")
//
// The UI rasterizes on the CPU as ever, the framebuffer stays the truth.  This module only shows
// it: the dirty rectangles the runtime computed go into a texture of the window, a triangle draws
// the texture to the swapchain image, the image is presented.  All windows share one device.
//
//   gp_window_mode()             window_main: is the output wanted (jfgui_GPU, Remote Desktop)
//   gp_present(cur, pw, n)       uiw_present: true when the GPU showed the frame, else the caller
//                                presents through the CPU path and calls gp_cpu_done(cur, pw)
//   gp_tick() / gp_deadline()    the window loop: finished jobs, frames that found no free image;
//                                the time of the next look (None: nothing to look at, no wake-up)
//   gp_window_gone(i)            before window i's plat window is destroyed
//   gp_shutdown()                window_main's end
//   sc_gpu(app, args)            the script command `gpu` (below)
//   Ui::gpu_output(win)          what a window shows through (status and counters for tests)
//
// Stages of a window: Cpu (plat::win_present: GDI, as without a GPU) -> Job (its swapchain is
// being built on a worker thread, the CPU path goes on) -> Switch (the whole framebuffer uploaded
// and presented on the hidden child, then the child shown) -> Gpu.  Back to Cpu: the window's size
// changed (also the modal size loop: every frame of a resize is a CPU frame, drawn before
// SetWindowPos returns, so nothing lags behind the frame; the swapchain is rebuilt when the size
// has settled), or an error.  Going back shows the framebuffer through the CPU path first, then
// hides the child: no gap, no wrong frame.  The first frames of a program are CPU frames: the
// instance, device and swapchain (hundreds of ms the first time) are made on a worker thread that
// starts after the first frame, so the first frame never waits for them.
//
// The swapchain lives on a child window (plat win_gpuout.rs): once a swapchain has presented on a
// window GDI stays invisible there, also after the swapchain is gone.
//
// Presents never wait: Swapchain::try_acquire.  A frame without a free image stays in the texture
// and is shown by gp_tick or with the next frame (gp_deadline: 2 ms); so is a frame less than 4 ms
// after the last present (nothing shows faster, the GPU need not draw it).  Nothing runs while the
// app is idle: no thread, no timer, no frame.  Minimised windows upload nothing; they get the whole
// frame when they come back.
//
// Popups (plat::popup: small, short-lived windows with a window region) and tooltips stay on the
// CPU path: a swapchain takes tens of ms, longer than many of them live.
//
// Layers (G3b).  The framebuffer stays the truth: all layers flattened, byte for byte what the CPU
// path holds.  What the GPU gets apart: a window's base texture holds the base layer only; each
// layer above it with commands has a texture of its own that holds, in the 32 x 32 px cells the
// layer touches, the pixels up to and including that layer, flattened on the CPU over what really
// lies below (shadows, translucent widgets, subpixel text, RTL: nothing to get wrong, the same
// kernels on the same pixels).  The render pass draws the base, then these cells opaque in layer
// order; the GPU blends nothing (blending a premultiplied layer differs from drawing directly by
// 1 .. 2 levels in about an eighth of its translucent pixels, measured, and subpixel text needs an
// opaque target; docs/GPU.md).  Beside each texture the pixels below the layer stay in a Store,
// cell by cell, so a layer that moves or closes puts them back in the framebuffer: nothing below
// it is drawn again or uploaded; closing costs no upload at all.  dirty.rs tells per cell which
// layer changed first (W::through): gp_split_draw restores below that layer, then draws and uploads
// only from there on.  Something changing below an open layer draws what lies above it in those
// cells, as ever.  Textures and stores exist while their layer has content and LINGER_US longer
// (a menu closed and another opened need no new memory).
//
// Images (G3b).  A big opaque Cmd::Image of the base layer that has cost as much in redraws as its
// upload would (rent or buy: a static image never takes GPU memory) becomes a texture, cached by
// surface identity and version, under a budget with LRU eviction; it goes up in parts over the
// frames that draw it (IMAGE_STEP_BYTES, the CPU draws it meanwhile).  In the cells it fills, as the
// last command there, its pixels are drawn by the GPU with Dc::blit_scaled's integer mapping: the
// CPU neither draws nor uploads them (Split::owed: the framebuffer lacks them; gp_settle draws them
// before anybody reads it).  Everything else, also the cells at the image's edge or with something
// on top of it, goes the usual way.
//
// A lost device or any other failure: every window back to the CPU path (the framebuffer holds the
// frame), a new device after 2 s, 4 s, ... 5 min (jfgui_GPU_RETRY_MS: the first wait).  No Vulkan, no
// usable adapter or surface: the CPU path for good, silently.
//
// Environment: jfgui_GPU=0|1 (off, on; default: DEFAULT_ON), jfgui_GPU_VERIFY=1 (every presented
// image read back and compared with the framebuffer), jfgui_GPU_TRACE=1 (stderr: the stages and
// their times), jfgui_GPU_LAYERS=0 (one texture per window: no layers, no images),
// jfgui_GPU_IMAGES=0 (no images), jfgui_GPU_IMAGE_MB=n (the images' budget); lib/gpu's
// jfgui_GPU_PRESENT, jfgui_GPU_ADAPTER, jfgui_GPU_VALIDATE, ...
//
// Script command `gpu` (headless runs stay on the CPU path without it; on other targets, or
// without a GPU, it prints a SKIP once and does nothing):
//   gpu on                 a real window per UI window (shown without activation) with
//                          the output, every presented image compared with the framebuffer, a
//                          render after every input edge; waits for the swapchain
//   gpu off                windows closed
//   gpu wait [MS]          until the scoped windows are on the swapchain (after `size`, `gpu lose`)
//   gpu expect gpu|cpu     the main window's stage
//   gpu expect same        every window presents once more: the image read back = the framebuffer
//                          byte for byte (x byte: 255), and no frame differed since the last check
//   gpu expect screen      the composed screen at the window = the framebuffer (another window on
//                          top of ours is a note, not a failure)
//   gpu expect window      the window's DC = the framebuffer (the CPU path's pixels)
//   gpu expect upload N    at most N bytes uploaded since the last check
//   gpu expect base N      at most N bytes of them to the base texture since the last check
//   gpu expect raster N    at most N pixels of the base layer drawn since the last check
//   gpu expect layers N    N layers above the base have a texture now
//   gpu expect images N    N image textures exist
//   gpu budget BYTES       the images' budget
//   gpu lose               the device behaves as lost from now on (Device::test_lose)
//   gpu print              the counters

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::{Rc, Weak};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use gpu::{
    Access, Adapter, BindEntry, BindGroup, BindGroupDesc, BindGroupLayout, Binding, BindingKind, Buffer, BufferDesc, BufferUsage,
    ColorAttachment, ColorTarget, Cull, DataLayout, Device, DeviceDesc, Format, GpuError, Instance, Layout, Level, Load, MemoryKind,
    PipelineLayoutDesc, PresentMode, RenderPassDesc, RenderPipeline, RenderPipelineDesc, Resource, ShaderModule, ShaderStage, Stages,
    Swapchain, SwapchainDesc, Texture, TextureDesc, TextureRegion, TextureUsage, Topology, TryAcquire,
};
use rt::sync::SpinLock;

use super::auto::{next_word, skip_ws, word_eq};
use super::cmd::{render_layer, Cmd};
use super::consts::*;
use super::dirty::TOP_BLOCKED;
use super::{App, GpuOutput, Ui};
use gfx::{Dc, Surface, FMT_A8, FMT_XRGB};

/// the output without jfgui_GPU in the environment: on, jfgui_GPU=0 turns it off (docs/GPU.md
/// "Ausgabe der UI" says what it costs)
const DEFAULT_ON: bool = true;
/// the present mode (FIFO when the surface lacks it; jfgui_GPU_PRESENT overrides it): a burst of
/// frames shows its newest within a refresh, where FIFO lags 3 .. 5 refreshes behind (docs/GPU.md
/// "Ausgabe der UI")
const PRESENT: PresentMode = PresentMode::Mailbox;
/// swapchain images: as fast as three in every measurement, a window's size less GPU memory
const IMAGES: u32 = 2;
/// frames closer together than this wait for gp_tick: 250 per second are plenty for a window (us)
const MIN_PRESENT_US: u64 = 4_000;
/// a window's size is settled when it did not change for this long (us)
const STABLE_US: u64 = 100_000;
/// a frame that found no free image is tried again after this long (us)
const RETRY_US: u64 = 2_000;
/// while a job runs the loop looks this often (us)
const JOB_US: u64 = 8_000;
const WORKER_STACK: usize = 2 << 20;
/// the first wait before a new device (ms), its longest
const RETRY_FIRST_MS: u64 = 2_000;
const RETRY_MAX_MS: u64 = 300_000;
/// an upload above this keeps a large staging buffer for a while: gp_tick gives it back
const BIG_UPLOAD: u64 = 128 << 10;
const TRIM_US: u64 = 1_200_000;
/// a Remote Desktop session is looked at again after this long (us)
const REMOTE_LOOK_US: u64 = 10_000_000;
/// the layers above the base
const UPPER: usize = L_COUNT - 1;
/// dirty.rs's cells: what a layer's texture and store are made of
const CELL: i32 = 32;
const CELL_ROW: usize = CELL as usize * 4;
const CELL_BYTES: usize = CELL as usize * CELL_ROW;
const NO_BLOCK: u32 = u32::MAX;
/// a layer's texture: its size is rounded up to this (px), half of the spare room on each side
/// of what it holds now
const LAYER_STEP: i32 = 256;
const LAYER_SPARE: i32 = 128;
/// a layer's texture this many times larger than needed goes
const LAYER_SHRINK: i64 = 4;
/// a layer's texture and store stay this long after its last content (us)
const LINGER_US: u64 = 1_000_000;
/// the bytes image textures may take (jfgui_GPU_IMAGE_MB)
const IMAGE_BUDGET: u64 = 256 << 20;
/// an image goes up with this many bytes per frame: no frame waits for a whole big one
const IMAGE_STEP_BYTES: u64 = 4 << 20;
/// images whose redraws are counted
const IMAGES_KEPT: usize = 64;
/// an image texture not drawn for this long goes with the next frame (us)
const IMAGE_UNUSED_US: u64 = 30_000_000;

/// the shaders, compiled from gpu_out.wgsl (uigputest --test shaders compares them with what
/// wgsl::compile makes of it, --gen-shaders writes them again)
mod spv {
    include!("gpu_out_spv.rs");
}

/// the embedded shaders of the output and their source (uigputest: the embedding check)
pub const GPU_OUT_WGSL: &str = include_str!("gpu_out.wgsl");
pub const GPU_OUT_VS: &[u32] = spv::VS;
pub const GPU_OUT_FS: &[u32] = spv::FS;
pub const GPU_OUT_FS_IMAGE: &[u32] = spv::FS_IMAGE;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Cpu,
    Job,
    Switch,
    Gpu,
}

struct Tex {
    t: Texture,
    group: BindGroup,
    w: i32,
    h: i32,
}

/// the pixels below a layer, cell by cell, while the layer touches the cell
#[derive(Default)]
struct Store {
    idx: Vec<u32>,
    px: Vec<u8>,
    free: Vec<u32>,
    used: u32,
}

impl Store {
    fn has(&self, i: usize) -> bool {
        self.idx.get(i).is_some_and(|&b| b != NO_BLOCK)
    }

    fn get(&self, i: usize) -> &[u8] {
        &self.px[self.idx[i] as usize * CELL_BYTES..][..CELL_BYTES]
    }

    /// nothing kept, the memory stays
    fn empty(&mut self) {
        self.idx.clear();
        self.px.clear();
        self.free.clear();
        self.used = 0;
    }

    /// room for n cells more without moving what is kept
    fn reserve(&mut self, n: usize) {
        self.px.reserve_exact((n + self.used as usize).saturating_mul(CELL_BYTES).saturating_sub(self.px.len()));
    }

    /// the cell's block, a new one when it has none (cells: the window's)
    fn put(&mut self, i: usize, cells: usize) -> &mut [u8] {
        if self.idx.len() != cells {
            self.empty();
            self.idx.resize(cells, NO_BLOCK);
        }
        let mut b = self.idx[i];
        if b == NO_BLOCK {
            b = self.free.pop().unwrap_or_else(|| {
                let n = (self.px.len() / CELL_BYTES) as u32;
                self.px.resize(self.px.len() + CELL_BYTES, 0);
                n
            });
            self.idx[i] = b;
            self.used += 1;
        }
        &mut self.px[b as usize * CELL_BYTES..][..CELL_BYTES]
    }

    fn drop_cell(&mut self, i: usize) {
        if self.has(i) {
            self.free.push(self.idx[i]);
            self.idx[i] = NO_BLOCK;
            self.used -= 1;
        }
    }
}

/// a layer's texture: holds the window's rectangle ox, oy, w, h
struct LayerTex {
    t: Texture,
    group: BindGroup,
    ox: i32,
    oy: i32,
    w: i32,
    h: i32,
    fresh: bool, // nothing uploaded yet (Layout::Undefined)
}

/// a layer above the base: its texture, the rectangles of its cells (none: it has no content, the
/// texture and the store linger), the pixels below it
#[derive(Default)]
struct Upper {
    tex: Option<LayerTex>,
    rects: Vec<[i32; 4]>,
    store: Store,
}

/// an image's texture (Rc: a window's last frame keeps drawing an evicted one until its next)
struct ImageTex {
    _t: Texture,
    group: BindGroup,
}

/// an image's texture on the way up: the rows that went up so far
struct ImageLoad {
    t: Texture,
    group: BindGroup,
    row: i32,
}

/// an image the output has seen: what its redraws cost so far, its texture
struct Image {
    key: usize,
    weak: Weak<Surface>, // holds the address: no other surface comes to this identity
    ver: u32,
    bytes: u64,
    acc: u64,
    opaque: Option<bool>,
    tex: Option<Rc<ImageTex>>,
    load: Option<ImageLoad>,
    tick: u64,
    used_us: u64,
}

/// an image drawn by the GPU in a window's frame: its place and steps (Dc::blit_scaled), the
/// rectangles of its cells
struct Quad {
    tex: Rc<ImageTex>,
    src: Rc<Surface>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    step: [u32; 2],
    rects: Vec<[i32; 4]>,
}

/// what a window on the swapchain keeps apart (the module's header)
#[derive(Default)]
struct Split {
    upper: [Upper; UPPER],
    /// a layer above the base has cells apart
    active: bool,
    /// when the memory of layers without content goes (0: none lingers)
    linger_at: u64,
    /// the base texture holds no pixels of a layer above it
    clean: bool,
    /// this frame's uploads are done
    drawn: bool,
    err: Option<GpuError>,
    // ---- per cell, this frame: the lowest layer that changed (L_COUNT: none); the layers that
    // touch the cell (bits); the rectangles of a stage
    m: Vec<u8>,
    nr: Vec<u8>,
    rects: Vec<[i32; 4]>,
    // ---- per cell: the image on top of it (quad + 1) the GPU draws; the framebuffer lacks its
    // pixels; the base texture lacks its pixels
    cols: usize,
    rows: usize,
    hole: Vec<u8>,
    owed: Vec<bool>,
    stale: Vec<bool>,
    any_owed: bool,
    any_stale: bool,
    quads: Vec<Quad>,
}

impl Split {
    /// layers, images and whatever hangs on them go; the base texture is not clean then
    fn reset(&mut self) {
        *self = Split::default();
    }
}

/// a window of the output
struct Win {
    pw: i32,
    stage: Stage,
    surf: Option<gpu::Surface>,
    sc: Option<Swapchain>, // also on the CPU path: the next build reuses it (oldSwapchain)
    tex: Option<Tex>,
    rb: Option<Buffer>, // jfgui_GPU_VERIFY: the presented image
    rb_px: Vec<u8>,
    full: bool,    // the texture lacks pixels: the whole frame goes up next
    undef: bool,   // the texture is new (Layout::Undefined)
    pending: bool, // the texture holds pixels that were not presented
    last_present: u64,
    job: u32,      // the job in flight
    size: (i32, i32),
    stable_at: u64,
    fail_at: u64,  // no new swapchain before this time
    fail_ms: u64,
    out: GpuOutput,
    sp: Split,
}

struct Core {
    inst: Instance,
    dev: Device,
    bgl: BindGroupLayout,
    pipe: RenderPipeline,
    pipe_image: RenderPipeline,
    format: Format,
    name: String,
}

enum Dev {
    None,
    Starting(u32),
    Ready(Core),
    Retry(u64),
    Never(String),
}

struct Built {
    surf: gpu::Surface,
    sc: Swapchain,
    tex: Texture,
    group: BindGroup,
}

enum Done {
    Device { job: u32, win: usize, r: gpu::Result<(Core, Built)> },
    Window { job: u32, win: usize, r: gpu::Result<Built> },
}

struct Jobs {
    done: SpinLock<Vec<Done>>,
    running: AtomicU32,
}

/// the output's state: one per process (Mods)
pub(crate) struct S {
    want: bool,
    verify: bool,
    trace: bool,
    script: bool,
    skip: bool,
    dev: Dev,
    wins: Vec<Option<Win>>,
    jobs: Arc<Jobs>,
    threads: Vec<plat::Thread>,
    next_job: u32,
    retry_ms: u64,
    ready_at: u64,
    trim_at: u64,
    t0: u64,
    script_pw: [i32; super::consts::WIN_MAX],
    layers_on: bool,
    images_on: bool,
    images: Vec<Image>,
    image_budget: u64,
    image_bytes: u64,
    image_uploads: u64,
    image_hits: u64,
    image_evicted: u64,
    tick: u64,
    look_at: u64,
}

impl Default for S {
    fn default() -> S {
        S {
            want: false,
            verify: false,
            trace: false,
            script: false,
            skip: false,
            dev: Dev::None,
            wins: Vec::new(),
            jobs: Arc::new(Jobs { done: SpinLock::new(Vec::new()), running: AtomicU32::new(0) }),
            threads: Vec::new(),
            next_job: 1,
            retry_ms: 0,
            ready_at: 0,
            trim_at: 0,
            t0: 0,
            script_pw: [-1; super::consts::WIN_MAX],
            layers_on: false,
            images_on: false,
            images: Vec::new(),
            image_budget: IMAGE_BUDGET,
            image_bytes: 0,
            image_uploads: 0,
            image_hits: 0,
            image_evicted: 0,
            tick: 0,
            look_at: 0,
        }
    }
}

fn env_on(name: &str) -> bool {
    plat::env(name).is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("on"))
}

fn wanted() -> bool {
    let on = match plat::env("jfgui_GPU").as_deref() {
        Some("0") | Some("off") => false,
        Some("1") | Some("on") => true,
        _ => DEFAULT_ON,
    };
    on && !plat::win_remote_session()
}

fn ms(a: u64, b: u64) -> f64 {
    b.saturating_sub(a) as f64 / 1000.0
}

impl S {
    fn setup(&mut self) {
        self.verify = env_on("jfgui_GPU_VERIFY");
        self.trace = env_on("jfgui_GPU_TRACE");
        self.t0 = plat::time_us();
        self.retry_ms = plat::env("jfgui_GPU_RETRY_MS").and_then(|v| v.parse().ok()).unwrap_or(RETRY_FIRST_MS);
        let off = |name: &str| plat::env(name).is_some_and(|v| v == "0" || v.eq_ignore_ascii_case("off"));
        self.layers_on = !off("jfgui_GPU_LAYERS");
        self.images_on = self.layers_on && !off("jfgui_GPU_IMAGES");
        if let Some(mb) = plat::env("jfgui_GPU_IMAGE_MB").and_then(|v| v.parse::<u64>().ok()) {
            self.image_budget = mb << 20;
        }
    }

    /// an image's texture goes (a window's last frame may keep drawing it)
    fn evict(&mut self, k: usize) {
        let e = &mut self.images[k];
        if e.tex.take().is_some() | e.load.take().is_some() {
            self.image_bytes -= e.bytes;
            self.image_evicted += 1;
            self.trim_at = plat::time_us() + TRIM_US;
        }
    }

    /// every image texture goes: the device with them
    fn images_gone(&mut self) {
        for e in self.images.iter_mut() {
            e.tex = None;
            e.load = None;
        }
        self.image_bytes = 0;
    }

    fn desc(&self) -> SwapchainDesc {
        SwapchainDesc { present: PRESENT, copy_src: self.verify, min_images: IMAGES, ..SwapchainDesc::DEFAULT }
    }

    fn note(&self, what: &str) {
        if self.trace {
            plat::write_stderr(format!("gpu: {:9.2} ms  {what}\n", ms(self.t0, plat::time_us())).as_bytes());
        }
    }

    fn win(&mut self, i: usize) -> Option<&mut Win> {
        self.wins.get_mut(i).and_then(|w| w.as_mut())
    }

    /// the worker threads: one per job, joined when none runs (they end by themselves)
    fn spawn(&mut self, f: impl FnOnce() -> Done + Send + 'static) -> bool {
        let jobs = self.jobs.clone();
        jobs.running.fetch_add(1, Ordering::AcqRel);
        let j = jobs.clone();
        let t = plat::thread_spawn(WORKER_STACK, Box::new(move || {
            let d = f();
            j.done.lock().push(d);
            j.running.fetch_sub(1, Ordering::AcqRel);
        }));
        match t {
            Some(t) => {
                self.threads.push(t);
                true
            }
            None => {
                jobs.running.fetch_sub(1, Ordering::AcqRel);
                false
            }
        }
    }
}

// ------------------------------------------------------------------ the worker's jobs

/// a swapchain (the old one rebuilt, or a new one on the surface) and the window's texture
fn build_window(dev: &Device, bgl: &BindGroupLayout, surf: gpu::Surface, old: Option<Swapchain>, desc: &SwapchainDesc) -> gpu::Result<Built> {
    let sc = match old {
        Some(mut sc) => {
            sc.rebuild()?;
            sc
        }
        None => dev.create_swapchain(&surf, desc)?,
    };
    let (w, h) = sc.size();
    if w == 0 || h == 0 {
        return Err(GpuError::NotReady); // minimised meanwhile
    }
    let tex = dev.create_texture(&TextureDesc::d2(w, h, Format::Bgra8Unorm, TextureUsage::COPY_DST | TextureUsage::SAMPLED))?;
    let group = dev.create_bind_group(&BindGroupDesc {
        layout: bgl,
        entries: &[BindEntry { binding: 0, resource: Resource::Texture(tex.view()) }],
    })?;
    Ok(Built { surf, sc, tex, group })
}

/// instance, the adapter for this window, device, pipeline, the first swapchain
fn build_device(nw: plat::NativeWindow, pw: i32, display: Option<(u32, u32)>, desc: SwapchainDesc, trace: bool) -> gpu::Result<(Core, Built)> {
    let t0 = plat::time_us();
    let inst = Instance::new()?;
    let t1 = plat::time_us();
    let surf = gpu::Surface::native(&inst, nw, pw)?;
    let list: Vec<Adapter> = inst.adapters_low_power(display)?;
    let adapter = list.into_iter().find(|a| a.can_present(&surf)).ok_or(GpuError::NoAdapter)?;
    let t2 = plat::time_us();
    let dev = adapter.create_device(&DeviceDesc { max_level: Level::Basis })?;
    let t3 = plat::time_us();
    let bgl = dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::Texture, stages: Stages::FRAGMENT }])?;
    let built = build_window(&dev, &bgl, surf, None, &desc)?;
    let t4 = plat::time_us();
    let format = built.sc.format();
    let vs = ShaderModule::from_spirv(&dev, spv::VS)?;
    let fs = ShaderModule::from_spirv(&dev, spv::FS)?;
    let fs_image = ShaderModule::from_spirv(&dev, spv::FS_IMAGE)?;
    let layout = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 16 })?;
    let pipeline = |fragment: &'_ ShaderModule, entry: &'static str| {
        dev.create_render_pipeline(&RenderPipelineDesc {
            layout: &layout,
            vertex: ShaderStage::new(&vs, "vs"),
            fragment: Some(ShaderStage::new(fragment, entry)),
            buffers: &[],
            topology: Topology::Triangles,
            cull: Cull::None,
            front_cw: false,
            targets: &[ColorTarget { format, blend: None, write_mask: 0xF }],
            depth: None,
        })
    };
    let pipe = pipeline(&fs, "fs")?;
    let pipe_image = pipeline(&fs_image, "fs_image")?;
    let t5 = plat::time_us();
    if trace {
        plat::write_stderr(format!(
            "gpu: worker: instance {:.2} ms, adapter {:.2}, device {:.2}, swapchain + texture {:.2}, pipeline {:.2} ({})\n",
            ms(t0, t1), ms(t1, t2), ms(t2, t3), ms(t3, t4), ms(t4, t5), adapter.info().name).as_bytes());
    }
    let name = adapter.info().name.clone();
    Ok((Core { inst, dev, bgl, pipe, pipe_image, format, name }, built))
}

// ------------------------------------------------------------------ one frame

/// the frame's rectangles (None: all of it) into the window's texture
fn upload(w: &mut Win, core: &Core, fb: &Surface, rects: Option<&[[i32; 4]]>, trim_at: &mut u64) -> gpu::Result<()> {
    let Some(tex) = &w.tex else { return Ok(()) };
    let all = [[0, 0, fb.w, fb.h]];
    let (rs, before) = match rects {
        Some(r) if !w.full && !w.undef => (r, Layout::Sampled),
        // the whole frame replaces everything: the old contents do not matter
        _ => (&all[..], Layout::Undefined),
    };
    core.dev.queue().write_texture_rects(&tex.t, rs, &fb.px, fb.stride as u32, before, Layout::Sampled)?;
    let bytes: u64 = rs.iter().map(|r| r[2].max(0) as u64 * r[3].max(0) as u64 * 4).sum();
    w.out.upload_bytes += bytes;
    w.out.upload_base += bytes;
    if bytes > BIG_UPLOAD {
        *trim_at = plat::time_us() + TRIM_US;
    }
    w.full = false;
    w.undef = false;
    w.pending = true;
    Ok(())
}

enum Shown {
    Done,
    /// no image free yet: gp_tick or the next frame
    Wait,
    NoArea,
    Stale,
}

/// the texture to a swapchain image and on the screen, without waiting
fn show(w: &mut Win, core: &Core, fb: &Surface) -> gpu::Result<Shown> {
    let (Some(sc), Some(tex)) = (w.sc.as_mut(), w.tex.as_ref()) else { return Ok(Shown::Stale) };
    let fr = match sc.try_acquire()? {
        TryAcquire::Frame(f) => f,
        TryAcquire::NotReady => return Ok(Shown::Wait),
        TryAcquire::NoArea => return Ok(Shown::NoArea),
        TryAcquire::Stale => return Ok(Shown::Stale),
    };
    let sc = &*sc;
    let mut e = core.dev.encoder()?;
    let after = if w.rb.is_some() { Layout::CopySrc } else { Layout::Present };
    e.begin_render(&RenderPassDesc {
        color: &[ColorAttachment { view: sc.view(&fr), load: Load::Discard, clear: [0.0; 4], store: true, before: Layout::Undefined, after }],
        depth: None,
    });
    e.set_render_pipeline(&core.pipe);
    e.set_bind_group(0, &tex.group);
    e.push_constants(0, &[0i32; 4]);
    e.draw(3, 1, 0, 0);
    let scissor = |e: &mut gpu::CommandEncoder, r: &[i32; 4]| e.set_scissor(r[0] as u32, r[1] as u32, r[2] as u32, r[3] as u32);
    // the images the CPU left out
    if !w.sp.quads.is_empty() {
        e.set_render_pipeline(&core.pipe_image);
        for q in &w.sp.quads {
            e.set_bind_group(0, &q.tex.group);
            e.push_constants(0, &[q.x, q.y, q.step[0] as i32, q.step[1] as i32]);
            for r in &q.rects {
                scissor(&mut e, r);
                e.draw(3, 1, 0, 0);
            }
        }
        e.set_render_pipeline(&core.pipe);
    }
    // the layers above, opaque: each cell holds what is there up to its layer
    for u in w.sp.upper.iter().filter(|u| !u.rects.is_empty()) {
        if let Some(t) = &u.tex {
            e.set_bind_group(0, &t.group);
            e.push_constants(0, &[t.ox, t.oy, 0, 0]);
            for r in &u.rects {
                scissor(&mut e, r);
                e.draw(3, 1, 0, 0);
            }
        }
    }
    e.end_render();
    if let Some(rb) = &w.rb {
        let img = sc.texture(&fr);
        e.copy_texture_to_buffer(TextureRegion::mip(img, 0), rb, DataLayout::default());
        e.texture_barrier(img, Layout::CopySrc, Layout::Present);
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    }
    let cmd = e.finish()?;
    let id = sc.present(fr, [cmd])?;
    w.pending = false;
    w.last_present = plat::time_us();
    w.out.frames += 1;
    if let Some(rb) = &w.rb {
        core.dev.wait(id, 5_000_000_000)?;
        let n = fb.w as usize * fb.h as usize * 4;
        w.rb_px.resize(n, 0);
        rb.read(0, &mut w.rb_px[..n])?;
        let (x, y, bad) = compare(&w.rb_px, fb, core.format == Format::Rgba8Unorm);
        w.out.verified += 1;
        if bad != 0 {
            w.out.verify_bad += 1;
            plat::write_stderr(format!("gpu: the presented image differs from the framebuffer in {bad} pixels, first at {x}, {y}\n").as_bytes());
        }
    }
    Ok(Shown::Done)
}

/// the presented image (rows of w * 4 bytes) against the framebuffer: every colour byte equal and
/// the fourth byte 255 (the framebuffer's x byte means nothing, the window is opaque) -> first
/// x, y and the number of pixels that differ
fn compare(img: &[u8], fb: &Surface, swap_rb: bool) -> (i32, i32, u64) {
    let (mut fx, mut fy, mut bad) = (0, 0, 0u64);
    let row = fb.w as usize * 4;
    for y in 0..fb.h as usize {
        let a = &img[y * row..y * row + row];
        let b = &fb.px[y * fb.stride as usize..y * fb.stride as usize + row];
        // the usual case at memcmp speed: fails only for the x byte
        for (x, (p, q)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
            let same = if swap_rb { p[0] == q[2] && p[1] == q[1] && p[2] == q[0] } else { p[..3] == q[..3] };
            if !same || p[3] != 255 {
                if bad == 0 {
                    fx = x as i32;
                    fy = y as i32;
                }
                bad += 1;
            }
        }
    }
    (fx, fy, bad)
}

// ------------------------------------------------------------------ cells

// the cell's box in the window (x0, y0, x1, y1)
fn cell_box(i: usize, cols: usize, w: i32, h: i32) -> [i32; 4] {
    let (c, r) = ((i % cols) as i32, (i / cols) as i32);
    [c * CELL, r * CELL, ((c + 1) * CELL).min(w), ((r + 1) * CELL).min(h)]
}

fn cell_save(fb: &Surface, b: [i32; 4], to: &mut [u8]) {
    let n = (b[2] - b[0]) as usize * 4;
    for y in b[1]..b[3] {
        let s = y as usize * fb.stride as usize + b[0] as usize * 4;
        let d = (y - b[1]) as usize * CELL_ROW;
        to[d..d + n].copy_from_slice(&fb.px[s..s + n]);
    }
}

fn cell_load(fb: &mut Surface, b: [i32; 4], from: &[u8]) {
    let n = (b[2] - b[0]) as usize * 4;
    for y in b[1]..b[3] {
        let d = y as usize * fb.stride as usize + b[0] as usize * 4;
        let s = (y - b[1]) as usize * CELL_ROW;
        fb.px[d..d + n].copy_from_slice(&from[s..s + n]);
    }
}

// the cells f holds for as rectangles x, y, w, h cut to the w x h window: a run in a row extends a
// rectangle of the row above with the same columns
fn cell_rects(out: &mut Vec<[i32; 4]>, cols: usize, rows: usize, w: i32, h: i32, f: impl Fn(usize) -> bool) {
    out.clear();
    for row in 0..rows {
        let mut c = 0;
        while c < cols {
            if !f(row * cols + c) {
                c += 1;
                continue;
            }
            let mut e = c + 1;
            while e < cols && f(row * cols + e) {
                e += 1;
            }
            let (x, rw, y) = (c as i32 * CELL, (e as i32 * CELL).min(w) - c as i32 * CELL, row as i32 * CELL);
            let rh = (y + CELL).min(h) - y;
            if let Some(r) = out.iter_mut().find(|r| r[0] == x && r[2] == rw && r[1] + r[3] == y) {
                r[3] += rh;
            } else {
                out.push([x, y, rw, rh]);
            }
            c = e;
        }
    }
}

// an image's pixels in window cell box b of the framebuffer, as Cmd::Image draws them there
fn quad_blit(fb: &mut Surface, q: &Quad, b: [i32; 4]) {
    let mut dc = Dc::new(fb.w, fb.h);
    dc.clip_push(b[0], b[1], b[2] - b[0], b[3] - b[1]);
    dc.blit_scaled(fb, &q.src, q.x, q.y, q.w, q.h);
}

// every alpha 255: the image replaces what lies below it
fn surface_opaque(sf: &Surface) -> bool {
    if sf.fmt == FMT_XRGB {
        return true;
    }
    let (row, stride) = (sf.w as usize * 4, sf.stride as usize);
    sf.fmt != FMT_A8 && sf.px.chunks(stride).take(sf.h as usize).all(|r| r[..row].chunks_exact(4).all(|p| p[3] == 255))
}

// the texture of an image, nothing uploaded yet
fn image_texture(core: &Core, sf: &Surface) -> gpu::Result<ImageLoad> {
    let t = core.dev.create_texture(&TextureDesc::d2(sf.w as u32, sf.h as u32, Format::Bgra8Unorm, TextureUsage::COPY_DST | TextureUsage::SAMPLED))?;
    let group = core.dev.create_bind_group(&BindGroupDesc {
        layout: &core.bgl,
        entries: &[BindEntry { binding: 0, resource: Resource::Texture(t.view()) }],
    })?;
    Ok(ImageLoad { t, group, row: 0 })
}

// a texture for what a layer holds now: bb (x0, y0, x1, y1) with spare room, inside the window
fn layer_texture(core: &Core, bb: [i32; 4], fw: i32, fh: i32) -> gpu::Result<LayerTex> {
    let size = |n: i32, lim: i32| ((n + LAYER_SPARE + LAYER_STEP - 1) / LAYER_STEP * LAYER_STEP).min(lim);
    let (w, h) = (size(bb[2] - bb[0], fw), size(bb[3] - bb[1], fh));
    let ox = (bb[0] - (w - (bb[2] - bb[0])) / 2).clamp(0, fw - w);
    let oy = (bb[1] - (h - (bb[3] - bb[1])) / 2).clamp(0, fh - h);
    let t = core.dev.create_texture(&TextureDesc::d2(w as u32, h as u32, Format::Bgra8Unorm, TextureUsage::COPY_DST | TextureUsage::SAMPLED))?;
    let group = core.dev.create_bind_group(&BindGroupDesc {
        layout: &core.bgl,
        entries: &[BindEntry { binding: 0, resource: Resource::Texture(t.view()) }],
    })?;
    Ok(LayerTex { t, group, ox, oy, w, h, fresh: true })
}

// ------------------------------------------------------------------ the runtime's calls

impl Ui {
    /// window_main: the output's decision for this run
    pub(crate) fn gp_window_mode(&mut self) {
        let s = &mut self.m.gout;
        s.want = wanted();
        s.setup();
    }

    /// uiw_present: window `cur` changed in n rectangles (drects; -1: all) -> the GPU took the frame
    pub(crate) fn gp_present(&mut self, cur: u32, pw: i32, n: i32) -> bool {
        if !self.m.gout.want {
            return false;
        }
        self.gp_poll();
        let t0 = plat::time_us();
        let i = cur as usize;
        if self.m.gout.verify {
            self.gp_settle(i); // the presented image is compared with the framebuffer
        }
        let s = &mut self.m.gout;
        let Dev::Ready(core) = &s.dev else { return false };
        let Some(Some(w)) = s.wins.get_mut(i) else { return false };
        if w.stage != Stage::Gpu {
            return false;
        }
        if let Some(e) = w.sp.err.take() {
            self.gp_failed(i, e);
            return true;
        }
        let fb = &self.wp.rt.fb;
        let fits = w.tex.as_ref().is_some_and(|t| (t.w, t.h) == (fb.w, fb.h));
        if !fits {
            self.gp_leave(i, "the size changed");
            return true;
        }
        if plat::win_state(pw) & plat::WST_MINIMIZED != 0 {
            self.gp_flatten(i); // nobody sees it: the whole frame when the window comes back
            return true;
        }
        let rects = if n < 0 { None } else { Some(&self.drects[..n as usize]) };
        let soon = t0 < w.last_present + MIN_PRESENT_US && !s.script;
        // (a frame drawn layer by layer has uploaded its parts)
        let up = if core::mem::take(&mut w.sp.drawn) { Ok(()) } else { upload(w, core, fb, rects, &mut s.trim_at) };
        let r = up.and_then(|_| if soon { Ok(Shown::Wait) } else { show(w, core, fb) });
        w.out.present_us += plat::time_us().saturating_sub(t0);
        match r {
            Ok(Shown::Done) | Ok(Shown::Wait) => {}
            Ok(Shown::NoArea) => {
                self.gp_flatten(i);
                return true;
            }
            Ok(Shown::Stale) => {
                self.gp_leave(i, "stale");
                return true;
            }
            Err(e) => {
                self.gp_failed(i, e);
                return true;
            }
        }
        plat::present_account(t0, true);
        true
    }

    /// uiw_present: the frame went through the CPU path
    pub(crate) fn gp_cpu_done(&mut self, cur: u32, pw: i32) {
        let s = &mut self.m.gout;
        if !s.want {
            return;
        }
        let i = cur as usize;
        if s.wins.len() <= i {
            s.wins.resize_with(i + 1, || None);
        }
        let now = plat::time_us();
        let w = s.wins[i].get_or_insert_with(|| Win {
            pw,
            stage: Stage::Cpu,
            surf: None,
            sc: None,
            tex: None,
            rb: None,
            rb_px: Vec::new(),
            full: true,
            undef: true,
            pending: false,
            last_present: 0,
            job: 0,
            size: (0, 0),
            stable_at: now,
            fail_at: 0,
            fail_ms: 0,
            out: GpuOutput::default(),
            sp: Split::default(),
        });
        w.pw = pw;
        w.out.cpu_frames += 1;
        let size = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        if w.size != size {
            w.size = size;
            w.stable_at = now;
        }
    }

    /// back to the CPU path, the frame shown first (no gap): window i's size changed or its
    /// swapchain went stale; the swapchain is rebuilt when the size has settled
    fn gp_leave(&mut self, i: usize, why: &str) {
        self.gp_settle(i);
        if let Some(p) = self.uiw_parts_mut(i as u32) {
            p.dirty.busy = false;
        }
        let Some(fb) = self.uiw_parts(i as u32).map(|p| &p.rt.fb) else { return };
        let size = (fb.w, fb.h);
        let Some(Some(w)) = self.m.gout.wins.get(i) else { return };
        let pw = w.pw;
        let t0 = plat::time_us();
        plat::win_present(pw, &Ui::frame_of(fb), None);
        plat::win_gpu_child_show(pw, false);
        let now = plat::time_us();
        let s = &mut self.m.gout;
        s.note(&format!("window {i}: back to the CPU path ({why}): {:.2} ms", ms(t0, now)));
        let Some(w) = s.win(i) else { return };
        w.stage = Stage::Cpu;
        w.tex = None;
        w.sp.reset();
        w.pending = false;
        w.size = size;
        w.stable_at = now;
        w.out.leaves += 1;
        w.out.cpu_frames += 1;
    }

    /// something failed.  A lost surface takes the window back to the CPU path, anything else
    /// every window, with a new device later; every framebuffer holds its frame
    fn gp_failed(&mut self, i: usize, e: GpuError) {
        plat::log_warn("gpu", &format!("output: {e}"));
        self.m.gout.note(&format!("window {i}: {e}"));
        if matches!(e, GpuError::SurfaceLost) {
            self.gp_leave(i, "surface lost");
            let now = plat::time_us();
            if let Some(w) = self.m.gout.win(i) {
                w.sc = None;
                w.surf = None;
                w.fail_ms = (w.fail_ms * 2).clamp(RETRY_FIRST_MS, RETRY_MAX_MS);
                w.fail_at = now + w.fail_ms * 1000;
            }
            return;
        }
        let n = self.m.gout.wins.len();
        for k in 0..n {
            let on = self.m.gout.wins[k].as_ref().is_some_and(|w| w.stage == Stage::Gpu);
            if on {
                self.gp_leave(k, "failure");
            }
            if let Some(w) = self.m.gout.win(k) {
                w.stage = Stage::Cpu;
                w.sc = None;
                w.surf = None;
                w.tex = None;
                w.rb = None;
                w.pending = false;
                w.sp.reset();
                w.out.error = Some(format!("{e}"));
            }
        }
        let s = &mut self.m.gout;
        s.images_gone();
        let now = plat::time_us();
        if matches!(e, GpuError::Invalid(_) | GpuError::Unsupported(_)) {
            s.dev = Dev::Never(format!("{e}"));
            return;
        }
        // a device that lives for a minute starts the waits over
        if now.saturating_sub(s.ready_at) > 60_000_000 {
            s.retry_ms = plat::env("jfgui_GPU_RETRY_MS").and_then(|v| v.parse().ok()).unwrap_or(RETRY_FIRST_MS);
        }
        s.dev = Dev::Retry(now + s.retry_ms * 1000);
        s.retry_ms = (s.retry_ms * 2).min(RETRY_MAX_MS);
    }

    /// the jobs that finished: a device, a window's swapchain
    fn gp_poll(&mut self) {
        let done = core::mem::take(&mut *self.m.gout.jobs.done.lock());
        for d in done {
            match d {
                Done::Device { job, win, r } => {
                    if !matches!(self.m.gout.dev, Dev::Starting(j) if j == job) {
                        continue;
                    }
                    match r {
                        Ok((core, built)) => {
                            let s = &mut self.m.gout;
                            s.note(&format!("device ready: {}", core.name));
                            s.dev = Dev::Ready(core);
                            s.ready_at = plat::time_us();
                            self.gp_built(win, built);
                        }
                        Err(e) => {
                            let s = &mut self.m.gout;
                            s.note(&format!("no device: {e}"));
                            match e {
                                GpuError::DeviceLost | GpuError::OutOfMemory | GpuError::Timeout | GpuError::Failed(_) | GpuError::SurfaceLost
                                | GpuError::NotReady => {
                                    s.dev = Dev::Retry(plat::time_us() + s.retry_ms * 1000);
                                    s.retry_ms = (s.retry_ms * 2).min(RETRY_MAX_MS);
                                }
                                _ => s.dev = Dev::Never(format!("{e}")), // no Vulkan, no usable adapter: the CPU path for good
                            }
                            if let Some(w) = s.win(win) {
                                w.stage = Stage::Cpu;
                                w.out.error = Some(format!("{e}"));
                            }
                        }
                    }
                }
                Done::Window { job, win, r } => {
                    let live = self.m.gout.win(win).is_some_and(|w| w.job == job && w.stage == Stage::Job);
                    if !live {
                        continue;
                    }
                    match r {
                        Ok(b) => self.gp_built(win, b),
                        Err(e) => {
                            let now = plat::time_us();
                            let s = &mut self.m.gout;
                            s.note(&format!("window {win}: no swapchain: {e}"));
                            if let Some(w) = s.win(win) {
                                w.stage = Stage::Cpu;
                                w.out.error = Some(format!("{e}"));
                                w.fail_ms = (w.fail_ms * 2).clamp(RETRY_FIRST_MS, RETRY_MAX_MS);
                                w.fail_at = now + w.fail_ms * 1000;
                            }
                        }
                    }
                }
            }
        }
        let s = &mut self.m.gout;
        if !s.threads.is_empty() && s.jobs.running.load(Ordering::Acquire) == 0 {
            for t in s.threads.drain(..) {
                t.join();
            }
        }
    }

    /// a swapchain and a texture for window i are there: on to the switch
    fn gp_built(&mut self, i: usize, b: Built) {
        let verify = self.m.gout.verify;
        let s = &mut self.m.gout;
        let Dev::Ready(core) = &s.dev else { return };
        let (dev, format) = (core.dev.clone(), core.format);
        let Some(w) = s.win(i) else { return };
        if b.sc.format() != format {
            w.stage = Stage::Cpu;
            w.fail_at = u64::MAX;
            return;
        }
        let (tw, th) = b.sc.size();
        let tex = Tex { t: b.tex, group: b.group, w: tw as i32, h: th as i32 };
        w.surf = Some(b.surf);
        w.sc = Some(b.sc);
        w.tex = Some(tex);
        w.full = true;
        w.undef = true;
        w.rb = None;
        if verify {
            let size = tw as u64 * th as u64 * 4;
            w.rb = dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Readback }).ok();
        }
        w.stage = Stage::Switch;
        self.gp_switch(i);
    }

    /// the whole framebuffer on the new swapchain, then the child shown: from now on the GPU
    fn gp_switch(&mut self, i: usize) {
        let t0 = plat::time_us();
        let mut s = core::mem::take(&mut self.m.gout);
        let mut failed = None;
        let mut again = false;
        let fb = self.uiw_parts(i as u32).map(|p| &p.rt.fb);
        if let (Dev::Ready(core), Some(Some(w)), Some(fb)) = (&s.dev, s.wins.get_mut(i), fb) {
            if w.tex.as_ref().is_none_or(|t| (t.w, t.h) != (fb.w, fb.h)) {
                w.stage = Stage::Cpu; // the size changed meanwhile
                w.stable_at = t0;
                again = true;
            } else {
                match upload(w, core, fb, None, &mut s.trim_at).and_then(|_| show(w, core, fb)) {
                    Ok(Shown::Done) => {
                        plat::win_gpu_child_show(w.pw, true);
                        plat::win_release_kept(w.pw);
                        w.stage = Stage::Gpu;
                        w.out.switches += 1;
                        w.out.error = None;
                    }
                    Ok(Shown::Wait) => w.pending = true,
                    Ok(Shown::NoArea) | Ok(Shown::Stale) => {
                        w.stage = Stage::Cpu;
                        w.stable_at = t0;
                    }
                    Err(e) => failed = Some(e),
                }
            }
        }
        self.m.gout = s;
        if let Some(e) = failed {
            self.gp_failed(i, e);
        } else if !again {
            let t1 = plat::time_us();
            let on = self.m.gout.wins.get(i).and_then(|w| w.as_ref()).is_some_and(|w| w.stage == Stage::Gpu);
            if on {
                self.m.gout.note(&format!("window {i}: switched to the swapchain ({:.2} ms)", ms(t0, t1)));
            }
        }
    }

    /// the window loop: finished jobs, frames still to show, new jobs
    pub(crate) fn gp_tick(&mut self) {
        if !self.m.gout.want {
            return;
        }
        self.gp_poll();
        let n = self.m.gout.wins.len();
        let now = plat::time_us();
        for w in self.m.gout.wins.iter_mut().flatten() {
            if w.sp.linger_at != 0 && now >= w.sp.linger_at {
                w.sp.linger_at = 0;
                for u in w.sp.upper.iter_mut().filter(|u| u.rects.is_empty()) {
                    u.tex = None;
                    u.store = Store::default();
                }
                self.m.gout.trim_at = now + TRIM_US; // (destroyed when the GPU has finished)
            }
        }
        for i in 0..n {
            let Some(w) = self.m.gout.wins[i].as_ref() else { continue };
            match w.stage {
                Stage::Switch => self.gp_switch(i),
                Stage::Gpu if w.pending && plat::time_us() >= w.last_present + MIN_PRESENT_US => self.gp_retry(i),
                _ => {}
            }
        }
        self.gp_jobs();
        let s = &mut self.m.gout;
        if s.trim_at != 0 && plat::time_us() >= s.trim_at {
            s.trim_at = 0;
            if let Dev::Ready(core) = &s.dev {
                let _ = core.dev.poll();
            }
        }
    }

    /// a frame that was not shown yet: window i's texture to the screen (a minimised window
    /// comes back: the whole frame first)
    fn gp_retry(&mut self, i: usize) {
        if self.m.gout.verify {
            self.gp_settle(i);
        }
        let mut s = core::mem::take(&mut self.m.gout);
        let mut r = Ok(Shown::Done);
        let fb = self.uiw_parts(i as u32).map(|p| &p.rt.fb);
        if let (Dev::Ready(core), Some(Some(w)), Some(fb)) = (&s.dev, s.wins.get_mut(i), fb) {
            if plat::win_state(w.pw) & plat::WST_MINIMIZED == 0 {
                let t0 = plat::time_us();
                r = if w.full { upload(w, core, fb, None, &mut s.trim_at) } else { Ok(()) }.and_then(|_| show(w, core, fb));
                plat::present_account(t0, false);
            }
        }
        self.m.gout = s;
        match r {
            Ok(Shown::Stale) => self.gp_leave(i, "stale"),
            Err(e) => self.gp_failed(i, e),
            _ => {}
        }
    }

    /// what should run now: the device, the swapchains of windows on the CPU path
    fn gp_jobs(&mut self) {
        let now = plat::time_us();
        let s = &mut self.m.gout;
        if plat::win_in_sizemove() {
            return;
        }
        match s.dev {
            Dev::None => {}
            Dev::Retry(at) if at <= now => s.dev = Dev::None,
            _ => {}
        }
        if matches!(s.dev, Dev::None) {
            if plat::win_remote_session() {
                s.dev = Dev::Retry(now + REMOTE_LOOK_US); // this session has no use for it (yet)
                return;
            }
            let Some((i, pw)) = s.wins.iter().enumerate().find_map(|(i, w)| w.as_ref().filter(|w| w.stage == Stage::Cpu).map(|w| (i, w.pw))) else {
                return;
            };
            let Some(nw) = plat::win_gpu_child(pw) else {
                s.dev = Dev::Never(String::from("no child window"));
                return;
            };
            let display = plat::win_display_adapter(pw);
            let desc = s.desc();
            let job = s.next_job;
            s.next_job += 1;
            let trace = s.trace;
            s.note("start: instance, device, swapchain on a worker");
            let ok = s.spawn(move || Done::Device { job, win: i, r: build_device(nw, pw, display, desc, trace) });
            if ok {
                s.dev = Dev::Starting(job);
                if let Some(w) = s.win(i) {
                    w.stage = Stage::Job;
                    w.job = job;
                }
            } else {
                s.dev = Dev::Never(String::from("no thread"));
            }
            return;
        }
        let Dev::Ready(core) = &s.dev else { return };
        let (dev, bgl, inst) = (core.dev.clone(), core.bgl.clone(), core.inst.clone());
        let n = s.wins.len();
        let desc = s.desc();
        for i in 0..n {
            let Some(w) = s.wins[i].as_ref() else { continue };
            if w.stage != Stage::Cpu || now < w.fail_at || now < w.stable_at + STABLE_US {
                continue;
            }
            let pw = w.pw;
            if plat::win_state(pw) & plat::WST_MINIMIZED != 0 || plat::win_size(pw) != w.size {
                continue;
            }
            let Some(nw) = plat::win_gpu_child(pw) else { continue };
            let job = s.next_job;
            s.next_job += 1;
            let Some(w) = s.wins[i].as_mut() else { continue };
            let old = w.sc.take();
            let surf = match w.surf.take() {
                Some(x) => x,
                None => match gpu::Surface::native(&inst, nw, pw) {
                    Ok(x) => x,
                    Err(_) => {
                        w.fail_at = u64::MAX;
                        continue;
                    }
                },
            };
            w.stage = Stage::Job;
            w.job = job;
            let desc = desc;
            let (d2, b2) = (dev.clone(), bgl.clone());
            s.note(&format!("window {i}: swapchain on a worker"));
            let ok = s.spawn(move || Done::Window { job, win: i, r: build_window(&d2, &b2, surf, old, &desc) });
            if !ok {
                if let Some(w) = s.win(i) {
                    w.stage = Stage::Cpu;
                    w.fail_at = u64::MAX;
                }
            }
        }
    }

    /// when the window loop should look again (plat::time_us; None: not until an event)
    pub(crate) fn gp_deadline(&self) -> Option<u64> {
        let s = &self.m.gout;
        if !s.want || s.wins.is_empty() {
            return None;
        }
        let now = plat::time_us();
        let mut d = u64::MAX;
        if s.jobs.running.load(Ordering::Acquire) != 0 {
            d = now + JOB_US;
        }
        match s.dev {
            Dev::None if s.wins.iter().flatten().next().is_some() => d = now,
            Dev::Retry(at) => d = d.min(at),
            _ => {}
        }
        if s.trim_at != 0 {
            d = d.min(s.trim_at);
        }
        let ready = matches!(s.dev, Dev::Ready(_));
        let moving = plat::win_in_sizemove();
        for w in s.wins.iter().flatten() {
            if w.sp.linger_at != 0 {
                d = d.min(w.sp.linger_at);
            }
            let minimised = plat::win_state(w.pw) & plat::WST_MINIMIZED != 0;
            match w.stage {
                Stage::Switch if !minimised => d = d.min(now + RETRY_US),
                Stage::Gpu if w.pending && !minimised => d = d.min((w.last_present + MIN_PRESENT_US).max(now + RETRY_US.min(1000))),
                Stage::Cpu if ready && !minimised && !moving && w.fail_at != u64::MAX => {
                    // (a size the frame has not reached yet: its event is on the way)
                    let settle = if plat::win_size(w.pw) == w.size { 1000 } else { 16_000 };
                    d = d.min((w.stable_at + STABLE_US).max(w.fail_at).max(now + settle));
                }
                _ => {}
            }
        }
        if d == u64::MAX { None } else { Some(d) }
    }

    /// wait for the jobs (the message loop pumps: a driver may send messages to the window while
    /// it builds a swapchain), at most ms
    fn gp_wait_jobs(&mut self, ms: u64) {
        let dl = plat::time_us() + ms * 1000;
        while self.m.gout.jobs.running.load(Ordering::Acquire) != 0 && plat::time_us() < dl {
            plat::wait(plat::time_us() + 1000);
        }
        self.gp_poll();
    }

    /// window i goes away: its swapchain and surface before its plat window
    pub(crate) fn gp_window_gone(&mut self, i: u32) {
        let i = i as usize;
        if self.m.gout.wins.get(i).and_then(|w| w.as_ref()).is_none() {
            return;
        }
        if self.m.gout.jobs.running.load(Ordering::Acquire) != 0 {
            self.gp_wait_jobs(3000);
        }
        if let Some(w) = self.m.gout.wins.get_mut(i) {
            *w = None;
        }
    }

    /// the window loop ended: everything of the GPU goes (a job still running after 2 s is left
    /// to the process's end)
    pub(crate) fn gp_shutdown(&mut self) {
        if !self.m.gout.want {
            return;
        }
        let t0 = plat::time_us();
        self.gp_wait_jobs(2000);
        let s = &mut self.m.gout;
        // the driver's end of a process takes 100 ms: the windows go first
        for w in s.wins.iter().flatten() {
            plat::win_show(w.pw, plat::PWS_HIDE);
        }
        // the process ends now: tearing the device down took 190 ms of the exit (measured). The
        // validation layer wants to see every object destroyed
        if env_on("jfgui_GPU_VALIDATE") {
            s.wins.clear();
            s.images.clear();
            s.dev = Dev::None;
        } else {
            core::mem::forget(core::mem::take(&mut s.wins));
            core::mem::forget(core::mem::take(&mut s.images));
            core::mem::forget(core::mem::replace(&mut s.dev, Dev::None));
        }
        s.want = false;
        s.note(&format!("shut down: {:.2} ms", ms(t0, plat::time_us())));
    }

    /// what window `win` (a UI window index, 0 = main) shows through
    pub fn gpu_output(&self, win: u32) -> GpuOutput {
        let s = &self.m.gout;
        let mut o = s.wins.get(win as usize).and_then(|w| w.as_ref()).map_or_else(GpuOutput::default, |w| {
            let mut o = w.out.clone();
            o.on = w.stage == Stage::Gpu;
            o.building = matches!(w.stage, Stage::Job | Stage::Switch);
            o.pending = w.pending;
            for u in &w.sp.upper {
                if let Some(t) = &u.tex {
                    o.layer_textures += !u.rects.is_empty() as u32;
                    o.layer_bytes += t.w as u64 * t.h as u64 * 4;
                }
                o.layer_store += u.store.px.capacity() as u64;
            }
            o.image_quads = w.sp.quads.len() as u32;
            o
        });
        o.wanted = s.want;
        o.image_textures = s.images.iter().filter(|e| e.tex.is_some()).count() as u32;
        o.image_bytes = s.image_bytes;
        o.image_uploads = s.image_uploads;
        o.image_hits = s.image_hits;
        o.image_evicted = s.image_evicted;
        match &s.dev {
            Dev::Ready(core) => {
                o.adapter = core.name.clone();
                o.reserved = core.dev.memory_reserved();
                o.present_mode = s.wins.get(win as usize).and_then(|w| w.as_ref()).and_then(|w| w.sc.as_ref()).map_or("", |sc| match sc.present_mode() {
                    PresentMode::Fifo => "fifo",
                    PresentMode::Mailbox => "mailbox",
                    PresentMode::Immediate => "immediate",
                });
            }
            Dev::Never(why) => {
                o.never = true;
                if o.error.is_none() {
                    o.error = Some(why.clone());
                }
            }
            _ => {}
        }
        o
    }

    /// (test aid) the device behaves as lost from now on: the next frame goes back to the CPU
    /// path, a new device follows
    pub fn gpu_test_lose(&mut self) {
        if let Dev::Ready(core) = &self.m.gout.dev {
            core.dev.test_lose();
        }
    }

    /// (test aid) the bytes image textures may take
    pub fn gpu_test_image_budget(&mut self, bytes: u64) {
        self.m.gout.image_budget = bytes;
    }

    /// the pixels of the images the GPU draws into the framebuffers (Ui::fb reads them then)
    pub fn gpu_settle(&mut self) {
        self.gp_settle_all();
    }

    // ------------------------------------------------------------------ layers and images

    /// render_live: does the live window keep its layers apart this frame?
    pub(crate) fn gp_split_wanted(&self) -> bool {
        let s = &self.m.gout;
        s.want && s.layers_on && s.wins.get(self.uiw_cur as usize).and_then(|w| w.as_ref()).is_some_and(|w| w.stage == Stage::Gpu)
    }

    /// can window i's frame be drawn apart: the texture fits, it can be seen, no inspector draws
    /// over the framebuffer
    fn gp_can_split(&self, i: usize) -> bool {
        let Some(Some(w)) = self.m.gout.wins.get(i) else { return false };
        let fb = &self.wp.rt.fb;
        !self.m.ins.on && !w.full && !w.undef && w.tex.as_ref().is_some_and(|t| (t.w, t.h) == (fb.w, fb.h)) && plat::win_state(w.pw) & plat::WST_MINIMIZED == 0
    }

    /// dirty_compute, after hashing (dirty.rs has W::imgs, W::top; cells: this frame, cells_prev:
    /// the last): which images the GPU draws in this frame, in which cells -> anything apart
    pub(crate) fn gp_plan_images(&mut self) -> bool {
        let i = self.uiw_cur as usize;
        if !self.gp_can_split(i) {
            return true; // gp_split_begin settles and flattens with the last frame's images
        }
        let (fw, fh) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        let opaque_dc = self.dc.opac == 255;
        let Ui { m, wp, .. } = self;
        let s = &mut m.gout;
        s.tick += 1;
        let (tick, now) = (s.tick, plat::time_us());
        let d = &wp.dirty;
        let (cols, rows) = (d.cols as usize, d.rows as usize);
        let total = cols * rows;
        let Some(Some(w)) = s.wins.get_mut(i) else { return false };
        let sp = &mut w.sp;
        if (sp.cols, sp.rows) != (cols, rows) || sp.hole.len() != total {
            (sp.cols, sp.rows) = (cols, rows);
            sp.hole = vec![0; total];
            sp.owed = vec![false; total];
            sp.stale = vec![false; total];
            sp.any_owed = false;
            sp.any_stale = false;
        }
        sp.hole.fill(0);
        let mut quads = core::mem::take(&mut sp.quads);
        quads.clear();
        // an image gone or drawn in another version takes its texture along
        let (cur, prev) = (d.cells.as_deref().unwrap_or(&[]), d.cells_prev.as_deref().unwrap_or(&[]));
        let mut k = 0;
        while k < s.images.len() {
            if s.images[k].weak.strong_count() == 0 {
                s.evict(k);
                s.images.remove(k);
            } else {
                k += 1;
            }
        }
        if s.images_on && opaque_dc {
            for (tag0, seen) in d.imgs.iter().enumerate() {
                let tag = tag0 as u8 + 1;
                let Some(Cmd::Image { x, y, w: iw, h: ih, sf, ver }) = wp.cmd.layers[0].get(seen.index as usize) else { continue };
                if sf.w <= 0 || sf.h <= 0 || sf.px.is_empty() || sf.fmt == FMT_A8 {
                    continue;
                }
                let key = Rc::as_ptr(sf) as usize;
                let vis = seen.vis;
                // the cells it is on top of, whole; what is new in this frame
                let (c0, c1) = (((vis[0] + CELL - 1) >> 5).max(0) as usize, (((vis[2].min(fw) + (if vis[2] >= fw { CELL - 1 } else { 0 })) >> 5).max(0) as usize).min(cols));
                let (r0, r1) = (((vis[1] + CELL - 1) >> 5).max(0) as usize, (((vis[3].min(fh) + (if vis[3] >= fh { CELL - 1 } else { 0 })) >> 5).max(0) as usize).min(rows));
                let mut changed = 0u64;
                let mut any = false;
                for r in r0..r1 {
                    for c in c0..c1 {
                        let k = r * cols + c;
                        if d.top.get(k) == Some(&tag) {
                            any = true;
                            if cur.get(k) != prev.get(k) {
                                changed += CELL_BYTES as u64;
                            }
                        }
                    }
                }
                if !any {
                    continue;
                }
                // another version of this surface: that one is over
                let mut q = 0;
                while q < s.images.len() {
                    if s.images[q].key == key && s.images[q].ver != *ver {
                        s.evict(q);
                        s.images.remove(q);
                    } else {
                        q += 1;
                    }
                }
                let at = match s.images.iter().position(|e| e.key == key) {
                    Some(at) => {
                        s.images[at].acc += changed;
                        at
                    }
                    None => {
                        if s.images.len() >= IMAGES_KEPT {
                            let Some(old) = (0..s.images.len()).min_by_key(|&k| s.images[k].tick) else { continue };
                            s.evict(old);
                            s.images.remove(old);
                        }
                        s.images.push(Image {
                            key,
                            weak: Rc::downgrade(sf),
                            ver: *ver,
                            bytes: sf.w as u64 * sf.h as u64 * 4,
                            acc: 0,
                            opaque: None,
                            tex: None,
                            load: None,
                            tick,
                            used_us: now,
                        });
                        s.images.len() - 1
                    }
                };
                s.images[at].tick = tick;
                s.images[at].used_us = now;
                // rent or buy: the redraws have cost what the upload costs
                let e = &s.images[at];
                if e.tex.is_none() && e.load.is_none() && e.acc >= e.bytes && e.bytes <= s.image_budget {
                    if let Dev::Ready(core) = &s.dev {
                        let lim = core.dev.limits().max_texture_2d as i32;
                        let opaque = e.opaque.unwrap_or_else(|| surface_opaque(sf));
                        s.images[at].opaque = Some(opaque);
                        if opaque && sf.w <= lim && sf.h <= lim {
                            let need = s.images[at].bytes;
                            while s.image_bytes + need > s.image_budget {
                                let old = (0..s.images.len()).filter(|&k| (s.images[k].tex.is_some() || s.images[k].load.is_some()) && s.images[k].tick != tick).min_by_key(|&k| s.images[k].tick);
                                let Some(old) = old else { break };
                                s.evict(old);
                            }
                            if s.image_bytes + need <= s.image_budget {
                                if let Dev::Ready(core) = &s.dev {
                                    // (a device gone or no memory: the CPU draws it, a failure of the
                                    // device shows with the next present)
                                    match image_texture(core, sf) {
                                        Ok(load) => {
                                            s.images[at].load = Some(load);
                                            s.image_bytes += need;
                                        }
                                        Err(_) => s.images[at].opaque = Some(false),
                                    }
                                }
                            }
                        }
                    }
                }
                // the next rows of a texture on the way up; the last ones make it usable
                if let (Dev::Ready(core), Some(load)) = (&s.dev, s.images[at].load.as_mut()) {
                    let t0 = plat::time_us();
                    let rows = ((IMAGE_STEP_BYTES / (sf.w as u64 * 4)).max(1) as i32).min(sf.h - load.row);
                    let before = if load.row == 0 { Layout::Undefined } else { Layout::Sampled };
                    match core.dev.queue().write_texture_rects(&load.t, &[[0, load.row, sf.w, rows]], &sf.px, sf.stride as u32, before, Layout::Sampled) {
                        Ok(()) => {
                            load.row += rows;
                            let done = load.row >= sf.h;
                            let bytes = sf.w as u64 * rows as u64 * 4;
                            s.trim_at = plat::time_us() + TRIM_US;
                            if let Some(Some(w)) = s.wins.get_mut(i) {
                                w.out.upload_bytes += bytes;
                            }
                            if done {
                                if let Some(load) = s.images[at].load.take() {
                                    s.images[at].tex = Some(Rc::new(ImageTex { _t: load.t, group: load.group }));
                                    s.image_uploads += 1;
                                }
                                s.note(&format!("image {} x {} as a texture, the last part {:.2} ms", sf.w, sf.h, ms(t0, plat::time_us())));
                            }
                        }
                        Err(_) => {
                            s.evict(at);
                            s.images[at].opaque = Some(false);
                        }
                    }
                }
                let Some(tex) = s.images[at].tex.clone() else { continue };
                if quads.len() >= TOP_BLOCKED as usize - 1 {
                    continue;
                }
                s.image_hits += 1;
                let Some(Some(w)) = s.wins.get_mut(i) else { continue };
                let sp = &mut w.sp;
                let q = quads.len() as u8 + 1;
                for r in r0..r1 {
                    for c in c0..c1 {
                        let k = r * cols + c;
                        if d.top.get(k) == Some(&tag) {
                            sp.hole[k] = q;
                        }
                    }
                }
                let mut rects = Vec::new();
                let hole = &sp.hole;
                cell_rects(&mut rects, cols, rows, fw, fh, |k| hole[k] == q);
                let step = [(((sf.w as u64) << 16) / *iw as u64) as u32, (((sf.h as u64) << 16) / *ih as u64) as u32];
                quads.push(Quad { tex, src: sf.clone(), x: *x, y: *y, w: *iw, h: *ih, step, rects });
            }
        }
        // what was not drawn for a long time gives its memory back
        if now >= s.look_at && !s.images.is_empty() {
            s.look_at = now + IMAGE_UNUSED_US / 4;
            for k in 0..s.images.len() {
                if now.saturating_sub(s.images[k].used_us) > IMAGE_UNUSED_US {
                    s.evict(k);
                }
            }
        }
        let Some(Some(w)) = s.wins.get_mut(i) else { return false };
        w.sp.quads = quads;
        !w.sp.quads.is_empty()
    }

    /// render_live, after dirty_compute: is this frame drawn layer by layer?  (Else it goes as
    /// without layers: the whole base texture holds the framebuffer's cells then.)
    pub(crate) fn gp_split_begin(&mut self, n: i32) -> bool {
        if !self.wp.dirty.split {
            return false;
        }
        let i = self.uiw_cur as usize;
        let can = self.gp_can_split(i);
        let upper_now = self.wp.cmd.layers[1..].iter().any(|l| !l.is_empty());
        let Some(w) = self.m.gout.win(i) else { return false };
        let sp = &mut w.sp;
        let kept = sp.active || !sp.quads.is_empty() || sp.any_stale;
        if !can {
            if kept {
                self.gp_flatten(i);
            }
            if let Some(w) = self.m.gout.win(i).filter(|_| upper_now) {
                w.sp.clean = false; // the layers go into the base texture with this frame
            }
            self.wp.dirty.busy = false;
            return false;
        }
        if !kept && !upper_now {
            sp.clean = true; // what a layer left in the base texture has changed and gone up
            self.wp.dirty.busy = false;
            return false;
        }
        let stale = sp.any_stale && sp.stale.iter().zip(sp.hole.iter()).any(|(&a, &h)| a && h == 0);
        if n == 0 && !stale {
            return false; // nothing changed
        }
        if self.wp.rt.present_n == 0 {
            self.wp.rt.present_n = -1;
        }
        true
    }

    /// render_live: the frame layer by layer (the module's header): from the lowest layer that
    /// changed in a cell upwards: back to the pixels below it, save them for each layer that
    /// touches the cell, draw the layer, upload the cell to the layer's texture
    pub(crate) fn gp_split_draw(&mut self, n: i32) {
        let icon_style = (self.style, self.color(T_ACCENT), self.theme_dark, self.color(T_TEXT_DISABLED), self.wp.rt.dpi as u32);
        let i = self.uiw_cur as usize;
        let Ui { m, wp, dc, text, icon_hook, drects, .. } = self;
        let s = &mut m.gout;
        let S { dev, wins, trim_at, .. } = s;
        let (Dev::Ready(core), Some(Some(w))) = (&*dev, wins.get_mut(i)) else {
            for l in wp.cmd.layers.iter() {
                render_layer(l, 0, dc, &mut wp.rt.fb, text, icon_hook, icon_style);
            }
            return;
        };
        let d = &wp.dirty;
        let (cols, rows) = (d.cols as usize, d.rows as usize);
        let total = cols * rows;
        let fb = &mut wp.rt.fb;
        let (fw, fh) = (fb.w, fb.h);
        let Win { sp, tex: base, out, pending, .. } = w;
        if sp.hole.len() != total {
            (sp.cols, sp.rows) = (cols, rows);
            sp.hole = vec![0; total];
            sp.owed = vec![false; total];
            sp.stale = vec![false; total];
        }
        let Split { upper, m: lowest, nr, rects, hole, owed, stale, quads, linger_at, .. } = sp;
        // ---- per cell: what changed, which layers touch it
        lowest.clear();
        lowest.resize(total, L_COUNT as u8);
        nr.clear();
        nr.resize(total, 0);
        let all = d.forced || (!sp.active && !sp.clean);
        let now: [&[u64]; L_COUNT] = core::array::from_fn(|l| d.through(true, l));
        let was: [&[u64]; L_COUNT] = core::array::from_fn(|l| d.through(false, l));
        for k in 0..total {
            let mut low = L_COUNT;
            if all || was[0].len() != total {
                low = 0;
            } else {
                for l in 0..L_COUNT {
                    if now[l][k] != was[l][k] {
                        low = l;
                        break;
                    }
                }
            }
            if stale[k] && hole[k] == 0 {
                low = 0; // the image the CPU left out is no longer drawn by the GPU
            }
            lowest[k] = low as u8;
            for l in 1..L_COUNT {
                if now[l][k] != now[l - 1][k] {
                    nr[k] |= 1 << l;
                }
            }
        }
        // explicit damage: its cells from the base up
        if n >= 0 {
            for r in drects.iter().take(n as usize).skip(d.n0.max(0) as usize) {
                let (c0, c1) = ((r[0].max(0) >> 5) as usize, (((r[0] + r[2] + CELL - 1) >> 5).max(0) as usize).min(cols));
                let (r0, r1) = ((r[1].max(0) >> 5) as usize, (((r[1] + r[3] + CELL - 1) >> 5).max(0) as usize).min(rows));
                for row in r0..r1 {
                    lowest[row * cols + c0..row * cols + c1.max(c0)].fill(0);
                }
            }
        }
        // ---- the layers' textures: a new one takes all of its cells
        let mut err = None;
        for (u, l) in upper.iter_mut().zip(1..) {
            let bit = 1u8 << l;
            let mut bb = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
            for k in (0..total).filter(|&k| nr[k] & bit != 0) {
                let b = cell_box(k, cols, fw, fh);
                bb = [bb[0].min(b[0]), bb[1].min(b[1]), bb[2].max(b[2]), bb[3].max(b[3])];
            }
            if bb[0] >= bb[2] {
                if u.tex.is_some() && !u.rects.is_empty() {
                    *linger_at = plat::time_us() + LINGER_US;
                }
                continue;
            }
            let area = (bb[2] - bb[0]) as i64 * (bb[3] - bb[1]) as i64;
            let fits = u.tex.as_ref().is_some_and(|t| {
                bb[0] >= t.ox && bb[1] >= t.oy && bb[2] <= t.ox + t.w && bb[3] <= t.oy + t.h
                    && (t.w as i64 * t.h as i64) <= (area * LAYER_SHRINK).max((LAYER_STEP * LAYER_STEP) as i64)
            });
            if !fits {
                u.tex = None;
                match layer_texture(core, bb, fw, fh) {
                    Ok(t) => u.tex = Some(t),
                    Err(e) => err = Some(e),
                }
                for k in (0..total).filter(|&k| nr[k] & bit != 0) {
                    lowest[k] = lowest[k].min(l);
                }
            }
        }
        // ---- stage by stage
        let stride = fb.stride as usize;
        for l in 0..L_COUNT {
            let bit = 1u8 << l;
            if l > 0 {
                for k in 0..total {
                    let low = lowest[k] as usize;
                    if low > l {
                        continue;
                    }
                    let b = cell_box(k, cols, fw, fh);
                    if low == l {
                        // what lay below this layer: kept by the lowest layer that was here, else
                        // the framebuffer holds it already
                        if let Some(u) = upper[l - 1..].iter().find(|u| u.store.has(k)) {
                            cell_load(fb, b, u.store.get(k));
                            owed[k] = false;
                        }
                    }
                    if nr[k] & bit != 0 {
                        let st = &upper[l - 1].store;
                        if !st.has(k) && st.free.is_empty() && st.px.len() + CELL_BYTES > st.px.capacity() {
                            // (the layer's other new cells too: no reallocation for each of them)
                            let n = (k..total).filter(|&c| nr[c] & bit != 0 && !st.has(c)).count();
                            upper[l - 1].store.reserve(n);
                        }
                        if owed[k] && hole[k] != 0 {
                            if let Some(q) = quads.get(hole[k] as usize - 1) {
                                quad_blit(fb, q, b);
                            }
                            owed[k] = false;
                        }
                        cell_save(fb, b, upper[l - 1].store.put(k, total));
                    }
                }
            }
            let is_base = l == 0;
            cell_rects(rects, cols, rows, fw, fh, |k| (lowest[k] as usize) <= l && (if is_base { hole[k] == 0 } else { nr[k] & bit != 0 }));
            for r in rects.iter() {
                dc.clip_push(r[0], r[1], r[2], r[3]);
                render_layer(&wp.cmd.layers[l], 0, dc, fb, text, icon_hook, icon_style);
                dc.clip_pop();
            }
            let bytes: u64 = rects.iter().map(|r| r[2] as u64 * r[3] as u64 * 4).sum();
            if is_base {
                out.base_px += bytes / 4;
                for k in 0..total {
                    if lowest[k] == 0 {
                        let h = hole[k] != 0;
                        owed[k] = h;
                        stale[k] = h;
                    }
                }
                if err.is_none() {
                    if let Some(t) = base.as_ref() {
                        if let Err(e) = core.dev.queue().write_texture_rects(&t.t, rects, &fb.px, stride as u32, Layout::Sampled, Layout::Sampled) {
                            err = Some(e);
                        }
                    }
                }
                out.upload_base += bytes;
            } else {
                let u = &mut upper[l - 1];
                if err.is_none() {
                    if let Some(t) = u.tex.as_mut() {
                        let before = if t.fresh { Layout::Undefined } else { Layout::Sampled };
                        match core.dev.queue().write_texture_rects_at(&t.t, rects, (t.ox, t.oy), &fb.px, stride as u32, before, Layout::Sampled) {
                            Ok(()) => t.fresh = t.fresh && rects.is_empty(),
                            Err(e) => err = Some(e),
                        }
                    }
                }
                for k in 0..total {
                    if nr[k] & bit == 0 {
                        u.store.drop_cell(k);
                    }
                }
                if u.store.used == 0 {
                    u.store.empty();
                }
                cell_rects(&mut u.rects, cols, rows, fw, fh, |k| nr[k] & bit != 0);
            }
            out.upload_bytes += bytes;
            if bytes > BIG_UPLOAD {
                *trim_at = plat::time_us() + TRIM_US;
            }
        }
        sp.any_owed = sp.owed.iter().any(|&v| v);
        sp.any_stale = sp.stale.iter().any(|&v| v);
        sp.active = sp.upper.iter().any(|u| !u.rects.is_empty());
        sp.clean = true;
        sp.drawn = true;
        sp.err = err;
        *pending = true;
        out.split_frames += 1;
        wp.dirty.busy = sp.active || !sp.quads.is_empty();
    }

    /// render_live's end: its time; n: the rectangles a frame without layers drew
    pub(crate) fn gp_frame_done(&mut self, t0: u64, n: i32, split: bool) {
        let i = self.uiw_cur as usize;
        let (fw, fh) = (self.wp.rt.fb.w as u64, self.wp.rt.fb.h as u64);
        let px = if split {
            0
        } else if n < 0 {
            fw * fh
        } else {
            self.drects.iter().take(n as usize).map(|r| r[2].max(0) as u64 * r[3].max(0) as u64).sum()
        };
        if let Some(w) = self.m.gout.win(i) {
            w.out.render_us += plat::time_us().saturating_sub(t0);
            w.out.base_px += px;
        }
    }

    /// the images the GPU draws of window i into its framebuffer, where it lacks them
    fn gp_settle(&mut self, i: usize) {
        let Some(Some(w)) = self.m.gout.wins.get_mut(i) else { return };
        if !w.sp.any_owed {
            return;
        }
        let sp = &mut w.sp;
        let (mut owed, hole, quads) = (core::mem::take(&mut sp.owed), core::mem::take(&mut sp.hole), core::mem::take(&mut sp.quads));
        let (cols, rows) = (sp.cols, sp.rows);
        sp.any_owed = false;
        if let Some(p) = self.uiw_parts_mut(i as u32) {
            let fb = &mut p.rt.fb;
            let (fw, fh) = (fb.w, fb.h);
            let mut rects = Vec::new();
            for (q, quad) in quads.iter().enumerate() {
                let tag = q as u8 + 1;
                cell_rects(&mut rects, cols, rows, fw, fh, |k| owed[k] && hole[k] == tag);
                for r in &rects {
                    quad_blit(fb, quad, [r[0], r[1], r[0] + r[2], r[1] + r[3]]);
                }
            }
        }
        owed.fill(false);
        let Some(Some(w)) = self.m.gout.wins.get_mut(i) else { return };
        (w.sp.owed, w.sp.hole, w.sp.quads) = (owed, hole, quads);
    }

    /// every window's framebuffer holds its whole frame
    pub(crate) fn gp_settle_all(&mut self) {
        for i in 0..self.m.gout.wins.len() {
            self.gp_settle(i);
        }
    }

    /// back to one texture: the framebuffer is whole again and goes up with the next look; layers
    /// and images come back with the next change
    fn gp_flatten(&mut self, i: usize) {
        self.gp_settle(i);
        let s = &mut self.m.gout;
        let trim = plat::time_us() + TRIM_US;
        let mut freed = false;
        if let Some(Some(w)) = s.wins.get_mut(i) {
            freed = w.sp.active;
            w.sp.reset();
            w.full = true;
            w.pending = true;
        }
        if freed {
            s.trim_at = trim;
        }
        if let Some(p) = self.uiw_parts_mut(i as u32) {
            p.dirty.busy = false;
        }
    }

    /// (--verify-dirty) the frame was repaired: it goes up as a whole
    pub(crate) fn gp_repaired(&mut self) {
        let i = self.uiw_cur as usize;
        if self.m.gout.wins.get(i).and_then(|w| w.as_ref()).is_some_and(|w| w.stage == Stage::Gpu) {
            self.gp_flatten(i);
        }
    }

    // ------------------------------------------------------------------ scripts

    /// a headless run shows its windows through `gpu on`
    pub(crate) fn gp_scripted(&self) -> bool {
        self.m.gout.script
    }

    /// the real window of UI window `cur` in a script (made at its first frame, of the
    /// framebuffer's size), messages pumped; the script's own events are not queued now (a frame
    /// follows process_events)
    pub(crate) fn gp_script_pw(&mut self, cur: u32) -> i32 {
        let i = cur as usize & (super::consts::WIN_MAX - 1);
        let (fw, fh) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        let s = &mut self.m.gout;
        let mut pw = s.script_pw[i];
        if pw < 0 {
            let off = 40 + 48 * i as i32;
            pw = plat::win_create("gpu output test", 320, 240, plat::PWF_SYSFRAME | plat::PWF_RESIZABLE, plat::PW_NOOWNER, off, off);
            if pw < 0 {
                return -1;
            }
            s.script_pw[i] = pw;
            plat::win_test(plat::WT_NOANIM, pw, [0; 4]);
            plat::win_show(pw, plat::PWS_SHOWNA);
        }
        if plat::win_size(pw) != (fw, fh) {
            plat::win_set_rect(pw, plat::PW_KEEP, plat::PW_KEEP, fw, fh);
        }
        script_pump();
        pw
    }

    fn gp_scoped(&self) -> usize {
        self.uiw_cur as usize
    }

    /// the script waits: the window loop's work, the real windows' messages
    fn gp_script_wait(&mut self, ms: u64) -> bool {
        let dl = plat::time_us() + ms * 1000;
        loop {
            script_pump();
            self.gp_tick();
            let s = &self.m.gout;
            let building = s.jobs.running.load(Ordering::Acquire) != 0;
            let all = s.wins.iter().flatten().all(|w| w.stage == Stage::Gpu);
            if matches!(s.dev, Dev::Never(_)) {
                return false;
            }
            if all && !building && !s.wins.is_empty() {
                return true;
            }
            if plat::time_us() >= dl {
                return false;
            }
            plat::wait(plat::time_us() + 1000);
        }
    }

    fn gp_skip(&mut self, why: &str) {
        self.m.gout.skip = true;
        plat::write_stdout(format!("SKIP gpu: {why}\n").as_bytes());
    }

    /// the script command `gpu` (the module's header)
    pub(crate) fn sc_gpu(&mut self, app: &mut dyn App, a: &[u8]) {
        if word_eq(a, "on").is_some() {
            self.m.gout.script = true;
            self.m.gout.setup();
            self.m.gout.want = true;
            self.m.gout.verify = true;
            self.m.gout.trace = env_on("jfgui_GPU_TRACE");
            self.m.gout.skip = false;
            self.dirty_force_full();
            self.invalidate();
            self.run_passes(app);
            self.render();
            self.present();
            if !self.gp_script_wait(20_000) {
                let why = match &self.m.gout.dev {
                    Dev::Never(w) => w.clone(),
                    _ => String::from("the swapchain did not come"),
                };
                if matches!(self.m.gout.dev, Dev::Never(_)) {
                    self.gp_skip(&why);
                } else {
                    self.fail(why.as_bytes());
                }
            }
            return;
        }
        if word_eq(a, "off").is_some() {
            self.m.gout.script = false;
            self.gp_wait_jobs(3000);
            let s = &mut self.m.gout;
            s.wins.clear();
            s.images.clear();
            s.image_bytes = 0;
            s.dev = Dev::None;
            s.want = false;
            for pw in s.script_pw.iter_mut() {
                if *pw >= 0 {
                    plat::win_destroy(*pw);
                    *pw = -1;
                }
            }
            script_pump();
            return;
        }
        if !self.m.gout.script {
            self.fail(b"gpu on first");
            return;
        }
        if word_eq(a, "wait").is_some() {
            if self.m.gout.skip {
                return;
            }
            let (t, _) = super::runtime::parse_int(next_word(a));
            self.render();
            self.present();
            if !self.gp_script_wait(if t > 0 { t as u64 } else { 20_000 }) {
                self.fail(b"not on the swapchain");
            }
            return;
        }
        if word_eq(a, "lose").is_some() {
            self.gpu_test_lose();
            return;
        }
        if let Some(n) = word_eq(a, "budget") {
            let (v, _) = super::runtime::parse_int(skip_ws(n));
            self.gpu_test_image_budget(v.max(0) as u64);
            return;
        }
        if word_eq(a, "print").is_some() {
            let o = self.gpu_output(self.gp_scoped() as u32);
            plat::write_stdout(format!("gpu: {o:?}\n").as_bytes());
            return;
        }
        let Some(r) = word_eq(a, "expect") else {
            self.fail(b"unknown gpu command");
            return;
        };
        let r = skip_ws(r);
        if self.m.gout.skip {
            return;
        }
        self.render();
        self.present();
        let i = self.gp_scoped();
        if word_eq(r, "gpu").is_some() || word_eq(r, "cpu").is_some() {
            let want_gpu = word_eq(r, "gpu").is_some();
            let on = self.gpu_output(i as u32).on;
            if on != want_gpu {
                self.fail(if on { b"on the swapchain" } else { b"on the CPU path" });
            }
        } else if word_eq(r, "same").is_some() {
            self.gp_expect_same();
        } else if word_eq(r, "window").is_some() {
            self.gp_expect_pixels(i, false);
        } else if word_eq(r, "screen").is_some() {
            self.gp_expect_pixels(i, true);
        } else if let Some(n) = word_eq(r, "upload") {
            let (max, _) = super::runtime::parse_int(skip_ws(n));
            let bytes = self.m.gout.win(i).map_or(0, |w| core::mem::take(&mut w.out.upload_bytes));
            if bytes > max as u64 {
                self.fail(format!("{bytes} bytes uploaded").as_bytes());
            }
        } else if let Some(n) = word_eq(r, "base") {
            let (max, _) = super::runtime::parse_int(skip_ws(n));
            let bytes = self.m.gout.win(i).map_or(0, |w| core::mem::take(&mut w.out.upload_base));
            if bytes > max as u64 {
                self.fail(format!("{bytes} bytes uploaded to the base texture").as_bytes());
            }
        } else if let Some(n) = word_eq(r, "raster") {
            let (max, _) = super::runtime::parse_int(skip_ws(n));
            let px = self.m.gout.win(i).map_or(0, |w| core::mem::take(&mut w.out.base_px));
            if px > max as u64 {
                self.fail(format!("{px} pixels of the base layer drawn").as_bytes());
            }
        } else if let Some(n) = word_eq(r, "layers") {
            let (want, _) = super::runtime::parse_int(skip_ws(n));
            let have = self.gpu_output(i as u32).layer_textures;
            if have != want as u32 {
                self.fail(format!("{have} layer textures").as_bytes());
            }
        } else if let Some(n) = word_eq(r, "images") {
            let (want, _) = super::runtime::parse_int(skip_ws(n));
            let have = self.gpu_output(i as u32).image_textures;
            if have != want as u32 {
                self.fail(format!("{have} image textures").as_bytes());
            }
        } else {
            self.fail(b"unknown gpu expectation");
        }
    }

    // every window on the GPU: present once more, read back, compare; no frame differed since the
    // last check
    fn gp_expect_same(&mut self) {
        self.gp_settle_all();
        let mut s = core::mem::take(&mut self.m.gout);
        let mut msg = None;
        let mut any = false;
        for i in 0..s.wins.len() {
            let fb = self.uiw_parts(i as u32).map(|p| &p.rt.fb);
            if let (Dev::Ready(core), Some(Some(w)), Some(fb)) = (&s.dev, s.wins.get_mut(i), fb) {
                if w.stage != Stage::Gpu {
                    continue;
                }
                any = true;
                let frames = w.out.verified;
                for _ in 0..50 {
                    match show(w, core, fb) {
                        Ok(Shown::Wait) => {
                            plat::wait(plat::time_us() + 4_000); // no image free now
                        }
                        Ok(_) => break,
                        Err(e) => {
                            msg = Some(format!("{e}"));
                            break;
                        }
                    }
                }
                if w.out.verified == frames && msg.is_none() {
                    msg = Some(format!("window {i}: no frame presented"));
                }
                if w.out.verify_bad != 0 {
                    msg = Some(format!("window {i}: {} of {} presented images differ", w.out.verify_bad, w.out.verified));
                    w.out.verify_bad = 0;
                }
            }
        }
        if !any && msg.is_none() {
            msg = Some(String::from("no window on the swapchain"));
        }
        self.m.gout = s;
        script_pump();
        if let Some(m) = msg {
            self.fail(m.as_bytes());
        }
    }

    // the window's DC (the CPU path) or the composed screen against the framebuffer
    fn gp_expect_pixels(&mut self, i: usize, screen: bool) {
        self.gp_settle_all();
        let pw = self.m.gout.script_pw[i & (super::consts::WIN_MAX - 1)];
        if pw < 0 {
            self.fail(b"no window");
            return;
        }
        // what was presented reaches the screen with the next compositions
        for _ in 0..12 {
            script_pump();
            plat::wait(plat::time_us() + 8_000);
        }
        let Some(fb) = self.uiw_parts(i as u32).map(|p| &p.rt.fb) else { return };
        let (w, h) = (fb.w, fb.h);
        let px = if screen {
            plat::win_get_rect(pw).and_then(|r| plat::win_test_screen(r[0], r[1], w, h))
        } else {
            plat::win_test_image(pw).filter(|x| x.0 == w && x.1 == h).map(|x| x.2)
        };
        let Some(px) = px else {
            self.fail(b"no picture");
            return;
        };
        let mut bad = 0u64;
        let mut first = (0, 0);
        let row = w as usize * 4;
        for y in 0..h as usize {
            let a = &px[y * row..y * row + row];
            let b = &fb.px[y * fb.stride as usize..y * fb.stride as usize + row];
            for (x, (p, q)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
                // (the screen: the system rounds the window's corners)
                let corner = screen && (x < 8 || x + 8 >= w as usize) && (y < 8 || y + 8 >= h as usize);
                if p[..3] != q[..3] && !corner {
                    if bad == 0 {
                        first = (x, y);
                    }
                    bad += 1;
                }
            }
        }
        if bad != 0 {
            let total = w as u64 * h as u64;
            if screen && plat::win_test(plat::WT_FROM_POINT, pw, [first.0 as i32, first.1 as i32, 0, 0]) != 1 {
                plat::write_stdout(format!("note: gpu expect screen: {bad} of {total} pixels differ: another window lies on top of ours\n").as_bytes());
            } else {
                self.fail(format!("{bad} pixels differ, first at {}, {}", first.0, first.1).as_bytes());
            }
        }
    }
}

// the real windows of a script: their messages, their events dropped (a wake-up of a task stays)
fn script_pump() {
    plat::wait(0);
    let mut keep: Vec<plat::Event> = vec![];
    while let Some(e) = plat::evq_pop() {
        if e.typ == plat::E_WAKE {
            keep.push(e);
        }
    }
    for e in keep {
        plat::evq_push(e);
    }
}
