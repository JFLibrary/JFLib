// linux_x11 -- the X11 display backend: our own X11 protocol client over a Unix socket (no
// Xlib / xcb) behind linux_disp::Display
//                     [src/plat/linux/x11.inc, x11_wire.s, x11_window.s, x11_present.s,
//                      x11_input.s, x11_xi.s, x11_clip.s, x11_dnd.s, x11_popup.s,
//                      x11_cursor.s, x11_monitor.s; docs/modules/WINDOWS_PLATFORM.md Ã‚Â§4, Ã‚Â§9, Ã‚Â§10]
//
//   x11_connect() -> backend        $DISPLAY reachable: the backend (the main window is made
//                                   by its win_create), None (no message) otherwise
//   x11t::*                         the self-test entry points of linuxtest (x11synth,
//                                   x11err): events without a server, keysyms, the core
//                                   keymap, raw requests, the last X error
//
// The state of the assembly's globals lives in one X11 record (module state, one UI thread):
// the backend object is empty and every Display call borrows the record for its duration.
// Nothing inside calls back into the plat API that would borrow it again (the refresh
// callback is only stored, as on X11 in the assembly), so a re-entrant borrow would be a
// bug and panics.  Files:
//   linux_x11_wire.rs     connection, authentication, setup, requests, replies, errors
//   linux_x11_window.rs   window records, hints, WM messages, DPI, state, events, wait
//   linux_x11_present.rs  MIT-SHM (memfd / SysV) or PutImage, per window
//   linux_x11_input.rs    core keyboard / pointer input, keysyms, the core keymap (built into
//                         the key table of linux_xkb.rs: xkb_begin / xkb_set_key / xkb_key)
//   linux_x11_xi.rs       XInput 2 pens
//   linux_x11_clip.rs     CLIPBOARD (INCR both ways, images)
//   linux_x11_dnd.rs      XDND 5: drops of every kind into the windows, the app's drags out
//   linux_x11_misc.rs     popups (override-redirect), cursors (font / RENDER), monitors (RandR)
//   linux_x11_screen.rs   the monitor list, global shortcuts (GrabKey), GetImage of the root,
//                         the colour pick (pointer grab), RandR change events
//
// Deviation from the assembly (the Rust API hands the frame over only for the call): the
// MIT-SHM segments are whole frames brought up to date from the caller's frame (what each
// lacks, linux_x11_present.rs), so repaints (Expose, after a map) read the segment of the
// last put; the PutImage fallback keeps a shadow image of the frame, and popups keep a copy
// of their pixels -- where the assembly re-read the caller's framebuffer.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::linux_damage::Reg;
use crate::{CUR_COUNT, CUR_USER_MAX, Display, Frame, PW_MAX, PPW_MAX};

#[path = "linux_x11_clip.rs"]
mod clip;
#[path = "linux_x11_dnd.rs"]
mod dnd;
#[path = "linux_x11_input.rs"]
mod input;
#[path = "linux_x11_misc.rs"]
mod misc;
#[path = "linux_x11_present.rs"]
mod present;
#[path = "linux_x11_screen.rs"]
mod screen;
#[path = "linux_x11_window.rs"]
mod window;
#[path = "linux_x11_wire.rs"]
mod wire;
#[path = "linux_x11_xi.rs"]
mod xi;

// ------------------------------------------------------------------ constants  [x11.inc]

const X11_OUT_CAP: usize = 1048576; // request buffer
const X11_IN_CAP: usize = 4194304; // receive buffer (largest reply we ask for: 1 MB)
const X11_PROP_CHUNK: u32 = 262144; // longs per GetProperty (1 MB of data)

// requests (major opcodes)
const XR_CREATE_WINDOW: u32 = 1;
const XR_CHANGE_WINDOW_ATTR: u32 = 2;
const XR_DESTROY_WINDOW: u32 = 4;
const XR_MAP_WINDOW: u32 = 8;
const XR_UNMAP_WINDOW: u32 = 10;
const XR_CONFIGURE_WINDOW: u32 = 12;
const XR_INTERN_ATOM: u32 = 16;
const XR_CHANGE_PROPERTY: u32 = 18;
const XR_DELETE_PROPERTY: u32 = 19;
const XR_GET_PROPERTY: u32 = 20;
const XR_SET_SELECTION_OWNER: u32 = 22;
const XR_GET_SELECTION_OWNER: u32 = 23;
const XR_CONVERT_SELECTION: u32 = 24;
const XR_SEND_EVENT: u32 = 25;
const XR_UNGRAB_POINTER: u32 = 27;
const XR_TRANSLATE_COORDS: u32 = 40;
const XR_GET_INPUT_FOCUS: u32 = 43;
const XR_OPEN_FONT: u32 = 45;
const XR_CREATE_PIXMAP: u32 = 53;
const XR_CREATE_GC: u32 = 55;
const XR_POLY_FILL_RECTANGLE: u32 = 70;
const XR_PUT_IMAGE: u32 = 72;
const XR_CREATE_COLORMAP: u32 = 78;
const XR_CREATE_CURSOR: u32 = 93;
const XR_CREATE_GLYPH_CURSOR: u32 = 94;
const XR_QUERY_EXTENSION: u32 = 98;
const XR_GET_KEYBOARD_MAPPING: u32 = 101;
const XR_GET_MODIFIER_MAPPING: u32 = 119;
// extension minor opcodes
const XSHM_QUERY_VERSION: u32 = 0;
const XSHM_ATTACH: u32 = 1;
const XSHM_DETACH: u32 = 2;
const XSHM_PUT_IMAGE: u32 = 3;
const XSHM_ATTACH_FD: u32 = 6;
const XKB_SELECT_EVENTS: u32 = 1;
const XKB_PER_CLIENT_FLAGS: u32 = 21;

