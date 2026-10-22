// linux_wl -- the Wayland display backend behind linux_disp::Display
//                                     [src/plat/linux/wl_window.s, wl_wire.s, wl_present.s, ...]
//
//   wl_connect() -> the backend, None when no compositor answers ($WAYLAND_DISPLAY in
//        $XDG_RUNTIME_DIR, /run/user/UID, /mnt/wslg/runtime-dir)
//   test hooks for linuxtest (synthetic events, no compositor needed): wl_test_event,
//        wl_test_setup, wl_input_service, wl_cursor_image, wl_cursor_render, wl_test_protoerr,
//        wl_test_roundtrip (a compositor or tools/wlproxy/wlinject.py)
//   xkb_* (linux_xkb.rs): the keymap of the Wayland keyboard, also used by X11
//
// The modules follow the assembly files:
//   linux_wl_wire.rs     connection, object table, marshalling, fd passing, dispatch,
//                        roundtrips, fatal errors, the jfgui_WLDEBUG trace   [wl_wire.s]
//   linux_wl_proto.rs    interface descriptors (event tables) and requests    [wl_proto.s]
//   linux_wl_window.rs   globals, xdg toplevels, configure, scale, commands, the wait loop
//                                                                             [wl_window.s]
//   linux_wl_present.rs  wl_shm buffers, damage, frame callbacks            [wl_present.s]
//   linux_wl_input.rs    wl_seat pointer / keyboard, key repeat               [wl_input.s]
//   linux_wl_tablet.rs   zwp_tablet_v2 pens                                  [wl_tablet.s]
//   linux_wl_popup.rs    xdg_popup windows                                    [wl_popup.s]
//   linux_wl_clip.rs     wl_data_device: clipboard both ways, drag and drop in and out
//                                                                             [wl_clip.s]
//   linux_wl_cursor.rs   cursor-shape protocol, own cursor images, custom cursors [wl_cursor.s]
//
// State: the assembly's globals are one record (Wl) owned by this module, created on the
// first use (the synthetic tests use it without a connection).  Every entry point (the
// Display methods, the hooks) borrows it once for its whole call and passes `&mut Wl` down
// to the event handlers; a nested borrow (re-entry) panics instead of aliasing.  Only the
// UI thread calls in; wake() from other threads goes through the eventfd of linux_thread.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{Display, Frame, PPW_MAX, PW_MAX};

/// (screen.rs) the outputs as monitors (empty without a compositor connection)
pub(crate) fn wl_monitors() -> Vec<crate::screen::Monitor> {
    with(|wl| wl.monitors())
}

#[path = "linux_wl_clip.rs"]
mod clip;
#[path = "linux_wl_cursor.rs"]
mod cursor;
#[path = "linux_wl_ime.rs"]
mod ime;
#[path = "linux_wl_input.rs"]
mod input;
#[path = "linux_wl_popup.rs"]
mod popup;
#[path = "linux_wl_present.rs"]
mod present;
#[path = "linux_wl_proto.rs"]
mod proto;
#[path = "linux_wl_tablet.rs"]
mod tablet;
#[path = "linux_wl_window.rs"]
mod window;
#[path = "linux_wl_wire.rs"]
mod wire;
#[path = "linux_xkb.rs"]
mod xkb;
pub use xkb::*;

/// the protocol state of the backend (the assembly's g_wl_* / g_wlw / g_wp / ... globals)
pub(crate) struct Wl {
    pub w: wire::Wire,
    pub g: window::Globals,
    pub outs: [window::Out; window::OUT_MAX],
    pub win: [window::WinRec; PW_MAX],
    pub wp: [present::Wp; PW_MAX],
    pub inp: input::Input,
    pub tab: tablet::Tablet,
    pub wq: [popup::Wq; PPW_MAX as usize],
    pub clip: clip::Clip,
    pub cur: cursor::Cursor,
    pub ime: ime::Ime,
}

impl Wl {
    fn new() -> Wl {
        Wl {
            w: wire::Wire::new(),
            g: window::Globals::default(),
            outs: Default::default(),
            win: Default::default(),
            wp: Default::default(),
            inp: input::Input::new(),
            tab: tablet::Tablet::default(),
            wq: Default::default(),
            clip: clip::Clip::default(),
            cur: cursor::Cursor::default(),
            ime: Default::default(),
        }
    }
}

struct Cell {
    v: UnsafeCell<Option<Box<Wl>>>,
    busy: AtomicBool,
}

