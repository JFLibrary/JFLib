// chrome -- PaintCraft: main window chrome like paint.net 5 (title row with menus, image list
// and window toggles; the two tool bar rows; the status bar)
//                                                                 [apps/PaintCraft/src/chrome.s]
//
//   chrome_init()                  window toggles on (all four tool windows shown)
//   chrome_states()                CF_CHECKED of the window toggles
//   chrome_cmd(id, arg) -> handled window toggles 700..705, Hilfe flyout 711..719,
//                                  UIC_WINCLOSE of the tool windows
//   chrome_wnd_open(WND_*) -> the visibility flag of a tool window (1 = shown), shared with
//                                  ui.window_begin
//   chrome_titlebar()              48 DIP title row: icon, menu bar, tabs_frame, toggles
//   chrome_toolbar()               tool bar row 1 (common commands)
//   chrome_toolrow()               tool bar row 2: "Werkzeug:" + tool menu + tool_options
//   chrome_statusbar()             tool hint | selection, size, cursor | units | zoom
//   PN_ZOOM_PRESETS                zoom presets in 1/100 % (descending, ZOOM_PRESETS_N)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

use crate::app::constants::*;
use crate::document::model::{doc_cur, doc_size};
use crate::workspace::view::{view_cursor, view_dpi100, view_units, view_zoom};
use crate::app::formatting::fmt_cap;
use crate::app::Pn;

const ROW_H: i32 = 40; // tool bar rows (DIP)
const TITLE_H: i32 = 48; // title row (ST_LARGE title bar)
const ZOOMLIST_CAP: usize = 512;
const SBTXT_CAP: usize = 64;
const BTN: u32 = ST_ICON_ONLY | ST_SUBTLE;
const TRP_BTN: i32 = 28; // tool row page button size (DIP)
const TRP_W: i32 = 2 * TRP_BTN + 2 * 6; // both buttons + the row gaps

/// pn_zoom_presets: paint.net-like zoom steps in 1/100 % (descending, like the zoom drop-down)
pub(crate) static PN_ZOOM_PRESETS: [u32; ZOOM_PRESETS_N as usize] = [
    640000, 480000, 360000, 240000, 160000, 120000, 80000, 70000, 60000, 50000, 40000, 30000, 20000, 17500,
    15000, 12500, 10000, 6600, 5000, 3300, 2500, 1600, 1200, 800, 500, 300, 200, 100,
];
const RANGE_ZSLIDER: Range = Range::new(0, ZOOM_PRESETS_N - 1, 1, 0);
/// tool bar row 1 (0 = divider)
static TOOLBAR_ROWS: [u32; 16] = [
    CMD_NEW,
    CMD_OPEN,
    CMD_SAVE,
    0,
    CMD_PRINT,
    0,
    CMD_CUT,
    CMD_COPY,
    CMD_PASTE,
    CMD_CROP,
    CMD_DESELECT,
    0,
    CMD_UNDO,
    CMD_REDO,
    0,
    CMD_GRID,
];
/// the Hilfe flyout (0 = separator)
static HELP_ROWS: [u32; 12] = [
    CMD_HELP_DOCS,
    CMD_HELP_SEARCH,
    0,
    CMD_HELP_FORUM,
    CMD_HELP_TUTORIALS,
    CMD_HELP_PLUGINS,
    CMD_HELP_WEBSITE,
    CMD_HELP_DONATE,
    0,
    CMD_HELP_FEEDBACK,
    0,
    CMD_HELP_ABOUT,
];
static UNITS_LIST: [&str; 3] = ["Pixel", "Zoll", "Zentimeter"];

/// the state of chrome.s
pub(crate) struct Chrome {
    /// g_wnd_open: the visibility flags of the four tool windows (WND_*)
    pub(crate) wnd_open: [Shared<i32>; WND_COUNT as usize],
    sb_units: i32,
    /// g_trp_on: tool row page buttons shown (last pass)
    trp_on: bool,
    sb_zsel: i32,
    sb_zslider: i32,
    sb_ulast: i32,
    sb_zlast: i32,
}

impl Default for Chrome {
    fn default() -> Chrome {
        Chrome {
            wnd_open: Default::default(),
            sb_units: 0,
            trp_on: false,
            sb_zsel: 0,
            sb_zslider: 0,
            sb_ulast: 0,
            sb_zlast: 0,
        }
    }
}

