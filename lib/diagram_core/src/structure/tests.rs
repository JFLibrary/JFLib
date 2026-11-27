// tests of structure (cargo test -p jflib-diagram-core --lib structure)
//
//   the Relationships cell (read, written as Visio writes it, both sides), membership by
//   position after a move (in, out, partly out, nested, two overlapping containers, locked,
//   categories), automatic size (expand, fit), what moves and goes with a container, lists
//   (order, layout, reorder, insert marks, a gap closed on delete), disband, repair after a
//   one-sided edit, callouts (target, anchor, the tail aimed), the structure functions of the
//   formula engine; every edit is undone and redone
//
// Every document is built here; nothing comes from a Visio file.

use alloc::string::String;
use alloc::vec::Vec;

use super::*;
use crate::cell::{Cell, Formula};
use crate::formula::{Engine, Outcome, Val};
use crate::{Change, DocKind, Page, PageHeader, PageRef, ShapeData, ShapeKind, Style, StyleRefs};

struct Fx {
    doc: Document,
    page: PageRef,
    owner: Owner,
    eng: Engine,
    next: u32,
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + b.abs())
}

macro_rules! assert_box {
    ($a:expr, $b:expr) => {{
        let (a, b): ([f64; 4], [f64; 4]) = ($a, $b);
        assert!((0..4).all(|k| close(a[k], b[k])), "{}: {:?} is not {:?}", stringify!($a), a, b);
    }};
}

fn fx() -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    for (id, name, base) in [(0u32, "No Style", None), (1, "Normal", Some(0u32))] {
        let mut st = Style::default();
        st.header.id = id;
        st.header.name_u = Some(doc.intern(name));
        st.header.name = Some(doc.intern(name));
        if let Some(b) = base {
            st.header.based_on = StyleRefs::all(b);
        }
        doc.insert_style(st).unwrap();
    }
    let mut ph = PageHeader { id: 0, ..PageHeader::default() };
    ph.name_u = Some(doc.intern("Page-1"));
    let page = doc.insert_page(Page::new(ph)).unwrap();
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
    // an upright box x0, y0, x1, y1
    fn rect(&mut self, b: [f64; 4]) -> ShapeRef {
        let doc = &mut self.doc;
        let (w, h) = (b[2] - b[0], b[3] - b[1]);
        let mut d = ShapeData::new(ShapeKind::Shape);
        d.sheet.cells = alloc::vec![
            cnum(doc, "PinX", (b[0] + b[2]) * 0.5),
            cnum(doc, "PinY", (b[1] + b[3]) * 0.5),
            cnum(doc, "Width", w),
            cnum(doc, "Height", h),
            cf(doc, "LocPinX", "Width*0.5", w * 0.5),
            cf(doc, "LocPinY", "Height*0.5", h * 0.5),
        ];
        let id = self.next;
        self.next += 1;
        let slot = self.doc.page_mut(self.page).unwrap().shapes.insert(None, id, d).unwrap();
        ShapeRef { owner: self.owner, slot }
    }

    // a container (or list) with a heading on top of `head` inches
    fn container(&mut self, b: [f64; 4], kind: &str, head: f64, resize: f64) -> ShapeRef {
        let c = self.rect(b);
        self.edit(|tx, _| {
            let s = SheetRef::Shape(c);
            let k = tx.intern(kind);
            tx.set_user(s, "msvStructureType", Value::Text(k), Some(Unit::Str))?;
            tx.set_user(s, "msvSDContainerMargin", Value::Num(0.1), Some(Unit::Inches))?;
            tx.set_user(s, "msvSDContainerResize", Value::Num(resize), None)?;
            if head > 0.0 {
                let t = tx.intern("Top");
                tx.set_user(s, "JFD_HeadingEdge", Value::Text(t), Some(Unit::Str))?;
                tx.set_user(s, "JFD_HeadingSize", Value::Num(head), Some(Unit::Inches))?;
            }
            Ok(())
        });
        c
    }

    fn list(&mut self, b: [f64; 4], dir: i64, spacing: f64) -> ShapeRef {
        let l = self.container(b, "List", 0.0, 2.0);
        self.edit(|tx, _| {
            let s = SheetRef::Shape(l);
            tx.set_user(s, "msvSDListDirection", Value::Num(dir as f64), None)?;
            tx.set_user(s, "msvSDListSpacing", Value::Num(spacing), Some(Unit::Inches))
        });
        l
    }

    fn user(&mut self, s: ShapeRef, name: &str, v: &str) {
        self.edit(|tx, _| {
            let k = tx.intern(v);
            tx.set_user(SheetRef::Shape(s), name, Value::Text(k), Some(Unit::Str))
        });
    }

    // one transaction; it is undone and redone once to see that it round-trips
    fn edit<T>(&mut self, f: impl FnOnce(&mut Transaction, &mut Engine) -> Result<T, EditError>) -> T {
        let mut tx = self.doc.transaction("test");
        let r = f(&mut tx, &mut self.eng).expect("edit");
        let c: Option<Change> = tx.commit();
        if let Some(mut c) = c {
            let before = self.snapshot();
            assert!(c.undo(&mut self.doc));
            assert!(c.redo(&mut self.doc));
            assert_eq!(before, self.snapshot(), "undo + redo");
            self.eng.refresh(&self.doc, &c.sheets());
        }
        r
    }

    // what the tests look at: every shape's box and Relationships formula
    fn snapshot(&self) -> Vec<(u32, [i64; 4], Option<String>)> {
        self.doc
            .all_shapes(self.owner)
            .map(|s| {
                let b = shape_box(&self.doc, s);
                let q = |x: f64| (x * 1e6).round() as i64;
                (self.doc.shape(s).unwrap().id(), [q(b[0]), q(b[1]), q(b[2]), q(b[3])], rel::formula(&self.doc, s).map(String::from))
            })
            .collect()
    }

    fn id(&self, s: ShapeRef) -> u32 {
        self.doc.shape(s).unwrap().id()
    }

    fn rel_text(&self, s: ShapeRef) -> String {
        rel::formula(&self.doc, s).map(String::from).unwrap_or_default()
    }

    fn move_by(&mut self, shapes: &[ShapeRef], d: [f64; 2]) -> Vec<ShapeRef> {
        let owner = self.owner;
        let shapes = shapes.to_vec();
        self.edit(|tx, eng| {
            let along = carried(tx.doc(), &shapes);
            ops::move_shapes(tx, eng, &along, d)?;
            settle(tx, eng, owner, &shapes, true)
        })
    }
}

