// linux_disp -- display backend dispatch on Linux: Wayland (linux_wl*.rs) or X11 (linux_x11*.rs)
//                                                        [src/plat/linux/disp.s, lx_popup.s]
//
// The display-dependent functions go through the backend object (trait Display).  The
// first win_create (the main window, owner PW_NOOWNER) picks the backend:
//   jfgui_BACKEND=wayland | x11     forces one (only the first letter counts, like disp.s)
//   WAYLAND_DISPLAY set              Wayland first, then X11 if DISPLAY is set
//   only DISPLAY set                 X11 first, then Wayland (default socket)
//   neither                          Wayland (default socket), else an error
// When no backend can open its main window the process ends with exit code 1 and a
// message (as disp.s).  Before that the minimum size of window 0 and the refresh callback
// are remembered and handed to the chosen backend.  wait() without a backend (headless
// runs) sleeps until its deadline or a wake() (lx_wait_nodisp).
//
// The trait is the contract between the two backend agents and with Windows (win_api.rs has
// the same free functions): change it only together and say so in the report.
//
// plat-x11 (R4): the backend choice follows disp.s exactly (first letter of
// jfgui_BACKEND, the fallback order, exit 1 when every backend fails, the Wayland
// backend's "quiet" flag: disp_wl_quiet()); Display::pen_devices (default -1) was added
// for plat_pen_devices; cursor_dpi without a display answers the dpi (pre_cdpi); the
// headless wait is lx_wait_nodisp (wake fd, E_WAKE).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use rt::sync::SpinLock;

use crate::Frame;

pub const BK_NONE: u32 = 0;
pub const BK_WAYLAND: u32 = 1;
pub const BK_X11: u32 = 2;

/// a display backend (window-indexed; see the free functions below for the semantics)
pub trait Display: Send {
    fn ime_pos(&mut self,_x:i32,_y:i32,_h:i32) {}
    fn win_create(&mut self, title: &str, w_dip: i32, h_dip: i32, flags: u32, owner: i32, x: i32, y: i32) -> i32;
    fn win_destroy(&mut self, w: i32);
    fn win_show(&mut self, w: i32, mode: u32);
    fn win_activate(&mut self, w: i32);
    fn win_enable(&mut self, w: i32, on: bool);
    fn win_set_rect(&mut self, w: i32, x: i32, y: i32, cw: i32, ch: i32);
    fn win_get_rect(&mut self, w: i32) -> Option<[i32; 4]>;
    fn win_work_area(&mut self, w: i32) -> Option<[i32; 4]>;
    fn win_set_title(&mut self, w: i32, title: &str);
    fn win_state(&mut self, w: i32) -> u32;
    fn win_cmd(&mut self, w: i32, cmd: u32);
    fn win_begin_move(&mut self, w: i32);
    fn win_begin_resize(&mut self, w: i32, edges: u32);
    fn win_size(&mut self, w: i32) -> (i32, i32);
    fn win_dpi(&mut self, w: i32) -> i32;
    fn win_set_min_size(&mut self, w: i32, w_dip: i32, h_dip: i32);
    fn win_present(&mut self, w: i32, f: &Frame, rects: Option<&[[i32; 4]]>);
    fn win_set_cursor(&mut self, w: i32, shape: u32);
    fn set_refresh(&mut self, f: Option<fn()>);
    fn wait(&mut self, deadline_us: u64) -> u32;
    fn popup(&mut self, slot: u32, owner: i32, rect: Option<[i32; 4]>, f: Option<&Frame>);
    /// (§6) how the next popup() of the slot hangs at its anchor -- a backend that places popups
    /// itself (Wayland) builds its positioner from it; the others show them where popup() says
    fn popup_anchor(&mut self, _slot: u32, _anchor: [i32; 4], _kind: u32, _nat: [i32; 2]) {}
    /// where the backend put the slot's popup when it is not where the UI asked (Wayland)
    fn popup_placed(&mut self, _slot: u32) -> Option<[i32; 2]> {
        None
    }
    fn cursor_create(&mut self, px: &[u8], w: i32, h: i32, hot_x: i32, hot_y: i32, dpi: i32) -> i32;
    fn cursor_destroy(&mut self, id: i32);
    fn cursor_dpi(&mut self, dpi: i32) -> i32;
    fn clip_set(&mut self, fmt: u32, data: &[u8]) -> bool;
    fn clip_set_many(&mut self, entries: &[(u32, &[u8])]) -> bool;
    fn clip_get(&mut self, fmt: u32) -> Option<Vec<u8>>;
    fn clip_has(&mut self, fmt: u32) -> bool;
    /// pens / pen tablets the backend sees, -1 = unknown (plat_pen_devices: wl_pen_devices /
    /// x11_pen_devices)  -- plat-x11: added with a default, the Wayland backend overrides it
    fn pen_devices(&mut self) -> i32 {
        -1
    }
    /// drag data out of the app (dnd.rs drag_start; the left button is held) -> started;
    /// E_DRAG_END follows -- added with a default for the drag and drop work (X11, Wayland)
    fn drag_start(&mut self, _data: crate::DragData) -> bool {
        false
    }
}

