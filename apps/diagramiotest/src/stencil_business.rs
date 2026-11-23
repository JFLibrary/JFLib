// stencil_business -- (JD26) the general and business-graphics stencils of apps/JFDiagrammer/stencils:
// decorative, charting (SH-003, SH-004), venn (SH-007), marketing, matrix, pyramid, cycle (SH-030 to
// SH-033) and cause_effect (SH-041); own sources only
//
//   stencils/business/<folder>/<measure>   both measures build; write -> read -> write is the
//                       same; names, QuickInfos and keywords in five languages, the category;
//                       the engine keeps nothing
//   stencils/business/drop   every master dropped in French and recalculated: no error values
//   stencils/business/parts   the parts of a group master (SWOT) show their texts in the
//                       language of the drop; the master's parts carry five texts, guarded
//   stencils/business/counts   levels, steps and phases from the shape data and from a control
//                       point (SETATREF): the parts shown, the group growing
//   stencils/business/values   the pie slice's angle from its control point, the pie chart
//                       slice's start, share and colour from its shape data
//   stencils/business/fishbone   a category glued to the effect's spine and a cause glued to the
//                       category follow when the effect moves
//
//   diagramiotest --write-stencil-previews DIR   also probeblatt-jd26.png: the sample masters of
//                       the eight stencils in their own size and in a variant (a moved control
//                       point, other shape data, the parts in English), the fishbone glued
//                       together, control points as yellow diamonds, connection points as blue
//                       dots; probeblatt-jd26-zeile1.png .. -zeile5.png each row enlarged;
//                       probeblatt-jd26-kontur.png, -kontrast.png the sheet in two other designs
//   diagramiotest --write-jd26-gegenprobe DIR   the files of the Visio counter-check (gegenprobe())

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::design::apply::{self as design_apply, Scope};
use diagram_core::design::catalog;
use diagram_core::formula::{Engine, Input};
use diagram_core::glue::{self, End, Target};
use diagram_core::instance::{self, MasterTexts, StencilTexts};
use diagram_core::render::Options;
use diagram_core::{ops, CellAddr, Document, Owner, PageRef, RowKey, SecKey, ShapeRef, SheetRef, Text, Transaction, Unit, Value};
use diagram_io::stencil;
use diagram_io::vsdx::{self, Measure};
use diagram_view::export::raster;
use diagram_view::PageView;

use crate::stencil_structs::{none, put, render_page_at, set_single, sheet_doc};
use crate::templates::files_of;
use crate::{out, H};

const MM: f64 = 1.0 / 25.4;

/// the stencils of JD26 (folder names)
pub(crate) const FOLDERS: [&str; 8] = ["decorative", "charting", "venn", "marketing", "matrix", "pyramid", "cycle", "cause_effect"];

/// stencil `folder` built (metric)
pub(crate) fn built(folder: &'static str) -> Result<Document, String> {
    stencil::build(&files_of("stencils", folder), Measure::Metric).map(|b| b.doc).map_err(|e| format!("stencils/{folder}/{e}"))
}

// an instance of NameU `nu` in language `lang` with its centre at (x, y) mm
fn put_lang(tx: &mut Transaction, src: &Document, page: PageRef, nu: &str, at: (f64, f64), lang: usize) -> Option<ShapeRef> {
    let m = src.masters().find(|&m| MasterTexts::of(src, m).name_u == nu)?;
    let local = instance::import_master(tx, src, m).ok()?;
    instance::place(tx, page, local, at.0 * MM, at.1 * MM, lang).ok()
}

/// a shape data value (Prop.<row>.Value) of an instance
pub(crate) fn set_prop(tx: &mut Transaction, s: ShapeRef, row: &str, v: f64) {
    let a = CellAddr::row(SecKey::one(tx.intern("Property")), RowKey::Named(tx.intern(row)), tx.intern("Value"));
    let _ = tx.set_constant(SheetRef::Shape(s), &a, Value::Num(v), None);
}

/// a control point's X and Y (mm, local) on an instance
pub(crate) fn set_ctl(tx: &mut Transaction, s: ShapeRef, row: &str, x: Option<f64>, y: Option<f64>) {
    let sk = SecKey::one(tx.intern("Control"));
    let rk = RowKey::Named(tx.intern(row));
    for (c, v) in [("X", x), ("Y", y)] {
        if let Some(v) = v {
            let a = CellAddr::row(sk, rk, tx.intern(c));
            let _ = tx.set_constant(SheetRef::Shape(s), &a, Value::Num(v * MM), Some(Unit::Millimeters));
        }
    }
}

/// a 1-D instance of NameU `nu` with its ends at b and e (mm)
pub(crate) fn put_1d(tx: &mut Transaction, src: &Document, page: PageRef, nu: &str, b: (f64, f64), e: (f64, f64)) -> Option<ShapeRef> {
    crate::stencil_shapes::put_1d(tx, src, page, nu, b, e)
}

// the connection point `row` of shape s on the page (inches)
fn point(doc: &Document, s: ShapeRef, row: u32) -> Option<[f64; 2]> {
    glue::connection_points(doc, s).into_iter().find(|p| p.row == row).map(|p| p.at)
}

// glue end `end` of 1-D shape c to connection point `row` of t
pub(crate) fn glue_to(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef, end: End, t: ShapeRef, row: u32) -> bool {
    let Some(p) = point(tx.doc(), t, row) else { return false };
    glue::glue(tx, eng, c, end, p, Target::Point { shape: t, row }).is_ok()
}

fn recalc_page(tx: &mut Transaction, eng: &mut Engine, page: PageRef) {
    let all: Vec<SheetRef> = tx.doc().all_shapes(Owner::Page(page)).map(SheetRef::Shape).collect();
    let _ = eng.recalc(tx, &all);
}

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

fn centred(tx: &mut Transaction, s: ShapeRef) {
    let sk = SecKey::one(tx.intern("Paragraph"));
    let a = CellAddr::row(sk, RowKey::Index(0), tx.intern("HorzAlign"));
    let _ = tx.set_constant(SheetRef::Shape(s), &a, Value::Num(1.0), None);
}

