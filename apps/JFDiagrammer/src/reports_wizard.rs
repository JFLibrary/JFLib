// reports_wizard -- (JD19, AS-05) the wizard Berichtsdefinition: Neu... and Ändern... of the
// dialog Berichte, four steps as Visio's
//
//   1 Shapes      every page, the current page or the selected shapes; Erweitert... the
//                 criteria (property, condition, value; case)
//   2 Eigenschaften   the columns: built-in fields (<Anzeigetext>, <Master-Name>, <Shape-Name>,
//                 <Shape-ID>, <PinX>, <PinY>, <Breite>, <Höhe>), the shape data of the drawing's
//                 shapes, with „Alle Eigenschaften anzeigen“ also hidden shape data and user cells
//   3 Titel       the report's title; Teilergebnisse... (group by a column, total, average,
//                 count, minimum, maximum per column, summaries only, grand total, duplicates),
//                 Sortieren... (the columns' order, three sort keys), Format... (decimals, units)
//   4 Name        name, description, where it is kept: in this drawing (the document sheet,
//                 one undo step; Ändern replaces the drawing's definition) or in a file (.vrd)
//
// The sub-dialogs show instead of the wizard while they are open (as the file dialogs of the
// data selector do) and work on copies: Abbrechen leaves the definition as it was.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::report::{self, Builtin, Condition, Field, Filter, Property, ReportDef, Scope, Sort, BUILTINS, FUNCS};
use diagram_core::{Document, Owner, SheetRef};
use diagram_io::report as rio;
use ui::i18n::{t, tf};
use ui::*;

use crate::reports::{Entry, Src};
use crate::{edit_doc, Diagrammer};

const ID: u32 = 0x7E10_0000;
const ID_PROP: u32 = 0x7E11_0000;
const ID_SUB: u32 = 0x7E12_0000;
const ID_FILE: u32 = 0x7E13_0000;
const W: i32 = 480;

// sub-dialogs
const SUB_NONE: u8 = 0;
const SUB_ADVANCED: u8 = 1;
const SUB_TOTALS: u8 = 2;
const SUB_SORT: u8 = 3;
const SUB_FORMAT: u8 = 4;
const SUB_FILE: u8 = 5;

/// the conditions in the order of the list
const CONDS: [Condition; 8] = [Condition::Eq, Condition::Ne, Condition::Gt, Condition::Ge, Condition::Lt, Condition::Le, Condition::Exists, Condition::NotExists];

/// the wizard's state
pub(crate) struct Wizard {
    open: Shared<i32>,
    step: u8,
    sub: u8,
    sub_open: Shared<i32>,
    /// Durchsuchen... of the last step
    file_dlg: Shared<i32>,
    def: ReportDef,
    editing: Option<Src>,
    scope: i32,
    /// the properties offered (shown, property) and which are taken
    props: Vec<(String, Property)>,
    checked: Vec<i32>,
    show_all: i32,
    // Erweitert...
    filters: Vec<Filter>,
    case: i32,
    f_prop: i32,
    f_cond: i32,
    f_value: String,
    f_pick: i32,
    // Teilergebnisse...
    group: i32,
    funcs: Vec<[i32; 5]>,
    func_col: i32,
    only_sums: i32,
    grand: i32,
    dups: i32,
    // Sortieren...
    order: Vec<usize>,
    order_sel: i32,
    keys: [i32; 3],
    dirs: [i32; 3],
    // Format...
    precision: i32,
    units: i32,
    // the last two steps
    title: String,
    name: String,
    desc: String,
    place: i32,
    path: String,
    error: Option<String>,
}

impl Default for Wizard {
    fn default() -> Wizard {
        Wizard {
            open: Shared::new(0),
            step: 0,
            sub: SUB_NONE,
            sub_open: Shared::new(0),
            file_dlg: Shared::new(0),
            def: ReportDef::new(""),
            editing: None,
            scope: 0,
            props: Vec::new(),
            checked: Vec::new(),
            show_all: 0,
            filters: Vec::new(),
            case: 0,
            f_prop: 0,
            f_cond: 0,
            f_value: String::new(),
            f_pick: -1,
            group: 0,
            funcs: Vec::new(),
            func_col: 0,
            only_sums: 0,
            grand: 0,
            dups: 0,
            order: Vec::new(),
            order_sel: -1,
            keys: [0; 3],
            dirs: [0; 3],
            precision: 2,
            units: 1,
            title: String::new(),
            name: String::new(),
            desc: String::new(),
            place: 0,
            path: String::new(),
            error: None,
        }
    }
}

impl Wizard {
    /// the wizard or one of its sub-dialogs is open
    pub(crate) fn is_open(&self) -> bool {
        self.open.get() != 0 || self.sub != SUB_NONE
    }
}

