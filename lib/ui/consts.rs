// consts -- public constants of the immediate-mode UI                (ASMLIB/inc/ui.inc)
//
// Widget calls return event flags.  Bit 0 (EV_ACT) always means "the primary action
// happened" (clicked, changed, selected, closed).  Sizes and positions given to the UI are
// DIP (device independent pixels, 96 dpi = 100 %); drawing calls and last_rect use
// physical pixels.

// ------------------------------------------------------------------ widget events
pub const EV_ACT: u32 = 1; // primary action (click / change / select / close)
pub const EV_COMMIT: u32 = 2; // value committed (drag released, Enter, focus lost)
pub const EV_HOVER: u32 = 4; // pointer is over the widget
pub const EV_HELD: u32 = 8; // pressed and still held
pub const EV_FOCUS: u32 = 16; // widget has keyboard focus
pub const EV_DOUBLE: u32 = 32; // double click
pub const EV_RCLICK: u32 = 64; // right click (released over the widget)
pub const EV_OPEN: u32 = 128; // a popup / dropdown opened
pub const EV_DRAG: u32 = 256; // being dragged (moved while held)

// ------------------------------------------------------------------ hit box flags (behave)
pub const HB_FOCUS: u32 = 1; // reachable with Tab, takes focus on click
pub const HB_BLOCK: u32 = 2; // swallows the pointer (panels, popups)
pub const HB_DRAG: u32 = 4; // window drag region (title bar)
pub const HB_TEXT: u32 = 8; // text input (gets all keys while focused)
pub const HB_MOVES: u32 = 16; // wants a frame on every pointer move while hovered
pub const HB_DISABLED: u32 = 32; // disabled: visible to the dump, no input
pub const HB_MIN: u32 = 64; // caption buttons (for the OS: snap layouts, etc.)
pub const HB_MAX: u32 = 128;
pub const HB_CLOSE: u32 = 256;
// keyboard (kbnav.rs): a focusable box that is not a Tab stop (focus(id), arrows of its group
// reach it); the current member of a focus group (Tab enters the group there: the checked
// radio / tool button, the selected swatch); a splitter grip (F8 reaches it, arrows move it)
pub const HB_NOTAB: u32 = 512;
pub const HB_CURRENT: u32 = 1024;
pub const HB_SPLITTER: u32 = 2048;
pub const HB_CURSOR_SHIFT: u32 = 12; // cursor shape: flags | CUR_* << HB_CURSOR_SHIFT
// behave only (not kept in the hit list): Space acts, Enter does not (check boxes, radios,
// switches: Enter belongs to the dialog's default button); Space and Enter are the widget's
// own (lists, grids, trees: behave gives no EV_ACT for them)
pub const HB_SPACE: u32 = 0x10000;
pub const HB_OWNKEYS: u32 = 0x20000;

// ------------------------------------------------------------------ semantic kinds and states
pub const K_BOX: u16 = 0;
pub const K_TEXT: u16 = 1;
pub const K_BUTTON: u16 = 2;
pub const K_CHECKBOX: u16 = 3;
pub const K_RADIO: u16 = 4;
pub const K_SWITCH: u16 = 5;
pub const K_SLIDER: u16 = 6;
pub const K_PROGRESS: u16 = 7;
pub const K_TEXTBOX: u16 = 8;
pub const K_MENU: u16 = 9;
pub const K_MENUITEM: u16 = 10;
pub const K_LIST: u16 = 11;
pub const K_LISTITEM: u16 = 12;
pub const K_TAB: u16 = 13;
pub const K_CANVAS: u16 = 14;
pub const K_ICON: u16 = 15;
pub const K_IMAGE: u16 = 16;
pub const K_COMBO: u16 = 17;
pub const K_DIALOG: u16 = 18;
pub const K_TITLEBAR: u16 = 19;
pub const K_SWATCH: u16 = 20;
pub const K_LINK: u16 = 21;
/// a heading of a document or section (value: its level, 1 the top; md_view.rs, Ui::heading)
pub const K_HEADING: u16 = 22;
pub const K_KIND_COUNT: u16 = 23;
pub const SS_HOVER: u16 = 1;
pub const SS_PRESSED: u16 = 2;
pub const SS_FOCUSED: u16 = 4;
pub const SS_CHECKED: u16 = 8;
pub const SS_DISABLED: u16 = 16;
pub const SS_SELECTED: u16 = 32;
pub const SS_OPEN: u16 = 64;

