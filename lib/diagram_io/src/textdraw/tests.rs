// tests of textdraw.rs: the guide's example, quoting, separators, comments and headings, numbers
// and units, shape data, hostile texts (bounds, broken records), the layout, building on a page

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::vsdx::{self, Measure};
use diagram_core::{DocKind, Document};

const MM: f64 = 1.0 / 25.4;

// an organisation chart in the form the user guide shows (own content): comment headings, empty
// fields, a space before an ID, the links not in the shapes' order
const GUIDE: &str = concat!(
    ";Shape,ID,Master,Text,X,Y,Width,Height,Property\n",
    "Shape,CHEF,,Geschäftsleitung\n",
    "Shape,EINKAUF,,Einkauf\n",
    "Shape,TECHNIK,,Technik\n",
    "Shape,WERKSTATT,,Werkstatt\n",
    "Shape,LABOR,,Labor\n",
    "\n",
    ";Link,ID,Master,Text,From,To\n",
    "Link,,,,CHEF,TECHNIK\n",
    "Link,,,, CHEF,EINKAUF\n",
    "Link,,,,TECHNIK,WERKSTATT\n",
    "Link,,,,TECHNIK,LABOR\n",
);

fn read(text: &str) -> Plan {
    parse(text, &Options::default(), &Limits::DEFAULT)
}

fn issues(p: &Plan) -> Vec<(u32, Issue, bool)> {
    p.problems.iter().map(|x| (x.line, x.issue.clone(), x.skipped)).collect()
}

#[test]
fn guide_example() {
    let p = read(GUIDE);
    assert_eq!(p.sep, ',');
    assert!(p.sep_detected);
    assert_eq!(p.comments, 2);
    assert!(p.problems.is_empty(), "{:?}", p.problems);
    let texts: Vec<&str> = p.shapes.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(texts, ["Geschäftsleitung", "Einkauf", "Technik", "Werkstatt", "Labor"]);
    assert_eq!(p.shapes[0].line, 2);
    assert!(p.shapes.iter().all(|s| s.pos.is_none() && s.size == [None, None] && s.props.is_empty()));
    let ends: Vec<(usize, usize)> = p.links.iter().map(|l| (l.from, l.to)).collect();
    assert_eq!(ends, [(0, 2), (0, 1), (2, 3), (2, 4)]);
    assert_eq!(p.links[1].line, 10);
    assert_eq!(p.masters, [MasterUse { name: String::new(), link: false, count: 5 }, MasterUse { name: String::new(), link: true, count: 4 }]);
    assert_eq!(p.shapes[0].master, 0);
    assert_eq!(p.links[0].master, 1);
}

