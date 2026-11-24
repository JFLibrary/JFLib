// tests of datagraphic.rs: a graphic master written and read back, applied, positions and
// stacking, icon rules, colour by value with its removal, editing and re-applying, undo, the
// legend.  The callout masters are built here (the app's own come from its stencil).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::legend;
use super::*;
use crate::cell::Formula;
use crate::document::DocKind;
use crate::ops::{self, Basic};
use crate::shape::Text;

struct Fx {
    doc: Document,
    eng: Engine,
    page: PageRef,
    shapes: Vec<ShapeRef>,
}

fn s(x: &str) -> String {
    String::from(x)
}

fn num(doc: &Document, sheet: SheetRef, sec: &str, row: &str, c: &str) -> Option<f64> {
    let a = CellAddr::row(SecKey::one(doc.sym(sec)?), RowKey::Named(doc.sym(row)?), doc.sym(c)?);
    doc.resolve(sheet, &a).num()
}

fn text(doc: &Document, sheet: SheetRef, sec: &str, row: &str, c: &str) -> Option<String> {
    let a = CellAddr::row(SecKey::one(doc.sym(sec)?), RowKey::Named(doc.sym(row)?), doc.sym(c)?);
    match doc.resolve(sheet, &a).value() {
        Value::Text(t) => Some(s(doc.str(t))),
        Value::Num(x) => Some(num_text(x)),
        Value::Absent => None,
    }
}

fn formula(doc: &Document, sheet: SheetRef, addr: &CellAddr) -> Option<String> {
    match doc.resolve(sheet, addr).formula() {
        Formula::Expr(f) => Some(s(doc.str(f))),
        _ => None,
    }
}

