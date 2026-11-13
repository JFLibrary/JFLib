// win_gpuout -- the child window a window's GPU output draws on (lib/ui's presenter, docs/GPU.md)
//
//   win_gpu_child(w) -> NativeWindow     the child of window w for a swapchain, created hidden on
//                                        the first call and given the client size of w
//   win_gpu_child_show(w, show)          show it (sized to the client area) / hide it
//   win_gpu_child_destroy(w)             destroy it (a swapchain on it must be gone)
//   win_release_kept(w)                  free the copy WM_PAINT repaints from (the child shows
//                                        the window; the next win_present makes a new copy)
//   win_in_sizemove()                    the modal move / size loop runs
//   win_display_adapter(w) -> (vendor id, device id)   the GPU that drives the window's monitor
//   win_remote_session()                 this is a Remote Desktop session
//   gpu_wait(deadline_us)                pump the messages until then (Linux: linux_gpuout.rs)
//
// Why a child: once a swapchain has presented on a window, what GDI draws on that window stays
// invisible, also after the swapchain and its surface are destroyed (measured: DWM keeps showing
// the last image; gputest and tests/ of uigputest).  So the top-level window stays the CPU path
// for its whole life; the swapchain lives on a child that covers its client area.  Hiding the
// child shows at once what the CPU path last drew; a child that is shown before its first
// present shows that too (a child has no redirection surface of its own and this one never
// paints).
//
// The child takes no input: WS_DISABLED and HTTRANSPARENT send everything to the parent (hit
// tests with the parent's resize band included), WindowFromPoint does not find it, it never has
// the focus.  It is destroyed together with its parent.

use super::{BeginPaint, CreateWindowExW, DefWindowProcW, DestroyWindow, EndPaint, GetClientRect, GetMonitorInfoW, GetSystemMetrics,
            MonitorFromWindow, MonitorInfo, NativeWindow, PaintStruct, Rect, RegisterClassExW, SetWindowPos, ShowWindow, UiOnly, WndClassEx,
            hinst, rec, st};
use core::cell::Cell;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn EnumDisplayDevicesW(device: *const u16, n: u32, dd: *mut DisplayDevice, flags: u32) -> i32;
}

/// MONITORINFOEXW
#[repr(C)]
struct MonitorInfoEx {
    info: MonitorInfo,
    device: [u16; 32],
}

/// DISPLAY_DEVICEW
#[repr(C)]
struct DisplayDevice {
    cb: u32,
    name: [u16; 32],
    string: [u16; 128],
    flags: u32,
    id: [u16; 128],
    key: [u16; 128],
}

const WS_CHILD: u32 = 0x4000_0000;
const WS_DISABLED: u32 = 0x0800_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const SW_HIDE: i32 = 0;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_SHOWWINDOW: u32 = 0x0040;
const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_NCHITTEST: u32 = 0x0084;
const HTTRANSPARENT: isize = -1;
const MONITOR_NEAREST: u32 = 2;
const SM_REMOTESESSION: i32 = 0x1000;
static CLASS_NAME: [u16; 12] = [0x4A, 0x66, 0x47, 0x75, 0x69, 0x47, 0x70, 0x75, 0x4F, 0x75, 0x74, 0]; // "JfGuiGpuOut"

static CLASS: UiOnly<Cell<bool>> = UiOnly::new(Cell::new(false));

