// tests of data.rs: the data parts written and read back, ADO data, what stays a part

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use diagram_core::data::{self, Connection, DataValue, Recordset};
use diagram_core::{DocKind, Owner, ShapeKind, ShapeRef, SheetRef, Text};

use super::*;
use crate::vsdx::{self, Measure};

fn s(x: &str) -> String {
    String::from(x)
}

// a drawing with two shapes, a recordset keyed by "Gerät" (row 1 on shape a) and one in row
// order without connection
fn drawing() -> (Document, ShapeRef, ShapeRef) {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().unwrap();
    let mut tx = doc.transaction("setup");
    let a = tx.add_shape(Owner::Page(page), None, ShapeKind::Shape).unwrap();
    tx.set_text(a, Some(Text::plain("Server1"))).unwrap();
    let b = tx.add_shape(Owner::Page(page), None, ShapeKind::Shape).unwrap();
    let headers = vec![s("Gerät"), s("IP-Adresse"), s("Ports"), s("Aktiv"), s("Seit"), s("Preis"), s("Notiz")];
    let date = diagram_core::field::ole_days(2024, 2, 29, 8, 30, 0);
    let rows = vec![
        vec![DataValue::Text(s("Server1")), DataValue::Text(s("10.0.0.1")), DataValue::Num(24.0), DataValue::Bool(true), DataValue::Date(date), DataValue::Currency(1999.5), DataValue::Text(s("a'b \"c\" <d> & e\nzweite Zeile"))],
        vec![DataValue::Text(s("Server2")), DataValue::Empty, DataValue::Num(0.25), DataValue::Bool(false), DataValue::Empty, DataValue::Currency(-3.0), DataValue::Empty],
    ];
    let mut rs = Recordset::from_table("Geräte", &headers, rows, &[s("Gerät")]);
    let (cs, cmd) = data::excel_connection("C:\\Daten\\geräte.xlsx", &data::ExcelTable::Sheet(s("Tabelle1")), true);
    rs.command = Some(cmd.clone());
    rs.time_refreshed = Some(s("2026-11-23T14:05:00"));
    rs.refresh_interval = 15;
    let conn = Connection { connection_string: Some(cs), command: Some(cmd), ..Connection::default() };
    let id = data::add_recordset(&mut tx, Some(conn), rs).unwrap();
    data::link(&mut tx, a, id, 1).unwrap();
    let rs2 = Recordset::from_table("Liste", &[s("A")], vec![vec![DataValue::Text(s("x"))], vec![DataValue::Text(s("y"))]], &[]);
    let id2 = data::add_recordset(&mut tx, None, rs2).unwrap();
    data::link(&mut tx, b, id2, 2).unwrap();
    tx.commit();
    (doc, a, b)
}

fn round(doc: &Document) -> (Document, Vec<u8>) {
    let bytes = vsdx::write(doc, &vsdx::WriteOptions::DEFAULT).expect("write");
    let back = vsdx::read(&bytes, &vsdx::Limits::VSDX).expect("read");
    (back.doc, bytes)
}

