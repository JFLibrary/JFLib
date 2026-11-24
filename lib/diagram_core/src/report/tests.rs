// tests of report.rs: the XML read and written (Visio's shape, hostile texts), the definitions in
// the document sheet with undo, running (scopes, criteria, columns, sorting, grouping,
// subtotals, the grand total, duplicates, formats), the table shape and its refresh

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::cell::Value;
use crate::document::{DocKind, Document, Owner, PageRef, ShapeRef, SheetRef};
use crate::formula::Engine;
use crate::shape::{ShapeKind, Text};
use crate::units::Unit;

fn s(x: &str) -> String {
    String::from(x)
}

// a definition as Visio 16 writes one (the attributes and elements the research names; the
// values invented)
const VISIO: &str = "<?xml version=\"1.0\"?>\n<VisioReportDefinition xmlns=\"urn:schemas-microsoft-com:office:visio:reportdefinition\" ReportExists=\"1\" SelectDrawingDataMethod=\"2\" Description=\"Ger&#228;te &amp; Kosten\" Precision=\"2\" ShowUnits=\"1\" CaseSensitive=\"0\" UserCells=\"0\" HiddenProperties=\"0\" AdvancedFilter=\"1\" FilterExpression=\"\" Version=\"3\"><Name>JD19 Bericht</Name><Title>JD19-Test</Title>\
<VisioRptDefField ID=\"1\" Display=\"1\" DisplayOrder=\"1\" Type=\"0\" SummaryTypes=\"0\" Tag=\"2\"><Name>Name</Name><DisplayName>Name</DisplayName></VisioRptDefField>\
<VisioRptDefField ID=\"2\" Display=\"1\" DisplayOrder=\"2\" Type=\"2\" SummaryTypes=\"3\" Tag=\"2\"><Name>Kosten</Name><DisplayName>Kosten</DisplayName></VisioRptDefField>\
<VisioRptDefField ID=\"3\" Display=\"1\" DisplayOrder=\"3\" Type=\"0\" SummaryTypes=\"0\" Tag=\"1\"><Name>TEXTINFO</Name><DisplayName>&lt;Anzeigetext&gt;</DisplayName></VisioRptDefField>\
<VisioRptDefFilter ID=\"1\" Property=\"Auslastung\" Condition=\"8\" AndOr=\"1\" PropertyType=\"2\" PropertyDataType=\"2\" Value=\"\" Unit=\"\"/>\
<VisioRptDefGroup ID=\"1\" GroupingField=\"0\" ItemDisplay=\"1\" GrandSummary=\"1\" ExcludeDuplicates=\"0\"/>\
<VisioRptDefSort ID=\"1\" SortField=\"2\" Ascending=\"0\" CaseSensitive=\"0\"/></VisioReportDefinition>";

#[test]
fn reads_visio_shaped_xml() {
    let d = parse(VISIO).unwrap();
    assert_eq!(d.name, "JD19 Bericht");
    assert_eq!(d.title, "JD19-Test");
    assert_eq!(d.description, "Geräte & Kosten");
    assert_eq!(d.scope, Scope::Page);
    assert_eq!((d.precision, d.show_units, d.case_sensitive, d.advanced_filter, d.exists), (2, true, false, true, true));
    assert_eq!(d.version, "3");
    assert_eq!(d.fields.len(), 3);
    assert_eq!(d.fields[1].property, Property::Data(s("Kosten")));
    assert_eq!(Func::of_mask(d.fields[1].summary), vec![Func::Total, Func::Average]);
    assert_eq!(d.fields[2].property, Property::Builtin(Builtin::Text));
    assert_eq!(d.fields[2].display_name, "<Anzeigetext>");
    assert_eq!(d.filters[0].condition, Condition::Exists);
    assert_eq!(d.filters[0].property, Property::Data(s("Auslastung")));
    assert!(d.filters[0].and);
    let g = d.grouping.as_ref().unwrap();
    assert_eq!((g.field, g.show_items, g.grand_summary), (0, true, true));
    assert_eq!(d.sorts, vec![Sort::new(1, 2, false)]);
}

#[test]
fn round_trip_is_stable() {
    let d = parse(VISIO).unwrap();
    let x = to_xml(&d);
    assert!(x.starts_with("<?xml version=\"1.0\"?>\n<VisioReportDefinition xmlns=\"urn:schemas-microsoft-com:office:visio:reportdefinition\" ReportExists=\"1\""));
    let back = parse(&x).unwrap();
    assert_eq!(back, d);
    assert_eq!(to_xml(&back), x);
}

