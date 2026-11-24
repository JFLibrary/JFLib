// reports -- (JD19) shape reports: Überprüfen › Shape-Berichte (RU-06), the dialog Berichte
// (DA-09), running a report (EX-12) and the table shape's Bericht aktualisieren (DA-13)
//
//   ReportsState              the dialog's list, choice and status, the files opened, the run
//   open(app, ui)             Überprüfen › Shape-Berichte: the dialog Berichte
//   dialog(app, ui)           every pass: Berichte, Bericht ausführen, the file dialogs, the
//                             wizard (reports_wizard.rs) and Daten exportieren (reports_export.rs)
//   entries(app) -> Vec<Entry>   what the list shows: the drawing's definitions (the document
//                             sheet, diagram_io::report), the predefined ones
//                             (reports_builtin.rs) and the .vrd files opened by Durchsuchen
//   run_table(app, def) -> Table   a definition run against the open drawing (the page shown,
//                             the selection; the words of the summary rows in the window's
//                             language)
//   write_file(path, &table, kind) -> Result<message, message>   EX-12: XLSX, CSV, HTML
//   refresh(app, ui)          the table shape's menu: Bericht aktualisieren, in place, one undo
//   menu_states(app, ui)      Bericht aktualisieren shows in the menu of a report table only
//   available(app, id)        when the commands can run
//
// The dialog as Visio's (A 185624): the list Bericht / Pfad, Neu..., Ändern..., Entfernen,
// Durchsuchen..., the description, „Nur zeichnungsspezifische Berichte anzeigen“ (a predefined
// report shows when a shape of the drawing passes its criteria; the drawing's and the opened
// files' always), Ausführen... and Schließen.  Ausführen asks for the output: Tabellen-Shape (the
// page shown, the layer „Bericht“, with a copy of the definition or a link to it -- "" the
// drawing's definition of that name, "jfd:<key>" a predefined one, else the .vrd file's path),
// Excel-Arbeitsmappe (.xlsx), CSV-Datei (.csv: ';' and decimal comma in German, Spanish, Italian
// and French, ',' and point in English; formulas guarded), Webseite (.html).  XML is not
// offered: Visio's XML report format is not documented, and a format of our own would not be
// what Visio users expect.  Entfernen removes a definition from the drawing (one undo step) or a
// file from the list (the file stays); a predefined report cannot be removed.  Test aids: the
// save dialogs start in --export-dir DIR, Durchsuchen in --data-dir DIR.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::report::{self, Labels, ReportDef, RowKind, Table, Val};
use diagram_io::report as rio;
use diagram_core::{Owner, SheetRef};
use sheet_io::{csv, xlsx, Value};
use ui::i18n::{t, tf};
use ui::*;

use crate::commands::{self, REPORTS, REPORT_REFRESH};
use crate::{edit_canvas, edit_doc, Diagrammer};

const ID: u32 = 0x7E00_0000;
const ID_GRID: u32 = 0x7E00_0100;
const ID_RUN: u32 = 0x7E01_0000;
const ID_BROWSE: u32 = 0x7E02_0000;
const ID_SAVE: u32 = 0x7E03_0000;
const W: i32 = 560;

/// where a definition comes from
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Src {
    /// a predefined one (its key)
    Builtin(&'static str),
    /// the drawing's (the document sheet's row, visPropRptN)
    Drawing(String),
    /// a .vrd file opened in this run (its path)
    File(String),
}

/// a row of the list
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub name: String,
    /// the column Pfad
    pub place: String,
    pub def: ReportDef,
    pub src: Src,
}

/// what a report is written as
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Out {
    Shape,
    Xlsx,
    Csv,
    Html,
}

impl Out {
    fn of(k: i32) -> Out {
        match k {
            1 => Out::Xlsx,
            2 => Out::Csv,
            3 => Out::Html,
            _ => Out::Shape,
        }
    }