// SAFETY: `busy` hands out the one &mut Wl at a time (a second borrow panics).
unsafe impl Sync for Cell {}

static WL: Cell = Cell { v: UnsafeCell::new(None), busy: AtomicBool::new(false) };

/// run f on the backend state (created on the first call)
fn with<R>(f: impl FnOnce(&mut Wl) -> R) -> R {
    if WL.busy.swap(true, Ordering::Acquire) {
        panic!("wayland: backend state re-entered");
    }
    // SAFETY: the busy flag is ours until the store below: nobody else has the state.
    let wl = unsafe { &mut *WL.v.get() }.get_or_insert_with(|| Box::new(Wl::new()));
    let r = f(wl);
    WL.busy.store(false, Ordering::Release);
    r
}

/// the backend object (a handle: the state lives in WL)
struct WlBackend;

/// connect to the compositor -> the backend, None when there is no Wayland display (quiet
/// unless jfgui_BACKEND=wayland forces it)  [wl_connect]
pub fn wl_connect() -> Option<Box<dyn Display>> {
    if with(|wl| wl.connect()) { Some(Box::new(WlBackend)) } else { None }
}

impl Display for WlBackend {
    fn ime_pos(&mut self,x:i32,y:i32,h:i32) { with(|wl| wl.ime_pos(x,y,h)); }
    fn win_create(&mut self, title: &str, w_dip: i32, h_dip: i32, flags: u32, owner: i32, x: i32, y: i32) -> i32 {
        with(|wl| wl.win_create(title, w_dip, h_dip, flags, owner, x, y))
    }
    fn win_destroy(&mut self, w: i32) {
        with(|wl| wl.win_destroy(w))
    }
    fn win_show(&mut self, w: i32, mode: u32) {
        with(|wl| wl.win_show(w, mode))
    }
    fn win_activate(&mut self, _w: i32) {}
    fn win_enable(&mut self, _w: i32, _on: bool) {}
    fn win_set_rect(&mut self, w: i32, x: i32, y: i32, cw: i32, ch: i32) {
        with(|wl| wl.win_set_rect(w, x, y, cw, ch))
    }
    fn win_get_rect(&mut self, w: i32) -> Option<[i32; 4]> {
        with(|wl| wl.win_get_rect(w))
    }
    fn win_work_area(&mut self, w: i32) -> Option<[i32; 4]> {
        with(|wl| wl.win_work_area(w))
    }
    fn win_set_title(&mut self, w: i32, title: &str) {
        with(|wl| wl.win_set_title(w, title.as_bytes()))
    }
    fn win_state(&mut self, w: i32) -> u32 {
        with(|wl| wl.win_state(w))
    }
    fn win_cmd(&mut self, w: i32, cmd: u32) {
        with(|wl| wl.win_cmd(w, cmd))
    }
    fn win_begin_move(&mut self, w: i32) {
        with(|wl| wl.win_begin_move(w))
    }
    fn win_begin_resize(&mut self, w: i32, edges: u32) {
        with(|wl| wl.win_begin_resize(w, edges))
    }
    fn win_size(&mut self, w: i32) -> (i32, i32) {
        with(|wl| wl.win_size(w))
    }
    fn win_dpi(&mut self, w: i32) -> i32 {
        with(|wl| wl.win_dpi(w))
    }
    fn win_set_min_size(&mut self, w: i32, w_dip: i32, h_dip: i32) {
        with(|wl| wl.win_set_min_size(w, w_dip, h_dip))
    }
    fn win_present(&mut self, w: i32, f: &Frame, rects: Option<&[[i32; 4]]>) {
        with(|wl| wl.win_present(w, f, rects))
    }
    fn win_set_cursor(&mut self, w: i32, shape: u32) {
        with(|wl| wl.win_set_cursor(w, shape))
    }
    fn set_refresh(&mut self, f: Option<fn()>) {
        with(|wl| wl.g.refresh = f)
    }
    fn wait(&mut self, deadline_us: u64) -> u32 {
        with(|wl| wl.wait(deadline_us))
    }
    fn popup(&mut self, slot: u32, owner: i32, rect: Option<[i32; 4]>, f: Option<&Frame>) {
        with(|wl| wl.popup(slot, owner, rect, f))
    }
    fn popup_anchor(&mut self, slot: u32, anchor: [i32; 4], kind: u32, nat: [i32; 2]) {
        with(|wl| wl.popup_anchor(slot, anchor, kind, nat))
    }
    fn popup_placed(&mut self, slot: u32) -> Option<[i32; 2]> {
        with(|wl| wl.popup_placed(slot))
    }
    fn cursor_create(&mut self, px: &[u8], w: i32, h: i32, hot_x: i32, hot_y: i32, dpi: i32) -> i32 {
        with(|wl| wl.cursor_create(px, w, h, hot_x, hot_y, dpi))
    }
    fn cursor_destroy(&mut self, id: i32) {
        with(|wl| wl.cursor_destroy(id))
    }
    fn cursor_dpi(&mut self, dpi: i32) -> i32 {
        cursor::cursor_dpi(dpi)
    }
    fn clip_set(&mut self, fmt: u32, data: &[u8]) -> bool {
        with(|wl| wl.clip_set(fmt, data))
    }
    fn clip_set_many(&mut self, entries: &[(u32, &[u8])]) -> bool {
        with(|wl| wl.clip_set_many(entries))
    }
    fn clip_get(&mut self, fmt: u32) -> Option<Vec<u8>> {
        with(|wl| wl.clip_get(fmt))
    }
    fn clip_has(&mut self, fmt: u32) -> bool {
        with(|wl| wl.clip_has(fmt))
    }
    fn drag_start(&mut self, data: crate::DragData) -> bool {
        with(|wl| wl.drag_start(data))
    }
    /// tablets announced, -1 = no tablet protocol  [wl_pen_devices]
    fn pen_devices(&mut self) -> i32 {
        with(|wl| wl.pen_devices())
    }
}

