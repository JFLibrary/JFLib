// reports -- (JD19) the group "reports": shape report definitions in VSDX files (DA-09, DA-13)
// through diagram_core::report
//
//   run(h)               a drawing with five devices, two definitions in the document sheet and a
//                        report table on the page, written and read back: the definitions the
//                        same (rows visPropRpt1 and 2, U="STR"), the table's marks and layer,
//                        the second write byte for byte the first, the report run on the read
//                        drawing the same; a document sheet written the way Visio writes one
//                        (visPropRpt1 with Visio's XML, escaped, a Prompt cell) read, run and
//                        written back; a row that is no definition (a DOCTYPE, broken XML) kept
//                        unchanged while the file opens
//   write_check(dir)     --write-report DIR: jd19-bericht.vsdx for the Visio counter-check
//                        (shapes with shape data, two definitions, a report table)

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use diagram_core::report::{self, Builtin, Condition, Field, Filter, Func, Property, ReportDef, Sort};
use diagram_core::{DocKind, Document, Owner, PageRef, SheetRef, Text, Value};
use diagram_io::vsdx::{self, Measure};

use crate::pkg::{DECL, NS};
use crate::H;

fn s(x: &str) -> String {
    String::from(x)
}

fn write(doc: &Document) -> Vec<u8> {
    vsdx::write(doc, &vsdx::WriteOptions::DEFAULT).unwrap_or_default()
}

fn read(bytes: &[u8]) -> Option<Document> {
    vsdx::read(bytes, &vsdx::Limits::VSDX).ok().map(|l| l.doc)
}

/// the devices of the drawing (as in the user's Visio sample): name, place, cost, load
const DEVICES: [(&str, &str, f64, f64); 5] = [("Server A", "Raum 1", 1200.0, 80.0), ("Server B", "Raum 2", 800.5, 45.0), ("Router", "Raum 1", 300.0, 95.0), ("Switch", "Raum 1", 150.0, 20.0), ("Firewall", "Raum 2", 99.0, 60.0)];

/// the definitions: a device list (shape data, the text, a criterion, grouped with a subtotal,
/// sorted) and an inventory (summaries only)
pub(crate) fn definitions() -> (ReportDef, ReportDef) {
    let mut list = ReportDef::new("Geräteliste");
    list.title = s("Geräte nach Standort");
    list.description = s("Die Geräte mit Kosten und Auslastung, nach Standort gruppiert");
    list.fields = vec![
        Field::new(1, Property::Data(s("Name")), "Name"),
        Field::new(2, Property::Data(s("Standort")), "Standort"),
        Field::new(3, Property::Data(s("Kosten")), "Kosten"),
        Field::new(4, Property::Data(s("Auslastung")), "Auslastung"),
        Field::new(5, Property::Builtin(Builtin::Text), "<Anzeigetext>"),
    ];
    list.fields[2].data_type = 7;
    list.fields[2].summary = Func::Total.bit();
    list.fields[3].data_type = 2;
    let mut f = Filter::new(1, Property::Data(s("Auslastung")), Condition::Exists, "");
    f.data_type = 2;
    list.filters.push(f);
    list.advanced_filter = true;
    if let Some(g) = list.grouping.as_mut() {
        g.field = 2;
        g.grand_summary = true;
    }
    list.sorts.push(Sort::new(1, 4, false));
    let mut inv = ReportDef::new("Inventar");
    inv.description = s("Anzahl der Shapes je Master");
    let mut m = Field::new(1, Property::Builtin(Builtin::Master), "Master-Name");
    m.summary = Func::Count.bit();
    inv.fields.push(m);
    if let Some(g) = inv.grouping.as_mut() {
        g.field = 1;
        g.show_items = false;
        g.grand_summary = true;
    }
    (list, inv)
}

// five boxes with shape data on page 1, the definitions in the document, the device list as a
// table on page 1 (on the layer "Bericht")
fn drawing() -> (Document, PageRef) {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().expect("page");
    let mut eng = diagram_core::formula::Engine::new();
    let (list, inv) = definitions();
    let mut tx = doc.transaction("report");
    for (i, (name, place, cost, load)) in DEVICES.iter().enumerate() {
        let x = 1.0 + 1.6 * i as f64;
        let Ok(sh) = diagram_core::ops::new_shape(&mut tx, &mut eng, Owner::Page(page), diagram_core::ops::Basic::Rectangle, [x, 9.0], [x + 1.2, 9.8]) else { continue };
        let _ = tx.set_text(sh, Some(Text::plain(name)));
        let sheet = SheetRef::Shape(sh);
        let n = Value::Text(tx.intern(name));
        let _ = tx.set_shape_data(sheet, "Name", "Name", n);
        let p = Value::Text(tx.intern(place));
        let _ = tx.set_shape_data(sheet, "Standort", "Standort", p);
        let _ = tx.set_shape_data(sheet, "Kosten", "Kosten", Value::Num(*cost));
        let _ = tx.set_shape_data(sheet, "Auslastung", "Auslastung", Value::Num(*load));
    }
    let _ = report::add(&mut tx, &list);
    let _ = report::add(&mut tx, &inv);
    let table = report::run(tx.doc(), &list, &report::Ctx { page: Some(page), ..Default::default() });
    let layer = report::report_layer(&mut tx, page, "Bericht", "Report").ok();
    let p = report::Place { name: list.name.clone(), def_copy: Some(report::to_xml(&list)), link: None, layers: layer.into_iter().collect() };
    let _ = report::place(&mut tx, &mut eng, Owner::Page(page), [1.0, 7.5], &table, &p);
    tx.commit();
    (doc, page)
}

