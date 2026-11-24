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
//                               Seite einrichten: (JD15) the placement style and direction
//                               (PlaceStyle: place_code / place_split), its depth (PlaceDepth)
//                               and flipping (PlaceFlip), routing style (RouteStyle), line jumps
//                               (LineJumpCode, LineJumpStyle; JD21: all six codes, the nine
//                               styles, their size LineJumpFactorX/Y and direction
//                               PageLineJumpDirX/Y), the space between shapes
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

/// (JD21 jumps) the line jump size factor of Visio's files (LineJumpFactorX/Y), also for a
/// page without the cells
const JUMP_FACTOR: f64 = 2.0 / 3.0;

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
    /// (JD21 jumps) LineJumpStyle as read (0 stays 0 while the arc stays chosen), the sizes
    /// (LineJumpFactorX/Y), the directions (PageLineJumpDirX/Y)
    pub jump_style_raw: i32,
    pub jump_size: [f64; 2],
    pub jump_dir: [i32; 2],
    pub avenue: [i32; 2],
    pub node: i32,
    pub page_plow: i32,
    pub split: i32,
    /// (JD15, PG-11) the page's placement: style and direction (PlaceStyle, as read: the code
    /// stays when neither changes), depth (PlaceDepth), flipping (PlaceFlip)
    pub place_style: i32,
    pub place_dir: i32,
    pub place_code: i32,
    pub place_depth: i32,
    pub place_flip: i32,
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
            jump_style_raw: 0,
            jump_size: [JUMP_FACTOR; 2],
            jump_dir: [0; 2],
            avenue: [75, 75],
            node: 25,
            page_plow: 0,
            split: 0,
            place_style: 0,
            place_dir: 0,
            place_code: 0,
            place_depth: 0,
            place_flip: 0,
        }
    }
}

/// (JD15, PG-11) the placement styles of the register (Visio's Configure Layout: Flowchart,
/// Compact Tree, Hierarchy, Radial, Circular) and the directions
// i18n
static PLACE_STYLES: [&str; 6] = ["Standard|Routing", "Flussdiagramm", "Kompakte Struktur", "Hierarchie", "Radial", "Kreisförmig"];
// i18n
static PLACE_DIRS: [&str; 4] = ["Von oben nach unten", "Von unten nach oben", "Von links nach rechts", "Von rechts nach links"];

/// (JD15, PG-11) style and direction -> PlaceStyle (Visio's visPLOPlace* codes: 1 top to
/// bottom, 2 left to right, 3 radial, 4 bottom to top, 5 right to left, 6 circular, 7 .. 14 the
/// compact trees, 15 .. 26 the hierarchies; centred where Visio offers an alignment)
pub(crate) fn place_code(style: i32, dir: i32) -> i32 {
    let d = dir.clamp(0, 3) as usize;
    match style {
        1 => [1, 4, 2, 5][d],
        2 => [7, 11, 8, 10][d],
        3 => [16, 19, 22, 25][d],
        4 => 3,
        5 => 6,
        _ => 0,
    }
}

