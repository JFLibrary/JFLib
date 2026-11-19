// rebuild_tests -- render::rebuild against a whole build after edits: the same items, tops,
// notes, fonts and grid answers; refused when shapes come, go or change their order; and
// measure_text
//
//   cargo test -p jflib-diagram-core --lib rebuild

use alloc::vec::Vec;

use crate::hit::tests::{add, add_background, add_kind, add_rect, draw, new_doc, poly, rect_pts};
use crate::render::{measure_text, rebuild, ApproxMeasure, Drawing, Face, Options};
use crate::*;

fn set(doc: &mut Document, shape: ShapeRef, name: &str, v: f64) {
    let mut tx = doc.transaction("set");
    let addr = tx.single(name);
    tx.set_constant(SheetRef::Shape(shape), &addr, Value::Num(v), None).unwrap();
    tx.commit();
}

fn redo(doc: &Document, d: &mut Drawing, shapes: &[ShapeRef]) -> bool {
    rebuild(doc, d, &mut ApproxMeasure, &Options::default(), shapes)
}

/// d as rebuilt equals a whole build of the document now
fn same(doc: &Document, page: PageRef, d: &Drawing) {
    let full = draw(doc, page);
    assert_eq!(d.items.len(), full.items.len());
    for (k, (a, b)) in d.items.iter().zip(&full.items).enumerate() {
        assert_eq!(a, b, "item {k}");
    }
    assert_eq!(d.tops, full.tops);
    assert_eq!(d.notes, full.notes);
    assert_eq!(d.fonts, full.fonts);
    // the grid gives the same answers over the page and around it
    let (mut x, mut a, mut b) = (-1.0, Vec::new(), Vec::new());
    while x < 10.0 {
        let mut y = -1.0;
        while y < 12.0 {
            for r in [[x, y, x + 0.3, y + 0.3], [x, y, x + 2.5, y + 1.7]] {
                d.query(r, &mut a);
                full.query(r, &mut b);
                assert_eq!(a, b, "query {r:?}");
            }
            y += 0.45;
        }
        x += 0.45;
    }
}

fn page_of_shapes() -> (Document, PageRef, Vec<ShapeRef>) {
    let (mut doc, page) = new_doc();
    let mut out = Vec::new();
    for i in 0..6u32 {
        let x = 1.0 + (i % 3) as f64 * 2.5;
        let y = 2.0 + (i / 3) as f64 * 3.0;
        let text = if i % 2 == 0 { Some("Kasten mit Text") } else { None };
        out.push(add_rect(&mut doc, page, i + 1, [x, y], [1.5, 1.0], &[], text));
    }
    (doc, page, out)
}

#[test]
fn moved_shape_same_count() {
    let (mut doc, page, s) = page_of_shapes();
    let mut d = draw(&doc, page);
    set(&mut doc, s[1], "PinX", 6.3);
    set(&mut doc, s[1], "PinY", 9.0);
    assert!(redo(&doc, &mut d, &[s[1]]));
    same(&doc, page, &d);
    // far off the page: the grid still finds it
    set(&mut doc, s[4], "PinX", 30.0);
    assert!(redo(&doc, &mut d, &[s[4]]));
    same(&doc, page, &d);
    let mut out = Vec::new();
    d.query([29.0, 4.0, 31.0, 6.0], &mut out);
    assert!(out.iter().all(|&i| d.items[i as usize].top == s[4]) && !out.is_empty());
}

#[test]
fn items_come_and_go() {
    let (mut doc, page, s) = page_of_shapes();
    let mut d = draw(&doc, page);
    // a shadow adds an item, no line removes one, a text adds one, wrapping adds runs
    set(&mut doc, s[0], "ShdwPattern", 1.0);
    set(&mut doc, s[2], "LinePattern", 0.0);
    {
        let mut tx = doc.transaction("text");
        tx.set_text(s[3], Some(Text::plain("neu"))).unwrap();
        tx.commit();
    }
    set(&mut doc, s[4], "Width", 0.4);
    assert!(redo(&doc, &mut d, &[s[2], s[0], s[3], s[4], s[0]]));
    same(&doc, page, &d);
    // and back
    set(&mut doc, s[0], "ShdwPattern", 0.0);
    set(&mut doc, s[2], "LinePattern", 1.0);
    assert!(redo(&doc, &mut d, &[s[0], s[2]]));
    same(&doc, page, &d);
    // notes follow: a compound line is noted
    set(&mut doc, s[5], "CompoundType", 1.0);
    assert!(redo(&doc, &mut d, &[s[5]]));
    assert_eq!(d.notes, alloc::vec![(render::Note::CompoundLine, 1)]);
    same(&doc, page, &d);
}

