// tests of tools (cargo test -p jflib-diagram-core --lib tools)
//
//   1. number: rows, columns, the selection's order, along the connectors (a branch, a loop);
//      before / after / instead of the text, prefix, start and step, numbering again replaces
//      the number, a locked text is left out, numbering on drop counts on
//   2. array: the offsets (edge to edge, middle to middle, every direction), copies made at
//      their cells, refused grids
//   3. dimension: anchors between two shapes and of one shape (connection points glued, box
//      points tied by formula), the ends follow when the shapes move, a free end stays
//
// Every document is built here; nothing is read from files.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::array::{self, Grid};
use super::dimension::{self, Anchor, Kind};
use super::number::{self, Numbering, Order, Place};
use crate::cell::{Cell, Formula, Value};
use crate::document::{DocKind, Document, Owner, PageRef, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::glue::{self, End, Target};
use crate::names;
use crate::shape::{ShapeData, ShapeKind, Text, TextItem};
use crate::sheet::{Row, RowKey, SecKey, Section};
use crate::units::Unit;
use crate::{ops, Page, PageHeader};

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
    Fx { doc, page, owner: Owner::Page(page), eng: Engine::new(), next: 0 }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + b.abs())
}

fn near(p: [f64; 2], q: [f64; 2]) -> bool {
    close(p[0], q[0]) && close(p[1], q[1])
}

impl Fx {
    // an upright rectangle w x h around `at` with its outline and the text `text`; `points`:
    // connection points in the middle of each side (bottom, right, top, left, as the own
    // masters have them)
    fn rect(&mut self, at: [f64; 2], size: [f64; 2], text: &str, points: bool) -> ShapeRef {
        let doc = &mut self.doc;
        let (w, h) = (size[0], size[1]);
        let mut d = ShapeData::new(ShapeKind::Shape);
        let num = |doc: &mut Document, n: &str, v: f64| Cell::constant(doc.intern(n), Value::Num(v), None);
        let lf = |doc: &mut Document, n: &str, f: &str, v: f64| {
            let (n, f) = (doc.intern(n), doc.intern(f));
            Cell { name: n, value: Value::Num(v), formula: Formula::Expr(f), unit: None, error: None, stale: false }
        };
        d.sheet.cells = vec![num(doc, "PinX", at[0]), num(doc, "PinY", at[1]), num(doc, "Width", w), num(doc, "Height", h), lf(doc, "LocPinX", "Width*0.5", w * 0.5), lf(doc, "LocPinY", "Height*0.5", h * 0.5)];
        let mut geo = Section::new(SecKey::one(names::GEOMETRY));
        geo.ix_written = true;
        for (i, p) in [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h], [0.0, 0.0]].iter().enumerate() {
            let mut r = Row::new(RowKey::Index(i as u32 + 1));
            r.kind = Some(if i == 0 { names::MOVE_TO } else { names::LINE_TO });
            r.cells = vec![Cell::constant(names::X, Value::Num(p[0]), None), Cell::constant(names::Y, Value::Num(p[1]), None)];
            geo.rows.push(r);
        }
        d.sheet.sections.push(geo);
        if points {
            let mut con = Section::new(SecKey::one(names::CONNECTION));
            for (i, (fx, fy)) in [(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)].into_iter().enumerate() {
                let mut r = Row::new(RowKey::Index(i as u32));
                r.cells = vec![lf(doc, "X", &alloc::format!("Width*{fx}"), w * fx), lf(doc, "Y", &alloc::format!("Height*{fy}"), h * fy)];
                con.rows.push(r);
            }
            d.sheet.sections.push(con);
        }
        if !text.is_empty() {
            d.text = Some(alloc::boxed::Box::new(Text::plain(text)));
        }
        let id = self.doc.all_shapes(self.owner).filter_map(|s| self.doc.shape(s).map(|x| x.id())).max().unwrap_or(0).max(self.next) + 1;
        self.next = id;
        let slot = self.doc.page_mut(self.page).unwrap().shapes.insert(None, id, d).unwrap();
        ShapeRef { owner: self.owner, slot }
    }

    fn edit<T>(&mut self, f: impl FnOnce(&mut Transaction, &mut Engine) -> Result<T, EditError>) -> T {
        let mut tx = self.doc.transaction("test");
        let r = f(&mut tx, &mut self.eng).expect("edit");
        let _ = tx.commit();
        r
    }

    fn text(&self, s: ShapeRef) -> String {
        self.doc.shape(s).and_then(|x| x.text()).map(|t| t.chars()).unwrap_or_default()
    }

    // a dynamic connector from a to b
    fn connect(&mut self, a: ShapeRef, b: ShapeRef) -> ShapeRef {
        let (pa, pb) = (ops::frame(&self.doc, a).centre(), ops::frame(&self.doc, b).centre());
        let owner = self.owner;
        self.edit(|tx, eng| glue::new_connector(tx, eng, owner, (pa, Target::Shape(a)), (pb, Target::Shape(b))))
    }

    // move a shape as a gesture does: its pin, then what is glued to it
    fn move_by(&mut self, s: ShapeRef, d: [f64; 2]) {
        let owner = self.owner;
        self.edit(|tx, eng| {
            ops::move_shapes(tx, eng, &[s], d)?;
            let deps = glue::dependents(tx.doc(), owner, &[s]);
            glue::update(tx, eng, &deps)
        });
    }

    fn ends(&self, s: ShapeRef) -> ([f64; 2], [f64; 2]) {
        glue::ends(&self.doc, s).unwrap()
    }
}

