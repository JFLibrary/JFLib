// panes -- the window's areas around the canvas: the task panes in the library's dock area
// (FEATURES.md 3.13, AB-01 .. AB-06), the page tabs under the canvas (ZF-14), the status bar
// (SB-01, SB-02, SB-04) and the context menus of shape, page, page tab and stencil (ZF-12,
// ZF-13, SL-08)
//
//   P_*                       the panels: Shapes (left), Shape formatieren, Größe und Position,
//                             Shape-Daten, Verschieben und Zoomen (right), (JD18) Externe
//                             Daten (under the canvas; all but Shapes hidden at first;
//                             Ansicht › Aufgabenbereiche shows them, the layout is kept in the
//                             settings)
//   panels() / content(app, ui, id)   the panels' descriptions and contents
//   center(app, ui)           the canvas and the page tabs under it; with an open drawing
//                             (JD4) the page of the tab drawn by viewer.rs, one tab per page
//                             (background pages after the others), Alle Seiten as a list
//   status(app, ui)           page, measures of the selection (a 1-D shape: length and angle,
//                             JD15 SB-02), zoom (the slider has 100 % in the middle, SB-04)
//
// The contents live in their own files: the Shapes panel (JD9, stencils_panel.rs), Shape
// formatieren (JD11, format_pane.rs), Größe und Position, Shape-Daten, Verschieben und Zoomen
// (JD8 rest: pane_sizepos.rs, pane_data.rs, pane_panzoom.rs).  The context menus list their
// commands (disabled while a command cannot run, commands.rs); the zoom value of the status bar
// opens the dialog Zoom.

use alloc::format;
use ui::i18n::{self, t, tf};
use ui::*;

use crate::commands::*;
use crate::icons::*;
use crate::ribbon::{SEL_CONTAINER, SEL_NONE, SEL_SHAPE, SEL_TEXT};
use crate::{commands, Diagrammer};

pub(crate) const P_SHAPES: u32 = 10;
pub(crate) const P_FORMAT: u32 = 11;
pub(crate) const P_SIZEPOS: u32 = 12;
pub(crate) const P_DATA: u32 = 13;
pub(crate) const P_PANZOOM: u32 = 14;
/// (JD18, AB-11) Externe Daten: under the canvas, as in Visio (data_pane.rs)
pub(crate) const P_EXTERNAL: u32 = 15;
/// (JD18, DA-05) Aktualisierungskonflikte: on the right, shown after a refresh with conflicts
/// (data_refresh.rs)
pub(crate) const P_CONFLICTS: u32 = 16;

pub(crate) const ID_DOCK: u32 = 500;
/// the canvas (JD8 rest: the task panes hand the keyboard back to it after Enter)
pub(crate) const ID_CANVAS: u32 = 300;
const ID_PAGES: u32 = 310;
const ID_PAGES_ALL: u32 = 0x5A00;
const ID_PANE: u32 = 0x5000;
const ID_STATUS: u32 = 0x5800;
const SIDEBAR_W: i32 = 240;

/// the panels (titles in the current language)
pub(crate) fn panels() -> [DockPanel<'static>; 7] {
    [
        DockPanel::new(P_SHAPES, t("Shapes"), JD_STENCIL_MENU).at(DOCK_LEFT, 0),
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_FORMAT, crate::tc("Shape formatieren|Bereich"), JD_FILL).at(DOCK_RIGHT, 0) },
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_SIZEPOS, t("Größe und Position"), JD_POSITION).at(DOCK_RIGHT, 0) },
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_DATA, t("Shape-Daten"), JD_SHAPE_DATA).at(DOCK_RIGHT, 0) },
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_PANZOOM, t("Verschieben und Zoomen"), JD_ZOOM).at(DOCK_RIGHT, 0) },
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_EXTERNAL, t("Externe Daten"), JD_EXTERNAL_DATA).at(DOCK_BOTTOM, 0) },
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_CONFLICTS, t("Aktualisierungskonflikte"), JD_REFRESH_ALL).at(DOCK_RIGHT, 0) },
    ]
}

/// the dock layout before the user changed it: the Shapes panel 240 DIP wide
pub(crate) fn initial() -> DockState {
    let mut d = DockState::new();
    d.set_size(DOCK_LEFT, SIDEBAR_W);
    d
}

