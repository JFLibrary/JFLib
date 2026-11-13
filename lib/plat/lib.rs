// plat -- the platform layer: one module per OS with the same functions  [src/plat]
//
//   process: entry point (calls the app's jfgui_main), args(), arg_value(name), exit(code),
//            std_start() (feature std: the start of a program with std, from its fn main),
//            write_stdout / write_stderr, time_us(), sleep_ms(ms), date()
//   environment: env(name) (= getenv), setenv(name, Some / None), config_dir(), temp_dir(), cwd()
//   folders: user_dir(DIR_DOCUMENTS ..) (desktop, documents, downloads, pictures, music,
//            videos: Known Folders / XDG user dirs), log_dir()                  (dirs.rs)
//   files:   read_file(path), write_file(path, data) (native: exclusive sibling + rename),
//            File::open(path, FO_*) / open_temp(dir) -> read_at / write_at / size / sync,
//            file_exists(path) (0 / 1 file / 2 dir), mkdir (with parents), delete, rename,
//            trash (into the recycle bin / freedesktop trash; trash.rs),
//            file_exists_checked(path) (Some(0) missing, None error), FileLock::acquire(path),
//            AtomicFile::create(path) -> write_at / commit (write_file in pieces),
//            dir_list(path, f(name, is_dir, size_lo)) -- checked against the app's
//            permissions first (guarded.rs); a refused path behaves like a missing one
//   policy:  the app's permissions (apps/NAME/PERMISSIONS): install, the fs / process /
//            url / plugin / print checks, grants for the user's files      (policy.rs)
//   boot:    the app's name, app:// and its manifest at start-up (rt::main! through
//            rt::boot; ui::main! calls the parts itself)                    (boot.rs)
//   vfs:     "scheme://path" names in every file function: app:// (the bundled assets),
//            config:// data:// cache:// logs:// (the app's folders), documents:// and the
//            other user folders, mem://, mounted providers
//            (MemFs, DirFs, Overlay, Sub, Bundle); data_dir(), cache_dir()   (vfs.rs);
//            display_name(path); Android content:// (ContentFs, DocHost)     (content.rs);
//            persistent_name(path): a chosen file's name that lasts across restarts (Flatpak's
//            documents portal: linux_docs.rs)
//   secret:  secret_set / secret_get / secret_delete / secret_available: passwords, tokens and
//            keys in the system's store (Credential Manager + DPAPI, Secret Service; needs
//            `secrets` in the manifest)                                        (secret.rs)
//   instance: claim(app) -> Primary / Forwarded / Alone, take() -> requests of later starts
//            (the app's single instance; pipe / abstract socket per user)  (instance.rs)
//   spell:   sys_spell_find / sys_spell_languages, SysSpeller (check, suggest): the system's
//            spell checker (Windows ISpellChecker, macOS NSSpellChecker, iOS UITextChecker;
//            elsewhere none)                                                (spell.rs)
//   tray:    tray_set / tray_remove (the symbol in the notification area, its menu), notify /
//            notify_close (system notifications), tray_take() -> events   (tray.rs)
//   power:   inhibit(reason, INHIBIT_SUSPEND | INHIBIT_IDLE) -> Option<Inhibit> (no standby /
//            screen saver while the guard lives), inhibited()              (power.rs)
//   autostart: autostart_set(on, args) / autostart_get() / autostart_entry() (Run key, XDG
//            autostart, Background portal, MSIX StartupTask; needs `autostart` in the manifest)
//            (autostart.rs)
//   pkg:     packaged() (MSIX, Flatpak, AppImage, Snap or a plain program), msix_updates (App
//            Installer), FlatpakMonitor / flatpak_spawn_latest (the Flatpak portal),
//            restart_appimage                                                   (pkg.rs)
//   taskbar: taskbar_progress / taskbar_badge / taskbar_tasks (the taskbar button: progress,
//            overlay / count, jump list; ITaskbarList3, Unity LauncherEntry), app_user_model_id
//            / set_app_user_model_id (Windows: the app's explicit id)        (taskbar.rs)
//   assoc:   assoc_register / assoc_conflicts / assoc_unregister: the app's PACKAGE file types
//            and URL schemes for this program without a setup (HKCU\Software\Classes; .desktop,
//            shared-mime-info, mimeapps.list), file_uri_path; default_browser_status /
//            default_browser_request (PACKAGE browser=1)                        (assoc.rs)
//   recent:  recent_add / recent_list / recent_remove / recent_clear: the system's recent files
//            (SHAddToRecentDocs and the jump list; recently-used.xbel)          (recent.rs)
//   audio:   Stream (open / open_fill, write, start / pause / stop, volume, latency, the default
//            device followed), play / play_file (WAV, Ogg Vorbis), sound(SOUND_*): the system's
//            sounds (WASAPI, PulseAudio protocol, WebAudio, AAudio)               (audio.rs)
//   screen:  monitors() (rect, work area, dpi, refresh, primary; SET_MONITORS), pick_start (a
//            pixel's colour anywhere), capture_start (pictures of the screen)  (screen.rs)
//   hotkey:  hotkey_register / hotkey_unregister / hotkey_take: global shortcuts (RegisterHotKey,
//            XGrabKey, the GlobalShortcuts portal)                             (hotkey.rs)
//   threads: thread_spawn(stack, f) -> Thread (join), thread_yield(), Sem (post / wait),
//            wake() (any thread: E_WAKE on the UI thread), wait_events(deadline, on_event)
//            (headless), cpu_count(), mem_total(); the windows, the clipboard, the dialogs and
//            the portals belong to the UI thread, the first one that uses them: other threads
//            panic                                                          (ui_only.rs)
//   pool:    thr_init / thr_count / thr_shutdown / thr_self, parallel_for, Job   (pool.rs)
//   task:    async execution: Executor (ui() in the event loop, new() / block_on), Task,
//            spawn_blocking (a pool for blocking work), sleep, CancelToken, Progress, join /
//            select                                                            (task.rs)
//   aio:     files without blocking: read_file / write_file / copy_file with progress and
//            cancellation, Reader / Writer chunk by chunk (AtomicFile underneath)  (aio.rs)
//   sync:    Mutex, Condvar, RwLock, channel (futex / WaitOnAddress)          (sync.rs)
//   rand:    fill_random, random_bytes, random_u64 (the OS CSPRNG)             (rand.rs)
//   net:     TcpListener, TcpStream, UdpSocket, SocketAddr, resolve            (net.rs)
//   fs:      stat / lstat / read_dir / links / set_times / free_space /
//            write_file_private / local_zone_id                                (fs.rs)
//   cpu:     cpu_features() (CPU_*), cpu_brand()                                 (cpu.rs)
//   crash:   crash_hook(Some(f)): f(&CrashInfo) before exit code 3 -- hardware faults (VEH /
//            signals, the stderr line of ASMLIB) and panics; crash_test() crashes on purpose
//   windows: the window-indexed API of the assembly (win_create / win_show / win_present /
//            wait ..., popups, clipboard, drops, pen, cursors; consts.rs, evq.rs, pwin0.rs)
//   dnd:     drag and drop between programs: DragData (files, text, URLs, HTML, image),
//            the drop target hook, the pending drop, drag_start (dragging out)    (dnd.rs)
//   (plat-shared) print(job) + page layout / PDF writer (print.rs; win_print.rs,
//            linux_print.rs), plugin libraries lib_open / lib_sym / lib_close / lib_seg and
//            the crash guard guard_call / guard_run / guard_tag (plib.rs; win_lib.rs,
//            linux_elfload.rs, linux_guard.rs), monitor_icc / monitor_key (icc.rs), the
//            portal's file dialog / OpenURI / Print (linux_portal.rs)
//   (jxl-module) the loadable JPEG XL writer module: jxlenc_set_path / jxlenc_ready /
//            jxlenc_encode (modhost.rs; the records in modabi.rs)
//
// The layer knows events, pixel rows, files and threads only -- no UI, no gfx.  One of the
// four library crates that may contain `unsafe` (rt, simd, plat, gpu); nothing unsafe leaves it
// but dl_sym and dl_close (a symbol of a library loaded at run time is the caller's to trust).
#![no_std]
#[cfg(feature = "std")]
extern crate std;