#[test]
fn unknown_attributes_and_elements_are_kept() {
    let t = "<VisioReportDefinition xmlns=\"urn:schemas-microsoft-com:office:visio:reportdefinition\" Future=\"7\"><Name>a</Name>\
<VisioRptDefField ID=\"1\" Display=\"1\" DisplayOrder=\"1\" Type=\"0\" SummaryTypes=\"0\" Tag=\"3\" Width=\"12\"><Name>visLegendShape</Name><DisplayName/></VisioRptDefField>\
<Later Kind=\"x\"><Inner>t &amp; u</Inner></Later><Other xmlns=\"urn:x\" A=\"1\"/></VisioReportDefinition>";
    let d = parse(t).unwrap();
    assert_eq!(d.attrs, vec![(s("Future"), s("7"))]);
    assert_eq!(d.fields[0].property, Property::User(s("visLegendShape")));
    assert_eq!(d.fields[0].attrs, vec![(s("Width"), s("12"))]);
    assert_eq!(d.extra.len(), 2);
    assert!(d.grouping.is_none());
    let x = to_xml(&d);
    assert!(x.contains("Future=\"7\""), "{x}");
    assert!(x.contains("<Later Kind=\"x\"><Inner>t &amp; u</Inner></Later>"), "{x}");
    assert!(x.contains("<Other xmlns=\"urn:x\" A=\"1\"/>"), "{x}");
    assert!(!x.contains("VisioRptDefGroup"));
    assert_eq!(parse(&x).unwrap(), d);
}

#[test]
fn hostile_xml_is_refused() {
    let doctype = "<?xml version=\"1.0\"?><!DOCTYPE r [<!ENTITY a \"aaaa\">]><VisioReportDefinition><Name>&a;</Name></VisioReportDefinition>";
    assert_eq!(parse(doctype), Err(ReportError::Unsupported));
    assert_eq!(parse("<VisioReportDefinition><Name>&a;</Name></VisioReportDefinition>"), Err(ReportError::Unsupported));
    assert_eq!(parse("<VisioReportDefinition><Name>x</Nam></VisioReportDefinition>"), Err(ReportError::Syntax));
    assert_eq!(parse("<Report/>"), Err(ReportError::NotReport));
    assert_eq!(parse(""), Err(ReportError::Syntax));
    assert_eq!(parse("<VisioReportDefinition><!-- c --></VisioReportDefinition>"), Err(ReportError::Unsupported));
    // deep nesting, too many fields, too long
    let deep = format!("<VisioReportDefinition>{}{}</VisioReportDefinition>", "<a>".repeat(100), "</a>".repeat(100));
    assert_eq!(parse(&deep), Err(ReportError::Limit));
    let many = format!("<VisioReportDefinition>{}</VisioReportDefinition>", "<VisioRptDefField ID=\"1\"/>".repeat(600));
    assert_eq!(parse(&many), Err(ReportError::Limit));
    let long = format!("<VisioReportDefinition><Name>{}</Name></VisioReportDefinition>", "x".repeat(MAX_BYTES));
    assert_eq!(parse(&long), Err(ReportError::Limit));
    // odd numbers and texts do not panic
    let odd = "<VisioReportDefinition Precision=\"-4\" SelectDrawingDataMethod=\"99999999999\"><Name>\u{1F600}</Name><VisioRptDefFilter Condition=\"x\" PropertyType=\"9\"/></VisioReportDefinition>";
    let d = parse(odd).unwrap();
    assert_eq!(d.precision, 0);
    assert_eq!(d.scope, Scope::Other(u32::MAX));
    assert_eq!(d.filters[0].condition, Condition::Other(0));
    // a written name with characters XML cannot carry
    let mut d = ReportDef::new("a\u{1}b<c>\"d\"\n");
    d.description = s("x'y");
    assert_eq!(parse(&to_xml(&d)).unwrap().name, "ab<c>\"d\"\n");
}

#[test]
fn codes_round_trip() {
    for (c, n) in CONDITIONS {
        assert_eq!(Condition::of(n), c);
        assert_eq!(c.code(), n);
    }
    for sc in [Scope::AllPages, Scope::Page, Scope::Selection, Scope::Other(9)] {
        assert_eq!(Scope::of(sc.code()), sc);
    }
    for b in BUILTINS {
        assert_eq!(Builtin::of(b.key()), b);
    }
    assert_eq!(Func::of_mask(31), FUNCS.to_vec());
}

