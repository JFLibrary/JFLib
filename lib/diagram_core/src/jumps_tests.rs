// jumps_tests -- (JD21) line jumps in the connectors' Geometry, against what Visio 16 wrote in
// the JD21v check (visio-run2 [jumps], visio-run3 [jumps2], visio-run4 [jumprows]):
//
//   1. the rows of every style both ways and of vertical pieces (Visio's numbers to 1e-9)
//   2. connectors coming, moving, brought to the front under the codes 1, 3, 4, 5; slopes
//   3. the pages of JD21's first check after Visio laid them anew: size, close, slanted, a
//      connector's own codes, the page's default size
//   4. reading: the way without jumps (glue::path, route_edit handles), read(), drawn
//   5. partners that lose their jumps (moved away, deleted), the page laid anew, plain lines,
//      hidden and grouped connectors, a large page
//
//   cargo test -p jflib-diagram-core --lib jumps

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::glue::Target;
use crate::route_styles::tests::{fx, Fx};
use crate::{route_edit, CellAddr, RowKey, SecKey, ShapeRef, SheetRef, Value};

/// the step a free connector of Visio's makes before it goes its way (3.75 mm)
const S: f64 = 3.75 / 25.4;
const MM: f64 = 1.0 / 25.4;

// ---- building

/// a free dynamic connector laid along `way` (its ends the first and last point)
fn conn(f: &mut Fx, way: &[[f64; 2]]) -> ShapeRef {
    let n = way.len();
    let mut tx = f.doc.transaction("Verbinder");
    let c = glue::new_connector(&mut tx, &mut f.eng, f.owner, (way[0], Target::None), (way[n - 1], Target::None)).unwrap();
    route_edit::set_way(&mut tx, &mut f.eng, c, way).unwrap();
    tx.commit();
    c
}

/// a connector laid again along a new way
fn relay(f: &mut Fx, c: ShapeRef, way: &[[f64; 2]]) {
    let mut tx = f.doc.transaction("Weg");
    let sheet = SheetRef::Shape(c);
    let n = way.len();
    for (cell, v) in [("BeginX", way[0][0]), ("BeginY", way[0][1]), ("EndX", way[n - 1][0]), ("EndY", way[n - 1][1])] {
        let a = tx.single(cell);
        tx.set_constant(sheet, &a, Value::Num(v), None).unwrap();
    }
    let ends: Vec<(SheetRef, CellAddr)> = [names::BEGIN_X, names::BEGIN_Y, names::END_X, names::END_Y].iter().map(|n| (sheet, CellAddr::single(*n))).collect();
    f.eng.recalc_cells(&mut tx, &ends).unwrap();
    route_edit::set_way(&mut tx, &mut f.eng, c, way).unwrap();
    tx.commit();
}

/// Visio's U of a free connector from a to b (a step of S to the right or down, as it routes)
fn u_way(a: [f64; 2], b: [f64; 2]) -> Vec<[f64; 2]> {
    if a[1] == b[1] {
        vec![a, [a[0], a[1] - S], [b[0], b[1] - S], b]
    } else {
        vec![a, [a[0] + S, a[1]], [b[0] + S, b[1]], b]
    }
}

/// a connector's rows on the page as Visio's check prints them: type, point, bow
fn rows(f: &Fx, c: ShapeRef) -> Vec<(u32, [f64; 2], f64)> {
    let m = f.doc.page_xform(c);
    rows_of(&f.doc, c)
        .unwrap()
        .into_iter()
        .map(|(k, p)| {
            let (tag, bow) = match k {
                Kind::Move => (138, 0.0),
                Kind::Line => (139, 0.0),
                Kind::Arc(b) => (140, b),
                Kind::Poly(_) => (193, 0.0),
                Kind::Other => (0, 0.0),
            };
            (tag, m.apply(p), bow)
        })
        .collect()
}

/// a way as Visio's check prints it: "138(1,2) 139(1,1.852) 140(3.18,1.852;bow -0.033) ..."
fn visio(text: &str) -> Vec<(u32, [f64; 2], f64)> {
    text.split(')')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (tag, rest) = p.split_once('(').unwrap();
            let (xy, bow) = match rest.split_once(';') {
                Some((xy, b)) => (xy, b.trim().trim_start_matches("bow").trim().parse::<f64>().unwrap()),
                None => (rest, 0.0),
            };
            let (x, y) = xy.split_once(',').unwrap();
            (tag.trim().parse().unwrap(), [x.trim().parse().unwrap(), y.trim().parse().unwrap()], bow)
        })
        .collect()
}

/// the rows match Visio's (its numbers rounded to three places)
#[track_caller]
fn assert_way(f: &Fx, c: ShapeRef, want: &str) {
    let got = rows(f, c);
    let want = visio(want);
    let near = |a: f64, b: f64| (a - b).abs() <= 6e-4;
    let ok = got.len() == want.len() && got.iter().zip(&want).all(|(g, w)| g.0 == w.0 && near(g.1[0], w.1[0]) && near(g.1[1], w.1[1]) && near(g.2, w.2));
    assert!(ok, "rows\n  {got:?}\nnot Visio's\n  {want:?}");
}

/// the numbers of a row's A cell (POLYLINE)
fn poly_args(f: &Fx, c: ShapeRef, row: u32) -> Vec<f64> {
    let sheet = SheetRef::Shape(c);
    let sk = f.doc.geometry(sheet)[0];
    match f.doc.resolve(sheet, &CellAddr::row(sk, RowKey::Index(row), names::A)).value() {
        Value::Text(s) => parse_args(f.doc.str(s), "POLYLINE").unwrap(),
        v => panic!("A of row {row}: {v:?}"),
    }
}

/// a cell of a row (local value)
fn row_num(f: &Fx, c: ShapeRef, row: u32, cell: Sym) -> f64 {
    let sheet = SheetRef::Shape(c);
    let sk = f.doc.geometry(sheet)[0];
    f.doc.resolve(sheet, &CellAddr::row(sk, RowKey::Index(row), cell)).num().unwrap()
}

fn row_kind(f: &Fx, c: ShapeRef, row: u32) -> Option<Sym> {
    let sheet = SheetRef::Shape(c);
    let sk = f.doc.geometry(sheet)[0];
    f.doc.rows(sheet, sk).into_iter().find(|r| r.key == RowKey::Index(row)).and_then(|r| r.kind)
}

