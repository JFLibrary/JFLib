// win_screen -- (screen.rs) the monitors, pictures of the screen and the colour pick on Windows
//
//   monitors() -> Vec<Monitor>   EnumDisplayMonitors; per monitor GetMonitorInfoW (MONITORINFOEXW:
//                                rectangle, work area, MONITORINFOF_PRIMARY, the device name),
//                                GetDpiForMonitor (shcore, effective dpi; else the system's),
//                                EnumDisplaySettingsW (ENUM_CURRENT_SETTINGS: dmDisplayFrequency)
//   capture_start(rect)          BitBlt (SRCCOPY | CAPTUREBLT) from the screen DC into a top-down
//                                32-bit DIB section, answered at once (ScreenEvent::Shot)
//   pick_start(test)             a frozen picture of the virtual desktop in a topmost window over
//                                all monitors, a magnifier (9 x 9 pixels) beside the pointer with
//                                the colour and its hex value; a left click, Enter or Space takes
//                                the pixel under the pointer, the arrows move the pointer by one
//                                pixel (Shift: ten), Esc / a right click cancels.  test = Some(x,
//                                y): that pixel at once (1 x 1 BitBlt), no window
//   (test aid) screen_test_metrics() -> (SM_CMONITORS, virtual desktop rectangle), the
//                                independent figures of GetSystemMetrics
//
// The process is made per-monitor DPI aware first (win_api.rs), so every figure is in physical
// pixels of the virtual desktop.  WM_DISPLAYCHANGE and a changed work area (SPI_SETWORKAREA)
// reach the app as E_SETTINGS with SET_MONITORS (win_api.rs).  The overlay belongs to the UI
// thread: its messages are dispatched by wait().

use alloc::string::String;
use alloc::vec::Vec;

use rt::sync::SpinLock;

use super::{
    BeginPaint, BitmapInfoHeader, CreateDIBSection, CreateWindowExW, DefWindowProcW, DeleteObject, DestroyWindow,
    EndPaint, GetDC, GetDeviceCaps, GetModuleHandleW, GetMonitorInfoW, GetSystemMetrics, LoadCursorW, MonitorInfo, PaintStruct, Point,
    Rect, RegisterClassExW, ReleaseDC, SetCursor, WndClassEx, dll_proc,
};
use crate::screen::{Monitor, ScreenError, Shot, push_color, push_shot};

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn EnumDisplayMonitors(dc: isize, clip: *const Rect, f: unsafe extern "system" fn(isize, isize, *mut Rect, isize) -> i32, data: isize) -> i32;
    fn EnumDisplaySettingsW(device: *const u16, mode: u32, dm: *mut u8) -> i32;
    fn InvalidateRect(h: isize, r: *const Rect, erase: i32) -> i32;
    fn FillRect(dc: isize, r: *const Rect, brush: isize) -> i32;
    fn FrameRect(dc: isize, r: *const Rect, brush: isize) -> i32;
    fn SetForegroundWindow(h: isize) -> i32;
    fn SetFocus(h: isize) -> isize;
    fn GetCursorPos(p: *mut Point) -> i32;
    fn SetCursorPos(x: i32, y: i32) -> i32;
    fn GetKeyState(vk: i32) -> i16;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateCompatibleDC(dc: isize) -> isize;
    fn DeleteDC(dc: isize) -> i32;
    fn SelectObject(dc: isize, obj: isize) -> isize;
    fn BitBlt(dst: isize, x: i32, y: i32, w: i32, h: i32, src: isize, xs: i32, ys: i32, rop: u32) -> i32;
    fn StretchBlt(dst: isize, x: i32, y: i32, w: i32, h: i32, src: isize, xs: i32, ys: i32, ws: i32, hs: i32, rop: u32) -> i32;
    fn SetStretchBltMode(dc: isize, mode: i32) -> i32;
    fn CreateSolidBrush(c: u32) -> isize;
    fn GetStockObject(i: i32) -> isize;
    fn SetBkMode(dc: isize, mode: i32) -> i32;
    fn SetTextColor(dc: isize, c: u32) -> u32;
    fn TextOutW(dc: isize, x: i32, y: i32, s: *const u16, n: i32) -> i32;
}

