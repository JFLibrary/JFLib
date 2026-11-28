// data -- (JD18) the group "data": external data in VSDX files (DF-12) through diagram_io::data
// and diagram_core::data
//
//   run(h)            a drawing with two recordsets (an Excel workbook keyed by a column, a text
//                     file in row order) and linked shapes written and read back: the model the
//                     same, the bytes the same on the second write, Visio's part names, content
//                     types and relationships, the shape data and DataLinked; a package written
//                     the way Visio writes one (other r:ids, a Visio_RowID column, every
//                     attribute, conflicts, auto link comparisons, unknown attributes) read and
//                     written back; a refresh in the model saved and read again; data parts that
//                     are not understood kept as bytes; hostile data parts
//   write(dir, xlsx)  --write-data DIR [--xlsx PATH]: jd18-verknuepft.vsdx, the drawing for the
//                     Visio counter-check, its recordset reading PATH (default
//                     DIR\jd18-daten.xlsx, sheet "Geräte")
//
// The values: text with quotes, markup and a line break, numbers, a date with a time, a
// Boolean, an amount of money, empty cells.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use diagram_core::data::{self, Connection, DataValue, ExcelTable, Recordset};
use diagram_core::{DocKind, Document, Owner, ShapeKind, ShapeRef, SheetRef, Text};
use diagram_io::vsdx::{self, Measure};

use crate::pkg::{DECL, NS};
use crate::H;

fn s(x: &str) -> String {
    String::from(x)
}

fn t(x: &str) -> DataValue {
    DataValue::Text(s(x))
}

/// the devices of the drawing (the counter-check's workbook holds the same)
pub(crate) fn devices() -> (Vec<String>, Vec<Vec<DataValue>>) {
    let date = diagram_core::field::ole_days(2024, 2, 29, 8, 30, 0);
    (
        vec![s("Gerät"), s("IP-Adresse"), s("Ports"), s("Aktiv"), s("Seit"), s("Preis"), s("Notiz")],
        vec![
            vec![t("Server1"), t("10.0.0.1"), DataValue::Num(24.0), DataValue::Bool(true), DataValue::Date(date), DataValue::Currency(1999.5), t("Rack A, \"oben\" <1>")],
            vec![t("Server2"), t("10.0.0.2"), DataValue::Num(48.0), DataValue::Bool(false), DataValue::Date(date + 1.0), DataValue::Currency(2500.0), t("zwei\nZeilen")],
            vec![t("Switch"), t("10.0.0.3"), DataValue::Num(8.0), DataValue::Bool(true), DataValue::Empty, DataValue::Currency(-3.0), DataValue::Empty],
        ],
    )
}

// a drawing with three boxes named by their text, the devices linked by the column "Gerät", a
// second recordset in row order from a text file
fn drawing(xlsx: &str, csv: &str) -> (Document, Vec<ShapeRef>) {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().expect("page");
    let mut eng = diagram_core::formula::Engine::new();
    let mut tx = doc.transaction("data");
    let mut shapes = Vec::new();
    for (i, name) in ["Server1", "Server2", "Switch"].iter().enumerate() {
        let x = 1.0 + 2.5 * i as f64;
        let sh = diagram_core::ops::new_shape(&mut tx, &mut eng, Owner::Page(page), diagram_core::ops::Basic::Rectangle, [x, 6.0], [x + 1.8, 7.0])
            .unwrap_or_else(|_| tx.add_shape(Owner::Page(page), None, ShapeKind::Shape).expect("shape"));
        let _ = tx.set_text(sh, Some(Text::plain(name)));
        shapes.push(sh);
    }
    let (headers, rows) = devices();
    let mut rs = Recordset::from_table("Geräte", &headers, rows, &[s("Gerät")]);
    let (cs, cmd) = data::excel_connection(xlsx, &ExcelTable::Sheet(s("Geräte")), true);
    rs.command = Some(cmd.clone());
    rs.time_refreshed = Some(s("2026-11-23T14:05:00"));
    let conn = Connection { connection_string: Some(cs), command: Some(cmd), ..Connection::default() };
    let id = data::add_recordset(&mut tx, Some(conn), rs).expect("recordset");
    let _ = data::auto_link(&mut tx, &shapes, id, &[(0, data::ShapeField::Text)], false);
    let list = Recordset::from_table("Liste", &[s("Name"), s("Wert")], vec![vec![t("a"), DataValue::Num(1.0)], vec![t("b"), DataValue::Num(2.5)]], &[]);
    let (cs, cmd) = data::text_connection(csv, true);
    let mut list = list;
    list.command = Some(cmd.clone());
    let id2 = data::add_recordset(&mut tx, Some(Connection { connection_string: Some(cs), command: Some(cmd), ..Connection::default() }), list).expect("list");
    let _ = data::link(&mut tx, shapes[2], id2, 2);
    tx.commit();
    (doc, shapes)
}

