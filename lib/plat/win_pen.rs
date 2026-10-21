// win_pen -- (PN1) pen input on Windows: WM_POINTER* of pen pointers (Windows 8+)
//                                                    [src/plat/win/win_pen.s; PEN_INPUT.md]
//
//   win_pointer(hwnd, msg, wparam, w) -> true handled (the window procedure returns 0), false
//        = on to DefWindowProc.  Called by the window procedure (win_api.rs) for
//        WM_POINTERUPDATE .. WM_POINTERCAPTURECHANGED with the event window = w.
//   pen_devices() -> pens / pen tablets attached (GetPointerDevices; -1 = the API is missing)
//   pen_api_set(Some(api))   (test aid) the pointer API through the given functions instead
//        of user32's (plattest's fakes); None = back to user32 (loaded again on first use)
//
// Only pointers of type PT_PEN are handled.  EnableMouseInPointer is never called, so the
// mouse keeps sending WM_MOUSEMOVE / WM_*BUTTON*; touch and every pointer the API cannot
// describe go on to DefWindowProc, which turns them into mouse messages as before.  A handled
// pen message produces no mouse message.  With pen input off (pen_enable(false)) nothing is
// handled: pens arrive as the system's mouse emulation.
//   WM_POINTERUPDATE   every coalesced sample since the last message (GetPointerPenInfoHistory,
//                      oldest first, at most PEN_HIST) -> E_MOVE with pen data
//   WM_POINTERDOWN     tip contact -> E_DOWN BTN_LEFT, or BTN_RIGHT while the barrel button is
//                      held (PEN_FLAG_BARREL / POINTER_FLAG_SECONDBUTTON), like paint.net
//   WM_POINTERUP       E_UP of the button of the press
//   WM_POINTERLEAVE    E_LEAVE (no button held)    WM_POINTERCAPTURECHANGED  E_UP if held
// Pen data (evq_pen_data): pressure 0..1024 -> 1..PRESSURE_ONE (PEN_MASK_PRESSURE, else none),
// PEN_FLAG_INVERTED / PEN_FLAG_ERASER -> PTR_ERASER, tilt x / y in degrees.  The API's results
// are not trusted: counts are clamped, positions go through ScreenToClient.

use alloc::vec;
use core::cell::Cell;
use core::sync::atomic::{AtomicUsize, Ordering};

use super::{HTCLIENT, Point, SendMessageW, ScreenToClient, UiThread, dll_proc, st, win_push};
use crate::{BTN_LEFT, BTN_RIGHT, E_DOWN, E_LEAVE, E_MOVE, E_UP, PRESSURE_ONE, PTR_ERASER, PTR_PEN, evq_pen_data,
            pen_enabled};

const WIN_PT_PEN: u32 = 3; // POINTER_INPUT_TYPE
pub const PEN_HIST: usize = 16; // samples per WM_POINTERUPDATE
const PPI_SIZE: usize = 120; // POINTER_PEN_INFO (x64)
const PFL_SECONDBUTTON: u32 = 0x20;
const PEN_FLAG_BARREL: u32 = 1;
const PEN_FLAG_INVERTED: u32 = 2;
const PEN_FLAG_ERASER: u32 = 4;
const PEN_MASK_PRESSURE: u32 = 1;
const PEN_MASK_TILT_X: u32 = 4;
const PEN_MASK_TILT_Y: u32 = 8;
const PEN_PRESSURE_MAX: u32 = 1024;
const PDI_SIZE: usize = 1080; // POINTER_DEVICE_INFO (x64)
const PDI_TYPE: usize = 16; // POINTER_DEVICE_TYPE: 1 integrated pen, 2 external pen
const PDI_MAX: u32 = 32;
const WM_POINTERUPDATE: u32 = 0x0245;
const WM_POINTERDOWN: u32 = 0x0246;
const WM_POINTERUP: u32 = 0x0247;
const WM_POINTERLEAVE: u32 = 0x024A;
const WM_POINTERCAPTURECHANGED: u32 = 0x024C;
const WM_SETCURSOR: u32 = 0x0020;
const WM_MOUSEMOVE: u32 = 0x0200;

