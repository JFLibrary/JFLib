// win_popup -- OS popup windows on Windows (U3)                    [src/plat/win/win_popup.s]
//
// Popups (menus, combo lists, tooltips) of a secondary UI window that do not fit into it are
// shown in borderless popup windows owned by that window (WS_POPUP, WS_EX_TOOLWINDOW |
// WS_EX_NOACTIVATE, class style CS_DROPSHADOW, a rounded window region).  They never take the
// keyboard; their pointer input is pushed as events of the OWNER window in owner client
// coordinates, so the UI hit-tests them like the rest of the window.
//
//   popup(slot, owner, rect, frame)
//        show / move / present popup slot `slot` (0 .. PPW_MAX-1): rect = x, y, w, h in px of
//        the owner's client area (popup pixel 0, 0 = owner pixel x, y); the slot keeps a copy
//        of the frame's pixels (WM_PAINT repaints from it).  rect None hides it.
//   popup_anchor(slot, anchor, kind, nat) / popup_placed(slot)   (§6, Wayland's placement) the
//        UI places popups on Windows itself (inside the monitor's work area): nothing to do / None
//   win_popup_wheel(hwnd, sx, sy, data) -> true if hwnd is one of the popups: the wheel event
//        (screen point sx, sy) is pushed for its owner (win_api.rs: Windows sends the wheel to
//        the focus window)
//   win_popup_under(owner) -> true if the pointer is over a shown popup of owner (the owner's
//        WM_MOUSELEAVE is then no leave)

use alloc::vec::Vec;
use core::cell::Cell;

use super::{
    BeginPaint, BitmapInfoHeader, ClientToScreen, CreateWindowExW, DefWindowProcW, DestroyWindow, EndPaint, GetDC,
    LoadCursorW, PaintStruct, Point, RegisterClassExW, ReleaseCapture, ReleaseDC, ScreenToClient, SetCapture,
    SetDIBitsToDevice, SetWindowPos, ShowWindow, TrackMouse, TrackMouseEvent, UiThread, WndClassEx, dip, frame_ok,
    hinst, lp_xy, win_dpi, win_hwnd, win_mods,
};
use crate::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, E_DOWN, E_LEAVE, E_MOVE, E_UP, E_WHEEL, Event, Frame, PPW_MAX, PW_MAX, evq_push};

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetWindowRgn(h: isize, rgn: isize, redraw: i32) -> i32;
    fn GetCursorPos(p: *mut Point) -> i32;
    fn WindowFromPoint(p: Point) -> isize;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateRoundRectRgn(x0: i32, y0: i32, x1: i32, y1: i32, w: i32, h: i32) -> isize;
}

const CS_DROPSHADOW: u32 = 0x0002_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_SHOWWINDOW: u32 = 0x0040;
const SW_HIDE: i32 = 0;
const PQ_RADIUS_DIP: i32 = 8;
const IDC_ARROW: usize = 32512;
const MA_NOACTIVATE: isize = 3;
const TME_LEAVE: u32 = 2;
const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_MBUTTONUP: u32 = 0x0208;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_MOUSEHWHEEL: u32 = 0x020E;
const WM_MOUSELEAVE: u32 = 0x02A3;
static S_PQ_CLASS: [u16; 11] = [0x4A, 0x66, 0x47, 0x75, 0x69, 0x50, 0x6F, 0x70, 0x75, 0x70, 0]; // "JfGuiPopup"

/// one popup slot (PQ_* of the assembly)
struct Pq {
    hwnd: Cell<isize>, // 0 = not created
    owner: Cell<i32>,  // owner platform window (-1 = hidden)
    dx: Cell<i32>,     // popup pixel 0, 0 = owner client pixel dx, dy
    dy: Cell<i32>,
    w: Cell<i32>,
    h: Cell<i32>,
    track: Cell<bool>,     // TrackMouseEvent armed
    px: Cell<Vec<u8>>,     // the pixels shown (empty = none)
    pdim: Cell<(i32, i32, i32)>, // their width, height, stride
    ownhwnd: Cell<isize>,  // HWND of the owner it was created for
}

impl Pq {
    const fn new() -> Pq {
        Pq {
            hwnd: Cell::new(0),
            owner: Cell::new(-1),
            dx: Cell::new(0),
            dy: Cell::new(0),
            w: Cell::new(0),
            h: Cell::new(0),
            track: Cell::new(false),
            px: Cell::new(Vec::new()),
            pdim: Cell::new((0, 0, 0)),
            ownhwnd: Cell::new(0),
        }
    }
}

struct Pqs {
    pq: [Pq; PPW_MAX as usize],
    class: Cell<bool>,
}

static PQ: UiThread<Pqs> = UiThread(Pqs { pq: [const { Pq::new() }; PPW_MAX as usize], class: Cell::new(false) });

fn pqs() -> &'static Pqs {
    &PQ.0
}

