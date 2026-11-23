// data_pane -- (JD18, AB-11) the pane Externe Daten: a tab per recordset, its rows with a
// link mark, the menu of the pane and of a tab, dragging rows onto shapes or the empty page
//
// STUB (skeleton): the import agent fills it in.

use ui::Ui;

use crate::Diagrammer;

/// the pane's state
#[derive(Default)]
pub(crate) struct ExtState {
    /// the recordset whose tab is shown (data_cmds::recordset_shown)
    pub tab: Option<u32>,
}

/// the pane's content (panes::content)
pub(crate) fn pane(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// Datenquelle entfernen (the recordset shown)
pub(crate) fn remove_source(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// Datenquelle umbenennen
pub(crate) fn rename_source(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// Spalteneinstellungen
pub(crate) fn columns_open(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// Verknüpfte Shapes auswählen (of the rows selected in the pane)
pub(crate) fn select_linked(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// Verknüpfung mit Zeile aufheben (the selected shapes)
pub(crate) fn unlink(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// whether a selected shape is bound to a row
pub(crate) fn selection_linked(_app: &Diagrammer) -> bool {
    false
}

/// every pass: the pane's dialogs (rename, column settings)
pub(crate) fn dialog(_app: &mut Diagrammer, _ui: &mut Ui) {}
