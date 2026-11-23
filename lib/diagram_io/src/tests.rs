// the crate's unit tests (cargo test -p jflib-diagram-io --lib).  The modules keep theirs beside
// them (template/tests.rs, the tests in template_src.rs); this module in the crate root is what
// build_rs.py's suite looks for (unit_test_packages), so the suite runs all of them.  Tests of
// the crate as a whole come here.

use crate::vsdx::{self, Measure};
use diagram_core::DocKind;

// a new template reads back as a template, its windows part as Visio needs it
#[test]
fn new_template_round_trip() {
    let doc = vsdx::new_document(DocKind::Template, Measure::Metric);
    let bytes = vsdx::write(&doc, &vsdx::WriteOptions::DEFAULT).expect("write");
    let back = vsdx::read(&bytes, &vsdx::Limits::VSDX).expect("read");
    assert_eq!(back.doc.kind, DocKind::Template);
    assert!(crate::template::stencil_windows(&back.doc).is_empty());
}

// (JD13) a 2-D master whose Height is a formula of its text (a box that fits its text): it
// builds, the master page and the pin take the computed size, and an instance with a long text
// grows; a formula size next to an SVG drawing and a size of 0 are still refused
#[test]
fn stencil_size_from_text() {
    use crate::stencil;
    use diagram_core::formula::Engine;
    use diagram_core::{instance, CellAddr, Owner, ShapeRef, SheetRef, Text};

    const ST: &str = "stencil: Test\nfile: test\nname.de: T\nname.en: T\nname.es: T\nname.it: T\nname.fr: T\ncategory: general\nmasters: auto\n";
    let names = "name.de: A\nname.en: A\nname.es: A\nname.it: A\nname.fr: A\nprompt.de: P\nprompt.en: P\nprompt.es: P\nprompt.it: P\nprompt.fr: P\n";
    let geometry = "[Geometry]\nMoveTo  X = 0; Y = 0\nLineTo  X = Width; Y = 0\nLineTo  X = Width; Y = Height\nLineTo  X = Geometry1.X1; Y = Geometry1.Y1\n";
    let master = |height: &str, extra: &str| alloc::format!("master: Auto\n{names}{extra}\n[Shape]\nWidth = 40 mm\nHeight = {height}\n\n{geometry}");
    let build = |m: &str| {
        let m = alloc::string::String::from(m);
        let files = move |f: &str| match f {
            "stencil.txt" => Some(alloc::string::String::from(ST)),
            "auto.master" => Some(m.clone()),
            "a.svg" => Some(alloc::string::String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\"><rect width=\"10\" height=\"10\"/></svg>")),
            _ => None,
        };
        stencil::build(&files, Measure::Metric)
    };
    const MM: f64 = 1.0 / 25.4;
    let built = build(&master("GUARD(MAX(10 mm, TEXTHEIGHT(TheText,Width)))", "")).expect("a formula height builds");
    let doc = &built.doc;
    let m = doc.masters().next().expect("the master");
    let page = |n: &str| doc.sym(n).and_then(|s| doc.master(m).and_then(|x| x.sheet.cell(s))).and_then(|c| c.value.num()).unwrap_or(f64::NAN);
    let slot = doc.master(m).and_then(|x| x.master_shape()).expect("the master shape");
    let sheet = SheetRef::Shape(ShapeRef { owner: Owner::Master(m), slot });
    let num = |d: &diagram_core::Document, s: SheetRef, n: &str| d.sym(n).and_then(|c| d.resolve(s, &CellAddr::single(c)).num()).unwrap_or(f64::NAN);
    assert!((page("PageHeight") - 10.0 * MM).abs() < 1e-9 && (page("PageWidth") - 40.0 * MM).abs() < 1e-9);
    assert!((num(doc, sheet, "PinY") - 5.0 * MM).abs() < 1e-9 && (num(doc, sheet, "Height") - 10.0 * MM).abs() < 1e-9);
    // an instance with a long text: the formula comes along and makes it taller
    let mut d = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let pg = d.pages().next().expect("a page");
    let mut tx = d.transaction("drop");
    let local = instance::import_master(&mut tx, doc, m).expect("import");
    let s = instance::place(&mut tx, pg, local, 1.0, 1.0, 0).expect("place");
    tx.set_text(s, Some(Text::plain("a text long enough to need several lines in a box of forty millimetres"))).expect("text");
    Engine::new().recalc(&mut tx, &[SheetRef::Shape(s)]).expect("recalc");
    assert!(num(tx.doc(), SheetRef::Shape(s), "Height") > 12.0 * MM);
    drop(tx);
    // still refused: a formula size with an SVG drawing, a size of 0
    let refused = |m: &str| build(m).err().is_some_and(|e| e.message.contains("oder eine Formel (nicht mit svg:)"));
    assert!(refused(&master("GUARD(MAX(10 mm, TEXTHEIGHT(TheText,Width)))", "svg: a.svg\n")));
    assert!(refused(&master("0 mm", "")));
}

