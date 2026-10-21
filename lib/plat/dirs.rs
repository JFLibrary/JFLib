// dirs -- the user's own folders (documents, downloads, pictures ...) and the folder of logs
//
//   user_dir(DIR_DESKTOP | DIR_DOCUMENTS | DIR_DOWNLOADS | DIR_PICTURES | DIR_MUSIC | DIR_VIDEOS)
//       -> Option<String>
//       Windows: the known folders (SHGetKnownFolderPath; also when the user moved them, for
//       example to OneDrive).  Linux: the XDG user dirs of $XDG_CONFIG_HOME/user-dirs.dirs
//       (XDG_DOCUMENTS_DIR="$HOME/Dokumente": "$HOME/..." or an absolute path, backslash
//       escapes, the last line wins), else $HOME/Desktop, Documents, Downloads, Pictures,
//       Music, Videos.  Browser and Android: "/documents" of the virtual files for
//       DIR_DOCUMENTS, None for the others.
//   log_dir() -> Option<String>   the base folder of logs: Linux $XDG_STATE_HOME (absolute)
//       or $HOME/.local/state; Windows %LOCALAPPDATA% (like cache_dir); the browser /data.
//       vfs::app_dir(APP_LOG) adds the app's own folder (logs://).
//   user_dir_scheme(kind) -> Option<&str>   the vfs scheme of a user folder ("documents" ..)
//
// The paths have '/' separators and no trailing slash, like config_dir; the folders need
// not exist.  user-dirs.dirs is a file of the platform (read unchecked).

use alloc::string::String;

/// user_dir: the desktop
pub const DIR_DESKTOP: u32 = 0;
/// user_dir: the documents
pub const DIR_DOCUMENTS: u32 = 1;
/// user_dir: the downloads of the browser
pub const DIR_DOWNLOADS: u32 = 2;
/// user_dir: pictures and photos
pub const DIR_PICTURES: u32 = 3;
/// user_dir: music
pub const DIR_MUSIC: u32 = 4;
/// user_dir: videos
pub const DIR_VIDEOS: u32 = 5;
/// the number of DIR_* kinds
pub const DIR_COUNT: u32 = 6;

// the vfs schemes of the user folders, in DIR_* order
const SCHEMES: [&str; DIR_COUNT as usize] = ["desktop", "documents", "downloads", "pictures", "music", "videos"];

/// the vfs scheme of a user folder: "desktop", "documents", "downloads", "pictures", "music",
/// "videos" (None for another kind)
pub fn user_dir_scheme(kind: u32) -> Option<&'static str> {
    SCHEMES.get(kind as usize).copied()
}

/// the user's folder of kind DIR_* (None: unknown kind, or none on this system)
pub fn user_dir(kind: u32) -> Option<String> {
    if kind >= DIR_COUNT {
        return None;
    }
    os_user_dir(kind)
}

