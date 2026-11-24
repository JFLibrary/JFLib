// tests of the shape operations and of frames that follow their geometry (cargo test -p
// jflib-diagram-core --lib)
//
//   Vereinigen, Schneiden, Subtrahieren, Kombinieren, Fragmentieren on rectangles, ellipses, turned
//   and flipped shapes, groups, holes, shapes touching along an edge, an empty result; the areas
//   on the page against closed formulas (the result's rows drawn again by geometry.rs), arcs
//   staying EllipticalArcTo / Ellipse with formulas that stretch with the box, the primary's
//   format and text, the inputs gone; fit_frame after vertex edits (also turned, LockCalcWH),
//   update_alignment_box of a group; every operation undone and redone

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use super::*;
use crate::ops::{self, Basic};
use crate::{Change, DocKind, Page, PageHeader, PageRef, Style};

const PI: f64 = core::f64::consts::PI;

struct Fx {
    doc: Document,
    page: PageRef,
    owner: Owner,
}

fn fx() -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    for (id, name, based) in [(0, "No Style", None), (1, "Normal", Some(0)), (4, "Text Only", Some(1))] {
        let mut st = Style::default();
        st.header.id = id;
        st.header.name_u = Some(doc.intern(name));
        st.header.name = Some(doc.intern(name));
        if let Some(b) = based {
            st.header.based_on = StyleRefs::all(b);
        }
        doc.insert_style(st).unwrap();
    }
    let mut ph = PageHeader { id: 0, ..PageHeader::default() };
    ph.name_u = Some(doc.intern("Page-1"));
    let page = doc.insert_page(Page::new(ph)).unwrap();
    Fx { doc, page, owner: Owner::Page(page) }
}

fn dump(doc: &Document, page: PageRef) -> String {
    let mut s = String::new();
    let owner = Owner::Page(page);
    for r in doc.all_shapes(owner) {
        let sh = doc.shape(r).unwrap();
        let _ = writeln!(s, "shape {} {:?}", sh.id(), sh.data());
    }
    s
}

// run as one committed change; undo gives the page back, redo the result; leave it applied
fn gesture<T>(f: &mut Fx, eng: &mut Engine, label: &str, run: impl FnOnce(&mut Transaction, &mut Engine) -> Result<T, EditError>) -> T {
    let before = dump(&f.doc, f.page);
    let (out, change) = {
        let mut tx = f.doc.transaction(label);
        let out = run(&mut tx, eng).expect(label);
        (out, tx.commit())
    };
    if let Some(mut c) = change {
        let after = dump(&f.doc, f.page);
        assert!(c.undo(&mut f.doc));
        eng.refresh(&f.doc, &c.sheets());
        assert_eq!(dump(&f.doc, f.page), before, "{label}: undo");
        assert!(c.redo(&mut f.doc));
        eng.refresh(&f.doc, &c.sheets());
        assert_eq!(dump(&f.doc, f.page), after, "{label}: redo");
        let _: &Change = &c;
    }
    out
}

fn rect(f: &mut Fx, eng: &mut Engine, a: [f64; 2], b: [f64; 2]) -> ShapeRef {
    let o = f.owner;
    gesture(f, eng, "Rechteck", |tx, eng| ops::new_shape(tx, eng, o, Basic::Rectangle, a, b))
}

fn ellipse(f: &mut Fx, eng: &mut Engine, a: [f64; 2], b: [f64; 2]) -> ShapeRef {
    let o = f.owner;
    gesture(f, eng, "Ellipse", |tx, eng| ops::new_shape(tx, eng, o, Basic::Ellipse, a, b))
}

fn op(f: &mut Fx, eng: &mut Engine, shapes: &[ShapeRef], o: Operation) -> Vec<ShapeRef> {
    gesture(f, eng, "Vorgang", |tx, eng| operate(tx, eng, shapes, o))
}

// the filled area of a shape on the page: its closed sub-paths as geometry.rs draws them (arcs as
// cubic pieces, finely cut), filled even-odd
fn page_area(doc: &Document, s: ShapeRef) -> f64 {
    let sheet = SheetRef::Shape(s);
    let xf = doc.page_xform(s);
    let (w, h) = (ops::val(doc, sheet, names::WIDTH), ops::val(doc, sheet, names::HEIGHT));
    let mut cs = Vec::new();
    for g in doc.geometry_paths(sheet, w, h) {
        for sub in &g.path.subs {
            if !sub.closed {
                continue;
            }
            let mut pts = Vec::new();
            sub.map(&xf).flatten(64, &mut pts);
            pts.pop();
            cs.push(pts);
        }
    }
    region::area(&region::union(&[Operand::closed(cs)], FillRule::EvenOdd).unwrap())
}

