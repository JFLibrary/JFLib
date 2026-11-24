// dlg_layout -- (JD21) the dialogs of the layout commands: Layout konfigurieren (Seitenlayout neu
// anordnen › Weitere Layoutoptionen, CN-13, RW-09), Abstandsoptionen (CN-14, RS-25), Shapes
// verteilen (Weitere Verteilungsoptionen, RS-25)
//
//   LayoutDialog                 the three dialogs' state
//   open_layout(app, ui)         Layout konfigurieren with the page's settings: placement
//                                (style, direction, alignment, spacing), connectors (style,
//                                appearance, Routing-Stil auf Verbinder anwenden), apply to the
//                                selection or the page, Seite vergrößern, Seitenumbrüche
//                                vermeiden; a preview of the result (boxes and connections)
//   open_spacing(app, ui) / open_distribute(app, ui)
//   dialogs(app, ui)             every pass: the open dialog
//
// OK of Layout konfigurieren is one undo step: the page's cells as Visio's dialog sets them
// (PlaceStyle, AvenueSizeX/Y, RouteStyle, LineRouteExt, ResizePage; layout::store), the layout
// (layout::relayout, the routing style on the connectors when asked), then off the breaks and
// the page enlarged (layout_cmds::finish).  Abstandsoptionen keeps its spacing for the session
// and the settings, not in the drawing (Visio does not document where it keeps it).  Shapes
// verteilen: across, equal gaps or the left edges, middles, right edges evenly between the
// outer two; up and down alike (Visio's options; "create guides" is not offered).

use alloc::vec::Vec;

use diagram_core::layout::{self, Align, Dir, Opts, Place, Routing, Style};
use diagram_core::{ops, CellAddr, Document, Owner, ShapeRef, SheetRef};
use ui::i18n::t;
use ui::*;

use crate::layout_cmds;
use crate::layout_ui::ROUTE_STYLES;
use crate::{edit_canvas, edit_doc, tc, Diagrammer};

const ID_LAYOUT: u32 = 0x7DA0_0000;
const ID_SPACING: u32 = 0x7DB0_0000;
const ID_DIST: u32 = 0x7DC0_0000;

// i18n
static STYLES: [&str; 5] = ["Flussdiagramm", "Hierarchie", "Kompakte Struktur", "Radial", "Kreisförmig"];
// i18n
static DIRS: [&str; 4] = ["Von oben nach unten", "Von unten nach oben", "Von links nach rechts", "Von rechts nach links"];
/// the compact trees in PlaceStyle order 7 .. 14
// i18n
static TREE_DIRS: [&str; 8] = [
    "Nach unten, dann rechts",
    "Nach rechts, dann unten",
    "Nach rechts, dann oben",
    "Nach oben, dann rechts",
    "Nach oben, dann links",
    "Nach links, dann oben",
    "Nach links, dann unten",
    "Nach unten, dann links",
];
// i18n
static ALIGN_V: [&str; 3] = ["Links|Ausrichtung", "Zentriert|Ausrichtung", "Rechts|Ausrichtung"];
// i18n
static ALIGN_H: [&str; 3] = ["Oben|Ausrichtung", "Mitte|Ausrichtung", "Unten|Ausrichtung"];

/// the dialogs' state
pub(crate) struct LayoutDialog {
    pub dlg: Shared<i32>,
    style: i32,
    dir: i32,
    align: i32,
    /// tenths of a millimetre
    spacing: i32,
    route: i32,
    /// 0 straight, 1 curved (LineRouteExt 1, 2)
    look: i32,
    apply_route: i32,
    /// 0 the selection, 1 the page
    scope: i32,
    enlarge: i32,
    avoid: i32,
    /// the preview: what it shows (place code, spacing, scope, version) and its boxes and lines
    shown: Option<(i64, i32, i32, u32)>,
    boxes: Vec<[f64; 4]>,
    lines: Vec<([f64; 2], [f64; 2])>,
    /// Abstandsoptionen: 0 the page's spacing, 1 its own; x and y in tenths of a millimetre
    pub sp_dlg: Shared<i32>,
    sp_own: i32,
    sp: [i32; 2],
    /// Shapes verteilen: across (0 none, 1 equal gaps, 2 left, 3 middles, 4 right), up and down
    pub dist_dlg: Shared<i32>,
    dist: [i32; 2],
}