const SRCCOPY: u32 = 0x00CC_0020;
const CAPTUREBLT: u32 = 0x4000_0000;
const SM_XVIRTUALSCREEN: i32 = 76;
const SM_YVIRTUALSCREEN: i32 = 77;
const SM_CXVIRTUALSCREEN: i32 = 78;
const SM_CYVIRTUALSCREEN: i32 = 79;
const SM_CMONITORS: i32 = 80;
const MONITORINFOF_PRIMARY: u32 = 1;
const ENUM_CURRENT_SETTINGS: u32 = 0xFFFF_FFFF;
const DEVMODE_SIZE: usize = 220; // DEVMODEW
const DM_FREQ: usize = 184; // dmDisplayFrequency
const WS_POPUP: u32 = 0x8000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_EX_TOPMOST: u32 = 0x8;
const WS_EX_TOOLWINDOW: u32 = 0x80;
const WM_DESTROY: u32 = 0x0002;
const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETCURSOR: u32 = 0x0020;
const WM_KEYDOWN: u32 = 0x0100;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_CAPTURECHANGED: u32 = 0x0215;
const VK_SHIFT: i32 = 0x10;
const IDC_CROSS: usize = 32515;
const BLACK_BRUSH: i32 = 4;
const WHITE_BRUSH: i32 = 0;
const DEFAULT_GUI_FONT: i32 = 17;
const COLORONCOLOR: i32 = 3;
const TRANSPARENT: i32 = 1;
const MAG_CELLS: i32 = 9; // magnifier: source pixels per side
const MAG_GAP: i32 = 24; // px between the pointer and the magnifier (at 96 dpi)

/// MONITORINFOEXW (MONITORINFO + szDevice[32])
#[repr(C)]
struct MonitorInfoEx {
    mi: MonitorInfo,
    device: [u16; 32],
}

/// (crate) the process per-monitor DPI aware (once), as for the first window
fn dpi_aware() {
    super::win_dyn_init();
}

// the screen DC's dpi (the primary monitor's at start-up)
fn system_dpi() -> i32 {
    // SAFETY: the screen DC, released again.
    let d = unsafe {
        let dc = GetDC(0);
        let d = GetDeviceCaps(dc, 88); // LOGPIXELSX
        ReleaseDC(0, dc);
        d
    };
    if d <= 0 { 96 } else { d }
}

// EnumDisplayMonitors' callback: the HMONITORs into the Vec behind data
unsafe extern "system" fn enum_proc(mon: isize, _dc: isize, _r: *mut Rect, data: isize) -> i32 {
    // SAFETY: data is the &mut Vec<isize> of monitors() for the time of the call.
    let v = unsafe { &mut *(data as *mut Vec<isize>) };
    v.push(mon);
    1
}