// ---- 1. number

// a grid of boxes, slightly out of line, created out of order
fn scattered(f: &mut Fx) -> Vec<ShapeRef> {
    // (x, y) of the middles: two rows (y about 5 and about 2), three columns
    let at = [[4.1, 2.0], [1.0, 5.1], [7.0, 4.9], [1.2, 1.9], [4.0, 5.0], [6.9, 2.1]];
    at.iter().enumerate().map(|(i, &p)| f.rect(p, [1.0, 0.6], &alloc::format!("T{i}"), false)).collect()
}

#[test]
fn rows_and_columns_order() {
    let mut f = fx();
    let s = scattered(&mut f);
    let rows = number::order(&f.doc, &s, Order::Rows);
    assert_eq!(rows, vec![s[1], s[4], s[2], s[3], s[0], s[5]]);
    let cols = number::order(&f.doc, &s, Order::Columns);
    assert_eq!(cols, vec![s[1], s[3], s[4], s[0], s[2], s[5]]);
    let sel = number::order(&f.doc, &[s[5], s[0]], Order::Selection);
    assert_eq!(sel, vec![s[5], s[0]]);
}

#[test]
fn connection_order_follows_the_flow() {
    let mut f = fx();
    // a start on the right, a step, a decision with two branches that meet again, a loop back
    let start = f.rect([8.0, 8.0], [1.0, 0.5], "Start", false);
    let step = f.rect([5.0, 6.0], [1.0, 0.5], "Schritt", false);
    let dec = f.rect([5.0, 4.0], [1.0, 0.5], "Entscheidung", false);
    let yes = f.rect([7.0, 2.0], [1.0, 0.5], "Ja", false);
    let no = f.rect([3.0, 2.0], [1.0, 0.5], "Nein", false);
    let end = f.rect([5.0, 0.5], [1.0, 0.5], "Ende", false);
    let alone = f.rect([1.0, 8.0], [1.0, 0.5], "Notiz", false);
    for (a, b) in [(start, step), (step, dec), (dec, yes), (dec, no), (yes, end), (no, end), (no, step)] {
        f.connect(a, b);
    }
    let all = vec![end, no, yes, dec, step, start, alone];
    let o = number::order(&f.doc, &all, Order::Connections);
    // the loop no -> step: step waits for no, which comes later; the first in rows that is left
    // (step) goes on; the lonely note is ready from the start and first in rows
    assert_eq!(o, vec![alone, start, step, dec, no, yes, end]);
}

