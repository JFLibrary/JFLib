// tests of groups and instance::detach (cargo test -p jflib-diagram-core --lib groups)
//
//   the Group Properties read with Visio's defaults and written, Zu Gruppe hinzufügen / Aus Gruppe
//   entfernen keep every shape where it is on the page (an upright and a turned group), the box
//   follows, dropping into a group needs IsDropTarget and IsDropSource, typing into a group
//   (IsTextEditTarget), the ungroup warning, an instance of a group master made plain and
//   ungrouped, the glue of a guide (SP-17); every edit is undone and redone
//
// Every document is built here; nothing comes from a Visio file.

use alloc::string::String;
use alloc::vec::Vec;

use super::*;
use crate::cell::Cell;
use crate::page::Connect;
use crate::shape::Text;
use crate::sheet::Extras;
use crate::units::Unit;
use crate::{Change, DocKind, Page, PageHeader, PageRef, Style, StyleRefs};

struct Fx {
    doc: Document,
    page: PageRef,
    owner: Owner,
    eng: Engine,
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7 * (1.0 + b.abs())
}

fn assert_box(a: [f64; 4], b: [f64; 4]) {
    assert!((0..4).all(|k| close(a[k], b[k])), "{a:?} is not {b:?}");
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
    Fx { doc, page, owner: Owner::Page(page), eng: Engine::new() }
}

impl Fx {
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

    fn snapshot(&self) -> Vec<(u32, Option<u32>, [i64; 4])> {
        self.doc
            .all_shapes(self.owner)
            .map(|s| {
                let sh = self.doc.shape(s).unwrap();
                let b = ops::bounds(&self.doc, &[s]).unwrap_or([0.0; 4]);
                let q = |x: f64| (x * 1e6).round() as i64;
                (sh.id(), sh.parent().map(|p| self.doc.shape(p).unwrap().id()), [q(b[0]), q(b[1]), q(b[2]), q(b[3])])
            })
            .collect()
    }

    // a rectangle drawn with the tool (x0, y0, x1, y1)
    fn rect(&mut self, b: [f64; 4]) -> ShapeRef {
        let owner = self.owner;
        self.edit(|tx, eng| ops::new_shape(tx, eng, owner, ops::Basic::Rectangle, [b[0], b[1]], [b[2], b[3]]))
    }

    fn group(&mut self, shapes: &[ShapeRef]) -> ShapeRef {
        let owner = self.owner;
        let v = shapes.to_vec();
        self.edit(|tx, eng| ops::group(tx, eng, owner, &v))
    }

    fn bounds(&self, s: ShapeRef) -> [f64; 4] {
        ops::bounds(&self.doc, &[s]).unwrap()
    }

    fn formula(&self, s: ShapeRef, cell: &str) -> String {
        let a = CellAddr::single(self.doc.sym(cell).unwrap());
        match self.doc.resolve(SheetRef::Shape(s), &a).formula() {
            Formula::Expr(f) => String::from(self.doc.str(f)),
            _ => String::new(),
        }
    }

    fn id(&self, s: ShapeRef) -> u32 {
        self.doc.shape(s).unwrap().id()
    }
}

#[test]
fn props_with_visio_defaults() {
    let mut f = fx();
    let a = f.rect([0.0, 0.0, 1.0, 1.0]);
    let b = f.rect([2.0, 0.0, 3.0, 1.0]);
    let g = f.group(&[a, b]);
    assert_eq!(props(&f.doc, g), Props::default());
    assert_eq!(props(&f.doc, g).select_mode, SelectMode::GroupFirst);
    let p = Props { select_mode: SelectMode::MembersFirst, display_mode: 1, drop_target: true, snap_target: false, text_edit_target: false, dont_move_children: true };
    f.edit(|tx, eng| set_props(tx, eng, g, &p));
    assert_eq!(props(&f.doc, g), p);
    assert_eq!(SelectMode::of(0), SelectMode::GroupOnly);
    // IsDropSource and the double click
    assert!(!is_drop_source(&f.doc, a));
    f.edit(|tx, eng| set_drop_source(tx, eng, a, true));
    assert!(is_drop_source(&f.doc, a));
    assert!(!double_click_opens(&f.doc, g));
    f.edit(|tx, eng| set_double_click_opens(tx, eng, g, true));
    assert!(double_click_opens(&f.doc, g));
    assert_eq!(f.formula(g, "EventDblClick"), "OPENGROUPWIN()");
    f.edit(|tx, eng| set_double_click_opens(tx, eng, g, false));
    assert!(!double_click_opens(&f.doc, g));
}

