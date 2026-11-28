// tests of autoconn (cargo test -p jflib-diagram-core --lib)
//
//   1. which connector a dropped box splits; the Visio cells that allow it
//   2. split: Connect entries, glue of both halves, the copy's format without the text
//   3. insert: the shape lined up and spaced, the shapes downstream moved on, one beside left
//   4. heal: one in and one out joined, ambiguity, text kept, chains, loops
//   5. plow: PlowCode and ShapePlowCode
//
// Every document is built here; nothing is read from files.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::formula::Engine;
use crate::names;
use crate::shape::{ShapeData, Text};
use crate::sheet::{Row, RowKey, SecKey, Section};
use crate::{Cell, DocKind, Formula, Page, PageHeader, Value};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * (1.0 + b.abs())
}

struct Fx {
    doc: Document,
    page: PageRef,
    owner: Owner,
    eng: Engine,
    next: u32,
}

fn fx() -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap();
    Fx { doc, page, owner: Owner::Page(page), eng: Engine::new(), next: 1 }
}

fn cnum(doc: &mut Document, name: &str, v: f64) -> Cell {
    Cell::constant(doc.intern(name), Value::Num(v), None)
}

fn cf(doc: &mut Document, name: &str, f: &str, v: f64) -> Cell {
    let (n, f) = (doc.intern(name), doc.intern(f));
    Cell { name: n, value: Value::Num(v), formula: Formula::Expr(f), unit: None, error: None, stale: false }
}

impl Fx {
    // a rectangle w x h around `at` with ShapeSplit `split` and connection points in the middle
    // of each side
    fn rect(&mut self, at: [f64; 2], size: [f64; 2], split: bool) -> ShapeRef {
        let doc = &mut self.doc;
        let (w, h) = (size[0], size[1]);
        let mut d = ShapeData::new(ShapeKind::Shape);
        let cells = vec![
            cnum(doc, "PinX", at[0]),
            cnum(doc, "PinY", at[1]),
            cnum(doc, "Width", w),
            cnum(doc, "Height", h),
            cf(doc, "LocPinX", "Width*0.5", w * 0.5),
            cf(doc, "LocPinY", "Height*0.5", h * 0.5),
            cnum(doc, "Angle", 0.0),
            cnum(doc, "ShapeSplit", if split { 1.0 } else { 0.0 }),
        ];
        d.sheet.cells.extend(cells);
        let mut geo = Section::new(SecKey::one(names::GEOMETRY));
        geo.ix_written = true;
        for (i, p) in [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h], [0.0, 0.0]].iter().enumerate() {
            let kind = if i == 0 { names::MOVE_TO } else { names::LINE_TO };
            let mut r = Row::new(RowKey::Index(i as u32 + 1));
            r.kind = Some(kind);
            r.cells = vec![Cell::constant(names::X, Value::Num(p[0]), None), Cell::constant(names::Y, Value::Num(p[1]), None)];
            geo.rows.push(r);
        }
        d.sheet.sections.push(geo);
        let mut con = Section::new(SecKey::one(names::CONNECTION));
        for (i, (fx, fy)) in [(0.0, 0.5), (1.0, 0.5), (0.5, 0.0), (0.5, 1.0)].into_iter().enumerate() {
            let mut r = Row::new(RowKey::Index(i as u32));
            r.cells = vec![cf(doc, "X", &alloc::format!("Width*{fx}"), w * fx), cf(doc, "Y", &alloc::format!("Height*{fy}"), h * fy)];
            con.rows.push(r);
        }
        d.sheet.sections.push(con);
        let id = self.next;
        self.next += 1;
        let tree = &mut self.doc.page_mut(self.page).unwrap().shapes;
        let slot = tree.insert(None, id, d).unwrap();
        ShapeRef { owner: self.owner, slot }
    }

    // a dynamic connector glued to the whole of a and b
    fn join(&mut self, a: ShapeRef, b: ShapeRef) -> ShapeRef {
        let (pa, pb) = (ops::frame(&self.doc, a).centre(), ops::frame(&self.doc, b).centre());
        let mut tx = self.doc.transaction("Verbinden");
        let c = glue::new_connector(&mut tx, &mut self.eng, self.owner, (pa, Target::Shape(a)), (pb, Target::Shape(b))).unwrap();
        tx.commit();
        self.next = self.next.max(self.doc.shape(c).unwrap().id() + 1);
        c
    }

    fn text(&mut self, s: ShapeRef, t: &str) {
        let mut tx = self.doc.transaction("Text");
        ops::set_text(&mut tx, &mut self.eng, s, Text::plain(t)).unwrap();
        tx.commit();
    }

    fn cell(&mut self, sheet: SheetRef, name: &str, v: f64) {
        let mut tx = self.doc.transaction("Zelle");
        let a = tx.single(name);
        tx.set_constant(sheet, &a, Value::Num(v), None).unwrap();
        tx.commit();
    }

    fn mid(&self, s: ShapeRef) -> [f64; 2] {
        centre(structure::shape_box(&self.doc, s))
    }

    fn box_at(&self, at: [f64; 2], size: [f64; 2]) -> [f64; 4] {
        [at[0] - size[0] * 0.5, at[1] - size[1] * 0.5, at[0] + size[0] * 0.5, at[1] + size[1] * 0.5]
    }

    // delete as the app does: heal, detach what stays, delete, route the joined connectors
    fn delete(&mut self, shapes: &[ShapeRef]) -> Healed {
        let mut tx = self.doc.transaction("Löschen");
        let h = heal(&mut tx, &mut self.eng, self.owner, shapes).unwrap();
        let mut all = shapes.to_vec();
        all.extend(h.gone.iter().copied());
        glue::detach(&mut tx, &mut self.eng, self.owner, &all).unwrap();
        tx.delete_shapes(&all).unwrap();
        glue::update(&mut tx, &mut self.eng, &h.joined).unwrap();
        tx.commit();
        h
    }
}

