// consts -- the platform constants of jfgui.inc / ui.inc shared by every OS layer and the UI
//                                                        [ASMLIB/inc/jfgui.inc, ui.inc]

// ---- events (Event::typ)
pub const E_NONE: u8 = 0;
pub const E_MOVE: u8 = 1;
pub const E_DOWN: u8 = 2; // btn
pub const E_UP: u8 = 3; // btn
pub const E_WHEEL: u8 = 4; // data = (i16 dy) | (i16 dx << 16), 120 per notch, up / right positive
pub const E_KEY_DOWN: u8 = 5; // data = VK | (repeat << 31)
pub const E_KEY_UP: u8 = 6; // data = VK
pub const E_TEXT: u8 = 7; // data = Unicode codepoint
pub const E_RESIZE: u8 = 8; // x = width, y = height (physical pixels)
pub const E_DPI: u8 = 9; // data = dpi (96 = 100 %)
pub const E_FOCUS_IN: u8 = 10;
pub const E_FOCUS_OUT: u8 = 11;
pub const E_LEAVE: u8 = 12; // pointer left the window
pub const E_CLOSE: u8 = 13; // user asked to close the window
pub const E_WAKE: u8 = 14; // generic wake-up (timer, repaint request)
pub const E_PAINT: u8 = 15; // the OS needs the window content again
pub const E_WINPOS: u8 = 16; // window moved: x, y = client origin in screen px
pub const E_DROP: u8 = 17; // files dropped: x, y = position, data = count (drop_take)
pub const E_PREEDIT: u8 = 18; // ime_preedit(win), uncommitted text
pub const E_IME_COMMIT: u8 = 19; // data = bounded ime_commit_take token
pub const E_POWER: u8 = 20; // data = 1: the system goes to sleep, 0: it woke up
pub const E_SESSION_END: u8 = 21; // the user logs off, the system shuts down, SIGTERM: save and end
pub const E_SETTINGS: u8 = 22; // data = SET_* bits: system settings changed
pub const E_DRAG: u8 = 23; // a drag over the window: x, y, data = DT_* | DE_* << 8 (0: it left; dnd.rs)
pub const E_DRAG_END: u8 = 24; // our outgoing drag (drag_start) ended: data = DE_* effect, 0 = none

// E_SETTINGS: what changed
pub const SET_THEME: u32 = 1; // light / dark, accent colour
pub const SET_LOCALE: u32 = 2; // language, regional format
pub const SET_CONTRAST: u32 = 4; // high contrast, system colours
pub const SET_FONTS: u32 = 8; // fonts, text scaling
pub const SET_OTHER: u32 = 16;
pub const SET_MONITORS: u32 = 32; // monitors added, removed, moved, rescaled; work areas (screen::monitors)
pub const EVQ_CAP: usize = 256; // event ring capacity (power of two)

// ---- pointer types, buttons, modifiers
pub const PTR_MOUSE: u32 = 0;
pub const PTR_PEN: u32 = 1; // pen tip
pub const PTR_ERASER: u32 = 2; // eraser end of a pen
pub const PTR_TOUCH: u32 = 3;
pub const BTN_LEFT: u8 = 1;
pub const BTN_RIGHT: u8 = 2;
pub const BTN_MIDDLE: u8 = 3;
pub const BTN_X1: u8 = 4;
pub const BTN_X2: u8 = 5;
pub const MOD_SHIFT: u8 = 1;
pub const MOD_CTRL: u8 = 2;
pub const MOD_ALT: u8 = 4;
pub const MOD_SUPER: u8 = 8;
pub const MOD_ALTGR: u8 = 16;
pub const MOD_CAPS: u8 = 32;
pub const MOD_NUM: u8 = 64;

// ---- cursors (CUR_*: system shapes; CUR_USER0.. custom cursors of cursor_create)
pub const CUR_ARROW: u32 = 0;
pub const CUR_IBEAM: u32 = 1;
pub const CUR_HAND: u32 = 2;
pub const CUR_CROSS: u32 = 3;
pub const CUR_MOVE: u32 = 4;
pub const CUR_WAIT: u32 = 5;
pub const CUR_SIZE_NS: u32 = 6;
pub const CUR_SIZE_WE: u32 = 7;
pub const CUR_SIZE_NWSE: u32 = 8;
pub const CUR_SIZE_NESW: u32 = 9;
pub const CUR_NONE: u32 = 10;
pub const CUR_COUNT: u32 = 11;
pub const CUR_CUSTOM: u32 = 15; // hit boxes only: the cursor chosen with the UI's custom call
pub const CUR_USER0: u32 = 16;
pub const CUR_USER_MAX: u32 = 32;
pub const CUR_TABLE: u32 = CUR_USER0 + CUR_USER_MAX;
pub const CUR_MAX_PX: i32 = 256; // cursor_create: largest width / height

