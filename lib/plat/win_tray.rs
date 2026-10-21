// win_tray -- (tray.rs) the symbol in the notification area and the notifications on Windows:
// Shell_NotifyIconW (shell32, loaded on first use) on a message-only window of the UI thread
//
//   tray_set / tray_remove / notify / notify_close / supported   (tray.rs has the contract)
//
// The window procedure turns the symbol's messages (NOTIFYICON_VERSION_4: LOWORD(lParam) is
// the event, wParam the anchor point) into tray::push events: NIN_SELECT / NIN_KEYSELECT ->
// Activate, WM_LBUTTONDBLCLK -> SecondaryActivate, WM_CONTEXTMENU -> the menu
// (TrackPopupMenuEx, TPM_RETURNCMD) -> Menu(id), NIN_BALLOONUSERCLICK -> NoteClicked,
// NIN_BALLOONTIMEOUT / NIN_BALLOONHIDE -> NoteClosed.  A notification is the symbol's balloon
// (Windows 10 / 11 show it as a toast); without a symbol one is added for the time the
// balloon is shown.  "TaskbarCreated" (Explorer restarted) adds the symbol again.  Call it
// from the UI thread: the window belongs to the thread that first uses it.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use rt::sync::SpinLock;

use super::{
    BitmapInfoHeader, CreateBitmap, CreateDIBSection, CreateIconIndirect, CreateWindowExW, DefWindowProcW, DeleteObject,
    DestroyIcon, GetDC, GetModuleHandleW, IconInfo, LoadIconW, Point, PostMessageW, RegisterClassExW, ReleaseDC,
    WndClassEx, dll_proc,
};
use crate::tray::{self, Note, TI_CHECKED, TI_DISABLED, TI_SEPARATOR, Tray, TrayEvent, TrayIcon, TrayItem};

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreatePopupMenu() -> isize;
    fn AppendMenuW(menu: isize, flags: u32, id: usize, text: *const u16) -> i32;
    fn TrackPopupMenuEx(menu: isize, flags: u32, x: i32, y: i32, wnd: isize, params: usize) -> i32;
    fn DestroyMenu(menu: isize) -> i32;
    fn SetForegroundWindow(h: isize) -> i32;
    fn GetCursorPos(p: *mut Point) -> i32;
    fn RegisterWindowMessageW(name: *const u16) -> u32;
}

/// NOTIFYICONDATAW (x64, 976 bytes)
#[repr(C)]
struct NotifyIconData {
    size: u32,
    wnd: isize,
    id: u32,
    flags: u32,
    callback: u32,
    icon: isize,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    version: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: [u8; 16],
    balloon_icon: isize,
}

const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIM_SETVERSION: u32 = 4;
const NIF_MESSAGE: u32 = 1;
const NIF_ICON: u32 = 2;
const NIF_TIP: u32 = 4;
const NIF_INFO: u32 = 0x10;
const NIF_SHOWTIP: u32 = 0x80;
const NIIF_USER: u32 = 4;
const NIIF_LARGE_ICON: u32 = 0x20;
const WM_TRAY: u32 = 0x8000 + 0x31; // WM_APP + 0x31
const WM_CONTEXTMENU: u32 = 0x7B;
const WM_LBUTTONDBLCLK: u32 = 0x203;
const NIN_SELECT: u32 = 0x400;
const NIN_KEYSELECT: u32 = 0x401;
const NIN_BALLOONHIDE: u32 = 0x403;
const NIN_BALLOONTIMEOUT: u32 = 0x404;
const NIN_BALLOONUSERCLICK: u32 = 0x405;
const MF_STRING: u32 = 0;
const MF_GRAYED: u32 = 1;
const MF_CHECKED: u32 = 8;
const MF_POPUP: u32 = 0x10;
const MF_SEPARATOR: u32 = 0x800;
const TPM_RIGHTBUTTON: u32 = 2;
const TPM_RETURNCMD: u32 = 0x100;
const HWND_MESSAGE: isize = -3;
const ICON_ID: u32 = 1;

