// tests of data.rs: recordsets from tables, linking, refresh with conflicts, auto link, undo

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::document::{DocKind, PageRef};
use crate::shape::{ShapeKind, Text};

fn s(x: &str) -> String {
    String::from(x)
}

fn t(x: &str) -> DataValue {
    DataValue::Text(s(x))
}

struct Fx {
    doc: Document,
    shapes: Vec<ShapeRef>,
}

// a drawing with one page and three shapes with the texts Server1, Server2, Drucker
fn fx() -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    let mut tx = doc.transaction("setup");
    let page = tx.add_page("Zeichenblatt-1", "Page-1", false, None).unwrap();
    let mut shapes = Vec::new();
    for name in ["Server1", "Server2", "Drucker"] {
        let sh = tx.add_shape(Owner::Page(page), None, ShapeKind::Shape).unwrap();
        tx.set_text(sh, Some(Text::plain(name))).unwrap();
        shapes.push(sh);
    }
    tx.commit();
    let _: PageRef = page;
    Fx { doc, shapes }
}

fn table() -> (Vec<String>, Vec<Vec<DataValue>>) {
    (
        vec![s("Gerät"), s("IP-Adresse"), s("Ports"), s("Aktiv")],
        vec![
            vec![t("Server1"), t("10.0.0.1"), DataValue::Num(24.0), DataValue::Bool(true)],
            vec![t("Server2"), t("10.0.0.2"), DataValue::Num(48.0), DataValue::Bool(false)],
            vec![t("Switch"), t("10.0.0.3"), DataValue::Num(8.0), DataValue::Bool(true)],
        ],
    )
}

fn add(f: &mut Fx, key: &[&str]) -> u32 {
    let (h, rows) = table();
    let key: Vec<String> = key.iter().map(|k| s(k)).collect();
    let rs = Recordset::from_table("Geräte", &h, rows, &key);
    let (cs, cmd) = excel_connection("C:\\Daten\\geräte.xlsx", &ExcelTable::Sheet(s("Tabelle1")), true);
    let conn = Connection { connection_string: Some(cs), command: Some(cmd.clone()), ..Connection::default() };
    let mut rs = rs;
    rs.command = Some(cmd);
    let mut tx = f.doc.transaction("add");
    let id = add_recordset(&mut tx, Some(conn), rs).unwrap();
    tx.commit();
    id
}

fn value(doc: &Document, shape: ShapeRef, name: &str) -> String {
    field_text(doc, SheetRef::Shape(shape), name).unwrap_or_else(|| s("<none>"))
}

#[test]
fn from_table_guesses_types_names_and_ids() {
    let (h, rows) = table();
    let rs = Recordset::from_table("Geräte", &h, rows, &[]);
    let kinds: Vec<u16> = rs.columns.iter().map(|c| c.data_type).collect();
    assert_eq!(kinds, vec![kind::TEXT, kind::TEXT, kind::NUMBER, kind::BOOL]);
    assert_eq!(rs.columns[1].name, "_VisDM_IPAdresse");
    assert_eq!(rs.columns[1].label, "IP-Adresse");
    assert_eq!(rs.columns[0].name, "_VisDM_Gerät");
    assert!(rs.row_order);
    assert_eq!(rs.rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(rs.next_row, 4);
    // duplicate and empty headers
    let rs = Recordset::from_table("x", &[s("A"), s("A"), s(""), s("1x")], vec![vec![t("1")]], &[]);
    let ids: Vec<&str> = rs.columns.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["A", "A1", "F", "1x"]);
    assert_eq!(rs.columns[3].name, "_VisDM_1x");
    assert_eq!(rs.rows[0].values.len(), 4);
}

#[test]
fn key_values_follow_the_rows() {
    let (h, rows) = table();
    let rs = Recordset::from_table("Geräte", &h, rows, &[s("Gerät")]);
    assert!(!rs.row_order);
    assert_eq!(rs.key.len(), 1);
    assert_eq!(rs.key[0].values.as_slice(), &[(1, s("Server1")), (2, s("Server2")), (3, s("Switch"))]);
    assert_eq!(rs.key_of(&rs.rows[2]), "Switch");
}

