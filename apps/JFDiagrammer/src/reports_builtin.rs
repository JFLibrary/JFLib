// reports_builtin -- (JD19, DA-09) the shape reports that come with JFDiagrammer, in the
// language of the window (Visio keeps its own as .vrd files beside the program)
//
//   all() -> Vec<(key, ReportDef)>     every predefined report: names, descriptions and column
//                                      headings in the current language
//   by_key(key) -> Option<ReportDef>   one of them (a table shape linked to it: "jfd:<key>")
//
// Which ones, and why (the templates' shapes have this shape data, stencils/net_common/*.inc):
//   inventory          Inventar: every shape counted by its master (any drawing)
//   network_devices    Netzwerkgeräte: shapes with an IP address (network.inc: network and
//                      detailed network, servers, computers, peripherals)
//   network_equipment  Netzwerkausstattung: shapes with a manufacturer (asset.inc), grouped by it
//   computers          Computer: shapes with an operating system (computer.inc: computers, servers)
//   rack               Rackgeräte: shapes with height units (rackdev.inc), their sum
// No flowchart, organisation chart, room or door report: the app's flowchart, block and other
// templates have no shape data (FEATURES DA-09 names them for Visio's templates); they come
// with the templates that bring such data (JD27, JD28, JD31).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use diagram_core::report::{Builtin, Condition, Field, Filter, Func, Property, ReportDef, Sort};
use ui::i18n::t;

fn data(id: u32, name: &str, heading: &str) -> Field {
    Field::new(id, Property::Data(String::from(name)), heading)
}

fn master(id: u32) -> Field {
    Field::new(id, Property::Builtin(Builtin::Master), t("Gerät"))
}

fn exists(name: &str) -> Filter {
    Filter::new(1, Property::Data(String::from(name)), Condition::Exists, "")
}

fn def(name: &str, description: &str) -> ReportDef {
    let mut d = ReportDef::new(name);
    d.description = String::from(description);
    d
}

/// the predefined reports in the current language: (key, definition)
pub(crate) fn all() -> Vec<(&'static str, ReportDef)> {
    let mut inv = def(t("Inventar"), t("Anzahl der Shapes auf allen Seiten, nach Master-Shape gruppiert."));
    let mut f = Field::new(1, Property::Builtin(Builtin::Master), t("Master-Shape"));
    f.summary = Func::Count.bit();
    inv.fields.push(f);
    if let Some(g) = inv.grouping.as_mut() {
        g.field = 1;
        g.show_items = false;
        g.grand_summary = true;
    }
    inv.precision = 0;

    let mut net = def(t("Netzwerkgeräte"), t("Geräte mit IP-Adresse: Netzwerkname, Adressen und Hersteller, nach Netzwerkname sortiert."));
    net.fields = vec![master(1), data(2, "NetworkName", t("Netzwerkname")), data(3, "IPAddress", t("IP-Adresse")), data(4, "SubnetMask", t("Subnetzmaske")), data(5, "MACAddress", t("MAC-Adresse")), data(6, "Manufacturer", t("Hersteller"))];
    net.filters.push(exists("IPAddress"));
    net.advanced_filter = true;
    net.sorts.push(Sort::new(1, 2, true));

    let mut eq = def(t("Netzwerkausstattung"), t("Hersteller, Produkt-, Serien- und Inventarnummern und Standorte der Geräte, nach Hersteller gruppiert."));
    eq.fields = vec![data(1, "Manufacturer", t("Hersteller")), master(2), data(3, "ProductNumber", t("Produktnummer")), data(4, "SerialNumber", t("Seriennummer")), data(5, "AssetNumber", t("Inventarnummer")), data(6, "Building", t("Gebäude")), data(7, "Room", t("Raum"))];
    eq.fields[1].summary = Func::Count.bit();
    eq.filters.push(exists("Manufacturer"));
    eq.advanced_filter = true;
    if let Some(g) = eq.grouping.as_mut() {
        g.field = 1;
        g.grand_summary = true;
    }
    eq.sorts.push(Sort::new(1, 2, true));
    eq.precision = 0;

    let mut pc = def(t("Computer"), t("Computer und Server mit Betriebssystem, Prozessor, Arbeitsspeicher und Festplatte."));
    pc.fields = vec![data(1, "NetworkName", t("Netzwerkname")), master(2), data(3, "OperatingSystem", t("Betriebssystem")), data(4, "CPU", t("Prozessor")), data(5, "Memory", t("Arbeitsspeicher")), data(6, "HardDriveSize", t("Festplatte")), data(7, "IPAddress", t("IP-Adresse"))];
    pc.filters.push(exists("OperatingSystem"));
    pc.advanced_filter = true;
    pc.sorts.push(Sort::new(1, 1, true));

    let mut rack = def(t("Rackgeräte"), t("Geräte im Rack mit ihren Höheneinheiten; die Summe zeigt den belegten Platz."));
    rack.fields = vec![master(1), data(2, "Manufacturer", t("Hersteller")), data(3, "ProductNumber", t("Produktnummer")), data(4, "RackUnits", t("Höheneinheiten"))];
    rack.fields[3].data_type = 2;
    rack.fields[3].summary = Func::Total.bit();
    rack.filters.push(exists("RackUnits"));
    rack.advanced_filter = true;
    rack.precision = 0;

    vec![("inventory", inv), ("network_devices", net), ("network_equipment", eq), ("computers", pc), ("rack", rack)]
}

/// a predefined report by its key
pub(crate) fn by_key(key: &str) -> Option<ReportDef> {
    all().into_iter().find(|(k, _)| *k == key).map(|(_, d)| d)
}
