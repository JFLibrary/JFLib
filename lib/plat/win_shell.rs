// win_shell -- the shell and process level: open a folder / a URL, the system theme, the home
// folder, the drives, start this program again
//                       [src/plat/win/win_shell.s, win_files.s (home, drives), plat_win.s
//                        (theme, spawn_self)]
//
//   open_folder(path) -> 0 started, -1 failed (no such path, no shell32, error)
//        path: UTF-8, '/' or '\' separators.  A folder opens in Explorer (ShellExecuteW "open"
//        on the folder); a file is shown selected in its folder (ShellExecuteW "open"
//        explorer.exe with /select,"path").  shell32 and ole32 are loaded at run time (COM
//        initialised once, as for the file dialog).  jfgui_OPEN_DRYRUN=1: nothing is started;
//        the call is printed to stdout and 0 returned (tests).
//   open_url(url) -> 0 / -1   a web page (http / https / mailto ...) in the default
//        application (ShellExecuteW "open" on the URL); the same dry run
//   theme() -> 0 light, 1 dark, -1 unknown (HKCU ... Personalize\AppsUseLightTheme)
//   home() -> %USERPROFILE% (or "C:/")       drives() -> "C:/", "D:/", ...
//   spawn_self(args) -> 0 / -1   this executable again: "exe" "arg1" "arg2" ... (every
//        argument quoted with the Microsoft rules), no inherited handles

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use super::{dll_proc, wide_cap};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn GetLogicalDrives() -> u32;
    fn GetModuleFileNameW(m: isize, buf: *mut u16, n: u32) -> u32;
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: usize, ta: usize, inherit: i32, flags: u32, env: usize,
                      dir: *const u16, si: *const StartupInfo, pi: *mut [isize; 3]) -> i32;
    fn CloseHandle(h: isize) -> i32;
}

/// STARTUPINFOW (x64, 104 bytes)
#[repr(C)]
struct StartupInfo {
    cb: u32,
    rest: [u32; 25],
}

const SH_WPATH: usize = 1040; // wide chars of a path
const SH_WURL: usize = 2048; // wide chars of a URL
const SW_SHOWNORMAL: i32 = 1;
const FILE_ATTR_DIRECTORY: u32 = 0x10;
const PATHW_MAX: usize = 1024; // wide chars of the exe path / one spawn argument
const SPAWN_CMD_MAX: usize = 16384; // wide chars of a spawn_self command line
static S_WOPEN: [u16; 5] = [b'o' as u16, b'p' as u16, b'e' as u16, b'n' as u16, 0];

static SH_STATE: AtomicU32 = AtomicU32::new(0);
static SH_EXEC: AtomicUsize = AtomicUsize::new(0);

/// ShellExecuteW or 0 (shell32 loaded, COM initialised on the first call)  [sh_load]
fn sh_load() -> usize {
    if SH_STATE.swap(1, Ordering::SeqCst) == 0 {
        let co = dll_proc(b"ole32.dll\0", b"CoInitializeEx\0");
        if co != 0 {
            // SAFETY: CoInitializeEx(NULL, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).
            let ci: unsafe extern "system" fn(usize, u32) -> i32 = unsafe { core::mem::transmute(co) };
            unsafe { ci(0, 6) };
        }
        SH_EXEC.store(dll_proc(b"shell32.dll\0", b"ShellExecuteW\0"), Ordering::SeqCst);
    }
    SH_EXEC.load(Ordering::SeqCst)
}

/// ShellExecuteW(no owner, "open", file, params, no directory, SW_SHOWNORMAL) > 32
fn shell_exec(file: &[u16], params: Option<&[u16]>) -> bool {
    let p = sh_load();
    if p == 0 {
        return false;
    }
    // SAFETY: ShellExecuteW(HWND, LPCWSTR, LPCWSTR, LPCWSTR, LPCWSTR, INT) -> HINSTANCE with
    // NUL-terminated strings.
    let f: unsafe extern "system" fn(isize, *const u16, *const u16, *const u16, *const u16, i32) -> isize =
        unsafe { core::mem::transmute(p) };
    let pp = params.map_or(core::ptr::null(), |v| v.as_ptr());
    (unsafe { f(0, S_WOPEN.as_ptr(), file.as_ptr(), pp, core::ptr::null(), SW_SHOWNORMAL) }) > 32
}

