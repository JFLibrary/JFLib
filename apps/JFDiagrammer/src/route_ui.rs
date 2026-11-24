// route_ui -- (JD21, CN-04, CN-05, SS-19, SS-40) a connector's way by hand on the canvas, the
// routing styles with their directions in the dialogs, the shapes connectors may cross
//
//   RouteState                  the routing rows of Verbinder-Verhalten (ShapeRouteStyle,
//                               ConFixedCode, ConLineRouteExt), Layout-Verhalten (ShapePermeableX,
//                               ShapePermeableY, ShapePermeablePlace) and Seite einrichten › Layout
//                               und Routing (LineRouteExt, LineAdjustFrom, LineAdjustTo) while open
//   points(doc, r, geo, &mut pts) -> bool   the midpoints (SP_SEGMENT) and vertices (SP_VERTEX)
//                               of a single selected dynamic connector for ui.shape_edit (with
//                               the pointer and the line tools; edit_canvas::model asks), tags
//                               TAG_PIECE | i and TAG_BEND | j; true: listed (the line tool's
//                               vertex editing of its Geometry rows stays off: routing would
//                               overwrite them)
//   apply(app, ui, &ev) -> bool  SeEv::Point / PointInsert / PointDelete on those handles ->
//                               diagram_core::route_edit, one undo step (live while dragging)
//   pick(ui, ids, &mut code, first)   a routing style and its direction as two rows (RouteStyle
//                               / ShapeRouteStyle codes; `first` names code 0)
//   open_conn / conn_rows / conn_cells   Verbinder-Verhalten's rows Weg (jumps_ui.rs hooks them)
//   open_shape / shape_rows / shape_cells   Layout-Verhalten's rows (layout_ui.rs)
//   load_page / page_rows / page_codes   Seite einrichten › Layout und Routing (layout_ui.rs)
//
// The gestures (Visio's, details in diagram_core/src/route_edit.rs): a midpoint dragged moves
// its piece across (a straight connector gets a vertex there), a vertex dragged moves the pieces
// at it, Ctrl + click with the line or pencil tool adds a vertex, a click on a vertex and Del
// removes it; a piece dragged onto the line of another snaps there within SNAP_PX and the bends
// between them go.  The way is kept afterwards (ConFixedCode 1); Verbinder zurücksetzen frees it.

use alloc::vec::Vec;

use diagram_core::layout::Dir;
use diagram_core::route_edit::{self, Edit};
use diagram_core::route_styles::{Kind, RouteStyle};
use diagram_core::{CellAddr, Document, PageRef, ShapeRef, SheetRef};
use ui::i18n::t;
use ui::*;

use crate::edit_canvas::{self, Geo};
use crate::edit_doc;
use crate::viewer::DIP;
use crate::Diagrammer;

/// a midpoint's tag: TAG_PIECE | the piece's index
pub(crate) const TAG_PIECE: u32 = 0x4000_0000;
/// a vertex's tag: TAG_BEND | the index of its point
pub(crate) const TAG_BEND: u32 = 0x2000_0000;
const INDEX: u32 = 0x00FF_FFFF;
/// how near (screen px) a dragged piece snaps onto the line of another
const SNAP_PX: f64 = 6.0;

/// the routing rows of the dialogs while they are open
#[derive(Default)]
pub(crate) struct RouteState {
    /// Verbinder-Verhalten: ShapeRouteStyle, ConFixedCode, ConLineRouteExt as chosen and as read
    pub conn: [i32; 3],
    conn_read: [i32; 3],
    /// Layout-Verhalten: ShapePermeableX, ShapePermeableY, ShapePermeablePlace
    pub perm: [i32; 3],
    perm_read: [i32; 3],
    /// Seite einrichten: LineRouteExt, LineAdjustFrom, LineAdjustTo
    pub page: [i32; 3],
}

const CONN_CELLS: [&str; 3] = ["ShapeRouteStyle", "ConFixedCode", "ConLineRouteExt"];
const PERM_CELLS: [&str; 3] = ["ShapePermeableX", "ShapePermeableY", "ShapePermeablePlace"];
const PAGE_CELLS: [&str; 3] = ["LineRouteExt", "LineAdjustFrom", "LineAdjustTo"];