fn nums(text: &str) -> Vec<f64> {
    parse_args(text, "POLYLINE").unwrap()
}

#[track_caller]
fn assert_nums(got: &[f64], want: &[f64]) {
    assert!(got.len() == want.len() && got.iter().zip(want).all(|(a, b)| (a - b).abs() <= 1e-9), "{got:?}\nnot\n{want:?}");
}

// ---- 1. the rows of every style

// Visio's row 4 for each style (1 .. 9) and PageLineJumpDirX 0 / 2, left to right and right to
// left: ArcTo's A in mm, or the POLYLINE (visio-run4 [jumprows])
const ROWS: [(u32, u32, &str, &str); 18] = [
    (1, 0, "-0.83333333333334", "0.83333333333334"),
    (1, 2, "0.83333333333334", "-0.83333333333334"),
    (2, 0, "", ""),
    (2, 2, "", ""),
    (3, 0, "POLYLINE(0, 0, 0.42296587926509,-0.083333333333337, 0.43608923884514,-0.083333333333335)", "POLYLINE(0, 0, 0.56391076115486,-0.083333333333331, 0.57703412073491,-0.083333333333332)"),
    (3, 2, "POLYLINE(0, 0, 0.42296587926509,-0.41666666666667, 0.43608923884514,-0.41666666666667)", "POLYLINE(0, 0, 0.56391076115486,-0.41666666666667, 0.57703412073491,-0.41666666666667)"),
    (4, 0, "POLYLINE(0, 0, 0.42952755905512,-0.083333333333336)", "POLYLINE(0, 0, 0.57047244094488,-0.083333333333332)"),
    (4, 2, "POLYLINE(0, 0, 0.42952755905512,-0.41666666666667)", "POLYLINE(0, 0, 0.57047244094488,-0.41666666666667)"),
    (5, 0, "POLYLINE(0, 0, 0.42624671916011,-0.10666666666667, 0.43280839895013,-0.10666666666667)", "POLYLINE(0, 0, 0.56719160104987,-0.10666666666667, 0.5737532808399,-0.10666666666667)"),
    (5, 2, "POLYLINE(0, 0, 0.42624671916011,-0.39333333333334, 0.43280839895013,-0.39333333333334)", "POLYLINE(0, 0, 0.56719160104987,-0.39333333333333, 0.5737532808399,-0.39333333333333)"),
    (6, 0, "POLYLINE(0, 0, 0.4249343832021,-0.13333333333334, 0.42952755905512,-0.083333333333336, 0.43412073490814,-0.13333333333334)", "POLYLINE(0, 0, 0.56587926509186,-0.13333333333333, 0.57047244094488,-0.083333333333332, 0.5750656167979,-0.13333333333333)"),
    (6, 2, "POLYLINE(0, 0, 0.4249343832021,-0.36666666666667, 0.42952755905512,-0.41666666666667, 0.43412073490814,-0.36666666666667)", "POLYLINE(0, 0, 0.56587926509186,-0.36666666666667, 0.57047244094488,-0.41666666666667, 0.5750656167979,-0.36666666666667)"),
    (7, 0, "POLYLINE(0, 0, 0.4242782152231,-0.15333333333334, 0.42755905511811,-0.090000000000003, 0.43149606299213,-0.090000000000002, 0.43477690288714,-0.15333333333334)", "POLYLINE(0, 0, 0.56522309711286,-0.15333333333333, 0.56850393700787,-0.089999999999998, 0.57244094488189,-0.089999999999999, 0.5757217847769,-0.15333333333333)"),
    (7, 2, "POLYLINE(0, 0, 0.4242782152231,-0.34666666666667, 0.42755905511811,-0.41000000000001, 0.43149606299213,-0.41, 0.43477690288714,-0.34666666666667)", "POLYLINE(0, 0, 0.56522309711286,-0.34666666666667, 0.56850393700787,-0.41, 0.57244094488189,-0.41, 0.5757217847769,-0.34666666666667)"),
    (8, 0, "POLYLINE(0, 0, 0.4238845144357,-0.16666666666667, 0.42624671916011,-0.10666666666667, 0.42952755905512,-0.083333333333336, 0.43280839895013,-0.10666666666667, 0.43517060367454,-0.16666666666667)", "POLYLINE(0, 0, 0.56482939632546,-0.16666666666666, 0.56719160104987,-0.10666666666667, 0.57047244094488,-0.083333333333332, 0.5737532808399,-0.10666666666667, 0.5761154855643,-0.16666666666667)"),
    (8, 2, "POLYLINE(0, 0, 0.4238845144357,-0.33333333333334, 0.42624671916011,-0.39333333333334, 0.42952755905512,-0.41666666666667, 0.43280839895013,-0.39333333333334, 0.43517060367454,-0.33333333333334)", "POLYLINE(0, 0, 0.56482939632546,-0.33333333333333, 0.56719160104987,-0.39333333333333, 0.57047244094488,-0.41666666666667, 0.5737532808399,-0.39333333333333, 0.5761154855643,-0.33333333333333)"),
    (9, 0, "POLYLINE(0, 0, 0.42362204724409,-0.17666666666667, 0.4254593175853,-0.12, 0.42808398950131,-0.086666666666669, 0.43097112860892,-0.086666666666669, 0.43359580052493,-0.12, 0.43543307086614,-0.17666666666667)", "POLYLINE(0, 0, 0.56456692913386,-0.17666666666667, 0.56640419947507,-0.12, 0.56902887139108,-0.086666666666665, 0.57191601049869,-0.086666666666665, 0.5745406824147,-0.12, 0.57637795275591,-0.17666666666667)"),
    (9, 2, "POLYLINE(0, 0, 0.42362204724409,-0.32333333333334, 0.4254593175853,-0.38000000000001, 0.42808398950131,-0.41333333333334, 0.43097112860892,-0.41333333333334, 0.43359580052493,-0.38, 0.43543307086614,-0.32333333333334)", "POLYLINE(0, 0, 0.56456692913386,-0.32333333333333, 0.56640419947507,-0.38, 0.56902887139108,-0.41333333333333, 0.57191601049869,-0.41333333333333, 0.5745406824147,-0.38, 0.57637795275591,-0.32333333333333)"),
];

