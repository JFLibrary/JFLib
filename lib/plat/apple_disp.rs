// apple_disp -- the window API on macOS and iOS before there are windows (docs/APPLE.md AP4)
//
// Every function answers as linux_disp.rs does without a display: no window opens, sizes are 0, the
// dpi is 96, the clipboard is empty, wait() sleeps until its deadline or a wake().  The main window
// ends the process with exit code 1 and a message, as on Linux without Wayland or X11.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::Frame;

/// see win_api.rs (Windows) for the semantics of every function
pub fn win_create(_title: &str, _w_dip: i32, _h_dip: i32, _flags: u32, owner: i32, _x: i32, _y: i32) -> i32 {
    if owner != crate::PW_NOOWNER {
        return -1; // an owned window before any display
    }
    crate::write_stderr(b"error: no windows on macOS and iOS yet (docs/APPLE.md, AP4; use --shot FILE.png for a headless render)\n");
    crate::exit(1)
}

pub fn win_destroy(_w: i32) {}
pub fn win_show(_w: i32, _mode: u32) {}
pub fn win_activate(_w: i32) {}
pub fn win_enable(_w: i32, _on: bool) {}
pub fn win_set_rect(_w: i32, _x: i32, _y: i32, _cw: i32, _ch: i32) {}
pub fn win_get_rect(_w: i32) -> Option<[i32; 4]> {
    None
}
pub fn win_work_area(_w: i32) -> Option<[i32; 4]> {
    None
}
pub fn win_set_title(_w: i32, _title: &str) {}
pub fn win_state(_w: i32) -> u32 {
    0
}
pub fn win_cmd(_w: i32, _cmd: u32) {}
pub fn win_begin_move(_w: i32) {}
pub fn win_begin_resize(_w: i32, _edges: u32) {}
pub fn win_show_menu(_w: i32, _x: i32, _y: i32) {}
pub fn win_size(_w: i32) -> (i32, i32) {
    (0, 0)
}
pub fn win_dpi(_w: i32) -> i32 {
    96
}
pub fn win_set_min_size(_w: i32, _w_dip: i32, _h_dip: i32) {}
pub fn win_present(_w: i32, _f: &Frame, _rects: Option<&[[i32; 4]]>) {}
pub fn win_set_cursor(_w: i32, _shape: u32) {}
pub fn win_hit_hook(_f: Option<fn(i32, i32, i32) -> u32>) {}
pub fn win_move_hook(_f: Option<fn(i32, &mut [i32; 4])>) {}
pub fn set_refresh(_f: Option<fn()>) {}

/// without a display: sleep until the deadline (0 = poll, u64::MAX = forever) or a wake()
pub fn wait(deadline_us: u64) -> u32 {
    if crate::evq_count() == 0 {
        crate::wait_events(deadline_us, &mut |e| {
            crate::evq_push(*e);
        });
    }
    crate::evq_count()
}

pub fn popup(_slot: u32, _owner: i32, _rect: Option<[i32; 4]>, _f: Option<&Frame>) {}
pub fn popup_anchor(_slot: u32, _anchor: [i32; 4], _kind: u32, _nat: [i32; 2]) {}
pub fn popup_placed(_slot: u32) -> Option<[i32; 2]> {
    None
}
pub fn cursor_create(_px: &[u8], _w: i32, _h: i32, _hot_x: i32, _hot_y: i32, _dpi: i32) -> i32 {
    -1
}
pub fn cursor_destroy(_id: i32) {}
/// the dpi to draw a cursor for a window of this dpi (no display: the dpi itself)
pub fn cursor_dpi(dpi: i32) -> i32 {
    dpi
}
pub fn clip_set(_fmt: u32, _data: &[u8]) -> bool {
    false
}
pub fn clip_set_many(_entries: &[(u32, &[u8])]) -> bool {
    false
}
pub fn clip_get(_fmt: u32) -> Option<Vec<u8>> {
    None
}
pub fn clip_has(_fmt: u32) -> bool {
    false
}
pub fn ime_pos(_x: i32, _y: i32, _h: i32) {}

/// (dnd.rs) dragging data out of the app: not without windows
pub(crate) fn os_drag_start(_data: crate::DragData) -> bool {
    false
}

