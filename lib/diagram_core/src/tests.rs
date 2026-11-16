// tests -- unit tests of the model: strings, units, cells, inheritance, deleted rows, views,
// transactions with undo and redo, a large drawing
//
//   cargo test -p jflib-diagram-core --lib
//
// Every document here is built in the test; nothing comes from a Visio file.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write as _;

use crate::names;
use crate::*;

// ---- building documents

fn num_cell(doc: &mut Document, name: &str, v: f64) -> Cell {
    Cell::constant(doc.intern(name), Value::Num(v), None)
}

fn formula_cell(doc: &mut Document, name: &str, f: &str, v: f64) -> Cell {
    let n = doc.intern(name);
    let f = doc.intern(f);
    Cell { name: n, value: Value::Num(v), formula: Formula::Expr(f), unit: None, error: None, stale: false }
}

fn row(doc: &mut Document, key: RowKey, kind: Option<&str>, cells: &[(&str, f64)]) -> Row {
    let mut r = Row::new(key);
    r.kind = kind.map(|k| doc.intern(k));
    for &(n, v) in cells {
        let c = num_cell(doc, n, v);
        r.cells.push(c);
    }
    r
}

fn style(doc: &mut Document, id: u32, name: &str, based_on: StyleRefs, cells: &[(&str, f64)]) -> StyleRef {
    let mut st = Style::default();
    st.header.id = id;
    st.header.name_u = Some(doc.intern(name));
    st.header.based_on = based_on;
    for &(n, v) in cells {
        let c = num_cell(doc, n, v);
        st.sheet.cells.push(c);
    }
    doc.insert_style(st).unwrap()
}

struct Fixture {
    doc: Document,
    master: MasterRef,
    page: PageRef,
    // the instance on the page and its subshape
    inst: ShapeRef,
    sub: ShapeRef,
    // a shape without master on the page
    plain: ShapeRef,
}

// No Style (0) <- Normal (1); master 2 "Box" with a group master shape (ID 5) and a subshape
// (ID 6); page 0 with an instance (ID 1, subshape ID 2) and a plain shape (ID 3)
fn fixture() -> Fixture {
    let mut doc = Document::new(DocKind::Drawing);
    style(&mut doc, 0, "No Style", StyleRefs::default(), &[("LineWeight", 0.01), ("FillForegnd", 1.0), ("Rounding", 0.0)]);
    {
        let s0 = doc.style_by_id(0).unwrap();
        let size = doc.intern("Size");
        let mut sec = Section::new(SecKey::one(names::CHARACTER));
        let mut r = Row::new(RowKey::Index(0));
        r.cells.push(Cell::constant(size, Value::Num(0.1667), Some(Unit::Points)));
        sec.rows.push(r);
        doc.style_mut(s0).unwrap().sheet.sections.push(sec);
    }
    style(&mut doc, 1, "Normal", StyleRefs::all(0), &[("LineWeight", 0.02)]);

    // the master
    let mut mh = MasterHeader { id: 2, ..MasterHeader::default() };
    mh.name = Some(doc.intern("Kasten"));
    mh.name_u = Some(doc.intern("Box"));
    let master = doc.insert_master(Master::new(mh)).unwrap();
    let mut ms = ShapeData::new(ShapeKind::Group);
    ms.header.styles = StyleRefs::all(1);
    ms.sheet.cells.push(num_cell(&mut doc, "Width", 1.0));
    ms.sheet.cells.push(formula_cell(&mut doc, "Height", "Width*0.5", 0.5));
    ms.sheet.cells.push(formula_cell(&mut doc, "LocPinX", "GUARD(Width*0.5)", 0.5));
    let mut geo = Section::new(SecKey::one(names::GEOMETRY));
    geo.ix_written = true;
    geo.rows.push(row(&mut doc, RowKey::Index(1), Some("MoveTo"), &[("X", 0.0), ("Y", 0.0)]));
    geo.rows.push(row(&mut doc, RowKey::Index(2), Some("LineTo"), &[("X", 1.0), ("Y", 0.0)]));
    geo.rows.push(row(&mut doc, RowKey::Index(3), Some("LineTo"), &[("X", 1.0), ("Y", 1.0)]));
    ms.sheet.sections.push(geo);
    let mut prop = Section::new(SecKey::one(names::PROPERTY));
    let serial = doc.intern("Serial");
    let mut pr = Row::new(RowKey::Named(serial));
    let label = doc.intern("Seriennummer");
    pr.cells.push(Cell::constant(names::LABEL, Value::Text(label), Some(Unit::Str)));
    pr.cells.push(Cell::constant(names::VALUE, Value::Text(Sym::EMPTY), Some(Unit::Str)));
    prop.rows.push(pr);
    ms.sheet.sections.push(prop);
    ms.text = Some(alloc::boxed::Box::new(Text::plain("Hallo")));
    let tree = &mut doc.master_mut(master).unwrap().shapes;
    let top = tree.insert(None, 5, ms).unwrap();
    let mut msub = ShapeData::new(ShapeKind::Shape);
    msub.sheet.cells.push(Cell::constant(names::WIDTH, Value::Num(0.25), None));
    let tree = &mut doc.master_mut(master).unwrap().shapes;
    tree.insert(Some(top), 6, msub).unwrap();

    // the page
    let mut ph = PageHeader { id: 0, ..PageHeader::default() };
    ph.name = Some(doc.intern("Zeichenblatt-1"));
    ph.name_u = Some(doc.intern("Page-1"));
    let page = doc.insert_page(Page::new(ph)).unwrap();
    let mut inst = ShapeData::new(ShapeKind::Group);
    inst.header.master = Some(2);
    inst.sheet.cells.push(num_cell(&mut doc, "PinX", 3.0));
    let h = doc.intern("Height");
    inst.sheet.cells.push(Cell { name: h, value: Value::Num(0.75), formula: Formula::Inherited, unit: None, error: None, stale: false });
    let mut geo = Section::new(SecKey::one(names::GEOMETRY));
    let mut deleted = Row::new(RowKey::Index(2));
    deleted.del = true;
    geo.rows.push(deleted);
    inst.sheet.sections.push(geo);
    let mut sub = ShapeData::new(ShapeKind::Shape);
    sub.header.master_shape = Some(6);
    let plain = ShapeData::new(ShapeKind::Shape);
    let tree = &mut doc.page_mut(page).unwrap().shapes;
    let i = tree.insert(None, 1, inst).unwrap();
    let s = tree.insert(Some(i), 2, sub).unwrap();
    let p = tree.insert(None, 3, plain).unwrap();
    let owner = Owner::Page(page);
    Fixture {
        doc,
        master,
        page,
        inst: ShapeRef { owner, slot: i },
        sub: ShapeRef { owner, slot: s },
        plain: ShapeRef { owner, slot: p },
    }
}

