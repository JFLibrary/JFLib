// data_fixture -- (JD18) the fixture of JFDiagrammer's scripts data_link.uis and
// data_refresh_conflict.uis
//
//   write(dir)        --write-link-fixture DIR: jd18-link.vsdx and jd18-link.csv
//
// jd18-link.vsdx: one page with six boxes named by their text (Server A, Server B, Router,
// Drucker, Firewall, Switch), nothing linked yet, and the recordset "Geräte" (Name, IP-Adresse,
// Standort; key Name) as it was read once: Server A, Server B, Router, Drucker.  Its connection
// reads the text file apps/JFDiagrammer/tests/jd18-link.csv (relative to the repository, where
// the scripts run), so the app may not read it before the user chose it (fs.user).
// jd18-link.csv: the source as it is now -- Server A has another address, Server B another
// place, Drucker is gone, Switch is new.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::data::{self, Connection, DataValue, Recordset};
use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::{DocKind, Owner, Text};
use diagram_io::vsdx::{self, Measure, WriteOptions};

/// the text file the recordset reads (from the repository's folder)
pub(crate) const CSV: &str = "apps/JFDiagrammer/tests/jd18-link.csv";

const BOXES: [&str; 6] = ["Server A", "Server B", "Router", "Drucker", "Firewall", "Switch"];

// the rows as they were read last
const CACHED: [[&str; 3]; 4] = [["Server A", "10.0.0.1", "Raum 1"], ["Server B", "10.0.0.2", "Raum 2"], ["Router", "10.0.0.254", "Raum 1"], ["Drucker", "10.0.0.50", "Flur"]];

// the source now
const NOW: &str = "Name,IP-Adresse,Standort\r\nServer A,10.0.0.11,Raum 1\r\nServer B,10.0.0.2,Raum 3\r\nRouter,10.0.0.254,Raum 1\r\nSwitch,10.0.0.253,Raum 2\r\n";

fn drawing() -> Result<Vec<u8>, String> {
    let e = |e: diagram_core::EditError| format!("{e:?}");
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().ok_or("no page")?;
    let mut eng = Engine::new();
    let mut tx = doc.transaction("Geräte");
    // two rows of three boxes, 2 x 1 in, on the A4 landscape page (11.69 x 8.27 in)
    for (k, name) in BOXES.iter().enumerate() {
        let (x, y) = (1.5 + 3.5 * (k % 3) as f64, if k < 3 { 5.5 } else { 2.5 });
        let s = ops::new_shape(&mut tx, &mut eng, Owner::Page(page), Basic::Rectangle, [x, y], [x + 2.0, y + 1.0]).map_err(e)?;
        ops::set_text(&mut tx, &mut eng, s, Text::plain(name)).map_err(e)?;
    }
    let headers: Vec<String> = ["Name", "IP-Adresse", "Standort"].iter().map(|h| String::from(*h)).collect();
    let rows: Vec<Vec<DataValue>> = CACHED.iter().map(|r| r.iter().map(|v| DataValue::Text(String::from(*v))).collect()).collect();
    let mut rs = Recordset::from_table("Geräte", &headers, rows, &[String::from("Name")]);
    let (cs, cmd) = data::text_connection(CSV, true);
    rs.command = Some(cmd.clone());
    rs.time_refreshed = Some(String::from("2026-11-23T10:00:00"));
    let conn = Connection { connection_string: Some(cs), command: Some(cmd), ..Connection::default() };
    data::add_recordset(&mut tx, Some(conn), rs).map_err(e)?;
    let _ = tx.commit();
    vsdx::write(&doc, &WriteOptions::DEFAULT).map_err(|e| format!("{e}"))
}

/// --write-link-fixture DIR
pub(crate) fn write(dir: &str) -> i32 {
    let bytes = match drawing() {
        Ok(b) => b,
        Err(e) => {
            crate::out(&format!("jd18-link: {e}\n"));
            return 1;
        }
    };
    for (name, data) in [("jd18-link.vsdx", bytes.as_slice()), ("jd18-link.csv", NOW.as_bytes())] {
        let path = format!("{dir}/{name}");
        if !plat::write_file(&path, data) {
            crate::out(&format!("cannot write {path}\n"));
            return 1;
        }
        crate::out(&format!("wrote {path}\n"));
    }
    0
}
