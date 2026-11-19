// edit_canvas -- (JD7) editing on the canvas: the page drawn by diagram_view (viewer.rs), the
// view of ui.diagram (DG_EDIT), the gestures of ui.shape_edit turned into cell writes of
// diagram_core (ops, glue), the rulers, guides and page breaks
//
//   canvas(app, ui)            the canvas area: rulers (Ansicht › Lineal), the canvas, the edit
//                              layer, page breaks, the text being edited (edit_text.rs)
//   Geo                        the page's coordinates: world (DIP at 100 %, y down, the page's
//                              top left corner at 0) <-> page (drawing units in inches, y up)
//                              <-> paper (inches, y up: the display list)
//   model(app)                 the shapes as ui.shape_edit sees them (frames, ends, pins, locks,
//                              glue, connection points; control points and vertices of the
//                              selection), made again when the drawing, the page, the selection
//                              or the tool change
//   apply(app, ui, ev)         what the user did -> one transaction each (edit_doc::edit); a
//                              drag is applied as a preview at every step, recorded when released
//   tool_of(app)               the command of the active tool -> SE_* (pointer, connector, line,
//                              rectangle, ellipse, text)
//   selected(app)              the selected shapes (ShapeRef)
//
// Keys: a shape's key is its slot + 1 in the page's tree; a guide's key the same (guides are
// shapes of type Guide, diagram_core::render does not draw them, the edit layer does).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::geometry::Xform;
use diagram_core::{glue, hit, ops, CellAddr, Document, Owner, PageRef, RowKey, SecKey, ShapeKind, ShapeRef, SheetRef, Unit, Value};
use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::edit_doc;
use crate::viewer::DIP;
use crate::Diagrammer;

const ID_RULER_TOP: u32 = 0x5B00;
const ID_RULER_LEFT: u32 = 0x5B01;
const RULER_DIP: i32 = 20;

/// a page's coordinates (see the file header)
#[derive(Clone, Copy, Debug)]
pub(crate) struct Geo {
    /// the page in paper inches
    pub w: f64,
    pub h: f64,
    /// paper inches per drawing inch
    pub scale: f64,
}

impl Geo {
    /// a page point (drawing units) in world units
    pub fn world(&self, p: [f64; 2]) -> [f32; 2] {
        [(p[0] * self.scale * DIP) as f32, ((self.h - p[1] * self.scale) * DIP) as f32]
    }
    /// a world point on the page (drawing units)
    pub fn page(&self, w: [f32; 2]) -> [f64; 2] {
        [w[0] as f64 / DIP / self.scale, (self.h - w[1] as f64 / DIP) / self.scale]
    }
    /// a world point on the paper (the display list's inches)
    pub fn paper(&self, w: [f32; 2]) -> [f64; 2] {
        [w[0] as f64 / DIP, self.h - w[1] as f64 / DIP]
    }
    /// a world vector as a page vector
    pub fn page_vec(&self, v: [f32; 2]) -> [f64; 2] {
        [v[0] as f64 / DIP / self.scale, -(v[1] as f64) / DIP / self.scale]
    }
    /// a paper box x0, y0, x1, y1 (y up) in world units x0, y0, x1, y1 (y down)
    pub fn world_box(&self, b: [f64; 4]) -> [f32; 4] {
        [(b[0] * DIP) as f32, ((self.h - b[3]) * DIP) as f32, (b[2] * DIP) as f32, ((self.h - b[1]) * DIP) as f32]
    }
}

// the square root without libm (x87 / portable pow)
fn sqrt(x: f64) -> f64 {
    if x > 0.0 { rt::x87::pow(x, 0.5) } else { 0.0 }
}

/// a shape's key in the edit layer
pub(crate) fn key_of(r: ShapeRef) -> u64 {
    r.slot as u64 + 1
}

/// the shape of a key on page `page`
pub(crate) fn shape_of(page: PageRef, key: u64) -> ShapeRef {
    ShapeRef { owner: Owner::Page(page), slot: key.saturating_sub(1) as u32 }
}

/// the shapes the edit layer sees, made for one state of the drawing
#[derive(Default)]
pub(crate) struct Model {
    /// what it was made for: drawing serial, page, revision, tool, the selection
    key: (u32, usize, u32, u32, Vec<u64>),
    pub shapes: Vec<SeShape>,
    pub points: Vec<SePoint>,
    pub guides: Vec<SeGuide>,
    pub ver: u32,
    pub geo: Option<Geo>,
    /// the page shown
    pub page: Option<PageRef>,
    /// the rows of the control points listed (a control point's tag is its index here)
    pub controls: Vec<RowKey>,
}

/// the tool of the active command
pub(crate) fn tool_of(app: &Diagrammer) -> u32 {
    // (JD9) a tile of the Shapes panel armed: the next click places it (stencils_drop.rs)
    if app.stencils.armed.is_some() {
        return SE_PLACE;
    }
    match app.tool {
        CONNECTOR_TOOL => SE_CONNECT,
        LINE_TOOL | PENCIL => SE_LINE,
        // (JD8 rest) the freeform and the arc tool (cmds_draw.rs)
        FREEFORM => SE_FREEFORM,
        ARC => SE_ARC,
        RECTANGLE => SE_RECT,
        ELLIPSE => SE_ELLIPSE,
        TEXT_TOOL => SE_TEXT,
        _ => SE_POINTER,
    }
}

/// the selected shapes of the page shown
pub(crate) fn selected(app: &Diagrammer) -> Vec<ShapeRef> {
    let Some(page) = app.model.page else { return Vec::new() };
    app.se.sel.iter().map(|&k| shape_of(page, k)).collect()
}

