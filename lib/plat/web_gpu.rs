// web_gpu -- what the UI's output through the GPU needs of the browser (lib/ui/gpu_out.rs, lib/gpu
// web_*.rs, web/gpu.js, web/loader.js; docs/GPU.md "Ausgabe im Browser")
//
//   win_gpu_child(w)                   a canvas above the page's #screen as an OffscreenCanvas in this
//                                      worker, the analogue of Android's SurfaceView: only for window 0
//                                      while nothing else shows on #screen (tool windows and popups are
//                                      composed on the CPU, below it).  None: the page was asked, the canvas
//                                      is on its way; each surface takes a new one
//   win_gpu_child_later(w)             no canvas now, it comes: no reason to give up, nothing to poll for
//   win_gpu_child_alone(w)             the window still shows on it alone
//   win_gpu_child_show(w, show)        show: the frame is on the canvas, #screen rests with its last frame
//                                      under it.  hide: the frame is on its way to #screen, the page
//                                      takes the canvas away after it (after_flush): no gap, no wrong frame
//   win_release_kept(w)                the copy of the window for #screen goes, and the worker's pooled
//                                      pixel buffers with it
//   win_gpu_child_blank()              the page found that the canvas shows nothing (once)
//
// A canvas that has shown nothing yet shows nothing: until the first frame reaches the compositor the
// page's #screen is seen through it, the same pixels (the switch needs no message and no timing).  A
// new period on the GPU takes a new canvas, so it can never show an old frame.  The worker's thread is the
// only one that touches any of this: there are no job threads here, in none of the three bundles (the
// compute workers of the threads bundle cannot own WebGPU objects); gpu_out.rs runs its jobs on this
// thread when the browser has answered.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// (browser) the output wants a canvas above #screen; the page answers with jfgui_gpu_canvas
pub const REQUEST_GPU_CANVAS: u32 = 28;
/// (browser) the canvas above #screen goes (after the frame that replaces it)
pub const REQUEST_GPU_HIDE: u32 = 29;
/// (browser) the canvas above #screen shows: the worker's pixel buffers for the page may go
pub const REQUEST_GPU_SHOWN: u32 = 30;

// the canvas that came, 0: none
static CANVAS: AtomicU32 = AtomicU32::new(0);
// the page was asked, its answer is on the way
static ASKED: AtomicBool = AtomicBool::new(false);
// the next flush takes the canvas away
static HIDE: AtomicBool = AtomicBool::new(false);
// the page's watcher found that the canvas shows nothing
static BLANK: AtomicBool = AtomicBool::new(false);

/// (worker.js) the page's canvas, as a number of web/gpu.js's table
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_gpu_canvas(handle: u32) {
    CANVAS.store(handle, Ordering::Release);
    ASKED.store(false, Ordering::Release);
    super::wake();
}

/// (worker.js) the page's watcher (watchGpuCanvas in loader.js): the canvas shows nothing
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_gpu_blank() {
    BLANK.store(true, Ordering::Release);
    super::wake();
}

/// the page found that the canvas shows nothing, though everything computes right; asked once
pub fn win_gpu_child_blank() -> bool {
    BLANK.swap(false, Ordering::AcqRel)
}

pub(super) fn after_flush() {
    if HIDE.swap(false, Ordering::AcqRel) {
        super::notice(REQUEST_GPU_HIDE, &[]);
    }
}

/// a target for swapchains: the OffscreenCanvas of the page's canvas, a number of web/gpu.js's table
#[derive(Debug)]
pub struct NativeWindow {
    canvas: u32,
}

impl NativeWindow {
    /// the number, once (lib/gpu's Surface owns it then)
    pub fn take(self) -> u32 {
        self.canvas
    }
}

/// how the output came up: nothing keeps it across runs here
pub fn gpu_out_state(_state: u32) {}

/// the canvas for window w; None: see win_gpu_child_later
pub fn win_gpu_child(w: i32) -> Option<NativeWindow> {
    if w != 0 || !super::windows::only_main() {
        return None;
    }
    match CANVAS.swap(0, Ordering::AcqRel) {
        0 => {
            if !ASKED.swap(true, Ordering::AcqRel) {
                super::notice(REQUEST_GPU_CANVAS, &[]);
            }
            None
        }
        canvas => Some(NativeWindow { canvas }),
    }
}

/// no canvas for window w now, but it may come: the page or the next frame wakes the loop
pub fn win_gpu_child_later(w: i32) -> bool {
    w != 0 || !super::windows::only_main() || (CANVAS.load(Ordering::Acquire) == 0 && ASKED.load(Ordering::Acquire))
}

/// the window still shows on its canvas alone: nothing else came on #screen
pub fn win_gpu_child_alone(w: i32) -> bool {
    w == 0 && super::windows::only_main()
}

/// show: the window shows on the canvas above; hide: on #screen, the canvas above goes after that frame
pub fn win_gpu_child_show(w: i32, show: bool) {
    if w == 0 {
        super::windows::gpu_shows(show);
        if show {
            super::notice(REQUEST_GPU_SHOWN, &[]);
        } else {
            HIDE.store(true, Ordering::Release);
        }
    }
}

/// the frame is on the canvas above: the copy for #screen goes (it comes back whole)
pub fn win_release_kept(w: i32) {
    super::windows::release_pixels(w);
}

/// no modal size loop here
pub fn win_in_sizemove() -> bool {
    false
}

/// the browser chooses the GPU
pub fn win_display_adapter(_w: i32) -> Option<(u32, u32)> {
    None
}

/// no remote sessions to look for here
pub fn win_remote_session() -> bool {
    false
}

// the script command `gpu` (a script's real windows) has nothing to work with here: scripts run on the
// desktop, this target has its own test (apps/gallery/tests/gpu_web.mjs)
pub const WT_NOANIM: u32 = 0;
pub const WT_FROM_POINT: u32 = 0;
pub fn win_test(_op: u32, _w: i32, _a: [i32; 4]) -> i64 {
    0
}
pub fn win_test_image(_w: i32) -> Option<(i32, i32, alloc::vec::Vec<u8>)> {
    None
}
pub fn win_test_screen(_x: i32, _y: i32, _w: i32, _h: i32) -> Option<alloc::vec::Vec<u8>> {
    None
}
pub fn gpu_wait(_deadline_us: u64) {}
