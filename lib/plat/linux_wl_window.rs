// linux_wl_window -- Wayland windows (indexed records): globals, xdg toplevels, configure,
// scaling, window commands and the event loop                    [src/plat/linux/wl_window.s]
//
//   win_create(title, w_dip, h_dip, flags, owner, x, y) -> index / -1
//        owner -1: the main window (registry, globals; its first configure is awaited).
//        Else a toplevel owned by `owner`: xdg_toplevel.set_parent(owner), client-side frame
//        unless PWF_SYSFRAME gets a server-side one (xdg-decoration), xdg_dialog_v1.set_modal
//        for PWF_MODAL when the compositor has xdg_wm_dialog_v1.  Created unmapped; x, y are
//        ignored (Wayland clients can neither place their windows nor learn where they are).
//   win_destroy(w)  win_show(w, mode)  (show = objects + the initial commit; the first
//        configure is awaited asynchronously, presents wait for it; hide = the window's
//        objects are destroyed: WSLg keeps the RAIL window of an unmapped toplevel, only a
//        destroyed wl_surface removes it)   win_activate / win_enable: nothing
//   win_set_rect(w, x, y, cw, ch)   size only (the position can not be set)
//   win_get_rect / win_work_area -> None (unknown on Wayland)
//   win_set_title  win_state -> WST_*  win_cmd  win_begin_move  win_begin_resize
//   win_size -> (w, h)  win_dpi  win_set_min_size (also before the window exists)
//   wait(deadline_us) -> queued events (0 = poll without blocking, u64::MAX = forever,
//        otherwise an absolute time_us() value)
//
// Logical size = DIP (the compositor's surface coordinates).  Physical size = logical *
// scale; the scale comes from wp_fractional_scale (with a viewport),
// wl_surface.preferred_buffer_scale or the largest wl_output.scale the surface is on.
// jfgui_WLDEBUG=1 traces every protocol message to stderr; jfgui_SCALE=N (1..4) pretends
// every output has integer scale N (HiDPI tests).
// WSLg (Weston RAIL, measured 1.0.79): a toplevel with set_parent gets its own RAIL window
// (own taskbar button, not owned in Windows terms), is centred on its parent when mapped, is
// not kept above it and stays visible while the parent is minimized.

use alloc::vec::Vec;

use super::proto::*;
use super::wire::{fatal, poll, Args, PollFd, A, EINTR, POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT};
use super::Wl;
use crate::{
    E_CLOSE, E_DPI, E_RESIZE, PWF_MASK, PWF_MODAL, PWF_RESIZABLE, PWF_SYSFRAME, PWS_SHOWNA, PW_KEEP, PW_MAX,
    PW_NOOWNER, WCMD_CLOSE, WCMD_MAXIMIZE, WCMD_MINIMIZE, WCMD_RESTORE, WST_ACTIVE, WST_CSD, WST_FULLSCREEN,
    WST_MAXIMIZED, WST_MINIMIZED, WST_TILED,
};

pub(crate) const OUT_MAX: usize = 8;
const POLL_MAX: usize = 12; // wayland fd + clipboard transfers + the wake eventfd
const STATE_BIG: u32 = WST_MAXIMIZED | WST_FULLSCREEN | WST_TILED;
const TITLE_MAX: usize = 256; // bytes per window incl. the NUL
pub(crate) const WLWF_USED: u32 = 0x100;
pub(crate) const WLWF_TOP: u32 = 0x200; // top-level window (no owner)

/// one wl_output
#[derive(Clone, Copy, Default)]
pub(crate) struct Out {
    pub id: u32, // wl_output object (0 = free slot)
    pub name: u32, // registry name
    pub scale: i32,
    pub mw: i32, // current mode width / height
    pub mh: i32,
    pub ver: u32,
}

/// the bound globals and process-wide window state
#[derive(Default)]
pub(crate) struct Globals {
    pub reg: u32,
    pub comp: u32,
    pub comp_ver: u32,
    pub shm: u32,
    pub seat: u32,
    pub seat_ver: u32,
    pub xdg_wm: u32,
    pub ddm: u32,
    pub ddm_ver: u32,
    pub vpr: u32,
    pub fsm: u32,
    pub csm: u32,
    pub decom: u32,
    pub dialogm: u32,
    pub tabm: u32,
    pub scale_ovr: i32,
    pub refresh: Option<fn()>, // stored only: Wayland has no modal size loop
    pub titles: [Vec<u8>; PW_MAX],
}

