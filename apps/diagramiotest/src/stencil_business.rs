// stencil_business -- (JD26) the general and business-graphics stencils of apps/JFDiagrammer/stencils:
// decorative, charting (SH-003, SH-004), venn (SH-007), marketing, matrix, pyramid, cycle (SH-030 to
// SH-033) and cause_effect (SH-041); own sources only
//
//   diagramiotest --write-stencil-previews DIR   also probeblatt-jd26.png: the sample masters of
//                       the eight stencils in their own size and in a variant (a moved control
//                       point, other shape data, the parts in English), the fishbone glued
//                       together, control points as yellow diamonds, connection points as blue
//                       dots; probeblatt-jd26-zeile1.png .. -zeile5.png each row enlarged;
//                       probeblatt-jd26-kontur.png, -kontrast.png the sheet in two other designs

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::design::apply::{self as design_apply, Scope};
use diagram_core::design::catalog;
use diagram_core::formula::Engine;
use diagram_core::glue::{self, End, Target};
use diagram_core::instance::{self, MasterTexts};
use diagram_core::render::Options;
use diagram_core::{ops, CellAddr, Document, Owner, PageRef, RowKey, SecKey, ShapeRef, SheetRef, Text, Transaction, Unit, Value};
use diagram_io::stencil;
use diagram_io::vsdx::Measure;
use diagram_view::export::raster;
use diagram_view::PageView;

use crate::stencil_structs::{none, put, render_page_at, set_single, sheet_doc};
use crate::templates::files_of;
use crate::out;

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

/// diagramiotest --write-stencil-previews DIR: probeblatt-jd26.png and its parts
pub fn previews(dir: &str) -> i32 {
    let mut docs: Vec<Document> = Vec::new();
    for f in FOLDERS {
        match built(f) {
            Ok(d) => docs.push(d),
            Err(e) => {
                out(&format!("error: {e}\n"));
                return 1;
            }
        }
    }
    let [dec, cha, ven, mkt, mat, pyr, cyc, cae] = [&docs[0], &docs[1], &docs[2], &docs[3], &docs[4], &docs[5], &docs[6], &docs[7]];
    let Ok(st) = built("standard") else { return 1 };
    const PW: f64 = 420.0;
    const PH: f64 = 410.0;
    let Some((mut doc, page)) = sheet_doc(PW, PH) else { return 1 };
    // five rows under the title, each with its title at the top and labels at the bottom
    const ROWS: [f64; 5] = [66.0, 82.0, 76.0, 70.0, 96.0];
    let top = |r: usize| PH - 12.0 - ROWS[..r].iter().sum::<f64>();
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
        let t = top(r) + 1.0;
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
