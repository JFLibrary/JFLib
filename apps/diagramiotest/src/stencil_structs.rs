// stencil_structs -- (JD10) the structure stencils of apps/JFDiagrammer/stencils: containers and
// lists, the cross-functional flowchart, the callouts (diagram_io::stencil, own sources only)
//
//   stencils/structure/<set>/<measure>   both measures build, write -> read -> write is the same
//                       (roundtrip); the masters with NameU, msvStructureType and Hidden (the CFF
//                       Container and the two lists are hidden); the engine keeps nothing
//   stencils/structure/container   all 6 container styles x 4 heading styles dropped and
//                       recalculated: JFD_HeadingEdge/Size from msvSDHeadingStyle, the text hidden
//                       without a heading, the heading band and the corner brackets per style
//   stencils/structure/heading-off  JFD_HeadingSize = 0 on the list, the lanes, the separators
//                       and the CFF Container: no band, no text; lanes turn their label with
//                       JFD_LabelUpright
//   stencils/structure/callouts    the pointer shows with the tip outside, every pointer section
//                       is hidden with the tip inside the body
//
//   diagramiotest --write-stencil-previews DIR   containers.png, cff.png, callouts.png: every
//                       master (also the hidden ones) in several sizes and with all styles,
//                       heading styles and pointer positions, for a look at them

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use diagram_core::formula::{Engine, Outcome};
use diagram_core::instance::{self, MasterTexts};
use diagram_core::render::Options;
use diagram_core::{attr, CellAddr, DocKind, Document, PageRef, RowKey, SecKey, ShapeRef, SheetRef, Text, Transaction, Unit, Value};
use diagram_io::vsdx::{self, Measure};
use diagram_view::export::raster;
use diagram_view::PageView;

use crate::stencils::{build_set, cell, master_shape, named, read_back, sym_text};
use crate::{out, H};

const CONTAINERS: &[(&str, &str)] = &[
    ("stencil.txt", include_str!("../../JFDiagrammer/stencils/containers/stencil.txt")),
    ("container.master", include_str!("../../JFDiagrammer/stencils/containers/container.master")),
    ("list.master", include_str!("../../JFDiagrammer/stencils/containers/list.master")),
    ("list_item.master", include_str!("../../JFDiagrammer/stencils/containers/list_item.master")),
];

const CFF: &[(&str, &str)] = &[
    ("stencil.txt", include_str!("../../JFDiagrammer/stencils/cff/stencil.txt")),
    ("swimlane.master", include_str!("../../JFDiagrammer/stencils/cff/swimlane.master")),
    ("separator.master", include_str!("../../JFDiagrammer/stencils/cff/separator.master")),
    ("swimlane_vertical.master", include_str!("../../JFDiagrammer/stencils/cff/swimlane_vertical.master")),
    ("separator_vertical.master", include_str!("../../JFDiagrammer/stencils/cff/separator_vertical.master")),
    ("cff_container.master", include_str!("../../JFDiagrammer/stencils/cff/cff_container.master")),
    ("swimlane_list.master", include_str!("../../JFDiagrammer/stencils/cff/swimlane_list.master")),
    ("phase_list.master", include_str!("../../JFDiagrammer/stencils/cff/phase_list.master")),
];

pub(crate) const CALLOUTS: &[(&str, &str)] = &[
    ("stencil.txt", include_str!("../../JFDiagrammer/stencils/callouts/stencil.txt")),
    ("callout_box.master", include_str!("../../JFDiagrammer/stencils/callouts/callout_box.master")),
    ("callout_rounded.master", include_str!("../../JFDiagrammer/stencils/callouts/callout_rounded.master")),
    ("callout_bubble.master", include_str!("../../JFDiagrammer/stencils/callouts/callout_bubble.master")),
    ("callout_line.master", include_str!("../../JFDiagrammer/stencils/callouts/callout_line.master")),
    ("callout_bracket.master", include_str!("../../JFDiagrammer/stencils/callouts/callout_bracket.master")),
    ("callout_circle.master", include_str!("../../JFDiagrammer/stencils/callouts/callout_circle.master")),
    ("tip.master", include_str!("../../JFDiagrammer/stencils/callouts/tip.master")),
];