fn kinds(doc: &Document, s: ShapeRef) -> Vec<String> {
    let sheet = SheetRef::Shape(s);
    let mut out = Vec::new();
    for sk in doc.geometry(sheet) {
        for r in doc.rows(sheet, sk) {
            out.push(String::from(r.kind.map_or("", |k| doc.str(k))));
        }
    }
    out
}

fn cell_formula(doc: &Document, s: ShapeRef, addr: &CellAddr) -> String {
    match doc.resolve(SheetRef::Shape(s), addr).formula() {
        Formula::Expr(f) => String::from(doc.str(f)),
        _ => String::new(),
    }
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn bbox(doc: &Document, s: ShapeRef) -> [f64; 4] {
    ops::frame(doc, s).bounds()
}

fn assert_box(b: [f64; 4], want: [f64; 4], tol: f64) {
    for k in 0..4 {
        assert!(close(b[k], want[k], tol), "{b:?} is not {want:?}");
    }
}

// ---------------------------------------------------------------- operations

#[test]
fn union_of_rectangles() {
    let mut f = fx();
    let mut eng = Engine::new();
    let a = rect(&mut f, &mut eng, [0.0, 0.0], [2.0, 2.0]);
    let b = rect(&mut f, &mut eng, [1.0, 1.0], [3.0, 3.0]);
    assert!(available(&f.doc, &[a, b], Operation::Union));
    let r = op(&mut f, &mut eng, &[a, b], Operation::Union);
    assert_eq!(r.len(), 1);
    assert!(f.doc.shape(a).is_none() && f.doc.shape(b).is_none());
    assert_eq!(f.doc.top_shapes(f.owner).collect::<Vec<_>>(), r);
    assert!(close(page_area(&f.doc, r[0]), 7.0, 1e-9));
    assert_box(bbox(&f.doc, r[0]), [0.0, 0.0, 3.0, 3.0], 1e-12);
    let k = kinds(&f.doc, r[0]);
    assert_eq!(k.len(), 9, "{k:?}");
    assert!(k[0] == "MoveTo" && k[1..].iter().all(|x| x == "LineTo"));
    // the rows stretch with the box
    let sk = SecKey { name: names::GEOMETRY, index: 0 };
    let x = cell_formula(&f.doc, r[0], &CellAddr::row(sk, RowKey::Index(2), names::X));
    assert!(x.starts_with("Width*"), "{x}");
    assert_eq!(cell_formula(&f.doc, r[0], &CellAddr::single(names::LOC_PIN_X)), "Width*0.5");
}

#[test]
fn rectangles_sharing_an_edge_and_disjoint_ones() {
    let mut f = fx();
    let mut eng = Engine::new();
    let a = rect(&mut f, &mut eng, [0.0, 0.0], [2.0, 2.0]);
    let b = rect(&mut f, &mut eng, [2.0, 0.0], [4.0, 2.0]);
    let r = op(&mut f, &mut eng, &[a, b], Operation::Union);
    assert_eq!(r.len(), 1);
    assert_eq!(f.doc.geometry(SheetRef::Shape(r[0])).len(), 1);
    assert!(close(page_area(&f.doc, r[0]), 8.0, 1e-9));
    // two apart: one shape, two sections
    let c = rect(&mut f, &mut eng, [6.0, 0.0], [7.0, 1.0]);
    let r2 = op(&mut f, &mut eng, &[r[0], c], Operation::Union);
    assert_eq!(f.doc.geometry(SheetRef::Shape(r2[0])).len(), 2);
    assert!(close(page_area(&f.doc, r2[0]), 9.0, 1e-9));
    // disjoint ones intersected: nothing, and the inputs are gone
    let d = rect(&mut f, &mut eng, [10.0, 0.0], [11.0, 1.0]);
    let none = op(&mut f, &mut eng, &[r2[0], d], Operation::Intersect);
    assert!(none.is_empty());
    assert_eq!(f.doc.top_shapes(f.owner).count(), 0);
}

#[test]
fn arcs_stay_arcs() {
    let mut f = fx();
    let mut eng = Engine::new();
    // a square and a circle of radius 1 about (2, 1): half of it outside
    let a = rect(&mut f, &mut eng, [0.0, 0.0], [2.0, 2.0]);
    let c = ellipse(&mut f, &mut eng, [1.0, 0.0], [3.0, 2.0]);
    let r = op(&mut f, &mut eng, &[a, c], Operation::Union);
    let area = page_area(&f.doc, r[0]);
    assert!(close(area, 4.0 + PI / 2.0, 2e-3), "{area}");
    assert_box(bbox(&f.doc, r[0]), [0.0, 0.0, 3.0, 2.0], 1e-6);
    let k = kinds(&f.doc, r[0]);
    assert!(k.iter().any(|x| x == "EllipticalArcTo"), "{k:?}");
    assert!(k.len() < 12, "{k:?}");
    // the arc's control point and ratio stretch with the box
    let sheet = SheetRef::Shape(r[0]);
    let sk = SecKey { name: names::GEOMETRY, index: 0 };
    let row = f.doc.rows(sheet, sk).into_iter().find(|x| x.kind.is_some_and(|k| f.doc.str(k) == "EllipticalArcTo")).unwrap();
    let d = cell_formula(&f.doc, r[0], &CellAddr::row(sk, row.key, names::D));
    assert!(d.starts_with("Width/Height*"), "{d}");
    let a_ = cell_formula(&f.doc, r[0], &CellAddr::row(sk, row.key, names::A));
    assert!(a_.starts_with("Width*"), "{a_}");
    assert!(close(f.doc.resolve(sheet, &CellAddr::row(sk, row.key, names::D)).num().unwrap(), 1.0, 1e-9));
    // stretched to twice the width: the arc is an ellipse with twice the ratio, the area grows
    // as the box
    let mut tx = f.doc.transaction("Breite");
    tx.set_constant(sheet, &CellAddr::single(names::WIDTH), Value::Num(6.0), None).unwrap();
    eng.recalc(&mut tx, &[sheet]).unwrap();
    tx.commit();
    let area2 = page_area(&f.doc, r[0]);
    assert!(close(area2, 2.0 * area, 4e-3), "{area2} {area}");
    assert!(close(f.doc.resolve(sheet, &CellAddr::row(sk, row.key, names::D)).num().unwrap(), 2.0, 1e-9));
}

#[test]
fn intersect_and_subtract_with_a_circle() {
    let mut f = fx();
    let mut eng = Engine::new();
    let a = rect(&mut f, &mut eng, [0.0, 0.0], [2.0, 2.0]);
    let c = ellipse(&mut f, &mut eng, [1.0, 0.0], [3.0, 2.0]);
    let doc0 = f.doc.clone();
    let r = op(&mut f, &mut eng, &[a, c], Operation::Intersect);
    assert_eq!(r.len(), 1);
    assert!(close(page_area(&f.doc, r[0]), PI / 2.0, 2e-3));
    let k = kinds(&f.doc, r[0]);
    assert_eq!(k.iter().filter(|x| *x == "LineTo").count(), 1, "{k:?}");
    assert_box(bbox(&f.doc, r[0]), [1.0, 0.0, 2.0, 2.0], 1e-6);
    // the square without the circle, the circle without the square
    f.doc = doc0.clone();
    eng = Engine::new();
    let r = op(&mut f, &mut eng, &[a, c], Operation::Subtract);
    assert!(close(page_area(&f.doc, r[0]), 4.0 - PI / 2.0, 2e-3));
    assert_box(bbox(&f.doc, r[0]), [0.0, 0.0, 2.0, 2.0], 1e-6);
    f.doc = doc0;
    eng = Engine::new();
    let r = op(&mut f, &mut eng, &[c, a], Operation::Subtract);
    assert!(close(page_area(&f.doc, r[0]), PI / 2.0, 2e-3));
    assert_box(bbox(&f.doc, r[0]), [2.0, 0.0, 3.0, 2.0], 1e-6);
}

#[test]
fn subtract_makes_a_hole_and_keeps_the_primary_format() {
    let mut f = fx();
    let mut eng = Engine::new();
    let a = rect(&mut f, &mut eng, [0.0, 0.0], [4.0, 4.0]);
    let b = rect(&mut f, &mut eng, [1.0, 1.0], [2.0, 2.0]);
    let c = rect(&mut f, &mut eng, [3.0, -1.0], [5.0, 5.0]);
    // the primary's line colour and text
    {
        let mut tx = f.doc.transaction("Format");
        let lc = tx.intern("LineColor");
        tx.set_formula(SheetRef::Shape(a), &CellAddr::single(lc), "RGB(255,0,0)").unwrap();
        ops::set_text(&mut tx, &mut eng, a, crate::Text::plain("Platte")).unwrap();
        tx.commit();
    }
    let r = op(&mut f, &mut eng, &[a, b, c], Operation::Subtract);
    assert_eq!(r.len(), 1);
    assert!(close(page_area(&f.doc, r[0]), 12.0 - 1.0, 1e-9));
    assert_eq!(f.doc.geometry(SheetRef::Shape(r[0])).len(), 2);
    let lc = f.doc.sym("LineColor").unwrap();
    assert_eq!(cell_formula(&f.doc, r[0], &CellAddr::single(lc)), "RGB(255,0,0)");
    let t = f.doc.shape(r[0]).unwrap().text().cloned().unwrap();
    assert_eq!(t, crate::Text::plain("Platte"));
    assert_eq!(f.doc.shape(r[0]).unwrap().header().styles, StyleRefs::all(1));
    assert_eq!(f.doc.top_shapes(f.owner).count(), 1);
}

#[test]
fn combine_keeps_sections_and_ellipses() {
    let mut f = fx();
    let mut eng = Engine::new();
    let a = rect(&mut f, &mut eng, [0.0, 0.0], [2.0, 2.0]);
    let b = rect(&mut f, &mut eng, [1.0, 1.0], [3.0, 3.0]);
    let e = ellipse(&mut f, &mut eng, [4.0, 0.0], [6.0, 1.0]);
    assert!(!available(&f.doc, &[a], Operation::Combine));
    let r = op(&mut f, &mut eng, &[a, b, e], Operation::Combine);
    assert_eq!(r.len(), 1);
    assert_eq!(f.doc.geometry(SheetRef::Shape(r[0])).len(), 3);
    // the overlap is a hole: 4 + 4 - 2 * 1, the ellipse pi / 2
    assert!(close(page_area(&f.doc, r[0]), 6.0 + PI / 2.0, 2e-3));
    let k = kinds(&f.doc, r[0]);
    assert!(k.iter().filter(|x| *x == "Ellipse").count() == 1, "{k:?}");
    assert_box(bbox(&f.doc, r[0]), [0.0, 0.0, 6.0, 3.0], 1e-9);
}

#[test]
fn fragment_into_pieces_also_by_a_line() {
    let mut f = fx();
    let mut eng = Engine::new();
    let a = rect(&mut f, &mut eng, [0.0, 0.0], [2.0, 2.0]);
    let b = rect(&mut f, &mut eng, [1.0, 1.0], [3.0, 3.0]);
    {
        let mut tx = f.doc.transaction("Text");
        ops::set_text(&mut tx, &mut eng, a, crate::Text::plain("weg")).unwrap();
        tx.commit();
    }
    let r = op(&mut f, &mut eng, &[a, b], Operation::Fragment);
    assert_eq!(r.len(), 3);
    let mut areas: Vec<f64> = r.iter().map(|&s| page_area(&f.doc, s)).collect();
    areas.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert!(close(areas[0], 1.0, 1e-9) && close(areas[1], 3.0, 1e-9) && close(areas[2], 3.0, 1e-9), "{areas:?}");
    assert!(r.iter().all(|&s| f.doc.shape(s).unwrap().text().is_none()));
    // a line through a circle: two halves; the line is gone
    let c = ellipse(&mut f, &mut eng, [10.0, 0.0], [12.0, 2.0]);
    let o = f.owner;
    let l = gesture(&mut f, &mut eng, "Linie", |tx, eng| crate::glue::new_line(tx, eng, o, ([9.0, 1.0], crate::glue::Target::None), ([13.0, 1.0], crate::glue::Target::None)));
    assert!(!available(&f.doc, &[l], Operation::Union));
    let r = op(&mut f, &mut eng, &[c, l], Operation::Fragment);
    assert_eq!(r.len(), 2);
    for &s in &r {
        assert!(close(page_area(&f.doc, s), PI / 2.0, 2e-3));
        assert!(kinds(&f.doc, s).iter().any(|k| k == "EllipticalArcTo"));
    }
    assert!(f.doc.shape(l).is_none());
}

#[test]
fn turned_flipped_and_grouped_inputs() {
    let mut f = fx();
    let mut eng = Engine::new();
    // a 2 x 2 square turned by 45 degrees about its middle (1, 1), flipped as well, and a square
    // over its right half
    let a = rect(&mut f, &mut eng, [0.0, 0.0], [2.0, 2.0]);
    gesture(&mut f, &mut eng, "Drehen", |tx, eng| ops::rotate_shapes(tx, eng, &[a], [1.0, 1.0], PI / 4.0));
    gesture(&mut f, &mut eng, "Spiegeln", |tx, eng| ops::flip_shapes(tx, eng, &[a], true));
    let b = rect(&mut f, &mut eng, [1.0, -1.0], [4.0, 3.0]);
    let doc0 = f.doc.clone();
    let i = op(&mut f, &mut eng, &[a, b], Operation::Intersect);
    // the diamond's right half: 2
    assert!(close(page_area(&f.doc, i[0]), 2.0, 1e-9));
    let s2 = core::f64::consts::SQRT_2;
    assert_box(bbox(&f.doc, i[0]), [1.0, 1.0 - s2, 1.0 + s2, 1.0 + s2], 1e-9);
    // grouped: the group of the diamond and a circle, subtracted from a big square
    f.doc = doc0;
    eng = Engine::new();
    let c = ellipse(&mut f, &mut eng, [-3.0, 0.0], [-1.0, 2.0]);
    let o = f.owner;
    let g = gesture(&mut f, &mut eng, "Gruppieren", |tx, eng| ops::group(tx, eng, o, &[a, c]));
    let big = rect(&mut f, &mut eng, [-5.0, -5.0], [5.0, 5.0]);
    let r = op(&mut f, &mut eng, &[big, g], Operation::Subtract);
    assert!(close(page_area(&f.doc, r[0]), 100.0 - 4.0 - PI, 3e-3));
    assert!(f.doc.shape(g).is_none() && f.doc.shape(b).is_some());
}

#[test]
fn turned_ellipse_and_the_area_identity() {
    // area(A + B) + area(A & B) = area(A) + area(B) for a turned ellipse and a turned rectangle
    let mut f = fx();
    let mut eng = Engine::new();
    let a = ellipse(&mut f, &mut eng, [0.0, 0.0], [4.0, 2.0]);
    gesture(&mut f, &mut eng, "Drehen", |tx, eng| ops::rotate_shapes(tx, eng, &[a], [2.0, 1.0], 0.5));
    let b = rect(&mut f, &mut eng, [1.0, 0.5], [3.5, 4.0]);
    gesture(&mut f, &mut eng, "Drehen", |tx, eng| ops::rotate_shapes(tx, eng, &[b], [2.0, 2.0], -0.3));
    let (aa, ab) = (page_area(&f.doc, a), page_area(&f.doc, b));
    let doc0 = f.doc.clone();
    let u = op(&mut f, &mut eng, &[a, b], Operation::Union);
    let au = page_area(&f.doc, u[0]);
    f.doc = doc0.clone();
    eng = Engine::new();
    let i = op(&mut f, &mut eng, &[a, b], Operation::Intersect);
    let ai = page_area(&f.doc, i[0]);
    assert!(close(au + ai, aa + ab, 5e-3), "{au} {ai} {aa} {ab}");
    // the turned ellipse's arc keeps its axes: C turned, D 2
    let k = kinds(&f.doc, i[0]);
    assert!(k.iter().any(|x| x == "EllipticalArcTo"));
    f.doc = doc0;
    eng = Engine::new();
    let fr = op(&mut f, &mut eng, &[a, b], Operation::Fragment);
    // the ellipse sticks out on two sides, the rectangle too: five pieces
    assert_eq!(fr.len(), 5);
    let total: f64 = fr.iter().map(|&s| page_area(&f.doc, s)).sum();
    assert!(close(total, au, 5e-3));
}

// ---------------------------------------------------------------- frames

#[test]
fn frame_follows_vertex_edits() {
    let mut f = fx();
    let mut eng = Engine::new();
    let r = rect(&mut f, &mut eng, [1.0, 1.0], [3.0, 2.0]);
    // the top right corner out to (4, 3): the box grows, the other corners stay on the page
    gesture(&mut f, &mut eng, "Scheitelpunkt", |tx, eng| ops::set_vertex(tx, eng, r, 0, 3, [4.0, 3.0]));
    assert_box(bbox(&f.doc, r), [1.0, 1.0, 4.0, 3.0], 1e-12);
    let v: Vec<[f64; 2]> = ops::vertices(&f.doc, r).iter().map(|v| v.at).collect();
    let want = [[1.0, 1.0], [3.0, 1.0], [4.0, 3.0], [1.0, 2.0]];
    for (p, q) in v.iter().zip(want.iter()) {
        assert!(close(p[0], q[0], 1e-12) && close(p[1], q[1], 1e-12), "{v:?}");
    }
    let sheet = SheetRef::Shape(r);
    assert!(close(ops::val(&f.doc, sheet, names::WIDTH), 3.0, 1e-12));
    assert!(close(ops::val(&f.doc, sheet, names::PIN_X), 2.5, 1e-12));
    // turned by 90 degrees: still where it was
    gesture(&mut f, &mut eng, "Drehen", |tx, eng| ops::rotate_shapes(tx, eng, &[r], [2.5, 2.0], PI / 2.0));
    let before: Vec<[f64; 2]> = ops::vertices(&f.doc, r).iter().map(|v| v.at).collect();
    let mid = before[1];
    gesture(&mut f, &mut eng, "Scheitelpunkt", |tx, eng| ops::set_vertex(tx, eng, r, 0, 2, [mid[0] + 1.0, mid[1] - 2.0]));
    let after: Vec<[f64; 2]> = ops::vertices(&f.doc, r).iter().map(|v| v.at).collect();
    assert!(close(after[1][0], mid[0] + 1.0, 1e-9) && close(after[1][1], mid[1] - 2.0, 1e-9));
    for k in [0, 2, 3] {
        assert!(close(after[k][0], before[k][0], 1e-9) && close(after[k][1], before[k][1], 1e-9), "{k}");
    }
    let g = geometry_bounds(&f.doc, r).unwrap();
    assert_box(g, [0.0, 0.0, ops::val(&f.doc, sheet, names::WIDTH), ops::val(&f.doc, sheet, names::HEIGHT)], 1e-9);
    // LockCalcWH: the box stays
    {
        let mut tx = f.doc.transaction("Sperre");
        let l = tx.intern("LockCalcWH");
        tx.set_constant(sheet, &CellAddr::single(l), Value::Num(1.0), None).unwrap();
        tx.commit();
    }
    let w0 = ops::val(&f.doc, sheet, names::WIDTH);
    let far = [before[0][0] - 5.0, before[0][1]];
    gesture(&mut f, &mut eng, "Scheitelpunkt", |tx, eng| ops::set_vertex(tx, eng, r, 0, 1, far));
    assert_eq!(ops::val(&f.doc, sheet, names::WIDTH), w0);
}

#[test]
fn frame_follows_a_shape_with_an_arc() {
    let mut f = fx();
    let mut eng = Engine::new();
    // an ellipse's box is its box: nothing to do, also flipped
    let e = ellipse(&mut f, &mut eng, [10.0, 0.0], [12.0, 1.0]);
    gesture(&mut f, &mut eng, "Spiegeln", |tx, eng| ops::flip_shapes(tx, eng, &[e], false));
    let mut tx = f.doc.transaction("x");
    assert!(!fit_frame(&mut tx, &mut eng, e).unwrap());
    drop(tx);
    // a square with half a circle on its right; its lower left corner out to (-1, 0): a triangle
    // more, the arc stays where it is (its D keeps the value 1 in the wider box)
    let a = rect(&mut f, &mut eng, [0.0, 0.0], [2.0, 2.0]);
    let c = ellipse(&mut f, &mut eng, [1.0, 0.0], [3.0, 2.0]);
    let u = op(&mut f, &mut eng, &[a, c], Operation::Union)[0];
    let v = ops::vertices(&f.doc, u);
    let corner = v.iter().find(|x| close(x.at[0], 0.0, 1e-9) && close(x.at[1], 0.0, 1e-9)).copied().unwrap();
    gesture(&mut f, &mut eng, "Scheitelpunkt", |tx, eng| ops::set_vertex(tx, eng, u, corner.section, corner.row, [-1.0, 0.0]));
    assert_box(bbox(&f.doc, u), [-1.0, 0.0, 3.0, 2.0], 1e-6);
    let area = page_area(&f.doc, u);
    assert!(close(area, 4.0 + PI / 2.0 + 1.0, 2e-3), "{area}");
    let sheet = SheetRef::Shape(u);
    let sk = SecKey { name: names::GEOMETRY, index: 0 };
    let row = f.doc.rows(sheet, sk).into_iter().find(|x| x.kind.is_some_and(|k| f.doc.str(k) == "EllipticalArcTo")).unwrap();
    assert!(close(f.doc.resolve(sheet, &CellAddr::row(sk, row.key, names::D)).num().unwrap(), 1.0, 1e-9));
}

#[test]
fn alignment_box_of_a_group() {
    let mut f = fx();
    let mut eng = Engine::new();
    let a = rect(&mut f, &mut eng, [0.0, 0.0], [1.0, 1.0]);
    let b = rect(&mut f, &mut eng, [2.0, 2.0], [3.0, 3.0]);
    let o = f.owner;
    let g = gesture(&mut f, &mut eng, "Gruppieren", |tx, eng| ops::group(tx, eng, o, &[a, b]));
    assert_box(bbox(&f.doc, g), [0.0, 0.0, 3.0, 3.0], 1e-12);
    // a member moved out of the box
    gesture(&mut f, &mut eng, "Verschieben", |tx, eng| ops::move_shapes(tx, eng, &[b], [2.0, 1.0]));
    let (ca, cb) = (ops::frame(&f.doc, a).corners(), ops::frame(&f.doc, b).corners());
    assert_box(bbox(&f.doc, g), [0.0, 0.0, 3.0, 3.0], 1e-12);
    let changed = gesture(&mut f, &mut eng, "Ausrichtungsfeld", |tx, eng| update_alignment_box(tx, eng, g));
    assert!(changed);
    assert_box(bbox(&f.doc, g), [0.0, 0.0, 5.0, 4.0], 1e-12);
    let (na, nb) = (ops::frame(&f.doc, a).corners(), ops::frame(&f.doc, b).corners());
    for k in 0..4 {
        assert!(close(na[k][0], ca[k][0], 1e-12) && close(na[k][1], ca[k][1], 1e-12));
        assert!(close(nb[k][0], cb[k][0], 1e-12) && close(nb[k][1], cb[k][1], 1e-12));
    }
    // the members' pins keep the group's proportional form
    let px = cell_formula(&f.doc, b, &CellAddr::single(names::PIN_X));
    assert!(px.starts_with("Sheet.") && px.contains("!Width*"), "{px}");
    // the group scales its members afterwards as before
    let mut tx = f.doc.transaction("x");
    assert!(!update_alignment_box(&mut tx, &mut eng, g).unwrap());
}

#[test]
fn availability() {
    let mut f = fx();
    let mut eng = Engine::new();
    let a = rect(&mut f, &mut eng, [0.0, 0.0], [1.0, 1.0]);
    let b = rect(&mut f, &mut eng, [2.0, 0.0], [3.0, 1.0]);
    let o = f.owner;
    let l = gesture(&mut f, &mut eng, "Linie", |tx, eng| crate::glue::new_line(tx, eng, o, ([0.0, 5.0], crate::glue::Target::None), ([4.0, 5.0], crate::glue::Target::None)));
    let d = &f.doc;
    assert!(available(d, &[a], Operation::Union) && available(d, &[a], Operation::Fragment));
    assert!(!available(d, &[a], Operation::Intersect) && !available(d, &[a], Operation::Subtract) && !available(d, &[a], Operation::Combine));
    assert!(available(d, &[a, b], Operation::Intersect) && available(d, &[a, b], Operation::Subtract));
    assert!(available(d, &[a, l], Operation::Combine) && !available(d, &[a, l], Operation::Intersect));
    assert!(!available(d, &[l, a, b], Operation::Subtract) && available(d, &[a, l, b], Operation::Subtract));
    assert!(!available(d, &[], Operation::Union));
    let mut tx = f.doc.transaction("x");
    assert_eq!(operate(&mut tx, &mut eng, &[a], Operation::Intersect), Err(EditError::Invalid));
}
