// layout_ui -- (JD10) layout behaviour per shape (CN-17) and the page's layout and routing
// (PG-11)
//
//   LayoutState                 the dialog Layout-Verhalten and the values of Seite einrichten ›
//                               Layout und Routing while the dialog is open
//   owns / available / run      the command Layout-Verhalten (the shape's menu)
//   dialog(app, ui)             Layout-Verhalten, every pass: placement (ObjType: decide
//                               automatically 0, lay out and route around 1, do not 4), the
//                               connectors' routing style (ShapeRouteStyle), plowing
//                               (ShapePlowCode), fixed codes (ShapeFixedCode: do not move in a
//                               layout 1, ignore connection points 0x20)
//   load(app, page) / tab(app, ui) / apply(app, page, tx)   the register Layout und Routing of
//                               Seite einrichten: routing style (RouteStyle), line jumps
//                               (LineJumpCode, LineJumpStyle), the space between shapes
//                               (AvenueSizeX/Y), the space between lines and shapes
//                               (LineToNodeX/Y), move shapes away on drop (PlowCode), split
//                               connectors (PageShapeSplit)
//
// The codes are Visio's (learn.microsoft.com, Shape Layout and Page Layout sections; research in
// jd10-strukturen.md).  The router of diagram_core::glue reads RouteStyle and ShapeRouteStyle
// (straight for 2 and 16, else right angles), LineToNodeX/Y as its clearance, and leaves shapes
// with ObjType 4 out of its obstacles; the other codes are kept for Visio and for the page
// layout of P2 (CN-13).

use alloc::vec::Vec;

use diagram_core::{CellAddr, Document, PageRef, ShapeRef, SheetRef, Unit, Value};
use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::edit_canvas;
use crate::edit_doc;
use crate::Diagrammer;

const ID_DLG: u32 = 0x5C00;

/// the routing styles in RouteStyle / ShapeRouteStyle order (0 .. 16)
// i18n
pub(crate) static ROUTE_STYLES: [&str; 17] = [
    "Standard|Routing", "Rechtwinklig", "Gerade", "Organigramm (von oben nach unten)", "Organigramm (von links nach rechts)", "Flussdiagramm (von oben nach unten)",
    "Flussdiagramm (von links nach rechts)", "Baum (von oben nach unten)", "Baum (von links nach rechts)", "Netzwerk", "Organigramm (von unten nach oben)",
    "Organigramm (von rechts nach links)", "Flussdiagramm (von unten nach oben)", "Flussdiagramm (von rechts nach links)", "Baum (von unten nach oben)",
    "Baum (von rechts nach links)", "Mittig zu Mittig",
];

/// the dialog's and the register's values
pub(crate) struct LayoutState {
    pub dlg: Shared<i32>,
    /// Layout-Verhalten: the shapes and their values
    pub shapes: Vec<ShapeRef>,
    pub placement: i32,
    pub route: i32,
    pub plow: i32,
    pub fixed: i32,
    pub ignore_points: i32,
    /// Layout und Routing of a page
    pub page: Option<PageRef>,
    pub page_route: i32,
    pub jumps: i32,
    pub jump_style: i32,
    pub avenue: [i32; 2],
    pub node: i32,
    pub page_plow: i32,
    pub split: i32,
}

impl Default for LayoutState {
    fn default() -> LayoutState {
        LayoutState {
            dlg: Shared::new(0),
            shapes: Vec::new(),
            placement: 0,
            route: 0,
            plow: 0,
            fixed: 0,
            ignore_points: 0,
            page: None,
            page_route: 0,
            jumps: 1,
            jump_style: 0,
            avenue: [75, 75],
            node: 25,
            page_plow: 0,
            split: 0,
        }
    }
}

pub(crate) fn owns(id: u32) -> bool {
    id == LAYOUT_BEHAVIOR
}

pub(crate) fn available(app: &Diagrammer, _id: u32) -> bool {
    let Some(o) = app.opened.as_ref() else { return false };
    edit_canvas::selected(app).iter().any(|&s| !diagram_core::glue::is_one_d(&o.doc, s))
}

fn num(doc: &Document, s: SheetRef, cell: &str) -> Option<f64> {
    doc.sym(cell).and_then(|c| doc.resolve(s, &CellAddr::single(c)).num()).filter(|v| v.is_finite())
}

