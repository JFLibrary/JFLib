// dlg_field -- (JD8 rest) the dialog Feld (TX-06, RE-13; Einfügen › Feld, Textwerkzeuge › Feld):
// a field into the text, at the caret when the text is being edited, else at the end of the
// text of every selected shape (diagram_core::field::insert)
//
//   FieldDlg                    the dialog's state: where the field goes, category, field,
//                               format, the custom formula
//   open(app, ui)               FIELD: where it goes (cmds_text::target), the dialog
//   dialog(app, ui)             every pass: the categories, their fields, the formats shown
//                               with the field's value now; OK puts the field in (one undo
//                               step) and, when the text was being edited, editing goes on
//                               behind the field
//
// Categories and formulas (the order of Visio's dialog; the functions Visio's fields use):
//   Shape-Daten          Prop.<Zeile> of the first shape
//   Datum/Uhrzeit        DOCCREATION(), NOW(), DOCLASTEDIT(), DOCLASTPRINT() (U="DATE")
//   Dokumentinfo         CREATOR(), DESCRIPTION(), DIRECTORY(), FILENAME(), KEYWORDS(),
//                        SUBJECT(), TITLE(), MANAGER(), COMPANY(), CATEGORY(), HYPERLINKBASE()
//   Seiteninfo           PAGENAME(), PAGENUMBER(), PAGECOUNT()
//   Geometrie            Width, Height (the page's unit), Angle (degrees)
//   Objektinfo           ID(), NAME(), MASTERNAME(), DATA1() .. DATA3()
//   Benutzerdefinierte Zellen   User.<Zeile>
//   Eigene Formel        the user's formula (a leading '=' is left out)
// Formats are pictures of Visio's FORMAT (field.rs); the list shows each with the value the
// field has now.  The date pictures follow the language (dd.MM.yyyy, M/d/yyyy, dd/MM/yyyy).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::field::{self, Kind, Shown, Spec};
use diagram_core::formula::{Outcome, Val};
use diagram_core::{CellAddr, ShapeRef, SheetRef, Unit, Value};
use ui::i18n::t;
use ui::*;

use crate::Diagrammer;

const ID: u32 = 0x7820_0000;
const ID_CATS: u32 = ID + 0x100;
const ID_ITEMS: u32 = ID + 0x200;

/// a field the dialog offers
#[derive(Clone, Debug)]
struct Item {
    label: String,
    formula: String,
    kind: Kind,
    unit: Option<Unit>,
}

/// the dialog's state
#[derive(Default)]
pub(crate) struct FieldDlg {
    pub open: Shared<i32>,
    shapes: Vec<ShapeRef>,
    caret: Option<usize>,
    resume: bool,
    cat: i32,
    item: i32,
    fmt: i32,
    formula: String,
    error: bool,
}

/// the categories (Visio's order)
fn categories() -> [&'static str; 8] {
    [t("Shape-Daten"), t("Datum/Uhrzeit"), t("Dokumentinfo"), t("Seiteninfo"), t("Geometrie"), t("Objektinfo"), t("Benutzerdefinierte Zellen"), t("Eigene Formel")]
}

/// Feld (FIELD): the dialog for the text being edited or the selected shapes
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    let tg = crate::cmds_text::target(app, ui);
    if tg.shapes.is_empty() {
        return;
    }
    let d = &mut app.txt.field;
    d.shapes = tg.shapes;
    d.caret = tg.caret;
    d.resume = tg.resume;
    d.item = 0;
    d.fmt = 0;
    d.error = false;
    d.open.set(1);
    ui.repass();
}

fn item(label: &str, formula: &str, kind: Kind, unit: Option<Unit>) -> Item {
    Item { label: String::from(label), formula: String::from(formula), kind, unit }
}

