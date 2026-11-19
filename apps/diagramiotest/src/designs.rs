// designs -- (JD11) the own designs on one sample drawing: a flowchart with connectors in a
// container, a title and a note, built through diagram_core's ops and glue the way
// JFDiagrammer builds it, the design applied with design::apply
//
//   sample(design, variant, title)  the drawing (A4 landscape); the process steps use quick
//                                   style 1, start and end quick style 3, the decision colour
//                                   and style 2, the container style 4
//   probe(dir)                      --design-probe DIR: designs.png (every design and variant
//                                   in a grid), design-<name>.png (one design, 2 x 2) and
//                                   JD11-<Name>.vsdx (variant 1) for the Visio counter-check

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::design::apply::{self, Scope};
use diagram_core::design::{catalog, Design};
use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::{glue, CellAddr, DocKind, Document, EditError, Owner, RowKey, SecKey, ShapeRef, SheetRef, Text, Transaction, Value};
use diagram_io::vsdx::{self, Measure, WriteOptions};
use diagram_view::PageView;
use gfx::Surface;

use crate::out;

/// the names of the variants as the probe labels them
pub(crate) fn variant_names(d: &Design) -> [&'static str; 4] {
    match d.id {
        catalog::NETLAB => ["Signalblau", "Petrol", "Signalblau kräftig", "Graphit"],
        catalog::CONTOUR => ["Anthrazit", "Signalblau", "Ziegel", "Abgerundet"],
        _ => ["Signalblau", "Petrol", "Ziegel", "Anthrazit gerundet"],
    }
}

/// the German name of a design
pub(crate) fn design_name(d: &Design) -> &'static str {
    match d.id {
        catalog::NETLAB => "NetLab",
        catalog::CONTOUR => "Kontur",
        _ => "Kontrast",
    }
}

fn e(e: EditError) -> String {
    format!("{e:?}")
}

// the QuickStyle cells: colour `c` (100 ..), style `m` (100 ..)
fn quick(tx: &mut Transaction, s: ShapeRef, c: f64, m: f64) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(s);
    for (n, v) in [
        ("QuickStyleLineColor", c),
        ("QuickStyleFillColor", c),
        ("QuickStyleShadowColor", c),
        ("QuickStyleFontColor", c),
        ("QuickStyleLineMatrix", m),
        ("QuickStyleFillMatrix", m),
        ("QuickStyleEffectsMatrix", m),
        ("QuickStyleFontMatrix", m),
    ] {
        let a = tx.single(n);
        tx.set_constant(sheet, &a, Value::Num(v), None)?;
    }
    Ok(())
}

// the text size of a shape's first character row, in points
fn text_size(tx: &mut Transaction, s: ShapeRef, pt: f64) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(s);
    let ch = tx.intern("Character");
    let sk = SecKey::one(ch);
    // a Character row inherited from the style is made the shape's own by setting a cell
    let inherited = tx.doc().has_section(sheet, sk) || tx.doc().sheet(sheet).is_some_and(|sh| sh.sections.iter().any(|x| x.key().name == ch));
    if !inherited {
        tx.add_section(sheet, "Character")?;
        tx.add_row(sheet, sk, Some(RowKey::Index(0)), None)?;
    }
    let size = tx.intern("Size");
    tx.set_constant(sheet, &CellAddr::row(sk, RowKey::Index(0), size), Value::Num(pt / 72.0), Some(diagram_core::Unit::Points))
}

fn labelled(tx: &mut Transaction, eng: &mut Engine, owner: Owner, kind: Basic, a: [f64; 2], b: [f64; 2], text: &str) -> Result<ShapeRef, EditError> {
    let s = ops::new_shape(tx, eng, owner, kind, a, b)?;
    ops::set_text(tx, eng, s, Text::plain(&format!("{text}\n")))?;
    Ok(s)
}

