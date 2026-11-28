// tests of replace (cargo test -p jflib-diagram-core --lib replace)
//
//   Shape ändern keeps place, size, text, shape data, format and glue; the master's
//   ReplaceLock* cells keep its own; ReplaceCopyCells copies anyway; 1-D only for 1-D,
//   containers only for containers, LockReplace; container membership; several at once, one
//   undo step
//
// Every document is built here; nothing comes from a Visio file.

use alloc::string::String;
use alloc::vec::Vec;

use super::*;
use crate::ops;
use crate::shape::{ShapeKind, Text};
use crate::units::Unit;
use crate::{Change, DocKind, Page, PageHeader, Style, StyleRefs};

struct Fx {
    doc: Document,
    owner: Owner,
    eng: Engine,
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7 * (1.0 + b.abs())
}

fn fx() -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    for (id, name, base) in [(0u32, "No Style", None), (1, "Normal", Some(0u32)), (2, "Connector", Some(0u32))] {
        let mut st = crate::Style::default();
        st.header.id = id;
        st.header.name_u = Some(doc.intern(name));
        st.header.name = Some(doc.intern(name));
        if let Some(b) = base {
            st.header.based_on = StyleRefs::all(b);
        }
        doc.insert_style(st).unwrap();
    }
    let _ = Style::default();
    let mut ph = PageHeader { id: 0, ..PageHeader::default() };
    ph.name_u = Some(doc.intern("Page-1"));
    let page = doc.insert_page(Page::new(ph)).unwrap();
    Fx { doc, owner: Owner::Page(page), eng: Engine::new() }
}

impl Fx {
    fn edit<T>(&mut self, f: impl FnOnce(&mut Transaction, &mut Engine) -> Result<T, EditError>) -> T {
        let mut tx = self.doc.transaction("test");
        let r = f(&mut tx, &mut self.eng).expect("edit");
        let c: Option<Change> = tx.commit();
        if let Some(mut c) = c {
            assert!(c.undo(&mut self.doc));
            assert!(c.redo(&mut self.doc));
            self.eng.refresh(&self.doc, &c.sheets());
        }
        r
    }

    fn rect(&mut self, b: [f64; 4]) -> ShapeRef {
        let owner = self.owner;
        self.edit(|tx, eng| ops::new_shape(tx, eng, owner, ops::Basic::Rectangle, [b[0], b[1]], [b[2], b[3]]))
    }

    // a master: an ellipse 2 x 1 with four connection points and shape data "Kosten" (and
    // whatever `more` writes into its shape)
    fn master(&mut self, name: &str, more: impl FnOnce(&mut Transaction, ShapeRef) -> Result<(), EditError>) -> MasterRef {
        let mut tx = self.doc.transaction("master");
        let m = tx.add_master(name, name).unwrap();
        let mut eng = Engine::new();
        let s = ops::new_shape(&mut tx, &mut eng, Owner::Master(m), ops::Basic::Ellipse, [0.0, 0.0], [2.0, 1.0]).unwrap();
        let sh = SheetRef::Shape(s);
        let sk = tx.add_section(sh, "Connection").unwrap();
        for (i, (x, y)) in [(0.0, 0.5), (2.0, 0.5), (1.0, 0.0), (1.0, 1.0)].into_iter().enumerate() {
            let rk = tx.add_row(sh, sk, Some(RowKey::Index(i as u32)), None).unwrap();
            let (cx, cy) = (tx.intern("X"), tx.intern("Y"));
            tx.set_constant(sh, &CellAddr::row(sk, rk, cx), Value::Num(x), Some(Unit::Inches)).unwrap();
            tx.set_constant(sh, &CellAddr::row(sk, rk, cy), Value::Num(y), Some(Unit::Inches)).unwrap();
        }
        let z = tx.intern("0");
        tx.set_shape_data(sh, "Kosten", "Kosten", Value::Text(z)).unwrap();
        more(&mut tx, s).unwrap();
        eng.recalc(&mut tx, &[sh]).unwrap();
        tx.commit();
        m
    }

    fn data(&self, s: ShapeRef, row: &str) -> Option<String> {
        self.doc.shape_data(SheetRef::Shape(s)).into_iter().find(|r| r.name() == Some(row)).and_then(|r| r.text("Value").map(String::from))
    }

