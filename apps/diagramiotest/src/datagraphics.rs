// datagraphics -- (JD19) the group "datagraphics": data graphics through diagram_core::datagraphic
// and VSDX files
//
//   run(h)            JFDiagrammer's callout masters (stencils/datagraphics) built in both
//                     measures: hidden, Visio's cells, their formulas computed; a drawing with
//                     linked data, a data graphic of all four kinds applied and a legend written
//                     and read back (the model, the bytes, the values the callouts show, a
//                     refresh of the data reaching the callouts, removal); a data graphic built
//                     the way Visio stores one (a copy of the callout master's shape in the item,
//                     USE() by UniqueID, its own compiled icon formula) read, kept and applied
//                     to another shape
//   write(dir)        --write-dg DIR: jd19-datengrafik.vsdx, the drawing for the Visio counter-check
//                     (five boxes linked to DIR\jd19-daten.xlsx, sheet "Geraete"; page 1 a data
//                     graphic with a data bar, an icon set, a text and a colour by value with a
//                     vertical legend; page 2 two boxes with colours by ranges and arrows by
//                     numbers), and jd19-dg.vsdx, the fixture of JFDiagrammer's scripts
//                     tests/data_graphics*.uis (the same boxes linked, no graphic yet)

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use diagram_core::data::{self, Connection, DataValue, ExcelTable, Recordset};
use diagram_core::datagraphic::{self as dg, legend, DataGraphic, Item, Kind, Op, Opt, Pos, Rule, Shade};
use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::{CellAddr, DocKind, Document, MasterRef, Owner, PageRef, RowKey, SecKey, ShapeRef, SheetRef, Text, Value};
use diagram_io::stencil;
use diagram_io::vsdx::{self, Limits, Measure, WriteOptions};

use crate::H;

fn s(x: &str) -> String {
    String::from(x)
}

/// the callout masters as JFDiagrammer builds them
fn callouts(m: Measure) -> Result<Document, String> {
    stencil::build(&crate::templates::files_of("stencils", "datagraphics"), m).map(|b| b.doc).map_err(|e| format!("stencils/datagraphics/{e}"))
}

// the rows of the counter-check's workbook
const ROWS: [(&str, f64, &str, f64, &str); 5] =
    [("Server A", 35.0, "OK", 1999.5, "Raum 1"), ("Server B", 72.0, "Warnung", 2500.0, "Raum 2"), ("Router", 91.0, "Fehler", 899.0, "Raum 1"), ("Switch", 18.0, "OK", 350.0, "Raum 2"), ("Firewall", 55.0, "OK", 1200.0, "Raum 3")];

fn table() -> (Vec<String>, Vec<Vec<DataValue>>) {
    let headers = vec![s("Name"), s("Auslastung"), s("Status"), s("Kosten"), s("Standort")];
    let rows = ROWS.iter().map(|r| vec![DataValue::Text(s(r.0)), DataValue::Num(r.1), DataValue::Text(s(r.2)), DataValue::Currency(r.3), DataValue::Text(s(r.4))]).collect();
    (headers, rows)
}

/// the first data graphic: a bar, traffic lights, the cost as text, the site as colour
pub(crate) fn graphic_one() -> DataGraphic {
    let mut g = DataGraphic::new("Geräte");
    let mut bar = Item::new(Kind::DataBar, "{Auslastung}", "DG Progress bar");
    bar.set_opt("msvCalloutPropMin", Opt::Num(0.0));
    bar.set_opt("msvCalloutPropMax", Opt::Num(100.0));
    g.push(bar);
    let mut icons = Item::new(Kind::IconSet, "{Status}", "DG Traffic lights");
    icons.rules = vec![Rule::new(Op::Equal, "OK", ""), Rule::new(Op::Equal, "Warnung", ""), Rule::new(Op::Equal, "Fehler", "")];
    icons.pos = Some(Pos { h: dg::H_RIGHT_EDGE, v: dg::V_TOP_EDGE });
    g.push(icons);
    let mut text = Item::new(Kind::Text, "{Kosten}", "DG Text");
    text.set_opt("msvCalloutPropValueFormat", Opt::Text(s("#,##0.00 €")));
    text.pos = Some(Pos { h: dg::H_CENTER, v: dg::V_BELOW });
    g.push(text);
    let mut cbv = Item::new(Kind::ColorByValue, "{Standort}", "");
    cbv.shades = vec![
        Shade { value: s("Raum 1"), to: s(""), fill: 0xC6E0B4, text: None },
        Shade { value: s("Raum 2"), to: s(""), fill: 0xBDD7EE, text: None },
        Shade { value: s("Raum 3"), to: s(""), fill: 0xF8CBAD, text: Some(0x7F3F00) },
    ];
    g.push(cbv);
    g
}