/// one sample of the pointer API: the POINTER_PEN_INFO fields the pen path reads
#[derive(Clone, Copy, Default)]
pub struct PenInfo {
    pub flags: u32, // POINTER_INFO.pointerFlags
    pub x: i32,     // ptPixelLocation (screen px)
    pub y: i32,
    pub pen_flags: u32, // PEN_FLAG_*
    pub pen_mask: u32,  // PEN_MASK_*
    pub pressure: u32,  // 0..1024
    pub tilt_x: i32,    // -90..90 degrees
    pub tilt_y: i32,
}

/// the pointer API (functions that are missing are None)
#[derive(Clone, Copy)]
pub struct PenApi {
    /// GetPointerType(id) -> the POINTER_INPUT_TYPE (None: failed)
    pub ptype: Option<fn(u32) -> Option<u32>>,
    /// GetPointerPenInfo(id) -> the newest sample
    pub info: Option<fn(u32) -> Option<PenInfo>>,
    /// GetPointerPenInfoHistory(id, &count, out): newest first, at most *count (the room)
    /// records; *count = the samples available -> false: failed
    pub hist: Option<fn(u32, &mut u32, &mut [PenInfo]) -> bool>,
    /// GetPointerDevices(&count, types): *count = the devices; with Some(types) (*count of
    /// them) their POINTER_DEVICE_TYPEs -> false: failed
    pub devs: Option<fn(&mut u32, Option<&mut [u32]>) -> bool>,
}

const NO_API: PenApi = PenApi { ptype: None, info: None, hist: None, devs: None };

struct Pen {
    loaded: Cell<bool>,
    api: Cell<PenApi>,
    btn: Cell<u8>, // BTN_* held by the pen tip (0 = none)
}

static PEN: UiThread<Pen> = UiThread(Pen { loaded: Cell::new(false), api: Cell::new(NO_API), btn: Cell::new(0) });

// user32's functions (0 = missing)
static P_TYPE: AtomicUsize = AtomicUsize::new(0);
static P_INFO: AtomicUsize = AtomicUsize::new(0);
static P_HIST: AtomicUsize = AtomicUsize::new(0);
static P_DEVS: AtomicUsize = AtomicUsize::new(0);

fn pen() -> &'static Pen {
    &PEN.0
}

/// (test aid) the pointer API through `api` (None: user32's again, loaded on first use)
pub fn pen_api_set(api: Option<PenApi>) {
    let p = pen();
    match api {
        Some(a) => {
            p.api.set(a);
            p.loaded.set(true);
        }
        None => p.loaded.set(false),
    }
}