/// names of the K_* kinds (dump, selectors kind:"Label")
pub const KIND_NAMES: [&str; 23] = [
    "box", "text", "button", "checkbox", "radio", "switch", "slider", "progress", "textbox",
    "menu", "menuitem", "list", "listitem", "tab", "canvas", "icon", "image", "combo", "dialog",
    "titlebar", "swatch", "link", "heading",
];
/// names of the SS_* state bits (dump)
pub const STATE_NAMES: [&str; 7] = ["hover", "pressed", "focused", "checked", "disabled", "selected", "open"];

// ------------------------------------------------------------------ layers
pub const L_BASE: u32 = 0;
pub const L_DIALOG: u32 = 1;
pub const L_POPUP: u32 = 2;
pub const L_TIP: u32 = 3;
pub const L_COUNT: usize = 4;

// ------------------------------------------------------------------ box flags (box_begin)
pub const BOX_H: u32 = 0; // children left to right
pub const BOX_V: u32 = 1; // children top to bottom
pub const BOX_CENTER: u32 = 2; // centre children along the main axis
pub const BOX_END: u32 = 4; // align children to the end of the main axis
pub const BOX_XCENTER: u32 = 8; // cross axis: centre children (default: stretch)
pub const BOX_XSTART: u32 = 16; // cross axis: natural size at the start
pub const BOX_XEND: u32 = 32; // cross axis: natural size at the end
pub const BOX_CARD: u32 = 64; // draw a card (fill + stroke, 8 DIP corners)
pub const BOX_BG: u32 = 128; // fill with the window background colour
pub const BOX_CLIP: u32 = 256; // clip children to the box
pub const BOX_BLOCK: u32 = 512; // swallow pointer input over the box
pub const BOX_DISABLED: u32 = 1024; // disable every widget inside
pub const BOX_LAYER: u32 = 2048; // fill with the layer colour (command bars)
pub const BOX_WRAP: u32 = 4096; // (horizontal) start a new row when full
pub const BOX_FIXED: u32 = 8192; // the box's size never depends on its content
// grid flags (gridbox_begin, with the box decorations and padding): the children's alignment
pub const GRID_H_START: u32 = 2; // horizontal (default: stretched across the cell)
pub const GRID_H_CENTER: u32 = 4;
pub const GRID_H_END: u32 = 6;
pub const GRID_V_START: u32 = 8; // vertical (default: centred)
pub const GRID_V_END: u32 = 16;
pub const GRID_V_STRETCH: u32 = 24;
// next_align(h, v): one grid child's alignment
pub const AL_DEFAULT: u32 = 0; // the grid's
pub const AL_START: u32 = 1;
pub const AL_CENTER: u32 = 2;
pub const AL_END: u32 = 3;
pub const AL_STRETCH: u32 = 4;
pub const PAD_SHIFT: u32 = 16; // padding in DIP: flags | N << PAD_SHIFT
pub const PAD_2: u32 = 2 << 16;
pub const PAD_4: u32 = 4 << 16;
pub const PAD_8: u32 = 8 << 16;
pub const PAD_12: u32 = 12 << 16;
pub const PAD_16: u32 = 16 << 16;
pub const PAD_20: u32 = 20 << 16;
pub const PAD_24: u32 = 24 << 16;
pub const PAD_32: u32 = 32 << 16;

/// padding in DIP for box_begin flags: BOX_V | pad(24)
pub const fn pad(dip: u32) -> u32 {
    dip << PAD_SHIFT
}

// easing curves (ease, anim_ex, anim_color)
pub const EASE_OUT: u32 = 0; // fast start, soft stop (quadratic; anim's)
pub const EASE_LINEAR: u32 = 1;
pub const EASE_IN: u32 = 2; // soft start (quadratic)
pub const EASE_IN_OUT: u32 = 3; // soft start and stop (cubic)
pub const EASE_OUT_CUBIC: u32 = 4; // a softer stop than EASE_OUT
pub const EASE_OUT_BACK: u32 = 5; // overshoots a little, then settles

