// tests of glue and routing (cargo test -p jflib-diagram-core --lib)
//
//   1. connection points on the page; the Visio cells and Connect entries of static and
//      dynamic glue; ends that follow their targets (also a subshape of a turned group)
//   2. dynamic glue docks on the side that faces the other shape and changes side
//   3. right-angle routes: no obstacle crossed, few bends, port directions kept, Z in the middle
//   4. lines keep their form, detach and unglue make constants, undo of a whole gesture
//   5. styles, rows inherited from a master, the text in the middle, a large page
//
// Every document is built here; nothing is read from files.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::formula::Engine;
use crate::route::Opts;
use crate::{
    Cell, CellAddr, Connect, DocKind, Document, Formula, Master, MasterHeader, Owner, Page, PageHeader, PageRef, Row, RowKey,
    SecKey, Section, ShapeData, ShapeHeader, ShapeKind, ShapeRef, Sheet, SheetRef, Unit, Value,
};

const PI: f64 = core::f64::consts::PI;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + b.abs())
}

fn near(p: [f64; 2], q: [f64; 2]) -> bool {
    close(p[0], q[0]) && close(p[1], q[1])
}

macro_rules! assert_pt {
    ($a:expr, $b:expr) => {{
        let (a, b): ([f64; 2], [f64; 2]) = ($a, $b);
        assert!(near(a, b), "{} = {:?}, not {:?}", stringify!($a), a, b);
    }};
}

fn assert_path(got: &[[f64; 2]], want: &[[f64; 2]]) {
    assert!(got.len() == want.len() && got.iter().zip(want).all(|(a, b)| near(*a, *b)), "path {got:?}, not {want:?}");
}

// ---- building documents

struct Fx {
    doc: Document,
    page: PageRef,
    owner: Owner,
    eng: Engine,
}

fn fx() -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap();
    Fx { doc, page, owner: Owner::Page(page), eng: Engine::new() }
}

fn cnum(doc: &mut Document, name: &str, v: f64) -> Cell {
    Cell::constant(doc.intern(name), Value::Num(v), None)
}

fn cf(doc: &mut Document, name: &str, f: &str, v: f64) -> Cell {
    let (n, f) = (doc.intern(name), doc.intern(f));
    Cell { name: n, value: Value::Num(v), formula: Formula::Expr(f), unit: None, error: None, stale: false }
}

fn row(key: u32, kind: Option<crate::Sym>, cells: Vec<Cell>) -> Row {
    let mut r = Row::new(RowKey::Index(key));
    r.kind = kind;
    r.cells = cells;
    r
}

fn xform(doc: &mut Document, d: &mut ShapeData, at: [f64; 2], size: [f64; 2], deg: f64) {
    let (w, h) = (size[0], size[1]);
    let cells = vec![
        cnum(doc, "PinX", at[0]),
        cnum(doc, "PinY", at[1]),
        cnum(doc, "Width", w),
        cnum(doc, "Height", h),
        cf(doc, "LocPinX", "Width*0.5", w * 0.5),
        cf(doc, "LocPinY", "Height*0.5", h * 0.5),
        cnum(doc, "Angle", deg * PI / 180.0),
    ];
    d.sheet.cells.extend(cells);
}

impl Fx {
    fn insert(&mut self, parent: Option<ShapeRef>, id: u32, d: ShapeData) -> ShapeRef {
        let tree = &mut self.doc.page_mut(self.page).unwrap().shapes;
        let slot = tree.insert(parent.map(|p| p.slot), id, d).unwrap();
        ShapeRef { owner: self.owner, slot }
    }

    // a rectangle w x h around `at` (in its group's coordinates when it has one), turned by
    // `deg`: connection points in the middle of the left, right, bottom and top side facing
    // in (rows 0-3, with Visio's formulas), one in the middle without direction (row 4)
    fn rect(&mut self, parent: Option<ShapeRef>, id: u32, at: [f64; 2], size: [f64; 2], deg: f64) -> ShapeRef {
        let doc = &mut self.doc;
        let mut d = ShapeData::new(ShapeKind::Shape);
        xform(doc, &mut d, at, size, deg);
        let (w, h) = (size[0], size[1]);
        let mut geo = Section::new(SecKey::one(names::GEOMETRY));
        geo.ix_written = true;
        for (i, p) in [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h], [0.0, 0.0]].iter().enumerate() {
            let kind = if i == 0 { names::MOVE_TO } else { names::LINE_TO };
            geo.rows.push(row(i as u32 + 1, Some(kind), vec![Cell::constant(names::X, Value::Num(p[0]), None), Cell::constant(names::Y, Value::Num(p[1]), None)]));
        }
        d.sheet.sections.push(geo);
        let mut con = Section::new(SecKey::one(names::CONNECTION));
        for (i, (fx, fy, dx, dy)) in [(0.0, 0.5, 1.0, 0.0), (1.0, 0.5, -1.0, 0.0), (0.5, 0.0, 0.0, 1.0), (0.5, 1.0, 0.0, -1.0), (0.5, 0.5, 0.0, 0.0)].into_iter().enumerate() {
            let cells = vec![
                cf(doc, "X", &alloc::format!("Width*{fx}"), w * fx),
                cf(doc, "Y", &alloc::format!("Height*{fy}"), h * fy),
                cnum(doc, "DirX", dx),
                cnum(doc, "DirY", dy),
                cnum(doc, "Type", 0.0),
            ];
            con.rows.push(row(i as u32, None, cells));
        }
        d.sheet.sections.push(con);
        self.insert(parent, id, d)
    }

    fn group(&mut self, id: u32, at: [f64; 2], size: [f64; 2], deg: f64) -> ShapeRef {
        let mut d = ShapeData::new(ShapeKind::Group);
        xform(&mut self.doc, &mut d, at, size, deg);
        self.insert(None, id, d)
    }

    // move a shape's pin as a gesture would, then update what is glued to it
    fn move_to(&mut self, s: ShapeRef, at: [f64; 2]) -> Option<crate::Change> {
        let mut tx = self.doc.transaction("Verschieben");
        let sheet = SheetRef::Shape(s);
        let (px, py) = (CellAddr::single(names::PIN_X), CellAddr::single(names::PIN_Y));
        tx.set_length(sheet, &px, at[0], Unit::Inches).unwrap();
        tx.set_length(sheet, &py, at[1], Unit::Inches).unwrap();
        self.eng.recalc_cells(&mut tx, &[(sheet, px), (sheet, py)]).unwrap();
        let deps = dependents(tx.doc(), self.owner, &[s]);
        update(&mut tx, &mut self.eng, &deps).unwrap();
        tx.commit()
    }

    fn connector(&mut self, begin: ([f64; 2], Target), end: ([f64; 2], Target)) -> ShapeRef {
        let mut tx = self.doc.transaction("Verbinden");
        let c = new_connector(&mut tx, &mut self.eng, self.owner, begin, end).unwrap();
        tx.commit();
        c
    }

    fn formula(&self, s: ShapeRef, cell: &str) -> String {
        let a = CellAddr::single(self.doc.sym(cell).expect(cell));
        match self.doc.resolve(SheetRef::Shape(s), &a).formula() {
            Formula::Expr(f) => String::from(self.doc.str(f)),
            Formula::Absent => String::new(),
            other => alloc::format!("{other:?}"),
        }
    }

    fn value(&self, s: ShapeRef, cell: &str) -> f64 {
        let a = CellAddr::single(self.doc.sym(cell).expect(cell));
        self.doc.resolve(SheetRef::Shape(s), &a).num().expect(cell)
    }

    fn connects(&self) -> Vec<(u32, String, i32, u32, String, i32)> {
        let s = |x: Option<crate::Sym>| String::from(x.map_or("", |x| self.doc.str(x)));
        self.doc
            .page(self.page)
            .unwrap()
            .connects
            .iter()
            .map(|c| (c.from_sheet, s(c.from_cell), c.from_part.unwrap_or(-1), c.to_sheet, s(c.to_cell), c.to_part.unwrap_or(-1)))
            .collect()
    }

    fn id(&self, s: ShapeRef) -> u32 {
        self.doc.shape(s).unwrap().id()
    }
}

