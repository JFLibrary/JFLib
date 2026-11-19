// stencil_shapes -- (JD13) the shape stencils of apps/JFDiagrammer/stencils: blocks, arrows, the
// flowchart after ISO 5807 and the connectors (1-D masters); diagram_io::stencil, own sources only
//
//   stencils/shapes/<set>/<measure>   both measures build; write -> read -> write is the same
//                       (roundtrip); the masters by NameU and German name with names, QuickInfos
//                       and keywords in five languages; the stencil's title and category; the
//                       master pages in mm or in; the engine keeps nothing
//   stencils/shapes/flowchart/<NameU>   User.msvShapeCategories "Flowchart", four connection
//                       points
//   stencils/shapes/1d/<NameU>   the 1-D masters (connectors, 1-D arrow): BeginX ... on the master
//                       shape, PinX and Width as GUARD formulas of the ends, the master page from
//                       the ends; an instance with its ends set elsewhere: the pin in the middle,
//                       Width the length (upright connectors: EndX-BeginX), Angle along the line
//   stencils/shapes/1d/glue   ends glued to connection points of a box follow the moved box
//   stencils/shapes/drop   every master dropped the way JFDiagrammer drops (instance::place) and
//                       recalculated: nothing kept, no error values; the message tells where a
//                       dropped 1-D shape's ends lie (instance::place sets the pin, not the ends)
//
//   diagramiotest --write-stencil-previews DIR   also probeblatt.png (previews): every new master
//                       and the Tip in its own size and in a variant, the connectors glued between
//                       boxes, a small flow, control points as small yellow diamonds, connection
//                       points as small blue dots, labelled in German and English; and
//                       probeblatt-zeile1.png .. -zeile5.png: each row enlarged (300 dpi);
//                       probeblatt-kontur-abgerundet.png, probeblatt-kontrast.png: the sheet in
//                       the designs Kontur (variant Abgerundet) and Kontrast

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use diagram_core::design::apply::{self as design_apply, Scope};
use diagram_core::design::catalog;
use diagram_core::formula::{math, Engine, Outcome};
use diagram_core::glue::{self, End, Target};
use diagram_core::instance::{self, MasterTexts, StencilTexts};
use diagram_core::{ops, CellAddr, DocKind, Document, MasterRef, Owner, PageRef, RowKey, SecKey, ShapeRef, SheetRef, Transaction, Unit, Value};
use diagram_core::render::Options;
use diagram_io::vsdx::{self, Measure};
use diagram_view::export::raster;
use diagram_view::PageView;

use crate::stencil_structs::{none, put, render_page_at, set_single, sheet_doc, CALLOUTS};
use crate::stencils::{build_set, cell, master_shape, named, read_back, sym_text, STANDARD};
use crate::{out, H};

const BLOCKS: &[(&str, &str)] = &[
    ("stencil.txt", include_str!("../../JFDiagrammer/stencils/blocks/stencil.txt")),
    ("box_arrow.master", include_str!("../../JFDiagrammer/stencils/blocks/box_arrow.master")),
    ("box_3d.master", include_str!("../../JFDiagrammer/stencils/blocks/box_3d.master")),
    ("partial_layer.master", include_str!("../../JFDiagrammer/stencils/blocks/partial_layer.master")),
    ("arrow_1d.master", include_str!("../../JFDiagrammer/stencils/blocks/arrow_1d.master")),
];

const ARROWS: &[(&str, &str)] = &[
    ("stencil.txt", include_str!("../../JFDiagrammer/stencils/arrows/stencil.txt")),
    ("simple_arrow.master", include_str!("../../JFDiagrammer/stencils/arrows/simple_arrow.master")),
    ("bent_arrow.master", include_str!("../../JFDiagrammer/stencils/arrows/bent_arrow.master")),
    ("u_turn_arrow.master", include_str!("../../JFDiagrammer/stencils/arrows/u_turn_arrow.master")),
    ("quad_arrow.master", include_str!("../../JFDiagrammer/stencils/arrows/quad_arrow.master")),
];

const FLOWCHART: &[(&str, &str)] = &[
    ("stencil.txt", include_str!("../../JFDiagrammer/stencils/flowchart/stencil.txt")),
    ("process.master", include_str!("../../JFDiagrammer/stencils/flowchart/process.master")),
    ("decision.master", include_str!("../../JFDiagrammer/stencils/flowchart/decision.master")),
    ("document.master", include_str!("../../JFDiagrammer/stencils/flowchart/document.master")),
    ("database.master", include_str!("../../JFDiagrammer/stencils/flowchart/database.master")),
];

const CONNECTORS: &[(&str, &str)] = &[
    ("stencil.txt", include_str!("../../JFDiagrammer/stencils/connectors/stencil.txt")),
    ("curved_connector.master", include_str!("../../JFDiagrammer/stencils/connectors/curved_connector.master")),
    ("elbow_connector.master", include_str!("../../JFDiagrammer/stencils/connectors/elbow_connector.master")),
    ("hollow_connector.master", include_str!("../../JFDiagrammer/stencils/connectors/hollow_connector.master")),
    ("line_arrow.master", include_str!("../../JFDiagrammer/stencils/connectors/line_arrow.master")),
];