// events
const XE_ERROR: u8 = 0;
const XE_REPLY: u8 = 1;
const XE_KEY_PRESS: u8 = 2;
const XE_KEY_RELEASE: u8 = 3;
const XE_BUTTON_PRESS: u8 = 4;
const XE_BUTTON_RELEASE: u8 = 5;
const XE_MOTION: u8 = 6;
const XE_ENTER: u8 = 7;
const XE_LEAVE: u8 = 8;
const XE_FOCUS_IN: u8 = 9;
const XE_FOCUS_OUT: u8 = 10;
const XE_EXPOSE: u8 = 12;
const XE_UNMAP: u8 = 18;
const XE_MAP: u8 = 19;
const XE_CONFIGURE: u8 = 22;
const XE_PROPERTY: u8 = 28;
const XE_SELECTION_CLEAR: u8 = 29;
const XE_SELECTION_REQUEST: u8 = 30;
const XE_SELECTION_NOTIFY: u8 = 31;
const XE_CLIENT_MESSAGE: u8 = 33;
const XE_MAPPING: u8 = 34;
const XE_GENERIC: u8 = 35;
// input event fields (Key / Button / Motion / Enter / Leave)
const XEV_DETAIL: usize = 1;
const XEV_TIME: usize = 4;
const XEV_ROOT_X: usize = 20;
const XEV_ROOT_Y: usize = 22;
const XEV_X: usize = 24;
const XEV_Y: usize = 26;
const XEV_STATE: usize = 28;
const XEV_MODE: usize = 30; // Enter / Leave

// event masks
const XM_KEY_PRESS: u32 = 0x000001;
const XM_KEY_RELEASE: u32 = 0x000002;
const XM_BUTTON_PRESS: u32 = 0x000004;
const XM_BUTTON_RELEASE: u32 = 0x000008;
const XM_ENTER: u32 = 0x000010;
const XM_LEAVE: u32 = 0x000020;
const XM_POINTER_MOTION: u32 = 0x000040;
const XM_EXPOSURE: u32 = 0x008000;
const XM_STRUCTURE: u32 = 0x020000;
const XM_SUBSTRUCTURE_NOTIFY: u32 = 0x080000;
const XM_SUBSTRUCTURE_REDIRECT: u32 = 0x100000;
const XM_FOCUS: u32 = 0x200000;
const XM_PROPERTY: u32 = 0x400000;
const XM_WINDOW: u32 = XM_KEY_PRESS
    | XM_KEY_RELEASE
    | XM_BUTTON_PRESS
    | XM_BUTTON_RELEASE
    | XM_ENTER
    | XM_LEAVE
    | XM_POINTER_MOTION
    | XM_EXPOSURE
    | XM_STRUCTURE
    | XM_FOCUS
    | XM_PROPERTY;
// window attribute value-mask bits
const XCW_BACK_PIXMAP: u32 = 0x0001;
const XCW_BORDER_PIXEL: u32 = 0x0008;
const XCW_BIT_GRAVITY: u32 = 0x0010;
const XCW_OVERRIDE: u32 = 0x0200;
const XCW_EVENT_MASK: u32 = 0x0800;
const XCW_COLORMAP: u32 = 0x2000;
const XCW_CURSOR: u32 = 0x4000;

// predefined atoms
const XA_ATOM: u32 = 4;
const XA_CARDINAL: u32 = 6;
const XA_INTEGER: u32 = 19;
const XA_RESOURCE_MANAGER: u32 = 23;
const XA_STRING: u32 = 31;
const XA_WINDOW: u32 = 33;
const XA_WM_HINTS: u32 = 35;
const XA_WM_CLIENT_MACHINE: u32 = 36;
const XA_WM_ICON_NAME: u32 = 37;
const XA_WM_NAME: u32 = 39;
const XA_WM_NORMAL_HINTS: u32 = 40;
const XA_WM_SIZE_HINTS: u32 = 41;
const XA_WM_CLASS: u32 = 67;
const XA_WM_TRANSIENT_FOR: u32 = 68;

// interned atoms: indices into X11::atoms, same order as the names in x11_wire.s
const A_WM_PROTOCOLS: usize = 0;
const A_WM_DELETE_WINDOW: usize = 1;
const A_WM_STATE: usize = 2;
const A_WM_CHANGE_STATE: usize = 3;
const A_NET_WM_PING: usize = 4;
const A_NET_WM_PID: usize = 5;
const A_NET_WM_NAME: usize = 6;
const A_UTF8_STRING: usize = 7;
const A_NET_WM_STATE: usize = 8;
const A_NET_WM_STATE_MAX_VERT: usize = 9;
const A_NET_WM_STATE_MAX_HORZ: usize = 10;
const A_NET_WM_STATE_FULLSCREEN: usize = 11;
const A_NET_WM_STATE_HIDDEN: usize = 12;
const A_NET_WM_MOVERESIZE: usize = 13;
const A_NET_WM_WINDOW_TYPE: usize = 14;
const A_NET_WM_WINDOW_TYPE_NORMAL: usize = 15;
const A_MOTIF_WM_HINTS: usize = 16;
const A_CLIPBOARD: usize = 17;
const A_TARGETS: usize = 18;
const A_TIMESTAMP: usize = 20;
const A_INCR: usize = 21;
const A_TEXT: usize = 22;
const A_TEXT_PLAIN_UTF8: usize = 23;
const A_TEXT_PLAIN: usize = 24;
const A_IMAGE_PNG: usize = 25;
const A_JFGUI_SEL: usize = 26;
const A_NET_WM_ICON_NAME: usize = 27;
const A_COUNT: usize = 28;

