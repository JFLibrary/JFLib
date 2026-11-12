// gpu_out -- UI windows on the screen through a swapchain (Windows; Linux in programs against glibc;
// Android; the browser: docs/GPU.md "Ausgabe der UI", "Ausgabe unter Linux", "Ausgabe unter Android",
// "Ausgabe im Browser")
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
//   gp_window_show(i, show)      before window i is hidden or shown again
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
// A hidden window goes back to Cpu too: a swapchain's image does not come back with its window (white
// until the next present, measured on Windows, and a frame that did not change presents nothing).  What
// the CPU path holds does come back, the swapchain follows as after a resize; nothing is built or
// switched for a window nobody sees (unseen: the UI hid it, or the system with its minimised owner).
//
// The swapchain lives on a child window (plat win_gpuout.rs): once a swapchain has presented on a
// window GDI stays invisible there, also after the swapchain is gone.
//
// Linux: the same code.  A Swapchain there is a set of dma-bufs lib/gpu makes itself (vk_dmabuf.rs,
// no VkSurface: that would want libwayland or libxcb), plat attaches them with its own protocol code
// (linux_gpuout.rs); shared memory and dma-bufs take turns on one wl_surface or X window, so there is
// no child and win_gpu_child only says what the display accepts.  The differences this code sees: a
// frame reaches the window after the GPU has finished it (flush, busy: the loop looks every ms while
// one is on its way), the next one starts when the display has answered the last (next_look: no
// polling, the answer ends the wait; the frame stays in the texture and its newest state is drawn
// then), the compositor gets the frame's rectangles (damage), a worker may not ask the window system
// (Surface::track_size).  What cannot be read back is said by the tests: no client reads a Wayland
// window, Xwayland's GetImage of a flipped window lags one frame.
//
// Android: the same code again.  The host's View with its Bitmap is the CPU path and stays below; the
// swapchain lives on a SurfaceView above it (plat android_gpu.rs), the analogue of Windows' child
// window: the surface exists while the View is visible, shows nothing until its first present and
// then covers the Bitmap.  Going back: the frame to the Bitmap, the SurfaceView away, and with it the
// surface and swapchain (win_gpu_child_show).  The system can also take the surface (the activity
// stops): gp_surface_gone, before the host's callback returns.  win_gpu_child says None for a while
// (the surface is on its way, a tool window or popup shows on the canvas below): win_gpu_child_later,
// the host wakes the loop.
//
// The browser: the same code once more, through WebGPU (lib/gpu web_*.rs, web/gpu.js).  The UI runs in a
// worker in all three bundles and WebGPU's objects stay on that thread: there are no job threads, the jobs
// run on this thread when the browser has answered (S::web_start: requestAdapter and requestDevice
// answer later, the app goes on through the CPU path meanwhile).  The page's #screen is the CPU path and
// stays below; a canvas above it, an OffscreenCanvas here, is the analogue of the SurfaceView: it shows
// nothing until its first frame, then covers #screen; each period on the GPU has a new one (plat
// web_gpu.rs).  Before a device is used, a small picture through all three pipelines is read back and
// compared: a browser whose WebGPU draws wrongly never shows a frame (S::web_probe).  After the switch the
// page looks at what reached the canvas (watchGpuCanvas in web/loader.js): nothing there, and the CPU path
// takes over for good (plat::win_gpu_child_blank).  Reading back comes
// later here: jfgui_GPU_VERIFY compares when the browser has answered (verify_poll).  The shaders go as
// WGSL (gpu_out.wgsl), no SPIR-V in the bundle.  Runs with ?agent=1 stay on the CPU path like scripts,
// unless ?gpu=1 asks; no script command here.
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
// Canvases (G3c, canvas.rs).  While a canvas's view moves and its content stays, its content comes
// from tiles: TILE x TILE px textures of the content at one scale (a Level), drawn once by
// CanvasContent::draw_view on this thread (at most FILL_US per frame) and kept under a budget with
// LRU eviction.  The render pass draws them with the transform: by copy (fs) at the level's scale
// and whole pixels away, else bilinear (fs_tile).  In the cells the canvas ends up on top of (also
// those it only partly covers, with the scissor at its edge) the CPU neither draws nor uploads it:
// its command's hash stands only for where it lies (dirty.rs), Split::owed and Split::stale as for
// images, gp_settle runs its Draw before anybody reads the framebuffer.  Cells with something over
// it and cells without their tiles go the CPU's way: its Draw, the overlay, the upload.  SETTLE_US
// after the last move gp_tick asks for a frame, the canvas is no longer apart, the CPU draws what
// the view shows: at rest the picture is the CPU path's.  A frame that changes the content (new
// CanvasDesc::ver, ui.canvas_gpu_damage) ends the gesture at once and costs no tile; a tile is only
// drawn when a view moves over it.  Nothing of it exists before the first move, and nothing SETTLE
// + CANVAS_UNUSED_US after the last one.  Right-to-left windows draw canvases on the CPU.
//
// Views (G5e1, gpu_view.rs).  A view of GPU content (ui.gpu_view: a 3D scene between widgets) draws its frames
// on this device into a texture of its size (gp_views_begin, at render time before the cells are hashed: a new
// ver, size or device draws one, nothing else does).  Where the window is on its swapchain and the view ends up
// on top it is a quad like an image's (QuadSrc::View, drawn by the shader that copies: `fs` at its place), its
// command's hash stands for where it lies as a canvas apart's (dirty.canv), and a new frame counts as a canvas's
// move: its cells owed and stale, the cells with something over it and under a layer drawn again by the CPU,
// whose Draw reads the frame back (the only way the CPU gets it: a copy to a read-back buffer, waited for).
// VIEW_REST_US after the last frame gp_tick settles the window once: at rest the framebuffer holds the picture.
// Anywhere else the view's command hashes its frame's number, so each new frame is drawn by the CPU, which reads
// it back.  A view's frame is bound to the display (G5e3, docs/GPU.md "3D-Ansicht"): while its window is on its
// swapchain with a frame not shown yet (Win::pending), a new ver waits; gp_views_wake asks for a render once the
// window's frame has been shown, at the latest VIEW_HOLD_US after the first frame waited (a display that never
// answers does not stop the view).  Without this a view drawn as fast as the app can starved the compositor on the
// same GPU (Intel under Linux: no frame presented at all).  AF_GPU_3D makes the device at level Voll on the fastest
// adapter; without the flag views work on the Basis device of the frugal adapter.  Without an output in this run (not
// wanted, never possible) views make a device of their own (gp_view_device).  A lost device detaches every content
// (gp_views_gone) and attaches it to the next one.
//
// A lost device or any other failure: every window back to the CPU path (the framebuffer holds the
// frame), a new device after 2 s, 4 s, ... 5 min (jfgui_GPU_RETRY_MS: the first wait).  No Vulkan, no
// usable adapter or surface, a display that takes or imports no dma-bufs: the CPU path for good,
// silently.  plat::gpu_out_state tells the platform how the first start went (Android keeps it across
// runs: a driver that killed the program does not get a third chance).
//
// Environment: jfgui_GPU=0|1 (off, on; default: DEFAULT_ON, DEFAULT_ON_LINUX, DEFAULT_ON_BROWSER; Android:
// GalleryActivity always sets it, GPU_DEFAULT there; the browser: ?gpu=0|1), jfgui_GPU_VERIFY=1 (every presented
// image read back and compared with the framebuffer), jfgui_GPU_TRACE=1 (stderr: the stages and
// their times), jfgui_GPU_LAYERS=0 (one texture per window: no layers, no images),
// jfgui_GPU_IMAGES=0 (no images), jfgui_GPU_IMAGE_MB=n (the images' budget), jfgui_GPU_CANVAS=0 (no
// canvas uses tiles), jfgui_GPU_CANVAS_CPU=a,b (the canvases named so draw on the CPU),
// jfgui_GPU_CANVAS_PAN=a,b (they only pan from tiles), jfgui_GPU_CANVAS_ZOOM=a,b (they also zoom from
// tiles, whatever the widget chose), jfgui_GPU_CANVAS_MB=n (the tiles' budget),
// jfgui_GPU_CANVAS_SETTLE_MS=n (the time until the CPU's picture follows a move), jfgui_GPU_OUTPUT=0 (the
// windows through the CPU path, the GPU stays for views, which read every frame back), jfgui_GPU_VIEWS=0 (no view
// is drawn by the output's render pass: every frame read back into the framebuffer);
// lib/gpu's jfgui_GPU_PRESENT, jfgui_GPU_ADAPTER, jfgui_GPU_VALIDATE, ...
//
// Script command `gpu` (headless runs stay on the CPU path without it; on other targets (a static
// Linux program, the browser), or without a GPU, it prints a SKIP once and does nothing):
//   gpu on [fast]          a real window per UI window (shown without activation) with
//                          the output, every presented image compared with the framebuffer (fast:
//                          not compared, for measurements), a render after every input edge; waits
//                          for the swapchain; a canvas stays apart until `gpu settle` or `same`
//   gpu off                windows closed
//   gpu wait [MS]          until the scoped windows are on the swapchain (after `size`, `gpu lose`)
//   gpu expect gpu|cpu     the main window's stage
//   gpu settle             every canvas goes back to the CPU's picture now
//   gpu expect same        every canvas settled, every window presents once more: the image read
//                          back = the framebuffer byte for byte (x byte: 255), and no frame differed
//                          since the last check (a canvas's scaled tiles count as GpuOutput::canvas_diff_*)
//   gpu expect screen      the composed screen at the window = the framebuffer (another window on
//                          top of ours is a note, not a failure); X11: what the server holds for the
//                          window; Wayland: a note, no client can read it
//   gpu expect window      the window's DC = the framebuffer (the CPU path's pixels); Linux as `screen`
//   gpu expect upload N    at most N bytes uploaded since the last check
//   gpu expect base N      at most N bytes of them to the base texture since the last check
//   gpu expect raster N    at most N pixels of the base layer drawn since the last check
//   gpu expect layers N    N layers above the base have a texture now
//   gpu expect images N    N image textures exist
//   gpu expect apart N     N canvases of the window come from tiles now
//   gpu expect tiles N     N canvas tiles exist
//   gpu expect fills N     at most N tiles were drawn since the last check
//   gpu expect nodiff      no pixel of a canvas's tiles differed from the CPU's since the last
//                          `same` or `nodiff`
//   gpu budget BYTES       the images' budget
//   gpu tilebudget BYTES   the tiles' budget
//   gpu shot FILE          the image the window presents now (read back: also what a gesture's
//                          tiles show; `shot` writes the framebuffer)
//   gpu lose               the device behaves as lost from now on (Device::test_lose)
//   gpu hold MS            real time passes, the windows stay as they are (a look from outside)
//   gpu print              the counters

#[cfg(not(target_arch = "wasm32"))]
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
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
use super::canvas::{CanvasGpu, CanvasView};
use super::cmd::{render_layer, Cmd, DrawRef, Gfx};
use super::consts::*;
use super::dirty::TOP_BLOCKED;
use super::gpu_view::{Entry, GpuContent, Own, Target, ViewTex, GPU_VIEW_FORMAT};
use super::{App, GpuOutput, Mods, Ui};
use gfx::{Dc, Surface, FMT_A8, FMT_XRGB};
use text::Text;

/// the output without jfgui_GPU in the environment: on, jfgui_GPU=0 turns it off (docs/GPU.md
/// "Ausgabe der UI" says what it costs)
const DEFAULT_ON: bool = true;
/// the same on Linux, measured apart: a weak GPU may answer differently (docs/GPU.md "Ausgabe unter
/// Linux"); this line decides
const DEFAULT_ON_LINUX: bool = true;
/// the same in the browser, where the numbers differ again (docs/GPU.md "Ausgabe im Browser"): the one
/// switch for it.  A run with ?agent=1 stays on the CPU path unless ?gpu=1 asks: such runs read #screen.
/// Phones and tablets stay there too until one has been tested: web/loader.js sets jfgui_GPU=0
const DEFAULT_ON_BROWSER: bool = true;
/// the same on macOS and iOS (Metal, AP6): off, the user's decision of 2026-11-11 after the measurements (docs/APPLE.md
/// AP6d); jfgui_GPU=1 and a script's `gpu on` turn it on
const DEFAULT_ON_APPLE: bool = false;
/// the present mode (FIFO when the surface lacks it; jfgui_GPU_PRESENT overrides it): a burst of
/// frames shows its newest within a refresh, where FIFO lags 3 .. 5 refreshes behind (docs/GPU.md
/// "Ausgabe der UI")
const PRESENT: PresentMode = PresentMode::Mailbox;
/// swapchain images: as fast as three in every measurement, a window's size less GPU memory
const IMAGES: u32 = 2;
/// frames closer together than this wait for gp_tick: 250 per second are plenty for a window (us).
/// (The browser: every frame waits for the gp_tick of its turn; the next tick may be 33 ms away.)
const MIN_PRESENT_US: u64 = if cfg!(target_arch = "wasm32") { 0 } else { 4_000 };
/// a window's size is settled when it did not change for this long (us)
const STABLE_US: u64 = 100_000;
/// a frame that found no free image is tried again after this long (us)
const RETRY_US: u64 = 2_000;
/// while a job runs the loop looks this often (us)
const JOB_US: u64 = 8_000;
/// (a driver compiles shaders on this thread; on Linux a stack has exactly this size, on Windows
/// this is what is committed at the start: docs/GPU.md "Linux")
#[cfg(not(target_arch = "wasm32"))]
const WORKER_STACK: usize = if cfg!(windows) { 2 << 20 } else { 8 << 20 };
/// the first wait before a new device (ms), its longest
const RETRY_FIRST_MS: u64 = 2_000;
const RETRY_MAX_MS: u64 = 300_000;
/// an upload above this keeps a large staging buffer for a while: gp_tick gives it back
const BIG_UPLOAD: u64 = 128 << 10;
const TRIM_US: u64 = 1_200_000;
/// a Remote Desktop session is looked at again after this long (us)
const REMOTE_LOOK_US: u64 = 10_000_000;
/// (scripts) a window whose size differs from its frame for this long stays so (us)
const KEPT_US: u64 = 500_000;
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
/// a canvas's tile: its content, a texel of its neighbours around it (fs_tile reads them)
const TILE: i32 = 256;
const TILE_BORDER: i32 = 1;
const TILE_TEX: i32 = TILE + 2 * TILE_BORDER;
const TILE_BYTES: u64 = TILE_TEX as u64 * TILE_TEX as u64 * 4;
/// the bytes canvas tiles may take (jfgui_GPU_CANVAS_MB)
const CANVAS_BUDGET: u64 = 128 << 20;
/// a frame draws tiles for at most this long; what is left goes the CPU's way in this frame (us)
const FILL_US: u64 = 5_000;
/// this long after a canvas's last move the CPU draws its picture (us)
const SETTLE_US: u64 = 180_000;
/// the tiles go this long after the last move (us)
const CANVAS_UNUSED_US: u64 = 30_000_000;
/// a level serves a view scaled by this much at most, then the next one is drawn
const K_MIN: f64 = 0.55;
const K_MAX: f64 = 1.9;
/// whole pixels away, up to this much of a pixel: a copy
const PHASE_TOL: f64 = 1.0 / 64.0;
/// the scales kept per canvas
const LEVELS_MAX: usize = 6;
/// (scripts) a window shows once more, 4 ms between the tries: longer than a display takes to answer
const SHOW_TRIES: u32 = 300;
/// (views) this long after a view's last frame the window is settled: its frame read back into the framebuffer (us)
const VIEW_REST_US: u64 = 180_000;
/// (views, G5e3) a new frame of a view waits for its window's last frame to be shown at most this long, then it is
/// drawn anyway (us): a display that never answers costs the view its rate, not its picture
const VIEW_HOLD_US: u64 = 100_000;

