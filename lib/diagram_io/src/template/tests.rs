// the template build tool and the runtime helpers on small inline sources: a stencil of two
// masters, a template with pages, layers, a style, a rule set and a starter diagram

use super::*;
use crate::stencil::Built;
use crate::vsdx::Limits;

const STENCIL: &str = "stencil: T\nfile: t\nname.de: T\nname.en: T\nname.es: T\nname.it: T\nname.fr: T\ncategory: general\nmasters: box, note\n";

const BOX: &str = "\
master: Box
name.de: Kasten
name.en: Box
name.es: Caja
name.it: Scatola
name.fr: Boîte
prompt.de: p
prompt.en: p
prompt.es: p
prompt.it: p
prompt.fr: p
text.de: Kasten
text.en: Box
text.es: Caja
text.it: Scatola
text.fr: Boîte

[Shape]
Width = 20 mm
Width.us = 1 in
Height = 10 mm
Height.us = 0.5 in

[Geometry]
MoveTo X = 0; Y = 0
LineTo X = Width; Y = 0
LineTo X = Width; Y = Height
LineTo X = 0; Y = Height
LineTo X = 0; Y = 0\n";

const NOTE: &str = "\
master: Note
name.de: Notiz
name.en: Note
name.es: Nota
name.it: Nota
name.fr: Note
prompt.de: p
prompt.en: p
prompt.es: p
prompt.it: p
prompt.fr: p
style: Text Only

[Shape]
Width = 40 mm
Height = 10 mm\n";

fn stencil_doc(m: Measure) -> Built {
    let files = |n: &str| match n {
        "stencil.txt" => Some(String::from(STENCIL)),
        "box.master" => Some(String::from(BOX)),
        "note.master" => Some(String::from(NOTE)),
        _ => None,
    };
    stencil::build(&files, m).expect("the test stencil builds")
}

const TEMPLATE: &str = "\
template: Test Template
file: tt
name.de: Testvorlage
name.en: Test template
name.es: Plantilla de prueba
name.it: Modello di prova
name.fr: Modèle de test
description.de: Beschreibung
description.en: Description
description.es: Descripción
description.it: Descrizione
description.fr: Description
category: flowchart
order: 33
stencils: t
registers: cff
rules: rules.txt
starters: demo
default: demo
design: Contour
variant: 2
page1.de: Erste
page1.en: First
page1.es: Primera
page1.it: Prima
page1.fr: Première

[PageSheet]
PageWidth = 210 mm; PageHeight = 297 mm
PageWidth.us = 8.5 in; PageHeight.us = 11 in

[Style Normal]
Char.Size = 8 pt
LineWeight = 2 pt

[Layer]
Row Name.de = \"Notizen\"; Name.en = \"Notes\"; Name.es = \"Notas\"; Name.it = \"Note\"; Name.fr = \"Notes\"
    Print = FALSE

[Pages]
Page-1: BackPage = \"Background-1\"
Background-1: Background = TRUE
    Name.de = \"Hintergrund\"; Name.en = \"Background\"; Name.es = \"Fondo\"; Name.it = \"Sfondo\"; Name.fr = \"Arrière-plan\"
Page-2: Name.de = \"Zweite\"; Name.en = \"Second\"; Name.es = \"Segunda\"; Name.it = \"Seconda\"; Name.fr = \"Deuxième\"\n";

const RULES: &str = "\
ruleset: Tests
name.de: Prüfung
name.en: Check
name.es: Prueba
name.it: Prova
name.fr: Vérification
description.de: d & <x>
description.en: d
description.es: d
description.it: d
description.fr: d

[Rules]
Glued: Target = 0; Filter = \"ROLE()=1\"; Test = \"AND(AGGCOUNT(GLUEDSHAPES(4))=1,AGGCOUNT(GLUEDSHAPES(5))=1)\"
    Category.de = \"Verbindungen\"; Category.en = \"Connectivity\"; Category.es = \"c\"; Category.it = \"c\"; Category.fr = \"c\"
    Description.de = \"Nicht geklebt\"; Description.en = \"Not glued\"; Description.es = \"d\"; Description.it = \"d\"; Description.fr = \"d\"\n";