fn single(doc: &Document, name: &str) -> CellAddr {
    CellAddr::single(doc.sym(name).unwrap_or(Sym::EMPTY))
}

fn geo_addr(ix: u32, cell: Sym) -> CellAddr {
    CellAddr::row(SecKey::one(names::GEOMETRY), RowKey::Index(ix), cell)
}

// the content of a document without slots and allocation state: what undo must bring back
fn dump(doc: &Document) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "settings {:?}", doc.settings);
    for st in doc.styles() {
        let st = doc.style(st).unwrap();
        let _ = writeln!(s, "style {:?} {:?}", st.header, st.sheet);
    }
    for m in doc.masters() {
        let ms = doc.master(m).unwrap();
        let _ = writeln!(s, "master {:?} {:?}", ms.header, ms.sheet);
        dump_tree(doc, Owner::Master(m), &mut s);
    }
    for p in doc.pages() {
        let pg = doc.page(p).unwrap();
        let _ = writeln!(s, "page {:?} {:?} {:?} theme {:?}", pg.header, pg.sheet, pg.connects, pg.theme);
        dump_tree(doc, Owner::Page(p), &mut s);
    }
    s
}

fn dump_tree(doc: &Document, owner: Owner, s: &mut String) {
    for r in doc.all_shapes(owner) {
        let sh = doc.shape(r).unwrap();
        let kids: Vec<u32> = sh.children().map(|c| doc.shape(c).unwrap().id()).collect();
        let _ = writeln!(s, "  shape {} parent {:?} kids {:?} {:?}", sh.id(), sh.parent().map(|p| doc.shape(p).unwrap().id()), kids, sh.data());
    }
}

// ---- strings, cells, units

#[test]
fn strings_intern_and_known_names() {
    let mut s = Strings::new();
    assert_eq!(s.str(Sym::EMPTY), "");
    assert_eq!(s.str(names::PIN_X), "PinX");
    assert_eq!(s.str(names::REL_ELLIPTICAL_ARC_TO), "RelEllipticalArcTo");
    assert_eq!(s.intern("PinX"), names::PIN_X);
    let a = s.intern("THEMEVAL()");
    assert_eq!(s.intern("THEMEVAL()"), a);
    assert_eq!(s.get("nope"), None);
    // growth keeps every handle
    let syms: Vec<Sym> = (0..5000).map(|i| s.intern(&format!("Formel {i}"))).collect();
    for (i, sym) in syms.iter().enumerate() {
        assert_eq!(s.str(*sym), format!("Formel {i}"));
        assert_eq!(s.get(&format!("Formel {i}")), Some(*sym));
    }
    assert_eq!(s.str(a), "THEMEVAL()");
}