fn single_num(doc: &Document, sheet: SheetRef, c: &str) -> f64 {
    doc.resolve(sheet, &CellAddr::single(doc.sym(c).unwrap())).num().unwrap_or(f64::NAN)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// a callout master: a group-less shape w x h with the callout cells; `geo` geometry sections
// as (NoShow formula, width formula)
fn callout_master(tx: &mut Transaction, eng: &mut Engine, name: &str, kind: Kind, size: (f64, f64), geo: &[(&str, &str)], extra: &[(&str, &str)]) -> MasterRef {
    let m = tx.add_master(name, name).unwrap();
    let k = tx.add_shape(Owner::Master(m), None, ShapeKind::Shape).unwrap();
    let ks = SheetRef::Shape(k);
    for (c, v) in [("PinX", size.0 * 0.5), ("PinY", size.1 * 0.5), ("Width", size.0), ("Height", size.1)] {
        let a = tx.single(c);
        tx.set_constant(ks, &a, Value::Num(v), None).unwrap();
    }
    for (c, f) in [("LocPinX", "Width*0.5"), ("LocPinY", "Height*0.5")] {
        let a = tx.single(c);
        tx.set_formula(ks, &a, f).unwrap();
    }
    set_user_str(tx, ks, "msvCalloutType", kind.callout_type()).unwrap();
    let fa = named_cell(tx, ks, names::PROPERTY, "msvCalloutField", "Value").unwrap();
    tx.set_constant(ks, &fa, Value::Num(50.0), None).unwrap();
    for (row, v) in [("msvCalloutPropMin", 0.0), ("msvCalloutPropMax", 100.0)] {
        let a = named_cell(tx, ks, names::PROPERTY, row, "Value").unwrap();
        tx.set_constant(ks, &a, Value::Num(v), None).unwrap();
    }
    for (n, f) in extra {
        tx.set_user_formula(ks, n, f).unwrap();
    }
    for (noshow, width) in geo {
        let sk = tx.add_section(ks, "Geometry").unwrap();
        tx.set_formula(ks, &CellAddr::section(sk, names::NO_SHOW), noshow).unwrap();
        let r1 = tx.add_row(ks, sk, None, Some("MoveTo")).unwrap();
        tx.set_constant(ks, &CellAddr::row(sk, r1, names::X), Value::Num(0.0), None).unwrap();
        tx.set_constant(ks, &CellAddr::row(sk, r1, names::Y), Value::Num(0.0), None).unwrap();
        let r2 = tx.add_row(ks, sk, None, Some("LineTo")).unwrap();
        tx.set_formula(ks, &CellAddr::row(sk, r2, names::X), width).unwrap();
        tx.set_constant(ks, &CellAddr::row(sk, r2, names::Y), Value::Num(0.0), None).unwrap();
    }
    let mut h = tx.doc().master(m).unwrap().header.clone();
    h.attrs.push((tx.intern("Hidden"), tx.intern("1")));
    tx.set_master_header(m, h).unwrap();
    eng.recalc(tx, &[ks]).unwrap();
    m
}

// a page with three boxes carrying shape data Name, Load (Auslastung), Status, Site
fn fx() -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    let mut eng = Engine::new();
    let mut tx = doc.transaction("setup");
    let page = tx.add_page("Zeichenblatt-1", "Page-1", false, None).unwrap();
    let ps = SheetRef::Page(page);
    for (c, v) in [("PageWidth", 11.0), ("PageHeight", 8.5), ("PageScale", 1.0), ("DrawingScale", 1.0)] {
        let a = tx.single(c);
        tx.set_constant(ps, &a, Value::Num(v), None).unwrap();
    }
    let rows = [("Server A", 35.0, "OK", "Raum 1"), ("Server B", 72.0, "Warnung", "Raum 2"), ("Router", 91.0, "Fehler", "Raum 1")];
    let mut shapes = Vec::new();
    for (i, (name, load, status, site)) in rows.iter().enumerate() {
        let x = 1.0 + 3.0 * i as f64;
        let sh = ops::new_shape(&mut tx, &mut eng, Owner::Page(page), Basic::Rectangle, [x, 4.0], [x + 2.0, 5.0]).unwrap();
        tx.set_text(sh, Some(Text::plain(name))).unwrap();
        let sheet = SheetRef::Shape(sh);
        let (n, st, si) = (tx.intern(name), tx.intern(status), tx.intern(site));
        tx.set_shape_data(sheet, "Name", "Name", Value::Text(n)).unwrap();
        tx.set_shape_data(sheet, "Load", "Auslastung", Value::Num(*load)).unwrap();
        tx.set_shape_data(sheet, "Status", "Status", Value::Text(st)).unwrap();
        tx.set_shape_data(sheet, "Site", "Standort", Value::Text(si)).unwrap();
        let fa = tx.single("FillForegnd");
        tx.set_formula(sheet, &fa, "RGB(1,2,3)").unwrap();
        eng.recalc(&mut tx, &[sheet]).unwrap();
        shapes.push(sh);
    }
    callout_master(&mut tx, &mut eng, "Text callout", Kind::Text, (1.0, 0.25), &[], &[]);
    callout_master(&mut tx, &mut eng, "Progress bar", Kind::DataBar, (1.0, 0.2), &[("FALSE", "Width*BOUND((Prop.msvCalloutField-Prop.msvCalloutPropMin)/(Prop.msvCalloutPropMax-Prop.msvCalloutPropMin),0,FALSE,0,1)")], &[]);
    callout_master(
        &mut tx,
        &mut eng,
        "Traffic lights",
        Kind::IconSet,
        (0.25, 0.25),
        &[("User.msvCalloutIconNumber<>0", "Width"), ("User.msvCalloutIconNumber<>1", "Width"), ("User.msvCalloutIconNumber<>2", "Width")],
        &[("msvCalloutIconCount", "3"), ("msvCalloutIconNumber", "-1")],
    );
    eng.recalc(&mut tx, &[ps]).unwrap();
    tx.commit();
    Fx { doc, eng, page, shapes }
}