const DEMO: &str = "\
starter: demo
name.de: Beispiel
name.en: Sample
name.es: Ejemplo
name.it: Esempio
name.fr: Exemple

[Shapes]
a:   Stencil = \"t\"; Master = \"Box\"; PinX = 50 mm; PinY = 200 mm; PinX.us = 2 in; PinY.us = 8 in
b:   Stencil = \"t\"; Master = \"Box\"; PinX = 50 mm; PinY = 150 mm; PinX.us = 2 in; PinY.us = 6 in
     Width = 30 mm; Width.us = 1.5 in
     Text.de = \"Zwei\"; Text.en = \"Two\"; Text.es = \"Dos\"; Text.it = \"Due\"; Text.fr = \"Deux\"
c:   From = \"a\"; To = \"b\"
     Text.de = \"Ja\"; Text.en = \"Yes\"; Text.es = \"Sí\"; Text.it = \"Sì\"; Text.fr = \"Oui\"
tip: Stencil = \"t\"; Master = \"Note\"; Tip = TRUE; PinX = 150 mm; PinY = 250 mm; PinX.us = 6 in; PinY.us = 10 in
     Text.de = \"Tipp\"; Text.en = \"Tip\"; Text.es = \"Consejo\"; Text.it = \"Suggerimento\"; Text.fr = \"Astuce\"\n";

fn files_of<'a>(table: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |n: &str| table.iter().find(|(f, _)| *f == n).map(|(_, t)| String::from(*t))
}

fn build_test(m: Measure, table: &[(&str, &str)]) -> Result<Vec<BuiltTemplate>, SourceError> {
    let st = stencil_doc(m);
    let stencils = move |stem: &str| (stem == "t").then(|| st.doc.clone());
    build(&files_of(table), m, &stencils)
}

const ALL: &[(&str, &str)] = &[("template.txt", TEMPLATE), ("rules.txt", RULES), ("demo.starter", DEMO)];

fn page_named(doc: &Document, name_u: &str) -> PageRef {
    doc.page_by_name_u(name_u).expect("the page")
}

fn num(doc: &Document, sheet: SheetRef, cell: &str) -> Option<f64> {
    let c = doc.sym(cell)?;
    doc.resolve(sheet, &CellAddr::single(c)).num()
}

fn part_text(doc: &Document, ct: &str) -> Option<String> {
    doc.parts.iter().find(|p| doc.str(p.content_type) == ct).map(|p| String::from_utf8_lossy(&p.data).into_owned())
}

