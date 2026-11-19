// stencil_network -- (JD13) the network and rack stencils of apps/JFDiagrammer/stencils
// (diagram_io::stencil, own sources and the shared network symbols, SH-G09)
//
//   stencils/network/<set>/<measure>   every set builds in both measures, write -> read ->
//                       write is the same, a second build gives the same bytes
//   stencils/network/data      a dropped device has the shape data of its includes (asset,
//                       network, computer) with the labels in the language it was dropped in
//   stencils/network/rack      the rack: height = base + top + units x 44.45 mm, its connection
//                       points on every unit boundary; a user's height goes to Prop.RackUnits
//                       (SETATREF) and snaps; devices alike; the master pages are 1:20
//
//   stencils/network/dock      a server dropped over a rack docks with its outward point on the
//                       nearest unit boundary of the rail (diagram_core::dock) and joins the
//                       rack (structure::settle); moving the rack moves it; beside the rack and
//                       out of reach nothing docks
//
//   diagramiotest --write-network-previews DIR   netprobe.png: the network symbols and a rack
//                       with devices, for a look at them

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::{Engine, Input};
use diagram_core::instance::{self, MasterTexts};
use diagram_core::render::Options;
use diagram_core::{dock, ops, structure, CellAddr, Owner, DocKind, Document, PageRef, RowKey, SecKey, ShapeRef, SheetRef, Text, Transaction, Unit, Value};
use diagram_io::vsdx::{self, Measure};
use diagram_view::export::raster;
use diagram_view::PageView;

use crate::stencils::{build_set, read_back};
use crate::{out, H};

const MM: f64 = 1.0 / 25.4;

macro_rules! src {
    ($dir:literal, $($name:literal),* $(,)?) => {
        &[$(($name, include_str!(concat!("../../JFDiagrammer/stencils/", $dir, "/", $name))),)*]
    };
}

// the files every network set may include, by the name the masters give
const COMMON: &[(&str, &str)] = &[
    ("../net_common/symbol.inc", include_str!("../../JFDiagrammer/stencils/net_common/symbol.inc")),
    ("../net_common/asset.inc", include_str!("../../JFDiagrammer/stencils/net_common/asset.inc")),
    ("../net_common/network.inc", include_str!("../../JFDiagrammer/stencils/net_common/network.inc")),
    ("../net_common/computer.inc", include_str!("../../JFDiagrammer/stencils/net_common/computer.inc")),
    ("../net_common/location.inc", include_str!("../../JFDiagrammer/stencils/net_common/location.inc")),
    ("../net_common/rackdev.inc", include_str!("../../JFDiagrammer/stencils/net_common/rackdev.inc")),
];

const PERIPHERALS: &[(&str, &str)] = src!("net_peripherals", "stencil.txt", "router.master", "switch.master", "router.svg", "switch_l2.svg");
const SERVERS: &[(&str, &str)] = src!("net_servers", "stencil.txt", "web_server.master", "web_server.svg");
const LOCATIONS: &[(&str, &str)] = src!("net_locations", "stencil.txt", "building.master", "building.svg");
const DETAILED: &[(&str, &str)] = src!("net_detailed", "stencil.txt", "satellite_dish.master", "satellite_dish.svg");
const RACK_EQUIPMENT: &[(&str, &str)] = src!("rack_equipment", "stencil.txt", "rack.master", "server.master", "switch.master", "server_2u.svg", "switch_1u.svg");

/// the sets: name, files, the number of masters
pub(crate) const SETS: &[(&str, &[(&str, &str)], usize)] =
    &[("peripherals", PERIPHERALS, 2), ("servers", SERVERS, 1), ("locations", LOCATIONS, 1), ("detailed", DETAILED, 1), ("rack_equipment", RACK_EQUIPMENT, 3)];