// the kind and unit of a row's Value cell (shape data, user cells)
fn kind_of(doc: &diagram_core::Document, sheet: SheetRef, addr: &CellAddr) -> (Kind, Option<Unit>) {
    let r = doc.resolve(sheet, addr);
    match (r.value(), r.unit()) {
        (Value::Text(_), _) | (_, Some(Unit::Str)) => (Kind::Text, None),
        (_, Some(Unit::Date)) => (Kind::Date, Some(Unit::Date)),
        (_, Some(Unit::Currency)) => (Kind::Currency, Some(Unit::Currency)),
        (_, u) => (Kind::Number, u),
    }
}

// the fields of a category for the first shape
fn items(app: &Diagrammer, cat: i32) -> Vec<Item> {
    let d = &app.txt.field;
    let (Some(o), Some(&s)) = (app.opened.as_ref(), d.shapes.first()) else { return Vec::new() };
    let doc = &o.doc;
    let sheet = SheetRef::Shape(s);
    let date = Some(Unit::Date);
    match cat {
        0 | 6 => {
            let rows = if cat == 0 { doc.shape_data(sheet) } else { doc.user_cells(sheet) };
            let prefix = if cat == 0 { "Prop" } else { "User" };
            rows.iter()
                .filter_map(|r| {
                    let name = r.name()?;
                    let label = r.text("Label").filter(|l| !l.is_empty() && cat == 0).unwrap_or(name);
                    let (kind, unit) = doc.sym("Value").map_or((Kind::Text, None), |v| kind_of(doc, sheet, &r.addr(v)));
                    Some(item(label, &format!("{prefix}.{name}"), kind, unit))
                })
                .collect()
        }
        1 => alloc::vec![
            item(t("Erstellungsdatum/-uhrzeit"), "DOCCREATION()", Kind::Date, date),
            item(t("Aktuelles Datum/aktuelle Uhrzeit"), "NOW()", Kind::Date, date),
            item(t("Letzte Bearbeitung"), "DOCLASTEDIT()", Kind::Date, date),
            item(t("Druckdatum/-uhrzeit"), "DOCLASTPRINT()", Kind::Date, date),
        ],
        2 => [
            (t("Ersteller"), "CREATOR()"),
            (t("Beschreibung"), "DESCRIPTION()"),
            (t("Verzeichnis"), "DIRECTORY()"),
            (t("Dateiname"), "FILENAME()"),
            (t("Stichwörter"), "KEYWORDS()"),
            (t("Betreff"), "SUBJECT()"),
            (t("Titel"), "TITLE()"),
            (t("Manager"), "MANAGER()"),
            (t("Firma"), "COMPANY()"),
            (t("Kategorie"), "CATEGORY()"),
            (t("Hyperlinkbasis"), "HYPERLINKBASE()"),
        ]
        .iter()
        .map(|(l, f)| item(l, f, Kind::Text, None))
        .collect(),
        3 => alloc::vec![
            item(t("Name"), "PAGENAME()", Kind::Text, None),
            item(t("Seitenzahl"), "PAGENUMBER()", Kind::Number, None),
            item(t("Seitenanzahl"), "PAGECOUNT()", Kind::Number, None),
        ],
        4 => {
            let u = Some(if crate::edit_canvas::metric(app) { Unit::Millimeters } else { Unit::Inches });
            alloc::vec![
                item(t("Breite"), "Width", Kind::Number, u),
                item(t("Höhe"), "Height", Kind::Number, u),
                item(t("Winkel"), "Angle", Kind::Number, Some(Unit::Degrees)),
            ]
        }
        5 => alloc::vec![
            item(t("ID"), "ID()", Kind::Number, None),
            item(t("Name"), "NAME()", Kind::Text, None),
            item(t("Master-Shape"), "MASTERNAME()", Kind::Text, None),
            item(t("Daten 1"), "DATA1()", Kind::Text, None),
            item(t("Daten 2"), "DATA2()", Kind::Text, None),
            item(t("Daten 3"), "DATA3()", Kind::Text, None),
        ],
        _ => {
            let f = d.formula.trim().trim_start_matches('=').trim();
            if f.is_empty() {
                return Vec::new();
            }
            let kind = match value_of(app, f) {
                Some(Val::Num(_)) => Kind::Number,
                _ => Kind::Text,
            };
            alloc::vec![item(t("Eigene Formel"), f, kind, None)]
        }
    }
}

