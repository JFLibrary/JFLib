// edit_doc -- (JD7) the drawing as a document to edit: every gesture as one transaction with
// the formula engine's recalculation in it, the live preview of a drag, undo and redo through
// ui::History, a new drawing (diagram_io::vsdx::new_document), saving (vsdx::write)
//
//   EditState                    per open drawing: the engine, the history, the preview of the
//                                gesture in progress, the file and its format, what changed
//   edit(o, label, done, f)      run f in a transaction: the preview so far is taken back, f
//                                writes cells (ops, glue: user_set, recalc_cells), the change
//                                is kept as the preview (done = false) or recorded as one undo
//                                step (done = true); a refused edit changes nothing
//   cancel(o)                    take the preview back (Esc during a drag)
//   undo(o) / redo(o)            a step back / forward, then the engine's dependencies again
//   new_drawing(app, ui)         "Zeichnung1", "Zeichnung2" ... (a counter per run, DM-01)
//   save(app, ui, as_new)        Speichern: to the file it came from (in its format), else and
//                                for Speichern unter the file dialog; save_to(app, ui, path)
//
// The flow of a gesture (jd6-formeln.md): cells through Engine::user_set (GUARD, SETATREF) and
// recalc_cells inside the transaction, so undo takes the results back with the change; after
// undo or redo Engine::refresh records the dependencies of the touched sheets anew.  Nothing is
// computed when a file opens.  While a drag runs, each step takes the last preview back and
// applies the whole gesture from its start (the widget reports totals), so the drawing shows
// the real result -- connectors routed, formulas computed -- at every step (ZF-09).

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::{Change, DocKind, Document, EditError, SheetRef, Transaction};
use diagram_io::vsdx::{self, Format, Measure, WriteOptions};
use ui::i18n::{t, tf};
use ui::*;

use crate::viewer::{Opened, PageTab};
use crate::Diagrammer;

/// one undo step: a committed transaction
pub(crate) struct Step(pub Change);

impl Edit<Document> for Step {
    fn apply(&mut self, d: &mut Document) {
        self.0.redo(d);
    }
    fn revert(&mut self, d: &mut Document) {
        self.0.undo(d);
    }
    fn label(&self) -> &str {
        self.0.label()
    }
    fn merge(&mut self, next: &mut dyn Edit<Document>) -> bool {
        next.value().and_then(|v| v.downcast_ref::<Step>()).is_some_and(|n| self.0.merge(&n.0))
    }
    fn value(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }
}

/// what an open drawing needs for editing
pub(crate) struct EditState {
    pub eng: Engine,
    pub history: History<Document>,
    /// the change of the gesture in progress (applied, not in the history yet)
    pub preview: Option<Change>,
    /// the file it was read from or saved to (None: a new drawing)
    pub path: Option<String>,
    pub format: Format,
    /// counts every change (the canvas' version)
    pub rev: u32,
    /// sheets changed since the page views were brought up to date
    pub changed: Vec<SheetRef>,
    /// the page views must be built anew (shapes added, removed, reordered; undo)
    pub rebuild: bool,
}

impl EditState {
    pub(crate) fn new(path: Option<String>, format: Format, metrics: Option<Rc<dyn diagram_core::formula::TextMetrics>>) -> EditState {
        EditState {
            eng: match metrics {
                Some(m) => Engine::with_metrics(m),
                None => Engine::new(),
            },
            history: History::new(500),
            preview: None,
            path,
            format,
            rev: 0,
            changed: Vec::new(),
            rebuild: false,
        }
    }

    fn touched(&mut self, c: &Change, structural: bool) {
        self.changed.extend(c.sheets());
        self.rebuild |= structural;
        self.rev = self.rev.wrapping_add(1);
    }
}