/// (JD15, PG-11) PlaceStyle -> (style, direction) of the register
pub(crate) fn place_split(code: i32) -> (i32, i32) {
    match code {
        1 => (1, 0),
        4 => (1, 1),
        2 => (1, 2),
        5 => (1, 3),
        3 => (4, 0),
        6 => (5, 0),
        7 | 9 => (2, 0),
        11 | 13 => (2, 1),
        8 | 12 => (2, 2),
        10 | 14 => (2, 3),
        15..=17 => (3, 0),
        18..=20 => (3, 1),
        21..=23 => (3, 2),
        24..=26 => (3, 3),
        _ => (0, 0),
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
    // (JD21 jumps) the six codes, the nine styles (0 is the arc too), size and direction
    st.jumps = (num(doc, s, "LineJumpCode").unwrap_or(1.0) as i32).clamp(0, 5);
    st.jump_style_raw = (num(doc, s, "LineJumpStyle").unwrap_or(0.0) as i32).clamp(0, 9);
    st.jump_style = (st.jump_style_raw - 1).max(0);
    st.jump_size = [num(doc, s, "LineJumpFactorX").unwrap_or(JUMP_FACTOR).clamp(0.0, 10.0), num(doc, s, "LineJumpFactorY").unwrap_or(JUMP_FACTOR).clamp(0.0, 10.0)];
    st.jump_dir = [(num(doc, s, "PageLineJumpDirX").unwrap_or(0.0) as i32).clamp(0, 2), (num(doc, s, "PageLineJumpDirY").unwrap_or(0.0) as i32).clamp(0, 2)];
    st.avenue = [mm10(num(doc, s, "AvenueSizeX"), 7.5 / 25.4), mm10(num(doc, s, "AvenueSizeY"), 7.5 / 25.4)];
    st.node = mm10(num(doc, s, "LineToNodeX"), 2.5 / 25.4);
    st.page_plow = (num(doc, s, "PlowCode").unwrap_or(0.0) == 1.0) as i32;
    st.split = (num(doc, s, "PageShapeSplit").unwrap_or(0.0) == 1.0) as i32;
    // (JD15, PG-11) the placement
    st.place_code = num(doc, s, "PlaceStyle").unwrap_or(0.0) as i32;
    (st.place_style, st.place_dir) = place_split(st.place_code);
    st.place_depth = (num(doc, s, "PlaceDepth").unwrap_or(0.0) as i32).clamp(0, 3);
    st.place_flip = (num(doc, s, "PlaceFlip").unwrap_or(0.0) as i32) & 3;
}

/// the register Layout und Routing (editable)
pub(crate) fn tab(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let st = &mut app.layout;
    let row = |ui: &mut Ui, s: &str| {
        ui.next_size(210, 0);
        ui.text(TS_BODY, s);
    };
    // (JD15, PG-11) the placement: style and direction, depth, flipping
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Platzierungsstil"));
    let styles: Vec<&str> = PLACE_STYLES.iter().map(|s| crate::tc(s)).collect();
    ui.next_size(240, 0);
    ui.combobox(id + 8, &mut st.place_style, &styles);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Richtung"));
    let dirs: Vec<&str> = PLACE_DIRS.iter().map(|s| t(s)).collect();
    ui.next_size(240, 0);
    ui.next_flags(if matches!(st.place_style, 1..=3) { 0 } else { ST_DISABLED });
    ui.combobox(id + 9, &mut st.place_dir, &dirs);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Tiefe und Spiegelung"));
    ui.next_size(116, 0);
    ui.combobox(id + 10, &mut st.place_depth, &[crate::tc("Standard|Routing"), t("Mittel"), t("Tief"), crate::tc("Flach|Tiefe")]);
    ui.next_size(116, 0);
    ui.combobox(id + 11, &mut st.place_flip, &[t("Keine"), t("Horizontal"), t("Vertikal"), t("Beide")]);
    ui.box_end();
    let names: Vec<&str> = ROUTE_STYLES.iter().map(|s| crate::tc(s)).collect();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Routing-Stil"));
    ui.next_size(240, 0);
    ui.combobox(id, &mut st.page_route, &names);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Linienübergänge"));
    ui.next_size(240, 0);
    // (JD21 jumps) Visio's six codes (LineJumpCode 0 .. 5)
    ui.combobox(id + 1, &mut st.jumps, &[t("Keine"), t("Horizontale Linien"), t("Vertikale Linien"), t("Zuletzt verlegte Linie"), t("Zuletzt angezeigte Linie"), t("Zuerst angezeigte Linie")]);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Stil der Linienübergänge"));
    let jump_styles = crate::jumps_ui::style_names(true);
    let jump_styles: Vec<&str> = jump_styles.iter().map(|s| s.as_str()).collect();
    ui.next_size(240, 0);
    ui.combobox(id + 2, &mut st.jump_style, &jump_styles);
    ui.box_end();
    // (JD21 jumps) the size as a factor of the line spacing (LineToLineX/Y), horizontal and
    // vertical lines; the direction they rise to
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Größe der Linienübergänge (waagerecht, senkrecht)"));
    let factor = RangeF::new(0.0, 10.0, 0.05, 2, 0);
    ui.next_size(90, 0);
    ui.number_box_f(id + 12, &mut st.jump_size[0], &factor);
    ui.next_size(90, 0);
    ui.number_box_f(id + 13, &mut st.jump_size[1], &factor);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Richtung der Linienübergänge (waagerecht, senkrecht)"));
    ui.next_size(116, 0);
    ui.combobox(id + 14, &mut st.jump_dir[0], &[crate::tc("Standard|Routing"), crate::tc("Nach oben|Linienübergang"), crate::tc("Nach unten|Linienübergang")]);
    ui.next_size(116, 0);
    ui.combobox(id + 15, &mut st.jump_dir[1], &[crate::tc("Standard|Routing"), crate::tc("Nach links|Linienübergang"), crate::tc("Nach rechts|Linienübergang")]);
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

/// (JD15, JD21 jumps) the value a page that lacks the cell has: nothing to write
fn absent_default(cell: &str, v: f64) -> bool {
    let d = match cell {
        c if c.starts_with("Place") => 0.0,
        "LineJumpCode" => 1.0,
        "LineJumpStyle" | "PageLineJumpDirX" | "PageLineJumpDirY" => 0.0,
        "LineJumpFactorX" | "LineJumpFactorY" => JUMP_FACTOR,
        _ => return false,
    };
    (v - d).abs() < 1e-6
}

/// OK of Seite einrichten: the register's values into the page, one undo step when anything
/// changed; connectors on the page are routed again
pub(crate) fn commit(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &app.layout;
    let Some(page) = st.page else { return };
    let s = SheetRef::Page(page);
    let inch = |tenths: i32| tenths as f64 / 254.0;
    let lengths = [("AvenueSizeX", inch(st.avenue[0])), ("AvenueSizeY", inch(st.avenue[1])), ("LineToNodeX", inch(st.node)), ("LineToNodeY", inch(st.node))];
    // (JD15, PG-11) the placement code read stays unless style or direction changed
    let place = if place_split(st.place_code) == (st.place_style, st.place_dir) { st.place_code } else { place_code(st.place_style, st.place_dir) };
    let codes = [
        ("PlaceStyle", place as f64),
        ("PlaceDepth", st.place_depth as f64),
        ("PlaceFlip", st.place_flip as f64),
        ("RouteStyle", st.page_route as f64),
        ("LineJumpCode", st.jumps as f64),
        // (JD21 jumps) the arc read as 0 stays 0; the size and the direction
        ("LineJumpStyle", if st.jump_style_raw == 0 && st.jump_style == 0 { 0.0 } else { (st.jump_style + 1) as f64 }),
        ("LineJumpFactorX", st.jump_size[0]),
        ("LineJumpFactorY", st.jump_size[1]),
        ("PageLineJumpDirX", st.jump_dir[0] as f64),
        ("PageLineJumpDirY", st.jump_dir[1] as f64),
        ("PlowCode", if st.page_plow != 0 { 1.0 } else { 0.0 }),
        ("PageShapeSplit", if st.split != 0 { 1.0 } else { 0.0 }),
    ];
    let Some(o) = app.opened.as_mut() else { return };
    let changes: Vec<(&str, f64, Option<Unit>)> = lengths
        .iter()
        .map(|&(c, v)| (c, v, Some(Unit::Millimeters)))
        .chain(codes.iter().map(|&(c, v)| (c, v, None)))
        // (JD15) a placement cell the page lacks counts as 0 (Visio's default); (JD21 jumps) a
        // line jump cell as Visio's value
        .filter(|&(c, v, _)| num(&o.doc, s, c).map_or(!absent_default(c, v), |n| (n - v).abs() > 1e-6))
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
