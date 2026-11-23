// data_ask -- (JD18, SP-13) Shape-Daten beim Ablegen abfragen: the shape data fields marked
// "Beim Ablegen fragen" (Verify) open the dialog Shape-Daten after a shape was dropped from a
// stencil, duplicated, pasted or copied by Ctrl + Shift + drag
//
//   AskState                  the shapes waiting, the one asked for, its fields being typed
//   after_drop(app, ui, s)    (stencils_drop.rs, edit_cmds::paste, edit_canvas's copy drag) the
//                             new shapes: those with fields to ask for wait their turn
//   dialog(app, ui)           every pass: the dialog for one shape at a time
//
// The fields: the Property rows with Verify = TRUE that are not Invisible, ordered by SortKey;
// the inputs by type as in the pane Shape-Daten (pane_data.rs): text, number, currency,
// duration and variable list a field, fixed list a list, Boolean a check box, date the date
// picker; the prompt is the field's QuickInfo.  OK writes the values changed through the
// formula engine (Engine::user_set) in one undo step "Shape-Daten"; Abbrechen (Esc) keeps the
// master's values.  Several new shapes: one dialog per shape in turn (only the shapes that have
// such fields; a group is asked for itself, its members are not).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Input;
use diagram_core::{CellAddr, Document, EditError, RowKey, SecKey, ShapeRef, SheetRef, Unit, Value};
use ui::i18n::t;
use ui::*;

use crate::pane_data::{civil, formula_for, serial, value_text};
use crate::{edit_doc, Diagrammer};

const ID: u32 = 0x7C70_0000;

/// a field asked for
#[derive(Clone)]
struct Field {
    name: String,
    label: String,
    prompt: String,
    kind: i32,
    /// a fixed list's entries
    items: Vec<String>,
    text: String,
    flag: i32,
    date: (i32, u32, u32),
    pick: i32,
}

/// the question's state
pub(crate) struct AskState {
    open: Shared<i32>,
    /// the shapes still to ask for
    queue: Vec<ShapeRef>,
    shape: Option<ShapeRef>,
    fields: Vec<Field>,
    /// the values as the shape had them
    start: Vec<Field>,
    /// the drawing (Opened::serial)
    serial: u32,
    /// 2: the first field takes the keyboard next pass
    focus: u8,
}

impl Default for AskState {
    fn default() -> AskState {
        AskState { open: Shared::new(0), queue: Vec::new(), shape: None, fields: Vec::new(), start: Vec::new(), serial: 0, focus: 0 }
    }
}