// (JD13) rows of a shape's Geometry and Connection sections carry their type T in the file, as
// Visio writes them: an instance resized from its master overrides inherited rows (F='Inh'
// cells) without a type of its own, and Office's previewer (Explorer's preview pane) hangs on
// rows without T; a Connection row without any type is written T='Connection'
#[test]
fn inherited_rows_written_with_their_type() {
    use crate::stencil;
    use diagram_core::formula::Engine;
    use diagram_core::{instance, SheetRef, Unit, Value};

    const ST: &str = "stencil: Test\nfile: test\nname.de: T\nname.en: T\nname.es: T\nname.it: T\nname.fr: T\ncategory: general\nmasters: box\n";
    const BOX: &str = "master: Box\nname.de: B\nname.en: B\nname.es: B\nname.it: B\nname.fr: B\nprompt.de: P\nprompt.en: P\nprompt.es: P\nprompt.it: P\nprompt.fr: P\n\n[Shape]\nWidth = 40 mm\nHeight = 20 mm\n\n[Geometry]\nMoveTo  X = 0; Y = 0\nLineTo  X = Width; Y = 0\nLineTo  X = Width; Y = Height\nLineTo  X = 0; Y = Height\nLineTo  X = Geometry1.X1; Y = Geometry1.Y1\n\n[Connection]\nRow  X = Width*0.5; Y = 0; DirX = 0; DirY = 1\nRow  X = Width; Y = Height*0.5; DirX = -1; DirY = 0\n";
    let files = |f: &str| match f {
        "stencil.txt" => Some(alloc::string::String::from(ST)),
        "box.master" => Some(alloc::string::String::from(BOX)),
        _ => None,
    };
    let built = stencil::build(&files, Measure::Metric).expect("the stencil builds");
    let m = built.doc.masters().next().expect("the master");
    let mut d = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let pg = d.pages().next().expect("a page");
    {
        let mut tx = d.transaction("drop");
        let local = instance::import_master(&mut tx, &built.doc, m).expect("import");
        let s = instance::place(&mut tx, pg, local, 2.0, 2.0, 0).expect("place");
        let w = tx.single("Width");
        tx.set_constant(SheetRef::Shape(s), &w, Value::Num(3.0), Some(Unit::Millimeters)).expect("width");
        Engine::new().recalc(&mut tx, &[SheetRef::Shape(s)]).expect("recalc");
        tx.commit();
    }
    let bytes = vsdx::write(&d, &vsdx::WriteOptions::DEFAULT).expect("write");
    let entries = archive::read(&bytes, archive::Limits::default()).expect("zip");
    let page = entries.iter().find(|e| e.name == "visio/pages/page1.xml").expect("page part");
    let text = core::str::from_utf8(&page.data).expect("utf-8");
    let geo = text.find("<Section N='Geometry'").expect("the instance overrides geometry rows");
    let rows: alloc::vec::Vec<&str> = text[geo..].split("<Row ").skip(1).collect();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|r| r.starts_with("T='")), "rows without T: {text}");
    assert!(text.contains("<Row T='LineTo' IX='2'>") && text.contains("<Row T='Connection'"), "{text}");
    // and the file reads back with the types in the model, written the same way again
    let back = vsdx::read(&bytes, &vsdx::Limits::VSDX).expect("read");
    let again = vsdx::write(&back.doc, &vsdx::WriteOptions::DEFAULT).expect("write again");
    assert_eq!(bytes, again);
}

// (JD13) the file names of the own stencils carry the prefix "jfd_" and read back; Visio's own
// stencil names are none of ours
#[test]
fn stencil_file_names() {
    use crate::stencil::{file_name, parse_file_name};
    assert_eq!(file_name("arrows", Measure::Metric), "jfd_arrows_m.vssx");
    assert_eq!(file_name("arrows", Measure::Us), "jfd_arrows_u.vssx");
    assert_eq!(parse_file_name("jfd_arrows_u.vssx"), Some((alloc::string::String::from("arrows"), Measure::Us)));
    assert_eq!(parse_file_name("C:/x/JFD_Flowchart_M.VSSX"), Some((alloc::string::String::from("flowchart"), Measure::Metric)));
    for other in ["ARROWS_M.VSSX", "arrows_m.vssx", "jfd__m.vssx", "jfd_arrows.vssx", "jfd_arrows_m.vsdx"] {
        assert_eq!(parse_file_name(other), None, "{other}");
    }
}