fn write(doc: &Document) -> Vec<u8> {
    vsdx::write(doc, &vsdx::WriteOptions::DEFAULT).unwrap_or_default()
}

fn read(bytes: &[u8]) -> Option<Document> {
    vsdx::read(bytes, &vsdx::Limits::VSDX).ok().map(|l| l.doc)
}

fn field(doc: &Document, shape: ShapeRef, name: &str, cell: &str) -> String {
    doc.shape_data(SheetRef::Shape(shape))
        .into_iter()
        .find(|r| r.name() == Some(name))
        .map(|r| match (r.text(cell), r.num(cell)) {
            (Some(t), _) => String::from(t),
            (None, Some(x)) => format!("{x}"),
            _ => String::new(),
        })
        .unwrap_or_else(|| s("<none>"))
}

/// the group "data"
pub(crate) fn run(h: &mut H) {
    own_round_trip(h);
    visio_like(h);
    visio_stale_keys(h);
    refresh_saved(h);
    kept_and_hostile(h);
}

// (Visio check 2026-11-27) Visio 16 saves a drawing whose rows it has not loaded with every
// RowKeyValue Value the first row's key, the RowIDs in row order; its columns carry
// LangID='de-DE', its data part has no XML declaration, names without rs:name and hex
// character references.  The rows keep their IDs by the RowIDs' order, the links stay, and
// writing it again repairs the key values.
fn visio_stale_keys(h: &mut H) {
    let mut p = crate::pkg::minimal(&format!("{}{}{}", crate::pkg::shape(4), crate::pkg::shape(5), crate::pkg::shape(6)), "");
    p.rel("visio/document.xml", "rId8", "recordsets", "data/recordsets.xml");
    p.part(
        "visio/data/recordsets.xml",
        "application/vnd.ms-visio.recordsets+xml",
        format!("{DECL}<DataRecordSets NextID='2' ActiveRecordsetID='1' {NS}><DataRecordSet ID='1' Command='select * from `Ger\u{e4}te$`' Name='Ger\u{e4}te' NextRowID='4' Checksum='2256283350'><Rel r:id='rId1'/>\
<DataColumns><DataColumn ColumnNameID='Ger\u{e4}t' Name='Ger\u{e4}t' Label='Ger\u{e4}t' LangID='de-DE' DisplayOrder='0'/><DataColumn ColumnNameID='Ports' Name='Ports' Label='Ports' LangID='de-DE' DataType='2' DisplayOrder='1'/></DataColumns>\
<PrimaryKey ColumnNameID='Ger\u{e4}t'><RowKeyValue RowID='1' Value='Server1'/><RowKeyValue RowID='2' Value='Server1'/><RowKeyValue RowID='3' Value='Server1'/></PrimaryKey>\
<RowMap RowID='1' PageID='0' ShapeID='4'/><RowMap RowID='2' PageID='0' ShapeID='5'/><RowMap RowID='3' PageID='0' ShapeID='6'/></DataRecordSet></DataRecordSets>"),
    );
    p.rel("visio/data/recordsets.xml", "rId1", "recordset", "recordset1.xml");
    p.part(
        "visio/data/recordset1.xml",
        "application/vnd.ms-visio.recordset+xml",
        "<xml xmlns:s=\"uuid:BDC6E3F0-6DA3-11d1-A2A3-00AA00C14882\" xmlns:dt=\"uuid:C2F41010-65B3-11d1-A29F-00AA00C14882\" xmlns:rs=\"urn:schemas-microsoft-com:rowset\" xmlns:z=\"#RowsetSchema\">\r\n\t<s:Schema id=\"RowsetSchema\">\r\n\t\t<s:ElementType name=\"row\" content=\"eltOnly\" rs:updatable=\"true\">\r\n\
\t\t\t<s:AttributeType name=\"c0\" rs:name=\"Ger\u{e4}t\" rs:number=\"1\" rs:nullable=\"true\"><s:datatype dt:type=\"string\" dt:maxLength=\"255\"/></s:AttributeType>\r\n\
\t\t\t<s:AttributeType name=\"Ports\" rs:number=\"2\" rs:nullable=\"true\"><s:datatype dt:type=\"float\" dt:maxLength=\"8\"/></s:AttributeType>\r\n\
\t\t\t<s:extends type=\"rs:rowbase\"/>\r\n\t\t</s:ElementType>\r\n\t</s:Schema>\r\n\t<rs:data>\r\n\
\t\t<z:row c0=\"Server1\" Ports=\"24\"/>\r\n\t\t<z:row c0=\"Server2\" Ports=\"48\"/>\r\n\t\t<z:row c0=\"Switch&#xA;B\" Ports=\"8\"/>\r\n\t</rs:data>\r\n</xml>",
    );
    let Some(doc) = read(&p.bytes()) else {
        h.check("data/visio-stale/read", false, s("the file does not read"));
        return;
    };
    let rs = doc.data.recordsets.first();
    let ids: Vec<(u32, String)> = rs.map(|r| r.rows.iter().map(|x| (x.id, x.values[0].text())).collect()).unwrap_or_default();
    h.check("data/visio-stale/ids", ids == vec![(1, s("Server1")), (2, s("Server2")), (3, s("Switch\nB"))], format!("{ids:?}"));
    h.check("data/visio-stale/lang", rs.is_some_and(|r| r.columns[0].lang.as_deref() == Some("de-DE")), s("LangID as Visio writes it"));
    let keys: Vec<String> = rs.map(|r| r.key[0].values.iter().map(|(_, v)| v.clone()).collect()).unwrap_or_default();
    h.check("data/visio-stale/keys", keys == vec![s("Server1"), s("Server2"), s("Switch\nB")], format!("{keys:?}"));
    let shape5 = doc.page_by_id(0).and_then(|pg| doc.shape_by_id(diagram_core::Owner::Page(pg), 5));
    h.check("data/visio-stale/link", shape5.is_some_and(|sh| doc.data_link(sh, 1) == Some(2)), s("shape 5 stays on row 2"));
    let back = read(&write(&doc));
    let xml_ok = back.is_some_and(|b| b.data == doc.data);
    h.check("data/visio-stale/rewrite", xml_ok, s("written and read again: the same model, LangID kept"));
}

