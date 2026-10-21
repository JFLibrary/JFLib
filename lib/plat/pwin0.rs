// pwin0 -- the single-window platform API on top of the window-indexed win_* functions
// (window 0 = the main window), and the "current" window of the calls a UI pass makes
//                                                                      [src/plat/pwin0.s]
//
//   win_use(w)                       the window set_cursor / window_state / window_cmd /
//                                    begin_move / begin_resize refer to (-1 = none)
//   win_current() -> i32             that window
//   window_create(title, w_dip, h_dip, af_flags) -> bool   window 0 (AF_* flags), shown
//   window_create_at(title, w_dip, h_dip, af_flags, x, y, maximized) -> bool   the same at a
//                                    client origin (screen px; PW_DEFAULT: the system's), maximized
//   set_min_size(w_dip, h_dip)       window 0 (before or after create)
//   window_size() -> (w, h)          window 0, client pixels
//   dpi() -> i32   set_title(utf8)   present(frame, rects)     window 0
//   set_cursor(shape)  window_state() -> WST_*  window_cmd(cmd)  begin_move()
//   begin_resize(edges)              the current window
//
// The OS layers (win_api.rs; linux_disp.rs -> Wayland / X11) implement the win_* calls.

use core::sync::atomic::{AtomicI32, Ordering};

use crate::*;

const AF_CUSTOM_TITLEBAR: u32 = 1;
const AF_RESIZABLE: u32 = 2;

static CUR: AtomicI32 = AtomicI32::new(0);

/// the window the single-window per-window calls refer to (-1 = none)
pub fn win_use(w: i32) {
    CUR.store(w, Ordering::Relaxed);
}

/// the window set by win_use (0 by default)
pub fn win_current() -> i32 {
    CUR.load(Ordering::Relaxed)
}

/// the main window (AF_CUSTOM_TITLEBAR: no system caption; AF_RESIZABLE), shown at once
pub fn window_create(title: &str, w_dip: i32, h_dip: i32, af: u32) -> bool {
    window_create_at(title, w_dip, h_dip, af, PW_DEFAULT, PW_DEFAULT, false)
}

/// window_create with the client area's top left at x, y (screen px; PW_DEFAULT: where the
/// system puts it) and maximized (its normal size w_dip x h_dip)
pub fn window_create_at(title: &str, w_dip: i32, h_dip: i32, af: u32, x: i32, y: i32, maximized: bool) -> bool {
    let mut f = 0;
    if af & AF_CUSTOM_TITLEBAR == 0 {
        f = PWF_SYSFRAME;
    }
    if af & AF_RESIZABLE != 0 {
        f |= PWF_RESIZABLE;
    }
    let w = win_create(title, w_dip, h_dip, f, PW_NOOWNER, x, y);
    if w < 0 {
        return false;
    }
    // Windows: maximized before it is shown (no flash of the normal size); the others need
    // the window mapped first
    if maximized && cfg!(windows) {
        win_cmd(w, WCMD_MAXIMIZE);
    }
    win_show(w, PWS_SHOW);
    if maximized && !cfg!(windows) {
        win_cmd(w, WCMD_MAXIMIZE);
    }
    true
}

/// minimum client size of the main window (DIP)
pub fn set_min_size(w_dip: i32, h_dip: i32) {
    win_set_min_size(0, w_dip, h_dip)
}

/// client pixels of the main window
pub fn window_size() -> (i32, i32) {
    win_size(0)
}

/// dpi of the main window (96 = 100 %)
pub fn dpi() -> i32 {
    win_dpi(0)
}

pub fn set_title(title: &str) {
    win_set_title(0, title)
}

/// show dirty rectangles (None = everything) of the main window's framebuffer
pub fn present(f: &Frame, rects: Option<&[[i32; 4]]>) {
    win_present(0, f, rects)
}

/// CUR_* pointer shape of the current window
pub fn set_cursor(shape: u32) {
    win_set_cursor(win_current(), shape)
}

/// WST_* bits of the current window
pub fn window_state() -> u32 {
    win_state(win_current())
}

pub fn window_cmd(cmd: u32) {
    win_cmd(win_current(), cmd)
}

/// let the system move the current window (on a button press)
pub fn begin_move() {
    win_begin_move(win_current())
}

/// let the system resize the current window from an edge (EDGE_* bits)
pub fn begin_resize(edges: u32) {
    win_begin_resize(win_current(), edges)
}

/// wait(deadline) with a redraw closure for a modal move / size loop (Windows); the Linux
/// backends have no modal loop: there it is wait()  [main agent]
#[cfg(not(windows))]
pub fn wait_refresh(deadline_us: u64, _refresh: &mut dyn FnMut()) -> u32 {
    wait(deadline_us)
}