impl Default for LayoutDialog {
    fn default() -> LayoutDialog {
        LayoutDialog {
            dlg: Shared::new(0),
            style: 0,
            dir: 0,
            align: 1,
            spacing: 75,
            route: 0,
            look: 0,
            apply_route: 0,
            scope: 1,
            enlarge: 0,
            avoid: 0,
            shown: None,
            boxes: Vec::new(),
            lines: Vec::new(),
            sp_dlg: Shared::new(0),
            sp_own: 0,
            sp: [75, 75],
            dist_dlg: Shared::new(0),
            dist: [1, 0],
        }
    }
}

fn num(doc: &Document, s: SheetRef, cell: &str) -> Option<f64> {
    doc.sym(cell).and_then(|c| doc.resolve(s, &CellAddr::single(c)).num()).filter(|v| v.is_finite())
}

fn tenths(inches: f64) -> i32 {
    (inches * 254.0 + 0.5) as i32
}

const DIR4: [Dir; 4] = [Dir::Down, Dir::Up, Dir::Right, Dir::Left];

// the dialog's choices -> a placement
fn place_of(d: &LayoutDialog) -> Place {
    let dir = DIR4[d.dir.clamp(0, 3) as usize];
    match d.style {
        1 => Place { align: [Align::Start, Align::Centre, Align::End][d.align.clamp(0, 2) as usize], ..Place::new(Style::Hierarchy, dir) },
        2 => Place::from_code(7 + d.dir.clamp(0, 7) as i64).unwrap_or(Place::new(Style::CompactTree, Dir::Down)),
        3 => Place::new(Style::Radial, Dir::Down),
        4 => Place::new(Style::Circular, Dir::Down),
        _ => Place::new(Style::Flowchart, dir),
    }
}

// a placement -> the dialog's choices
fn choose(d: &mut LayoutDialog, p: Place) {
    let di = DIR4.iter().position(|&x| x == p.dir).unwrap_or(0) as i32;
    (d.style, d.dir, d.align) = match p.style {
        Style::Flowchart => (0, di, 1),
        Style::Hierarchy => (1, di, [Align::Start, Align::Centre, Align::End].iter().position(|&a| a == p.align).unwrap_or(1) as i32),
        Style::CompactTree => (2, (p.code() - 7).clamp(0, 7) as i32, 1),
        Style::Radial => (3, 0, 1),
        Style::Circular => (4, 0, 1),
    };
}

/// Weitere Layoutoptionen: Layout konfigurieren with the page's settings
pub(crate) fn open_layout(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(page) = app.model.page else { return };
    let chosen = !layout_cmds::chosen(app).is_empty();
    let avoid = layout_cmds::avoid_on(app);
    let Some(o) = app.opened.as_ref() else { return };
    let doc = &o.doc;
    let s = SheetRef::Page(page);
    let opts = Opts::from_page(doc, page);
    let k = diagram_core::pagesetup::scale(doc, page);
    let d = &mut app.lay.dlg;
    choose(d, opts.place);
    d.spacing = tenths(opts.spacing[0].max(opts.spacing[1]) * k);
    d.route = (num(doc, s, "RouteStyle").unwrap_or(0.0) as i32).clamp(0, ROUTE_STYLES.len() as i32 - 1);
    d.look = (num(doc, s, "LineRouteExt").unwrap_or(0.0) == 2.0) as i32;
    d.apply_route = 0;
    d.scope = if chosen { 0 } else { 1 };
    d.enlarge = num(doc, s, "ResizePage").is_some_and(|v| v != 0.0) as i32;
    d.avoid = avoid as i32;
    d.shown = None;
    d.dlg.set(1);
    ui.repass();
}

