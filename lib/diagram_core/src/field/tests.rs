// tests of the text fields (cargo test -p jflib-diagram-core --lib field)
//
//   format pictures for numbers (units, groups, decimals, sections, percent), dates and texts in
//   two locales; a new field shows its value at once, in the text the renderer lays out, and
//   follows the cell it reads (a resized shape, undo and redo); page and shape information;
//   the document's properties, the clock and the file (NOW, CREATOR, FILENAME ...); fields
//   count in TEXTHEIGHT; positions inside the text; Visio's fields with their text inside
//
// Every document is built here; nothing comes from a Visio file.

use alloc::string::String;
use alloc::vec::Vec;

use super::*;
use crate::formula::{Input, Outcome, Val};
use crate::render::{build, ApproxMeasure, ItemKind, Options};
use crate::{DocKind, Owner, Page, PageHeader, PageRef, Part};

fn de() -> Env {
    Env {
        decimal: ',',
        group: '.',
        months: ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"],
        days: ["Sonntag", "Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag"],
        date: "dd.MM.yyyy",
        ..Env::default()
    }
}

fn num(x: f64, u: Option<Unit>, pic: &str, env: &Env) -> String {
    format(&Shown::Num(x, u, ), pic, env)
}

#[test]
fn numbers() {
    let en = Env::default();
    let de = de();
    // a length in its cell's unit
    assert_eq!(num(1.0, Some(Unit::Millimeters), "0.00 u", &en), "25.40 mm");
    assert_eq!(num(1.0, Some(Unit::Millimeters), "0.00 u", &de), "25,40 mm");
    assert_eq!(num(1.0, Some(Unit::Millimeters), "0 u", &de), "25 mm");
    assert_eq!(num(1.0, Some(Unit::Millimeters), "0.# U", &en), "25.4 MM");
    assert_eq!(num(1.0, Some(Unit::Millimeters), "0.00", &en), "25.40");
    assert_eq!(num(2.5, Some(Unit::Inches), "0.0u", &en), "2.5in");
    // an angle in degrees
    assert_eq!(num(core::f64::consts::FRAC_PI_2, Some(Unit::Degrees), "0 u", &de), "90 °");
    // groups, optional decimals, leading zeros
    assert_eq!(num(12345.678, None, "#,##0.0", &en), "12,345.7");
    assert_eq!(num(12345.678, None, "#,##0.0", &de), "12.345,7");
    assert_eq!(num(3.5, None, "0.##", &en), "3.5");
    assert_eq!(num(3.0, None, "0.##", &en), "3");
    assert_eq!(num(7.0, None, "000", &en), "007");
    assert_eq!(num(0.25, None, "#.00", &en), ".25");
    // signs and sections
    assert_eq!(num(-2.0, None, "0.0", &en), "-2.0");
    assert_eq!(num(-2.0, None, "0.0;(0.0)", &en), "(2.0)");
    assert_eq!(num(0.0, None, "0.0;(0.0);\"null\"", &en), "null");
    assert_eq!(num(-0.0001, None, "0.0", &en), "0.0");
    // percent, literal text
    assert_eq!(num(0.125, None, "0.0%", &en), "12.5%");
    assert_eq!(num(4.0, None, "\"Nr. \"0", &de), "Nr. 4");
    // the general format
    assert_eq!(num(1.0, Some(Unit::Millimeters), "", &de), "25,4 mm");
    assert_eq!(num(5.0, None, "", &en), "5");
    assert_eq!(num(1.0 / 3.0, None, "", &en), "0.3333");
    assert_eq!(num(1.0, Some(Unit::Bool), "", &en), "TRUE");
}

#[test]
fn dates() {
    let en = Env::default();
    let de = de();
    // Thursday, 19 November 2026, 14:05:09
    let d = ole_days(2026, 11, 19, 14, 5, 9);
    assert_eq!(civil(d), (2026, 11, 19, 14, 5, 9, 4));
    assert_eq!(ole_days(1899, 12, 30, 0, 0, 0), 0.0);
    assert_eq!(ole_days(1970, 1, 1, 0, 0, 0), 25_569.0);
    assert_eq!(format(&Shown::Num(d, Some(Unit::Date)), "{{dd.MM.yyyy}}", &de), "19.11.2026");
    assert_eq!(format(&Shown::Num(d, Some(Unit::Date)), "{{dddd, d. MMMM yyyy}}", &de), "Donnerstag, 19. November 2026");
    assert_eq!(format(&Shown::Num(d, Some(Unit::Date)), "{{ddd, MMM d}}", &en), "Thu, Nov 19");
    assert_eq!(format(&Shown::Num(d, Some(Unit::Date)), "{{HH:mm:ss}}", &de), "14:05:09");
    assert_eq!(format(&Shown::Num(d, Some(Unit::Date)), "{{h:mm tt}}", &en), "2:05 PM");
    assert_eq!(format(&Shown::Num(d, Some(Unit::Date)), "am {{d.M.yy}}", &de), "am 19.11.26");
    assert_eq!(format(&Shown::Num(d, Some(Unit::Date)), "{{yyyy-MM-dd'T'HH}}", &en), "2026-11-19T14");
    // the general format of a date
    assert_eq!(format(&Shown::Num(d, Some(Unit::Date)), "", &en), "11/19/2026");
    assert_eq!(format(&Shown::Num(d, Some(Unit::Date)), "", &de), "19.11.2026");
    // late in a day rounds into the next one
    assert_eq!(civil(ole_days(2026, 12, 31, 23, 59, 59) + 0.9 / 86_400.0).0, 2027);
}