// window records
const XWF_USED: u32 = 0x100;
const XWF_TOP: u32 = 0x200; // top-level window (no owner)
const XWW_STATE: u32 = 1; // re-read _NET_WM_STATE / WM_STATE
const XWW_POS: u32 = 2; // TranslateCoordinates: where is it on the root
const XWW_FRAME: u32 = 4; // re-read _NET_FRAME_EXTENTS
const X11_WM_ICONIC: u32 = 3; // WM_STATE / WM_CHANGE_STATE IconicState
const X11_MOVERESIZE_MOVE: u32 = 8;

// syscalls
const SYS_READ: usize = 0;
const SYS_WRITE: usize = 1;
const SYS_CLOSE: usize = 3;
const SYS_POLL: usize = 7;
const SYS_MMAP: usize = 9;
const SYS_MUNMAP: usize = 11;
const SYS_SHMGET: usize = 29;
const SYS_SHMAT: usize = 30;
const SYS_SHMCTL: usize = 31;
const SYS_GETPID: usize = 39;
const SYS_SOCKET: usize = 41;
const SYS_CONNECT: usize = 42;
const SYS_RECVFROM: usize = 45;
const SYS_SENDMSG: usize = 46;
const SYS_UNAME: usize = 63;
const SYS_SHMDT: usize = 67;
const SYS_FTRUNCATE: usize = 77;
const SYS_MEMFD_CREATE: usize = 319;
const EINTR: isize = 4;
const EAGAIN: isize = 11;
const POLLIN: u16 = 0x001;
const POLLOUT: u16 = 0x004;
const POLLERR: u16 = 0x008;
const POLLHUP: u16 = 0x010;
const POLLNVAL: u16 = 0x020;

/// one window (index 0 = the main window)  [x11.inc XW_*]
#[derive(Clone, Copy)]
struct Xw {
    win: u32,      // X window (0 = no window)
    w: i32,        // client size (px)
    h: i32,
    rx: i32,       // client origin on the root (px), valid with posok
    ry: i32,
    posok: bool,
    state: u32,    // WST_MAXIMIZED / FULLSCREEN / TILED / MINIMIZED
    mapped: bool,
    flags: u32,    // PWF_* | XWF_*
    owner: i32,    // owner index (-1 = top-level)
    minw: i32,     // minimum client size (DIP)
    minh: i32,
    focus: bool,   // has the keyboard focus
    want: u32,     // XWW_* work for wait
    shown: bool,   // should be mapped
    cursor: i32,   // CUR_* (-1: none set)
    reqx: i32,     // asked client origin (PW_KEEP = none): re-applied on a re-map
    reqy: i32,
    reqpend: bool, // a move we asked for has not been reported yet
    hidmin: bool,  // unmapped because its owner was minimized
    framel: i32,   // _NET_FRAME_EXTENTS left / top (system frame)
    framet: i32,
}

const XW0: Xw = Xw {
    win: 0,
    w: 0,
    h: 0,
    rx: 0,
    ry: 0,
    posok: false,
    state: 0,
    mapped: false,
    flags: 0,
    owner: 0,
    minw: 0,
    minh: 0,
    focus: false,
    want: 0,
    shown: false,
    cursor: 0,
    reqx: 0,
    reqy: 0,
    reqpend: false,
    hidmin: false,
    framel: 0,
    framet: 0,
};

/// one shared memory segment of a window  [x11_present.s SS_*]
#[derive(Clone, Copy)]
struct Seg {
    seg: u32,      // shmseg (0 = none)
    busy: bool,    // waiting for ShmCompletion
    shmid: i32,    // SysV id (-1 = memfd mapping)
    cap: usize,    // bytes
    mem: *mut u8,  // our mapping
    t0: u64,       // time of the last put
    cw: i32,       // the size of the frame it holds (rows of cw * 4 bytes; 0: none)
    ch: i32,
}

const SEG0: Seg = Seg { seg: 0, busy: false, shmid: 0, cap: 0, mem: core::ptr::null_mut(), t0: 0, cw: 0, ch: 0 };
const PR_MAX: usize = 16; // pending rectangles

/// the presentation state of one window  [x11_present.s XP_*]
struct Xp {
    sh: Vec<u8>,          // MIT-SHM: the staging image (damage waiting for a free segment);
                          // PutImage: the shadow image of the presented frame (sw * 4 bytes a row)
    stg: Reg,             // where the staging image holds frame pixels no segment has yet
    sw: i32,              // the frame size (sfok: a frame was presented)
    shh: i32,
    sfok: bool,
    pend: bool,           // damage waits
    over: bool,           // more than PR_MAX rectangles: only the box
    bx: [i32; 4],         // bounding box (x0, y0, x1, y1)
    pr: Vec<[i32; 4]>,    // pending boxes
    slots: [Seg; 2],
    st: [Reg; 2],         // per segment: where it differs from the latest frame
    newest: usize,        // the segment of the last put (2: none)
    rpt: u64,             // time of the next full repaint (0 none)
    rptn: i32,            // repaints left
}