/// a window record (index 0 = the main window); its protocol objects carry the index as
/// udata, so the event handlers know their window
#[derive(Clone, Copy, Default)]
pub(crate) struct WinRec {
    pub surf: u32, // wl_surface (0 = not shown)
    pub xsurf: u32,
    pub top: u32,
    pub vp: u32,   // wp_viewport
    pub fs: u32,   // wp_fractional_scale_v1
    pub deco: u32, // zxdg_toplevel_decoration_v1
    pub deco_mode: u32,
    pub lw: i32, // logical (DIP) size
    pub lh: i32,
    pub pw: i32, // physical (buffer) size
    pub ph: i32,
    pub normal_w: i32, // logical size when not maximized
    pub normal_h: i32,
    pub minw: i32, // minimum size (DIP)
    pub minh: i32,
    pub flags: u32, // PWF_* | WLWF_*
    pub state: u32, // WST_*
    pub s120: i32,  // scale in 120ths
    pub frac: bool, // fractional scale (viewport)
    pub dpi: i32,
    pub pbs: i32,       // preferred_buffer_scale
    pub frac_pref: u32, // wp_fractional_scale preferred scale
    pub pend_w: i32,    // xdg_toplevel.configure until xdg_surface.configure
    pub pend_h: i32,
    pub pend_state: u32,
    pub ack_serial: u32,
    pub ack_pending: bool,
    pub mapped: bool,     // a buffer was attached since it was shown
    pub configured: bool, // the first configure since it was shown arrived
    pub resize_pending: bool,
    pub owner: i32, // owner index (-1 = top-level)
    pub shown: bool, // its initial state was committed (shown)
    pub outs: u32,   // bit per output slot the surface is on
    pub cursor: u32, // CUR_*
    pub hidmin: bool, // hidden because its owner was minimized
    pub dialog: u32, // xdg_dialog_v1
}

/// the globals we bind: name, interface, highest version we speak, lowest we accept
static GLOBALS: [(&str, &Iface, u32, u32); 11] = [
    ("wl_compositor", &IFACE_WL_COMPOSITOR, 6, 4),
    ("wl_shm", &IFACE_WL_SHM, 1, 1),
    ("wl_seat", &IFACE_WL_SEAT, 8, 1),
    ("xdg_wm_base", &IFACE_XDG_WM_BASE, 6, 1),
    ("wl_data_device_manager", &IFACE_WL_DDM, 3, 1),
    ("wp_viewporter", &IFACE_WP_VIEWPORTER, 1, 1),
    ("wp_fractional_scale_manager_v1", &IFACE_WP_FSM, 1, 1),
    ("wp_cursor_shape_manager_v1", &IFACE_WP_CSM, 1, 1),
    ("zxdg_decoration_manager_v1", &IFACE_ZXDG_DM, 1, 1),
    ("xdg_wm_dialog_v1", &IFACE_XDG_WM_DIALOG, 1, 1),
    ("zwp_tablet_manager_v2", &IFACE_TABLET_MGR, 1, 1), // (PN1) linux_wl_tablet.rs
];

/// leading decimal integer (parse_int: blanks, sign)
fn parse_int(s: &str) -> i32 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && b[i] == b'-' {
        neg = true;
        i += 1;
    } else if i < b.len() && b[i] == b'+' {
        i += 1;
    }
    let mut v = 0i32;
    while i < b.len() && b[i].is_ascii_digit() {
        v = v.wrapping_mul(10).wrapping_add((b[i] - b'0') as i32);
        i += 1;
    }
    if neg { v.wrapping_neg() } else { v }
}

/// window index -> its slot (w & 7, whether it exists or not)  [wlw_slot]
pub(crate) fn slot(w: i32) -> usize {
    (w as u32 as usize) & (PW_MAX - 1)
}

impl Wl {
    /// the environment, then the socket  [wl_plat_win_create's start + wl_connect]
    pub(crate) fn connect(&mut self) -> bool {
        if let Some(v) = crate::env("jfgui_WLDEBUG") {
            let c = v.as_bytes().first().copied().unwrap_or(0);
            if c != 0 && c != b'0' {
                self.w.debug = true;
            }
        }
        if let Some(v) = crate::env("jfgui_SCALE") {
            let s = parse_int(&v);
            if (1..=4).contains(&s) {
                self.g.scale_ovr = s; // test aid: pretend outputs have this scale
            }
        }
        // disp.s probing: quiet unless jfgui_BACKEND forces Wayland
        self.w.quiet = crate::disp_wl_quiet();
        self.w.connect()
    }

    /// window w if it exists (a hidden window has a record but no surface)  [wlw_rec]
    pub(crate) fn rec(&self, w: i32) -> Option<usize> {
        if (w as u32 as usize) < PW_MAX && self.win[w as usize].flags & WLWF_USED != 0 { Some(w as usize) } else { None }
    }

    /// index of the window with that wl_surface, -1 (surface 0 finds the main window while
    /// there is no connection: synthetic test events)  [wlw_find]
    pub(crate) fn find(&self, surface: u32) -> i32 {
        self.win.iter().position(|r| r.surf == surface).map_or(-1, |i| i as i32)
    }

    // ------------------------------------------------------------------ creation