/// jfgui_OPEN_DRYRUN starts with '1'
fn dry_run() -> bool {
    crate::env("jfgui_OPEN_DRYRUN").is_some_and(|v| v.starts_with('1'))
}

/// a web page (http / https / mailto ...) in the default application -> 0 / -1
pub fn open_url(url: &str) -> i32 {
    if url.is_empty() || !crate::policy::url_allowed(url) {
        return -1;
    }
    if dry_run() {
        crate::write_stdout(format!("plat_open_url: ShellExecuteW open {url}\n").as_bytes());
        return 0;
    }
    if shell_exec(&wide_cap(url, SH_WURL), None) { 0 } else { -1 }
}

/// show a folder (or a file selected in its folder) in the file manager -> 0 / -1
pub fn open_folder(path: &str) -> i32 {
    if path.is_empty() || !crate::policy::fs_allowed(path, crate::policy::FS_READ) {
        return -1;
    }
    // UTF-8 -> UTF-16 with backslashes
    let mut w = wide_cap(path, SH_WPATH);
    if w.len() <= 1 {
        return -1;
    }
    for c in w.iter_mut() {
        if *c == b'/' as u16 {
            *c = b'\\' as u16;
        }
    }
    // SAFETY: a NUL-terminated path.
    let a = unsafe { GetFileAttributesW(w.as_ptr()) };
    if a == u32::MAX {
        return -1; // no such file or folder
    }
    let is_file = a & FILE_ATTR_DIRECTORY == 0;
    if dry_run() {
        let s = if is_file {
            format!("plat_open_folder: ShellExecuteW open explorer.exe /select,{path}\n")
        } else {
            format!("plat_open_folder: ShellExecuteW open folder {path}\n")
        };
        crate::write_stdout(s.as_bytes());
        return 0;
    }
    let ok = if is_file {
        // parameters: /select,"path"
        let mut param: Vec<u16> = "/select,\"".encode_utf16().collect();
        param.extend_from_slice(&w[..w.len() - 1]);
        param.extend_from_slice(&[b'"' as u16, 0]);
        let exe: Vec<u16> = "explorer.exe\0".encode_utf16().collect();
        shell_exec(&exe, Some(&param))
    } else {
        shell_exec(&w, None)
    };
    if ok { 0 } else { -1 }
}

/// 0 light, 1 dark, -1 unknown (system setting)  [plat_theme]
pub fn theme() -> i32 {
    let p = dll_proc(b"advapi32.dll\0", b"RegGetValueW\0");
    if p == 0 {
        return -1;
    }
    let key = wide_cap("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize", 128);
    let val = wide_cap("AppsUseLightTheme", 32);
    let (mut data, mut size) = (0u32, 4u32);
    // SAFETY: RegGetValueW(HKEY, LPCWSTR, LPCWSTR, DWORD, LPDWORD, PVOID, LPDWORD) -> LSTATUS;
    // HKEY_CURRENT_USER, RRF_RT_REG_DWORD into a u32.
    let f: unsafe extern "system" fn(isize, *const u16, *const u16, u32, *mut u32, *mut u32, *mut u32) -> i32 =
        unsafe { core::mem::transmute(p) };
    let r = unsafe { f(0x8000_0001u32 as i32 as isize, key.as_ptr(), val.as_ptr(), 0x10, core::ptr::null_mut(), &mut data,
                       &mut size) };
    if r != 0 {
        return -1;
    }
    (data == 0) as i32 // AppsUseLightTheme = 0 -> dark
}

// a DWORD of HKEY_CURRENT_USER\key (None: missing)
fn reg_dword(key: &str, val: &str) -> Option<u32> {
    let p = dll_proc(b"advapi32.dll\0", b"RegGetValueW\0");
    if p == 0 {
        return None;
    }
    let key = wide_cap(key, 128);
    let val = wide_cap(val, 64);
    let (mut data, mut size) = (0u32, 4u32);
    // SAFETY: RegGetValueW(HKEY_CURRENT_USER, key, value, RRF_RT_REG_DWORD, NULL, &u32, &4).
    let f: unsafe extern "system" fn(isize, *const u16, *const u16, u32, *mut u32, *mut u32, *mut u32) -> i32 =
        unsafe { core::mem::transmute(p) };
    let r = unsafe { f(0x8000_0001u32 as i32 as isize, key.as_ptr(), val.as_ptr(), 0x10, core::ptr::null_mut(), &mut data,
                       &mut size) };
    (r == 0).then_some(data)
}