#[test]
fn builds_every_form() {
    let forms = build_test(Measure::Metric, ALL).unwrap();
    assert_eq!(forms.len(), 2);
    assert_eq!((forms[0].file.as_str(), forms[0].starter.as_deref()), ("tt", None));
    assert_eq!((forms[1].file.as_str(), forms[1].starter.as_deref()), ("tt-demo", Some("demo")));
    assert_eq!(forms[1].vstx_name(), "tt-demo_m.vstx");
    assert!(forms.iter().all(|f| f.listed && f.order == 33 && f.category == "flowchart"));
    let doc = &forms[0].doc;
    assert_eq!(doc.kind, DocKind::Template);
    // the catalogue's cells
    let i = info(doc);
    assert_eq!(i.template, "Test Template");
    assert_eq!(i.name.as_ref().map(|n| n[3].as_str()), Some("Modello di prova"));
    assert_eq!(i.order, 33);
    assert!(!i.default, "the starter is the default");
    assert_eq!(i.registers, ["cff"]);
    assert!(i.starter.is_none());
    let j = info(&forms[1].doc);
    assert!(j.default);
    assert_eq!(j.starter.as_ref().map(|n| n[1].as_str()), Some("Sample"));
    // pages: names in the build language, portrait, background, the layer on foreground pages
    let p1 = page_named(doc, "Page-1");
    assert_eq!(doc.page(p1).and_then(|p| p.header.name).map(|n| doc.str(n)), Some("Erste"));
    assert_eq!(num(doc, SheetRef::Page(p1), "PrintPageOrientation"), Some(1.0));
    assert!((num(doc, SheetRef::Page(p1), "PageWidth").unwrap() - 210.0 / 25.4).abs() < 1e-9);
    let bg = page_named(doc, "Background-1");
    assert!(doc.page(bg).unwrap().header.background);
    assert_eq!(doc.page(p1).unwrap().header.back_page, Some(doc.page(bg).unwrap().header.id));
    let p2 = page_named(doc, "Page-2");
    assert_eq!(num(doc, SheetRef::Page(p2), "PrintPageOrientation"), Some(1.0), "a copy of the first page sheet");
    assert_eq!(user_value(doc, SheetRef::Page(p2), &format!("{JFD_LAYER}1")).as_deref(), Some("Notizen|Notes|Notas|Note|Notes"));
    assert!(user_value(doc, SheetRef::Page(bg), &format!("{JFD_LAYER}1")).is_none(), "no layers on background pages");
    // the design Contour, variant 2
    assert_eq!(design::apply::page_design(doc, p1), Some((design::catalog::CONTOUR, 1)));
    // the style
    let normal = doc.style_by_name_u("Normal").unwrap();
    let ch = doc.sym("Character").unwrap();
    let size = doc.sym("Size").unwrap();
    let v = doc.resolve(SheetRef::Style(normal), &CellAddr::row(SecKey::one(ch), RowKey::Index(0), size)).num();
    assert_eq!(v, Some(8.0 / 72.0));
    // stencil windows and the rule set
    assert_eq!(stencil_windows(doc), [StencilWindow { document: String::from("t_m.vssx"), active: true }]);
    let v = part_text(doc, CT_VALIDATION).expect("the rule set's part");
    assert!(v.contains("NameU='Tests' Name='Prüfung' Description='d &amp; &lt;x&gt;'"), "{v}");
    assert!(v.contains("<RuleFilter>ROLE()=1</RuleFilter>"), "{v}");
    assert!(crate::write::check(&v).is_ok());
}

#[test]
fn starter_shapes() {
    let forms = build_test(Measure::Metric, ALL).unwrap();
    let doc = &forms[1].doc;
    let p1 = page_named(doc, "Page-1");
    let shapes: Vec<ShapeRef> = doc.top_shapes(Owner::Page(p1)).collect();
    assert_eq!(shapes.len(), 4);
    let a = doc.shape(shapes[0]).unwrap();
    assert_eq!((a.name().as_str(), a.name_u().as_str()), ("Kasten", "Box"));
    let b = doc.shape(shapes[1]).unwrap();
    assert_eq!(b.name_u(), format!("Box.{}", b.id()));
    assert_eq!(b.text().map(|t| t.chars()).as_deref(), Some("Zwei"));
    assert_eq!(user_value(doc, SheetRef::Shape(shapes[1]), instance::JFD_TEXT).as_deref(), Some("Zwei|Two|Dos|Due|Deux"));
    assert!((num(doc, SheetRef::Shape(shapes[1]), "Width").unwrap() - 30.0 / 25.4).abs() < 1e-9);
    // the connector glued to both
    let c = shapes[2];
    assert!(glue::is_connector(doc, c));
    assert_eq!(glue::glued(doc, c), [Target::Shape(shapes[0]), Target::Shape(shapes[1])]);
    // the tip
    assert_eq!(user_value(doc, SheetRef::Shape(shapes[3]), JFD_TIP).as_deref(), Some("1"));
    // nothing stale
    let res = Engine::new().calculate(doc);
    let mut d2 = doc.clone();
    let mut tx = d2.transaction("x");
    assert_eq!(res.write(&mut tx).unwrap(), 0, "every value computed before writing");
}