/// an INCR transfer to another client  [x11_clip.s XF_*]
struct Xfer {
    req: u32, // requestor window (0 = free)
    prop: u32,
    typ: u32,
    data: Vec<u8>,
    off: usize,
}

/// a pen device  [x11_xi.s XD_*]
#[derive(Clone, Copy, Default)]
struct Xd {
    id: u32,
    typ: u32,  // PTR_PEN / PTR_ERASER
    pnum: i32, // pressure valuator number
    pmin: i32, // valuator range
    pmax: i32,
    txn: i32,  // tilt x / y valuator numbers (-1 = none)
    tyn: i32,
    lastp: u32, // last pressure (0..PRESSURE_ONE)
    tx: i32,    // last tilt (degrees)
    ty: i32,
}

/// a popup window  [x11_popup.s XQ_*]
struct Xq {
    win: u32,     // X window (0 = not created)
    owner: i32,   // owner platform window (-1 = hidden)
    dx: i32,      // popup pixel 0, 0 = owner pixel dx, dy
    dy: i32,
    w: i32,
    h: i32,
    mapped: bool,
    px: Vec<u8>,  // the pixels shown (rows of pw * 4 bytes)
    pw: i32,
    ph: i32,
}

/// the X11 backend state (the globals of x11_*.s)
struct X11 {
    // ---- wire
    fd: i32,
    debug: bool,
    ready: bool,
    inb: Vec<u8>,
    in_len: usize,
    in_pos: usize,
    in_busy: u32,
    in_hold: bool,
    out: Vec<u8>,
    out_fd: i32,
    out_hasfd: bool,
    seq: u32,
    await_seq: u16,
    await_on: bool,
    await_state: u32,
    reply_pos: usize,
    watch_seq: u16,
    watch_on: bool,
    watch_err: u32,
    err_last: u32,
    err_seen: [u32; 8],
    auth: Vec<u8>,
    nodename: Vec<u8>,
    xid_base: u32,
    xid_mask: u32,
    xid_inc: u32,
    xid_last: u32,
    root: u32,
    visual: u32,
    need_cmap: bool,
    scr_w: u32,
    scr_h: u32,
    min_kc: u32,
    max_kc: u32,
    maxreq: u32,
    bigreq: bool,
    shm_op: u32,
    shm_ev: u32,
    shm_minor: u32,
    xkb_op: u32,
    xkb_ev: u32,
    atoms: [u32; A_COUNT],
    // ---- windows
    xw: [Xw; PW_MAX],
    // ---- input method (linux_ibus.rs): a text field has the focus, its caret on the root
    ime_on: bool,
    ime_win: u8,
    ime_rect: [i32; 3],
    refresh: Option<fn()>,
    xw_atoms: [u32; window::B_COUNT],
    gc: u32,
    cmap: u32,
    dpi: i32,
    scale_ovr: i32,
    time: u32,
    want: u32,
    evwin: i32,
    tooltype: u32,
    // ---- presentation
    xp: [Xp; PW_MAX],
    mode: i32,
    attaching: bool,
    // ---- input
    ks: [[u32; input::KS_COLS]; 256],
    held: [u32; 8],
    mods: u32,
    btn: u32,
    mx: i32,
    my: i32,
    press_rx: i32,
    press_ry: i32,
    press_btn: u32,
    inside: u32,
    btn_win: i32,
    leave_pending: i32,
    rep_merge: u32,
    detect_rep: bool,
    xkb_on: bool,
    group: u32,
    kb_ready: bool,
    role_alt: u32,
    role_num: u32,
    role_super: u32,
    role_l3: u32,
    // ---- XInput 2
    xi_dev: [Xd; xi::XD_MAX],
    xi_op: u32,
    xi_atoms: [u32; 3],
    xi_ndev: u32,
    xi_want: bool,
    xi_time: u32,
    xi_data: u32,
    xi_valid: bool,
    // ---- clipboard
    hs_time: u64,
    hs_full: u64,
    hs_owner: u32,
    hs_mask: u32,
    cb_wait: u64,
    cb_data: Option<Vec<u8>>,
    cb_bmp: Option<Vec<u8>>,
    // clipboard_platform: rich formats and corresponding MIME atoms.
    cb_rich: Vec<(u32, Vec<u8>)>,
    cb_rich_atoms: [u32; 5],
    rb: Vec<u8>,
    xfers: [Xfer; clip::XF_MAX],
    cb_fmt: u32,
    cb_own: bool,
    cb_time: u32,
    cb_got: bool,
    cb_prop: u32,
    cb_newval: bool,
    incr_limit: u32,
    incr_chunk: u32,
    cb_sel: u32,
    cv_sel: u32, // the selection sel_convert waits for
    // ---- XDND (linux_x11_dnd.rs): another program's drag over a window, the app's drag
    xd_atoms: [u32; dnd::D_COUNT],
    xd_bmp_atom: u32,
    xd_init: bool,
    xd: dnd::Target,
    xs: dnd::Source,
    // ---- popups, cursors, monitors
    xq: [Xq; PPW_MAX as usize],
    cursors: [u32; CUR_COUNT as usize],
    cur_font: u32,
    xcust: [u32; CUR_USER_MAX as usize],
    xr_state: u32,
    xr_op: u32,
    xr_fmt: u32,
    xm: [[i32; 4]; misc::XM_MAX],
    xm_t: u64,
    xm_n: i32,
    xm_op: u32,
    xm_primary: u32,
    xm_valid: bool,
    // ---- screen (linux_x11_screen.rs): RandR, work area, shortcuts, the pick, XTEST
    sc_rr_op: u32,
    sc_rr_ev: u32,
    sc_wa_atom: u32,
    sc_desk_atom: u32,
    sc_grabs: Vec<screen::Grab>,
    sc_key_down: u32,
    sc_pick: screen::Pick,
    sc_changed: bool,
    sc_xt_op: u32,
}

