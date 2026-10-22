// hotkey -- global keyboard shortcuts: chords that reach the app while another program has the
// keyboard
//
//   Chord::parse("Ctrl+Alt+K") -> Option<Chord>   modifiers ctrl, alt, shift, super (win, meta,
//                                     logo), one key: a-z 0-9 f1-f24, space enter tab esc
//                                     backspace delete insert home end pageup pagedown left up
//                                     right down print pause num0-num9 numadd numsub nummul
//                                     numdiv numdec minus plus comma period (case ignored)
//   chord.text() -> "Ctrl+Alt+K"      chord.trigger() -> "CTRL+ALT+k" (the XDG shortcuts
//                                     notation of the GlobalShortcuts portal)
//   hotkey_available() -> bool        global shortcuts work here
//   hotkey_register(id, chord, description) -> Result<(), HotkeyError>   id != 0; the same id
//                                     again replaces its chord; needs `input.global` in the
//                                     manifest (else Denied, "PERMDENY input.global")
//   hotkey_unregister(id) -> bool     hotkey_clear()   hotkey_list() -> [(id, chord)]
//   hotkey_take() -> Vec<HotkeyEvent> Pressed(id) when the chord was pressed anywhere (held keys
//                                     do not repeat), Refused(id, why) when the desktop refused
//                                     a registration later (the portal); each one calls wake()
//   set_dry_run(on)                   only record registrations (the UI's headless runs);
//                                     hotkey_test_press(id) delivers a press (test aid)
//
// Call register / unregister from the UI thread: the presses arrive through its event loop.
// Windows (win_hotkey.rs): RegisterHotKey (MOD_NOREPEAT) on a message-only window of the UI
// thread, WM_HOTKEY; a chord another program holds is Conflict.  X11 (linux_x11_screen.rs):
// XGrabKey on the root window for every keycode of the key and every combination of Caps Lock
// and Num Lock, KeyPress on the root window; BadAccess (another client grabbed it) is
// Conflict.  Wayland and Flatpak (linux_hotkey.rs): the GlobalShortcuts portal on a thread of
// its own: a session, BindShortcuts with the description and the preferred trigger -- the
// desktop asks the user, who may choose another chord or none; what is not bound comes as
// Refused(id, Dismissed); Activated -> Pressed.  Browser and Android: Unavailable.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use rt::sync::SpinLock;

use crate::{MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER};

/// a key with modifiers: VK code (the platform's E_KEY_DOWN codes) and MOD_SHIFT | MOD_CTRL |
/// MOD_ALT | MOD_SUPER
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Chord {
    pub vk: u32,
    pub mods: u8,
}

/// why a shortcut was not registered
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyError {
    /// no global shortcuts here (browser, Android, no X server / portal)
    Unavailable,
    /// the app's permissions lack `input.global`
    Denied,
    /// id 0, a key without a name, a description with NUL
    Invalid,
    /// another program (or another id of this app) holds the chord
    Conflict,
    /// the user or the desktop did not bind it (the portal's dialog)
    Dismissed,
    /// the system failed: what it said
    Failed(String),
}

impl core::fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            HotkeyError::Unavailable => f.write_str("global shortcuts are not available here"),
            HotkeyError::Denied => f.write_str("the app's permissions do not allow global shortcuts"),
            HotkeyError::Invalid => f.write_str("invalid shortcut"),
            HotkeyError::Conflict => f.write_str("the shortcut is taken"),
            HotkeyError::Dismissed => f.write_str("the shortcut was not bound"),
            HotkeyError::Failed(s) => write!(f, "global shortcut: {s}"),
        }
    }
}

/// what happened to a registered shortcut
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// its chord was pressed
    Pressed(u32),
    /// the desktop did not bind it after all (the portal answers later)
    Refused(u32, HotkeyError),
}

// key names: (name, VK, XDG keysym name)
static KEYS: &[(&str, u32, &str)] = &[
    ("space", 0x20, "space"),
    ("enter", 0x0D, "Return"),
    ("return", 0x0D, "Return"),
    ("tab", 0x09, "Tab"),
    ("esc", 0x1B, "Escape"),
    ("escape", 0x1B, "Escape"),
    ("backspace", 0x08, "BackSpace"),
    ("delete", 0x2E, "Delete"),
    ("del", 0x2E, "Delete"),
    ("insert", 0x2D, "Insert"),
    ("ins", 0x2D, "Insert"),
    ("home", 0x24, "Home"),
    ("end", 0x23, "End"),
    ("pageup", 0x21, "Page_Up"),
    ("pgup", 0x21, "Page_Up"),
    ("pagedown", 0x22, "Page_Down"),
    ("pgdn", 0x22, "Page_Down"),
    ("left", 0x25, "Left"),
    ("up", 0x26, "Up"),
    ("right", 0x27, "Right"),
    ("down", 0x28, "Down"),
    ("print", 0x2C, "Print"),
    ("pause", 0x13, "Pause"),
    ("numadd", 0x6B, "KP_Add"),
    ("numsub", 0x6D, "KP_Subtract"),
    ("nummul", 0x6A, "KP_Multiply"),
    ("numdiv", 0x6F, "KP_Divide"),
    ("numdec", 0x6E, "KP_Decimal"),
    ("minus", 0xBD, "minus"),
    ("plus", 0xBB, "plus"),
    ("comma", 0xBC, "comma"),
    ("period", 0xBE, "period"),
];

