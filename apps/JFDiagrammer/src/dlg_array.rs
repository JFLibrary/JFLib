// dlg_array -- (JD21, AS-11, RA-09) Ansicht › Add-Ons › Array: the selection copied in rows and
// columns (diagram_core::tools::array)
//
//   ArrayDialog               its state (kept for the session)
//   open(app, ui)             the dialog for the selection
//   dialog(app, ui)           every pass; OK makes the copies in one undo step and selects the
//                             selection with all its copies
//   available(app, id)        shapes are selected
//
// The selection stands in the first cell; its members and callouts come along (as Duplizieren
// takes them, structures::carried), copies join the containers they land in (structures::pasted).
// The spacing is in drawing units of the page (mm, US: inches), between the shapes or from middle
// to middle; the direction says where the rows and columns grow.

use alloc::vec::Vec;

use diagram_core::tools::array::{self, Grid};
use diagram_core::{Owner, ShapeRef};
use ui::i18n::t;
use ui::*;

use crate::commands::{self, *};
use crate::edit_canvas::{self, key_of};
use crate::{edit_doc, structures, Diagrammer};

const ID: u32 = 0x7EB8_0000;

// the directions in the list: (columns go left, rows go up)
// i18n
const DIRS: [(&str, bool, bool); 4] = [
    ("Nach rechts unten", false, false),
    ("Nach links unten", true, false),
    ("Nach rechts oben", false, true),
    ("Nach links oben", true, true),
];

/// the dialog's state
pub(crate) struct ArrayDialog {
    pub open: Shared<i32>,
    rows: i32,
    cols: i32,
    /// between columns, between rows (mm or inches as the page shows them)
    gap: [f64; 2],
    /// 0 between the shapes, 1 from middle to middle
    measure: i32,
    dir: i32,
    /// the gap's unit: inches (else mm)
    us: bool,
}

impl Default for ArrayDialog {
    fn default() -> ArrayDialog {
        ArrayDialog { open: Shared::new(0), rows: 2, cols: 3, gap: [10.0, 10.0], measure: 0, dir: 0, us: false }
    }
}

/// whether a command of this file can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    id != ARRAY_SHAPES || (app.opened.is_some() && !edit_canvas::selected(app).is_empty())
}

/// Array: the dialog
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    let us = !edit_canvas::metric(app);
    let d = &mut app.addons.array;
    if us != d.us {
        // the last values in the other unit
        let k = if us { 1.0 / 25.4 } else { 25.4 };
        d.gap = [d.gap[0] * k, d.gap[1] * k];
        d.us = us;
    }
    d.open.set(1);
    ui.repass();
}

fn row(ui: &mut Ui, s: &str) {
    ui.next_size(190, 0);
    ui.text(TS_BODY, s);
}

/// the dialog (every pass)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.addons.array.open.clone();
    if !ui.dialog_begin(ID, commands::label(ARRAY_SHAPES), &open) {
        return;
    }
    let d = &mut app.addons.array;
    let (unit, rf) = if d.us { ("in", RangeF::new(0.0, 100.0, 0.1, 2, 0)) } else { ("mm", RangeF::new(0.0, 2500.0, 1.0, 1, 0)) };
    ui.next_size(420, 0);
    ui.box_begin(ID + 1, BOX_V, 6);
    ui.text(TS_BODY_STRONG, t("Raster"));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Zeilen"));
    ui.next_size(90, 0);
    ui.number_box(ID + 2, &mut d.rows, &Range::new(1, 100, 1, 0));
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Spalten"));
    ui.next_size(90, 0);
    ui.number_box(ID + 3, &mut d.cols, &Range::new(1, 100, 1, 0));
    ui.box_end();
    ui.text(TS_BODY_STRONG, t("Abstand"));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, &ui::i18n::tf("Zwischen den Zeilen ({})", &[unit]));
    ui.next_size(90, 0);
    ui.number_box_f(ID + 4, &mut d.gap[1], &rf);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, &ui::i18n::tf("Zwischen den Spalten ({})", &[unit]));
    ui.next_size(90, 0);
    ui.number_box_f(ID + 5, &mut d.gap[0], &rf);
    ui.box_end();
    ui.box_begin(0, BOX_H, 16);
    ui.radio(ID + 6, t("Zwischen den Shapes"), &mut d.measure, 0);
    ui.radio(ID + 7, t("Von Mitte zu Mitte"), &mut d.measure, 1);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Richtung"));
    let dirs: Vec<&str> = DIRS.iter().map(|x| t(x.0)).collect();
    ui.next_size(200, 0);
    ui.combobox(ID + 8, &mut d.dir, &dirs);
    ui.box_end();
    let cells = d.rows.max(1) * d.cols.max(1);
    ui.next_size(420, 0);
    ui.text(TS_CAPTION | TX_SECONDARY, &ui::i18n::tf("Kopien: {}", &[&alloc::format!("{}", cells - 1)]));
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        apply(app, ui);
    }
    if r != DLG_NONE {
        app.addons.array.open.set(0);
        ui.repass();
    }
}

// OK: the copies (one undo step), the selection and its copies selected
fn apply(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(page) = app.model.page else { return };
    let owner = Owner::Page(page);
    let d = &app.addons.array;
    let k = if d.us { 1.0 } else { 1.0 / 25.4 };
    let (left, up) = (DIRS[d.dir.clamp(0, 3) as usize].1, DIRS[d.dir.clamp(0, 3) as usize].2);
    let g = Grid { rows: d.rows.max(1) as u32, cols: d.cols.max(1) as u32, gap: [d.gap[0] * k, d.gap[1] * k], centres: d.measure == 1, left, up };
    let sel = edit_canvas::selected(app);
    let Some(o) = app.opened.as_mut() else { return };
    let r = edit_doc::edit(o, commands::label(ARRAY_SHAPES), true, |tx, eng| {
        let shapes = structures::carried(tx.doc(), &sel);
        let made = array::make(tx, eng, owner, &shapes, &g)?;
        structures::pasted(tx, eng, owner, &made)?;
        Ok(made)
    });
    if let Ok(made) = r {
        let keys: Vec<u64> = sel.iter().chain(made.iter()).map(|&s: &ShapeRef| key_of(s)).collect();
        app.se.select(&keys);
    }
    ui.repass();
}