#[test]
fn values_formulas_errors_parse() {
    let mut s = Strings::new();
    assert_eq!(Value::parse("0.4921259842519685", None, &mut s), Value::Num(0.4921259842519685));
    assert_eq!(Value::parse("-3", None, &mut s), Value::Num(-3.0));
    assert_eq!(Value::parse("1e-3", None, &mut s), Value::Num(0.001));
    for t in ["Themed", "#ff0000", "1.#INF", "", "inf", "NaN", "+1", "NURBS(1, 3, 0, 0)"] {
        let v = Value::parse(t, None, &mut s);
        assert!(matches!(v, Value::Text(sym) if s.str(sym) == t), "{t}");
    }
    assert_eq!(Formula::parse("Inh", &mut s), Formula::Inherited);
    assert_eq!(Formula::parse("No Formula", &mut s), Formula::NoFormula);
    assert!(matches!(Formula::parse("Width*0.5", &mut s), Formula::Expr(f) if s.str(f) == "Width*0.5"));
    for e in ["#DIM!", "#DIV/0!", "#VALUE!", "#REF!", "#NUM!", "#N/A", "#NAME?"] {
        assert_eq!(CellError::parse(e).unwrap().as_str(), e);
    }
    assert_eq!(CellError::parse("#WHAT"), None);
    assert!(guarded(" guard(Width*0.5)"));
    assert!(!guarded("THEMEGUARD(1)"));
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn units_codes_and_conversion() {
    let codes = [
        "IN", "IN_F", "FT", "F_I", "YD", "MI", "MI_F", "NM", "MM", "CM", "M", "KM", "PT", "P", "P_PT", "D", "C", "C_D", "DL",
        "DP", "DT", "DEG", "AD", "RAD", "DA", "ED", "EH", "EM", "ES", "EW", "DE", "DATE", "AC", "HA", "PER", "CY", "BOOL",
        "COLOR", "STR", "GUID", "PNT", "NURBS", "POLYLINE", "NUM",
    ];
    for c in codes {
        assert_eq!(Unit::parse(c).unwrap().code(), Some(c), "{c}");
    }
    assert_eq!(Unit::parse("XYZ"), None);
    // the internal units: inches, radians, days
    assert!(close(Unit::Millimeters.to_internal(25.4), 1.0));
    assert!(close(Unit::Points.to_internal(8.0), 0.1111111111111111));
    assert!(close(Unit::Degrees.to_internal(270.0), 4.71238898038469));
    assert!(close(Unit::Meters.to_internal(3.0), 118.11023622047244));
    assert!(close(Unit::Feet.to_internal(1.0), 12.0));
    assert!(close(Unit::Centimeters.from_internal(1.0), 2.54));
    assert!(close(Unit::Picas.from_internal(1.0), 6.0));
    assert!(close(Unit::Ciceros.to_internal(1.0), 12.0 * Unit::Didots.to_internal(1.0)));
    assert!((Unit::Didots.to_internal(1.0) * 25.4 - 0.376065).abs() < 1e-6);
    assert!(close(Unit::Weeks.to_internal(1.0), 7.0));
    assert!(close(Unit::Hours.to_internal(36.0), 1.5));
    assert!(close(Unit::Percent.to_internal(50.0), 0.5));
    assert!(close(Unit::Hectares.to_internal(1.0) / Unit::Acres.to_internal(1.0), 2.471053814671653));
    assert!(close(convert(10.0, Unit::Millimeters, Unit::Centimeters).unwrap(), 1.0));
    assert!(close(convert(1.0, Unit::Radians, Unit::Degrees).unwrap(), 180.0 / core::f64::consts::PI));
    assert_eq!(convert(1.0, Unit::Millimeters, Unit::Degrees), None);
    assert_eq!(Unit::Color.factor(), None);
    assert!(close(Unit::Str.to_internal(3.0), 3.0));
    let page = Implicit { drawing: Unit::Millimeters, ..Implicit::default() };
    assert_eq!(page.resolve(Unit::DrawingLength), Unit::Millimeters);
    assert_eq!(page.resolve(Unit::TextLength), Unit::Points);
    assert_eq!(page.resolve(Unit::Feet), Unit::Feet);
    assert_eq!(Unit::DrawingLength.dimension(), Dimension::Length);
    assert_eq!(Unit::Date.dimension(), Dimension::Date);
}

// ---- inheritance

#[test]
fn inheritance_master_style_default() {
    let f = fixture();
    let d = &f.doc;
    let inst = SheetRef::Shape(f.inst);
    // own cell
    let r = d.resolve(inst, &single(d, "PinX"));
    assert_eq!((r.origin, r.num()), (Origin::Local, Some(3.0)));
    // from the master shape
    let r = d.resolve(inst, &single(d, "Width"));
    assert_eq!((r.origin, r.num()), (Origin::Master, Some(1.0)));
    let ms = d.master_shape_of(f.inst).unwrap();
    assert_eq!(r.source, Some(SheetRef::Shape(ms)));
    // F="Inh": the formula from the master, the value the instance's own
    let r = d.resolve(inst, &single(d, "Height"));
    assert_eq!(r.origin, Origin::Master);
    assert!(matches!(r.formula(), Formula::Expr(x) if d.str(x) == "Width*0.5"));
    assert_eq!(r.num(), Some(0.75));
    // line cell: the master shape's LineStyle Normal (1)
    let r = d.resolve(inst, &single(d, "LineWeight"));
    assert_eq!((r.origin, r.num(), r.source), (Origin::Style, Some(0.02), Some(SheetRef::Style(d.style_by_id(1).unwrap()))));
    // fill cell: Normal is based on No Style (0)
    let r = d.resolve(inst, &single(d, "FillForegnd"));
    assert_eq!((r.origin, r.num()), (Origin::Style, Some(1.0)));
    // text section row through TextStyle
    let size = d.sym("Size").unwrap();
    let r = d.resolve(inst, &CellAddr::row(SecKey::one(names::CHARACTER), RowKey::Index(0), size));
    assert_eq!((r.origin, r.num(), r.unit()), (Origin::Style, Some(0.1667), Some(Unit::Points)));
    // nowhere
    let r = d.resolve(inst, &CellAddr::single(names::ANGLE));
    assert_eq!((r.origin, r.value()), (Origin::Default, Value::Absent));
    assert!(!r.exists());
    // a shape without master and without styles has no line weight
    let r = d.resolve(SheetRef::Shape(f.plain), &single(d, "LineWeight"));
    assert_eq!(r.origin, Origin::Default);
    // the subshape inherits from the master's subshape
    let r = d.resolve(SheetRef::Shape(f.sub), &CellAddr::single(names::WIDTH));
    assert_eq!((r.origin, r.num()), (Origin::Master, Some(0.25)));
    assert_eq!(d.master_of(f.sub), Some(f.master));
    assert_eq!(d.value_of(inst, "Width"), Value::Num(1.0));
}

#[test]
fn inheritance_own_style_overrides_master() {
    let mut f = fixture();
    // the instance names style 0 for its lines: line cells skip the master shape
    let tree = &mut f.doc.page_mut(f.page).unwrap().shapes;
    tree.data_mut(f.inst.slot).unwrap().header.styles.line = Some(0);
    let d = &f.doc;
    let r = d.resolve(SheetRef::Shape(f.inst), &single(d, "LineWeight"));
    assert_eq!((r.origin, r.num()), (Origin::Style, Some(0.01)));
    // other cells still come from the master shape
    assert_eq!(d.resolve(SheetRef::Shape(f.inst), &single(d, "Width")).origin, Origin::Master);
}

#[test]
fn deleted_rows_and_sections() {
    let mut f = fixture();
    let inst = SheetRef::Shape(f.inst);
    let rows = f.doc.rows(inst, SecKey::one(names::GEOMETRY));
    let keys: Vec<RowKey> = rows.iter().map(|r| r.key).collect();
    assert_eq!(keys, vec![RowKey::Index(1), RowKey::Index(3)]);
    assert!(rows.iter().all(|r| r.origin == Origin::Master));
    assert_eq!(f.doc.str(rows[0].kind.unwrap()), "MoveTo");
    assert!(!rows[0].local);
    let r = f.doc.resolve(inst, &geo_addr(2, names::X));
    assert_eq!(r.origin, Origin::Deleted);
    let r = f.doc.resolve(inst, &geo_addr(3, names::Y));
    assert_eq!((r.origin, r.num()), (Origin::Master, Some(1.0)));
    // an own override of row 3 keeps the master's row type
    let x = f.doc.intern("X");
    let mut tx = f.doc.transaction("X");
    tx.set_constant(inst, &geo_addr(3, x), Value::Num(2.0), None).unwrap();
    tx.commit();
    let rows = f.doc.rows(inst, SecKey::one(names::GEOMETRY));
    assert!(rows[1].local && f.doc.str(rows[1].kind.unwrap()) == "LineTo");
    // a deleted section ends everything in it
    let mut tx = f.doc.transaction("Geometrie löschen");
    tx.delete_section(inst, SecKey::one(names::GEOMETRY)).unwrap();
    let ch = tx.commit().unwrap();
    assert!(f.doc.section_indices(inst, names::GEOMETRY).is_empty());
    assert_eq!(f.doc.resolve(inst, &geo_addr(1, names::X)).origin, Origin::Deleted);
    assert!(f.doc.rows(inst, SecKey::one(names::GEOMETRY)).is_empty());
    let mut ch = ch;
    assert!(ch.undo(&mut f.doc));
    assert_eq!(f.doc.section_indices(inst, names::GEOMETRY), vec![0]);
    assert_eq!(f.doc.resolve(inst, &geo_addr(3, names::X)).num(), Some(2.0));
}

#[test]
fn views_text_and_names() {
    let mut f = fixture();
    let inst = SheetRef::Shape(f.inst);
    let data = f.doc.shape_data(inst);
    assert_eq!(data.len(), 1);
    assert_eq!(data[0].name(), Some("Serial"));
    assert_eq!(data[0].text("Label"), Some("Seriennummer"));
    assert_eq!(data[0].row.origin, Origin::Master);
    let s = f.doc.shape(f.inst).unwrap();
    assert_eq!(s.text().unwrap().chars(), "Hallo");
    assert_eq!(s.name_u(), "Box.1");
    assert_eq!(s.name(), "Kasten.1");
    assert_eq!(f.doc.shape(f.plain).unwrap().name_u(), "Sheet.3");
    assert_eq!(f.doc.shape(f.sub).unwrap().name(), "Sheet.2");
    // own value over the master's field, own user cell, hyperlink
    let mut tx = f.doc.transaction("Daten");
    let v = tx.intern("SN-42");
    tx.set_shape_data(inst, "Serial", "Seriennummer", Value::Text(v)).unwrap();
    tx.set_user(inst, "Farbe", Value::Num(4.0), None).unwrap();
    tx.set_hyperlink(inst, "Row_1", "https://example.org", "", "Beispiel").unwrap();
    tx.set_shape_names(f.inst, Some("Mein Kasten"), Some("MyBox")).unwrap();
    tx.commit().unwrap();
    let data = f.doc.shape_data(inst);
    assert_eq!(data[0].text("Value"), Some("SN-42"));
    assert!(data[0].row.local);
    let users = f.doc.user_cells(inst);
    assert_eq!((users[0].name(), users[0].num("Value")), (Some("Farbe"), Some(4.0)));
    let links = f.doc.hyperlinks(inst);
    assert_eq!(links[0].text("Address"), Some("https://example.org"));
    let s = f.doc.shape(f.inst).unwrap();
    assert_eq!((s.name(), s.name_u()), (String::from("Mein Kasten"), String::from("MyBox")));
    assert_eq!(f.doc.page_by_name_u("Page-1"), Some(f.page));
    assert_eq!(f.doc.master_by_name_u("Box"), Some(f.master));
    assert_eq!(f.doc.unused_page_name("Page"), "Page-2");
    assert_eq!(f.doc.unused_page_name("Zeichenblatt"), "Zeichenblatt-2");
}

// ---- transactions

#[test]
fn transaction_undo_redo_cells() {
    let mut f = fixture();
    let before = dump(&f.doc);
    let inst = SheetRef::Shape(f.inst);
    let pin_y = CellAddr::single(names::PIN_Y);
    let pin_x = CellAddr::single(names::PIN_X);
    let mut tx = f.doc.transaction("Verschieben");
    tx.set_length(inst, &pin_x, 50.8, Unit::Millimeters).unwrap();
    tx.set_length(inst, &pin_y, 1.0, Unit::Inches).unwrap();
    let mut ch = tx.commit().unwrap();
    assert_eq!(ch.label(), "Verschieben");
    assert_eq!(ch.sheets(), vec![inst]);
    let r = f.doc.resolve(inst, &pin_x);
    assert!(close(r.num().unwrap(), 2.0) && r.unit() == Some(Unit::Millimeters));
    let after = dump(&f.doc);
    assert!(ch.undo(&mut f.doc));
    assert_eq!(dump(&f.doc), before);
    assert!(!ch.undo(&mut f.doc));
    assert!(ch.redo(&mut f.doc));
    assert_eq!(dump(&f.doc), after);
    assert!(ch.undo(&mut f.doc) && ch.redo(&mut f.doc) && ch.undo(&mut f.doc));
    assert_eq!(dump(&f.doc), before);
}

#[test]
fn transaction_dropped_rolls_back() {
    let mut f = fixture();
    let before = dump(&f.doc);
    {
        let mut tx = f.doc.transaction("Abbruch");
        tx.add_shape(Owner::Page(f.page), None, ShapeKind::Shape).unwrap();
        tx.set_constant(SheetRef::Shape(f.plain), &CellAddr::single(names::WIDTH), Value::Num(9.0), None).unwrap();
        tx.add_page("Zeichenblatt-2", "Page-2", false, None).unwrap();
    }
    assert_eq!(dump(&f.doc), before);
    let tx = f.doc.transaction("leer");
    assert!(tx.commit().is_none());
}

#[test]
fn formula_and_value_stay_a_pair() {
    let mut f = fixture();
    let inst = SheetRef::Shape(f.inst);
    let width = CellAddr::single(names::WIDTH);
    let mut tx = f.doc.transaction("Formel");
    tx.set_formula(inst, &width, "Height*2").unwrap();
    tx.commit();
    let c = f.doc.sheet(inst).unwrap().cell(names::WIDTH).copied().unwrap();
    // the old value stays until the engine computes the new one
    assert!(c.stale && c.value == Value::Num(1.0) && matches!(c.formula, Formula::Expr(x) if f.doc.str(x) == "Height*2"));
    let mut tx = f.doc.transaction("Ergebnis");
    tx.set_result(inst, &width, Value::Num(1.5), None).unwrap();
    // an inherited formula's result becomes an F="Inh" cell of the instance
    tx.set_result(SheetRef::Shape(f.sub), &width, Value::Num(0.3), None).unwrap();
    tx.commit();
    let c = f.doc.sheet(inst).unwrap().cell(names::WIDTH).copied().unwrap();
    assert!(!c.stale && c.value == Value::Num(1.5));
    let c = f.doc.sheet(SheetRef::Shape(f.sub)).unwrap().cell(names::WIDTH).copied().unwrap();
    assert_eq!((c.formula, c.value), (Formula::Inherited, Value::Num(0.3)));
    let r = f.doc.resolve(SheetRef::Shape(f.sub), &width);
    assert_eq!((r.origin, r.num()), (Origin::Master, Some(0.3)));
    // a constant replaces the formula; inherit_cell brings the master's back
    let mut tx = f.doc.transaction("Wert");
    tx.set_constant(inst, &width, Value::Num(2.0), Some(Unit::Inches)).unwrap();
    tx.commit();
    let c = f.doc.sheet(inst).unwrap().cell(names::WIDTH).copied().unwrap();
    assert_eq!((c.formula, c.value, c.unit), (Formula::Absent, Value::Num(2.0), Some(Unit::Inches)));
    let mut tx = f.doc.transaction("Formel löschen");
    tx.inherit_cell(inst, &width).unwrap();
    tx.commit();
    assert_eq!(f.doc.resolve(inst, &width).origin, Origin::Master);
    // GUARD
    let lpx = CellAddr::single(names::LOC_PIN_X);
    let mut tx = f.doc.transaction("Drehpunkt");
    assert_eq!(tx.set_constant(inst, &lpx, Value::Num(0.0), None), Err(EditError::Guarded));
    assert_eq!(tx.set_formula(inst, &lpx, "0"), Err(EditError::Guarded));
    tx.force_constant(inst, &lpx, Value::Num(0.0), None).unwrap();
    assert!(tx.commit().is_some());
    assert_eq!(f.doc.resolve(inst, &lpx).origin, Origin::Local);
    // a result for a deleted row is refused
    let mut tx = f.doc.transaction("x");
    assert_eq!(tx.set_result(inst, &geo_addr(2, names::X), Value::Num(1.0), None), Err(EditError::Deleted));
}

#[test]
fn rows_delete_restore_add() {
    let mut f = fixture();
    let before = dump(&f.doc);
    let inst = SheetRef::Shape(f.inst);
    let geo = SecKey::one(names::GEOMETRY);
    let mut tx = f.doc.transaction("Zeilen");
    // writing into a deleted row is refused, restoring it brings the master's row back
    assert_eq!(tx.set_constant(inst, &geo_addr(2, names::X), Value::Num(5.0), None), Err(EditError::Deleted));
    tx.restore_row(inst, geo, RowKey::Index(2)).unwrap();
    assert_eq!(tx.doc().resolve(inst, &geo_addr(2, names::X)).num(), Some(1.0));
    // an inherited row gets a Del mark, an own row goes
    tx.delete_row(inst, geo, RowKey::Index(1)).unwrap();
    let key = tx.add_row(inst, geo, None, Some("LineTo")).unwrap();
    assert_eq!(key, RowKey::Index(4));
    tx.set_constant(inst, &geo_addr(4, names::X), Value::Num(0.5), None).unwrap();
    let rows: Vec<RowKey> = tx.doc().rows(inst, geo).iter().map(|r| r.key).collect();
    assert_eq!(rows, vec![RowKey::Index(2), RowKey::Index(3), RowKey::Index(4)]);
    assert_eq!(tx.add_row(inst, geo, Some(RowKey::Index(3)), None), Err(EditError::Exists));
    tx.delete_row(inst, geo, RowKey::Index(4)).unwrap();
    assert_eq!(tx.delete_row(inst, geo, RowKey::Index(4)), Err(EditError::NoRow));
    // a cell in a row that exists nowhere: refused, nothing left behind
    let mid = dump(tx.doc());
    assert_eq!(tx.set_constant(inst, &geo_addr(9, names::X), Value::Num(1.0), None), Err(EditError::NoRow));
    let sec7 = SecKey { name: names::GEOMETRY, index: 7 };
    assert_eq!(tx.set_constant(inst, &CellAddr::row(sec7, RowKey::Index(1), names::X), Value::Num(1.0), None), Err(EditError::NoSection));
    assert_eq!(dump(tx.doc()), mid);
    let mut ch = tx.commit().unwrap();
    let after = dump(&f.doc);
    assert!(ch.undo(&mut f.doc));
    assert_eq!(dump(&f.doc), before);
    assert!(ch.redo(&mut f.doc));
    assert_eq!(dump(&f.doc), after);
    // a new Geometry section of the instance's own
    let mut tx = f.doc.transaction("Geometrie");
    let g2 = tx.add_section(inst, "Geometry").unwrap();
    assert_eq!(g2.index, 1);
    tx.add_row(inst, g2, None, Some("MoveTo")).unwrap();
    tx.commit();
    assert_eq!(f.doc.section_indices(inst, names::GEOMETRY), vec![0, 1]);
    assert_eq!(f.doc.geometry(inst).len(), 2);
}

#[test]
fn shapes_add_delete_move_undo() {
    let mut f = fixture();
    let owner = Owner::Page(f.page);
    let before = dump(&f.doc);
    let mut tx = f.doc.transaction("Gruppe");
    let g = tx.add_shape(owner, None, ShapeKind::Shape).unwrap();
    let a = tx.add_shape(owner, Some(g), ShapeKind::Shape).unwrap();
    let b = tx.add_shape(owner, Some(g), ShapeKind::Foreign).unwrap();
    assert_eq!(tx.doc().shape(g).unwrap().header().kind, ShapeKind::Group);
    let (ida, idb) = (tx.doc().shape(a).unwrap().id(), tx.doc().shape(b).unwrap().id());
    assert_eq!((ida, idb), (5, 6));
    tx.add_connect(owner, Connect { from_sheet: ida, from_cell: Some(names::BEGIN_X), from_part: Some(9), to_sheet: 3, to_cell: Some(names::PIN_X), to_part: Some(3), extras: Extras::new() }).unwrap();
    tx.add_connect(owner, Connect { from_sheet: 1, from_cell: None, from_part: None, to_sheet: 3, to_cell: None, to_part: None, extras: Extras::new() }).unwrap();
    // z-order and grouping
    tx.send_to_back(b).unwrap();
    let kids: Vec<ShapeRef> = tx.doc().shape(g).unwrap().children().collect();
    assert_eq!(kids, vec![b, a]);
    assert_eq!(tx.move_shape(g, Some(a), None), Err(EditError::Cycle));
    tx.move_shape(f.plain, Some(g), Some(1)).unwrap();
    let kids: Vec<ShapeRef> = tx.doc().shape(g).unwrap().children().collect();
    assert_eq!(kids, vec![b, f.plain, a]);
    tx.bring_forward(b).unwrap();
    tx.bring_to_front(f.plain).unwrap();
    let kids: Vec<ShapeRef> = tx.doc().shape(g).unwrap().children().collect();
    assert_eq!(kids, vec![b, a, f.plain]);
    let mut ch = tx.commit().unwrap();
    let grouped = dump(&f.doc);
    // deleting the group takes its subshapes and their glue records
    let mut tx = f.doc.transaction("Löschen");
    tx.delete_shape(g).unwrap();
    let mut del = tx.commit().unwrap();
    assert!(f.doc.shape(a).is_none() && f.doc.shape(f.plain).is_none());
    assert!(f.doc.page(f.page).unwrap().connects.is_empty());
    assert_eq!(f.doc.shape_by_id(owner, ida), None);
    assert!(del.undo(&mut f.doc));
    assert_eq!(dump(&f.doc), grouped);
    // the handles are valid again
    assert_eq!(f.doc.shape(a).unwrap().id(), ida);
    assert_eq!(f.doc.shape_by_id(owner, 3), Some(f.plain));
    assert!(ch.undo(&mut f.doc));
    assert_eq!(dump(&f.doc), before);
    // new shapes after an undo get new IDs, never the undone ones
    let mut tx = f.doc.transaction("neu");
    let c = tx.add_shape(owner, None, ShapeKind::Shape).unwrap();
    tx.commit();
    assert_eq!(f.doc.shape(c).unwrap().id(), 7);
    assert!(c != a && c != b && c != g);
}

#[test]
fn instances_and_deleted_subshapes() {
    let mut f = fixture();
    let owner = Owner::Page(f.page);
    let mut tx = f.doc.transaction("Ablegen");
    let i = tx.add_instance(owner, f.master, 4.0, 2.0).unwrap();
    tx.set_shape_names(i, Some("Kasten.4"), None).unwrap();
    let mut ch = tx.commit().unwrap();
    let s = f.doc.shape(i).unwrap();
    assert_eq!((s.id(), s.header().master, s.header().kind), (4, Some(2), ShapeKind::Group));
    let sub = s.children().next().unwrap();
    assert_eq!(f.doc.shape(sub).unwrap().header().master_shape, Some(6));
    assert_eq!(f.doc.resolve(SheetRef::Shape(sub), &CellAddr::single(names::WIDTH)).num(), Some(0.25));
    assert_eq!(f.doc.resolve(SheetRef::Shape(i), &CellAddr::single(names::PIN_X)).num(), Some(4.0));
    assert_eq!(f.doc.shape(i).unwrap().name(), "Kasten.4");
    assert_eq!(f.doc.shape(i).unwrap().name_u(), "Box.4");
    // deleting a subshape the master passes on marks it
    let mut tx = f.doc.transaction("Teil löschen");
    tx.delete_shape(sub).unwrap();
    tx.commit();
    assert!(f.doc.shape(sub).unwrap().is_deleted());
    let mut tx = f.doc.transaction("zurück");
    tx.undelete_shape(sub).unwrap();
    tx.commit();
    assert!(!f.doc.shape(sub).unwrap().is_deleted());
    // the master is in use
    let mut tx = f.doc.transaction("Master löschen");
    assert_eq!(tx.delete_master(f.master), Err(EditError::InUse));
    drop(tx);
    assert!(ch.undo(&mut f.doc));
    assert!(f.doc.shape(i).is_none());
}

#[test]
fn pages_backgrounds_masters_styles() {
    let mut f = fixture();
    let before = dump(&f.doc);
    f.doc.settings.top_page = Some(0);
    let mut tx = f.doc.transaction("Seiten");
    let bg = tx.add_page("Hintergrund-1", "Background-1", true, None).unwrap();
    let bg2 = tx.add_page("Hintergrund-2", "Background-2", true, None).unwrap();
    let p2 = tx.add_page("Zeichenblatt-2", "Page-2", false, Some(0)).unwrap();
    assert_eq!(tx.doc().pages().collect::<Vec<_>>(), vec![p2, f.page, bg, bg2]);
    assert_eq!(tx.set_background(f.page, Some(p2)), Err(EditError::NotBackground));
    tx.set_background(f.page, Some(bg)).unwrap();
    tx.set_background(bg, Some(bg2)).unwrap();
    assert_eq!(tx.set_background(bg2, Some(bg)), Err(EditError::Cycle));
    assert_eq!(tx.doc().background_of(f.page), Some(bg));
    tx.move_page(p2, 3).unwrap();
    assert_eq!(tx.doc().foreground_pages().collect::<Vec<_>>(), vec![f.page, p2]);
    tx.delete_page(bg).unwrap();
    assert_eq!(tx.doc().background_of(f.page), None);
    tx.delete_page(f.page).unwrap();
    assert_eq!(tx.doc().settings.top_page, tx.doc().page(p2).map(|p| p.header.id));
    // masters and styles
    let m = tx.add_master("Kreis", "Circle").unwrap();
    let ms = tx.add_shape(Owner::Master(m), None, ShapeKind::Shape).unwrap();
    assert_eq!(tx.doc().master(m).unwrap().master_shape(), Some(ms.slot));
    let s2 = tx.add_style("Verbinder", "Connector", StyleRefs::all(1)).unwrap();
    let s2id = tx.doc().style(s2).unwrap().header.id;
    assert_eq!(s2id, 2);
    let s1 = tx.doc().style_by_id(1).unwrap();
    assert_eq!(tx.set_based_on(s1, StyleRefs::all(s2id)), Err(EditError::Cycle));
    assert_eq!(tx.delete_style(s1), Err(EditError::InUse));
    tx.set_shape_styles(ms, StyleRefs::all(s2id)).unwrap();
    assert_eq!(tx.delete_style(s2), Err(EditError::InUse));
    tx.rename_style(s2, "Linie", "Line").unwrap();
    tx.rename_master(m, "Ring", "Ring").unwrap();
    tx.update_settings(|s| s.default_line_style = Some(2)).unwrap();
    let mut ch = tx.commit().unwrap();
    let after = dump(&f.doc);
    assert_eq!(f.doc.style_by_name_u("Line"), Some(s2));
    assert!(ch.undo(&mut f.doc));
    f.doc.settings.top_page = None;
    assert_eq!(dump(&f.doc), before);
    f.doc.settings.top_page = Some(0);
    assert!(ch.redo(&mut f.doc));
    assert_eq!(dump(&f.doc), after);
    assert!(f.doc.page(f.page).is_none());
}

#[test]
fn layers() {
    let mut f = fixture();
    let mut tx = f.doc.transaction("Ebenen");
    let a = tx.add_layer(f.page, "Verbinder", "Connector").unwrap();
    let b = tx.add_layer(f.page, "Geräte", "Devices").unwrap();
    assert_eq!((a, b), (0, 1));
    tx.set_layers(f.inst, &[0, 1]).unwrap();
    tx.set_layers(f.plain, &[1]).unwrap();
    let mut ch = tx.commit().unwrap();
    let layers = f.doc.layers(f.page);
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[1].text("Name"), Some("Geräte"));
    assert!(layers[1].flag("Visible") && !layers[1].flag("Lock"));
    assert_eq!(f.doc.layer_by_name(f.page, "Devices"), Some(1));
    assert_eq!(f.doc.layer_members(f.inst), vec![0, 1]);
    assert_eq!(f.doc.layer_members(f.plain), vec![1]);
    let mut tx = f.doc.transaction("Ebene löschen");
    tx.delete_layer(f.page, 1).unwrap();
    let mut del = tx.commit().unwrap();
    assert_eq!(f.doc.layers(f.page).len(), 1);
    assert_eq!(f.doc.layer_members(f.inst), vec![0]);
    assert!(f.doc.layer_members(f.plain).is_empty());
    assert!(del.undo(&mut f.doc));
    assert_eq!(f.doc.layer_members(f.plain), vec![1]);
    assert!(ch.undo(&mut f.doc));
    assert!(f.doc.layers(f.page).is_empty());
}