/// the second: colour by ranges of the load, arrows by numbers
fn graphic_two() -> DataGraphic {
    let mut g = DataGraphic::new("Auslastung");
    let mut cbv = Item::new(Kind::ColorByValue, "{Auslastung}", "");
    cbv.ranges = true;
    cbv.shades = vec![
        Shade { value: s("0"), to: s("50"), fill: 0xA9D18E, text: None },
        Shade { value: s("50"), to: s("80"), fill: 0xFFE699, text: None },
        Shade { value: s("80"), to: s("100"), fill: 0xF4B183, text: None },
    ];
    g.push(cbv);
    let mut arrows = Item::new(Kind::IconSet, "{Auslastung}", "DG Arrows");
    arrows.rules = vec![Rule::new(Op::Less, "50", ""), Rule::new(Op::NotUsed, "", ""), Rule::new(Op::Between, "50", "80"), Rule::new(Op::NotUsed, "", ""), Rule::new(Op::Greater, "80", "")];
    g.push(arrows);
    g
}

struct Drawing {
    doc: Document,
    page: PageRef,
    boxes: Vec<ShapeRef>,
    rs: u32,
}

// five boxes named by their text, linked to the workbook's rows by Name
fn linked(xlsx: &str) -> Result<Drawing, String> {
    let e = |e: diagram_core::EditError| format!("{e:?}");
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().ok_or("no page")?;
    let mut eng = Engine::new();
    let mut tx = doc.transaction("Geräte");
    let mut boxes = Vec::new();
    for (k, r) in ROWS.iter().enumerate() {
        let (x, y) = (1.0 + 2.6 * (k % 3) as f64, if k < 3 { 5.2 } else { 2.6 });
        let sh = ops::new_shape(&mut tx, &mut eng, Owner::Page(page), Basic::Rectangle, [x, y], [x + 1.4, y + 0.7]).map_err(e)?;
        ops::set_text(&mut tx, &mut eng, sh, Text::plain(r.0)).map_err(e)?;
        boxes.push(sh);
    }
    let (headers, rows) = table();
    let mut rs = Recordset::from_table("Geräte", &headers, rows, &[s("Name")]);
    let (cs, cmd) = data::excel_connection(xlsx, &ExcelTable::Sheet(s("Geraete")), true);
    rs.command = Some(cmd.clone());
    rs.time_refreshed = Some(s("2026-11-24T10:00:00"));
    let conn = Connection { connection_string: Some(cs), command: Some(cmd), ..Connection::default() };
    let id = data::add_recordset(&mut tx, Some(conn), rs).map_err(e)?;
    let (_, written) = data::auto_link(&mut tx, &boxes, id, &[(0, data::ShapeField::Text)], false).map_err(e)?;
    eng.recalc_cells(&mut tx, &written).map_err(e)?;
    let _ = tx.commit();
    Ok(Drawing { doc, page, boxes, rs: id })
}