fn graphic() -> DataGraphic {
    let mut g = DataGraphic::new("Geräte");
    let mut bar = Item::new(Kind::DataBar, "{Auslastung}", "Progress bar");
    bar.set_opt("msvCalloutPropMax", Opt::Num(200.0));
    g.push(bar);
    let mut icons = Item::new(Kind::IconSet, "{Status}", "Traffic lights");
    icons.rules = vec![Rule::new(Op::Equal, "OK", ""), Rule::new(Op::Equal, "Warnung", ""), Rule::new(Op::Equal, "Fehler", "")];
    icons.pos = Some(Pos { h: H_RIGHT, v: V_TOP });
    g.push(icons);
    let mut text = Item::new(Kind::Text, "{Name}", "Text callout");
    text.pos = Some(Pos { h: H_CENTER, v: V_BELOW });
    g.push(text);
    g
}

fn create_in(f: &mut Fx, g: &DataGraphic) -> MasterRef {
    let mut tx = f.doc.transaction("Neue Datengrafik");
    let m = create(&mut tx, &mut f.eng, g, None).unwrap();
    tx.commit();
    m
}

fn apply_to(f: &mut Fx, shapes: &[ShapeRef], m: MasterRef) -> Vec<ShapeRef> {
    let mut tx = f.doc.transaction("Datengrafik anwenden");
    let done = apply(&mut tx, &mut f.eng, shapes, m, "Datengrafik").unwrap();
    tx.commit();
    done
}

#[test]
fn a_master_is_written_and_read_back() {
    let mut f = fx();
    let mut g = graphic();
    g.hide_text = true;
    g.default_pos = Pos { h: H_FAR_LEFT, v: V_MIDDLE };
    let m = create_in(&mut f, &g);
    assert!(is_graphic(&f.doc, m));
    assert_eq!(graphics(&f.doc), vec![m]);
    let h = &f.doc.master(m).unwrap().header;
    assert!(h.unique_id.is_some_and(|u| f.doc.str(u).starts_with('{') && f.doc.str(u).len() == 38));
    let hidden = f.doc.sym("Hidden").unwrap();
    assert_eq!(crate::attr(&h.attrs, hidden).map(|v| f.doc.str(v)), Some("1"));
    let back = read(&f.doc, m).unwrap();
    assert_eq!(back.name, "Geräte");
    assert!(back.hide_text && !back.show_border);
    assert_eq!(back.default_pos, Pos { h: H_FAR_LEFT, v: V_MIDDLE });
    assert_eq!(back.items.len(), 3);
    assert_eq!(back.next_item, 4);
    for (a, b) in g.items.iter().zip(&back.items) {
        assert_eq!((a.id, a.kind, &a.field, &a.callout, a.pos), (b.id, b.kind, &b.field, &b.callout, b.pos));
        assert_eq!(a.rules, b.rules);
    }
    assert_eq!(back.items[0].opt("msvCalloutPropMax"), Some(&Opt::Num(200.0)));
    assert_eq!(back.items[0].opt("msvCalloutPropMin"), Some(&Opt::Num(0.0)));
    // Visio's cells: the page sheet's marker and layer, the item's USE and field
    let ps = SheetRef::Master(m);
    assert_eq!(user_num(&f.doc, ps, "visUSEType"), Some(1.0));
    assert_eq!(user_num(&f.doc, ps, "visDGNextItemID"), Some(4.0));
    assert!(near(user_num(&f.doc, ps, "msvDGCalloutGap").unwrap(), GAP));
    let layers = f.doc.section_rows(ps, SecKey::one(names::LAYER));
    assert_eq!(layers.first().and_then(|l| l.text("NameUniv")), Some(LAYER_U));
    let group = master_group(&f.doc, m).unwrap();
    assert_eq!(user_formula(&f.doc, SheetRef::Shape(group), "visDGDefaultPos").as_deref(), Some("PNT(0,3)"));
    let first = f.doc.shape(group).unwrap().children().next().unwrap();
    let fs = SheetRef::Shape(first);
    assert_eq!(user_formula(&f.doc, fs, "visDGUseMaster").as_deref(), Some("USE(\"Progress bar\")"));
    assert_eq!(user_num(&f.doc, fs, "visDGUseMaster"), Some(254.0));
    assert_eq!(user_text(&f.doc, fs, "visDGCalloutField").as_deref(), Some("{Auslastung}"));
    assert_eq!(user_text(&f.doc, fs, "msvCalloutType").as_deref(), Some("Data Bar"));
    assert_eq!(user_num(&f.doc, fs, "visDGDefaultPos"), Some(1.0));
    // a second graphic of the same name gets another
    let m2 = create_in(&mut f, &g);
    assert_eq!(master_name(&f.doc, m2), "Geräte (2)");
}

