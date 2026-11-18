// tests of the editing gestures (cargo test -p jflib-diagram-core --lib)
//
//   moving (also a subshape of a turned group, 1D shapes with a glued end, locks, GUARD), the
//   eight grips of turned and flipped shapes, scaling a selection, turning, flipping, the
//   pin, PAR and LOC against page_xform, grouping and ungrouping (every corner stays), the
//   z-order, rows in the middle of a section (also inherited), control points, vertices, new
//   shapes and text, copy and paste; every gesture is undone and redone (with eng.refresh)
//
// Every document is built here; nothing comes from a Visio file.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use super::*;
use crate::formula::{Engine, Outcome, Val};
use crate::{
    Change, DocKind, Document, Formula, Page, PageHeader, PageRef, Row, RowKey, SecKey, Section, ShapeData, ShapeKind,
    ShapeRef, Style, StyleRefs,
};

const DEG: f64 = core::f64::consts::PI / 180.0;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + b.abs())
}

fn close_pt(a: [f64; 2], b: [f64; 2]) -> bool {
    close(a[0], b[0]) && close(a[1], b[1])
}

macro_rules! assert_pt {
    ($a:expr, $b:expr) => {{
        let (a, b): ([f64; 2], [f64; 2]) = ($a, $b);
        assert!(close_pt(a, b), "{}: {:?} is not {:?}", stringify!($a), a, b);
    }};
}

macro_rules! assert_close {
    ($a:expr, $b:expr) => {{
        let (a, b): (f64, f64) = ($a, $b);
        assert!(close(a, b), "{}: {} is not {}", stringify!($a), a, b);
    }};
}

// ---- building documents

struct Fx {
    doc: Document,
    page: PageRef,
    owner: Owner,
}

fn style(doc: &mut Document, id: u32, name: &str, based_on: Option<u32>) {
    let mut st = Style::default();
    st.header.id = id;
    st.header.name_u = Some(doc.intern(name));
    st.header.name = Some(doc.intern(name));
    if let Some(b) = based_on {
        st.header.based_on = StyleRefs::all(b);
    }
    doc.insert_style(st).unwrap();
}

// No Style (0), Normal (1), Text Only (4); one page
fn fx() -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    style(&mut doc, 0, "No Style", None);
    style(&mut doc, 1, "Normal", Some(0));
    style(&mut doc, 4, "Text Only", Some(1));
    let mut ph = PageHeader { id: 0, ..PageHeader::default() };
    ph.name_u = Some(doc.intern("Page-1"));
    let page = doc.insert_page(Page::new(ph)).unwrap();
    Fx { doc, page, owner: Owner::Page(page) }
}

fn cnum(doc: &mut Document, name: &str, v: f64) -> Cell {
    Cell::constant(doc.intern(name), Value::Num(v), None)
}

fn cf(doc: &mut Document, name: &str, f: &str, v: f64) -> Cell {
    let (n, f) = (doc.intern(name), doc.intern(f));
    Cell { name: n, value: Value::Num(v), formula: Formula::Expr(f), unit: None, error: None, stale: false }
}

// a 2D shape: pin, size, angle in degrees, flips, the local pin in the middle (Width*0.5)
fn box_data(doc: &mut Document, kind: ShapeKind, pin: [f64; 2], size: [f64; 2], deg: f64, flips: (bool, bool)) -> ShapeData {
    let mut d = ShapeData::new(kind);
    let cells = alloc::vec![
        cnum(doc, "PinX", pin[0]),
        cnum(doc, "PinY", pin[1]),
        cnum(doc, "Width", size[0]),
        cnum(doc, "Height", size[1]),
        cf(doc, "LocPinX", "Width*0.5", size[0] * 0.5),
        cf(doc, "LocPinY", "Height*0.5", size[1] * 0.5),
        cnum(doc, "Angle", deg * DEG),
        cnum(doc, "FlipX", if flips.0 { 1.0 } else { 0.0 }),
        cnum(doc, "FlipY", if flips.1 { 1.0 } else { 0.0 }),
    ];
    d.sheet.cells = cells;
    d
}

fn insert(f: &mut Fx, parent: Option<ShapeRef>, id: u32, d: ShapeData) -> ShapeRef {
    let owner = f.owner;
    let slot = f.doc.page_mut(f.page).unwrap().shapes.insert(parent.map(|p| p.slot), id, d).unwrap();
    ShapeRef { owner, slot }
}

fn add_box(f: &mut Fx, parent: Option<ShapeRef>, id: u32, pin: [f64; 2], size: [f64; 2], deg: f64, flips: (bool, bool)) -> ShapeRef {
    let d = box_data(&mut f.doc, ShapeKind::Shape, pin, size, deg, flips);
    insert(f, parent, id, d)
}

fn add_group(f: &mut Fx, parent: Option<ShapeRef>, id: u32, pin: [f64; 2], size: [f64; 2], deg: f64, flips: (bool, bool)) -> ShapeRef {
    let d = box_data(&mut f.doc, ShapeKind::Group, pin, size, deg, flips);
    insert(f, parent, id, d)
}

// a 1D line with Visio's formulas for Width, Angle and the pin
fn add_line(f: &mut Fx, parent: Option<ShapeRef>, id: u32, b: [f64; 2], e: [f64; 2]) -> ShapeRef {
    let doc = &mut f.doc;
    let (dx, dy) = (e[0] - b[0], e[1] - b[1]);
    let len = math::sqrt(dx * dx + dy * dy);
    let mut d = ShapeData::new(ShapeKind::Shape);
    d.sheet.cells = alloc::vec![
        cnum(doc, "BeginX", b[0]),
        cnum(doc, "BeginY", b[1]),
        cnum(doc, "EndX", e[0]),
        cnum(doc, "EndY", e[1]),
        cf(doc, "Width", "SQRT((EndX-BeginX)^2+(EndY-BeginY)^2)", len),
        cnum(doc, "Height", 0.0),
        cf(doc, "Angle", "ATAN2(EndY-BeginY,EndX-BeginX)", math::atan2(dy, dx)),
        cf(doc, "PinX", "GUARD((BeginX+EndX)/2)", (b[0] + e[0]) * 0.5),
        cf(doc, "PinY", "GUARD((BeginY+EndY)/2)", (b[1] + e[1]) * 0.5),
        cf(doc, "LocPinX", "GUARD(Width*0.5)", len * 0.5),
        cf(doc, "LocPinY", "GUARD(Height*0.5)", 0.0),
    ];
    insert(f, parent, id, d)
}

fn set_cell(f: &mut Fx, s: ShapeRef, c: Cell) {
    let d = f.doc.sheet_mut(SheetRef::Shape(s)).unwrap();
    match d.cells.iter_mut().find(|x| x.name == c.name) {
        Some(x) => *x = c,
        None => d.cells.push(c),
    }
}

fn connect(f: &mut Fx, from: u32, from_cell: &str, to: u32, to_cell: &str) {
    let fc = f.doc.intern(from_cell);
    let tc = f.doc.intern(to_cell);
    f.doc.page_mut(f.page).unwrap().connects.push(Connect {
        from_sheet: from,
        from_cell: Some(fc),
        from_part: None,
        to_sheet: to,
        to_cell: Some(tc),
        to_part: None,
        extras: Default::default(),
    });
}

fn num(doc: &Document, s: ShapeRef, cell: &str) -> f64 {
    doc.resolve(SheetRef::Shape(s), &CellAddr::single(doc.sym(cell).unwrap())).num().unwrap_or(0.0)
}

fn formula(doc: &Document, s: ShapeRef, cell: &str) -> String {
    match doc.resolve(SheetRef::Shape(s), &CellAddr::single(doc.sym(cell).unwrap())).formula() {
        Formula::Expr(f) => String::from(doc.str(f)),
        _ => String::new(),
    }
}

fn row_formula(doc: &Document, s: ShapeRef, addr: CellAddr) -> String {
    match doc.resolve(SheetRef::Shape(s), &addr).formula() {
        Formula::Expr(f) => String::from(doc.str(f)),
        _ => String::new(),
    }
}

fn id(doc: &Document, s: ShapeRef) -> u32 {
    doc.shape(s).unwrap().id()
}

// the page as text: shapes with parents and data, the Connect entries (slots and next IDs left
// out: undo keeps them as they are)
fn dump(doc: &Document, page: PageRef) -> String {
    let mut s = String::new();
    let pg = doc.page(page).unwrap();
    let _ = writeln!(s, "connects {:?}", pg.connects);
    let owner = Owner::Page(page);
    for r in doc.all_shapes(owner) {
        let sh = doc.shape(r).unwrap();
        let kids: Vec<u32> = sh.children().map(|c| doc.shape(c).unwrap().id()).collect();
        let _ = writeln!(s, "shape {} parent {:?} kids {:?} {:?}", sh.id(), sh.parent().map(|p| doc.shape(p).unwrap().id()), kids, sh.data());
    }
    s
}

// undo and redo a committed gesture: undo gives `before` back, redo the state after it
fn undo_redo(doc: &mut Document, page: PageRef, eng: &mut Engine, before: &str, change: &mut Change) {
    let after = dump(doc, page);
    assert!(change.undo(doc), "undo");
    eng.refresh(doc, &change.sheets());
    assert_eq!(dump(doc, page), before, "undo gives the page back");
    assert!(change.redo(doc), "redo");
    eng.refresh(doc, &change.sheets());
    assert_eq!(dump(doc, page), after, "redo gives the gesture back");
}