    fn user(&self, s: ShapeRef, row: &str) -> Option<f64> {
        self.doc.user_cells(SheetRef::Shape(s)).into_iter().find(|r| r.name() == Some(row)).and_then(|r| r.num("Value"))
    }

    fn cell(&self, s: ShapeRef, cell: &str) -> Option<f64> {
        self.doc.sym(cell).and_then(|c| self.doc.resolve(SheetRef::Shape(s), &CellAddr::single(c)).num())
    }

    fn text(&self, s: ShapeRef) -> String {
        self.doc.shape(s).and_then(|x| x.text()).map(|t| t.chars()).unwrap_or_default().trim_end().into()
    }
}

// the old shape: text, its own shape data, a red fill, User.Keep
fn decorate(f: &mut Fx, s: ShapeRef) {
    f.edit(|tx, _| {
        let sh = SheetRef::Shape(s);
        tx.set_text(s, Some(Text::plain("Hallo")))?;
        let v = tx.intern("Anna");
        tx.set_shape_data(sh, "Besitzer", "Besitzer", Value::Text(v))?;
        let v = tx.intern("12");
        tx.set_shape_data(sh, "Kosten", "Kosten", Value::Text(v))?;
        let a = tx.single("FillForegnd");
        tx.set_formula(sh, &a, "RGB(255,0,0)")?;
        tx.set_user(sh, "Keep", Value::Num(7.0), None)
    });
}

#[test]
fn keeps_place_text_data_format_and_glue() {
    let mut f = fx();
    let m = f.master("Kreis", |_, _| Ok(()));
    let old = f.rect([1.0, 1.0, 4.0, 3.0]);
    let other = f.rect([6.0, 1.0, 7.0, 2.0]);
    decorate(&mut f, old);
    let owner = f.owner;
    let conn = f.edit(|tx, eng| glue::new_connector(tx, eng, owner, ([4.0, 2.0], Target::Shape(old)), ([6.0, 1.5], Target::Shape(other))));
    let before = ops::frame(&f.doc, old);
    let new = f.edit(|tx, eng| replace(tx, eng, &[old], m, 0));
    assert_eq!(new.len(), 1);
    let n = new[0];
    assert!(f.doc.shape(old).is_none_or(|s| s.is_deleted()));
    assert_eq!(f.doc.master_of(n), Some(m));
    // place and size: the old shape's own
    let fr = ops::frame(&f.doc, n);
    assert!(close(fr.w, before.w) && close(fr.h, before.h), "{} x {}", fr.w, fr.h);
    assert!(close(fr.centre()[0], before.centre()[0]) && close(fr.centre()[1], before.centre()[1]));
    // text, shape data (its own row and the master's row with the old value), format
    assert_eq!(f.text(n), "Hallo");
    assert_eq!(f.data(n, "Besitzer").as_deref(), Some("Anna"));
    assert_eq!(f.data(n, "Kosten").as_deref(), Some("12"));
    let red = f.doc.sym("FillForegnd").map(|c| f.doc.resolve(SheetRef::Shape(n), &CellAddr::single(c)).formula());
    assert!(matches!(red, Some(Formula::Expr(x)) if f.doc.str(x) == "RGB(255,0,0)"));
    // User cells never move
    assert_eq!(f.user(n, "Keep"), None);
    // the connector glued to the new shape now
    assert_eq!(glue::glued(&f.doc, conn)[0], Target::Shape(n));
    // the z-order: where the old one was
    let tops: Vec<ShapeRef> = f.doc.top_shapes(owner).collect();
    assert_eq!(tops[0], n);
}