// ---- (E3) more system shapes: CSS's cursor names, past the custom cursors (CUR_X0 ..
// CUR_X_END); a platform without one shows cursor_fallback's
pub const CUR_X0: u32 = 64;
pub const CUR_NOT_ALLOWED: u32 = 64; // not-allowed
pub const CUR_NO_DROP: u32 = 65; // no-drop
pub const CUR_GRAB: u32 = 66; // grab: an open hand
pub const CUR_GRABBING: u32 = 67; // grabbing: a closed hand
pub const CUR_HELP: u32 = 68; // help: the arrow with a question mark
pub const CUR_PROGRESS: u32 = 69; // progress: the arrow with a busy sign (work in the background)
pub const CUR_ZOOM_IN: u32 = 70; // zoom-in
pub const CUR_ZOOM_OUT: u32 = 71; // zoom-out
pub const CUR_CELL: u32 = 72; // cell: a thick cross (tables)
pub const CUR_VTEXT: u32 = 73; // vertical-text
pub const CUR_ALIAS: u32 = 74; // alias: a link will be made
pub const CUR_COPY: u32 = 75; // copy: a copy will be made
pub const CUR_CONTEXT_MENU: u32 = 76; // context-menu
pub const CUR_ALL_SCROLL: u32 = 77; // all-scroll
pub const CUR_COL_RESIZE: u32 = 78; // col-resize
pub const CUR_ROW_RESIZE: u32 = 79; // row-resize
pub const CUR_SIZE_N: u32 = 80; // n-resize: one edge or corner
pub const CUR_SIZE_S: u32 = 81;
pub const CUR_SIZE_E: u32 = 82;
pub const CUR_SIZE_W: u32 = 83;
pub const CUR_SIZE_NE: u32 = 84;
pub const CUR_SIZE_NW: u32 = 85;
pub const CUR_SIZE_SE: u32 = 86;
pub const CUR_SIZE_SW: u32 = 87;
pub const CUR_X_END: u32 = 88;

/// (E3) the nearest of the first shapes (CUR_ARROW .. CUR_NONE) to a shape of CUR_X0 ..
/// CUR_X_END, for the platforms and cursor themes without it; other shapes unchanged
pub const fn cursor_fallback(shape: u32) -> u32 {
    match shape {
        CUR_GRAB => CUR_HAND,
        CUR_GRABBING | CUR_ALL_SCROLL => CUR_MOVE,
        CUR_PROGRESS => CUR_WAIT,
        CUR_ZOOM_IN | CUR_ZOOM_OUT | CUR_CELL => CUR_CROSS,
        CUR_VTEXT => CUR_IBEAM,
        CUR_COL_RESIZE | CUR_SIZE_E | CUR_SIZE_W => CUR_SIZE_WE,
        CUR_ROW_RESIZE | CUR_SIZE_N | CUR_SIZE_S => CUR_SIZE_NS,
        CUR_SIZE_NE | CUR_SIZE_SW => CUR_SIZE_NESW,
        CUR_SIZE_NW | CUR_SIZE_SE => CUR_SIZE_NWSE,
        CUR_X0..CUR_X_END => CUR_ARROW,
        _ => shape,
    }
}

// ---- window edges, commands, states
pub const EDGE_TOP: u32 = 1;
pub const EDGE_BOTTOM: u32 = 2;
pub const EDGE_LEFT: u32 = 4;
pub const EDGE_RIGHT: u32 = 8;
pub const WCMD_MINIMIZE: u32 = 1;
pub const WCMD_MAXIMIZE: u32 = 2;
pub const WCMD_RESTORE: u32 = 3;
pub const WCMD_CLOSE: u32 = 4;
pub const WST_MAXIMIZED: u32 = 1;
pub const WST_FULLSCREEN: u32 = 2;
pub const WST_ACTIVE: u32 = 4; // one of the APP's windows is active
pub const WST_TILED: u32 = 8;
pub const WST_MINIMIZED: u32 = 16;
pub const WST_CSD: u32 = 32; // no system frame: the app must draw its own title bar
/// the window is drawn inside its owner (a Wayland subsurface): win_get_rect / win_set_rect /
/// E_WINPOS use positions relative to the owner's surface origin (px), the owner carries it
pub const WST_CHILD: u32 = 64;
/// with WST_TILED: which edges touch the tiling (EDGE_* << WST_TILED_SHIFT; Wayland's
/// xdg_toplevel tiled_* states, X11 the edges of a one-direction maximize); the UI keeps its
/// resize band off them, as from a maximized window
pub const WST_TILED_SHIFT: u32 = 7;
pub const WST_TILED_TOP: u32 = EDGE_TOP << WST_TILED_SHIFT;
pub const WST_TILED_BOTTOM: u32 = EDGE_BOTTOM << WST_TILED_SHIFT;
pub const WST_TILED_LEFT: u32 = EDGE_LEFT << WST_TILED_SHIFT;
pub const WST_TILED_RIGHT: u32 = EDGE_RIGHT << WST_TILED_SHIFT;
/// (Windows) the window is not shown, whoever hid it: the system hides a tool window while its owner
/// is minimised
pub const WST_HIDDEN: u32 = 1 << 11;

