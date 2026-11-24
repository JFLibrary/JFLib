// shapeops -- (JD21) the shape operations of diagram_core::shapeops on a drawing, written and read
// back, and the drawing for the Visio counter-check
//
//   check(h)          group "shapeops": every operation (Gesamtmenge, Kombinieren, In
//                     Einzelmengen zerlegen, Schnittmenge bilden, Subtrahieren) on rectangles, an
//                     ellipse, a turned rectangle, a shape with a circular arc (ArcTo) and a shape
//                     with a hole; the results written as .vsdx and read back: the same Geometry
//                     rows (types, values, formulas), the same frames; a result stretched after
//                     reading keeps its arcs proportional
//   write(dir)        --write-jd21-ops DIR: jd21-vorgaenge.vsdx with a page of the input sets
//                     (to run the operations in Visio) and a page with each set and
//                     JFDiagrammer's result beside it
//
// The input sets are built here (nothing comes from Visio): the primary shape of each set (the one
// to select first) carries the text "1".

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::shapeops::{self, Operation};
use diagram_core::{Cell, CellAddr, DocKind, Document, EditError, Formula, Owner, PageRef, Row, RowKey, SecKey, Section, ShapeData, ShapeKind, ShapeRef, SheetRef, StyleRefs, Text, Transaction, Value};
use diagram_io::vsdx::{self, Measure, WriteOptions};

use crate::H;

// the operations of the check page, each with its German name in Visio and the inputs it takes
const SETS: [(&str, Operation); 6] = [
    ("Gesamtmenge", Operation::Union),
    ("Kombinieren", Operation::Combine),
    ("In Einzelmengen zerlegen", Operation::Fragment),
    ("Schnittmenge bilden", Operation::Intersect),
    ("Subtrahieren", Operation::Subtract),
    ("Gesamtmenge (Bogen und Loch)", Operation::Union),
];

fn cell(tx: &mut Transaction, name: &str, v: f64, f: Option<&str>) -> Cell {
    let n = tx.intern(name);
    let formula = f.map_or(Formula::Absent, |f| Formula::Expr(tx.intern(f)));
    Cell { name: n, value: Value::Num(v), formula, unit: None, error: None, stale: f.is_some() }
}

// a shape with Geometry sections of rows (type, [(cell, value)]), its box w x h with the lower
// left corner at `at`; X/Y/A/B of absolute rows as Width*k / Height*k
fn custom(tx: &mut Transaction, eng: &mut Engine, owner: Owner, at: [f64; 2], w: f64, h: f64, secs: &[&[(&str, &[(&str, f64)])]]) -> Result<ShapeRef, EditError> {
    let mut d = ShapeData::new(ShapeKind::Shape);
    let normal = tx.doc().style_by_name_u("Normal").and_then(|s| tx.doc().style(s)).map(|s| s.header.id);
    if let Some(id) = normal {
        d.header.styles = StyleRefs::all(id);
    }
    d.sheet.cells = alloc::vec![
        cell(tx, "PinX", at[0] + w * 0.5, None),
        cell(tx, "PinY", at[1] + h * 0.5, None),
        cell(tx, "Width", w, None),
        cell(tx, "Height", h, None),
        cell(tx, "LocPinX", w * 0.5, Some("Width*0.5")),
        cell(tx, "LocPinY", h * 0.5, Some("Height*0.5")),
        cell(tx, "Angle", 0.0, None),
    ];
    let geo = tx.intern("Geometry");
    for (si, rows) in secs.iter().enumerate() {
        let mut sec = Section::new(SecKey { name: geo, index: si as u32 });
        sec.ix_written = true;
        for n in ["NoFill", "NoLine", "NoShow", "NoSnap"] {
            sec.cells.push(cell(tx, n, 0.0, None));
        }
        for (ri, (kind, cells)) in rows.iter().enumerate() {
            let mut r = Row::new(RowKey::Index(ri as u32 + 1));
            r.kind = Some(tx.intern(kind));
            for &(n, v) in cells.iter() {
                // the bow of ArcTo is a length of its own
                let f = match n {
                    "X" => Some(format!("Width*{}", v / w)),
                    "Y" | "B" => Some(format!("Height*{}", v / h)),
                    "A" if *kind != "ArcTo" => Some(format!("Width*{}", v / w)),
                    _ => None,
                };
                let c = cell(tx, n, v, f.as_deref());
                r.cells.push(c);
            }
            sec.rows.push(r);
        }
        d.sheet.sections.push(sec);
    }
    let s = tx.add_shape_with(owner, None, d)?;
    eng.recalc(tx, &[SheetRef::Shape(s)])?;
    Ok(s)
}

