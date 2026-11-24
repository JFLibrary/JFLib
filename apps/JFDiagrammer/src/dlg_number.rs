// dlg_number -- (JD21, AS-11, RA-09) Ansicht › Add-Ons › Shapes nummerieren: the dialog, and
// Beim Ablegen weiterzählen (diagram_core::tools::number)
//
//   NumberDialog              its state (kept for the session: the next run starts with the
//                             settings of the last)
//   open(app, ui)             the dialog: the selection when shapes are selected, else the page
//   dialog(app, ui)           every pass; OK numbers the shapes in one undo step and, with
//                             "Beim Ablegen weiterzählen", lets the page number dropped shapes
//                             from the next number on (User cells of the page, the same step)
//   toggle_drop(app, ui)      the command Beim Ablegen weiterzählen: off when on, else the
//                             dialog with the box ticked
//   drop_on(app)              its check mark: the page shown numbers dropped shapes
//   available(app, id)        a drawing is open (and has a shape to number)
//
// Which shapes: 2-D shapes that are no container, list, callout or guide (tools::plain);
// connectors, lines and dimensions are left out.  The order Auswahlreihenfolge needs the
// selection; for the whole page it is the order in rows.  The dialog shows how a number looks.

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::tools::{self, number};
use diagram_core::{Owner, ShapeRef};
use ui::i18n::t;
use ui::*;

use crate::commands::{self, *};
use crate::{edit_doc, structures, Diagrammer};

const ID: u32 = 0x7EB0_0000;

// the orders in the list
// i18n
const ORDERS: [&str; 4] = [
    "Auswahlreihenfolge",
    "Von links nach rechts, dann von oben nach unten",
    "Von oben nach unten, dann von links nach rechts",
    "Entlang der Verbinder",
];

/// the dialog's state
pub(crate) struct NumberDialog {
    pub open: Shared<i32>,
    /// 0 the selection, 1 the page
    scope: i32,
    /// ORDERS
    order: i32,
    start: i32,
    step: i32,
    prefix: String,
    /// 0 before, 1 after, 2 instead of the text
    place: i32,
    /// Beim Ablegen weiterzählen (a check box: 0 / 1)
    on_drop: i32,
}

impl Default for NumberDialog {
    fn default() -> NumberDialog {
        NumberDialog { open: Shared::new(0), scope: 0, order: 1, start: 1, step: 1, prefix: String::new(), place: 0, on_drop: 0 }
    }
}

// the shapes that may be numbered: the selection's or the page's
fn shapes(app: &Diagrammer, selection: bool) -> Vec<ShapeRef> {
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return Vec::new() };
    let doc = &o.doc;
    if selection {
        structures::ordered(app).into_iter().filter(|&s| tools::plain(doc, s)).collect()
    } else {
        doc.top_shapes(Owner::Page(page)).filter(|&s| tools::plain(doc, s)).collect()
    }
}

/// whether a command of this file can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    match id {
        NUMBER_SHAPES => !shapes(app, false).is_empty(),
        NUMBER_ON_DROP => app.opened.is_some() && app.model.page.is_some(),
        _ => true,
    }
}

/// Shapes nummerieren: the dialog
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    let selected = !shapes(app, true).is_empty();
    let on = drop_on(app) as i32;
    let d = &mut app.addons.number;
    d.scope = if selected { 0 } else { 1 };
    if !selected && d.order == 0 {
        d.order = 1;
    }
    d.on_drop = on;
    app.addons.number.open.set(1);
    ui.repass();
}

/// the page shown numbers shapes as they are dropped
pub(crate) fn drop_on(app: &Diagrammer) -> bool {
    match (app.opened.as_ref(), app.model.page) {
        (Some(o), Some(page)) => number::on_drop(&o.doc, page).is_some(),
        _ => false,
    }
}

/// Beim Ablegen weiterzählen: off when on (one undo step), else the dialog with it ticked
pub(crate) fn toggle_drop(app: &mut Diagrammer, ui: &mut Ui) {
    if drop_on(app) {
        let Some(page) = app.model.page else { return };
        if let Some(o) = app.opened.as_mut() {
            let _ = edit_doc::edit(o, commands::label(NUMBER_ON_DROP), true, |tx, _| number::keep_on_drop(tx, page, None));
        }
        ui.repass();
        return;
    }
    open(app, ui);
    app.addons.number.on_drop = 1;
}