// the display name of a named key (Chord::text)
fn key_display(vk: u32) -> Option<String> {
    let s = match vk {
        0x41..=0x5A | 0x30..=0x39 => return Some(String::from(char::from(vk as u8))),
        0x70..=0x87 => return Some(alloc::format!("F{}", vk - 0x6F)),
        0x60..=0x69 => return Some(alloc::format!("Num{}", vk - 0x60)),
        0x20 => "Space",
        0x0D => "Enter",
        0x09 => "Tab",
        0x1B => "Esc",
        0x08 => "Backspace",
        0x2E => "Delete",
        0x2D => "Insert",
        0x24 => "Home",
        0x23 => "End",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2C => "Print",
        0x13 => "Pause",
        0x6B => "NumAdd",
        0x6D => "NumSub",
        0x6A => "NumMul",
        0x6F => "NumDiv",
        0x6E => "NumDec",
        0xBD => "Minus",
        0xBB => "Plus",
        0xBC => "Comma",
        0xBE => "Period",
        _ => return None,
    };
    Some(String::from(s))
}

impl Chord {
    /// "Ctrl+Alt+K", "super+f12", "Shift+NumAdd" -> the chord (None: no key, an unknown name,
    /// two keys)
    pub fn parse(s: &str) -> Option<Chord> {
        let mut c = Chord::default();
        for part in s.split('+') {
            let p = part.trim().to_ascii_lowercase();
            if p.is_empty() {
                return None;
            }
            let m = match p.as_str() {
                "ctrl" | "control" | "strg" => MOD_CTRL,
                "alt" => MOD_ALT,
                "shift" => MOD_SHIFT,
                "super" | "win" | "meta" | "logo" | "cmd" => MOD_SUPER,
                _ => 0,
            };
            if m != 0 {
                c.mods |= m;
                continue;
            }
            if c.vk != 0 {
                return None; // a second key
            }
            let b = p.as_bytes();
            c.vk = if b.len() == 1 && b[0].is_ascii_alphanumeric() {
                b[0].to_ascii_uppercase() as u32
            } else if let Some(n) = p.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=24).contains(n)) {
                0x6F + n
            } else if let Some(n) = p.strip_prefix("num").and_then(|n| n.parse::<u32>().ok()).filter(|n| *n <= 9) {
                0x60 + n
            } else {
                KEYS.iter().find(|k| k.0 == p).map(|k| k.1)?
            };
        }
        (c.vk != 0).then_some(c)
    }

    /// "Ctrl+Alt+K" (the modifiers in the order Ctrl, Alt, Shift, Super)
    pub fn text(&self) -> String {
        let mut s = String::new();
        for (m, n) in [(MOD_CTRL, "Ctrl+"), (MOD_ALT, "Alt+"), (MOD_SHIFT, "Shift+"), (MOD_SUPER, "Super+")] {
            if self.mods & m != 0 {
                s.push_str(n);
            }
        }
        s.push_str(&key_display(self.vk).unwrap_or_else(|| alloc::format!("0x{:02X}", self.vk)));
        s
    }

    /// the XDG shortcuts notation ("CTRL+ALT+k", "LOGO+F12"): modifiers CTRL ALT SHIFT LOGO,
    /// the key as its xkb keysym name; None for a key without one
    pub fn trigger(&self) -> Option<String> {
        let key = match self.vk {
            0x41..=0x5A => String::from(char::from(self.vk as u8 + 32)),
            0x30..=0x39 => String::from(char::from(self.vk as u8)),
            0x70..=0x87 => alloc::format!("F{}", self.vk - 0x6F),
            0x60..=0x69 => alloc::format!("KP_{}", self.vk - 0x60),
            v => String::from(KEYS.iter().find(|k| k.1 == v)?.2),
        };
        let mut s = String::new();
        for (m, n) in [(MOD_CTRL, "CTRL+"), (MOD_ALT, "ALT+"), (MOD_SHIFT, "SHIFT+"), (MOD_SUPER, "LOGO+")] {
            if self.mods & m != 0 {
                s.push_str(n);
            }
        }
        s.push_str(&key);
        Some(s)
    }
}