/// run `f` as one transaction (see the file header) -> what f returned (Err: nothing changed)
pub(crate) fn edit<T>(o: &mut Opened, label: &str, done: bool, f: impl FnOnce(&mut Transaction, &mut Engine) -> Result<T, EditError>) -> Result<T, EditError> {
    take_back(o);
    let ed = &mut o.ed;
    let mut tx = o.doc.transaction(label);
    let r = f(&mut tx, &mut ed.eng);
    match r {
        Ok(v) => {
            if let Some(c) = tx.commit() {
                let structural = changes_structure(&c);
                ed.touched(&c, structural);
                if done {
                    ed.history.record(Box::new(Step(c)));
                } else {
                    ed.preview = Some(c);
                }
            }
            Ok(v)
        }
        Err(e) => {
            tx.rollback();
            // the engine saw cells of the refused edit: it builds its graph again when needed
            ed.eng.reset();
            Err(e)
        }
    }
}

// whether a change adds, removes or reorders shapes (then the views are built anew): its sheets
// include shapes that no longer exist or the page's own sheet (Connect entries, shape order)
fn changes_structure(c: &Change) -> bool {
    c.sheets().iter().any(|s| matches!(s, SheetRef::Page(_) | SheetRef::Master(_) | SheetRef::Style(_) | SheetRef::Document))
}

// take the preview of the gesture back
fn take_back(o: &mut Opened) {
    if let Some(mut c) = o.ed.preview.take() {
        // (a change that no longer fits refuses and leaves the drawing as it is)
        c.undo(&mut o.doc);
        let sheets = c.sheets();
        o.ed.eng.refresh(&o.doc, &sheets);
        o.ed.changed.extend(sheets);
        o.ed.rebuild |= changes_structure(&c);
        o.ed.rev = o.ed.rev.wrapping_add(1);
    }
}

/// take the preview back (Esc during a drag)
pub(crate) fn cancel(o: &mut Opened) {
    take_back(o);
}

/// a step back -> done
pub(crate) fn undo(o: &mut Opened) -> bool {
    take_back(o);
    if !o.ed.history.undo(&mut o.doc) {
        return false;
    }
    let sheets = o.ed.history.redo_value().and_then(|v| v.downcast_ref::<Step>()).map(|s| s.0.sheets()).unwrap_or_default();
    after_history(o, sheets);
    true
}

/// a step forward -> done
pub(crate) fn redo(o: &mut Opened) -> bool {
    take_back(o);
    if !o.ed.history.redo(&mut o.doc) {
        return false;
    }
    let sheets = o.ed.history.undo_value().and_then(|v| v.downcast_ref::<Step>()).map(|s| s.0.sheets()).unwrap_or_default();
    after_history(o, sheets);
    true
}

fn after_history(o: &mut Opened, sheets: Vec<SheetRef>) {
    o.ed.eng.refresh(&o.doc, &sheets);
    o.ed.changed.extend(sheets);
    // shapes may come back or go: the views anew
    o.ed.rebuild = true;
    o.ed.rev = o.ed.rev.wrapping_add(1);
}

// ---------------------------------------------------------------- new, save

static NEW_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// a new empty drawing (Neu › Leere Zeichnung, and at start): metric unless the options say US
pub(crate) fn new_drawing(app: &mut Diagrammer, ui: &mut Ui, us: bool) {
    let mut doc = vsdx::new_document(DocKind::Drawing, if us { Measure::Us } else { Measure::Metric });
    // the page's shown name in the user's language ("Seite-1"); NameU stays "Page-1"
    let first: Vec<_> = doc.pages().collect();
    for (i, p) in first.into_iter().enumerate() {
        let n = doc.intern(&tf("Seite-{}", &[&format!("{}", i + 1)]));
        if let Some(pg) = doc.page_mut(p) {
            pg.header.name = Some(n);
        }
    }
    let n = NEW_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1;
    let name = tf("Zeichnung{}", &[&format!("{n}")]);
    let format = Format { kind: DocKind::Drawing, macros: false };
    install(app, ui, doc, name, None, format, None);
}