#[test]
fn relationships_parse_and_write() {
    let groups = rel::parse("SUM(DEPENDSON(1,Sheet.566!SheetRef(),Sheet.565!SheetRef()),DEPENDSON(4,Sheet.7!SheetRef()))");
    assert_eq!(groups, alloc::vec![(1, alloc::vec![566, 565]), (4, alloc::vec![7])]);
    assert_eq!(rel::parse("SUM(DEPENDSON(4))"), alloc::vec![(4, Vec::new())]);
    assert!(rel::parse("0").is_empty());
    // cut off: an open quote ran the scan one past the end, an open call cut its last letter
    // in two (both panicked, third review); what is there still counts
    assert_eq!(rel::parse("DEPENDSON(1,\""), alloc::vec![(1, Vec::new())]);
    assert_eq!(rel::parse("DEPENDSON(1,\"ä"), alloc::vec![(1, Vec::new())]);
    assert_eq!(rel::parse("DEPENDSON(1,ä"), alloc::vec![(1, Vec::new())]);
    assert_eq!(rel::parse("DEPENDSON(1,Sheet.3!SheetRef()"), alloc::vec![(1, alloc::vec![3])]);

    let mut f = fx();
    let c = f.container([0.0, 0.0, 4.0, 3.0], "Container", 0.0, 1.0);
    let a = f.rect([1.0, 1.0, 2.0, 2.0]);
    let b = f.rect([2.5, 1.0, 3.5, 2.0]);
    f.edit(|tx, eng| add_members(tx, eng, c, &[a, b], None));
    let (ia, ib, ic) = (f.id(a), f.id(b), f.id(c));
    assert_eq!(f.rel_text(c), alloc::format!("SUM(DEPENDSON(1,Sheet.{ia}!SheetRef(),Sheet.{ib}!SheetRef()))"));
    assert_eq!(f.rel_text(a), alloc::format!("SUM(DEPENDSON(4,Sheet.{ic}!SheetRef()))"));
    assert_eq!(members(&f.doc, c), alloc::vec![a, b]);
    assert_eq!(containers_of(&f.doc, b), alloc::vec![c]);
    // the value is 0 as in Visio's files
    let addr = CellAddr::single(f.doc.sym("Relationships").unwrap());
    assert_eq!(f.doc.resolve(SheetRef::Shape(c), &addr).num(), Some(0.0));
    // the engine evaluates it to 0 too
    assert!(matches!(f.eng.evaluate(&f.doc, SheetRef::Shape(c), &addr), Outcome::Value(Val::Num(x)) if x == 0.0));
    f.edit(|tx, eng| remove_members(tx, eng, c, &[a]));
    assert_eq!(members(&f.doc, c), alloc::vec![b]);
    assert!(containers_of(&f.doc, a).is_empty());
    assert_eq!(f.rel_text(a), "");
}