// a frame of diagram_core (local -> page, w, h) as the edit layer's box
fn se_box(geo: &Geo, f: &ops::Frame) -> SeBox {
    let o = geo.world(f.xf.apply([0.0, f.h]));
    let x = geo.world(f.xf.apply([f.w, f.h]));
    let y = geo.world(f.xf.apply([0.0, 0.0]));
    SeBox { o, u: [x[0] - o[0], x[1] - o[1]], v: [y[0] - o[0], y[1] - o[1]] }
}

/// the edit layer's box as a frame of diagram_core
pub(crate) fn frame_of(geo: &Geo, b: &SeBox) -> ops::Frame {
    let o = geo.page(b.o);
    let u = geo.page_vec(b.u);
    let v = geo.page_vec(b.v);
    let w = sqrt(u[0] * u[0] + u[1] * u[1]);
    let h = sqrt(v[0] * v[0] + v[1] * v[1]);
    let (w1, h1) = (if w > 0.0 { w } else { 1.0 }, if h > 0.0 { h } else { 1.0 });
    // local x along u, local y against v (v runs from the top edge down), the origin at o + v
    let xf = Xform { a: u[0] / w1, b: u[1] / w1, c: -v[0] / h1, d: -v[1] / h1, e: o[0] + v[0], f: o[1] + v[1] };
    ops::Frame { xf, w, h }
}

// a cell's number with inheritance
fn num(doc: &Document, s: SheetRef, cell: &str) -> Option<f64> {
    doc.sym(cell).and_then(|c| doc.resolve(s, &CellAddr::single(c)).num())
}

/// make the model for the current state (cheap when nothing changed)
pub(crate) fn model(app: &mut Diagrammer) {
    let tool = tool_of(app);
    let k = app.page_index();
    let sel: Vec<u64> = app.se.sel.iter().copied().collect();
    let Some(o) = app.opened.as_mut() else {
        app.model = Model::default();
        return;
    };
    let Some(tab) = o.tabs.get(k) else {
        app.model = Model::default();
        return;
    };
    let page = tab.page;
    let key = (o.serial, k, o.ed.rev, tool, sel.clone());
    if app.model.key == key && app.model.geo.is_some() {
        return;
    }
    let Some(paint) = crate::viewer::paint(app, k) else { return };
    let o = app.opened.as_mut().expect("open");
    let view = paint.view();
    let d = view.drawing();
    let geo = Geo { w: d.width, h: d.height, scale: if d.scale > 0.0 { d.scale } else { 1.0 } };
    let doc = &o.doc;
    let owner = Owner::Page(page);
    let mut m = Model { key, geo: Some(geo), page: Some(page), ..Model::default() };
    m.ver = o.serial.wrapping_mul(7919).wrapping_add(o.ed.rev).wrapping_add(k as u32 * 104_729);
    // boxes of what is drawn, per top-level shape
    let boxes: BTreeMap<ShapeRef, [f64; 4]> = hit::top_boxes(d).into_iter().collect();
    // the groups whose children are selectable now: the selected groups (a second click picks a
    // child) and the groups around a selected child
    let mut open: Vec<ShapeRef> = Vec::new();
    for &sk in &sel {
        let r = shape_of(page, sk);
        if doc.shape(r).is_some_and(|s| s.header().kind == ShapeKind::Group) {
            open.push(r);
        }
        let mut cur = doc.shape(r).and_then(|s| s.parent());
        while let Some(p) = cur {
            open.push(p);
            cur = doc.shape(p).and_then(|s| s.parent());
        }
    }
    // (JD8 rest) the page's layers: hidden shapes are left out, locked ones not selectable
    let layer_flags = crate::layers::Flags::of(doc, page);
    let add = |r: ShapeRef, parent: u64, m: &mut Model, eng: &mut diagram_core::formula::Engine| {
        let Some(s) = doc.shape(r) else { return };
        if s.is_deleted() {
            return;
        }
        let layered = match &layer_flags {
            Some(lf) => match lf.flags(doc, r) {
                Some(f) => f,
                None => return,
            },
            None => 0,
        };
        let sheet = SheetRef::Shape(r);
        // guides: lines for the edit layer, no frame
        if s.header().kind == ShapeKind::Guide {
            if let Some(g) = guide_of(doc, &geo, r) {
                m.guides.push(g);
            }
            return;
        }
        let f = ops::frame(doc, r);
        let bx = se_box(&geo, &f);
        let mut flags = 0u32;
        let mut ends = [[0.0f32; 2]; 2];
        if glue::is_one_d(doc, r) {
            flags |= SEF_1D;
            if let Some((b, e)) = glue::ends(doc, r) {
                ends = [geo.world(b), geo.world(e)];
            }
            if glue::is_connector(doc, r) {
                flags |= SEF_DYNAMIC;
            }
            let g = glue::glued(doc, r);
            for (i, t) in g.iter().enumerate() {
                flags |= match (i, t) {
                    (0, glue::Target::Point { .. }) => SEF_BEGIN_GLUED,
                    (0, glue::Target::Shape(_)) => SEF_BEGIN_DYN,
                    (1, glue::Target::Point { .. }) => SEF_END_GLUED,
                    (1, glue::Target::Shape(_)) => SEF_END_DYN,
                    _ => 0,
                };
            }
        }
        if s.header().kind == ShapeKind::Group {
            flags |= SEF_GROUP;
        }
        flags |= lock_flags(&ops::locks(eng, doc, r));
        flags |= layered;
        // (JD10) no Verbindungspfeile on containers, lists, callouts
        if crate::structures_cmds::no_arrows(doc, r) {
            flags |= SEF_NOARROWS;
        }
        let lp = [num(doc, sheet, "LocPinX").unwrap_or(f.w * 0.5), num(doc, sheet, "LocPinY").unwrap_or(f.h * 0.5)];
        let pin = geo.world(f.xf.apply(lp));
        let bbox = boxes.get(&r).map(|b| geo.world_box(*b)).unwrap_or_else(|| {
            let b = bx.aabb();
            if flags & SEF_1D != 0 {
                [ends[0][0].min(ends[1][0]), ends[0][1].min(ends[1][1]), ends[0][0].max(ends[1][0]), ends[0][1].max(ends[1][1])]
            } else {
                b
            }
        });
        let k = key_of(r);
        let p0 = m.points.len() as u32;
        // connection points (top-level shapes: glue targets)
        if parent == 0 && flags & SEF_1D == 0 {
            for c in glue::connection_points(doc, r) {
                let dir = c.dir.map_or([0.0, 0.0], |d| [d[0] as f32, -(d[1] as f32)]);
                m.points.push(SePoint { at: geo.world(c.at), kind: SP_CONN, tag: c.row, dir, flags: 0 });
            }
        }
        // control points and vertices of the selection
        if sel.contains(&k) {
            for c in ops::controls(doc, r) {
                let tag = m.controls.len() as u32;
                m.controls.push(c.row);
                m.points.push(SePoint { at: geo.world(c.at), kind: SP_CONTROL, tag, dir: [0.0; 2], flags: if c.can_glue { SPF_GLUE } else { 0 } });
            }
            if tool == SE_LINE && sel.len() == 1 {
                for v in ops::vertices(doc, r) {
                    m.points.push(SePoint { at: geo.world(v.at), kind: SP_VERTEX, tag: v.section << 16 | (v.row & 0xFFFF), dir: [0.0; 2], flags: 0 });
                }
            }
        }
        let p1 = m.points.len() as u32;
        m.shapes.push(SeShape { key: k, parent, bx, ends, pin, bbox, flags, points: p0..p1 });
    };
    let tops: Vec<ShapeRef> = doc.top_shapes(owner).collect();
    for &r in &tops {
        add(r, 0, &mut m, &mut o.ed.eng);
    }
    // the children of the open groups (sub-selection)
    open.sort();
    open.dedup();
    for g in open.iter() {
        let kids: Vec<ShapeRef> = doc.shape(*g).map(|s| s.children().collect()).unwrap_or_default();
        for c in kids {
            add(c, key_of(*g), &mut m, &mut o.ed.eng);
        }
    }
    drop(view);
    app.model = m;
}

