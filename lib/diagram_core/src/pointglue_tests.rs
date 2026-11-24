// tests of pointglue (cargo test -p jflib-diagram-core --lib)
//
//   1. a control point glues as Visio writes it (LOC(PNT(..)), Connect from Controls.Row_k),
//      follows its target, stays when its own shape moves, is let go; outward points refused;
//      followers
//   2. anchors: a connection point made at a vertex, a handle, a control point (and used again);
//      an end glued there follows a vertex edit
//   3. guides: an end on a horizontal and on a vertical guide follows the guide
//   4. the connection point tool: add, move (form kept), kind, delete (glue let go and renumbered)
//
// Every document is built here; nothing is read from files.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::formula::Engine;
use crate::{Cell, Change, DocKind, Page, PageHeader, PageRef, Row, Section, ShapeData, Unit};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + b.abs())
}

macro_rules! assert_pt {
    ($a:expr, $b:expr) => {{
        let (a, b): ([f64; 2], [f64; 2]) = ($a, $b);
        assert!(close(a[0], b[0]) && close(a[1], b[1]), "{} = {:?}, not {:?}", stringify!($a), a, b);
    }};
}

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

fn row(key: RowKey, kind: Option<crate::Sym>, cells: Vec<Cell>) -> Row {
    let mut r = Row::new(key);
    r.kind = kind;
    r.cells = cells;
    r
}

impl Fx {
    fn insert(&mut self, id: u32, d: ShapeData) -> ShapeRef {
        let tree = &mut self.doc.page_mut(self.page).unwrap().shapes;
        let slot = tree.insert(None, id, d).unwrap();
        ShapeRef { owner: self.owner, slot }
    }

    fn frame_cells(&mut self, d: &mut ShapeData, at: [f64; 2], size: [f64; 2]) {
        let doc = &mut self.doc;
        let (w, h) = (size[0], size[1]);
        let cells = vec![
            cnum(doc, "PinX", at[0]),
            cnum(doc, "PinY", at[1]),
            cnum(doc, "Width", w),
            cnum(doc, "Height", h),
            cf(doc, "LocPinX", "Width*0.5", w * 0.5),
            cf(doc, "LocPinY", "Height*0.5", h * 0.5),
            cnum(doc, "Angle", 0.0),
        ];
        d.sheet.cells.extend(cells);
    }