// a rectangle made a diamond (its relative geometry rows moved to the edges' middles)
fn diamond(tx: &mut Transaction, eng: &mut Engine, owner: Owner, a: [f64; 2], b: [f64; 2], text: &str) -> Result<ShapeRef, EditError> {
    let s = labelled(tx, eng, owner, Basic::Rectangle, a, b, text)?;
    let sheet = SheetRef::Shape(s);
    let gk = SecKey { name: tx.intern("Geometry"), index: 0 };
    let (x, y) = (tx.intern("X"), tx.intern("Y"));
    for (i, (px, py)) in [(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5), (0.5, 0.0)].into_iter().enumerate() {
        let rk = RowKey::Index(i as u32 + 1);
        tx.set_constant(sheet, &CellAddr::row(gk, rk, x), Value::Num(px), None)?;
        tx.set_constant(sheet, &CellAddr::row(gk, rk, y), Value::Num(py), None)?;
    }
    Ok(s)
}

fn center(doc: &Document, s: ShapeRef) -> [f64; 2] {
    let n = |c: &str| doc.sym(c).and_then(|c| doc.resolve(SheetRef::Shape(s), &CellAddr::single(c)).num()).unwrap_or(0.0);
    [n("PinX"), n("PinY")]
}

fn connect(tx: &mut Transaction, eng: &mut Engine, owner: Owner, a: ShapeRef, b: ShapeRef, label: &str) -> Result<ShapeRef, EditError> {
    let (pa, pb) = (center(tx.doc(), a), center(tx.doc(), b));
    let c = glue::new_connector(tx, eng, owner, (pa, glue::Target::Shape(a)), (pb, glue::Target::Shape(b)))?;
    if !label.is_empty() {
        ops::set_text(tx, eng, c, Text::plain(&format!("{label}\n")))?;
    }
    Ok(c)
}

/// the sample drawing in a design and variant (see the module comment)
pub(crate) fn sample(design: &Design, variant: u32, title: &str) -> Result<Document, String> {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().ok_or("no page")?;
    let owner = Owner::Page(page);
    let mut eng = Engine::new();
    let mut tx = doc.transaction("Design");
    apply::apply(&mut tx, &mut eng, design.id, &design.theme_xml(), Scope::Page(page), variant).map_err(e)?;
    let _ = tx.commit();
    let mut tx = doc.transaction("Zeichnung");
    let t = labelled(&mut tx, &mut eng, owner, Basic::TextBox, [0.6, 7.2], [9.0, 7.8], title).map_err(e)?;
    text_size(&mut tx, t, 20.0).map_err(e)?;
    let box_ = labelled(&mut tx, &mut eng, owner, Basic::Rectangle, [0.6, 0.6], [11.1, 6.8], "Bestellung").map_err(e)?;
    quick(&mut tx, box_, 100.0, 103.0).map_err(e)?;
    text_size(&mut tx, box_, 13.0).map_err(e)?;
    let va = tx.single("VerticalAlign");
    tx.set_constant(SheetRef::Shape(box_), &va, Value::Num(0.0), None).map_err(e)?;
    let note = labelled(&mut tx, &mut eng, owner, Basic::TextBox, [8.2, 5.4], [10.9, 6.2], "Lieferung binnen zwei Werktagen").map_err(e)?;
    text_size(&mut tx, note, 11.0).map_err(e)?;
    let start = labelled(&mut tx, &mut eng, owner, Basic::Ellipse, [1.0, 3.3], [2.4, 4.3], "Start").map_err(e)?;
    quick(&mut tx, start, 100.0, 102.0).map_err(e)?;
    let check = labelled(&mut tx, &mut eng, owner, Basic::Rectangle, [3.0, 3.2], [4.9, 4.4], "Auftrag prüfen").map_err(e)?;
    let dec = diamond(&mut tx, &mut eng, owner, [5.5, 2.85], [7.6, 4.75], "Vollständig?").map_err(e)?;
    quick(&mut tx, dec, 101.0, 101.0).map_err(e)?;
    let ask = labelled(&mut tx, &mut eng, owner, Basic::Rectangle, [5.6, 1.0], [7.5, 2.0], "Nachfragen").map_err(e)?;
    let ship = labelled(&mut tx, &mut eng, owner, Basic::Rectangle, [8.4, 3.2], [10.4, 4.4], "Ausliefern").map_err(e)?;
    let end = labelled(&mut tx, &mut eng, owner, Basic::Ellipse, [8.7, 1.0], [10.1, 2.0], "Ende").map_err(e)?;
    quick(&mut tx, end, 100.0, 102.0).map_err(e)?;
    for (a, b, l) in [(start, check, ""), (check, dec, ""), (dec, ship, "ja"), (dec, ask, "nein"), (ask, check, ""), (ship, end, "")] {
        connect(&mut tx, &mut eng, owner, a, b, l).map_err(e)?;
    }
    let _ = tx.commit();
    // everything computed, as the app does before saving
    let res = eng.calculate(&doc);
    let mut tx = doc.transaction("Rechnen");
    res.write(&mut tx).map_err(e)?;
    let _ = tx.commit();
    Ok(doc)
}