fn lock_flags(l: &ops::Locks) -> u32 {
    use ops::Locks as L;
    let pairs = [(L::WIDTH, SEF_LOCK_W), (L::HEIGHT, SEF_LOCK_H), (L::ASPECT, SEF_LOCK_ASPECT), (L::ROTATE, SEF_LOCK_ROTATE), (L::BEGIN, SEF_LOCK_BEGIN), (L::END, SEF_LOCK_END), (L::SELECT, SEF_NOSELECT)];
    let mut f = pairs.iter().filter(|(b, _)| l.has(*b)).fold(0, |m, (_, s)| m | s);
    if l.has(L::MOVE_X) && l.has(L::MOVE_Y) {
        f |= SEF_LOCK_MOVE;
    }
    f
}

// a guide shape as a line: vertical when its line runs up and down on the page
fn guide_of(doc: &Document, geo: &Geo, r: ShapeRef) -> Option<SeGuide> {
    let sheet = SheetRef::Shape(r);
    let xf = doc.page_xform(r);
    let geo_sec = doc.geometry(sheet).into_iter().next();
    let mut dir = [1.0, 0.0];
    if let Some(sk) = geo_sec {
        for row in doc.rows(sheet, sk) {
            if row.kind.map(|k| doc.str(k)) == Some("InfiniteLine") {
                let c = |n: &str| doc.sym(n).and_then(|c| doc.resolve(sheet, &CellAddr::row(sk, row.key, c)).num()).unwrap_or(0.0);
                dir = [c("A") - c("X"), c("B") - c("Y")];
                break;
            }
        }
    }
    let d = xf.apply_vec(dir);
    let at = xf.apply([0.0, 0.0]);
    let w = geo.world(at);
    let vertical = d[1].abs() > d[0].abs();
    Some(SeGuide { key: key_of(r), vertical, at: if vertical { w[0] } else { w[1] } })
}

// ---------------------------------------------------------------- the canvas