/// 0 light, 1 dark, -1 unknown: jfgui_THEME=light|dark, else unknown (the system's appearance
/// comes with the window backend)
pub fn theme() -> i32 {
    match crate::env("jfgui_THEME").as_deref() {
        Some("dark") => 1,
        Some("light") => 0,
        _ => -1,
    }
}

/// pens / pen tablets (-1 = unknown)
pub fn pen_devices() -> i32 {
    -1
}

/// $HOME (or "/")  [plat_home]
pub fn home() -> Option<String> {
    Some(crate::env("HOME").unwrap_or_else(|| String::from("/")))
}

/// the root "/"  [plat_drives]
pub fn drives() -> Vec<String> {
    vec![String::from("/")]
}

/// printing as linux_print.rs checks it: the manifest, a job without pixels (ERR_DIMS), the test
/// variant jfgui_PRINT_TO=FILE (the PDF into a file); the system's print dialog comes with AP7, until
/// then ERR_UNSUPPORTED
pub fn print(job: &mut crate::PrintJob<'_>) -> i32 {
    job.copies = 0;
    if !crate::policy::print_allowed() {
        return crate::ERR_UNSUPPORTED;
    }
    if job.src.is_none() {
        return crate::ERR_DIMS;
    }
    if let Some(to) = crate::env("jfgui_PRINT_TO").filter(|v| !v.is_empty()) {
        if crate::env("jfgui_PRINT_SMASK").as_deref() == Some("1") {
            job.flags |= crate::PJF_SMASK; // test: alpha as /SMask instead of white
        }
        let page = crate::page_init(job.paper, job.flags);
        let pdf = match crate::print_pdf(job, &page) {
            Ok(p) => p,
            Err(e) => return e,
        };
        if !crate::write_file_raw(&to, &pdf) {
            return crate::ERR_IO;
        }
        job.copies = 1;
        return 0;
    }
    crate::ERR_UNSUPPORTED
}

/// the system's file dialog: not before the window backend (AP4) -> (-1, 0): the library's own
/// dialog, as in a browser without the File System Access API  [plat_file_dialog]
pub fn file_dialog(_flags: u32, _path: &mut String, _exts: &[&str]) -> (i32, u32) {
    (-1, 0)
}

/// macOS: /usr/bin/open with the argument (LaunchServices), waited for up to 5 s -> 0 / -1; iOS: -1
fn open_with_launch_services(arg: &str) -> i32 {
    if cfg!(target_os = "ios") || arg.is_empty() || arg.starts_with('-') {
        return -1;
    }
    let c = crate::process::Command { program: "/usr/bin/open", args: &[arg], cwd: None, env: &[] };
    let Ok(mut child) = c.spawn_own() else { return -1 };
    child.close_stdin();
    for _ in 0..500 {
        match child.try_wait() {
            Ok(Some(0)) => return 0,
            Ok(Some(_)) | Err(_) => return -1,
            Ok(None) => crate::sleep_ms(10),
        }
    }
    -1
}

/// show a folder / a file in the Finder -> 0 / -1  [plat_open_folder]
pub fn open_folder(path: &str) -> i32 {
    if !crate::policy::fs_allowed(path, crate::policy::FS_READ) {
        return -1;
    }
    open_with_launch_services(path)
}

/// a web page in the default application -> 0 / -1  [plat_open_url]
pub fn open_url(url: &str) -> i32 {
    if !crate::policy::url_allowed(url) {
        return -1;
    }
    open_with_launch_services(url)
}

/// start this program again, on its own (process.rs launch: a session of its own, standard handles
/// on /dev/null), with the arguments (at most 31) -> 0 / -1  [plat_spawn_self]; iOS: -1
pub fn spawn_self(args: &[&str]) -> i32 {
    if cfg!(target_os = "ios") || args.len() >= 32 || args.iter().any(|a| a.contains('\0')) {
        return -1;
    }
    match crate::current_exe() {
        Some(exe) if crate::process::launch_own(&exe, args).is_ok() => 0,
        _ => -1,
    }
}

/// like spawn_self: the program file now at this process's path (an update replaced it) -> 0 / -1
pub fn restart_self(args: &[&str]) -> i32 {
    spawn_self(args)
}