pub(crate) fn run(h: &mut H) {
    own(h);
    visio_like(h);
}

fn defs(doc: &Document) -> Vec<(String, Option<ReportDef>)> {
    report::stored(doc).into_iter().map(|st| (st.row, st.def.ok())).collect()
}

fn own(h: &mut H) {
    let (doc, page) = drawing();
    let before = defs(&doc);
    h.check("reports/own/model", before.len() == 2 && before[0].0 == "visPropRpt1" && before.iter().all(|d| d.1.is_some()), format!("{} definitions in the document sheet", before.len()));
    let bytes = write(&doc);
    let Some(back) = read(&bytes) else {
        h.check("reports/own/read", false, s("the written drawing does not read"));
        return;
    };
    let after = defs(&back);
    h.check("reports/own/round-trip", after == before, s("the definitions written and read back unchanged"));
    let xml = archive::opc::Package::read(&bytes, archive::Limits::default(), archive::opc::Rules::DEFAULT)
        .ok()
        .and_then(|p| p.part("visio/document.xml").map(|b| String::from_utf8_lossy(b).into_owned()))
        .unwrap_or_default();
    let at = xml.find("N='visPropRpt1'").unwrap_or(0);
    let row = &xml[at..xml[at..].find("</Row>").map_or(at, |e| at + e)];
    h.check("reports/own/cell", at > 0 && row.contains("V='&lt;?xml version=\"1.0\"?&gt;&#10;&lt;VisioReportDefinition") && row.contains("U='STR'"), s("visPropRpt1: Value is the escaped XML, U='STR'"));
    let table = back.top_shapes(Owner::Page(back.pages().next().unwrap_or(page))).find_map(|sh| report::report_shape(&back, sh));
    let ok = table.as_ref().is_some_and(|t| t.name == "Geräteliste" && t.def.as_deref().and_then(|d| report::parse(d).ok()) == before[0].1.clone() && back.layer_members(t.shape).len() == 1);
    h.check("reports/own/table", ok, s("the table shape found again: its name, the copy of its definition, its layer"));
    let layer = table.as_ref().and_then(|t| back.layer_members(t.shape).first().copied());
    let named = back.pages().next().and_then(|p| back.layer_by_name(p, "Report")) == layer;
    h.check("reports/own/layer", named, s("the table on the layer Report (Bericht)"));
    let again = write(&back);
    h.check("reports/own/stable", again == bytes, s("the second write byte for byte the first"));
    let ctx = |d: &Document| report::Ctx { page: d.pages().next(), ..Default::default() };
    let list = before[0].1.clone().unwrap_or_else(|| ReportDef::new(""));
    let t1 = report::run(&doc, &list, &ctx(&doc));
    let t2 = report::run(&back, &list, &ctx(&back));
    let rows: Vec<Vec<String>> = t2.rows.iter().map(|r| r.cells.iter().map(|c| c.text.clone()).collect()).collect();
    h.check("reports/own/run", t1 == t2 && t2.shapes == 5 && rows.first().is_some_and(|r| r[1] == "Raum 1") && rows.last().is_some_and(|r| r[2] == "2549.50"), format!("the report on the read drawing: {} rows, last {:?}", rows.len(), rows.last()));
}

// a text as Visio escapes it into an attribute value
fn attr(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;").replace('\n', "&#10;")
}