/// the canvas area: rulers, the canvas, the edit layer; `menu` runs right after the edit layer
/// (the context menu of the canvas: the edit layer selects on a right click first)
pub(crate) fn canvas(app: &mut Diagrammer, ui: &mut Ui, id: u32, menu: impl FnOnce(&mut Diagrammer, &mut Ui)) {
    let rulers = app.view_rulers;
    model(app);
    if rulers {
        // the corner and the top ruler
        ui.next_size(-1, RULER_DIP);
        ui.box_begin(0, BOX_H, 0);
        ui.next_size(RULER_DIP, RULER_DIP);
        ui.box_begin(0, BOX_V | BOX_LAYER, 0);
        ui.box_end();
        ruler(app, ui, false);
        ui.box_end();
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_H, 0);
        ruler(app, ui, true);
    }
    // the grid snaps whether it is shown or not (Visio's)
    let grid = grid_step(app);
    app.canvas.grid_lines = app.view_grid;
    let st = &mut app.canvas;
    let name = t("Zeichenfläche");
    if st.name != name {
        st.name = name.into();
    }
    st.grid = grid;
    st.snap = app.snap;
    let k = app.page_index();
    let paint = crate::viewer::paint(app, k);
    let nodes: Vec<DNode> = crate::viewer::sheet_node(app, k).into_iter().collect();
    let ver = app.opened.as_ref().map_or(0, |o| crate::viewer::canvas_ver(o.serial, k, o.ed.rev));
    let dm = DModel { nodes: &nodes, ports: &[], edges: &[], wps: &[], ver: app.opened.as_ref().map_or(0, |o| o.serial.wrapping_mul(4096).wrapping_add(k as u32 + 1)) };
    let paint: Option<Rc<dyn DgPaint>> = paint.map(|p| p as Rc<dyn DgPaint>);
    ui.next_size(-1, -1);
    ui.diagram(id, &mut app.canvas, &dm, DG_EDIT, paint, ver);
    // (JD9) the canvas takes the Shapes panel's tiles (its rectangle is the last one)
    crate::stencils_drop::drop_target(app, ui, id);
    // the edit layer; (JD10) the insert triangles of the lists
    let marks = crate::structures::marks(app);
    app.se.arrows = app.structs.arrows && app.opts.get(&crate::options::CONNECT_ARROWS);
    let tool = tool_of(app);
    let evs = {
        let m = &app.model;
        let page = m.page;
        let geo = m.geo;
        let opened = app.opened.as_ref();
        let pv = opened.and_then(|o| o.paint_of(k));
        let view = pv.as_ref().map(|p| p.view());
        let doc = opened.map(|o| &o.doc);
        let hitf = |p: [f32; 2], tol: f32, within: u64| -> u64 {
            let (Some(view), Some(geo), Some(page), Some(doc)) = (view.as_ref(), geo, page, doc) else { return 0 };
            hit_at(doc, view.drawing(), &geo, page, p, tol, within)
        };
        let sm = SeModel { shapes: &m.shapes, points: &m.points, guides: &m.guides, marks: &marks, ver: m.ver, hit: &hitf };
        ui.shape_edit(id, &mut app.se, &mut app.canvas, &sm, tool)
    };
    for ev in evs {
        apply(app, ui, ev);
    }
    // (JD10) the selection's order, the glow of a container under a dragged shape
    crate::structures::sync_order(app);
    crate::structures::overlay(app, ui);
    menu(app, ui);
    // one record per shape and per handle (scripts, screen readers)
    {
        let m = &app.model;
        let page = m.page;
        let doc = app.opened.as_ref().map(|o| &o.doc);
        let label = |key: u64| -> String {
            match (doc, page) {
                (Some(doc), Some(page)) => shape_label(doc, shape_of(page, key)),
                _ => String::new(),
            }
        };
        let nohit = |_: [f32; 2], _: f32, _: u64| 0u64;
        let sm = SeModel { shapes: &m.shapes, points: &m.points, guides: &m.guides, marks: &marks, ver: m.ver, hit: &nohit };
        ui.shape_edit_sems(id, &mut app.se, &app.canvas, &sm, &label);
    }
    // page breaks, a guide being dragged out of a ruler
    if app.view_breaks {
        page_breaks(app, ui);
    }
    if let Some((horizontal, at)) = app.guide_drag {
        let vp = app.canvas.vp;
        let r = vp.rect;
        ui.clip_push(r[0], r[1], r[2], r[3]);
        ui.draw_color(0xFF3D_8BFD);
        let wr = vp.world_rect();
        let (a, b) = if horizontal { ([wr[0], at], [wr[2], at]) } else { ([at, wr[1]], [at, wr[3]]) };
        dashed(ui, vp.to_screen(a), vp.to_screen(b));
        ui.clip_pop();
    }
    // the text being edited
    crate::edit_text::frame(app, ui);
    if rulers {
        ui.box_end();
    }
}

/// a shape's name for scripts and screen readers: its text (the first line), else its name
pub(crate) fn shape_label(doc: &Document, s: ShapeRef) -> String {
    let Some(sh) = doc.shape(s) else { return String::new() };
    let text = sh.text().map(|t| t.chars()).unwrap_or_default();
    let line = text.split(['\n', '\u{2028}', '\u{2029}']).find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if !line.is_empty() {
        return String::from(line);
    }
    sh.name()
}

/// whether the selection is a container (User.msvStructureType = "Container"; JD10: a list too,
/// not a flowchart's pool, which has its own register)
pub(crate) fn container_selected(app: &Diagrammer) -> bool {
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return false };
    let doc = &o.doc;
    app.se.sel.iter().any(|&k| {
        let s = shape_of(page, k);
        diagram_core::structure::is_container(doc, s) && !diagram_core::structure::cff::is_pool(doc, s)
    })
}

// the stretch of the outline nearest to p: its section and the row of the vertex it starts at
fn nearest_stretch(doc: &Document, s: ShapeRef, p: [f64; 2]) -> Option<(u32, u32)> {
    let vs = ops::vertices(doc, s);
    let mut best: Option<(f64, u32, u32)> = None;
    for w in vs.windows(2) {
        if w[0].section != w[1].section {
            continue;
        }
        let (a, b) = (w[0].at, w[1].at);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let (qx, qy) = (a[0] + dx * t - p[0], a[1] + dy * t - p[1]);
        let d = qx * qx + qy * qy;
        if best.is_none_or(|x| d < x.0) {
            best = Some((d, w[0].section, w[0].row));
        }
    }
    best.map(|b| (b.1, b.2))
}

// the grid step in world units: a round number of the page's unit about 16 DIP at this zoom
fn grid_step(app: &Diagrammer) -> f32 {
    let Some(geo) = app.model.geo else { return 16.0 };
    let metric = metric(app);
    // world units per drawing unit (mm or inch)
    let unit = if metric { DIP * geo.scale / 25.4 } else { DIP * geo.scale };
    let zoom = app.canvas.vp.zoom.max(0.1) as f64;
    let want = 16.0 / zoom; // world units
    let steps: &[f64] = if metric { &[0.5, 1.0, 2.0, 2.5, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0] } else { &[1.0 / 32.0, 1.0 / 16.0, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 6.0, 12.0, 24.0, 60.0, 120.0] };
    let s = steps.iter().map(|s| s * unit).find(|&s| s >= want).unwrap_or(steps[steps.len() - 1] * unit);
    s as f32
}