#[test]
fn add_hands_out_ids_and_names() {
    let mut f = fx();
    let a = add(&mut f, &[]);
    let b = add(&mut f, &[]);
    assert_eq!((a, b), (1, 2));
    let names: Vec<&str> = f.doc.data.recordsets.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["Geräte", "Geräte (2)"]);
    assert_eq!(f.doc.data.connections.iter().map(|c| c.id).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(f.doc.data.active, Some(2));
    let mut tx = f.doc.transaction("rm");
    remove_recordset(&mut tx, 1).unwrap();
    tx.commit();
    assert_eq!(f.doc.data.connections.len(), 1);
    assert_eq!(f.doc.data.recordsets[0].connection, Some(2));
}

#[test]
fn link_writes_shape_data_and_undo_takes_it_back() {
    let mut f = fx();
    let rs = add(&mut f, &["Gerät"]);
    let shape = f.shapes[0];
    let mut tx = f.doc.transaction("link");
    let written = link(&mut tx, shape, rs, 1).unwrap();
    let change = tx.commit().unwrap();
    assert_eq!(written.len(), 4);
    assert_eq!(f.doc.data_link(shape, rs), Some(1));
    assert_eq!(value(&f.doc, shape, "_VisDM_IPAdresse"), "10.0.0.1");
    assert_eq!(value(&f.doc, shape, "_VisDM_Ports"), "24");
    assert_eq!(value(&f.doc, shape, "_VisDM_Aktiv"), "TRUE");
    let row = f.doc.shape_data(SheetRef::Shape(shape)).into_iter().find(|r| r.name() == Some("_VisDM_Ports")).unwrap();
    assert_eq!(row.num("Type"), Some(2.0));
    assert_eq!(row.text("Label"), Some("Ports"));
    assert_eq!(row.num(DATA_LINKED), Some(1.0));
    // relinking the shape to another row replaces the link
    let mut tx = f.doc.transaction("link2");
    link(&mut tx, shape, rs, 2).unwrap();
    let change2 = tx.commit().unwrap();
    assert_eq!(f.doc.data_links(shape), vec![(rs, 2)]);
    assert_eq!(f.doc.recordset(rs).unwrap().links.len(), 1);
    let mut change2 = change2;
    assert!(change2.undo(&mut f.doc));
    let mut change = change;
    assert!(change.undo(&mut f.doc));
    assert_eq!(f.doc.data_link(shape, rs), None);
    assert_eq!(value(&f.doc, shape, "_VisDM_IPAdresse"), "<none>");
    assert!(change.redo(&mut f.doc));
    assert_eq!(f.doc.data_link(shape, rs), Some(1));
}

#[test]
fn unlink_keeps_values() {
    let mut f = fx();
    let rs = add(&mut f, &[]);
    let shape = f.shapes[1];
    let mut tx = f.doc.transaction("link");
    link(&mut tx, shape, rs, 2).unwrap();
    unlink(&mut tx, shape, Some(rs)).unwrap();
    tx.commit();
    assert_eq!(f.doc.data_link(shape, rs), None);
    assert_eq!(value(&f.doc, shape, "_VisDM_Gerät"), "Server2");
    assert!(!data_linked(&f.doc, SheetRef::Shape(shape), "_VisDM_Gerät"));
}

