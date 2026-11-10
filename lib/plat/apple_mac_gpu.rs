// apple_mac_gpu -- the GPU's layer in a window on macOS (AP6, docs/APPLE.md): a CAMetalLayer as a
// sublayer of the view's layer, above the picture the CPU path puts into that layer (win_present).
// lib/gpu's Metal swapchain draws into it; the counterpart of the child window on Windows and the
// SurfaceView on Android.
//
//   win_native(w)          the window's GPU layer, shown (a program that draws the window with the
//                          GPU alone: gputest)
//   win_gpu_child(w)       the same layer for the UI's output, hidden until win_gpu_child_show:
//                          the CPU's picture stays visible below it until the GPU's first frame
//   win_gpu_child_show     show or hide it (hiding brings the CPU path's picture back at once:
//                          it is the view layer's contents under the sublayer)
//
// The layer follows the view's bounds (autoresizing), its contentsScale follows the window's
// backingScaleFactor (size_changed); the drawable size in pixels is lib/gpu's.  Main thread only,
// as all of AppKit; lib/gpu may use the layer on its own thread (nextDrawable, drawableSize).

use super::*;

/// a window's GPU layer (a CAMetalLayer, retained: released when this goes)
pub struct NativeWindow {
    pub layer: usize,
}

impl Drop for NativeWindow {
    fn drop(&mut self) {
        release(self.layer as Id);
    }
}

/// the window's CAMetalLayer, made at the first call (main thread)
fn gpu_layer(w: i32) -> Option<Id> {
    if let Some(l) = rec(w, |r| r.gpu).filter(|l| !l.is_null()) {
        return Some(l);
    }
    let (_, view, scale) = ids(w)?;
    // SAFETY: Core Animation objects on the main thread; [CAMetalLayer layer] is autoreleased and
    // retained here (the window record keeps it until win_destroy).
    let layer = unsafe {
        let cls = class(c"CAMetalLayer");
        if cls.is_null() {
            return None;
        }
        let l: Id = msg!(cls, c"layer"; Id);
        if l.is_null() {
            return None;
        }
        msg!(l, c"retain"; Id);
        let host: Id = msg!(view, c"layer"; Id);
        if host.is_null() {
            release(l);
            return None;
        }
        let tx = class(c"CATransaction");
        msg!(tx, c"begin"; ());
        msg!(tx, c"setDisableActions:", true => bool; ());
        let b: NSRect = msg!(host, c"bounds"; NSRect);
        msg!(l, c"setFrame:", b => NSRect; ());
        msg!(l, c"setAutoresizingMask:", 18u32 => u32; ()); // kCALayerWidthSizable | kCALayerHeightSizable
        msg!(l, c"setContentsGravity:", kCAGravityTopLeft => Id; ());
        msg!(l, c"setContentsScale:", scale => f64; ());
        msg!(l, c"setOpaque:", true => bool; ());
        msg!(l, c"setHidden:", true => bool; ());
        msg!(host, c"addSublayer:", l => Id; ());
        msg!(tx, c"commit"; ());
        l
    };
    rec(w, |r| r.gpu = layer);
    Some(layer)
}

fn retained(l: Id) -> NativeWindow {
    // SAFETY: a live layer; the NativeWindow owns this reference.
    unsafe { msg!(l, c"retain"; Id) };
    NativeWindow { layer: l as usize }
}

/// (gpu) the window's GPU layer, shown: the GPU draws the whole window (do not call win_present
/// for it; the view layer's own picture stays below, hidden)
pub fn win_native(w: i32) -> Option<NativeWindow> {
    let l = gpu_layer(w)?;
    show(l, true);
    Some(retained(l))
}

/// (gpu) the UI output's layer of window w, hidden until win_gpu_child_show
pub fn win_gpu_child(w: i32) -> Option<NativeWindow> {
    gpu_layer(w).map(retained)
}

fn show(l: Id, on: bool) {
    // SAFETY: a layer of ours, on the main thread, in a transaction without animation.
    unsafe {
        let tx = class(c"CATransaction");
        msg!(tx, c"begin"; ());
        msg!(tx, c"setDisableActions:", true => bool; ());
        msg!(l, c"setHidden:", !on => bool; ());
        msg!(tx, c"commit"; ());
    }
}

/// (gpu) show or hide the UI output's layer of window w
pub fn win_gpu_child_show(w: i32, on: bool) {
    if let Some(l) = rec(w, |r| r.gpu).filter(|l| !l.is_null()) {
        show(l, on);
    }
}

/// (gpu) the layer comes at once (Android: the surface comes later)
pub fn win_gpu_child_later(_w: i32) -> bool {
    false
}

/// (gpu) nothing else is composed over the window's layer (popups are windows of their own)
pub fn win_gpu_child_alone(_w: i32) -> bool {
    true
}

/// (gpu) the PCI ids of the GPU that shows the window: none on a Mac (one GPU per screen at most)
pub fn win_display_adapter(_w: i32) -> Option<(u32, u32)> {
    None
}

/// (gpu) a thread for the GPU output's job (a plain plat thread)
pub fn gpu_thread_spawn(stack: usize, f: alloc::boxed::Box<dyn FnOnce() + Send>) -> Option<crate::Thread> {
    crate::thread_spawn(stack, f)
}

/// the layer goes with its window (win_destroy)
pub(super) fn drop_layer(l: Id) {
    if l.is_null() {
        return;
    }
    // SAFETY: our layer, removed from its host and released once.
    unsafe { msg!(l, c"removeFromSuperlayer"; ()) };
    release(l);
}

/// the layer's scale follows the window's (size_changed)
pub(super) fn layer_scale(l: Id, scale: f64) {
    if l.is_null() {
        return;
    }
    // SAFETY: our layer, on the main thread, without animation.
    unsafe {
        let tx = class(c"CATransaction");
        msg!(tx, c"begin"; ());
        msg!(tx, c"setDisableActions:", true => bool; ());
        msg!(l, c"setContentsScale:", scale => f64; ());
        msg!(tx, c"commit"; ());
    }
}

/// (test aid) the process's CPU time (us), resident memory and its own footprint (bytes), as on
/// Windows and Linux (render3dtest and uigputest measure with it)
pub fn win_test_usage() -> (u64, u64, u64) {
    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut u64) -> i32;
        static mach_task_self_: u32;
        fn task_info(task: u32, flavor: u32, info: *mut u64, count: *mut u32) -> i32;
    }
    let mut ru = [0u64; 18]; // struct rusage: two timevals (16 bytes each) first
    let mut vm = [0u64; 48]; // task_vm_info: resident_size at word 2, phys_footprint at word 19
    let mut count = (vm.len() * 2) as u32; // in natural_t (4 bytes)
    // SAFETY: getrusage(RUSAGE_SELF) into a buffer as large as struct rusage; task_info with
    // TASK_VM_INFO (22) into a buffer of `count` natural_t; mach_task_self_ is the task's port.
    let (r, k) = unsafe { (getrusage(0, ru.as_mut_ptr()), task_info(mach_task_self_, 22, vm.as_mut_ptr(), &mut count)) };
    let tv = |i: usize| ru[i * 2] * 1_000_000 + (ru[i * 2 + 1] & 0xffff_ffff);
    let cpu = if r == 0 { tv(0) + tv(1) } else { 0 };
    if k != 0 {
        return (cpu, 0, 0);
    }
    (cpu, vm[2], vm[19])
}
