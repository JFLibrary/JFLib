// sysmenu -- the app's menus in the system's own menu bar (macOS: between the app menu and the Window
// menu, as every Mac app has them; docs/APPLE.md "Menüs in der Menüleiste")
//
//   sysmenu_supported() -> bool        whether this system shows an app's menus in its own bar (macOS
//                                      with a window of the app; elsewhere false: the app draws its menu
//                                      bar in the window)
//   sysmenu_set(&[SysMenuItem])        the app's menus: the top level are the menus of the bar, their
//                                      `sub` the rows; the same list again changes nothing
//   sysmenu_take() -> Vec<u32>         the ids of the rows chosen since the last call (each choice calls
//                                      wake())
//
// A row's key equivalent is a character ("s", or a function key's: "\u{F704}" = F1) with MOD_* bits;
// MOD_CTRL is the Mac's Command key, MOD_ALT its Option key.  A disabled row cannot be chosen, its key
// then reaches the window as a normal key press.

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
    #[cfg(not(target_os = "macos"))]
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
}