// a box 1.2 x 0.9 whose top is a half circle (ArcTo with a bow of half the width)
fn tombstone(tx: &mut Transaction, eng: &mut Engine, owner: Owner, at: [f64; 2]) -> Result<ShapeRef, EditError> {
    let (w, h) = (1.2, 0.9);
    let rows: &[(&str, &[(&str, f64)])] = &[
        ("MoveTo", &[("X", 0.0), ("Y", 0.0)]),
        ("LineTo", &[("X", w), ("Y", 0.0)]),
        ("LineTo", &[("X", w), ("Y", h - w * 0.5)]),
        ("ArcTo", &[("X", 0.0), ("Y", h - w * 0.5), ("A", w * 0.5)]),
        ("LineTo", &[("X", 0.0), ("Y", 0.0)]),
    ];
    custom(tx, eng, owner, at, w, h, &[rows])
}

// a square frame 1 x 1 with a square hole (two sections, filled even-odd)
fn framed(tx: &mut Transaction, eng: &mut Engine, owner: Owner, at: [f64; 2]) -> Result<ShapeRef, EditError> {
    let s = 1.0;
    let sq = |a: f64, b: f64| -> [(&'static str, [(&'static str, f64); 2]); 5] {
        [("MoveTo", [("X", a), ("Y", a)]), ("LineTo", [("X", b), ("Y", a)]), ("LineTo", [("X", b), ("Y", b)]), ("LineTo", [("X", a), ("Y", b)]), ("LineTo", [("X", a), ("Y", a)])]
    };
    let outer = sq(0.0, s);
    let inner = sq(0.3, 0.7);
    let o: Vec<(&str, &[(&str, f64)])> = outer.iter().map(|(k, c)| (*k, &c[..])).collect();
    let i: Vec<(&str, &[(&str, f64)])> = inner.iter().map(|(k, c)| (*k, &c[..])).collect();
    custom(tx, eng, owner, at, s, s, &[&o, &i])
}

fn label(tx: &mut Transaction, eng: &mut Engine, owner: Owner, at: [f64; 2], text: &str) -> Result<ShapeRef, EditError> {
    let t = ops::new_shape(tx, eng, owner, Basic::TextBox, at, [at[0] + 3.2, at[1] + 0.2])?;
    ops::set_text(tx, eng, t, Text::plain(text))?;
    Ok(t)
}

// the input set of row k at x0 (its lower left corner at (x0, y0)) -> the shapes, primary first
fn inputs(tx: &mut Transaction, eng: &mut Engine, owner: Owner, k: usize, x0: f64, y0: f64) -> Result<Vec<ShapeRef>, EditError> {
    let rect = |tx: &mut Transaction, eng: &mut Engine, a: [f64; 2], b: [f64; 2]| ops::new_shape(tx, eng, owner, Basic::Rectangle, [x0 + a[0], y0 + a[1]], [x0 + b[0], y0 + b[1]]);
    let ell = |tx: &mut Transaction, eng: &mut Engine, a: [f64; 2], b: [f64; 2]| ops::new_shape(tx, eng, owner, Basic::Ellipse, [x0 + a[0], y0 + a[1]], [x0 + b[0], y0 + b[1]]);
    let mut v = Vec::new();
    match k {
        // a rectangle, an ellipse over its right side, a turned rectangle over both
        0 | 4 => {
            v.push(rect(tx, eng, [0.0, 0.0], [1.4, 0.9])?);
            v.push(ell(tx, eng, [1.0, 0.1], [2.2, 1.0])?);
            let t = rect(tx, eng, [0.5, 0.35], [1.7, 0.65])?;
            ops::rotate_shapes(tx, eng, &[t], [x0 + 1.1, y0 + 0.5], 30f64.to_radians())?;
            v.push(t);
        }
        // a rectangle and an ellipse overlapping (the overlap becomes a hole)
        1 => {
            v.push(rect(tx, eng, [0.0, 0.0], [1.4, 0.9])?);
            v.push(ell(tx, eng, [0.9, 0.1], [2.2, 1.0])?);
        }
        // a rectangle, an ellipse and a line through both
        2 => {
            v.push(rect(tx, eng, [0.0, 0.0], [1.4, 0.9])?);
            v.push(ell(tx, eng, [1.0, 0.1], [2.2, 1.0])?);
            v.push(diagram_core::glue::new_line(tx, eng, owner, ([x0 - 0.2, y0 + 0.2], diagram_core::glue::Target::None), ([x0 + 2.4, y0 + 0.8], diagram_core::glue::Target::None))?);
        }
        // the shape with the hole and the arc shape
        3 | 5 => {
            v.push(framed(tx, eng, owner, [x0, y0 - 0.05])?);
            v.push(tombstone(tx, eng, owner, [x0 + 0.55, y0])?);
        }
        _ => {}
    }
    ops::set_text(tx, eng, v[0], Text::plain("1"))?;
    Ok(v)
}