// the options of the dialog for the page
fn opts_of(d: &LayoutDialog, doc: &Document, page: diagram_core::PageRef) -> Opts {
    let mut o = Opts::from_page(doc, page);
    o.place = place_of(d);
    let k = diagram_core::pagesetup::scale(doc, page);
    let sp = d.spacing.max(0) as f64 / 254.0 / k;
    o.spacing = [sp, sp];
    o
}

// the preview's boxes and connections, made anew when a choice or the drawing changed
fn update_preview(app: &mut Diagrammer) {
    let Some(page) = app.model.page else { return };
    let only = if app.lay.dlg.scope == 0 { layout_cmds::chosen(app) } else { Vec::new() };
    let Some(o) = app.opened.as_ref() else { return };
    let d = &mut app.lay.dlg;
    let key = (place_of(d).code(), d.spacing, d.scope, o.ed.rev);
    if d.shown == Some(key) {
        return;
    }
    let opts = opts_of(d, &o.doc, page);
    let dia = layout::diagram(&o.doc, Owner::Page(page), &only);
    let at = layout::arrange(&dia, &opts);
    d.boxes = dia
        .boxes
        .iter()
        .zip(&at)
        .map(|(b, c)| {
            let (w, h) = ((b[2] - b[0]) * 0.5, (b[3] - b[1]) * 0.5);
            [c[0] - w, c[1] - h, c[0] + w, c[1] + h]
        })
        .collect();
    d.lines = dia.edges.iter().filter_map(|&(a, b)| Some((*at.get(a)?, *at.get(b)?))).collect();
    d.shown = Some(key);
}

// the preview: the laid-out boxes and their connections scaled into a frame
fn draw_preview(app: &Diagrammer, ui: &mut Ui, id: u32) {
    let d = &app.lay.dlg;
    let _ = id;
    let r = ui.place(ui.dp(300), ui.dp(170));
    let (x0, y0, w, h) = (r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64);
    ui.draw_color(ui.color(T_CTRL_STROKE));
    ui.draw_frame(r[0], r[1], r[2], r[3], 0, ui.dp(1).max(1));
    let Some(bb) = d.boxes.iter().copied().reduce(|m, b| [m[0].min(b[0]), m[1].min(b[1]), m[2].max(b[2]), m[3].max(b[3])]) else {
        return;
    };
    let pad = ui.dp(10) as f64;
    let s = ((w - 2.0 * pad) / (bb[2] - bb[0]).max(1e-6)).min((h - 2.0 * pad) / (bb[3] - bb[1]).max(1e-6));
    if !(s.is_finite() && s > 0.0) {
        return;
    }
    let ox = x0 + (w - (bb[2] - bb[0]) * s) * 0.5;
    let oy = y0 + (h - (bb[3] - bb[1]) * s) * 0.5;
    // page (y up) -> pixels (y down)
    let px = |p: [f64; 2]| [ox + (p[0] - bb[0]) * s, oy + (bb[3] - p[1]) * s];
    ui.draw_color(ui.color(T_TEXT_DISABLED));
    let lw = (ui.dp(1).max(1) * 256) as i32;
    for (a, b) in &d.lines {
        let (p, q) = (px(*a), px(*b));
        ui.draw_line((p[0] * 256.0) as i32, (p[1] * 256.0) as i32, (q[0] * 256.0) as i32, (q[1] * 256.0) as i32, lw);
    }
    for b in &d.boxes {
        let (p, q) = (px([b[0], b[3]]), px([b[2], b[1]]));
        let (bw, bh) = (((q[0] - p[0]) as i32).max(2), ((q[1] - p[1]) as i32).max(2));
        ui.draw_color(ui.color(T_CARD));
        ui.draw_rect(p[0] as i32, p[1] as i32, bw, bh);
        ui.draw_color(ui.color(T_ACCENT));
        ui.draw_frame(p[0] as i32, p[1] as i32, bw, bh, 0, ui.dp(1).max(1));
    }
}

fn row(ui: &mut Ui, s: &str) {
    ui.next_size(210, 0);
    ui.text(TS_BODY, s);
}