    // a rectangle w x h around `at` (Geometry with absolute rows) with connection points in the
    // middle of the left, right, bottom and top side (rows 0-3) of the kinds given, and one in
    // the middle (row 4, kind 0)
    fn rect(&mut self, id: u32, at: [f64; 2], size: [f64; 2], kinds: [u8; 4]) -> ShapeRef {
        let mut d = ShapeData::new(ShapeKind::Shape);
        self.frame_cells(&mut d, at, size);
        let doc = &mut self.doc;
        let (w, h) = (size[0], size[1]);
        let mut geo = Section::new(SecKey::one(names::GEOMETRY));
        geo.ix_written = true;
        for (i, p) in [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h], [0.0, 0.0]].iter().enumerate() {
            let kind = if i == 0 { names::MOVE_TO } else { names::LINE_TO };
            geo.rows.push(row(RowKey::Index(i as u32 + 1), Some(kind), vec![Cell::constant(names::X, Value::Num(p[0]), None), Cell::constant(names::Y, Value::Num(p[1]), None)]));
        }
        d.sheet.sections.push(geo);
        let mut con = Section::new(SecKey::one(names::CONNECTION));
        for (i, (fx, fy)) in [(0.0, 0.5), (1.0, 0.5), (0.5, 0.0), (0.5, 1.0), (0.5, 0.5)].into_iter().enumerate() {
            let kind = kinds.get(i).copied().unwrap_or(0) as f64;
            let cells = vec![cf(doc, "X", &alloc::format!("Width*{fx}"), w * fx), cf(doc, "Y", &alloc::format!("Height*{fy}"), h * fy), cnum(doc, "DirX", 0.0), cnum(doc, "DirY", 0.0), cnum(doc, "Type", kind)];
            con.rows.push(row(RowKey::Index(i as u32), None, cells));
        }
        d.sheet.sections.push(con);
        self.insert(id, d)
    }

    // a bus: a 2-D shape w x 0.25 around `at` with two control points that glue (Row_1, Row_2)
    // above it, and a Geometry line to each of them
    fn bus(&mut self, id: u32, at: [f64; 2], w: f64) -> ShapeRef {
        let mut d = ShapeData::new(ShapeKind::Shape);
        self.frame_cells(&mut d, at, [w, 0.25]);
        let doc = &mut self.doc;
        let mut ctl = Section::new(SecKey::one(names::CONTROL));
        for (k, fx) in [(1, 0.25), (2, 0.75)] {
            let name = doc.intern(&alloc::format!("Row_{k}"));
            let cells = vec![cf(doc, "X", &alloc::format!("Width*{fx}"), w * fx), cf(doc, "Y", "Height+1", 1.25), cnum(doc, "XCon", 0.0), cnum(doc, "YCon", 2.0), cnum(doc, "CanGlue", 1.0)];
            ctl.rows.push(row(RowKey::Named(name), None, cells));
        }
        d.sheet.sections.push(ctl);
        let mut geo = Section::new(SecKey::one(names::GEOMETRY));
        geo.ix_written = true;
        geo.rows.push(row(RowKey::Index(1), Some(names::MOVE_TO), vec![cf(doc, "X", "Controls.Row_1", w * 0.25), cnum(doc, "Y", 0.25)]));
        geo.rows.push(row(RowKey::Index(2), Some(names::LINE_TO), vec![cf(doc, "X", "Controls.Row_1", w * 0.25), cf(doc, "Y", "Controls.Row_1.Y", 1.25)]));
        d.sheet.sections.push(geo);
        self.insert(id, d)
    }

    // a guide across the page at height y (horizontal) or up it at x
    fn guide(&mut self, id: u32, at: f64, horizontal: bool) -> ShapeRef {
        let mut d = ShapeData::new(ShapeKind::Guide);
        let doc = &mut self.doc;
        let cells = vec![
            cnum(doc, "PinX", if horizontal { 0.0 } else { at }),
            cnum(doc, "PinY", if horizontal { at } else { 0.0 }),
            cnum(doc, "Width", 0.0),
            cnum(doc, "Height", 0.0),
            cnum(doc, "LocPinX", 0.0),
            cnum(doc, "LocPinY", 0.0),
            cnum(doc, "Angle", if horizontal { 0.0 } else { core::f64::consts::FRAC_PI_2 }),
        ];
        d.sheet.cells.extend(cells);
        let mut geo = Section::new(SecKey::one(names::GEOMETRY));
        geo.ix_written = true;
        let line = doc.intern("InfiniteLine");
        geo.rows.push(row(RowKey::Index(1), Some(line), vec![cnum(doc, "X", 0.0), cnum(doc, "Y", 0.0), cnum(doc, "A", 1.0), cnum(doc, "B", 0.0)]));
        d.sheet.sections.push(geo);
        self.insert(id, d)
    }

    fn edit<R>(&mut self, label: &str, f: impl FnOnce(&mut Transaction, &mut Engine) -> Result<R, EditError>) -> R {
        let mut tx = self.doc.transaction(label);
        let r = f(&mut tx, &mut self.eng).unwrap();
        tx.commit();
        r
    }

    // move a shape's pin as a gesture would; 1-D shapes glued to it are updated
    fn move_to(&mut self, s: ShapeRef, at: [f64; 2]) -> Option<Change> {
        let owner = self.owner;
        let mut tx = self.doc.transaction("Verschieben");
        let sheet = SheetRef::Shape(s);
        let (px, py) = (CellAddr::single(names::PIN_X), CellAddr::single(names::PIN_Y));
        tx.set_length(sheet, &px, at[0], Unit::Inches).unwrap();
        tx.set_length(sheet, &py, at[1], Unit::Inches).unwrap();
        self.eng.recalc_cells(&mut tx, &[(sheet, px), (sheet, py)]).unwrap();
        let deps = glue::dependents(tx.doc(), owner, &[s]);
        glue::update(&mut tx, &mut self.eng, &deps).unwrap();
        tx.commit()
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

    fn formula(&self, s: ShapeRef, addr: &CellAddr) -> String {
        formula_of(&self.doc, SheetRef::Shape(s), addr)
    }

    fn value(&self, s: ShapeRef, addr: &CellAddr) -> f64 {
        self.doc.resolve(SheetRef::Shape(s), addr).num().unwrap()
    }

    fn row_key(&self, name: &str) -> RowKey {
        RowKey::Named(self.doc.sym(name).unwrap())
    }

    fn ctl(&self, name: &str, cell: crate::Sym) -> CellAddr {
        CellAddr::row(control_key(), self.row_key(name), cell)
    }

    fn conn(&self, ix: u32, cell: crate::Sym) -> CellAddr {
        CellAddr::row(conn_key(), RowKey::Index(ix), cell)
    }
}