/// whether the page shows millimetres (else inches)
pub(crate) fn metric(app: &Diagrammer) -> bool {
    let Some(o) = app.opened.as_ref() else { return true };
    let Some(tab) = o.tabs.get(app.page_index()) else { return true };
    let doc = &o.doc;
    let s = SheetRef::Page(tab.page);
    doc.sym("PageWidth").map(|c| doc.resolve(s, &CellAddr::single(c)).unit()).map_or(true, |u| matches!(u, Some(Unit::Millimeters) | Some(Unit::Centimeters) | Some(Unit::Meters)))
}

// a ruler: millimetres or inches of the drawing, 0 at the page's bottom left corner
fn ruler(app: &mut Diagrammer, ui: &mut Ui, vertical: bool) {
    let Some(geo) = app.model.geo else {
        ui.next_size(if vertical { RULER_DIP } else { -1 }, if vertical { -1 } else { RULER_DIP });
        ui.box_begin(0, BOX_V | BOX_LAYER, 0);
        ui.box_end();
        return;
    };
    let metric = metric(app);
    let per_inch = if metric { 25.4 } else { 1.0 };
    // units per world unit
    let k = per_inch / (DIP * geo.scale);
    let mut marks: Vec<f32> = Vec::new();
    if app.se.busy() || !app.se.sel.is_empty() {
        let mut b: Option<[f32; 4]> = None;
        for s in &app.model.shapes {
            if app.se.sel.contains(&s.key) {
                let r = s.bbox;
                b = Some(match b {
                    None => r,
                    Some(a) => [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])],
                });
            }
        }
        if let Some(b) = b {
            let (a, z) = if vertical { (b[1], b[3]) } else { (b[0], b[2]) };
            marks = alloc::vec![a, (a + z) * 0.5, z];
        }
    }
    let vp = app.canvas.vp;
    let (mx, my) = ui.mouse();
    let inside = mx >= vp.rect[0] && my >= vp.rect[1] && mx < vp.rect[0] + vp.rect[2] && my < vp.rect[1] + vp.rect[3];
    let pw = vp.to_world(mx as f32, my as f32);
    let spec = RulerSpec {
        vertical,
        origin: if vertical { (geo.h * DIP) as f32 } else { 0.0 },
        per_world: if vertical { -k } else { k },
        page: if vertical { [0.0, (geo.h * DIP) as f32] } else { [0.0, (geo.w * DIP) as f32] },
        marks: &marks,
        pointer: inside.then_some(if vertical { pw[1] } else { pw[0] }),
    };
    ui.next_size(if vertical { RULER_DIP } else { -1 }, if vertical { -1 } else { RULER_DIP });
    let ev = ui.ruler(if vertical { ID_RULER_LEFT } else { ID_RULER_TOP }, &vp, &spec);
    match ev {
        RulerEv::Guide { at, done: true } => add_guide(app, ui, !vertical, at),
        RulerEv::Guide { at, done: false } => app.guide_drag = Some((!vertical, at)),
        RulerEv::GuideCancel => app.guide_drag = None,
        _ => {}
    }
}

// a guide dropped from a ruler: horizontal (from the top ruler) at world y, or vertical at x
fn add_guide(app: &mut Diagrammer, ui: &mut Ui, horizontal: bool, at: f32) {
    app.guide_drag = None;
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return };
    let p = geo.page(if horizontal { [0.0, at] } else { [at, 0.0] });
    let Some(o) = app.opened.as_mut() else { return };
    let style = o.doc.settings.default_guide_style;
    let r = edit_doc::edit(o, t("Führungslinie"), true, |tx, eng| {
        let s = tx.add_shape(Owner::Page(page), None, ShapeKind::Guide)?;
        let sh = SheetRef::Shape(s);
        let set = |tx: &mut diagram_core::Transaction, n: &str, v: f64, u: Unit| {
            let a = tx.single(n);
            tx.set_length(sh, &a, v, u)
        };
        set(tx, "PinX", if horizontal { 0.0 } else { p[0] }, Unit::Inches)?;
        set(tx, "PinY", if horizontal { p[1] } else { 0.0 }, Unit::Inches)?;
        set(tx, "Width", 0.0, Unit::Inches)?;
        set(tx, "Height", 0.0, Unit::Inches)?;
        set(tx, "LocPinX", 0.0, Unit::Inches)?;
        set(tx, "LocPinY", 0.0, Unit::Inches)?;
        set(tx, "Angle", if horizontal { 0.0 } else { core::f64::consts::FRAC_PI_2 }, Unit::Radians)?;
        if style.is_some() {
            tx.set_shape_styles(s, diagram_core::StyleRefs { line: style, fill: style, text: style })?;
        }
        let gname = tx.intern("Geometry");
        let sk = SecKey { name: gname, index: 0 };
        let a = tx.add_section(sh, "Geometry")?;
        let _ = sk;
        let nf = tx.intern("NoFill");
        tx.set_constant(sh, &CellAddr::section(a, nf), Value::Num(1.0), None)?;
        let row = tx.add_row(sh, a, Some(RowKey::Index(1)), Some("InfiniteLine"))?;
        for (n, v) in [("X", 0.0), ("Y", 0.0), ("A", 1.0), ("B", 0.0)] {
            let c = tx.intern(n);
            tx.set_length(sh, &CellAddr::row(a, row, c), v, Unit::Inches)?;
        }
        let _ = eng;
        Ok(())
    });
    if let Err(e) = r {
        app.error = Some((String::from(t("Die Führungslinie lässt sich nicht anlegen.")), format!("{e:?}")));
    }
    ui.repass();
}