#[test]
fn add_and_remove_keep_the_page_place() {
    let mut f = fx();
    let a = f.rect([0.0, 0.0, 1.0, 1.0]);
    let b = f.rect([2.0, 0.0, 3.0, 1.0]);
    let c = f.rect([4.0, 2.0, 5.0, 3.5]);
    let g = f.group(&[a, b]);
    assert_box(f.bounds(g), [0.0, 0.0, 3.0, 1.0]);
    let added = f.edit(|tx, eng| add_to_group(tx, eng, g, &[c]));
    assert_eq!(added, alloc::vec![c]);
    assert_eq!(f.doc.shape(c).unwrap().parent(), Some(g));
    assert_box(f.bounds(c), [4.0, 2.0, 5.0, 3.5]);
    assert_box(f.bounds(a), [0.0, 0.0, 1.0, 1.0]);
    // the box around all three, the new member scales with the group
    assert_box(f.bounds(g), [0.0, 0.0, 5.0, 3.5]);
    let gid = f.id(g);
    assert!(f.formula(c, "PinX").starts_with(&alloc::format!("Sheet.{gid}!Width*")), "{}", f.formula(c, "PinX"));
    // on top of the members
    assert_eq!(f.doc.shape(g).unwrap().children().last(), Some(c));
    // out again: where it was, above the group, the box back
    let moved = f.edit(|tx, eng| remove_from_group(tx, eng, &[c]));
    assert_eq!(moved, alloc::vec![c]);
    assert_eq!(f.doc.shape(c).unwrap().parent(), None);
    assert_box(f.bounds(c), [4.0, 2.0, 5.0, 3.5]);
    assert!(!f.formula(c, "PinX").contains("Sheet."), "no formula names the group: {}", f.formula(c, "PinX"));
    let tops: Vec<ShapeRef> = f.doc.top_shapes(f.owner).collect();
    assert_eq!(tops, alloc::vec![g, c]);
    assert_box(f.bounds(g), [0.0, 0.0, 3.0, 1.0]);
    // the last members out: the group goes
    f.edit(|tx, eng| remove_from_group(tx, eng, &[a, b]));
    assert!(f.doc.shape(g).is_none_or(|s| s.is_deleted()));
    assert_box(f.bounds(a), [0.0, 0.0, 1.0, 1.0]);
    assert_box(f.bounds(b), [2.0, 0.0, 3.0, 1.0]);
}

#[test]
fn a_turned_group_takes_a_shape_where_it_is() {
    let mut f = fx();
    let a = f.rect([0.0, 0.0, 2.0, 1.0]);
    let b = f.rect([3.0, 0.0, 4.0, 1.0]);
    let c = f.rect([1.0, 3.0, 2.0, 3.5]);
    let g = f.group(&[a, b]);
    f.edit(|tx, eng| ops::rotate_shapes(tx, eng, &[g], [2.0, 0.5], 0.5));
    f.edit(|tx, eng| ops::flip_shapes(tx, eng, &[g], true));
    let before = ops::frame(&f.doc, c).corners();
    f.edit(|tx, eng| add_to_group(tx, eng, g, &[c]));
    let after = ops::frame(&f.doc, c).corners();
    for k in 0..4 {
        assert!(close(before[k][0], after[k][0]) && close(before[k][1], after[k][1]), "corner {k}: {:?} -> {:?}", before[k], after[k]);
    }
    f.edit(|tx, eng| remove_from_group(tx, eng, &[c]));
    let back = ops::frame(&f.doc, c).corners();
    for k in 0..4 {
        assert!(close(before[k][0], back[k][0]) && close(before[k][1], back[k][1]), "corner {k}: {:?} -> {:?}", before[k], back[k]);
    }
}

