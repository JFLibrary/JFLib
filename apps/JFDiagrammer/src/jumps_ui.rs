// jumps_ui -- (JD21, CN-06) line jumps in the app: Entwurf › Verbinder › Linienübergänge
// anzeigen (LINE_JUMPS) and the dialog Verbinder-Verhalten (CONN_BEHAVIOR, from a connector's
// context menu); the page's options are in Seite einrichten › Layout und Routing (layout_ui.rs)
//
//   JumpsState                the dialog's values and connectors; the codes pages had before
//                             Linienübergänge anzeigen turned them off
//   shown(app) / toggle(app, ui)   LINE_JUMPS: checked while the page shown has jumps
//                             (LineJumpCode not 0); off writes 0, on the code the page had before
//                             in this session, else 1 (horizontal lines, the value of Visio's
//                             files); one undo step
//   available(app, id)        LINE_JUMPS needs a page, CONN_BEHAVIOR a selected dynamic connector
//   menu_states(app, ui)      every pass: Verbinder-Verhalten shows in the shape's menu only
//                             while a connector is selected
//   open(app, ui) / dialog(app, ui)   Verbinder-Verhalten (Visio: Entwicklertools › Verhalten ›
//                             Verbinder › Linienübergänge): add (ConLineJumpCode), style
//                             (ConLineJumpStyle), horizontal and vertical direction
//                             (ConLineJumpDirX / Y) for every selected connector, one undo step
//   style_names(page)         the jump styles' names: the page's (Bogen .. 7 Seiten, LineJumpStyle
//                             1 .. 9) or the connector's (Seitenvorgabe first, 0 .. 9)
//
// What the codes mean and how the jumps are drawn: lib/diagram_core/src/render/jumps.rs.  The
// drawing follows by itself: a page cell builds the page anew (viewer::sync), a connector's
// cells make it and the connectors it crosses anew (render::affected).

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{CellAddr, Document, PageRef, ShapeRef, SheetRef, Value};
use ui::i18n::{t, tf};
use ui::*;

use crate::commands::{self, CONN_BEHAVIOR, LINE_JUMPS};
use crate::{edit_canvas, edit_doc, Diagrammer};

const ID: u32 = 0x7921_0000;

/// the dialog Verbinder-Verhalten and the codes Linienübergänge anzeigen turned off
pub(crate) struct JumpsState {
    pub open: Shared<i32>,
    conns: Vec<ShapeRef>,
    /// ConLineJumpCode, ConLineJumpStyle, ConLineJumpDirX, ConLineJumpDirY as chosen
    code: i32,
    style: i32,
    dir: [i32; 2],
    /// (document serial, page, LineJumpCode before it was turned off)
    before: Vec<(u32, PageRef, f64)>,
}

impl Default for JumpsState {
    fn default() -> JumpsState {
        JumpsState { open: Shared::new(0), conns: Vec::new(), code: 0, style: 0, dir: [0; 2], before: Vec::new() }
    }
}

fn num(doc: &Document, s: SheetRef, cell: &str) -> Option<f64> {
    doc.sym(cell).and_then(|c| doc.resolve(s, &CellAddr::single(c)).num()).filter(|v| v.is_finite())
}

/// the page's LineJumpCode (absent: 1, as the "No Style" of Visio's files)
pub(crate) fn page_code(doc: &Document, page: PageRef) -> f64 {
    num(doc, SheetRef::Page(page), "LineJumpCode").unwrap_or(1.0)
}

/// Linienübergänge anzeigen is on: the page shown has jumps
pub(crate) fn shown(app: &Diagrammer) -> bool {
    match (app.opened.as_ref(), app.model.page) {
        (Some(o), Some(p)) => page_code(&o.doc, p) != 0.0,
        _ => false,
    }
}

// the selected dynamic connectors
fn connectors(app: &Diagrammer) -> Vec<ShapeRef> {
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    edit_canvas::selected(app).into_iter().filter(|&s| diagram_core::glue::is_connector(&o.doc, s)).collect()
}

/// whether LINE_JUMPS / CONN_BEHAVIOR can run (others: true)
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    match id {
        LINE_JUMPS => app.opened.is_some() && app.model.page.is_some(),
        CONN_BEHAVIOR => !connectors(app).is_empty(),
        _ => true,
    }
}

/// every pass after commands::states: Verbinder-Verhalten only in a connector's menu
pub(crate) fn menu_states(app: &Diagrammer, ui: &mut Ui) {
    if connectors(app).is_empty() {
        ui.cmd_state(CONN_BEHAVIOR, CF_DISABLED | CF_HIDDEN);
    }
}