// the value of a formula for the first shape now (None: it cannot be computed here)
fn value_of(app: &Diagrammer, formula: &str) -> Option<Val> {
    let o = app.opened.as_ref()?;
    let &s = app.txt.field.shapes.first()?;
    let doc = &o.doc;
    let mut eng = diagram_core::formula::Engine::new();
    let addr = CellAddr::single(doc.sym("Width")?);
    match eng.evaluate_text(doc, SheetRef::Shape(s), &addr, formula) {
        Ok(Outcome::Value(v)) => Some(v.plain().clone()),
        _ => None,
    }
}

/// the format pictures for a kind of value and the current language
fn pictures(kind: Kind, unit: Option<Unit>) -> Vec<String> {
    let col = crate::cmds_text::lang_col();
    let s = |x: &str| String::from(x);
    match kind {
        Kind::Text => alloc::vec![s("@"), s("@+"), s("@-")],
        Kind::Date => {
            let short = crate::cmds_text::date_picture(col);
            let long = ["dddd, d. MMMM yyyy", "dddd, MMMM d, yyyy", "dddd, d 'de' MMMM 'de' yyyy", "dddd d MMMM yyyy", "dddd d MMMM yyyy"][col.min(4)];
            let time = if col == 1 { "h:mm tt" } else { "HH:mm" };
            alloc::vec![
                format!("{{{{{short}}}}}"),
                format!("{{{{{long}}}}}"),
                format!("{{{{{time}}}}}"),
                format!("{{{{{short} {time}}}}}"),
                s("{{yyyy-MM-dd}}"),
                s("{{MMMM yyyy}}"),
            ]
        }
        _ if unit.is_some_and(|u| !matches!(u, Unit::Currency | Unit::Percent)) => {
            alloc::vec![s("0 u"), s("0.0 u"), s("0.00 u"), s("0.00"), s("#,##0.00 u")]
        }
        Kind::Currency => alloc::vec![s("#,##0.00"), s("0")],
        _ => alloc::vec![s(""), s("0"), s("0.00"), s("#,##0")],
    }
}

// the label of a picture: the value in it, else a name for it
fn picture_label(app: &Diagrammer, v: &Option<Val>, it: &Item, pic: &str) -> String {
    let Some(o) = app.opened.as_ref() else { return String::from(pic) };
    let shown = match v {
        Some(Val::Num(x)) => Shown::Num(*x, it.unit),
        Some(Val::Str(s)) | Some(Val::Font(s)) => Shown::Text(s),
        _ => Shown::Nothing,
    };
    let text = field::format(&shown, pic, &o.doc.fields);
    if !text.is_empty() {
        return text;
    }
    match pic {
        "" => String::from(t("Allgemein")),
        "@" => String::from(t("Normal")),
        "@+" => String::from(t("Großbuchstaben")),
        "@-" => String::from(t("Kleinbuchstaben")),
        p => String::from(p),
    }
}

