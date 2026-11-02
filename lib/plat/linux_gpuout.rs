// linux_gpuout -- what lib/gpu's images need from the window system: dmabuf buffers on a window,
// through JFLib's own Wayland and X11 code (programs against glibc only; docs/GPU.md "Ausgabe unter
// Linux").  No libwayland, no libxcb: Vulkan's surfaces would want a pointer from them, so the GPU
// draws into images of its own and this code hands them to the window.
//
//   win_gpu_child(w) -> NativeWindow   what the display accepts for window w (format + modifier
//                                      pairs, its render device) and the window's size: None = the
//                                      CPU path.  The name is the Windows presenter's, whose child
//                                      window this stands for; a Linux window needs no child: one
//                                      wl_surface or X window takes shared memory and dmabufs in turn
//   win_gpu_child_show, win_release_kept, win_in_sizemove, win_display_adapter, win_remote_session
//                                      nothing to do or know here
//   gpu_buf_new(w, &GpuBufDesc) -> handle     a buffer of window w from its planes (takes the fds);
//                                      0: refused.  On Wayland its answer comes later: PENDING first
//   gpu_buf_state(w, handle) -> GPU_BUF_*     FREE: nobody reads it, it is not the one on the window
//   gpu_buf_free(w, handle)            the buffer goes (also while it is on the window: what it shows
//                                      stays, as long as nobody writes the memory)
//   gpu_can_show(w) -> bool            a new frame is welcome now: the last commit has been answered
//                                      (the frame callback, PresentCompleteNotify; 1 s at most)
//   gpu_show(w, handle, rects)         the buffer, finished by the GPU, goes on the window (at once, or
//                                      as soon as the window can take it); rects: what changed against
//                                      the last one, None = all.  From now on window w shows through
//                                      buffers, until the next win_present.
//   gpu_next_look(w) -> time_us        when a frame that cannot go on yet should look again; the wait
//                                      returns earlier when the answer comes
//   gpu_wait(deadline_us)              pump the display until the deadline, also with events queued
//                                      (they stay queued; a plain wait would not read the socket)
//
// Only the thread of the window loop calls these (the backends are single-threaded: linux_disp.rs).
//
// No wrong frame can show:
//   * a buffer goes to the window only after the GPU has finished it (lib/gpu: Swapchain::flush
//     looks at the timeline without waiting).  So nothing depends on implicit sync: at the attach no
//     fence is pending on the buffer, on any compositor, kernel or driver.
//   * a buffer is drawn into only after the compositor gave it back (wl_buffer.release,
//     PresentIdleNotify) and no read of it is pending on its dma-buf (poll POLLOUT: the compositor's
//     GPU may still be reading it when the release arrives), and never while it is the newest.
//   * win_present ends the buffers' turn: a frame still on its way is dropped, shared memory shows
//     the whole frame.
// Test aids: win_test_image(w) reads what the window system holds where it can (X11: GetImage).

use alloc::vec::Vec;

use super::disp::{disp_backend, BK_WAYLAND, BK_X11};

/// a buffer's state (gpu_buf_state)
pub const GPU_BUF_PENDING: u32 = 0;
pub const GPU_BUF_FREE: u32 = 1;
pub const GPU_BUF_BUSY: u32 = 2;
pub const GPU_BUF_FAILED: u32 = 3;

/// what a display accepts as buffers
#[derive(Clone, Debug, Default)]
pub struct GpuCaps {
    /// the device it renders with (dev_t of a node of the DRM device), when it says so
    pub device: Option<u64>,
    /// DRM fourcc and modifier
    pub formats: Vec<(u32, u64)>,
}

/// a plane of a buffer: a dma-buf (owned: closed once sent), its offset and row pitch
#[derive(Clone, Copy, Debug)]
pub struct GpuPlane {
    pub fd: i32,
    pub offset: u32,
    pub stride: u32,
}

/// a buffer of memory the GPU drew into
#[derive(Clone, Debug)]
pub struct GpuBufDesc {
    pub w: i32,
    pub h: i32,
    pub fourcc: u32,
    pub modifier: u64,
    pub planes: Vec<GpuPlane>,
}

/// the Linux form of the handles lib/gpu gets for a window (Windows: HWND, HINSTANCE)
#[derive(Clone, Debug)]
pub struct NativeWindow {
    pub caps: GpuCaps,
    pub size: (i32, i32),
}