fn control_at(doc: &Document, s: ShapeRef, name: &str) -> [f64; 2] {
    ops::controls(doc, s).into_iter().find(|c| c.name == name).unwrap().at
}

// ---- 1. control points

#[test]
fn a_control_point_glues_as_visio_writes_it_and_follows() {
    let mut f = fx();
    let bus = f.bus(1, [4.0, 2.0], 4.0);
    let pc = f.rect(2, [3.0, 5.0], [1.0, 1.0], [0, 0, 0, 0]);
    let row1 = f.row_key("Row_1");
    // the bottom point of the computer (row 2)
    let at = glue::connection_points(&f.doc, pc)[2].at;
    assert_pt!(at, [3.0, 4.5]);
    f.edit("Steuerpunkt", |tx, eng| glue_control(tx, eng, bus, row1, at, Target::Point { shape: pc, row: 2 }));
    let want = "LOC(PNT(Sheet.2!Connections.X3,Sheet.2!Connections.Y3))";
    assert_eq!(f.formula(bus, &f.ctl("Row_1", names::X)), want);
    assert_eq!(f.formula(bus, &f.ctl("Row_1", names::Y)), want);
    assert_eq!(f.connects(), vec![(1, String::from("Controls.Row_1"), 100, 2, String::from("Connections.X3"), 102)]);
    assert_pt!(control_at(&f.doc, bus, "Row_1"), at);
    assert_eq!(glued_controls(&f.doc, bus), vec![(row1, Target::Point { shape: pc, row: 2 })]);
    // the computer moves: the tap follows (the engine), and so does the bus's line to it
    f.move_to(pc, [5.0, 6.0]);
    assert_pt!(control_at(&f.doc, bus, "Row_1"), [5.0, 5.5]);
    let v = ops::vertices(&f.doc, bus);
    assert_pt!(v[1].at, [5.0, 5.5]);
    assert_eq!(followers(&f.doc, f.owner, &[pc]), vec![bus]);
    assert!(followers(&f.doc, f.owner, &[bus]).is_empty());
    // the bus moves: the tap stays on the computer
    f.move_to(bus, [3.0, 1.0]);
    assert_pt!(control_at(&f.doc, bus, "Row_1"), [5.0, 5.5]);
    // the second tap to the same computer's top point: its FromPart is 101
    let row2 = f.row_key("Row_2");
    let top = glue::connection_points(&f.doc, pc)[3].at;
    f.edit("Steuerpunkt", |tx, eng| glue_control(tx, eng, bus, row2, top, Target::Point { shape: pc, row: 3 }));
    assert_eq!(f.connects()[1], (1, String::from("Controls.Row_2"), 101, 2, String::from("Connections.X4"), 103));
    // let go: the value stays where it was put, the Connect entry goes
    f.edit("Steuerpunkt", |tx, eng| glue_control(tx, eng, bus, row1, [2.0, 3.0], Target::None));
    assert_eq!(f.connects().len(), 1);
    assert_pt!(control_at(&f.doc, bus, "Row_1"), [2.0, 3.0]);
    assert!(!f.formula(bus, &f.ctl("Row_1", names::X)).starts_with("LOC"));
    f.move_to(pc, [7.0, 6.0]);
    assert_pt!(control_at(&f.doc, bus, "Row_1"), [2.0, 3.0]);
}