#[test]
fn dropping_into_a_group_needs_both_cells() {
    let mut f = fx();
    let a = f.rect([0.0, 0.0, 2.0, 2.0]);
    let b = f.rect([3.0, 0.0, 5.0, 2.0]);
    let g = f.group(&[a, b]);
    let s = f.rect([0.5, 0.5, 1.0, 1.0]);
    let owner = f.owner;
    // the group takes no drops yet
    assert_eq!(drop_target_at(&f.doc, owner, [0.75, 0.75], &[s]), None);
    f.edit(|tx, eng| set_props(tx, eng, g, &Props { drop_target: true, ..Props::default() }));
    assert_eq!(drop_target_at(&f.doc, owner, [0.75, 0.75], &[s]), Some(g));
    // the shape is no drop source: it stays on the page
    let took = f.edit(|tx, eng| dropped(tx, eng, owner, &[s]));
    assert!(took.is_empty());
    assert_eq!(f.doc.shape(s).unwrap().parent(), None);
    f.edit(|tx, eng| set_drop_source(tx, eng, s, true));
    let took = f.edit(|tx, eng| dropped(tx, eng, owner, &[s]));
    assert_eq!(took, alloc::vec![g]);
    assert_eq!(f.doc.shape(s).unwrap().parent(), Some(g));
    assert_box(f.bounds(s), [0.5, 0.5, 1.0, 1.0]);
    // beside the group: nothing
    let t = f.rect([7.0, 0.0, 8.0, 1.0]);
    f.edit(|tx, eng| set_drop_source(tx, eng, t, true));
    assert!(f.edit(|tx, eng| dropped(tx, eng, owner, &[t])).is_empty());
}

#[test]
fn typing_into_a_group() {
    let mut f = fx();
    let a = f.rect([0.0, 0.0, 1.0, 1.0]);
    let b = f.rect([2.0, 0.0, 3.0, 1.0]);
    let g = f.group(&[a, b]);
    assert_eq!(text_target(&f.doc, g), g);
    f.edit(|tx, eng| set_props(tx, eng, g, &Props { text_edit_target: false, ..Props::default() }));
    assert_eq!(text_target(&f.doc, g), b, "the member on top");
    assert_eq!(text_target(&f.doc, a), a);
}

#[test]
fn the_ungroup_warning() {
    let mut f = fx();
    let a = f.rect([0.0, 0.0, 1.0, 1.0]);
    let b = f.rect([2.0, 0.0, 3.0, 1.0]);
    let g = f.group(&[a, b]);
    // pins and sizes in the group's terms: no warning
    assert_eq!(ungroup_warning(&f.doc, g), None);
    let gid = f.id(g);
    f.edit(|tx, eng| {
        tx.set_user(SheetRef::Shape(g), "Cost", Value::Num(5.0), None)?;
        let a2 = tx.named("User", "Cost", "Value");
        tx.set_user(SheetRef::Shape(a), "Cost", Value::Num(0.0), None)?;
        tx.set_formula(SheetRef::Shape(a), &a2, &alloc::format!("Sheet.{gid}!User.Cost*2"))?;
        eng.recalc(tx, &[SheetRef::Shape(a)])
    });
    assert_eq!(ungroup_warning(&f.doc, g), Some(Warning::Formulas(1)));
    assert_eq!(ungroup_warning(&f.doc, a), None);
}