#[test]
fn same_sources_same_bytes_and_round_trip() {
    for m in [Measure::Metric, Measure::Us] {
        let st = stencil_doc(m);
        let stencils = |stem: &str| (stem == "t").then(|| st.doc.clone());
        let a = build_vstx(&files_of(ALL), m, &stencils).unwrap();
        let b = build_vstx(&files_of(ALL), m, &stencils).unwrap();
        for ((fa, ba), (_, bb)) in a.iter().zip(&b) {
            assert_eq!(ba, bb, "{}: the same bytes", fa.vstx_name());
            let l = vsdx::read(ba, &Limits::VSDX).unwrap();
            assert_eq!(l.format.kind, DocKind::Template);
            let again = vsdx::write(&l.doc, &WriteOptions::DEFAULT).unwrap();
            assert_eq!(&again, ba, "{}: read and written again", fa.vstx_name());
            assert_eq!(stencil_windows(&l.doc), stencil_windows(&fa.doc));
        }
        if m == Measure::Us {
            let p1 = page_named(&a[0].0.doc, "Page-1");
            assert_eq!(num(&a[0].0.doc, SheetRef::Page(p1), "PageWidth"), Some(8.5));
            assert_eq!(stencil_windows(&a[0].0.doc)[0].document, "t_u.vssx");
        }
    }
}

#[test]
fn windows_part_kept_and_replaced() {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    assert!(stencil_windows(&doc).is_empty());
    let list = [
        StencilWindow { document: String::from("a_m.vssx"), active: false },
        StencilWindow { document: String::from("C:\\Shapes\\B & C.vssx"), active: true },
    ];
    set_stencil_windows(&mut doc, &list);
    assert_eq!(stencil_windows(&doc), list);
    let text = part_text(&doc, CT_WINDOWS).unwrap();
    assert!(text.contains("<Window ID='0' WindowType='Drawing'"), "{text}");
    assert!(text.contains("WindowState='67109889' ParentWindow='0' Document='C:\\Shapes\\B &amp; C.vssx'"), "{text}");
    assert_eq!(doc.parts.iter().filter(|p| doc.str(p.content_type) == CT_WINDOWS).count(), 1);
    // a part as Visio writes it: the drawing window and its children stay, the stencil windows
    // come in StencilGroupPos order, new ones replace them
    let visio = "<?xml version='1.0' encoding='utf-8' ?>\r\n<Windows ClientWidth='1920' ClientHeight='1000' xmlns='http://schemas.microsoft.com/office/visio/2012/main' xmlns:r='http://schemas.openxmlformats.org/officeDocument/2006/relationships' xml:space='preserve'><Window ID='3' WindowType='Drawing' WindowState='1073741824' ContainerType='Page' Page='0'><ShowRulers>1</ShowRulers><SnapSettings>65847</SnapSettings></Window><Window ID='5' WindowType='Stencil' WindowState='1025' ParentWindow='3' Document='X:\\Second.vssx'><StencilGroup>10</StencilGroup><StencilGroupPos>2</StencilGroupPos></Window><Window ID='4' WindowType='Stencil' WindowState='67109889' ParentWindow='3' Document='X:\\First.vssx'><StencilGroup>10</StencilGroup><StencilGroupPos>1</StencilGroupPos></Window></Windows>";
    let i = doc.parts.iter().position(|p| doc.str(p.content_type) == CT_WINDOWS).unwrap();
    doc.parts[i].data = visio.as_bytes().to_vec();
    assert_eq!(
        stencil_windows(&doc),
        [StencilWindow { document: String::from("X:\\First.vssx"), active: true }, StencilWindow { document: String::from("X:\\Second.vssx"), active: false }]
    );
    set_stencil_windows(&mut doc, &list[..1]);
    let text = part_text(&doc, CT_WINDOWS).unwrap();
    assert!(text.contains("<Window ID='3' WindowType='Drawing' WindowState='1073741824' ContainerType='Page' Page='0'><ShowRulers>1</ShowRulers><SnapSettings>65847</SnapSettings></Window>"), "{text}");
    assert!(text.contains("ClientWidth='1920'"));
    assert!(text.contains("<Window ID='6' WindowType='Stencil' WindowState='1025' ParentWindow='3' Document='a_m.vssx'>"), "{text}");
    assert!(!text.contains("First.vssx"));
    assert!(crate::write::check(&text).is_ok());
    // through a file
    let bytes = vsdx::write(&doc, &WriteOptions::DEFAULT).unwrap();
    let back = vsdx::read(&bytes, &Limits::VSDX).unwrap().doc;
    assert_eq!(stencil_windows(&back), list[..1]);
    // a broken part: none, and set_stencil_windows writes a part of its own
    doc.parts[i].data = b"<Windows><Window".to_vec();
    assert!(stencil_windows(&doc).is_empty());
    set_stencil_windows(&mut doc, &list);
    assert_eq!(stencil_windows(&doc), list);
}

