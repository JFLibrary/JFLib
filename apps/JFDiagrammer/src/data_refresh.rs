// data_refresh -- (JD18, RD-03, DA-05) Alle aktualisieren, Daten aktualisieren (per source),
// Aktualisierung konfigurieren (interval, overwrite, another file), the refresh interval, the
// conflicts after a refresh and their resolution (the pane Aktualisierungskonflikte), Nach
// Aktualisierung automatisch verknüpfen
//
//   RefreshState              the dialogs, the sources waiting, the conflicts of the last
//                             refreshes, the interval clock
//   refresh_all(app, ui)      every source that can be refreshed (not switched off by Options,
//                             an Excel workbook or a text file), one after another
//   open(app, ui)             Daten aktualisieren...: every source with its last refresh;
//                             Aktualisieren (the one chosen), Alle aktualisieren, Konfigurieren...
//   settings_open(app, ui)    Aktualisierung konfigurieren... for the source shown: the interval
//                             in minutes, overwriting values the user changed, another file
//   conflicts_open(app, ui)   the pane Aktualisierungskonflikte (panes::P_CONFLICTS)
//   pane(app, ui)             its content: rows gone (Shape löschen, Verknüpfung aufheben,
//                             Behalten), values the user changed (Datenwert übernehmen, Eigenen
//                             Wert behalten), keys found twice; a click on the shape selects it
//   dialog(app, ui)           every pass: the dialogs, a refresh the interval asks for
//   auto_link_on / toggle_auto_link   Nach Aktualisierung automatisch verknüpfen (a setting of
//                             the program, on at first as in Visio)
//
// A source is read through data_source::read.  A file this run may not read (fs.user grants
// only the files the user chose in this run: after the drawing is opened again) or that is not
// there is asked for: the file dialog starts at the connection's path, the chosen file goes into
// the connection (diagram_core::data::excel_connection / text_connection, same table and header
// row) in the same undo step as the refresh.  The refresh (diagram_core::data::refresh) is one
// undo step "Daten aktualisieren" with the recalculation of the cells it wrote; after it, with
// the setting on, the recordset's comparisons (data_link::comparisons) link shapes of every page
// that have no row of it yet.  Rows gone stay in the recordset as RefreshConflict (also from a
// file); the values the user changed and the keys found twice are kept for the pane until they
// are resolved or the drawing closes.
//
// The interval: a recordset with RefreshInterval runs every so many minutes (the virtual clock
// in headless runs, a ui timer wakes the window), never while a dialog, a menu, a gesture or a
// text edit is open, at most one source per pass, without asking for files: a source this run
// cannot read waits until the user refreshes it.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::data::{self, Conflict, Connection, Recordset, RowLink, Source};
use diagram_core::{Document, Owner, ShapeRef, SheetRef};
use ui::i18n::{t, tf};
use ui::*;

use crate::commands::{self, DATA_REFRESH, DATA_REFRESH_SETTINGS};
use crate::data_source::{self, Loaded, ReadError};
use crate::{edit_doc, Diagrammer};

const ID: u32 = 0x7C30_0000;
const ID_ASK: u32 = 0x7C38_0000;
const ID_SET: u32 = 0x7C40_0000;
const ID_FILE: u32 = 0x7C48_0000;
const ID_PANE: u32 = 0x7C50_0000;
/// the interval's ui timer
const TIMER: u32 = 0x7C30_0001;

// the pane's buttons: a row gone, a value the user changed, a key found twice
// i18n
static GONE: [&str; 3] = ["Shape löschen", "Verknüpfung aufheben", "Behalten"];
// i18n
static USER: [&str; 2] = ["Datenwert übernehmen", "Eigenen Wert behalten"];
// i18n
static DUP: [&str; 1] = ["Ausblenden"];

// what the file dialog is for
#[derive(Clone, Copy)]
enum Pick {
    /// the file of a source that could not be read (the refresh goes on with it)
    Locate(u32),
    /// Aktualisierung konfigurieren's Datei ändern
    Settings,
}

// a conflict kept for the pane (the rows gone are the recordsets' RefreshConflict)
#[derive(Clone, PartialEq)]
enum Kept {
    User { rs: u32, shape: ShapeRef, column: String, user: String, data: String },
    Dup { rs: u32, key: String, rows: u32 },
}

impl Kept {
    fn rs(&self) -> u32 {
        match self {
            Kept::User { rs, .. } | Kept::Dup { rs, .. } => *rs,
        }
    }
}

// an entry of the pane
#[derive(Clone)]
enum Entry {
    Gone { rs: u32, link: RowLink, shape: ShapeRef },
    Kept(Kept),
}