#[test]
fn texts() {
    let en = Env::default();
    assert_eq!(format(&Shown::Text("Ablauf"), "@", &en), "Ablauf");
    assert_eq!(format(&Shown::Text("Ablauf"), "@+", &en), "ABLAUF");
    assert_eq!(format(&Shown::Text("Ablauf"), "@-", &en), "ablauf");
    assert_eq!(format(&Shown::Text("Ablauf"), "", &en), "Ablauf");
    assert_eq!(format(&Shown::Text("Ablauf"), "\"Seite: \"@", &en), "Seite: Ablauf");
    assert_eq!(format(&Shown::Nothing, "0.0", &en), "");
    assert_eq!(cached("<fld IX='0'>12.11.2026 &amp; mehr</fld>").as_deref(), Some("12.11.2026 & mehr"));
    assert_eq!(cached("<fld IX='1'/>").as_deref(), Some(""));
    assert_eq!(cached("<x/>"), None);
}

// ---- documents

struct Fx {
    doc: Document,
    eng: Engine,
    page: PageRef,
    shape: ShapeRef,
}

fn fx(text: Option<&str>) -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    let mut ph = PageHeader { id: 0, ..PageHeader::default() };
    ph.name_u = Some(doc.intern("Page-1"));
    ph.name = Some(doc.intern("Ablauf"));
    let page = doc.insert_page(Page::new(ph)).unwrap();
    let mut eng = Engine::new();
    let mut tx = doc.transaction("Rechteck");
    let shape = crate::ops::new_shape(&mut tx, &mut eng, Owner::Page(page), crate::ops::Basic::Rectangle, [1.0, 1.0], [3.0, 2.0]).unwrap();
    if let Some(t) = text {
        crate::ops::set_text(&mut tx, &mut eng, shape, Text::plain(t)).unwrap();
    }
    tx.commit();
    Fx { doc, eng, page, shape }
}

impl Fx {
    fn add(&mut self, at: Option<usize>, spec: &Spec) -> (u32, crate::Change) {
        let mut tx = self.doc.transaction("Feld");
        let ix = insert(&mut tx, &mut self.eng, self.shape, at, spec).unwrap();
        (ix, tx.commit().unwrap())
    }

    fn text(&self) -> String {
        plain(&self.doc, self.shape)
    }

    // the text the renderer lays out
    fn drawn(&self) -> String {
        let d = build(&self.doc, self.page, &mut ApproxMeasure, &Options::default());
        let mut s = String::new();
        for it in &d.items {
            if let ItemKind::Text(t) = &it.kind {
                for r in &t.runs {
                    s.push_str(&r.text);
                }
            }
        }
        s
    }
}

const WIDTH_MM: Spec = Spec { formula: "Width", format: "0.0 u", unit: Some(Unit::Millimeters), kind: Kind::Number };

#[test]
fn a_new_field_shows_its_value_and_follows() {
    let mut f = fx(Some("Breite\n"));
    let (ix, mut change) = f.add(None, &WIDTH_MM);
    assert_eq!(ix, 0);
    // on a line of its own, in front of the end mark; the Field row as Visio keeps it
    assert_eq!(f.text(), "Breite\n50.8 mm\n");
    let sheet = SheetRef::Shape(f.shape);
    let v = f.doc.resolve(sheet, &field_addr(0, names::VALUE));
    assert_eq!(v.formula(), crate::Formula::Expr(f.doc.sym("Width").unwrap()));
    assert_eq!(v.unit(), Some(Unit::Millimeters));
    assert_eq!(picture(&f.doc, sheet, 0), "0.0 u");
    assert_eq!(f.doc.resolve(sheet, &field_addr(0, names::TYPE)).num(), Some(2.0));
    assert!(f.drawn().contains("50.8 mm"), "{}", f.drawn());
    // the shape gets wider: the field follows
    let mut tx = f.doc.transaction("Breite");
    let w = tx.single("Width");
    f.eng.user_set(&mut tx, sheet, &w, Input::Length(3.0, Unit::Inches)).unwrap();
    let mut wider = tx.commit().unwrap();
    assert_eq!(f.text(), "Breite\n76.2 mm\n");
    assert!(f.drawn().contains("76.2 mm"));
    // undo and redo
    assert!(wider.undo(&mut f.doc));
    assert_eq!(f.text(), "Breite\n50.8 mm\n");
    assert!(change.undo(&mut f.doc));
    assert_eq!(f.text(), "Breite\n");
    assert!(f.doc.rows(sheet, SecKey::one(names::FIELD)).is_empty());
    assert!(change.redo(&mut f.doc));
    assert_eq!(f.text(), "Breite\n50.8 mm\n");
}