// gpu_out_state: how the UI's output through the GPU came up
/// nothing known, nothing to remember
pub const GPU_STATE_UNKNOWN: u32 = 2;
/// the first start is about to run a driver
pub const GPU_STATE_TRYING: u32 = 3;
/// a window showed through the GPU
pub const GPU_STATE_OK: u32 = 4;
/// this system cannot: the CPU path for good
pub const GPU_STATE_NEVER: u32 = 5;

// ---- clipboard formats
pub const CLIP_TEXT: u32 = 1; // UTF-8
pub const CLIP_PNG: u32 = 2; // PNG file bytes
pub const CLIP_IMAGE: u32 = 3; // get: image file bytes (PNG, or DIB as BMP); set: PNG + DIB
/// UTF-8 HTML fragment (Windows CF_HTML framing is handled by the platform).
pub const CLIP_HTML: u32 = 4;
/// Rich Text Format document bytes.
pub const CLIP_RTF: u32 = 5;
/// Private document fragment bytes, MIME application/x-jfgui-docx-fragment.
pub const CLIP_DOCX: u32 = 6;

// ---- platform windows (window-indexed API, docs/modules/WINDOWS_PLATFORM.md)
pub const PW_MAX: usize = 8; // index 0 = the main window
pub const PW_DEFAULT: i32 = i32::MIN; // win_create x / y: the system (or owner-relative) default
pub const PW_KEEP: i32 = i32::MIN; // win_set_rect: leave this field unchanged
pub const PW_NOOWNER: i32 = -1; // win_create owner: a top-level window
pub const PWF_TOOL: u32 = 1; // win_create flags (the same bits as the UI's WF_*)
pub const PWF_DIALOG: u32 = 2;
pub const PWF_MODAL: u32 = 4;
pub const PWF_RESIZABLE: u32 = 8;
pub const PWF_NOCLOSE: u32 = 16;
pub const PWF_SYSFRAME: u32 = 32; // system caption / border (else the app draws the frame)
pub const PWF_NOACTIVATE: u32 = 64;
pub const PWF_MASK: u32 = 127;
pub const PWS_HIDE: u32 = 0; // win_show modes
pub const PWS_SHOW: u32 = 1;
pub const PWS_SHOWNA: u32 = 2; // show without activating
pub const PPW_MAX: u32 = 4; // popup slots (popup())
// popup_anchor kinds: how a popup hangs at its anchor when the platform places it (Wayland's
// xdg_positioner; the compositor keeps it on screen)
pub const PA_FREE: u32 = 0; // where it is (slides to stay on screen)
pub const PA_BELOW: u32 = 1; // below the anchor; above it when there is no room
pub const PA_ABOVE: u32 = 2; // above the anchor; below it when there is no room
pub const PA_SIDE: u32 = 3; // beside the anchor (submenus); on its other side when there is no room
pub const PA_POINT: u32 = 4; // at a point (context menus): flips both ways

// ---- file dialog (file_dialog flags)
pub const FD_OPEN: u32 = 0;
pub const FD_SAVE: u32 = 1;
pub const FD_FOLDER: u32 = 2; // choose a folder (Android: a document tree; Windows: the own dialog)
pub const FD_MULTI: u32 = 4; // (E8) FD_OPEN: several files; all of them in file_dialog_paths()
pub const FD_EXT_SHIFT: u32 = 8; // bits 8..15: initially selected extension (index)
/// file_dialog's answer when the system answers later (Android's picker): file_dialog_poll
pub const FD_PENDING: i32 = -2;

// ---- UI hit-box flags the platform asks through the hit hook (HB_*)
pub const HB_FOCUS: u32 = 1;
pub const HB_BLOCK: u32 = 2;
pub const HB_DRAG: u32 = 4; // window drag region (title bar)
pub const HB_TEXT: u32 = 8; // text input
pub const HB_MOVES: u32 = 16;
pub const HB_DISABLED: u32 = 32;
pub const HB_MIN: u32 = 64; // caption buttons (snap layouts, ...)
pub const HB_MAX: u32 = 128;
pub const HB_CLOSE: u32 = 256;
pub const HB_CURSOR_SHIFT: u32 = 12;

// ---- (L7) full screen: win_cmd commands; win_state reports WST_FULLSCREEN
pub const WCMD_FULLSCREEN: u32 = 5; // the window covers its monitor without a frame
pub const WCMD_WINDOWED: u32 = 6; // back to the place and frame it had before
// below the other windows (a title bar action, TB_ACT_LOWER: X11 and Windows; Wayland has no
// such request: nothing)
pub const WCMD_LOWER: u32 = 7;