/// a drawing's page as an image `w` pixels wide
pub(crate) fn render(doc: &Document, w: u32) -> Option<Surface> {
    let page = doc.pages().next()?;
    let mut fonts = crate::render::fonts();
    let mut v = PageView::new(doc, page, &mut fonts, &diagram_core::render::Options::default());
    let d = v.drawing();
    let h = (w as f64 * d.height / d.width.max(1e-6) + 0.5) as u32;
    Some(v.render(w, h))
}

// tiles in a grid of `cols` columns on the window grey, with a gap -> (XRGB bytes, w, h)
fn grid(tiles: &[Surface], cols: usize, gap: usize) -> (Vec<u8>, usize, usize) {
    let (tw, th) = (tiles[0].w as usize, tiles[0].h as usize);
    let rows = tiles.len().div_ceil(cols);
    let (w, h) = (cols * tw + (cols + 1) * gap, rows * th + (rows + 1) * gap);
    let mut px = Vec::with_capacity(w * h * 4);
    for _ in 0..w * h {
        px.extend_from_slice(&[0xF1, 0xEE, 0xEC, 0xFF]);
    }
    for (i, t) in tiles.iter().enumerate() {
        let (x0, y0) = (gap + (i % cols) * (tw + gap), gap + (i / cols) * (th + gap));
        let n = tw.min(t.w as usize) * 4;
        for y in 0..th.min(t.h as usize) {
            let src = &t.px[y * t.stride as usize..y * t.stride as usize + n];
            let d = ((y0 + y) * w + x0) * 4;
            px[d..d + n].copy_from_slice(src);
        }
    }
    (px, w, h)
}

/// --design-probe DIR
pub(crate) fn probe(dir: &str) -> i32 {
    let mut all = Vec::new();
    for d in catalog::all() {
        let names = variant_names(&d);
        let mut tiles = Vec::new();
        for v in 0..4u32 {
            let title = format!("{} · Variante {} · {}", design_name(&d), v + 1, names[v as usize]);
            let doc = match sample(&d, v, &title) {
                Ok(doc) => doc,
                Err(err) => {
                    out(&format!("{}: {err}\n", d.name));
                    return 1;
                }
            };
            if v == 0 {
                let path = format!("{dir}/JD11-{}.vsdx", design_name(&d));
                match vsdx::write(&doc, &WriteOptions::DEFAULT) {
                    Ok(bytes) if plat::write_file(&path, &bytes) => out(&format!("wrote {path}\n")),
                    r => {
                        out(&format!("cannot write {path}: {:?}\n", r.err()));
                        return 1;
                    }
                }
            }
            let Some(big) = render(&doc, 1100) else { return 1 };
            let Some(small) = render(&doc, 760) else { return 1 };
            tiles.push(big);
            all.push(small);
        }
        let (px, w, h) = grid(&tiles, 2, 24);
        let path = format!("{dir}/design-{}.png", design_name(&d).to_lowercase());
        if !plat::write_file(&path, &img_png::encode(&px, w, h, w * 4, false)) {
            return 1;
        }
        out(&format!("wrote {path}\n"));
    }
    let (px, w, h) = grid(&all, 4, 20);
    let path = format!("{dir}/designs.png");
    if !plat::write_file(&path, &img_png::encode(&px, w, h, w * 4, false)) {
        return 1;
    }
    out(&format!("wrote {path}\n"));
    0
}

// ---------------------------------------------------------------- group "designs"

const GOLDEN_W: u32 = 800;
const INDEX: [&str; 5] = ["ThemeIndex", "ColorSchemeIndex", "EffectSchemeIndex", "ConnectorSchemeIndex", "FontSchemeIndex"];