#[test]
fn control_points_glue_to_inward_and_both_but_not_outward_points() {
    let mut f = fx();
    let bus = f.bus(1, [4.0, 2.0], 4.0);
    let pc = f.rect(2, [3.0, 5.0], [1.0, 1.0], [1, 2, 0, 0]);
    let row1 = f.row_key("Row_1");
    let pts = glue::connection_points(&f.doc, pc);
    let mut tx = f.doc.transaction("x");
    assert_eq!(glue_control(&mut tx, &mut f.eng, bus, row1, pts[0].at, Target::Point { shape: pc, row: 0 }), Err(EditError::Invalid));
    assert_eq!(glue_control(&mut tx, &mut f.eng, bus, row1, pts[0].at, Target::Shape(pc)), Err(EditError::Invalid));
    assert_eq!(glue_control(&mut tx, &mut f.eng, bus, row1, pts[0].at, Target::Point { shape: pc, row: 9 }), Err(EditError::NoRow));
    assert_eq!(glue_control(&mut tx, &mut f.eng, bus, row1, pts[0].at, Target::Point { shape: bus, row: 0 }), Err(EditError::Invalid));
    // kind 2 (both) takes it
    glue_control(&mut tx, &mut f.eng, bus, row1, pts[1].at, Target::Point { shape: pc, row: 1 }).unwrap();
    tx.commit();
    assert_eq!(f.connects().len(), 1);
}

#[test]
fn a_glued_control_point_is_undone_and_let_go_when_its_target_goes() {
    let mut f = fx();
    let bus = f.bus(1, [4.0, 2.0], 4.0);
    let pc = f.rect(2, [3.0, 5.0], [1.0, 1.0], [0; 4]);
    let row1 = f.row_key("Row_1");
    let before = f.formula(bus, &f.ctl("Row_1", names::X));
    let at = glue::connection_points(&f.doc, pc)[2].at;
    let mut tx = f.doc.transaction("Steuerpunkt");
    glue_control(&mut tx, &mut f.eng, bus, row1, at, Target::Point { shape: pc, row: 2 }).unwrap();
    let mut ch = tx.commit().unwrap();
    assert!(ch.undo(&mut f.doc));
    assert_eq!(f.formula(bus, &f.ctl("Row_1", names::X)), before);
    assert!(f.connects().is_empty());
    assert!(ch.redo(&mut f.doc));
    assert_eq!(f.connects().len(), 1);
    f.eng.refresh(&f.doc, &ch.sheets());
    // the computer is deleted: the tap keeps its place as a value
    let owner = f.owner;
    f.edit("Löschen", |tx, eng| {
        glue::detach(tx, eng, owner, &[pc])?;
        tx.delete_shapes(&[pc])
    });
    assert!(f.connects().is_empty());
    assert_eq!(f.formula(bus, &f.ctl("Row_1", names::X)), "");
    assert!(close(f.value(bus, &f.ctl("Row_1", names::X)), at[0] - 2.0));
}

// ---- 2. anchors

#[test]
fn anchors_make_connection_points_once_and_ends_follow_them() {
    let mut f = fx();
    let a = f.rect(1, [2.0, 2.0], [2.0, 1.0], [0; 4]);
    let bus = f.bus(2, [6.0, 6.0], 4.0);
    // the vertex at the top right (Geometry1 row 3)
    let v = Anchor::Vertex { section: 0, row: 3 };
    assert_pt!(anchor_at(&f.doc, a, v).unwrap(), [3.0, 2.5]);
    let ix = f.edit("Kleben", |tx, eng| anchor_point(tx, eng, a, v));
    assert_eq!(ix, 5);
    assert_eq!(f.formula(a, &f.conn(5, names::X)), "Geometry1.X3");
    assert_eq!(f.formula(a, &f.conn(5, names::Y)), "Geometry1.Y3");
    assert_eq!(f.value(a, &f.conn(5, names::TYPE)), 0.0);
    assert_eq!(f.edit("Kleben", |tx, eng| anchor_point(tx, eng, a, v)), 5);
    // a line glued there follows the vertex
    let owner = f.owner;
    let line = f.edit("Linie", |tx, eng| glue::new_line(tx, eng, owner, ([0.0, 5.0], Target::None), ([3.0, 2.5], Target::Point { shape: a, row: 5 })));
    f.edit("Scheitelpunkt", |tx, eng| ops::set_vertex(tx, eng, a, 0, 3, [3.5, 3.0]));
    let deps = glue::dependents(&f.doc, owner, &[a]);
    f.edit("x", |tx, eng| glue::update(tx, eng, &deps));
    assert_pt!(glue::ends(&f.doc, line).unwrap().1, [3.5, 3.0]);
    // a handle: the middle of the right side is connection point 1 already; the top right
    // corner gets a new one
    assert_eq!(f.edit("Kleben", |tx, eng| anchor_point(tx, eng, a, Anchor::Handle { fx: 1.0, fy: 0.5 })), 1);
    let h = Anchor::Handle { fx: 1.0, fy: 1.0 };
    let ih = f.edit("Kleben", |tx, eng| anchor_point(tx, eng, a, h));
    assert_eq!(ih, 6);
    assert_eq!(f.formula(a, &f.conn(6, names::X)), "Width*1");
    assert_eq!(f.formula(a, &f.conn(6, names::Y)), "Height*1");
    assert_pt!(point_at(&f.doc, a, 6).unwrap(), anchor_at(&f.doc, a, h).unwrap());
    // a control point of the bus
    let c = Anchor::Control(f.row_key("Row_2"));
    let ic = f.edit("Kleben", |tx, eng| anchor_point(tx, eng, bus, c));
    assert_eq!(ic, 0);
    assert_eq!(f.formula(bus, &f.conn(0, names::X)), "Controls.Row_2");
    assert_eq!(f.formula(bus, &f.conn(0, names::Y)), "Controls.Row_2.Y");
    assert_pt!(point_at(&f.doc, bus, 0).unwrap(), control_at(&f.doc, bus, "Row_2"));
    // no point at an InfiniteLine, a missing row
    let mut tx = f.doc.transaction("x");
    assert_eq!(anchor_point(&mut tx, &mut f.eng, a, Anchor::Vertex { section: 0, row: 9 }), Err(EditError::NoRow));
}

