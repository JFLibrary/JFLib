// panes -- the window's areas around the canvas: the task panes in the library's dock area
// (FEATURES.md 3.13, AB-01 .. AB-06), the page tabs under the canvas (ZF-14), the status bar
// (SB-01, SB-02, SB-04) and the context menus of shape, page, page tab and stencil (ZF-12,
// ZF-13, SL-08)
//
//   P_*                       the panels: Shapes (left), Shape formatieren, Größe und Position,
//                             Shape-Daten, Verschieben und Zoomen (right, hidden at first;
//                             Ansicht › Aufgabenbereiche shows them, the layout is kept in the
//                             settings)
//   panels() / content(app, ui, id)   the panels' descriptions and contents
//   center(app, ui)           the canvas and the page tabs under it; with an open drawing
//                             (JD4) the page of the tab drawn by viewer.rs, one tab per page
//                             (background pages after the others), Alle Seiten as a list
//   status(app, ui)           page, measures of the selection, zoom
//
// Contents are placeholders until their stages: the Shapes panel shows the stencils (JD9,
// stencils_panel.rs), the format and data panes their layout with disabled fields (JD7, JD11),
// Verschieben und Zoomen the overview of the canvas and a working zoom slider.  The context
// menus list their commands (disabled until a handler exists, commands.rs).

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

pub(crate) const ID_DOCK: u32 = 500;
const ID_CANVAS: u32 = 300;
const ID_PAGES: u32 = 310;
const ID_PAGES_ALL: u32 = 0x5A00;
const ID_PANE: u32 = 0x5000;
const ID_STATUS: u32 = 0x5800;
const SIDEBAR_W: i32 = 240;

/// the panels (titles in the current language)
pub(crate) fn panels() -> [DockPanel<'static>; 5] {
    [
        DockPanel::new(P_SHAPES, t("Shapes"), JD_STENCIL_MENU).at(DOCK_LEFT, 0),
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_FORMAT, crate::tc("Shape formatieren|Bereich"), JD_FILL).at(DOCK_RIGHT, 0) },
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_SIZEPOS, t("Größe und Position"), JD_POSITION).at(DOCK_RIGHT, 0) },
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_DATA, t("Shape-Daten"), JD_SHAPE_DATA).at(DOCK_RIGHT, 0) },
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_PANZOOM, t("Verschieben und Zoomen"), JD_ZOOM).at(DOCK_RIGHT, 0) },
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
        _ => {}
    }
}

// ------------------------------------------------------------------ the context menus