#[test]
fn every_style_writes_visios_rows() {
    for (style, dirx, ltr, rtl) in ROWS {
        let mut f = fx();
        f.set(None, "LineJumpStyle", style as f64);
        f.set(None, "PageLineJumpDirX", dirx as f64);
        // a page without the size cells: 2/3 of 2.5 mm
        conn(&mut f, &u_way([3.0, 1.0], [3.0, 5.0]));
        let h = conn(&mut f, &u_way([1.0, 3.0], [6.0, 3.0]));
        let back = conn(&mut f, &u_way([6.0, 2.0], [1.0, 2.0]));
        for (c, sign, want) in [(h, 1.0, ltr), (back, -1.0, rtl)] {
            let what = format!("style {style}, dir {dirx}, {}", if sign > 0.0 { "left to right" } else { "right to left" });
            assert_eq!(rows(&f, c).len(), 6, "{what}");
            // row 3: the jump's start, row 4: its end (local, mm), Visio's numbers
            let start = if sign > 0.0 { 53.716666666667 } else { -71.616666666667 };
            assert!((row_num(&f, c, 3, names::X) - start * MM).abs() < 1e-9, "{what}: {}", row_num(&f, c, 3, names::X) / MM);
            let end = if sign > 0.0 { 55.383333333333 } else { -73.283333333333 };
            assert!((row_num(&f, c, 4, names::X) - end * MM).abs() < 1e-9, "{what}: {}", row_num(&f, c, 4, names::X) / MM);
            assert!((row_num(&f, c, 4, names::Y) + 1.25 * MM).abs() < 1e-9, "{what}");
            match style {
                1 => {
                    assert_eq!(row_kind(&f, c, 4), Some(names::ARC_TO), "{what}");
                    let bow: f64 = want.parse().unwrap();
                    assert!((row_num(&f, c, 4, names::A) - bow * MM).abs() < 1e-9, "{what}: bow {}", row_num(&f, c, 4, names::A) / MM);
                }
                2 => assert_eq!(row_kind(&f, c, 4), Some(names::MOVE_TO), "{what}"),
                _ => {
                    assert_eq!(row_kind(&f, c, 4), Some(names::POLYLINE_TO), "{what}");
                    assert_nums(&poly_args(&f, c, 4), &nums(want));
                }
            }
        }
    }
}

#[test]
fn vertical_pieces_jump_left_by_default() {
    // LineJumpCode 2, style 9: Visio's rows of a vertical piece going up and one going down
    let up_left = "POLYLINE(0, 0, 1.1766666666667,0.45570866141732, 1.12,0.45800524934383, 1.0866666666667,0.46128608923885, 1.0866666666667,0.46489501312336, 1.12,0.46817585301837, 1.1766666666667,0.47047244094488)";
    let down_left = "POLYLINE(0, 0, 1.1766666666667,0.52952755905512, 1.12,0.53182414698163, 1.0866666666667,0.53510498687664, 1.0866666666667,0.53871391076115, 1.12,0.54199475065617, 1.1766666666667,0.54429133858268)";
    let up_right = "POLYLINE(0, 0, 1.3233333333333,0.45570866141732, 1.38,0.45800524934383, 1.4133333333333,0.46128608923885, 1.4133333333333,0.46489501312336, 1.38,0.46817585301837, 1.3233333333333,0.47047244094488)";
    let down_right = "POLYLINE(0, 0, 1.3233333333333,0.52952755905512, 1.38,0.53182414698163, 1.4133333333333,0.53510498687664, 1.4133333333333,0.53871391076115, 1.38,0.54199475065617, 1.3233333333333,0.54429133858268)";
    for (diry, up, down) in [(0.0, up_left, down_left), (1.0, up_left, down_left), (2.0, up_right, down_right)] {
        let mut f = fx();
        f.set(None, "LineJumpCode", 2.0);
        f.set(None, "LineJumpStyle", 9.0);
        f.set(None, "PageLineJumpDirY", diry);
        conn(&mut f, &u_way([1.0, 3.0], [6.0, 3.0]));
        let v = conn(&mut f, &u_way([3.0, 1.0], [3.0, 5.0]));
        let w = conn(&mut f, &[[4.0, 5.0], [4.0 + S, 5.0], [4.0 + S, 1.0], [4.0, 1.0]]);
        assert!((row_num(&f, v, 3, names::Y) - 46.216666666667 * MM).abs() < 1e-9);
        assert!((row_num(&f, v, 4, names::Y) - 47.883333333333 * MM).abs() < 1e-9);
        assert_nums(&poly_args(&f, v, 4), &nums(up));
        assert!((row_num(&f, w, 3, names::Y) + 53.716666666667 * MM).abs() < 1e-9);
        assert_nums(&poly_args(&f, w, 4), &nums(down));
    }
}

#[test]
fn the_polyline_value_is_text_with_its_formula() {
    let mut f = fx();
    f.set(None, "LineJumpStyle", 3.0);
    conn(&mut f, &u_way([3.0, 1.0], [3.0, 5.0]));
    let h = conn(&mut f, &u_way([1.0, 3.0], [6.0, 3.0]));
    let sheet = SheetRef::Shape(h);
    let sk = f.doc.geometry(sheet)[0];
    let r = f.doc.resolve(sheet, &CellAddr::row(sk, RowKey::Index(4), names::A));
    assert_eq!(r.unit(), Some(crate::Unit::Polyline));
    let crate::Formula::Expr(fs) = r.formula() else { panic!("no formula") };
    assert!(f.doc.str(fs).starts_with("POLYLINE(0, 0, "));
}

// ---- 2. connectors coming and going (visio-run3 [jumps2])