type Want = (&'static str, &'static str, bool);

/// per stencil: its folder, its sources, its masters (NameU, msvStructureType, hidden)
const SETS: [(&str, &[(&str, &str)], &[Want]); 3] = [
    ("containers", CONTAINERS, &[("Container", "Container", false), ("List", "List", false), ("List item", "", false)]),
    (
        "cff",
        CFF,
        &[
            ("Swimlane", "Container", false),
            ("Separator", "Container", false),
            ("Swimlane (vertical)", "Container", false),
            ("Separator (vertical)", "Container", false),
            ("CFF Container", "Container", true),
            ("Swimlane List", "List", true),
            ("Phase List", "List", true),
        ],
    ),
    (
        "callouts",
        CALLOUTS,
        &[
            ("Callout box", "Callout", false),
            ("Callout rounded", "Callout", false),
            ("Callout bubble", "Callout", false),
            ("Callout line", "Callout", false),
            ("Callout bracket", "Callout", false),
            ("Callout circle", "Callout", false),
            // (JD13) the hidden note box of the templates' start drawings: no callout
            ("Tip", "", true),
        ],
    ),
];

const MM: f64 = 1.0 / 25.4;

// ---------------------------------------------------------------- reading and setting cells

fn user(doc: &Document, s: SheetRef, n: &str) -> Option<Value> {
    cell(doc, s, Some(("User", 0)), named(doc, n), "Value").map(|c| c.value)
}

fn num(doc: &Document, s: SheetRef, sec: Option<(&str, u32)>, row: Option<RowKey>, n: &str) -> f64 {
    match cell(doc, s, sec, row, n).map(|c| c.value) {
        Some(Value::Num(x)) => x,
        _ => f64::NAN,
    }
}

fn user_num(doc: &Document, s: SheetRef, n: &str) -> f64 {
    match user(doc, s, n) {
        Some(Value::Num(x)) => x,
        _ => f64::NAN,
    }
}

fn user_text(doc: &Document, s: SheetRef, n: &str) -> String {
    user(doc, s, n).map(|v| sym_text(doc, v)).unwrap_or_default()
}

// a geometry section's own cell (NoShow ...) as a flag
fn geo_flag(doc: &Document, s: SheetRef, ix: u32, n: &str) -> bool {
    num(doc, s, Some(("Geometry", ix)), None, n) != 0.0
}

pub(crate) fn set_user(tx: &mut Transaction, s: ShapeRef, n: &str, v: f64, u: Option<Unit>) {
    let _ = tx.set_user(SheetRef::Shape(s), n, Value::Num(v), u);
}

pub(crate) fn set_single(tx: &mut Transaction, s: SheetRef, n: &str, v: f64, u: Option<Unit>) {
    let a = CellAddr::single(tx.intern(n));
    let _ = tx.set_constant(s, &a, Value::Num(v), u);
}

// the tip of a callout (Controls.Association) at (x, y) inches in the shape
fn set_tip(tx: &mut Transaction, s: ShapeRef, x: f64, y: f64) {
    let sk = SecKey::one(tx.intern("Control"));
    let rk = RowKey::Named(tx.intern("Association"));
    for (c, v) in [("X", x), ("Y", y)] {
        let a = CellAddr::row(sk, rk, tx.intern(c));
        let _ = tx.set_constant(SheetRef::Shape(s), &a, Value::Num(v), Some(Unit::Millimeters));
    }
}