// ------------------------------------------------------------------ test hooks (linuxtest)

/// call the handler of event `event` ("enter", "key", ...) of interface `iface`
/// ("wl_pointer", "wl_keyboard", ...) with decoded arguments (object id 1, udata 0), as
/// linuxtest --test synth does -> false when there is no such event  [hcall]
pub fn wl_test_event(iface: &str, event: &str, args: &[u64]) -> bool {
    with(|wl| wl.test_event(iface, event, args))
}

/// (main agent) wl_test_event with the handler's udata (a tablet tool's slot ...), as
/// plattest --test pen does  [tcall in apps/plattest/src/pen.s]
pub fn wl_test_event_u(iface: &str, event: &str, args: &[u64], udata: u64) -> bool {
    with(|wl| wl.test_event_u(iface, event, args, udata))
}

/// (main agent) the pointer scale while there is no window (g_wl_s120, 120ths) alone
pub fn wl_test_scale(s120: i32) {
    with(|wl| wl.inp.s120_test = s120)
}

/// the pointer scale while there is no window (g_wl_s120, 120ths) and the seat version
/// the handlers assume (g_wl_seat_ver)
pub fn wl_test_setup(s120: i32, seat_ver: u32) {
    with(|wl| {
        wl.inp.s120_test = s120;
        wl.g.seat_ver = seat_ver;
    })
}

/// key repeat at `now` (time_us) -> the next repeat deadline, u64::MAX = none
/// [wl_input_service]
pub fn wl_input_service(now: u64) -> u64 {
    with(|wl| wl.input_service(now))
}

/// the built-in cursor image of a CUR_* shape: (width, height, hot x, hot y, rows of 'X'
/// black / 'o' white / '.' transparent)  [wl_cursor_image]
pub fn wl_cursor_image(shape: u32) -> Option<(u32, u32, u32, u32, &'static [u8])> {
    cursor::image(shape).map(|a| (a.w as u32, a.h as u32, a.hx as u32, a.hy as u32, a.px))
}

/// expand a cursor image into ARGB pixels (S x S blocks, rows of width * S pixels)
/// [wl_cursor_render]
pub fn wl_cursor_render(dst: &mut [u32], shape: u32, s: u32) {
    if let Some(a) = cursor::image(shape) {
        cursor::render(dst, a, s);
    }
}

/// a wl_display.sync round trip: the events that arrive before its answer are dispatched
/// first (linuxtest --test burst: a burst of keys while it waits) -> 0 ok / -1 timeout
pub fn wl_test_roundtrip() -> i32 {
    with(|wl| wl.roundtrip())
}

/// attach a buffer id that does not exist to the main window's surface: the compositor
/// answers with wl_display.error, which ends the process (linuxtest --test protoerr)
pub fn wl_test_protoerr() {
    with(|wl| {
        let s = wl.win[0].surf;
        wl.w.send(s, &proto::SURFACE_ATTACH, &[wire::A::O(1000), wire::A::I(0), wire::A::I(0)]);
    })
}