/// a built-in field's name in the list (in angle brackets, as Visio shows them)
pub(crate) fn builtin_label(b: &Builtin) -> String {
    let s = match b {
        Builtin::Text => t("<Anzeigetext>"),
        Builtin::Master => t("<Master-Name>"),
        Builtin::Name => t("<Shape-Name>"),
        Builtin::NameId => t("<Shape-ID>"),
        Builtin::PinX => t("<PinX>"),
        Builtin::PinY => t("<PinY>"),
        Builtin::Width => t("<Breite>"),
        Builtin::Height => t("<Höhe>"),
        Builtin::Other(k) => return format!("<{k}>"),
    };
    String::from(s)
}

/// the properties a report on this drawing can show: the shape data of its shapes (by row name,
/// shown by label; with `all` also hidden shape data and user cells), then the built-in fields
pub(crate) fn properties(doc: &Document, all: bool) -> Vec<(String, Property)> {
    let mut data: Vec<(String, Property)> = Vec::new();
    let pages: Vec<diagram_core::PageRef> = doc.pages().collect();
    'pages: for p in pages {
        for s in doc.all_shapes(Owner::Page(p)) {
            if data.len() > 400 {
                break 'pages;
            }
            let sheet = SheetRef::Shape(s);
            for r in doc.shape_data(sheet) {
                let Some(name) = r.name() else { continue };
                if (!all && r.flag("Invisible")) || data.iter().any(|d| d.1 == Property::Data(String::from(name))) {
                    continue;
                }
                let label = r.text("Label").filter(|l| !l.trim().is_empty()).unwrap_or(name);
                data.push((String::from(label), Property::Data(String::from(name))));
            }
            if all {
                for r in doc.user_cells(sheet) {
                    let Some(name) = r.name() else { continue };
                    if name.starts_with("JFD_") || data.iter().any(|d| d.1 == Property::User(String::from(name))) {
                        continue;
                    }
                    data.push((format!("User.{name}"), Property::User(String::from(name))));
                }
            }
        }
    }
    // two fields with one label: the row names tell them apart
    let shown: Vec<String> = data.iter().map(|d| d.0.clone()).collect();
    for d in &mut data {
        if shown.iter().filter(|s| **s == d.0).count() > 1 {
            d.0 = format!("{} ({})", d.0, d.1.key());
        }
    }
    data.sort_by_key(|d| d.0.to_lowercase());
    // the drawing's fields first (what a report mostly shows), then the built-in ones
    data.extend(BUILTINS.iter().map(|b| (builtin_label(b), Property::Builtin(b.clone()))));
    data
}

// the data type of a property in the drawing (a shape data row's Type; lengths are numbers)
fn data_type(doc: &Document, p: &Property) -> u16 {
    match p {
        Property::Builtin(b) if b.is_length() => 2,
        Property::Data(n) => {
            for pg in doc.pages() {
                for s in doc.all_shapes(Owner::Page(pg)) {
                    if let Some(r) = doc.shape_data(SheetRef::Shape(s)).into_iter().find(|r| r.name() == Some(n.as_str())) {
                        return r.num("Type").map_or(0, |x| x.clamp(0.0, 7.0) as u16);
                    }
                }
            }
            0
        }
        _ => 0,
    }
}

fn start(app: &mut Diagrammer, ui: &mut Ui, def: ReportDef, editing: Option<Src>) {
    let Some(o) = app.opened.as_ref() else { return };
    let w = &mut app.reports.wizard;
    w.scope = match def.scope {
        Scope::Page => 1,
        Scope::Selection => 2,
        _ => 0,
    };
    w.show_all = (def.user_cells || def.hidden_properties) as i32;
    w.props = properties(&o.doc, w.show_all != 0);
    // a column of the definition the drawing does not have stays on offer
    for f in &def.fields {
        if !w.props.iter().any(|p| p.1 == f.property) {
            w.props.push((String::from(f.heading()), f.property.clone()));
        }
    }
    w.checked = w.props.iter().map(|p| def.fields.iter().any(|f| f.property == p.1) as i32).collect();
    w.title = def.title.clone();
    w.name = def.name.clone();
    w.desc = def.description.clone();
    w.place = matches!(editing, Some(Src::File(_))) as i32;
    w.path = match &editing {
        Some(Src::File(p)) => p.clone(),
        _ => String::new(),
    };
    w.def = def;
    w.editing = editing;
    w.step = 0;
    w.sub = SUB_NONE;
    w.error = None;
    w.open.set(1);
    ui.repass();
}

/// Neu...: an empty definition (every page)
pub(crate) fn open_new(app: &mut Diagrammer, ui: &mut Ui) {
    start(app, ui, ReportDef::new(""), None);
}