/// a drawing with one instance of each master of `src` (French names) -> the shapes by NameU
fn drop_all(src: &Document) -> Result<(Document, PageRef, Vec<(String, ShapeRef)>), String> {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().ok_or("no page")?;
    let fr = instance::lang_index("fr").unwrap_or(4);
    let mut tx = doc.transaction("drop");
    let mut out = Vec::new();
    for (i, m) in src.masters().enumerate() {
        let nu = MasterTexts::of(src, m).name_u;
        let local = instance::import_master(&mut tx, src, m).map_err(|e| format!("{nu}: {e:?}"))?;
        let s = instance::place(&mut tx, page, local, 2.0 + 3.0 * i as f64, 3.0, fr).map_err(|e| format!("{nu}: {e:?}"))?;
        out.push((nu, s));
    }
    let sheets: Vec<SheetRef> = out.iter().map(|(_, s)| SheetRef::Shape(*s)).collect();
    Engine::new().recalc(&mut tx, &sheets).map_err(|e| format!("recalc: {e:?}"))?;
    let _ = tx.commit();
    Ok((doc, page, out))
}

// change cells of shape `s`, recalculate it, and count the formula cells of the whole drawing that
// the engine keeps or that come out as an error
fn edit(doc: &mut Document, s: ShapeRef, f: impl FnOnce(&mut Transaction)) -> usize {
    let mut tx = doc.transaction("variant");
    f(&mut tx);
    let kept = Engine::new().recalc(&mut tx, &[SheetRef::Shape(s)]).map_or(usize::MAX, |r| r.kept.len());
    let _ = tx.commit();
    let res = Engine::new().calculate(doc);
    kept + res.cells.iter().filter(|(_, _, o)| matches!(o, Outcome::Kept(_)) || matches!(o, Outcome::Value(v) if v.is_err())).count() + res.cycles.len()
}

// ---------------------------------------------------------------- the checks

/// the group "stencils/structure" (called by stencils::run)
pub fn run(h: &mut H) {
    for (name, set, want) in SETS {
        for m in [Measure::Metric, Measure::Us] {
            let tag = format!("stencils/structure/{name}/{}", if m == Measure::Us { "us" } else { "metric" });
            let bytes = match build_set(set, m) {
                Ok((_, b)) => b,
                Err(e) => {
                    h.check(&tag, false, e.to_string());
                    continue;
                }
            };
            h.check(&tag, true, format!("{} bytes", bytes.len()));
            let Some(doc) = read_back(h, &format!("{tag}/roundtrip"), &bytes) else { continue };
            let hidden = doc.sym("Hidden");
            let got: Vec<(String, String, bool)> = doc
                .masters()
                .map(|mr| {
                    let kind = master_shape(&doc, mr).map(|s| user_text(&doc, s, "msvStructureType")).unwrap_or_default();
                    let hid = doc.master(mr).and_then(|x| hidden.and_then(|s| attr(&x.header.attrs, s))).is_some_and(|v| doc.str(v) == "1");
                    (MasterTexts::of(&doc, mr).name_u, kind, hid)
                })
                .collect();
            let same = got.len() == want.len() && got.iter().zip(want.iter()).all(|(g, w)| g.0 == w.0 && g.1 == w.1 && g.2 == w.2);
            h.check(&format!("{tag}/masters"), same, format!("{got:?}"));
            let mut d = doc.clone();
            let res = Engine::new().calculate(&d);
            let bad: Vec<String> = res
                .cells
                .iter()
                .filter(|(_, _, o)| matches!(o, Outcome::Kept(_)) || matches!(o, Outcome::Value(v) if v.is_err()))
                .map(|(_, a, o)| format!("{} {o:?}", d.str(a.cell)))
                .collect();
            let mut tx = d.transaction("check");
            let written = res.write(&mut tx).unwrap_or(usize::MAX);
            drop(tx);
            h.check(&format!("{tag}/engine"), bad.is_empty() && written == 0 && res.cycles.is_empty(), format!("{} formula cells, {written} changed, kept {bad:?}", res.cells.len()));
            if m == Measure::Metric {
                match drop_all(&doc) {
                    Ok((dd, _, shapes)) => match name {
                        "containers" => containers(h, dd, &shapes),
                        "cff" => cff(h, dd, &shapes),
                        _ => callouts(h, dd, &shapes),
                    },
                    Err(e) => h.check(&format!("{tag}/drop"), false, e),
                }
            }
        }
    }
}