#[test]
fn data_round_trip() {
    let (doc, a, _) = drawing();
    let (back, bytes) = round(&doc);
    assert_eq!(back.data, doc.data);
    // no data part is left as a kept part
    assert!(!back.parts.iter().any(|p| back.str(p.name).contains("/data/")));
    // the shape data written by link survive with their link
    let shape = back.data_shape(doc.data_ids(a).unwrap().0, doc.data_ids(a).unwrap().1).unwrap();
    assert_eq!(back.data_link(shape, 1), Some(1));
    let ports = back.shape_data(SheetRef::Shape(shape)).into_iter().find(|r| r.name() == Some("_VisDM_Ports")).unwrap();
    assert_eq!(ports.num("Value"), Some(24.0));
    assert_eq!(ports.num("DataLinked"), Some(1.0));
    // written again: the same bytes
    let (_, again) = round(&back);
    assert_eq!(bytes, again);
    // the package has Visio's names and relationships
    let pkg = archive::opc::Package::read(&bytes, archive::Limits::default(), archive::opc::Rules::DEFAULT).unwrap();
    assert_eq!(pkg.content_type("visio/data/connections.xml"), Some(CT_CONNECTIONS));
    assert_eq!(pkg.content_type("visio/data/recordsets.xml"), Some(CT_RECORDSETS));
    assert_eq!(pkg.content_type("visio/data/recordset1.xml"), Some(CT_RECORDSET));
    assert_eq!(pkg.content_type("visio/data/recordset2.xml"), Some(CT_RECORDSET));
    let rels = pkg.relationships("visio/data/recordsets.xml").unwrap();
    assert_eq!(rels.len(), 2);
    let main = pkg.relationships("visio/document.xml").unwrap();
    assert!(main.iter().any(|r| r.kind == REL_CONNECTIONS));
    assert!(main.iter().any(|r| r.kind == REL_RECORDSETS));
    let rs_xml = core::str::from_utf8(pkg.part("visio/data/recordsets.xml").unwrap()).unwrap();
    assert!(rs_xml.contains("<Rel r:id='rId1'/>"), "{rs_xml}");
    assert!(rs_xml.contains("RowOrder='0'"));
    assert!(rs_xml.contains("<PrimaryKey ColumnNameID='Gerät'><RowKeyValue RowID='1' Value='Server1'/>"));
    let (pid, sid) = doc.data_ids(a).unwrap();
    assert!(rs_xml.contains(&alloc::format!("<RowMap RowID='1' PageID='{pid}' ShapeID='{sid}'/>")), "{rs_xml}");
    assert!(rs_xml.contains("Command='select * from `Tabelle1$`'"));
    assert!(rs_xml.contains("RefreshInterval='15'"));
    assert!(rs_xml.contains("Command='select * from `Tabelle1$`'"));
    let ado = core::str::from_utf8(pkg.part("visio/data/recordset1.xml").unwrap()).unwrap();
    assert!(ado.contains("rs:name='IP-Adresse'"));
    assert!(ado.contains("c5='2024-02-29T08:30:00'"), "{ado}");
    assert!(ado.contains("c4='True'"));
    assert!(ado.contains("dt:type='number' rs:dbtype='currency'"));
}

#[test]
fn ado_with_row_ids_and_odd_values() {
    let mut rs = Recordset::from_table("x", &[s("Name"), s("Wert")], Vec::new(), &[]);
    rs.columns[1].data_type = data::kind::NUMBER;
    let xml = "<xml xmlns:s='uuid:BDC6E3F0-6DA3-11d1-A2A3-00AA00C14882' xmlns:dt='uuid:C2F41010-65B3-11d1-A29F-00AA00C14882' xmlns:rs='urn:schemas-microsoft-com:rowset' xmlns:z='#RowsetSchema'>\
<s:Schema id='RowsetSchema'><s:ElementType name='row' content='eltOnly'>\
<s:AttributeType name='c0' rs:name='Visio_RowID'><s:datatype dt:type='int'/></s:AttributeType>\
<s:AttributeType name='Name'><s:datatype dt:type='string'/></s:AttributeType>\
<s:AttributeType name='c2' rs:name='Wert'><s:datatype dt:type='float'/></s:AttributeType>\
<s:AttributeType name='c3' rs:name='Unbekannt'><s:datatype dt:type='string'/></s:AttributeType>\
</s:ElementType></s:Schema><rs:data>\
<z:row c0='7' Name='Seattle' c2='1.5E3' c3='weg'/>\
<z:row c0='7' Name='Redmond' c2='kein Wert'/>\
<z:row Name='&#10;neu'/>\
</rs:data></xml>";
    let rows = read_ado(xml.as_bytes(), &rs, 16, 64).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].id, 7);
    // a repeated or missing row ID gets the next free one
    assert_ne!(rows[1].id, 7);
    assert_ne!(rows[2].id, rows[1].id);
    assert_eq!(rows[0].values, vec![DataValue::Text(s("Seattle")), DataValue::Num(1500.0)]);
    assert_eq!(rows[1].values[1], DataValue::Text(s("kein Wert")));
    assert_eq!(rows[2].values[0], DataValue::Text(s("\nneu")));
    assert!(read_ado(b"<notado/>", &rs, 16, 64).is_none());
    assert!(read_ado(b"<xml><broken></xml>", &rs, 16, 64).is_none());
}

