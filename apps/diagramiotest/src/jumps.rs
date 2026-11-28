// jumps -- (JD21, CN-06) line jumps: a drawing of crossing dynamic connectors built as
// JFDiagrammer builds it, one page per jump style, per LineJumpCode, per direction, a size, close
// crossings, slanted straight connectors and connectors with settings of their own; the jumps
// in the connectors' Geometry (JD21v: written when they are laid, as Visio does), written and
// read back, the SVG and PNG export; a file Visio laid anew against the same laying here
//
//   check(h)          group "jumps": every page's jumps (count per connector, style, side,
//                     width) against what the page asks for; written and read back: the page's
//                     and the connectors' line jump cells and their Geometry rows with the
//                     jumps, the same jumps after reading; the SVG export holds them (cubic
//                     pieces, gaps), the PNG export shows an arc above the line
//   write(dir)        --write-jd21-jumps DIR: JD21-Gegenprobe-Linienuebergaenge.vsdx and a PNG
//                     of each page drawn by JFDiagrammer's drawing code, to lay next to Visio
//   compare(file)     --compare-visio-jumps FILE: a drawing Visio saved after it laid every
//                     connector anew in page order (jd21v_jumps.vbs): the same laying here, and
//                     every connector's rows against Visio's (types, points, bows, POLYLINE
//                     numbers to 1e-6 in) -> 0: all equal
//
// The pages: three horizontal connectors (y 2, 4, 6 in, x 1 .. 9) and three vertical ones (x 3,
// 5, 7, y 1 .. 7), laid in that order (the verticals lie above and have the higher IDs), each
// with free ends; a title box above them says what Visio should show.  The first page is called
// "Vorgabe" (Visio renames a page named like a style, "Standard").

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::jumps::{self, Jump, JumpStyle};
use diagram_core::render::Options;
use diagram_core::{glue, route_edit, CellAddr, DocKind, Document, Owner, PageRef, ShapeRef, SheetRef, Text, Value};
use diagram_io::vsdx::{self, Measure, WriteOptions};
use diagram_view::export::{raster, svg};
use diagram_view::PageView;

use crate::H;

/// a page of the drawing: its name, its cells, what changes on it, the title
struct Spec {
    name: &'static str,
    cells: &'static [(&'static str, f64)],
    layout: Layout,
    title: &'static str,
    /// jumps per horizontal and per vertical connector; (style, side of the horizontals' or
    /// verticals' jumps: [x, y] of the normal)
    want: ([usize; 3], [usize; 3]),
    look: Option<(JumpStyle, [f64; 2])>,
}

#[derive(Copy, Clone, PartialEq)]
enum Layout {
    /// the grid
    Grid,
    /// the middle horizontal brought to the front and laid again (moved by nothing)
    MiddleOnTop,
    /// the grid, the middle vertical 0.05 in right of the first
    Close,
    /// the first horizontal ConLineJumpCode 2 (always)
    Always,
    /// first horizontal never, second square and down, middle vertical neither
    Own,
    /// two straight connectors as an X and a flat one under them
    Slanted,
}

const CODE: &str = "LineJumpCode";
const STYLE: &str = "LineJumpStyle";
const BASE: [(&str, f64); 2] = [("LineToLineX", 0.125), ("LineToLineY", 0.125)];

