// pane_data -- (JD8 rest) the task pane Shape-Daten (AB-03) and the dialog Shape-Daten definieren
// (DA-02)
//
//   pane(app, ui, id)          the fields (Prop rows) of the selection: label and value, the
//                              input by type -- text, number, currency, duration: a field;
//                              fixed list: a list; variable list: a field (the list's values in
//                              its QuickInfo); Boolean: a check box; date: the date picker.
//                              Several shapes selected: only the fields they all have (by row
//                              name); a change goes to every one of them.  The prompt is the
//                              field's QuickInfo; fields marked invisible are left out.
//   define_open(app, ui)       Shape-Daten definieren: the dialog with the fields of the first
//                              selected shape -- label, name (new fields only), type, format,
//                              value, prompt, sort key, ask on drop, invisible; new, delete.
//                              OK writes them to every selected shape (one undo step).
//   define_dialog(app, ui)     every pass
//
// Visio's cells (the Property section; names and codes are Visio's for the file's sake): Label,
// Prompt, Type (0 text, 1 fixed list, 2 number, 3 Boolean, 4 variable list, 5 date, 6 duration,
// 7 currency), Format (a list's values separated by ';', else a format picture), Value, SortKey,
// Invisible, Verify (ask when the shape is dropped).  A date is a number of days since
// 1899-12-30 with the unit DATE; a list value is written as the text chosen; Boolean TRUE / FALSE.
// Values go through the engine (Engine::user_set), so text fields and formulas that show them
// follow.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Input;
use diagram_core::{CellAddr, Document, EditError, RowKey, SecKey, ShapeRef, SheetRef, Unit, Value};
use ui::i18n::{self, t};
use ui::*;

use crate::{edit_canvas, edit_doc, Diagrammer};

const ID_DLG: u32 = 0x5C10_0000;

// i18n
const TYPES: [&str; 8] = ["Zeichenfolge", "Feste Liste", "Zahl", "Boolesch", "Variable Liste", "Datum", "Dauer", "Währung"];

/// a field in the dialog
#[derive(Clone, Default)]
struct DefRow {
    /// the row's name when the shape had it (None: new)
    orig: Option<String>,
    name: String,
    label: String,
    kind: i32,
    format: String,
    value: String,
    prompt: String,
    sort: String,
    ask: i32,
    hidden: i32,
    deleted: bool,
    value_changed: bool,
}

/// the pane's field being typed into and the dialog
#[derive(Default)]
pub(crate) struct DataState {
    field: u32,
    buf: String,
    dlg: Shared<i32>,
    rows: Vec<DefRow>,
    cur: i32,
    shapes: Vec<ShapeRef>,
}

// ---------------------------------------------------------------- reading

/// a field as the pane shows it
struct Field {
    name: String,
    label: String,
    prompt: String,
    kind: i32,
    format: String,
    value: String,
    flag: bool,
    days: Option<f64>,
}

// a value as text: a text as it is, a number with the region's decimal sign
fn value_text(doc: &Document, v: Value) -> String {
    match v {
        Value::Text(s) => String::from(doc.str(s)),
        Value::Num(x) => {
            let s = if x == (x as i64) as f64 { format!("{}", x as i64) } else { format!("{x}") };
            let dec = i18n::locale().decimal();
            s.replace('.', dec.encode_utf8(&mut [0; 4]))
        }
        Value::Absent => String::new(),
    }
}

/// days since 1899-12-30 -> (year, month, day)
pub(crate) fn civil(days: f64) -> (i32, u32, u32) {
    // days from 1970-01-01, then Howard Hinnant's civil_from_days
    let z = (days as i64) - 25569 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// (year, month, day) -> days since 1899-12-30
pub(crate) fn serial(y: i32, m: u32, d: u32) -> f64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468 + 25569) as f64
}