    pub(crate) fn ext(self) -> &'static str {
        match self {
            Out::Xlsx => "xlsx",
            Out::Csv => "csv",
            Out::Html => "html",
            Out::Shape => "",
        }
    }
}

/// the dialog's state
pub(crate) struct ReportsState {
    pub open: Shared<i32>,
    grid: GridState,
    /// the chosen entry's name and source (the list is made anew every pass)
    pub pick: Option<(String, Src)>,
    only_drawing: i32,
    /// .vrd files opened by Durchsuchen (path, definition)
    pub files: Vec<(String, ReportDef)>,
    browse: Shared<i32>,
    browse_path: String,
    /// Bericht ausführen: its entry, the output, copy (0) or link (1)
    run_open: Shared<i32>,
    run: Option<Entry>,
    out: i32,
    link: i32,
    /// the save dialog of a file output, the table waiting for it
    save: Shared<i32>,
    save_path: String,
    pending: Option<(Table, Out)>,
    /// the last result: severity (0 information, 3 error), message
    pub status: Option<(u32, String)>,
    /// which predefined reports apply to the drawing: (serial, revision) -> keys
    specific: Option<((u32, u32), Vec<&'static str>)>,
    /// (reports_wizard.rs) Neu..., Ändern...
    pub wizard: crate::reports_wizard::Wizard,
    /// (reports_export.rs) Daten exportieren
    pub export: crate::reports_export::ExportData,
}

impl Default for ReportsState {
    fn default() -> ReportsState {
        ReportsState {
            open: Shared::new(0),
            grid: GridState::default(),
            pick: None,
            only_drawing: 1,
            files: Vec::new(),
            browse: Shared::new(0),
            browse_path: String::new(),
            run_open: Shared::new(0),
            run: None,
            out: 0,
            link: 0,
            save: Shared::new(0),
            save_path: String::new(),
            pending: None,
            status: None,
            specific: None,
            wizard: Default::default(),
            export: Default::default(),
        }
    }
}

/// the summary rows' words in the window's language
pub(crate) fn labels() -> Labels {
    let s = String::from;
    Labels { total: s(t("Summe")), average: s(t("Mittelwert")), count: s(t("Anzahl")), min: s(t("Minimum")), max: s(t("Maximum")), grand: s(t("Gesamt")), yes: s(t("WAHR")), no: s(t("FALSCH")) }
}

/// a definition run against the open drawing: the page shown, the selection
pub(crate) fn run_table(app: &Diagrammer, def: &ReportDef) -> Option<Table> {
    let o = app.opened.as_ref()?;
    let sel = edit_canvas::selected(app);
    let ctx = report::Ctx { page: app.model.page, selection: &sel, labels: labels() };
    Some(report::run(&o.doc, def, &ctx))
}

/// the definitions the list shows, by name
pub(crate) fn entries(app: &Diagrammer) -> Vec<Entry> {
    let mut v = Vec::new();
    let Some(o) = app.opened.as_ref() else { return v };
    let here = o.ed.path.clone().unwrap_or_else(|| String::from(t("In dieser Zeichnung")));
    for st in rio::stored(&o.doc) {
        if let Ok(def) = st.def {
            v.push(Entry { name: def.name.clone(), place: here.clone(), def, src: Src::Drawing(st.row) });
        }
    }
    let specific = app.reports.specific.as_ref().map(|s| &s.1);
    for (key, def) in crate::reports_builtin::all() {
        if app.reports.only_drawing != 0 && !specific.is_some_and(|s| s.contains(&key)) {
            continue;
        }
        v.push(Entry { name: def.name.clone(), place: String::from(t("Mitgeliefert (JFDiagrammer)")), def, src: Src::Builtin(key) });
    }
    for (path, def) in &app.reports.files {
        v.push(Entry { name: def.name.clone(), place: path.clone(), def: def.clone(), src: Src::File(path.clone()) });
    }
    v.sort_by_key(|e| e.name.to_lowercase());
    v
}

// which predefined reports have shapes in the drawing (made again when it changed)
fn update_specific(app: &mut Diagrammer) {
    let Some(o) = app.opened.as_ref() else { return };
    let key = (o.serial, o.ed.rev);
    if app.reports.specific.as_ref().is_some_and(|s| s.0 == key) {
        return;
    }
    let mut keys = Vec::new();
    for (k, mut def) in crate::reports_builtin::all() {
        def.scope = report::Scope::AllPages;
        let ctx = report::Ctx::default();
        if k == "inventory" || report::run(&o.doc, &def, &ctx).shapes > 0 {
            keys.push(k);
        }
    }
    app.reports.specific = Some((key, keys));
}

/// Überprüfen › Shape-Berichte
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    if app.opened.is_none() {
        return;
    }
    app.reports.status = None;
    app.reports.open.set(1);
    ui.repass();
}