/// a panel's content
pub(crate) fn content(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    match id {
        P_SHAPES => shapes(app, ui),
        P_FORMAT => format_pane(app, ui),
        P_SIZEPOS => size_pos(app, ui),
        P_DATA => shape_data(app, ui),
        P_PANZOOM => pan_zoom(app, ui),
        P_EXTERNAL => crate::data_pane::pane(app, ui),
        P_CONFLICTS => crate::data_refresh::pane(app, ui),
        _ => {}
    }
}

// ------------------------------------------------------------------ the context menus

// i18n
static SHAPE_MENU: [Mi; 23] = [
    // (JD15, SP-12) the rows of the shape's Actions section first, as in Visio (cmds_actions.rs)
    Mi::List(crate::cmds_actions::ACTION_ROWS, crate::cmds_actions::ACTION_ROWS_MAX, ""),
    Mi::Cmd(CUT), Mi::Cmd(COPY), Mi::Cmd(PASTE), Mi::Sep,
    Mi::Sub("Gruppierung", &[Mi::Cmd(GROUP), Mi::Cmd(UNGROUP)]),
    // (JD8 rest) ZF-12's Shape ▸
    Mi::Sub("Shape", &[Mi::Cmd(ROTATE_LEFT), Mi::Cmd(ROTATE_RIGHT), Mi::Cmd(FLIP_H), Mi::Cmd(FLIP_V), Mi::Sep, Mi::Cmd(PANE_SIZEPOS)]),
    Mi::Sub("Container", &[Mi::Cmd(CT_ADD), Mi::Cmd(CT_REMOVE), Mi::Sep, Mi::Cmd(CT_SELECT), Mi::Cmd(CT_LOCK), Mi::Cmd(CT_DISBAND)]),
    Mi::Cmd(CONNECT_SHAPES), Mi::Cmd(LAYOUT_BEHAVIOR),
    // (JD21 jumps) a connector's line jumps (hidden for other shapes)
    Mi::Cmd(CONN_BEHAVIOR),
    // (JD15, ST-05) a lane's commands (hidden for other shapes)
    Mi::Cmd(CFF_LANE_BEFORE), Mi::Cmd(CFF_LANE_AFTER),
    Mi::Sub("Anordnen", &[Mi::Cmd(BRING_FRONT), Mi::Cmd(BRING_FORWARD), Mi::Cmd(SEND_BACK), Mi::Cmd(SEND_BACKWARD)]),
    // (JD18) Verknüpfung mit Zeile aufheben (data_pane.rs)
    Mi::Sub("Daten", &[Mi::Cmd(PANE_DATA), Mi::Cmd(SHAPE_DATA_DEFINE), Mi::Sep, Mi::Cmd(DATA_UNLINK)]),
    // (JD8 rest) the shape's links follow its Hyperlink row (cmds_text::LINK_ROWS, ZF-12)
    Mi::Cmd(HYPERLINK), Mi::List(crate::cmds_text::LINK_ROWS, crate::cmds_text::LINK_ROWS_MAX, ""), Mi::Cmd(COMMENT_NEW),
    Mi::Sub("Formatieren", &[Mi::Cmd(FORMAT_TEXT), Mi::Cmd(FORMAT_LINE), Mi::Cmd(FORMAT_FILL), Mi::Cmd(FORMAT_SHADOW)]),
    Mi::Sub("Verbinderart", &[Mi::Cmd(CONN_RIGHT_ANGLE), Mi::Cmd(CONN_STRAIGHT), Mi::Cmd(CONN_CURVED), Mi::Sep, Mi::Cmd(CONN_RESET)]),
    Mi::Sep, Mi::Cmd(ADD_TO_MY_SHAPES),
    // (JD15, SP-01) Shape-Name
    Mi::Cmd(SHAPE_NAME),
];
// i18n
static PAGE_MENU: [Mi; 4] = [
    Mi::Cmd(PASTE),
    Mi::Sub("Zeichentools", &[Mi::Cmd(RECTANGLE), Mi::Cmd(ELLIPSE), Mi::Cmd(LINE_TOOL), Mi::Cmd(FREEFORM), Mi::Cmd(ARC), Mi::Cmd(PENCIL)]),
    Mi::Sep,
    Mi::Cmd(PAGE_SETUP),
];
static TAB_MENU: [Mi; 8] = [
    Mi::Cmd(PAGE_INSERT), Mi::Cmd(PAGE_DELETE), Mi::Cmd(PAGE_RENAME), Mi::Cmd(PAGE_DUPLICATE), Mi::Cmd(PAGE_REORDER), Mi::Sep,
    Mi::Cmd(PAGE_SETUP), Mi::Cmd(DRAWING_SCALE),
];

