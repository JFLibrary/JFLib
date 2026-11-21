// cmds_shell -- (JD8 rest) the commands of the shell that open something rather than change the
// drawing: a menu button's own command opens its menu in the ribbon (from a shortcut, the quick
// access toolbar, a key tip), Stärke / Striche / Pfeile open their choices at the Linie button,
// Schriftart and Schriftgrad take the keyboard, Schnellformatvorlagen and Linie / Füllung
// formatieren open Shape formatieren, Zoom its dialog (RA-05, SB-04)
//
//   open_menu(app, ui, id)        the menu of button `id` (ribbon::menu_place): its register comes
//                                 forward, the flyout opens at the button
//   available(app, id)            a menu button is usable while one of its rows is; the format
//                                 commands want a selection
//   font_focus(app, ui, id)       Schriftart / Schriftgrad: the box takes the keyboard
//   line_choice(app, ui, id)      Stärke, Striche, Pfeile: their flyout at Linie
//   quick_styles(app, ui)         Schnellformatvorlagen: the flyout in Shape formatieren
//   ShellState                    the dialog Zoom: open, the choice, the percentage
//   zoom_open(app, ui) / zoom_dialog(app, ui)   the dialog (the second every pass)
//
// The menus themselves are the ribbon's (ribbon.rs): a command here only brings them up where the
// user would have clicked, so the ribbon stays the one place that lists the rows.

use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::ribbon::{self, Reg};
use crate::Diagrammer;

const ID_ZOOM: u32 = 0x5C00_0000;

/// the zoom levels the dialog offers (Visio's list), then Ganze Seite, Seitenbreite, own value
const ZOOMS: [i32; 6] = [400, 200, 150, 100, 75, 50];
const Z_PAGE: i32 = 6;
const Z_WIDTH: i32 = 7;
const Z_OWN: i32 = 8;

/// the dialog Zoom
pub(crate) struct ShellState {
    pub zoom_dlg: Shared<i32>,
    zoom_pick: i32,
    zoom_pct: i32,
}

impl Default for ShellState {
    fn default() -> ShellState {
        ShellState { zoom_dlg: Shared::new(0), zoom_pick: 3, zoom_pct: 100 }
    }
}

/// the menu of the button whose command is `id`: its register comes forward (the File register
/// closes), the flyout opens at the button
pub(crate) fn open_menu(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let Some((reg, pid)) = ribbon::menu_place(id) else { return };
    if !ribbon::tabs(app).contains(&reg) {
        return;
    }
    app.backstage = None;
    app.reg = reg;
    ui.popup_open(pid);
    ui.repass();
}

/// whether a command of this file can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    let sel = app.opened.is_some() && !app.se.sel.is_empty();
    match id {
        FONT | FONT_SIZE => !crate::format_cells::selection(app).is_empty(),
        LINE_WEIGHT | LINE_DASH | LINE_ARROWS | QUICK_STYLES | FORMAT_LINE | FORMAT_FILL => sel,
        AUTO_ALIGN => crate::cmds_arrange::available(app),
        SHAPE_DATA_DEFINE => crate::pane_data::available(app),
        ADD_TO_MY_SHAPES => crate::cmds_myshapes::available(app),
        // a menu button: while a row of its menu leads somewhere
        _ => ribbon::menu_usable(app, id).unwrap_or(true),
    }
}

/// Schriftart, Schriftgrad: the box of the Start register (Text while text is edited) takes the
/// keyboard; Enter or Alt+Down opens its list
pub(crate) fn font_focus(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    app.backstage = None;
    if app.reg != Reg::Text || !ribbon::tabs(app).contains(&Reg::Text) {
        app.reg = Reg::Start;
    }
    ui.focus(id);
    ui.repass();
}

/// Stärke, Striche, Pfeile: their choices as a flyout at the Linie button
pub(crate) fn line_choice(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    app.backstage = None;
    app.reg = Reg::Start;
    crate::format_menus::open_line_choice(app, id);
    ui.repass();
}

/// Schnellformatvorlagen: Shape formatieren on Füllung und Linie with the quick styles open
pub(crate) fn quick_styles(app: &mut Diagrammer, ui: &mut Ui) {
    crate::format_menus::pane(app, 0);
    ui.popup_open(crate::format_pane::QUICK_POPUP);
    ui.repass();
}

// ---------------------------------------------------------------- Zoom

/// Ansicht › Zoom, the zoom value of the status bar: the dialog with the current zoom chosen
pub(crate) fn zoom_open(app: &mut Diagrammer, ui: &mut Ui) {
    let z = (app.canvas.vp.zoom * 100.0 + 0.5) as i32;
    let st = &mut app.shell;
    st.zoom_pct = z.clamp(10, 400);
    st.zoom_pick = ZOOMS.iter().position(|&p| p == z).map_or(Z_OWN, |k| k as i32);
    st.zoom_dlg.set(1);
    ui.repass();
}

/// every pass: the dialog Zoom
pub(crate) fn zoom_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.shell.zoom_dlg.clone();
    if !ui.dialog_begin(ID_ZOOM, t("Zoom"), &dlg) {
        return;
    }
    let st = &mut app.shell;
    ui.next_size(300, 0);
    ui.box_begin(ID_ZOOM + 1, BOX_V, 6);
    ui.text(TS_BODY_STRONG, t("Zoom auf"));
    for (k, p) in ZOOMS.iter().enumerate() {
        ui.radio(ID_ZOOM + 2 + k as u32, &alloc::format!("{p} %"), &mut st.zoom_pick, k as i32);
    }
    ui.radio(ID_ZOOM + 10, t("Ganze Seite"), &mut st.zoom_pick, Z_PAGE);
    ui.radio(ID_ZOOM + 11, crate::commands::label(PAGE_WIDTH), &mut st.zoom_pick, Z_WIDTH);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.radio(ID_ZOOM + 12, t("Prozent:"), &mut st.zoom_pick, Z_OWN);
    ui.next_size(90, 0);
    if ui.number_box(ID_ZOOM + 13, &mut st.zoom_pct, &Range::new(10, 400, 5, 0)) & (EV_ACT | EV_COMMIT) != 0 {
        st.zoom_pick = Z_OWN;
    }
    ui.box_end();
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        let (pick, pct) = (app.shell.zoom_pick, app.shell.zoom_pct);
        match pick {
            Z_PAGE => app.canvas.request_fit(),
            Z_WIDTH => crate::viewer::page_width(app),
            Z_OWN => app.set_zoom(pct.clamp(10, 400)),
            k => app.set_zoom(ZOOMS[k.clamp(0, ZOOMS.len() as i32 - 1) as usize]),
        }
    }
    if r != DLG_NONE {
        app.shell.zoom_dlg.set(0);
        ui.repass();
    }
}