// Layout konfigurieren, every pass
fn layout_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.lay.dlg.dlg.clone();
    if !ui.dialog_begin(ID_LAYOUT, t("Layout konfigurieren"), &dlg) {
        return;
    }
    update_preview(app);
    let any_sel = !layout_cmds::chosen(app).is_empty();
    let d = &mut app.lay.dlg;
    ui.next_size(520, 0);
    ui.box_begin(ID_LAYOUT + 1, BOX_V, 6);
    ui.text(TS_BODY_STRONG, t("Platzierung"));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, tc("Stil|Layout"));
    let styles: Vec<&str> = STYLES.iter().map(|s| t(s)).collect();
    ui.next_size(250, 0);
    ui.combobox(ID_LAYOUT + 2, &mut d.style, &styles);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Richtung"));
    let dirs: Vec<&str> = if d.style == 2 { TREE_DIRS.iter().map(|s| t(s)).collect() } else { DIRS.iter().map(|s| t(s)).collect() };
    d.dir = d.dir.clamp(0, dirs.len() as i32 - 1);
    ui.next_size(250, 0);
    ui.next_flags(if d.style <= 2 { 0 } else { ST_DISABLED });
    ui.combobox(ID_LAYOUT + 3, &mut d.dir, &dirs);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, tc("Ausrichtung|Layout"));
    let al = if d.style == 1 && d.dir >= 2 { &ALIGN_H } else { &ALIGN_V };
    let aligns: Vec<&str> = al.iter().map(|s| tc(s)).collect();
    ui.next_size(250, 0);
    ui.next_flags(if d.style == 1 { 0 } else { ST_DISABLED });
    ui.combobox(ID_LAYOUT + 4, &mut d.align, &aligns);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Abstand (0,1 mm)"));
    ui.next_size(110, 0);
    ui.number_box(ID_LAYOUT + 5, &mut d.spacing, &Range::new(0, 2000, 5, 0));
    ui.box_end();
    ui.text(TS_BODY_STRONG, t("Verbinder"));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Routing-Stil"));
    let names: Vec<&str> = ROUTE_STYLES.iter().map(|s| tc(s)).collect();
    ui.next_size(250, 0);
    ui.combobox(ID_LAYOUT + 6, &mut d.route, &names);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Darstellung"));
    ui.next_size(250, 0);
    ui.combobox(ID_LAYOUT + 7, &mut d.look, &[t("Gerade"), t("Gebogen")]);
    ui.box_end();
    ui.checkbox(ID_LAYOUT + 8, t("Routing-Stil auf Verbinder anwenden"), &mut d.apply_route);
    ui.text(TS_BODY_STRONG, t("Einstellungen anwenden auf"));
    ui.box_begin(0, BOX_H, 16);
    ui.next_flags(if any_sel { 0 } else { ST_DISABLED });
    ui.radio(ID_LAYOUT + 9, t("Auswahl"), &mut d.scope, 0);
    ui.radio(ID_LAYOUT + 10, t("Aktuelle Seite"), &mut d.scope, 1);
    ui.box_end();
    if !any_sel {
        d.scope = 1;
    }
    ui.checkbox(ID_LAYOUT + 11, t("Seite vergrößern, damit die Zeichnung passt"), &mut d.enlarge);
    ui.checkbox(ID_LAYOUT + 12, tc("Seitenumbrüche vermeiden"), &mut d.avoid);
    ui.text(TS_BODY_STRONG, t("Vorschau"));
    ui.box_end();
    draw_preview(app, ui, ID_LAYOUT + 13);
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        apply_layout(app, ui);
    }
    if r != DLG_NONE {
        app.lay.dlg.dlg.set(0);
        ui.repass();
    }
}