fn shape_of(shapes: &[(String, ShapeRef)], nu: &str) -> Option<ShapeRef> {
    shapes.iter().find(|(n, _)| n == nu).map(|(_, s)| *s)
}

fn containers(h: &mut H, mut doc: Document, shapes: &[(String, ShapeRef)]) {
    let Some(c) = shape_of(shapes, "Container") else { return h.check("stencils/structure/container", false, String::from("no Container")) };
    let sc = SheetRef::Shape(c);
    let mut fails: Vec<String> = Vec::new();
    for style in 1..=6 {
        for head in 1..=4 {
            let bad = edit(&mut doc, c, |tx| {
                set_user(tx, c, "msvSDContainerStyle", style as f64, None);
                set_user(tx, c, "msvSDHeadingStyle", head as f64, None);
            });
            let edge = user_text(&doc, sc, "JFD_HeadingEdge");
            let size = user_num(&doc, sc, "JFD_HeadingSize") / MM;
            let want_edge = ["Top", "Top", "Left", "None"][head - 1];
            let want_size = if head == 4 { 0.0 } else { 8.0 };
            let hide = num(&doc, sc, None, None, "HideText") != 0.0;
            let band_shown = !geo_flag(&doc, sc, 1, "NoShow");
            let want_band = head != 2 && head != 4 && [1, 2, 5].contains(&style);
            let corners = !geo_flag(&doc, sc, 3, "NoShow");
            let angle = num(&doc, sc, None, None, "TxtAngle");
            let want_angle = if head == 3 { core::f64::consts::FRAC_PI_2 } else { 0.0 };
            if bad != 0 || edge != want_edge || (size - want_size).abs() > 1e-9 || hide != (head == 4) || band_shown != want_band || corners != (style == 5) || (angle - want_angle).abs() > 1e-9 {
                fails.push(format!("style {style} heading {head}: {bad} errors, {edge} {size} mm, hide {hide}, band {band_shown}, corners {corners}, angle {angle}"));
            }
        }
    }
    h.check("stencils/structure/container/styles", fails.is_empty(), if fails.is_empty() { String::from("6 styles x 4 heading styles") } else { fails.join("; ") });
    // the list: no heading with JFD_HeadingSize = 0
    if let Some(l) = shape_of(shapes, "List") {
        let bad = edit(&mut doc, l, |tx| set_user(tx, l, "JFD_HeadingSize", 0.0, Some(Unit::Millimeters)));
        let sl = SheetRef::Shape(l);
        let (hide, band) = (num(&doc, sl, None, None, "HideText") != 0.0, !geo_flag(&doc, sl, 1, "NoShow"));
        h.check("stencils/structure/heading-off/List", bad == 0 && hide && !band, format!("{bad} errors, HideText {hide}, band {band}, {}", user_text(&doc, sl, "msvSDListItemMaster")));
    }
}