// next_size values: > 0 fixed DIP, 0 = natural size, < 0 = fill weight
pub const SIZE_AUTO: i32 = 0;
pub const SIZE_FILL: i32 = -1;

// ------------------------------------------------------------------ text styles (text)
pub const TS_CAPTION: u32 = 0; // 12 / 16
pub const TS_BODY: u32 = 1; // 14 / 20
pub const TS_BODY_STRONG: u32 = 2; // 14 / 20 semibold
pub const TS_SUBTITLE: u32 = 3; // 20 / 28 semibold
pub const TS_TITLE: u32 = 4; // 28 / 36 semibold
pub const TS_TITLE_LARGE: u32 = 5; // 40 / 52 semibold
pub const TS_DISPLAY: u32 = 6; // 68 / 92 semibold
pub const TS_COUNT: u32 = 7;
pub const TX_SECONDARY: u32 = 0x100; // secondary text colour
pub const TX_TERTIARY: u32 = 0x200;
pub const TX_ACCENT: u32 = 0x400; // accent text colour
pub const TX_WRAP: u32 = 0x800; // wrap at the box width
pub const TX_ELLIPSIS: u32 = 0x1000; // cut with "…" when too long
pub const TX_CENTER: u32 = 0x2000; // horizontal alignment
pub const TX_RIGHT: u32 = 0x4000;
pub const TX_DISABLED: u32 = 0x8000;
pub const TX_ON_ACCENT: u32 = 0x10000; // text on an accent fill (T_ON_ACCENT)
pub const TX_HYPHENATE: u32 = 0x20000; // with TX_WRAP: hyphenate words that do not fit (hyph.rs)

// ------------------------------------------------------------------ widget style flags (next_flags)
pub const ST_DISABLED: u32 = 1;
pub const ST_LARGE: u32 = 2; // icon button: big icon above the label
pub const ST_SUBTLE: u32 = 4; // transparent button (toolbar style)
pub const ST_ACCENT: u32 = 8; // accent button
pub const ST_CHECKED: u32 = 16; // toggle / tool buttons: checked look
pub const ST_COMPACT: u32 = 32; // smaller height
pub const ST_NOFOCUS: u32 = 64; // not reachable with Tab
pub const ST_ICON_ONLY: u32 = 128; // hide the label (tooltip only)
pub const ST_ALIGN_START: u32 = 256; // start-aligned button content; styles may ellipsize long labels
/// Button displayed on an accent-colored surface, using its on-accent foreground.
pub const ST_ON_ACCENT: u32 = 512;
/// Button with a menu indicator beside its label; use the normal popup API for behavior.
pub const ST_MENU: u32 = 1024;
/// Rounded identity/action button; supporting styles use a circular or pill surface.
pub const ST_ROUND: u32 = 2048;

