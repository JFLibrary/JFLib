// data_import -- (JD18, RD-02, AS-03, IM-05) Benutzerdefinierter Import: Visio's data selector
// (Datenauswahl) in four pages; it adds a recordset with its connection to the drawing
//
//   open(app, ui)           Daten › Benutzerdefinierter Import (also the menus of the pane Externe
//                           Daten): the dialog on its first page
//   dialog(app, ui)         every pass: the page shown, or its file dialog
//
// The pages (Visio's steps, the NetLab look):
//   1 Datenquelle           Microsoft Excel-Arbeitsmappe or Textdatei (CSV); the databases (SQL
//                           Server, Access, ODBC, OLE DB) are shown disabled: they come in P3
//                           (FEATURES F6)
//   2 the file              its path (typed, or Durchsuchen...: .xlsx/.xlsm, .csv/.txt); a
//                           workbook: a sheet ("Tabelle1$"), a named range, a table or an own
//                           range ("Tabelle1!A1:C9"); a text file: the delimiter and the encoding
//                           found (shown, not changeable: a refresh detects them again, so a
//                           choice here would not hold); Erste Datenzeile enthält
//                           Spaltenüberschriften
//   3 Spalten und Zeilen    a check box per column, the rows as a table with a check box each
//   4 Eindeutiger Bezeichner   one or more key columns (proposed: the first column whose values
//                           are all there and all different) or the order of the rows
// Fertig stellen: Recordset::from_table with the columns and rows taken, the connection Visio
// writes (data::excel_connection / text_connection; the Command on the connection and on the
// recordset), TimeRefreshed now, data::add_recordset as one undo step; the pane Externe Daten
// opens on its tab.
//
// Columns left out: the Command names the columns taken ("select `A`,`B` from `Tabelle1$`", as
// Visio writes it), so a refresh that reads through data_source::read_columns (data_refresh.rs) leaves them out
// (Visio's refresh too).  Rows left out are not in the recordset; a refresh reads the whole
// source again and brings them back as new rows (not linked); page 3 says so.
//
// Files: fs.user lets the run read the files the user chose in the file dialog; a path typed into
// the field is read only when the run may (else the page asks for Durchsuchen).  The file dialog
// is a dialog of its own: while it is open the data selector waits behind it (the library has no
// dialog in a dialog).  Test aid --data-dir DIR: the folder the file dialog starts in (scripts,
// relative to the working folder).

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use diagram_core::data::{self, Connection, DataValue, ExcelTable, Recordset};
use sheet_io::{csv, xlsx};
use ui::i18n::{t, tf};
use ui::*;

use crate::commands::{self, DATA_CUSTOM_IMPORT};
use crate::data_source::{self, Loaded, ReadError};
use crate::Diagrammer;

const ID: u32 = 0x7C60_0000;
// the column check boxes (page 3) ID + 0x1000 + c, the key check boxes (page 4) ID + 0x6000 + c,
// the row check boxes ID + 0x10000 + i (at most ROW_CHECKS of them)
const ID_COL: u32 = ID + 0x1000;
const ID_KEY: u32 = ID + 0x6000;
const ID_ROW: u32 = ID + 0x1_0000;
const ROW_CHECKS: usize = 0xE_0000;
/// the preview of the rows (its rows ID_GRID + 1 + i)
const ID_GRID: u32 = 0x6C60_0000;
/// the file dialog (its name box ID_FILE + 12)
const ID_FILE: u32 = 0x7C70_0000;
/// the width of the pages' content (DIP)
const W: i32 = 500;

const EXCEL: i32 = 0;
const TEXT: i32 = 1;

// a sheet, named range or table of a workbook as the second page offers it (None: an own range)
struct Choice {
    label: String,
    table: Option<ExcelTable>,
}

// the file read on the second page
enum Src {
    Excel { wb: xlsx::Workbook, choices: Vec<Choice> },
    Text { csv: csv::Csv },
}

/// the data selector's state
pub(crate) struct ImportState {
    pub open: Shared<i32>,
    page: u8,
    /// EXCEL or TEXT
    kind: i32,
    /// the file's path as typed or chosen
    path: String,
    file_dlg: Shared<i32>,
    file_path: String,
    /// the file read, and the path it was read from
    src: Option<Src>,
    src_path: String,
    /// the workbook's choice (index into the choices), the own range typed
    choice: i32,
    range: String,
    header: i32,
    /// the table read with these choices (pages 3 and 4), the workbook's table it came from
    table: Option<Loaded>,
    excel: Option<ExcelTable>,
    /// the check boxes: columns, rows, key columns; key by the rows' order (1) or by columns (0)
    cols: Vec<i32>,
    rows: Vec<i32>,
    keys: Vec<i32>,
    by_order: i32,
    proposed: Option<usize>,
    error: Option<String>,
    grid: GridState,
}