type Want = &'static [(&'static str, &'static str)];

/// per stencil: its folder, its sources, its category, its masters (NameU, German name)
const SETS: [(&str, &[(&str, &str)], &str, Want); 4] = [
    ("blocks", BLOCKS, "general", &[("Box with arrow", "Feld mit Pfeil"), ("3-D box", "3D-Feld"), ("Partial layer", "Teilebene"), ("1-D arrow", "1D-Pfeil")]),
    ("arrows", ARROWS, "flowchart", &[("Simple arrow", "Einfacher Pfeil"), ("Bent arrow", "Gebogener Pfeil"), ("U-turn arrow", "180-Grad-Pfeil"), ("Quad arrow", "Pfeil in vier Richtungen")]),
    ("flowchart", FLOWCHART, "flowchart", &[("Process", "Prozess"), ("Decision", "Entscheidung"), ("Document", "Dokument"), ("Database", "Datenbank")]),
    ("connectors", CONNECTORS, "extras", &[("Curved connector", "Kurvenverbinder"), ("Elbow connector", "Winkelverbinder"), ("Hollow connector", "Hohler Verbinder"), ("Line with arrow", "Linie mit Pfeil")]),
];

/// the 1-D masters: stencil index in SETS, NameU, whether the box stays upright (Width =
/// EndX-BeginX, Angle 0) instead of lying along the line
const ONE_D: [(usize, &str, bool); 5] =
    [(3, "Curved connector", true), (3, "Elbow connector", true), (3, "Hollow connector", false), (3, "Line with arrow", false), (0, "1-D arrow", false)];

const MM: f64 = 1.0 / 25.4;

// ---------------------------------------------------------------- helpers

fn num(doc: &Document, s: SheetRef, n: &str) -> f64 {
    cell(doc, s, None, None, n).and_then(|c| c.value.num()).unwrap_or(f64::NAN)
}

fn formula_text(doc: &Document, s: SheetRef, n: &str) -> String {
    cell(doc, s, None, None, n).and_then(|c| c.formula.expr()).map(|f| String::from(doc.str(f))).unwrap_or_default()
}

fn user_text(doc: &Document, s: SheetRef, n: &str) -> String {
    cell(doc, s, Some(("User", 0)), named(doc, n), "Value").map(|c| sym_text(doc, c.value)).unwrap_or_default()
}

fn master_of(doc: &Document, nu: &str) -> Option<MasterRef> {
    doc.masters().find(|&m| MasterTexts::of(doc, m).name_u == nu)
}

// a master page's cell (PageWidth ...): its value and unit
fn page_cell(doc: &Document, m: MasterRef, n: &str) -> (f64, Option<Unit>) {
    let c = doc.sym(n).and_then(|s| doc.master(m).and_then(|x| x.sheet.cell(s)));
    (c.and_then(|c| c.value.num()).unwrap_or(f64::NAN), c.and_then(|c| c.unit))
}

// the formula cells of a document the engine keeps or that come out as an error, and how many
// values a calculation would change
fn engine_state(doc: &Document) -> (usize, usize, Vec<String>) {
    let mut d = doc.clone();
    let res = Engine::new().calculate(&d);
    let bad: Vec<String> = res
        .cells
        .iter()
        .filter(|(_, _, o)| matches!(o, Outcome::Kept(_)) || matches!(o, Outcome::Value(v) if v.is_err()))
        .map(|(_, a, o)| format!("{} {o:?}", d.str(a.cell)))
        .chain(res.cycles.iter().map(|(_, a)| format!("{} cycle", d.str(a.cell))))
        .collect();
    let mut tx = d.transaction("check");
    let written = res.write(&mut tx).unwrap_or(usize::MAX);
    drop(tx);
    (res.cells.len(), written, bad)
}

/// an instance of the 1-D master `nu` of `src` with its ends at `b` and `e` (mm on the page): the
/// pin instance::place gives it goes again, so that the master's 1-D formulas place the box
pub(crate) fn put_1d(tx: &mut Transaction, src: &Document, page: PageRef, nu: &str, b: (f64, f64), e: (f64, f64)) -> Option<ShapeRef> {
    let m = master_of(src, nu)?;
    let local = instance::import_master(tx, src, m).ok()?;
    let s = instance::place(tx, page, local, 0.0, 0.0, 0).ok()?;
    let sheet = SheetRef::Shape(s);
    for n in ["PinX", "PinY"] {
        let a = CellAddr::single(tx.intern(n));
        tx.inherit_cell(sheet, &a).ok()?;
    }
    for (n, v) in [("BeginX", b.0), ("BeginY", b.1), ("EndX", e.0), ("EndY", e.1)] {
        set_single(tx, sheet, n, v * MM, Some(Unit::Millimeters));
    }
    Some(s)
}

