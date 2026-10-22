// win_api -- the window-indexed platform API on Windows: windows (records), input, the event
// wait, presentation, cursors                                     [src/plat/win/win_window.s]
// (popups: win_popup.rs; clipboard, IME, drops: win_clip.rs; file dialog: win_filedlg.rs;
//  pens: win_pen.rs; shell, theme, home, drives, spawn_self: win_shell.rs)
//
//   win_create(title, w_dip, h_dip, flags, owner, x, y) -> index / -1
//        owner PW_NOOWNER: a top-level window (WS_OVERLAPPEDWINDOW, preferably index 0); else
//        a window owned by window `owner`: WS_POPUP | WS_CAPTION | WS_SYSMENU (+ WS_THICKFRAME
//        with PWF_RESIZABLE), WS_EX_TOOLWINDOW for PWF_TOOL (no taskbar button, not in
//        Alt+Tab), WS_EX_NOACTIVATE for PWF_NOACTIVATE.  Created hidden; x, y = client origin
//        (PW_DEFAULT: system default / owner client origin + 32 px).  Without PWF_SYSFRAME the
//        caption is removed (the library draws the frame; DWM keeps shadow and corners).
//   win_destroy(w)                    win_show(w, PWS_HIDE / SHOW / SHOWNA)
//   win_activate(w)                   win_enable(w, on)
//   win_set_rect(w, x, y, cw, ch)     client origin (screen px) and size; PW_KEEP
//   win_get_rect(w) -> [x, y, w, h]   client origin + client size (None: no window)
//   win_work_area(w) -> [x, y, w, h]  work area of the window's monitor
//   win_set_title(w, utf8)            win_state(w) -> WST_* (WST_ACTIVE: the app is active)
//   win_cmd(w, WCMD_*)                win_begin_move(w)  win_begin_resize(w, edges)
//   win_size(w) -> (w, h)             win_dpi(w) -> 96 = 100 %
//   win_set_min_size(w, w_dip, h_dip) (also before the window exists)
//   win_present(w, frame, rects)      rects x, y, w, h; None = all.  The window keeps a copy
//                                     of the presented pixels: WM_PAINT repaints from it.
//   win_set_cursor(w, shape)          CUR_* or a custom cursor CUR_USER0..CUR_TABLE-1
//   cursor_create(px, w, h, hot_x, hot_y, dpi) -> CUR_USER0 + k / -1   a cursor from
//                                     premultiplied ARGB pixels (rows of w * 4 bytes,
//                                     1..CUR_MAX_PX; CreateIconIndirect)
//   cursor_destroy(id)                cursor_dpi(dpi) -> the dpi to draw cursors for (the
//                                     window's own on Windows)
//   win_hit_hook(f)                   f(w, x, y) -> HB_* flags of the UI at a client point
//                                     (WM_NCHITTEST, WM_MOUSEACTIVATE)
//   wait(deadline_us) -> queued events   0 = poll, u64::MAX = forever, else plat::time_us()
//   set_refresh(f)                    f() redraws during the modal move / size loop
//   win_test_send(w, msg, wp, lp)     (test aid) SendMessageW to window w (input, hit test,
//                                     activation, size-move messages: no pointers); more test
//                                     aids in win_test.rs (win_test, file_dialog_autoclose)
// (the single-window window_create / present / ... are in pwin0.rs)
//
// Library frame (no PWF_SYSFRAME, AF_CUSTOM_TITLEBAR for the main window): the caption is
// removed (WM_NCCALCSIZE) while the resize borders, shadow and rounded corners stay;
// WM_NCHITTEST asks the UI hit boxes (hit hook): HB_DRAG moves the window, HB_MAX gives snap
// layouts (main window), the caption buttons are drawn by the UI.
// Every event carries its window (Event::win).  The window procedure never acts on input: it
// only records events into the ring (evq.rs); it re-enters (SendMessage, SetWindowPos), so
// the window of the events it pushes is saved and restored.
// The owner keeps looking active while one of its own windows is active: WM_NCACTIVATE (FALSE)
// is drawn active unless the whole app loses the activation (WM_ACTIVATEAPP).
// Tool windows are not activated by clicks (WM_MOUSEACTIVATE -> MA_NOACTIVATE) unless the
// click hits a text box: the keyboard stays with the main window.
//
// State: everything lives on the UI thread (the thread that creates the windows runs their
// window procedure); the records are Cells, so the re-entrant window procedure and the
// callbacks (refresh, hit hook) never hold a reference across a Win32 call.

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;

use crate::{
    CUR_ARROW, CUR_COUNT, CUR_MAX_PX, CUR_TABLE, CUR_USER0, CUR_USER_MAX, E_CLOSE, E_DOWN, E_DPI, E_DROP, E_FOCUS_IN,
    E_FOCUS_OUT, E_KEY_DOWN, E_KEY_UP, E_LEAVE, E_MOVE, E_PAINT, E_POWER, E_RESIZE, E_SESSION_END, E_SETTINGS, E_TEXT,
    E_UP, E_WHEEL, E_WINPOS, SET_CONTRAST, SET_FONTS, SET_LOCALE, SET_MONITORS, SET_THEME,
    Event, Frame, HB_CLOSE, HB_DRAG, HB_MAX, HB_MIN, HB_TEXT, MOD_ALT, MOD_ALTGR, MOD_CAPS, MOD_CTRL, MOD_NUM, MOD_SHIFT,
    MOD_SUPER, PW_DEFAULT, PW_KEEP, PW_MAX, PW_NOOWNER, PWF_MASK, PWF_NOACTIVATE, PWF_RESIZABLE, PWF_SYSFRAME,
    PWF_TOOL, PWS_SHOWNA, WCMD_CLOSE, WCMD_MAXIMIZE, WCMD_MINIMIZE, WST_ACTIVE, WST_MAXIMIZED, WST_MINIMIZED, evq_count,
    evq_push,
};

#[path = "win_clip.rs"]
mod clip;
pub use clip::*;
#[path = "win_filedlg.rs"]
mod filedlg;
pub use filedlg::*;
#[path = "win_pen.rs"]
mod pen;
pub use pen::*;
#[path = "win_popup.rs"]
mod popup;
pub use popup::*;
#[path = "win_shell.rs"]
mod shell;
pub use shell::*;
#[path = "win_test.rs"]
mod wtest;
pub use wtest::*;
// r12-inst: the setup's shell calls (plat::inst::*: exe path, known folders, .lnk, HKCU key, CreateProcessW)
#[path = "win_inst.rs"]
pub mod inst;
// tray.rs: the symbol in the notification area and the notifications (Shell_NotifyIconW)
#[path = "win_tray.rs"]
pub(crate) mod win_tray;
// screen.rs: the monitors, pictures of the screen, the colour pick (BitBlt, an overlay)
#[path = "win_screen.rs"]
pub(crate) mod win_screen;
pub use win_screen::{pick_test_cancel, pick_test_click, pick_test_move, pick_test_window, screen_test_metrics, screen_test_topmost};
// hotkey.rs: global shortcuts (RegisterHotKey on a message-only window)
#[path = "win_hotkey.rs"]
pub(crate) mod win_hotkey;
pub use win_hotkey::{hotkey_test_input, hotkey_test_post};
// a11y.rs: the accessibility tree as UI Automation providers (WM_GETOBJECT)
#[path = "win_uia.rs"]
pub(crate) mod win_uia;
// dnd.rs: drag and drop through OLE (IDropTarget per window, DoDragDrop)
#[path = "win_dnd.rs"]
pub(crate) mod win_dnd;
pub(crate) use win_dnd::os_drag_start;
pub use win_dnd::{dnd_test_drop, dnd_test_formats, dnd_test_source};
pub use win_uia::{
    UiaTestAct, UiaTestCaret, UiaTestElem, uia_test_act, uia_test_caret, uia_test_caret_move, uia_test_events, uia_test_hwnd, uia_test_listen,
    uia_test_system_caret, uia_test_tree, uia_test_unlisten,
};

// ---------------------------------------------------------------- Win32