#[test]
fn membership_follows_moves() {
    let mut f = fx();
    let c = f.container([0.0, 0.0, 4.0, 4.0], "Container", 0.5, 1.0);
    let a = f.rect([5.0, 1.0, 6.0, 2.0]);
    // dragged in wholly: a member
    f.move_by(&[a], [-4.0, 0.0]);
    assert_eq!(members(&f.doc, c), alloc::vec![a]);
    // partly out: stays, the container grows (expand) by the margin around it
    f.move_by(&[a], [2.5, 0.0]);
    assert_eq!(members(&f.doc, c), alloc::vec![a]);
    assert_box!(shape_box(&f.doc, c), [0.0, 0.0, 4.6, 4.0]);
    // wholly out: no longer a member
    f.move_by(&[a], [3.0, 0.0]);
    assert!(members(&f.doc, c).is_empty());
    assert!(containers_of(&f.doc, a).is_empty());
}

#[test]
fn container_carries_its_members() {
    let mut f = fx();
    let c = f.container([0.0, 0.0, 4.0, 4.0], "Container", 0.0, 1.0);
    let inner = f.container([0.5, 0.5, 2.5, 2.5], "Container", 0.0, 1.0);
    let a = f.rect([1.0, 1.0, 2.0, 2.0]);
    let b = f.rect([3.0, 3.0, 3.5, 3.5]);
    f.move_by(&[inner], [0.0, 0.0]);
    f.move_by(&[a], [0.0, 0.0]);
    f.move_by(&[b], [0.0, 0.0]);
    // nesting: a is in both (Visio keeps membership in every enclosing container)
    let mut ca = containers_of(&f.doc, a);
    ca.sort();
    let mut want = alloc::vec![c, inner];
    want.sort();
    assert_eq!(ca, want);
    assert_eq!(members(&f.doc, c), alloc::vec![inner, a, b]);
    let all = carried(&f.doc, &[c]);
    assert_eq!(all.len(), 4);
    f.move_by(&[c], [10.0, 5.0]);
    assert_box!(shape_box(&f.doc, a), [11.0, 6.0, 12.0, 7.0]);
    assert_box!(shape_box(&f.doc, b), [13.0, 8.0, 13.5, 8.5]);
    assert_eq!(members(&f.doc, c), alloc::vec![inner, a, b]);
    assert_eq!(containers_of(&f.doc, a).len(), 2);
}

#[test]
fn overlapping_containers_both_take_a_shape() {
    // a lane and a phase of a cross-functional flowchart
    let mut f = fx();
    let lane = f.container([0.0, 0.0, 10.0, 2.0], "Container", 0.0, 0.0);
    let phase = f.container([4.0, -1.0, 8.0, 3.0], "Container", 0.0, 0.0);
    let a = f.rect([5.0, 0.5, 6.0, 1.5]);
    f.move_by(&[a], [0.0, 0.0]);
    let mut cs = containers_of(&f.doc, a);
    cs.sort();
    let mut want = alloc::vec![lane, phase];
    want.sort();
    assert_eq!(cs, want);
}