#[test]
fn merge_drag_steps() {
    let mut f = fixture();
    let inst = SheetRef::Shape(f.inst);
    let pin_x = CellAddr::single(names::PIN_X);
    let mut first: Option<Change> = None;
    for x in 1..=5 {
        let mut tx = f.doc.transaction("Ziehen");
        tx.set_constant(inst, &pin_x, Value::Num(3.0 + x as f64), None).unwrap();
        let ch = tx.commit().unwrap().merging(7);
        match first.as_mut() {
            None => first = Some(ch),
            Some(c) => assert!(c.merge(&ch)),
        }
    }
    let mut ch = first.unwrap();
    assert_eq!(ch.steps(), 5);
    assert_eq!(f.doc.resolve(inst, &pin_x).num(), Some(8.0));
    assert!(ch.undo(&mut f.doc));
    assert_eq!(f.doc.resolve(inst, &pin_x).num(), Some(3.0));
    assert!(ch.redo(&mut f.doc));
    assert_eq!(f.doc.resolve(inst, &pin_x).num(), Some(8.0));
    let other = f.doc.transaction("x").commit();
    assert!(other.is_none());
    let mut tx = f.doc.transaction("anders");
    tx.set_constant(inst, &pin_x, Value::Num(0.0), None).unwrap();
    let plain = tx.commit().unwrap();
    assert!(!ch.merge(&plain));
}

