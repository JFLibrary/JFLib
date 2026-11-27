// pointglue -- (JD21 glue) glue beyond the ends of 1-D shapes written and read back: a control
// point glued to a connection point (SP-04), connection points made by the connection point tool
// and their kinds (CN-01), the connection points made at a vertex and at a handle and the ends
// glued there, an end on a guide (CN-02)
//
//   check(h)          group "pointglue": the drawing built as JFDiagrammer builds it
//                     (diagram_core::pointglue), written as .vsdx and read back: the same Connect
//                     entries (FromCell, FromPart, ToCell, ToPart), the control point's
//                     LOC(PNT(..)) formulas, the Connection rows (formulas and Type), the guide's
//                     formula; after reading, the target moved: the control point follows, the
//                     guide moved: the end follows
//
// Everything is built here; nothing comes from Visio.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::glue::{self, End, Target};
use diagram_core::ops::{self, Basic};
use diagram_core::pointglue::{self, Anchor};
use diagram_core::{CellAddr, DocKind, Document, EditError, Formula, Owner, RowKey, SecKey, ShapeKind, ShapeRef, SheetRef, Transaction, Unit, Value};
use diagram_io::vsdx::{self, Measure, WriteOptions};

use crate::H;

struct Built {
    doc: Document,
    bus: ShapeRef,
    target: ShapeRef,
    guide: ShapeRef,
    on_guide: ShapeRef,
}

// a control row Row_1 of a shape that glues (X = Width*0.25, Y = Height+0.5 in)
fn add_control(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef) -> Result<RowKey, EditError> {
    let sheet = SheetRef::Shape(s);
    let sec = SecKey::one(tx.intern("Control"));
    let rk = RowKey::Named(tx.intern("Row_1"));
    tx.add_row(sheet, sec, Some(rk), None)?;
    let (x, y) = (tx.intern("X"), tx.intern("Y"));
    tx.set_formula(sheet, &CellAddr::row(sec, rk, x), "Width*0.25")?;
    tx.set_formula(sheet, &CellAddr::row(sec, rk, y), "Height+0.5")?;
    for (n, v) in [("XCon", 0.0), ("YCon", 0.0), ("CanGlue", 1.0)] {
        let c = tx.intern(n);
        tx.set_constant(sheet, &CellAddr::row(sec, rk, c), Value::Num(v), None)?;
    }
    eng.recalc(tx, &[sheet])?;
    Ok(rk)
}