#[test]
fn quotes_line_breaks_and_separators() {
    // a tab-separated text with CR LF, quoted fields with separators, doubled quotes and line
    // breaks inside, a quoted record type, spaces around fields
    let text = "\"Shape\"\tA\tProzess\t\"Erste\r\nZeile, \"\"zitiert\"\"\"\r\nShape\t B \t\t  Zweite  \r\n\r\nLink\tL1\tDynamic connector\t\"a\tb\"\tA\tb\r\n";
    let p = read(text);
    assert_eq!((p.sep, p.sep_detected), ('\t', true));
    assert!(p.problems.is_empty(), "{:?}", p.problems);
    assert_eq!(p.shapes.len(), 2);
    assert_eq!(p.shapes[0].text, "Erste\nZeile, \"zitiert\"");
    assert_eq!(p.shapes[1].id, "B");
    assert_eq!(p.shapes[1].text, "Zweite");
    assert_eq!(p.shapes[1].line, 3);
    // IDs without regard to case
    assert_eq!((p.links[0].from, p.links[0].to, p.links[0].line), (0, 1, 5));
    assert_eq!(p.links[0].text, "a\tb");
    assert_eq!(p.masters[0].name, "Prozess");
    assert_eq!(p.masters[1].name, "");
    assert_eq!(p.masters[2], MasterUse { name: String::from("Dynamic connector"), link: true, count: 1 });
    // a semicolon file (German Excel), the separator from the heading; CR alone ends lines too
    let p = read(";Shape;ID\rShape;x;;\"a;b\"\rShape;y;;c\rLink;;;;x;y");
    assert_eq!((p.sep, p.sep_detected), (';', true));
    assert_eq!(p.shapes[0].text, "a;b");
    assert_eq!(p.links.len(), 1);
    // Excel's "sep=" line: the separator, no record, the lines counted on
    let p = read("SEP=|\r\nShape|a|;x\nShape|b\n");
    assert_eq!((p.sep, p.sep_detected), ('|', true));
    assert!(p.problems.is_empty(), "{:?}", p.problems);
    assert_eq!((p.shapes[0].master, p.shapes[1].line), (0, 3));
    assert_eq!(p.masters[0].name, ";x");
    assert_eq!(detect_sep("sep=;\nShape,a", ','), (';', true));
    // nothing to detect: the fallback; a given separator wins
    assert_eq!(detect_sep("nothing here", '\t'), ('\t', false));
    let p = parse("Shape,a;b", &Options { sep: Some(';'), ..Options::default() }, &Limits::DEFAULT);
    assert_eq!((p.sep, p.sep_detected), (';', false));
    assert_eq!(p.problems.len(), 1);
    // a BOM before the first record
    let p = read("\u{feff}Shape,a,,x");
    assert_eq!(p.shapes.len(), 1);
    assert!(p.problems.is_empty());
    // text after a closing quote is kept; a quote inside an unquoted field is a character
    let p = read("Shape,a,,\"x\"y ,\nShape,b,,say \"hi\"\n");
    assert_eq!(p.shapes[0].text, "xy");
    assert_eq!(p.shapes[1].text, "say \"hi\"");
}

#[test]
fn numbers_units_and_positions() {
    let o = Options { per_number: MM, ..Options::default() };
    let p = parse("Shape,a,,,10,20,30,15\nShape,b,,,1 in,2.54cm,\"2,5\",36pt\nShape,c,,,0.1 m,1e2,,\nShape,d,,,5,,\nShape,e,,,x,1,-3,0\nShape,f,,,1e9,1,inf,4 parsec\n", &o, &Limits::DEFAULT);
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let s = &p.shapes;
    let pos = s[0].pos.unwrap();
    assert!(near(pos[0], 10.0 * MM) && near(pos[1], 20.0 * MM));
    assert!(near(s[0].size[0].unwrap(), 30.0 * MM) && near(s[0].size[1].unwrap(), 15.0 * MM));
    let pos = s[1].pos.unwrap();
    assert!(near(pos[0], 1.0) && near(pos[1], 1.0));
    assert!(near(s[1].size[0].unwrap(), 2.5 * MM) && near(s[1].size[1].unwrap(), 0.5));
    let pos = s[2].pos.unwrap();
    assert!(near(pos[0], 100.0 * MM) && near(pos[1], 100.0 * MM));
    // X without Y, numbers that do not read, a size <= 0, out of range: warnings, drawn anyway
    assert_eq!(s.len(), 6);
    assert!(s[3].pos.is_none() && s[4].pos.is_none() && s[5].pos.is_none());
    assert_eq!(s[4].size, [None, None]);
    let i = issues(&p);
    assert!(i.contains(&(4, Issue::HalfPosition, false)));
    assert!(i.contains(&(5, Issue::BadNumber(NumField::X, String::from("x")), false)));
    assert!(i.contains(&(5, Issue::OutOfRange(NumField::Width, String::from("-3")), false)));
    assert!(i.contains(&(5, Issue::OutOfRange(NumField::Height, String::from("0")), false)));
    assert!(i.contains(&(5, Issue::HalfPosition, false)));
    assert!(i.contains(&(6, Issue::OutOfRange(NumField::X, String::from("1e9")), false)));
    assert!(i.contains(&(6, Issue::BadNumber(NumField::Width, String::from("inf")), false)));
    assert!(i.contains(&(6, Issue::BadNumber(NumField::Height, String::from("4 parsec")), false)));
    assert_eq!(p.errors(), 0);
    // plain numbers for shape data
    assert_eq!(plain_number("12"), Some(12.0));
    assert_eq!(plain_number("-12,5"), Some(-12.5));
    assert_eq!(plain_number("0.25"), Some(0.25));
    for t in ["0049", "+49", "1e3", "555-4300", "12.", ".5", "", "-", "1.2.3", "12 mm"] {
        assert_eq!(plain_number(t), None, "{t}");
    }
}