#[test]
fn applying_makes_callouts_that_compute_from_the_shape() {
    let mut f = fx();
    let m = create_in(&mut f, &graphic());
    let all = f.shapes.clone();
    let done = apply_to(&mut f, &all, m);
    assert_eq!(done.len(), 3);
    let a = f.shapes[0];
    let doc = &f.doc;
    assert_eq!(applied(doc, a), Some(m));
    assert_eq!(shapes_with(doc, m), f.shapes);
    assert_eq!(doc.shape(a).unwrap().header().kind, ShapeKind::Group);
    let cs = callouts(doc, a);
    assert_eq!(cs.len(), 3);
    let id = doc.shape(a).unwrap().id();
    // the bar reads the shape's load through its own field: 35 of 0..200
    let bar = cs[0];
    let fa = CellAddr::row(SecKey::one(names::PROPERTY), RowKey::Named(doc.sym("msvCalloutField").unwrap()), names::VALUE);
    assert_eq!(formula(doc, SheetRef::Shape(bar), &fa), Some(alloc::format!("Sheet.{id}!Prop.Load")));
    assert_eq!(num(doc, SheetRef::Shape(bar), "Property", "msvCalloutField", "Value"), Some(35.0));
    assert_eq!(text(doc, SheetRef::Shape(bar), "Property", "msvCalloutField", "SortKey").as_deref(), Some("Auslastung"));
    let x2 = CellAddr::row(SecKey { name: names::GEOMETRY, index: 0 }, RowKey::Index(2), names::X);
    assert!(near(doc.resolve(SheetRef::Shape(bar), &x2).num().unwrap(), 1.0 * 35.0 / 200.0));
    // the icon: Status OK is icon 0, shown alone
    let icon = cs[1];
    assert_eq!(user_num(doc, SheetRef::Shape(icon), "msvCalloutIconNumber"), Some(0.0));
    let shown: Vec<bool> = (0..3).map(|k| doc.resolve(SheetRef::Shape(icon), &CellAddr::section(SecKey { name: names::GEOMETRY, index: k }, names::NO_SHOW)).num() == Some(0.0)).collect();
    assert_eq!(shown, vec![true, false, false]);
    let router_icon = callouts(doc, f.shapes[2])[1];
    assert_eq!(user_num(doc, SheetRef::Shape(router_icon), "msvCalloutIconNumber"), Some(2.0));
    // places: the bar far right in the middle, the icon right at the top, the text centred below
    let gap = GAP;
    assert!(near(single_num(doc, SheetRef::Shape(bar), "PinX"), 2.0 + gap + 0.5));
    assert!(near(single_num(doc, SheetRef::Shape(bar), "PinY"), 0.5));
    assert!(near(single_num(doc, SheetRef::Shape(icon), "PinX"), 2.0 - 0.125));
    assert!(near(single_num(doc, SheetRef::Shape(icon), "PinY"), 1.0 - 0.125));
    let txt = cs[2];
    assert!(near(single_num(doc, SheetRef::Shape(txt), "PinX"), 1.0));
    assert!(near(single_num(doc, SheetRef::Shape(txt), "PinY"), -gap - 0.125));
    // Visio's cells on the shape and on the callout; the layer
    let uid = doc.str(doc.master(m).unwrap().header.unique_id.unwrap());
    assert_eq!(user_formula(doc, SheetRef::Shape(a), "visDGDisplayFormat"), Some(alloc::format!("USE(\"{uid}\")")));
    assert_eq!(user_formula(doc, SheetRef::Shape(a), "visDGDefaultPos").as_deref(), Some("PNT(6,3)"));
    assert!(num(doc, SheetRef::Shape(a), "Control", "msvDGPosition", "X").is_some_and(|x| near(x, 2.0 * 1.25)));
    assert_eq!(user_num(doc, SheetRef::Shape(bar), "visDGCalloutItem"), Some(1.0));
    let ix = doc.layer_by_name(f.page, LAYER_U).unwrap();
    assert_eq!(doc.layer_members(bar), vec![ix]);
    assert!(near(user_num(doc, SheetRef::Page(f.page), "msvDGCalloutGap").unwrap(), gap));
}