/// the shaders, compiled from gpu_out.wgsl (uigputest --test shaders compares them with what
/// wgsl::compile makes of it, --gen-shaders writes them again); the browser takes the WGSL itself
#[cfg(not(target_arch = "wasm32"))]
mod spv {
    include!("gpu_out_spv.rs");
}

/// the embedded shaders of the output and their source (uigputest: the embedding check)
pub const GPU_OUT_WGSL: &str = include_str!("gpu_out.wgsl");
#[cfg(not(target_arch = "wasm32"))]
pub const GPU_OUT_VS: &[u32] = spv::VS;
#[cfg(not(target_arch = "wasm32"))]
pub const GPU_OUT_FS: &[u32] = spv::FS;
#[cfg(not(target_arch = "wasm32"))]
pub const GPU_OUT_FS_IMAGE: &[u32] = spv::FS_IMAGE;
#[cfg(not(target_arch = "wasm32"))]
pub const GPU_OUT_FS_TILE: &[u32] = spv::FS_TILE;

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

/// a tile's texture (Rc: a window's last frame keeps drawing an evicted one until its next)
struct TileTex {
    t: Texture,
    group: BindGroup,
    fresh: core::cell::Cell<bool>, // nothing uploaded yet (Layout::Undefined)
}

struct Tile {
    tex: Rc<TileTex>,
    tick: u64,
}

/// a canvas's tiles at one scale: grid pixel g shows the content point (g - f) / scale
struct TileLevel {
    scale: f64,
    fx: f64,
    fy: f64,
    tiles: BTreeMap<(i32, i32), Tile>,
}

/// a canvas the output has seen
struct Canvas {
    win: usize,
    id: u32,
    ver: u32,
    /// the view of the last frame and of the one before it
    view: CanvasView,
    prev: CanvasView,
    known: bool,
    damaged: bool,
    /// its view comes from tiles, until this time; it moved in this frame
    apart: bool,
    until: u64,
    moved: bool,
    levels: Vec<TileLevel>,
}

/// a tile in a window's frame: its texture, its box in the window (x0, y0, x1, y1), the shader's
/// place
struct TileDraw {
    tex: Rc<TileTex>,
    b: [i32; 4],
    place: [i32; 4],
}

/// what a quad draws: an image with its steps (Dc::blit_scaled), or a canvas's tiles (copied or
/// scaled; loose: a difference from the CPU's picture is no failure) with its command's Draw, its box and
/// what of it shows (x0, y0, x1, y1: a scrolling parent clips it, the frame's clip stack is gone later)
enum QuadSrc {
    Image { tex: Rc<ImageTex>, src: Rc<Surface>, w: i32, h: i32, step: [u32; 2] },
    Canvas { f: DrawRef, w: i32, h: i32, vis: [i32; 4], tiles: Vec<TileDraw>, copy: bool, loose: bool },
    /// (G5e1) a view's frame: its target, one to one at the quad's place; its Draw reads the frame back
    View { tex: Rc<ViewTex>, f: DrawRef, w: i32, h: i32, vis: [i32; 4] },
}

/// an image or a canvas drawn by the GPU in a window's frame: its place, the rectangles of its cells
struct Quad {
    x: i32,
    y: i32,
    src: QuadSrc,
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
    // ---- per cell: a canvas covers it only partly (the CPU draws the rest); a canvas moved
    // there in this frame; a canvas moved in this frame
    part: Vec<bool>,
    redraw: Vec<bool>,
    moved: bool,
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
    sc: Option<Swapchain>, // also on the CPU path: the next build reuses it (oldSwapchain)
    surf: Option<gpu::Surface>, // (after sc: it goes first)
    tex: Option<Tex>,
    rb: Option<Buffer>, // jfgui_GPU_VERIFY: the presented image
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    rb_px: Vec<u8>,
    rb_keep: Option<Vec<u8>>, // `gpu shot`: the image as presented
    full: bool,    // the texture lacks pixels: the whole frame goes up next
    undef: bool,   // the texture is new (Layout::Undefined)
    pending: bool, // the texture holds pixels that were not presented
    hidden: bool,  // the UI hid it: no swapchain is built or shown for it
    test_hold: bool, // (Ui::gpu_test_hold) its frames are not shown, as if the display did not answer
    apart_before: bool, // the last frame had layers, images or canvases apart
    last_present: u64,
    job: u32,      // the job in flight
    lost: bool,    // (Android) the surface went while the job built on it
    size: (i32, i32),
    stable_at: u64,
    fail_at: u64,  // no new swapchain before this time
    fail_ms: u64,
    out: GpuOutput,
    sp: Split,
    #[cfg(target_arch = "wasm32")]
    verify_wait: Vec<Verify>,
}

/// (the browser) a presented image on its way back, the framebuffer of that frame beside it
#[cfg(target_arch = "wasm32")]
struct Verify {
    rb: Buffer,
    fb: Surface,
    loose: Vec<[i32; 4]>,
    /// (G5e2) where the output's pass drew views: not compared.  Their frame reaches the framebuffer here only when its
    /// read-back has come, later than the image that shows it (gpu_view.rs); at rest the framebuffer has it, which the
    /// tests compare with the screen
    late: Vec<[i32; 4]>,
}

/// (the browser) a new device's small picture on its way back
#[cfg(target_arch = "wasm32")]
struct Probe {
    job: u32,
    win: usize,
    core: Core,
    built: Built,
    rb: Buffer,
    px: Vec<u8>,
    at: u64,
}