#[test]
fn shape_data_fields() {
    let text = concat!(
        "Shape,a,,A,,,,,Kosten=12,Owner = Jakob,frei,http://x.y/?a=b,,=leer\n",
        ";Shape,ID,Master,Text,X,Y,Width,Height,Abteilung,Property,Telefon\n",
        "Shape,b,,B,,,,,Finanzen,ohne Kopf,555-4300,Abteilung=doppelt\n",
    );
    let p = read(text);
    assert!(p.problems.is_empty(), "{:?}", p.problems);
    let a: Vec<(&str, &str, &str)> = p.shapes[0].props.iter().map(|x| (x.name.as_str(), x.label.as_str(), x.value.as_str())).collect();
    assert_eq!(a, [("Kosten", "Kosten", "12"), ("Owner", "Owner", "Jakob"), ("Property3", "Property3", "frei"), ("Property4", "Property4", "http://x.y/?a=b"), ("Property6", "Property6", "=leer")]);
    let b: Vec<(&str, &str, &str)> = p.shapes[1].props.iter().map(|x| (x.name.as_str(), x.label.as_str(), x.value.as_str())).collect();
    assert_eq!(b, [("Abteilung", "Abteilung", "Finanzen"), ("Property2", "Property2", "ohne Kopf"), ("Telefon", "Telefon", "555-4300"), ("Abteilung2", "Abteilung", "doppelt")]);
    assert_eq!(row_name("IP-Adresse 2"), "IP_Adresse_2");
    assert_eq!(row_name("1st"), "_1st");
    assert_eq!(row_name(""), "_");
}

#[test]
fn broken_records() {
    let text = concat!(
        "Shape,a,,A\n",
        "Shape,A,,again\n",
        "Shape,,,no id\n",
        "Rect,x,,?\n",
        "Link,l1,,,a,zz\n",
        "Link,l1,,,a,a\n",
        "Link,l2,,,a,\n",
        "Link,,,,zz,a\n",
        "Link,l1,,,b,a\n",
        "Shape,b,,B\n",
        "Link,dup,,,a,b\n",
        "Link,DUP,,,b,a\n",
    );
    let p = read(text);
    let i = issues(&p);
    assert_eq!(
        i,
        [
            (2, Issue::DuplicateId(String::from("A")), true),
            (3, Issue::NoId, false),
            (4, Issue::UnknownRecord(String::from("Rect")), true),
            (5, Issue::UnknownEnd(String::from("zz")), true),
            (6, Issue::SelfLink(String::from("a")), true),
            (7, Issue::NoEnd, true),
            (8, Issue::UnknownEnd(String::from("zz")), true),
            (12, Issue::DuplicateLinkId(String::from("DUP")), false),
        ]
    );
    // a shape without an ID is drawn; the link before its target's record is resolved
    assert_eq!(p.shapes.len(), 3);
    assert_eq!(p.links.len(), 3);
    assert_eq!((p.links[0].from, p.links[0].to), (2, 0));
    assert_eq!(p.errors(), 6);
    assert_eq!(p.warnings(), 2);
    // a quote left open swallows the rest: that record is an error
    let p = read("Shape,a,,ok\nShape,b,,\"open\nShape,c,,lost\n");
    assert_eq!(p.shapes.len(), 1);
    assert_eq!(issues(&p), [(2, Issue::UnclosedQuote, true)]);
    // empty and separator-only lines are no records
    let p = read("\n\n,,,\n   \n\t\nShape,a\n");
    assert_eq!(p.shapes.len(), 1);
    assert!(p.problems.is_empty(), "{:?}", p.problems);
}