/// (settings.rs) the system's settings: theme, accent (DWM), high contrast and its colours,
/// client-area animations, text scale (Accessibility), double-click and caret blink times
pub(crate) fn sys_settings_os(s: &mut crate::SysSettings) {
    let t = theme();
    if t >= 0 {
        s.dark = Some(t == 1);
    }
    if let Some(c) = reg_dword("Software\\Microsoft\\Windows\\DWM", "AccentColor") {
        s.accent = Some((c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16 & 0xFF)); // 0xAABBGGRR
    }
    if let Some(t) = reg_dword("Software\\Microsoft\\Accessibility", "TextScaleFactor") {
        s.text_scale = t.clamp(100, 225);
    }
    let spi = dll_proc(b"user32.dll\0", b"SystemParametersInfoW\0");
    if spi != 0 {
        // SAFETY: SystemParametersInfoW(UINT, UINT, PVOID, UINT) -> BOOL with our buffers.
        let f: unsafe extern "system" fn(u32, u32, *mut u8, u32) -> i32 = unsafe { core::mem::transmute(spi) };
        // HIGHCONTRASTW: cbSize, dwFlags, lpszDefaultScheme (16 bytes on x64)
        let mut hc = [0u32; 4];
        hc[0] = 16;
        if unsafe { f(0x0042, 16, hc.as_mut_ptr() as *mut u8, 0) } != 0 && hc[1] & 1 != 0 {
            s.high_contrast = true; // HCF_HIGHCONTRASTON
            s.contrast_colors = sys_colors();
        }
        let mut on = 1i32;
        // SPI_GETCLIENTAREAANIMATION
        if unsafe { f(0x1042, 0, &mut on as *mut i32 as *mut u8, 0) } != 0 {
            s.reduce_motion = on == 0;
        }
        // font smoothing: SPI_GETFONTSMOOTHING, _TYPE (2 = ClearType), _ORIENTATION (1 = RGB)
        let mut smooth = 1i32;
        if unsafe { f(0x004A, 0, &mut smooth as *mut i32 as *mut u8, 0) } != 0 {
            s.font_aa = Some(smooth != 0);
            let mut ty = 1u32;
            if smooth != 0 && unsafe { f(0x200A, 0, &mut ty as *mut u32 as *mut u8, 0) } != 0 {
                let mut or = 1u32;
                let cleartype = ty == 2;
                if cleartype && unsafe { f(0x2012, 0, &mut or as *mut u32 as *mut u8, 0) } == 0 {
                    or = 1;
                }
                s.font_rgba = Some(if !cleartype {
                    crate::FONT_RGBA_NONE
                } else if or == 0 {
                    crate::FONT_RGBA_BGR
                } else {
                    crate::FONT_RGBA_RGB
                });
            }
        }
    }
    let dbl = dll_proc(b"user32.dll\0", b"GetDoubleClickTime\0");
    if dbl != 0 {
        // SAFETY: GetDoubleClickTime() -> UINT.
        let f: unsafe extern "system" fn() -> u32 = unsafe { core::mem::transmute(dbl) };
        s.double_click_ms = unsafe { f() }.clamp(100, 5000);
    }
    let blink = dll_proc(b"user32.dll\0", b"GetCaretBlinkTime\0");
    if blink != 0 {
        // SAFETY: GetCaretBlinkTime() -> UINT (INFINITE: no blinking).
        let f: unsafe extern "system" fn() -> u32 = unsafe { core::mem::transmute(blink) };
        let ms = unsafe { f() };
        s.caret_blink_ms = if ms == u32::MAX { 0 } else { ms.min(5000) };
    }
}