impl X11 {
    fn new() -> X11 {
        X11 {
            fd: -1,
            debug: false,
            ready: false,
            inb: Vec::new(),
            in_len: 0,
            in_pos: 0,
            in_busy: 0,
            in_hold: false,
            out: Vec::new(),
            out_fd: -1,
            out_hasfd: false,
            seq: 0,
            await_seq: 0,
            await_on: false,
            await_state: 0,
            reply_pos: 0,
            watch_seq: 0,
            watch_on: false,
            watch_err: 0,
            err_last: 0,
            err_seen: [0; 8],
            auth: Vec::new(),
            nodename: Vec::new(),
            xid_base: 0,
            xid_mask: 0,
            xid_inc: 0,
            xid_last: 0,
            root: 0,
            visual: 0,
            need_cmap: false,
            scr_w: 0,
            scr_h: 0,
            min_kc: 0,
            max_kc: 0,
            maxreq: 0,
            bigreq: false,
            shm_op: 0,
            shm_ev: 0,
            shm_minor: 0,
            xkb_op: 0,
            xkb_ev: 0,
            atoms: [0; A_COUNT],
            xw: [XW0; PW_MAX],
            ime_on: false,
            ime_win: 0,
            ime_rect: [0; 3],
            refresh: None,
            xw_atoms: [0; window::B_COUNT],
            gc: 0,
            cmap: 0,
            dpi: 0,
            scale_ovr: 0,
            time: 0,
            want: 0,
            evwin: 0,
            tooltype: 0,
            xp: core::array::from_fn(|_| Xp {
                sh: Vec::new(),
                stg: Reg::new(),
                sw: 0,
                shh: 0,
                sfok: false,
                pend: false,
                over: false,
                bx: [0; 4],
                pr: Vec::new(),
                slots: [SEG0; 2],
                st: [Reg::new(), Reg::new()],
                newest: 2,
                rpt: 0,
                rptn: 0,
            }),
            mode: 0,
            attaching: false,
            ks: [[0; input::KS_COLS]; 256],
            held: [0; 8],
            mods: 0,
            btn: 0,
            mx: 0,
            my: 0,
            press_rx: 0,
            press_ry: 0,
            press_btn: 0,
            inside: 0,
            btn_win: 0,
            leave_pending: 0,
            rep_merge: 0,
            detect_rep: false,
            xkb_on: false,
            group: 0,
            kb_ready: false,
            role_alt: 0,
            role_num: 0,
            role_super: 0,
            role_l3: 0,
            xi_dev: [Xd::default(); xi::XD_MAX],
            xi_op: 0,
            xi_atoms: [0; 3],
            xi_ndev: 0,
            xi_want: false,
            xi_time: 0,
            xi_data: 0,
            xi_valid: false,
            hs_time: 0,
            hs_full: 0,
            hs_owner: 0,
            hs_mask: 0,
            cb_wait: 0,
            cb_data: None,
            cb_bmp: None,
            cb_rich: Vec::new(),
            cb_rich_atoms: [0; 5],
            rb: Vec::new(),
            xfers: core::array::from_fn(|_| Xfer { req: 0, prop: 0, typ: 0, data: Vec::new(), off: 0 }),
            cb_fmt: 0,
            cb_own: false,
            cb_time: 0,
            cb_got: false,
            cb_prop: 0,
            cb_newval: false,
            incr_limit: 0,
            incr_chunk: 0,
            cb_sel: 0,
            cv_sel: 0,
            xd_atoms: [0; dnd::D_COUNT],
            xd_bmp_atom: 0,
            xd_init: false,
            xd: Default::default(),
            xs: Default::default(),
            xq: core::array::from_fn(|_| Xq {
                win: 0,
                owner: 0,
                dx: 0,
                dy: 0,
                w: 0,
                h: 0,
                mapped: false,
                px: Vec::new(),
                pw: 0,
                ph: 0,
            }),
            cursors: [0; CUR_COUNT as usize],
            cur_font: 0,
            xcust: [0; CUR_USER_MAX as usize],
            xr_state: 0,
            xr_op: 0,
            xr_fmt: 0,
            xm: [[0; 4]; misc::XM_MAX],
            xm_t: 0,
            xm_n: 0,
            xm_op: 0,
            xm_primary: 0,
            xm_valid: false,
            sc_rr_op: 0,
            sc_rr_ev: 0,
            sc_wa_atom: 0,
            sc_desk_atom: 0,
            sc_grabs: Vec::new(),
            sc_key_down: 0,
            sc_pick: screen::Pick::default(),
            sc_changed: false,
            sc_xt_op: 0,
        }
    }
}