type Dump = (Vec<(u32, ShapeHeader, Sheet)>, Vec<Connect>);

// what a page holds, without slots and the next ID (they are no content)
fn dump(doc: &Document, owner: Owner) -> Dump {
    let shapes = doc
        .all_shapes(owner)
        .map(|s| {
            let sh = doc.shape(s).unwrap();
            (sh.id(), sh.header().clone(), sh.sheet().clone())
        })
        .collect();
    (shapes, connects(doc, owner).to_vec())
}

fn axis_aligned(path: &[[f64; 2]]) -> bool {
    path.windows(2).all(|w| close(w[0][0], w[1][0]) || close(w[0][1], w[1][1]))
}

// the direction of a piece as a unit axis vector
fn heading(p: [f64; 2], q: [f64; 2]) -> [f64; 2] {
    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
    if dx.abs() >= dy.abs() { [dx.signum(), 0.0] } else { [0.0, dy.signum()] }
}

// ---- 1. connection points and glue

#[test]
fn connection_points_turn_with_the_shape() {
    let mut f = fx();
    let a = f.rect(None, 1, [4.0, 3.0], [2.0, 1.0], 90.0);
    let cp = connection_points(&f.doc, a);
    assert_eq!(cp.len(), 5);
    assert_eq!(cp.iter().map(|p| p.row).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
    // local (2, 0.5) -> the pin plus (1, 0) turned by 90 degrees; facing in: (-1, 0) -> (0, -1)
    assert_pt!(cp[1].at, [4.0, 4.0]);
    assert_pt!(cp[1].dir.unwrap(), [0.0, -1.0]);
    assert_pt!(cp[0].at, [4.0, 2.0]);
    assert_pt!(cp[0].dir.unwrap(), [0.0, 1.0]);
    assert_pt!(cp[2].at, [4.5, 3.0]);
    assert_pt!(cp[2].dir.unwrap(), [-1.0, 0.0]);
    assert!(cp[4].dir.is_none());
    assert!(cp.iter().all(|p| p.kind == 0));
}

#[test]
fn static_glue_writes_visio_cells_and_follows() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [6.0, 2.0], [1.0, 1.0], 0.0);
    let c = f.connector(([2.5, 2.0], Target::Point { shape: a, row: 1 }), ([5.5, 2.0], Target::Point { shape: b, row: 0 }));
    let id = f.id(c);
    assert_eq!(f.formula(c, "BeginX"), "PAR(PNT(Sheet.1!Connections.X2,Sheet.1!Connections.Y2))");
    assert_eq!(f.formula(c, "BeginY"), "PAR(PNT(Sheet.1!Connections.X2,Sheet.1!Connections.Y2))");
    assert_eq!(f.formula(c, "EndX"), "PAR(PNT(Sheet.2!Connections.X1,Sheet.2!Connections.Y1))");
    assert_eq!(f.formula(c, "EndY"), "PAR(PNT(Sheet.2!Connections.X1,Sheet.2!Connections.Y1))");
    assert_eq!(f.formula(c, "BegTrigger"), "_XFTRIGGER(Sheet.1!EventXFMod)");
    assert_eq!(f.formula(c, "EndTrigger"), "_XFTRIGGER(Sheet.2!EventXFMod)");
    assert_eq!(f.value(c, "BegTrigger"), 2.0);
    assert_eq!(f.formula(c, "Width"), "GUARD(EndX-BeginX)");
    assert_eq!(f.formula(c, "PinX"), "GUARD((BeginX+EndX)/2)");
    assert_eq!(f.value(c, "ObjType"), 2.0);
    assert_eq!(f.value(c, "GlueType"), 2.0);
    // one Connect per glued end, as Visio writes them
    assert_eq!(
        f.connects(),
        vec![
            (id, String::from("BeginX"), 9, 1, String::from("Connections.X2"), 101),
            (id, String::from("EndX"), 12, 2, String::from("Connections.X1"), 100),
        ]
    );
    assert_eq!(glued(&f.doc, c), [Target::Point { shape: a, row: 1 }, Target::Point { shape: b, row: 0 }]);
    assert!(is_connector(&f.doc, c) && is_one_d(&f.doc, c) && !is_one_d(&f.doc, a));
    assert_eq!(style(&f.doc, c), Style::RightAngle);
    let (bp, ep) = ends(&f.doc, c).unwrap();
    assert_pt!(bp, [2.5, 2.0]);
    assert_pt!(ep, [5.5, 2.0]);
    assert_path(&path(&f.doc, c), &[[2.5, 2.0], [5.5, 2.0]]);
    assert_eq!(f.value(c, "Width"), 3.0);

    // B moves up: the engine's recalculation moves the end, update routes a Z bending halfway
    let mut tx = f.doc.transaction("Verschieben");
    let (sb, py) = (SheetRef::Shape(b), CellAddr::single(names::PIN_Y));
    tx.set_length(sb, &py, 4.0, Unit::Inches).unwrap();
    f.eng.recalc_cells(&mut tx, &[(sb, py)]).unwrap();
    assert!(close(tx.doc().resolve(SheetRef::Shape(c), &CellAddr::single(names::END_Y)).num().unwrap(), 4.0), "the engine follows");
    let deps = dependents(tx.doc(), f.owner, &[b]);
    assert_eq!(deps, vec![c]);
    update(&mut tx, &mut f.eng, &deps).unwrap();
    tx.commit().unwrap();
    assert_path(&path(&f.doc, c), &[[2.5, 2.0], [4.0, 2.0], [4.0, 4.0], [5.5, 4.0]]);
    assert_eq!(f.formula(c, "EndX"), "PAR(PNT(Sheet.2!Connections.X1,Sheet.2!Connections.Y1))");
    // the text goes to the middle of the way, in local coordinates (origin at Begin)
    let tp = f.doc.sym("TextPosition").unwrap();
    let ctl = SecKey::one(names::CONTROL);
    let tx_ = |cell| f.doc.resolve(SheetRef::Shape(c), &CellAddr::row(ctl, RowKey::Named(tp), cell)).num().unwrap();
    assert!(close(tx_(names::X), 1.5) && close(tx_(names::Y), 1.0), "{} {}", tx_(names::X), tx_(names::Y));
    assert!(close(f.value(c, "TxtPinX"), 1.5));
    assert!(close(f.value(c, "TxtPinY"), 1.0));
}