#[repr(C)]
struct WndClassEx {
    size: u32,
    style: u32,
    proc_: unsafe extern "system" fn(isize, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    inst: isize,
    icon: isize,
    cursor: isize,
    brush: isize,
    menu: *const u16,
    class: *const u16,
    icon_sm: isize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    l: i32,
    t: i32,
    r: i32,
    b: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct Msg {
    hwnd: isize,
    msg: u32,
    wp: usize,
    lp: isize,
    time: u32,
    pt: [i32; 2],
    priv_: u32,
}

#[repr(C)]
struct PaintStruct {
    hdc: isize,
    erase: i32,
    rc: Rect,
    restore: i32,
    incupdate: i32,
    reserved: [u8; 32],
}

#[repr(C)]
#[derive(Default)]
struct BitmapInfoHeader {
    size: u32,
    w: i32,
    h: i32,
    planes: u16,
    bits: u16,
    compression: u32,
    size_image: u32,
    xppm: i32,
    yppm: i32,
    clr_used: u32,
    clr_imp: u32,
}

#[repr(C)]
struct TrackMouse {
    size: u32,
    flags: u32,
    hwnd: isize,
    hover: u32,
}

#[repr(C)]
#[derive(Default)]
struct MonitorInfo {
    size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
}

#[repr(C)]
struct IconInfo {
    icon: i32,
    hot_x: u32,
    hot_y: u32,
    mask: isize,
    color: isize,
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(c: *const WndClassEx) -> u16;
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32,
                       parent: isize, menu: isize, inst: isize, param: usize) -> isize;
    fn DefWindowProcW(h: isize, m: u32, wp: usize, lp: isize) -> isize;
    fn DestroyWindow(h: isize) -> i32;
    fn ShowWindow(h: isize, cmd: i32) -> i32;
    fn SetWindowPos(h: isize, after: isize, x: i32, y: i32, w: i32, cy: i32, flags: u32) -> i32;
    fn GetWindowRect(h: isize, r: *mut Rect) -> i32;
    fn GetClientRect(h: isize, r: *mut Rect) -> i32;
    fn ClientToScreen(h: isize, p: *mut Point) -> i32;
    fn ScreenToClient(h: isize, p: *mut Point) -> i32;
    fn IsIconic(h: isize) -> i32;
    fn IsZoomed(h: isize) -> i32;
    fn IsWindowVisible(h: isize) -> i32;
    fn SetActiveWindow(h: isize) -> isize;
    fn EnableWindow(h: isize, on: i32) -> i32;
    fn PeekMessageW(m: *mut Msg, h: isize, a: u32, b: u32, remove: u32) -> i32;
    fn TranslateMessage(m: *const Msg) -> i32;
    fn DispatchMessageW(m: *const Msg) -> isize;
    fn MsgWaitForMultipleObjectsEx(n: u32, h: usize, ms: u32, mask: u32, flags: u32) -> u32;
    fn PostQuitMessage(code: i32);
    fn PostMessageW(h: isize, m: u32, wp: usize, lp: isize) -> i32;
    fn SendMessageW(h: isize, m: u32, wp: usize, lp: isize) -> isize;
    fn GetWindowLongPtrW(h: isize, i: i32) -> isize;
    fn SetWindowLongPtrW(h: isize, i: i32, v: isize) -> isize;
    fn LoadCursorW(inst: isize, name: usize) -> isize;
    fn LoadIconW(inst: isize, name: usize) -> isize;
    fn SetCursor(c: isize) -> isize;
    fn GetDC(h: isize) -> isize;
    fn ReleaseDC(h: isize, dc: isize) -> i32;
    fn BeginPaint(h: isize, ps: *mut PaintStruct) -> isize;
    fn EndPaint(h: isize, ps: *const PaintStruct) -> i32;
    fn SetCapture(h: isize) -> isize;
    fn ReleaseCapture() -> i32;
    fn TrackMouseEvent(t: *mut TrackMouse) -> i32;
    fn GetKeyState(vk: i32) -> i16;
    fn WindowFromPoint(p: Point) -> isize;
    fn GetSystemMetrics(i: i32) -> i32;
    fn MonitorFromWindow(h: isize, flags: u32) -> isize;
    fn GetMonitorInfoW(m: isize, mi: *mut MonitorInfo) -> i32;
    fn SetWindowTextW(h: isize, s: *const u16) -> i32;
    fn CreateIconIndirect(ii: *const IconInfo) -> isize;
    fn DestroyIcon(h: isize) -> i32;
    fn SetProcessDPIAware() -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetDIBitsToDevice(dc: isize, xd: i32, yd: i32, w: u32, h: u32, xs: i32, ys: i32, start: u32, lines: u32,
                         bits: *const u8, bmi: *const BitmapInfoHeader, usage: u32) -> i32;
    fn GetDeviceCaps(dc: isize, i: i32) -> i32;
    fn CreateDIBSection(dc: isize, bmi: *const BitmapInfoHeader, usage: u32, bits: *mut *mut u8, sect: isize,
                        off: u32) -> isize;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const u8) -> isize;
    fn DeleteObject(h: isize) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> isize;
    fn LoadLibraryA(name: *const u8) -> isize;
    fn GetProcAddress(m: isize, name: *const u8) -> usize;
    fn WaitForSingleObject(h: isize, ms: u32) -> u32;
}

const TITLE_MAX: usize = 512; // wide chars of a title (with the NUL)
const WWF_USED: u32 = 0x100;
const WWF_CUSTOM: u32 = 0x200; // library frame (no caption)
const WWF_TOP: u32 = 0x400; // top-level (no owner)
const SELF_NONE: u32 = 0;
const SELF_PLACE: u32 = 1; // our SetWindowPos (its result is not reported)
const SELF_CREATE: u32 = 2; // being created

const WM_DESTROY: u32 = 0x0002;
const WM_MOVE: u32 = 0x0003;
const WM_SIZE: u32 = 0x0005;
const WM_SETFOCUS: u32 = 0x0007;
const WM_KILLFOCUS: u32 = 0x0008;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_QUIT: u32 = 0x0012;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_GETOBJECT: u32 = 0x003D;
const WM_ACTIVATEAPP: u32 = 0x001C;
const WM_SETCURSOR: u32 = 0x0020;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const WM_GETMINMAXINFO: u32 = 0x0024;
const WM_NCCREATE: u32 = 0x0081;
const WM_NCCALCSIZE: u32 = 0x0083;
const WM_NCHITTEST: u32 = 0x0084;
const WM_NCACTIVATE: u32 = 0x0086;
const WM_NCMOUSEMOVE: u32 = 0x00A0;
const WM_NCLBUTTONDOWN: u32 = 0x00A1;
const WM_NCLBUTTONUP: u32 = 0x00A2;
const WM_NCLBUTTONDBLCLK: u32 = 0x00A3;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_CHAR: u32 = 0x0102;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_SYSCHAR: u32 = 0x0106;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_MBUTTONUP: u32 = 0x0208;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_XBUTTONDOWN: u32 = 0x020B;
const WM_XBUTTONUP: u32 = 0x020C;
const WM_MOUSEHWHEEL: u32 = 0x020E;
const WM_CAPTURECHANGED: u32 = 0x0215;
const WM_POINTERUPDATE: u32 = 0x0245;
const WM_POINTERCAPTURECHANGED: u32 = 0x024C;
const WM_ENTERSIZEMOVE: u32 = 0x0231;
const WM_EXITSIZEMOVE: u32 = 0x0232;
const WM_DROPFILES: u32 = 0x0233;
const WM_NCMOUSELEAVE: u32 = 0x02A2;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_DPICHANGED: u32 = 0x02E0;
const WM_QUERYENDSESSION: u32 = 0x0011;
const WM_SYSCOLORCHANGE: u32 = 0x0015;
const WM_ENDSESSION: u32 = 0x0016;
const WM_SETTINGCHANGE: u32 = 0x001A;
const WM_DISPLAYCHANGE: u32 = 0x007E;
const SPI_SETWORKAREA: usize = 0x002F;
const WM_POWERBROADCAST: u32 = 0x0218;
const PBT_APMSUSPEND: usize = 4;
const PBT_APMRESUMESUSPEND: usize = 7;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
const SPI_SETHIGHCONTRAST: usize = 0x0043;

// lifecycle: the system sleeps (E_POWER sent), the session end was announced (E_SESSION_END)
static SLEEPING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static ENDING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

// WM_SETTINGCHANGE -> SET_* bits (0: nothing the UI follows)
fn setting_bits(wp: usize, lp: isize) -> u32 {
    if wp == SPI_SETHIGHCONTRAST {
        return SET_CONTRAST;
    }
    if wp == SPI_SETWORKAREA {
        return SET_MONITORS; // a taskbar moved or changed its size
    }
    // SPI_SETFONTSMOOTHING, _TYPE, _CONTRAST, _ORIENTATION
    if matches!(wp, 0x004B | 0x200B | 0x200D | 0x2013) {
        return SET_FONTS;
    }
    if lp == 0 {
        return 0;
    }
    // SAFETY: lParam of WM_SETTINGCHANGE is a NUL-terminated UTF-16 string (or 0).
    let name = unsafe {
        let p = lp as *const u16;
        let mut n = 0;
        while n < 64 && *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(core::slice::from_raw_parts(p, n))
    };
    match name.as_str() {
        "ImmersiveColorSet" => SET_THEME,
        "intl" => SET_LOCALE,
        "WindowMetrics" => SET_FONTS,
        _ => 0,
    }
}
const SIZE_MINIMIZED: usize = 1;
const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const WS_EX_ACCEPTFILES: u32 = 0x0000_0010;
const GWLP_USERDATA: i32 = -21;
const MA_ACTIVATE: isize = 1;
const MA_NOACTIVATE: isize = 3;
const MONITOR_NEAREST: u32 = 2;
const SW_HIDE: i32 = 0;
const SW_SHOW: i32 = 5;
const SW_SHOWNA: i32 = 8;
const SW_MINIMIZE: i32 = 6;
const SW_MAXIMIZE: i32 = 3;
const SW_RESTORE: i32 = 9;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_FRAMECHANGED: u32 = 0x0020;
const CW_USEDEFAULT: i32 = 0x8000_0000u32 as i32;
const PM_REMOVE: u32 = 1;
const QS_ALLINPUT: u32 = 0x04FF;
const MWMO_INPUTAVAILABLE: u32 = 4;
const WAIT_TIMEOUT: u32 = 0x102;
const INFINITE: u32 = 0xFFFF_FFFF;
const TME_LEAVE: u32 = 2;
const TME_NONCLIENT: u32 = 0x10;
const HTCLIENT: isize = 1;
const HTCAPTION: isize = 2;
const HTMINBUTTON: isize = 8;
const HTMAXBUTTON: isize = 9;
const HTTOP: isize = 12;
const HTTOPLEFT: isize = 13;
const HTTOPRIGHT: isize = 14;
const HTCLOSE: isize = 20;
const SM_CYFRAME: i32 = 33;
const SM_CXPADDEDBORDER: i32 = 92;
const LOGPIXELSX: i32 = 88;
const CS_HREDRAW_VREDRAW: u32 = 3;
const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_CAPITAL: i32 = 0x14;
const VK_LWIN: i32 = 0x5B;
const VK_RWIN: i32 = 0x5C;
const VK_F4: usize = 0x73;
const VK_NUMLOCK: i32 = 0x90;
const VK_RMENU: i32 = 0xA5;

// the cursor shapes CUR_ARROW .. CUR_NONE (0: hidden)
static IDC_TABLE: [u16; CUR_COUNT as usize] = [32512, 32513, 32649, 32515, 32646, 32514, 32645, 32644, 32642, 32643, 0];
// edge bits (top 1, bottom 2, left 4, right 8) -> HT code
static HT_EDGE: [u8; 16] = [1, 12, 15, 1, 10, 13, 16, 10, 11, 14, 17, 11, 1, 1, 1, 1];
static W_CLASS: [u16; 6] = [0x4A, 0x46, 0x47, 0x55, 0x49, 0]; // "JFGUI"

// ---------------------------------------------------------------- state

/// UI-thread state: the windows, their window procedure and the callbacks
struct UiThread<T>(T);

// SAFETY: every field is a Cell that only the UI thread touches -- the thread that creates the
// windows (their window procedure runs there) and calls the plat window API, as in the
// assembly (no locks there either).  wake() from other threads uses win_thread.rs's atomics.
unsafe impl<T> Sync for UiThread<T> {}