// the registrations: id, chord, description
static REG: SpinLock<Vec<(u32, Chord, String)>> = SpinLock::new(Vec::new());
static EVENTS: SpinLock<Vec<HotkeyEvent>> = SpinLock::new(Vec::new());
static DRY: AtomicBool = AtomicBool::new(false);
const EVENTS_MAX: usize = 64;

/// (platform) queue an event and wake the UI thread
#[allow(dead_code)]
pub(crate) fn push(e: HotkeyEvent) {
    {
        let mut q = EVENTS.lock();
        if q.len() == EVENTS_MAX {
            q.remove(0);
        }
        q.push(e);
    }
    crate::wake();
}

/// (platform) a registration the desktop dropped later: forgotten, Refused queued
#[allow(dead_code)]
pub(crate) fn refused(id: u32, why: HotkeyError) {
    REG.lock().retain(|r| r.0 != id);
    push(HotkeyEvent::Refused(id, why));
}

/// (platform) whether id is registered
#[allow(dead_code)]
pub(crate) fn registered(id: u32) -> bool {
    REG.lock().iter().any(|r| r.0 == id)
}

/// what happened since the last call, oldest first
pub fn hotkey_take() -> Vec<HotkeyEvent> {
    core::mem::take(&mut *EVENTS.lock())
}

/// only record registrations from now on (the UI's headless runs: scripts, --shot)
pub fn set_dry_run(on: bool) {
    DRY.store(on, Ordering::Relaxed);
}

fn dry_run() -> bool {
    DRY.load(Ordering::Relaxed)
}

/// whether global shortcuts work here
pub fn hotkey_available() -> bool {
    dry_run() || native::available()
}

/// register chord as the global shortcut id (the same id again replaces it); description says
/// what it does (the portal's dialog shows it)
pub fn hotkey_register(id: u32, chord: Chord, description: &str) -> Result<(), HotkeyError> {
    if id == 0 || chord.vk == 0 || chord.vk > 0xFF || description.contains('\0') || chord.trigger().is_none() && !cfg!(windows) {
        return Err(HotkeyError::Invalid);
    }
    if !crate::policy::input_global_allowed() {
        return Err(HotkeyError::Denied);
    }
    if REG.lock().iter().any(|r| r.0 != id && r.1 == chord) {
        return Err(HotkeyError::Conflict); // another id of this app has it
    }
    if REG.lock().iter().any(|r| r.0 == id) {
        hotkey_unregister(id);
    }
    // recorded first: the portal's thread binds the set it reads
    REG.lock().push((id, chord, String::from(description)));
    if !dry_run() {
        if let Err(e) = native::register(id, chord, description) {
            REG.lock().retain(|r| r.0 != id);
            return Err(e);
        }
    }
    Ok(())
}

/// remove the shortcut id -> false when it was not registered
pub fn hotkey_unregister(id: u32) -> bool {
    let had = {
        let mut r = REG.lock();
        let n = r.len();
        r.retain(|e| e.0 != id);
        r.len() != n
    };
    if had && !dry_run() {
        native::unregister(id);
    }
    had
}

/// remove every shortcut of the app
pub fn hotkey_clear() {
    let ids: Vec<u32> = REG.lock().iter().map(|r| r.0).collect();
    for id in ids {
        hotkey_unregister(id);
    }
}

/// the registered shortcuts: (id, chord)
pub fn hotkey_list() -> Vec<(u32, Chord)> {
    REG.lock().iter().map(|r| (r.0, r.1)).collect()
}

/// (platform) the registrations with their descriptions
#[allow(dead_code)]
pub(crate) fn hotkey_entries() -> Vec<(u32, Chord, String)> {
    REG.lock().clone()
}

/// (test aid) a press of the registered shortcut id, as the system would report it -> false
/// when id is not registered
pub fn hotkey_test_press(id: u32) -> bool {
    if !registered(id) {
        return false;
    }
    push(HotkeyEvent::Pressed(id));
    true
}

#[cfg(windows)]
use crate::win_hotkey as native;

#[cfg(target_os = "linux")]
use crate::linux_hotkey as native;

#[cfg(not(any(windows, target_os = "linux")))]
mod native {
    use super::{Chord, HotkeyError};
    pub fn available() -> bool {
        false
    }
    pub fn register(_id: u32, _c: Chord, _d: &str) -> Result<(), HotkeyError> {
        Err(HotkeyError::Unavailable)
    }
    pub fn unregister(_id: u32) {}
}