fn read_back(doc: &Document) -> Option<Document> {
    let bytes = vsdx::write(doc, &WriteOptions::DEFAULT).ok()?;
    vsdx::read(&bytes, &vsdx::Limits::VSDX).ok().map(|l| l.doc)
}

fn page_num(doc: &Document, name: &str) -> Option<f64> {
    let page = doc.pages().next()?;
    let c = doc.sym(name)?;
    doc.resolve(SheetRef::Page(page), &CellAddr::single(c)).num()
}

fn formula_text(doc: &Document, s: ShapeRef, cell: &str) -> String {
    let Some(c) = doc.sym(cell) else { return String::new() };
    match doc.sheet(SheetRef::Shape(s)).and_then(|sh| sh.cells.iter().find(|x| x.name == c)).map(|x| x.formula) {
        Some(diagram_core::Formula::Expr(f)) => String::from(doc.str(f)),
        Some(_) => String::from("(value)"),
        None => String::from("(inherited)"),
    }
}

// the theme value the engine gives a cell of a shape
fn themed_color(doc: &Document, s: ShapeRef, sec: Option<&str>, cell: &str) -> Option<u32> {
    use diagram_core::formula::{Color, Outcome, Val};
    let mut eng = Engine::new();
    let c = doc.sym(cell)?;
    let a = match sec {
        Some(n) => CellAddr::row(SecKey::one(doc.sym(n)?), RowKey::Index(0), c),
        None => CellAddr::single(c),
    };
    let mut v = match eng.evaluate(doc, SheetRef::Shape(s), &a) {
        Outcome::Value(v) => v,
        _ => return None,
    };
    while let Val::Themed(inner) = v {
        v = (*inner).clone();
    }
    match v {
        Val::Color(Color::Rgb(r, g, b)) => Some(((r as u32) << 16) | ((g as u32) << 8) | b as u32),
        _ => None,
    }
}

// the theme value the engine gives a single numeric cell of a shape
fn themed_num(doc: &Document, s: ShapeRef, cell: &str) -> Option<f64> {
    use diagram_core::formula::{Outcome, Val};
    let mut eng = Engine::new();
    let c = doc.sym(cell)?;
    let mut v = match eng.evaluate(doc, SheetRef::Shape(s), &CellAddr::single(c)) {
        Outcome::Value(v) => v,
        _ => return None,
    };
    while let Val::Themed(inner) = v {
        v = (*inner).clone();
    }
    match v {
        Val::Num(x) => Some(x),
        _ => None,
    }
}

/// group "designs": the own designs' parts, applying with undo, foreign parts, the golden images
pub(crate) fn check(h: &mut crate::H) {
    let golden = crate::render::find_up("apps/diagramiotest/tests/golden");
    for d in catalog::all() {
        let name = design_name(&d).to_lowercase();
        let part = d.theme_xml();
        h.check(&format!("designs/{name}/part"), apply::part_id(&part) == Some(d.id) && d.id >= diagram_core::design::FIRST_ID, format!("id {}", d.id));
        let Ok(doc) = sample(&d, 0, &format!("{} · Variante 1", design_name(&d))) else {
            h.check(&format!("designs/{name}/sample"), false, String::from("the sample builds"));
            continue;
        };
        // the part and the page's cells survive writing and reading
        let back = read_back(&doc);
        let ok = back.as_ref().is_some_and(|b| b.themes.iter().any(|t| t.data == part) && INDEX.iter().all(|n| page_num(b, n) == Some(d.id as f64)));
        h.check(&format!("designs/{name}/roundtrip"), ok, format!("{} parts", back.map_or(0, |b| b.themes.len())));
        // the golden image of variant 1
        let Some(img) = render(&doc, GOLDEN_W) else { continue };
        let Some(dir) = &golden else { continue };
        let path = format!("{dir}/design-{name}.png");
        let want = plat::read_file(&path).and_then(|b| img_png::decode_pargb(&b));
        let what = match want.as_ref().and_then(|g| crate::render::diff(&img, g)) {
            Some((far, max)) => {
                let limit = (img.w as f64 * img.h as f64 * crate::render::PER_MILLE / 1000.0) as u64;
                (far <= limit, format!("{far} pixels off by more than {} (at most {limit}), largest {max}", crate::render::FAR))
            }
            None => (false, format!("{path}: missing or another size (write it with --write-design-golden)")),
        };
        h.check(&format!("designs/{name}/golden"), what.0, what.1);
    }
    behaviour(h);
}

