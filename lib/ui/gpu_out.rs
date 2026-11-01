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
// A lost device or any other failure: every window back to the CPU path (the framebuffer holds the
// frame), a new device after 2 s, 4 s, ... 5 min (jfgui_GPU_RETRY_MS: the first wait).  No Vulkan, no
// usable adapter or surface: the CPU path for good, silently.
//
// Environment: jfgui_GPU=0|1 (off, on; default: DEFAULT_ON), jfgui_GPU_VERIFY=1 (every presented
// image read back and compared with the framebuffer), jfgui_GPU_TRACE=1 (stderr: the stages and
// their times); lib/gpu's jfgui_GPU_PRESENT, jfgui_GPU_ADAPTER, jfgui_GPU_VALIDATE, ...
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
//   gpu lose               the device behaves as lost from now on (Device::test_lose)
//   gpu print              the counters

use alloc::boxed::Box;
use alloc::format;
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
use super::{App, GpuOutput, Ui};
use gfx::Surface;

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

/// the shaders, compiled from gpu_out.wgsl (uigputest --test shaders compares them with what
/// wgsl::compile makes of it, --gen-shaders writes them again)
mod spv {
    include!("gpu_out_spv.rs");
}

/// the embedded shaders of the output and their source (uigputest: the embedding check)
pub const GPU_OUT_WGSL: &str = include_str!("gpu_out.wgsl");
pub const GPU_OUT_VS: &[u32] = spv::VS;
pub const GPU_OUT_FS: &[u32] = spv::FS;

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
}

struct Core {
    inst: Instance,
    dev: Device,
    bgl: BindGroupLayout,
    pipe: RenderPipeline,
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
    let layout = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 })?;
    let pipe = dev.create_render_pipeline(&RenderPipelineDesc {
        layout: &layout,
        vertex: ShaderStage::new(&vs, "vs"),
        fragment: Some(ShaderStage::new(&fs, "fs")),
        buffers: &[],
        topology: Topology::Triangles,
        cull: Cull::None,
        front_cw: false,
        targets: &[ColorTarget { format, blend: None, write_mask: 0xF }],
        depth: None,
    })?;
    let t5 = plat::time_us();
    if trace {
        plat::write_stderr(format!(
            "gpu: worker: instance {:.2} ms, adapter {:.2}, device {:.2}, swapchain + texture {:.2}, pipeline {:.2} ({})\n",
            ms(t0, t1), ms(t1, t2), ms(t2, t3), ms(t3, t4), ms(t4, t5), adapter.info().name).as_bytes());
    }
    let name = adapter.info().name.clone();
    Ok((Core { inst, dev, bgl, pipe, format, name }, built))
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
    e.draw(3, 1, 0, 0);
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
        let s = &mut self.m.gout;
        let Dev::Ready(core) = &s.dev else { return false };
        let Some(Some(w)) = s.wins.get_mut(i) else { return false };
        if w.stage != Stage::Gpu {
            return false;
        }
        let fb = &self.wp.rt.fb;
        let fits = w.tex.as_ref().is_some_and(|t| (t.w, t.h) == (fb.w, fb.h));
        if !fits {
            self.gp_leave(i, "the size changed");
            return true;
        }
        if plat::win_state(pw) & plat::WST_MINIMIZED != 0 {
            w.full = true; // nobody sees it: the whole frame when the window comes back
            w.pending = true;
            return true;
        }
        let rects = if n < 0 { None } else { Some(&self.drects[..n as usize]) };
        let soon = t0 < w.last_present + MIN_PRESENT_US && !s.script;
        let r = upload(w, core, fb, rects, &mut s.trim_at).and_then(|_| if soon { Ok(Shown::Wait) } else { show(w, core, fb) });
        match r {
            Ok(Shown::Done) | Ok(Shown::Wait) => {}
            Ok(Shown::NoArea) => w.full = true,
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
                w.out.error = Some(format!("{e}"));
            }
        }
        let s = &mut self.m.gout;
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
            s.dev = Dev::None;
        } else {
            core::mem::forget(core::mem::take(&mut s.wins));
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
            o
        });
        o.wanted = s.want;
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
        } else {
            self.fail(b"unknown gpu expectation");
        }
    }

    // every window on the GPU: present once more, read back, compare; no frame differed since the
    // last check
    fn gp_expect_same(&mut self) {
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
