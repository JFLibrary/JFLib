// tests of ops::arcs and the vertices moved and taken out together (JD21, SP-05)
// (cargo test -p jflib-diagram-core --lib)

use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::cell::{Cell, Formula, Value};
use crate::document::Owner;
use crate::formula::Engine;
use crate::ops::{delete_vertices, move_vertices, vertices};
use crate::sheet::{Row, SecKey, Section};
use crate::{DocKind, Page, PageHeader, ShapeData, ShapeKind};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + b.abs())
}

macro_rules! assert_pt {
    ($a:expr, $b:expr) => {{
        let (a, b): ([f64; 2], [f64; 2]) = ($a, $b);
        assert!(close(a[0], b[0]) && close(a[1], b[1]), "{} = {:?}, not {:?}", stringify!($a), a, b);
    }};
}

fn cnum(doc: &mut Document, name: &str, v: f64) -> Cell {
    Cell::constant(doc.intern(name), Value::Num(v), None)
}

fn cf(doc: &mut Document, name: &str, f: &str, v: f64) -> Cell {
    let (n, f) = (doc.intern(name), doc.intern(f));
    Cell { name: n, value: Value::Num(v), formula: Formula::Expr(f), unit: None, error: None, stale: false }
}

// a shape 4 x 2 with its pin at (5, 5) (its local origin at (3, 4)): MoveTo (0, 0), LineTo
// (4, 0), ArcTo (4, 2) with a bow of 0.5, EllipticalArcTo (0, 2) through (2, 2.5) (C 0, D 1),
// LineTo (0, 0)
fn shape() -> (Document, ShapeRef, Engine) {
    let mut doc = Document::new(DocKind::Drawing);
    let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap();
    let mut d = ShapeData::new(ShapeKind::Shape);
    let cells = vec![
        cnum(&mut doc, "PinX", 5.0),
        cnum(&mut doc, "PinY", 5.0),
        cnum(&mut doc, "Width", 4.0),
        cnum(&mut doc, "Height", 2.0),
        cf(&mut doc, "LocPinX", "Width*0.5", 2.0),
        cf(&mut doc, "LocPinY", "Height*0.5", 1.0),
        cnum(&mut doc, "Angle", 0.0),
    ];
    d.sheet.cells.extend(cells);
    let mut geo = Section::new(SecKey::one(names::GEOMETRY));
    geo.ix_written = true;
    let rows: Vec<(&str, Vec<(&str, f64)>)> = vec![
        ("MoveTo", vec![("X", 0.0), ("Y", 0.0)]),
        ("LineTo", vec![("X", 4.0), ("Y", 0.0)]),
        ("ArcTo", vec![("X", 4.0), ("Y", 2.0), ("A", 0.5)]),
        ("EllipticalArcTo", vec![("X", 0.0), ("Y", 2.0), ("A", 2.0), ("B", 2.5), ("C", 0.0), ("D", 1.0)]),
        ("LineTo", vec![("X", 0.0), ("Y", 0.0)]),
    ];
    for (i, (kind, cells)) in rows.into_iter().enumerate() {
        let mut r = Row::new(RowKey::Index(i as u32 + 1));
        r.kind = Some(doc.intern(kind));
        r.cells = cells.into_iter().map(|(n, v)| cnum(&mut doc, n, v)).collect();
        geo.rows.push(r);
    }
    d.sheet.sections.push(geo);
    let slot = doc.page_mut(page).unwrap().shapes.insert(None, 1, d).unwrap();
    (doc, ShapeRef { owner: Owner::Page(page), slot }, Engine::new())
}

fn edit(doc: &mut Document, eng: &mut Engine, f: impl FnOnce(&mut Transaction, &mut Engine) -> Result<(), EditError>) {
    let mut tx = doc.transaction("Scheitelpunkt");
    f(&mut tx, eng).unwrap();
    tx.commit();
}

fn cell(doc: &Document, s: ShapeRef, row: u32, c: crate::strings::Sym) -> f64 {
    doc.resolve(SheetRef::Shape(s), &CellAddr::row(SecKey::one(names::GEOMETRY), RowKey::Index(row), c)).num().unwrap()
}