/// the refresh state: the dialogs, the interval clock, the conflicts of the last refreshes
pub(crate) struct RefreshState {
    list: Shared<i32>,
    pick: i32,
    status: String,
    /// the sources waiting (the first one is being read or asked for)
    queue: Vec<u32>,
    /// the user started it: a file that cannot be read is asked for
    asking: bool,
    /// Daten aktualisieren comes back when the queue or the settings are done
    back: bool,
    ask: Shared<i32>,
    ask_text: String,
    file: Shared<i32>,
    file_path: String,
    file_for: Option<Pick>,
    set: Shared<i32>,
    set_rs: u32,
    set_auto: i32,
    set_minutes: i32,
    set_over: i32,
    set_path: Option<String>,
    kept: Vec<Kept>,
    /// (recordset, interval minutes, due at ms)
    due: Vec<(u32, u32, u32)>,
    /// the drawing these belong to (Opened::serial)
    serial: u32,
}

impl Default for RefreshState {
    fn default() -> RefreshState {
        RefreshState {
            list: Shared::new(0),
            pick: 0,
            status: String::new(),
            queue: Vec::new(),
            asking: false,
            back: false,
            ask: Shared::new(0),
            ask_text: String::new(),
            file: Shared::new(0),
            file_path: String::new(),
            file_for: None,
            set: Shared::new(0),
            set_rs: 0,
            set_auto: 0,
            set_minutes: 60,
            set_over: 0,
            set_path: None,
            kept: Vec::new(),
            due: Vec::new(),
            serial: u32::MAX,
        }
    }
}

// ---------------------------------------------------------------- sources

// whether a recordset can be refreshed at all
fn refreshable(data: &data::DataSets, rs: &Recordset) -> bool {
    !rs.no_refresh() && !matches!(data::source_of(data, rs), Source::Other)
}

// a path as the file dialog takes it: a relative one from the working folder
fn absolute(path: &str) -> String {
    let p = path.replace('\\', "/");
    if p.starts_with('/') || p.get(1..2) == Some(":") {
        p
    } else {
        format!("{}/{}", plat::cwd().trim_end_matches(['/', '\\']), p)
    }
}

fn file_of(source: &Source) -> &str {
    match source {
        Source::Excel { path, .. } | Source::Text { path, .. } => path,
        Source::Other => "",
    }
}

// the source with another file
fn moved(source: &Source, path: &str) -> Source {
    match source {
        Source::Excel { table, header, .. } => Source::Excel { path: String::from(path), table: table.clone(), header: *header },
        Source::Text { header, .. } => Source::Text { path: String::from(path), header: *header },
        Source::Other => Source::Other,
    }
}

// a chosen file as a connection names it: a Windows path with backslashes, as Visio's provider
// writes it (the file dialog gives '/')
fn native(path: &str) -> String {
    let b = path.as_bytes();
    if (b.len() > 2 && b[1] == b':') || path.starts_with("//") {
        path.replace('/', "\\")
    } else {
        String::from(path)
    }
}

// a recordset reading another file: its connection (when it has one) and the recordset with the
// new command
fn relinked(doc: &Document, rs: &Recordset, path: &str) -> Option<(Option<Connection>, Recordset)> {
    let path = native(path);
    let (cs, cmd) = match data::source_of(&doc.data, rs) {
        Source::Excel { table, header, .. } => data::excel_connection(&path, &table, header),
        Source::Text { header, .. } => data::text_connection(&path, header),
        Source::Other => return None,
    };
    let conn = rs.connection.and_then(|id| doc.data.connections.iter().find(|c| c.id == id)).map(|c| {
        let mut c = c.clone();
        c.connection_string = Some(cs);
        // the connection's own command only when no other recordset reads through it
        let shared = doc.data.recordsets.iter().any(|r| r.id != rs.id && r.connection == rs.connection);
        if c.command.is_some() && !shared {
            c.command = Some(cmd.clone());
        }
        c
    });
    let mut r = rs.clone();
    r.command = Some(cmd);
    Some((conn, r))
}

// "2026-11-23T14:05:00" in the user's format
fn when(text: &str) -> String {
    let n: Vec<u32> = text.split(['-', 'T', ':', ' ']).filter_map(|p| p.trim().parse().ok()).collect();
    match n.as_slice() {
        [y, mo, d, h, mi, rest @ ..] => {
            let c = datetime::Civil::new(*y as i32, *mo as u8, *d as u8, *h as u8, *mi as u8, rest.first().copied().unwrap_or(0) as u8);
            ui::i18n::fmt_date_time(&c)
        }
        _ => String::from(text),
    }
}

// ---------------------------------------------------------------- the commands

/// Alle aktualisieren
pub(crate) fn refresh_all(app: &mut Diagrammer, ui: &mut Ui) {
    sync(app);
    let ids: Vec<u32> = match app.opened.as_ref() {
        Some(o) => o.doc.data.recordsets.iter().filter(|r| refreshable(&o.doc.data, r)).map(|r| r.id).collect(),
        None => return,
    };
    let st = &mut app.drefresh;
    st.queue = ids;
    st.asking = true;
    st.back = false;
    st.status.clear();
    run(app, ui);
}