extern crate alloc;

/// Owned native child processes with pipes and process-tree lifetime management.
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub mod process;
/// Mutex, Condvar, RwLock and bounded channels for worker threads.
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub mod sync;
/// TCP and UDP sockets (IPv4 / IPv6) with timeouts, numeric names.
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub mod net;
/// Metadata, directory listings, links, times, free space, owner-only files, the local zone.
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub mod fs;
/// Running as a system service (Windows service control manager, systemd units).
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub mod service;
/// Changes below folders as they happen (continuous data protection).
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub mod watch;
/// The operating system's trusted certificate authorities (TLS clients of public servers).
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub mod certs;
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
mod rand;
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub use rand::*;

use alloc::string::String;

#[cfg(not(any(jfgui_glibc, feature = "std")))] pub use rt::os::exit; pub use rt::os::{write_stderr, write_stdout};
// a program with std (feature std): its exit and the symbol its fn main calls
#[cfg(feature = "std")]
mod hosted;
#[cfg(feature = "std")]
pub use hosted::{exit, jfgui_std_start};
#[cfg(all(feature = "std", any(target_arch = "wasm32", target_os = "android")))]
pub use hosted::std_start;
// the library's version (the root Cargo.toml, docs/VERSIONING.md); the app's own: rt::app_version!()
pub use rt::{VERSION, VERSION_MAJOR, VERSION_MINOR, VERSION_PATCH};