fn shape_ends(doc: &Document, c: ShapeRef) -> [Option<ShapeRef>; 2] {
    glue::glued(doc, c).map(shape_of)
}

// ---- 1. the connector under a box

#[test]
fn a_box_on_the_way_finds_the_connector() {
    let mut f = fx();
    let a = f.rect([2.0, 8.0], [1.0, 1.0], true);
    let b = f.rect([2.0, 4.0], [1.0, 1.0], true);
    let c = f.join(a, b);
    let o = f.owner;
    // on the way between them: that connector
    assert_eq!(split_target(&f.doc, o, f.box_at([2.0, 6.0], [1.0, 0.6]), &[]), Some(c));
    // beside the way, over a shape it joins, holding one of its ends: none
    assert_eq!(split_target(&f.doc, o, f.box_at([4.0, 6.0], [1.0, 0.6]), &[]), None);
    assert_eq!(split_target(&f.doc, o, f.box_at([2.0, 7.4], [1.0, 0.6]), &[]), None);
    assert_eq!(split_target(&f.doc, o, f.box_at([2.0, 6.0], [1.0, 4.0]), &[]), None);
    // a shape glued to it is skipped
    assert_eq!(split_target(&f.doc, o, f.box_at([2.0, 6.0], [1.0, 0.6]), &[a]), None);
    // the connector says no
    f.cell(SheetRef::Shape(c), "ShapeSplittable", 0.0);
    assert_eq!(split_target(&f.doc, o, f.box_at([2.0, 6.0], [1.0, 0.6]), &[]), None);
}