struct State {
    wnd: isize,
    /// the symbol was asked for (tray_set) and is on the taskbar
    shown: bool,
    /// the symbol was added only for a notification
    temp: bool,
    tray: Tray,
    /// HICONs this module made (destroyed when replaced)
    icon: isize,
    note_icon: isize,
    /// the notification shown as the balloon
    note: u32,
}

static ST: SpinLock<State> =
    SpinLock::new(State { wnd: 0, shown: false, temp: false, tray: Tray { icon: None, tooltip: String::new(), menu: Vec::new() }, icon: 0, note_icon: 0, note: 0 });
static SHELL: AtomicUsize = AtomicUsize::new(0);
static TASKBAR_MSG: AtomicUsize = AtomicUsize::new(0);

fn shell_notify(msg: u32, d: &NotifyIconData) -> bool {
    let mut p = SHELL.load(Ordering::Relaxed);
    if p == 0 {
        p = dll_proc(b"shell32.dll\0", b"Shell_NotifyIconW\0");
        SHELL.store(p, Ordering::Relaxed);
    }
    if p == 0 {
        return false;
    }
    // SAFETY: Shell_NotifyIconW(DWORD, PNOTIFYICONDATAW) -> BOOL with a complete record.
    let f: unsafe extern "system" fn(u32, *const NotifyIconData) -> i32 = unsafe { core::mem::transmute(p) };
    unsafe { f(msg, d) != 0 }
}

fn wide_into(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for u in s.encode_utf16() {
        if n + 1 >= dst.len() {
            break;
        }
        dst[n] = u;
        n += 1;
    }
    dst[n] = 0;
}

fn data(wnd: isize) -> NotifyIconData {
    NotifyIconData {
        size: core::mem::size_of::<NotifyIconData>() as u32,
        wnd,
        id: ICON_ID,
        flags: 0,
        callback: WM_TRAY,
        icon: 0,
        tip: [0; 128],
        state: 0,
        state_mask: 0,
        info: [0; 256],
        version: 0,
        info_title: [0; 64],
        info_flags: 0,
        guid: [0; 16],
        balloon_icon: 0,
    }
}

// the program's icon (resource 1), else the system's application icon
fn app_icon() -> isize {
    // SAFETY: the own module and a numeric resource name (MAKEINTRESOURCE).
    unsafe {
        let h = LoadIconW(GetModuleHandleW(core::ptr::null()), 1);
        if h != 0 { h } else { LoadIconW(0, 32512) }
    }
}

// an HICON of straight ARGB pixels (0 on failure; also the taskbar's overlay, win_taskbar.rs)
pub(crate) fn make_icon(i: &TrayIcon) -> isize {
    let (w, h) = (i.w as i32, i.h as i32);
    if w <= 0 || h <= 0 || w > 256 || h > 256 || i.px.len() < (w * h) as usize {
        return 0;
    }
    let bmi = BitmapInfoHeader { size: 40, w, h: -h, planes: 1, bits: 32, ..Default::default() };
    let mut bits: *mut u8 = core::ptr::null_mut();
    // SAFETY: a screen DC, released again; a valid BITMAPINFOHEADER (BI_RGB, no colour table).
    let color = unsafe {
        let dc = GetDC(0);
        let c = CreateDIBSection(dc, &bmi, 0, &mut bits, 0, 0);
        ReleaseDC(0, dc);
        c
    };
    if color == 0 || bits.is_null() {
        if color != 0 {
            // SAFETY: the bitmap from above.
            unsafe { DeleteObject(color) };
        }
        return 0;
    }
    // SAFETY: the DIB section holds w * h 32-bit pixels (BGRA in memory = 0xAARRGGBB).
    let dst = unsafe { core::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize) };
    dst.copy_from_slice(&i.px[..(w * h) as usize]);
    let mask_bytes = alloc::vec![0u8; (((w + 15) >> 4) * 2 * h) as usize];
    // SAFETY: word-aligned rows of a w x h monochrome bitmap (all 0: the alpha decides).
    let mask = unsafe { CreateBitmap(w, h, 1, 1, mask_bytes.as_ptr()) };
    let info = IconInfo { icon: 1, hot_x: 0, hot_y: 0, mask, color };
    // SAFETY: both bitmaps are valid; the icon copies them.
    let icon = if mask != 0 { unsafe { CreateIconIndirect(&info) } } else { 0 };
    // SAFETY: the bitmaps from above.
    unsafe {
        DeleteObject(color);
        if mask != 0 {
            DeleteObject(mask);
        }
    }
    icon
}