// ------------------------------------------------------------------ built-in commands (App::command ids)
pub const UIC_CLOSE: u32 = 0x7FFF_0001; // window close requested: return true to keep the window
pub const UIC_THEME: u32 = 0x7FFF_0002; // theme changed
pub const UIC_DPI: u32 = 0x7FFF_0003; // dpi changed
pub const UIC_ESCAPE: u32 = 0x7FFF_0004; // Esc not consumed by any widget
pub const UIC_TIMER: u32 = 0x7FFF_0005; // arg = timer id
pub const UIC_WINCLOSE: u32 = 0x7FFF_0006; // arg = window id: close requested; true keeps it
pub const UIC_DROP: u32 = 0x7FFF_0007; // arg = window id: files dropped (drop_*)
pub const UIC_JOB: u32 = 0x7FFF_0008; // arg = job id finished
pub const UIC_WAKE: u32 = 0x7FFF_0009; // plat::wake() from another thread
pub const UIC_EXIT: u32 = 0x7FFF_000A; // the window loop ends (only after a real window ran)
pub const UIC_OPEN: u32 = 0x7FFF_000B; // arg = requests: later starts of the app (AF_SINGLE_INSTANCE, open_take)
pub const UIC_TRAY: u32 = 0x7FFF_000C; // the notification-area symbol: arg = TRAY_ACTIVATE / TRAY_SECONDARY (tray_set)
pub const UIC_NOTE: u32 = 0x7FFF_000D; // arg = id: a system notification was clicked (notify)
pub const UIC_NOTE_GONE: u32 = 0x7FFF_000E; // arg = id: a notification went away unclicked
// lifecycle (runtime.rs)
pub const UIC_ACTIVE: u32 = 0x7FFF_000F; // arg 1: one of the app's windows got the keyboard, 0: none has it
pub const UIC_SUSPEND: u32 = 0x7FFF_0010; // arg 1: the system goes to sleep, 0: it woke up
pub const UIC_SESSION_END: u32 = 0x7FFF_0011; // logout / shutdown / SIGTERM: save now; UIC_EXIT and the end follow
pub const UIC_SETTINGS: u32 = 0x7FFF_0012; // arg = plat::SET_* bits: system settings changed (theme and format already applied)
// drag and drop (dnd.rs)
pub const UIC_DRAG_END: u32 = 0x7FFF_0013; // arg = DE_* effect (0 = cancelled / refused): a drag of the app ended
// permissions (perm.rs)
pub const UIC_PERMISSION: u32 = 0x7FFF_0014; // arg 1 allowed, 0 refused: the user answered a permission question (try the access again)
pub const TRAY_ACTIVATE: u32 = 1; // UIC_TRAY: the symbol was clicked
pub const TRAY_SECONDARY: u32 = 2; // UIC_TRAY: double click (Windows) / middle click (Linux)

// ------------------------------------------------------------------ drag and drop (dnd.rs)
// drag_source / drop_target states; the kinds DT_* and effects DE_* come from plat (dnd.rs)
pub const DS_START: u32 = 1; // drag_source: the widget starts being dragged (call drag_set now)
pub const DS_ACTIVE: u32 = 2; // drag_source: the widget's content is being dragged
pub const DS_END: u32 = 4; // drag_source: the drag ended (once; drag_effect() tells how)
pub const DS_HOVER: u32 = 8; // drop_target: a matching drag is over the widget (highlighted)
pub const DS_DROP: u32 = 16; // drop_target: dropped onto the widget (drop_data(), drop_effect())

// ------------------------------------------------------------------ app flags (AppDesc)
pub const AF_CUSTOM_TITLEBAR: u32 = 1;
pub const AF_RESIZABLE: u32 = 2;
pub const AF_DARK: u32 = 4; // force dark theme
pub const AF_LIGHT: u32 = 8; // force light theme
pub const AF_SINGLE_INSTANCE: u32 = 16; // one window per user: later starts hand their command line over (UIC_OPEN)
pub const AF_LOG_FILE: u32 = 32; // window mode: the log also goes to logs://APP.log (1 MiB, 3 older files)
pub const AF_SAVE_STATE: u32 = 64; // window mode: the main and tool windows' placement kept in config://windows.ini