/// whether report command `id` can run now (other commands: true)
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    match id {
        REPORTS | commands::DATA_EXPORT => app.opened.is_some(),
        REPORT_REFRESH => selected_table(app).is_some(),
        _ => true,
    }
}

// the report table the selection is (or is in)
fn selected_table(app: &Diagrammer) -> Option<report::ReportShape> {
    let o = app.opened.as_ref()?;
    let s = *edit_canvas::selected(app).first()?;
    report::report_shape(&o.doc, s)
}

/// every pass: Bericht aktualisieren only in a report table's menu
pub(crate) fn menu_states(app: &Diagrammer, ui: &mut Ui) {
    if selected_table(app).is_none() {
        ui.cmd_state(REPORT_REFRESH, CF_DISABLED | CF_HIDDEN);
    }
}

// the entry chosen in the list
fn chosen(app: &Diagrammer, list: &[Entry]) -> Option<usize> {
    let (name, src) = app.reports.pick.as_ref()?;
    list.iter().position(|e| &e.src == src && &e.name == name)
}

/// every pass: the dialogs
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    crate::reports_export::dialog(app, ui);
    if app.reports.wizard.is_open() {
        crate::reports_wizard::dialog(app, ui);
        return;
    }
    // the file dialogs also while closed (a file dialog forgets the last name it showed when it
    // sees itself closed), in front of the other dialogs while open
    let (browsing, saving) = (app.reports.browse.get() != 0, app.reports.save.get() != 0);
    browse_dialog(app, ui);
    save_dialog(app, ui);
    if browsing || saving {
        return;
    }
    if app.reports.run_open.get() != 0 {
        run_dialog(app, ui);
        return;
    }
    let open = app.reports.open.clone();
    if open.get() == 0 {
        return;
    }
    if app.opened.is_none() {
        open.set(0);
        return;
    }
    update_specific(app);
    let list = entries(app);
    if chosen(app, &list).is_none() {
        app.reports.pick = list.first().map(|e| (e.name.clone(), e.src.clone()));
    }
    let pick = chosen(app, &list);
    if !ui.dialog_begin(ID, t("Berichte"), &open) {
        return;
    }
    ui.next_size(W, 0);
    ui.box_begin(ID + 1, BOX_H, 12);
    // the list
    let cols = [GridCol::new(t("Bericht"), 190).flags(GC_NOSORT), GridCol::new(t("Pfad"), 200).flags(GC_NOSORT | GC_FILL)];
    let keys: Vec<u64> = (0..list.len() as u64).collect();
    let st = &mut app.reports;
    st.grid.select_only(pick.map(|k| k as u64));
    ui.next_size(W - 150, 230);
    let (a, b) = ui.grid_begin(ID_GRID, &mut st.grid, &cols, &keys);
    for i in a..b {
        let e = &list[i as usize];
        ui.grid_row(&mut st.grid, i);
        ui.grid_text(&st.grid, 0, &e.name);
        ui.grid_text(&st.grid, 1, &e.place);
    }
    ui.grid_end(&mut st.grid);
    if let Some(k) = st.grid.cur.map(|k| k as usize).filter(|&k| Some(k) != pick) {
        if let Some(e) = list.get(k) {
            st.pick = Some((e.name.clone(), e.src.clone()));
            ui.repass();
        }
    }
    let pick = chosen(app, &list);
    let entry = pick.and_then(|k| list.get(k));
    ui.box_begin(0, BOX_V, 8);
    let button = |ui: &mut Ui, id: u32, label: &str, on: bool| -> bool {
        ui.next_size(130, 0);
        ui.next_flags(if on { 0 } else { ST_DISABLED });
        ui.button(id, label) & EV_ACT != 0 && on
    };
    let mut action = 0;
    if button(ui, ID + 2, t("Neu..."), true) {
        action = 1;
    }
    if button(ui, ID + 3, t("Ändern..."), entry.is_some()) {
        action = 2;
    }
    let removable = entry.is_some_and(|e| !matches!(e.src, Src::Builtin(_)));
    if button(ui, ID + 4, t("Entfernen"), removable) {
        action = 3;
    }
    if button(ui, ID + 5, t("Durchsuchen..."), true) {
        action = 4;
    }
    ui.box_end();
    ui.box_end();
    ui.spacer(4);
    ui.text(TS_BODY_STRONG, t("Beschreibung"));
    ui.next_size(W, 40);
    ui.text(TS_BODY | TX_WRAP, entry.map_or("", |e| e.def.description.as_str()));
    let before = app.reports.only_drawing;
    ui.checkbox(ID + 6, t("Nur zeichnungsspezifische Berichte anzeigen"), &mut app.reports.only_drawing);
    if app.reports.only_drawing != before {
        ui.repass();
    }
    if let Some((sev, msg)) = &app.reports.status {
        ui.next_size(W, 0);
        ui.text(TS_BODY | TX_WRAP | if *sev == 3 { TX_ACCENT } else { TX_SECONDARY }, msg);
    }
    let r = ui.dialog_buttons(entry.is_some().then(|| t("Ausführen...")), None, Some(t("Schließen")));
    ui.dialog_end();
    let entry = entry.cloned();
    match action {
        1 => crate::reports_wizard::open_new(app, ui),
        2 => {
            if let Some(e) = &entry {
                crate::reports_wizard::open_edit(app, ui, e);
            }
        }
        3 => {
            if let Some(e) = &entry {
                remove(app, e);
            }
            ui.repass();
        }
        4 => {
            let dir = plat::arg_value("--data-dir").map(|d| format!("{}/", d.trim_end_matches(['/', '\\']))).unwrap_or_default();
            app.reports.browse_path = dir;
            app.reports.browse.set(1);
            ui.repass();
        }
        _ => {}
    }
    if r == DLG_PRIMARY {
        if let Some(e) = entry {
            app.reports.run = Some(e);
            app.reports.status = None;
            app.reports.run_open.set(1);
        }
        open.set(1);
        ui.repass();
    } else if r != DLG_NONE {
        open.set(0);
        ui.repass();
    }
}