#[test]
fn horizontal_lines_partners_gain_jumps() {
    let mut f = fx();
    f.set(None, "LineJumpCode", 1.0);
    let h1 = conn(&mut f, &u_way([1.0, 2.0], [7.0, 2.0]));
    let h2 = conn(&mut f, &u_way([1.0, 4.0], [7.0, 4.0]));
    assert_way(&f, h1, "138(1,2) 139(1,1.852) 139(7,1.852) 139(7,2)");
    let v1 = conn(&mut f, &u_way([3.0, 1.0], [3.0, 5.0]));
    assert_way(&f, h1, "138(1,2) 139(1,1.852) 139(3.115,1.852) 140(3.18,1.852;bow -0.033) 139(7,1.852) 139(7,2)");
    assert_way(&f, h2, "138(1,4) 139(1,3.852) 139(3.115,3.852) 140(3.18,3.852;bow -0.033) 139(7,3.852) 139(7,4)");
    assert_way(&f, v1, "138(3,1) 139(3.148,1) 139(3.148,5) 139(3,5)");
    let v2 = conn(&mut f, &u_way([5.0, 5.0], [5.0, 1.0]));
    assert_way(&f, h1, "138(1,2) 139(1,1.852) 139(3.115,1.852) 140(3.18,1.852;bow -0.033) 139(5.115,1.852) 140(5.18,1.852;bow -0.033) 139(7,1.852) 139(7,2)");
    assert_way(&f, v2, "138(5,5) 139(5.148,5) 139(5.148,1) 139(5,1)");
    let h3 = conn(&mut f, &u_way([7.0, 3.0], [1.0, 3.0]));
    assert_way(&f, h3, "138(7,3) 139(7,2.852) 139(5.18,2.852) 140(5.115,2.852;bow 0.033) 139(3.18,2.852) 140(3.115,2.852;bow 0.033) 139(1,2.852) 139(1,3)");
    assert_way(&f, v1, "138(3,1) 139(3.148,1) 139(3.148,5) 139(3,5)");
    let mut tx = f.doc.transaction("vor");
    tx.bring_to_front(h1).unwrap();
    tx.commit();
    relay(&mut f, h1, &u_way([1.01, 2.0], [7.0, 2.0]));
    assert_way(&f, h1, "138(1.01,2) 139(1.01,1.852) 139(3.115,1.852) 140(3.18,1.852;bow -0.033) 139(5.115,1.852) 140(5.18,1.852;bow -0.033) 139(7,1.852) 139(7,2)");
    assert_way(&f, v1, "138(3,1) 139(3.148,1) 139(3.148,5) 139(3,5)");
}

#[test]
fn last_laid_jumps_partners_lose_theirs() {
    let mut f = fx();
    f.set(None, "LineJumpCode", 3.0);
    let h1 = conn(&mut f, &u_way([1.0, 2.0], [7.0, 2.0]));
    let h2 = conn(&mut f, &u_way([1.0, 4.0], [7.0, 4.0]));
    let v1 = conn(&mut f, &u_way([3.0, 1.0], [3.0, 5.0]));
    assert_way(&f, h1, "138(1,2) 139(1,1.852) 139(7,1.852) 139(7,2)");
    assert_way(&f, h2, "138(1,4) 139(1,3.852) 139(7,3.852) 139(7,4)");
    assert_way(&f, v1, "138(3,1) 139(3.148,1) 139(3.148,1.82) 140(3.148,1.885;bow -0.033) 139(3.148,3.82) 140(3.148,3.885;bow -0.033) 139(3.148,5) 139(3,5)");
    let v2 = conn(&mut f, &u_way([5.0, 5.0], [5.0, 1.0]));
    assert_way(&f, h1, "138(1,2) 139(1,1.852) 139(7,1.852) 139(7,2)");
    assert_way(&f, v2, "138(5,5) 139(5.148,5) 139(5.148,3.885) 140(5.148,3.82;bow 0.033) 139(5.148,1.885) 140(5.148,1.82;bow 0.033) 139(5.148,1) 139(5,1)");
    let h3 = conn(&mut f, &u_way([7.0, 3.0], [1.0, 3.0]));
    assert_way(&f, h3, "138(7,3) 139(7,2.852) 139(5.18,2.852) 140(5.115,2.852;bow 0.033) 139(3.18,2.852) 140(3.115,2.852;bow 0.033) 139(1,2.852) 139(1,3)");
    assert_way(&f, v1, "138(3,1) 139(3.148,1) 139(3.148,1.82) 140(3.148,1.885;bow -0.033) 139(3.148,3.82) 140(3.148,3.885;bow -0.033) 139(3.148,5) 139(3,5)");
    assert_way(&f, v2, "138(5,5) 139(5.148,5) 139(5.148,3.885) 140(5.148,3.82;bow 0.033) 139(5.148,1.885) 140(5.148,1.82;bow 0.033) 139(5.148,1) 139(5,1)");
    // h1 laid again: it jumps, v1 loses its jump over it and keeps the one over h2
    let mut tx = f.doc.transaction("vor");
    tx.bring_to_front(h1).unwrap();
    tx.commit();
    relay(&mut f, h1, &u_way([1.01, 2.0], [7.0, 2.0]));
    assert_way(&f, h1, "138(1.01,2) 139(1.01,1.852) 139(3.115,1.852) 140(3.18,1.852;bow -0.033) 139(5.115,1.852) 140(5.18,1.852;bow -0.033) 139(7,1.852) 139(7,2)");
    assert_way(&f, v1, "138(3,1) 139(3.148,1) 139(3.148,3.82) 140(3.148,3.885;bow -0.033) 139(3.148,5) 139(3,5)");
}