mod cpu;
mod capabilities;
pub use capabilities::*;
/// The app's permissions (apps/NAME/PERMISSIONS) and their checks.
pub mod policy;
/// OS sandbox derived from the manifest: Landlock + seccomp (Linux), mitigation policies (Windows).
pub mod sandbox;
#[cfg(target_os = "linux")]
#[path = "linux_sandbox.rs"]
mod linux_sandbox;
#[cfg(windows)]
#[path = "win_sandbox.rs"]
mod win_sandbox;
/// What build_rs.py builds into a program (name, assets, manifest), set up at start-up.
pub mod boot;
/// One file namespace: app://, config://, data://, cache://, mem:// and mounted providers.
pub mod vfs;
pub use vfs::display_name;
/// The documents of other apps as files: Android's content:// names (the name logic on all
/// platforms, tested with a host in memory).
pub mod content;
/// Passwords, tokens and keys in the system's secret store (Credential Manager, Secret Service).
pub mod secret;
/// One running instance per app and user; later starts hand over their arguments.
pub mod instance;
/// The system's spell checker (Windows ISpellChecker, macOS NSSpellChecker, iOS UITextChecker);
/// Hunspell files are the spell crate's.
pub mod spell;
/// The app's symbol in the notification area and system notifications.
pub mod tray;
/// The app's menus in the system's own menu bar (macOS).
pub mod sysmenu;
/// Keeping the computer awake: no standby, no screen saver while a guard lives.
pub mod power;
pub use power::{INHIBIT_IDLE, INHIBIT_SUSPEND, Inhibit, inhibit, inhibited};

pub mod sensors;

/// Android: while `on`, the back key reaches the app as Esc (a page of the app goes back, a popup
/// closes; `UIC_ESCAPE` when nothing took it) instead of ending the activity.  Elsewhere nothing:
/// there Esc is a key of its own
pub fn back_trap(on: bool) {
    #[cfg(target_os = "android")]
    web::system_host::back_trap(on);
    #[cfg(not(target_os = "android"))]
    let _ = on;
}

/// Android: the runtime ended (ui.quit): the activity goes and its process with it, a new start
/// needs a new runtime; iOS: the process ends (UIKit's loop never returns).  Elsewhere nothing:
/// there the main loop ends the program
pub fn app_finish() {
    #[cfg(target_os = "android")]
    web::system_host::app_finish();
    #[cfg(target_os = "ios")]
    apple::ios::finish();
}