/// Ändern...: the chosen definition (a predefined one is kept as a new one)
pub(crate) fn open_edit(app: &mut Diagrammer, ui: &mut Ui, e: &Entry) {
    let editing = match &e.src {
        Src::Builtin(_) => None,
        s => Some(s.clone()),
    };
    start(app, ui, e.def.clone(), editing);
}

/// every pass: the wizard or its open sub-dialog
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    // the file dialog also while closed (it forgets the last name it showed when it sees itself
    // closed)
    file_dialog(app, ui);
    match app.reports.wizard.sub {
        SUB_ADVANCED => return advanced(app, ui),
        SUB_TOTALS => return totals(app, ui),
        SUB_SORT => return sorting(app, ui),
        SUB_FORMAT => return format_dialog(app, ui),
        SUB_FILE => return,
        _ => {}
    }
    let open = app.reports.wizard.open.clone();
    if open.get() == 0 {
        return;
    }
    if !ui.dialog_begin(ID, t("Assistent für Berichtsdefinitionen"), &open) {
        return;
    }
    let step = app.reports.wizard.step;
    ui.next_size(W, 0);
    ui.box_begin(ID + 1, BOX_V, 8);
    let mut sub = SUB_NONE;
    match step {
        0 => sub = page_shapes(app, ui),
        1 => page_props(app, ui),
        2 => sub = page_title(app, ui),
        _ => sub = page_name(app, ui),
    }
    if let Some(e) = &app.reports.wizard.error {
        ui.next_size(W, 0);
        ui.text(TS_CAPTION | TX_WRAP | TX_ACCENT, e);
    }
    ui.box_end();
    let last = step >= 3;
    let r = ui.dialog_buttons(Some(if last { t("Fertig stellen") } else { t("Weiter >") }), (step > 0).then(|| t("< Zurück")), Some(t("Abbrechen")));
    ui.dialog_end();
    if sub != SUB_NONE {
        open_sub(app, sub);
        ui.repass();
        return;
    }
    let keep = match r {
        DLG_PRIMARY if last => !finish(app),
        DLG_PRIMARY => next(app),
        DLG_SECONDARY => {
            let w = &mut app.reports.wizard;
            w.error = None;
            w.step = w.step.saturating_sub(1);
            true
        }
        DLG_NONE => return,
        _ => false,
    };
    app.reports.wizard.open.set(keep as i32);
    ui.repass();
}

// Weiter: the step's choices into the definition -> stay open
fn next(app: &mut Diagrammer) -> bool {
    let doc = app.opened.as_ref().map(|o| &o.doc);
    let w = &mut app.reports.wizard;
    w.error = None;
    match w.step {
        0 => {
            w.def.scope = [Scope::AllPages, Scope::Page, Scope::Selection][w.scope.clamp(0, 2) as usize];
        }
        1 => {
            let Some(doc) = doc else { return false };
            if !w.checked.iter().any(|&c| c != 0) {
                w.error = Some(String::from(t("Wählen Sie mindestens eine Eigenschaft.")));
                return true;
            }
            take_columns(w, doc);
            if w.title.trim().is_empty() {
                w.title = String::from(t("Bericht"));
            }
        }
        2 => {
            w.def.title = w.title.clone();
            if w.name.trim().is_empty() {
                w.name = w.title.clone();
            }
        }
        _ => {}
    }
    w.step += 1;
    true
}

// the checked properties as the definition's columns: the ones it had keep their settings and
// order, new ones come after them; groups and sort keys of columns that went go too
fn take_columns(w: &mut Wizard, doc: &Document) {
    let mut kept: Vec<Field> = Vec::new();
    let mut new: Vec<Field> = Vec::new();
    let mut next_id = w.def.next_field_id();
    for (k, (shown, p)) in w.props.iter().enumerate() {
        if w.checked.get(k).copied().unwrap_or(0) == 0 {
            continue;
        }
        match w.def.fields.iter().find(|f| &f.property == p) {
            Some(f) => kept.push(f.clone()),
            None => {
                let heading: String = shown.chars().filter(|c| !matches!(c, '<' | '>')).collect();
                let mut f = Field::new(next_id, p.clone(), &heading);
                f.data_type = data_type(doc, p);
                next_id += 1;
                new.push(f);
            }
        }
    }
    kept.sort_by_key(|f| f.order);
    kept.extend(new);
    for (i, f) in kept.iter_mut().enumerate() {
        f.order = i as u32 + 1;
    }
    let ids: Vec<u32> = kept.iter().map(|f| f.id).collect();
    w.def.fields = kept;
    w.def.sorts.retain(|s| ids.contains(&s.field));
    if let Some(g) = w.def.grouping.as_mut() {
        if !ids.contains(&g.field) {
            g.field = 0;
        }
    }
}