fn build() -> Result<Built, String> {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().ok_or("no page")?;
    let owner = Owner::Page(page);
    let mut eng = Engine::new();
    let e = |e: EditError| format!("{e:?}");
    let mut tx = doc.transaction("Kleben");
    let bus = ops::new_shape(&mut tx, &mut eng, owner, Basic::Rectangle, [1.0, 1.0], [7.0, 1.2]).map_err(e)?;
    let target = ops::new_shape(&mut tx, &mut eng, owner, Basic::Rectangle, [2.0, 3.0], [3.0, 3.8]).map_err(e)?;
    let corner = ops::new_shape(&mut tx, &mut eng, owner, Basic::Rectangle, [5.0, 3.0], [6.0, 3.8]).map_err(e)?;
    // the connection point tool: three points, inward, outward, both
    let p0 = pointglue::add_point(&mut tx, &mut eng, target, [2.5, 3.0]).map_err(e)?;
    let p1 = pointglue::add_point(&mut tx, &mut eng, target, [2.0, 3.4]).map_err(e)?;
    let p2 = pointglue::add_point(&mut tx, &mut eng, target, [3.0, 3.4]).map_err(e)?;
    pointglue::set_point_kind(&mut tx, &mut eng, target, p1, 1).map_err(e)?;
    pointglue::set_point_kind(&mut tx, &mut eng, target, p2, 2).map_err(e)?;
    // the bus's control point onto the target's bottom point
    let row = add_control(&mut tx, &mut eng, bus).map_err(e)?;
    pointglue::glue_control(&mut tx, &mut eng, bus, row, [2.5, 3.0], Target::Point { shape: target, row: p0 }).map_err(e)?;
    // a line to the corner (a vertex) and one to the middle of the left side (a handle)
    let v = pointglue::anchor_point(&mut tx, &mut eng, corner, Anchor::Vertex { section: 0, row: 3 }).map_err(e)?;
    let at = pointglue::point_at(tx.doc(), corner, v).ok_or("no vertex point")?;
    glue::new_line(&mut tx, &mut eng, owner, ([7.0, 5.0], Target::None), (at, Target::Point { shape: corner, row: v })).map_err(e)?;
    let hnd = pointglue::anchor_point(&mut tx, &mut eng, corner, Anchor::Handle { fx: 0.0, fy: 0.5 }).map_err(e)?;
    let at = pointglue::point_at(tx.doc(), corner, hnd).ok_or("no handle point")?;
    glue::new_connector(&mut tx, &mut eng, owner, ([4.0, 5.0], Target::None), (at, Target::Point { shape: corner, row: hnd })).map_err(e)?;
    // a guide across the page at 6 in, a line's end on it
    let guide = tx.add_shape(owner, None, ShapeKind::Guide).map_err(e)?;
    let gs = SheetRef::Shape(guide);
    for (n, v, u) in [("PinX", 0.0, Unit::Inches), ("PinY", 6.0, Unit::Inches), ("Width", 0.0, Unit::Inches), ("Height", 0.0, Unit::Inches), ("LocPinX", 0.0, Unit::Inches), ("LocPinY", 0.0, Unit::Inches), ("Angle", 0.0, Unit::Radians)] {
        let a = tx.single(n);
        tx.set_length(gs, &a, v, u).map_err(e)?;
    }
    let sk = tx.add_section(gs, "Geometry").map_err(e)?;
    let rk = tx.add_row(gs, sk, Some(RowKey::Index(1)), Some("InfiniteLine")).map_err(e)?;
    for (n, v) in [("X", 0.0), ("Y", 0.0), ("A", 1.0), ("B", 0.0)] {
        let c = tx.intern(n);
        tx.set_length(gs, &CellAddr::row(sk, rk, c), v, Unit::Inches).map_err(e)?;
    }
    let on_guide = glue::new_line(&mut tx, &mut eng, owner, ([1.0, 7.0], Target::None), ([2.0, 6.1], Target::None)).map_err(e)?;
    pointglue::glue_guide(&mut tx, &mut eng, on_guide, End::End, [2.0, 6.1], guide).map_err(e)?;
    let _ = tx.commit();
    Ok(Built { doc, bus, target, guide, on_guide })
}

// the page's Connect entries as text, sorted
fn connects(doc: &Document) -> Vec<String> {
    let Some(p) = doc.pages().next().and_then(|p| doc.page(p)) else { return Vec::new() };
    let s = |x: Option<diagram_core::Sym>| String::from(x.map_or("", |x| doc.str(x)));
    let mut v: Vec<String> = p.connects.iter().map(|c| format!("{} {} {} -> {} {} {}", c.from_sheet, s(c.from_cell), c.from_part.unwrap_or(-1), c.to_sheet, s(c.to_cell), c.to_part.unwrap_or(-1))).collect();
    v.sort();
    v
}

// a shape's Connection and Control rows as text: formulas (else values) and Type
fn rows(doc: &Document, s: ShapeRef) -> String {
    let sheet = SheetRef::Shape(s);
    let mut out = String::new();
    for name in ["Connection", "Control"] {
        let Some(sec) = doc.sym(name) else { continue };
        let sk = SecKey::one(sec);
        for r in doc.rows(sheet, sk) {
            let key = match r.key {
                RowKey::Named(n) => String::from(doc.str(n)),
                RowKey::Index(i) => format!("{i}"),
            };
            out.push_str(&format!("{name} {key}:"));
            for c in ["X", "Y", "Type", "CanGlue"] {
                let Some(cs) = doc.sym(c) else { continue };
                let rc = doc.resolve(sheet, &CellAddr::row(sk, r.key, cs));
                if !rc.exists() {
                    continue;
                }
                match rc.formula() {
                    Formula::Expr(f) => out.push_str(&format!(" {c}={}", doc.str(f))),
                    _ => out.push_str(&format!(" {c}={:.6}", rc.num().unwrap_or(f64::NAN))),
                }
            }
            out.push('\n');
        }
    }
    out
}

fn formula(doc: &Document, s: ShapeRef, cell: &str) -> String {
    let Some(c) = doc.sym(cell) else { return String::new() };
    match doc.resolve(SheetRef::Shape(s), &CellAddr::single(c)).formula() {
        Formula::Expr(f) => String::from(doc.str(f)),
        _ => String::new(),
    }
}

fn same_shape(doc: &Document, back: &Document, s: ShapeRef) -> Option<ShapeRef> {
    let id = doc.shape(s)?.id();
    let p = back.pages().next()?;
    back.shape_by_id(Owner::Page(p), id)
}

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