// the probe sheet's page and rows (mm)
const PW: f64 = 420.0;
const PH: f64 = 410.0;
const ROWS: [f64; 5] = [66.0, 82.0, 76.0, 70.0, 96.0];

fn row_top(r: usize) -> f64 {
    PH - 12.0 - ROWS[..r].iter().sum::<f64>()
}

// every JD26 stencil built (metric), in the order of FOLDERS
fn all_built() -> Option<Vec<Document>> {
    let mut docs: Vec<Document> = Vec::new();
    for f in FOLDERS {
        match built(f) {
            Ok(d) => docs.push(d),
            Err(e) => {
                out(&format!("error: {e}\n"));
                return None;
            }
        }
    }
    Some(docs)
}

/// diagramiotest --write-stencil-previews DIR: probeblatt-jd26.png and its parts
pub fn previews(dir: &str) -> i32 {
    let Ok(st) = built("standard") else { return 1 };
    let Some((mut doc, page, marked)) = probe_doc() else { return 1 };
    // the markers
    {
        let all: Vec<SheetRef> = doc.all_shapes(Owner::Page(page)).map(SheetRef::Shape).collect();
        let mut tx = doc.transaction("recalc");
        let _ = Engine::new().recalc(&mut tx, &all);
        let _ = tx.commit();
    }
    let mut marks: Vec<([f64; 2], bool)> = Vec::new();
    for &s in &marked {
        marks.extend(ops::controls(&doc, s).into_iter().map(|c| (c.at, true)));
        marks.extend(glue::connection_points(&doc, s).into_iter().map(|p| (p.at, false)));
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
    let mut ok = render_page_at(&mut doc, page, &format!("{dir}/probeblatt-jd26.png"), 150.0);
    for r in 0..5 {
        let t = row_top(r) + 1.0;
        ok &= render_region(&mut doc, page, &format!("{dir}/probeblatt-jd26-zeile{}.png", r + 1), 250.0, [0.0, t - ROWS[r] - 1.0, PW, t]);
    }
    for (id, variant, file) in [(catalog::CONTOUR, 0, "probeblatt-jd26-kontur.png"), (catalog::CONTRAST, 0, "probeblatt-jd26-kontrast.png")] {
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

// the probe sheet as a drawing (without the markers): the page and the shapes to mark
fn probe_doc() -> Option<(Document, PageRef, Vec<ShapeRef>)> {
    let docs = all_built()?;
    let [dec, cha, ven, mkt, mat, pyr, cyc, cae] = [&docs[0], &docs[1], &docs[2], &docs[3], &docs[4], &docs[5], &docs[6], &docs[7]];
    let st = built("standard").ok()?;
    let (mut doc, page) = sheet_doc(PW, PH)?;
    // five rows under the title, each with its title at the top and labels at the bottom
    let top = row_top;
    let mid = |r: usize| top(r) - 6.0 - (ROWS[r] - 16.0) * 0.5;
    let lab = |r: usize| top(r) - ROWS[r] + 5.0;
    let mut marked: Vec<ShapeRef> = Vec::new();
    let mut tx = doc.transaction("probe");
    let mut eng = Engine::new();
    let label = |tx: &mut Transaction, x: f64, y: f64, w: f64, t: &str| {
        put(tx, &st, page, "Text box", (x, y), (w, 8.0), Some(t), &centred);
    };
    let title = |tx: &mut Transaction, r: usize, t: &str| {
        put(tx, &st, page, "Text box", (10.0 + 130.0, top(r) - 2.0), (260.0, 6.0), Some(t), &none);
    };
    put(&mut tx, &st, page, "Text box", (10.0 + 200.0, PH - 5.0), (400.0, 8.0), Some("JD26 Probeblatt: Allgemein und Geschäftsgrafiken im NetLab-Stil (Design NetLab) – gelbe Raute = Steuerpunkt, blauer Punkt = Verbindungspunkt"), &none);
    let mut add = |s: Option<ShapeRef>| marked.extend(s);

    // ---- row 1: Dekorative Shapes, Diagramm- und Mathematik-Shapes
    title(&mut tx, 0, "Dekorative Shapes (decorative, general) · Diagramm- und Mathematik-Shapes (charting, general)");
    let y = mid(0);
    add(put(&mut tx, dec, page, "Smiley face", (25.0, y), (0.0, 0.0), None, &none));
    add(put(&mut tx, dec, page, "Smiley face", (60.0, y), (0.0, 0.0), None, &|tx, s| set_ctl(tx, s, "Row_1", None, Some(10.8))));
    add(put(&mut tx, dec, page, "Smiley face", (95.0, y), (0.0, 0.0), None, &|tx, s| set_ctl(tx, s, "Row_1", None, Some(14.0))));
    add(put(&mut tx, dec, page, "Ribbon up", (145.0, y + 8.0), (0.0, 0.0), Some("Überschrift"), &none));
    add(put(&mut tx, dec, page, "Ribbon up", (145.0, y - 16.0), (56.0, 16.0), Some("Variante"), &|tx, s| set_ctl(tx, s, "Row_1", Some(6.0), None)));
    add(put(&mut tx, cha, page, "Pie slice", (200.0, y), (0.0, 0.0), None, &none));
    // the handle at 250° clockwise from the top
    let a = 250.0f64.to_radians();
    add(put(&mut tx, cha, page, "Pie slice", (240.0, y), (0.0, 0.0), Some("70 %"), &move |tx, s| {
        set_ctl(tx, s, "Row_1", Some(15.0 + 15.0 * diagram_core::formula::math::sin(a)), Some(15.0 + 15.0 * diagram_core::formula::math::cos(a)))
    }));
    // a pie of three slices from their shape data
    for (start, share, colour, t) in [(0.0, 50.0, 1.0, "50 %"), (50.0, 30.0, 2.0, "30 %"), (80.0, 20.0, 3.0, "20 %")] {
        add(put(&mut tx, cha, page, "Pie chart slice", (285.0, y), (0.0, 0.0), Some(t), &move |tx, s| {
            set_prop(tx, s, "Start", start);
            set_prop(tx, s, "Share", share);
            set_prop(tx, s, "Color", colour);
        }));
    }
    add(put(&mut tx, cha, page, "Not equal", (330.0, y), (0.0, 0.0), None, &none));
    add(put(&mut tx, cha, page, "Not equal", (365.0, y), (26.0, 26.0), None, &|tx, s| set_ctl(tx, s, "Row_1", Some(23.4), Some(20.0))));
    let ly = lab(0);
    for (x, w, t) in [
        (25.0, 34.0, "lachend"),
        (60.0, 34.0, "neutral"),
        (95.0, 34.0, "traurig"),
        (145.0, 70.0, "Band nach oben; Kerbe 6 mm"),
        (200.0, 34.0, "Kreissegment 90°"),
        (240.0, 40.0, "Steuerpunkt auf 250°"),
        (285.0, 44.0, "3 Segmente (Daten)"),
        (330.0, 30.0, "Ungleich ≠"),
        (365.0, 36.0, "26 mm, dicker"),
    ] {
        label(&mut tx, x, ly, w, t);
    }

    // ---- row 2: Venn, SWOT (parts with their texts, also in English)
    title(&mut tx, 1, "Venn-Diagramm-Shapes (venn, general) · Marketingdiagramme (marketing, business): SWOT");
    let y = mid(1);
    add(put(&mut tx, ven, page, "Venn diagram 3 sets", (40.0, y), (0.0, 0.0), None, &none));
    add(put_lang(&mut tx, ven, page, "Venn diagram 3 sets", (110.0, y), 1));
    add(put(&mut tx, ven, page, "Intersection of 2 sets", (180.0, y), (0.0, 0.0), Some("A und B"), &none));
    add(put(&mut tx, mkt, page, "SWOT", (265.0, y), (0.0, 0.0), None, &none));
    add(put_lang(&mut tx, mkt, page, "SWOT", (355.0, y), 1));
    let ly = lab(1);
    for (x, w, t) in [
        (40.0, 60.0, "Venn-Diagramm aus 3 Mengen"),
        (110.0, 60.0, "dasselbe, abgelegt auf Englisch"),
        (180.0, 60.0, "Schnittmenge von 2 Mengen"),
        (265.0, 80.0, "SWOT (Teile mit Text)"),
        (355.0, 80.0, "SWOT, abgelegt auf Englisch"),
    ] {
        label(&mut tx, x, ly, w, t);
    }

    // ---- row 3: Stufendiagramm, Matrix
    title(&mut tx, 2, "Marketingdiagramme: Stufendiagramm · Matrixdiagramm-Shapes (matrix, business)");
    let y = mid(2);
    add(put(&mut tx, mkt, page, "Step chart", (50.0, y), (0.0, 0.0), None, &none));
    add(put(&mut tx, mkt, page, "Step chart", (150.0, y), (0.0, 0.0), None, &|tx, s| set_prop(tx, s, "Steps", 6.0)));
    add(put(&mut tx, mat, page, "Matrix with labels", (250.0, y), (0.0, 0.0), None, &none));
    add(put(&mut tx, mat, page, "Axis 1", (340.0, y), (0.0, 0.0), None, &none));
    add(put(&mut tx, mat, page, "Axis 1", (385.0, y), (34.0, 40.0), None, &|tx, s| set_ctl(tx, s, "Row_1", Some(28.0), None)));
    let ly = lab(2);
    for (x, w, t) in [
        (50.0, 76.0, "Stufendiagramm, 4 Stufen"),
        (150.0, 100.0, "6 Stufen (Shape-Daten; die Gruppe wächst)"),
        (250.0, 80.0, "Matrix mit Beschriftungen"),
        (340.0, 40.0, "Achse 1"),
        (385.0, 40.0, "größere Spitzen"),
    ] {
        label(&mut tx, x, ly, w, t);
    }

    // ---- row 4: Pyramiden, Zyklus
    title(&mut tx, 3, "Pyramiden-Shapes (pyramid, business) · Zyklusdiagramm-Shapes (cycle, business)");
    let y = mid(3);
    add(put(&mut tx, pyr, page, "Layered pyramid", (45.0, y), (0.0, 0.0), None, &none));
    add(put(&mut tx, pyr, page, "Layered pyramid", (130.0, y), (0.0, 0.0), None, &|tx, s| set_prop(tx, s, "Levels", 3.0)));
    add(put(&mut tx, pyr, page, "Funnel", (195.0, y), (0.0, 0.0), Some("Interessenten"), &none));
    add(put(&mut tx, cyc, page, "Circular motion", (255.0, y), (0.0, 0.0), None, &none));
    add(put(&mut tx, cyc, page, "Circular motion", (315.0, y), (0.0, 0.0), None, &|tx, s| {
        set_prop(tx, s, "Phases", 6.0);
        set_ctl(tx, s, "Row_1", None, Some(37.0));
    }));
    add(put(&mut tx, cyc, page, "Target", (375.0, y), (0.0, 0.0), Some("Ziel"), &none));
    let ly = lab(3);
    for (x, w, t) in [
        (45.0, 82.0, "Pyramide mit Ebenen, 4"),
        (130.0, 82.0, "3 Ebenen (Shape-Daten)"),
        (195.0, 50.0, "Trichter, 3 Stufen"),
        (255.0, 56.0, "Kreisbewegung, 4 Phasen"),
        (315.0, 56.0, "6 Phasen, schmaler Ring"),
        (375.0, 50.0, "Zielscheibe, 4 Ringe"),
    ] {
        label(&mut tx, x, ly, w, t);
    }

    // ---- row 5: Ursache-Wirkung, glued together
    title(&mut tx, 4, "Ursache-Wirkung-Diagramm-Shapes (cause_effect, business): Kategorien kleben an der Hauptgräte, Ursachen an den Kategorien");
    let y = mid(4);
    let effect = put(&mut tx, cae, page, "Effect", (200.0, y), (0.0, 0.0), Some("Lieferung verspätet"), &none);
    add(effect);
    recalc_page(&mut tx, &mut eng, page);
    let mut glued = 0;
    let mut cats: Vec<ShapeRef> = Vec::new();
    if let Some(e) = effect {
        // the categories: their ends on spine points 4, 9 (row 3, 8) above and below
        for (nu, row, bx, by, t) in [
            ("Category 1", 3u32, 95.0, y + 24.0, "Mensch"),
            ("Category 1", 8, 160.0, y + 24.0, "Maschine"),
            ("Category 2", 3, 95.0, y - 24.0, "Methode"),
            ("Category 2", 8, 160.0, y - 24.0, "Material"),
        ] {
            let Some(p) = point(tx.doc(), e, row) else { continue };
            let Some(c) = put_1d(&mut tx, cae, page, nu, (bx, by), (p[0] / MM, p[1] / MM)) else { continue };
            let _ = tx.set_text(c, Some(Text::plain(t)));
            if glue_to(&mut tx, &mut eng, c, End::End, e, row) {
                glued += 1;
            }
            cats.push(c);
            add(Some(c));
        }
        recalc_page(&mut tx, &mut eng, page);
        // a primary cause on each upper category (its point 5, row 4) and one on a lower one
        for (k, t) in [(0usize, "Schulung"), (1, "Wartung"), (2, "Ablauf")] {
            let Some(&c) = cats.get(k) else { continue };
            let Some(p) = point(tx.doc(), c, 4) else { continue };
            let Some(u) = put_1d(&mut tx, cae, page, "Primary cause 1", (p[0] / MM - 30.0, p[1] / MM), (p[0] / MM, p[1] / MM)) else { continue };
            let _ = tx.set_text(u, Some(Text::plain(t)));
            if glue_to(&mut tx, &mut eng, u, End::End, c, 4) {
                glued += 1;
            }
            add(Some(u));
        }
    }
    out(&format!("probeblatt-jd26: {glued} von 7 Enden geklebt\n"));
    label(&mut tx, 200.0, lab(4), 300.0, "Wirkung, je zwei Kategorien oben (Kategorie 1) und unten (Kategorie 2), drei Primäre Ursachen");
    let _ = tx.commit();
    Some((doc, page, marked))
}

// a part of the page (mm, y up: x0, y0, x1, y1) as PNG at `dpi`
fn render_region(doc: &mut Document, page: PageRef, path: &str, dpi: f64, r: [f64; 4]) -> bool {
    let mut fonts = crate::render::fonts();
    let mut v = PageView::new(doc, page, &mut fonts, &Options::default());
    let ro = raster::RasterOptions { dpi, region: Some(r.map(|x| x * MM)), ..Default::default() };
    let Some(img) = raster::render(&mut v, &ro) else { return false };
    let ok = plat::write_file(path, &crate::render::png(&img));
    out(&format!("{} {path} ({} x {})\n", if ok { "wrote" } else { "error: cannot write" }, img.w, img.h));
    ok
}

// ---------------------------------------------------------------- the checks

fn num(doc: &Document, s: SheetRef, n: &str) -> f64 {
    crate::stencils::cell(doc, s, None, None, n).and_then(|c| c.value.num()).unwrap_or(f64::NAN)
}

fn user_num(doc: &Document, s: SheetRef, n: &str) -> f64 {
    crate::stencils::cell(doc, s, Some(("User", 0)), crate::stencils::named(doc, n), "Value").and_then(|c| c.value.num()).unwrap_or(f64::NAN)
}

fn prop_num(doc: &Document, s: SheetRef, n: &str) -> f64 {
    crate::stencils::cell(doc, s, Some(("Property", 0)), crate::stencils::named(doc, n), "Value").and_then(|c| c.value.num()).unwrap_or(f64::NAN)
}

// geometry section `ix` (from 0) of a sheet hidden (NoShow)?
fn no_show(doc: &Document, s: SheetRef, ix: u32) -> bool {
    crate::stencils::cell(doc, s, Some(("Geometry", ix)), None, "NoShow").and_then(|c| c.value.num()).is_some_and(|v| v != 0.0)
}

fn text_of(doc: &Document, s: ShapeRef) -> String {
    doc.shape(s).and_then(|x| x.text()).map(|t| t.chars()).unwrap_or_default()
}

// the text a subshape shows: its own, else its master subshape's
fn shown_text(doc: &Document, s: ShapeRef) -> String {
    let own = text_of(doc, s);
    if !own.is_empty() {
        return own;
    }
    doc.master_shape_of(s).map(|m| text_of(doc, m)).unwrap_or_default()
}

fn children(doc: &Document, s: ShapeRef) -> Vec<ShapeRef> {
    doc.shape(s).map(|x| x.children().collect()).unwrap_or_default()
}

/// the group "stencils/business" (with the group "stencils")
pub fn run(h: &mut H) {
    // every stencil in both measures: built, written and read back unchanged, five languages,
    // the category, the master pages in the measure's unit, nothing the engine keeps
    let mut metric: Vec<Document> = Vec::new();
    for folder in FOLDERS {
        for m in [Measure::Metric, Measure::Us] {
            let tag = format!("stencils/business/{folder}/{}", if m == Measure::Us { "us" } else { "metric" });
            let bytes = match stencil::build_vssx(&files_of("stencils", folder), m) {
                Ok((_, b)) => b,
                Err(e) => {
                    h.check(&tag, false, format!("{e}"));
                    continue;
                }
            };
            let Some(doc) = crate::stencils::read_back(h, &format!("{tag}/roundtrip"), &bytes) else { continue };
            let n = doc.masters().count();
            let five = doc.masters().all(|mr| {
                let t = MasterTexts::of(&doc, mr);
                t.names.iter().chain(t.prompts.iter()).chain(t.keywords.iter()).all(|x| !x.is_empty())
            });
            let st = StencilTexts::of(&doc);
            let cat_ok = st.category == if matches!(folder, "decorative" | "charting" | "venn") { "general" } else { "business" };
            h.check(&format!("{tag}/masters"), n > 0 && five && cat_ok && st.names.iter().all(|x| !x.is_empty()), format!("{n} masters, five languages {five}, category {}", st.category));
            let (cells, written, bad) = crate::stencil_shapes::engine_state(&doc);
            h.check(&format!("{tag}/engine"), bad.is_empty() && written == 0, format!("{cells} formula cells, {written} changed, kept {bad:?}"));
            if m == Measure::Metric {
                metric.push(doc);
            }
        }
    }
    if metric.len() != FOLDERS.len() {
        return;
    }
    drop_all(h, &metric);
    parts(h, &metric);
    counts(h, &metric);
    values(h, &metric);
    fishbone(h, &metric);
}

// every master dropped in French and recalculated: no error values, nothing kept
fn drop_all(h: &mut H, docs: &[Document]) {
    let Some((mut doc, page)) = sheet_doc(400.0, 400.0) else { return };
    let fr = instance::lang_index("fr").unwrap_or(4);
    let mut tx = doc.transaction("drop");
    let mut shapes: Vec<ShapeRef> = Vec::new();
    let mut fail = String::new();
    let mut eng = Engine::new();
    let mut kept = 0usize;
    for (k, src) in docs.iter().enumerate() {
        for (i, m) in src.masters().enumerate() {
            let nu = MasterTexts::of(src, m).name_u;
            match instance::import_master(&mut tx, src, m).and_then(|local| instance::place(&mut tx, page, local, 1.0 + 0.5 * i as f64, 1.0 + 1.5 * k as f64, fr)) {
                Ok(s) => {
                    // one drop at a time, as the app recalculates (Engine::recalc over many sheets
                    // in one call grows quadratically, JD13)
                    let sheets: Vec<SheetRef> = core::iter::once(s).chain(children(tx.doc(), s)).map(SheetRef::Shape).collect();
                    kept += eng.recalc(&mut tx, &sheets).map_or(usize::MAX / 2, |r| r.kept.len());
                    shapes.push(s);
                }
                Err(e) => fail.push_str(&format!(" {nu}: {e:?}")),
            }
        }
    }
    let _ = tx.commit();
    let (_, _, bad) = crate::stencil_shapes::engine_state(&doc);
    let want: usize = docs.iter().map(|d| d.masters().count()).sum();
    h.check("stencils/business/drop", fail.is_empty() && shapes.len() == want && kept == 0 && bad.is_empty(), format!("{} instances of {want}, recalc kept {kept}, errors {bad:?}{fail}", shapes.len()));
}

fn master<'d>(docs: &'d [Document], folder: &str, nu: &str) -> Option<(&'d Document, diagram_core::MasterRef)> {
    let k = FOLDERS.iter().position(|f| *f == folder)?;
    let d = &docs[k];
    d.masters().find(|&m| MasterTexts::of(d, m).name_u == nu).map(|m| (d, m))
}

// a drawing with an instance of folder/nu in language `lang` at (x, y) mm
fn one(docs: &[Document], folder: &str, nu: &str, lang: usize) -> Option<(Document, PageRef, ShapeRef)> {
    let (src, m) = master(docs, folder, nu)?;
    let (mut doc, page) = sheet_doc(300.0, 200.0)?;
    let s = {
        let mut tx = doc.transaction("drop");
        let local = instance::import_master(&mut tx, src, m).ok()?;
        let s = instance::place(&mut tx, page, local, 100.0 * MM, 100.0 * MM, lang).ok()?;
        let sheets: Vec<SheetRef> = core::iter::once(s).chain(children(tx.doc(), s)).map(SheetRef::Shape).collect();
        let _ = Engine::new().recalc(&mut tx, &sheets);
        let _ = tx.commit();
        s
    };
    Some((doc, page, s))
}

// a change by the user (a dialog or a gesture) and its consequences
fn user_set(doc: &mut Document, sheet: SheetRef, addr: impl Fn(&mut Transaction) -> CellAddr, v: f64, u: Option<Unit>) -> bool {
    let mut tx = doc.transaction("set");
    let a = addr(&mut tx);
    let ok = Engine::new().user_set(&mut tx, sheet, &a, Input::Value(Value::Num(v), u)).is_ok();
    let _ = tx.commit();
    ok
}

fn recalc_all(doc: &mut Document, page: PageRef) {
    let mut tx = doc.transaction("recalc");
    let all: Vec<SheetRef> = tx.doc().all_shapes(Owner::Page(page)).map(SheetRef::Shape).collect();
    let _ = Engine::new().recalc(&mut tx, &all);
    let _ = tx.commit();
}

// the parts of group masters: their texts in five languages, in the language of the drop
fn parts(h: &mut H, docs: &[Document]) {
    for (lang, want) in [(0usize, ["Stärken", "Schwächen", "Chancen", "Risiken"]), (1, ["Strengths", "Weaknesses", "Opportunities", "Threats"]), (4, ["Forces", "Faiblesses", "Opportunités", "Menaces"])] {
        let Some((doc, _, s)) = one(docs, "marketing", "SWOT", lang) else {
            h.check("stencils/business/parts", false, String::from("SWOT does not drop"));
            return;
        };
        let got: Vec<String> = children(&doc, s).iter().map(|&c| shown_text(&doc, c)).collect();
        h.check("stencils/business/parts", got == want, format!("SWOT dropped in {}: {got:?}", instance::LANGS[lang]));
    }
    // the master's subshapes: five texts in User.JFD_Text, the transform guarded
    if let Some((d, m)) = master(docs, "venn", "Venn diagram 3 sets") {
        let slot = d.master(m).and_then(|x| x.master_shape());
        let subs: Vec<ShapeRef> = slot.map(|sl| children(d, ShapeRef { owner: Owner::Master(m), slot: sl })).unwrap_or_default();
        let texts: Vec<String> = subs
            .iter()
            .map(|&c| crate::stencils::cell(d, SheetRef::Shape(c), Some(("User", 0)), crate::stencils::named(d, instance::JFD_TEXT), "Value").map(|x| crate::stencils::sym_text(d, x.value)).unwrap_or_default())
            .collect();
        let guarded = subs.iter().all(|&c| {
            crate::stencils::cell(d, SheetRef::Shape(c), None, None, "PinX").and_then(|x| x.formula.expr()).is_some_and(|f| d.str(f).starts_with("GUARD(Sheet."))
        });
        h.check("stencils/business/parts/master", subs.len() == 3 && texts.iter().all(|t| t.split('|').count() == 5) && guarded, format!("{} parts, JFD_Text {texts:?}, guarded {guarded}", subs.len()));
    }
}

// counts from the shape data and from a control point (SETATREF): levels, steps, phases
fn counts(h: &mut H, docs: &[Document]) {
    // the layered pyramid: 6 levels from the shape data, the group grows, 6 parts shown
    if let Some((mut doc, page, s)) = one(docs, "pyramid", "Layered pyramid", 0) {
        let sh = SheetRef::Shape(s);
        let h0 = num(&doc, sh, "Height");
        let ok = user_set(&mut doc, sh, |tx| CellAddr::row(SecKey::one(tx.intern("Property")), RowKey::Named(tx.intern("Levels")), tx.intern("Value")), 6.0, None);
        recalc_all(&mut doc, page);
        let h1 = num(&doc, sh, "Height");
        let kids = children(&doc, s);
        let shown = kids.iter().filter(|&&c| !no_show(&doc, SheetRef::Shape(c), 0)).count();
        h.check("stencils/business/counts/levels", ok && (h1 / h0 - 1.5).abs() < 1e-6 && shown == 6, format!("4 -> 6 levels: height {:.1} -> {:.1} mm, {shown} of {} parts shown", h0 / MM, h1 / MM, kids.len()));
        // the control point at the top dragged down by one level: 5 levels
        let y = num(&doc, sh, "Height") - user_num(&doc, sh, "LH");
        let ok = user_set(&mut doc, sh, |tx| CellAddr::row(SecKey::one(tx.intern("Control")), RowKey::Named(tx.intern("Row_1")), tx.intern("Y")), y, Some(Unit::Millimeters));
        recalc_all(&mut doc, page);
        let n = prop_num(&doc, sh, "Levels");
        h.check("stencils/business/counts/levels-drag", ok && n == 5.0, format!("control point down by one level: Prop.Levels {n}, height {:.1} mm", num(&doc, sh, "Height") / MM));
    }
    // the step chart: dragging the control point two steps to the right gives 6 steps
    if let Some((mut doc, page, s)) = one(docs, "marketing", "Step chart", 0) {
        let sh = SheetRef::Shape(s);
        let (w0, sw) = (num(&doc, sh, "Width"), user_num(&doc, sh, "SW"));
        let ok = user_set(&mut doc, sh, |tx| CellAddr::row(SecKey::one(tx.intern("Control")), RowKey::Named(tx.intern("Row_1")), tx.intern("X")), w0 + 2.0 * sw, Some(Unit::Millimeters));
        recalc_all(&mut doc, page);
        let (n, w1) = (prop_num(&doc, sh, "Steps"), num(&doc, sh, "Width"));
        let shown = children(&doc, s).iter().filter(|&&c| !no_show(&doc, SheetRef::Shape(c), 0)).count();
        h.check("stencils/business/counts/steps", ok && n == 6.0 && (w1 - 6.0 * sw).abs() < 1e-9 && shown == 6, format!("control point +2 steps: Prop.Steps {n}, width {:.1} -> {:.1} mm, {shown} steps shown", w0 / MM, w1 / MM));
        // resizing the width goes to the width of a step (SETATREF)
        let ok = user_set(&mut doc, sh, |tx| tx.single("Width"), w1 * 1.5, Some(Unit::Millimeters));
        recalc_all(&mut doc, page);
        let sw1 = user_num(&doc, sh, "SW");
        h.check("stencils/business/counts/steps-width", ok && (sw1 - sw * 1.5).abs() < 1e-9 && (num(&doc, sh, "Width") - w1 * 1.5).abs() < 1e-9, format!("width x 1.5: step width {:.1} -> {:.1} mm", sw / MM, sw1 / MM));
    }
    // the circular motion: 6 phases from the shape data
    if let Some((mut doc, page, s)) = one(docs, "cycle", "Circular motion", 0) {
        let sh = SheetRef::Shape(s);
        let ok = user_set(&mut doc, sh, |tx| CellAddr::row(SecKey::one(tx.intern("Property")), RowKey::Named(tx.intern("Phases")), tx.intern("Value")), 6.0, None);
        recalc_all(&mut doc, page);
        let shown = children(&doc, s).iter().filter(|&&c| !no_show(&doc, SheetRef::Shape(c), 0)).count();
        h.check("stencils/business/counts/phases", ok && shown == 6, format!("6 phases: {shown} segments shown"));
    }
}

// values: the pie slice's angle from its control point, the pie chart slice from its data
fn values(h: &mut H, docs: &[Document]) {
    if let Some((mut doc, page, s)) = one(docs, "charting", "Pie slice", 0) {
        let sh = SheetRef::Shape(s);
        let (w, hh) = (num(&doc, sh, "Width"), num(&doc, sh, "Height"));
        // the handle to the left (270° clockwise from the top)
        let okx = user_set(&mut doc, sh, |tx| CellAddr::row(SecKey::one(tx.intern("Control")), RowKey::Named(tx.intern("Row_1")), tx.intern("X")), 0.0, Some(Unit::Millimeters));
        let oky = user_set(&mut doc, sh, |tx| CellAddr::row(SecKey::one(tx.intern("Control")), RowKey::Named(tx.intern("Row_1")), tx.intern("Y")), hh * 0.5, Some(Unit::Millimeters));
        recalc_all(&mut doc, page);
        let a = user_num(&doc, sh, "S").to_degrees();
        h.check("stencils/business/values/pie-slice", okx && oky && (a - 270.0).abs() < 0.01, format!("handle at the left of a {:.0} mm slice: {a:.2}°", w / MM));
    }
    if let Some((mut doc, page, s)) = one(docs, "charting", "Pie chart slice", 0) {
        let sh = SheetRef::Shape(s);
        let p = |tx: &mut Transaction, n: &str| CellAddr::row(SecKey::one(tx.intern("Property")), RowKey::Named(tx.intern(n)), tx.intern("Value"));
        let ok = user_set(&mut doc, sh, |tx| p(tx, "Start"), 20.0, None) && user_set(&mut doc, sh, |tx| p(tx, "Share"), 30.0, None) && user_set(&mut doc, sh, |tx| p(tx, "Color"), 3.0, None);
        recalc_all(&mut doc, page);
        let (a0, a1) = (user_num(&doc, sh, "A0").to_degrees(), user_num(&doc, sh, "A1").to_degrees());
        let colour = num(&doc, sh, "QuickStyleFillColor");
        h.check("stencils/business/values/pie-chart", ok && (a0 - 72.0).abs() < 1e-6 && (a1 - 180.0).abs() < 1e-6 && colour == 102.0, format!("start 20 %, share 30 %, colour 3: {a0:.1}° .. {a1:.1}°, QuickStyleFillColor {colour}"));
    }
}

// the fishbone: a category glued to the effect's spine follows it, a cause glued to the
// category follows the category
fn fishbone(h: &mut H, docs: &[Document]) {
    let (Some((ce, me)), Some((cc, mc)), Some((cp, mp))) = (master(docs, "cause_effect", "Effect"), master(docs, "cause_effect", "Category 1"), master(docs, "cause_effect", "Primary cause 1")) else {
        h.check("stencils/business/fishbone", false, String::from("masters missing"));
        return;
    };
    let _ = (me, mc, mp);
    let Some((mut doc, page)) = sheet_doc(300.0, 200.0) else { return };
    let mut eng = Engine::new();
    let mut tx = doc.transaction("fishbone");
    let effect = put(&mut tx, ce, page, "Effect", (150.0, 100.0), (0.0, 0.0), None, &none);
    recalc_page(&mut tx, &mut eng, page);
    let mut cat = None;
    let mut cause = None;
    let mut glued = 0;
    if let Some(e) = effect {
        if let Some(p) = point(tx.doc(), e, 4) {
            cat = put_1d(&mut tx, cc, page, "Category 1", (p[0] / MM - 20.0, p[1] / MM + 30.0), (p[0] / MM, p[1] / MM));
            if let Some(c) = cat {
                glued += usize::from(glue_to(&mut tx, &mut eng, c, End::End, e, 4));
                recalc_page(&mut tx, &mut eng, page);
                if let Some(q) = point(tx.doc(), c, 4) {
                    cause = put_1d(&mut tx, cp, page, "Primary cause 1", (q[0] / MM - 30.0, q[1] / MM), (q[0] / MM, q[1] / MM));
                    if let Some(u) = cause {
                        glued += usize::from(glue_to(&mut tx, &mut eng, u, End::End, c, 4));
                    }
                }
            }
        }
    }
    let _ = tx.commit();
    let (Some(e), Some(c), Some(u)) = (effect, cat, cause) else {
        h.check("stencils/business/fishbone", false, String::from("the shapes do not drop"));
        return;
    };
    h.check("stencils/business/fishbone/glue", glued == 2, format!("{glued} of 2 ends glued"));
    // move the effect by (20, -10) mm: the category's end and the cause's end follow
    let mut tx = doc.transaction("move");
    let moved = ops::move_shapes(&mut tx, &mut eng, &[e], [20.0 * MM, -10.0 * MM]).is_ok();
    let _ = tx.commit();
    recalc_all(&mut doc, page);
    let near = |a: Option<[f64; 2]>, b: Option<[f64; 2]>| a.zip(b).is_some_and(|(a, b)| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6);
    let cat_end = glue::ends(&doc, c).map(|x| x.1);
    let cause_end = glue::ends(&doc, u).map(|x| x.1);
    let ok = moved && near(cat_end, point(&doc, e, 4)) && near(cause_end, point(&doc, c, 4));
    h.check(
        "stencils/business/fishbone/follow",
        ok,
        format!("effect moved: category end at the spine point {}, cause end at the bone point {}", near(cat_end, point(&doc, e, 4)), near(cause_end, point(&doc, c, 4))),
    );
}

// ---------------------------------------------------------------- the Visio counter-check

// a master's size in inches (its master shape's Width and Height)
fn master_size(doc: &Document, m: diagram_core::MasterRef) -> (f64, f64) {
    let Some(slot) = doc.master(m).and_then(|x| x.master_shape()) else { return (1.0, 1.0) };
    let sheet = SheetRef::Shape(ShapeRef { owner: Owner::Master(m), slot });
    let n = |c: &str| doc.sym(c).and_then(|s| doc.resolve(sheet, &CellAddr::single(s)).num()).unwrap_or(1.0).abs();
    (n("Width"), n("Height"))
}

fn save(doc: &Document, path: &str) -> bool {
    let ok = vsdx::write(doc, &vsdx::WriteOptions::DEFAULT).is_ok_and(|b| plat::write_file(path, &b));
    out(&format!("{} {path}\n", if ok { "wrote" } else { "error: cannot write" }));
    ok
}

// the top-level shape of a page whose NameU is `nu`
fn by_name_u(doc: &Document, page: PageRef, nu: &str) -> Option<ShapeRef> {
    doc.top_shapes(Owner::Page(page)).find(|&s| doc.shape(s).and_then(|x| x.header().name_u).is_some_and(|n| doc.str(n) == nu))
}

/// diagramiotest --write-jd26-gegenprobe DIR: the files for the Visio counter-check of JD26 --
/// schablonen/: the eight stencils as .vssx (metric and US); vorlagen/: the templates block,
/// cause_effect and charts (both measures, blank and with their starter); one drawing per template
/// made with it in German (template::to_drawing; in the charts drawing the pyramid set to 5
/// levels and the cycle to 6 phases through their shape data) with a PNG beside it;
/// JD26-Probeblatt.vsdx (the probe sheet without markers); JD26-Alle-Shapes.vsdx (every master
/// of the eight stencils, a page per stencil) and each page as JD26-Alle-Shapes-<folder>.png
pub fn gegenprobe(dir: &str) -> i32 {
    let mut ok = true;
    let _ = plat::fs::mkdir_private(&format!("{dir}/schablonen"));
    let _ = plat::fs::mkdir_private(&format!("{dir}/vorlagen"));
    for folder in FOLDERS {
        for m in [Measure::Metric, Measure::Us] {
            match stencil::build_vssx(&files_of("stencils", folder), m) {
                Ok((b, bytes)) => ok &= plat::write_file(&format!("{dir}/schablonen/{}", b.vssx_name()), &bytes),
                Err(e) => {
                    out(&format!("error: stencils/{folder}/{e}\n"));
                    ok = false;
                }
            }
        }
    }
    // the templates and a drawing made with each
    for m in [Measure::Metric, Measure::Us] {
        let Ok(st) = crate::templates::stencils(m) else {
            out("error: the stencils do not build\n");
            return 1;
        };
        for (folder, r) in crate::templates::build_all(m, &st) {
            if !matches!(folder, "block" | "cause_effect" | "charts") {
                continue;
            }
            let Ok(forms) = r else {
                out(&format!("error: templates/{folder}\n"));
                ok = false;
                continue;
            };
            for (b, bytes) in forms {
                ok &= plat::write_file(&format!("{dir}/vorlagen/{}", b.vstx_name()), &bytes);
                if b.starter.is_none() || m == Measure::Us {
                    continue;
                }
                let Ok(l) = vsdx::read(&bytes, &vsdx::Limits::VSDX) else {
                    ok = false;
                    continue;
                };
                let mut doc = l.doc;
                diagram_io::template::to_drawing(&mut doc, 0);
                let Some(page) = doc.pages().next() else { continue };
                if folder == "charts" {
                    let mut eng = Engine::new();
                    let mut tx = doc.transaction("data");
                    for (nu, row, v) in [("Layered pyramid", "Levels", 5.0), ("Circular motion", "Phases", 6.0)] {
                        if let Some(s) = by_name_u(tx.doc(), page, nu) {
                            let a = CellAddr::row(SecKey::one(tx.intern("Property")), RowKey::Named(tx.intern(row)), tx.intern("Value"));
                            let _ = eng.user_set(&mut tx, SheetRef::Shape(s), &a, Input::Value(Value::Num(v), None));
                        }
                    }
                    recalc_page(&mut tx, &mut eng, page);
                    let _ = tx.commit();
                }
                let name = match folder {
                    "block" => "JD26-Blockdiagramm",
                    "cause_effect" => "JD26-Ursache-Wirkung",
                    _ => "JD26-Diagramme-und-Grafiken",
                };
                ok &= save(&doc, &format!("{dir}/{name}.vsdx"));
                ok &= render_page_at(&mut doc, page, &format!("{dir}/{name}.png"), 110.0);
            }
        }
    }
    // the probe sheet
    match probe_doc() {
        Some((mut doc, page, _)) => {
            recalc_all(&mut doc, page);
            ok &= save(&doc, &format!("{dir}/JD26-Probeblatt.vsdx"));
        }
        None => ok = false,
    }
    // every master, a page per stencil (420 x 600 mm), in rows by their own size
    let Some(docs) = all_built() else { return 1 };
    let Some((mut doc, first)) = sheet_doc(420.0, 600.0) else { return 1 };
    {
        let mut tx = doc.transaction("Alle Shapes");
        let mut page = first;
        for (k, src) in docs.iter().enumerate() {
            let title = String::from(StencilTexts::of(src).name(0));
            if k == 0 {
                let _ = tx.rename_page(page, &title, FOLDERS[k]);
            } else {
                match tx.add_page_like(first, &title, FOLDERS[k], false, None) {
                    Ok(p) => page = p,
                    Err(_) => {
                        ok = false;
                        continue;
                    }
                }
            }
            let (mut x, mut y, mut row_h) = (10.0 * MM, 590.0 * MM, 0.0f64);
            let mut placed: Vec<SheetRef> = Vec::new();
            for m in src.masters() {
                let (w, h) = master_size(src, m);
                if x + w > 410.0 * MM && x > 10.0 * MM {
                    x = 10.0 * MM;
                    y -= row_h + 12.0 * MM;
                    row_h = 0.0;
                }
                let local = instance::import_master(&mut tx, src, m);
                let s = local.and_then(|l| instance::place(&mut tx, page, l, x + w * 0.5, y - h * 0.5, 0));
                match s {
                    Ok(s) => {
                        placed.push(SheetRef::Shape(s));
                        placed.extend(children(tx.doc(), s).into_iter().map(SheetRef::Shape));
                    }
                    Err(_) => ok = false,
                }
                x += w + 10.0 * MM;
                row_h = row_h.max(h);
            }
            let _ = Engine::new().recalc(&mut tx, &placed);
        }
        let _ = tx.commit();
    }
    ok &= save(&doc, &format!("{dir}/JD26-Alle-Shapes.vsdx"));
    // each page in the masters' own size, to look at
    let pages: Vec<PageRef> = doc.pages().collect();
    for (k, p) in pages.into_iter().enumerate() {
        if let Some(f) = FOLDERS.get(k) {
            ok &= render_page_at(&mut doc, p, &format!("{dir}/JD26-Alle-Shapes-{f}.png"), 110.0);
        }
    }
    if ok { 0 } else { 1 }
}