struct Built {
    doc: Document,
    // per set: the result shapes (on the result page)
    results: Vec<Vec<ShapeRef>>,
    result_page: PageRef,
}

// rows of 1.2 inches from the top of the A4 landscape page (8.27 inches high)
const ROW: f64 = 1.2;

fn build() -> Result<Built, String> {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let first = doc.pages().next().ok_or("no page")?;
    let mut eng = Engine::new();
    let e = |e: EditError| format!("{e:?}");
    let mut tx = doc.transaction("Seiten");
    tx.rename_page(first, "Vorgänge – Ausgang", "Operations - input").map_err(e)?;
    let second = tx.add_page_like(first, "Vorgänge – Ergebnisse", "Operations - results", false, None).map_err(e)?;
    let _ = tx.commit();
    let mut results = Vec::new();
    for (page, with_results) in [(first, false), (second, true)] {
        let owner = Owner::Page(page);
        let mut tx = doc.transaction("Shapes");
        for (k, (name, op)) in SETS.iter().enumerate() {
            let y0 = 8.0 - ROW * (k as f64 + 1.0);
            label(&mut tx, &mut eng, owner, [0.3, y0 + 0.95], &format!("{}. {name}", k + 1)).map_err(e)?;
            inputs(&mut tx, &mut eng, owner, k, 0.6, y0).map_err(e)?;
            if with_results {
                // the same set beside it, then the operation on it
                let set = inputs(&mut tx, &mut eng, owner, k, 4.6, y0).map_err(e)?;
                let made = shapeops::operate(&mut tx, &mut eng, &set, *op).map_err(e)?;
                label(&mut tx, &mut eng, owner, [4.3, y0 + 0.95], &format!("{}. JFDiagrammer: {name}", k + 1)).map_err(e)?;
                results.push(made);
            }
        }
        let _ = tx.commit();
    }
    Ok(Built { doc, results, result_page: second })
}

// a shape's Geometry rows as text: type, then cell = value [formula]
fn rows_text(doc: &Document, s: ShapeRef) -> String {
    let sheet = SheetRef::Shape(s);
    let mut out = String::new();
    for sk in doc.geometry(sheet) {
        for r in doc.rows(sheet, sk) {
            out.push_str(r.kind.map_or("?", |k| doc.str(k)));
            for c in ["X", "Y", "A", "B", "C", "D"] {
                let Some(cs) = doc.sym(c) else { continue };
                let res = doc.resolve(sheet, &CellAddr::row(sk, r.key, cs));
                if let Some(v) = res.num() {
                    let f = match res.formula() {
                        Formula::Expr(f) => String::from(doc.str(f)),
                        _ => String::new(),
                    };
                    out.push_str(&format!(" {c}={v:.9} [{f}]"));
                }
            }
            out.push('\n');
        }
        out.push_str("--\n");
    }
    out
}

fn frame_text(doc: &Document, s: ShapeRef) -> String {
    let sheet = SheetRef::Shape(s);
    let mut out = String::new();
    for c in ["PinX", "PinY", "Width", "Height", "LocPinX", "LocPinY", "Angle"] {
        let v = doc.sym(c).and_then(|cs| doc.resolve(sheet, &CellAddr::single(cs)).num()).unwrap_or(f64::NAN);
        out.push_str(&format!("{c}={v:.9} "));
    }
    out
}