/// the dialog (every pass)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.txt.field.open.clone();
    if !ui.dialog_begin(ID, t("Feld"), &open) {
        return;
    }
    let cats = categories();
    ui.box_begin(0, BOX_H, 12);
    // the categories
    ui.box_begin(0, BOX_V, 4);
    ui.text(TS_BODY_STRONG, t("Kategorie"));
    let mut cat = app.txt.field.cat;
    let old = cat;
    ui.next_size(210, 236);
    let (a, b) = ui.list_begin(ID_CATS, &mut cat, cats.len() as i32, 28);
    for i in a..b {
        ui.list_item(i, cats[i as usize]);
    }
    ui.list_end(&mut cat);
    ui.box_end();
    if cat != old {
        app.txt.field.item = 0;
        app.txt.field.fmt = 0;
        app.txt.field.error = false;
    }
    app.txt.field.cat = cat.clamp(0, 7);
    // the fields, or the custom formula
    ui.box_begin(0, BOX_V, 4);
    let list = items(app, app.txt.field.cat);
    if app.txt.field.cat == 7 {
        ui.text(TS_BODY_STRONG, t("Formel"));
        ui.next_size(260, 0);
        let mut f = core::mem::take(&mut app.txt.field.formula);
        if ui.textbox(ID + 10, Some(t("z. B. Width*2")), &mut f, 512) & EV_ACT != 0 {
            app.txt.field.error = false;
        }
        app.txt.field.formula = f;
        if app.txt.field.error {
            ui.next_size(260, 0);
            ui.text(TS_CAPTION | TX_WRAP, t("Die Formel lässt sich nicht lesen."));
        }
    } else {
        ui.text(TS_BODY_STRONG, t("Feldname"));
        let names: Vec<&str> = list.iter().map(|i| i.label.as_str()).collect();
        let mut it = app.txt.field.item;
        let old = it;
        ui.next_size(260, 236);
        let (a, b) = ui.list_begin(ID_ITEMS, &mut it, names.len() as i32, 28);
        for i in a..b {
            ui.list_item(i, names[i as usize]);
        }
        ui.list_end(&mut it);
        if names.is_empty() {
            ui.next_size(260, 0);
            ui.text(TS_CAPTION | TX_WRAP, t("Das Shape hat keine Einträge dieser Art."));
        }
        if it != old {
            app.txt.field.fmt = 0;
        }
        app.txt.field.item = it;
    }
    ui.box_end();
    ui.box_end();
    // the format, shown with the value now
    let chosen = list.get(app.txt.field.item.max(0) as usize).cloned();
    if let Some(it) = &chosen {
        let v = value_of(app, &it.formula);
        let pics = pictures(it.kind, it.unit);
        let labels: Vec<String> = pics.iter().map(|p| picture_label(app, &v, it, p)).collect();
        let labels: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        ui.spacer(4);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(120, 0);
        ui.text(TS_BODY, t("Format"));
        let mut fmt = app.txt.field.fmt.clamp(0, pics.len() as i32 - 1);
        ui.next_size(350, 0);
        ui.combobox(ID + 20, &mut fmt, &labels);
        app.txt.field.fmt = fmt;
        ui.box_end();
    }
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        match chosen {
            Some(it) if diagram_core::formula::parse(&it.formula).is_ok() => {
                let pics = pictures(it.kind, it.unit);
                let pic = pics.get(app.txt.field.fmt.max(0) as usize).cloned().unwrap_or_default();
                put(app, ui, &it, &pic);
            }
            Some(_) => {
                // a formula that does not parse: the dialog stays
                app.txt.field.error = true;
                app.txt.field.open.set(1);
                ui.repass();
                return;
            }
            None => {}
        }
    }
    if r != DLG_NONE {
        app.txt.field.open.set(0);
        ui.repass();
    }
}

// the field into the shapes (one undo step); editing goes on behind it when it was on
fn put(app: &mut Diagrammer, ui: &mut Ui, it: &Item, pic: &str) {
    let d = &app.txt.field;
    let (shapes, caret, resume) = (d.shapes.clone(), d.caret, d.resume);
    let Some(o) = app.opened.as_mut() else { return };
    let spec = Spec { formula: &it.formula, format: pic, unit: it.unit, kind: it.kind };
    let r = crate::edit_doc::edit(o, t("Feld einfügen"), true, |tx, eng| {
        let mut first = None;
        for (k, &s) in shapes.iter().enumerate() {
            let ix = field::insert(tx, eng, s, if k == 0 { caret } else { None }, &spec)?;
            if k == 0 {
                first = Some(ix);
            }
            crate::edit_canvas::follow(tx, eng, s.owner, &[s])?;
        }
        Ok(first)
    });
    if let (Ok(Some(ix)), true, Some(&s), Some(c)) = (r, resume, shapes.first(), caret) {
        let len = app.opened.as_ref().map_or(0, |o| field::shown(&o.doc, SheetRef::Shape(s), ix).chars().count());
        crate::cmds_text::resume(app, s, c + len);
    }
    ui.repass();
}
