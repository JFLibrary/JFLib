// web_a11y -- the accessibility bridge of the browser and Android hosts (plat::a11y's native
// there): the host keeps a mirror of the tree -- web/a11y.js an ARIA shadow tree in the DOM,
// android/GalleryAccessibility.java the virtual views of an AccessibilityNodeProvider -- and
// gets the differences of every published tree (a11y_wire.rs: the format).
//
//   jfgui_a11y(on) -> 1      (host) the mirror on / off (plat::a11y::active() follows it); on
//                            again: everything anew with the next frame.  jfgui_A11Y=0 refuses.
//   jfgui_a11y_act(win, key_lo, key_hi, act, a, b, value, len) -> 1 queued / 0 refused
//                            an action for node key of window win (plat::a11y::Act): 1 click,
//                            2 focus, 3 value (value), 4 text (the input buffer's len UTF-8
//                            bytes), 5 select (anchor a, caret b in characters)
// The browser gets every update as a request (REQUEST_A11Y, no completion) in frame order.
// Android takes them on its UI thread after a frame:
//   jfgui_a11y_take() -> byte count   the updates since the last call (each: u32 byte count
//                                      + update), copied for jfgui_a11y_data()
//   jfgui_a11y_data() -> address      of that copy, valid until the next take

use alloc::string::String;
use alloc::vec::Vec;

use rt::sync::SpinLock;

use crate::a11y::{self, wire::Mirror, Act, Change, Request, Snapshot, WinTree};

static MIRROR: SpinLock<Mirror> = SpinLock::new(Mirror::new());
#[cfg(target_os = "android")]
static OUT: SpinLock<Vec<u8>> = SpinLock::new(Vec::new());
#[cfg(target_os = "android")]
static TAKEN: SpinLock<Vec<u8>> = SpinLock::new(Vec::new());

/// (plat::a11y) the host turns the mirror on itself
pub fn start(_app: &str) {}

// a window's client area in the canvas / view (px)
fn place(w: &WinTree) -> [i32; 4] {
    let r = if w.pw >= 0 { super::win_get_rect(w.pw) } else { None };
    r.unwrap_or([w.origin[0], w.origin[1], w.size[0], w.size[1]])
}

/// (plat::a11y) a tree was published: its differences to the host's mirror go out
pub fn changed(s: &Snapshot, c: Vec<Change>) {
    let Some(cur) = a11y::current().filter(|a| core::ptr::eq(&**a, s)) else { return };
    let mut m = MIRROR.lock();
    let Some(bytes) = m.update(&cur, &c, &place) else { return };
    #[cfg(target_arch = "wasm32")]
    super::notice(super::REQUEST_A11Y, &bytes);
    #[cfg(target_os = "android")]
    {
        let mut q = OUT.lock();
        let bytes = if q.len() + bytes.len() > 32 << 20 {
            // the host does not take them: start over with everything
            q.clear();
            m.reset();
            let Some(b) = m.update(&cur, &c, &place) else { return };
            b
        } else {
            bytes
        };
        q.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        q.extend_from_slice(&bytes);
    }
}

/// (host) the mirror on (1) or off (0); on again sends everything anew
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_a11y(on: u32) -> u32 {
    if crate::env("jfgui_A11Y").is_some_and(|v| v.starts_with('0')) {
        return 0;
    }
    MIRROR.lock().reset();
    #[cfg(target_os = "android")]
    OUT.lock().clear();
    a11y::set_active(on != 0);
    if on != 0 {
        crate::wake(); // a frame publishes the tree
    }
    1
}

/// (host) an action of the assistive technology for node key_hi:key_lo of window win
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_a11y_act(win: u32, key_lo: u32, key_hi: u32, act: u32, a: i32, b: i32, value: f64, len: u32) -> u32 {
    if win > 255 || !a11y::active() {
        return 0;
    }
    let act = match act {
        1 => Act::Click,
        2 => Act::Focus,
        3 if value.is_finite() => Act::Value(value),
        4 => match super::input(len).map(String::from_utf8) {
            Some(Ok(t)) => Act::Text(t),
            _ => return 0,
        },
        5 => Act::Select(a, b),
        _ => return 0,
    };
    a11y::request(Request { win: win as u8, key: (key_hi as u64) << 32 | key_lo as u64, act });
    1
}

/// (Android) take the updates since the last call -> their byte count (jfgui_a11y_data)
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_a11y_take() -> u32 {
    let mut t = TAKEN.lock();
    *t = core::mem::take(&mut *OUT.lock());
    t.len() as u32
}

/// (Android) the address of the updates taken last
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_a11y_data() -> usize {
    TAKEN.lock().as_ptr() as usize
}