/// one window record (WW_* of the assembly)
struct Ww {
    hwnd: Cell<isize>, // 0 = no window
    w: Cell<i32>,      // client size (px)
    h: Cell<i32>,
    dpi: Cell<i32>,
    minw: Cell<i32>, // minimum client size (DIP)
    minh: Cell<i32>,
    flags: Cell<u32>, // PWF_* | WWF_*
    owner: Cell<i32>, // owner index (-1 = top-level)
    min: Cell<bool>,  // minimized
    track: Cell<bool>, // TrackMouseEvent armed
    nctrack: Cell<bool>,
    cursor: Cell<u32>, // CUR_*
    mx: Cell<i32>,     // last pointer position (client px)
    my: Cell<i32>,
    px: Cell<i32>, // last client origin reported (screen px)
    py: Cell<i32>,
    selfp: Cell<u32>,     // SELF_*
    req: Cell<[i32; 4]>, // what our SetWindowPos asked for (x, y, w, h)
    movepend: Cell<bool>, // moved inside the modal move loop: reported at its end
    psf: Cell<bool>,      // kept holds the pixels of the last present
    kept: Cell<Vec<u8>>,  // copy of those pixels (WM_PAINT repaints from it)
    kdim: Cell<(i32, i32, i32)>, // its width, height, stride
}

impl Ww {
    const fn new() -> Ww {
        Ww {
            hwnd: Cell::new(0),
            w: Cell::new(0),
            h: Cell::new(0),
            dpi: Cell::new(0),
            minw: Cell::new(0),
            minh: Cell::new(0),
            flags: Cell::new(0),
            owner: Cell::new(0),
            min: Cell::new(false),
            track: Cell::new(false),
            nctrack: Cell::new(false),
            cursor: Cell::new(0),
            mx: Cell::new(0),
            my: Cell::new(0),
            px: Cell::new(0),
            py: Cell::new(0),
            selfp: Cell::new(SELF_NONE),
            req: Cell::new([0; 4]),
            movepend: Cell::new(false),
            psf: Cell::new(false),
            kept: Cell::new(Vec::new()),
            kdim: Cell::new((0, 0, 0)),
        }
    }

    /// every field 0 (mem_zero of the record)
    fn clear(&self) {
        self.hwnd.set(0);
        self.w.set(0);
        self.h.set(0);
        self.dpi.set(0);
        self.minw.set(0);
        self.minh.set(0);
        self.flags.set(0);
        self.owner.set(0);
        self.min.set(false);
        self.track.set(false);
        self.nctrack.set(false);
        self.cursor.set(0);
        self.mx.set(0);
        self.my.set(0);
        self.px.set(0);
        self.py.set(0);
        self.selfp.set(SELF_NONE);
        self.req.set([0; 4]);
        self.movepend.set(false);
        self.psf.set(false);
        drop(self.kept.take());
        self.kdim.set((0, 0, 0));
    }
}

struct St {
    ww: [Ww; PW_MAX],
    cursors: [Cell<isize>; CUR_TABLE as usize], // built-in shapes, then the custom cursors
    refresh: Cell<Option<fn()>>,
    refresh_dyn: Cell<Option<*mut (dyn FnMut() + 'static)>>, // wait_refresh's closure while it runs
    hit: Cell<Option<fn(i32, i32, i32) -> u32>>,
    ev_win: Cell<i32>,    // window of the events being pushed
    creating: Cell<i32>,  // index + 1 of the window inside CreateWindowExW
    app_active: Cell<bool>,
    nc_force: Cell<bool>, // WM_NCACTIVATE(FALSE) is meant (the app deactivates)
    btn_mask: Cell<u32>,
    btn_win: Cell<i32>, // window of the button press
    hi_surr: Cell<u32>,
    dyn_done: Cell<bool>,
    class_done: Cell<bool>,
    in_sizemove: Cell<bool>,
    in_refresh: Cell<bool>,
    ptr_win: Cell<i32>, // window under the pointer (-1 = none)
    get_dpi: Cell<usize>, // GetDpiForWindow
    hinst: Cell<isize>,
}

static S: UiThread<St> = UiThread(St {
    ww: [const { Ww::new() }; PW_MAX],
    cursors: [const { Cell::new(0) }; CUR_TABLE as usize],
    refresh: Cell::new(None),
    refresh_dyn: Cell::new(None),
    hit: Cell::new(None),
    ev_win: Cell::new(0),
    creating: Cell::new(0),
    app_active: Cell::new(false),
    nc_force: Cell::new(false),
    btn_mask: Cell::new(0),
    btn_win: Cell::new(0),
    hi_surr: Cell::new(0),
    dyn_done: Cell::new(false),
    class_done: Cell::new(false),
    in_sizemove: Cell::new(false),
    in_refresh: Cell::new(false),
    ptr_win: Cell::new(-1),
    get_dpi: Cell::new(0),
    hinst: Cell::new(0),
});

fn st() -> &'static St {
    &S.0
}

/// the record of window w when it exists  [ww_rec]
fn rec(w: i32) -> Option<&'static Ww> {
    let r = st().ww.get(usize::try_from(w).ok()?)?;
    if r.hwnd.get() == 0 { None } else { Some(r) }
}

/// HWND of window w (0 = none)  [plat_win_hwnd]
fn win_hwnd(w: i32) -> isize {
    rec(w).map_or(0, |r| r.hwnd.get())
}

/// the main window's HWND (g_hwnd)
fn main_hwnd() -> isize {
    st().ww[0].hwnd.get()
}

/// (crate) the main window's HWND (g_hwnd, 0 = none): the owner of print dialogs, the monitor
/// of the colour profile (win_print.s / win_icc.s, plat-shared)
#[allow(dead_code)]
pub(crate) fn hwnd_main() -> isize {
    main_hwnd()
}

/// (crate) HWND of platform window w (plat_win_hwnd, 0 = none)
#[allow(dead_code)]
pub(crate) fn hwnd_of(w: i32) -> isize {
    win_hwnd(w)
}

/// module instance (g_hinst)
fn hinst() -> isize {
    let s = st();
    if s.hinst.get() == 0 {
        // SAFETY: our own module.
        s.hinst.set(unsafe { GetModuleHandleW(core::ptr::null()) });
    }
    s.hinst.get()
}

/// UTF-8 -> NUL-terminated UTF-16 of at most cap wide chars (cap - 1 of text; a pair that
/// does not fit ends the text)  [utf8_to_w]
fn wide_cap(s: &str, cap: usize) -> Vec<u16> {
    let mut w: Vec<u16> = Vec::with_capacity(s.len().min(cap) + 1);
    let room = cap.saturating_sub(1);
    let mut buf = [0u16; 2];
    for c in s.chars() {
        if c == '\0' {
            break;
        }
        let u = c.encode_utf16(&mut buf);
        if w.len() + u.len() > room {
            break;
        }
        w.extend_from_slice(u);
    }
    w.push(0);
    w
}

/// a function of a system DLL (0 = missing)
fn dll_proc(dll: &[u8], name: &[u8]) -> usize {
    // SAFETY: NUL-terminated names; both calls return 0 on failure.
    unsafe {
        let m = LoadLibraryA(dll.as_ptr());
        if m == 0 { 0 } else { GetProcAddress(m, name.as_ptr()) }
    }
}

// ---------------------------------------------------------------- creation

/// optional functions, DPI awareness, cursors and the window class (once) -> false on failure
fn win_setup() -> bool {
    let s = st();
    if s.class_done.get() {
        return true;
    }
    win_dyn_init();
    for (k, &idc) in IDC_TABLE.iter().enumerate() {
        // SAFETY: system cursor ids.
        let h = if idc == 0 { 0 } else { unsafe { LoadCursorW(0, idc as usize) } };
        s.cursors[k].set(h);
    }
    let inst = hinst();
    // SAFETY: a valid WNDCLASSEXW; the class name is static.
    let ok = unsafe {
        let icon = LoadIconW(inst, 1); // icon resource 1 (if any)
        let wc = WndClassEx {
            size: core::mem::size_of::<WndClassEx>() as u32,
            style: CS_HREDRAW_VREDRAW,
            proc_: wndproc,
            cls_extra: 0,
            wnd_extra: 0,
            inst,
            icon,
            cursor: 0,
            brush: 0,
            menu: core::ptr::null(),
            class: W_CLASS.as_ptr(),
            icon_sm: icon,
        };
        RegisterClassExW(&wc) != 0
    };
    if ok {
        s.class_done.set(true);
    }
    ok
}

/// GetDpiForWindow (once); per-monitor DPI awareness v2 if available, else system aware
/// (the assembly does the latter at process start)
fn win_dyn_init() {
    let s = st();
    if s.dyn_done.get() {
        return;
    }
    s.dyn_done.set(true);
    s.get_dpi.set(dll_proc(b"user32.dll\0", b"GetDpiForWindow\0"));
    let p = dll_proc(b"user32.dll\0", b"SetProcessDpiAwarenessContext\0");
    let mut ok = false;
    if p != 0 {
        // SAFETY: SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT) -> BOOL;
        // -4 = DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2.
        let f: unsafe extern "system" fn(isize) -> i32 = unsafe { core::mem::transmute(p) };
        ok = unsafe { f(-4) } != 0;
    }
    if !ok {
        // SAFETY: no arguments.
        unsafe { SetProcessDPIAware() };
    }
}

/// the window's dpi (GetDpiForWindow, else the DC's LOGPIXELSX, else 96)
fn win_query_dpi(hwnd: isize) -> i32 {
    let p = st().get_dpi.get();
    if p != 0 {
        // SAFETY: GetDpiForWindow(HWND) -> UINT.
        let f: unsafe extern "system" fn(isize) -> u32 = unsafe { core::mem::transmute(p) };
        let d = unsafe { f(hwnd) };
        if d != 0 {
            return d as i32;
        }
    }
    // SAFETY: a DC of our window, released again.
    let d = unsafe {
        let dc = GetDC(hwnd);
        let d = GetDeviceCaps(dc, LOGPIXELSX);
        ReleaseDC(hwnd, dc);
        d
    };
    if d == 0 { 96 } else { d }
}

/// v * dpi / 96 rounded (unsigned, as the assembly's div)
fn dip(v: i32, dpi: i32) -> i32 {
    ((v as u32).wrapping_mul(dpi as u32).wrapping_add(48) / 96) as i32
}