#[test]
fn upper_and_lower_ones_jump() {
    // code 4: the same as code 3 here (every new connector lies on top)
    let mut f = fx();
    f.set(None, "LineJumpCode", 4.0);
    let h1 = conn(&mut f, &u_way([1.0, 2.0], [7.0, 2.0]));
    let h2 = conn(&mut f, &u_way([1.0, 4.0], [7.0, 4.0]));
    let v1 = conn(&mut f, &u_way([3.0, 1.0], [3.0, 5.0]));
    assert_way(&f, h2, "138(1,4) 139(1,3.852) 139(7,3.852) 139(7,4)");
    assert_way(&f, v1, "138(3,1) 139(3.148,1) 139(3.148,1.82) 140(3.148,1.885;bow -0.033) 139(3.148,3.82) 140(3.148,3.885;bow -0.033) 139(3.148,5) 139(3,5)");
    let v2 = conn(&mut f, &u_way([5.0, 5.0], [5.0, 1.0]));
    assert_way(&f, v2, "138(5,5) 139(5.148,5) 139(5.148,3.885) 140(5.148,3.82;bow 0.033) 139(5.148,1.885) 140(5.148,1.82;bow 0.033) 139(5.148,1) 139(5,1)");
    let h3 = conn(&mut f, &u_way([7.0, 3.0], [1.0, 3.0]));
    assert_way(&f, h3, "138(7,3) 139(7,2.852) 139(5.18,2.852) 140(5.115,2.852;bow 0.033) 139(3.18,2.852) 140(3.115,2.852;bow 0.033) 139(1,2.852) 139(1,3)");
    let mut tx = f.doc.transaction("vor");
    tx.bring_to_front(h1).unwrap();
    tx.commit();
    relay(&mut f, h1, &u_way([1.01, 2.0], [7.0, 2.0]));
    assert_way(&f, h1, "138(1.01,2) 139(1.01,1.852) 139(3.115,1.852) 140(3.18,1.852;bow -0.033) 139(5.115,1.852) 140(5.18,1.852;bow -0.033) 139(7,1.852) 139(7,2)");
    assert_way(&f, v1, "138(3,1) 139(3.148,1) 139(3.148,3.82) 140(3.148,3.885;bow -0.033) 139(3.148,5) 139(3,5)");
    // code 5: the lower one jumps
    let mut f = fx();
    f.set(None, "LineJumpCode", 5.0);
    let h1 = conn(&mut f, &u_way([1.0, 2.0], [7.0, 2.0]));
    let h2 = conn(&mut f, &u_way([1.0, 4.0], [7.0, 4.0]));
    let v1 = conn(&mut f, &u_way([3.0, 1.0], [3.0, 5.0]));
    assert_way(&f, h1, "138(1,2) 139(1,1.852) 139(3.115,1.852) 140(3.18,1.852;bow -0.033) 139(7,1.852) 139(7,2)");
    assert_way(&f, h2, "138(1,4) 139(1,3.852) 139(3.115,3.852) 140(3.18,3.852;bow -0.033) 139(7,3.852) 139(7,4)");
    assert_way(&f, v1, "138(3,1) 139(3.148,1) 139(3.148,5) 139(3,5)");
    let v2 = conn(&mut f, &u_way([5.0, 5.0], [5.0, 1.0]));
    assert_way(&f, h1, "138(1,2) 139(1,1.852) 139(3.115,1.852) 140(3.18,1.852;bow -0.033) 139(5.115,1.852) 140(5.18,1.852;bow -0.033) 139(7,1.852) 139(7,2)");
    assert_way(&f, v2, "138(5,5) 139(5.148,5) 139(5.148,1) 139(5,1)");
    let h3 = conn(&mut f, &u_way([7.0, 3.0], [1.0, 3.0]));
    assert_way(&f, h3, "138(7,3) 139(7,2.852) 139(1,2.852) 139(1,3)");
    assert_way(&f, v1, "138(3,1) 139(3.148,1) 139(3.148,2.82) 140(3.148,2.885;bow -0.033) 139(3.148,5) 139(3,5)");
    assert_way(&f, v2, "138(5,5) 139(5.148,5) 139(5.148,2.885) 140(5.148,2.82;bow 0.033) 139(5.148,1) 139(5,1)");
    let mut tx = f.doc.transaction("vor");
    tx.bring_to_front(h1).unwrap();
    tx.commit();
    relay(&mut f, h1, &u_way([1.01, 2.0], [7.0, 2.0]));
    assert_way(&f, h1, "138(1.01,2) 139(1.01,1.852) 139(7,1.852) 139(7,2)");
    assert_way(&f, v1, "138(3,1) 139(3.148,1) 139(3.148,1.82) 140(3.148,1.885;bow -0.033) 139(3.148,2.82) 140(3.148,2.885;bow -0.033) 139(3.148,5) 139(3,5)");
}

#[test]
fn slopes_equal_ones_do_not_jump_close_ones_merge() {
    let mut f = fx();
    f.set(None, "LineJumpCode", 1.0);
    f.set(None, "RouteStyle", 2.0);
    let up = conn(&mut f, &[[1.0, 1.0], [5.0, 5.0]]);
    let down = conn(&mut f, &[[1.0, 5.0], [5.0, 1.0]]);
    let gentle = conn(&mut f, &[[1.0, 2.5], [5.0, 3.2]]);
    let steep = conn(&mut f, &[[2.6, 0.5], [3.2, 5.5]]);
    assert_way(&f, up, "138(1,1) 139(2.863,2.863) 140(2.91,2.91;bow -0.033) 139(5,5)");
    assert_way(&f, down, "138(1,5) 139(2.888,3.112) 140(2.934,3.066;bow -0.033) 139(5,1)");
    assert_way(&f, gentle, "138(1,2.5) 139(2.786,2.813) 140(2.912,2.835;bow -0.064) 139(3.095,2.867) 140(3.16,2.878;bow -0.033) 139(5,3.2)");
    assert_way(&f, steep, "138(2.6,0.5) 139(3.2,5.5)");
}

// ---- 3. the pages of JD21's first check, laid anew by Visio (visio-run2 [jumps])

// the grid of a page: horizontals at y 2, 4, 6 (x 1 .. 9), verticals at x 3, 5, 7 (y 1 .. 7) as
// Visio laid them (a step of 0.264 in), in the order Visio laid them
fn grid(f: &mut Fx, xs: [f64; 3]) -> (Vec<ShapeRef>, Vec<ShapeRef>) {
    let mut hs = Vec::new();
    for y in [2.0, 4.0, 6.0] {
        hs.push(conn(f, &[[1.0, y], [1.0, y - 0.264], [9.0, y - 0.264], [9.0, y]]));
    }
    let mut vs = Vec::new();
    for x in xs {
        vs.push(conn(f, &[[x, 1.0], [x + 0.264, 1.0], [x + 0.264, 7.0], [x, 7.0]]));
    }
    (hs, vs)
}

#[test]
fn a_page_without_cells_jumps_two_thirds_of_2_5_mm() {
    let mut f = fx();
    let (hs, vs) = grid(&mut f, [3.0, 5.0, 7.0]);
    assert_way(&f, hs[0], "138(1,2) 139(1,1.736) 139(3.231,1.736) 140(3.297,1.736;bow -0.033) 139(5.231,1.736) 140(5.297,1.736;bow -0.033) 139(7.231,1.736) 140(7.297,1.736;bow -0.033) 139(9,1.736) 139(9,2)");
    assert_way(&f, vs[0], "138(3,1) 139(3.264,1) 139(3.264,7) 139(3,7)");
}