// ------------------------------------------------------------------ theme tokens (color)
// byte offsets as in ASMLIB; index = token / 4
pub const T_BG: u32 = 0; // window background (Mica fallback)
pub const T_BG_ALT: u32 = 4;
pub const T_LAYER: u32 = 8; // layer fill (command bars, panels)
pub const T_LAYER_STROKE: u32 = 12;
pub const T_CARD: u32 = 16;
pub const T_CARD_STROKE: u32 = 20;
pub const T_CTRL: u32 = 24; // control fill: rest, hover, pressed, disabled (24..36)
pub const T_CTRL_HOVER: u32 = 28;
pub const T_CTRL_PRESSED: u32 = 32;
pub const T_CTRL_DISABLED: u32 = 36;
pub const T_CTRL_STROKE: u32 = 40; // control border
pub const T_CTRL_STROKE2: u32 = 44; // control border, bottom edge
pub const T_STRONG_STROKE: u32 = 48; // checkbox / radio border
pub const T_SUBTLE: u32 = 52; // transparent button: rest, hover, pressed, disabled (52..64)
pub const T_SUBTLE_HOVER: u32 = 56;
pub const T_SUBTLE_PRESSED: u32 = 60;
pub const T_SUBTLE_DISABLED: u32 = 64;
pub const T_TEXT: u32 = 68; // text: primary, secondary, tertiary, disabled (68..80)
pub const T_TEXT2: u32 = 72;
pub const T_TEXT3: u32 = 76;
pub const T_TEXT_DISABLED: u32 = 80;
pub const T_ACCENT: u32 = 84; // accent fill: rest, hover, pressed, disabled (84..96)
pub const T_ACCENT_HOVER: u32 = 88;
pub const T_ACCENT_PRESSED: u32 = 92;
pub const T_ACCENT_DISABLED: u32 = 96;
pub const T_ON_ACCENT: u32 = 100; // text on accent
pub const T_ON_ACCENT_DIS: u32 = 104;
pub const T_ACCENT_TEXT: u32 = 108; // accent-coloured text / links
pub const T_FOCUS_OUT: u32 = 112;
pub const T_FOCUS_IN: u32 = 116;
pub const T_FLYOUT: u32 = 120; // menus, popups
pub const T_FLYOUT_STROKE: u32 = 124;
pub const T_SMOKE: u32 = 128; // dim behind dialogs
pub const T_DIVIDER: u32 = 132;
pub const T_INPUT_ACTIVE: u32 = 136; // focused text box fill
pub const T_CRIT: u32 = 140;
pub const T_OK: u32 = 144;
pub const T_WARN: u32 = 148;
pub const T_INFO: u32 = 152;
pub const T_CRIT_BG: u32 = 156;
pub const T_OK_BG: u32 = 160;
pub const T_WARN_BG: u32 = 164;
pub const T_INFO_BG: u32 = 168;
pub const T_CLOSE_HOVER: u32 = 172;
pub const T_SHADOW: u32 = 176;
pub const T_SCROLL: u32 = 180; // scroll bar thumb
pub const T_TRACK: u32 = 184; // slider / progress track
pub const T_KNOB: u32 = 188; // solid control fill (slider thumb, toggle knob)
pub const T_ALT_FILL: u32 = 192; // toggle off background
pub const T_STRONG_FILL: u32 = 196; // toggle off knob / strong fill
pub const T_COUNT: usize = 50;

// ------------------------------------------------------------------ records
/// RANGE min, max, step, flags -- slider / number box descriptor
#[derive(Clone, Copy, Default, Debug)]
pub struct Range {
    pub min: i32,
    pub max: i32,
    pub step: i32,
    pub flags: u32,
}

impl Range {
    pub const fn new(min: i32, max: i32, step: i32, flags: u32) -> Range {
        Range { min, max, step, flags }
    }
}

pub const RG_VERT: u32 = 1; // vertical slider
pub const RG_TICKS: u32 = 2;
pub const RG_TIP: u32 = 4; // value tooltip while dragging
/// Logarithmic pointer/paint mapping for integer sliders with positive bounds.
/// Keyboard increments, semantic values and range endpoints retain their units.
pub const RG_LOG: u32 = 8;
pub const LF_REORDER: u32 = 0x10000; // list_begin: rows can be dragged to a new place

// decimal values, colour tracks, angle, pan, colour wheel (ui_value.s, ui_wheel.s)
pub const TK_CHECKER: u32 = 1; // draw the gradient over a checkerboard (alpha tracks)
pub const TK_CUSTOM: u32 = 2; // the track is drawn by a custom function (no stops)
pub const TK_RECT: u32 = 4; // square ends, the gradient spans the whole rectangle
pub const TK_MAXN: usize = 16;
pub const ANG_360: u32 = 1; // angle: 0 .. 360 instead of -180 .. 180
pub const PAN_UNIT: u32 = 1; // pan: coordinates 0 .. 1 instead of -1 .. 1
pub const CW_HSV: u32 = 1; // color_wheel: an HSV record instead of 0xAARRGGBB
pub const CP_ALPHA: u32 = 1; // color_picker_ex: alpha (transparency) track
pub const CP_FIELDS: u32 = 2; // ... number boxes R G B (A) H S V and a hex text box
pub const PART_BASE: u32 = 0x6000_0000; // part_id(id, k): ids of the parts of composite widgets