#[test]
fn untracked_change_makes_undo_refuse() {
    let mut f = fixture();
    let inst = SheetRef::Shape(f.inst);
    let mut tx = f.doc.transaction("Breite");
    tx.set_constant(inst, &CellAddr::single(names::WIDTH), Value::Num(2.0), None).unwrap();
    let mut ch = tx.commit().unwrap();
    // someone removes the cells behind the history's back
    f.doc.page_mut(f.page).unwrap().shapes.data_mut(f.inst.slot).unwrap().sheet.cells.clear();
    let before = dump(&f.doc);
    assert!(!ch.undo(&mut f.doc));
    assert_eq!(dump(&f.doc), before);
    assert!(!ch.is_undone());
}

// the pattern of lib/ui/history.rs: an Edit wraps the Change (the trait mirrored here, ui is not
// a dependency of the model)
trait Edit<S> {
    fn apply(&mut self, s: &mut S);
    fn revert(&mut self, s: &mut S);
    fn label(&self) -> &str;
}

struct Step(Change);

impl Edit<Document> for Step {
    fn apply(&mut self, d: &mut Document) {
        self.0.redo(d);
    }
    fn revert(&mut self, d: &mut Document) {
        self.0.undo(d);
    }
    fn label(&self) -> &str {
        self.0.label()
    }
}