fn own_round_trip(h: &mut H) {
    let (doc, shapes) = drawing("C:\\Daten\\geräte.xlsx", "C:\\Daten\\liste.csv");
    let linked = shapes.iter().filter(|&&s| doc.data_link(s, 1).is_some()).count();
    h.check("data/link/auto", linked == 3, format!("{linked} of 3 shapes linked by their text"));
    let bytes = write(&doc);
    let Some(back) = read(&bytes) else {
        h.check("data/roundtrip/read", false, s("the written file does not read"));
        return;
    };
    h.check("data/roundtrip/model", back.data == doc.data, format!("{} recordsets, {} connections", back.data.recordsets.len(), back.data.connections.len()));
    let again = write(&back);
    h.check("data/roundtrip/bytes", again == bytes, format!("{} bytes, written again the same", bytes.len()));
    let pkg = archive::opc::Package::read(&bytes, archive::Limits::default(), archive::opc::Rules::DEFAULT);
    let Ok(pkg) = pkg else {
        h.check("data/roundtrip/package", false, s("not a package"));
        return;
    };
    let ct = |n: &str| pkg.content_type(n).unwrap_or("-");
    h.check(
        "data/parts/types",
        ct("visio/data/connections.xml") == "application/vnd.ms-visio.connections+xml"
            && ct("visio/data/recordsets.xml") == "application/vnd.ms-visio.recordsets+xml"
            && ct("visio/data/recordset1.xml") == "application/vnd.ms-visio.recordset+xml"
            && ct("visio/data/recordset2.xml") == "application/vnd.ms-visio.recordset+xml",
        s("connections.xml, recordsets.xml, recordset1.xml, recordset2.xml with Visio's content types"),
    );
    let main = pkg.relationships("visio/document.xml").unwrap_or_default();
    let has = |k: &str| main.iter().any(|r| r.kind.ends_with(k));
    h.check("data/parts/rels", has("/connections") && has("/recordsets"), s("the main part names connections.xml and recordsets.xml"));
    let rs_rels = pkg.relationships("visio/data/recordsets.xml").unwrap_or_default();
    h.check("data/parts/recordset-rels", rs_rels.len() == 2 && rs_rels.iter().all(|r| r.kind.ends_with("/recordset")), format!("{} relationships from recordsets.xml", rs_rels.len()));
    let xml = pkg.part("visio/data/recordsets.xml").map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
    h.check("data/parts/rowmap", xml.matches("<RowMap ").count() == 4, format!("{} RowMap elements", xml.matches("<RowMap ").count()));
    h.check("data/parts/key", xml.contains("<PrimaryKey ColumnNameID='Gerät'><RowKeyValue RowID='1' Value='Server1'/>") && xml.contains("RowOrder='0'"), s("the key column with its values, RowOrder 0"));
    let ado = pkg.part("visio/data/recordset1.xml").map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
    h.check("data/parts/ado", ado.contains("rs:name='IP-Adresse'") && ado.contains("dt:type='dateTime'") && ado.contains("c7='zwei&#10;Zeilen'"), s("ADO recordset: names, types, a line break in a value"));
    // the shape data after reading
    let (pid, sid) = doc.data_ids(shapes[0]).unwrap_or((0, 0));
    let shape = back.data_shape(pid, sid);
    match shape {
        Some(sh) => {
            let ok = field(&back, sh, "_VisDM_IPAdresse", "Value") == "10.0.0.1" && field(&back, sh, "_VisDM_Ports", "Value") == "24" && field(&back, sh, "_VisDM_Aktiv", "Value") == "TRUE" && field(&back, sh, "_VisDM_Ports", "DataLinked") == "1";
            h.check("data/shape/fields", ok, s("Prop rows of the linked shape: values, DataLinked"));
            h.check("data/shape/label", field(&back, sh, "_VisDM_IPAdresse", "Label") == "IP-Adresse", s("the column's label as the field's label"));
        }
        None => h.check("data/shape/fields", false, s("the linked shape is gone")),
    }
}