// Entfernen: a definition of the drawing (one undo step) or an opened file from the list
fn remove(app: &mut Diagrammer, e: &Entry) {
    match &e.src {
        Src::Drawing(row) => {
            let Some(o) = app.opened.as_mut() else { return };
            let r = edit_doc::edit(o, t("Bericht entfernen"), true, |tx, _| rio::remove(tx, row));
            app.reports.status = Some(match r {
                Ok(()) => (0, tf("„{}“ wurde aus der Zeichnung entfernt.", &[&e.name])),
                Err(err) => (3, tf("„{}“ ließ sich nicht entfernen: {}", &[&e.name, &format!("{err:?}")])),
            });
        }
        Src::File(path) => {
            app.reports.files.retain(|f| &f.0 != path);
            app.reports.status = Some((0, tf("„{}“ wurde aus der Liste entfernt; die Datei bleibt.", &[&e.name])));
        }
        Src::Builtin(_) => {}
    }
    app.reports.pick = None;
}

/// a .vrd file's text (UTF-8, or UTF-16 with its BOM)
pub(crate) fn vrd_text(bytes: &[u8]) -> Option<String> {
    let utf16 = |le: bool| -> Option<String> {
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        String::from_utf16(&units).ok()
    };
    match bytes {
        [0xFF, 0xFE, ..] => utf16(true),
        [0xFE, 0xFF, ..] => utf16(false),
        _ => core::str::from_utf8(bytes).ok().map(String::from),
    }
}