#[test]
fn subshapes_and_background_pages() {
    let (mut doc, page) = new_doc();
    let bg = add_background(&mut doc, page);
    let under = add_rect(&mut doc, bg, 1, [4.0, 1.0], [7.0, 0.5], &[], Some("Fuss"));
    let g = add_kind(&mut doc, page, None, 1, ShapeKind::Group, [3.0, 3.0], [4.0, 4.0], Vec::new(), &[], None);
    let pa = poly(&mut doc, 0, &rect_pts(2.0, 2.0));
    let child = add(&mut doc, page, Some(g), 2, [1.5, 1.5], [2.0, 2.0], alloc::vec![pa], &[], Some("innen"));
    let last = add_rect(&mut doc, page, 3, [6.0, 6.0], [1.0, 1.0], &[], None);
    let mut d = draw(&doc, page);
    set(&mut doc, child, "Width", 3.0);
    set(&mut doc, under, "PinY", 1.5);
    set(&mut doc, under, "LinePattern", 0.0);
    assert!(redo(&doc, &mut d, &[child, under]));
    same(&doc, page, &d);
    // nothing named: nothing to do
    assert!(redo(&doc, &mut d, &[]));
    // a shape of another page is passed over
    let (mut other, p2) = new_doc();
    let foreign = add_rect(&mut other, p2, 1, [1.0, 1.0], [1.0, 1.0], &[], None);
    let _ = foreign;
    assert!(redo(&doc, &mut d, &[last]));
    same(&doc, page, &d);
}

#[test]
fn refused_when_shapes_change_order() {
    let (mut doc, page, s) = page_of_shapes();
    let d0 = draw(&doc, page);
    // a new shape
    {
        let mut tx = doc.transaction("add");
        tx.add_shape(Owner::Page(page), None, ShapeKind::Shape).unwrap();
        let ch = tx.commit();
        let mut d = d0.clone();
        assert!(!redo(&doc, &mut d, &[s[0]]));
        assert_eq!(d.items, d0.items, "refused: nothing changed");
        let mut ch = ch.unwrap();
        assert!(ch.undo(&mut doc));
    }
    // back as built: fine again
    let mut d = d0.clone();
    assert!(redo(&doc, &mut d, &[s[0]]));
    // the order
    {
        let mut tx = doc.transaction("front");
        tx.bring_to_front(s[0]).unwrap();
        let mut ch = tx.commit().unwrap();
        let mut d = d0.clone();
        assert!(!redo(&doc, &mut d, &[s[0]]));
        assert!(ch.undo(&mut doc));
    }
    // a deleted shape
    {
        let mut tx = doc.transaction("delete");
        tx.delete_shape(s[2]).unwrap();
        tx.commit();
        let mut d = d0.clone();
        assert!(!redo(&doc, &mut d, &[s[1]]));
    }
    // the page's size
    let (mut doc, page, s) = page_of_shapes();
    let mut d = draw(&doc, page);
    {
        let mut tx = doc.transaction("size");
        let a = tx.single("PageWidth");
        tx.set_constant(SheetRef::Page(page), &a, Value::Num(11.0), None).unwrap();
        tx.commit();
    }
    assert!(!redo(&doc, &mut d, &[s[0]]));
}

#[test]
fn text_measured_as_laid_out() {
    let f = Face { family: "Calibri", bold: false, italic: false };
    let m = &mut ApproxMeasure;
    let close = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12;
    // half an em a character, a quarter for a space, at a size of 1: "ab cd" = 1 + 0.25 + 1
    let r = measure_text(m, &f, 1.0, "ab cd", None);
    assert!(close(r, (2.25, 1.2)), "{r:?}");
    // wrapped at 1.5: two lines, the space hangs
    let r = measure_text(m, &f, 1.0, "ab cd", Some(1.5));
    assert!(close(r, (1.0, 2.4)), "{r:?}");
    // the end mark makes no line, a line break and a paragraph do
    let r = measure_text(m, &f, 1.0, "ab\n", None);
    assert!(close(r, (1.0, 1.2)), "{r:?}");
    let r = measure_text(m, &f, 1.0, "ab\u{2028}c\nd", None);
    assert!(close(r, (1.0, 3.6)), "{r:?}");
    // empty: one line
    let r = measure_text(m, &f, 0.5, "", None);
    assert!(close(r, (0.0, 0.6)), "{r:?}");
    // a word wider than the line breaks at characters
    let r = measure_text(m, &f, 1.0, "abcdef", Some(1.2));
    assert!(close(r, (1.0, 3.6)), "{r:?}");
}
