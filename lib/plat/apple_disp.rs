// apple_disp -- what macOS and iOS share of the window-level API (home, drives, printing, the file
// dialog, opening URLs and folders, starting this program again), and what iOS's host path leaves
// to the platform (docs/APPLE.md AP5: the windows are web_windows.rs's in the canvas of
// apple_ios.rs; wait() for programs without the host, the clipboard until AP5b, the theme).
// macOS's windows: apple_mac.rs.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[cfg(target_os = "ios")]
pub use nodisp::*;

// iOS (AP5): the windows live in one canvas the UIKit host shows (web_windows.rs, apple_ios.rs);
// what the host path leaves to the platform stays here
#[cfg(target_os = "ios")]
mod nodisp {
    /// a context menu of the system at a window position: none on iOS (the app's own menus)
    pub fn win_show_menu(_w: i32, _x: i32, _y: i32) {}

    /// a program without the host (a headless run, a command-line program): sleep until the
    /// deadline (0 = poll, u64::MAX = forever) or a wake(); the host's runtime never waits
    pub fn wait(deadline_us: u64) -> u32 {
        if crate::evq_count() == 0 {
            crate::wait_events(deadline_us, &mut |e| {
                crate::evq_push(*e);
            });
        }
        crate::evq_count()
    }

    // the clipboard: UIPasteboard (apple_ios_services.rs)
    pub use super::super::ios::{clip_get, clip_has, clip_set, clip_set_many};

    /// (dnd.rs) dragging data out of the app: not on iOS yet
    pub(crate) fn os_drag_start(_data: crate::DragData) -> bool {
        false
    }

    /// 0 light, 1 dark, -1 unknown: jfgui_THEME=light|dark, else the scene's appearance as the host
    /// sent it (UITraitCollection, web_system.rs)
    pub fn theme() -> i32 {
        match crate::env("jfgui_THEME").as_deref() {
            Some("dark") => 1,
            Some("light") => 0,
            _ => match crate::web::system_host::dark() {
                Some(true) => 1,
                Some(false) => 0,
                None => -1,
            },
        }
    }

    /// pens (-1 = unknown; the Apple Pencil shows itself at its first touch)
    pub fn pen_devices() -> i32 {
        -1
    }
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
/// variant jfgui_PRINT_TO=FILE (the PDF into a file); macOS: the system's print dialog
/// (apple_mac_app.rs print_dialog); iOS: ERR_UNSUPPORTED before AP5
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
    #[cfg(target_os = "macos")]
    return super::mac::print_dialog(job);
    #[cfg(not(target_os = "macos"))]
    crate::ERR_UNSUPPORTED
}

// iOS's file dialog: the system's picker, answering later (apple_ios_services.rs); macOS: apple_mac.rs
#[cfg(target_os = "ios")]
pub use super::ios::{file_dialog, file_dialog_poll};

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

/// a web page in the default application (iOS: UIApplication openURL) -> 0 / -1  [plat_open_url]
pub fn open_url(url: &str) -> i32 {
    if !crate::policy::url_allowed(url) {
        return -1;
    }
    #[cfg(target_os = "ios")]
    return super::ios::open_url(url);
    #[cfg(not(target_os = "ios"))]
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