// the high-contrast scheme's colours (GetSysColor, COLORREF 0x00BBGGRR)
fn sys_colors() -> Option<crate::HcColors> {
    let p = dll_proc(b"user32.dll\0", b"GetSysColor\0");
    if p == 0 {
        return None;
    }
    // SAFETY: GetSysColor(int) -> DWORD.
    let f: unsafe extern "system" fn(i32) -> u32 = unsafe { core::mem::transmute(p) };
    let c = |i: i32| {
        let v = unsafe { f(i) };
        (v & 0xFF) << 16 | (v & 0xFF00) | (v >> 16 & 0xFF)
    };
    Some(crate::HcColors {
        bg: c(5),               // COLOR_WINDOW
        text: c(8),             // COLOR_WINDOWTEXT
        highlight: c(13),       // COLOR_HIGHLIGHT
        highlight_text: c(14),  // COLOR_HIGHLIGHTTEXT
        button: c(15),          // COLOR_BTNFACE
        button_text: c(18),     // COLOR_BTNTEXT
        disabled: c(17),        // COLOR_GRAYTEXT
        link: c(26),            // COLOR_HOTLIGHT
    })
}

/// the user's home folder (%USERPROFILE%, else "C:/")  [plat_home]
pub fn home() -> Option<String> {
    Some(crate::env("USERPROFILE").unwrap_or_else(|| String::from("C:/")))
}

/// the drive roots ("C:/", "D:/", ...)  [plat_drives]
pub fn drives() -> Vec<String> {
    // SAFETY: no arguments.
    let m = unsafe { GetLogicalDrives() };
    (0..26u8).filter(|&k| m & 1 << k != 0).map(|k| format!("{}:/", (b'A' + k) as char)).collect()
}

/// like spawn_self, but the program file now at this process's path (after an update a new
/// file): the loader keeps the path the program was started from, also when the running exe
/// was renamed since -> 0 / -1
pub fn restart_self(args: &[&str]) -> i32 {
    spawn_self(args)
}

/// start this executable again with the arguments (no inherited handles) -> 0 / -1
pub fn spawn_self(args: &[&str]) -> i32 {
    let mut exe = alloc::vec![0u16; PATHW_MAX];
    // SAFETY: exe holds PATHW_MAX wide chars.
    let n = unsafe { GetModuleFileNameW(0, exe.as_mut_ptr(), PATHW_MAX as u32) } as usize;
    if n == 0 || n >= PATHW_MAX {
        return -1;
    }
    // command line: the exe path quoted, then the arguments (n backslashes + quote -> 2n + 1
    // backslashes + quote, backslashes before the closing quote doubled)
    let room = SPAWN_CMD_MAX - 4; // (a closing quote + NUL always fit)
    let mut cmd: Vec<u16> = Vec::with_capacity(n + 64);
    cmd.push(b'"' as u16);
    for &c in &exe[..n] {
        if cmd.len() >= room {
            return -1;
        }
        cmd.push(c);
    }
    cmd.push(b'"' as u16);
    for a in args {
        let wa = wide_cap(a, PATHW_MAX);
        if cmd.len() >= room {
            return -1; // no room
        }
        cmd.push(b' ' as u16);
        cmd.push(b'"' as u16);
        let mut bs = 0; // backslashes seen
        for &c in &wa[..wa.len() - 1] {
            if c == b'\\' as u16 {
                bs += 1;
            } else {
                if c == b'"' as u16 {
                    for _ in 0..bs + 1 {
                        if cmd.len() >= room {
                            return -1;
                        }
                        cmd.push(b'\\' as u16);
                    }
                }
                bs = 0;
            }
            if cmd.len() >= room {
                return -1;
            }
            cmd.push(c);
        }
        for _ in 0..bs {
            if cmd.len() >= room {
                return -1;
            }
            cmd.push(b'\\' as u16);
        }
        if cmd.len() >= room {
            return -1;
        }
        cmd.push(b'"' as u16);
    }
    cmd.push(0);
    let mut exe0 = exe[..n].to_vec();
    exe0.push(0);
    let si = StartupInfo { cb: 104, rest: [0; 25] };
    let mut pi = [0isize; 3]; // hProcess, hThread, ids
    // SAFETY: NUL-terminated exe path and (writable) command line, a valid STARTUPINFOW and
    // PROCESS_INFORMATION (24 bytes); our environment and directory.
    let ok = unsafe { CreateProcessW(exe0.as_ptr(), cmd.as_mut_ptr(), 0, 0, 0, 0, 0, core::ptr::null(), &si, &mut pi) };
    if ok == 0 {
        return -1;
    }
    // SAFETY: the handles of the new process.
    unsafe {
        CloseHandle(pi[0]);
        CloseHandle(pi[1]);
    }
    0
}