// a control point's X and Y (mm, local) on an instance
fn set_ctl(tx: &mut Transaction, s: ShapeRef, row: &str, x: Option<f64>, y: Option<f64>) {
    let sk = SecKey::one(tx.intern("Control"));
    let rk = RowKey::Named(tx.intern(row));
    for (c, v) in [("X", x), ("Y", y)] {
        if let Some(v) = v {
            let a = CellAddr::row(sk, rk, tx.intern(c));
            let _ = tx.set_constant(SheetRef::Shape(s), &a, Value::Num(v * MM), Some(Unit::Millimeters));
        }
    }
}

// ---------------------------------------------------------------- the checks

/// the group "stencils/shapes" (called with the group "stencils")
pub fn run(h: &mut H) {
    let mut metric: Vec<Option<Document>> = Vec::new();
    for (name, set, category, want) in SETS {
        let mut kept_metric = None;
        for m in [Measure::Metric, Measure::Us] {
            let tag = format!("stencils/shapes/{name}/{}", if m == Measure::Us { "us" } else { "metric" });
            let bytes = match build_set(set, m) {
                Ok((_, b)) => b,
                Err(e) => {
                    h.check(&tag, false, e.to_string());
                    continue;
                }
            };
            h.check(&tag, true, format!("{} bytes", bytes.len()));
            let Some(doc) = read_back(h, &format!("{tag}/roundtrip"), &bytes) else { continue };
            // the masters: NameU, German name, five languages
            let got: Vec<(String, String)> = doc.masters().map(|mr| {
                let t = MasterTexts::of(&doc, mr);
                (t.name_u.clone(), t.names[0].clone())
            }).collect();
            let five = doc.masters().all(|mr| {
                let t = MasterTexts::of(&doc, mr);
                t.names.iter().chain(t.prompts.iter()).chain(t.keywords.iter()).all(|x| !x.is_empty())
            });
            let same = got.len() == want.len() && got.iter().zip(want.iter()).all(|(g, w)| g.0 == w.0 && g.1 == w.1);
            h.check(&format!("{tag}/masters"), same && five, format!("{got:?}, five languages {five}"));
            let st = StencilTexts::of(&doc);
            h.check(&format!("{tag}/stencil"), st.names.iter().all(|n| !n.is_empty()) && st.category == category, format!("{:?} {}", st.names, st.category));
            // the master pages in the measure's unit
            let want_unit = if m == Measure::Us { Unit::Inches } else { Unit::Millimeters };
            let units: Vec<Option<Unit>> = doc.masters().map(|mr| page_cell(&doc, mr, "PageWidth").1).collect();
            h.check(&format!("{tag}/measure"), units.iter().all(|u| *u == Some(want_unit)), format!("PageWidth units {units:?}"));
            let (cells, written, bad) = engine_state(&doc);
            h.check(&format!("{tag}/engine"), bad.is_empty() && written == 0, format!("{cells} formula cells, {written} changed, kept {bad:?}"));
            if m == Measure::Metric {
                kept_metric = Some(doc);
            }
        }
        metric.push(kept_metric);
    }
    let docs: Vec<&Document> = metric.iter().flatten().collect();
    if docs.len() != SETS.len() {
        return;
    }
    flowchart(h, docs[2]);
    one_d(h, &docs);
    drop_all(h, &docs);
}

fn flowchart(h: &mut H, doc: &Document) {
    for m in doc.masters() {
        let nu = MasterTexts::of(doc, m).name_u;
        let Some(s) = master_shape(doc, m) else { continue };
        let cat = user_text(doc, s, "msvShapeCategories");
        let conn = doc.sym("Connection").map_or(0, |c| doc.rows(s, SecKey::one(c)).len());
        h.check(&format!("stencils/shapes/flowchart/{nu}"), cat == "Flowchart" && conn == 4, format!("msvShapeCategories {cat:?}, {conn} connection points"));
    }
}

