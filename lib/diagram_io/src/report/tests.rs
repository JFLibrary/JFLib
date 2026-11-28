// tests of report.rs: the XML read and written (Visio's shape, unknown parts kept, hostile
// texts), the definitions in the document sheet with undo

use alloc::format;
use alloc::string::String;
use alloc::vec;

use super::*;
use diagram_core::report::{Builtin, Func};
use diagram_core::DocKind;

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

// (JD19) "exists" and "does not exist" as Visio's own INVENTRY.VRD writes them: 8 with the
// Boolean Value 1 or 0 (PropertyDataType 3); an earlier own 7 reads as "does not exist"
#[test]
fn exists_as_visio_writes_it() {
    let base = VISIO.replace("AdvancedFilter=\"1\"", "AdvancedFilter=\"0\"");
    let not = base.replace("Condition=\"8\" AndOr=\"1\" PropertyType=\"2\" PropertyDataType=\"2\" Value=\"\"", "Condition=\"8\" AndOr=\"1\" PropertyType=\"2\" PropertyDataType=\"3\" Value=\"0\"");
    let seven = base.replace("Condition=\"8\"", "Condition=\"7\"");
    let d = parse(&base).unwrap();
    assert_eq!((d.filters[0].condition, d.filters[0].value.as_str(), d.filters[0].data_type), (Condition::Exists, "1", 3));
    assert!(to_xml(&d).contains("Condition=\"8\" AndOr=\"1\" PropertyType=\"2\" PropertyDataType=\"3\" Value=\"1\""));
    for x in [not, seven] {
        let d = parse(&x).unwrap();
        assert_eq!(d.filters[0].condition, Condition::NotExists);
        assert!(to_xml(&d).contains("Condition=\"8\" AndOr=\"1\" PropertyType=\"2\" PropertyDataType=\"3\" Value=\"0\""));
        assert_eq!(parse(&to_xml(&d)).unwrap(), d);
    }
}

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
    // SummaryTypes 3 = count and total (Visio's bits, checked in Visio 16)
    assert_eq!(Func::of_mask(d.fields[1].summary), vec![Func::Total, Func::Count]);
    assert_eq!(d.fields[2].property, Property::Builtin(Builtin::Text));
    assert_eq!(d.fields[2].display_name, "<Anzeigetext>");
    assert_eq!(d.filters[0].condition, Condition::Exists);
    assert_eq!(d.filters[0].property, Property::Data(s("Auslastung")));
    assert!(d.filters[0].and);
    let g = d.grouping.as_ref().unwrap();
    assert_eq!((g.field, g.show_items, g.grand_summary), (0, true, true));
    assert_eq!(d.sorts, vec![Sort::new(1, 2, false)]);
    // without a namespace, with a BOM, spaces around
    let plain = VISIO.replace(" xmlns=\"urn:schemas-microsoft-com:office:visio:reportdefinition\"", "");
    assert_eq!(parse(&format!("\u{feff}{plain}\n")).unwrap(), d);
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
    let t = "<VisioReportDefinition xmlns=\"urn:schemas-microsoft-com:office:visio:reportdefinition\" xmlns:p=\"urn:p\" Future=\"7\" p:x=\"1\"><Name>a</Name>\
<VisioRptDefField ID=\"1\" Display=\"1\" DisplayOrder=\"1\" Type=\"0\" SummaryTypes=\"0\" Tag=\"3\" Width=\"12\"><Name>visLegendShape</Name><DisplayName/><Later/></VisioRptDefField>\
<Later Kind=\"x\"><Inner>t &amp; u</Inner></Later><Other xmlns=\"urn:x\" A=\"1\"/><p:Gone/></VisioReportDefinition>";
    let d = parse(t).unwrap();
    assert_eq!(d.attrs, vec![(s("Future"), s("7"))]);
    assert_eq!(d.fields[0].property, Property::User(s("visLegendShape")));
    assert_eq!(d.fields[0].attrs, vec![(s("Width"), s("12"))]);
    assert_eq!(d.extra, vec![s("<Later Kind=\"x\"><Inner>t &amp; u</Inner></Later>"), s("<Other xmlns=\"urn:x\" A=\"1\"/>")]);
    assert!(d.grouping.is_none());
    let x = to_xml(&d);
    assert!(x.contains("Future=\"7\""), "{x}");
    assert!(x.contains("<Later Kind=\"x\"><Inner>t &amp; u</Inner></Later><Other xmlns=\"urn:x\" A=\"1\"/></VisioReportDefinition>"), "{x}");
    assert!(!x.contains("VisioRptDefGroup"));
    assert_eq!(parse(&x).unwrap(), d);
}