fn cff(h: &mut H, mut doc: Document, shapes: &[(String, ShapeRef)]) {
    for nu in ["Swimlane", "Separator", "Swimlane (vertical)", "Separator (vertical)", "CFF Container"] {
        let Some(s) = shape_of(shapes, nu) else { continue };
        let ss = SheetRef::Shape(s);
        let before = (num(&doc, ss, None, None, "HideText") != 0.0, !geo_flag(&doc, ss, 0, "NoShow"));
        let bad = edit(&mut doc, s, |tx| set_user(tx, s, "JFD_HeadingSize", 0.0, Some(Unit::Millimeters)));
        let after = (num(&doc, ss, None, None, "HideText") != 0.0, !geo_flag(&doc, ss, 0, "NoShow"));
        h.check(&format!("stencils/structure/heading-off/{nu}"), bad == 0 && before == (false, true) && after == (true, false), format!("{bad} errors, (HideText, band) {before:?} -> {after:?}"));
    }
    // the lane's label: turned by 90 degrees, upright with JFD_LabelUpright
    if let Some(s) = shape_of(shapes, "Swimlane") {
        let ss = SheetRef::Shape(s);
        let bad = edit(&mut doc, s, |tx| set_user(tx, s, "JFD_HeadingSize", 10.0 * MM, Some(Unit::Millimeters)));
        let turned = (num(&doc, ss, None, None, "TxtAngle"), num(&doc, ss, None, None, "TxtWidth") / MM);
        let bad2 = edit(&mut doc, s, |tx| set_user(tx, s, "JFD_LabelUpright", 1.0, Some(Unit::Bool)));
        let upright = (num(&doc, ss, None, None, "TxtAngle"), num(&doc, ss, None, None, "TxtWidth") / MM);
        let ok = bad + bad2 == 0
            && (turned.0 - core::f64::consts::FRAC_PI_2).abs() < 1e-9
            && (turned.1 - 40.0).abs() < 1e-6
            && upright.0.abs() < 1e-9
            && (upright.1 - 10.0).abs() < 1e-6;
        h.check("stencils/structure/cff/label-upright", ok, format!("(TxtAngle, TxtWidth mm) {turned:?} -> {upright:?}, heading text {:?}", user_text(&doc, ss, "visHeadingText")));
    }
}

fn callouts(h: &mut H, mut doc: Document, shapes: &[(String, ShapeRef)]) {
    for (nu, s) in shapes {
        let s = *s;
        let ss = SheetRef::Shape(s);
        // (JD13) the Tip has no pointer
        if user_text(&doc, ss, "msvStructureType") != "Callout" {
            continue;
        }
        let geo = doc.geometry(ss);
        // the sections with a NoShow formula: the pointer and its dot
        let has_noshow = |d: &Document, g: SecKey| d.sym("NoShow").is_some_and(|n| d.resolve(ss, &CellAddr::section(g, n)).exists());
        let pointer: Vec<SecKey> = geo.iter().copied().filter(|&g| has_noshow(&doc, g)).collect();
        let shown = |d: &Document| pointer.iter().filter(|g| !geo_flag(d, ss, g.index, "NoShow")).count();
        let out_count = shown(&doc);
        let (w, hh) = (num(&doc, ss, None, None, "Width"), num(&doc, ss, None, None, "Height"));
        let bad = edit(&mut doc, s, |tx| set_tip(tx, s, w * 0.5, hh * 0.5));
        let in_count = shown(&doc);
        let side = user_num(&doc, ss, "Side");
        let wedge = nu == "Callout rounded" || nu == "Callout bubble";
        let ok = bad == 0 && if wedge { side == 0.0 } else { !pointer.is_empty() && out_count == pointer.len() && in_count == 0 };
        h.check(
            &format!("stencils/structure/callouts/{nu}"),
            ok,
            format!("{bad} errors, pointer sections {} shown outside {out_count}, inside {in_count}, wedge side inside {side}", pointer.len()),
        );
    }
}

// ---------------------------------------------------------------- previews

// a drawing page of w x h mm for the previews
pub(crate) fn sheet_doc(w: f64, hh: f64) -> Option<(Document, PageRef)> {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next()?;
    let mut tx = doc.transaction("page");
    set_single(&mut tx, SheetRef::Page(page), "PageWidth", w * MM, Some(Unit::Millimeters));
    set_single(&mut tx, SheetRef::Page(page), "PageHeight", hh * MM, Some(Unit::Millimeters));
    let _ = tx.commit();
    Some((doc, page))
}