impl Default for ImportState {
    fn default() -> ImportState {
        ImportState {
            open: Shared::new(0),
            page: 0,
            kind: EXCEL,
            path: String::new(),
            file_dlg: Shared::new(0),
            file_path: String::new(),
            src: None,
            src_path: String::new(),
            choice: 0,
            range: String::new(),
            header: 1,
            table: None,
            excel: None,
            cols: Vec::new(),
            rows: Vec::new(),
            keys: Vec::new(),
            by_order: 0,
            proposed: None,
            error: None,
            grid: GridState::default(),
        }
    }
}

/// Daten › Benutzerdefinierter Import: the data selector on its first page (the source type and
/// the folder of the file dialog stay from the last time)
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    if app.opened.is_none() {
        return;
    }
    let d = &mut app.dimport;
    if d.file_path.is_empty() {
        if let Some(dir) = plat::arg_value("--data-dir") {
            let dir = if dir.starts_with('/') || dir.get(1..2) == Some(":") { dir } else { format!("{}/{}", plat::cwd(), dir) };
            d.file_path = format!("{}/", dir.trim_end_matches(['/', '\\']));
        }
    }
    let (kind, file_path) = (d.kind, core::mem::take(&mut d.file_path));
    *d = ImportState { kind, file_path, ..ImportState::default() };
    d.open.set(1);
    ui.repass();
}

/// every pass: the data selector while it is open (behind it its file dialog)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    if app.dimport.open.get() == 0 {
        return;
    }
    if app.dimport.file_dlg.get() != 0 {
        file_dialog(app, ui);
        return;
    }
    let open = app.dimport.open.clone();
    if !ui.dialog_begin(ID, t("Datenauswahl"), &open) {
        return;
    }
    let page = app.dimport.page;
    match page {
        0 => page_source(&mut app.dimport, ui),
        1 => page_file(&mut app.dimport, ui),
        2 => page_rows(&mut app.dimport, ui),
        _ => page_key(&mut app.dimport, ui),
    }
    if let Some(e) = &app.dimport.error {
        ui.next_size(W, 0);
        ui.text(TS_CAPTION | TX_WRAP | TX_ACCENT, e);
    }
    let last = page >= 3;
    let primary = if last { t("Fertig stellen") } else { t("Weiter >") };
    let r = ui.dialog_buttons(Some(primary), (page > 0).then(|| t("< Zurück")), Some(t("Abbrechen")));
    ui.dialog_end();
    let keep = match r {
        DLG_PRIMARY if last => !finish(app, ui),
        DLG_PRIMARY => {
            next(&mut app.dimport);
            true
        }
        DLG_SECONDARY => {
            let d = &mut app.dimport;
            d.error = None;
            d.page = d.page.saturating_sub(1);
            true
        }
        _ => false,
    };
    if r != DLG_NONE {
        app.dimport.open.set(keep as i32);
        if !keep {
            // nothing of the file stays in memory
            let d = &mut app.dimport;
            d.src = None;
            d.table = None;
        }
        ui.repass();
    }
}

// the file dialog (Durchsuchen...): a chosen file is read at once
fn file_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let d = &mut app.dimport;
    let dlg = d.file_dlg.clone();
    let exts = if d.kind == TEXT { [t("csv;txt=Textdatei"), t("=Alle Dateien")] } else { [t("xlsx;xlsm=Excel-Arbeitsmappe"), t("=Alle Dateien")] };
    if ui.filedlg(ID_FILE, FD_OPEN, &dlg, &mut d.file_path, 4096, &exts) & EV_ACT != 0 {
        d.path = ui.filedlg_paths().into_iter().next().unwrap_or_else(|| d.file_path.clone());
        load(d);
    }
    if dlg.get() == 0 {
        ui.repass(); // the data selector again
    }
}

// ---------------------------------------------------------------- the pages