struct Disp {
    bk: Option<Box<dyn Display>>,
    kind: u32,
    min0: (i32, i32),
    refresh: Option<fn()>,
    hit: Option<fn(i32, i32, i32) -> u32>,
}

static DISP: SpinLock<Disp> = SpinLock::new(Disp { bk: None, kind: BK_NONE, min0: (0, 0), refresh: None, hit: None });
static WL_QUIET: AtomicBool = AtomicBool::new(false);

/// 0 none, 1 Wayland, 2 X11
pub fn disp_backend() -> u32 {
    DISP.lock().kind
}

/// (disp.s g_wl_quiet) the Wayland backend is being tried without jfgui_BACKEND=wayland:
/// it should fail without messages (X11 may follow)
pub fn disp_wl_quiet() -> bool {
    WL_QUIET.load(Ordering::Relaxed)
}

fn nonempty_env(name: &str) -> bool {
    crate::env(name).is_some_and(|v| !v.is_empty())
}

// one backend: connect, replay the remembered minimum size / refresh callback, create the
// main window -> (backend, index) or None
fn try_backend(k: u32, title: &str, w_dip: i32, h_dip: i32, flags: u32, x: i32, y: i32) -> Option<(Box<dyn Display>, i32)> {
    let mut b = if k == BK_WAYLAND { crate::wl_connect()? } else { crate::x11_connect()? };
    let (min0, refresh) = {
        let d = DISP.lock();
        (d.min0, d.refresh)
    };
    b.win_set_min_size(0, min0.0, min0.1);
    if refresh.is_some() {
        b.set_refresh(refresh);
    }
    let w = b.win_create(title, w_dip, h_dip, flags, crate::PW_NOOWNER, x, y);
    if w < 0 { None } else { Some((b, w)) }
}

// the first window chooses the backend (pre_win_create); every failure ends the process
fn choose(title: &str, w_dip: i32, h_dip: i32, flags: u32, x: i32, y: i32) -> i32 {
    // forced: 0 auto, BK_WAYLAND, BK_X11 (only the first letter of jfgui_BACKEND counts)
    let forced = match crate::env("jfgui_BACKEND").and_then(|v| v.bytes().next()) {
        Some(b'x') => BK_X11,
        Some(b'w') => BK_WAYLAND,
        _ => 0,
    };
    let mut x11_first = false;
    let mut next = BK_WAYLAND;
    if forced != 0 {
        next = forced;
    } else if !nonempty_env("WAYLAND_DISPLAY") && nonempty_env("DISPLAY") {
        x11_first = true;
        next = BK_X11;
    }
    loop {
        if next == BK_WAYLAND {
            WL_QUIET.store(forced != BK_WAYLAND, Ordering::Relaxed);
            DISP.lock().kind = BK_WAYLAND;
            if let Some((b, w)) = try_backend(BK_WAYLAND, title, w_dip, h_dip, flags, x, y) {
                DISP.lock().bk = Some(b);
                return w;
            }
            if forced != 0 || x11_first || !nonempty_env("DISPLAY") {
                break;
            }
            next = BK_X11;
        } else {
            DISP.lock().kind = BK_X11;
            if let Some((b, w)) = try_backend(BK_X11, title, w_dip, h_dip, flags, x, y) {
                DISP.lock().bk = Some(b);
                return w;
            }
            if forced != 0 || !x11_first {
                break;
            }
            x11_first = false; // X11 first failed: try Wayland too
            next = BK_WAYLAND;
        }
    }
    crate::write_stderr(
        b"error: no display available -- neither a Wayland compositor (WAYLAND_DISPLAY) nor an X11 server \
(DISPLAY) could be opened (use --shot FILE.png for a headless render)\n",
    );
    crate::exit(1)
}