/// UIINPUT -- pointer state handed to custom widgets (canvas)
#[derive(Clone, Default)]
pub struct UiInput {
    pub ev: u32,          // EV_* flags
    pub btn: u32,         // buttons held (bit 1 << BTN_*)
    pub pressed: u32,     // buttons pressed this frame
    pub released: u32,    // buttons released this frame
    pub x: i32,           // pointer x relative to the widget (px)
    pub y: i32,
    pub press_x: i32,     // position of the last press (px, relative)
    pub press_y: i32,
    pub wheel_x: i32,     // wheel delta (120 per notch)
    pub wheel_y: i32,
    pub mods: u32,
    pub pts: alloc::vec::Vec<[i32; 2]>, // move points since the last frame (relative px)
    pub rect: [i32; 4],   // widget rectangle (absolute px)
    pub dpi: i32,
    pub key: u32,         // first unconsumed key this frame (VK | mods << 16) or 0
    pub pressure: u32,    // pen pressure now: 0 = none (mouse), 1..UI_PRESSURE_ONE
    pub ptype: u32,       // PTR_MOUSE / PTR_PEN / PTR_ERASER / PTR_TOUCH
    pub ppress: alloc::vec::Vec<u32>, // pressure per move point
    pub tilt_x: i32,      // pen tilt in degrees (-63..63), 0 without tilt
    pub tilt_y: i32,
}

pub const UI_PRESSURE_ONE: u32 = 65535;

// ------------------------------------------------------------------ containers
pub const SC_V: u32 = 0x0100_0000; // vertical scrolling, children top to bottom (default)
pub const SC_H: u32 = 0x0200_0000; // horizontal scrolling, children left to right
pub const SC_NOBAR: u32 = 0x0400_0000; // no scroll bars
pub const SC_NOWHEEL: u32 = 0x0800_0000; // ignore the mouse wheel
pub const IB_INFO: u32 = 0;
pub const IB_SUCCESS: u32 = 1;
pub const IB_WARNING: u32 = 2;
pub const IB_ERROR: u32 = 3;
pub const DLG_NONE: u32 = 0;
pub const DLG_PRIMARY: u32 = 1;
pub const DLG_SECONDARY: u32 = 2;
pub const DLG_CLOSE: u32 = 3;

// ------------------------------------------------------------------ text input
pub const TB_CAP_MASK: u32 = 0x007F_FFFF; // the capacity in bytes (at most 8 MB)
pub const TB_HYPHENATE: u32 = 0x0080_0000; // with TB_MULTI | TB_WRAP: hyphenate words that do not fit (hyph.rs)
pub const TB_PASSWORD: u32 = 0x0100_0000; // a dot per character; no copy / cut
pub const TB_READONLY: u32 = 0x0200_0000; // selectable and copyable, not editable
pub const TB_DIGITS: u32 = 0x0400_0000; // accept only 0-9
pub const TB_MULTI: u32 = 0x0800_0000; // multi-line
pub const TB_SIGNED: u32 = 0x1000_0000; // with TB_DIGITS: also '-'
pub const TB_DECIMAL: u32 = 0x2000_0000; // with TB_DIGITS: also ',' and '.'
pub const TB_WRAP: u32 = 0x4000_0000; // with TB_MULTI: wrap long lines at the box width
pub const TB_SPELL: u32 = 0x8000_0000; // spell checking: misspelled words underlined, suggestions (edit_spell.rs)
pub const UIT_CARET: u32 = 0x7FFE_0001; // timer id of the caret blink

// ------------------------------------------------------------------ popups, menus, commands
pub const UIT_TOOLTIP: u32 = 0x7FFE_0010; // timer id of the tooltip delay
pub const FL_BELOW: u32 = 0;
pub const FL_ABOVE: u32 = 1;
pub const FL_RIGHT: u32 = 2;
pub const FL_ALIGN_END: u32 = 4;
pub const FL_MINW: u32 = 8;
pub const FL_POINT: u32 = 16;
pub const FL_OVER: u32 = 32;
pub const CF_DISABLED: u32 = 1;
pub const CF_CHECKED: u32 = 2;
pub const CF_HIDDEN: u32 = 4;
pub const FD_OPEN: u32 = 0;
pub const FD_SAVE: u32 = 1;
pub const FD_FOLDER: u32 = 2;
pub const FD_EXT_SHIFT: u32 = 8;