/// a definition in a .vrd file -> it, or the message to show
pub(crate) fn read_vrd(path: &str) -> Result<ReportDef, String> {
    let name = String::from(plat::display_name(path));
    let bytes = crate::data_source::read_bytes(path).map_err(|_| tf("„{}“ ließ sich nicht lesen.", &[&name]))?;
    if bytes.len() > rio::MAX_BYTES {
        return Err(tf("„{}“ ist zu groß für eine Berichtsdefinition.", &[&name]));
    }
    let text = vrd_text(&bytes).ok_or_else(|| tf("„{}“ ist keine Berichtsdefinition.", &[&name]))?;
    rio::parse(&text).map_err(|e| tf("„{}“ ist keine Berichtsdefinition ({}).", &[&name, &format!("{e:?}")]))
}

// Durchsuchen...: a .vrd file into the list
fn browse_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &mut app.reports;
    let dlg = st.browse.clone();
    let was = dlg.get() != 0;
    let exts = [t("vrd=Berichtsdefinition"), t("=Alle Dateien")];
    if ui.filedlg(ID_BROWSE, FD_OPEN, &dlg, &mut st.browse_path, 4096, &exts) & EV_ACT != 0 {
        let path = ui.filedlg_paths().into_iter().next().unwrap_or_else(|| st.browse_path.clone());
        match read_vrd(&path) {
            Ok(def) => {
                st.files.retain(|f| f.0 != path);
                st.pick = Some((def.name.clone(), Src::File(path.clone())));
                st.files.push((path, def));
                st.status = None;
            }
            Err(m) => st.status = Some((3, m)),
        }
    }
    if was && dlg.get() == 0 {
        ui.repass();
    }
}

// ---------------------------------------------------------------- Ausführen

fn run_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.reports.run_open.clone();
    let Some(e) = app.reports.run.clone() else {
        open.set(0);
        return;
    };
    if !ui.dialog_begin(ID_RUN, t("Bericht ausführen"), &open) {
        return;
    }
    let st = &mut app.reports;
    ui.next_size(380, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, &tf("Bericht „{}“", &[&e.name]));
    ui.text(TS_BODY, t("Berichtsformat"));
    ui.radio(ID_RUN + 1, t("Tabellen-Shape"), &mut st.out, 0);
    ui.radio(ID_RUN + 2, t("Excel-Arbeitsmappe (.xlsx)"), &mut st.out, 1);
    ui.radio(ID_RUN + 3, t("CSV-Datei (.csv)"), &mut st.out, 2);
    ui.radio(ID_RUN + 4, t("Webseite (.html)"), &mut st.out, 3);
    if st.out == 0 {
        ui.spacer(4);
        ui.radio(ID_RUN + 5, t("Kopie der Berichtsdefinition"), &mut st.link, 0);
        ui.radio(ID_RUN + 6, t("Verknüpfung mit der Berichtsdefinition"), &mut st.link, 1);
        ui.next_size(380, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Die Tabelle kommt auf die Seite und die Ebene „Bericht“; ihr Kontextmenü aktualisiert sie."));
    }
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_NONE {
        return;
    }
    open.set(0);
    ui.repass();
    if r != DLG_PRIMARY {
        return;
    }
    let Some(table) = run_table(app, &e.def) else { return };
    let out = Out::of(app.reports.out);
    if out == Out::Shape {
        let link = match (&e.src, app.reports.link) {
            (_, 0) => None,
            (Src::Drawing(_), _) => Some(String::new()),
            (Src::Builtin(k), _) => Some(format!("jfd:{k}")),
            (Src::File(p), _) => Some(p.clone()),
        };
        let msg = put_on_page(app, ui, &e, &table, link);
        app.reports.status = Some(msg);
        if app.reports.status.as_ref().is_some_and(|s| s.0 == 0) {
            app.reports.open.set(0);
        }
        return;
    }
    let dir = plat::arg_value("--export-dir").map(|d| format!("{}/", d.trim_end_matches(['/', '\\']))).unwrap_or_default();
    app.reports.save_path = format!("{dir}{}.{}", file_stem(&e.name), out.ext());
    app.reports.pending = Some((table, out));
    app.reports.save.set(1);
}