// run f on the backend (None without one).  The backend is taken out of the lock for the
// call: a call that re-enters the plat API from inside a backend method (a refresh or hit
// hook) finds no backend and gets the headless answer instead of a deadlock.
fn with<R>(f: impl FnOnce(&mut dyn Display) -> R) -> Option<R> {
    let mut b = DISP.lock().bk.take()?;
    let r = f(b.as_mut());
    DISP.lock().bk = Some(b);
    Some(r)
}

/// see win_api.rs (Windows) for the semantics of every function
pub fn win_create(title: &str, w_dip: i32, h_dip: i32, flags: u32, owner: i32, x: i32, y: i32) -> i32 {
    let none = {
        let d = DISP.lock();
        d.bk.is_none() && d.kind == BK_NONE
    };
    if none {
        if owner != crate::PW_NOOWNER {
            return -1; // an owned window before any display
        }
        let w = choose(title, w_dip, h_dip, flags, x, y);
        if w >= 0 {
            crate::life_start(); // SIGTERM, sleep and settings for the app
        }
        return w;
    }
    with(|b| b.win_create(title, w_dip, h_dip, flags, owner, x, y)).unwrap_or(-1)
}

pub fn win_destroy(w: i32) {
    with(|b| b.win_destroy(w));
}

pub fn win_show(w: i32, mode: u32) {
    with(|b| b.win_show(w, mode));
}

pub fn win_activate(w: i32) {
    with(|b| b.win_activate(w));
}

pub fn win_enable(w: i32, on: bool) {
    with(|b| b.win_enable(w, on));
}

pub fn win_set_rect(w: i32, x: i32, y: i32, cw: i32, ch: i32) {
    with(|b| b.win_set_rect(w, x, y, cw, ch));
}

pub fn win_get_rect(w: i32) -> Option<[i32; 4]> {
    with(|b| b.win_get_rect(w)).flatten()
}

pub fn win_work_area(w: i32) -> Option<[i32; 4]> {
    with(|b| b.win_work_area(w)).flatten()
}

pub fn win_set_title(w: i32, title: &str) {
    with(|b| b.win_set_title(w, title));
}

pub fn win_state(w: i32) -> u32 {
    with(|b| b.win_state(w)).unwrap_or(0)
}

pub fn win_cmd(w: i32, cmd: u32) {
    with(|b| b.win_cmd(w, cmd));
}

pub fn win_begin_move(w: i32) {
    with(|b| b.win_begin_move(w));
}

pub fn win_begin_resize(w: i32, edges: u32) {
    with(|b| b.win_begin_resize(w, edges));
}

pub fn win_size(w: i32) -> (i32, i32) {
    with(|b| b.win_size(w)).unwrap_or((0, 0))
}

pub fn win_dpi(w: i32) -> i32 {
    with(|b| b.win_dpi(w)).unwrap_or(96)
}

pub fn win_set_min_size(w: i32, w_dip: i32, h_dip: i32) {
    let mut d = DISP.lock();
    match d.bk.as_mut() {
        Some(b) => b.win_set_min_size(w, w_dip, h_dip),
        None if w == 0 => d.min0 = (w_dip, h_dip),
        None => {}
    }
}

/// show the frame's dirty rectangles (x, y, w, h; None = all) in window w; pixels outside
/// them must be what the last present of w showed (linux_x11_present.rs, linux_wl_present.rs)
pub fn win_present(w: i32, f: &Frame, rects: Option<&[[i32; 4]]>) {
    let t0 = crate::time_us();
    with(|b| b.win_present(w, f, rects));
    crate::present_account(t0, true);
}