// applying: undo, foreign parts kept, mixed scheme numbers, THEMEGUARD and locks, text boxes
fn behaviour(h: &mut crate::H) {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let Some(page) = doc.pages().next() else { return };
    let owner = Owner::Page(page);
    let mut eng = Engine::new();
    let netlab = catalog::netlab();
    let contrast = catalog::contrast();
    h.check("designs/new-document", apply::page_design(&doc, page) == Some((netlab.id, 0)), format!("{:?}", apply::page_design(&doc, page)));
    // a foreign part (another number, as a design read from a Visio file) with mixed numbers:
    // the colour scheme 41, everything else 37
    let own = String::from_utf8_lossy(&netlab.theme_xml()).into_owned();
    let foreign = own
        .replace(&format!("schemeEnum=\"{}\"", netlab.id), "schemeEnum=\"37\"")
        .replacen("schemeEnum=\"37\"", "schemeEnum=\"41\"", 1)
        .replace("name=\"NetLab\"", "name=\"Fremd\"");
    let mut tx = doc.transaction("fremd");
    let fp = tx.intern("/visio/theme/theme9.xml");
    let mut themes = tx.doc().themes.clone();
    themes.push(diagram_core::Theme { part: fp, data: foreign.clone().into_bytes() });
    let _ = tx.set_themes(themes);
    let _ = tx.commit();
    let foreign_id = apply::part_id(foreign.as_bytes());
    // shapes: plain red line (a design resets it), THEMEGUARD red (kept), locked colours (kept),
    // a text box
    let mut tx = doc.transaction("Shapes");
    let mut made = Vec::new();
    for (k, f) in ["RGB(255,0,0)", "THEMEGUARD(RGB(255,0,0))", "RGB(255,0,0)"].iter().enumerate() {
        let x = 1.0 + k as f64 * 2.0;
        if let Ok(s) = ops::new_shape(&mut tx, &mut eng, owner, Basic::Rectangle, [x, 1.0], [x + 1.5, 2.0]) {
            let a = tx.single("LineColor");
            let _ = tx.set_formula(SheetRef::Shape(s), &a, f);
            if k == 2 {
                let l = tx.single("LockThemeColors");
                let _ = tx.set_constant(SheetRef::Shape(s), &l, Value::Num(1.0), None);
            }
            made.push(s);
        }
    }
    let tb = labelled(&mut tx, &mut eng, owner, Basic::TextBox, [1.0, 4.0], [4.0, 4.5], "Titel").ok();
    let conn = if made.len() == 3 { connect(&mut tx, &mut eng, owner, made[0], made[1], "").ok() } else { None };
    let _ = tx.commit();
    let (Some(tb), true) = (tb, made.len() == 3) else {
        h.check("designs/apply/shapes", false, String::from("shapes made"));
        return;
    };
    let before = doc.themes.clone();
    let mut tx = doc.transaction("Kontrast");
    let r = apply::apply(&mut tx, &mut eng, contrast.id, &contrast.theme_xml(), Scope::All, 0);
    let change = tx.commit();
    h.check("designs/apply/ok", r.is_ok(), format!("{r:?}"));
    h.check("designs/apply/page", apply::page_design(&doc, page) == Some((contrast.id, 0)), format!("{:?}", apply::page_design(&doc, page)));
    let kept_foreign = doc.themes.iter().any(|t| t.data == foreign.as_bytes());
    let has_netlab = apply::find_part(&doc, netlab.id).is_some();
    h.check(
        "designs/apply/parts",
        kept_foreign && !has_netlab && apply::find_part(&doc, contrast.id).is_some(),
        format!("foreign kept {kept_foreign}, NetLab dropped {}", !has_netlab),
    );
    let f = [formula_text(&doc, made[0], "LineColor"), formula_text(&doc, made[1], "LineColor"), formula_text(&doc, made[2], "LineColor")];
    h.check("designs/apply/guard-and-locks", f[0] == "(inherited)" && f[1] == "THEMEGUARD(RGB(255,0,0))" && f[2] == "RGB(255,0,0)", format!("{f:?}"));
    // the text box keeps dark text on Contrast's solid style (font style 1, Text Only)
    let tbc = themed_color(&doc, tb, Some("Character"), "Color");
    h.check("designs/apply/text-box-dark", tbc == Some(0x1D2228), format!("{tbc:?}"));
    // a box shows Contrast's solid fill; FillColor2 is the fill colour slice itself
    let fill = themed_color(&doc, made[0], None, "FillForegnd");
    let fill2 = themed_color(&doc, made[0], None, "FillBkgnd");
    h.check("designs/apply/solid-fill", fill == Some(contrast.variants[0].colors[0]) && fill2 == fill, format!("{fill:?} {fill2:?}"));
    // the connector's line ends come from Visio's list entry 1 (entry 0 stands for matrix 0):
    // Contrast's connector style 1 has end 4, size 2, no begin; its cap is flat (1)
    let ends = conn.map(|c| (themed_num(&doc, c, "BeginArrow"), themed_num(&doc, c, "EndArrow"), themed_num(&doc, c, "EndArrowSize"), themed_num(&doc, c, "LineCap")));
    h.check("designs/apply/connector-ends", ends == Some((Some(0.0), Some(4.0), Some(2.0), Some(1.0))), format!("{ends:?}"));
    // a variant, then undo of both restores the parts and the cells
    let mut tx = doc.transaction("Variante");
    let r = apply::set_variant(&mut tx, &mut eng, Scope::Page(page), 2);
    let change2 = tx.commit();
    h.check("designs/variant", r.is_ok() && apply::page_design(&doc, page) == Some((contrast.id, 2)), format!("{:?}", apply::page_design(&doc, page)));
    let undone = change2.is_some_and(|mut c| c.undo(&mut doc)) && change.is_some_and(|mut c| c.undo(&mut doc));
    h.check(
        "designs/undo",
        undone && doc.themes == before && apply::page_design(&doc, page) == Some((netlab.id, 0)) && formula_text(&doc, made[0], "LineColor") == "RGB(255,0,0)",
        format!("{:?}", apply::page_design(&doc, page)),
    );
    // the foreign part applied: each component by its own number
    let Some(fid) = foreign_id else {
        h.check("designs/foreign/id", false, String::from("the foreign part has a number"));
        return;
    };
    let mut tx = doc.transaction("Fremd");
    let r = apply::apply(&mut tx, &mut eng, fid, foreign.as_bytes(), Scope::All, 1);
    let _ = tx.commit();
    let nums: Vec<Option<f64>> = INDEX.iter().map(|n| page_num(&doc, n)).collect();
    h.check("designs/foreign/mixed-numbers", r.is_ok() && nums == [Some(37.0), Some(41.0), Some(37.0), Some(37.0), Some(37.0)], format!("{nums:?}"));
    let back = read_back(&doc);
    h.check("designs/foreign/kept", back.is_some_and(|b| b.themes.iter().any(|t| t.data == foreign.as_bytes())), String::from("written back unchanged"));
    let mut tx = doc.transaction("Kein Design");
    let r = apply::remove(&mut tx, &mut eng, Scope::All);
    let _ = tx.commit();
    h.check(
        "designs/none",
        r.is_ok() && apply::page_design(&doc, page).is_none() && doc.themes.iter().any(|t| t.data == foreign.as_bytes()),
        format!("{:?}", apply::page_design(&doc, page)),
    );
}

/// --write-design-golden DIR
pub(crate) fn write_golden(dir: &str) -> i32 {
    for d in catalog::all() {
        let name = design_name(&d).to_lowercase();
        let Ok(doc) = sample(&d, 0, &format!("{} · Variante 1", design_name(&d))) else { return 1 };
        let Some(img) = render(&doc, GOLDEN_W) else { return 1 };
        let path = format!("{dir}/design-{name}.png");
        if !plat::write_file(&path, &crate::render::png(&img)) {
            return 1;
        }
        out(&format!("wrote {path}\n"));
    }
    0
}