// (JD26) a group master of parts: [Part NAME] with its text in five languages and its own
// sections; Sheet.1 / Sheet.2 name the master shape and the first part and are written as their
// IDs; a drop in English shows the part's English text; parts next to svg: or on a 1-D master,
// a Sheet.N that is not there and a language suffix on a part's cell are refused
#[test]
fn stencil_parts() {
    use crate::stencil;
    use diagram_core::{instance, CellAddr, Owner, ShapeRef, SheetRef};

    const ST: &str = "stencil: Test\nfile: test\nname.de: T\nname.en: T\nname.es: T\nname.it: T\nname.fr: T\ncategory: general\nmasters: grp\n";
    let names = "name.de: A\nname.en: A\nname.es: A\nname.it: A\nname.fr: A\nprompt.de: P\nprompt.en: P\nprompt.es: P\nprompt.it: P\nprompt.fr: P\n";
    let part = "[Part Left]\ntext.de: Links\ntext.en: Left\ntext.es: Izquierda\ntext.it: Sinistra\ntext.fr: Gauche\n\n[Shape]\nWidth = GUARD(Sheet.1!Width*0.5)\nPinX = GUARD(Width*0.5)\nQuickStyleFillColor = 101\n\n[Geometry]\nMoveTo  X = 0; Y = 0\nLineTo  X = Width; Y = 0\nLineTo  X = Width; Y = Height\nLineTo  X = Geometry1.X1; Y = Geometry1.Y1\n";
    let master = |extra: &str, shape: &str, p: &str| alloc::format!("master: Group\n{names}{extra}\n[Shape]\nWidth = 40 mm\nHeight = 20 mm\n{shape}\n{p}");
    let build = |m: &str| {
        let m = alloc::string::String::from(m);
        let files = move |f: &str| match f {
            "stencil.txt" => Some(alloc::string::String::from(ST)),
            "grp.master" => Some(m.clone()),
            "a.svg" => Some(alloc::string::String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\"><rect width=\"10\" height=\"10\"/></svg>")),
            _ => None,
        };
        stencil::build(&files, Measure::Metric)
    };
    let built = build(&master("", "TxtWidth = Sheet.2!Width", part)).expect("a group of parts builds");
    let doc = &built.doc;
    let m = doc.masters().next().expect("the master");
    let slot = doc.master(m).and_then(|x| x.master_shape()).expect("the master shape");
    let group = ShapeRef { owner: Owner::Master(m), slot };
    let kids: alloc::vec::Vec<ShapeRef> = doc.shape(group).map(|s| s.children().collect()).unwrap_or_default();
    assert_eq!(kids.len(), 1);
    let (gid, pid) = (doc.shape(group).map_or(0, |s| s.id()), doc.shape(kids[0]).map_or(0, |s| s.id()));
    let formula = |s: SheetRef, n: &str| doc.sym(n).and_then(|c| doc.resolve(s, &CellAddr::single(c)).formula().expr()).map(|f| alloc::string::String::from(doc.str(f))).unwrap_or_default();
    assert_eq!(formula(SheetRef::Shape(kids[0]), "Width"), alloc::format!("GUARD(Sheet.{gid}!Width*0.5)"));
    assert_eq!(formula(SheetRef::Shape(group), "TxtWidth"), alloc::format!("Sheet.{pid}!Width"));
    // unset transform cells of a part: the group's size, guarded
    assert_eq!(formula(SheetRef::Shape(kids[0]), "Height"), alloc::format!("GUARD(Sheet.{gid}!Height)"));
    let num = |d: &diagram_core::Document, s: SheetRef, n: &str| d.sym(n).and_then(|c| d.resolve(s, &CellAddr::single(c)).num()).unwrap_or(f64::NAN);
    assert!((num(doc, SheetRef::Shape(kids[0]), "Width") * 25.4 - 20.0).abs() < 1e-9);
    // dropped in English: the part shows its English text
    let mut d = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let pg = d.pages().next().expect("a page");
    let mut tx = d.transaction("drop");
    let local = instance::import_master(&mut tx, doc, m).expect("import");
    let s = instance::place(&mut tx, pg, local, 1.0, 1.0, 1).expect("place");
    let sub = tx.doc().shape(s).and_then(|x| x.children().next()).expect("the part's instance");
    assert_eq!(tx.doc().shape(sub).and_then(|x| x.text()).map(|t| t.chars()).as_deref(), Some("Left"));
    drop(tx);
    // refused
    let refused = |m: &str, what: &str| build(m).err().is_some_and(|e| e.message.contains(what));
    assert!(refused(&master("svg: a.svg\n", "", part), "svg"));
    assert!(refused(&master("", "TxtWidth = Sheet.3!Width", part), "Sheet.3!"));
    assert!(refused(&master("", "", &part.replace("QuickStyleFillColor = 101", "Comment.de = \"x\"")), "Sprachendungen"));
}

// (JD26) the logical sheet numbers of a group master become IDs, strings stay as written
#[test]
fn stencil_sheet_ids() {
    assert_eq!(crate::stencil::sheet_ids_for_test("Sheet.1!Width+Sheet.12!Height", &[5, 6]).err().map(|_| ()), Some(()));
    assert_eq!(crate::stencil::sheet_ids_for_test("Sheet.1!Width*0.5+Sheet.2!X", &[5, 9]).ok().as_deref(), Some("Sheet.5!Width*0.5+Sheet.9!X"));
    assert_eq!(crate::stencil::sheet_ids_for_test("\"Sheet.1!\"&Sheet.2!X", &[5, 9]).ok().as_deref(), Some("\"Sheet.1!\"&Sheet.9!X"));
    assert_eq!(crate::stencil::sheet_ids_for_test("MySheet.1!X+Sheet.10", &[5]).ok().as_deref(), Some("MySheet.1!X+Sheet.10"));
}