#[test]
fn locks_and_categories() {
    let mut f = fx();
    let c = f.container([0.0, 0.0, 4.0, 4.0], "Container", 0.0, 0.0);
    f.edit(|tx, _| tx.set_user(SheetRef::Shape(c), "msvSDContainerLocked", Value::Num(1.0), None));
    let a = f.rect([1.0, 1.0, 2.0, 2.0]);
    f.move_by(&[a], [0.0, 0.0]);
    assert!(members(&f.doc, c).is_empty(), "a locked container takes nothing");
    f.edit(|tx, _| tx.set_user(SheetRef::Shape(c), "msvSDContainerLocked", Value::Num(0.0), None));
    f.user(c, "msvSDContainerExcludedCategories", "DoNotContain");
    f.user(a, "msvShapeCategories", "Foo;DoNotContain");
    f.move_by(&[a], [0.0, 0.0]);
    assert!(members(&f.doc, c).is_empty(), "an excluded category");
    f.user(c, "msvSDContainerExcludedCategories", "");
    f.user(c, "msvSDContainerRequiredCategories", "Swimlane;Bar");
    f.move_by(&[a], [0.0, 0.0]);
    assert!(members(&f.doc, c).is_empty(), "a required category missing");
    f.user(a, "msvShapeCategories", "bar");
    f.move_by(&[a], [0.0, 0.0]);
    assert_eq!(members(&f.doc, c), alloc::vec![a]);
    assert!(has_category(&f.doc, a, "BAR"));
}

#[test]
fn fit_to_contents() {
    let mut f = fx();
    let c = f.container([0.0, 0.0, 6.0, 6.0], "Container", 0.5, 0.0);
    let a = f.rect([1.0, 1.0, 2.0, 2.0]);
    let b = f.rect([3.0, 2.0, 4.0, 3.0]);
    f.edit(|tx, eng| add_members(tx, eng, c, &[a, b], None));
    assert_box!(inner_box(&f.doc, c), [0.0, 0.0, 6.0, 5.5]);
    assert!(f.edit(|tx, eng| fit(tx, eng, c, true)));
    // the members' box, the margin, the heading on top
    assert_box!(shape_box(&f.doc, c), [0.9, 0.9, 4.1, 3.6]);
    assert!(!f.edit(|tx, eng| fit(tx, eng, c, true)));
}

#[test]
fn lists_keep_order() {
    let mut f = fx();
    // left to right, 0.25 apart
    let l = f.list([0.0, 0.0, 1.0, 1.0], 0, 0.25);
    let a = f.rect([0.1, 0.1, 1.1, 0.6]);
    let b = f.rect([0.1, 0.1, 0.6, 0.9]);
    let owner = f.owner;
    f.edit(|tx, eng| {
        add_members(tx, eng, l, &[a, b], None)?;
        layout_list(tx, eng, l)?;
        settle(tx, eng, owner, &[l], false)
    });
    assert_eq!(members(&f.doc, l), alloc::vec![a, b]);
    // from the list's top left corner, top aligned, the list fitted around them
    let la = shape_box(&f.doc, a);
    let lb = shape_box(&f.doc, b);
    assert!(close(lb[0] - la[2], 0.25));
    assert!(close(la[3], lb[3]));
    let lbx = shape_box(&f.doc, l);
    assert!(close(lbx[0], la[0] - 0.1) && close(lbx[2], lb[2] + 0.1));
    // b dragged in front of a: the order changes
    let d = la[0] - lb[0];
    f.move_by(&[b], [d, 0.0]);
    assert_eq!(members(&f.doc, l), alloc::vec![b, a]);
    let (na, nb) = (shape_box(&f.doc, a), shape_box(&f.doc, b));
    assert!(nb[2] < na[0]);
    // the marks: before, between, after
    let marks = insert_marks(&f.doc, l);
    assert_eq!(marks.len(), 3);
    assert_eq!(marks[1].index, 1);
    assert!(close(marks[1].at[0], (nb[2] + na[0]) * 0.5));
    // move_in_list puts it back
    f.edit(|tx, eng| move_in_list(tx, eng, l, b, 2));
    assert_eq!(members(&f.doc, l), alloc::vec![a, b]);
    // deleting a closes the gap
    f.edit(|tx, eng| {
        forget(tx, eng, &[a])?;
        tx.delete_shapes(&[a])
    });
    assert_eq!(members(&f.doc, l), alloc::vec![b]);
    let nb = shape_box(&f.doc, b);
    assert!(close(nb[0], shape_box(&f.doc, l)[0] + 0.1));
}