/// chrome_init() -- every tool window starts visible (paint.net default)
pub(crate) fn chrome_init(p: &mut Pn, _ui: &mut Ui) {
    let c = &mut p.chrome;
    c.sb_ulast = -1;
    c.sb_zlast = -1;
    for f in c.wnd_open.iter() {
        f.set(1);
    }
}

/// chrome_wnd_open(i) -> the flag (i = WND_*; out of range -> the Werkzeuge flag)
pub(crate) fn chrome_wnd_open(c: &Chrome, i: i32) -> Shared<i32> {
    let i = if (i as u32) < WND_COUNT as u32 { i as usize } else { 0 };
    c.wnd_open[i].clone()
}

/// chrome_states() -- toggles show the window state
pub(crate) fn chrome_states(p: &mut Pn, ui: &mut Ui) {
    for (i, f) in p.chrome.wnd_open.iter().enumerate() {
        crate::app::commands::pn_cmd_set(ui, CMD_WND_TOOLS + i as u32, if f.get() != 0 { CF_CHECKED } else { 0 });
    }
    crate::app::commands::pn_cmd_set(ui, CMD_WND_RESET, 0);
}

/// chrome_cmd(id, arg) -> handled
pub(crate) fn chrome_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    let k = id.wrapping_sub(CMD_WND_TOOLS);
    if k < WND_COUNT as u32 {
        let f = &p.chrome.wnd_open[k as usize];
        f.set(f.get() ^ 1);
        return true;
    }
    if id == CMD_WND_RESET {
        // Fensteranordnung zurücksetzen: every tool window back to its default place
        crate::workspace::panels::tools::wtools_reset_pos(p); // pn-ui
        crate::workspace::panels::history::whist_reset_pos(p); // pn-ui
        crate::workspace::panels::layers::wlay_reset_pos(p); // pn-ui
        crate::workspace::panels::colors::wcol_reset_pos(p); // pn-ui
        return true;
    }
    if id == CMD_HELP {
        ui.popup_open(ID_HELP_FLYOUT);
        return true;
    }
    // a tool window was closed with its own close button
    if id == UIC_WINCLOSE {
        let k = arg.wrapping_sub(WINID_TOOLS);
        if k < WND_COUNT as u32 {
            p.chrome.wnd_open[k as usize].set(0);
            return true;
        }
    }
    false
}

// ------------------------------------------------------------------ helpers

// cmdbtn(id) -- subtle icon-only button for a command (tooltip "Label (Strg+X)")
fn cmdbtn(ui: &mut Ui, id: u32) {
    ui.cmd_button(id, BTN);
}

// vsep() -- a short vertical divider between tool bar groups
fn vsep(ui: &mut Ui) {
    ui.spacer(4);
    ui.next_size(1, 20);
    ui.box_begin(0, BOX_H | BOX_FIXED, 0);
    let c = ui.color(T_CTRL_STROKE2);
    ui.draw_color(c);
    let r = ui.box_rect();
    ui.draw_rect(r[0], r[1], r[2], r[3]);
    ui.box_end();
    ui.spacer(4);
}

// hline() -- a hairline divider across the current vertical box
fn hline(ui: &mut Ui) {
    ui.next_size(-1, 1);
    ui.box_begin(0, BOX_H | BOX_FIXED, 0);
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    let h = ui.hair();
    let r = ui.box_rect();
    ui.draw_rect(r[0], r[1], r[2], h);
    ui.box_end();
}

// cmd_icon(id) -> icon of a command (0 when unknown)
fn cmd_icon(ui: &Ui, id: u32) -> u32 {
    ui.cmd_find(id).map_or(0, |c| c.icon)
}

// ------------------------------------------------------------------ title row

