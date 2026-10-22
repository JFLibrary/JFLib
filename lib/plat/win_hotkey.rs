// win_hotkey -- (hotkey.rs) global shortcuts on Windows: RegisterHotKey on a message-only window
// of the UI thread
//
//   available() -> true          register(id, chord, _) / unregister(id)
//   (test aids) hotkey_test_post(id) posts WM_HOTKEY of a registered id to the window (the path
//   of the window procedure without a key); hotkey_test_input(vk) presses and releases vk with
//   SendInput (the real way through the system: only for keys nobody uses, F13-F24)
//
// The app's ids map to the window's hotkey ids 1..0xBFFF (the range RegisterHotKey allows
// applications).  MOD_NOREPEAT: a held chord reports once.  ERROR_HOTKEY_ALREADY_REGISTERED
// (another program or thread holds the chord) is Conflict.  WM_HOTKEY -> hotkey::push(Pressed).

use alloc::string::String;
use alloc::vec::Vec;

use rt::sync::SpinLock;

use super::{CreateWindowExW, DefWindowProcW, GetModuleHandleW, PostMessageW, RegisterClassExW, WndClassEx};
use crate::hotkey::{self, Chord, HotkeyError, HotkeyEvent};
use crate::{MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER};

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterHotKey(h: isize, id: i32, mods: u32, vk: u32) -> i32;
    fn UnregisterHotKey(h: isize, id: i32) -> i32;
    fn SendInput(n: u32, inputs: *const Input, size: i32) -> u32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
}

/// INPUT with a KEYBDINPUT (x64: 40 bytes, the union sized by MOUSEINPUT)
#[repr(C)]
struct Input {
    typ: u32,
    _pad: u32,
    vk: u16,
    scan: u16,
    flags: u32,
    time: u32,
    _pad2: u32,
    extra: usize,
    _rest: [u8; 8],
}

const HWND_MESSAGE: isize = -3;
const WM_HOTKEY: u32 = 0x0312;
const MOD_NOREPEAT: u32 = 0x4000;
const ERROR_HOTKEY_ALREADY_REGISTERED: u32 = 1409;
const INPUT_KEYBOARD: u32 = 1;
const KEYEVENTF_KEYUP: u32 = 2;
const ID_MAX: i32 = 0xBFFF;

struct State {
    wnd: isize,
    /// (window hotkey id, app id)
    slots: Vec<(i32, u32)>,
    next: i32,
}

static ST: SpinLock<State> = SpinLock::new(State { wnd: 0, slots: Vec::new(), next: 1 });

// the message-only window that receives WM_HOTKEY (made on first use, on the calling thread)
fn window(s: &mut State) -> isize {
    if s.wnd != 0 {
        return s.wnd;
    }
    let class: Vec<u16> = "jfgui-hotkey\0".encode_utf16().collect();
    // SAFETY: the own module; a complete WNDCLASSEXW; class is NUL-terminated and kept alive.
    unsafe {
        let inst = GetModuleHandleW(core::ptr::null());
        let wc = WndClassEx {
            size: core::mem::size_of::<WndClassEx>() as u32,
            style: 0,
            proc_: hotkey_proc,
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
    }
    s.wnd
}

pub fn available() -> bool {
    true
}

fn win_mods(m: u8) -> u32 {
    let mut r = MOD_NOREPEAT;
    if m & MOD_ALT != 0 {
        r |= 1;
    }
    if m & MOD_CTRL != 0 {
        r |= 2;
    }
    if m & MOD_SHIFT != 0 {
        r |= 4;
    }
    if m & MOD_SUPER != 0 {
        r |= 8;
    }
    r
}

pub fn register(id: u32, chord: Chord, _description: &str) -> Result<(), HotkeyError> {
    let mut s = ST.lock();
    let wnd = window(&mut s);
    if wnd == 0 {
        return Err(HotkeyError::Failed(String::from("no window for the shortcuts")));
    }
    // a free window id (they are reused after unregister)
    let mut k = s.next;
    while s.slots.iter().any(|e| e.0 == k) {
        k = if k >= ID_MAX { 1 } else { k + 1 };
    }
    // SAFETY: our window; the error code belongs to this call.
    let (ok, err) = unsafe {
        let ok = RegisterHotKey(wnd, k, win_mods(chord.mods), chord.vk) != 0;
        (ok, if ok { 0 } else { GetLastError() })
    };
    if !ok {
        return Err(if err == ERROR_HOTKEY_ALREADY_REGISTERED {
            HotkeyError::Conflict
        } else {
            HotkeyError::Failed(alloc::format!("RegisterHotKey: error {err}"))
        });
    }
    s.slots.push((k, id));
    s.next = if k >= ID_MAX { 1 } else { k + 1 };
    Ok(())
}

pub fn unregister(id: u32) {
    let mut s = ST.lock();
    let wnd = s.wnd;
    s.slots.retain(|&(k, a)| {
        if a == id {
            // SAFETY: our window and one of its hotkey ids.
            unsafe { UnregisterHotKey(wnd, k) };
        }
        a != id
    });
}

unsafe extern "system" fn hotkey_proc(wnd: isize, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_HOTKEY {
        let app = ST.lock().slots.iter().find(|e| e.0 as usize == wp).map(|e| e.1);
        if let Some(id) = app {
            hotkey::push(HotkeyEvent::Pressed(id));
        }
        return 0;
    }
    // SAFETY: the default handling of every other message.
    unsafe { DefWindowProcW(wnd, msg, wp, lp) }
}

/// (test aid) WM_HOTKEY of the registered id posted to the window -> false when unknown
pub fn hotkey_test_post(id: u32) -> bool {
    let s = ST.lock();
    let Some(&(k, _)) = s.slots.iter().find(|e| e.1 == id) else { return false };
    // SAFETY: our window; the message is handled when the UI thread pumps.
    unsafe { PostMessageW(s.wnd, WM_HOTKEY, k as usize, 0) != 0 }
}

/// (test aid) press and release vk through SendInput (only F13-F24: keys no program uses) ->
/// the events the system took (2 = both)
pub fn hotkey_test_input(vk: u32) -> u32 {
    if !(0x7C..=0x87).contains(&vk) {
        return 0;
    }
    let key = |flags| Input { typ: INPUT_KEYBOARD, _pad: 0, vk: vk as u16, scan: 0, flags, time: 0, _pad2: 0, extra: 0, _rest: [0; 8] };
    let inputs = [key(0), key(KEYEVENTF_KEYUP)];
    // SAFETY: two complete INPUT records of the size given.
    unsafe { SendInput(2, inputs.as_ptr(), core::mem::size_of::<Input>() as i32) }
}