fn num(doc: &Document, s: SheetRef, cell: &str) -> Option<f64> {
    doc.sym(cell).and_then(|c| doc.resolve(s, &CellAddr::single(c)).num()).filter(|v| v.is_finite())
}

// ---------------------------------------------------------------- the handles on the canvas

/// the midpoints and vertices of a dynamic connector whose way can be edited by hand, for the
/// edit layer -> whether it has them
pub(crate) fn points(doc: &Document, r: ShapeRef, geo: &Geo, out: &mut Vec<SePoint>) -> bool {
    if route_edit::way(doc, r).is_none() {
        return false;
    }
    for h in route_edit::handles(doc, r) {
        let at = geo.world(h.at);
        if h.bend {
            out.push(SePoint { at, kind: SP_VERTEX, tag: TAG_BEND | (h.index as u32 & INDEX), dir: [0.0; 2], flags: 0 });
        } else {
            let end = geo.world([h.at[0] + h.half[0], h.at[1] + h.half[1]]);
            let flags = if h.free { SPF_FREE } else { 0 };
            out.push(SePoint { at, kind: SP_SEGMENT, tag: TAG_PIECE | (h.index as u32 & INDEX), dir: [end[0] - at[0], end[1] - at[1]], flags });
        }
    }
    true
}

/// an event on a connector's midpoint or vertex: the way changed by hand (one undo step; a drag
/// live) -> whether it was one
pub(crate) fn apply(app: &mut Diagrammer, ui: &mut Ui, ev: &SeEv) -> bool {
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return false };
    let (key, edit, done, label) = match *ev {
        SeEv::Point { key, tag, at, done, .. } if tag & TAG_PIECE != 0 => (key, Edit::Piece((tag & INDEX) as usize, geo.page(at)), done, t("Segment verschieben")),
        SeEv::Point { key, tag, at, done, .. } if tag & TAG_BEND != 0 => (key, Edit::Bend((tag & INDEX) as usize, geo.page(at)), done, t("Scheitelpunkt verschieben")),
        SeEv::PointInsert { key, at } => (key, Edit::Add(geo.page(at)), true, t("Scheitelpunkt hinzufügen")),
        SeEv::PointDelete { key, tag } if tag & TAG_BEND != 0 => (key, Edit::Remove((tag & INDEX) as usize), true, t("Scheitelpunkt löschen")),
        _ => return false,
    };
    let s = edit_canvas::shape_of(page, key);
    let scale = app.canvas.vp.scale();
    let Some(o) = app.opened.as_mut() else { return false };
    // Ctrl + click on a shape that is no such connector: the line tool's vertices
    if route_edit::way(&o.doc, s).is_none() {
        return false;
    }
    let snap = if scale > 0.0 { SNAP_PX / scale / DIP / geo.scale } else { 0.0 };
    let _ = edit_doc::edit(o, label, done, |tx, eng| route_edit::edit(tx, eng, s, edit, snap).map(|_| ()));
    ui.repass();
    true
}

// ---------------------------------------------------------------- routing style and direction

// i18n
static STYLES: [&str; 11] = [
    "Standard|Routing", "Rechtwinklig", "Gerade", "Organigramm|Routing", "Flussdiagramm", "Baum|Routing", "Netzwerk", "Einfach|Routing",
    "Einfach waagerecht-senkrecht", "Einfach senkrecht-waagerecht", "Mittig zu Mittig",
];
const KINDS: [Kind; 10] = [
    Kind::RightAngle, Kind::Straight, Kind::OrgChart, Kind::Flowchart, Kind::Tree, Kind::Network, Kind::Simple, Kind::SimpleHV, Kind::SimpleVH, Kind::CentreToCentre,
];
// i18n
static DIRS: [&str; 4] = ["Von oben nach unten", "Von unten nach oben", "Von links nach rechts", "Von rechts nach links"];
const DIR_OF: [Dir; 4] = [Dir::Down, Dir::Up, Dir::Right, Dir::Left];

