// android_gpu -- what the GPU layer and the UI's output through the GPU need of Android
// (lib/gpu vk_load.rs, vk_swap.rs; lib/ui/gpu_out.rs; docs/GPU.md "Ausgabe unter Android")
//
//   dl_open(name) / dl_sym(handle, name) / dl_close(handle)   libvulkan.so at run time: no
//                                      DT_NEEDED, the program starts on a phone without it
//   gpu_thread_spawn(stack, f)         a real thread for the output's jobs.  thread_spawn stays
//                                      None here: the pools and tasks of this target are cooperative
//                                      and start nothing behind the UI's back; nothing but the
//                                      output's jobs, which end by themselves, runs on these
//   win_gpu_child(w)                   the surface of the SurfaceView (ANativeWindow), the analogue
//                                      of Windows' child window; only for window 0 while nothing
//                                      else shows on the canvas (tool windows and popups are
//                                      composed on the CPU, below the surface)
//   win_gpu_child_later(w)             no surface now, a frame or the host brings it: not a reason
//                                      to give up, and nothing to poll for
//   win_gpu_child_show(w, show)        show: the frame is on the surface, the canvas path rests;
//                                      hide: the frame is on its way to the Bitmap, the host takes
//                                      the SurfaceView away (the surface goes with it)
//   win_release_kept(w)                the canvas's copy of the window goes
//
// The surface comes and goes with the system (GalleryActivity's SurfaceHolder.Callback, bridge.c):
// jfgui_gpu_surface(window) on the UI thread.  bridge.c owns the ANativeWindow reference; the
// VkSurfaceKHR takes its own, so a worker still building on a surface that went away gets errors,
// not a dangling pointer.

use alloc::boxed::Box;
use core::{
    ffi::c_void,
    sync::atomic::{AtomicUsize, Ordering},
};

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(name: *const u8, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const u8) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> i32;
}

#[repr(C, align(8))]
struct PthreadAttr([u64; 8]);

#[link(name = "c")]
unsafe extern "C" {
    fn pthread_attr_init(attr: *mut PthreadAttr) -> i32;
    fn pthread_attr_setstacksize(attr: *mut PthreadAttr, size: usize) -> i32;
    fn pthread_attr_destroy(attr: *mut PthreadAttr) -> i32;
    fn pthread_create(out: *mut usize, attr: *const PthreadAttr, start: extern "C" fn(*mut c_void) -> *mut c_void, arg: *mut c_void) -> i32;
    fn pthread_join(thread: usize, out: *mut *mut c_void) -> i32;
}

/// open a shared library (a name as the linker searches it, or a path) -> its handle
pub fn dl_open(name: &str) -> Option<usize> {
    let mut z = alloc::vec::Vec::with_capacity(name.len() + 1);
    z.extend_from_slice(name.as_bytes());
    z.push(0);
    // SAFETY: z is NUL-terminated and outlives the call; RTLD_NOW (2), local to this library.
    let h = unsafe { dlopen(z.as_ptr(), 2) };
    (!h.is_null()).then_some(h as usize)
}

/// a symbol of the library (name NUL-terminated) -> its address, 0 when missing
pub fn dl_sym(handle: usize, name: &[u8]) -> usize {
    debug_assert!(name.last() == Some(&0));
    // SAFETY: a handle of dl_open; name is NUL-terminated.
    unsafe { dlsym(handle as *mut c_void, name.as_ptr()) as usize }
}

/// give the library back
pub fn dl_close(handle: usize) {
    // SAFETY: a handle of dl_open, closed once.
    unsafe { dlclose(handle as *mut c_void) };
}

type Start = Box<dyn FnOnce() + Send>;

extern "C" fn thread_main(arg: *mut c_void) -> *mut c_void {
    // SAFETY: gpu_thread_spawn hands over this box, once.
    let f = unsafe { Box::from_raw(arg as *mut Start) };
    f();
    core::ptr::null_mut()
}

/// a real thread with stack bytes of stack (the output's jobs) -> its handle
pub fn gpu_thread_spawn(stack: usize, f: Start) -> Option<super::Thread> {
    let arg = Box::into_raw(Box::new(f));
    let mut attr = PthreadAttr([0; 8]);
    let mut handle = 0usize;
    // SAFETY: attr is at least as large as bionic's pthread_attr_t and lives across the calls;
    // thread_main takes the box when the thread started, else it is taken back below.
    let r = unsafe {
        pthread_attr_init(&mut attr);
        pthread_attr_setstacksize(&mut attr, stack);
        let r = pthread_create(&mut handle, &attr, thread_main, arg as *mut c_void);
        pthread_attr_destroy(&mut attr);
        r
    };
    if r != 0 {
        // SAFETY: no thread started, the box is still ours.
        drop(unsafe { Box::from_raw(arg) });
        return None;
    }
    Some(super::Thread { handle })
}

pub(super) fn thread_join(handle: usize) {
    // SAFETY: a thread of gpu_thread_spawn, joined once (Thread::join takes self).
    unsafe { pthread_join(handle, core::ptr::null_mut()) };
}

/// a target for swapchains: the ANativeWindow of the SurfaceView
#[derive(Clone, Copy, Debug)]
pub struct NativeWindow {
    pub window: usize,
}

// the surface's ANativeWindow, 0 while there is none
static SURFACE: AtomicUsize = AtomicUsize::new(0);

/// (bridge.c) the surface came (an ANativeWindow) or went (0; nothing uses it when this returns)
/// -> 1: the frame waits to go to the canvas, the host renders once more
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_gpu_surface(window: usize) -> u32 {
    SURFACE.store(window, Ordering::Release);
    (window == 0 && super::surface_gone()) as u32
}

pub const CALL_GPU_SURFACE: u32 = 60;

fn ask_surface(show: bool) {
    let _ = super::android_host::call(CALL_GPU_SURFACE, &[show as u8]);
}

/// the surface for window w; None: see win_gpu_child_later
pub fn win_gpu_child(w: i32) -> Option<NativeWindow> {
    if w != 0 || !super::windows::only_main() {
        return None;
    }
    match SURFACE.load(Ordering::Acquire) {
        0 => {
            ask_surface(true); // (the host wakes the loop when it is there)
            None
        }
        window => Some(NativeWindow { window }),
    }
}

/// no surface for window w now, but it may come: the host or the next frame wakes the loop
pub fn win_gpu_child_later(w: i32) -> bool {
    w != 0 || !super::windows::only_main() || SURFACE.load(Ordering::Acquire) == 0
}

/// the window still shows on its surface alone: nothing else came on the canvas
pub fn win_gpu_child_alone(w: i32) -> bool {
    w == 0 && super::windows::only_main()
}

/// show: the window shows on the surface; hide: on the canvas, the surface goes
pub fn win_gpu_child_show(w: i32, show: bool) {
    if w == 0 {
        super::windows::gpu_shows(show);
        if !show {
            ask_surface(false);
        }
    }
}

/// the frame is on the surface: the canvas's copy of the window goes (it comes back whole)
pub fn win_release_kept(w: i32) {
    super::windows::release_pixels(w);
}

/// no modal size loop here
pub fn win_in_sizemove() -> bool {
    false
}

/// one GPU, which drives the display
pub fn win_display_adapter(_w: i32) -> Option<(u32, u32)> {
    None
}

/// no remote sessions here
pub fn win_remote_session() -> bool {
    false
}