// the counter-check: both graphics, the legend
fn check_drawing(xlsx: &str, src: &Document) -> Result<(Drawing, MasterRef, MasterRef, ShapeRef), String> {
    let e = |e: diagram_core::EditError| format!("{e:?}");
    let mut d = linked(xlsx)?;
    let mut eng = Engine::new();
    let mut tx = d.doc.transaction("Datengrafik");
    let m1 = dg::create(&mut tx, &mut eng, &graphic_one(), Some(src)).map_err(e)?;
    dg::apply(&mut tx, &mut eng, &d.boxes, m1, "Datengrafik").map_err(e)?;
    let t = legend::Texts { title: s("Legende"), text: s("Wert als Text"), ..legend::Texts::default() };
    let lg = legend::insert(&mut tx, &mut eng, d.page, false, &t, "Datengrafik").map_err(e)?;
    // page 2: two boxes linked to Server A and Router
    let p2 = tx.add_page("Seite-2", "Page-2", false, None).map_err(e)?;
    for (k, row) in [1u32, 3].into_iter().enumerate() {
        let x = 1.5 + 3.0 * k as f64;
        let sh = ops::new_shape(&mut tx, &mut eng, Owner::Page(p2), Basic::Rectangle, [x, 4.0], [x + 1.4, 4.7]).map_err(e)?;
        ops::set_text(&mut tx, &mut eng, sh, Text::plain(ROWS[row as usize - 1].0)).map_err(e)?;
        let w = data::link(&mut tx, sh, d.rs, row).map_err(e)?;
        eng.recalc_cells(&mut tx, &w).map_err(e)?;
    }
    let m2 = dg::create(&mut tx, &mut eng, &graphic_two(), Some(src)).map_err(e)?;
    let on2: Vec<ShapeRef> = tx.doc().top_shapes(Owner::Page(p2)).collect();
    dg::apply(&mut tx, &mut eng, &on2, m2, "Datengrafik").map_err(e)?;
    let _ = tx.commit();
    Ok((d, m1, m2, lg))
}

fn write_doc(doc: &Document) -> Vec<u8> {
    vsdx::write(doc, &WriteOptions::DEFAULT).unwrap_or_default()
}

/// --write-dg DIR
pub(crate) fn write(dir: &str) -> i32 {
    let xlsx = format!("{dir}\\jd19-daten.xlsx");
    let src = match callouts(Measure::Metric) {
        Ok(d) => d,
        Err(e) => {
            crate::out(&format!("{e}\n"));
            return 1;
        }
    };
    let check = check_drawing(&xlsx, &src).map(|(d, ..)| write_doc(&d.doc));
    // the scripts' fixture reads the workbook next to it (apps/JFDiagrammer/tests)
    let fixture = linked("apps/JFDiagrammer/tests/jd19-daten.xlsx").map(|d| write_doc(&d.doc));
    for (name, bytes) in [("jd19-datengrafik.vsdx", check), ("jd19-dg.vsdx", fixture)] {
        let path = format!("{dir}/{name}");
        match bytes {
            Ok(b) if !b.is_empty() && plat::write_file(&path, &b) => crate::out(&format!("wrote {path} ({} bytes)\n", b.len())),
            Ok(_) => {
                crate::out(&format!("cannot write {path}\n"));
                return 1;
            }
            Err(e) => {
                crate::out(&format!("{name}: {e}\n"));
                return 1;
            }
        }
    }
    0
}

// ---------------------------------------------------------------- checks

fn num(doc: &Document, sheet: SheetRef, sec: &str, row: &str, cell: &str) -> Option<f64> {
    let a = CellAddr::row(SecKey::one(doc.sym(sec)?), RowKey::Named(doc.sym(row)?), doc.sym(cell)?);
    doc.resolve(sheet, &a).num()
}

fn single(doc: &Document, sheet: SheetRef, cell: &str) -> Option<f64> {
    doc.resolve(sheet, &CellAddr::single(doc.sym(cell)?)).num()
}

fn fill(doc: &Document, s: ShapeRef) -> String {
    let Some(c) = doc.sym("FillForegnd") else { return String::new() };
    match doc.resolve(SheetRef::Shape(s), &CellAddr::single(c)).value() {
        Value::Text(t) => s_(doc.str(t)),
        v => format!("{v:?}"),
    }
}

fn s_(x: &str) -> String {
    String::from(x)
}