pub(crate) fn check(h: &mut H) {
    let Built { doc, bus, target, guide, on_guide } = match build() {
        Ok(b) => b,
        Err(e) => {
            h.check("pointglue: build the drawing", false, e);
            return;
        }
    };
    let before = connects(&doc);
    // (JD21) an end on a guide has two entries (X and Y to the guide's Angle, ToPart 4), as Visio 16 writes them
    h.check("pointglue: Connect entries of a control point, a vertex, a handle, a guide", before.len() == 5 && before.iter().any(|c| c.contains("Controls.Row_1 100")) && before.iter().filter(|c| c.contains("Angle 4")).count() == 2, format!("{before:?}"));
    h.check("pointglue: the control point's formula", formula_row(&doc, bus).starts_with("LOC(PNT(Sheet."), formula_row(&doc, bus));
    let bytes = match vsdx::write(&doc, &WriteOptions::DEFAULT) {
        Ok(b) => b,
        Err(e) => {
            h.check("pointglue: write", false, format!("{e}"));
            return;
        }
    };
    let mut back = match vsdx::read(&bytes, &vsdx::Limits::VSDX) {
        Ok(l) => l.doc,
        Err(e) => {
            h.check("pointglue: read back", false, format!("{e}"));
            return;
        }
    };
    let after = connects(&back);
    h.check("pointglue: written and read back, the same Connect entries", after == before, format!("{after:?}"));
    let mut same = 0;
    let mut first = String::new();
    let all: Vec<ShapeRef> = doc.all_shapes(Owner::Page(doc.pages().next().expect("page"))).collect();
    for &s in &all {
        let Some(b) = same_shape(&doc, &back, s) else { continue };
        let (r0, r1) = (rows(&doc, s), rows(&back, b));
        let (g0, g1) = (formula(&doc, s, "EndY"), formula(&back, b, "EndY"));
        if r0 == r1 && g0 == g1 {
            same += 1;
        } else if first.is_empty() {
            first = format!("{r0}{g0}\n/\n{r1}{g1}");
        }
    }
    h.check("pointglue: Connection and Control rows, the guide's formula unchanged", same == all.len(), format!("{same} of {} {first}", all.len()));
    let kinds: Vec<u8> = (0..3).filter_map(|r| same_shape(&doc, &back, target).and_then(|t| pointglue::point_kind(&back, t, r))).collect();
    h.check("pointglue: the kinds of the new points (inward, outward, both)", kinds == [0, 1, 2], format!("{kinds:?}"));
    // after reading: the target moves, the control point follows; the guide moves, the end too
    let (Some(bb), Some(tb), Some(gb), Some(lb)) = (same_shape(&doc, &back, bus), same_shape(&doc, &back, target), same_shape(&doc, &back, guide), same_shape(&doc, &back, on_guide)) else {
        h.check("pointglue: the shapes after reading", false, String::from("missing"));
        return;
    };
    let owner = Owner::Page(back.pages().next().expect("page"));
    let mut eng = Engine::new();
    let mut tx = back.transaction("Verschieben");
    let moved = ops::move_shapes(&mut tx, &mut eng, &[tb, gb], [1.0, 0.5]).and_then(|_| {
        let deps = glue::dependents(tx.doc(), owner, &[tb, gb]);
        glue::update(&mut tx, &mut eng, &deps)
    });
    let _ = tx.commit();
    let want = pointglue::point_at(&back, tb, 0).unwrap_or([0.0, 0.0]);
    let ctl = ops::controls(&back, bb).first().map_or([f64::NAN, f64::NAN], |c| c.at);
    h.check("pointglue: after reading the control point follows its target", moved.is_ok() && close(ctl, want), format!("{ctl:?} / {want:?}"));
    let end = glue::ends(&back, lb).map_or([f64::NAN, f64::NAN], |e| e.1);
    h.check("pointglue: after reading the end follows its guide", close(end, [2.0, 6.5]), format!("{end:?}"));
}

// the control point's X formula
fn formula_row(doc: &Document, s: ShapeRef) -> String {
    let (Some(sec), Some(x)) = (doc.sym("Control"), doc.sym("X")) else { return String::new() };
    let rk = doc.rows(SheetRef::Shape(s), SecKey::one(sec)).first().map(|r| r.key);
    match rk.map(|k| doc.resolve(SheetRef::Shape(s), &CellAddr::row(SecKey::one(sec), k, x)).formula()) {
        Some(Formula::Expr(f)) => String::from(doc.str(f)),
        _ => String::new(),
    }
}