// i18n: none (the file is for the user's look in Visio, German like the instructions)
const SPECS: [Spec; 23] = [
    Spec { name: "Vorgabe", cells: &[], layout: Layout::Grid, title: "Ohne Linienübergangs-Zellen der Seite: Visios Vorgabe (waagerechte springen, Bogen nach oben, 2/3 von 2,5 mm breit)", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Arc, [0.0, 1.0])) },
    Spec { name: "Stil Bogen", cells: &[(CODE, 1.0), (STYLE, 1.0)], layout: Layout::Grid, title: "Stil 1 Bogen: waagerechte springen", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Arc, [0.0, 1.0])) },
    Spec { name: "Stil Lücke", cells: &[(CODE, 1.0), (STYLE, 2.0)], layout: Layout::Grid, title: "Stil 2 Lücke", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Gap, [0.0, 1.0])) },
    Spec { name: "Stil Quadrat", cells: &[(CODE, 1.0), (STYLE, 3.0)], layout: Layout::Grid, title: "Stil 3 Quadrat", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Square, [0.0, 1.0])) },
    Spec { name: "Stil 2 Seiten", cells: &[(CODE, 1.0), (STYLE, 4.0)], layout: Layout::Grid, title: "Stil 4: 2 Seiten", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Sides(2), [0.0, 1.0])) },
    Spec { name: "Stil 3 Seiten", cells: &[(CODE, 1.0), (STYLE, 5.0)], layout: Layout::Grid, title: "Stil 5: 3 Seiten", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Sides(3), [0.0, 1.0])) },
    Spec { name: "Stil 4 Seiten", cells: &[(CODE, 1.0), (STYLE, 6.0)], layout: Layout::Grid, title: "Stil 6: 4 Seiten", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Sides(4), [0.0, 1.0])) },
    Spec { name: "Stil 5 Seiten", cells: &[(CODE, 1.0), (STYLE, 7.0)], layout: Layout::Grid, title: "Stil 7: 5 Seiten", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Sides(5), [0.0, 1.0])) },
    Spec { name: "Stil 6 Seiten", cells: &[(CODE, 1.0), (STYLE, 8.0)], layout: Layout::Grid, title: "Stil 8: 6 Seiten", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Sides(6), [0.0, 1.0])) },
    Spec { name: "Stil 7 Seiten", cells: &[(CODE, 1.0), (STYLE, 9.0)], layout: Layout::Grid, title: "Stil 9: 7 Seiten", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Sides(7), [0.0, 1.0])) },
    Spec { name: "Wo keine", cells: &[(CODE, 0.0)], layout: Layout::Grid, title: "LineJumpCode 0: keine Linienübergänge", want: ([0; 3], [0; 3]), look: None },
    Spec { name: "Wo senkrecht", cells: &[(CODE, 2.0)], layout: Layout::Grid, title: "LineJumpCode 2: senkrechte springen (Vorgabe-Richtung)", want: ([0; 3], [3; 3]), look: Some((JumpStyle::Arc, [-1.0, 0.0])) },
    Spec { name: "Wo zuletzt verlegt", cells: &[(CODE, 3.0)], layout: Layout::MiddleOnTop, title: "LineJumpCode 3: zuletzt verlegte (die senkrechten nach den waagerechten, die mittlere waagerechte zuletzt neu verlegt)", want: ([0, 3, 0], [2; 3]), look: None },
    Spec { name: "Wo zuletzt angezeigt", cells: &[(CODE, 4.0)], layout: Layout::MiddleOnTop, title: "LineJumpCode 4: die obere springt (die mittlere waagerechte ganz oben und neu verlegt)", want: ([0, 3, 0], [2; 3]), look: None },
    Spec { name: "Wo zuerst angezeigt", cells: &[(CODE, 5.0)], layout: Layout::MiddleOnTop, title: "LineJumpCode 5: die untere springt (die mittlere waagerechte ganz oben und neu verlegt)", want: ([3, 0, 3], [1; 3]), look: None },
    Spec { name: "Richtung unten", cells: &[(CODE, 1.0), ("PageLineJumpDirX", 2.0)], layout: Layout::Grid, title: "PageLineJumpDirX 2: waagerechte springen nach unten", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Arc, [0.0, -1.0])) },
    Spec { name: "Richtung rechts", cells: &[(CODE, 2.0), ("PageLineJumpDirY", 2.0)], layout: Layout::Grid, title: "PageLineJumpDirY 2: senkrechte springen nach rechts", want: ([0; 3], [3; 3]), look: Some((JumpStyle::Arc, [1.0, 0.0])) },
    Spec { name: "Richtung links", cells: &[(CODE, 2.0), ("PageLineJumpDirY", 1.0)], layout: Layout::Grid, title: "PageLineJumpDirY 1: senkrechte springen nach links", want: ([0; 3], [3; 3]), look: Some((JumpStyle::Arc, [-1.0, 0.0])) },
    Spec { name: "Größe", cells: &[(CODE, 1.0), ("LineJumpFactorX", 3.0), ("LineJumpFactorY", 0.3)], layout: Layout::Grid, title: "LineJumpFactorX 3 (waagerecht groß), LineToLineX 0,125 in", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Arc, [0.0, 1.0])) },
    Spec { name: "Nah", cells: &[(CODE, 1.0)], layout: Layout::Close, title: "Zwei senkrechte 1,3 mm nebeneinander: ein breiter Sprung, halb so hoch wie breit", want: ([2; 3], [0; 3]), look: None },
    Spec { name: "Schräg", cells: &[(CODE, 1.0)], layout: Layout::Slanted, title: "Gerade Verbinder schräg: die flachere springt, beim X (gleiche Steigung) keine", want: ([0; 3], [0; 3]), look: None },
    Spec { name: "Eigene immer", cells: &[(CODE, 0.0)], layout: Layout::Always, title: "Seite keine; oberste waagerechte ConLineJumpCode 2 (immer)", want: ([0, 0, 3], [0; 3]), look: None },
    Spec { name: "Eigene nie Stil", cells: &[(CODE, 1.0)], layout: Layout::Own, title: "Seite waagerecht; oben: nie; Mitte: Quadrat nach unten; mittlere senkrechte: keiner springt", want: ([2, 2, 0], [0; 3]), look: None },
];