#[test]
fn glue_to_a_subshape_of_a_turned_group() {
    let mut f = fx();
    let g = f.group(10, [5.0, 5.0], [2.0, 2.0], 90.0);
    let s = f.rect(Some(g), 11, [1.5, 1.0], [0.5, 0.5], 0.0);
    // the subshape's right point: (1.75, 1.0) in the group, (5, 5.75) on the page, facing down
    let cp = connection_points(&f.doc, s)[1];
    assert_pt!(cp.at, [5.0, 5.75]);
    assert_pt!(cp.dir.unwrap(), [0.0, -1.0]);
    let c = f.connector(([0.0, 0.0], Target::None), (cp.at, Target::Point { shape: s, row: 1 }));
    assert_eq!(f.formula(c, "EndX"), "PAR(PNT(Sheet.11!Connections.X2,Sheet.11!Connections.Y2))");
    assert_pt!(ends(&f.doc, c).unwrap().1, [5.0, 5.75]);
    let p = path(&f.doc, c);
    // the way arrives from above (against the point's direction) and does not cross the subshape
    let n = p.len();
    assert_pt!(heading(p[n - 2], p[n - 1]), [0.0, -1.0]);
    let sb = page_box(&f.doc, s).unwrap();
    assert!(p.windows(2).all(|w| !route::hits_box(w[0], w[1], &sb)), "{p:?}");
    // the group moves: the end follows through the engine
    f.move_to(g, [6.0, 5.0]).unwrap();
    let cp = connection_points(&f.doc, s)[1];
    assert_pt!(cp.at, [6.0, 5.75]);
    assert_pt!(ends(&f.doc, c).unwrap().1, [6.0, 5.75]);
    assert_pt!(*path(&f.doc, c).last().unwrap(), [6.0, 5.75]);
    assert_eq!(dependents(&f.doc, f.owner, &[g]), vec![c]);
}

// ---- 2. dynamic glue

#[test]
fn dynamic_glue_faces_the_other_shape() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [6.0, 2.0], [1.0, 1.0], 0.0);
    let c = f.connector(([2.0, 2.0], Target::Shape(a)), ([6.0, 2.0], Target::Shape(b)));
    assert_eq!(f.formula(c, "BeginX"), "_WALKGLUE(BegTrigger,EndTrigger,WalkPreference)");
    assert_eq!(f.formula(c, "EndY"), "_WALKGLUE(EndTrigger,BegTrigger,WalkPreference)");
    let id = f.id(c);
    assert_eq!(
        f.connects(),
        vec![(id, String::from("BeginX"), 9, 1, String::from("PinX"), 3), (id, String::from("EndX"), 12, 2, String::from("PinX"), 3)]
    );
    assert_eq!(glued(&f.doc, c), [Target::Shape(a), Target::Shape(b)]);
    // side to side, in the middle of the facing sides
    assert_path(&path(&f.doc, c), &[[2.5, 2.0], [5.5, 2.0]]);
    // B above A: top to bottom
    f.move_to(b, [2.0, 6.0]).unwrap();
    assert_path(&path(&f.doc, c), &[[2.0, 2.5], [2.0, 5.5]]);
    // a side shorter than 5 mm: Visio's formula, the box keeps its size, the way stays right
    assert_eq!(f.formula(c, "Width"), "GUARD(0.19685039370079DL)");
    assert!(close(f.value(c, "Width"), 0.19685039370079), "{}", f.value(c, "Width"));
    assert_eq!(f.formula(c, "Height"), "GUARD(EndY-BeginY)");
    // B on the other side: left to right
    f.move_to(b, [-2.0, 2.0]).unwrap();
    assert_path(&path(&f.doc, c), &[[1.5, 2.0], [-1.5, 2.0]]);
    assert_eq!(f.formula(c, "Width"), "GUARD(EndX-BeginX)");
    // diagonal: the cheapest pair of sides gives one bend
    f.move_to(b, [6.0, 6.0]).unwrap();
    let p = path(&f.doc, c);
    assert_eq!(route::bends(&p), 1, "{p:?}");
    // the values of the _WALKGLUE cells are the docking points, the formulas stay
    let (bp, ep) = ends(&f.doc, c).unwrap();
    assert_pt!(bp, p[0]);
    assert_pt!(ep, p[2]);
    assert_eq!(f.formula(c, "EndX"), "_WALKGLUE(EndTrigger,BegTrigger,WalkPreference)");
}

// (JD21) a shape a dynamic connector is glued to becomes placeable (ObjType 1) when its ObjType
// let Visio decide, as Visio does (Visio check visio-run4 [objtype]); one that says otherwise keeps it
#[test]
fn glued_shapes_become_placeable() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [6.0, 2.0], [1.0, 1.0], 0.0);
    let objtype = |f: &Fx, s| f.doc.sym("ObjType").and_then(|c| f.doc.resolve(SheetRef::Shape(s), &CellAddr::single(c)).num());
    assert_eq!(objtype(&f, a).unwrap_or(0.0), 0.0);
    {
        let mut tx = f.doc.transaction("ObjType");
        let c = CellAddr::single(tx.intern("ObjType"));
        tx.set_constant(SheetRef::Shape(b), &c, Value::Num(4.0), None).unwrap();
        tx.commit();
    }
    f.connector(([2.0, 2.0], Target::Shape(a)), ([6.0, 2.0], Target::Shape(b)));
    assert_eq!(objtype(&f, a), Some(1.0));
    assert_eq!(objtype(&f, b), Some(4.0));
}

// (JD15, SS-18) WalkPreference decides between ways of the same cost: B diagonal to A, both ends
// dynamic -- by default the begin leaves A's side (Visio's side to side), with bit 1
// (visWalkPrefBegNS) its top; bit 2 (visWalkPrefEndNS) docks the end on B's bottom; a dearer way
// is never taken for it (B straight right of A: side to side whatever the bits say)
#[test]
fn walk_preference_picks_the_sides() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [6.0, 6.0], [1.0, 1.0], 0.0);
    let c = f.connector(([2.0, 2.0], Target::Shape(a)), ([6.0, 6.0], Target::Shape(b)));
    let walk = |f: &mut Fx, v: f64| {
        let mut tx = f.doc.transaction("WalkPreference");
        let cell = CellAddr::single(tx.intern("WalkPreference"));
        tx.set_constant(SheetRef::Shape(c), &cell, Value::Num(v), None).unwrap();
        update(&mut tx, &mut f.eng, &[c]).unwrap();
        tx.commit();
        path(&f.doc, c)
    };
    let p = path(&f.doc, c);
    assert_eq!(route::bends(&p), 1, "{p:?}");
    assert_pt!(p[0], [2.5, 2.0]);
    let p = walk(&mut f, 1.0);
    assert_path(&p, &[[2.0, 2.5], [2.0, 6.0], [5.5, 6.0]]);
    let p = walk(&mut f, 2.0);
    assert_path(&p, &[[2.5, 2.0], [6.0, 2.0], [6.0, 5.5]]);
    // back to the default
    let p = walk(&mut f, 0.0);
    assert_pt!(p[0], [2.5, 2.0]);
    // a dearer way is never taken
    f.move_to(b, [6.0, 2.0]).unwrap();
    let p = walk(&mut f, 3.0);
    assert_path(&p, &[[2.5, 2.0], [5.5, 2.0]]);
}

#[test]
fn dynamic_glue_lines_up_when_it_can() {
    // B is a little to the right of A's top point: the end docks straight above it
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [2.3, 6.0], [1.0, 1.0], 0.0);
    let c = f.connector(([2.0, 2.5], Target::Point { shape: a, row: 3 }), ([2.3, 6.0], Target::Shape(b)));
    assert_path(&path(&f.doc, c), &[[2.0, 2.5], [2.0, 5.5]]);
    // both dynamic, the boxes overlap in x from 1.8 to 2.5: straight up in the middle of it
    let d = f.connector(([2.0, 2.0], Target::Shape(a)), ([2.3, 6.0], Target::Shape(b)));
    assert_path(&path(&f.doc, d), &[[2.15, 2.5], [2.15, 5.5]]);
    // out of reach: the middle of the side again
    f.move_to(b, [5.0, 6.0]).unwrap();
    let p = path(&f.doc, c);
    assert!(near(p[p.len() - 1], [5.0, 5.5]) || near(p[p.len() - 1], [4.5, 6.0]), "{p:?}");
}

