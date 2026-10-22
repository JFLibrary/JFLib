// linux_x11_window -- X11 windows (indexed records): creation, properties, window manager
// hints and messages, DPI, window state and the event loop    [src/plat/linux/x11_window.s]
//
//   connect_main() -> bool          jfgui_SCALE, the connection, our atoms, the DPI
//   win_create(title, w_dip, h_dip, flags, owner, x, y) -> index / -1
//        owner -1: the main window; else a window owned by `owner`: WM_TRANSIENT_FOR,
//        _NET_WM_WINDOW_TYPE_UTILITY (PWF_TOOL; DIALOG under Weston's XWM, which never
//        shows UTILITY windows) or _DIALOG, _NET_WM_STATE {SKIP_TASKBAR, SKIP_PAGER,
//        (MODAL)}, WM_NORMAL_HINTS (USPosition | PPosition | PMinSize, PMaxSize when not
//        resizable), WM_HINTS input (False with PWF_NOACTIVATE) + window group = main,
//        _MOTIF_WM_HINTS without decorations unless PWF_SYSFRAME.  Created unmapped.
//   win_destroy / win_show / win_activate / win_set_rect / win_get_rect / win_work_area /
//   win_set_title / win_state / win_cmd / moveresize / begin_resize / win_size /
//   win_set_min_size (also before the window exists)
//   wait(deadline_us) -> queued events   deadline 0 = poll without blocking, u64::MAX =
//        wait forever, otherwise an absolute time_us() value
//   event(packet)                   one X event (dispatch)
//   rec(w) -> index of an existing window / find(xid) -> index / -1 / hide_owned(w, hide)
//
// Sizes are physical pixels; DIP sizes are scaled by dpi / 96.  The DPI is Xft.dpi from the
// root window's RESOURCE_MANAGER (96 when absent), jfgui_SCALE=N (1..4) forces N * 96; a
// change of RESOURCE_MANAGER sends E_DPI to every window and resizes them to keep their DIP
// size.  Positions: ConfigureNotify / MapNotify are followed by TranslateCoordinates
// (-> E_WINPOS); a window mapped again gets its position again (USPosition +
// ConfigureWindow).  Work that needs a round trip (state, DPI, keymap, positions) is flagged
// by the event handlers and done in wait().  Every event carries its window (push adds
// evwin, set per event).

use alloc::format;
use alloc::vec::Vec;

use super::*;
use crate::{
    E_CLOSE, E_DPI, E_RESIZE, E_WINPOS, PW_DEFAULT, PW_KEEP, PW_NOOWNER, PWF_DIALOG, PWF_MASK,
    PWF_MODAL, PWF_NOACTIVATE, PWF_RESIZABLE, PWF_SYSFRAME, PWF_TOOL, PWS_SHOWNA, WCMD_CLOSE, WCMD_MAXIMIZE,
    WCMD_MINIMIZE, WCMD_RESTORE, WST_ACTIVE, WST_FULLSCREEN, WST_MAXIMIZED, WST_MINIMIZED, WST_TILED,
};