#[test]
fn the_control_point_moves_items_at_the_default_position() {
    let mut f = fx();
    let m = create_in(&mut f, &graphic());
    let a = f.shapes[0];
    apply_to(&mut f, &[a], m);
    let bar = callouts(&f.doc, a)[0];
    // the control dragged to the left edge, the top: the bar follows, the fixed items stay
    let mut tx = f.doc.transaction("Steuerpunkt");
    let sheet = SheetRef::Shape(a);
    let cx = named_cell(&mut tx, sheet, names::CONTROL, "msvDGPosition", "X").unwrap();
    let cy = named_cell(&mut tx, sheet, names::CONTROL, "msvDGPosition", "Y").unwrap();
    f.eng.user_set(&mut tx, sheet, &cx, crate::formula::Input::Value(Value::Num(0.05), None)).unwrap();
    f.eng.user_set(&mut tx, sheet, &cy, crate::formula::Input::Value(Value::Num(1.0), None)).unwrap();
    tx.commit();
    let doc = &f.doc;
    assert_eq!(user_num(doc, SheetRef::Shape(bar), "JFD_DGH"), Some(1.0));
    assert_eq!(user_num(doc, SheetRef::Shape(bar), "JFD_DGV"), Some(5.0));
    assert!(near(single_num(doc, SheetRef::Shape(bar), "PinX"), 0.0));
    assert!(near(single_num(doc, SheetRef::Shape(bar), "PinY"), 1.0));
}

#[test]
fn items_at_one_place_stack() {
    let mut f = fx();
    let mut g = DataGraphic::new("Text");
    g.push(Item::new(Kind::Text, "{Name}", "Text callout"));
    g.push(Item::new(Kind::Text, "{Standort}", "Text callout"));
    let mut above = Item::new(Kind::Text, "{Status}", "Text callout");
    above.pos = Some(Pos { h: H_CENTER, v: V_ABOVE });
    g.push(above.clone());
    g.push(above);
    let m = create_in(&mut f, &g);
    let a = f.shapes[0];
    apply_to(&mut f, &[a], m);
    let cs = callouts(&f.doc, a);
    let y: Vec<f64> = cs.iter().map(|&c| single_num(&f.doc, SheetRef::Shape(c), "PinY")).collect();
    assert!(near(y[0], 0.5) && near(y[1], 0.5 - 0.27), "{y:?}");
    assert!(near(y[2], 1.0 + GAP + 0.125) && near(y[3], y[2] + 0.27), "{y:?}");
}