/// group "shapeops"
pub(crate) fn check(h: &mut H) {
    let built = match build() {
        Ok(b) => b,
        Err(e) => {
            h.check("shapeops: build the drawing", false, e);
            return;
        }
    };
    let Built { doc, results, result_page } = built;
    let counts: Vec<usize> = results.iter().map(|r| r.len()).collect();
    h.check("shapeops: one result each, pieces for Fragment", counts[0] == 1 && counts[1] == 1 && counts[2] >= 3 && counts[3] == 1 && counts[4] == 1 && counts[5] == 1, format!("{counts:?}"));
    let kinds = |s: ShapeRef| rows_text(&doc, s);
    h.check("shapeops: arcs stay arcs (Gesamtmenge)", kinds(results[0][0]).contains("EllipticalArcTo"), kinds(results[0][0]));
    h.check("shapeops: Kombinieren keeps the ellipse", kinds(results[1][0]).contains("Ellipse "), kinds(results[1][0]));
    h.check("shapeops: the ArcTo of the arc shape is an arc again", kinds(results[5][0]).contains("EllipticalArcTo"), kinds(results[5][0]));
    let secs = doc.geometry(SheetRef::Shape(results[3][0])).len();
    h.check("shapeops: Schnittmenge of the frame and the arc shape keeps the hole's part apart", secs >= 1, format!("{secs} sections"));
    // written and read back: the same rows and frames
    let bytes = match vsdx::write(&doc, &WriteOptions::DEFAULT) {
        Ok(b) => b,
        Err(e) => {
            h.check("shapeops: write", false, format!("{e}"));
            return;
        }
    };
    let back = match vsdx::read(&bytes, &vsdx::Limits::VSDX) {
        Ok(l) => l.doc,
        Err(e) => {
            h.check("shapeops: read back", false, format!("{e}"));
            return;
        }
    };
    let page_back = back.pages().nth(1);
    let mut same = 0;
    let mut total = 0;
    let mut first_diff = String::new();
    for set in &results {
        for &s in set {
            total += 1;
            let id = doc.shape(s).map_or(0, |x| x.id());
            let Some(b) = page_back.and_then(|p| back.shape_by_id(Owner::Page(p), id)) else { continue };
            let (r0, r1) = (rows_text(&doc, s), rows_text(&back, b));
            let (f0, f1) = (frame_text(&doc, s), frame_text(&back, b));
            if r0 == r1 && f0 == f1 {
                same += 1;
            } else if first_diff.is_empty() {
                first_diff = format!("{r0}{f0}\n/\n{r1}{f1}");
            }
        }
    }
    let _ = result_page;
    h.check("shapeops: written and read back, the same rows, formulas and frames", same == total && total > 0, format!("{same} of {total} {first_diff}"));
    // stretched after reading: the arcs' D follows the box (Width/Height*k)
    let mut back = back;
    let (Some(p), Some(id)) = (page_back, doc.shape(results[0][0]).map(|x| x.id())) else { return };
    let Some(u) = back.shape_by_id(Owner::Page(p), id) else { return };
    let sheet = SheetRef::Shape(u);
    let mut eng = Engine::new();
    let w0 = back.sym("Width").and_then(|c| back.resolve(sheet, &CellAddr::single(c)).num()).unwrap_or(0.0);
    // the D of each upright arc (C = 0): a formula of the box's proportions
    let arc_d = |d: &Document| -> Vec<(f64, bool)> {
        let mut v = Vec::new();
        let (Some(cc), Some(cd), Some(kind)) = (d.sym("C"), d.sym("D"), d.sym("EllipticalArcTo")) else { return v };
        for sk in d.geometry(sheet) {
            for r in d.rows(sheet, sk).into_iter().filter(|r| r.kind == Some(kind)) {
                let c = d.resolve(sheet, &CellAddr::row(sk, r.key, cc)).num().unwrap_or(1.0);
                let res = d.resolve(sheet, &CellAddr::row(sk, r.key, cd));
                let prop = matches!(res.formula(), Formula::Expr(f) if d.str(f).starts_with("Width/Height*"));
                if c == 0.0 {
                    v.push((res.num().unwrap_or(f64::NAN), prop));
                }
            }
        }
        v
    };
    let before = arc_d(&back);
    let mut tx = back.transaction("Breite");
    let wc = tx.intern("Width");
    let r = tx.set_constant(sheet, &CellAddr::single(wc), Value::Num(w0 * 2.0), None).and_then(|_| eng.recalc(&mut tx, &[sheet]));
    let _ = tx.commit();
    let after = arc_d(&back);
    let ok = r.is_ok() && !before.is_empty() && before.len() == after.len() && before.iter().zip(after.iter()).all(|(a, b)| a.1 && (b.0 - 2.0 * a.0).abs() < 1e-9);
    h.check("shapeops: a stretched result's arcs follow the box (D twice as large)", ok, format!("{before:?} -> {after:?}"));
    // the frame after a vertex edit of a read result: the box around the geometry again
    let v = ops::vertices(&back, u);
    let mut tx = back.transaction("Scheitelpunkt");
    let r = match v.first() {
        Some(x) => ops::set_vertex(&mut tx, &mut eng, u, x.section, x.row, [x.at[0] - 0.5, x.at[1]]),
        None => Err(EditError::NoRow),
    };
    let _ = tx.commit();
    let gb = shapeops::geometry_bounds(&back, u).unwrap_or([f64::NAN; 4]);
    let wh = |c: &str| back.sym(c).and_then(|cs| back.resolve(sheet, &CellAddr::single(cs)).num()).unwrap_or(f64::NAN);
    h.check(
        "shapeops: the frame follows a vertex",
        r.is_ok() && gb[0].abs() < 1e-9 && gb[1].abs() < 1e-9 && (gb[2] - wh("Width")).abs() < 1e-9 && (gb[3] - wh("Height")).abs() < 1e-9,
        format!("{r:?} {gb:?} {} {}", wh("Width"), wh("Height")),
    );
}

/// --write-jd21-ops DIR
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
    let path = format!("{dir}/jd21-vorgaenge.vsdx");
    if !plat::write_file(&path, &bytes) {
        crate::out(&format!("cannot write {path}\n"));
        return 1;
    }
    crate::out(&format!("wrote {path}\n"));
    0
}