#[test]
fn hostile_texts_stay_bounded() {
    let lim = Limits { max_bytes: 4096, max_records: 3, max_fields: 12, max_field_chars: 16, max_id_chars: 8, max_props: 2, max_problems: 4 };
    // too big: nothing read
    let big = "Shape,a\n".repeat(600);
    let p = parse(&big, &Options::default(), &lim);
    assert!(p.shapes.is_empty());
    assert_eq!(issues(&p), [(0, Issue::TooBig, true)]);
    // too many records: the rest left out
    let p = parse("Shape,a\nShape,b\nShape,c\nShape,d\nShape,e\n", &Options::default(), &lim);
    assert_eq!(p.shapes.len(), 3);
    assert_eq!(issues(&p), [(4, Issue::TooManyRecords, true)]);
    // a long field is cut, a long ID refused, too many fields and shape data fields cut
    let long = "x".repeat(100);
    let text = format!("Shape,a,,{long}\nShape,{long}\nShape,b,,,,,,,1,2,3,4,5,6,7,8,9\n");
    let p = parse(&text, &Options::default(), &lim);
    assert_eq!(p.shapes[0].text.chars().count(), 16);
    assert_eq!(p.shapes.len(), 2);
    assert_eq!(p.shapes[1].props.len(), 2);
    let i = issues(&p);
    assert!(i.contains(&(1, Issue::FieldTooLong, false)));
    assert!(i.contains(&(2, Issue::IdTooLong, true)));
    assert!(i.contains(&(3, Issue::TooManyFields, false)));
    // the problems are bounded too
    let lots = "Bad\n".repeat(20);
    let p = parse(&lots, &Options::default(), &lim);
    assert_eq!(p.problems.len(), 5);
    assert_eq!(p.problems[4].issue, Issue::TooManyProblems);
    // control characters, invalid-looking text, a NUL: read as text, no panic
    let p = read("Shape,\u{0}a\u{1},,\u{202e}evil\u{0}\nLink,,,,\u{0}a\u{1},\u{0}a\u{1}\n\"\"\"\n");
    assert_eq!(p.shapes.len(), 1);
    assert_eq!(p.links.len(), 0);
    // only quotes, only separators, a lone CR: nothing and no panic
    for t in ["\"", "\"\"", ",,,,", "\r", ";", "Shape", "Link", "Link,", "\"Shape"] {
        let _ = read(t);
    }
}

// the middles of a layout's boxes, the box of index k
fn boxes(c: &[[f64; 2]], sizes: &[[f64; 2]]) -> Vec<[f64; 4]> {
    c.iter().zip(sizes).map(|(m, s)| [m[0] - s[0] * 0.5, m[1] - s[1] * 0.5, m[0] + s[0] * 0.5, m[1] + s[1] * 0.5]).collect()
}

fn overlap(b: &[[f64; 4]]) -> bool {
    for i in 0..b.len() {
        for j in i + 1..b.len() {
            if b[i][0] < b[j][2] - 1e-9 && b[j][0] < b[i][2] - 1e-9 && b[i][1] < b[j][3] - 1e-9 && b[j][1] < b[i][3] - 1e-9 {
                return true;
            }
        }
    }
    false
}

#[test]
fn layout_of_the_guide_example() {
    let p = read(GUIDE);
    let sizes = vec![[1.0, 0.5]; 5];
    let edges: Vec<(usize, usize)> = p.links.iter().map(|l| (l.from, l.to)).collect();
    let c = layout(&sizes, &edges, [0.4, 0.6]);
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    // three layers: Geschäftsleitung; Einkauf, Technik; Werkstatt, Labor
    assert!(near(c[0][1], 0.25));
    assert!(near(c[1][1], 1.35) && near(c[2][1], 1.35));
    assert!(near(c[3][1], 2.45) && near(c[4][1], 2.45));
    // Einkauf left of Technik (file order, not the links' order), Werkstatt and Labor centred
    // under Technik, Geschäftsleitung over its two
    assert!(c[1][0] < c[2][0]);
    assert!(near((c[3][0] + c[4][0]) * 0.5, c[2][0]));
    assert!(near((c[1][0] + c[2][0]) * 0.5, c[0][0]));
    assert!(near(c[4][0] - c[3][0], 1.4));
    let b = boxes(&c, &sizes);
    assert!(!overlap(&b));
    assert!(near(b.iter().map(|x| x[0]).fold(f64::INFINITY, f64::min), 0.0));
}