struct Core {
    inst: Instance,
    dev: Device,
    bgl: BindGroupLayout,
    pipe: RenderPipeline,
    pipe_image: RenderPipeline,
    pipe_tile: RenderPipeline,
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
    script_asked: [(i32, i32); super::consts::WIN_MAX],
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
    canvases_on: bool,
    canvases: Vec<Canvas>,
    canvas_cpu: String,
    canvas_pan: String,
    canvas_zoom: String,
    canvas_budget: u64,
    canvas_settle_us: u64,
    canvas_unused_us: u64,
    tile_bytes: u64,
    tile_free: Vec<Rc<TileTex>>,
    tile_sf: Surface,
    tile_fills: u64,
    tile_fill_us: u64,
    tile_evicted: u64,
    canvas_settles: u64,
    canvas_free_at: u64,
    said_ok: bool, // plat::gpu_out_state
    tile_marked: u64, // `gpu expect fills`
    diff_marked: u64, // `gpu expect nodiff`
    /// (views) counts the devices views can draw on (the output's, their own): a new one takes their contents along
    dev_gen: u32,
    /// (views) the output's render pass may draw them (jfgui_GPU_VIEWS); the app shows them (AF_GPU_3D)
    views_on: bool,
    want3d: bool,
    #[cfg(target_arch = "wasm32")]
    web_inst: Option<Instance>, // adapter and device asked for, on their way
    #[cfg(target_arch = "wasm32")]
    probe: Option<Probe>,
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
            // (the browser: one thread, S::spawn runs a job where it is called; the Arc is the other targets')
            #[allow(clippy::arc_with_non_send_sync)]
            jobs: Arc::new(Jobs { done: SpinLock::new(Vec::new()), running: AtomicU32::new(0) }),
            threads: Vec::new(),
            next_job: 1,
            retry_ms: 0,
            ready_at: 0,
            trim_at: 0,
            t0: 0,
            script_pw: [-1; super::consts::WIN_MAX],
            script_asked: [(0, 0); super::consts::WIN_MAX],
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
            canvases_on: false,
            canvases: Vec::new(),
            canvas_cpu: String::new(),
            canvas_pan: String::new(),
            canvas_zoom: String::new(),
            canvas_budget: CANVAS_BUDGET,
            canvas_settle_us: SETTLE_US,
            canvas_unused_us: CANVAS_UNUSED_US,
            tile_bytes: 0,
            tile_free: Vec::new(),
            tile_sf: Surface::default(),
            tile_fills: 0,
            tile_fill_us: 0,
            tile_evicted: 0,
            canvas_settles: 0,
            canvas_free_at: 0,
            said_ok: false,
            tile_marked: 0,
            diff_marked: 0,
            dev_gen: 0,
            views_on: true,
            want3d: false,
            #[cfg(target_arch = "wasm32")]
            web_inst: None,
            #[cfg(target_arch = "wasm32")]
            probe: None,
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
        // (a run with ?agent=1 reads #screen, as scripts read the framebuffer of the CPU path)
        _ if cfg!(target_arch = "wasm32") => DEFAULT_ON_BROWSER && !plat::args().iter().any(|a| a == "--agent"),
        _ if cfg!(target_vendor = "apple") => DEFAULT_ON_APPLE,
        _ => if cfg!(windows) { DEFAULT_ON } else { DEFAULT_ON_LINUX }, // (Android: the host says)
    };
    // (jfgui_GPU_OUTPUT=0: the windows through the CPU path, the GPU stays for views)
    let output = !plat::env("jfgui_GPU_OUTPUT").is_some_and(|v| v == "0" || v.eq_ignore_ascii_case("off"));
    on && output && !plat::win_remote_session()
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
        self.canvases_on = self.layers_on && !off("jfgui_GPU_CANVAS");
        self.canvas_cpu = plat::env("jfgui_GPU_CANVAS_CPU").unwrap_or_default();
        self.canvas_pan = plat::env("jfgui_GPU_CANVAS_PAN").unwrap_or_default();
        self.canvas_zoom = plat::env("jfgui_GPU_CANVAS_ZOOM").unwrap_or_default();
        if let Some(mb) = plat::env("jfgui_GPU_CANVAS_MB").and_then(|v| v.parse::<u64>().ok()) {
            self.canvas_budget = mb << 20;
        }
        if let Some(ms) = plat::env("jfgui_GPU_CANVAS_SETTLE_MS").and_then(|v| v.parse::<u64>().ok()) {
            self.canvas_settle_us = ms * 1000;
        }
        self.views_on = self.layers_on && !off("jfgui_GPU_VIEWS");
    }

    /// how this canvas may use the GPU (the variables say what the widget's choice is not)
    fn canvas_mode(&self, name: &str, gpu: CanvasGpu) -> CanvasGpu {
        let named = |list: &str| list.split(',').any(|n| n == name);
        if named(&self.canvas_cpu) {
            CanvasGpu::Cpu
        } else if named(&self.canvas_zoom) {
            CanvasGpu::Zoom
        } else if named(&self.canvas_pan) {
            CanvasGpu::Pan
        } else {
            gpu
        }
    }

    /// every tile goes, with the device or with the time: the textures die when no frame draws them
    fn canvases_gone(&mut self) {
        for c in self.canvases.iter_mut() {
            c.levels.clear();
            c.apart = false;
            c.moved = false;
        }
        self.tile_free.clear();
        self.tile_bytes = 0;
        self.tile_sf = Surface::default();
        self.canvas_free_at = 0;
    }

    /// a texture for a new tile: a free one, a new one, the one of the tile drawn longest ago
    /// (not one of this frame) -> None: the budget is full of this frame's tiles
    fn tile_texture(&mut self, tick: u64) -> Option<Rc<TileTex>> {
        if self.tile_bytes + TILE_BYTES <= self.canvas_budget {
            if let Some(k) = self.tile_free.iter().position(|t| Rc::strong_count(t) == 1) {
                return Some(self.tile_free.swap_remove(k));
            }
        }
        // (the budget may have become smaller: as many as it takes)
        while self.tile_bytes + TILE_BYTES > self.canvas_budget {
            if self.tile_free.pop().is_some() {
                self.tile_bytes -= TILE_BYTES; // (dies with the frame that still draws it)
                continue;
            }
            let mut old: Option<(usize, usize, (i32, i32), u64)> = None;
            for (ci, c) in self.canvases.iter().enumerate() {
                for (li, l) in c.levels.iter().enumerate() {
                    for (key, t) in l.tiles.iter() {
                        if t.tick != tick && old.is_none_or(|o| t.tick < o.3) {
                            old = Some((ci, li, *key, t.tick));
                        }
                    }
                }
            }
            let (ci, li, key, _) = old?;
            self.canvases[ci].levels[li].tiles.remove(&key)?;
            self.tile_evicted += 1;
            self.tile_bytes -= TILE_BYTES;
        }
        let Dev::Ready(core) = &self.dev else { return None };
        let t = core.dev.create_texture(&TextureDesc::d2(TILE_TEX as u32, TILE_TEX as u32, Format::Bgra8Unorm, TextureUsage::COPY_DST | TextureUsage::SAMPLED)).ok()?;
        let group = core.dev.create_bind_group(&BindGroupDesc {
            layout: &core.bgl,
            entries: &[BindEntry { binding: 0, resource: Resource::Texture(t.view()) }],
        }).ok()?;
        self.tile_bytes += TILE_BYTES;
        Some(Rc::new(TileTex { t, group, fresh: core::cell::Cell::new(true) }))
    }

    /// the tiles of a level go to the free ones
    fn tiles_free(&mut self, tiles: BTreeMap<(i32, i32), Tile>) {
        self.tile_free.extend(tiles.into_values().map(|t| t.tex));
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

    /// (views, G5e3) window i is on its swapchain and holds a frame that was not shown yet: a new frame of its views waits
    /// (what "shown" means per platform: docs/GPU.md "3D-Ansicht", G5e3)
    fn unshown(&self, i: usize) -> bool {
        self.wins.get(i).and_then(|w| w.as_ref()).is_some_and(|w| w.stage == Stage::Gpu && w.pending)
    }

    fn win(&mut self, i: usize) -> Option<&mut Win> {
        self.wins.get_mut(i).and_then(|w| w.as_mut())
    }

    /// (the browser) no threads: WebGPU's objects stay on this one, and nothing below waits
    #[cfg(target_arch = "wasm32")]
    fn spawn(&mut self, f: impl FnOnce() -> Done + 'static) -> bool {
        let d = f();
        self.jobs.done.lock().push(d);
        true
    }

    /// (the browser) what S::spawn's job on a thread does elsewhere: adapter and device first, which
    /// answer later, then the page's canvas, then everything else in one go on this thread, then the small
    /// picture -> true: nothing more to do for the device in this look
    #[cfg(target_arch = "wasm32")]
    fn web_start(&mut self) -> bool {
        if self.probe.is_some() {
            self.web_probe();
            return true;
        }
        if !matches!(self.dev, Dev::None | Dev::Starting(WEB_PREPARING)) {
            return matches!(self.dev, Dev::Starting(_));
        }
        let Some((i, pw)) = self.wins.iter().enumerate().find_map(|(i, w)| w.as_ref().filter(|w| w.stage == Stage::Cpu).map(|w| (i, w.pw))) else {
            return true;
        };
        if self.web_inst.is_none() {
            self.note("start: adapter and device asked for");
            match Instance::new() {
                Ok(inst) => {
                    self.web_inst = Some(inst);
                    self.dev = Dev::Starting(WEB_PREPARING);
                }
                Err(e) => {
                    self.note(&format!("no device: {e}"));
                    if let Some(w) = self.win(i) {
                        w.out.error = Some(format!("{e}"));
                    }
                    self.dev = Dev::Never(format!("{e}"));
                    return true;
                }
            }
        }
        let inst = self.web_inst.as_ref().map(|x| x.prepare());
        match inst {
            Some(Ok(true)) => {}
            Some(Err(e)) => {
                self.note(&format!("no device: {e}"));
                self.web_inst = None;
                if let Some(w) = self.win(i) {
                    w.out.error = Some(format!("{e}"));
                }
                self.dev = if matches!(e, GpuError::Failed(_)) {
                    let at = plat::time_us() + self.retry_ms * 1000;
                    self.retry_ms = (self.retry_ms * 2).min(RETRY_MAX_MS);
                    Dev::Retry(at)
                } else {
                    Dev::Never(format!("{e}")) // no adapter: the CPU path for good
                };
                return true;
            }
            _ => return true,
        }
        // (a tool window on #screen: it can wait; the page's canvas is on its way)
        let Some(nw) = plat::win_gpu_child(pw) else { return true };
        let job = self.next_job;
        self.next_job += 1;
        let desc = self.desc();
        let trace = self.trace;
        let t0 = plat::time_us();
        let r = build_device(self.web_inst.take(), nw, pw, None, desc, trace, self.want3d);
        self.note(&format!("device, pipelines, swapchain: {:.2} ms on this thread", ms(t0, plat::time_us())));
        if let Some(w) = self.win(i) {
            w.stage = Stage::Job;
            w.job = job;
        }
        self.dev = Dev::Starting(job);
        match r.and_then(|(core, built)| {
            let (rb, px) = probe_start(&core)?;
            Ok(Probe { job, win: i, core, built, rb, px, at: plat::time_us() })
        }) {
            Ok(p) => self.probe = Some(p),
            Err(e) => self.jobs.done.lock().push(Done::Device { job, win: i, r: Err(e) }),
        }
        true
    }

    /// (the browser) has the small picture come back as it should?
    #[cfg(target_arch = "wasm32")]
    fn web_probe(&mut self) {
        let Some(p) = &self.probe else { return };
        let mut got = vec![0u8; p.px.len()];
        let r = match p.rb.read(0, &mut got) {
            Err(GpuError::NotReady) if plat::time_us() < p.at + PROBE_US => return,
            Err(GpuError::NotReady) => Err(GpuError::Timeout),
            Err(e) => Err(e),
            Ok(()) if got == p.px => Ok(()),
            Ok(()) => Err(GpuError::Unsupported("the browser's WebGPU draws other pixels")),
        };
        let Some(p) = self.probe.take() else { return };
        self.note(&format!("the small picture: {:?}, {:.2} ms", r, ms(p.at, plat::time_us())));
        let (job, win) = (p.job, p.win);
        self.jobs.done.lock().push(Done::Device { job, win, r: r.map(|_| (p.core, p.built)) });
    }

    /// the worker threads: one per job, joined when none runs (they end by themselves)
    #[cfg(not(target_arch = "wasm32"))]
    fn spawn(&mut self, f: impl FnOnce() -> Done + Send + 'static) -> bool {
        let jobs = self.jobs.clone();
        jobs.running.fetch_add(1, Ordering::AcqRel);
        let j = jobs.clone();
        // (jfgui_GPU_WORKER_KB: to see what a driver needs, docs/GPU.md "Linux")
        let stack = plat::env("jfgui_GPU_WORKER_KB").and_then(|v| v.parse::<usize>().ok()).map_or(WORKER_STACK, |k| k << 10);
        let t = plat::gpu_thread_spawn(stack, Box::new(move || {
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

/// (the browser) S::dev while adapter and device are on their way
#[cfg(target_arch = "wasm32")]
const WEB_PREPARING: u32 = 0;
/// (the browser) a new device's small picture comes back within this time, or the device is not used (us)
#[cfg(target_arch = "wasm32")]
const PROBE_US: u64 = 5_000_000;

/// (the browser) a small picture through all three pipelines and the push constants, read back (the
/// rows of 256 bytes WebGPU wants) -> the buffer, the bytes it must hold
#[cfg(target_arch = "wasm32")]
fn probe_start(core: &Core) -> gpu::Result<(Buffer, Vec<u8>)> {
    const N: u32 = 8;
    const ROW: usize = 256;
    let dev = &core.dev;
    let src = dev.create_texture(&TextureDesc::d2(N, N, Format::Bgra8Unorm, TextureUsage::COPY_DST | TextureUsage::SAMPLED))?;
    let group = dev.create_bind_group(&BindGroupDesc {
        layout: &core.bgl,
        entries: &[BindEntry { binding: 0, resource: Resource::Texture(src.view()) }],
    })?;
    let target = dev.create_texture(&TextureDesc::d2(N, N, core.format, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC))?;
    let rb = dev.create_buffer(&BufferDesc { size: (ROW * N as usize) as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?;
    // texels B, G, R and a fourth byte that means nothing; what must come out of them
    let mut px = vec![0u8; (N * N * 4) as usize];
    let mut want = vec![0u8; ROW * N as usize];
    for y in 0..N as usize {
        for x in 0..N as usize {
            let t = [(x * 30 + 1) as u8, (y * 30 + 2) as u8, ((x ^ y) * 17 + 3) as u8, 0x55];
            px[(y * N as usize + x) * 4..][..4].copy_from_slice(&t);
            let rgba = core.format == Format::Rgba8Unorm;
            let c = if rgba { [t[2], t[1], t[0], 255] } else { [t[0], t[1], t[2], 255] };
            want[y * ROW + x * 4..][..4].copy_from_slice(&c);
        }
    }
    dev.queue().write_texture_rects(&src, &[[0, 0, N as i32, N as i32]], &px, N * 4, Layout::Undefined, Layout::Sampled)?;
    let mut e = dev.encoder()?;
    e.begin_render(&RenderPassDesc {
        color: &[ColorAttachment { view: target.view(), load: Load::Discard, clear: [0.0; 4], store: true, before: Layout::Undefined, after: Layout::CopySrc }],
        depth: None,
    });
    let half = N / 2;
    e.set_render_pipeline(&core.pipe);
    e.set_bind_group(0, &group);
    e.push_constants(0, &[0i32; 4]);
    e.set_scissor(0, 0, N, half);
    e.draw(3, 1, 0, 0);
    e.set_render_pipeline(&core.pipe_image);
    e.set_bind_group(0, &group);
    e.push_constants(0, &[0i32, 0, 65536, 65536]);
    e.set_scissor(0, half, half, half);
    e.draw(3, 1, 0, 0);
    e.set_render_pipeline(&core.pipe_tile);
    e.set_bind_group(0, &group);
    e.set_scissor(half, half, half, half);
    e.draw(3, 1, 0, 0);
    e.end_render();
    e.copy_texture_to_buffer(TextureRegion::mip(&target, 0), &rb, DataLayout { offset: 0, bytes_per_row: ROW as u32, rows_per_image: 0 });
    dev.queue().submit([e.finish()?])?;
    Ok((rb, want))
}

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

/// (AF_GPU_3D) the adapters for windows and 3D views together, the fastest first: discrete before integrated, none
/// the output would not take either (a CPU or virtual one; the browser's one unless it is software).  jfgui_GPU_ADAPTER
/// picks one as in Instance::adapter
fn fast_adapters(inst: &Instance) -> gpu::Result<Vec<Adapter>> {
    if plat::env("jfgui_GPU_ADAPTER").is_some_and(|s| !s.is_empty()) {
        return inst.adapter().map(|a| vec![a]);
    }
    let list: Vec<Adapter> = inst.adapters()?.into_iter().filter(|a| a.info().missing.is_none() && gpu::low_power_rank(a.info().kind, false).is_some()).collect();
    if list.is_empty() { Err(GpuError::NoAdapter) } else { Ok(list) }
}

/// instance, the adapter for this window, device, pipeline, the first swapchain.  want3d (AF_GPU_3D): the fastest
/// adapter at level Voll (3D views share the device), else the frugal one at level Basis
#[allow(clippy::too_many_arguments)]
fn build_device(prepared: Option<Instance>, nw: plat::NativeWindow, pw: i32, display: Option<(u32, u32)>, desc: SwapchainDesc, trace: bool, want3d: bool) -> gpu::Result<(Core, Built)> {
    let t0 = plat::time_us();
    #[cfg(target_os = "android")]
    if env_on("jfgui_GPU_TEST_DIE") {
        rt::os::exit(3); // (android_gpu.py: a driver that kills the program)
    }
    let inst = match prepared {
        Some(inst) => inst, // (the browser: it has answered)
        None => Instance::new()?,
    };
    let t1 = plat::time_us();
    let surf = gpu::Surface::native(&inst, nw, pw)?;
    let list: Vec<Adapter> = if want3d { fast_adapters(&inst)? } else { inst.adapters_low_power(display)? };
    let adapter = list.into_iter().find(|a| a.can_present(&surf)).ok_or(GpuError::NoAdapter)?;
    let t2 = plat::time_us();
    let dev = adapter.create_device(&DeviceDesc { max_level: if want3d { Level::Voll } else { Level::Basis } })?;
    let t3 = plat::time_us();
    let bgl = dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::Texture, stages: Stages::FRAGMENT }])?;
    let built = build_window(&dev, &bgl, surf, None, &desc)?;
    let t4 = plat::time_us();
    let format = built.sc.format();
    #[cfg(not(target_arch = "wasm32"))]
    let (vs, fs, fs_image, fs_tile) = (ShaderModule::from_spirv(&dev, spv::VS)?, ShaderModule::from_spirv(&dev, spv::FS)?,
                                       ShaderModule::from_spirv(&dev, spv::FS_IMAGE)?, ShaderModule::from_spirv(&dev, spv::FS_TILE)?);
    // (the browser takes the WGSL as it is)
    #[cfg(target_arch = "wasm32")]
    let (vs, fs, fs_image, fs_tile) = {
        let m = ShaderModule::from_wgsl(&dev, GPU_OUT_WGSL)?;
        (m.clone(), m.clone(), m.clone(), m)
    };
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
    let pipe_tile = pipeline(&fs_tile, "fs_tile")?;
    let t5 = plat::time_us();
    if trace {
        plat::write_stderr(format!(
            "gpu: worker: instance {:.2} ms, adapter {:.2}, device {:.2}, swapchain + texture {:.2}, pipeline {:.2} ({})\n",
            ms(t0, t1), ms(t1, t2), ms(t2, t3), ms(t3, t4), ms(t4, t5), adapter.info().name).as_bytes());
    }
    let name = adapter.info().name.clone();
    Ok((Core { inst, dev, bgl, pipe, pipe_image, pipe_tile, format, name }, built))
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
    if let Some(sc) = &w.sc {
        sc.damage(if rects.is_some() && before == Layout::Sampled { Some(rs) } else { None });
    }
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
    // (the browser: rows of 256 bytes, a buffer per frame, the answer comes later: verify_poll)
    #[cfg(target_arch = "wasm32")]
    let rb_frame = match &w.rb {
        Some(_) => Some(core.dev.create_buffer(&BufferDesc { size: (fb.w as u64 * 4 + 255 & !255) * fb.h as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?),
        None => None,
    };
    #[cfg(target_arch = "wasm32")]
    let rb_now = rb_frame.as_ref();
    #[cfg(not(target_arch = "wasm32"))]
    let rb_now = w.rb.as_ref();
    let row_bytes = if cfg!(target_arch = "wasm32") { (fb.w as u32 * 4 + 255) & !255 } else { 0 };
    let mut e = core.dev.encoder()?;
    let after = if rb_now.is_some() { Layout::CopySrc } else { Layout::Present };
    e.begin_render(&RenderPassDesc {
        color: &[ColorAttachment { view: sc.view(&fr), load: Load::Discard, clear: [0.0; 4], store: true, before: Layout::Undefined, after }],
        depth: None,
    });
    e.set_render_pipeline(&core.pipe);
    e.set_bind_group(0, &tex.group);
    e.push_constants(0, &[0i32; 4]);
    e.draw(3, 1, 0, 0);
    let scissor = |e: &mut gpu::CommandEncoder, r: &[i32; 4]| e.set_scissor(r[0] as u32, r[1] as u32, r[2] as u32, r[3] as u32);
    // the images and canvases the CPU left out
    if !w.sp.quads.is_empty() {
        for q in &w.sp.quads {
            match &q.src {
                QuadSrc::Image { tex, step, .. } => {
                    e.set_render_pipeline(&core.pipe_image);
                    e.set_bind_group(0, &tex.group);
                    e.push_constants(0, &[q.x, q.y, step[0] as i32, step[1] as i32]);
                    for r in &q.rects {
                        scissor(&mut e, r);
                        e.draw(3, 1, 0, 0);
                    }
                }
                QuadSrc::View { tex, .. } => {
                    // (its frame ended in Layout::Sampled; the content's pass made it visible to the shaders)
                    let Some(group) = &tex.group else { continue };
                    e.set_render_pipeline(&core.pipe);
                    e.set_bind_group(0, group);
                    e.push_constants(0, &[q.x, q.y, 0, 0]);
                    for r in &q.rects {
                        scissor(&mut e, r);
                        e.draw(3, 1, 0, 0);
                    }
                }
                QuadSrc::Canvas { tiles, copy, .. } => {
                    e.set_render_pipeline(if *copy { &core.pipe } else { &core.pipe_tile });
                    for t in tiles {
                        let mut bound = false;
                        for r in &q.rects {
                            let (x0, y0) = (t.b[0].max(r[0]), t.b[1].max(r[1]));
                            let (x1, y1) = (t.b[2].min(r[0] + r[2]), t.b[3].min(r[1] + r[3]));
                            if x0 < x1 && y0 < y1 {
                                if !bound {
                                    bound = true;
                                    e.set_bind_group(0, &t.tex.group);
                                    e.push_constants(0, &t.place);
                                }
                                scissor(&mut e, &[x0, y0, x1 - x0, y1 - y0]);
                                e.draw(3, 1, 0, 0);
                            }
                        }
                    }
                }
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
    if let Some(rb) = rb_now {
        let img = sc.texture(&fr);
        e.copy_texture_to_buffer(TextureRegion::mip(img, 0), rb, DataLayout { bytes_per_row: row_bytes, ..DataLayout::default() });
        e.texture_barrier(img, Layout::CopySrc, Layout::Present);
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    }
    let cmd = e.finish()?;
    let id = sc.present(fr, [cmd])?;
    w.pending = false;
    w.last_present = plat::time_us();
    w.out.frames += 1;
    #[cfg(target_arch = "wasm32")]
    {
        let _ = id;
        if let Some(rb) = rb_frame {
            let mut snap = Surface::new(fb.w, fb.h, FMT_XRGB);
            let n = snap.px.len();
            snap.px.copy_from_slice(&fb.px[..n]);
            let loose = w.sp.quads.iter().filter(|q| matches!(q.src, QuadSrc::Canvas { loose: true, .. })).flat_map(|q| q.rects.iter().copied()).collect();
            let late = w.sp.quads.iter().filter(|q| matches!(q.src, QuadSrc::View { .. })).flat_map(|q| q.rects.iter().copied()).collect();
            w.verify_wait.push(Verify { rb, fb: snap, loose, late });
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(rb) = &w.rb {
        core.dev.wait(id, 5_000_000_000)?;
        sc.flush()?;
        let n = fb.w as usize * fb.h as usize * 4;
        w.rb_px.resize(n, 0);
        rb.read(0, &mut w.rb_px[..n])?;
        let swap_rb = core.format == Format::Rgba8Unorm;
        if let Some(keep) = w.rb_keep.as_mut() {
            keep.clear();
            keep.extend_from_slice(&w.rb_px[..n]);
        }
        // what a gesture's tiles may show otherwise: counted, not compared
        for q in &w.sp.quads {
            if matches!(q.src, QuadSrc::Canvas { loose: true, .. }) {
                loose_compare(&mut w.rb_px, fb, &q.rects, swap_rb, &mut w.out);
            }
        }
        let (x, y, bad) = compare(&w.rb_px, fb, swap_rb);
        w.out.verified += 1;
        if bad != 0 {
            w.out.verify_bad += 1;
            plat::write_stderr(format!("gpu: the presented image differs from the framebuffer in {bad} pixels, first at {x}, {y}\n").as_bytes());
        }
    }
    Ok(Shown::Done)
}

/// (the browser) presented images that have come back by now: compared as show compares them elsewhere
#[cfg(target_arch = "wasm32")]
fn verify_poll(w: &mut Win, core: &Core) {
    let swap_rb = core.format == Format::Rgba8Unorm;
    while let Some(v) = w.verify_wait.first_mut() {
        let row = v.fb.w as usize * 4;
        let padded = (row + 255) & !255;
        let mut raw = vec![0u8; padded * v.fb.h as usize];
        match v.rb.read(0, &mut raw) {
            Err(GpuError::NotReady) => return,
            Err(_) => {
                w.verify_wait.clear();
                return;
            }
            Ok(()) => {}
        }
        let mut img: Vec<u8> = Vec::with_capacity(row * v.fb.h as usize);
        for r in raw.chunks_exact(padded) {
            img.extend_from_slice(&r[..row]);
        }
        loose_compare(&mut img, &v.fb, &v.loose, swap_rb, &mut w.out);
        // (a view's frame, which the framebuffer gets later: those pixels as the framebuffer has them)
        let mut ignore = GpuOutput::default();
        loose_compare(&mut img, &v.fb, &v.late, swap_rb, &mut ignore);
        let (x, y, bad) = compare(&img, &v.fb, swap_rb);
        w.out.verified += 1;
        if bad != 0 {
            w.out.verify_bad += 1;
            plat::write_stderr(format!("gpu: the presented image differs from the framebuffer in {bad} pixels, first at {x}, {y}\n").as_bytes());
        }
        w.verify_wait.remove(0);
    }
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

// rectangles of the presented image (rows of w * 4 bytes): how far they are from the framebuffer
// (GpuOutput::canvas_diff_*), then the framebuffer's pixels in them
fn loose_compare(img: &mut [u8], fb: &Surface, rects: &[[i32; 4]], swap_rb: bool, out: &mut GpuOutput) {
    let row = fb.w as usize * 4;
    let (mut n, mut sum, mut top) = (0u64, 0u64, 0u32);
    for r in rects {
        for y in r[1] as usize..(r[1] + r[3]) as usize {
            let a = &mut img[y * row + r[0] as usize * 4..y * row + (r[0] + r[2]) as usize * 4];
            let b = &fb.px[y * fb.stride as usize + r[0] as usize * 4..y * fb.stride as usize + (r[0] + r[2]) as usize * 4];
            for (p, q) in a.chunks_exact_mut(4).zip(b.chunks_exact(4)) {
                let c = if swap_rb { [q[2], q[1], q[0]] } else { [q[0], q[1], q[2]] };
                let d = (0..3).map(|i| p[i].abs_diff(c[i]) as u32).max().unwrap_or(0);
                if d != 0 {
                    n += 1;
                    sum += d as u64;
                    top = top.max(d);
                    p[..3].copy_from_slice(&c);
                }
            }
        }
    }
    out.canvas_diff_frames += (n != 0) as u64;
    out.canvas_diff_px += n;
    out.canvas_diff_sum += sum;
    out.canvas_diff_max = out.canvas_diff_max.max(top);
}

// floor and round without libm
fn flr(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

fn rnd(x: f64) -> f64 {
    flr(x + 0.5)
}

// a level for view v: at its scale whole pixels away, else (a zoom running) the one nearest in scale,
// else a new one: for a zoom at the scale of the view before (the content there is drawn already, a
// store may hold it) when the tiles can serve this one from there
fn level_for(c: &mut Canvas, v: CanvasView, zoom: bool) -> usize {
    let zoom = zoom && c.prev.scale != v.scale;
    let near = |l: &TileLevel, v: CanvasView| {
        let (dx, dy) = (v.ox - l.fx, v.oy - l.fy);
        (dx - rnd(dx)).abs() < PHASE_TOL && (dy - rnd(dy)).abs() < PHASE_TOL
    };
    let mut best: Option<(usize, f64)> = None;
    for (n, l) in c.levels.iter().enumerate() {
        let k = v.scale / l.scale;
        if k == 1.0 {
            if near(l, v) {
                return n;
            }
        } else if zoom && (K_MIN..=K_MAX).contains(&k) {
            let far = if k < 1.0 { 1.0 / k } else { k };
            if best.is_none_or(|b| far < b.1) {
                best = Some((n, far));
            }
        }
    }
    if let Some(b) = best {
        return b.0;
    }
    let p = c.prev;
    let from = if zoom && p.scale > 0.0 && (K_MIN..=K_MAX).contains(&(v.scale / p.scale)) { p } else { v };
    c.levels.push(TileLevel { scale: from.scale, fx: from.ox - flr(from.ox), fy: from.oy - flr(from.oy), tiles: BTreeMap::new() });
    c.levels.len() - 1
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

// an image's or a canvas's pixels in window box b of the framebuffer, as its command draws them there
fn quad_blit(fb: &mut Surface, q: &Quad, b: [i32; 4], text: &mut Text) {
    let mut dc = Dc::new(fb.w, fb.h);
    dc.clip_push(b[0], b[1], b[2] - b[0], b[3] - b[1]);
    match &q.src {
        QuadSrc::Image { src, w, h, .. } => dc.blit_scaled(fb, src, q.x, q.y, *w, *h),
        // (a view's Draw reads its frame back)
        QuadSrc::Canvas { f, w, h, vis, .. } | QuadSrc::View { f, w, h, vis, .. } => {
            dc.clip_push(vis[0], vis[1], vis[2] - vis[0], vis[3] - vis[1]);
            dc.clip_push(q.x, q.y, *w, *h);
            f.draw(&mut Gfx { dc: &mut dc, sf: fb, text }, [q.x, q.y, *w, *h]);
            dc.clip_pop();
            dc.clip_pop();
        }
    }
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

// ------------------------------------------------------------------ views (G5e1)

/// a view's content leaves its device: detach, the target and its read-back go (what was read back stays)
fn view_detach(e: &mut Entry) {
    if e.devgen == 0 {
        return;
    }
    if let Some(c) = e.content.upgrade() {
        if let Ok(mut c) = c.try_borrow_mut() {
            c.detach();
        }
    }
    e.info.detaches += 1;
    e.devgen = 0;
    e.output = false;
    e.refused = 0;
    e.tex = None;
    e.tried = None;
    e.apart = false;
    e.moved = false;
    e.rest_at = 0;
    e.held_at = 0;
    e.held_key = None;
    e.sh.c.borrow_mut().gpu = None;
}

/// one frame of view e: its target of this size (new: also the output's bind group for it and the read-back), the
/// content's pass into it, submitted.  The target rests in Layout::Sampled between frames: the pass goes from
/// there (Undefined the first time) to ColorTarget and back to Sampled at its end, which makes its writes
/// visible to the shaders that sample it (the output's pass, later on the same queue) and waits for their
/// reads before the next frame writes
fn view_frame(e: &mut Entry, content: &Rc<core::cell::RefCell<dyn GpuContent>>, dev: &Device, bgl: Option<&BindGroupLayout>, size: (u32, u32)) -> gpu::Result<()> {
    let (w, h) = size;
    if e.tex.as_ref().is_none_or(|t| (t.w, t.h) != size) {
        e.tex = None;
        e.sh.c.borrow_mut().gpu = None;
        let t = dev.create_texture(&TextureDesc::d2(w, h, GPU_VIEW_FORMAT, TextureUsage::COLOR_TARGET | TextureUsage::SAMPLED | TextureUsage::COPY_SRC))?;
        let group = match bgl {
            Some(l) => Some(dev.create_bind_group(&BindGroupDesc { layout: l, entries: &[BindEntry { binding: 0, resource: Resource::Texture(t.view()) }] })?),
            None => None,
        };
        let stride = (w * 4 + 255) & !255;
        e.sh.c.borrow_mut().gpu = Some(Target {
            dev: dev.clone(),
            t: t.clone(),
            // (the browser: a buffer per read-back, gpu_view.rs)
            #[cfg(not(target_arch = "wasm32"))]
            rb: dev.create_buffer(&BufferDesc { size: stride as u64 * h as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?,
            stride,
            #[cfg(target_arch = "wasm32")]
            wait: None,
        });
        e.tex = Some(Rc::new(ViewTex { t, group, w, h, fresh: core::cell::Cell::new(true) }));
    }
    let Some(tex) = e.tex.clone() else { return Err(GpuError::Invalid("no target")) };
    let fresh = tex.fresh.get();
    if let Err(err) = view_draw(&tex, content, dev, size, if fresh { Layout::Undefined } else { Layout::Sampled }) {
        // (nothing went to the queue: a new target holds no frame, so neither the output's pass nor a read-back may
        // take it; the next frame makes it again)
        if fresh {
            e.tex = None;
            e.sh.c.borrow_mut().gpu = None;
        }
        return Err(err);
    }
    tex.fresh.set(false);
    e.sh.c.borrow_mut().frame += 1;
    Ok(())
}

/// view_frame's pass: the content's frame into tex, submitted
fn view_draw(tex: &ViewTex, content: &Rc<core::cell::RefCell<dyn GpuContent>>, dev: &Device, size: (u32, u32), before: Layout) -> gpu::Result<()> {
    let mut c = content.try_borrow_mut().map_err(|_| GpuError::Invalid("the view's content is borrowed"))?;
    let mut enc = dev.encoder()?;
    let target = ColorAttachment { view: tex.t.view(), load: Load::Clear, clear: [0.0, 0.0, 0.0, 1.0], store: true, before, after: Layout::Sampled };
    c.draw(&mut enc, target, size)?;
    let id = dev.queue().submit([enc.finish()?])?;
    c.submitted(id);
    Ok(())
}

// ------------------------------------------------------------------ the runtime's calls

impl Ui {
    /// window_main: the output's decision for this run
    pub(crate) fn gp_window_mode(&mut self) {
        let want3d = self.desc.flags & AF_GPU_3D != 0;
        let s = &mut self.m.gout;
        s.want = wanted();
        s.want3d = want3d;
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
        // (the browser: the worker's turn may hold several presents, the browser shows only the last
        // image of a turn: gp_tick draws it, right after this, once)
        let soon = (t0 < w.last_present + MIN_PRESENT_US && !s.script) || cfg!(target_arch = "wasm32");
        // (what changes on the screen is known without layers, images and canvases)
        let apart = w.sp.drawn || w.sp.active || !w.sp.quads.is_empty();
        let before = core::mem::replace(&mut w.apart_before, apart);
        if apart || before {
            if let Some(sc) = &w.sc {
                sc.damage(None);
            }
        }
        // (a frame drawn layer by layer has uploaded its parts)
        let up = if core::mem::take(&mut w.sp.drawn) { Ok(()) } else { upload(w, core, fb, rects, &mut s.trim_at) };
        let r = up.and_then(|_| if soon || w.test_hold { Ok(Shown::Wait) } else { show(w, core, fb) });
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
            sc: None,
            surf: None,
            tex: None,
            rb: None,
            rb_px: Vec::new(),
            rb_keep: None,
            full: true,
            undef: true,
            pending: false,
            hidden: false,
            test_hold: false,
            apart_before: false,
            last_present: 0,
            job: 0,
            lost: false,
            size: (0, 0),
            stable_at: now,
            fail_at: 0,
            fail_ms: 0,
            out: GpuOutput::default(),
            sp: Split::default(),
            #[cfg(target_arch = "wasm32")]
            verify_wait: Vec::new(),
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
        // (the browser: its canvas goes with this frame, the next period on the GPU has a new one)
        #[cfg(target_arch = "wasm32")]
        {
            w.sc = None;
            w.surf = None;
        }
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
        // (views: their frames as read back last stay in the framebuffers; the contents leave the device)
        if matches!(self.m.gout.dev, Dev::Ready(_)) {
            let devgen = self.m.gout.dev_gen;
            self.gp_views_gone(Some(devgen));
        }
        let s = &mut self.m.gout;
        s.images_gone();
        s.canvases_gone();
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
        #[cfg(target_arch = "wasm32")]
        {
            let s = &mut self.m.gout;
            s.web_probe();
            if let Dev::Ready(core) = &s.dev {
                for w in s.wins.iter_mut().flatten() {
                    verify_poll(w, core);
                }
            }
        }
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
                            s.note(&format!("device ready: {} ({:?})", core.name, core.dev.level()));
                            s.dev = Dev::Ready(core);
                            s.dev_gen += 1; // (views: their contents come to it with their next frame)
                            s.ready_at = plat::time_us();
                            self.gp_built(win, built);
                            if !self.m.gview.views.is_empty() {
                                self.invalidate(); // (a view at rest gets its frame on the new device)
                            }
                        }
                        Err(e) => {
                            let s = &mut self.m.gout;
                            s.note(&format!("no device: {e}"));
                            match e {
                                GpuError::DeviceLost | GpuError::OutOfMemory | GpuError::Timeout | GpuError::Failed(_) | GpuError::SurfaceLost
                                | GpuError::NotReady => {
                                    s.dev = Dev::Retry(plat::time_us() + s.retry_ms * 1000);
                                    s.retry_ms = (s.retry_ms * 2).min(RETRY_MAX_MS);
                                    plat::gpu_out_state(plat::GPU_STATE_UNKNOWN);
                                }
                                _ => {
                                    s.dev = Dev::Never(format!("{e}")); // no Vulkan, no usable adapter: the CPU path for good
                                    plat::gpu_out_state(plat::GPU_STATE_NEVER);
                                }
                            }
                            if let Some(w) = s.win(win) {
                                w.stage = Stage::Cpu;
                                w.lost = false;
                                w.out.error = Some(format!("{e}"));
                                // (the browser: the page's canvas, which never showed anything, goes)
                                #[cfg(target_arch = "wasm32")]
                                plat::win_gpu_child_show(w.pw, false);
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
                                w.stable_at = now;
                                if !core::mem::take(&mut w.lost) {
                                    w.out.error = Some(format!("{e}"));
                                    w.fail_ms = (w.fail_ms * 2).clamp(RETRY_FIRST_MS, RETRY_MAX_MS);
                                    w.fail_at = now + w.fail_ms * 1000;
                                }
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
        if core::mem::take(&mut w.lost) {
            w.stage = Stage::Cpu; // (Android: the surface it was built on went meanwhile)
            return;
        }
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
            // (the browser: only a sign, show makes a buffer per frame)
            let size = if cfg!(target_arch = "wasm32") { 4 } else { tw as u64 * th as u64 * 4 };
            w.rb = dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Readback }).ok();
        }
        w.stage = Stage::Switch;
        if !unseen(w) {
            self.gp_switch(i);
        }
    }

    /// the whole framebuffer on the new swapchain, then the child shown: from now on the GPU
    fn gp_switch(&mut self, i: usize) {
        let t0 = plat::time_us();
        let mut s = core::mem::take(&mut self.m.gout);
        let mut failed = None;
        let mut again = false;
        let fb = self.uiw_parts(i as u32).map(|p| &p.rt.fb);
        if let (Dev::Ready(core), Some(Some(w)), Some(fb)) = (&s.dev, s.wins.get_mut(i), fb) {
            if w.tex.as_ref().is_none_or(|t| (t.w, t.h) != (fb.w, fb.h)) || !plat::win_gpu_child_alone(w.pw) {
                w.stage = Stage::Cpu; // the size changed meanwhile (Android: something else shows now)
                w.stable_at = t0;
                again = true;
            } else {
                let ready = w.sc.as_mut().map_or(Ok(true), |sc| sc.prepare());
                let shown = match ready {
                    Ok(true) => upload(w, core, fb, None, &mut s.trim_at).and_then(|_| show(w, core, fb)),
                    Ok(false) => Ok(Shown::Wait), // (Linux: the display has not answered for the images yet)
                    Err(e) => Err(e),
                };
                match shown {
                    Ok(Shown::Done) => {
                        plat::win_gpu_child_show(w.pw, true);
                        plat::win_release_kept(w.pw);
                        w.stage = Stage::Gpu;
                        w.out.switches += 1;
                        w.out.error = None;
                        if !core::mem::replace(&mut s.said_ok, true) {
                            plat::gpu_out_state(plat::GPU_STATE_OK);
                        }
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
        // (the browser tells a lost device, or an error of an earlier call, between frames too)
        #[cfg(target_arch = "wasm32")]
        {
            let e = if let Dev::Ready(core) = &self.m.gout.dev { core.dev.poll().err() } else { None };
            // (and the page's watcher: the canvas shows nothing, a new one would not either)
            let e = e.or(plat::win_gpu_child_blank().then_some(GpuError::Unsupported("the canvas shows nothing")));
            if let Some(e) = e {
                self.gp_failed(0, e);
                return;
            }
        }
        let n = self.m.gout.wins.len();
        let now = plat::time_us();
        self.gp_canvas_tick(now);
        self.gp_views_tick(now);
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
            // (Linux: a frame the GPU has finished goes to the window)
            if matches!(w.stage, Stage::Gpu | Stage::Switch) && w.sc.as_ref().is_some_and(|sc| sc.busy()) {
                if let Some(Err(e)) = w.sc.as_ref().map(|sc| sc.flush()) {
                    self.gp_failed(i, e);
                    continue;
                }
            }
            let Some(w) = self.m.gout.wins[i].as_ref() else { continue };
            match w.stage {
                Stage::Switch if !unseen(w) => self.gp_switch(i),
                Stage::Gpu if unseen(w) => self.gp_leave(i, "hidden"),
                // (Android: a tool window or popup is composed on the canvas, below the surface)
                Stage::Gpu if !plat::win_gpu_child_alone(w.pw) => self.gp_leave(i, "something else shows"),
                Stage::Gpu if w.pending && !w.test_hold && plat::time_us() >= w.last_present + MIN_PRESENT_US => self.gp_retry(i),
                _ => {}
            }
        }
        self.gp_views_wake(plat::time_us());
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
        #[cfg(target_arch = "wasm32")]
        if s.web_start() {
            return;
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
                if !plat::win_gpu_child_later(pw) {
                    s.dev = Dev::Never(String::from("no child window"));
                }
                return;
            };
            let display = plat::win_display_adapter(pw);
            let desc = s.desc();
            let job = s.next_job;
            s.next_job += 1;
            let (trace, want3d) = (s.trace, s.want3d);
            s.note("start: instance, device, swapchain on a worker");
            plat::gpu_out_state(plat::GPU_STATE_TRYING);
            let ok = s.spawn(move || Done::Device { job, win: i, r: build_device(None, nw, pw, display, desc, trace, want3d) });
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
            if w.stage != Stage::Cpu || unseen(w) || now < w.fail_at || now < w.stable_at + STABLE_US {
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
                Some(x) => {
                    x.track_size(); // (Linux: a worker may not ask the window system)
                    x
                }
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
            #[allow(clippy::redundant_locals)] // each window's job takes a copy
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
            Dev::None if s.wins.iter().flatten().next().is_some_and(|w| !plat::win_gpu_child_later(w.pw)) => d = now,
            Dev::Retry(at) => d = d.min(at),
            _ => {}
        }
        if s.trim_at != 0 {
            d = d.min(s.trim_at);
        }
        if !s.script {
            for c in s.canvases.iter().filter(|c| c.apart) {
                d = d.min(c.until);
            }
        }
        if s.canvas_free_at != 0 {
            d = d.min(s.canvas_free_at);
        }
        // (views: the look at rest, once after the last frame; G5e3: a frame that waits for its window, at the latest
        // when it has waited long enough, at once when the window's frame has been shown)
        for e in self.m.gview.views.iter() {
            if e.rest_at != 0 {
                d = d.min(e.rest_at);
            }
            if e.held_at != 0 && !e.woke {
                d = d.min(if s.unshown(e.win) { e.held_at + VIEW_HOLD_US } else { now });
            }
        }
        let ready = matches!(s.dev, Dev::Ready(_));
        let moving = plat::win_in_sizemove();
        for w in s.wins.iter().flatten() {
            if w.sp.linger_at != 0 {
                d = d.min(w.sp.linger_at);
            }
            let minimised = plat::win_state(w.pw) & plat::WST_MINIMIZED != 0;
            // (Linux: a frame on its way, a display that has not answered yet)
            let on_way = w.sc.as_ref().is_some_and(|sc| sc.busy());
            match w.stage {
                Stage::Switch if !minimised && !unseen(w) => d = d.min(now + RETRY_US),
                Stage::Gpu if on_way => d = d.min(now + RETRY_US.min(1000)),
                Stage::Gpu if w.pending && !minimised && !w.test_hold => {
                    let retry = (w.last_present + MIN_PRESENT_US).max(now + RETRY_US.min(1000));
                    d = d.min(w.sc.as_ref().map_or(retry, |sc| sc.next_look(retry)));
                }
                Stage::Cpu if ready && !minimised && !moving && !unseen(w) && w.fail_at != u64::MAX && !plat::win_gpu_child_later(w.pw) => {
                    // (a size the frame has not reached yet: its event is on the way)
                    let settle = if plat::win_size(w.pw) == w.size { 1000 } else { 16_000 };
                    d = d.min((w.stable_at + STABLE_US).max(w.fail_at).max(now + settle));
                }
                _ => {}
            }
        }
        if d == u64::MAX { None } else { Some(d) }
    }

    /// (Android) the system took the surface (the activity stopped; or the output's own hide, then
    /// nothing is on it any more): every window back to the CPU path, its swapchain gone before the
    /// host's callback returns -> a frame waits to reach the host's Bitmap (the View may be seen
    /// again before the next frame)
    #[cfg(target_os = "android")]
    pub(crate) fn gp_surface_gone(&mut self) -> bool {
        if !self.m.gout.want {
            return false;
        }
        let mut waits = false;
        for i in 0..self.m.gout.wins.len() {
            let Some(stage) = self.m.gout.wins[i].as_ref().map(|w| w.stage) else { continue };
            if stage == Stage::Gpu {
                self.gp_leave(i, "the surface went");
                waits = true;
            }
            let Some(w) = self.m.gout.win(i) else { continue };
            if stage == Stage::Job {
                w.lost = true; // (what it built goes when it ends)
            } else {
                w.stage = Stage::Cpu;
            }
            w.sc = None;
            w.surf = None;
            w.tex = None;
            w.rb = None;
            w.pending = false;
            w.sp.reset();
        }
        waits
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
        // (its views: their contents leave the device, the records go)
        let views = &mut self.m.gview.views;
        for e in views.iter_mut().filter(|e| e.win == i) {
            view_detach(e);
        }
        views.retain(|e| e.win != i);
        if self.m.gout.wins.get(i).and_then(|w| w.as_ref()).is_none() {
            return;
        }
        if self.m.gout.jobs.running.load(Ordering::Acquire) != 0 {
            self.gp_wait_jobs(3000);
        }
        if let Some(w) = self.m.gout.wins.get_mut(i) {
            *w = None;
        }
        let s = &mut self.m.gout;
        for k in (0..s.canvases.len()).rev() {
            if s.canvases[k].win == i {
                let c = s.canvases.remove(k);
                let n: usize = c.levels.iter().map(|l| l.tiles.len()).sum();
                s.tile_bytes -= n as u64 * TILE_BYTES;
            }
        }
    }

    /// window i is about to be hidden or shown again: hidden, it goes back to the CPU path (the top of
    /// this file), and its swapchain follows when it is back and has settled
    pub(crate) fn gp_window_show(&mut self, i: u32, show: bool) {
        let i = i as usize;
        let Some(stage) = self.m.gout.wins.get(i).and_then(|w| w.as_ref()).map(|w| w.stage) else { return };
        if !show && stage == Stage::Gpu {
            self.gp_leave(i, "hidden");
        }
        let now = plat::time_us();
        if let Some(w) = self.m.gout.win(i) {
            w.hidden = !show;
            w.stable_at = now;
        }
    }

    /// the window loop ended: everything of the GPU goes (a job still running after 2 s is left
    /// to the process's end)
    pub(crate) fn gp_shutdown(&mut self) {
        // (views: their contents drop what they built before a device goes, also the views' own)
        self.gp_views_gone(None);
        self.m.gview.own = Own::None;
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
            s.canvases.clear();
            s.tile_free.clear();
            s.dev = Dev::None;
        } else {
            core::mem::forget(core::mem::take(&mut s.wins));
            core::mem::forget(core::mem::take(&mut s.images));
            core::mem::forget(core::mem::take(&mut s.canvases));
            core::mem::forget(core::mem::take(&mut s.tile_free));
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
            o.image_quads = w.sp.quads.iter().filter(|q| matches!(q.src, QuadSrc::Image { .. })).count() as u32;
            o.view_quads = w.sp.quads.iter().filter(|q| matches!(q.src, QuadSrc::View { .. })).count() as u32;
            o.canvas_quads = w.sp.quads.len() as u32 - o.image_quads - o.view_quads;
            o
        });
        o.wanted = s.want;
        o.image_textures = s.images.iter().filter(|e| e.tex.is_some()).count() as u32;
        o.image_bytes = s.image_bytes;
        o.image_uploads = s.image_uploads;
        o.image_hits = s.image_hits;
        o.image_evicted = s.image_evicted;
        o.canvas_apart = s.canvases.iter().filter(|c| c.apart && c.win == win as usize).count() as u32;
        o.canvas_tiles = s.canvases.iter().flat_map(|c| c.levels.iter()).map(|l| l.tiles.len() as u32).sum();
        o.canvas_bytes = s.tile_bytes;
        o.canvas_fills = s.tile_fills;
        o.canvas_fill_us = s.tile_fill_us;
        o.canvas_evicted = s.tile_evicted;
        o.canvas_settles = s.canvas_settles;
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

    /// (test aid, G5e3) window `win`'s frames are not shown while on (as if the display did not answer): they stay in
    /// its texture, its views' new frames wait for them; off: the next look shows the newest
    pub fn gpu_test_hold(&mut self, win: u32, on: bool) {
        if let Some(w) = self.m.gout.win(win as usize) {
            w.test_hold = on;
        }
    }

    /// (test aid) the bytes image textures may take
    pub fn gpu_test_image_budget(&mut self, bytes: u64) {
        self.m.gout.image_budget = bytes;
    }

    /// (test aid) the bytes canvas tiles may take
    pub fn gpu_test_canvas_budget(&mut self, bytes: u64) {
        self.m.gout.canvas_budget = bytes;
    }

    /// (test aid) the tiles go this long (ms) after the last move
    pub fn gpu_test_canvas_unused(&mut self, ms: u64) {
        self.m.gout.canvas_unused_us = ms * 1000;
    }

    /// every canvas whose view comes from tiles goes back to the CPU's picture with the next frame
    pub fn gpu_canvas_settle(&mut self) {
        let mut any = false;
        let s = &mut self.m.gout;
        for c in s.canvases.iter_mut().filter(|c| c.apart) {
            c.apart = false;
            any = true;
            s.canvas_settles += 1;
        }
        if any {
            self.invalidate();
        }
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
            sp.part = vec![false; total];
            sp.redraw = vec![false; total];
            sp.any_owed = false;
            sp.any_stale = false;
        }
        sp.hole.fill(0);
        sp.part.fill(false);
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
                quads.push(Quad { x: *x, y: *y, src: QuadSrc::Image { tex, src: sf.clone(), w: *iw, h: *ih, step }, rects });
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

    /// widgets build a CanvasDesc when this says so (canvas.rs)
    pub(crate) fn gp_canvas_wanted(&self) -> bool {
        let s = &self.m.gout;
        s.want && s.canvases_on
    }

    /// a widget about to record canvas id with this content version: would a move of its view come
    /// from tiles in the next frame?
    pub(crate) fn gp_canvas_steady(&self, id: u32, ver: u32) -> bool {
        let i = self.uiw_cur as usize;
        let s = &self.m.gout;
        s.want && s.canvases_on && !self.rtl && self.gp_split_wanted() && s.canvases.iter().find(|c| c.win == i && c.id == id).is_some_and(|c| c.ver == ver && !c.damaged)
    }

    /// dirty_compute, before hashing: what the view of each canvas of the frame did since the last
    /// frame, and so which of them come from tiles now (W::canv)
    pub(crate) fn gp_canvas_begin(&mut self) {
        let i = self.uiw_cur as usize;
        let can = self.wp.dirty.split && !self.rtl && self.dc.opac == 255 && self.gp_can_split(i);
        let now = plat::time_us();
        let Ui { m, wp, .. } = self;
        let s = &mut m.gout;
        let (settle, unused) = (s.canvas_settle_us, s.canvas_unused_us);
        wp.dirty.canv.clear();
        for rec in wp.canv.recs.iter() {
            let d = &rec.d;
            let mode = s.canvas_mode(d.name, d.gpu);
            if mode == CanvasGpu::Cpu {
                continue;
            }
            let ci = s.canvases.iter().position(|c| c.win == i && c.id == d.id).unwrap_or_else(|| {
                s.canvases.push(Canvas {
                    win: i,
                    id: d.id,
                    ver: d.ver,
                    view: d.view,
                    prev: d.view,
                    known: false,
                    damaged: false,
                    apart: false,
                    until: 0,
                    moved: false,
                    levels: Vec::new(),
                });
                s.canvases.len() - 1
            });
            let new_ver = s.canvases[ci].ver != d.ver;
            if new_ver {
                let levels = core::mem::take(&mut s.canvases[ci].levels);
                for l in levels {
                    s.tiles_free(l.tiles);
                }
            }
            let c = &mut s.canvases[ci];
            let changed = new_ver || c.damaged;
            let moved = c.known && c.view != d.view && d.view.scale > 0.0 && d.view.scale < 1.0e6;
            let (dx, dy) = (d.view.ox - c.view.ox, d.view.oy - c.view.oy);
            let whole = d.view.scale == c.view.scale && (dx - rnd(dx)).abs() < PHASE_TOL && (dy - rnd(dy)).abs() < PHASE_TOL;
            c.moved = false;
            if changed || !can || rec.layer != 0 {
                c.apart = false; // the CPU draws this frame, as without tiles
            } else if moved {
                c.apart = mode == CanvasGpu::Zoom || whole;
                if c.apart {
                    c.moved = true;
                    c.until = now + settle;
                    s.canvas_free_at = c.until + unused;
                }
            }
            let c = &mut s.canvases[ci];
            if moved {
                c.prev = c.view;
            }
            c.ver = d.ver;
            c.view = d.view;
            c.damaged = false;
            c.known = true;
            if c.apart {
                wp.dirty.canv.push((rec.index, d.rect));
            }
        }
    }

    /// dirty_compute, after hashing: the tiles each canvas apart needs now (the ones missing are
    /// drawn for up to FILL_US), the cells the GPU draws it in -> any canvas apart
    pub(crate) fn gp_plan_canvases(&mut self) -> bool {
        let i = self.uiw_cur as usize;
        if self.wp.dirty.canv.is_empty() {
            return false;
        }
        let (fw, fh) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        let Ui { m, wp, text, .. } = self;
        let font = text.get_font();
        let s = &mut m.gout;
        s.tick += 1;
        let (tick, t0) = (s.tick, plat::time_us());
        let fill_end = if s.script { u64::MAX } else { t0 + FILL_US };
        let d = &wp.dirty;
        let (cols, rows) = (d.cols as usize, d.rows as usize);
        let Some(Some(w)) = s.wins.get_mut(i) else { return false };
        let mut quads = core::mem::take(&mut w.sp.quads);
        let mut sf = core::mem::take(&mut s.tile_sf);
        if sf.px.is_empty() {
            sf = Surface::new(TILE_TEX, TILE_TEX, FMT_XRGB);
        }
        for (tag0, seen) in d.imgs.iter().enumerate() {
            let Some(Cmd::Custom { x, y, w: cw, h: ch, f: Some(f), .. }) = wp.cmd.layers[0].get(seen.index as usize) else { continue };
            let Some(rec) = wp.canv.recs.iter().find(|r| r.layer == 0 && r.index == seen.index) else { continue };
            let Some(ci) = s.canvases.iter().position(|c| c.win == i && c.id == rec.d.id) else { continue };
            if quads.len() >= TOP_BLOCKED as usize - 1 {
                continue;
            }
            let (tag, vis, v) = (tag0 as u8 + 1, seen.vis, rec.d.view);
            let zoom = s.canvas_mode(rec.d.name, rec.d.gpu) == CanvasGpu::Zoom;
            let (moved, li) = {
                let c = &mut s.canvases[ci];
                (c.moved, level_for(c, v, zoom))
            };
            let mut li = li;
            if s.canvases[ci].levels.len() > LEVELS_MAX {
                let old = s.canvases[ci].levels.remove(0);
                s.tiles_free(old.tiles);
                li -= 1;
            }
            // ---- the transform: window = o + (grid - f) * k
            let (lscale, fx, fy) = {
                let l = &s.canvases[ci].levels[li];
                (l.scale, l.fx, l.fy)
            };
            let k = v.scale / lscale;
            let (dx, dy) = (v.ox - fx, v.oy - fy);
            let (rx, ry) = (rnd(dx), rnd(dy));
            let copy = k == 1.0 && (dx - rx).abs() < PHASE_TOL && (dy - ry).abs() < PHASE_TOL;
            let t = TILE as f64;
            let range = |w0: i32, w1: i32, o: f64, f: f64| -> (i32, i32) {
                let g = |w: f64| (w - o) / k + f;
                (flr(g(w0 as f64 - 1.0) / t) as i32, flr(g(w1 as f64 + 1.0) / t) as i32)
            };
            let (ix0, ix1) = range(vis[0], vis[2], v.ox, fx);
            let (iy0, iy1) = range(vis[1], vis[3], v.oy, fy);
            // the pixel a position at the left or top of belongs to: the first whose centre lies there
            let edge = |g: f64, o: f64, f: f64| -> i32 { (flr(o + (g - f) * k - 0.5) + 1.0) as i32 };
            let mut tiles = Vec::new();
            let mut missing: Vec<[i32; 4]> = Vec::new();
            for iy in iy0..=iy1 {
                for ix in ix0..=ix1 {
                    let (gx, gy) = (ix as f64 * t, iy as f64 * t);
                    let (b, place) = if copy {
                        let (x0, y0) = (rx as i32 + ix * TILE, ry as i32 + iy * TILE);
                        ([x0, y0, x0 + TILE, y0 + TILE], [x0 - TILE_BORDER, y0 - TILE_BORDER, 0, 0])
                    } else {
                        let b = [edge(gx, v.ox, fx), edge(gy, v.oy, fy), edge(gx + t, v.ox, fx), edge(gy + t, v.oy, fy)];
                        let at = |g: f64, o: f64, f: f64| rnd((o + (g - TILE_BORDER as f64 - f) * k) * 256.0) as i32;
                        let step = rnd(65536.0 / k) as i32;
                        (b, [at(gx, v.ox, fx), at(gy, v.oy, fy), step, step])
                    };
                    if b[0] >= vis[2] || b[2] <= vis[0] || b[1] >= vis[3] || b[3] <= vis[1] || b[0] >= b[2] || b[1] >= b[3] {
                        continue;
                    }
                    if let Some(tile) = s.canvases[ci].levels[li].tiles.get_mut(&(ix, iy)) {
                        tile.tick = tick;
                        tiles.push(TileDraw { tex: tile.tex.clone(), b, place });
                        continue;
                    }
                    let t1 = plat::time_us();
                    let tex = if t1 < fill_end { s.tile_texture(tick) } else { None };
                    let Some(tex) = tex else {
                        missing.push(b);
                        continue;
                    };
                    let tv = CanvasView { scale: lscale, ox: fx - gx + TILE_BORDER as f64, oy: fy - gy + TILE_BORDER as f64 };
                    let mut dc = Dc::new(TILE_TEX, TILE_TEX);
                    dc.begin(&sf);
                    rec.d.content.draw_view(&mut Gfx { dc: &mut dc, sf: &mut sf, text: &mut *text }, tv, [0, 0, TILE_TEX, TILE_TEX]);
                    dc.end();
                    let Dev::Ready(core) = &s.dev else {
                        missing.push(b);
                        continue;
                    };
                    let before = if tex.fresh.replace(false) { Layout::Undefined } else { Layout::Sampled };
                    if core.dev.queue().write_texture_rects(&tex.t, &[[0, 0, TILE_TEX, TILE_TEX]], &sf.px, sf.stride as u32, before, Layout::Sampled).is_err() {
                        missing.push(b); // (a failure of the device shows with the next present)
                        continue;
                    }
                    s.tile_fills += 1;
                    s.tile_fill_us += plat::time_us().saturating_sub(t1);
                    s.trim_at = plat::time_us() + TRIM_US;
                    if let Some(Some(w)) = s.wins.get_mut(i) {
                        w.out.upload_bytes += TILE_BYTES;
                    }
                    tiles.push(TileDraw { tex: tex.clone(), b, place });
                    s.canvases[ci].levels[li].tiles.insert((ix, iy), Tile { tex, tick });
                }
            }
            // ---- the cells: on top of them and with its tiles the GPU draws it, else the CPU
            let Some(Some(w)) = s.wins.get_mut(i) else { continue };
            let sp = &mut w.sp;
            if sp.hole.len() != cols * rows {
                continue;
            }
            let q = quads.len() as u8 + 1;
            let span = |a: i32, b: i32, n: usize| ((a.max(0) >> 5) as usize, (((b + CELL - 1).max(0) >> 5) as usize).min(n));
            let (c0, c1) = span(vis[0], vis[2], cols);
            let (r0, r1) = span(vis[1], vis[3], rows);
            for r in r0..r1 {
                for c in c0..c1 {
                    let k = r * cols + c;
                    if d.top.get(k) == Some(&tag) {
                        let b = cell_box(k, cols, fw, fh);
                        sp.hole[k] = q;
                        sp.part[k] = b[0] < vis[0] || b[1] < vis[1] || b[2] > vis[2] || b[3] > vis[3];
                        if moved {
                            sp.owed[k] = true;
                            sp.stale[k] = true;
                        }
                    }
                    if moved {
                        sp.redraw[k] = true;
                    }
                }
            }
            for b in &missing {
                let (c0, c1) = span(b[0].max(vis[0]), b[2].min(vis[2]), cols);
                let (r0, r1) = span(b[1].max(vis[1]), b[3].min(vis[3]), rows);
                for r in r0..r1 {
                    for c in c0..c1 {
                        let k = r * cols + c;
                        if sp.hole[k] == q {
                            sp.hole[k] = 0;
                            sp.part[k] = false;
                        }
                    }
                }
            }
            sp.moved |= moved;
            w.out.canvas_frames += moved as u64;
            let mut rects = Vec::new();
            let hole = &sp.hole;
            cell_rects(&mut rects, cols, rows, fw, fh, |k| hole[k] == q);
            for r in rects.iter_mut() {
                let (x0, y0) = (r[0].max(vis[0]), r[1].max(vis[1]));
                let (x1, y1) = ((r[0] + r[2]).min(vis[2]), (r[1] + r[3]).min(vis[3]));
                *r = [x0, y0, x1 - x0, y1 - y0];
            }
            rects.retain(|r| r[2] > 0 && r[3] > 0);
            if !rects.is_empty() {
                let loose = !(rec.d.exact_pan && copy);
                quads.push(Quad { x: *x, y: *y, src: QuadSrc::Canvas { f: f.clone(), w: *cw, h: *ch, vis, tiles, copy, loose }, rects });
            }
        }
        s.tile_sf = sf;
        text.set_font(font.0, font.1);
        if let Some(Some(w)) = s.wins.get_mut(i) {
            w.sp.quads = quads;
        }
        true
    }

    /// the window loop: gestures over (a frame draws their picture), tiles nobody moved over for a
    /// long time
    fn gp_canvas_tick(&mut self, now: u64) {
        let s = &mut self.m.gout;
        if !s.script && s.canvases.iter().any(|c| c.apart && now >= c.until) {
            for c in s.canvases.iter_mut().filter(|c| c.apart && now >= c.until) {
                c.apart = false;
                s.canvas_settles += 1;
            }
            self.invalidate();
        }
        let s = &mut self.m.gout;
        if s.canvas_free_at != 0 && now >= s.canvas_free_at {
            s.canvases_gone();
            s.trim_at = now + TRIM_US;
        }
    }

    /// ui.canvas_gpu_damage: the tiles over content rectangle r go, the next frame is the CPU's
    pub(crate) fn gp_canvas_damage(&mut self, id: u32, r: [f64; 4]) {
        let i = self.uiw_cur as usize;
        let S { canvases, tile_free, .. } = &mut self.m.gout;
        let Some(c) = canvases.iter_mut().find(|c| c.win == i && c.id == id) else { return };
        c.damaged = true;
        for l in c.levels.iter_mut() {
            let t = TILE as f64;
            let cell = |u: f64, f: f64, m: f64| flr((u * l.scale + f + m) / t) as i32;
            let (ix0, ix1) = (cell(r[0], l.fx, -2.0), cell(r[2], l.fx, 2.0));
            let (iy0, iy1) = (cell(r[1], l.fy, -2.0), cell(r[3], l.fy, 2.0));
            l.tiles.retain(|&(ix, iy), tile| {
                let hit = ix >= ix0 && ix <= ix1 && iy >= iy0 && iy <= iy1;
                if hit {
                    tile_free.push(tile.tex.clone());
                }
                !hit
            });
        }
    }

    // ------------------------------------------------------------------ views (G5e1, gpu_view.rs)

    /// the device views draw on now: the output's (device, generation, true); else, when this run has no output
    /// (not wanted, never possible), their own, made on first use; None while the output's device is on its way
    /// or a new one is awaited (a view shows what it showed last)
    fn gp_view_device(&mut self, now: u64) -> Option<(Device, u32, bool)> {
        let Mods { gout: s, gview, .. } = &mut self.m;
        if let (true, Dev::Ready(core)) = (s.want, &s.dev) {
            return Some((core.dev.clone(), s.dev_gen, true));
        }
        if let Own::Ready { dev, devgen, .. } = &gview.own {
            return Some((dev.clone(), *devgen, false));
        }
        if s.want && !matches!(s.dev, Dev::Never(_)) {
            return None;
        }
        match gview.own {
            Own::Never(_) => return None,
            Own::Retry(at) if now < at => return None,
            _ => {}
        }
        // (the browser: adapter and device answer later, and the views draw on the output's device anyway)
        if cfg!(target_arch = "wasm32") {
            gview.own = Own::Never(String::from("the browser: views draw on the output's device"));
            return None;
        }
        let t0 = plat::time_us();
        let r = Instance::new().and_then(|inst| {
            let a = inst.adapter()?;
            let dev = a.create_device(&DeviceDesc::DEFAULT)?;
            Ok((inst, a.info().name.clone(), dev))
        });
        match r {
            Ok((inst, name, dev)) => {
                s.dev_gen += 1;
                s.note(&format!("views: a device of their own: {name} ({:?}), {:.2} ms", dev.level(), ms(t0, plat::time_us())));
                gview.own = Own::Ready { _inst: inst, dev: dev.clone(), devgen: s.dev_gen };
                Some((dev, s.dev_gen, false))
            }
            Err(e) => {
                s.note(&format!("views: no device of their own: {e}"));
                gview.own = match e {
                    GpuError::DeviceLost | GpuError::OutOfMemory | GpuError::Timeout | GpuError::Failed(_) | GpuError::NotReady => Own::Retry(now + RETRY_FIRST_MS * 1000),
                    _ => Own::Never(format!("{e}")), // no Vulkan, jfgui_GPU=0, no usable adapter: they paint
                };
                None
            }
        }
    }

    /// dirty_compute, before hashing: the live window's views of this frame.  The device (a new one takes the
    /// content along: detach, attach); a frame where the ver, size or device changed, drawn and submitted now,
    /// before anything reads it; whether the output's render pass draws it (dirty.canv: its hash stands for its
    /// place then, as a canvas apart's), else its command hashes the frame's number: a new frame, a new picture
    pub(crate) fn gp_views_begin(&mut self) {
        if self.wp.canv.views.is_empty() {
            return;
        }
        // (contents the app dropped: their records go, and with them what holds a device; also without an output,
        // where gp_tick does not look)
        self.m.gview.views.retain(|e| e.content.strong_count() > 0);
        let i = self.uiw_cur as usize;
        let now = plat::time_us();
        let dev = self.gp_view_device(now);
        let pw = self.uiw_pw(i as u32);
        // (nobody sees a minimised or hidden window: no frame for it, the next one when it is back)
        let unseen = pw >= 0 && plat::win_state(pw) & (plat::WST_MINIMIZED | plat::WST_HIDDEN) != 0;
        let can = self.wp.dirty.split && !self.rtl && self.dc.opac == 255 && self.m.gout.views_on && self.gp_can_split(i);
        // (G5e3) the window's last frame is not on the screen yet: a new frame of its views waits for it
        let unshown = self.m.gout.unshown(i);
        let mut failed: Option<(GpuError, bool, u32)> = None;
        let Ui { m, wp, .. } = self;
        let Mods { gout: s, gview, .. } = m;
        let bgl = match &s.dev {
            Dev::Ready(core) if s.want => Some(core.bgl.clone()),
            _ => None,
        };
        let lim = dev.as_ref().map_or(0, |d| d.0.limits().max_texture_2d);
        for rec in wp.canv.views.iter_mut() {
            rec.apart = false;
            let Some(Cmd::Custom { x, y, w, h, ver, .. }) = wp.cmd.layers[rec.layer as usize].get_mut(rec.index as usize) else { continue };
            let Some(e) = gview.views.iter_mut().find(|e| e.win == i && e.id == rec.id) else { continue };
            let Some(content) = e.content.upgrade() else { continue };
            // ---- the device
            if let Some((d, devgen, out)) = &dev {
                if e.devgen != *devgen && e.refused != *devgen {
                    if e.devgen != 0 {
                        view_detach(e);
                    }
                    let r = match content.try_borrow_mut() {
                        Ok(mut c) => c.attach(d),
                        Err(_) => Err(GpuError::Invalid("the view's content is borrowed")),
                    };
                    match r {
                        Ok(()) => {
                            e.devgen = *devgen;
                            e.output = *out;
                            e.level = Some(d.level());
                            e.info.attaches += 1;
                            e.info.error = None;
                            s.note(&format!("view {}: attached to device {devgen}", e.id));
                        }
                        Err(err) => {
                            e.refused = *devgen;
                            e.info.error = Some(format!("{err}"));
                        }
                    }
                }
            }
            let attached = e.devgen != 0 && dev.as_ref().is_some_and(|d| d.1 == e.devgen);
            let size = ((*w).max(0) as u32, (*h).max(0) as u32);
            e.info.size = size;
            // (the browser: a read-back that came after its Draw goes to the framebuffer as a new frame would)
            e.moved = core::mem::take(&mut e.arrived);
            if e.moved {
                e.rest_at = now + VIEW_REST_US;
            }
            // ---- a frame
            let key = (rec.ver, size, e.devgen);
            let due = attached && !unseen && size.0 > 0 && size.1 > 0 && size.0 <= lim && size.1 <= lim && e.tried != Some(key);
            // (G5e3) bound to the display: while the window's last frame is not shown, the content is not asked (this
            // render takes the view's last frame); gp_views_wake asks for a render once it is shown, or after VIEW_HOLD_US
            e.woke = false;
            let hold = due && unshown && !(e.held_at != 0 && now >= e.held_at + VIEW_HOLD_US);
            if hold {
                if e.held_key != Some(key) {
                    e.held_key = Some(key);
                    e.info.held += 1;
                }
                if e.held_at == 0 {
                    e.held_at = now;
                }
            } else {
                e.held_at = 0;
                e.held_key = None;
            }
            if due && !hold {
                e.tried = Some(key);
                let (d, _, out) = dev.as_ref().unwrap_or_else(|| unreachable!());
                let t0 = plat::time_us();
                match view_frame(e, &content, d, if *out { bgl.as_ref() } else { None }, size) {
                    Ok(()) => {
                        e.moved = true;
                        e.rest_at = now + VIEW_REST_US;
                        e.info.error = None;
                    }
                    Err(err) => {
                        e.info.error = Some(format!("{err}"));
                        if matches!(err, GpuError::DeviceLost | GpuError::OutOfMemory | GpuError::Failed(_) | GpuError::Timeout | GpuError::SurfaceLost) {
                            failed = Some((err, *out, e.devgen));
                        }
                    }
                }
                e.info.draw_us += plat::time_us().saturating_sub(t0);
            }
            // ---- the output draws it where it ends up on top (gp_plan_views)
            let frame = e.sh.c.borrow().frame;
            let fits = e.tex.as_ref().is_some_and(|t| (t.w, t.h) == size && t.group.is_some());
            let apart = can && attached && e.output && rec.layer == 0 && fits && frame != 0;
            rec.apart = apart;
            e.apart = apart;
            if apart {
                wp.dirty.canv.push((rec.index, [*x, *y, *w, *h]));
            }
            // ---- the CPU's way: another frame or another painting, its cells drawn again (the browser: also the frame its
            // read-back brought, which comes later)
            let read = if cfg!(target_arch = "wasm32") { e.sh.c.borrow().image_frame as u32 } else { 0 };
            *ver = rec.ver.wrapping_mul(0x9E37_79B9) ^ (frame as u32).wrapping_mul(0x85EB_CA6B) ^ (frame >> 32) as u32 ^ read.wrapping_mul(0xC2B2_AE35);
        }
        // (their own device, when no view is on it any more)
        if let Own::Ready { devgen, .. } = gview.own {
            if !gview.views.iter().any(|e| e.devgen == devgen) {
                gview.own = Own::None;
                s.note("views: their own device goes");
            }
        }
        if let Some((err, out, devgen)) = failed {
            if out {
                // (every window back on the CPU path now: this frame is hashed and drawn as on it)
                self.gp_failed(i, err);
                self.wp.dirty.split = false;
                self.wp.dirty.canv.clear();
                for rec in self.wp.canv.views.iter_mut() {
                    rec.apart = false;
                }
            } else {
                self.gp_views_gone(Some(devgen));
                self.m.gview.own = Own::Retry(now + RETRY_FIRST_MS * 1000);
            }
        }
    }

    /// dirty_compute, after hashing: the cells the output's render pass draws each view apart in (where it ended
    /// up on top, as a canvas's), a new frame as a canvas's move: those cells owed and stale, the cells with
    /// something over it or under a layer drawn again (their Draw reads the frame back) -> any view drawn so
    pub(crate) fn gp_plan_views(&mut self) -> bool {
        let i = self.uiw_cur as usize;
        if !self.wp.canv.views.iter().any(|v| v.apart) {
            return false;
        }
        let (fw, fh) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        let Ui { m, wp, .. } = self;
        let Mods { gout: s, gview, .. } = m;
        let d = &wp.dirty;
        let (cols, rows) = (d.cols as usize, d.rows as usize);
        let Some(Some(w)) = s.wins.get_mut(i) else { return false };
        let sp = &mut w.sp;
        if sp.hole.len() != cols * rows {
            return false; // (gp_plan_images sizes them where a window can be drawn apart)
        }
        let mut quads = core::mem::take(&mut sp.quads);
        let span = |a: i32, b: i32, n: usize| ((a.max(0) >> 5) as usize, (((b + CELL - 1).max(0) >> 5) as usize).min(n));
        let mut any = false;
        for rec in wp.canv.views.iter().filter(|v| v.apart) {
            let Some(e) = gview.views.iter().find(|e| e.win == i && e.id == rec.id) else { continue };
            let Some(tex) = e.tex.clone() else { continue };
            let moved = e.moved;
            let Some(Cmd::Custom { x, y, w: cw, h: ch, .. }) = wp.cmd.layers[0].get(rec.index as usize) else { continue };
            let seen = d.imgs.iter().position(|s| s.index == rec.index);
            let q = quads.len() as u8 + 1;
            let Some(tag0) = seen.filter(|_| quads.len() < TOP_BLOCKED as usize - 1) else {
                // (not on top anywhere, or too many: the CPU draws its cells, from the frame read back)
                if moved {
                    let (c0, c1) = span(*x, x + cw, cols);
                    let (r0, r1) = span(*y, y + ch, rows);
                    for r in r0..r1 {
                        sp.redraw[r * cols + c0..r * cols + c1.max(c0)].fill(true);
                    }
                    sp.moved = true;
                }
                continue;
            };
            let (tag, vis) = (tag0 as u8 + 1, d.imgs[tag0].vis);
            let (c0, c1) = span(vis[0], vis[2], cols);
            let (r0, r1) = span(vis[1], vis[3], rows);
            for r in r0..r1 {
                for c in c0..c1 {
                    let k = r * cols + c;
                    if d.top.get(k) == Some(&tag) {
                        let b = cell_box(k, cols, fw, fh);
                        sp.hole[k] = q;
                        sp.part[k] = b[0] < vis[0] || b[1] < vis[1] || b[2] > vis[2] || b[3] > vis[3];
                        if moved {
                            sp.owed[k] = true;
                            sp.stale[k] = true;
                        }
                    }
                    if moved {
                        sp.redraw[k] = true;
                    }
                }
            }
            sp.moved |= moved;
            let mut rects = Vec::new();
            let hole = &sp.hole;
            cell_rects(&mut rects, cols, rows, fw, fh, |k| hole[k] == q);
            for r in rects.iter_mut() {
                let (x0, y0) = (r[0].max(vis[0]), r[1].max(vis[1]));
                let (x1, y1) = ((r[0] + r[2]).min(vis[2]), (r[1] + r[3]).min(vis[3]));
                *r = [x0, y0, x1 - x0, y1 - y0];
            }
            rects.retain(|r| r[2] > 0 && r[3] > 0);
            if !rects.is_empty() {
                quads.push(Quad { x: *x, y: *y, src: QuadSrc::View { tex, f: e.draw.clone(), w: *cw, h: *ch, vis }, rects });
                any = true;
            }
        }
        w.sp.quads = quads;
        any
    }

    /// the contents of the views on device generation devgen (None: every device) leave it: detach, their targets go
    /// (the frames read back last stay with their Draw)
    pub(crate) fn gp_views_gone(&mut self, devgen: Option<u32>) {
        for e in self.m.gview.views.iter_mut() {
            if e.devgen != 0 && devgen.is_none_or(|g| g == e.devgen) {
                view_detach(e);
            }
        }
    }

    /// the window loop: contents the app dropped; views at rest: their window settled once, the framebuffer holds
    /// their last frame again (a lost device later loses nothing of the picture)
    fn gp_views_tick(&mut self, now: u64) {
        let views = &mut self.m.gview.views;
        if views.is_empty() {
            return;
        }
        views.retain(|e| e.content.strong_count() > 0);
        // (G5e2, the browser) a read-back answered after the Draw that asked for it: the cells where the output's pass
        // draws the view lack the frame now (owed: any settle, by hand or at rest, takes it there), the next render takes
        // it to the cells under something (gp_views_begin, as a new frame) and the CPU path's cells (its command's
        // hash).  Before, a view at rest kept the frame its last Draw had, an older one
        #[cfg(target_arch = "wasm32")]
        {
            let Mods { gout, gview, .. } = &mut self.m;
            let mut came = false;
            for e in gview.views.iter_mut() {
                let mut c = e.sh.c.borrow_mut();
                let Some(g) = c.gpu.as_ref() else { continue };
                if g.wait.is_none() {
                    // (at rest, with no read-back on its way: the newest frame is asked for here, not only by a Draw,
                    // which may not come again; Firefox 159 kept an older frame without this)
                    if e.rest_at == 0 && c.image_frame != c.frame && c.error.is_none() {
                        c.read();
                    }
                    continue;
                }
                let before = c.image_frame;
                c.read();
                if c.image_frame == before {
                    continue;
                }
                e.arrived = true;
                e.rest_at = now + VIEW_REST_US;
                came = true;
                let Some(Some(w)) = gout.wins.get_mut(e.win) else { continue };
                let sp = &mut w.sp;
                let mine = Rc::as_ptr(&e.draw) as *const ();
                if let Some(q) = sp.quads.iter().position(|q| matches!(&q.src, QuadSrc::View { f, .. } if Rc::as_ptr(f) as *const () == mine)) {
                    let tag = q as u8 + 1;
                    for (owed, hole) in sp.owed.iter_mut().zip(sp.hole.iter()) {
                        if *hole == tag {
                            *owed = true;
                            sp.any_owed = true;
                        }
                    }
                }
            }
            if came {
                self.invalidate();
            }
        }
        let views = &mut self.m.gview.views;
        let mut rest: Vec<usize> = Vec::new();
        for e in views.iter_mut().filter(|e| e.rest_at != 0 && now >= e.rest_at) {
            e.rest_at = 0;
            if !rest.contains(&e.win) {
                rest.push(e.win);
            }
        }
        for i in rest {
            self.gp_settle(i);
        }
    }

    /// (G5e3) the window loop, after the frames still to show: a view whose new frame waited for its window asks for a
    /// render once the window's frame has been shown (or it went back to the CPU path, or is gone), or when it has
    /// waited VIEW_HOLD_US; once per wait (woke: the render clears it, gp_views_begin)
    fn gp_views_wake(&mut self, now: u64) {
        let Mods { gout, gview, .. } = &mut self.m;
        let mut wake = false;
        for e in gview.views.iter_mut().filter(|e| e.held_at != 0 && !e.woke) {
            if !gout.unshown(e.win) || now >= e.held_at + VIEW_HOLD_US {
                e.woke = true;
                wake = true;
            }
        }
        if wake {
            self.invalidate();
        }
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
        let kept = sp.active || !sp.quads.is_empty() || sp.any_stale || sp.moved;
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
        if n == 0 && !stale && !sp.moved {
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
            sp.part = vec![false; total];
            sp.redraw = vec![false; total];
        }
        let Split { upper, m: lowest, nr, rects, hole, owed, stale, quads, linger_at, part, redraw, .. } = sp;
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
            for l in 1..L_COUNT {
                if now[l][k] != now[l - 1][k] {
                    nr[k] |= 1 << l;
                }
            }
            // a canvas moved below: the CPU's way where the GPU does not draw it, and for a layer
            // over it, which holds what lies below
            if redraw[k] && (hole[k] == 0 || nr[k] != 0) {
                low = 0;
            }
            lowest[k] = low as u8;
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
                                quad_blit(fb, q, b, text);
                            }
                            owed[k] = false;
                        }
                        cell_save(fb, b, upper[l - 1].store.put(k, total));
                    }
                }
            }
            let is_base = l == 0;
            cell_rects(rects, cols, rows, fw, fh, |k| (lowest[k] as usize) <= l && (if is_base { hole[k] == 0 || part[k] } else { nr[k] & bit != 0 }));
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
        // (a cell a layer has left in this frame got the old view back from its store)
        if sp.moved {
            for k in 0..total {
                if sp.redraw[k] && sp.hole[k] != 0 && sp.nr[k] == 0 {
                    sp.owed[k] = true;
                }
            }
        }
        sp.any_owed = sp.owed.iter().any(|&v| v);
        sp.any_stale = sp.stale.iter().any(|&v| v);
        sp.redraw.fill(false);
        sp.moved = false;
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
        let font = self.text.get_font();
        let cur = self.uiw_cur as usize;
        let Ui { wp, m, text, .. } = self;
        let parts = if i == cur { Some(wp) } else { m.win.recs[i & (WIN_MAX - 1)].save.as_deref_mut() };
        if let Some(p) = parts {
            let fb = &mut p.rt.fb;
            let (fw, fh) = (fb.w, fb.h);
            let mut rects = Vec::new();
            for (q, quad) in quads.iter().enumerate() {
                let tag = q as u8 + 1;
                cell_rects(&mut rects, cols, rows, fw, fh, |k| owed[k] && hole[k] == tag);
                for r in &rects {
                    quad_blit(fb, quad, [r[0], r[1], r[0] + r[2], r[1] + r[3]], text);
                }
            }
        }
        self.text.set_font(font.0, font.1);
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
            // (Linux: one window without an owner, the first)
            let owner = if cfg!(windows) { plat::PW_NOOWNER } else { s.script_pw.iter().copied().find(|&p| p >= 0).unwrap_or(plat::PW_NOOWNER) };
            pw = plat::win_create("gpu output test", 320, 240, plat::PWF_SYSFRAME | plat::PWF_RESIZABLE, owner, off, off);
            if pw < 0 {
                return -1;
            }
            s.script_pw[i] = pw;
            plat::win_test(plat::WT_NOANIM, pw, [0; 4]);
            plat::win_show(pw, plat::PWS_SHOWNA);
        }
        // (asked once: a window manager that keeps it at another size says so again each time)
        if plat::win_size(pw) != (fw, fh) && s.script_asked[i] != (fw, fh) {
            s.script_asked[i] = (fw, fh);
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
            // (a window the system keeps at another size stays on the CPU path: a narrow tool window
            // in a script's real window with a frame, a frame larger than the screen)
            let all = s.wins.iter().flatten().all(|w| w.stage == Stage::Gpu || kept_at_other_size(w));
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
        if let Some(r) = word_eq(a, "on") {
            self.m.gout.script = true;
            self.m.gout.setup();
            self.m.gout.want = true;
            self.m.gout.want3d = self.desc.flags & AF_GPU_3D != 0;
            self.m.gout.verify = word_eq(skip_ws(r), "fast").is_none();
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
                    _ => {
                        let stages: Vec<(usize, Stage)> = self.m.gout.wins.iter().enumerate().filter_map(|(i, w)| w.as_ref().map(|w| (i, w.stage))).collect();
                        format!("the swapchain did not come: {stages:?}")
                    }
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
            if matches!(self.m.gout.dev, Dev::Ready(_)) {
                let devgen = self.m.gout.dev_gen;
                self.gp_views_gone(Some(devgen)); // (views: their own device with the next frame)
            }
            let s = &mut self.m.gout;
            s.wins.clear();
            s.images.clear();
            s.image_bytes = 0;
            s.canvases.clear();
            s.tile_free.clear();
            s.tile_bytes = 0;
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
        if let Some(n) = word_eq(a, "tilebudget") {
            let (v, _) = super::runtime::parse_int(skip_ws(n));
            self.gpu_test_canvas_budget(v.max(0) as u64);
            return;
        }
        if let Some(r) = word_eq(a, "shot") {
            let r = skip_ws(r);
            self.gp_shot(&String::from_utf8_lossy(r).into_owned());
            return;
        }
        if word_eq(a, "settle").is_some() {
            if !self.m.gout.skip {
                self.gpu_canvas_settle();
                self.render();
                self.present();
            }
            return;
        }
        if word_eq(a, "hold").is_some() {
            let (t, _) = super::runtime::parse_int(next_word(a));
            let dl = plat::time_us() + t.max(0) as u64 * 1000;
            while plat::time_us() < dl {
                script_pump();
                self.gp_tick();
                plat::gpu_wait(plat::time_us() + 10_000);
            }
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
        // (a window manager that keeps the window smaller than the frame, larger than the screen: the
        // window stays on the CPU path; a note, as for a window on top of ours)
        let needs_gpu = ["gpu", "apart", "tiles", "layers", "images"].iter().any(|w| word_eq(r, w).is_some());
        if needs_gpu && self.m.gout.win(i).is_some_and(|w| w.stage != Stage::Gpu && kept_at_other_size(w)) {
            plat::write_stdout(format!("note: gpu expect {}: the system keeps the window at another size (no result)\n", String::from_utf8_lossy(r).trim_end()).as_bytes());
            return;
        }
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
        } else if let Some(n) = word_eq(r, "apart") {
            let (want, _) = super::runtime::parse_int(skip_ws(n));
            let have = self.gpu_output(i as u32).canvas_apart;
            if have != want as u32 {
                self.fail(format!("{have} canvases apart").as_bytes());
            }
        } else if let Some(n) = word_eq(r, "tiles") {
            let (want, _) = super::runtime::parse_int(skip_ws(n));
            let have = self.gpu_output(i as u32).canvas_tiles;
            if have != want as u32 {
                self.fail(format!("{have} canvas tiles").as_bytes());
            }
        } else if let Some(n) = word_eq(r, "fills") {
            let (max, _) = super::runtime::parse_int(skip_ws(n));
            let s = &mut self.m.gout;
            let fills = s.tile_fills - core::mem::replace(&mut s.tile_marked, s.tile_fills);
            if fills > max as u64 {
                self.fail(format!("{fills} tiles drawn").as_bytes());
            }
        } else if word_eq(r, "nodiff").is_some() {
            let o = self.gpu_output(i as u32);
            let px = o.canvas_diff_px - core::mem::replace(&mut self.m.gout.diff_marked, o.canvas_diff_px);
            if px != 0 {
                self.fail(format!("{px} pixels of a canvas's tiles differ, by {} at most in this window so far", o.canvas_diff_max).as_bytes());
            }
        } else {
            self.fail(b"unknown gpu expectation");
        }
    }

    // `gpu shot`: the window presents once more, the image read back into a file
    fn gp_shot(&mut self, file: &str) {
        if self.m.gout.skip {
            return;
        }
        self.render();
        self.present();
        let i = self.gp_scoped();
        self.gp_settle_all();
        let mut s = core::mem::take(&mut self.m.gout);
        let mut raw = None;
        let fb = self.uiw_parts(i as u32).map(|p| &p.rt.fb);
        let (mut w_px, mut h_px) = (0, 0);
        if let (Dev::Ready(core), Some(Some(w)), Some(fb)) = (&s.dev, s.wins.get_mut(i), fb) {
            (w_px, h_px) = (fb.w, fb.h);
            w.rb_keep = Some(Vec::new());
            for _ in 0..SHOW_TRIES {
                match show(w, core, fb) {
                    Ok(Shown::Wait) => plat::gpu_wait(plat::time_us() + 4_000),
                    _ => break,
                }
            }
            raw = w.rb_keep.take().filter(|v| !v.is_empty());
            if core.format == Format::Rgba8Unorm {
                if let Some(v) = raw.as_mut() {
                    for p in v.chunks_exact_mut(4) {
                        p.swap(0, 2);
                    }
                }
            }
        }
        self.m.gout = s;
        script_pump();
        let saved = raw.is_some_and(|v| {
            let mut sf = Surface::new(w_px, h_px, FMT_XRGB);
            sf.px.copy_from_slice(&v);
            Ui::save_png(&sf, file)
        });
        if !saved {
            self.fail(b"no presented image (gpu on fast reads nothing back)");
        }
    }

    // every window on the GPU: present once more, read back, compare; no frame differed since the
    // last check
    fn gp_expect_same(&mut self) {
        if self.m.gout.canvases.iter().any(|c| c.apart) {
            self.gpu_canvas_settle();
            self.render();
            self.present();
        }
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
                for _ in 0..SHOW_TRIES {
                    match show(w, core, fb) {
                        Ok(Shown::Wait) => {
                            plat::gpu_wait(plat::time_us() + 4_000); // no image free now
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
            if s.wins.iter().flatten().any(kept_at_other_size) {
                plat::write_stdout(b"note: gpu expect same: the system keeps the window at another size (no result)\n");
            } else {
                msg = Some(String::from("no window on the swapchain"));
            }
        }
        self.m.gout = s;
        self.m.gout.diff_marked = self.gpu_output(self.gp_scoped() as u32).canvas_diff_px;
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
        // (Linux: Xwayland's GetImage on a window flipped to a buffer reads the buffer before the last
        // one, seen on KWin 6.6 while its own composition shows the last: one more frame of the same
        // content; a Wayland client reads nothing anyway)
        if !cfg!(windows) && screen {
            let mut s = core::mem::take(&mut self.m.gout);
            let fb = self.uiw_parts(i as u32).map(|p| &p.rt.fb);
            if let (Dev::Ready(core), Some(Some(w)), Some(fb)) = (&s.dev, s.wins.get_mut(i), fb) {
                for _ in 0..SHOW_TRIES {
                    if w.stage != Stage::Gpu || !matches!(show(w, core, fb), Ok(Shown::Wait)) {
                        break;
                    }
                    plat::gpu_wait(plat::time_us() + 4_000);
                }
            }
            self.m.gout = s;
        }
        // (Linux: a frame waits for the display's answer to the last one, or for the GPU)
        for _ in 0..SHOW_TRIES {
            let open = self.m.gout.wins.get(i).and_then(|w| w.as_ref()).is_some_and(|w| w.pending || w.sc.as_ref().is_some_and(|sc| sc.busy()));
            if !open {
                break;
            }
            script_pump();
            self.gp_tick();
            plat::gpu_wait(plat::time_us() + 4_000);
        }
        // what was presented reaches the screen with the next compositions
        for _ in 0..12 {
            script_pump();
            self.gp_tick();
            plat::gpu_wait(plat::time_us() + 8_000);
        }
        let Some(fb) = self.uiw_parts(i as u32).map(|p| &p.rt.fb) else { return };
        let (w, h) = (fb.w, fb.h);
        let px = if screen && cfg!(windows) {
            plat::win_get_rect(pw).and_then(|r| plat::win_test_screen(r[0], r[1], w, h))
        } else {
            plat::win_test_image(pw).filter(|x| x.0 == w && x.1 == h).map(|x| x.2)
        };
        let Some(px) = px else {
            if cfg!(windows) {
                self.fail(b"no picture");
            } else {
                // (a Wayland client cannot read a window back; X11: the system keeps it at another size)
                plat::write_stdout(b"note: gpu expect screen: no way to read the window back here (no result)\n");
            }
            return;
        };
        let mut bad = 0u64;
        let mut first = (0, 0);
        let mut some = Vec::new(); // of the pixels that differ, to look what lies over them
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
                    if bad % 4096 == 0 {
                        some.push([x as i32, y as i32, 0, 0]);
                    }
                    bad += 1;
                }
            }
        }
        if bad != 0 {
            let total = w as u64 * h as u64;
            // (not only the first: it can lie in the shadow of a window above, where hit tests find ours)
            if screen && some.iter().any(|&p| plat::win_test(plat::WT_FROM_POINT, pw, p) != 1) {
                plat::write_stdout(format!("note: gpu expect screen: {bad} of {total} pixels differ: another window lies on top of ours\n").as_bytes());
            } else {
                self.fail(format!("{bad} pixels differ, first at {}, {}", first.0, first.1).as_bytes());
            }
        }
    }
}

/// nobody sees the window: the UI hid it, or the system did (a tool window with its minimised owner)
fn unseen(w: &Win) -> bool {
    w.hidden || plat::win_state(w.pw) & plat::WST_HIDDEN != 0
}

/// a script's real window that the system keeps at another size than its frame, for a while now
fn kept_at_other_size(w: &Win) -> bool {
    plat::win_size(w.pw) != w.size && plat::time_us() >= w.stable_at + KEPT_US
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