#[test]
fn dynamic_glue_docks_on_the_outline() {
    // a triangle in a box: the top end docks where the middle line meets the slanted outline
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let mut d = ShapeData::new(ShapeKind::Shape);
    xform(&mut f.doc, &mut d, [2.0, 6.0], [2.0, 2.0], 0.0);
    let mut geo = Section::new(SecKey::one(names::GEOMETRY));
    for (i, p) in [[0.0, 0.0], [2.0, 0.0], [0.0, 2.0], [0.0, 0.0]].iter().enumerate() {
        let kind = if i == 0 { names::MOVE_TO } else { names::LINE_TO };
        geo.rows.push(row(i as u32 + 1, Some(kind), vec![Cell::constant(names::X, Value::Num(p[0]), None), Cell::constant(names::Y, Value::Num(p[1]), None)]));
    }
    d.sheet.sections.push(geo);
    let t = f.insert(None, 2, d);
    let c = f.connector(([2.0, 2.0], Target::Shape(a)), ([2.0, 6.0], Target::Shape(t)));
    // from A's top up to the triangle's bottom edge at x = 2 (its box from 1 to 3)
    assert_path(&path(&f.doc, c), &[[2.0, 2.5], [2.0, 5.0]]);
    // to the right of the triangle: its slanted side meets the middle line at (2, 6)
    f.move_to(a, [6.0, 6.0]).unwrap();
    assert_path(&path(&f.doc, c), &[[5.5, 6.0], [2.0, 6.0]]);
}

// ---- 3. routes

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

// boxes that keep a gap of `gap` to each other, in a field of `size`
fn scatter(rng: &mut Lcg, n: usize, size: f64, gap: f64) -> Vec<[f64; 4]> {
    let mut out: Vec<[f64; 4]> = Vec::new();
    let mut tries = 0;
    while out.len() < n && tries < n * 200 {
        tries += 1;
        let (w, h) = (0.3 + rng.next() * 1.2, 0.3 + rng.next() * 1.2);
        let (x, y) = (rng.next() * (size - w), rng.next() * (size - h));
        let b = [x, y, x + w, y + h];
        if out.iter().all(|o| b[0] > o[2] + gap || o[0] > b[2] + gap || b[1] > o[3] + gap || o[1] > b[3] + gap) {
            out.push(b);
        }
    }
    out
}

// a port in the middle of side `k` (0 right, 1 left, 2 top, 3 bottom) of box b, leaving it
fn side_port(b: [f64; 4], k: usize) -> Port {
    let (mx, my) = ((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5);
    let (at, dir) = match k {
        0 => ([b[2], my], [1.0, 0.0]),
        1 => ([b[0], my], [-1.0, 0.0]),
        2 => ([mx, b[3]], [0.0, 1.0]),
        _ => ([mx, b[1]], [0.0, -1.0]),
    };
    Port { at, dir: Some(dir), own: Some(b) }
}

fn check_route(p: &[[f64; 2]], a: &Port, b: &Port, boxes: &[[f64; 4]]) {
    assert!(p.len() >= 2 && near(p[0], a.at) && near(*p.last().unwrap(), b.at), "{p:?}");
    assert!(axis_aligned(p), "not right-angled: {p:?}");
    for w in p.windows(2) {
        for o in boxes {
            assert!(!route::hits_box(w[0], w[1], o), "{:?}-{:?} crosses {o:?} ({p:?})", w[0], w[1]);
        }
    }
    if let Some(d) = a.dir {
        assert_pt!(heading(p[0], p[1]), d);
    }
    if let Some(d) = b.dir {
        let n = p.len();
        assert_pt!(heading(p[n - 2], p[n - 1]), [-d[0], -d[1]]);
    }
}

#[test]
fn routes_never_cross_obstacles() {
    let mut rng = Lcg(7);
    let opts = Opts::default();
    for case in 0..150 {
        let boxes = scatter(&mut rng, 14, 12.0, 2.0 * opts.clearance + 0.01);
        if boxes.len() < 3 {
            continue;
        }
        let (i, j) = (case % boxes.len(), (case * 7 + 1) % boxes.len());
        if i == j {
            continue;
        }
        let a = side_port(boxes[i], case % 4);
        let b = side_port(boxes[j], (case / 4) % 4);
        let others: Vec<[f64; 4]> = boxes.iter().enumerate().filter(|(k, _)| *k != i && *k != j).map(|(_, o)| *o).collect();
        let p = route::orthogonal(a, b, &others, &opts);
        check_route(&p, &a, &b, &boxes);
        // keeps the clearance to the other obstacles
        for w in p.windows(2) {
            for o in &others {
                let g = [o[0] - opts.clearance * 0.99, o[1] - opts.clearance * 0.99, o[2] + opts.clearance * 0.99, o[3] + opts.clearance * 0.99];
                assert!(!route::hits_box(w[0], w[1], &g), "too close to {o:?}: {p:?}");
            }
        }
        // free ends too
        let fa = Port::free([rng.next() * 12.0, -1.0]);
        let fb = Port::free([rng.next() * 12.0, 13.0]);
        let p = route::orthogonal(fa, fb, &boxes, &opts);
        check_route(&p, &fa, &fb, &boxes);
    }
}

#[test]
fn simple_cases_have_few_bends() {
    let opts = Opts::default();
    let a = side_port([-1.0, -0.5, 0.0, 0.5], 0);
    // facing each other on one line: straight
    let b = side_port([5.0, -0.5, 6.0, 0.5], 1);
    assert_path(&route::orthogonal(a, b, &[], &opts), &[[0.0, 0.0], [5.0, 0.0]]);
    // a point below-facing on the right: one bend
    let b = side_port([4.5, 3.0, 5.5, 4.0], 3);
    assert_path(&route::orthogonal(a, b, &[], &opts), &[[0.0, 0.0], [5.0, 0.0], [5.0, 3.0]]);
    // facing, offset: a Z bending halfway
    let b = side_port([5.0, 1.5, 6.0, 2.5], 1);
    assert_path(&route::orthogonal(a, b, &[], &opts), &[[0.0, 0.0], [2.5, 0.0], [2.5, 2.0], [5.0, 2.0]]);
    // an obstacle in between: around it, still few bends
    let wall = [2.0, -2.0, 3.0, 1.0];
    let p = route::orthogonal(a, side_port([5.0, -0.5, 6.0, 0.5], 1), &[wall], &opts);
    check_route(&p, &a, &side_port([5.0, -0.5, 6.0, 0.5], 1), &[wall]);
    assert!(route::bends(&p) <= 4, "{p:?}");
    // turned away from each other: around both shapes
    let a2 = side_port([0.0, -0.5, 1.0, 0.5], 1);
    let b2 = side_port([4.0, -0.5, 5.0, 0.5], 0);
    let p = route::orthogonal(a2, b2, &[], &opts);
    check_route(&p, &a2, &b2, &[[0.0, -0.5, 1.0, 0.5], [4.0, -0.5, 5.0, 0.5]]);
    assert!(route::bends(&p) <= 4, "{p:?}");
    // a direction into the own shape counts as none: the way leaves on the side it needs
    let inward = Port { at: [0.0, 0.0], dir: Some([1.0, 0.0]), own: Some([0.0, -0.5, 1.0, 0.5]) };
    let p = route::orthogonal(inward, Port::free([-3.0, 0.0]), &[], &opts);
    assert_path(&p, &[[0.0, 0.0], [-3.0, 0.0]]);
}

#[test]
fn the_estimate_never_overshoots() {
    // the fewest bends by hand: heading +x from the origin
    let b = |e: [f64; 2], a: Option<usize>| route::min_bends(0, [0.0, 0.0], e, a);
    assert_eq!((b([5.0, 0.0], None), b([5.0, 3.0], None), b([-5.0, 3.0], None), b([-5.0, 0.0], None)), (0, 1, 2, 3));
    assert_eq!((b([5.0, 0.0], Some(0)), b([5.0, 3.0], Some(0)), b([0.0, 3.0], Some(0)), b([-5.0, 3.0], Some(0))), (0, 2, 2, 4));
    assert_eq!((b([5.0, 3.0], Some(1)), b([-5.0, 3.0], Some(1)), b([5.0, 0.0], Some(1))), (2, 2, 2));
    assert_eq!((b([5.0, 3.0], Some(2)), b([0.0, 3.0], Some(2)), b([5.0, -3.0], Some(2)), b([5.0, 0.0], Some(2))), (1, 1, 3, 1));
    // at the goal itself: the turn into the last piece
    assert_eq!((b([0.0, 0.0], Some(0)), b([0.0, 0.0], Some(2)), b([0.0, 0.0], Some(1))), (0, 1, 2));
    // the same cost as a search without the estimate, over many pages
    let mut rng = Lcg(99);
    let opts = Opts::default();
    for case in 0..60 {
        let boxes = scatter(&mut rng, 16, 10.0, 2.0 * opts.clearance + 0.01);
        let (i, j) = (case % boxes.len(), (case * 5 + 3) % boxes.len());
        if i == j {
            continue;
        }
        let a = side_port(boxes[i], case % 4);
        let b = if case % 3 == 0 { Port::free([rng.next() * 10.0, rng.next() * 10.0]) } else { side_port(boxes[j], (case / 4) % 4) };
        if boxes.iter().any(|o| route::hits_box(b.at, b.at, o) || (b.own.is_none() && o[0] <= b.at[0] && b.at[0] <= o[2] && o[1] <= b.at[1] && b.at[1] <= o[3])) {
            continue;
        }
        let others: Vec<[f64; 4]> = boxes.iter().enumerate().filter(|(k, _)| *k != i && (b.own.is_none() || *k != j)).map(|(_, o)| *o).collect();
        let fast = route::orthogonal(a, b, &others, &opts);
        let exact = route::orthogonal_exact(a, b, &others, &opts);
        let (cf, ce) = (route::cost(&fast, &opts), route::cost(&exact, &opts));
        assert!(cf <= ce + 1e-9, "case {case}: {cf} > {ce}: {fast:?} / {exact:?}");
    }
}

#[test]
fn close_shapes_connect_straight() {
    // 0.1 apart, less than twice the clearance: straight from one to the other
    let opts = Opts::default();
    let (a, b) = (side_port([0.0, 0.0, 1.0, 1.0], 3), side_port([0.0, -1.1, 1.0, -0.1], 2));
    assert_path(&route::orthogonal(a, b, &[], &opts), &[[0.5, 0.0], [0.5, -0.1]]);
    // from a point in the middle without direction
    let m = Port { at: [0.5, 0.5], dir: None, own: Some([0.0, 0.0, 1.0, 1.0]) };
    assert_path(&route::orthogonal(m, b, &[], &opts), &[[0.5, 0.5], [0.5, -0.1]]);
}

#[test]
fn spline_knots_are_clamped() {
    assert_eq!(route::spline_knots(2), (1, vec![0.0, 0.0, 1.0, 1.0]));
    assert_eq!(route::spline_knots(3), (2, vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]));
    assert_eq!(route::spline_knots(5), (3, vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0]));
}