/// (ui touch.rs) the host's main pointer is a finger at the start (Android, iOS; a phone's browser:
/// `pointer: coarse`; a Windows tablet in slate posture): no scroll bars from the first frame on; the
/// next pointer event decides again
pub fn touch_primary() -> bool {
    #[cfg(target_arch = "wasm32")]
    return web::touch_primary();
    #[cfg(any(target_os = "android", target_os = "ios"))]
    return true;
    #[cfg(windows)]
    return win_touch_primary();
    #[cfg(not(any(target_arch = "wasm32", target_os = "android", target_os = "ios", windows)))]
    false
}

/// (ui touch.rs) a text field got the focus through a finger (show) or lost it: the on-screen keyboard
/// of window w.  Windows: InputPane; Wayland: text-input follows the focus by itself (the compositor's
/// keyboard); elsewhere the host's job (Android, the browser) or nothing there is to ask (X11)
pub fn touch_keyboard(w: i32, show: bool) {
    #[cfg(windows)]
    win_touch_keyboard(w, show);
    #[cfg(not(windows))]
    let _ = (w, show);
}

/// (ui touch.rs) a finger's gesture reached a widget (a tap, a drag), or it did not (a scroll):
/// Android and iOS ask after the finger lifted whether the keyboard has to follow
pub fn touch_reached(reached: bool) {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    web::system_host::touch_reached(reached);
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let _ = reached;
}

/// (ui touch.rs) a finger rested long enough: the host's feedback (Android: performLongClick)
pub fn touch_long_press() {
    #[cfg(target_os = "android")]
    web::system_host::touch_long_press();
}

/// where a window the user moves may be (x, y; w its width; screen the area's size; dpi for the
/// margin): the title bar stays on the screen, at least 48 DIP of the window at each side
pub fn reach_clamp(p: [i32; 2], w: i32, screen: [i32; 2], dpi: i32) -> [i32; 2] {
    let m = 48 * dpi / 96;
    [p[0].clamp(m.min(w) - w, (screen[0] - m).max(m.min(w) - w)), p[1].clamp(0, (screen[1] - m).max(0))]
}
/// Starting the app when the user logs in (Run key, XDG autostart, Background portal, an MSIX
/// package's StartupTask).
pub mod autostart;
/// The package the program runs from (MSIX, Flatpak, AppImage, Snap) and what its package
/// system offers for updates (App Installer, the Flatpak portal).
pub mod pkg;
// the few WinRT calls (StartupTask, Package) through raw COM
#[cfg(windows)]
mod win_winrt;
pub use autostart::{AUTOSTART_WAIT_MS, AutostartError, autostart_entry, autostart_get, autostart_set};
/// The app's taskbar / dock button: progress, a badge, the jump list's tasks.
pub mod taskbar;
/// The app's file types and URL schemes registered without a setup (developer builds); a web
/// browser's default (default_browser_status / default_browser_request).
pub mod assoc;
pub use assoc::{BrowserStatus, default_browser_request, default_browser_status};
/// The system's list of recently used files (jump lists, recently-used.xbel).
pub mod recent;
pub use recent::{recent_add, recent_clear, recent_list, recent_remove};
/// Sound output: streams, short sounds from memory or files, the system's own sounds.
#[cfg_attr(target_vendor = "apple", allow(dead_code))] // no sound backend on Apple yet: the mixer waits for it
pub mod audio;
/// The monitors, a pixel's colour anywhere on the screen, pictures of the screen.
pub mod screen;
pub use screen::{Monitor, monitors};
/// Global keyboard shortcuts: chords that reach the app while another program has the keyboard.
pub mod hotkey;
pub use hotkey::{Chord, HotkeyError, HotkeyEvent, hotkey_register, hotkey_take, hotkey_unregister};
// what the desktop integration shares on Linux (the session-bus link, the .desktop id)
#[cfg(target_os = "linux")]
mod linux_desk;
// the damage regions of the X11 / Wayland presents
#[cfg(target_os = "linux")]
mod linux_damage;
// the copy of a present's dirty rectangles into the window's pixels (Windows, X11, Wayland): in
// row bands on the pool when large, past the caches at 4K
#[cfg(any(windows, target_os = "linux"))]
mod present_copy;
#[cfg(any(windows, target_os = "linux"))]
pub use present_copy::present_copy_bench;
// the log: levels, a ring for crash reports, stderr, a rotating file
mod log;
pub use log::*;
// the accessibility tree for screen readers (AT-SPI2, UI Automation)
pub mod a11y;
// the system settings the UI follows (theme, accent, contrast, motion, text scale, timing)
mod settings;
pub use settings::{HcColors, SysSettings, sys_settings, sys_settings_env, font_hint_parse, font_rgba_parse,
    Titlebar, titlebar_layout_parse, titlebar_action_parse, TB_BTN_CLOSE, TB_BTN_MIN, TB_BTN_MAX,
    TB_ACT_NONE, TB_ACT_MAXIMIZE, TB_ACT_MINIMIZE, TB_ACT_LOWER, TB_ACT_MENU,
    FONT_HINT_NONE, FONT_HINT_SLIGHT, FONT_HINT_MEDIUM, FONT_HINT_FULL,
    FONT_RGBA_NONE, FONT_RGBA_RGB, FONT_RGBA_BGR, FONT_RGBA_VRGB, FONT_RGBA_VBGR};