// run a gesture as one committed change, check undo and redo, leave it applied
fn gesture<T>(f: &mut Fx, eng: &mut Engine, label: &str, run: impl FnOnce(&mut Transaction, &mut Engine) -> Result<T, EditError>) -> T {
    let before = dump(&f.doc, f.page);
    let (out, change) = {
        let mut tx = f.doc.transaction(label);
        let out = run(&mut tx, eng).expect(label);
        (out, tx.commit())
    };
    if let Some(mut c) = change {
        undo_redo(&mut f.doc, f.page, eng, &before, &mut c);
    }
    out
}

fn corners(doc: &Document, s: ShapeRef) -> [[f64; 2]; 4] {
    frame(doc, s).corners()
}

fn page_ends(doc: &Document, s: ShapeRef) -> ([f64; 2], [f64; 2]) {
    let par = parent_xf(doc, s);
    let (b, e) = ends(doc, s);
    (par.apply(b), par.apply(e))
}

fn same_corners(a: [[f64; 2]; 4], b: [[f64; 2]; 4]) {
    for i in 0..4 {
        assert!(close_pt(a[i], b[i]), "corner {i}: {:?} is not {:?}", a[i], b[i]);
    }
}

// ---- text helpers

#[test]
fn numbers_and_formula_texts() {
    assert_eq!(text::num(0.5), "0.5");
    assert_eq!(text::num(0.6734), "0.6734");
    assert_eq!(text::num(1.0 / 3.0), "0.33333333333333");
    assert_eq!(text::num(-2.0), "-2");
    assert_eq!(text::num(1e-20), "0");
    assert_eq!(text::num(12345.678), "12345.678");
    assert_eq!(text::proportional("Sheet.12!Width*0.25"), Some((Some(12), false, 0.25)));
    assert_eq!(text::proportional("Height*0.5"), Some((None, true, 0.5)));
    assert_eq!(text::proportional("Width*0.5+1"), None);
    assert_eq!(text::proportional("GUARD(Width*0.5)"), None);
    assert_eq!(text::call_arg("BOUND(Width*0.5,0,FALSE,0,MIN(Width,2))", "BOUND"), Some((6, 15)));
    assert!(text::calls("IF(A,SETATREF(Width),1)", "SETATREF"));
    assert!(!text::calls("SETATREFEXPR()", "SETATREF"));
    // references with their sheets; strings, numbers with units and function names are no names
    let f = "IF(Sheet.5!Width>0.25DL,Pages[Page-2]!Sheet.5!Height,\"Sheet.5!x\")+'My name'!PinX+Geometry1.X3";
    let it = text::items(f);
    let sheets: Vec<(&str, bool)> = it.iter().filter_map(|i| match i {
        text::Item::Sheet { text, foreign, .. } => Some((*text, *foreign)),
        _ => None,
    }).collect();
    assert_eq!(sheets, alloc::vec![("Sheet.5", false), ("Sheet.5", true), ("'My name'", false)]);
    let cells: Vec<&str> = it.iter().filter_map(|i| match i {
        text::Item::Cell { name, .. } => Some(*name),
        _ => None,
    }).collect();
    assert_eq!(cells, alloc::vec!["Width", "Height", "PinX", "Geometry1.X3"]);
    // renumbering: rows from IX 3 one down, a reference to the own sheet by ID too
    let map = |x: u32| Some(if x >= 3 { x + 1 } else { x });
    let mut gone = |_: &str, _: u32| String::from("0");
    let r = text::renumber_rows("Geometry1.X3+Geometry1.Y2+Geometry2.X3+Sheet.7!Geometry1.A5+Sheet.8!Geometry1.X4", Some(7), "Geometry1", text::Form::Geometry, &map, &mut gone);
    assert_eq!(r, "Geometry1.X4+Geometry1.Y2+Geometry2.X3+Sheet.7!Geometry1.A6+Sheet.8!Geometry1.X4");
    let r = text::renumber_rows("Connections.X3+Connections.Y1", None, "Connections", text::Form::Numbered, &|x| Some(if x >= 1 { x + 1 } else { x }), &mut gone);
    assert_eq!(r, "Connections.X4+Connections.Y1");
    let r = text::renumber_rows("Char.Size+Char.Color[2]", None, "Char", text::Form::Bracket, &|x| Some(x + 1), &mut gone);
    assert_eq!(r, "Char.Size[2]+Char.Color[3]");
    let mut gone = |c: &str, ix: u32| format!("<{c}{ix}>");
    let r = text::renumber_rows("Geometry1.X2*2+Geometry1.X3", None, "Geometry1", text::Form::Geometry, &|x| if x == 2 { None } else if x > 2 { Some(x - 1) } else { Some(x) }, &mut gone);
    assert_eq!(r, "<X2>*2+Geometry1.X2");
}

#[test]
fn lock_bits_follow_the_lock_names() {
    assert_eq!(Locks::bit("LockWidth"), Locks::WIDTH);
    assert_eq!(Locks::bit("LockHeight"), Locks::HEIGHT);
    assert_eq!(Locks::bit("LockAspect"), Locks::ASPECT);
    assert_eq!(Locks::bit("LockMoveX"), Locks::MOVE_X);
    assert_eq!(Locks::bit("LockMoveY"), Locks::MOVE_Y);
    assert_eq!(Locks::bit("LockRotate"), Locks::ROTATE);
    assert_eq!(Locks::bit("LockBegin"), Locks::BEGIN);
    assert_eq!(Locks::bit("LockEnd"), Locks::END);
    assert_eq!(Locks::bit("LockDelete"), Locks::DELETE);
    assert_eq!(Locks::bit("LockSelect"), Locks::SELECT);
    assert_eq!(Locks::bit("LockFormat"), Locks::FORMAT);
    assert_eq!(Locks::bit("LockTextEdit"), Locks::TEXT_EDIT);
    assert_eq!(Locks::bit("LockVtxEdit"), Locks::VTX_EDIT);
    assert_eq!(Locks::bit("LockCrop"), Locks::CROP);
    assert_eq!(Locks::bit("LockGroup"), Locks::GROUP);
    assert_eq!(Locks::bit("LockCalcWH"), Locks::CALC_WH);
    let mut f = fx();
    let a = add_box(&mut f, None, 1, [1.0, 1.0], [1.0, 1.0], 0.0, (false, false));
    let c = cnum(&mut f.doc, "LockMoveX", 1.0);
    set_cell(&mut f, a, c);
    let c = cf(&mut f.doc, "LockRotate", "1=1", 1.0);
    set_cell(&mut f, a, c);
    let l = locks(&mut Engine::new(), &f.doc, a);
    assert!(l.has(Locks::MOVE_X) && l.has(Locks::ROTATE) && !l.has(Locks::WIDTH), "{l:?}");
}

// ---- move

#[test]
fn move_shapes_subshapes_lines_and_locks() {
    let mut f = fx();
    let a = add_box(&mut f, None, 1, [2.0, 3.0], [1.0, 0.5], 0.0, (false, false));
    let g = add_group(&mut f, None, 2, [6.0, 4.0], [4.0, 2.0], 90.0, (false, true));
    let c = add_box(&mut f, Some(g), 3, [1.0, 1.0], [1.0, 0.5], 20.0, (false, false));
    let fx_ = cf(&mut f.doc, "PinX", "Sheet.2!Width*0.25", 1.0);
    set_cell(&mut f, c, fx_);
    let fy_ = cf(&mut f.doc, "PinY", "Sheet.2!Height*0.5", 1.0);
    set_cell(&mut f, c, fy_);
    let l = add_line(&mut f, None, 4, [1.0, 6.0], [3.0, 7.0]);
    connect(&mut f, 4, "BeginX", 1, "PinX");
    let k = add_box(&mut f, None, 5, [8.0, 1.0], [1.0, 1.0], 0.0, (false, false));
    let lock = cnum(&mut f.doc, "LockMoveX", 1.0);
    set_cell(&mut f, k, lock);
    let gd = add_box(&mut f, None, 6, [9.0, 9.0], [1.0, 1.0], 0.0, (false, false));
    let guard = cf(&mut f.doc, "PinX", "GUARD(9)", 9.0);
    set_cell(&mut f, gd, guard);
    let mut eng = Engine::new();
    let c_pin = frame(&f.doc, c).at([0.5, 0.25]);
    let (lb, le) = page_ends(&f.doc, l);
    gesture(&mut f, &mut eng, "Verschieben", |tx, eng| move_shapes(tx, eng, &[a, c, l, k, gd, a], [1.0, -0.5]));
    let d = &f.doc;
    assert_close!(num(d, a, "PinX"), 3.0);
    assert_close!(num(d, a, "PinY"), 2.5);
    assert_eq!(formula(d, a, "PinX"), "");
    // the subshape moved by the page vector through the turned, flipped group; its formulas
    // keep Visio's proportional form
    assert_pt!(frame(d, c).at([0.5, 0.25]), [c_pin[0] + 1.0, c_pin[1] - 0.5]);
    assert!(formula(d, c, "PinX").starts_with("Sheet.2!Width*"), "{}", formula(d, c, "PinX"));
    assert!(formula(d, c, "PinY").starts_with("Sheet.2!Height*"), "{}", formula(d, c, "PinY"));
    // the glued begin stays, the free end moves; the pin follows its formula
    let (nb, ne) = page_ends(d, l);
    assert_pt!(nb, lb);
    assert_pt!(ne, [le[0] + 1.0, le[1] - 0.5]);
    assert_close!(num(d, l, "PinX"), (nb[0] + ne[0]) * 0.5);
    // LockMoveX keeps x; GUARD keeps the cell
    assert_close!(num(d, k, "PinX"), 8.0);
    assert_close!(num(d, k, "PinY"), 0.5);
    assert_close!(num(d, gd, "PinX"), 9.0);
    assert_close!(num(d, gd, "PinY"), 8.5);
}