/// Linienübergänge anzeigen: the page's LineJumpCode 0, or back to what it was
pub(crate) fn toggle(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(page) = app.model.page else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let now = page_code(&o.doc, page);
    let serial = o.serial;
    let st = &mut app.jumps;
    let code = if now != 0.0 {
        st.before.retain(|b| !(b.0 == serial && b.1 == page));
        st.before.push((serial, page, now));
        0.0
    } else {
        st.before.iter().rev().find(|b| b.0 == serial && b.1 == page).map_or(1.0, |b| b.2)
    };
    let sheet = SheetRef::Page(page);
    let _ = edit_doc::edit(o, commands::label(LINE_JUMPS), true, |tx, eng| {
        let a = tx.single("LineJumpCode");
        tx.set_constant(sheet, &a, Value::Num(code), None)?;
        eng.recalc(tx, &[sheet])
    });
    ui.repass();
}

/// the jump styles' names: the page's 1 .. 9 (Bogen .. 7 Seiten), or the connector's 0 .. 9
/// (Seitenvorgabe first)
pub(crate) fn style_names(page: bool) -> Vec<String> {
    let mut v: Vec<String> = Vec::with_capacity(10);
    if !page {
        v.push(String::from(t("Seitenvorgabe")));
    }
    for s in [t("Bogen"), t("Lücke"), t("Quadrat")] {
        v.push(String::from(s));
    }
    for n in ["2", "3", "4", "5", "6", "7"] {
        let s = tf("{} Seiten|Linienübergang", &[n]);
        v.push(String::from(s.split('|').next().unwrap_or(&s)));
    }
    v
}

/// Verbinder-Verhalten: the dialog with the first selected connector's values
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    let conns = connectors(app);
    let Some(&first) = conns.first() else { return };
    let Some(o) = app.opened.as_ref() else { return };
    let s = SheetRef::Shape(first);
    let code = |cell: &str, max: f64| num(&o.doc, s, cell).unwrap_or(0.0).clamp(0.0, max) as i32;
    let st = &mut app.jumps;
    st.code = code("ConLineJumpCode", 4.0);
    st.style = code("ConLineJumpStyle", 9.0);
    st.dir = [code("ConLineJumpDirX", 2.0), code("ConLineJumpDirY", 2.0)];
    st.conns = conns;
    st.open.set(1);
    ui.repass();
}

/// the dialog (every pass)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.jumps.open.clone();
    if !ui.dialog_begin(ID, commands::label(CONN_BEHAVIOR), &open) {
        return;
    }
    let st = &mut app.jumps;
    let row = |ui: &mut Ui, s: &str| {
        ui.next_size(200, 0);
        ui.text(TS_BODY, s);
    };
    ui.next_size(460, 0);
    ui.box_begin(ID + 1, BOX_V, 8);
    ui.text(TS_BODY_STRONG, t("Linienübergänge"));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Linienübergänge hinzufügen"));
    ui.next_size(240, 0);
    ui.combobox(ID + 2, &mut st.code, &[t("Seitenvorgabe"), t("Nie"), t("Immer"), t("Der andere Verbinder springt"), t("Kein Verbinder springt")]);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Stil der Linienübergänge"));
    let names = style_names(false);
    let names: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    ui.next_size(240, 0);
    ui.combobox(ID + 3, &mut st.style, &names);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Waagerechte Richtung"));
    ui.next_size(240, 0);
    ui.combobox(ID + 4, &mut st.dir[0], &[t("Seitenvorgabe"), crate::tc("Nach oben|Linienübergang"), crate::tc("Nach unten|Linienübergang")]);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Senkrechte Richtung"));
    ui.next_size(240, 0);
    ui.combobox(ID + 5, &mut st.dir[1], &[t("Seitenvorgabe"), crate::tc("Nach links|Linienübergang"), crate::tc("Nach rechts|Linienübergang")]);
    ui.box_end();
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        apply(app);
    }
    if r != DLG_NONE {
        app.jumps.open.set(0);
        ui.repass();
    }
}

// the dialog's values into every chosen connector (one undo step; nothing when nothing changed)
fn apply(app: &mut Diagrammer) {
    let st = &app.jumps;
    let conns = st.conns.clone();
    let want = [("ConLineJumpCode", st.code), ("ConLineJumpStyle", st.style), ("ConLineJumpDirX", st.dir[0]), ("ConLineJumpDirY", st.dir[1])];
    let Some(o) = app.opened.as_mut() else { return };
    let conns: Vec<ShapeRef> = conns.into_iter().filter(|&s| o.doc.shape(s).is_some()).collect();
    let differs = conns.iter().any(|&s| want.iter().any(|&(c, v)| num(&o.doc, SheetRef::Shape(s), c).unwrap_or(0.0) != v as f64));
    if !differs {
        return;
    }
    let _ = edit_doc::edit(o, commands::label(CONN_BEHAVIOR), true, |tx, eng| {
        for &s in &conns {
            let sheet = SheetRef::Shape(s);
            for &(c, v) in &want {
                let a = tx.single(c);
                tx.set_constant(sheet, &a, Value::Num(v as f64), None)?;
            }
            eng.recalc(tx, &[sheet])?;
        }
        Ok(())
    });
}