// ------------------------------------------------------------------ windows (win.rs)
// windows including the main window (index 0); fixed: the index is a bit in u32 masks and
// Event::win, plat keeps PW_MAX windows; one more warns (window_begin)
pub const WIN_MAX: usize = 8;
pub const WF_TOOL: u32 = 1;
pub const WF_DIALOG: u32 = 2;
pub const WF_MODAL: u32 = 4;
pub const WF_RESIZABLE: u32 = 8;
pub const WF_NOCLOSE: u32 = 16;
pub const WF_SYSFRAME: u32 = 32;
pub const WF_NOACTIVATE: u32 = 64;
pub const WF_AUTOHEIGHT: u32 = 128;
pub const WF_KEYS: u32 = 256;
pub const UWS_OPEN: u32 = 1;
pub const UWS_VISIBLE: u32 = 2;
pub const UWS_ACTIVE: u32 = 4;
pub const UWS_MINIMIZED: u32 = 8;

/// WINRECT -- window placement, owned by the app (DIP; see ui.inc)
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct WinRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub flags: u32,
}

impl WinRect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32, flags: u32) -> WinRect {
        WinRect { x, y, w, h, flags }
    }
}

pub const WRF_POS: u32 = 1;
pub const WRF_MAX: u32 = 2;
pub const WRF_CHANGED: u32 = 4;
pub const WRF_ABS: u32 = 8;

// ------------------------------------------------------------------ input (jfgui.inc)
pub const BTN_LEFT: u32 = 1;
pub const BTN_RIGHT: u32 = 2;
pub const BTN_MIDDLE: u32 = 3;
pub const BTN_X1: u32 = 4;
pub const BTN_X2: u32 = 5;
pub const MOD_SHIFT: u32 = 1;
pub const MOD_CTRL: u32 = 2;
pub const MOD_ALT: u32 = 4;
pub const MOD_SUPER: u32 = 8;
pub const MOD_ALTGR: u32 = 16;
pub const MOD_CAPS: u32 = 32;
pub const MOD_NUM: u32 = 64;
pub const PTR_MOUSE: u32 = 0;
pub const PTR_PEN: u32 = 1;
pub const PTR_ERASER: u32 = 2;
pub const PTR_TOUCH: u32 = 3;

// cursors (CUR_*)
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
pub const CUR_CUSTOM: u32 = 15; // hit boxes only: the cursor of cursor_custom
pub const CUR_USER_MAX: u32 = 32; // fixed handle range; cursor_new answers 0 when it is full
pub const CUR_MAX_PX: i32 = 256;

// fonts
pub const FONT_REG: u32 = 0;
pub const FONT_SEMI: u32 = 1;
pub const FONT_BOLD: u32 = 2;
pub const FONT_ICON: u32 = 3;

// clipboard formats
pub const CLIP_TEXT: u32 = 1;
pub const CLIP_PNG: u32 = 2;
pub const CLIP_IMAGE: u32 = 3;
/// UTF-8 HTML fragment; CF_HTML framing is handled by the platform.
pub const CLIP_HTML: u32 = 4;
/// Rich Text Format document bytes.
pub const CLIP_RTF: u32 = 5;
/// Private document fragment bytes, MIME application/x-jfgui-docx-fragment.
pub const CLIP_DOCX: u32 = 6;