/// the window class exists (registered once)  [pq_class]
fn pq_class() -> bool {
    let s = pqs();
    if s.class.get() {
        return true;
    }
    // SAFETY: a valid WNDCLASSEXW; the class name is static.
    let ok = unsafe {
        let wc = WndClassEx {
            size: core::mem::size_of::<WndClassEx>() as u32,
            style: CS_DROPSHADOW,
            proc_: pq_proc,
            cls_extra: 0,
            wnd_extra: 0,
            inst: hinst(),
            icon: 0,
            cursor: LoadCursorW(0, IDC_ARROW),
            brush: 0,
            menu: core::ptr::null(),
            class: S_PQ_CLASS.as_ptr(),
            icon_sm: 0,
        };
        RegisterClassExW(&wc) != 0
    };
    s.class.set(ok);
    ok
}

/// show / move / present popup slot over window owner (rect None hides it)  [plat_popup]
pub fn popup(slot: u32, owner: i32, rect: Option<[i32; 4]>, f: Option<&Frame>) {
    let Some(q) = pqs().pq.get(slot as usize) else { return };
    let Some(rect) = rect else {
        q.owner.set(-1);
        drop(q.px.take());
        let h = q.hwnd.get();
        if h != 0 {
            // SAFETY: our popup window.
            unsafe { ShowWindow(h, SW_HIDE) };
        }
        return;
    };
    let oh = win_hwnd(owner);
    if oh == 0 || !pq_class() {
        return;
    }
    let mut top = false; // new window or hidden until now: on top again
    let mut sized = false;
    // created for another owner: a new window (the owner is fixed at creation)
    if q.hwnd.get() != 0 && q.ownhwnd.get() != oh {
        // SAFETY: our popup window.
        unsafe { DestroyWindow(q.hwnd.get()) };
        q.hwnd.set(0);
    }
    if q.hwnd.get() == 0 {
        // SAFETY: a registered class; no title.
        let h = unsafe {
            CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, S_PQ_CLASS.as_ptr(), core::ptr::null(), WS_POPUP, 0, 0, 1,
                            1, oh, 0, hinst(), 0)
        };
        if h == 0 {
            return;
        }
        q.hwnd.set(h);
        q.ownhwnd.set(oh);
        q.w.set(0);
        q.track.set(false);
        top = true;
    }
    if q.owner.get() == -1 {
        top = true;
    }
    q.owner.set(owner);
    // the pixels (the caller's frame is copied: WM_PAINT repaints from the copy)
    let mut px = q.px.take();
    px.clear();
    match f {
        Some(f) if frame_ok(f) => {
            px.extend_from_slice(&f.px[..f.stride as usize * f.h as usize]);
            q.pdim.set((f.w, f.h, f.stride));
        }
        _ => q.pdim.set((0, 0, 0)),
    }
    q.px.set(px);
    q.dx.set(rect[0]);
    q.dy.set(rect[1]);
    if rect[2] != q.w.get() || rect[3] != q.h.get() {
        sized = true;
        q.w.set(rect[2]);
        q.h.set(rect[3]);
    }
    // owner client point -> screen
    let mut p = Point { x: q.dx.get(), y: q.dy.get() };
    // SAFETY: the owner's window and a valid POINT.
    unsafe { ClientToScreen(oh, &mut p) };
    let h = q.hwnd.get();
    if sized {
        // rounded corners like the flyout surface (8 DIP)
        let e = dip(PQ_RADIUS_DIP * 2, win_dpi(owner)); // ellipse size = 2 r
        // SAFETY: a new region, handed to the window.
        unsafe {
            let rgn = CreateRoundRectRgn(0, 0, q.w.get() + 1, q.h.get() + 1, e, e);
            if rgn != 0 {
                SetWindowRgn(h, rgn, 0); // redrawn below anyway
            }
        }
    }
    // a shown popup keeps its z-order
    let mut fl = SWP_NOACTIVATE | SWP_SHOWWINDOW;
    if !top {
        fl |= SWP_NOZORDER;
    }
    // SAFETY: our popup window (HWND_TOP = 0).
    unsafe {
        SetWindowPos(h, 0, p.x, p.y, q.w.get(), q.h.get(), fl);
        let dc = GetDC(h);
        if dc != 0 {
            pq_blit(q, dc);
            ReleaseDC(h, dc);
        }
    }
}

/// (§6) how the next popup() hangs at its anchor: the UI already placed it on the screen
pub fn popup_anchor(_slot: u32, _anchor: [i32; 4], _kind: u32, _nat: [i32; 2]) {}

/// (§6) the platform never moves a popup on Windows
pub fn popup_placed(_slot: u32) -> Option<[i32; 2]> {
    None
}

/// (test aid) the window of a shown popup slot
pub(super) fn popup_hwnd(slot: u32) -> Option<isize> {
    let q = pqs().pq.get(slot as usize)?;
    if q.hwnd.get() == 0 || q.owner.get() < 0 { None } else { Some(q.hwnd.get()) }
}