// ---- 3. guides

#[test]
fn ends_glue_to_guides_and_follow_them() {
    let mut f = fx();
    let g = f.guide(1, 3.0, true);
    let gv = f.guide(2, 6.0, false);
    assert!(is_glue_guide(&f.doc, g) && is_glue_guide(&f.doc, gv));
    let owner = f.owner;
    let line = f.edit("Linie", |tx, eng| glue::new_line(tx, eng, owner, ([1.0, 1.0], Target::None), ([2.0, 2.9], Target::None)));
    f.edit("Kleben", |tx, eng| glue_guide(tx, eng, line, End::End, [2.0, 2.9], g));
    assert_eq!(f.formula(line, &CellAddr::single(names::END_Y)), "Sheet.1!PinY");
    assert_pt!(glue::ends(&f.doc, line).unwrap().1, [2.0, 3.0]);
    let id = f.doc.shape(line).unwrap().id();
    assert_eq!(f.connects(), vec![(id, String::from("EndX"), 12, 1, String::from("Angle"), 2)]);
    assert_eq!(guide_of(&f.doc, line), [None, Some(g)]);
    assert_eq!(glue::glued(&f.doc, line), [Target::None, Target::None]);
    // the guide moves up: the end follows, its x stays
    f.move_to(g, [0.0, 4.0]);
    assert_pt!(glue::ends(&f.doc, line).unwrap().1, [2.0, 4.0]);
    // the begin on the vertical guide
    f.edit("Kleben", |tx, eng| glue_guide(tx, eng, line, End::Begin, [6.2, 1.0], gv));
    assert_eq!(f.formula(line, &CellAddr::single(names::BEGIN_X)), "Sheet.2!PinX");
    f.move_to(gv, [7.0, 0.0]);
    assert_pt!(glue::ends(&f.doc, line).unwrap().0, [7.0, 1.0]);
    assert_eq!(guide_of(&f.doc, line), [Some(gv), Some(g)]);
    // glued elsewhere: the guide's entry goes
    f.edit("Kleben", |tx, eng| glue::glue(tx, eng, line, End::End, [5.0, 5.0], Target::None));
    assert_eq!(guide_of(&f.doc, line), [Some(gv), None]);
    // not a guide, a dynamic connector to a guide routes with the end in place
    let mut tx = f.doc.transaction("x");
    assert_eq!(glue_guide(&mut tx, &mut f.eng, line, End::End, [5.0, 5.0], line), Err(EditError::Invalid));
    drop(tx);
    let c = f.edit("Verbinder", |tx, eng| glue::new_connector(tx, eng, owner, ([1.0, 6.0], Target::None), ([4.0, 8.0], Target::None)));
    f.edit("Kleben", |tx, eng| glue_guide(tx, eng, c, End::End, [4.0, 4.2], g));
    assert_pt!(glue::ends(&f.doc, c).unwrap().1, [4.0, 4.0]);
    let way = glue::path(&f.doc, c);
    assert_pt!(*way.last().unwrap(), [4.0, 4.0]);
}