// ------------------------------------------------------------------ the module state

struct Global(RefCell<Option<Box<X11>>>);
// SAFETY: the X11 state is used by the UI thread only (wake() and the pool never touch it);
// RefCell checks the (non-reentrant) borrows at run time.
unsafe impl Sync for Global {}
static XG: Global = Global(RefCell::new(None));

/// run f on the X11 state (created on first use: the synthetic tests need no connection)
fn with_x<R>(f: impl FnOnce(&mut X11) -> R) -> R {
    let mut g = XG.0.borrow_mut();
    let x = g.get_or_insert_with(|| Box::new(X11::new()));
    f(x)
}

// ------------------------------------------------------------------ helpers

fn rd16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn rds16(b: &[u8], o: usize) -> i32 {
    rd16(b, o) as i16 as i32
}

fn wr16(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 2].copy_from_slice(&(v as u16).to_le_bytes());
}

fn wr32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// "x11: msg" on stderr  (fmt_err)
fn eprint(s: &str) {
    crate::write_stderr(s.as_bytes());
}

/// str_copy(dst, s, cap): the length of s cut to at most cap - 1 bytes on a character
/// boundary (utf8_clip)
fn clip_len(s: &[u8], cap: usize) -> usize {
    if s.len() <= cap - 1 {
        return s.len();
    }
    let mut n = cap - 1;
    while n > 0 && s[n] & 0xC0 == 0x80 {
        n -= 1;
    }
    n
}

/// parse_int of core/str.s: blanks, a sign, decimal digits (wrapping) -> (value, bytes used)
fn parse_int(b: &[u8]) -> (i32, usize) {
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
    let mut v: i32 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        v = v.wrapping_mul(10).wrapping_add((b[i] - b'0') as i32);
        i += 1;
    }
    (if neg { v.wrapping_neg() } else { v }, i)
}

// ------------------------------------------------------------------ the backend object

struct Backend;

/// connect to $DISPLAY -> the backend, None when there is no X server  [x11_plat_win_create]
pub fn x11_connect() -> Option<Box<dyn Display>> {
    if with_x(|x| x.fd >= 0 || x.connect_main()) { Some(Box::new(Backend)) } else { None }
}

impl Display for Backend {
    fn win_create(&mut self, title: &str, w_dip: i32, h_dip: i32, flags: u32, owner: i32, x: i32, y: i32) -> i32 {
        with_x(|s| s.win_create(title, w_dip, h_dip, flags, owner, x, y))
    }
    fn win_destroy(&mut self, w: i32) {
        with_x(|s| s.win_destroy(w))
    }
    fn win_show(&mut self, w: i32, mode: u32) {
        with_x(|s| s.win_show(w, mode))
    }
    fn win_activate(&mut self, w: i32) {
        with_x(|s| s.win_activate(w))
    }
    fn win_enable(&mut self, _w: i32, _on: bool) {
        // nothing: the UI keeps windows below a modal one inert (the dialog is MODAL)
    }
    fn win_set_rect(&mut self, w: i32, x: i32, y: i32, cw: i32, ch: i32) {
        with_x(|s| s.win_set_rect(w, x, y, cw, ch))
    }
    fn win_get_rect(&mut self, w: i32) -> Option<[i32; 4]> {
        with_x(|s| s.win_get_rect(w))
    }
    fn win_work_area(&mut self, w: i32) -> Option<[i32; 4]> {
        with_x(|s| s.win_work_area(w))
    }
    fn win_set_title(&mut self, w: i32, title: &str) {
        with_x(|s| s.win_set_title(w, title))
    }
    fn win_state(&mut self, w: i32) -> u32 {
        with_x(|s| s.win_state(w))
    }
    fn win_cmd(&mut self, w: i32, cmd: u32) {
        with_x(|s| s.win_cmd(w, cmd))
    }
    fn win_begin_move(&mut self, w: i32) {
        with_x(|s| s.moveresize(w, X11_MOVERESIZE_MOVE))
    }
    fn win_begin_resize(&mut self, w: i32, edges: u32) {
        with_x(|s| s.begin_resize(w, edges))
    }
    fn win_size(&mut self, w: i32) -> (i32, i32) {
        with_x(|s| s.win_size(w))
    }
    fn win_dpi(&mut self, _w: i32) -> i32 {
        with_x(|s| if s.dpi != 0 { s.dpi } else { 96 })
    }
    fn win_set_min_size(&mut self, w: i32, w_dip: i32, h_dip: i32) {
        with_x(|s| s.win_set_min_size(w, w_dip, h_dip))
    }
    fn win_present(&mut self, w: i32, f: &Frame, rects: Option<&[[i32; 4]]>) {
        with_x(|s| s.win_present(w, f, rects))
    }
    fn win_set_cursor(&mut self, w: i32, shape: u32) {
        with_x(|s| s.win_set_cursor(w, shape))
    }
    fn set_refresh(&mut self, f: Option<fn()>) {
        // X11 delivers ConfigureNotify to the normal event loop: stored only
        with_x(|s| s.refresh = f)
    }
    fn wait(&mut self, deadline_us: u64) -> u32 {
        with_x(|s| s.wait(deadline_us))
    }
    fn popup(&mut self, slot: u32, owner: i32, rect: Option<[i32; 4]>, f: Option<&Frame>) {
        with_x(|s| s.popup(slot, owner, rect, f))
    }
    fn cursor_create(&mut self, px: &[u8], w: i32, h: i32, hot_x: i32, hot_y: i32, _dpi: i32) -> i32 {
        with_x(|s| s.cursor_create(px, w, h, hot_x, hot_y))
    }
    fn cursor_destroy(&mut self, id: i32) {
        with_x(|s| s.cursor_destroy(id))
    }
    fn cursor_dpi(&mut self, dpi: i32) -> i32 {
        dpi // X11: the window's
    }
    fn clip_set(&mut self, fmt: u32, data: &[u8]) -> bool {
        with_x(|s| s.clip_set(fmt, data))
    }
    fn clip_set_many(&mut self, entries: &[(u32, &[u8])]) -> bool {
        with_x(|s| s.clip_set_many(entries))
    }
    fn clip_get(&mut self, fmt: u32) -> Option<Vec<u8>> {
        with_x(|s| s.clip_get(fmt))
    }
    fn clip_has(&mut self, fmt: u32) -> bool {
        with_x(|s| s.clip_has(fmt))
    }
    fn pen_devices(&mut self) -> i32 {
        with_x(|s| if s.xi_op == 0 { -1 } else { s.xi_ndev as i32 })
    }
    fn ime_pos(&mut self, x: i32, y: i32, h: i32) {
        with_x(|s| s.ime_pos(x, y, h))
    }
    fn drag_start(&mut self, data: crate::DragData) -> bool {
        with_x(|s| s.drag_start(data))
    }
}