/// the whole kept frame into the popup window  [pq_blit]
fn pq_blit(q: &Pq, dc: isize) {
    let px = q.px.take();
    let (w, h, stride) = q.pdim.get();
    if w > 0 && h > 0 && px.len() >= stride as usize * h as usize {
        let bmi = BitmapInfoHeader { size: 40, w: stride / 4, h: -h, planes: 1, bits: 32, ..Default::default() };
        // SAFETY: px holds h rows of stride bytes (checked).
        unsafe { SetDIBitsToDevice(dc, 0, 0, w as u32, h as u32, 0, 0, 0, h as u32, px.as_ptr(), &bmi, 0) };
    }
    q.px.set(px);
}

/// the slot of a popup window  [pq_find]
fn pq_find(hwnd: isize) -> Option<&'static Pq> {
    if hwnd == 0 {
        return None;
    }
    pqs().pq.iter().find(|q| q.hwnd.get() == hwnd)
}

/// an event of the popup's owner (x, y popup client px)  [pq_push]
fn pq_push(q: &Pq, typ: u8, btn: u8, x: i32, y: i32, data: u32) {
    let o = q.owner.get();
    if o < 0 {
        return;
    }
    let win = (o as u32 & (PW_MAX as u32 - 1)) as u8;
    evq_push(Event { typ, btn, mods: win_mods(), win, x: x.wrapping_add(q.dx.get()), y: y.wrapping_add(q.dy.get()), data });
}

/// the window under the pointer (0 = none)  [pq_under]
fn pq_under() -> isize {
    let mut p = Point::default();
    // SAFETY: a valid POINT; a POINT by value.
    unsafe {
        GetCursorPos(&mut p);
        WindowFromPoint(p)
    }
}

/// the pointer is over a shown popup of owner  [win_popup_under]
pub(super) fn win_popup_under(owner: i32) -> bool {
    pq_find(pq_under()).is_some_and(|q| q.owner.get() == owner)
}

/// hwnd is a popup: the wheel event (screen point) is pushed for its owner  [win_popup_wheel]
pub(super) fn win_popup_wheel(hwnd: isize, sx: i32, sy: i32, data: u32) -> bool {
    let Some(q) = pq_find(hwnd) else { return false };
    if q.owner.get() < 0 {
        return false;
    }
    let mut p = Point { x: sx, y: sy };
    // SAFETY: our popup window and a valid POINT.
    unsafe { ScreenToClient(hwnd, &mut p) };
    pq_push(q, E_WHEEL, 0, p.x, p.y, data);
    true
}

unsafe extern "system" fn pq_proc(hwnd: isize, msg: u32, wp: usize, lp: isize) -> isize {
    let q = pq_find(hwnd);
    // SAFETY: default handling of the message.
    let def = || unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    if msg == WM_MOUSEACTIVATE {
        return MA_NOACTIVATE;
    }
    if msg == WM_ERASEBKGND {
        return 1;
    }
    let Some(q) = q else { return def() };
    let (x, y) = lp_xy(lp);
    match msg {
        WM_PAINT => {
            // SAFETY: plain Win32 paint bracket on our popup window.
            let mut ps: PaintStruct = unsafe { core::mem::zeroed() };
            let dc = unsafe { BeginPaint(hwnd, &mut ps) };
            if dc != 0 {
                pq_blit(q, dc);
                unsafe { EndPaint(hwnd, &ps) };
            }
            0
        }
        WM_MOUSEMOVE => {
            if !q.track.get() {
                q.track.set(true);
                let mut t = TrackMouse { size: 24, flags: TME_LEAVE, hwnd, hover: 0 };
                // SAFETY: a valid TRACKMOUSEEVENT.
                unsafe { TrackMouseEvent(&mut t) };
            }
            pq_push(q, E_MOVE, 0, x, y, 0);
            0
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            // SAFETY: our popup window.
            unsafe { SetCapture(hwnd) };
            pq_push(q, E_DOWN, pq_btn(msg), x, y, 0);
            0
        }
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => {
            // SAFETY: no arguments.
            unsafe { ReleaseCapture() };
            pq_push(q, E_UP, pq_btn(msg), x, y, 0);
            0
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let d = (wp >> 16) as u16 as u32;
            let data = if msg == WM_MOUSEWHEEL { d } else { d << 16 };
            let mut p = Point { x, y };
            // SAFETY: our popup window and a valid POINT.
            unsafe { ScreenToClient(hwnd, &mut p) };
            pq_push(q, E_WHEEL, 0, p.x, p.y, data);
            0
        }
        WM_MOUSELEAVE => {
            q.track.set(false);
            // onto the owner or another popup of it: no leave
            let u = pq_under();
            if u != 0 {
                if u == q.ownhwnd.get() {
                    return 0;
                }
                if pq_find(u).is_some_and(|o| o.owner.get() == q.owner.get()) {
                    return 0;
                }
            }
            pq_push(q, E_LEAVE, 0, (-1i32).wrapping_sub(q.dx.get()), (-1i32).wrapping_sub(q.dy.get()), 0);
            0
        }
        _ => def(),
    }
}

fn pq_btn(msg: u32) -> u8 {
    match msg {
        WM_LBUTTONDOWN | WM_LBUTTONUP => BTN_LEFT,
        WM_RBUTTONDOWN | WM_RBUTTONUP => BTN_RIGHT,
        _ => BTN_MIDDLE,
    }
}
