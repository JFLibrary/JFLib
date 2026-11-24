// jumps_tests -- (JD21) line jumps: crossings found (straight, slanted, along each other, at
// bends, a T), which connector jumps by LineJumpCode and ConLineJumpCode, each style's shape and
// size, the directions, close jumps merged, plain lines and hidden connectors left out, a
// rebuild that follows a moved connector, a large page
//
//   cargo test -p jflib-diagram-core --lib jumps

use alloc::vec;
use alloc::vec::Vec;

use super::{cross, JumpStyle};
use crate::hit::tests::{add, draw, new_doc, poly};
use crate::render::{affected, rebuild, ApproxMeasure, Drawing, ItemKind, Options, Role};
use crate::*;

const W: f64 = 0.125 * 2.0 / 3.0;

fn set_cell(doc: &mut Document, sheet: SheetRef, name: &str, v: f64) {
    let mut tx = doc.transaction("set");
    let addr = tx.single(name);
    tx.set_constant(sheet, &addr, Value::Num(v), None).unwrap();
    tx.commit();
}

fn page_cell(doc: &mut Document, page: PageRef, name: &str, v: f64) {
    set_cell(doc, SheetRef::Page(page), name, v);
}

/// a dynamic connector through `pts` (page inches: its local pin at the page's origin)
fn conn(doc: &mut Document, page: PageRef, id: u32, pts: &[[f64; 2]], cells: &[(&str, f64)]) -> ShapeRef {
    let g = poly(doc, 0, pts);
    let mut all: Vec<(&str, f64)> = vec![("BeginX", pts[0][0]), ("BeginY", pts[0][1]), ("ObjType", 2.0)];
    all.extend_from_slice(cells);
    add(doc, page, None, id, [0.0, 0.0], [0.0, 0.0], vec![g], &all, None)
}

/// the jumps a connector got
fn jumps_of(d: &Drawing, s: ShapeRef) -> Vec<crate::render::Jump> {
    d.jumps().find(|(c, _)| *c == s).map(|(_, j)| j.to_vec()).unwrap_or_default()
}