// one instance of master NameU `nu` of `src` with its centre at (x, y) mm, w x h mm (0: the
// master's size), the text `text` (None: the master's)
#[allow(clippy::too_many_arguments)]
pub(crate) fn put(tx: &mut Transaction, src: &Document, page: PageRef, nu: &str, at: (f64, f64), size: (f64, f64), text: Option<&str>, more: &dyn Fn(&mut Transaction, ShapeRef)) -> Option<ShapeRef> {
    let m = src.masters().find(|&m| MasterTexts::of(src, m).name_u == nu)?;
    let local = instance::import_master(tx, src, m).ok()?;
    let s = instance::place(tx, page, local, at.0 * MM, at.1 * MM, 0).ok()?;
    if size.0 > 0.0 {
        set_single(tx, SheetRef::Shape(s), "Width", size.0 * MM, Some(Unit::Millimeters));
        set_single(tx, SheetRef::Shape(s), "Height", size.1 * MM, Some(Unit::Millimeters));
    }
    if let Some(t) = text {
        let _ = tx.set_text(s, Some(Text::plain(t)));
    }
    more(tx, s);
    Some(s)
}

pub(crate) fn none(_: &mut Transaction, _: ShapeRef) {}

fn render_page(doc: &mut Document, page: PageRef, path: &str) -> bool {
    render_page_at(doc, page, path, 110.0)
}

/// recalculate the page's shapes and write the page as PNG at `dpi`
pub(crate) fn render_page_at(doc: &mut Document, page: PageRef, path: &str, dpi: f64) -> bool {
    {
        let mut tx = doc.transaction("recalc");
        let sheets: Vec<SheetRef> = tx.doc().all_shapes(diagram_core::Owner::Page(page)).map(SheetRef::Shape).collect();
        let _ = Engine::new().recalc(&mut tx, &sheets);
        let _ = tx.commit();
    }
    let mut fonts = crate::render::fonts();
    let mut v = PageView::new(doc, page, &mut fonts, &Options::default());
    let ro = raster::RasterOptions { dpi, ..Default::default() };
    let Some(img) = raster::render(&mut v, &ro) else { return false };
    let ok = plat::write_file(path, &crate::render::png(&img));
    out(&format!("{} {path} ({} x {})\n", if ok { "wrote" } else { "error: cannot write" }, img.w, img.h));
    ok
}

fn built(set: &[(&str, &str)]) -> Option<Document> {
    build_set(set, Measure::Metric).ok().map(|(b, _)| b.doc)
}