#[test]
fn disband_and_repair() {
    let mut f = fx();
    let c = f.container([0.0, 0.0, 4.0, 4.0], "Container", 0.0, 1.0);
    let a = f.rect([1.0, 1.0, 2.0, 2.0]);
    f.edit(|tx, eng| add_members(tx, eng, c, &[a], None));
    // a one-sided edit: the container no longer names a
    let cell = f.doc.intern("Relationships");
    f.edit(|tx, _| tx.force_constant(SheetRef::Shape(c), &CellAddr::single(cell), Value::Num(0.0), None));
    assert_eq!(containers_of(&f.doc, a), alloc::vec![c]);
    let owner = f.owner;
    f.edit(|tx, eng| repair(tx, eng, owner, &[]));
    assert!(containers_of(&f.doc, a).is_empty());
    // disband: the member stays, the container goes
    f.edit(|tx, eng| add_members(tx, eng, c, &[a], None));
    f.edit(|tx, eng| disband(tx, eng, c));
    assert!(f.doc.shape(c).is_none_or(|s| s.is_deleted()));
    assert!(f.doc.shape(a).is_some());
    assert_eq!(f.rel_text(a), "");
}

#[test]
fn callouts_follow_their_target() {
    let mut f = fx();
    let t = f.rect([0.0, 0.0, 2.0, 1.0]);
    let co = f.rect([3.0, 2.0, 4.0, 2.5]);
    f.user(co, "msvStructureType", "Callout");
    // the tail: control row 1, a point of its own
    f.edit(|tx, _| {
        let s = SheetRef::Shape(co);
        let sec = tx.add_section(s, "Control")?;
        let rn = tx.intern("Association");
        let row = tx.add_row(s, sec, Some(crate::RowKey::Named(rn)), None)?;
        for (n, v) in [("X", -0.5), ("Y", -0.5)] {
            let c = tx.intern(n);
            tx.set_length(s, &CellAddr::row(sec, row, c), v, Unit::Inches)?;
        }
        Ok(())
    });
    f.edit(|tx, eng| {
        set_target(tx, eng, co, Some(t))?;
        set_anchor(tx, co, [1.5, 0.5])?;
        aim_callouts(tx, eng, &[co])
    });
    assert_eq!(target_of(&f.doc, co), Some(t));
    assert_eq!(callouts_of(&f.doc, t), alloc::vec![co]);
    let tip = ops::controls(&f.doc, co)[0].at;
    assert!(close(tip[0], 1.5) && close(tip[1], 0.5), "{tip:?}");
    // the target moves: the callout comes along
    f.move_by(&[t], [1.0, 1.0]);
    assert_box!(shape_box(&f.doc, co), [4.0, 3.0, 5.0, 3.5]);
    let tip = ops::controls(&f.doc, co)[0].at;
    assert!(close(tip[0], 2.5) && close(tip[1], 1.5), "{tip:?}");
    // the callout moves on its own: its tail stays on the target
    f.move_by(&[co], [2.0, 0.0]);
    let tip = ops::controls(&f.doc, co)[0].at;
    assert!(close(tip[0], 2.5) && close(tip[1], 1.5), "{tip:?}");
    // deleting the target takes the callout along
    assert_eq!(carried(&f.doc, &[t]), alloc::vec![t, co]);
}

