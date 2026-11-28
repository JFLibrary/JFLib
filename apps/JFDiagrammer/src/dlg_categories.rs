// dlg_categories -- (JD21, ST-02) Kategorien: the categories a container takes or refuses and the
// categories of shapes (Visio's cells msvSDContainerRequiredCategories, msvSDContainerExcluded-
// Categories, msvShapeCategories; Visio has no dialog for them, only the formula sheet)
//
//   CatDialog                 its state
//   open(app, ui)             for the selection: the container among it (or around it) and the
//                             selected shapes
//   dialog(app, ui)           every pass; OK writes the cells in one undo step
//   available(app, id)        shapes are selected
//
// Categories are written as Visio writes them: ';' between them, no spaces (a ',' typed is taken
// as ';', doubles go, case is kept).  A shape joins a container when it has one of the required
// categories (none required: any) and none of the excluded (diagram_core::structure, Visio's
// rule); the new rule applies from the next move or drop, as in Visio -- members stay.  Several
// shapes with different categories show an empty field and keep their own unless one is typed.
// The categories used on the page are listed as a hint.

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::structure;
use diagram_core::{Owner, ShapeRef, SheetRef, Unit, Value};
use ui::i18n::t;
use ui::*;

use crate::commands::{self, *};
use crate::{edit_canvas, edit_doc, structures, Diagrammer};

const ID: u32 = 0x7EBC_0000;

/// the dialog's state
pub(crate) struct CatDialog {
    pub open: Shared<i32>,
    container: Option<ShapeRef>,
    shapes: Vec<ShapeRef>,
    required: String,
    excluded: String,
    own: String,
    /// the shapes had different categories when it opened (the field started empty)
    mixed: bool,
    /// the texts as they opened (unchanged: nothing written)
    was: (String, String, String),
    /// the categories on the page
    known: String,
}

impl Default for CatDialog {
    fn default() -> CatDialog {
        CatDialog {
            open: Shared::new(0),
            container: None,
            shapes: Vec::new(),
            required: String::new(),
            excluded: String::new(),
            own: String::new(),
            mixed: false,
            was: (String::new(), String::new(), String::new()),
            known: String::new(),
        }
    }
}

/// whether a command of this file can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    id != CATEGORIES || (app.opened.is_some() && !app.se.sel.is_empty())
}

/// a list as Visio writes it: ';' between, no spaces, each once
pub(crate) fn normal(s: &str) -> String {
    let mut seen: Vec<String> = Vec::new();
    for p in s.split([';', ',']).map(str::trim).filter(|p| !p.is_empty()) {
        if !seen.iter().any(|x| x.eq_ignore_ascii_case(p)) {
            seen.push(String::from(p));
        }
    }
    seen.join(";")
}

/// Kategorien: the dialog for the selection
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return };
    let doc = &o.doc;
    let container = structures::selected_container(app);
    let shapes = edit_canvas::selected(app);
    let (required, excluded) = container.and_then(|c| structure::container(doc, c)).map_or((String::new(), String::new()), |i| (i.required.join(";"), i.excluded.join(";")));
    let lists: Vec<String> = shapes.iter().map(|&s| structure::categories(doc, s).join(";")).collect();
    let mixed = lists.windows(2).any(|w| w[0] != w[1]);
    let own = if mixed { String::new() } else { lists.first().cloned().unwrap_or_default() };
    let mut known: BTreeSet<String> = BTreeSet::new();
    for s in doc.all_shapes(Owner::Page(page)) {
        known.extend(structure::categories(doc, s));
        if let Some(i) = structure::container(doc, s) {
            known.extend(i.required);
            known.extend(i.excluded);
        }
    }
    let d = &mut app.addons.cats;
    d.container = container;
    d.shapes = shapes;
    d.was = (required.clone(), excluded.clone(), own.clone());
    d.required = required;
    d.excluded = excluded;
    d.own = own;
    d.mixed = mixed;
    d.known = known.into_iter().collect::<Vec<_>>().join("; ");
    d.open.set(1);
    ui.repass();
}

fn field(ui: &mut Ui, id: u32, label: &str, v: &mut String) {
    ui.next_size(440, 0);
    ui.text(TS_BODY, label);
    ui.next_size(440, 0);
    ui.textbox(id, None, v, 256);
}

/// the dialog (every pass)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.addons.cats.open.clone();
    if !ui.dialog_begin(ID, commands::label(CATEGORIES), &open) {
        return;
    }
    let d = &mut app.addons.cats;
    ui.next_size(440, 0);
    ui.box_begin(ID + 1, BOX_V, 6);
    if d.container.is_some() {
        ui.text(TS_BODY_STRONG, t("Container"));
        field(ui, ID + 2, t("Erforderliche Kategorien"), &mut d.required);
        field(ui, ID + 3, t("Ausgeschlossene Kategorien"), &mut d.excluded);
        ui.next_size(440, 0);
        ui.text(
            TS_CAPTION | TX_SECONDARY | TX_WRAP,
            t("Ein Shape wird Mitglied, wenn es eine der erforderlichen Kategorien hat (ohne Angabe: jede) und keine der ausgeschlossenen. Die Regel gilt ab dem nächsten Verschieben oder Ablegen."),
        );
    }
    ui.text(TS_BODY_STRONG, if d.shapes.len() > 1 { t("Ausgewählte Shapes") } else { t("Shape") });
    field(ui, ID + 4, t("Kategorien des Shapes"), &mut d.own);
    if d.mixed {
        ui.next_size(440, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Die Shapes haben verschiedene Kategorien; leer lassen behält sie."));
    }
    ui.next_size(440, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Mehrere Kategorien durch Semikolon trennen."));
    if !d.known.is_empty() {
        ui.next_size(440, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &ui::i18n::tf("Auf dieser Seite: {}", &[&d.known]));
    }
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        apply(app, ui);
    }
    if r != DLG_NONE {
        app.addons.cats.open.set(0);
        ui.repass();
    }
}

// OK: the cells that changed (one undo step)
fn apply(app: &mut Diagrammer, ui: &mut Ui) {
    let d = &app.addons.cats;
    let (req, exc, own) = (normal(&d.required), normal(&d.excluded), normal(&d.own));
    let mut cells: Vec<(ShapeRef, &'static str, String)> = Vec::new();
    if let Some(c) = d.container {
        if req != normal(&d.was.0) {
            cells.push((c, "msvSDContainerRequiredCategories", req));
        }
        if exc != normal(&d.was.1) {
            cells.push((c, "msvSDContainerExcludedCategories", exc));
        }
    }
    // mixed and left empty: each keeps its own
    if own != normal(&d.was.2) && !(d.mixed && own.is_empty()) {
        for &s in &d.shapes {
            cells.push((s, "msvShapeCategories", own.clone()));
        }
    }
    if cells.is_empty() {
        return;
    }
    let Some(o) = app.opened.as_mut() else { return };
    let _ = edit_doc::edit(o, commands::label(CATEGORIES), true, |tx, eng| {
        let mut sheets = Vec::new();
        for (s, name, v) in &cells {
            let sym = tx.intern(v);
            let sheet = SheetRef::Shape(*s);
            tx.set_user(sheet, name, Value::Text(sym), Some(Unit::Str))?;
            sheets.push(sheet);
        }
        eng.recalc(tx, &sheets).map(|_| ())
    });
    ui.repass();
}