#[test]
fn arcs_show_their_control_points_and_eccentricity_handles() {
    let (doc, s, _) = shape();
    let h = arc_handles(&doc, s);
    assert_eq!(h.len(), 2);
    assert_eq!((h[0].section, h[0].row), (0, 3));
    // the ArcTo's middle: the chord (4, 0)-(4, 2) moved 0.5 to its right
    assert_pt!(h[0].at, [7.5, 5.0]);
    assert!(h[0].ecc.is_none());
    // the EllipticalArcTo's control point, its handles a quarter chord (1) along the axis
    assert_pt!(h[1].at, [5.0, 6.5]);
    let e = h[1].ecc.unwrap();
    assert_pt!(e[0], [6.0, 6.5]);
    assert_pt!(e[1], [4.0, 6.5]);
}

#[test]
fn dragging_an_arcs_control_point_bends_it() {
    let (mut doc, s, mut eng) = shape();
    edit(&mut doc, &mut eng, |tx, eng| set_arc_point(tx, eng, s, 0, 3, [8.0, 5.3]));
    // the bow: the projection onto the chord's normal (the height along the chord is dropped)
    let h = arc_handles(&doc, s);
    assert_pt!(h[0].at, [8.0, 5.0]);
    // the box grew to the arc (fit_frame): the vertices stay where they were on the page
    let v = vertices(&doc, s);
    assert_pt!(v[1].at, [7.0, 4.0]);
    assert_pt!(v[2].at, [7.0, 6.0]);
    assert!(cell(&doc, s, 3, names::A) > 0.99 && cell(&doc, s, 3, names::A) < 1.01);
    // the elliptical arc through another point
    edit(&mut doc, &mut eng, |tx, eng| set_arc_point(tx, eng, s, 0, 4, [5.5, 7.0]));
    let h = arc_handles(&doc, s);
    assert_pt!(h[1].at, [5.5, 7.0]);
}

#[test]
fn eccentricity_handles_turn_and_stretch_the_ellipse() {
    let (mut doc, s, mut eng) = shape();
    // the first handle (6, 6.5) up to (5, 8.5): the axis turns upright, the ratio doubles
    edit(&mut doc, &mut eng, |tx, eng| set_eccentricity(tx, eng, s, 0, 4, [5.0, 8.5]));
    let c = cell(&doc, s, 4, names::C);
    let d = cell(&doc, s, 4, names::D);
    assert!(close(c, core::f64::consts::FRAC_PI_2), "C = {c}");
    assert!(close(d, 2.0), "D = {d}");
    // the control point stays on the page, the arc still runs through it
    let h = arc_handles(&doc, s);
    assert_pt!(h[1].at, [5.0, 6.5]);
    // not an elliptical arc
    let mut tx = doc.transaction("x");
    assert_eq!(set_eccentricity(&mut tx, &mut eng, s, 0, 3, [1.0, 1.0]), Err(EditError::NoRow));
    assert_eq!(set_arc_point(&mut tx, &mut eng, s, 0, 2, [1.0, 1.0]), Err(EditError::NoRow));
}

#[test]
fn several_vertices_move_and_go_together() {
    let (mut doc, s, mut eng) = shape();
    // the start (with its closing row) and the corner (4, 0) one to the right
    edit(&mut doc, &mut eng, |tx, eng| move_vertices(tx, eng, s, &[(0, 1), (0, 2)], [1.0, 0.0]));
    let v = vertices(&doc, s);
    assert_eq!(v.len(), 4);
    assert_pt!(v[0].at, [4.0, 4.0]);
    assert_pt!(v[1].at, [8.0, 4.0]);
    assert_pt!(v[2].at, [7.0, 6.0]);
    assert_pt!(v[3].at, [3.0, 6.0]);
    // the closing row went with the start
    let last = glue_last(&doc, s);
    assert_pt!(last, [4.0, 4.0]);
    // two go at once; a sub-path keeps two
    edit(&mut doc, &mut eng, |tx, eng| delete_vertices(tx, eng, s, &[(0, 2), (0, 3), (0, 4)]));
    assert_eq!(vertices(&doc, s).len(), 2);
}

// the page point of the last row of the shape's first section
fn glue_last(doc: &Document, s: ShapeRef) -> [f64; 2] {
    let sheet = SheetRef::Shape(s);
    let sk = SecKey::one(names::GEOMETRY);
    let last = doc.rows(sheet, sk).last().map(|r| r.key).unwrap();
    let n = |c| doc.resolve(sheet, &CellAddr::row(sk, last, c)).num().unwrap();
    doc.page_xform(s).apply([n(names::X), n(names::Y)])
}