// the user's languages and regional format (BCP 47 tags)
mod locale;
pub use locale::{locale_tag, system_languages, system_locale};
mod guarded;
pub use guarded::*;
// the user's folders (documents, downloads, pictures ...) and the folder of logs
mod dirs;
pub use dirs::*;
// moving files to the trash instead of deleting them
mod trash;
mod control_stream;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub use control_stream::*;
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub mod control_ipc;
#[cfg_attr(all(any(target_arch = "wasm32", target_os = "android"), not(web_threads)), path = "web_pool.rs")]
mod pool;
/// Async execution: executors in the event loop and for block_on, a pool for blocking work,
/// timers, cancellation, progress, joins.
pub mod task;
/// Files without blocking the caller: chunked reads, atomic writes and copies with progress and
/// cancellation.
pub mod aio;
pub use cpu::*;
pub use pool::*;

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::*;
// a managed program on the runtime another application ships; values of HKLM
#[cfg(windows)]
mod win_clr;
#[cfg(windows)]
pub use win_clr::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

// macOS and iOS through libSystem (docs/APPLE.md)
#[cfg(target_vendor = "apple")]
mod apple;
#[cfg(target_vendor = "apple")]
pub use apple::*;

/// Host-driven boundary shared by browser workers, the native Android adapter and the iOS host
/// (apple_ios.rs: only the host part there, docs/APPLE.md AP5).
#[cfg(any(target_arch = "wasm32", target_os = "android", target_os = "ios"))]
#[cfg_attr(target_os = "ios", allow(dead_code))] // (iOS: the host part only, the rest is the browser's)
pub mod web;
#[cfg(any(target_arch = "wasm32", target_os = "android", target_os = "ios"))]
pub use web::*;