/// the line item of a connector
fn line_of(d: &Drawing, s: ShapeRef) -> crate::geometry::Path {
    d.items
        .iter()
        .find_map(|it| match &it.kind {
            ItemKind::Stroke { path, .. } if it.shape == s && it.role == Role::Shape => Some(path.clone()),
            _ => None,
        })
        .unwrap()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// a horizontal and a vertical connector crossing at (4, 5)
fn plus() -> (Document, PageRef, ShapeRef, ShapeRef) {
    let (mut doc, page) = new_doc();
    let h = conn(&mut doc, page, 1, &[[2.0, 5.0], [6.0, 5.0]], &[]);
    let v = conn(&mut doc, page, 2, &[[4.0, 3.0], [4.0, 7.0]], &[]);
    (doc, page, h, v)
}

#[test]
fn crossings() {
    // straight across: the parameter on the first piece
    assert!(near(cross([0.0, 0.0], [4.0, 0.0], [1.0, -1.0], [1.0, 1.0]).unwrap(), 0.25));
    // slanted
    let t = cross([0.0, 0.0], [2.0, 2.0], [0.0, 2.0], [2.0, 0.0]).unwrap();
    assert!(near(t, 0.5));
    // parallel and along each other: no crossing
    assert!(cross([0.0, 0.0], [4.0, 0.0], [0.0, 1.0], [4.0, 1.0]).is_none());
    assert!(cross([0.0, 0.0], [4.0, 0.0], [1.0, 0.0], [3.0, 0.0]).is_none());
    // an end on the other line (a T) and ends meeting: no crossing
    assert!(cross([0.0, 0.0], [4.0, 0.0], [2.0, 0.0], [2.0, 3.0]).is_none());
    assert!(cross([0.0, 0.0], [4.0, 0.0], [4.0, 0.0], [4.0, 3.0]).is_none());
    // short of the line: none
    assert!(cross([0.0, 0.0], [4.0, 0.0], [2.0, 0.5], [2.0, 3.0]).is_none());
}

#[test]
fn horizontal_line_jumps_by_default() {
    let (doc, page, h, v) = plus();
    let d = draw(&doc, page);
    let j = jumps_of(&d, h);
    assert_eq!(j.len(), 1);
    assert!(jumps_of(&d, v).is_empty());
    let j = j[0];
    assert_eq!(j.style, JumpStyle::Arc);
    // 2/3 of 0.125 in wide, centred on the crossing, half as high, upwards
    assert!(near(j.to - j.from, W));
    assert!(near((j.start[0] + j.end[0]) * 0.5, 4.0) && near(j.start[1], 5.0));
    assert!(near(j.height, W * 0.5));
    assert!(near(j.normal[1], 1.0));
    // the drawn line: to the jump, two quarter curves, on to the end
    let p = line_of(&d, h);
    assert_eq!(p.subs.len(), 1);
    let segs = &p.subs[0].segs;
    assert_eq!(segs.len(), 4);
    assert!(matches!(segs[1], crate::geometry::Seg::Cubic(_, _, top) if near(top[0], 4.0) && near(top[1], 5.0 + W * 0.5)));
}

#[test]
fn page_codes() {
    let (mut doc, page, h, v) = plus();
    for (code, h_jumps, v_jumps) in [(0.0, 0, 0), (1.0, 1, 0), (2.0, 0, 1), (3.0, 0, 1), (4.0, 0, 1), (5.0, 1, 0)] {
        page_cell(&mut doc, page, "LineJumpCode", code);
        let d = draw(&doc, page);
        assert_eq!(jumps_of(&d, h).len(), h_jumps, "code {code}");
        assert_eq!(jumps_of(&d, v).len(), v_jumps, "code {code}");
    }
}

#[test]
fn slanted_pieces_by_their_direction() {
    let (mut doc, page) = new_doc();
    // a flat slant and a steep one
    let flat = conn(&mut doc, page, 1, &[[1.0, 4.0], [7.0, 6.0]], &[]);
    let steep = conn(&mut doc, page, 2, &[[3.0, 1.0], [5.0, 9.0]], &[]);
    let d = draw(&doc, page);
    assert_eq!(jumps_of(&d, flat).len(), 1);
    assert!(jumps_of(&d, steep).is_empty());
    page_cell(&mut doc, page, "LineJumpCode", 2.0);
    let d = draw(&doc, page);
    assert!(jumps_of(&d, flat).is_empty());
    assert_eq!(jumps_of(&d, steep).len(), 1);
    // equal slants (an X): the upper one in the z-order
    let (mut doc, page) = new_doc();
    let a = conn(&mut doc, page, 1, &[[1.0, 1.0], [5.0, 5.0]], &[]);
    let b = conn(&mut doc, page, 2, &[[1.0, 5.0], [5.0, 1.0]], &[]);
    let d = draw(&doc, page);
    assert!(jumps_of(&d, a).is_empty());
    assert_eq!(jumps_of(&d, b).len(), 1);
}

#[test]
fn connector_codes() {
    // (code of h, code of v, page code) -> (h jumps, v jumps)
    let cases: [(f64, f64, f64, usize, usize); 10] = [
        (1.0, 0.0, 1.0, 0, 0), // h never: the page picks h, nothing
        (0.0, 1.0, 1.0, 1, 0), // v never: h still jumps
        (0.0, 2.0, 1.0, 0, 1), // v always
        (0.0, 2.0, 0.0, 0, 1), // always on a page without jumps
        (2.0, 2.0, 2.0, 0, 1), // both always: the page's rule (vertical)
        (3.0, 0.0, 2.0, 0, 1), // h: the other jumps
        (0.0, 3.0, 1.0, 1, 0), // v: the other (h) jumps
        (4.0, 2.0, 1.0, 0, 0), // h: neither
        (1.0, 3.0, 1.0, 0, 0), // v asks h, h never
        (3.0, 3.0, 4.0, 0, 1), // each asks the other: the page's rule (upper: v)
    ];
    for (ch, cv, pc, hj, vj) in cases {
        let (mut doc, page, h, v) = plus();
        page_cell(&mut doc, page, "LineJumpCode", pc);
        set_cell(&mut doc, SheetRef::Shape(h), "ConLineJumpCode", ch);
        set_cell(&mut doc, SheetRef::Shape(v), "ConLineJumpCode", cv);
        let d = draw(&doc, page);
        assert_eq!((jumps_of(&d, h).len(), jumps_of(&d, v).len()), (hj, vj), "h {ch} v {cv} page {pc}");
    }
}

#[test]
fn last_routed_is_the_newer_id() {
    let (mut doc, page) = new_doc();
    // the vertical one comes first in the z-order but has the higher ID
    let v = conn(&mut doc, page, 9, &[[4.0, 3.0], [4.0, 7.0]], &[]);
    let h = conn(&mut doc, page, 3, &[[2.0, 5.0], [6.0, 5.0]], &[]);
    page_cell(&mut doc, page, "LineJumpCode", 3.0);
    let d = draw(&doc, page);
    assert_eq!(jumps_of(&d, v).len(), 1);
    assert!(jumps_of(&d, h).is_empty());
    // last displayed: the top one (h)
    page_cell(&mut doc, page, "LineJumpCode", 4.0);
    let d = draw(&doc, page);
    assert_eq!(jumps_of(&d, h).len(), 1);
}

#[test]
fn styles_shapes_and_sizes() {
    use crate::geometry::Seg;
    for code in 0..=9u32 {
        let (mut doc, page, h, _) = plus();
        page_cell(&mut doc, page, "LineJumpStyle", code as f64);
        page_cell(&mut doc, page, "LineJumpFactorX", 0.5);
        page_cell(&mut doc, page, "LineToLineX", 0.4);
        let d = draw(&doc, page);
        let j = jumps_of(&d, h)[0];
        assert_eq!(j.style, JumpStyle::from_code(code));
        // 0.5 x 0.4 = 0.2 in wide, 0.1 high
        assert!(near(j.to - j.from, 0.2) && near(j.height, 0.1));
        let p = line_of(&d, h);
        let top = |s: &Seg| s.end()[1];
        match j.style {
            JumpStyle::Gap => {
                assert_eq!(p.subs.len(), 2);
                assert!(near(p.subs[0].end()[0], 3.9) && near(p.subs[1].start[0], 4.1));
            }
            JumpStyle::Arc => {
                assert_eq!(p.subs[0].segs.len(), 4);
                assert!(near(top(&p.subs[0].segs[1]), 5.1));
            }
            JumpStyle::Square => {
                let s = &p.subs[0].segs;
                assert_eq!(s.len(), 5);
                assert!(near(s[1].end()[0], 3.9) && near(top(&s[1]), 5.1) && near(s[2].end()[0], 4.1) && near(top(&s[2]), 5.1));
            }
            JumpStyle::Sides(n) => {
                assert_eq!(n as u32, code - 2);
                let s = &p.subs[0].segs;
                // to the jump, n sides, on to the end
                assert_eq!(s.len(), n as usize + 2);
                let peak = s.iter().map(top).fold(f64::MIN, f64::max);
                // the corners on the half circle: the highest one at the top for an even n
                if n % 2 == 0 {
                    assert!(near(peak, 5.1));
                } else {
                    assert!(peak < 5.1 && peak > 5.05);
                }
                assert!(s.iter().all(|g| matches!(g, Seg::Line(_))));
            }
        }
    }
}

#[test]
fn connector_style_and_directions() {
    let (mut doc, page, h, v) = plus();
    // h jumps down with a gap of its own; then v (vertical lines) to the right
    set_cell(&mut doc, SheetRef::Shape(h), "ConLineJumpStyle", 2.0);
    set_cell(&mut doc, SheetRef::Shape(h), "ConLineJumpDirX", 2.0);
    let d = draw(&doc, page);
    let j = jumps_of(&d, h)[0];
    assert_eq!(j.style, JumpStyle::Gap);
    assert!(near(j.normal[1], -1.0));
    page_cell(&mut doc, page, "LineJumpCode", 2.0);
    let d = draw(&doc, page);
    assert!(near(jumps_of(&d, v)[0].normal[0], -1.0));
    page_cell(&mut doc, page, "PageLineJumpDirY", 2.0);
    let d = draw(&doc, page);
    assert!(near(jumps_of(&d, v)[0].normal[0], 1.0));
    // the connector's own direction wins over the page's
    set_cell(&mut doc, SheetRef::Shape(v), "ConLineJumpDirY", 1.0);
    let d = draw(&doc, page);
    assert!(near(jumps_of(&d, v)[0].normal[0], -1.0));
    // the vertical size
    page_cell(&mut doc, page, "LineJumpFactorY", 1.0);
    page_cell(&mut doc, page, "LineToLineY", 0.3);
    let d = draw(&doc, page);
    let j = jumps_of(&d, v)[0];
    assert!(near(j.to - j.from, 0.3));
}

#[test]
fn close_jumps_merge_far_ones_do_not() {
    let (mut doc, page) = new_doc();
    let h = conn(&mut doc, page, 1, &[[1.0, 5.0], [8.0, 5.0]], &[]);
    // two verticals 0.05 apart (closer than the width), one far off
    conn(&mut doc, page, 2, &[[3.0, 3.0], [3.0, 7.0]], &[]);
    conn(&mut doc, page, 3, &[[3.05, 3.0], [3.05, 7.0]], &[]);
    conn(&mut doc, page, 4, &[[6.0, 3.0], [6.0, 7.0]], &[]);
    let d = draw(&doc, page);
    let j = jumps_of(&d, h);
    assert_eq!(j.len(), 2);
    assert!(near(j[0].from, 2.0 - W * 0.5) && near(j[0].to, 2.05 + W * 0.5));
    assert!(near(j[0].height, W * 0.5));
    assert!(near(j[1].to - j[1].from, W));
}

#[test]
fn bends_ends_and_plain_lines() {
    let (mut doc, page) = new_doc();
    // a right-angle connector bending at (4, 5); a vertical crossing right next to the bend and
    // one crossing the first piece in its middle
    let bent = conn(&mut doc, page, 1, &[[1.0, 5.0], [4.0, 5.0], [4.0, 8.0]], &[]);
    conn(&mut doc, page, 2, &[[3.99, 3.0], [3.99, 7.0]], &[]);
    conn(&mut doc, page, 3, &[[2.0, 3.0], [2.0, 7.0]], &[]);
    // a connector whose end lies on the line (a T), one ending just short of it
    conn(&mut doc, page, 4, &[[3.0, 3.0], [3.0, 5.0]], &[]);
    conn(&mut doc, page, 5, &[[3.5, 3.0], [3.5, 4.9]], &[]);
    // a plain line (no routable flag) across: neither jumps nor is jumped over
    let g = poly(&mut doc, 0, &[[1.5, 3.0], [1.5, 7.0]]);
    add(&mut doc, page, None, 6, [0.0, 0.0], [0.0, 0.0], vec![g], &[("BeginX", 1.5), ("BeginY", 3.0)], None);
    let d = draw(&doc, page);
    let j = jumps_of(&d, bent);
    assert_eq!(j.len(), 1);
    assert!(near((j[0].from + j[0].to) * 0.5, 1.0));
    // a connector with rounded corners keeps its jumps on the straight parts
    set_cell(&mut doc, SheetRef::Shape(bent), "Rounding", 0.2);
    let d = draw(&doc, page);
    assert_eq!(jumps_of(&d, bent).len(), 1);
}

#[test]
fn hidden_and_grouped_connectors() {
    let (mut doc, page, h, v) = plus();
    // a vertical connector without a line takes no part
    set_cell(&mut doc, SheetRef::Shape(v), "LinePattern", 0.0);
    let d = draw(&doc, page);
    assert!(jumps_of(&d, h).is_empty());
    // a connector in a group (the group's local coordinates: moved by (1, 0))
    let (mut doc, page, h, _) = plus();
    let grp = add(&mut doc, page, None, 10, [1.0, 0.0], [0.0, 0.0], vec![], &[], None);
    let g = poly(&mut doc, 0, &[[4.5, 3.0], [4.5, 7.0]]);
    add(&mut doc, page, Some(grp), 11, [0.0, 0.0], [0.0, 0.0], vec![g], &[("BeginX", 4.5), ("BeginY", 3.0), ("ObjType", 2.0)], None);
    let d = draw(&doc, page);
    let j = jumps_of(&d, h);
    assert_eq!(j.len(), 2);
    assert!(near((j[1].from + j[1].to) * 0.5, 3.5));
    // a connector on a hidden layer is not drawn and nobody jumps over it
    let (mut doc, page, h, v) = plus();
    let mut tx = doc.transaction("Ebene");
    let hidden = tx.add_layer(page, "Verborgen", "Hidden").unwrap();
    let visible = tx.single("Visible");
    let sec = SecKey::one(tx.intern("Layer"));
    tx.set_constant(SheetRef::Page(page), &CellAddr::row(sec, RowKey::Index(hidden), visible.cell), Value::Num(0.0), None).unwrap();
    tx.set_layers(v, &[hidden]).unwrap();
    tx.commit().unwrap();
    let d = draw(&doc, page);
    assert!(jumps_of(&d, h).is_empty());
    // shown again with Options::hidden_layers: the jump is back
    let d = crate::render::build(&doc, page, &mut ApproxMeasure, &Options { hidden_layers: true, ..Options::default() });
    assert_eq!(jumps_of(&d, h).len(), 1);
}

fn redo(doc: &Document, d: &mut Drawing, shapes: &[ShapeRef]) -> bool {
    rebuild(doc, d, &mut ApproxMeasure, &Options::default(), shapes)
}

fn same(doc: &Document, page: PageRef, d: &Drawing) {
    let full = draw(doc, page);
    assert_eq!(d.items, full.items);
    assert_eq!(d.tops, full.tops);
    let a: Vec<_> = d.jumps().map(|(s, j)| (s, j.to_vec())).collect();
    let b: Vec<_> = full.jumps().map(|(s, j)| (s, j.to_vec())).collect();
    assert_eq!(a, b);
}

#[test]
fn rebuild_follows_a_moved_connector() {
    let (mut doc, page) = new_doc();
    let h1 = conn(&mut doc, page, 1, &[[1.0, 5.0], [8.0, 5.0]], &[]);
    let h2 = conn(&mut doc, page, 2, &[[1.0, 8.0], [8.0, 8.0]], &[]);
    let far = conn(&mut doc, page, 3, &[[1.0, 1.0], [8.0, 1.0]], &[]);
    let v = conn(&mut doc, page, 4, &[[4.0, 3.0], [4.0, 6.0]], &[]);
    let mut d = draw(&doc, page);
    assert_eq!(jumps_of(&d, h1).len(), 1);
    assert!(jumps_of(&d, h2).is_empty());
    // v grows up across h2: h1 (crossed before and after) and h2 (now) are made anew, far not
    {
        let mut tx = doc.transaction("geometry");
        let (geo, y) = (tx.intern("Geometry"), tx.intern("Y"));
        let a = CellAddr::row(SecKey { name: geo, index: 0 }, RowKey::Index(2), y);
        tx.set_constant(SheetRef::Shape(v), &a, Value::Num(9.0), None).unwrap();
        tx.commit();
    }
    let tops = affected(&doc, &d, &Options::default(), &[v]);
    assert!(tops.contains(&v) && tops.contains(&h1) && tops.contains(&h2) && !tops.contains(&far));
    assert!(redo(&doc, &mut d, &[v]));
    same(&doc, page, &d);
    assert_eq!(jumps_of(&d, h2).len(), 1);
    // v moves away from both: their jumps go
    set_cell(&mut doc, SheetRef::Shape(v), "PinX", 20.0);
    let tops = affected(&doc, &d, &Options::default(), &[v]);
    assert!(tops.contains(&h1) && tops.contains(&h2));
    assert!(redo(&doc, &mut d, &[v]));
    same(&doc, page, &d);
    assert!(jumps_of(&d, h1).is_empty() && jumps_of(&d, h2).is_empty());
    // a shape that is no connector changes nothing else
    let tops = affected(&doc, &d, &Options::default(), &[far]);
    assert_eq!(tops, vec![far]);
}

#[test]
fn large_page_stays_quick() {
    // 60 horizontal and 60 vertical connectors: 3600 crossings, each horizontal jumps 60 times
    let (mut doc, page) = new_doc();
    let mut hs = Vec::new();
    for i in 0..60u32 {
        let y = 0.5 + i as f64 * 0.17;
        hs.push(conn(&mut doc, page, 1 + i, &[[0.2, y], [11.0, y]], &[]));
    }
    for i in 0..60u32 {
        let x = 0.5 + i as f64 * 0.17;
        conn(&mut doc, page, 100 + i, &[[x, 0.2], [x, 11.0]], &[]);
    }
    let mut d = draw(&doc, page);
    assert!(hs.iter().all(|&h| jumps_of(&d, h).len() == 60));
    assert_eq!(d.jumps().map(|(_, j)| j.len()).sum::<usize>(), 3600);
    // one horizontal moved: it and the 60 verticals are made anew, the other horizontals not
    set_cell(&mut doc, SheetRef::Shape(hs[3]), "PinY", 0.05);
    let tops = affected(&doc, &d, &Options::default(), &[hs[3]]);
    assert_eq!(tops.len(), 61);
    assert!(redo(&doc, &mut d, &[hs[3]]));
    same(&doc, page, &d);
}