/// the pointer API (user32's loaded on the first call)  [pen_api]
fn pen_api() -> PenApi {
    let p = pen();
    if !p.loaded.get() {
        p.loaded.set(true);
        let u = b"user32.dll\0";
        P_TYPE.store(dll_proc(u, b"GetPointerType\0"), Ordering::Relaxed);
        P_INFO.store(dll_proc(u, b"GetPointerPenInfo\0"), Ordering::Relaxed);
        P_HIST.store(dll_proc(u, b"GetPointerPenInfoHistory\0"), Ordering::Relaxed);
        P_DEVS.store(dll_proc(u, b"GetPointerDevices\0"), Ordering::Relaxed);
        let on = |a: &AtomicUsize| a.load(Ordering::Relaxed) != 0;
        p.api.set(PenApi {
            ptype: if on(&P_TYPE) { Some(real_type) } else { None },
            info: if on(&P_INFO) { Some(real_info) } else { None },
            hist: if on(&P_HIST) { Some(real_hist) } else { None },
            devs: if on(&P_DEVS) { Some(real_devs) } else { None },
        });
    }
    p.api.get()
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// the fields of a POINTER_PEN_INFO record
fn ppi(b: &[u8]) -> PenInfo {
    PenInfo {
        flags: u32_at(b, 12),
        x: u32_at(b, 32) as i32,
        y: u32_at(b, 36) as i32,
        pen_flags: u32_at(b, 96),
        pen_mask: u32_at(b, 100),
        pressure: u32_at(b, 104),
        tilt_x: u32_at(b, 112) as i32,
        tilt_y: u32_at(b, 116) as i32,
    }
}

fn real_type(id: u32) -> Option<u32> {
    let mut t = 0u32;
    // SAFETY: GetPointerType(UINT32, POINTER_INPUT_TYPE*) -> BOOL (loaded: P_TYPE != 0).
    let f: unsafe extern "system" fn(u32, *mut u32) -> i32 = unsafe { core::mem::transmute(P_TYPE.load(Ordering::Relaxed)) };
    if unsafe { f(id, &mut t) } != 0 { Some(t) } else { None }
}

fn real_info(id: u32) -> Option<PenInfo> {
    let mut b = [0u8; PPI_SIZE];
    // SAFETY: GetPointerPenInfo(UINT32, POINTER_PEN_INFO*) -> BOOL; b holds one record.
    let f: unsafe extern "system" fn(u32, *mut u8) -> i32 = unsafe { core::mem::transmute(P_INFO.load(Ordering::Relaxed)) };
    if unsafe { f(id, b.as_mut_ptr()) } != 0 { Some(ppi(&b)) } else { None }
}

fn real_hist(id: u32, count: &mut u32, out: &mut [PenInfo]) -> bool {
    let room = (*count as usize).min(out.len()).min(PEN_HIST);
    let mut b = [0u8; PEN_HIST * PPI_SIZE];
    let mut c = room as u32;
    // SAFETY: GetPointerPenInfoHistory(UINT32, UINT32*, POINTER_PEN_INFO*) -> BOOL; b holds
    // `room` records, the system writes at most *count of them.
    let f: unsafe extern "system" fn(u32, *mut u32, *mut u8) -> i32 =
        unsafe { core::mem::transmute(P_HIST.load(Ordering::Relaxed)) };
    if unsafe { f(id, &mut c, b.as_mut_ptr()) } == 0 {
        return false;
    }
    for (k, o) in out.iter_mut().enumerate().take(room.min(c as usize)) {
        *o = ppi(&b[k * PPI_SIZE..(k + 1) * PPI_SIZE]);
    }
    *count = c;
    true
}

fn real_devs(count: &mut u32, types: Option<&mut [u32]>) -> bool {
    // SAFETY: GetPointerDevices(UINT32*, POINTER_DEVICE_INFO*) -> BOOL.
    let f: unsafe extern "system" fn(*mut u32, *mut u8) -> i32 = unsafe { core::mem::transmute(P_DEVS.load(Ordering::Relaxed)) };
    match types {
        None => unsafe { f(count, core::ptr::null_mut()) != 0 },
        Some(t) => {
            let mut b = vec![0u8; t.len() * PDI_SIZE];
            let mut c = t.len() as u32;
            // SAFETY: b holds c records.
            if unsafe { f(&mut c, b.as_mut_ptr()) } == 0 {
                return false;
            }
            for (k, v) in t.iter_mut().enumerate().take(c as usize) {
                *v = u32_at(&b, k * PDI_SIZE + PDI_TYPE);
            }
            *count = c;
            true
        }
    }
}

/// a pointer message of window w -> handled  [win_pointer]
pub(super) fn win_pointer(hwnd: isize, msg: u32, wp: usize, w: i32) -> bool {
    if !pen_enabled() {
        return false;
    }
    let api = pen_api();
    let Some(ptype) = api.ptype else { return false };
    let id = wp as u16 as u32; // pointer id
    if ptype(id) != Some(WIN_PT_PEN) {
        return false;
    }
    let s = st();
    let p = pen();
    match msg {
        WM_POINTERUPDATE | WM_POINTERDOWN | WM_POINTERUP => {}
        WM_POINTERLEAVE => {
            if p.btn.get() == 0 {
                if s.ptr_win.get() == s.ev_win.get() {
                    s.ptr_win.set(-1);
                }
                win_push(E_LEAVE, 0, 0, 0, 0);
            }
            return true;
        }
        WM_POINTERCAPTURECHANGED => {
            pen_release();
            return true;
        }
        _ => return true, // WM_POINTERENTER (and the rest): nothing
    }
    // the samples: the history (newest first), else the current one
    let mut buf = [PenInfo::default(); PEN_HIST];
    let mut count = 1u32;
    let mut have = false;
    if msg == WM_POINTERUPDATE
        && let Some(hist) = api.hist
    {
        count = PEN_HIST as u32;
        have = hist(id, &mut count, &mut buf);
        if !have {
            count = 1;
        }
    }
    if !have {
        let Some(info) = api.info else { return false };
        let Some(i) = info(id) else { return false };
        buf[0] = i;
        count = 1;
    }
    let n = (count as usize).min(PEN_HIST); // untrusted count: 1 .. PEN_HIST
    if n == 0 {
        return true;
    }
    if msg == WM_POINTERUPDATE {
        // every sample, oldest first
        for k in (0..n).rev() {
            let (x, y, data, _) = sample(hwnd, w, &buf[k]);
            s.ptr_win.set(s.ev_win.get());
            win_push(E_MOVE, 0, x, y, data);
        }
        // no mouse message follows a handled pen message: the window's cursor shape through
        // its own WM_SETCURSOR handler
        // SAFETY: our window.
        unsafe { SendMessageW(hwnd, WM_SETCURSOR, hwnd as usize, HTCLIENT | (WM_MOUSEMOVE as isize) << 16) };
        return true;
    }
    // down / up: the current sample (entry 0)
    let (x, y, data, e) = sample(hwnd, w, &buf[0]);
    if msg == WM_POINTERUP {
        let b = p.btn.get();
        if b == 0 {
            return true;
        }
        p.btn.set(0);
        let m = s.btn_mask.get();
        if m & 1 << (b & 31) == 0 {
            return true; // released already (focus / capture lost)
        }
        s.btn_mask.set(m & !(1 << (b & 31)));
        win_push(E_UP, b, x, y, data);
        return true;
    }
    if p.btn.get() != 0 {
        return true; // already down (a second contact)
    }
    let b = if e.pen_flags & PEN_FLAG_BARREL != 0 || e.flags & PFL_SECONDBUTTON != 0 { BTN_RIGHT } else { BTN_LEFT };
    p.btn.set(b);
    s.ptr_win.set(s.ev_win.get());
    s.btn_mask.set(s.btn_mask.get() | 1 << b);
    s.btn_win.set(s.ev_win.get());
    win_push(E_DOWN, b, x, y, data);
    true
}

/// a sample's client point, its pen data; the window record's last position  [.wpt_sample]
fn sample(hwnd: isize, w: i32, e: &PenInfo) -> (i32, i32, u32, PenInfo) {
    let mut pt = Point { x: e.x, y: e.y };
    // SAFETY: our window and a valid POINT.
    unsafe { ScreenToClient(hwnd, &mut pt) };
    let r = &st().ww[w as usize];
    r.mx.set(pt.x);
    r.my.set(pt.y);
    // pressure 0..1024 -> 0..PRESSURE_ONE, -1 without the pressure axis
    let pressure =
        if e.pen_mask & PEN_MASK_PRESSURE != 0 { (e.pressure.min(PEN_PRESSURE_MAX) * PRESSURE_ONE as u32 >> 10) as i32 } else { -1 };
    let typ = if e.pen_flags & (PEN_FLAG_INVERTED | PEN_FLAG_ERASER) != 0 { PTR_ERASER } else { PTR_PEN };
    let tx = if e.pen_mask & PEN_MASK_TILT_X != 0 { e.tilt_x } else { 0 };
    let ty = if e.pen_mask & PEN_MASK_TILT_Y != 0 { e.tilt_y } else { 0 };
    (pt.x, pt.y, evq_pen_data(pressure, typ, tx, ty), *e)
}

/// the pen lost its capture: E_UP of its button  [pen_release]
fn pen_release() {
    let p = pen();
    let b = p.btn.get();
    if b == 0 {
        return;
    }
    p.btn.set(0);
    let s = st();
    let m = s.btn_mask.get();
    if m & 1 << (b & 31) == 0 {
        return; // released already (WM_CAPTURECHANGED)
    }
    s.btn_mask.set(m & !(1 << (b & 31)));
    win_push(E_UP, b, -1, -1, 0); // at the last position
}

/// pens / pen tablets attached, -1 = no pointer API  [plat_pen_devices]
pub fn pen_devices() -> i32 {
    let Some(devs) = pen_api().devs else { return -1 };
    let mut count = 0u32;
    if !devs(&mut count, None) || count == 0 {
        return 0;
    }
    let n = count.min(PDI_MAX);
    let mut types = vec![0u32; n as usize];
    count = n;
    if !devs(&mut count, Some(&mut types)) {
        return 0;
    }
    // untrusted: at most the buffer; 1 integrated pen, 2 external pen
    types.iter().take(count.min(n) as usize).filter(|&&t| t.wrapping_sub(1) <= 1).count() as i32
}