/// a page as built: its connectors (horizontals, verticals) and the slanted ones
struct Built {
    page: PageRef,
    hs: Vec<ShapeRef>,
    vs: Vec<ShapeRef>,
    slanted: Vec<ShapeRef>,
}

fn err(e: diagram_core::EditError) -> String {
    format!("{e:?}")
}

fn set(tx: &mut diagram_core::Transaction, sheet: SheetRef, cell: &str, v: f64) -> Result<(), String> {
    let a = tx.single(cell);
    tx.set_constant(sheet, &a, Value::Num(v), None).map_err(err)
}

fn line(tx: &mut diagram_core::Transaction, eng: &mut Engine, owner: Owner, a: [f64; 2], b: [f64; 2]) -> Result<ShapeRef, String> {
    glue::new_connector(tx, eng, owner, (a, glue::Target::None), (b, glue::Target::None)).map_err(err)
}

/// the drawing: the pages of SPECS (the first one renamed, the rest added like it)
fn build() -> Result<(Document, Vec<Built>), String> {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let first = doc.pages().next().ok_or("no page")?;
    let mut eng = Engine::new();
    let mut out = Vec::new();
    for (k, spec) in SPECS.iter().enumerate() {
        let mut tx = doc.transaction("Seite");
        let page = if k == 0 {
            tx.rename_page(first, spec.name, spec.name).map_err(err)?;
            first
        } else {
            tx.add_page_like(first, spec.name, spec.name, false, None).map_err(err)?
        };
        let ps = SheetRef::Page(page);
        if !spec.cells.is_empty() {
            for &(c, v) in BASE.iter().chain(spec.cells.iter()) {
                set(&mut tx, ps, c, v)?;
            }
        }
        let owner = Owner::Page(page);
        let (mut hs, mut vs, mut slanted) = (Vec::new(), Vec::new(), Vec::new());
        if spec.layout == Layout::Slanted {
            for (a, b) in [([2.0, 1.0], [8.0, 7.0]), ([2.0, 7.0], [8.0, 1.0]), ([1.0, 2.0], [10.0, 3.0])] {
                slanted.push(line(&mut tx, &mut eng, owner, a, b)?);
            }
            glue::set_style(&mut tx, &mut eng, &slanted, glue::Style::Straight).map_err(err)?;
        } else {
            for y in [2.0, 4.0, 6.0] {
                hs.push(line(&mut tx, &mut eng, owner, [1.0, y], [9.0, y])?);
            }
            let xs = if spec.layout == Layout::Close { [3.0, 3.05, 7.0] } else { [3.0, 5.0, 7.0] };
            for x in xs {
                vs.push(line(&mut tx, &mut eng, owner, [x, 1.0], [x, 7.0])?);
            }
        }
        let all: Vec<ShapeRef> = hs.iter().chain(&vs).copied().collect();
        match spec.layout {
            Layout::MiddleOnTop => {
                // to the front and laid again (as a nudge in Visio): the last one laid
                tx.bring_to_front(hs[1]).map_err(err)?;
                glue::update(&mut tx, &mut eng, &[hs[1]]).map_err(err)?;
            }
            Layout::Always => {
                set(&mut tx, SheetRef::Shape(hs[2]), "ConLineJumpCode", 2.0)?;
                jumps::lay_again(&mut tx, &mut eng, &all).map_err(err)?;
            }
            Layout::Own => {
                set(&mut tx, SheetRef::Shape(hs[2]), "ConLineJumpCode", 1.0)?;
                set(&mut tx, SheetRef::Shape(hs[1]), "ConLineJumpStyle", 3.0)?;
                set(&mut tx, SheetRef::Shape(hs[1]), "ConLineJumpDirX", 2.0)?;
                set(&mut tx, SheetRef::Shape(vs[1]), "ConLineJumpCode", 4.0)?;
                jumps::lay_again(&mut tx, &mut eng, &all).map_err(err)?;
            }
            _ => {}
        }
        // the title above the connectors
        let t = ops::new_shape(&mut tx, &mut eng, owner, Basic::Rectangle, [0.5, 7.4], [11.2, 8.0]).map_err(err)?;
        ops::set_text(&mut tx, &mut eng, t, Text::plain(&format!("{}\n", spec.title))).map_err(err)?;
        let _ = tx.commit();
        out.push(Built { page, hs, vs, slanted });
    }
    Ok((doc, out))
}

