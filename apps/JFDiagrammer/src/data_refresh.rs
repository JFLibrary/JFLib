// data_refresh -- (JD18, RD-03, DA-05) Alle aktualisieren, Daten aktualisieren (per source),
// the refresh interval, the conflicts after a refresh and their resolution
//
// STUB (skeleton): the refresh and link agent fills it in.

use ui::Ui;

use crate::Diagrammer;

/// the refresh state: the dialogs, the interval clock, the conflicts of the last refresh
#[derive(Default)]
pub(crate) struct RefreshState {}

/// Alle aktualisieren
pub(crate) fn refresh_all(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// Daten aktualisieren... (the dialog with every source)
pub(crate) fn open(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// Aktualisierung konfigurieren... (the source shown: interval, overwrite user values)
pub(crate) fn settings_open(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// Aktualisierungskonflikte
pub(crate) fn conflicts_open(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// whether there are conflicts to show
pub(crate) fn has_conflicts(_app: &Diagrammer) -> bool {
    false
}

/// every pass: the dialogs, the interval refresh
pub(crate) fn dialog(app: &mut Diagrammer, _ui: &mut Ui) {
    // STUB: keeps build_rs's -D dead-code quiet until the real file comes
    let _ = &app.drefresh;
    let _ = crate::data_cmds::now_text;
    let _ = crate::data_cmds::recordset_shown;
    match crate::data_source::read(&diagram_core::data::Source::Other) {
        Ok(l) => drop((l.headers, l.rows)),
        Err(crate::data_source::ReadError::Invalid(m)) => drop(m),
        Err(_) => {}
    }
}