// ------------------------------------------------------------------ self tests (linuxtest)

/// the X11 internals linuxtest --test x11synth / x11err drives (x11_event, x11_ks_value,
/// x11_kb_*, g_x11_xkb_on, g_x11_mods, x11_req, x11_sync, g_x11_err_last; the key table is
/// plat::xkb_*)
pub mod x11t {
    use super::with_x;

    /// one X event packet (32 bytes) through the event handler (x11_event)
    pub fn event(pkt: &[u8; 32]) {
        with_x(|s| {
            let mut p = *pkt;
            s.event(&mut p);
        })
    }

    /// one X event packet of any length (a GenericEvent: 32 bytes + its extra length)
    /// through the event handler (x11_event)
    pub fn event_bytes(pkt: &[u8]) {
        if pkt.len() < 32 {
            return;
        }
        with_x(|s| {
            let mut p = pkt.to_vec();
            s.event(&mut p);
        })
    }

    /// XInput 2 without a server: its major opcode and the valuator label atoms ("Abs
    /// Pressure", "Abs Tilt X", "Abs Tilt Y"); op 0 turns it off again (g_xi_op, g_xi_atoms)
    pub fn xi_setup(op: u32, atoms: [u32; 3]) {
        with_x(|s| {
            s.xi_op = op;
            s.xi_atoms = atoms;
        })
    }

    /// the pen devices of an XIQueryDevice reply -> pens found (x11_xi_parse_devices)
    pub fn xi_parse_devices(reply: &[u8]) -> u32 {
        if reply.len() < 32 {
            return 0;
        }
        with_x(|s| s.xi_parse_devices(reply))
    }

    /// pens found, -1 = no XInput 2 (x11_pen_devices)
    pub fn pen_devices() -> i32 {
        with_x(|s| if s.xi_op == 0 { -1 } else { s.xi_ndev as i32 })
    }

    /// a keysym as an xkb.s symbol value (x11_ks_value)
    pub fn ks_value(ks: u32) -> u32 {
        super::input::ks_value(ks)
    }

    /// the core keyboard mapping: `per` keysyms per keycode from `first` (x11_kb_set_keysyms)
    pub fn kb_set_keysyms(ks: &[u32], first: u32, count: u32, per: u32) {
        with_x(|s| s.kb_set_keysyms(ks, first, count, per))
    }

    /// the modifier mapping: 8 modifiers x per keycodes (x11_kb_set_modmap)
    pub fn kb_set_modmap(kc: &[u8], per: u32) {
        with_x(|s| s.kb_set_modmap(kc, per))
    }

    /// build the key records of a group (x11_kb_build)
    pub fn kb_build(group: u32) {
        with_x(|s| s.kb_build(group))
    }

    /// XKB on (event state bits 13-14 select the group)  (g_x11_xkb_on)
    pub fn set_xkb_on(on: bool) {
        with_x(|s| s.xkb_on = on)
    }

    /// the current MOD_* bits (g_x11_mods)
    pub fn mods() -> u32 {
        with_x(|s| s.mods)
    }

    /// a raw request: major opcode, body after the 4-byte header -> its sequence number
    /// (x11_req; nothing without a connection)
    pub fn req(major: u32, body: &[u8]) -> u32 {
        with_x(|s| {
            if s.fd < 0 {
                return 0;
            }
            let (o, seq) = s.req(major, 4 + body.len().next_multiple_of(4));
            s.out[o..o + body.len()].copy_from_slice(body);
            seq
        })
    }

    /// round trip: every earlier request has been processed (x11_sync)
    pub fn sync() {
        with_x(|s| {
            if s.fd >= 0 {
                s.sync();
            }
        })
    }

