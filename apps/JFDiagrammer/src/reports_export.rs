// reports_export -- (JD19, EX-16) Daten exportieren (Inventar): the shape data of the selected
// shapes, of the page shown or of every page as an Excel workbook or a CSV file, the columns
// chosen; Datei › Exportieren › Daten exportieren (and the command DATA_EXPORT)
//
//   ExportData                the dialog's choices
//   open(app, ui)             the dialog: which shapes, which columns (the shape's name, its
//                             master, its text, every shape data field of the drawing), the
//                             format; Exportieren... asks where
//   dialog(app, ui)           every pass
//
// The report engine does the work (diagram_core::report): a definition made on the spot -- the
// columns chosen, the shapes that have at least one of the chosen shape data fields (criteria
// "exists" joined by "or"), no summaries, numbers with two decimals and without units -- run and
// written by reports::write_file (XLSX: a title row and the headings bold and kept in view;
// CSV in the language's separators).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::report::{Builtin, Condition, Field, Filter, Property, ReportDef, Scope};
use ui::i18n::{t, tf};
use ui::*;

use crate::reports::{self, Out};
use crate::Diagrammer;

const ID: u32 = 0x7E30_0000;
const ID_COL: u32 = 0x7E31_0000;
const ID_SAVE: u32 = 0x7E32_0000;
const W: i32 = 460;

/// the dialog's state
pub(crate) struct ExportData {
    open: Shared<i32>,
    /// 0 the selected shapes, 1 the page shown, 2 every page
    scope: i32,
    /// the columns offered and taken
    cols: Vec<(String, Property)>,
    taken: Vec<i32>,
    /// 0 XLSX, 1 CSV
    format: i32,
    save: Shared<i32>,
    path: String,
    /// the last result: severity (0 information, 3 error), message
    pub status: Option<(u32, String)>,
}

impl Default for ExportData {
    fn default() -> ExportData {
        ExportData { open: Shared::new(0), scope: 1, cols: Vec::new(), taken: Vec::new(), format: 0, save: Shared::new(0), path: String::new(), status: None }
    }
}

/// Datei › Exportieren › Daten exportieren
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    let sel = !crate::edit_canvas::selected(app).is_empty();
    let Some(o) = app.opened.as_ref() else { return };
    let mut cols: Vec<(String, Property)> = [Builtin::Text, Builtin::Name, Builtin::Master]
        .into_iter()
        .map(|b| {
            let l: String = crate::reports_wizard::builtin_label(&b).chars().filter(|c| !matches!(c, '<' | '>')).collect();
            (l, Property::Builtin(b))
        })
        .collect();
    cols.extend(crate::reports_wizard::properties(&o.doc, false).into_iter().filter(|p| matches!(p.1, Property::Data(_))));
    let e = &mut app.reports.export;
    // the shape data fields are taken, of the built-in ones the text
    e.taken = cols.iter().map(|c| matches!(c.1, Property::Data(_) | Property::Builtin(Builtin::Text)) as i32).collect();
    e.cols = cols;
    e.scope = if sel { 0 } else { 1 };
    e.status = None;
    e.open.set(1);
    ui.repass();
}

// the definition of the export
fn definition(e: &ExportData) -> ReportDef {
    let mut d = ReportDef::new(t("Shape-Daten"));
    d.scope = [Scope::Selection, Scope::Page, Scope::AllPages][e.scope.clamp(0, 2) as usize];
    d.show_units = false;
    let mut id = 1;
    for (k, (label, p)) in e.cols.iter().enumerate() {
        if e.taken.get(k).copied().unwrap_or(0) == 0 {
            continue;
        }
        d.fields.push(Field::new(id, p.clone(), label));
        id += 1;
        if let Property::Data(_) = p {
            let mut f = Filter::new(d.filters.len() as u32 + 1, p.clone(), Condition::Exists, "");
            f.and = d.filters.is_empty();
            d.filters.push(f);
        }
    }
    d
}