/// Daten aktualisieren... (the dialog with every source)
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    sync(app);
    let shown = crate::data_cmds::recordset_shown(app);
    let Some(o) = app.opened.as_ref() else { return };
    let st = &mut app.drefresh;
    st.pick = shown.and_then(|s| o.doc.data.recordsets.iter().position(|r| r.id == s)).unwrap_or(0) as i32;
    st.status.clear();
    st.list.set(1);
    ui.repass();
}

/// Aktualisierung konfigurieren... (the source shown: interval, overwrite user values)
pub(crate) fn settings_open(app: &mut Diagrammer, ui: &mut Ui) {
    sync(app);
    let Some(rs) = crate::data_cmds::recordset_shown(app) else { return };
    app.drefresh.back = false;
    settings_for(app, ui, rs);
}

fn settings_for(app: &mut Diagrammer, ui: &mut Ui, rs: u32) {
    let Some(set) = app.opened.as_ref().and_then(|o| o.doc.recordset(rs)) else { return };
    let (interval, over) = (set.refresh_interval, set.overwrite_all);
    let st = &mut app.drefresh;
    st.set_rs = rs;
    st.set_auto = (interval > 0) as i32;
    st.set_minutes = if interval > 0 { interval.min(10_080) as i32 } else { 60 };
    st.set_over = over as i32;
    st.set_path = None;
    st.set.set(1);
    ui.repass();
}

/// Aktualisierungskonflikte: the pane
pub(crate) fn conflicts_open(app: &mut Diagrammer, ui: &mut Ui) {
    app.panes.show(crate::panes::P_CONFLICTS);
    ui.repass();
}

/// whether there are conflicts to show
pub(crate) fn has_conflicts(app: &Diagrammer) -> bool {
    !entries(app).is_empty()
}

/// Nach Aktualisierung automatisch verknüpfen is on
pub(crate) fn auto_link_on(app: &Diagrammer) -> bool {
    app.opts.ini().get("data", "autolink") != Some("0")
}

/// Nach Aktualisierung automatisch verknüpfen on / off
pub(crate) fn toggle_auto_link(app: &mut Diagrammer, ui: &mut Ui) {
    let on = auto_link_on(app);
    app.opts.ini_mut().set("data", "autolink", if on { "0" } else { "1" });
    app.opts.save();
    ui.repass();
}

// another drawing: nothing of the last one stays
fn sync(app: &mut Diagrammer) {
    let serial = app.opened.as_ref().map_or(u32::MAX - 1, |o| o.serial);
    let st = &mut app.drefresh;
    if st.serial != serial {
        st.serial = serial;
        st.kept.clear();
        st.due.clear();
        st.queue.clear();
        st.file_for = None;
        for d in [&st.list, &st.ask, &st.file, &st.set] {
            d.set(0);
        }
    }
}

// ---------------------------------------------------------------- refreshing

// the sources of the queue, one after another; a file that cannot be read stops it for the
// question (when the user started it)
fn run(app: &mut Diagrammer, ui: &mut Ui) {
    while let Some(&rs) = app.drefresh.queue.first() {
        let Some(o) = app.opened.as_ref() else {
            app.drefresh.queue.clear();
            break;
        };
        let Some(set) = o.doc.recordset(rs) else {
            app.drefresh.queue.remove(0);
            continue;
        };
        let source = data::source_of(&o.doc.data, set);
        // (JD18) the columns the import kept (the Command's list)
        let cols = data_source::recordset_columns(&o.doc.data, set);
        let name = set.name.clone();
        if !refreshable(&o.doc.data, set) {
            note(app, tf("„{}“ lässt sich nicht aktualisieren.", &[&name]));
            app.drefresh.queue.remove(0);
            continue;
        }
        match data_source::read_columns(&source, &cols) {
            Ok(l) => {
                apply(app, ui, rs, None, l);
                app.drefresh.queue.remove(0);
            }
            Err(e @ (ReadError::Denied | ReadError::Missing)) if app.drefresh.asking => {
                let file = String::from(plat::display_name(file_of(&source)));
                let st = &mut app.drefresh;
                st.ask_text = if matches!(e, ReadError::Denied) {
                    tf("„{}“ wurde in dieser Sitzung noch nicht geöffnet. Wählen Sie die Datei aus, damit die Daten von „{}“ aktualisiert werden.", &[&file, &name])
                } else {
                    tf("„{}“ wurde nicht gefunden. Wählen Sie die Datei aus, damit die Daten von „{}“ aktualisiert werden.", &[&file, &name])
                };
                st.file_path = absolute(file_of(&source));
                st.file_for = Some(Pick::Locate(rs));
                st.ask.set(1);
                ui.repass();
                return;
            }
            Err(ReadError::Denied | ReadError::Missing) => {
                app.drefresh.queue.remove(0);
            }
            Err(ReadError::Invalid(m)) => {
                failed(app, &name, &m);
                app.drefresh.queue.remove(0);
            }
        }
    }
    let st = &mut app.drefresh;
    if st.back {
        st.back = false;
        st.list.set(1);
    }
    ui.repass();
}