#[test]
fn a_new_drawing_from_a_template() {
    let forms = build_test(Measure::Metric, ALL).unwrap();
    let bytes = vsdx::write(&forms[1].doc, &WriteOptions::DEFAULT).unwrap();
    let mut doc = vsdx::read(&bytes, &Limits::VSDX).unwrap().doc;
    let windows = stencil_windows(&doc);
    to_drawing(&mut doc, 1);
    assert_eq!(doc.kind, DocKind::Drawing);
    assert_eq!(stencil_windows(&doc), windows, "the windows part stays");
    let p1 = page_named(&doc, "Page-1");
    assert_eq!(doc.page(p1).and_then(|p| p.header.name).map(|n| doc.str(n)), Some("First"));
    assert!(user_value(&doc, SheetRef::Page(p1), instance::JFD_NAME).is_none());
    let bg = page_named(&doc, "Background-1");
    assert_eq!(doc.page(bg).and_then(|p| p.header.name).map(|n| doc.str(n)), Some("Background"));
    // the layer's name in English
    let (layer, name) = (doc.sym("Layer").unwrap(), doc.sym("Name").unwrap());
    let v = doc.resolve(SheetRef::Page(p1), &CellAddr::row(SecKey::one(layer), RowKey::Index(0), name)).value();
    assert_eq!(v.text().map(|t| doc.str(t)), Some("Notes"));
    assert!(user_value(&doc, SheetRef::Page(p1), &format!("{JFD_LAYER}1")).is_none());
    // shapes: names, texts (own and the master's)
    let shapes: Vec<ShapeRef> = doc.top_shapes(Owner::Page(p1)).collect();
    let a = doc.shape(shapes[0]).unwrap();
    assert_eq!((a.name().as_str(), a.name_u().as_str()), ("Box", "Box"));
    assert_eq!(a.text().map(|t| t.chars()).as_deref(), Some("Box"), "the master's text in English");
    let b = doc.shape(shapes[1]).unwrap();
    assert_eq!(b.name(), format!("Box.{}", b.id()));
    assert_eq!(b.text().map(|t| t.chars()).as_deref(), Some("Two"));
    assert!(user_value(&doc, SheetRef::Shape(shapes[1]), instance::JFD_TEXT).is_none());
    assert_eq!(doc.shape(shapes[2]).unwrap().text().map(|t| t.chars()).as_deref(), Some("Yes"));
    assert_eq!(user_value(&doc, SheetRef::Shape(shapes[3]), JFD_TIP).as_deref(), Some("1"), "JFD_Tip stays");
    // the document sheet: the catalogue's cells go, template, registers and rules stay
    let i = info(&doc);
    assert_eq!(i.template, "Test Template");
    assert_eq!(i.registers, ["cff"]);
    assert!(i.name.is_none() && i.description.is_none() && i.starter.is_none() && !i.default && i.category.is_empty() && i.order == 0);
    assert!(user_value(&doc, SheetRef::Document, &format!("{JFD_RULE}1")).is_some());
    let v = part_text(&doc, CT_VALIDATION).unwrap();
    assert!(v.contains("Name='Check'") && v.contains("Category='Connectivity' Description='Not glued'"), "{v}");
    assert!(v.contains("<RuleTest>AND(AGGCOUNT(GLUEDSHAPES(4))=1,AGGCOUNT(GLUEDSHAPES(5))=1)</RuleTest>"), "{v}");
    // a drawing that is written and read again
    let bytes = vsdx::write(&doc, &WriteOptions::DEFAULT).unwrap();
    let back = vsdx::read(&bytes, &Limits::VSDX).unwrap();
    assert_eq!(back.format.kind, DocKind::Drawing);
    assert_eq!(vsdx::write(&back.doc, &WriteOptions::DEFAULT).unwrap(), bytes);
}

