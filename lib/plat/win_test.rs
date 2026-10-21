// win_test -- test aids of the Windows platform layer (plattest --test window): they look at
// and poke our own windows only (no input is injected into the system)
//
//   win_test(op, w, a) -> i64   op:
//     WT_PIXEL      0x00RRGGBB of client pixel a[0], a[1] of window w (GetPixel on its DC; -1
//                   outside / no window)
//     WT_REDRAW     invalidate the whole window and paint it now (WM_PAINT repaints from the
//                   kept copy) -> 1
//     WT_MOVE       move / size the window like the user would (SetWindowPos of the frame, not
//                   win_set_rect): client origin a[0], a[1], client size a[2], a[3] -> 1
//     WT_DPI        a WM_DPICHANGED of dpi a[0] with the suggested rect scaled around the
//                   client origin (as Windows sends it when the window changes monitors) -> 1
//     WT_CURSOR_IS  the current cursor is the one of shape a[0] (CUR_* / custom) -> 1 / 0
//     WT_VISIBLE    window w is visible -> 1 / 0
//     WT_POPUP_RECT popup slot w: its window rect x, y, w, h packed as 4 x i16 (-1: not shown)
//     WT_POPUP_PIXEL popup slot w: 0x00RRGGBB at a[0], a[1]
//     WT_ENABLED    window w takes input -> 1 / 0
//     WT_CARET      the system caret's position x | y << 32 (GetCaretPos)
//     WT_ACTIVE     window w is the active window -> 1 / 0
//     WT_CLIP_ACCESS  0 when the clipboard can be opened, else the error (5: access denied,
//                   e.g. in a sandbox that keeps processes off the clipboard)
//     WT_POPUP_SEND popup slot w: SendMessageW(msg a[0], wParam a[1], lParam = point a[2], a[3])
//   file_dialog_autoclose(ms, name)   the next file_dialog is closed after ms by a timer of the
//                   UI thread: name None = Cancel, Some(name) = the name typed + OK

use alloc::string::String;
use core::cell::RefCell;

use super::{GetDC, Point, ReleaseDC, Rect, SendMessageW, SetWindowPos, UiThread, cur_handle, rec, wide_cap, win_insets};

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RedrawWindow(h: isize, r: *const Rect, rgn: isize, flags: u32) -> i32;
    fn GetCursor() -> isize;
    fn IsWindowVisible(h: isize) -> i32;
    fn IsWindowEnabled(h: isize) -> i32;
    fn GetWindowRect(h: isize, r: *mut Rect) -> i32;
    fn GetCaretPos(p: *mut Point) -> i32;
    fn GetActiveWindow() -> isize;
    fn SetTimer(h: isize, id: usize, ms: u32, f: Option<unsafe extern "system" fn(isize, u32, usize, u32)>) -> usize;
    fn KillTimer(h: isize, id: usize) -> i32;
    fn EnumThreadWindows(tid: u32, f: unsafe extern "system" fn(isize, isize) -> i32, lp: isize) -> i32;
    fn EnumChildWindows(h: isize, f: unsafe extern "system" fn(isize, isize) -> i32, lp: isize) -> i32;
    fn GetClassNameW(h: isize, buf: *mut u16, n: i32) -> i32;
    fn GetDlgCtrlID(h: isize) -> i32;
    fn PostMessageW(h: isize, m: u32, wp: usize, lp: isize) -> i32;
    fn OpenClipboard(h: isize) -> i32;
    fn CloseClipboard() -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetPixel(dc: isize, x: i32, y: i32) -> u32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetLastError() -> u32;
}

pub const WT_PIXEL: u32 = 1;
pub const WT_REDRAW: u32 = 2;
pub const WT_MOVE: u32 = 3;
pub const WT_DPI: u32 = 4;
pub const WT_CURSOR_IS: u32 = 5;
pub const WT_VISIBLE: u32 = 6;
pub const WT_POPUP_RECT: u32 = 7;
pub const WT_POPUP_PIXEL: u32 = 8;
pub const WT_ENABLED: u32 = 9;
pub const WT_CARET: u32 = 10;
pub const WT_ACTIVE: u32 = 11;
pub const WT_CLIP_ACCESS: u32 = 12;
pub const WT_POPUP_SEND: u32 = 13;