#[test]
fn layout_cycles_groups_and_long_chains() {
    // a cycle a -> b -> c -> a and a pair d -> e, a single f, all of different sizes
    let sizes = [[1.0, 0.5], [2.0, 0.5], [0.5, 1.0], [1.0, 1.0], [1.0, 0.5], [0.3, 0.3]];
    let c = layout(&sizes, &[(0, 1), (1, 2), (2, 0), (3, 4), (5, 5), (9, 1)], [0.25, 0.5]);
    assert!(c[0][1] < c[1][1] && c[1][1] < c[2][1]);
    assert!(c[3][1] < c[4][1]);
    assert!(!overlap(&boxes(&c, &sizes)));
    // many single shapes: rows, not one long line
    let sizes = vec![[1.0, 0.5]; 30];
    let c = layout(&sizes, &[], [0.4, 0.6]);
    let rows = {
        let mut ys: Vec<i64> = c.iter().map(|m| (m[1] * 1000.0) as i64).collect();
        ys.sort_unstable();
        ys.dedup();
        ys.len()
    };
    assert!((4..=8).contains(&rows), "{rows} rows");
    assert!(!overlap(&boxes(&c, &sizes)));
    // a chain of 5 000 shapes and a full fan: no recursion, no overlap in the fan
    let n = 5000;
    let edges: Vec<(usize, usize)> = (1..n).map(|i| (i - 1, i)).collect();
    let c = layout(&vec![[1.0, 0.5]; n], &edges, [0.4, 0.6]);
    assert!(c[n - 1][1] > c[0][1]);
    let sizes = vec![[1.0, 0.5]; 200];
    let edges: Vec<(usize, usize)> = (1..200).map(|i| (0, i)).chain((1..199).map(|i| (i, i + 1))).collect();
    let c = layout(&sizes, &edges, [0.4, 0.6]);
    assert!(!overlap(&boxes(&c, &sizes)));
    assert!(layout(&[], &[(0, 1)], [0.4, 0.6]).is_empty());
}

// a stencil with a 2-D box (shape data Cost, label "Kosten") and a 1-D line
fn stencil() -> Document {
    const ST: &str = "stencil: Test\nfile: test\nname.de: T\nname.en: T\nname.es: T\nname.it: T\nname.fr: T\ncategory: general\nmasters: box, line\n";
    let names = |n: &str| format!("name.de: {n}\nname.en: {n}\nname.es: {n}\nname.it: {n}\nname.fr: {n}\nprompt.de: P\nprompt.en: P\nprompt.es: P\nprompt.it: P\nprompt.fr: P\n");
    let bx = format!(
        "master: Box\n{}\n[Shape]\nWidth = 40 mm\nHeight = 20 mm\n\n[Geometry]\nMoveTo  X = 0; Y = 0\nLineTo  X = Width; Y = 0\nLineTo  X = Width; Y = Height\nLineTo  X = 0; Y = Height\nLineTo  X = Geometry1.X1; Y = Geometry1.Y1\n\n[Property]\nCost:  Type = 2; Value = 0\n       Label.de = \"Kosten\"\n       Label.en = \"Kosten\"\n       Label.es = \"Kosten\"\n       Label.it = \"Kosten\"\n       Label.fr = \"Kosten\"\n",
        names("Kasten")
    );
    let line = format!("master: Line\n{}\n[Shape]\nBeginX = 0 mm\nBeginY = 5 mm\nEndX = 40 mm\nEndY = 5 mm\nHeight = 0 mm\nGlueType = 2\n\n[Geometry]\nMoveTo  X = 0; Y = 0\nLineTo  X = Width; Y = 0\n", names("Linie"));
    let files = move |f: &str| match f {
        "stencil.txt" => Some(String::from(ST)),
        "box.master" => Some(bx.clone()),
        "line.master" => Some(line.clone()),
        _ => None,
    };
    crate::stencil::build(&files, Measure::Metric).expect("the test stencil builds").doc
}