fn one_d(h: &mut H, docs: &[&Document]) {
    // the master shapes
    for (set, nu, _) in ONE_D {
        let src = docs[set];
        let tag = format!("stencils/shapes/1d/{nu}");
        let Some((m, ms)) = master_of(src, nu).and_then(|m| Some((m, master_shape(src, m)?))) else {
            h.check(&tag, false, String::from("no such master"));
            continue;
        };
        let e: Vec<f64> = ["BeginX", "BeginY", "EndX", "EndY"].iter().map(|n| num(src, ms, n)).collect();
        let (pin_f, w_f) = (formula_text(src, ms, "PinX"), formula_text(src, ms, "Width"));
        let (pw, ph) = (page_cell(src, m, "PageWidth").0, page_cell(src, m, "PageHeight").0);
        let ok = e.iter().all(|x| x.is_finite())
            && pin_f == "GUARD((BeginX+EndX)/2)"
            && w_f.starts_with("GUARD(")
            && (pw - (e[0] + e[2])).abs() < 1e-9
            && (ph - (e[1] + e[3])).abs() < 1e-9
            && (num(src, ms, "PinX") - (e[0] + e[2]) * 0.5).abs() < 1e-9;
        h.check(&format!("{tag}/master"), ok, format!("ends {:?} mm, PinX {pin_f}, Width {w_f}, page {:.2} x {:.2} mm", e.iter().map(|x| x / MM).collect::<Vec<_>>(), pw / MM, ph / MM));
    }
    // instances with their ends elsewhere: the box follows
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let Some(page) = doc.pages().next() else { return };
    let mut eng = Engine::new();
    let mut tx = doc.transaction("1d");
    let mut placed: Vec<(&str, bool, Option<ShapeRef>, f64)> = Vec::new();
    for (k, &(set, nu, upright)) in ONE_D.iter().enumerate() {
        let y = 20.0 + 30.0 * k as f64;
        placed.push((nu, upright, put_1d(&mut tx, docs[set], page, nu, (20.0, y), (80.0, y + 25.0)), y));
    }
    let sheets: Vec<SheetRef> = placed.iter().filter_map(|p| p.2).map(SheetRef::Shape).collect();
    let kept = eng.recalc(&mut tx, &sheets).map_or(usize::MAX, |r| r.kept.len());
    for &(nu, upright, s, y) in &placed {
        let tag = format!("stencils/shapes/1d/{nu}/instance");
        let Some(s) = s else {
            h.check(&tag, false, String::from("not placed"));
            continue;
        };
        let d = tx.doc();
        let sh = SheetRef::Shape(s);
        let (px, py, w, a) = (num(d, sh, "PinX") / MM, num(d, sh, "PinY") / MM, num(d, sh, "Width") / MM, num(d, sh, "Angle"));
        let (want_w, want_a) = if upright { (60.0, 0.0) } else { (math::sqrt(60.0 * 60.0 + 25.0 * 25.0), math::atan2(25.0, 60.0)) };
        let ends = glue::ends(d, s).map(|(b, e)| [b[0] / MM, b[1] / MM, e[0] / MM, e[1] / MM]);
        let ok = kept == 0
            && glue::is_one_d(d, s)
            && !glue::is_connector(d, s)
            && (px - 50.0).abs() < 1e-6
            && (py - (y + 12.5)).abs() < 1e-6
            && (w - want_w).abs() < 1e-6
            && (a - want_a).abs() < 1e-9
            && ends.is_some_and(|e| (e[0] - 20.0).abs() < 1e-6 && (e[3] - (y + 25.0)).abs() < 1e-6);
        h.check(&tag, ok, format!("recalc kept {kept}, pin ({px:.3}, {py:.3}) mm, Width {w:.3} mm, Angle {a:.4}, ends {ends:?}"));
    }
    // glue: the line's end and the elbow connector's begin to a box's connection points; the box
    // moves, both ends follow
    let bx = put(&mut tx, docs[2], page, "Process", (140.0, 60.0), (0.0, 0.0), None, &none);
    let (line, elbow) = (placed[3].2, placed[1].2);
    let mut what = String::new();
    let ok = match (bx, line, elbow) {
        (Some(bx), Some(line), Some(elbow)) => {
            let r = (|| -> Result<bool, diagram_core::EditError> {
                eng.recalc(&mut tx, &[SheetRef::Shape(bx)])?;
                let point = |d: &Document, row: u32| glue::connection_points(d, bx).into_iter().find(|p| p.row == row).map_or([0.0; 2], |p| p.at);
                let (left, right) = (point(tx.doc(), 3), point(tx.doc(), 1));
                glue::glue(&mut tx, &mut eng, line, End::End, left, Target::Point { shape: bx, row: 3 })?;
                glue::glue(&mut tx, &mut eng, elbow, End::Begin, right, Target::Point { shape: bx, row: 1 })?;
                set_single(&mut tx, SheetRef::Shape(bx), "PinY", 95.0 * MM, Some(Unit::Millimeters));
                eng.recalc(&mut tx, &[SheetRef::Shape(bx)])?;
                glue::update(&mut tx, &mut eng, &[line, elbow])?;
                let d = tx.doc();
                let (left, right) = (point(d, 3), point(d, 1));
                let le = glue::ends(d, line).map_or([0.0; 2], |e| e.1);
                let eb = glue::ends(d, elbow).map_or([0.0; 2], |e| e.0);
                let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
                let pin = (num(d, SheetRef::Shape(line), "PinY") / MM, glue::ends(d, line).map_or(0.0, |e| (e.0[1] + e.1[1]) * 0.5 / MM));
                what = format!("line end {:?} at the left point {:?}, elbow begin {:?} at the right point {:?}, line PinY {:.3} = middle {:.3} mm", le.map(|v| v / MM), left.map(|v| v / MM), eb.map(|v| v / MM), right.map(|v| v / MM), pin.0, pin.1);
                Ok(near(le, left) && near(eb, right) && (pin.0 - pin.1).abs() < 1e-6 && (left[1] / MM - 95.0).abs() < 1e-6)
            })();
            r.unwrap_or_else(|e| {
                what = format!("{e:?}");
                false
            })
        }
        _ => false,
    };
    h.check("stencils/shapes/1d/glue", ok, what);
    drop(tx);
}