/// diagramiotest --write-stencil-previews DIR
pub fn previews(dir: &str) -> i32 {
    let (Some(cs), Some(cf), Some(co)) = (built(CONTAINERS), built(CFF), built(CALLOUTS)) else {
        out("error: a structure stencil does not build (diagramiotest --check --only stencils/structure)\n");
        return 1;
    };
    let mut ok = true;
    // containers: 6 styles (columns) x 4 heading styles (rows), then sizes, then lists
    {
        let Some((mut doc, page)) = sheet_doc(420.0, 470.0) else { return 1 };
        let mut tx = doc.transaction("previews");
        for style in 1..=6 {
            for head in 1..=4 {
                let x = 10.0 + 35.0 + (style - 1) as f64 * 68.0;
                let y = 470.0 - 10.0 - 25.0 - (head - 1) as f64 * 58.0;
                let label = format!("Stil {style} / Kopf {head}");
                put(&mut tx, &cs, page, "Container", (x, y), (62.0, 48.0), Some(&label), &|tx, s| {
                    set_user(tx, s, "msvSDContainerStyle", style as f64, None);
                    set_user(tx, s, "msvSDHeadingStyle", head as f64, None);
                });
            }
        }
        // sizes: wide and flat, narrow and tall with the band on the left, small, large
        let sizes: [(f64, f64, f64, f64, u32, u32); 5] =
            [(85.0, 210.0, 150.0, 26.0, 1, 1), (190.0, 180.0, 24.0, 90.0, 2, 3), (225.0, 205.0, 22.0, 18.0, 6, 1), (310.0, 200.0, 140.0, 60.0, 5, 1), (85.0, 165.0, 150.0, 40.0, 3, 3)];
        for (x, y, w, hh, style, head) in sizes {
            put(&mut tx, &cs, page, "Container", (x, y), (w, hh), Some("Größe"), &|tx, s| {
                set_user(tx, s, "msvSDContainerStyle", style as f64, None);
                set_user(tx, s, "msvSDHeadingStyle", head as f64, None);
            });
        }
        // a list with three items stacked under its heading, a list without heading, an item
        let (lx, ly, lw, lh) = (45.0, 95.0, 60.0, 8.0 + 2.0 + 3.0 * 8.0 + 2.0);
        put(&mut tx, &cs, page, "List", (lx, ly), (lw, lh), None, &none);
        for i in 0..3 {
            let iy = ly + lh * 0.5 - 8.0 - 2.0 - 4.0 - 8.0 * i as f64;
            let t = format!("Element {}", i + 1);
            put(&mut tx, &cs, page, "List item", (lx, iy), (56.0, 8.0), Some(&t), &none);
        }
        put(&mut tx, &cs, page, "List", (130.0, 95.0), (60.0, 30.0), Some("ohne Kopf"), &|tx, s| set_user(tx, s, "JFD_HeadingSize", 0.0, Some(Unit::Millimeters)));
        put(&mut tx, &cs, page, "List", (215.0, 95.0), (90.0, 30.0), Some("breite Liste"), &none);
        put(&mut tx, &cs, page, "List item", (45.0, 40.0), (0.0, 0.0), None, &none);
        let _ = tx.commit();
        ok &= render_page(&mut doc, page, &format!("{dir}/containers.png"));
    }
    // the cross-functional flowchart: a horizontal and a vertical one built by hand, single
    // lanes and separators in other sizes, upright labels, no heading
    {
        let Some((mut doc, page)) = sheet_doc(460.0, 440.0) else { return 1 };
        let mut tx = doc.transaction("previews");
        // horizontal: CFF 210 x 106 (title 10), phase strip 8, two lanes of 44
        let (x0, top) = (10.0, 430.0);
        put(&mut tx, &cf, page, "CFF Container", (x0 + 105.0, top - 53.0), (210.0, 106.0), Some("Bestellung"), &none);
        put(&mut tx, &cf, page, "Phase List", (x0 + 110.0, top - 58.0), (200.0, 96.0), None, &none);
        put(&mut tx, &cf, page, "Swimlane List", (x0 + 105.0, top - 62.0), (210.0, 88.0), None, &none);
        put(&mut tx, &cf, page, "Separator", (x0 + 10.0 + 50.0, top - 10.0 - 48.0), (100.0, 96.0), Some("Planung"), &none);
        put(&mut tx, &cf, page, "Separator", (x0 + 10.0 + 150.0, top - 10.0 - 48.0), (100.0, 96.0), Some("Umsetzung"), &none);
        put(&mut tx, &cf, page, "Swimlane", (x0 + 105.0, top - 18.0 - 22.0), (210.0, 44.0), Some("Vertrieb"), &none);
        put(&mut tx, &cf, page, "Swimlane", (x0 + 105.0, top - 18.0 - 66.0), (210.0, 44.0), Some("Lager"), &none);
        // vertical: CFF 116 x 160 (title 10), phase column 8, lanes 54 wide with a 10 mm head
        let (vx, vtop) = (250.0, 430.0);
        put(&mut tx, &cf, page, "CFF Container", (vx + 58.0, vtop - 80.0), (116.0, 160.0), Some("Vertikal"), &none);
        put(&mut tx, &cf, page, "Separator (vertical)", (vx + 58.0, vtop - 20.0 - 35.0), (116.0, 70.0), Some("Phase 1"), &none);
        put(&mut tx, &cf, page, "Separator (vertical)", (vx + 58.0, vtop - 20.0 - 105.0), (116.0, 70.0), Some("Phase 2"), &none);
        put(&mut tx, &cf, page, "Swimlane (vertical)", (vx + 8.0 + 27.0, vtop - 10.0 - 75.0), (54.0, 150.0), Some("Team A"), &none);
        put(&mut tx, &cf, page, "Swimlane (vertical)", (vx + 8.0 + 81.0, vtop - 10.0 - 75.0), (54.0, 150.0), Some("Team B"), &none);
        // single shapes: lane small, upright label, no heading; separator alone
        put(&mut tx, &cf, page, "Swimlane", (x0 + 70.0, 240.0), (140.0, 30.0), Some("Upright"), &|tx, s| set_user(tx, s, "JFD_LabelUpright", 1.0, Some(Unit::Bool)));
        put(&mut tx, &cf, page, "Swimlane", (x0 + 70.0, 200.0), (140.0, 30.0), Some("ohne Kopf"), &|tx, s| set_user(tx, s, "JFD_HeadingSize", 0.0, Some(Unit::Millimeters)));
        put(&mut tx, &cf, page, "Swimlane", (x0 + 70.0, 160.0), (0.0, 0.0), None, &none);
        put(&mut tx, &cf, page, "Separator", (x0 + 220.0, 200.0), (60.0, 80.0), None, &none);
        put(&mut tx, &cf, page, "Separator (vertical)", (x0 + 320.0, 200.0), (80.0, 60.0), Some("Upright"), &|tx, s| set_user(tx, s, "JFD_LabelUpright", 1.0, Some(Unit::Bool)));
        put(&mut tx, &cf, page, "Swimlane (vertical)", (x0 + 400.0, 160.0), (40.0, 120.0), None, &none);
        put(&mut tx, &cf, page, "CFF Container", (x0 + 110.0, 70.0), (200.0, 60.0), None, &none);
        put(&mut tx, &cf, page, "CFF Container", (x0 + 330.0, 70.0), (180.0, 60.0), Some("ohne Titel"), &|tx, s| set_user(tx, s, "JFD_HeadingSize", 0.0, Some(Unit::Millimeters)));
        let _ = tx.commit();
        ok &= render_page(&mut doc, page, &format!("{dir}/cff.png"));
    }
    // callouts: one row per master, the tip in five places, then a wider one
    {
        let names = ["Callout box", "Callout rounded", "Callout bubble", "Callout line", "Callout bracket", "Callout circle"];
        let Some((mut doc, page)) = sheet_doc(520.0, 6.0 * 55.0 + 20.0) else { return 1 };
        let mut tx = doc.transaction("previews");
        for (row, nu) in names.iter().enumerate() {
            let y = 6.0 * 55.0 + 20.0 - 40.0 - row as f64 * 55.0;
            let (w, hh) = if *nu == "Callout circle" { (20.0, 20.0) } else { (40.0, 16.0) };
            // tip relative to the body's lower left corner (mm); None: the master's
            let tips: [Option<(f64, f64)>; 6] = [None, Some((w + 18.0, hh + 12.0)), Some((w * 0.5, hh * 0.5)), Some((-30.0, hh * 0.5)), Some((w * 0.6, -18.0)), Some((w + 25.0, -6.0))];
            for (col, tip) in tips.iter().enumerate() {
                let x = 45.0 + col as f64 * 80.0;
                let size = if col == 5 { (w * 1.6, hh * 1.4) } else { (0.0, 0.0) };
                let t = format!("{} {}", &nu[8..], col + 1);
                let tip = *tip;
                put(&mut tx, &co, page, nu, (x, y), size, Some(&t), &move |tx, s| {
                    if let Some((tx_, ty)) = tip {
                        set_tip(tx, s, tx_ * MM, ty * MM);
                    }
                });
            }
        }
        let _ = tx.commit();
        ok &= render_page(&mut doc, page, &format!("{dir}/callouts.png"));
    }
    if ok { 0 } else { 1 }
}