/// a name as a file name: what a file name cannot hold becomes '_'
pub(crate) fn file_stem(name: &str) -> String {
    let s: String = name.chars().map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { '_' } else { c }).collect();
    let s = String::from(s.trim().trim_end_matches('.'));
    if s.is_empty() { String::from(t("Bericht")) } else { s }
}

// the save dialog of a file output; the file is written when it closes with a path
fn save_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &mut app.reports;
    let out = st.pending.as_ref().map_or(Out::Xlsx, |p| p.1);
    // i18n
    let exts = [match out {
        Out::Xlsx => t("xlsx=Excel-Arbeitsmappe"),
        Out::Csv => t("csv=CSV-Datei"),
        _ => t("html;htm=Webseite"),
    }];
    let dlg = st.save.clone();
    let was = dlg.get() != 0;
    let ev = ui.filedlg(ID_SAVE, FD_SAVE, &dlg, &mut st.save_path, 4096, &exts);
    if ev & EV_ACT != 0 {
        let path = st.save_path.clone();
        if let Some((table, out)) = st.pending.take() {
            st.status = Some(match write_file(&path, &table, out) {
                Ok(m) => (0, m),
                Err(m) => (3, m),
            });
        }
    }
    if was && dlg.get() == 0 {
        // closed (without a file too): nothing waits any more
        app.reports.pending = None;
        ui.repass();
    }
}

// ---------------------------------------------------------------- the outputs

/// a table cell as a spreadsheet value
pub(crate) fn value(v: &Val) -> Value {
    match v {
        Val::Empty => Value::Empty,
        Val::Num(x) => Value::Number(*x),
        Val::Text(s) => Value::Text(s.clone()),
        Val::Bool(b) => Value::Bool(*b),
        Val::Date(d) => Value::Date(*d),
    }
}

/// the CSV options of the window's language: ';' and the decimal comma in German, Spanish,
/// Italian and French, else ',' and the point; formulas guarded (the file opens in Excel)
pub(crate) fn csv_options() -> csv::WriteOptions {
    let mut o = if ui::i18n::language() == 1 { csv::WriteOptions::DEFAULT } else { csv::WriteOptions::GERMAN };
    o.guard_formulas = true;
    o
}

/// a sheet's name from a report's: at most 31 characters, none of [ ] : * ? / \
pub(crate) fn sheet_name(name: &str) -> String {
    let s: String = name.chars().filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\') && !c.is_control()).take(31).collect();
    let s = String::from(s.trim().trim_matches('\''));
    if s.is_empty() { String::from(t("Bericht")) } else { s }
}

