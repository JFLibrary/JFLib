// data_ask -- (JD18, SP-13) Shape-Daten beim Ablegen abfragen: fields marked "ask on drop"
// (Verify) open a dialog after a shape is dropped or duplicated
//
// STUB (skeleton): the refresh and link agent fills it in.

use diagram_core::ShapeRef;
use ui::Ui;

use crate::Diagrammer;

/// the question's state
#[derive(Default)]
pub(crate) struct AskState {}

/// after shapes were dropped or duplicated: ask for their fields marked "ask on drop"
#[allow(dead_code)]
pub(crate) fn after_drop(_app: &mut Diagrammer, _ui: &mut Ui, _shapes: &[ShapeRef]) {}

/// every pass: the dialog when open
pub(crate) fn dialog(app: &mut Diagrammer, _ui: &mut Ui) {
    let _ = &app.dask; // STUB: keeps build_rs's -D dead-code quiet until the real file comes
}