// OK: the page's cells, the layout, the connectors, the breaks, the page (one undo step)
fn apply_layout(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(page) = app.model.page else { return };
    let only = if app.lay.dlg.scope == 0 { layout_cmds::chosen(app) } else { Vec::new() };
    let d = &app.lay.dlg;
    let (route, ext) = (d.route as i64, if d.look == 1 { 2 } else { 1 });
    let (apply, avoid, enlarge) = (d.apply_route != 0, d.avoid != 0, d.enlarge != 0);
    if avoid != layout_cmds::avoid_on(app) {
        layout_cmds::toggle_avoid(app, ui);
    }
    let Some(o) = app.opened.as_mut() else { return };
    let opts = opts_of(&app.lay.dlg, &o.doc, page);
    let routing = if apply { Routing::Apply { route, ext } } else { Routing::Follow };
    let _ = edit_doc::edit(o, t("Seitenlayout neu anordnen"), true, |tx, eng| {
        layout::store(tx, eng, page, &opts, Some((route, ext)), Some(enlarge))?;
        let moved = layout::relayout(tx, eng, Owner::Page(page), &only, &opts, routing)?;
        layout_cmds::finish(tx, eng, page, &opts, &moved, avoid, enlarge)
    });
    ui.repass();
}

/// Abstandsoptionen with the spacing in use
pub(crate) fn open_spacing(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(page) = app.model.page else { return };
    let Some(o) = app.opened.as_ref() else { return };
    let k = diagram_core::pagesetup::scale(&o.doc, page);
    let page_sp = Opts::from_page(&o.doc, page).spacing;
    let own = app.lay.spacing;
    let d = &mut app.lay.dlg;
    d.sp_own = own.is_some() as i32;
    let sp = own.unwrap_or([page_sp[0] * k, page_sp[1] * k]);
    d.sp = [tenths(sp[0]), tenths(sp[1])];
    d.sp_dlg.set(1);
    ui.repass();
}

fn spacing_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.lay.dlg.sp_dlg.clone();
    if !ui.dialog_begin(ID_SPACING, tc("Abstandsoptionen"), &dlg) {
        return;
    }
    let d = &mut app.lay.dlg;
    ui.next_size(420, 0);
    ui.box_begin(ID_SPACING + 1, BOX_V, 6);
    ui.radio(ID_SPACING + 2, t("Standardabstand der Seite verwenden"), &mut d.sp_own, 0);
    ui.radio(ID_SPACING + 3, t("Diesen Abstand verwenden"), &mut d.sp_own, 1);
    let rg = Range::new(0, 2000, 5, 0);
    for (k, label) in [t("Horizontal (0,1 mm)"), t("Vertikal (0,1 mm)")].into_iter().enumerate() {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        row(ui, label);
        ui.next_size(110, 0);
        ui.next_flags(if d.sp_own == 1 { 0 } else { ST_DISABLED });
        ui.number_box(ID_SPACING + 4 + k as u32, &mut d.sp[k], &rg);
        ui.box_end();
    }
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        let d = &app.lay.dlg;
        app.lay.spacing = (d.sp_own == 1).then(|| [d.sp[0].max(0) as f64 / 254.0, d.sp[1].max(0) as f64 / 254.0]);
        let v = app.lay.spacing.map_or(alloc::string::String::new(), |s| alloc::format!("{} {}", s[0], s[1]));
        app.opts.ini_mut().set("layout", "spacing", &v);
        app.opts.save();
    }
    if r != DLG_NONE {
        app.lay.dlg.sp_dlg.set(0);
        ui.repass();
    }
}

/// the spacing of Abstandsoptionen from the settings (at start)
pub(crate) fn load_spacing(app: &mut Diagrammer) {
    let v = app.opts.ini().get("layout", "spacing").unwrap_or_default();
    let mut it = v.split_whitespace().filter_map(|x| x.parse::<f64>().ok()).filter(|x| x.is_finite() && *x >= 0.0);
    app.lay.spacing = match (it.next(), it.next()) {
        (Some(a), Some(b)) => Some([a, b]),
        _ => None,
    };
}

/// Weitere Verteilungsoptionen: Shapes verteilen
pub(crate) fn open_distribute(app: &mut Diagrammer, ui: &mut Ui) {
    app.lay.dlg.dist_dlg.set(1);
    ui.repass();
}