mod clipdrop;
// drag and drop between programs (the shared part; win_dnd.rs, linux_x11_dnd.rs, linux_wl_clip.rs)
#[cfg_attr(target_os = "ios", allow(dead_code))] // no drags between programs on iOS yet
mod dnd;
pub use dnd::*;
// clipboard_platform: reusable rich clipboard framing.
mod cliprich;
pub use cliprich::*;
mod consts;
// the UI thread's own state: UiOnly<T> lets only the UI thread at it (a safe call on another
// thread panics instead of racing)
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
mod ui_only;
// (E3) drawn cursors for the shapes Windows and X11 have none of (Wayland's own images, macOS'
// zoom and cell)
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
mod cursor_art;
mod evq;
mod ime;
pub use ime::*;
#[cfg_attr(any(target_arch = "wasm32", target_os = "android", target_vendor = "apple"), path = "web_icc.rs")]
mod icc;
mod pwin0;
pub use clipdrop::*;
pub use consts::*;
pub use evq::*;
pub use icc::*;
pub use pwin0::*;
// plat-shared: printing (print.rs + win_print.rs / linux_print.rs), plugin libraries and the
// crash guard (plib.rs + win_lib.rs / linux_elfload.rs, linux_guard.rs; x86-64 and ARM64)
#[cfg_attr(any(target_arch = "wasm32", target_os = "android", target_vendor = "apple"), path = "web_lib.rs")]
mod plib;
mod print;
pub use plib::*;
pub use print::*;
// jxl-module: loadable modules, the host side (the JPEG XL writer jxlenc; modhost.rs, modabi.rs)
#[cfg_attr(any(target_arch = "wasm32", target_os = "android", target_vendor = "apple"), path = "web_modhost.rs")]
mod modhost;
pub use modhost::*;

/// one platform event (the EV_* record): type, button, modifiers, window index (0 = the
/// main window), position (px; E_RESIZE: size), data
#[derive(Clone, Copy, Default)]
pub struct Event {
    pub typ: u8,
    pub btn: u8,
    pub mods: u8,
    pub win: u8,
    pub x: i32,
    pub y: i32,
    pub data: u32,
}

/// the pixels to present: rows of 32-bit B, G, R, x
pub struct Frame<'a> {
    pub px: &'a [u8],
    pub w: i32,
    pub h: i32,
    pub stride: i32,
}

// ---- files
pub const FO_READ: u32 = 0; // existing file, read only
pub const FO_WRITE: u32 = 1; // create or truncate, read + write
pub const FO_RW: u32 = 2; // open or create (contents kept), read + write

/// a date and time (date(): Windows local time, Linux UTC)
#[derive(Clone, Copy, Default, Debug)]
pub struct Date {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub min: u16,
    pub sec: u16,
    pub ms: u16,
    pub wday: u16, // 0 = Sunday
}

/// The name under which a file the user chose stays reachable after a restart, for a list of
/// recent files or a session (ui.recent_add keeps this one).  A document of Flatpak's
/// documents portal (`$XDG_RUNTIME_DIR/doc/ID/NAME`) is asked for its lasting form
/// (`Documents.Add`, persistent; linux_docs.rs): `<mount>/<ID>/<NAME>` of the portal's answer.
/// Where the portal reaches the real file that is a lasting document of it; for a document
/// of its own mount the portal answers that document's own ID, so inside the sandbox the name
/// stays as the dialog gave it (the portal's file dialog makes lasting documents, "Open with"
/// session ones).  Every other name comes back unchanged: Android keeps the grant of a
/// `content://` document and the browser the handle of a `/handles/` file when the user
/// chooses it, and OS paths need nothing.  The name must be readable under the app's
/// permissions.
pub fn persistent_name(name: &str) -> String {
    #[cfg(target_os = "linux")]
    if crate::policy::fs_allowed(name, crate::policy::FS_READ) {
        if let Some(p) = crate::linux_docs::persistent(name) {
            return p;
        }
    }
    String::from(name)
}

// (E8) the files of the last file_dialog with FD_MULTI
static FD_PATHS: rt::sync::SpinLock<alloc::vec::Vec<String>> = rt::sync::SpinLock::new(alloc::vec::Vec::new());

/// (E8) the files the last system file dialog with FD_MULTI chose (its first is also in
/// file_dialog's path; Ctrl / Shift in the dialog choose more); one entry after a dialog that
/// chose one file, empty after a cancelled one.  Windows and Linux's portal choose several, the
/// other systems' dialogs one (docs/GUIDE.md section 7)
pub fn file_dialog_paths() -> alloc::vec::Vec<String> {
    FD_PATHS.lock().clone()
}

// (backends) the answer of a dialog: every chosen file
#[allow(dead_code)]
pub(crate) fn fd_paths_set(v: alloc::vec::Vec<String>) {
    *FD_PATHS.lock() = v;
}