/// chrome_titlebar() -- app icon, menus, image list, window toggles, caption buttons
pub(crate) fn chrome_titlebar(p: &mut Pn, ui: &mut Ui) {
    ui.next_flags(ST_LARGE);
    ui.titlebar_begin_opt(None, 0); // no title text (paint.net shows the images)
    app_icon(ui);
    ui.spacer(6);
    ui.menubar(None);
    ui.spacer(12);
    // the image list takes the free width (weight 1000 against the end spacer's 1)
    ui.next_size(-1000, TITLE_H);
    crate::workspace::tabs::tabs_frame(p, ui);
    ui.spacer(8);
    // window toggles, settings, help: one Tab stop, the arrows move inside
    ui.focus_group_begin(ID_TITLE_TOOLS);
    for id in [CMD_WND_TOOLS, CMD_WND_HISTORY, CMD_WND_LAYERS, CMD_WND_COLORS] {
        cmdbtn(ui, id);
    }
    ui.spacer(8);
    cmdbtn(ui, CMD_SETTINGS);
    cmdbtn(ui, CMD_HELP);
    ui.focus_group_end();
    help_flyout(ui);
    ui.spacer(8);
    ui.titlebar_end();
}

// app_icon() -- 20 DIP rounded square in the accent colour with a white brush
fn app_icon(ui: &mut Ui) {
    ui.next_size(20, 20);
    ui.box_begin(ID_APP_ICON, BOX_H | BOX_FIXED, 0);
    let r = ui.box_rect();
    let c = ui.color(T_ACCENT);
    ui.draw_color(c);
    let rr = ui.dp(5);
    ui.draw_rrect(r[0], r[1], r[2], r[3], rr);
    let c = ui.color(T_ON_ACCENT);
    ui.draw_color(c);
    // 14 DIP glyph centred: (20 - 14) / 2 = 3 DIP
    let s = ui.dp(14) << 8;
    let o = ui.dp(3);
    ui.draw_icon(IC_BRUSH, r[0] + o, r[1] + o, s);
    ui.box_end();
}

// help_flyout() -- the Hilfe menu under the help button (CMD_HELP opens it)
fn help_flyout(ui: &mut Ui) {
    if ui.flyout_begin(ID_HELP_FLYOUT, FL_BELOW | FL_ALIGN_END) {
        for &id in HELP_ROWS.iter() {
            if id == 0 {
                ui.menu_sep();
            } else {
                ui.menu_cmd(id);
            }
        }
    }
    ui.flyout_end();
}

// ------------------------------------------------------------------ tool bar rows

/// chrome_toolbar() -- row 1: file, print, clipboard / selection, history, view toggles
pub(crate) fn chrome_toolbar(_p: &mut Pn, ui: &mut Ui) {
    ui.next_size(-1, ROW_H);
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_FIXED, 2);
    ui.spacer(6);
    ui.focus_group_begin(0); // the row is one Tab stop, the arrows move inside
    for &id in TOOLBAR_ROWS.iter() {
        if id == 0 {
            vsep(ui);
        } else {
            cmdbtn(ui, id);
        }
    }
    cmdbtn(ui, CMD_RULERS);
    ui.focus_group_end();
    ui.box_end();
}

/// chrome_toolrow() -- row 2: "Werkzeug:" with the tool menu, then the tool's options
pub(crate) fn chrome_toolrow(p: &mut Pn, ui: &mut Ui) {
    ui.next_size(-1, ROW_H);
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_FIXED, 6);
    ui.spacer(8);
    ui.text(TS_BODY, crate::preferences::localization::tr("Werkzeug:"));
    // drop-down button with the icon of the current tool
    let t = crate::tools::tool_cur(&p.tool);
    let ic = cmd_icon(ui, CMD_TOOL0 + t as u32);
    ui.next_icon(ic);
    ui.dropdown_button(ID_TOOL_CHOOSER, crate::preferences::localization::tr(""));
    let l = ui.cmd_label(CMD_TOOL0 + t as u32);
    ui.tooltip(&l);
    if ui.flyout_begin(ID_TOOL_CHOOSER, FL_BELOW) {
        for k in 0..TOOL_COUNT as u32 {
            ui.menu_cmd(CMD_TOOL0 + k);
        }
    }
    ui.flyout_end();
    vsep(ui);
    // the options scroll sideways when the row is too narrow (shapes / text at 1280 px): wheel
    // over the row, Tab focus, and the page buttons at its end (U3)
    ui.next_size(SIZE_FILL, ROW_H);
    ui.scroll_begin(ID_TOOLROW_SCROLL, SC_H | SC_NOBAR | BOX_XCENTER);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    crate::tools::tool_options(p, ui);
    ui.spacer(8);
    ui.box_end();
    ui.scroll_end();
    toolrow_pager(p, ui);
    ui.box_end();
    hline(ui);
}

