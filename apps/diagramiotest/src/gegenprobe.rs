// gegenprobe -- (JD7) a drawing edited through diagram_core's ops and glue the way JFDiagrammer
// edits it: shapes with four connection points each, a dynamic connector glued to a point at one
// end (static glue) and to a whole shape at the other (dynamic glue), one glued to two points, a
// line glued to points (it keeps its form), a group; a shape moved after the gluing (the
// connectors follow); written with diagram_io and read back
//
//   check(h)          group "edit": the glue survives the move, the file and the reading
//                     (Connect entries, PAR(PNT()) and _WALKGLUE formulas, the ends where the
//                     points are); moving a glued shape after reading moves the ends again
//   write(dir)        --write-gegenprobe DIR: JD7-Gegenprobe-Kleben.vsdx for the Visio
//                     counter-check (move the shapes in Visio: do the connectors follow?)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::{glue, CellAddr, DocKind, Document, Owner, RowKey, SecKey, ShapeRef, SheetRef, Text, Transaction};
use diagram_io::vsdx::{self, Measure, WriteOptions};

use crate::H;

// the four points of a box: left, right, bottom, top middle; X, Y as fractions of Width,
// Height; DirX, DirY into the shape
const POINTS: [(&str, &str, f64, f64); 4] =
    [("Width*0", "Height*0.5", 1.0, 0.0), ("Width*1", "Height*0.5", -1.0, 0.0), ("Width*0.5", "Height*0", 0.0, 1.0), ("Width*0.5", "Height*1", 0.0, -1.0)];

fn points(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef) -> Result<(), diagram_core::EditError> {
    let sheet = SheetRef::Shape(s);
    let sk: SecKey = tx.add_section(sheet, "Connection")?;
    for (i, (x, y, dx, dy)) in POINTS.iter().enumerate() {
        let rk = tx.add_row(sheet, sk, Some(RowKey::Index(i as u32)), Some("Connection"))?;
        let (cx, cy, cdx, cdy, ct) = (tx.intern("X"), tx.intern("Y"), tx.intern("DirX"), tx.intern("DirY"), tx.intern("Type"));
        tx.set_formula(sheet, &CellAddr::row(sk, rk, cx), x)?;
        tx.set_formula(sheet, &CellAddr::row(sk, rk, cy), y)?;
        tx.set_constant(sheet, &CellAddr::row(sk, rk, cdx), diagram_core::Value::Num(*dx), None)?;
        tx.set_constant(sheet, &CellAddr::row(sk, rk, cdy), diagram_core::Value::Num(*dy), None)?;
        tx.set_constant(sheet, &CellAddr::row(sk, rk, ct), diagram_core::Value::Num(0.0), None)?;
    }
    eng.recalc(tx, &[sheet])?;
    Ok(())
}

fn labelled(tx: &mut Transaction, eng: &mut Engine, owner: Owner, kind: Basic, a: [f64; 2], b: [f64; 2], text: &str) -> Result<ShapeRef, diagram_core::EditError> {
    let s = ops::new_shape(tx, eng, owner, kind, a, b)?;
    ops::set_text(tx, eng, s, Text::plain(&format!("{text}\n")))?;
    points(tx, eng, s)?;
    Ok(s)
}

// the point of row `row` of shape s on the page
fn point(doc: &Document, s: ShapeRef, row: u32) -> [f64; 2] {
    glue::connection_points(doc, s).into_iter().find(|p| p.row == row).map_or([0.0; 2], |p| p.at)
}

struct Built {
    doc: Document,
    a: ShapeRef,
    b: ShapeRef,
    c: ShapeRef,
    conns: Vec<ShapeRef>,
}