fn jumps_of(doc: &Document, s: ShapeRef) -> Vec<Jump> {
    jumps::read(doc, s)
}

fn num(doc: &Document, s: SheetRef, cell: &str) -> Option<f64> {
    doc.sym(cell).and_then(|c| doc.resolve(s, &CellAddr::single(c)).num())
}

fn geometry_rows(doc: &Document, s: ShapeRef) -> usize {
    let Some(g) = doc.sym("Geometry") else { return 0 };
    doc.section_indices(SheetRef::Shape(s), g).into_iter().map(|ix| doc.rows(SheetRef::Shape(s), diagram_core::SecKey { name: g, index: ix }).len()).sum()
}

/// group "jumps"
pub(crate) fn check(h: &mut H) {
    let (doc, built) = match build() {
        Ok(b) => b,
        Err(e) => {
            h.check("jumps: build the drawing", false, e);
            return;
        }
    };
    // the jumps of every page
    for (spec, b) in SPECS.iter().zip(&built) {
        let hs: Vec<usize> = b.hs.iter().map(|&s| jumps_of(&doc, s).len()).collect();
        let vs: Vec<usize> = b.vs.iter().map(|&s| jumps_of(&doc, s).len()).collect();
        if spec.layout == Layout::Slanted {
            let n: Vec<usize> = b.slanted.iter().map(|&s| jumps_of(&doc, s).len()).collect();
            h.check("jumps: slanted straight connectors (the flat one twice, the X none)", n == [0, 0, 2], format!("{n:?}"));
            continue;
        }
        h.check(&format!("jumps: page {} counts", spec.name), hs == spec.want.0 && vs == spec.want.1, format!("{hs:?} {vs:?}"));
        if let Some((style, side)) = spec.look {
            let all: Vec<Jump> = b.hs.iter().chain(&b.vs).flat_map(|&s| jumps_of(&doc, s)).collect();
            let gap = style == JumpStyle::Gap;
            let ok = !all.is_empty() && all.iter().all(|j| j.style == style && (gap || ((j.normal[0] - side[0]).abs() < 1e-9 && (j.normal[1] - side[1]).abs() < 1e-9)));
            h.check(&format!("jumps: page {} style and side", spec.name), ok, format!("{:?}", all.first().map(|j| (j.style, j.normal))));
        }
        if spec.name == "Vorgabe" {
            let w = jumps_of(&doc, b.hs[0]).first().map_or(0.0, |j| j.width());
            h.check("jumps: a page without the cells: 2/3 of 2.5 mm", (w - 2.5 / 25.4 * 2.0 / 3.0).abs() < 1e-9, format!("{w}"));
        }
        if spec.name == "Größe" {
            let w = jumps_of(&doc, b.hs[0]).first().map_or(0.0, |j| j.width());
            h.check("jumps: size LineJumpFactorX 3 x LineToLineX 0.125", (w - 0.375).abs() < 1e-9, format!("{w}"));
        }
        if spec.name == "Nah" {
            let j = jumps_of(&doc, b.hs[0]);
            let (w, ht) = j.first().map_or((0.0, 0.0), |j| (j.width(), j.height));
            let want = 0.05 + 0.125 * 2.0 / 3.0;
            h.check("jumps: close crossings merged, half as high as wide", (w - want).abs() < 1e-9 && (ht - want * 0.5).abs() < 1e-9, format!("{w} {ht}"));
        }
        if spec.name == "Eigene nie Stil" {
            let j = jumps_of(&doc, b.hs[1]);
            let ok = j.len() == 2 && j.iter().all(|j| j.style == JumpStyle::Square && j.normal[1] < 0.0);
            h.check("jumps: a connector's own style and direction", ok, format!("{:?}", j.first().map(|j| (j.style, j.normal))));
        }
    }
    // written and read back
    let bytes = match vsdx::write(&doc, &WriteOptions::DEFAULT) {
        Ok(b) => b,
        Err(e) => {
            h.check("jumps: write", false, format!("{e}"));
            return;
        }
    };
    let back = match vsdx::read(&bytes, &vsdx::Limits::VSDX) {
        Ok(l) => l.doc,
        Err(e) => {
            h.check("jumps: read back", false, format!("{e}"));
            return;
        }
    };
    let pages: Vec<PageRef> = back.pages().collect();
    let page_cells = ["LineJumpCode", "LineJumpStyle", "LineJumpFactorX", "LineJumpFactorY", "LineToLineX", "LineToLineY", "PageLineJumpDirX", "PageLineJumpDirY"];
    let con_cells = ["ConLineJumpCode", "ConLineJumpStyle", "ConLineJumpDirX", "ConLineJumpDirY"];
    let (mut cells_ok, mut rows_ok, mut same_jumps, mut n) = (true, true, true, 0usize);
    for (b, &pb) in built.iter().zip(&pages) {
        for c in page_cells {
            cells_ok &= num(&doc, SheetRef::Page(b.page), c) == num(&back, SheetRef::Page(pb), c);
        }
        for &s in b.hs.iter().chain(&b.vs).chain(&b.slanted) {
            let id = doc.shape(s).map_or(0, |x| x.id());
            let Some(t) = back.shape_by_id(Owner::Page(pb), id) else {
                cells_ok = false;
                continue;
            };
            for c in con_cells {
                cells_ok &= num(&doc, SheetRef::Shape(s), c) == num(&back, SheetRef::Shape(t), c);
            }
            let (a, z) = (jumps_of(&doc, s), jumps_of(&back, t));
            rows_ok &= geometry_rows(&doc, s) == geometry_rows(&back, t);
            n += a.len();
            same_jumps &= a.len() == z.len() && a.iter().zip(&z).all(|(x, y)| (x.start[0] - y.start[0]).abs() < 1e-9 && (x.start[1] - y.start[1]).abs() < 1e-9 && x.style == y.style && (x.height - y.height).abs() < 1e-9);
        }
    }
    h.check("jumps: page and connector line jump cells read back", cells_ok, String::new());
    h.check("jumps: the connectors' Geometry rows with their jumps read back", rows_ok, String::new());
    h.check("jumps: the same jumps after reading", same_jumps && n > 50, format!("{n} jumps"));
    // the SVG export: arcs as cubic pieces, gaps as sub-paths
    let mut fonts = crate::render::fonts();
    let svg_of = |fonts: &mut diagram_view::FontSet, k: usize| {
        let v = PageView::new(&doc, built[k].page, fonts, &Options::default());
        svg::write(&doc, v.drawing(), &svg::SvgOptions::default())
    };
    let group = |text: &str, s: ShapeRef| -> String {
        let id = doc.shape(s).map_or(0, |x| x.id());
        let open = format!("<g id=\"shape{id}\"");
        text.find(&open).map(|i| {
            let rest = &text[i..];
            String::from(&rest[..rest.find("</g>").unwrap_or(rest.len())])
        }).unwrap_or_default()
    };
    let arc = svg_of(&mut fonts, 1);
    let cs = group(&arc, built[1].hs[0]).matches('C').count();
    h.check("jumps: SVG of the arcs (two cubic pieces per jump)", cs == 6, format!("{cs} C"));
    // (the line end's path has its own M: the gaps' line has three more than the arcs')
    let gap = svg_of(&mut fonts, 2);
    let ms = group(&gap, built[2].hs[0]).matches('M').count();
    let arc_ms = group(&arc, built[1].hs[0]).matches('M').count();
    h.check("jumps: SVG of the gaps (a sub-path more per jump)", ms == arc_ms + 3, format!("{ms} M, arcs {arc_ms} M"));
    let none = svg_of(&mut fonts, 10);
    let cs = group(&none, built[10].hs[0]).matches('C').count();
    h.check("jumps: SVG without jumps (LineJumpCode 0)", cs == 0, format!("{cs} C"));
    // the PNG export: on the size page the line at (3.1, 2), 0.1 in right of the crossing, is
    // lifted (the arc spans x 2.8125 .. 3.1875 and is 0.16 in high there), the arc's flank
    // passes above it; on the page without jumps the line is there
    let dark = |fonts: &mut diagram_view::FontSet, k: usize, at: [f64; 2]| -> Option<bool> {
        let mut v = PageView::new(&doc, built[k].page, fonts, &Options::default());
        let dpi = 200.0;
        let sf = raster::render(&mut v, &raster::RasterOptions { dpi, region: Some([2.5, 1.5, 3.5, 2.5]), background: raster::Background::Page })?;
        // rows count from the region's top (y 2.5)
        let (x, y) = (((at[0] - 2.5) * dpi) as usize, ((2.5 - at[1]) * dpi) as usize);
        let mut darkest = 255;
        for dy in 0..3usize {
            let i = (y + dy - 1) * sf.stride as usize + x * 4;
            darkest = darkest.min(sf.px[i]).min(sf.px[i + 1]).min(sf.px[i + 2]);
        }
        Some(darkest < 200)
    };
    let lifted = dark(&mut fonts, 18, [3.1, 2.0]);
    let flank = dark(&mut fonts, 18, [3.1, 2.0 + 0.1875 * 0.8457]);
    let plain = dark(&mut fonts, 10, [3.1, 2.0]);
    h.check("jumps: PNG shows the arc instead of the line", lifted == Some(false) && flank == Some(true) && plain == Some(true), format!("{lifted:?} {flank:?} / {plain:?}"));
}