// a package as Visio writes one (made here from the model's own XML with Visio's habits:
// other r:ids and names, a Visio_RowID column, every attribute, conflicts, comparisons)
fn visio_like(h: &mut H) {
    let mut p = crate::pkg::minimal(&crate::pkg::shape(1), "");
    p.rel("visio/document.xml", "rId7", "connections", "data/connections.xml");
    p.rel("visio/document.xml", "rId8", "recordsets", "data/recordsets.xml");
    p.part(
        "visio/data/connections.xml",
        "application/vnd.ms-visio.connections+xml",
        format!("{DECL}<DataConnections {NS} NextID='2'><DataConnection ID='1' FileName='' ConnectionString='Provider=Microsoft.ACE.OLEDB.12.0;User ID=Admin;Data Source=C:\\Daten\\b.xlsx;Mode=Read;Extended Properties=&quot;HDR=YES;IMEX=1;MaxScanRows=0;Excel 12.0;&quot;' Command='select * from `Tabelle1$`' FriendlyName='b Tabelle1$' Timeout='30' AlwaysUseConnectionFile='0' Zukunft='x'/></DataConnections>"),
    );
    p.part(
        "visio/data/recordsets.xml",
        "application/vnd.ms-visio.recordsets+xml",
        format!("{DECL}<DataRecordSets {NS} NextID='2' ActiveRecordsetID='1' DataWindowOrder='1'><DataRecordSet ID='1' ConnectionID='1' Command='select * from `Tabelle1$`' Options='0' TimeRefreshed='2026-11-20T09:15:00' NextRowID='5' Name='Tabelle1' RowOrder='0' RefreshOverwriteAll='0' RefreshNoReconciliationUI='0' RefreshInterval='5' ReplaceLinks='0' Checksum='12345'><Rel r:id='rId3'/>\
<DataColumns SortColumn='Name' SortAsc='1'><DataColumn ColumnNameID='Name' Name='Name' Label='Name' OrigLabel='Name' LangID='de-DE' Calendar='0' DataType='0' UnitType='0' Currency='0' Degree='0' DisplayWidth='100' DisplayOrder='0' Mapped='1' Hyperlink='0'/>\
<DataColumn ColumnNameID='Wert' Name='Wert' Label='Wert (Euro)' OrigLabel='Wert' LangID='de-DE' Calendar='0' DataType='7' UnitType='0' Currency='111' Degree='0' DisplayWidth='100' DisplayOrder='1' Mapped='1' Hyperlink='0'/></DataColumns>\
<PrimaryKey ColumnNameID='Name'><RowKeyValue RowID='2' Value='Alpha'/><RowKeyValue RowID='4' Value='Beta'/></PrimaryKey>\
<RowMap RowID='2' PageID='0' ShapeID='1'/><RefreshConflict RowID='3' PageID='0' ShapeID='9'/><AutoLinkComparison ColumnName='Name' ContextType='3' ContextTypeLabel='Shape-Text'/></DataRecordSet></DataRecordSets>"),
    );
    p.rel("visio/data/recordsets.xml", "rId3", "recordset", "recordset7.xml");
    p.part(
        "visio/data/recordset7.xml",
        "application/vnd.ms-visio.recordset+xml",
        "<xml xmlns:s='uuid:BDC6E3F0-6DA3-11d1-A2A3-00AA00C14882' xmlns:dt='uuid:C2F41010-65B3-11d1-A29F-00AA00C14882' xmlns:rs='urn:schemas-microsoft-com:rowset' xmlns:z='#RowsetSchema'><s:Schema id='RowsetSchema'><s:ElementType name='row' content='eltOnly'>\
<s:AttributeType name='c0' rs:name='Visio_RowID'><s:datatype dt:type='int' dt:maxLength='4'/></s:AttributeType>\
<s:AttributeType name='Name' rs:number='1'><s:datatype dt:type='string' dt:maxLength='255'/></s:AttributeType>\
<s:AttributeType name='Wert' rs:number='2'><s:datatype dt:type='number' rs:dbtype='currency' dt:maxLength='8'/></s:AttributeType>\
<s:extends type='rs:rowbase'/></s:ElementType></s:Schema><rs:data><z:row c0='2' Name='Alpha' Wert='12.5'/><z:row c0='4' Name='Beta'/></rs:data></xml>",
    );
    let Some(doc) = read(&p.bytes()) else {
        h.check("data/visio/read", false, s("the Visio-like file does not read"));
        return;
    };
    let rs = doc.data.recordsets.first();
    let ok = rs.is_some_and(|r| {
        r.rows.len() == 2
            && r.rows[0].id == 2
            && r.rows[0].values == vec![t("Alpha"), DataValue::Currency(12.5)]
            && r.rows[1].id == 4
            && r.rows[1].values[1] == DataValue::Empty
            && r.refresh_interval == 5
            && r.checksum == Some(12345)
            && r.links.len() == 1
            && r.conflicts.len() == 1
            && r.auto_link.len() == 1
            && r.columns[1].label == "Wert (Euro)"
    });
    h.check("data/visio/model", ok, s("rows with Visio_RowID, typed values, interval, conflicts, comparisons"));
    let conn_ok = doc.data.connections.first().is_some_and(|c| c.attrs.iter().any(|(k, v)| k == "Zukunft" && v == "x") && c.timeout == Some(30));
    h.check("data/visio/unknown", conn_ok, s("an unknown attribute of DataConnection kept"));
    let src = rs.map(|r| data::source_of(&doc.data, r));
    h.check("data/visio/source", src == Some(data::Source::Excel { path: s("C:\\Daten\\b.xlsx"), table: ExcelTable::Sheet(s("Tabelle1")), header: true }), format!("{src:?}"));
    let bytes = write(&doc);
    let back = read(&bytes);
    h.check("data/visio/rewrite", back.as_ref().is_some_and(|b| b.data == doc.data), s("written as JFDiagrammer writes it and read back: the same model"));
    let xml = archive::opc::Package::read(&bytes, archive::Limits::default(), archive::opc::Rules::DEFAULT)
        .ok()
        .and_then(|p| p.part("visio/data/recordsets.xml").map(|b| String::from_utf8_lossy(b).into_owned()))
        .unwrap_or_default();
    h.check("data/visio/attrs", xml.contains("Checksum='12345'") && xml.contains("DataWindowOrder='1'") && xml.contains("<AutoLinkComparison ColumnName='Name' ContextType='3' ContextTypeLabel='Shape-Text'/>"), s("every attribute written back"));
}