// ------------------------------------------------------------------ style metrics (style.inc)
pub const SM_DLG_MIN: u32 = 0;
pub const SM_DLG_MAX: u32 = 1;
pub const SM_DLG_MARGIN: u32 = 2;
pub const SM_DLG_CONTENT_BOX: u32 = 3;
pub const SM_DLG_CONTENT_GAP: u32 = 4;
pub const SM_DLG_TITLE_TS: u32 = 5;
pub const SM_DLG_FOOTER_BOX: u32 = 6;
pub const SM_DLG_FOOTER_GAP: u32 = 7;
pub const SM_SB_H: u32 = 8;
pub const SM_SB_PAD: u32 = 9;
pub const SM_SB_GAP: u32 = 10;
pub const SM_TB_H: u32 = 11;
pub const SM_TB_H_LARGE: u32 = 12;
pub const SM_TB_PAD: u32 = 13;
pub const SM_TB_ICON: u32 = 14;
pub const SM_TB_TITLE_TS: u32 = 15;
pub const SM_CAP_W: u32 = 16;
pub const SM_CAP_W_TOOL: u32 = 17;
pub const SM_WC_TITLE_PAD: u32 = 18;
pub const SM_WC_TITLE_GAP: u32 = 19;
pub const SM_GRIP: u32 = 20;
pub const SM_GRIP_CORNER: u32 = 21;
pub const SM_WIN_TBH_TOOL: u32 = 22;
pub const SM_WIN_TBH_DIALOG: u32 = 23;
pub const SM_CAP_LEFT: u32 = 24;
pub const SM_DLG_BTN_ORDER: u32 = 25;
pub const SM_DOCK_HDR: u32 = 26; // tab strip of a dock group (dock.rs; 0 = 32)
pub const SM_DOCK_BTN: u32 = 27; // its buttons (0 = 28)
pub const SM_COUNT: u32 = 28;
// menu / list rows of flyouts (menu_item*: Sp::flags)
pub const MIF_DISABLED: u32 = 1;
pub const MIF_CHECKED: u32 = 2;
pub const MIF_SUB: u32 = 4;
pub const MIF_OPEN: u32 = 8;
pub const MIF_SELECTED: u32 = 16;
pub const MIF_LIST: u32 = 32;
pub const MIF_TWOCOL: u32 = 64;

// ------------------------------------------------------------------ data views (data.rs, chart.rs)
// badge / status dot / meter kinds (StyleData::badge flags)
pub const BADGE_NEUTRAL: u32 = 0;
pub const BADGE_INFO: u32 = 1;
pub const BADGE_OK: u32 = 2;
pub const BADGE_WARN: u32 = 3;
pub const BADGE_CRIT: u32 = 4;
pub const BADGE_ACCENT: u32 = 5;
// grid column flags (GridCol::flags)
pub const GC_RIGHT: u32 = 1; // cell content right-aligned (numbers)
pub const GC_CENTER: u32 = 2;
pub const GC_NOSORT: u32 = 4; // the header does not sort
pub const GC_FILL: u32 = 8; // the column takes the width the others leave (at least its width)
// grid header cell paint flags (StyleData::grid_header Sp::flags)
pub const GH_SORT_ASC: u32 = 1;
pub const GH_SORT_DESC: u32 = 2;
pub const GH_LAST: u32 = 4; // the last column (no divider)
pub const GH_RIGHT: u32 = 8; // label right-aligned
// grid row paint flags (StyleData::grid_row Sp::flags)
pub const GR_FOCUS: u32 = 1; // the focused row of a focused grid (focus visual)
pub const GR_STRIPE: u32 = 2; // an odd row of a striped grid
pub const GR_CURRENT: u32 = 4; // the current (focused) row, focus not visible
// ------------------------------------------------------------------ shell (shell.rs, notify.rs)
pub const NAV_COMPACT: u32 = 1; // nav_item / nav_section: the pane shows icons only
pub const SEG_FIRST: u32 = 1; // segment: the first / last item of the control
pub const SEG_LAST: u32 = 2;
pub const WS_TODO: i32 = 0; // wizard step states
pub const WS_CURRENT: i32 = 1;
pub const WS_DONE: i32 = 2;
pub const WS_ERROR: i32 = 3;
pub const CBI_SEP: u32 = 0x1_0000; // command bar item: a separator before it
pub const CBI_END: u32 = 0x2_0000; // command bar item: starts the end-aligned group
pub const UIT_TOAST: u32 = 0x7FFE_0020; // timer id of toast expiry
pub const UIT_SPIN: u32 = 0x7FFE_0021; // (reserved) spinner frame timer
pub const UIT_SPELL: u32 = 0x7FFE_0030; // timer id: spell checking continues in the next frame
// spell_state(): the checker of the text boxes (edit_spell.rs)
pub const SPELL_OFF: u32 = 0; // spell_enable(false), or no TB_SPELL box asked yet
pub const SPELL_LOADING: u32 = 1; // its dictionary is being read (window mode)
pub const SPELL_READY: u32 = 2; // checking (the marks are up to date or follow on the timer)
pub const SPELL_NONE: u32 = 3; // no checker for the language