#[test]
fn structure_functions() {
    let mut f = fx();
    let l = f.list([0.0, 0.0, 1.0, 1.0], 2, 0.1);
    let a = f.rect([0.0, 0.0, 1.0, 0.5]);
    let b = f.rect([0.0, 0.0, 1.0, 0.5]);
    let co = f.rect([3.0, 3.0, 4.0, 3.5]);
    f.user(co, "msvStructureType", "Callout");
    f.user(a, "msvShapeCategories", "Step;Bar");
    // formulas that read the structure
    let set = |f: &mut Fx, s: ShapeRef, name: &str, formula: &str| {
        f.edit(|tx, eng| {
            tx.set_user_formula(SheetRef::Shape(s), name, formula)?;
            eng.recalc(tx, &[SheetRef::Shape(s)]).map(|_| ())
        })
    };
    for &s in &[a, b] {
        set(&mut f, s, "Order", "LISTORDER()");
        set(&mut f, s, "Count", "CONTAINERCOUNT()");
        set(&mut f, s, "Cat", "HASCATEGORY(\"bar\")");
        set(&mut f, s, "Callouts", "CALLOUTCOUNT()");
    }
    set(&mut f, l, "Members", "LISTMEMBERCOUNT()");
    set(&mut f, l, "CMembers", "CONTAINERMEMBERCOUNT()");
    let num = |f: &Fx, s: ShapeRef, name: &str| user_num(&f.doc, s, name).unwrap_or(-1.0);
    assert_eq!(num(&f, a, "Order"), -1.0);
    assert_eq!(num(&f, a, "Count"), 0.0);
    assert_eq!(num(&f, a, "Cat"), 1.0);
    assert_eq!(num(&f, b, "Cat"), 0.0);
    f.edit(|tx, eng| {
        add_members(tx, eng, l, &[a, b], None)?;
        layout_list(tx, eng, l).map(|_| ())
    });
    assert_eq!((num(&f, a, "Order"), num(&f, b, "Order")), (1.0, 2.0));
    assert_eq!(num(&f, a, "Count"), 1.0);
    assert_eq!(num(&f, l, "Members"), 2.0);
    assert_eq!(num(&f, l, "CMembers"), 2.0);
    // reordered: the order follows
    f.edit(|tx, eng| move_in_list(tx, eng, l, b, 0).map(|_| ()));
    assert_eq!((num(&f, a, "Order"), num(&f, b, "Order")), (2.0, 1.0));
    // a callout tied to a
    f.edit(|tx, eng| set_target(tx, eng, co, Some(a)));
    assert_eq!(num(&f, a, "Callouts"), 1.0);
    // sheet references: the list and the target through ID()
    set(&mut f, a, "ListId", "ID(LISTSHEETREF())");
    set(&mut f, co, "TargetId", "ID(CALLOUTTARGETREF())");
    set(&mut f, a, "First", "ID(CONTAINERSHEETREF(1))");
    let _ = (f.id(l), f.id(a));
    let _ = (num(&f, a, "ListId"), num(&f, co, "TargetId"), num(&f, a, "First"));
}

// a horizontal flowchart: pool (title 0.5), lane list, phase list, two lanes (heading 0.4 on
// the left), two phases (heading 0.3 on top)
fn flowchart(f: &mut Fx) -> (cff::Cff, [ShapeRef; 2], [ShapeRef; 2]) {
    let pool = f.container([0.0, 0.0, 8.0, 5.0], "Container", 0.5, 0.0);
    f.user(pool, "msvShapeCategories", "CFF Container");
    f.user(pool, "msvSDContainerRequiredCategories", "CFF List");
    let ll = f.list([0.0, 0.0, 8.0, 4.0], 2, 0.0);
    f.user(ll, "msvShapeCategories", "CFF List;DoNotContain");
    f.user(ll, "msvSDListRequiredCategories", "Swimlane");
    f.user(ll, "msvSDContainerExcludedCategories", "DoNotContain");
    let pl = f.list([0.0, 0.0, 8.0, 4.0], 0, 0.0);
    f.user(pl, "msvShapeCategories", "CFF List;DoNotContain");
    f.user(pl, "msvSDListRequiredCategories", "Phase");
    f.user(pl, "msvSDContainerRequiredCategories", "Phase");
    let lane = |f: &mut Fx, y: f64| {
        let l = f.container([0.0, y, 6.0, y + 1.5], "Container", 0.0, 1.0);
        f.user(l, "msvShapeCategories", "Swimlane;DoNotContain");
        f.user(l, "msvSDContainerExcludedCategories", "DoNotContain");
        f.user(l, "JFD_HeadingEdge", "Left");
        f.edit(|tx, _| tx.set_user(SheetRef::Shape(l), "JFD_HeadingSize", Value::Num(0.4), Some(Unit::Inches)));
        l
    };
    let l1 = lane(f, 2.0);
    let l2 = lane(f, 0.0);
    let phase = |f: &mut Fx, x: f64, w: f64| {
        let p = f.container([x, 0.0, x + w, 3.0], "Container", 0.3, 0.0);
        f.user(p, "msvShapeCategories", "Phase;DoNotContain");
        f.user(p, "msvSDContainerExcludedCategories", "DoNotContain");
        p
    };
    let p1 = phase(f, 0.4, 2.0);
    let p2 = phase(f, 2.4, 1.0);
    f.edit(|tx, eng| {
        add_members(tx, eng, pool, &[ll, pl], None)?;
        add_members(tx, eng, ll, &[l1, l2], None)?;
        add_members(tx, eng, pl, &[p1, p2], None)
    });
    let c = cff::of_pool(&f.doc, pool).expect("a flowchart");
    assert_eq!(c.lane_list, Some(ll));
    assert_eq!(c.phase_list, Some(pl));
    assert!(c.horizontal);
    (c, [l1, l2], [p1, p2])
}

