// sysmenu -- the app's menus in the system's own menu bar (macOS: between the app menu and the Window
// menu, as every Mac app has them; iPadOS: between the application menu and Window as well;
// docs/APPLE.md "Menüs in der Menüleiste" and AP9)
//
//   sysmenu_supported() -> bool        whether this system shows an app's menus in its own bar (macOS
//                                      with a window of the app, an iPad under UIKit; elsewhere false:
//                                      the app draws its menu bar in the window)
//   sysmenu_window_row() -> bool       whether the window keeps its own menu row as well (iPadOS: its
//                                      bar hides until a swipe down from the top or the pointer there
//                                      reveals it, the row stays the way for the finger; macOS: false)
//   sysmenu_set(&[SysMenuItem])        the app's menus: the top level are the menus of the bar, their
//                                      `sub` the rows; the same list again changes nothing; an empty
//                                      list takes them out again
//   sysmenu_take() -> Vec<u32>         the ids of the rows chosen since the last call (each choice calls
//                                      wake())
//
// A row's key equivalent is a character ("s", or a function key's: "\u{F704}" = F1) with MOD_* bits;
// MOD_CTRL is the Mac's Command key, MOD_ALT its Option key.  A disabled row cannot be chosen, its key
// then reaches the window as a normal key press.  On the iPad the rows with a key also show while
// Command is held; Insert has no key there.

use alloc::string::String;
use alloc::vec::Vec;

use rt::sync::SpinLock;

pub use crate::tray::{TI_CHECKED, TI_DISABLED, TI_SEPARATOR};

/// one menu or row
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SysMenuItem {
    /// reported by sysmenu_take (0: a row without action, e.g. a menu or a submenu)
    pub id: u32,
    pub label: String,
    /// TI_CHECKED, TI_DISABLED, TI_SEPARATOR
    pub flags: u32,
    /// the key equivalent: one character ("" = none)
    pub key: String,
    /// MOD_SHIFT | MOD_CTRL | MOD_ALT of the key equivalent
    pub mods: u32,
    /// a menu or submenu when not empty
    pub sub: Vec<SysMenuItem>,
}

static CHOSEN: SpinLock<Vec<u32>> = SpinLock::new(Vec::new());
static LAST: SpinLock<Vec<SysMenuItem>> = SpinLock::new(Vec::new());
const CHOSEN_MAX: usize = 64;

/// (platform) a row was chosen: queue its id and wake the UI thread
#[allow(dead_code)]
pub(crate) fn push(id: u32) {
    {
        let mut q = CHOSEN.lock();
        if q.len() == CHOSEN_MAX {
            q.remove(0);
        }
        q.push(id);
    }
    crate::wake();
}

/// the rows chosen since the last call, oldest first
pub fn sysmenu_take() -> Vec<u32> {
    core::mem::take(&mut *CHOSEN.lock())
}

/// whether the app's menus go into the system's menu bar here
pub fn sysmenu_supported() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::apple::mac::sysmenu_supported()
    }
    #[cfg(target_os = "ios")]
    {
        crate::apple::ios::pad::sysmenu_supported()
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        false
    }
}

/// whether the window keeps drawing its own menu row besides the system's bar (iPadOS; false where
/// the system's bar is the only place, macOS, and where sysmenu_supported is false)
pub fn sysmenu_window_row() -> bool {
    #[cfg(target_os = "ios")]
    {
        crate::apple::ios::pad::sysmenu_window_row()
    }
    #[cfg(not(target_os = "ios"))]
    {
        false
    }
}

/// put the app's menus into the system's menu bar (an unchanged list does nothing)
pub fn sysmenu_set(menus: &[SysMenuItem]) {
    {
        let mut last = LAST.lock();
        if last.as_slice() == menus {
            return;
        }
        *last = menus.to_vec();
    }
    #[cfg(target_os = "macos")]
    crate::apple::mac::sysmenu_set(menus);
    #[cfg(target_os = "ios")]
    crate::apple::ios::pad::sysmenu_set();
}

/// (iOS: UIKit asks when it builds the main menu) the menus of the last sysmenu_set
#[allow(dead_code)]
pub(crate) fn current() -> Vec<SysMenuItem> {
    LAST.lock().clone()
}
