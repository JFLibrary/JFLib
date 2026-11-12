// apple_ios_gpu -- the GPU's layer of the only window on iOS (AP6e, docs/APPLE.md): a CAMetalLayer as a
// sublayer of JFView's layer (apple_ios.rs makes it and keeps it at the view's bounds and scale), above
// the picture of the CPU path.  The counterpart of android_gpu.rs on Android.
//
//   win_native(w)          the layer, shown (a program that draws its window with the GPU alone)
//   win_gpu_child(w)       the layer for the UI's output, hidden until win_gpu_child_show; only window 0
//                          and only while nothing else shows on the canvas (popups and tool windows are
//                          composed there, below the layer)
//   win_gpu_child_show     the layer shows the window, or the canvas again
//
// A program without UIKit's host (simctl spawn, tests without a window) has no layer: None.  A scene
// that is not active does no GPU work (apple_ios.rs gpu_hold: win_state says WST_HIDDEN, the output goes
// back to the CPU path before iOS would refuse the command buffers of a background app).

use alloc::vec::Vec;

use crate::apple::ios;

/// the window's GPU layer (a CAMetalLayer, retained: released when this goes) and the window's size in
/// pixels when it was taken (lib/gpu builds swapchains on a worker)
pub struct NativeWindow {
    pub layer: usize,
    pub size: (i32, i32),
}

impl Drop for NativeWindow {
    fn drop(&mut self) {
        ios::gpu_layer_release(self.layer);
    }
}

fn native(w: i32) -> Option<NativeWindow> {
    let layer = ios::gpu_layer()?;
    Some(NativeWindow { layer, size: super::windows::win_size(w) })
}

/// (gpu) the window's GPU layer, shown: the GPU draws the whole window
pub fn win_native(w: i32) -> Option<NativeWindow> {
    if w != 0 {
        return None;
    }
    let nw = native(w)?;
    ios::gpu_layer_show(true);
    Some(nw)
}

/// (gpu) the UI output's layer, hidden until win_gpu_child_show
pub fn win_gpu_child(w: i32) -> Option<NativeWindow> {
    if w != 0 || !super::windows::only_main() {
        return None;
    }
    native(w)
}

/// (gpu) no layer for window w now, but it may come: when the canvas shows nothing else
pub fn win_gpu_child_later(w: i32) -> bool {
    w == 0 && !super::windows::only_main()
}

/// (gpu) the window still shows on the layer alone: nothing else came on the canvas
pub fn win_gpu_child_alone(w: i32) -> bool {
    w == 0 && super::windows::only_main()
}

/// (gpu) the layer shows the window, or the canvas again (the frame that takes it there is whole)
pub fn win_gpu_child_show(w: i32, on: bool) {
    if w == 0 {
        super::windows::gpu_shows(on);
        ios::gpu_layer_show(on);
    }
}

/// (gpu) the frame is on the layer: the canvas's copy of the window goes (it comes back whole)
pub fn win_release_kept(w: i32) {
    super::windows::release_pixels(w);
    if w == 0 {
        ios::gpu_release_picture();
    }
}

/// (gpu) a thread for the GPU output's job
pub fn gpu_thread_spawn(stack: usize, f: alloc::boxed::Box<dyn FnOnce() + Send>) -> Option<crate::Thread> {
    crate::thread_spawn(stack, f)
}

/// (gpu) how the output came up: Android keeps it across runs
pub fn gpu_out_state(_state: u32) {}

/// (gpu) UIKit's loop calls the host's tick: nothing waits here
pub fn gpu_wait(_deadline_us: u64) {}

/// (gpu) no live resize loop on iOS
pub fn win_in_sizemove() -> bool {
    false
}

/// (gpu) one GPU on an iPhone or iPad
pub fn win_display_adapter(_w: i32) -> Option<(u32, u32)> {
    None
}

/// (gpu) no remote sessions on iOS
pub fn win_remote_session() -> bool {
    false
}

// (tests) what the desktop's tests ask: there is no window system to ask on this target

pub const WT_PIXEL: u32 = 1;
pub const WT_POPUP_RECT: u32 = 2;
pub const WT_MOVE: u32 = 3;
pub const WT_SHOW: u32 = 4;
pub const WT_DPI: u32 = 5;
pub const WT_REDRAW: u32 = 6;
pub const WT_NOANIM: u32 = 16;
pub const WT_FROM_POINT: u32 = 17;

/// -1: nothing to say about the pixel or the scale; 0 for the rest
pub fn win_test(op: u32, _w: i32, _a: [i32; 4]) -> i64 {
    if op == WT_PIXEL || op == WT_DPI { -1 } else { 0 }
}

pub fn win_test_image(_w: i32) -> Option<(i32, i32, Vec<u8>)> {
    None
}

pub fn win_test_screen(_x: i32, _y: i32, _w: i32, _h: i32) -> Option<Vec<u8>> {
    None
}

pub fn win_test_send(_w: i32, _msg: u32, _wp: usize, _lp: isize) -> isize {
    0
}

/// (bench: Wayland's presentation feedback) nothing here
pub fn gpu_test_presented() -> Vec<(u64, u64)> {
    Vec::new()
}