/// The answer of a system file dialog that answers later (file_dialog -> FD_PENDING: only
/// Android's picker and the browser's, web.rs, and iOS's, apple_ios_services.rs): None while it
/// is open, then once (1 chosen / 0 not, index, path).
#[cfg(not(any(target_os = "android", target_arch = "wasm32", target_os = "ios")))]
pub fn file_dialog_poll() -> Option<(i32, u32, String)> {
    None
}

/// the argument after `name` on the command line (argument 0, the program, is skipped)  [arg_value]
pub fn arg_value(name: &str) -> Option<String> {
    let a = args();
    let i = a.iter().skip(1).position(|x| x == name)? + 1;
    a.get(i + 1).cloned()
}

/// the library's options that take a value (`--size 800x600`): a program whose plain
/// arguments are files skips the value after these
pub const VALUE_OPTS: &[&str] = &[
    "--size", "--scale", "--auto-close", "--anim", "--theme", "--accent", "--theme-file", "--contrast", "--text-scale",
    "--lang", "--shot", "--script", "--threads", "--simd",
];

// ---- crashes (docs/modules/CRASH.md)
pub use rt::crash::{CK_ACCESS, CK_BUS, CK_DIVIDE, CK_ILLEGAL, CK_OTHER, CK_PANIC, CK_STACK, CrashHook, CrashInfo};

/// f(&CrashInfo) runs in the crash handler, after the stderr line, before exit code 3; also
/// for panics (CK_PANIC)  [plat_crash_hook]
pub fn crash_hook(f: Option<CrashHook>) {
    rt::crash::set_hook(f)
}

/// a deliberate crash for tests: `mov eax, 16` (5 bytes), then a 4-byte write to address 0x10
/// -- the faulting instruction is at crash_test + 5
#[unsafe(naked)]
#[cfg(target_arch = "x86_64")]
pub extern "C" fn crash_test() -> ! {
    core::arch::naked_asm!("mov eax, 16", "mov dword ptr [rax], eax", "ud2")
}

/// (plat-win) crash_test called from a known place (call rel32): the return address
/// crash_test_caller + 5 must be among CrashInfo::frames (plattest's call chain check)
#[unsafe(naked)]
#[cfg(target_arch = "x86_64")]
pub extern "C" fn crash_test_caller() -> ! {
    core::arch::naked_asm!("call {0}", "ud2", sym crash_test)
}

/// Deliberate trap for browser runtime diagnostics.
#[cfg(target_arch = "wasm32")]
pub extern "C" fn crash_test() -> ! { core::arch::wasm32::unreachable() }
/// Deliberate trap through the compatibility caller API.
#[cfg(target_arch = "wasm32")]
pub extern "C" fn crash_test_caller() -> ! { crash_test() }

/// a deliberate crash for tests on ARM64 (Apple, Linux, Windows): x0 = 16, then a 4-byte write to address 0x10 --
/// the faulting instruction is at crash_test + 4
#[unsafe(naked)]
#[cfg(all(target_arch = "aarch64", any(target_vendor = "apple", target_os = "linux", windows)))]
pub extern "C" fn crash_test() -> ! {
    core::arch::naked_asm!("mov x0, #16", "str w0, [x0]", "brk #0")
}

/// crash_test called from a known place (bl): the return address crash_test_caller + 4 must be
/// among CrashInfo::frames
#[unsafe(naked)]
#[cfg(all(target_arch = "aarch64", any(target_vendor = "apple", target_os = "linux", windows)))]
pub extern "C" fn crash_test_caller() -> ! {
    core::arch::naked_asm!("bl {0}", "brk #0", sym crash_test)
}

/// Deliberate termination on Android, where no hardware exception translator is installed.
#[cfg(all(target_os = "android", not(target_arch = "x86_64")))]
pub extern "C" fn crash_test() -> ! { rt::os::exit(3) }
/// Compatibility caller for the Android diagnostic termination.
#[cfg(all(target_os = "android", not(target_arch = "x86_64")))]
pub extern "C" fn crash_test_caller() -> ! { crash_test() }