// 1: what kind of data
fn page_source(d: &mut ImportState, ui: &mut Ui) {
    ui.next_size(W, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, t("Welche Daten möchten Sie importieren?"));
    let before = d.kind;
    ui.radio(ID + 1, t("Microsoft Excel-Arbeitsmappe"), &mut d.kind, EXCEL);
    ui.radio(ID + 2, t("Textdatei (CSV)"), &mut d.kind, TEXT);
    // P3 (F6): shown, not yet chosen
    let mut later = d.kind;
    ui.next_flags(ST_DISABLED);
    ui.radio(ID + 3, t("Microsoft SQL Server, Access, ODBC oder OLE DB"), &mut later, 2);
    ui.next_size(W, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Datenbanken folgen in einer späteren Version."));
    if d.kind != before {
        // another kind of file: the one read before does not fit
        d.src = None;
        d.src_path.clear();
        d.path.clear();
        d.error = None;
    }
}

// 2: the file and what of it
fn page_file(d: &mut ImportState, ui: &mut Ui) {
    let excel = d.kind == EXCEL;
    ui.next_size(W, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, if excel { t("Verbindung zu einer Microsoft Excel-Arbeitsmappe") } else { t("Verbindung zu einer Textdatei") });
    ui.text(TS_BODY, if excel { t("Arbeitsmappe") } else { t("Textdatei") });
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(W - 140, 0);
    ui.textbox(ID + 10, None, &mut d.path, 4096);
    ui.next_size(132, 0);
    if ui.button(ID + 11, t("Durchsuchen...")) & EV_ACT != 0 {
        if !d.path.trim().is_empty() {
            d.file_path = String::from(d.path.trim());
        }
        d.file_dlg.set(1);
        ui.repass();
    }
    ui.box_end();
    let fresh = d.src.is_some() && d.src_path == d.path.trim();
    match (&d.src, fresh) {
        (Some(Src::Excel { choices, .. }), true) => {
            ui.spacer(4);
            ui.next_size(W, 0);
            ui.text(TS_BODY | TX_WRAP, t("Welches Arbeitsblatt oder welchen Bereich möchten Sie verwenden?"));
            let labels: Vec<&str> = choices.iter().map(|c| c.label.as_str()).collect();
            let before = d.choice;
            ui.next_size(W, 0);
            ui.combobox(ID + 12, &mut d.choice, &labels);
            if d.choice != before {
                d.error = None;
            }
            if choices.get(d.choice.max(0) as usize).is_some_and(|c| c.table.is_none()) {
                ui.text(TS_CAPTION | TX_SECONDARY, t("Bereich, z. B. Tabelle1!A1:C9"));
                ui.next_size(W, 0);
                ui.textbox(ID + 13, None, &mut d.range, 256);
            }
        }
        (Some(Src::Text { csv }), true) => {
            ui.spacer(4);
            let delim = match csv.delimiter {
                b';' => t("Semikolon"),
                b',' => t("Komma"),
                b'\t' => t("Tabstopp"),
                b'|' => t("Senkrechter Strich"),
                _ => "?",
            };
            let enc = match csv.encoding {
                csv::Encoding::Utf8 => "UTF-8",
                csv::Encoding::Utf8Bom => t("UTF-8 mit BOM"),
                csv::Encoding::Utf16Le | csv::Encoding::Utf16Be => "UTF-16",
                csv::Encoding::Windows1252 => "Windows-1252",
            };
            ui.text(TS_BODY, &tf("Trennzeichen: {}", &[delim]));
            ui.text(TS_BODY, &tf("Codierung: {}", &[enc]));
            ui.next_size(W, 0);
            ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Trennzeichen und Codierung werden beim Lesen erkannt, auch beim Aktualisieren."));
        }
        _ => {}
    }
    ui.spacer(4);
    ui.checkbox(ID + 14, t("Erste Datenzeile enthält Spaltenüberschriften"), &mut d.header);
}

// 3: the columns and rows taken
fn page_rows(d: &mut ImportState, ui: &mut Ui) {
    let Some(l) = &d.table else { return };
    ui.next_size(W, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, t("Spalten und Zeilen auswählen"));
    ui.text(TS_BODY, t("Spalten"));
    // side by side, lines as needed; many columns scroll
    let many = l.headers.len() > 12;
    if many {
        ui.next_size(W, 96);
        ui.scroll_begin(ID + 0x20, SC_V | BOX_V);
    }
    ui.next_size(W - if many { 16 } else { 0 }, 0);
    ui.box_begin(ID + 0x23, BOX_H | BOX_WRAP, 12);
    for (c, h) in l.headers.iter().enumerate().take(0x5000) {
        ui.checkbox(ID_COL + c as u32, h, &mut d.cols[c]);
    }
    ui.box_end();
    if many {
        ui.scroll_end();
    }
    ui.text(TS_BODY, t("Zeilen"));
    // the preview: a check box per row, the columns taken
    let shown: Vec<usize> = (0..l.headers.len()).filter(|&c| d.cols.get(c).is_some_and(|&v| v != 0)).collect();
    // the columns share the width (the grid does not scroll sideways)
    let cw = ((W - 64 - 16) / shown.len().max(1) as i32).clamp(48, 110);
    let mut gcols = vec![GridCol::new("", 64).flags(GC_NOSORT)];
    gcols.extend(shown.iter().map(|&c| GridCol::new(l.headers[c].as_str(), cw).flags(GC_NOSORT)));
    let keys: Vec<u64> = (0..l.rows.len() as u64).collect();
    ui.next_size(W, 180);
    let (first, end) = ui.grid_begin(ID_GRID, &mut d.grid, &gcols, &keys);
    for i in first..end {
        let k = i as usize;
        ui.grid_row(&mut d.grid, i);
        if k < ROW_CHECKS {
            ui.grid_cell(&d.grid, 0);
            ui.checkbox(ID_ROW + k as u32, &format!("{}", k + 1), &mut d.rows[k]);
        }
        let row = &l.rows[k];
        for (j, &c) in shown.iter().enumerate() {
            ui.grid_text(&d.grid, j + 1, &row.get(c).map(DataValue::text).unwrap_or_default());
        }
    }
    ui.grid_end(&mut d.grid);
    ui.box_begin(0, BOX_H, 8);
    if ui.button(ID + 0x21, t("Alle Zeilen")) & EV_ACT != 0 {
        d.rows.iter_mut().for_each(|v| *v = 1);
        ui.repass();
    }
    if ui.button(ID + 0x22, t("Keine Zeile")) & EV_ACT != 0 {
        d.rows.iter_mut().for_each(|v| *v = 0);
        ui.repass();
    }
    ui.box_end();
    ui.next_size(W, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Ausgelassene Spalten bleiben draußen; ausgelassene Zeilen kommen beim Aktualisieren wieder dazu."));
}

// 4: what identifies a row
fn page_key(d: &mut ImportState, ui: &mut Ui) {
    let Some(l) = &d.table else { return };
    ui.next_size(W, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, t("Eindeutigen Bezeichner festlegen"));
    ui.next_size(W, 0);
    ui.text(TS_BODY | TX_WRAP, t("Mit einem eindeutigen Bezeichner erkennt das Aktualisieren die Zeilen wieder, auch wenn sich ihre Reihenfolge in der Quelle ändert."));
    ui.next_size(W, 0);
    ui.radio(ID + 0x30, t("Zeilen werden anhand der Werte dieser Spalten identifiziert:"), &mut d.by_order, 0);
    ui.next_size(W, 120);
    ui.scroll_begin(ID + 0x31, SC_V | BOX_V | pad(4));
    for c in (0..l.headers.len()).filter(|&c| d.cols[c] != 0).take(0x4000) {
        let label = if d.proposed == Some(c) { tf("{} (empfohlen)", &[&l.headers[c]]) } else { l.headers[c].clone() };
        ui.next_flags(if d.by_order != 0 { ST_DISABLED } else { 0 });
        ui.checkbox(ID_KEY + c as u32, &label, &mut d.keys[c]);
    }
    ui.scroll_end();
    ui.next_size(W, 0);
    ui.radio(ID + 0x32, t("Zeilen werden anhand ihrer Reihenfolge identifiziert"), &mut d.by_order, 1);
    if d.by_order == 0 {
        let key: Vec<usize> = (0..l.headers.len()).filter(|&c| d.cols[c] != 0 && d.keys[c] != 0).collect();
        if !key.is_empty() && !unique(l, &d.rows, &key) {
            ui.next_size(W, 0);
            ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Einige Zeilen haben in diesen Spalten dieselben Werte: sie werden beim Aktualisieren nicht sicher wiedererkannt."));
        }
    } else {
        ui.next_size(W, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Werden in der Quelle Zeilen eingefügt, gelöscht oder umsortiert, passen die Verknüpfungen danach nicht mehr."));
    }
}

// ---------------------------------------------------------------- going on

// Weiter >
fn next(d: &mut ImportState) {
    d.error = None;
    match d.page {
        0 => d.page = 1,
        1 => {
            // a path typed (or changed) is read first; the page shows what it holds
            if d.src.is_none() || d.src_path != d.path.trim() {
                load(d);
                return;
            }
            if read_table(d) {
                d.page = 2;
            }
        }
        2 => {
            let Some(l) = &d.table else { return };
            if !d.cols.iter().any(|&v| v != 0) {
                d.error = Some(String::from(t("Wählen Sie mindestens eine Spalte aus.")));
                return;
            }
            // the proposal: the first column taken whose values are all there and all different
            d.proposed = (0..l.headers.len()).filter(|&c| d.cols[c] != 0).find(|&c| unique(l, &d.rows, &[c]) && taken_rows(l, &d.rows).all(|r| !r.get(c).map(DataValue::text).unwrap_or_default().trim().is_empty()));
            d.keys = vec![0; l.headers.len()];
            if let Some(p) = d.proposed {
                d.keys[p] = 1;
            }
            d.by_order = d.proposed.is_none() as i32;
            d.page = 3;
        }
        _ => {}
    }
}

// the file of the path field read (its sheets and names, or its records)
fn load(d: &mut ImportState) {
    d.error = None;
    d.src = None;
    d.table = None;
    let path = String::from(d.path.trim());
    d.src_path = path.clone();
    if path.is_empty() {
        d.error = Some(String::from(t("Wählen Sie eine Datei aus.")));
        return;
    }
    let bytes = match data_source::read_bytes(&path) {
        Ok(b) => b,
        Err(e) => {
            d.error = Some(read_error(e));
            return;
        }
    };
    if d.kind == TEXT {
        match data_source::text(&bytes) {
            Ok(c) => d.src = Some(Src::Text { csv: c }),
            Err(e) => d.error = Some(read_error(e)),
        }
        return;
    }
    match data_source::workbook(&bytes) {
        Ok(wb) => {
            let mut choices = Vec::new();
            for s in wb.sheets().iter().filter(|s| s.kind == xlsx::SheetKind::Worksheet) {
                choices.push(Choice { label: format!("{}$", s.name), table: Some(ExcelTable::Sheet(s.name.clone())) });
            }
            for n in wb.names().iter().filter(|n| !n.builtin() && !n.hidden) {
                let Some(area) = n.area else { continue };
                // a table (ListObject) as its range: the provider reads ranges, not tables
                let table = if n.table {
                    let Some(sheet) = area.sheet.and_then(|i| wb.sheets().get(i)) else { continue };
                    ExcelTable::Range(sheet.name.clone(), area.a1())
                } else {
                    ExcelTable::Name(n.name.clone())
                };
                choices.push(Choice { label: n.name.clone(), table: Some(table) });
            }
            choices.push(Choice { label: String::from(t("Benutzerdefinierter Bereich")), table: None });
            d.choice = 0;
            d.src = Some(Src::Excel { wb, choices });
        }
        Err(e) => d.error = Some(read_error(e)),
    }
}

// what could not be read, as the page says it
fn read_error(e: ReadError) -> String {
    match e {
        ReadError::Denied => String::from(t("Diese Datei darf nicht gelesen werden: wählen Sie sie über „Durchsuchen...“ aus.")),
        ReadError::Missing => String::from(t("Die Datei ist nicht vorhanden.")),
        ReadError::Invalid(m) => tf("Die Datenquelle ist nicht lesbar: {}", &[&m]),
    }
}

// the table of the choices on page 2 -> read (else the error is set)
fn read_table(d: &mut ImportState) -> bool {
    let header = d.header != 0;
    let (loaded, excel) = match &d.src {
        Some(Src::Excel { wb, choices }) => {
            let Some(c) = choices.get(d.choice.max(0) as usize) else { return false };
            let table = match &c.table {
                Some(t) => t.clone(),
                None => match wb.area(d.range.trim()) {
                    Some(a) if a.sheet.is_some() => {
                        let sheet = a.sheet.and_then(|i| wb.sheets().get(i)).map(|s| s.name.clone()).unwrap_or_default();
                        ExcelTable::Range(sheet, a.a1())
                    }
                    _ => {
                        d.error = Some(String::from(t("Tragen Sie einen Bereich mit Blatt ein, z. B. Tabelle1!A1:C9.")));
                        return false;
                    }
                },
            };
            match data_source::excel_table(wb, &table, header) {
                Ok(tb) => (data_source::loaded(tb), Some(table)),
                Err(e) => {
                    d.error = Some(read_error(e));
                    return false;
                }
            }
        }
        Some(Src::Text { csv }) => (data_source::loaded(csv::to_table(csv, header, true)), None),
        None => return false,
    };
    if loaded.headers.is_empty() {
        d.error = Some(String::from(t("Hier stehen keine Daten.")));
        return false;
    }
    d.cols = vec![1; loaded.headers.len()];
    d.rows = vec![1; loaded.rows.len()];
    d.grid = GridState::default();
    d.table = Some(loaded);
    d.excel = excel;
    true
}

// the rows taken
fn taken_rows<'a>(l: &'a Loaded, rows: &'a [i32]) -> impl Iterator<Item = &'a Vec<DataValue>> + 'a {
    l.rows.iter().enumerate().filter(|(i, _)| rows.get(*i).is_none_or(|&v| v != 0)).map(|(_, r)| r)
}

// whether the rows taken differ in these columns
fn unique(l: &Loaded, rows: &[i32], cols: &[usize]) -> bool {
    let mut seen = BTreeSet::new();
    for r in taken_rows(l, rows) {
        let k: Vec<String> = cols.iter().map(|&c| r.get(c).map(DataValue::text).unwrap_or_default()).collect();
        if !seen.insert(k.join("\t")) {
            return false;
        }
    }
    true
}

// Fertig stellen: the recordset into the drawing -> done (the dialog closes)
fn finish(app: &mut Diagrammer, ui: &mut Ui) -> bool {
    let d = &mut app.dimport;
    d.error = None;
    let Some(l) = &d.table else { return false };
    let ci: Vec<usize> = (0..l.headers.len()).filter(|&c| d.cols[c] != 0).collect();
    let key: Vec<String> = if d.by_order != 0 { Vec::new() } else { ci.iter().filter(|&&c| d.keys.get(c).is_some_and(|&v| v != 0)).map(|&c| l.headers[c].clone()).collect() };
    if d.by_order == 0 && key.is_empty() {
        d.error = Some(String::from(t("Wählen Sie eine Spalte als Bezeichner oder die Reihenfolge der Zeilen.")));
        return false;
    }
    let headers: Vec<String> = ci.iter().map(|&c| l.headers[c].clone()).collect();
    let rows: Vec<Vec<DataValue>> = taken_rows(l, &d.rows).map(|r| ci.iter().map(|&c| r.get(c).cloned().unwrap_or_default()).collect()).collect();
    // the path as Windows writes it there (Visio refreshes through it)
    let path = if cfg!(windows) { d.src_path.replace('/', "\\") } else { d.src_path.clone() };
    let header = d.header != 0;
    let (cs, cmd, name) = match (&d.excel, d.kind) {
        (Some(table), EXCEL) => {
            let (cs, cmd) = data::excel_connection(&path, table, header);
            let name = match table {
                ExcelTable::Sheet(s) => s.clone(),
                ExcelTable::Range(..) => table.ole(),
                ExcelTable::Name(n) => n.clone(),
            };
            (cs, cmd, name)
        }
        _ => {
            let (cs, cmd) = data::text_connection(&path, header);
            let name = String::from(path.rsplit(['\\', '/']).next().unwrap_or(&path));
            (cs, cmd, name)
        }
    };
    let cmd = if ci.len() < l.headers.len() { data_source::command_with_columns(&cmd, &headers) } else { cmd };
    let mut rs = Recordset::from_table(&name, &headers, rows, &key);
    rs.command = Some(cmd.clone());
    rs.time_refreshed = Some(crate::data_cmds::now_text());
    let conn = Connection { connection_string: Some(cs), command: Some(cmd), ..Connection::default() };
    let Some(o) = app.opened.as_mut() else { return false };
    match crate::edit_doc::edit(o, commands::label(DATA_CUSTOM_IMPORT), true, |tx, _| data::add_recordset(tx, Some(conn), rs)) {
        Ok(id) => {
            app.ext.tab = Some(id);
            app.panes.show(crate::panes::P_EXTERNAL);
            let name = app.opened.as_ref().and_then(|o| o.doc.recordset(id)).map(|r| r.name.clone()).unwrap_or_default();
            ui.announce(&tf("Datenquelle „{}“ importiert", &[&name]), LIVE_POLITE);
            app.dimport.table = None;
            app.dimport.src = None;
            true
        }
        Err(e) => {
            app.dimport.error = Some(tf("Die Daten lassen sich nicht importieren: {}", &[&format!("{e:?}")]));
            false
        }
    }
}