// page breaks: where the printer's paper ends on a page larger than it (ZF-01), dashed
fn page_breaks(app: &mut Diagrammer, ui: &mut Ui) {
    let (Some(geo), Some(o)) = (app.model.geo, app.opened.as_ref()) else { return };
    let Some(tab) = o.tabs.get(app.page_index()) else { return };
    let doc = &o.doc;
    let s = SheetRef::Page(tab.page);
    // the paper: PaperKind 9 = A4, 1 = Letter (others: A4), PrintPageOrientation 2 = landscape
    let kind = num(doc, s, "PaperKind").unwrap_or(9.0) as i32;
    let (mut pw, mut ph) = match kind {
        1 => (8.5, 11.0),
        5 => (8.5, 14.0),
        8 => (297.0 / 25.4, 420.0 / 25.4),
        11 => (148.0 / 25.4, 210.0 / 25.4),
        _ => (210.0 / 25.4, 297.0 / 25.4),
    };
    if num(doc, s, "PrintPageOrientation").unwrap_or(1.0) as i32 == 2 {
        core::mem::swap(&mut pw, &mut ph);
    }
    let margin = |n: &str| num(doc, s, n).unwrap_or(0.25);
    let (uw, uh) = (pw - margin("PageLeftMargin") - margin("PageRightMargin"), ph - margin("PageTopMargin") - margin("PageBottomMargin"));
    if !(uw > 0.1 && uh > 0.1) || (geo.w <= pw + 1e-6 && geo.h <= ph + 1e-6) {
        return;
    }
    let vp = app.canvas.vp;
    let r = vp.rect;
    ui.clip_push(r[0], r[1], r[2], r[3]);
    let c = 0xFF9A_A3AD;
    ui.draw_color(c);
    let mut x = uw;
    while x < geo.w - 1e-6 {
        let a = vp.to_screen([(x * DIP) as f32, 0.0]);
        let b = vp.to_screen([(x * DIP) as f32, (geo.h * DIP) as f32]);
        dashed(ui, a, b);
        x += uw;
    }
    let mut y = uh;
    while y < geo.h - 1e-6 {
        let wy = ((geo.h - y) * DIP) as f32;
        let a = vp.to_screen([0.0, wy]);
        let b = vp.to_screen([(geo.w * DIP) as f32, wy]);
        dashed(ui, a, b);
        y += uh;
    }
    ui.clip_pop();
}

fn dashed(ui: &mut Ui, a: [f32; 2], b: [f32; 2]) {
    let len = sqrt(((b[0] - a[0]) * (b[0] - a[0]) + (b[1] - a[1]) * (b[1] - a[1])) as f64) as f32;
    if !(len > 1.0) {
        return;
    }
    let (on, off) = (ui.dp(6) as f32, ui.dp(4) as f32);
    let w = (ui.hair().max(1) * 256) as i32;
    let mut s = 0.0;
    let mut n = 0;
    while s < len && n < 2000 {
        let e = (s + on).min(len);
        let p = [a[0] + (b[0] - a[0]) * s / len, a[1] + (b[1] - a[1]) * s / len];
        let q = [a[0] + (b[0] - a[0]) * e / len, a[1] + (b[1] - a[1]) * e / len];
        ui.draw_line((p[0] * 256.0) as i32, (p[1] * 256.0) as i32, (q[0] * 256.0) as i32, (q[1] * 256.0) as i32, w);
        s += on + off;
        n += 1;
    }
}

// the topmost shape at a world point: a top-level one, or the child of group `within`
fn hit_at(doc: &Document, d: &diagram_core::render::Drawing, geo: &Geo, page: PageRef, p: [f32; 2], tol: f32, within: u64) -> u64 {
    let pp = geo.paper(p);
    for h in hit::all_at(d, pp, tol as f64 / DIP) {
        if h.shape.owner != Owner::Page(page) {
            continue;
        }
        // (JD10) a container's inside is clicked through (ST-01)
        if within == 0 && crate::structures::click_through(doc, h.top, geo.page(p), tol as f64 / DIP / geo.scale) {
            continue;
        }
        if within == 0 {
            return key_of(h.top);
        }
        // the child of `within` on the way from the hit shape up
        let g = shape_of(page, within);
        let mut cur = Some(h.shape);
        while let Some(c) = cur {
            let parent = doc.shape(c).and_then(|s| s.parent());
            if parent == Some(g) {
                return key_of(c);
            }
            cur = parent;
        }
    }
    0
}

// ---------------------------------------------------------------- what the user did

