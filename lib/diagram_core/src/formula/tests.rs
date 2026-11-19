// tests of the formula engine (cargo test -p jflib-diagram-core --lib)
//
//   1. the parser: numbers with units, strings, literals, precedence, references, errors
//   2. functions through Engine::evaluate_text on a small page
//   3. points and coordinate systems (PAR, LOC, a subshape of a turned group)
//   4. the engine on documents: calculate and write, inheritance, Keep, cycles, deep chains,
//      recalc with undo, user_set, run_action, themed values, defaults
//   5. Stored::matches, close and Val::stored
//
// Every document is built here; nothing is read from files.

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::parse::Node;
use super::*;
use crate::names;
use crate::{
    Cell, CellAddr, CellError, ColorEntry, DocKind, Document, EditError, Formula, Master, MasterHeader, MasterRef, Owner,
    Page, PageHeader, PageRef, Row, RowKey, SecKey, Section, ShapeData, ShapeKind, ShapeRef, Sheet, SheetRef, Strings,
    Style, StyleRefs, Sym, Text, Unit, Value,
};

const PI: f64 = core::f64::consts::PI;

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * b.abs().max(1.0)
}

macro_rules! assert_near {
    ($a:expr, $b:expr) => {{
        let (a, b): (f64, f64) = ($a, $b);
        assert!(near(a, b), "{}: {} is not {}", stringify!($a), a, b);
    }};
}

// ---- building documents

fn num(doc: &mut Document, name: &str, v: f64) -> Cell {
    Cell::constant(doc.intern(name), Value::Num(v), None)
}

fn with_f(doc: &mut Document, name: &str, f: &str, v: Value) -> Cell {
    let (n, f) = (doc.intern(name), doc.intern(f));
    Cell { name: n, value: v, formula: Formula::Expr(f), unit: None, error: None, stale: false }
}

// a row in a section of a sheet (the section is made when missing)
fn put_row(sheet: &mut Sheet, section: Sym, key: RowKey, cells: Vec<Cell>) {
    let pos = match sheet.sections.iter().position(|s| s.name == section) {
        Some(p) => p,
        None => {
            sheet.sections.push(Section::new(SecKey::one(section)));
            sheet.sections.len() - 1
        }
    };
    let mut r = Row::new(key);
    r.cells = cells;
    sheet.sections[pos].rows.push(r);
}

// User.<name>: a constant or a formula with its stored value
fn user(doc: &mut Document, sheet: &mut Sheet, name: &str, v: Value, f: Option<&str>) {
    let key = RowKey::Named(doc.intern(name));
    let c = match f {
        Some(f) => with_f(doc, "Value", f, v),
        None => Cell::constant(names::VALUE, v, None),
    };
    put_row(sheet, names::USER, key, vec![c]);
}

// pin, size, the local pin in the middle, angle in degrees, flips
fn xform(doc: &mut Document, d: &mut ShapeData, pin: (f64, f64), size: (f64, f64), deg: f64, flip: (bool, bool)) {
    let cells = [
        ("PinX", pin.0),
        ("PinY", pin.1),
        ("Width", size.0),
        ("Height", size.1),
        ("LocPinX", size.0 / 2.0),
        ("LocPinY", size.1 / 2.0),
        ("Angle", deg * PI / 180.0),
        ("FlipX", if flip.0 { 1.0 } else { 0.0 }),
        ("FlipY", if flip.1 { 1.0 } else { 0.0 }),
    ];
    for (n, v) in cells {
        let c = num(doc, n, v);
        d.sheet.cells.push(c);
    }
}

fn user_addr(doc: &Document, name: &str) -> CellAddr {
    CellAddr::row(SecKey::one(names::USER), RowKey::Named(doc.sym(name).expect(name)), names::VALUE)
}

fn single(cell: Sym) -> CellAddr {
    CellAddr::single(cell)
}

// where evaluate_text pretends to evaluate (a user cell no sheet has; its axis is X)
fn probe() -> CellAddr {
    CellAddr::row(SecKey::one(names::USER), RowKey::Named(names::LABEL), names::VALUE)
}

fn eval_in(doc: &Document, sheet: SheetRef, text: &str) -> Val {
    match Engine::new().evaluate_text(doc, sheet, &probe(), text) {
        Ok(Outcome::Value(v)) => v,
        other => panic!("{text}: {other:?}"),
    }
}

fn number(doc: &Document, sheet: SheetRef, addr: &CellAddr) -> f64 {
    doc.resolve(sheet, addr).num().expect("a number")
}

// a page with shapes for the function tests
struct Fx {
    doc: Document,
    page: PageRef,
    // Sheet.1: 2 x 1 at (4, 3); User, Prop, Geometry, Connection and Character rows
    a: ShapeRef,
    // Sheet.2: 2 x 1 at (5, 5), turned by 90 degrees, a connection point at (0.5, 0.25)
    b: ShapeRef,
    // Sheet.3 "Some name": 2 x 1 at (1, 1), turned by 90 degrees and flipped in x
    c: ShapeRef,
    // Sheet.4: a group 2 x 2 at (6, 2), turned by 90 degrees
    g: ShapeRef,
    // Sheet.5: its subshape, 0.5 x 0.5 at (0.5, 0.5) in the group
    s: ShapeRef,
    // Sheet.6: a 1-D shape
    d: ShapeRef,
}

fn fx() -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    let mut ph = PageHeader { id: 0, ..PageHeader::default() };
    ph.name = Some(doc.intern("Zeichenblatt-1"));
    ph.name_u = Some(doc.intern("Page-1"));
    let mut pg = Page::new(ph);
    pg.sheet.cells.push(num(&mut doc, "PageScale", 1.0));
    pg.sheet.cells.push(num(&mut doc, "PageWidth", 8.5));
    let page = doc.insert_page(pg).unwrap();
    let lang = num(&mut doc, "DocLangID", 1033.0);
    doc.doc_sheet_mut().sheet.cells.push(lang);
    let mut st = Style::default();
    st.header.id = 1;
    st.header.name_u = Some(doc.intern("Normal"));
    st.header.name = Some(doc.intern("Standard"));
    st.sheet.cells.push(num(&mut doc, "LineWeight", 0.02));
    doc.insert_style(st).unwrap();

    let mut a = ShapeData::new(ShapeKind::Shape);
    xform(&mut doc, &mut a, (4.0, 3.0), (2.0, 1.0), 0.0, (false, false));
    a.text = Some(Box::new(Text::plain("Hallo")));
    user(&mut doc, &mut a.sheet, "Flag", Value::Num(0.0), None);
    user(&mut doc, &mut a.sheet, "Num", Value::Num(7.0), None);
    let hallo = doc.intern("Hallo");
    user(&mut doc, &mut a.sheet, "Txt", Value::Text(hallo), None);
    let (abc, at) = (doc.intern("abc"), doc.intern("@"));
    put_row(&mut a.sheet, names::PROPERTY, RowKey::Named(names::NAME), vec![
        Cell::constant(names::VALUE, Value::Text(abc), Some(Unit::Str)),
        Cell::constant(names::FORMAT, Value::Text(at), Some(Unit::Str)),
    ]);
    let mut geo = Section::new(SecKey::one(names::GEOMETRY));
    geo.cells.push(Cell::constant(names::NO_FILL, Value::Num(1.0), None));
    a.sheet.sections.push(geo);
    let xy = |x: f64, y: f64| vec![Cell::constant(names::X, Value::Num(x), None), Cell::constant(names::Y, Value::Num(y), None)];
    put_row(&mut a.sheet, names::GEOMETRY, RowKey::Index(1), xy(0.0, 0.0));
    put_row(&mut a.sheet, names::GEOMETRY, RowKey::Index(2), xy(2.0, 0.0));
    put_row(&mut a.sheet, names::CONNECTION, RowKey::Index(0), xy(0.5, 0.25));
    let size = doc.intern("Size");
    put_row(&mut a.sheet, names::CHARACTER, RowKey::Index(0), vec![Cell::constant(size, Value::Num(0.25), None)]);
    put_row(&mut a.sheet, names::CHARACTER, RowKey::Index(1), vec![Cell::constant(size, Value::Num(0.5), None)]);

    let mut b = ShapeData::new(ShapeKind::Shape);
    xform(&mut doc, &mut b, (5.0, 5.0), (2.0, 1.0), 90.0, (false, false));
    put_row(&mut b.sheet, names::CONNECTION, RowKey::Index(0), xy(0.5, 0.25));

    let mut c = ShapeData::new(ShapeKind::Shape);
    c.header.name_u = Some(doc.intern("Some name"));
    xform(&mut doc, &mut c, (1.0, 1.0), (2.0, 1.0), 90.0, (true, false));

    let mut g = ShapeData::new(ShapeKind::Group);
    xform(&mut doc, &mut g, (6.0, 2.0), (2.0, 2.0), 90.0, (false, false));
    let mut s = ShapeData::new(ShapeKind::Shape);
    xform(&mut doc, &mut s, (0.5, 0.5), (0.5, 0.5), 0.0, (false, false));

    let mut d = ShapeData::new(ShapeKind::Shape);
    for (n, v) in [("BeginX", 1.0), ("BeginY", 1.0), ("EndX", 3.0), ("EndY", 1.0), ("Width", 2.0)] {
        let cell = num(&mut doc, n, v);
        d.sheet.cells.push(cell);
    }

    let tree = &mut doc.page_mut(page).unwrap().shapes;
    let sa = tree.insert(None, 1, a).unwrap();
    let sb = tree.insert(None, 2, b).unwrap();
    let sc = tree.insert(None, 3, c).unwrap();
    let sg = tree.insert(None, 4, g).unwrap();
    let ss = tree.insert(Some(sg), 5, s).unwrap();
    let sd = tree.insert(None, 6, d).unwrap();
    let owner = Owner::Page(page);
    let r = |slot| ShapeRef { owner, slot };
    Fx { doc, page, a: r(sa), b: r(sb), c: r(sc), g: r(sg), s: r(ss), d: r(sd) }
}

impl Fx {
    fn at(&self, shape: ShapeRef, text: &str) -> Val {
        eval_in(&self.doc, SheetRef::Shape(shape), text)
    }

    fn v(&self, text: &str) -> Val {
        self.at(self.a, text)
    }

    fn n(&self, text: &str) -> f64 {
        match self.v(text).plain() {
            Val::Num(x) => *x,
            other => panic!("{text}: {other:?}"),
        }
    }