#[test]
fn hostile_xml_is_refused() {
    let doctype = "<?xml version=\"1.0\"?><!DOCTYPE r [<!ENTITY a \"aaaa\">]><VisioReportDefinition><Name>&a;</Name></VisioReportDefinition>";
    assert_eq!(parse(doctype), Err(ReportError::Unsupported));
    assert!(parse("<VisioReportDefinition><Name>&a;</Name></VisioReportDefinition>").is_err());
    assert_eq!(parse("<VisioReportDefinition><Name>x</Nam></VisioReportDefinition>"), Err(ReportError::Syntax));
    assert_eq!(parse("<VisioReportDefinition><Name>x</Name>"), Err(ReportError::Syntax));
    assert_eq!(parse("<VisioReportDefinition/><b/>"), Err(ReportError::Syntax));
    assert_eq!(parse("<Report/>"), Err(ReportError::NotReport));
    assert_eq!(parse("<v:VisioReportDefinition xmlns:v=\"http://schemas.microsoft.com/office/visio/2012/main\"/>"), Err(ReportError::NotReport));
    assert_eq!(parse(""), Err(ReportError::Syntax));
    // a comment is passed over
    assert_eq!(parse("<VisioReportDefinition><!-- c --><Name>n</Name></VisioReportDefinition>").unwrap().name, "n");
    // deep nesting, too many fields, too long
    let deep = format!("<VisioReportDefinition>{}{}</VisioReportDefinition>", "<a>".repeat(100), "</a>".repeat(100));
    assert_eq!(parse(&deep), Err(ReportError::Limit));
    let many = format!("<VisioReportDefinition>{}</VisioReportDefinition>", "<VisioRptDefField ID=\"1\"/>".repeat(600));
    assert_eq!(parse(&many), Err(ReportError::Limit));
    let long = format!("<VisioReportDefinition><Name>{}</Name></VisioReportDefinition>", "x".repeat(MAX_BYTES));
    assert_eq!(parse(&long), Err(ReportError::Limit));
    let attrs: String = (0..100).map(|i| format!(" a{i}=\"1\"")).collect();
    assert_eq!(parse(&format!("<VisioReportDefinition{attrs}/>")), Err(ReportError::Limit));
    // odd numbers and texts do not panic
    let odd = "<VisioReportDefinition Precision=\"-4\" SelectDrawingDataMethod=\"99999999999\"><Name>\u{1F600}</Name><VisioRptDefFilter Condition=\"x\" PropertyType=\"9\"/></VisioReportDefinition>";
    let d = parse(odd).unwrap();
    assert_eq!(d.precision, 0);
    assert_eq!(d.scope, Scope::Other(u32::MAX));
    assert_eq!(d.filters[0].condition, Condition::Other(0));
    // written: what XML cannot carry is left out, the rest comes back
    let mut d = ReportDef::new("a\u{1}b<c>\"d\"\n");
    d.description = s("x'y\tz");
    let back = parse(&to_xml(&d)).unwrap();
    assert_eq!(back.name, "ab<c>\"d\"\n");
    assert_eq!(back.description, "x'y\tz");
}

#[test]
fn definitions_in_the_document_sheet() {
    let mut doc = Document::new(DocKind::Drawing);
    let mut tx = doc.transaction("setup");
    tx.add_page("Seite-1", "Page-1", false, None).unwrap();
    tx.commit();
    let d1 = ReportDef::new("Inventar");
    let mut d2 = parse(VISIO).unwrap();
    let mut tx = doc.transaction("add");
    assert_eq!(add(&mut tx, &d1).unwrap(), "visPropRpt1");
    assert_eq!(add(&mut tx, &d2).unwrap(), "visPropRpt2");
    tx.commit();
    let st = stored(&doc);
    assert_eq!(st.len(), 2);
    assert_eq!(st[1].def.as_ref().unwrap(), &d2);
    // the cell is a string as Visio writes it
    let row = doc.user_cells(SheetRef::Document).into_iter().find(|r| r.name() == Some("visPropRpt2")).unwrap();
    assert_eq!(row.get("Value").unit(), Some(Unit::Str));
    assert_eq!(find(&doc, "jd19 bericht").unwrap().row, "visPropRpt2");
    // replace, rename
    d2.precision = 0;
    let mut tx = doc.transaction("replace");
    replace(&mut tx, "visPropRpt2", &d2).unwrap();
    rename(&mut tx, "visPropRpt1", "Bestand").unwrap();
    assert_eq!(replace(&mut tx, "visPropRpt9", &d2), Err(EditError::NoRow));
    let mut ch = tx.commit().unwrap();
    let st = stored(&doc);
    assert_eq!(st[0].def.as_ref().unwrap().name, "Bestand");
    assert_eq!(st[0].def.as_ref().unwrap().title, "Bestand");
    assert_eq!(st[1].def.as_ref().unwrap().precision, 0);
    // remove the first: the second moves up; undo brings both back
    let mut tx = doc.transaction("remove");
    remove(&mut tx, "visPropRpt1").unwrap();
    let mut rm = tx.commit().unwrap();
    let st = stored(&doc);
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].row, "visPropRpt1");
    assert_eq!(st[0].def.as_ref().unwrap().name, "JD19 Bericht");
    assert!(rm.undo(&mut doc));
    assert_eq!(stored(&doc).len(), 2);
    assert!(ch.undo(&mut doc));
    assert_eq!(stored(&doc)[0].def.as_ref().unwrap().name, "Inventar");
    // a row that holds no definition is listed with its error
    let mut tx = doc.transaction("bad");
    let v = Value::Text(tx.intern("<kaputt"));
    tx.set_user(SheetRef::Document, "visPropRpt7", v, Some(Unit::Str)).unwrap();
    tx.commit();
    let st = stored(&doc);
    assert_eq!(st.last().unwrap().def, Err(ReportError::Syntax));
    let mut tx = doc.transaction("rename bad");
    assert_eq!(rename(&mut tx, "visPropRpt7", "x"), Err(EditError::Invalid));
}