#[test]
fn the_cells_decide() {
    let mut f = fx();
    let a = f.rect([2.0, 8.0], [1.0, 1.0], true);
    let b = f.rect([2.0, 4.0], [1.0, 1.0], false);
    let lone = f.rect([6.0, 6.0], [1.0, 1.0], true);
    let c = f.join(a, b);
    assert!(splittable(&f.doc, c) && !splits(&f.doc, c));
    assert!(splits(&f.doc, a) && !splits(&f.doc, b));
    // a shape in a flow already: no
    assert!(!can_insert(&f.doc, f.owner, a) && can_insert(&f.doc, f.owner, lone));
    // the page: missing means yes (Visio's default), 0 no
    assert!(page_splits(&f.doc, f.page));
    f.cell(SheetRef::Page(f.page), "PageShapeSplit", 0.0);
    assert!(!page_splits(&f.doc, f.page));
}

// ---- 2. split

#[test]
fn split_glues_both_halves() {
    let mut f = fx();
    let a = f.rect([2.0, 8.0], [1.0, 1.0], true);
    let b = f.rect([2.0, 4.0], [1.0, 1.0], true);
    let s = f.rect([2.0, 6.0], [1.0, 0.6], true);
    let c = f.join(a, b);
    f.text(c, "ja");
    f.cell(SheetRef::Shape(c), "LineColor", 3.0);
    let mut tx = f.doc.transaction("Teilen");
    let n = split(&mut tx, &mut f.eng, c, s).unwrap();
    tx.commit().unwrap();
    let doc = &f.doc;
    assert_eq!(shape_ends(doc, c), [Some(a), Some(s)]);
    assert_eq!(shape_ends(doc, n), [Some(s), Some(b)]);
    assert_eq!(glue::glued(doc, n)[0], Target::Shape(s));
    // the copy: a connector with the format, without the text; the original keeps it
    assert!(splittable(doc, n));
    assert_eq!(doc.resolve(SheetRef::Shape(n), &CellAddr::single(doc.sym("LineColor").unwrap())).num(), Some(3.0));
    assert_eq!(field::plain(doc, c), "ja");
    assert_eq!(field::plain(doc, n), "");
    // the ways: A down to S, S down to B, straight
    let (p, q) = (glue::path(doc, c), glue::path(doc, n));
    assert!(close(p[0][1], 7.5) && close(p.last().unwrap()[1], 6.3), "{p:?}");
    assert!(close(q[0][1], 5.7) && close(q.last().unwrap()[1], 4.5), "{q:?}");
    // the Connect entries: two per connector, to PinX (part 3: dynamic)
    let page = doc.page(f.page).unwrap();
    let ids = [doc.shape(c).unwrap().id(), doc.shape(n).unwrap().id()];
    assert_eq!(page.connects.iter().filter(|k| ids.contains(&k.from_sheet)).count(), 4);
    assert!(page.connects.iter().filter(|k| ids.contains(&k.from_sheet)).all(|k| k.to_part == Some(3)));
    // not a 1-D shape, not something that is no connector
    let mut tx = f.doc.transaction("x");
    assert_eq!(split(&mut tx, &mut f.eng, c, n), Err(EditError::Invalid));
    assert_eq!(split(&mut tx, &mut f.eng, a, s), Err(EditError::Invalid));
}

#[test]
fn split_keeps_a_static_end() {
    let mut f = fx();
    let a = f.rect([2.0, 6.0], [1.0, 1.0], true);
    let b = f.rect([8.0, 6.0], [1.0, 1.0], true);
    let s = f.rect([5.0, 6.0], [1.0, 1.0], true);
    let mut tx = f.doc.transaction("Verbinden");
    let c = glue::new_connector(&mut tx, &mut f.eng, f.owner, ([2.5, 6.0], Target::Point { shape: a, row: 1 }), ([7.5, 6.0], Target::Point { shape: b, row: 0 })).unwrap();
    tx.commit();
    let mut tx = f.doc.transaction("Teilen");
    let n = split(&mut tx, &mut f.eng, c, s).unwrap();
    tx.commit().unwrap();
    assert_eq!(glue::glued(&f.doc, c), [Target::Point { shape: a, row: 1 }, Target::Shape(s)]);
    assert_eq!(glue::glued(&f.doc, n), [Target::Shape(s), Target::Point { shape: b, row: 0 }]);
}