#[test]
fn history_adapter() {
    let mut f = fixture();
    let start = dump(&f.doc);
    let mut done: Vec<alloc::boxed::Box<dyn Edit<Document>>> = Vec::new();
    for x in 0..3 {
        let mut tx = f.doc.transaction(&format!("Schritt {x}"));
        tx.add_shape(Owner::Page(f.page), None, ShapeKind::Shape).unwrap();
        tx.set_constant(SheetRef::Shape(f.plain), &CellAddr::single(names::PIN_X), Value::Num(x as f64), None).unwrap();
        done.push(alloc::boxed::Box::new(Step(tx.commit().unwrap())));
    }
    let end = dump(&f.doc);
    for e in done.iter_mut().rev() {
        e.revert(&mut f.doc);
    }
    assert_eq!(dump(&f.doc), start);
    for e in done.iter_mut() {
        e.apply(&mut f.doc);
    }
    assert_eq!(dump(&f.doc), end);
    assert_eq!(done[2].label(), "Schritt 2");
}

// ---- a large drawing: thousands of shapes, no quadratic access

#[test]
fn large_drawing() {
    let mut f = fixture();
    let owner = Owner::Page(f.page);
    const N: u32 = 20_000;
    {
        let tree = &mut f.doc.page_mut(f.page).unwrap().shapes;
        for i in 0..N {
            let mut d = ShapeData::new(ShapeKind::Shape);
            d.header.master = Some(2);
            d.sheet.cells.push(Cell::constant(names::PIN_X, Value::Num(i as f64), None));
            tree.insert(None, 100 + i, d).unwrap();
        }
    }
    assert_eq!(f.doc.tree(owner).unwrap().len(), N as usize + 3);
    let mut sum = 0.0;
    for r in (0..N).map(|i| f.doc.shape_by_id(owner, 100 + i).unwrap()) {
        sum += f.doc.resolve(SheetRef::Shape(r), &CellAddr::single(names::WIDTH)).num().unwrap_or(0.0);
        assert!(f.doc.rows(SheetRef::Shape(r), SecKey::one(names::GEOMETRY)).len() >= 2);
    }
    assert!(sum >= N as f64);
    // move all of them in one step, undo, redo
    let shapes: Vec<ShapeRef> = (0..N).map(|i| f.doc.shape_by_id(owner, 100 + i).unwrap()).collect();
    let mut tx = f.doc.transaction("Alle verschieben");
    for &s in &shapes {
        tx.set_constant(SheetRef::Shape(s), &CellAddr::single(names::PIN_Y), Value::Num(1.0), None).unwrap();
    }
    let mut ch = tx.commit().unwrap();
    assert_eq!(ch.steps(), N as usize);
    assert!(ch.undo(&mut f.doc) && ch.redo(&mut f.doc));
    // delete the last shapes one by one (by ID through the tree's map)
    let mut tx = f.doc.transaction("Löschen");
    for &s in shapes.iter().rev().take(2000) {
        tx.delete_shape(s).unwrap();
    }
    let mut del = tx.commit().unwrap();
    assert!(del.undo(&mut f.doc));
    assert_eq!(f.doc.tree(owner).unwrap().len(), N as usize + 3);
    assert!(core::mem::size_of::<Cell>() <= 48, "{}", core::mem::size_of::<Cell>());
}