// "/x/" -> "/x", Windows "C:\\a\\" -> "C:/a" (on Linux '\' is a character of the name); "/"
// and "C:/" stay
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn tidy(s: &str) -> String {
    let mut s: String = if cfg!(windows) { s.replace('\\', "/") } else { String::from(s) };
    while s.len() > 1 && s.ends_with('/') && !(s.len() == 3 && s.as_bytes()[1] == b':') {
        s.pop();
    }
    s
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
fn os_user_dir(kind: u32) -> Option<String> {
    win::known_folder(kind as usize).map(|p| tidy(&p)).filter(|p| !p.is_empty())
}

/// the base folder of logs: %LOCALAPPDATA% (vfs::app_dir(APP_LOG) = <it>/<app>/Logs)
#[cfg(windows)]
pub fn log_dir() -> Option<String> {
    crate::data_dir()
}

#[cfg(windows)]
pub(crate) use win::{KF_PROGRAMS, known_folder};

#[cfg(windows)]
mod win {
    use alloc::string::String;
    use core::sync::atomic::{AtomicUsize, Ordering};

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn LoadLibraryA(name: *const u8) -> isize;
        fn GetProcAddress(m: isize, name: *const u8) -> usize;
    }

    /// a COM GUID
    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);

    /// known_folder: the start menu's "Programs" (the setup's shortcuts)
    pub(crate) const KF_PROGRAMS: usize = 6;

    // FOLDERID_Desktop, _Documents, _Downloads, _Pictures, _Music, _Videos (DIR_* order), _Programs
    static FIDS: [Guid; 7] = [
        Guid(0xB4BF_CC3A, 0xDB2C, 0x424C, [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41]),
        Guid(0xFDD3_9AD0, 0x238F, 0x46AF, [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7]),
        Guid(0x374D_E290, 0x123F, 0x4565, [0x91, 0x64, 0x39, 0xC4, 0x92, 0x5E, 0x46, 0x7B]),
        Guid(0x33E2_8130, 0x4E1E, 0x4676, [0x83, 0x5A, 0x98, 0x39, 0x5C, 0x3B, 0xC3, 0xBB]),
        Guid(0x4BD8_D571, 0x6D19, 0x48D3, [0xBE, 0x97, 0x42, 0x22, 0x20, 0x08, 0x0E, 0x43]),
        Guid(0x1898_9B1D, 0x99B5, 0x455B, [0x84, 0x1C, 0xAB, 0x7C, 0x74, 0xE4, 0xDD, 0xFC]),
        Guid(0xA77F_5D77, 0x2E2B, 0x44C3, [0xA6, 0xA2, 0xAB, 0xA6, 0x01, 0x05, 0x4A, 0x51]),
    ];
    // KF_FLAG_DONT_VERIFY: the configured path, also when the folder is missing
    const KF_FLAG_DONT_VERIFY: u32 = 0x4000;

    // SHGetKnownFolderPath and CoTaskMemFree: 0 = not looked up yet, 1 = missing
    static SHGET: AtomicUsize = AtomicUsize::new(0);
    static COFREE: AtomicUsize = AtomicUsize::new(0);

    fn proc_of(slot: &AtomicUsize, dll: &[u8], name: &[u8]) -> usize {
        let v = slot.load(Ordering::Acquire);
        if v != 0 {
            return if v == 1 { 0 } else { v };
        }
        // SAFETY: NUL-terminated names; both calls return 0 on failure.
        let p = unsafe {
            let m = LoadLibraryA(dll.as_ptr());
            if m == 0 { 0 } else { GetProcAddress(m, name.as_ptr()) }
        };
        slot.store(if p == 0 { 1 } else { p }, Ordering::Release);
        p
    }

    /// the folder k (DIR_* or KF_PROGRAMS) as the shell returns it (backslashes)
    pub(crate) fn known_folder(k: usize) -> Option<String> {
        let fid = FIDS.get(k)?;
        let get = proc_of(&SHGET, b"shell32.dll\0", b"SHGetKnownFolderPath\0");
        if get == 0 {
            return None;
        }
        let mut path: *mut u16 = core::ptr::null_mut();
        // SAFETY: SHGetKnownFolderPath(REFKNOWNFOLDERID, DWORD, HANDLE, PWSTR*) -> HRESULT; no
        // token (the current user).
        let f: unsafe extern "system" fn(*const Guid, u32, isize, *mut *mut u16) -> i32 = unsafe { core::mem::transmute(get) };
        let hr = unsafe { f(fid, KF_FLAG_DONT_VERIFY, 0, &mut path) };
        if path.is_null() {
            return None;
        }
        // SAFETY: the shell returned a NUL-terminated wide string; it is freed below.
        let s = unsafe {
            let mut n = 0;
            while *path.add(n) != 0 {
                n += 1;
            }
            crate::from_wide(core::slice::from_raw_parts(path, n))
        };
        let free = proc_of(&COFREE, b"ole32.dll\0", b"CoTaskMemFree\0");
        if free != 0 {
            // SAFETY: CoTaskMemFree(LPVOID) on the shell's allocation (also after a failure).
            let ff: unsafe extern "system" fn(*mut u16) = unsafe { core::mem::transmute(free) };
            unsafe { ff(path) };
        }
        (hr >= 0 && !s.is_empty()).then_some(s)
    }
}