// Fertig stellen -> the wizard closes
fn finish(app: &mut Diagrammer) -> bool {
    let w = &mut app.reports.wizard;
    let name = String::from(w.name.trim());
    if name.is_empty() {
        w.error = Some(String::from(t("Der Bericht braucht einen Namen.")));
        return false;
    }
    let mut def = w.def.clone();
    def.name = name.clone();
    def.title = w.title.clone();
    def.description = w.desc.clone();
    def.advanced_filter = !def.filters.is_empty();
    def.user_cells = w.show_all != 0;
    def.hidden_properties = w.show_all != 0;
    if def.grouping.is_none() {
        def.grouping = Some(report::Grouping::default());
    }
    let editing = w.editing.clone();
    if w.place == 1 {
        let path = String::from(w.path.trim());
        if path.is_empty() {
            w.error = Some(String::from(t("Wählen Sie eine Datei.")));
            return false;
        }
        if !plat::write_file(&path, rio::to_xml(&def).as_bytes()) {
            w.error = Some(tf("„{}“ ließ sich nicht schreiben.", &[&String::from(plat::display_name(&path))]));
            return false;
        }
        let st = &mut app.reports;
        st.files.retain(|f| f.0 != path);
        st.files.push((path.clone(), def));
        st.status = Some((0, tf("Gespeichert: {}", &[&String::from(plat::display_name(&path))])));
        st.pick = Some((name, Src::File(path)));
        return true;
    }
    let Some(o) = app.opened.as_mut() else { return true };
    let row = match &editing {
        Some(Src::Drawing(r)) => Some(r.clone()),
        _ => None,
    };
    let clash = rio::stored(&o.doc).into_iter().any(|s| Some(&s.row) != row.as_ref() && s.def.as_ref().is_ok_and(|d| d.name.to_lowercase() == name.to_lowercase()));
    if clash {
        app.reports.wizard.error = Some(String::from(t("Die Zeichnung hat schon einen Bericht dieses Namens.")));
        return false;
    }
    let label = if row.is_some() { t("Bericht ändern") } else { t("Neuer Bericht") };
    let r = edit_doc::edit(o, label, true, |tx, _| match &row {
        Some(r) => rio::replace(tx, r, &def).map(|_| r.clone()),
        None => rio::add(tx, &def),
    });
    match r {
        Ok(r) => {
            let st = &mut app.reports;
            st.status = Some((0, tf("„{}“ ist in der Zeichnung gespeichert.", &[&name])));
            st.pick = Some((name, Src::Drawing(r)));
            true
        }
        Err(e) => {
            app.reports.wizard.error = Some(tf("Der Bericht ließ sich nicht speichern: {}", &[&format!("{e:?}")]));
            false
        }
    }
}

// ---------------------------------------------------------------- the four steps