// the drawing (inches: the metric page is A4 landscape, 11.69 x 8.27 in)
fn build() -> Result<Built, String> {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().ok_or("no page")?;
    let owner = Owner::Page(page);
    let mut eng = Engine::new();
    let e = |e: diagram_core::EditError| format!("{e:?}");
    // three boxes with text and points, an ellipse, two boxes grouped
    let mut tx = doc.transaction("Shapes");
    let a = labelled(&mut tx, &mut eng, owner, Basic::Rectangle, [1.0, 5.5], [3.0, 6.5], "Start (Punkte)").map_err(e)?;
    let b = labelled(&mut tx, &mut eng, owner, Basic::Rectangle, [6.0, 5.5], [8.0, 6.5], "Prüfen (Punkte)").map_err(e)?;
    let c = labelled(&mut tx, &mut eng, owner, Basic::Ellipse, [6.0, 2.0], [8.0, 3.0], "Ende (ganzes Shape)").map_err(e)?;
    let g1 = ops::new_shape(&mut tx, &mut eng, owner, Basic::Rectangle, [1.0, 1.5], [2.0, 2.2]).map_err(e)?;
    let g2 = ops::new_shape(&mut tx, &mut eng, owner, Basic::Rectangle, [2.2, 1.5], [3.2, 2.2]).map_err(e)?;
    let g = ops::group(&mut tx, &mut eng, owner, &[g1, g2]).map_err(e)?;
    ops::set_text(&mut tx, &mut eng, g, Text::plain("Gruppe\n")).map_err(e)?;
    let _ = tx.commit();
    // connectors: point to point (static, both ends), point to whole shape (static + dynamic),
    // a line between points (CN-12)
    let mut conns = Vec::new();
    let mut tx = doc.transaction("Verbinder");
    let (pa, pb) = (point(tx.doc(), a, 1), point(tx.doc(), b, 0));
    conns.push(glue::new_connector(&mut tx, &mut eng, owner, (pa, glue::Target::Point { shape: a, row: 1 }), (pb, glue::Target::Point { shape: b, row: 0 })).map_err(e)?);
    let pb2 = point(tx.doc(), b, 2);
    conns.push(glue::new_connector(&mut tx, &mut eng, owner, (pb2, glue::Target::Point { shape: b, row: 2 }), ([7.0, 2.5], glue::Target::Shape(c))).map_err(e)?);
    let pa2 = point(tx.doc(), a, 2);
    conns.push(glue::new_connector(&mut tx, &mut eng, owner, (pa2, glue::Target::Point { shape: a, row: 2 }), ([7.0, 2.5], glue::Target::Shape(c))).map_err(e)?);
    let (la, lc) = (point(tx.doc(), a, 3), point(tx.doc(), b, 3));
    conns.push(glue::new_line(&mut tx, &mut eng, owner, (la, glue::Target::Point { shape: a, row: 3 }), (lc, glue::Target::Point { shape: b, row: 3 })).map_err(e)?);
    let _ = tx.commit();
    // moved after the gluing: the connectors follow
    let mut tx = doc.transaction("Verschieben");
    ops::move_shapes(&mut tx, &mut eng, &[b], [0.5, -0.75]).map_err(e)?;
    let deps = glue::dependents(tx.doc(), owner, &[b]);
    glue::update(&mut tx, &mut eng, &deps).map_err(e)?;
    let _ = tx.commit();
    Ok(Built { doc, a, b, c, conns })
}