// a master "Pair": a group with two boxes, the second one's width half the group's
fn pair_master(f: &mut Fx) -> crate::MasterRef {
    let mut tx = f.doc.transaction("master");
    let m = tx.add_master("Pair", "Pair").unwrap();
    let owner = Owner::Master(m);
    let mut eng = Engine::new();
    let a = ops::new_shape(&mut tx, &mut eng, owner, ops::Basic::Rectangle, [0.0, 0.0], [1.0, 1.0]).unwrap();
    let b = ops::new_shape(&mut tx, &mut eng, owner, ops::Basic::Rectangle, [2.0, 0.0], [3.0, 1.0]).unwrap();
    let g = ops::group(&mut tx, &mut eng, owner, &[a, b]).unwrap();
    let gid = tx.doc().shape(g).unwrap().id();
    let w = tx.single("Width");
    tx.set_formula(SheetRef::Shape(b), &w, &alloc::format!("Sheet.{gid}!Width*0.25")).unwrap();
    let t = tx.single("TheText");
    let _ = t;
    tx.set_text(a, Some(Text::plain("links"))).unwrap();
    eng.recalc(&mut tx, &[SheetRef::Shape(b)]).unwrap();
    tx.commit();
    m
}

#[test]
fn an_instance_made_plain_and_ungrouped() {
    let mut f = fx();
    let m = pair_master(&mut f);
    let page = f.page;
    let inst = f.edit(|tx, eng| {
        let s = crate::instance::place(tx, page, m, 5.0, 5.0, 0)?;
        let mut sheets = alloc::vec![SheetRef::Shape(s)];
        sheets.extend(tx.doc().shape(s).unwrap().children().map(SheetRef::Shape));
        eng.recalc(tx, &sheets)?;
        Ok(s)
    });
    assert_eq!(ungroup_warning(&f.doc, inst), Some(Warning::Master));
    let kids: Vec<ShapeRef> = f.doc.shape(inst).unwrap().children().collect();
    assert_eq!(kids.len(), 2);
    let before: Vec<[f64; 4]> = kids.iter().map(|&k| f.bounds(k)).collect();
    // refused while it is an instance
    {
        let mut tx = f.doc.transaction("refused");
        assert!(ops::ungroup(&mut tx, &mut f.eng, inst).is_err());
    }
    f.edit(|tx, _| crate::instance::detach(tx, inst));
    let iid = f.id(inst);
    assert!(f.doc.shape(inst).unwrap().header().master.is_none());
    for &k in &kids {
        assert!(f.doc.shape(k).unwrap().header().master_shape.is_none());
    }
    // the master's formula names the instance now, the text is the subshape's own
    assert_eq!(f.formula(kids[1], "Width"), alloc::format!("Sheet.{iid}!Width*0.25"));
    assert_eq!(f.doc.shape(kids[0]).unwrap().text().map(|t| t.chars()).unwrap_or_default().trim_end(), "links");
    let after: Vec<[f64; 4]> = kids.iter().map(|&k| f.bounds(k)).collect();
    for k in 0..2 {
        assert_box(after[k], before[k]);
    }
    let out = f.edit(|tx, eng| ops::ungroup(tx, eng, inst));
    assert_eq!(out.len(), 2);
    for k in 0..2 {
        assert_box(f.bounds(out[k]), before[k]);
    }
}

#[test]
fn a_guide_and_its_shape_only_together() {
    let mut f = fx();
    let a = f.rect([0.0, 0.0, 1.0, 1.0]);
    let b = f.rect([2.0, 0.0, 3.0, 1.0]);
    let owner = f.owner;
    let guide = f.edit(|tx, _| {
        let s = tx.add_shape(owner, None, ShapeKind::Guide)?;
        let a2 = tx.single("PinY");
        tx.set_constant(SheetRef::Shape(s), &a2, Value::Num(0.5), None)?;
        Ok(s)
    });
    let (ia, ig) = (f.id(a), f.id(guide));
    // a glued to the guide (PinY follows it)
    f.edit(|tx, _| {
        let py = tx.intern("PinY");
        let c = Connect { from_sheet: ia, from_cell: Some(py), from_part: Some(4), to_sheet: ig, to_cell: Some(py), to_part: Some(0), extras: Extras::new() };
        tx.add_connect(owner, c)?;
        let a2 = tx.single("PinY");
        tx.set_formula(SheetRef::Shape(a), &a2, &alloc::format!("Sheet.{ig}!PinY"))
    });
    let glued = |f: &Fx| ops::connects(&f.doc, owner).iter().any(|c| c.from_sheet == ia && c.to_sheet == ig);
    assert!(glued(&f));
    // grouped together: the glue stays
    let g = f.group(&[a, guide]);
    assert!(glued(&f));
    f.edit(|tx, eng| ops::ungroup(tx, eng, g));
    // a without its guide: the glue breaks, the place stays
    let before = f.bounds(a);
    f.group(&[a, b]);
    assert!(!glued(&f));
    assert_box(f.bounds(a), before);
    let _ = Cell::constant(f.doc.sym("PinY").unwrap(), Value::Num(0.0), Some(Unit::Inches));
}