// a refresh in the model (a row gone, a value changed, the user's own value) saved and read
fn refresh_saved(h: &mut H) {
    let (mut doc, shapes) = drawing("C:\\Daten\\geräte.xlsx", "C:\\Daten\\liste.csv");
    let mut tx = doc.transaction("user");
    let _ = data::user_edited(&mut tx, SheetRef::Shape(shapes[0]), "_VisDM_Ports");
    tx.commit();
    let (headers, mut rows) = devices();
    rows.remove(1);
    rows[0][1] = t("10.0.1.1");
    rows[0][2] = DataValue::Num(32.0);
    let mut tx = doc.transaction("refresh");
    let r = data::refresh(&mut tx, 1, &headers, rows, "2026-11-23T15:00:00");
    tx.commit();
    let Ok(r) = r else {
        h.check("data/refresh/run", false, s("refresh refused"));
        return;
    };
    h.check("data/refresh/conflicts", r.conflicts.len() == 2, format!("{} conflicts (row gone, user value)", r.conflicts.len()));
    let back = read(&write(&doc));
    let ok = back.as_ref().is_some_and(|b| {
        let rs = &b.data.recordsets[0];
        rs.conflicts.len() == 1 && rs.rows.len() == 2 && rs.time_refreshed.as_deref() == Some("2026-11-23T15:00:00")
    });
    h.check("data/refresh/saved", ok, s("the refreshed rows, the time and the conflict as RefreshConflict"));
    let ip = back.as_ref().and_then(|b| doc.data_ids(shapes[0]).and_then(|(p, s)| b.data_shape(p, s)).map(|sh| field(b, sh, "_VisDM_IPAdresse", "Value")));
    h.check("data/refresh/value", ip.as_deref() == Some("10.0.1.1"), format!("{ip:?}"));
}