// ---- 3. insert

#[test]
fn insert_lines_up_and_makes_room() {
    let mut f = fx();
    // a flow from the top down: A, B, D; E hangs off B to the side above the drop
    let a = f.rect([2.0, 8.0], [1.0, 1.0], true);
    let b = f.rect([2.0, 6.0], [1.0, 1.0], true);
    let d = f.rect([2.0, 4.0], [1.0, 1.0], true);
    let e = f.rect([5.0, 8.0], [1.0, 1.0], true);
    let ab = f.join(a, b);
    f.join(b, d);
    f.join(b, e);
    // S dropped a little to the right of the way, 0.2 under A
    let s = f.rect([2.3, 7.0], [1.0, 0.6], true);
    let before = (f.mid(d), f.mid(e));
    let gap = [0.3, 0.3];
    let mut tx = f.doc.transaction("Ablegen");
    let r = insert(&mut tx, &mut f.eng, f.owner, s, ab, gap).unwrap();
    tx.commit().unwrap();
    // S on A's middle line, 0.3 under A
    let sb = structure::shape_box(&f.doc, s);
    assert!(close(f.mid(s)[0], 2.0), "{sb:?}");
    assert!(close(sb[3], 7.2), "{sb:?}");
    // B 0.3 under S, D along with it, E (beside, above S) stays
    let bb = structure::shape_box(&f.doc, b);
    assert!(close(bb[3], sb[1] - 0.3), "{bb:?} {sb:?}");
    let moved = 6.5 - bb[3];
    assert!(close(f.mid(d)[1], before.0[1] - moved), "{:?}", f.mid(d));
    assert_eq!(f.mid(e), before.1);
    assert_eq!(r.moved, vec![s, b, d]);
    assert_eq!(shape_ends(&f.doc, ab), [Some(a), Some(s)]);
    assert_eq!(shape_ends(&f.doc, r.connector), [Some(s), Some(b)]);
}

#[test]
fn insert_with_room_moves_nothing_after() {
    let mut f = fx();
    let a = f.rect([2.0, 9.0], [1.0, 1.0], true);
    let b = f.rect([2.0, 3.0], [1.0, 1.0], true);
    let ab = f.join(a, b);
    let s = f.rect([2.0, 6.0], [1.0, 1.0], true);
    let mut tx = f.doc.transaction("Ablegen");
    let r = insert(&mut tx, &mut f.eng, f.owner, s, ab, [0.3, 0.3]).unwrap();
    tx.commit().unwrap();
    assert!(r.moved.is_empty());
    assert_eq!(f.mid(b), [2.0, 3.0]);
    assert_eq!(f.mid(s), [2.0, 6.0]);
}

#[test]
fn a_fixed_shape_stops_the_room_making() {
    let mut f = fx();
    let a = f.rect([2.0, 3.0], [1.0, 1.0], true);
    let b = f.rect([4.0, 3.0], [1.0, 1.0], true);
    let ab = f.join(a, b);
    f.cell(SheetRef::Shape(b), "ShapePlowCode", 1.0);
    let s = f.rect([3.0, 3.0], [0.6, 0.6], true);
    let mut tx = f.doc.transaction("Ablegen");
    let r = insert(&mut tx, &mut f.eng, f.owner, s, ab, [0.3, 0.3]).unwrap();
    tx.commit().unwrap();
    assert_eq!(r.moved, vec![s]);
    assert_eq!(f.mid(b), [4.0, 3.0]);
}