// i18n
static SHAPE_MENU: [Mi; 16] = [
    Mi::Cmd(CUT), Mi::Cmd(COPY), Mi::Cmd(PASTE), Mi::Sep,
    Mi::Sub("Gruppierung", &[Mi::Cmd(GROUP), Mi::Cmd(UNGROUP)]),
    Mi::Sub("Container", &[Mi::Cmd(CT_ADD), Mi::Cmd(CT_REMOVE), Mi::Sep, Mi::Cmd(CT_SELECT), Mi::Cmd(CT_LOCK), Mi::Cmd(CT_DISBAND)]),
    Mi::Cmd(CONNECT_SHAPES), Mi::Cmd(LAYOUT_BEHAVIOR),
    Mi::Sub("Anordnen", &[Mi::Cmd(BRING_FRONT), Mi::Cmd(BRING_FORWARD), Mi::Cmd(SEND_BACK), Mi::Cmd(SEND_BACKWARD)]),
    Mi::Sub("Daten", &[Mi::Cmd(PANE_DATA), Mi::Cmd(SHAPE_DATA_DEFINE)]),
    Mi::Cmd(HYPERLINK), Mi::Cmd(COMMENT_NEW),
    Mi::Sub("Formatieren", &[Mi::Cmd(FORMAT_TEXT), Mi::Cmd(FORMAT_LINE), Mi::Cmd(FORMAT_FILL), Mi::Cmd(FORMAT_SHADOW)]),
    Mi::Sub("Verbinderart", &[Mi::Cmd(CONN_RIGHT_ANGLE), Mi::Cmd(CONN_STRAIGHT), Mi::Cmd(CONN_CURVED), Mi::Sep, Mi::Cmd(CONN_RESET)]),
    Mi::Sep, Mi::Cmd(ADD_TO_MY_SHAPES),
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
    ui.tabs_scrolling(ID_PAGES, &mut app.page_tab, &names);
    if app.page_tab != before {
        app.canvas.request_fit();
        app.fit_again = true;
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

// the pages as menu items (the page shown checked); a click shows that page
fn pages_list(app: &mut Diagrammer, ui: &mut Ui, id0: u32) {
    let names = page_names(app);
    for (k, n) in names.iter().enumerate() {
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

// Größe und Position (AB-04): the fields of a 2D shape, disabled until a shape is selected
fn size_pos(app: &mut Diagrammer, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_PANE + 0x20, SC_V | BOX_V | pad(10));
    // (JD7) the measures of the single selection
    let m = crate::edit_canvas::measures(app);
    let sel = m.is_some();
    let unit = if crate::edit_canvas::metric(app) { "mm" } else { "in" };
    let v = m.unwrap_or([0.0; 6]);
    let rows = [
        ("X", length(v[0], unit)),
        ("Y", length(v[1], unit)),
        (t("Breite"), length(v[2], unit)),
        (t("Höhe"), length(v[3], unit)),
        (t("Winkel"), angle(v[4])),
        (t("Drehpunkt"), alloc::string::String::from(if v[5] != 0.0 { t("Mitte") } else { t("Benutzerdefiniert") })),
    ];
    for (k, (label, v)) in rows.iter().enumerate() {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(90, 0);
        ui.text(TS_BODY, label);
        let mut s = if sel { v.clone() } else { alloc::string::String::new() };
        ui.next_size(-1, 0);
        ui.next_flags(ST_DISABLED);
        ui.textbox(ID_PANE + 0x21 + k as u32, None, &mut s, 32);
        ui.box_end();
    }
    if !sel {
        ui.spacer(6);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Kein Shape ausgewählt."));
    }
    ui.scroll_end();
}

// Shape-Daten (AB-03): the empty state until shapes carry data
fn shape_data(_app: &mut Diagrammer, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(ID_PANE + 0x30, BOX_V | pad(10), 0);
    ui.empty_state(JD_SHAPE_DATA, t("Keine Shape-Daten"), t("Wählen Sie ein Shape aus, um seine Daten zu sehen."));
    ui.box_end();
}

// Verschieben und Zoomen (AB-06): the canvas' overview and a zoom slider
fn pan_zoom(app: &mut Diagrammer, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(ID_PANE + 0x40, BOX_V | pad(10), 8);
    let model = DModel { nodes: &[], ports: &[], edges: &[], wps: &[], ver: 0 };
    ui.next_size(-1, 160);
    ui.diagram_minimap(ID_PANE + 0x41, &mut app.canvas, &model);
    zoom_slider(app, ui, ID_PANE + 0x42, -1);
    ui.box_end();
}

// a zoom slider 10 .. 400 % (width -1: the rest of the row)
fn zoom_slider(app: &mut Diagrammer, ui: &mut Ui, id: u32, w: i32) {
    let mut z = (app.canvas.vp.zoom * 100.0 + 0.5) as i32;
    ui.next_size(w, 0);
    if ui.slider(id, &mut z, &Range::new(10, 400, 10, 0)) & (EV_ACT | EV_COMMIT) != 0 {
        app.set_zoom(z);
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
    let dims = match crate::edit_canvas::measures(app) {
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
    ui.next_size(48, 0);
    ui.text(TS_CAPTION | TX_CENTER, &tf("{} %", &[&format!("{}", zoom.max(1))]));
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