    /// -> index / -1  [wl_plat_win_create]
    pub(crate) fn win_create(&mut self, title: &str, w_dip: i32, h_dip: i32, flags: u32, owner: i32, _x: i32,
                             _y: i32) -> i32 {
        let idx;
        if owner == PW_NOOWNER {
            if self.win[0].surf != 0 {
                return -1;
            }
            if self.g.reg == 0 {
                if self.w.fd < 0 && !self.connect() {
                    return -1; // no compositor: linux_disp tries X11 / reports
                }
                self.g.reg = self.w.send(1, &DISPLAY_GET_REGISTRY, &[A::N(&IFACE_WL_REGISTRY)]);
                if self.roundtrip() != 0 {
                    fatal("the compositor did not answer within 5 s");
                }
                for (name, id) in [("wl_compositor", self.g.comp), ("wl_shm", self.g.shm), ("xdg_wm_base", self.g.xdg_wm)] {
                    if id == 0 {
                        crate::write_stderr(alloc::format!("wayland: the compositor does not offer {name}\n").as_bytes());
                        crate::exit(1);
                    }
                }
                self.clip_init();
                self.tablet_init(); // (PN1) pen tablets
                if self.roundtrip() != 0 {
                    // outputs, seat capabilities
                    fatal("the compositor did not answer within 5 s");
                }
            }
            idx = 0;
        } else {
            if self.win[0].surf == 0 || self.rec(owner).is_none() {
                return -1;
            }
            match (1..PW_MAX).find(|&i| self.win[i].flags & WLWF_USED == 0) {
                Some(i) => idx = i,
                None => return -1,
            }
        }
        let r = &mut self.win[idx];
        let (minw, minh) = (r.minw, r.minh); // a minimum size set before creation stays
        *r = WinRec::default();
        r.minw = minw;
        r.minh = minh;
        r.flags = (flags & PWF_MASK) | WLWF_USED | if owner == PW_NOOWNER { WLWF_TOP } else { 0 };
        r.owner = owner;
        r.lw = w_dip;
        r.lh = h_dip;
        r.normal_w = w_dip;
        r.normal_h = h_dip;
        r.s120 = 120;
        r.dpi = 96;
        // the title is kept: the objects are made again each time the window is shown
        self.win_set_title(idx as i32, title.as_bytes());
        if owner != PW_NOOWNER {
            // until it is on an output: the owner's scale
            let o = self.win[slot(owner)];
            let r = &mut self.win[idx];
            r.s120 = o.s120;
            r.frac = o.frac;
            r.dpi = o.dpi;
            r.frac_pref = o.frac_pref;
            let (lw, lh) = (r.lw, r.lh);
            let pw = self.phys(lw, idx);
            let ph = self.phys(lh, idx);
            self.win[idx].pw = pw;
            self.win[idx].ph = ph;
            return idx as i32; // owned windows: objects when shown
        }
        self.objects_create(0);
        self.update_scale(0);
        self.win[0].shown = true;
        let s = self.win[0].surf;
        self.w.send(s, &SURFACE_COMMIT, &[]);
        // wait for the first configure
        let deadline = crate::time_us() + 5_000_000;
        loop {
            self.w.flush_wait();
            self.dispatch();
            if self.win[0].configured {
                break;
            }
            let now = crate::time_us();
            if now >= deadline {
                fatal("the compositor did not answer within 5 s");
            }
            if self.w.wait_readable(((deadline - now) / 1000) as i32 + 1) {
                self.w.recv();
            }
        }
        self.cursor_init();
        self.w.flush_wait();
        0
    }

    /// the protocol objects of a shown window: wl_surface (+ viewport, fractional scale),
    /// xdg_surface, xdg_toplevel (set_parent for owned windows, title, app id, size limits),
    /// decoration, xdg_dialog (not committed)  [wl_objects_create]
    fn objects_create(&mut self, w: usize) {
        let flags = self.win[w].flags;
        let wu = w as u64;
        let surf = self.w.send(self.g.comp, &COMPOSITOR_CREATE_SURFACE, &[A::N(&IFACE_WL_SURFACE)]);
        self.win[w].surf = surf;
        self.w.set_udata(surf, wu);
        if self.g.fsm != 0 && self.g.vpr != 0 {
            let vp = self.w.send(self.g.vpr, &VIEWPORTER_GET_VIEWPORT, &[A::N(&IFACE_WP_VIEWPORT), A::O(surf)]);
            self.win[w].vp = vp;
            let fs = self.w.send(self.g.fsm, &FSM_GET_FRACTIONAL_SCALE, &[A::N(&IFACE_WP_FS), A::O(surf)]);
            self.win[w].fs = fs;
            self.w.set_udata(fs, wu);
        }
        let xs = self.w.send(self.g.xdg_wm, &WM_BASE_GET_XDG_SURFACE, &[A::N(&IFACE_XDG_SURFACE), A::O(surf)]);
        self.win[w].xsurf = xs;
        self.w.set_udata(xs, wu);
        let top = self.w.send(xs, &XDG_SURFACE_GET_TOPLEVEL, &[A::N(&IFACE_XDG_TOPLEVEL)]);
        self.win[w].top = top;
        self.w.set_udata(top, wu);
        if flags & WLWF_TOP == 0 {
            // owned: its parent (0 while the owner is hidden)
            let parent = self.win[slot(self.win[w].owner)].top;
            self.w.send(top, &TOPLEVEL_SET_PARENT, &[A::O(parent)]);
        }
        let title = core::mem::take(&mut self.g.titles[w]);
        self.w.send(top, &TOPLEVEL_SET_TITLE, &[A::S(Some(&title))]);
        self.g.titles[w] = title;
        // app id = basename of argv[0]
        if let Some(a0) = crate::args().first() {
            let id = match a0.rfind('/') {
                Some(n) => &a0[n + 1..],
                None => &a0[..],
            };
            self.w.send(top, &TOPLEVEL_SET_APP_ID, &[A::S(Some(id.as_bytes()))]);
        }
        self.size_limits(w as i32);
        if self.g.decom != 0 {
            let d = self.w.send(self.g.decom, &DM_GET_TOPLEVEL_DECORATION, &[A::N(&IFACE_ZXDG_TD), A::O(top)]);
            self.win[w].deco = d;
            self.w.set_udata(d, wu);
            // server side, unless we draw our own
            let mode = if flags & PWF_SYSFRAME != 0 { 2 } else { 1 };
            self.w.send(d, &TD_SET_MODE, &[A::U(mode)]);
        }
        if flags & PWF_MODAL != 0 && self.g.dialogm != 0 {
            // xdg_wm_dialog_v1 (not on WSLg)
            let d = self.w.send(self.g.dialogm, &WM_DIALOG_GET_XDG_DIALOG, &[A::N(&IFACE_XDG_DIALOG), A::O(top)]);
            self.win[w].dialog = d;
            self.w.send(d, &DIALOG_SET_MODAL, &[]);
        }
    }