// ------------------------------------------------------------------ the canvas and its pages

/// the document area: the canvas, the page tabs under it
pub(crate) fn center(app: &mut Diagrammer, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    // (JD7) the canvas with the rulers and the edit layer (edit_canvas.rs); the shape's menu while
    // a shape is selected, else the page's (the edit layer selects on the right click first)
    crate::edit_canvas::canvas(app, ui, ID_CANVAS, |app, ui| {
        // (JD15, KB-04) not for a Ctrl + Shift right click (the magnifier) or right drag (a pan)
        if app.se.right_taken() {
            return;
        }
        let menu: &'static [Mi] = if matches!(app.selection_kind(), SEL_SHAPE | SEL_CONTAINER | SEL_TEXT) { &SHAPE_MENU } else { &PAGE_MENU };
        ui.context_menu(i18n::menu(menu));
    });
    if app.fit_again {
        app.fit_again = false;
        app.canvas.request_fit();
        ui.repass();
    }
    // the page tabs (ZF-14): the page, the plus button, all pages
    ui.next_size(-1, 26);
    ui.box_begin(0, BOX_H | BOX_LAYER, 0);
    let names = page_names(app);
    let names: alloc::vec::Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let before = app.page_tab;
    ui.next_size(-1, 26);
    let ev = ui.tabs_scrolling(ID_PAGES, &mut app.page_tab, &names);
    if app.page_tab != before {
        app.canvas.request_fit();
        app.fit_again = true;
    }
    // (JD8 rest: pages) a double click on a tab renames its page (ZF-14)
    if ev & EV_DOUBLE != 0 && commands::usable(app, PAGE_RENAME) {
        crate::pages::open_rename(app, ui);
    }
    // the page tab's menu: the menu key on the strip, a right click on a tab (the strip's
    // record is the last widget, its tabs lie above it)
    let strip = ui.last_rect();
    if ui.context_menu(i18n::menu(&TAB_MENU)) == 0 && ui.released() & (1 << BTN_RIGHT) != 0 && ui.popup_count() == 0 {
        let (mx, my) = ui.mouse();
        if mx >= strip[0] && mx < strip[0] + strip[2] && my >= strip[1] && my < strip[1] + strip[3] {
            ui.menu_popup(i18n::menu(&TAB_MENU));
        }
    }
    for id in [PAGE_INSERT, PAGES_ALL] {
        ui.next_size(28, 26);
        ui.cmd_button(id, ST_SUBTLE | ST_ICON_ONLY | ST_COMPACT);
        ui.tooltip(&commands::tip(ui, id));
    }
    // Alle Seiten: the pages as a list at its button
    if app.pages_menu {
        app.pages_menu = false;
        ui.popup_open(ID_PAGES_ALL);
    }
    if ui.flyout_begin(ID_PAGES_ALL, 0) {
        pages_list(app, ui, ID_PAGES_ALL + 1);
    }
    ui.flyout_end();
    ui.box_end();
    ui.box_end();
}

/// the page tabs' names: the open drawing's pages, else the empty drawing's one
pub(crate) fn page_names(app: &Diagrammer) -> alloc::vec::Vec<alloc::string::String> {
    match &app.opened {
        Some(o) if !o.tabs.is_empty() => o.tabs.iter().map(|t| t.name.clone()).collect(),
        Some(_) => alloc::vec::Vec::new(),
        None => alloc::vec![tf("Seite-{}", &["1"])],
    }
}