fn distribute_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.lay.dlg.dist_dlg.clone();
    if !ui.dialog_begin(ID_DIST, t("Shapes verteilen"), &dlg) {
        return;
    }
    let d = &mut app.lay.dlg;
    ui.next_size(440, 0);
    ui.box_begin(ID_DIST + 1, BOX_H, 24);
    for (axis, (head, names)) in [
        (t("Horizontal verteilen"), [t("Keine"), t("Gleicher Abstand"), t("Linke Kanten"), t("Mitten"), t("Rechte Kanten")]),
        (t("Vertikal verteilen"), [t("Keine"), t("Gleicher Abstand"), t("Unterkanten"), t("Mitten"), t("Oberkanten")]),
    ]
    .into_iter()
    .enumerate()
    {
        ui.box_begin(0, BOX_V, 6);
        ui.text(TS_BODY_STRONG, head);
        for (k, name) in names.into_iter().enumerate() {
            ui.radio(ID_DIST + 2 + axis as u32 * 8 + k as u32, name, &mut d.dist[axis], k as i32);
        }
        ui.box_end();
    }
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        distribute(app, ui);
    }
    if r != DLG_NONE {
        app.lay.dlg.dist_dlg.set(0);
        ui.repass();
    }
}

/// the moves that distribute boxes along `axis` (0 across, 1 up): `how` 1 equal gaps between
/// the outer two, 2 / 3 / 4 the low edges, middles, high edges evenly between the outer two
pub(crate) fn distribution(boxes: &[(ShapeRef, [f64; 4])], axis: usize, how: i32) -> Vec<(ShapeRef, f64)> {
    let n = boxes.len();
    if n < 3 || !(1..=4).contains(&how) {
        return Vec::new();
    }
    let key = |b: &[f64; 4]| match how {
        2 => b[axis],
        4 => b[axis + 2],
        _ => (b[axis] + b[axis + 2]) * 0.5,
    };
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| key(&boxes[a].1).total_cmp(&key(&boxes[b].1)).then(a.cmp(&b)));
    let (first, last) = (boxes[order[0]].1, boxes[order[n - 1]].1);
    let mut out = Vec::new();
    if how == 1 {
        let total: f64 = boxes.iter().map(|(_, b)| b[axis + 2] - b[axis]).sum();
        let gap = ((last[axis + 2] - first[axis]) - total) / (n - 1) as f64;
        let mut at = first[axis];
        for &i in &order {
            let b = boxes[i].1;
            out.push((boxes[i].0, at - b[axis]));
            at += b[axis + 2] - b[axis] + gap;
        }
    } else {
        let (a, z) = (key(&first), key(&last));
        for (k, &i) in order.iter().enumerate() {
            let want = a + (z - a) * k as f64 / (n - 1) as f64;
            out.push((boxes[i].0, want - key(&boxes[i].1)));
        }
    }
    out.retain(|m| m.1.abs() > 1e-9);
    out
}

// OK of Shapes verteilen: one undo step
fn distribute(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(page) = app.model.page else { return };
    let sel = edit_canvas::selected(app);
    let how = app.lay.dlg.dist;
    let Some(o) = app.opened.as_mut() else { return };
    let boxes: Vec<(ShapeRef, [f64; 4])> = sel.iter().filter_map(|&s| Some((s, ops::bounds(&o.doc, &[s])?))).collect();
    let mut moves: Vec<(ShapeRef, [f64; 2])> = Vec::new();
    for axis in 0..2 {
        for (s, d) in distribution(&boxes, axis, how[axis]) {
            match moves.iter_mut().find(|m| m.0 == s) {
                Some(m) => m.1[axis] += d,
                None => {
                    let mut v = [0.0; 2];
                    v[axis] = d;
                    moves.push((s, v));
                }
            }
        }
    }
    if moves.is_empty() {
        return;
    }
    let owner = Owner::Page(page);
    let _ = edit_doc::edit(o, t("Shapes verteilen"), true, |tx, eng| {
        let moved = layout::move_nodes(tx, eng, owner, &moves)?;
        layout::reroute(tx, eng, owner, &moved, Routing::Follow, false)
    });
    ui.repass();
}

/// every pass: the open dialog
pub(crate) fn dialogs(app: &mut Diagrammer, ui: &mut Ui) {
    layout_dialog(app, ui);
    spacing_dialog(app, ui);
    distribute_dialog(app, ui);
}