fn note(app: &mut Diagrammer, line: String) {
    let st = &mut app.drefresh;
    if !st.status.is_empty() {
        st.status.push('\n');
    }
    st.status.push_str(&line);
}

fn failed(app: &mut Diagrammer, name: &str, msg: &str) {
    app.error = Some((tf("„{}“ lässt sich nicht aktualisieren.", &[name]), tf("Die Datenquelle ist nicht lesbar: {}", &[msg])));
    note(app, tf("„{}“ lässt sich nicht aktualisieren.", &[name]));
}

// the file the user chose for a source that could not be read: read it, refresh, go on
fn located(app: &mut Diagrammer, ui: &mut Ui, rs: u32, path: &str) {
    let found = app.opened.as_ref().and_then(|o| {
        let set = o.doc.recordset(rs)?;
        Some((moved(&data::source_of(&o.doc.data, set), path), set.name.clone(), data_source::recordset_columns(&o.doc.data, set)))
    });
    if let Some((source, name, cols)) = found {
        match data_source::read_columns(&source, &cols) {
            Ok(l) => apply(app, ui, rs, Some(path), l),
            Err(ReadError::Invalid(m)) => failed(app, &name, &m),
            Err(_) => failed(app, &name, t("Die Datei lässt sich nicht lesen.")),
        }
    }
    if app.drefresh.queue.first() == Some(&rs) {
        app.drefresh.queue.remove(0);
    }
    run(app, ui);
}

// new rows for recordset `rs` (from another file when `path` is given): one undo step
fn apply(app: &mut Diagrammer, ui: &mut Ui, rs: u32, path: Option<&str>, l: Loaded) {
    let auto = auto_link_on(app);
    let Some(o) = app.opened.as_mut() else { return };
    let Some(set) = o.doc.recordset(rs).cloned() else { return };
    let moved = match path {
        Some(p) => relinked(&o.doc, &set, p),
        None => None,
    };
    let pairs = if auto { crate::data_link::comparisons(&o.doc, &set) } else { Vec::new() };
    let now = crate::data_cmds::now_text();
    let Loaded { headers, rows } = l;
    let r = edit_doc::edit(o, t("Daten aktualisieren"), true, |tx, eng| {
        if let Some((conn, r)) = moved {
            if let Some(c) = conn {
                data::set_connection(tx, c)?;
            }
            data::set_recordset(tx, r)?;
        }
        let res = data::refresh(tx, rs, &headers, rows, &now)?;
        eng.recalc_cells(tx, &res.written)?;
        // Nach Aktualisierung automatisch verknüpfen: shapes of every page without a row of it
        let mut linked = 0;
        if !pairs.is_empty() {
            let doc = tx.doc();
            let shapes: Vec<ShapeRef> = doc.pages().flat_map(|p| doc.all_shapes(Owner::Page(p))).collect();
            let (made, w) = data::auto_link(tx, &shapes, rs, &pairs, false)?;
            eng.recalc_cells(tx, &w)?;
            linked = made.len();
        }
        Ok((res, linked))
    });
    let (res, linked) = match r {
        Ok(x) => x,
        Err(e) => {
            failed(app, &set.name, &format!("{e:?}"));
            return;
        }
    };
    // the conflicts the pane keeps (the rows gone are in the recordset)
    let st = &mut app.drefresh;
    st.kept.retain(|k| k.rs() != rs);
    for c in &res.conflicts {
        match c {
            Conflict::UserValue { shape, column, user, data } => st.kept.push(Kept::User { rs, shape: *shape, column: column.clone(), user: user.clone(), data: data.text() }),
            Conflict::DuplicateKey { key, rows } => st.kept.push(Kept::Dup { rs, key: key.clone(), rows: *rows }),
            Conflict::RowGone { .. } => {}
        }
    }
    let mut line = tf("„{}“ aktualisiert: {} geänderte, {} neue, {} entfernte Zeilen.", &[&set.name, &format!("{}", res.changed), &format!("{}", res.added), &format!("{}", res.removed)]);
    if linked > 0 {
        line.push(' ');
        line.push_str(&if linked == 1 { String::from(t("1 Shape neu verknüpft.")) } else { tf("{} Shapes neu verknüpft.", &[&format!("{linked}")]) });
    }
    let n = res.conflicts.len();
    if n > 0 {
        line.push(' ');
        line.push_str(&if n == 1 { String::from(t("1 Konflikt.")) } else { tf("{} Konflikte.", &[&format!("{n}")]) });
    }
    ui.announce(&line, LIVE_POLITE);
    note(app, line);
    if n > 0 {
        app.panes.show(crate::panes::P_CONFLICTS);
    }
    ui.repass();
}