fn bar_fraction(doc: &Document, bar: ShapeRef) -> Option<f64> {
    num(doc, SheetRef::Shape(bar), "User", "F", "Value")
}

pub(crate) fn run(h: &mut H) {
    stencils(h);
    let Ok(src) = callouts(Measure::Metric) else { return };
    round_trip(h, &src);
    visio_shaped(h, &src);
}

// the callout masters
fn stencils(h: &mut H) {
    for m in [Measure::Metric, Measure::Us] {
        let doc = match callouts(m) {
            Ok(d) => d,
            Err(e) => {
                h.check("datagraphics/stencil", false, e);
                continue;
            }
        };
        let hidden = doc.sym("Hidden");
        let masters: Vec<MasterRef> = doc.masters().collect();
        let all_hidden = masters.iter().all(|&x| doc.master(x).is_some_and(|ms| hidden.and_then(|s| diagram_core::attr(&ms.header.attrs, s)).is_some_and(|v| doc.str(v) == "1")));
        h.check("datagraphics/stencil", masters.len() == 11 && all_hidden, format!("{m:?}: {} masters, all hidden: {all_hidden}", masters.len()));
        let mut kinds = Vec::new();
        for &x in &masters {
            let Some(slot) = doc.master(x).and_then(|ms| ms.master_shape()) else { continue };
            let sh = SheetRef::Shape(diagram_core::ShapeRef { owner: Owner::Master(x), slot });
            kinds.push(dg::user_text(&doc, sh, "msvCalloutType").and_then(|t| Kind::parse(&t)));
        }
        let n = |k: Kind| kinds.iter().filter(|x| **x == Some(k)).count();
        h.check("datagraphics/stencil", (n(Kind::Text), n(Kind::DataBar), n(Kind::IconSet)) == (3, 4, 4), format!("{m:?}: text, data bar, icon set masters {:?}", (n(Kind::Text), n(Kind::DataBar), n(Kind::IconSet))));
        // the sample values computed: a bar half full, the first icon, the text "Wert: 123"
        let ms = |name: &str| dg::find_master(&doc, name).and_then(|x| doc.master(x).and_then(|ms| ms.master_shape()).map(|slot| SheetRef::Shape(diagram_core::ShapeRef { owner: Owner::Master(x), slot })));
        let f = ms("DG Progress bar").and_then(|sh| num(&doc, sh, "User", "F", "Value"));
        h.check("datagraphics/stencil", f == Some(0.5), format!("{m:?}: progress bar at 50 of 0..100: {f:?}"));
        let text = ms("DG Text").map(|sh| diagram_core::field::shown(&doc, sh, 0));
        h.check("datagraphics/stencil", text.as_deref() == Some("Wert: 123"), format!("{m:?}: text callout shows {text:?}"));
        let stars = ms("DG Star rating").and_then(|sh| num(&doc, sh, "User", "N", "Value"));
        h.check("datagraphics/stencil", stars == Some(3.0), format!("{m:?}: 50 % are {stars:?} stars of 5"));
    }
}