/// apply an event of the edit layer
pub(crate) fn apply(app: &mut Diagrammer, ui: &mut Ui, ev: SeEv) {
    // (JD9) the click of an armed tile (SE_PLACE): the shape goes there (stencils_drop.rs)
    if let SeEv::Draw { tool: SE_PLACE, a, .. } = ev {
        crate::stencils_drop::clicked(app, ui, a);
        return;
    }
    // (JD8 rest) a freeform stroke, an arc (cmds_draw.rs)
    if crate::cmds_draw::apply(app, ui, &ev) {
        return;
    }
    // (JD10) a running move (the glow), the Verbindungspfeile, the insert triangles
    crate::structures::track(app, &ev);
    match ev {
        SeEv::Arrow { key, dir } => return crate::structures_cmds::arrow(app, ui, key, dir),
        SeEv::Mark { tag } => return crate::structures::insert_at(app, ui, tag),
        _ => {}
    }
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return };
    let owner = Owner::Page(page);
    let refs = |keys: &[u64]| -> Vec<ShapeRef> { keys.iter().map(|&k| shape_of(page, k)).collect() };
    let mut select: Option<Vec<u64>> = None;
    let Some(o) = app.opened.as_mut() else { return };
    let r: Result<(), diagram_core::EditError> = match ev {
        SeEv::Select => Ok(()),
        SeEv::Move { keys, total, done } => {
            let shapes = refs(&keys);
            let d = geo.page_vec(total);
            // (JD10) members and callouts come along; released: containers, lists
            edit_doc::edit(o, t("Verschieben"), done, |tx, eng| crate::structures::move_shapes(tx, eng, owner, &shapes, d, done))
        }
        SeEv::Resize { keys, from, to, done } => {
            let shapes = refs(&keys);
            let single = shapes.len() == 1 && o.doc.shape(shapes[0]).is_some();
            edit_doc::edit(o, t("Größe ändern"), done, |tx, eng| {
                if single && keys.len() == 1 && app_frame_matches(tx.doc(), &geo, shapes[0], &from) {
                    ops::set_frame(tx, eng, shapes[0], &frame_of(&geo, &to))?;
                } else {
                    let (a, b) = (geo.page(from.o), geo.page(from.at(1.0, 1.0)));
                    let (c, e) = (geo.page(to.o), geo.page(to.at(1.0, 1.0)));
                    let fb = [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])];
                    let tb = [c[0].min(e[0]), c[1].min(e[1]), c[0].max(e[0]), c[1].max(e[1])];
                    ops::scale_shapes(tx, eng, &shapes, fb, tb)?;
                }
                follow(tx, eng, owner, &shapes)?;
                crate::structures::changed(tx, eng, owner, &shapes)
            })
        }
        SeEv::Rotate { keys, centre, angle, done } => {
            let shapes = refs(&keys);
            let c = geo.page(centre);
            edit_doc::edit(o, t("Drehen"), done, |tx, eng| {
                ops::rotate_shapes(tx, eng, &shapes, c, angle as f64)?;
                follow(tx, eng, owner, &shapes)?;
                crate::structures::changed(tx, eng, owner, &shapes)
            })
        }
        SeEv::Pin { key, at, done } => {
            let s = shape_of(page, key);
            let p = geo.page(at);
            edit_doc::edit(o, t("Drehpunkt verschieben"), done, |tx, eng| ops::set_pin(tx, eng, s, p))
        }
        SeEv::End { key, end, at, glue: g, done } => {
            let s = shape_of(page, key);
            let p = geo.page(at);
            let target = target_of(page, g);
            let e = if end == 0 { glue::End::Begin } else { glue::End::End };
            edit_doc::edit(o, t("Endpunkt verschieben"), done, |tx, eng| {
                glue::glue(tx, eng, s, e, p, target)?;
                glue::update(tx, eng, &[s])
            })
        }
        SeEv::Point { key, idx, tag, at, glue: g, done } => {
            let s = shape_of(page, key);
            let p = geo.page(at);
            let vertex = app.model.points.get(idx as usize).is_some_and(|q| q.kind == SP_VERTEX);
            let _ = g;
            if vertex {
                edit_doc::edit(o, t("Scheitelpunkt verschieben"), done, |tx, eng| {
                    ops::set_vertex(tx, eng, s, tag >> 16, tag & 0xFFFF, p)?;
                    follow(tx, eng, owner, &[s])
                })
            } else {
                let row = app.model.controls.get(tag as usize).copied();
                match row {
                    Some(row) => edit_doc::edit(o, t("Steuerpunkt verschieben"), done, |tx, eng| {
                        ops::set_control(tx, eng, s, row, p)?;
                        // (JD10) a callout's tip dropped on a shape ties it there
                        crate::structures::callout_point(tx, eng, owner, s, row, p, done)?;
                        follow(tx, eng, owner, &[s])
                    }),
                    None => Ok(()),
                }
            }
        }
        SeEv::PointInsert { key, at } => {
            let s = shape_of(page, key);
            let p = geo.page(at);
            // after the vertex that starts the nearest stretch of the outline
            match nearest_stretch(&o.doc, s, p) {
                Some((sec, after)) => edit_doc::edit(o, t("Scheitelpunkt hinzufügen"), true, |tx, eng| ops::insert_vertex(tx, eng, s, sec, after, p).map(|_| ())),
                None => Ok(()),
            }
        }
        SeEv::PointDelete { key, tag } => {
            let s = shape_of(page, key);
            edit_doc::edit(o, t("Scheitelpunkt löschen"), true, |tx, eng| ops::delete_vertex(tx, eng, s, tag >> 16, tag & 0xFFFF))
        }
        SeEv::Cancel => {
            edit_doc::cancel(o);
            Ok(())
        }
        SeEv::Connect { tool, a, ga, b, gb } => {
            let (pa, pb) = (geo.page(a), geo.page(b));
            let (ta, tb) = (target_of(page, ga), target_of(page, gb));
            let label = if tool == SE_CONNECT { t("Verbinder") } else { t("Linie") };
            match edit_doc::edit(o, label, true, |tx, eng| if tool == SE_CONNECT { glue::new_connector(tx, eng, owner, (pa, ta), (pb, tb)) } else { glue::new_line(tx, eng, owner, (pa, ta), (pb, tb)) }) {
                Ok(s) => {
                    select = Some(alloc::vec![key_of(s)]);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        SeEv::Draw { tool, a, b } => {
            let (mut pa, mut pb) = (geo.page(a), geo.page(b));
            let kind = match tool {
                SE_ELLIPSE => ops::Basic::Ellipse,
                SE_TEXT => ops::Basic::TextBox,
                _ => ops::Basic::Rectangle,
            };
            // a click with the text tool: a box of the default size around the point
            if (pa[0] - pb[0]).abs() < 1e-9 && (pa[1] - pb[1]).abs() < 1e-9 {
                let (w, h) = if tool == SE_TEXT { (1.0 / geo.scale, 0.25 / geo.scale) } else { (0.0, 0.0) };
                pa = [pa[0] - w * 0.5, pa[1] - h * 0.5];
                pb = [pb[0] + w * 0.5, pb[1] + h * 0.5];
            }
            let label = match tool {
                SE_ELLIPSE => t("Ellipse"),
                SE_TEXT => t("Textfeld"),
                _ => t("Rechteck"),
            };
            if (pa[0] - pb[0]).abs() < 1e-9 || (pa[1] - pb[1]).abs() < 1e-9 {
                Ok(())
            } else {
                match edit_doc::edit(o, label, true, |tx, eng| {
                    let s = ops::new_shape(tx, eng, owner, kind, pa, pb)?;
                    // (JD10) drawn inside a container: a member
                    crate::structures::joined(tx, eng, owner, &[s])?;
                    Ok(s)
                }) {
                    Ok(s) => {
                        select = Some(alloc::vec![key_of(s)]);
                        if tool == SE_TEXT {
                            app.text_req = Some((key_of(s), None, true));
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
        }
        SeEv::Text { key, ch } => {
            app.text_req = Some((key, ch, false));
            Ok(())
        }
        SeEv::Context { .. } => Ok(()),
        SeEv::Delete => {
            let shapes = refs(&app.se.sel.iter().copied().collect::<Vec<_>>());
            let r = delete(o, owner, &shapes);
            if r.is_ok() {
                select = Some(Vec::new());
            }
            r
        }
        _ => Ok(()),
    };
    if let Err(e) = r {
        // a refused edit (a locked or guarded cell): the drawing stays as it was
        let _ = e;
    }
    if let Some(keys) = select {
        app.se.select(&keys);
    }
    ui.repass();
}

// a frame of the edit layer is the shape's own (one shape resized by its handles)
fn app_frame_matches(doc: &Document, geo: &Geo, s: ShapeRef, b: &SeBox) -> bool {
    let f = se_box(geo, &ops::frame(doc, s));
    let close = |a: [f32; 2], c: [f32; 2]| (a[0] - c[0]).abs() < 0.5 && (a[1] - c[1]).abs() < 0.5;
    close(f.o, b.o) && close(f.u, b.u) && close(f.v, b.v)
}

/// connectors glued to these shapes (and the shapes themselves when they are connectors)
/// follow: routed again; so are the dynamic connectors whose way now runs through one of them
/// (an obstacle moved into it, Visio's)
pub(crate) fn follow(tx: &mut diagram_core::Transaction, eng: &mut diagram_core::formula::Engine, owner: Owner, shapes: &[ShapeRef]) -> Result<(), diagram_core::EditError> {
    let doc = tx.doc();
    let mut deps = glue::dependents(doc, owner, shapes);
    for &s in shapes {
        if glue::is_one_d(doc, s) && !deps.contains(&s) {
            deps.push(s);
        }
    }
    let boxes: Vec<[f64; 4]> = shapes.iter().filter(|&&s| !glue::is_one_d(doc, s)).map(|&s| ops::frame(doc, s).bounds()).collect();
    if !boxes.is_empty() {
        for c in doc.top_shapes(owner) {
            if deps.contains(&c) || shapes.contains(&c) || !glue::is_connector(doc, c) {
                continue;
            }
            let way = glue::path(doc, c);
            if way.windows(2).any(|w| boxes.iter().any(|b| diagram_core::route::hits_box(w[0], w[1], b))) {
                deps.push(c);
            }
        }
    }
    if deps.is_empty() {
        return Ok(());
    }
    glue::update(tx, eng, &deps)
}

fn target_of(page: PageRef, g: SeGlue) -> glue::Target {
    match g {
        SeGlue::None => glue::Target::None,
        SeGlue::Point { key, tag } => glue::Target::Point { shape: shape_of(page, key), row: tag },
        SeGlue::Shape(key) => glue::Target::Shape(shape_of(page, key)),
    }
}

/// delete shapes: connections to them go first (their connectors keep their place); (JD10)
/// members and callouts go along, lists close the gap (structures::delete)
pub(crate) fn delete(o: &mut crate::viewer::Opened, owner: Owner, shapes: &[ShapeRef]) -> Result<(), diagram_core::EditError> {
    crate::structures::delete(o, owner, shapes)
}

/// the measures of the single selection for the status bar and Größe und Position: x, y (the
/// pin), width, height (drawing units in the page's unit), angle (degrees), whether the pin is
/// in the middle (1) or elsewhere (0)
pub(crate) fn measures(app: &Diagrammer) -> Option<[f64; 6]> {
    let o = app.opened.as_ref()?;
    let page = app.model.page?;
    if app.se.sel.len() != 1 {
        return None;
    }
    let s = shape_of(page, *app.se.sel.iter().next()?);
    let doc = &o.doc;
    let sh = SheetRef::Shape(s);
    let k = if metric(app) { 25.4 } else { 1.0 };
    let f = ops::frame(doc, s);
    let pin = [num(doc, sh, "PinX").unwrap_or(0.0), num(doc, sh, "PinY").unwrap_or(0.0)];
    // the Angle cell, as Visio shows it (the frame's own turn includes the flips)
    let angle = num(doc, sh, "Angle").unwrap_or(0.0).to_degrees();
    let angle = if angle.abs() < 1e-9 { 0.0 } else { angle };
    // a connector's box may run backwards (Width = EndX - BeginX): the sizes as lengths
    let lp = [num(doc, sh, "LocPinX").unwrap_or(f.w * 0.5), num(doc, sh, "LocPinY").unwrap_or(f.h * 0.5)];
    let centred = (lp[0] - f.w * 0.5).abs() < 1e-9 && (lp[1] - f.h * 0.5).abs() < 1e-9;
    Some([pin[0] * k, pin[1] * k, f.w.abs() * k, f.h.abs() * k, angle, if centred { 1.0 } else { 0.0 }])
}