// ---------------------------------------------------------------- Linux

// the user-dirs.dirs keys and the fallbacks below $HOME, in DIR_* order
#[cfg(target_os = "linux")]
const XDG: [(&str, &str); DIR_COUNT as usize] = [
    ("XDG_DESKTOP_DIR", "Desktop"),
    ("XDG_DOCUMENTS_DIR", "Documents"),
    ("XDG_DOWNLOAD_DIR", "Downloads"),
    ("XDG_PICTURES_DIR", "Pictures"),
    ("XDG_MUSIC_DIR", "Music"),
    ("XDG_VIDEOS_DIR", "Videos"),
];

// $HOME without its trailing slashes ("" for "/"), None when unset or relative
#[cfg(target_os = "linux")]
fn home_base() -> Option<String> {
    let h = crate::env("HOME").filter(|h| h.starts_with('/'))?;
    Some(String::from(h.trim_end_matches('/')))
}

#[cfg(target_os = "linux")]
fn os_user_dir(kind: u32) -> Option<String> {
    let (key, fallback) = XDG[kind as usize];
    let home = home_base();
    if let Some(cfg) = crate::config_dir() {
        if let Some(text) = crate::read_file_raw(&alloc::format!("{cfg}/user-dirs.dirs")) {
            if let Some(v) = user_dirs_value(&String::from_utf8_lossy(&text), key, home.as_deref()) {
                return Some(v);
            }
        }
    }
    home.map(|h| alloc::format!("{h}/{fallback}"))
}

// the folder of key in a user-dirs.dirs text: KEY="$HOME/rel" or KEY="/abs" (the shell that
// sources the file lets the last line win); other values are skipped
#[cfg(target_os = "linux")]
fn user_dirs_value(text: &str, key: &str, home: Option<&str>) -> Option<String> {
    let mut found = None;
    for line in text.lines() {
        let Some(r) = line.trim_start().strip_prefix(key) else { continue };
        let Some(r) = r.trim_start().strip_prefix('=') else { continue };
        let Some(r) = r.trim_start().strip_prefix('"') else { continue };
        let mut v = String::new();
        let (mut esc, mut closed) = (false, false);
        for c in r.chars() {
            match c {
                _ if esc => {
                    v.push(c);
                    esc = false;
                }
                '\\' => esc = true,
                '"' => {
                    closed = true;
                    break;
                }
                _ => v.push(c),
            }
        }
        if !closed {
            continue;
        }
        let p = match v.strip_prefix("$HOME") {
            Some(rel) if rel.is_empty() || rel.starts_with('/') => match home {
                Some(h) => alloc::format!("{h}{rel}"),
                None => continue,
            },
            Some(_) => continue,
            None if v.starts_with('/') => v,
            None => continue,
        };
        let p = tidy(&p);
        found = Some(if p.is_empty() { String::from("/") } else { p });
    }
    found
}

/// the base folder of logs: $XDG_STATE_HOME (absolute), else $HOME/.local/state
/// (vfs::app_dir(APP_LOG) = <it>/<app>)
#[cfg(target_os = "linux")]
pub fn log_dir() -> Option<String> {
    if let Some(v) = crate::env("XDG_STATE_HOME").filter(|v| v.starts_with('/')) {
        return Some(tidy(&v));
    }
    home_base().map(|h| alloc::format!("{h}/.local/state"))
}

// ---------------------------------------------------------------- browser, Android

#[cfg(not(any(windows, target_os = "linux")))]
fn os_user_dir(kind: u32) -> Option<String> {
    // the virtual files have one folder for the user's documents (home())
    if kind == DIR_DOCUMENTS { crate::home() } else { None }
}

/// the base folder of logs: the virtual /data (vfs::app_dir(APP_LOG) = /data/<app>/Logs)
#[cfg(not(any(windows, target_os = "linux")))]
pub fn log_dir() -> Option<String> {
    crate::data_dir()
}
