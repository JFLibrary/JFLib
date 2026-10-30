// tray -- the app's symbol in the notification area (system tray) and system notifications
//
//   tray_set(&Tray) -> bool        show or update the symbol: icon (None = the program's own:
//                                  Windows icon resource 1, Linux the icon named like the app),
//                                  tooltip, menu (TrayItem tree; empty = no menu)
//   tray_remove()                  take the symbol away
//   notify(&Note) -> u32           a system notification -> its id (0: not shown); clicking
//                                  it reports NoteClicked
//   notify_close(id)
//   tray_take() -> Vec<TrayEvent>  what happened since the last call; every event calls wake()
//   tray_supported() -> bool       whether this system shows the symbol: a notification area
//                                  (Linux: a StatusNotifierWatcher runs -- KDE, Xfce, ...; GNOME
//                                  only with the AppIndicator extension); notifications work
//                                  without one
//   set_dry_run(on)                print instead of showing from now on (headless UI runs)
//
// Windows (win_tray.rs): Shell_NotifyIconW on a message-only window of the UI thread, the menu
// with TrackPopupMenuEx, notifications as the symbol's balloon (Windows 10 / 11 show it as a
// toast; without a visible symbol a hidden one is added for the time it is shown).  Linux
// (linux_tray.rs): StatusNotifierItem with a com.canonical.dbusmenu menu and
// org.freedesktop.Notifications, on a session-bus connection of its own thread.  Elsewhere
// nothing is shown.  jfgui_TRAY_DRYRUN=1 prints what would be shown instead (tests).

use alloc::string::String;
use alloc::vec::Vec;

use rt::sync::SpinLock;

/// TrayItem flags: a check mark
pub const TI_CHECKED: u32 = 1;
/// TrayItem flags: greyed out
pub const TI_DISABLED: u32 = 2;
/// TrayItem flags: a separator line (id and label are ignored)
pub const TI_SEPARATOR: u32 = 4;

/// one row of the symbol's menu
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayItem {
    /// reported by TrayEvent::Menu (0: a row without action, e.g. a submenu)
    pub id: u32,
    pub label: String,
    /// TI_*
    pub flags: u32,
    /// a submenu when not empty
    pub sub: Vec<TrayItem>,
}

/// an icon: straight ARGB pixels (0xAARRGGBB), w x h
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayIcon {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u32>,
}

/// the symbol
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tray {
    /// None: the program's own icon
    pub icon: Option<TrayIcon>,
    pub tooltip: String,
    pub menu: Vec<TrayItem>,
}

/// a system notification
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Note {
    pub title: String,
    pub body: String,
    /// None: the program's own icon
    pub icon: Option<TrayIcon>,
}

/// what happened at the symbol or a notification
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// the symbol was clicked (Linux: activated)
    Activate,
    /// the symbol was double-clicked (Windows) / middle-clicked (Linux)
    SecondaryActivate,
    /// a menu row was chosen: its TrayItem::id
    Menu(u32),
    /// a notification was clicked: its id
    NoteClicked(u32),
    /// a notification went away without a click (timeout, closed): its id
    NoteClosed(u32),
}

static EVENTS: SpinLock<Vec<TrayEvent>> = SpinLock::new(Vec::new());
const EVENTS_MAX: usize = 64;

/// (platform) queue an event and wake the UI thread
#[allow(dead_code)]
pub(crate) fn push(e: TrayEvent) {
    {
        let mut q = EVENTS.lock();
        if q.len() == EVENTS_MAX {
            q.remove(0);
        }
        q.push(e);
    }
    crate::wake();
}

/// the events since the last call, oldest first
pub fn tray_take() -> Vec<TrayEvent> {
    core::mem::take(&mut *EVENTS.lock())
}

static DRY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// print what would be shown instead of showing it (the UI's headless runs: scripts, --shot)
pub fn set_dry_run(on: bool) {
    DRY.store(on, core::sync::atomic::Ordering::Relaxed);
}

/// set_dry_run or jfgui_TRAY_DRYRUN=1: print instead of showing
pub(crate) fn dry_run() -> bool {
    DRY.load(core::sync::atomic::Ordering::Relaxed) || crate::env("jfgui_TRAY_DRYRUN").is_some_and(|v| v.starts_with('1'))
}

// the dry run's lines: "tray: ..." on stdout
fn dry(line: &str) {
    crate::write_stdout(alloc::format!("tray: {line}\n").as_bytes());
}

fn menu_text(items: &[TrayItem], out: &mut String) {
    for (k, it) in items.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        if it.flags & TI_SEPARATOR != 0 {
            out.push('-');
            continue;
        }
        out.push_str(&alloc::format!("{}={}", it.id, it.label));
        if it.flags & TI_CHECKED != 0 {
            out.push('*');
        }
        if it.flags & TI_DISABLED != 0 {
            out.push('~');
        }
        if !it.sub.is_empty() {
            out.push('[');
            menu_text(&it.sub, out);
            out.push(']');
        }
    }
}

static NOTE_ID: SpinLock<u32> = SpinLock::new(0);

fn next_note_id() -> u32 {
    let mut n = NOTE_ID.lock();
    *n = n.wrapping_add(1).max(1);
    *n
}

/// show or update the symbol -> false when this system has no notification area
pub fn tray_set(t: &Tray) -> bool {
    if dry_run() {
        let mut m = String::new();
        menu_text(&t.menu, &mut m);
        let icon = t.icon.as_ref().map_or(String::from("app"), |i| alloc::format!("{}x{}", i.w, i.h));
        dry(&alloc::format!("set icon={icon} tooltip={} menu={m}", t.tooltip));
        return true;
    }
    native::tray_set(t)
}

/// take the symbol away
pub fn tray_remove() {
    if dry_run() {
        dry("remove");
        return;
    }
    native::tray_remove()
}

/// a system notification -> its id (0: not shown)
pub fn notify(n: &Note) -> u32 {
    let id = next_note_id();
    if dry_run() {
        dry(&alloc::format!("notify {id} title={} body={}", n.title, n.body));
        return id;
    }
    if native::notify(id, n) { id } else { 0 }
}

/// take a notification away
pub fn notify_close(id: u32) {
    if dry_run() {
        dry(&alloc::format!("close {id}"));
        return;
    }
    native::notify_close(id)
}

/// whether the symbol can be shown here (a notification area; Linux: a StatusNotifierWatcher
/// runs -- GNOME needs the AppIndicator extension); notifications work without one
pub fn tray_supported() -> bool {
    dry_run() || native::supported()
}

#[cfg(windows)]
use crate::win_tray as native;

#[cfg(target_os = "linux")]
use crate::linux_tray as native;

#[cfg(not(any(windows, target_os = "linux")))]
mod native {
    use super::{Note, Tray};
    pub fn tray_set(_t: &Tray) -> bool {
        false
    }
    pub fn tray_remove() {}
    pub fn notify(_id: u32, _n: &Note) -> bool {
        false
    }
    pub fn notify_close(_id: u32) {}
    pub fn supported() -> bool {
        false
    }
}