/// Layout-Verhalten: the dialog with the first selected 2D shape's values
pub(crate) fn run(app: &mut Diagrammer, ui: &mut Ui, _id: u32) -> bool {
    let Some(o) = app.opened.as_ref() else { return true };
    let doc = &o.doc;
    let shapes: Vec<ShapeRef> = edit_canvas::selected(app).into_iter().filter(|&s| !diagram_core::glue::is_one_d(doc, s)).collect();
    let Some(&first) = shapes.first() else { return true };
    let s = SheetRef::Shape(first);
    let obj = num(doc, s, "ObjType").unwrap_or(0.0) as i64;
    let fixed = num(doc, s, "ShapeFixedCode").unwrap_or(0.0) as i64;
    let st = &mut app.layout;
    st.placement = if obj & 4 != 0 { 2 } else if obj & 1 != 0 { 1 } else { 0 };
    st.route = (num(doc, s, "ShapeRouteStyle").unwrap_or(0.0) as i32).clamp(0, ROUTE_STYLES.len() as i32 - 1);
    st.plow = (num(doc, s, "ShapePlowCode").unwrap_or(0.0) as i32).clamp(0, 2);
    st.fixed = (fixed & 1 != 0) as i32;
    st.ignore_points = (fixed & 0x20 != 0) as i32;
    st.shapes = shapes;
    st.dlg.set(1);
    ui.repass();
    true
}

/// the dialog Layout-Verhalten, every pass
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.layout.dlg.clone();
    if !ui.dialog_begin(ID_DLG, t("Layout-Verhalten"), &dlg) {
        return;
    }
    let st = &mut app.layout;
    let label = |ui: &mut Ui, s: &str| {
        ui.next_size(200, 0);
        ui.text(TS_BODY, s);
    };
    ui.next_size(460, 0);
    ui.box_begin(ID_DLG + 1, BOX_V, 8);
    ui.text(TS_BODY_STRONG, t("Platzierung"));
    ui.radio(ID_DLG + 2, t("Automatisch entscheiden"), &mut st.placement, 0);
    ui.radio(ID_DLG + 3, t("Layout und Routing um das Shape"), &mut st.placement, 1);
    ui.radio(ID_DLG + 4, t("Kein Layout und Routing um das Shape"), &mut st.placement, 2);
    ui.checkbox(ID_DLG + 5, t("Beim Layout der Seite nicht verschieben"), &mut st.fixed);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    label(ui, t("Andere Shapes beim Ablegen verschieben"));
    ui.next_size(200, 0);
    ui.combobox(ID_DLG + 6, &mut st.plow, &[crate::tc("Standard|Routing"), t("Nein"), t("Ja")]);
    ui.box_end();
    ui.text(TS_BODY_STRONG, t("Verbinder"));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    label(ui, t("Routing-Stil"));
    let names: Vec<&str> = ROUTE_STYLES.iter().map(|s| crate::tc(s)).collect();
    ui.next_size(240, 0);
    ui.combobox(ID_DLG + 7, &mut st.route, &names);
    ui.box_end();
    ui.checkbox(ID_DLG + 8, t("Verbindungspunkte beim Routing nicht benutzen"), &mut st.ignore_points);
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        apply_shapes(app, ui);
    }
    if r != DLG_NONE {
        app.layout.dlg.set(0);
        ui.repass();
    }
}

// write the dialog's values into every chosen shape (one undo step)
fn apply_shapes(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &app.layout;
    let shapes = st.shapes.clone();
    let (placement, route, plow, fixed, ignore) = (st.placement, st.route, st.plow, st.fixed, st.ignore_points);
    let Some(o) = app.opened.as_mut() else { return };
    let _ = edit_doc::edit(o, t("Layout-Verhalten"), true, |tx, eng| {
        for &s in &shapes {
            let sheet = SheetRef::Shape(s);
            let (obj, code) = {
                let doc = tx.doc();
                (num(doc, sheet, "ObjType").unwrap_or(0.0) as i64, num(doc, sheet, "ShapeFixedCode").unwrap_or(0.0) as i64)
            };
            let obj = (obj & !5) | [0, 1, 4][placement.clamp(0, 2) as usize];
            let code = (code & !0x21) | if fixed != 0 { 1 } else { 0 } | if ignore != 0 { 0x20 } else { 0 };
            for (cell, v) in [("ObjType", obj as f64), ("ShapeRouteStyle", route as f64), ("ShapePlowCode", plow as f64), ("ShapeFixedCode", code as f64)] {
                let a = tx.single(cell);
                tx.set_constant(sheet, &a, Value::Num(v), None)?;
            }
            eng.recalc(tx, &[sheet])?;
        }
        // connectors around the shapes take the new obstacles and styles
        let owner = shapes.first().map(|s| s.owner);
        if let Some(owner) = owner {
            edit_canvas::follow(tx, eng, owner, &shapes)?;
        }
        Ok(())
    });
    ui.repass();
}

// ---------------------------------------------------------------- Seite einrichten › Layout und Routing