#[test]
fn auto_link_matches_text_and_data() {
    let mut f = fx();
    let rs = add(&mut f, &["Gerät"]);
    let shapes = f.shapes.clone();
    let mut tx = f.doc.transaction("auto");
    let (made, _) = auto_link(&mut tx, &shapes, rs, &[(0, ShapeField::Text)], false).unwrap();
    tx.commit();
    assert_eq!(made, vec![(shapes[0], 1), (shapes[1], 2)]);
    assert_eq!(f.doc.data_link(shapes[2], rs), None);
    // a second run without replace keeps the links; with a data field it matches too
    let mut tx = f.doc.transaction("auto2");
    let (made, _) = auto_link(&mut tx, &shapes, rs, &[(1, ShapeField::Data(s("_VisDM_IPAdresse")))], false).unwrap();
    assert!(made.is_empty());
    let (made, _) = auto_link(&mut tx, &shapes, rs, &[(1, ShapeField::Data(s("_VisDM_IPAdresse")))], true).unwrap();
    tx.commit();
    assert_eq!(made.len(), 2);
}

#[test]
fn refresh_by_key_updates_reports_and_keeps_ids() {
    let mut f = fx();
    let rs = add(&mut f, &["Gerät"]);
    let (a, b) = (f.shapes[0], f.shapes[1]);
    let mut tx = f.doc.transaction("link");
    link(&mut tx, a, rs, 1).unwrap();
    link(&mut tx, b, rs, 2).unwrap();
    // the user overwrites a linked value on shape a
    let sk = SecKey::one(names::PROPERTY);
    let rk = RowKey::Named(tx.intern("_VisDM_Ports"));
    let v = names::VALUE;
    tx.set_constant(SheetRef::Shape(a), &CellAddr::row(sk, rk, v), Value::Num(99.0), None).unwrap();
    user_edited(&mut tx, SheetRef::Shape(a), "_VisDM_Ports").unwrap();
    tx.commit();
    // new data: Server2 gone, Server1 with new IP and ports, a new row first, a new column
    let h = vec![s("Gerät"), s("IP-Adresse"), s("Ports"), s("Aktiv"), s("Raum")];
    let rows = vec![
        vec![t("Router"), t("10.0.0.9"), DataValue::Num(4.0), DataValue::Bool(true), t("K1")],
        vec![t("Server1"), t("10.0.1.1"), DataValue::Num(32.0), DataValue::Bool(true), t("R2")],
        vec![t("Switch"), t("10.0.0.3"), DataValue::Num(8.0), DataValue::Bool(true), t("R2")],
    ];
    let mut tx = f.doc.transaction("refresh");
    let r = refresh(&mut tx, rs, &h, rows, "2026-11-23T14:05:00").unwrap();
    let change = tx.commit().unwrap();
    assert_eq!((r.added, r.changed, r.removed), (1, 2, 1));
    let set = f.doc.recordset(rs).unwrap();
    // Server1 keeps ID 1, Switch 3, Router is new: 4
    let ids: Vec<(u32, String)> = set.rows.iter().map(|r| (r.id, r.values[0].text())).collect();
    assert_eq!(ids, vec![(4, s("Router")), (1, s("Server1")), (3, s("Switch"))]);
    assert_eq!(set.next_row, 5);
    assert_eq!(set.time_refreshed.as_deref(), Some("2026-11-23T14:05:00"));
    assert_eq!(set.columns.len(), 5);
    // a: new IP, new column, the user's ports stay and are reported
    assert_eq!(value(&f.doc, a, "_VisDM_IPAdresse"), "10.0.1.1");
    assert_eq!(value(&f.doc, a, "_VisDM_Raum"), "R2");
    assert_eq!(value(&f.doc, a, "_VisDM_Ports"), "99");
    assert!(r.conflicts.iter().any(|c| matches!(c, Conflict::UserValue { shape, column, user, .. } if *shape == a && column == "_VisDM_Ports" && user == "99")));
    // b: its row is gone; it stays linked and is a RefreshConflict
    assert!(r.conflicts.iter().any(|c| matches!(c, Conflict::RowGone { row: 2, shape } if *shape == b)));
    assert_eq!(set.conflicts.len(), 1);
    assert_eq!(f.doc.data_link(b, rs), Some(2));
    // resolving: take the data value on a, unlink b
    let mut tx = f.doc.transaction("resolve");
    resolve_take_data(&mut tx, a, rs, "_VisDM_Ports").unwrap();
    resolve_unlink(&mut tx, b, rs).unwrap();
    tx.commit();
    assert_eq!(value(&f.doc, a, "_VisDM_Ports"), "32");
    assert_eq!(f.doc.data_link(b, rs), None);
    assert!(f.doc.recordset(rs).unwrap().conflicts.is_empty());
    let _ = change;
}