// ---------------------------------------------------------------- the dialogs

/// every pass: the dialogs, the interval refresh
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    sync(app);
    list_dialog(app, ui);
    ask_dialog(app, ui);
    file_dialog(app, ui);
    settings_dialog(app, ui);
    interval(app, ui);
}

// Daten aktualisieren...
fn list_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.drefresh.list.clone();
    if open.get() == 0 {
        return;
    }
    let Some(o) = app.opened.as_ref() else {
        open.set(0);
        return;
    };
    let rows: Vec<(u32, String, bool)> = o
        .doc
        .data
        .recordsets
        .iter()
        .map(|r| {
            let last = r.time_refreshed.as_deref().map_or_else(|| String::from(t("noch nie")), when);
            let ok = refreshable(&o.doc.data, r);
            let line = if ok { tf("{} – zuletzt aktualisiert: {}", &[&r.name, &last]) } else { tf("{} – lässt sich nicht aktualisieren", &[&r.name]) };
            (r.id, line, ok)
        })
        .collect();
    let title = commands::label(DATA_REFRESH).trim_end_matches(['.', '…']);
    if !ui.dialog_begin(ID, title, &open) {
        return;
    }
    let st = &mut app.drefresh;
    ui.next_size(440, 0);
    ui.box_begin(ID + 1, BOX_V, 8);
    ui.text(TS_BODY, t("Datenquellen"));
    ui.next_size(-1, 140);
    let (a, b) = ui.list_begin(ID + 0x100, &mut st.pick, rows.len() as i32, 28);
    for i in a..b {
        ui.list_item(i, &rows[i as usize].1);
    }
    ui.list_end(&mut st.pick);
    let chosen = usize::try_from(st.pick).ok().and_then(|k| rows.get(k));
    ui.next_flags(ST_SUBTLE | ST_ALIGN_START | if chosen.is_some() { 0 } else { ST_DISABLED });
    let configure = ui.button(ID + 3, &format!("{}…", t("Konfigurieren"))) & EV_ACT != 0;
    if !st.status.is_empty() {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, &st.status);
    }
    ui.box_end();
    let can = chosen.is_some_and(|c| c.2);
    let r = ui.dialog_buttons(Some(t("Aktualisieren")), Some(commands::label(crate::commands::DATA_REFRESH_ALL)), Some(t("Schließen")));
    ui.dialog_end();
    let chosen = chosen.map(|c| c.0);
    match r {
        DLG_PRIMARY if can => {
            let st = &mut app.drefresh;
            st.queue = chosen.into_iter().collect();
            st.asking = true;
            st.back = true;
            st.status.clear();
            run(app, ui);
        }
        DLG_PRIMARY => open.set(1),
        DLG_SECONDARY => {
            let ids: Vec<u32> = rows.iter().filter(|r| r.2).map(|r| r.0).collect();
            let st = &mut app.drefresh;
            st.queue = ids;
            st.asking = true;
            st.back = true;
            st.status.clear();
            run(app, ui);
        }
        _ => {}
    }
    if configure {
        if let Some(rs) = chosen {
            open.set(0);
            app.drefresh.back = true;
            settings_for(app, ui, rs);
        }
    }
}

// the question before a file is chosen again
fn ask_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.drefresh.ask.clone();
    if !ui.dialog_begin(ID_ASK, t("Datenquelle suchen"), &open) {
        return;
    }
    ui.next_size(400, 0);
    ui.text(TS_BODY | TX_WRAP, &app.drefresh.ask_text);
    let r = ui.dialog_buttons(Some(&format!("{}…", t("Durchsuchen"))), Some(t("Überspringen")), Some(t("Abbrechen")));
    ui.dialog_end();
    match r {
        DLG_PRIMARY => {
            app.drefresh.file.set(1);
            ui.repass();
        }
        DLG_SECONDARY => {
            let st = &mut app.drefresh;
            st.file_for = None;
            if !st.queue.is_empty() {
                st.queue.remove(0);
            }
            run(app, ui);
        }
        DLG_CLOSE => {
            let st = &mut app.drefresh;
            st.file_for = None;
            st.queue.clear();
            run(app, ui);
        }
        _ => {}
    }
}