// ---------------------------------------------------------------- the document

struct Fx {
    doc: Document,
    page: PageRef,
    page2: PageRef,
    shapes: Vec<ShapeRef>,
}

fn data(tx: &mut crate::edit::Transaction, sh: ShapeRef, name: &str, label: &str, v: Value, unit: Option<Unit>) {
    let sheet = SheetRef::Shape(sh);
    tx.set_shape_data(sheet, name, label, v).unwrap();
    if let Some(u) = unit {
        let (sk, rk) = (crate::sheet::SecKey::one(crate::names::PROPERTY), crate::sheet::RowKey::Named(tx.intern(name)));
        tx.set_constant(sheet, &crate::sheet::CellAddr::row(sk, rk, crate::names::VALUE), v, Some(u)).unwrap();
    }
}

// two pages: Server A, Server B, Router, Switch (with Standort, Kosten, Auslastung) and a box
// without data on page 1; Firewall on page 2
fn fx() -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    let mut eng = Engine::new();
    let mut tx = doc.transaction("setup");
    let page = tx.add_page("Seite-1", "Page-1", false, None).unwrap();
    let page2 = tx.add_page("Seite-2", "Page-2", false, None).unwrap();
    let rows: [(&str, &str, f64, f64, PageRef); 6] = [
        ("Server A", "Raum 1", 1200.0, 80.0, page),
        ("Server B", "Raum 2", 800.5, 45.0, page),
        ("Router", "Raum 1", 300.0, 95.0, page),
        ("Switch", "raum 1", 150.0, 20.0, page),
        ("Kasten", "", 0.0, -1.0, page),
        ("Firewall", "Raum 2", 99.0, 60.0, page2),
    ];
    let mut shapes = Vec::new();
    for (i, (name, place, cost, load, pg)) in rows.into_iter().enumerate() {
        let x = 1.0 + 2.0 * i as f64;
        let sh = crate::ops::new_shape(&mut tx, &mut eng, Owner::Page(pg), crate::ops::Basic::Rectangle, [x, 5.0], [x + 1.0, 6.0]).unwrap();
        tx.set_text(sh, Some(Text::plain(name))).unwrap();
        if load >= 0.0 {
            let n = Value::Text(tx.intern(name));
            data(&mut tx, sh, "Name", "Name", n, None);
            let p = Value::Text(tx.intern(place));
            data(&mut tx, sh, "Standort", "Standort", p, None);
            data(&mut tx, sh, "Kosten", "Kosten", Value::Num(cost), Some(Unit::Currency));
            data(&mut tx, sh, "Auslastung", "Auslastung in %", Value::Num(load), None);
        }
        shapes.push(sh);
    }
    tx.commit();
    Fx { doc, page, page2, shapes }
}

fn texts(t: &Table) -> Vec<Vec<String>> {
    t.rows.iter().map(|r| r.cells.iter().map(|c| c.text.clone()).collect()).collect()
}

fn ctx(page: PageRef) -> Ctx<'static> {
    let mut labels = Labels::default();
    labels.total = s("Summe");
    labels.count = s("Anzahl");
    labels.grand = s("Gesamt");
    Ctx { page: Some(page), selection: &[], labels }
}

fn def(fields: &[(u32, Property, &str, u32)]) -> ReportDef {
    let mut d = ReportDef::new("Test");
    d.title = s("Titel");
    for (id, p, name, sum) in fields {
        let mut f = Field::new(*id, p.clone(), name);
        f.summary = *sum;
        d.fields.push(f);
    }
    d
}

