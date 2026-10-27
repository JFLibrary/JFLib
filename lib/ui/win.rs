// win -- multi-window runtime: tool windows and dialog windows built by the same App::frame
// as the main window, each with its own UI state                       [src/ui/ui_win.s]
//
// App-facing:
//   window_begin(id, title, open, flags, rect) -> true: emit the content now (always pair
//        with window_end; see WF_* / WinRect; open / rect: shared with the library)
//   window_end()
//   window_place(id, &rect)              move / resize (copied into the window's WINRECT)
//   window_rect(id) -> Option<WinRect>   current placement (None = unknown window)
//   window_state(id) -> UWS_*            UWS_OPEN | UWS_VISIBLE | UWS_ACTIVE | UWS_MINIMIZED
//   window_activate(id)                  give it the keyboard (raises it)
//   window_snap(id, area, dist)          its edges snap to the area's while the user moves it
//   window_destroy(id)                   free its state and buffers (created again when shown)
//   window_current() -> id               id of the window being built (0 = main)
//   window_modal() -> id                 id of the topmost modal window or 0
//   sync_hook_set(f)                     f() -> true while background jobs run (`sync`)
// Platform: hit_flags_win(win, x, y), win_hooks_set(on) (OS windows through the plat_win_*
// API, the asm's uiw_plat_hooks table), uiw_hit_pw (the platform's hit hook), uiw_pw /
// uiw_from_pw (UI index <-> platform window).
// Runtime (runtime.rs, ctx.rs, chrome.rs, auto.rs, drop.rs): the uiw_* functions below.
//
// How it works: the per-window state of every module lives in WinParts (lib.rs); `Ui::wp`
// is the live window's.  uiw_use(i) swaps the live parts into the record of the window
// that was live and window i's parts out of its record (core::mem::swap of a Box), so every
// widget, popup, dialog and the dirty-region code work unchanged in whichever window is
// live.  Outside of the passes and of the event dispatch the main window (index 0) is live.
// Platform operations are never done inside window_begin: they are recorded in Rec::want
// and done by uiw_settle after the frame settled (headless runs: none -- the windows only
// exist as framebuffers).
//
// The asm keeps the app's `&open` and `&rect` pointers in the window record and writes
// through them later (close button, Esc, placement by the user); here the app passes a
// Shared<T> (Rc<Cell<T>>) and the record keeps a clone of it.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;

use super::consts::*;
use super::ctx::Hit;
use super::layout::px2dip_at;
use super::{Ui, WinParts};
use gfx::{Surface, FMT_XRGB};
use plat::Event;
use rt::sync::SpinLock;

// fixed: a window index is a bit in u32 masks and Event::win (WIN_MAX, plat's PW_MAX); the
// stacks below are nesting depths of window_begin, which real apps keep at two or three
const WSTACK_MAX: usize = 16; // window_begin records inside each other (entered or not)
const NEST_MAX: u32 = 4; // windows entered inside each other (warns)
const DEF_W: i32 = 280;
const DEF_H: i32 = 400;
const WSE_ENTERED: u32 = 0x10000; // stack entry: prev | index << 8 | WSE_ENTERED
const TITLE_MAX: usize = 256;
const SIZE_MAX_DIP: i32 = 8192; // largest client width / height of a window (DIP)
const FWDQ_MAX: usize = super::ctx::KEYQ_MAX; // (U3) keys a borrowing tool window hands on per pass (all)
const FWDT_MAX: usize = super::ctx::TEXTQ_MAX; // (U3) typed codepoints handed on per pass (all)
// (U3) popup slots (items outside their window): fixed, plat has PPW_MAX popup windows;
// more open at once are drawn clipped to their window (limit_hit)
const EXT_MAX: usize = 4;
const POPUP_ITEMS: u32 = 7; // (U3) popup_ext items: 6 popups + the tooltip

/// chrome ids of window i: WCH_BASE | i << 4 | part
pub(crate) const WCH_BASE: u32 = 0xFFFF_7800;
pub(crate) const WCH_CLOSE: u32 = 1; // close button
pub(crate) const WCH_CONTENT: u32 = 2; // content box
pub(crate) const WCH_FOOTER: u32 = 3; // dialog footer (dialog_buttons in a WF_DIALOG window)
pub(crate) const WCH_BTN: u32 = 4; // footer buttons 4, 5, 6
/// root box id of window i (> 0): WROOT_BASE | i
pub(crate) const WROOT_BASE: u32 = 0xFFFF_7F00;

// window record state bits (UWI_*)
pub(crate) const UWI_OPEN: u32 = 1; // *open was set at the last window_begin
pub(crate) const UWI_VISIBLE: u32 = 2; // shown (built and rendered every frame)
pub(crate) const UWI_CREATED: u32 = 16; // its WinParts and framebuffer exist
pub(crate) const UWI_SEEN: u32 = 32; // window_begin ran in the current pass
pub(crate) const UWI_SHOWN: u32 = 64; // was shown before (placement done)
pub(crate) const UWI_NEWSHOW: u32 = 128; // became visible in this frame
pub(crate) const UWI_BUILDING: u32 = 256; // between window_begin and window_end
pub(crate) const UWI_FRESH: u32 = 512; // shown again: its focus starts empty
pub(crate) const UWI_DEFPOS: u32 = 1024; // placed by the default rule (re-centred while its height settles)

// pending platform operations (UWW_*)
const UWW_SHOW: u32 = 1;
const UWW_HIDE: u32 = 2;
const UWW_RECT: u32 = 4;
const UWW_TITLE: u32 = 8;
const UWW_ACTIVATE: u32 = 16;
const UWW_CREATE: u32 = 64;

/// a value the app shares with the library: the asm passes its address and the library
/// keeps it (a window's `open` flag, its WINRECT, a content dialog's `open` flag)
pub struct Shared<T: Copy>(Rc<Cell<T>>);

impl<T: Copy> Shared<T> {
    pub fn new(v: T) -> Shared<T> {
        Shared(Rc::new(Cell::new(v)))
    }
    pub fn get(&self) -> T {
        self.0.get()
    }
    pub fn set(&self, v: T) {
        self.0.set(v)
    }
}

impl<T: Copy> Clone for Shared<T> {
    fn clone(&self) -> Shared<T> {
        Shared(self.0.clone())
    }
}

impl<T: Copy + Default> Default for Shared<T> {
    fn default() -> Shared<T> {
        Shared::new(T::default())
    }
}

/// a window record (UW_*); index 0 = the main window
pub(crate) struct Rec {
    /// app id (0 for the main window; 0 in a free record)
    pub(crate) id: u32,
    /// WF_*
    pub(crate) flags: u32,
    /// UWI_*
    pub(crate) state: u32,
    /// owner window index
    pub(crate) owner: u32,
    /// frame of the last window_begin
    pub(crate) seen: u32,
    /// stacking order (larger = above)
    pub(crate) z: u32,
    /// the app's open flag (None = always open)
    pub(crate) openp: Option<Shared<i32>>,
    /// WINRECT: the app's, or `own` when the app passes none
    pub(crate) rect: Shared<WinRect>,
    pub(crate) own: Shared<WinRect>,
    /// the window's per-window state while another window is live (None: live, or no
    /// state yet)
    pub(crate) save: Option<Box<WinParts>>,
    /// WINRECT as last applied
    pub(crate) applied: WinRect,
    /// UWW_* platform operations not done yet
    pub(crate) want: u32,
    /// library title bar height in DIP (0 = none)
    pub(crate) tbh: i32,
    /// natural content height in DIP (WF_AUTOHEIGHT)
    pub(crate) autoh: i32,
    /// screen position (px) reported by the platform
    pub(crate) sx: i32,
    pub(crate) sy: i32,
    /// the title copy (a change is passed on to the platform)
    pub(crate) title: String,
    /// window_snap: area x, y, w, h (DIP of the owner's client area), distance (DIP; 0 = off)
    pub(crate) snap: [i32; 5],
}

impl Default for Rec {
    fn default() -> Rec {
        let own = Shared::default();
        Rec {
            id: 0,
            flags: 0,
            state: 0,
            owner: 0,
            seen: 0,
            z: 0,
            openp: None,
            rect: own.clone(),
            own,
            save: None,
            applied: WinRect::default(),
            want: 0,
            tbh: 0,
            autoh: 0,
            sx: 0,
            sy: 0,
            title: String::new(),
            snap: [0; 5],
        }
    }
}

// (U3) a popup slot: an item of the owner's popups outside its framebuffer
#[derive(Default)]
struct Ex {
    /// owner window index + 1 (0 = free)
    owner: u32,
    /// popup_ext item
    item: u32,
    /// x, y, w, h (px, owner coordinates)
    rect: [i32; 4],
    /// used by the owner's last render
    used: bool,
    /// the platform shows it
    shown: bool,
    /// (§6) its natural place (px, owner) when it was shown -- popup_placed answers for it --
    /// and that place as the popup code computed it (the key of uiw_popup_nudge)
    nat: [i32; 2],
    key: [i32; 2],
    sf: Surface,
}

/// state shared by all windows
pub(crate) struct S {
    pub(crate) recs: [Rec; WIN_MAX],
    /// window with the keyboard
    pub(crate) active: u32,
    /// topmost modal window (0 = none)
    pub(crate) modal: u32,
    /// bit mask of inert windows
    pub(crate) inert: u32,
    /// window under the pointer (-1 = none)
    pub(crate) ptr: i32,
    /// windows that got input since the last pass
    pub(crate) edges: u32,
    /// bit mask of window records in use (not the main window)
    pub(crate) used: u32,
    /// the live window takes no accelerators (a dialog window's pass end)
    pub(crate) noaccel: bool,
    stack: [u32; WSTACK_MAX],
    sp: u32,
    over: u32,
    /// OS windows (the runtime runs with a display: the asm's uiw_plat_hooks are set)
    hooks: bool,
    synchook: Option<fn() -> bool>,
    zc: u32,
    // (U3) key forwarding of tool windows (kfocus: the window that borrows the keys + 1;
    // kbnav.rs sets it for F6 / Ctrl+F6)
    pub(crate) kfocus: u32,
    fwd: u32,
    fwd_to: u32,
    fwdq: Vec<u32>,
    fwdt: Vec<u32>,
    // (U3) popup slots
    ex: [Ex; EXT_MAX],
    /// (§6) headless: the virtual screen (the script's `screen`): x, y, w, h in DIP of the main
    /// window's client area (w 0: none, the main window's client area holds the popups)
    pub(crate) screen: [i32; 4],
    /// OS windows disabled by uiw_enable_sync (bit = index)
    disabled: u32,
    /// uiw_follow_owner nesting
    follow: u32,
    /// OS windows placed at their WINRECT position (bit = index)
    placed: u32,
    /// uiw_owner_origin: the position is known
    origin_ok: bool,
    /// UI window -> platform window (0xFF none) and back
    u2p: [u8; WIN_MAX],
    p2u: [u8; WIN_MAX],
    /// the framebuffer of a window without state (uiw_var's zeroes)
    zero_fb: Surface,
    /// winstate.rs: saved tool windows window_begin has not met yet; the main window's last
    /// placement while it was neither maximized nor minimized
    pub(crate) saved: Vec<super::winstate::Saved>,
    pub(crate) norm: super::winstate::MainState,
    pub(crate) cand: Option<super::winstate::MainState>,
}

impl Default for S {
    fn default() -> S {
        S {
            recs: Default::default(),
            active: 0,
            modal: 0,
            inert: 0,
            ptr: 0,
            edges: 0,
            used: 0,
            noaccel: false,
            stack: [0; WSTACK_MAX],
            sp: 0,
            over: 0,
            hooks: false,
            synchook: None,
            zc: 0,
            kfocus: 0,
            fwd: 0,
            fwd_to: 0,
            fwdq: Vec::new(),
            fwdt: Vec::new(),
            ex: Default::default(),
            screen: [0; 4],
            disabled: 0,
            follow: 0,
            placed: 0,
            origin_ok: false,
            u2p: [0xFF; WIN_MAX],
            p2u: [0xFF; WIN_MAX],
            zero_fb: Surface::new(0, 0, FMT_XRGB),
            saved: Vec::new(),
            norm: Default::default(),
            cand: None,
        }
    }
}

// ------------------------------------------------------------------ the platform's hit hook