#[test]
fn refresh_overwrite_all_and_duplicates() {
    let mut f = fx();
    let rs = add(&mut f, &["Gerät"]);
    let a = f.shapes[0];
    let mut set = f.doc.recordset(rs).unwrap().clone();
    set.overwrite_all = true;
    let mut tx = f.doc.transaction("link");
    set_recordset(&mut tx, set).unwrap();
    link(&mut tx, a, rs, 1).unwrap();
    let rk = RowKey::Named(tx.intern("_VisDM_Ports"));
    tx.set_constant(SheetRef::Shape(a), &CellAddr::row(SecKey::one(names::PROPERTY), rk, names::VALUE), Value::Num(99.0), None).unwrap();
    user_edited(&mut tx, SheetRef::Shape(a), "_VisDM_Ports").unwrap();
    tx.commit();
    let (h, mut rows) = table();
    rows.push(vec![t("Server1"), t("10.9.9.9"), DataValue::Num(1.0), DataValue::Bool(false)]);
    let mut tx = f.doc.transaction("refresh");
    let r = refresh(&mut tx, rs, &h, rows, "now").unwrap();
    tx.commit();
    assert_eq!(value(&f.doc, a, "_VisDM_Ports"), "24");
    assert!(r.conflicts.iter().any(|c| matches!(c, Conflict::DuplicateKey { key, rows: 2 } if key == "Server1")));
    // the duplicate is a row of its own with a new ID; the shape keeps the first
    assert_eq!(f.doc.recordset(rs).unwrap().rows.len(), 4);
    assert_eq!(value(&f.doc, a, "_VisDM_IPAdresse"), "10.0.0.1");
}

#[test]
fn refresh_by_order() {
    let mut f = fx();
    let rs = add(&mut f, &[]);
    let a = f.shapes[0];
    let mut tx = f.doc.transaction("link");
    link(&mut tx, a, rs, 3).unwrap();
    tx.commit();
    let (h, mut rows) = table();
    rows.truncate(2);
    let mut tx = f.doc.transaction("refresh");
    let r = refresh(&mut tx, rs, &h, rows, "now").unwrap();
    tx.commit();
    assert_eq!(r.removed, 1);
    assert!(matches!(r.conflicts.as_slice(), [Conflict::RowGone { row: 3, .. }]));
}

#[test]
fn refresh_without_the_key_column_is_refused() {
    let mut f = fx();
    let rs = add(&mut f, &["Gerät"]);
    let a = f.shapes[0];
    let mut tx = f.doc.transaction("link");
    link(&mut tx, a, rs, 1).unwrap();
    tx.commit();
    let (rows_before, ip_before) = (f.doc.recordset(rs).unwrap().rows.clone(), value(&f.doc, a, "IP_Adresse"));
    // the key column renamed in the source, the rows in another order: matched by position, a got
    // another row's data without a conflict
    let (mut h, mut rows) = table();
    h[0] = s("Name");
    rows.reverse();
    assert_eq!(missing_key(&f.doc, rs, &h), vec![s("Gerät")]);
    let mut tx = f.doc.transaction("refresh");
    assert_eq!(refresh(&mut tx, rs, &h, rows, "now").err(), Some(EditError::Invalid));
    tx.commit();
    assert_eq!(f.doc.recordset(rs).unwrap().rows, rows_before);
    assert_eq!(value(&f.doc, a, "IP_Adresse"), ip_before);
    assert!(missing_key(&f.doc, rs, &table().0).is_empty());
}