#[test]
fn scopes_and_criteria() {
    let f = fx();
    let mut d = def(&[(1, Property::Builtin(Builtin::Text), "Text", 0)]);
    // every page: the five boxes and the firewall
    let t = run(&f.doc, &d, &ctx(f.page));
    assert_eq!(t.shapes, 6);
    assert_eq!(t.title, "Titel");
    assert_eq!(t.columns[0].name, "Text");
    // the current page
    d.scope = Scope::Page;
    assert_eq!(run(&f.doc, &d, &ctx(f.page2)).shapes, 1);
    // the selection
    d.scope = Scope::Selection;
    let sel = [f.shapes[1], f.shapes[2]];
    let c = Ctx { selection: &sel, ..ctx(f.page) };
    assert_eq!(texts(&run(&f.doc, &d, &c)), vec![vec![s("Server B")], vec![s("Router")]]);
    // exists: the box without data is out
    d.scope = Scope::AllPages;
    d.filters.push(Filter::new(1, Property::Data(s("Auslastung")), Condition::Exists, ""));
    assert_eq!(run(&f.doc, &d, &ctx(f.page)).shapes, 5);
    // and > 50 (by the label too)
    d.filters.push(Filter::new(2, Property::Data(s("Auslastung in %")), Condition::Gt, "50"));
    assert_eq!(texts(&run(&f.doc, &d, &ctx(f.page))), vec![vec![s("Server A")], vec![s("Router")], vec![s("Firewall")]]);
    // a text criterion, case ignored and then not
    d.filters.truncate(1);
    d.filters.push(Filter::new(3, Property::Data(s("Standort")), Condition::Eq, "RAUM 1"));
    assert_eq!(run(&f.doc, &d, &ctx(f.page)).shapes, 3);
    d.case_sensitive = true;
    assert_eq!(run(&f.doc, &d, &ctx(f.page)).shapes, 0);
    // does not exist; an "or"
    d.case_sensitive = false;
    d.filters = vec![Filter::new(1, Property::Data(s("Kosten")), Condition::NotExists, "")];
    assert_eq!(texts(&run(&f.doc, &d, &ctx(f.page))), vec![vec![s("Kasten")]]);
    let mut or = Filter::new(2, Property::Builtin(Builtin::Text), Condition::Eq, "Router");
    or.and = false;
    d.filters.push(or);
    assert_eq!(run(&f.doc, &d, &ctx(f.page)).shapes, 2);
    // a currency criterion against a number with a decimal comma
    d.filters = vec![Filter::new(1, Property::Data(s("Kosten")), Condition::Le, "300,0")];
    assert_eq!(run(&f.doc, &d, &ctx(f.page)).shapes, 3);
}

#[test]
fn columns_sorting_and_formats() {
    let mut f = fx();
    f.doc.fields.decimal = ',';
    let mut d = def(&[
        (1, Property::Data(s("Name")), "Name", 0),
        (2, Property::Data(s("Kosten")), "Kosten", 0),
        (3, Property::Builtin(Builtin::Width), "Breite", 0),
        (4, Property::Builtin(Builtin::NameId), "ID", 0),
    ]);
    d.fields[3].display = false;
    d.filters.push(Filter::new(1, Property::Data(s("Kosten")), Condition::Exists, ""));
    d.sorts.push(Sort::new(1, 2, false));
    let t = run(&f.doc, &d, &ctx(f.page));
    assert_eq!(t.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["Name", "Kosten", "Breite"]);
    assert!(!t.columns[0].numeric && t.columns[1].numeric && t.columns[2].numeric);
    let rows = texts(&t);
    assert_eq!(rows[0], vec![s("Server A"), s("1200,00"), s("1,00 in")]);
    assert_eq!(rows[1][1], "800,50");
    assert_eq!(rows.iter().map(|r| r[0].as_str()).collect::<Vec<_>>(), vec!["Server A", "Server B", "Router", "Switch", "Firewall"]);
    assert_eq!(t.rows[1].cells[1].value, Val::Num(800.5));
    // the column order, no units, no decimals
    d.fields[0].order = 9;
    d.show_units = false;
    d.precision = 0;
    let t = run(&f.doc, &d, &ctx(f.page));
    assert_eq!(texts(&t)[0], vec![s("1200"), s("1"), s("Server A")]);
    // ascending by text
    d.sorts = vec![Sort::new(1, 1, true)];
    let t = run(&f.doc, &d, &ctx(f.page));
    assert_eq!(texts(&t).iter().map(|r| r[2].clone()).collect::<Vec<_>>(), vec!["Firewall", "Router", "Server A", "Server B", "Switch"]);
}