// the file dialog: the file of a source that could not be read, or another file in the settings
fn file_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.drefresh.file.clone();
    if open.get() == 0 {
        return;
    }
    let exts = [t("xlsx;xlsm;csv;txt=Datenquellen"), t("xlsx;xlsm=Excel-Arbeitsmappen"), t("csv;txt=Textdateien"), t("=Alle Dateien")];
    let chosen = ui.filedlg(ID_FILE, FD_OPEN, &open, &mut app.drefresh.file_path, 4096, &exts) & EV_ACT != 0;
    if !chosen && open.get() != 0 {
        return;
    }
    let path = if chosen { ui.filedlg_paths().into_iter().next().or_else(|| Some(app.drefresh.file_path.clone())) } else { None };
    match (app.drefresh.file_for.take(), path) {
        (Some(Pick::Locate(rs)), Some(p)) => located(app, ui, rs, &p),
        (Some(Pick::Locate(_)), None) => {
            // no file: this source is skipped
            if !app.drefresh.queue.is_empty() {
                app.drefresh.queue.remove(0);
            }
            run(app, ui);
        }
        (Some(Pick::Settings), p) => {
            if p.is_some() {
                app.drefresh.set_path = p;
            }
            app.drefresh.set.set(1);
            ui.repass();
        }
        (None, _) => {}
    }
}

// Aktualisierung konfigurieren...
fn settings_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.drefresh.set.clone();
    if open.get() == 0 {
        return;
    }
    let found = app.opened.as_ref().and_then(|o| {
        let r = o.doc.recordset(app.drefresh.set_rs)?;
        Some((r.name.clone(), data::source_of(&o.doc.data, r)))
    });
    let Some((name, source)) = found else {
        open.set(0);
        return;
    };
    let title = commands::label(DATA_REFRESH_SETTINGS).trim_end_matches(['.', '…']);
    if !ui.dialog_begin(ID_SET, title, &open) {
        return;
    }
    let st = &mut app.drefresh;
    ui.next_size(440, 0);
    ui.box_begin(ID_SET + 1, BOX_V, 8);
    ui.text(TS_BODY_STRONG, &name);
    let file = st.set_path.clone().unwrap_or_else(|| String::from(file_of(&source)));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &if file.is_empty() { String::from(t("Keine Datei: diese Datenquelle wird nicht aktualisiert.")) } else { tf("Datei: {}", &[&file]) });
    ui.next_flags(ST_SUBTLE | ST_ALIGN_START | if file.is_empty() { ST_DISABLED } else { 0 });
    let change = ui.button(ID_SET + 2, &format!("{}…", t("Datei ändern"))) & EV_ACT != 0;
    ui.spacer(4);
    ui.text(TS_BODY_STRONG, t("Automatische Aktualisierung"));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.checkbox(ID_SET + 3, t("Aktualisieren alle"), &mut st.set_auto);
    ui.next_size(90, 0);
    ui.next_flags(if st.set_auto != 0 { 0 } else { ST_DISABLED });
    ui.number_box(ID_SET + 4, &mut st.set_minutes, &Range::new(1, 10_080, 1, 0));
    ui.text(TS_BODY, t("Minuten"));
    ui.box_end();
    ui.spacer(4);
    ui.next_size(-1, 0);
    ui.checkbox_wrap(ID_SET + 5, t("Benutzerdefinierte Änderungen an Shape-Daten überschreiben"), &mut st.set_over);
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if change {
        let st = &mut app.drefresh;
        st.file_path = absolute(&file);
        st.file_for = Some(Pick::Settings);
        open.set(0);
        st.file.set(1);
        ui.repass();
        return;
    }
    if r == DLG_PRIMARY {
        settings_apply(app);
    }
    if r != DLG_NONE {
        let st = &mut app.drefresh;
        if st.back {
            st.back = false;
            st.list.set(1);
        }
        ui.repass();
    }
}

// the settings into the recordset (and its connection): one undo step
fn settings_apply(app: &mut Diagrammer) {
    let st = &app.drefresh;
    let (rs, auto, minutes, over, path) = (st.set_rs, st.set_auto != 0, st.set_minutes, st.set_over != 0, st.set_path.clone());
    let Some(o) = app.opened.as_mut() else { return };
    let Some(set) = o.doc.recordset(rs).cloned() else { return };
    let (conn, mut r) = match path.as_deref().and_then(|p| relinked(&o.doc, &set, p)) {
        Some((c, r)) => (c, r),
        None => (None, set.clone()),
    };
    r.refresh_interval = if auto { minutes.clamp(1, 10_080) as u32 } else { 0 };
    r.overwrite_all = over;
    if r == set && conn.is_none() {
        return;
    }
    let label = commands::label(DATA_REFRESH_SETTINGS).trim_end_matches(['.', '…']);
    let _ = edit_doc::edit(o, label, true, |tx, _| {
        if let Some(c) = conn {
            data::set_connection(tx, c)?;
        }
        data::set_recordset(tx, r)
    });
}