// the message-only window that receives the symbol's messages (made on first use)
fn window(s: &mut State) -> isize {
    if s.wnd != 0 {
        return s.wnd;
    }
    let class: Vec<u16> = "jfgui-tray\0".encode_utf16().collect();
    // SAFETY: the own module; a complete WNDCLASSEXW; class is NUL-terminated and kept alive.
    unsafe {
        let inst = GetModuleHandleW(core::ptr::null());
        let wc = WndClassEx {
            size: core::mem::size_of::<WndClassEx>() as u32,
            style: 0,
            proc_: tray_proc,
            cls_extra: 0,
            wnd_extra: 0,
            inst,
            icon: 0,
            cursor: 0,
            brush: 0,
            menu: core::ptr::null(),
            class: class.as_ptr(),
            icon_sm: 0,
        };
        RegisterClassExW(&wc);
        s.wnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, HWND_MESSAGE, 0, inst, 0);
        let name: Vec<u16> = "TaskbarCreated\0".encode_utf16().collect();
        TASKBAR_MSG.store(RegisterWindowMessageW(name.as_ptr()) as usize, Ordering::Relaxed);
    }
    s.wnd
}

// add or update the symbol from s.tray
fn apply(s: &mut State, add: bool) -> bool {
    let wnd = window(s);
    if wnd == 0 {
        return false;
    }
    let new_icon = s.tray.icon.as_ref().map_or(0, make_icon);
    if s.icon != 0 {
        // SAFETY: an icon this module made.
        unsafe { DestroyIcon(s.icon) };
    }
    s.icon = new_icon;
    let mut d = data(wnd);
    d.flags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    d.icon = if s.icon != 0 { s.icon } else { app_icon() };
    wide_into(&mut d.tip, &s.tray.tooltip);
    if add {
        if !shell_notify(NIM_ADD, &d) {
            return false;
        }
        d.version = 4; // NOTIFYICON_VERSION_4
        shell_notify(NIM_SETVERSION, &d);
        true
    } else {
        shell_notify(NIM_MODIFY, &d)
    }
}

pub fn tray_set(t: &Tray) -> bool {
    let mut s = ST.lock();
    s.tray = t.clone();
    let add = !s.shown && !s.temp;
    let ok = apply(&mut s, add);
    s.shown = ok;
    s.temp = false;
    ok
}

pub fn tray_remove() {
    let mut s = ST.lock();
    if s.shown && s.note == 0 {
        shell_notify(NIM_DELETE, &data(s.wnd));
    } else if s.shown {
        s.temp = true; // the balloon still needs it: removed when it goes
    }
    s.shown = false;
}

pub fn notify(id: u32, n: &Note) -> bool {
    let mut s = ST.lock();
    if !s.shown && !s.temp && !apply(&mut s, true) {
        return false;
    }
    if !s.shown {
        s.temp = true;
    }
    if s.note_icon != 0 {
        // SAFETY: an icon this module made.
        unsafe { DestroyIcon(s.note_icon) };
        s.note_icon = 0;
    }
    s.note_icon = n.icon.as_ref().map_or(0, make_icon);
    let mut d = data(s.wnd);
    d.flags = NIF_INFO;
    wide_into(&mut d.info_title, &n.title);
    wide_into(&mut d.info, if n.body.is_empty() { " " } else { &n.body });
    d.info_flags = NIIF_USER | if s.note_icon != 0 { NIIF_LARGE_ICON } else { 0 };
    d.balloon_icon = s.note_icon;
    let ok = shell_notify(NIM_MODIFY, &d);
    if ok {
        s.note = id;
    }
    ok
}

pub fn notify_close(id: u32) {
    let mut s = ST.lock();
    if s.note != id || s.wnd == 0 {
        return;
    }
    let mut d = data(s.wnd);
    d.flags = NIF_INFO; // an empty text takes the balloon away
    shell_notify(NIM_MODIFY, &d);
    note_gone(&mut s);
}