#[test]
fn groups_subtotals_and_grand_total() {
    let f = fx();
    let mut d = def(&[(1, Property::Data(s("Standort")), "Standort", 0), (2, Property::Data(s("Name")), "Name", 0), (3, Property::Data(s("Kosten")), "Kosten", Func::Total.bit() | Func::Count.bit())]);
    d.filters.push(Filter::new(1, Property::Data(s("Kosten")), Condition::Exists, ""));
    d.precision = 1;
    let g = d.grouping.as_mut().unwrap();
    g.field = 1;
    g.grand_summary = true;
    let t = run(&f.doc, &d, &ctx(f.page));
    let r = texts(&t);
    // Raum 1 (case ignored: "raum 1" joins it), then Raum 2
    assert_eq!(t.rows[0].kind, RowKind::Group);
    assert_eq!(r[0], vec![s("Raum 1"), s(""), s("")]);
    assert_eq!(r[1][1], "Server A");
    assert_eq!(r[4], vec![s("Summe"), s(""), s("1650.0")]);
    assert_eq!(t.rows[4].kind, RowKind::Summary(Some(Func::Total)));
    assert_eq!(r[5], vec![s("Anzahl"), s(""), s("3")]);
    assert_eq!(r[6], vec![s("Raum 2"), s(""), s("")]);
    let last = t.rows.len() - 1;
    assert_eq!(t.rows[last].kind, RowKind::Grand(Some(Func::Count)));
    assert_eq!(r[last - 1], vec![s("Gesamt – Summe"), s(""), s("2549.5")]);
    assert_eq!(r[last][2], "5");
    // summaries only: the inventory's shape
    let mut inv = def(&[(1, Property::Data(s("Standort")), "Standort", Func::Count.bit()), (2, Property::Data(s("Kosten")), "Kosten", Func::Max.bit())]);
    inv.precision = 0;
    inv.filters.push(Filter::new(1, Property::Data(s("Kosten")), Condition::Exists, ""));
    let g = inv.grouping.as_mut().unwrap();
    g.field = 1;
    g.show_items = false;
    g.grand_summary = true;
    let mut c = ctx(f.page);
    c.labels.max = s("Max");
    let t = run(&f.doc, &inv, &c);
    assert_eq!(t.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["Standort", "Anzahl", "Kosten (Max)"]);
    assert_eq!(texts(&t), vec![vec![s("Raum 1"), s("3"), s("1200")], vec![s("Raum 2"), s("2"), s("801")], vec![s("Gesamt"), s("5"), s("1200")]]);
    assert_eq!(t.rows[2].kind, RowKind::Grand(None));
    // without grouping: the summaries at the end
    let mut flat = def(&[(1, Property::Data(s("Name")), "Name", 0), (2, Property::Data(s("Auslastung")), "Last", Func::Average.bit() | Func::Min.bit())]);
    flat.filters.push(Filter::new(1, Property::Data(s("Kosten")), Condition::Exists, ""));
    flat.precision = 0;
    let mut c = ctx(f.page);
    c.labels.average = s("Mittel");
    c.labels.min = s("Min");
    let r = texts(&run(&f.doc, &flat, &c));
    assert_eq!(r[5], vec![s("Mittel"), s("60")]);
    assert_eq!(r[6], vec![s("Min"), s("20")]);
}

#[test]
fn duplicates_and_master_names() {
    let mut f = fx();
    let mut d = def(&[(1, Property::Data(s("Standort")), "Standort", 0)]);
    d.filters.push(Filter::new(1, Property::Data(s("Kosten")), Condition::Exists, ""));
    d.sorts.push(Sort::new(1, 1, true));
    d.grouping.as_mut().unwrap().exclude_duplicates = true;
    assert_eq!(texts(&run(&f.doc, &d, &ctx(f.page))), vec![vec![s("Raum 1")], vec![s("raum 1")], vec![s("Raum 2")]]);
    // a master's name; a shape without master shows nothing
    let mut tx = f.doc.transaction("master");
    let m = tx.add_master("Server", "Server").unwrap();
    let mid = tx.doc().master(m).unwrap().header.id;
    let mut h = tx.doc().shape(f.shapes[0]).unwrap().header().clone();
    h.master = Some(mid);
    tx.set_shape_header(f.shapes[0], h).unwrap();
    tx.commit();
    let d = def(&[(1, Property::Builtin(Builtin::Master), "Master", 0), (2, Property::Builtin(Builtin::NameId), "ID", 0)]);
    let r = texts(&run(&f.doc, &d, &ctx(f.page)));
    assert_eq!(r[0][0], "Server");
    assert_eq!(r[1][0], "");
    assert!(r[1][1].starts_with("Sheet."));
}