// ---- size

#[test]
fn every_grip_keeps_the_opposite_edge() {
    let grips = [(-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0)];
    for (deg, flips) in [(0.0, (false, false)), (30.0, (false, false)), (135.0, (true, false)), (-70.0, (false, true)), (200.0, (true, true))] {
        for locpin_const in [false, true] {
            let mut f = fx();
            let s = add_box(&mut f, None, 1, [4.0, 3.0], [2.0, 1.0], deg, flips);
            if locpin_const {
                let c = cnum(&mut f.doc, "LocPinX", 0.3);
                set_cell(&mut f, s, c);
            }
            let mut eng = Engine::new();
            for (gx, gy) in grips {
                let old = frame(&f.doc, s);
                // drag the grip 0.4 out sideways and 0.25 out up or down (local)
                let to = old.at([old.w * (gx + 1) as f64 * 0.5 + 0.4 * gx as f64, old.h * (gy + 1) as f64 * 0.5 + 0.25 * gy as f64]);
                let want = old.drag(gx, gy, to, false);
                assert_close!(want.w, old.w + 0.4 * gx.abs() as f64);
                assert_close!(want.h, old.h + 0.25 * gy.abs() as f64);
                let mut tx = f.doc.transaction("Größe");
                set_frame(&mut tx, &mut eng, s, &want).unwrap();
                let now = frame(tx.doc(), s);
                assert_close!(now.w, want.w);
                assert_close!(now.h, want.h);
                assert_pt!(now.grip(-gx, -gy), old.grip(-gx, -gy));
                assert_pt!(now.grip(gx, gy), to);
                // an edge grip: the opposite edge as a whole stays, both its ends
                if gy == 0 {
                    assert_pt!(now.grip(-gx, -1), old.grip(-gx, -1));
                    assert_pt!(now.grip(-gx, 1), old.grip(-gx, 1));
                }
                if gx == 0 {
                    assert_pt!(now.grip(-1, -gy), old.grip(-1, -gy));
                    assert_pt!(now.grip(1, -gy), old.grip(1, -gy));
                }
                drop(tx);
                eng.refresh(&f.doc, &[SheetRef::Shape(s)]);
            }
        }
    }
}

#[test]
fn set_frame_locks_aspect_and_1d() {
    let mut f = fx();
    let a = add_box(&mut f, None, 1, [2.0, 2.0], [2.0, 1.0], 0.0, (false, false));
    let lw = cnum(&mut f.doc, "LockWidth", 1.0);
    set_cell(&mut f, a, lw);
    let b = add_box(&mut f, None, 2, [6.0, 2.0], [2.0, 1.0], 0.0, (false, false));
    let la = cnum(&mut f.doc, "LockAspect", 1.0);
    set_cell(&mut f, b, la);
    let l = add_line(&mut f, None, 3, [1.0, 5.0], [3.0, 5.0]);
    let mut eng = Engine::new();
    // LockWidth: only the height changes
    let fa = frame(&f.doc, a);
    let want = fa.drag(1, 1, fa.at([3.0, 2.0]), false);
    gesture(&mut f, &mut eng, "Größe", |tx, eng| set_frame(tx, eng, a, &want));
    assert_close!(num(&f.doc, a, "Width"), 2.0);
    assert_close!(num(&f.doc, a, "Height"), 2.0);
    // LockAspect: the side that changed more decides
    let fb = frame(&f.doc, b);
    let want = fb.drag(1, 0, fb.at([3.0, 0.5]), false);
    gesture(&mut f, &mut eng, "Größe", |tx, eng| set_frame(tx, eng, b, &want));
    assert_close!(num(&f.doc, b, "Width"), 3.0);
    assert_close!(num(&f.doc, b, "Height"), 1.5);
    // a corner keeps the proportions when asked
    let fb = frame(&f.doc, b);
    let k = fb.drag(1, 1, fb.at([6.0, 1.6]), true);
    assert_close!(k.w / k.h, 2.0);
    // 1D: the right grip moves the end
    let fl = frame(&f.doc, l);
    let want = fl.drag(1, 0, fl.at([3.0, 0.0]), false);
    gesture(&mut f, &mut eng, "Größe", |tx, eng| set_frame(tx, eng, l, &want));
    let (bp, ep) = page_ends(&f.doc, l);
    assert_pt!(bp, [1.0, 5.0]);
    assert_pt!(ep, [4.0, 5.0]);
    assert_close!(num(&f.doc, l, "Width"), 3.0);
}

#[test]
fn scale_a_selection_into_another_box() {
    let mut f = fx();
    let a = add_box(&mut f, None, 1, [1.0, 1.0], [2.0, 2.0], 0.0, (false, false));
    let b = add_box(&mut f, None, 2, [4.0, 2.5], [2.0, 1.0], 90.0, (false, false));
    let l = add_line(&mut f, None, 3, [0.0, 3.0], [5.0, 0.0]);
    let mut eng = Engine::new();
    let from = bounds(&f.doc, &[a, b, l]).unwrap();
    assert_eq!(from, [0.0, 0.0, 5.0, 3.5]);
    let to = [1.0, 1.0, 11.0, 8.0];
    gesture(&mut f, &mut eng, "Größe", |tx, eng| scale_shapes(tx, eng, &[a, b, l], from, to));
    let d = &f.doc;
    let back = bounds(d, &[a, b, l]).unwrap();
    for i in 0..4 {
        assert_close!(back[i], to[i]);
    }
    // the turned shape grows along its own axes: its width lies up the page
    assert_close!(num(d, b, "Width"), 2.0 * 2.0);
    assert_close!(num(d, b, "Height"), 1.0 * 2.0);
    let (bp, ep) = page_ends(d, l);
    assert_pt!(bp, [1.0, 7.0]);
    assert_pt!(ep, [11.0, 1.0]);
}

// ---- turn, flip, pin

#[test]
fn rotate_about_a_centre() {
    let mut f = fx();
    let a = add_box(&mut f, None, 1, [3.0, 2.0], [2.0, 1.0], 10.0, (true, false));
    let g = add_group(&mut f, None, 2, [7.0, 5.0], [3.0, 2.0], 40.0, (false, true));
    let c = add_box(&mut f, Some(g), 3, [1.0, 0.7], [0.8, 0.4], -15.0, (true, false));
    let l = add_line(&mut f, None, 4, [1.0, 6.0], [2.0, 8.0]);
    let k = add_box(&mut f, None, 5, [9.0, 1.0], [1.0, 1.0], 0.0, (false, false));
    let lock = cnum(&mut f.doc, "LockRotate", 1.0);
    set_cell(&mut f, k, lock);
    let mut eng = Engine::new();
    let centre = [4.0, 4.0];
    let t = 37.0 * DEG;
    let rot = |p: [f64; 2]| {
        let (s, co) = math::sincos(t);
        [centre[0] + (p[0] - centre[0]) * co - (p[1] - centre[1]) * s, centre[1] + (p[0] - centre[0]) * s + (p[1] - centre[1]) * co]
    };
    let before: Vec<[[f64; 2]; 4]> = [a, c, k].iter().map(|&s| corners(&f.doc, s)).collect();
    let (lb, le) = page_ends(&f.doc, l);
    gesture(&mut f, &mut eng, "Drehen", |tx, eng| rotate_shapes(tx, eng, &[a, c, l, k], centre, t));
    let d = &f.doc;
    same_corners(corners(d, a), before[0].map(rot));
    same_corners(corners(d, c), before[1].map(rot));
    same_corners(corners(d, k), before[2]);
    assert_close!(num(d, a, "Angle"), 47.0 * DEG);
    // a subshape of a mirrored group turns the other way in the group's coordinates
    assert_close!(num(d, c, "Angle"), (-15.0 - 37.0) * DEG);
    let (nb, ne) = page_ends(d, l);
    assert_pt!(nb, rot(lb));
    assert_pt!(ne, rot(le));
}

#[test]
fn flip_turned_shapes_subshapes_and_lines() {
    for horizontal in [true, false] {
        let mut f = fx();
        let a = add_box(&mut f, None, 1, [3.0, 2.0], [2.0, 1.0], 30.0, (false, false));
        let g = add_group(&mut f, None, 2, [7.0, 5.0], [3.0, 2.0], 40.0, (true, false));
        let c = add_box(&mut f, Some(g), 3, [1.0, 0.7], [0.8, 0.4], -15.0, (false, true));
        let l = add_line(&mut f, None, 4, [1.0, 6.0], [2.0, 8.0]);
        let mut eng = Engine::new();
        let bx = bounds(&f.doc, &[a, c, l]).unwrap();
        let mid = [(bx[0] + bx[2]) * 0.5, (bx[1] + bx[3]) * 0.5];
        let mirror = |p: [f64; 2]| if horizontal { [2.0 * mid[0] - p[0], p[1]] } else { [p[0], 2.0 * mid[1] - p[1]] };
        let ca = corners(&f.doc, a);
        let cc = corners(&f.doc, c);
        let (lb, le) = page_ends(&f.doc, l);
        gesture(&mut f, &mut eng, "Spiegeln", |tx, eng| flip_shapes(tx, eng, &[a, c, l], horizontal));
        let d = &f.doc;
        // every local point lands on its mirror image (the transform is the mirror times the old)
        same_corners(corners(d, a), ca.map(mirror));
        same_corners(corners(d, c), cc.map(mirror));
        let (nb, ne) = page_ends(d, l);
        assert_pt!(nb, mirror(lb));
        assert_pt!(ne, mirror(le));
        // Visio: a turned shape flipped horizontally has FlipX and the negative angle
        if horizontal {
            assert_close!(num(d, a, "FlipX"), 1.0);
            assert_close!(num(d, a, "FlipY"), 0.0);
        } else {
            assert_close!(num(d, a, "FlipX"), 0.0);
            assert_close!(num(d, a, "FlipY"), 1.0);
        }
        assert_close!(num(d, a, "Angle"), -30.0 * DEG);
        assert_close!(num(d, l, "FlipY"), 1.0);
    }
}