pub fn supported() -> bool {
    dll_proc(b"shell32.dll\0", b"Shell_NotifyIconW\0") != 0
}

// the balloon went: a symbol added only for it goes too
fn note_gone(s: &mut State) {
    s.note = 0;
    if s.temp && !s.shown {
        shell_notify(NIM_DELETE, &data(s.wnd));
        s.temp = false;
    }
}

// a native popup menu of the items; ids are 1 + the index in flat
fn build_menu(items: &[TrayItem], flat: &mut Vec<u32>) -> isize {
    // SAFETY: a new, empty popup menu.
    let m = unsafe { CreatePopupMenu() };
    if m == 0 {
        return 0;
    }
    for it in items {
        let text: Vec<u16> = it.label.encode_utf16().chain(core::iter::once(0)).collect();
        // SAFETY: m is our menu, text is NUL-terminated (copied by the system).
        unsafe {
            if it.flags & TI_SEPARATOR != 0 {
                AppendMenuW(m, MF_SEPARATOR, 0, core::ptr::null());
            } else if !it.sub.is_empty() {
                let sub = build_menu(&it.sub, flat);
                let f = MF_POPUP | if it.flags & TI_DISABLED != 0 { MF_GRAYED } else { 0 };
                AppendMenuW(m, f, sub as usize, text.as_ptr());
            } else {
                flat.push(it.id);
                let f = MF_STRING
                    | if it.flags & TI_DISABLED != 0 { MF_GRAYED } else { 0 }
                    | if it.flags & TI_CHECKED != 0 { MF_CHECKED } else { 0 };
                AppendMenuW(m, f, flat.len(), text.as_ptr());
            }
        }
    }
    m
}

fn show_menu(wnd: isize, x: i32, y: i32) {
    let items = ST.lock().tray.menu.clone();
    if items.is_empty() {
        return;
    }
    let mut flat = Vec::new();
    let m = build_menu(&items, &mut flat);
    if m == 0 {
        return;
    }
    // SAFETY: our window and menu; the foreground window lets the menu close on a click
    // elsewhere, the WM_NULL after it is the documented companion.
    let cmd = unsafe {
        SetForegroundWindow(wnd);
        let c = TrackPopupMenuEx(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, wnd, 0);
        PostMessageW(wnd, 0, 0, 0);
        DestroyMenu(m);
        c
    };
    if cmd > 0 {
        if let Some(&id) = flat.get(cmd as usize - 1) {
            tray::push(TrayEvent::Menu(id));
        }
    }
}

unsafe extern "system" fn tray_proc(wnd: isize, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_TRAY {
        let event = (lp as u32) & 0xFFFF;
        match event {
            NIN_SELECT | NIN_KEYSELECT => tray::push(TrayEvent::Activate),
            WM_LBUTTONDBLCLK => tray::push(TrayEvent::SecondaryActivate),
            WM_CONTEXTMENU => {
                let (mut x, mut y) = ((wp & 0xFFFF) as i16 as i32, ((wp >> 16) & 0xFFFF) as i16 as i32);
                let mut p = Point { x: 0, y: 0 };
                // SAFETY: an out-parameter; the cursor is the better anchor when it is known.
                if unsafe { GetCursorPos(&mut p) } != 0 {
                    (x, y) = (p.x, p.y);
                }
                show_menu(wnd, x, y);
            }
            NIN_BALLOONUSERCLICK | NIN_BALLOONTIMEOUT | NIN_BALLOONHIDE => {
                let mut s = ST.lock();
                let id = s.note;
                if id != 0 {
                    note_gone(&mut s);
                    drop(s);
                    tray::push(if event == NIN_BALLOONUSERCLICK { TrayEvent::NoteClicked(id) } else { TrayEvent::NoteClosed(id) });
                }
            }
            _ => {}
        }
        return 0;
    }
    let taskbar = TASKBAR_MSG.load(Ordering::Relaxed);
    if taskbar != 0 && msg as usize == taskbar {
        // Explorer started again: the symbol is gone, add it once more
        let mut s = ST.lock();
        if s.shown {
            apply(&mut s, true);
        }
        return 0;
    }
    // SAFETY: the default handling of every other message.
    unsafe { DefWindowProcW(wnd, msg, wp, lp) }
}