// (IsSnapTarget) a connector glued to a connection point of a member of a turned group ends on it
#[test]
fn glue_to_a_member_of_a_group() {
    let mut f = fx();
    let a = f.rect([0.0, 0.0, 1.0, 1.0]);
    let b = f.rect([2.0, 0.0, 3.0, 1.0]);
    f.edit(|tx, _| {
        let sh = SheetRef::Shape(b);
        let sk = tx.add_section(sh, "Connection")?;
        let rk = tx.add_row(sh, sk, Some(crate::sheet::RowKey::Index(0)), None)?;
        let (cx, cy) = (tx.intern("X"), tx.intern("Y"));
        tx.set_formula(sh, &CellAddr::row(sk, rk, cx), "Width*1")?;
        tx.set_formula(sh, &CellAddr::row(sk, rk, cy), "Height*0.5")
    });
    f.edit(|tx, eng| eng.recalc(tx, &[SheetRef::Shape(b)]));
    let g = f.group(&[a, b]);
    f.edit(|tx, eng| ops::rotate_shapes(tx, eng, &[g], [1.5, 0.5], 0.7));
    let p = crate::glue::connection_points(&f.doc, b)[0].at;
    let owner = f.owner;
    let c = f.edit(|tx, eng| crate::glue::new_connector(tx, eng, owner, ([6.0, 6.0], crate::glue::Target::None), (p, crate::glue::Target::Point { shape: b, row: 0 })));
    let (_, e) = crate::glue::ends(&f.doc, c).unwrap();
    assert!(close(e[0], p[0]) && close(e[1], p[1]), "{e:?} is not {p:?}");
    // the group moves: the end follows
    f.edit(|tx, eng| {
        ops::move_shapes(tx, eng, &[g], [1.0, 2.0])?;
        let deps = crate::glue::dependents(tx.doc(), owner, &[g]);
        crate::glue::update(tx, eng, &deps)
    });
    let p = crate::glue::connection_points(&f.doc, b)[0].at;
    let (_, e) = crate::glue::ends(&f.doc, c).unwrap();
    assert!(close(e[0], p[0]) && close(e[1], p[1]), "{e:?} is not {p:?}");
}

// an instance of a master dropped into a group: the box takes it in
#[test]
fn an_instance_joins_a_group() {
    let mut f = fx();
    let m = pair_master(&mut f);
    let a = f.rect([0.0, 0.0, 1.0, 1.0]);
    let b = f.rect([2.0, 0.0, 3.0, 1.0]);
    let g = f.group(&[a, b]);
    let page = f.page;
    let inst = f.edit(|tx, eng| {
        let s = crate::instance::place(tx, page, m, 1.5, -3.0, 0)?;
        let mut sheets = alloc::vec![SheetRef::Shape(s)];
        sheets.extend(tx.doc().shape(s).unwrap().children().map(SheetRef::Shape));
        eng.recalc(tx, &sheets)?;
        Ok(s)
    });
    let before = f.bounds(inst);
    f.edit(|tx, eng| add_to_group(tx, eng, g, &[inst]));
    assert_box(f.bounds(inst), before);
    let gb = f.bounds(g);
    assert!(close(gb[1], before[1]), "{gb:?} takes in {before:?}");
}