#[test]
fn colour_by_value_and_removal_restore_the_shape() {
    let mut f = fx();
    let mut g = DataGraphic::new("Farben");
    let mut item = Item::new(Kind::ColorByValue, "{Standort}", "");
    item.shades = vec![Shade { value: s("Raum 1"), to: s(""), fill: 0x00ff00, text: Some(0x000080) }, Shade { value: s("Raum 2"), to: s(""), fill: 0xff0000, text: None }];
    g.push(item);
    let mut ranges = Item::new(Kind::ColorByValue, "{Auslastung}", "");
    ranges.ranges = true;
    ranges.shades = vec![Shade { value: s("0"), to: s("50"), fill: 0x00aa00, text: None }, Shade { value: s("50"), to: s("80"), fill: 0xffcc00, text: None }];
    let mut g2 = DataGraphic::new("Bereiche");
    g2.push(ranges);
    let m = create_in(&mut f, &g);
    let m2 = create_in(&mut f, &g2);
    let back = read(&f.doc, m).unwrap();
    assert_eq!(back.items[0].shades, g.items[0].shades);
    assert!(!back.items[0].ranges);
    let (a, b, c) = (f.shapes[0], f.shapes[1], f.shapes[2]);
    let fill = |doc: &Document, x: ShapeRef| match doc.resolve(SheetRef::Shape(x), &CellAddr::single(names::FILL_FOREGND)).value() {
        Value::Text(t) => s(doc.str(t)),
        v => alloc::format!("{v:?}"),
    };
    let before = fill(&f.doc, a);
    assert_eq!(before, "#010203");
    apply_to(&mut f, &[a, b, c], m);
    assert_eq!(fill(&f.doc, a), "#00ff00");
    assert_eq!(fill(&f.doc, b), "#ff0000");
    // the shape keeps its kind (no callouts), its colours are kept
    assert_eq!(f.doc.shape(a).unwrap().header().kind, ShapeKind::Shape);
    assert!(user_text(&f.doc, SheetRef::Shape(a), "visDGOldColors").is_some_and(|t| t.contains("FillForegnd=F:RGB(1,2,3)")));
    // another graphic replaces it: the ranges
    apply_to(&mut f, &[a, b, c], m2);
    assert_eq!(fill(&f.doc, a), "#00aa00");
    assert_eq!(fill(&f.doc, b), "#ffcc00");
    assert_eq!(fill(&f.doc, c), before, "91 is in no range: the old colour");
    // removed: the old formula is back, the rows are gone
    let mut tx = f.doc.transaction("Keine Datengrafik");
    remove(&mut tx, &mut f.eng, &[a, b, c]).unwrap();
    tx.commit();
    assert_eq!(fill(&f.doc, a), before);
    assert_eq!(formula(&f.doc, SheetRef::Shape(a), &CellAddr::single(names::FILL_FOREGND)).as_deref(), Some("RGB(1,2,3)"));
    assert!(user_row(&f.doc, SheetRef::Shape(a), "visDGCBVFill").is_none());
    assert!(user_row(&f.doc, SheetRef::Shape(a), "visDGOldColors").is_none());
    assert_eq!(applied(&f.doc, a), None);
}

#[test]
fn removal_takes_callouts_back_and_undo_restores() {
    let mut f = fx();
    let m = create_in(&mut f, &graphic());
    let a = f.shapes[0];
    let mut g = graphic();
    g.hide_text = true;
    let mut tx = f.doc.transaction("ausblenden");
    update(&mut tx, &mut f.eng, m, &g, None, "Datengrafik").unwrap();
    tx.commit();
    let mut tx = f.doc.transaction("Datengrafik anwenden");
    apply(&mut tx, &mut f.eng, &[a], m, "Datengrafik").unwrap();
    let mut change = tx.commit().unwrap();
    assert_eq!(callouts(&f.doc, a).len(), 3);
    assert_eq!(single_num(&f.doc, SheetRef::Shape(a), "HideText"), 1.0);
    assert!(change.undo(&mut f.doc));
    assert_eq!(callouts(&f.doc, a).len(), 0);
    assert_eq!(applied(&f.doc, a), None);
    assert_eq!(f.doc.shape(a).unwrap().header().kind, ShapeKind::Shape);
    assert!(change.redo(&mut f.doc));
    assert_eq!(callouts(&f.doc, a).len(), 3);
    let mut tx = f.doc.transaction("Keine Datengrafik");
    remove(&mut tx, &mut f.eng, &[a]).unwrap();
    tx.commit();
    assert_eq!(callouts(&f.doc, a).len(), 0);
    assert_eq!(f.doc.shape(a).unwrap().header().kind, ShapeKind::Shape);
    assert!(f.doc.resolve(SheetRef::Shape(a), &CellAddr::single(f.doc.sym("HideText").unwrap())).num().is_none_or(|x| x == 0.0));
    assert!(num(&f.doc, SheetRef::Shape(a), "Control", "msvDGPosition", "X").is_none());
}