#[test]
fn numbers_go_before_after_or_instead_of_the_text() {
    let mut f = fx();
    let s = scattered(&mut f);
    let order = number::order(&f.doc, &s, Order::Rows);
    let n = Numbering { start: 10, step: 5, prefix: String::from("S-"), place: Place::Before };
    let next = f.edit(|tx, eng| number::number(tx, eng, &order, &n));
    assert_eq!(next, 40);
    assert_eq!(f.text(s[1]), "S-10 T1");
    assert_eq!(f.text(s[5]), "S-35 T5");
    // again, now after the text: the old number goes, one number only
    let n2 = Numbering { start: 1, step: 1, prefix: String::new(), place: Place::After };
    f.edit(|tx, eng| number::number(tx, eng, &order, &n2));
    assert_eq!(f.text(s[1]), "T1 1");
    assert_eq!(f.text(s[0]), "T0 5");
    assert_eq!(crate::structure::user_text(&f.doc, s[0], number::NUMBER_CELL).as_deref(), Some("5"));
    // instead of the text, then back before it: the text is gone with "instead"
    f.edit(|tx, eng| number::set_number(tx, eng, s[0], "A", Place::Instead));
    assert_eq!(f.text(s[0]), "A");
    f.edit(|tx, eng| number::set_number(tx, eng, s[0], "B", Place::Before));
    assert_eq!(f.text(s[0]), "B");
    // removed: the text without its number
    f.edit(|tx, eng| number::remove(tx, eng, s[1]));
    assert_eq!(f.text(s[1]), "T1");
    // an empty shape gets the number as its text
    let e = f.rect([10.0, 10.0], [1.0, 1.0], "", false);
    f.edit(|tx, eng| number::set_number(tx, eng, e, "7", Place::After));
    assert_eq!(f.text(e), "7");
}

#[test]
fn formatting_and_fields_stay() {
    let mut f = fx();
    let s = f.rect([1.0, 1.0], [1.0, 1.0], "", false);
    let t = Text { items: vec![TextItem::Cp(0), TextItem::Chars("Kopf".into()), TextItem::Cp(1), TextItem::Chars("Rest\n".into())], extras: Default::default() };
    f.edit(|tx, eng| ops::set_text(tx, eng, s, t.clone()));
    f.edit(|tx, eng| number::set_number(tx, eng, s, "1", Place::Before));
    f.edit(|tx, eng| number::set_number(tx, eng, s, "2", Place::After));
    let got = f.doc.shape(s).unwrap().text().unwrap().items.clone();
    assert_eq!(got, vec![TextItem::Cp(0), TextItem::Chars("Kopf".into()), TextItem::Cp(1), TextItem::Chars("Rest 2\n".into())]);
    f.edit(|tx, eng| number::set_number(tx, eng, s, "3", Place::Before));
    let got = f.doc.shape(s).unwrap().text().unwrap().items.clone();
    assert_eq!(got, vec![TextItem::Cp(0), TextItem::Chars("3 Kopf".into()), TextItem::Cp(1), TextItem::Chars("Rest\n".into())]);
}

#[test]
fn a_locked_text_takes_no_number() {
    let mut f = fx();
    let a = f.rect([1.0, 1.0], [1.0, 1.0], "A", false);
    let b = f.rect([3.0, 1.0], [1.0, 1.0], "B", false);
    let c = f.rect([5.0, 1.0], [1.0, 1.0], "C", false);
    f.edit(|tx, _| {
        let l = tx.single("LockTextEdit");
        tx.set_constant(SheetRef::Shape(b), &l, Value::Num(1.0), Some(Unit::Bool))
    });
    let next = f.edit(|tx, eng| number::number(tx, eng, &[a, b, c], &Numbering::default()));
    assert_eq!((f.text(a).as_str(), f.text(b).as_str(), f.text(c).as_str(), next), ("1 A", "B", "2 C", 3));
}

#[test]
fn numbering_on_drop_counts_on() {
    let mut f = fx();
    let page = f.page;
    assert!(number::on_drop(&f.doc, page).is_none());
    let n = Numbering { start: 4, step: 2, prefix: String::from("#"), place: Place::After };
    f.edit(|tx, _| number::keep_on_drop(tx, page, Some(&n)));
    assert_eq!(number::on_drop(&f.doc, page), Some(n.clone()));
    let a = f.rect([1.0, 1.0], [1.0, 1.0], "Server", false);
    let b = f.rect([3.0, 1.0], [1.0, 1.0], "", false);
    assert!(f.edit(|tx, eng| number::dropped(tx, eng, a)));
    assert!(f.edit(|tx, eng| number::dropped(tx, eng, b)));
    assert_eq!((f.text(a).as_str(), f.text(b).as_str()), ("Server #4", "#6"));
    assert_eq!(number::on_drop(&f.doc, page).map(|n| n.start), Some(8));
    // off: nothing more
    f.edit(|tx, _| number::keep_on_drop(tx, page, None));
    assert!(number::on_drop(&f.doc, page).is_none());
    let c = f.rect([5.0, 1.0], [1.0, 1.0], "X", false);
    assert!(!f.edit(|tx, eng| number::dropped(tx, eng, c)));
    assert_eq!(f.text(c), "X");
}