// ---- cases found in review

#[test]
fn header_ids_stay_unique_on_redo() {
    let mut f = fixture();
    let mut tx = f.doc.transaction("Seite");
    let p2 = tx.add_page("Zeichenblatt-2", "Page-2", false, None).unwrap();
    tx.commit();
    let mut h = f.doc.page(f.page).unwrap().header.clone();
    h.id = 9;
    let mut tx = f.doc.transaction("ID");
    tx.set_page_header(f.page, h).unwrap();
    let mut first = tx.commit().unwrap();
    assert!(first.undo(&mut f.doc));
    let mut h2 = f.doc.page(p2).unwrap().header.clone();
    h2.id = 9;
    let mut tx = f.doc.transaction("ID 2");
    tx.set_page_header(p2, h2).unwrap();
    tx.commit();
    // redoing the first would give two pages the ID 9: refused, nothing changes
    assert!(!first.redo(&mut f.doc));
    assert!(!first.is_broken());
    assert_eq!((f.doc.page_by_id(9), f.doc.page_by_id(0)), (Some(p2), Some(f.page)));
}

#[test]
fn ids_run_out_without_reuse() {
    let mut f = fixture();
    let tree = &mut f.doc.page_mut(f.page).unwrap().shapes;
    tree.insert(None, u32::MAX, ShapeData::new(ShapeKind::Shape)).unwrap();
    assert_eq!(tree.next_id(), None);
    let mut tx = f.doc.transaction("neu");
    assert_eq!(tx.add_shape(Owner::Page(f.page), None, ShapeKind::Shape), Err(EditError::NoIds));
    assert_eq!(tx.add_instance(Owner::Page(f.page), f.master, 0.0, 0.0), Err(EditError::NoIds));
    assert!(tx.commit().is_none());
}