fn row(ui: &mut Ui, s: &str) {
    ui.next_size(150, 0);
    ui.text(TS_BODY, s);
}

/// the dialog (every pass)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.addons.number.open.clone();
    if !ui.dialog_begin(ID, commands::label(NUMBER_SHAPES), &open) {
        return;
    }
    let any_sel = !shapes(app, true).is_empty();
    let d = &mut app.addons.number;
    if !any_sel {
        d.scope = 1;
    }
    ui.next_size(460, 0);
    ui.box_begin(ID + 1, BOX_V, 6);
    ui.text(TS_BODY_STRONG, t("Bereich"));
    ui.box_begin(0, BOX_H, 16);
    ui.next_flags(if any_sel { 0 } else { ST_DISABLED });
    ui.radio(ID + 2, t("Ausgewählte Shapes"), &mut d.scope, 0);
    ui.radio(ID + 3, t("Alle Shapes der Seite"), &mut d.scope, 1);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Reihenfolge"));
    let orders: Vec<&str> = ORDERS.iter().map(|s| t(s)).collect();
    ui.next_size(300, 0);
    ui.combobox(ID + 4, &mut d.order, &orders);
    ui.box_end();
    if d.scope == 1 && d.order == 0 {
        ui.next_size(460, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Für die ganze Seite gilt die Reihenfolge von links nach rechts."));
    }
    ui.text(TS_BODY_STRONG, t("Nummern"));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Startnummer"));
    ui.next_size(110, 0);
    ui.number_box(ID + 5, &mut d.start, &Range::new(-99_999, 99_999, 1, 0));
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Schrittweite"));
    ui.next_size(110, 0);
    ui.number_box(ID + 6, &mut d.step, &Range::new(-9_999, 9_999, 1, 0));
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Präfix"));
    ui.next_size(200, 0);
    ui.textbox(ID + 7, None, &mut d.prefix, 32);
    ui.box_end();
    ui.text(TS_BODY_STRONG, t("Nummer im Text"));
    ui.box_begin(0, BOX_H, 16);
    ui.radio(ID + 8, t("Vor dem Text"), &mut d.place, 0);
    ui.radio(ID + 9, t("Nach dem Text"), &mut d.place, 1);
    ui.radio(ID + 10, t("Text ersetzen"), &mut d.place, 2);
    ui.box_end();
    let sample = number::label(&d.prefix, d.start as i64);
    let shown = match d.place {
        0 => alloc::format!("{sample} {}", t("Text")),
        1 => alloc::format!("{} {sample}", t("Text")),
        _ => sample,
    };
    ui.next_size(460, 0);
    ui.text(TS_CAPTION | TX_SECONDARY, &ui::i18n::tf("Beispiel: {}", &[&shown]));
    ui.checkbox(ID + 11, commands::label(NUMBER_ON_DROP), &mut d.on_drop);
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        apply(app, ui);
    }
    if r != DLG_NONE {
        app.addons.number.open.set(0);
        ui.repass();
    }
}

// OK: the numbers, the page's numbering on drop (one undo step)
fn apply(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(page) = app.model.page else { return };
    let d = &app.addons.number;
    let selection = d.scope == 0;
    let by = match d.order {
        0 if selection => number::Order::Selection,
        2 => number::Order::Columns,
        3 => number::Order::Connections,
        _ => number::Order::Rows,
    };
    let n = number::Numbering {
        start: d.start as i64,
        step: d.step as i64,
        prefix: d.prefix.clone(),
        place: match d.place {
            1 => number::Place::After,
            2 => number::Place::Instead,
            _ => number::Place::Before,
        },
    };
    let on_drop = d.on_drop != 0;
    let list = shapes(app, selection);
    let Some(o) = app.opened.as_mut() else { return };
    let order = number::order(&o.doc, &list, by);
    let _ = edit_doc::edit(o, commands::label(NUMBER_SHAPES), true, |tx, eng| {
        let next = number::number(tx, eng, &order, &n)?;
        let keep = number::Numbering { start: next, ..n.clone() };
        number::keep_on_drop(tx, page, on_drop.then_some(&keep))
    });
    ui.repass();
}