#[test]
fn turning_and_flipping_a_group_takes_its_members_along() {
    let mut f = fx();
    let g = add_group(&mut f, None, 1, [5.0, 5.0], [3.0, 2.0], 20.0, (false, false));
    let c = add_box(&mut f, Some(g), 2, [1.0, 0.6], [1.0, 0.5], 35.0, (true, false));
    let l = add_line(&mut f, Some(g), 3, [0.2, 0.2], [2.5, 1.8]);
    let mut eng = Engine::new();
    let (cc, (lb, le)) = (corners(&f.doc, c), page_ends(&f.doc, l));
    let t = -64.0 * DEG;
    let centre = [1.0, 2.0];
    let (s, co) = math::sincos(t);
    let rot = |p: [f64; 2]| [centre[0] + (p[0] - centre[0]) * co - (p[1] - centre[1]) * s, centre[1] + (p[0] - centre[0]) * s + (p[1] - centre[1]) * co];
    gesture(&mut f, &mut eng, "Drehen", |tx, eng| rotate_shapes(tx, eng, &[g, c], centre, t));
    same_corners(corners(&f.doc, c), cc.map(rot));
    assert_pt!(page_ends(&f.doc, l).0, rot(lb));
    assert_pt!(page_ends(&f.doc, l).1, rot(le));
    assert_close!(num(&f.doc, c, "Angle"), 35.0 * DEG);
    let (cc, lb) = (corners(&f.doc, c), page_ends(&f.doc, l).0);
    let bx = frame(&f.doc, g).bounds();
    let mx = (bx[0] + bx[2]) * 0.5;
    gesture(&mut f, &mut eng, "Spiegeln", |tx, eng| flip_shapes(tx, eng, &[g], true));
    same_corners(corners(&f.doc, c), cc.map(|p| [2.0 * mx - p[0], p[1]]));
    assert_pt!(page_ends(&f.doc, l).0, [2.0 * mx - lb[0], lb[1]]);
    assert_close!(num(&f.doc, g, "FlipX"), 1.0);
    assert_close!(num(&f.doc, c, "FlipX"), 1.0);
}

#[test]
fn group_inside_a_group_and_setatref() {
    let mut f = fx();
    let outer = add_group(&mut f, None, 1, [5.0, 5.0], [4.0, 4.0], 30.0, (false, true));
    let a = add_box(&mut f, Some(outer), 2, [1.0, 1.0], [1.0, 0.5], 10.0, (false, false));
    let b = add_box(&mut f, Some(outer), 3, [3.0, 2.5], [0.5, 0.5], 0.0, (false, false));
    let keep = add_box(&mut f, Some(outer), 4, [2.0, 3.5], [0.5, 0.5], 0.0, (false, false));
    let mut eng = Engine::new();
    let (ca, cb, ck_) = (corners(&f.doc, a), corners(&f.doc, b), corners(&f.doc, keep));
    let owner = f.owner;
    let g = gesture(&mut f, &mut eng, "Gruppieren", |tx, eng| group(tx, eng, owner, &[b, a]));
    let d = &f.doc;
    assert_eq!(d.shape(g).unwrap().parent(), Some(outer));
    // the members lay below `keep`: so does the group
    let kids: Vec<u32> = d.shape(outer).unwrap().children().map(|s| id(d, s)).collect();
    assert_eq!(kids, alloc::vec![id(d, g), 4]);
    same_corners(corners(d, a), ca);
    same_corners(corners(d, b), cb);
    same_corners(corners(d, keep), ck_);
    // SETATREF sends a size to the cell it names
    let sa = add_box(&mut f, None, 9, [1.0, 1.0], [2.0, 1.0], 0.0, (false, false));
    {
        let doc = &mut f.doc;
        let w = cf(doc, "Width", "SETATREF(User.W)", 2.0);
        let mut user = Section::new(SecKey::one(names::USER));
        let mut ur = Row::new(RowKey::Named(doc.intern("W")));
        ur.cells = alloc::vec![cnum(doc, "Value", 2.0)];
        user.rows.push(ur);
        let sh = doc.sheet_mut(SheetRef::Shape(sa)).unwrap();
        sh.cells.retain(|c| c.name != names::WIDTH);
        sh.cells.push(w);
        sh.sections.push(user);
    }
    let fa = frame(&f.doc, sa);
    let want = fa.drag(1, 0, fa.at([3.0, 0.5]), false);
    gesture(&mut f, &mut eng, "Größe", |tx, eng| set_frame(tx, eng, sa, &want));
    let d = &f.doc;
    let wa = CellAddr::row(SecKey::one(names::USER), RowKey::Named(d.sym("W").unwrap()), names::VALUE);
    assert_close!(d.resolve(SheetRef::Shape(sa), &wa).num().unwrap(), 3.0);
    assert_eq!(formula(d, sa, "Width"), "SETATREF(User.W)");
    assert_close!(num(d, sa, "Width"), 3.0);
    assert_pt!(frame(d, sa).grip(-1, 0), fa.grip(-1, 0));
}

#[test]
fn set_pin_leaves_the_shape_in_place() {
    let mut f = fx();
    let a = add_box(&mut f, None, 1, [3.0, 2.0], [2.0, 1.0], 25.0, (true, false));
    let mut eng = Engine::new();
    let before = corners(&f.doc, a);
    let at = frame(&f.doc, a).at([0.5, 0.25]);
    gesture(&mut f, &mut eng, "Drehpunkt", |tx, eng| set_pin(tx, eng, a, at));
    let d = &f.doc;
    same_corners(corners(d, a), before);
    assert_pt!([num(d, a, "PinX"), num(d, a, "PinY")], at);
    assert_eq!(formula(d, a, "LocPinX"), "Width*0.25");
    assert_eq!(formula(d, a, "LocPinY"), "Height*0.25");
}

// ---- PAR and LOC against the transforms of geometry.rs

fn probe() -> CellAddr {
    CellAddr::row(SecKey::one(names::USER), RowKey::Named(names::LABEL), names::VALUE)
}

fn point(eng: &mut Engine, doc: &Document, s: ShapeRef, text: &str) -> [f64; 2] {
    match eng.evaluate_text(doc, SheetRef::Shape(s), &probe(), text) {
        Ok(Outcome::Value(Val::Point(x, y))) => [x, y],
        other => panic!("{text}: {other:?}"),
    }
}

#[test]
fn par_and_loc_agree_with_page_xform_for_turned_and_flipped_shapes() {
    let mut f = fx();
    // a shape whose local coordinates are the page's
    let t = add_box(&mut f, None, 10, [0.0, 0.0], [1.0, 1.0], 0.0, (false, false));
    let z = cnum(&mut f.doc, "LocPinX", 0.0);
    set_cell(&mut f, t, z);
    let z = cnum(&mut f.doc, "LocPinY", 0.0);
    set_cell(&mut f, t, z);
    let g = add_group(&mut f, None, 20, [5.0, 4.0], [4.0, 2.0], 30.0, (true, false));
    let s = add_box(&mut f, Some(g), 21, [1.0, 0.5], [1.0, 0.6], 45.0, (false, true));
    let lp = cnum(&mut f.doc, "LocPinX", 0.2);
    set_cell(&mut f, s, lp);
    let lp = cnum(&mut f.doc, "LocPinY", 0.1);
    set_cell(&mut f, s, lp);
    let g3 = add_group(&mut f, Some(g), 22, [3.0, 1.5], [1.5, 1.0], -60.0, (true, true));
    let s3 = add_box(&mut f, Some(g3), 23, [0.5, 0.4], [0.6, 0.3], 10.0, (false, false));
    let s2 = add_box(&mut f, None, 30, [8.0, 2.0], [2.0, 1.0], 120.0, (true, true));
    let mut eng = Engine::new();
    let doc = &f.doc;
    for x in [s, s2, s3, g, g3] {
        let xid = id(doc, x);
        let xf = doc.page_xform(x);
        let (w, h) = (num(doc, x, "Width"), num(doc, x, "Height"));
        for (kx, ky) in [(0.3, 0.8), (1.0, 0.0), (0.0, 1.0), (0.5, 0.5)] {
            // local -> page: PAR seen from the page-aligned top-level shape
            let p = point(&mut eng, doc, t, &format!("PAR(PNT(Sheet.{xid}!Width*{kx},Sheet.{xid}!Height*{ky}))"));
            assert_pt!(p, xf.apply([w * kx, h * ky]));
            // page -> local: LOC of a point given in the page-aligned shape
            let q = [1.7 + kx, 2.9 - ky];
            let l = point(&mut eng, doc, x, &format!("LOC(PNT(Sheet.10!Width*0+{},Sheet.10!Height*0+{}))", q[0], q[1]));
            assert_pt!(l, xf.inverse().unwrap().apply(q));
        }
        // PAR of the shape's own point: into its parent's coordinates
        let lx = doc.local_xform(SheetRef::Shape(x));
        assert_pt!(point(&mut eng, doc, x, "PAR(PNT(Width*0.3,Height*0.8))"), lx.apply([w * 0.3, h * 0.8]));
    }
    // and the frame's corners are where the transform puts them
    let fr = frame(doc, s3);
    assert_pt!(fr.corners()[2], doc.page_xform(s3).apply([0.6, 0.3]));
}

// ---- groups