// ---- 2. array

#[test]
fn array_offsets() {
    let g = Grid { rows: 2, cols: 3, gap: [0.5, 0.25], centres: false, left: false, up: false };
    let o = array::offsets([1.0, 0.5], &g);
    let want = [(0, 1, [1.5, 0.0]), (0, 2, [3.0, 0.0]), (1, 0, [0.0, -0.75]), (1, 1, [1.5, -0.75]), (1, 2, [3.0, -0.75])];
    assert_eq!(o.len(), want.len());
    for (got, w) in o.iter().zip(want) {
        assert!(got.0 == w.0 && got.1 == w.1 && near(got.2, w.2), "{got:?} != {w:?}");
    }
    // middle to middle, to the left and up
    let g = Grid { rows: 2, cols: 2, gap: [2.0, 3.0], centres: true, left: true, up: true };
    let o: Vec<[f64; 2]> = array::offsets([1.0, 0.5], &g).into_iter().map(|x| x.2).collect();
    assert!(near(o[0], [-2.0, 0.0]) && near(o[1], [0.0, 3.0]) && near(o[2], [-2.0, 3.0]), "{o:?}");
}

#[test]
fn array_makes_copies_in_their_cells() {
    let mut f = fx();
    let a = f.rect([1.0, 5.0], [1.0, 0.5], "A", true);
    let b = f.rect([2.5, 5.0], [1.0, 0.5], "B", true);
    let c = f.connect(a, b);
    let owner = f.owner;
    let g = Grid { rows: 2, cols: 3, gap: [0.5, 0.5], centres: false, left: false, up: false };
    let made = f.edit(|tx, eng| array::make(tx, eng, owner, &[a, b, c], &g));
    // five cells of three shapes each
    assert_eq!(made.len(), 15);
    // the box of a and b is 2.5 wide and 0.5 high: columns 3.0 apart, rows 1.0
    let box_of = |f: &Fx, k: usize| ops::bounds(&f.doc, &made[k * 3..k * 3 + 3]).unwrap();
    let b0 = ops::bounds(&f.doc, &[a, b]).unwrap();
    for (k, (r, col)) in [(0, 1), (0, 2), (1, 0), (1, 1), (1, 2)].into_iter().enumerate() {
        let bx = box_of(&f, k);
        assert!(close(bx[0], b0[0] + col as f64 * 3.0) && close(bx[3], b0[3] - r as f64 * 1.0), "cell {r} {col}: {bx:?}");
    }
    // the copied connector joins the copies
    let copy_c = made[2];
    let [ta, tb] = glue::glued(&f.doc, copy_c);
    assert_eq!((ta, tb), (Target::Shape(made[0]), Target::Shape(made[1])));
    // refused
    let mut tx = f.doc.transaction("x");
    assert_eq!(array::make(&mut tx, &mut f.eng, owner, &[a], &Grid { rows: 0, ..g }), Err(EditError::Invalid));
    assert_eq!(array::make(&mut tx, &mut f.eng, owner, &[a], &Grid { rows: 200, cols: 200, ..g }), Err(EditError::Invalid));
}

// ---- 3. dimension

