// consts -- constants shared by the Kritzel modules  [apps/paint/inc/paint.inc]
//
// All of paint.inc, also the constants no module uses (yet).
#![allow(dead_code)]

pub(crate) const MAX_LAYERS: usize = 16;
pub(crate) const DOC_MAX: i32 = 16384; // largest canvas side in pixels
pub(crate) const DEF_W: i32 = 1152; // new image size
pub(crate) const DEF_H: i32 = 648;

// tools
pub(crate) const TOOL_PENCIL: i32 = 0;
pub(crate) const TOOL_FILL: i32 = 1;
pub(crate) const TOOL_TEXT: i32 = 2;
pub(crate) const TOOL_ERASER: i32 = 3;
pub(crate) const TOOL_PICKER: i32 = 4;
pub(crate) const TOOL_ZOOM: i32 = 5;
pub(crate) const TOOL_BRUSH: i32 = 6;
pub(crate) const TOOL_SELECT: i32 = 7;
pub(crate) const TOOL_SHAPE: i32 = 8;
pub(crate) const TOOL_COUNT: i32 = 9;

// brushes (TOOL_BRUSH variants)
pub(crate) const B_BRUSH: i32 = 0;
pub(crate) const B_CALLI_BRUSH: i32 = 1;
pub(crate) const B_CALLI_PEN: i32 = 2;
pub(crate) const B_AIRBRUSH: i32 = 3;
pub(crate) const B_OIL: i32 = 4;
pub(crate) const B_CRAYON: i32 = 5;
pub(crate) const B_MARKER: i32 = 6;
pub(crate) const B_PENCIL2: i32 = 7;
pub(crate) const B_WATERCOLOR: i32 = 8;
pub(crate) const B_COUNT: i32 = 9;

// shapes
pub(crate) const SH_LINE: i32 = 0;
pub(crate) const SH_CURVE: i32 = 1;
pub(crate) const SH_OVAL: i32 = 2;
pub(crate) const SH_RECT: i32 = 3;
pub(crate) const SH_RRECT: i32 = 4;
pub(crate) const SH_POLYGON: i32 = 5;
pub(crate) const SH_TRIANGLE: i32 = 6;
pub(crate) const SH_RTRIANGLE: i32 = 7;
pub(crate) const SH_DIAMOND: i32 = 8;
pub(crate) const SH_PENTAGON: i32 = 9;
pub(crate) const SH_HEXAGON: i32 = 10;
pub(crate) const SH_ARROW_R: i32 = 11;
pub(crate) const SH_ARROW_L: i32 = 12;
pub(crate) const SH_ARROW_U: i32 = 13;
pub(crate) const SH_ARROW_D: i32 = 14;
pub(crate) const SH_STAR4: i32 = 15;
pub(crate) const SH_STAR5: i32 = 16;
pub(crate) const SH_STAR6: i32 = 17;
pub(crate) const SH_CALLOUT_R: i32 = 18;
pub(crate) const SH_CALLOUT_O: i32 = 19;
pub(crate) const SH_CALLOUT_C: i32 = 20;
pub(crate) const SH_HEART: i32 = 21;
pub(crate) const SH_LIGHTNING: i32 = 22;
pub(crate) const SH_COUNT: i32 = 23;

// widget ids
pub(crate) const ID_CANVAS: u32 = 1000;
pub(crate) const ID_SIZE: u32 = 1001;
pub(crate) const ID_OPACITY: u32 = 1002;
pub(crate) const ID_ZOOM_SLIDER: u32 = 1003;
pub(crate) const ID_ZOOM_IN: u32 = 1004;
pub(crate) const ID_ZOOM_OUT: u32 = 1005;
pub(crate) const ID_ZOOM_FIT: u32 = 1006;
pub(crate) const ID_TOOL0: u32 = 1100; // + tool
pub(crate) const ID_COLOR1: u32 = 1200;
pub(crate) const ID_COLOR2: u32 = 1201;
pub(crate) const ID_PAL0: u32 = 1210; // + index (30)
pub(crate) const ID_EDITCOL: u32 = 1250;
pub(crate) const ID_BRUSH: u32 = 1260;
pub(crate) const ID_SHAPE0: u32 = 1300; // + shape
pub(crate) const ID_SELECT: u32 = 1400;
pub(crate) const ID_LAYERS: u32 = 1410;
pub(crate) const ID_UNDO_BTN: u32 = 1420;
pub(crate) const ID_REDO_BTN: u32 = 1421;
pub(crate) const ID_SAVE_BTN: u32 = 1422;
pub(crate) const ID_CROP: u32 = 1430;
pub(crate) const ID_RESIZE: u32 = 1431;
pub(crate) const ID_ROTATE: u32 = 1432;
pub(crate) const ID_HSCROLL: u32 = 1440;
pub(crate) const ID_VSCROLL: u32 = 1441;
pub(crate) const ID_HANDLE0: u32 = 1500; // + handle index (0..7)