#[test]
fn the_masters_locks_and_copy_cells() {
    let mut f = fx();
    let m = f.master("Fest", |tx, s| {
        let sh = SheetRef::Shape(s);
        for c in ["ReplaceLockShapeData", "ReplaceLockText", "ReplaceLockFormat"] {
            let a = tx.single(c);
            tx.set_constant(sh, &a, Value::Num(1.0), Some(Unit::Bool))?;
        }
        tx.set_user(sh, "Keep", Value::Num(0.0), None)?;
        let a = tx.single("ReplaceCopyCells");
        tx.set_formula(sh, &a, "DEPENDSON(User.Keep)")?;
        tx.set_text(s, Some(Text::plain("Master")))
    });
    assert!(Behavior::of(&f.doc, SheetRef::Shape(ShapeRef { owner: Owner::Master(m), slot: f.doc.master(m).unwrap().master_shape().unwrap() })).lock_text);
    let old = f.rect([1.0, 1.0, 3.0, 2.0]);
    decorate(&mut f, old);
    let n = f.edit(|tx, eng| replace(tx, eng, &[old], m, 0))[0];
    assert_eq!(f.text(n), "Master");
    assert_eq!(f.data(n, "Besitzer"), None);
    assert_eq!(f.data(n, "Kosten").as_deref(), Some("0"));
    let red = f.doc.sym("FillForegnd").map(|c| f.doc.resolve(SheetRef::Shape(n), &CellAddr::single(c)).formula());
    assert!(!matches!(red, Some(Formula::Expr(x)) if f.doc.str(x) == "RGB(255,0,0)"));
    // ReplaceCopyCells: User.Keep comes along
    assert_eq!(f.user(n, "Keep"), Some(7.0));
    // the place moves always
    assert!(close(f.cell(n, "Width").unwrap(), 2.0));
}

#[test]
fn what_cannot_be_replaced() {
    let mut f = fx();
    let m = f.master("Kreis", |_, _| Ok(()));
    let owner = f.owner;
    let line = f.edit(|tx, eng| glue::new_line(tx, eng, owner, ([0.0, 0.0], Target::None), ([2.0, 0.0], Target::None)));
    assert!(!can_replace(&f.doc, line, m), "1-D for 2-D");
    let s = f.rect([0.0, 1.0, 1.0, 2.0]);
    assert!(can_replace(&f.doc, s, m));
    f.edit(|tx, _| {
        let a = tx.single("LockReplace");
        tx.set_constant(SheetRef::Shape(s), &a, Value::Num(1.0), Some(Unit::Bool))
    });
    assert!(!can_replace(&f.doc, s, m), "LockReplace");
    let c = f.rect([3.0, 0.0, 6.0, 3.0]);
    f.edit(|tx, _| {
        let k = tx.intern("Container");
        tx.set_user(SheetRef::Shape(c), "msvStructureType", Value::Text(k), Some(Unit::Str))
    });
    assert!(!can_replace(&f.doc, c, m), "a container only by a container");
    let guide = f.edit(|tx, _| tx.add_shape(owner, None, ShapeKind::Guide));
    assert!(!can_replace(&f.doc, guide, m));
    // the refused ones are left alone
    let made = f.edit(|tx, eng| replace(tx, eng, &[line, s], m, 0));
    assert!(made.is_empty());
}

#[test]
fn several_at_once_keep_their_container() {
    let mut f = fx();
    let m = f.master("Kreis", |_, _| Ok(()));
    let c = f.rect([0.0, 0.0, 8.0, 4.0]);
    f.edit(|tx, _| {
        let k = tx.intern("Container");
        tx.set_user(SheetRef::Shape(c), "msvStructureType", Value::Text(k), Some(Unit::Str))?;
        tx.set_user(SheetRef::Shape(c), "msvSDContainerResize", Value::Num(0.0), None)
    });
    let a = f.rect([1.0, 1.0, 2.0, 2.0]);
    let b = f.rect([4.0, 1.0, 5.0, 2.0]);
    f.edit(|tx, eng| structure::add_members(tx, eng, c, &[a, b], None));
    let shapes_before = f.doc.top_shapes(f.owner).count();
    let made = f.edit(|tx, eng| replace(tx, eng, &[a, b], m, 0));
    assert_eq!(made.len(), 2);
    assert_eq!(f.doc.top_shapes(f.owner).count(), shapes_before);
    let ms = structure::members(&f.doc, c);
    assert!(ms.contains(&made[0]) && ms.contains(&made[1]), "{ms:?}");
    assert_eq!(structure::containers_of(&f.doc, made[1]), alloc::vec![c]);
    let _ = Vec::<u8>::new();
}