#[test]
fn editing_a_graphic_updates_every_shape_that_uses_it() {
    let mut f = fx();
    let m = create_in(&mut f, &graphic());
    let all = f.shapes.clone();
    apply_to(&mut f, &all, m);
    let mut g = read(&f.doc, m).unwrap();
    g.items.remove(2);
    g.items[1].rules = vec![Rule::new(Op::Greater, "80", ""), Rule::new(Op::Between, "50", "80"), Rule::new(Op::Less, "50", "")];
    g.items[1].field = s("{Auslastung}");
    g.name = s("Last");
    let mut tx = f.doc.transaction("Datengrafik bearbeiten");
    let users = update(&mut tx, &mut f.eng, m, &g, None, "Datengrafik").unwrap();
    tx.commit();
    assert_eq!(users, all);
    assert_eq!(master_name(&f.doc, m), "Last");
    for (k, &sh) in all.iter().enumerate() {
        let cs = callouts(&f.doc, sh);
        assert_eq!(cs.len(), 2);
        let want = [2.0, 1.0, 0.0][k];
        assert_eq!(user_num(&f.doc, SheetRef::Shape(cs[1]), "msvCalloutIconNumber"), Some(want), "shape {k}");
    }
    // duplicate, rename, select, delete
    let mut tx = f.doc.transaction("Duplizieren");
    let d = duplicate(&mut tx, &mut f.eng, m, "Last").unwrap();
    assert_eq!(master_name(tx.doc(), d), "Last (2)");
    assert_eq!(rename(&mut tx, d, "Last"), Err(EditError::Exists));
    rename(&mut tx, d, "Kopie").unwrap();
    tx.commit();
    assert_eq!(read(&f.doc, d).unwrap().items.len(), 2);
    assert!(shapes_with(&f.doc, d).is_empty());
    let mut tx = f.doc.transaction("Löschen");
    delete(&mut tx, &mut f.eng, m).unwrap();
    tx.commit();
    assert!(f.doc.master(m).is_none());
    assert!(all.iter().all(|&x| callouts(&f.doc, x).is_empty() && applied(&f.doc, x).is_none()));
    assert_eq!(graphics(&f.doc), vec![d]);
}

#[test]
fn fields_and_formulas() {
    let f = fx();
    let a = f.shapes[0];
    let fs = fields(&f.doc, &[a]);
    assert_eq!(fs.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), vec!["Name", "Auslastung", "Status", "Standort"]);
    assert_eq!(fs[1].row, "Load");
    assert_eq!(resolve_field(&f.doc, a, "{Auslastung}", Some(5)), Some((s("Sheet.5!Prop.Load"), s("Auslastung"))));
    assert_eq!(resolve_field(&f.doc, a, "{Auslastung}*2", None), Some((s("Prop.Load*2"), s("{Auslastung}*2"))));
    assert_eq!(resolve_field(&f.doc, a, "{Kosten}", None), None);
    assert_eq!(icon_formula(&[Rule::new(Op::Equal, "OK", ""), Rule::new(Op::NotUsed, "", ""), Rule::new(Op::Between, "80", "50,5")]), "IFERROR(IF(STRSAME(Prop.msvCalloutField,\"OK\",TRUE),0,IF(AND(Prop.msvCalloutField>=50.5,Prop.msvCalloutField<=80),2,-1)),-1)");
    assert_eq!(Pos::parse("PNT(6, 3)"), Some(Pos { h: 6, v: 3 }));
    assert_eq!(Pos::parse("PNT(9,-1)"), Some(Pos { h: 6, v: 0 }));
    assert_eq!(Kind::parse("Text Callout"), Some(Kind::Text));
    assert_eq!(Kind::parse("Color by Value"), Some(Kind::ColorByValue));
    assert_eq!(use_arg("USE(\"{1A-2B}\")").as_deref(), Some("{1A-2B}"));
    assert_eq!(parse_num("1,5"), Some(1.5));
    assert_ne!(guid(&["a"]), guid(&["b"]));
    assert_eq!(guid(&["a", "b"]), guid(&["a", "b"]));
}