// parts the reader does not understand stay as they are; hostile parts are refused as data
fn kept_and_hostile(h: &mut H) {
    let mut p = crate::pkg::minimal(&crate::pkg::shape(1), "");
    p.rel("visio/document.xml", "rId8", "recordsets", "data/recordsets.xml");
    let odd = format!("{DECL}<DataRecordSets {NS} NextID='2'><DataRecordSet ID='1'><Rel r:id='rId1'/><DataColumns><DataColumn ColumnNameID='a' Name='a' Label='a'/></DataColumns><Neu/></DataRecordSet></DataRecordSets>");
    p.part("visio/data/recordsets.xml", "application/vnd.ms-visio.recordsets+xml", odd.clone());
    let doc = read(&p.bytes());
    let kept = doc.as_ref().is_some_and(|d| d.data.recordsets.is_empty() && d.parts.iter().any(|x| d.str(x.name).ends_with("recordsets.xml") && x.data == odd.as_bytes()));
    h.check("data/kept/unknown-child", kept, s("a DataRecordSet with an unknown child: the parts stay bytes"));
    let back = doc.as_ref().and_then(|d| read(&write(d)));
    h.check("data/kept/written", back.is_some_and(|d| d.parts.iter().any(|x| x.data == odd.as_bytes())), s("written back unchanged"));
    // a recordset part that is no ADO data: kept as the recordset's raw bytes
    let mut p = crate::pkg::minimal(&crate::pkg::shape(1), "");
    p.rel("visio/document.xml", "rId8", "recordsets", "data/recordsets.xml");
    p.part("visio/data/recordsets.xml", "application/vnd.ms-visio.recordsets+xml", format!("{DECL}<DataRecordSets {NS} NextID='2'><DataRecordSet ID='1'><Rel r:id='rId1'/><DataColumns><DataColumn ColumnNameID='a' Name='a' Label='a'/></DataColumns></DataRecordSet></DataRecordSets>"));
    p.rel("visio/data/recordsets.xml", "rId1", "recordset", "recordset1.xml");
    p.part("visio/data/recordset1.xml", "application/vnd.ms-visio.recordset+xml", "kein XML <<<");
    let doc = read(&p.bytes());
    let raw = doc.as_ref().is_some_and(|d| d.data.recordsets.first().is_some_and(|r| r.raw.as_deref().map(|v| v.as_slice()) == Some(b"kein XML <<<".as_slice())));
    h.check("data/kept/raw", raw, s("a recordset part that is no ADO data kept as bytes"));
    let back = doc.as_ref().and_then(|d| read(&write(d)));
    h.check("data/kept/raw-written", back.is_some_and(|d| d.data.recordsets.first().is_some_and(|r| r.raw.is_some())), s("and written back"));
    // hostile: deep nesting and a huge row; refused as data (the file still opens) and quickly
    let mut p = crate::pkg::minimal(&crate::pkg::shape(1), "");
    p.rel("visio/document.xml", "rId8", "recordsets", "data/recordsets.xml");
    let deep = format!("{DECL}<DataRecordSets {NS} NextID='2'>{}{}</DataRecordSets>", "<x>".repeat(100_000), "</x>".repeat(100_000));
    p.part("visio/data/recordsets.xml", "application/vnd.ms-visio.recordsets+xml", deep);
    let t0 = plat::time_us();
    let doc = read(&p.bytes());
    let us = plat::time_us().wrapping_sub(t0);
    h.check("data/hostile/deep", doc.is_some_and(|d| d.data.recordsets.is_empty()) && us < crate::QUICK_US, format!("100000 levels: data refused, file opens ({} ms)", us / 1000));
    let mut p = crate::pkg::minimal(&crate::pkg::shape(1), "");
    p.rel("visio/document.xml", "rId8", "recordsets", "data/recordsets.xml");
    p.part("visio/data/recordsets.xml", "application/vnd.ms-visio.recordsets+xml", format!("{DECL}<DataRecordSets {NS} NextID='2'><DataRecordSet ID='1'><Rel r:id='rId1'/><DataColumns><DataColumn ColumnNameID='a' Name='a' Label='a'/></DataColumns></DataRecordSet></DataRecordSets>"));
    p.rel("visio/data/recordsets.xml", "rId1", "recordset", "recordset1.xml");
    let wide: String = (0..40_000).map(|i| format!(" x{i}='y'")).collect();
    p.part("visio/data/recordset1.xml", "application/vnd.ms-visio.recordset+xml", format!("<xml><rs:data xmlns:rs='urn:schemas-microsoft-com:rowset' xmlns:z='#RowsetSchema'><z:row{wide}/></rs:data></xml>"));
    let t0 = plat::time_us();
    let doc = read(&p.bytes());
    let us = plat::time_us().wrapping_sub(t0);
    h.check("data/hostile/wide", doc.is_some_and(|d| d.data.recordsets.first().is_some_and(|r| r.raw.is_some())) && us < crate::QUICK_US, format!("40000 attributes on a row: kept as bytes ({} ms)", us / 1000));
}