// commands (on_command ids)
pub(crate) const CMD_NEW: u32 = 1;
pub(crate) const CMD_OPEN: u32 = 2;
pub(crate) const CMD_SAVE: u32 = 3;
pub(crate) const CMD_SAVE_AS: u32 = 4;
pub(crate) const CMD_EXIT: u32 = 5;
pub(crate) const CMD_FILE_DONE: u32 = 6; // the file dialog confirmed (arg = FD_OPEN / FD_SAVE)
pub(crate) const CMD_SAVE_TYPE0: u32 = 80; // "Speichern unter" > type i: CMD_SAVE_TYPE0 + index in
pub(crate) const CMD_SAVE_PNG: u32 = 80; //   the extension list (fileio.rs EXTS)
pub(crate) const CMD_SAVE_JPEG: u32 = 81;
pub(crate) const CMD_SAVE_BMP: u32 = 82;
pub(crate) const CMD_SAVE_GIF: u32 = 83;
pub(crate) const CMD_ASK_DONE: u32 = 7; // the "save changes?" prompt closed (arg = DLG_*)
pub(crate) const CMD_UNDO: u32 = 10;
pub(crate) const CMD_REDO: u32 = 11;
pub(crate) const CMD_CUT: u32 = 12;
pub(crate) const CMD_COPY: u32 = 13;
pub(crate) const CMD_PASTE: u32 = 14;
pub(crate) const CMD_SELECT_ALL: u32 = 15;
pub(crate) const CMD_DELETE: u32 = 16;
pub(crate) const CMD_SEL_INVERT: u32 = 17; // (select agent) invert the selection
pub(crate) const CMD_SEL_TRANSP: u32 = 18; // (select agent) toggle "Hintergrundfarbe aussparen"
pub(crate) const CMD_SEL_MODE: u32 = 19; // (select agent) arg 0 rectangle, 1 free-form
pub(crate) const CMD_ZOOM_IN: u32 = 20;
pub(crate) const CMD_ZOOM_OUT: u32 = 21;
pub(crate) const CMD_ZOOM_100: u32 = 22;
pub(crate) const CMD_ZOOM_FIT: u32 = 23;
pub(crate) const CMD_ROT_R: u32 = 30;
pub(crate) const CMD_ROT_L: u32 = 31;
pub(crate) const CMD_ROT_180: u32 = 32;
pub(crate) const CMD_FLIP_H: u32 = 33;
pub(crate) const CMD_FLIP_V: u32 = 34;
pub(crate) const CMD_CROP: u32 = 35;
pub(crate) const CMD_RESIZE: u32 = 36;
pub(crate) const CMD_PROPERTIES: u32 = 37;
pub(crate) const CMD_CANVAS: u32 = 38; // apply new_w x new_h (canvas handles)
pub(crate) const CMD_LAYER_ADD: u32 = 40;
pub(crate) const CMD_LAYER_DEL: u32 = 41;
pub(crate) const CMD_LAYER_DUP: u32 = 42;
pub(crate) const CMD_LAYER_MERGE: u32 = 43;
pub(crate) const CMD_LAYER_UP: u32 = 44;
pub(crate) const CMD_LAYER_DOWN: u32 = 45;
pub(crate) const CMD_TOGGLE_LAYERS: u32 = 46;
pub(crate) const CMD_EDIT_COLOR: u32 = 50;
pub(crate) const CMD_SWAP_COLORS: u32 = 51;
pub(crate) const CMD_THEME: u32 = 60;
pub(crate) const CMD_RESIZE_OK: u32 = 70; // dialogs: apply
pub(crate) const CMD_PROPS_OK: u32 = 71;
pub(crate) const CMD_COLOR_OK: u32 = 72;