/// the page's values into the register (Seite einrichten opens)
pub(crate) fn load(app: &mut Diagrammer, page: PageRef) {
    let Some(o) = app.opened.as_ref() else { return };
    let doc = &o.doc;
    let s = SheetRef::Page(page);
    let mm10 = |v: Option<f64>, d: f64| (v.unwrap_or(d) * 254.0 + 0.5) as i32;
    let st = &mut app.layout;
    st.page = Some(page);
    st.page_route = (num(doc, s, "RouteStyle").unwrap_or(0.0) as i32).clamp(0, ROUTE_STYLES.len() as i32 - 1);
    st.jumps = (num(doc, s, "LineJumpCode").unwrap_or(1.0) as i32).clamp(0, 4);
    st.jump_style = (num(doc, s, "LineJumpStyle").unwrap_or(0.0) as i32).clamp(0, 4);
    st.avenue = [mm10(num(doc, s, "AvenueSizeX"), 7.5 / 25.4), mm10(num(doc, s, "AvenueSizeY"), 7.5 / 25.4)];
    st.node = mm10(num(doc, s, "LineToNodeX"), 2.5 / 25.4);
    st.page_plow = (num(doc, s, "PlowCode").unwrap_or(0.0) == 1.0) as i32;
    st.split = (num(doc, s, "PageShapeSplit").unwrap_or(0.0) == 1.0) as i32;
}

/// the register Layout und Routing (editable)
pub(crate) fn tab(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let st = &mut app.layout;
    let row = |ui: &mut Ui, s: &str| {
        ui.next_size(210, 0);
        ui.text(TS_BODY, s);
    };
    let names: Vec<&str> = ROUTE_STYLES.iter().map(|s| crate::tc(s)).collect();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Routing-Stil"));
    ui.next_size(240, 0);
    ui.combobox(id, &mut st.page_route, &names);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Linienübergänge"));
    ui.next_size(240, 0);
    ui.combobox(id + 1, &mut st.jumps, &[t("Keine"), t("Horizontale Linien"), t("Vertikale Linien"), t("Zuletzt gezeichnete Linien"), t("Zuletzt gezeichnete Linien (umgekehrt)")]);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Stil der Linienübergänge"));
    ui.next_size(240, 0);
    ui.combobox(id + 2, &mut st.jump_style, &[crate::tc("Standard|Routing"), t("Bogen"), t("Lücke"), t("Quadrat"), t("Mehrseitig")]);
    ui.box_end();
    // lengths in tenths of a millimetre
    let range = Range::new(0, 2000, 5, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Abstand zwischen Shapes (0,1 mm)"));
    ui.next_size(90, 0);
    ui.number_box(id + 3, &mut st.avenue[0], &range);
    ui.next_size(90, 0);
    ui.number_box(id + 4, &mut st.avenue[1], &range);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Abstand Linie zu Shape (0,1 mm)"));
    ui.next_size(90, 0);
    ui.number_box(id + 5, &mut st.node, &range);
    ui.box_end();
    ui.checkbox(id + 6, t("Shapes beim Ablegen verschieben"), &mut st.page_plow);
    ui.checkbox(id + 7, t("Shapes auf Verbinder teilen"), &mut st.split);
}

/// OK of Seite einrichten: the register's values into the page, one undo step when anything
/// changed; connectors on the page are routed again
pub(crate) fn commit(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &app.layout;
    let Some(page) = st.page else { return };
    let s = SheetRef::Page(page);
    let inch = |tenths: i32| tenths as f64 / 254.0;
    let lengths = [("AvenueSizeX", inch(st.avenue[0])), ("AvenueSizeY", inch(st.avenue[1])), ("LineToNodeX", inch(st.node)), ("LineToNodeY", inch(st.node))];
    let codes = [
        ("RouteStyle", st.page_route as f64),
        ("LineJumpCode", st.jumps as f64),
        ("LineJumpStyle", st.jump_style as f64),
        ("PlowCode", if st.page_plow != 0 { 1.0 } else { 0.0 }),
        ("PageShapeSplit", if st.split != 0 { 1.0 } else { 0.0 }),
    ];
    let Some(o) = app.opened.as_mut() else { return };
    let changes: Vec<(&str, f64, Option<Unit>)> = lengths
        .iter()
        .map(|&(c, v)| (c, v, Some(Unit::Millimeters)))
        .chain(codes.iter().map(|&(c, v)| (c, v, None)))
        .filter(|&(c, v, _)| num(&o.doc, s, c).is_none_or(|n| (n - v).abs() > 1e-6))
        .collect();
    if changes.is_empty() {
        return;
    }
    let owner = diagram_core::Owner::Page(page);
    let _ = edit_doc::edit(o, t("Layout und Routing"), true, |tx, eng| {
        for (cell, v, unit) in &changes {
            let a = tx.single(cell);
            tx.set_constant(s, &a, Value::Num(*v), *unit)?;
        }
        eng.recalc(tx, &[s])?;
        // every dynamic connector of the page takes the new style and clearance
        let conns: Vec<ShapeRef> = tx.doc().top_shapes(owner).filter(|&c| diagram_core::glue::is_connector(tx.doc(), c)).collect();
        diagram_core::glue::update(tx, eng, &conns)
    });
    ui.repass();
}