/// --write-jd21-jumps DIR: the drawing for the Visio counter-check and a PNG of each page
pub(crate) fn write(dir: &str) -> i32 {
    img_core::io::register(img_core::IMGF_PNG, Some(&img_png::CODEC));
    let (doc, built) = match build() {
        Ok(b) => b,
        Err(e) => {
            crate::out(&format!("build: {e}\n"));
            return 1;
        }
    };
    let bytes = match vsdx::write(&doc, &WriteOptions::DEFAULT) {
        Ok(b) => b,
        Err(e) => {
            crate::out(&format!("write: {e}\n"));
            return 1;
        }
    };
    let path = format!("{dir}/JD21-Gegenprobe-Linienuebergaenge.vsdx");
    if !plat::write_file(&path, &bytes) {
        crate::out(&format!("cannot write {path}\n"));
        return 1;
    }
    crate::out(&format!("wrote {path}\n"));
    let mut fonts = crate::render::fonts();
    let mut status = 0;
    for (k, (spec, b)) in SPECS.iter().zip(&built).enumerate() {
        let mut v = PageView::new(&doc, b.page, &mut fonts, &Options::default());
        let img = raster::render(&mut v, &raster::RasterOptions { dpi: 150.0, ..Default::default() });
        let name: String = spec.name.chars().map(|c| match c {
            'ä' => 'a',
            'ö' => 'o',
            'ü' => 'u',
            'ß' => 's',
            ' ' => '-',
            c => c,
        }).collect();
        let png_path = format!("{dir}/JD21-Seite-{:02}-{name}.png", k + 1);
        let ok = img.as_ref().and_then(|i| raster::png(i, 150.0)).is_some_and(|p| plat::write_file(&png_path, &p));
        crate::out(&format!("wrote {png_path}{}\n", if ok { "" } else { " (FAILED)" }));
        if !ok {
            status = 1;
        }
    }
    status
}