// FNV-1a of the device name: the monitor's id
fn name_id(name: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn monitor_dpi(mon: isize) -> u32 {
    static GDFM: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(1);
    let mut p = GDFM.load(core::sync::atomic::Ordering::Relaxed);
    if p == 1 {
        p = dll_proc(b"shcore.dll\0", b"GetDpiForMonitor\0");
        GDFM.store(p, core::sync::atomic::Ordering::Relaxed);
    }
    if p != 0 {
        let (mut x, mut y) = (0u32, 0u32);
        // SAFETY: GetDpiForMonitor(HMONITOR, MDT_EFFECTIVE_DPI, UINT*, UINT*) -> HRESULT.
        let f: unsafe extern "system" fn(isize, u32, *mut u32, *mut u32) -> i32 = unsafe { core::mem::transmute(p) };
        if unsafe { f(mon, 0, &mut x, &mut y) } == 0 && x > 0 {
            return x;
        }
    }
    system_dpi() as u32
}

/// every monitor (screen.rs sorts them)
pub fn monitors() -> Vec<Monitor> {
    dpi_aware();
    let mut hs: Vec<isize> = Vec::new();
    // SAFETY: the callback only pushes into hs, which outlives the call.
    unsafe { EnumDisplayMonitors(0, core::ptr::null(), enum_proc, &mut hs as *mut Vec<isize> as isize) };
    let mut out = Vec::new();
    for mon in hs {
        // SAFETY: a MONITORINFOEXW with its size set; GetMonitorInfoW fills it.
        let mut mi: MonitorInfoEx = unsafe { core::mem::zeroed() };
        mi.mi.size = core::mem::size_of::<MonitorInfoEx>() as u32;
        if unsafe { GetMonitorInfoW(mon, &mut mi as *mut MonitorInfoEx as *mut MonitorInfo) } == 0 {
            continue;
        }
        let n = mi.device.iter().position(|&c| c == 0).unwrap_or(32);
        let name = crate::from_wide(&mi.device[..n]);
        let mut dm = [0u8; DEVMODE_SIZE];
        dm[68..70].copy_from_slice(&(DEVMODE_SIZE as u16).to_le_bytes()); // dmSize
        let mut dev = mi.device;
        dev[31] = 0;
        // SAFETY: a NUL-terminated device name, a DEVMODEW-sized buffer with dmSize set.
        let hz = if unsafe { EnumDisplaySettingsW(dev.as_ptr(), ENUM_CURRENT_SETTINGS, dm.as_mut_ptr()) } != 0 {
            u32::from_le_bytes([dm[DM_FREQ], dm[DM_FREQ + 1], dm[DM_FREQ + 2], dm[DM_FREQ + 3]])
        } else {
            0
        };
        let (r, w) = (mi.mi.monitor, mi.mi.work);
        out.push(Monitor {
            id: name_id(&name),
            name,
            rect: [r.l, r.t, r.r - r.l, r.b - r.t],
            work: [w.l, w.t, w.r - w.l, w.b - w.t],
            dpi: monitor_dpi(mon),
            refresh_mhz: if hz > 1 { hz * 1000 } else { 0 }, // 0 / 1: the hardware default
            primary: mi.mi.flags & MONITORINFOF_PRIMARY != 0,
        });
    }
    out
}

/// (test aid) GetSystemMetrics: the number of monitors and the virtual desktop x, y, w, h
pub fn screen_test_metrics() -> (i32, [i32; 4]) {
    dpi_aware();
    // SAFETY: plain metrics.
    unsafe {
        (
            GetSystemMetrics(SM_CMONITORS),
            [GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)],
        )
    }
}

fn virtual_rect() -> [i32; 4] {
    screen_test_metrics().1
}

/// a memory DC with a top-down 32-bit DIB section of w x h selected: (dc, bitmap, old, bits)
struct Dib {
    dc: isize,
    bmp: isize,
    old: isize,
    bits: *mut u8,
    w: i32,
    h: i32,
}

impl Dib {
    fn new(w: i32, h: i32) -> Option<Dib> {
        let bmi = BitmapInfoHeader { size: 40, w, h: -h, planes: 1, bits: 32, ..Default::default() };
        let mut bits: *mut u8 = core::ptr::null_mut();
        // SAFETY: the screen DC (released again), a valid BITMAPINFOHEADER; on failure nothing
        // is left behind.
        unsafe {
            let sdc = GetDC(0);
            let dc = CreateCompatibleDC(sdc);
            let bmp = CreateDIBSection(sdc, &bmi, 0, &mut bits, 0, 0);
            ReleaseDC(0, sdc);
            if dc == 0 || bmp == 0 || bits.is_null() {
                if bmp != 0 {
                    DeleteObject(bmp);
                }
                if dc != 0 {
                    DeleteDC(dc);
                }
                return None;
            }
            let old = SelectObject(dc, bmp);
            Some(Dib { dc, bmp, old, bits, w, h })
        }
    }

    /// copy the desktop rectangle x, y (w x h of the DIB) into it
    fn grab(&self, x: i32, y: i32) -> bool {
        // SAFETY: the screen DC (released again) into our memory DC.
        unsafe {
            let sdc = GetDC(0);
            let ok = BitBlt(self.dc, 0, 0, self.w, self.h, sdc, x, y, SRCCOPY | CAPTUREBLT) != 0;
            ReleaseDC(0, sdc);
            ok
        }
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: the DIB section holds w * h 32-bit pixels while it lives.
        unsafe { core::slice::from_raw_parts(self.bits, self.w as usize * self.h as usize * 4) }
    }

    /// 0xRRGGBB at x, y (0 outside)
    fn pixel(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return 0;
        }
        let o = (y as usize * self.w as usize + x as usize) * 4;
        let b = self.bytes();
        (b[o + 2] as u32) << 16 | (b[o + 1] as u32) << 8 | b[o] as u32
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        // SAFETY: the objects made in new.
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp);
            DeleteDC(self.dc);
        }
    }
}