/// a RouteStyle code as the rows show it: the style (0: the code 0) and the direction
pub(crate) fn split(code: i32) -> (i32, i32) {
    match RouteStyle::from_code(code as i64) {
        Some(rs) => {
            let k = KINDS.iter().position(|&x| x == rs.kind).map_or(0, |i| i as i32 + 1);
            let d = rs.dir.and_then(|d| DIR_OF.iter().position(|&x| x == d)).unwrap_or(0) as i32;
            (k, d)
        }
        None => (0, 0),
    }
}

/// the rows' style and direction as a RouteStyle code
pub(crate) fn join(style: i32, dir: i32) -> i32 {
    if style <= 0 {
        return 0;
    }
    let kind = KINDS[(style as usize - 1).min(KINDS.len() - 1)];
    RouteStyle::new(kind, Some(DIR_OF[dir.clamp(0, 3) as usize])).code() as i32
}

/// the routing style and its direction as two rows of a dialog (combo boxes `ids`): `code` is
/// the RouteStyle / ShapeRouteStyle code, `first` the name of code 0 (Standard, Seitenvorgabe)
pub(crate) fn pick(ui: &mut Ui, ids: (u32, u32), code: &mut i32, first: &str, label_w: i32) {
    let (mut style, mut dir) = split(*code);
    let names: Vec<&str> = core::iter::once(first).chain(STYLES[1..].iter().map(|s| crate::tc(s))).collect();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(label_w, 0);
    ui.text(TS_BODY, t("Routing-Stil"));
    ui.next_size(240, 0);
    ui.combobox(ids.0, &mut style, &names);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(label_w, 0);
    ui.text(TS_BODY, t("Richtung des Routings"));
    let dirs: Vec<&str> = DIRS.iter().map(|s| t(s)).collect();
    let directed = style > 0 && KINDS[(style as usize - 1).min(KINDS.len() - 1)].directed();
    ui.next_size(240, 0);
    ui.next_flags(if directed { 0 } else { ST_DISABLED });
    ui.combobox(ids.1, &mut dir, &dirs);
    ui.box_end();
    let now = join(style, dir);
    // a code read stays while neither row changed (also one the rows cannot show)
    if (style, dir) != split(*code) {
        *code = now;
    }
}

fn row(ui: &mut Ui, w: i32, s: &str) {
    ui.next_size(w, 0);
    ui.text(TS_BODY, s);
}

// ---------------------------------------------------------------- Verbinder-Verhalten

/// Verbinder-Verhalten opens: the connector's routing cells
pub(crate) fn open_conn(app: &mut Diagrammer, c: ShapeRef) {
    let Some(o) = app.opened.as_ref() else { return };
    let s = SheetRef::Shape(c);
    let v = |cell: &str, max: f64| num(&o.doc, s, cell).unwrap_or(0.0).clamp(0.0, max) as i32;
    let read = [v(CONN_CELLS[0], 22.0), v(CONN_CELLS[1], 3.0), v(CONN_CELLS[2], 2.0)];
    app.route.conn = read;
    app.route.conn_read = read;
}

/// the rows Weg of Verbinder-Verhalten (Visio's Verhalten › Verbinder › Pfad): style, direction,
/// when to lay it anew, appearance
pub(crate) fn conn_rows(st: &mut RouteState, ui: &mut Ui, id: u32) {
    ui.text(TS_BODY_STRONG, t("Weg"));
    pick(ui, (id, id + 1), &mut st.conn[0], t("Seitenvorgabe"), 200);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, 200, t("Neu verlegen"));
    ui.next_size(240, 0);
    ui.combobox(id + 2, &mut st.conn[1], &[t("Frei"), t("Nach Bedarf"), t("Nie"), t("Bei Überkreuzung")]);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, 200, t("Darstellung"));
    ui.next_size(240, 0);
    ui.combobox(id + 3, &mut st.conn[2], &[t("Seitenvorgabe"), t("Gerade"), t("Gebogen")]);
    ui.box_end();
}