fn near(p: [f64; 2], q: [f64; 2]) -> bool {
    (p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6
}

fn formula(doc: &Document, s: ShapeRef, cell: &str) -> String {
    let Some(c) = doc.sym(cell) else { return String::new() };
    match doc.resolve(SheetRef::Shape(s), &CellAddr::single(c)).formula() {
        diagram_core::Formula::Expr(f) => String::from(doc.str(f)),
        _ => String::new(),
    }
}

/// group "edit"
pub(crate) fn check(h: &mut H) {
    let built = match build() {
        Ok(b) => b,
        Err(e) => {
            h.check("edit: build the drawing", false, e);
            return;
        }
    };
    let Built { doc, a, b, c, conns } = built;
    // the static ends sit on their points after the move
    let ends = |doc: &Document, s: ShapeRef| glue::ends(doc, s).unwrap_or(([0.0; 2], [0.0; 2]));
    let (b0, e0) = ends(&doc, conns[0]);
    h.check("edit: static begin on A's right point", near(b0, point(&doc, a, 1)), format!("{b0:?}"));
    h.check("edit: static end on B's left point after B moved", near(e0, point(&doc, b, 0)), format!("{e0:?}"));
    let g = glue::glued(&doc, conns[1]);
    h.check("edit: point to whole shape", matches!(g, [glue::Target::Point { .. }, glue::Target::Shape(x)] if x == c), format!("{g:?}"));
    h.check("edit: PAR(PNT()) formula", formula(&doc, conns[0], "BeginX").starts_with("PAR(PNT("), formula(&doc, conns[0], "BeginX"));
    // the line keeps its form: its Width is the distance of its ends, its Angle their direction
    let num = |s: ShapeRef, cell: &str| doc.sym(cell).and_then(|cs| doc.resolve(SheetRef::Shape(s), &CellAddr::single(cs)).num()).unwrap_or(f64::NAN);
    let (lb, le) = ends(&doc, conns[3]);
    let (dx, dy) = (le[0] - lb[0], le[1] - lb[1]);
    let len = rt::x87::pow(dx * dx + dy * dy, 0.5);
    let ang = rt::x87::atan2(dy, dx);
    h.check("edit: the glued line's width and angle", (num(conns[3], "Width") - len).abs() < 1e-9 && (num(conns[3], "Angle") - ang).abs() < 1e-9, format!("{} {} / {len} {ang}", num(conns[3], "Width"), num(conns[3], "Angle")));
    h.check("edit: _WALKGLUE formula", formula(&doc, conns[1], "EndX").starts_with("_WALKGLUE("), formula(&doc, conns[1], "EndX"));
    // written and read back: the same glue
    let bytes = match vsdx::write(&doc, &WriteOptions::DEFAULT) {
        Ok(b) => b,
        Err(e) => {
            h.check("edit: write", false, format!("{e}"));
            return;
        }
    };
    let back = match vsdx::read(&bytes, &vsdx::Limits::VSDX) {
        Ok(l) => l.doc,
        Err(e) => {
            h.check("edit: read back", false, format!("{e}"));
            return;
        }
    };
    let page = |d: &Document| d.pages().next();
    let n = |d: &Document| page(d).and_then(|p| d.page(p)).map_or(0, |p| p.connects.len());
    h.check("edit: Connect entries kept", n(&doc) == n(&back) && n(&doc) >= 7, format!("{} / {}", n(&doc), n(&back)));
    let id = |d: &Document, s: ShapeRef| d.shape(s).map_or(0, |x| x.id());
    let find = |d: &Document, idv: u32| page(d).and_then(|p| d.shape_by_id(Owner::Page(p), idv));
    let (Some(ra), Some(rb), Some(rc0)) = (find(&back, id(&doc, a)), find(&back, id(&doc, b)), find(&back, id(&doc, conns[0]))) else {
        h.check("edit: shapes read back", false, String::new());
        return;
    };
    h.check("edit: glue read back", glue::glued(&back, rc0) == [glue::Target::Point { shape: ra, row: 1 }, glue::Target::Point { shape: rb, row: 0 }], String::new());
    // moving B after reading moves the end again
    let mut back = back;
    let Some(p) = page(&back) else { return };
    let owner = Owner::Page(p);
    let mut eng = Engine::new();
    let mut tx = back.transaction("Verschieben");
    let r = ops::move_shapes(&mut tx, &mut eng, &[rb], [0.0, 1.0]).and_then(|_| {
        let deps = glue::dependents(tx.doc(), owner, &[rb]);
        glue::update(&mut tx, &mut eng, &deps)
    });
    let _ = tx.commit();
    let (_, e1) = ends(&back, rc0);
    h.check("edit: the end follows after reading", r.is_ok() && near(e1, point(&back, rb, 0)), format!("{e1:?} {r:?}"));
}

/// --write-gegenprobe DIR
pub(crate) fn write(dir: &str) -> i32 {
    let built = match build() {
        Ok(b) => b,
        Err(e) => {
            crate::out(&format!("build: {e}\n"));
            return 1;
        }
    };
    let bytes = match vsdx::write(&built.doc, &WriteOptions::DEFAULT) {
        Ok(b) => b,
        Err(e) => {
            crate::out(&format!("write: {e}\n"));
            return 1;
        }
    };
    let path = format!("{dir}/JD7-Gegenprobe-Kleben.vsdx");
    if !plat::write_file(&path, &bytes) {
        crate::out(&format!("cannot write {path}\n"));
        return 1;
    }
    crate::out(&format!("wrote {path}\n"));
    0
}