fn text_of(doc: &Document, s: ShapeRef) -> String {
    doc.shape(s).and_then(|x| x.text()).map(|t| t.chars()).unwrap_or_default()
}

fn master_of(doc: &Document, s: ShapeRef) -> Option<MasterRef> {
    doc.shape(s).and_then(|x| x.header().master).and_then(|id| doc.master_by_id(id))
}

fn prop(doc: &Document, s: ShapeRef, name: &str) -> Option<(Option<String>, Value)> {
    let row = doc.shape_data(SheetRef::Shape(s)).into_iter().find(|r| r.name() == Some(name))?;
    Some((row.text("Label").map(String::from), row.get("Value").value()))
}

#[test]
fn builds_shapes_connectors_and_data_on_a_page() {
    let st = stencil();
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().expect("a page");
    let text = concat!(
        ";Shape,ID,Master,Text,X,Y,Width,Height,Kosten,Ort\n",
        "Shape,s1,Kasten,Start,,,,,12,Halle 1\n",
        "Shape,s2,,\"Zwei\nZeilen\",,,50,,Kosten=7\n",
        "Shape,s3,Box,Drei,100,50,30,15\n",
        "Shape,s4,Linie,falsch\n",
        "Link,l1,,ja,s1,s2\n",
        "Link,l2,Line,,s2,s3\n",
        "Link,l3,Kasten,,s1,s3\n",
    );
    let plan = parse(text, &Options { per_number: MM, ..Options::default() }, &Limits::DEFAULT);
    assert!(plan.problems.is_empty(), "{:?}", plan.problems);
    let names: Vec<(&str, bool)> = plan.masters.iter().map(|m| (m.name.as_str(), m.link)).collect();
    assert_eq!(names, [("Kasten", false), ("", false), ("Box", false), ("Linie", false), ("", true), ("Line", true), ("Kasten", true)]);
    let mut eng = Engine::new();
    let mut tx = doc.transaction("Zeichnung aus Text");
    let bx = st.masters().next().expect("box");
    let ln = st.masters().nth(1).expect("line");
    let bx = instance::import_master(&mut tx, &st, bx).expect("import box");
    let ln = instance::import_master(&mut tx, &st, ln).expect("import line");
    // Kasten and Box: the box; Linie (a 1-D master for a shape) and Kasten for a link do not fit
    let masters = [Some(bx), None, Some(bx), Some(ln), None, Some(ln), Some(bx)];
    let opts = Build { default_size: [30.0 * MM, 15.0 * MM], gap: [10.0 * MM, 15.0 * MM], margin: 10.0 * MM, lang: 0 };
    let built = apply(&mut tx, &mut eng, page, &plan, &masters, &opts).expect("built");
    tx.commit();
    assert_eq!(built.shapes.len(), 4);
    assert_eq!(built.connectors.len(), 3);
    assert_eq!(built.unfit, [3, 6]);
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let [s1, s2, s3, s4] = [built.shapes[0], built.shapes[1], built.shapes[2], built.shapes[3]];
    // texts, sizes
    assert_eq!(text_of(&doc, s1), "Start");
    assert_eq!(text_of(&doc, s2), "Zwei\nZeilen");
    let f1 = ops::frame(&doc, s1);
    assert!(near(f1.w, 40.0 * MM) && near(f1.h, 20.0 * MM), "the master's size");
    let f2 = ops::frame(&doc, s2);
    assert!(near(f2.w, 50.0 * MM) && near(f2.h, 15.0 * MM), "the record's width, the default height");
    let f3 = ops::frame(&doc, s3);
    assert!(near(f3.w, 30.0 * MM) && near(f3.h, 15.0 * MM));
    let c3 = f3.centre();
    assert!(near(c3[0], 100.0 * MM) && near(c3[1], 50.0 * MM), "placed by X and Y");
    // s4: Linie does not fit a shape, a plain rectangle of the default size
    let f4 = ops::frame(&doc, s4);
    assert!(near(f4.w, 30.0 * MM) && near(f4.h, 15.0 * MM));
    assert_eq!(master_of(&doc, s4), None);
    assert_eq!(master_of(&doc, s1), Some(bx));
    // laid out: right of the placed s3, top at the page's top margin, s2 below s1
    let [_, ph] = pagesetup::size(&doc, page);
    let b1 = f1.bounds();
    assert!(b1[0] >= c3[0] + 15.0 * MM + 10.0 * MM - 1e-6);
    assert!(near(b1[3], ph - 10.0 * MM));
    assert!(ops::frame(&doc, s2).centre()[1] < f1.centre()[1]);
    // shape data: the master's field by its label, new fields with Type 2 for numbers
    let (label, v) = prop(&doc, s1, "Cost").expect("Cost");
    assert_eq!(label.as_deref(), Some("Kosten"));
    assert_eq!(v, Value::Num(12.0));
    assert!(doc.shape_data(SheetRef::Shape(s1)).iter().all(|r| r.name() != Some("Kosten")));
    let (label, v) = prop(&doc, s1, "Ort").expect("Ort");
    assert_eq!(label.as_deref(), Some("Ort"));
    assert!(matches!(v, Value::Text(t) if doc.str(t) == "Halle 1"));
    let row = doc.shape_data(SheetRef::Shape(s2)).into_iter().find(|r| r.name() == Some("Kosten")).expect("Kosten");
    assert_eq!(row.num("Value"), Some(7.0));
    assert_eq!(row.num("Type"), Some(2.0));
    // the connectors: glued to the whole shapes at both ends; l2 an instance of the line, l1
    // and l3 (Kasten does not fit a link) dynamic connectors; l1's text
    let [c1, c2, c3c] = [built.connectors[0], built.connectors[1], built.connectors[2]];
    assert_eq!(glue::glued(&doc, c1), [glue::Target::Shape(s1), glue::Target::Shape(s2)]);
    assert_eq!(glue::glued(&doc, c2), [glue::Target::Shape(s2), glue::Target::Shape(s3)]);
    assert_eq!(glue::glued(&doc, c3c), [glue::Target::Shape(s1), glue::Target::Shape(s3)]);
    assert!(glue::is_connector(&doc, c1));
    assert_eq!(master_of(&doc, c2), Some(ln));
    assert_eq!(text_of(&doc, c1), "ja");
    let b = built.bounds.expect("bounds");
    assert!(b[0] < b[2] && b[1] < b[3]);
}