unsafe extern "system" fn gc_proc(hwnd: isize, msg: u32, wp: usize, lp: isize) -> isize {
    match msg {
        WM_NCHITTEST => HTTRANSPARENT,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            // SAFETY: plain Win32 paint bracket: nothing is drawn, the swapchain owns the pixels.
            let mut ps: PaintStruct = unsafe { core::mem::zeroed() };
            unsafe {
                BeginPaint(hwnd, &mut ps);
                EndPaint(hwnd, &ps);
            }
            0
        }
        // SAFETY: default handling of the message.
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn class() -> bool {
    if CLASS.get().get() {
        return true;
    }
    // SAFETY: a valid WNDCLASSEXW; the class name is static.
    let ok = unsafe {
        let wc = WndClassEx {
            size: core::mem::size_of::<WndClassEx>() as u32,
            style: 0,
            proc_: gc_proc,
            cls_extra: 0,
            wnd_extra: 0,
            inst: hinst(),
            icon: 0,
            cursor: 0,
            brush: 0,
            menu: core::ptr::null(),
            class: CLASS_NAME.as_ptr(),
            icon_sm: 0,
        };
        RegisterClassExW(&wc) != 0
    };
    CLASS.get().set(ok);
    ok
}

fn client(parent: isize) -> (i32, i32) {
    let mut c = Rect::default();
    // SAFETY: our window, a valid RECT.
    unsafe { GetClientRect(parent, &mut c) };
    (c.r - c.l, c.b - c.t)
}

/// (gpu) the child window of window w for its swapchain: hidden when new, the client size of w
/// (None: no such window, or the child could not be made)
pub fn win_gpu_child(w: i32) -> Option<NativeWindow> {
    let r = rec(w)?;
    let parent = r.hwnd.get();
    let (cw, ch) = client(parent);
    let mut h = r.gchild.get();
    if h == 0 {
        if !class() {
            return None;
        }
        // SAFETY: a registered class, no title; the child belongs to our window and thread.
        h = unsafe {
            CreateWindowExW(0, CLASS_NAME.as_ptr(), core::ptr::null(), WS_CHILD | WS_DISABLED | WS_CLIPSIBLINGS, 0, 0, cw, ch,
                            parent, 0, hinst(), 0)
        };
        if h == 0 {
            return None;
        }
        r.gchild.set(h);
    } else {
        // SAFETY: our child window.
        unsafe { SetWindowPos(h, 0, 0, 0, cw, ch, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE) };
    }
    Some(NativeWindow { hwnd: h, hinstance: hinst() })
}

/// (gpu) show the child of window w over its client area (sized to it), or hide it: what the
/// window's CPU present last drew is what shows then
pub fn win_gpu_child_show(w: i32, show: bool) {
    let Some(r) = rec(w) else { return };
    let h = r.gchild.get();
    if h == 0 {
        return;
    }
    if show {
        let (cw, ch) = client(r.hwnd.get());
        // SAFETY: our child window.
        unsafe { SetWindowPos(h, 0, 0, 0, cw, ch, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW) };
    } else {
        // SAFETY: our child window.
        unsafe { ShowWindow(h, SW_HIDE) };
    }
}

/// (gpu) destroy the child of window w (the swapchain on it must be gone)
pub fn win_gpu_child_destroy(w: i32) {
    let Some(r) = rec(w) else { return };
    let h = r.gchild.replace(0);
    if h != 0 {
        // SAFETY: our child window.
        unsafe { DestroyWindow(h) };
    }
}

/// (gpu) window w's copy for WM_PAINT is not needed while the child shows the window (the next
/// win_present makes a new one)
pub fn win_release_kept(w: i32) {
    if let Some(r) = rec(w) {
        r.kept_free();
    }
}

/// the modal move / size loop of a window runs (the program's own loop waits for its end)
pub fn gpu_wait(deadline_us: u64) {
    super::wait(deadline_us);
}

/// no child now, but it may come (the host or the next frame wakes the loop): only Android
pub fn win_gpu_child_later(_w: i32) -> bool {
    false
}

/// the window still shows on its child alone: here always
pub fn win_gpu_child_alone(_w: i32) -> bool {
    true
}

/// how the output came up (GPU_STATE_*): Android keeps it across runs
pub fn gpu_out_state(_state: u32) {}

/// a thread for the output's jobs
pub fn gpu_thread_spawn(stack: usize, f: alloc::boxed::Box<dyn FnOnce() + Send>) -> Option<crate::Thread> {
    crate::thread_spawn(stack, f)
}

pub fn win_in_sizemove() -> bool {
    st().in_sizemove.get()
}

/// this program runs in a Remote Desktop session (the GPU output stays off)
pub fn win_remote_session() -> bool {
    // SAFETY: a plain query.
    unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
}

/// the 4 hex digits after `key` in a device id ("PCI\\VEN_10DE&DEV_1E84&...")
fn hex_after(id: &[u16], key: &str) -> Option<u32> {
    let k: alloc::vec::Vec<u16> = key.encode_utf16().collect();
    let at = id.windows(k.len()).position(|w| w == &k[..])? + k.len();
    let mut v = 0;
    for &c in id.get(at..at + 4)? {
        v = v * 16 + char::from_u32(c as u32)?.to_digit(16)?;
    }
    Some(v)
}

/// the PCI vendor and device id of the GPU that drives the monitor window w is on (None: not
/// known, not a PCI device)
pub fn win_display_adapter(w: i32) -> Option<(u32, u32)> {
    let r = rec(w)?;
    // SAFETY: our window; the structures carry their sizes and live until the calls return.
    unsafe {
        let m = MonitorFromWindow(r.hwnd.get(), MONITOR_NEAREST);
        let mut mi: MonitorInfoEx = core::mem::zeroed();
        mi.info.size = core::mem::size_of::<MonitorInfoEx>() as u32;
        if m == 0 || GetMonitorInfoW(m, &mut mi as *mut MonitorInfoEx as *mut MonitorInfo) == 0 {
            return None;
        }
        for n in 0..64 {
            let mut dd: DisplayDevice = core::mem::zeroed();
            dd.cb = core::mem::size_of::<DisplayDevice>() as u32;
            if EnumDisplayDevicesW(core::ptr::null(), n, &mut dd, 0) == 0 {
                return None;
            }
            if dd.name == mi.device {
                return Some((hex_after(&dd.id, "VEN_")?, hex_after(&dd.id, "DEV_")?));
            }
        }
    }
    None
}