/// x, y, w, h of the desktop as B, G, R, x rows (x = 255)
fn grab_rect(r: [i32; 4]) -> Result<Shot, ScreenError> {
    dpi_aware();
    let d = Dib::new(r[2], r[3]).ok_or_else(|| ScreenError::Failed(String::from("no memory for the picture")))?;
    if !d.grab(r[0], r[1]) {
        return Err(ScreenError::Failed(String::from("BitBlt from the screen failed")));
    }
    let mut px = d.bytes().to_vec();
    for p in px.chunks_exact_mut(4) {
        p[3] = 255;
    }
    Ok(Shot { rect: r, w: r[2], h: r[3], px, file: None })
}

pub fn capture_available() -> bool {
    true
}

/// the picture at once (ScreenEvent::Shot)
pub fn capture_start(rect: Option<[i32; 4]>) -> Result<(), ScreenError> {
    let r = rect.unwrap_or_else(|| {
        dpi_aware();
        virtual_rect()
    });
    push_shot(grab_rect(r));
    Ok(())
}

pub fn pick_available() -> bool {
    true
}

// ---------------------------------------------------------------- the pick overlay

struct Overlay {
    hwnd: isize,
    dib: Dib,
    /// the virtual desktop's origin (the window's and the picture's top left)
    vx: i32,
    vy: i32,
    /// the pointer (window px) and the magnifier's rectangle drawn last
    px: i32,
    py: i32,
    mag: Rect,
    zoom: i32,
    /// (tests) the pointer was set by pick_test_move: the system's moves are ignored
    pinned: bool,
}

// SAFETY: the overlay is made, used and destroyed on the UI thread only; the lock guards the
// window procedure's re-entry, the raw pointer of the DIB never leaves that thread.
unsafe impl Send for Overlay {}

static OV: SpinLock<Option<Overlay>> = SpinLock::new(None);
static CLASS: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// a pick: the overlay, or with a test point that pixel at once
pub fn pick_start(test: Option<(i32, i32)>) -> Result<(), ScreenError> {
    dpi_aware();
    if let Some((x, y)) = test {
        let s = grab_rect([x, y, 1, 1])?;
        push_color(crate::screen::shot_pixel(&s, 0, 0).ok_or(ScreenError::Failed(String::from("no pixel"))));
        return Ok(());
    }
    let v = virtual_rect();
    let dib = Dib::new(v[2], v[3]).ok_or_else(|| ScreenError::Failed(String::from("no memory for the picture")))?;
    if !dib.grab(v[0], v[1]) {
        return Err(ScreenError::Failed(String::from("BitBlt from the screen failed")));
    }
    let class: Vec<u16> = "jfgui-pick\0".encode_utf16().collect();
    // SAFETY: the own module; a complete WNDCLASSEXW with a NUL-terminated name kept alive.
    let inst = unsafe { GetModuleHandleW(core::ptr::null()) };
    if !CLASS.swap(true, core::sync::atomic::Ordering::Relaxed) {
        let wc = WndClassEx {
            size: core::mem::size_of::<WndClassEx>() as u32,
            style: 0,
            proc_: pick_proc,
            cls_extra: 0,
            wnd_extra: 0,
            inst,
            icon: 0,
            // SAFETY: a system cursor id.
            cursor: unsafe { LoadCursorW(0, IDC_CROSS) },
            brush: 0,
            menu: core::ptr::null(),
            class: class.as_ptr(),
            icon_sm: 0,
        };
        // SAFETY: see above.
        unsafe { RegisterClassExW(&wc) };
    }
    let mut p = Point::default();
    // SAFETY: an out-parameter.
    unsafe { GetCursorPos(&mut p) };
    let zoom = (12 * system_dpi() / 96).max(8);
    *OV.lock() = Some(Overlay { hwnd: 0, dib, vx: v[0], vy: v[1], px: p.x - v[0], py: p.y - v[1], mag: Rect::default(), zoom, pinned: false });
    // SAFETY: a popup over the virtual desktop; its procedure reads OV (set above).
    let h = unsafe {
        CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW, class.as_ptr(), class.as_ptr(), WS_POPUP | WS_VISIBLE, v[0], v[1], v[2], v[3], 0, 0, inst, 0)
    };
    if h == 0 {
        OV.lock().take();
        return Err(ScreenError::Failed(String::from("no window for the pick")));
    }
    if let Some(o) = OV.lock().as_mut() {
        o.hwnd = h;
    }
    // SAFETY: our window; the app is in the foreground (the user just clicked it).
    unsafe {
        SetForegroundWindow(h);
        SetFocus(h);
    }
    Ok(())
}

