// dlg_snap -- (JD15) the dialog Ausrichten und Kleben (RA-07, part of DM-03): Ansicht › Visuelle
// Unterstützung's dialog launcher and Alt+F9 open it
//
//   SnapDialog                the dialog's state: open, the choices while it is open
//   open(app, ui)             SNAP_GLUE: the dialog with the switches as they are now
//   dialog(app, ui)           every pass: OK applies the choices, Abbrechen leaves them
//
// The switches (Visio's dialog, register Allgemein):
//   Derzeit aktiv: Ausrichten (app.snap: snapping to the grid and to guides at all), Kleben
//     (app.se.glue: ends glue to connection points and shapes), Dynamisches Gitter
//     (app.se.dyn_grid, also Ansicht › Dynamisches Gitter)
//   Ausrichten an: Raster (app.snap_grid), Führungslinien (app.se.snap_guides)
//   Anzeigen: Verbindungspunkte anzeigen (app.se.show_points, also Ansicht › Verbindungspunkte)
// They hold for the window, not the drawing: Visio keeps them in the document (DocumentSettings
// Snap and Glue), here they stay as long as the app runs (DM-03, JD22).  Visio's further targets
// (ruler subdivisions, alignment box, shape geometry, handles, vertices, intersections), the
// glue targets and the snap strength are not offered yet (JD22).  The page's InhibitSnap turns
// snapping off whatever the dialog says (edit_canvas.rs).

use ui::i18n::t;
use ui::*;

use crate::{tc, Diagrammer};

const ID: u32 = 0x7A50_0000;

/// the dialog's state
pub(crate) struct SnapDialog {
    pub dlg: Shared<i32>,
    // Ausrichten, Kleben, Dynamisches Gitter, Raster, Führungslinien, Verbindungspunkte anzeigen
    v: [i32; 6],
}

impl Default for SnapDialog {
    fn default() -> SnapDialog {
        SnapDialog { dlg: Shared::new(0), v: [1; 6] }
    }
}

/// Ausrichten und Kleben (SNAP_GLUE, Alt+F9, the launcher of Visuelle Unterstützung)
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    let se = &app.se;
    app.snap_dlg.v = [app.snap, se.glue, se.dyn_grid, app.snap_grid, se.snap_guides, se.show_points].map(|b| b as i32);
    app.snap_dlg.dlg.set(1);
    ui.repass();
}

/// every pass: the dialog while it is open
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.snap_dlg.dlg.clone();
    if !ui.dialog_begin(ID, t("Ausrichten und Kleben"), &dlg) {
        return;
    }
    let v = &mut app.snap_dlg.v;
    ui.next_size(340, 0);
    ui.box_begin(ID + 1, BOX_V, 6);
    ui.text(TS_BODY_STRONG, t("Derzeit aktiv"));
    ui.checkbox(ID + 2, tc("Ausrichten|Einrasten"), &mut v[0]);
    ui.checkbox(ID + 3, t("Kleben"), &mut v[1]);
    ui.checkbox(ID + 4, t("Dynamisches Gitter"), &mut v[2]);
    ui.separator();
    ui.text(TS_BODY_STRONG, t("Ausrichten an"));
    // the targets count while Ausrichten is on
    let off = if v[0] == 0 { ST_DISABLED } else { 0 };
    ui.next_flags(off);
    ui.checkbox(ID + 5, t("Raster"), &mut v[3]);
    ui.next_flags(off);
    ui.checkbox(ID + 6, t("Führungslinien"), &mut v[4]);
    ui.separator();
    ui.text(TS_BODY_STRONG, t("Anzeigen"));
    ui.checkbox(ID + 7, t("Verbindungspunkte anzeigen"), &mut v[5]);
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        let v = app.snap_dlg.v.map(|x| x != 0);
        app.snap = v[0];
        app.se.glue = v[1];
        app.se.dyn_grid = v[2];
        app.snap_grid = v[3];
        app.se.snap_guides = v[4];
        app.se.show_points = v[5];
    }
    if r != DLG_NONE {
        app.snap_dlg.dlg.set(0);
        ui.repass();
    }
}