// ---- 4. the connection point tool

#[test]
fn the_connection_point_tool_adds_moves_and_deletes_points() {
    let mut f = fx();
    let a = f.rect(1, [2.0, 2.0], [2.0, 1.0], [0; 4]);
    let bus = f.bus(2, [6.0, 6.0], 4.0);
    let ix = f.edit("Verbindungspunkt", |tx, eng| add_point(tx, eng, a, [1.5, 2.25]));
    assert_eq!(ix, 5);
    assert_eq!(f.formula(a, &f.conn(5, names::X)), "Width*0.25");
    assert_eq!(f.formula(a, &f.conn(5, names::Y)), "Height*0.75");
    assert_pt!(point_at(&f.doc, a, 5).unwrap(), [1.5, 2.25]);
    assert_eq!(point_kind(&f.doc, a, 5), Some(0));
    // moved: still proportional
    f.edit("Verbindungspunkt", |tx, eng| move_point(tx, eng, a, 5, [2.5, 1.75]));
    assert_eq!(f.formula(a, &f.conn(5, names::X)), "Width*0.75");
    assert_eq!(f.formula(a, &f.conn(5, names::Y)), "Height*0.25");
    // its kind
    f.edit("Art", |tx, eng| set_point_kind(tx, eng, a, 5, 2));
    assert_eq!(point_kind(&f.doc, a, 5), Some(2));
    // glue to row 1 (a line), row 3 (a connector) and row 5 (a tap of the bus)
    let owner = f.owner;
    let p1 = point_at(&f.doc, a, 1).unwrap();
    let p3 = point_at(&f.doc, a, 3).unwrap();
    let p5 = point_at(&f.doc, a, 5).unwrap();
    let line = f.edit("Linie", |tx, eng| glue::new_line(tx, eng, owner, ([5.0, 2.0], Target::None), (p1, Target::Point { shape: a, row: 1 })));
    let conn = f.edit("Verbinder", |tx, eng| glue::new_connector(tx, eng, owner, ([2.0, 6.0], Target::None), (p3, Target::Point { shape: a, row: 3 })));
    let row1 = f.row_key("Row_1");
    f.edit("Steuerpunkt", |tx, eng| glue_control(tx, eng, bus, row1, p5, Target::Point { shape: a, row: 5 }));
    // row 1 goes: the line lets go there, the others are glued to rows 2 and 4 now
    f.edit("Verbindungspunkt löschen", |tx, eng| delete_point(tx, eng, a, 1));
    assert_eq!(glue::glued(&f.doc, line)[1], Target::None);
    assert_pt!(glue::ends(&f.doc, line).unwrap().1, p1);
    assert_eq!(glue::glued(&f.doc, conn)[1], Target::Point { shape: a, row: 2 });
    assert_eq!(f.formula(conn, &CellAddr::single(names::END_X)), "PAR(PNT(Sheet.1!Connections.X3,Sheet.1!Connections.Y3))");
    assert_pt!(glue::ends(&f.doc, conn).unwrap().1, p3);
    assert_eq!(glued_controls(&f.doc, bus), vec![(row1, Target::Point { shape: a, row: 4 })]);
    assert_eq!(f.formula(bus, &f.ctl("Row_1", names::X)), "LOC(PNT(Sheet.1!Connections.X5,Sheet.1!Connections.Y5))");
    assert_pt!(control_at(&f.doc, bus, "Row_1"), p5);
    assert_eq!(glue::connection_points(&f.doc, a).len(), 5);
    assert_eq!(point_kind(&f.doc, a, 4), Some(2));
    // the shape moves: both still follow
    f.move_to(a, [3.0, 2.0]);
    assert_pt!(glue::ends(&f.doc, conn).unwrap().1, [p3[0] + 1.0, p3[1]]);
    assert_pt!(control_at(&f.doc, bus, "Row_1"), [p5[0] + 1.0, p5[1]]);
}