fn round_trip(h: &mut H, src: &Document) {
    let (d, m1, m2, lg) = match check_drawing("C:\\Daten\\jd19-daten.xlsx", src) {
        Ok(x) => x,
        Err(e) => {
            h.check("datagraphics/build", false, e);
            return;
        }
    };
    let model1 = dg::read(&d.doc, m1);
    h.check("datagraphics/build", model1.as_ref().is_some_and(|g| g.items.len() == 4) && dg::shapes_with(&d.doc, m1).len() == 5, format!("graphic one on {} shapes", dg::shapes_with(&d.doc, m1).len()));
    let bytes = write_doc(&d.doc);
    let Some(doc) = crate::stencils::read_back(h, "datagraphics/roundtrip", &bytes) else { return };
    // the graphics and their models
    let gs = dg::graphics(&doc);
    h.check("datagraphics/roundtrip", gs.len() == 2, format!("{} data graphics after reading", gs.len()));
    let (Some(r1), Some(r2)) = (dg::find_master(&doc, "Geräte"), dg::find_master(&doc, "Auslastung")) else {
        h.check("datagraphics/roundtrip", false, s("the graphics by name"));
        return;
    };
    let same = dg::read(&doc, r1) == model1 && dg::read(&doc, r2) == dg::read(&d.doc, m2);
    h.check("datagraphics/roundtrip", same, s("both models read back as written (items, positions, options, rules, colours)"));
    let users: Vec<ShapeRef> = dg::shapes_with(&doc, r1);
    h.check("datagraphics/roundtrip", users.len() == 5 && users.iter().all(|&x| dg::callouts(&doc, x).len() == 3), format!("graphic one on {} shapes with three callouts each", users.len()));
    // what the callouts show: Server B 72 %, Warnung, 2,500.00 €, Raum 2
    let page = doc.pages().next();
    let b = page.and_then(|p| doc.top_shapes(Owner::Page(p)).find(|&x| doc.shape(x).and_then(|sh| sh.text()).is_some_and(|t| t.chars() == "Server B")));
    if let Some(b) = b {
        let cs = dg::callouts(&doc, b);
        let frac = cs.first().and_then(|&c| bar_fraction(&doc, c));
        let icon = cs.get(1).and_then(|&c| num(&doc, SheetRef::Shape(c), "User", "msvCalloutIconNumber", "Value"));
        let text = cs.get(2).map(|&c| diagram_core::field::shown(&doc, SheetRef::Shape(c), 0));
        h.check("datagraphics/values", frac == Some(0.72) && icon == Some(1.0), format!("Server B: bar {frac:?}, icon {icon:?}"));
        h.check("datagraphics/values", text.as_deref() == Some("Kosten: 2,500.00 €"), format!("Server B: text {text:?}"));
        h.check("datagraphics/values", fill(&doc, b) == "#bdd7ee", format!("Server B: fill {}", fill(&doc, b)));
        let layer = page.and_then(|p| doc.layer_by_name(p, dg::LAYER_U));
        h.check("datagraphics/values", layer.is_some() && cs.iter().all(|&c| doc.layer_members(c) == layer.into_iter().collect::<Vec<_>>()), format!("the callouts on the layer {layer:?}"));
        // the bar's place: far right, middle (the shape is 1.4 x 0.7 in)
        let px = cs.first().and_then(|&c| single(&doc, SheetRef::Shape(c), "PinX"));
        let w = cs.first().and_then(|&c| single(&doc, SheetRef::Shape(c), "Width")).unwrap_or(0.0);
        h.check("datagraphics/values", px.is_some_and(|x| (x - (1.4 + dg::GAP + w * 0.5)).abs() < 1e-9), format!("bar pin x {px:?}"));
    } else {
        h.check("datagraphics/values", false, s("Server B not found"));
    }
    // page 2: ranges and arrows
    let p2 = doc.pages().nth(1);
    let on2: Vec<ShapeRef> = p2.map(|p| doc.top_shapes(Owner::Page(p)).collect()).unwrap_or_default();
    let fills: Vec<String> = on2.iter().map(|&x| fill(&doc, x)).collect();
    let arrows: Vec<Option<f64>> = on2.iter().map(|&x| dg::callouts(&doc, x).first().and_then(|&c| num(&doc, SheetRef::Shape(c), "User", "msvCalloutIconNumber", "Value"))).collect();
    h.check("datagraphics/values", fills == ["#a9d18e", "#f4b183"] && arrows == [Some(0.0), Some(4.0)], format!("page 2: fills {fills:?}, arrows {arrows:?}"));
    // the legend: a list of four containers of lists
    let lgs: Vec<ShapeRef> = page.map(|p| doc.top_shapes(Owner::Page(p)).filter(|&x| legend::is_legend(&doc, x)).collect()).unwrap_or_default();
    let ok = lgs.len() == 1 && {
        let boxes = diagram_core::structure::items(&doc, lgs[0]);
        boxes.len() == 4 && boxes.iter().all(|&c| diagram_core::structure::members(&doc, c).len() == 1)
    };
    h.check("datagraphics/legend", ok && d.doc.shape(lg).is_some(), format!("{} legend(s), four containers with one list each: {ok}", lgs.len()));
    // a refresh: Server B at 40 % -- the bar follows in the same step
    let mut doc2 = doc.clone();
    let mut eng = Engine::new();
    let rs = doc2.data.recordsets.first().map(|r| r.id).unwrap_or(0);
    let (headers, mut rows) = table();
    rows[1][1] = DataValue::Num(40.0);
    let mut tx = doc2.transaction("Aktualisieren");
    let refreshed = data::refresh(&mut tx, rs, &headers, rows, "2026-11-24T11:00:00");
    let r = refreshed.map_err(|e| format!("{e:?}")).and_then(|r| eng.recalc_cells(&mut tx, &r.written).map_err(|e| format!("{e:?}")));
    let _ = tx.commit();
    let frac = b.and_then(|b| dg::callouts(&doc2, b).first().and_then(|&c| bar_fraction(&doc2, c)));
    h.check("datagraphics/refresh", r.is_ok() && frac == Some(0.4), format!("after the refresh the bar shows {frac:?}"));
    // removal: callouts gone, the old colours back, the rows gone; written again it is plain
    let mut tx = doc2.transaction("Keine Datengrafik");
    let users = dg::shapes_with(tx.doc(), r1);
    let r = dg::remove(&mut tx, &mut eng, &users);
    let _ = tx.commit();
    let clean = users.iter().all(|&x| dg::callouts(&doc2, x).is_empty() && dg::applied(&doc2, x).is_none() && doc2.shape(x).is_some_and(|sh| sh.header().kind == diagram_core::ShapeKind::Shape));
    h.check("datagraphics/remove", r.is_ok() && clean, format!("graphic one removed from {} shapes: {clean}", users.len()));
}