fn page_shapes(app: &mut Diagrammer, ui: &mut Ui) -> u8 {
    let w = &mut app.reports.wizard;
    ui.next_size(W, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, t("Welche Shapes sollen in den Bericht?"));
    ui.radio(ID + 0x10, t("Shapes auf allen Seiten"), &mut w.scope, 0);
    ui.radio(ID + 0x11, t("Shapes auf der aktuellen Seite"), &mut w.scope, 1);
    ui.radio(ID + 0x12, t("Ausgewählte Shapes"), &mut w.scope, 2);
    ui.spacer(4);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    let adv = ui.button(ID + 0x13, t("Erweitert...")) & EV_ACT != 0;
    let n = w.def.filters.len();
    ui.text(TS_BODY | TX_SECONDARY, &if n == 0 { String::from(t("Keine Kriterien")) } else { tf("Kriterien: {}", &[&format!("{n}")]) });
    ui.box_end();
    ui.next_size(W, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Mit Erweitert... kommen nur die Shapes in den Bericht, deren Eigenschaften die Kriterien erfüllen."));
    if adv { SUB_ADVANCED } else { SUB_NONE }
}

fn page_props(app: &mut Diagrammer, ui: &mut Ui) {
    let before = app.reports.wizard.show_all;
    let w = &mut app.reports.wizard;
    ui.next_size(W, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, t("Welche Eigenschaften soll der Bericht zeigen?"));
    ui.next_size(W, 280);
    ui.scroll_begin(ID + 0x20, SC_V | BOX_V);
    for (k, (shown, _)) in w.props.iter().enumerate().take(0x8000) {
        if let Some(c) = w.checked.get_mut(k) {
            ui.checkbox(ID_PROP + k as u32, shown, c);
        }
    }
    ui.scroll_end();
    ui.checkbox(ID + 0x21, t("Alle Eigenschaften anzeigen"), &mut w.show_all);
    ui.next_size(W, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Jede gewählte Eigenschaft wird eine Spalte des Berichts. Alle Eigenschaften: auch verborgene Shape-Daten und Benutzerzellen."));
    if w.show_all != before {
        // the list anew; what was checked stays checked
        let taken: Vec<Property> = w.props.iter().zip(&w.checked).filter(|(_, c)| **c != 0).map(|(p, _)| p.1.clone()).collect();
        if let Some(o) = app.opened.as_ref() {
            let w = &mut app.reports.wizard;
            w.props = properties(&o.doc, w.show_all != 0);
            for p in &taken {
                if !w.props.iter().any(|x| &x.1 == p) {
                    w.props.push((String::from(p.key()), p.clone()));
                }
            }
            w.checked = w.props.iter().map(|p| taken.contains(&p.1) as i32).collect();
        }
        ui.repass();
    }
}

fn page_title(app: &mut Diagrammer, ui: &mut Ui) -> u8 {
    let w = &mut app.reports.wizard;
    ui.text(TS_BODY_STRONG, t("Berichtstitel"));
    ui.next_size(W, 0);
    ui.textbox(ID + 0x30, None, &mut w.title, 256);
    ui.spacer(4);
    let mut sub = SUB_NONE;
    ui.box_begin(0, BOX_H, 8);
    if ui.button(ID + 0x31, t("Teilergebnisse...")) & EV_ACT != 0 {
        sub = SUB_TOTALS;
    }
    if ui.button(ID + 0x32, t("Sortieren...")) & EV_ACT != 0 {
        sub = SUB_SORT;
    }
    if ui.button(ID + 0x33, t("Format...")) & EV_ACT != 0 {
        sub = SUB_FORMAT;
    }
    ui.box_end();
    // what is set so far
    let d = &w.def;
    let group = d.grouping.as_ref().and_then(|g| d.field(g.field)).map(|f| String::from(f.heading()));
    let sums = d.fields.iter().filter(|f| f.summary != 0).count();
    let mut lines: Vec<String> = Vec::new();
    lines.push(match group {
        Some(g) => tf("Gruppiert nach: {}", &[&g]),
        None => String::from(t("Nicht gruppiert")),
    });
    if sums > 0 {
        lines.push(tf("Spalten mit Teilergebnissen: {}", &[&format!("{sums}")]));
    }
    let sorts: Vec<String> = d.sorts.iter().filter_map(|s| d.field(s.field).map(|f| format!("{} {}", f.heading(), if s.ascending { "↑" } else { "↓" }))).collect();
    if !sorts.is_empty() {
        lines.push(tf("Sortiert nach: {}", &[&sorts.join(", ")]));
    }
    lines.push(tf("Dezimalstellen: {}", &[&format!("{}", d.precision)]));
    for l in &lines {
        ui.next_size(W, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, l);
    }
    sub
}

fn page_name(app: &mut Diagrammer, ui: &mut Ui) -> u8 {
    let w = &mut app.reports.wizard;
    ui.text(TS_BODY_STRONG, t("Name"));
    ui.next_size(W, 0);
    ui.textbox(ID + 0x40, None, &mut w.name, 256);
    ui.text(TS_BODY_STRONG, t("Beschreibung"));
    ui.next_size(W, 0);
    ui.textbox(ID + 0x41, None, &mut w.desc, 1024);
    ui.spacer(4);
    ui.radio(ID + 0x42, t("In dieser Zeichnung speichern"), &mut w.place, 0);
    ui.radio(ID + 0x43, t("In einer Datei speichern"), &mut w.place, 1);
    let mut sub = SUB_NONE;
    if w.place == 1 {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(W - 140, 0);
        ui.textbox(ID + 0x44, None, &mut w.path, 4096);
        if ui.button(ID + 0x45, t("Durchsuchen...")) & EV_ACT != 0 {
            sub = SUB_FILE;
        }
        ui.box_end();
    }
    ui.next_size(W, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Eine Berichtsdefinition in einer Datei (.vrd) lässt sich in anderen Zeichnungen über Durchsuchen... verwenden."));
    sub
}

// ---------------------------------------------------------------- the sub-dialogs

fn headings(def: &ReportDef) -> Vec<String> {
    def.columns().iter().map(|f| String::from(f.heading())).collect()
}

fn open_sub(app: &mut Diagrammer, sub: u8) {
    let w = &mut app.reports.wizard;
    let cols: Vec<u32> = w.def.columns().iter().map(|f| f.id).collect();
    match sub {
        SUB_ADVANCED => {
            w.filters = w.def.filters.clone();
            w.case = w.def.case_sensitive as i32;
            w.f_pick = -1;
            w.f_value.clear();
        }
        SUB_TOTALS => {
            let g = w.def.grouping.clone().unwrap_or_default();
            w.group = cols.iter().position(|&id| id == g.field).map_or(0, |k| k as i32 + 1);
            w.funcs = cols.iter().map(|&id| {
                let mask = w.def.field(id).map_or(0, |f| f.summary);
                let mut v = [0; 5];
                for (i, f) in FUNCS.iter().enumerate() {
                    v[i] = (mask & f.bit() != 0) as i32;
                }
                v
            }).collect();
            w.func_col = 0;
            w.only_sums = (!g.show_items) as i32;
            w.grand = g.grand_summary as i32;
            w.dups = g.exclude_duplicates as i32;
        }
        SUB_SORT => {
            w.order = (0..cols.len()).collect();
            w.order_sel = -1;
            for i in 0..3 {
                let s = w.def.sorts.get(i);
                w.keys[i] = s.and_then(|s| cols.iter().position(|&id| id == s.field)).map_or(0, |k| k as i32 + 1);
                w.dirs[i] = s.map_or(0, |s| (!s.ascending) as i32);
            }
        }
        SUB_FORMAT => {
            w.precision = w.def.precision.min(6) as i32;
            w.units = w.def.show_units as i32;
        }
        SUB_FILE => {
            if w.path.trim().is_empty() {
                let dir = plat::arg_value("--export-dir").map(|d| format!("{}/", d.trim_end_matches(['/', '\\']))).unwrap_or_default();
                let stem = crate::reports::file_stem(if w.name.trim().is_empty() { &w.title } else { &w.name });
                w.path = format!("{dir}{stem}.vrd");
            }
        }
        _ => {}
    }
    w.sub = sub;
    if sub == SUB_FILE {
        w.file_dlg.set(1);
    } else {
        w.sub_open.set(1);
    }
}

// the end of a sub-dialog: back to the wizard
fn close_sub(app: &mut Diagrammer, ui: &mut Ui) {
    let w = &mut app.reports.wizard;
    w.sub = SUB_NONE;
    w.sub_open.set(0);
    w.open.set(1);
    ui.repass();
}

fn cond_label(c: Condition) -> &'static str {
    match c {
        Condition::Eq => "=",
        Condition::Ne => "<>",
        Condition::Gt => ">",
        Condition::Ge => ">=",
        Condition::Lt => "<",
        Condition::Le => "<=",
        Condition::Exists => t("existiert"),
        Condition::NotExists => t("existiert nicht"),
        Condition::Other(_) => "?",
    }
}

fn prop_label(w: &Wizard, p: &Property) -> String {
    w.props.iter().find(|x| &x.1 == p).map_or_else(|| String::from(p.key()), |x| x.0.clone())
}

// Erweitert...: the criteria
fn advanced(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.reports.wizard.sub_open.clone();
    if !ui.dialog_begin(ID_SUB, t("Erweitert"), &open) {
        return;
    }
    let w = &mut app.reports.wizard;
    let names: Vec<String> = w.props.iter().map(|p| p.0.clone()).collect();
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let conds: Vec<&str> = CONDS.iter().map(|c| cond_label(*c)).collect();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.box_begin(0, BOX_V, 2);
    ui.text(TS_BODY, t("Eigenschaft"));
    ui.next_size(200, 0);
    ui.combobox(ID_SUB + 1, &mut w.f_prop, &refs);
    ui.box_end();
    ui.box_begin(0, BOX_V, 2);
    ui.text(TS_BODY, t("Bedingung"));
    ui.next_size(130, 0);
    ui.combobox(ID_SUB + 2, &mut w.f_cond, &conds);
    ui.box_end();
    let needs = CONDS.get(w.f_cond.max(0) as usize).is_none_or(|c| c.has_value());
    ui.box_begin(0, BOX_V, 2);
    ui.text(TS_BODY, t("Wert"));
    ui.next_size(120, 0);
    ui.next_flags(if needs { 0 } else { ST_DISABLED });
    ui.textbox(ID_SUB + 3, None, &mut w.f_value, 256);
    ui.box_end();
    ui.box_end();
    let mut add = false;
    ui.box_begin(0, BOX_H, 8);
    if ui.button(ID_SUB + 4, t("Hinzufügen")) & EV_ACT != 0 {
        add = true;
    }
    let has = w.f_pick >= 0 && (w.f_pick as usize) < w.filters.len();
    ui.next_flags(if has { 0 } else { ST_DISABLED });
    if ui.button(ID_SUB + 5, t("Löschen")) & EV_ACT != 0 && has {
        w.filters.remove(w.f_pick as usize);
        w.f_pick = -1;
        ui.repass();
    }
    ui.next_flags(if w.filters.is_empty() { ST_DISABLED } else { 0 });
    if ui.button(ID_SUB + 6, t("Alle löschen")) & EV_ACT != 0 {
        w.filters.clear();
        w.f_pick = -1;
        ui.repass();
    }
    ui.box_end();
    ui.text(TS_BODY, t("Festgelegte Kriterien"));
    let lines: Vec<String> = w.filters.iter().map(|f| {
        let v = if f.condition.has_value() { format!(" {}", f.value) } else { String::new() };
        format!("{} {}{v}", prop_label(w, &f.property), cond_label(f.condition))
    }).collect();
    ui.next_size(W, 120);
    let n = lines.len() as i32;
    let mut sel = w.f_pick.clamp(-1, n - 1);
    let (a, b) = ui.list_begin(ID_SUB + 0x1000, &mut sel, n, 24);
    for i in a..b {
        ui.list_item(i, &lines[i as usize]);
    }
    ui.list_end(&mut sel);
    w.f_pick = sel;
    ui.checkbox(ID_SUB + 7, t("Groß-/Kleinschreibung beachten"), &mut w.case);
    if add {
        if let Some((_, p)) = w.props.get(w.f_prop.max(0) as usize) {
            let c = CONDS.get(w.f_cond.max(0) as usize).copied().unwrap_or(Condition::Eq);
            let id = w.filters.iter().map(|f| f.id).max().unwrap_or(0) + 1;
            let mut f = Filter::new(id, p.clone(), c, if c.has_value() { w.f_value.trim() } else { "" });
            f.data_type = match p {
                Property::Builtin(b) if b.is_length() => 2,
                _ => app.opened.as_ref().map_or(0, |o| data_type(&o.doc, p)),
            };
            app.reports.wizard.filters.push(f);
            app.reports.wizard.f_value.clear();
        }
        ui.repass();
    }
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_NONE {
        return;
    }
    if r == DLG_PRIMARY {
        let w = &mut app.reports.wizard;
        w.def.filters = w.filters.clone();
        w.def.case_sensitive = w.case != 0;
    }
    close_sub(app, ui);
}

// Teilergebnisse...: group, the summaries per column, what shows
fn totals(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.reports.wizard.sub_open.clone();
    if !ui.dialog_begin(ID_SUB, t("Teilergebnisse"), &open) {
        return;
    }
    let w = &mut app.reports.wizard;
    let heads = headings(&w.def);
    let mut choices: Vec<&str> = alloc::vec![t("(keine)")];
    choices.extend(heads.iter().map(|s| s.as_str()));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.text(TS_BODY, t("Gruppieren nach"));
    ui.next_size(220, 0);
    ui.combobox(ID_SUB + 1, &mut w.group, &choices);
    ui.box_end();
    ui.spacer(4);
    ui.text(TS_BODY_STRONG, t("Teilergebnisse je Spalte"));
    ui.box_begin(0, BOX_H, 12);
    let n = heads.len() as i32;
    let mut sel = w.func_col.clamp(-1, n - 1);
    ui.next_size(220, 150);
    let (a, b) = ui.list_begin(ID_SUB + 0x1000, &mut sel, n, 24);
    for i in a..b {
        ui.list_item(i, &heads[i as usize]);
    }
    ui.list_end(&mut sel);
    w.func_col = sel;
    ui.box_begin(0, BOX_V, 4);
    let fnames = [t("Summe"), t("Mittelwert"), t("Anzahl"), t("Minimum"), t("Maximum")];
    if let Some(v) = w.funcs.get_mut(sel.max(0) as usize).filter(|_| sel >= 0) {
        for (i, f) in fnames.iter().enumerate() {
            ui.checkbox(ID_SUB + 0x100 + i as u32, f, &mut v[i]);
        }
    }
    ui.box_end();
    ui.box_end();
    ui.spacer(4);
    ui.radio(ID_SUB + 3, t("Zeilen und Teilergebnisse anzeigen"), &mut w.only_sums, 0);
    ui.radio(ID_SUB + 4, t("Nur Teilergebnisse anzeigen"), &mut w.only_sums, 1);
    ui.checkbox(ID_SUB + 5, t("Gesamtergebnis anzeigen"), &mut w.grand);
    ui.checkbox(ID_SUB + 6, t("Doppelte Zeilen ausschließen"), &mut w.dups);
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_NONE {
        return;
    }
    if r == DLG_PRIMARY {
        let w = &mut app.reports.wizard;
        let cols: Vec<u32> = w.def.columns().iter().map(|f| f.id).collect();
        for (k, id) in cols.iter().enumerate() {
            let mask = w.funcs.get(k).map_or(0, |v| FUNCS.iter().enumerate().filter(|(i, _)| v[*i] != 0).fold(0, |m, (_, f)| m | f.bit()));
            if let Some(f) = w.def.fields.iter_mut().find(|f| f.id == *id) {
                f.summary = mask;
            }
        }
        let g = w.def.grouping.get_or_insert_with(Default::default);
        g.field = if w.group > 0 { cols.get(w.group as usize - 1).copied().unwrap_or(0) } else { 0 };
        g.show_items = w.only_sums == 0;
        g.grand_summary = w.grand != 0;
        g.exclude_duplicates = w.dups != 0;
    }
    close_sub(app, ui);
}

// Sortieren...: the columns' order, three sort keys
fn sorting(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.reports.wizard.sub_open.clone();
    if !ui.dialog_begin(ID_SUB, t("Sortieren"), &open) {
        return;
    }
    let w = &mut app.reports.wizard;
    let heads = headings(&w.def);
    ui.text(TS_BODY_STRONG, t("Spaltenreihenfolge"));
    ui.box_begin(0, BOX_H, 12);
    let n = w.order.len() as i32;
    let mut sel = w.order_sel.clamp(-1, n - 1);
    ui.next_size(260, 150);
    let (a, b) = ui.list_begin(ID_SUB + 0x1000, &mut sel, n, 24);
    for i in a..b {
        let k = w.order[i as usize];
        ui.list_item(i, heads.get(k).map_or("", |s| s.as_str()));
    }
    ui.list_end(&mut sel);
    ui.box_begin(0, BOX_V, 8);
    let (up, down) = (sel > 0 && sel < n, sel >= 0 && sel + 1 < n);
    ui.next_size(120, 0);
    ui.next_flags(if up { 0 } else { ST_DISABLED });
    if ui.button(ID_SUB + 1, t("Nach oben")) & EV_ACT != 0 && up {
        w.order.swap(sel as usize, sel as usize - 1);
        sel -= 1;
        ui.repass();
    }
    ui.next_size(120, 0);
    ui.next_flags(if down { 0 } else { ST_DISABLED });
    if ui.button(ID_SUB + 2, t("Nach unten")) & EV_ACT != 0 && down {
        w.order.swap(sel as usize, sel as usize + 1);
        sel += 1;
        ui.repass();
    }
    ui.box_end();
    ui.box_end();
    w.order_sel = sel;
    ui.spacer(4);
    ui.text(TS_BODY_STRONG, t("Sortieren nach"));
    let mut choices: Vec<&str> = alloc::vec![t("(keine)")];
    choices.extend(heads.iter().map(|s| s.as_str()));
    let dirs = [t("Aufsteigend"), t("Absteigend")];
    for i in 0..3 {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(220, 0);
        ui.combobox(ID_SUB + 0x20 + i as u32 * 2, &mut w.keys[i], &choices);
        ui.next_size(140, 0);
        ui.combobox(ID_SUB + 0x21 + i as u32 * 2, &mut w.dirs[i], &dirs);
        ui.box_end();
    }
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_NONE {
        return;
    }
    if r == DLG_PRIMARY {
        let w = &mut app.reports.wizard;
        let cols: Vec<u32> = w.def.columns().iter().map(|f| f.id).collect();
        for (pos, &k) in w.order.iter().enumerate() {
            if let Some(f) = cols.get(k).and_then(|id| w.def.fields.iter_mut().find(|f| f.id == *id)) {
                f.order = pos as u32 + 1;
            }
        }
        w.def.sorts.clear();
        for i in 0..3 {
            if let Some(&id) = (w.keys[i] > 0).then(|| cols.get(w.keys[i] as usize - 1)).flatten() {
                if !w.def.sorts.iter().any(|s| s.field == id) {
                    let sid = w.def.sorts.len() as u32 + 1;
                    w.def.sorts.push(Sort::new(sid, id, w.dirs[i] == 0));
                }
            }
        }
    }
    close_sub(app, ui);
}

// Format...: decimals and units
fn format_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.reports.wizard.sub_open.clone();
    if !ui.dialog_begin(ID_SUB, t("Format"), &open) {
        return;
    }
    let w = &mut app.reports.wizard;
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.text(TS_BODY, t("Dezimalstellen"));
    ui.next_size(90, 0);
    ui.number_box(ID_SUB + 1, &mut w.precision, &Range::new(0, 6, 1, 0));
    ui.box_end();
    ui.checkbox(ID_SUB + 2, t("Einheiten anzeigen"), &mut w.units);
    ui.next_size(320, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Gilt für alle Zahlen des Berichts; Längen zeigen ihre Einheit."));
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_NONE {
        return;
    }
    if r == DLG_PRIMARY {
        let w = &mut app.reports.wizard;
        w.def.precision = w.precision.clamp(0, 6) as u32;
        w.def.show_units = w.units != 0;
    }
    close_sub(app, ui);
}

// Durchsuchen... of the last step: where the .vrd file goes
fn file_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let w = &mut app.reports.wizard;
    let dlg = w.file_dlg.clone();
    let was = dlg.get() != 0;
    let exts = [t("vrd=Berichtsdefinition")];
    if ui.filedlg(ID_FILE, FD_SAVE, &dlg, &mut w.path, 4096, &exts) & EV_ACT != 0 {
        if let Some(p) = ui.filedlg_paths().into_iter().next() {
            w.path = p;
        }
    }
    if was && dlg.get() == 0 {
        close_sub(app, ui);
    }
}