const TITLE_MAX: usize = 1024;
const WANT_DPI: u32 = 2; // X11::want bits (global work)
const WANT_KEYMAP: u32 = 4;
const WANT_WIN: u32 = 8; // some window has want bits
const STATE_BIG: u32 = WST_MAXIMIZED | WST_FULLSCREEN | WST_TILED;
const HINTS_WORDS: usize = 18; // WM_SIZE_HINTS
const US_POSITION: u32 = 1;
const P_POSITION: u32 = 4;
const P_MIN_SIZE: u32 = 16;
const P_MAX_SIZE: u32 = 32;
const GC_GRAPHICS_EXPOSURES: u32 = 0x10000;
const NET_STATE_REMOVE: u32 = 0;
const NET_STATE_ADD: u32 = 1;
// own atoms (X11::xw_atoms)
const B_TYPE_UTILITY: usize = 0;
const B_TYPE_DIALOG: usize = 1;
const B_SKIP_TASKBAR: usize = 2;
const B_SKIP_PAGER: usize = 3;
const B_STATE_MODAL: usize = 4;
const B_ACTIVE_WINDOW: usize = 5;
pub(super) const B_FRAME_EXTENTS: usize = 6;
const B_WM_CHECK: usize = 7;
const B_USER_TIME: usize = 8;
pub(super) const B_COUNT: usize = 9;
const XW_ATOM_NAMES: [&str; B_COUNT] = [
    "_NET_WM_WINDOW_TYPE_UTILITY",
    "_NET_WM_WINDOW_TYPE_DIALOG",
    "_NET_WM_STATE_SKIP_TASKBAR",
    "_NET_WM_STATE_SKIP_PAGER",
    "_NET_WM_STATE_MODAL",
    "_NET_ACTIVE_WINDOW",
    "_NET_FRAME_EXTENTS",
    "_NET_SUPPORTING_WM_CHECK",
    "_NET_WM_USER_TIME",
];
// EDGE_* bits -> _NET_WM_MOVERESIZE direction (0xFF = none)
const EDGE_DIR: [u8; 16] = [0xFF, 1, 5, 0xFF, 7, 0, 6, 0xFF, 3, 2, 4, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

/// poll the X socket and the wake eventfd; a wake-up is consumed (E_WAKE queued) -> revents
/// of the socket (0: timeout, EINTR or only a wake-up)  [plat_wake_poll]
fn wake_poll(fd: i32, events: u16, ms: i32) -> u16 {
    let wfd = super::super::thread::wake_fd();
    let mut pfd = [(fd as u32 as u64) | (events as u64) << 32, (wfd as u32 as u64) | (POLLIN as u64) << 32];
    // SAFETY: two pollfds.
    let r = unsafe { rt::os::syscall3(SYS_POLL, pfd.as_mut_ptr() as usize, 2, ms as isize as usize) };
    if r <= 0 {
        return 0;
    }
    if (pfd[1] >> 48) as u16 & POLLIN != 0 {
        crate::evq_push(super::super::thread::wake_consume());
    }
    (pfd[0] >> 48) as u16
}

impl X11 {
    // ------------------------------------------------------------------ records

    /// x11w_rec(w): Some(index) when window w exists
    pub(super) fn rec(&self, w: i32) -> Option<usize> {
        let i = w as u32 as usize;
        if i < PW_MAX && self.xw[i].win != 0 { Some(i) } else { None }
    }

    /// x11w_find(xid) -> index of our window with that X id, -1 (xid 0 finds the main window
    /// while there is no connection: synthetic test events)
    pub(super) fn find(&self, xid: u32) -> i32 {
        self.xw.iter().position(|r| r.win == xid).map_or(-1, |i| i as i32)
    }

    // ------------------------------------------------------------------ creation

    /// the connection part of x11_plat_win_create for the main window: jfgui_SCALE, connect,
    /// our atoms, the DPI
    pub(super) fn connect_main(&mut self) -> bool {
        if let Some(v) = crate::env("jfgui_SCALE") {
            let n = parse_int(v.as_bytes()).0;
            if (1..=4).contains(&n) {
                self.scale_ovr = n;
            }
        }
        if !self.connect() {
            return false;
        }
        self.intern_own();
        self.dpi = self.read_dpi();
        true
    }

    /// x11_plat_win_create(title, w_dip, h_dip, flags, owner, x, y) -> index / -1
    pub(super) fn win_create(&mut self, title: &str, w_dip: i32, h_dip: i32, flags: u32, owner: i32, x: i32, y: i32) -> i32 {
        let idx: usize;
        if owner == PW_NOOWNER {
            if self.xw[0].win != 0 || self.fd < 0 {
                return -1;
            }
            idx = 0;
        } else {
            if self.xw[0].win == 0 || self.rec(owner).is_none() {
                return -1; // no connection / no owner
            }
            match (1..PW_MAX).find(|&i| self.xw[i].flags & XWF_USED == 0) {
                Some(i) => idx = i,
                None => return -1,
            }
        }
        // a minimum size set before creation stays
        let (minw, minh) = (self.xw[idx].minw, self.xw[idx].minh);
        let mut r = XW0;
        r.minw = minw;
        r.minh = minh;
        r.flags = flags & PWF_MASK | XWF_USED | if owner == PW_NOOWNER { XWF_TOP } else { 0 };
        r.owner = owner;
        r.reqx = PW_KEEP;
        r.reqy = PW_KEEP;
        r.w = self.dip_px(w_dip);
        r.h = self.dip_px(h_dip);
        // position: asked, or next to the owner (the manager may place it)
        if x != PW_DEFAULT {
            r.reqx = x;
            r.reqy = y;
        } else if owner != PW_NOOWNER {
            let o = self.xw[owner as usize];
            if o.posok {
                let d = self.dip_px(32);
                r.reqx = o.rx + d;
                r.reqy = o.ry + d;
            }
        }
        r.win = self.new_id();
        self.xw[idx] = r;
        let win = r.win;
        // colormap for a visual other than the root's (created once)
        if self.need_cmap && self.cmap == 0 {
            self.cmap = self.new_id();
            let (o, _) = self.req(XR_CREATE_COLORMAP, 16); // alloc None
            let (c, root, vis) = (self.cmap, self.root, self.visual);
            wr32(&mut self.out, o, c);
            wr32(&mut self.out, o + 4, root);
            wr32(&mut self.out, o + 8, vis);
        }
        let (op, bytes) = if self.cmap != 0 { (XR_CREATE_WINDOW | 24 << 8, 32 + 20) } else { (XR_CREATE_WINDOW, 32 + 16) };
        let (o, _) = self.req(op, bytes);
        let (px, py) = if r.reqx == PW_KEEP { (0, 0) } else { (r.reqx, r.reqy) };
        let mut mask = XCW_BACK_PIXMAP | XCW_BORDER_PIXEL | XCW_BIT_GRAVITY | XCW_EVENT_MASK;
        if self.cmap != 0 {
            mask |= XCW_COLORMAP;
        }
        let (root, vis, cmap) = (self.root, self.visual, self.cmap);
        let b = &mut self.out;
        wr32(b, o, win);
        wr32(b, o + 4, root);
        wr16(b, o + 8, px as u32);
        wr16(b, o + 10, py as u32);
        wr16(b, o + 12, r.w as u32);
        wr16(b, o + 14, r.h as u32);
        wr16(b, o + 16, 0); // border width
        wr16(b, o + 18, 1); // InputOutput
        wr32(b, o + 20, vis);
        wr32(b, o + 24, mask);
        wr32(b, o + 28, 0); // background None (no flicker)
        wr32(b, o + 32, 0); // border pixel
        wr32(b, o + 36, 1); // bit gravity NorthWest
        wr32(b, o + 40, XM_WINDOW);
        if cmap != 0 {
            wr32(b, o + 44, cmap);
        }
        self.win_set_title(idx as i32, title);
        // WM_CLASS = basename(argv[0]) NUL "jfgui" NUL
        let a0 = crate::args().into_iter().next();
        let base: Vec<u8> = match a0 {
            Some(a) => match a.as_bytes().iter().rposition(|&c| c == b'/') {
                Some(p) => a.as_bytes()[p + 1..].to_vec(),
                None => a.into_bytes(),
            },
            None => b"jfgui".to_vec(),
        };
        let mut class = base[..clip_len(&base, 100)].to_vec();
        class.push(0);
        class.extend_from_slice(b"jfgui\0");
        self.change_property(win, XA_WM_CLASS, XA_STRING, 8, 0, &class);
        self.dnd_window(win); // K1: XdndAware
        // WM_PROTOCOLS
        let (del, ping) = (self.atoms[A_WM_DELETE_WINDOW], self.atoms[A_NET_WM_PING]);
        self.change_prop32(win, self.atoms[A_WM_PROTOCOLS], XA_ATOM, &[del, ping]);
        // _NET_WM_PID
        // SAFETY: getpid has no arguments.
        let pid = unsafe { rt::os::syscall3(SYS_GETPID, 0, 0, 0) } as u32;
        self.change_prop32(win, self.atoms[A_NET_WM_PID], XA_CARDINAL, &[pid]);
        // WM_CLIENT_MACHINE
        let node = self.nodename.clone();
        self.change_property(win, XA_WM_CLIENT_MACHINE, XA_STRING, 8, 0, &node);
        // WM_HINTS: input (not for PWF_NOACTIVATE) + normal state (+ window group)
        let mut hints = [0u32; 9];
        hints[0] = 3; // InputHint | StateHint
        hints[1] = if flags & PWF_NOACTIVATE != 0 { 0 } else { 1 };
        hints[2] = 1; // NormalState
        if idx != 0 {
            hints[0] |= 64; // WindowGroupHint: the main window
            hints[8] = self.xw[0].win;
        }
        self.change_prop32(win, XA_WM_HINTS, XA_WM_HINTS, &hints);
        // _NET_WM_WINDOW_TYPE
        let mut typ = self.atoms[A_NET_WM_WINDOW_TYPE_NORMAL];
        if idx != 0 && flags & (PWF_TOOL | PWF_DIALOG) != 0 {
            typ = self.xw_atoms[B_TYPE_DIALOG];
            if flags & PWF_TOOL != 0 {
                typ = self.tool_type();
            }
        }
        self.change_prop32(win, self.atoms[A_NET_WM_WINDOW_TYPE], XA_ATOM, &[typ]);
        if idx != 0 {
            // owned: WM_TRANSIENT_FOR, _NET_WM_STATE
            let ow = self.xw[owner as usize].win;
            self.change_prop32(win, XA_WM_TRANSIENT_FOR, XA_WINDOW, &[ow]);
            if flags & (PWF_TOOL | PWF_DIALOG) != 0 {
                let mut st = Vec::from([self.xw_atoms[B_SKIP_TASKBAR], self.xw_atoms[B_SKIP_PAGER]]);
                if flags & PWF_MODAL != 0 {
                    st.push(self.xw_atoms[B_STATE_MODAL]);
                }
                self.change_prop32(win, self.atoms[A_NET_WM_STATE], XA_ATOM, &st);
            }
        }
        // no window manager frame when the app draws its own
        if flags & PWF_SYSFRAME == 0 {
            let a = self.atoms[A_MOTIF_WM_HINTS];
            self.change_prop32(win, a, a, &[2, 0, 0, 0, 0]); // MWM_HINTS_DECORATIONS, decorations 0
        }
        self.update_hints(idx as i32);
        if idx == 0 {
            // the main window: root RESOURCE_MANAGER changes (Xft.dpi), a GC, input, clipboard
            let (o, _) = self.req(XR_CHANGE_WINDOW_ATTR, 16);
            let root = self.root;
            wr32(&mut self.out, o, root);
            wr32(&mut self.out, o + 4, XCW_EVENT_MASK);
            wr32(&mut self.out, o + 8, XM_PROPERTY);
            self.gc = self.new_id();
            let (o, _) = self.req(XR_CREATE_GC, 20);
            let gc = self.gc;
            wr32(&mut self.out, o, gc);
            wr32(&mut self.out, o + 4, win);
            wr32(&mut self.out, o + 8, GC_GRAPHICS_EXPOSURES);
            wr32(&mut self.out, o + 12, 0);
            self.input_init();
            self.clip_init();
            self.xi_init(); // (PN1) XInput 2 pens
            self.screen_init(); // RandR changes, the work area (linux_x11_screen.rs)
        }
        self.xi_select(win); // (PN1) the pens' events on this window
        self.flush();
        idx as i32
    }

    /// x11w_intern: the backend's own atoms (one batch)
    fn intern_own(&mut self) {
        let mut seqs = [0u32; B_COUNT];
        for (i, n) in XW_ATOM_NAMES.iter().enumerate() {
            seqs[i] = self.intern_send(n.as_bytes(), false);
        }
        for i in 0..B_COUNT {
            self.xw_atoms[i] = self.atom_reply(seqs[i]);
        }
    }

    /// x11w_tool_type: _NET_WM_WINDOW_TYPE of tool windows -- UTILITY, but DIALOG under
    /// Weston's XWM (WSLg), which never maps UTILITY windows; jfgui_X11TOOL=utility|dialog
    /// forces one
    fn tool_type(&mut self) -> u32 {
        if self.tooltype != 0 {
            return self.tooltype;
        }
        let mut t = self.xw_atoms[B_TYPE_UTILITY];
        let forced = crate::env("jfgui_X11TOOL").and_then(|v| v.bytes().next());
        match forced {
            Some(b'u') => {}
            Some(b'd') => t = self.xw_atoms[B_TYPE_DIALOG],
            _ => {
                // _NET_SUPPORTING_WM_CHECK -> _NET_WM_NAME
                let root = self.root;
                if let Some(r) = self.get_property(root, self.xw_atoms[B_WM_CHECK], XA_WINDOW, 0, 1, false) {
                    if rd32(&r, 16) >= 1 {
                        let wm = rd32(&r, 32);
                        if let Some(n) = self.get_property(wm, self.atoms[A_NET_WM_NAME], 0, 0, 16, false) {
                            if rd32(&n, 16) >= 6 && &n[32..38] == b"Weston" {
                                t = self.xw_atoms[B_TYPE_DIALOG];
                            }
                        }
                    }
                }
            }
        }
        self.tooltype = t;
        t
    }

    /// x11_dip_px(dip) -> physical pixels (rounded)
    pub(super) fn dip_px(&self, dip: i32) -> i32 {
        dip.wrapping_mul(self.dpi).wrapping_add(48) / 96
    }

    /// x11_plat_win_set_min_size(w, w_dip, h_dip): minimum client size (also before creation)
    pub(super) fn win_set_min_size(&mut self, w: i32, wd: i32, hd: i32) {
        let i = w as u32 as usize;
        if i >= PW_MAX {
            return;
        }
        self.xw[i].minw = wd;
        self.xw[i].minh = hd;
        if self.xw[i].win != 0 {
            self.update_hints(w);
            self.flush();
        }
    }

    /// x11_update_hints(w): WM_NORMAL_HINTS -- minimum size (fixed size if not resizable),
    /// the asked position (USPosition)
    fn update_hints(&mut self, w: i32) {
        let Some(i) = self.rec(w) else { return };
        let r = self.xw[i];
        let mut h = [0u32; HINTS_WORDS];
        h[3] = r.w as u32; // (obsolete) size
        h[4] = r.h as u32;
        if r.flags & PWF_RESIZABLE != 0 {
            if r.minw != 0 {
                h[5] = self.dip_px(r.minw) as u32;
                h[6] = self.dip_px(r.minh) as u32;
                h[0] = P_MIN_SIZE;
            }
        } else {
            h[0] = P_MIN_SIZE | P_MAX_SIZE;
            h[5] = r.w as u32;
            h[7] = r.w as u32;
            h[6] = r.h as u32;
            h[8] = r.h as u32;
        }
        if r.reqx != PW_KEEP {
            h[0] |= US_POSITION | P_POSITION;
            h[1] = r.reqx.wrapping_sub(r.framel) as u32;
            h[2] = r.reqy.wrapping_sub(r.framet) as u32;
        }
        self.change_prop32(r.win, XA_WM_NORMAL_HINTS, XA_WM_SIZE_HINTS, &h);
    }

    /// x11w_configure(w, mask, x, y, cw, ch): ConfigureWindow (mask: 1 x, 2 y, 4 w, 8 h); x, y
    /// are client positions (the manager's frame extents are taken off)
    fn configure(&mut self, w: i32, mask: u32, x: i32, y: i32, cw: i32, ch: i32) {
        let Some(i) = self.rec(w) else { return };
        let mask = mask & 15;
        if mask == 0 {
            return;
        }
        let r = self.xw[i];
        let (o, _) = self.req(XR_CONFIGURE_WINDOW, 12 + 4 * mask.count_ones() as usize);
        wr32(&mut self.out, o, r.win);
        wr16(&mut self.out, o + 4, mask);
        let mut p = o + 8;
        for (bit, v) in [(1, x.wrapping_sub(r.framel)), (2, y.wrapping_sub(r.framet)), (4, cw), (8, ch)] {
            if mask & bit != 0 {
                wr32(&mut self.out, p, v as u32);
                p += 4;
            }
        }
    }

    // ------------------------------------------------------------------ window operations

    /// x11_plat_win_destroy(w): destroy the window and its presentation buffers
    pub(super) fn win_destroy(&mut self, w: i32) {
        let Some(i) = self.rec(w) else { return };
        let (o, _) = self.req(XR_DESTROY_WINDOW, 8);
        let win = self.xw[i].win;
        wr32(&mut self.out, o, win);
        self.present_free(i);
        self.flush();
        self.xw[i] = XW0;
    }

    /// x11_plat_win_show(w, mode): PWS_HIDE unmaps; PWS_SHOW / PWS_SHOWNA map it at its
    /// position again (SHOWNA: _NET_WM_USER_TIME 0 asks the manager not to focus it)
    pub(super) fn win_show(&mut self, w: i32, mode: u32) {
        let Some(i) = self.rec(w) else { return };
        if mode == 0 {
            self.xw[i].hidmin = false;
            if !self.xw[i].shown {
                return;
            }
            self.xw[i].shown = false;
            let (o, _) = self.req(XR_UNMAP_WINDOW, 8);
            let win = self.xw[i].win;
            wr32(&mut self.out, o, win);
        } else {
            if self.xw[i].shown {
                return;
            }
            self.xw[i].shown = true;
            self.map(w, mode);
        }
        self.flush();
    }

    /// x11w_map(w, mode): MapWindow at the last known (or asked) position
    fn map(&mut self, w: i32, mode: u32) {
        let Some(i) = self.rec(w) else { return };
        if self.xw[i].posok {
            // mapped again: where it was
            self.xw[i].reqx = self.xw[i].rx;
            self.xw[i].reqy = self.xw[i].ry;
        }
        if self.xw[i].reqx != PW_KEEP {
            self.update_hints(w);
            let (x, y) = (self.xw[i].reqx, self.xw[i].reqy);
            self.configure(w, 3, x, y, 0, 0);
            self.xw[i].reqpend = true;
        }
        let win = self.xw[i].win;
        if mode == PWS_SHOWNA {
            // _NET_WM_USER_TIME 0: no focus on map
            self.change_prop32(win, self.xw_atoms[B_USER_TIME], XA_CARDINAL, &[0]);
        }
        let (o, _) = self.req(XR_MAP_WINDOW, 8);
        wr32(&mut self.out, o, win);
    }

    /// x11_plat_win_activate(w): _NET_ACTIVE_WINDOW (source: application)
    pub(super) fn win_activate(&mut self, w: i32) {
        let Some(i) = self.rec(w) else { return };
        let r = self.xw[i];
        if !r.mapped || r.flags & PWF_NOACTIVATE != 0 || r.focus {
            return; // not shown / never focused / it has the focus already
        }
        let data = [1, self.time, self.focus_xid(), 0, 0];
        self.wm_message(w, self.xw_atoms[B_ACTIVE_WINDOW], &data);
        self.flush();
    }

    /// x11w_focus_xid -> X id of our window with the keyboard focus (0 none)
    fn focus_xid(&self) -> u32 {
        self.xw.iter().find(|r| r.focus).map_or(0, |r| r.win)
    }

    /// x11_plat_win_set_rect(w, x, y, cw, ch): client origin on the root (PW_KEEP) and client
    /// size (PW_KEEP); not reported back as E_RESIZE / E_WINPOS
    pub(super) fn win_set_rect(&mut self, w: i32, x: i32, y: i32, cw: i32, ch: i32) {
        let Some(i) = self.rec(w) else { return };
        let mut mask = 0;
        if x != PW_KEEP && y != PW_KEEP {
            self.xw[i].reqx = x;
            self.xw[i].reqy = y;
            let r = self.xw[i];
            if !(r.posok && r.rx == x && r.ry == y) {
                mask |= 3;
                self.xw[i].reqpend = true;
            }
        }
        if cw != PW_KEEP && cw != self.xw[i].w {
            self.xw[i].w = cw;
            mask |= 4;
        }
        if ch != PW_KEEP && ch != self.xw[i].h {
            self.xw[i].h = ch;
            mask |= 8;
        }
        if mask == 0 {
            return;
        }
        self.update_hints(w); // fixed-size windows: the hints first
        let (ww, hh) = (self.xw[i].w, self.xw[i].h);
        self.configure(w, mask, x, y, ww, hh);
        self.flush();
    }

    /// x11_plat_win_get_rect(w): client origin on the root and client size, None while the
    /// position is unknown
    pub(super) fn win_get_rect(&mut self, w: i32) -> Option<[i32; 4]> {
        let r = self.xw[self.rec(w)?];
        if r.posok { Some([r.rx, r.ry, r.w, r.h]) } else { None }
    }

    /// x11_plat_win_work_area(w): (U3) the monitor under the window's centre (RandR 1.5), else
    /// the whole root
    pub(super) fn win_work_area(&mut self, w: i32) -> Option<[i32; 4]> {
        if let Some(i) = self.rec(w) {
            let r = self.xw[i];
            if r.posok {
                let cx = (r.w as u32 >> 1) as i32 + r.rx;
                let cy = (r.h as u32 >> 1) as i32 + r.ry;
                if let Some(m) = self.monitor_rect(cx, cy) {
                    return Some(m);
                }
            }
        }
        Some([0, 0, self.scr_w as i32, self.scr_h as i32])
    }

    /// x11_plat_win_size(w) -> (width, height) in physical pixels
    pub(super) fn win_size(&self, w: i32) -> (i32, i32) {
        match self.rec(w) {
            Some(i) => (self.xw[i].w, self.xw[i].h),
            None => (0, 0),
        }
    }

    /// x11_plat_win_set_title(w, utf8): _NET_WM_NAME / WM_NAME (+ icon names), UTF8_STRING
    pub(super) fn win_set_title(&mut self, w: i32, title: &str) {
        let t = title.as_bytes();
        let t = &t[..clip_len(t, TITLE_MAX)];
        let Some(i) = self.rec(w) else { return };
        let win = self.xw[i].win;
        let utf8 = self.atoms[A_UTF8_STRING];
        for prop in [self.atoms[A_NET_WM_NAME], XA_WM_NAME, self.atoms[A_NET_WM_ICON_NAME], XA_WM_ICON_NAME] {
            self.change_property(win, prop, utf8, 8, 0, t);
        }
        self.flush();
    }

    /// x11_wm_message(w, type, data5): ClientMessage about window w to the root window
    /// (SubstructureRedirect | SubstructureNotify, as EWMH asks)
    fn wm_message(&mut self, w: i32, typ: u32, data: &[u32; 5]) {
        let Some(i) = self.rec(w) else { return };
        let mut ev = [0u8; 32];
        ev[0] = XE_CLIENT_MESSAGE;
        ev[1] = 32;
        wr32(&mut ev, 4, self.xw[i].win);
        wr32(&mut ev, 8, typ);
        for (k, v) in data.iter().enumerate() {
            wr32(&mut ev, 12 + 4 * k, *v);
        }
        let root = self.root;
        self.send_event(root, XM_SUBSTRUCTURE_NOTIFY | XM_SUBSTRUCTURE_REDIRECT, &ev, false);
    }

    /// x11_net_state(w, action, atom1, atom2): _NET_WM_STATE request to the window manager
    fn net_state(&mut self, w: i32, action: u32, a1: u32, a2: u32) {
        let t = self.atoms[A_NET_WM_STATE];
        self.wm_message(w, t, &[action, a1, a2, 1, 0]);
    }

    /// x11_plat_win_cmd(w, cmd): WCMD_MINIMIZE / MAXIMIZE / RESTORE / CLOSE; minimizing a
    /// window unmaps the windows it owns until it gets the focus again
    pub(super) fn win_cmd(&mut self, w: i32, cmd: u32) {
        let Some(i) = self.rec(w) else { return };
        match cmd {
            WCMD_CLOSE => {
                self.evwin = w;
                self.push(E_CLOSE, 0, 0, 0, 0);
            }
            WCMD_MINIMIZE => {
                self.wm_message(w, self.atoms[A_WM_CHANGE_STATE], &[X11_WM_ICONIC, 0, 0, 0, 0]);
                self.xw[i].state |= WST_MINIMIZED; // until activated again
                self.hide_owned(w, true);
                self.flush();
            }
            WCMD_MAXIMIZE => {
                let (v, h) = (self.atoms[A_NET_WM_STATE_MAX_VERT], self.atoms[A_NET_WM_STATE_MAX_HORZ]);
                self.net_state(w, NET_STATE_ADD, v, h);
                self.flush();
            }
            WCMD_RESTORE => {
                if self.xw[i].state & WST_FULLSCREEN != 0 {
                    let f = self.atoms[A_NET_WM_STATE_FULLSCREEN];
                    self.net_state(w, NET_STATE_REMOVE, f, 0);
                }
                let (v, h) = (self.atoms[A_NET_WM_STATE_MAX_VERT], self.atoms[A_NET_WM_STATE_MAX_HORZ]);
                self.net_state(w, NET_STATE_REMOVE, v, h);
                self.flush();
            }
            _ => {}
        }
    }

    /// x11w_hide_owned(w, hide): hide = true unmaps the shown windows owned by w (they come
    /// back with hide = false, when w gets the focus again)
    pub(super) fn hide_owned(&mut self, w: i32, hide: bool) {
        for j in 0..PW_MAX {
            let r = self.xw[j];
            if r.win == 0 || r.owner != w || j as i32 == w {
                continue;
            }
            if hide {
                if !r.shown {
                    continue;
                }
                self.xw[j].shown = false;
                self.xw[j].hidmin = true;
                let (o, _) = self.req(XR_UNMAP_WINDOW, 8);
                wr32(&mut self.out, o, r.win);
            } else if r.hidmin {
                self.xw[j].hidmin = false;
                self.xw[j].shown = true;
                self.map(j as i32, PWS_SHOWNA);
            }
        }
    }

    /// x11_plat_win_state(w) -> WST_* bits (WST_ACTIVE: one of the app's windows has the
    /// keyboard focus)
    pub(super) fn win_state(&self, w: i32) -> u32 {
        let Some(i) = self.rec(w) else { return 0 };
        let mut st = self.xw[i].state;
        if self.focus_xid() != 0 {
            st |= WST_ACTIVE;
        }
        st
    }

    /// x11_plat_win_begin_resize(w, edges): let the window manager resize from an edge
    pub(super) fn begin_resize(&mut self, w: i32, edges: u32) {
        let d = EDGE_DIR[(edges & 15) as usize];
        if d != 0xFF {
            self.moveresize(w, d as u32);
        }
    }

    /// x11_moveresize(w, direction): _NET_WM_MOVERESIZE with the pointer position of the last
    /// button press; only while a button is held (else the manager would start a move that
    /// waits for the next click)
    pub(super) fn moveresize(&mut self, w: i32, dir: u32) {
        if self.rec(w).is_none() || self.btn == 0 {
            return;
        }
        let (o, _) = self.req(XR_UNGRAB_POINTER, 8); // the implicit grab blocks the manager's
        let t = self.time;
        wr32(&mut self.out, o, t);
        let data = [self.press_rx as u32, self.press_ry as u32, dir, self.press_btn, 1];
        self.wm_message(w, self.atoms[A_NET_WM_MOVERESIZE], &data);
        self.release_buttons();
        self.flush();
    }

    // ------------------------------------------------------------------ properties

    /// x11_read_dpi -> jfgui_SCALE * 96, else Xft.dpi, else 96
    fn read_dpi(&mut self) -> i32 {
        if self.scale_ovr != 0 {
            return self.scale_ovr * 96;
        }
        let root = self.root;
        let Some(r) = self.get_property(root, XA_RESOURCE_MANAGER, XA_STRING, 0, 65536, false) else { return 96 };
        if r[1] != 8 {
            return 96;
        }
        let end = (32 + rd32(&r, 16) as usize).min(r.len());
        let mut p = 32;
        loop {
            if p + 8 > end {
                return 96;
            }
            if &r[p..p + 8] == b"Xft.dpi:" {
                break;
            }
            loop {
                if p >= end {
                    return 96;
                }
                let c = r[p];
                p += 1;
                if c == b'\n' {
                    break;
                }
            }
        }
        p += 8;
        while p < end && (r[p] == b' ' || r[p] == 9) {
            p += 1;
        }
        let mut n = 0;
        while n < 15 && p + n < end && r[p + n].is_ascii_digit() {
            n += 1;
        }
        if n == 0 {
            return 96;
        }
        let v = parse_int(&r[p..p + n]).0;
        if (48..=960).contains(&v) { v } else { 96 }
    }

    /// x11_read_state(w): WST_MAXIMIZED / FULLSCREEN / TILED / MINIMIZED of window w from
    /// _NET_WM_STATE and WM_STATE (focus is tracked separately)
    fn read_state(&mut self, w: i32) {
        let Some(i) = self.rec(w) else { return };
        let win = self.xw[i].win;
        let mut st = 0u32;
        let mut vh = 0u32; // bit 0 vert, bit 1 horz
        if let Some(r) = self.get_property(win, self.atoms[A_NET_WM_STATE], XA_ATOM, 0, 64, false) {
            if r[1] == 32 {
                let n = rd32(&r, 16) as usize;
                for k in 0..n {
                    let a = rd32(&r, 32 + 4 * k);
                    if a == self.atoms[A_NET_WM_STATE_MAX_VERT] {
                        vh |= 1;
                    }
                    if a == self.atoms[A_NET_WM_STATE_MAX_HORZ] {
                        vh |= 2;
                    }
                    if a == self.atoms[A_NET_WM_STATE_FULLSCREEN] {
                        st |= WST_FULLSCREEN;
                    }
                    if a == self.atoms[A_NET_WM_STATE_HIDDEN] {
                        st |= WST_MINIMIZED;
                    }
                }
            }
        }
        if vh == 3 {
            st |= WST_MAXIMIZED;
        } else if vh != 0 {
            st |= WST_TILED;
        }
        let ws = self.atoms[A_WM_STATE];
        if let Some(r) = self.get_property(win, ws, ws, 0, 2, false) {
            if r[1] == 32 && rd32(&r, 16) >= 1 && rd32(&r, 32) == X11_WM_ICONIC {
                st |= WST_MINIMIZED;
            }
        }
        self.xw[i].state = st;
        if self.debug {
            eprint(&format!("x11: window state {st:x}\n"));
        }
    }

    /// x11w_read_frame(w): _NET_FRAME_EXTENTS (left, right, top, bottom) of a window with the
    /// manager's frame: positions are for the client, the manager places the frame
    fn read_frame(&mut self, w: i32) {
        let Some(i) = self.rec(w) else { return };
        let win = self.xw[i].win;
        if let Some(r) = self.get_property(win, self.xw_atoms[B_FRAME_EXTENTS], XA_CARDINAL, 0, 4, false) {
            if r[1] == 32 && rd32(&r, 16) >= 4 {
                self.xw[i].framel = rd32(&r, 32) as i32;
                self.xw[i].framet = rd32(&r, 40) as i32;
            }
        }
    }

    /// x11w_translate(w): where is the window's client on the root (TranslateCoordinates);
    /// only while it is mapped (the manager parks unmapped windows far outside)
    fn translate(&mut self, w: i32) {
        let Some(i) = self.rec(w) else { return };
        if !self.xw[i].mapped {
            return;
        }
        let (o, seq) = self.req(XR_TRANSLATE_COORDS, 16);
        let (win, root) = (self.xw[i].win, self.root);
        wr32(&mut self.out, o, win);
        wr32(&mut self.out, o + 4, root);
        let Some(r) = self.reply(seq) else { return };
        let (x, y) = (rds16(&r, 12), rds16(&r, 14));
        if x < -30000 || y < -30000 {
            return;
        }
        self.pos_update(w, x, y);
    }

    /// x11w_pos_update(w, root_x, root_y): the client origin of window w is here -- E_WINPOS
    /// when it changed (not for a move we asked for)
    fn pos_update(&mut self, w: i32, x: i32, y: i32) {
        let Some(i) = self.rec(w) else { return };
        let r = &mut self.xw[i];
        if r.posok && r.rx == x && r.ry == y {
            return;
        }
        r.rx = x;
        r.ry = y;
        r.posok = true;
        if r.reqpend {
            r.reqpend = false;
            if r.reqx == x && r.reqy == y {
                return; // where we asked it to go
            }
        }
        let ev = self.evwin;
        self.evwin = w;
        self.push(E_WINPOS, 0, x, y, 0);
        self.evwin = ev;
    }

    /// x11_dpi_check: re-read the DPI; on a change E_DPI for every window, which keep their
    /// DIP size
    fn dpi_check(&mut self) {
        let new = self.read_dpi();
        let old = self.dpi;
        if new == old {
            return;
        }
        self.dpi = new;
        let ev = self.evwin;
        for j in 0..PW_MAX {
            if self.xw[j].win == 0 {
                continue;
            }
            self.evwin = j as i32;
            self.push(E_DPI, 0, 0, 0, new as u32);
            let r = self.xw[j];
            if r.state & STATE_BIG == 0 {
                let nw = (r.w as u32).wrapping_mul(new as u32) / old as u32;
                let nh = (r.h as u32).wrapping_mul(new as u32) / old as u32;
                self.configure(j as i32, 12, 0, 0, nw as i32, nh as i32);
            }
            self.update_hints(j as i32);
        }
        self.evwin = ev;
        self.flush();
    }

    /// x11_deferred: work the event handlers asked for (needs round trips)
    fn deferred(&mut self) {
        self.dnd_deferred(); // XDND: drops, positions, the app's drag (linux_x11_dnd.rs)
        self.xi_deferred(); // (PN1) pen devices changed
        self.screen_deferred(); // the pick's pixel, SET_MONITORS (linux_x11_screen.rs)
        if self.want & WANT_DPI != 0 {
            self.want &= !WANT_DPI;
            self.dpi_check();
        }
        if self.want & WANT_KEYMAP != 0 {
            self.want &= !WANT_KEYMAP;
            self.kb_load();
        }
        if self.want & WANT_WIN != 0 {
            self.want &= !WANT_WIN;
            for j in 0..PW_MAX {
                let bits = self.xw[j].want;
                self.xw[j].want = 0;
                if self.xw[j].win == 0 {
                    continue;
                }
                if bits & XWW_FRAME != 0 {
                    self.read_frame(j as i32);
                }
                if bits & XWW_STATE != 0 {
                    self.read_state(j as i32);
                }
                if bits & XWW_POS != 0 {
                    self.translate(j as i32);
                }
            }
        }
    }

    /// x11w_want(w, bits): ask wait() for work on window w
    fn want_win(&mut self, w: i32, bits: u32) {
        let i = w as u32 as usize;
        if i < PW_MAX {
            self.xw[i].want |= bits;
            self.want |= WANT_WIN;
        }
    }

    // ------------------------------------------------------------------ events

    /// x11_event(packet): handle one event (dispatch, the self tests)
    pub(super) fn event(&mut self, b: &mut [u8]) {
        if self.screen_event(b) {
            return; // shortcuts, the pick, RandR (linux_x11_screen.rs)
        }
        let typ = b[0] & 0x7F;
        // the window it is about -> evwin (0: the main window / not ours)
        let xid = match typ {
            XE_KEY_PRESS..=XE_LEAVE => rd32(b, 12), // key / button / motion / crossing: event
            XE_UNMAP | XE_MAP | XE_CONFIGURE => rd32(b, 8),
            XE_FOCUS_IN | XE_FOCUS_OUT | XE_EXPOSE | XE_PROPERTY | XE_CLIENT_MESSAGE => rd32(b, 4),
            _ => 0,
        };
        let mut idx = self.find(xid);
        if idx < 0 {
            // (U3) a popup window: pointer events become the owner's
            let r = self.popup_route(b);
            if r == -2 {
                return;
            }
            idx = r;
        }
        self.evwin = idx.max(0);
        match typ {
            XE_KEY_PRESS => self.key(b, true),
            XE_KEY_RELEASE => self.key(b, false),
            XE_BUTTON_PRESS => self.button(b, true),
            XE_BUTTON_RELEASE => self.button(b, false),
            XE_MOTION => self.motion(b),
            XE_ENTER => self.enter(b),
            XE_LEAVE => self.leave(b),
            XE_FOCUS_IN if idx >= 0 => self.focus(b, true),
            XE_FOCUS_OUT if idx >= 0 => self.focus(b, false),
            XE_EXPOSE => self.expose(b),
            XE_UNMAP if idx >= 0 => {
                if let Some(i) = self.rec(idx) {
                    self.xw[i].mapped = false;
                }
            }
            XE_MAP if idx >= 0 => {
                if let Some(i) = self.rec(idx) {
                    self.xw[i].mapped = true;
                    let mut bits = XWW_POS; // where the manager put it
                    if self.xw[i].flags & PWF_SYSFRAME != 0 {
                        bits |= XWW_FRAME; // the manager's frame is known now
                    }
                    self.want_win(idx, bits);
                    self.present_mapped(i); // shown again later (WSLg)
                }
            }
            XE_CONFIGURE if idx >= 0 => {
                if let Some(i) = self.rec(idx) {
                    // position: asked with TranslateCoordinates (a real event has frame-relative
                    // coordinates; Weston's XWM also sends synthetic ones that are frame-relative
                    // before the right root coordinates, measured on WSLg)
                    self.want_win(idx, XWW_POS);
                    let (w, h) = (rd16(b, 20) as i32, rd16(b, 22) as i32);
                    if w != self.xw[i].w || h != self.xw[i].h {
                        self.xw[i].w = w;
                        self.xw[i].h = h;
                        self.push(E_RESIZE, 0, w, h, 0);
                    }
                }
            }
            XE_PROPERTY => {
                self.time = rd32(b, 12);
                let (win, atom) = (rd32(b, 4), rd32(b, 8));
                if idx >= 0 {
                    if atom == self.atoms[A_NET_WM_STATE] || atom == self.atoms[A_WM_STATE] {
                        self.want_win(idx, XWW_STATE);
                    } else if atom == self.xw_atoms[B_FRAME_EXTENTS] {
                        self.want_win(idx, XWW_FRAME);
                    } else {
                        self.clip_property(b);
                    }
                } else if win == self.root {
                    if atom == XA_RESOURCE_MANAGER {
                        self.want |= WANT_DPI;
                    }
                } else {
                    self.clip_property(b); // an INCR transfer to another client
                }
            }
            XE_SELECTION_CLEAR => self.clip_clear(b),
            XE_SELECTION_REQUEST => self.clip_request(b),
            XE_SELECTION_NOTIFY => self.clip_notify(b),
            XE_CLIENT_MESSAGE if idx >= 0 => {
                if self.dnd_client(b, idx) {
                    return; // K1: XDND messages
                }
                if rd32(b, 8) != self.atoms[A_WM_PROTOCOLS] {
                    return;
                }
                let p = rd32(b, 12);
                if p == self.atoms[A_WM_DELETE_WINDOW] {
                    self.push(E_CLOSE, 0, 0, 0, 0);
                } else if p == self.atoms[A_NET_WM_PING] {
                    // pong: the same message back to the root window
                    let mut ev = [0u8; 32];
                    ev.copy_from_slice(&b[..32]);
                    ev[0] &= 0x7F;
                    let root = self.root;
                    wr32(&mut ev, 4, root);
                    self.send_event(root, XM_SUBSTRUCTURE_NOTIFY | XM_SUBSTRUCTURE_REDIRECT, &ev, false);
                }
            }
            XE_MAPPING => {
                if b[4] != 2 {
                    self.want |= WANT_KEYMAP; // (pointer mapping: not used)
                }
            }
            t if t > XE_MAPPING => {
                if t == XE_GENERIC {
                    // (PN1) XInput 2 events
                    if b[1] != 0 && b[1] as u32 == self.xi_op {
                        self.xi_event(b);
                    }
                } else if t as u32 == self.shm_ev && self.shm_op != 0 {
                    self.shm_done(b);
                } else if t as u32 == self.xkb_ev && self.xkb_op != 0 && b[1] <= 1 {
                    // XkbNewKeyboardNotify 0, XkbMapNotify 1
                    self.want |= WANT_KEYMAP;
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ event loop

    /// x11_plat_wait(deadline_us) -> number of queued events
    pub(super) fn wait(&mut self, deadline: u64) -> u32 {
        if self.xw[0].win == 0 {
            return crate::evq_count();
        }
        let mut polled = false;
        loop {
            self.dispatch();
            self.deferred();
            let now = crate::time_us();
            let mut next = self.present_service(now).min(self.dnd_next()); // deferred presents, a drop that waits
            self.flush();
            let n = crate::evq_count();
            if n != 0 {
                return n;
            }
            if self.want != 0 {
                continue;
            }
            let ms: i32;
            if deadline == 0 || (deadline != u64::MAX && now >= deadline) {
                // no waiting, but read what the server sent once per call
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
            let events = if self.out.is_empty() { POLLIN } else { POLLIN | POLLOUT };
            let rev = wake_poll(self.fd, events, ms);
            if rev & POLLIN != 0 {
                self.recv();
            } else if rev & (POLLERR | POLLHUP | POLLNVAL) != 0 {
                // error without data: poll reports it
                if self.poll(POLLIN, 0) & POLLIN != 0 {
                    self.recv();
                }
            }
        }
    }
}