#[test]
fn insert_undoes_as_one_step() {
    let mut f = fx();
    let a = f.rect([2.0, 8.0], [1.0, 1.0], true);
    let b = f.rect([2.0, 6.0], [1.0, 1.0], true);
    let ab = f.join(a, b);
    let s = f.rect([2.0, 7.0], [1.0, 0.6], true);
    let before: Vec<(u32, String)> = f.doc.all_shapes(f.owner).map(|x| (f.doc.shape(x).unwrap().id(), alloc::format!("{:?}", f.doc.shape(x).unwrap().sheet()))).collect();
    let connects = f.doc.page(f.page).unwrap().connects.clone();
    let mut tx = f.doc.transaction("Ablegen");
    insert(&mut tx, &mut f.eng, f.owner, s, ab, [0.3, 0.3]).unwrap();
    let mut ch = tx.commit().unwrap();
    assert!(ch.undo(&mut f.doc));
    let after: Vec<(u32, String)> = f.doc.all_shapes(f.owner).map(|x| (f.doc.shape(x).unwrap().id(), alloc::format!("{:?}", f.doc.shape(x).unwrap().sheet()))).collect();
    assert_eq!(after, before);
    assert_eq!(f.doc.page(f.page).unwrap().connects, connects);
}

// ---- 4. heal

#[test]
fn deleting_the_middle_heals_the_flow() {
    let mut f = fx();
    let a = f.rect([2.0, 8.0], [1.0, 1.0], true);
    let x = f.rect([2.0, 6.0], [1.0, 1.0], true);
    let b = f.rect([2.0, 4.0], [1.0, 1.0], true);
    let ax = f.join(a, x);
    let xb = f.join(x, b);
    let h = f.delete(&[x]);
    assert_eq!(h, Healed { gone: vec![xb], joined: vec![ax] });
    assert!(f.doc.shape(xb).is_none() && f.doc.shape(x).is_none());
    assert_eq!(shape_ends(&f.doc, ax), [Some(a), Some(b)]);
    // routed straight down from A to B
    let p = glue::path(&f.doc, ax);
    assert!(close(p[0][1], 7.5) && close(p.last().unwrap()[1], 4.5), "{p:?}");
}

#[test]
fn text_and_ambiguity_decide() {
    let mut f = fx();
    let a = f.rect([2.0, 8.0], [1.0, 1.0], true);
    let a2 = f.rect([4.0, 8.0], [1.0, 1.0], true);
    let x = f.rect([3.0, 6.0], [1.0, 1.0], true);
    let b = f.rect([2.0, 4.0], [1.0, 1.0], true);
    let b2 = f.rect([4.0, 4.0], [1.0, 1.0], true);
    let c1 = f.join(a, x);
    let c2 = f.join(a2, x);
    let c3 = f.join(x, b);
    let c4 = f.join(x, b2);
    f.text(c4, "nein");
    // two in, two out: nothing heals; the one with text stays (its end lets go)
    let h = f.delete(&[x]);
    assert_eq!(h, Healed { gone: vec![c1, c2, c3], joined: vec![] });
    assert!(f.doc.shape(c4).is_some());
    assert_eq!(shape_ends(&f.doc, c4), [None, Some(b2)]);
}

#[test]
fn the_one_with_text_survives() {
    let mut f = fx();
    let a = f.rect([2.0, 8.0], [1.0, 1.0], true);
    let x = f.rect([2.0, 6.0], [1.0, 1.0], true);
    let b = f.rect([2.0, 4.0], [1.0, 1.0], true);
    let ax = f.join(a, x);
    let xb = f.join(x, b);
    f.text(xb, "weiter");
    let h = f.delete(&[x]);
    assert_eq!(h, Healed { gone: vec![ax], joined: vec![xb] });
    assert_eq!(shape_ends(&f.doc, xb), [Some(a), Some(b)]);
    // both with text: no healing, both stay
    let mut f = fx();
    let a = f.rect([2.0, 8.0], [1.0, 1.0], true);
    let x = f.rect([2.0, 6.0], [1.0, 1.0], true);
    let b = f.rect([2.0, 4.0], [1.0, 1.0], true);
    let ax = f.join(a, x);
    let xb = f.join(x, b);
    f.text(ax, "1");
    f.text(xb, "2");
    assert_eq!(f.delete(&[x]), Healed::default());
    assert_eq!(shape_ends(&f.doc, ax), [Some(a), None]);
    assert_eq!(shape_ends(&f.doc, xb), [None, Some(b)]);
}