// toolrow_pager() -- "<" / ">" page buttons after the options scroll view while its content
// is wider than the row (decided from the last measure; the buttons' own width is added back
// so the decision does not flip because of them)
fn toolrow_pager(p: &mut Pn, ui: &mut Ui) {
    let vw = ui.px2dip(ui.last_rect()[2]); // view width (DIP)
    let content = ui.scroll_extent(ID_TOOLROW_SCROLL).0; // content width (DIP)
    let limit = if p.chrome.trp_on { vw + TRP_W } else { vw };
    let on = content > limit;
    p.chrome.trp_on = on;
    if !on {
        return;
    }
    let pos = ui.scroll_pos(ID_TOOLROW_SCROLL).0;
    let maxp = (content - vw).max(0);
    let page = ((vw as u32 >> 1) + (vw as u32 >> 2)) as i32; // page = 3/4 of the view
    // back
    ui.next_flags(if pos > 0 { 0 } else { ST_DISABLED });
    ui.next_size(TRP_BTN, TRP_BTN);
    if ui.icon_button(ID_TOOLROW_BACK, IC_CHEV_LEFT, "Weitere Optionen links") & EV_ACT != 0 {
        ui.scroll_to(ID_TOOLROW_SCROLL, (pos - page).max(0), -1);
    }
    // forward (+ 1: DIP rounding of the px position)
    ui.next_flags(if pos + 1 < maxp { 0 } else { ST_DISABLED });
    ui.next_size(TRP_BTN, TRP_BTN);
    if ui.icon_button(ID_TOOLROW_FWD, IC_CHEV_RIGHT, "Weitere Optionen rechts") & EV_ACT != 0 {
        ui.scroll_to(ID_TOOLROW_SCROLL, (pos + page).min(maxp), -1);
    }
}

// ------------------------------------------------------------------ status bar

// unit_str(px) -- px in the current units ("800", "8,33", "21,17"; 96 dpi until the document
// has its own resolution)
fn unit_str(p: &Pn, px: i32) -> String {
    let u = view_units(&p.view);
    if u == UNIT_PX as u32 {
        return format!("{}", px);
    }
    // hundredths: px * 100 / dpi (Zoll) or px * 254 / dpi (Zentimeter), rounded; dpi = the
    // image resolution (view_dpi100, 96 dpi without one)
    let k = if u == UNIT_IN as u32 { 100 } else { 254 };
    let v = px.wrapping_mul(k);
    let neg = v < 0;
    let a = v.unsigned_abs();
    let dpi = view_dpi100(p) as u64;
    let h = ((a as u64 * 100 + (dpi >> 1)) / dpi) as u32;
    let (i, f) = (h / 100, h % 100);
    let s = if neg && h != 0 { format!("-{},{:02}", i as i32, f as i32) } else { format!("{},{:02}", i as i32, f as i32) };
    fmt_cap(s, 16)
}

// sb_field(icon, text, width) -- icon + caption text of a fixed width
fn sb_field(ui: &mut Ui, icon: u32, text: &str, width: i32) {
    ui.icon(icon, 16);
    ui.spacer(4);
    ui.next_size(width, 0);
    ui.text(TS_CAPTION, text);
}

// fmt_pair_x(a, b) -- "a × b"
fn fmt_pair_x(a: &str, b: &str) -> String {
    fmt_cap(format!("{} × {}", a, b), SBTXT_CAP)
}