fn fields_of(doc: &Document, s: ShapeRef) -> Vec<Field> {
    let sheet = SheetRef::Shape(s);
    let mut out: Vec<(f64, String, Field)> = Vec::new();
    for r in doc.shape_data(sheet) {
        let Some(name) = r.name() else { continue };
        if r.flag("Invisible") {
            continue;
        }
        let kind = r.num("Type").unwrap_or(0.0) as i32;
        let v = r.get("Value").value();
        let days = (kind == 5).then(|| r.num("Value")).flatten();
        let value = match (kind, days) {
            (5, Some(d)) => {
                let (y, m, dd) = civil(d);
                format!("{dd:02}.{m:02}.{y}")
            }
            _ => value_text(doc, v),
        };
        let label = r.text("Label").filter(|l| !l.is_empty()).unwrap_or(name);
        let sort = r.text("SortKey").map(String::from).unwrap_or_default();
        out.push((
            0.0,
            sort,
            Field {
                name: String::from(name),
                label: String::from(label),
                prompt: String::from(r.text("Prompt").unwrap_or("")),
                kind,
                format: String::from(r.text("Format").unwrap_or("")),
                value,
                flag: r.flag("Value"),
                days,
            },
        ));
    }
    // Visio orders by SortKey, the rows' order otherwise (a stable sort keeps it)
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out.into_iter().map(|x| x.2).collect()
}

// ---------------------------------------------------------------- the pane