#[test]
fn a_hundred_obstacles() {
    // a page of 100 shapes: the route stays clear (the time is measured by route_bench)
    let mut rng = Lcg(42);
    let opts = Opts::default();
    let boxes = scatter(&mut rng, 100, 30.0, 2.0 * opts.clearance + 0.01);
    assert_eq!(boxes.len(), 100);
    for k in 0..20 {
        let (i, j) = (k, 99 - k);
        let a = side_port(boxes[i], k % 4);
        let b = side_port(boxes[j], (k + 1) % 4);
        let others: Vec<[f64; 4]> = boxes.iter().enumerate().filter(|(n, _)| *n != i && *n != j).map(|(_, o)| *o).collect();
        let p = route::orthogonal(a, b, &others, &opts);
        check_route(&p, &a, &b, &boxes);
    }
}

#[test]
#[ignore = "timing: run alone with --ignored and measure the process (200 routes)"]
fn route_bench() {
    let mut rng = Lcg(42);
    let opts = Opts::default();
    let boxes = scatter(&mut rng, 100, 30.0, 2.0 * opts.clearance + 0.01);
    let mut total = 0usize;
    for k in 0..200 {
        let (i, j) = (k % 100, (k * 37 + 11) % 100);
        if i == j {
            continue;
        }
        let a = side_port(boxes[i], k % 4);
        let b = side_port(boxes[j], (k / 4) % 4);
        let others: Vec<[f64; 4]> = boxes.iter().enumerate().filter(|(n, _)| *n != i && *n != j).map(|(_, o)| *o).collect();
        total += route::orthogonal(a, b, &others, &opts).len();
    }
    assert!(total > 0);
}

#[test]
fn both_ends_on_one_shape_go_around_it() {
    let opts = Opts::default();
    let box_ = [0.0, 0.0, 2.0, 1.0];
    let (a, b) = (side_port(box_, 2), side_port(box_, 3));
    let p = route::orthogonal(a, b, &[], &opts);
    check_route(&p, &a, &b, &[box_]);
    assert_eq!(route::bends(&p), 4, "{p:?}");
}

#[test]
fn connector_routes_around_shapes_on_the_page() {
    let mut f = fx();
    let a = f.rect(None, 1, [1.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [7.0, 2.0], [1.0, 1.0], 0.0);
    let wall = f.rect(None, 3, [4.0, 2.0], [1.0, 3.0], 0.0);
    let c = f.connector(([1.5, 2.0], Target::Point { shape: a, row: 1 }), ([6.5, 2.0], Target::Point { shape: b, row: 0 }));
    let p = path(&f.doc, c);
    let boxes: Vec<[f64; 4]> = [a, b, wall].iter().map(|s| page_box(&f.doc, *s).unwrap()).collect();
    let (pa, pb) = (Port { at: [1.5, 2.0], dir: Some([1.0, 0.0]), own: None }, Port { at: [6.5, 2.0], dir: Some([-1.0, 0.0]), own: None });
    check_route(&p, &pa, &pb, &boxes);
    assert!(route::bends(&p) == 4, "{p:?}");
}

// ---- 4. lines, unglue, detach, undo

#[test]
fn a_line_keeps_its_form() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [6.0, 2.0], [1.0, 1.0], 0.0);
    let mut tx = f.doc.transaction("Linie");
    let l = new_line(&mut tx, &mut f.eng, f.owner, ([2.5, 2.0], Target::Point { shape: a, row: 1 }), ([5.5, 2.0], Target::Point { shape: b, row: 0 })).unwrap();
    // lines glue statically only
    assert_eq!(glue(&mut tx, &mut f.eng, l, End::End, [6.0, 2.0], Target::Shape(b)), Err(EditError::Invalid));
    tx.commit().unwrap();
    assert!(!is_connector(&f.doc, l) && is_one_d(&f.doc, l));
    assert_eq!(f.formula(l, "BeginX"), "PAR(PNT(Sheet.1!Connections.X2,Sheet.1!Connections.Y2))");
    assert_eq!(f.connects().len(), 2);
    f.move_to(b, [6.0, 5.0]).unwrap();
    // one straight piece from end to end: Width and Angle follow, the rows keep their formulas
    assert!(close(f.value(l, "Width"), crate::geometry::sqrt(18.0)), "{}", f.value(l, "Width"));
    assert!(close(f.value(l, "Angle"), crate::geometry::atan2(3.0, 3.0)));
    assert_path(&path(&f.doc, l), &[[2.5, 2.0], [5.5, 5.0]]);
    let sec = SecKey::one(names::GEOMETRY);
    let x2 = f.doc.resolve(SheetRef::Shape(l), &CellAddr::row(sec, RowKey::Index(2), names::X));
    assert_eq!(x2.formula().expr().map(|s| f.doc.str(s)), Some("Width*1"));
}