/// make `doc` the open drawing (opened from a file, or new)
pub(crate) fn install(app: &mut Diagrammer, ui: &mut Ui, doc: Document, name: String, path: Option<String>, format: Format, notice: Option<(String, String)>) {
    let mut tabs: Vec<PageTab> = Vec::new();
    for bg in [false, true] {
        for p in doc.pages() {
            let Some(pg) = doc.page(p) else { continue };
            if pg.header.background != bg {
                continue;
            }
            let n = pg.header.name.or(pg.header.name_u).map(|s| String::from(doc.str(s))).unwrap_or_else(|| tf("Seite-{}", &[&format!("{}", tabs.len() + 1)]));
            tabs.push(PageTab { page: p, name: n });
        }
    }
    let serial = app.opened.as_ref().map_or(1, |o| o.serial.wrapping_add(1));
    let metrics = Some(crate::viewer::metrics(app));
    let ed = EditState::new(path, format, metrics);
    let n = tabs.len();
    app.opened = Some(Opened::new(doc, name, tabs, notice, serial, ed));
    let _ = n;
    app.error = None;
    app.page_tab = 0;
    app.backstage = None;
    app.se = SeState::new();
    // (JD10) the drawing's own switch for the Verbindungspfeile
    app.structs.arrows = app.opened.as_ref().is_none_or(|o| crate::structures_cmds::arrows_of(&o.doc));
    app.canvas.request_fit();
    app.fit_again = true;
    ui.repass();
}

/// Speichern (as_new = false) / Speichern unter: to the drawing's file, else ask for one
pub(crate) fn save(app: &mut Diagrammer, ui: &mut Ui, as_new: bool) {
    let Some(o) = app.opened.as_ref() else { return };
    match (&o.ed.path, as_new) {
        (Some(p), false) => {
            let p = p.clone();
            save_to(app, ui, &p);
        }
        _ => {
            let stem = o.name.clone();
            let ext = o.ed.format.extension();
            // the folder of the file it came from, else --save-dir (a test aid), else the
            // dialog's own start
            let dir = match &o.ed.path {
                Some(p) => p.rsplit_once(['/', '\\']).map(|x| String::from(x.0)),
                None => plat::arg_value("--save-dir").map(|d| if d.starts_with('/') || d.get(1..2) == Some(":") { d } else { format!("{}/{}", plat::cwd(), d) }),
            };
            app.save_path = match dir {
                Some(d) => format!("{d}/{stem}.{ext}"),
                None => format!("{stem}.{ext}"),
            };
            app.save_dlg.set(1);
            ui.repass();
        }
    }
}

/// write the drawing to `path` (its format from the extension; unknown: .vsdx appended)
pub(crate) fn save_to(app: &mut Diagrammer, ui: &mut Ui, path: &str) -> bool {
    let Some(o) = app.opened.as_mut() else { return false };
    take_back(o);
    let ext = path.rsplit_once('.').map(|x| x.1).unwrap_or("");
    let (path, format) = match Format::from_extension(ext) {
        Some(f) => (String::from(path), f),
        None => (format!("{path}.vsdx"), Format { kind: DocKind::Drawing, macros: false }),
    };
    let name = String::from(plat::display_name(&path));
    let opts = WriteOptions { format: Some(format), ..WriteOptions::DEFAULT };
    let bytes = match vsdx::write(&o.doc, &opts) {
        Ok(b) => b,
        Err(e) => {
            app.error = Some((tf("„{}“ kann nicht gespeichert werden.", &[&name]), format!("{e}")));
            ui.repass();
            return false;
        }
    };
    if !plat::write_file(&path, &bytes) {
        app.error = Some((tf("„{}“ kann nicht gespeichert werden.", &[&name]), String::from(t("Die Datei lässt sich nicht schreiben."))));
        ui.repass();
        return false;
    }
    let stem = String::from(name.rsplit_once('.').map_or(name.as_str(), |x| x.0));
    o.name = stem;
    o.ed.path = Some(path.clone());
    o.ed.format = format;
    o.ed.history.mark_saved();
    app.error = None;
    ui.recent_add(&path);
    ui.repass();
    true
}