// a set's files and the common ones
fn files<'a>(set: &'a [(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    set.iter().chain(COMMON.iter()).copied().collect()
}

fn built(set: &[(&str, &str)], m: Measure) -> Result<Document, String> {
    let f = files(set);
    build_set(&f, m).map(|(b, _)| b.doc).map_err(|e| format!("{e}"))
}

/// the group "stencils/network"
pub fn run(h: &mut H) {
    let mut rack: Option<Document> = None;
    let mut peri: Option<Document> = None;
    for &(name, set, count) in SETS {
        for m in [Measure::Metric, Measure::Us] {
            let tag = format!("stencils/network/{name}/{}", if m == Measure::Us { "us" } else { "metric" });
            let f = files(set);
            match build_set(&f, m) {
                Err(e) => h.check(&tag, false, format!("{e}")),
                Ok((b, bytes)) => {
                    let n = b.doc.masters().count();
                    h.check(&tag, n == count && b.category == "network", format!("{n} masters (want {count}), category {}, {} bytes", b.category, bytes.len()));
                    let again = build_set(&f, m).map(|(_, x)| x).unwrap_or_default();
                    h.check(&format!("{tag}/deterministic"), again == bytes, format!("same bytes: {}", again == bytes));
                    let back = read_back(h, &format!("{tag}/roundtrip"), &bytes);
                    if m == Measure::Metric {
                        match name {
                            "rack_equipment" => rack = back,
                            "peripherals" => peri = back,
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    if let Some(doc) = &peri {
        data(h, doc);
    }
    if let Some(doc) = &rack {
        rack_checks(h, doc);
        dock_checks(h, doc);
    }
}

fn new_drawing(scale: f64) -> Option<(Document, PageRef)> {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next()?;
    let mut tx = doc.transaction("page");
    set_single(&mut tx, SheetRef::Page(page), "PageScale", MM, Some(Unit::Millimeters));
    set_single(&mut tx, SheetRef::Page(page), "DrawingScale", scale * MM, Some(Unit::Millimeters));
    let _ = tx.commit();
    Some((doc, page))
}

fn set_single(tx: &mut Transaction, s: SheetRef, n: &str, v: f64, u: Option<Unit>) {
    let a = CellAddr::single(tx.intern(n));
    let _ = tx.set_constant(s, &a, Value::Num(v), u);
}

// one instance of master `nu` of `src` with its pin at (x, y) mm in language `lang`
fn put(tx: &mut Transaction, src: &Document, page: PageRef, nu: &str, at: (f64, f64), lang: usize) -> Option<ShapeRef> {
    let m = src.masters().find(|&m| MasterTexts::of(src, m).name_u == nu)?;
    let local = instance::import_master(tx, src, m).ok()?;
    instance::place(tx, page, local, at.0 * MM, at.1 * MM, lang).ok()
}

fn recalc_all(doc: &mut Document, page: PageRef) {
    let mut tx = doc.transaction("recalc");
    let sheets: Vec<SheetRef> = tx.doc().all_shapes(diagram_core::Owner::Page(page)).map(SheetRef::Shape).collect();
    let _ = Engine::new().recalc(&mut tx, &sheets);
    let _ = tx.commit();
}

fn num(doc: &Document, s: ShapeRef, sec: Option<(&str, RowKey)>, cell: &str) -> Option<f64> {
    let c = doc.sym(cell)?;
    let addr = match sec {
        None => CellAddr::single(c),
        Some((name, row)) => CellAddr::row(SecKey::one(doc.sym(name)?), row, c),
    };
    let r = doc.resolve(SheetRef::Shape(s), &addr);
    match r.cached.or(r.cell).map(|c| c.value) {
        Some(Value::Num(x)) => Some(x),
        _ => None,
    }
}

fn text(doc: &Document, s: ShapeRef, sec: &str, row: &str, cell: &str) -> String {
    let (Some(sk), Some(rk), Some(c)) = (doc.sym(sec), doc.sym(row), doc.sym(cell)) else { return String::new() };
    let r = doc.resolve(SheetRef::Shape(s), &CellAddr::row(SecKey::one(sk), RowKey::Named(rk), c));
    match r.cached.or(r.cell).map(|c| c.value) {
        Some(Value::Text(t)) => String::from(doc.str(t)),
        _ => String::new(),
    }
}

fn named(doc: &Document, n: &str) -> Option<RowKey> {
    doc.sym(n).map(RowKey::Named)
}

// shape data of a dropped router: all rows of asset.inc + network.inc, labels in French
fn data(h: &mut H, src: &Document) {
    let Some((mut doc, page)) = new_drawing(1.0) else { return };
    let s = {
        let mut tx = doc.transaction("drop");
        let s = put(&mut tx, src, page, "Router", (50.0, 50.0), 4);
        let _ = tx.commit();
        s
    };
    let Some(s) = s else {
        h.check("stencils/network/data", false, String::from("Router not placed"));
        return;
    };
    let rows = ["AssetNumber", "SerialNumber", "Location", "Building", "Room", "Manufacturer", "ProductNumber", "PartNumber", "ProductDescription", "NetworkName", "IPAddress", "SubnetMask", "MACAddress", "AdminInterface", "NumberofPorts", "CommunityString", "NetworkDescription"];
    let missing: Vec<&str> = rows.iter().copied().filter(|r| text(&doc, s, "Property", r, "Label").is_empty()).collect();
    h.check("stencils/network/data/rows", missing.is_empty(), format!("missing labels: {missing:?}"));
    let ip = text(&doc, s, "Property", "IPAddress", "Label");
    let class = text(&doc, s, "Property", "ShapeClass", "Value");
    let sub = text(&doc, s, "Property", "SubShapeType", "Value");
    h.check("stencils/network/data/french", ip == "Adresse IP" && class == "Équipements" && sub == "Routeur", format!("IPAddress label {ip:?}, ShapeClass {class:?}, SubShapeType {sub:?}"));
}

// the rack and its devices: heights from units, SETATREF, connection points, scale
fn rack_checks(h: &mut H, src: &Document) {
    let ms_scale: Vec<(String, f64)> = src
        .masters()
        .map(|m| {
            let nu = MasterTexts::of(src, m).name_u;
            let d = src.sym("DrawingScale").and_then(|c| src.master(m)?.sheet.cell(c)).map(|c| c.value);
            (nu, if let Some(Value::Num(x)) = d { x / MM } else { 0.0 })
        })
        .collect();
    h.check("stencils/network/rack/scale", ms_scale.iter().all(|(_, s)| (*s - 20.0).abs() < 1e-9), format!("master page DrawingScale (mm): {ms_scale:?}"));
    let Some((mut doc, page)) = new_drawing(20.0) else { return };
    let (rack, server, switch) = {
        let mut tx = doc.transaction("drop");
        let r = put(&mut tx, src, page, "Rack", (400.0, 1200.0), 0);
        let s = put(&mut tx, src, page, "Server", (1200.0, 600.0), 0);
        let w = put(&mut tx, src, page, "Switch", (1200.0, 300.0), 0);
        let _ = tx.commit();
        (r, s, w)
    };
    let (Some(rack), Some(server), Some(switch)) = (rack, server, switch) else {
        h.check("stencils/network/rack", false, String::from("rack, server or switch not placed"));
        return;
    };
    recalc_all(&mut doc, page);
    let hmm = |doc: &Document, s: ShapeRef| num(doc, s, None, "Height").map_or(-1.0, |v| v / MM);
    let u = 44.45;
    h.check("stencils/network/rack/height", (hmm(&doc, rack) - (100.0 + 42.0 * u)).abs() < 1e-6 && (hmm(&doc, server) - 2.0 * u).abs() < 1e-6 && (hmm(&doc, switch) - u).abs() < 1e-6,
        format!("rack {:.3} mm (want {:.3}), server {:.3}, switch {:.3}", hmm(&doc, rack), 100.0 + 42.0 * u, hmm(&doc, server), hmm(&doc, switch)));
    // connection points: k (row k) at the mount line, k units up; past the height on the top
    let conn = |doc: &Document, s: ShapeRef, k: u32, c: &str| num(doc, s, Some(("Connection", RowKey::Index(k))), c).map_or(-1.0, |v| v / MM);
    h.check("stencils/network/rack/points", (conn(&doc, rack, 0, "Y") - 60.0).abs() < 1e-6 && (conn(&doc, rack, 10, "Y") - (60.0 + 10.0 * u)).abs() < 1e-6 && (conn(&doc, rack, 50, "Y") - (60.0 + 42.0 * u)).abs() < 1e-6 && (conn(&doc, rack, 3, "X") - 38.7).abs() < 1e-6,
        format!("row 0 y {:.2}, row 10 y {:.2}, row 50 y {:.2}, x {:.2}", conn(&doc, rack, 0, "Y"), conn(&doc, rack, 10, "Y"), conn(&doc, rack, 50, "Y"), conn(&doc, rack, 3, "X")));
    let dev_type = num(&doc, server, Some(("Connection", RowKey::Index(0))), "Type");
    let rack_type = num(&doc, rack, Some(("Connection", RowKey::Index(0))), "Type");
    h.check("stencils/network/rack/point-types", dev_type == Some(1.0) && rack_type.unwrap_or(0.0) == 0.0, format!("device {dev_type:?} (outward 1), rack {rack_type:?} (inward 0)"));
    // a user's height: the rack to 1000 mm -> 20 units -> 989 mm; the server to 100 mm -> 2 U;
    // the switch to 160 mm -> 4 U
    let mut eng = Engine::new();
    let mut user_height = |doc: &mut Document, s: ShapeRef, mm: f64| {
        let mut tx = doc.transaction("height");
        let a = CellAddr::single(tx.intern("Height"));
        let r = eng.user_set(&mut tx, SheetRef::Shape(s), &a, Input::Length(mm, Unit::Millimeters));
        let _ = tx.commit();
        r.is_ok()
    };
    let ok = user_height(&mut doc, rack, 1000.0) & user_height(&mut doc, server, 100.0) & user_height(&mut doc, switch, 160.0);
    let units = |doc: &Document, s: ShapeRef| num(doc, s, named(doc, "RackUnits").map(|r| ("Property", r)), "Value").unwrap_or(-1.0);
    h.check("stencils/network/rack/setatref",
        ok && units(&doc, rack) == 20.0 && (hmm(&doc, rack) - (100.0 + 20.0 * u)).abs() < 1e-6 && units(&doc, server) == 2.0 && units(&doc, switch) == 4.0 && (hmm(&doc, switch) - 4.0 * u).abs() < 1e-6,
        format!("rack {} U {:.2} mm, server {} U, switch {} U {:.2} mm", units(&doc, rack), hmm(&doc, rack), units(&doc, server), units(&doc, switch), hmm(&doc, switch)));
    let formula = |doc: &Document, s: ShapeRef| {
        let c = doc.sym("Height");
        c.map(|c| doc.resolve(SheetRef::Shape(s), &CellAddr::single(c)).formula()).map(|f| matches!(f, diagram_core::Formula::Expr(_) | diagram_core::Formula::Inherited))
    };
    h.check("stencils/network/rack/formula-kept", formula(&doc, rack) == Some(true) && formula(&doc, switch) == Some(true), format!("rack {:?}, switch {:?}", formula(&doc, rack), formula(&doc, switch)));
}

/// diagramiotest --write-network-previews DIR
pub fn previews(dir: &str) -> i32 {
    let mut sets: Vec<Document> = Vec::new();
    for &(name, set, _) in SETS {
        match built(set, Measure::Metric) {
            Ok(d) => sets.push(d),
            Err(e) => {
                out(&format!("error: stencil set {name} does not build: {e}\n"));
                return 1;
            }
        }
    }
    // left: the symbols at 1:1 with their names, connected; right: a rack at 1:20 with devices
    let Some((mut net, np)) = new_drawing(1.0) else { return 1 };
    {
        let mut tx = net.transaction("previews");
        set_single(&mut tx, SheetRef::Page(np), "PageWidth", 170.0 * MM, Some(Unit::Millimeters));
        set_single(&mut tx, SheetRef::Page(np), "PageHeight", 130.0 * MM, Some(Unit::Millimeters));
        let place = [
            (2, "Building", (25.0, 50.0), "Gebäude"),
            (0, "Router", (65.0, 50.0), "Router"),
            (3, "Satellite dish", (25.0, 100.0), "Satellitenschüssel"),
            (0, "Switch", (105.0, 50.0), "Switch"),
            (1, "Web server", (145.0, 50.0), "Webserver"),
        ];
        let mut shapes: Vec<ShapeRef> = Vec::new();
        for (k, nu, at, label) in place {
            if let Some(s) = put(&mut tx, &sets[k], np, nu, at, 0) {
                let _ = tx.set_text(s, Some(Text::plain(label)));
                shapes.push(s);
            }
        }
        let mut eng = Engine::new();
        let owner = Owner::Page(np);
        if shapes.len() == 5 {
            for (a, b) in [(0, 1), (2, 1), (1, 3), (3, 4)] {
                let (pa, pb) = (ops::frame(tx.doc(), shapes[a]).centre(), ops::frame(tx.doc(), shapes[b]).centre());
                if let Ok(c) = diagram_core::glue::new_connector(&mut tx, &mut eng, owner, (pa, diagram_core::glue::Target::Shape(shapes[a])), (pb, diagram_core::glue::Target::Shape(shapes[b]))) {
                    // a network plan's lines have no arrowheads
                    set_single(&mut tx, SheetRef::Shape(c), "EndArrow", 0.0, None);
                    let _ = diagram_core::glue::update(&mut tx, &mut eng, &[c]);
                }
            }
        }
        let _ = tx.commit();
    }
    let Some((mut rk, rp)) = new_drawing(20.0) else { return 1 };
    {
        let mut tx = rk.transaction("previews");
        set_single(&mut tx, SheetRef::Page(rp), "PageWidth", 1700.0 * MM, Some(Unit::Millimeters));
        set_single(&mut tx, SheetRef::Page(rp), "PageHeight", 2600.0 * MM, Some(Unit::Millimeters));
        let rack = &sets[4];
        let r = put(&mut tx, rack, rp, "Rack", (420.0, 1300.0), 0);
        if let Some(r) = r {
            let sk = SecKey::one(tx.intern("Property"));
            let row = RowKey::Named(tx.intern("RackUnits"));
            let a = CellAddr::row(sk, row, tx.intern("Value"));
            let _ = tx.set_constant(SheetRef::Shape(r), &a, Value::Num(24.0), None);
            let _ = tx.set_text(r, Some(Text::plain("Rack A1 (24 HE)")));
        }
        let _ = tx.commit();
    }
    recalc_all(&mut rk, rp);
    {
        // devices on the units: bottom of the rack opening at y = 1300 - h/2 + 60
        let mut tx = rk.transaction("devices");
        let rack = &sets[4];
        let rack_h = 100.0 + 24.0 * 44.45;
        let base = 1300.0 - rack_h * 0.5 + 60.0;
        let x = 420.0;
        let at = |unit: f64, units: f64| (x, base + unit * 44.45 + units * 44.45 * 0.5);
        let devs = [("Switch", 22.0, 1.0, "Core-Switch"), ("Switch", 21.0, 1.0, "Access-Switch"), ("Server", 16.0, 2.0, "Webserver 1"), ("Server", 14.0, 2.0, "Webserver 2"), ("Server", 10.0, 2.0, "Datenbank")];
        for (nu, unit, units, label) in devs {
            if let Some(s) = put(&mut tx, rack, rp, nu, at(unit, units), 0) {
                let _ = tx.set_text(s, Some(Text::plain(label)));
            }
        }
        let _ = tx.commit();
    }
    recalc_all(&mut net, np);
    recalc_all(&mut rk, rp);
    let mut fonts = crate::render::fonts();
    let ro = raster::RasterOptions { dpi: 200.0, ..Default::default() };
    let a = {
        let mut v = PageView::new(&mut net, np, &mut fonts, &Options::default());
        raster::render(&mut v, &ro)
    };
    let b = {
        let mut v = PageView::new(&mut rk, rp, &mut fonts, &Options::default());
        raster::render(&mut v, &ro)
    };
    let (Some(a), Some(b)) = (a, b) else {
        out("error: cannot render\n");
        return 1;
    };
    // side by side on white
    let (w, hh) = (a.w + b.w, a.h.max(b.h));
    let mut sf = gfx::Surface::new(w, hh, a.fmt);
    for p in sf.px.iter_mut() {
        *p = 255;
    }
    for (img, x0) in [(&a, 0), (&b, a.w)] {
        for y in 0..img.h as usize {
            let src = &img.px[y * img.stride as usize..y * img.stride as usize + img.w as usize * 4];
            let d0 = y * sf.stride as usize + x0 as usize * 4;
            sf.px[d0..d0 + img.w as usize * 4].copy_from_slice(src);
        }
    }
    let path = format!("{dir}/netprobe.png");
    let ok = plat::write_file(&path, &crate::render::png(&sf));
    out(&format!("{} {path} ({} x {})\n", if ok { "wrote" } else { "error: cannot write" }, sf.w, sf.h));
    // the rack alone, larger
    let ro2 = raster::RasterOptions { dpi: 600.0, ..Default::default() };
    let c = {
        let mut v = PageView::new(&mut rk, rp, &mut fonts, &Options::default());
        raster::render(&mut v, &ro2)
    };
    if let Some(c) = c {
        let p2 = format!("{dir}/netprobe_rack.png");
        let ok2 = plat::write_file(&p2, &crate::render::png(&c));
        out(&format!("{} {p2} ({} x {})\n", if ok2 { "wrote" } else { "error: cannot write" }, c.w, c.h));
    }
    if ok { 0 } else { 1 }
}

// a server dropped over a rack docks on a unit and joins it; the rack carries it
fn dock_checks(h: &mut H, src: &Document) {
    let Some((mut doc, page)) = new_drawing(20.0) else { return };
    let owner = Owner::Page(page);
    let (rack, server, far) = {
        let mut tx = doc.transaction("drop");
        // rack 42 U with its pin at (400, 1200): bottom at 1200 - 1966.9/2 = 216.55, base 60
        let r = put(&mut tx, src, page, "Rack", (400.0, 1200.0), 0);
        // the server's middle over the rack, its bottom-left near unit 10 (and a bit off the rail)
        let s = put(&mut tx, src, page, "Server", (410.0, 216.55 + 60.0 + 10.3 * 44.45 + 44.45), 0);
        // another one far beside the rack
        let f = put(&mut tx, src, page, "Server", (1600.0, 900.0), 0);
        let _ = tx.commit();
        (r, s, f)
    };
    let (Some(rack), Some(server), Some(far)) = (rack, server, far) else {
        h.check("stencils/network/dock", false, String::from("not placed"));
        return;
    };
    recalc_all(&mut doc, page);
    let mut eng = Engine::new();
    let corner = |doc: &Document, s: ShapeRef| {
        let f = ops::frame(doc, s);
        let p = f.at([0.0, 0.0]);
        [p[0] / MM, p[1] / MM]
    };
    let reach = 5.0 * 20.0 * MM;
    let snapped = {
        let mut tx = doc.transaction("dock");
        let r = dock::dock(&mut tx, &mut eng, owner, &[server], reach);
        let _ = structure::settle(&mut tx, &mut eng, owner, &[server], true);
        let _ = tx.commit();
        r
    };
    let c = corner(&doc, server);
    let want = [400.0 - 280.0 + 38.7, 216.55 + 60.0 + 10.0 * 44.45];
    let ok = matches!(snapped, Ok(Some(s)) if s.target == rack && s.target_point == 10);
    h.check("stencils/network/dock/unit", ok && (c[0] - want[0]).abs() < 1e-6 && (c[1] - want[1]).abs() < 1e-6, format!("{snapped:?}: corner {c:?} (want {want:?})"));
    let members = structure::members(&doc, rack);
    h.check("stencils/network/dock/member", members.contains(&server) && !members.contains(&far), format!("members {members:?}"));
    let none = {
        let tx = doc.transaction("far");
        dock::snap(tx.doc(), owner, &[far], reach)
    };
    h.check("stencils/network/dock/out-of-reach", none.is_none(), format!("{none:?}"));
    // the rack moves 100 mm to the right: the server comes along
    {
        let mut tx = doc.transaction("move");
        let along = structure::carried(tx.doc(), &[rack]);
        let _ = ops::move_shapes(&mut tx, &mut eng, &along, [100.0 * MM, 0.0]);
        let _ = tx.commit();
    }
    let c2 = corner(&doc, server);
    h.check("stencils/network/dock/carried", (c2[0] - want[0] - 100.0).abs() < 1e-6 && (c2[1] - want[1]).abs() < 1e-6, format!("corner after the move {c2:?}"));
}