// the pages as menu items (the page shown checked), the background pages after a line; a click
// shows that page
fn pages_list(app: &mut Diagrammer, ui: &mut Ui, id0: u32) {
    let names = page_names(app);
    // (JD8 rest: pages) where the background pages begin
    let first_bg = app.opened.as_ref().and_then(|o| o.tabs.iter().position(|t| o.doc.page(t.page).is_some_and(|p| p.header.background)));
    for (k, n) in names.iter().enumerate() {
        if Some(k) == first_bg && k > 0 {
            ui.menu_sep();
        }
        ui.next_flags(if k == app.page_index() { ST_CHECKED } else { 0 });
        if ui.menu_item(id0 + k as u32, n) & EV_ACT != 0 && k != app.page_index() {
            app.page_tab = k as i32;
            app.canvas.request_fit();
            app.fit_again = true;
            ui.repass();
        }
    }
}

// ------------------------------------------------------------------ the panels

// Shapes: the stencils (JD9, stencils_panel.rs)
fn shapes(app: &mut Diagrammer, ui: &mut Ui) {
    crate::stencils_panel::panel(app, ui);
}

// Shape formatieren (AB-05): (JD11) format_pane.rs
fn format_pane(app: &mut Diagrammer, ui: &mut Ui) {
    crate::format_pane::pane(app, ui, ID_PANE);
}

// Größe und Position (AB-04): (JD8 rest) pane_sizepos.rs
fn size_pos(app: &mut Diagrammer, ui: &mut Ui) {
    crate::pane_sizepos::pane(app, ui, ID_PANE + 0x20);
}

// Shape-Daten (AB-03): (JD8 rest) pane_data.rs
fn shape_data(app: &mut Diagrammer, ui: &mut Ui) {
    crate::pane_data::pane(app, ui, ID_PANE + 0x30);
}

// Verschieben und Zoomen (AB-06): (JD8 rest) pane_panzoom.rs
fn pan_zoom(app: &mut Diagrammer, ui: &mut Ui) {
    crate::pane_panzoom::pane(app, ui, ID_PANE + 0x40);
}

// a zoom slider 10 .. 400 % (width -1: the rest of the row)
fn zoom_slider(app: &mut Diagrammer, ui: &mut Ui, id: u32, w: i32) {
    ui.next_size(w, 0);
    zoom_slider_here(app, ui, id);
}

/// (JD15, SB-04, AB-06) the zoom slider with 100 % in the middle: the left half runs from 10 to
/// 100 %, the right half from 100 to 400 % (Visio's); its value is the position 0 .. 1000 (500:
/// 100 %), the size comes from next_size
pub(crate) fn zoom_slider_here(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let z = app.canvas.vp.zoom * 100.0;
    let mut pos = if z <= 100.0 { (z - 10.0) / 90.0 * 500.0 } else { 500.0 + (z - 100.0) / 300.0 * 500.0 };
    pos = pos.clamp(0.0, 1000.0);
    let mut v = (pos + 0.5) as i32;
    if ui.slider(id, &mut v, &Range::new(0, 1000, 50, 0)) & (EV_ACT | EV_COMMIT) != 0 {
        let p = v.clamp(0, 1000) as f32;
        let z = if p <= 500.0 { 10.0 + p / 500.0 * 90.0 } else { 100.0 + (p - 500.0) / 500.0 * 300.0 };
        app.set_zoom((z + 0.5) as i32);
    }
    ui.tooltip(t("Zoom"));
}

// ------------------------------------------------------------------ the status bar