/// --write-data DIR [--xlsx PATH]: the drawing for the Visio counter-check, and next to it the
/// text file of its second recordset (jd18-liste.csv, comma-separated as the ACE text driver
/// reads by default)
pub(crate) fn write_check(dir: &str, xlsx: Option<&str>) -> i32 {
    let default = format!("{dir}\\jd18-daten.xlsx");
    let path = xlsx.unwrap_or(&default);
    let csv = format!("{dir}\\jd18-liste.csv");
    let (doc, _) = drawing(path, &csv);
    let bytes = write(&doc);
    let out = format!("{dir}/jd18-verknuepft.vsdx");
    if bytes.is_empty() || !plat::write_file(&out, &bytes) || !plat::write_file(&format!("{dir}/jd18-liste.csv"), b"Name,Wert\r\na,1\r\nb,2.5\r\n") {
        crate::out(&format!("cannot write {out}\n"));
        return 1;
    }
    crate::out(&format!("{out}: {} bytes, data from {path} and {csv}\n", bytes.len()));
    0
}

/// --data-dump FILE: the data parts of a drawing as diagram_io reads them
pub(crate) fn dump(file: &str) -> i32 {
    let Some(bytes) = plat::read_file(file) else {
        crate::out(&format!("{file}: cannot read\n"));
        return 1;
    };
    let doc = match vsdx::read(&bytes, &vsdx::Limits::VSDX) {
        Ok(l) => l.doc,
        Err(e) => {
            crate::out(&format!("{file}: {e:?}\n"));
            return 1;
        }
    };
    let kept = doc.parts.iter().filter(|p| doc.str(p.name).contains("/data/")).count();
    crate::out(&format!("{file}: {} connections, {} recordsets, {kept} data parts kept as bytes\n", doc.data.connections.len(), doc.data.recordsets.len()));
    for c in &doc.data.connections {
        crate::out(&format!("  connection {}: {:?} {:?}\n", c.id, c.connection_string, c.command));
    }
    for rs in &doc.data.recordsets {
        crate::out(&format!("  recordset {} {:?} conn {:?} row_order {} overwrite_all {} interval {} checksum {:?} raw {}\n", rs.id, rs.name, rs.connection, rs.row_order, rs.overwrite_all, rs.refresh_interval, rs.checksum, rs.raw.is_some()));
        for c in &rs.columns {
            crate::out(&format!("    column {:?} name {:?} label {:?} type {} lang {:?}\n", c.id, c.name, c.label, c.data_type, c.lang));
        }
        for k in &rs.key {
            crate::out(&format!("    key {:?}: {:?}\n", k.column, k.values));
        }
        for r in rs.rows.iter() {
            crate::out(&format!("    row {}: {}\n", r.id, r.values.iter().map(|v| v.text()).collect::<Vec<_>>().join(" | ")));
        }
        for l in &rs.links {
            crate::out(&format!("    link row {} -> page {} shape {}\n", l.row, l.page, l.shape));
        }
        for l in &rs.conflicts {
            crate::out(&format!("    conflict row {} -> page {} shape {}\n", l.row, l.page, l.shape));
        }
    }
    0
}