// end the pick: the window and the picture go, the answer is queued
fn pick_end(r: Result<u32, ScreenError>) {
    let Some(o) = OV.lock().take() else { return };
    // SAFETY: our window (its WM_DESTROY finds OV empty).
    unsafe { DestroyWindow(o.hwnd) };
    drop(o);
    push_color(r);
}

// the magnifier's rectangle for the pointer at x, y (window px), kept inside w x h
fn mag_rect(x: i32, y: i32, zoom: i32, w: i32, h: i32) -> Rect {
    let side = MAG_CELLS * zoom + 2;
    let gap = MAG_GAP * zoom / 12;
    let tall = side + 22 * zoom / 12;
    let mut l = x + gap;
    let mut t = y + gap;
    if l + side > w {
        l = x - gap - side;
    }
    if t + tall > h {
        t = y - gap - tall;
    }
    Rect { l, t, r: l + side, b: t + tall }
}

fn paint(hwnd: isize) {
    let g = OV.lock();
    let Some(o) = g.as_ref() else { return };
    // SAFETY: a PAINTSTRUCT for our window; GDI calls on its DC and our memory DC (no messages).
    unsafe {
        let mut ps: PaintStruct = core::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let r = ps.rc;
        BitBlt(dc, r.l, r.t, r.r - r.l, r.b - r.t, o.dib.dc, r.l, r.t, SRCCOPY);
        let m = o.mag;
        if m.r > m.l {
            let z = o.zoom;
            let side = MAG_CELLS * z;
            SetStretchBltMode(dc, COLORONCOLOR);
            let half = MAG_CELLS / 2;
            StretchBlt(dc, m.l + 1, m.t + 1, side, side, o.dib.dc, o.px - half, o.py - half, MAG_CELLS, MAG_CELLS, SRCCOPY);
            let black = GetStockObject(BLACK_BRUSH);
            let white = GetStockObject(WHITE_BRUSH);
            FrameRect(dc, &Rect { l: m.l, t: m.t, r: m.l + side + 2, b: m.t + side + 2 }, black);
            // the cell under the pointer: white inside black
            let c = Rect { l: m.l + 1 + half * z, t: m.t + 1 + half * z, r: m.l + 1 + (half + 1) * z, b: m.t + 1 + (half + 1) * z };
            FrameRect(dc, &c, black);
            FrameRect(dc, &Rect { l: c.l + 1, t: c.t + 1, r: c.r - 1, b: c.b - 1 }, white);
            // a bar below: the colour and its hex value
            let rgb = o.dib.pixel(o.px, o.py);
            let bar = Rect { l: m.l, t: m.t + side + 2, r: m.r, b: m.b };
            FillRect(dc, &bar, black);
            let sw = CreateSolidBrush(((rgb & 255) << 16) | (rgb & 0xFF00) | (rgb >> 16)); // COLORREF 0x00BBGGRR
            let sq = bar.b - bar.t - 6;
            FillRect(dc, &Rect { l: bar.l + 3, t: bar.t + 3, r: bar.l + 3 + sq, b: bar.b - 3 }, sw);
            DeleteObject(sw);
            let old = SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, 0x00FF_FFFF);
            let t: Vec<u16> = alloc::format!("#{rgb:06X}").encode_utf16().collect();
            TextOutW(dc, bar.l + 8 + sq, bar.t + 3, t.as_ptr(), t.len() as i32);
            SelectObject(dc, old);
        }
        EndPaint(hwnd, &ps);
    }
}

// the pointer moved to x, y (window px): the old and new magnifier repainted
fn moved(hwnd: isize, x: i32, y: i32) {
    let (old, new) = {
        let mut g = OV.lock();
        let Some(o) = g.as_mut() else { return };
        o.px = x.clamp(0, o.dib.w - 1);
        o.py = y.clamp(0, o.dib.h - 1);
        let old = o.mag;
        o.mag = mag_rect(o.px, o.py, o.zoom, o.dib.w, o.dib.h);
        (old, o.mag)
    };
    // SAFETY: our window; InvalidateRect only marks (WM_PAINT comes later).
    unsafe {
        InvalidateRect(hwnd, &old, 0);
        InvalidateRect(hwnd, &new, 0);
    }
}