#[test]
fn size_close_and_own_codes_as_visio() {
    // Größe: LineJumpFactorX 3 x LineToLineX 0.125
    let mut f = fx();
    for (c, v) in [("LineToLineX", 0.125), ("LineToLineY", 0.125), ("LineJumpCode", 1.0), ("LineJumpFactorX", 3.0), ("LineJumpFactorY", 0.3)] {
        f.set(None, c, v);
    }
    let (hs, _) = grid(&mut f, [3.0, 5.0, 7.0]);
    assert_way(&f, hs[0], "138(1,2) 139(1,1.736) 139(3.076,1.736) 140(3.451,1.736;bow -0.188) 139(5.076,1.736) 140(5.451,1.736;bow -0.188) 139(7.076,1.736) 140(7.451,1.736;bow -0.188) 139(9,1.736) 139(9,2)");
    // Nah: two verticals 0.05 in apart, one jump over both
    let mut f = fx();
    for (c, v) in [("LineToLineX", 0.125), ("LineToLineY", 0.125), ("LineJumpCode", 1.0)] {
        f.set(None, c, v);
    }
    let (hs, _) = grid(&mut f, [3.0, 3.05, 7.0]);
    assert_way(&f, hs[0], "138(1,2) 139(1,1.736) 139(3.222,1.736) 140(3.356,1.736;bow -0.067) 139(7.222,1.736) 140(7.306,1.736;bow -0.042) 139(9,1.736) 139(9,2)");
    // Eigene nie Stil: the top one never, the middle one squares down, a vertical: neither
    let mut f = fx();
    for (c, v) in [("LineToLineX", 0.125), ("LineToLineY", 0.125), ("LineJumpCode", 1.0)] {
        f.set(None, c, v);
    }
    let (hs, vs) = grid(&mut f, [3.0, 5.0, 7.0]);
    f.set(Some(hs[2]), "ConLineJumpCode", 1.0);
    f.set(Some(hs[1]), "ConLineJumpStyle", 3.0);
    f.set(Some(hs[1]), "ConLineJumpDirX", 2.0);
    f.set(Some(vs[1]), "ConLineJumpCode", 4.0);
    let mut tx = f.doc.transaction("Verhalten");
    lay_again(&mut tx, &mut f.eng, &[hs[0], hs[1], hs[2], vs[0], vs[1], vs[2]]).unwrap();
    tx.commit();
    assert_way(&f, hs[0], "138(1,2) 139(1,1.736) 139(3.222,1.736) 140(3.306,1.736;bow -0.042) 139(7.222,1.736) 140(7.306,1.736;bow -0.042) 139(9,1.736) 139(9,2)");
    assert_way(&f, hs[1], "138(1,4) 139(1,3.736) 139(3.222,3.736) 193(3.306,3.736) 139(7.222,3.736) 193(7.306,3.736) 139(9,3.736) 139(9,4)");
    assert_way(&f, hs[2], "138(1,6) 139(1,5.736) 139(9,5.736) 139(9,6)");
    let j = read(&f.doc, hs[1]);
    assert!(j.len() == 2 && j.iter().all(|j| j.style == JumpStyle::Square && j.normal[1] < -0.99), "{j:?}");
    // Eigene immer: no jumps on the page, the top one always
    let mut f = fx();
    for (c, v) in [("LineToLineX", 0.125), ("LineToLineY", 0.125), ("LineJumpCode", 0.0)] {
        f.set(None, c, v);
    }
    let (hs, vs) = grid(&mut f, [3.0, 5.0, 7.0]);
    f.set(Some(hs[2]), "ConLineJumpCode", 2.0);
    let mut tx = f.doc.transaction("Verhalten");
    lay_again(&mut tx, &mut f.eng, &[hs[2]]).unwrap();
    tx.commit();
    assert_way(&f, hs[2], "138(1,6) 139(1,5.736) 139(3.222,5.736) 140(3.306,5.736;bow -0.042) 139(5.222,5.736) 140(5.306,5.736;bow -0.042) 139(7.222,5.736) 140(7.306,5.736;bow -0.042) 139(9,5.736) 139(9,6)");
    assert!(read(&f.doc, hs[0]).is_empty() && read(&f.doc, vs[0]).is_empty());
}

#[test]
fn slanted_straight_connectors_as_visio() {
    let mut f = fx();
    for (c, v) in [("LineToLineX", 0.125), ("LineToLineY", 0.125), ("LineJumpCode", 1.0), ("RouteStyle", 2.0)] {
        f.set(None, c, v);
    }
    let a = conn(&mut f, &[[2.0, 1.0], [8.0, 7.0]]);
    let b = conn(&mut f, &[[2.0, 7.0], [8.0, 1.0]]);
    let flat = conn(&mut f, &[[1.0, 2.0], [10.0, 3.0]]);
    assert_way(&f, a, "138(2,1) 139(8,7)");
    assert_way(&f, b, "138(2,7) 139(8,1)");
    assert_way(&f, flat, "138(1,2) 139(3.209,2.245) 140(3.291,2.255;bow -0.042) 139(6.359,2.595) 140(6.441,2.605;bow -0.042) 139(10,3)");
}

// ---- 4. reading

#[test]
fn ways_and_handles_leave_the_jumps_out() {
    let mut f = fx();
    f.set(None, "LineJumpStyle", 3.0);
    let v = conn(&mut f, &[[3.0, 1.0], [3.0, 5.0]]);
    let h = conn(&mut f, &[[1.0, 3.0], [2.0, 3.0], [2.0, 2.0], [6.0, 2.0]]);
    assert_eq!(read(&f.doc, h).len(), 1);
    assert!(read(&f.doc, v).is_empty());
    let plain = vec![[1.0, 3.0], [2.0, 3.0], [2.0, 2.0], [6.0, 2.0]];
    let same = |a: &[[f64; 2]], b: &[[f64; 2]]| a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p[0] - q[0]).abs() < 1e-9 && (p[1] - q[1]).abs() < 1e-9);
    assert!(same(&way(&f.doc, h).unwrap(), &plain));
    assert!(same(&glue::path(&f.doc, h), &plain));
    assert!(way(&f.doc, v).is_none());
    // the handles: as without the jump (three midpoints, two vertices)
    let w = route_edit::way(&f.doc, h).unwrap();
    assert!(same(&w.pts, &plain) && w.right);
    let hs = route_edit::handles(&f.doc, h);
    assert_eq!((hs.iter().filter(|x| !x.bend).count(), hs.iter().filter(|x| x.bend).count()), (3, 2));
    // a piece moved by hand: the jump moves along
    let mut tx = f.doc.transaction("Weg");
    assert!(route_edit::edit(&mut tx, &mut f.eng, h, route_edit::Edit::Piece(2, [4.0, 2.5]), 0.0).unwrap());
    tx.commit();
    let j = read(&f.doc, h);
    assert!(j.len() == 1 && (j[0].start[1] - 2.5).abs() < 1e-9, "{j:?}");
}