    /// the window disappears: its protocol objects are destroyed (on WSLg only a destroyed
    /// wl_surface removes the RAIL window: attach NULL + commit, or destroying the
    /// xdg_toplevel, leave it on the screen -- measured, WSLg 1.0.79)  [wl_objects_destroy]
    fn objects_destroy(&mut self, w: usize) {
        let r = self.win[w];
        if r.surf == 0 {
            return;
        }
        if r.dialog != 0 {
            self.w.send(r.dialog, &DIALOG_DESTROY, &[]);
        }
        if r.deco != 0 {
            self.w.send(r.deco, &TD_DESTROY, &[]);
        }
        self.w.send(r.top, &TOPLEVEL_DESTROY, &[]);
        self.w.send(r.xsurf, &XDG_SURFACE_DESTROY, &[]);
        if r.fs != 0 {
            self.w.send(r.fs, &FS_DESTROY, &[]);
        }
        if r.vp != 0 {
            self.w.send(r.vp, &VIEWPORT_DESTROY, &[]);
        }
        self.w.send(r.surf, &SURFACE_DESTROY, &[]);
        let r = &mut self.win[w];
        r.surf = 0;
        r.xsurf = 0;
        r.top = 0;
        r.vp = 0;
        r.fs = 0;
        r.deco = 0;
        r.dialog = 0;
        r.shown = false;
        r.configured = false;
        r.mapped = false;
        r.ack_pending = false;
        r.resize_pending = false;
        r.outs = 0;
        self.present_unmapped(w);
        if self.inp.ptr_win == w as i32 {
            self.inp.ptr_win = -1;
            self.inp.ptr_in = false;
        }
        self.w.flush();
    }

    /// xdg_toplevel min / max size: fixed windows min = max = the size, resizable ones the
    /// minimum (applied with the next commit)  [wl_size_limits]
    fn size_limits(&mut self, w: i32) {
        let Some(i) = self.rec(w) else { return };
        let r = self.win[i];
        if r.top == 0 {
            return;
        }
        if r.flags & PWF_RESIZABLE == 0 {
            self.w.send(r.top, &TOPLEVEL_SET_MIN_SIZE, &[A::I(r.lw), A::I(r.lh)]);
            self.w.send(r.top, &TOPLEVEL_SET_MAX_SIZE, &[A::I(r.lw), A::I(r.lh)]);
        } else if r.minw != 0 {
            self.w.send(r.top, &TOPLEVEL_SET_MIN_SIZE, &[A::I(r.minw), A::I(r.minh)]);
        }
    }

    /// minimum size (also before the window exists)  [wl_plat_win_set_min_size]
    pub(crate) fn win_set_min_size(&mut self, w: i32, w_dip: i32, h_dip: i32) {
        if w as u32 as usize >= PW_MAX {
            return;
        }
        let r = &mut self.win[w as usize];
        r.minw = w_dip;
        r.minh = h_dip;
        if r.surf == 0 || r.flags & PWF_RESIZABLE == 0 {
            return;
        }
        let s = r.surf;
        self.size_limits(w);
        self.w.send(s, &SURFACE_COMMIT, &[]);
        self.w.flush();
    }

    // ------------------------------------------------------------------ scale + size

    /// logical -> physical pixels at window w's scale  [win_phys]
    pub(crate) fn phys(&self, logical: i32, w: usize) -> i32 {
        let r = &self.win[w & (PW_MAX - 1)];
        let v = logical.wrapping_mul(r.s120);
        if r.frac { v.wrapping_add(60) / 120 } else { v / 120 }
    }

    /// recompute the physical size; E_RESIZE when it changed  [win_set_phys]
    fn set_phys(&mut self, w: usize) {
        let pw = self.phys(self.win[w].lw, w);
        let ph = self.phys(self.win[w].lh, w);
        let r = &mut self.win[w];
        if pw == r.pw && ph == r.ph {
            return;
        }
        r.pw = pw;
        r.ph = ph;
        if r.configured {
            r.resize_pending = true;
            self.inp.evwin = w as i32;
            self.push(E_RESIZE, 0, pw, ph, 0);
        }
    }

    /// the outputs changed: every window picks its scale again  [wl_update_scale_all]
    fn update_scale_all(&mut self) {
        for w in 0..PW_MAX {
            self.update_scale(w as i32);
        }
    }