#[test]
fn group_and_ungroup_keep_every_corner() {
    let mut f = fx();
    let a = add_box(&mut f, None, 1, [2.0, 2.0], [2.0, 1.0], 30.0, (true, false));
    let e = add_box(&mut f, None, 4, [8.0, 8.0], [1.0, 1.0], 0.0, (false, false));
    let b = add_box(&mut f, None, 2, [5.0, 3.0], [1.0, 1.0], 0.0, (false, false));
    let l = add_line(&mut f, None, 3, [1.0, 5.0], [4.0, 6.0]);
    let mut eng = Engine::new();
    let (ca, cb) = (corners(&f.doc, a), corners(&f.doc, b));
    let (lb, le) = page_ends(&f.doc, l);
    let owner = f.owner;
    let g = gesture(&mut f, &mut eng, "Gruppieren", |tx, eng| group(tx, eng, owner, &[l, a, b]));
    {
        let d = &f.doc;
        same_corners(corners(d, a), ca);
        same_corners(corners(d, b), cb);
        let (nb, ne) = page_ends(d, l);
        assert_pt!(nb, lb);
        assert_pt!(ne, le);
        // the group: the hull of the members, at the place of the topmost member
        let mut hull = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for p in ca.iter().chain(cb.iter()).chain([lb, le].iter()) {
            hull = [hull[0].min(p[0]), hull[1].min(p[1]), hull[2].max(p[0]), hull[3].max(p[1])];
        }
        let gb = frame(d, g).bounds();
        for i in 0..4 {
            assert_close!(gb[i], hull[i]);
        }
        let top: Vec<u32> = d.top_shapes(f.owner).map(|s| id(d, s)).collect();
        let gid = id(d, g);
        assert_eq!(top, alloc::vec![4, gid]);
        let kids: Vec<u32> = d.shape(g).unwrap().children().map(|s| id(d, s)).collect();
        assert_eq!(kids, alloc::vec![1, 2, 3]);
        assert_eq!(d.shape(g).unwrap().header().kind, ShapeKind::Group);
        assert_eq!(d.shape(g).unwrap().header().styles, StyleRefs::all(1));
        assert!(formula(d, a, "PinX").starts_with(&format!("Sheet.{gid}!Width*")));
        assert!(formula(d, b, "PinY").starts_with(&format!("Sheet.{gid}!Height*")));
        assert!(formula(d, b, "Width").starts_with(&format!("Sheet.{gid}!Width*")));
        // a turned member keeps its size as a value
        assert_eq!(formula(d, a, "Width"), "");
        assert!(formula(d, l, "BeginX").starts_with(&format!("Sheet.{gid}!Width*")));
        assert_eq!(formula(d, g, "LocPinX"), "Width*0.5");
        let _ = e;
    }
    // the group scales its members through their formulas
    {
        let fg = frame(&f.doc, g);
        let want = fg.drag(1, 0, fg.at([fg.w * 2.0, 0.0]), false);
        let x0 = fg.bounds()[0];
        let mut tx = f.doc.transaction("Größe");
        set_frame(&mut tx, &mut eng, g, &want).unwrap();
        let nb = corners(tx.doc(), b);
        for i in 0..4 {
            assert_pt!(nb[i], [x0 + (cb[i][0] - x0) * 2.0, cb[i][1]]);
        }
        let (nlb, _) = page_ends(tx.doc(), l);
        assert_pt!(nlb, [x0 + (lb[0] - x0) * 2.0, lb[1]]);
        drop(tx);
        eng.refresh(&f.doc, &[SheetRef::Shape(g), SheetRef::Shape(a), SheetRef::Shape(b), SheetRef::Shape(l)]);
    }
    // ungroup: the members go back at the group's place, every corner stays
    let kids = gesture(&mut f, &mut eng, "Gruppierung aufheben", |tx, eng| ungroup(tx, eng, g));
    let d = &f.doc;
    assert_eq!(kids, alloc::vec![a, b, l]);
    assert!(d.shape(g).is_none());
    same_corners(corners(d, a), ca);
    same_corners(corners(d, b), cb);
    let (nb, ne) = page_ends(d, l);
    assert_pt!(nb, lb);
    assert_pt!(ne, le);
    let top: Vec<u32> = d.top_shapes(f.owner).map(|s| id(d, s)).collect();
    assert_eq!(top, alloc::vec![4, 1, 2, 3]);
    assert_eq!(formula(d, a, "PinX"), "");
    assert_eq!(formula(d, b, "Width"), "");
}

#[test]
fn ungroup_a_turned_mirrored_group_and_group_it_again() {
    let mut f = fx();
    let g0 = add_group(&mut f, None, 10, [5.0, 5.0], [3.0, 2.0], 45.0, (false, true));
    let c = add_box(&mut f, Some(g0), 11, [1.0, 0.6], [1.0, 0.5], 20.0, (true, false));
    let fxc = cf(&mut f.doc, "PinX", "Sheet.10!Width*0.33333333333333", 1.0);
    set_cell(&mut f, c, fxc);
    let c2 = add_box(&mut f, Some(g0), 12, [2.2, 1.4], [0.6, 0.6], 0.0, (false, false));
    let other = add_box(&mut f, None, 13, [9.0, 2.0], [1.0, 1.0], 80.0, (false, false));
    // glue from outside to the group goes when the group goes
    let l = add_line(&mut f, None, 14, [0.5, 0.5], [5.0, 5.0]);
    let ex = cf(&mut f.doc, "EndX", "Sheet.10!PinX", 5.0);
    set_cell(&mut f, l, ex);
    connect(&mut f, 14, "EndX", 10, "PinX");
    let mut eng = Engine::new();
    let (cc, cc2, co) = (corners(&f.doc, c), corners(&f.doc, c2), corners(&f.doc, other));
    let kids = gesture(&mut f, &mut eng, "Gruppierung aufheben", |tx, eng| ungroup(tx, eng, g0));
    {
        let d = &f.doc;
        same_corners(corners(d, c), cc);
        same_corners(corners(d, c2), cc2);
        // one flip of the group: the angle runs the other way, the flips add up
        assert_close!(num(d, c, "Angle"), 25.0 * DEG);
        assert_close!(num(d, c, "FlipX"), 1.0);
        assert_close!(num(d, c, "FlipY"), 1.0);
        assert_eq!(formula(d, l, "EndX"), "");
        assert!(d.page(f.page).unwrap().connects.is_empty());
        assert_eq!(kids.len(), 2);
    }
    // group the turned members with a turned shape, then take that group apart again
    let owner = f.owner;
    let g = gesture(&mut f, &mut eng, "Gruppieren", |tx, eng| group(tx, eng, owner, &[c, c2, other]));
    same_corners(corners(&f.doc, c), cc);
    same_corners(corners(&f.doc, other), co);
    gesture(&mut f, &mut eng, "Gruppierung aufheben", |tx, eng| ungroup(tx, eng, g));
    same_corners(corners(&f.doc, c), cc);
    same_corners(corners(&f.doc, c2), cc2);
    same_corners(corners(&f.doc, other), co);
}

#[test]
fn group_refuses_mixed_parents_and_locked_ungroup() {
    let mut f = fx();
    let g = add_group(&mut f, None, 1, [5.0, 5.0], [2.0, 2.0], 0.0, (false, false));
    let c = add_box(&mut f, Some(g), 2, [1.0, 1.0], [1.0, 1.0], 0.0, (false, false));
    let a = add_box(&mut f, None, 3, [1.0, 1.0], [1.0, 1.0], 0.0, (false, false));
    let lock = cnum(&mut f.doc, "LockGroup", 1.0);
    set_cell(&mut f, g, lock);
    let mut eng = Engine::new();
    let mut tx = f.doc.transaction("x");
    assert_eq!(group(&mut tx, &mut eng, Owner::Page(f.page), &[a, c]), Err(EditError::Invalid));
    assert_eq!(ungroup(&mut tx, &mut eng, g), Err(EditError::Invalid));
    assert_eq!(ungroup(&mut tx, &mut eng, a), Err(EditError::Invalid));
    assert!(tx.commit().is_none());
}

// ---- z-order

#[test]
fn reorder_front_back_forward_backward() {
    let mut f = fx();
    let s: Vec<ShapeRef> = (1..=5).map(|i| add_box(&mut f, None, i, [i as f64, 1.0], [0.5, 0.5], 0.0, (false, false))).collect();
    let order = |f: &Fx| -> Vec<u32> { f.doc.top_shapes(f.owner).map(|r| id(&f.doc, r)).collect() };
    let mut eng = Engine::new();
    gesture(&mut f, &mut eng, "Vordergrund", |tx, _| reorder(tx, &[s[1], s[3]], Z::Front));
    assert_eq!(order(&f), alloc::vec![1, 3, 5, 2, 4]);
    gesture(&mut f, &mut eng, "Hintergrund", |tx, _| reorder(tx, &[s[4], s[3]], Z::Back));
    assert_eq!(order(&f), alloc::vec![5, 4, 1, 3, 2]);
    gesture(&mut f, &mut eng, "Nach vorne", |tx, _| reorder(tx, &[s[4], s[3]], Z::Forward));
    assert_eq!(order(&f), alloc::vec![1, 5, 4, 3, 2]);
    gesture(&mut f, &mut eng, "Nach hinten", |tx, _| reorder(tx, &[s[2], s[1]], Z::Backward));
    assert_eq!(order(&f), alloc::vec![1, 5, 3, 2, 4]);
    // on top already: nothing changes, no step
    let mut tx = f.doc.transaction("z");
    reorder(&mut tx, &[s[3]], Z::Front).unwrap();
    assert!(tx.commit().is_none());
}