/// a hidden window (see the file header) -> index / -1  [plat_win_create]
pub fn win_create(title: &str, w_dip: i32, h_dip: i32, flags: u32, owner: i32, x: i32, y: i32) -> i32 {
    if !win_setup() {
        return -1;
    }
    let s = st();
    // the owner must be a window of ours
    let mut parent = 0isize;
    let top = owner == PW_NOOWNER;
    let mut idx = 0usize;
    if top {
        if s.ww[0].flags.get() & WWF_USED != 0 {
            idx = 1;
        }
    } else {
        match rec(owner) {
            Some(r) => parent = r.hwnd.get(),
            None => return -1,
        }
        idx = 1;
    }
    if idx != 0 {
        match (1..PW_MAX).find(|&i| s.ww[i].flags.get() & WWF_USED == 0) {
            Some(i) => idx = i,
            None => return -1,
        }
    }
    let r = &s.ww[idx];
    let (minw, minh) = (r.minw.get(), r.minh.get()); // a minimum size set before creation stays
    r.clear();
    r.minw.set(minw);
    r.minh.set(minh);
    let mut f = flags & PWF_MASK | WWF_USED;
    if flags & PWF_SYSFRAME == 0 {
        f |= WWF_CUSTOM;
    }
    if top {
        f |= WWF_TOP;
    }
    r.flags.set(f);
    r.owner.set(owner);
    r.dpi.set(96);
    r.selfp.set(SELF_CREATE); // no events while it is being created
    r.px.set(i32::MIN);
    // styles
    let (ex, style) = if top {
        // (the assembly gives only owned windows WS_EX_ACCEPTFILES -- the main window then
        // never gets real drops; plat-win: the main window accepts them too)
        (WS_EX_ACCEPTFILES, WS_OVERLAPPEDWINDOW)
    } else {
        let mut st_ = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        if flags & PWF_RESIZABLE != 0 {
            st_ |= WS_THICKFRAME;
        }
        let mut ex = 0;
        if flags & PWF_TOOL != 0 {
            ex |= WS_EX_TOOLWINDOW;
        }
        if flags & PWF_NOACTIVATE != 0 {
            ex |= WS_EX_NOACTIVATE;
        }
        (ex | WS_EX_ACCEPTFILES, st_)
    };
    let wt = wide_cap(title, TITLE_MAX);
    let xy0 = if top { CW_USEDEFAULT } else { 0 };
    s.creating.set(idx as i32 + 1);
    // SAFETY: a registered class, NUL-terminated strings that outlive the call; lpParam =
    // index + 1 (WM_NCCREATE stores it in GWLP_USERDATA).
    let hwnd = unsafe {
        CreateWindowExW(ex, W_CLASS.as_ptr(), wt.as_ptr(), style, xy0, xy0, 640, 480, parent, 0, hinst(), idx + 1)
    };
    s.creating.set(0);
    if hwnd == 0 {
        r.flags.set(0);
        r.hwnd.set(0);
        return -1;
    }
    r.hwnd.set(hwnd);
    r.dpi.set(win_query_dpi(hwnd));
    if idx == 0 {
        super::thread::wake_window(hwnd); // plat::wake posts to the main window (g_hwnd)
        clip::win_drop_selftest(hwnd); // jfgui_TEST_DROPFILES
    }
    win_dnd::register(hwnd, idx as i32); // an OLE drop target (else WM_DROPFILES as before)
    // SAFETY: our window; the frame without caption takes effect now.
    unsafe {
        SetWindowPos(hwnd, 0, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED)
    };
    // client size in pixels
    let dpi = r.dpi.get();
    let (cw, ch) = (dip(w_dip, dpi), dip(h_dip, dpi));
    // position
    let (mut px, mut py) = (x, y);
    if x == PW_DEFAULT && !top {
        // owned: owner client origin + 32 px
        let mut p = Point::default();
        // SAFETY: the owner's window and a valid POINT.
        unsafe { ClientToScreen(parent, &mut p) };
        let d = dip(32, dpi);
        px = p.x + d;
        py = p.y + d;
    }
    win_place(idx as i32, px, py, cw, ch);
    r.selfp.set(SELF_NONE);
    // the state the events will be compared against
    let mut p = Point::default();
    // SAFETY: our window and a valid POINT.
    unsafe { ClientToScreen(hwnd, &mut p) };
    r.px.set(p.x);
    r.py.set(p.y);
    client_size(r);
    idx as i32
}

/// WW_W / WW_H from GetClientRect
fn client_size(r: &Ww) {
    let mut c = Rect::default();
    // SAFETY: our window and a valid RECT.
    unsafe { GetClientRect(r.hwnd.get(), &mut c) };
    r.w.set(c.r);
    r.h.set(c.b);
}

/// frame thickness left, top, right, bottom (window rect minus client rect) as it is now
fn win_insets(hwnd: isize) -> [i32; 4] {
    let (mut wr, mut cr, mut p) = (Rect::default(), Rect::default(), Point::default());
    // SAFETY: valid out-parameters.
    unsafe {
        GetWindowRect(hwnd, &mut wr);
        GetClientRect(hwnd, &mut cr);
        ClientToScreen(hwnd, &mut p);
    }
    [p.x - wr.l, p.y - wr.t, wr.r - p.x - cr.r, wr.b - p.y - cr.b]
}

/// client origin x, y (PW_KEEP: unchanged) and client size cw, ch (PW_KEEP: unchanged); its
/// own WM_MOVE / WM_SIZE are not reported when the system gives what was asked  [win_place]
fn win_place(w: i32, x: i32, y: i32, cw: i32, ch: i32) {
    let Some(r) = rec(w) else { return };
    let hwnd = r.hwnd.get();
    // SAFETY: our window.
    if unsafe { IsIconic(hwnd) } != 0 || unsafe { IsZoomed(hwnd) } != 0 {
        return;
    }
    let ins = win_insets(hwnd);
    let (mut c, mut p) = (Rect::default(), Point::default());
    // SAFETY: valid out-parameters.
    unsafe {
        GetClientRect(hwnd, &mut c);
        ClientToScreen(hwnd, &mut p);
    }
    let mut flags = SWP_NOZORDER | SWP_NOACTIVATE;
    let (mut x, mut y) = (x, y);
    if x == PW_KEEP || y == PW_KEEP {
        x = p.x;
        y = p.y;
        flags |= SWP_NOMOVE;
    }
    let cw = if cw == PW_KEEP { c.r } else { cw };
    let ch = if ch == PW_KEEP { c.b } else { ch };
    if r.selfp.get() != SELF_CREATE {
        r.selfp.set(SELF_PLACE);
    }
    r.req.set([x, y, cw, ch]);
    // SAFETY: our window.
    unsafe {
        SetWindowPos(hwnd, 0, x - ins[0], y - ins[1], cw + ins[0] + ins[2], ch + ins[1] + ins[3], flags);
    }
    if r.selfp.get() == SELF_PLACE {
        r.selfp.set(SELF_NONE);
    }
}

// ---------------------------------------------------------------- window procedure

/// add the modifiers and the event window, queue it  [win_push]
fn win_push(typ: u8, btn: u8, x: i32, y: i32, data: u32) {
    let win = (st().ev_win.get() as u32 & (PW_MAX as u32 - 1)) as u8;
    evq_push(Event { typ, btn, mods: win_mods(), win, x, y, data });
}

/// MOD_* bits from the current keyboard state  [win_mods]
fn win_mods() -> u8 {
    // SAFETY: GetKeyState has no memory arguments.
    let ks = |vk| unsafe { GetKeyState(vk) };
    let mut m = 0;
    if ks(VK_SHIFT) < 0 {
        m |= MOD_SHIFT;
    }
    if ks(VK_CONTROL) < 0 {
        m |= MOD_CTRL;
    }
    if ks(VK_MENU) < 0 {
        m |= MOD_ALT;
    }
    if (ks(VK_LWIN) | ks(VK_RWIN)) < 0 {
        m |= MOD_SUPER;
    }
    if ks(VK_RMENU) < 0 {
        m |= MOD_ALTGR;
    }
    if ks(VK_CAPITAL) & 1 != 0 {
        m |= MOD_CAPS;
    }
    if ks(VK_NUMLOCK) & 1 != 0 {
        m |= MOD_NUM;
    }
    m
}

/// x, y of an lParam point
fn lp_xy(lp: isize) -> (i32, i32) {
    (lp as i16 as i32, (lp as i32) >> 16)
}

/// synthesize E_UP for every held button (in the window of the press)  [release_all_buttons]
fn release_all_buttons() {
    let s = st();
    let save = s.ev_win.get();
    s.ev_win.set(s.btn_win.get());
    for b in 1..=5u8 {
        let m = s.btn_mask.get();
        if m & 1 << b != 0 {
            s.btn_mask.set(m & !(1 << b));
            win_push(E_UP, b, -1, -1, 0);
        }
    }
    s.ev_win.set(save);
}

/// HB_* flags of the UI at a client point of window w (0 without a hit hook)  [ui_hit_at]
fn ui_hit_at(w: i32, x: i32, y: i32) -> u32 {
    match st().hit.get() {
        Some(f) => f(w, x, y),
        None => 0,
    }
}

/// a screen point (lParam) -> client x, y of hwnd  [nc_client_pos]
fn nc_client_pos(hwnd: isize, lp: isize) -> (i32, i32) {
    let (x, y) = lp_xy(lp);
    let mut p = Point { x, y };
    // SAFETY: our window and a valid POINT.
    unsafe { ScreenToClient(hwnd, &mut p) };
    (p.x, p.y)
}

fn track_leave(hwnd: isize, flags: u32) {
    let mut t = TrackMouse { size: 24, flags, hwnd, hover: 0 };
    // SAFETY: a valid TRACKMOUSEEVENT.
    unsafe { TrackMouseEvent(&mut t) };
}

/// redraw from inside the modal size / move loop  [live_refresh]
fn live_refresh() {
    let s = st();
    if !s.in_sizemove.get() || s.in_refresh.get() {
        return;
    }
    if let Some(p) = s.refresh_dyn.get() {
        s.in_refresh.set(true);
        // SAFETY: p is the closure wait_refresh borrowed for the duration of its call; this runs
        // inside that call (the modal loop of a DispatchMessageW of it), never re-entered
        // (in_refresh).
        unsafe { (*p)() };
        s.in_refresh.set(false);
    } else if let Some(f) = s.refresh.get() {
        s.in_refresh.set(true);
        f();
        s.in_refresh.set(false);
    }
}