    /// pick window w's scale; on change E_DPI, then E_RESIZE  [wl_update_scale]
    pub(crate) fn update_scale(&mut self, w: i32) {
        let i = slot(w);
        let r = self.win[i];
        let (s, frac) = if r.frac_pref != 0 && r.vp != 0 {
            (r.frac_pref as i32, true)
        } else {
            let mut v = self.g.scale_ovr;
            if v == 0 {
                v = r.pbs;
            }
            if v != 0 {
                (v.wrapping_mul(120), false)
            } else {
                // entered outputs, or every output while the surface is on none
                let mut best = 1;
                for (k, o) in self.outs.iter().enumerate() {
                    if o.id != 0 && (r.outs == 0 || r.outs & 1 << k != 0) && o.scale > best {
                        best = o.scale;
                    }
                }
                (best * 120, false)
            }
        };
        if s == r.s120 && frac == r.frac {
            return;
        }
        let dpi = ((s as u32).wrapping_mul(96).wrapping_add(60) / 120) as i32;
        let r = &mut self.win[i];
        r.s120 = s;
        r.frac = frac;
        r.dpi = dpi;
        if !r.configured {
            return;
        }
        self.inp.evwin = w;
        self.push(E_DPI, 0, 0, 0, dpi as u32);
        self.set_phys(i);
        if w == self.inp.ptr_win {
            self.cursor_apply();
        }
    }

    /// largest output mode width * height * 4  [wl_max_mode_bytes]
    pub(crate) fn max_mode_bytes(&self) -> usize {
        self.outs.iter().filter(|o| o.id != 0).map(|o| o.mw as u32 as usize * o.mh as u32 as usize * 4).max().unwrap_or(0)
    }

    // ------------------------------------------------------------------ window operations

    /// destroy the window's objects and buffers, free its record  [wl_plat_win_destroy]
    pub(crate) fn win_destroy(&mut self, w: i32) {
        let Some(i) = self.rec(w) else { return };
        self.objects_destroy(i);
        self.present_free(i);
        self.w.flush();
        if self.inp.kb_win == w {
            self.inp.kb_win = 0;
        }
        self.win[i] = WinRec::default();
    }

    /// show: its objects and the initial commit (the compositor answers with a configure;
    /// then the next present, or the last surface, attaches a buffer); hide: its objects
    /// are destroyed  [wl_plat_win_show]
    pub(crate) fn win_show(&mut self, w: i32, mode: u32) {
        let Some(i) = self.rec(w) else { return };
        self.win[i].hidmin = false;
        if mode == 0 {
            self.objects_destroy(i);
            return;
        }
        if self.win[i].shown {
            return;
        }
        if self.win[i].surf == 0 {
            self.objects_create(i);
        }
        self.win[i].shown = true;
        self.update_scale(w);
        let s = self.win[i].surf;
        self.w.send(s, &SURFACE_COMMIT, &[]);
        self.w.flush();
    }

    /// hide = true: hide the shown windows owned by w (they come back with hide = false,
    /// when w is activated again)  [wl_hide_owned]
    fn hide_owned(&mut self, w: usize, hide: bool) {
        for i in 0..PW_MAX {
            if i == w || self.rec(i as i32).is_none() || self.win[i].owner != w as i32 {
                continue;
            }
            if hide {
                if self.win[i].shown {
                    self.objects_destroy(i);
                    self.win[i].hidmin = true;
                }
            } else if self.win[i].hidmin {
                // its last surface comes back with the configure (present_service)
                self.win_show(i as i32, PWS_SHOWNA);
            }
        }
    }

    /// the size (the position can not be set on Wayland); applied with the next present, not
    /// reported back  [wl_plat_win_set_rect]
    pub(crate) fn win_set_rect(&mut self, w: i32, _x: i32, _y: i32, cw: i32, ch: i32) {
        let Some(i) = self.rec(w) else { return };
        if self.win[i].state & STATE_BIG != 0 {
            return; // maximized: the compositor's size
        }
        let mut changed = false;
        if cw != PW_KEEP {
            self.win[i].pw = cw;
            let l = self.phys_logical(cw, i);
            if l != self.win[i].lw {
                self.win[i].lw = l;
                self.win[i].normal_w = l;
                changed = true;
            }
        }
        if ch != PW_KEEP {
            self.win[i].ph = ch;
            let l = self.phys_logical(ch, i);
            if l != self.win[i].lh {
                self.win[i].lh = l;
                self.win[i].normal_h = l;
                changed = true;
            }
        }
        if changed {
            self.size_limits(w); // fixed-size windows: min = max = the size
        }
    }

    /// logical size of px physical pixels at window w's scale  [phys_logical]
    fn phys_logical(&self, px: i32, w: usize) -> i32 {
        let mut s = self.win[w & (PW_MAX - 1)].s120 as u32;
        if s == 0 {
            s = 120;
        }
        ((px as u32).wrapping_mul(120).wrapping_add(s >> 1) / s) as i32
    }

    /// the position is unknown on Wayland  [wl_plat_win_get_rect]
    pub(crate) fn win_get_rect(&mut self, _w: i32) -> Option<[i32; 4]> {
        None
    }

    /// unknown on Wayland (the assembly fills in the largest output mode and returns 0)
    /// [wl_plat_win_work_area]
    pub(crate) fn win_work_area(&mut self, _w: i32) -> Option<[i32; 4]> {
        None
    }