// ---- rows in the middle of a section

fn geo_row(doc: &mut Document, ix: u32, kind: &str, x: f64, y: f64) -> Row {
    let mut r = Row::new(RowKey::Index(ix));
    r.kind = Some(doc.intern(kind));
    r.cells = alloc::vec![cnum(doc, "X", x), cnum(doc, "Y", y)];
    r
}

fn gaddr(ix: u32, cell: Sym) -> CellAddr {
    CellAddr::row(SecKey { name: names::GEOMETRY, index: 0 }, RowKey::Index(ix), cell)
}

fn path_points(doc: &Document, s: ShapeRef) -> Vec<[f64; 2]> {
    let sheet = SheetRef::Shape(s);
    let (w, h) = (num(doc, s, "Width"), num(doc, s, "Height"));
    let mut out = Vec::new();
    for g in doc.geometry_paths(sheet, w, h) {
        for sub in &g.path.subs {
            out.push(sub.start);
            for seg in &sub.segs {
                out.push(seg.end());
            }
        }
    }
    out
}

#[test]
fn insert_and_remove_rows_renumber_and_follow_formulas() {
    let mut f = fx();
    let s = add_box(&mut f, None, 1, [3.0, 3.0], [2.0, 2.0], 0.0, (false, false));
    {
        let doc = &mut f.doc;
        let mut sec = Section::new(SecKey { name: names::GEOMETRY, index: 0 });
        sec.ix_written = true;
        sec.rows.push(geo_row(doc, 1, "MoveTo", 0.0, 0.0));
        sec.rows.push(geo_row(doc, 2, "LineTo", 2.0, 0.0));
        sec.rows.push(geo_row(doc, 3, "LineTo", 2.0, 2.0));
        sec.rows.push(geo_row(doc, 4, "LineTo", 0.0, 2.0));
        let mut close_row = Row::new(RowKey::Index(5));
        close_row.kind = Some(doc.intern("LineTo"));
        close_row.cells = alloc::vec![cf(doc, "X", "Geometry1.X1", 0.0), cf(doc, "Y", "Geometry1.Y1", 0.0)];
        sec.rows.push(close_row);
        let mut user = Section::new(SecKey::one(names::USER));
        let mut ur = Row::new(RowKey::Named(doc.intern("Probe")));
        ur.cells = alloc::vec![cf(doc, "Value", "Geometry1.Y3*2+Geometry1.X4", 4.0)];
        user.rows.push(ur);
        let d = doc.sheet_mut(SheetRef::Shape(s)).unwrap();
        d.sections.push(sec);
        d.sections.push(user);
    }
    let mut eng = Engine::new();
    let gk = SecKey { name: names::GEOMETRY, index: 0 };
    let probe_addr = CellAddr::row(SecKey::one(names::USER), RowKey::Named(f.doc.sym("Probe").unwrap()), names::VALUE);
    let sheet = SheetRef::Shape(s);
    let before = dump(&f.doc, f.page);
    // a row at IX 3: the rows from there move down, the formulas follow
    let mut change = {
        let mut tx = f.doc.transaction("Scheitelpunkt");
        let k = tx.insert_row(sheet, gk, RowKey::Index(3), Some("LineTo")).unwrap();
        assert_eq!(k, RowKey::Index(3));
        let x = tx.intern("X");
        let y = tx.intern("Y");
        tx.set_constant(sheet, &gaddr(3, x), Value::Num(3.0), None).unwrap();
        tx.set_constant(sheet, &gaddr(3, y), Value::Num(1.0), None).unwrap();
        eng.recalc(&mut tx, &[sheet]).unwrap();
        tx.commit().unwrap()
    };
    {
        let d = &f.doc;
        let x = d.sym("X").unwrap();
        assert_eq!(row_formula(d, s, gaddr(6, x)), "Geometry1.X1");
        assert_eq!(row_formula(d, s, probe_addr), "Geometry1.Y4*2+Geometry1.X5");
        assert_close!(d.resolve(sheet, &probe_addr).num().unwrap(), 4.0);
        assert_eq!(path_points(d, s), alloc::vec![[0.0, 0.0], [2.0, 0.0], [3.0, 1.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]]);
    }
    undo_redo(&mut f.doc, f.page, &mut eng, &before, &mut change);
    assert!(change.undo(&mut f.doc));
    eng.refresh(&f.doc, &change.sheets());
    // take row 3 out: references to it become its values, the rows after it move up
    let mut change = {
        let mut tx = f.doc.transaction("Scheitelpunkt löschen");
        tx.remove_row(sheet, gk, RowKey::Index(3)).unwrap();
        eng.recalc(&mut tx, &[sheet]).unwrap();
        tx.commit().unwrap()
    };
    {
        let d = &f.doc;
        let x = d.sym("X").unwrap();
        assert_eq!(row_formula(d, s, gaddr(4, x)), "Geometry1.X1");
        assert_eq!(row_formula(d, s, probe_addr), "2*2+Geometry1.X3");
        assert_eq!(path_points(d, s), alloc::vec![[0.0, 0.0], [2.0, 0.0], [0.0, 2.0], [0.0, 0.0]]);
        assert!(!d.has_row(sheet, gk, RowKey::Index(5)));
    }
    undo_redo(&mut f.doc, f.page, &mut eng, &before, &mut change);
}

#[test]
fn insert_row_into_an_inherited_section_makes_it_local() {
    let mut f = fx();
    // a master with a square of four rows; an instance that inherits it and moves row 2
    let mut mh = crate::MasterHeader { id: 2, ..crate::MasterHeader::default() };
    mh.name_u = Some(f.doc.intern("Square"));
    let m = f.doc.insert_master(crate::Master::new(mh)).unwrap();
    let mut ms = box_data(&mut f.doc, ShapeKind::Shape, [0.5, 0.5], [1.0, 1.0], 0.0, (false, false));
    {
        let doc = &mut f.doc;
        let mut sec = Section::new(SecKey { name: names::GEOMETRY, index: 0 });
        sec.ix_written = true;
        sec.rows.push(geo_row(doc, 1, "MoveTo", 0.0, 0.0));
        sec.rows.push(geo_row(doc, 2, "LineTo", 1.0, 0.0));
        sec.rows.push(geo_row(doc, 3, "LineTo", 1.0, 1.0));
        sec.rows.push(geo_row(doc, 4, "LineTo", 0.0, 0.0));
        ms.sheet.sections.push(sec);
    }
    f.doc.master_mut(m).unwrap().shapes.insert(None, 5, ms).unwrap();
    let mut inst = ShapeData::new(ShapeKind::Shape);
    inst.header.master = Some(2);
    inst.sheet.cells.push(cnum(&mut f.doc, "PinX", 4.0));
    inst.sheet.cells.push(cnum(&mut f.doc, "PinY", 4.0));
    {
        let doc = &mut f.doc;
        let mut sec = Section::new(SecKey { name: names::GEOMETRY, index: 0 });
        let mut r = Row::new(RowKey::Index(2));
        r.cells = alloc::vec![cnum(doc, "X", 0.8)];
        sec.rows.push(r);
        inst.sheet.sections.push(sec);
    }
    let s = insert(&mut f, None, 1, inst);
    let mut eng = Engine::new();
    let gk = SecKey { name: names::GEOMETRY, index: 0 };
    let sheet = SheetRef::Shape(s);
    let master_before = format!("{:?}", f.doc.master(m).unwrap().shapes);
    let at = frame(&f.doc, s).at([0.5, 0.0]);
    let ix = gesture(&mut f, &mut eng, "Scheitelpunkt", |tx, eng| insert_vertex(tx, eng, s, 0, 1, at));
    assert_eq!(ix, 2);
    let d = &f.doc;
    assert_eq!(path_points(d, s), alloc::vec![[0.0, 0.0], [0.5, 0.0], [0.8, 0.0], [1.0, 1.0], [0.0, 0.0]]);
    // the instance has every row as its own now, with every cell; the master is unchanged
    let own = d.sheet(sheet).unwrap().section(gk).unwrap();
    assert_eq!(own.rows.len(), 5);
    assert!(own.rows.iter().all(|r| !r.del && r.kind.is_some() && r.cells.len() == 2));
    assert_eq!(format!("{:?}", d.master(m).unwrap().shapes), master_before);
    // taking two rows out leaves Del marks for the master's rows the instance no longer has
    gesture(&mut f, &mut eng, "Scheitelpunkt löschen", |tx, eng| delete_vertex(tx, eng, s, 0, 2));
    gesture(&mut f, &mut eng, "Scheitelpunkt löschen", |tx, eng| delete_vertex(tx, eng, s, 0, 2));
    let d = &f.doc;
    assert_eq!(path_points(d, s), alloc::vec![[0.0, 0.0], [1.0, 1.0], [0.0, 0.0]]);
    let own = d.sheet(sheet).unwrap().section(gk).unwrap();
    assert_eq!(own.rows.iter().filter(|r| r.del).map(|r| r.key).collect::<Vec<_>>(), alloc::vec![RowKey::Index(4)]);
}

// ---- control points

fn control_row(doc: &mut Document, name: &str, cells: Vec<Cell>) -> Row {
    let mut r = Row::new(RowKey::Named(doc.intern(name)));
    r.cells = cells;
    r
}

fn ck(doc: &Document, name: &str, cell: Sym) -> CellAddr {
    CellAddr::row(SecKey::one(names::CONTROL), RowKey::Named(doc.sym(name).unwrap()), cell)
}