#[test]
fn read_gives_style_side_and_size() {
    for (style, want) in [(0.0, JumpStyle::Arc), (2.0, JumpStyle::Gap), (3.0, JumpStyle::Square), (4.0, JumpStyle::Sides(2)), (5.0, JumpStyle::Sides(3)), (9.0, JumpStyle::Sides(7))] {
        let mut f = fx();
        f.set(None, "LineJumpStyle", style);
        f.set(None, "LineToLineX", 0.3);
        f.set(None, "LineJumpFactorX", 1.0);
        conn(&mut f, &[[4.0, 1.0], [4.0, 5.0]]);
        let h = conn(&mut f, &[[1.0, 3.0], [6.0, 3.0]]);
        let j = read(&f.doc, h);
        assert_eq!(j.len(), 1);
        assert_eq!(j[0].style, want);
        assert!((j[0].width() - 0.3).abs() < 1e-9 && (j[0].start[0] - 3.85).abs() < 1e-9);
        if want != JumpStyle::Gap {
            assert!((j[0].normal[1] - 1.0).abs() < 1e-9, "{j:?}");
        }
        let h_want = match want {
            JumpStyle::Sides(3) => 0.15 * 0.86,
            JumpStyle::Sides(7) => 0.15 * 0.98,
            _ => 0.15,
        };
        assert!((j[0].height - h_want).abs() < 1e-9, "{want:?}: {}", j[0].height);
    }
}

#[test]
fn the_drawing_shows_the_rows() {
    use crate::render::{build, ApproxMeasure, ItemKind, Options, Role};
    let mut f = fx();
    conn(&mut f, &[[4.0, 1.0], [4.0, 5.0]]);
    let h = conn(&mut f, &[[1.0, 3.0], [6.0, 3.0]]);
    let d = build(&f.doc, f.page, &mut ApproxMeasure, &Options::default());
    let line = d
        .items
        .iter()
        .find_map(|it| match &it.kind {
            ItemKind::Stroke { path, .. } if it.shape == h && it.role == Role::Shape => Some(path.clone()),
            _ => None,
        })
        .unwrap();
    // to the jump, the arc in two quarter pieces, on to the end
    let segs = &line.subs[0].segs;
    assert_eq!(segs.len(), 4);
    let top = segs[1].end();
    assert!((top[0] - 4.0).abs() < 1e-9 && (top[1] - (3.0 + 2.5 * MM / 3.0)).abs() < 1e-9, "{top:?}");
}

// ---- 5. partners, the page, what takes part

#[test]
fn a_connector_moved_away_or_deleted_takes_the_jumps_over_it() {
    let mut f = fx();
    let h1 = conn(&mut f, &[[1.0, 5.0], [8.0, 5.0]]);
    let h2 = conn(&mut f, &[[1.0, 8.0], [8.0, 8.0]]);
    let v = conn(&mut f, &[[4.0, 3.0], [4.0, 6.0]]);
    assert_eq!((read(&f.doc, h1).len(), read(&f.doc, h2).len()), (1, 0));
    // v grows across h2: h2 jumps too
    relay(&mut f, v, &[[4.0, 3.0], [4.0, 9.0]]);
    assert_eq!((read(&f.doc, h1).len(), read(&f.doc, h2).len()), (1, 1));
    // v moves away: neither jumps
    relay(&mut f, v, &[[12.0, 3.0], [12.0, 9.0]]);
    assert_eq!((read(&f.doc, h1).len(), read(&f.doc, h2).len()), (0, 0));
    relay(&mut f, v, &[[4.0, 3.0], [4.0, 9.0]]);
    assert_eq!(read(&f.doc, h1).len(), 1);
    // v deleted: the jumps over it go
    let mut tx = f.doc.transaction("Löschen");
    glue::detach(&mut tx, &mut f.eng, f.owner, &[v]).unwrap();
    tx.delete_shapes(&[v]).unwrap();
    tx.commit();
    assert_eq!((read(&f.doc, h1).len(), read(&f.doc, h2).len()), (0, 0));
    // a merged jump over two close ones: one moves away, the jump shrinks to the other
    let mut f = fx();
    let h = conn(&mut f, &[[1.0, 5.0], [8.0, 5.0]]);
    conn(&mut f, &[[4.0, 3.0], [4.0, 6.0]]);
    let w = conn(&mut f, &[[4.04, 3.0], [4.04, 6.0]]);
    let j = read(&f.doc, h);
    assert!(j.len() == 1 && (j[0].width() - (0.04 + 2.5 * MM * 2.0 / 3.0)).abs() < 1e-9, "{j:?}");
    relay(&mut f, w, &[[6.0, 3.0], [6.0, 4.0]]);
    let j = read(&f.doc, h);
    assert!(j.len() == 1 && (j[0].width() - 2.5 * MM * 2.0 / 3.0).abs() < 1e-9, "{j:?}");
}

#[test]
fn the_page_laid_anew_follows_its_cells() {
    let mut f = fx();
    let h = conn(&mut f, &[[2.0, 5.0], [6.0, 5.0]]);
    let v = conn(&mut f, &[[4.0, 3.0], [4.0, 7.0]]);
    assert_eq!((read(&f.doc, h).len(), read(&f.doc, v).len()), (1, 0));
    for (code, hj, vj) in [(0.0, 0, 0), (2.0, 0, 1), (3.0, 0, 1), (4.0, 0, 1), (5.0, 1, 0), (1.0, 1, 0)] {
        f.set(None, "LineJumpCode", code);
        // a cell alone changes nothing: the file shows its rows
        let before = (read(&f.doc, h).len(), read(&f.doc, v).len());
        let mut tx = f.doc.transaction("Seite");
        lay_page(&mut tx, &mut f.eng, f.page).unwrap();
        tx.commit();
        assert_eq!((read(&f.doc, h).len(), read(&f.doc, v).len()), (hj, vj), "code {code} (before {before:?})");
    }
    // rows that do not change are not written again
    let mut tx = f.doc.transaction("Seite");
    lay_page(&mut tx, &mut f.eng, f.page).unwrap();
    assert!(tx.commit().is_none());
}