#[test]
fn ids_from_key_values_without_row_id_column() {
    let (doc, _, _) = drawing();
    let mut rs = doc.data.recordsets[0].clone();
    // rows written without Visio_RowID come back with the IDs their keys have
    rs.rows = alloc::sync::Arc::new(rs.rows.iter().rev().cloned().collect());
    let xml = ado(&rs);
    let mut probe = rs.clone();
    probe.rows = alloc::sync::Arc::new(Vec::new());
    let rows = read_ado(xml.as_bytes(), &probe, 16, 64).unwrap();
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![2, 1]);
}

// data parts the reader does not understand stay parts and are written back unchanged
#[test]
fn not_understood_stays_a_part() {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let bad = b"<?xml version='1.0'?><DataRecordSets xmlns='http://schemas.microsoft.com/office/visio/2012/main' NextID='2'><DataRecordSet ID='1'><Strange/></DataRecordSet></DataRecordSets>".to_vec();
    let part = Part {
        name: doc.intern("/visio/data/recordsets.xml"),
        content_type: doc.intern(CT_RECORDSETS),
        rel_type: doc.intern(REL_RECORDSETS),
        data: bad.clone(),
    };
    doc.parts.push(part);
    let (back, _) = round(&doc);
    assert!(back.data.recordsets.is_empty());
    let kept = back.parts.iter().find(|p| back.str(p.content_type) == CT_RECORDSETS).unwrap();
    assert_eq!(kept.data, bad);
}

#[test]
fn hostile_data_parts() {
    let lim = vsdx::Limits::VSDX;
    // deep nesting, a duplicate ID, a non-number, a missing ColumnNameID: all refused as data
    let deep = alloc::format!("<DataRecordSets NextID='1'>{}{}</DataRecordSets>", "<a>".repeat(5000), "</a>".repeat(5000));
    for xml in [
        deep.as_str(),
        "<DataRecordSets NextID='1'><DataRecordSet ID='1'><DataColumns/></DataRecordSet><DataRecordSet ID='1'><DataColumns/></DataRecordSet></DataRecordSets>",
        "<DataRecordSets NextID='x'/>",
        "<DataRecordSets NextID='1'><DataRecordSet ID='1'><DataColumns><DataColumn Name='a'/></DataColumns></DataRecordSet></DataRecordSets>",
        "<!DOCTYPE x [<!ENTITY a 'b'>]><DataRecordSets NextID='1'/>",
    ] {
        let mut d = DataSets::default();
        let ok = tree(xml.as_bytes(), lim.depth, 64, 1000).map(|t| recordsets(&t, &mut d).is_ok()).unwrap_or(false);
        assert!(!ok, "{xml}");
    }
    // too many elements
    let many = alloc::format!("<DataConnections NextID='1'>{}</DataConnections>", "<DataConnection ID='1' FileName=''/>".repeat(50));
    assert!(tree(many.as_bytes(), 16, 64, 10).is_none());
    // a z:row with more attributes than allowed
    let rs = Recordset::default();
    let wide = alloc::format!("<xml><rs:data xmlns:rs='urn:schemas-microsoft-com:rowset' xmlns:z='#RowsetSchema'><z:row {}/></rs:data></xml>", (0..100).map(|i| alloc::format!("a{i}='x'")).collect::<Vec<_>>().join(" "));
    assert!(read_ado(wide.as_bytes(), &rs, 16, 50).is_none());
}

#[test]
fn iso_dates() {
    assert_eq!(iso_days("1899-12-30T00:00:00"), Some(0.0));
    assert_eq!(iso_days("1900-01-01"), Some(2.0));
    assert_eq!(iso_days("12:00:00"), Some(0.5));
    assert!(iso_days("2026-13-01").is_none());
    assert!(iso_days("gestern").is_none());
}