#[test]
fn control_points_write_cells_as_visio_does() {
    let mut f = fx();
    let s = add_box(&mut f, None, 1, [3.0, 3.0], [2.0, 1.0], 90.0, (false, false));
    let target = add_box(&mut f, None, 2, [8.0, 8.0], [1.0, 1.0], 0.0, (false, false));
    {
        let doc = &mut f.doc;
        let mut sec = Section::new(SecKey::one(names::CONTROL));
        let r1 = alloc::vec![cf(doc, "X", "Width*0.25", 0.5), cf(doc, "Y", "Height*0.5", 0.5), cnum(doc, "XCon", 0.0), cnum(doc, "YCon", 0.0), cnum(doc, "CanGlue", 1.0)];
        sec.rows.push(control_row(doc, "Row_1", r1));
        let r2 = alloc::vec![cnum(doc, "X", 1.5), cnum(doc, "Y", 0.2), cnum(doc, "XCon", 4.0), cnum(doc, "YCon", 2.0)];
        sec.rows.push(control_row(doc, "Row_2", r2));
        let r3 = alloc::vec![cf(doc, "X", "BOUND(Width*0.5,0,FALSE,0,Width)", 1.0), cnum(doc, "Y", 0.0), cnum(doc, "XCon", 0.0), cnum(doc, "YCon", 1.0)];
        sec.rows.push(control_row(doc, "Row_3", r3));
        let r4 = alloc::vec![cnum(doc, "X", 0.0), cnum(doc, "Y", 0.0), cnum(doc, "XCon", 5.0)];
        sec.rows.push(control_row(doc, "Hidden", r4));
        let r5 = alloc::vec![cf(doc, "X", "LOC(PNT(Sheet.2!PinX,Sheet.2!PinY))", 5.0), cf(doc, "Y", "LOC(PNT(Sheet.2!PinX,Sheet.2!PinY))", -5.0), cnum(doc, "XCon", 3.0)];
        sec.rows.push(control_row(doc, "Glued", r5));
        doc.sheet_mut(SheetRef::Shape(s)).unwrap().sections.push(sec);
    }
    connect(&mut f, 1, "Controls.Glued", 2, "PinX");
    let _ = target;
    let mut eng = Engine::new();
    let list = controls(&f.doc, s);
    let names_: Vec<&str> = list.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names_, alloc::vec!["Row_1", "Row_2", "Row_3", "Glued"]);
    let xf = frame(&f.doc, s);
    assert_pt!(list[0].at, xf.at([0.5, 0.5]));
    assert!(list[0].can_glue && !list[0].x_locked && list[2].y_locked);
    let row = |name: &str| list.iter().find(|c| c.name == name).unwrap().row;
    let (r1, r2, r3, rg) = (row("Row_1"), row("Row_2"), row("Row_3"), row("Glued"));
    // proportional: Width*k, Height*k
    gesture(&mut f, &mut eng, "Steuerpunkt", |tx, eng| set_control(tx, eng, s, r1, xf.at([1.3468, 0.8])));
    assert_eq!(row_formula(&f.doc, s, ck(&f.doc, "Row_1", names::X)), "Width*0.6734");
    assert_eq!(row_formula(&f.doc, s, ck(&f.doc, "Row_1", names::Y)), "Height*0.8");
    assert_pt!(controls(&f.doc, s)[0].at, xf.at([1.3468, 0.8]));
    // offset from the right: Width-d DL; offset from the bottom: the value
    gesture(&mut f, &mut eng, "Steuerpunkt", |tx, eng| set_control(tx, eng, s, r2, xf.at([1.75, 0.4])));
    assert_eq!(row_formula(&f.doc, s, ck(&f.doc, "Row_2", names::X)), "Width-0.25DL");
    assert_eq!(row_formula(&f.doc, s, ck(&f.doc, "Row_2", names::Y)), "");
    assert_close!(f.doc.resolve(SheetRef::Shape(s), &ck(&f.doc, "Row_2", names::Y)).num().unwrap(), 0.4);
    // BOUND keeps its form; a locked direction stays
    gesture(&mut f, &mut eng, "Steuerpunkt", |tx, eng| set_control(tx, eng, s, r3, xf.at([1.2, 0.7])));
    assert_eq!(row_formula(&f.doc, s, ck(&f.doc, "Row_3", names::X)), "BOUND(Width*0.6,0,FALSE,0,Width)");
    assert_close!(f.doc.resolve(SheetRef::Shape(s), &ck(&f.doc, "Row_3", names::Y)).num().unwrap(), 0.0);
    // a glued point is let go; offset from the middle
    gesture(&mut f, &mut eng, "Steuerpunkt", |tx, eng| set_control(tx, eng, s, rg, xf.at([0.9, 0.3])));
    assert_eq!(row_formula(&f.doc, s, ck(&f.doc, "Glued", names::X)), "Width*0.5-0.1DL");
    assert!(f.doc.page(f.page).unwrap().connects.is_empty());
    assert_close!(f.doc.resolve(SheetRef::Shape(s), &ck(&f.doc, "Glued", names::X)).num().unwrap(), 0.9);
}

// ---- vertices

#[test]
fn vertices_move_insert_delete_with_closing_rows() {
    let mut f = fx();
    let mut eng = Engine::new();
    let owner = f.owner;
    let r = gesture(&mut f, &mut eng, "Rechteck", |tx, eng| new_shape(tx, eng, owner, Basic::Rectangle, [1.0, 1.0], [3.0, 2.0]));
    let v = vertices(&f.doc, r);
    assert_eq!(v.len(), 4);
    assert_pt!(v[0].at, [1.0, 1.0]);
    assert_pt!(v[2].at, [3.0, 2.0]);
    // the start takes its closing row along (relative rows: fractions)
    gesture(&mut f, &mut eng, "Scheitelpunkt", |tx, eng| set_vertex(tx, eng, r, 0, 1, [1.5, 1.0]));
    let x = f.doc.sym("X").unwrap();
    assert_close!(f.doc.resolve(SheetRef::Shape(r), &gaddr(1, x)).num().unwrap(), 0.25);
    assert_close!(f.doc.resolve(SheetRef::Shape(r), &gaddr(5, x)).num().unwrap(), 0.25);
    assert_eq!(path_points(&f.doc, r), alloc::vec![[0.5, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0], [0.5, 0.0]]);
    // a vertex on the top edge
    let ix = gesture(&mut f, &mut eng, "Scheitelpunkt hinzufügen", |tx, eng| insert_vertex(tx, eng, r, 0, 3, [2.0, 2.5]));
    assert_eq!(ix, 4);
    assert_eq!(vertices(&f.doc, r).len(), 5);
    let kind = f.doc.rows(SheetRef::Shape(r), SecKey { name: names::GEOMETRY, index: 0 })[3].kind.map(|k| String::from(f.doc.str(k)));
    assert_eq!(kind.as_deref(), Some("RelLineTo"));
    assert_eq!(path_points(&f.doc, r), alloc::vec![[0.5, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.5], [0.0, 1.0], [0.5, 0.0]]);
    // the start out: the next vertex starts, the closing row goes there
    gesture(&mut f, &mut eng, "Scheitelpunkt löschen", |tx, eng| delete_vertex(tx, eng, r, 0, 1));
    assert_eq!(path_points(&f.doc, r), alloc::vec![[2.0, 0.0], [2.0, 1.0], [1.0, 1.5], [0.0, 1.0], [2.0, 0.0]]);
    let rows = f.doc.rows(SheetRef::Shape(r), SecKey { name: names::GEOMETRY, index: 0 });
    assert_eq!(f.doc.str(rows[0].kind.unwrap()), "RelMoveTo");
    // LockVtxEdit refuses
    let lock = cnum(&mut f.doc, "LockVtxEdit", 1.0);
    set_cell(&mut f, r, lock);
    let mut tx = f.doc.transaction("x");
    assert_eq!(set_vertex(&mut tx, &mut eng, r, 0, 1, [0.0, 0.0]), Err(EditError::Invalid));
}

// ---- new shapes, text

#[test]
fn new_shapes_of_the_drawing_tools_and_text() {
    let mut f = fx();
    let mut eng = Engine::new();
    let owner = Owner::Page(f.page);
    let r = gesture(&mut f, &mut eng, "Rechteck", |tx, eng| new_shape(tx, eng, owner, Basic::Rectangle, [3.0, 1.0], [1.0, 2.0]));
    let e = gesture(&mut f, &mut eng, "Ellipse", |tx, eng| new_shape(tx, eng, owner, Basic::Ellipse, [4.0, 4.0], [6.0, 5.0]));
    let t = gesture(&mut f, &mut eng, "Text", |tx, eng| new_shape(tx, eng, owner, Basic::TextBox, [1.0, 6.0], [3.0, 6.5]));
    let d = &f.doc;
    assert_eq!(d.top_shapes(owner).collect::<Vec<_>>(), alloc::vec![r, e, t]);
    let fr = frame(d, r).bounds();
    for (i, v) in [1.0, 1.0, 3.0, 2.0].into_iter().enumerate() {
        assert_close!(fr[i], v);
    }
    assert_eq!(formula(d, r, "LocPinX"), "Width*0.5");
    assert_eq!(d.shape(r).unwrap().header().styles, StyleRefs::all(1));
    assert_eq!(path_points(d, r), alloc::vec![[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0], [0.0, 0.0]]);
    let a = d.sym("A").unwrap();
    assert_eq!(row_formula(d, e, gaddr(1, a)), "Width*1");
    assert_eq!(d.shape(t).unwrap().header().styles, StyleRefs { line: Some(4), fill: Some(4), text: Some(1) });
    assert_eq!(formula(d, t, "Height"), "TEXTHEIGHT(TheText,Width)");
    let h0 = num(d, t, "Height");
    assert!(h0 > 0.0 && h0 < 0.5, "{h0}");
    // more lines: the box grows
    gesture(&mut f, &mut eng, "Text", |tx, eng| set_text(tx, eng, t, Text::plain("eins\nzwei\ndrei")));
    let h1 = num(&f.doc, t, "Height");
    assert!(h1 > h0 * 2.5, "{h0} {h1}");
    assert_eq!(f.doc.shape(t).unwrap().text().unwrap().chars(), "eins\nzwei\ndrei");
    // a dragged box without area makes no shape; LockTextEdit refuses text
    let mut tx = f.doc.transaction("x");
    assert_eq!(new_shape(&mut tx, &mut eng, owner, Basic::Rectangle, [1.0, 1.0], [1.0, 3.0]), Err(EditError::Invalid));
    drop(tx);
    let lock = cnum(&mut f.doc, "LockTextEdit", 1.0);
    set_cell(&mut f, r, lock);
    let mut tx = f.doc.transaction("x");
    assert_eq!(set_text(&mut tx, &mut eng, r, Text::plain("x")), Err(EditError::Invalid));
}