    /// the sequence number of the last X error (g_x11_err_last)
    pub fn err_last() -> u32 {
        with_x(|s| s.err_last)
    }

    /// XTEST FakeInput: 2 / 3 key press / release (detail: keycode), 4 / 5 button press /
    /// release, 6 motion to the root point x, y -> false without XTEST or a connection
    pub fn fake_input(typ: u32, detail: u32, x: i32, y: i32) -> bool {
        with_x(|s| s.fd >= 0 && s.fake_input(typ, detail, x, y))
    }

    /// the input focus: (window: 0 None, 1 PointerRoot; revert-to) (GetInputFocus)
    pub fn focus() -> (u32, u32) {
        with_x(|s| {
            if s.fd < 0 {
                return (0, 0);
            }
            let (_, seq) = s.req(super::XR_GET_INPUT_FOCUS, 4);
            s.reply(seq).map_or((0, 0), |r| (super::rd32(&r, 8), r[1] as u32))
        })
    }

    /// SetInputFocus(window, revert-to) at CurrentTime
    pub fn set_focus(win: u32, revert: u32) {
        with_x(|s| {
            if s.fd >= 0 {
                let (o, _) = s.req(42 | (revert & 0xFF) << 8, 12);
                super::wr32(&mut s.out, o, win);
                super::wr32(&mut s.out, o + 4, 0);
                s.sync();
            }
        })
    }

    /// the root window (0 without a connection)
    pub fn root() -> u32 {
        with_x(|s| if s.fd >= 0 { s.root } else { 0 })
    }

    /// the modifier masks of Alt, Num Lock and Super (their roles in the modifier mapping)
    pub fn roles() -> (u32, u32, u32) {
        with_x(|s| (s.role_alt, s.role_num, s.role_super))
    }

    /// the keycodes (X, evdev + 8) that give vk without modifiers
    pub fn vk_keycodes(vk: u32) -> alloc::vec::Vec<u32> {
        crate::xkb_vk_keys(vk).into_iter().map(|e| e + 8).collect()
    }

    /// GetImage of window w's rectangle (window px): the capture's path on an own window
    pub fn image_of(w: i32, r: [i32; 4]) -> Result<crate::screen::Shot, crate::screen::ScreenError> {
        with_x(|s| s.image_of(w, r))
    }

}

// ------------------------------------------------------------------ screen, shortcuts
/// (screen.rs, hotkey.rs through linux_screen.rs / linux_hotkey.rs) the X11 parts; only while
/// the main window exists (its connection)
pub(crate) mod x11scr {
    use super::with_x;
    use crate::hotkey::{Chord, HotkeyError};
    use crate::screen::{Monitor, ScreenError, Shot};
    use alloc::vec::Vec;

    /// connected, the main window made
    pub(crate) fn ready() -> bool {
        with_x(|s| s.fd >= 0 && s.xw[0].win != 0)
    }

    pub(crate) fn monitors() -> Vec<Monitor> {
        with_x(|s| if s.fd >= 0 { s.monitors() } else { Vec::new() })
    }

    pub(crate) fn hotkey_grab(id: u32, chord: Chord) -> Result<(), HotkeyError> {
        with_x(|s| s.hotkey_grab(id, chord))
    }

    pub(crate) fn hotkey_ungrab(id: u32) {
        with_x(|s| {
            if s.fd >= 0 {
                s.hotkey_ungrab(id)
            }
        })
    }

    pub(crate) fn capture(rect: Option<[i32; 4]>) -> Result<Shot, ScreenError> {
        with_x(|s| s.capture(rect))
    }

    pub(crate) fn pick_start(test: Option<(i32, i32)>) -> Result<(), ScreenError> {
        with_x(|s| s.pick_start(test))
    }
}

// ------------------------------------------------------------------ plat-shared
// (plat-shared) what the portal (the parent window "x11:<XID>", portal.s g_x11_win) and the
// monitor profile (lx_icc.s: the root window's _ICC_PROFILE) need from the connection

/// the X11 state other plat modules read (plat-shared)
pub mod x11s {
    use super::{X11_PROP_CHUNK, with_x};
    use alloc::vec::Vec;

    /// the main window's XID (0 = none)  (g_x11_win)
    pub fn main_xid() -> u32 {
        with_x(|s| s.xw[0].win)
    }

    /// the root window property `name` (AnyPropertyType, X11_PROP_CHUNK longs) -> the
    /// GetProperty reply (see get_property), None without a connection / on an error.  `atom`
    /// caches the atom between calls: 0 not asked yet (InternAtom only-if-exists is sent),
    /// u32::MAX the name does not exist  (lx_icc.s g_icc_atom)
    pub fn root_property(name: &[u8], atom: &mut u32) -> Option<Vec<u8>> {
        with_x(|s| {
            if s.fd < 0 {
                return None;
            }
            if *atom == 0 {
                *atom = u32::MAX;
                let seq = s.intern_send(name, true);
                let a = s.reply(seq).map_or(0, |r| super::rd32(&r, 8));
                if a == 0 {
                    return None;
                }
                *atom = a;
            }
            if *atom == u32::MAX {
                return None;
            }
            let root = s.root;
            s.get_property(root, *atom, 0, 0, X11_PROP_CHUNK, false)
        })
    }
}