    /// (physical pixels)  [wl_plat_win_size]
    pub(crate) fn win_size(&self, w: i32) -> (i32, i32) {
        match self.rec(w) {
            Some(i) => (self.win[i].pw, self.win[i].ph),
            None => (0, 0),
        }
    }

    /// [wl_plat_win_dpi]
    pub(crate) fn win_dpi(&self, w: i32) -> i32 {
        let d = self.win[slot(w)].dpi;
        if d == 0 { 96 } else { d }
    }

    /// (kept per window: sent again when it is shown)  [wl_plat_win_set_title]
    pub(crate) fn win_set_title(&mut self, w: i32, title: &[u8]) {
        if w as u32 as usize >= PW_MAX {
            return;
        }
        let t = super::wire::cstr(title);
        let t = &t[..t.len().min(TITLE_MAX - 1)];
        self.g.titles[w as usize] = t.to_vec();
        let top = self.win[w as usize].top;
        if top != 0 {
            self.w.send(top, &TOPLEVEL_SET_TITLE, &[A::S(Some(t))]);
            self.w.flush();
        }
    }

    /// WCMD_MINIMIZE / MAXIMIZE / RESTORE / CLOSE; minimizing a window hides the windows it
    /// owns until it is activated again  [wl_plat_win_cmd]
    pub(crate) fn win_cmd(&mut self, w: i32, cmd: u32) {
        let Some(i) = self.rec(w) else { return };
        if cmd == WCMD_CLOSE {
            self.inp.evwin = w;
            self.push(E_CLOSE, 0, 0, 0, 0);
            return;
        }
        let top = self.win[i].top;
        if top == 0 {
            return; // hidden
        }
        match cmd {
            WCMD_MINIMIZE => {
                self.w.send(top, &TOPLEVEL_SET_MINIMIZED, &[]);
                self.win[i].state |= WST_MINIMIZED;
                self.hide_owned(i, true);
            }
            WCMD_MAXIMIZE => {
                self.w.send(top, &TOPLEVEL_SET_MAXIMIZED, &[]);
            }
            WCMD_RESTORE => {
                if self.win[i].state & WST_FULLSCREEN != 0 {
                    self.w.send(top, &TOPLEVEL_UNSET_FULLSCREEN, &[]);
                }
                if self.win[i].state & WST_MAXIMIZED != 0 {
                    self.w.send(top, &TOPLEVEL_UNSET_MAXIMIZED, &[]);
                }
            }
            _ => return,
        }
        self.w.flush();
    }

    /// WST_* bits of window w, WST_ACTIVE when one of the app's windows is activated, plus
    /// WST_CSD when the compositor draws no frame for it (no xdg-decoration, or it chose
    /// client-side): the UI draws its own title bar and resize band (WSLg, GNOME)
    /// [wl_plat_win_state]
    pub(crate) fn win_state(&self, w: i32) -> u32 {
        let r = &self.win[slot(w)];
        let mut s = r.state & !WST_ACTIVE;
        if self.win.iter().any(|x| x.state & WST_ACTIVE != 0 && x.surf != 0) {
            s |= WST_ACTIVE;
        }
        if self.g.decom == 0 || r.deco_mode != 2 {
            s |= WST_CSD;
        }
        s
    }

    /// let the compositor move the window (call on a button press)  [wl_plat_win_begin_move]
    pub(crate) fn win_begin_move(&mut self, w: i32) {
        let Some(i) = self.rec(w) else { return };
        let top = self.win[i].top;
        if top == 0 || self.g.seat == 0 {
            return;
        }
        self.w.send(top, &TOPLEVEL_MOVE, &[A::O(self.g.seat), A::U(self.inp.btn_serial)]);
        self.release_buttons();
        self.w.flush();
    }

    /// let the compositor resize from an edge (EDGE_* bits equal xdg_toplevel.resize_edge
    /// values)  [wl_plat_win_begin_resize]
    pub(crate) fn win_begin_resize(&mut self, w: i32, edges: u32) {
        let e = edges & 15;
        if e == 0 {
            return;
        }
        let Some(i) = self.rec(w) else { return };
        let top = self.win[i].top;
        if top == 0 || self.g.seat == 0 {
            return;
        }
        self.w.send(top, &TOPLEVEL_RESIZE, &[A::O(self.g.seat), A::U(self.inp.btn_serial), A::U(e)]);
        self.release_buttons();
        self.w.flush();
    }

    // ------------------------------------------------------------------ event loop

    /// configures that need no new frame (the window is mapped at the size it has):
    /// acknowledged right away  [wl_ack_idle]
    fn ack_idle(&mut self) {
        for i in 0..PW_MAX {
            let r = self.win[i];
            if r.surf == 0 || !r.ack_pending || !r.mapped || r.resize_pending {
                continue;
            }
            self.win[i].ack_pending = false;
            self.w.send(r.xsurf, &XDG_SURFACE_ACK_CONFIGURE, &[A::U(r.ack_serial)]);
            self.w.send(r.surf, &SURFACE_COMMIT, &[]);
        }
    }