/// the cells the rows Weg set (each with whether it changed) and whether the connectors must be
/// routed anew (style or appearance changed)
pub(crate) fn conn_cells(st: &RouteState) -> (Vec<(&'static str, i32, bool)>, bool) {
    let cells: Vec<(&'static str, i32, bool)> = (0..3).map(|k| (CONN_CELLS[k], st.conn[k], st.conn[k] != st.conn_read[k])).collect();
    let reroute = cells[0].2 || cells[2].2;
    (cells, reroute)
}

// ---------------------------------------------------------------- Layout-Verhalten

/// Layout-Verhalten opens: the shape's permeability
pub(crate) fn open_shape(app: &mut Diagrammer, s: ShapeRef) {
    let Some(o) = app.opened.as_ref() else { return };
    let sh = SheetRef::Shape(s);
    let read = PERM_CELLS.map(|c| num(&o.doc, sh, c).is_some_and(|v| v != 0.0) as i32);
    app.route.perm = read;
    app.route.perm_read = read;
}

/// Layout-Verhalten's rows: connectors may cross the shape (Visio's Verhalten › Platzierung ›
/// Interaktion mit Verbindern), shapes may be placed on it
pub(crate) fn shape_rows(st: &mut RouteState, ui: &mut Ui, id: u32) {
    ui.checkbox(id, t("Verbinder dürfen das Shape waagerecht durchqueren"), &mut st.perm[0]);
    ui.checkbox(id + 1, t("Verbinder dürfen das Shape senkrecht durchqueren"), &mut st.perm[1]);
    ui.checkbox(id + 2, t("Andere Shapes dürfen beim Layout darauf platziert werden"), &mut st.perm[2]);
}

/// the permeability cells that changed (cell, value)
pub(crate) fn shape_cells(st: &RouteState) -> Vec<(&'static str, f64)> {
    (0..3).filter(|&k| st.perm[k] != st.perm_read[k]).map(|k| (PERM_CELLS[k], if st.perm[k] != 0 { 1.0 } else { 0.0 })).collect()
}

// ---------------------------------------------------------------- Seite einrichten

/// Seite einrichten opens: the page's appearance and spacing apart of lines
pub(crate) fn load_page(app: &mut Diagrammer, page: PageRef) {
    let Some(o) = app.opened.as_ref() else { return };
    let s = SheetRef::Page(page);
    app.route.page = [
        num(&o.doc, s, PAGE_CELLS[0]).unwrap_or(0.0).clamp(0.0, 2.0) as i32,
        num(&o.doc, s, PAGE_CELLS[1]).unwrap_or(0.0).clamp(0.0, 3.0) as i32,
        num(&o.doc, s, PAGE_CELLS[2]).unwrap_or(0.0).clamp(0.0, 3.0) as i32,
    ];
}

/// Layout und Routing's rows after the routing style: appearance (LineRouteExt), separate
/// (LineAdjustFrom), overlap (LineAdjustTo); the two last are kept for Visio
pub(crate) fn page_rows(st: &mut RouteState, ui: &mut Ui, id: u32, label_w: i32) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, label_w, t("Darstellung"));
    ui.next_size(240, 0);
    ui.combobox(id, &mut st.page[0], &[crate::tc("Standard|Routing"), t("Gerade"), t("Gebogen")]);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, label_w, t("Linien trennen und überlappen"));
    ui.next_size(116, 0);
    ui.combobox(id + 1, &mut st.page[1], &[t("Unverbundene"), t("Alle"), t("Keine"), t("Wie der Stil")]);
    ui.next_size(116, 0);
    ui.combobox(id + 2, &mut st.page[2], &[t("Wie der Stil"), t("Benachbarte"), t("Keine"), t("Verbundene")]);
    ui.box_end();
}

/// the page cells of these rows with their values
pub(crate) fn page_codes(st: &RouteState) -> [(&'static str, f64); 3] {
    [(PAGE_CELLS[0], st.page[0] as f64), (PAGE_CELLS[1], st.page[1] as f64), (PAGE_CELLS[2], st.page[2] as f64)]
}