    fn s(&self, text: &str) -> String {
        match self.v(text) {
            Val::Str(s) => String::from(&*s),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn e(&self, text: &str) -> CellError {
        match self.v(text) {
            Val::Err(e) => e,
            other => panic!("{text}: {other:?}"),
        }
    }

    fn hex(&self, text: &str) -> String {
        match self.v(text).stored("Value") {
            Stored::Text(s) => s,
            other => panic!("{text}: {other:?}"),
        }
    }

    fn pt(&self, shape: ShapeRef, text: &str) -> (f64, f64) {
        match self.at(shape, text) {
            Val::Point(x, y) => (x, y),
            other => panic!("{text}: {other:?}"),
        }
    }
}

// one shape on a page with User rows (name, formula or "" for a constant, stored value)
fn one(rows: &[(&str, &str, f64)]) -> (Document, SheetRef) {
    let mut doc = Document::new(DocKind::Drawing);
    let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap();
    let mut d = ShapeData::new(ShapeKind::Shape);
    xform(&mut doc, &mut d, (1.0, 1.0), (2.0, 1.0), 0.0, (false, false));
    for &(n, f, v) in rows {
        user(&mut doc, &mut d.sheet, n, Value::Num(v), (!f.is_empty()).then_some(f));
    }
    let slot = doc.page_mut(page).unwrap().shapes.insert(None, 1, d).unwrap();
    (doc, SheetRef::Shape(ShapeRef { owner: Owner::Page(page), slot }))
}

// an Actions row with this formula (untracked) -> the address of its Action cell
fn add_action(doc: &mut Document, sheet: SheetRef, row: &str, f: &str) -> CellAddr {
    let key = RowKey::Named(doc.intern(row));
    let c = with_f(doc, "Action", f, Value::Num(0.0));
    let cell = c.name;
    put_row(doc.sheet_mut(sheet).unwrap(), names::ACTIONS, key, vec![c]);
    CellAddr::row(SecKey::one(names::ACTIONS), key, cell)
}

// two shapes with formulas whose stored values are right (except Dep, stored as `dep`):
// Sheet.1 Width 2, Height = Width*0.5, LocPinX = GUARD(Width*0.5), User.Num 7, User.Target 1,
// User.Proxy = SETATREF(User.Target), User.Tg = THEMEGUARD(5);
// Sheet.2 User.Dep = Sheet.1!Width*2, User.Dep2 = Sheet.1!User.Num+1
struct Tiny {
    doc: Document,
    a: SheetRef,
    b: SheetRef,
}

fn tiny(dep: f64) -> Tiny {
    let mut doc = Document::new(DocKind::Drawing);
    let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap();
    let mut a = ShapeData::new(ShapeKind::Shape);
    a.sheet.cells.push(num(&mut doc, "Width", 2.0));
    a.sheet.cells.push(with_f(&mut doc, "Height", "Width*0.5", Value::Num(1.0)));
    a.sheet.cells.push(with_f(&mut doc, "LocPinX", "GUARD(Width*0.5)", Value::Num(1.0)));
    user(&mut doc, &mut a.sheet, "Num", Value::Num(7.0), None);
    user(&mut doc, &mut a.sheet, "Target", Value::Num(1.0), None);
    user(&mut doc, &mut a.sheet, "Proxy", Value::Num(1.0), Some("SETATREF(User.Target)"));
    user(&mut doc, &mut a.sheet, "Tg", Value::Num(5.0), Some("THEMEGUARD(5)"));
    let mut b = ShapeData::new(ShapeKind::Shape);
    user(&mut doc, &mut b.sheet, "Dep", Value::Num(dep), Some("Sheet.1!Width*2"));
    user(&mut doc, &mut b.sheet, "Dep2", Value::Num(8.0), Some("Sheet.1!User.Num+1"));
    let tree = &mut doc.page_mut(page).unwrap().shapes;
    let sa = tree.insert(None, 1, a).unwrap();
    let sb = tree.insert(None, 2, b).unwrap();
    let owner = Owner::Page(page);
    Tiny { doc, a: SheetRef::Shape(ShapeRef { owner, slot: sa }), b: SheetRef::Shape(ShapeRef { owner, slot: sb }) }
}

// master 2 "Kasten"/"Box": a group master shape (ID 5: Width 1, Height = Width*0.5,
// User.Sub = Sheet.6!Width, User.Me = Sheet.5!Width) with a subshape (ID 6, Width 0.25); on the
// page an instance (ID 1, Width 3) with its subshape (ID 2, MasterShape 6, Width 0.75)
struct Mx {
    doc: Document,
    master: MasterRef,
    ms: SheetRef,
    inst: SheetRef,
}

fn mx() -> Mx {
    let mut doc = Document::new(DocKind::Drawing);
    let mut mh = MasterHeader { id: 2, ..MasterHeader::default() };
    mh.name = Some(doc.intern("Kasten"));
    mh.name_u = Some(doc.intern("Box"));
    let master = doc.insert_master(Master::new(mh)).unwrap();
    let mut ms = ShapeData::new(ShapeKind::Group);
    ms.sheet.cells.push(num(&mut doc, "Width", 1.0));
    ms.sheet.cells.push(with_f(&mut doc, "Height", "Width*0.5", Value::Num(0.5)));
    user(&mut doc, &mut ms.sheet, "Sub", Value::Num(0.25), Some("Sheet.6!Width"));
    user(&mut doc, &mut ms.sheet, "Me", Value::Num(1.0), Some("Sheet.5!Width"));
    let mut msub = ShapeData::new(ShapeKind::Shape);
    msub.sheet.cells.push(num(&mut doc, "Width", 0.25));
    let tree = &mut doc.master_mut(master).unwrap().shapes;
    let top = tree.insert(None, 5, ms).unwrap();
    tree.insert(Some(top), 6, msub).unwrap();

    let mut ph = PageHeader { id: 0, ..PageHeader::default() };
    ph.name_u = Some(doc.intern("Page-1"));
    let page = doc.insert_page(Page::new(ph)).unwrap();
    let mut inst = ShapeData::new(ShapeKind::Group);
    inst.header.master = Some(2);
    inst.sheet.cells.push(num(&mut doc, "Width", 3.0));
    let mut isub = ShapeData::new(ShapeKind::Shape);
    isub.header.master_shape = Some(6);
    isub.sheet.cells.push(num(&mut doc, "Width", 0.75));
    let tree = &mut doc.page_mut(page).unwrap().shapes;
    let i = tree.insert(None, 1, inst).unwrap();
    tree.insert(Some(i), 2, isub).unwrap();
    Mx {
        doc,
        master,
        ms: SheetRef::Shape(ShapeRef { owner: Owner::Master(master), slot: top }),
        inst: SheetRef::Shape(ShapeRef { owner: Owner::Page(page), slot: i }),
    }
}

// ==== 1. the parser

fn root(p: &Parsed) -> Node {
    p.nodes[p.root as usize]
}

fn node(p: &Parsed, i: u32) -> Node {
    p.nodes[i as usize]
}

fn constant(text: &str) -> f64 {
    parse(text).unwrap_or_else(|e| panic!("{text}: {e:?}")).constant().unwrap_or_else(|| panic!("{text}: no constant"))
}

#[test]
fn parses_numbers_with_units() {
    assert_near!(constant("10 mm"), 10.0 / 25.4);
    assert_near!(constant("10mm"), 10.0 / 25.4);
    assert_near!(constant("180DEG"), PI);
    assert_near!(constant("90 deg"), PI / 2.0);
    // DL converts like inches without a page
    assert_near!(constant("0.25DL"), 0.25);
    assert_near!(constant("1.5E-3IN"), 0.0015);
    assert_near!(constant("1e3"), 1000.0);
    assert_near!(constant(".5"), 0.5);
    assert_near!(constant("2 cm"), 2.0 / 2.54);
    // feet and inches add up
    assert_near!(constant("1FT 6IN"), 18.0);
    assert_near!(constant("-3"), -3.0);
    assert_near!(constant("12 pt"), 12.0 / 72.0);
    // 60% is the postfix operator: 0.6 when evaluated
    let p = parse("60%").unwrap();
    assert!(matches!(root(&p), Node::Pct(_)));
    assert_eq!(p.constant(), None);
    let f = fx();
    assert_near!(f.n("60%"), 0.6);
    assert_near!(f.n("(1+2)%"), 0.03);
    assert_near!(f.n("50%*2"), 1.0);
}

#[test]
fn parses_strings_booleans_errors() {
    let p = parse(r#""a""b""#).unwrap();
    assert_eq!(&*p.strs[0], "a\"b");
    let f = fx();
    assert_eq!(f.s(r#""a""b""#), "a\"b");
    assert_eq!(f.s(r#""""""#), "\"");
    assert_eq!(f.s(r#""""#), "");
}

#[test]
fn parses_booleans_and_error_literals() {
    let f = fx();
    assert_eq!(root(&parse("TRUE").unwrap()), Node::Bool(true));
    assert_eq!(root(&parse("false").unwrap()), Node::Bool(false));
    assert_near!(f.n("TRUE"), 1.0);
    assert_near!(f.n("FALSE"), 0.0);
    let errs = [
        ("#REF!", CellError::Ref),
        ("#DIV/0!", CellError::DivZero),
        ("#div/0!", CellError::DivZero),
        ("#VALUE!", CellError::Value),
        ("#NAME?", CellError::Name),
        ("#NUM!", CellError::Num),
        ("#N/A", CellError::NotAvailable),
        ("#DIM!", CellError::Dim),
    ];
    for (t, e) in errs {
        assert_eq!(root(&parse(t).unwrap()), Node::Err(e), "{t}");
        assert_eq!(f.e(t), e, "{t}");
    }
}

#[test]
fn parses_precedence_and_associativity() {
    // the tree: * binds tighter than +, comparisons loosest, & between them
    let p = parse("1+2*3").unwrap();
    let Node::Bin(BinOp::Add, l, r) = root(&p) else { panic!("{:?}", root(&p)) };
    assert_eq!(node(&p, l), Node::Num(1.0));
    assert!(matches!(node(&p, r), Node::Bin(BinOp::Mul, ..)));
    let p = parse("1=2&3").unwrap();
    assert!(matches!(root(&p), Node::Bin(BinOp::Eq, _, r) if matches!(node(&p, r), Node::Bin(BinOp::Concat, ..))));
    let p = parse("1&2+3").unwrap();
    assert!(matches!(root(&p), Node::Bin(BinOp::Concat, _, r) if matches!(node(&p, r), Node::Bin(BinOp::Add, ..))));
    // = and <> below the other comparisons
    let p = parse("1=2<3").unwrap();
    assert!(matches!(root(&p), Node::Bin(BinOp::Eq, _, r) if matches!(node(&p, r), Node::Bin(BinOp::Lt, ..))));
    // unary minus binds tighter than ^ ([MS-VSDX] 2.5.2: -2^2 is 4); everything left to right,
    // ^ too
    let p = parse("-2^2").unwrap();
    assert!(matches!(root(&p), Node::Bin(BinOp::Pow, l, _) if matches!(node(&p, l), Node::Neg(_))));
    let p = parse("2^3^2").unwrap();
    assert!(matches!(root(&p), Node::Bin(BinOp::Pow, l, r) if matches!(node(&p, l), Node::Bin(BinOp::Pow, ..)) && node(&p, r) == Node::Num(2.0)));
    let p = parse("10-4-3").unwrap();
    assert!(matches!(root(&p), Node::Bin(BinOp::Sub, l, _) if matches!(node(&p, l), Node::Bin(BinOp::Sub, ..))));
    // postfix % binds tightest
    let p = parse("2*3%").unwrap();
    assert!(matches!(root(&p), Node::Bin(BinOp::Mul, _, r) if matches!(node(&p, r), Node::Pct(_))));

}

#[test]
fn evaluates_by_precedence() {
    let f = fx();
    let cases = [
        ("1+2*3", 7.0),
        ("(1+2)*3", 9.0),
        ("((1))", 1.0),
        ("2^3^2", 64.0),
        ("2^(3^2)", 512.0),
        ("-2^2", 4.0),
        ("-(2^2)", -4.0),
        ("0-2^2", -4.0),
        ("2^-1", 0.5),
        ("2*-3", -6.0),
        ("+5", 5.0),
        ("10-4-3", 3.0),
        ("16/4/2", 2.0),
        ("1+1=2", 1.0),
        ("1=2<3", 1.0),
        ("0=2<3", 0.0),
        ("1<2", 1.0),
        ("2<=2", 1.0),
        ("3>=4", 0.0),
        ("3>4", 0.0),
        ("1<>2", 1.0),
        ("2=2.0", 1.0),
        (r#""a"&1="a1""#, 1.0),
        (r#""a"<"b""#, 1.0),
    ];
    for (t, want) in cases {
        assert_near!(f.n(t), want);
    }
    assert_eq!(f.s("1&2+3"), "15");
    assert_eq!(f.s(r#""x"&1.5"#), "x1.5");
}

fn one_ref(text: &str) -> CellRef {
    let p = parse(text).unwrap_or_else(|e| panic!("{text}: {e:?}"));
    assert_eq!(p.refs().len(), 1, "{text}");
    p.refs()[0].clone()
}

fn local(cell: CellPart) -> CellRef {
    CellRef { sheet: SheetName::Local, cell }
}

fn single_part(n: &str) -> CellPart {
    CellPart::Single(n.into())
}

fn indexed(section: &'static str, index: u32, row: u32, cell: &str) -> CellPart {
    CellPart::Indexed { section, index, row, cell: cell.into() }
}

fn named(section: &'static str, row: &str, cell: &str) -> CellPart {
    CellPart::Named { section, row: row.into(), cell: cell.into() }
}

#[test]
fn parses_reference_forms() {
    let cases: Vec<(&str, CellRef)> = vec![
        ("Width", local(single_part("Width"))),
        ("Geometry1.X2", local(indexed("Geometry", 0, 2, "X"))),
        ("Geometry2.Y1", local(indexed("Geometry", 1, 1, "Y"))),
        ("Geometry1.NoFill", local(CellPart::Section { section: "Geometry", index: 0, cell: "NoFill".into() })),
        ("Connections.X1", local(indexed("Connection", 0, 0, "X"))),
        ("Connections.Name.Y", local(named("Connection", "Name", "Y"))),
        ("Scratch.X1", local(indexed("Scratch", 0, 0, "X"))),
        ("Char.Size", local(indexed("Character", 0, 0, "Size"))),
        ("Char.Size[2]", local(indexed("Character", 0, 1, "Size"))),
        ("Para.HorzAlign", local(indexed("Paragraph", 0, 0, "HorzAlign"))),
        ("Prop.Name", local(named("Property", "Name", "Value"))),
        ("Prop.Name.Format", local(named("Property", "Name", "Format"))),
        ("User.Name", local(named("User", "Name", "Value"))),
        ("User.Name.Prompt", local(named("User", "Name", "Prompt"))),
        ("Controls.Row_1", local(named("Control", "Row_1", "X"))),
        ("Controls.Row_1.Y", local(named("Control", "Row_1", "Y"))),
        ("Actions.Row_1.Action", local(named("Actions", "Row_1", "Action"))),
        ("Reviewer.X1", local(CellPart::Unknown("Reviewer.X1".into()))),
        ("Sheet.5!Width", CellRef { sheet: SheetName::Id(5), cell: single_part("Width") }),
        ("ThePage!PageScale", CellRef { sheet: SheetName::ThePage, cell: single_part("PageScale") }),
        ("TheDoc!X", CellRef { sheet: SheetName::TheDoc, cell: single_part("X") }),
        ("TheDoc!DocLangID", CellRef { sheet: SheetName::TheDoc, cell: single_part("DocLangID") }),
        ("Pages[Page-1]!Sheet.3!Width", CellRef { sheet: SheetName::Page("Page-1".into(), Some(3)), cell: single_part("Width") }),
        ("Pages[Page-1]!PageWidth", CellRef { sheet: SheetName::Page("Page-1".into(), None), cell: single_part("PageWidth") }),
        ("Masters[M]!Width", CellRef { sheet: SheetName::Master("M".into(), None), cell: single_part("Width") }),
        ("Masters[Name]!Sheet.5!Width", CellRef { sheet: SheetName::Master("Name".into(), Some(5)), cell: single_part("Width") }),
        ("Styles[Normal]!LineColor", CellRef { sheet: SheetName::Style("Normal".into()), cell: single_part("LineColor") }),
        ("'Some name'!Width", CellRef { sheet: SheetName::Named("Some name".into()), cell: single_part("Width") }),
        ("Name.7!Width", CellRef { sheet: SheetName::Named("Name.7".into()), cell: single_part("Width") }),
        ("Sheet.5!Geometry1.X2", CellRef { sheet: SheetName::Id(5), cell: indexed("Geometry", 0, 2, "X") }),
        ("Sheet.5!Char.Size[3]", CellRef { sheet: SheetName::Id(5), cell: indexed("Character", 0, 2, "Size") }),
    ];
    for (t, want) in cases {
        assert_eq!(one_ref(t), want, "{t}");
    }
}

#[test]
fn parses_sheet_functions_and_bad_references() {
    // a function with a sheet in front: the reference names the sheet only
    let p = parse("Sheet.5!SheetRef()").unwrap();
    assert_eq!(p.refs(), &[CellRef { sheet: SheetName::Id(5), cell: CellPart::None }]);
    assert_eq!(p.functions().collect::<Vec<_>>(), vec![Func::SheetRef]);
    // forms that are no reference
    for t in ["Geometry.X1", "Geometry0.X1", "Sheet.x!Width", "User.A.B.C", "Char.Size[0]"] {
        assert_eq!(parse(t).err().map(|e| e.kind), Some(ParseErrorKind::BadReference), "{t}");
    }
}

#[test]
fn parses_calls_empty_arguments_unknown_functions() {
    let p = parse("SETATREF(User.X,,1)").unwrap();
    let Node::Call(f, start, n, sheet) = root(&p) else { panic!("{:?}", root(&p)) };
    assert_eq!((f, n, sheet), (Func::SetAtRef, 3, u32::MAX));
    let args: Vec<Node> = p.args[start as usize..(start + n) as usize].iter().map(|&i| node(&p, i)).collect();
    assert!(matches!(args[0], Node::Ref(_)));
    assert_eq!(args[1], Node::Missing);
    assert_eq!(args[2], Node::Num(1.0));
    let p = parse("MAX(1,)").unwrap();
    assert!(matches!(root(&p), Node::Call(Func::Max, _, 2, _)));
    let p = parse("PI()").unwrap();
    assert!(matches!(root(&p), Node::Call(Func::Pi, _, 0, _)));
    // names in any case
    assert!(parse("abs(-1)").unwrap().top_is(Func::Abs));
    assert!(parse("guard(1)").unwrap().top_is(Func::Guard));
    assert!(parse("IF(1,GUARD(1),2)").unwrap().calls(Func::Guard));
    assert!(!parse("IF(1,GUARD(1),2)").unwrap().top_is(Func::Guard));
    // unknown names keep their names
    let p = parse("FOO(1)+Bar()*ABS(2)").unwrap();
    let fs: Vec<Func> = p.functions().collect();
    assert_eq!(fs.len(), 3);
    assert!(matches!(fs[0], Func::Unknown(_)));
    assert_eq!(p.func_name(fs[0]), "FOO");
    assert_eq!(p.unknown().collect::<Vec<_>>(), vec!["FOO", "Bar"]);
    assert_eq!(p.func_name(Func::Abs), "ABS");
    assert_eq!(Func::by_name("themeval"), Some(Func::ThemeVal));
    assert_eq!(Func::by_name("NOPE"), None);
}

fn perr(text: &str) -> (usize, ParseErrorKind) {
    let e = parse(text).err().unwrap_or_else(|| panic!("{text} parsed"));
    (e.pos, e.kind)
}

#[test]
fn parse_errors_with_positions() {
    use ParseErrorKind::*;
    assert_eq!(perr("(1+2"), (0, Unclosed));
    assert_eq!(perr("1+(2"), (2, Unclosed));
    assert_eq!(perr("ABS(1"), (0, Unclosed));
    assert_eq!(perr("1+ABS(1"), (2, Unclosed));
    assert_eq!(perr(r#""abc"#), (0, UnclosedString));
    assert_eq!(perr(r#"1&"abc"#), (2, UnclosedString));
    assert_eq!(perr("'abc!Width").1, UnclosedString);
    assert_eq!(perr("1 2"), (2, Trailing));
    assert_eq!(perr("1)"), (1, Trailing));
    assert_eq!(perr("1 + $"), (4, BadChar));
    assert_eq!(perr("#WHAT"), (0, BadChar));
    assert_eq!(perr("1+"), (2, UnexpectedEnd));
    assert_eq!(perr(""), (0, UnexpectedEnd));
    assert_eq!(perr("   "), (3, UnexpectedEnd));
    assert_eq!(perr("1+*2"), (2, Unexpected));
    assert_eq!(perr("ABS(1 2)"), (6, Unexpected));
    assert_eq!(perr("ABS(1;2)"), (5, BadChar));
}

#[test]
fn parse_too_deep_nesting() {
    use ParseErrorKind::*;
    let deep = format!("{}1{}", "(".repeat(300), ")".repeat(300));
    assert_eq!(perr(&deep).1, TooComplex);
    let minus = format!("{}1", "-".repeat(300));
    assert_eq!(perr(&minus).1, TooComplex);
    // a nesting the limit allows
    let ok = format!("{}1{}", "(".repeat(100), ")".repeat(100));
    assert_eq!(parse(&ok).unwrap().constant(), Some(1.0));
}


#[test]
fn cell_part_and_unit_code() {
    assert_eq!(cell_part("Geometry3.A4", None), Some(indexed("Geometry", 2, 4, "A")));
    assert_eq!(cell_part("Char.Color", Some(3)), Some(indexed("Character", 0, 2, "Color")));
    assert_eq!(cell_part("PinX", None), Some(single_part("PinX")));
    assert_eq!(cell_part("PinX", Some(1)), None);
    assert_eq!(cell_part("Annotation.X1", None), Some(CellPart::Unknown("Annotation.X1".into())));
    assert_eq!(unit_code("mm"), Some(Unit::Millimeters));
    assert_eq!(unit_code("Deg"), Some(Unit::Degrees));
    assert_eq!(unit_code("DL"), Some(Unit::DrawingLength));
    assert_eq!(unit_code("xyz"), None);
    // letters after a number that are no unit: trailing text
    assert_eq!(parse("2 inX").err().map(|e| e.kind), Some(ParseErrorKind::Trailing));
}

// ==== 2. functions

#[test]
fn math_functions() {
    let f = fx();
    let cases = [
        ("ABS(-3)", 3.0),
        ("SQRT(16)", 4.0),
        ("SQRT(2)", core::f64::consts::SQRT_2),
        ("POW(2,10)", 1024.0),
        ("POW(2,0.5)", core::f64::consts::SQRT_2),
        ("2^0.5", core::f64::consts::SQRT_2),
        ("EXP(1)", core::f64::consts::E),
        ("LN(EXP(2))", 2.0),
        ("LN(10)", core::f64::consts::LN_10),
        ("LOG10(1000)", 3.0),
        ("SIN(PI()/6)", 0.5),
        ("SIN(30DEG)", 0.5),
        ("COS(PI()/3)", 0.5),
        ("COS(PI())", -1.0),
        ("TAN(PI()/4)", 1.0),
        ("ASIN(0.5)", PI / 6.0),
        ("ASIN(1)", PI / 2.0),
        ("ACOS(0.5)", PI / 3.0),
        ("ACOS(-1)", PI),
        ("ATAN(1)", PI / 4.0),
        ("SINH(1)", 1.1752011936438014),
        ("COSH(0)", 1.0),
        ("TANH(0)", 0.0),
        ("PI()", PI),
        ("MIN(3,1,2)", 1.0),
        ("MAX(3,1,2)", 3.0),
        ("SUM(1,2,3)", 6.0),
        ("MAGNITUDE(1,3,1,4)", 5.0),
        ("MAGNITUDE(2,3,1,4)", 52f64.sqrt_free()),
    ];
    for (t, want) in cases {
        assert_near!(f.n(t), want);
    }
}

#[test]
fn atan2_takes_y_first() {
    let f = fx();
    // ATAN2 takes y first
    assert_near!(f.n("ATAN2(1,0)"), PI / 2.0);
    assert_near!(f.n("ATAN2(-1,0)"), -PI / 2.0);
    assert_near!(f.n("ATAN2(0,-1)"), PI);
    assert_near!(f.n("ATAN2(1,2)"), 0.4636476090008061);
    assert_near!(f.n("ATAN2(2,1)"), 1.1071487177940904);
    assert_near!(f.n("ATAN2(-1,-1)"), -3.0 * PI / 4.0);
}

#[test]
fn math_domain_errors() {
    let f = fx();
    // out of the domain
    assert_eq!(f.e("SQRT(-1)"), CellError::Num);
    assert_eq!(f.e("LN(0)"), CellError::Num);
    assert_eq!(f.e("LOG10(-1)"), CellError::Num);
    assert_eq!(f.e("ASIN(2)"), CellError::Num);
    assert_eq!(f.e("ACOS(-1.5)"), CellError::Num);
}

// sqrt for the expectation without std
trait SqrtFree {
    fn sqrt_free(self) -> f64;
}

impl SqrtFree for f64 {
    fn sqrt_free(self) -> f64 {
        math::sqrt(self)
    }
}

#[test]
fn rounding_functions() {
    let f = fx();
    let cases = [
        ("ROUND(1.2345,2)", 1.23),
        ("ROUND(2.5)", 3.0),
        ("ROUND(-2.5)", -3.0),
        ("ROUND(1234.5,-2)", 1200.0),
        ("ROUND(2.4,0)", 2.0),
        ("TRUNC(1.789,1)", 1.7),
        ("TRUNC(-1.789)", -1.0),
        ("TRUNC(1789,-2)", 1700.0),
        ("INT(1.5)", 1.0),
        ("INT(-1.5)", -2.0),
        ("INTUP(1.2)", 2.0),
        ("INTUP(-1.2)", -1.0),
        ("INTUP(3)", 3.0),
        // FLOOR towards zero, CEILING away from zero, in multiples (the default multiple has the
        // sign of the number)
        ("FLOOR(7,3)", 6.0),
        ("CEILING(7,3)", 9.0),
        ("FLOOR(-7,-3)", -6.0),
        ("CEILING(-7,-3)", -9.0),
        ("FLOOR(-1.8)", -1.0),
        ("CEILING(-1.2)", -2.0),
        ("FLOOR(0,3)", 0.0),
        ("CEILING(5,0)", 0.0),
        ("FLOOR(6,3)", 6.0),
        ("CEILING(6,3)", 6.0),
        ("CEILING(1.2)", 2.0),
        ("FLOOR(1.8)", 1.0),
        ("FLOOR(1.3,0.5)", 1.0),
        ("CEILING(1.3,0.5)", 1.5),
        // MODULUS takes the sign of the divisor
        ("MODULUS(7,3)", 1.0),
        ("MODULUS(-7,3)", 2.0),
        ("MODULUS(7,-3)", -2.0),
        ("MODULUS(-7,-3)", -1.0),
        ("MODULUS(7.5,2)", 1.5),
        // SIGN with fuzz
        ("SIGN(-2)", -1.0),
        ("SIGN(5)", 1.0),
        ("SIGN(0)", 0.0),
        ("SIGN(0.001,0.01)", 0.0),
        ("SIGN(-0.5,0.1)", -1.0),
        // angles
        ("ANG360(-90DEG)", 1.5 * PI),
        ("ANG360(450DEG)", 0.5 * PI),
        ("ANG360(PI())", PI),
        ("DEG(PI())", 180.0),
        ("DEG(PI()/2)", 90.0),
        // bits
        ("BITAND(12,10)", 8.0),
        ("BITOR(12,10)", 14.0),
        ("BITXOR(12,10)", 6.0),
        ("BITNOT(0)", -1.0),
        ("BITNOT(255)", -256.0),
    ];
    for (t, want) in cases {
        assert_near!(f.n(t), want);
    }
    assert_eq!(f.e("MODULUS(5,0)"), CellError::DivZero);
    // a multiple of the other sign
    assert_eq!(f.e("FLOOR(-7,3)"), CellError::Num);
    assert_eq!(f.e("CEILING(7,-3)"), CellError::Num);
}

#[test]
fn logic_and_errors() {
    let f = fx();
    let cases = [
        ("IF(1>2,10,20)", 20.0),
        ("IF(2>1,10,20)", 10.0),
        ("IF(TRUE,5)", 5.0),
        ("IF(FALSE,5)", 0.0),
        // lazy: the branch not taken is not evaluated
        ("IF(TRUE,1,1/0)", 1.0),
        ("IF(FALSE,1/0,2)", 2.0),
        ("AND(1,1)", 1.0),
        ("AND(1,0)", 0.0),
        ("AND(1,2,3)", 1.0),
        ("OR(0,0)", 0.0),
        ("OR(0,1)", 1.0),
        ("NOT(0)", 1.0),
        ("NOT(5)", 0.0),
        ("ISERR(1/0)", 1.0),
        ("ISERR(#N/A)", 0.0),
        ("ISERR(1)", 0.0),
        ("ISERROR(#N/A)", 1.0),
        ("ISERROR(SQRT(-1))", 1.0),
        ("ISERROR(1)", 0.0),
        ("ISERRNA(#N/A)", 1.0),
        ("ISERRNA(#REF!)", 0.0),
        ("ISERRVALUE(#VALUE!)", 1.0),
        ("ISERRVALUE(1/0)", 0.0),
        ("ISERRVALUE(\"x\"+1)", 1.0),
    ];
    for (t, want) in cases {
        assert_near!(f.n(t), want);
    }
}

#[test]
fn division_by_zero_and_error_propagation() {
    let f = fx();
    // division by zero and propagation through operators and functions
    assert_eq!(f.e("1/0"), CellError::DivZero);
    assert_eq!(f.e("1/0+1"), CellError::DivZero);
    assert_eq!(f.e("2*(1/0)"), CellError::DivZero);
    assert_eq!(f.e("ABS(1/0)"), CellError::DivZero);
    assert_eq!(f.e("IF(1/0,1,2)"), CellError::DivZero);
    assert_eq!(f.e("#REF!+1"), CellError::Ref);
    assert_eq!(f.e("MAX(1,#N/A)"), CellError::NotAvailable);
    assert_eq!(f.e("SQRT(-1)*0"), CellError::Num);
    assert_eq!(f.e("\"x\"&(1/0)"), CellError::DivZero);
    assert_eq!(f.e("\"abc\"+1"), CellError::Value);
    assert_eq!(f.e("POW(0,-1)"), CellError::Num);
}

#[test]
fn index_and_lookup() {
    let f = fx();
    assert_eq!(f.s(r#"INDEX(0,"a;b;c")"#), "a");
    assert_eq!(f.s(r#"INDEX(1,"a;b;c")"#), "b");
    assert_eq!(f.s(r#"INDEX(2.9,"a;b;c")"#), "c");
    // out of range: empty, or the error value argument
    assert_eq!(f.s(r#"INDEX(5,"a;b;c")"#), "");
    assert_eq!(f.s(r#"INDEX(-1,"a;b;c")"#), "");
    assert_eq!(f.s(r#"INDEX(5,"a;b;c",";","none")"#), "none");
    assert_eq!(f.s(r#"INDEX(5,"a;b;c",,"none")"#), "none");
    assert_eq!(f.e(r#"INDEX(5,"a;b;c",";",#N/A)"#), CellError::NotAvailable);
    // a custom delimiter
    assert_eq!(f.s(r#"INDEX(1,"x|y|z","|")"#), "y");
    assert_eq!(f.s(r#"INDEX(1,"x, y, z",", ")"#), "y");
    // an error in an argument
    assert_eq!(f.e(r#"INDEX(1/0,"a;b")"#), CellError::DivZero);
    // LOOKUP: the position, -1 when missing
    assert_near!(f.n(r#"LOOKUP("b","a;b;c")"#), 1.0);
    assert_near!(f.n(r#"LOOKUP("a","a;b;c")"#), 0.0);
    assert_near!(f.n(r#"LOOKUP("x","a;b;c")"#), -1.0);
    assert_near!(f.n(r#"LOOKUP("z","x|y|z","|")"#), 2.0);
}

#[test]
fn text_functions() {
    let f = fx();
    assert_near!(f.n(r#"STRSAME("abc","abc")"#), 1.0);
    assert_near!(f.n(r#"STRSAME("abc","ABC")"#), 0.0);
    assert_near!(f.n(r#"STRSAME("abc","ABC",TRUE)"#), 1.0);
    assert_near!(f.n(r#"STRSAME("abc","ABC",FALSE)"#), 0.0);
    assert_near!(f.n(r#"LEN("Hallo")"#), 5.0);
    assert_near!(f.n(r#"LEN("")"#), 0.0);
    assert_near!(f.n("LEN(\"äöü\")"), 3.0);
    assert_near!(f.n("LEN(User.Txt)"), 5.0);
    assert_eq!(f.s(r#"LEFT("Hallo",2)"#), "Ha");
    assert_eq!(f.s(r#"LEFT("Hallo")"#), "H");
    assert_eq!(f.s(r#"RIGHT("Hallo",3)"#), "llo");
    assert_eq!(f.s(r#"RIGHT("ab",5)"#), "ab");
    assert_eq!(f.s(r#"MID("Hallo",2,3)"#), "all");
    assert_eq!(f.s(r#"MID("Hallo",4,10)"#), "lo");
}

#[test]
fn find_function() {
    let f = fx();
    assert_near!(f.n(r#"FIND("l","Hallo")"#), 3.0);
    assert_near!(f.n(r#"FIND("l","Hallo",4)"#), 4.0);
    assert_near!(f.n(r#"FIND("lo","Hallo")"#), 4.0);
    assert_near!(f.n(r#"FIND("L","Hallo",1,TRUE)"#), 3.0);
    assert_eq!(f.e(r#"FIND("z","Hallo")"#), CellError::Value);
    assert_eq!(f.e(r#"FIND("L","Hallo")"#), CellError::Value);
}

#[test]
fn substitute_replace_rept() {
    let f = fx();
    assert_eq!(f.s(r#"SUBSTITUTE("a-b-c","-","+")"#), "a+b+c");
    assert_eq!(f.s(r#"SUBSTITUTE("a-b-c","-","+",2)"#), "a-b+c");
    assert_eq!(f.s(r#"SUBSTITUTE("a-b-c","-","+",1)"#), "a+b-c");
    assert_eq!(f.s(r#"SUBSTITUTE("a-b-c","x","+")"#), "a-b-c");
    assert_eq!(f.s(r#"REPLACE("abcdef",2,3,"X")"#), "aXef");
    assert_eq!(f.s(r#"REPLACE("abc",4,0,"d")"#), "abcd");
    assert_eq!(f.s(r#"REPT("ab",3)"#), "ababab");
    assert_eq!(f.s(r#"REPT("ab",0)"#), "");
    assert_eq!(f.e(r#"REPT("ab",-1)"#), CellError::Value);
}

#[test]
fn case_trim_char_concat() {
    let f = fx();
    assert_eq!(f.s(r#"LOWER("AbC")"#), "abc");
    assert_eq!(f.s(r#"UPPER("aBc")"#), "ABC");
    assert_eq!(f.s(r#"TRIM("  a   b  ")"#), "a b");
    assert_eq!(f.s("CHAR(65)"), "A");
    assert_eq!(f.s("CHAR(228)"), "ä");
    assert_eq!(f.s(r#""a"&"b""#), "ab");
    assert_eq!(f.s("1&2"), "12");
    assert_eq!(f.s(r#"User.Txt&"!""#), "Hallo!");
}

#[test]
fn colour_functions() {
    let f = fx();
    // RGB truncates, takes the low byte past 255 ([MS-VSDX] 2.5.3.130), negatives are 0
    assert_eq!(f.v("RGB(255.9,-5,128.4)"), Val::Color(Color::Rgb(255, 0, 128)));
    assert_eq!(f.hex("RGB(255.9,-5,128.4)"), "#ff0080");
    assert_eq!(f.v("RGB(300,256,0)"), Val::Color(Color::Rgb(44, 0, 0)));
    assert_eq!(f.hex("RGB(10,20,30)"), "#0a141e");
}

#[test]
fn hsl_hue_sat_lum() {
    let f = fx();
    // HSL on 0..240
    assert_eq!(f.hex("HSL(0,240,120)"), "#ff0000");
    assert_eq!(f.hex("HSL(80,240,120)"), "#00ff00");
    assert_eq!(f.hex("HSL(160,240,120)"), "#0000ff");
    assert_eq!(f.hex("HSL(160,0,120)"), "#808080");
    assert_eq!(f.hex("HSL(0,0,240)"), "#ffffff");
    assert_near!(f.n("HUE(RGB(255,0,0))"), 0.0);
    assert_near!(f.n("SAT(RGB(255,0,0))"), 240.0);
    assert_near!(f.n("LUM(RGB(255,0,0))"), 120.0);
    assert_near!(f.n("HUE(RGB(0,255,0))"), 80.0);
    assert_near!(f.n("HUE(RGB(0,0,255))"), 160.0);
    // a grey has no saturation (and the undefined hue 160)
    assert_near!(f.n("HUE(RGB(128,128,128))"), 160.0);
    assert_near!(f.n("SAT(RGB(128,128,128))"), 0.0);
    assert_near!(f.n("LUM(RGB(128,128,128))"), 120.0);
    assert_near!(f.n("RED(RGB(10,20,30))"), 10.0);
    assert_near!(f.n("GREEN(RGB(10,20,30))"), 20.0);
    assert_near!(f.n("BLUE(RGB(10,20,30))"), 30.0);
}

#[test]
fn colour_indices_and_lumdiff() {
    let f = fx();
    // colour indices through the default table
    assert_near!(f.n("RED(2)"), 255.0);
    assert_near!(f.n("GREEN(2)"), 0.0);
    assert_near!(f.n("GREEN(3)"), 255.0);
    assert_near!(f.n("BLUE(4)"), 255.0);
    assert_near!(f.n("RED(19)"), 128.0);
    assert_near!(f.n("LUM(1)"), 240.0);
    assert_near!(f.n("LUMDIFF(RGB(255,255,255),RGB(0,0,0))"), 240.0);
    assert_near!(f.n("LUMDIFF(RGB(0,0,0),RGB(255,255,255))"), -240.0);
}

#[test]
fn tint_shade_msotint_blend() {
    let f = fx();
    // TINT and SHADE move the luminosity
    assert_eq!(f.hex("TINT(RGB(128,128,128),40)"), "#aaaaaa");
    assert_eq!(f.hex("SHADE(RGB(128,128,128),40)"), "#555555");
    // Office's tint in per cent
    assert_eq!(f.hex("MSOTINT(RGB(255,255,255),-25)"), "#bfbfbf");
    assert_eq!(f.hex("MSOTINT(RGB(0,0,0),35)"), "#595959");
    assert_eq!(f.hex("MSOSHADE(RGB(255,255,255),25)"), "#bfbfbf");
    // BLEND mixes in part of the second colour
    assert_eq!(f.hex("BLEND(RGB(0,0,0),RGB(200,100,50),0.5)"), "#643219");
    assert_eq!(f.hex("BLEND(RGB(10,20,30),RGB(200,100,50),0)"), "#0a141e");
    assert_eq!(f.hex("BLEND(RGB(10,20,30),RGB(200,100,50),1)"), "#c86432");
}

#[test]
fn colour_index_from_the_document_table() {
    let mut f = fx();
    let rgb = f.doc.intern("#123456");
    f.doc.colors.push(ColorEntry { ix: 2, rgb });
    assert_near!(f.n("RED(2)"), 18.0);
    assert_near!(f.n("BLUE(2)"), 86.0);
    // other indices keep the defaults
    assert_near!(f.n("BLUE(4)"), 255.0);
}

#[test]
fn nurbs_and_polyline_texts() {
    let f = fx();
    let shape = |t: &str| match f.v(t) {
        Val::Shape(s) => String::from(&*s),
        other => panic!("{t}: {other:?}"),
    };
    assert_eq!(shape("POLYLINE(0,0,1,2,3,4)"), "POLYLINE(0, 0, 1,2, 3,4)");
    assert_eq!(shape("POLYLINE(0,0,0.5,1)"), "POLYLINE(0, 0, 0.5,1)");
    assert_eq!(shape("NURBS(10,3,0,0,1,2,0,1,3,4,1,1)"), "NURBS(10, 3, 0, 0, 1,2,0,1, 3,4,1,1)");
    assert_eq!(f.v("POLYLINE(0,0,1,2)").stored("Geometry"), Stored::Text(String::from("POLYLINE(0, 0, 1,2)")));
    // incomplete groups
    assert_eq!(f.e("POLYLINE(0,0,1)"), CellError::Value);
    assert_eq!(f.e("NURBS(10,3,0,0,1,2)"), CellError::Value);
}

#[test]
fn bound_function() {
    let f = fx();
    // BOUND(value, type, ignore1, a1, b1 [, ignore2, a2, b2 ...])
    let cases = [
        // inside: the value
        ("BOUND(5,0,FALSE,0,10)", 5.0),
        // outside every range: the nearest end, the first on a tie
        ("BOUND(15,0,FALSE,0,10)", 10.0),
        ("BOUND(-3,0,FALSE,0,10)", 0.0),
        ("BOUND(12,0,FALSE,0,10,FALSE,20,30)", 10.0),
        ("BOUND(18,0,FALSE,0,10,FALSE,20,30)", 20.0),
        ("BOUND(25,0,FALSE,0,10,FALSE,20,30)", 25.0),
        ("BOUND(15,0,FALSE,0,10,FALSE,20,30)", 10.0),
        // the ignore flags: an ignored range does not count
        ("BOUND(12,0,FALSE,0,10,TRUE,11,13)", 10.0),
        ("BOUND(12,0,FALSE,0,10,FALSE,11,13)", 12.0),
        ("BOUND(15,0,TRUE,0,10)", 15.0),
        // exclusive: inside goes to the nearest end, outside stays
        ("BOUND(3,1,FALSE,0,10)", 0.0),
        ("BOUND(8,1,FALSE,0,10)", 10.0),
        ("BOUND(15,1,FALSE,0,10)", 15.0),
        // type 2 leaves the value alone
        ("BOUND(15,2,FALSE,0,10)", 15.0),
        // a range written backwards
        ("BOUND(15,0,FALSE,10,0)", 10.0),
    ];
    for (t, want) in cases {
        assert_near!(f.n(t), want);
    }
    assert_eq!(f.e("BOUND(1,0,FALSE,0)"), CellError::Value);
    assert_eq!(f.e("BOUND(1,0,FALSE,0,10,FALSE)"), CellError::Value);
    assert_eq!(f.e("BOUND(1,3,FALSE,0,10)"), CellError::Value);
}

#[test]
fn points_and_information_functions() {
    let f = fx();
    assert_eq!(f.v("PNT(1,2)"), Val::Point(1.0, 2.0));
    assert_near!(f.n("PNTX(PNT(1,2))"), 1.0);
    assert_near!(f.n("PNTY(PNT(1,2))"), 2.0);
    assert_eq!(f.v("PNT(1,2)+PNT(3,4)"), Val::Point(4.0, 6.0));
    assert_eq!(f.v("PNT(3,4)-PNT(1,2)"), Val::Point(2.0, 2.0));
    assert_eq!(f.v("-PNT(1,2)"), Val::Point(-1.0, -2.0));
}

#[test]
fn pass_through_functions() {
    let f = fx();
    // GUARD and THEMEGUARD pass their value through
    assert_near!(f.n("GUARD(5)"), 5.0);
    assert_near!(f.n("GUARD(Width*2)"), 4.0);
    assert_near!(f.n("THEMEGUARD(Width)"), 2.0);
    assert_near!(f.n("DEPENDSON(Width,Height)"), 0.0);
    assert_near!(f.n(r#"ARG("x")"#), 0.0);
    assert_near!(f.n(r#"ARG("x",3)"#), 3.0);
    assert_near!(f.n("USERUI(0,5,6)"), 5.0);
    assert_eq!(f.e("REF()"), CellError::Ref);
}

#[test]
fn shape_and_page_information() {
    let f = fx();
    assert_eq!(f.s("MASTERNAME()"), "<no master>");
    assert_near!(f.n("ID()"), 1.0);
    assert_eq!(f.at(f.b, "ID()"), Val::Num(2.0));
    assert_eq!(f.at(f.s, "ID()"), Val::Num(5.0));
    assert_eq!(eval_in(&f.doc, SheetRef::Page(f.page), "ID()"), Val::Num(0.0));
    assert_eq!(f.s("PAGENAME()"), "Zeichenblatt-1");
    assert_eq!(f.s("PAGENAME(750)"), "Page-1");
    assert_near!(f.n("PAGECOUNT()"), 1.0);
    assert_near!(f.n("PAGENUMBER()"), 1.0);
    assert_eq!(f.s("NAME()"), "Sheet.1");
    assert_eq!(f.at(f.c, "NAME(750)"), Val::str("Some name"));
    assert_eq!(f.s("TYPEDESC()"), "Shape");
    assert_near!(f.n("DEFAULTEVENT()"), 0.0);
    assert_near!(f.n("IS1D()"), 0.0);
    assert_eq!(f.at(f.d, "IS1D()"), Val::Num(1.0));
    assert_near!(f.n("TYPE()"), 3.0);
    assert_eq!(f.at(f.g, "TYPE()"), Val::Num(2.0));
}

#[test]
fn themeval_and_references_as_values() {
    let f = fx();
    // THEMEVAL() in a cell with a theme meaning is a themed value (theme.rs is provisional:
    // nothing more here)
    let o = Engine::new().evaluate_text(&f.doc, SheetRef::Shape(f.a), &single(names::LINE_WEIGHT), "THEMEVAL()");
    assert!(matches!(o, Ok(Outcome::Value(Val::Themed(_)))), "{o:?}");
    // references as values
    assert_eq!(
        f.v("GETREF(Width)"),
        Val::Target(Target { sheet: SheetRef::Shape(f.a), addr: Some(single(names::WIDTH)) })
    );
    assert_eq!(f.v("Sheet.2!SheetRef()"), Val::Target(Target { sheet: SheetRef::Shape(f.b), addr: None }));
}

#[test]
fn references_read_their_cells() {
    let f = fx();
    let cases = [
        ("Width", 2.0),
        ("PinX", 4.0),
        ("Geometry1.X2", 2.0),
        ("Geometry1.Y1", 0.0),
        ("Geometry1.NoFill", 1.0),
        ("Connections.X1", 0.5),
        ("Connections.Y1", 0.25),
        ("Char.Size", 0.25),
        ("Char.Size[2]", 0.5),
        ("User.Num", 7.0),
        ("User.Num.Value", 7.0),
        ("Sheet.2!Width", 2.0),
        ("Sheet.2!Angle", PI / 2.0),
        ("ThePage!PageWidth", 8.5),
        ("ThePage!PageScale", 1.0),
        ("TheDoc!DocLangID", 1033.0),
        ("Pages[Page-1]!Sheet.2!PinX", 5.0),
        ("Pages[Zeichenblatt-1]!PageWidth", 8.5),
        ("Styles[Normal]!LineWeight", 0.02),
        ("Styles[Standard]!LineWeight", 0.02),
        ("'Some name'!PinX", 1.0),
        ("Sheet.5!PinX", 0.5),
    ];
    for (t, want) in cases {
        assert_near!(f.n(t), want);
    }
    assert_eq!(f.s("Prop.Name"), "abc");
    assert_eq!(f.s("Prop.Name.Format"), "@");
    assert_eq!(f.s("User.Txt"), "Hallo");
    assert_eq!(f.e("Sheet.9!Width"), CellError::Ref);
    assert_eq!(f.e("Pages[Nope]!PageWidth"), CellError::Ref);
    // a subshape sees its group's shapes by ID, and ThePage
    assert_eq!(f.at(f.s, "Sheet.1!Width"), Val::Num(2.0));
    assert_eq!(f.at(f.c, "ID()"), Val::Num(3.0));
    assert_eq!(f.at(f.s, "ThePage!PageWidth"), Val::Num(8.5));
}


#[test]
fn text_of_values_and_comparisons() {
    let f = fx();
    assert_eq!(f.s(r#"0.1&"""#), "0.1");
    assert_eq!(f.s(r#"-2&"""#), "-2");
    assert_eq!(f.s(r#"RGB(255,0,0)&"""#), "#ff0000");
    assert_eq!(f.s(r#"PNT(1,2.5)&"""#), "1, 2.5");
    assert_eq!(f.s(r#"TRUE&"""#), "1");
    // colours by value, strings exactly, numbers within a tiny tolerance
    assert_near!(f.n("RGB(1,2,3)=RGB(1,2,3)"), 1.0);
    assert_near!(f.n("RGB(1,2,3)=RGB(1,2,4)"), 0.0);
    assert_near!(f.n(r#""a"="A""#), 0.0);
    assert_near!(f.n(r#""10"=10"#), 1.0);
    assert_near!(f.n("0.1+0.2=0.3"), 1.0);
}

#[test]
fn shape_text_and_text_sizes() {
    let f = fx();
    assert_eq!(f.s("SHAPETEXT(TheText)"), "Hallo");
    assert_eq!(f.at(f.b, "SHAPETEXT(TheText)"), Val::str(""));
    // without the app's fonts the Estimate measures (Char.Size 0.25, no margins); lines of
    // 1.2 em, and a narrow width breaks the line
    let (w, h1) = Estimate.measure("Hallo", "", 0.25, None);
    assert_near!(h1, 0.3);
    assert_near!(f.n("TEXTWIDTH(TheText)"), w);
    assert_near!(f.n("TEXTHEIGHT(TheText,10)"), 0.3);
    let narrow = w / 2.5;
    let wrapped = f.n(&format!("TEXTHEIGHT(TheText,{narrow})"));
    assert_near!(wrapped, Estimate.measure("Hallo", "", 0.25, Some(narrow)).1);
    assert_near!(wrapped, 0.9);
    // an empty text: no width, one line of the default size (12 pt)
    assert_eq!(f.at(f.b, "TEXTWIDTH(TheText)"), Val::Num(0.0));
    let h = match f.at(f.b, "TEXTHEIGHT(TheText,1)") {
        Val::Num(x) => x,
        v => panic!("{v:?}"),
    };
    assert_near!(h, 0.2);
    // two paragraphs: two lines, as wide as the wider one
    let (w2, h2) = Estimate.measure("ab\ncde", "", 1.0, None);
    assert_near!(h2, 2.4);
    assert_near!(w2, Estimate.measure("cde", "", 1.0, None).0);
}

struct Fixed;

impl TextMetrics for Fixed {
    fn measure(&self, text: &str, _font: &str, size: f64, _max: Option<f64>) -> (f64, f64) {
        (text.chars().count() as f64 * 0.1, size)
    }
}

#[test]
fn text_sizes_with_the_apps_metrics() {
    let f = fx();
    let mut eng = Engine::with_metrics(Rc::new(Fixed));
    let o = eng.evaluate_text(&f.doc, SheetRef::Shape(f.a), &probe(), "TEXTWIDTH(TheText)");
    assert!(matches!(o, Ok(Outcome::Value(Val::Num(x))) if near(x, 0.5)), "{o:?}");
    let o = eng.evaluate_text(&f.doc, SheetRef::Shape(f.a), &probe(), "TEXTHEIGHT(TheText,5)");
    assert!(matches!(o, Ok(Outcome::Value(Val::Num(x))) if near(x, 0.25)), "{o:?}");
}

#[test]
fn not_computed_functions_are_kept() {
    let (mut doc, sh) = one(&[]);
    let cases = [
        (r#"FORMAT(1,"0.0")"#, Keep::NotComputed("FORMAT")),
        ("_WALKGLUE(1)", Keep::NotComputed("_WALKGLUE")),
        ("RAND()", Keep::NotComputed("RAND")),
        (r#"FONT("Calibri")"#, Keep::NotComputed("FONT")),
        ("Reviewer.X1", Keep::Reference),
        ("Reviewer.X1+FOO()", Keep::Unknown(String::from("FOO"))),
    ];
    for (t, k) in cases {
        assert_eq!(Engine::new().evaluate_text(&doc, sh, &probe(), t), Ok(Outcome::Kept(k)), "{t}");
    }
    // in a font cell FONT gives the name
    let font = single(doc.intern("Font"));
    assert_eq!(
        Engine::new().evaluate_text(&doc, sh, &font, r#"FONT("Calibri")"#),
        Ok(Outcome::Value(Val::Font(Rc::from("Calibri"))))
    );
}

#[test]
fn colour_helpers() {
    assert_eq!(rgb_to_hsl(255, 0, 0), (0, 240, 120));
    assert_eq!(hsl_to_rgb(0, 240, 120), (255, 0, 0));
    assert_eq!(rgb_to_hsl(255, 255, 255), (160, 0, 240));
    assert_eq!(hsl_to_rgb(160, 0, 0), (0, 0, 0));
    assert_eq!(Color::parse("#4D1e1A"), Some(Color::Rgb(0x4d, 0x1e, 0x1a)));
    assert_eq!(Color::parse("#4d1e1"), None);
    assert_eq!(Color::parse("4d1e1a"), None);
    assert_eq!(Color::hex(0xab, 0, 0x0f), "#ab000f");
}

// ==== 3. points and coordinates

fn assert_pt(p: (f64, f64), want: (f64, f64)) {
    assert!(near(p.0, want.0) && near(p.1, want.1), "{p:?} is not {want:?}");
}

#[test]
fn par_and_loc_of_sibling_points() {
    let f = fx();
    // Sheet.2 turned by 90 degrees: (2, 1) local -> pin + R(90) (local - locpin)
    assert_pt(f.pt(f.a, "PAR(PNT(Sheet.2!Width,Sheet.2!Height))"), (4.5, 6.0));
    // its connection point (0.5, 0.25)
    assert_pt(f.pt(f.a, "PAR(PNT(Sheet.2!Connections.X1,Sheet.2!Connections.Y1))"), (5.25, 4.5));
    // Sheet.3 flipped in x, then turned by 90 degrees
    assert_pt(f.pt(f.a, "PAR(PNT(Sheet.3!Width,Sheet.3!Height))"), (0.5, 0.0));
    assert_pt(f.pt(f.a, "PAR(PNT(Sheet.3!LocPinX,Sheet.3!LocPinY))"), (1.0, 1.0));
    // the own point in page coordinates: Sheet.1 is not turned
    assert_pt(f.pt(f.a, "PAR(PNT(Width,Height))"), (5.0, 3.5));
    assert_pt(f.pt(f.a, "PAR(PNT(0,0))"), (3.0, 2.5));
}

#[test]
fn loc_of_sibling_points() {
    let f = fx();
    // LOC: into the evaluating shape's own coordinates
    assert_pt(f.pt(f.a, "LOC(PNT(Sheet.2!Width,Sheet.2!Height))"), (1.5, 3.5));
    assert_pt(f.pt(f.a, "LOC(PNT(Width,Height))"), (2.0, 1.0));
    // in Sheet.2's own coordinates a page point turns back
    assert_pt(f.pt(f.b, "LOC(PNT(Sheet.1!LocPinX,Sheet.1!LocPinY))"), (-1.0, 1.5));
}

#[test]
fn points_of_a_subshape_in_a_turned_group() {
    let f = fx();
    // Sheet.5 (in the group Sheet.4, turned by 90 degrees about (6, 2)) in page coordinates
    assert_pt(f.pt(f.a, "PAR(PNT(Sheet.5!Width,Sheet.5!Height))"), (6.25, 1.75));
    assert_pt(f.pt(f.a, "PAR(PNT(Sheet.5!LocPinX,Sheet.5!LocPinY))"), (6.5, 1.5));
    // seen from the subshape, PAR is the group's coordinates
    assert_pt(f.pt(f.s, "PAR(PNT(Width,Height))"), (0.75, 0.75));
    assert_pt(f.pt(f.s, "LOC(PNT(Width,Height))"), (0.5, 0.5));
    // a page point (Sheet.1's pin) in the subshape's coordinates
    assert_pt(f.pt(f.s, "LOC(PNT(Sheet.1!LocPinX,Sheet.1!LocPinY))"), (1.75, 2.75));
}


#[test]
fn loctoloc_loctopar_and_angles() {
    let f = fx();
    // a point of Sheet.2 in Sheet.1's coordinates
    assert_pt(f.pt(f.a, "LOCTOLOC(PNT(Sheet.2!LocPinX,Sheet.2!LocPinY),Sheet.2!Width,Sheet.1!Width)"), (2.0, 2.5));
    // a point of the subshape in the coordinates of its parent, the turned group
    assert_pt(f.pt(f.a, "LOCTOPAR(PNT(Sheet.5!LocPinX,Sheet.5!LocPinY),Sheet.5!Width,Sheet.5!Width)"), (0.5, 0.5));
    // angles: Sheet.2 is turned by 90 degrees on the page
    let num_at = |s: ShapeRef, t: &str| match f.at(s, t) {
        Val::Num(x) => x,
        v => panic!("{t}: {v:?}"),
    };
    assert_near!(num_at(f.b, "ANGLETOPAR(0)"), PI / 2.0);
    assert_near!(num_at(f.b, "ANGLETOPAR(10DEG)"), 100.0 * PI / 180.0);
    assert_near!(num_at(f.b, "ANGLETOLOC(0)"), -PI / 2.0);
    assert_near!(num_at(f.a, "ANGLETOPAR(0,Sheet.5!Width,Sheet.1!Width)"), PI / 2.0);
}

// ==== 4. the engine on documents

#[test]
fn calculate_writes_nothing_and_write_only_differs() {
    let mut t = tiny(99.0);
    let dep = user_addr(&t.doc, "Dep");
    let height = single(names::HEIGHT);
    let mut eng = Engine::new();
    let res = eng.calculate(&t.doc);
    assert_eq!(res.get(t.b, &dep), Some(&Outcome::Value(Val::Num(4.0))));
    assert_eq!(res.get(t.a, &height), Some(&Outcome::Value(Val::Num(1.0))));
    assert_eq!(res.get(t.a, &single(names::WIDTH)), None);
    assert!(res.cycles.is_empty());
    // nothing written yet: the wrong cached value stays
    assert_eq!(number(&t.doc, t.b, &dep), 99.0);
    let before = format!("{:?}", t.doc.sheet(t.b));
    let _ = eng.calculate(&t.doc);
    assert_eq!(format!("{:?}", t.doc.sheet(t.b)), before);
    // write: only the cell that differs
    let mut tx = t.doc.transaction("Berechnen");
    assert_eq!(res.write(&mut tx).unwrap(), 1);
    let ch = tx.commit().unwrap();
    assert_eq!(ch.steps(), 1);
    assert_eq!(number(&t.doc, t.b, &dep), 4.0);
    let f = t.doc.sym("Sheet.1!Width*2").unwrap();
    assert_eq!(t.doc.resolve(t.b, &dep).formula(), Formula::Expr(f));
    // a second write has nothing to do
    let res = eng.calculate(&t.doc);
    let mut tx = t.doc.transaction("Berechnen");
    assert_eq!(res.write(&mut tx).unwrap(), 0);
    assert!(tx.commit().is_none());
    // one cell from stored inputs
    assert_eq!(eng.evaluate(&t.doc, t.b, &dep), Outcome::Value(Val::Num(4.0)));
    assert_eq!(eng.evaluate(&t.doc, t.a, &single(names::WIDTH)), Outcome::Value(Val::Num(2.0)));
}

#[test]
fn inherited_formula_runs_in_the_instance() {
    let mut m = mx();
    let height = single(names::HEIGHT);
    let mut eng = Engine::new();
    let res = eng.calculate(&m.doc);
    assert_eq!(res.get(m.inst, &height), Some(&Outcome::Value(Val::Num(1.5))));
    assert_eq!(res.get(m.ms, &height), Some(&Outcome::Value(Val::Num(0.5))));
    let mut tx = m.doc.transaction("Berechnen");
    let n = res.write(&mut tx).unwrap();
    tx.commit();
    // Height, User.Sub and User.Me of the instance
    assert_eq!(n, 3);
    let own = *m.doc.sheet(m.inst).unwrap().cell(names::HEIGHT).unwrap();
    assert_eq!(own.formula, Formula::Inherited);
    assert_eq!(own.value, Value::Num(1.5));
    // the master shape keeps its formula and value
    let r = m.doc.resolve(m.ms, &height);
    assert_eq!(r.num(), Some(0.5));
    assert!(matches!(r.formula(), Formula::Expr(_)));
    // the instance's value is its own, the formula still the master's
    let r = m.doc.resolve(m.inst, &height);
    assert_eq!(r.num(), Some(1.5));
    assert_eq!(r.source, Some(m.ms));
}

#[test]
fn master_sheet_ids_map_to_the_instance() {
    let m = mx();
    let (sub, me) = (user_addr(&m.doc, "Sub"), user_addr(&m.doc, "Me"));
    let mut eng = Engine::new();
    let res = eng.calculate(&m.doc);
    // Sheet.6 in the master is the instance's subshape with MasterShape 6, Sheet.5 the instance
    assert_eq!(res.get(m.inst, &sub), Some(&Outcome::Value(Val::Num(0.75))));
    assert_eq!(res.get(m.inst, &me), Some(&Outcome::Value(Val::Num(3.0))));
    assert_eq!(res.get(m.ms, &sub), Some(&Outcome::Value(Val::Num(0.25))));
    assert_eq!(res.get(m.ms, &me), Some(&Outcome::Value(Val::Num(1.0))));
    assert_eq!(eng.evaluate(&m.doc, m.inst, &sub), Outcome::Value(Val::Num(0.75)));
    // master information and Masters[...]! seen from the instance
    assert_eq!(eval_in(&m.doc, m.inst, "MASTERNAME()"), Val::str("Kasten"));
    assert_eq!(eval_in(&m.doc, m.inst, "MASTERNAME(750)"), Val::str("Box"));
    assert_eq!(eval_in(&m.doc, m.inst, "Masters[Box]!Sheet.5!Width"), Val::Num(1.0));
    assert_eq!(eval_in(&m.doc, m.inst, "Masters[Kasten]!Sheet.6!Width"), Val::Num(0.25));
}

#[test]
fn unknown_functions_and_parse_errors_keep_the_value() {
    let (mut doc, sh) = one(&[("U", "FOO(1)+2", 42.0), ("V", "User.U+1", 0.0), ("P", "1+", 7.0), ("R", "RAND()", 0.5)]);
    let (u, v, p, r) = (user_addr(&doc, "U"), user_addr(&doc, "V"), user_addr(&doc, "P"), user_addr(&doc, "R"));
    let mut eng = Engine::new();
    let res = eng.calculate(&doc);
    assert_eq!(res.get(sh, &u), Some(&Outcome::Kept(Keep::Unknown(String::from("FOO")))));
    // a reader gets the stored value
    assert_eq!(res.get(sh, &v), Some(&Outcome::Value(Val::Num(43.0))));
    assert_eq!(
        res.get(sh, &p),
        Some(&Outcome::Kept(Keep::Parse(ParseError { pos: 2, kind: ParseErrorKind::UnexpectedEnd })))
    );
    assert_eq!(res.get(sh, &r), Some(&Outcome::Kept(Keep::NotComputed("RAND"))));
    let mut tx = doc.transaction("Berechnen");
    assert_eq!(res.write(&mut tx).unwrap(), 1);
    tx.commit();
    for (a, want) in [(&u, 42.0), (&p, 7.0), (&r, 0.5), (&v, 43.0)] {
        let c = doc.resolve(sh, a);
        assert_eq!(c.num(), Some(want));
        assert_eq!(c.error(), None);
    }
}

#[test]
fn evaluate_text_keeps_unknown_and_parse_errors() {
    let (doc, sh) = one(&[]);
    // the same through evaluate_text
    assert_eq!(
        Engine::new().evaluate_text(&doc, sh, &probe(), "BAR(1)"),
        Ok(Outcome::Kept(Keep::Unknown(String::from("BAR"))))
    );
    assert_eq!(Engine::new().evaluate_text(&doc, sh, &probe(), "1+").err().map(|e| e.kind), Some(ParseErrorKind::UnexpectedEnd));
}

#[test]
fn cycles_do_not_crash() {
    let (mut doc, sh) = one(&[("A", "User.B+1", 0.0), ("B", "User.A+1", 0.0), ("C", "User.C*2", 3.0)]);
    let (a, b, c) = (user_addr(&doc, "A"), user_addr(&doc, "B"), user_addr(&doc, "C"));
    let mut eng = Engine::new();
    let res = eng.calculate(&doc);
    assert!(!res.cycles.is_empty());
    assert!(res.cycles.iter().any(|(s, x)| *s == sh && (*x == a || *x == b)));
    assert!(res.cycles.iter().any(|(s, x)| *s == sh && *x == c));
    // every cell came out as a number
    for x in [&a, &b, &c] {
        assert!(matches!(res.get(sh, x), Some(Outcome::Value(Val::Num(_)))), "{:?}", res.get(sh, x));
    }
    let mut tx = doc.transaction("Berechnen");
    res.write(&mut tx).unwrap();
    tx.commit();
    // recalculating a cycle again does not crash either
    let mut tx = doc.transaction("Ändern");
    tx.set_constant(sh, &single(names::WIDTH), Value::Num(5.0), None).unwrap();
    eng.recalc(&mut tx, &[sh]).unwrap();
    tx.commit();
}

// n User rows: r<i> = r<i-1>+1 (forward) or r<i> = r<i+1>+1 (each needs the next one, so the
// first cell computed pulls in the whole chain)
fn chain(n: usize, forward: bool) -> (Document, SheetRef, Vec<CellAddr>) {
    let rows_n: Vec<String> = (0..n).map(|i| format!("r{i}")).collect();
    let formulas: Vec<String> = (0..n)
        .map(|i| match (forward, i) {
            (true, 0) => String::from("1"),
            (true, i) => format!("User.r{}+1", i - 1),
            (false, i) if i == n - 1 => String::from("1"),
            (false, i) => format!("User.r{}+1", i + 1),
        })
        .collect();
    let rows: Vec<(&str, &str, f64)> = rows_n.iter().zip(&formulas).map(|(a, b)| (a.as_str(), b.as_str(), 0.0)).collect();
    let (doc, sh) = one(&rows);
    let addrs = rows_n.iter().map(|x| user_addr(&doc, x)).collect();
    (doc, sh, addrs)
}

#[test]
fn deep_chain_forward() {
    let (doc, sh, addrs) = chain(500, true);
    let res = Engine::new().calculate(&doc);
    for (i, a) in addrs.iter().enumerate() {
        assert_eq!(res.get(sh, a), Some(&Outcome::Value(Val::Num(i as f64 + 1.0))), "r{i}");
    }
}

// the first cell pulls in the others, within the engine's depth limit
#[test]
fn chain_backward_within_the_depth_limit() {
    let n = 40;
    let (doc, sh, addrs) = chain(n, false);
    let res = Engine::new().calculate(&doc);
    for (i, a) in addrs.iter().enumerate() {
        assert_eq!(res.get(sh, a), Some(&Outcome::Value(Val::Num((n - i) as f64))), "r{i}");
    }
}

#[test]
fn deep_chain_backward() {
    let n = 500;
    let (mut doc, sh, addrs) = chain(n, false);
    let mut eng = Engine::new();
    let res = eng.calculate(&doc);
    for (i, a) in addrs.iter().enumerate() {
        assert_eq!(res.get(sh, a), Some(&Outcome::Value(Val::Num((n - i) as f64))), "r{i}");
    }
    assert!(res.cycles.is_empty());
    let mut tx = doc.transaction("Berechnen");
    res.write(&mut tx).unwrap();
    tx.commit();
    assert_eq!(number(&doc, sh, &addrs[0]), 500.0);
    // a change at the end runs through the whole chain
    let mut tx = doc.transaction("Ändern");
    tx.set_formula(sh, &addrs[n - 1], "10").unwrap();
    eng.recalc_cells(&mut tx, &[(sh, addrs[n - 1])]).unwrap();
    assert_eq!(number(tx.doc(), sh, &addrs[0]), 509.0);
}

// after a full calculation has the right values, a change at the end of a long chain
#[test]
fn deep_chain_recalc_after_a_change() {
    let n = 500;
    let (mut doc, sh, addrs) = chain(n, true);
    let mut eng = Engine::new();
    let res = eng.calculate(&doc);
    let mut tx = doc.transaction("Berechnen");
    res.write(&mut tx).unwrap();
    tx.commit();
    assert_eq!(number(&doc, sh, &addrs[n - 1]), 500.0);
    let mut tx = doc.transaction("Ändern");
    tx.set_formula(sh, &addrs[0], "10").unwrap();
    let rc = eng.recalc_cells(&mut tx, &[(sh, addrs[0])]).unwrap();
    assert_eq!(rc.written, n, "{:?}", rc.kept);
    assert_eq!(number(tx.doc(), sh, &addrs[n - 1]), 509.0);
    tx.commit();
}

#[test]
fn recalc_updates_dependents_and_undo_restores() {
    let mut t = tiny(4.0);
    let (dep, width, height) = (user_addr(&t.doc, "Dep"), single(names::WIDTH), single(names::HEIGHT));
    let mut eng = Engine::new();
    let _ = eng.calculate(&t.doc);
    // a constant
    let mut tx = t.doc.transaction("Breite");
    tx.set_constant(t.a, &width, Value::Num(4.0), None).unwrap();
    let rc = eng.recalc(&mut tx, &[t.a]).unwrap();
    assert!(rc.written >= 2, "{rc:?}");
    assert_eq!(number(tx.doc(), t.b, &dep), 8.0);
    assert_eq!(number(tx.doc(), t.a, &height), 2.0);
    let mut ch = tx.commit().unwrap();
    assert!(ch.undo(&mut t.doc));
    assert_eq!(number(&t.doc, t.b, &dep), 4.0);
    assert_eq!(number(&t.doc, t.a, &height), 1.0);
    assert_eq!(number(&t.doc, t.a, &width), 2.0);
    eng.refresh(&t.doc, &ch.sheets());
    assert!(ch.redo(&mut t.doc));
    assert_eq!(number(&t.doc, t.b, &dep), 8.0);
    assert!(ch.undo(&mut t.doc));
    eng.refresh(&t.doc, &ch.sheets());
}

#[test]
fn recalc_after_set_formula_clears_stale() {
    let mut t = tiny(4.0);
    let (dep, width, height) = (user_addr(&t.doc, "Dep"), single(names::WIDTH), single(names::HEIGHT));
    let mut eng = Engine::new();
    let _ = eng.calculate(&t.doc);
    // a formula: stale until the recalc in the same transaction
    let mut tx = t.doc.transaction("Breite");
    tx.set_formula(t.a, &width, "3 in").unwrap();
    assert!(tx.doc().resolve(t.a, &width).cell.unwrap().stale);
    eng.recalc(&mut tx, &[t.a]).unwrap();
    let w = tx.doc().resolve(t.a, &width);
    assert_eq!(w.num(), Some(3.0));
    assert!(!w.cell.unwrap().stale);
    assert_eq!(number(tx.doc(), t.b, &dep), 6.0);
    assert_eq!(number(tx.doc(), t.a, &height), 1.5);
    let mut ch = tx.commit().unwrap();
    assert!(ch.undo(&mut t.doc));
    assert_eq!(number(&t.doc, t.a, &width), 2.0);
    assert_eq!(t.doc.resolve(t.a, &width).formula(), Formula::Absent);
    assert_eq!(number(&t.doc, t.b, &dep), 4.0);
}

#[test]
fn recalc_cells_follows_dependents() {
    let mut t = tiny(4.0);
    let (num_a, dep2) = (user_addr(&t.doc, "Num"), user_addr(&t.doc, "Dep2"));
    let mut eng = Engine::new();
    let _ = eng.calculate(&t.doc);
    let mut tx = t.doc.transaction("Zahl");
    tx.set_constant(t.a, &num_a, Value::Num(10.0), None).unwrap();
    let rc = eng.recalc_cells(&mut tx, &[(t.a, num_a)]).unwrap();
    assert_eq!(rc.written, 1, "{rc:?}");
    assert_eq!(number(tx.doc(), t.b, &dep2), 11.0);
    tx.commit();
}

#[test]
fn refresh_after_undo_records_dependencies_anew() {
    let mut t = tiny(4.0);
    let (dep, num_a, width) = (user_addr(&t.doc, "Dep"), user_addr(&t.doc, "Num"), single(names::WIDTH));
    let mut eng = Engine::new();
    let _ = eng.calculate(&t.doc);
    // Dep reads User.Num instead of Width for a while
    let mut tx = t.doc.transaction("Formel");
    tx.set_formula(t.b, &dep, "Sheet.1!User.Num*2").unwrap();
    eng.recalc(&mut tx, &[t.b]).unwrap();
    assert_eq!(number(tx.doc(), t.b, &dep), 14.0);
    let mut ch = tx.commit().unwrap();
    assert!(ch.undo(&mut t.doc));
    assert_eq!(number(&t.doc, t.b, &dep), 4.0);
    eng.refresh(&t.doc, &ch.sheets());
    // Dep reads Width again
    let mut tx = t.doc.transaction("Breite");
    tx.set_constant(t.a, &width, Value::Num(5.0), None).unwrap();
    eng.recalc(&mut tx, &[t.a]).unwrap();
    assert_eq!(number(tx.doc(), t.b, &dep), 10.0);
    tx.commit();
    // and no longer User.Num
    let mut tx = t.doc.transaction("Zahl");
    tx.set_constant(t.a, &num_a, Value::Num(1.0), None).unwrap();
    eng.recalc_cells(&mut tx, &[(t.a, num_a)]).unwrap();
    assert_eq!(number(tx.doc(), t.b, &dep), 10.0);
}

#[test]
fn user_set_guard_and_plain_cells() {
    let mut t = tiny(4.0);
    let (lpx, height, tg) = (single(names::LOC_PIN_X), single(names::HEIGHT), user_addr(&t.doc, "Tg"));
    let mut eng = Engine::new();
    assert!(eng.guarded(&t.doc, t.a, &lpx));
    assert!(eng.theme_guarded(&t.doc, t.a, &lpx));
    assert!(!eng.guarded(&t.doc, t.a, &height));
    assert!(!eng.theme_guarded(&t.doc, t.a, &height));
    assert!(!eng.guarded(&t.doc, t.a, &tg));
    assert!(eng.theme_guarded(&t.doc, t.a, &tg));
    let mut tx = t.doc.transaction("Ziehen");
    assert_eq!(eng.user_set(&mut tx, t.a, &lpx, Input::Value(Value::Num(0.3), None)).err(), Some(EditError::Guarded));
    assert_eq!(eng.user_set(&mut tx, t.a, &lpx, Input::Formula("0.3")).err(), Some(EditError::Guarded));
    assert!(tx.commit().is_none());
    assert_eq!(number(&t.doc, t.a, &lpx), 1.0);
}

#[test]
fn user_set_plain_cell_recalculates() {
    let mut t = tiny(4.0);
    let (lpx, height, width) = (single(names::LOC_PIN_X), single(names::HEIGHT), single(names::WIDTH));
    let mut eng = Engine::new();
    // a plain cell takes the input and its dependents follow
    let dep = user_addr(&t.doc, "Dep");
    let mut tx = t.doc.transaction("Breite");
    eng.user_set(&mut tx, t.a, &width, Input::Length(3.0, Unit::Inches)).unwrap();
    assert_eq!(number(tx.doc(), t.a, &width), 3.0);
    assert_eq!(number(tx.doc(), t.a, &height), 1.5);
    assert_eq!(number(tx.doc(), t.a, &lpx), 1.5);
    assert_eq!(number(tx.doc(), t.b, &dep), 6.0);
    tx.commit();
}

#[test]
fn user_set_through_setatref() {
    let mut t = tiny(4.0);
    let (proxy, target) = (user_addr(&t.doc, "Proxy"), user_addr(&t.doc, "Target"));
    let f = t.doc.sym("SETATREF(User.Target)").unwrap();
    let mut eng = Engine::new();
    let mut tx = t.doc.transaction("Eingabe");
    eng.user_set(&mut tx, t.a, &proxy, Input::Value(Value::Num(5.0), None)).unwrap();
    assert_eq!(number(tx.doc(), t.a, &target), 5.0);
    assert_eq!(tx.doc().resolve(t.a, &target).formula(), Formula::Absent);
    // the formula stays, its value follows
    let r = tx.doc().resolve(t.a, &proxy);
    assert_eq!(r.formula(), Formula::Expr(f));
    assert_eq!(r.num(), Some(5.0));
    tx.commit();
    // a length in another unit
    let mut tx = t.doc.transaction("Eingabe");
    eng.user_set(&mut tx, t.a, &proxy, Input::Length(25.4, Unit::Millimeters)).unwrap();
    assert_near!(number(tx.doc(), t.a, &target), 1.0);
    assert_near!(number(tx.doc(), t.a, &proxy), 1.0);
    assert_eq!(tx.doc().resolve(t.a, &proxy).formula(), Formula::Expr(f));
}

#[test]
fn user_set_setatref_with_expression() {
    let (mut doc, sh) = one(&[("Target", "", 1.0), ("Proxy", "SETATREF(User.Target,SETATREFEXPR()*2)", 1.0)]);
    let (proxy, target) = (user_addr(&doc, "Proxy"), user_addr(&doc, "Target"));
    let mut eng = Engine::new();
    let mut tx = doc.transaction("Eingabe");
    eng.user_set(&mut tx, sh, &proxy, Input::Value(Value::Num(5.0), None)).unwrap();
    assert_eq!(number(tx.doc(), sh, &target), 10.0);
}

#[test]
fn run_action_setf_toggles() {
    let (mut doc, sh) = one(&[("Flag", "", 0.0)]);
    let flag = user_addr(&doc, "Flag");
    let act = add_action(&mut doc, sh, "Row_1", "SETF(GetRef(User.Flag),NOT(User.Flag))");
    let mut eng = Engine::new();
    // recalculating the action does nothing
    let res = eng.calculate(&doc);
    assert_eq!(res.get(sh, &act), Some(&Outcome::Value(Val::Num(0.0))));
    let mut tx = doc.transaction("Aktion");
    let eff = eng.run_action(&mut tx, sh, &act).unwrap();
    assert!(eff.is_empty(), "{eff:?}");
    assert_eq!(number(tx.doc(), sh, &flag), 1.0);
    tx.commit();
    let mut tx = doc.transaction("Aktion");
    let eff = eng.run_action(&mut tx, sh, &act).unwrap();
    assert!(eff.is_empty(), "{eff:?}");
    assert_eq!(number(tx.doc(), sh, &flag), 0.0);
    tx.commit();
    assert_eq!(number(&doc, sh, &flag), 0.0);
}

#[test]
fn run_action_effects() {
    let (mut doc, sh) = one(&[]);
    let cases = [
        ("Row_1", "OPENTEXTWIN()", Effect::OpenTextWindow(sh)),
        ("Row_2", "DOCMD(1312)", Effect::Command(1312)),
        ("Row_3", r#"RUNADDON("x")"#, Effect::AddOn(String::from("x"))),
        ("Row_4", r#"RUNMACRO("m")"#, Effect::Refused(String::from("m"))),
        ("Row_5", r#"GOTOPAGE("Seite 2")"#, Effect::GotoPage(String::from("Seite 2"))),
        ("Row_6", r#"HYPERLINK("https://example.org","a")"#, Effect::Hyperlink(String::from("https://example.org"), String::from("a"))),
        ("Row_7", "OPENSHEETWIN()", Effect::OpenShapeSheet(sh)),
        ("Row_8", "OPENGROUPWIN()", Effect::OpenGroup(sh)),
        ("Row_9", r#"PLAYSOUND("ding")"#, Effect::PlaySound(String::from("ding"))),
        ("Row_10", r#"CALLTHIS("x")"#, Effect::Refused(String::from("x"))),
        ("Row_11", r#"RUNADDONWARGS("a","b")"#, Effect::AddOn(String::from("a"))),
    ];
    let addrs: Vec<CellAddr> = cases.iter().map(|(r, f, _)| add_action(&mut doc, sh, r, f)).collect();
    let mut eng = Engine::new();
    // calculate computes 0 and has no effects
    let res = eng.calculate(&doc);
    for a in &addrs {
        assert_eq!(res.get(sh, a), Some(&Outcome::Value(Val::Num(0.0))));
    }
    for ((_, f, want), a) in cases.iter().zip(&addrs) {
        let mut tx = doc.transaction("Aktion");
        let eff = eng.run_action(&mut tx, sh, a).unwrap();
        assert_eq!(eff, vec![want.clone()], "{f}");
        assert!(tx.commit().is_none(), "{f}");
    }
}

#[test]
fn themed_values() {
    // LineWeight = THEMEVAL() (a cell with a theme meaning), User.T2 reads it
    let (mut doc, sh) = one(&[("T2", "LineWeight+1", 0.0)]);
    let lw = with_f(&mut doc, "LineWeight", "THEMEVAL()", Value::Num(0.0));
    doc.sheet_mut(sh).unwrap().cells.push(lw);
    let (t, t2) = (single(names::LINE_WEIGHT), user_addr(&doc, "T2"));
    let mut eng = Engine::new();
    let res = eng.calculate(&doc);
    let inner = match res.get(sh, &t) {
        Some(Outcome::Value(Val::Themed(v))) => (**v).clone(),
        other => panic!("{other:?}"),
    };
    assert!(!matches!(inner, Val::Themed(_)));
    let x = inner.num(Axis::X).unwrap();
    // a reader gets the value behind it, not Themed
    assert_eq!(res.get(sh, &t2), Some(&Outcome::Value(Val::Num(x + 1.0))));
    let mut tx = doc.transaction("Berechnen");
    res.write(&mut tx).unwrap();
    tx.commit();
    // V stores "Themed"
    let v = doc.resolve(sh, &t).value();
    assert!(matches!(v, Value::Text(s) if doc.str(s) == "Themed"), "{v:?}");
    assert_eq!(number(&doc, sh, &t2), x + 1.0);
    // read again from the stored "Themed"
    assert_eq!(eval_in(&doc, sh, "LineWeight+5"), Val::Num(x + 5.0));
    let res = eng.calculate(&doc);
    assert_eq!(res.get(sh, &t2), Some(&Outcome::Value(Val::Num(x + 1.0))));
    let mut tx = doc.transaction("Berechnen");
    assert_eq!(res.write(&mut tx).unwrap(), 0);
}

#[test]
fn defaults_of_missing_cells() {
    let mut f = fx();
    // single cells no sheet has: 0, a few 1
    assert_near!(f.n("Rounding"), 0.0);
    assert_near!(f.n("TxtAngle"), 0.0);
    f.doc.intern("DropOnPageScale");
    assert_near!(f.n("DropOnPageScale"), 1.0);
    // a row cell that does not exist
    assert_eq!(f.e("User.Nope"), CellError::Ref);
    assert_eq!(f.e("Geometry1.X9"), CellError::Ref);
    assert_eq!(f.e("Geometry2.X1"), CellError::Ref);
    assert_eq!(f.e("Prop.Name.Nope"), CellError::Ref);
}

#[test]
fn default_one_without_the_name_in_the_document() {
    let f = fx();
    assert!(f.doc.sym("DropOnPageScale").is_none());
    assert_near!(f.n("DropOnPageScale"), 1.0);
}


#[test]
fn style_formula_runs_in_the_shape() {
    // the style's Rounding = Width*0.1 runs with the Width of the shape that uses the style
    let mut doc = Document::new(DocKind::Drawing);
    let mut st = Style::default();
    st.header.id = 1;
    st.header.name_u = Some(doc.intern("Rund"));
    st.sheet.cells.push(num(&mut doc, "Width", 10.0));
    st.sheet.cells.push(with_f(&mut doc, "Rounding", "Width*0.1", Value::Num(1.0)));
    let style = SheetRef::Style(doc.insert_style(st).unwrap());
    let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap();
    let mut d = ShapeData::new(ShapeKind::Shape);
    d.header.styles = StyleRefs { line: Some(1), ..StyleRefs::default() };
    d.sheet.cells.push(num(&mut doc, "Width", 2.0));
    let slot = doc.page_mut(page).unwrap().shapes.insert(None, 1, d).unwrap();
    let sh = SheetRef::Shape(ShapeRef { owner: Owner::Page(page), slot });
    let rounding = single(names::ROUNDING);
    let res = Engine::new().calculate(&doc);
    assert_eq!(res.get(sh, &rounding), Some(&Outcome::Value(Val::Num(0.2))));
    assert_eq!(res.get(style, &rounding), Some(&Outcome::Value(Val::Num(1.0))));
    let mut tx = doc.transaction("Berechnen");
    assert_eq!(res.write(&mut tx).unwrap(), 1);
    tx.commit();
    let own = *doc.sheet(sh).unwrap().cell(names::ROUNDING).unwrap();
    assert_eq!((own.formula, own.value), (Formula::Inherited, Value::Num(0.2)));
    assert_eq!(number(&doc, style, &rounding), 1.0);
}

#[test]
fn formula_cells_include_inherited_ones() {
    let m = mx();
    let height = single(names::HEIGHT);
    let cells = sheet_formula_cells(&m.doc, m.inst);
    for a in [height, user_addr(&m.doc, "Sub"), user_addr(&m.doc, "Me")] {
        assert!(cells.contains(&NodeKey::new(m.inst, &a)), "{a:?}");
    }
    assert!(!cells.contains(&NodeKey::new(m.inst, &single(names::WIDTH))));
    assert!(formula_cells(&m.doc).contains(&NodeKey::new(m.ms, &height)));
    let sheets = all_sheets(&m.doc);
    for s in [SheetRef::Document, SheetRef::Master(m.master), m.ms, m.inst] {
        assert!(sheets.contains(&s), "{s:?}");
    }
}

#[test]
fn deleted_rows_read_ref() {
    // the master shape has Geometry1 rows 1 and 2, the instance deletes row 2
    let mut m = mx();
    let row = |ix: u32, x: f64| {
        let mut r = Row::new(RowKey::Index(ix));
        r.cells.push(Cell::constant(names::X, Value::Num(x), None));
        r
    };
    let mut geo = Section::new(SecKey::one(names::GEOMETRY));
    geo.rows.push(row(1, 0.5));
    geo.rows.push(row(2, 1.0));
    m.doc.sheet_mut(m.ms).unwrap().sections.push(geo);
    let mut del = Section::new(SecKey::one(names::GEOMETRY));
    let mut gone = Row::new(RowKey::Index(2));
    gone.del = true;
    del.rows.push(gone);
    m.doc.sheet_mut(m.inst).unwrap().sections.push(del);
    assert_eq!(eval_in(&m.doc, m.ms, "Geometry1.X2"), Val::Num(1.0));
    assert_eq!(eval_in(&m.doc, m.inst, "Geometry1.X1"), Val::Num(0.5));
    assert_eq!(eval_in(&m.doc, m.inst, "Geometry1.X2"), Val::Err(CellError::Ref));
    assert_eq!(eval_in(&m.doc, m.inst, "ISERROR(Geometry1.X2)"), Val::Num(1.0));
}

#[test]
fn write_stores_errors_texts_colours_points() {
    let (mut doc, sh) = one(&[("E", "1/0", 5.0), ("S", r#""ab"&"c""#, 0.0)]);
    let ff = with_f(&mut doc, "FillForegnd", "RGB(255,0,0)", Value::Num(0.0));
    let cx = with_f(&mut doc, "X", "PNT(1,2)", Value::Num(0.0));
    let cy = with_f(&mut doc, "Y", "PNT(1,2)", Value::Num(0.0));
    {
        let sheet = doc.sheet_mut(sh).unwrap();
        sheet.cells.push(ff);
        put_row(sheet, names::CONNECTION, RowKey::Index(0), vec![cx, cy]);
    }
    let mut eng = Engine::new();
    let res = eng.calculate(&doc);
    let mut tx = doc.transaction("Berechnen");
    assert_eq!(res.write(&mut tx).unwrap(), 5);
    tx.commit();
    // an error leaves V alone and sets E
    let e = doc.resolve(sh, &user_addr(&doc, "E"));
    assert_eq!(e.value(), Value::Num(5.0));
    assert_eq!(e.error().map(|x| doc.str(x)), Some("#DIV/0!"));
    let text = |a: &CellAddr| match doc.resolve(sh, a).value() {
        Value::Text(t) => String::from(doc.str(t)),
        v => panic!("{v:?}"),
    };
    assert_eq!(text(&user_addr(&doc, "S")), "abc");
    assert_eq!(text(&single(names::FILL_FOREGND)), "#ff0000");
    // a point: X takes x, Y takes y
    let conn = |c: Sym| CellAddr::row(SecKey::one(names::CONNECTION), RowKey::Index(0), c);
    assert_eq!(number(&doc, sh, &conn(names::X)), 1.0);
    assert_eq!(number(&doc, sh, &conn(names::Y)), 2.0);
    // nothing left to write
    let res = eng.calculate(&doc);
    let mut tx = doc.transaction("Berechnen");
    assert_eq!(res.write(&mut tx).unwrap(), 0);
}

#[test]
fn setf_with_a_formula_text_and_without_a_reference() {
    let (mut doc, sh) = one(&[("W", "", 0.0)]);
    let w = user_addr(&doc, "W");
    let act = add_action(&mut doc, sh, "Row_1", r#"SETF(GetRef(User.W),"Width*3")"#);
    let bad = add_action(&mut doc, sh, "Row_2", "SETF(5,1)");
    let mut eng = Engine::new();
    let res = eng.calculate(&doc);
    assert_eq!(res.get(sh, &bad), Some(&Outcome::Value(Val::Err(CellError::Ref))));
    let mut tx = doc.transaction("Aktion");
    assert!(eng.run_action(&mut tx, sh, &act).unwrap().is_empty());
    let r = tx.doc().resolve(sh, &w);
    assert_eq!(r.num(), Some(6.0));
    assert!(matches!(r.formula(), Formula::Expr(f) if tx.doc().str(f) == "Width*3"));
    tx.commit();
    // a cell without a formula does nothing
    let mut tx = doc.transaction("Aktion");
    assert!(eng.run_action(&mut tx, sh, &w).unwrap().is_empty());
    assert!(tx.commit().is_none());
}

#[test]
fn engine_graph_and_reset() {
    let mut t = tiny(4.0);
    let dep = user_addr(&t.doc, "Dep");
    let mut eng = Engine::new();
    let _ = eng.calculate(&t.doc);
    let (cells, edges) = eng.graph_size();
    assert!(cells >= 6 && edges >= 5, "{cells} {edges}");
    eng.reset();
    assert_eq!(eng.graph_size(), (0, 0));
    // after a reset the next recalc builds the graph again
    let mut tx = t.doc.transaction("Breite");
    tx.set_constant(t.a, &single(names::WIDTH), Value::Num(3.0), None).unwrap();
    eng.recalc(&mut tx, &[t.a]).unwrap();
    assert_eq!(number(tx.doc(), t.b, &dep), 6.0);
}

// ==== 5. stored values

#[test]
fn close_tolerance() {
    assert!(close(1.0, 1.0, 0.0));
    assert!(close(1.0, 1.0 + 1e-13, 0.0));
    assert!(!close(1.0, 1.0 + 1e-9, 0.0));
    assert!(close(1000.0, 1000.0005, 1e-6));
    assert!(!close(1000.0, 1000.01, 1e-6));
    assert!(close(0.0, 5e-13, 0.0));
    assert!(!close(0.0, 2e-12, 0.0));
    assert!(close(-2.0, -2.0 * (1.0 + 1e-10), 1e-9));
}

#[test]
fn stored_matches() {
    let mut st = Strings::new();
    let red = st.intern("#FF0000");
    let abc = st.intern("ABC");
    let two = st.intern("2");
    let nurbs = st.intern("NURBS(1,3,0,0.0000000000001)");
    let poly = st.intern("POLYLINE(0, 0, 1,2)");
    assert!(Stored::Num(1.0).matches(Value::Num(1.0 + 1e-13), None, &st, 0.0));
    assert!(!Stored::Num(1.0).matches(Value::Num(1.001), None, &st, 0.0));
    assert!(Stored::Num(1.0).matches(Value::Num(1.001), None, &st, 1e-2));
    assert!(Stored::Num(0.0).matches(Value::Absent, None, &st, 0.0));
    assert!(!Stored::Num(1.0).matches(Value::Absent, None, &st, 0.0));
    assert!(Stored::Num(2.0).matches(Value::Text(two), None, &st, 0.0));
    // colours in any case, other texts exactly
    assert!(Stored::Text(String::from("#ff0000")).matches(Value::Text(red), None, &st, 0.0));
    assert!(!Stored::Text(String::from("abc")).matches(Value::Text(abc), None, &st, 0.0));
    assert!(Stored::Text(String::from("ABC")).matches(Value::Text(abc), None, &st, 0.0));
    assert!(Stored::Text(String::from("2")).matches(Value::Num(2.0), None, &st, 0.0));
    assert!(Stored::Text(String::new()).matches(Value::Absent, None, &st, 0.0));
    // NURBS and POLYLINE texts: the same numbers
    assert!(Stored::Text(String::from("NURBS(1, 3, 0, 0)")).matches(Value::Text(nurbs), None, &st, 0.0));
    assert!(Stored::Text(String::from("POLYLINE(0,0,1,2)")).matches(Value::Text(poly), None, &st, 0.0));
    assert!(!Stored::Text(String::from("POLYLINE(0,0,1,3)")).matches(Value::Text(poly), None, &st, 0.0));
}

#[test]
fn stored_matches_errors() {
    let st = Strings::new();
    // errors by E
    assert!(Stored::Error(CellError::Ref).matches(Value::Num(3.0), Some("#REF!"), &st, 0.0));
    assert!(!Stored::Error(CellError::Ref).matches(Value::Num(3.0), Some("#N/A"), &st, 0.0));
    assert!(!Stored::Error(CellError::Ref).matches(Value::Num(3.0), None, &st, 0.0));
}

#[test]
fn val_stored_forms() {
    // a point gives the coordinate of the cell's axis
    let p = Val::Point(1.0, 2.0);
    for (cell, want) in [("X", 1.0), ("Y", 2.0), ("PinX", 1.0), ("PinY", 2.0), ("BeginY", 2.0), ("EndX", 1.0), ("Width", 1.0)] {
        assert_eq!(p.stored(cell), Stored::Num(want), "{cell}");
    }
    assert_eq!(Axis::of("TxtPinY"), Axis::Y);
    assert_eq!(Axis::of("Value"), Axis::X);
    // colours: RGB as #rrggbb in lower case, an index as its number
    assert_eq!(Val::Color(Color::Rgb(255, 0, 171)).stored("FillForegnd"), Stored::Text(String::from("#ff00ab")));
    assert_eq!(Val::Color(Color::Index(4)).stored("LineColor"), Stored::Num(4.0));
    assert_eq!(Val::Num(f64::INFINITY).stored("Width"), Stored::Error(CellError::Num));
    assert_eq!(Val::Err(CellError::DivZero).stored("Width"), Stored::Error(CellError::DivZero));
    assert_eq!(Val::Themed(alloc::rc::Rc::new(Val::Num(1.0))).stored("FillForegnd"), Stored::Text(String::from("Themed")));
    assert_eq!(Val::str("abc").stored("Value"), Stored::Text(String::from("abc")));
    assert_eq!(Val::Missing.stored("Value"), Stored::Num(0.0));
}

#[test]
fn val_from_stored_value() {
    // and back
    let mut st = Strings::new();
    let c = st.intern("#4D1E1A");
    assert_eq!(Val::from_value(Value::Text(c), None, &st), Val::Color(Color::Rgb(0x4d, 0x1e, 0x1a)));
    assert_eq!(Val::from_value(Value::Num(3.0), Some("#DIV/0!"), &st), Val::Err(CellError::DivZero));
    assert_eq!(Val::from_value(Value::Absent, None, &st), Val::Num(0.0));
}

// (JD7) a cell holds a number: a point computed for another cell (PAR(PNT()) in EndY) is that
// cell's coordinate when a formula reads it -- Width = SQRT((EndX-BeginX)^2+(EndY-BeginY)^2) of a
// glued line read x for EndY before, and its angle came out 45 degrees
#[test]
fn a_point_read_from_a_cell_is_that_cells_coordinate() {
    let mut doc = Document::new(DocKind::Drawing);
    let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap();
    let mut d = ShapeData::new(ShapeKind::Shape);
    xform(&mut doc, &mut d, (1.0, 1.0), (2.0, 1.0), 0.0, (false, false));
    let bx = with_f(&mut doc, "BeginX", "PNT(3,4)", Value::Num(3.0));
    let by = with_f(&mut doc, "BeginY", "PNT(3,4)", Value::Num(4.0));
    d.sheet.cells.push(bx);
    d.sheet.cells.push(by);
    let slot = doc.page_mut(page).unwrap().shapes.insert(None, 1, d).unwrap();
    let sh = SheetRef::Shape(ShapeRef { owner: Owner::Page(page), slot });
    // the probe is a cell of the x axis (User.Label.Value): BeginY still gives y
    assert_eq!(eval_in(&doc, sh, "BeginY-BeginX"), Val::Num(1.0));
    assert_eq!(eval_in(&doc, sh, "BeginY*2"), Val::Num(8.0));
    // a point made in the formula itself stays a point until it is stored
    assert_eq!(eval_in(&doc, sh, "PNTY(PNT(BeginX,BeginY))"), Val::Num(4.0));
}