// ---------------------------------------------------------------- the interval

// nothing the user is doing would be disturbed by a refresh now
fn idle(app: &Diagrammer, ui: &Ui) -> bool {
    let st = &app.drefresh;
    let mine = [&st.list, &st.ask, &st.file, &st.set].iter().any(|d| d.get() != 0);
    !mine
        && ui.modal_active() == 0
        && ui.popup_count() == 0
        && ui.mbtn() == 0
        && !ui.dragging()
        && app.text.is_none()
        && app.backstage.is_none()
        && app.opened.as_ref().is_some_and(|o| o.ed.preview.is_none())
}

// the recordsets with an interval: the one due is refreshed (one per pass), the timer wakes the
// window for the next
fn interval(app: &mut Diagrammer, ui: &mut Ui) {
    let now = ui.now();
    let Some(o) = app.opened.as_ref() else { return };
    let mut fire: Option<u32> = None;
    let mut next: Option<u32> = None;
    let mut due = core::mem::take(&mut app.drefresh.due);
    due.retain(|d| o.doc.recordset(d.0).is_some_and(|r| r.refresh_interval == d.1 && refreshable(&o.doc.data, r)));
    for r in &o.doc.data.recordsets {
        if r.refresh_interval == 0 || !refreshable(&o.doc.data, r) {
            continue;
        }
        let ms = r.refresh_interval.saturating_mul(60_000);
        let at = match due.iter().find(|d| d.0 == r.id) {
            Some(d) => d.2,
            None => {
                due.push((r.id, r.refresh_interval, now.wrapping_add(ms)));
                now.wrapping_add(ms)
            }
        };
        let left = at.wrapping_sub(now) as i32;
        if left <= 0 {
            fire = fire.or(Some(r.id));
        } else {
            next = Some(next.map_or(left as u32, |n| n.min(left as u32)));
        }
    }
    app.drefresh.due = due;
    if let Some(rs) = fire {
        if idle(app, ui) {
            let ms = app.opened.as_ref().and_then(|o| o.doc.recordset(rs)).map_or(60_000, |r| r.refresh_interval.saturating_mul(60_000));
            if let Some(d) = app.drefresh.due.iter_mut().find(|d| d.0 == rs) {
                d.2 = now.wrapping_add(ms);
            }
            next = Some(next.map_or(ms, |n| n.min(ms)));
            let st = &mut app.drefresh;
            let (asking, queue) = (st.asking, core::mem::take(&mut st.queue));
            st.asking = false;
            st.queue = alloc::vec![rs];
            st.status.clear();
            run(app, ui);
            let st = &mut app.drefresh;
            st.asking = asking;
            st.queue = queue;
        } else {
            // busy: a second later again
            next = Some(1000);
        }
    }
    if let Some(ms) = next {
        ui.timer(TIMER, ms.max(16));
    }
}

// ---------------------------------------------------------------- the conflicts

// the conflicts there are now: the rows gone of every recordset (still bound, the shape still
// there), the values and keys kept from the last refreshes (still meaning something)
fn entries(app: &Diagrammer) -> Vec<Entry> {
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    // the values kept belong to the drawing they were found in
    let kept: &[Kept] = if app.drefresh.serial == o.serial { &app.drefresh.kept } else { &[] };
    let doc = &o.doc;
    let mut out = Vec::new();
    for r in &doc.data.recordsets {
        for l in &r.conflicts {
            let Some(shape) = doc.data_shape(l.page, l.shape) else { continue };
            if r.row_of(l.page, l.shape) == Some(l.row) {
                out.push(Entry::Gone { rs: r.id, link: *l, shape });
            }
        }
    }
    for k in kept {
        let ok = match k {
            Kept::User { rs, shape, column, .. } => doc.data_link(*shape, *rs).is_some() && !linked_field(doc, *shape, column),
            Kept::Dup { rs, .. } => doc.recordset(*rs).is_some(),
        };
        if ok {
            out.push(Entry::Kept(k.clone()));
        }
    }
    out
}

// whether a shape data field holds the linked value (DataLinked = 1)
fn linked_field(doc: &Document, shape: ShapeRef, name: &str) -> bool {
    doc.shape_data(SheetRef::Shape(shape)).into_iter().find(|r| r.name() == Some(name)).is_some_and(|r| r.num(data::DATA_LINKED).unwrap_or(0.0) != 0.0)
}

// what a shape is called in the pane: its text, else its name
fn shape_title(doc: &Document, s: ShapeRef) -> String {
    let text = diagram_core::field::plain(doc, s);
    let text = text.trim();
    if text.is_empty() {
        doc.shape(s).map(|x| x.name()).unwrap_or_default()
    } else {
        String::from(text)
    }
}