/// chrome_statusbar() -- hint, selection / image size / cursor, units, zoom
pub(crate) fn chrome_statusbar(p: &mut Pn, ui: &mut Ui) {
    ui.statusbar_begin();
    // tool icon and help text (takes the free width)
    let t = crate::tools::tool_cur(&p.tool);
    let ic = cmd_icon(ui, CMD_TOOL0 + t as u32);
    ui.icon(ic, 16);
    ui.next_size(-1, 0);
    let hint = crate::tools::tool_hint(p);
    ui.text(TS_CAPTION | TX_ELLIPSIS, &hint);
    // selection size (only while there is a selection)
    let (w, h) = crate::document::selection::sel_size(p);
    if w != 0 {
        let s = fmt_pair_x(&unit_str(p, w), &unit_str(p, h));
        sb_field(ui, IC_SELECT_RECT, &s, 96);
    }
    // image size
    let (w, h) = doc_size(&p.doc, doc_cur(&p.doc));
    let s = fmt_pair_x(&unit_str(p, w), &unit_str(p, h));
    sb_field(ui, IC_IMAGE, &s, 96);
    // cursor position (empty while the pointer is not over the canvas)
    let (x, y, ok) = view_cursor(&p.view);
    let s = if ok { fmt_cap(format!("{}, {}", unit_str(p, x), unit_str(p, y)), SBTXT_CAP) } else { String::new() };
    sb_field(ui, IC_POINTER, &s, 96);
    // units (the combo box value follows the view only when the view changed, so a choice
    // waiting for its command is not undone by the next settle pass)
    let u = view_units(&p.view) as i32;
    let c = &mut p.chrome;
    if u != c.sb_ulast {
        c.sb_ulast = u;
        c.sb_units = u;
    }
    ui.next_size(0, 26);
    if ui.combobox(ID_SB_UNITS, &mut c.sb_units, &UNITS_LIST) & EV_ACT != 0 {
        ui.cmd_run(CMD_UNIT_SET, c.sb_units as u32);
    }
    // zoom: preset combo box (current value first when it is no preset), Ganzes Bild, slider
    let zoom = view_zoom(p);
    let (list, sel, first) = zoom_list(zoom);
    let c = &mut p.chrome;
    // combo box and slider follow the view only when the zoom changed (see units)
    if zoom as i32 != c.sb_zlast {
        c.sb_zlast = zoom as i32;
        c.sb_zsel = sel;
        c.sb_zslider = ZOOM_PRESETS_N - 1 - zoom_nearest(zoom);
    }
    ui.next_size(84, 26);
    let rows: Vec<&str> = list.iter().map(|s| s.as_str()).collect();
    if ui.combobox(ID_SB_ZOOM, &mut c.sb_zsel, &rows) & EV_ACT != 0 {
        let k = c.sb_zsel.wrapping_sub(first) as u32;
        if k < ZOOM_PRESETS_N as u32 {
            ui.cmd_run(CMD_ZOOM_SET, PN_ZOOM_PRESETS[k as usize]);
        }
    }
    ui.next_flags(ST_COMPACT);
    ui.cmd_button(CMD_ZOOM_WINDOW, BTN);
    // slider over the presets (left = small)
    ui.next_size(120, 0);
    if ui.slider(ID_SB_ZOOM_SLIDER, &mut c.sb_zslider, &RANGE_ZSLIDER) & EV_ACT != 0 {
        let k = (ZOOM_PRESETS_N - 1).wrapping_sub(c.sb_zslider) as u32;
        if k < ZOOM_PRESETS_N as u32 {
            ui.cmd_run(CMD_ZOOM_SET, PN_ZOOM_PRESETS[k as usize]);
        }
    }
    ui.statusbar_end();
}

// zoom_nearest(zoom) -> index of the largest preset <= zoom (last preset if none)
fn zoom_nearest(zoom: u32) -> i32 {
    let mut i = 0;
    while PN_ZOOM_PRESETS[i] > zoom {
        i += 1;
        if i >= ZOOM_PRESETS_N as usize - 1 {
            break;
        }
    }
    i as i32
}

// zoom_list() -> (rows "6400%" ... "1%" with the current zoom as an extra first row when it is
// no preset, the selected row, the index of the first preset row)
fn zoom_list(zoom: u32) -> (Vec<String>, i32, i32) {
    let mut v = Vec::with_capacity(ZOOM_PRESETS_N as usize + 1);
    let mut len = 0usize;
    let mut item = |z: u32, v: &mut Vec<String>| {
        // zoom_item: "N%" (skipped when the list buffer is full)
        if len >= ZOOMLIST_CAP - 16 {
            return;
        }
        let s = format!("{}%", (z / 100) as i32);
        len += s.len() + 1;
        v.push(s);
    };
    let (sel, first) = match PN_ZOOM_PRESETS.iter().position(|&z| z == zoom) {
        Some(i) => (i as i32, 0),
        None => {
            item(zoom, &mut v); // no: the current value first
            (0, 1)
        }
    };
    for &z in PN_ZOOM_PRESETS.iter() {
        item(z, &mut v);
    }
    (v, sel, first)
}