#[test]
fn next_index_passes_deleted_marks() {
    let mut f = fixture();
    let inst = SheetRef::Shape(f.inst);
    let geo = SecKey::one(names::GEOMETRY);
    let mut tx = f.doc.transaction("Ecke");
    // rows 1 and 3 are left (2 is deleted); delete 3, then add a row: it gets 4, past both marks
    tx.delete_row(inst, geo, RowKey::Index(3)).unwrap();
    assert_eq!(tx.add_row(inst, geo, None, Some("LineTo")).unwrap(), RowKey::Index(4));
    // the deleted inherited section keeps its index; a new one gets the next
    tx.delete_section(inst, geo).unwrap();
    assert_eq!(tx.add_section(inst, "Geometry").unwrap().index, 1);
    assert!(tx.commit().is_some());
}

#[test]
fn text_cells_keep_their_text() {
    let mut s = Strings::new();
    for (t, u) in [("007", Some(Unit::Str)), ("1E3", Some(Unit::Str)), ("12345678901234567890", Some(Unit::Str)), ("{0}", Some(Unit::Guid))] {
        assert!(matches!(Value::parse(t, u, &mut s), Value::Text(x) if s.str(x) == t), "{t}");
    }
    let other = Unit::Other(s.intern("XY"));
    assert!(matches!(Value::parse("5", Some(other), &mut s), Value::Text(_)));
    assert!(matches!(Value::parse("1e999", None, &mut s), Value::Text(x) if s.str(x) == "1e999"));
    assert_eq!(Value::parse("007", Some(Unit::Millimeters), &mut s), Value::Num(7.0));
    assert_eq!(convert(1.0, Unit::Str, Unit::Color), None);
    assert_eq!(convert(1.0, other, other), None);
}

#[test]
fn structure_keeps_inheritance() {
    let mut f = fixture();
    let owner = Owner::Page(f.page);
    let mut tx = f.doc.transaction("Verschieben");
    // a subshape of an instance stays in its instance
    assert_eq!(tx.move_shape(f.sub, None, None), Err(EditError::Invalid));
    assert_eq!(tx.move_shape(f.sub, Some(f.plain), None), Err(EditError::Invalid));
    tx.move_shape(f.sub, Some(f.inst), Some(0)).unwrap();
    // the master shape of a master in use stays the master shape
    let ms = tx.doc().master_shape_of(f.inst).unwrap();
    let extra = tx.add_shape(Owner::Master(f.master), None, ShapeKind::Shape).unwrap();
    assert_eq!(tx.send_to_back(extra), Err(EditError::InUse));
    assert_eq!(tx.move_shape(extra, None, Some(0)), Err(EditError::InUse));
    assert_eq!(tx.delete_shape(ms), Err(EditError::InUse));
    tx.delete_shape(extra).unwrap();
    // no instance of a master inside a master it depends on, at any depth
    let c = tx.add_master("Gruppe", "Group").unwrap();
    tx.add_shape(Owner::Master(c), None, ShapeKind::Group).unwrap();
    tx.add_instance(Owner::Master(c), f.master, 0.0, 0.0).unwrap();
    assert_eq!(tx.add_instance(Owner::Master(f.master), c, 0.0, 0.0), Err(EditError::Cycle));
    assert_eq!(tx.add_instance(Owner::Master(c), c, 0.0, 0.0), Err(EditError::Cycle));
    let mut ch = tx.commit().unwrap();
    assert_eq!(f.doc.resolve(SheetRef::Shape(f.sub), &CellAddr::single(names::WIDTH)).origin, Origin::Master);
    assert!(ch.undo(&mut f.doc));
    assert_eq!(f.doc.tree(owner).unwrap().len(), 3);
}

#[test]
fn delete_many_and_sheets_touched() {
    let mut f = fixture();
    let owner = Owner::Page(f.page);
    let before = dump(&f.doc);
    let mut tx = f.doc.transaction("Viele");
    let g = tx.add_shape(owner, None, ShapeKind::Group).unwrap();
    let a = tx.add_shape(owner, Some(g), ShapeKind::Shape).unwrap();
    let ida = tx.doc().shape(a).unwrap().id();
    tx.add_connect(owner, Connect { from_sheet: ida, from_cell: None, from_part: None, to_sheet: 3, to_cell: None, to_part: None, extras: Extras::new() }).unwrap();
    let ch = tx.commit().unwrap();
    let touched = ch.sheets();
    assert!(touched.contains(&SheetRef::Shape(g)) && touched.contains(&SheetRef::Shape(a)) && touched.contains(&SheetRef::Page(f.page)));
    let mid = dump(&f.doc);
    // a, g twice, the plain shape and the instance's subshape (a Del mark)
    let mut tx = f.doc.transaction("Löschen");
    tx.delete_shapes(&[a, g, g, f.plain, f.sub]).unwrap();
    let mut del = tx.commit().unwrap();
    assert!(f.doc.shape(g).is_none() && f.doc.shape(f.plain).is_none());
    assert!(f.doc.shape(f.sub).unwrap().is_deleted());
    assert!(f.doc.page(f.page).unwrap().connects.is_empty());
    assert!(del.undo(&mut f.doc));
    assert_eq!(dump(&f.doc), mid);
    let mut ch = ch;
    assert!(ch.undo(&mut f.doc));
    assert_eq!(dump(&f.doc), before);
}