// a row of a connector's first Geometry section as compared: type, point on the page, A's
// number (ArcTo's bow), the numbers of a POLYLINE
type RowSeen = (String, [f64; 2], f64, Vec<f64>);

fn rows_seen(doc: &Document, c: ShapeRef) -> Vec<RowSeen> {
    let sheet = SheetRef::Shape(c);
    let Some(&sk) = doc.geometry(sheet).first() else { return Vec::new() };
    let m = doc.page_xform(c);
    let (Some(x), Some(y), Some(a)) = (doc.sym("X"), doc.sym("Y"), doc.sym("A")) else { return Vec::new() };
    doc.rows(sheet, sk)
        .into_iter()
        .map(|r| {
            let cell = |s| doc.resolve(sheet, &CellAddr::row(sk, r.key, s));
            let p = m.apply([cell(x).num().unwrap_or(0.0), cell(y).num().unwrap_or(0.0)]);
            let kind = String::from(r.kind.map_or("", |k| doc.str(k)));
            let (bow, poly) = match cell(a).value() {
                Value::Num(v) => (v, Vec::new()),
                Value::Text(t) => (0.0, diagram_core::geometry::parse_args(doc.str(t), "POLYLINE").unwrap_or_default()),
                _ => (0.0, Vec::new()),
            };
            let bow = if kind == "ArcTo" { bow } else { 0.0 };
            (kind, p, bow, poly)
        })
        .collect()
}