// a data graphic as Visio stores one: the item a copy of the callout master's shape, the shape
// naming the graphic by its UniqueID, the icon formula of Visio's own
fn visio_shaped(h: &mut H, src: &Document) {
    let e = |e: diagram_core::EditError| format!("{e:?}");
    let build = || -> Result<(Document, MasterRef, ShapeRef), String> {
        let mut d = linked("C:\\Daten\\jd19-daten.xlsx")?;
        let mut eng = Engine::new();
        let mut tx = d.doc.transaction("Visio");
        let cm = diagram_core::instance::import_master(&mut tx, src, dg::find_master(src, "DG Traffic lights").ok_or("no traffic lights")?).map_err(e)?;
        let m = tx.add_master("Data - Geräte", "Data - Geräte").map_err(e)?;
        let mut hd = tx.doc().master(m).ok_or("master")?.header.clone();
        hd.unique_id = Some(tx.intern("{0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F9}"));
        tx.set_master_header(m, hd).map_err(e)?;
        let ps = SheetRef::Master(m);
        tx.set_user(ps, "visUSEType", Value::Num(1.0), None).map_err(e)?;
        tx.set_user(ps, "visDGNextItemID", Value::Num(2.0), None).map_err(e)?;
        let g = tx.add_shape(Owner::Master(m), None, diagram_core::ShapeKind::Group).map_err(e)?;
        for (c, v) in [("PinX", 0.5), ("PinY", 0.5), ("Width", 1.0), ("Height", 1.0)] {
            let a = tx.single(c);
            tx.set_constant(SheetRef::Shape(g), &a, Value::Num(v), None).map_err(e)?;
        }
        tx.set_user_formula(SheetRef::Shape(g), "visDGDefaultPos", "PNT(6,3)").map_err(e)?;
        // the item: a plain shape (Visio's copy), its cells as Visio writes them
        let it = tx.add_shape(Owner::Master(m), Some(g), diagram_core::ShapeKind::Shape).map_err(e)?;
        let is = SheetRef::Shape(it);
        let (ty, f0) = (tx.intern("Icon Set"), tx.intern("{Status}"));
        tx.set_user(is, "msvCalloutType", Value::Text(ty), Some(diagram_core::Unit::Str)).map_err(e)?;
        tx.set_user(is, "visDGField0", Value::Text(f0), Some(diagram_core::Unit::Str)).map_err(e)?;
        tx.set_user(is, "visDGItemID", Value::Num(1.0), None).map_err(e)?;
        tx.set_user(is, "visDGDefaultPos", Value::Num(1.0), Some(diagram_core::Unit::Bool)).map_err(e)?;
        tx.set_user_formula(is, "visDGUseMaster", "USE(\"DG Traffic lights\")").map_err(e)?;
        tx.set_user_formula(is, "msvCalloutIconNumber", "IF(STRSAME(Prop.msvCalloutField,\"Fehler\"),2,IF(STRSAME(Prop.msvCalloutField,\"OK\"),0,1))").map_err(e)?;
        eng.recalc(&mut tx, &[ps, SheetRef::Shape(g), is]).map_err(e)?;
        // a shape showing it: the callout instance, the shape's cells
        let target = d.boxes[0];
        let ts = SheetRef::Shape(target);
        tx.set_user_formula(ts, "visDGDisplayFormat", "USE(\"{0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F9}\")").map_err(e)?;
        let k = tx.add_instance(Owner::Page(d.page), cm, 0.0, 0.0).map_err(e)?;
        tx.move_shape(k, Some(target), None).map_err(e)?;
        tx.set_user(SheetRef::Shape(k), "visDGCalloutItem", Value::Num(1.0), None).map_err(e)?;
        let _ = tx.commit();
        let _ = cm;
        Ok((d.doc, m, d.boxes[1]))
    };
    let (doc, _, other) = match build() {
        Ok(x) => x,
        Err(err) => {
            h.check("datagraphics/visio", false, err);
            return;
        }
    };
    let bytes = write_doc(&doc);
    let Some(mut back) = crate::stencils::read_back(h, "datagraphics/visio", &bytes) else { return };
    let Some(m) = dg::find_master(&back, "Data - Geräte") else {
        h.check("datagraphics/visio", false, s("the graphic by name"));
        return;
    };
    let g = dg::read(&back, m);
    let ok = g.as_ref().is_some_and(|g| {
        g.items.len() == 1 && g.items[0].kind == Kind::IconSet && g.items[0].field == "{Status}" && g.items[0].callout == "DG Traffic lights" && g.items[0].pos.is_none() && g.items[0].icon_formula.as_deref().is_some_and(|f| f.starts_with("IF(STRSAME")) && g.default_pos == Pos::DEFAULT
    });
    h.check("datagraphics/visio", ok, format!("Visio's graphic read: {:?}", g.map(|g| g.items)));
    let first = back.pages().next().and_then(|p| back.top_shapes(Owner::Page(p)).next());
    h.check("datagraphics/visio", first.is_some_and(|f| dg::applied(&back, f) == Some(m) && dg::callouts(&back, f).len() == 1), s("the shape names it by UniqueID, its callout found"));
    // applied to another shape: the kept formula passes on (Router: Fehler -> 2)
    let mut eng = Engine::new();
    let other = back.pages().next().and_then(|p| back.top_shapes(Owner::Page(p)).nth(2)).unwrap_or(other);
    let mut tx = back.transaction("anwenden");
    let r = dg::apply(&mut tx, &mut eng, &[other], m, "Datengrafik");
    let _ = tx.commit();
    let icon = dg::callouts(&back, other).first().and_then(|&c| num(&back, SheetRef::Shape(c), "User", "msvCalloutIconNumber", "Value"));
    h.check("datagraphics/visio", r.is_ok_and(|v| v.len() == 1) && icon == Some(2.0), format!("applied anew: icon {icon:?}"));
    // what was not ours stays: the master's other cells written again unchanged
    let again = write_doc(&back);
    h.check("datagraphics/visio", vsdx::read(&again, &Limits::VSDX).is_ok(), format!("written again: {} bytes", again.len()));
}