// the rows of a table for a spreadsheet: the headings, then the rows
fn sheet_rows(table: &Table) -> Vec<Vec<Value>> {
    let mut rows: Vec<Vec<Value>> = Vec::with_capacity(table.rows.len() + 1);
    rows.push(table.columns.iter().map(|c| Value::Text(c.name.clone())).collect());
    for r in &table.rows {
        rows.push(r.cells.iter().map(|c| value(&c.value)).collect());
    }
    rows
}

fn html_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// a table as a web page (a heading, the table; groups and totals bold)
pub(crate) fn html(table: &Table) -> String {
    let lang = ["de", "en", "es", "it", "fr"].get(ui::i18n::language() as usize).copied().unwrap_or("de");
    let mut o = format!(
        "<!DOCTYPE html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n<style>\nbody {{ font-family: sans-serif; margin: 2em; color: #1d2228; }}\ntable {{ border-collapse: collapse; }}\nth, td {{ border: 1px solid #a3a8af; padding: 0.25em 0.6em; text-align: left; }}\nth {{ background: #e3e6ea; }}\ntd.num {{ text-align: right; }}\ntr.group td, tr.summary td, tr.total td {{ font-weight: bold; }}\ntr.summary td {{ background: #f1f2f4; }}\ntr.total td {{ background: #e3e6ea; }}\n</style>\n</head>\n<body>\n<h1>{}</h1>\n<table>\n<thead><tr>",
        html_esc(&table.title),
        html_esc(&table.title)
    );
    for c in &table.columns {
        o.push_str(&format!("<th>{}</th>", html_esc(&c.name)));
    }
    o.push_str("</tr></thead>\n<tbody>\n");
    for r in &table.rows {
        let class = match r.kind {
            RowKind::Item => "",
            RowKind::Group => " class=\"group\"",
            RowKind::Summary(_) => " class=\"summary\"",
            RowKind::Grand(_) => " class=\"total\"",
        };
        o.push_str(&format!("<tr{class}>"));
        for (k, c) in r.cells.iter().enumerate() {
            let num = table.columns.get(k).is_some_and(|col| col.numeric) || matches!(c.value, Val::Num(_));
            o.push_str(&format!("<td{}>{}</td>", if num { " class=\"num\"" } else { "" }, html_esc(&c.text)));
        }
        o.push_str("</tr>\n");
    }
    o.push_str("</tbody>\n</table>\n</body>\n</html>\n");
    o
}

/// write a table into a file: Ok(what was written) or the error to show
pub(crate) fn write_file(path: &str, table: &Table, out: Out) -> Result<String, String> {
    let name = String::from(plat::display_name(path));
    let bytes = match out {
        Out::Xlsx => {
            let mut rows = alloc::vec![alloc::vec![Value::Text(table.title.clone())]];
            rows.extend(sheet_rows(table));
            let sheet = xlsx::SheetOut { name: sheet_name(&table.title), rows, header_rows: 2, widths: Vec::new(), autofilter: false };
            xlsx::write(&xlsx::BookOut { sheets: alloc::vec![sheet] }, &xlsx::WriteOptions::DEFAULT).map_err(|e| tf("„{}“ ließ sich nicht schreiben: {}", &[&name, &e.text()]))?
        }
        Out::Csv => csv::write(&sheet_rows(table), &csv_options()).map_err(|e| tf("„{}“ ließ sich nicht schreiben: {}", &[&name, &e.text()]))?,
        _ => html(table).into_bytes(),
    };
    if !plat::write_file(path, &bytes) {
        return Err(tf("„{}“ ließ sich nicht schreiben.", &[&name]));
    }
    Ok(tf("Gespeichert: {} (Zeilen: {})", &[&name, &format!("{}", table.rows.len())]))
}