#[test]
fn text_block_frame_moves_and_sizes() {
    let mut f = fx();
    let s = add_box(&mut f, None, 1, [4.0, 4.0], [2.0, 1.0], 30.0, (false, false));
    {
        let doc = &mut f.doc;
        let cells = alloc::vec![cf(doc, "TxtPinX", "Width*0.5", 1.0), cf(doc, "TxtPinY", "Height*0.5", 0.5), cf(doc, "TxtWidth", "Width*1", 2.0), cf(doc, "TxtHeight", "Height*1", 1.0), cf(doc, "TxtLocPinX", "TxtWidth*0.5", 1.0), cf(doc, "TxtLocPinY", "TxtHeight*0.5", 0.5)];
        doc.sheet_mut(SheetRef::Shape(s)).unwrap().cells.extend(cells);
    }
    let mut eng = Engine::new();
    let tf = text_frame(&f.doc, s);
    same_corners(tf.corners(), frame(&f.doc, s).corners());
    // the text block's top edge up by 0.5: the bottom stays
    let want = tf.drag(0, 1, tf.at([1.0, 1.5]), false);
    gesture(&mut f, &mut eng, "Textblock", |tx, eng| set_text_frame(tx, eng, s, &want));
    let now = text_frame(&f.doc, s);
    assert_close!(now.h, 1.5);
    assert_pt!(now.grip(0, -1), tf.grip(0, -1));
    assert_eq!(formula(&f.doc, s, "TxtHeight"), "Height*1.5");
    assert_eq!(formula(&f.doc, s, "TxtPinY"), "Height*0.75");
}

// ---- copy and paste

#[test]
fn copy_and_paste_rewrites_ids_and_lets_outside_glue_go() {
    let mut f = fx();
    let a = add_box(&mut f, None, 1, [2.0, 2.0], [1.0, 1.0], 0.0, (false, false));
    let dd = add_box(&mut f, None, 4, [8.0, 2.0], [1.0, 1.0], 0.0, (false, false));
    for s in [a, dd] {
        let doc = &mut f.doc;
        let mut sec = Section::new(SecKey::one(names::CONNECTION));
        let mut r = Row::new(RowKey::Index(0));
        r.cells = alloc::vec![cf(doc, "X", "Width*1", 1.0), cf(doc, "Y", "Height*0.5", 0.5)];
        sec.rows.push(r);
        doc.sheet_mut(SheetRef::Shape(s)).unwrap().sections.push(sec);
    }
    let c = add_line(&mut f, None, 3, [2.5, 2.0], [8.5, 2.0]);
    let bx = cf(&mut f.doc, "BeginX", "PAR(PNT(Sheet.1!Connections.X1,Sheet.1!Connections.Y1))", 2.5);
    set_cell(&mut f, c, bx);
    let by = cf(&mut f.doc, "BeginY", "PAR(PNT(Sheet.1!Connections.X1,Sheet.1!Connections.Y1))", 2.0);
    set_cell(&mut f, c, by);
    let ex = cf(&mut f.doc, "EndX", "PAR(PNT(Sheet.4!Connections.X1,Sheet.4!Connections.Y1))", 8.5);
    set_cell(&mut f, c, ex);
    connect(&mut f, 3, "BeginX", 1, "Connections.X1");
    connect(&mut f, 3, "EndX", 4, "Connections.X1");
    // a group with a member in its formulas, named after its ID
    let g = add_group(&mut f, None, 20, [5.0, 6.0], [2.0, 2.0], 0.0, (false, false));
    let nu = f.doc.intern("Gruppe.20");
    f.doc.page_mut(f.page).unwrap().shapes.data_mut(g.slot).unwrap().header.name_u = Some(nu);
    let k = add_box(&mut f, Some(g), 21, [0.5, 1.0], [1.0, 1.0], 0.0, (false, false));
    let kx = cf(&mut f.doc, "PinX", "Sheet.20!Width*0.25", 0.5);
    set_cell(&mut f, k, kx);
    let ky = cf(&mut f.doc, "PinY", "Gruppe.20!Height*0.5", 1.0);
    set_cell(&mut f, k, ky);
    let mut eng = Engine::new();
    let clip = copy(&f.doc, &[a, c, g, k]);
    assert_eq!(clip.len(), 3);
    let cb = clip.bounds().unwrap();
    assert_close!(cb[0], 1.5);
    let owner = f.owner;
    let new = gesture(&mut f, &mut eng, "Einfügen", |tx, eng| paste(tx, eng, owner, &clip, [0.0, 3.0]));
    let d = &f.doc;
    assert_eq!(new.len(), 3);
    let (na, nc, ng) = (new[0], new[1], new[2]);
    let (ia, ig) = (id(d, na), id(d, ng));
    assert!(ia > 21 && ig > 21);
    assert_pt!([num(d, na, "PinX"), num(d, na, "PinY")], [2.0, 5.0]);
    // the connector glued inside the copy names the copy; the end glued outside is a value
    assert_eq!(formula(d, nc, "BeginX"), format!("PAR(PNT(Sheet.{ia}!Connections.X1,Sheet.{ia}!Connections.Y1))"));
    assert_pt!(page_ends(d, nc).0, [2.5, 5.0]);
    assert_eq!(formula(d, nc, "EndX"), "");
    assert_pt!(page_ends(d, nc).1, [8.5, 5.0]);
    let cs = &d.page(f.page).unwrap().connects;
    assert_eq!(cs.len(), 3);
    assert_eq!((cs[2].from_sheet, cs[2].to_sheet), (id(d, nc), ia));
    // the group's member names the new group by ID and by name
    let nk = d.shape(ng).unwrap().children().next().unwrap();
    assert_eq!(formula(d, nk, "PinX"), format!("Sheet.{ig}!Width*0.25"));
    assert_eq!(formula(d, nk, "PinY"), format!("Gruppe.{ig}!Height*0.5"));
    assert_eq!(d.shape(ng).unwrap().name_u(), format!("Gruppe.{ig}"));
    assert_pt!(frame(d, nk).centre(), [4.5, 9.0]);
}

#[test]
fn copy_a_subshape_takes_its_groups_transform() {
    let mut f = fx();
    let g = add_group(&mut f, None, 1, [5.0, 5.0], [3.0, 2.0], 50.0, (true, false));
    let c = add_box(&mut f, Some(g), 2, [1.0, 0.6], [1.0, 0.5], 20.0, (false, true));
    let pinx = cf(&mut f.doc, "PinX", "Sheet.1!Width*0.33333333333333", 1.0);
    set_cell(&mut f, c, pinx);
    let mut eng = Engine::new();
    let cc = corners(&f.doc, c);
    let clip = copy(&f.doc, &[c]);
    let owner = f.owner;
    let new = gesture(&mut f, &mut eng, "Einfügen", |tx, eng| paste(tx, eng, owner, &clip, [1.0, 0.0]));
    let d = &f.doc;
    assert!(d.shape(new[0]).unwrap().parent().is_none());
    same_corners(corners(d, new[0]), cc.map(|p| [p[0] + 1.0, p[1]]));
    assert_eq!(formula(d, new[0], "PinX"), "");
}

// ---- speed: a small selection in a large drawing (timed from outside: no clock without std)

#[test]
fn gestures_on_a_large_drawing() {
    let mut f = fx();
    let mut shapes = Vec::new();
    for i in 0..2000u32 {
        let (x, y) = ((i % 50) as f64 * 1.5, (i / 50) as f64 * 1.5);
        shapes.push(add_box(&mut f, None, i + 1, [x, y], [1.0, 0.5], (i % 7) as f64 * 10.0, (i % 3 == 0, false)));
    }
    let mut eng = Engine::new();
    {
        let mut tx = f.doc.transaction("warm");
        move_shapes(&mut tx, &mut eng, &shapes[..1], [0.0, 0.0]).unwrap();
    }
    let sel = [shapes[10], shapes[500], shapes[1999]];
    for i in 0..200 {
        let mut tx = f.doc.transaction("Verschieben");
        move_shapes(&mut tx, &mut eng, &sel, [0.01 * i as f64, 0.0]).unwrap();
        assert_close!(num(tx.doc(), sel[0], "PinX"), 15.0 + 0.01 * i as f64);
        let fr = frame(tx.doc(), sel[0]);
        set_frame(&mut tx, &mut eng, sel[0], &fr.drag(1, 1, fr.grip(1, 1), false)).unwrap();
        rotate_shapes(&mut tx, &mut eng, &sel, [10.0, 10.0], 0.01).unwrap();
        // the preview step is taken back (Livedynamik)
        drop(tx);
    }
    assert_close!(num(&f.doc, sel[0], "PinX"), 15.0);
}