#[test]
fn the_legend_is_a_list_of_containers_of_lists() {
    let mut f = fx();
    let mut g = graphic();
    let mut cbv = Item::new(Kind::ColorByValue, "{Standort}", "");
    cbv.shades = vec![Shade { value: s("Raum 1"), to: s(""), fill: 0x00ff00, text: None }, Shade { value: s("Raum 2"), to: s(""), fill: 0xff0000, text: None }];
    g.push(cbv);
    let m = create_in(&mut f, &g);
    let all = f.shapes.clone();
    apply_to(&mut f, &all, m);
    let t = legend::Texts { title: s("Legende"), ..legend::Texts::default() };
    let groups = legend::groups(&f.doc, f.page, &t);
    assert_eq!(groups.iter().map(|g| g.label.as_str()).collect::<Vec<_>>(), vec!["Auslastung", "Status", "Name", "Standort"]);
    assert_eq!(groups[0].entries[0].text, "0 – 200");
    assert_eq!(groups[1].entries.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), vec!["= OK", "= Warnung", "= Fehler"]);
    assert_eq!(groups[3].entries[1].swatch, legend::Swatch::Color(0xff0000));
    let mut tx = f.doc.transaction("Legende einfügen");
    let outer = legend::insert(&mut tx, &mut f.eng, f.page, false, &t, "Datengrafik").unwrap();
    tx.commit();
    let doc = &f.doc;
    assert!(legend::is_legend(doc, outer));
    assert_eq!(crate::structure::kind(doc, outer), Some(crate::structure::Kind::List));
    let boxes = crate::structure::items(doc, outer);
    assert_eq!(boxes.len(), 4);
    for (k, &c) in boxes.iter().enumerate() {
        assert_eq!(crate::structure::kind(doc, c), Some(crate::structure::Kind::Container));
        let inner = crate::structure::members(doc, c);
        assert_eq!(inner.len(), 1);
        assert_eq!(crate::structure::items(doc, inner[0]).len(), groups[k].entries.len());
        assert_eq!(doc.shape(c).unwrap().text().map(|t| t.chars()).as_deref(), Some(groups[k].label.as_str()));
    }
    // vertical: the containers one below the other, the legend at the top right of the page
    let b0 = crate::structure::shape_box(doc, boxes[0]);
    let b1 = crate::structure::shape_box(doc, boxes[1]);
    assert!(b1[3] <= b0[1] + 1e-6, "{b0:?} {b1:?}");
    let ob = crate::structure::shape_box(doc, outer);
    assert!(near(ob[2], 11.0 - 0.25) && near(ob[3], 8.5 - 0.25), "{ob:?}");
    // a page without data graphics has no legend
    let mut tx = f.doc.transaction("leer");
    let p2 = tx.add_page("Zeichenblatt-2", "Page-2", false, None).unwrap();
    assert_eq!(legend::insert(&mut tx, &mut f.eng, p2, true, &t, "Datengrafik"), Err(EditError::Invalid));
}