/// every pass: the dialog, then the save dialog
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    // the save dialog also while closed (it forgets the last name it showed when it sees itself
    // closed), alone while open
    let saving = app.reports.export.save.get() != 0;
    save_dialog(app, ui);
    if saving {
        return;
    }
    let open = app.reports.export.open.clone();
    if open.get() == 0 {
        return;
    }
    if app.opened.is_none() {
        open.set(0);
        return;
    }
    let has_sel = !crate::edit_canvas::selected(app).is_empty();
    if !ui.dialog_begin(ID, crate::commands::label(crate::commands::DATA_EXPORT), &open) {
        return;
    }
    let e = &mut app.reports.export;
    ui.next_size(W, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, t("Welche Shapes?"));
    ui.next_flags(if has_sel { 0 } else { ST_DISABLED });
    ui.radio(ID + 1, t("Ausgewählte Shapes"), &mut e.scope, 0);
    ui.radio(ID + 2, t("Shapes auf der aktuellen Seite"), &mut e.scope, 1);
    ui.radio(ID + 3, t("Shapes auf allen Seiten"), &mut e.scope, 2);
    ui.spacer(4);
    ui.text(TS_BODY_STRONG, t("Spalten"));
    ui.next_size(W, 180);
    ui.scroll_begin(ID + 4, SC_V | BOX_V);
    for (k, (label, _)) in e.cols.iter().enumerate().take(0x8000) {
        if let Some(c) = e.taken.get_mut(k) {
            ui.checkbox(ID_COL + k as u32, label, c);
        }
    }
    ui.scroll_end();
    ui.spacer(4);
    ui.text(TS_BODY_STRONG, t("Dateiformat"));
    ui.radio(ID + 5, t("Excel-Arbeitsmappe (.xlsx)"), &mut e.format, 0);
    ui.radio(ID + 6, t("CSV-Datei (.csv)"), &mut e.format, 1);
    ui.next_size(W, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Eine Zeile je Shape, das mindestens eines der gewählten Shape-Daten-Felder hat."));
    if let Some((sev, msg)) = &e.status {
        ui.next_size(W, 0);
        ui.text(TS_BODY | TX_WRAP | if *sev == 3 { TX_ACCENT } else { TX_SECONDARY }, msg);
    }
    let any = e.taken.iter().any(|&c| c != 0);
    let r = ui.dialog_buttons(any.then(|| t("Exportieren...")), None, Some(t("Schließen")));
    ui.dialog_end();
    if r == DLG_NONE {
        return;
    }
    if r == DLG_PRIMARY {
        let dir = plat::arg_value("--export-dir").map(|d| format!("{}/", d.trim_end_matches(['/', '\\']))).unwrap_or_default();
        let stem = reports::file_stem(&format!("{} {}", app.drawing_name(), t("Shape-Daten")));
        let e = &mut app.reports.export;
        e.path = format!("{dir}{stem}.{}", if e.format == 1 { "csv" } else { "xlsx" });
        e.save.set(1);
        // the dialog shows again after the save dialog
        open.set(1);
    }
    ui.repass();
}

fn save_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let csv = app.reports.export.format == 1;
    // i18n
    let exts = [if csv { t("csv=CSV-Datei") } else { t("xlsx=Excel-Arbeitsmappe") }];
    let e = &mut app.reports.export;
    let dlg = e.save.clone();
    let was = dlg.get() != 0;
    if ui.filedlg(ID_SAVE, FD_SAVE, &dlg, &mut e.path, 4096, &exts) & EV_ACT != 0 {
        let path = ui.filedlg_paths().into_iter().next().unwrap_or_else(|| e.path.clone());
        let def = definition(&app.reports.export);
        let status = match reports::run_table(app, &def) {
            Some(mut table) => {
                table.title = tf("Shape-Daten: {}", &[&app.drawing_name()]);
                let out = if csv { Out::Csv } else { Out::Xlsx };
                match reports::write_file(&path, &table, out) {
                    Ok(m) => (0, m),
                    Err(m) => (3, m),
                }
            }
            None => (3, String::from(t("Es ist keine Zeichnung geöffnet."))),
        };
        ui.announce(&status.1, LIVE_POLITE);
        app.reports.export.status = Some(status);
    }
    if was && dlg.get() == 0 {
        ui.repass();
    }
}