pub fn win_set_cursor(w: i32, shape: u32) {
    with(|b| b.win_set_cursor(w, shape));
}

/// stored only: the Linux backends need no hit tests
pub fn win_hit_hook(f: Option<fn(i32, i32, i32) -> u32>) {
    DISP.lock().hit = f;
}

/// none: the window manager or compositor moves the windows and does not ask (no snapping)
pub fn win_move_hook(_f: Option<fn(i32, &mut [i32; 4])>) {}

pub fn set_refresh(f: Option<fn()>) {
    let mut d = DISP.lock();
    d.refresh = f;
    if let Some(b) = d.bk.as_mut() {
        b.set_refresh(f);
    }
}

/// pump the display until an event is queued or the deadline passes (0 = poll, u64::MAX =
/// forever); without a display (headless) sleep until the deadline or a wake()
/// [lx_wait_nodisp]
pub fn wait(deadline_us: u64) -> u32 {
    crate::life_poll(); // a SIGTERM: E_SESSION_END
    if crate::evq_count() != 0 {
        return crate::evq_count();
    }
    if let Some(n) = with(|b| b.wait(deadline_us)) {
        crate::life_poll();
        return n.max(crate::evq_count());
    }
    if crate::evq_count() == 0 {
        crate::wait_events(deadline_us, &mut |e| {
            crate::evq_push(*e);
        });
    }
    crate::evq_count()
}

/// show / move / present popup slot `slot` over window `owner` (rect None hides it)  [lx_popup.s]
pub fn popup(slot: u32, owner: i32, rect: Option<[i32; 4]>, f: Option<&Frame>) {
    with(|b| b.popup(slot, owner, rect, f));
}

/// (§6) how the next popup() of slot `slot` hangs at its anchor: anchor x, y, w, h and the
/// popup's natural place x, y (owner px), PA_* kind.  Wayland builds the xdg_positioner from
/// it (the compositor flips / slides the popup to keep it on screen); X11 places it as asked.
pub fn popup_anchor(slot: u32, anchor: [i32; 4], kind: u32, nat: [i32; 2]) {
    with(|b| b.popup_anchor(slot, anchor, kind, nat));
}

/// (§6) where the platform put popup slot `slot` (owner px) when that is not its natural place
/// (Wayland: the compositor's configure); None: where the UI asked, or not known yet
pub fn popup_placed(slot: u32) -> Option<[i32; 2]> {
    with(|b| b.popup_placed(slot)).flatten()
}

pub fn cursor_create(px: &[u8], w: i32, h: i32, hot_x: i32, hot_y: i32, dpi: i32) -> i32 {
    with(|b| b.cursor_create(px, w, h, hot_x, hot_y, dpi)).unwrap_or(-1)
}

pub fn cursor_destroy(id: i32) {
    with(|b| b.cursor_destroy(id));
}

/// the dpi to draw a cursor for a window of this dpi (no display yet: the dpi itself)
pub fn cursor_dpi(dpi: i32) -> i32 {
    with(|b| b.cursor_dpi(dpi)).unwrap_or(dpi)
}

pub fn clip_set(fmt: u32, data: &[u8]) -> bool {
    with(|b| b.clip_set(fmt, data)).unwrap_or(false)
}

/// Publish distinct TEXT/HTML/RTF/DOCX representations of one selection.
pub fn clip_set_many(entries: &[(u32, &[u8])]) -> bool {
    if !crate::clip_many_valid(entries) { return false; }
    with(|b| b.clip_set_many(entries)).unwrap_or(false)
}

pub fn clip_get(fmt: u32) -> Option<Vec<u8>> {
    let data = with(|b| b.clip_get(fmt)).flatten()?;
    if matches!(fmt, crate::CLIP_HTML | crate::CLIP_RTF | crate::CLIP_DOCX) && data.len() > crate::CLIP_RICH_MAX {
        return None;
    }
    if fmt == crate::CLIP_HTML && (data.contains(&0) || core::str::from_utf8(&data).is_err()) {
        return None;
    }
    Some(data)
}

pub fn clip_has(fmt: u32) -> bool {
    with(|b| b.clip_has(fmt)).unwrap_or(false)
}