#[test]
fn unglue_and_detach_leave_constants() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [6.0, 2.0], [1.0, 1.0], 0.0);
    let c = f.connector(([2.5, 2.0], Target::Point { shape: a, row: 1 }), ([5.5, 2.0], Target::Shape(b)));
    // the end leaves B
    let mut tx = f.doc.transaction("Lösen");
    glue(&mut tx, &mut f.eng, c, End::End, [8.0, 3.0], Target::None).unwrap();
    tx.commit().unwrap();
    assert_eq!(f.formula(c, "EndX"), "");
    assert_pt!(ends(&f.doc, c).unwrap().1, [8.0, 3.0]);
    assert_eq!(f.formula(c, "EndTrigger"), "_XFTRIGGER(EventXFMod)");
    assert_eq!(f.value(c, "EndTrigger"), 1.0);
    assert_eq!(f.connects().len(), 1);
    assert_eq!(glued(&f.doc, c)[1], Target::None);
    assert_pt!(*path(&f.doc, c).last().unwrap(), [8.0, 3.0]);
    // A is deleted: the begin keeps its place as a constant
    let mut tx = f.doc.transaction("Löschen");
    detach(&mut tx, &mut f.eng, f.owner, &[a]).unwrap();
    tx.delete_shapes(&[a]).unwrap();
    tx.commit().unwrap();
    assert_eq!(f.formula(c, "BeginX"), "");
    assert_eq!(f.formula(c, "BegTrigger"), "_XFTRIGGER(EventXFMod)");
    assert_pt!(ends(&f.doc, c).unwrap().0, [2.5, 2.0]);
    assert!(f.connects().is_empty());
    assert_eq!(glued(&f.doc, c), [Target::None, Target::None]);
}

#[test]
fn undo_takes_a_whole_gesture_back() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [6.0, 2.0], [1.0, 1.0], 0.0);
    let before = dump(&f.doc, f.owner);
    let mut tx = f.doc.transaction("Verbinden");
    let c = new_connector(&mut tx, &mut f.eng, f.owner, ([2.5, 2.0], Target::Point { shape: a, row: 1 }), ([6.0, 2.0], Target::Shape(b))).unwrap();
    let mut made = tx.commit().unwrap();
    let connected = dump(&f.doc, f.owner);
    let mut moved = f.move_to(b, [5.0, 6.0]).unwrap();
    assert_ne!(dump(&f.doc, f.owner), connected);
    assert!(moved.undo(&mut f.doc));
    f.eng.refresh(&f.doc, &moved.sheets());
    assert_eq!(dump(&f.doc, f.owner), connected);
    assert!(made.undo(&mut f.doc));
    f.eng.refresh(&f.doc, &made.sheets());
    assert_eq!(dump(&f.doc, f.owner), before);
    assert!(f.doc.shape(c).is_none());
    // redo both: the same document again, and the next gesture works on it
    assert!(made.redo(&mut f.doc) && moved.redo(&mut f.doc));
    f.eng.refresh(&f.doc, &made.sheets());
    f.eng.refresh(&f.doc, &moved.sheets());
    f.move_to(b, [6.0, 2.0]).unwrap();
    assert_path(&path(&f.doc, c), &[[2.5, 2.0], [5.5, 2.0]]);
}

#[test]
fn nothing_changes_without_a_gesture() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [6.0, 2.0], [1.0, 1.0], 0.0);
    let other = f.rect(None, 3, [6.0, 6.0], [1.0, 1.0], 0.0);
    let c = f.connector(([2.5, 2.0], Target::Point { shape: a, row: 1 }), ([6.0, 2.0], Target::Shape(b)));
    assert_eq!(dependents(&f.doc, f.owner, &[other]), Vec::<ShapeRef>::new());
    assert_eq!(dependents(&f.doc, f.owner, &[a, b]), vec![c]);
    assert_eq!(dependents(&f.doc, f.owner, &[c]), vec![c]);
    let mut tx = f.doc.transaction("nichts");
    update(&mut tx, &mut f.eng, &[]).unwrap();
    // routing the same state again writes nothing
    update(&mut tx, &mut f.eng, &[c]).unwrap();
    assert!(tx.commit().is_none());
}

#[test]
fn glue_refuses_what_does_not_fit() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let c = f.connector(([0.0, 0.0], Target::None), ([1.0, 1.0], Target::None));
    let mut tx = f.doc.transaction("x");
    assert_eq!(glue(&mut tx, &mut f.eng, c, End::Begin, [0.0, 0.0], Target::Point { shape: a, row: 9 }), Err(EditError::NoRow));
    assert_eq!(glue(&mut tx, &mut f.eng, c, End::Begin, [0.0, 0.0], Target::Shape(c)), Err(EditError::Invalid));
    assert_eq!(glue(&mut tx, &mut f.eng, a, End::Begin, [0.0, 0.0], Target::None), Err(EditError::Invalid));
}

// ---- 5. styles, master rows

#[test]
fn styles_straight_and_curved() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [6.0, 4.0], [1.0, 1.0], 0.0);
    let c = f.connector(([2.5, 2.0], Target::Point { shape: a, row: 1 }), ([5.5, 4.0], Target::Point { shape: b, row: 0 }));
    assert_path(&path(&f.doc, c), &[[2.5, 2.0], [4.0, 2.0], [4.0, 4.0], [5.5, 4.0]]);
    let geo = SecKey::one(names::GEOMETRY);
    let kinds = |doc: &Document| doc.rows(SheetRef::Shape(c), geo).iter().map(|r| r.kind.map_or("", |k| doc.str(k)).len()).collect::<Vec<_>>();
    // curved: MoveTo and one NURBSTo over the corners
    let mut tx = f.doc.transaction("Gebogen");
    set_style(&mut tx, &mut f.eng, &[c], Style::Curved).unwrap();
    tx.commit().unwrap();
    assert_eq!(style(&f.doc, c), Style::Curved);
    assert_eq!(f.value(c, "ShapeRouteStyle"), 1.0);
    assert_eq!(f.value(c, "ConLineRouteExt"), 2.0);
    let rows = f.doc.rows(SheetRef::Shape(c), geo);
    assert_eq!(rows.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![Some(names::MOVE_TO), Some(names::NURBS_TO)]);
    let e = f.doc.resolve(SheetRef::Shape(c), &CellAddr::row(geo, rows[1].key, names::E));
    let ef = e.formula().expr().map(|s| String::from(f.doc.str(s))).unwrap();
    assert!(ef.starts_with("NURBS(1,3,0,0,"), "{ef}");
    let ev = e.value().text().map(|s| String::from(f.doc.str(s))).unwrap();
    assert!(ev.starts_with("NURBS(1, 3, 0, 0, "), "{ev}");
    assert_eq!(e.unit(), Some(Unit::Nurbs));
    let p = path(&f.doc, c);
    assert_pt!(p[0], [2.5, 2.0]);
    assert_pt!(*p.last().unwrap(), [5.5, 4.0]);
    assert!(p.len() > 10 && !axis_aligned(&p));
    // straight: one line from end to end
    let mut tx = f.doc.transaction("Gerade");
    set_style(&mut tx, &mut f.eng, &[c], Style::Straight).unwrap();
    tx.commit().unwrap();
    assert_eq!(style(&f.doc, c), Style::Straight);
    assert_eq!(f.value(c, "ShapeRouteStyle"), 16.0);
    assert_path(&path(&f.doc, c), &[[2.5, 2.0], [5.5, 4.0]]);
    // right angles again
    let mut tx = f.doc.transaction("Rechtwinklig");
    set_style(&mut tx, &mut f.eng, &[c], Style::RightAngle).unwrap();
    tx.commit().unwrap();
    assert_path(&path(&f.doc, c), &[[2.5, 2.0], [4.0, 2.0], [4.0, 4.0], [5.5, 4.0]]);
    assert_eq!(kinds(&f.doc), vec![6, 6, 6, 6]);
}