    /// pump the compositor until an event is queued or the deadline passes -> queued
    /// events  [wl_plat_wait]
    pub(crate) fn wait(&mut self, deadline: u64) -> u32 {
        if self.win[0].surf == 0 {
            return crate::evq_count();
        }
        let mut polled = false; // the socket was polled once
        loop {
            self.dispatch();
            let now = crate::time_us();
            let mut next = self.input_service(now); // key repeat -> next deadline
            self.ack_idle();
            let p = self.present_service(now); // deferred frames -> next deadline
            if p < next {
                next = p;
            }
            self.w.flush();
            let n = crate::evq_count();
            if n != 0 {
                return n;
            }
            let ms: i32;
            if deadline == 0 || (deadline != u64::MAX && now >= deadline) {
                // no waiting, but always read what the compositor sent once per call (frame
                // callbacks, pings), even when the caller is late
                if polled {
                    return 0;
                }
                ms = 0;
            } else {
                if deadline != u64::MAX && deadline < next {
                    next = deadline;
                }
                ms = if next == u64::MAX {
                    -1
                } else if next <= now {
                    0
                } else {
                    ((next - now + 999) / 1000).min(0x7FFF_FFFF) as i32
                };
            }
            polled = true;
            let mut fds = [PollFd::default(); POLL_MAX];
            fds[0] = PollFd { fd: self.w.fd, events: POLLIN | if self.w.pending_out() != 0 { POLLOUT } else { 0 },
                              revents: 0 };
            let nclip = self.clip_pollfds(&mut fds[1..POLL_MAX - 1]);
            let wk = nclip + 1; // the wake eventfd after them (poll ignores fd -1)
            fds[wk] = PollFd { fd: crate::linux::thread::wake_fd(), events: POLLIN, revents: 0 };
            let r = poll(&mut fds[..wk + 1], ms);
            if r == -EINTR {
                continue;
            }
            if r < 0 {
                fatal("poll() failed");
            }
            let rev = fds[0].revents;
            if rev & POLLIN != 0 {
                self.w.recv();
            } else if rev & (POLLERR | POLLHUP | POLLNVAL) != 0 {
                fatal("connection to the compositor lost");
            }
            if fds[wk].revents & POLLIN != 0 {
                crate::evq_push(crate::linux::thread::wake_consume()); // wake() from another thread
            }
            let ready = fds;
            self.clip_service(&ready[1..1 + nclip]);
        }
    }
}

// ------------------------------------------------------------------ registry

/// wl_registry.global(name, interface, version): bind what we use  [wlh_global]
pub(crate) fn wlh_global(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    let name = a.u(0);
    let Some(iface) = a.s(1) else { return };
    let ver = a.u(2);
    if iface == b"zwp_text_input_manager_v3" && wl.ime.manager==0 {
        wl.ime.manager=wl.w.send(wl.g.reg,&REGISTRY_BIND,&[A::U(name),A::B(&super::ime::MANAGER,1)]);
        wl.ime_setup(); return;
    }
    if iface == b"wl_output" {
        let Some(k) = wl.outs.iter().position(|o| o.id == 0) else { return };
        let v = ver.min(4);
        let id = wl.w.send(wl.g.reg, &REGISTRY_BIND, &[A::U(name), A::B(&IFACE_WL_OUTPUT, v)]);
        wl.outs[k] = Out { id, name, scale: 1, mw: 0, mh: 0, ver: v };
        wl.w.set_udata(id, k as u64 + 1);
        return;
    }
    let Some(gi) = GLOBALS.iter().position(|g| g.0.as_bytes() == iface) else { return };
    let (gname, gif, maxv, minv) = GLOBALS[gi];
    if wl.global_id(gi) != 0 {
        return; // first one wins (e.g. seats)
    }
    if ver < minv {
        crate::write_stderr(alloc::format!("wayland: {gname} version {ver} is too old, not used\n").as_bytes());
        return;
    }
    let v = ver.min(maxv);
    let id = wl.w.send(wl.g.reg, &REGISTRY_BIND, &[A::U(name), A::B(gif, v)]);
    wl.set_global(gi, id, v);
    wl.ime_setup();
}

impl Wl {
    fn global_id(&self, gi: usize) -> u32 {
        let g = &self.g;
        [g.comp, g.shm, g.seat, g.xdg_wm, g.ddm, g.vpr, g.fsm, g.csm, g.decom, g.dialogm, g.tabm][gi]
    }

    fn set_global(&mut self, gi: usize, id: u32, v: u32) {
        let g = &mut self.g;
        match gi {
            0 => {
                g.comp = id;
                g.comp_ver = v;
            }
            1 => g.shm = id,
            2 => {
                g.seat = id;
                g.seat_ver = v;
            }
            3 => g.xdg_wm = id,
            4 => {
                g.ddm = id;
                g.ddm_ver = v;
            }
            5 => g.vpr = id,
            6 => g.fsm = id,
            7 => g.csm = id,
            8 => g.decom = id,
            9 => g.dialogm = id,
            _ => g.tabm = id,
        }
    }
}

/// an output went away  [wlh_global_remove]
pub(crate) fn wlh_global_remove(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    let name = a.u(0);
    for k in 0..OUT_MAX {
        let o = wl.outs[k];
        if o.id == 0 || o.name != name {
            continue;
        }
        wl.w.set_udata(o.id, 0);
        if o.ver >= 3 {
            wl.w.send(o.id, &OUTPUT_RELEASE, &[]);
        }
        wl.outs[k] = Out::default();
        for r in wl.win.iter_mut() {
            r.outs &= !(1 << k); // no window is on it any more
        }
        wl.update_scale_all();
    }
}