/// the pane
pub(crate) fn pane(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let shapes = crate::format_cells::selection(app);
    let fields: Vec<Field> = match (app.opened.as_ref(), shapes.first()) {
        (Some(o), Some(&first)) => {
            let mut f = fields_of(&o.doc, first);
            // several shapes: the fields every one has
            for &s in &shapes[1..] {
                let names: Vec<String> = fields_of(&o.doc, s).into_iter().map(|f| f.name).collect();
                f.retain(|x| names.iter().any(|n| n.eq_ignore_ascii_case(&x.name)));
            }
            f
        }
        _ => Vec::new(),
    };
    ui.next_size(-1, -1);
    ui.scroll_begin(id, SC_V | BOX_V | pad(10));
    if fields.is_empty() {
        let (title, msg) = if shapes.is_empty() {
            (t("Keine Shape-Daten"), t("Wählen Sie ein Shape aus, um seine Daten zu sehen."))
        } else {
            (t("Keine Shape-Daten"), t("Das Shape hat keine Shape-Daten."))
        };
        ui.empty_state(crate::icons::JD_SHAPE_DATA, title, msg);
    }
    let mut change: Option<(String, i32, Input<'static>, String)> = None;
    for (k, f) in fields.iter().enumerate() {
        let fid = id + 0x100 + k as u32 * 4;
        ui.text(TS_CAPTION | TX_SECONDARY, &f.label);
        ui.next_size(-1, 0);
        match f.kind {
            3 => {
                let mut v = f.flag as i32;
                if ui.checkbox(fid, &f.label, &mut v) & EV_ACT != 0 {
                    change = Some((f.name.clone(), 3, Input::Formula(if v != 0 { "TRUE" } else { "FALSE" }), String::new()));
                }
            }
            1 => {
                let items: Vec<&str> = f.format.split(';').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                let mut sel = items.iter().position(|x| *x == f.value).map_or(-1, |k| k as i32);
                let before = sel;
                ui.combobox(fid, &mut sel, &items);
                if sel != before && sel >= 0 {
                    change = Some((f.name.clone(), 1, Input::Formula(""), String::from(items[sel as usize])));
                }
            }
            5 => {
                let (mut y, mut m, mut d) = f.days.map_or((2026, 1, 1), civil);
                let before = (y, m, d);
                ui.date_picker(fid, &mut y, &mut m, &mut d);
                if (y, m, d) != before {
                    change = Some((f.name.clone(), 5, Input::Value(Value::Num(serial(y, m, d)), Some(Unit::Date)), String::new()));
                }
            }
            _ => {
                let focused = ui.focused() == fid;
                if focused && app.data.field != fid {
                    app.data.field = fid;
                    app.data.buf = f.value.clone();
                }
                let mut s = if focused { app.data.buf.clone() } else { f.value.clone() };
                let ev = ui.textbox(fid, None, &mut s, 1024);
                if focused {
                    app.data.buf = s.clone();
                }
                if ev & EV_COMMIT != 0 && s != f.value {
                    app.data.field = 0;
                    change = Some((f.name.clone(), f.kind, Input::Formula(""), s));
                }
                if ev & EV_COMMIT != 0 && focused {
                    ui.focus(crate::panes::ID_CANVAS);
                }
            }
        }
        let tip = if f.kind == 4 && !f.format.is_empty() { format!("{}\n{}", f.prompt, f.format.replace(';', ", ")) } else { f.prompt.clone() };
        if !tip.trim().is_empty() {
            ui.tooltip(tip.trim());
        }
        ui.spacer(4);
    }
    if !shapes.is_empty() {
        ui.spacer(6);
        ui.next_size(-1, 28);
        ui.next_flags(ST_SUBTLE | ST_ALIGN_START);
        if ui.button(id + 0x10, &format!("{}…", crate::commands::label(crate::commands::SHAPE_DATA_DEFINE))) & EV_ACT != 0 {
            define_open(app, ui);
        }
    }
    ui.scroll_end();
    if let Some((name, kind, input, text)) = change {
        set_value(app, ui, &shapes, &name, kind, input, &text);
    }
}

// a value typed for a field of type `kind` as a formula
fn formula_for(kind: i32, text: &str) -> String {
    let t = text.trim();
    let number = t.replace(',', ".");
    match kind {
        2 | 6 | 7 if number.parse::<f64>().is_ok() => number,
        3 => String::from(if matches!(t.to_ascii_lowercase().as_str(), "1" | "true" | "wahr" | "ja" | "yes") { "TRUE" } else { "FALSE" }),
        _ => format!("\"{}\"", text.replace('"', "\"\"")),
    }
}

// one field's value into every shape that has it (one undo step)
fn set_value(app: &mut Diagrammer, ui: &mut Ui, shapes: &[ShapeRef], name: &str, kind: i32, input: Input<'static>, text: &str) {
    let Some(o) = app.opened.as_mut() else { return };
    let formula = formula_for(kind, text);
    let _ = edit_doc::edit(o, t("Shape-Daten"), true, |tx, eng| {
        let sk = SecKey::one(tx.intern("Property"));
        let rk = RowKey::Named(tx.intern(name));
        let c = tx.intern("Value");
        for &s in shapes {
            let sheet = SheetRef::Shape(s);
            if !tx.doc().has_row(sheet, sk, rk) {
                continue;
            }
            let a = CellAddr::row(sk, rk, c);
            let inp = match &input {
                Input::Value(v, u) => Input::Value(*v, *u),
                _ => Input::Formula(&formula),
            };
            match eng.user_set(tx, sheet, &a, inp) {
                Ok(_) | Err(EditError::Guarded) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    });
    ui.repass();
}

// ---------------------------------------------------------------- Shape-Daten definieren

/// Shape-Daten definieren: the dialog with the first selected shape's fields
pub(crate) fn define_open(app: &mut Diagrammer, ui: &mut Ui) {
    let shapes = crate::format_cells::selection(app);
    let Some(o) = app.opened.as_ref() else { return };
    let Some(&first) = shapes.first() else { return };
    let doc = &o.doc;
    let mut rows = Vec::new();
    for r in doc.shape_data(SheetRef::Shape(first)) {
        let Some(name) = r.name() else { continue };
        let kind = r.num("Type").unwrap_or(0.0) as i32;
        let value = match kind {
            5 => r.num("Value").map(|d| {
                let (y, m, dd) = civil(d);
                format!("{dd:02}.{m:02}.{y}")
            }),
            3 => Some(String::from(if r.flag("Value") { "TRUE" } else { "FALSE" })),
            _ => Some(value_text(doc, r.get("Value").value())),
        };
        rows.push(DefRow {
            orig: Some(String::from(name)),
            name: String::from(name),
            label: String::from(r.text("Label").unwrap_or("")),
            kind,
            format: String::from(r.text("Format").unwrap_or("")),
            value: value.unwrap_or_default(),
            prompt: String::from(r.text("Prompt").unwrap_or("")),
            sort: String::from(r.text("SortKey").unwrap_or("")),
            ask: r.flag("Verify") as i32,
            hidden: r.flag("Invisible") as i32,
            deleted: false,
            value_changed: false,
        });
    }
    let st = &mut app.data;
    st.rows = rows;
    st.cur = if st.rows.is_empty() { -1 } else { 0 };
    st.shapes = shapes;
    st.dlg.set(1);
    ui.repass();
}

// a name for a new field: Row_1, Row_2 ... (Visio's)
fn new_name(rows: &[DefRow]) -> String {
    let mut k = 1;
    loop {
        let n = format!("Row_{k}");
        if !rows.iter().any(|r| r.name.eq_ignore_ascii_case(&n)) {
            return n;
        }
        k += 1;
    }
}

/// every pass: the dialog Shape-Daten definieren
pub(crate) fn define_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.data.dlg.clone();
    if !ui.dialog_begin(ID_DLG, crate::commands::label(crate::commands::SHAPE_DATA_DEFINE), &dlg) {
        return;
    }
    let st = &mut app.data;
    ui.next_size(560, 0);
    ui.box_begin(ID_DLG + 1, BOX_V, 8);
    // the fields: a list with Neu, Löschen
    let shown: Vec<usize> = (0..st.rows.len()).filter(|&k| !st.rows[k].deleted).collect();
    let labels: Vec<String> = shown.iter().map(|&k| {
        let r = &st.rows[k];
        if r.label.is_empty() { r.name.clone() } else { format!("{} ({})", r.label, r.name) }
    }).collect();
    let labels_ref: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let mut pick = shown.iter().position(|&k| k as i32 == st.cur).map_or(-1, |p| p as i32);
    ui.text(TS_BODY_STRONG, t("Felder"));
    ui.next_size(-1, 120);
    let (first, end) = ui.list_begin(ID_DLG + 0x40, &mut pick, labels_ref.len() as i32, 28);
    for i in first..end {
        ui.list_item(i, labels_ref[i as usize]);
    }
    ui.list_end(&mut pick);
    if pick >= 0 {
        st.cur = shown[pick as usize] as i32;
    }
    ui.box_begin(0, BOX_H, 8);
    if ui.button(ID_DLG + 3, t("Neu")) & EV_ACT != 0 {
        let name = new_name(&st.rows);
        st.rows.push(DefRow { name, value_changed: true, ..DefRow::default() });
        st.cur = st.rows.len() as i32 - 1;
        ui.repass();
    }
    ui.next_flags(if st.cur >= 0 { 0 } else { ST_DISABLED });
    if ui.button(ID_DLG + 4, t("Löschen")) & EV_ACT != 0 && st.cur >= 0 {
        st.rows[st.cur as usize].deleted = true;
        st.cur = (0..st.rows.len()).find(|&k| !st.rows[k].deleted).map_or(-1, |k| k as i32);
        ui.repass();
    }
    ui.box_end();
    // the chosen field's properties
    if let Some(r) = usize::try_from(st.cur).ok().and_then(|k| st.rows.get_mut(k)) {
        let row = |ui: &mut Ui, label: &str| {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.next_size(170, 0);
            ui.text(TS_BODY, label);
            ui.next_size(-1, 0);
        };
        row(ui, t("Bezeichnung"));
        ui.textbox(ID_DLG + 0x10, None, &mut r.label, 256);
        ui.box_end();
        row(ui, t("Name"));
        ui.next_flags(if r.orig.is_some() { ST_DISABLED } else { 0 });
        ui.textbox(ID_DLG + 0x11, None, &mut r.name, 64);
        ui.box_end();
        row(ui, t("Typ"));
        let types: Vec<&str> = TYPES.iter().map(|s| t(s)).collect();
        ui.combobox(ID_DLG + 0x12, &mut r.kind, &types);
        ui.box_end();
        row(ui, t("Format"));
        ui.textbox(ID_DLG + 0x13, None, &mut r.format, 1024);
        ui.box_end();
        row(ui, t("Wert"));
        if ui.textbox(ID_DLG + 0x14, None, &mut r.value, 1024) & EV_ACT != 0 {
            r.value_changed = true;
        }
        ui.box_end();
        row(ui, t("Eingabeaufforderung"));
        ui.textbox(ID_DLG + 0x15, None, &mut r.prompt, 1024);
        ui.box_end();
        row(ui, t("Sortierschlüssel"));
        ui.textbox(ID_DLG + 0x16, None, &mut r.sort, 64);
        ui.box_end();
        ui.checkbox(ID_DLG + 0x17, t("Beim Ablegen fragen"), &mut r.ask);
        ui.checkbox(ID_DLG + 0x18, t("Ausgeblendet"), &mut r.hidden);
        if r.kind == 1 || r.kind == 4 {
            ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Listen: die Einträge im Format durch Semikolon getrennt."));
        }
    }
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        define_apply(app, ui);
    }
    if r != DLG_NONE {
        app.data.dlg.set(0);
        ui.repass();
    }
}

// a field name Visio accepts: letters, digits, '_' (no blanks), not empty
fn clean_name(s: &str) -> String {
    let n: String = s.trim().chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if n.is_empty() { String::from("Row_1") } else { n }
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

// the dialog's fields into every chosen shape (one undo step)
fn define_apply(app: &mut Diagrammer, ui: &mut Ui) {
    let rows = app.data.rows.clone();
    let shapes = app.data.shapes.clone();
    let Some(o) = app.opened.as_mut() else { return };
    let _ = edit_doc::edit(o, crate::commands::label(crate::commands::SHAPE_DATA_DEFINE), true, |tx, eng| {
        let sk = SecKey::one(tx.intern("Property"));
        for &s in &shapes {
            let sheet = SheetRef::Shape(s);
            for d in &rows {
                let name = match &d.orig {
                    Some(n) => n.clone(),
                    None => clean_name(&d.name),
                };
                let rk = RowKey::Named(tx.intern(&name));
                let has = tx.doc().has_row(sheet, sk, rk);
                if d.deleted {
                    if has {
                        tx.delete_row(sheet, sk, rk)?;
                    }
                    continue;
                }
                if !has {
                    tx.add_row(sheet, sk, Some(rk), None)?;
                }
                let cells: [(&str, String); 7] = [
                    ("Label", quoted(&d.label)),
                    ("Prompt", quoted(&d.prompt)),
                    ("Type", format!("{}", d.kind.clamp(0, 7))),
                    ("Format", quoted(&d.format)),
                    ("SortKey", quoted(&d.sort)),
                    ("Verify", String::from(if d.ask != 0 { "TRUE" } else { "FALSE" })),
                    ("Invisible", String::from(if d.hidden != 0 { "TRUE" } else { "FALSE" })),
                ];
                for (cell, f) in &cells {
                    let a = CellAddr::row(sk, rk, tx.intern(cell));
                    match eng.user_set(tx, sheet, &a, Input::Formula(f)) {
                        Ok(_) | Err(EditError::Guarded) => {}
                        Err(e) => return Err(e),
                    }
                }
                if d.value_changed || !has {
                    let a = CellAddr::row(sk, rk, tx.intern("Value"));
                    let v = d.value.trim();
                    let res = if d.kind == 5 {
                        // a date typed as d.m.y (or y-m-d)
                        let parts: Vec<i64> = v.split(|c: char| c == '.' || c == '-' || c == '/').filter_map(|p| p.trim().parse().ok()).collect();
                        match parts.as_slice() {
                            [a1, b1, c1] if *a1 > 31 => eng.user_set(tx, sheet, &a, Input::Value(Value::Num(serial(*a1 as i32, *b1 as u32, *c1 as u32)), Some(Unit::Date))).map(|_| ()),
                            [a1, b1, c1] => eng.user_set(tx, sheet, &a, Input::Value(Value::Num(serial(*c1 as i32, *b1 as u32, *a1 as u32)), Some(Unit::Date))).map(|_| ()),
                            _ if v.is_empty() => Ok(()),
                            _ => Err(EditError::Invalid),
                        }
                    } else if v.is_empty() && d.kind != 0 {
                        Ok(())
                    } else {
                        eng.user_set(tx, sheet, &a, Input::Formula(&formula_for(d.kind, &d.value))).map(|_| ())
                    };
                    match res {
                        Ok(_) | Err(EditError::Guarded) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Ok(())
    });
    ui.repass();
}

/// Shape-Daten definieren wants shapes selected
pub(crate) fn available(app: &Diagrammer) -> bool {
    app.opened.is_some() && !edit_canvas::selected(app).is_empty()
}