// every master dropped the way JFDiagrammer drops it (instance::place in French + recalc)
fn drop_all(h: &mut H, docs: &[&Document]) {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let Some(page) = doc.pages().next() else { return };
    let fr = instance::lang_index("fr").unwrap_or(4);
    let mut tx = doc.transaction("drop");
    let mut shapes: Vec<(String, ShapeRef)> = Vec::new();
    let mut fail = String::new();
    for (k, src) in docs.iter().enumerate() {
        for (i, m) in src.masters().enumerate() {
            let nu = MasterTexts::of(src, m).name_u;
            match instance::import_master(&mut tx, src, m).and_then(|local| instance::place(&mut tx, page, local, 2.0 + 2.0 * i as f64, 2.0 + 2.0 * k as f64, fr)) {
                Ok(s) => shapes.push((nu, s)),
                Err(e) => fail.push_str(&format!(" {nu}: {e:?}")),
            }
        }
    }
    let sheets: Vec<SheetRef> = shapes.iter().map(|(_, s)| SheetRef::Shape(*s)).collect();
    let kept = Engine::new().recalc(&mut tx, &sheets).map_or(usize::MAX, |r| r.kept.len());
    let _ = tx.commit();
    let (_, _, bad) = engine_state(&doc);
    // where the ends of a dropped 1-D shape are: instance::place gives it a pin of its own
    let off: Vec<String> = shapes
        .iter()
        .filter(|(_, s)| glue::is_one_d(&doc, *s))
        .filter_map(|(nu, s)| {
            let (b, e) = glue::ends(&doc, *s)?;
            let sh = SheetRef::Shape(*s);
            let (px, py) = (num(&doc, sh, "PinX"), num(&doc, sh, "PinY"));
            let (mx, my) = ((b[0] + e[0]) * 0.5, (b[1] + e[1]) * 0.5);
            ((px - mx).abs() > 1e-9 || (py - my).abs() > 1e-9).then(|| format!("{nu}: pin ({:.1}, {:.1}) mm, ends' middle ({:.1}, {:.1}) mm", px / MM, py / MM, mx / MM, my / MM))
        })
        .collect();
    let n = shapes.len();
    h.check(
        "stencils/shapes/drop",
        fail.is_empty() && n == 16 && kept == 0 && bad.is_empty(),
        format!("{n} instances, recalc kept {kept}, errors {bad:?}{fail}; 1-D ends apart from the pin after instance::place: {}", if off.is_empty() { String::from("none") } else { off.join("; ") }),
    );
}

// ---------------------------------------------------------------- the probe sheet

// a marker's look: fill, line (None: no line)
fn paint(tx: &mut Transaction, s: ShapeRef, fill: &str, line: Option<&str>) {
    let sheet = SheetRef::Shape(s);
    let f = tx.intern(fill);
    for n in ["FillForegnd", "FillBkgnd"] {
        let a = CellAddr::single(tx.intern(n));
        let _ = tx.set_constant(sheet, &a, Value::Text(f), None);
    }
    set_single(tx, sheet, "FillPattern", 1.0, None);
    match line {
        Some(l) => {
            let c = tx.intern(l);
            let a = CellAddr::single(tx.intern("LineColor"));
            let _ = tx.set_constant(sheet, &a, Value::Text(c), None);
            set_single(tx, sheet, "LinePattern", 1.0, None);
            set_single(tx, sheet, "LineWeight", 0.5 / 72.0, Some(Unit::Points));
        }
        None => set_single(tx, sheet, "LinePattern", 0.0, None),
    }
}

// a label's paragraph centred
fn centred(tx: &mut Transaction, s: ShapeRef) {
    let sk = SecKey::one(tx.intern("Paragraph"));
    let a = CellAddr::row(sk, RowKey::Index(0), tx.intern("HorzAlign"));
    let _ = tx.set_constant(SheetRef::Shape(s), &a, Value::Num(1.0), None);
}

