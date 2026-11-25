// data_cmds -- (JD18) when the commands of the register Daten and of the pane Externe Daten can
// run (commands::usable asks), and the helpers they share
//
//   available(app, id)        false only for a data command that cannot run now: an import or
//                             a drawing from text needs an open drawing; refreshing, linking,
//                             the source's menu need a recordset; Verknüpfung aufheben a
//                             selected shape bound to a row; Konflikte some conflict
//   recordset_shown(app)      the recordset the pane Externe Daten shows (its tab), else the
//                             document's active one, else the first
//   now_text()                the time a refresh notes ("2026-11-23T14:05:00", the local
//                             clock)
//
// The functions behind the commands live in data_import.rs (Benutzerdefinierter Import, AS-03),
// data_pane.rs (the pane, AB-11), data_refresh.rs (aktualisieren, Intervall, Konflikte; DA-05),
// data_link.rs (Daten verknüpfen, AS-04), data_textdraw.rs (Zeichnung aus Text, IM-08/AS-20)
// and data_ask.rs (Shape-Daten beim Ablegen abfragen, SP-13); the model is
// diagram_core::data, the parts diagram_io::data, reading a source data_source.rs.

use alloc::format;
use alloc::string::String;

use crate::commands::*;
use crate::Diagrammer;

/// whether data command `id` can run now (other commands: true)
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    let doc = app.opened.as_ref().map(|o| &o.doc);
    let has_rs = doc.is_some_and(|d| !d.data.recordsets.is_empty());
    match id {
        DATA_CUSTOM_IMPORT | DATA_TEXT_DRAWING => doc.is_some(),
        DATA_REFRESH_ALL | DATA_REFRESH | DATA_LINK | DATA_SOURCE_REMOVE | DATA_SOURCE_RENAME | DATA_COLUMNS | DATA_SELECT_LINKED | DATA_REFRESH_SETTINGS => has_rs,
        DATA_UNLINK => has_rs && crate::data_pane::selection_linked(app),
        DATA_CONFLICTS => has_rs && crate::data_refresh::has_conflicts(app),
        _ => true,
    }
}

/// the recordset the pane shows: its tab, else the document's active one, else the first
pub(crate) fn recordset_shown(app: &Diagrammer) -> Option<u32> {
    let doc = &app.opened.as_ref()?.doc;
    let ids = || doc.data.recordsets.iter().map(|r| r.id);
    let tab = app.ext.tab.filter(|t| ids().any(|i| i == *t));
    tab.or(doc.data.active.filter(|a| ids().any(|i| i == *a))).or_else(|| ids().next())
}

/// the time a refresh notes, as Visio writes TimeRefreshed (local time, no zone)
pub(crate) fn now_text() -> String {
    let (days, _) = crate::cmds_text::local_now();
    let (y, m, d, h, mi, s, _) = diagram_core::field::civil(days);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}")
}