#[test]
fn chains_heal_and_loops_do_not() {
    let mut f = fx();
    let a = f.rect([2.0, 10.0], [1.0, 1.0], true);
    let x = f.rect([2.0, 8.0], [1.0, 1.0], true);
    let y = f.rect([2.0, 6.0], [1.0, 1.0], true);
    let b = f.rect([2.0, 4.0], [1.0, 1.0], true);
    let ax = f.join(a, x);
    let xy = f.join(x, y);
    let yb = f.join(y, b);
    let h = f.delete(&[x, y]);
    assert_eq!(h.joined, vec![ax]);
    assert!(h.gone.contains(&xy) && h.gone.contains(&yb));
    assert_eq!(shape_ends(&f.doc, ax), [Some(a), Some(b)]);
    // A -> X -> A: no connector from A to itself, both go
    let mut f = fx();
    let a = f.rect([2.0, 8.0], [1.0, 1.0], true);
    let x = f.rect([5.0, 8.0], [1.0, 1.0], true);
    let c1 = f.join(a, x);
    let c2 = f.join(x, a);
    let h = f.delete(&[x]);
    assert_eq!(h, Healed { gone: vec![c1, c2], joined: vec![] });
}

// ---- 5. plow

#[test]
fn plowing_follows_the_cells() {
    let mut f = fx();
    let t = f.rect([3.0, 3.0], [1.0, 1.0], false);
    let s = f.rect([2.4, 3.0], [1.0, 1.0], false);
    let o = f.owner;
    // PlowCode 0: nothing moves
    let mut tx = f.doc.transaction("Ablegen");
    assert!(plow(&mut tx, &mut f.eng, o, &[s], [0.25, 0.25]).unwrap().is_empty());
    drop(tx);
    assert!(!plows(&f.doc, f.page));
    // PlowCode 1: T moves the shortest way out (right) to the spacing
    f.cell(SheetRef::Page(f.page), "PlowCode", 1.0);
    assert!(plows(&f.doc, f.page));
    let mut tx = f.doc.transaction("Ablegen");
    assert_eq!(plow(&mut tx, &mut f.eng, o, &[s], [0.25, 0.25]).unwrap(), vec![t]);
    tx.commit();
    assert!(close(f.mid(t)[0], 2.9 + 0.25 + 0.5) && close(f.mid(t)[1], 3.0), "{:?}", f.mid(t));
    // ShapePlowCode 1 keeps a shape, 2 moves it on a page that does not plow
    let mut f = fx();
    let t = f.rect([3.0, 3.0], [1.0, 1.0], false);
    let s = f.rect([2.4, 3.0], [1.0, 1.0], false);
    f.cell(SheetRef::Shape(t), "ShapePlowCode", 2.0);
    let mut tx = f.doc.transaction("Ablegen");
    assert_eq!(plow(&mut tx, &mut f.eng, f.owner, &[s], [0.25, 0.25]).unwrap(), vec![t]);
    drop(tx);
    f.cell(SheetRef::Page(f.page), "PlowCode", 1.0);
    f.cell(SheetRef::Shape(t), "ShapePlowCode", 1.0);
    let mut tx = f.doc.transaction("Ablegen");
    assert!(plow(&mut tx, &mut f.eng, f.owner, &[s], [0.25, 0.25]).unwrap().is_empty());
}

#[test]
fn spacing_reads_the_page() {
    let mut f = fx();
    assert!(close(spacing(&f.doc, f.page)[0], 7.5 / 25.4));
    f.cell(SheetRef::Page(f.page), "AvenueSizeX", 0.5);
    assert!(close(spacing(&f.doc, f.page)[0], 0.5));
}