const RDW_INVALIDATE: u32 = 1;
const RDW_UPDATENOW: u32 = 0x100;
const CLR_INVALID: u32 = 0xFFFF_FFFF;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const WM_DPICHANGED: u32 = 0x02E0;
const WM_SETTEXT: u32 = 0x000C;
const WM_COMMAND: u32 = 0x0111;
const IDOK: usize = 1;
const IDCANCEL: usize = 2;
const AC_TRIES: u32 = 50; // timer ticks to find the dialog

fn rgb(c: u32) -> i64 {
    if c == CLR_INVALID { -1 } else { (((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF)) as i64 }
}

/// see the file header
pub fn win_test(op: u32, w: i32, a: [i32; 4]) -> i64 {
    match op {
        WT_POPUP_RECT | WT_POPUP_PIXEL => {
            let Some(h) = super::popup::popup_hwnd(w as u32) else { return -1 };
            // SAFETY: our popup window; valid out-parameters / a DC released again.
            unsafe {
                if op == WT_POPUP_RECT {
                    let mut r = Rect::default();
                    if IsWindowVisible(h) == 0 || GetWindowRect(h, &mut r) == 0 {
                        return -1;
                    }
                    return (r.l as u16 as i64) | (r.t as u16 as i64) << 16 | ((r.r - r.l) as u16 as i64) << 32
                        | ((r.b - r.t) as u16 as i64) << 48;
                }
                let dc = GetDC(h);
                let c = GetPixel(dc, a[0], a[1]);
                ReleaseDC(h, dc);
                rgb(c)
            }
        }
        WT_POPUP_SEND => {
            let Some(h) = super::popup::popup_hwnd(w as u32) else { return -1 };
            if !(0x0200..=0x020E).contains(&a[0]) && a[0] != 0x02A3 {
                return -1; // mouse messages only (plain values)
            }
            let lp = ((a[2] as u32 & 0xFFFF) | (a[3] as u32) << 16) as i32 as isize;
            // SAFETY: our popup window; a mouse message without pointers.
            unsafe { SendMessageW(h, a[0] as u32, a[1] as usize, lp) as i64 }
        }
        WT_CARET => {
            let mut p = Point::default();
            // SAFETY: a valid POINT.
            unsafe { GetCaretPos(&mut p) };
            (p.x as u32 as i64) | (p.y as i64) << 32
        }
        WT_CLIP_ACCESS => {
            // SAFETY: plain calls; the clipboard is closed again at once.
            unsafe {
                if OpenClipboard(0) != 0 {
                    CloseClipboard();
                    0
                } else {
                    GetLastError() as i64
                }
            }
        }
        WT_CURSOR_IS => {
            // SAFETY: no arguments.
            (unsafe { GetCursor() } == cur_handle(a[0] as u32)) as i64
        }
        _ => {
            let Some(r) = rec(w) else { return -1 };
            let h = r.hwnd.get();
            // SAFETY: our window; valid out-parameters; the DC is released again.
            unsafe {
                match op {
                    WT_PIXEL => {
                        let dc = GetDC(h);
                        let c = GetPixel(dc, a[0], a[1]);
                        ReleaseDC(h, dc);
                        rgb(c)
                    }
                    WT_REDRAW => (RedrawWindow(h, core::ptr::null(), 0, RDW_INVALIDATE | RDW_UPDATENOW) != 0) as i64,
                    WT_MOVE => {
                        let i = win_insets(h);
                        SetWindowPos(h, 0, a[0] - i[0], a[1] - i[1], a[2] + i[0] + i[2], a[3] + i[1] + i[3],
                                     SWP_NOZORDER | SWP_NOACTIVATE);
                        1
                    }
                    WT_DPI => {
                        let mut wr = Rect::default();
                        GetWindowRect(h, &mut wr);
                        let old = r.dpi.get().max(1);
                        let sc = |v: i32| v * a[0] / old;
                        let q = Rect { l: wr.l, t: wr.t, r: wr.l + sc(wr.r - wr.l), b: wr.t + sc(wr.b - wr.t) };
                        let d = a[0] as usize & 0xFFFF;
                        SendMessageW(h, WM_DPICHANGED, d | d << 16, &q as *const Rect as isize);
                        1
                    }
                    WT_VISIBLE => (IsWindowVisible(h) != 0) as i64,
                    WT_ENABLED => (IsWindowEnabled(h) != 0) as i64,
                    WT_ACTIVE => (GetActiveWindow() == h) as i64,
                    _ => -1,
                }
            }
        }
    }
}

// ---------------------------------------------------------------- file dialog autoclose

struct Ac {
    ms: core::cell::Cell<u32>, // 0 = not armed
    name: RefCell<Option<String>>,
    tries: core::cell::Cell<u32>,
    found: core::cell::Cell<isize>,
}

static AC: UiThread<Ac> = UiThread(Ac {
    ms: core::cell::Cell::new(0),
    name: RefCell::new(None),
    tries: core::cell::Cell::new(0),
    found: core::cell::Cell::new(0),
});

/// (test aid) close the next file_dialog after ms: Cancel (name None) or name + OK
pub fn file_dialog_autoclose(ms: u32, name: Option<&str>) {
    let a = &AC.0;
    a.ms.set(ms.max(1));
    *a.name.borrow_mut() = name.map(String::from);
    a.tries.set(0);
}

/// file_dialog calls this right before the modal call: arm the timer
pub(super) fn autoclose_arm() {
    let a = &AC.0;
    let ms = a.ms.get();
    if ms == 0 {
        return;
    }
    a.ms.set(0);
    // SAFETY: a thread timer of the UI thread; its WM_TIMER is dispatched by the dialog's loop.
    unsafe { SetTimer(0, 0, ms, Some(ac_timer)) };
}

fn is_class(h: isize, want: &str) -> bool {
    let mut b = [0u16; 64];
    // SAFETY: b holds 64 wide chars.
    let n = unsafe { GetClassNameW(h, b.as_mut_ptr(), 64) } as usize;
    want.encode_utf16().eq(b[..n.min(64)].iter().copied())
}

unsafe extern "system" fn find_dlg(h: isize, _lp: isize) -> i32 {
    // SAFETY: a window of this thread.
    if is_class(h, "#32770") && unsafe { IsWindowVisible(h) } != 0 {
        AC.0.found.set(h);
        return 0;
    }
    1
}

unsafe extern "system" fn find_name(h: isize, _lp: isize) -> i32 {
    // the file name box: the Explorer dialog's first combo box edit (id 1001, under a
    // FloatNotifySink), or the classic dialog's cmb13 (1148) / edt1 (1152)
    // SAFETY: a child window of the dialog.
    let id = unsafe { GetDlgCtrlID(h) };
    if (id == 1001 && is_class(h, "Edit")) || id == 1148 || id == 1152 {
        AC.0.found.set(h);
        return 0;
    }
    1
}

unsafe extern "system" fn ac_timer(_h: isize, _m: u32, id: usize, _t: u32) {
    let a = &AC.0;
    a.found.set(0);
    // SAFETY: our own thread's windows; messages only to the dialog of our process.
    unsafe {
        EnumThreadWindows(GetCurrentThreadId(), find_dlg, 0);
        let dlg = a.found.get();
        a.tries.set(a.tries.get() + 1);
        let last = a.tries.get() >= AC_TRIES;
        if dlg == 0 {
            if last {
                KillTimer(0, id);
            }
            return;
        }
        if a.name.borrow().is_none() {
            KillTimer(0, id);
            PostMessageW(dlg, WM_COMMAND, IDCANCEL, 0);
            return;
        }
        // the name box (the dialog fills itself in after it is shown: try again later)
        a.found.set(0);
        EnumChildWindows(dlg, find_name, 0);
        let ed = a.found.get();
        if ed == 0 && !last {
            return;
        }
        KillTimer(0, id);
        if let Some(n) = a.name.borrow_mut().take()
            && ed != 0
        {
            let w = wide_cap(&n, 1040);
            SendMessageW(ed, WM_SETTEXT, 0, w.as_ptr() as isize);
        }
        PostMessageW(dlg, WM_COMMAND, IDOK, 0);
    }
}