#[test]
fn builds_beside_earlier_shapes_and_rolls_back() {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().expect("a page");
    let mut eng = Engine::new();
    // an earlier shape at the left
    let earlier = {
        let mut tx = doc.transaction("Rechteck");
        let s = ops::new_shape(&mut tx, &mut eng, Owner::Page(page), ops::Basic::Rectangle, [0.5, 5.0], [3.0, 6.0]).expect("rect");
        tx.commit();
        s
    };
    let plan = read(GUIDE);
    let mut tx = doc.transaction("Zeichnung aus Text");
    let built = apply(&mut tx, &mut eng, page, &plan, &[], &Build::default()).expect("built");
    let doc_now = tx.doc();
    for &s in &built.shapes {
        assert!(ops::frame(doc_now, s).bounds()[0] >= 3.0 + 0.4 - 1e-9);
    }
    // the tree: Geschäftsleitung over Einkauf and Technik, Technik over Werkstatt and Labor
    let c: Vec<[f64; 2]> = built.shapes.iter().map(|&s| ops::frame(doc_now, s).centre()).collect();
    assert!(c[0][1] > c[1][1] && (c[1][1] - c[2][1]).abs() < 1e-9 && c[2][1] > c[3][1]);
    assert!(((c[3][0] + c[4][0]) * 0.5 - c[2][0]).abs() < 1e-6);
    assert_eq!(built.connectors.len(), 4);
    let count = doc_now.top_shapes(Owner::Page(page)).count();
    assert_eq!(count, 10);
    // taken back: only the earlier shape remains
    tx.rollback();
    let left: Vec<ShapeRef> = doc.top_shapes(Owner::Page(page)).filter(|&s| doc.shape(s).is_some_and(|x| !x.is_deleted())).collect();
    assert_eq!(left, [earlier]);
}