#[test]
fn flowchart_layout() {
    let mut f = fx();
    let (c, [l1, l2], [p1, p2]) = flowchart(&mut f);
    f.edit(|tx, eng| cff::layout(tx, eng, &c));
    // lanes from under the title (0.5) and the phase row (0.3), as wide as the longest lane
    assert_box!(shape_box(&f.doc, l1), [0.0, 2.7, 6.0, 4.2]);
    assert_box!(shape_box(&f.doc, l2), [0.0, 1.2, 6.0, 2.7]);
    // the phases after the lane headings, the last one up to the lanes' end
    assert_box!(shape_box(&f.doc, p1), [0.4, 1.2, 2.4, 4.5]);
    assert_box!(shape_box(&f.doc, p2), [2.4, 1.2, 6.0, 4.5]);
    assert_box!(shape_box(&f.doc, c.pool), [0.0, 1.2, 6.0, 5.0]);
    // a shape dropped into lane 2 and phase 1 joins both and the lane list, not the pool
    let a = f.rect([1.0, 1.5, 1.8, 2.1]);
    f.move_by(&[a], [0.0, 0.0]);
    let cs = containers_of(&f.doc, a);
    assert!(cs.contains(&l2) && cs.contains(&p1) && cs.contains(&c.lane_list.unwrap()), "{cs:?}");
    assert!(!cs.contains(&c.pool) && !cs.contains(&c.phase_list.unwrap()));
    // lane 1 gets taller: lane 2 and the shape in it move down, the phases grow
    let owner = f.owner;
    f.edit(|tx, eng| {
        ops::set_frame(tx, eng, l1, &Frame::from_box([0.0, 2.2, 6.0, 4.2]))?;
        settle(tx, eng, owner, &[l1], false)
    });
    assert_box!(shape_box(&f.doc, l2), [0.0, 0.7, 6.0, 2.2]);
    assert_box!(shape_box(&f.doc, a), [1.0, 1.0, 1.8, 1.6]);
    assert_box!(shape_box(&f.doc, p1), [0.4, 0.7, 2.4, 4.5]);
    // the lanes reordered by moving lane 2 above lane 1
    f.move_by(&[l2], [0.0, 2.0]);
    assert_eq!(lanes(&f.doc, &c), alloc::vec![l2, l1]);
    assert_box!(shape_box(&f.doc, l2), [0.0, 2.7, 6.0, 4.2]);
    assert_box!(shape_box(&f.doc, l1), [0.0, 0.7, 6.0, 2.7]);
    assert_box!(shape_box(&f.doc, a), [1.0, 3.0, 1.8, 3.6]);
    // a lane that is too wide makes them all as wide (the last phase takes the rest)
    f.edit(|tx, eng| {
        ops::set_frame(tx, eng, l1, &Frame::from_box([0.0, 0.7, 7.0, 2.7]))?;
        settle(tx, eng, owner, &[l1], false)
    });
    assert_box!(shape_box(&f.doc, l2), [0.0, 2.7, 7.0, 4.2]);
    assert_box!(shape_box(&f.doc, p2), [2.4, 0.7, 7.0, 4.5]);
}

fn lanes(doc: &Document, c: &cff::Cff) -> Vec<ShapeRef> {
    cff::lanes(doc, c)
}