#[test]
fn faults_name_file_and_line() {
    let with = |name: &str, text: &str| -> SourceError {
        let mut table: Vec<(&str, &str)> = ALL.iter().copied().filter(|(n, _)| *n != name).collect();
        table.push((name, text));
        let owned: Vec<(String, String)> = table.iter().map(|(a, b)| (String::from(*a), String::from(*b))).collect();
        let refs: Vec<(&str, &str)> = owned.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        build_test(Measure::Metric, &refs).expect_err("a fault")
    };
    let t = |from: &str, to: &str| TEMPLATE.replacen(from, to, 1);
    let line_of = |text: &str, needle: &str| text.lines().position(|l| l.contains(needle)).unwrap() as u32 + 1;
    // template.txt
    let bad = t("order: 33", "order: 33\ncolour: red");
    let e = with("template.txt", &bad);
    assert_eq!((e.file.as_str(), e.line), ("template.txt", line_of(&bad, "colour")), "{e}");
    let e = with("template.txt", &t("name.fr: Modèle de test\n", ""));
    assert!(e.message.contains("name.fr"), "{e}");
    let e = with("template.txt", &t("stencils: t", "stencils: t, nope"));
    assert_eq!(e.line, line_of(TEMPLATE, "stencils:"), "{e}");
    let e = with("template.txt", &t("Char.Size = 8 pt", "Char.Size = 8 pt * 2"));
    assert_eq!(e.line, line_of(TEMPLATE, "Char.Size"), "{e}");
    let e = with("template.txt", &t("Char.Size", "Char.Sise"));
    assert!(e.message.contains("Char.Sise"), "{e}");
    let e = with("template.txt", &t("[Style Normal]", "[Style Fancy]"));
    assert!(e.message.contains("Fancy"), "{e}");
    let e = with("template.txt", &t("default: demo", "default: other"));
    assert_eq!(e.line, line_of(TEMPLATE, "default:"), "{e}");
    let e = with("template.txt", &t("design: Contour", "design: Paisley"));
    assert_eq!(e.line, line_of(TEMPLATE, "design:"), "{e}");
    let e = with("template.txt", &t("registers: cff", "registers: gantt"));
    assert_eq!(e.line, line_of(TEMPLATE, "registers:"), "{e}");
    let e = with("template.txt", &t("Page-1: BackPage = \"Background-1\"", "Page-1: BackPage = \"Page-2\""));
    assert_eq!(e.line, line_of(TEMPLATE, "Page-1: BackPage"), "{e}");
    // the starter
    let e = with("demo.starter", &DEMO.replacen("Master = \"Box\"", "Master = \"Ring\"", 1));
    assert_eq!((e.file.as_str(), e.line), ("demo.starter", line_of(DEMO, "a:")), "{e}");
    let e = with("demo.starter", &DEMO.replacen("To = \"b\"", "To = \"z\"", 1));
    assert_eq!(e.line, line_of(DEMO, "c:"), "{e}");
    let e = with("demo.starter", &DEMO.replacen("Text.fr = \"Deux\"", "", 1));
    assert!(e.message.contains("Text.fr"), "{e}");
    let e = with("demo.starter", &DEMO.replacen("PinX = 50 mm; PinY = 200 mm", "PinX = 50; PinY = 200 mm", 1));
    assert!(e.message.contains("PinX"), "{e}");
    // the rules
    let e = with("rules.txt", &RULES.replacen("Filter = \"ROLE()=1\"", "Filter = \"ROLE(=1\"", 1));
    assert_eq!((e.file.as_str(), e.line), ("rules.txt", line_of(RULES, "Glued:")), "{e}");
    let e = with("rules.txt", &RULES.replacen("Target = 0; ", "", 1));
    assert!(e.message.contains("Target"), "{e}");
    // a file that is not there
    let table: Vec<(&str, &str)> = ALL.iter().copied().filter(|(n, _)| *n != "demo.starter").collect();
    let e = build_test(Measure::Metric, &table).unwrap_err();
    assert_eq!((e.file.as_str(), e.line), ("template.txt", line_of(TEMPLATE, "starters:")), "{e}");
}