// the table on the page shown, on the layer „Bericht“, in the middle of the page; one undo step
fn put_on_page(app: &mut Diagrammer, ui: &mut Ui, e: &Entry, table: &Table, link: Option<String>) -> (u32, String) {
    let Some(page) = app.model.page else { return (3, String::from(t("Es ist keine Seite zu sehen."))) };
    let Some(o) = app.opened.as_mut() else { return (3, String::from(t("Es ist keine Zeichnung geöffnet."))) };
    let def_copy = link.is_none().then(|| rio::to_xml(&e.def));
    let owner = Owner::Page(page);
    let r = edit_doc::edit(o, t("Bericht ausführen"), true, |tx, eng| {
        let layer = report::report_layer(tx, page, t("Bericht"), "Report")?;
        let p = report::Place { name: e.name.clone(), def_copy, link, layers: alloc::vec![layer] };
        let g = report::place(tx, eng, owner, [0.0, 0.0], table, &p)?;
        // into the middle of the page
        let doc = tx.doc();
        let num = |c: &str| doc.sym(c).and_then(|s| doc.resolve(SheetRef::Page(page), &diagram_core::CellAddr::single(s)).num()).unwrap_or(0.0);
        let (pw, ph) = (diagram_core::dock::reach(doc, owner, num("PageWidth")), diagram_core::dock::reach(doc, owner, num("PageHeight")));
        if let Some(b) = diagram_core::ops::bounds(doc, &[g]) {
            let d = [(pw - (b[2] - b[0])) * 0.5 - b[0], (ph + (b[3] - b[1])) * 0.5 - b[3]];
            diagram_core::ops::move_shapes(tx, eng, &[g], d)?;
        }
        Ok(g)
    });
    match r {
        Ok(g) => {
            app.se.select(&[edit_canvas::key_of(g)]);
            let msg = tf("Der Bericht „{}“ steht als Tabelle auf der Seite (Zeilen: {}).", &[&e.name, &format!("{}", table.rows.len())]);
            ui.announce(&msg, LIVE_POLITE);
            (0, msg)
        }
        Err(err) => (3, tf("Der Bericht ließ sich nicht auf die Seite stellen: {}", &[&format!("{err:?}")])),
    }
}

/// the definition behind a report table: its copy, else its link
fn definition_of(app: &Diagrammer, rs: &report::ReportShape) -> Result<ReportDef, String> {
    if let Some(xml) = &rs.def {
        return rio::parse(xml).map_err(|e| tf("Die Kopie der Berichtsdefinition ist beschädigt ({}).", &[&format!("{e:?}")]));
    }
    let link = rs.link.as_deref().unwrap_or("");
    if let Some(key) = link.strip_prefix("jfd:") {
        return crate::reports_builtin::by_key(key).ok_or_else(|| tf("Den mitgelieferten Bericht „{}“ gibt es nicht.", &[key]));
    }
    if link.is_empty() {
        let o = app.opened.as_ref().ok_or_else(String::new)?;
        return rio::find(&o.doc, &rs.name).and_then(|s| s.def.ok()).ok_or_else(|| tf("Die Zeichnung hat keine Berichtsdefinition „{}“ mehr.", &[&rs.name]));
    }
    read_vrd(link)
}

/// the table shape's menu: Bericht aktualisieren -- the definition run again, the table built
/// anew where it is (one undo step)
pub(crate) fn refresh(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(rs) = selected_table(app) else { return };
    let def = match definition_of(app, &rs) {
        Ok(d) => d,
        Err(m) => {
            if let Some(o) = app.opened.as_mut() {
                o.notice = Some((String::from(t("Bericht aktualisieren")), m));
            }
            ui.repass();
            return;
        }
    };
    // the selection is the table itself: a report on the selection takes no shape of it
    let Some(table) = run_table(app, &def) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let r = edit_doc::edit(o, commands::label(REPORT_REFRESH), true, |tx, eng| report::refresh(tx, eng, rs.shape, &table));
    if let Ok(g) = r {
        app.se.select(&[edit_canvas::key_of(g)]);
        ui.announce(&tf("Bericht „{}“ aktualisiert", &[&rs.name]), LIVE_POLITE);
    }
    ui.repass();
}