#[test]
fn rows_of_a_master_are_kept_or_marked() {
    let mut f = fx();
    // a master like a dynamic connector's: MoveTo 1, LineTo 2, LineTo 3 and the 1-D cells
    let doc = &mut f.doc;
    let mut ms = ShapeData::new(ShapeKind::Shape);
    let cells = vec![
        cf(doc, "PinX", "GUARD((BeginX+EndX)/2)", 1.5),
        cf(doc, "PinY", "GUARD((BeginY+EndY)/2)", 1.5),
        cf(doc, "Width", "GUARD(EndX-BeginX)", 1.0),
        cf(doc, "Height", "GUARD(EndY-BeginY)", -1.0),
        cf(doc, "LocPinX", "GUARD(Width*0.5)", 0.5),
        cf(doc, "LocPinY", "GUARD(Height*0.5)", -0.5),
        cf(doc, "Angle", "GUARD(0DA)", 0.0),
        cnum(doc, "BeginX", 1.0),
        cnum(doc, "BeginY", 2.0),
        cnum(doc, "EndX", 2.0),
        cnum(doc, "EndY", 1.0),
        cnum(doc, "ObjType", 2.0),
        cnum(doc, "GlueType", 2.0),
    ];
    ms.sheet.cells = cells;
    let mut geo = Section::new(SecKey::one(names::GEOMETRY));
    geo.ix_written = true;
    geo.cells.push(Cell::constant(names::NO_FILL, Value::Num(1.0), None));
    for (k, kind, x, y) in [(1, names::MOVE_TO, 0.0, 0.0), (2, names::LINE_TO, 0.0, -1.0), (3, names::LINE_TO, 1.0, -1.0)] {
        geo.rows.push(row(k, Some(kind), vec![Cell::constant(names::X, Value::Num(x), None), Cell::constant(names::Y, Value::Num(y), None)]));
    }
    ms.sheet.sections.push(geo);
    let mut m = Master::new(MasterHeader { id: 2, ..MasterHeader::default() });
    m.shapes.insert(None, 5, ms).unwrap();
    doc.insert_master(m).unwrap();
    let mut inst = ShapeData::new(ShapeKind::Shape);
    inst.header.master = Some(2);
    inst.sheet.cells = vec![cnum(doc, "BeginX", 1.0), cnum(doc, "BeginY", 1.0), cnum(doc, "EndX", 3.5), cnum(doc, "EndY", 3.0)];
    let c = f.insert(None, 7, inst);
    let a = f.rect(None, 1, [0.5, 1.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [4.0, 3.0], [1.0, 1.0], 0.0);
    assert!(is_connector(&f.doc, c));
    let mut tx = f.doc.transaction("Kleben");
    glue(&mut tx, &mut f.eng, c, End::Begin, [1.0, 1.0], Target::Point { shape: a, row: 1 }).unwrap();
    glue(&mut tx, &mut f.eng, c, End::End, [3.5, 3.0], Target::Point { shape: b, row: 0 }).unwrap();
    tx.commit().unwrap();
    let geo = SecKey::one(names::GEOMETRY);
    let own = |doc: &Document| -> Vec<(u32, bool, Option<crate::Sym>)> {
        let s = doc.sheet(SheetRef::Shape(c)).unwrap();
        s.section(geo).map_or(Vec::new(), |sec| sec.rows.iter().map(|r| (match r.key { RowKey::Index(i) => i, _ => 0 }, r.del, r.kind)).collect())
    };
    // a Z: the master's three rows and one of its own
    assert_path(&path(&f.doc, c), &[[1.0, 1.0], [2.25, 1.0], [2.25, 3.0], [3.5, 3.0]]);
    assert_eq!(own(&f.doc), vec![(1, false, None), (2, false, None), (3, false, None), (4, false, Some(names::LINE_TO))]);
    // straight: row 3 gets a Del mark, row 4 goes
    f.move_to(b, [4.0, 1.0]).unwrap();
    assert_path(&path(&f.doc, c), &[[1.0, 1.0], [3.5, 1.0]]);
    assert_eq!(own(&f.doc), vec![(1, false, None), (2, false, None), (3, true, None)]);
    assert_eq!(f.formula(c, "Height"), "GUARD(0.19685039370079DL)");
    // curved: the master's rows have other types, they get Del marks and new rows follow
    f.move_to(b, [4.0, 3.0]).unwrap();
    assert_eq!(own(&f.doc), vec![(1, false, None), (2, false, None), (3, false, None), (4, false, Some(names::LINE_TO))]);
    let mut tx = f.doc.transaction("Gebogen");
    set_style(&mut tx, &mut f.eng, &[c], Style::Curved).unwrap();
    tx.commit().unwrap();
    let rows = f.doc.rows(SheetRef::Shape(c), geo);
    assert_eq!(rows.iter().map(|r| (r.key, r.kind)).collect::<Vec<_>>(), vec![(RowKey::Index(5), Some(names::MOVE_TO)), (RowKey::Index(6), Some(names::NURBS_TO))]);
    let p = path(&f.doc, c);
    assert_pt!(p[0], [1.0, 1.0]);
    assert_pt!(*p.last().unwrap(), [3.5, 3.0]);
    // right angles: the master's rows come back
    let mut tx = f.doc.transaction("Rechtwinklig");
    set_style(&mut tx, &mut f.eng, &[c], Style::RightAngle).unwrap();
    tx.commit().unwrap();
    assert_path(&path(&f.doc, c), &[[1.0, 1.0], [2.25, 1.0], [2.25, 3.0], [3.5, 3.0]]);
    let rows = f.doc.rows(SheetRef::Shape(c), geo);
    assert_eq!(rows.iter().map(|r| (r.key, r.kind)).collect::<Vec<_>>(), vec![
        (RowKey::Index(1), Some(names::MOVE_TO)),
        (RowKey::Index(2), Some(names::LINE_TO)),
        (RowKey::Index(3), Some(names::LINE_TO)),
        (RowKey::Index(4), Some(names::LINE_TO)),
    ]);
}

#[test]
fn a_fixed_route_keeps_its_corners() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [6.0, 4.0], [1.0, 1.0], 0.0);
    let c = f.connector(([2.5, 2.0], Target::Point { shape: a, row: 1 }), ([5.5, 4.0], Target::Point { shape: b, row: 0 }));
    let mut tx = f.doc.transaction("fest");
    let fc = tx.single("ConFixedCode");
    tx.set_constant(SheetRef::Shape(c), &fc, Value::Num(2.0), None).unwrap();
    tx.commit().unwrap();
    // B moves up and right: the middle piece stays at x = 4
    f.move_to(b, [7.0, 5.0]).unwrap();
    assert_path(&path(&f.doc, c), &[[2.5, 2.0], [4.0, 2.0], [4.0, 5.0], [6.5, 5.0]]);
    // reset: routed anew, bending halfway
    let mut tx = f.doc.transaction("Zurücksetzen");
    reset(&mut tx, &mut f.eng, &[c]).unwrap();
    tx.commit().unwrap();
    assert_path(&path(&f.doc, c), &[[2.5, 2.0], [4.5, 2.0], [4.5, 5.0], [6.5, 5.0]]);
}