/// Forward caret position to backends with text-input support.
pub fn ime_pos(x: i32, y: i32, h: i32) { with(|b| b.ime_pos(x,y,h)); }

/// (dnd.rs) the backend drags data out of the app
pub(crate) fn os_drag_start(data: crate::DragData) -> bool {
    with(|b| b.drag_start(data)).unwrap_or(false)
}

// ---------------------------------------------------------------- process level (plat-x11 agent)
// [src/plat/linux/plat_linux.s, x11_xi.s]

/// 0 light, 1 dark, -1 unknown: jfgui_THEME=light|dark wins, else GTK_THEME "Name:dark"
/// means dark  [plat_theme]
pub fn theme() -> i32 {
    if let Some(t) = crate::env("jfgui_THEME") {
        if t == "dark" {
            return 1;
        }
        if t == "light" {
            return 0;
        }
    }
    match crate::env("GTK_THEME") {
        Some(g) if g.ends_with(":dark") => 1,
        _ => -1,
    }
}

/// pens / pen tablets of the display backend (-1 = unknown or no display)  [plat_pen_devices]
pub fn pen_devices() -> i32 {
    with(|b| b.pen_devices()).unwrap_or(-1)
}

/// $HOME (or "/")  [plat_home]
pub fn home() -> Option<String> {
    Some(crate::env("HOME").unwrap_or_else(|| String::from("/")))
}

/// the root "/"  [plat_drives]
pub fn drives() -> Vec<String> {
    vec![String::from("/")]
}

const SPAWN_ARGS: usize = 32; // argv entries at most (argv[0] included)
const SYS_FORK: usize = 57;
const SYS_EXECVE: usize = 59;

/// fork; the child execs /proc/self/exe with argv[0], the arguments (at most 31) and the
/// current environment (exit 127 when that fails) -> 0 / -1  [plat_spawn_self]
pub fn spawn_self(args: &[&str]) -> i32 {
    spawn_exe(b"/proc/self/exe\0", args)
}

/// like spawn_self, but the program file now at this process's path: after an update put a
/// new file there, /proc/self/exe still names the old one ("PATH (deleted)") -> 0 / -1
pub fn restart_self(args: &[&str]) -> i32 {
    let Some(p) = crate::current_exe() else { return -1 };
    let mut path = String::from(p.strip_suffix(" (deleted)").unwrap_or(&p)).into_bytes();
    if path.contains(&0) {
        return -1;
    }
    path.push(0);
    spawn_exe(&path, args)
}

// fork; the child execs exe (NUL-terminated) with argv[0], args and the environment
fn spawn_exe(exe: &[u8], args: &[&str]) -> i32 {
    if args.len() >= SPAWN_ARGS {
        return -1;
    }
    fn z(s: &[u8]) -> Vec<u8> {
        let mut v = s.to_vec();
        v.push(0);
        v
    }
    // every string and pointer array is built before the fork: the child must not allocate
    let a0 = super::args().into_iter().next().unwrap_or_default();
    let mut strs: Vec<Vec<u8>> = Vec::with_capacity(args.len() + 1);
    strs.push(z(a0.as_bytes()));
    for a in args {
        strs.push(z(a.as_bytes()));
    }
    let envs: Vec<Vec<u8>> = {
        let mut g = super::ENV.lock();
        super::env_list(&mut g).iter().map(|e| z(e)).collect()
    };
    let mut argv: Vec<usize> = strs.iter().map(|s| s.as_ptr() as usize).collect();
    argv.push(0);
    let mut envp: Vec<usize> = envs.iter().map(|s| s.as_ptr() as usize).collect();
    envp.push(0);
    // SAFETY: fork has no arguments; the child only calls execve with the NUL-terminated
    // strings and 0-terminated pointer arrays built above, then exit_group.
    unsafe {
        let pid = rt::os::syscall3(SYS_FORK, 0, 0, 0);
        if pid < 0 {
            return -1;
        }
        if pid == 0 {
            rt::os::syscall3(SYS_EXECVE, exe.as_ptr() as usize, argv.as_ptr() as usize, envp.as_ptr() as usize);
            rt::os::syscall3(rt::os::SYS_EXIT_GROUP, 127, 0, 0);
            loop {}
        }
    }
    0
}
