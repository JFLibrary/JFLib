// data_import -- (JD18, RD-02, AS-03, IM-05) Benutzerdefinierter Import: the data selector
// (source type, file, sheet or range, columns and rows, key) that adds a recordset
//
// STUB (skeleton): the import agent fills it in.

use ui::Ui;

use crate::Diagrammer;

/// the data selector's state
#[derive(Default)]
pub(crate) struct ImportState {}

/// Daten › Benutzerdefinierter Import
pub(crate) fn open(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// every pass: the data selector when open
pub(crate) fn dialog(_app: &mut Diagrammer, _ui: &mut Ui) {}