/// diagramiotest --write-stencil-previews DIR: probeblatt.png
pub fn previews(dir: &str) -> i32 {
    let built = |set: &[(&str, &str)]| build_set(set, Measure::Metric).ok().map(|(b, _)| b.doc);
    let (Some(bl), Some(ar), Some(fc), Some(cn), Some(st), Some(co)) = (built(BLOCKS), built(ARROWS), built(FLOWCHART), built(CONNECTORS), built(STANDARD), built(CALLOUTS)) else {
        out("error: a shape stencil does not build (diagramiotest --check --only stencils/shapes)\n");
        return 1;
    };
    const PW: f64 = 420.0;
    const PH: f64 = 312.0;
    let Some((mut doc, page)) = sheet_doc(PW, PH) else { return 1 };
    // eight columns of 50 mm, five rows of 58 mm under the title: the row's title at its top,
    // shapes up to 42 mm high, the labels under them
    let col = |c: usize| 35.0 + c as f64 * 50.0;
    let band = |r: usize| PH - 18.0 - r as f64 * 58.0;
    let at = |r: usize| band(r) - 25.0;
    let lab = |r: usize| band(r) - 51.0;
    let mut marked: Vec<ShapeRef> = Vec::new();
    let mut tx = doc.transaction("probe");
    let mut eng = Engine::new();
    let label = |tx: &mut Transaction, x: f64, y: f64, w: f64, t: &str| {
        put(tx, &st, page, "Text box", (x, y), (w, 9.0), Some(t), &centred);
    };
    let title = |tx: &mut Transaction, r: usize, t: &str| {
        put(tx, &st, page, "Text box", (10.0 + 100.0, band(r) - 1.0), (200.0, 6.0), Some(t), &none);
    };
    put(&mut tx, &st, page, "Text box", (10.0 + 200.0, PH - 6.0), (400.0, 8.0), Some("JD13 Probeblatt: neue Schablonen im NetLab-Stil (Formatvorlagen Normal / Connector, Design NetLab) – gelbe Raute = Steuerpunkt, blauer Punkt = Verbindungspunkt"), &none);
    // ---- row 0: Blöcke
    title(&mut tx, 0, "Blöcke (blocks, general)");
    let y = at(0);
    let mut add = |s: Option<ShapeRef>| marked.extend(s);
    add(put(&mut tx, &bl, page, "Box with arrow", (col(0), y), (0.0, 0.0), Some("Text"), &none));
    add(put(&mut tx, &bl, page, "Box with arrow", (col(1), y), (46.0, 36.0), Some("Variante"), &|tx, s| {
        set_ctl(tx, s, "Row_1", None, Some(16.0));
        set_ctl(tx, s, "Row_2", Some(15.0), Some(9.0));
    }));
    add(put(&mut tx, &bl, page, "3-D box", (col(2), y), (0.0, 0.0), Some("Text"), &none));
    add(put(&mut tx, &bl, page, "3-D box", (col(3), y), (46.0, 32.0), Some("Tiefe 9 mm"), &|tx, s| set_ctl(tx, s, "Row_1", Some(9.0), None)));
    add(put(&mut tx, &bl, page, "Partial layer", (col(4), y), (0.0, 0.0), Some("Text"), &none));
    // two layers around the same corner: 40 mm with 60 % inside, 24 mm with 50 %
    let corner = (col(5) - 20.0, y - 20.0);
    add(put(&mut tx, &bl, page, "Partial layer", (corner.0 + 20.0, corner.1 + 20.0), (40.0, 40.0), Some("Ebene 2"), &|tx, s| set_ctl(tx, s, "Row_1", Some(24.0), None)));
    add(put(&mut tx, &bl, page, "Partial layer", (corner.0 + 12.0, corner.1 + 12.0), (24.0, 24.0), Some("1"), &none));
    add(put_1d(&mut tx, &bl, page, "1-D arrow", (col(6) - 20.0, y), (col(6) + 20.0, y)));
    add(put_1d(&mut tx, &bl, page, "1-D arrow", (col(7) - 20.0, y - 12.0), (col(7) + 20.0, y + 12.0)).inspect(|&s| set_ctl(&mut tx, s, "Row_2", None, Some(8.5))));
    let ly = lab(0);
    for (c, t) in [(0, "Feld mit Pfeil / Box with arrow"), (1, "Variante 46 × 36"), (2, "3D-Feld / 3-D box"), (3, "Variante, Tiefe 9 mm"), (4, "Teilebene / Partial layer"), (5, "zwei Teilebenen"), (6, "1D-Pfeil / 1-D arrow"), (7, "1D-Pfeil schräg")] {
        label(&mut tx, col(c), ly, 48.0, t);
    }
    // ---- row 1: Pfeil-Shapes
    title(&mut tx, 1, "Pfeil-Shapes (arrows, flowchart)");
    let y = at(1);
    add(put(&mut tx, &ar, page, "Simple arrow", (col(0), y), (0.0, 0.0), Some("Text"), &none));
    add(put(&mut tx, &ar, page, "Simple arrow", (col(1), y), (46.0, 16.0), None, &|tx, s| set_ctl(tx, s, "Row_1", Some(34.0), Some(12.0))));
    add(put(&mut tx, &ar, page, "Bent arrow", (col(2), y), (0.0, 0.0), Some("Text"), &none));
    add(put(&mut tx, &ar, page, "Bent arrow", (col(3), y), (44.0, 36.0), None, &|tx, s| {
        set_ctl(tx, s, "Row_1", Some(32.0), None);
        set_ctl(tx, s, "Row_2", None, Some(24.0));
    }));
    add(put(&mut tx, &ar, page, "U-turn arrow", (col(4), y), (0.0, 0.0), None, &none));
    add(put(&mut tx, &ar, page, "U-turn arrow", (col(5), y), (36.0, 40.0), None, &|tx, s| {
        set_ctl(tx, s, "Row_1", Some(5.0), None);
        set_ctl(tx, s, "Row_2", None, Some(12.0));
    }));
    add(put(&mut tx, &ar, page, "Quad arrow", (col(6), y), (0.0, 0.0), None, &none));
    add(put(&mut tx, &ar, page, "Quad arrow", (col(7), y), (36.0, 36.0), None, &|tx, s| {
        set_ctl(tx, s, "Row_1", Some(27.0), Some(25.0));
        set_ctl(tx, s, "Row_2", Some(20.5), None);
    }));
    let ly = lab(1);
    for (c, t) in [(0, "Einfacher Pfeil / Simple arrow"), (1, "Variante 46 × 16"), (2, "Gebogener Pfeil / Bent arrow"), (3, "Variante 44 × 36"), (4, "180-Grad-Pfeil / U-turn arrow"), (5, "Variante 36 × 40"), (6, "Pfeil in vier Richtungen / Quad"), (7, "Variante 36 × 36")] {
        label(&mut tx, col(c), ly, 48.0, t);
    }
    // ---- row 2: Standardflussdiagramm-Shapes, then a small flow with lines glued
    title(&mut tx, 2, "Standardflussdiagramm-Shapes (flowchart, ISO 5807)");
    let y = at(2);
    for (c, nu, t) in [(0, "Process", "Prozess"), (1, "Decision", "Entscheidung"), (2, "Document", "Dokument"), (3, "Database", "Datenbank")] {
        add(put(&mut tx, &fc, page, nu, (col(c), y), (0.0, 0.0), Some(t), &none));
    }
    let flow: Vec<Option<ShapeRef>> = [(4, "Process", "Daten lesen"), (5, "Decision", "gültig?"), (6, "Document", "Bericht"), (7, "Database", "Archiv")]
        .iter()
        .map(|&(c, nu, t)| put(&mut tx, &fc, page, nu, (col(c), y), (0.0, 0.0), Some(t), &none))
        .collect();
    let ly = lab(2);
    for (c, t) in [(0, "Prozess / Process"), (1, "Entscheidung / Decision"), (2, "Dokument / Document"), (3, "Datenbank / Database")] {
        label(&mut tx, col(c), ly, 48.0, t);
    }
    label(&mut tx, (col(4) + col(7)) * 0.5, ly, 190.0, "kleiner Ablauf, verbunden mit „Linie mit Pfeil“ (geklebt an die Verbindungspunkte)");
    // ---- row 3: Verbinder, each glued between two boxes
    title(&mut tx, 3, "Verbinder (connectors, extras) – je zwischen zwei Prozess-Kästen geklebt");
    let y = at(3);
    let boxes: Vec<(Option<ShapeRef>, Option<ShapeRef>)> = [((col(0) - 6.0, y + 9.0), (col(1) + 6.0, y - 9.0)), ((col(2) - 8.0, y + 10.0), (col(3) + 10.0, y - 12.0)), ((col(4) - 10.0, y), (col(5) + 10.0, y)), ((col(6) - 8.0, y - 9.0), (col(7) + 8.0, y + 9.0))]
        .iter()
        .map(|&(a, b)| (put(&mut tx, &fc, page, "Process", a, (22.0, 13.0), Some("A"), &none), put(&mut tx, &fc, page, "Process", b, (22.0, 13.0), Some("B"), &none)))
        .collect();
    let conns: Vec<Option<ShapeRef>> = ["Curved connector", "Elbow connector", "Hollow connector", "Line with arrow"].iter().enumerate().map(|(k, nu)| put_1d(&mut tx, &cn, page, nu, (col(2 * k), y), (col(2 * k + 1), y))).collect();
    let ly = lab(3);
    for (k, t) in ["Kurvenverbinder / Curved connector", "Winkelverbinder / Elbow connector", "Hohler Verbinder / Hollow connector", "Linie mit Pfeil / Line with arrow"].iter().enumerate() {
        label(&mut tx, (col(2 * k) + col(2 * k + 1)) * 0.5, ly, 98.0, t);
    }
    // ---- row 4: Tipp (hidden master of callouts)
    title(&mut tx, 4, "Tipp (callouts, verborgen) – für die Startdiagramme der Vorlagen");
    let y = at(4);
    add(put(&mut tx, &co, page, "Tip", (col(0) + 25.0, y), (0.0, 0.0), Some("Tipp: Ziehen Sie Shapes aus der Seitenleiste auf die Seite. Verbinder kleben an den blauen Verbindungspunkten."), &none));
    add(put(&mut tx, &co, page, "Tip", (col(3) + 5.0, y + 4.0), (110.0, 22.0), Some("Breiter Tipp mit zwei Zeilen: der Text steht links oben, der Balken links hat die Linienfarbe des Designs."), &none));
    label(&mut tx, col(0) + 25.0, lab(4), 98.0, "Tipp / Tip, 70 × 30 mm");
    label(&mut tx, col(3) + 5.0, lab(4), 110.0, "Tipp 110 × 22 mm");
    // values first, then the glue (it reads the connection points on the page)
    let all: Vec<SheetRef> = tx.doc().all_shapes(Owner::Page(page)).map(SheetRef::Shape).collect();
    let _ = eng.recalc(&mut tx, &all);
    let point = |d: &Document, s: ShapeRef, row: u32| glue::connection_points(d, s).into_iter().find(|p| p.row == row).map(|p| p.at);
    // (connector, from, its row, to, its row)
    let mut glued: Vec<(Option<ShapeRef>, Option<ShapeRef>, u32, Option<ShapeRef>, u32)> = Vec::new();
    for (k, &(a, b)) in boxes.iter().enumerate() {
        let to_row = if k == 1 { 2 } else { 3 };
        glued.push((conns[k], a, 1, b, to_row));
    }
    let lines: Vec<Option<ShapeRef>> = (0..3).map(|k| put_1d(&mut tx, &cn, page, "Line with arrow", (col(4 + k), y), (col(5 + k), y))).collect();
    let _ = eng.recalc(&mut tx, &lines.iter().flatten().map(|&s| SheetRef::Shape(s)).collect::<Vec<_>>());
    for k in 0..3 {
        glued.push((lines[k], flow[k], 1, flow[k + 1], 3));
    }
    for (c, a, ra, b, rb) in glued {
        let (Some(c), Some(a), Some(b)) = (c, a, b) else { continue };
        let (Some(pa), Some(pb)) = (point(tx.doc(), a, ra), point(tx.doc(), b, rb)) else { continue };
        let r = glue::glue(&mut tx, &mut eng, c, End::Begin, pa, Target::Point { shape: a, row: ra }).and_then(|_| glue::glue(&mut tx, &mut eng, c, End::End, pb, Target::Point { shape: b, row: rb }));
        if let Err(e) = r {
            out(&format!("note: glue failed: {e:?}\n"));
        }
    }
    marked.extend(conns.iter().flatten());
    let _ = tx.commit();
    // the markers: control points (yellow diamonds) and connection points (blue dots)
    {
        let all: Vec<SheetRef> = doc.all_shapes(Owner::Page(page)).map(SheetRef::Shape).collect();
        let mut tx = doc.transaction("recalc");
        let _ = Engine::new().recalc(&mut tx, &all);
        let _ = tx.commit();
    }
    let mut marks: Vec<([f64; 2], bool)> = Vec::new();
    for &s in &marked {
        marks.extend(ops::controls(&doc, s).into_iter().map(|c| (c.at, true)));
        if !glue::is_one_d(&doc, s) {
            marks.extend(glue::connection_points(&doc, s).into_iter().map(|p| (p.at, false)));
        }
    }
    let mut tx = doc.transaction("markers");
    for (p, ctl) in marks {
        let (x, y) = (p[0] / MM, p[1] / MM);
        if ctl {
            put(&mut tx, &st, page, "Diamond", (x, y), (2.6, 2.6), None, &|tx, s| paint(tx, s, "#FFD21F", Some("#6B5600")));
        } else {
            put(&mut tx, &st, page, "Ellipse", (x, y), (1.6, 1.6), None, &|tx, s| paint(tx, s, "#2F7FD8", None));
        }
    }
    let _ = tx.commit();
    let mut ok = render_page_at(&mut doc, page, &format!("{dir}/probeblatt.png"), 150.0);
    // each row once more, enlarged
    for r in 0..5 {
        let top = band(r) + 2.0;
        ok &= render_region(&mut doc, page, &format!("{dir}/probeblatt-zeile{}.png", r + 1), 300.0, [0.0, top - 60.0, PW, top]);
    }
    // the same sheet in two other designs: line, fill and text come from the styles
    for (id, variant, file) in [(catalog::CONTOUR, 3, "probeblatt-kontur-abgerundet.png"), (catalog::CONTRAST, 0, "probeblatt-kontrast.png")] {
        let Some(d) = catalog::all().into_iter().find(|d| d.id == id) else { continue };
        let mut other = doc.clone();
        let mut tx = other.transaction("design");
        let mut eng = Engine::new();
        if design_apply::apply(&mut tx, &mut eng, d.id, &d.theme_xml(), Scope::All, variant).is_ok() {
            let _ = tx.commit();
            ok &= render_page_at(&mut other, page, &format!("{dir}/{file}"), 110.0);
        }
    }
    if ok { 0 } else { 1 }
}

// a part of the page (mm, y up: x0, y0, x1, y1) as PNG at `dpi`; the shapes are calculated
fn render_region(doc: &mut Document, page: PageRef, path: &str, dpi: f64, r: [f64; 4]) -> bool {
    let mut fonts = crate::render::fonts();
    let mut v = PageView::new(doc, page, &mut fonts, &Options::default());
    let ro = raster::RasterOptions { dpi, region: Some(r.map(|x| x * MM)), ..Default::default() };
    let Some(img) = raster::render(&mut v, &ro) else { return false };
    let ok = plat::write_file(path, &crate::render::png(&img));
    out(&format!("{} {path} ({} x {})\n", if ok { "wrote" } else { "error: cannot write" }, img.w, img.h));
    ok
}