/// the pane Aktualisierungskonflikte
pub(crate) fn pane(app: &mut Diagrammer, ui: &mut Ui) {
    let list = entries(app);
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_PANE, SC_V | BOX_V | pad(10));
    if list.is_empty() {
        ui.empty_state(crate::icons::JD_REFRESH_ALL, t("Keine Konflikte"), t("Nach dem Aktualisieren stehen hier die Zeilen, die in der Datenquelle fehlen, und die Werte, die Sie selbst geändert haben."));
    }
    let mut act: Option<(usize, u32)> = None;
    if let Some(o) = app.opened.as_ref() {
        let doc = &o.doc;
        let rs_name = |id: u32| doc.recordset(id).map(|r| r.name.clone()).unwrap_or_default();
        for (k, e) in list.iter().enumerate() {
            let id = ID_PANE + 0x100 + k as u32 * 8;
            let (shape, desc, buttons): (Option<ShapeRef>, String, &[&'static str]) = match e {
                Entry::Gone { rs, shape, .. } => (Some(*shape), tf("Die Zeile fehlt in „{}“.", &[&rs_name(*rs)]), &GONE),
                Entry::Kept(Kept::User { rs, shape, column, user, data }) => {
                    let label = doc.recordset(*rs).and_then(|r| r.column_by_name(column).map(|c| r.columns[c].label.clone())).unwrap_or_else(|| column.clone());
                    (Some(*shape), tf("{}: Ihr Wert „{}“, in der Datenquelle „{}“.", &[&label, user, data]), &USER)
                }
                Entry::Kept(Kept::Dup { rs, key, rows }) => (None, tf("Der Schlüssel „{}“ steht {}-mal in „{}“; die erste Zeile gilt.", &[key, &format!("{rows}"), &rs_name(*rs)]), &DUP),
            };
            match shape {
                Some(s) => {
                    if ui.link(id, &shape_title(doc, s), TS_BODY_STRONG) & EV_ACT != 0 {
                        act = Some((k, 0));
                    }
                }
                None => ui.text(TS_BODY_STRONG, t("Doppelter Schlüssel")),
            }
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_WRAP, &desc);
            for (j, b) in buttons.iter().enumerate() {
                ui.next_flags(ST_SUBTLE | ST_ALIGN_START);
                if ui.button(id + 1 + j as u32, t(*b)) & EV_ACT != 0 {
                    act = Some((k, 1 + j as u32));
                }
            }
            ui.spacer(4);
            ui.separator();
        }
    }
    ui.scroll_end();
    if let Some((k, a)) = act {
        if let Some(e) = list.get(k) {
            resolve(app, ui, e, a);
        }
    }
}

// what the user decided about a conflict (0: the shape selected and shown)
fn resolve(app: &mut Diagrammer, ui: &mut Ui, e: &Entry, action: u32) {
    let shape = match e {
        Entry::Gone { shape, .. } | Entry::Kept(Kept::User { shape, .. }) => Some(*shape),
        Entry::Kept(Kept::Dup { .. }) => None,
    };
    if action == 0 {
        if let Some(s) = shape {
            let tab = app.opened.as_ref().and_then(|o| o.tabs.iter().position(|tb| Owner::Page(tb.page) == s.owner));
            if let Some(k) = tab {
                crate::cmds_text::show_shape(app, ui, k, Some(s));
            }
        }
        return;
    }
    let Some(o) = app.opened.as_mut() else { return };
    match (e, action) {
        (Entry::Gone { shape, .. }, 1) => {
            if crate::edit_canvas::delete(o, shape.owner, &[*shape]).is_ok() {
                app.se.select(&[]);
            }
        }
        (Entry::Gone { rs, shape, .. }, 2) => {
            let (rs, s) = (*rs, *shape);
            let _ = edit_doc::edit(o, t("Verknüpfung aufheben"), true, |tx, _| data::resolve_unlink(tx, s, rs));
        }
        (Entry::Gone { rs, link, .. }, 3) => {
            if let Some(mut r) = o.doc.recordset(*rs).cloned() {
                r.conflicts.retain(|l| l != link);
                let _ = edit_doc::edit(o, t("Behalten"), true, |tx, _| data::set_recordset(tx, r));
            }
        }
        (Entry::Kept(Kept::User { rs, shape, column, .. }), 1) => {
            // the entry stays kept: the linked value hides it, undo shows it again
            let (rs, s, c) = (*rs, *shape, column.clone());
            let _ = edit_doc::edit(o, t("Datenwert übernehmen"), true, |tx, eng| {
                let w = data::resolve_take_data(tx, s, rs, &c)?;
                eng.recalc_cells(tx, &w)?;
                Ok(())
            });
        }
        (Entry::Kept(k), _) => app.drefresh.kept.retain(|x| x != k),
        _ => {}
    }
    ui.repass();
}