// the document sheet as Visio writes one (the definition's XML in Visio's shape, the values
// invented; the escaped text in V, a Prompt cell) and a row that is no definition
fn visio_like(h: &mut H) {
    let def = "<?xml version=\"1.0\"?>\n<VisioReportDefinition xmlns=\"urn:schemas-microsoft-com:office:visio:reportdefinition\" ReportExists=\"1\" SelectDrawingDataMethod=\"1\" Description=\"Alle Shapes mit Auslastung\" Precision=\"1\" ShowUnits=\"0\" CaseSensitive=\"0\" UserCells=\"0\" HiddenProperties=\"0\" AdvancedFilter=\"1\" FilterExpression=\"\" Version=\"3\">\
<Name>JD19 Bericht</Name><Title>JD19-Test</Title>\
<VisioRptDefField ID=\"1\" Display=\"1\" DisplayOrder=\"1\" Type=\"0\" SummaryTypes=\"0\" Tag=\"1\"><Name>TEXTINFO</Name><DisplayName>&lt;Displayed Text&gt;</DisplayName></VisioRptDefField>\
<VisioRptDefField ID=\"2\" Display=\"1\" DisplayOrder=\"2\" Type=\"2\" SummaryTypes=\"1\" Tag=\"2\"><Name>Kosten</Name><DisplayName>Kosten</DisplayName></VisioRptDefField>\
<VisioRptDefFilter ID=\"1\" Property=\"Kosten\" Condition=\"3\" AndOr=\"1\" PropertyType=\"2\" PropertyDataType=\"2\" Value=\"100\" Unit=\"\"/>\
<VisioRptDefGroup ID=\"1\" GroupingField=\"0\" ItemDisplay=\"1\" GrandSummary=\"0\" ExcludeDuplicates=\"0\"/>\
<VisioRptDefSort ID=\"1\" SortField=\"2\" Ascending=\"1\" CaseSensitive=\"0\"/></VisioReportDefinition>";
    let bad = "<?xml version=\"1.0\"?><!DOCTYPE x [<!ENTITY e \"e\">]><VisioReportDefinition><Name>&e;</Name></VisioReportDefinition>";
    let shapes: String = DEVICES
        .iter()
        .enumerate()
        .map(|(i, (name, _, cost, _))| {
            let id = i + 1;
            format!("<Shape ID='{id}' Type='Shape' LineStyle='0' FillStyle='0' TextStyle='0'><Cell N='PinX' V='{id}'/><Cell N='PinY' V='1'/><Cell N='Width' V='1'/><Cell N='Height' V='0.5'/>\
<Section N='Property'><Row N='Kosten'><Cell N='Value' V='{cost}' U='CY'/><Cell N='Label' V='Kosten'/><Cell N='Type' V='7'/></Row></Section><Text>{name}</Text></Shape>")
        })
        .collect();
    let mut p = crate::pkg::minimal(&shapes, "");
    let doc_xml = format!(
        "{DECL}<VisioDocument {NS}><StyleSheets><StyleSheet ID='0' NameU='No Style'/></StyleSheets><DocumentSheet LineStyle='0' FillStyle='0' TextStyle='0'>\
<Section N='User'><Row N='visPropRpt1'><Cell N='Value' V='{}' U='STR'/><Cell N='Prompt' V=''/></Row><Row N='visPropRpt2'><Cell N='Value' V='{}' U='STR'/><Cell N='Prompt' V=''/></Row></Section></DocumentSheet></VisioDocument>",
        attr(def),
        attr(bad)
    );
    p.set("visio/document.xml", doc_xml);
    let Some(doc) = read(&p.bytes()) else {
        h.check("reports/visio/read", false, s("the Visio-like file does not read"));
        return;
    };
    let st = report::stored(&doc);
    let d = st.first().and_then(|x| x.def.clone().ok());
    let ok = d.as_ref().is_some_and(|d| d.name == "JD19 Bericht" && d.title == "JD19-Test" && d.precision == 1 && d.fields.len() == 2 && d.fields[0].property == Property::Builtin(Builtin::Text) && d.filters[0].condition == Condition::Gt && d.sorts.len() == 1);
    h.check("reports/visio/model", ok, s("visPropRpt1 as Visio writes it: name, title, fields, criterion, sort"));
    h.check("reports/visio/hostile", st.len() == 2 && st[1].def == Err(report::ReportError::Unsupported), format!("visPropRpt2 with a DOCTYPE refused, the file opens ({:?})", st.get(1).map(|x| &x.def)));
    let t = d.map(|d| report::run(&doc, &d, &report::Ctx { page: doc.pages().next(), ..Default::default() })).unwrap_or_default();
    let rows: Vec<Vec<String>> = t.rows.iter().map(|r| r.cells.iter().map(|c| c.text.clone()).collect()).collect();
    let want = vec![vec![s("Switch"), s("150.0")], vec![s("Router"), s("300.0")], vec![s("Server B"), s("800.5")], vec![s("Server A"), s("1200.0")], vec![s("Total"), s("2450.5")]];
    h.check("reports/visio/run", rows == want, format!("costs above 100, sorted, with the total: {rows:?}"));
    let back = read(&write(&doc));
    let same = back.as_ref().is_some_and(|b| {
        let a = report::stored(b);
        a.len() == 2 && a[0].def == st[0].def && a[1].xml == bad
    });
    h.check("reports/visio/rewrite", same, s("written back: the definition the same, the broken row unchanged"));
}

/// --write-report DIR: jd19-bericht.vsdx (the Visio counter-check)
pub(crate) fn write_check(dir: &str) -> i32 {
    let (doc, _) = drawing();
    let bytes = write(&doc);
    let out = format!("{dir}/jd19-bericht.vsdx");
    if bytes.is_empty() || !plat::write_file(&out, &bytes) {
        crate::out(&format!("cannot write {out}\n"));
        return 1;
    }
    crate::out(&format!("{out}: {} bytes\n", bytes.len()));
    0
}