/// page, measures of the selection, zoom (SB-01, SB-02, SB-04)
pub(crate) fn status(app: &mut Diagrammer, ui: &mut Ui) {
    let zoom = (app.canvas.vp.zoom * 100.0 + 0.5) as i32;
    ui.statusbar_begin();
    let n = page_names(app).len();
    let page = tf("Seite {} von {}", &[&format!("{}", (app.page_index() + 1).min(n.max(1))), &format!("{}", n.max(1))]);
    ui.next_flags(ST_SUBTLE);
    if ui.label_button(ID_STATUS + 1, &page, TS_CAPTION, 0) & EV_ACT != 0 {
        ui.popup_open(ID_STATUS + 1);
    }
    if ui.flyout_begin(ID_STATUS + 1, 0) {
        pages_list(app, ui, ID_STATUS + 0x10);
    }
    ui.flyout_end();
    // the selection's measures (JD7: the single selection's); a click opens Größe und Position
    let unit = if crate::edit_canvas::metric(app) { "mm" } else { "in" };
    // (JD15, SB-02) a 1-D shape: its length (the Width cell) and angle
    let one_d = app.se.sel.len() == 1 && app.model.shapes.iter().any(|s| app.se.sel.contains(&s.key) && s.flags & SEF_1D != 0);
    // (a selected guide has no measures)
    let guide = app.model.guides.iter().any(|g| app.se.sel.contains(&g.key));
    let dims = match crate::edit_canvas::measures(app) {
        Some(_) if guide => tf("Breite: {}  Höhe: {}  Winkel: {}", &["–", "–", "–"]),
        Some(v) if one_d => tf("Länge: {}  Winkel: {}", &[&length(v[2], unit), &angle(v[4])]),
        Some(v) => tf("Breite: {}  Höhe: {}  Winkel: {}", &[&length(v[2], unit), &length(v[3], unit), &angle(v[4])]),
        None if app.selection_kind() == SEL_NONE || app.opened.is_some() => tf("Breite: {}  Höhe: {}  Winkel: {}", &["–", "–", "–"]),
        None => tf("Breite: {}  Höhe: {}  Winkel: {}", &["40 mm", "20 mm", "0°"]),
    };
    ui.next_flags(ST_SUBTLE);
    if ui.label_button(ID_STATUS + 3, &dims, TS_CAPTION, 0) & EV_ACT != 0 {
        app.panes.focus(P_SIZEPOS);
        ui.repass();
    }
    ui.tooltip(t("Größe und Position"));
    ui.spacer(-1);
    status_cmd(ui, ID_STATUS + 5, ZOOM_OUT);
    zoom_slider(app, ui, ID_STATUS + 4, 120);
    status_cmd(ui, ID_STATUS + 6, ZOOM_IN);
    // (JD8 rest, SB-04) the zoom value: a click opens the dialog Zoom
    ui.next_size(48, 0);
    ui.next_flags(ST_SUBTLE);
    if ui.label_button(ID_STATUS + 8, &tf("{} %", &[&format!("{}", zoom.max(1))]), TS_CAPTION, 0) & EV_ACT != 0 {
        crate::cmds_shell::zoom_open(app, ui);
    }
    ui.tooltip(&commands::tip(ui, ZOOM));
    status_cmd(ui, ID_STATUS + 7, FIT_WINDOW);
    ui.statusbar_end();
}

// a length in the page's unit: whole millimetres, inches with two decimals (the region's decimal
// sign)
fn length(v: f64, unit: &str) -> alloc::string::String {
    let s = if unit == "mm" { format!("{:.0}", v) } else { format!("{:.2}", v) };
    let s = if s == "-0" { alloc::string::String::from("0") } else { s };
    let dec = i18n::locale().decimal();
    format!("{} {}", s.replace('.', dec.encode_utf8(&mut [0; 4])), unit)
}

// an angle in whole degrees, -179 .. 180 as Visio shows the Angle cell
fn angle(v: f64) -> alloc::string::String {
    // no float remainder without libm: whole turns off by steps (the angle is small)
    let mut d = v;
    let mut n = 0;
    while d <= -180.0 && n < 64 {
        d += 360.0;
        n += 1;
    }
    while d > 180.0 && n < 64 {
        d -= 360.0;
        n += 1;
    }
    let r = if d >= 0.0 { (d + 0.5) as i64 } else { -((-d + 0.5) as i64) };
    format!("{}°", if r == -180 { 180 } else { r })
}

// an icon button of the status bar for a command (its own widget id: the ribbon may show the
// same command)
fn status_cmd(ui: &mut Ui, wid: u32, id: u32) {
    // (the zoom commands: always usable)
    ui.next_size(24, 22);
    ui.next_flags(ST_SUBTLE | ST_ICON_ONLY | ST_COMPACT | if commands::enabled(id) { 0 } else { ST_DISABLED });
    ui.next_icon(commands::def(id).map_or(0, |d| d.icon));
    if ui.button(wid, commands::label(id)) & EV_ACT != 0 {
        ui.cmd_run(id, 0);
    }
    ui.tooltip(&commands::tip(ui, id));
}