// the colour under the pointer
fn current() -> Option<u32> {
    OV.lock().as_ref().map(|o| o.dib.pixel(o.px, o.py))
}

unsafe extern "system" fn pick_proc(hwnd: isize, msg: u32, wp: usize, lp: isize) -> isize {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            return 0;
        }
        WM_ERASEBKGND => return 1,
        WM_SETCURSOR => {
            // SAFETY: a system cursor.
            unsafe { SetCursor(LoadCursorW(0, IDC_CROSS)) };
            return 1;
        }
        WM_MOUSEMOVE => {
            if !OV.lock().as_ref().is_some_and(|o| o.pinned) {
                moved(hwnd, (lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
            }
            return 0;
        }
        WM_LBUTTONDOWN => {
            moved(hwnd, (lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
            if let Some(c) = current() {
                pick_end(Ok(c));
            }
            return 0;
        }
        WM_RBUTTONDOWN => {
            pick_end(Err(ScreenError::Cancelled));
            return 0;
        }
        WM_KEYDOWN => {
            // SAFETY: the key state of this thread's input.
            let step = if unsafe { GetKeyState(VK_SHIFT) } < 0 { 10 } else { 1 };
            let (dx, dy) = match wp {
                0x25 => (-step, 0),
                0x27 => (step, 0),
                0x26 => (0, -step),
                0x28 => (0, step),
                _ => (0, 0),
            };
            if dx | dy != 0 {
                let mut p = Point::default();
                // SAFETY: out-parameter; moving the pointer sends WM_MOUSEMOVE later.
                unsafe {
                    GetCursorPos(&mut p);
                    SetCursorPos(p.x + dx, p.y + dy);
                }
            } else if wp == 0x1B {
                pick_end(Err(ScreenError::Cancelled));
            } else if wp == 0x0D || wp == 0x20 {
                if let Some(c) = current() {
                    pick_end(Ok(c));
                }
            }
            return 0;
        }
        WM_CAPTURECHANGED | WM_DESTROY => {}
        _ => {}
    }
    // SAFETY: the default handling of every other message.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// (test aid) the pick overlay's window, 0 = none (tests close it with Esc through it)
pub fn pick_test_window() -> isize {
    OV.lock().as_ref().map_or(0, |o| o.hwnd)
}

/// (test aid) pointer at the desktop point x, y of the open pick and a click there, as the
/// window would see it -> false without a pick
pub fn pick_test_click(x: i32, y: i32) -> bool {
    let Some((h, vx, vy)) = OV.lock().as_ref().map(|o| (o.hwnd, o.vx, o.vy)) else { return false };
    let lp = (((y - vy) as u32 & 0xFFFF) << 16 | ((x - vx) as u32 & 0xFFFF)) as isize;
    // SAFETY: our window procedure, called directly on the UI thread.
    unsafe { pick_proc(h, WM_LBUTTONDOWN, 1, lp) };
    true
}

/// (test aid) the pointer at the desktop point x, y of the open pick (the magnifier follows on
/// the next WM_PAINT); the real pointer's moves are ignored from then on -> false without a pick
pub fn pick_test_move(x: i32, y: i32) -> bool {
    let Some((h, vx, vy)) = OV.lock().as_mut().map(|o| {
        o.pinned = true;
        (o.hwnd, o.vx, o.vy)
    }) else {
        return false;
    };
    moved(h, x - vx, y - vy);
    true
}

/// (test aid) Esc in the open pick -> false without a pick
pub fn pick_test_cancel() -> bool {
    let Some(h) = OV.lock().as_ref().map(|o| o.hwnd) else { return false };
    // SAFETY: our window procedure, called directly on the UI thread.
    unsafe { pick_proc(h, WM_KEYDOWN, 0x1B, 0) };
    true
}

/// (test aid) window w above every other window (HWND_TOPMOST), shown without activation, so a
/// picture of the screen shows it -> false without the window
pub fn screen_test_topmost(w: i32) -> bool {
    let Some(r) = super::rec(w) else { return false };
    // SAFETY: our window; SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW.
    unsafe { super::SetWindowPos(r.hwnd.get(), -1, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010 | 0x0040) != 0 }
}