pub fn win_gpu_child(w: i32) -> Option<NativeWindow> {
    let caps = match disp_backend() {
        BK_WAYLAND => super::wl::gpuw::gpu_caps(),
        BK_X11 => super::x11::gpux::gpu_caps(w),
        _ => None,
    }?;
    if caps.formats.is_empty() {
        return None;
    }
    Some(NativeWindow { caps, size: crate::win_size(w) })
}

pub fn win_gpu_child_show(_w: i32, _show: bool) {}

pub fn win_release_kept(_w: i32) {}

pub fn win_in_sizemove() -> bool {
    false
}

/// the GPU that drives the window's monitor: not known here, the display's device decides (GpuCaps)
pub fn win_display_adapter(_w: i32) -> Option<(u32, u32)> {
    None
}

pub fn win_remote_session() -> bool {
    false
}

pub fn gpu_buf_new(w: i32, d: &GpuBufDesc) -> u32 {
    match disp_backend() {
        BK_WAYLAND => super::wl::gpuw::gpu_buf_new(w, d),
        BK_X11 => super::x11::gpux::gpu_buf_new(w, d),
        _ => {
            close_planes(d);
            0
        }
    }
}

/// the descriptors of a buffer nobody takes
pub(crate) fn close_planes(d: &GpuBufDesc) {
    for p in &d.planes {
        // SAFETY: closes a descriptor the caller handed over.
        unsafe { rt::os::syscall3(rt::os::SYS_CLOSE, p.fd as usize, 0, 0) };
    }
}

pub fn gpu_buf_state(w: i32, h: u32) -> u32 {
    match disp_backend() {
        BK_WAYLAND => super::wl::gpuw::gpu_buf_state(w, h),
        BK_X11 => super::x11::gpux::gpu_buf_state(w, h),
        _ => GPU_BUF_FAILED,
    }
}

pub fn gpu_buf_free(w: i32, h: u32) {
    match disp_backend() {
        BK_WAYLAND => super::wl::gpuw::gpu_buf_free(w, h),
        BK_X11 => super::x11::gpux::gpu_buf_free(w, h),
        _ => {}
    }
}

pub fn gpu_can_show(w: i32) -> bool {
    match disp_backend() {
        BK_WAYLAND => super::wl::gpuw::gpu_can_show(w),
        BK_X11 => super::x11::gpux::gpu_can_show(w),
        _ => false,
    }
}

pub fn gpu_show(w: i32, h: u32, rects: Option<&[[i32; 4]]>) {
    match disp_backend() {
        BK_WAYLAND => super::wl::gpuw::gpu_show(w, h, rects),
        BK_X11 => super::x11::gpux::gpu_show(w, h, rects),
        _ => {}
    }
}

pub fn gpu_next_look(w: i32) -> u64 {
    match disp_backend() {
        BK_WAYLAND => super::wl::gpuw::gpu_next_look(w),
        BK_X11 => super::x11::gpux::gpu_next_look(w),
        _ => u64::MAX,
    }
}

pub fn gpu_wait(deadline_us: u64) {
    let now = crate::time_us();
    let ms = (deadline_us.saturating_sub(now).div_ceil(1000)).min(1000) as i32;
    match disp_backend() {
        BK_WAYLAND => super::wl::gpuw::gpu_pump(ms),
        BK_X11 => super::x11::gpux::gpu_pump(ms),
        _ => crate::sleep_ms(ms as u32),
    }
}

// ------------------------------------------------------------------ test aids, as on Windows

pub const WT_NOANIM: u32 = 16;
pub const WT_FROM_POINT: u32 = 17;

/// (tests) WT_NOANIM: nothing to turn off; WT_FROM_POINT: nobody tells a client what lies on top of
/// its window: it counts as ours
pub fn win_test(_op: u32, _w: i32, _a: [i32; 4]) -> i64 {
    1
}

/// (tests) what the window system holds for window w: (w, h, rows of w * 4 bytes): X11 reads it
/// back (GetImage; also a buffer on the window), Wayland has no way to: None
pub fn win_test_image(w: i32) -> Option<(i32, i32, Vec<u8>)> {
    match disp_backend() {
        BK_X11 => super::x11::gpux::gpu_test_image(w),
        _ => None,
    }
}

/// (tests) a part of the screen: no client on Linux can read it without asking the user
pub fn win_test_screen(_x: i32, _y: i32, _w: i32, _h: i32) -> Option<Vec<u8>> {
    None
}