#[test]
fn groups_count_their_members_and_report_tables_are_left_out() {
    let mut f = fx();
    let mut eng = Engine::new();
    let mut tx = f.doc.transaction("group");
    let owner = Owner::Page(f.page);
    let g = crate::ops::group(&mut tx, &mut eng, owner, &[f.shapes[0], f.shapes[1]]).unwrap();
    tx.commit();
    let mut d = def(&[(1, Property::Builtin(Builtin::Text), "Text", 0)]);
    d.scope = Scope::Page;
    // the group (no master, no data) stands for its two members
    assert_eq!(run(&f.doc, &d, &ctx(f.page)).shapes, 5);
    // with shape data of its own it is one shape
    let mut tx = f.doc.transaction("data");
    tx.set_shape_data(SheetRef::Shape(g), "Name", "Name", Value::Num(1.0)).unwrap();
    tx.commit();
    assert_eq!(run(&f.doc, &d, &ctx(f.page)).shapes, 4);
    // a report table on the page is no shape of the report
    let t = run(&f.doc, &d, &ctx(f.page));
    let mut tx = f.doc.transaction("place");
    let p = Place { name: s("Test"), def_copy: Some(to_xml(&d)), ..Place::default() };
    place(&mut tx, &mut eng, owner, [1.0, 10.0], &t, &p).unwrap();
    tx.commit();
    assert_eq!(run(&f.doc, &d, &ctx(f.page)).shapes, 4);
}

#[test]
fn definitions_in_the_document_sheet() {
    let mut f = fx();
    let d1 = ReportDef::new("Inventar");
    let mut d2 = parse(VISIO).unwrap();
    let mut tx = f.doc.transaction("add");
    assert_eq!(add(&mut tx, &d1).unwrap(), "visPropRpt1");
    assert_eq!(add(&mut tx, &d2).unwrap(), "visPropRpt2");
    tx.commit();
    let st = stored(&f.doc);
    assert_eq!(st.len(), 2);
    assert_eq!(st[1].def.as_ref().unwrap(), &d2);
    // the cell is a string as Visio writes it
    let row = f.doc.user_cells(SheetRef::Document).into_iter().find(|r| r.name() == Some("visPropRpt2")).unwrap();
    assert_eq!(row.get("Value").unit(), Some(Unit::Str));
    assert_eq!(find(&f.doc, "jd19 bericht").unwrap().row, "visPropRpt2");
    // replace, rename
    d2.precision = 0;
    let mut tx = f.doc.transaction("replace");
    replace(&mut tx, "visPropRpt2", &d2).unwrap();
    rename(&mut tx, "visPropRpt1", "Bestand").unwrap();
    assert_eq!(replace(&mut tx, "visPropRpt9", &d2), Err(crate::edit::EditError::NoRow));
    let ch = tx.commit().unwrap();
    let st = stored(&f.doc);
    assert_eq!(st[0].def.as_ref().unwrap().name, "Bestand");
    assert_eq!(st[0].def.as_ref().unwrap().title, "Bestand");
    assert_eq!(st[1].def.as_ref().unwrap().precision, 0);
    // remove the first: the second moves up; undo brings both back
    let mut tx = f.doc.transaction("remove");
    remove(&mut tx, "visPropRpt1").unwrap();
    let mut rm = tx.commit().unwrap();
    let st = stored(&f.doc);
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].row, "visPropRpt1");
    assert_eq!(st[0].def.as_ref().unwrap().name, "JD19 Bericht");
    assert!(rm.undo(&mut f.doc));
    assert_eq!(stored(&f.doc).len(), 2);
    let mut ch = ch;
    assert!(ch.undo(&mut f.doc));
    assert_eq!(stored(&f.doc)[0].def.as_ref().unwrap().name, "Inventar");
    // a row that holds no definition is listed with its error
    let mut tx = f.doc.transaction("bad");
    let v = Value::Text(tx.intern("<kaputt"));
    tx.set_user(SheetRef::Document, "visPropRpt7", v, Some(Unit::Str)).unwrap();
    tx.commit();
    let st = stored(&f.doc);
    assert_eq!(st.last().unwrap().def, Err(ReportError::Syntax));
    let mut tx = f.doc.transaction("rename bad");
    assert_eq!(rename(&mut tx, "visPropRpt7", "x"), Err(crate::edit::EditError::Invalid));
}