// what the platform's non-client hit tests (WM_NCHITTEST, click activation) see of the UI:
// they run inside window procedures without the UI context, so every window's hit list of
// its last pass is published at the end of the pass (uiw_publish_hits)
struct Published {
    hits: [Vec<Hit>; WIN_MAX],
    modal: [u32; WIN_MAX],
    /// windows that exist and are shown (uiw_drawable)
    drawable: u32,
    p2u: [u8; WIN_MAX],
    /// right-to-left: the width each window is mirrored across (0 = not mirrored)
    mirror: [i32; WIN_MAX],
    /// drag and drop (dnd.rs): the drop targets of each window's last pass, what a window
    /// takes outside them, the kinds of the app's own drag while the platform carries it
    zones: [Vec<super::dnd::Zone>; WIN_MAX],
    accept: u32,
    own: u32,
    /// window_snap for the move hook (uiw_moving_pw): owner index, area x, y, w, h (px from
    /// the owner's platform client origin), distance (px; 0 = off)
    snap: [[i32; 6]; WIN_MAX],
}

static PUBLISHED: SpinLock<Published> = SpinLock::new(Published {
    hits: [const { Vec::new() }; WIN_MAX],
    modal: [0; WIN_MAX],
    drawable: 1,
    p2u: [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
    mirror: [0; WIN_MAX],
    zones: [const { Vec::new() }; WIN_MAX],
    accept: plat::DT_FILES,
    own: 0,
    snap: [[0; 6]; WIN_MAX],
});

/// uiw_hit_pw(pw, x, y) -> HB_* flags at a client point (px) of platform window pw in the
/// last pass (the platform's non-client hit tests and click activation)
pub(crate) fn uiw_hit_pw(pw: i32, x: i32, y: i32) -> u32 {
    if !(0..WIN_MAX as i32).contains(&pw) {
        return 0;
    }
    let p = PUBLISHED.lock();
    let i = p.p2u[pw as usize];
    if i == 0xFF || (p.drawable >> i) & 1 == 0 {
        return 0;
    }
    let i = i as usize;
    // the hits are logical: a mirrored window's point too
    let x = if p.mirror[i] > 0 { p.mirror[i] - 1 - x } else { x };
    super::ctx::hit_find_list(&p.hits[i], p.modal[i], x, y).1
}

/// uiw_drag_pw(pw, x, y, types) -> (accepted DT_* kinds, DE_* effects) of a drag offering
/// `types` at a client point (px) of platform window pw (the platform's drag hook, dnd.rs):
/// the drop target there, else what the windows take (drop_accept)
pub(crate) fn uiw_drag_pw(pw: i32, x: i32, y: i32, types: u32) -> (u32, u32) {
    if !(0..WIN_MAX as i32).contains(&pw) {
        return (0, 0);
    }
    let p = PUBLISHED.lock();
    let i = p.p2u[pw as usize];
    if i == 0xFF || (p.drawable >> i) & 1 == 0 {
        return (0, 0);
    }
    let i = i as usize;
    let x = if p.mirror[i] > 0 { p.mirror[i] - 1 - x } else { x }; // logical
    let offered = types & plat::DT_STD;
    let all = offered | p.own; // the app's own drag: its own kinds too
    let t = match super::dnd::zone_find(&p.zones[i], &p.hits[i], p.modal[i], x, y, all) {
        Some(z) => z.types & all,
        None => p.accept & all,
    };
    // the platform knows the standard kinds: a target of the app's own kind takes them all
    let t = if t != 0 && t & offered == 0 { offered } else { t & offered };
    (t, if t != 0 { plat::DE_ALL } else { 0 })
}

/// (dnd.rs) what the drag hook needs besides the targets: the kinds the windows take, the
/// kinds of the app's own drag while the platform carries it (0 = none)
pub(crate) fn uiw_publish_dnd(accept: u32, own: u32) {
    let mut p = PUBLISHED.lock();
    p.accept = accept;
    p.own = own;
}

/// uiw_moving_pw(pw, &rect): (the platform's move hook, inside its modal move loop) the user
/// drags platform window pw to rect (x, y, w, h of its client area, screen px): x, y snap to
/// the area window_snap gave
pub(crate) fn uiw_moving_pw(pw: i32, r: &mut [i32; 4]) {
    if !(0..WIN_MAX as i32).contains(&pw) {
        return;
    }
    let (s, opw) = {
        let p = PUBLISHED.lock();
        let i = p.p2u[pw as usize];
        if i == 0xFF {
            return;
        }
        let s = p.snap[i as usize];
        if s[5] == 0 {
            return;
        }
        (s, p.p2u.iter().position(|&u| u as i32 == s[0]))
    };
    let Some(o) = opw.and_then(|o| plat::win_get_rect(o as i32)) else { return };
    let a = [o[0] + s[1], o[1] + s[2], s[3], s[4]];
    (r[0], r[1]) = snap_to(r[0], r[1], r[2], r[3], a, s[5]);
}

/// snap_to(x, y, w, h, area, dist) -> (x, y): a rectangle at x, y moved so that its edges sit
/// on the area's edges they are at most dist away from (left / right, top / bottom: the
/// nearer one); an axis snaps only while the rectangle overlaps the area (with dist) on the
/// other axis.  Any one unit.
fn snap_to(x: i32, y: i32, w: i32, h: i32, a: [i32; 4], d: i32) -> (i32, i32) {
    let axis = |p: i32, s: i32, lo: i32, len: i32| {
        let near = (p - lo).abs(); // leading edge to the area's start
        let far = (p + s - lo - len).abs(); // trailing edge to its end
        if near <= d && near <= far {
            lo
        } else if far <= d {
            lo + len - s
        } else {
            p
        }
    };
    let mut r = (x, y);
    if y < a[1] + a[3] + d && y + h > a[1] - d {
        r.0 = axis(x, w, a[0], a[2]);
    }
    if x < a[0] + a[2] + d && x + w > a[0] - d {
        r.1 = axis(y, h, a[1], a[3]);
    }
    r
}

/// dip2px_at(dip, dpi) -> round(dip * dpi / 96)
fn dip2px_at(dip: i32, dpi: i32) -> i32 {
    let v = dip.wrapping_mul(dpi);
    if v < 0 { (v - 48) / 96 } else { (v + 48) / 96 }
}

// blit_copy(dst, src, x, y): src at x, y, clipped (gfx_blit_copy)
fn blit_copy(dst: &mut Surface, src: &Surface, x: i32, y: i32) {
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + src.w).min(dst.w);
    let y1 = (y + src.h).min(dst.h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let n = (x1 - x0) as usize * 4;
    for yy in y0..y1 {
        let d = yy as usize * dst.stride as usize + x0 as usize * 4;
        let s = (yy - y) as usize * src.stride as usize + (x0 - x) as usize * 4;
        dst.px[d..d + n].copy_from_slice(&src.px[s..s + n]);
    }
}

impl Ui {
    // ------------------------------------------------------------------ records

    /// uiw_index(id) -> index of the window with app id `id` (0 -> 0), -1 if none
    pub(crate) fn uiw_index(&self, id: u32) -> i32 {
        if id == 0 {
            return 0;
        }
        (1..WIN_MAX).find(|&i| self.m.win.recs[i].id == id).map_or(-1, |i| i as i32)
    }

    /// uiw_rec(i).id: the app id of window i (0 = the main window)
    pub(crate) fn uiw_rec_id(&self, i: u32) -> u32 {
        self.m.win.recs[(i as usize) & (WIN_MAX - 1)].id
    }

    /// uiw_title(i): the window's title copy
    pub(crate) fn uiw_title(&self, i: u32) -> &str {
        &self.m.win.recs[(i as usize) & (WIN_MAX - 1)].title
    }

    /// any window record besides the main window in use (g_uiw_used != 0)
    pub(crate) fn uiw_any_used(&self) -> bool {
        self.m.win.used != 0
    }

    /// uiw_init: window records, window 0
    pub(crate) fn uiw_init(&mut self) {
        let w = &mut self.m.win;
        w.recs[0].state = UWI_OPEN | UWI_VISIBLE | UWI_CREATED | UWI_SHOWN;
        let r = &w.recs[0];
        r.rect.set(WinRect { flags: WRF_ABS, ..r.rect.get() });
        self.wp.lay.root_id = 1;
        // platform windows: only the main window
        w.u2p = [0xFF; WIN_MAX];
        w.p2u = [0xFF; WIN_MAX];
        w.u2p[0] = 0;
        w.p2u[0] = 0;
        self.uiw_cur = 0;
        w.active = 0;
        w.modal = 0;
        w.ptr = 0;
    }

    /// uiw_use(i): make window i live: the live parts go to the record of the window that
    /// was live, window i's parts become live
    pub(crate) fn uiw_use(&mut self, i: u32) {
        let i = (i as usize) & (WIN_MAX - 1);
        let cur = self.uiw_cur as usize & (WIN_MAX - 1);
        if i == cur {
            return;
        }
        let Some(mut b) = self.m.win.recs[i].save.take() else {
            return; // no such window: stay
        };
        core::mem::swap(&mut self.wp, &mut *b);
        self.m.win.recs[cur].save = Some(b);
        self.uiw_cur = i as u32;
        // per-window platform calls of this pass (cursor, state, move / resize) go to it
        plat::win_use(self.uiw_pw(i as u32));
    }

    /// uiw_var: the per-window parts of window i (the live ones if i is live; None for a
    /// window without state)
    pub(crate) fn uiw_parts(&self, i: u32) -> Option<&WinParts> {
        let i = (i as usize) & (WIN_MAX - 1);
        if i == self.uiw_cur as usize {
            Some(&self.wp)
        } else {
            self.m.win.recs[i].save.as_deref()
        }
    }

    pub(crate) fn uiw_parts_mut(&mut self, i: u32) -> Option<&mut WinParts> {
        let i = (i as usize) & (WIN_MAX - 1);
        if i == self.uiw_cur as usize {
            Some(&mut self.wp)
        } else {
            self.m.win.recs[i].save.as_deref_mut()
        }
    }

    /// the dpi of window i (uiw_var(i, &g_ui_dpi); 0 for a window without state)
    pub(crate) fn win_dpi(&self, i: u32) -> i32 {
        self.uiw_parts(i).map_or(0, |p| p.rt.dpi)
    }

    /// the framebuffer of window i (uiw_var(i, &g_fb))
    pub(crate) fn win_fb(&self, i: u32) -> &Surface {
        match self.uiw_parts(i) {
            Some(p) => &p.rt.fb,
            None => &self.m.win.zero_fb,
        }
    }

    // the focus of window i (uiw_var(i, &g_focus))
    fn win_focus(&self, i: u32) -> u32 {
        self.uiw_parts(i).map_or(0, |p| p.ctx.focus)
    }

    /// uiw_take_newshow(i): the window became visible in this frame (clears the bit)
    pub(crate) fn uiw_take_newshow(&mut self, i: u32) -> bool {
        let r = &mut self.m.win.recs[(i as usize) & (WIN_MAX - 1)];
        let n = r.state & UWI_NEWSHOW != 0;
        r.state &= !UWI_NEWSHOW;
        n
    }

    // ------------------------------------------------------------------ per-pass

    /// uiw_frame_begin: start of a pass, no window begun yet
    pub(crate) fn uiw_frame_begin(&mut self) {
        let w = &mut self.m.win;
        if w.used != 0 {
            for r in w.recs.iter_mut() {
                r.state &= !UWI_SEEN;
            }
        }
        w.sp = 0;
        w.over = 0;
    }

    /// uiw_frame_end: end of a pass over all windows: modal window and inert windows
    pub(crate) fn uiw_frame_end(&mut self) {
        if self.m.win.used == 0 {
            return; // the main window only
        }
        if self.uiw_modal_update() {
            self.repass = true;
        }
    }

    /// uiw_unwind: after App::frame close windows the app left open (window_begin without
    /// window_end) so the main window's pass ends in the main window
    pub(crate) fn uiw_unwind(&mut self) {
        while self.m.win.sp != 0 {
            self.warn("ui_window_begin without ui_window_end");
            self.window_end();
        }
        self.m.win.over = 0;
        self.uiw_use(0);
    }

    // uiw_modal_update -> the modal window changed: the topmost visible WF_MODAL window;
    // every window but it and the windows it owns is inert
    fn uiw_modal_update(&mut self) -> bool {
        let mut modal = 0u32;
        let mut mz: i32 = -1;
        // interactive runs without OS windows: the secondary windows cannot be seen, so
        // none may block the main window
        if self.headless || self.m.win.hooks {
            for i in 1..WIN_MAX {
                let r = &self.m.win.recs[i];
                if r.state & (UWI_VISIBLE | UWI_CREATED) != UWI_VISIBLE | UWI_CREATED || r.flags & WF_MODAL == 0 {
                    continue;
                }
                if r.z as i32 > mz {
                    mz = r.z as i32;
                    modal = i as u32;
                }
            }
        }
        // inert mask
        let mut inert = 0u32;
        if modal != 0 {
            for i in 0..WIN_MAX as u32 {
                if i != modal && !self.uiw_owned_by(i, modal) {
                    inert |= 1 << i;
                }
            }
        }
        self.m.win.inert = inert;
        if modal == self.m.win.modal {
            return false;
        }
        self.m.win.modal = modal;
        if modal != 0 {
            self.uiw_activate(modal); // the modal window takes the keyboard
        }
        true
    }

    // uiw_owned_by(i, owner): `owner` owns window i (directly or further up)
    fn uiw_owned_by(&self, i: u32, owner: u32) -> bool {
        let mut i = i;
        for _ in 0..WIN_MAX {
            if i == 0 {
                return false;
            }
            i = self.m.win.recs[(i as usize) & (WIN_MAX - 1)].owner;
            if i == owner {
                return true;
            }
        }
        false
    }

    /// uiw_inert_now: the live window is inert (a modal window is open elsewhere)
    pub(crate) fn uiw_inert_now(&self) -> bool {
        (self.m.win.inert >> (self.uiw_cur & 31)) & 1 != 0
    }

    /// uiw_border -> border thickness (px) of the live window: a hairline around secondary
    /// windows with the library frame, 0 for the main window
    pub(crate) fn uiw_border(&mut self) -> i32 {
        let cur = self.uiw_cur;
        if cur == 0 || self.m.win.recs[(cur as usize) & (WIN_MAX - 1)].flags & WF_SYSFRAME != 0 {
            return 0;
        }
        self.hair()
    }

    /// uiw_escape: Esc no widget took in the live window: UIC_ESCAPE (arg = window id), in
    /// a dialog window a close request
    pub(crate) fn uiw_escape(&mut self) {
        let r = &self.m.win.recs[(self.uiw_cur as usize) & (WIN_MAX - 1)];
        let (id, flags) = (r.id, r.flags);
        let mut cmd = UIC_ESCAPE;
        if id != 0 && flags & WF_DIALOG != 0 {
            if flags & WF_NOCLOSE != 0 {
                return;
            }
            cmd = UIC_WINCLOSE;
        }
        self.cmd_run(cmd, id);
    }

    /// uiw_focus_arrived: behave met the id of a pending focus(id): it gets the focus in
    /// the live window; asked from another window, that window keeps its old focus and the
    /// live one is activated
    pub(crate) fn uiw_focus_arrived(&mut self) {
        let id = self.focus_req;
        self.focus_req = 0;
        self.wp.ctx.focus = id;
        self.wp.ctx.focus_visible = true;
        let from = self.focus_req_win;
        if from == self.uiw_cur {
            return;
        }
        let old = self.focus_req_old;
        if let Some(p) = self.uiw_parts_mut(from) {
            p.ctx.focus = old;
        }
        self.repass = true;
        let cur = self.uiw_cur;
        if cur != self.m.win.active {
            self.uiw_activate(cur);
        }
    }

    /// uiw_clear_edges: after the first pass of a batch: the input edges of every window
    /// that got input are consumed (the main window always)
    pub(crate) fn uiw_clear_edges(&mut self) {
        self.dnd_clear_edges(); // DS_DROP / DS_END were seen
        self.input_clear_edges(); // main window (live)
        let e = self.m.win.edges & !1;
        self.m.win.edges = 0;
        if e == 0 {
            return;
        }
        for i in 1..WIN_MAX as u32 {
            if (e >> i) & 1 != 0 && self.uiw_drawable(i) {
                self.uiw_use(i);
                self.input_clear_edges();
            }
        }
        self.uiw_use(0);
    }

    /// uiw_drawable(i): the window exists and is shown (the main window: always)
    pub(crate) fn uiw_drawable(&self, i: u32) -> bool {
        let s = self.m.win.recs[(i as usize) & (WIN_MAX - 1)].state;
        s & (UWI_VISIBLE | UWI_CREATED) == UWI_VISIBLE | UWI_CREATED
    }

    // ------------------------------------------------------------------ window_begin / end

    /// ui_window_begin(id, title, open, flags, rect) -> true: the window is shown, emit its
    /// content now (window_end always follows).  open None = always open; rect None: the
    /// library keeps the placement.  The owner is the window being built.
    pub fn window_begin(&mut self, id: u32, title: &str, open: Option<&Shared<i32>>, flags: u32, rect: Option<&Shared<WinRect>>) -> bool {
        let mut flags = flags;
        let sp = self.m.win.sp;
        if sp as usize >= WSTACK_MAX {
            self.m.win.over += 1;
            self.limit_hit(super::ctx::LIM_WINNEST);
            return false;
        }
        let owner = self.uiw_cur; // the window being built
        self.m.win.stack[sp as usize & (WSTACK_MAX - 1)] = owner; // not entered (yet)
        self.m.win.sp += 1;
        if id == 0 {
            self.warn("ui_window_begin: window id 0");
            return false;
        }
        // windows entered inside each other
        if self.uiw_depth() >= NEST_MAX {
            self.warn(&alloc::format!("window {}: windows nested too deeply", id));
            return false;
        }
        // the state of the last run (winstate.rs): the open flag before it is looked at
        if !self.m.win.saved.is_empty() {
            self.ws_open(id, flags, open);
        }
        let mut i = self.uiw_index(id);
        if i < 0 {
            // a window that was never shown needs no record while it is closed
            if let Some(o) = open {
                if o.get() == 0 {
                    return false;
                }
            }
            i = self.uiw_alloc();
            if i < 0 {
                self.warn(&alloc::format!("window {}: too many windows ({} besides the main window)", id, WIN_MAX - 1));
                return false;
            }
            self.m.win.recs[i as usize].id = id;
            self.m.win.used |= 1 << i;
        }
        let i = i as u32;
        let iu = i as usize;
        if self.m.win.recs[iu].state & (UWI_SEEN | UWI_BUILDING) != 0 {
            self.warn(&alloc::format!("window {} begun twice in one pass", id));
            return false;
        }
        let frame = self.frame;
        {
            let r = &mut self.m.win.recs[iu];
            r.state |= UWI_SEEN;
            r.seen = frame;
        }
        // no system frame (Wayland without decorations): the library draws it
        if flags & WF_SYSFRAME != 0 && !self.uiw_sysframe_ok() {
            flags &= !WF_SYSFRAME;
        }
        // title bar height (the style's, DIP)
        let tbh = if flags & WF_SYSFRAME != 0 {
            0
        } else {
            let st = self.style;
            st.metric(if flags & WF_TOOL != 0 { SM_WIN_TBH_TOOL } else { SM_WIN_TBH_DIALOG })
        };
        {
            let r = &mut self.m.win.recs[iu];
            r.flags = flags;
            r.owner = owner;
            r.openp = open.cloned();
            r.rect = match rect {
                Some(p) => p.clone(),
                None => r.own.clone(),
            };
            r.tbh = tbh;
            // title (a copy; a change is passed on to the platform)
            let t = super::basic::cstr(title);
            let mut n = t.len().min(TITLE_MAX - 1);
            while !t.is_char_boundary(n) {
                n -= 1;
            }
            if r.title != t[..n] {
                r.title = String::from(&t[..n]);
                r.want |= UWW_TITLE;
            }
        }
        // the placement of the last run (winstate.rs)
        if !self.m.win.saved.is_empty() {
            self.ws_rect(i, id);
        }
        // open?
        let is_open = open.is_none_or(|o| o.get() != 0);
        if !is_open {
            let r = &mut self.m.win.recs[iu];
            r.state &= !UWI_OPEN;
            if r.state & UWI_VISIBLE != 0 {
                self.uiw_hide(i);
            }
            return false;
        }
        self.m.win.recs[iu].state |= UWI_OPEN;
        if self.m.win.recs[iu].state & UWI_CREATED == 0 && !self.uiw_create(i) {
            self.warn(&alloc::format!("window {}: out of memory", id));
            return false;
        }
        if self.m.win.recs[iu].state & UWI_VISIBLE == 0 {
            self.uiw_show(i);
        }
        // enter the window
        let top = (self.m.win.sp - 1) as usize & (WSTACK_MAX - 1);
        self.m.win.stack[top] = i << 8 | owner | WSE_ENTERED;
        self.uiw_use(i);
        self.m.win.recs[iu].state |= UWI_BUILDING;
        if self.m.win.recs[iu].state & UWI_FRESH != 0 {
            self.m.win.recs[iu].state &= !UWI_FRESH;
            // a window shown (again) starts without focus; a dialog window with the focus on
            // its first control (kbnav.rs)
            self.wp.ctx.focus = 0;
            self.wp.ctx.focus_visible = false;
            self.wp.ctx.active = 0;
            self.wp.ctx.kb.autofocus = flags & WF_DIALOG != 0;
        }
        self.uiw_apply_rect(i);
        let (w, h) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        self.uiw_pass_begin(w, h);
        self.win_chrome_begin(i);
        true
    }

    // uiw_depth -> windows entered on the window_begin stack
    fn uiw_depth(&self) -> u32 {
        let n = (self.m.win.sp as usize).min(WSTACK_MAX);
        self.m.win.stack[..n].iter().filter(|&&e| e & WSE_ENTERED != 0).count() as u32
    }

    // uiw_alloc -> index of a free window record (1..WIN_MAX-1) or -1; when every record is
    // in use, the hidden window seen longest ago is freed for it
    fn uiw_alloc(&mut self) -> i32 {
        if let Some(i) = (1..WIN_MAX).find(|&i| self.m.win.recs[i].id == 0) {
            return i as i32;
        }
        // recycle: a window that is not shown, not being built, not begun in this pass
        let mut cand: i32 = -1;
        let mut oldest = u32::MAX;
        for i in 1..WIN_MAX {
            let r = &self.m.win.recs[i];
            if r.state & (UWI_VISIBLE | UWI_BUILDING | UWI_SEEN) != 0 {
                continue;
            }
            if r.seen < oldest {
                oldest = r.seen;
                cand = i as i32;
            }
        }
        if cand < 0 {
            return -1;
        }
        let id = self.m.win.recs[cand as usize].id;
        self.window_destroy(id);
        cand
    }

    /// ui_window_end: close window_begin (also when it returned false)
    pub fn window_end(&mut self) {
        if self.m.win.over != 0 {
            self.m.win.over -= 1;
            return;
        }
        if self.m.win.sp == 0 {
            self.warn("ui_window_end without ui_window_begin");
            return;
        }
        let sp = (self.m.win.sp - 1) & (WSTACK_MAX as u32 - 1);
        self.m.win.sp = sp;
        let e = self.m.win.stack[sp as usize];
        if e & WSE_ENTERED == 0 {
            return;
        }
        let i = (e >> 8) & 255; // index
        let back = e & 255; // window to return to
        let iu = (i as usize) & (WIN_MAX - 1);
        let h = self.win_chrome_end(i); // natural content height (DIP)
        let flags = self.m.win.recs[iu].flags;
        if flags & WF_AUTOHEIGHT != 0 {
            self.m.win.recs[iu].autoh = h;
        }
        // resize handles of the library frame
        if flags & (WF_RESIZABLE | WF_SYSFRAME) == WF_RESIZABLE {
            self.edge_grips();
        }
        self.m.win.noaccel = flags & WF_DIALOG != 0;
        self.uiw_pass_end();
        self.m.win.noaccel = false;
        self.m.win.recs[iu].state &= !UWI_BUILDING;
        self.uiw_use(back);
    }

    // uiw_create(i) -> ok: per-window state and buffers of a new window; its UI state
    // starts empty, its dpi is the owner's
    fn uiw_create(&mut self, i: u32) -> bool {
        let iu = (i as usize) & (WIN_MAX - 1);
        let owner = self.m.win.recs[iu].owner;
        let dpi = self.win_dpi(owner);
        let mut p = Box::new(WinParts::new());
        p.rt.dpi = dpi;
        p.ctx.mx = -1;
        p.ctx.my = -1;
        p.lay.root_id = i | WROOT_BASE;
        p.cmd = super::cmd::W::new(&super::cmd::CAPS_SMALL);
        p.dirty.full = true; // dirty_force_full
        self.m.win.recs[iu].save = Some(p);
        let r = &mut self.m.win.recs[iu];
        r.state |= UWI_CREATED;
        r.want |= UWW_CREATE;
        true
    }

    // uiw_show(i): the window becomes visible: default size / placement the first time,
    // stacking order, activation (dialogs and plain windows; not tool / WF_NOACTIVATE)
    fn uiw_show(&mut self, i: u32) {
        let iu = (i as usize) & (WIN_MAX - 1);
        let mut rc = self.m.win.recs[iu].rect.get();
        if rc.w <= 0 {
            rc.w = DEF_W;
        }
        if rc.h <= 0 {
            rc.h = DEF_H;
        }
        self.m.win.recs[iu].rect.set(rc);
        if rc.flags & WRF_POS == 0 {
            self.uiw_place_default(i);
            self.m.win.recs[iu].state |= UWI_DEFPOS;
        }
        self.m.win.zc = self.m.win.zc.wrapping_add(1);
        let zc = self.m.win.zc;
        let r = &mut self.m.win.recs[iu];
        r.state |= UWI_VISIBLE | UWI_NEWSHOW | UWI_SHOWN | UWI_FRESH;
        r.want = (r.want & !UWW_HIDE) | UWW_SHOW;
        r.z = zc;
        if r.flags & (WF_TOOL | WF_NOACTIVATE) == 0 {
            self.uiw_activate(i);
        }
    }

    /// uiw_hide(i): the window disappears (its state and buffers stay)
    pub(crate) fn uiw_hide(&mut self, i: u32) {
        let iu = (i as usize) & (WIN_MAX - 1);
        let r = &mut self.m.win.recs[iu];
        r.state &= !(UWI_VISIBLE | UWI_NEWSHOW);
        r.want = (r.want & !UWW_SHOW) | UWW_HIDE;
        let mut owner = r.owner;
        if self.m.win.ptr == i as i32 {
            self.m.win.ptr = -1;
        }
        if self.m.win.active != i {
            return;
        }
        // the keyboard returns to the nearest visible owner
        let mut n = WIN_MAX;
        loop {
            let o = owner;
            if self.uiw_drawable(o) || o == 0 {
                break;
            }
            owner = self.m.win.recs[(o as usize) & (WIN_MAX - 1)].owner;
            n -= 1;
            if n == 0 {
                owner = 0;
                break;
            }
        }
        self.uiw_activate(owner);
    }

    /// uiw_activate(i): the window gets the keyboard and goes on top
    pub(crate) fn uiw_activate(&mut self, i: u32) {
        let i = i & (WIN_MAX as u32 - 1);
        if self.m.win.active == i {
            return;
        }
        // interactive runs without OS windows: the keyboard stays with the main window
        if i != 0 && !self.headless && !self.m.win.hooks {
            return;
        }
        self.m.win.active = i;
        self.m.win.recs[i as usize].want |= UWW_ACTIVATE;
        if i != 0 {
            self.m.win.zc = self.m.win.zc.wrapping_add(1);
            self.m.win.recs[i as usize].z = self.m.win.zc;
        }
        self.invalidate();
    }

    // uiw_place_default(i): first placement (the same on every platform and headless):
    // dialogs centred on the owner, other windows at the owner's client origin + (24 + 32k,
    // 48 + 32k) DIP, k = windows of that owner placed before; written back with WRF_POS
    fn uiw_place_default(&mut self, i: u32) {
        let iu = (i as usize) & (WIN_MAX - 1);
        let (owner, flags, tbh) = {
            let r = &self.m.win.recs[iu];
            (r.owner, r.flags, r.tbh)
        };
        let mut rc = self.m.win.recs[iu].rect.get();
        if flags & WF_DIALOG == 0 {
            // k = shown windows of the same owner (not dialogs)
            let mut k = 0;
            for j in 1..WIN_MAX {
                if j == iu {
                    continue;
                }
                let r = &self.m.win.recs[j];
                if r.id != 0 && r.owner == owner && r.state & UWI_SHOWN != 0 && r.flags & WF_DIALOG == 0 {
                    k += 1;
                }
            }
            rc.x = k * 32 + 24;
            rc.y = k * 32 + 48;
        } else {
            // owner outer rectangle relative to its client origin, in DIP
            let (ow, oh, oy) = self.uiw_outer_dip(owner);
            rc.x = (ow - rc.w) >> 1;
            rc.y = ((oh - (tbh + rc.h)) >> 1) + oy;
        }
        rc.flags |= WRF_POS;
        self.m.win.recs[iu].rect.set(rc);
    }

    // uiw_outer_dip(i) -> (outer width, outer height (DIP), y of the outer top edge relative
    // to the client origin (-title bar height; main window: 0))
    fn uiw_outer_dip(&self, i: u32) -> (i32, i32, i32) {
        if i == 0 {
            // the main window: its framebuffer
            let fb = self.win_fb(0);
            let dpi = self.win_dpi(0);
            return (px2dip_at(fb.w, dpi), px2dip_at(fb.h, dpi), 0);
        }
        let r = &self.m.win.recs[(i as usize) & (WIN_MAX - 1)];
        let rc = r.rect.get();
        (rc.w, rc.h + r.tbh, -r.tbh)
    }

    // uiw_apply_rect(i): (the window is live) apply its WINRECT when it changed:
    // framebuffer size = client + title bar + border at the window's dpi
    fn uiw_apply_rect(&mut self, i: u32) {
        let iu = (i as usize) & (WIN_MAX - 1);
        let mut rc = self.m.win.recs[iu].rect.get();
        if rc.w <= 0 {
            rc.w = DEF_W;
        }
        if rc.h <= 0 {
            rc.h = DEF_H;
        }
        if rc.w > SIZE_MAX_DIP {
            rc.w = SIZE_MAX_DIP;
        }
        if rc.h > SIZE_MAX_DIP {
            rc.h = SIZE_MAX_DIP;
        }
        self.m.win.recs[iu].rect.set(rc);
        if !self.wp.rt.fb.px.is_empty() && rc == self.m.win.recs[iu].applied {
            return;
        }
        let tbh = {
            let r = &mut self.m.win.recs[iu];
            r.applied = rc;
            r.want |= UWW_RECT;
            r.tbh
        };
        let b2 = self.uiw_border() * 2; // 2 borders
        let w = self.dp(rc.w) + b2; // framebuffer width
        let h = self.dp(rc.h + tbh) + b2; // framebuffer height
        if !self.wp.rt.fb.px.is_empty() && w == self.wp.rt.fb.w && h == self.wp.rt.fb.h {
            return;
        }
        if self.wp.rt.fb.px.is_empty() {
            self.wp.rt.fb = Surface::new(w, h, FMT_XRGB);
        } else {
            self.wp.rt.fb.resize(w, h);
        }
        self.dirty_force_full();
    }

    // ------------------------------------------------------------------ settle / sync

    /// uiw_settle -> the frame must be built again: after the settled pass, windows that
    /// were not begun are hidden, WF_AUTOHEIGHT windows take their content height, platform
    /// operations are done
    pub(crate) fn uiw_settle(&mut self) -> bool {
        let mut changed = false;
        if self.m.win.used != 0 {
            for i in 1..WIN_MAX as u32 {
                let iu = i as usize;
                let (state, flags, autoh) = {
                    let r = &self.m.win.recs[iu];
                    (r.state, r.flags, r.autoh)
                };
                if state & UWI_VISIBLE == 0 {
                    continue;
                }
                if state & UWI_SEEN == 0 {
                    self.uiw_hide(i);
                    changed = true;
                    continue;
                }
                if flags & WF_AUTOHEIGHT == 0 || autoh <= 0 {
                    continue;
                }
                let mut rc = self.m.win.recs[iu].rect.get();
                if rc.h == autoh {
                    continue;
                }
                rc.h = autoh;
                self.m.win.recs[iu].rect.set(rc);
                changed = true;
                // a dialog placed by the default rule stays centred while its height settles
                if state & UWI_DEFPOS != 0 && flags & WF_DIALOG != 0 {
                    self.uiw_place_default(i);
                }
            }
            changed |= self.uiw_modal_update();
        }
        // platform operations once the windows stopped changing
        if !changed {
            self.uiw_sync();
        }
        changed
    }

    // uiw_sync: pending platform operations (nothing headless)
    fn uiw_sync(&mut self) {
        if !self.m.win.hooks || self.headless {
            return; // no OS windows (headless, or no platform support yet): nothing to do
        }
        self.uiw_enable_sync(1); // windows a closed modal window disabled first (before it is hidden)
        for i in 0..WIN_MAX as u32 {
            let iu = i as usize;
            let want = self.m.win.recs[iu].want;
            if want == 0 {
                continue;
            }
            self.m.win.recs[iu].want = 0;
            if i != 0 {
                if want & UWW_CREATE != 0 {
                    let (flags, owner) = (self.m.win.recs[iu].flags, self.m.win.recs[iu].owner);
                    self.uwp_create(i, flags, owner);
                }
                if want & UWW_RECT != 0 {
                    let rc = self.m.win.recs[iu].rect.get();
                    self.uwp_rect(i, &rc);
                }
                if want & UWW_TITLE != 0 {
                    let t = self.m.win.recs[iu].title.clone();
                    self.uwp_title(i, &t);
                }
                if want & UWW_SHOW != 0 {
                    // shown (and activated) after its first present: uiw_sync_show
                    self.m.win.recs[iu].want |= want & (UWW_SHOW | UWW_ACTIVATE);
                    continue;
                }
                if want & UWW_HIDE != 0 {
                    self.uwp_show(i, false);
                }
            }
            if i == 0 && want & UWW_RECT != 0 {
                // window_place(0, ...)
                let rc = self.m.win.recs[0].rect.get();
                self.uwp_rect(0, &rc);
            }
            if want & UWW_ACTIVATE != 0 {
                self.uwp_activate(i);
            }
        }
    }

    /// uiw_sync_show: after render: windows waiting to be shown appear (and are activated)
    /// now that their framebuffer has content
    pub(crate) fn uiw_sync_show(&mut self) {
        if !self.m.win.hooks || self.headless {
            return;
        }
        for i in 1..WIN_MAX as u32 {
            let iu = i as usize;
            let r = &self.m.win.recs[iu];
            if r.want & UWW_SHOW == 0 || r.state & UWI_VISIBLE == 0 {
                continue;
            }
            self.m.win.recs[iu].want &= !UWW_SHOW;
            self.uwp_show(i, true);
            if self.m.win.recs[iu].want & UWW_ACTIVATE != 0 {
                self.m.win.recs[iu].want &= !UWW_ACTIVATE;
                self.uwp_activate(i);
            }
        }
        self.uiw_enable_sync(2); // a new modal window is shown: the windows below it are disabled now
    }

    /// ui_win_hooks_set: secondary windows become OS windows (the runtime calls it when it
    /// runs with a display; without it they are built but only exist as framebuffers)
    pub(crate) fn win_hooks_set(&mut self, on: bool) {
        self.m.win.hooks = on;
    }

    /// ui_sync_hook_set(f): f() -> true while background jobs are pending (the script
    /// command `sync` waits for false)
    pub fn sync_hook_set(&mut self, f: Option<fn() -> bool>) {
        self.m.win.synchook = f;
    }

    /// uiw_sync_pending -> background jobs are pending (false without a hook)
    pub(crate) fn uiw_sync_pending(&mut self) -> bool {
        match self.m.win.synchook {
            Some(f) => f(),
            None => false,
        }
    }

    /// uiw_present(n): the live window's framebuffer changed in n dirty rectangles (drects;
    /// -1 = everything): hand it to the platform
    pub(crate) fn uiw_present(&mut self, n: i32) {
        if self.headless {
            return;
        }
        let cur = self.uiw_cur;
        if cur == 0 {
            let rects = if n < 0 { None } else { Some(&self.drects[..n as usize]) };
            plat::present(&Ui::frame_of(&self.wp.rt.fb), rects);
            return;
        }
        if !self.m.win.hooks {
            return;
        }
        // uwp_present(index, &fb, rects, n)
        let pw = self.uiw_pw(cur);
        if pw < 0 {
            return;
        }
        let rects = if n < 0 { None } else { Some(&self.drects[..n as usize]) };
        plat::win_present(pw, &Ui::frame_of(&self.wp.rt.fb), rects);
    }

    /// (the platform's hit hook) publish the hit list of the live window's last pass for
    /// the non-client hit tests, which run without the UI context (uiw_hit_pw).  Called at
    /// the end of uiw_pass_end.
    pub(crate) fn uiw_publish_hits(&mut self) {
        if self.headless {
            return;
        }
        let cur = (self.uiw_cur as usize) & (WIN_MAX - 1);
        let mut drawable = 0;
        for i in 0..WIN_MAX as u32 {
            if self.uiw_drawable(i) {
                drawable |= 1 << i;
            }
        }
        let c = &self.wp.ctx;
        let mut p = PUBLISHED.lock();
        p.hits[cur].clear();
        p.hits[cur].extend_from_slice(&c.hits[c.hit_cur ^ 1]);
        p.zones[cur].clear();
        p.zones[cur].extend_from_slice(&c.zones[c.hit_cur ^ 1]);
        p.modal[cur] = c.modal_prev;
        p.mirror[cur] = if self.rtl { self.wp.rt.fb.w } else { 0 };
        p.drawable = drawable;
        p.p2u = self.m.win.p2u;
    }

    // publish_state: the window map and the shown windows for the hit hook
    fn publish_state(&self) {
        if self.headless {
            return;
        }
        let mut drawable = 0;
        for i in 0..WIN_MAX as u32 {
            if self.uiw_drawable(i) {
                drawable |= 1 << i;
            }
        }
        let mut p = PUBLISHED.lock();
        p.drawable = drawable;
        p.p2u = self.m.win.p2u;
    }

    // ------------------------------------------------------------------ events

    /// uiw_input_target(ev) -> the window an input event goes to, -1 = drop it.  Keys go
    /// to the keyboard window; from a tool window without a focused text box (and without
    /// WF_KEYS) they go on to its owner; with a modal window open every key of an inert
    /// window goes to the modal one.  A pointer event in another window than the last one
    /// first leaves that window (E_LEAVE).  Headless runs activate windows on a press like
    /// the Windows window manager.
    pub(crate) fn uiw_input_target(&mut self, ev: &Event) -> i32 {
        let w = ev.win as u32;
        if w as usize >= WIN_MAX || (w != 0 && !self.uiw_drawable(w)) {
            return -1; // a dead or hidden window
        }
        let t = match ev.typ {
            plat::E_KEY_DOWN | plat::E_KEY_UP | plat::E_TEXT | plat::E_IME_COMMIT | plat::E_PREEDIT => self.uiw_key_target(w),
            plat::E_LEAVE => {
                if self.m.win.ptr == w as i32 {
                    self.m.win.ptr = -1;
                }
                w
            }
            plat::E_FOCUS_OUT => w,
            _ => {
                // pointer: another window than the last one -> that one is left first
                let last = self.m.win.ptr;
                if last != w as i32 {
                    self.m.win.ptr = w as i32;
                    if last >= 0 && self.uiw_drawable(last as u32) {
                        self.uiw_use(last as u32);
                        let leave = Event { typ: plat::E_LEAVE, btn: 0, mods: ev.mods, win: last as u8, x: -1, y: -1, data: 0 };
                        self.input_event(&leave);
                        self.m.win.edges |= 1 << last;
                    }
                }
                if ev.typ == plat::E_DOWN {
                    self.uiw_kfocus_press(w, ev.x, ev.y);
                    self.uiw_popups_dismiss(w); // (U3) popups of other windows close
                    if self.headless {
                        self.uiw_click_activate(w, ev.x, ev.y);
                    }
                }
                w
            }
        };
        let t = t & (WIN_MAX as u32 - 1);
        self.m.win.edges |= 1 << t;
        t as i32
    }

    // uiw_key_target(i) -> the window that gets a key / text event sent to window i
    fn uiw_key_target(&mut self, i: u32) -> u32 {
        let mut b = i;
        if b != 0 && !self.uiw_drawable(b) {
            b = self.m.win.active; // a gone window: the keyboard window
        }
        if (self.m.win.inert >> (b & 31)) & 1 != 0 {
            return self.m.win.modal & (WIN_MAX as u32 - 1);
        }
        // (U3) a tool window whose (non-text) widget got the focus by a press borrows the
        // keys of its owner: its widgets see them first, the rest goes on to the owner
        // (uiw_fwd_take / uiw_fwd_inject)
        let kf = self.m.win.kfocus;
        if kf != 0 {
            let t = kf - 1;
            if self.uiw_borrows(t) && (b == t || b == self.m.win.recs[(t as usize) & (WIN_MAX - 1)].owner) {
                self.m.win.fwd |= 1 << (t & 31);
                return t & (WIN_MAX as u32 - 1);
            }
        }
        let mut n = WIN_MAX;
        loop {
            if b == 0 {
                break;
            }
            let r = &self.m.win.recs[(b as usize) & (WIN_MAX - 1)];
            if r.flags & WF_TOOL == 0 || r.flags & WF_KEYS != 0 {
                break;
            }
            let owner = r.owner;
            if self.uiw_focus_is_text(b) {
                break;
            }
            // (U3) a focused widget of the tool window: it borrows the key (the rest goes on)
            if self.uiw_borrows(b) {
                self.m.win.fwd |= 1 << (b & 31);
                break;
            }
            b = owner;
            n -= 1;
            if n == 0 {
                break;
            }
        }
        b & (WIN_MAX as u32 - 1)
    }

    /// uiw_keys_window -> the window a key would go to now (uiw_key_target without its side
    /// effects; the dump's "keys", the script's `expect window ID keys`)
    pub(crate) fn uiw_keys_window(&self) -> u32 {
        let mut b = self.m.win.active & (WIN_MAX as u32 - 1);
        if (self.m.win.inert >> b) & 1 != 0 {
            return self.m.win.modal & (WIN_MAX as u32 - 1);
        }
        let kf = self.m.win.kfocus;
        if kf != 0 {
            let t = kf - 1;
            if self.uiw_borrows(t) && (b == t || b == self.m.win.recs[(t as usize) & (WIN_MAX - 1)].owner) {
                return t & (WIN_MAX as u32 - 1);
            }
        }
        for _ in 0..WIN_MAX {
            if b == 0 {
                break;
            }
            let r = &self.m.win.recs[(b as usize) & (WIN_MAX - 1)];
            if r.flags & WF_TOOL == 0 || r.flags & WF_KEYS != 0 || self.uiw_focus_is_text(b) || self.uiw_borrows(b) {
                break;
            }
            b = r.owner;
        }
        b & (WIN_MAX as u32 - 1)
    }

    // uiw_borrows(i): tool window i (shown, not inert, without WF_KEYS) has a focused
    // widget, so it may borrow its owner's keys (U3)
    fn uiw_borrows(&self, i: u32) -> bool {
        if i == 0 || i as usize >= WIN_MAX || !self.uiw_drawable(i) || (self.m.win.inert >> i) & 1 != 0 {
            return false;
        }
        if self.m.win.recs[i as usize].flags & (WF_TOOL | WF_KEYS) != WF_TOOL {
            return false;
        }
        self.win_focus(i) != 0
    }

    // uiw_kfocus_press(i, x, y): a press in window i: in a tool window (not on a text box)
    // that window borrows the keys from now on while one of its widgets has the focus; a
    // press anywhere else gives the keys their normal way again (U3)
    fn uiw_kfocus_press(&mut self, i: u32, x: i32, y: i32) {
        self.m.win.kfocus = 0;
        self.m.kbnav.tool = false; // (kbnav.rs: a press ends the keys F6 gave a tool window)
        if i == 0 || self.m.win.recs[(i as usize) & (WIN_MAX - 1)].flags & (WF_TOOL | WF_KEYS) != WF_TOOL {
            return;
        }
        if self.hit_flags_win(i, x, y) & HB_TEXT != 0 {
            return; // (a text box activates the window)
        }
        self.m.win.kfocus = i + 1;
    }

    /// uiw_fwd_take: (start of uiw_pass_end of the live window) a window that borrowed keys
    /// this frame and has no open popup hands the keys and typed text its widgets did not
    /// consume to its owner (injected by uiw_fwd_inject after the pass) (U3)
    pub(crate) fn uiw_fwd_take(&mut self) {
        let cur = self.uiw_cur;
        if cur as usize >= WIN_MAX || (self.m.win.fwd >> cur) & 1 == 0 {
            return;
        }
        self.m.win.fwd &= !(1 << cur);
        if self.popup_count() != 0 {
            return; // menus / lists of the window keep them
        }
        self.m.win.fwd_to = self.m.win.recs[cur as usize].owner & (WIN_MAX as u32 - 1);
        let n = FWDQ_MAX - self.m.win.fwdq.len().min(FWDQ_MAX);
        let k = self.kb_take_rest(n); // (kbnav.rs: Tab, F6, arrows in groups... stay here)
        self.m.win.fwdq.extend_from_slice(&k);
        let n = FWDT_MAX - self.m.win.fwdt.len().min(FWDT_MAX);
        let t = self.text_take(n);
        self.m.win.fwdt.extend_from_slice(&t);
    }

    /// uiw_fwd_inject -> keys were handed on (after the first pass of a batch, whose edges
    /// were just cleared): they are the owner's input for the next pass, which the caller
    /// runs and clears again (U3)
    pub(crate) fn uiw_fwd_inject(&mut self) -> bool {
        self.m.win.fwd = 0;
        if self.m.win.fwdq.is_empty() && self.m.win.fwdt.is_empty() {
            return false;
        }
        let to = self.m.win.fwd_to;
        let mut r = false;
        if self.uiw_drawable(to) {
            self.uiw_use(to);
            let q = core::mem::take(&mut self.m.win.fwdq);
            self.keys_inject(&q);
            let t = core::mem::take(&mut self.m.win.fwdt);
            self.text_inject(&t);
            self.uiw_use(0);
            self.m.win.edges |= 1 << to;
            self.repass = true;
            r = true;
        }
        self.m.win.fwdq.clear();
        self.m.win.fwdt.clear();
        r
    }

    /// uiw_fwd_arm(i): (U3) typed text for window i outside an input event (ctx.rs
    /// text_refill): what the widgets of a borrowing tool window leave goes on to its owner,
    /// as uiw_key_target arranges for typed text
    pub(crate) fn uiw_fwd_arm(&mut self, i: u32) {
        if self.uiw_borrows(i) && !self.uiw_focus_is_text(i) {
            self.m.win.fwd |= 1 << (i & 31);
        }
    }

    // uiw_focus_is_text(i): the focused widget of window i was a text input in the last pass
    fn uiw_focus_is_text(&self, i: u32) -> bool {
        let Some(p) = self.uiw_parts(i) else { return false };
        let f = p.ctx.focus;
        if f == 0 {
            return false;
        }
        let list = &p.ctx.hits[p.ctx.hit_cur ^ 1];
        match list.iter().rev().find(|h| h.id == f) {
            Some(h) => h.flags as u32 & HB_TEXT != 0,
            None => false,
        }
    }

    // uiw_click_activate(i, x, y): headless window manager: a press activates the window,
    // except tool / WF_NOACTIVATE windows, which only take the keyboard for a text box;
    // inert windows (a modal window is open) are not activated
    fn uiw_click_activate(&mut self, i: u32, x: i32, y: i32) {
        if (self.m.win.inert >> (i & 31)) & 1 != 0 {
            return;
        }
        if i != 0 {
            let f = self.m.win.recs[(i as usize) & (WIN_MAX - 1)].flags;
            if f & (WF_TOOL | WF_NOACTIVATE) != 0 {
                if f & WF_NOACTIVATE != 0 || self.hit_flags_win(i, x, y) & HB_TEXT == 0 {
                    return;
                }
            }
        }
        self.uiw_activate(i);
    }

    /// ui_hit_flags_win(i, x, y) -> HB_* flags of the topmost hit box at the point (px) of
    /// window i in its last pass (0 = none); no state is swapped
    pub(crate) fn hit_flags_win(&mut self, i: u32, x: i32, y: i32) -> u32 {
        let i = i & (WIN_MAX as u32 - 1);
        if !self.uiw_drawable(i) {
            return 0;
        }
        let Some(p) = self.uiw_parts(i) else { return 0 };
        let c = &p.ctx;
        super::ctx::hit_find_list(&c.hits[c.hit_cur ^ 1], c.modal_prev, x, y).1
    }

    /// uiw_moved_passes -> pointer moves (or leaving) in some window need a pass
    pub(crate) fn uiw_moved_passes(&mut self) -> bool {
        if self.m.win.used == 0 {
            // the main window only
            if !self.wp.ctx.moved {
                return false;
            }
            let r = self.needs_move_pass();
            self.wp.ctx.moved = false;
            return r;
        }
        let mut r = false;
        for i in 0..WIN_MAX as u32 {
            if !self.uiw_drawable(i) || !self.uiw_parts(i).is_some_and(|p| p.ctx.moved) {
                continue;
            }
            self.uiw_use(i);
            r |= self.needs_move_pass();
            self.wp.ctx.moved = false;
        }
        self.uiw_use(0);
        r
    }

    /// uiw_resize(i, w, h): the window's client area got this size (E_RESIZE)
    pub(crate) fn uiw_resize(&mut self, app: &mut dyn super::App, i: u32, w: i32, h: i32) {
        if i != 0 {
            if i as usize >= WIN_MAX || !self.uiw_drawable(i) || w <= 0 || h <= 0 {
                return;
            }
            // the new client size in DIP goes into the WINRECT (the user resized it)
            let iu = i as usize;
            let dpi = self.win_dpi(i);
            self.uiw_use(i);
            let b2 = self.uiw_border() * 2;
            let tbh = self.m.win.recs[iu].tbh;
            let mut rc = self.m.win.recs[iu].rect.get();
            rc.w = px2dip_at(w - b2, dpi);
            rc.h = (px2dip_at(h - b2, dpi) - tbh).max(1);
            rc.flags |= WRF_CHANGED;
            if !self.headless {
                rc.flags &= !WRF_MAX;
                if self.uiw_plat_state(i) & plat::WST_MAXIMIZED != 0 {
                    rc.flags |= WRF_MAX;
                }
            }
            self.m.win.recs[iu].rect.set(rc);
            self.m.win.recs[iu].applied = rc;
        }
        self.uiw_use(i);
        self.resize(app, w, h);
        self.uiw_use(0);
        if i == 0 {
            self.ws_track(); // the normal size of the main window (winstate.rs)
        }
    }

    /// uiw_dpi(i, dpi): the window's dpi changed (E_DPI, script `scale`): full redraw; the
    /// main window also gets UIC_DPI, other windows are laid out again at the new dpi
    pub(crate) fn uiw_dpi(&mut self, i: u32, dpi: i32) {
        if i as usize >= WIN_MAX || !self.uiw_drawable(i) || dpi < 24 {
            return;
        }
        self.uiw_use(i);
        self.wp.rt.dpi = dpi;
        self.dirty_force_full();
        if i == 0 {
            self.cmd_run(UIC_DPI, dpi as u32);
        } else {
            self.m.win.recs[i as usize].applied.w = -1; // apply the rect again
        }
        self.uiw_use(0);
        self.invalidate();
    }

    /// uiw_focus_in(i): the OS gave the window the keyboard (E_FOCUS_IN)
    pub(crate) fn uiw_focus_in(&mut self, i: u32) {
        if i as usize >= WIN_MAX || !self.uiw_drawable(i) || i == self.m.win.active {
            return;
        }
        self.uiw_activate(i);
    }

    /// uiw_winpos(i, x, y): the platform reports the window's screen position (E_WINPOS:
    /// client origin of the main window, outer top-left of the others, px): kept in the
    /// WINRECT (main window: WRF_ABS px; others: DIP relative to the owner's client origin)
    pub(crate) fn uiw_winpos(&mut self, i: u32, x: i32, y: i32) {
        if i as usize >= WIN_MAX {
            return;
        }
        let iu = i as usize;
        if self.m.win.recs[iu].state == 0 {
            return;
        }
        self.m.win.recs[iu].sx = x;
        self.m.win.recs[iu].sy = y;
        let mut rc = self.m.win.recs[iu].rect.get();
        if i == 0 {
            rc.x = x;
            rc.y = y;
            rc.flags |= WRF_ABS | WRF_POS;
            let a = &mut self.m.win.recs[0].applied;
            a.flags = rc.flags;
            a.x = x;
            a.y = y;
        } else {
            // owner client origin on the screen
            let (ox, oy) = self.uiw_owner_origin(i);
            let odpi = self.win_dpi(self.m.win.recs[iu].owner);
            rc.x = px2dip_at(x - ox, odpi);
            rc.y = px2dip_at(y - oy, odpi);
            rc.flags |= WRF_POS | WRF_CHANGED;
            let r = &mut self.m.win.recs[iu];
            r.applied.x = rc.x;
            r.applied.y = rc.y;
            r.applied.flags = rc.flags;
            r.state &= !UWI_DEFPOS;
        }
        self.m.win.recs[iu].rect.set(rc);
        if i == 0 {
            self.ws_track(); // the normal position of the main window (winstate.rs)
        }
        // windows it owns stay where they are: their offsets change
        self.uiw_follow_owner(i);
    }

    /// uiw_close_request(i): the user wants to close the window (E_CLOSE, close button):
    /// App::command(UIC_WINCLOSE, id); false closes it (uiw_cmd_done)
    pub(crate) fn uiw_close_request(&mut self, i: u32) {
        if i == 0 || i as usize >= WIN_MAX {
            return;
        }
        let r = &self.m.win.recs[i as usize];
        if r.id == 0 || r.flags & WF_NOCLOSE != 0 {
            return;
        }
        let id = r.id;
        self.cmd_run(UIC_WINCLOSE, id);
        self.invalidate();
    }

    /// uiw_cmd_done(id, arg, result): after App::command: UIC_WINCLOSE that the app did not
    /// handle (false) closes the window: *open = 0, hidden
    pub(crate) fn uiw_cmd_done(&mut self, id: u32, arg: u32, r: bool) {
        if id != UIC_WINCLOSE || r {
            return;
        }
        let i = self.uiw_index(arg);
        if i <= 0 {
            return;
        }
        let rec = &self.m.win.recs[i as usize];
        if let Some(o) = &rec.openp {
            o.set(0);
        }
        if rec.state & UWI_VISIBLE != 0 {
            self.uiw_hide(i as u32);
        }
    }

    // ------------------------------------------------------------------ app API

    /// ui_window_place(id, &rect): set the window's placement (copied into its WINRECT;
    /// applied when it is built next).  id 0: the main window (headless: its size)
    pub fn window_place(&mut self, id: u32, rect: &WinRect) {
        let i = self.uiw_index(id);
        if i < 0 {
            return;
        }
        let iu = i as usize;
        self.m.win.recs[iu].state &= !UWI_DEFPOS;
        self.m.win.recs[iu].rect.set(*rect);
        if i == 0 {
            // main window: the client size (headless), the OS window (uwp_rect)
            if !self.headless {
                self.m.win.recs[0].want |= UWW_RECT;
            } else if rect.w > 0 && rect.h > 0 {
                let (w, h) = (self.dp(rect.w), self.dp(rect.h));
                // ui_resize without App::frame (the asm's resize runs the passes only in
                // the script runner; ui_resize itself sets need)
                self.resize_noapp(w, h);
            }
        }
        self.invalidate();
    }

    /// ui_window_snap(id, area, dist): while the user moves window id, its outer edges snap
    /// to the edges of `area` (x, y, w, h in DIP of the owner's client area, like its
    /// WINRECT) once they are at most `dist` DIP away -- both axes at a corner; dist 0 turns
    /// it off.  Live where the platform lets the UI see the move (Windows: WM_MOVING) and for
    /// scripts (movewin); a window manager that moves the windows itself (X11, Wayland) and
    /// the browser do not snap.  Call it in every frame the area may change (after the
    /// window's first window_begin).
    pub fn window_snap(&mut self, id: u32, area: [i32; 4], dist: i32) {
        let i = self.uiw_index(id);
        if i <= 0 {
            return;
        }
        let iu = i as usize;
        let s = [area[0], area[1], area[2], area[3], dist.max(0)];
        self.m.win.recs[iu].snap = s;
        if self.headless {
            return;
        }
        // px from the owner's platform client origin: below a library title bar of the owner
        // (uiw_owner_origin)
        let owner = self.m.win.recs[iu].owner;
        let dpi = self.win_dpi(owner);
        let mut p = [0; 6];
        if s[4] != 0 && dpi >= 24 {
            let tb = if owner != 0 { dip2px_at(self.m.win.recs[(owner as usize) & (WIN_MAX - 1)].tbh, dpi) } else { 0 };
            p = [owner as i32, dip2px_at(s[0], dpi), tb + dip2px_at(s[1], dpi), dip2px_at(s[2], dpi), dip2px_at(s[3], dpi), dip2px_at(s[4], dpi)];
        }
        PUBLISHED.lock().snap[iu] = p;
    }

    /// uiw_snap_dip(i, x, y) -> (x, y): a user move of window i to x, y (DIP of the owner)
    /// snapped like the platform's move hook does it (window_snap; scripts: movewin)
    pub(crate) fn uiw_snap_dip(&self, i: u32, x: i32, y: i32) -> (i32, i32) {
        let iu = (i as usize) & (WIN_MAX - 1);
        let r = &self.m.win.recs[iu];
        let s = r.snap;
        if s[4] == 0 {
            return (x, y);
        }
        // the outer size: the framebuffer (title bar, border), before it exists the WINRECT
        let rc = r.rect.get();
        let (fb, dpi) = (self.win_fb(i), self.win_dpi(i));
        let (w, h) = if fb.w > 0 && dpi >= 24 { (px2dip_at(fb.w, dpi), px2dip_at(fb.h, dpi)) } else { (rc.w, rc.h + r.tbh) };
        snap_to(x, y, w, h, [s[0], s[1], s[2], s[3]], s[4])
    }

    /// ui_window_rect(id) -> the window's placement (None = unknown window)
    pub fn window_rect(&self, id: u32) -> Option<WinRect> {
        let i = self.uiw_index(id);
        if i < 0 {
            return None;
        }
        Some(self.m.win.recs[i as usize].rect.get())
    }

    /// ui_window_state(id) -> UWS_OPEN | UWS_VISIBLE | UWS_ACTIVE | UWS_MINIMIZED (0 unknown)
    pub fn window_state(&mut self, id: u32) -> u32 {
        let i = self.uiw_index(id);
        if i < 0 {
            return 0;
        }
        let i = i as u32;
        let st = self.m.win.recs[i as usize].state;
        let mut s = 0;
        if st & UWI_OPEN != 0 {
            s |= UWS_OPEN;
        }
        if st & (UWI_VISIBLE | UWI_CREATED) != UWI_VISIBLE | UWI_CREATED {
            return s;
        }
        s |= UWS_VISIBLE;
        if i == self.m.win.active {
            s |= UWS_ACTIVE;
        }
        if self.uiw_plat_state(i) & plat::WST_MINIMIZED != 0 {
            s |= UWS_MINIMIZED;
        }
        s
    }

    /// ui_window_activate(id): give the window the keyboard and put it on top
    pub fn window_activate(&mut self, id: u32) {
        let i = self.uiw_index(id);
        if i < 0 || !self.uiw_drawable(i as u32) {
            return;
        }
        self.uiw_activate(i as u32);
    }

    /// ui_window_destroy(id): hide the window and free its state and buffers (not while it
    /// is being built)
    pub fn window_destroy(&mut self, id: u32) {
        let i = self.uiw_index(id);
        if i <= 0 {
            return;
        }
        let i = i as u32;
        let iu = i as usize;
        if self.m.win.recs[iu].state & UWI_BUILDING != 0 || i == self.uiw_cur {
            self.warn("ui_window_destroy: the window is being built");
            return;
        }
        if self.m.win.recs[iu].state & UWI_VISIBLE != 0 {
            self.uiw_hide(i);
        }
        if self.m.win.recs[iu].state & UWI_CREATED != 0 {
            self.m.win.recs[iu].save = None; // its WinParts (command lists, dirty grid, fb)
            if self.m.win.hooks && !self.headless {
                self.uwp_destroy(i);
            }
        }
        self.m.win.used &= !(1 << i);
        self.m.win.recs[iu] = Rec::default();
        if !self.headless {
            PUBLISHED.lock().snap[iu] = [0; 6];
        }
        self.uiw_modal_update();
        self.publish_state();
    }

    /// ui_window_current -> id of the window being built (0 = main window)
    pub fn window_current(&self) -> u32 {
        self.uiw_rec_id(self.uiw_cur)
    }

    /// ui_window_modal -> id of the topmost modal window, 0 = none
    pub fn window_modal(&self) -> u32 {
        let m = self.m.win.modal;
        if m == 0 { 0 } else { self.uiw_rec_id(m) }
    }

    // ui_resize without the passes a headless ui_resize runs (no App here): a new
    // framebuffer size of the live window; the frame is built at the next opportunity
    fn resize_noapp(&mut self, w: i32, h: i32) {
        if w <= 0 || h <= 0 || (w == self.wp.rt.fb.w && h == self.wp.rt.fb.h) {
            return;
        }
        self.wp.rt.fb.resize(w, h);
        self.dirty_force_full();
        self.need = true;
    }

    // ------------------------------------------------------------------ automation helpers

    /// uiw_desk_pos(i) -> outer top-left of window i on the virtual desktop (DIP; the main
    /// window's client origin is 0, 0)
    pub(crate) fn uiw_desk_pos(&self, i: u32) -> (i32, i32) {
        let (mut x, mut y) = (0, 0);
        let mut b = i;
        for _ in 0..WIN_MAX {
            if b == 0 {
                break;
            }
            let r = &self.m.win.recs[(b as usize) & (WIN_MAX - 1)];
            let rc = r.rect.get();
            x += rc.x;
            y += rc.y;
            b = r.owner;
            if b == 0 {
                break;
            }
            // the owner's client starts below its title bar
            y += self.m.win.recs[(b as usize) & (WIN_MAX - 1)].tbh;
        }
        (x, y)
    }

    /// uiw_user_rect(i, x, y, w, h): the user moved (w = 0) or resized (x = i32::MIN) the
    /// window: WINRECT updated with WRF_CHANGED, the framebuffer follows (headless)
    pub(crate) fn uiw_user_rect(&mut self, i: u32, x: i32, y: i32, w: i32, h: i32) {
        if i == 0 || i as usize >= WIN_MAX {
            return;
        }
        let iu = i as usize;
        let mut rc = self.m.win.recs[iu].rect.get();
        if x != i32::MIN {
            rc.x = x;
            rc.y = y;
            rc.flags |= WRF_POS | WRF_CHANGED;
            self.m.win.recs[iu].state &= !UWI_DEFPOS;
        }
        if w > 0 {
            rc.w = w;
            if h > 0 {
                rc.h = h;
                rc.flags |= WRF_CHANGED;
            }
        }
        self.m.win.recs[iu].rect.set(rc);
        self.m.win.recs[iu].want |= UWW_RECT;
        self.invalidate();
    }

    // ------------------------------------------------------------------ OS windows (platform glue)
    // The UI windows become plat::win_* windows (the asm's uiw_plat_hooks table, set by the
    // runtime when it runs with a display).  UI index i and platform window index may
    // differ: u2p / p2u map them (0xFF = none); events are translated in process_events.
    // Positions: a WINRECT offset (DIP of the owner) is added to the owner's client origin
    // (its platform client origin + its library title bar).

    /// uiw_index + uiw_pw: (UI index of app window id (-1 none, 0 the main window), its
    /// platform window (-1 none))  // pn-core
    pub fn window_pw(&self, id: u32) -> (i32, i32) {
        let i = self.uiw_index(id);
        if i < 0 {
            return (i, -1);
        }
        (i, self.uiw_pw(i as u32))
    }

    /// uiw_pw(i) -> platform window of UI window i (-1 = none)
    pub(crate) fn uiw_pw(&self, i: u32) -> i32 {
        let p = self.m.win.u2p[(i as usize) & (WIN_MAX - 1)];
        if p == 0xFF { -1 } else { p as i32 }
    }

    /// uiw_from_pw(pw) -> UI window of platform window pw (-1 = none)
    pub(crate) fn uiw_from_pw(&self, pw: i32) -> i32 {
        if !(0..WIN_MAX as i32).contains(&pw) {
            return -1;
        }
        let u = self.m.win.p2u[pw as usize];
        if u == 0xFF { -1 } else { u as i32 }
    }

    // uiw_os: the windows are OS windows (a display, hooks set)
    fn uiw_os(&self) -> bool {
        !self.headless && self.m.win.hooks
    }

    /// uiw_plat_state(i) -> WST_* of the window's OS window (0: headless / none)
    pub(crate) fn uiw_plat_state(&self, i: u32) -> u32 {
        if !self.uiw_os() {
            return 0;
        }
        let pw = self.uiw_pw(i);
        if pw < 0 { 0 } else { plat::win_state(pw) }
    }

    /// uiw_app_active: the app is active (headless: always)
    pub(crate) fn uiw_app_active(&self) -> bool {
        if self.headless {
            return true;
        }
        plat::win_state(0) & plat::WST_ACTIVE != 0
    }

    // uiw_sysframe_ok: WF_SYSFRAME gets a system frame (not: Wayland without server-side
    // decorations, WST_CSD)
    fn uiw_sysframe_ok(&self) -> bool {
        if !self.uiw_os() {
            return true;
        }
        plat::win_state(0) & plat::WST_CSD == 0
    }

    // uiw_owner_origin(i) -> screen position (px) of the client origin of the window's
    // owner as the UI sees it (below a library title bar); OS windows ask the platform,
    // headless runs use the last reported position.  origin_ok = false when the platform
    // does not know it (Wayland; X11 before the window manager placed it)
    fn uiw_owner_origin(&mut self, i: u32) -> (i32, i32) {
        let owner = self.m.win.recs[(i as usize) & (WIN_MAX - 1)].owner;
        let ou = (owner as usize) & (WIN_MAX - 1);
        let (mut x, mut y) = (self.m.win.recs[ou].sx, self.m.win.recs[ou].sy);
        self.m.win.origin_ok = true;
        if self.uiw_os() {
            self.m.win.origin_ok = false;
            let pw = self.uiw_pw(owner);
            if pw >= 0 {
                if let Some(r) = plat::win_get_rect(pw) {
                    self.m.win.origin_ok = true;
                    x = r[0];
                    y = r[1];
                }
            }
        }
        if owner != 0 {
            // the owner's client starts below its title bar
            let dpi = self.win_dpi(owner);
            y += dip2px_at(self.m.win.recs[ou].tbh, dpi);
        }
        (x, y)
    }

    // uiw_attached(i): the centre of the window (its WINRECT) lies inside the client area
    // of its owner: such a window is docked to the owner and moves with it (paint.net
    // style); a window dragged out of its owner stays where it is on the screen
    fn uiw_attached(&self, i: u32) -> bool {
        let r = &self.m.win.recs[(i as usize) & (WIN_MAX - 1)];
        let rc = r.rect.get();
        let owner = r.owner;
        let odpi = self.win_dpi(owner);
        if odpi < 24 {
            return false;
        }
        let fb = self.win_fb(owner);
        let ow = px2dip_at(fb.w, odpi); // owner client width (DIP)
        let oh = px2dip_at(fb.h, odpi) - self.m.win.recs[(owner as usize) & (WIN_MAX - 1)].tbh;
        let cx = (rc.w >> 1) + rc.x; // centre (DIP from the owner's client origin)
        if cx < 0 || cx >= ow {
            return false;
        }
        let cy = ((rc.h + r.tbh) >> 1) + rc.y;
        cy >= 0 && cy < oh
    }

    // uiw_follow_owner(i): window i moved: the windows it owns that are docked to it
    // (uiw_attached) keep their WINRECT offsets and are moved along; the others stay where
    // they are on the screen (Windows, X11), so their offsets are taken again from the
    // platform
    fn uiw_follow_owner(&mut self, i: u32) {
        if !self.uiw_os() || self.m.win.follow as usize >= WIN_MAX {
            return; // (ownership has no cycles; a guard)
        }
        self.m.win.follow += 1;
        for j in 1..WIN_MAX as u32 {
            if j == i {
                continue;
            }
            let r = &self.m.win.recs[j as usize];
            if r.id == 0 || r.owner != i || !self.uiw_drawable(j) {
                continue;
            }
            if (self.m.win.placed >> j) & 1 == 0 || self.uiw_attached(j) {
                // its owner's position is known now (or it is docked): the WINRECT
                // position is applied (Win32 already moved it live during the drag: a
                // no-op then)
                self.m.win.recs[j as usize].want |= UWW_RECT;
                self.invalidate();
                continue;
            }
            let pw = self.uiw_pw(j);
            if pw < 0 {
                continue;
            }
            if let Some(r) = plat::win_get_rect(pw) {
                self.uiw_winpos(j, r[0], r[1]);
            }
        }
        self.m.win.follow -= 1;
    }

    // uiw_enable_sync(phase): OS windows follow the inert mask: phase 1 enables windows
    // that are no longer inert (before a modal window disappears), phase 2 disables inert
    // windows (after a new modal window was shown)
    fn uiw_enable_sync(&mut self, phase: u32) {
        if !self.m.win.hooks {
            return;
        }
        for i in 0..WIN_MAX as u32 {
            if self.uiw_pw(i) < 0 {
                continue;
            }
            let want = (self.m.win.inert >> i) & 1 == 0; // enabled
            let now = (self.m.win.disabled >> i) & 1 == 0;
            if want == now {
                continue;
            }
            if phase == 1 {
                if !want {
                    continue; // phase 1: enabling only
                }
                self.m.win.disabled &= !(1 << i);
            } else {
                if want {
                    continue; // phase 2: disabling only
                }
                self.m.win.disabled |= 1 << i;
            }
            self.uwp_enable(i, want);
        }
    }

    // ---- the hook table: UI window index -> plat::win_*

    // uwp_create(i, flags, owner) -> ok: the OS window (hidden) for a new UI window, as
    // large as its framebuffer
    fn uwp_create(&mut self, i: u32, flags: u32, owner: u32) -> bool {
        let i = i & (WIN_MAX as u32 - 1);
        let opw = self.uiw_pw(owner);
        if opw < 0 {
            return false; // the owner has no OS window
        }
        let dpi = self.win_dpi(i);
        let fb = self.win_fb(i);
        let (w, h) = (px2dip_at(fb.w, dpi), px2dip_at(fb.h, dpi));
        let title = self.m.win.recs[i as usize].title.clone();
        let pw = plat::win_create(&title, w, h, flags & plat::PWF_MASK, opw, plat::PW_DEFAULT, plat::PW_DEFAULT);
        if pw < 0 || pw as usize >= WIN_MAX {
            return false;
        }
        self.m.win.u2p[i as usize] = pw as u8;
        self.m.win.p2u[pw as usize] = i as u8;
        self.m.win.disabled &= !(1 << i);
        if flags & WF_RESIZABLE != 0 {
            let tbh = self.m.win.recs[i as usize].tbh;
            plat::win_set_min_size(pw, 120, tbh + 48);
        }
        self.publish_state();
        // it may be on a monitor with another dpi
        let d = plat::win_dpi(pw);
        if d != dpi {
            self.uiw_dpi(i, d);
        }
        true
    }

    // uwp_destroy(i): close its OS window
    fn uwp_destroy(&mut self, i: u32) {
        let i = i & (WIN_MAX as u32 - 1);
        let pw = self.uiw_pw(i);
        if pw < 0 {
            return;
        }
        let pw = pw as usize & (WIN_MAX - 1);
        self.m.win.p2u[pw] = 0xFF;
        self.m.win.u2p[i as usize] = 0xFF;
        self.m.win.disabled &= !(1 << i);
        self.m.win.placed &= !(1 << i);
        self.publish_state();
        plat::win_destroy(pw as i32);
    }

    // uwp_show(i, show): tool / WF_NOACTIVATE windows appear without taking the keyboard
    fn uwp_show(&mut self, i: u32, show: bool) {
        let pw = self.uiw_pw(i);
        if pw < 0 {
            return;
        }
        let mut mode = plat::PWS_HIDE;
        if show {
            mode = plat::PWS_SHOW;
            if self.m.win.recs[(i as usize) & (WIN_MAX - 1)].flags & (WF_TOOL | WF_NOACTIVATE) != 0 {
                mode = plat::PWS_SHOWNA;
            }
        }
        plat::win_show(pw, mode);
    }

    // uwp_rect(i, &rect): place / size the OS window: secondary windows at the owner's
    // client origin + the offset (when WRF_POS), as large as the framebuffer; the main
    // window (window_place(0)): screen px position, DIP client size, WRF_MAX
    fn uwp_rect(&mut self, i: u32, rc: &WinRect) {
        let pw = self.uiw_pw(i);
        if pw < 0 {
            return;
        }
        if i != 0 {
            let (fw, fh) = {
                let fb = self.win_fb(i);
                (fb.w, fb.h)
            };
            let (mut x, mut y) = (plat::PW_KEEP, plat::PW_KEEP);
            if rc.flags & WRF_POS != 0 {
                let owner = self.m.win.recs[(i as usize) & (WIN_MAX - 1)].owner;
                let odpi = self.win_dpi(owner);
                let (ox, oy) = self.uiw_owner_origin(i);
                if self.m.win.origin_ok {
                    self.m.win.placed |= 1 << i;
                    x = ox + dip2px_at(rc.x, odpi);
                    y = oy + dip2px_at(rc.y, odpi);
                }
                // not known yet: placed when it is (uiw_winpos of the owner)
            }
            plat::win_set_rect(pw, x, y, fw, fh);
            return;
        }
        let dpi = self.win_dpi(0);
        let st = plat::win_state(pw);
        if rc.flags & WRF_MAX != 0 {
            if st & plat::WST_MAXIMIZED == 0 {
                plat::win_cmd(pw, plat::WCMD_MAXIMIZE);
            }
            return;
        }
        if st & plat::WST_MAXIMIZED != 0 {
            plat::win_cmd(pw, plat::WCMD_RESTORE);
        }
        let (mut x, mut y) = (plat::PW_KEEP, plat::PW_KEEP);
        if rc.flags & WRF_POS != 0 {
            x = rc.x;
            y = rc.y;
        }
        let w = if rc.w > 0 { dip2px_at(rc.w, dpi) } else { plat::PW_KEEP };
        let h = if rc.h > 0 { dip2px_at(rc.h, dpi) } else { plat::PW_KEEP };
        plat::win_set_rect(pw, x, y, w, h);
        // a size change of our own is not reported back as E_RESIZE (Win32 WW_SELF): take
        // the client size the platform has now (U3)
        if w == plat::PW_KEEP && h == plat::PW_KEEP {
            return;
        }
        let (cw, ch) = plat::win_size(pw);
        if cw <= 0 || ch <= 0 {
            return;
        }
        self.resize_noapp(cw, ch); // (the main window is live here)
    }

    // uwp_title(i, s)
    fn uwp_title(&mut self, i: u32, s: &str) {
        let pw = self.uiw_pw(i);
        if pw >= 0 {
            plat::win_set_title(pw, s);
        }
    }

    // uwp_activate(i)
    fn uwp_activate(&mut self, i: u32) {
        let pw = self.uiw_pw(i);
        if pw >= 0 {
            plat::win_activate(pw);
        }
    }

    // uwp_enable(i, on)
    fn uwp_enable(&mut self, i: u32, on: bool) {
        let pw = self.uiw_pw(i);
        if pw >= 0 {
            plat::win_enable(pw, on);
        }
    }

    // ------------------------------------------------------------------ popup windows (U3, §6)
    // Popups (flyouts, menus, combo lists, the calendar, context menus) and tooltips of every
    // window may leave its framebuffer: the popup code's bounds ask uiw_popup_area for the
    // screen (OS windows: the work area of the monitor; flyouts flip above / to the left
    // where it ends) or the virtual desktop (headless: the script's `screen`, without one the
    // main window's client area -- the main window's popups then stay inside).  Every such
    // item gets a slot after the owner's render: a surface with the owner's popup and tooltip
    // layers replayed moved (render_popup).  With OS windows the platform shows it as a
    // borderless, non-activating popup window over the owner (plat::popup); its input comes
    // back as events of the owner (owner coordinates), so hit tests, focus and light dismiss
    // work as inside the window.  Wayland does not tell where a window is: popups that fit
    // stay inside it, the others get the compositor's placement (plat::popup_anchor: below the
    // anchor, flipped when there is no room) and the UI lays them out where it put them
    // (uiw_popup_nudge).  Headless runs composite the slots on the virtual desktop (`shot
    // desktop`, uiw_popups_blit).

    /// uiw_popup_area -> Some((x0, y0, x1, y1), known) (px, coordinates of the live window) when
    /// the live window's popups may leave it, else None.  known false: the platform does not
    /// tell where the window is (Wayland) -- a wide area that the compositor keeps on screen
    pub(crate) fn uiw_popup_area(&mut self) -> Option<([i32; 4], bool)> {
        let cur = self.uiw_cur;
        if cur as usize >= WIN_MAX {
            return None;
        }
        let mut a = [0i32; 4];
        let mut known = true;
        if self.uiw_os() {
            // OS windows: the work area of the monitor, relative to the window
            let pw = self.uiw_pw(cur);
            if pw < 0 {
                return None;
            }
            match (plat::win_get_rect(pw), plat::win_work_area(pw)) {
                (Some(wr), Some(wa)) => {
                    a[0] = wa[0] - wr[0];
                    a[2] = a[0] + wa[2];
                    a[1] = wa[1] - wr[1];
                    a[3] = a[1] + wa[3];
                }
                _ => {
                    // position unknown (Wayland): wide; the compositor keeps the popup on screen
                    a = [-4096, -4096, self.wp.rt.fb.w + 4096, self.wp.rt.fb.h + 4096];
                    known = false;
                }
            }
        } else {
            // headless: the virtual screen, else the main window's client (DIP)
            let scr = self.m.win.screen;
            let [sx, sy, sw, sh] = if scr[2] > 0 && scr[3] > 0 {
                scr
            } else if cur == 0 {
                return None;
            } else {
                let mdpi = self.win_dpi(0);
                let fb = self.win_fb(0);
                [0, 0, px2dip_at(fb.w, mdpi), px2dip_at(fb.h, mdpi)]
            };
            let (wx, wy) = self.uiw_desk_pos(cur); // outer top-left of the window (DIP)
            let dpi = self.wp.rt.dpi;
            a[0] = dip2px_at(sx - wx, dpi);
            a[1] = dip2px_at(sy - wy, dpi);
            a[2] = dip2px_at(sx + sw - wx, dpi);
            a[3] = dip2px_at(sy + sh - wy, dpi);
        }
        // never less than the window itself
        a[0] = a[0].min(0);
        a[1] = a[1].min(0);
        a[2] = a[2].max(self.wp.rt.fb.w);
        a[3] = a[3].max(self.wp.rt.fb.h);
        Some((a, known))
    }

    /// uiw_popup_nudge(item, key) -> (dx, dy) px: how far the platform moved item `item` of the
    /// live window from its natural place, computed as key (x, y, physical px) -- Wayland
    /// compositors flip / slide popups to keep them on the screen; the popup is then laid out
    /// there.  (0, 0): not moved, or the platform has not placed it yet
    pub(crate) fn uiw_popup_nudge(&self, item: u32, key: [i32; 2]) -> (i32, i32) {
        if !self.uiw_os() {
            return (0, 0);
        }
        let own = self.uiw_cur + 1;
        let Some(k) = (0..EXT_MAX).find(|&k| {
            let e = &self.m.win.ex[k];
            e.owner == own && e.item == item && e.shown && e.key == key
        }) else {
            return (0, 0);
        };
        let nat = self.m.win.ex[k].nat;
        match plat::popup_placed(k as u32) {
            Some(p) => (p[0] - nat[0], p[1] - nat[1]),
            None => (0, 0),
        }
    }

    /// uiw_popups_render(i): after the render of window i (live): the slots of its items
    /// outside the framebuffer, re-rendered when the window changed or the item moved;
    /// shown / moved / presented by the platform (OS windows), slots no longer used hidden.
    /// The main window (index 0): while a secondary window is shown, EVERY popup gets a
    /// slot, so menus and lists of the main window stay above the tool windows (headless:
    /// `shot desktop` blits them last).
    pub(crate) fn uiw_popups_render(&mut self, i: u32) {
        if i as usize >= WIN_MAX {
            return;
        }
        // the main window: every popup while a secondary window is shown, else (§6) those
        // outside the window
        let all = i == 0 && (1..WIN_MAX as u32).any(|j| self.uiw_drawable(j));
        let own = i + 1;
        // nothing used yet this round
        for e in self.m.win.ex.iter_mut() {
            if e.owner == own {
                e.used = false;
            }
        }
        for item in 0..POPUP_ITEMS {
            let rect = if all { self.popup_rect(item) } else { self.popup_ext(item) };
            let Some(rect) = rect else { continue };
            // the slot of this item, else a free one
            let k = match (0..EXT_MAX).find(|&k| self.m.win.ex[k].owner == own && self.m.win.ex[k].item == item) {
                Some(k) => k,
                None => match (0..EXT_MAX).find(|&k| self.m.win.ex[k].owner == 0) {
                    Some(k) => {
                        let e = &mut self.m.win.ex[k];
                        e.owner = own;
                        e.item = item;
                        e.shown = false;
                        e.rect[2] = 0;
                        k
                    }
                    None => {
                        self.limit_hit(super::ctx::LIM_POPUP_SLOTS); // all slots taken: drawn clipped
                        continue;
                    }
                },
            };
            // changed: the window rendered something, or the item moved / resized / is new
            let mut changed = self.wp.rt.present_n != 0;
            {
                let e = &mut self.m.win.ex[k];
                e.used = true;
                if e.rect != rect {
                    changed = true;
                    e.rect = rect;
                }
                if !e.shown {
                    changed = true;
                }
            }
            if !changed {
                continue;
            }
            // the surface: popup + tooltip layers of the owner, moved
            let mut sf = core::mem::take(&mut self.m.win.ex[k].sf);
            if sf.w != rect[2] || sf.h != rect[3] || sf.px.is_empty() {
                sf = Surface::new(rect[2], rect[3], FMT_XRGB);
                if sf.px.is_empty() {
                    self.m.win.ex[k].owner = 0;
                    continue;
                }
            }
            self.dc.begin(&sf);
            let c = self.color(T_FLYOUT) | 0xFF00_0000;
            self.dc.set_color(c);
            self.dc.fill(&mut sf, 0, 0, rect[2], rect[3]);
            self.render_popup(&mut sf, -rect[0], -rect[1]);
            self.dc.end();
            if self.uiw_os() {
                let pw = self.uiw_pw(i);
                if pw >= 0 {
                    // (§6) how it hangs at its anchor (Wayland places it from that)
                    let (anchor, kind, nat, key) =
                        self.popup_place(item).unwrap_or(([rect[0], rect[1], 1, 1], plat::PA_FREE, [rect[0], rect[1]], [rect[0], rect[1]]));
                    plat::popup_anchor(k as u32, anchor, kind, nat);
                    plat::popup(k as u32, pw, Some(rect), Some(&Ui::frame_of(&sf)));
                    let e = &mut self.m.win.ex[k];
                    e.shown = true;
                    e.nat = nat;
                    e.key = key;
                }
            }
            self.m.win.ex[k].sf = sf;
        }
        self.uiw_popups_drop_ex(i, true);
    }

    /// uiw_popups_drop(i): window i is not drawn: its popup slots go away
    pub(crate) fn uiw_popups_drop(&mut self, i: u32) {
        self.uiw_popups_drop_ex(i, false);
    }

    // uiw_popups_drop_ex(i, unused_only): free the slots of window i (with unused_only:
    // those its last render did not use); shown ones are hidden first
    fn uiw_popups_drop_ex(&mut self, i: u32, unused_only: bool) {
        let own = i + 1;
        for k in 0..EXT_MAX {
            let e = &self.m.win.ex[k];
            if e.owner != own || (unused_only && e.used) {
                continue;
            }
            if e.shown && self.uiw_os() {
                plat::popup(k as u32, 0, None, None); // hide
            }
            let e = &mut self.m.win.ex[k];
            e.sf = Surface::default();
            e.owner = 0;
            e.shown = false;
            e.used = false;
        }
    }

    /// uiw_popups_blit(sf, i, x0, y0, dpi): (headless `shot desktop`) the popup slots of
    /// window i onto sf, a desktop whose top-left is x0, y0 (DIP)
    pub(crate) fn uiw_popups_blit(&mut self, sf: &mut Surface, i: u32, x0: i32, y0: i32, dpi: i32) {
        let own = i + 1;
        let (wx, wy) = self.uiw_desk_pos(i);
        let wdpi = self.win_dpi(i);
        for k in 0..EXT_MAX {
            let e = &self.m.win.ex[k];
            if e.owner != own || !e.used || e.sf.px.is_empty() {
                continue;
            }
            let x = dip2px_at(px2dip_at(e.rect[0], wdpi) + wx - x0, dpi);
            let y = dip2px_at(px2dip_at(e.rect[1], wdpi) + wy - y0, dpi);
            blit_copy(sf, &e.sf, x, y);
        }
    }

    /// uiw_popups_dismiss(i): a press in window i: popups of every other window close
    /// (light dismiss across windows; a press on an OS popup window counts as its owner)
    pub(crate) fn uiw_popups_dismiss(&mut self, i: u32) {
        let mut swapped = false;
        for j in 0..WIN_MAX as u32 {
            if j == i || (j != 0 && !self.uiw_drawable(j)) {
                continue;
            }
            self.uiw_use(j);
            swapped = true;
            if self.popup_count() != 0 {
                self.popup_close();
                self.invalidate();
            }
        }
        if swapped {
            self.uiw_use(0);
        }
    }
}