// the fields of a shape to ask for, by SortKey
fn fields_of(doc: &Document, s: ShapeRef) -> Vec<Field> {
    let mut out: Vec<(String, Field)> = Vec::new();
    for r in doc.shape_data(SheetRef::Shape(s)) {
        let Some(name) = r.name() else { continue };
        if !r.flag("Verify") || r.flag("Invisible") {
            continue;
        }
        let kind = r.num("Type").unwrap_or(0.0) as i32;
        let format = r.text("Format").unwrap_or("");
        let text = value_text(doc, r.get("Value").value());
        let items: Vec<String> = if kind == 1 { format.split(';').map(|x| String::from(x.trim())).filter(|x| !x.is_empty()).collect() } else { Vec::new() };
        let pick = items.iter().position(|x| *x == text).map_or(-1, |k| k as i32);
        let date = match (kind, r.num("Value")) {
            (5, Some(d)) => civil(d),
            _ => (2026, 1, 1),
        };
        let label = r.text("Label").filter(|l| !l.is_empty()).unwrap_or(name);
        let mut prompt = String::from(r.text("Prompt").unwrap_or(""));
        if kind == 4 && !format.is_empty() {
            prompt = format!("{}\n{}", prompt, format.replace(';', ", "));
        }
        let field = Field { name: String::from(name), label: String::from(label), prompt: String::from(prompt.trim()), kind, items, text, flag: r.flag("Value") as i32, date, pick };
        out.push((String::from(r.text("SortKey").unwrap_or("")), field));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter().map(|x| x.1).collect()
}

/// after shapes were dropped, duplicated or pasted: ask for their fields marked "ask on drop"
pub(crate) fn after_drop(app: &mut Diagrammer, ui: &mut Ui, shapes: &[ShapeRef]) {
    let Some(o) = app.opened.as_ref() else { return };
    let ask: Vec<ShapeRef> = shapes.iter().copied().filter(|&s| !fields_of(&o.doc, s).is_empty()).collect();
    if ask.is_empty() {
        return;
    }
    let serial = o.serial;
    let st = &mut app.dask;
    if st.serial != serial {
        st.queue.clear();
        st.open.set(0);
        st.serial = serial;
    }
    st.queue.extend(ask);
    if st.open.get() == 0 {
        next(app, ui);
    }
}

// the next shape that still exists and still has fields to ask for
fn next(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &mut app.dask;
    st.shape = None;
    st.open.set(0);
    let Some(o) = app.opened.as_ref() else {
        st.queue.clear();
        return;
    };
    while !st.queue.is_empty() {
        let s = st.queue.remove(0);
        if o.doc.shape(s).is_none() {
            continue;
        }
        let fields = fields_of(&o.doc, s);
        if fields.is_empty() {
            continue;
        }
        st.shape = Some(s);
        st.start = fields.clone();
        st.fields = fields;
        st.focus = 2;
        st.open.set(1);
        break;
    }
    ui.repass();
}

/// every pass: the dialog when open
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.dask.open.clone();
    if open.get() == 0 {
        return;
    }
    // the drawing changed or the shape is gone (undo): no question
    let title = match (app.opened.as_ref(), app.dask.shape) {
        (Some(o), Some(s)) if o.serial == app.dask.serial && o.doc.shape(s).is_some() => o.doc.shape(s).map(|x| x.name()).unwrap_or_default(),
        _ => {
            next(app, ui);
            return;
        }
    };
    if !ui.dialog_begin(ID, t("Shape-Daten"), &open) {
        return;
    }
    let st = &mut app.dask;
    ui.next_size(360, 0);
    ui.box_begin(ID + 1, BOX_V, 6);
    ui.text(TS_BODY_STRONG, &title);
    for (k, f) in st.fields.iter_mut().enumerate() {
        let fid = ID + 0x10 + k as u32 * 4;
        ui.text(TS_CAPTION | TX_SECONDARY, &f.label);
        ui.next_size(-1, 0);
        match f.kind {
            3 => {
                ui.checkbox(fid, &f.label, &mut f.flag);
            }
            1 => {
                let items: Vec<&str> = f.items.iter().map(|s| s.as_str()).collect();
                ui.combobox(fid, &mut f.pick, &items);
            }
            5 => {
                let (mut y, mut m, mut d) = f.date;
                ui.date_picker(fid, &mut y, &mut m, &mut d);
                f.date = (y, m, d);
            }
            _ => {
                ui.textbox(fid, None, &mut f.text, 1024);
            }
        }
        if !f.prompt.is_empty() {
            ui.tooltip(&f.prompt);
        }
        if k == 0 && st.focus > 0 {
            if st.focus == 2 {
                ui.focus(fid);
            }
            st.focus -= 1;
        }
    }
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        apply(app);
    }
    if r != DLG_NONE {
        next(app, ui);
    }
}

// the values changed into the shape: one undo step
fn apply(app: &mut Diagrammer) {
    let st = &app.dask;
    let Some(s) = st.shape else { return };
    let changed: Vec<Field> = st
        .fields
        .iter()
        .zip(st.start.iter())
        .filter(|(f, b)| match f.kind {
            3 => f.flag != b.flag,
            1 => f.pick != b.pick && f.pick >= 0,
            5 => f.date != b.date,
            _ => f.text != b.text,
        })
        .map(|(f, _)| f.clone())
        .collect();
    if changed.is_empty() {
        return;
    }
    let Some(o) = app.opened.as_mut() else { return };
    let _ = edit_doc::edit(o, t("Shape-Daten"), true, |tx, eng| {
        let sheet = SheetRef::Shape(s);
        let sk = SecKey::one(tx.intern("Property"));
        let c = tx.intern("Value");
        for f in &changed {
            let rk = RowKey::Named(tx.intern(&f.name));
            let a = CellAddr::row(sk, rk, c);
            let formula = match f.kind {
                3 => String::from(if f.flag != 0 { "TRUE" } else { "FALSE" }),
                1 => formula_for(1, f.items.get(f.pick as usize).map_or("", |x| x.as_str())),
                _ => formula_for(f.kind, &f.text),
            };
            let input = if f.kind == 5 { Input::Value(Value::Num(serial(f.date.0, f.date.1, f.date.2)), Some(Unit::Date)) } else { Input::Formula(&formula) };
            match eng.user_set(tx, sheet, &a, input) {
                Ok(_) | Err(EditError::Guarded) => {}
                Err(e) => return Err(e),
            }
            // a value typed over linked data is the user's (a refresh keeps it)
            diagram_core::data::user_edited(tx, sheet, &f.name)?;
        }
        Ok(())
    });
}