#[test]
fn table_shape_and_refresh() {
    let mut f = fx();
    let mut eng = Engine::new();
    let mut d = def(&[(1, Property::Data(s("Name")), "Name", 0), (2, Property::Data(s("Kosten")), "Kosten", Func::Total.bit())]);
    d.filters.push(Filter::new(1, Property::Data(s("Kosten")), Condition::Exists, ""));
    let t = run(&f.doc, &d, &ctx(f.page));
    let owner = Owner::Page(f.page);
    let mut tx = f.doc.transaction("Bericht");
    let layer = report_layer(&mut tx, f.page, "Bericht", "Report").unwrap();
    let p = Place { name: s("Test"), def_copy: None, link: Some(s("")), layers: vec![layer] };
    let g = place(&mut tx, &mut eng, owner, [1.0, 10.0], &t, &p).unwrap();
    tx.commit();
    let doc = &f.doc;
    let sh = doc.shape(g).unwrap();
    assert_eq!(sh.header().kind, ShapeKind::Group);
    // title + heading + 5 rows + the total, two columns each (the title one cell)
    assert_eq!(sh.children().count(), 1 + 2 * 7);
    let found = report_shape(doc, sh.children().nth(3).unwrap()).unwrap();
    assert_eq!(found.shape, g);
    assert_eq!((found.name.as_str(), found.def.as_deref(), found.link.as_deref()), ("Test", None, Some("")));
    assert_eq!(doc.layer_members(g), vec![layer]);
    assert_eq!(doc.layer_by_name(f.page, "Report"), Some(layer));
    let texts_of = |doc: &Document, g: ShapeRef| -> Vec<String> { doc.shape(g).unwrap().children().map(|c| doc.shape(c).unwrap().text().map(|t| t.chars()).unwrap_or_default()).collect() };
    let cells = texts_of(doc, g);
    assert_eq!(cells[0], "Titel");
    assert_eq!(&cells[1..5], &[s("Name"), s("Kosten"), s("Server A"), s("1200.00")]);
    assert_eq!(cells.last().unwrap(), "2549.50");
    let top = crate::ops::bounds(doc, &[g]).unwrap();
    assert!((top[0] - 1.0).abs() < 1e-9 && (top[3] - 10.0).abs() < 1e-9, "{top:?}");
    // the data changes, the table is built anew in place, one undo step takes it back
    let mut tx = f.doc.transaction("Kosten");
    tx.set_shape_data(SheetRef::Shape(f.shapes[0]), "Kosten", "Kosten", Value::Num(1000.0)).unwrap();
    tx.commit();
    let t = run(&f.doc, &d, &ctx(f.page));
    let mut tx = f.doc.transaction("Bericht aktualisieren");
    let g2 = refresh(&mut tx, &mut eng, g, &t).unwrap();
    let mut ch = tx.commit().unwrap();
    assert_ne!(g2, g);
    assert!(f.doc.shape(g).is_none());
    assert_eq!(texts_of(&f.doc, g2)[4], "1000.00");
    let b2 = crate::ops::bounds(&f.doc, &[g2]).unwrap();
    assert!((b2[0] - top[0]).abs() < 1e-9 && (b2[3] - top[3]).abs() < 1e-9);
    assert_eq!(f.doc.layer_members(g2), vec![layer]);
    assert_eq!(report_shape(&f.doc, g2).unwrap().link.as_deref(), Some(""));
    assert!(ch.undo(&mut f.doc));
    assert!(f.doc.shape(g).is_some() && f.doc.shape(g2).is_none());
    assert_eq!(texts_of(&f.doc, g)[4], "1200.00");
}

#[test]
fn numbers_kept_as_text_count_in_summaries() {
    let mut f = fx();
    let mut tx = f.doc.transaction("text");
    let v = Value::Text(tx.intern("450"));
    tx.set_shape_data(SheetRef::Shape(f.shapes[2]), "Kosten", "Kosten", v).unwrap();
    tx.commit();
    let mut d = def(&[(1, Property::Data(s("Name")), "Name", 0), (2, Property::Data(s("Kosten")), "Kosten", Func::Total.bit())]);
    d.filters.push(Filter::new(1, Property::Data(s("Kosten")), Condition::Exists, ""));
    let t = run(&f.doc, &d, &ctx(f.page));
    let r = texts(&t);
    assert_eq!(r[2], vec![s("Router"), s("450")]);
    assert_eq!(r.last().unwrap(), &vec![s("Summe"), s("2699.50")]);
}