#[test]
fn dimension_between_two_shapes_glues_to_their_facing_points() {
    let mut f = fx();
    let a = f.rect([2.0, 2.0], [1.0, 1.0], "A", true);
    let b = f.rect([6.0, 3.0], [2.0, 1.0], "B", true);
    // horizontal: a's right side, b's left side
    let (p, q) = dimension::between(&f.doc, b, a, Kind::Horizontal);
    assert_eq!((p, q), (Anchor::Point { shape: a, row: 1 }, Anchor::Point { shape: b, row: 3 }));
    // vertical: the lower one's top, the upper one's bottom
    let (p, q) = dimension::between(&f.doc, a, b, Kind::Vertical);
    assert_eq!((p, q), (Anchor::Point { shape: a, row: 2 }, Anchor::Point { shape: b, row: 0 }));
    // aligned: the nearest pair
    let (p, q) = dimension::between(&f.doc, a, b, Kind::Aligned);
    assert_eq!((p, q), (Anchor::Point { shape: a, row: 1 }, Anchor::Point { shape: b, row: 3 }));
    // one shape: its width
    let (p, q) = dimension::of_shape(&f.doc, b, Kind::Horizontal);
    assert_eq!((p, q), (Anchor::Point { shape: b, row: 3 }, Anchor::Point { shape: b, row: 1 }));
    // placed on a line: glued, and it follows b
    let owner = f.owner;
    let (pa, pb) = (dimension::point(&f.doc, &p), dimension::point(&f.doc, &q));
    let d = f.edit(|tx, eng| glue::new_line(tx, eng, owner, (pa, Target::None), (pb, Target::None)));
    let (p, q) = dimension::between(&f.doc, a, b, Kind::Horizontal);
    f.edit(|tx, eng| dimension::place(tx, eng, d, Kind::Horizontal, &p, &q, 0.25));
    assert_eq!(glue::glued(&f.doc, d), [Target::Point { shape: a, row: 1 }, Target::Point { shape: b, row: 3 }]);
    let (bx, ex) = dimension::end_formulas(&f.doc, d);
    assert_eq!(bx.as_deref(), Some("PAR(PNT(Sheet.1!Connections.X2,Sheet.1!Connections.Y2))"));
    assert_eq!(ex.as_deref(), Some("PAR(PNT(Sheet.2!Connections.X4,Sheet.2!Connections.Y4))"));
    assert!(near(f.ends(d).1, [5.0, 3.0]));
    f.move_by(b, [1.0, -0.5]);
    assert!(near(f.ends(d).0, [2.5, 2.0]) && near(f.ends(d).1, [6.0, 2.5]), "{:?}", f.ends(d));
}

#[test]
fn dimension_ties_to_the_box_of_a_shape_without_points() {
    let mut f = fx();
    let a = f.rect([2.0, 2.0], [2.0, 1.0], "A", false);
    let b = f.rect([2.0, 6.0], [1.0, 1.0], "B", false);
    let (p, q) = dimension::of_shape(&f.doc, a, Kind::Vertical);
    assert_eq!((p, q), (Anchor::Box { shape: a, at: [0.5, 0.0] }, Anchor::Box { shape: a, at: [0.5, 1.0] }));
    let (p, q) = dimension::between(&f.doc, b, a, Kind::Vertical);
    assert_eq!((p, q), (Anchor::Box { shape: a, at: [0.5, 1.0] }, Anchor::Box { shape: b, at: [0.5, 0.0] }));
    let owner = f.owner;
    let d = f.edit(|tx, eng| glue::new_line(tx, eng, owner, ([0.0, 0.0], Target::None), ([1.0, 1.0], Target::None)));
    f.edit(|tx, eng| dimension::place(tx, eng, d, Kind::Vertical, &p, &q, 0.25));
    let (bx, ex) = dimension::end_formulas(&f.doc, d);
    assert_eq!(bx.as_deref(), Some("PAR(PNT(Sheet.1!Width*0.5,Sheet.1!Height*1))"));
    assert_eq!(ex.as_deref(), Some("PAR(PNT(Sheet.2!Width*0.5,Sheet.2!Height*0))"));
    assert!(near(f.ends(d).0, [2.0, 2.5]) && near(f.ends(d).1, [2.0, 5.5]), "{:?}", f.ends(d));
    // no Connect entry for a box point; the ends still follow both shapes
    assert_eq!(glue::glued(&f.doc, d), [Target::None, Target::None]);
    f.move_by(b, [0.5, 1.0]);
    f.move_by(a, [-1.0, 0.0]);
    assert!(near(f.ends(d).0, [1.0, 2.5]) && near(f.ends(d).1, [2.5, 6.5]), "{:?}", f.ends(d));
    // a free end stays where it was put
    f.edit(|tx, eng| dimension::attach(tx, eng, d, End::End, &Anchor::Free([9.0, 9.0])));
    f.move_by(b, [1.0, 1.0]);
    assert!(near(f.ends(d).1, [9.0, 9.0]));
}