#[test]
fn fields_at_a_position_and_in_an_empty_text() {
    let mut f = fx(Some("Seite  von \n"));
    let page = Spec { formula: "PAGENUMBER()", format: "0", unit: None, kind: Kind::Number };
    let count = Spec { formula: "PAGECOUNT()", format: "0", unit: None, kind: Kind::Number };
    f.add(Some(6), &page);
    assert_eq!(f.text(), "Seite 1 von \n");
    // a position after the first field: it counts with the text it shows
    f.add(Some(12), &count);
    assert_eq!(f.text(), "Seite 1 von 1\n");
    // a position inside a field goes behind it
    let name = Spec { formula: "PAGENAME()", format: "@+", unit: None, kind: Kind::Text };
    f.add(Some(7), &name);
    assert_eq!(f.text(), "Seite 1ABLAUF von 1\n");
    let segs = segments(&f.doc, f.shape);
    assert_eq!(segs.len(), 6, "{segs:?}");
    // an empty text: the field alone
    let mut g = fx(None);
    g.add(None, &Spec { formula: "ID()", format: "\"#\"0", unit: None, kind: Kind::Number });
    assert_eq!(g.text(), alloc::format!("#{}", g.doc.shape(g.shape).unwrap().id()));
}

#[test]
fn shape_and_custom_fields() {
    let mut f = fx(None);
    f.add(None, &Spec { formula: "NAME()", format: "@", unit: None, kind: Kind::Text });
    let name = f.doc.shape(f.shape).unwrap().name();
    assert_eq!(f.text(), name);
    let mut g = fx(None);
    g.add(None, &Spec { formula: "\"Fläche: \"&Width*Height*645.16&\" mm²\"", format: "", unit: None, kind: Kind::Text });
    assert!(g.text().starts_with("Fläche: 1290.3"), "{}", g.text());
    // an angle in degrees
    let mut h = fx(None);
    h.add(None, &Spec { formula: "Angle", format: "0 u", unit: Some(Unit::Degrees), kind: Kind::Number });
    assert_eq!(h.text(), "0 °");
}

#[test]
fn clock_file_and_properties() {
    let mut f = fx(None);
    // nothing known yet: the field keeps what it had
    let now = Spec { formula: "NOW()", format: "{{dd.MM.yyyy HH:mm}}", unit: Some(Unit::Date), kind: Kind::Date };
    f.add(None, &now);
    assert_eq!(f.text(), "30.12.1899 00:00");
    // with the app's clock
    let mut g = fx(None);
    g.doc.fields.now = ole_days(2026, 11, 19, 9, 30, 0);
    g.add(None, &now);
    assert_eq!(g.text(), "19.11.2026 09:30");
    // the file
    let mut h = fx(None);
    h.doc.fields.path = String::from("C:\\Pläne\\Netz.vsdx");
    h.add(None, &Spec { formula: "FILENAME()", format: "", unit: None, kind: Kind::Text });
    h.add(Some(0), &Spec { formula: "DIRECTORY()", format: "", unit: None, kind: Kind::Text });
    assert_eq!(h.text(), "C:\\Pläne\\Netz.vsdx");
    // the document's properties (UTC, shown in local time)
    let mut p = fx(None);
    let core = "<?xml version='1.0'?><cp:coreProperties xmlns:cp='x' xmlns:dc='y' xmlns:dcterms='z'><dc:title>Netzplan &amp; Räume</dc:title><dc:creator>J. F.</dc:creator><cp:keywords/><dcterms:created xsi:type='dcterms:W3CDTF'>2026-11-01T08:00:00Z</dcterms:created><dcterms:modified>2026-11-02T10:15:00+01:00</dcterms:modified></cp:coreProperties>";
    let app = "<Properties><Company>JF</Company><Manager>M</Manager></Properties>";
    for (name, ct, data) in [("/docProps/core.xml", "application/vnd.openxmlformats-package.core-properties+xml", core), ("/docProps/app.xml", "application/vnd.openxmlformats-officedocument.extended-properties+xml", app)] {
        let part = Part { name: p.doc.intern(name), content_type: p.doc.intern(ct), rel_type: p.doc.intern(""), data: Vec::from(data.as_bytes()) };
        p.doc.parts.push(part);
    }
    p.doc.fields.utc_offset = 1.0 / 24.0;
    let pr = props(&p.doc);
    assert_eq!((pr.title.as_str(), pr.creator.as_str(), pr.keywords.as_str(), pr.company.as_str()), ("Netzplan & Räume", "J. F.", "", "JF"));
    p.add(None, &Spec { formula: "TITLE()&\" (\"&CREATOR()&\", \"&COMPANY()&\")\"", format: "", unit: None, kind: Kind::Text });
    assert_eq!(p.text(), "Netzplan & Räume (J. F., JF)");
    let mut q = p;
    let created = Spec { formula: "DOCCREATION()", format: "{{dd.MM.yyyy HH:mm}}", unit: Some(Unit::Date), kind: Kind::Date };
    let saved = Spec { formula: "DOCLASTSAVE()", format: "{{HH:mm}}", unit: Some(Unit::Date), kind: Kind::Date };
    q.add(None, &created);
    q.add(None, &saved);
    assert_eq!(q.text(), "Netzplan & Räume (J. F., JF)\n01.11.2026 09:00\n10:15");
}