#[test]
fn deleted_shapes_lose_their_links_on_refresh() {
    let mut f = fx();
    let rs = add(&mut f, &[]);
    let a = f.shapes[0];
    let mut tx = f.doc.transaction("link");
    link(&mut tx, a, rs, 1).unwrap();
    tx.delete_shape(a).unwrap();
    tx.commit();
    let (h, rows) = table();
    let mut tx = f.doc.transaction("refresh");
    refresh(&mut tx, rs, &h, rows, "now").unwrap();
    tx.commit();
    assert!(f.doc.recordset(rs).unwrap().links.is_empty());
}

#[test]
fn connections_and_sources() {
    let (cs, cmd) = excel_connection("D:/Daten/a.xlsx", &ExcelTable::Sheet(s("Tabelle1")), true);
    assert_eq!(cmd, "select * from `Tabelle1$`");
    let mut d = DataSets::default();
    d.connections.push(Connection { id: 1, connection_string: Some(cs), ..Connection::default() });
    let mut rs = Recordset { connection: Some(1), command: Some(cmd), ..Recordset::default() };
    assert_eq!(source_of(&d, &rs), Source::Excel { path: s("D:/Daten/a.xlsx"), table: ExcelTable::Sheet(s("Tabelle1")), header: true });
    let (cs, cmd) = excel_connection("D:/Daten/a.xlsx", &ExcelTable::Range(s("Tabelle 2"), s("$A$1:$C$9")), false);
    assert_eq!(cmd, "select * from `Tabelle 2$A1:C9`");
    d.connections[0].connection_string = Some(cs);
    rs.command = Some(cmd);
    assert_eq!(source_of(&d, &rs), Source::Excel { path: s("D:/Daten/a.xlsx"), table: ExcelTable::Range(s("Tabelle 2"), s("A1:C9")), header: false });
    let (cs, cmd) = excel_connection("D:/Daten/a.xlsx", &ExcelTable::Name(s("Geräte")), true);
    d.connections[0].connection_string = Some(cs);
    rs.command = Some(cmd);
    assert_eq!(source_of(&d, &rs), Source::Excel { path: s("D:/Daten/a.xlsx"), table: ExcelTable::Name(s("Geräte")), header: true });
    let (cs, cmd) = text_connection("D:/Daten/liste.csv", true);
    d.connections[0].connection_string = Some(cs);
    rs.command = Some(cmd);
    assert_eq!(source_of(&d, &rs), Source::Text { path: s("D:/Daten/liste.csv"), header: true });
    // what Visio writes for SQL Server stays Other
    d.connections[0].connection_string = Some(s("Provider=SQLOLEDB.1;Integrated Security=SSPI;Data Source=srv;Initial Catalog=db"));
    rs.command = Some(s("select * from \"dbo\".\"Geraete\""));
    assert_eq!(source_of(&d, &rs), Source::Other);
    assert_eq!(conn_value("A=1;Extended Properties=\"HDR=NO;IMEX=1\";B=2", "b").as_deref(), Some("2"));
    assert_eq!(command_object("SELECT * FROM [Tab$]").as_deref(), Some("Tab$"));
}

#[test]
fn values_as_text() {
    assert_eq!(DataValue::Date(crate::field::ole_days(2026, 11, 23, 0, 0, 0)).text(), "2026-11-23");
    assert_eq!(DataValue::Date(crate::field::ole_days(2026, 11, 23, 14, 5, 0)).text(), "2026-11-23 14:05:00");
    assert_eq!(DataValue::Num(1.5).text(), "1.5");
    assert_eq!(DataValue::Currency(12.0).text(), "12");
    assert_eq!(guess_kind([DataValue::Num(1.0), DataValue::Currency(2.0), DataValue::Empty].iter()), kind::NUMBER);
    assert_eq!(guess_kind([DataValue::Num(1.0), t("x")].iter()), kind::TEXT);
    assert_eq!(guess_kind([DataValue::Empty].iter()), kind::TEXT);
}