#[test]
fn the_page_scale_and_spacing_count() {
    let mut f = fx();
    // 1:10 and a spacing of 0.25 in on paper: the clearance is 2.5 drawing inches
    let cells = vec![cnum(&mut f.doc, "PageScale", 1.0), cnum(&mut f.doc, "DrawingScale", 10.0), cnum(&mut f.doc, "LineToNodeX", 0.25)];
    f.doc.page_mut(f.page).unwrap().sheet.cells = cells;
    let o = opts_for(&f.doc, f.owner);
    assert!(close(o.clearance, 2.5) && close(o.bend, 5.0), "{o:?}");
}

#[test]
#[ignore = "timing: run alone with --ignored and measure the process (100 gestures)"]
fn update_bench() {
    // 100 shapes on the page, a connector dynamically glued across it; one end's shape moves
    // 100 times, each a gesture with recalculation and routing
    let mut f = fx();
    let mut rng = Lcg(42);
    let boxes = scatter(&mut rng, 100, 30.0, 0.26);
    let mut shapes = Vec::new();
    for (i, b) in boxes.iter().enumerate() {
        let s = f.rect(None, i as u32 + 1, [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5], [b[2] - b[0], b[3] - b[1]], 0.0);
        shapes.push(s);
    }
    let (a, b) = (shapes[0], shapes[99]);
    let c = f.connector(([0.0, 0.0], Target::Shape(a)), ([0.0, 0.0], Target::Shape(b)));
    let home = centre(&f.doc, b);
    for k in 0..100 {
        let d = (k % 10) as f64 * 0.01;
        f.move_to(b, [home[0] + d, home[1] - d]);
    }
    assert!(path(&f.doc, c).len() >= 2);
}

// (JD8 rest) the freeform and arc tools: 1-D shapes that glue like a line and keep their form
#[test]
fn freeform_glues_and_keeps_its_form() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 2.0], [1.0, 1.0], 0.0);
    let b = f.rect(None, 2, [6.0, 2.0], [1.0, 1.0], 0.0);
    // a wave from A's right side to B's left side
    let mut stroke = Vec::new();
    for i in 0..=60 {
        let t = i as f64 / 60.0;
        stroke.push([2.5 + 3.0 * t, 2.0 + 0.6 * crate::formula::math::sincos(t * PI * 2.0).0]);
    }
    let mut tx = f.doc.transaction("Freihandform");
    let s = crate::ops::new_freeform(&mut tx, &mut f.eng, f.owner, &stroke, 0.01, 0.05, Target::Point { shape: a, row: 1 }, Target::Point { shape: b, row: 0 }).unwrap();
    // a Shape target is refused, as for lines
    assert!(crate::ops::new_freeform(&mut tx, &mut f.eng, f.owner, &stroke, 0.01, 0.05, Target::Shape(a), Target::None).is_err());
    tx.commit().unwrap();
    assert!(is_one_d(&f.doc, s) && !is_connector(&f.doc, s));
    assert_eq!(f.formula(s, "BeginX"), "PAR(PNT(Sheet.1!Connections.X2,Sheet.1!Connections.Y2))");
    assert_eq!(f.connects().len(), 2);
    // the curve runs from end to end near the stroke, through the NURBS row the engine computed
    let p = path(&f.doc, s);
    assert!(p.len() > 8);
    assert!(near(p[0], [2.5, 2.0]) && near(*p.last().unwrap(), [5.5, 2.0]), "{:?} {:?}", p[0], p.last());
    let top = p.iter().fold(f64::MIN, |m, q| m.max(q[1]));
    assert!(top > 2.3 && top < 2.6, "{top}");
    let sec = SecKey::one(names::GEOMETRY);
    let e = f.doc.resolve(SheetRef::Shape(s), &CellAddr::row(sec, RowKey::Index(2), names::E));
    let ef = e.formula().expr().map(|x| String::from(f.doc.str(x))).unwrap();
    assert!(ef.starts_with("NURBS(") && ef.contains("Width*"), "{ef}");
    // B moves: the curve follows end to end, scaled and turned with its chord
    f.move_to(b, [6.0, 5.0]).unwrap();
    let p = path(&f.doc, s);
    assert!(near(p[0], [2.5, 2.0]) && near(*p.last().unwrap(), [5.5, 5.0]), "{:?} {:?}", p[0], p.last());
    // a closed stroke: a filled 2-D shape
    let ring: Vec<[f64; 2]> = (0..=40).map(|i| {
        let (s, c) = crate::formula::math::sincos(i as f64 / 40.0 * PI * 2.0);
        [8.0 + c, 8.0 + s]
    }).collect();
    let mut tx = f.doc.transaction("Freihandform");
    let r = crate::ops::new_freeform(&mut tx, &mut f.eng, f.owner, &ring, 0.01, 0.05, Target::None, Target::None).unwrap();
    tx.commit().unwrap();
    assert!(!is_one_d(&f.doc, r));
    let g = f.doc.geometry_paths(SheetRef::Shape(r), f.value(r, "Width"), f.value(r, "Height"));
    assert!(!g[0].no_fill && g[0].path.subs[0].segs.len() > 4);
    let w = f.value(r, "Width");
    assert!(w > 1.9 && w < 2.01, "{w}");
}

#[test]
fn arc_is_a_quarter_ellipse_that_glues() {
    let mut f = fx();
    let a = f.rect(None, 1, [2.0, 4.0], [1.0, 1.0], 0.0);
    // from A's bottom (2, 3.5) to (5, 1.5), leaving vertically: the centre at (5, 3.5)
    let mut tx = f.doc.transaction("Bogen");
    let s = crate::ops::new_arc(&mut tx, &mut f.eng, f.owner, ([2.0, 3.5], Target::Point { shape: a, row: 2 }), ([5.0, 1.5], Target::None), false).unwrap();
    tx.commit().unwrap();
    assert!(is_one_d(&f.doc, s));
    assert_eq!(f.formula(s, "BeginX"), "PAR(PNT(Sheet.1!Connections.X3,Sheet.1!Connections.Y3))");
    let p = path(&f.doc, s);
    assert!(near(p[0], [2.0, 3.5]) && near(*p.last().unwrap(), [5.0, 1.5]));
    // every point on the ellipse around (5, 3.5) with half axes 3 and 2
    for q in &p {
        let (u, v) = ((q[0] - 5.0) / 3.0, (q[1] - 3.5) / 2.0);
        assert!((u * u + v * v - 1.0).abs() < 1e-3, "{q:?}");
    }
    // leaving horizontally: the other quarter, around (2, 1.5)
    let mut tx = f.doc.transaction("Bogen");
    let t = crate::ops::new_arc(&mut tx, &mut f.eng, f.owner, ([2.0, 3.5], Target::None), ([5.0, 1.5], Target::None), true).unwrap();
    tx.commit().unwrap();
    for q in &path(&f.doc, t) {
        let (u, v) = ((q[0] - 2.0) / 3.0, (q[1] - 1.5) / 2.0);
        assert!((u * u + v * v - 1.0).abs() < 1e-3, "{q:?}");
    }
    // A moves: the arc follows from its begin, its other end stays
    f.move_to(a, [1.0, 4.0]).unwrap();
    let p = path(&f.doc, s);
    assert!(near(p[0], [1.0, 3.5]) && near(*p.last().unwrap(), [5.0, 1.5]));
    // begin = end is refused
    let mut tx = f.doc.transaction("Bogen");
    assert_eq!(crate::ops::new_arc(&mut tx, &mut f.eng, f.owner, ([1.0, 1.0], Target::None), ([1.0, 1.0], Target::None), true), Err(EditError::Invalid));
}