/// remember the current mode size  [wlh_output_mode]
pub(crate) fn wlh_output_mode(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    if u == 0 || a.u(0) & 1 == 0 {
        return; // WL_OUTPUT_MODE_CURRENT
    }
    let o = &mut wl.outs[(u - 1) as usize & (OUT_MAX - 1)];
    o.mw = a.i(1);
    o.mh = a.i(2);
}

/// [wlh_output_scale]
pub(crate) fn wlh_output_scale(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    if u == 0 {
        return;
    }
    wl.outs[(u - 1) as usize & (OUT_MAX - 1)].scale = a.i(0).clamp(1, 8);
}

/// output properties are complete  [wlh_output_done]
pub(crate) fn wlh_output_done(wl: &mut Wl, _id: u32, _a: &Args, _u: u64) {
    wl.update_scale_all();
}

/// surface.enter / leave bookkeeping  [win_out_mark]
fn win_out_mark(wl: &mut Wl, output: u32, entered: bool, w: u64) {
    let i = w as usize & (PW_MAX - 1);
    for k in 0..OUT_MAX {
        if output == 0 || wl.outs[k].id != output {
            continue;
        }
        wl.win[i].outs &= !(1 << k);
        if entered {
            wl.win[i].outs |= 1 << k;
        }
    }
    wl.update_scale(w as i32);
}

/// the surface entered an output (scale bookkeeping)  [wlh_surface_enter]
pub(crate) fn wlh_surface_enter(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    win_out_mark(wl, a.u(0), true, u);
}

/// the surface left an output  [wlh_surface_leave]
pub(crate) fn wlh_surface_leave(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    win_out_mark(wl, a.u(0), false, u);
}

/// wl_surface.preferred_buffer_scale (compositor v6)  [wlh_surface_pbs]
pub(crate) fn wlh_surface_pbs(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    wl.win[u as usize & (PW_MAX - 1)].pbs = a.i(0).max(1);
    wl.update_scale(u as i32);
}

/// wp_fractional_scale_v1.preferred_scale (120ths)  [wlh_frac_scale]
pub(crate) fn wlh_frac_scale(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    let v = a.u(0);
    wl.win[u as usize & (PW_MAX - 1)].frac_pref = if v < 60 { 120 } else { v };
    wl.update_scale(u as i32);
}

// ------------------------------------------------------------------ xdg shell events

/// xdg_wm_base.ping: answer at once  [wlh_ping]
pub(crate) fn wlh_ping(wl: &mut Wl, id: u32, a: &Args, _u: u64) {
    wl.w.send(id, &WM_BASE_PONG, &[A::U(a.u(0))]);
}

/// xdg_toplevel.configure(width, height, states)  [wlh_top_configure]
pub(crate) fn wlh_top_configure(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    let r = &mut wl.win[u as usize & (PW_MAX - 1)];
    r.pend_w = a.i(0);
    r.pend_h = a.i(1);
    let mut st = 0;
    for s in a.arr(2).chunks_exact(4) {
        match u32::from_le_bytes([s[0], s[1], s[2], s[3]]) {
            1 => st |= WST_MAXIMIZED,
            2 => st |= WST_FULLSCREEN,
            4 => st |= WST_ACTIVE,
            5..=8 => st |= WST_TILED,
            _ => {}
        }
    }
    r.pend_state = st;
}

/// xdg_surface.configure(serial): latch the state  [wlh_xdg_configure]
pub(crate) fn wlh_xdg_configure(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    let i = u as usize & (PW_MAX - 1);
    wl.win[i].ack_serial = a.u(0);
    wl.win[i].ack_pending = true;
    let old = wl.win[i].state;
    let mut new = wl.win[i].pend_state;
    if new & WST_ACTIVE != 0 {
        if old & WST_MINIMIZED != 0 {
            // activated again after a minimize: the windows it hid come back
            wl.hide_owned(i, false);
        }
    } else {
        new |= old & WST_MINIMIZED; // stays minimized until activated again
    }
    let r = &mut wl.win[i];
    r.state = new;
    let (mut w, mut h) = (r.pend_w, r.pend_h);
    if w <= 0 || h <= 0 {
        // no size suggested: back to the normal size when leaving a maximized (or tiled /
        // fullscreen) state, else keep ours
        w = r.lw;
        h = r.lh;
        if old & STATE_BIG != 0 && new & STATE_BIG == 0 {
            w = r.normal_w;
            h = r.normal_h;
        }
    }
    if new & STATE_BIG == 0 {
        r.normal_w = w;
        r.normal_h = h;
    }
    r.lw = w;
    r.lh = h;
    wl.set_phys(i);
    wl.win[i].configured = true;
}

/// the user asked to close the window  [wlh_top_close]
pub(crate) fn wlh_top_close(wl: &mut Wl, _id: u32, _a: &Args, u: u64) {
    wl.inp.evwin = (u as usize & (PW_MAX - 1)) as i32;
    wl.push(E_CLOSE, 0, 0, 0, 0);
}

/// decoration mode chosen by the compositor  [wlh_deco_configure]
pub(crate) fn wlh_deco_configure(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    wl.win[u as usize & (PW_MAX - 1)].deco_mode = a.u(0);
}