#[test]
fn connector_codes_decide() {
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
        let mut f = fx();
        f.set(None, "LineJumpCode", pc);
        let h = conn(&mut f, &[[2.0, 5.0], [6.0, 5.0]]);
        let v = conn(&mut f, &[[4.0, 3.0], [4.0, 7.0]]);
        f.set(Some(h), "ConLineJumpCode", ch);
        f.set(Some(v), "ConLineJumpCode", cv);
        let mut tx = f.doc.transaction("Verhalten");
        lay_again(&mut tx, &mut f.eng, &[h, v]).unwrap();
        tx.commit();
        assert_eq!((read(&f.doc, h).len(), read(&f.doc, v).len()), (hj, vj), "h {ch} v {cv} page {pc}");
    }
}

#[test]
fn bends_ends_plain_lines_hidden_and_grouped() {
    use crate::hit::tests::{add, poly};
    let mut f = fx();
    // a right-angle connector bending at (4, 5); a vertical crossing right next to the bend and
    // one crossing the first piece in its middle; a T; one ending short of the line
    let bent = conn(&mut f, &[[1.0, 5.0], [4.0, 5.0], [4.0, 8.0]]);
    conn(&mut f, &[[3.99, 3.0], [3.99, 7.0]]);
    conn(&mut f, &[[2.0, 3.0], [2.0, 7.0]]);
    conn(&mut f, &[[3.0, 3.0], [3.0, 5.0]]);
    conn(&mut f, &[[3.5, 3.0], [3.5, 4.9]]);
    // a plain line (no routable flag) across: neither jumps nor is jumped over
    let g = poly(&mut f.doc, 0, &[[1.5, 3.0], [1.5, 7.0]]);
    add(&mut f.doc, f.page, None, 60, [0.0, 0.0], [0.0, 0.0], vec![g], &[("BeginX", 1.5), ("BeginY", 3.0)], None);
    let mut tx = f.doc.transaction("Seite");
    lay_page(&mut tx, &mut f.eng, f.page).unwrap();
    tx.commit();
    let j = read(&f.doc, bent);
    assert_eq!(j.len(), 1);
    assert!(((j[0].start[0] + j[0].end[0]) * 0.5 - 2.0).abs() < 1e-9);
    // a connector without a line, one on a hidden layer: nobody jumps over them
    let mut f = fx();
    let h = conn(&mut f, &[[2.0, 5.0], [6.0, 5.0]]);
    let v = conn(&mut f, &[[4.0, 3.0], [4.0, 7.0]]);
    f.set(Some(v), "LinePattern", 0.0);
    relay(&mut f, h, &[[2.0, 5.0], [6.0, 5.0]]);
    assert!(read(&f.doc, h).is_empty());
    f.set(Some(v), "LinePattern", 1.0);
    let mut tx = f.doc.transaction("Ebene");
    let hidden = tx.add_layer(f.page, "Verborgen", "Hidden").unwrap();
    let visible = tx.single("Visible");
    let sec = SecKey::one(tx.intern("Layer"));
    tx.set_constant(SheetRef::Page(f.page), &CellAddr::row(sec, RowKey::Index(hidden), visible.cell), Value::Num(0.0), None).unwrap();
    tx.set_layers(v, &[hidden]).unwrap();
    tx.commit().unwrap();
    relay(&mut f, h, &[[2.0, 5.0], [6.0, 5.0]]);
    assert!(read(&f.doc, h).is_empty());
    // a connector in a group (the group's local coordinates: moved by (1, 0))
    let mut f = fx();
    let h = conn(&mut f, &[[2.0, 5.0], [7.0, 5.0]]);
    let grp = add(&mut f.doc, f.page, None, 70, [1.0, 0.0], [0.0, 0.0], vec![], &[], None);
    let g = poly(&mut f.doc, 0, &[[4.5, 3.0], [4.5, 7.0]]);
    add(&mut f.doc, f.page, Some(grp), 71, [0.0, 0.0], [0.0, 0.0], vec![g], &[("BeginX", 4.5), ("BeginY", 3.0), ("ObjType", 2.0)], None);
    relay(&mut f, h, &[[2.0, 5.0], [7.0, 5.0]]);
    let j = read(&f.doc, h);
    assert_eq!(j.len(), 1);
    assert!(((j[0].start[0] + j[0].end[0]) * 0.5 - 5.5).abs() < 1e-9, "{j:?}");
}

#[test]
fn large_page_stays_quick() {
    // 60 horizontal and 60 vertical connectors: 3600 crossings, each horizontal jumps 60 times
    let mut f = fx();
    let mut hs = Vec::new();
    let mut tx = f.doc.transaction("viele");
    for i in 0..60u32 {
        let y = 0.5 + i as f64 * 0.17;
        hs.push(glue::new_connector(&mut tx, &mut f.eng, f.owner, ([0.2, y], Target::None), ([11.0, y], Target::None)).unwrap());
    }
    for i in 0..60u32 {
        let x = 0.5 + i as f64 * 0.17;
        glue::new_connector(&mut tx, &mut f.eng, f.owner, ([x, 0.2], Target::None), ([x, 11.0], Target::None)).unwrap();
    }
    tx.commit();
    assert!(hs.iter().all(|&h| read(&f.doc, h).len() == 60));
    // one horizontal moved: only it and the verticals are written
    let mut tx = f.doc.transaction("eine");
    let sheet = SheetRef::Shape(hs[3]);
    for (c, v) in [("BeginY", 0.05), ("EndY", 0.05)] {
        let a = tx.single(c);
        tx.set_constant(sheet, &a, Value::Num(v), None).unwrap();
    }
    glue::update(&mut tx, &mut f.eng, &[hs[3]]).unwrap();
    let change = tx.commit().unwrap();
    let shapes: Vec<SheetRef> = change.sheets().into_iter().filter(|s| matches!(s, SheetRef::Shape(_))).collect();
    assert_eq!(shapes.len(), 1, "{}", shapes.len());
}