/// (win_dnd) redraw inside the modal loop of the app's own DoDragDrop: the wait_refresh
/// closure (else the set_refresh hook) while events are queued -- the UI shows the
/// highlight of its drop targets and moves the dragged label
fn dnd_refresh() {
    let s = st();
    if !win_dnd::dragging() || s.in_refresh.get() || evq_count() == 0 {
        return;
    }
    if let Some(p) = s.refresh_dyn.get() {
        s.in_refresh.set(true);
        // SAFETY: p is the closure wait_refresh borrowed for the duration of its call; this runs
        // inside that call (DoDragDrop of its wait), never re-entered (in_refresh).
        unsafe { (*p)() };
        s.in_refresh.set(false);
    } else if let Some(f) = s.refresh.get() {
        s.in_refresh.set(true);
        f();
        s.in_refresh.set(false);
    }
}

/// the window of record `w` moves its client origin from px, py to nx, ny: the visible windows
/// it owns whose centre lies inside its old client area are docked to it and move by the same
/// amount -- right away, so they follow live while the user drags (the modal move loop reports
/// the owner's E_WINPOS only at its end).  Windows dragged out of it stay.  [ww_follow]
fn ww_follow(w: i32, nx: i32, ny: i32) {
    let s = st();
    let o = &s.ww[w as usize];
    let (dx, dy) = (nx.wrapping_sub(o.px.get()), ny.wrapping_sub(o.py.get()));
    if dx | dy == 0 {
        return;
    }
    for i in 0..PW_MAX as i32 {
        if i == w {
            continue;
        }
        let Some(c) = rec(i) else { continue };
        if c.owner.get() != w {
            continue;
        }
        let ch = c.hwnd.get();
        let mut r = Rect::default();
        // SAFETY: our windows, a valid RECT.
        if unsafe { IsWindowVisible(ch) } == 0 || unsafe { GetWindowRect(ch, &mut r) } == 0 {
            continue;
        }
        // centre inside the owner's old client area?
        let cx = ((r.l + r.r) >> 1) - o.px.get();
        let cy = ((r.t + r.b) >> 1) - o.py.get();
        if cx < 0 || cx >= o.w.get() || cy < 0 || cy >= o.h.get() {
            continue;
        }
        // SAFETY: our window.
        unsafe { SetWindowPos(ch, 0, r.l + dx, r.t + dy, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
    }
}

unsafe extern "system" fn wndproc(hwnd: isize, msg: u32, wp: usize, lp: isize) -> isize {
    let s = st();
    // which window: GWLP_USERDATA = index + 1 (set in WM_NCCREATE; messages before it arrive
    // during CreateWindowExW: creating)
    // SAFETY: our window.
    let mut k = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    if k == 0 {
        if msg == WM_NCCREATE {
            // SAFETY: lParam points to the CREATESTRUCTW; lpCreateParams is its first field.
            k = unsafe { *(lp as *const isize) };
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, k) };
        } else {
            k = s.creating.get() as isize;
        }
    }
    let idx = k.wrapping_sub(1);
    let ours = (0..PW_MAX as isize).contains(&idx) && {
        let r = &s.ww[idx as usize];
        if r.hwnd.get() == hwnd {
            true
        } else if r.hwnd.get() == 0 && k == s.creating.get() as isize && k != 0 {
            r.hwnd.set(hwnd); // the first messages of a new window
            true
        } else {
            false // late messages of a destroyed one
        }
    };
    if !ours {
        // SAFETY: not one of ours (should not happen).
        return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    }
    let save = s.ev_win.get();
    s.ev_win.set(idx as i32);
    // SAFETY: the message parameters come from the system for this window.
    let r = unsafe { dispatch(hwnd, msg, wp, lp, idx as i32) };
    s.ev_win.set(save);
    r
}