#[test]
fn fields_count_in_text_sizes() {
    let mut f = fx(None);
    let sheet = SheetRef::Shape(f.shape);
    let probe = |f: &mut Fx| match f.eng.evaluate_text(&f.doc, sheet, &CellAddr::single(names::WIDTH), "TEXTWIDTH(TheText)") {
        Ok(Outcome::Value(Val::Num(w))) => w,
        o => panic!("{o:?}"),
    };
    let empty = probe(&mut f);
    f.add(None, &Spec { formula: "\"eine längere Zeile Text\"", format: "", unit: None, kind: Kind::Text });
    assert!(probe(&mut f) > empty + 0.5, "{} {}", probe(&mut f), empty);
    match f.eng.evaluate_text(&f.doc, sheet, &CellAddr::single(names::WIDTH), "SHAPETEXT(TheText)") {
        Ok(Outcome::Value(Val::Str(s))) => assert_eq!(&*s, "eine längere Zeile Text"),
        o => panic!("{o:?}"),
    }
}

#[test]
fn visio_fields_keep_their_text() {
    let mut f = fx(None);
    let x = f.doc.intern("<fld IX='0'>19.11.2026</fld>");
    let mut tx = f.doc.transaction("Text");
    let t = Text { items: alloc::vec![TextItem::Chars("Stand: ".into()), TextItem::Other(x), TextItem::Chars("\n".into())], ..Text::default() };
    crate::ops::set_text(&mut tx, &mut f.eng, f.shape, t).unwrap();
    tx.commit();
    assert_eq!(f.text(), "Stand: 19.11.2026\n");
    assert!(f.drawn().contains("19.11.2026"));
    // a new field behind it
    f.add(None, &Spec { formula: "PAGENAME()", format: "", unit: None, kind: Kind::Text });
    assert_eq!(f.text(), "Stand: 19.11.2026\nAblauf\n");
}

#[test]
fn field_marks_in_written_text() {
    // (JD15) "{fld N}" is the mark of Field row N; anything else stays characters
    let t = marked("Länge: {fld 0} / {fld 12}");
    assert_eq!(
        t.items,
        alloc::vec![TextItem::Chars("Länge: ".into()), TextItem::Field(0), TextItem::Chars(" / ".into()), TextItem::Field(12)]
    );
    assert_eq!(marks(&t), alloc::vec![0, 12]);
    assert_eq!(marked("{fld 3}").items, alloc::vec![TextItem::Field(3)]);
    for plain in ["", "{fld}", "{fld x}", "{fld 1", "a {b} c", "{fld -1}", "{FLD 1}"] {
        assert_eq!(marked(plain).items, alloc::vec![TextItem::Chars(plain.into())], "{plain}");
        assert!(marks(&marked(plain)).is_empty());
    }
    // a "{" before a mark stays a character
    assert_eq!(marked("{{fld 2}}").items, alloc::vec![TextItem::Chars("{".into()), TextItem::Field(2), TextItem::Chars("}".into())]);
    // the mark shows its field's value in the shape
    let mut f = fx(None);
    let (ix, _) = f.add(None, &Spec { formula: "PAGENAME()", format: "", unit: None, kind: Kind::Text });
    let mut tx = f.doc.transaction("Text");
    crate::ops::set_text(&mut tx, &mut f.eng, f.shape, marked(&alloc::format!("Seite: {{fld {ix}}}"))).unwrap();
    tx.commit();
    assert_eq!(f.text(), "Seite: Ablauf");
}