fn same_rows(a: &[RowSeen], b: &[RowSeen]) -> bool {
    let near = |x: f64, y: f64| (x - y).abs() <= 1e-6;
    a.len() == b.len()
        && a.iter().zip(b).all(|(r, s)| r.0 == s.0 && near(r.1[0], s.1[0]) && near(r.1[1], s.1[1]) && near(r.2, s.2) && r.3.len() == s.3.len() && r.3.iter().zip(&s.3).all(|(x, y)| near(*x, *y)))
}

fn describe(rows: &[RowSeen]) -> String {
    let mut t = String::new();
    for r in rows {
        let tag = match r.0.as_str() {
            "MoveTo" => "M",
            "LineTo" => "L",
            "ArcTo" => "A",
            "PolylineTo" => "P",
            k => k,
        };
        t.push_str(&format!("{tag}({:.4},{:.4}", r.1[0], r.1[1]));
        if r.0 == "ArcTo" {
            t.push_str(&format!(";{:.4}", r.2));
        }
        t.push_str(") ");
    }
    t
}

/// --compare-visio-jumps FILE: every connector of every page laid anew in page order as Visio's
/// check did (each along Visio's way without its jumps), the rows against Visio's -> 0: equal
pub(crate) fn compare(path: &str) -> i32 {
    let Some(bytes) = plat::read_file(path) else {
        crate::out(&format!("cannot read {path}\n"));
        return 1;
    };
    let mut doc = match vsdx::read(&bytes, &vsdx::Limits::VSDX) {
        Ok(l) => l.doc,
        Err(e) => {
            crate::out(&format!("read {path}: {e}\n"));
            return 1;
        }
    };
    let mut eng = Engine::new();
    let pages: Vec<PageRef> = doc.pages().collect();
    let (mut total, mut bad) = (0usize, 0usize);
    for (k, &page) in pages.iter().enumerate() {
        let conns: Vec<ShapeRef> = doc.top_shapes(Owner::Page(page)).filter(|&s| glue::is_connector(&doc, s)).collect();
        if conns.is_empty() {
            continue;
        }
        let visio: Vec<Vec<RowSeen>> = conns.iter().map(|&c| rows_seen(&doc, c)).collect();
        let ways: Vec<Vec<[f64; 2]>> = conns.iter().map(|&c| jumps::way(&doc, c).unwrap_or_else(|| glue::path(&doc, c))).collect();
        let mut tx = doc.transaction("Visio");
        for (&c, w) in conns.iter().zip(&ways) {
            if let Err(e) = route_edit::set_way(&mut tx, &mut eng, c, w) {
                crate::out(&format!("page {}: laying a connector: {e:?}\n", k + 1));
            }
        }
        tx.commit();
        let name = doc.page(page).and_then(|p| p.header.name.or(p.header.name_u)).map(|s| String::from(doc.str(s))).unwrap_or_default();
        let mut equal = 0;
        for (&c, v) in conns.iter().zip(&visio) {
            total += 1;
            let ours = rows_seen(&doc, c);
            if same_rows(&ours, v) {
                equal += 1;
            } else {
                bad += 1;
                let id = doc.shape(c).map_or(0, |s| s.id());
                crate::out(&format!("  page {} shape {id}:\n    Visio {}\n    ours  {}\n", k + 1, describe(v), describe(&ours)));
            }
        }
        crate::out(&format!("page {} {name}: {equal} of {} connectors as Visio\n", k + 1, conns.len()));
    }
    crate::out(&format!("{} of {total} connectors as Visio\n", total - bad));
    i32::from(bad > 0 || total == 0)
}