// the window procedure of window w (ev_win = w)
unsafe fn dispatch(hwnd: isize, msg: u32, wp: usize, lp: isize, w: i32) -> isize {
    let s = st();
    let r = &s.ww[w as usize];
    // SAFETY: default handling of the message.
    let def = || unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    match msg {
        WM_POINTERUPDATE..=WM_POINTERCAPTURECHANGED => {
            // (PN1) pens: handled -> 0, else DefWindowProc (mouse emulation)
            if pen::win_pointer(hwnd, msg, wp, w) { 0 } else { def() }
        }
        WM_MOUSEMOVE => {
            let (x, y) = lp_xy(lp);
            r.mx.set(x);
            r.my.set(y);
            s.ptr_win.set(w);
            if !r.track.get() {
                r.track.set(true);
                track_leave(hwnd, TME_LEAVE);
            }
            win_push(E_MOVE, 0, x, y, 0);
            0
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
            let b = button(msg, wp);
            s.btn_mask.set(s.btn_mask.get() | 1 << b);
            s.btn_win.set(w);
            let (x, y) = lp_xy(lp);
            r.mx.set(x);
            r.my.set(y);
            // SAFETY: our window.
            unsafe { SetCapture(hwnd) };
            win_push(E_DOWN, b, x, y, 0);
            (msg == WM_XBUTTONDOWN) as isize // X buttons must return TRUE
        }
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => {
            let b = button(msg, wp);
            s.btn_mask.set(s.btn_mask.get() & !(1 << b));
            let (x, y) = lp_xy(lp);
            win_push(E_UP, b, x, y, 0);
            if s.btn_mask.get() == 0 {
                // SAFETY: no arguments.
                unsafe { ReleaseCapture() };
            }
            (msg == WM_XBUTTONUP) as isize
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let d = (wp >> 16) as u16 as u32;
            let data = if msg == WM_MOUSEWHEEL { d } else { d << 16 }; // dy up / dx right positive
            // the position arrives in screen coordinates; it belongs to our window under the
            // pointer (Windows may send it to the focus window, e.g. the main window while the
            // pointer is over a tool window that never takes the focus) (U3)
            let (sx, sy) = lp_xy(lp);
            let mut to = hwnd;
            // SAFETY: a POINT by value.
            let under = unsafe { WindowFromPoint(Point { x: sx, y: sy }) };
            if under != 0 && under != hwnd {
                match (0..PW_MAX).find(|&i| s.ww[i].hwnd.get() == under) {
                    Some(i) => {
                        to = under;
                        s.ev_win.set(i as i32); // (restored on return)
                    }
                    None => {
                        // one of the popup windows: pushed for its owner
                        if popup::win_popup_wheel(under, sx, sy, data) {
                            return 0;
                        }
                    }
                }
            }
            let mut p = Point { x: sx, y: sy };
            // SAFETY: our window and a valid POINT.
            unsafe { ScreenToClient(to, &mut p) };
            win_push(E_WHEEL, 0, p.x, p.y, data);
            0
        }
        WM_MOUSELEAVE => {
            r.track.set(false);
            if popup::win_popup_under(w) {
                return 0; // (U3) onto one of its popup windows: the pointer is still "in" it
            }
            if s.ptr_win.get() == w {
                s.ptr_win.set(-1);
            }
            win_push(E_LEAVE, 0, 0, 0, 0);
            0
        }
        WM_CAPTURECHANGED => {
            if lp != hwnd {
                release_all_buttons();
            }
            0
        }
        WM_SYSKEYDOWN => {
            push_key(E_KEY_DOWN, wp, lp);
            if wp as u32 as usize == VK_F4 { def() } else { 0 } // keep Alt+F4; no system menu on Alt / F10
        }
        WM_KEYDOWN => {
            push_key(E_KEY_DOWN, wp, lp);
            0
        }
        WM_KEYUP | WM_SYSKEYUP => {
            push_key(E_KEY_UP, wp, lp);
            0
        }
        0x010F => { // WM_IME_COMPOSITION: default processing still commits via WM_CHAR.
            clip::ime_composition(hwnd,s.ev_win.get() as u8,lp as u32); def()
        }
        0x010E => { crate::ime_preedit_set(s.ev_win.get() as u8,"",0); def() } // WM_IME_ENDCOMPOSITION
        WM_CHAR => {
            let mut c = wp as u16 as u32;
            match c & 0xFC00 {
                0xD800 => {
                    s.hi_surr.set(c); // high surrogate: wait for the low half
                    return 0;
                }
                0xDC00 => {
                    let hi = s.hi_surr.get();
                    if hi == 0 {
                        return 0;
                    }
                    s.hi_surr.set(0);
                    c = ((hi - 0xD800) << 10) + (c - 0xDC00) + 0x10000;
                }
                _ => {}
            }
            if c >= 32 && c != 127 {
                win_push(E_TEXT, 0, 0, 0, c); // control characters come as keys
            }
            0
        }
        WM_SYSCHAR => 0,
        WM_SETCURSOR => {
            if lp as u16 as isize != HTCLIENT {
                return def();
            }
            // SAFETY: a cursor handle (0 hides the cursor).
            unsafe { SetCursor(cur_handle(r.cursor.get())) };
            1
        }
        WM_SIZE => {
            if wp == SIZE_MINIMIZED {
                r.min.set(true);
                return 0;
            }
            r.min.set(false);
            let (cw, ch) = ((lp & 0xFFFF) as i32, ((lp >> 16) & 0xFFFF) as i32);
            if cw == r.w.get() && ch == r.h.get() {
                return 0;
            }
            r.w.set(cw);
            r.h.set(ch);
            if r.selfp.get() == SELF_CREATE {
                return 0; // being created: not reported
            }
            let q = r.req.get();
            if r.selfp.get() == SELF_PLACE && cw == q[2] && ch == q[3] {
                return 0; // our own SetWindowPos gave what we asked
            }
            win_push(E_RESIZE, 0, cw, ch, 0);
            live_refresh();
            0
        }
        WM_MOVE => {
            // SAFETY: our window.
            if unsafe { IsIconic(hwnd) } != 0 {
                return 0; // minimized windows sit at -32000
            }
            let (x, y) = (lp as i16 as i32, ((lp as i32) >> 16) as i16 as i32); // client origin (screen)
            if x == r.px.get() && y == r.py.get() {
                return 0;
            }
            if r.selfp.get() != SELF_CREATE {
                ww_follow(w, x, y); // docked tool windows move along (live, also in the move loop)
            }
            r.px.set(x);
            r.py.set(y);
            if r.selfp.get() == SELF_CREATE {
                return 0;
            }
            let q = r.req.get();
            if r.selfp.get() == SELF_PLACE && x == q[0] && y == q[1] {
                return 0;
            }
            if s.in_sizemove.get() {
                r.movepend.set(true); // reported when the move ends
                return 0;
            }
            win_push(E_WINPOS, 0, x, y, 0);
            0
        }
        WM_PAINT => {
            // SAFETY: plain Win32 paint bracket on our window.
            let mut ps: PaintStruct = unsafe { core::mem::zeroed() };
            let dc = unsafe { BeginPaint(hwnd, &mut ps) };
            if r.psf.get() {
                present_kept(r, dc, None);
            }
            unsafe { EndPaint(hwnd, &ps) };
            win_push(E_PAINT, 0, 0, 0, 0);
            0
        }
        WM_ERASEBKGND => 1,
        // UI Automation asks for the window's accessibility tree (win_uia.rs)
        WM_GETOBJECT => win_uia::get_object(hwnd, w, wp, lp).unwrap_or_else(def),
        WM_CLOSE => {
            win_push(E_CLOSE, 0, 0, 0, 0);
            0
        }
        // lifecycle (the main window speaks for the app)
        WM_POWERBROADCAST if w == 0 => {
            let sleep = match wp {
                PBT_APMSUSPEND => Some(true),
                PBT_APMRESUMESUSPEND | PBT_APMRESUMEAUTOMATIC => Some(false),
                _ => None,
            };
            if let Some(on) = sleep {
                if SLEEPING.swap(on, core::sync::atomic::Ordering::Relaxed) != on {
                    win_push(E_POWER, 0, 0, 0, on as u32);
                }
            }
            1
        }
        WM_QUERYENDSESSION | WM_ENDSESSION if w == 0 => {
            // the app saves on E_SESSION_END right after this returns (before WM_ENDSESSION
            // arrives: Windows asks every app first)
            if (msg == WM_QUERYENDSESSION || wp != 0) && !ENDING.swap(true, core::sync::atomic::Ordering::Relaxed) {
                win_push(E_SESSION_END, 0, 0, 0, 0);
            }
            if msg == WM_QUERYENDSESSION { 1 } else { 0 }
        }
        WM_DISPLAYCHANGE if w == 0 => {
            // a monitor came, went, moved or changed its mode (screen.rs: monitors())
            win_push(E_SETTINGS, 0, 0, 0, SET_MONITORS);
            def()
        }
        WM_SETTINGCHANGE | WM_SYSCOLORCHANGE if w == 0 => {
            let bits = if msg == WM_SYSCOLORCHANGE { SET_CONTRAST } else { setting_bits(wp, lp) };
            if bits != 0 {
                win_push(E_SETTINGS, 0, 0, 0, bits);
            }
            def()
        }
        WM_DESTROY => {
            win_uia::destroyed(hwnd);
            win_dnd::revoke(hwnd);
            r.hwnd.set(0);
            if w == 0 {
                super::thread::wake_window(0);
                // SAFETY: the main window: end the message loop.
                unsafe { PostQuitMessage(0) };
            }
            0
        }
        WM_SETFOCUS => {
            win_push(E_FOCUS_IN, 0, 0, 0, 0);
            0
        }
        WM_KILLFOCUS => {
            release_all_buttons();
            clip::caret_focus_lost(hwnd); // magnifiers: no caret in a window without the keyboard
            win_push(E_FOCUS_OUT, 0, 0, 0, 0);
            0
        }
        WM_DPICHANGED => {
            r.dpi.set(wp as u16 as i32);
            win_push(E_DPI, 0, 0, 0, r.dpi.get() as u32); // before the resize it causes
            // SAFETY: lParam points to the suggested window RECT.
            let q = unsafe { *(lp as *const Rect) };
            unsafe { SetWindowPos(hwnd, 0, q.l, q.t, q.r - q.l, q.b - q.t, SWP_NOZORDER | SWP_NOACTIVATE) };
            0
        }
        WM_GETMINMAXINFO => {
            if r.minw.get() == 0 || r.hwnd.get() == 0 || r.selfp.get() == SELF_CREATE {
                return 0; // (being created: insets not known yet)
            }
            let ins = win_insets(hwnd);
            let mw = dip(r.minw.get(), r.dpi.get()) + ins[0] + ins[2];
            let mh = dip(r.minh.get(), r.dpi.get()) + ins[1] + ins[3];
            // SAFETY: lParam points to MINMAXINFO (ptMinTrackSize at +24).
            unsafe {
                let mmi = lp as *mut i32;
                *mmi.add(6) = mw;
                *mmi.add(7) = mh;
            }
            0
        }
        // ---- library frame (no caption)
        WM_NCCALCSIZE => {
            if r.flags.get() & WWF_CUSTOM == 0 || wp == 0 {
                return def();
            }
            // SAFETY: lParam points to NCCALCSIZE_PARAMS (rgrc[0].top at +4).
            let top = unsafe { *((lp + 4) as *const i32) };
            def(); // standard borders ...
            let mut t = top; // ... but no caption
            // SAFETY: our window.
            if unsafe { IsZoomed(hwnd) } != 0 {
                // maximized windows hang over the screen edge by the frame size
                t += unsafe { GetSystemMetrics(SM_CYFRAME) + GetSystemMetrics(SM_CXPADDEDBORDER) };
            }
            unsafe { *((lp + 4) as *mut i32) = t };
            0
        }
        WM_NCHITTEST => {
            if r.flags.get() & WWF_CUSTOM == 0 {
                return def();
            }
            let ht = def();
            if ht != HTCLIENT {
                return ht; // side and bottom borders
            }
            let (x, y) = nc_client_pos(hwnd, lp);
            // top resize band (resizable windows, not when maximized)
            // SAFETY: our window.
            if r.flags.get() & (WWF_TOP | PWF_RESIZABLE) != 0 && unsafe { IsZoomed(hwnd) } == 0 {
                let band = (r.dpi.get() + 12) >> 4; // about 6 px at 100 %
                if y < band {
                    if x < band * 2 {
                        return HTTOPLEFT;
                    }
                    if x < r.w.get() - band - band {
                        return HTTOP;
                    }
                    return HTTOPRIGHT;
                }
            }
            let hb = ui_hit_at(w, x, y);
            if r.flags.get() & WWF_TOP != 0 {
                // owned windows: buttons are client area
                if hb & HB_CLOSE != 0 {
                    return HTCLOSE;
                }
                if hb & HB_MAX != 0 {
                    return HTMAXBUTTON;
                }
                if hb & HB_MIN != 0 {
                    return HTMINBUTTON;
                }
            }
            if hb & (HB_CLOSE | HB_MAX | HB_MIN) == 0 && hb & HB_DRAG != 0 { HTCAPTION } else { HTCLIENT }
        }
        // caption buttons are non-client for Windows (snap layouts) but drawn by the UI:
        // their mouse messages become client events
        WM_NCMOUSEMOVE => {
            if !nc_is_button(wp) {
                return def();
            }
            let (x, y) = nc_client_pos(hwnd, lp);
            if !r.nctrack.get() {
                r.nctrack.set(true);
                track_leave(hwnd, TME_LEAVE | TME_NONCLIENT);
            }
            win_push(E_MOVE, 0, x, y, 0);
            0
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK | WM_NCLBUTTONUP => {
            if !nc_is_button(wp) {
                return def();
            }
            let (x, y) = nc_client_pos(hwnd, lp);
            win_push(if msg == WM_NCLBUTTONUP { E_UP } else { E_DOWN }, crate::BTN_LEFT, x, y, 0);
            0
        }
        WM_NCMOUSELEAVE => {
            r.nctrack.set(false);
            win_push(E_LEAVE, 0, 0, 0, 0);
            def()
        }
        // ---- activation
        WM_MOUSEACTIVATE => {
            // owned tool windows keep the keyboard where it is, unless a text box is clicked
            let f = r.flags.get();
            if f & WWF_TOP != 0 || f & (PWF_TOOL | PWF_NOACTIVATE) == 0 {
                return def();
            }
            if f & PWF_NOACTIVATE != 0 || lp as u16 as isize != HTCLIENT {
                return MA_NOACTIVATE;
            }
            // the pointer moved there before the click
            if ui_hit_at(w, r.mx.get(), r.my.get()) & HB_TEXT != 0 { MA_ACTIVATE } else { MA_NOACTIVATE }
        }
        WM_NCACTIVATE => {
            // another window of the app is being activated: keep looking active
            if wp != 0 || s.nc_force.get() {
                return def();
            }
            // SAFETY: default handling with wParam TRUE.
            unsafe { DefWindowProcW(hwnd, msg, 1, lp) }
        }
        WM_ACTIVATEAPP => {
            let active = wp != 0;
            if active == s.app_active.get() {
                return 0; // every window of the app gets this
            }
            s.app_active.set(active);
            if !active {
                s.nc_force.set(true); // the app loses the activation: every frame looks inactive
            }
            for i in 0..PW_MAX {
                let h = s.ww[i].hwnd.get();
                if h != 0 {
                    // SAFETY: our window.
                    unsafe { SendMessageW(h, WM_NCACTIVATE, active as usize, 0) };
                }
            }
            s.nc_force.set(false);
            0
        }
        WM_ENTERSIZEMOVE => {
            s.in_sizemove.set(true);
            def()
        }
        WM_EXITSIZEMOVE => {
            s.in_sizemove.set(false);
            if r.movepend.get() {
                r.movepend.set(false);
                win_push(E_WINPOS, 0, r.px.get(), r.py.get(), 0);
            }
            def()
        }
        super::thread::WM_APP_WAKE => {
            evq_push(super::thread::wake_consume()); // plat::wake from another thread -> E_WAKE
            0
        }
        WM_DROPFILES => {
            // K1: files dropped -> E_DROP (x, y, count)
            let (n, x, y) = clip::win_drop_files(wp as isize);
            if n != 0 {
                win_push(E_DROP, 0, x, y, n);
            }
            0
        }
        _ => {
            if w == 0 {
                crate::taskbar::window_message(msg); // "TaskbarButtonCreated": progress, overlay again
            }
            def()
        }
    }
}

/// BTN_* of a button message (X buttons: wParam's high word 2 = X2)  [xbutton]
fn button(msg: u32, wp: usize) -> u8 {
    match msg {
        WM_LBUTTONDOWN | WM_LBUTTONUP => crate::BTN_LEFT,
        WM_RBUTTONDOWN | WM_RBUTTONUP => crate::BTN_RIGHT,
        WM_MBUTTONDOWN | WM_MBUTTONUP => crate::BTN_MIDDLE,
        _ if (wp >> 16) & 0xFFFF == 2 => crate::BTN_X2,
        _ => crate::BTN_X1,
    }
}

/// key event from wParam / lParam: VK (8 bits) | previous key state << 31  [push_key]
fn push_key(typ: u8, wp: usize, lp: isize) {
    let data = (wp as u32 & 0xFF) | (((lp as u32) >> 30) & 1) << 31;
    win_push(typ, 0, 0, 0, data);
}

/// wParam is a caption button hit code  [nc_is_button]
fn nc_is_button(wp: usize) -> bool {
    let h = wp as u32 as isize;
    h == HTMINBUTTON || h == HTMAXBUTTON || h == HTCLOSE
}

/// f(w, x, y) -> HB_* flags at a client point (None = no hit tests)
pub fn win_hit_hook(f: Option<fn(i32, i32, i32) -> u32>) {
    st().hit.set(f);
}

/// f() redraws during the modal move / size loop
pub fn set_refresh(f: Option<fn()>) {
    st().refresh.set(f);
}

// ---------------------------------------------------------------- loop

/// wait() with a redraw closure for the modal move / size loop: refresh() runs (instead of the
/// set_refresh hook) whenever the window must redraw before the loop returns  [main agent]
pub fn wait_refresh(deadline_us: u64, refresh: &mut dyn FnMut()) -> u32 {
    let s = st();
    let p: *mut (dyn FnMut() + '_) = refresh;
    // SAFETY: only the lifetime is erased; the pointer is removed again before this function
    // returns, so it never outlives the borrow.
    let p: *mut (dyn FnMut() + 'static) = unsafe { core::mem::transmute(p) };
    let prev = s.refresh_dyn.replace(Some(p));
    let n = wait(deadline_us);
    s.refresh_dyn.set(prev);
    n
}

// wait() dispatches messages while fewer events are queued (room for what one message adds)
const PUMP_MAX: u32 = (crate::EVQ_CAP - 64) as u32;

/// pump the OS until an event is queued or the deadline (plat::time_us; 0 = poll without
/// blocking, u64::MAX = forever) passes -> events queued (wake() makes it return at once)
pub fn wait(deadline_us: u64) -> u32 {
    // SAFETY: a plain MSG record, filled by PeekMessageW.
    let mut m: Msg = unsafe { core::mem::zeroed() };
    loop {
        win_dnd::run_pending(); // the app drags something out: DoDragDrop (modal)
        // SAFETY: the standard message loop on the UI thread.  A burst (a WM_CHAR per typed
        // or committed character) stops short of a full event ring: the rest waits in the
        // thread's queue until the app took these events
        while evq_count() < PUMP_MAX && unsafe { PeekMessageW(&mut m, 0, 0, 0, PM_REMOVE) } != 0 {
            if m.msg == WM_QUIT {
                evq_push(Event { typ: E_CLOSE, ..Event::default() });
                continue;
            }
            unsafe {
                TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
        // plat::wake without a window: its event (checked on every call, like the posted
        // message, so wait(0) takes it too)
        let ev = super::thread::wake_event();
        // SAFETY: a valid event handle.
        if ev != 0 && unsafe { WaitForSingleObject(ev, 0) } == 0 {
            evq_push(super::thread::wake_consume());
        }
        let n = evq_count();
        if n != 0 || deadline_us == 0 {
            return n;
        }
        let ms = if deadline_us == u64::MAX {
            INFINITE
        } else {
            let now = crate::time_us();
            if now >= deadline_us {
                return evq_count();
            }
            ((deadline_us - now + 999) / 1000).min(INFINITE as u64 - 1) as u32
        };
        // handles: the plat::wake event, if any
        // SAFETY: ev is a valid handle when n = 1.
        let r = unsafe { MsgWaitForMultipleObjectsEx((ev != 0) as u32, &ev as *const isize as usize, ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
        if r == WAIT_TIMEOUT {
            return evq_count();
        }
        if r == 0 && ev != 0 {
            evq_push(super::thread::wake_consume()); // WAIT_OBJECT_0: the wake event
        }
        // else messages (or a failure): pump again
    }
}

// ---------------------------------------------------------------- presentation

/// the frame's pixel rows are usable: w, h > 0, stride a multiple of 4 holding a row, enough
/// bytes for h rows
fn frame_ok(f: &Frame) -> bool {
    f.w > 0
        && f.h > 0
        && f.stride >= f.w.saturating_mul(4)
        && f.stride % 4 == 0
        && (f.px.len() as u64) >= f.stride as u64 * f.h as u64
}

/// show the frame's dirty rectangles (x, y, w, h; None = the whole frame); the window keeps a
/// copy of the pixels (WM_PAINT repaints from it)  [plat_win_present]
pub fn win_present(w: i32, f: &Frame, rects: Option<&[[i32; 4]]>) {
    let Some(r) = rec(w) else { return };
    if !frame_ok(f) {
        return;
    }
    // the copy: everything when the size changed, else the rectangles
    let mut k = r.kept.take();
    let dim = (f.w, f.h, f.stride);
    let n = (f.stride * f.h) as usize;
    match rects {
        Some(rs) if r.psf.get() && r.kdim.get() == dim && k.len() == n => {
            for q in rs {
                if let Some((x, y, cw, ch)) = clip_rect(q, f.w, f.h) {
                    let (x0, x1) = (x as usize * 4, (x + cw) as usize * 4);
                    for row in y..y + ch {
                        let o = row as usize * f.stride as usize;
                        k[o + x0..o + x1].copy_from_slice(&f.px[o + x0..o + x1]);
                    }
                }
            }
        }
        _ => {
            k.clear();
            k.extend_from_slice(&f.px[..n]);
        }
    }
    r.kept.set(k);
    r.kdim.set(dim);
    r.psf.set(true);
    if r.min.get() {
        return;
    }
    let hwnd = r.hwnd.get();
    // SAFETY: a DC of our window, released again.
    let dc = unsafe { GetDC(hwnd) };
    match rects {
        None => present_kept(r, dc, None),
        Some(rs) => {
            for q in rs {
                present_kept(r, dc, Some(*q));
            }
        }
    }
    // SAFETY: the DC from above.
    unsafe { ReleaseDC(hwnd, dc) };
}

/// a rectangle clipped to w x h -> (x, y, w, h) with w, h > 0
fn clip_rect(q: &[i32; 4], fw: i32, fh: i32) -> Option<(i32, i32, i32, i32)> {
    let (mut x, mut y, mut w, mut h) = (q[0], q[1], q[2], q[3]);
    if x < 0 {
        w = w.wrapping_add(x);
        x = 0;
    }
    if y < 0 {
        h = h.wrapping_add(y);
        y = 0;
    }
    w = w.min(fw - x);
    h = h.min(fh - y);
    if w <= 0 || h <= 0 { None } else { Some((x, y, w, h)) }
}

/// one SetDIBitsToDevice call from the kept copy (rect None = all, clipped)  [present_rect]
fn present_kept(r: &Ww, dc: isize, rect: Option<[i32; 4]>) {
    let (fw, fh, stride) = r.kdim.get();
    let k = r.kept.take();
    let q = rect.unwrap_or([0, 0, fw, fh]);
    if let Some((x, y, w, h)) = clip_rect(&q, fw, fh)
        && k.len() >= (stride * fh) as usize
    {
        let bmi = BitmapInfoHeader {
            size: 40,
            w: stride / 4, // biWidth = stride in pixels
            h: -h,         // top-down, h rows
            planes: 1,
            bits: 32,
            ..Default::default()
        };
        // SAFETY: rows y .. y + h of the copy (stride bytes each) lie inside it (checked).
        unsafe {
            let bits = k.as_ptr().add(y as usize * stride as usize);
            SetDIBitsToDevice(dc, x, y, w as u32, h as u32, x, 0, 0, h as u32, bits, &bmi, 0);
        }
    }
    r.kept.set(k);
}

// ---------------------------------------------------------------- window operations

/// close the window (w = 0: ends the message loop)  [plat_win_destroy]
pub fn win_destroy(w: i32) {
    if let Some(r) = rec(w) {
        // SAFETY: our window.
        unsafe { DestroyWindow(r.hwnd.get()) };
    }
    if let Some(r) = st().ww.get(usize::try_from(w).unwrap_or(PW_MAX)) {
        r.clear();
        if w == 0 {
            super::thread::wake_window(0);
        }
        if st().ptr_win.get() == w {
            st().ptr_win.set(-1);
        }
    }
}

/// PWS_HIDE / PWS_SHOW (activates) / PWS_SHOWNA
pub fn win_show(w: i32, mode: u32) {
    let Some(r) = rec(w) else { return };
    let cmd = match mode {
        0 => SW_HIDE,
        PWS_SHOWNA => SW_SHOWNA,
        _ => SW_SHOW,
    };
    // SAFETY: our window.
    unsafe { ShowWindow(r.hwnd.get(), cmd) };
}

/// make the window the active one (keyboard focus)
pub fn win_activate(w: i32) {
    let Some(r) = rec(w) else { return };
    if r.flags.get() & PWF_NOACTIVATE != 0 {
        return;
    }
    // SAFETY: our window.
    unsafe {
        if IsWindowVisible(r.hwnd.get()) != 0 {
            SetActiveWindow(r.hwnd.get());
        }
    }
}

/// a disabled window takes no input (modal dialogs disable the windows below them)
pub fn win_enable(w: i32, on: bool) {
    if let Some(r) = rec(w) {
        // SAFETY: our window.
        unsafe { EnableWindow(r.hwnd.get(), on as i32) };
    }
}

/// client origin (screen px) and client size; PW_KEEP keeps a field
pub fn win_set_rect(w: i32, x: i32, y: i32, cw: i32, ch: i32) {
    win_place(w, x, y, cw, ch)
}

/// client origin (screen px) and client size (minimized: where it was); None: no window
pub fn win_get_rect(w: i32) -> Option<[i32; 4]> {
    let r = rec(w)?;
    if r.min.get() {
        return Some([r.px.get(), r.py.get(), r.w.get(), r.h.get()]);
    }
    let (mut p, mut c) = (Point::default(), Rect::default());
    // SAFETY: our window, valid out-parameters.
    unsafe {
        ClientToScreen(r.hwnd.get(), &mut p);
        GetClientRect(r.hwnd.get(), &mut c);
    }
    Some([p.x, p.y, c.r, c.b])
}

/// work area (without the taskbar) of the monitor the window is on: x, y, w, h (screen px)
pub fn win_work_area(w: i32) -> Option<[i32; 4]> {
    let r = rec(w)?;
    // SAFETY: our window; MONITORINFO with its size set.
    unsafe {
        let m = MonitorFromWindow(r.hwnd.get(), MONITOR_NEAREST);
        if m == 0 {
            return None;
        }
        let mut mi = MonitorInfo { size: 40, ..Default::default() };
        if GetMonitorInfoW(m, &mut mi) == 0 {
            return None;
        }
        Some([mi.work.l, mi.work.t, mi.work.r - mi.work.l, mi.work.b - mi.work.t])
    }
}

pub fn win_set_title(w: i32, title: &str) {
    if let Some(r) = rec(w) {
        let wt = wide_cap(title, TITLE_MAX);
        // SAFETY: our window, a NUL-terminated title.
        unsafe { SetWindowTextW(r.hwnd.get(), wt.as_ptr()) };
    }
}

/// WST_* bits (WST_ACTIVE: one of the app's windows is active)
pub fn win_state(w: i32) -> u32 {
    let Some(r) = rec(w) else { return 0 };
    let mut v = 0;
    // SAFETY: our window.
    unsafe {
        if IsZoomed(r.hwnd.get()) != 0 {
            v |= WST_MAXIMIZED;
        }
        if IsIconic(r.hwnd.get()) != 0 {
            v |= WST_MINIMIZED;
        }
    }
    if st().app_active.get() {
        v |= WST_ACTIVE;
    }
    v
}

/// WCMD_MINIMIZE / MAXIMIZE / RESTORE / CLOSE (asks, like the system menu does)
pub fn win_cmd(w: i32, cmd: u32) {
    let Some(r) = rec(w) else { return };
    let h = r.hwnd.get();
    // SAFETY: our window.
    unsafe {
        match cmd {
            WCMD_CLOSE => {
                PostMessageW(h, WM_CLOSE, 0, 0);
            }
            WCMD_MINIMIZE => {
                ShowWindow(h, SW_MINIMIZE);
            }
            WCMD_MAXIMIZE => {
                ShowWindow(h, SW_MAXIMIZE);
            }
            _ => {
                ShowWindow(h, SW_RESTORE);
            }
        }
    }
}

/// let the system move the window (call on a button press)
pub fn win_begin_move(w: i32) {
    nc_drag(w, HTCAPTION as usize)
}

/// let the system resize the window from an edge (EDGE_* bits)
pub fn win_begin_resize(w: i32, edges: u32) {
    nc_drag(w, HT_EDGE[(edges & 15) as usize] as usize)
}

fn nc_drag(w: i32, ht: usize) {
    let Some(r) = rec(w) else { return };
    // SAFETY: our window; the system runs its modal move / size loop.
    unsafe { ReleaseCapture() };
    st().btn_mask.set(0);
    unsafe { SendMessageW(r.hwnd.get(), WM_NCLBUTTONDOWN, ht, 0) };
}

/// client width, height in px ((0, 0) without a window)
pub fn win_size(w: i32) -> (i32, i32) {
    rec(w).map_or((0, 0), |r| (r.w.get(), r.h.get()))
}

/// 96 = 100 %
pub fn win_dpi(w: i32) -> i32 {
    match rec(w) {
        Some(r) if r.dpi.get() != 0 => r.dpi.get(),
        _ => 96,
    }
}

/// minimum client size (also before the window exists)
pub fn win_set_min_size(w: i32, w_dip: i32, h_dip: i32) {
    if let Some(r) = st().ww.get(usize::try_from(w).unwrap_or(PW_MAX)) {
        r.minw.set(w_dip);
        r.minh.set(h_dip);
    }
}

/// messages whose parameters are plain values (no pointers): the ones a test may send
fn pointer_free(msg: u32) -> bool {
    matches!(msg,
        WM_CLOSE | WM_ACTIVATEAPP | WM_SETCURSOR | WM_MOUSEACTIVATE | WM_NCHITTEST | WM_NCACTIVATE
        | WM_NCMOUSEMOVE..=WM_NCLBUTTONDBLCLK | WM_KEYDOWN..=WM_SYSCHAR | WM_MOUSEMOVE..=WM_MOUSEHWHEEL
        | WM_CAPTURECHANGED | WM_POINTERUPDATE..=WM_POINTERCAPTURECHANGED | WM_ENTERSIZEMOVE | WM_EXITSIZEMOVE
        | WM_NCMOUSELEAVE | WM_MOUSELEAVE | WM_DISPLAYCHANGE)
}

/// (test aid) SendMessageW to window w -> its result (0 without the window or for a message
/// that could carry a pointer)
pub fn win_test_send(w: i32, msg: u32, wp: usize, lp: isize) -> isize {
    match rec(w) {
        // SAFETY: our window; only messages whose parameters are plain values.
        Some(r) if pointer_free(msg) || (msg == WM_SETTINGCHANGE && lp == 0) => unsafe { SendMessageW(r.hwnd.get(), msg, wp, lp) },
        _ => 0,
    }
}

// ---------------------------------------------------------------- cursors

/// CUR_* shape while the pointer is over window w
pub fn win_set_cursor(w: i32, shape: u32) {
    if shape >= CUR_TABLE {
        return;
    }
    let Some(r) = rec(w) else { return };
    if r.cursor.get() == shape {
        return;
    }
    r.cursor.set(shape);
    if st().ptr_win.get() == w {
        // SAFETY: a cursor handle (0 hides it).
        unsafe { SetCursor(cur_handle(shape)) };
    }
}

/// HCURSOR: the built-in shapes (CUR_NONE = 0: hidden), a custom cursor, the arrow for empty /
/// unknown slots  [cur_handle]
fn cur_handle(shape: u32) -> isize {
    let c = &st().cursors;
    let arrow = c[CUR_ARROW as usize].get();
    if shape >= CUR_TABLE {
        return arrow;
    }
    if shape < CUR_COUNT {
        return c[shape as usize].get();
    }
    if shape < CUR_USER0 {
        return arrow;
    }
    match c[shape as usize].get() {
        0 => arrow,
        h => h,
    }
}

/// a cursor from w x h premultiplied 0xAARRGGBB pixels (rows of w * 4 bytes), hot spot in px:
/// a 32-bit top-down DIB section with straight alpha + an empty AND mask -> CreateIconIndirect;
/// Windows shows it at its size (dpi is not needed here) -> CUR_USER0 + k / -1
pub fn cursor_create(px: &[u8], w: i32, h: i32, hot_x: i32, hot_y: i32, _dpi: i32) -> i32 {
    if (w - 1) as u32 >= CUR_MAX_PX as u32 || (h - 1) as u32 >= CUR_MAX_PX as u32 {
        return -1;
    }
    let n = (w * h) as usize;
    if px.len() < n * 4 {
        return -1;
    }
    let s = st();
    let Some(slot) = (CUR_USER0..CUR_TABLE).find(|&k| s.cursors[k as usize].get() == 0) else { return -1 };
    let bmi = BitmapInfoHeader { size: 40, w, h: -h, planes: 1, bits: 32, ..Default::default() };
    let mut bits: *mut u8 = core::ptr::null_mut();
    // SAFETY: a screen DC, released again; a valid BITMAPINFOHEADER (BI_RGB, no colour table).
    let color = unsafe {
        let dc = GetDC(0);
        let c = CreateDIBSection(dc, &bmi, 0, &mut bits, 0, 0);
        ReleaseDC(0, dc);
        c
    };
    if color == 0 {
        return -1;
    }
    if bits.is_null() {
        // SAFETY: the bitmap from above.
        unsafe { DeleteObject(color) };
        return -1;
    }
    // SAFETY: the DIB section holds w * h 32-bit pixels.
    let dst = unsafe { core::slice::from_raw_parts_mut(bits as *mut u32, n) };
    for (d, p) in dst.iter_mut().zip(px.chunks_exact(4)) {
        let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
        let a = v >> 24;
        *d = if a == 0 || a == 255 {
            v
        } else {
            // straight alpha for Windows
            let mut o = a << 24;
            for sh in [16, 8, 0] {
                let c = (((v >> sh) & 255) * 255 + (a >> 1)) / a;
                o |= c.min(255) << sh;
            }
            o
        };
    }
    // AND mask: monochrome, word-aligned rows, all 0 (the alpha decides)
    let mask_bytes = vec_zero((((w + 15) >> 4) * 2 * h) as usize);
    // SAFETY: mask_bytes holds the word-aligned rows of a w x h monochrome bitmap.
    let mask = unsafe { CreateBitmap(w, h, 1, 1, mask_bytes.as_ptr()) };
    if mask == 0 {
        // SAFETY: the bitmap from above.
        unsafe { DeleteObject(color) };
        return -1;
    }
    let ii = IconInfo { icon: 0, hot_x: hot_x as u32 & 0xFFFF, hot_y: hot_y as u32 & 0xFFFF, mask, color };
    // SAFETY: a valid ICONINFO (fIcon FALSE: a cursor); the bitmaps are copied, then freed.
    let cur = unsafe {
        let c = CreateIconIndirect(&ii);
        DeleteObject(mask);
        DeleteObject(color);
        c
    };
    if cur == 0 {
        return -1;
    }
    s.cursors[slot as usize].set(cur);
    slot as i32
}

fn vec_zero(n: usize) -> Vec<u8> {
    alloc::vec![0u8; n.max(1)]
}

/// free a custom cursor; windows showing it fall back to the arrow (their shape is forgotten,
/// so the next win_set_cursor applies even with the same id)
pub fn cursor_destroy(id: i32) {
    let k = (id as u32).wrapping_sub(CUR_USER0);
    if k >= CUR_USER_MAX {
        return;
    }
    let s = st();
    let slot = &s.cursors[(CUR_USER0 + k) as usize];
    let h = slot.get();
    if h == 0 {
        return;
    }
    slot.set(0);
    for (i, r) in s.ww.iter().enumerate() {
        if r.cursor.get() == id as u32 {
            r.cursor.set(u32::MAX);
            if s.ptr_win.get() == i as i32 {
                // SAFETY: the arrow cursor.
                unsafe { SetCursor(s.cursors[CUR_ARROW as usize].get()) };
            }
        }
    }
    // SAFETY: the cursor was ours.
    unsafe { DestroyIcon(h) };
}

/// the dpi to draw a cursor for a window of this dpi (Windows: the window's own)
pub fn cursor_dpi(dpi: i32) -> i32 {
    dpi
}
